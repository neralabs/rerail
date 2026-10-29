//! Planner and lowering (03 §12; 05 Phase 6 row 5).
//!
//! A port of Typing-support's `rdl.py` (T) planner and SQL backend: `Stats` (the ingestion statistics, read in one
//! statement and refused when their fingerprint does not match the facts), `plan_rule` (Selinger dynamic programming
//! over the join literals, filters placed as soon as they can run), `make_plan`, `view_stats`, `sql_select`,
//! `ensure_indexes`, `merge_lattice`, `semi_naive_round` and `run_sql`'s stratum loop, in T's pure SQL mode (no
//! `RDL_NATIVE`). Its chosen plans and outputs on T's programs are the tests' goldens.
//!
//! Two input modes (decision 0009): T's, symbols as text in an attached facts file (`run_sql`, the goldens' mode),
//! and the product's, a program lowered to integers (`lower`) over the bound base-relation vtabs (decision 0008) with
//! the statistics in an attached schema (`run_atoms`). The planner is the same in both; only the rendering of symbol
//! operations (`@cat`, its splits, `flat_bin`'s operator) and where inputs and statistics live differ.
//!
//! T's native modes (`RDL_NATIVE`, `run_sql_native`) are ported too: `native_stratum` (the native set engine for a
//! whole stratum, `program`), `native_fixpoint` (a flat-lattice component, `fixpoint` with `kseed`, `vtab`,
//! `direct`), `native_set_fixpoint` (`setfix`), `join_spec` (`join`), `observe` and `live_stats` (statistics the
//! native store reports), `merge_upsert` (`upsert`) and the `tape_nearest` climb (`nearest`). They call the SQL
//! functions of T's extension (`logic::native`), which the caller registers on the connection, as T loads it.
//! Adaptations still to come: the emitted SQL through `sql::sql_ast` and plans into `compiled_plans`. Left out of the
//! port: T's measurement switches (`RDL_PLAN0`, `RDL_SQL_REPLAN`, `RDL_CLUSTER=0`, `RDL_WHOLE=0`, `RDL_REPLAN`, traces
//! and profiles, with `timed_plan`, T's planning timer) and its 1 GB page caches (over the 500 MB budget; SQLite's
//! defaults instead).

use super::front::{AggOp, Atom, Literal, Program, Rule, Term, Value, cmp_binds, strata};
use super::native::TOP;
use indexmap::IndexMap;
use rusqlite::Connection;
use rusqlite::types::Value as SqlValue;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;
use std::path::Path;

/// A planning or evaluation failure.
#[derive(Debug)]
pub(in crate::code_rule_dsl) enum Error {
    Front(super::front::Error),
    Sql(rusqlite::Error),
    /// The facts have no planner statistics, or statistics of other data.
    Statistics(String),
    /// A rule the planner cannot order or lower.
    Plan(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Front(e) => write!(f, "{e}"),
            Error::Sql(e) => write!(f, "{e}"),
            Error::Statistics(m) | Error::Plan(m) => f.write_str(m),
        }
    }
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Sql(e)
    }
}

impl From<super::front::Error> for Error {
    fn from(e: super::front::Error) -> Self {
        Error::Front(e)
    }
}

type Result<T> = std::result::Result<T, Error>;

pub(in crate::code_rule_dsl) mod lower;

/// Where a run reads its inputs and statistics, and how it holds symbols.
#[derive(Clone, Debug)]
pub(in crate::code_rule_dsl) struct Inputs {
    /// The schema prefix of the input relations: T's attached facts file (`f.`) or the bound vtabs (`main.`).
    facts: String,
    /// The schema prefix of the `stat_*` tables.
    stats: String,
    /// Symbols are atoms: the name atoms of `flat_bin`'s operators `+`, `-` and `*`. `None`: symbols are text.
    atoms: Option<[i64; 3]>,
    /// T's `RDL_NATIVE` modes (`native_modes`); empty is T's pure SQL mode.
    native: BTreeSet<String>,
}

/// T's `NATIVE`: the comma-separated modes, `all` as every one but `direct` and `upsert`, and `program` with the
/// pieces it runs on.
fn native_modes(modes: &str) -> BTreeSet<String> {
    let mut native: BTreeSet<String> = modes.split(',').filter(|m| !m.is_empty()).map(str::to_owned).collect();
    if native.remove("all") {
        native.extend(["nearest", "fixpoint", "kseed", "vtab", "setfix", "join"].map(str::to_owned));
    }
    if native.contains("program") {
        native.extend(["nearest", "setfix", "join"].map(str::to_owned));
    }
    native
}

/// Lowercase hex of a string's bytes (Python's `s.encode().hex()`).
fn hex(s: &str) -> String {
    s.bytes().map(|b| format!("{b:02x}")).collect()
}

/// A string from its hex (Python's `bytes.fromhex(v).decode()`).
fn unhex(s: &str) -> Option<String> {
    let bytes = (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect::<Option<Vec<u8>>>()?;
    String::from_utf8(bytes).ok()
}

impl Inputs {
    /// T's facts file attached as `f`, statistics in it.
    fn file() -> Self {
        Inputs { facts: "f.".to_owned(), stats: "f.".to_owned(), atoms: None, native: BTreeSet::new() }
    }
    /// `sym_text` of an expression in atoms mode, the expression itself in text mode.
    fn text(&self, e: &str) -> String {
        if self.atoms.is_some() { format!("sym_text({e})") } else { e.to_owned() }
    }
    /// `sym_atom` of a text expression in atoms mode.
    fn symbol(&self, e: &str) -> String {
        if self.atoms.is_some() { format!("sym_atom({e})") } else { e.to_owned() }
    }
}

/// A constant of a rule or of the statistics (`stat_val.val`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(in crate::code_rule_dsl) enum Constant {
    Int(i64),
    Str(String),
}

fn constant(t: &Term) -> Option<Constant> {
    match t {
        Term::Int(i) => Some(Constant::Int(*i)),
        Term::Str(s) => Some(Constant::Str(s.clone())),
        _ => None,
    }
}

fn sql_constant(v: &SqlValue) -> Option<Constant> {
    match v {
        SqlValue::Integer(i) => Some(Constant::Int(*i)),
        SqlValue::Text(s) => Some(Constant::Str(s.clone())),
        _ => None,
    }
}

fn int(v: &SqlValue) -> i64 {
    match v {
        SqlValue::Integer(i) => *i,
        _ => 0,
    }
}

fn text(v: &SqlValue) -> String {
    match v {
        SqlValue::Text(s) => s.clone(),
        SqlValue::Integer(i) => i.to_string(),
        _ => String::new(),
    }
}

/// How a join literal is read: a range inside one document, or a `nearest` climb or descent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) enum Mode {
    Range,
    Climb,
    Descend,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Mode::Range => "range",
            Mode::Climb => "climb",
            Mode::Descend => "descend",
        }
    }
}

/// A planned body literal: its position in the rule's body (T compared literals by identity), and its mode.
#[derive(Clone, Debug)]
pub(in crate::code_rule_dsl) struct Step {
    pub(in crate::code_rule_dsl) at: usize,
    pub(in crate::code_rule_dsl) lit: Literal,
    pub(in crate::code_rule_dsl) mode: Option<Mode>,
}

/// An atom's binding pattern: its relation, and which columns are bound, constants by value (T's `atom_sig`).
type Sig = (String, Vec<(usize, Option<Constant>)>);

fn atom_sig(a: &Atom, bound: &BTreeSet<String>) -> Sig {
    let cols = a
        .args
        .iter()
        .enumerate()
        .filter_map(|(c, t)| match t {
            Term::Var(v) if bound.contains(v) => Some((c, None)),
            _ => constant(t).map(|k| (c, Some(k))),
        })
        .collect();
    (a.rel.clone(), cols)
}

/// Bindings a step must have seen before its observed fanout replaces the estimate.
const OBS_MIN: i64 = 32;

/// The corpus statistics the planner prices plans with, read from the tables `stats.py` builds at ingestion
/// (`stat_*` in the facts database) in one statement. Nothing is counted at plan time (ast-index-spike S174, S178).
/// Statistics whose fingerprint (row counts) does not match the facts are refused.
pub(in crate::code_rule_dsl) struct Stats {
    /// (table, column) → (rows, ndv, sum of squared postings, the same within documents).
    col: HashMap<(String, String), [i64; 4]>,
    kfreq: IndexMap<String, i64>,
    size: HashMap<String, f64>,
    nodes: f64,
    parents: IndexMap<String, IndexMap<String, f64>>,
    pub(in crate::code_rule_dsl) card: HashMap<String, f64>,
    /// (rel, col) → posting-weighted fanout of an equality probe: sq / rows.
    pub(in crate::code_rule_dsl) fan: HashMap<(String, usize), f64>,
    /// (derived rel, col) → {value: postings}, for a column of few values (native statistics).
    pub(in crate::code_rule_dsl) dfreq: HashMap<(String, usize), HashMap<Constant, i64>>,
    /// Binding pattern → [bindings in, bindings out]: fanouts native joins observed.
    pub(in crate::code_rule_dsl) obs: HashMap<Sig, [i64; 2]>,
    /// The recursive component being planned (`atom_rows`).
    pub(in crate::code_rule_dsl) solving: BTreeSet<String>,
    /// Input rel → index column lists.
    indexes: HashMap<String, Vec<Vec<String>>>,
    /// Documents (node-id intervals, `stat_docrange`), if the facts have them.
    docs: Option<i64>,
    /// Exact postings of the program's constants: (table, column, value) → n.
    memo: HashMap<(String, String, Constant), i64>,
    steps_cache: RefCell<HashMap<(Option<String>, String), f64>>,
    kindsets: BTreeMap<String, Vec<String>>,
}

