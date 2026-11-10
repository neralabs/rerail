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
//! native store reports), the `tape_nearest` climb (`nearest`). They call the SQL
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
pub(in crate::code_rule_dsl) mod placement;
mod demand;
pub(in crate::code_rule_dsl) mod sketch;
pub(in crate::code_rule_dsl) mod statkeep;
mod keeps;
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
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = Vec::with_capacity(2 * s.len());
    for b in s.bytes() {
        out.extend_from_slice(&[DIGITS[usize::from(b >> 4)], DIGITS[usize::from(b & 15)]]);
    }
    String::from_utf8(out).expect("hex digits")
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

/// The rule an observed fanout belongs to (`Stats::obs`): its head and body, digested. A fanout through a bound
/// column depends on which values the rule's bindings bring: `docof($x, $d)` with `$d` bound reads a whole document,
/// and one rule's bindings fall in 11-node documents while another's fall in documents of thousands of nodes. Pooled
/// over rules, the first set the second's estimate (`up_once`: 1,285 rows a binding, priced at 10).
fn obs_rule(head: &Atom, body: &[Literal]) -> u64 {
    // The rule's digest (`digest_atom`, `digest_literal`), not its text written out: it is asked at every search.
    let d = super::derived::digest(|d| {
        digest_atom(d, head);
        d.u64(body.len() as u64);
        body.iter().for_each(|l| digest_literal(d, l));
    });
    (d as u64) ^ ((d >> 64) as u64)
}

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
    /// (rule, binding pattern) → [bindings in, bindings out]: fanouts native joins observed, per rule (`obs_rule`).
    pub(in crate::code_rule_dsl) obs: HashMap<(u64, Sig), [i64; 2]>,
    /// The recursive component being planned (`atom_rows`).
    pub(in crate::code_rule_dsl) solving: BTreeSet<String>,
    /// `solving`'s digest, for plan keys (`plan_key`): set with it (`solve`), not made again at every lookup.
    solving_digest: u128,
    /// Input rel → index column lists.
    indexes: HashMap<String, Vec<Vec<String>>>,
    /// Documents (node-id intervals, `stat_docrange`), if the facts have them.
    docs: Option<i64>,
    /// Exact postings of the program's constants: table → (column, value) → n. By table first: a relation's are
    /// read without a pass over every table's (`of_relation`, once per input of a session's run).
    memo: HashMap<String, HashMap<(String, Constant), i64>>,
    steps_cache: RefCell<HashMap<(Option<String>, String), f64>>,
    kindsets: BTreeMap<String, Vec<String>>,
}

impl Stats {
    /// The recursive component being planned (`solving`), and its digest.
    pub(in crate::code_rule_dsl) fn solve(&mut self, scc: BTreeSet<String>) {
        self.solving_digest = super::derived::digest(|d| {
            d.u64(scc.len() as u64);
            scc.iter().for_each(|r| d.str(r));
        });
        self.solving = scc;
    }

