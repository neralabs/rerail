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
//! Rule bodies are typed queries (`sql::select`, over `sql_ast`'s expressions), checked before they are rendered; the wave keeps its planned program in `compiled_plans` (`waves::matches`). Left out of the
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

pub(in crate::code_rule_dsl) mod corpus;
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
    /// Batch inputs: relations a wave reads from its chunk's `demand(batch)`, which have no
    /// statistics outside a batch. Each is read from its source and planned from its row estimate.
    batch: HashMap<String, (Source, f64)>,
    /// The planner's statistics, measured by the caller from the inputs in memory (`corpus::run`): nothing is read
    /// or counted at plan time.
    given: Option<Stats>,
    /// Outputs left in the native engine (`DERIVED`) for the caller: neither written to their tables nor decoded.
    kept: BTreeSet<String>,
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
        Inputs { facts: "f.".to_owned(), stats: "f.".to_owned(), atoms: None, native: BTreeSet::new(), batch: HashMap::new(), given: None, kept: BTreeSet::new() }
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
#[derive(Clone, Debug)]
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
        if let Some(given) = &inputs.given {
            return Ok(given.clone());
        }
        Self::read(db, prog, inputs, true)
    }

    /// The statistics in schema `stats` (T's, atomized) for `prog`'s constants, read once; no fingerprint (the
    /// caller then sets what it measures, `measured`).
    pub(in crate::code_rule_dsl) fn of_schema(db: &Connection, prog: &Program, stats: &str) -> Result<Self> {
        let inputs = Inputs { facts: "main.".to_owned(), stats: format!("{stats}."), atoms: Some([0; 3]), native: BTreeSet::new(), batch: HashMap::new(), given: None, kept: BTreeSet::new() };
        Self::read(db, prog, &inputs, false)
    }

    /// No statistics but the program's kindsets: the caller measures every input (`measured`, `tree`), as the
    /// typing programs' own ingestion does (`corpus::run` without T's facts, list step 29).
    pub(in crate::code_rule_dsl) fn empty(prog: &Program) -> Self {
        Stats {
            col: HashMap::new(),
            kfreq: IndexMap::new(),
            size: HashMap::new(),
            nodes: 0.0,
            parents: IndexMap::new(),
            card: HashMap::new(),
            fan: HashMap::new(),
            dfreq: HashMap::new(),
            obs: HashMap::new(),
            solving: BTreeSet::new(),
            indexes: HashMap::new(),
            docs: None,
            memo: HashMap::new(),
            steps_cache: RefCell::new(HashMap::new()),
            kindsets: prog.kindsets.clone(),
        }
    }

    /// The tree's statistics measured in memory, replacing what was read (T's `stat_kind`, `stat_edge` and
    /// `stat_docrange`'s count): per kind atom its nodes and their summed subtree sizes, per (parent kind, child
    /// kind) its edges, and the documents.
    pub(in crate::code_rule_dsl) fn tree(&mut self, kinds: &BTreeMap<i64, (i64, i64)>, edges: &BTreeMap<(i64, i64), i64>, docs: i64) {
        self.kfreq = kinds.iter().map(|(k, (n, _))| (k.to_string(), *n)).collect();
        self.size = kinds.iter().filter(|(_, (n, _))| *n != 0).map(|(k, (n, size))| (k.to_string(), *size as f64 / *n as f64)).collect();
        self.nodes = kinds.values().map(|(n, _)| n).sum::<i64>() as f64;
        let mut parents: IndexMap<String, IndexMap<String, i64>> = IndexMap::new();
        for ((p, c), n) in edges {
            *parents.entry(c.to_string()).or_default().entry(p.to_string()).or_default() += n;
        }
        self.parents = parents
            .into_iter()
            .map(|(c, ps)| {
                let total = ps.values().sum::<i64>() as f64;
                (c, ps.into_iter().map(|(p, n)| (p, n as f64 / total)).collect())
            })
            .collect();
        self.docs = Some(docs);
        self.steps_cache.borrow_mut().clear();
    }

    /// Whether any column of `rel` has statistics (read or measured).
    pub(in crate::code_rule_dsl) fn knows(&self, rel: &str) -> bool {
        self.col.keys().any(|(r, _)| r == rel)
    }

    /// A relation's statistics measured in memory: its rows and per column (rows, ndv, sq, sqd), replacing what was
    /// read; with `vals`, a value column's exact postings per value (`stat_val`'s `n`).
    pub(in crate::code_rule_dsl) fn measured(&mut self, rel: &str, cols: impl IntoIterator<Item = (String, [i64; 4])>, vals: impl IntoIterator<Item = (String, i64, i64)>) {
        for (col, m) in cols {
            self.col.insert((rel.to_owned(), col), m);
        }
        for (col, v, n) in vals {
            self.memo.insert((rel.to_owned(), col, Constant::Int(v)), n);
        }
    }

    fn read(db: &Connection, prog: &Program, inputs: &Inputs, fingerprint: bool) -> Result<Self> {
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
        for t in prog.inputs.iter().filter(|t| fingerprint && !inputs.batch.contains_key(*t)) {
            sql += &format!(" UNION ALL SELECT 'rows', '{t}', NULL, count(*), NULL, NULL, NULL FROM {f}{t}");
        }
        let params: Vec<SqlValue> = consts
            .iter()
            .map(|c| match c {
                Constant::Int(i) => SqlValue::Integer(*i),
                Constant::Str(s) => SqlValue::Text(s.clone()),
            })
            .collect();
        // A program over batch inputs alone reads no statistics.
        let only_batch = prog.inputs.iter().all(|t| inputs.batch.contains_key(t));
        let rows: Vec<(String, [SqlValue; 6])> = (|| {
            if only_batch {
                return Ok(Vec::new());
            }
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
    super::sql::select::input_columns(rel).and_then(|names| names.get(col)).map_or_else(|| format!("c{col}"), |&n| n.to_owned())
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
        Literal::Agg { var, op, over, atom } => {
            let mut need = atom.vars();
            need.remove(var);
            if let Some(o) = over {
                need.remove(o);
            }
            if let AggOp::Join { by, .. } = op {
                by.iter().for_each(|k| _ = need.remove(k));
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

/// Above this many join literals a body is ordered greedily, not searched (decision 0038).
const GREEDY: usize = 12;
/// A kept plan is searched again where its work per node grew more than this (decision 0038)...
const REPLAN: f64 = 4.0;
/// ...and by more than this many rows of work: below it, running the kept order costs less than a search (about
/// 1.6 ms for 12 joins, some 10^4 to 10^5 rows of native joins). A plan made where a text test matched nothing
/// prices at a few rows, and any later chunk is 4 times that.
const REPLAN_ROWS: f64 = 10_000.0;

thread_local! {
    /// Plans kept across a corpus run's chunks, on the thread that runs them: per rule (head, body, delta, the
    /// component solved), its order and its work per node when planned. `None` outside a corpus run: every plan is
    /// searched.
    static PLANS: std::cell::RefCell<Option<HashMap<String, (Vec<Step>, f64)>>> = const { std::cell::RefCell::new(None) };
}

/// Plans kept across chunks on this thread while held (a corpus run), none kept at the start.
pub(in crate::code_rule_dsl) struct KeepPlans;

impl KeepPlans {
    pub(in crate::code_rule_dsl) fn start() -> Self {
        PLANS.with(|p| *p.borrow_mut() = Some(HashMap::new()));
        KeepPlans
    }
}

impl Drop for KeepPlans {
    fn drop(&mut self) {
        PLANS.with(|p| *p.borrow_mut() = None);
    }
}

/// A rule body in plan order, its estimated work and its estimated output rows.
pub(in crate::code_rule_dsl) type Plan = (Vec<Step>, f64, f64);

impl Model {
    fn card(&self, rel: &str) -> f64 {
        // `node` is the corpus's nodes, except where a batch binds it: then its estimate is the batch's.
        if rel == "node" && !self.inputs.batch.contains_key(rel) { self.stats.nodes } else { self.stats.card.get(rel).copied().unwrap_or(1000.0) }
    }

    fn ndv_or_1(&self, rel: &str, c: usize) -> f64 {
        self.ndv.get(&(rel.to_owned(), c)).copied().unwrap_or(1.0).max(1.0)
    }

    /// Orders a rule body (T's `plan_rule`). Dynamic programming over subsets of the join literals (Selinger): the
    /// cheapest order of each subset, with the filters applied as early as they can run. Work counts rows produced per
    /// step plus tape reads for `nearest` (climb: steps per row; descent: mean subtree size per row). With `first`
    /// (a body position), only orders anchored on that atom (a semi-naive delta).
    pub(in crate::code_rule_dsl) fn plan_rule(&self, prog: &Program, head: &Atom, body: &[Literal], first: Option<usize>) -> Result<Plan> {
        Ok(self.plan_rule_explained(prog, head, body, first)?.0)
    }

    /// `plan_rule`, and when profiling, the plan as EXPLAIN shows it (`profile::RulePlan`): the search's size, and per
    /// step its access method, source and the rows expected after it.
    pub(in crate::code_rule_dsl) fn plan_rule_explained(
        &self,
        prog: &Program,
        head: &Atom,
        body: &[Literal],
        first: Option<usize>,
    ) -> Result<(Plan, Option<super::profile::RulePlan>)> {
        self.plan_inner(prog, head, body, first, None)
    }

    /// `plan_rule_explained`, reusing this rule's plan from an earlier chunk (decision 0038): the kept order is
    /// re-priced with this chunk's statistics, and searched again only where its work per node grew more than
    /// `REPLAN` times, or it no longer runs (a relation it reads is missing). The planner's cache lives for a
    /// corpus run (`reset_plans`).
    pub(in crate::code_rule_dsl) fn plan_rule_reused(
        &self,
        prog: &Program,
        head: &Atom,
        body: &[Literal],
        first: Option<usize>,
    ) -> Result<(Plan, Option<super::profile::RulePlan>)> {
        let key_span = super::profile::span(|| "key".into());
        let key = format!("{head} :- {} @{first:?} /{}", body.iter().map(|l| l.to_string()).collect::<Vec<_>>().join(", "), self.stats.solving.iter().cloned().collect::<Vec<_>>().join(","));
        drop(key_span);
        let nodes = self.stats.nodes.max(1.0);
        let Some(kept) = PLANS.with(|p| p.borrow().as_ref().map(|p| p.get(&key).cloned())) else {
            return self.plan_inner(prog, head, body, first, None);
        };
        if let Some((order, per_node)) = kept {
            let priced = {
                let _span = super::profile::span(|| "re-price".into());
                self.plan_inner(prog, head, body, first, Some(&order))
            };
            match priced {
                Ok(got) if got.0.1 / nodes <= REPLAN * per_node.max(1e-12) || got.0.1 - per_node * nodes <= REPLAN_ROWS => {
                    super::profile::decision("plan cache (decision 0038)", || "a kept plan reused, re-priced with this chunk's statistics".into());
                    return Ok(got);
                }
                Ok(_) => super::profile::decision("plan cache (decision 0038)", || format!("replanned: the kept order's work per node grew over {REPLAN}x, by over {REPLAN_ROWS} rows")),
                Err(_) => super::profile::decision("plan cache (decision 0038)", || "replanned: the kept order no longer runs".into()),
            }
        } else {
            super::profile::decision("plan cache (decision 0038)", || "planned (first sight of the rule)".into());
        }
        let got = {
            let _span = super::profile::span(|| "search".into());
            self.plan_inner(prog, head, body, first, None)?
        };
        PLANS.with(|p| {
            if let Some(p) = p.borrow_mut().as_mut() {
                p.insert(key, (got.0.0.clone(), got.0.1 / nodes));
            }
        });
        Ok(got)
    }

    /// The search, or with `fixed`, the pricing of that order (an error where it cannot run).
    fn plan_inner(
        &self,
        prog: &Program,
        head: &Atom,
        body: &[Literal],
        first: Option<usize>,
        fixed: Option<&[Step]>,
    ) -> Result<(Plan, Option<super::profile::RulePlan>)> {
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
            // The delta atom of a probe is not such a scan: it reads the last wave's rows, priced by the relation's
            // live size (an upper bound), not the corpus.
            let delta = first.is_some_and(|f| matches!(&body[f], Literal::Atom(d) if std::ptr::eq(d, a)));
            if !delta && stats.solving.contains(&a.rel) && !a.args.iter().any(|t| matches!(t, Term::Var(v) if bound.contains(v))) {
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
        let long = body.iter().filter(|l| is_join(l)).count() > GREEDY;
        let step = |l: &Literal, rows: f64, bound: &BTreeSet<String>| -> Option<(f64, f64, Option<Mode>)> {
            // In a long body's greedy order, a step that reads with no bound variable (a scan, a cross product) is
            // priced from at least one binding (Postgres's `clamp_row_est`): an estimate near 0 would make every next
            // step look free (decision 0038). The search keeps T's pricing, which takes a cross product after an
            // unfilled relation (`magic_returns`) on purpose; a kept plan made where a relation was empty is re-priced
            // in the next chunk and replanned where it grew.
            let unbound = matches!(l, Literal::Atom(a) if !a.args.iter().any(|t| matches!(t, Term::Var(v) if bound.contains(v))));
            let rows = if long && unbound && !bound.is_empty() { rows.max(1.0) } else { rows };
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
        let mut candidates = 0u64;
        let (work, rows, order, subsets) = if let Some(fixed) = fixed {
            // The kept order, priced now: each join step as the search prices it, each filter once ready.
            let (mut work, mut rows, mut bound, mut order) = (0.0, 1.0, BTreeSet::new(), Vec::new());
            for s in fixed {
                let l = body.get(s.at).filter(|l| **l == s.lit).ok_or_else(|| Error::Plan("a kept plan of another body".into()))?;
                if is_join(l) {
                    let (w, out, mode) = step(l, rows, &bound).ok_or_else(|| Error::Plan("a kept step cannot run".into()))?;
                    work += w;
                    rows = out;
                    order.push(Step { at: s.at, lit: l.clone(), mode });
                } else {
                    if !ready(l, &bound) {
                        return Err(Error::Plan("a kept filter cannot run".into()));
                    }
                    if !(matches!(l, Literal::Cmp { op: "=", .. }) || matches!(l, Literal::Agg { .. })) {
                        rows *= 0.5;
                    }
                    order.push(s.clone());
                }
                bound.extend(l.vars());
            }
            (work, rows, order, 0)
        } else {
        type Best = (f64, f64, Vec<Step>, BTreeSet<String>, Vec<usize>);
        let mut subsets_reached = 0u64;
        let mut best_of: HashMap<u64, Best> = HashMap::new();
        best_of.insert(0, (0.0, 1.0, Vec::new(), BTreeSet::new(), (0..body.len()).filter(|&i| !is_join(&body[i])).collect()));
        // The greedy order's next step shares a variable with what is joined, while one can (no cross product while a
        // connected step remains, decision 0038). The search prices cross products itself: a cheap one with a tiny
        // relation is T's choice on its programs (`dec1` before `pv`: work 67, not 7,001).
        let connected = |l: &Literal, bound: &BTreeSet<String>| bound.is_empty() || !l.vars().is_disjoint(bound);
        let full_mask: u64 = (0..joins.len()).map(|i| 1u64 << i).sum();
        let disconnected = |i: usize, prev: u64, bound: &BTreeSet<String>| {
            !connected(&body[joins[i]], bound) && (0..joins.len()).any(|j| j != i && prev & (1 << j) == 0 && connected(&body[joins[j]], bound))
        };
        if joins.len() > GREEDY {
            // Long bodies are ordered greedily (Postgres's `geqo_threshold`): the cheapest connected step next.
            let (mut work, mut rows, mut order, mut bound, mut rest) = best_of.remove(&0).expect("the empty order");
            let mut done: u64 = 0;
            while done != full_mask {
                let mut best: Option<(usize, f64, f64, Option<Mode>)> = None;
                for i in 0..joins.len() {
                    let l = &body[joins[i]];
                    if done & (1 << i) != 0 || order.is_empty() && (!matches!(l, Literal::Atom(_)) || first.is_some_and(|f| f != joins[i])) {
                        continue;
                    }
                    if disconnected(i, done, &bound) {
                        continue;
                    }
                    let Some((w, out, mode)) = step(l, rows, &bound) else { continue };
                    candidates += 1;
                    if best.is_none_or(|b| w < b.1) {
                        best = Some((i, w, out, mode));
                    }
                }
                let Some((i, w, out, mode)) = best else { break };
                order.push(Step { at: joins[i], lit: body[joins[i]].clone(), mode });
                bound.extend(body[joins[i]].vars());
                rows = close(&mut order, out, &mut bound, &mut rest);
                work += w;
                done |= 1 << i;
            }
            if done == full_mask {
                best_of.insert(full_mask, (work, rows, order, bound, rest));
            }
        } else {
        // The search keeps per subset only its best cost and a back pointer (the last join, its mode, the subset
        // before). What a subset binds and which filters it has placed do not depend on the order that reached it
        // (readiness only grows with what is bound), so they are worked out once per subset, not per candidate; the
        // chosen order is rebuilt from the back pointers at the end, its filters placed as `close` places them.
        struct Cell {
            work: f64,
            rows: f64,
            prev: u64,
            last: usize,
            mode: Option<Mode>,
        }
        let filters: Vec<usize> = (0..body.len()).filter(|&i| !is_join(&body[i])).collect();
        let halves = |l: &Literal| !(matches!(l, Literal::Cmp { op: "=", .. }) || matches!(l, Literal::Agg { .. }));
        // A subset's bound variables and how many halving filters it has placed.
        let closure = |mask: u64| -> (BTreeSet<String>, usize) {
            let mut bound = BTreeSet::new();
            if mask == 0 {
                return (bound, 0);
            }
            for (i, &j) in joins.iter().enumerate() {
                if mask & (1 << i) != 0 {
                    bound.extend(body[j].vars());
                }
            }
            let mut placed = vec![false; filters.len()];
            let mut progressed = true;
            while progressed {
                progressed = false;
                for (k, &f) in filters.iter().enumerate() {
                    if !placed[k] && ready(&body[f], &bound) {
                        placed[k] = true;
                        bound.extend(body[f].vars());
                        progressed = true;
                    }
                }
            }
            let h = filters.iter().zip(&placed).filter(|(f, p)| **p && halves(&body[**f])).count();
            (bound, h)
        };
        // A join's pricing depends on which of its variables are bound (and, for a range read, those of the body's
        // `docof` atoms), not on the rows coming in: its factors are worked out once per such pattern, and each
        // candidate applies them to its rows exactly as `step` does.
        #[derive(Clone, Copy)]
        enum Price {
            No,
            Atom(f64, Option<f64>),
            Climb(f64),
            Descend(f64),
        }
        let docof_vars: BTreeSet<String> = body
            .iter()
            .filter_map(|d| match d {
                Literal::Atom(d) if !d.neg && self.docof.contains_key(&d.rel) => Some(d.vars()),
                _ => None,
            })
            .flatten()
            .collect();
        let relevant: Vec<Vec<String>> = joins
            .iter()
            .map(|&j| {
                let mut v = body[j].vars();
                if matches!(body[j], Literal::Atom(_)) {
                    v.extend(docof_vars.iter().cloned());
                }
                v.into_iter().collect()
            })
            .collect();
        let mut prices: HashMap<(usize, u64), Price> = HashMap::new();
        let mut price = |i: usize, bound: &BTreeSet<String>| -> Price {
            let vars = &relevant[i];
            let pattern = vars.iter().enumerate().fold(0u64, |m, (k, v)| if k < 64 && bound.contains(v) { m | 1 << k } else { m });
            let compute = || match &body[joins[i]] {
                Literal::Atom(a) => Price::Atom(atom_rows(a, bound), doc_range(a, bound).map(|(_, r)| r)),
                l @ Literal::Nearest { n, a, kindset } => {
                    if !ready(l, bound) {
                        Price::No
                    } else if bound.contains(n) {
                        Price::Climb(stats.steps(kinds.get(n.as_str()).map(String::as_str), kindset))
                    } else {
                        Price::Descend(kinds.get(a.as_str()).and_then(|k| stats.size.get(k)).copied().unwrap_or(stats.nodes / 1000.0))
                    }
                }
                _ => Price::No,
            };
            if vars.len() > 64 {
                return compute();
            }
            *prices.entry((i, pattern)).or_insert_with(compute)
        };
        // `step` (outside a long body's greedy order) from a price.
        let priced = |p: Price, rows: f64| -> Option<(f64, f64, Option<Mode>)> {
            match p {
                Price::No => None,
                Price::Atom(ar, range) => {
                    let out = rows * ar;
                    if let Some(r) = range
                        && rows * r < out
                    {
                        let out = rows * r;
                        return Some((out + rows, out, Some(Mode::Range)));
                    }
                    Some((out + rows, out, None))
                }
                Price::Climb(steps) => Some((rows * steps, rows, Some(Mode::Climb))),
                Price::Descend(size) => Some((rows * size, rows * size, Some(Mode::Descend))),
            }
        };
        let mut cells: HashMap<u64, Cell> = HashMap::new();
        let mut closed: HashMap<u64, (BTreeSet<String>, usize)> = HashMap::new();
        cells.insert(0, Cell { work: 0.0, rows: 1.0, prev: 0, last: usize::MAX, mode: None });
        closed.insert(0, closure(0));
        for size in 1..=joins.len() {
            for subset in combinations(joins.len(), size) {
                let mask: u64 = subset.iter().map(|&i| 1u64 << i).sum();
                let mut best: Option<Cell> = None;
                let mine = closure(mask);
                for i in py_set_order(&subset) {
                    let prev = mask & !(1 << i);
                    let Some(from) = cells.get(&prev) else { continue };
                    let l = &body[joins[i]];
                    if prev == 0 && !matches!(l, Literal::Atom(_)) {
                        continue; // an anchor is an atom
                    }
                    if prev == 0 && first.is_some_and(|f| f != joins[i]) {
                        continue; // semi-naive: the delta first
                    }
                    let (bound, h_prev) = &closed[&prev];
                    let Some((w, mut out, mode)) = priced(price(i, bound), from.rows) else { continue };
                    candidates += 1;
                    // `close`'s halving filters newly placed by this join.
                    for _ in *h_prev..mine.1 {
                        out *= 0.5;
                    }
                    let cand = Cell { work: from.work + w, rows: out, prev, last: i, mode };
                    if best.as_ref().is_none_or(|b| cand.work < b.work) {
                        best = Some(cand);
                    }
                }
                if let Some(b) = best {
                    cells.insert(mask, b);
                    closed.insert(mask, mine);
                }
            }
        }
        // The best orders, rebuilt: the full one, and (for `RDL_EXPLAIN`) the two-join prefixes.
        let rebuild = |mask: u64| -> Option<Best> {
            let cell = cells.get(&mask)?;
            let mut path = Vec::new();
            let mut at = mask;
            while at != 0 {
                let c = &cells[&at];
                path.push((c.last, c.mode));
                at = c.prev;
            }
            let (mut order, mut bound, mut rest) = (Vec::new(), BTreeSet::new(), filters.clone());
            for (i, mode) in path.into_iter().rev() {
                order.push(Step { at: joins[i], lit: body[joins[i]].clone(), mode });
                bound.extend(body[joins[i]].vars());
                close(&mut order, 1.0, &mut bound, &mut rest);
            }
            Some((cell.work, cell.rows, order, bound, rest))
        };
        let explain = std::env::var("RDL_EXPLAIN").is_ok_and(|r| r == head.rel);
        for &mask in cells.keys() {
            if mask == full_mask || (explain && mask.count_ones() == 2) {
                if let Some(b) = rebuild(mask) {
                    best_of.insert(mask, b);
                }
            }
        }
        subsets_reached = cells.len() as u64;
        }
        let full: u64 = (0..joins.len()).map(|i| 1u64 << i).sum();
        if std::env::var("RDL_EXPLAIN").is_ok_and(|r| r == head.rel) {
            // The chosen order's estimates step by step, and every two-step prefix's, for this head's rules.
            if let Some((work, rows, order, _, _)) = best_of.get(&full) {
                eprintln!("EXPLAIN {head} (delta {first:?}): work {work:.0} rows {rows:.1}");
                let mut bound = BTreeSet::new();
                let mut r = 1.0;
                for s in order {
                    if let Literal::Atom(a) = &s.lit && !a.neg {
                        let per = atom_rows(a, &bound);
                        r *= per;
                        // What every positive atom would cost here instead.
                        let others: Vec<String> = body
                            .iter()
                            .filter_map(|l| if let Literal::Atom(o) = l && !o.neg && !order.iter().take_while(|t| !std::ptr::eq(*t, s)).any(|t| t.lit == *l) { Some(format!("{}={:.2}", o.rel, atom_rows(o, &bound))) } else { None })
                            .collect();
                        eprintln!("    {:<40} per binding {per:.2}, rows {r:.1}   [{}]", s.lit.to_string(), others.join(" "));
                    } else {
                        eprintln!("    {}", s.lit);
                    }
                    bound.extend(s.lit.vars());
                }
            }
            for (mask, b) in &best_of {
                if mask.count_ones() == 2 {
                    eprintln!("  prefix {:?}: work {:.0} rows {:.1}", shown(&b.2), b.0, b.1);
                }
            }
        }
        let subsets = if subsets_reached > 0 { subsets_reached } else { best_of.len() as u64 };
        let Some((work, rows, order, _, rest)) = best_of.remove(&full) else {
            return Err(Error::Plan(format!("cannot order the body of {head}")));
        };
        if !rest.is_empty() {
            let rest: Vec<String> = rest.iter().map(|&i| body[i].to_string()).collect();
            return Err(Error::Plan(format!("unplaced literals {rest:?} in {head}")));
        }
        (work, rows, order, subsets)
        };
        let _explain = super::profile::span(|| "explain record".into());
        let explained = super::profile::on().then(|| {
            // The chosen order step by step, priced as the search priced it.
            let mut bound: BTreeSet<String> = BTreeSet::new();
            let mut r = 1.0;
            let mut steps = Vec::new();
            for s in &order {
                let cols = |a: &Atom, bound: &BTreeSet<String>| -> Vec<String> {
                    (0..a.args.len())
                        .filter(|&c| constant(&a.args[c]).is_some() || matches!(&a.args[c], Term::Var(v) if bound.contains(v)))
                        .map(|c| sql_col(&a.rel, c))
                        .collect()
                };
                let source = |rel: &str| -> String {
                    if rel.starts_with(super::strings::RELATION) {
                        "text test".into()
                    } else if prog.inputs.contains(&rel.to_owned()) {
                        "input".into()
                    } else if self.view_src.contains_key(rel) {
                        "view of an input".into()
                    } else {
                        "derived".into()
                    }
                };
                let (access, src) = match &s.lit {
                    Literal::Atom(a) if !a.neg => {
                        if let Some((_, out, _)) = step(&s.lit, r, &bound) {
                            r = out;
                        }
                        let c = cols(a, &bound);
                        let access = match s.mode {
                            Some(Mode::Range) => format!("range in document on ({})", c.join(", ")),
                            _ if c.is_empty() => "scan".to_owned(),
                            _ if c.len() == a.args.len() => format!("membership on ({})", c.join(", ")),
                            _ => format!("lookup on ({})", c.join(", ")),
                        };
                        (access, source(&a.rel))
                    }
                    Literal::Atom(a) => {
                        r *= 0.5;
                        (format!("anti-join on ({})", cols(a, &bound).join(", ")), source(&a.rel))
                    }
                    Literal::Nearest { .. } => {
                        if let Some((_, out, _)) = step(&s.lit, r, &bound) {
                            r = out;
                        }
                        (s.mode.map_or("nearest", Mode::name).to_owned() + " the tree", "tree".to_owned())
                    }
                    Literal::Cmp { op, .. } => {
                        if *op != "=" {
                            r *= 0.5;
                        }
                        (if *op == "=" { "assign or test" } else { "filter" }.to_owned(), String::new())
                    }
                    Literal::Agg { .. } => ("aggregate".to_owned(), String::new()),
                    _ => {
                        r *= 0.5;
                        ("filter".to_owned(), String::new())
                    }
                };
                bound.extend(s.lit.vars());
                steps.push(super::profile::StepPlan { literal: s.lit.to_string(), access, source: src, rows: r });
            }
            super::profile::RulePlan { work, rows, subsets, candidates, steps }
        });
        Ok(((order, work, rows), explained))
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
    for (rel, (_, rows)) in &inputs.batch {
        let rows = rows.max(1.0);
        stats.card.insert(rel.clone(), rows);
        // A base relation bound as a batch source is its vtab, which answers T's indexes' probes; any other
        // batch source is scanned.
        stats.indexes.insert(rel.clone(), vtab_indexes(rel));
        let names = super::sql::select::input_columns(rel);
        for col in 0..prog.decls.get(rel).map_or(0, Vec::len) {
            // A base relation's distinct values by column: a chunk's documents, kinds, child indexes, fields
            // and parents are far fewer than its rows. Other columns are taken as keys.
            let distinct = match names.and_then(|names| names.get(col)).copied() {
                Some("d") => rows.min(300.0),
                Some("kind") => rows.min(200.0),
                Some("idx") => rows.min(20.0),
                Some("f") => rows.min(50.0),
                Some("depth") => rows.min(30.0),
                Some("p" | "par") => (rows / 3.0).max(1.0),
                Some("s") => (rows / 5.0).max(1.0),
                _ => rows,
            };
            ndv.insert((rel.clone(), col), distinct);
            stats.fan.insert((rel.clone(), col), (rows / distinct).max(1.0));
        }
    }
    // An input no rule reads needs no statistics: a `.rr` program declares every `core.rr` relation (its prelude).
    let read: BTreeSet<&str> = prog.rules.iter().flat_map(|r| &r.body).filter_map(Literal::atom).map(|a| a.rel.as_str()).collect();
    for rel in prog.inputs.iter().filter(|rel| !inputs.batch.contains_key(*rel) && read.contains(rel.as_str())) {
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
        for rel in prog.inputs.iter().filter(|rel| !inputs.batch.contains_key(*rel)) {
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
        "text" | "name" | "str" | "strprefix" | "node_text" | "comment" => &[&["n"], &["s", "n"]],
        "value" | "static_value" => &[&["n"], &["v", "n"]],
        "value_type_of" => &[&["n"], &["t", "n"]],
        "value_type" => &[&["v"], &["t", "v"]],
        "fstring_piece" => &[&["n"], &["v", "n"]],
        "fstring_template" => &[&["n"], &["v", "n"]],
        "cpython_syntax_error" => &[&["d"]],
        "intval" | "op" | "tape" | "line" | "end_line" | "col" | "reference_open" => &[&["n"]],
        "reference_edge" => &[&["src"], &["dst"]],
        "core_node" => &[&["n"], &["t"], &["d"]],
        "doc" | "pkg" | "syntax_error" | "source" | "line_count" => &[&["d"]],
        "src_line" | "src_line_lf" | "tape_line" => &[&["d", "l"], &["sp"]],
        "rr_docof" => &[&["n"], &["d"]],
        "op_at" | "prev_token" => &[&["n"], &["s", "n"]],
        "token" => &[&["p", "i"], &["start"]],
        "visit_index" => &[&["n"]],
        "path_part" | "file_name" => &[&["d"], &["s", "d"]],
        // The transducer's `nearest` rows (`facts::vtab::syntax::with_nearest`), looked up either way.
        rel if rel.starts_with("nearest_") || rel.starts_with(crate::code_rule_dsl::logic::front::AXIS) => &[&["n"], &["a"]],
        // A text test's symbols (decision 0037, `logic::strings::as_relations`).
        rel if rel.starts_with(super::strings::RELATION) => &[&["s"]],
        // A `match_all` regex's matches, by span and by match (`facts::vtab::syntax::Extra`).
        rel if rel.starts_with(super::strings::MATCHES) => &[&["sp"], &["m"]],
        // A `match_group` regex's groups, by span, by match and by group span.
        rel if rel.starts_with(super::strings::GROUPS) => &[&["sp"], &["m"], &["gsp"]],
        _ => &[],
    };
    lists.iter().map(|l| l.iter().map(|c| (*c).to_owned()).collect()).collect()
}

// ---- SQL backend -------------------------------------------------------------------------------------------------
//
// A rule body is built as a typed query (`sql::select`), checked against the program's relations and the
// columns in scope, then rendered (list step 20).

use super::sql::select::{Cte, FromItem, Join, Query, Relation, Select, Source};
use super::sql::sql_ast::{SqlBinaryOperator as Op, SqlExpression as Expr, SqlUnaryOperator, SqlValue as Value_};

fn integer(i: i64) -> Expr {
    Expr::Value(Value_::Integer(i))
}
fn string(s: &str) -> Expr {
    Expr::Value(Value_::Text(s.to_owned()))
}
fn col(alias: &str, column: &str) -> Expr {
    Expr::Qualified { alias: alias.to_owned(), column: column.to_owned() }
}
fn bin(operator: Op, left: Expr, right: Expr) -> Expr {
    Expr::Binary { operator, left: Box::new(left), right: Box::new(right) }
}
fn eq(left: Expr, right: Expr) -> Expr {
    bin(Op::Equal, left, right)
}
fn call(name: &'static str, arguments: Vec<Expr>) -> Expr {
    Expr::Call { name, arguments }
}
fn length(e: &Expr) -> Expr {
    call("length", vec![e.clone()])
}
fn substr(arguments: Vec<Expr>) -> Expr {
    call("substr", arguments)
}
/// A symbol's text: `sym_text` of it in atoms mode.
fn text_of(atoms: Option<[i64; 3]>, e: &Expr) -> Expr {
    if atoms.is_some() { call("sym_text", vec![e.clone()]) } else { e.clone() }
}
/// A text's symbol: `sym_atom` of it in atoms mode.
fn symbol_of(atoms: Option<[i64; 3]>, e: Expr) -> Expr {
    if atoms.is_some() { call("sym_atom", vec![e]) } else { e }
}

fn sql_term(t: &Term, env: &HashMap<String, Expr>, atoms: Option<[i64; 3]>) -> Result<Expr> {
    Ok(match t {
        Term::Var(v) => env.get(v).cloned().ok_or_else(|| Error::Plan(format!("${v} is unbound in SQL")))?,
        Term::Int(i) => integer(*i),
        Term::Str(s) => string(s),
        Term::Top => integer(TOP),
        Term::Call(f, args) => {
            let a = args.iter().map(|x| sql_term(x, env, atoms)).collect::<Result<Vec<_>>>()?;
            match (f.as_str(), a.as_slice()) {
                // Membership of a kind or field among atoms (`plan::lower`): 1 or 0.
                ("in", [x, _]) => {
                    let items: Vec<&Term> = match &args[1] {
                        Term::Call(g, items) if g == super::strings::SET => items.iter().collect(),
                        one => vec![one],
                    };
                    let mut out = None;
                    for item in items {
                        let e = eq(x.clone(), sql_term(item, env, atoms)?);
                        out = Some(match out {
                            None => e,
                            Some(o) => bin(Op::Or, o, e),
                        });
                    }
                    out.unwrap_or_else(|| integer(0))
                }
                // A literal test on a name or path (`logic::strings`): the atoms' text in atoms mode.
                (f, [x, y]) if super::strings::is_test(f) => {
                    let text = |e: &Expr| if atoms.is_some() { call("sym_text", vec![e.clone()]) } else { e.clone() };
                    let op = super::strings::TESTS.iter().find(|t| **t == f).expect("a test");
                    // A pattern set is its patterns' text already (`set` below).
                    let set = matches!(&args[1], Term::Call(g, _) if g == super::strings::SET);
                    call("str_test", vec![string(op), text(x), if set { y.clone() } else { text(y) }])
                }
                // A pattern set (decision 0037): its patterns' text joined by `strings::SET_SEP`.
                (f, items) if f == super::strings::SET => {
                    let text = |e: &Expr| if atoms.is_some() { call("sym_text", vec![e.clone()]) } else { e.clone() };
                    let sep = string(&super::strings::SET_SEP.to_string());
                    let mut out = items.first().map(text).unwrap_or_else(|| string(""));
                    for item in items.iter().skip(1) {
                        out = bin(Op::Concatenate, bin(Op::Concatenate, out, sep.clone()), text(item));
                    }
                    out
                }
                // A node's document (`typing::cross_rounds`): nodes are `d << 32 | index`.
                ("doc", [x]) => bin(Op::Divide, x.clone(), integer(1i64 << 32)),
                // A sibling's position (`logic::convert`: `precedes`, `follows`), and `.rr`'s arithmetic.
                ("add", [x, y]) => bin(Op::Add, x.clone(), y.clone()),
                ("sub", [x, y]) => bin(Op::Subtract, x.clone(), y.clone()),
                ("mul", [x, y]) => bin(Op::Multiply, x.clone(), y.clone()),
                ("div", [x, y]) => bin(Op::Divide, x.clone(), y.clone()),
                // `.rr`'s string functions (Soufflé's): `substr` counts from 0.
                ("strlen", [x]) => length(&text_of(atoms, x)),
                ("concat", [x, y]) => symbol_of(atoms, bin(Op::Concatenate, text_of(atoms, x), text_of(atoms, y))),
                ("to_string", [x]) => symbol_of(atoms, bin(Op::Concatenate, x.clone(), string(""))),
                // A named group of a regex's match (`.rr`'s `match` with captures, `strings::group`); NULL without one.
                ("rx_group", [x, re, g]) => symbol_of(atoms, call("rx_group", vec![text_of(atoms, x), text_of(atoms, re), text_of(atoms, g)])),
                ("substr", [x, i, n]) => symbol_of(atoms, substr(vec![text_of(atoms, x), bin(Op::Add, i.clone(), integer(1)), n.clone()])),
                ("cat", [x, y]) if atoms.is_some() => call("sym_cat", vec![x.clone(), y.clone()]),
                ("cat", [x, y]) => bin(Op::Concatenate, bin(Op::Concatenate, x.clone(), string(".")), y.clone()),
                // core.rr's functors whose meaning is W's (`register_functors`), ahead of `strings::functions`' below.
                ("replace_all", [s, re, by]) => symbol_of(atoms, call("rr_replace_all", vec![text_of(atoms, s), text_of(atoms, re), text_of(atoms, by)])),
                // A JSON string literal as W's signed-list stages write one (`json_group_array`'s elements): SQLite's
                // `json_quote` of the text.
                ("json_str", [x]) => symbol_of(atoms, call("json_quote", vec![bin(Op::Concatenate, text_of(atoms, x), string(""))])),
                ("ed25519_ok", [k, m, sig]) => call("rr_ed25519_ok", vec![text_of(atoms, k), text_of(atoms, m), text_of(atoms, sig)]),
                ("rx_replace", [s, re, by]) => symbol_of(atoms, call("rr_rx_replace", vec![text_of(atoms, s), text_of(atoms, re), text_of(atoms, by)])),
                ("flat_bin", [o, x, y]) => {
                    let big = integer(1i64 << 40);
                    let [plus, minus, times] = match atoms {
                        Some(ops) => ops.map(integer),
                        None => ["+", "-", "*"].map(string),
                    };
                    let small = |e: &Expr| bin(Op::Less, call("abs", vec![e.clone()]), big.clone());
                    Expr::Case {
                        branches: vec![
                            (bin(Op::Or, eq(x.clone(), integer(TOP)), eq(y.clone(), integer(TOP))), integer(TOP)),
                            (eq(o.clone(), plus), bin(Op::Add, x.clone(), y.clone())),
                            (eq(o.clone(), minus), bin(Op::Subtract, x.clone(), y.clone())),
                            (bin(Op::And, bin(Op::And, eq(o.clone(), times), small(x)), small(y)), bin(Op::Multiply, x.clone(), y.clone())),
                        ],
                        fallback: Box::new(integer(TOP)),
                    }
                }
                // `core.rr`'s text functors (`strings::functions`): symbols in as text, a symbol result back.
                (f, items) if let Some((f, kinds, ret)) = super::strings::functions::signature(f) && kinds.len() == items.len() => {
                    use super::strings::functions::Kind;
                    let items = items.iter().zip(kinds).map(|(e, k)| if *k == Kind::Text { text_of(atoms, e) } else { e.clone() }).collect();
                    let value = call(f, items);
                    if ret == Kind::Text { symbol_of(atoms, value) } else { value }
                }
                _ => return Err(Error::Plan(format!("no SQL for {t}"))),
            }
        }
        Term::Wild => return Err(Error::Plan("no SQL for _".to_owned())),
    })
}

/// A kind set's members as SQL values: kind atoms, or kind names.
fn kinds(members: &[String]) -> Vec<Expr> {
    members.iter().map(|k| k.parse::<i64>().map_or_else(|_| string(k), integer)).collect()
}

impl Inputs {
    /// A symbol expression as text: `sym_text` of it in atoms mode.
    fn text_of(&self, e: &Expr) -> Expr {
        if self.atoms.is_some() { call("sym_text", vec![e.clone()]) } else { e.clone() }
    }
    /// `sym_atom` of a text expression in atoms mode.
    fn symbol_of(&self, e: Expr) -> Expr {
        if self.atoms.is_some() { call("sym_atom", vec![e]) } else { e }
    }
}

impl Model {
    /// Where a rule reads `rel`: its batch source, the facts' input, or the derived table (or its delta).
    fn relation(&self, prog: &Program, rel: &str, delta: bool) -> Relation {
        if let Some((source, _)) = self.inputs.batch.get(rel) {
            Relation::Batch { rel: rel.to_owned(), source: source.clone() }
        } else if prog.inputs.iter().any(|i| i == rel) {
            Relation::Input { schema: self.inputs.facts.clone(), rel: rel.to_owned() }
        } else {
            Relation::Derived { rel: rel.to_owned(), delta }
        }
    }

    /// Whether a statement of `prog` may read `relation`: its inputs, batch inputs and derived relations, and
    /// the tape relations the `nearest` walks read, and views the run planned from.
    fn declared(&self, prog: &Program, relation: &Relation) -> bool {
        match relation {
            Relation::Input { rel, .. } => prog.inputs.contains(rel) || matches!(rel.as_str(), "tape" | "node"),
            Relation::Batch { rel, .. } => self.inputs.batch.contains_key(rel),
            Relation::Derived { rel, .. } | Relation::Bound { rel, .. } => {
                // Or a view the run made and planned from (`rdl_kinds`: its statistics are the model's).
                prog.decls.contains_key(rel) || prog.rules.iter().any(|rule| rule.head.rel == *rel) || self.stats.card.contains_key(rel)
            }
            _ => true,
        }
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
        Ok(self.checked_query(prog, head, order, delta_at, tables)?.render())
    }

    /// `sql_select`'s query, checked, before it is rendered.
    pub(in crate::code_rule_dsl) fn checked_query(
        &self,
        prog: &Program,
        head: &Atom,
        order: &[Step],
        delta_at: Option<usize>,
        tables: Option<&HashMap<String, (String, String)>>,
    ) -> Result<Query> {
        let query = self.sql_query(prog, head, order, delta_at, tables)?;
        query.check(&|relation| self.declared(prog, relation)).map_err(|e| Error::Plan(format!("the SQL for {head}: {e}")))?;
        Ok(query)
    }

    /// `sql_select`'s query, before it is checked.
    pub(in crate::code_rule_dsl) fn sql_query(
        &self,
        prog: &Program,
        head: &Atom,
        order: &[Step],
        delta_at: Option<usize>,
        tables: Option<&HashMap<String, (String, String)>>,
    ) -> Result<Query> {
        let atoms = self.inputs.atoms;
        let (mut env, mut frm, mut wh, mut ctes) = (HashMap::<String, Expr>::new(), Vec::<(FromItem, Vec<Expr>)>::new(), Vec::<Expr>::new(), Vec::<Cte>::new());
        let mut alias = 0;
        let mut a_name = || {
            alias += 1;
            format!("t{alias}")
        };
        let neg_conds = |a: &Atom, al: &str, env: &HashMap<String, Expr>, skip: &[&str]| -> Result<Vec<Expr>> {
            a.args
                .iter()
                .enumerate()
                .filter(|(_, t)| !matches!(t, Term::Wild) && !matches!(t, Term::Var(v) if skip.contains(&v.as_str())))
                .map(|(c, t)| Ok(eq(col(al, &sql_col(&a.rel, c)), sql_term(t, env, atoms)?)))
                .collect()
        };
        let subquery = |what: Expr, relation: Relation, al: String, predicates: Vec<Expr>| Select {
            columns: vec![(what, None)],
            from: vec![FromItem::cross(relation, al)],
            predicates,
            ..Select::default()
        };
        for (idx, s) in order.iter().enumerate() {
            match &s.lit {
                Literal::Atom(l) if !l.neg => {
                    let al = a_name();
                    let relation = match tables.and_then(|t| t.get(&l.rel)) {
                        Some((full, delta)) => {
                            Relation::Bound { rel: l.rel.clone(), table: if Some(idx) == delta_at { delta.clone() } else { full.clone() } }
                        }
                        None => self.relation(prog, &l.rel, Some(idx) == delta_at),
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
                        let range = FromItem::cross(Relation::DocRange { schema: self.inputs.stats.clone() }, ra.clone());
                        frm.push((range, vec![eq(col(&ra, "d"), sql_term(&dv, &env, atoms)?)]));
                        let rc = l.args.iter().position(|t| matches!(t, Term::Var(x) if *x == v)).expect("the range column");
                        conds.push(Expr::Between {
                            input: Box::new(col(&al, &sql_col(&l.rel, rc))),
                            low: Box::new(col(&ra, "lo")),
                            high: Box::new(col(&ra, "hi")),
                        });
                    }
                    for (c, t) in l.args.iter().enumerate() {
                        let column = col(&al, &sql_col(&l.rel, c));
                        match t {
                            Term::Var(v) => match env.get(v) {
                                Some(e) => conds.push(eq(column, e.clone())),
                                None => {
                                    env.insert(v.clone(), column);
                                }
                            },
                            Term::Int(_) | Term::Str(_) | Term::Top => conds.push(eq(column, sql_term(t, &env, atoms)?)),
                            _ => {}
                        }
                    }
                    frm.push((FromItem::cross(relation, al), conds));
                }
                Literal::Atom(l) => {
                    let al = a_name();
                    let conds = neg_conds(l, &al, &env, &[])?;
                    let query = subquery(integer(1), self.relation(prog, &l.rel, false), al, conds);
                    wh.push(Expr::Exists { negated: true, query: Box::new(query) });
                }
                Literal::Cmp { l, op, r } => {
                    let lv = if let Term::Var(v) = l { Some(v) } else { None };
                    if *op == "=" && lv.is_some_and(|v| !env.contains_key(v)) {
                        let e = sql_term(r, &env, atoms)?;
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
                        let w = inputs.text_of(&env[lv]);
                        let Term::Call(_, parts) = r else { unreachable!() };
                        let (Term::Var(q), Term::Var(x)) = (&parts[0], &parts[1]) else { unreachable!() };
                        if let Some(eq_) = env.get(q).map(|e| inputs.text_of(e)) {
                            wh.push(eq(
                                substr(vec![w.clone(), integer(1), bin(Op::Add, length(&eq_), integer(1))]),
                                bin(Op::Concatenate, eq_.clone(), string(".")),
                            ));
                            env.insert(x.clone(), inputs.symbol_of(substr(vec![w, bin(Op::Add, length(&eq_), integer(2))])));
                        } else if let Some(ex) = env.get(x).map(|e| inputs.text_of(e)) {
                            let negated = Expr::Unary { operator: SqlUnaryOperator::Negate, input: Box::new(length(&ex)) };
                            wh.push(eq(substr(vec![w.clone(), bin(Op::Subtract, negated, integer(1))]), bin(Op::Concatenate, string("."), ex.clone())));
                            wh.push(bin(Op::Greater, length(&w), length(&ex)));
                            let end = bin(Op::Subtract, bin(Op::Subtract, length(&w), length(&ex)), integer(1));
                            env.insert(q.clone(), inputs.symbol_of(substr(vec![w, integer(1), end])));
                        } else {
                            let al = a_name();
                            let at = col(&al, "i");
                            let conds = vec![bin(Op::LessOrEqual, at.clone(), length(&w)), eq(substr(vec![w.clone(), at.clone(), integer(1)]), string("."))];
                            frm.push((FromItem::cross(Relation::Integers, al), conds));
                            env.insert(q.clone(), inputs.symbol_of(substr(vec![w.clone(), integer(1), bin(Op::Subtract, at.clone(), integer(1))])));
                            env.insert(x.clone(), inputs.symbol_of(substr(vec![w, bin(Op::Add, at, integer(1))])));
                        }
                    } else {
                        let operator = match *op {
                            "=" => Op::Equal,
                            "!=" | "<>" => Op::NotEqual,
                            "<" => Op::Less,
                            "<=" => Op::LessOrEqual,
                            ">" => Op::Greater,
                            ">=" => Op::GreaterOrEqual,
                            o => return Err(Error::Plan(format!("no SQL for the comparison {o}"))),
                        };
                        wh.push(bin(operator, sql_term(l, &env, atoms)?, sql_term(r, &env, atoms)?));
                    }
                }
                Literal::Agg { var, op, over, atom } => {
                    let al = a_name();
                    // The aggregate's own variables are read from its rows, not tested: the aggregated one, and
                    // join's order where the rule has not bound it.
                    let mut own: Vec<&str> = over.as_deref().into_iter().collect();
                    if let AggOp::Join { by, .. } = op {
                        own.extend(by.iter().map(String::as_str).filter(|k| !env.contains_key(*k)));
                    }
                    let conds = neg_conds(atom, &al, &env, &own)?;
                    // A variable's column in the aggregate's rows, as text where it is a symbol (`sym_text` of its
                    // atom), so that join orders and writes symbols by their text.
                    let column = |v: &str, text: bool| {
                        let c = atom.args.iter().position(|t| matches!(t, Term::Var(x) if x == v)).expect("the aggregated column");
                        let e = col(&al, &sql_col(&atom.rel, c));
                        if text && prog.decls.get(&atom.rel).is_some_and(|d| d[c].1 == "sym") { text_of(atoms, &e) } else { e }
                    };
                    let what = match (op, over) {
                        (AggOp::Count, _) | (_, None) => call("count", vec![Expr::All]),
                        (AggOp::Min | AggOp::Max, Some(o)) => call(if *op == AggOp::Min { "min" } else { "max" }, vec![column(o, false)]),
                        (AggOp::Sum, Some(o)) => call("coalesce", vec![call("sum", vec![column(o, false)]), integer(0)]),
                        (AggOp::Join { by, sep }, Some(o)) => {
                            let args = [string(sep), column(o, true)].into_iter().chain(by.iter().map(|k| column(k, true))).collect();
                            symbol_of(atoms, call("rr_join", args))
                        }
                    };
                    let query = Expr::Scalar(Box::new(subquery(what, self.relation(prog, &atom.rel, false), al, conds)));
                    // A result already bound (`def(d, x, s, …), s = max t : { … }`) is compared with it, not rebound.
                    match env.get(var) {
                        Some(bound) => wh.push(eq(bound.clone(), query)),
                        None => {
                            env.insert(var.clone(), query);
                        }
                    }
                }
                Literal::Nearest { n, a, kindset } if s.mode == Some(Mode::Climb) && self.inputs.native.contains("nearest") => {
                    // The climb is a native function over the tape: no CTE, and its result is just a column expression.
                    let near = call("tape_nearest", vec![env[n].clone(), string(&prog.kindsets[kindset].join(","))]);
                    if let Some(e) = env.get(a) {
                        wh.push(eq(near, e.clone()));
                    } else {
                        env.insert(a.clone(), near.clone());
                        wh.push(Expr::Unary { operator: SqlUnaryOperator::IsNotNull, input: Box::new(near) });
                    }
                }
                Literal::Nearest { n, a, kindset } => {
                    // A lowered program's members are kind atoms.
                    let ks = kinds(&prog.kindsets[kindset]);
                    let member = |e: Expr| Expr::In { input: Box::new(e), values: ks.clone() };
                    let input = |rel: &str| Relation::Input { schema: self.inputs.facts.clone(), rel: rel.to_owned() };
                    let on = |rel: &str, alias: &str, on: Vec<Expr>| FromItem { relation: input(rel), alias: alias.to_owned(), join: Join::On(on) };
                    // Everything joined so far becomes the seed CTE; the walk is a recursive CTE seeded only from it.
                    let k = ctes.len();
                    let (seed, walk, near) = (format!("seed{k}"), format!("walk{k}"), format!("near{k}"));
                    let seed_cols: Vec<String> = env.keys().cloned().collect::<BTreeSet<_>>().into_iter().collect();
                    if frm.is_empty() {
                        return Err(Error::Plan(format!("the walk of {} reads nothing before it", s.lit)));
                    }
                    let (items, conds): (Vec<FromItem>, Vec<Vec<Expr>>) = frm.drain(..).unzip();
                    ctes.push(Cte::walk(
                        seed.clone(),
                        None,
                        vec![Select {
                            distinct: true,
                            columns: seed_cols.iter().map(|v| (env[v].clone(), Some(v.clone()))).collect(),
                            from: items,
                            predicates: conds.into_iter().flatten().chain(wh.drain(..)).collect(),
                        }],
                    ));
                    // The seed's columns are threaded through the walk (ast-index-spike S050 `_path_threaded`), so the
                    // walk's output is the extended row and nothing joins back to the seed.
                    let threaded = |alias: &str| seed_cols.iter().map(|v| (col(alias, v), None)).collect::<Vec<_>>();
                    let with = |alias: &str, extra: Vec<Expr>| threaded(alias).into_iter().chain(extra.into_iter().map(|e| (e, None))).collect::<Vec<_>>();
                    let mut walk_columns = seed_cols.clone();
                    let out_var = if s.mode == Some(Mode::Climb) {
                        walk_columns.push("cur".to_owned());
                        let above = bin(Op::GreaterOrEqual, col("t", "par"), integer(0));
                        let arms = vec![
                            Select {
                                columns: with("s", vec![col("t", "par")]),
                                from: vec![FromItem::cross(Relation::Cte(seed.clone()), "s"), on("tape", "t", vec![eq(col("t", "n"), col("s", n))])],
                                predicates: vec![above.clone()],
                                ..Select::default()
                            },
                            Select {
                                columns: with("w", vec![col("t", "par")]),
                                from: vec![
                                    FromItem::cross(Relation::Cte(walk.clone()), "w"),
                                    on("node", "k", vec![eq(col("k", "n"), col("w", "cur")), Expr::Unary { operator: SqlUnaryOperator::Not, input: Box::new(member(col("k", "kind"))) }]),
                                    on("tape", "t", vec![eq(col("t", "n"), col("w", "cur"))]),
                                ],
                                predicates: vec![above],
                                ..Select::default()
                            },
                        ];
                        ctes.push(Cte::walk(walk.clone(), Some(walk_columns), arms));
                        let mut predicates = vec![eq(col("k", "n"), col("w", "cur")), member(col("k", "kind"))];
                        if env.contains_key(a) {
                            predicates.push(eq(col("w", "cur"), col("w", a)));
                        }
                        let arm = Select {
                            columns: with("w", Vec::new()).into_iter().chain([(col("w", "cur"), Some("near_out".to_owned()))]).collect(),
                            from: vec![FromItem::cross(Relation::Cte(walk.clone()), "w"), FromItem::cross(input("node"), "k")],
                            predicates,
                            ..Select::default()
                        };
                        ctes.push(Cte::walk(near.clone(), None, vec![arm]));
                        a
                    } else {
                        walk_columns.extend(["cur".to_owned(), "stop".to_owned()]);
                        let size = col("t", "size");
                        let arms = vec![
                            Select {
                                columns: with("s", vec![bin(Op::Add, col("s", a), integer(1)), bin(Op::Add, col("s", a), size.clone())]),
                                from: vec![FromItem::cross(Relation::Cte(seed.clone()), "s"), on("tape", "t", vec![eq(col("t", "n"), col("s", a))])],
                                predicates: vec![bin(Op::Greater, size.clone(), integer(0))],
                                ..Select::default()
                            },
                            Select {
                                columns: with(
                                    "w",
                                    vec![
                                        Expr::Case {
                                            branches: vec![(member(col("k", "kind")), bin(Op::Add, bin(Op::Add, col("w", "cur"), size), integer(1)))],
                                            fallback: Box::new(bin(Op::Add, col("w", "cur"), integer(1))),
                                        },
                                        col("w", "stop"),
                                    ],
                                ),
                                from: vec![
                                    FromItem::cross(Relation::Cte(walk.clone()), "w"),
                                    on("node", "k", vec![eq(col("k", "n"), col("w", "cur"))]),
                                    on("tape", "t", vec![eq(col("t", "n"), col("w", "cur"))]),
                                ],
                                predicates: vec![bin(Op::Less, col("w", "cur"), col("w", "stop"))],
                                ..Select::default()
                            },
                        ];
                        ctes.push(Cte::walk(walk.clone(), Some(walk_columns), arms));
                        let mut predicates = vec![bin(Op::LessOrEqual, col("w", "cur"), col("w", "stop"))];
                        if env.contains_key(n) {
                            predicates.push(eq(col("w", "cur"), col("w", n)));
                        }
                        let arm = Select {
                            columns: with("w", Vec::new()).into_iter().chain([(col("w", "cur"), Some("near_out".to_owned()))]).collect(),
                            from: vec![FromItem::cross(Relation::Cte(walk.clone()), "w")],
                            predicates,
                            ..Select::default()
                        };
                        ctes.push(Cte::walk(near.clone(), None, vec![arm]));
                        n
                    };
                    env = seed_cols.iter().map(|v| (v.clone(), col(&near, v))).collect();
                    env.entry(out_var.clone()).or_insert_with(|| col(&near, "near_out"));
                    frm = vec![(FromItem::cross(Relation::Cte(near.clone()), near), Vec::new())];
                }
                Literal::Ast { .. } => return Err(Error::Plan(format!("no SQL for {}", s.lit))),
            }
        }
        let columns = head
            .args
            .iter()
            .enumerate()
            .map(|(i, t)| Ok((sql_term(t, &env, atoms)?, Some(format!("c{i}")))))
            .collect::<Result<Vec<_>>>()?;
        if frm.is_empty() {
            return Err(Error::Plan(format!("rule for {head} reads nothing")));
        }
        let (from, conds): (Vec<FromItem>, Vec<Vec<Expr>>) = frm.into_iter().unzip();
        let body = Select { distinct: true, columns, from, predicates: conds.into_iter().flatten().chain(wh).collect() };
        Ok(Query { ctes, body })
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
    /// The kept outputs (`Inputs::kept`) the native engine holds (`native::derived`); the others are in their tables.
    pub(in crate::code_rule_dsl) kept: BTreeSet<String>,
}

/// The set engine's component calls. Over atoms they are made in process (`native::component`), with no statement
/// each; over text symbols (T's extension loaded on the connection, which keeps its own symbols) through its SQL
/// functions.
struct Fx<'a> {
    db: &'a Connection,
    direct: bool,
}
impl Fx<'_> {
    fn open(&self, spec: &str) -> Result<()> {
        let _span = super::profile::span(|| "engine open (load inputs, build indexes)".into());
        if self.direct {
            super::native::component::open(self.db, spec)?;
        } else {
            self.db.query_row("SELECT set_fixpoint_open(?1)", [spec], |_| Ok(()))?;
        }
        Ok(())
    }
    fn run(&self, name: &str, waves: u32) -> Result<String> {
        let _span = super::profile::span(|| "engine run (joins)".into());
        Ok(if self.direct { super::native::component::run(self.db, name, waves)? } else { self.db.query_row("SELECT set_fixpoint_run(?1, ?2)", rusqlite::params![name, waves], |r| r.get(0))? })
    }
    /// Per probe, its steps' (in, out) bindings since the last call.
    fn steps(&self, name: &str) -> Result<Vec<Vec<(i64, i64)>>> {
        let _span = super::profile::span(|| "engine step counts".into());
        if self.direct {
            return Ok(super::native::component::step_counts(self.db, name)?);
        }
        let text: String = self.db.query_row("SELECT set_fixpoint_steps(?1)", [name], |r| r.get(0))?;
        Ok(text
            .split('\n')
            .map(|line| line.split(',').filter_map(|x| { let (i, o) = x.split_once(':')?; Some((i.parse().ok()?, o.parse().ok()?)) }).collect())
            .collect())
    }
    /// Each relation's live statistics, in spec order.
    fn stats(&self, name: &str, symbols_text: &dyn Fn(usize, usize) -> bool) -> Result<Vec<LiveRel>> {
        let _span = super::profile::span(|| "engine live statistics".into());
        if self.direct {
            return Ok(super::native::component::live(self.db, name)?
                .into_iter()
                .map(|l| LiveRel { n: l.n, ndv: l.ndv, sq: l.sq, freq: l.freq.into_iter().map(|(c, v)| (c, v.into_iter().map(|(v, m)| (Constant::Int(v), m)).collect())).collect() })
                .collect());
        }
        let text: String = self.db.query_row("SELECT set_fixpoint_stats(?1)", [name], |r| r.get(0))?;
        Ok(text.split('\n').enumerate().map(|(i, line)| LiveRel::parse(line, &|c| symbols_text(i, c))).collect())
    }
    fn reprobe(&self, name: &str, i: usize, kind: &str, body: &str) -> Result<()> {
        let _span = super::profile::span(|| "engine reprobe".into());
        if self.direct {
            super::native::component::reprobe(self.db, name, i, kind, body)?;
        } else {
            self.db.query_row("SELECT set_fixpoint_reprobe(?1, ?2, ?3, ?4)", rusqlite::params![name, i as i64, kind, body], |_| Ok(()))?;
        }
        Ok(())
    }
    fn close(&self, name: &str) -> Result<()> {
        let _span = super::profile::span(|| "engine close".into());
        if self.direct {
            super::native::component::close(self.db, name)?;
        } else {
            self.db.query_row("SELECT set_fixpoint_close(?1)", [name], |_| Ok(()))?;
        }
        Ok(())
    }
    fn materialize(&self, table: &str) -> Result<()> {
        let _span = super::profile::span(|| "engine materialize".into());
        if self.direct {
            super::native::component::materialize(self.db, table)?;
        } else {
            self.db.query_row("SELECT set_fixpoint_materialize(?1)", [table], |_| Ok(()))?;
        }
        Ok(())
    }
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
    /// Why the native engine declined the last stratum it was offered (EXPLAIN's `why`).
    why_not: Option<String>,
    /// Derived relations whose table is made only when SQL first needs it (`ensure`): its `CREATE` statement. A
    /// relation the native engine derives and keeps never has one.
    pending: HashMap<String, String>,
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

    /// The rows `select` added to `head`'s relation (keys changed, for a lattice).
    fn insert_rows(&self, head: &Atom, select: &str) -> Result<usize> {
        Ok(if self.prog.lattice.contains_key(&head.rel) {
            self.merge_lattice(&head.rel, select, false)?.max(0) as usize
        } else {
            let statement = format!("INSERT OR IGNORE INTO {} {select}", head.rel);
            super::profile::plan(self.db, &statement);
            self.db.execute(&statement, [])?
        })
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
fn observe(stats: &mut Stats, order: &[Step], counts: &[(i64, i64)]) {
    let mut bound = BTreeSet::new();
    for (i, (s, &(n_in, n_out))) in order.iter().zip(counts).enumerate() {
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
/// A column's type in the native store's spec: `s`, a symbol it reads as text, only where symbols are text. Over
/// atoms every column is an integer (a name, kind or field atom), as the planner's SQL holds it (O13).
fn sym_char(inputs: &Inputs, t: &str) -> char {
    if t == "sym" && inputs.atoms.is_none() { 's' } else { 'i' }
}

fn prog_types<'p>(prog: &'p Program, r: &str) -> Vec<&'p str> {
    prog.decls[r].iter().map(|(_, t)| t.as_str()).collect()
}

/// A relation's statistics from the native store (`set_fixpoint_stats`, T's `live_stats`): rows, then per column
/// the distinct count and the sum of squared postings, whose ratio to rows is the skew-weighted fanout ingestion
/// gives inputs.
/// A native relation's live statistics (`setfix::live`): rows, per column distinct values and the sum of squared
/// postings, and each value's postings in a column of few values.
struct LiveRel {
    n: i64,
    ndv: Vec<i64>,
    sq: Vec<i64>,
    freq: Vec<(usize, HashMap<Constant, i64>)>,
}
impl LiveRel {
    /// From T's extension's text line (`set_fixpoint_stats`); `text(c)`: column `c` holds symbols as text.
    fn parse(line: &str, text: &dyn Fn(usize) -> bool) -> LiveRel {
        let mut parts = line.split(" |");
        let nums: Vec<i64> = parts.next().unwrap_or("").split_whitespace().filter_map(|x| x.parse().ok()).collect();
        let (n, rest) = nums.split_first().map_or((0, &[][..]), |(n, r)| (*n, r));
        let k = rest.len() / 2;
        let freq = parts
            .filter_map(|f| {
                let (c, vals) = f.trim().split_once(':')?;
                let c = c.parse::<usize>().ok()?;
                let freq = vals
                    .split(',')
                    .filter(|x| !x.is_empty())
                    .filter_map(|x| {
                        let (v, m) = x.split_once('=')?;
                        let v = unhex(v)?;
                        let v = if text(c) { Constant::Str(v) } else { Constant::Int(v.parse().ok()?) };
                        Some((v, m.parse().ok()?))
                    })
                    .collect();
                Some((c, freq))
            })
            .collect();
        LiveRel { n, ndv: rest[..k].to_vec(), sq: rest[k..].to_vec(), freq }
    }
}

fn live_stats(model: &mut Model, r: &str, live: LiveRel) {
    let n = live.n;
    model.stats.card.insert(r.to_owned(), (n as f64).max(1.0));
    for (c, (&ndv, &sq)) in live.ndv.iter().zip(&live.sq).enumerate() {
        model.ndv.insert((r.to_owned(), c), ndv.max(1) as f64);
        model.stats.fan.insert((r.to_owned(), c), if n != 0 { sq as f64 / n as f64 } else { 1.0 });
        model.stats.dfreq.remove(&(r.to_owned(), c));
    }
    for (c, freq) in live.freq {
        model.stats.dfreq.insert((r.to_owned(), c), freq);
    }
}

/// A plan's step strings, which say whether a replan changed it.
/// A stratum's name in the profile: its relations, cut at 80 characters.
fn stratum_key(scc: &BTreeSet<String>) -> String {
    let key = scc.iter().cloned().collect::<Vec<_>>().join("/");
    if key.len() > 80 { format!("{}…({} relations)", &key[..key.floor_char_boundary(80)], scc.len()) } else { key }
}

/// A delta probe's plan with the stratum's own relations named as read: the last wave's delta at `at`, else the
/// rows so far.
fn delta_sources(mut x: super::profile::RulePlan, order: &[Step], at: usize, scc: &BTreeSet<String>) -> super::profile::RulePlan {
    for (i, st) in order.iter().enumerate() {
        if let Literal::Atom(l) = &st.lit
            && scc.contains(&l.rel)
        {
            x.steps[i].source = if i == at { "delta (last wave)".into() } else { "this stratum".into() };
        }
    }
    x
}

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
    fn fx(&self) -> Fx<'_> {
        Fx { db: self.db, direct: self.model.inputs.atoms.is_some() }
    }

    /// Makes the tables of `rels` not yet made (`pending`).
    fn ensure<'r>(&mut self, rels: impl IntoIterator<Item = &'r String>) -> Result<()> {
        for r in rels {
            if let Some(create) = self.pending.remove(r) {
                self.db.execute(&create, [])?;
            }
        }
        Ok(())
    }

    /// Moves the native set engine's relations among `rels` into their tables (`set_fixpoint_materialize`).
    fn materialize(&mut self, rels: &[String]) -> Result<()> {
        let held: BTreeSet<String> = rels.iter().filter(|r| self.native_rels.contains(*r)).cloned().collect();
        self.ensure(&held)?;
        for r in held {
            self.fx().materialize(&r)?;
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
            // A view that only projects an input (`kind($n, $k) :- node(_, $n, $k, _, _)`) is read from the input,
            // in place: its columns are the input's.
            let projected = self.model.view_src.get(&l.rel).filter(|(_, vc)| vc.iter().all(Option::is_some)).filter(|_| {
                let rules: Vec<&Rule> = prog.rules.iter().filter(|r| r.head.rel == l.rel).collect();
                matches!(rules.as_slice(), [r] if matches!(r.body.as_slice(), [Literal::Atom(a)] if {
                    let vars: Vec<&String> = a.args.iter().filter_map(|t| if let Term::Var(v) = t { Some(v) } else { None }).collect();
                    a.args.iter().all(|t| matches!(t, Term::Var(_) | Term::Wild)) && vars.iter().collect::<HashSet<_>>().len() == vars.len()
                }))
            });
            let sql_col = |rel: &str, c: usize| match projected { Some((_, vc)) => vc[c].clone().unwrap(), None => sql_col(rel, c) };
            let cols = keep.iter().map(|&c| sql_col(&l.rel, c)).collect::<Vec<_>>().join(", ");
            let cols = if cols.is_empty() { "1".to_owned() } else { cols };
            let mut wheres = Vec::new();
            for (c, t) in l.args.iter().enumerate() {
                if is_const(t) {
                    wheres.push(format!("{} = {}", sql_col(&l.rel, c), sql_term(t, &HashMap::new(), self.model.inputs.atoms).ok()?.render()));
                }
            }
            let decl = prog.decls.get(&l.rel).unwrap_or_else(|| panic!("{} is read and not declared", l.rel));
            let types: String = keep.iter().map(|&c| sym_char(&self.model.inputs, &decl[c].1)).collect();
            let types = if types.is_empty() { "i".to_owned() } else { types };
            let mut sel = self.model.relation(prog, projected.map_or(&l.rel, |(base, _)| base), false).render();
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
        self.why_not = None;
        if scc.iter().any(|r| prog.lattice.contains_key(r)) {
            self.why_not = Some("a lattice relation: the native set engine holds sets only".into());
            return Ok(false);
        }
        let rels: Vec<&String> = scc.iter().collect();
        let ri: BTreeMap<String, usize> = rels.iter().enumerate().map(|(i, r)| ((*r).clone(), i)).collect();
        let name = format!("prog_{}", rels[0]);
        let mut gates: Vec<&(String, crate::code_rule_dsl::logic::magic::Gate)> = prog.gates.iter().filter(|(g, _)| scc.contains(g)).collect();
        gates.sort_by(|a, b| (a.1.level, &a.0).cmp(&(b.1.level, &b.0)));
        // A gate's rule only placed it here.
        let rules: Vec<&Rule> = rules.iter().copied().filter(|r| !gates.iter().any(|(g, _)| *g == r.head.rel)).collect();
        let types = |r: &str| prog_types(prog, r).iter().map(|t| sym_char(&self.model.inputs, t)).collect::<String>();
        let mut lines = vec![format!("name {name}")];
        // A stratum's relations were made empty by `run` unless the program states facts of them.
        lines.extend(rels.iter().map(|r| format!("rel {r} {}{}", types(r), if prog.facts.get(*r).is_none_or(|f| f.is_empty()) { " empty" } else { "" })));
        lines.push("nowrite".to_owned());
        let native_rels = self.native_rels.clone();
        let mut est: HashMap<&str, f64> = HashMap::new();
        let mut orders = Vec::new();
        let seeds: Vec<&Rule> = rules.iter().copied().filter(|r| recursive(&r.body, scc).is_empty()).collect();
        // A rule's planning, as a span named by its number and body size (`plan #0009 violation, 15 literals`).
        let planning = |r: &Rule, what: &str| {
            super::profile::span(|| {
                let no = prog.rules.iter().position(|x| std::ptr::eq(x, r)).map_or("?".to_owned(), |k| format!("{k:04}"));
                format!("{what} #{no} {}, {} literals", r.head.rel, r.body.len())
            })
        };
        let profiling = super::profile::on();
        let rule_no = |r: &Rule| prog.rules.iter().position(|x| std::ptr::eq(x, r));
        // Per seed, then per probe: (kind, rule, plan as EXPLAIN shows it, planning CPU, replans).
        let mut explained: Vec<(String, Option<usize>, Option<super::profile::RulePlan>, f64, u64)> = Vec::new();
        // The first step of `order` without a native step (EXPLAIN's reason the engine declined).
        let missing = |this: &Self, h: &Atom, order: &[Step], at: Option<usize>| -> String {
            let k = (1..=order.len()).find(|&k| this.join_spec(h, &order[..k], at.filter(|&a| a < k), &ri, scc, &native_rels).is_none()).unwrap_or(order.len());
            let s = &order[k.saturating_sub(1)];
            format!("{}{} has no native step", s.lit, if s.mode == Some(Mode::Range) { " (a range read inside a document)" } else { "" })
        };
        for r in &seeds {
            let span = planning(r, "plan");
            let t0 = if profiling { super::profile::now() } else { 0.0 };
            let ((order, _, rows), x) = self.model.plan_rule_reused(prog, &r.head, &r.body, None)?;
            let took = if profiling { super::profile::now() - t0 } else { 0.0 };
            drop(span);
            let Some(spec) = self.join_spec(&r.head, &order, None, &ri, scc, &native_rels) else {
                self.why_not = Some(format!("rule #{:04}: {}", rule_no(r).unwrap_or(0), missing(self, &r.head, &order, None)));
                return Ok(false);
            };
            explained.push((format!("seed {}", r.head.rel), rule_no(r), x, took, 0));
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
                let _span = planning(r, "plan");
                let t0 = if profiling { super::profile::now() } else { 0.0 };
                let ((order, _, _), x) = self.model.plan_rule_reused(prog, &r.head, &r.body, Some(ai))?;
                let took = if profiling { super::profile::now() - t0 } else { 0.0 };
                let at = order.iter().position(|s| s.at == ai).expect("the delta atom");
                let Some(spec) = self.join_spec(&r.head, &order, Some(at), &ri, scc, &native_rels) else {
                    self.why_not = Some(format!("rule #{:04}: {}", rule_no(r).unwrap_or(0), missing(self, &r.head, &order, Some(at))));
                    return Ok(false);
                };
                let x = x.map(|x| delta_sources(x, &order, at, scc));
                explained.push((format!("join {} (delta {})", r.head.rel, a.rel), rule_no(r), x, took, 0));
                // The wave whose delta is all of the relation (it was empty before): any order, the atom read in full.
                let ((whole, _, _), _) = self.model.plan_rule_reused(prog, &r.head, &r.body, None)?;
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
        let started = std::time::Instant::now();
        let mut counted: Vec<Vec<(u64, u64)>> = Vec::new();
        self.fx().open(&lines.join("\n"))?;
        let mut chunk: i64 = 2;
        loop {
            let f: String = self.fx().run(&name, chunk as u32)?;
            let f: Vec<&str> = f.split_whitespace().collect();
            let steps = self.fx().steps(&name)?;
            for (o, line) in orders.iter().chain(probes.iter().map(|p| &p.2)).zip(&steps) {
                observe(&mut self.model.stats, o, line);
            }
            for (k, line) in steps.iter().enumerate() {
                if counted.len() <= k {
                    counted.resize(k + 1, Vec::new());
                }
                if counted[k].len() < line.len() {
                    counted[k].resize(line.len(), (0, 0));
                }
                for (i, &(a, b)) in line.iter().enumerate() {
                    counted[k][i].0 += a as u64;
                    counted[k][i].1 += b as u64;
                }
            }
            let text_symbols = |i: usize, c: usize| self.model.inputs.atoms.is_none() && prog_types(prog, rels[i]).get(c) == Some(&"sym");
            let live = self.fx().stats(&name, &text_symbols)?;
            for (r, l) in rels.iter().zip(live) {
                live_stats(&mut self.model, r, l);
            }
            if f.last() == Some(&"1") || chunk == 0 {
                break;
            }
            for k in 0..probes.len() {
                let (r, ai) = (probes[k].0, probes[k].1);
                let span = planning(r, "replan");
                let t0 = if profiling { super::profile::now() } else { 0.0 };
                let ((new, _, _), x) = self.model.plan_rule_explained(prog, &r.head, &r.body, Some(ai))?;
                drop(span);
                let e = &mut explained[seeds.len() + k];
                e.3 += if profiling { super::profile::now() - t0 } else { 0.0 };
                if shown(&new) == shown(&probes[k].2) {
                    continue;
                }
                e.4 += 1;
                let at = new.iter().position(|s| s.at == ai).expect("the delta atom");
                e.2 = x.map(|x| delta_sources(x, &new, at, scc));
                let at = new.iter().position(|s| s.at == ai).expect("the delta atom");
                let spec = self
                    .join_spec(&r.head, &new, Some(at), &ri, scc, &native_rels)
                    .ok_or_else(|| Error::Plan(format!("a replanned probe of {} has no native step", r.head)))?;
                self.fx().reprobe(&name, seeds.len() + k, "join", &hex(&spec))?;
                probes[k].2 = new;
            }
            chunk *= 2;
        }
        if super::profile::on() {
            let empty = Vec::new();
            let kinds = seeds.iter().map(|r| format!("seed {}", r.head)).chain(probes.iter().map(|(r, ai, _)| format!("join {} (delta at {ai})", r.head)));
            for ((k, o), kind) in orders.iter().chain(probes.iter().map(|p| &p.2)).enumerate().zip(kinds) {
                super::profile::native(format!("{kind}: {}", shown(o).join(" , ")), counted.get(k).unwrap_or(&empty));
            }
            let key = stratum_key(scc);
            super::profile::stratum(&key, "native set engine", || {
                format!(
                    "every rule has a native step: component {name}, {} seed joins, {} delta probes, {} gates; inputs read in place, derived relations held in memory",
                    seeds.len(),
                    probes.len(),
                    gates.len()
                )
            });
            for (k, (kind, rule, x, took, replans)) in explained.iter().enumerate() {
                if let Some(x) = x {
                    super::profile::planned(&key, kind, *rule, x, *took, *replans, Some(counted.get(k).unwrap_or(&empty)), None);
                }
            }
        }
        if let Some(file) = std::env::var_os("RDL_COMPONENTS") {
            // The component's plans and each step's observed bindings in and out (`set_fixpoint_steps`), appended.
            let mut text = format!("COMPONENT {} rounds={} ms={:.1}\n", rels.iter().map(|r| r.as_str()).collect::<Vec<_>>().join("/"), chunk.trailing_zeros(), started.elapsed().as_secs_f64() * 1e3);
            for l in &lines {
                let parts: Vec<&str> = l.split(' ').collect();
                if matches!(parts.first(), Some(&"seed" | &"join")) {
                    let spec = parts.get(if parts[0] == "seed" { 2 } else { 3 }).and_then(|h| unhex(h)).unwrap_or_default();
                    let spec: Vec<String> = spec.split('\n').map(|step| step.split(' ').map(|t| t.strip_prefix("t:").map_or(t.to_owned(), |rest| {
                        let f: Vec<&str> = rest.splitn(4, ':').collect();
                        format!("t:{}:[{}]", f.first().unwrap_or(&""), f.get(1).and_then(|h| unhex(h)).unwrap_or_default())
                    })).collect::<Vec<_>>().join(" ")).collect();
                    text += &format!("  SPEC {} {}\n", parts[0], spec.join(" | "));
                }
            }
            let empty = Vec::new();
            for (k, o) in orders.iter().chain(probes.iter().map(|p| &p.2)).enumerate() {
                let line: Vec<String> = counted.get(k).unwrap_or(&empty).iter().map(|(a, b)| format!("{a}:{b}")).collect();
                let work: u64 = counted.get(k).unwrap_or(&empty).iter().map(|(a, _)| a).sum();
                text += &format!("  work={work} {}\n    {}\n", line.join(","), shown(o).join(" , "));
            }
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(file) {
                let _ = f.write_all(text.as_bytes());
            }
        }
        self.fx().close(&name)?;
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
        // Over atoms a transfer's operator is its name atom; `lattice_fixpoint` reads the operator's text.
        if let Some([plus, minus, times]) = self.model.inputs.atoms {
            self.db.execute(
                &format!("UPDATE lfx_{rel}_bin SET c1 = CASE c1 WHEN {plus} THEN '+' WHEN {minus} THEN '-' WHEN {times} THEN '*' ELSE '?' END"),
                [],
            )?;
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
                    .map(|c| sym_char(&self.model.inputs, &prog.decls[&a.rel][c].1)),
                _ => None,
            })
        };
        let tables: HashMap<String, (String, String)> = rels.iter().map(|r| ((*r).clone(), (format!("sfx_f_{r}"), format!("sfx_d_{r}")))).collect();
        let types = |r: &str| prog_types(prog, r).iter().map(|t| sym_char(&self.model.inputs, t)).collect::<String>();
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
        self.fx().open(&lines.join("\n"))?;
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
            let f: String = self.fx().run(&name, chunk as u32)?;
            if f.split_whitespace().last() == Some("1") || chunk == 0 {
                break;
            }
            let text_symbols = |i: usize, c: usize| self.model.inputs.atoms.is_none() && prog_types(prog, rels[i]).get(c) == Some(&"sym");
            let live = self.fx().stats(&name, &text_symbols)?;
            for (r, l) in rels.iter().zip(live) {
                live_stats(&mut self.model, r, l);
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
                self.fx().reprobe(&name, k, kind, &body)?;
                probes[k].1 = new;
                probes[k].2 = nat;
            }
            chunk *= 2;
        }
        for r in &rels {
            let (full, delta) = &tables[*r];
            self.db.execute_batch(&format!("DROP TABLE temp.{full}; DROP TABLE temp.{delta};"))?;
        }
        self.fx().close(&name)?;
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
    let inputs = Inputs {
        facts: "main.".to_owned(),
        stats: format!("{stats}."),
        atoms: Some(lowered.ops),
        native: BTreeSet::new(),
        batch: HashMap::new(),
        given: None,
        kept: BTreeSet::new(),
    };
    let out = run(&lowered.program, db, inputs, Some(lowered));
    super::native::flush_symbols(db)?;
    out
}

/// `run_atoms` with T's native modes (O13): the components a mode takes run in `logic::native` (registered on
/// `db`) over atoms, the rest in SQL. T's `nearest` and `kseed` modes read a per-run copy of the tree
/// (`tape_load`), which is not kept: `nearest` is the transducer's (`Lowered::transduce_nearest`), so those two
/// modes are dropped from `modes`.
pub(in crate::code_rule_dsl) fn run_atoms_native(lowered: &lower::Lowered, db: &Connection, stats: &str, modes: &str) -> Result<Outcome> {
    let mut native = native_modes(modes);
    native.remove("nearest");
    native.remove("kseed");
    let inputs = Inputs { facts: "main.".to_owned(), stats: format!("{stats}."), atoms: Some(lowered.ops), native, batch: HashMap::new(), given: None, kept: BTreeSet::new() };
    run_native_inputs(lowered, db, inputs)
}

/// `run_atoms_native` over input relations in schema `facts` (the bound vtabs are `main`'s; a cross-file pass reads
/// what the chunks wrote, `corpus::run`).
/// With the planner's statistics `given` (measured by the caller, in memory), and the outputs `kept` in the native
/// engine for the caller (`Outcome::kept`): neither written to their tables nor decoded.
pub(in crate::code_rule_dsl) fn run_atoms_native_over(
    lowered: &lower::Lowered,
    db: &Connection,
    facts: &str,
    given: Stats,
    kept: BTreeSet<String>,
    modes: &str,
) -> Result<Outcome> {
    let mut native = native_modes(modes);
    native.remove("nearest");
    native.remove("kseed");
    // The statistics schema is still read for document ranges (`stat_docrange`).
    let inputs = Inputs { facts: format!("{facts}."), stats: "s.".to_owned(), atoms: Some(lowered.ops), native, batch: HashMap::new(), given: Some(given), kept };
    run_native_inputs(lowered, db, inputs)
}

fn run_native_inputs(lowered: &lower::Lowered, db: &Connection, inputs: Inputs) -> Result<Outcome> {
    db.execute_batch("PRAGMA temp_store = MEMORY;")?;
    super::native::register(db)?;
    super::native::register_symbols(db)?;
    super::native::forget_inputs(db);
    super::native::hold_symbols(db, true)?;
    let out = run(&lowered.program, db, inputs, Some(lowered));
    // Flushed while still held: the run's symbols are the table's next rows.
    let flushed = super::native::flush_symbols(db);
    super::native::hold_symbols(db, false)?;
    flushed?;
    out
}

/// Inserts `rows` (each of one width) with `insert` (`INSERT [OR …] INTO t`), many rows per statement: a statement
/// per row would be a host loop over data.
pub(in crate::code_rule_dsl) fn insert_many(db: &Connection, insert: &str, rows: Vec<Vec<SqlValue>>) -> Result<()> {
    let Some(width) = rows.first().map(Vec::len) else { return Ok(()) };
    // SQLite binds at most 32766 parameters per statement.
    let per = (32_000 / width.max(1)).max(1);
    let mut rows = rows.into_iter().peekable();
    while rows.peek().is_some() {
        let part: Vec<Vec<SqlValue>> = rows.by_ref().take(per).collect();
        let row = format!("({})", vec!["?"; width].join(", "));
        let holes = vec![row.as_str(); part.len()].join(", ");
        db.execute(&format!("{insert} VALUES {holes}"), rusqlite::params_from_iter(part.into_iter().flatten()))?;
    }
    Ok(())
}

/// Positions in a symbol, for splitting one at its dots (`$m = @cat($q, $x)` with only $m bound).
const RDL_INT: &str = "CREATE TEMP TABLE rdl_int(i INTEGER PRIMARY KEY); INSERT INTO rdl_int WITH RECURSIVE i(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM i WHERE i < 4096) SELECT i FROM i;";

/// The relations a single rule projects from one input atom, as views (`CREATE TEMP VIEW`), with
/// their statistics, input columns and document columns recorded in `model`.
/// A view's statement.
pub(in crate::code_rule_dsl) fn view_sql(rel: &str, query: &Query) -> String {
    format!("CREATE TEMP VIEW {rel} AS {}", query.render())
}

/// The views `run` makes of one-atom projections of an input, each as its query (`view_sql`).
fn plan_views(prog: &Program, model: &mut Model, rules_by: &IndexMap<&str, Vec<&Rule>>) -> Result<Vec<(String, Query)>> {
    let mut views = Vec::new();
    for (&rel, rs) in rules_by {
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
        let mut query = model.checked_query(prog, h, &order, None, None)?;
        query.body.distinct = false;
        views.push((rel.to_owned(), query));
        model.view_stats(rel, h, a);
        let src = first_columns(a);
        let cols = h.args.iter().map(|t| if let Term::Var(v) = t { src.get(v.as_str()).map(|&c| sql_col(&a.rel, c)) } else { None }).collect();
        model.view_src.insert(rel.to_owned(), (a.rel.clone(), cols));
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
                model.docof.insert(rel.to_owned(), (pos(n), pos(d)));
            }
        }
    }
    Ok(views)
}

/// core.rr's functors whose meaning is W's, which `strings::functions` does not give: `@replace_all(s, re, by)`, each
/// match of `re` in `s` replaced by `by`, `$1`
/// and `${name}` its groups, as W's `replace` transform does (`matcher::transform::replace`); `@rx_replace(s, re, by)`
/// the same as W's SQL `rx_replace` does it (`waves::relational::functions`: the `regex` crate's `replace_all`, `$1`,
/// `$name`, `${name}` and `$$` as it reads them); `@ed25519_ok(key, message, sig)`, 1 where `crypto::ed25519_ok`
/// holds, else 0. NULL gives NULL (`rr_ed25519_ok`: 0). `rr_join(sep, x, k, …)` is `join`'s aggregate: the distinct
/// `(k, …, x)` rows' `x` ordered by `k, …` then `x` (numbers by value, then text by code point, as `sorted()` orders a
/// list of one type), joined by `sep`; `""` over no rows.
fn register_functors(db: &Connection) -> rusqlite::Result<()> {
    use rusqlite::functions::FunctionFlags;
    let flags = FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC;
    fn compiled(f: &str, re: &str) -> rusqlite::Result<regex::Regex> {
        static REGEXES: std::sync::LazyLock<std::sync::Mutex<HashMap<String, Option<regex::Regex>>>> = std::sync::LazyLock::new(Default::default);
        let regex = REGEXES.lock().expect("the regex cache").entry(re.to_owned()).or_insert_with(|| regex::Regex::new(re).ok()).clone();
        regex.ok_or_else(|| rusqlite::Error::UserFunctionError(format!("@{f}: bad regex {re:?}").into()))
    }
    let texts = |ctx: &rusqlite::functions::Context<'_>| -> rusqlite::Result<Option<[String; 3]>> {
        Ok(match (ctx.get::<Option<String>>(0)?, ctx.get::<Option<String>>(1)?, ctx.get::<Option<String>>(2)?) {
            (Some(a), Some(b), Some(c)) => Some([a, b, c]),
            _ => None,
        })
    };
    db.create_scalar_function(c"rr_replace_all", 3, flags, move |ctx| {
        let Some([s, re, by]) = texts(ctx)? else { return Ok(None) };
        Ok(Some(crate::code_rule_dsl::matcher::transform::replace(&s, &compiled("replace_all", &re)?, &by)))
    })?;
    db.create_scalar_function(c"rr_rx_replace", 3, flags, move |ctx| {
        let Some([s, re, by]) = texts(ctx)? else { return Ok(None) };
        Ok(Some(compiled("rx_replace", &re)?.replace_all(&s, by.as_str()).into_owned()))
    })?;
    db.create_scalar_function(c"rr_ed25519_ok", 3, flags, move |ctx| {
        Ok(texts(ctx)?.is_some_and(|[key, message, sig]| super::crypto::ed25519_ok(&key, &message, &sig)))
    })?;
    db.create_aggregate_function(c"rr_join", -1, flags, OrderedJoin)
}

/// `rr_join`'s rows: each one's order key (its `k, …`, then `x`) and `x`'s text.
struct OrderedJoin;
impl rusqlite::functions::Aggregate<(String, Vec<(Vec<rusqlite::types::Value>, String)>), String> for OrderedJoin {
    fn init(&self, ctx: &mut rusqlite::functions::Context<'_>) -> rusqlite::Result<(String, Vec<(Vec<rusqlite::types::Value>, String)>)> {
        Ok((ctx.get::<Option<String>>(0)?.unwrap_or_default(), Vec::new()))
    }
    fn step(&self, ctx: &mut rusqlite::functions::Context<'_>, acc: &mut (String, Vec<(Vec<rusqlite::types::Value>, String)>)) -> rusqlite::Result<()> {
        use rusqlite::types::Value;
        let x: Value = ctx.get(1)?;
        let text = match &x {
            Value::Text(s) => s.clone(),
            Value::Integer(i) => i.to_string(),
            Value::Real(r) => r.to_string(),
            Value::Null => return Ok(()),
            Value::Blob(_) => return Err(rusqlite::Error::UserFunctionError("join: a blob".into())),
        };
        let mut key: Vec<Value> = (2..ctx.len()).map(|i| ctx.get(i)).collect::<rusqlite::Result<_>>()?;
        key.push(x);
        acc.1.push((key, text));
        Ok(())
    }
    fn finalize(&self, _: &mut rusqlite::functions::Context<'_>, acc: Option<(String, Vec<(Vec<rusqlite::types::Value>, String)>)>) -> rusqlite::Result<String> {
        use rusqlite::types::Value;
        use std::cmp::Ordering;
        let Some((sep, mut rows)) = acc else { return Ok(String::new()) };
        // Numbers by value before text by code point (UTF-8 byte order is code point order).
        fn value(a: &Value, b: &Value) -> Ordering {
            let number = |v: &Value| match v {
                Value::Integer(i) => Some(*i as f64),
                Value::Real(r) => Some(*r),
                _ => None,
            };
            match (a, b) {
                (Value::Integer(x), Value::Integer(y)) => x.cmp(y),
                (Value::Text(x), Value::Text(y)) => x.cmp(y),
                _ => match (number(a), number(b)) {
                    (Some(x), Some(y)) => x.total_cmp(&y),
                    (Some(_), None) => Ordering::Less,
                    (None, Some(_)) => Ordering::Greater,
                    (None, None) => Ordering::Equal,
                },
            }
        }
        let key = |a: &[Value], b: &[Value]| a.iter().zip(b).map(|(x, y)| value(x, y)).find(|o| o.is_ne()).unwrap_or(Ordering::Equal);
        rows.sort_by(|a, b| key(&a.0, &b.0));
        rows.dedup_by(|a, b| key(&a.0, &b.0).is_eq());
        Ok(rows.iter().map(|(_, x)| x.as_str()).collect::<Vec<_>>().join(&sep))
    }
}

fn run(prog: &Program, db: &Connection, inputs: Inputs, lowered: Option<&lower::Lowered>) -> Result<Outcome> {
    let db = db;
    let setup = super::profile::span(|| "setup".into());
    super::strings::register(db)?;
    register_functors(db)?;
    // The previous run's tables are dropped; its views are kept where this run plans the same (`plan_views`), and the
    // positions table (`rdl_int`) is the same for every run.
    let temp: Vec<(String, String, String)> = db
        .prepare("SELECT type, name, coalesce(sql, '') FROM temp.sqlite_master WHERE type IN ('table', 'view')")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut old_views: HashMap<String, String> = HashMap::new();
    for (ty, name, sql) in temp {
        if ty == "view" {
            old_views.insert(name, sql);
        } else if name != "rdl_int" {
            db.execute_batch(&format!("DROP TABLE IF EXISTS temp.{name}"))?;
        }
    }
    // Positions in a symbol, for splitting one at its dots (`$m = @cat($q, $x)` with only $m bound).
    if !db.prepare("SELECT 1 FROM temp.sqlite_master WHERE name = 'rdl_int'")?.exists([])? {
        db.execute_batch(RDL_INT)?;
    }
    let atoms = inputs.atoms.is_some();
    let facts = inputs.facts.clone();
    if inputs.native.contains("nearest") {
        db.query_row("SELECT tape_load(?1)", [facts.trim_end_matches('.')], |_| Ok(()))?;
    }
    drop(setup);
    let planning = super::profile::span(|| "statistics and plan".into());
    let model = make_plan(prog, db, inputs)?;
    let (sccs, _) = strata(prog)?;
    let mut rules_by: IndexMap<&str, Vec<&Rule>> = IndexMap::new();
    for r in &prog.rules {
        rules_by.entry(r.head.rel.as_str()).or_default().push(r);
    }
    drop(planning);
    let tables = super::profile::span(|| "views and tables".into());
    let native = model.inputs.native.clone();
    let mut run = Run { prog, db, model, views: HashSet::new(), made: HashSet::new(), native_rels: BTreeSet::new(), pending: HashMap::new(), why_not: None };
    for (rel, view) in plan_views(prog, &mut run.model, &rules_by)? {
        let view = view_sql(&rel, &view);
        // SQLite keeps a temp view's statement without `TEMP`.
        if old_views.remove(&rel).as_deref() != Some(&view.replacen("CREATE TEMP VIEW", "CREATE VIEW", 1)) {
            db.execute_batch(&format!("DROP VIEW IF EXISTS temp.{rel}"))?;
            db.execute(&view, [])?;
        }
        run.views.insert(rel);
    }
    for name in old_views.keys() {
        db.execute_batch(&format!("DROP VIEW IF EXISTS temp.{name}"))?;
    }
    // Relations the program's rules or outputs name (a part of a split program declares the whole's).
    let named: HashSet<&str> = prog
        .rules
        .iter()
        .flat_map(|r| std::iter::once(r.head.rel.as_str()).chain(r.body.iter().filter_map(|l| match l {
            Literal::Atom(a) | Literal::Agg { atom: a, .. } => Some(a.rel.as_str()),
            _ => None,
        })))
        .chain(prog.outputs.iter().map(String::as_str))
        .chain(prog.facts.keys().map(String::as_str))
        .collect();
    for (rel, cols) in &prog.decls {
        if prog.inputs.contains(rel) || run.views.contains(rel) || !named.contains(rel.as_str()) {
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
        let create = format!("CREATE TEMP TABLE {rel}({names}, PRIMARY KEY ({key})) WITHOUT ROWID");
        // A rule's head is made when SQL first needs it; a relation of facts alone, or of neither, now.
        // (Over atoms: T's extension, over text, reads its relations' tables itself.)
        if atoms && rules_by.contains_key(rel.as_str()) && prog.facts.get(rel).is_none_or(|f| f.is_empty()) {
            run.pending.insert(rel.clone(), create);
            continue;
        }
        db.execute(&create, [])?;
        let rows: Vec<Vec<SqlValue>> = prog
            .facts
            .get(rel)
            .into_iter()
            .flatten()
            .map(|row| {
                row.iter()
                    .map(|v| match v {
                        Value::Int(i) => SqlValue::Integer(*i),
                        Value::Str(s) => SqlValue::Text(s.clone()),
                    })
                    .collect()
            })
            .collect();
        insert_many(db, &format!("INSERT OR IGNORE INTO {rel}"), rows)?;
    }
    drop(tables);
    let mut outcome = Outcome::default();
    for scc in &sccs {
        let rules: Vec<&Rule> = scc.iter().flat_map(|rel| rules_by.get(rel.as_str()).into_iter().flatten().copied()).collect();
        let key = scc.iter().cloned().collect::<Vec<_>>().join("/");
        let _stratum = super::profile::span(|| format!("stratum {}", if key.len() > 80 { format!("{}…({} relations)", &key[..80], scc.len()) } else { key.clone() }));
        let rule_no = |r: &Rule| prog.rules.iter().position(|x| std::ptr::eq(x, r)).expect("a rule of the program");
        let skey = stratum_key(scc);
        let mut why = if native.contains("program") { String::new() } else { format!("the native engine is off (modes: {native:?})") };
        if super::profile::on() {
            for r in &rules {
                super::profile::note(format!("rule #{:04}", rule_no(r)), || r.to_string());
            }
        }
        if native.contains("program") && !scc.iter().all(|r| run.views.contains(r)) {
            let native_span = super::profile::span(|| "native".into());
            let ran = run.native_stratum(scc, &rules)?;
            drop(native_span);
            if ran {
                outcome.native.insert(key);
                continue;
            }
            why = format!("the native engine declined: {}", run.why_not.take().unwrap_or_else(|| "no reason given".into()));
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
        // This stratum runs in SQL: its relations' and those it reads' tables.
        let read: Vec<String> = rules
            .iter()
            .flat_map(|r| &r.body)
            .filter_map(|l| match l {
                Literal::Atom(a) | Literal::Agg { atom: a, .. } => Some(a.rel.clone()),
                _ => None,
            })
            .chain(scc.iter().cloned())
            .collect();
        run.ensure(&read)?;
        let mut planned = Vec::new();
        let mut explained = Vec::new();
        let planning = super::profile::span(|| "plan".into());
        for rule in &rules {
            let t0 = if super::profile::on() { super::profile::now() } else { 0.0 };
            let ((order, work, _), x) = run.model.plan_rule_reused(prog, &rule.head, &rule.body, None)?;
            explained.push((x, if super::profile::on() { super::profile::now() - t0 } else { 0.0 }));
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
        drop(planning);
        if scc.iter().all(|r| run.views.contains(r)) {
            super::profile::stratum(&skey, "SQL view", || "a view over the inputs: SQLite reads it in place where a rule joins it".into());
            continue;
        }
        let recursive =
            scc.len() > 1 || planned.iter().flat_map(|(_, o)| o).any(|s| matches!(&s.lit, Literal::Atom(a) if !a.neg && scc.contains(&a.rel)));
        if !recursive {
            super::profile::stratum(&skey, "SQLite (INSERT ... SELECT per rule)", || why.clone());
            for (((h, order), rule), (x, took)) in planned.iter().zip(&rules).zip(&explained) {
                let _rule = super::profile::span(|| format!("sql #{:04} {}", rule_no(rule), h.rel));
                let sql = run.model.sql_select(prog, h, order, None, None)?;
                let n = run.insert_rows(h, &sql)?;
                if let Some(x) = x {
                    super::profile::planned(&skey, &format!("sql insert {} (span sql #{:04})", h.rel, rule_no(rule)), Some(rule_no(rule)), x, *took, 0, None, Some(n as u64));
                }
            }
        } else {
            for r in scc {
                db.execute(&format!("CREATE TEMP TABLE delta_{r} AS SELECT * FROM {r} WHERE 0"), [])?;
            }
            if native.contains("fixpoint") && run.native_fixpoint(scc, &rules)? {
                super::profile::stratum(&skey, "lattice fixpoint (lattice_fixpoint: CPU waves, or the Metal work queue)", || why.clone());
                outcome.native.insert(key);
                for r in scc {
                    run.refresh(r, false)?;
                }
                continue;
            }
            if native.contains("setfix") && run.native_set_fixpoint(scc, &rules)? {
                super::profile::stratum(&skey, "native set fixpoint (setfix)", || why.clone());
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
                super::profile::stratum(&skey, "SQLite semi-naive fixpoint", || format!("{why}; {n} rounds"));
                for (((h, _), rule), (x, took)) in planned.iter().zip(&rules).zip(&explained) {
                    if let Some(x) = x {
                        super::profile::planned(&skey, &format!("sql round {}", h.rel), Some(rule_no(rule)), x, *took, 0, None, None);
                    }
                }
                outcome.iterations.insert(key, n);
            }
        }
        let _refresh = super::profile::span(|| "refresh and distinct counts".into());
        for r in scc {
            run.refresh(r, false)?;
            for c in 0..prog.decls[r].len() {
                let d = run.count(&format!("SELECT count(DISTINCT c{c}) FROM {r}"))?;
                run.model.ndv.insert((r.clone(), c), if d == 0 { 1.0 } else { d as f64 });
            }
        }
    }
    let _outputs = super::profile::span(|| "outputs".into());
    let kept = run.model.inputs.kept.clone();
    outcome.kept = kept.iter().filter(|r| run.native_rels.contains(*r)).cloned().collect();
    let written: Vec<String> = prog.outputs.iter().filter(|r| !outcome.kept.contains(*r)).cloned().collect();
    run.materialize(&written)?;
    let (outputs, nodes) = decoded_outputs(prog, db, lowered, &facts, atoms, &kept)?;
    outcome.outputs = outputs;
    outcome.nodes = nodes;
    Ok(outcome)
}

/// The program's outputs as text rows, sorted (symbols as text, kinds and fields by name), and per
/// output the columns that hold nodes.
#[allow(clippy::type_complexity)]
fn decoded_outputs(
    prog: &Program,
    db: &Connection,
    lowered: Option<&lower::Lowered>,
    facts: &str,
    atoms: bool,
    kept: &BTreeSet<String>,
) -> Result<(BTreeMap<String, Vec<String>>, BTreeMap<String, Vec<usize>>)> {
    let (mut outputs, mut all_nodes) = (BTreeMap::new(), BTreeMap::new());
    let text = |e: &str| if atoms { format!("sym_text({e})") } else { e.to_owned() };
    for rel in prog.outputs.iter().filter(|r| !kept.contains(*r)) {
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
        all_nodes.insert(name.clone(), nodes);
        outputs.insert(name, rows);
    }
    Ok((outputs, all_nodes))
}

pub(in crate::code_rule_dsl) mod emit;

#[cfg(test)]
mod tests;