impl Stats {
    fn new(db: &Connection, prog: &Program, inputs: &Inputs) -> Result<Self> {
        let consts: BTreeSet<Constant> = prog
            .rules
            .iter()
            .flat_map(|r| &r.body)
            .filter_map(|l| if let Literal::Atom(a) = l { Some(a) } else { None })
            .flat_map(|a| a.args.iter().filter_map(constant))
            .collect();
        let holes = if consts.is_empty() { "NULL".to_owned() } else { vec!["?"; consts.len()].join(", ") };
        let (f, s) = (&inputs.facts, &inputs.stats);
        let mut sql = format!(
            "SELECT 'meta', k, NULL, v, NULL, NULL, NULL FROM {s}stat_meta \
             UNION ALL SELECT 'col', tbl, col, rows, ndv, sq, sqd FROM {s}stat_col \
             UNION ALL SELECT 'kind', kind, NULL, n, size, NULL, NULL FROM {s}stat_kind \
             UNION ALL SELECT 'edge', pkind, ckind, n, field, NULL, NULL FROM {s}stat_edge"
        );
        sql += &format!(" UNION ALL SELECT 'val', tbl, col, n, val, docs, sqd FROM {s}stat_val WHERE val IN ({holes})");
        for t in &prog.inputs {
            sql += &format!(" UNION ALL SELECT 'rows', '{t}', NULL, count(*), NULL, NULL, NULL FROM {f}{t}");
        }
        let params: Vec<SqlValue> = consts
            .iter()
            .map(|c| match c {
                Constant::Int(i) => SqlValue::Integer(*i),
                Constant::Str(s) => SqlValue::Text(s.clone()),
            })
            .collect();
        let rows: Vec<(String, [SqlValue; 6])> = (|| {
            let mut st = db.prepare(&sql)?;
            let rows = st.query_map(rusqlite::params_from_iter(params), |r| {
                Ok((r.get(0)?, [r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?]))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })()
        .map_err(|e| Error::Statistics(format!("the facts have no planner statistics ({e}); build them at ingestion")))?;
        let of = |tag: &'static str| rows.iter().filter(move |(t, _)| t == tag).map(|(_, r)| r);
        let meta: HashMap<String, i64> = of("meta").map(|r| (text(&r[0]), int(&r[2]))).collect();
        let stale: Vec<String> =
            of("rows").filter(|r| meta.get(&format!("rows.{}", text(&r[0]))) != Some(&int(&r[2]))).map(|r| text(&r[0])).collect();
        if !stale.is_empty() {
            return Err(Error::Statistics(format!("planner statistics do not match the facts ({stale:?}); rebuild them")));
        }
        let mut parents: IndexMap<String, IndexMap<String, i64>> = IndexMap::new();
        for r in of("edge") {
            *parents.entry(text(&r[1])).or_default().entry(text(&r[0])).or_default() += int(&r[2]);
        }
        let kfreq: IndexMap<String, i64> = of("kind").map(|r| (text(&r[0]), int(&r[2]))).collect();
        Ok(Stats {
            col: of("col").map(|r| ((text(&r[0]), text(&r[1])), [int(&r[2]), int(&r[3]), int(&r[4]), int(&r[5])])).collect(),
            size: of("kind").filter(|r| int(&r[2]) != 0).map(|r| (text(&r[0]), int(&r[3]) as f64 / int(&r[2]) as f64)).collect(),
            nodes: kfreq.values().sum::<i64>() as f64,
            kfreq,
            parents: parents
                .into_iter()
                .map(|(c, ps)| {
                    let total = ps.values().sum::<i64>() as f64;
                    (c, ps.into_iter().map(|(p, n)| (p, n as f64 / total)).collect())
                })
                .collect(),
            card: HashMap::new(),
            fan: HashMap::new(),
            dfreq: HashMap::new(),
            obs: HashMap::new(),
            solving: BTreeSet::new(),
            indexes: HashMap::new(),
            docs: meta.get("docs").copied(),
            memo: of("val").filter_map(|r| Some(((text(&r[0]), text(&r[1]), sql_constant(&r[3])?), int(&r[2])))).collect(),
            steps_cache: RefCell::new(HashMap::new()),
            kindsets: prog.kindsets.clone(),
        })
    }

    /// Exact postings for a constant probe on an input column; `None` if the column has no statistics.
    fn const_freq(&self, rel: &str, cname: &str, v: &Constant) -> Option<i64> {
        if let Some(&n) = self.memo.get(&(rel.to_owned(), cname.to_owned(), v.clone())) {
            return Some(n);
        }
        self.col.contains_key(&(rel.to_owned(), cname.to_owned())).then_some(0)
    }

    /// Posting-weighted fanout of an equality probe on `cname` confined to one document: sqd / rows.
    fn fan_doc(&self, rel: &str, cname: &str) -> f64 {
        let [rows, _, _, sqd] = self.col[&(rel.to_owned(), cname.to_owned())];
        if rows != 0 { sqd as f64 / rows as f64 } else { 1.0 }
    }

    /// Expected parent reads to climb from `kind` to the nearest ancestor in `kset` (an absorbing Markov chain).
    fn steps(&self, kind: Option<&str>, kset: &str) -> f64 {
        let key = (kind.map(str::to_owned), kset.to_owned());
        if let Some(&s) = self.steps_cache.borrow().get(&key) {
            return s;
        }
        let members: HashSet<&str> = self.kindsets.get(kset).into_iter().flatten().map(String::as_str).collect();
        let mut est: IndexMap<&str, f64> = IndexMap::new();
        // Tree depth bound; converges well before.
        for _ in 0..40 {
            let mut nxt = IndexMap::new();
            for (c, ps) in &self.parents {
                let mut s = 0.0;
                for (p, pr) in ps {
                    s += pr * if members.contains(p.as_str()) { 0.0 } else { est.get(p.as_str()).copied().unwrap_or(0.0) };
                }
                nxt.insert(c.as_str(), 1.0 + s);
            }
            est = nxt;
        }
        let mut out = match kind.and_then(|k| est.get(k)) {
            Some(&e) => e,
            None => 1.0 + est.values().sum::<f64>() / est.len().max(1) as f64,
        };
        if kind.is_none() {
            let total = self.kfreq.values().sum::<i64>() as f64;
            out = 0.0;
            for (k, &n) in &self.kfreq {
                if let Some(e) = est.get(k.as_str()) {
                    out += n as f64 / total * e;
                }
            }
        }
        self.steps_cache.borrow_mut().insert(key, out);
        out
    }
}

/// The planner's model of the data: statistics, distinct counts, and what the views are.
pub(in crate::code_rule_dsl) struct Model {
    pub(in crate::code_rule_dsl) stats: Stats,
    pub(in crate::code_rule_dsl) ndv: HashMap<(String, usize), f64>,
    /// rel → (node column, document column) for projections of `node(d, n)`: documents are id intervals.
    docof: HashMap<String, (usize, usize)>,
    /// View over one input atom → (input, the input column behind each view column).
    view_src: HashMap<String, (String, Vec<Option<String>>)>,
    inputs: Inputs,
}

/// T's input column names; a derived relation's columns are `c0`, `c1`, ….
pub(in crate::code_rule_dsl) fn sql_col(rel: &str, col: usize) -> String {
    let names: &[&str] = match rel {
        "node" => &["d", "n", "kind", "start", "end_"],
        "doc" => &["d", "path", "module"],
        "child" => &["p", "c", "idx"],
        "field" => &["p", "f", "c"],
        "text" => &["n", "s"],
        "intval" => &["n", "v"],
        "op" => &["n", "s"],
        "pkg" => &["d", "p"],
        _ => &[],
    };
    names.get(col).map_or_else(|| format!("c{col}"), |&n| n.to_owned())
}

fn is_join(l: &Literal) -> bool {
    matches!(l, Literal::Atom(a) if !a.neg) || matches!(l, Literal::Nearest { .. })
}

/// Whether a filter can run: `$x = e` assigns $x if it is unbound when `e`'s variables are bound (SQL and the native
/// `cat`/`eq` steps bind an unbound left side); otherwise it is a test.
fn ready(l: &Literal, bound: &BTreeSet<String>) -> bool {
    match l {
        Literal::Atom(a) if a.neg => a.vars().is_subset(bound),
        Literal::Cmp { l, op, r } => cmp_binds(l, op, r, bound).is_some(),
        Literal::Agg { var, over, atom, .. } => {
            let mut need = atom.vars();
            need.remove(var);
            if let Some(o) = over {
                need.remove(o);
            }
            need.is_subset(bound)
        }
        Literal::Nearest { n, a, .. } => bound.contains(n) || bound.contains(a),
        _ => true,
    }
}

/// CPython's iteration order over a `frozenset` of small ints (the subset in `plan_rule`'s DP, whose ties go to the
/// first): sets of up to four live in eight slots, probed `i = 5i + 1`; larger ones were resized to 32 slots.
fn py_set_order(sorted: &[usize]) -> Vec<usize> {
    if sorted.len() >= 5 || sorted.iter().all(|&v| v < 8) {
        return sorted.to_vec();
    }
    let mut slots = [None; 8];
    for &v in sorted {
        let (mut i, mut perturb) = (v & 7, v);
        while slots[i].is_some() {
            perturb >>= 5;
            i = (i * 5 + 1 + perturb) & 7;
        }
        slots[i] = Some(v);
    }
    slots.into_iter().flatten().collect()
}

fn combinations(n: usize, k: usize) -> Vec<Vec<usize>> {
    let mut out = Vec::new();
    let mut idx: Vec<usize> = (0..k).collect();
    if k > n {
        return out;
    }
    loop {
        out.push(idx.clone());
        let Some(i) = (0..k).rev().find(|&i| idx[i] != i + n - k) else { return out };
        idx[i] += 1;
        for j in i + 1..k {
            idx[j] = idx[j - 1] + 1;
        }
    }
}

/// A rule body in plan order, its estimated work and its estimated output rows.
pub(in crate::code_rule_dsl) type Plan = (Vec<Step>, f64, f64);

impl Model {
    fn card(&self, rel: &str) -> f64 {
        if rel == "node" { self.stats.nodes } else { self.stats.card.get(rel).copied().unwrap_or(1000.0) }
    }

    fn ndv_or_1(&self, rel: &str, c: usize) -> f64 {
        self.ndv.get(&(rel.to_owned(), c)).copied().unwrap_or(1.0).max(1.0)
    }

    /// Orders a rule body (T's `plan_rule`). Dynamic programming over subsets of the join literals (Selinger): the
    /// cheapest order of each subset, with the filters applied as early as they can run. Work counts rows produced per
    /// step plus tape reads for `nearest` (climb: steps per row; descent: mean subtree size per row). With `first`
    /// (a body position), only orders anchored on that atom (a semi-naive delta).
    pub(in crate::code_rule_dsl) fn plan_rule(&self, prog: &Program, head: &Atom, body: &[Literal], first: Option<usize>) -> Result<Plan> {
        let stats = &self.stats;
        // var → constant kind, from kind($v, "k") atoms.
        let mut kinds: HashMap<&str, String> = HashMap::new();
        for l in body {
            if let Literal::Atom(a) = l
                && !a.neg
                && a.rel == "kind"
                && let (Term::Var(v), Some(k)) = (&a.args[0], constant(&a.args[1]))
            {
                // Lowered programs' kinds are kind atoms, and their statistics are keyed by the atoms' text.
                kinds.insert(v.as_str(), match k {
                    Constant::Str(s) => s,
                    Constant::Int(i) => i.to_string(),
                });
            }
        }
        let atom_rows_ = |a: &Atom, bound: &BTreeSet<String>| -> f64 {
            if let Some(seen) = stats.obs.get(&atom_sig(a, bound))
                && seen[0] >= OBS_MIN
            {
                return (seen[1] as f64 / seen[0] as f64).max(0.01);
            }
            if a.rel == "kind"
                && let Some(k) = a.args.get(1).and_then(constant)
            {
                if matches!(&a.args[0], Term::Var(v) if bound.contains(v)) {
                    return 1.0;
                }
                let n = match &k {
                    Constant::Str(s) => stats.kfreq.get(s).copied().unwrap_or(0),
                    Constant::Int(i) => stats.kfreq.get(&i.to_string()).copied().unwrap_or(0),
                };
                return if n == 0 { 0.5 } else { n as f64 };
            }
            let cols: Vec<usize> = (0..a.args.len())
                .filter(|&c| constant(&a.args[c]).is_some() || matches!(&a.args[c], Term::Var(v) if bound.contains(v)))
                .collect();
            if cols.is_empty() {
                return self.card(&a.rel);
            }
            // The most selective bound column gives its expected postings (exact for a constant on an input); the
            // other bound columns divide by their NDV.
            let postings = |c: usize| -> f64 {
                let k = constant(&a.args[c]);
                if let Some(k) = &k
                    && prog.inputs.contains(&a.rel)
                    && let Some(f) = stats.const_freq(&a.rel, &sql_col(&a.rel, c), k)
                {
                    return f as f64;
                }
                if let Some(k) = &k
                    && let Some(d) = stats.dfreq.get(&(a.rel.clone(), c))
                {
                    return d.get(k).copied().unwrap_or(0) as f64;
                }
                stats.fan.get(&(a.rel.clone(), c)).copied().unwrap_or_else(|| self.card(&a.rel) / self.ndv_or_1(&a.rel, c))
            };
            let mut first = cols[0];
            for &c in &cols[1..] {
                if postings(c) < postings(first) {
                    first = c;
                }
            }
            let mut rows = postings(first);
            for &c in &cols {
                if c != first {
                    rows /= self.ndv_or_1(&a.rel, c);
                }
            }
            // Every column bound: a membership test, at most one row of a set.
            if cols.len() == a.args.len() && !prog.lattice.contains_key(&a.rel) {
                rows = rows.min(1.0);
            }
            rows.max(0.01)
        };
        let atom_rows = |a: &Atom, bound: &BTreeSet<String>| -> f64 {
            let mut rows = atom_rows_(a, bound);
            // No variable bound on a relation of the component being solved: a scan of it per binding, priced as a
            // pass over the corpus (shared among its constants' values) at least.
            if stats.solving.contains(&a.rel) && !a.args.iter().any(|t| matches!(t, Term::Var(v) if bound.contains(v))) {
                let mut floor = stats.nodes;
                for (c, t) in a.args.iter().enumerate() {
                    if constant(t).is_some() {
                        floor /= self.ndv_or_1(&a.rel, c);
                    }
                }
                rows = rows.max(floor);
            }
            rows
        };
        // (column, rows) if `a` can be read as a range inside one document: a body literal `docof($v, $d)` has $d
        // bound and $v unbound, $v is `a`'s column, and an input index on `a` is the bound column then that column.
        let doc_range = |a: &Atom, bound: &BTreeSet<String>| -> Option<(usize, f64)> {
            if stats.docs.is_none() || !prog.inputs.contains(&a.rel) {
                return None;
            }
            for d in body {
                let Literal::Atom(d) = d else { continue };
                if d.neg {
                    continue;
                }
                let Some(&(nc, dc)) = self.docof.get(&d.rel) else { continue };
                let (Term::Var(v), dv) = (&d.args[nc], &d.args[dc]) else { continue };
                if bound.contains(v) || !(constant(dv).is_some() || matches!(dv, Term::Var(x) if bound.contains(x))) {
                    continue;
                }
                let rcols: Vec<usize> = (0..a.args.len()).filter(|&c| matches!(&a.args[c], Term::Var(x) if x == v)).collect();
                let bcols: Vec<usize> = (0..a.args.len())
                    .filter(|&c| constant(&a.args[c]).is_some() || matches!(&a.args[c], Term::Var(x) if bound.contains(x)))
                    .collect();
                if rcols.len() != 1 || bcols.len() != 1 {
                    continue;
                }
                let (rname, bname) = (sql_col(&a.rel, rcols[0]), sql_col(&a.rel, bcols[0]));
                if !stats.indexes.get(&a.rel).into_iter().flatten().any(|ix| ix.len() >= 2 && ix[0] == bname && ix[1] == rname) {
                    continue;
                }
                let mut rows = stats.fan_doc(&a.rel, &bname);
                if let Some(k) = constant(&a.args[bcols[0]]) {
                    let f = stats.const_freq(&a.rel, &bname, &k).filter(|&f| f != 0).map_or(rows, |f| f as f64);
                    rows = rows.min(f);
                }
                return Some((rcols[0], rows));
            }
            None
        };
        // (work, rows out, mode) of adding join literal `l`, or None if it can't run yet.
        let step = |l: &Literal, rows: f64, bound: &BTreeSet<String>| -> Option<(f64, f64, Option<Mode>)> {
            match l {
                Literal::Atom(a) => {
                    let out = rows * atom_rows(a, bound);
                    if let Some((_, r)) = doc_range(a, bound)
                        && rows * r < out
                    {
                        let out = rows * r;
                        return Some((out + rows, out, Some(Mode::Range)));
                    }
                    Some((out + rows, out, None))
                }
                Literal::Nearest { n, a, kindset } => {
                    if !ready(l, bound) {
                        return None;
                    }
                    if bound.contains(n) {
                        // Climb from $n: one parent read per expected step; a bound $a just filters.
                        return Some((rows * stats.steps(kinds.get(n.as_str()).map(String::as_str), kindset), rows, Some(Mode::Climb)));
                    }
                    // Descend from $a: every node walked.
                    let size = kinds.get(a.as_str()).and_then(|k| stats.size.get(k)).copied().unwrap_or(stats.nodes / 1000.0);
                    Some((rows * size, rows * size, Some(Mode::Descend)))
                }
                _ => None,
            }
        };
        // Applies every filter (negation, comparison, aggregate) whose variables are bound.
        let close = |order: &mut Vec<Step>, mut rows: f64, bound: &mut BTreeSet<String>, rest: &mut Vec<usize>| -> f64 {
            let mut progressed = true;
            while progressed {
                progressed = false;
                for i in rest.clone() {
                    let l = &body[i];
                    if !is_join(l) && ready(l, bound) {
                        order.push(Step { at: i, lit: l.clone(), mode: None });
                        rest.retain(|&j| j != i);
                        bound.extend(l.vars());
                        progressed = true;
                        if !(matches!(l, Literal::Cmp { op: "=", .. }) || matches!(l, Literal::Agg { .. })) {
                            rows *= 0.5;
                        }
                    }
                }
            }
            rows
        };
        if let Some(l) = body.iter().find(|l| matches!(l, Literal::Ast { .. })) {
            return Err(Error::Plan(format!("rule for {head}: no executor for {l} yet")));
        }
        let joins: Vec<usize> = (0..body.len()).filter(|&i| is_join(&body[i])).collect();
        if !joins.iter().any(|&i| matches!(body[i], Literal::Atom(_))) {
            return Err(Error::Plan(format!("rule for {head} has no positive atom")));
        }
        type Best = (f64, f64, Vec<Step>, BTreeSet<String>, Vec<usize>);
        let mut best_of: HashMap<u64, Best> = HashMap::new();
        best_of.insert(0, (0.0, 1.0, Vec::new(), BTreeSet::new(), (0..body.len()).filter(|&i| !is_join(&body[i])).collect()));
        for size in 1..=joins.len() {
            for subset in combinations(joins.len(), size) {
                let mask: u64 = subset.iter().map(|&i| 1u64 << i).sum();
                let mut best: Option<Best> = None;
                for i in py_set_order(&subset) {
                    let Some((work, rows, order, bound, rest)) = best_of.get(&(mask & !(1 << i))) else { continue };
                    let l = &body[joins[i]];
                    if order.is_empty() && !matches!(l, Literal::Atom(_)) {
                        continue; // an anchor is an atom
                    }
                    if order.is_empty() && first.is_some_and(|f| f != joins[i]) {
                        continue; // semi-naive: the delta first
                    }
                    let Some((w, out, mode)) = step(l, *rows, bound) else { continue };
                    let (mut order2, mut bound2, mut rest2) = (order.clone(), bound.clone(), rest.clone());
                    order2.push(Step { at: joins[i], lit: l.clone(), mode });
                    bound2.extend(l.vars());
                    let out = close(&mut order2, out, &mut bound2, &mut rest2);
                    let cand = (work + w, out, order2, bound2, rest2);
                    if best.as_ref().is_none_or(|b| cand.0 < b.0) {
                        best = Some(cand);
                    }
                }
                if let Some(b) = best {
                    best_of.insert(mask, b);
                }
            }
        }
        let full: u64 = (0..joins.len()).map(|i| 1u64 << i).sum();
        let Some((work, rows, order, _, rest)) = best_of.remove(&full) else {
            return Err(Error::Plan(format!("cannot order the body of {head}")));
        };
        if !rest.is_empty() {
            let rest: Vec<String> = rest.iter().map(|&i| body[i].to_string()).collect();
            return Err(Error::Plan(format!("unplaced literals {rest:?} in {head}")));
        }
        Ok((order, work, rows))
    }

    /// A view over one input atom takes its statistics from the input's: a constant filters by its exact postings
    /// fraction, and each column keeps the input column's NDV (at most the view's rows) and fanout (scaled by the
    /// filter, assuming independence).
    fn view_stats(&mut self, rel: &str, h: &Atom, a: &Atom) {
        let base = self.stats.card[&a.rel];
        let mut rows = base;
        for (c, t) in a.args.iter().enumerate() {
            if let Some(k) = constant(t) {
                rows *= self.stats.const_freq(&a.rel, &sql_col(&a.rel, c), &k).unwrap_or(0) as f64 / base.max(1.0);
            }
        }
        let card = rows.round_ties_even().max(1.0);
        self.stats.card.insert(rel.to_owned(), card);
        let src = first_columns(a);
        for (c, t) in h.args.iter().enumerate() {
            let Term::Var(v) = t else { continue };
            let Some(&bc) = src.get(v.as_str()) else { continue };
            let n = self.ndv[&(a.rel.clone(), bc)].min(card).max(1.0);
            self.ndv.insert((rel.to_owned(), c), n);
            let fan = (self.stats.fan[&(a.rel.clone(), bc)] * rows / base.max(1.0)).max(1.0);
            self.stats.fan.insert((rel.to_owned(), c), fan);
        }
    }
}

/// Each variable's first column in an atom.
fn first_columns(a: &Atom) -> HashMap<&str, usize> {
    let mut src = HashMap::new();
    for (c, t) in a.args.iter().enumerate().rev() {
        if let Term::Var(v) = t {
            src.insert(v.as_str(), c);
        }
    }
    src
}

/// Reads the statistics and the input relations' cardinalities, distinct counts, fanouts and indexes.
fn make_plan(prog: &Program, db: &Connection, inputs: Inputs) -> Result<Model> {
    let mut stats = Stats::new(db, prog, &inputs)?;
    let mut ndv = HashMap::new();
    for rel in &prog.inputs {
        for col in 0..prog.decls[rel].len() {
            let name = sql_col(rel, col);
            let Some(&[rows, n, sq, _]) = stats.col.get(&(rel.clone(), name.clone())) else {
                return Err(Error::Statistics(format!("no statistics for {rel}.{name}")));
            };
            stats.card.insert(rel.clone(), rows as f64);
            ndv.insert((rel.clone(), col), if n == 0 { 1.0 } else { n as f64 });
            stats.fan.insert((rel.clone(), col), if rows != 0 { sq as f64 / rows as f64 } else { 1.0 });
        }
    }
    // Input indexes as column lists, each ending in the rowid column if the table has one (SQLite appends it). The
    // vtabs answer the same probes as T's tables' indexes (`facts::vtab::syntax`), so they are listed as T's.
    if inputs.atoms.is_some() {
        for rel in &prog.inputs {
            stats.indexes.insert(rel.clone(), vtab_indexes(rel));
        }
        return Ok(Model { stats, ndv, docof: HashMap::new(), view_src: HashMap::new(), inputs });
    }
    for rel in &prog.inputs {
        let mut st = db.prepare(&format!("PRAGMA f.table_info({rel})"))?;
        let rowid: Vec<String> = st
            .query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(5)?)))?
            .filter_map(|r| r.ok())
            .filter(|(_, ty, pk)| *pk == 1 && ty.to_uppercase() == "INTEGER")
            .map(|(n, _, _)| n)
            .collect();
        let mut indexes = if rowid.is_empty() { Vec::new() } else { vec![rowid.clone()] };
        let names: Vec<String> =
            db.prepare(&format!("PRAGMA f.index_list({rel})"))?.query_map([], |r| r.get(1))?.collect::<rusqlite::Result<_>>()?;
        for name in names {
            let mut cols: Vec<String> =
                db.prepare(&format!("PRAGMA f.index_info({name})"))?.query_map([], |r| r.get(2))?.collect::<rusqlite::Result<_>>()?;
            cols.extend(rowid.iter().cloned());
            indexes.push(cols);
        }
        stats.indexes.insert(rel.clone(), indexes);
    }
    Ok(Model { stats, ndv, docof: HashMap::new(), view_src: HashMap::new(), inputs })
}