    /// What the planner knows of `rel` (`RERAIL_DUMP_STATS`, a debugging aid): each column's (rows, distinct, sum of
    /// squared postings, within documents), its cardinality, and its few-valued columns' value counts.
    pub(in crate::code_rule_dsl) fn describe(&self, rel: &str) -> String {
        let mut cols: Vec<String> = self.col.iter().filter(|((t, _), _)| t.contains(rel)).map(|((t, c), v)| format!("{t}.{c}={v:?}")).collect();
        cols.sort();
        let mut few: Vec<String> = self.dfreq.iter().filter(|((t, _), _)| t.contains(rel)).map(|((t, c), m)| format!("{t}.c{c}: {} values, {} rows", m.len(), m.values().sum::<i64>())).collect();
        few.sort();
        let mut card: Vec<String> = self.card.iter().filter(|(t, _)| t.contains(rel)).map(|(t, n)| format!("{t}={n}")).collect();
        card.sort();
        let kinds = if rel == "kind" { format!("; {} kinds, {} nodes", self.kfreq.len(), self.kfreq.values().sum::<i64>()) } else { String::new() };
        let consts: Vec<i64> = self.memo.get(rel).into_iter().flat_map(|m| m.values().copied()).collect();
        format!("{rel}: card {}; {}; {}{kinds}; {} constants counted, {} rows", card.join(" "), cols.join(" "), few.join(" "), consts.len(), consts.iter().sum::<i64>())
    }
    /// The caller's statistics, taken (a corpus's measured values run to hundreds of MB), else T's.
    fn new(db: &Connection, prog: &Program, inputs: &mut Inputs) -> Result<Self> {
        if let Some(given) = inputs.given.take() {
            return Ok(given);
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
            solving_digest: 0,
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

    /// `rel`'s statistics by column (`width` of them), and its exact postings of integer values (`measured`'s
    /// `vals`, by column), when every column is known: what a session's next run gives `rel__old`, whose rows are
    /// these (decision 0094 §4: statistics kept like data, not measured again).
    pub(in crate::code_rule_dsl) fn of_relation(&self, rel: &str, width: usize) -> Option<(Vec<[i64; 4]>, Vec<(usize, i64, i64)>)> {
        let names: Vec<String> = (0..width).map(|c| sql_col(rel, c)).collect();
        let cols = names.iter().map(|n| self.col.get(&(rel.to_owned(), n.clone())).copied()).collect::<Option<Vec<_>>>()?;
        let vals = self
            .memo
            .get(rel)
            .into_iter()
            .flatten()
            .filter_map(|((c, v), n)| match v {
                Constant::Int(v) => Some((names.iter().position(|x| x == c)?, *v, *n)),
                _ => None,
            })
            .collect();
        Some((cols, vals))
    }

    /// A relation's statistics measured in memory: its rows and per column (rows, ndv, sq, sqd), replacing what was
    /// read; with `vals`, a value column's exact postings per value (`stat_val`'s `n`).
    pub(in crate::code_rule_dsl) fn measured(&mut self, rel: &str, cols: impl IntoIterator<Item = (String, [i64; 4])>, vals: impl IntoIterator<Item = (String, i64, i64)>) {
        for (col, m) in cols {
            self.col.insert((rel.to_owned(), col), m);
        }
        for (col, v, n) in vals {
            self.memo.entry(rel.to_owned()).or_default().insert((col, Constant::Int(v)), n);
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
            sql += &format!(" UNION ALL SELECT 'rows', '{t}', NULL, count(*), NULL, NULL, NULL FROM {f}{}", crate::code_rule_dsl::logic::sql::ident(t));
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
            solving_digest: 0,
            indexes: HashMap::new(),
            docs: meta.get("docs").copied(),
            memo: of("val").filter_map(|r| Some((text(&r[0]), (text(&r[1]), sql_constant(&r[3])?), int(&r[2])))).fold(HashMap::new(), |mut m, (t, k, n)| {
                m.entry(t).or_insert_with(HashMap::new).insert(k, n);
                m
            }),
            steps_cache: RefCell::new(HashMap::new()),
            kindsets: prog.kindsets.clone(),
        })
    }

    /// Exact postings for a constant probe on an input column; `None` if the column has no statistics.
    fn const_freq(&self, rel: &str, cname: &str, v: &Constant) -> Option<i64> {
        if let Some(&n) = self.memo.get(rel).and_then(|m| m.get(&(cname.to_owned(), v.clone()))) {
            return Some(n);
        }
        self.col.contains_key(&(rel.to_owned(), cname.to_owned())).then_some(0)
    }

    /// Posting-weighted fanout of an equality probe on `cname` confined to one document: sqd / rows.
    fn fan_doc(&self, rel: &str, cname: &str) -> f64 {
        let [rows, _, _, sqd] = self.col[&(rel.to_owned(), cname.to_owned())];
        let skewed = skew(rel, cname.bytes().fold(0usize, |h, b| h.wrapping_mul(31).wrapping_add(b as usize)), 2);
        if rows != 0 { (sqd as f64 / rows as f64 * skewed).max(1.0) } else { 1.0 }
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
    /// The program's logical key and scalar analysis, made once per plan and shared (not made again per stratum),
    /// when first asked (`analysis`): only the SQL executor reads it, so the native set engine's runs never make it.
    analysis: std::sync::OnceLock<std::sync::Arc<super::analysis::ProgramAnalysis>>,
    pub(in crate::code_rule_dsl) stats: Stats,
    pub(in crate::code_rule_dsl) ndv: HashMap<(String, usize), f64>,
    /// rel → (node column, document column) for projections of `node(d, n)`: documents are id intervals.
    docof: HashMap<String, (usize, usize)>,
    /// View over one input atom → (input, the input column behind each view column).
    view_src: HashMap<String, (String, Vec<Option<String>>)>,
    inputs: Inputs,
    /// The relations the program's rules derive or its facts state. Their columns are `c0`, `c1`, … even where an
    /// input of the same name has named columns: `rr::lower`'s projections (`kind`, `docof` over nodes) are rules.
    derived: HashSet<String>,
}

impl Model {
    /// Column `col` of `rel` in SQL: an input's name, or `c{col}` for a relation the program derives.
    fn col(&self, rel: &str, col: usize) -> String {
        if self.derived.contains(rel) {
            format!("c{col}")
        } else if let Some((source, _)) = self.inputs.batch.get(rel) {
            if source.positional { format!("c{col}") } else { source.columns.get(col).cloned().unwrap_or_else(|| sql_col(rel, col)) }
        } else {
            sql_col(rel, col)
        }
    }
}

/// An input's column names (`sql::select::input_columns`); any other relation's columns are `c0`, `c1`, ….
pub(in crate::code_rule_dsl) fn sql_col(rel: &str, col: usize) -> String {
    super::sql::select::input_columns(rel).and_then(|names| names.get(col)).map_or_else(|| format!("c{col}"), |&n| n.to_owned())
}

/// A survivors rule of a maintained stratum, `h(v..) :- h__old(v..), !h__del(v..)` (decision 0077's rewrite), over
/// native relations: the kept relation and the deleted one.
fn survivors(r: &Rule, scc: &BTreeSet<String>, native_rels: &BTreeSet<String>) -> Option<(String, String)> {
    let h = &r.head.rel;
    let (old, gone) = (format!("{h}__old"), format!("{h}__del"));
    let vars: Vec<&str> = r.head.args.iter().map(|t| match t { Term::Var(v) => Some(v.as_str()), _ => None }).collect::<Option<_>>()?;
    let distinct = vars.iter().collect::<BTreeSet<_>>().len() == vars.len();
    let [Literal::Atom(a), Literal::Atom(b)] = r.body.as_slice() else { return None };
    (distinct && scc.contains(h) && !a.neg && b.neg && a.rel == old && b.rel == gone && a.args == r.head.args && b.args == r.head.args
        && native_rels.contains(&old) && native_rels.contains(&gone))
        .then_some((old, gone))
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
        Literal::Agg { var, over, atom, op } => {
            let mut need = atom.vars();
            need.remove(var);
            if let Some(o) = over {
                need.remove(o);
            }
            if let AggOp::Join { by, .. } = op {
                by.iter().for_each(|b| {
                    need.remove(b);
                });
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
    /// When profiling, the plan as EXPLAIN showed it when searched: a re-pricing reports it with its own work and rows.
    static PLANS: std::cell::RefCell<Option<rustc_hash::FxHashMap<PlanKey, Kept>>> = const { std::cell::RefCell::new(None) };
    /// The last priced body's cheapest start: the least work of any one join literal read first, from no binding.
    /// Every order costs at least this, so a kept order's work over it measures how far the order is from the best
    /// any start allows.
    static SEED: std::cell::Cell<f64> = const { std::cell::Cell::new(1.0) };
}

/// A kept plan's key: a digest of the rule, the delta and the component solved (`derived::digest`). Not the rule's
/// address: each chunk's program is a new value (the demand split makes one per chunk), so no address is kept between
/// chunks. Not the rule's text either: formatting every rule was a tenth of a session run's planning.
type PlanKey = u128;

/// A kept plan: its order, its work per node when searched, when profiling its EXPLAIN, its work over its body's
/// cheapest start (`SEED`) when searched, and its last price with the sizes it was priced at (`Priced`).
type Kept = (std::rc::Rc<[Step]>, f64, Option<super::profile::RulePlan>, f64, Option<std::rc::Rc<Priced>>);

/// A kept plan's last price: the corpus's nodes and the rows of each relation its body reads when priced, and the
/// price. A price reads statistics only; while no size moved over `PRICE_DRIFT` times it is taken as it is, not
/// worked out again (decision 0090 step 5: a session's run re-priced 7,400 kept plans, two thirds of its CPU).
struct Priced {
    sizes: Vec<f64>,
    got: (Plan, Option<super::profile::RulePlan>),
}

/// How far a size may move before a kept price is worked out again.
const PRICE_DRIFT: f64 = 2.0;

/// Whether sizes `now` are within `PRICE_DRIFT` of `then`, each counted as at least one row.
fn near(then: &[f64], now: &[f64]) -> bool {
    then.len() == now.len()
        && then.iter().zip(now).all(|(a, b)| {
            let (a, b) = (a.max(1.0), b.max(1.0));
            a <= b * PRICE_DRIFT && b <= a * PRICE_DRIFT
        })
}

/// Plans kept across chunks on this thread while held (a corpus run), none kept at the start; for a session's run
/// (`across`, the session's generation), the plans its earlier runs kept, put back at the end (decision 0090 step 5).
pub(in crate::code_rule_dsl) struct KeepPlans(Option<u64>);

thread_local! {
    /// A session's plans between its runs, with the session's generation: another session's are not taken.
    static SESSION_PLANS: std::cell::RefCell<Option<(u64, rustc_hash::FxHashMap<PlanKey, Kept>)>> = const { std::cell::RefCell::new(None) };
}

impl KeepPlans {
    pub(in crate::code_rule_dsl) fn start(across: Option<u64>) -> Self {
        let kept = across.and_then(|g| SESSION_PLANS.with(|s| s.borrow_mut().take().filter(|(k, _)| *k == g).map(|(_, p)| p)));
        PLANS.with(|p| *p.borrow_mut() = Some(kept.unwrap_or_default()));
        KeepPlans(across)
    }
}

impl Drop for KeepPlans {
    fn drop(&mut self) {
        let plans = PLANS.with(|p| p.borrow_mut().take());
        if let (Some(g), Some(plans)) = (self.0, plans) {
            SESSION_PLANS.with(|s| *s.borrow_mut() = Some((g, plans)));
        }
    }
}

thread_local! {
    /// Whether the planner reads its statistics skewed (`Model::plan_inner`, `RERAIL_STATS_SKEW`).
    static SKEWING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// `RERAIL_STATS_SKEW=f[:seed]`: the largest factor a statistic is skewed by, and the seed of the factors.
fn skew_setting() -> Option<(f64, u64)> {
    static SETTING: std::sync::OnceLock<Option<(f64, u64)>> = std::sync::OnceLock::new();
    *SETTING.get_or_init(|| {
        let v = std::env::var("RERAIL_STATS_SKEW").ok()?;
        let (f, seed) = v.split_once(':').map_or((v.as_str(), "0"), |(a, b)| (a, b));
        Some((f.parse().ok()?, seed.parse().ok()?))
    })
}

/// The factor statistic `what` of column `c` of `rel` is skewed by while skewing: fixed per (seed, rel, c, what),
/// log-uniform in [1/f, f]; 1 otherwise.
fn skew(rel: &str, c: usize, what: u64) -> f64 {
    let Some((f, seed)) = skew_setting().filter(|_| SKEWING.with(std::cell::Cell::get)) else { return 1.0 };
    let mut h = seed ^ (what << 56) ^ (c as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    for b in rel.bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
    }
    h ^= h >> 31;
    h = h.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h ^= h >> 29;
    let u = (h >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0;
    (u * f.ln()).exp()
}

/// A rule body in plan order, its estimated work and its estimated output rows.
pub(in crate::code_rule_dsl) type Plan = (Vec<Step>, f64, f64);

impl Model {
    /// `prog`'s logical key and scalar analysis (`analysis`), made the first time it is asked for; `prog` is the
    /// program this model plans.
    pub(in crate::code_rule_dsl) fn analysis(&self, prog: &Program) -> std::sync::Arc<super::analysis::ProgramAnalysis> {
        std::sync::Arc::clone(self.analysis.get_or_init(|| {
            let _analysis = super::profile::span(|| "logical key and scalar analysis".into());
            let analysis = program_analysis(prog);
            let _explain = super::profile::span(|| "explain analysis".into());
            analysis.explain();
            analysis
        }))
    }

    fn card(&self, rel: &str) -> f64 {
        // `node` is the corpus's nodes, except where a batch binds it: then its estimate is the batch's.
        if rel == "node" && !self.inputs.batch.contains_key(rel) {
            self.stats.nodes
        } else if rel.starts_with(super::strings::RELATION) {
            // A text test empty in the chunk that plans is not empty in the chunks that reuse its plans (decision
            // 0038): at least one string.
            self.stats.card.get(rel).copied().unwrap_or(1000.0).max(1.0)
        } else {
            self.stats.card.get(rel).copied().unwrap_or(1000.0)
        }
    }

    fn ndv_or_1(&self, rel: &str, c: usize) -> f64 {
        (self.ndv.get(&(rel.to_owned(), c)).copied().unwrap_or(1.0) * skew(rel, c, 0)).max(1.0)
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
        self.plan_inner(prog, head, body, first, None, true)
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
        let key = self.plan_key("", head, body, first);
        let nodes = self.stats.nodes.max(1.0);
        let Some(kept) = PLANS.with(|p| p.borrow().as_ref().map(|p| p.get(&key).cloned())) else {
            return self.plan_inner(prog, head, body, first, None, true);
        };
        let sizes = self.read_sizes(body);
        let remember = |key: &PlanKey, got: &(Plan, Option<super::profile::RulePlan>)| {
            PLANS.with(|p| {
                if let Some(k) = p.borrow_mut().as_mut().and_then(|p| p.get_mut(key)) {
                    k.4 = Some(std::rc::Rc::new(Priced { sizes: sizes.clone(), got: got.clone() }));
                }
            })
        };
        if let Some((_, _, _, _, Some(priced))) = &kept
            && near(&priced.sizes, &sizes)
        {
            super::profile::decision("plan cache (decision 0038)", || format!("a kept plan reused at its kept price: no size it reads moved over {PRICE_DRIFT}x"));
            return Ok(priced.got.clone());
        }
        if let Some((order, per_node, searched, gap, _)) = kept {
            let priced = {
                let _span = super::profile::span(|| "re-price".into());
                // The search's EXPLAIN (its steps' rows as the search priced them), with this pricing's work, rows and
                // steps priced: not built again per chunk.
                self.plan_inner(prog, head, body, first, Some(&order), false).map(|(got, _)| {
                    let x = searched.map(|x| super::profile::RulePlan { work: got.1, rows: got.2, subsets: 0, candidates: order.len() as u64, ..x });
                    (got, x)
                })
            };
            // A start much cheaper than when the order was searched (a text test that matched thousands of strings
            // there and dozens here): the kept order may now scan what that start would look up, and pricing the
            // kept order alone never shows it.
            let cheaper_start = |work: f64| work / SEED.with(|c| c.get()).max(1.0) > REPLAN * gap.max(1.0) && work > REPLAN_ROWS;
            match priced {
                Ok(got) if cheaper_start(got.0.1) => super::profile::decision("plan cache (decision 0038)", || "replanned: a start much cheaper than when the kept order was searched".into()),
                Ok(got) if got.0.1 / nodes <= REPLAN * per_node.max(1e-12) || got.0.1 - per_node * nodes <= REPLAN_ROWS => {
                    super::profile::decision("plan cache (decision 0038)", || "a kept plan reused, re-priced with this chunk's statistics".into());
                    remember(&key, &got);
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
            self.plan_inner(prog, head, body, first, None, true)?
        };
        PLANS.with(|p| {
            if let Some(p) = p.borrow_mut().as_mut() {
                let gap = got.0.1 / SEED.with(|c| c.get()).max(1.0);
                p.insert(key.clone(), (got.0.0.clone().into(), got.0.1 / nodes, got.1.clone(), gap, None));
            }
        });
        remember(&key, &got);
        Ok(got)
    }

    /// A kept plan's key: the rule as text, the delta (`first`) and the component solved, after `kind`.
    fn plan_key(&self, kind: &str, head: &Atom, body: &[Literal], first: Option<usize>) -> PlanKey {
        let _span = super::profile::span(|| "key".into());
        super::derived::digest(|d| {
            d.str(kind);
            digest_atom(d, head);
            d.u64(body.len() as u64);
            body.iter().for_each(|l| digest_literal(d, l));
            d.u64(first.map_or(u64::MAX, |f| f as u64));
            d.u64(self.stats.solving_digest as u64);
            d.u64((self.stats.solving_digest >> 64) as u64);
        })
    }

    /// The sizes a price reads (`Priced`): the corpus's nodes and the rows of each relation the body reads.
    fn read_sizes(&self, body: &[Literal]) -> Vec<f64> {
        std::iter::once(self.stats.nodes.max(1.0))
            .chain(body.iter().filter_map(|l| match l {
                Literal::Atom(a) | Literal::Agg { atom: a, .. } => Some(self.card(&a.rel)),
                _ => None,
            }))
            .collect()
    }

    /// The sizes a live replan reads (`plan_rule_live`): the corpus's nodes and each atom's rows. The delta atom is
    /// read first and priced by its live size; every later step's rows are per binding of it, so its size scales
    /// every order alike and does not change which is cheapest. It is left out (one row) unless the body reads its
    /// relation again: a wave that only grew the delta keeps the plan. Callers read the order only, so the kept
    /// price is not scaled.
    pub(in crate::code_rule_dsl) fn live_sizes(&self, body: &[Literal], first: Option<usize>) -> Vec<f64> {
        let delta_rel = first.and_then(|f| body[f].atom()).map(|a| a.rel.as_str());
        let once = delta_rel.is_some_and(|d| body.iter().filter(|l| l.atom().is_some_and(|a| a.rel == d)).count() == 1);
        std::iter::once(self.stats.nodes.max(1.0))
            .chain(body.iter().enumerate().filter_map(|(i, l)| match l {
                Literal::Atom(a) | Literal::Agg { atom: a, .. } => Some(if once && Some(i) == first { 1.0 } else { self.card(&a.rel) }),
                _ => None,
            }))
            .collect()
    }

    /// A stratum's plan searched again from its live counts between waves (`plan_rule_explained`), kept with the
    /// sizes it saw: while no size moved over `PRICE_DRIFT` times, the same search is not made again (decision
    /// 0090 step 5: in a session's run these searches were most of its planning). Outside a corpus run every one
    /// is searched.
    pub(in crate::code_rule_dsl) fn plan_rule_live(
        &self,
        prog: &Program,
        head: &Atom,
        body: &[Literal],
        first: Option<usize>,
    ) -> Result<(Plan, Option<super::profile::RulePlan>)> {
        if PLANS.with(|p| p.borrow().is_none()) {
            return self.plan_inner(prog, head, body, first, None, true);
        }
        let key = self.plan_key("live ", head, body, first);
        let sizes = self.live_sizes(body, first);
        // The last price, and the work of the order when it was searched.
        let kept = PLANS.with(|p| p.borrow().as_ref().and_then(|p| p.get(&key)).and_then(|k| Some((k.4.clone()?, k.1))));
        if let Some((priced, searched)) = &kept {
            if near(&priced.sizes, &sizes) {
                super::profile::decision("plan cache (decision 0038)", || format!("a live replan kept: no size it reads moved over {PRICE_DRIFT}x"));
                return Ok(priced.got.clone());
            }
            // A size moved: the kept order is priced again (no search), and kept while its work grew less than
            // `REPLAN` times or by fewer than `REPLAN_ROWS` rows over its work when searched, as a kept plan is
            // across chunks. Most moves are small relations growing between waves (1 row, then 50), where any
            // order costs less than a search.
            let then = *searched;
            let repriced = {
                let _span = super::profile::span(|| "re-price".into());
                self.plan_inner(prog, head, body, first, Some(&priced.got.0.0), false)
            };
            if let Ok((plan, _)) = repriced
                && (plan.1 <= REPLAN * then.max(1.0) || plan.1 - then <= REPLAN_ROWS)
            {
                super::profile::decision("plan cache (decision 0038)", || "a live replan kept, re-priced: its work grew little".into());
                let got = (plan, priced.got.1.clone());
                PLANS.with(|p| {
                    if let Some(k) = p.borrow_mut().as_mut().and_then(|p| p.get_mut(&key)) {
                        k.4 = Some(std::rc::Rc::new(Priced { sizes, got: got.clone() }));
                    }
                });
                return Ok(got);
            }
        }
        let got = {
            let _span = super::profile::span(|| "search".into());
            self.plan_inner(prog, head, body, first, None, true)?
        };
        PLANS.with(|p| {
            if let Some(p) = p.borrow_mut().as_mut() {
                // Its work as searched (for live replans, the second field), against which a re-pricing is judged.
                p.insert(key, (got.0.0.clone().into(), got.0.1, None, 0.0, Some(std::rc::Rc::new(Priced { sizes, got: got.clone() }))));
            }
        });
        Ok(got)
    }

    /// `plan_search`, under `RERAIL_STATS_SKEW` (a measurement aid, decision 0096): planned with each column's
    /// fanout and distinct values skewed by a fixed factor in [1/f, f]; a search is also made with the statistics as
    /// they are, and the skewed order priced with them, so `RERAIL_STATS_SKEW_LOG` gets the regret: the skewed
    /// order's work over the best order's, by the true statistics.
    fn plan_inner(
        &self,
        prog: &Program,
        head: &Atom,
        body: &[Literal],
        first: Option<usize>,
        fixed: Option<&[Step]>,
        explain: bool,
    ) -> Result<(Plan, Option<super::profile::RulePlan>)> {
        if skew_setting().is_none() {
            return self.plan_search(prog, head, body, first, fixed, explain);
        }
        SKEWING.with(|s| s.set(true));
        let got = self.plan_search(prog, head, body, first, fixed, explain);
        SKEWING.with(|s| s.set(false));
        let got = got?;
        if fixed.is_none()
            && std::env::var_os("RERAIL_STATS_SKEW_LOG").is_some()
            && let Ok((truth, _)) = self.plan_search(prog, head, body, first, None, false)
            && let Ok((priced, _)) = self.plan_search(prog, head, body, first, Some(&got.0.0), false)
        {
            use std::io::Write as _;
            static LOG: std::sync::OnceLock<Option<std::sync::Mutex<std::fs::File>>> = std::sync::OnceLock::new();
            let log = LOG.get_or_init(|| std::env::var_os("RERAIL_STATS_SKEW_LOG").and_then(|p| std::fs::OpenOptions::new().create(true).append(true).open(p).ok()).map(std::sync::Mutex::new));
            if let Some(f) = log {
                let same = shown(&got.0.0) == shown(&truth.0);
                let _ = writeln!(f.lock().unwrap(), "{:.6}\t{}\t{:.1}\t{:.1}\t{head}", priced.1 / truth.1.max(1e-9), u8::from(same), priced.1, truth.1);
            }
        }
        Ok(got)
    }

    /// The search, or with `fixed`, the pricing of that order (an error where it cannot run).
    fn plan_search(
        &self,
        prog: &Program,
        head: &Atom,
        body: &[Literal],
        first: Option<usize>,
        fixed: Option<&[Step]>,
        explain: bool,
    ) -> Result<(Plan, Option<super::profile::RulePlan>)> {
        let stats = &self.stats;
        let rule = obs_rule(head, body);
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
            // An atom that shares no bound variable, read after bindings exist, is a cross product: its fanout
            // observed in one wave or chunk is the relation's size then (none before a guest round makes the rows),
            // not a property of the join, so it is priced from the relation's own estimate (below).
            let crossed = !bound.is_empty() && !a.args.iter().any(|t| matches!(t, Term::Var(v) if bound.contains(v)));
            if !crossed
                && let Some(seen) = stats.obs.get(&(rule, atom_sig(a, bound)))
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
                    && let Some(f) = stats.const_freq(&a.rel, &self.col(&a.rel, c), k)
                {
                    return f as f64;
                }
                if let Some(k) = &k
                    && let Some(d) = stats.dfreq.get(&(a.rel.clone(), c))
                {
                    return d.get(k).copied().unwrap_or(0) as f64;
                }
                stats.fan.get(&(a.rel.clone(), c)).map(|f| (f * skew(&a.rel, c, 1)).max(1.0)).unwrap_or_else(|| self.card(&a.rel) / self.ndv_or_1(&a.rel, c))
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
                // A text test (decision 0037) probed by a string an input binds keeps the share of that column's
                // distinct strings it holds: both are measured per chunk. Priced at 1, a test of every identifier's
                // text looked free after the scan, and the planner never started from the test (notes T10, P6).
                if a.rel.starts_with(super::strings::RELATION)
                    && let [Term::Var(v)] = a.args.as_slice()
                {
                    let domain = body
                        .iter()
                        .filter_map(|l| match l {
                            Literal::Atom(b) if !b.neg && b.rel != a.rel && prog.inputs.contains(&b.rel) => {
                                b.args.iter().position(|t| matches!(t, Term::Var(x) if x == v)).map(|c| self.ndv_or_1(&b.rel, c))
                            }
                            _ => None,
                        })
                        .fold(f64::INFINITY, f64::min);
                    if domain.is_finite() {
                        rows = rows.min(self.card(&a.rel) / domain);
                    }
                }
            }
            rows.max(0.01)
        };
        let atom_rows = |a: &Atom, bound: &BTreeSet<String>| -> f64 {
            let mut rows = atom_rows_(a, bound);
            // A cross product never filters a binding: each one reads every matching row, and a relation empty
            // where the plan was priced is not empty where it is reused. At least one row per binding, so a connected
            // atom of fanout one is never ordered after it.
            let delta_atom = first.is_some_and(|f| matches!(&body[f], Literal::Atom(d) if std::ptr::eq(d, a)));
            if !delta_atom && !a.neg && !bound.is_empty() && !a.args.iter().any(|t| matches!(t, Term::Var(v) if bound.contains(v))) {
                rows = rows.max(1.0);
            }
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
                let (rname, bname) = (self.col(&a.rel, rcols[0]), self.col(&a.rel, bcols[0]));
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
        SEED.with(|c| c.set(joins.iter().filter_map(|&i| step(&body[i], 1.0, &BTreeSet::new()).map(|w| w.0)).fold(f64::INFINITY, f64::min).min(1e300)));
        if !joins.is_empty() && !joins.iter().any(|&i| matches!(body[i], Literal::Atom(_))) {
            return Err(Error::Plan(format!("rule for {head} has no positive atom")));
        }
        let mut candidates = 0u64;
        let (work, rows, order, subsets) = if joins.is_empty() {
            // A body of filters and aggregates alone (`n = count : { … }`) runs over the relational unit, one empty
            // binding: ground negations and constant expressions filter it; an unbound variable still fails.
            let (mut order, mut bound, mut rest) = (Vec::new(), BTreeSet::new(), (0..body.len()).collect());
            let rows = close(&mut order, 1.0, &mut bound, &mut rest);
            if !rest.is_empty() {
                return Err(Error::Plan(format!("unbound literals in {head}")));
            }
            (1.0, rows, order, 1)
        } else if let Some(fixed) = fixed {
            // The kept order, priced now: each join step as the search prices it, each filter once ready.
            let (mut work, mut rows, mut bound, mut order) = (0.0, 1.0, BTreeSet::new(), Vec::new());
            if fixed.len() != body.len() {
                return Err(Error::Plan("a kept plan of another body".into()));
            }
            for s in fixed {
                let l = body.get(s.at).filter(|l| **l == s.lit).ok_or_else(|| Error::Plan("a kept plan of another body".into()))?;
                if is_join(l) {
                    let (w, out, mode) = step(l, rows, &bound).ok_or_else(|| Error::Plan("a kept step cannot run".into()))?;
                    candidates += 1;
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
        // The search's variables as bits: a body with more than 128 is ordered greedily, as a long one is.
        let mut var_ids: BTreeMap<String, usize> = BTreeMap::new();
        for l in body {
            for v in l.vars() {
                let n = var_ids.len();
                var_ids.entry(v).or_insert(n);
            }
        }
        if joins.len() > GREEDY || var_ids.len() > 128 {
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
        // Variables as bits (`var_ids`): a subset's bound variables are a mask, and a set of names is made only to
        // price a join or test a filter the first time its own variables are bound so.
        let names: Vec<&String> = {
            let mut n: Vec<(&usize, &String)> = var_ids.iter().map(|(v, k)| (k, v)).collect();
            n.sort();
            n.into_iter().map(|(_, v)| v).collect()
        };
        let bits_of = |vs: BTreeSet<String>| vs.iter().fold(0u128, |m, v| m | 1u128 << var_ids[v]);
        let set_of = |b: u128| -> BTreeSet<String> { names.iter().enumerate().filter(|(k, _)| b >> k & 1 == 1).map(|(_, v)| (*v).clone()).collect() };
        let lit_bits: Vec<u128> = body.iter().map(|l| bits_of(l.vars())).collect();
        // Whether a filter is ready reads only its own variables (`ready`).
        let mut readiness: HashMap<(usize, u128), bool> = HashMap::new();
        let mut ready_at = |f: usize, bound: u128| -> bool {
            let own = bound & lit_bits[f];
            *readiness.entry((f, own)).or_insert_with(|| ready(&body[f], &set_of(own)))
        };
        // A subset's bound variables and how many halving filters it has placed.
        let mut closure = |mask: u64| -> (u128, usize) {
            let mut bound = 0u128;
            if mask == 0 {
                return (bound, 0);
            }
            for (i, &j) in joins.iter().enumerate() {
                if mask & (1 << i) != 0 {
                    bound |= lit_bits[j];
                }
            }
            let mut placed = vec![false; filters.len()];
            let mut progressed = true;
            while progressed {
                progressed = false;
                for (k, &f) in filters.iter().enumerate() {
                    if !placed[k] && ready_at(f, bound) {
                        placed[k] = true;
                        bound |= lit_bits[f];
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
        let relevant: Vec<u128> = relevant.into_iter().map(|v| bits_of(v.into_iter().collect())).collect();
        let mut prices: HashMap<(usize, u128), Price> = HashMap::new();
        let mut price = |i: usize, bits: u128| -> Price {
            let pattern = bits & relevant[i];
            let compute = || {
                let bound = &set_of(bits);
                match &body[joins[i]] {
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
                }
            };
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
        // By subset mask (at most `GREEDY` joins, so 4,096 subsets).
        let mut cells: Vec<Option<Cell>> = (0..1usize << joins.len()).map(|_| None).collect();
        let mut closed: Vec<(u128, usize)> = vec![(0, 0); 1 << joins.len()];
        cells[0] = Some(Cell { work: 0.0, rows: 1.0, prev: 0, last: usize::MAX, mode: None });
        closed[0] = closure(0);
        // Only the subsets one join more than a subset reached (with the delta first, half of them or fewer): a
        // subset no order reaches has no cell, and its closure is not worked out. Each is solved as before, its last
        // joins tried in the same order, so the plan is the same.
        let mut reached: Vec<u64> = vec![0];
        for _size in 1..=joins.len() {
            let mut next: Vec<u64> = reached.iter().flat_map(|&m| (0..joins.len()).filter(move |&j| m & (1 << j) == 0).map(move |j| m | 1 << j)).collect();
            next.sort_unstable();
            next.dedup();
            reached.clear();
            for mask in next {
                let subset: Vec<usize> = (0..joins.len()).filter(|&i| mask & (1 << i) != 0).collect();
                let mut best: Option<Cell> = None;
                let mine = closure(mask);
                for i in py_set_order(&subset) {
                    let prev = mask & !(1 << i);
                    let Some(from) = cells[prev as usize].as_ref() else { continue };
                    let l = &body[joins[i]];
                    if prev == 0 && !matches!(l, Literal::Atom(_)) {
                        continue; // an anchor is an atom
                    }
                    if prev == 0 && first.is_some_and(|f| f != joins[i]) {
                        continue; // semi-naive: the delta first
                    }
                    let (bound, h_prev) = &closed[prev as usize];
                    let Some((w, mut out, mode)) = priced(price(i, *bound), from.rows) else { continue };
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
                    cells[mask as usize] = Some(b);
                    closed[mask as usize] = mine;
                    reached.push(mask);
                }
            }
        }
        // The best orders, rebuilt: the full one, and (for `RDL_EXPLAIN`) the two-join prefixes.
        let rebuild = |mask: u64| -> Option<Best> {
            let cell = cells[mask as usize].as_ref()?;
            let mut path = Vec::new();
            let mut at = mask;
            while at != 0 {
                let c = cells[at as usize].as_ref().expect("a reached subset");
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
        for mask in (0..cells.len() as u64).filter(|&m| cells[m as usize].is_some()) {
            if mask == full_mask || (explain && mask.count_ones() == 2) {
                if let Some(b) = rebuild(mask) {
                    best_of.insert(mask, b);
                }
            }
        }
        subsets_reached = cells.iter().filter(|c| c.is_some()).count() as u64;
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
        // The orders priced, charged to the planning span they were priced in (the roofline's candidate work).
        super::profile::work(super::profile::Unit::Candidate, candidates);
        // Also when the run may use the device: placement prices each join step from these (decision 0095).
        let explained = (explain && (super::profile::on() || placement::device_on())).then(|| {
            let _explain = super::profile::span(|| "explain record".into());
            // The chosen order step by step, priced as the search priced it.
            let mut bound: BTreeSet<String> = BTreeSet::new();
            let mut r = 1.0;
            let mut steps = Vec::new();
            for s in &order {
                let cols = |a: &Atom, bound: &BTreeSet<String>| -> Vec<String> {
                    (0..a.args.len())
                        .filter(|&c| constant(&a.args[c]).is_some() || matches!(&a.args[c], Term::Var(v) if bound.contains(v)))
                        .map(|c| self.col(&a.rel, c))
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
                rows *= self.stats.const_freq(&a.rel, &self.col(&a.rel, c), &k).unwrap_or(0) as f64 / base.max(1.0);
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

/// `ProgramAnalysis::analyze`'s result for `prog`, kept for the programs analysed last: a session's runs plan the
/// same programs again (its file-local part every run, a maintained part while the same strata change), and the
/// analysis is a function of what the digest reads (each relation's columns and lattice keys, the inputs, the
/// relations with facts, the natives, and the rules in order).
fn program_analysis(prog: &Program) -> std::sync::Arc<super::analysis::ProgramAnalysis> {
    type Kept = Vec<(u128, std::sync::Arc<super::analysis::ProgramAnalysis>)>;
    /// The programs analysed last, newest at the end: a few, as each holds a result per rule.
    const KEEP: usize = 4;
    static KEPT: std::sync::Mutex<Kept> = std::sync::Mutex::new(Vec::new());
    let key = super::derived::digest(|d| {
        d.u64(prog.decls.len() as u64);
        for (rel, cols) in &prog.decls {
            d.str(rel);
            d.u64(cols.len() as u64);
            cols.iter().for_each(|(_, t)| d.str(t));
        }
        d.u64(prog.lattice.len() as u64);
        for (rel, l) in &prog.lattice {
            d.str(rel);
            d.u64(l.keys as u64);
        }
        d.u64(prog.inputs.len() as u64);
        prog.inputs.iter().for_each(|r| d.str(r));
        let facts: Vec<&String> = prog.facts.iter().filter(|(_, v)| !v.is_empty()).map(|(r, _)| r).collect();
        d.u64(facts.len() as u64);
        facts.iter().for_each(|r| d.str(r));
        d.u64(prog.natives.len() as u64);
        prog.natives.keys().for_each(|r| d.str(r));
        d.u64(prog.rules.len() as u64);
        for r in &prog.rules {
            digest_atom(d, &r.head);
            d.u64(r.body.len() as u64);
            r.body.iter().for_each(|l| digest_literal(d, l));
        }
    });
    let mut kept = KEPT.lock().unwrap();
    if let Some(at) = kept.iter().position(|(k, _)| *k == key) {
        let hit = kept.remove(at);
        let analysis = std::sync::Arc::clone(&hit.1);
        kept.push(hit);
        super::profile::decision("program analysis", || "kept: the same program analysed earlier".into());
        return analysis;
    }
    drop(kept);
    super::profile::decision("program analysis", || "analysed".into());
    let analysis = std::sync::Arc::new(super::analysis::ProgramAnalysis::analyze(prog));
    let mut kept = KEPT.lock().unwrap();
    if kept.len() >= KEEP {
        kept.remove(0);
    }
    kept.push((key, std::sync::Arc::clone(&analysis)));
    analysis
}

/// Reads the statistics and the input relations' cardinalities, distinct counts, fanouts and indexes.
fn make_plan(prog: &Program, db: &Connection, mut inputs: Inputs) -> Result<Model> {
    let analysis = std::sync::OnceLock::new();
    // A relation of the program's facts is its table too (`run`: `c0`, `c1`, …), though an input of that name has
    // named columns (the converter's `option_item`, core.rr's `option_item(name, i, v)`).
    let derived: HashSet<String> = prog.rules.iter().map(|r| r.head.rel.clone()).chain(prog.facts.keys().filter(|rel| !prog.inputs.contains(*rel)).cloned()).collect();
    let mut stats = Stats::new(db, prog, &mut inputs)?;
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
        return Ok(Model { analysis, stats, ndv, docof: HashMap::new(), view_src: HashMap::new(), inputs, derived });
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
    Ok(Model { analysis, stats, ndv, docof: HashMap::new(), view_src: HashMap::new(), inputs, derived })
}

/// T's facts file's indexes, which the base-relation vtabs' lookups mirror.
fn vtab_indexes(rel: &str) -> Vec<Vec<String>> {
    let lists: &[&[&str]] = match rel {
        "node" => &[&["n"], &["kind", "n"]],
        "child" => &[&["c"], &["p", "idx"]],
        "field" => &[&["p", "f"], &["c"]],
        "text" | "name" | "str" | "strprefix" | "node_text" | "comment" => &[&["n"], &["s", "n"]],
        // W's values (`facts::vtab::syntax::Extra`).
        "value" | "static_value" => &[&["n"], &["v", "n"]],
        "value_type_of" => &[&["n"], &["t", "n"]],
        "value_type" => &[&["v"], &["t", "v"]],
        "fstring_piece" => &[&["n"], &["v", "n"]],
        "fstring_template" => &[&["n"], &["v", "n"]],
        "cpython_syntax_error" => &[&["d"]],
        "string_annotation" => &[&["s"]],
        "parsed" => &[&["s"], &["root"]],
        "literal_json" => &[&["n"], &["v", "n"]],
        "cpython_unparse" | "cpython_ast_dump" => &[&["n"], &["s", "n"]],
        "dump_sha256" => &[&["n"], &["h", "n"]],
        "body_sha256" | "signature_sha256" => &[&["f"], &["h", "f"]],
        "tree_digest" | "def_digest" => &[&["n"], &["s", "n"]],
        "rx_span" => &[&["c"], &["m"]],
        "ast_match" => &[&["n"], &["c"]],
        "intval" | "op" | "op_t" | "tape" | "line" | "end_line" | "col" | "docof" | "visit_index" | "ast_depth" | "spread" | "reference_open" => &[&["n"]],
        "first_token" | "last_token" => &[&["n"]],
        "token_before" | "token_after" => &[&["x"]],
        "next_sibling" => &[&["a"], &["b"]],
        "split_line" | "lf_line" => &[&["d"]],
        "str_part" | "str_part_start" | "str_part_word" => &[&["lit"]],
        "reference_edge" => &[&["src"], &["dst"]],
        "core_node" => &[&["n"], &["t"], &["d"]],
        // Containment, made per node looked up either way (`facts::vtab::syntax::within`, decision 0050).
        "within" => &[&["n"], &["a"]],
        "span_within" => &[&["inner"], &["outer"]],
        "doc" | "pkg" | "syntax_error" | "source" | "line_count" => &[&["d"]],
        "src_line" | "src_line_lf" | "tape_line" => &[&["d", "l"], &["sp"]],
        "op_at" | "prev_token" => &[&["n"], &["s", "n"]],
        "token" => &[&["p", "i"], &["start"]],
        "path_part" | "file_name" => &[&["d"], &["s", "d"]],
        // The transducer's `nearest` rows (`facts::vtab::syntax::with_nearest`), looked up either way.
        rel if crate::code_rule_dsl::logic::front::is_transduced(rel) => &[&["n"], &["a"]],
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

/// Whether `t` calls a function that can have no value (`strings::functions`' NULL, as `to_number("a")`; a regex
/// group of a string the regex does not match, `rx_group`): a row that binds or derives its value needs it not NULL,
/// or a negation over it would hold (`NOT EXISTS (… = NULL)`). `x / 0` is SQL's NULL, a value the row keeps (the
/// executor's walks carry it as a nullable seed, `sql_executor::walks::nullable`).
fn partial(t: &Term) -> bool {
    matches!(t, Term::Call(f, args) if super::strings::functions::signature(f).is_some() || f == "rx_group" || args.iter().any(partial))
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
                // Soufflé's `max(a, b)` and `min(a, b)`: SQL's scalar `max` and `min`.
                ("max", [x, y]) => call("max", vec![x.clone(), y.clone()]),
                ("min", [x, y]) => call("min", vec![x.clone(), y.clone()]),
                // `.rr`'s string functions (Soufflé's): `substr` counts from 0.
                ("strlen", [x]) => length(&text_of(atoms, x)),
                ("concat", [x, y]) => symbol_of(atoms, bin(Op::Concatenate, text_of(atoms, x), text_of(atoms, y))),
                ("to_string", [x]) => symbol_of(atoms, bin(Op::Concatenate, x.clone(), string(""))),
                // A named group of a regex's match (`.rr`'s `match` with captures, `strings::group`); NULL without one.
                ("rx_group", [x, re, g]) => symbol_of(atoms, call("rx_group", vec![text_of(atoms, x), text_of(atoms, re), text_of(atoms, g)])),
                ("substr", [x, i, n]) => symbol_of(atoms, substr(vec![text_of(atoms, x), bin(Op::Add, i.clone(), integer(1)), n.clone()])),
                // `@dot` (`core.rr`): `a.b`, and `"" . b` is `b`.
                ("cat", [x, y]) if atoms.is_some() => call("sym_cat", vec![x.clone(), y.clone()]),
                // `@dot("", y)` is `y` (`core.rr`).
                ("cat", [x, y]) => Expr::Case {
                    branches: vec![(bin(Op::Equal, x.clone(), string("")), y.clone())],
                    fallback: Box::new(bin(Op::Concatenate, bin(Op::Concatenate, x.clone(), string(".")), y.clone())),
                },
                // core.rr's functors whose meaning is W's (`register_functors`), ahead of `strings::functions`' below.
                ("replace_all", [s, re, by]) => symbol_of(atoms, call("rr_replace_all", vec![text_of(atoms, s), text_of(atoms, re), text_of(atoms, by)])),
                // A JSON string literal as W's signed-list stages write one (`json_group_array`'s elements): SQLite's
                // `json_quote` of the text.
                ("json_str", [x]) => symbol_of(atoms, call("json_quote", vec![bin(Op::Concatenate, text_of(atoms, x), string(""))])),
                ("nfkc", [x]) => symbol_of(atoms, call("rr_nfkc", vec![text_of(atoms, x)])),
                ("py_repr", [x]) => symbol_of(atoms, call("rr_py_repr", vec![text_of(atoms, x)])),
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
                // `.rr`'s `@place(path, line)` (`rr::lower`'s `PLACE`): `-2 - (path << 32 | line)`, the path's symbol
                // atom and the line, below `unset` (-1) and every node id; `unset` for a line outside `0 .. 2^32`.
                // `@place_path` and `@place_line` take it apart.
                ("place", [p, l]) if atoms.is_some() => {
                    let valid = bin(Op::And, bin(Op::GreaterOrEqual, l.clone(), integer(0)), bin(Op::Less, l.clone(), integer(1 << 32)));
                    let place = bin(Op::Subtract, integer(-2), bin(Op::Add, bin(Op::Multiply, p.clone(), integer(1 << 32)), l.clone()));
                    Expr::Case { branches: vec![(valid, place)], fallback: Box::new(integer(-1)) }
                }
                ("place_path", [x]) if atoms.is_some() => bin(Op::Divide, bin(Op::Subtract, integer(-2), x.clone()), integer(1 << 32)),
                ("place_line", [x]) if atoms.is_some() => {
                    let n = bin(Op::Subtract, integer(-2), x.clone());
                    bin(Op::Subtract, n.clone(), bin(Op::Multiply, bin(Op::Divide, n, integer(1 << 32)), integer(1 << 32)))
                }
                ("place" | "place_path" | "place_line", _) => return Err(Error::Plan(format!("no SQL for {t}: a place is made of a symbol atom"))),
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
                .map(|(c, t)| Ok(eq(col(al, &self.col(&a.rel, c)), sql_term(t, env, atoms)?)))
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
                            input: Box::new(col(&al, &self.col(&l.rel, rc))),
                            low: Box::new(col(&ra, "lo")),
                            high: Box::new(col(&ra, "hi")),
                        });
                    }
                    for (c, t) in l.args.iter().enumerate() {
                        let column = col(&al, &self.col(&l.rel, c));
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
                    // A negation of a component's own (gated) relation reads its full table there.
                    let relation = match tables.and_then(|t| t.get(&l.rel)) {
                        Some((full, _)) => Relation::Bound { rel: l.rel.clone(), table: full.clone() },
                        None => self.relation(prog, &l.rel, false),
                    };
                    let query = subquery(integer(1), relation, al, conds);
                    wh.push(Expr::Exists { negated: true, query: Box::new(query) });
                }
                Literal::Cmp { l, op, r } => {
                    let lv = if let Term::Var(v) = l { Some(v) } else { None };
                    if *op == "=" && lv.is_some_and(|v| !env.contains_key(v)) {
                        let e = sql_term(r, &env, atoms)?;
                        if partial(r) {
                            wh.push(Expr::Unary { operator: SqlUnaryOperator::IsNotNull, input: Box::new(e.clone()) });
                        }
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
                        // A split of the bound whole, as `@dot` makes it: `"" . w` is `w`, else at the "." a bound
                        // part places, or at every "." with a part before it (`rdl_int`). In atoms mode the text
                        // is `sym_text`'s, and the parts are interned (`sym_atom`).
                        let inputs = &self.inputs;
                        let w = inputs.text_of(&env[lv]);
                        let Term::Call(_, parts) = r else { unreachable!() };
                        let (Term::Var(q), Term::Var(x)) = (&parts[0], &parts[1]) else { unreachable!() };
                        let case = |when: Expr, then: Expr, otherwise: Expr| Expr::Case { branches: vec![(when, then)], fallback: Box::new(otherwise) };
                        if let Some(eq_) = env.get(q).map(|e| inputs.text_of(e)) {
                            let empty = eq(eq_.clone(), string(""));
                            let prefix = eq(substr(vec![w.clone(), integer(1), bin(Op::Add, length(&eq_), integer(1))]), bin(Op::Concatenate, eq_.clone(), string(".")));
                            wh.push(bin(Op::Or, empty.clone(), prefix));
                            env.insert(x.clone(), inputs.symbol_of(case(empty, w.clone(), substr(vec![w, bin(Op::Add, length(&eq_), integer(2))]))));
                        } else if let Some(ex) = env.get(x).map(|e| inputs.text_of(e)) {
                            let whole = eq(w.clone(), ex.clone());
                            let negated = Expr::Unary { operator: SqlUnaryOperator::Negate, input: Box::new(length(&ex)) };
                            let suffix = bin(
                                Op::And,
                                eq(substr(vec![w.clone(), bin(Op::Subtract, negated, integer(1))]), bin(Op::Concatenate, string("."), ex.clone())),
                                bin(Op::Greater, length(&w), bin(Op::Add, length(&ex), integer(1))),
                            );
                            wh.push(bin(Op::Or, whole.clone(), suffix));
                            let end = bin(Op::Subtract, bin(Op::Subtract, length(&w), length(&ex)), integer(1));
                            env.insert(q.clone(), inputs.symbol_of(case(whole, string(""), substr(vec![w, integer(1), end]))));
                        } else {
                            // Position 1 stands for `"" . w`: a "." there has no part before it.
                            let al = a_name();
                            let at = col(&al, "i");
                            let first = eq(at.clone(), integer(1));
                            let conds = vec![
                                bin(Op::LessOrEqual, at.clone(), call("max", vec![length(&w), integer(1)])),
                                bin(Op::Or, first.clone(), eq(substr(vec![w.clone(), at.clone(), integer(1)]), string("."))),
                            ];
                            frm.push((FromItem::cross(Relation::Integers, al), conds));
                            env.insert(q.clone(), inputs.symbol_of(substr(vec![w.clone(), integer(1), bin(Op::Subtract, at.clone(), integer(1))])));
                            env.insert(x.clone(), inputs.symbol_of(case(first, w.clone(), substr(vec![w, bin(Op::Add, at, integer(1))]))));
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
                    // The aggregated variable and a `join`'s ordering ones are the body's own (where the rule does not
                    // bind them): not correlated.
                    let mut local: Vec<&str> = over.iter().map(String::as_str).collect();
                    if let AggOp::Join { by, .. } = op {
                        local.extend(by.iter().map(String::as_str).filter(|b| !env.contains_key(*b)));
                    }
                    let mut conds = neg_conds(atom, &al, &env, &local)?;
                    // Repeated local value fields are equal within the bag, even when
                    // the outer rule binds a variable with that name.
                    if let Some(over)=over {
                        let columns=atom.args.iter().enumerate().filter_map(|(c,t)|matches!(t,Term::Var(v) if v==over).then_some(c)).collect::<Vec<_>>();
                        for pair in columns.windows(2) {conds.push(eq(col(&al,&self.col(&atom.rel,pair[0])),col(&al,&self.col(&atom.rel,pair[1]))));}
                    }
                    let column = |v: &str| {
                        let c = atom.args.iter().position(|t| matches!(t, Term::Var(x) if x == v)).expect("the aggregated column");
                        let symbol = prog.decls.get(&atom.rel).and_then(|cols| cols.get(c)).is_some_and(|(_, t)| t == "sym");
                        (col(&al, &self.col(&atom.rel, c)), symbol)
                    };
                    let what = match (op, over) {
                        (AggOp::Count, _) | (_, None) => call("count", vec![Expr::All]),
                        (AggOp::Min | AggOp::Max, Some(o)) => call(if *op == AggOp::Min { "min" } else { "max" }, vec![column(o).0]),
                        // Soufflé's sum is 0 over no rows.
                        (AggOp::Sum, Some(o)) => call("coalesce", vec![call("rr_exact_sum", vec![column(o).0]), integer(0)]),
                        // 1 over no rows (`strings::functions::Product`).
                        (AggOp::Product, Some(o)) => call("coalesce", vec![call("rr_product", vec![column(o).0]), integer(1)]),
                        // `rr_join(sep, value, key, …)` (`register_functors`) orders and joins the distinct rows' text;
                        // symbols in as text.
                        (AggOp::Join { by, sep }, Some(o)) => {
                            let text = |(e, symbol): (Expr, bool)| if symbol { self.inputs.text_of(&e) } else { e };
                            let mut args = vec![string(sep), text(column(o))];
                            for b in by {
                                args.push(match env.get(b) {
                                    Some(e) => e.clone(),
                                    None => text(column(b)),
                                });
                            }
                            self.inputs.symbol_of(call("coalesce", vec![call("rr_join", args), string("")]))
                        }
                    };
                    let query = subquery(what, self.relation(prog, &atom.rel, false), al, conds);
                    let value = Expr::Scalar(Box::new(query));
                    // Planning may bind the result first (`n = 0` before the count): the aggregate then tests that
                    // binding, which replacing it would drop.
                    if let Some(bound) = env.get(var) {
                        wh.push(eq(bound.clone(), value));
                    } else {
                        env.insert(var.clone(), value);
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
        let mut columns = Vec::new();
        for (i, t) in head.args.iter().enumerate() {
            let e = sql_term(t, &env, atoms)?;
            if partial(t) {
                wh.push(Expr::Unary { operator: SqlUnaryOperator::IsNotNull, input: Box::new(e.clone()) });
            }
            columns.push((e, Some(format!("c{i}"))));
        }
        // A safe scalar rule (global aggregates, ground comparisons, or Boolean negation)
        // evaluates once. SQLite permits its SELECT and WHERE without an outer FROM.
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
    /// The kept relations handed to the caller during the run (`run_atoms_native_handing`), no longer held.
    pub(in crate::code_rule_dsl) handed: BTreeSet<String>,
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
struct Run<'a, 'b> {
    prog: &'a Program,
    db: &'a Connection,
    model: Model,
    views: HashSet<String>,
    made: HashSet<(String, Vec<usize>)>,
    /// Each derived table's secondary indexes, as column orders (`index_for`).
    orders: HashMap<String, Vec<Vec<usize>>>,
    /// Derived tables with no key: views materialized as plain tables (`ensure_indexes`).
    keyless: HashSet<String>,
    /// Relations held only by the native set engine (`program`) until `materialize`.
    native_rels: BTreeSet<String>,
    /// Why the native engine declined the last stratum it was offered (EXPLAIN's `why`).
    why_not: Option<String>,
    /// What kind of literal the native engine declined (`decline_class`), for the profile's SQLite fallbacks.
    why_class: Option<String>,
    /// Derived relations whose table is made only when SQL first needs it (`ensure`): its `CREATE` statement. A
    /// relation the native engine derives and keeps never has one.
    pending: HashMap<String, String>,
    /// Relations whose one rule projects one atom (`kind($n, $k) :- node(_, $n, $k, _, _)`, variables distinct): a
    /// native join reads such a view from its input in place. Found once per program, not per atom of each spec.
    projections: HashSet<String>,
    /// The program's inputs, as a set: asked per atom of each spec (`prog.inputs` is a list).
    inputs: rustc_hash::FxHashSet<&'a str>,
    /// Each rule's number in the program, by address: for spans, EXPLAIN and the engine's reasons. Made once per
    /// program (a map of every rule per stratum cost more than the stratum's own planning).
    numbers: rustc_hash::FxHashMap<*const Rule, usize>,
    /// Answers a request (decision 0065 §2a): the request relation and its new rows, to the produced relations'
    /// rows. A component with a request pauses for it (`setfix`'s `request`).
    answer: Option<Answer<'b>>,
    /// Each produced relation's rows answered so far, for a stratum above its request's (its table's rows).
    answered: BTreeMap<String, Vec<Vec<i64>>>,
}

/// The runner's answer to requests (`Run::answer`): a request relation and its new rows (atoms and integers), to the
/// rows each produced relation gains.
pub(in crate::code_rule_dsl) type Answer<'a> = &'a mut dyn FnMut(&str, &[Vec<i64>]) -> Result<Vec<(String, Vec<Vec<i64>>)>>;

/// Answers a paused component's requests and adds the answers to its produced relations (decision 0065 §2a).
/// Rows for a produced relation outside it are kept for the stratum that holds that relation.
fn answer_pending(db: &Connection, direct: bool, answer: &mut Option<Answer<'_>>, answered: &mut BTreeMap<String, Vec<Vec<i64>>>, name: &str, scc: &BTreeSet<String>) -> Result<()> {
    if !direct {
        return Err(Error::Plan(format!("{name}: requests are answered only over atoms (the in-process engine)")));
    }
    let asked = super::native::component::pending(db, name)?;
    let answer = answer.as_deref_mut().ok_or_else(|| Error::Plan(format!("{name}: a request, and no one to answer it")))?;
    let mut by: BTreeMap<String, Vec<Vec<i64>>> = BTreeMap::new();
    for (rel, rows) in &asked {
        let _span = super::profile::span(|| format!("answer {rel} ({} rows)", rows.len()));
        for (p, got) in answer(rel, rows)? {
            by.entry(p).or_default().extend(got);
        }
    }
    for (p, rows) in by {
        if scc.contains(&p) {
            super::native::component::add(db, name, &p, &rows)?;
        } else {
            answered.entry(p).or_default().extend(rows);
        }
    }
    Ok(())
}

/// The relations of `prog` whose one rule's body is a single atom of distinct variables and wildcards.
fn projections(prog: &Program) -> HashSet<String> {
    let mut by_head: HashMap<&str, Vec<&Rule>> = HashMap::new();
    for r in &prog.rules {
        by_head.entry(r.head.rel.as_str()).or_default().push(r);
    }
    by_head
        .into_iter()
        .filter(|(_, rules)| {
            matches!(rules.as_slice(), [r] if matches!(r.body.as_slice(), [Literal::Atom(a)] if {
                let vars: Vec<&String> = a.args.iter().filter_map(|t| if let Term::Var(v) = t { Some(v) } else { None }).collect();
                a.args.iter().all(|t| matches!(t, Term::Var(_) | Term::Wild)) && vars.iter().collect::<HashSet<_>>().len() == vars.len()
            }))
        })
        .map(|(rel, _)| rel.to_owned())
        .collect()
}

/// A recursive atom's delta variant: the rule's head, its plan with that atom first, and the atom's plan position.

impl Run<'_, '_> {
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
                    let q = crate::code_rule_dsl::logic::sql::ident(r);
                    self.db.execute_batch(&format!(
                        "CREATE TEMP TABLE {r}__m AS SELECT * FROM {q}; DROP VIEW {q}; ALTER TABLE {r}__m RENAME TO {q};"
                    ))?;
                    self.views.remove(r);
                    self.model.view_src.remove(r);
                    self.keyless.insert(r.clone());
                }
            }
            if let Some(a) = a
                && !self.inputs.contains(a.rel.as_str())
                && !self.views.contains(&a.rel)
            {
                let cols: Vec<usize> = (0..a.args.len())
                    .filter(|&c| constant(&a.args[c]).is_some() || matches!(&a.args[c], Term::Var(v) if bound.contains(v)))
                    .collect();
                if !cols.is_empty() && self.made.insert((a.rel.clone(), cols.clone())) {
                    self.index_for(&a.rel, &cols)?;
                }
            }
            bound.extend(s.lit.vars());
        }
        Ok(())
    }

    /// An index serving an equality search on `cols` of derived table `rel`, with as few indexes as serve every
    /// search: each index is one column order, and serves a search whose columns are its first ones. The table's
    /// key (all columns in order, or a lattice's key columns; `WITHOUT ROWID`, so the table is that order) serves
    /// its prefixes. A search containing an index's columns extends that index (rebuilt once, the old one dropped)
    /// rather than adding one, so the indexes form chains of searches (Soufflé's index selection: a minimum chain
    /// cover, here greedy as searches arrive). Each secondary index of a `WITHOUT ROWID` table holds the whole row,
    /// a copy of the relation that every insert maintains: on `calls.rr` this makes 22 indexes where one per search
    /// made 96.
    fn index_for(&mut self, rel: &str, cols: &[usize]) -> Result<()> {
        let want: BTreeSet<usize> = cols.iter().copied().collect();
        let key = if self.keyless.contains(rel) {
            0
        } else {
            self.prog.lattice.get(rel).map_or_else(|| self.prog.decls.get(rel).map_or(0, Vec::len), |l| l.keys)
        };
        if want.len() <= key && want.iter().all(|&c| c < want.len()) {
            return Ok(());
        }
        let orders = self.orders.entry(rel.to_owned()).or_default();
        let prefix = |o: &[usize]| o.len() >= want.len() && o[..want.len()].iter().copied().collect::<BTreeSet<_>>() == want;
        if orders.iter().any(|o| prefix(o)) {
            return Ok(());
        }
        let name = |i: usize| format!("{rel}__o{i}");
        let on = |o: &[usize]| o.iter().map(|c| format!("c{c}")).collect::<Vec<_>>().join(", ");
        // The longest order whose columns the search contains, extended by the search's other columns.
        let extend = orders.iter().enumerate().filter(|(_, o)| o.iter().all(|c| want.contains(c))).max_by_key(|(_, o)| o.len()).map(|(i, _)| i);
        let i = match extend {
            Some(i) => {
                let mut o = orders[i].clone();
                o.extend(want.iter().filter(|c| !orders[i].contains(c)));
                orders[i] = o;
                i
            }
            None => {
                orders.push(want.iter().copied().collect());
                orders.len() - 1
            }
        };
        self.db.execute_batch(&format!("DROP INDEX IF EXISTS {}; CREATE INDEX {} ON {}({})", name(i), name(i), crate::code_rule_dsl::logic::sql::ident(rel), on(&orders[i])))?;
        Ok(())
    }

    fn count(&self, sql: &str) -> Result<i64> {
        Ok(self.db.query_row(sql, [], |r| r.get(0))?)
    }

    /// The rows `select` added to `head`'s relation (keys changed, for a lattice).
    fn insert_rows(&self, head: &Atom, select: &str) -> Result<usize> {
        Ok(if self.prog.lattice.contains_key(&head.rel) {
            sql_kernel::merge(self, &head.rel, select)?
        } else {
            let statement = format!("INSERT OR IGNORE INTO {} {select}", crate::code_rule_dsl::logic::sql::ident(&head.rel));
            super::profile::plan(self.db, &statement);
            self.db.execute(&statement, [])?
        })
    }

    /// Makes `rel`'s rows natively from the tables of the relations it reads (`front::Native`); returns them.
    fn native_relation(&mut self, rel: &str, native: &crate::code_rule_dsl::logic::front::Native) -> Result<usize> {
        let reads: Vec<String> = native.reads().into_iter().map(str::to_owned).collect();
        self.ensure(&reads)?;
        self.materialize(&reads)?;
        use crate::code_rule_dsl::logic::front::Native;
        let (edge, target, roots) = match native {
            Native::DfsFirst { edge, target, roots } => (edge, target, roots),
            Native::MatchAll { input, pattern, groups, positions } => return self.match_all(rel, input, pattern, groups, *positions),
            Native::Components { edge, own } => return self.components(rel, edge, own),
        };
        let cols = |r: &str, n: usize| (0..n).map(|c| self.model.col(r, c)).collect::<Vec<_>>();
        let read = |sql: &str, width: usize| -> Result<Vec<Vec<i64>>> {
            let mut st = self.db.prepare(sql)?;
            let rows = st.query_map([], |row| (0..width).map(|i| row.get::<_, i64>(i)).collect::<rusqlite::Result<Vec<i64>>>())?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        };
        let e = cols(edge, 3);
        let edges: Vec<(i64, i64, i64)> = read(&format!("SELECT DISTINCT {}, {}, {} FROM {}", e[0], e[1], e[2], crate::code_rule_dsl::logic::sql::ident(edge)), 3)?.into_iter().map(|r| (r[0], r[1], r[2])).collect();
        let t = cols(target, 1);
        let targets: Vec<i64> = read(&format!("SELECT DISTINCT {} FROM {}", t[0], crate::code_rule_dsl::logic::sql::ident(target)), 1)?.into_iter().map(|r| r[0]).collect();
        // `dfs_first_memo`'s roots in increasing order of their second column: a number, or a symbol as text.
        let roots: Option<Vec<i64>> = match roots {
            Some(roots) => {
                let r = cols(roots, 2);
                let symbol = self.prog.decls[roots.as_str()][1].1 == "sym" && self.model.inputs.atoms.is_some();
                let key = if symbol { format!("sym_text({})", r[1]) } else { r[1].clone() };
                let mut seen = BTreeSet::new();
                let ordered = read(&format!("SELECT {}, {key} FROM {} ORDER BY 2, 1", r[0], crate::code_rule_dsl::logic::sql::ident(roots)), 1)?;
                Some(ordered.into_iter().map(|row| row[0]).filter(|r| seen.insert(*r)).collect())
            }
            None => None,
        };
        let rows = crate::code_rule_dsl::logic::native::dfs_first::dfs_first(&edges, &targets, roots.as_deref());
        let n = rows.len();
        insert_many(self.db, &format!("INSERT OR IGNORE INTO {}", crate::code_rule_dsl::logic::sql::ident(rel)), rows.into_iter().map(|(a, b, c)| vec![SqlValue::Integer(a), SqlValue::Integer(b), SqlValue::Integer(c)]).collect())?;
        Ok(n)
    }

    /// `components` (`front::Native::Components`): each document's component and key from `edge`'s and `own`'s
    /// rows. Digests and keys are symbols: atoms (interned by `sym_atom`) or text, as the run stores them.
    fn components(&mut self, rel: &str, edge: &str, own: &str) -> Result<usize> {
        let atoms = self.model.inputs.atoms.is_some();
        let ident = crate::code_rule_dsl::logic::sql::ident;
        let (e0, e1) = (self.model.col(edge, 0), self.model.col(edge, 1));
        let edges: Vec<(i64, i64)> = {
            let mut st = self.db.prepare(&format!("SELECT DISTINCT {e0}, {e1} FROM {}", ident(edge)))?;
            let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let (o0, o1) = (self.model.col(own, 0), self.model.col(own, 1));
        let text = if atoms { format!("sym_text({o1})") } else { o1.clone() };
        let digests: Vec<(i64, String)> = {
            let mut st = self.db.prepare(&format!("SELECT DISTINCT {o0}, {text} FROM {}", ident(own)))?;
            let rows = st.query_map([], |r| Ok((r.get(0)?, r.get::<_, Option<String>>(1)?.unwrap_or_default())))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let rows = crate::code_rule_dsl::logic::native::components::components(&edges, &digests);
        let mut ids = if atoms { super::native::intern_batch(self.db, rows.iter().map(|(_, _, k)| k.as_str()))?.into_iter() } else { Vec::new().into_iter() };
        let values = rows
            .into_iter()
            .map(|(d, c, k)| {
                let k = if atoms { SqlValue::Integer(ids.next().expect("one atom per key")) } else { SqlValue::Text(k) };
                vec![SqlValue::Integer(d), SqlValue::Integer(c), k]
            })
            .collect();
        let before = self.db.total_changes();
        insert_many(self.db, &format!("INSERT OR IGNORE INTO {}", ident(rel)), values)?;
        Ok((self.db.total_changes() - before) as usize)
    }

    /// `match_all` over symbols (`front::Native::MatchAll`): each distinct text of `input`, and each match's groups.
    /// Symbols are atoms (interned by `sym_atom`) or text, as the run stores them.
    fn match_all(&mut self, rel: &str, input: &str, pattern: &str, groups: &[String], positions: bool) -> Result<usize> {
        let re = regex::Regex::new(pattern).map_err(|e| Error::Plan(format!("match_all: bad regex {pattern:?}: {e}")))?;
        let atoms = self.model.inputs.atoms.is_some();
        let c = self.model.col(input, 0);
        let text = if atoms { format!("sym_text({c})") } else { c.clone() };
        let texts: Vec<(SqlValue, String)> = {
            let mut st = self.db.prepare(&format!("SELECT DISTINCT {c}, {text} FROM {}", crate::code_rule_dsl::logic::sql::ident(input)))?;
            let rows = st.query_map([], |row| Ok((row.get::<_, SqlValue>(0)?, row.get::<_, Option<String>>(1)?)))?;
            rows.filter_map(|r| r.map(|(v, t)| t.map(|t| (v, t))).transpose()).collect::<rusqlite::Result<_>>()?
        };
        let captures: Vec<_> = texts.iter().flat_map(|(v, t)| {
            use crate::code_rule_dsl::logic::native::match_all;
            let rows: Vec<_> = if positions {
                match_all::occurrences(&re, groups, t).into_iter().map(|(row, at)| (row, Some(at))).collect()
            } else {
                match_all::rows(&re, groups, t).into_iter().map(|row| (row, None)).collect()
            };
            rows.into_iter().map(move |(row, at)| (v, row, at))
        }).collect();
        let mut ids = if atoms {
            super::native::intern_batch(self.db, captures.iter().flat_map(|(_, row, _)| row.iter().copied()))?.into_iter()
        } else { Vec::new().into_iter() };
        let rows = captures.into_iter().map(|(v, row, at)| {
            std::iter::once(v.clone()).chain(row.into_iter().map(|g| {
                if atoms { SqlValue::Integer(ids.next().expect("one atom per capture")) }
                else { SqlValue::Text(g.to_owned()) }
            })).chain(at.into_iter().flatten().map(SqlValue::Integer)).collect()
        }).collect();
        let before = self.db.total_changes();
        insert_many(self.db, &format!("INSERT OR IGNORE INTO {}", crate::code_rule_dsl::logic::sql::ident(rel)), rows)?;
        Ok((self.db.total_changes() - before) as usize)
    }

    fn refresh(&mut self, r: &str, at_least_one: bool) -> Result<()> {
        let n = self.count(&format!("SELECT count(*) FROM {}", crate::code_rule_dsl::logic::sql::ident(r)))? as f64;
        self.model.stats.card.insert(r.to_owned(), if at_least_one { n.max(1.0) } else { n });
        Ok(())
    }
}

/// Adds a native join's step counts ("in:out" per step, `set_fixpoint_steps`) to the observed fanouts of the binding
/// patterns its positive atoms were read with (T's `observe`). The first step (the delta, or a seed's anchor) is
/// skipped.
fn observe(stats: &mut Stats, rule: u64, order: &[Step], counts: &[(i64, i64)]) {
    // A probe that ran no step past its first (its delta empty this chunk) observed nothing.
    if counts.iter().skip(1).all(|&(n_in, _)| n_in == 0) {
        return;
    }
    let mut bound = BTreeSet::new();
    for (i, (s, &(n_in, n_out))) in order.iter().zip(counts).enumerate() {
        if i > 0
            && let Literal::Atom(l) = &s.lit
            && !l.neg
            && n_in != 0
        {
            let o = stats.obs.entry((rule, atom_sig(l, &bound))).or_insert([0, 0]);
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
    prog.decls.get(r).unwrap_or_else(|| panic!("no declaration of {r}")).iter().map(|(_, t)| t.as_str()).collect()
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

/// A recursive relation's statistics measured before this run, when the caller gave them under its name
/// (`Stats::measured`: the cross-file rounds give each hop relation's rows of the round before): its rows, and its
/// columns' distinct counts and fanouts, which otherwise start at 1 until the first replan.
fn prior_stats(model: &mut Model, prog: &Program, r: &str) -> Option<f64> {
    let width = prog.decls.get(r)?.len();
    let measured = |of: &str| (0..width).map(|c| model.stats.col.get(&(of.to_owned(), sql_col(of, c))).copied()).collect::<Option<Vec<[i64; 4]>>>();
    // A maintained relation starts as its old rows (`rel r … base r__old r__del`), which are measured under that
    // name: without them every column read as one value, so a bound column narrowed nothing (`cfn($c, $e, $f)` with
    // `$c` bound priced as all 2,139 rows, and the plan took cross products of `rpvar` and `attr_of__plus` instead).
    let cols = measured(r).or_else(|| measured(&format!("{r}__old")))?;
    let rows = cols.first().map_or(0, |m| m[0]);
    for (c, [n, ndv, sq, _]) in cols.into_iter().enumerate() {
        model.ndv.insert((r.to_owned(), c), ndv.max(1) as f64);
        model.stats.fan.insert((r.to_owned(), c), if n != 0 { sq as f64 / n as f64 } else { 1.0 });
    }
    Some((rows as f64).max(1.0))
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
/// The kind of literal the native set engine has no step for, as the profile groups SQLite's fallbacks: an
/// aggregate by its operator, a function by its name (the first one in the literal), a comparison by its operator,
/// a read by its relation when it is an input, else by who holds it (a relation an earlier SQLite stratum made is
/// a fallback that brings on the next).
fn decline_class(lit: &Literal, prog: &Program, native: &BTreeSet<String>) -> String {
    fn first_call(t: &Term) -> Option<&str> {
        match t {
            Term::Call(f, args) => Some(args.iter().find_map(first_call).filter(|_| f == "concat" || f == "add").unwrap_or(f.as_str())),
            _ => None,
        }
    }
    match lit {
        Literal::Agg { op, .. } => format!("aggregate {}", match op {
            super::front::AggOp::Count => "count",
            super::front::AggOp::Min => "min",
            super::front::AggOp::Max => "max",
            super::front::AggOp::Sum => "sum",
            super::front::AggOp::Product => "product",
            super::front::AggOp::Join { .. } => "join (string concatenation)",
        }),
        Literal::Atom(a) if prog.inputs.iter().any(|i| *i == a.rel) => format!("{}read of input {}", if a.neg { "negated " } else { "" }, a.rel),
        Literal::Atom(a) if native.contains(&a.rel) => format!("{}read of a relation the native engine holds", if a.neg { "negated " } else { "" }),
        Literal::Atom(a) => format!("{}read of a relation an earlier SQLite stratum made", if a.neg { "negated " } else { "" }),
        Literal::Cmp { l, op, r } => match first_call(l).or_else(|| first_call(r)) {
            Some(f) => format!("function @{f}"),
            None => format!("comparison {op}"),
        },
        other => format!("{}", other).split('(').next().unwrap_or("literal").to_owned(),
    }
}

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

/// Whether two orders of one body are the same: each step's position, literal and mode (what `shown` compared as
/// text, without writing either out).
fn same_order(a: &[Step], b: &[Step]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.at == y.at && x.mode == y.mode && x.lit == y.lit)
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
/// A term, atom or literal written into a derived value's inputs (`logic::derived`), field by field: the same
/// pieces in the same order as its text, without making the text.
fn digest_term(d: &mut super::derived::Inputs, t: &Term) {
    match t {
        Term::Var(v) => {
            d.u64(0);
            d.str(v);
        }
        Term::Wild => d.u64(1),
        Term::Int(i) => {
            d.u64(2);
            d.u64(*i as u64);
        }
        Term::Str(s) => {
            d.u64(3);
            d.str(s);
        }
        Term::Top => d.u64(4),
        Term::Call(f, args) => {
            d.u64(5);
            d.str(f);
            d.u64(args.len() as u64);
            args.iter().for_each(|a| digest_term(d, a));
        }
    }
}

fn digest_atom(d: &mut super::derived::Inputs, a: &Atom) {
    d.str(&a.rel);
    d.u64(u64::from(a.neg) | u64::from(a.gated) << 1);
    d.u64(a.args.len() as u64);
    a.args.iter().for_each(|t| digest_term(d, t));
}

fn digest_literal(d: &mut super::derived::Inputs, l: &Literal) {
    match l {
        Literal::Atom(a) => {
            d.u64(0);
            digest_atom(d, a);
        }
        Literal::Cmp { l, op, r } => {
            d.u64(1);
            d.str(op);
            digest_term(d, l);
            digest_term(d, r);
        }
        Literal::Agg { var, op, over, atom } => {
            d.u64(2);
            d.str(var);
            match op {
                AggOp::Join { by, sep } => {
                    d.u64(4);
                    d.u64(by.len() as u64);
                    by.iter().for_each(|b| d.str(b));
                    d.str(sep);
                }
                // Each aggregate its own tag: `sum` and `product` over one atom are different values.
                AggOp::Count => d.u64(0),
                AggOp::Min => d.u64(1),
                AggOp::Max => d.u64(2),
                AggOp::Sum => d.u64(3),
                AggOp::Product => d.u64(5),
            }
            d.str(over.as_deref().unwrap_or("\u{0}"));
            digest_atom(d, atom);
        }
        Literal::Nearest { n, a, kindset } => {
            d.u64(3);
            d.str(n);
            d.str(a);
            d.str(kindset);
        }
        Literal::Ast { document, node, matcher, captures } => {
            d.u64(4);
            d.str(document);
            d.str(node);
            d.str(matcher);
            d.u64(captures.len() as u64);
            for (k, v) in captures {
                d.str(k);
                d.str(v);
            }
        }
    }
}

fn const_tok(t: &Term) -> Option<String> {
    match t {
        Term::Int(i) => Some(format!("i:{i}")),
        Term::Str(s) => Some(format!("x:{}", hex(s))),
        _ => None,
    }
}

impl Run<'_, '_> {
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
        // An adopted input is bound as a source too, which SQL reads: it stays native.
        let held: BTreeSet<String> = rels.iter().filter(|r| self.native_rels.contains(*r) && !self.inputs.contains(r.as_str())).cloned().collect();
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
    /// `join_spec` for a whole body: `None` too when a head variable is bound by none of its steps (the native join
    /// would read past its slots). A prefix of a body (the probe for how far a body runs natively) is not checked.
    fn whole_join_spec(&self, h: &Atom, order: &[Step], at: Option<usize>, ri: &BTreeMap<String, usize>, scc: &BTreeSet<String>, native_rels: &BTreeSet<String>) -> Option<String> {
        let mut bound = BTreeSet::new();
        for s in order {
            let mut add = |t: &Term| if let Term::Var(v) = t { bound.insert(v.clone()); };
            match &s.lit {
                Literal::Atom(l) if !l.neg => l.args.iter().for_each(&mut add),
                // `$s = @cat($m, $x)` with s bound splits s: m and x are bound too.
                Literal::Cmp { l, op, r: Term::Call(f, args) } if *op == "=" && f == "cat" => { add(l); args.iter().for_each(&mut add) }
                Literal::Cmp { l, op, r } if *op == "=" => { add(l); add(r) }
                Literal::Nearest { n, a, .. } => { add(&Term::Var(n.clone())); add(&Term::Var(a.clone())) }
                Literal::Agg { var, .. } => add(&Term::Var(var.clone())),
                _ => {}
            }
        }
        let head_vars = h.args.iter().flat_map(|t| match t { Term::Call(_, a) => a.iter().collect::<Vec<_>>(), t => vec![t] });
        if head_vars.filter_map(|t| if let Term::Var(v) = t { Some(v) } else { None }).any(|v| !bound.contains(v)) {
            return None;
        }
        self.join_spec(h, order, at, ri, scc, native_rels)
    }

    /// A rule's native join spec (`join_spec_made`), kept for the run by a digest of every input it reads
    /// (`logic::derived`): each chunk's program is new, and most of its strata plan the same orders over the same
    /// sources.
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
        let inputs = |d: &mut super::derived::Inputs| {
            digest_atom(d, h);
            d.u64(at.map_or(u64::MAX, |a| a as u64));
            match self.model.inputs.atoms {
                Some(ops) => ops.iter().for_each(|&o| d.u64(o as u64)),
                None => d.u64(u64::MAX),
            }
            let read = |rel: &str, d: &mut super::derived::Inputs| {
                d.str(rel);
                if let Some(i) = ri.get(rel).filter(|_| scc.contains(rel)) {
                    d.u64(0);
                    d.u64(*i as u64);
                } else if native_rels.contains(rel) {
                    d.u64(1);
                } else {
                    d.u64(2);
                    d.str(&self.model.relation(prog, rel, false).render());
                    d.u64(u64::from(self.inputs.contains(rel)) | u64::from(self.model.derived.contains(rel)) << 1 | u64::from(self.projections.contains(rel)) << 2);
                    match self.model.view_src.get(rel) {
                        Some((base, cols)) => {
                            d.str(base);
                            d.u64(cols.len() as u64);
                            for c in cols {
                                d.str(c.as_deref().unwrap_or("\u{0}"));
                            }
                        }
                        None => d.u64(u64::MAX),
                    }
                }
                let decl = prog.decls.get(rel).map_or(&[][..], Vec::as_slice);
                d.u64(decl.len() as u64);
                for (_, t) in decl {
                    d.str(t);
                }
            };
            for s in order {
                digest_literal(d, &s.lit);
                d.u64(match s.mode {
                    None => 0,
                    Some(Mode::Range) => 1,
                    Some(Mode::Climb) => 2,
                    Some(Mode::Descend) => 3,
                });
                match &s.lit {
                    Literal::Atom(l) | Literal::Agg { atom: l, .. } => read(&l.rel, d),
                    Literal::Nearest { kindset, .. } => {
                        let kinds = prog.kindsets.get(kindset).map_or(&[][..], Vec::as_slice);
                        d.u64(kinds.len() as u64);
                        kinds.iter().for_each(|k| d.str(k));
                    }
                    _ => {}
                }
            }
        };
        super::derived::value("join spec", inputs, || self.join_spec_made(h, order, at, ri, scc, native_rels))
    }

    fn join_spec_made(
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
                // A variable's type: its column's in a positive atom; a count's, a sum's or a number function's
                // value is a number.
                Term::Var(n) => order.iter().find_map(|s| match &s.lit {
                    Literal::Atom(l) if !l.neg => l
                        .args
                        .iter()
                        .position(|x| matches!(x, Term::Var(v) if v == n))
                        .and_then(|c| Some(prog.decls.get(&l.rel)?.get(c)?.1.clone())),
                    Literal::Agg { var, op: AggOp::Count | AggOp::Sum | AggOp::Product, .. } if var == n => Some("int".to_owned()),
                    Literal::Cmp { l: Term::Var(v), op: "=", r: r @ Term::Call(..) } if v == n && numeric_call(r) => Some("int".to_owned()),
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
        // A computed term (`@concat`, the text and number functions `strings::functions::native_call` names) of
        // bound arguments, nested on either side, as `func` steps, inner ones first, each into a slot of its own
        // (`rr_fn<step>_<k>`): the term of its value. A bound variable or a constant is itself; anything else (an
        // unbound variable, a call with no native function) has none.
        fn expr(t: &Term, idx: usize, slots: &mut HashMap<String, usize>, lines: &mut Vec<String>) -> Option<Term> {
            match t {
                Term::Var(v) => slots.contains_key(v).then(|| t.clone()),
                Term::Int(_) | Term::Str(_) => Some(t.clone()),
                Term::Call(f, args) => {
                    let name = call_name(f, args.len())?;
                    let made: Vec<Term> = args.iter().map(|a| expr(a, idx, slots, lines)).collect::<Option<_>>()?;
                    let toks: Vec<String> = made.iter().map(|a| tok(a, slots)).collect::<Option<_>>()?;
                    let out = Term::Var(format!("rr_fn{idx}_{}", lines.len()));
                    lines.push(format!("func {name} {} {}", tok(&out, slots)?, toks.join(" ")));
                    Some(out)
                }
                _ => None,
            }
        }
        // The native function of call `f` with `n` arguments (`@concat` is `str_concat`).
        fn call_name(f: &str, n: usize) -> Option<&'static str> {
            if f == "concat" {
                return (n == 2).then_some("str_concat");
            }
            let (name, kinds, _) = crate::code_rule_dsl::logic::strings::functions::native_call(f)?;
            (kinds.len() == n).then_some(name)
        }
        // Whether a computed term's value is a number (a function's result kind), not a symbol.
        fn numeric_call(t: &Term) -> bool {
            matches!(t, Term::Call(f, _) if f != "concat"
                && crate::code_rule_dsl::logic::strings::functions::native_call(f).is_some_and(|(_, _, k)| k == crate::code_rule_dsl::logic::strings::functions::Kind::Number))
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
            let projected = self.model.view_src.get(&l.rel).filter(|(_, vc)| vc.iter().all(Option::is_some)).filter(|_| self.projections.contains(&l.rel));
            let sql_col = |rel: &str, c: usize| match projected { Some((_, vc)) => vc[c].clone().unwrap(), None => self.model.col(rel, c) };
            let cols = keep.iter().map(|&c| sql_col(&l.rel, c)).collect::<Vec<_>>().join(", ");
            let cols = if cols.is_empty() { "1".to_owned() } else { cols };
            let mut wheres = Vec::new();
            for (c, t) in l.args.iter().enumerate() {
                if is_const(t) {
                    wheres.push(format!("{} = {}", sql_col(&l.rel, c), sql_term(t, &HashMap::new(), self.model.inputs.atoms).ok()?.render()));
                }
            }
            // A relation this program part does not declare (an input view of the whole) is not joined natively: SQL
            // reads it.
            let decl = prog.decls.get(&l.rel)?;
            let types: String = keep
                .iter()
                .map(|&c| sym_char(&self.model.inputs, &decl.get(c).unwrap_or_else(|| panic!("{} is declared with {} columns and read as {l}", l.rel, decl.len())).1))
                .collect();
            let types = if types.is_empty() { "i".to_owned() } else { types };
            let mut sel = self.model.relation(prog, projected.map_or(&l.rel, |(base, _)| base), false).render();
            if !wheres.is_empty() {
                sel += &format!(" WHERE {}", wheres.join(" AND "));
            }
            let mut toks: Vec<String> = keep.iter().map(|&c| tok(&l.args[c], slots)).collect::<Option<_>>()?;
            if toks.is_empty() {
                toks.push("_".to_owned());
            }
            let base = if self.inputs.contains(l.rel.as_str()) || self.model.view_src.contains_key(&l.rel) { "b" } else { "d" };
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
                Literal::Cmp { l: Term::Call(fl, xl), op, r: Term::Call(fr, xr) }
                    if (*op == "=" || *op == "!=") && fl == "doc" && fr == "doc" && xl.len() == 1 && xr.len() == 1 =>
                {
                    // `@doc($a) = @doc($b)` (cross_rounds' hops): both nodes bound.
                    let bound = |t: &Term| matches!(t, Term::Var(v) if slots.contains_key(v));
                    if !bound(&xl[0]) || !bound(&xr[0]) {
                        return None;
                    }
                    let (ta, tb) = (tok(&xl[0], &mut slots)?, tok(&xr[0], &mut slots)?);
                    lines.push(format!("{} {ta} {tb}", if *op == "=" { "deq" } else { "dne" }));
                }
                Literal::Cmp { l: Term::Var(d), op: "=", r: Term::Call(f, args) } if f == "doc" && args.len() == 1 => {
                    // `$d = @doc($n)` (the cross-file memo's guard, decision 0076): n bound; d bound or tested.
                    if !matches!(&args[0], Term::Var(v) if slots.contains_key(v)) {
                        return None;
                    }
                    let tn = tok(&args[0], &mut slots)?;
                    lines.push(format!("docof {} {tn}", tok(&Term::Var(d.clone()), &mut slots)?));
                }
                Literal::Cmp { l: Term::Call(f, args), op: "=", r: Term::Int(want @ (0 | 1)) }
                    if (f == "in" || f == "equals")
                        && let [x, Term::Call(set, members)] = args.as_slice()
                        && set == crate::code_rule_dsl::logic::strings::SET
                        && members.iter().all(|m| matches!(m, Term::Int(_)))
                        && matches!(x, Term::Var(v) if slots.contains_key(v)) =>
                {
                    // `@equals($q, {"a", "b"}) = 1` once lowered (the strings are symbols): membership, natively.
                    let members: Vec<String> = members.iter().map(|m| tok(m, &mut slots)).collect::<Option<_>>()?;
                    lines.push(format!("in {} {want} {}", tok(x, &mut slots)?, members.join(" ")));
                }
                Literal::Cmp { l: Term::Call(f, args), op: "=", r: Term::Int(want @ (0 | 1)) }
                    if crate::code_rule_dsl::logic::strings::is_test(f)
                        && let [x, Term::Call(set, members)] = args.as_slice()
                        && set == crate::code_rule_dsl::logic::strings::SET
                        && members.iter().all(|m| matches!(m, Term::Int(_)))
                        && matches!(x, Term::Var(v) if slots.contains_key(v)) =>
                {
                    // `@contains($w, {"dolt_", "dolt log"}) = 1` once lowered (the patterns are symbols): one matcher
                    // of the symbols' names, made with the spec.
                    let members: Vec<String> = members.iter().map(|m| tok(m, &mut slots)).collect::<Option<_>>()?;
                    lines.push(format!("testset {f} {} {want} {}", tok(x, &mut slots)?, members.join(" ")));
                }
                Literal::Cmp { l: Term::Call(f, args), op: "=", r: Term::Int(want @ (0 | 1)) }
                    if crate::code_rule_dsl::logic::strings::is_test(f) && args.len() == 2 =>
                {
                    // `@prefix($s, p) = 1` (or `= 0`): a text test of a string no text relation gives (`strings::as_test`
                    // takes the others to the vocabulary), its string and pattern bound; a set's patterns as one string.
                    let pattern = match crate::code_rule_dsl::logic::strings::patterns(&args[1]) {
                        Some(ps) => Term::Str(ps.join(&crate::code_rule_dsl::logic::strings::SET_SEP.to_string())),
                        // A computed pattern (`@suffix($s, @concat(".", $x))`): made first, into its own slot.
                        None if matches!(&args[1], Term::Call(..)) => expr(&args[1], idx, &mut slots, &mut lines)?,
                        None => args[1].clone(),
                    };
                    // A constant string is a symbol (`Int`) once lowered; a computed string (`@lower($s)`) is made first.
                    let tested = expr(&args[0], idx, &mut slots, &mut lines)?;
                    let bound = |t: &Term| match t {
                        Term::Var(v) => slots.contains_key(v),
                        Term::Str(_) | Term::Int(_) => true,
                        _ => false,
                    };
                    if !bound(&tested) || !bound(&pattern) {
                        return None;
                    }
                    let (ts, tp) = (tok(&tested, &mut slots)?, tok(&pattern, &mut slots)?);
                    lines.push(format!("test {f} {ts} {tp} {want}"));
                }
                Literal::Cmp { l, op, r } if (*op == "=" || *op == "!=") && !matches!(r, Term::Call(f, _) if f == "cat") => {
                    // `$v = @f(…)` (a text or number function, `@concat`, nested calls): the call's value straight into
                    // `$v`, bound or tested; any other `=` or `!=`, of variables, constants or computed terms, as a test
                    // (or `$v = <bound term>`, binding `$v`). Calls on either side are made first, inner ones first.
                    let unbound = |t: &Term, slots: &HashMap<String, usize>| matches!(t, Term::Var(v) if !slots.contains_key(v));
                    let (l, r) = if matches!(r, Term::Var(_)) && matches!(l, Term::Call(..)) { (r, l) } else { (l, r) };
                    if *op == "="
                        && let (Term::Var(_), Term::Call(f, args)) = (l, r)
                    {
                        let name = call_name(f, args.len())?;
                        let made: Vec<Term> = args.iter().map(|a| expr(a, idx, &mut slots, &mut lines)).collect::<Option<_>>()?;
                        let toks: Vec<String> = made.iter().map(|a| tok(a, &mut slots)).collect::<Option<_>>()?;
                        lines.push(format!("func {name} {} {}", tok(l, &mut slots)?, toks.join(" ")));
                    } else {
                        // The bound side goes right.
                        let (a, b) = if unbound(r, &slots) { (r, l) } else { (l, r) };
                        if unbound(b, &slots) || (*op == "!=" && unbound(a, &slots)) {
                            return None;
                        }
                        let b = expr(b, idx, &mut slots, &mut lines)?;
                        let a = if unbound(a, &slots) { a.clone() } else { expr(a, idx, &mut slots, &mut lines)? };
                        let (ta, tb) = (tok(&a, &mut slots)?, tok(&b, &mut slots)?);
                        lines.push(format!("{} {ta} {tb}", if *op == "=" { "eq" } else { "ne" }));
                    }
                }
                Literal::Cmp { l, op: "=", r: Term::Call(f, args) } if f == "cat" => {
                    if !matches!(l, Term::Var(_)) {
                        return None;
                    }
                    let (a, b) = (&args[0], &args[1]);
                    let (tl, ta) = (tok(l, &mut slots)?, tok(a, &mut slots)?);
                    lines.push(format!("cat {tl} {ta} {}", tok(b, &mut slots)?));
                }
                Literal::Cmp { l, op, r } if matches!(*op, "<" | "<=" | ">" | ">=") => {
                    // Integers (and nodes) only: symbols are numbered in the order they are met. A number function's
                    // value (`@strlen($s) > 1`, `@add($i, $j) <= 2`) is made first.
                    let bad = |t: &Term| {
                        if matches!(t, Term::Call(..)) {
                            return !numeric_call(t);
                        }
                        matches!(t, Term::Var(v) if !slots.contains_key(v)) || matches!(typ(t).as_deref(), None | Some("sym"))
                    };
                    if bad(l) || bad(r) {
                        return None;
                    }
                    let (l, r) = (expr(l, idx, &mut slots, &mut lines)?, expr(r, idx, &mut slots, &mut lines)?);
                    let (a, b) = if matches!(*op, "<" | "<=") { (&l, &r) } else { (&r, &l) };
                    let ta = tok(a, &mut slots)?;
                    lines.push(format!("{} {ta} {}", if matches!(*op, "<" | ">") { "lt" } else { "le" }, tok(b, &mut slots)?));
                }
                Literal::Agg { var, op: op @ (AggOp::Count | AggOp::Sum | AggOp::Min | AggOp::Max), over, atom: a } => {
                    // The over-variable belongs to the bag even if an outer step bound the same name.
                    let local=a.args.iter().enumerate().filter_map(|(c,t)|matches!(t,Term::Var(v) if Some(v)==over.as_ref()).then_some(c)).collect::<Vec<_>>();
                    let mut a = a.clone();
                    let col = over.as_ref().and_then(|o| a.args.iter().position(|t| matches!(t, Term::Var(v) if v==o)));
                    if *op != AggOp::Count && col.is_none() { return None; }
                    if let Some(c) = col {
                        let typ = prog_types(prog, &a.rel).get(c).copied();
                        if *op == AggOp::Sum && typ == Some("sym") { return None; }
                        // Text extrema compare text unless this program holds symbols as atoms on both paths.
                        if *op != AggOp::Count && self.model.inputs.atoms.is_none() && typ == Some("sym") { return None; }
                    }
                    for term in &mut a.args {
                        if matches!(term, Term::Var(v) if Some(&*v)==over.as_ref()) { *term=Term::Wild; }
                    }
                    if a.args.iter().any(|t| matches!(t, Term::Var(v) if !slots.contains_key(v))) { return None; }
                    // Input constant filters are projected out by `atom`; its value column uses that projection too.
                    let value_col = col.map(|c| if scc.contains(&a.rel) || native_rels.contains(&a.rel) {c} else {
                        a.args[..c].iter().filter(|t| !matches!(t, Term::Int(_) | Term::Str(_))).count()
                    });
                    let (sc, mut toks) = atom(&a, None, &mut slots)?;
                    if local.len()>1 {
                        for c in local {
                            let c=if scc.contains(&a.rel)||native_rels.contains(&a.rel) {c} else {a.args[..c].iter().filter(|t|!matches!(t,Term::Int(_)|Term::Str(_))).count()};
                            toks[c]="a0".into();
                        }
                    }
                    let name=match op {AggOp::Count=>"count",AggOp::Sum=>"sum",AggOp::Min=>"min",_=>"max"};
                    let value=if *op==AggOp::Count {String::new()} else {format!(" {}",value_col?)};
                    lines.push(format!("{name} {sc} {}{value} {}",toks.join(" "),tok(&Term::Var(var.clone()),&mut slots)?));
                }
                Literal::Agg { var, op: AggOp::Join { by, sep }, over: Some(o), atom: a } => {
                    // `$s = join $x by ($k…) sep "…" : { atom }`: x and the unbound k are the atom's own columns, one
                    // each; every other variable is bound. A bound k is the same for every row, so it orders nothing.
                    // Symbols order by their names natively, as SQL orders their text.
                    let column = |v: &str| a.args.iter().position(|t| matches!(t, Term::Var(x) if x == v));
                    let uses = |v: &str| a.args.iter().filter(|t| matches!(t, Term::Var(x) if x == v)).count();
                    let local: Vec<&String> = std::iter::once(o).chain(by.iter().filter(|b| !slots.contains_key(*b))).collect();
                    if slots.contains_key(o) || local.iter().any(|v| uses(v) != 1) {
                        return None;
                    }
                    if a.args.iter().any(|t| matches!(t, Term::Var(v) if !slots.contains_key(v) && !local.contains(&v))) {
                        return None;
                    }
                    let sym = |c: usize| u8::from(prog_types(prog, &a.rel).get(c).copied() == Some("sym"));
                    // An input is read without its constant columns (`atom`): a column's place among those read.
                    let read = |c: usize| {
                        if scc.contains(&a.rel) || native_rels.contains(&a.rel) {
                            c
                        } else {
                            c - a.args[..c].iter().filter(|t| matches!(t, Term::Int(_) | Term::Str(_))).count()
                        }
                    };
                    let xc = column(o)?;
                    let mut bycols = Vec::new();
                    for b in by.iter().filter(|b| !slots.contains_key(*b)) {
                        let c = column(b)?;
                        bycols.push(format!("{}:{}", read(c), sym(c)));
                    }
                    let mut a = a.clone();
                    for v in &local {
                        let c = column(v)?;
                        a.args[c] = Term::Wild;
                    }
                    let (sc, toks) = atom(&a, None, &mut slots)?;
                    let sep = const_tok(&Term::Str(sep.clone()))?;
                    let bylist = if bycols.is_empty() { "-".to_owned() } else { bycols.join(",") };
                    lines.push(format!("strjoin {sc} {} {}:{} {bylist} {sep} {}", toks.join(" "), read(xc), sym(xc), tok(&Term::Var(var.clone()), &mut slots)?));
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
                // `q(…, @concat(m, ".", a))`, `q(…, @add($i, 1))`: the computed term made into a slot before the head
                // reads it.
                Term::Call(..) => {
                    let made = expr(t, order.len(), &mut slots, &mut lines)?;
                    head.push(tok(&made, &mut slots)?);
                }
                _ => head.push(tok(t, &mut slots)?),
            }
        }
        lines.push(format!("head {}", head.join(" ")));
        Some(lines.join("\n"))
    }

    /// T's `native_stratum`: plans with the stratum as the component being solved when a rule reads it.
    /// The join specs of a stratum's seed rules whose every atom reads a relation already run natively (decision
    /// 0100 §5): what a level's batch can pair before the stratum runs. Planned as the stratum will plan them
    /// (`plan_rule_reused` keeps the plan); a spec the stratum ends up planning otherwise is not found, and that seed
    /// runs by the host's walk.
    fn seed_specs(&mut self, scc: &BTreeSet<String>, rules: &[&Rule]) -> Vec<String> {
        let prog = self.prog;
        if scc.iter().any(|r| prog.lattice.contains_key(r) || prog.natives.contains_key(r) || prog.gates.iter().any(|(g, _)| g == r)) {
            return Vec::new();
        }
        let ri: BTreeMap<String, usize> = scc.iter().enumerate().map(|(i, r)| (r.clone(), i)).collect();
        let mut out = Vec::new();
        for r in rules.iter().filter(|r| recursive(&r.body, scc).is_empty()) {
            if !r.body.iter().filter_map(Literal::atom).all(|a| self.native_rels.contains(&a.rel)) {
                continue;
            }
            let Ok(((order, _, _), _)) = self.model.plan_rule_reused(prog, &r.head, &r.body, None) else { continue };
            if let Some(spec) = self.whole_join_spec(&r.head, &order, None, &ri, scc, &self.native_rels) {
                out.push(spec);
            }
        }
        out
    }

    fn native_stratum(&mut self, scc: &BTreeSet<String>, rules: &[&Rule]) -> Result<bool> {
        let reads = rules.iter().any(|r| !recursive(&r.body, scc).is_empty());
        self.model.stats.solve(if reads { scc.clone() } else { BTreeSet::new() });
        // The relations run natively so far, lent to the stratum (it reads them, and adds its own only at the end):
        // cloning them per stratum was quadratic in strata (3% of a combined run's main thread).
        let native_rels = std::mem::take(&mut self.native_rels);
        // Lent too, so the stratum may call `&mut self` methods while its spans name rules by number.
        let numbers = std::mem::take(&mut self.numbers);
        let out = self.native_stratum_(scc, rules, &native_rels, &numbers);
        self.numbers = numbers;
        let added = std::mem::replace(&mut self.native_rels, native_rels);
        self.native_rels.extend(added);
        self.model.stats.solve(BTreeSet::new());
        out
    }

    /// One stratum on the native set engine, as a component of seed joins (rules without a recursive atom, run once)
    /// and join probes (one per recursive atom, planned with its delta first), all planned by `plan_rule` and
    /// replanned from live counts between chunks of waves (T's `native_stratum_`). Lower strata it reads natively
    /// come from the native store (`r:`), others from their tables or the facts. Nothing is written back: the
    /// relations join `native_rels`. False (and nothing run) where a rule has a literal with no native step.
    fn native_stratum_(&mut self, scc: &BTreeSet<String>, rules: &[&Rule], native_rels: &BTreeSet<String>, numbers: &rustc_hash::FxHashMap<*const Rule, usize>) -> Result<bool> {
        let prog = self.prog;
        self.why_not = None;
        self.why_class = None;
        if scc.iter().any(|r| prog.lattice.contains_key(r)) {
            self.why_not = Some("a lattice relation: the native set engine holds sets only".into());
            self.why_class = Some("a lattice relation".into());
            return Ok(false);
        }
        if scc.iter().any(|r| prog.natives.contains_key(r)) {
            self.why_not = Some("a relation made natively from its inputs' tables (`dfs_first`)".into());
            self.why_class = Some("read of a dfs_first relation".into());
            return Ok(false);
        }
        let rels: Vec<&String> = scc.iter().collect();
        let ri: BTreeMap<String, usize> = rels.iter().enumerate().map(|(i, r)| ((*r).clone(), i)).collect();
        let name = format!("prog_{}", rels[0]);
        let mut gates: Vec<&(String, crate::code_rule_dsl::logic::magic::Gate)> = prog.gates.iter().filter(|(g, _)| scc.contains(g)).collect();
        gates.sort_by(|a, b| (a.1.level, &a.0).cmp(&(b.1.level, &b.0)));
        // A gate's rule only placed it here.
        let rules: Vec<&Rule> = rules.iter().copied().filter(|r| !gates.iter().any(|(g, _)| *g == r.head.rel)).collect();
        // A maintained stratum's survivors (`h :- h__old, !h__del`, decision 0077): the relation starts as the kept
        // `h__old` with `h__del`'s rows dead in place (decision 0090 step 4), not derived row by row.
        // `RERAIL_DRED_COPY`: derive them by the rule (to measure what starting in place saves).
        let based: BTreeMap<String, (String, String)> = if std::env::var_os("RERAIL_DRED_COPY").is_some() {
            BTreeMap::new()
        } else {
            rules.iter().filter_map(|r| survivors(r, scc, native_rels).map(|od| (r.head.rel.clone(), od))).collect()
        };
        let rules: Vec<&Rule> = rules.into_iter().filter(|r| !(based.contains_key(&r.head.rel) && survivors(r, scc, native_rels).is_some())).collect();
        // A rule that joins a relation of a lower stratum holding no rows derives nothing, so it is neither planned
        // nor given to the engine. In a maintained run most delta rules read a delta that is empty this time, and
        // planning, keying and specifying them was work in the program's size, not the delta's (0090 step 5,
        // 0094 §4). Only a positive atom counts: a negation of it holds, and an aggregate over no rows is a row.
        // A relation a request may fill during the stratum is never taken as empty. `RERAIL_KEEP_EMPTY_JOINS` runs
        // them all (to measure what leaving them out saves).
        let rules: Vec<&Rule> = if std::env::var_os("RERAIL_KEEP_EMPTY_JOINS").is_some() { rules } else {
            let fillable: BTreeSet<&str> = super::answers::produced(prog).into_iter().flat_map(|(p, r)| [p, r]).collect();
            let mut empty: HashMap<String, bool> = HashMap::new();
            let mut is_empty = |rel: &str| -> bool {
                // An input delta read from the changed documents (`input_delta`) is bound once, before the run; other
                // inputs may be bound when their first reader runs, so they are not taken as empty.
                let delta_input = self.inputs.contains(rel) && (rel.ends_with("__minus") || rel.ends_with("__plus"));
                if scc.contains(rel) || fillable.contains(rel) || !(native_rels.contains(rel) || delta_input) {
                    return false;
                }
                if let Some(&e) = empty.get(rel) {
                    return e;
                }
                let e = if delta_input {
                    crate::code_rule_dsl::facts::vtab::bound_source(self.db, rel).is_some_and(|s| s.len() == 0)
                } else {
                    super::native::component::held(self.db, rel).is_some_and(|h| h.rows() == 0)
                };
                empty.insert(rel.to_owned(), e);
                e
            };
            let before = rules.len();
            let kept: Vec<&Rule> = rules.into_iter().filter(|r| !r.body.iter().any(|l| matches!(l, Literal::Atom(a) if !a.neg && is_empty(&a.rel)))).collect();
            if before > kept.len() {
                super::profile::decision("rules over empty relations", || format!("{} of {before} rules not run: each joins a relation of a lower stratum that has no rows", before - kept.len()));
            }
            kept
        };
        let typed: HashMap<String, String> = rels.iter().map(|r| ((*r).clone(), prog_types(prog, r).iter().map(|t| sym_char(&self.model.inputs, t)).collect::<String>())).collect();
        let types = |r: &str| typed[r].clone();
        // Nothing left to run (every rule joined an empty relation, and no relation of the stratum has facts): the
        // stratum's relations are held with no rows, and no component is specified or opened. A relation with facts
        // (`d("a"). d(y) :- d(x), e(x, y).` with `e` empty) holds its facts, which the component seeds.
        // `RERAIL_KEEP_EMPTY_JOINS` opens it as before.
        let asks = scc.iter().any(|r| prog.answers.contains_key(r) || super::answers::produced(prog).contains_key(r.as_str()));
        let facts = scc.iter().any(|r| prog.facts.get(r).is_some_and(|rows| !rows.is_empty()));
        if rules.is_empty() && based.is_empty() && gates.is_empty() && !asks && !facts && std::env::var_os("RERAIL_KEEP_EMPTY_JOINS").is_none() {
            for r in &rels {
                super::native::component::hold_empty(self.db, r, types(r).into_bytes());
            }
            super::profile::stratum(&stratum_key(scc), "native set engine", || "every rule joins an empty relation: held empty, no component opened".to_owned());
            self.native_rels.extend(scc.iter().cloned());
            return Ok(true);
        }
        // The component's spec: its relations, and each rule planned and written as a native join (`spec`'s own CPU
        // is the joins' specs and their checks; planning is in its `plan #…` spans).
        let spec_span = super::profile::span(|| "spec".into());
        let mut lines = vec![format!("name {name}")];
        // The relations' lines go here, once the seeds SQL derives are known (below).
        let rel_at = lines.len();
        lines.push("nowrite".to_owned());
        // A component that answers requests (decision 0065 §2a) cannot fall back to SQL whole: a rule with no native
        // step is a SQL seed, run before the component opens, or a SQL probe over its relations' `set_rel` views.
        let answering = scc.iter().any(|r| prog.answers.contains_key(r));
        let views: HashMap<String, (String, String)> = rels.iter().map(|r| ((*r).clone(), (format!("nsx_f_{r}"), format!("nsx_d_{r}")))).collect();
        let mut sql_seeded: BTreeSet<String> = BTreeSet::new();
        let mut sql_probe: Vec<bool> = Vec::new();
        // Requests and what answers them (decision 0065 §2a): the run pauses for the first, and adds to the second.
        let produced = super::answers::produced(prog);
        for r in &rels {
            if prog.answers.contains_key(*r) && scc.iter().any(|x| produced.get(x.as_str()) == Some(&r.as_str())) {
                lines.push(format!("request {}", ri[*r]));
            }
            if produced.contains_key(r.as_str()) {
                lines.push(format!("produced {}", ri[*r]));
            }
        }
        let mut est: HashMap<&str, f64> = HashMap::new();
        for (h, (old, _)) in &based {
            let n = super::native::component::held(self.db, old).map_or(0, |o| o.rows());
            *est.entry(h.as_str()).or_default() += n as f64;
        }
        let mut orders = Vec::new();
        // Each order's rule, for the fanouts its runs observe (`obs_rule`).
        let mut order_rules: Vec<u64> = Vec::new();
        let seeds: Vec<&Rule> = rules.iter().copied().filter(|r| recursive(&r.body, scc).is_empty()).collect();
        // A rule's planning, as a span named by its number and body size (`plan #0009 violation, 15 literals`).
        let planning = |r: &Rule, what: &str| {
            super::profile::span(|| {
                let no = numbers.get(&(r as *const Rule)).map_or("?".to_owned(), |k| format!("{k:04}"));
                format!("{what} #{no} {}, {} literals", r.head.rel, r.body.len())
            })
        };
        let profiling = super::profile::on();
        let rule_no = |r: &Rule| numbers.get(&(r as *const Rule)).copied();
        // Per seed, then per probe: (kind, rule, plan as EXPLAIN shows it, planning CPU, replans).
        let mut explained: Vec<(String, Option<usize>, Option<super::profile::RulePlan>, f64, u64)> = Vec::new();
        // Its shape and planned work, for where it runs (`placement`, decision 0095).
        let mut shape = placement::Shape::default();
        // The first step of `order` without a native step (EXPLAIN's reason the engine declined).
        let missing = |this: &Self, h: &Atom, order: &[Step], at: Option<usize>| -> String {
            let k = (1..=order.len()).find(|&k| this.join_spec(h, &order[..k], at.filter(|&a| a < k), &ri, scc, &native_rels).is_none()).unwrap_or(order.len());
            let s = &order[k.saturating_sub(1)];
            format!("{}{} has no native step", s.lit, if s.mode == Some(Mode::Range) { " (a range read inside a document)" } else { "" })
        };
        // The same step's kind (`decline_class`).
        let missing_class = |this: &Self, h: &Atom, order: &[Step], at: Option<usize>| -> String {
            let k = (1..=order.len()).find(|&k| this.join_spec(h, &order[..k], at.filter(|&a| a < k), &ri, scc, &native_rels).is_none()).unwrap_or(order.len());
            let s = &order[k.saturating_sub(1)];
            if s.mode == Some(Mode::Range) { "a range read inside a document".to_owned() } else { decline_class(&s.lit, prog, native_rels) }
        };
        for r in &seeds {
            let span = planning(r, "plan");
            let t0 = if profiling { super::profile::now() } else { 0.0 };
            let ((order, work, rows), x) = self.model.plan_rule_reused(prog, &r.head, &r.body, None)?;
            let took = if profiling { super::profile::now() - t0 } else { 0.0 };
            drop(span);
            shape.add(&order, &r.head.args, work, false);
            if let Some(x) = &x {
                shape.add_joins(&order, x, |rel| self.model.stats.card.get(rel).copied().unwrap_or(0.0));
            }
            let Some(spec) = self.whole_join_spec(&r.head, &order, None, &ri, scc, &native_rels) else {
                if answering {
                    let reads: Vec<String> = r.body.iter().filter_map(|l| l.atom()).map(|a| a.rel.clone()).filter(|a| !scc.contains(a)).collect();
                    self.materialize(&reads)?;
                    self.ensure_indexes(&order)?;
                    let select = self.model.sql_select(prog, &r.head, &order, None, None)?;
                    self.insert_rows(&r.head, &select)?;
                    sql_seeded.insert(r.head.rel.clone());
                    continue;
                }
                self.why_not = Some(format!("rule #{:04}: {}", rule_no(r).unwrap_or(0), missing(self, &r.head, &order, None)));
                self.why_class = Some(missing_class(self, &r.head, &order, None));
                return Ok(false);
            };
            explained.push((format!("seed {}", r.head.rel), rule_no(r), x, took, 0));
            *est.entry(r.head.rel.as_str()).or_default() += rows;
            lines.push(format!("seed {} {}", ri[&r.head.rel], hex(&spec)));
            orders.push(order);
            order_rules.push(obs_rule(&r.head, &r.body));
        }
        // A stratum's relations were made empty by `run` unless the program states facts of them or a SQL seed derived
        // some. A produced input (decision 0065 §2a) starts from its rows where it is.
        let produced_inputs: BTreeSet<&str> = super::answers::produced(prog).into_keys().filter(|p| self.inputs.contains(p)).collect();
        let mut rel_lines = Vec::new();
        for r in &rels {
            if produced_inputs.contains(r.as_str()) {
                let cols = (0..prog.decls[*r].len()).map(|c| self.model.col(r, c)).collect::<Vec<_>>().join(", ");
                let sql = format!("SELECT {cols} FROM {}", self.model.relation(prog, r, false).render());
                rel_lines.push(format!("rel {r} {} from {}", types(r), hex(&sql)));
            } else if let Some((old, gone)) = based.get(*r) {
                // Its survivors rule is gone: it starts from the kept relation, plus what facts or SQL seeds wrote.
                let seeded = !(prog.facts.get(*r).is_none_or(|f| f.is_empty()) && !sql_seeded.contains(*r));
                rel_lines.push(format!("rel {r} {} base {} {}{}", types(r), hex(old), hex(gone), if seeded { " seeded" } else { "" }));
            } else {
                let empty = prog.facts.get(*r).is_none_or(|f| f.is_empty()) && !sql_seeded.contains(*r);
                rel_lines.push(format!("rel {r} {}{}", types(r), if empty { " empty" } else { "" }));
            }
        }
        lines.splice(rel_at..rel_at, rel_lines);
        let nseeds = orders.len();
        for r in &rels {
            let seeded = est.get(r.as_str()).copied().unwrap_or(0.0).max(1.0);
            let card = prior_stats(&mut self.model, prog, r).map_or(seeded, |n| n.max(seeded));
            self.model.stats.card.insert((*r).clone(), card);
        }
        let mut probes = Vec::new();
        for r in &rules {
            // The rule's whole plan, and its spec once one is wanted: one per rule, whichever atom is its delta.
            let mut whole_plan: Option<Vec<Step>> = None;
            let mut whole_spec: Option<Option<String>> = None;
            for (ai, a) in recursive(&r.body, scc) {
                let _span = planning(r, "plan");
                let t0 = if profiling { super::profile::now() } else { 0.0 };
                let ((order, work, _), x) = self.model.plan_rule_reused(prog, &r.head, &r.body, Some(ai))?;
                let took = if profiling { super::profile::now() - t0 } else { 0.0 };
                shape.add(&order, &r.head.args, work, true);
                if let Some(x) = &x {
                    shape.add_joins(&order, x, |rel| self.model.stats.card.get(rel).copied().unwrap_or(0.0));
                }
                let at = order.iter().position(|s| s.at == ai).expect("the delta atom");
                let Some(spec) = self.whole_join_spec(&r.head, &order, Some(at), &ri, scc, &native_rels) else {
                    if answering {
                        let reads: Vec<String> = r.body.iter().filter_map(|l| l.atom()).map(|a| a.rel.clone()).filter(|a| !scc.contains(a)).collect();
                        self.materialize(&reads)?;
                        // The component's own relations are read through its views; only the others take indexes.
                        let outside: Vec<_> = order.iter().filter(|st| st.lit.atom().is_none_or(|b| !scc.contains(&b.rel))).cloned().collect();
                        self.ensure_indexes(&outside)?;
                        let sql = self.model.sql_select(prog, &r.head, &order, Some(at), Some(&views))?;
                        explained.push((format!("probe {} (delta {})", r.head.rel, a.rel), rule_no(r), None, took, 0));
                        lines.push(format!("probe {} {} {}", ri[&r.head.rel], ri[&a.rel], hex(&sql)));
                        probes.push((*r, ai, order));
                        sql_probe.push(true);
                        continue;
                    }
                    self.why_not = Some(format!("rule #{:04}: {}", rule_no(r).unwrap_or(0), missing(self, &r.head, &order, Some(at))));
                    self.why_class = Some(missing_class(self, &r.head, &order, Some(at)));
                    return Ok(false);
                };
                let x = x.map(|x| delta_sources(x, &order, at, scc));
                explained.push((format!("join {} (delta {})", r.head.rel, a.rel), rule_no(r), x, took, 0));
                // The wave whose delta is all of the relation (it was empty before): any order, the atom read in full.
                if whole_plan.is_none() {
                    whole_plan = Some(self.model.plan_rule_reused(prog, &r.head, &r.body, None)?.0.0);
                }
                let whole = whole_plan.as_ref().expect("planned");
                let wspec = if whole[0].at != ai {
                    whole_spec.get_or_insert_with(|| self.whole_join_spec(&r.head, whole, None, &ri, scc, &native_rels)).clone()
                } else {
                    None
                };
                let mut line = format!("join {} {} {}", ri[&r.head.rel], ri[&a.rel], hex(&spec));
                if let Some(w) = wspec {
                    line += &format!(" {}", hex(&w));
                }
                lines.push(line);
                probes.push((*r, ai, order));
                sql_probe.push(false);
            }
        }
        for (g, d) in &gates {
            let idx = |rs: &[String]| {
                let s = rs.iter().filter_map(|r| ri.get(r)).map(usize::to_string).collect::<Vec<_>>().join(",");
                if s.is_empty() { "-".to_owned() } else { s }
            };
            let deps = d.deps.iter().filter_map(|x| gates.iter().position(|(g2, _)| g2 == x)).map(|i| i.to_string()).collect::<Vec<_>>().join(",");
            if let Some(src) = d.src.as_ref().filter(|s| !ri.contains_key(*s)) {
                return Err(Error::Plan(format!("gate {g}: its source {src} is not in its component {scc:?}")));
            }
            lines.push(format!(
                "gate {} {} {} {}",
                ri[g],
                d.src.as_ref().map_or("-".to_owned(), |s| ri[s].to_string()),
                idx(&d.quiet),
                if deps.is_empty() { "-".to_owned() } else { deps }
            ));
        }
        drop(spec_span);
        // The O125 spike (`RERAIL_SPIKE_SPECS=<dir>`): each stratum named as it starts, its spec written out.
        if let Some(dir) = std::env::var_os("RERAIL_SPIKE_SPECS") {
            let cpu = { let (u, s) = corpus::thread_usage(); u + s };
            eprintln!("O125 stratum {name} ({} relations, {} rules) at {:.1} s CPU", rels.len(), rules.len(), cpu);
            let _ = std::fs::write(std::path::Path::new(&dir).join(format!("{name}.spec")), format!("{}\n\n{}", rules.iter().map(|r| r.to_string()).collect::<Vec<_>>().join("\n"), lines.join("\n")));
        }
        let started = std::time::Instant::now();
        // Where it runs (decision 0095). The device has no relational form yet, so every stratum runs here; the
        // placement is recorded with the CPU it took, for the profile's "Placement" section.
        let placed = placement::place(&shape);
        let cpu_at = if profiling { let (u, s) = corpus::thread_usage(); u + s } else { 0.0 };
        let mut counted: Vec<Vec<(u64, u64)>> = Vec::new();
        // The relational lab (decision 0095, step 0): the stratum's sources and seeds kept to run it again alone.
        let lab = super::native::component::lab::wanted(&name, shape.work)
            .and_then(|path| super::native::component::lab::capture_inputs(self.db, &name, &lines.join("\n"), &path));
        // Per probe (spec order), its CPU seconds, while profiling: the run's per-probe `ms:rows:cpu_ms` field.
        let mut timed: Vec<f64> = Vec::new();
        self.fx().open(&lines.join("\n"))?;
        let sql_views = sql_probe.iter().any(|p| *p);
        if sql_views {
            for r in &rels {
                let (full, delta) = &views[*r];
                for (which, vt) in [("full", full), ("delta", delta)] {
                    self.db.execute(&format!("CREATE VIRTUAL TABLE temp.{vt} USING set_rel({name}, {}, {which})", ri[*r]), [])?;
                }
            }
        }
        // One wave first: the seeds' rows give live statistics before the next wave, the first in which most
        // relations' deltas are all their rows and the probes run their whole plans.
        // RDL_WAVE_TRACE: the component's planned joins, in probe order, before it runs (a stalled probe is named).
        if std::env::var_os("RDL_WAVE_TRACE").is_some() {
            for (k, o) in orders.iter().chain(probes.iter().map(|p| &p.2)).enumerate() {
                eprintln!("    {name} probe {k}: {}", shown(o).join(" , "));
            }
        }
        order_rules.extend(probes.iter().map(|p| obs_rule(&p.0.head, &p.0.body)));
        let mut chunk: i64 = 1;
        // Per probe, the sizes its last live replan read (`live_sizes`).
        let mut replanned_at: Vec<Option<Vec<f64>>> = vec![None; probes.len()];
        let keeping_plans = PLANS.with(|p| p.borrow().is_some());
        loop {
            let f: String = self.fx().run(&name, chunk as u32)?;
            let f: Vec<&str> = f.split_whitespace().collect();
            if let Some(per) = f.get(7).filter(|_| f.len() == 9 && super::profile::on()) {
                for (k, item) in per.split(',').enumerate() {
                    if timed.len() <= k {
                        timed.resize(k + 1, 0.0);
                    }
                    timed[k] += item.rsplit(':').next().and_then(|c| c.parse::<f64>().ok()).unwrap_or(0.0) / 1e3;
                }
            }
            if f.last() == Some(&"2") {
                answer_pending(self.db, self.model.inputs.atoms.is_some(), &mut self.answer, &mut self.answered, &name, scc)?;
                continue;
            }
            let steps = self.fx().steps(&name)?;
            for ((o, &rule), line) in orders.iter().chain(probes.iter().map(|p| &p.2)).zip(&order_rules).zip(&steps) {
                observe(&mut self.model.stats, rule, o, line);
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
            if chunk == 1 {
                // The whole plans, planned from seed estimates (a derived relation empty, its columns without
                // distinct counts), again from the live counts: once per rule, the same plan for each of its probes
                // (`setfix` runs it once a wave).
                let mut whole: Vec<(*const Rule, String)> = Vec::new();
                for k in 0..probes.len() {
                    if sql_probe[k] {
                        continue;
                    }
                    let r = probes[k].0;
                    let spec = match whole.iter().find(|(p, _)| std::ptr::eq(*p, r)) {
                        Some((_, w)) => w.clone(),
                        None => {
                            let _span = planning(r, "replan whole");
                            let ((new, _, _), _) = self.model.plan_rule_live(prog, &r.head, &r.body, None)?;
                            let Some(w) = self.whole_join_spec(&r.head, &new, None, &ri, scc, &native_rels) else { continue };
                            let w = hex(&w);
                            whole.push((r, w.clone()));
                            w
                        }
                    };
                    self.fx().reprobe(&name, nseeds + k, "whole", &spec)?;
                }
                chunk = 2;
                continue;
            }
            for k in 0..probes.len() {
                if sql_probe[k] {
                    continue;
                }
                let (r, ai) = (probes[k].0, probes[k].1);
                // No size it reads moved since its last replan: `plan_rule_live` would give the plan it gave then
                // (kept with those sizes, while plans are kept), which the probe runs.
                if keeping_plans {
                    let sizes = self.model.live_sizes(&r.body, Some(ai));
                    if replanned_at[k].as_ref() == Some(&sizes) {
                        continue;
                    }
                    replanned_at[k] = Some(sizes);
                }
                let span = planning(r, "replan");
                let t0 = if profiling { super::profile::now() } else { 0.0 };
                let ((new, _, _), x) = self.model.plan_rule_live(prog, &r.head, &r.body, Some(ai))?;
                drop(span);
                let e = &mut explained[nseeds + k];
                e.3 += if profiling { super::profile::now() - t0 } else { 0.0 };
                if same_order(&new, &probes[k].2) {
                    continue;
                }
                e.4 += 1;
                let at = new.iter().position(|s| s.at == ai).expect("the delta atom");
                e.2 = x.map(|x| delta_sources(x, &new, at, scc));
                let at = new.iter().position(|s| s.at == ai).expect("the delta atom");
                let spec = self
                    .whole_join_spec(&r.head, &new, Some(at), &ri, scc, &native_rels)
                    .ok_or_else(|| Error::Plan(format!("a replanned probe of {} has no native step", r.head)))?;
                self.fx().reprobe(&name, nseeds + k, "join", &hex(&spec))?;
                probes[k].2 = new;
            }
            chunk *= 2;
        }
        if profiling {
            let (u, s) = corpus::thread_usage();
            let work: u64 = counted.iter().flatten().map(|(a, _)| *a).sum();
            super::profile::placed(&stratum_key(scc), &placed, &shape, work, u + s - cpu_at);
        }
        if super::profile::on() {
            let empty = Vec::new();
            let kinds = seeds.iter().map(|r| format!("seed {}", r.head)).chain(probes.iter().map(|(r, ai, _)| format!("join {} (delta at {ai})", r.head)));
            for ((k, o), kind) in orders.iter().chain(probes.iter().map(|p| &p.2)).enumerate().zip(kinds) {
                super::profile::native(format!("{kind}: {}", shown(o).join(" , ")), counted.get(k).unwrap_or(&empty), timed.get(k).copied().unwrap_or(0.0));
            }
            // After the native plans: they are charged to this stratum's `engine run (joins)` by the current span.
            let _record = super::profile::span(|| "profile record".into());
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
        if sql_views {
            for r in &rels {
                let (full, delta) = &views[*r];
                self.db.execute_batch(&format!("DROP TABLE temp.{full}; DROP TABLE temp.{delta};"))?;
            }
        }
        {
            let _close = super::profile::span(|| "engine close".into());
            self.fx().close(&name)?;
        }
        if let Some(capture) = lab {
            super::native::component::lab::capture_outputs(self.db, capture);
        }
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
            self.db.execute(&format!("INSERT INTO {} {select}", crate::code_rule_dsl::logic::sql::ident(&head.rel)), [])?;
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
            self.db.execute_batch(&format!("DROP TABLE {q}; CREATE VIRTUAL TABLE temp.{q} USING lattice_result({rel});", q = crate::code_rule_dsl::logic::sql::ident(rel)))?;
            self.views.insert(rel.clone());
        } else if native.contains("direct") {
            // Written straight into the relation: no output table to copy back.
            self.db.query_row(&format!("SELECT lattice_fixpoint('lfx_{rel}', '{rel}')"), [], |_| Ok(()))?;
        } else {
            self.db.query_row(&format!("SELECT lattice_fixpoint('lfx_{rel}')"), [], |_| Ok(()))?;
            self.db.execute(&format!("INSERT INTO {} SELECT c0, c1 FROM lfx_{rel}_out", crate::code_rule_dsl::logic::sql::ident(rel)), [])?;
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
            let card = prior_stats(&mut self.model, prog, r).map_or(rows.max(1.0), |n| n.max(rows.max(1.0)));
            self.model.stats.card.insert((*r).clone(), card);
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
                        // A rest sharing no variable (a constant test) has no edge table to keep.
                        if key.is_empty() && ecols.is_empty() {
                            return Ok(Err("rest binds no variable".to_owned()));
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
            self.db.execute_batch(&format!("DROP TABLE IF EXISTS {0}; CREATE TEMP TABLE {0}({cols});", crate::code_rule_dsl::logic::sql::ident(&ehead.rel)))?;
            if !ehead.args.is_empty() {
                let select = self.model.sql_select(prog, ehead, order, None, None)?;
                self.db.execute(&format!("INSERT INTO {} {select}", crate::code_rule_dsl::logic::sql::ident(&ehead.rel)), [])?;
            }
        }
        let join = self.model.inputs.native.contains("join");
        let none = BTreeSet::new();
        for (r, order, at, ai) in &probes {
            let rel = match &r.body[*ai] {
                Literal::Atom(a) => a.rel.clone(),
                _ => unreachable!(),
            };
            if join && let Some(spec) = self.whole_join_spec(&r.head, order, Some(*at), &ri, scc, &none) {
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
                if same_order(&new, &probes[k].1) {
                    continue;
                }
                let nat = new.iter().position(|s| s.at == ai).expect("the delta atom");
                let spec = if join { self.whole_join_spec(&r.head, &new, Some(nat), &ri, scc, &none) } else { None };
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
    run(prog, &db, Inputs::file(), None, Hooks::default())
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
    run(prog, &db, Inputs { native: native_modes(modes), ..Inputs::file() }, None, Hooks::default())
}

/// Evaluates a lowered program (decision 0009) over the base relations bound on `db` (`facts::vtab::syntax::bind`),
/// with the planner's statistics in the attached schema `stats`. `db` is a connection to the content store: the
/// symbols the run makes are the store's atoms (pending until a flush, decision 0049), and derived relations are
/// `temp` tables, dropped by the next run. Outputs are decoded: names and kinds as text, node ids as the vtabs'.
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
    let out = run(&lowered.program, db, inputs, Some(lowered), Hooks::default());
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
    run_native_inputs(lowered, db, inputs, Hooks::default())
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
    run_native_inputs(lowered, db, inputs, Hooks::default())
}

/// What a run tells its caller as it goes, so neither holds every relation from the run's start to its end.
#[derive(Default)]
pub(in crate::code_rule_dsl) struct Hooks<'a> {
    /// A kept relation no later stratum reads: the caller writes it where it goes, and the run drops it
    /// (`Outcome::handed`).
    pub(in crate::code_rule_dsl) hand_off: Option<&'a mut dyn FnMut(&str) -> Result<()>>,
    /// Inputs a stratum is about to read for the first time: the caller binds their rows now.
    pub(in crate::code_rule_dsl) load: Option<&'a mut dyn FnMut(&[String]) -> Result<()>>,
    /// Inputs no later stratum reads: the caller may drop their rows (the run has dropped its views of them).
    pub(in crate::code_rule_dsl) unload: Option<&'a mut dyn FnMut(&[String]) -> Result<()>>,
    /// Answers the requests a component makes, inside the fixpoint (decision 0065 §2a).
    pub(in crate::code_rule_dsl) answer: Option<Answer<'a>>,
}

/// `run_atoms_native_over`, telling `hooks` of each kept relation and input as its last reader runs, and of each
/// input before its first.
pub(in crate::code_rule_dsl) fn run_atoms_native_hooked(
    lowered: &lower::Lowered,
    db: &Connection,
    facts: &str,
    given: Stats,
    kept: BTreeSet<String>,
    modes: &str,
    hooks: Hooks<'_>,
) -> Result<Outcome> {
    let mut native = native_modes(modes);
    native.remove("nearest");
    native.remove("kseed");
    let inputs = Inputs { facts: format!("{facts}."), stats: "s.".to_owned(), atoms: Some(lowered.ops), native, batch: HashMap::new(), given: Some(given), kept };
    run_native_inputs(lowered, db, inputs, hooks)
}

/// `run_atoms_native_over`, with requests answered by `answer` inside the fixpoint (decision 0065 §2a).
pub(in crate::code_rule_dsl) fn run_atoms_native_answering(
    lowered: &lower::Lowered,
    db: &Connection,
    facts: &str,
    given: Stats,
    kept: BTreeSet<String>,
    modes: &str,
    answer: Answer<'_>,
) -> Result<Outcome> {
    let mut native = native_modes(modes);
    native.remove("nearest");
    native.remove("kseed");
    let inputs = Inputs { facts: format!("{facts}."), stats: "s.".to_owned(), atoms: Some(lowered.ops), native, batch: HashMap::new(), given: Some(given), kept };
    run_native_inputs(lowered, db, inputs, Hooks { answer: Some(answer), ..Default::default() })
}

fn run_native_inputs(lowered: &lower::Lowered, db: &Connection, inputs: Inputs, hooks: Hooks<'_>) -> Result<Outcome> {
    db.execute_batch("PRAGMA temp_store = MEMORY;")?;
    super::native::register(db)?;
    super::native::register_symbols(db)?;
    super::native::forget_inputs(db);
    let out = run(&lowered.program, db, inputs, Some(lowered), hooks);
    // The run's symbols are pending in the store's arena; they are written when a flush is due or with the next
    // write transaction (decision 0049).
    let flush = super::profile::span(|| "flush symbols".into());
    super::native::flush_symbols(db)?;
    drop(flush);
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
    format!("CREATE TEMP VIEW {} AS {}", crate::code_rule_dsl::logic::sql::ident(rel), query.render())
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
        // A projection is a set. Omit DISTINCT only when the head retains an input key;
        // otherwise alternate input supports must not multiply aggregate counts.
        let preserves=|t:&Term|match t{Term::Var(v)=>h.args.iter().any(|x|matches!(x,Term::Var(x) if x==v)),Term::Wild=>false,_=>true};
        let key=match a.rel.as_str(){"node" if a.args.len()==5=>Some(1),"doc"|"tape"=>Some(0),_=>None};
        query.body.distinct=!(key.is_some_and(|k|a.args.get(k).is_some_and(preserves))||a.args.iter().all(preserves));
        views.push((rel.to_owned(), query));
        model.view_stats(rel, h, a);
        let src = first_columns(a);
        let cols = h.args.iter().map(|t| if let Term::Var(v) = t { src.get(v.as_str()).map(|&c| model.col(&a.rel, c)) } else { None }).collect();
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
/// holds, else 0; `@nfkc(s)`, `s` under Unicode NFKC, as CPython normalizes an identifier (PEP 3131: the parser's
/// `unicodedata.normalize("NFKC", …)` of each non-ASCII one; `identifier::nfkc`); `@py_repr(s)`, CPython's `repr(s)` of
/// the str `s` (`cpython::value::str_repr`: W's `repr_unparse` is `@py_repr(@py_unparse(n))`). NULL gives NULL
/// (`rr_ed25519_ok`: 0). `rr_join(sep, x, k, …)` is `join`'s aggregate: the distinct
/// `(k, …, x)` rows' `x` ordered by `k, …` then `x` (numbers by value, then text by code point, as `sorted()` orders a
/// list of one type), joined by `sep`; `""` over no rows.
pub(in crate::code_rule_dsl) fn register_functors(db: &Connection) -> rusqlite::Result<()> {
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
    db.create_scalar_function(c"rr_nfkc", 1, flags, |ctx| Ok(ctx.get::<Option<String>>(0)?.map(|s| crate::code_rule_dsl::identifier::nfkc(&s).into_owned())))?;
    db.create_scalar_function(c"rr_py_repr", 1, flags, |ctx| {
        Ok(ctx.get::<Option<String>>(0)?.map(|s| crate::code_rule_dsl::cpython::value::str_repr(&crate::code_rule_dsl::cpython::value::PyStr::Text(s))))
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

fn run(prog: &Program, db: &Connection, inputs: Inputs, lowered: Option<&lower::Lowered>, mut hooks: Hooks<'_>) -> Result<Outcome> {
    let (normalized,sentinels)=sql_kernel::normalize(prog)?;
    let prog=&normalized;
    let adjusted=lowered.map(|l|{
        let mut domains=l.domains.clone();
        for rel in &sentinels {if let Some(columns)=domains.get_mut(rel){while columns.len()<prog.decls[rel].len(){columns.insert(0,lower::Domain::Int);}}}
        lower::Lowered{program:prog.clone(),domains,ops:l.ops,dialect:l.dialect}
    });
    let lowered=adjusted.as_ref();
    let setup = super::profile::span(|| "setup".into());
    super::strings::register(db)?;
    super::native::aggregation::register_sum(db)?;
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
            db.execute_batch(&format!("DROP TABLE IF EXISTS temp.{}", crate::code_rule_dsl::logic::sql::ident(&name)))?;
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
    let mut model = {
        let _span = super::profile::span(|| "statistics".into());
        make_plan(prog, db, inputs)?
    };
    let demand_span = super::profile::span(|| "demand".into());
    // The literals a whole-program transformation reads, charged to it (here the demand split, then the strata).
    if super::profile::on() {
        super::profile::work(super::profile::Unit::Literal, prog.rules.iter().map(|r| 1 + r.body.len() as u64).sum::<u64>());
    }
    // Relations computed for every node that rules only negate, computed for the keys those rules reach (decision
    // 0048); in a corpus run only (kept plans), so T's golden plans are unchanged.
    let split;
    let corpus = PLANS.with(|p| p.borrow().is_some());
    // What each derived relation keeps (decision 0099): stored, or inlined into its readers, from this plan's
    // estimates; before demand, which then sees the readers as inlining left them. Corpus runs only, as demand.
    let inlined;
    let prog: &Program = match if corpus { keeps::choose(prog, &mut model).map_err(Error::Plan)? } else { None } {
        Some((p, done)) => {
            super::profile::decision("inline or store (decision 0099)", || format!("{} relations recomputed in their readers: {}", done.len(), done.join(", ")));
            inlined = p;
            // A measurement aid: `RERAIL_INLINE_DUMP=<file>` appends the rules as inlining left them.
            if let Some(path) = std::env::var_os("RERAIL_INLINE_DUMP") {
                use std::io::Write;
                if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                    for r in &inlined.rules {
                        let _ = writeln!(f, "{r}");
                    }
                }
            }
            model.analysis = std::sync::Arc::new(super::analysis::ProgramAnalysis::analyze(&inlined)).into();
            &inlined
        }
        None => prog,
    };
    let prog: &Program = match if corpus { demand::split_negations(prog, &model) } else { None } {
        Some((p, demanded)) => {
            super::profile::decision("demand (decision 0048)", || {
                format!("{} relations computed only for the keys their readers reach: {}", demanded.len(), demanded.join(", "))
            });
            split = p;
            // The program run is the split one: its analysis is that program's, made when asked.
            model.analysis = std::sync::OnceLock::new();
            &split
        }
        None => prog,
    };
    drop(demand_span);
    let strata_span = super::profile::span(|| "strata".into());
    if super::profile::on() {
        super::profile::work(super::profile::Unit::Literal, prog.rules.iter().map(|r| 1 + r.body.len() as u64).sum::<u64>());
    }
    let (sccs, _) = strata(prog)?;
    let mut rules_by: IndexMap<&str, Vec<&Rule>> = IndexMap::new();
    for r in &prog.rules {
        rules_by.entry(r.head.rel.as_str()).or_default().push(r);
    }
    drop(strata_span);
    drop(planning);
    let tables = super::profile::span(|| "views and tables".into());
    let native = model.inputs.native.clone();
    // Inputs kept in the engine from an earlier run (a session's, decision 0090 step 3) are read in place, as native
    // relations, with the indexes they have.
    let adopted: BTreeSet<String> = super::native::component::adopted(db).into_iter().filter(|r| prog.inputs.contains(r)).collect();
    let mut run = Run { prog, db, model, views: HashSet::new(), made: HashSet::new(), orders: HashMap::new(), keyless: HashSet::new(), native_rels: adopted, pending: HashMap::new(), why_not: None, projections: projections(prog), inputs: prog.inputs.iter().map(String::as_str).collect(), numbers: prog.rules.iter().enumerate().map(|(k, x)| (x as *const Rule, k)).collect(), answer: hooks.answer.take(), why_class: None, answered: BTreeMap::new() };
    for (rel, view) in plan_views(prog, &mut run.model, &rules_by)? {
        let view = view_sql(&rel, &view);
        // SQLite keeps a temp view's statement without `TEMP`.
        if old_views.remove(&rel).as_deref() != Some(&view.replacen("CREATE TEMP VIEW", "CREATE VIEW", 1)) {
            db.execute_batch(&format!("DROP VIEW IF EXISTS temp.{}", crate::code_rule_dsl::logic::sql::ident(&rel)))?;
            db.execute(&view, [])?;
        }
        run.views.insert(rel);
    }
    for name in old_views.keys() {
        db.execute_batch(&format!("DROP VIEW IF EXISTS temp.{}", crate::code_rule_dsl::logic::sql::ident(&name)))?;
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
                "node" | "int" | "flat" | "min" | "max" => Ok(format!("c{i} INTEGER")),
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
        let create = format!("CREATE TEMP TABLE {}({names}, PRIMARY KEY ({key})) WITHOUT ROWID", crate::code_rule_dsl::logic::sql::ident(rel));
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
        if prog.lattice.contains_key(rel) && !rows.is_empty(){
            let select=rows.iter().map(|row|format!("SELECT {}",row.iter().enumerate().map(|(i,v)|match v {
                SqlValue::Integer(n)=>format!("{n} AS c{i}"),
                SqlValue::Text(s)=>format!("'{}' AS c{i}",s.replace(char::from(39),"''")),
                _=>unreachable!(),
            }).collect::<Vec<_>>().join(","))).collect::<Vec<_>>().join(" UNION ALL ");
            sql_kernel::merge(&run,rel,&select)?;
        }else{insert_many(db, &format!("INSERT OR IGNORE INTO {}", crate::code_rule_dsl::logic::sql::ident(rel)), rows)?;}
    }
    drop(tables);
    let mut outcome = Outcome::default();
    // Each native relation is dropped after the last stratum that reads it (an output, a relation kept for the caller
    // and a gate's relations stay to the end).
    let last = last_reads(prog, &sccs, &run.views);
    let first = first_reads(prog, &sccs, &run.views);
    let mut unloaded: BTreeSet<String> = BTreeSet::new();
    let gated: BTreeSet<String> = prog.gates.iter().flat_map(|(g, d)| [g, &d.rel].into_iter().chain(&d.src).chain(&d.quiet).chain(&d.deps)).cloned().collect();
    let stays: BTreeSet<String> = prog.outputs.iter().chain(&run.model.inputs.kept).chain(&gated).cloned().collect();
    // What stays, and what the views among it read (through views): SQL reads a view's inputs once the run ends.
    let mut pinned: BTreeSet<String> = stays.clone();
    loop {
        let more: Vec<String> = prog
            .rules
            .iter()
            .filter(|r| pinned.contains(&r.head.rel) && run.views.contains(&r.head.rel))
            .flat_map(|r| r.body.iter().filter_map(Literal::atom).map(|a| a.rel.clone()))
            .filter(|r| !pinned.contains(r))
            .collect();
        if more.is_empty() {
            break;
        }
        pinned.extend(more);
    }
    // Kept relations given to the caller (`hand_off`) and dropped once no later stratum reads them.
    let mut handed: BTreeSet<String> = BTreeSet::new();
    let mut unindexed: BTreeSet<String> = BTreeSet::new();
    // Each input by the stratum that binds it (before its first reader) and the one that drops it (after its last),
    // found once rather than by a scan of every input per stratum.
    let mut loads: Vec<Vec<String>> = vec![Vec::new(); sccs.len().max(1)];
    let mut unloads: Vec<Vec<String>> = vec![Vec::new(); sccs.len() + 1];
    for r in &prog.inputs {
        loads[first.get(r).copied().unwrap_or(0).min(sccs.len().max(1) - 1)].push(r.clone());
        if let Some(&l) = last.get(r)
            && !pinned.contains(r)
        {
            unloads[(l + 1).min(sccs.len())].push(r.clone());
        }
    }
    // Each relation by the stratum after its last reader: a native one is released or read out there. A native
    // relation is one adopted before the first stratum or one a stratum made (`native_stratum`), so a stratum looks at
    // those of its bucket and the one before's own unread relations, not every native relation (quadratic in strata).
    let mut after_last: Vec<Vec<String>> = vec![Vec::new(); sccs.len() + 1];
    for (r, &l) in &last {
        after_last[(l + 1).min(sccs.len())].push(r.clone());
    }
    // Device waves (decision 0095 §5, 0098 §6). A stratum's level is one more than the deepest level it reads, so a
    // level's strata read only earlier levels' relations. Each level's count tables that an earlier chunk found worth
    // building (`count table`) are built as one batch when the level's first stratum runs: one submission per lane
    // for the level, not one per stratum. A table whose relation is not held yet is built by its step, as before.
    let level_of: Vec<usize> = {
        let of: HashMap<&str, usize> = sccs.iter().enumerate().flat_map(|(j, scc)| scc.iter().map(move |r| (r.as_str(), j))).collect();
        let by = rules_by_stratum(prog, &sccs);
        let mut level = vec![0usize; sccs.len()];
        for j in 0..sccs.len() {
            let deepest = by[j].iter().flat_map(|r| r.body.iter().filter_map(Literal::atom)).filter_map(|a| of.get(a.rel.as_str())).filter(|&&d| d != j && d < j).map(|&d| level[d] + 1).max();
            level[j] = deepest.unwrap_or(0);
        }
        level
    };
    let wants_of: Vec<BTreeSet<(String, Vec<usize>)>> = rules_by_stratum(prog, &sccs)
        .iter()
        .zip(&sccs)
        .map(|(rules, scc)| {
            rules
                .iter()
                .flat_map(|r| r.body.iter())
                .filter_map(|l| match l {
                    Literal::Agg { op: AggOp::Count, atom, .. } if !scc.contains(&atom.rel) => {
                        Some((atom.rel.clone(), (0..atom.args.len()).filter(|&c| !matches!(atom.args[c], Term::Wild)).collect()))
                    }
                    _ => None,
                })
                .collect()
        })
        .collect();
    let mut waved = vec![false; sccs.len()];
    let by_stratum = rules_by_stratum(prog, &sccs);
    for (at, scc) in sccs.iter().enumerate() {
        let candidates: Vec<String> = if at == 0 {
            run.native_rels.iter().cloned().collect()
        } else {
            let mut c = std::mem::take(&mut after_last[at]);
            c.extend(sccs[at - 1].iter().filter(|r| !last.contains_key(*r)).cloned());
            c.retain(|r| run.native_rels.contains(r));
            c
        };
        let done: Vec<String> = candidates.iter().filter(|r| !stays.contains(*r) && last.get(*r).is_none_or(|&l| l < at)).cloned().collect();
        if !done.is_empty() {
            super::native::component::release(db, &done);
            done.iter().for_each(|r| { run.native_rels.remove(r); });
        }
        // One that stays but that no later stratum reads keeps its rows only; a kept one the caller takes is handed
        // off and dropped.
        let read_out: Vec<String> = candidates.iter().filter(|r| run.native_rels.contains(*r) && stays.contains(*r) && last.get(*r).is_none_or(|&l| l < at) && unindexed.insert((*r).clone())).cloned().collect();
        // Inputs: bound before their first reader, dropped after their last; one no rule reads (an output as it
        // came) is bound before the first stratum.
        if let Some(load) = hooks.load.as_deref_mut() {
            let now: Vec<String> = std::mem::take(&mut loads[at]);
            if !now.is_empty() {
                load(&now)?;
            }
        }
        if let Some(unload) = hooks.unload.as_deref_mut() {
            // An output that is an input, or a view of one, is read once the run ends.
            let gone: Vec<String> = std::mem::take(&mut unloads[at]).into_iter().filter(|r| unloaded.insert(r.clone())).collect();
            if !gone.is_empty() {
                super::native::component::forget_inputs_of(db, &gone);
                unload(&gone)?;
            }
        }
        if let Some(hand) = hooks.hand_off.as_deref_mut() {
            let give: Vec<String> = read_out.iter().filter(|r| run.model.inputs.kept.contains(*r) && !gated.contains(*r)).cloned().collect();
            for r in &give {
                hand(r)?;
            }
            if !give.is_empty() {
                super::native::component::release(db, &give);
                give.iter().for_each(|r| { run.native_rels.remove(r); });
                handed.extend(give);
            }
        }
        // A relation kept for the caller keeps its indexes: a session's next run starts from it (`Rel::based`).
        let read_out: Vec<String> = read_out.into_iter().filter(|r| !run.model.inputs.kept.contains(r)).collect();
        if !read_out.is_empty() {
            super::native::component::unindex(db, &read_out);
        }
        let rules: Vec<&Rule> = scc.iter().flat_map(|rel| rules_by.get(rel.as_str()).into_iter().flatten().copied()).collect();
        let key = scc.iter().cloned().collect::<Vec<_>>().join("/");
        let _stratum = super::profile::span(|| format!("stratum {}", if key.len() > 80 { format!("{}…({} relations)", &key[..80], scc.len()) } else { key.clone() }));
        // A produced relation above its request (decision 0065 §2a): its request is whole, so it is answered once, or
        // holds what a component's pauses answered. An input's rows are the caller's to write.
        let produced = super::answers::produced(prog);
        if scc.iter().all(|r| produced.contains_key(r.as_str())) {
            for rel in scc {
                let rows = match run.answered.remove(rel) {
                    Some(rows) => rows,
                    None => {
                        let req = produced[rel.as_str()].to_owned();
                        run.materialize(std::slice::from_ref(&req))?;
                        let width = prog.decls[&req].len();
                        let cols = (0..width).map(|c| format!("c{c}")).collect::<Vec<_>>().join(", ");
                        let asked: Vec<Vec<i64>> = db
                            .prepare(&format!("SELECT {cols} FROM {}", crate::code_rule_dsl::logic::sql::ident(&req)))?
                            .query_map([], |r| (0..width).map(|c| r.get::<_, i64>(c)).collect::<rusqlite::Result<Vec<_>>>())?
                            .collect::<rusqlite::Result<_>>()?;
                        let answer = run.answer.as_deref_mut().ok_or_else(|| Error::Plan(format!("{req}: a request, and no one to answer it")))?;
                        let mut mine = Vec::new();
                        for (p, got) in answer(&req, &asked)? {
                            if p == *rel { mine.extend(got) } else { run.answered.entry(p).or_default().extend(got) }
                        }
                        mine
                    }
                };
                // An input's rows are the caller's to bind; a relation no rule reads has no table.
                if prog.inputs.contains(rel) || rows.is_empty() || !prog.rules.iter().any(|r| r.body.iter().filter_map(|l| l.atom()).any(|a| a.rel == *rel)) {
                    continue;
                }
                let width = prog.decls[rel].len();
                let marks = vec!["?"; width].join(", ");
                let mut insert = db.prepare(&format!("INSERT OR IGNORE INTO {} VALUES ({marks})", crate::code_rule_dsl::logic::sql::ident(rel)))?;
                for row in &rows {
                    insert.execute(rusqlite::params_from_iter(row.iter()))?;
                }
            }
            continue;
        }
        let rule_no = |r: &Rule| prog.rules.iter().position(|x| std::ptr::eq(x, r)).expect("a rule of the program");
        let skey = stratum_key(scc);
        let mut why = if native.contains("program") { String::new() } else { format!("the native engine is off (modes: {native:?})") };
        if super::profile::on() {
            for r in &rules {
                super::profile::note(format!("rule #{:04}", rule_no(r)), || r.to_string());
            }
        }
        if native.contains("program") && !waved[at] {
            let level: Vec<usize> = (at..sccs.len()).filter(|&j| level_of[j] == level_of[at] && !waved[j]).collect();
            let wants: BTreeSet<(String, Vec<usize>)> = level.iter().flat_map(|&j| wants_of[j].iter().cloned()).collect();
            level.iter().for_each(|&j| waved[j] = true);
            if !wants.is_empty() {
                let _wave = super::profile::span(|| "device wave (count tables)".into());
                let (launches, rels) = super::native::component::prefetch_tables(db, &wants)?;
                if launches > 0 {
                    super::profile::decision("device waves (decision 0095 §5)", || format!("a level of {} strata: {launches} count tables over {rels} relations in one batch", level.len()));
                }
            }
            // The level's first joins (decision 0100 §5, §6): every seed join of its strata that reads only
            // relations already held, sent to the executor as one batch while the level's strata run.
            let native_ones: Vec<usize> = level.iter().copied().filter(|&j| !sccs[j].iter().all(|r| run.views.contains(r))).collect();
            let specs: Vec<String> = native_ones.iter().flat_map(|&j| run.seed_specs(&sccs[j], &by_stratum[j])).collect();
            if !specs.is_empty() {
                let _wave = super::profile::span(|| "level wave (joins)".into());
                let sent = super::native::component::prefetch_joins(db, &specs)?;
                if sent > 0 {
                    super::profile::decision("level joins (decision 0100 §5)", || format!("a level of {} strata: {sent} of {} seed joins sent as one batch", level.len(), specs.len()));
                }
            }
        }
        if native.contains("program") && !scc.iter().all(|r| run.views.contains(r)) {
            let native_span = super::profile::span(|| "native".into());
            let t0 = level_trace().map(|_| thread_cpu());
            let ran = run.native_stratum(scc, &rules)?;
            drop(native_span);
            // `RERAIL_LEVEL_TRACE=<file>`: each native stratum's level and thread CPU (decision 0100 step 6).
            if let (Some(path), Some(t0)) = (level_trace(), t0) {
                use std::io::Write as _;
                if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                    let _ = writeln!(f, "stratum\t{}\t{at}\t{:.6}\t{ran}\t{}", level_of[at], thread_cpu() - t0, rules.len());
                }
            }
            if ran {
                outcome.native.insert(key);
                continue;
            }
            why = format!("the native engine declined: {}", run.why_not.take().unwrap_or_else(|| "no reason given".into()));
            if prog.gates.iter().any(|(g, _)| scc.contains(g)) {
                return Err(Error::Plan(format!("gated negations in {scc:?} need native steps for every rule")));
            }
            if scc.iter().any(|r| prog.answers.contains_key(r)) && scc.iter().any(|r| super::answers::produced(prog).contains_key(r.as_str())) {
                return Err(Error::Plan(format!("requests in {scc:?} are answered only inside the native fixpoint: {why}")));
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
        // A relation made natively (`dfs_first`): its inputs' rows in, the operator's rows out; its rule only
        // ordered it after them.
        if let [rel] = scc.iter().collect::<Vec<_>>().as_slice()
            && let Some(native) = prog.natives.get(*rel)
        {
            super::profile::stratum(&skey, "native operator over its inputs' tables (dfs_first, match_all)", || why.clone());
            let n = run.native_relation(rel, native)?;
            super::profile::note(format!("{rel} rows"), || n.to_string());
            run.refresh(rel, false)?;
            for c in 0..prog.decls[*rel].len() {
                let d = run.count(&format!("SELECT count(DISTINCT c{c}) FROM {}", crate::code_rule_dsl::logic::sql::ident(rel)))?;
                run.model.ndv.insert(((*rel).clone(), c), if d == 0 { 1.0 } else { d as f64 });
            }
            continue;
        }
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
                anchor: order.first().map_or_else(|| "scalar".into(),|s|s.lit.to_string()),
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
        let mut ran_native=false;
        if recursive {
            for r in scc {db.execute(&format!("CREATE TEMP TABLE delta_{r} AS SELECT * FROM {} WHERE 0", crate::code_rule_dsl::logic::sql::ident(r)),[])?;}
            if native.contains("fixpoint") && run.native_fixpoint(scc,&rules)? {ran_native=true;outcome.native.insert(key.clone());}
            else if native.contains("setfix") && run.native_set_fixpoint(scc,&rules)? {ran_native=true;outcome.native.insert(key.clone());}
        }
        if !ran_native {
            let n=sql_kernel::component(&mut run,scc,&rules,&planned,recursive)?;
            super::profile::stratum(&skey,"SQL wave kernel",||format!("{why}; {n} waves"));
            super::profile::fallback(&skey, run.why_class.take().unwrap_or_else(|| if native.contains("program") { "no reason given".into() } else { "the native engine is off".into() }));
            if recursive {outcome.iterations.insert(key,n as usize);}
            for (((h,_),rule),(x,took)) in planned.iter().zip(&rules).zip(&explained){
                if let Some(x)=x{super::profile::planned(&skey,&format!("sql wave {}",h.rel),Some(rule_no(rule)),x,*took,0,None,None);}
            }
        }
        let _refresh = super::profile::span(|| "refresh and distinct counts".into());
        for r in scc {
            run.refresh(r, false)?;
            for c in 0..prog.decls[r].len() {
                let d = run.count(&format!("SELECT count(DISTINCT c{c}) FROM {}", crate::code_rule_dsl::logic::sql::ident(r)))?;
                run.model.ndv.insert((r.clone(), c), if d == 0 { 1.0 } else { d as f64 });
            }
        }
    }
    // The chunk's end: pairs no seed took, and what the lanes kept for the chunk's joins.
    super::native::component::level_clear();
    if let Some(path) = level_trace() {
        use std::io::Write as _;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let (sent, used) = super::native::component::level_counts();
            let _ = writeln!(f, "chunk\t{sent}\t{used}");
        }
    }
    let _outputs = super::profile::span(|| "outputs".into());
    let kept = run.model.inputs.kept.clone();
    outcome.kept = kept.iter().filter(|r| run.native_rels.contains(*r)).cloned().collect();
    let written: Vec<String> = prog.outputs.iter().filter(|r| !outcome.kept.contains(*r) && !handed.contains(*r)).cloned().collect();
    run.materialize(&written)?;
    let (outputs, nodes) = decoded_outputs(prog, db, lowered, &facts, atoms, &kept)?;
    let mut outputs=outputs;let mut nodes=nodes;
    for rel in &sentinels {
        if let Some(rows)=outputs.get_mut(rel){for row in rows { *row=row.split_once('\t').map_or(String::new(),|(_,rest)|rest.into()); }}
        if let Some(cols)=nodes.get_mut(rel){*cols=cols.iter().filter_map(|i|i.checked_sub(1)).collect();}
    }
    outcome.outputs = outputs;
    outcome.nodes = nodes;
    outcome.handed = handed;
    Ok(outcome)
}

/// Per relation, the first stratum whose rules read it; a view is read where its first reader runs.
fn first_reads(prog: &Program, sccs: &[BTreeSet<String>], views: &HashSet<String>) -> HashMap<String, usize> {
    let mut first: HashMap<String, usize> = HashMap::new();
    let by = rules_by_stratum(prog, sccs);
    for (j, scc) in sccs.iter().enumerate().rev() {
        let at = if scc.iter().all(|r| views.contains(r)) { scc.iter().filter_map(|r| first.get(r)).copied().min().unwrap_or(j) } else { j };
        for r in &by[j] {
            for l in &r.body {
                if let Literal::Atom(a) | Literal::Agg { atom: a, .. } = l {
                    let e = first.entry(a.rel.clone()).or_insert(at);
                    *e = (*e).min(at);
                }
            }
        }
    }
    first
}

/// Per relation, the last stratum (an index into `sccs`) whose rules read it. A view is read where its readers run,
/// so what a view reads lives as long as the view's last reader.
/// Each stratum's rules (an index into `sccs`), found once: a scan of every rule per stratum was quadratic.
/// `RERAIL_LEVEL_TRACE`, read once.
fn level_trace() -> Option<&'static str> {
    static PATH: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    PATH.get_or_init(|| std::env::var("RERAIL_LEVEL_TRACE").ok()).as_deref()
}

/// This thread's CPU seconds (wasm32: `prof::single_thread_cpu`).
#[cfg(target_family = "wasm")]
fn thread_cpu() -> f64 {
    crate::code_rule_dsl::prof::single_thread_cpu().as_secs_f64()
}

/// This thread's CPU seconds.
#[cfg(unix)]
fn thread_cpu() -> f64 {
    let mut t = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: a valid timespec out.
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) };
    t.tv_sec as f64 + t.tv_nsec as f64 * 1e-9
}

/// This thread's CPU seconds (user + kernel).
#[cfg(windows)]
fn thread_cpu() -> f64 {
    let (user, kernel) = crate::code_rule_dsl::prof::windows_cpu(true);
    (user + kernel).as_secs_f64()
}

fn rules_by_stratum<'a>(prog: &'a Program, sccs: &[BTreeSet<String>]) -> Vec<Vec<&'a Rule>> {
    let of: HashMap<&str, usize> = sccs.iter().enumerate().flat_map(|(j, scc)| scc.iter().map(move |r| (r.as_str(), j))).collect();
    let mut by: Vec<Vec<&Rule>> = vec![Vec::new(); sccs.len()];
    for r in &prog.rules {
        if let Some(&j) = of.get(r.head.rel.as_str()) {
            by[j].push(r);
        }
    }
    by
}

fn last_reads(prog: &Program, sccs: &[BTreeSet<String>], views: &HashSet<String>) -> HashMap<String, usize> {
    let mut last: HashMap<String, usize> = HashMap::new();
    let by = rules_by_stratum(prog, sccs);
    for (j, scc) in sccs.iter().enumerate().rev() {
        let at = if scc.iter().all(|r| views.contains(r)) { scc.iter().filter_map(|r| last.get(r)).copied().max().unwrap_or(j).max(j) } else { j };
        for r in &by[j] {
            for l in &r.body {
                if let Literal::Atom(a) | Literal::Agg { atom: a, .. } = l {
                    let e = last.entry(a.rel.clone()).or_insert(at);
                    *e = (*e).max(at);
                }
            }
        }
    }
    last
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
            format!("SELECT {} FROM {}", cols.join(", "), crate::code_rule_dsl::logic::sql::ident(rel))
        } else {
            format!("SELECT * FROM {}", crate::code_rule_dsl::logic::sql::ident(rel))
        };
        let name = if rel == "finding" { "finding_at".to_owned() } else { rel.clone() };
        let kinds = lowered.map(|l| l.dialect.kinds());
        // A value names its grammar (its tape tag): a guest's kinds are its grammar's (decision 0058 §2).
        let of_tag = |tag: i64| u8::try_from(tag).ok().and_then(crate::code_rule_dsl::language::from_tape_tag).map(|d| d.kinds());
        let decode = |i: usize, v: i64| -> Option<String> {
            let run = kinds?;
            match domains.get(i)? {
                lower::Domain::Kind => {
                    let atom = u32::try_from(v).ok()?;
                    let (tag, code) = (crate::code_rule_dsl::facts::core::kind_tag(atom), crate::code_rule_dsl::facts::core::kind_code(atom));
                    Some(of_tag(i64::from(tag)).unwrap_or(run).kind_name(code)?.to_owned())
                }
                lower::Domain::Field => Some(of_tag(v >> 16).unwrap_or(run).field_name((v & 0xffff) as u16)?.to_owned()),
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
mod sql_kernel;

#[cfg(test)]
mod tests;