/// T's facts file's indexes, which the base-relation vtabs' lookups mirror.
fn vtab_indexes(rel: &str) -> Vec<Vec<String>> {
    let lists: &[&[&str]] = match rel {
        "node" => &[&["n"], &["kind", "n"]],
        "child" => &[&["c"], &["p", "idx"]],
        "field" => &[&["p", "f"], &["c"]],
        "text" => &[&["n"], &["s", "n"]],
        "intval" | "op" | "tape" => &[&["n"]],
        "doc" | "pkg" => &[&["d"]],
        _ => &[],
    };
    lists.iter().map(|l| l.iter().map(|c| (*c).to_owned()).collect()).collect()
}

// ---- SQL backend -------------------------------------------------------------------------------------------------

fn sql_term(t: &Term, env: &HashMap<String, String>, atoms: Option<[i64; 3]>) -> Result<String> {
    Ok(match t {
        Term::Var(v) => env.get(v).cloned().ok_or_else(|| Error::Plan(format!("${v} is unbound in SQL")))?,
        Term::Int(i) => i.to_string(),
        Term::Str(s) => format!("'{}'", s.replace('\'', "''")),
        Term::Top => TOP.to_string(),
        Term::Call(f, args) => {
            let a = args.iter().map(|x| sql_term(x, env, atoms)).collect::<Result<Vec<_>>>()?;
            match (f.as_str(), a.as_slice()) {
                ("cat", [x, y]) if atoms.is_some() => format!("sym_cat({x}, {y})"),
                ("cat", [x, y]) => format!("({x} || '.' || {y})"),
                ("flat_bin", [o, x, y]) => {
                    let big = 1i64 << 40;
                    let [plus, minus, times] = match atoms {
                        Some(ops) => ops.map(|o| o.to_string()),
                        None => ["'+'", "'-'", "'*'"].map(str::to_owned),
                    };
                    format!(
                        "(CASE WHEN {x} = {TOP} OR {y} = {TOP} THEN {TOP} WHEN {o} = {plus} THEN {x} + {y} WHEN {o} = {minus} THEN {x} - {y} \
                         WHEN {o} = {times} AND abs({x}) < {big} AND abs({y}) < {big} THEN {x} * {y} ELSE {TOP} END)"
                    )
                }
                _ => return Err(Error::Plan(format!("no SQL for {t}"))),
            }
        }
        Term::Wild => return Err(Error::Plan("no SQL for _".to_owned())),
    })
}

impl Model {
    fn table(&self, prog: &Program, rel: &str, which: &str) -> String {
        if prog.inputs.iter().any(|i| i == rel) { format!("{}{rel}", self.inputs.facts) } else { format!("{which}{rel}") }
    }

    /// One SELECT for a planned rule body. `delta_at` is the plan position of the recursive atom that reads its delta
    /// table. `tables` maps a relation to the (full, delta) tables to read it from instead (the native set fixpoint's
    /// vtabs).
    pub(in crate::code_rule_dsl) fn sql_select(
        &self,
        prog: &Program,
        head: &Atom,
        order: &[Step],
        delta_at: Option<usize>,
        tables: Option<&HashMap<String, (String, String)>>,
    ) -> Result<String> {
        let (mut env, mut frm, mut wh, mut ctes) = (HashMap::<String, String>::new(), Vec::<(String, Vec<String>)>::new(), Vec::new(), Vec::new());
        let mut alias = 0;
        let mut a_name = || {
            alias += 1;
            format!("t{alias}")
        };
        let neg_conds = |a: &Atom, al: &str, env: &HashMap<String, String>, skip: Option<&str>| -> Result<Vec<String>> {
            a.args
                .iter()
                .enumerate()
                .filter(|(_, t)| !matches!(t, Term::Wild) && !matches!(t, Term::Var(v) if Some(v.as_str()) == skip))
                .map(|(c, t)| Ok(format!("{al}.{} = {}", sql_col(&a.rel, c), sql_term(t, env, self.inputs.atoms)?)))
                .collect()
        };
        for (idx, s) in order.iter().enumerate() {
            match &s.lit {
                Literal::Atom(l) if !l.neg => {
                    let al = a_name();
                    let src = match tables.and_then(|t| t.get(&l.rel)) {
                        Some((full, delta)) => if Some(idx) == delta_at { delta.clone() } else { full.clone() },
                        None => self.table(prog, &l.rel, if Some(idx) == delta_at { "delta_" } else { "" }),
                    };
                    let mut conds = Vec::new();
                    if s.mode == Some(Mode::Range) {
                        // docof($v, $d) with $d bound: $v lies in $d's node-id interval.
                        let lv = l.vars();
                        let found = order.iter().find_map(|o| {
                            let Literal::Atom(d) = &o.lit else { return None };
                            let &(nc, dc) = self.docof.get(&d.rel)?;
                            let Term::Var(v) = &d.args[nc] else { return None };
                            let dv = &d.args[dc];
                            (lv.contains(v) && (constant(dv).is_some() || matches!(dv, Term::Var(x) if env.contains_key(x))))
                                .then(|| (v.clone(), dv.clone()))
                        });
                        let (v, dv) = found.ok_or_else(|| Error::Plan(format!("no document for the range read of {l}")))?;
                        let ra = a_name();
                        frm.push((format!("{}stat_docrange {ra}", self.inputs.stats), vec![format!("{ra}.d = {}", sql_term(&dv, &env, self.inputs.atoms)?)]));
                        let rc = l.args.iter().position(|t| matches!(t, Term::Var(x) if *x == v)).expect("the range column");
                        conds.push(format!("{al}.{} BETWEEN {ra}.lo AND {ra}.hi", sql_col(&l.rel, rc)));
                    }
                    for (col, t) in l.args.iter().enumerate() {
                        let c = format!("{al}.{}", sql_col(&l.rel, col));
                        match t {
                            Term::Var(v) => match env.get(v) {
                                Some(e) => conds.push(format!("{c} = {e}")),
                                None => {
                                    env.insert(v.clone(), c);
                                }
                            },
                            Term::Int(_) | Term::Str(_) | Term::Top => conds.push(format!("{c} = {}", sql_term(t, &env, self.inputs.atoms)?)),
                            _ => {}
                        }
                    }
                    frm.push((format!("{src} {al}"), conds));
                }
                Literal::Atom(l) => {
                    let al = a_name();
                    let conds = neg_conds(l, &al, &env, None)?;
                    let conds = if conds.is_empty() { "1".to_owned() } else { conds.join(" AND ") };
                    wh.push(format!("NOT EXISTS (SELECT 1 FROM {} {al} WHERE {conds})", self.table(prog, &l.rel, "")));
                }
                Literal::Cmp { l, op, r } => {
                    let lv = if let Term::Var(v) = l { Some(v) } else { None };
                    if *op == "=" && lv.is_some_and(|v| !env.contains_key(v)) {
                        let e = sql_term(r, &env, self.inputs.atoms)?;
                        env.insert(lv.expect("a variable").clone(), e);
                    } else if *op == "="
                        && let Some(lv) = lv
                        && r.is_cat()
                        && !{
                            let mut rv = BTreeSet::new();
                            r.vars(&mut rv);
                            rv.iter().all(|v| env.contains_key(v))
                        }
                    {
                        // A split of the bound whole: at the "." a bound part places, else at every "." (`rdl_int`).
                        // In atoms mode the text is `sym_text`'s, and the parts are interned (`sym_atom`).
                        let inputs = &self.inputs;
                        let w = inputs.text(&env[lv]);
                        let Term::Call(_, parts) = r else { unreachable!() };
                        let (Term::Var(q), Term::Var(x)) = (&parts[0], &parts[1]) else { unreachable!() };
                        if let Some(eq) = env.get(q).map(|e| inputs.text(e)) {
                            wh.push(format!("substr({w}, 1, length({eq}) + 1) = {eq} || '.'"));
                            env.insert(x.clone(), inputs.symbol(&format!("substr({w}, length({eq}) + 2)")));
                        } else if let Some(ex) = env.get(x).map(|e| inputs.text(e)) {
                            wh.push(format!("substr({w}, -length({ex}) - 1) = '.' || {ex} AND length({w}) > length({ex})"));
                            env.insert(q.clone(), inputs.symbol(&format!("substr({w}, 1, length({w}) - length({ex}) - 1)")));
                        } else {
                            let al = a_name();
                            frm.push((format!("rdl_int {al}"), vec![format!("{al}.i <= length({w})"), format!("substr({w}, {al}.i, 1) = '.'")]));
                            env.insert(q.clone(), inputs.symbol(&format!("substr({w}, 1, {al}.i - 1)")));
                            env.insert(x.clone(), inputs.symbol(&format!("substr({w}, {al}.i + 1)")));
                        }
                    } else {
                        let op = match *op {
                            "!=" => "<>",
                            o => o,
                        };
                        wh.push(format!("{} {op} {}", sql_term(l, &env, self.inputs.atoms)?, sql_term(r, &env, self.inputs.atoms)?));
                    }
                }
                Literal::Agg { var, op, over, atom } => {
                    let al = a_name();
                    let conds = neg_conds(atom, &al, &env, over.as_deref())?;
                    let conds = if conds.is_empty() { "1".to_owned() } else { conds.join(" AND ") };
                    let what = match (op, over) {
                        (AggOp::Count, _) | (_, None) => "count(*)".to_owned(),
                        (AggOp::Min | AggOp::Max, Some(o)) => {
                            let c = atom.args.iter().position(|t| matches!(t, Term::Var(v) if v == o)).expect("the aggregated column");
                            format!("{}({al}.{})", if *op == AggOp::Min { "min" } else { "max" }, sql_col(&atom.rel, c))
                        }
                    };
                    env.insert(var.clone(), format!("(SELECT {what} FROM {} {al} WHERE {conds})", self.table(prog, &atom.rel, "")));
                }
                Literal::Nearest { n, a, kindset } if s.mode == Some(Mode::Climb) && self.inputs.native.contains("nearest") => {
                    // The climb is a native function over the tape: no CTE, and its result is just a column expression.
                    let near = format!("tape_nearest({}, '{}')", env[n], prog.kindsets[kindset].join(","));
                    if let Some(e) = env.get(a) {
                        wh.push(format!("{near} = {e}"));
                    } else {
                        env.insert(a.clone(), near.clone());
                        wh.push(format!("{near} IS NOT NULL"));
                    }
                }
                Literal::Nearest { n, a, kindset } => {
                    // A lowered program's members are kind atoms.
                    let ks = prog.kindsets[kindset]
                        .iter()
                        .map(|k| if k.parse::<i64>().is_ok() { k.clone() } else { format!("'{k}'") })
                        .collect::<Vec<_>>()
                        .join(", ");
                    let f = &self.inputs.facts;
                    // Everything joined so far becomes the seed CTE; the walk is a recursive CTE seeded only from it.
                    let k = ctes.len();
                    let seed_cols: Vec<String> = env.keys().cloned().collect::<BTreeSet<_>>().into_iter().collect();
                    let seed_sel = seed_cols.iter().map(|v| format!("{} AS {v}", env[v])).collect::<Vec<_>>().join(", ");
                    let seed_from = match frm.split_first() {
                        Some((f, rest)) => f.0.clone() + &rest.iter().map(|(f, _)| format!(" CROSS JOIN {f}")).collect::<String>(),
                        None => String::new(),
                    };
                    let conds: Vec<String> = frm.iter().flat_map(|(_, c)| c.iter().cloned()).chain(wh.iter().cloned()).collect();
                    let seed_where = if conds.is_empty() { "1".to_owned() } else { conds.join(" AND ") };
                    ctes.push(format!("seed{k} AS (SELECT DISTINCT {seed_sel} FROM {seed_from} WHERE {seed_where})"));
                    // The seed's columns are threaded through the walk (ast-index-spike S050 `_path_threaded`), so the
                    // walk's output is the extended row and nothing joins back to the seed.
                    let cols = seed_cols.join(", ");
                    let wcols = seed_cols.iter().map(|v| format!("w.{v}")).collect::<Vec<_>>().join(", ");
                    let scols = seed_cols.iter().map(|v| format!("s.{v}")).collect::<Vec<_>>().join(", ");
                    let out_var = if s.mode == Some(Mode::Climb) {
                        ctes.push(format!(
                            "walk{k}({cols}, cur) AS (SELECT {scols}, t.par FROM seed{k} s JOIN {f}tape t ON t.n = s.{n} WHERE t.par >= 0 \
                             UNION ALL SELECT {wcols}, t.par FROM walk{k} w JOIN {f}node k ON k.n = w.cur AND k.kind NOT IN ({ks}) \
                             JOIN {f}tape t ON t.n = w.cur WHERE t.par >= 0)"
                        ));
                        let cond = if env.contains_key(a) { format!(" AND w.cur = w.{a}") } else { String::new() };
                        ctes.push(format!(
                            "near{k} AS (SELECT {wcols}, w.cur AS near_out FROM walk{k} w CROSS JOIN {f}node k WHERE k.n = w.cur AND k.kind IN ({ks}){cond})"
                        ));
                        a
                    } else {
                        ctes.push(format!(
                            "walk{k}({cols}, cur, stop) AS (SELECT {scols}, s.{a} + 1, s.{a} + t.size FROM seed{k} s JOIN {f}tape t ON t.n = s.{a} WHERE t.size > 0 \
                             UNION ALL SELECT {wcols}, CASE WHEN k.kind IN ({ks}) THEN w.cur + t.size + 1 ELSE w.cur + 1 END, w.stop \
                             FROM walk{k} w JOIN {f}node k ON k.n = w.cur JOIN {f}tape t ON t.n = w.cur WHERE w.cur < w.stop)"
                        ));
                        let cond = if env.contains_key(n) { format!(" AND w.cur = w.{n}") } else { String::new() };
                        ctes.push(format!("near{k} AS (SELECT {wcols}, w.cur AS near_out FROM walk{k} w WHERE w.cur <= w.stop{cond})"));
                        n
                    };
                    env = seed_cols.iter().map(|v| (v.clone(), format!("near{k}.{v}"))).collect();
                    env.entry(out_var.clone()).or_insert_with(|| format!("near{k}.near_out"));
                    frm = vec![(format!("near{k}"), Vec::new())];
                    wh.clear();
                }
                Literal::Ast { .. } => return Err(Error::Plan(format!("no SQL for {}", s.lit))),
            }
        }
        let sel = head.args.iter().enumerate().map(|(i, t)| Ok(format!("{} AS c{i}", sql_term(t, &env, self.inputs.atoms)?))).collect::<Result<Vec<_>>>()?.join(", ");
        let (f0, rest) = frm.split_first().ok_or_else(|| Error::Plan(format!("rule for {head} reads nothing")))?;
        let from_sql = f0.0.clone() + &rest.iter().map(|(f, _)| format!(" CROSS JOIN {f}")).collect::<String>();
        let conds: Vec<String> = frm.iter().flat_map(|(_, c)| c.iter().cloned()).chain(wh).collect();
        let w = if conds.is_empty() { "1".to_owned() } else { conds.join(" AND ") };
        let with_sql = if ctes.is_empty() { String::new() } else { format!("WITH RECURSIVE {} ", ctes.join(", ")) };
        Ok(format!("{with_sql}SELECT DISTINCT {sel} FROM {from_sql} WHERE {w}"))
    }
}

/// A rule's chosen plan, as T's `plan.json` lists it.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::code_rule_dsl) struct Chosen {
    /// The rule's index in the program, and its body positions in plan order with their modes.
    pub(in crate::code_rule_dsl) rule: usize,
    pub(in crate::code_rule_dsl) steps: Vec<(usize, Option<Mode>)>,
    pub(in crate::code_rule_dsl) head: String,
    pub(in crate::code_rule_dsl) anchor: String,
    pub(in crate::code_rule_dsl) walks: Vec<(String, &'static str)>,
    pub(in crate::code_rule_dsl) work: i64,
}

/// What a run derived: the chosen plans, each output's rows (sorted, tab-separated), and the rounds per recursive
/// component.
#[derive(Debug, Default)]
pub(in crate::code_rule_dsl) struct Outcome {
    pub(in crate::code_rule_dsl) chosen: Vec<Chosen>,
    pub(in crate::code_rule_dsl) outputs: BTreeMap<String, Vec<String>>,
    pub(in crate::code_rule_dsl) iterations: BTreeMap<String, usize>,
    /// The components a native mode ran (T's `iterations` value `"native"`).
    pub(in crate::code_rule_dsl) native: BTreeSet<String>,
    /// Each output's node columns (atoms mode: node ids are the vtabs', `d << 32 | index`).
    pub(in crate::code_rule_dsl) nodes: BTreeMap<String, Vec<usize>>,
}

/// A run's evaluation state on its connection.
struct Run<'a> {
    prog: &'a Program,
    db: &'a Connection,
    model: Model,
    views: HashSet<String>,
    made: HashSet<(String, Vec<usize>)>,
    /// Relations held only by the native set engine (`program`) until `materialize`.
    native_rels: BTreeSet<String>,
}

/// A recursive atom's delta variant: the rule's head, its plan with that atom first, and the atom's plan position.
type Delta = (Atom, Vec<Step>, usize);

impl Run<'_> {
    /// A composite index on each derived relation for every bound-column pattern the plan probes it with.
    fn ensure_indexes(&mut self, order: &[Step]) -> Result<()> {
        let mut bound = BTreeSet::new();
        for s in order {
            let a = match &s.lit {
                Literal::Atom(a) | Literal::Agg { atom: a, .. } => Some(a),
                _ => None,
            };
            if let Some(a) = a
                && self.views.contains(&a.rel)
                && let Some((base, cols)) = self.model.view_src.get(&a.rel)
            {
                // A view over an input is probed through the input's indexes. If none serves this probe, SQLite
                // builds an automatic index per statement, which for a filtered view scans every node, every round.
                // Materialize it once instead; it then gets indexes like any derived relation.
                let probed: HashSet<&String> = a
                    .args
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| constant(t).is_some() || matches!(t, Term::Var(v) if bound.contains(v)))
                    .filter_map(|(c, _)| cols[c].as_ref())
                    .collect();
                let served = self.model.stats.indexes.get(base).into_iter().flatten().any(|ix| ix.first().is_some_and(|c| probed.contains(c)));
                if !probed.is_empty() && !served {
                    let r = &a.rel;
                    self.db.execute_batch(&format!(
                        "CREATE TEMP TABLE {r}__m AS SELECT * FROM {r}; DROP VIEW {r}; ALTER TABLE {r}__m RENAME TO {r};"
                    ))?;
                    self.views.remove(r);
                    self.model.view_src.remove(r);
                }
            }
            if let Some(a) = a
                && !self.prog.inputs.contains(&a.rel)
                && !self.views.contains(&a.rel)
            {
                let cols: Vec<usize> = (0..a.args.len())
                    .filter(|&c| constant(&a.args[c]).is_some() || matches!(&a.args[c], Term::Var(v) if bound.contains(v)))
                    .collect();
                if !cols.is_empty() && self.made.insert((a.rel.clone(), cols.clone())) {
                    let name = format!("{}_{}", a.rel, cols.iter().map(usize::to_string).collect::<Vec<_>>().join("_"));
                    let on = cols.iter().map(|c| format!("c{c}")).collect::<Vec<_>>().join(", ");
                    self.db.execute(&format!("CREATE INDEX IF NOT EXISTS {name} ON {}({on})", a.rel), [])?;
                }
            }
            bound.extend(s.lit.vars());
        }
        Ok(())
    }

    fn count(&self, sql: &str) -> Result<i64> {
        Ok(self.db.query_row(sql, [], |r| r.get(0))?)
    }

    fn insert_rows(&self, head: &Atom, select: &str) -> Result<()> {
        if self.prog.lattice.contains_key(&head.rel) {
            self.merge_lattice(&head.rel, select, false)?;
        } else {
            self.db.execute(&format!("INSERT OR IGNORE INTO {} {select}", head.rel), [])?;
        }
        Ok(())
    }

    /// Joins candidate values per key (flat lattice: equal stays, different becomes top) into `rel`; with `delta`,
    /// records the keys whose value changed in `new_<rel>`. Returns the number of changed keys.
    fn merge_lattice(&self, rel: &str, select: &str, delta: bool) -> Result<i64> {
        if self.model.inputs.native.contains("upsert") {
            return self.merge_upsert(rel, select, delta);
        }
        let k = self.prog.lattice[rel].keys;
        let keys = (0..k).map(|i| format!("c{i}")).collect::<Vec<_>>().join(", ");
        let v = format!("c{k}");
        let on = (0..k).map(|i| format!("r.c{i} = j.c{i}")).collect::<Vec<_>>().join(" AND ");
        let joined = format!(
            "SELECT {keys}, CASE WHEN count(DISTINCT {v}) > 1 OR max({v}) = {TOP} THEN {TOP} ELSE min({v}) END AS {v} FROM cand GROUP BY {keys}"
        );
        let changed = format!("SELECT j.* FROM ({joined}) j LEFT JOIN {rel} r ON {on} WHERE r.{v} IS NULL OR (r.{v} <> {TOP} AND r.{v} <> j.{v})");
        let same = (0..k).map(|i| format!("r.c{i} = chg.c{i}")).collect::<Vec<_>>().join(" AND ");
        self.db.execute_batch(&format!(
            "DROP TABLE IF EXISTS cand; CREATE TEMP TABLE cand AS SELECT * FROM ({select}) WHERE 0; INSERT INTO cand {select};
             DROP TABLE IF EXISTS chg; CREATE TEMP TABLE chg AS {changed};
             UPDATE chg SET {v} = {TOP} WHERE EXISTS (SELECT 1 FROM {rel} r WHERE {same} AND r.{v} <> chg.{v});
             INSERT OR REPLACE INTO {rel} SELECT * FROM chg;"
        ))?;
        let n = self.count("SELECT count(*) FROM chg")?;
        if delta {
            self.db.execute(&format!("INSERT INTO new_{rel} SELECT * FROM chg"), [])?;
        }
        Ok(n)
    }

    /// One round: every rule over the full relations (`first`), else each delta variant of `deltas`, or of `planned`
    /// with its one order.
    fn semi_naive_round(&self, scc: &BTreeSet<String>, planned: &[(Atom, Vec<Step>)], first: bool, deltas: &[Delta]) -> Result<i64> {
        for r in scc {
            self.db.execute_batch(&format!("DROP TABLE IF EXISTS new_{r}; CREATE TEMP TABLE new_{r} AS SELECT * FROM {r} WHERE 0;"))?;
        }
        let rec_idx =
            |order: &[Step]| -> Vec<usize> { (0..order.len()).filter(|&i| matches!(&order[i].lit, Literal::Atom(a) if !a.neg && scc.contains(&a.rel))).collect() };
        let variants: Vec<(&Atom, &[Step], Vec<usize>)> = if !deltas.is_empty() && !first {
            deltas.iter().map(|(h, o, i)| (h, o.as_slice(), vec![*i])).collect()
        } else {
            planned.iter().map(|(h, o)| (h, o.as_slice(), rec_idx(o))).collect()
        };
        let mut by_head: IndexMap<&str, Vec<String>> = IndexMap::new();
        for (h, order, idx) in variants {
            let selects = by_head.entry(h.rel.as_str()).or_default();
            if first {
                selects.push(self.model.sql_select(self.prog, h, order, None, None)?);
            } else {
                // One variant per recursive atom, that atom reading the previous round's delta.
                for i in idx {
                    selects.push(self.model.sql_select(self.prog, h, order, Some(i), None)?);
                }
            }
        }
        let mut total = 0;
        for (rel, selects) in by_head {
            let union = selects.iter().map(|s| format!("SELECT * FROM ({s})")).collect::<Vec<_>>().join(" UNION ALL ");
            if self.prog.lattice.contains_key(rel) {
                total += self.merge_lattice(rel, &union, true)?;
            } else {
                self.db.execute_batch(&format!(
                    "DROP TABLE IF EXISTS cand; CREATE TEMP TABLE cand AS {union};
                     INSERT INTO new_{rel} SELECT * FROM cand EXCEPT SELECT * FROM {rel};"
                ))?;
                total += self.count(&format!("SELECT count(*) FROM new_{rel}"))?;
                self.db.execute(&format!("INSERT OR IGNORE INTO {rel} SELECT * FROM new_{rel}"), [])?;
            }
        }
        for r in scc {
            self.db.execute_batch(&format!("DROP TABLE IF EXISTS delta_{r}; ALTER TABLE new_{r} RENAME TO delta_{r};"))?;
        }
        Ok(total)
    }

    fn refresh(&mut self, r: &str, at_least_one: bool) -> Result<()> {
        let n = self.count(&format!("SELECT count(*) FROM {r}"))? as f64;
        self.model.stats.card.insert(r.to_owned(), if at_least_one { n.max(1.0) } else { n });
        Ok(())
    }
}

/// Adds a native join's step counts ("in:out" per step, `set_fixpoint_steps`) to the observed fanouts of the binding
/// patterns its positive atoms were read with (T's `observe`). The first step (the delta, or a seed's anchor) is
/// skipped.
fn observe(stats: &mut Stats, order: &[Step], line: &str) {
    if line.is_empty() {
        return;
    }
    let counts: Vec<(i64, i64)> = line
        .split(',')
        .filter_map(|x| {
            let (i, o) = x.split_once(':')?;
            Some((i.parse().ok()?, o.parse().ok()?))
        })
        .collect();
    let mut bound = BTreeSet::new();
    for (i, (s, &(n_in, n_out))) in order.iter().zip(&counts).enumerate() {
        if i > 0
            && let Literal::Atom(l) = &s.lit
            && !l.neg
            && n_in != 0
        {
            let o = stats.obs.entry(atom_sig(l, &bound)).or_insert([0, 0]);
            o[0] += n_in;
            o[1] += n_out;
        }
        if !matches!(&s.lit, Literal::Atom(l) if l.neg) {
            bound.extend(s.lit.vars());
        }
    }
}

/// A relation's column types (T's `prog_types`).
fn prog_types<'p>(prog: &'p Program, r: &str) -> Vec<&'p str> {
    prog.decls[r].iter().map(|(_, t)| t.as_str()).collect()
}

/// A relation's statistics from the native store (`set_fixpoint_stats`, T's `live_stats`): rows, then per column
/// the distinct count and the sum of squared postings, whose ratio to rows is the skew-weighted fanout ingestion
/// gives inputs.
fn live_stats(prog: &Program, model: &mut Model, r: &str, line: &str) {
    let mut parts = line.split(" |");
    let nums: Vec<i64> = parts.next().unwrap_or("").split_whitespace().filter_map(|x| x.parse().ok()).collect();
    let Some((&n, rest)) = nums.split_first() else { return };
    let k = rest.len() / 2;
    model.stats.card.insert(r.to_owned(), (n as f64).max(1.0));
    for c in 0..k {
        model.ndv.insert((r.to_owned(), c), rest[c].max(1) as f64);
        model.stats.fan.insert((r.to_owned(), c), if n != 0 { rest[k + c] as f64 / n as f64 } else { 1.0 });
        model.stats.dfreq.remove(&(r.to_owned(), c));
    }
    let types = prog_types(prog, r);
    for f in parts {
        let Some((c, vals)) = f.trim().split_once(':') else { continue };
        let Ok(c) = c.parse::<usize>() else { continue };
        let freq = vals
            .split(',')
            .filter(|x| !x.is_empty())
            .filter_map(|x| {
                let (v, m) = x.split_once('=')?;
                let v = unhex(v)?;
                let v = if types.get(c) == Some(&"sym") { Constant::Str(v) } else { Constant::Int(v.parse().ok()?) };
                Some((v, m.parse().ok()?))
            })
            .collect();
        model.stats.dfreq.insert((r.to_owned(), c), freq);
    }
}

/// A plan's step strings, which say whether a replan changed it.
fn shown(order: &[Step]) -> Vec<String> {
    order.iter().map(|s| s.lit.to_string()).collect()
}

/// Positive body atoms of `scc`, by body position.
fn recursive<'r>(body: &'r [Literal], scc: &BTreeSet<String>) -> Vec<(usize, &'r Atom)> {
    body.iter()
        .enumerate()
        .filter_map(|(i, l)| match l {
            Literal::Atom(a) if !a.neg && scc.contains(&a.rel) => Some((i, a)),
            _ => None,
        })
        .collect()
}

fn atom_of(rel: String, args: Vec<Term>) -> Atom {
    Atom { rel, args, neg: false, gated: false }
}

/// A native spec's token for a constant.
fn const_tok(t: &Term) -> Option<String> {
    match t {
        Term::Int(i) => Some(format!("i:{i}")),
        Term::Str(s) => Some(format!("x:{}", hex(s))),
        _ => None,
    }
}

impl Run<'_> {
    /// Moves the native set engine's relations among `rels` into their tables (`set_fixpoint_materialize`).
    fn materialize(&mut self, rels: &[String]) -> Result<()> {
        let held: BTreeSet<String> = rels.iter().filter(|r| self.native_rels.contains(*r)).cloned().collect();
        for r in held {
            self.db.query_row("SELECT set_fixpoint_materialize(?1)", [&r], |_| Ok(()))?;
            self.native_rels.remove(&r);
        }
        Ok(())
    }

    /// A probe's planned body as a native join spec (sqlite-ext `setfix.rs`, "native join probes", T's `join_spec`),
    /// or `None` where a literal has no native step (an aggregate other than `count`, a descent, a document range,
    /// top): that probe stays SQL.
    fn join_spec(
        &self,
        h: &Atom,
        order: &[Step],
        at: Option<usize>,
        ri: &BTreeMap<String, usize>,
        scc: &BTreeSet<String>,
        native_rels: &BTreeSet<String>,
    ) -> Option<String> {
        let prog = self.prog;
        let mut slots: HashMap<String, usize> = HashMap::new();
        let typ = |t: &Term| -> Option<String> {
            match t {
                Term::Int(_) => Some("int".to_owned()),
                Term::Str(_) => Some("sym".to_owned()),
                Term::Var(n) => order.iter().find_map(|s| match &s.lit {
                    Literal::Atom(l) if !l.neg => l
                        .args
                        .iter()
                        .position(|x| matches!(x, Term::Var(v) if v == n))
                        .map(|c| prog.decls[&l.rel][c].1.clone()),
                    _ => None,
                }),
                _ => None,
            }
        };
        fn tok(t: &Term, slots: &mut HashMap<String, usize>) -> Option<String> {
            match t {
                Term::Wild => Some("_".to_owned()),
                Term::Var(n) => {
                    let len = slots.len();
                    Some(format!("v{}", *slots.entry(n.clone()).or_insert(len)))
                }
                Term::Int(_) | Term::Str(_) => const_tok(t),
                _ => None,
            }
        }
        // The atom's source and tokens. An input relation is loaded with its constant columns as a filter and
        // projected away (`field($n, "path", $p)` loads the `path` edges only), so it is read in the size it is used.
        let atom = |l: &Atom, idx: Option<usize>, slots: &mut HashMap<String, usize>| -> Option<(String, Vec<String>)> {
            if scc.contains(&l.rel) {
                let src = format!("{}{}", if idx.is_some() && idx == at { "d" } else { "c" }, ri[&l.rel]);
                return Some((src, l.args.iter().map(|t| tok(t, slots)).collect::<Option<_>>()?));
            }
            if native_rels.contains(&l.rel) {
                return Some((format!("r:{}", hex(&l.rel)), l.args.iter().map(|t| tok(t, slots)).collect::<Option<_>>()?));
            }
            let is_const = |t: &Term| matches!(t, Term::Int(_) | Term::Str(_));
            let keep: Vec<usize> = (0..l.args.len()).filter(|&c| !is_const(&l.args[c])).collect();
            let cols = keep.iter().map(|&c| sql_col(&l.rel, c)).collect::<Vec<_>>().join(", ");
            let cols = if cols.is_empty() { "1".to_owned() } else { cols };
            let mut wheres = Vec::new();
            for (c, t) in l.args.iter().enumerate() {
                if is_const(t) {
                    wheres.push(format!("{} = {}", sql_col(&l.rel, c), sql_term(t, &HashMap::new(), self.model.inputs.atoms).ok()?));
                }
            }
            let types: String = keep.iter().map(|&c| if prog.decls[&l.rel][c].1 == "sym" { 's' } else { 'i' }).collect();
            let types = if types.is_empty() { "i".to_owned() } else { types };
            let mut sel = self.model.table(prog, &l.rel, "");
            if !wheres.is_empty() {
                sel += &format!(" WHERE {}", wheres.join(" AND "));
            }
            let mut toks: Vec<String> = keep.iter().map(|&c| tok(&l.args[c], slots)).collect::<Option<_>>()?;
            if toks.is_empty() {
                toks.push("_".to_owned());
            }
            let base = if prog.inputs.contains(&l.rel) || self.model.view_src.contains_key(&l.rel) { "b" } else { "d" };
            Some((format!("t:{base}:{}:{}:{types}", hex(&sel), hex(&cols)), toks))
        };
        let kinds = |kset: &str| hex(&prog.kindsets[kset].join(","));
        let mut lines = Vec::new();
        for (idx, s) in order.iter().enumerate() {
            match &s.lit {
                Literal::Atom(l) => {
                    if s.mode == Some(Mode::Range) {
                        return None;
                    }
                    let (sc, toks) = atom(l, Some(idx), &mut slots)?;
                    lines.push(format!("{} {sc} {}", if l.neg { "neg" } else { "pos" }, toks.join(" ")));
                }
                Literal::Cmp { l, op, r } if *op == "=" || *op == "!=" => {
                    if let Term::Call(f, args) = r {
                        if *op != "=" || f != "cat" || !matches!(l, Term::Var(_)) {
                            return None;
                        }
                        let (a, b) = (&args[0], &args[1]);
                        let (tl, ta) = (tok(l, &mut slots)?, tok(a, &mut slots)?);
                        lines.push(format!("cat {tl} {ta} {}", tok(b, &mut slots)?));
                    } else {
                        let unbound = |t: &Term, slots: &HashMap<String, usize>| matches!(t, Term::Var(v) if !slots.contains_key(v));
                        // The bound side goes right.
                        let (a, b) = if unbound(r, &slots) { (r, l) } else { (l, r) };
                        if unbound(b, &slots) {
                            return None;
                        }
                        let ta = tok(a, &mut slots)?;
                        lines.push(format!("{} {ta} {}", if *op == "=" { "eq" } else { "ne" }, tok(b, &mut slots)?));
                    }
                }
                Literal::Cmp { l, op, r } if matches!(*op, "<" | "<=" | ">" | ">=") => {
                    // Integers (and nodes) only: symbols are numbered in the order they are met.
                    let bad = |t: &Term| {
                        matches!(t, Term::Call(..))
                            || matches!(t, Term::Var(v) if !slots.contains_key(v))
                            || matches!(typ(t).as_deref(), None | Some("sym"))
                    };
                    if bad(l) || bad(r) {
                        return None;
                    }
                    let (a, b) = if matches!(*op, "<" | "<=") { (l, r) } else { (r, l) };
                    let ta = tok(a, &mut slots)?;
                    lines.push(format!("{} {ta} {}", if matches!(*op, "<" | ">") { "lt" } else { "le" }, tok(b, &mut slots)?));
                }
                Literal::Agg { var, op: AggOp::Count, atom: a, .. } => {
                    if a.args.iter().any(|t| matches!(t, Term::Var(v) if !slots.contains_key(v))) {
                        return None;
                    }
                    let (sc, toks) = atom(a, None, &mut slots)?;
                    lines.push(format!("count {sc} {} {}", toks.join(" "), tok(&Term::Var(var.clone()), &mut slots)?));
                }
                Literal::Nearest { n, a, kindset } if s.mode == Some(Mode::Climb) => {
                    let tn = tok(&Term::Var(n.clone()), &mut slots)?;
                    lines.push(format!("near {tn} {} {}", tok(&Term::Var(a.clone()), &mut slots)?, kinds(kindset)));
                }
                Literal::Nearest { n, a, kindset } if slots.contains_key(a) && !slots.contains_key(n) => {
                    let tn = tok(&Term::Var(n.clone()), &mut slots)?;
                    lines.push(format!("down {tn} {} {}", tok(&Term::Var(a.clone()), &mut slots)?, kinds(kindset)));
                }
                _ => return None,
            }
        }
        let mut head = Vec::new();
        for t in &h.args {
            match t {
                Term::Call(f, args) if f == "cat" && args.iter().all(|x| matches!(x, Term::Var(_))) => {
                    let t0 = tok(&args[0], &mut slots)?;
                    head.push(format!("cat:{t0}:{}", tok(&args[1], &mut slots)?));
                }
                _ => head.push(tok(t, &mut slots)?),
            }
        }
        lines.push(format!("head {}", head.join(" ")));
        Some(lines.join("\n"))
    }

    /// T's `native_stratum`: plans with the stratum as the component being solved when a rule reads it.
    fn native_stratum(&mut self, scc: &BTreeSet<String>, rules: &[&Rule]) -> Result<bool> {
        let reads = rules.iter().any(|r| !recursive(&r.body, scc).is_empty());
        self.model.stats.solving = if reads { scc.clone() } else { BTreeSet::new() };
        let out = self.native_stratum_(scc, rules);
        self.model.stats.solving.clear();
        out
    }

    /// One stratum on the native set engine, as a component of seed joins (rules without a recursive atom, run once)
    /// and join probes (one per recursive atom, planned with its delta first), all planned by `plan_rule` and
    /// replanned from live counts between chunks of waves (T's `native_stratum_`). Lower strata it reads natively
    /// come from the native store (`r:`), others from their tables or the facts. Nothing is written back: the
    /// relations join `native_rels`. False (and nothing run) where a rule has a literal with no native step.
    fn native_stratum_(&mut self, scc: &BTreeSet<String>, rules: &[&Rule]) -> Result<bool> {
        let prog = self.prog;
        if scc.iter().any(|r| prog.lattice.contains_key(r)) {
            return Ok(false);
        }
        let rels: Vec<&String> = scc.iter().collect();
        let ri: BTreeMap<String, usize> = rels.iter().enumerate().map(|(i, r)| ((*r).clone(), i)).collect();
        let name = format!("prog_{}", rels[0]);
        let mut gates: Vec<&(String, crate::code_rule_dsl::logic::magic::Gate)> = prog.gates.iter().filter(|(g, _)| scc.contains(g)).collect();
        gates.sort_by(|a, b| (a.1.level, &a.0).cmp(&(b.1.level, &b.0)));
        // A gate's rule only placed it here.
        let rules: Vec<&Rule> = rules.iter().copied().filter(|r| !gates.iter().any(|(g, _)| *g == r.head.rel)).collect();
        let types = |r: &str| prog_types(prog, r).iter().map(|t| if *t == "sym" { 's' } else { 'i' }).collect::<String>();
        let mut lines = vec![format!("name {name}")];
        lines.extend(rels.iter().map(|r| format!("rel {r} {}", types(r))));
        lines.push("nowrite".to_owned());
        let native_rels = self.native_rels.clone();
        let mut est: HashMap<&str, f64> = HashMap::new();
        let mut orders = Vec::new();
        let seeds: Vec<&Rule> = rules.iter().copied().filter(|r| recursive(&r.body, scc).is_empty()).collect();
        for r in &seeds {
            let (order, _, rows) = self.model.plan_rule(prog, &r.head, &r.body, None)?;
            let Some(spec) = self.join_spec(&r.head, &order, None, &ri, scc, &native_rels) else { return Ok(false) };
            *est.entry(r.head.rel.as_str()).or_default() += rows;
            lines.push(format!("seed {} {}", ri[&r.head.rel], hex(&spec)));
            orders.push(order);
        }
        for r in &rels {
            self.model.stats.card.insert((*r).clone(), est.get(r.as_str()).copied().unwrap_or(0.0).max(1.0));
        }
        let mut probes = Vec::new();
        for r in &rules {
            for (ai, a) in recursive(&r.body, scc) {
                let (order, _, _) = self.model.plan_rule(prog, &r.head, &r.body, Some(ai))?;
                let at = order.iter().position(|s| s.at == ai).expect("the delta atom");
                let Some(spec) = self.join_spec(&r.head, &order, Some(at), &ri, scc, &native_rels) else { return Ok(false) };
                // The wave whose delta is all of the relation (it was empty before): any order, the atom read in full.
                let (whole, _, _) = self.model.plan_rule(prog, &r.head, &r.body, None)?;
                let wspec = if whole[0].at != ai { self.join_spec(&r.head, &whole, None, &ri, scc, &native_rels) } else { None };
                let mut line = format!("join {} {} {}", ri[&r.head.rel], ri[&a.rel], hex(&spec));
                if let Some(w) = wspec {
                    line += &format!(" {}", hex(&w));
                }
                lines.push(line);
                probes.push((*r, ai, order));
            }
        }
        for (g, d) in &gates {
            let idx = |rs: &[String]| {
                let s = rs.iter().filter_map(|r| ri.get(r)).map(usize::to_string).collect::<Vec<_>>().join(",");
                if s.is_empty() { "-".to_owned() } else { s }
            };
            let deps = d.deps.iter().filter_map(|x| gates.iter().position(|(g2, _)| g2 == x)).map(|i| i.to_string()).collect::<Vec<_>>().join(",");
            lines.push(format!(
                "gate {} {} {} {}",
                ri[g],
                d.src.as_ref().map_or("-".to_owned(), |s| ri[s].to_string()),
                idx(&d.quiet),
                if deps.is_empty() { "-".to_owned() } else { deps }
            ));
        }
        self.db.query_row("SELECT set_fixpoint_open(?1)", [lines.join("\n")], |_| Ok(()))?;
        let mut chunk: i64 = 2;
        loop {
            let f: String = self.db.query_row("SELECT set_fixpoint_run(?1, ?2)", rusqlite::params![name, chunk], |r| r.get(0))?;
            let f: Vec<&str> = f.split_whitespace().collect();
            let steps: String = self.db.query_row("SELECT set_fixpoint_steps(?1)", [&name], |r| r.get(0))?;
            for (o, line) in orders.iter().chain(probes.iter().map(|p| &p.2)).zip(steps.split('\n')) {
                observe(&mut self.model.stats, o, line);
            }
            let live: String = self.db.query_row("SELECT set_fixpoint_stats(?1)", [&name], |r| r.get(0))?;
            for (r, line) in rels.iter().zip(live.split('\n')) {
                live_stats(prog, &mut self.model, r, line);
            }
            if f.last() == Some(&"1") || chunk == 0 {
                break;
            }
            for k in 0..probes.len() {
                let (r, ai) = (probes[k].0, probes[k].1);
                let (new, _, _) = self.model.plan_rule(prog, &r.head, &r.body, Some(ai))?;
                if shown(&new) == shown(&probes[k].2) {
                    continue;
                }
                let at = new.iter().position(|s| s.at == ai).expect("the delta atom");
                let spec = self
                    .join_spec(&r.head, &new, Some(at), &ri, scc, &native_rels)
                    .ok_or_else(|| Error::Plan(format!("a replanned probe of {} has no native step", r.head)))?;
                self.db.query_row(
                    "SELECT set_fixpoint_reprobe(?1, ?2, 'join', ?3)",
                    rusqlite::params![name, (seeds.len() + k) as i64, hex(&spec)],
                    |_| Ok(()),
                )?;
                probes[k].2 = new;
            }
            chunk *= 2;
        }
        self.db.query_row("SELECT set_fixpoint_close(?1)", [&name], |_| Ok(()))?;
        self.native_rels.extend(scc.iter().cloned());
        Ok(true)
    }

    /// Runs a one-relation flat-lattice component natively (sqlite-ext `lattice_fixpoint`, T's `native_fixpoint`)
    /// if every rule is a seed (no recursive atom), a copy (`R($n, $v) :- ..., R($x, $v)`) or a binary transfer
    /// (`R($n, @flat_bin($o, $a, $b)) :- ..., R($l, $a), R($r, $b)`). The rest of each body is non-recursive, so SQL
    /// evaluates it once, planned as usual, into the component's seed and edge tables. Returns false to fall back.
    fn native_fixpoint(&mut self, scc: &BTreeSet<String>, rules: &[&Rule]) -> Result<bool> {
        let prog = self.prog;
        let native = &self.model.inputs.native;
        let [rel] = scc.iter().collect::<Vec<_>>()[..] else { return Ok(false) };
        if !prog.lattice.contains_key(rel) || prog.decls[rel].len() != 2 {
            return Ok(false);
        }
        let mut parts: Vec<(&str, Atom, Vec<Literal>)> = Vec::new();
        for r in rules {
            let (h, body) = (&r.head, &r.body);
            let rec: Vec<&Atom> = body.iter().filter_map(|l| if let Literal::Atom(a) = l && a.rel == *rel { Some(a) } else { None }).collect();
            let rest: Vec<Literal> = body.iter().filter(|l| !matches!(l, Literal::Atom(a) if a.rel == *rel)).cloned().collect();
            if rec.iter().any(|a| a.neg) {
                return Ok(false);
            }
            let rvals: BTreeSet<&String> = rec.iter().filter_map(|a| if let Term::Var(v) = &a.args[1] { Some(v) } else { None }).collect();
            if rvals.len() != rec.len() || rest.iter().any(|l| l.vars().iter().any(|v| rvals.contains(v))) {
                return Ok(false);
            }
            let (key, val) = (&h.args[0], &h.args[1]);
            let kinds: Vec<usize> = body
                .iter()
                .enumerate()
                .filter(|(_, l)| {
                    matches!(l, Literal::Atom(a) if !a.neg && a.rel == "kind" && matches!((key, &a.args[0]), (Term::Var(k), Term::Var(x)) if k == x))
                })
                .map(|(i, _)| i)
                .collect();
            let mut val_vars = BTreeSet::new();
            val.vars(&mut val_vars);
            let kseed = match (key, kinds.as_slice()) {
                (Term::Var(kn), [ki]) if rec.is_empty() && native.contains("kseed") => {
                    let Literal::Atom(ka) = &body[*ki] else { unreachable!() };
                    matches!(&ka.args[1], Term::Var(_))
                        && body.iter().enumerate().all(|(i, l)| i == *ki || !l.vars().contains(kn))
                        && !val_vars.contains(kn)
                }
                _ => false,
            };
            if kseed {
                // A per-kind default (`R($n, v) :- kind($n, $k), <tests of $k>`): evaluated over the distinct kinds,
                // and the native side seeds every node of those kinds from the tape. No row per node.
                let Literal::Atom(ka) = &body[kinds[0]] else { unreachable!() };
                let k = ka.args[1].clone();
                let mut b = vec![Literal::Atom(atom_of("rdl_kinds".to_owned(), vec![k.clone()]))];
                b.extend(body.iter().enumerate().filter(|(i, _)| *i != kinds[0]).map(|(_, l)| l.clone()));
                parts.push(("kseed", atom_of(format!("lfx_{rel}_kseed"), vec![k, val.clone()]), b));
            } else if rec.is_empty() {
                parts.push(("seed", atom_of(format!("lfx_{rel}_seed"), vec![key.clone(), val.clone()]), body.clone()));
            } else if rec.len() == 1 && matches!((val, &rec[0].args[1]), (Term::Var(v), Term::Var(x)) if v == x) {
                parts.push(("copy", atom_of(format!("lfx_{rel}_copy"), vec![key.clone(), rec[0].args[0].clone()]), rest));
            } else if let Term::Call(f, a) = val
                && rec.len() == 2
                && f == "flat_bin"
                && matches!((&a[1], &rec[0].args[1]), (Term::Var(v), Term::Var(x)) if v == x)
                && matches!((&a[2], &rec[1].args[1]), (Term::Var(v), Term::Var(x)) if v == x)
            {
                let head = atom_of(format!("lfx_{rel}_bin"), vec![key.clone(), a[0].clone(), rec[0].args[0].clone(), rec[1].args[0].clone()]);
                parts.push(("bin", head, rest));
            } else {
                return Ok(false);
            }
        }
        let schema = self.model.inputs.facts.trim_end_matches('.').to_owned();
        let has_kseed = parts.iter().any(|(k, _, _)| *k == "kseed");
        if has_kseed && !self.views.contains("rdl_kinds") {
            self.db.execute(&format!("CREATE TEMP VIEW rdl_kinds AS SELECT kind AS c0 FROM {}stat_kind", self.model.inputs.stats), [])?;
            self.views.insert("rdl_kinds".to_owned());
            let n = self.model.stats.kfreq.len() as f64;
            self.model.stats.card.insert("rdl_kinds".to_owned(), n);
            self.model.ndv.insert(("rdl_kinds".to_owned(), 0), n);
            if !self.model.inputs.native.contains("nearest") {
                self.db.query_row("SELECT tape_load(?1)", [&schema], |_| Ok(()))?;
            }
        }
        for (kind, cols) in [("seed", 2), ("copy", 2), ("bin", 4), ("kseed", 2)] {
            self.db.execute(&format!("DROP TABLE IF EXISTS lfx_{rel}_{kind}"), [])?;
            if kind != "kseed" || has_kseed {
                let cs = (0..cols).map(|i| format!("c{i}")).collect::<Vec<_>>().join(", ");
                self.db.execute(&format!("CREATE TEMP TABLE lfx_{rel}_{kind}({cs})"), [])?;
            }
        }
        for (_, head, body) in &parts {
            let (order, _, _) = self.model.plan_rule(prog, head, body, None)?;
            self.ensure_indexes(&order)?;
            let select = self.model.sql_select(prog, head, &order, None, None)?;
            self.db.execute(&format!("INSERT INTO {} {select}", head.rel), [])?;
        }
        let native = &self.model.inputs.native;
        if native.contains("vtab") {
            // Kept in memory; the relation becomes a virtual table over it (no rows written).
            self.db.query_row(&format!("SELECT lattice_fixpoint('lfx_{rel}', '@{rel}')"), [], |_| Ok(()))?;
            self.db.execute_batch(&format!("DROP TABLE {rel}; CREATE VIRTUAL TABLE temp.{rel} USING lattice_result({rel});"))?;
            self.views.insert(rel.clone());
        } else if native.contains("direct") {
            // Written straight into the relation: no output table to copy back.
            self.db.query_row(&format!("SELECT lattice_fixpoint('lfx_{rel}', '{rel}')"), [], |_| Ok(()))?;
        } else {
            self.db.query_row(&format!("SELECT lattice_fixpoint('lfx_{rel}')"), [], |_| Ok(()))?;
            self.db.execute(&format!("INSERT INTO {rel} SELECT c0, c1 FROM lfx_{rel}_out"), [])?;
        }
        Ok(true)
    }

    /// Runs a component of set-valued relations natively (sqlite-ext `set_fixpoint_*`, `set_rel`; T's
    /// `native_set_fixpoint`). Seeds (rules with no recursive atom) run once in SQL. Each recursive rule is then an
    /// edge rule (a linear rule whose non-recursive rest SQL evaluates once into an edge table, chosen when that is
    /// cheaper than one delta pass and than one pass over the tape) or a probe (every other rule: its SQL, or with
    /// `join` its native join, runs once per wave for each recursive atom whose delta is non-empty, reading that atom
    /// from a `set_rel` delta vtab and the others from full ones). The native side keeps the tuples, deduplicates
    /// them and schedules the waves.
    fn native_set_fixpoint(&mut self, scc: &BTreeSet<String>, rules: &[&Rule]) -> Result<bool> {
        let prog = self.prog;
        if scc.iter().any(|r| prog.lattice.contains_key(r)) {
            return Ok(false);
        }
        let rels: Vec<&String> = scc.iter().collect();
        let ri: BTreeMap<String, usize> = rels.iter().enumerate().map(|(i, r)| ((*r).clone(), i)).collect();
        let name = format!("sfx{}_{}", self.made.len(), rels[0]);
        let mut seed_plans = Vec::new();
        for r in rules.iter().filter(|r| recursive(&r.body, scc).is_empty()) {
            seed_plans.push((&r.head, self.model.plan_rule(prog, &r.head, &r.body, None)?));
        }
        for r in &rels {
            let rows: f64 = seed_plans.iter().filter(|(h, _)| h.rel == **r).map(|(_, p)| p.2).sum();
            self.model.stats.card.insert((*r).clone(), rows.max(1.0));
        }
        let var_type = |v: &str, body: &[Literal]| -> Option<char> {
            body.iter().find_map(|l| match l {
                Literal::Atom(a) if !a.neg => a
                    .args
                    .iter()
                    .position(|t| matches!(t, Term::Var(x) if x == v))
                    .map(|c| if prog.decls[&a.rel][c].1 == "sym" { 's' } else { 'i' }),
                _ => None,
            })
        };
        let tables: HashMap<String, (String, String)> = rels.iter().map(|r| ((*r).clone(), (format!("sfx_f_{r}"), format!("sfx_d_{r}")))).collect();
        let types = |r: &str| prog_types(prog, r).iter().map(|t| if *t == "sym" { 's' } else { 'i' }).collect::<String>();
        let mut lines = vec![format!("name {name}")];
        lines.extend(rels.iter().map(|r| format!("rel {r} {}", types(r))));
        let mut edges: Vec<(Atom, Vec<Step>)> = Vec::new();
        let mut probes: Vec<(&Rule, Vec<Step>, usize, usize)> = Vec::new();
        for (j, r) in rules.iter().enumerate() {
            let (h, body) = (&r.head, &r.body);
            let rec = recursive(body, scc);
            if rec.is_empty() {
                continue;
            }
            if let [(ai, a)] = rec[..] {
                let rest: Vec<Literal> = body.iter().enumerate().filter(|(i, _)| *i != ai).map(|(_, l)| l.clone()).collect();
                // The spec line and edge plan of a linear rule, or the reason it is a probe.
                let edge_rule = || -> Result<std::result::Result<(String, Option<(Atom, Vec<Step>)>), String>> {
                    let restvars: BTreeSet<String> = rest.iter().flat_map(|l| l.vars()).collect();
                    let mut hvars = BTreeSet::new();
                    h.args.iter().for_each(|t| t.vars(&mut hvars));
                    let avars = a.vars();
                    let key: Vec<String> = avars.intersection(&restvars).cloned().collect();
                    let ecols: Vec<String> = hvars.intersection(&restvars).filter(|v| !avars.contains(*v)).cloned().collect();
                    if hvars.iter().any(|v| !avars.contains(v) && !restvars.contains(v)) {
                        return Ok(Err("head variable from nowhere".to_owned()));
                    }
                    let slot: HashMap<&String, usize> = avars.iter().chain(&ecols).enumerate().map(|(i, v)| (v, i)).collect();
                    let mut pat = Vec::new();
                    for t in &a.args {
                        pat.push(match t {
                            Term::Wild => "_".to_owned(),
                            Term::Var(v) => format!("v{}", slot[v]),
                            Term::Int(_) | Term::Str(_) => const_tok(t).expect("a constant"),
                            _ => return Ok(Err("pattern term".to_owned())),
                        });
                    }
                    let mut head = Vec::new();
                    for t in &h.args {
                        head.push(match t {
                            Term::Var(v) => format!("v{}", slot[v]),
                            Term::Int(_) | Term::Str(_) => const_tok(t).expect("a constant"),
                            Term::Call(f, xs) if f == "cat" && xs.iter().all(|x| matches!(x, Term::Var(_))) => {
                                let (Term::Var(x0), Term::Var(x1)) = (&xs[0], &xs[1]) else { unreachable!() };
                                format!("cat:v{}:v{}", slot[x0], slot[x1])
                            }
                            _ => return Ok(Err("head term".to_owned())),
                        });
                    }
                    let mut edge = None;
                    let src = if !rest.is_empty() {
                        if !rest.iter().any(|l| matches!(l, Literal::Atom(x) if !x.neg)) {
                            return Ok(Err("rest has no atom".to_owned()));
                        }
                        let ehead = atom_of(format!("sfx_{}_{j}", h.rel), key.iter().chain(&ecols).map(|v| Term::Var(v.clone())).collect());
                        let Ok((order, work, _)) = self.model.plan_rule(prog, &ehead, &rest, None) else {
                            return Ok(Err("rest cannot be planned alone".to_owned()));
                        };
                        if work > self.model.stats.nodes {
                            return Ok(Err(format!("rest costs {work:.0} > {} nodes alone", self.model.stats.nodes)));
                        }
                        let delta = self.model.plan_rule(prog, h, body, Some(ai))?.1;
                        if work > delta {
                            return Ok(Err(format!("edges cost {work:.0} > one delta pass {delta:.0}")));
                        }
                        let t: String = key.iter().chain(&ecols).map(|v| var_type(v, &rest).unwrap_or('i')).collect();
                        let src = format!("{} {}", ehead.rel, if t.is_empty() { "-".to_owned() } else { t });
                        edge = Some((ehead, order));
                        src
                    } else {
                        "- -".to_owned()
                    };
                    let vs = |vs: &[String]| vs.iter().map(|v| format!("v{}", slot[v])).collect::<Vec<_>>().join(" ");
                    let line = format!(
                        "rule {} {} {src} key {} ; ecols {} ; pat {} ; head {}",
                        ri[&h.rel],
                        ri[&a.rel],
                        vs(&key),
                        vs(&ecols),
                        pat.join(" "),
                        head.join(" ")
                    );
                    Ok(Ok((line, edge)))
                };
                if let Ok((line, edge)) = edge_rule()? {
                    lines.push(line);
                    edges.extend(edge);
                    continue;
                }
            }
            // A probe per recursive atom, planned with that atom's delta first.
            for (ai, _) in rec {
                let (order, _, _) = self.model.plan_rule(prog, h, body, Some(ai))?;
                let at = order.iter().position(|s| s.at == ai).expect("the delta atom");
                probes.push((*r, order, at, ai));
            }
        }
        for (h, (order, _, _)) in &seed_plans {
            self.ensure_indexes(order)?;
            let select = self.model.sql_select(prog, h, order, None, None)?;
            self.insert_rows(h, &select)?;
        }
        for (ehead, order) in &edges {
            self.ensure_indexes(order)?;
            let cols = (0..ehead.args.len()).map(|i| format!("c{i}")).collect::<Vec<_>>().join(", ");
            self.db.execute_batch(&format!("DROP TABLE IF EXISTS {0}; CREATE TEMP TABLE {0}({cols});", ehead.rel))?;
            if !ehead.args.is_empty() {
                let select = self.model.sql_select(prog, ehead, order, None, None)?;
                self.db.execute(&format!("INSERT INTO {} {select}", ehead.rel), [])?;
            }
        }
        let join = self.model.inputs.native.contains("join");
        let none = BTreeSet::new();
        for (r, order, at, ai) in &probes {
            let rel = match &r.body[*ai] {
                Literal::Atom(a) => a.rel.clone(),
                _ => unreachable!(),
            };
            if join && let Some(spec) = self.join_spec(&r.head, order, Some(*at), &ri, scc, &none) {
                lines.push(format!("join {} {} {}", ri[&r.head.rel], ri[&rel], hex(&spec)));
                continue;
            }
            self.ensure_indexes(order)?;
            let sql = self.model.sql_select(prog, &r.head, order, Some(*at), Some(&tables))?;
            lines.push(format!("probe {} {} {}", ri[&r.head.rel], ri[&rel], hex(&sql)));
        }
        self.db.query_row("SELECT set_fixpoint_open(?1)", [lines.join("\n")], |_| Ok(()))?;
        for r in &rels {
            let (full, delta) = &tables[*r];
            for (which, vt) in [("full", full), ("delta", delta)] {
                self.db.execute(&format!("CREATE VIRTUAL TABLE temp.{vt} USING set_rel({name}, {}, {which})", ri[*r]), [])?;
            }
        }
        // The probes were planned from seed estimates, and a derived relation's columns have no distinct counts then
        // (each is taken as 1, so a join on one looks unselective). Run a few waves, take the live counts from the
        // native store's hash indexes, replan the probes and resume, doubling the chunk until the fixpoint.
        let mut chunk: i64 = 2;
        loop {
            let f: String = self.db.query_row("SELECT set_fixpoint_run(?1, ?2)", rusqlite::params![name, chunk], |r| r.get(0))?;
            if f.split_whitespace().last() == Some("1") || chunk == 0 {
                break;
            }
            let live: String = self.db.query_row("SELECT set_fixpoint_stats(?1)", [&name], |r| r.get(0))?;
            for (r, line) in rels.iter().zip(live.split('\n')) {
                live_stats(prog, &mut self.model, r, line);
            }
            for k in 0..probes.len() {
                let (r, ai) = (probes[k].0, probes[k].3);
                let (new, _, _) = self.model.plan_rule(prog, &r.head, &r.body, Some(ai))?;
                if shown(&new) == shown(&probes[k].1) {
                    continue;
                }
                let nat = new.iter().position(|s| s.at == ai).expect("the delta atom");
                let spec = if join { self.join_spec(&r.head, &new, Some(nat), &ri, scc, &none) } else { None };
                let (kind, body) = match spec {
                    Some(spec) => ("join", hex(&spec)),
                    None => {
                        self.ensure_indexes(&new)?;
                        ("sql", hex(&self.model.sql_select(prog, &r.head, &new, Some(nat), Some(&tables))?))
                    }
                };
                self.db.query_row(
                    "SELECT set_fixpoint_reprobe(?1, ?2, ?3, ?4)",
                    rusqlite::params![name, k as i64, kind, body],
                    |_| Ok(()),
                )?;
                probes[k].1 = new;
                probes[k].2 = nat;
            }
            chunk *= 2;
        }
        for r in &rels {
            let (full, delta) = &tables[*r];
            self.db.execute_batch(&format!("DROP TABLE temp.{full}; DROP TABLE temp.{delta};"))?;
        }
        self.db.query_row("SELECT set_fixpoint_close(?1)", [&name], |_| Ok(()))?;
        self.made.insert((format!("sfx:{name}"), Vec::new()));
        Ok(true)
    }

    /// The same merge as `merge_lattice`, as one UPSERT (T's `merge_upsert`): each candidate either inserts its key
    /// or joins into the stored value (flat lattice: a different constant makes top). Triggers record the keys that
    /// changed, so there is no candidate table, GROUP BY, or compare pass.
    fn merge_upsert(&self, rel: &str, select: &str, delta: bool) -> Result<i64> {
        let k = self.prog.lattice[rel].keys;
        let keys = (0..k).map(|i| format!("c{i}")).collect::<Vec<_>>().join(", ");
        let v = format!("c{k}");
        let exists: Option<i64> =
            self.db.query_row(&format!("SELECT 1 FROM sqlite_temp_master WHERE name = 'chg_{rel}'"), [], |r| r.get(0)).ok();
        if exists.is_none() {
            let cols = (0..=k).map(|i| format!("c{i}")).collect::<Vec<_>>().join(", ");
            let news = (0..=k).map(|i| format!("NEW.c{i}")).collect::<Vec<_>>().join(", ");
            self.db.execute(&format!("CREATE TEMP TABLE chg_{rel}({cols}, PRIMARY KEY ({keys}))"), [])?;
            for ev in ["INSERT", "UPDATE"] {
                self.db.execute(
                    &format!(
                        "CREATE TEMP TRIGGER chg_{rel}_{} AFTER {ev} ON {rel} BEGIN \
                         INSERT INTO chg_{rel} VALUES ({news}) ON CONFLICT({keys}) DO UPDATE SET {v} = excluded.{v}; END",
                        ev.to_lowercase()
                    ),
                    [],
                )?;
            }
        }
        self.db.execute(
            &format!(
                "INSERT INTO {rel} SELECT * FROM ({select}) WHERE true ON CONFLICT({keys}) DO UPDATE SET {v} = {TOP} \
                 WHERE {v} <> {TOP} AND {v} <> excluded.{v}"
            ),
            [],
        )?;
        let n = self.count(&format!("SELECT count(*) FROM chg_{rel}"))?;
        if delta {
            self.db.execute(&format!("INSERT INTO new_{rel} SELECT * FROM chg_{rel}"), [])?;
        }
        self.db.execute(&format!("DELETE FROM chg_{rel}"), [])?;
        Ok(n)
    }
}

/// Evaluates `prog` over the facts database at `facts` (T's `run_sql`, pure SQL mode): views for projections of
/// one input, typed tables for the rest, then each stratum planned from the statistics and the counts of the strata
/// before it, non-recursive ones once and recursive ones semi-naively.
pub(in crate::code_rule_dsl) fn run_sql(prog: &Program, facts: &Path) -> Result<Outcome> {
    let db = Connection::open_in_memory()?;
    db.execute("ATTACH ?1 AS f", [format!("file:{}?mode=ro", facts.display())])?;
    // Map the facts file, so probes of the input relations' indexes are memory reads, and keep ephemeral b-trees
    // (DISTINCT, sorts, materialized subqueries) in memory.
    db.execute_batch("PRAGMA f.mmap_size = 4294967296; PRAGMA temp_store = MEMORY;")?;
    run(prog, &db, Inputs::file(), None)
}

/// `run_sql` with T's `RDL_NATIVE` modes (`native_modes`). `load` registers T's extension functions on the run's
/// connection before anything runs, as T's `run_sql` loads `RDL_EXT`.
pub(in crate::code_rule_dsl) fn run_sql_native(
    prog: &Program,
    facts: &Path,
    modes: &str,
    load: impl FnOnce(&Connection) -> rusqlite::Result<()>,
) -> Result<Outcome> {
    let db = Connection::open_in_memory()?;
    db.execute("ATTACH ?1 AS f", [format!("file:{}?mode=ro", facts.display())])?;
    db.execute_batch("PRAGMA f.mmap_size = 4294967296; PRAGMA temp_store = MEMORY;")?;
    load(&db)?;
    run(prog, &db, Inputs { native: native_modes(modes), ..Inputs::file() }, None)
}

/// Evaluates a lowered program (decision 0009) over the base relations bound on `db` (`facts::vtab::syntax::bind`),
/// with the planner's statistics in the attached schema `stats`. `db` is a connection to the content store: the
/// symbols the run makes are appended to its `atoms`, and derived relations are `temp` tables, dropped by the next
/// run. Outputs are decoded: names and kinds as text, node ids as the vtabs'.
pub(in crate::code_rule_dsl) fn run_atoms(lowered: &lower::Lowered, db: &Connection, stats: &str) -> Result<Outcome> {
    db.execute_batch("PRAGMA temp_store = MEMORY;")?;
    super::native::register_symbols(db)?;
    let inputs = Inputs { facts: "main.".to_owned(), stats: format!("{stats}."), atoms: Some(lowered.ops), native: BTreeSet::new() };
    let out = run(&lowered.program, db, inputs, Some(lowered));
    super::native::flush_symbols(db)?;
    out
}

fn run(prog: &Program, db: &Connection, inputs: Inputs, lowered: Option<&lower::Lowered>) -> Result<Outcome> {
    let db = db;
    let temp: Vec<(String, String)> = db
        .prepare("SELECT type, name FROM temp.sqlite_master WHERE type IN ('table', 'view')")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (ty, name) in temp {
        db.execute_batch(&format!("DROP {} IF EXISTS temp.{name}", if ty == "view" { "VIEW" } else { "TABLE" }))?;
    }
    // Positions in a symbol, for splitting one at its dots (`$m = @cat($q, $x)` with only $m bound).
    db.execute_batch("CREATE TEMP TABLE rdl_int(i INTEGER PRIMARY KEY); INSERT INTO rdl_int WITH RECURSIVE i(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM i WHERE i < 4096) SELECT i FROM i;")?;
    let atoms = inputs.atoms.is_some();
    let facts = inputs.facts.clone();
    if inputs.native.contains("nearest") {
        db.query_row("SELECT tape_load(?1)", [facts.trim_end_matches('.')], |_| Ok(()))?;
    }
    let model = make_plan(prog, db, inputs)?;
    let (sccs, _) = strata(prog)?;
    let mut rules_by: IndexMap<&str, Vec<&Rule>> = IndexMap::new();
    for r in &prog.rules {
        rules_by.entry(r.head.rel.as_str()).or_default().push(r);
    }
    let native = model.inputs.native.clone();
    let mut run = Run { prog, db, model, views: HashSet::new(), made: HashSet::new(), native_rels: BTreeSet::new() };
    for (&rel, rs) in &rules_by {
        let [rule] = rs.as_slice() else { continue };
        if prog.lattice.contains_key(rel) || prog.facts.contains_key(rel) {
            continue;
        }
        let (h, b) = (&rule.head, &rule.body);
        let [Literal::Atom(a)] = b.as_slice() else { continue };
        if a.neg || !prog.inputs.contains(&a.rel) {
            continue;
        }
        let order = vec![Step { at: 0, lit: b[0].clone(), mode: None }];
        let select = run.model.sql_select(prog, h, &order, None, None)?.replacen("SELECT DISTINCT", "SELECT", 1);
        db.execute(&format!("CREATE TEMP VIEW {rel} AS {select}"), [])?;
        run.views.insert(rel.to_owned());
        run.model.view_stats(rel, h, a);
        let src = first_columns(a);
        let cols = h.args.iter().map(|t| if let Term::Var(v) = t { src.get(v.as_str()).map(|&c| sql_col(&a.rel, c)) } else { None }).collect();
        run.model.view_src.insert(rel.to_owned(), (a.rel.clone(), cols));
        if let ([Term::Var(h0), Term::Var(h1)], [Term::Var(d), Term::Var(n), rest @ ..]) = (h.args.as_slice(), a.args.as_slice())
            && a.rel == "node"
            && rest.iter().all(|t| matches!(t, Term::Wild))
            && d != n
        {
            let names = [h0, h1];
            let (mut sorted_h, mut sorted_a) = (vec![h0, h1], vec![d, n]);
            sorted_h.sort();
            sorted_a.sort();
            if sorted_h == sorted_a {
                let pos = |x: &String| names.iter().position(|v| *v == x).expect("a head variable");
                run.model.docof.insert(rel.to_owned(), (pos(n), pos(d)));
            }
        }
    }
    for (rel, cols) in &prog.decls {
        if prog.inputs.contains(rel) || run.views.contains(rel) {
            continue;
        }
        // Typed columns: an untyped column compared with an INTEGER one gets numeric affinity applied to it, and
        // SQLite then can't use its index (lessons ledger L11, narrow typed tables).
        let names = cols
            .iter()
            .enumerate()
            .map(|(i, (_, t))| match t.as_str() {
                "node" | "int" | "flat" => Ok(format!("c{i} INTEGER")),
                "sym" if atoms => Ok(format!("c{i} INTEGER")),
                "sym" => Ok(format!("c{i} TEXT")),
                _ => Err(Error::Plan(format!("{rel}: no SQL type for {t}"))),
            })
            .collect::<Result<Vec<_>>>()?
            .join(", ");
        // Clustered on the key (WITHOUT ROWID), so an insert writes one b-tree, and every secondary index carries the
        // whole row.
        let key = match prog.lattice.get(rel) {
            Some(l) => (0..l.keys).map(|i| format!("c{i}")).collect::<Vec<_>>().join(", "),
            None => (0..cols.len()).map(|i| format!("c{i}")).collect::<Vec<_>>().join(", "),
        };
        db.execute(&format!("CREATE TEMP TABLE {rel}({names}, PRIMARY KEY ({key})) WITHOUT ROWID"), [])?;
        for row in prog.facts.get(rel).into_iter().flatten() {
            let values: Vec<SqlValue> = row
                .iter()
                .map(|v| match v {
                    Value::Int(i) => SqlValue::Integer(*i),
                    Value::Str(s) => SqlValue::Text(s.clone()),
                })
                .collect();
            let holes = vec!["?"; values.len()].join(", ");
            db.execute(&format!("INSERT OR IGNORE INTO {rel} VALUES ({holes})"), rusqlite::params_from_iter(values))?;
        }
    }
    let mut outcome = Outcome::default();
    for scc in &sccs {
        let rules: Vec<&Rule> = scc.iter().flat_map(|rel| rules_by.get(rel.as_str()).into_iter().flatten().copied()).collect();
        let key = scc.iter().cloned().collect::<Vec<_>>().join("/");
        if native.contains("program") && !scc.iter().all(|r| run.views.contains(r)) {
            if run.native_stratum(scc, &rules)? {
                outcome.native.insert(key);
                continue;
            }
            if prog.gates.iter().any(|(g, _)| scc.contains(g)) {
                return Err(Error::Plan(format!("gated negations in {scc:?} need native steps for every rule")));
            }
            // This stratum runs in SQL, which reads the native relations it joins from their tables.
            let read: Vec<String> = rules
                .iter()
                .flat_map(|r| &r.body)
                .filter_map(|l| match l {
                    Literal::Atom(a) | Literal::Agg { atom: a, .. } => Some(a.rel.clone()),
                    _ => None,
                })
                .collect();
            run.materialize(&read)?;
        }
        let mut planned = Vec::new();
        for rule in &rules {
            let (order, work, _) = run.model.plan_rule(prog, &rule.head, &rule.body, None)?;
            run.ensure_indexes(&order)?;
            outcome.chosen.push(Chosen {
                rule: prog.rules.iter().position(|r| std::ptr::eq(r, *rule)).expect("a rule of the program"),
                steps: order.iter().map(|s| (s.at, s.mode)).collect(),
                head: rule.head.to_string(),
                anchor: order[0].lit.to_string(),
                walks: order.iter().filter_map(|s| s.mode.map(|m| (s.lit.to_string(), m.name()))).collect(),
                work: work.round_ties_even() as i64,
            });
            planned.push((rule.head.clone(), order));
        }
        if scc.iter().all(|r| run.views.contains(r)) {
            continue;
        }
        let recursive =
            scc.len() > 1 || planned.iter().flat_map(|(_, o)| o).any(|s| matches!(&s.lit, Literal::Atom(a) if !a.neg && scc.contains(&a.rel)));
        if !recursive {
            for (h, order) in &planned {
                let sql = run.model.sql_select(prog, h, order, None, None)?;
                run.insert_rows(h, &sql)?;
            }
        } else {
            for r in scc {
                db.execute(&format!("CREATE TEMP TABLE delta_{r} AS SELECT * FROM {r} WHERE 0"), [])?;
            }
            if native.contains("fixpoint") && run.native_fixpoint(scc, &rules)? {
                outcome.native.insert(key);
                for r in scc {
                    run.refresh(r, false)?;
                }
                continue;
            }
            if native.contains("setfix") && run.native_set_fixpoint(scc, &rules)? {
                outcome.native.insert(key);
            } else {
                // First round: every rule over the full (initially empty or seeded) relations.
                let mut changed = run.semi_naive_round(scc, &planned, true, &[])?;
                let mut n = 1;
                // Each delta variant is planned with its delta first, from the first round's counts; a still-growing
                // relation scanned unjoined is priced as the native planner prices it (`atom_rows`).
                for r in scc {
                    run.refresh(r, true)?;
                }
                run.model.stats.solving = scc.clone();
                let mut deltas = Vec::new();
                for rule in &rules {
                    for (i, l) in rule.body.iter().enumerate() {
                        if matches!(l, Literal::Atom(a) if !a.neg && scc.contains(&a.rel)) {
                            let (order, _, _) = run.model.plan_rule(prog, &rule.head, &rule.body, Some(i))?;
                            let at = order.iter().position(|s| s.at == i).expect("the delta atom");
                            deltas.push((rule.head.clone(), order, at));
                        }
                    }
                }
                run.model.stats.solving.clear();
                while changed != 0 {
                    changed = run.semi_naive_round(scc, &planned, false, &deltas)?;
                    n += 1;
                }
                outcome.iterations.insert(key, n);
            }
        }
        for r in scc {
            run.refresh(r, false)?;
            for c in 0..prog.decls[r].len() {
                let d = run.count(&format!("SELECT count(DISTINCT c{c}) FROM {r}"))?;
                run.model.ndv.insert((r.clone(), c), if d == 0 { 1.0 } else { d as f64 });
            }
        }
    }
    run.materialize(&prog.outputs)?;
    let text = |e: &str| if atoms { format!("sym_text({e})") } else { e.to_owned() };
    for rel in &prog.outputs {
        // Each column's decoding: symbols as text (names in SQL, kinds and fields from the dialect's grammar).
        let domains: Vec<lower::Domain> = match lowered {
            Some(l) if rel != "finding" => l.domains[rel].clone(),
            _ => Vec::new(),
        };
        let sql = if rel == "finding" {
            format!(
                "SELECT DISTINCT {}, {}, k.start FROM finding x JOIN {facts}node k ON k.n = x.c1 JOIN {facts}doc d ON d.d = k.d",
                text("x.c0"),
                text("d.path")
            )
        } else if atoms {
            let cols: Vec<String> = domains
                .iter()
                .enumerate()
                .map(|(i, d)| if *d == lower::Domain::Name { text(&format!("c{i}")) } else { format!("c{i}") })
                .collect();
            format!("SELECT {} FROM {rel}", cols.join(", "))
        } else {
            format!("SELECT * FROM {rel}")
        };
        let name = if rel == "finding" { "finding_at".to_owned() } else { rel.clone() };
        let kinds = lowered.map(|l| l.dialect.kinds());
        let decode = |i: usize, v: i64| -> Option<String> {
            let kinds = kinds?;
            match domains.get(i)? {
                lower::Domain::Kind => Some(kinds.kind_name((v % i64::from(crate::code_rule_dsl::facts::core::KIND_STRIDE)) as u16)?.to_owned()),
                lower::Domain::Field => Some(kinds.field_name((v & 0xffff) as u16)?.to_owned()),
                _ => None,
            }
        };
        let mut st = db.prepare(&sql)?;
        let width = st.column_count();
        let mut rows: Vec<String> = st
            .query_map([], |r| {
                (0..width)
                    .map(|i| {
                        Ok(match r.get::<_, SqlValue>(i)? {
                            SqlValue::Integer(v) => decode(i, v).unwrap_or_else(|| v.to_string()),
                            SqlValue::Real(f) => format!("{f:?}"),
                            SqlValue::Text(s) => s,
                            SqlValue::Null => "None".to_owned(),
                            SqlValue::Blob(b) => format!("{b:?}"),
                        })
                    })
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map(|v| v.join("\t"))
            })?
            .collect::<rusqlite::Result<_>>()?;
        rows.sort();
        let nodes = domains.iter().enumerate().filter(|(_, d)| **d == lower::Domain::Node).map(|(i, _)| i).collect();
        outcome.nodes.insert(name.clone(), nodes);
        outcome.outputs.insert(name, rows);
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests;
