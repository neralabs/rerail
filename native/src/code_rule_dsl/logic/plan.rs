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

use super::front::Value;
use super::program_image::{AtomRef, CmpOp, Lit, Slot as VarSlot, TermRef, Version};
use body::{Body, RuleKey, VarSet};
use fields::Fields;
use super::native::TOP;
use super::native::component::spec::{Arg, ComponentSpec, EdgeSpec, Extreme, HeadTok, InputKey, JoinItem, JoinSpec, Line as SpecLine, Reprobe, Source as JoinSource, Start, Tok};
use std::sync::Arc;
use indexmap::IndexMap;
use rusqlite::Connection;
use rusqlite::types::Value as SqlValue;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;
use std::path::Path;

/// A planning or evaluation failure.
#[derive(Debug)]
pub enum Error {
    Front(super::front::Error),
    Sql(rusqlite::Error),
    Cache(crate::content_store::StoreError),
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
            Error::Cache(e) => write!(f, "{e}"),
            Error::Statistics(m) | Error::Plan(m) => f.write_str(m),
        }
    }
}

impl From<crate::content_store::StoreError> for Error {
    fn from(error: crate::content_store::StoreError) -> Self {
        Self::Cache(error)
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

pub type Result<T> = std::result::Result<T, Error>;

pub mod corpus;
pub mod placement;
pub mod choices;
pub mod issue;
pub mod demand;
pub mod keeps;
pub mod body;
mod select;
mod join_spec;
pub mod native_demand;
pub mod replanning;
pub mod compose;
pub mod connection;
pub use rerail_facts::code_rule_dsl::logic::plan::sketch;
pub mod statkeep;
pub mod lower;
pub mod text_demand;
pub mod joins;
pub mod table;
pub mod output;
pub mod shapes;
pub mod prepared;

/// Where a run reads its inputs and statistics, and how it holds symbols.
#[derive(Clone, Debug)]
pub struct Inputs {
    /// The schema prefix of the input relations: T's attached facts file (`f.`) or the bound vtabs (`main.`).
    pub facts: String,
    /// The schema prefix of the `stat_*` tables.
    pub stats: String,
    /// Symbols are atoms: the name atoms of `flat_bin`'s operators `+`, `-` and `*`. `None`: symbols are text.
    pub atoms: Option<[i64; 3]>,
    /// T's `RDL_NATIVE` modes (`native_modes`); empty is T's pure SQL mode.
    pub native: BTreeSet<String>,
    /// Batch inputs: relations a wave reads from its chunk's `demand(batch)`, which have no
    /// statistics outside a batch. Each is read from its source and planned from its row estimate.
    pub batch: HashMap<String, (Source, f64)>,
    /// The planner's statistics, measured by the caller from the inputs in memory (`corpus::run`): nothing is read
    /// or counted at plan time.
    pub given: Option<Stats>,
    /// Outputs left in the native engine (`DERIVED`) for the caller: neither written to their tables nor decoded.
    pub kept: BTreeSet<String>,
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

/// A string from its hex (Python's `bytes.fromhex(v).decode()`).
fn unhex(s: &str) -> Option<String> {
    let bytes = (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect::<Option<Vec<u8>>>()?;
    String::from_utf8(bytes).ok()
}

impl Inputs {
    /// T's facts file attached as `f`, statistics in it.
    pub fn file() -> Self {
        Inputs { facts: "f.".to_owned(), stats: "f.".to_owned(), atoms: None, native: BTreeSet::new(), batch: HashMap::new(), given: None, kept: BTreeSet::new() }
    }
}

/// A constant of a rule or of the statistics (`stat_val.val`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Constant {
    Int(i64),
    Str(String),
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
pub enum Mode {
    Range,
    Climb,
    Descend,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::Range => "range",
            Mode::Climb => "climb",
            Mode::Descend => "descend",
        }
    }
}

/// A planned body literal: its position in the body it was planned for (T compared literals by identity), and its
/// mode. A step names its literal and holds none (decision 0173): it is read beside its body, by `Order`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    pub at: usize,
    pub mode: Option<Mode>,
}

/// A plan read beside the rule it orders (`Body`, by handles): each step with its literal. Every reader of a plan's
/// literals reads them here (`view`), so none indexes a body itself.
#[derive(Clone, Copy)]
pub struct Order<'a, 'v> {
    pub b: &'a Body<'v>,
    pub steps: &'a [Step],
}

impl<'a, 'v> Order<'a, 'v> {
    pub fn new(b: &'a Body<'v>, steps: &'a [Step]) -> Order<'a, 'v> {
        Order { b, steps }
    }
    /// Step `s`'s literal.
    pub fn view(&self, s: &Step) -> Lit<'v> {
        self.b.view(s.at)
    }
    /// The literals in step order.
    pub fn lits(&self) -> impl Iterator<Item = Lit<'v>> + '_ {
        self.steps.iter().map(|s| self.view(s))
    }
    pub fn iter(&self) -> std::slice::Iter<'a, Step> {
        self.steps.iter()
    }
    pub fn len(&self) -> usize {
        self.steps.len()
    }
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
    /// The plan of its first `n` steps.
    pub fn take(&self, n: usize) -> Order<'a, 'v> {
        Order { b: self.b, steps: &self.steps[..n.min(self.steps.len())] }
    }
}

/// A plan's rows after each of its steps, every row walked (`StepPlan::all`): what the device sizes its stages' tables
/// by before they have run (`JoinSpec::est`).
fn step_estimates(x: &super::profile::RulePlan) -> super::native::setfix::spec::Est {
    super::native::setfix::spec::Est(x.steps.iter().map(|s| s.all).collect())
}

/// Tells each kind-set axis a planned order reads by its `n` how many keys come into that step (the rows the planner
/// expects after the step before): the axis chooses between its batch's solve and lookups per node (decision 0143).
fn expect_axis_lookups(db: impl crate::code_rule_dsl::facts::vtab::Scoped, plan: &super::profile::RulePlan) {
    let mut into = 1.0;
    for step in &plan.steps {
        let rel = step.literal.split('(').next().unwrap_or("").trim();
        if super::front::is_transduced(rel) && (step.access.starts_with("lookup on (n") || step.access.starts_with("membership on (n")) {
            if let Some(source) = crate::code_rule_dsl::facts::vtab::bound_source(&db, rel) {
                source.expect_lookups(0, into);
            }
        }
        into = step.rows;
    }
}

/// An atom's binding pattern: its relation, and which columns are bound, constants by value (T's `atom_sig`).
pub type Sig = (String, Vec<(usize, Option<Constant>)>);

/// The positive atoms of a body: a body of one has one order.
fn positive_atoms(b: &Body<'_>) -> usize {
    (0..b.len()).filter(|&i| matches!(b.view(i), Lit::Atom(a) if !a.neg())).count()
}

/// Bindings a step must have seen before its observed fanout replaces the estimate.
const OBS_MIN: i64 = 32;

/// The corpus statistics the planner prices plans with, read from the tables `stats.py` builds at ingestion
/// A kept relation's statistics, made the first time a run plans with them (decision 0101 §4): its columns'
/// (name, [rows, ndv, sum of squared postings, the same within documents]) and the constants' postings, with what
/// the caller keeps of them (`T`: their form patched by delta, decision 0096 step 4). A session's run binds every
/// kept relation as `r__old`, but most strata it reaches are quiet or join an empty delta, and never plan with it.
pub struct LazyStats<T> {
    made: std::sync::OnceLock<(Vec<(String, [i64; 4])>, Vec<(String, i64, i64)>, T)>,
    make: std::sync::Mutex<Option<LazyMake<T>>>,
}
type LazyMake<T> = Box<dyn FnOnce() -> (Vec<(String, [i64; 4])>, Vec<(String, i64, i64)>, T) + Send>;

impl<T> LazyStats<T> {
    pub fn new(make: impl FnOnce() -> (Vec<(String, [i64; 4])>, Vec<(String, i64, i64)>, T) + Send + 'static) -> std::sync::Arc<Self> {
        std::sync::Arc::new(LazyStats { made: std::sync::OnceLock::new(), make: std::sync::Mutex::new(Some(Box::new(make))) })
    }
    /// Its statistics, made now if they are not yet.
    fn get(&self) -> &(Vec<(String, [i64; 4])>, Vec<(String, i64, i64)>, T) {
        self.made.get_or_init(|| (self.make.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take().expect("made once"))())
    }
    /// Its statistics if a run made them.
    pub fn made(&self) -> Option<&(Vec<(String, [i64; 4])>, Vec<(String, i64, i64)>, T)> {
        self.made.get()
    }
}

/// What `Stats` holds of a relation's lazy statistics: the caller's part is not the planner's.
pub trait Deferred: Send + Sync {
    fn statistics(&self) -> (Vec<(String, [i64; 4])>, Vec<(String, i64, i64)>);
}
impl<T: Send + Sync> Deferred for LazyStats<T> {
    fn statistics(&self) -> (Vec<(String, [i64; 4])>, Vec<(String, i64, i64)>) {
        let (cols, vals, _) = self.get();
        (cols.clone(), vals.clone())
    }
}

/// A kept relation a maintained run binds where a stratum it reaches first reads it (decision 0117 step 4): until
/// then the planner knows its rows' count only, each column taken as a key.
pub struct Unbound {
    pub rel: String,
    pub width: usize,
    pub rows: usize,
}
impl Deferred for Unbound {
    fn statistics(&self) -> (Vec<(String, [i64; 4])>, Vec<(String, i64, i64)>) {
        let n = self.rows as i64;
        ((0..self.width).map(|c| (sql_col(&self.rel, c), [n, n.max(1), n, n])).collect(), Vec::new())
    }
}

/// `Stats::lazy`'s entries, shown by name only.
#[derive(Clone, Default)]
pub struct LazyMap(HashMap<String, std::sync::Arc<dyn Deferred>>);
impl std::fmt::Debug for LazyMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_set().entries(self.0.keys()).finish()
    }
}

/// Per-column statistics, by relation then column: a relation's are found, and a strand's model copies those it
/// reads (`Model::fork`), without a pass over every relation's (decision 0117 step 3: a pass a fork, 3,700 forks, was
/// 5.4 s of an edit).
#[derive(Clone, Debug, Default)]
pub struct ColStats(HashMap<String, HashMap<String, [i64; 4]>>);

impl ColStats {
    pub fn get(&self, (rel, col): &(String, String)) -> Option<&[i64; 4]> {
        self.0.get(rel)?.get(col)
    }
    pub fn insert(&mut self, (rel, col): (String, String), v: [i64; 4]) -> Option<[i64; 4]> {
        self.0.entry(rel).or_default().insert(col, v)
    }
    /// Whether any column of `rel` has statistics.
    pub fn has(&self, rel: &str) -> bool {
        self.0.get(rel).is_some_and(|c| !c.is_empty())
    }
    /// `rel`'s columns' statistics.
    pub fn of(&self, rel: &str) -> Option<&HashMap<String, [i64; 4]>> {
        self.0.get(rel)
    }
    /// Every (relation, column) and its statistics.
    pub fn iter(&self) -> impl Iterator<Item = ((&String, &String), &[i64; 4])> {
        self.0.iter().flat_map(|(r, cs)| cs.iter().map(move |(c, v)| ((r, c), v)))
    }
}

/// The per-column statistics: one definition, `table::ColMap` (decisions 0209 §3 and 0210 §9).
pub use table::ColMap;

impl FromIterator<((String, String), [i64; 4])> for ColStats {
    fn from_iter<I: IntoIterator<Item = ((String, String), [i64; 4])>>(it: I) -> Self {
        let mut c = ColStats::default();
        for (k, v) in it {
            c.insert(k, v);
        }
        c
    }
}

/// (`stat_*` in the facts database) in one statement. Nothing is counted at plan time (ast-index-spike S174, S178).
/// Statistics whose fingerprint (row counts) does not match the facts are refused.
/// The climbs `Stats::steps` priced, by kind and set: each a function of the tree's statistics alone, so a model's
/// plans made on several threads (`keeps::estimate`) share them.
#[derive(Debug, Default)]
struct StepsCache(parking_lot::Mutex<HashMap<(Option<String>, String), f64>>);

impl Clone for StepsCache {
    fn clone(&self) -> Self {
        StepsCache(parking_lot::Mutex::new(self.0.lock().clone()))
    }
}

/// The tree's fielded edges, counted by (parent kind, field) and (child kind, field), and each field's edges: the
/// share of a field's parents (or children) of a kind, and a kind's nodes' children by a field.
#[derive(Clone, Debug, Default)]
pub struct Fielded {
    parent: HashMap<(String, String), f64>,
    child: HashMap<(String, String), f64>,
    field: HashMap<String, f64>,
    /// By (parent kind, child kind, field).
    joint: HashMap<(String, String, String), f64>,
}

impl Fielded {
    /// The share of field `f`'s edges whose parent is of kind `k`: a node bound as a parent by `field(n, f, _)` is
    /// of kind `k` that often.
    pub fn parent_share(&self, k: &str, f: &str) -> Option<f64> {
        let all = *self.field.get(f)?;
        (all > 0.0).then(|| self.parent.get(&(k.to_owned(), f.to_owned())).copied().unwrap_or(0.0) / all)
    }
    /// The share of the edges by `f` from a `pk` parent to a `ck` child: among those whose child is `ck` when
    /// `of_child` (how often such a child's parent is `pk`), else among those whose parent is `pk`.
    pub fn joint_share(&self, pk: &str, ck: &str, f: &str, of_child: bool) -> Option<f64> {
        let all = if of_child { self.child.get(&(ck.to_owned(), f.to_owned())) } else { self.parent.get(&(pk.to_owned(), f.to_owned())) }.copied()?;
        (all > 0.0).then(|| self.joint.get(&(pk.to_owned(), ck.to_owned(), f.to_owned())).copied().unwrap_or(0.0) / all)
    }
    /// The share of field `f`'s edges whose child is of kind `k`.
    pub fn child_share(&self, k: &str, f: &str) -> Option<f64> {
        let all = *self.field.get(f)?;
        (all > 0.0).then(|| self.child.get(&(k.to_owned(), f.to_owned())).copied().unwrap_or(0.0) / all)
    }
    /// The children by field `f` of the nodes of kind `k`, all told (`parent`), or the nodes of kind `k` that are a
    /// child by `f` (`child`).
    pub fn edges(&self, k: &str, f: &str, parent: bool) -> Option<f64> {
        self.field.contains_key(f).then(|| if parent { &self.parent } else { &self.child }.get(&(k.to_owned(), f.to_owned())).copied().unwrap_or(0.0))
    }
    fn is_empty(&self) -> bool {
        self.field.is_empty()
    }
    fn of(edges: impl IntoIterator<Item = (String, String, String, f64)>) -> Fielded {
        let mut f = Fielded::default();
        for (p, c, field, n) in edges {
            *f.joint.entry((p.clone(), c.clone(), field.clone())).or_default() += n;
            *f.parent.entry((p, field.clone())).or_default() += n;
            *f.child.entry((c, field.clone())).or_default() += n;
            *f.field.entry(field).or_default() += n;
        }
        f
    }
}

#[derive(Clone, Debug)]
pub struct Stats {
    /// (table, column) → (rows, ndv, sum of squared postings, the same within documents).
    pub col: ColStats,
    // The tree's statistics (`tree`), and those below to `card`, shared by a component's model (`Model::fork`).
    pub kfreq: std::sync::Arc<IndexMap<String, i64>>,
    pub size: std::sync::Arc<HashMap<String, f64>>,
    pub nodes: f64,
    parents: std::sync::Arc<IndexMap<String, IndexMap<String, f64>>>,
    pub card: table::RelMap<f64>,
    /// Current wave cardinalities, distinct from full relation cardinalities.
    pub delta_rows: table::RelMap<f64>,
    /// (rel, col) → posting-weighted fanout of an equality probe: sq / rows.
    pub fan: ColMap<f64>,
    /// (derived rel, col) → {value: postings}, for a column of few values (native statistics).
    pub dfreq: ColMap<HashMap<Constant, i64>>,
    /// (rel, col) → its heavy values (decision 0209 §4), made for the columns a line raised (`Run::heavy_for`).
    /// Shared, not copied, by a component's model.
    pub heavy: ColMap<std::sync::Arc<table::HeavyCol>>,
    /// rel → the engine's relation as its heavy values were made or last merged over (`InRel::mark`): its change
    /// since is merged into them, not counted again (O172).
    pub heavy_mark: table::RelMap<std::sync::Arc<super::native::component::RelMark>>,
    /// (rule, binding pattern) → [bindings in, bindings out]: fanouts native joins observed, per rule (`obs_rule`).
    /// By rule first: a component's model takes its rules' alone (`Model::fork`).
    pub obs: HashMap<u64, HashMap<Sig, [i64; 2]>>,
    /// The recursive component being planned (`atom_rows`).
    pub solving: BTreeSet<String>,
    /// Members of `solving` that had rows when its component opened (a maintained stratum's survivors, rows SQL
    /// wrote): a first scan of one reads its live rows, which its card holds, not the corpus (`atom_rows`; `sized`).
    pub sized: BTreeSet<String>,
    /// `solving`'s digest, for plan keys (`plan_key`): set with it (`solve`), not made again at every lookup.
    solving_digest: u128,
    /// Input rel → index column lists.
    indexes: std::sync::Arc<HashMap<String, Vec<Vec<String>>>>,
    /// Documents (node-id intervals, `stat_docrange`), if the facts have them.
    docs: Option<i64>,
    /// Exact postings of the program's constants: table → (column, value) → n. By table first: a relation's are
    /// read without a pass over every table's (`of_relation`, once per input of a session's run).
    pub memo: HashMap<String, HashMap<(String, Constant), i64>>,
    /// The relations whose constants' postings in `memo` are estimates: counted over a sample of one chunk of a mix
    /// and scaled to the others (`corpus::Own`). A constant missing from them is unknown, not absent: the chunk
    /// measured may lack a value every other chunk holds. Priced as absent (one row), `#f65561` (a comprehension's
    /// field) led `pre_115`'s plan in every chunk of a cold PostHog run: 14.5 M rows into three anti-joins, and 47
    /// slow-plan checks measuring exact pair statistics that the kept plan never used.
    pub estimated: BTreeSet<String>,
    steps_cache: StepsCache,
    kindsets: std::sync::Arc<BTreeMap<String, Vec<String>>>,
    /// (relation, column) → a lookup's CPU as a multiple of a derived relation's, where its source says it is not
    /// one (`Source::lookup_cost`, decision 0312): placement prices the step by it.
    pub lookup: std::sync::Arc<BTreeMap<(String, String), f64>>,
    /// The tree's fielded edges by kind (`Own::fields`, `stat_edge`'s field): Q44's correlation of a node's kind with
    /// the fields it has or is.
    pub fields: std::sync::Arc<Fielded>,
    /// The program's relations' layouts (decision 0361 step 4), by name, shared by a component's model: estimates
    /// where they answer a column, and no statistics for a relation they answer whole.
    pub shapes: std::sync::Arc<shapes::Shapes>,
    /// The version's plans made from its layouts (decision 0361 step 5), by line, shared by a component's model: a
    /// line they hold is taken as kept, not searched or priced.
    pub prepared: std::sync::Arc<prepared::Prepared>,
    /// Relations whose statistics are made when first planned with (`lazy`, `Model::resolve`).
    lazy: LazyMap,
    /// Quiet strata's relations, whose columns take their `__old`'s statistics when first planned with
    /// (`prior_stats`, `Model::resolve`): their rows are known without them.
    prior_later: BTreeSet<String>,
}

impl Stats {
    /// The recursive component being planned (`solving`), and its digest.
    pub fn solve(&mut self, scc: BTreeSet<String>) {
        self.solving_digest = super::derived::digest(|d| {
            d.u64(scc.len() as u64);
            scc.iter().for_each(|r| d.str(r));
        });
        self.solving = scc;
        self.sized.clear();
    }

    /// `rels` of `solving` had rows when its component opened, measured (`live_stats`): the plans made from here
    /// price a first scan of one by its rows. A part of the plans' keys, with `solving`.
    pub fn sized(&mut self, rels: impl IntoIterator<Item = String>) {
        self.sized.extend(rels);
        self.solving_digest = super::derived::digest(|d| {
            d.u64(self.solving.len() as u64);
            self.solving.iter().for_each(|r| d.str(r));
            d.u64(self.sized.len() as u64);
            self.sized.iter().for_each(|r| d.str(r));
        });
    }

    /// What the planner knows of `rel` (`RERAIL_DUMP_STATS`, a debugging aid): each column's (rows, distinct, sum of
    /// squared postings, within documents), its cardinality, and its few-valued columns' value counts.
    pub fn describe(&self, rel: &str) -> String {
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
    fn new(db: &Connection, p: &body::Placed, inputs: &mut Inputs) -> Result<Self> {
        if let Some(given) = inputs.given.take() {
            return Ok(given);
        }
        Self::read(db, p, inputs, true)
    }

    /// The statistics in schema `stats` (T's, atomized) for the constants of `p` (a whole version), read once; no
    /// fingerprint (the caller then sets what it measures, `measured`).
    pub fn of_schema(db: &Connection, p: &body::Placed, stats: &str) -> Result<Self> {
        let inputs = Inputs { facts: "main.".to_owned(), stats: format!("{stats}."), atoms: Some([0; 3]), native: BTreeSet::new(), batch: HashMap::new(), given: None, kept: BTreeSet::new() };
        Self::read(db, p, &inputs, false)
    }

    /// No statistics but the program's kindsets: the caller measures every input (`measured`, `tree`), as the
    /// typing programs' own ingestion does (`corpus::run` without T's facts, list step 29).
    /// The mean subtree size of the nodes that have children (by their kinds' mean sizes: `size`, `kfreq`): the
    /// descendants a container of unknown kind has (decision 0299). Weighted by size, module roots set it, and an
    /// annotation's few nodes were priced as a module's (`annotation($n) :- ann_root($t), within($n, $t)` then
    /// scanned all of `within`: 37 M rows for 0.28 M); unweighted, leaves set it.
    pub fn subtree_fan(&self) -> Option<f64> {
        let (mut nodes, mut sizes) = (0.0f64, 0.0f64);
        for (k, &size) in self.size.iter() {
            if size >= 2.0 {
                let n = self.kfreq.get(k).copied().unwrap_or(0) as f64;
                nodes += n;
                sizes += n * size;
            }
        }
        (nodes > 0.0).then(|| sizes / nodes)
    }

    /// No statistics, of a lowered program read through its version's fields.
    pub fn empty_of_lowered(l: &lower::Lowered) -> Result<Self> {
        let placed = l.placed()?;
        Ok(Stats::empty_of(&placed.fields()?))
    }
    /// No statistics, of a program read through its fields (`Fields::kindsets`).
    pub fn empty_of(f: &fields::Fields<'_>) -> Self {
        Stats::with_kindsets(f.kindsets().iter().map(|(k, ks)| (f.text(*k).to_owned(), ks.iter().map(|&a| f.text(a).to_owned()).collect())).collect())
    }
    fn with_kindsets(kindsets: BTreeMap<String, Vec<String>>) -> Self {
        Stats {
            col: ColStats::default(),
            kfreq: Default::default(),
            size: Default::default(),
            nodes: 0.0,
            parents: Default::default(),
            card: Default::default(),
            delta_rows: Default::default(),
            fan: Default::default(),
            dfreq: Default::default(),
            heavy: Default::default(),
            heavy_mark: Default::default(),
            obs: HashMap::new(),
            solving: BTreeSet::new(),
            sized: BTreeSet::new(),
            solving_digest: 0,
            indexes: Default::default(),
            docs: None,
            memo: HashMap::new(),
            estimated: BTreeSet::new(),
            steps_cache: StepsCache::default(),
            kindsets: Arc::new(kindsets),
            lookup: Default::default(),
            fields: Default::default(),
            shapes: Default::default(),
            prepared: Default::default(),
            lazy: LazyMap::default(),
            prior_later: BTreeSet::new(),
        }
    }

    /// The tree's statistics measured in memory, replacing what was read (T's `stat_kind`, `stat_edge` and
    /// `stat_docrange`'s count): per kind atom its nodes and their summed subtree sizes, per (parent kind, child
    /// kind) its edges, and the documents.
    pub fn tree(&mut self, kinds: &BTreeMap<i64, (i64, i64)>, edges: &BTreeMap<(i64, i64), i64>, docs: i64) {
        self.kfreq = Arc::new(kinds.iter().map(|(k, (n, _))| (k.to_string(), *n)).collect());
        self.size = Arc::new(kinds.iter().filter(|(_, (n, _))| *n != 0).map(|(k, (n, size))| (k.to_string(), *size as f64 / *n as f64)).collect());
        self.nodes = kinds.values().map(|(n, _)| n).sum::<i64>() as f64;
        let mut parents: IndexMap<String, IndexMap<String, i64>> = IndexMap::new();
        for ((p, c), n) in edges {
            *parents.entry(c.to_string()).or_default().entry(p.to_string()).or_default() += n;
        }
        self.parents = Arc::new(parents
            .into_iter()
            .map(|(c, ps)| {
                let total = ps.values().sum::<i64>() as f64;
                (c, ps.into_iter().map(|(p, n)| (p, n as f64 / total)).collect())
            })
            .collect());
        self.docs = Some(docs);
        self.steps_cache.0.lock().clear();
    }

    /// Whether any column of `rel` has statistics (read or measured).
    /// The documents and kinds the statistics hold, which a layout's `values` facts are priced by (decision 0361
    /// item 5).
    pub fn counts(&self) -> shapes::Counts {
        shapes::Counts { docs: self.docs.filter(|&d| d > 0).map(|d| d as f64), kinds: (!self.kfreq.is_empty()).then(|| self.kfreq.len() as f64) }
    }

    /// Whether `rel`'s columns are measured now, not lazily.
    fn knows_now(&self, rel: &str) -> bool {
        self.col.has(rel)
    }

    pub fn knows(&self, rel: &str) -> bool {
        self.lazy.0.contains_key(rel) || self.col.has(rel)
    }

    /// `rel`'s statistics, to be made by `deferred` when a run first plans with them (`Model::resolve`).
    pub fn lazily(&mut self, rel: &str, rows: usize, deferred: std::sync::Arc<dyn Deferred>) {
        self.card.insert(rel.to_owned(), rows as f64);
        self.lazy.0.insert(rel.to_owned(), deferred);
    }

    /// `rel`'s statistics by column (`width` of them), and its exact postings of integer values (`measured`'s
    /// `vals`, by column), when every column is known: what a session's next run gives `rel__old`, whose rows are
    /// these (decision 0094 §4: statistics kept like data, not measured again).
    /// `of_relation` of the columns `cols` of `rel`, in that order (a copy of them: `corpus::projection_of`).
    pub fn of_columns(&self, rel: &str, columns: &[String], cols: &[usize]) -> Option<(Vec<[i64; 4]>, Vec<(usize, i64, i64)>)> {
        let names: Vec<String> = cols.iter().map(|&c| columns.get(c).cloned()).collect::<Option<_>>()?;
        let stats = names.iter().map(|n| self.col.get(&(rel.to_owned(), n.clone())).copied()).collect::<Option<Vec<_>>>()?;
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
        Some((stats, vals))
    }
    /// `columns`: its columns' names, as its source names them (a base relation's are its declaration's: `node`'s
    /// `d, n, kind, start, end_`).
    pub fn of_relation(&self, rel: &str, columns: &[String]) -> Option<(Vec<[i64; 4]>, Vec<(usize, i64, i64)>)> {
        let names: Vec<String> = columns.to_vec();
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
    pub fn measured(&mut self, rel: &str, cols: impl IntoIterator<Item = (String, [i64; 4])>, vals: impl IntoIterator<Item = (String, i64, i64)>) {
        // Positional names of a relation named like a base one are its base names (`input_column_name`).
        let named = |col: String| crate::code_rule_dsl::facts::core::input_column_name(rel, &col).map_or(col, str::to_owned);
        for (col, m) in cols {
            self.col.insert((rel.to_owned(), named(col)), m);
        }
        for (col, v, n) in vals {
            self.memo.entry(rel.to_owned()).or_default().insert((named(col), Constant::Int(v)), n);
        }
    }

    /// The tree's fielded edges, `(parent kind, child kind, field, edges)`, replacing what was read or measured.
    pub fn fielded(&mut self, edges: impl IntoIterator<Item = (String, String, String, f64)>) {
        let f = Fielded::of(edges);
        if !f.is_empty() {
            self.fields = std::sync::Arc::new(f);
            self.steps_cache.0.lock().clear();
        }
    }

    /// The lookup costs `source` declares for relation `rel`'s columns, those not one (`Source::lookup_cost`).
    pub fn lookup_costs(&mut self, rel: &str, source: &dyn crate::code_rule_dsl::facts::vtab::Source) {
        for (c, col) in source.columns().into_iter().enumerate() {
            let cost = source.lookup_cost(c);
            if cost != 1.0 {
                let col = crate::code_rule_dsl::facts::core::input_column_name(rel, &col).map_or(col, str::to_owned);
                std::sync::Arc::make_mut(&mut self.lookup).insert((rel.to_owned(), col), cost);
            }
        }
    }

    /// A lookup on `rel`'s column `col`, as a multiple of a derived relation's: one unless its source said.
    pub fn lookup_cost(&self, rel: &str, col: &str) -> f64 {
        if self.lookup.is_empty() {
            return 1.0;
        }
        self.lookup.get(&(rel.to_owned(), col.to_owned())).copied().unwrap_or(1.0)
    }

    /// The statistics of `p`'s constants and inputs (a whole version: its rules by handle, its fields).
    fn read(db: &Connection, p: &body::Placed, inputs: &Inputs, fingerprint: bool) -> Result<Self> {
        let pf = p.fields()?;
        let mut consts: BTreeSet<Constant> = BTreeSet::new();
        for k in 0..p.len() {
            let b = p.body(k);
            for i in 0..b.len() {
                if let Some(a) = b.plain_atom(i) {
                    consts.extend(a.args().filter_map(|t| b.constant(t)));
                }
            }
        }
        let holes = if consts.is_empty() { "NULL".to_owned() } else { vec!["?"; consts.len()].join(", ") };
        let (f, s) = (&inputs.facts, &inputs.stats);
        let mut sql = format!(
            "SELECT 'meta', k, NULL, v, NULL, NULL, NULL FROM {s}stat_meta \
             UNION ALL SELECT 'col', tbl, col, rows, ndv, sq, sqd FROM {s}stat_col \
             UNION ALL SELECT 'kind', kind, NULL, n, size, NULL, NULL FROM {s}stat_kind \
             UNION ALL SELECT 'edge', pkind, ckind, n, field, NULL, NULL FROM {s}stat_edge"
        );
        sql += &format!(" UNION ALL SELECT 'val', tbl, col, n, val, docs, sqd FROM {s}stat_val WHERE val IN ({holes})");
        for t in pf.inputs().filter(|t| fingerprint && !inputs.batch.contains_key(*t)) {
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
        let only_batch = pf.inputs().all(|t| inputs.batch.contains_key(t));
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
            size: Arc::new(of("kind").filter(|r| int(&r[2]) != 0).map(|r| (text(&r[0]), int(&r[3]) as f64 / int(&r[2]) as f64)).collect()),
            nodes: kfreq.values().sum::<i64>() as f64,
            kfreq: Arc::new(kfreq),
            parents: Arc::new(parents
                .into_iter()
                .map(|(c, ps)| {
                    let total = ps.values().sum::<i64>() as f64;
                    (c, ps.into_iter().map(|(p, n)| (p, n as f64 / total)).collect())
                })
                .collect()),
            card: Default::default(),
            delta_rows: Default::default(),
            fan: Default::default(),
            dfreq: Default::default(),
            heavy: Default::default(),
            heavy_mark: Default::default(),
            obs: HashMap::new(),
            solving: BTreeSet::new(),
            sized: BTreeSet::new(),
            solving_digest: 0,
            indexes: Default::default(),
            docs: meta.get("docs").copied(),
            estimated: BTreeSet::new(),
            memo: of("val").filter_map(|r| Some((text(&r[0]), (text(&r[1]), sql_constant(&r[3])?), int(&r[2])))).fold(HashMap::new(), |mut m, (t, k, n)| {
                m.entry(t).or_insert_with(HashMap::new).insert(k, n);
                m
            }),
            steps_cache: StepsCache::default(),
            kindsets: Arc::new(pf.kindsets().iter().map(|(k, ks)| (pf.text(*k).to_owned(), ks.iter().map(|&a| pf.text(a).to_owned()).collect())).collect()),
            lookup: Default::default(),
            fields: Arc::new(Fielded::of(of("edge").filter(|r| !matches!(r[3], SqlValue::Null)).map(|r| (text(&r[0]), text(&r[1]), text(&r[3]), int(&r[2]) as f64)))),
            shapes: Default::default(),
            prepared: Default::default(),
            lazy: LazyMap::default(),
            prior_later: BTreeSet::new(),
        })
    }

    /// Exact postings for a constant probe on an input column; `None` if the column has no statistics, or the value
    /// is missing from postings that are estimates (`estimated`).
    pub fn const_freq(&self, rel: &str, cname: &str, v: &Constant) -> Option<i64> {
        if let Some(&n) = self.memo.get(rel).and_then(|m| m.get(&(cname.to_owned(), v.clone()))) {
            return Some(n);
        }
        if self.estimated.contains(rel) {
            return None;
        }
        self.col.of(rel).is_some_and(|c| c.contains_key(cname)).then_some(0)
    }

    /// Posting-weighted fanout of an equality probe on `cname` confined to one document: sqd / rows.
    fn fan_doc(&self, rel: &str, cname: &str) -> f64 {
        let [rows, _, _, sqd] = self.col.of(rel).and_then(|c| c.get(cname)).copied().expect("a column's statistics");
        let skewed = skew(rel, cname.bytes().fold(0usize, |h, b| h.wrapping_mul(31).wrapping_add(b as usize)), 2);
        if rows != 0 { (sqd as f64 / rows as f64 * skewed).max(1.0) } else { 1.0 }
    }

    /// Expected parent reads to climb from `kind` to the nearest ancestor in `kset` (an absorbing Markov chain).
    fn steps(&self, kind: Option<&str>, kset: &str) -> f64 {
        let key = (kind.map(str::to_owned), kset.to_owned());
        if let Some(&s) = self.steps_cache.0.lock().get(&key) {
            return s;
        }
        let members: HashSet<&str> = self.kindsets.get(kset).into_iter().flatten().map(String::as_str).collect();
        let mut est: IndexMap<&str, f64> = IndexMap::new();
        // Tree depth bound; converges well before.
        for _ in 0..40 {
            let mut nxt = IndexMap::new();
            for (c, ps) in self.parents.iter() {
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
            for (k, &n) in self.kfreq.iter() {
                if let Some(e) = est.get(k.as_str()) {
                    out += n as f64 / total * e;
                }
            }
        }
        self.steps_cache.0.lock().insert(key, out);
        out
    }
}

/// The planner's model of the data: statistics, distinct counts, and what the views are.
pub struct Model {
    /// The program's logical key and scalar analysis, made once per plan and shared (not made again per stratum),
    /// when first asked (`analysis`): only the SQL executor reads it, so the native set engine's runs never make it.
    pub analysis: std::sync::OnceLock<std::sync::Arc<super::analysis::ProgramAnalysis>>,
    pub stats: Stats,
    pub ndv: ColMap<f64>,
    /// rel → (node column, document column) for projections of `node(d, n)`: documents are id intervals.
    pub docof: std::sync::Arc<HashMap<String, (usize, usize)>>,
    /// View over one input atom → (input, the input column behind each view column).
    pub view_src: std::sync::Arc<HashMap<String, (String, Vec<Option<String>>)>>,
    /// Shared, not copied, by a component's model (`Model::fork`): set before the strata run.
    pub inputs: std::sync::Arc<Inputs>,
    /// The run's documents are node-id intervals by their keys (`d << 32` to `d << 32 | 0xffff_ffff`,
    /// `docs_by_shift`): a range read computes its interval, and reads no table of them.
    pub docs_by_shift: bool,
    /// The program's join shape directives (`front::Shape`), for every form planned with this model.
    pub shapes: std::sync::Arc<Vec<crate::code_rule_dsl::logic::front::Shape>>,
    /// The relations the program's rules derive or its facts state. Their columns are `c0`, `c1`, … even where an
    /// input of the same name has named columns: `rr::lower`'s projections (`kind`, `docof` over nodes) are rules.
    pub derived: std::sync::Arc<HashSet<String>>,
}

thread_local! {
    /// Set while a rule is searched without its join shape directives (`Model::plan_inner`).
    static NO_SHAPES: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Reads of an input delta (`r__plus`, `r__minus`) priced with no size: each is priced at the default's 1,000 rows. A
/// delta is bound with the run and measured, so none should be (the guard in `a_session_maintains_a_recursive_cross_file_stratum`).
static UNSIZED_DELTAS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Relations whose statistics their layouts answered (`Model::resolve`, decision 0361 step 4): none was made.
static SHAPED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
#[cfg_attr(not(any(test, feature = "test-hooks")), allow(dead_code))]
pub fn shaped_statistics() -> u64 {
    SHAPED.load(std::sync::atomic::Ordering::Relaxed)
}
#[cfg_attr(not(any(test, feature = "test-hooks")), allow(dead_code))]
pub fn unsized_deltas() -> u64 {
    UNSIZED_DELTAS.load(std::sync::atomic::Ordering::Relaxed)
}

impl Model {
    /// Whether `rel` is one of `prog`'s inputs. A maintained program has thousands (each kept relation's `__old`,
    /// each changed input's `__plus` and `__minus`), and the planner asks for every atom it prices: a scan of the
    /// list was 40% of `plan_search` in a session step. A long list is hashed once per model; the programs a model
    /// plans are alive while it is, so an address and length name one list.
    /// Makes the lazy statistics (`Stats::lazily`) of the relations `rels` that have them, before a plan reads
    /// them: a stratum's rules, a seed join, a view, the inline choice's cone.
    pub fn resolve<'r>(&mut self, f: Fields<'_>, rels: impl IntoIterator<Item = &'r str>) -> Result<()> {
        if self.stats.lazy.0.is_empty() && self.stats.prior_later.is_empty() {
            return Ok(());
        }
        for rel in rels {
            if self.stats.prior_later.remove(rel) {
                prior_now(self, f, rel, true);
            }
            let Some(deferred) = self.stats.lazy.0.remove(rel) else { continue };
            // Its layout answers it whole: priced from the facts, nothing counted (decision 0361 step 4).
            if let Some(width) = f.arity(rel)
                && self.stats.shapes.answers(rel, self.stats.counts())
            {
                let rows = self.stats.card.get(rel).copied().unwrap_or(1.0).max(1.0);
                for c in 0..width {
                    let n = self.stats.shapes.ndv(rel, c, rows, self.stats.counts()).expect("answered");
                    self.ndv.insert((rel.to_owned(), c), n);
                    self.stats.fan.insert((rel.to_owned(), c), (rows / n).max(1.0));
                }
                SHAPED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                super::profile::decision("statistics answered by a layout (decision 0361)", || rel.to_owned());
                continue;
            }
            let _span = super::profile::span(|| format!("statistics of {rel}, made when first planned with"));
            let (cols, vals) = deferred.statistics();
            self.stats.measured(rel, cols, vals);
            if let Some(width) = f.arity(rel) {
                input_stats(&mut self.stats, &mut self.ndv, Some(width), rel)?;
            }
        }
        Ok(())
    }

    /// A component's own model (decision 0113 §4): what its strata plan with, run on any thread. It holds the
    /// statistics of `rels` alone (the relations its rules read, with their views' inputs and their kept `__old`
    /// relations, and its own), copied, and shares the rest (the inputs, the tree's statistics, the views). `rels`
    /// its strata resolve first (`resolve`) are resolved here, and the others' lazy statistics go with it. Its strata only read
    /// relations that are retired, and write their own: `absorb` takes those back when it retires.
    pub fn fork<'r>(&mut self, f: Fields<'_>, rels: &BTreeSet<String>, rules: &[u64], resolve: impl IntoIterator<Item = &'r str>) -> Result<Model> {
        // What its strata would resolve first (`read_by`): the others' lazy statistics go with it, made if asked.
        self.resolve(f, resolve)?;
        Ok(self.fork_with(&|r: &str| f.arity(r).unwrap_or(0), rels, rules))
    }

    /// `fork`, its strata's statistics resolved, with `width` a relation's columns (several programs' relations: a
    /// component of maintained forms, decision 0117).
    fn fork_with(&self, width: &dyn Fn(&str) -> usize, rels: &BTreeSet<String>, rules: &[u64]) -> Model {
        let st = &self.stats;
        let mut stats = Stats {
            col: ColStats(rels.iter().filter_map(|r| Some((r.clone(), st.col.of(r)?.clone()))).collect()),
            kfreq: st.kfreq.clone(),
            size: st.size.clone(),
            nodes: st.nodes,
            parents: st.parents.clone(),
            card: Default::default(),
            delta_rows: Default::default(),
            fan: Default::default(),
            dfreq: Default::default(),
            heavy: Default::default(),
            heavy_mark: Default::default(),
            obs: rules.iter().filter_map(|r| Some((*r, st.obs.get(r)?.clone()))).collect(),
            solving: BTreeSet::new(),
            sized: BTreeSet::new(),
            solving_digest: 0,
            indexes: st.indexes.clone(),
            docs: st.docs,
            memo: HashMap::default(),
            estimated: rels.iter().filter(|r| st.estimated.contains(*r)).cloned().collect(),
            steps_cache: StepsCache::default(),
            kindsets: st.kindsets.clone(),
            lookup: st.lookup.clone(),
            fields: st.fields.clone(),
            shapes: st.shapes.clone(),
            prepared: st.prepared.clone(),
            lazy: LazyMap(rels.iter().filter_map(|r| Some((r.clone(), st.lazy.0.get(r)?.clone()))).collect()),
            prior_later: rels.iter().filter(|r| st.prior_later.contains(*r)).cloned().collect(),
        };
        let mut ndv = ColMap::default();
        for r in rels {
            if let Some(&n) = st.card.get(r) { stats.card.insert(r.clone(), n); }
            if let Some(&n) = st.delta_rows.get(r) { stats.delta_rows.insert(r.clone(), n); }
            if let Some(m) = st.heavy_mark.get(r) { stats.heavy_mark.insert(r.clone(), m.clone()); }
            if let Some(m) = st.memo.get(r) { stats.memo.insert(r.clone(), m.clone()); }
            let w = width(r);
            ndv.share_from(&self.ndv, r, w);
            stats.fan.share_from(&st.fan, r, w);
            stats.dfreq.share_from(&st.dfreq, r, w);
            stats.heavy.share_from(&st.heavy, r, w);
        }
        stats.solve(BTreeSet::new());
        let analysis = std::sync::OnceLock::new();
        if let Some(a) = self.analysis.get() { let _ = analysis.set(a.clone()); }
        Model { shapes: self.shapes.clone(), analysis, stats, ndv, docof: self.docof.clone(), view_src: self.view_src.clone(), inputs: self.inputs.clone(), docs_by_shift: self.docs_by_shift, derived: self.derived.clone() }
    }

    /// Takes back what a component's model (`fork`) learnt of its own relations `own` and its rules `rules` (their
    /// observed fanouts), as the component retires: the same statistics a serial run would have left.
    pub fn absorb(&mut self, f: Fields<'_>, mut fork: Model, own: &BTreeSet<String>, rules: &[u64]) {
        self.absorb_with(&|r: &str| f.arity(r).unwrap_or(0), &mut fork, own, rules);
    }

    /// `absorb`, with `width` a relation's columns, taking from `fork` what is taken (a component of several forms
    /// takes each form's own in turn).
    fn absorb_with(&mut self, width: &dyn Fn(&str) -> usize, fork: &mut Model, own: &BTreeSet<String>, rules: &[u64]) {
        fn put<V>(to: &mut table::RelMap<V>, from: &mut table::RelMap<V>, k: &str) {
            match from.remove(k) { Some(v) => { to.insert(k, v); } None => { to.remove(k); } }
        }
        // Heavy values a fork made, of any relation it read: made once, kept for the strata after.
        for ((r, c), h) in fork.stats.heavy.iter() {
            if self.stats.heavy.at(&r, c).is_none_or(|m| !std::sync::Arc::ptr_eq(m, h)) {
                self.stats.heavy.insert((&*r, c), h.clone());
            }
        }
        for (r, m) in fork.stats.heavy_mark.iter() {
            self.stats.heavy_mark.insert(&*r, m.clone());
        }
        for r in own {
            put(&mut self.stats.card, &mut fork.stats.card, r);
            put(&mut self.stats.delta_rows, &mut fork.stats.delta_rows, r);
            let w = width(r);
            self.ndv.take_from(&mut fork.ndv, r, w);
            self.stats.fan.take_from(&mut fork.stats.fan, r, w);
            self.stats.dfreq.take_from(&mut fork.stats.dfreq, r, w);
            if fork.stats.prior_later.contains(r) { self.stats.prior_later.insert(r.clone()); } else { self.stats.prior_later.remove(r); }
        }
        for rule in rules {
            if let Some(o) = fork.stats.obs.remove(rule) { self.stats.obs.insert(*rule, o); }
        }
    }

    /// The relations `rules` read, for `resolve`. A rule of one positive atom whose arguments are distinct
    /// variables (a survivor, `h :- h__old, !h__del`) has one order, and its estimate is that atom's rows, which a
    /// lazy relation is given when bound (`Stats::lazily`): its relations' columns are not needed.
    /// Each rule is read by its handles in `vs` (`read_by_body`).
    fn read_by<'v>(vs: &'v body::Versions, rules: &[RuleKey]) -> Vec<&'v str> {
        rules.iter().flat_map(|&r| Self::read_by_body(&vs.at(r))).collect()
    }

    /// `read_by` of one rule, read by handles.
    fn read_by_body<'v>(b: &Body<'v>) -> Vec<&'v str> {
        let atoms = || (0..b.len()).filter_map(|i| b.atom(i));
        let pos: Vec<AtomRef<'v>> = atoms().filter(|a| !a.neg()).collect();
        let plain = pos.len() == 1 && {
            let mut seen = VarSet::default();
            pos[0].args().all(|t| t.var().is_some_and(|x| seen.insert(x)))
        };
        if plain { Vec::new() } else { atoms().map(|a| b.rel(a)).collect() }
    }

    /// Whether `rel` is one of the program's inputs (`f`, its version's: a set read once per version).
    fn is_input(&self, f: Fields<'_>, rel: &str) -> bool {
        f.is_input(rel)
    }

    /// Column `col` of `rel` in SQL: an input's name, or `c{col}` for a relation the program derives.
    pub fn col(&self, rel: &str, col: usize) -> String {
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
pub fn sql_col(rel: &str, col: usize) -> String {
    super::sql::select::input_columns(rel).and_then(|names| names.get(col)).map_or_else(|| format!("c{col}"), |&n| n.to_owned())
}

/// The kept-plan outcomes of `plan_rule_reused` in this process, by `CacheOutcome`: `RERAIL_PLAN_CACHE_COUNTS`
/// prints them at the end of each corpus run (`KeepPlans`).
static PLAN_CACHE: [std::sync::atomic::AtomicU64; 12] = [const { std::sync::atomic::AtomicU64::new(0) }; 12];
const PLAN_CACHE_NAMES: [&str; 12] = ["no cache", "first sight", "kept price", "re-priced", "replanned: cheaper start", "replanned: work grew", "replanned: no longer runs", "taken from the store", "kept: ran below a search", "searched: ran over its price", "started: its line's order", "started: its source rule's order"];
/// Why a kept plan was not taken as it is (decision 0122), under `RERAIL_PLAN_CACHE_COUNTS`: no price kept, never
/// ran, ran over a search's work, or its inputs grew past what that allows.
static BUDGET_MISS: [std::sync::atomic::AtomicU64; 4] = [const { std::sync::atomic::AtomicU64::new(0) }; 4];
/// Decision 0123's statistics, under `RERAIL_PLAN_CACHE_COUNTS`: batches made, relations they made, relations made
/// late (one line at a time, `Model::late`).
static STATISTICS: [std::sync::atomic::AtomicU64; 3] = [const { std::sync::atomic::AtomicU64::new(0) }; 3];
/// `RERAIL_PLAN_FRESH`: every plan searched, none reused (a measurement: what kept orders cost).
fn plan_fresh() -> bool {
    static FRESH: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var_os("RERAIL_PLAN_FRESH").is_some());
    *FRESH
}
thread_local! {
    /// Where a plan is being made, for the profile's account of the planning budget's misses.
    static PLAN_SITE: std::cell::Cell<&'static str> = const { std::cell::Cell::new("elsewhere") };
}
fn plan_site(site: &'static str) {
    PLAN_SITE.with(|c| c.set(site));
}
/// Orders made, by how (`RERAIL_PLAN_CACHE_COUNTS`): searched, and taken greedily under a search's cost (decision 0123
/// §7).
static SEARCHES: [std::sync::atomic::AtomicU64; 2] = [const { std::sync::atomic::AtomicU64::new(0) }; 2];
/// Decision 0123 §9's probes in this process: left unplanned when their stratum opened, and planned later (when their
/// delta came, or when the device took their stratum). Never reset, so a test reads its own run's by difference.
static LAZY: [std::sync::atomic::AtomicU64; 2] = [const { std::sync::atomic::AtomicU64::new(0) }; 2];
#[cfg_attr(not(any(test, feature = "test-hooks")), allow(dead_code))]
pub fn lazy_probes_counted() -> (u64, u64) {
    (LAZY[0].load(std::sync::atomic::Ordering::Relaxed), LAZY[1].load(std::sync::atomic::Ordering::Relaxed))
}

fn plan_cache(outcome: usize) {
    PLAN_CACHE[outcome].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// A run's quiet strata (decision 0101 §1), counted across its strands (`Run::strand`) and reported in its
/// `Outcome`: the strata passed on unchanged, and the relations held as kept with their rows' count alone, no
/// column measured (`quiet_card`). Each column of those reads as one value, and every probe of one as all its rows.
/// None should be (the guard in `a_session_passes_on_a_quiet_stratum`). Per run, not per process: a process-wide
/// count let one test's runs move another's.
#[derive(Debug, Default)]
struct QuietTally {
    strata: std::sync::atomic::AtomicUsize,
    unmeasured: std::sync::atomic::AtomicUsize,
}

/// A survivors rule of a maintained stratum, `h(v..) :- h__old(v..), !h__del(v..)` (decision 0077's rewrite): the
/// kept relation native, the deleted one of a lower stratum.
/// Under decision 0090's levels the rows gone are `h__gone`'s, decided in level order, not `h__del`'s.
/// A levels stratum's kept rows to check against, `h__live(v..) :- h__old(v..), !h__gone(v..)` with the negation
/// gated (it only places `h__live` in the stratum): `(h__old, "-")`, the kept relation with no row gone.
fn survivors(r: &Body<'_>, scc: &BTreeSet<String>, native_rels: &BTreeSet<String>) -> Option<(String, String)> {
    let h = r.rel(r.head());
    let vars: Vec<VarSlot> = r.head().args().map(|t| t.var()).collect::<Option<_>>()?;
    let distinct = VarSet::of(vars.iter().copied()).len() == vars.len();
    if r.len() != 2 {
        return None;
    }
    let (Some(a), Some(b)) = (r.plain_atom(0), r.plain_atom(1)) else { return None };
    let same_args = |x: AtomRef<'_>| x.args().map(|t| t.var()).eq(vars.iter().map(|&v| Some(v)));
    if !(distinct && scc.contains(h) && !a.neg() && b.neg() && same_args(a) && same_args(b)) {
        return None;
    }
    let (a_rel, b_rel) = (r.rel(a), r.rel(b));
    if let Some(base) = h.strip_suffix("__live") {
        let (old, gone) = (format!("{base}__old"), format!("{base}__gone"));
        return (b.gated() && a_rel == old && b_rel == gone && native_rels.contains(&old)).then(|| (old, "-".to_owned()));
    }
    // The kept relation is native, read in place; the deleted one is of a lower stratum, native or a table.
    let old = format!("{h}__old");
    [format!("{h}__del"), format!("{h}__gone")]
        .into_iter()
        .find(|gone| a_rel == old && b_rel == gone && native_rels.contains(&old) && !scc.contains(gone))
        .map(|gone| (old, gone))
}

/// A maintenance relation (decision 0090: the rows lost, checked, held, gone, inserted or derived again): it starts
/// empty and takes small batches from its first row, so the plan for a delta that is its whole relation (one that
/// scans the other relations first) is never the right one for it.
fn small_batches(rel: &str) -> bool {
    // `RERAIL_SMALL_WHOLE=dred`: DRed's and counting's relations take whole plans again, as before, to compare.
    static DRED_WHOLE: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var("RERAIL_SMALL_WHOLE").is_ok_and(|v| v == "dred"));
    let dred = ["__del", "__lost", "__ins", "__redo"];
    let levels = ["__cand", "__chk", "__gone", "__held"];
    levels.iter().any(|s| rel.ends_with(s)) || (!*DRED_WHOLE && dred.iter().any(|s| rel.ends_with(s)))
}

/// A levels stratum's relations of `h`, when `scc` has all five (`native::setfix::Levels`): cand, chk, held, gone,
/// live.
fn levels_of(h: &str, scc: &BTreeSet<String>) -> Option<[String; 5]> {
    let names = ["cand", "chk", "held", "gone", "live"].map(|s| format!("{h}__{s}"));
    names.iter().all(|n| scc.contains(n)).then_some(names)
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

/// What a join-order search costs, in rows of join work (O-live-replan item 1): a fixed part and one row a candidate.
/// Measured on the P3 bench (debug): a search costs about 0.3 ms plus 0.7 µs a candidate priced, and a join about
/// 0.8 µs a unit of work. The candidates follow from the joins `j`: with the delta first, `(j-1)·2^(j-2)+1` (`fgp`'s
/// 12 joins 11,265, `lcbind`'s 9 1,025, `lgpar`'s 6 81, as counted); without, `j·2^(j-1)`; past `GREEDY`, `j²`.
const SEARCH_FIXED_ROWS: f64 = 400.0;
/// Decision 0258: a line starts from a kept order only where a search would price at least this many orders. With
/// fewer, the search prices about what the start does, under the same set-up.
const START_CANDIDATES: f64 = 3.0;
/// Decision 0123 §9: a probe with no kept plan is planned when its delta first has rows. `RERAIL_LAZY_PROBES=off`
/// plans every probe when its stratum opens, as before (a measurement aid).
fn lazy_probes() -> bool {
    static ON: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var("RERAIL_LAZY_PROBES").map_or(true, |v| v != "off"));
    *ON
}

/// Decision 0123 §7: a kept order priced at `priced` rows of work, under what a search costs, whose line did `ran` rows,
/// a search's work or more, when it last ran. A search can pay there, and costs less than the line ran.
pub fn mispriced(priced: f64, ran: Option<f64>, b: &Body<'_>, first: Option<usize>) -> bool {
    let search = search_rows(b, first);
    ran.is_some_and(|ran| ran >= search.max(SEARCH_WORK) && priced <= search)
}

pub fn search_rows(b: &Body<'_>, first: Option<usize>) -> f64 {
    let j = (0..b.len()).filter(|&i| b.is_join(i)).count() as f64;
    let candidates = if j > GREEDY as f64 {
        j * j
    } else if first.is_some() {
        (j - 1.0).max(0.0) * 2f64.powf(j - 2.0) + 1.0
    } else {
        j * 2f64.powf(j - 1.0)
    };
    SEARCH_FIXED_ROWS + candidates
}

thread_local! {
    /// Plans kept across a corpus run's chunks, on the thread that runs them: per rule (head, body, delta, the
    /// component solved), its order and its work per node when planned. `None` outside a corpus run: every plan is
    /// searched.
    /// When profiling, the plan as EXPLAIN showed it when searched: a re-pricing reports it with its own work and rows.
    static PLANS: std::cell::RefCell<Option<SharedPlans>> = const { std::cell::RefCell::new(None) };
    /// The last priced body's cheapest start: the least work of any one join literal read first, from no binding.
    /// Every order costs at least this, so a kept order's work over it measures how far the order is from the best
    /// any start allows.
    static SEED: std::cell::Cell<f64> = const { std::cell::Cell::new(1.0) };
    /// A job's own plans (decision 0113, a strand that entered the run's, `enter_plans`): what it plans or prices is
    /// kept here, read before the run's, and taken into the run's once, as the job ends; the run's are read under a
    /// shared lock, not taken per rule (four strands planning took turns at one lock: 2.9 s of an edit's 7 s on them).
    static OWN_PLANS: std::cell::RefCell<Option<rustc_hash::FxHashMap<PlanKey, Kept>>> = const { std::cell::RefCell::new(None) };
    /// A job's keys planned (profiled, `RunPlans::planned_now`), kept as its plans are and taken into the run's as
    /// it ends: written to the run's, every plan of every job took the run's lock exclusively.
    static OWN_PLANNED: std::cell::RefCell<Option<rustc_hash::FxHashSet<PlanKey>>> = const { std::cell::RefCell::new(None) };
}

/// The plan kept for `key` (`RunPlans::find`), this thread's job's first: `None` outside a corpus run.
fn plans_find(key: &PlanKey, len: usize, nodes: f64) -> Option<Option<Kept>> {
    PLANS.with(|p| {
        let p = p.borrow();
        let shared = p.as_ref()?;
        if let Some(k) = OWN_PLANS.with(|o| o.borrow().as_ref().and_then(|m| m.get(key).cloned())) {
            return Some(Some(k));
        }
        // The run's plans as last published (decision 0301), or as a chunk's view holds them (Q88): read with no lock.
        if let Some(e) = shared.now().get(key) {
            return Some(Some(e.kept()));
        }
        let found = shared.stored_near(key, len, nodes);
        if let Some(k) = &found {
            plan_cache(7);
            plans_put(shared, *key, k.clone());
        }
        Some(found)
    })
}

/// The plan kept for `key` as it is, this thread's job's first, else the run's (not looked up in the store).
fn plans_get(key: &PlanKey) -> Option<Kept> {
    PLANS.with(|p| {
        let p = p.borrow();
        let shared = p.as_ref()?;
        OWN_PLANS.with(|o| o.borrow().as_ref().and_then(|m| m.get(key).cloned())).or_else(|| shared.now().get(key).map(|e| e.kept()))
    })
}

/// Keeps `kept` for `key` (decision 0301): where the run holds `key` with the same order, in its entry's cell, seen
/// by every job at once; a new key or a new order in this thread's job's plans (published as it ends), else published
/// now.
fn plans_put(shared: &SharedPlans, key: PlanKey, kept: Kept) {
    let kept = OWN_PLANS.with(|o| match o.borrow_mut().as_mut() {
        Some(m) if m.contains_key(&key) => {
            m.insert(key, kept);
            None
        }
        _ => Some(kept),
    });
    let Some(kept) = kept else { return };
    // A chunk's view (Q88) changes no entry it shares with the run's plans: its own go in as new entries.
    if shared.view.is_none() && shared.kept.load().get(&key).is_some_and(|e| e.set(&kept)) {
        return;
    }
    let kept = OWN_PLANS.with(|o| match o.borrow_mut().as_mut() {
        Some(m) => {
            m.insert(key, kept);
            None
        }
        None => Some(kept),
    });
    if let Some(kept) = kept {
        match &shared.view {
            Some(v) => v.put(vec![(key, kept)]),
            None => shared.publish(vec![(key, kept)]),
        }
    }
}

/// Sets what a kept plan's runs taught of it (decision 0301): its last price (`Priced`, `Kept`'s 4th) or the work its
/// line did (`ran`, its 5th), in this thread's job's plan where it holds `key`, else in the run's entry's cell for it.
/// Each is a write of its own field, the last writer's: concurrent chunks run the same rules, and one cell for both,
/// changed by compare-and-swap, retried and copied the entry under that contention (2,774 samples of a cold run).
fn plans_learnt(key: &PlanKey, learnt: Learnt) {
    PLANS.with(|p| {
        let p = p.borrow();
        let Some(shared) = p.as_ref() else { return };
        let learnt = OWN_PLANS.with(|o| {
            let mut o = o.borrow_mut();
            let Some(k) = o.as_mut().and_then(|m| m.get_mut(key)) else { return Some(learnt) };
            match learnt {
                Learnt::Priced(priced) => k.4 = Some(priced),
                Learnt::Ran(work) => k.5 = Some(work),
            }
            None
        });
        // In a chunk's view (Q88), a copy of the entry learns it, as the chunk's own: the view's entries are the
        // run's as filed, shared with the other chunks.
        let learnt = match (&shared.view, learnt) {
            (Some(v), Some(learnt)) => {
                let Some(mut k) = shared.now().get(key).map(|e| e.kept()) else { return };
                match learnt {
                    Learnt::Priced(priced) => k.4 = Some(priced),
                    Learnt::Ran(work) => k.5 = Some(work),
                }
                let k = OWN_PLANS.with(|o| match o.borrow_mut().as_mut() {
                    Some(m) => {
                        m.insert(*key, k);
                        None
                    }
                    None => Some(k),
                });
                if let Some(k) = k {
                    v.put(vec![(*key, k)]);
                }
                return;
            }
            (_, learnt) => learnt,
        };
        if let Some(learnt) = learnt
            && let Some(e) = shared.kept.load().get(key)
        {
            match learnt {
                Learnt::Priced(priced) => {
                    let old = e.priced.lock().replace(priced);
                    drop(old);
                }
                Learnt::Ran(work) => e.ran.store(work.to_bits(), std::sync::atomic::Ordering::Relaxed),
            }
        }
    });
}

/// What a kept plan's run taught of it (`plans_learnt`).
enum Learnt {
    Priced(Arc<Priced>),
    Ran(f64),
}


/// A kept plan's key: a digest of the rule, the delta and the component solved (`derived::digest`). Not the rule's
/// address: each chunk's program is a new value (the demand split makes one per chunk), so no address is kept between
/// chunks. Not the rule's text either: formatting every rule was a tenth of a session run's planning.
type PlanKey = u128;

/// A kept plan: its order, its work per node when searched, when profiling its EXPLAIN, its work over its body's
/// cheapest start (`SEED`) when searched, its last price with the sizes it was priced at (`Priced`), and the work
/// its rule's line did in the engine when it last ran (`ran`, decision 0122), if it has run.
type Kept = (Arc<[Step]>, f64, Option<super::profile::RulePlan>, f64, Option<Arc<Priced>>, Option<f64>);

/// Decision 0122: planning costs no more than running. A search costs about as much as this many rows of the
/// engine's join work (O125 step 07: 0.16 ms a search, 0.74 µs a row of work), so a rule whose line did less work
/// when it last ran, its inputs grown by no more than that allows, keeps its order with no search and no pricing;
/// in a run, a probe is searched again only once its work has paid for its searches. `RERAIL_PLAN_BUDGET=off`
/// plans as before (a measurement aid).
pub const SEARCH_WORK: f64 = 250.0;

pub(in crate::code_rule_dsl) fn plan_budget() -> bool {
    static ON: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| !std::env::var("RERAIL_PLAN_BUDGET").is_ok_and(|v| v == "off"));
    *ON
}

/// How many times an order's work may have grown from sizes `then` to `now`: the product of each size's growth (each
/// at least one row, a shrinking one counted as 1), since a fixed order's work is bounded by the product of what it
/// reads; unbounded where they do not compare.
fn growth(then: &[f64], now: &[f64]) -> f64 {
    if then.len() != now.len() {
        return f64::INFINITY;
    }
    then.iter().zip(now).map(|(a, b)| if *a < 0.0 || *b < 0.0 { if a == b { 1.0 } else { f64::INFINITY } } else { (b.max(1.0) / a.max(1.0)).max(1.0) }).product()
}

/// A run's kept plans, shared by the threads its strata run on (decision 0113): a stratum's job enters them
/// (`enter_plans`). Each stratum plans its own rules, so no two jobs write one key.
/// Within a chunk (Q88), with its view of them: what it plans and learns goes into the view, and is the run's only
/// as the main thread files the chunk (`OrderedPlans`).
#[derive(Clone)]
struct SharedPlans {
    run: Arc<RunPlans>,
    view: Option<Arc<ChunkView>>,
}

impl std::ops::Deref for SharedPlans {
    type Target = RunPlans;
    fn deref(&self) -> &RunPlans {
        &self.run
    }
}

impl SharedPlans {
    /// The plans a planner reads: its chunk's view, else the run's as last published.
    fn now(&self) -> arc_swap::Guard<Arc<Plans>> {
        match &self.view {
            Some(v) => v.kept.load(),
            None => self.run.kept.load(),
        }
    }
}

/// Q88: a chunk plans from the run's plans as filed up to this many chunks before it, never from what a chunk running
/// beside it published: up to `PLAN_WINDOW` evaluate at once (decision 0254: at most 8), so those are filed when it
/// begins, whatever the threads' timing, and its plans are a function of the run's inputs.
pub const PLAN_WINDOW: usize = 8;

/// A chunk's view of the run's plans (Q88): the run's as filed when it began (`OrderedPlans::next`), with what it
/// planned and learnt over them, its own entries (`own`), handed back with it (`in_view`).
pub struct ChunkView {
    kept: arc_swap::ArcSwap<Plans>,
    own: parking_lot::Mutex<rustc_hash::FxHashSet<PlanKey>>,
    observed: Option<keeps::SharedObserved>,
}

impl ChunkView {
    /// Takes `plans` as the chunk's own, new entries over the view's.
    fn put(&self, plans: Vec<(PlanKey, Kept)>) {
        if plans.is_empty() {
            return;
        }
        let entries: Vec<(PlanKey, Arc<Entry>)> = plans.into_iter().map(|(k, v)| (k, Entry::of(v))).collect();
        self.own.lock().extend(entries.iter().map(|(k, _)| *k));
        self.kept.rcu(|m| {
            let mut m = Plans::clone(m);
            m.extend(entries.iter().map(|(k, e)| (*k, Arc::clone(e))));
            m
        });
    }
}

/// What a chunk planned, learnt and observed (`in_view`), for the run's in the chunks' order (`OrderedPlans::file`).
#[derive(Default)]
pub struct ChunkPlans(Vec<(PlanKey, Kept)>, Option<keeps::Observed>);

/// Q88: the run's plans and observations as its chunks are filed, in their order. Chunk `n` (from 0) sees them as they
/// were once chunks `0..=n - window` were filed; what it made is the run's when it is filed.
pub struct OrderedPlans {
    run: SharedPlans,
    observed: Option<keeps::SharedObserved>,
    window: usize,
    /// The run's plans and observations once `first + i` chunks were filed, for those a chunk to begin may see.
    snapshots: std::collections::VecDeque<(Arc<Plans>, Option<Arc<keeps::Observed>>)>,
    first: usize,
    begun: usize,
    filed: usize,
}

impl OrderedPlans {
    /// Over this thread's run's plans: `None` outside a corpus run.
    pub fn begin(window: usize) -> Option<Self> {
        let run = share_plans()?;
        let observed = keeps::share_observed();
        let at = (run.kept.load_full(), keeps::observed_now(&observed));
        let observed = at.1.is_some().then_some(observed);
        Some(OrderedPlans { run, observed, window: window.max(1), snapshots: [at].into(), first: 0, begun: 0, filed: 0 })
    }

    /// The next chunk's view.
    pub fn next(&mut self) -> Arc<ChunkView> {
        let n = self.begun;
        self.begun += 1;
        let at = (n + 1).saturating_sub(self.window);
        assert!(at <= self.filed, "Q88: chunk {n} begins with {} filed, under {at}", self.filed);
        while self.first < at {
            self.snapshots.pop_front();
            self.first += 1;
        }
        let (plans, observed) = self.snapshots[at - self.first].clone();
        Arc::new(ChunkView {
            kept: arc_swap::ArcSwap::new(plans),
            own: Default::default(),
            observed: self.observed.as_ref().map(|_| keeps::chunk_observed(observed)),
        })
    }

    /// Files the next chunk's plans and observations into the run's, in the chunks' order.
    pub fn file(&mut self, made: ChunkPlans) {
        let ChunkPlans(plans, observed) = made;
        self.run.publish(plans);
        if let (Some(run), Some(o)) = (&self.observed, observed) {
            keeps::file_observed(run, o);
        }
        self.filed += 1;
        let at = (self.run.kept.load_full(), self.observed.as_ref().and_then(keeps::observed_now));
        self.snapshots.push_back(at);
    }
}

/// `f` planning within a chunk's view (Q88): what it plans, learns and observes stays in the view (and the jobs it
/// starts, which enter it), and is handed back with its result for the run's (`OrderedPlans::file`).
pub fn in_view<R>(view: &Arc<ChunkView>, f: impl FnOnce() -> R) -> (R, ChunkPlans) {
    let Some(run) = share_plans() else { return (f(), ChunkPlans::default()) };
    let shared = SharedPlans { run: Arc::clone(&run.run), view: Some(Arc::clone(view)) };
    let r = {
        let _plans = enter_plans(Some(shared));
        let _observed = view.observed.clone().map(keeps::enter_observed);
        f()
    };
    let keys: Vec<PlanKey> = {
        let mut keys: Vec<PlanKey> = view.own.lock().drain().collect();
        keys.sort_unstable();
        keys
    };
    let kept = view.kept.load();
    let plans = keys.into_iter().filter_map(|k| kept.get(&k).map(|e| (k, e.kept()))).collect();
    (r, ChunkPlans(plans, view.observed.as_ref().map(keeps::observed_made)))
}

/// A kept plan as the run shares it (decision 0301): its order and what its search found of it (work per node,
/// EXPLAIN, gap), fixed until a put replaces them, and what its runs taught (its last price, the work its line did),
/// each a cell of its own written by the last writer. A run's plans are read 1.9 M times over PostHog and get a new
/// order a few hundred times; what changes all the time is what is learnt, and that changes a cell, not the map.
pub struct Entry {
    order: Arc<[Step]>,
    found: arc_swap::ArcSwap<Found>,
    /// Behind a mutex held to swap a pointer: written at every planning hit (330,000 times over PostHog), where an
    /// `ArcSwapOption`'s store settled every loading thread's debts (942 samples of a cold run).
    priced: parking_lot::Mutex<Option<Arc<Priced>>>,
    /// The work its line did, as f64 bits; `NOT_RAN` is none.
    ran: std::sync::atomic::AtomicU64,
}

/// `Entry::ran`'s none: a NaN's bits no run's work has.
const NOT_RAN: u64 = u64::MAX;

/// What a kept plan's search found of it (`Entry::found`).
struct Found {
    per_node: f64,
    explain: Option<super::profile::RulePlan>,
    gap: f64,
}

impl Entry {
    fn of(k: Kept) -> Arc<Entry> {
        let found = arc_swap::ArcSwap::from_pointee(Found { per_node: k.1, explain: k.2, gap: k.3 });
        Arc::new(Entry { order: k.0, found, priced: parking_lot::Mutex::new(k.4), ran: std::sync::atomic::AtomicU64::new(k.5.map_or(NOT_RAN, f64::to_bits)) })
    }

    /// The plan as `Kept`.
    fn kept(&self) -> Kept {
        let found = self.found.load();
        let ran = self.ran.load(std::sync::atomic::Ordering::Relaxed);
        (Arc::clone(&self.order), found.per_node, found.explain.clone(), found.gap, self.priced.lock().clone(), (ran != NOT_RAN).then(|| f64::from_bits(ran)))
    }

    /// Takes `k` as this plan's where its order is this one's (else false: a new order is a new entry).
    fn set(&self, k: &Kept) -> bool {
        let same = Arc::ptr_eq(&self.order, &k.0) || *self.order == *k.0;
        if same {
            self.found.store(Arc::new(Found { per_node: k.1, explain: k.2.clone(), gap: k.3 }));
            let old = std::mem::replace(&mut *self.priced.lock(), k.4.clone());
            drop(old);
            self.ran.store(k.5.map_or(NOT_RAN, f64::to_bits), std::sync::atomic::Ordering::Relaxed);
        }
        same
    }
}

/// The run's plans by key, as published (`RunPlans::kept`): in shards by the key's hash, each shared, so a
/// publication copies the shards its plans fall in, not every plan (a run publishes on every stratum's entry that
/// planned anew: on 50 files of 317 programs, copying the whole map each time was the run's largest cost).
#[derive(Clone)]
struct Plans {
    shards: [Arc<rustc_hash::FxHashMap<PlanKey, Arc<Entry>>>; PLAN_SHARDS],
}

const PLAN_SHARDS: usize = 64;

impl Default for Plans {
    fn default() -> Self {
        Plans { shards: std::array::from_fn(|_| Arc::default()) }
    }
}

impl Plans {
    fn shard(key: &PlanKey) -> usize {
        use std::hash::BuildHasher;
        (rustc_hash::FxBuildHasher.hash_one(key) % PLAN_SHARDS as u64) as usize
    }
    fn get(&self, key: &PlanKey) -> Option<&Arc<Entry>> {
        self.shards[Self::shard(key)].get(key)
    }
    fn iter(&self) -> impl Iterator<Item = (&PlanKey, &Arc<Entry>)> {
        self.shards.iter().flat_map(|m| m.iter())
    }
    /// Adds `entries`, copying each shard they fall in once (while an earlier map shares it).
    fn extend(&mut self, entries: impl IntoIterator<Item = (PlanKey, Arc<Entry>)>) {
        for (k, e) in entries {
            Arc::make_mut(&mut self.shards[Self::shard(&k)]).insert(k, e);
        }
    }
}

impl FromIterator<(PlanKey, Arc<Entry>)> for Plans {
    fn from_iter<I: IntoIterator<Item = (PlanKey, Arc<Entry>)>>(entries: I) -> Self {
        let mut plans = Plans::default();
        plans.extend(entries);
        plans
    }
}

/// Publications of new plans and of new orders (decision 0301), and the entries they carried: for the profile.
static PUBLISHED: [std::sync::atomic::AtomicU64; 2] = [const { std::sync::atomic::AtomicU64::new(0) }; 2];

/// A run's plans: those it made or took (`kept`), and those the store kept from earlier processes (`stored`,
/// decision 0116 step 2), taken into `kept` when a rule is first planned. Decision 0301: each part shared as it is
/// used. `kept` is read with no lock (RCU: an immutable map, a new one published for new plans), its entries'
/// learning in cells; the lines run and the keys planned are collected apart, under locks no planner reads under.
pub struct RunPlans {
    kept: arc_swap::ArcSwap<Plans>,
    stored: arc_swap::ArcSwap<StoredPlans>,
    /// The lines run since the last critical path (decision 0203 §4): each stratum's sent as it ran (no lock),
    /// received only by `lines_take`.
    lines: (std::sync::mpsc::Sender<Vec<joins::LineRun>>, parking_lot::Mutex<std::sync::mpsc::Receiver<Vec<joins::LineRun>>>),
    /// The keys planned in this run (profiled), for the profile's account of the planning budget's misses: the run's,
    /// shared with its jobs as the plans are.
    planned_now: parking_lot::Mutex<rustc_hash::FxHashSet<PlanKey>>,
    /// The last order kept for each rule line (its body and delta, `line_key`), whatever component solved it: a line
    /// seen first under another key, or a maintenance rule of a rule planned before, starts from it (decision 0258).
    /// A session's, carried between its runs with its plans.
    orders: arc_swap::ArcSwap<rustc_hash::FxHashMap<u128, Arc<[Step]>>>,
    /// The prepared plans of the versions planned (decision 0361 step 5, Q228): the store's, read with the kept
    /// plans, and those this run made, by the version's identity; and whether it made any.
    prepared: arc_swap::ArcSwap<prepared::ByVersion>,
    prepared_new: std::sync::atomic::AtomicBool,
}

impl Default for RunPlans {
    fn default() -> Self {
        RunPlans::new(Default::default(), Default::default(), Default::default())
    }
}

/// Decision 0258: a line's key apart from what solves it: its body and its delta.
#[doc(hidden)]
pub fn line_key(b: &Body<'_>, first: Option<usize>) -> u128 {
    super::derived::digest(|d| {
        d.str("line");
        d.u64(b.key() as u64);
        d.u64((b.key() >> 64) as u64);
        d.u64(first.map_or(u64::MAX, |f| f as u64));
    })
}

/// The order last kept for line `line` (`RunPlans::orders`).
fn orders_get(line: u128) -> Option<Arc<[Step]>> {
    PLANS.with(|p| p.borrow().as_ref().and_then(|s| s.orders.load().get(&line).cloned()))
}

/// Keeps `order` as line `line`'s last (`RunPlans::orders`).
fn orders_put(line: u128, order: &[Step]) {
    PLANS.with(|p| {
        if let Some(s) = p.borrow().as_ref() {
            let order: Arc<[Step]> = order.into();
            s.orders.rcu(|o| {
                let mut o = rustc_hash::FxHashMap::clone(o);
                o.insert(line, Arc::clone(&order));
                o
            });
        }
    });
}

/// Decision 0258: a line seen first starts from an order kept before (its own under another key, or its source
/// rule's), under `RERAIL_SOURCE_ORDERS=on`. Off by default: measured, it costs more planning than it saves (finding
/// the order is most of it, and greedy-first already takes a cheap order under a search's cost), for the same plans.
fn source_orders() -> bool {
    static ON: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var("RERAIL_SOURCE_ORDERS").is_ok_and(|v| v == "on"));
    *ON
}

/// Whether `key` was planned in this thread's run (`RunPlans::planned_now`).
fn planned_now(key: &PlanKey) -> bool {
    OWN_PLANNED.with(|o| o.borrow().as_ref().is_some_and(|o| o.contains(key)))
        || PLANS.with(|p| p.borrow().as_ref().is_some_and(|s| s.planned_now.lock().contains(key)))
}

/// Notes `key` as planned in this run (`planned_now`): in this thread's job's set, else the run's.
fn note_planned(key: PlanKey) {
    let key = OWN_PLANNED.with(|o| match o.borrow_mut().as_mut() {
        Some(o) => {
            o.insert(key);
            None
        }
        None => Some(key),
    });
    if let Some(key) = key {
        PLANS.with(|p| {
            if let Some(s) = p.borrow().as_ref() {
                s.planned_now.lock().insert(key);
            }
        });
    }
}

impl RunPlans {
    fn new(kept: rustc_hash::FxHashMap<PlanKey, Kept>, orders: rustc_hash::FxHashMap<u128, Arc<[Step]>>, stored: Arc<StoredPlans>) -> Self {
        RunPlans {
            kept: arc_swap::ArcSwap::from_pointee(kept.into_iter().map(|(k, v)| (k, Entry::of(v))).collect()),
            stored: arc_swap::ArcSwap::new(stored),
            lines: {
                let (tx, rx) = std::sync::mpsc::channel();
                (tx, parking_lot::Mutex::new(rx))
            },
            planned_now: Default::default(),
            orders: arc_swap::ArcSwap::from_pointee(orders),
            prepared: Default::default(),
            prepared_new: Default::default(),
        }
    }

    /// Tests: plans with `stored` as the store's.
    #[cfg(test)]
    pub(in crate::code_rule_dsl) fn with_stored(stored: Arc<StoredPlans>) -> Self {
        RunPlans::new(Default::default(), Default::default(), stored)
    }

    /// Publishes `plans`, new keys or new orders (decision 0301): a new map, the old one's entries shared.
    fn publish(&self, plans: Vec<(PlanKey, Kept)>) {
        if plans.is_empty() {
            return;
        }
        let entries: Vec<(PlanKey, Arc<Entry>)> = plans.into_iter().map(|(k, v)| (k, Entry::of(v))).collect();
        PUBLISHED[0].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        PUBLISHED[1].fetch_add(entries.len() as u64, std::sync::atomic::Ordering::Relaxed);
        self.kept.rcu(|m| {
            let mut m = Plans::clone(m);
            m.extend(entries.iter().map(|(k, e)| (*k, Arc::clone(e))));
            m
        });
    }

    /// The run's plans as `Kept`, and its orders: a session keeps them for its next run.
    fn into_kept(self) -> (rustc_hash::FxHashMap<PlanKey, Kept>, rustc_hash::FxHashMap<u128, Arc<[Step]>>) {
        let kept = self.kept.load().iter().map(|(k, e)| (*k, e.kept())).collect();
        (kept, rustc_hash::FxHashMap::clone(&self.orders.load()))
    }

    /// The plan the store kept for `key` at the scale of `nodes`, else at the nearest scale stored (O-replan-churn:
    /// an edit's chunk and the cross part are a bucket or two apart, and a plan of the other's is re-priced before
    /// it is kept, so a plan from another scale is a better start than a search from nothing).
    fn stored_near(&self, key: &PlanKey, len: usize, nodes: f64) -> Option<Kept> {
        let at = scale(nodes);
        let stored = self.stored.load();
        let found = (0..=12u8).find_map(|d| {
            [at.checked_sub(d), at.checked_add(d)].into_iter().flatten().find_map(|s| stored.get(&(*key, s)))
        })?;
        found.kept(len)
    }

    /// The plan kept for `key`: the run's, else the store's at the scale of `nodes` or the nearest stored, its steps
    /// taken from `body` by position (the key digests the body, so its literals are the planned ones).
    #[cfg(test)]
    fn find(&self, key: &PlanKey, len: usize, nodes: f64) -> Option<Kept> {
        if let Some(e) = self.kept.load().get(key) {
            return Some(e.kept());
        }
        let k = self.stored_near(key, len, nodes)?;
        self.publish(vec![(*key, k.clone())]);
        Some(k)
    }
}

/// A step as the store keeps it: its position in the body, and its mode.
type StoredStep = (u16, Option<Mode>);

/// A kept plan as the store keeps it across processes (decision 0116 step 2): `Kept` without its literals (they
/// are the body's, by position) or its EXPLAIN. Plans are kept per prepared program, under the engine's build.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredPlan {
    order: Vec<StoredStep>,
    per_node: f64,
    gap: f64,
    /// The last price: the sizes it read, its order, work and rows.
    priced: Option<(Vec<f64>, Vec<StoredStep>, f64, f64)>,
    /// The work its rule's line did when it last ran (decision 0122), kept with a price: a new process's planning
    /// budget reads it as a session's does.
    ran: Option<f64>,
}

/// The plans the store keeps for a prepared program, by plan key and the scale they were priced at (`scale`): an
/// edit's run plans its edited file's part at that file's scale, and a full run at the corpus's, and each keeps its
/// own (a plan priced over 20 files is not near one priced over one, so each would search the other's again).
pub type StoredPlans = rustc_hash::FxHashMap<(PlanKey, u8), StoredPlan>;

/// A scale's bucket: the corpus's nodes, by powers of 4.
fn scale(nodes: f64) -> u8 {
    (nodes.max(1.0).log2() / 2.0) as u8
}

impl StoredPlan {
    /// Whether `other` need not be written over this one: the same but for the work its line last ran (`ran`), where
    /// that is within 2x (the budget compares it with a search's cost, so an edit's few rows more or less are the
    /// same answer, and every plan is not written again each run).
    fn same(&self, other: &StoredPlan) -> bool {
        let ran = match (self.ran, other.ran) {
            (None, None) => true,
            (Some(a), Some(b)) => a.max(1.0) <= 2.0 * b.max(1.0) && b.max(1.0) <= 2.0 * a.max(1.0),
            _ => false,
        };
        ran && self.order == other.order && self.per_node == other.per_node && self.gap == other.gap && self.priced == other.priced
    }

    fn of(k: &Kept) -> Option<StoredPlan> {
        let steps = |order: &[Step]| order.iter().map(|s| Some((u16::try_from(s.at).ok().filter(|&a| a < 1 << 14)?, s.mode))).collect::<Option<Vec<_>>>();
        let priced = match &k.4 {
            Some(p) => Some((p.sizes.clone(), steps(&p.got.0.0)?, p.got.0.1, p.got.0.2)),
            None => None,
        };
        Some(StoredPlan { order: steps(&k.0)?, per_node: k.1, gap: k.3, ran: k.5.filter(|_| priced.is_some()), priced })
    }

    fn kept(&self, len: usize) -> Option<Kept> {
        let steps = |order: &[StoredStep]| order.iter().map(|&(at, mode)| (usize::from(at) < len).then_some(Step { at: at.into(), mode })).collect::<Option<Vec<_>>>();
        let priced = match &self.priced {
            Some((sizes, order, work, rows)) => Some(Arc::new(Priced { sizes: sizes.clone(), got: ((steps(order)?, *work, *rows), None) })),
            None => None,
        };
        Some((steps(&self.order)?.into(), self.per_node, None, self.gap, priced, self.ran))
    }

    /// As integers, one row per plan: `[key high, key low, scale, per node, gap, steps, the steps four to an
    /// integer, then when priced: work, rows, sizes, the sizes, steps, the steps, and when it ran: its work]` (floats
    /// by their bits; a row without the last reads as not run).
    pub fn row((key, scale): (PlanKey, u8), p: &StoredPlan) -> Vec<i64> {
        let mut row = vec![(key >> 64) as i64, key as i64, i64::from(scale), p.per_node.to_bits() as i64, p.gap.to_bits() as i64];
        let steps = |row: &mut Vec<i64>, order: &[StoredStep]| {
            row.push(order.len() as i64);
            for four in order.chunks(4) {
                let mut x = 0u64;
                for (i, &(at, mode)) in four.iter().enumerate() {
                    let m = match mode { None => 0u64, Some(Mode::Range) => 1, Some(Mode::Climb) => 2, Some(Mode::Descend) => 3 };
                    x |= (u64::from(at) << 2 | m) << (16 * i);
                }
                row.push(x as i64);
            }
        };
        steps(&mut row, &p.order);
        if let Some((sizes, order, work, rows)) = &p.priced {
            row.push(work.to_bits() as i64);
            row.push(rows.to_bits() as i64);
            row.push(sizes.len() as i64);
            row.extend(sizes.iter().map(|s| s.to_bits() as i64));
            steps(&mut row, order);
            if let Some(ran) = p.ran {
                row.push(ran.to_bits() as i64);
            }
        }
        row
    }

    /// A row `row` made, or `None` where it is not one.
    pub fn of_row(row: &[i64]) -> Option<((PlanKey, u8), StoredPlan)> {
        let mut at = row.iter().copied();
        let mut next = || at.next();
        let key = ((next()? as u64 as u128) << 64 | next()? as u64 as u128, u8::try_from(next()?).ok()?);
        let (per_node, gap) = (f64::from_bits(next()? as u64), f64::from_bits(next()? as u64));
        let steps = |next: &mut dyn FnMut() -> Option<i64>| -> Option<Vec<StoredStep>> {
            let n = usize::try_from(next()?).ok()?;
            let mut out = Vec::with_capacity(n.min(1 << 10));
            while out.len() < n {
                let x = next()? as u64;
                for i in 0..4.min(n - out.len()) {
                    let v = (x >> (16 * i)) & 0xffff;
                    let mode = match v & 3 { 0 => None, 1 => Some(Mode::Range), 2 => Some(Mode::Climb), _ => Some(Mode::Descend) };
                    out.push(((v >> 2) as u16, mode));
                }
            }
            Some(out)
        };
        let order = steps(&mut next)?;
        let (priced, ran) = match next() {
            None => (None, None),
            Some(work) => {
                let rows = f64::from_bits(next()? as u64);
                let n = usize::try_from(next()?).ok()?;
                let sizes = (0..n).map(|_| next().map(|b| f64::from_bits(b as u64))).collect::<Option<Vec<_>>>()?;
                let order = steps(&mut next)?;
                (Some((sizes, order, f64::from_bits(work as u64), rows)), next().map(|b| f64::from_bits(b as u64)))
            }
        };
        Some((key, StoredPlan { order, per_node, gap, priced, ran }))
    }
}

/// Gives this thread's run the plans the store kept (`plan::choices`, before the run's first wave).
pub fn take_stored_plans(stored: Arc<StoredPlans>) {
    PLANS.with(|p| {
        if let Some(p) = p.borrow().as_ref() {
            p.stored.store(stored);
        }
    });
}

/// Whether this thread runs with kept plans (`PLANS`): only then are prepared plans read from or kept for the store.
#[doc(hidden)]
pub fn run_holds_plans() -> bool {
    PLANS.with(|p| p.borrow().is_some())
}

/// The prepared plans the store kept of the version whose identity is `id` (Q228).
#[doc(hidden)]
pub fn prepared_stored(id: &[u8; 32]) -> Option<Arc<prepared::Prepared>> {
    PLANS.with(|p| p.borrow().as_ref().and_then(|p| p.prepared.load().get(id).cloned()))
}

/// Keeps `made`, the prepared plans this run made of the version whose identity is `id`, for the store. A version
/// with no prepared line is held but not new: the store keeps a row a line (`prepared::store_rows`), so it has none
/// of it, and every run makes it again; counted new, a warm run gathered a write of the kept plans as read
/// (`the_run_after_a_cold_one_writes_nothing`, Q243).
#[doc(hidden)]
pub fn prepared_made(id: [u8; 32], made: Arc<prepared::Prepared>) {
    let new = !made.0.is_empty();
    PLANS.with(|p| {
        if let Some(p) = p.borrow().as_ref() {
            p.prepared.rcu(|m| {
                let mut m = prepared::ByVersion::clone(m);
                m.insert(id, Arc::clone(&made));
                m
            });
            if new {
                p.prepared_new.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
    });
}

/// Gives this thread's run the prepared plans the store kept (with `take_stored_plans`).
pub fn take_stored_prepared(stored: prepared::ByVersion) {
    PLANS.with(|p| {
        if let Some(p) = p.borrow().as_ref() {
            p.prepared.store(Arc::new(stored));
        }
    });
}

/// This thread's run's prepared plans for the store, the stored with the made: `None` where it made none.
pub fn prepared_to_store() -> Option<Arc<prepared::ByVersion>> {
    PLANS.with(|p| p.borrow().as_ref().filter(|p| p.prepared_new.load(std::sync::atomic::Ordering::Relaxed)).map(|p| p.prepared.load_full()))
}

/// The prepared plans held for this thread's run, as read and made.
pub fn prepared_held() -> Arc<prepared::ByVersion> {
    PLANS.with(|p| p.borrow().as_ref().map(|p| p.prepared.load_full()).unwrap_or_default())
}

/// This thread's run's plans for the store: the stored ones with the run's over them, or `None` where the run
/// changed none.
pub fn plans_to_store() -> Option<StoredPlans> {
    let plans = PLANS.with(|p| p.borrow().clone())?;
    let (kept, stored) = (plans.kept.load(), plans.stored.load());
    let mut out: Option<StoredPlans> = None;
    for (key, e) in kept.iter() {
        let k = &e.kept();
        // Its scale is the nodes its last price read (`read_sizes` and `live_sizes` start with them): a plan never
        // priced is not kept.
        let Some(nodes) = k.4.as_ref().and_then(|p| p.sizes.first().copied()) else { continue };
        let Some(s) = StoredPlan::of(k) else { continue };
        let key = (*key, scale(nodes));
        if !stored.get(&key).is_some_and(|t| t.same(&s)) {
            out.get_or_insert_with(|| StoredPlans::clone(&stored)).insert(key, s);
        }
    }
    out
}

/// `RERAIL_JOIN_LINES=FILE` (a measurement aid): each line that ran a search's work or more, appended as it ends,
/// tab-separated: its stratum, its delta, its order (literal positions), its rows of work, its price for the rows it
/// read, its wall seconds, its plan key and its rule.
fn line_log() -> Option<&'static parking_lot::Mutex<std::fs::File>> {
    static LOG: std::sync::LazyLock<Option<parking_lot::Mutex<std::fs::File>>> = std::sync::LazyLock::new(|| {
        let p = std::env::var_os("RERAIL_JOIN_LINES")?;
        std::fs::OpenOptions::new().create(true).append(true).open(p).ok().map(parking_lot::Mutex::new)
    });
    LOG.as_ref()
}

/// Adds a stratum's lines as they ran to the run's (decision 0203 §4): sent to its log (decision 0301: a channel,
/// whose sends take no lock, that only `lines_take` reads).
fn lines_push(runs: Vec<joins::LineRun>) {
    if runs.is_empty() {
        return;
    }
    PLANS.with(|p| {
        if let Some(p) = p.borrow().as_ref() {
            let _ = p.lines.0.send(runs);
        }
    });
}

/// The lines run since the last call.
fn lines_take() -> Vec<joins::LineRun> {
    PLANS.with(|p| p.borrow().as_ref().map(|p| p.lines.1.lock().try_iter().flatten().collect()).unwrap_or_default())
}

/// The plans the store kept for this thread's run, as read (the join memo's write, where no plan changed).
pub fn stored_plans() -> Option<StoredPlans> {
    PLANS.with(|p| p.borrow().as_ref().map(|p| StoredPlans::clone(&p.stored.load())))
}

/// This thread's kept plans, for a job on another thread to enter.
fn share_plans() -> Option<SharedPlans> {
    PLANS.with(|p| p.borrow().clone())
}

/// While the guard lives, this thread plans with `plans` (another thread's, `share_plans`), into plans of its own
/// (`OWN_PLANS`: new keys and new orders) published as it ends, its keys planned handed on then too.
fn enter_plans(plans: Option<SharedPlans>) -> impl Drop {
    type Own = (Option<rustc_hash::FxHashMap<PlanKey, Kept>>, Option<rustc_hash::FxHashSet<PlanKey>>);
    struct Entered(Option<SharedPlans>, Own);
    impl Drop for Entered {
        fn drop(&mut self) {
            let own = OWN_PLANS.with(|o| std::mem::replace(&mut *o.borrow_mut(), self.1.0.take()));
            let planned = OWN_PLANNED.with(|o| std::mem::replace(&mut *o.borrow_mut(), self.1.1.take()));
            let entered = PLANS.with(|p| std::mem::replace(&mut *p.borrow_mut(), self.0.take()));
            let Some(shared) = entered else { return };
            // In a chunk's view (Q88), the job's plans are the chunk's own, its entries.
            if let Some(v) = &shared.view {
                let mut own: Vec<(PlanKey, Kept)> = own.into_iter().flatten().collect();
                own.sort_unstable_by_key(|(k, _)| *k);
                v.put(own);
                if let Some(planned) = planned.filter(|p| !p.is_empty()) {
                    shared.planned_now.lock().extend(planned);
                }
                return;
            }
            // Decision 0301: the job's plans whose order the run holds go to their entries' cells; the others, new
            // keys or new orders, are published together.
            let new: Vec<(PlanKey, Kept)> = {
                let kept = shared.kept.load();
                own.into_iter().flatten().filter(|(k, v)| !kept.get(k).is_some_and(|e| e.set(v))).collect()
            };
            shared.publish(new);
            if let Some(planned) = planned.filter(|p| !p.is_empty()) {
                shared.planned_now.lock().extend(planned);
            }
        }
    }
    let own = plans.is_some().then(rustc_hash::FxHashMap::default);
    let planned = plans.is_some().then(rustc_hash::FxHashSet::default);
    Entered(
        PLANS.with(|p| std::mem::replace(&mut *p.borrow_mut(), plans)),
        (
            OWN_PLANS.with(|o| std::mem::replace(&mut *o.borrow_mut(), own)),
            OWN_PLANNED.with(|o| std::mem::replace(&mut *o.borrow_mut(), planned)),
        ),
    )
}

/// A kept plan's last price: the corpus's nodes and the rows of each relation its body reads when priced, and the
/// price. A price reads statistics only; while no size moved over `PRICE_DRIFT` times it is taken as it is, not
/// worked out again (decision 0090 step 5: a session's run re-priced 7,400 kept plans, two thirds of its CPU).
pub struct Priced {
    sizes: Vec<f64>,
    got: (Plan, Option<super::profile::RulePlan>),
}

/// How far a size may move before a kept price is worked out again.
pub const PRICE_DRIFT: f64 = 2.0;

/// Whether sizes `now` are within `PRICE_DRIFT` of `then`, each counted as at least one row.
/// Whether a kept plan's distributions (`live_sizes` after its cardinalities) are near this pricing's, at another
/// scale: an entry is near as it was (a column of few values, as `kind`'s) or in proportion to the nodes (a column
/// of node ids, whose distinct values grow with the corpus). The last `literals` entries, a bit per literal, are
/// compared as they are. A rule planned for a chunk of 22,146 nodes and for the cross part's 61,997 compared its
/// distributions as they were, so each scale replanned what the other kept, every run (O-replan-churn).
pub fn near_distributions(then: &[f64], now: &[f64], scale: f64, literals: usize) -> bool {
    let bits = then.len().saturating_sub(literals);
    then.len() == now.len()
        && then.iter().zip(now).enumerate().all(|(i, (a, b))| {
            let one = |a: f64| near(&[a], &[*b]);
            one(*a) || (i < bits && *a > 0.0 && *b > 0.0 && one(a * scale))
        })
}

pub fn near(then: &[f64], now: &[f64]) -> bool {
    then.len() == now.len()
        && then.iter().zip(now).all(|(a, b)| {
            // Negative signatures distinguish unknown and proven-empty constant postings.
            if *a < 0.0 || *b < 0.0 { return a == b; }
            // An empty relation is not near a filled one: a plan priced where it was empty takes any cross product
            // after it as free (`text` crossed after an empty `rr_ast_5_1`: 6,240 -> 115M rows when it held one).
            if (*a == 0.0) != (*b == 0.0) { return false; }
            let (a, b) = (a.max(1.0), b.max(1.0));
            a <= b * PRICE_DRIFT && b <= a * PRICE_DRIFT
        })
}

/// Plans kept across chunks on this thread while held (a corpus run), none kept at the start; for a session's run
/// (`across`, the session's generation), the plans its earlier runs kept, put back at the end (decision 0090 step 5).
pub struct KeepPlans(Option<u64>);

thread_local! {
    /// A session's plans between its runs, with the session's generation: another session's are not taken.
    static SESSION_PLANS: std::cell::RefCell<Option<(u64, rustc_hash::FxHashMap<PlanKey, Kept>, rustc_hash::FxHashMap<u128, Arc<[Step]>>)>> = const { std::cell::RefCell::new(None) };
}

impl KeepPlans {
    pub fn start(across: Option<u64>) -> Self {
        compose::clear();
        let (kept, orders) = across.and_then(|g| SESSION_PLANS.with(|s| s.borrow_mut().take().filter(|(k, _, _)| *k == g).map(|(_, p, o)| (p, o)))).unwrap_or_default();
        PLANS.with(|p| *p.borrow_mut() = Some(SharedPlans { run: Arc::new(RunPlans::new(kept, orders, Default::default())), view: None }));
        keeps::observing(true);
        KeepPlans(across)
    }
}

impl Drop for KeepPlans {
    fn drop(&mut self) {
        if std::env::var_os("RERAIL_PLAN_CACHE_COUNTS").is_some() {
            let counts: Vec<String> = PLAN_CACHE.iter().zip(PLAN_CACHE_NAMES).map(|(c, n)| format!("{n} {}", c.swap(0, std::sync::atomic::Ordering::Relaxed))).collect();
            let miss: Vec<String> = BUDGET_MISS.iter().zip(["no price", "never ran", "ran over a search", "inputs grew"]).map(|(c, n)| format!("{n} {}", c.swap(0, std::sync::atomic::Ordering::Relaxed))).collect();
            let st: Vec<u64> = STATISTICS.iter().map(|c| c.swap(0, std::sync::atomic::Ordering::Relaxed)).collect();
            let searches: Vec<u64> = SEARCHES.iter().map(|c| c.swap(0, std::sync::atomic::Ordering::Relaxed)).collect();
            let lazy = lazy_probes_counted();
            eprintln!("plan cache: {}; budget missed: {}; statistics: {} batches of {} relations, {} late, {} answered by layouts; {} lines prepared ({} versions' plans read, {} made); orders: {} searched, {} greedy (under a search's cost); probes: {} left unplanned at open, {} planned later", counts.join(", "), miss.join(", "), st[0], st[1], st[2], SHAPED.swap(0, std::sync::atomic::Ordering::Relaxed), prepared::TAKEN.swap(0, std::sync::atomic::Ordering::Relaxed), prepared::VERSIONS[0].swap(0, std::sync::atomic::Ordering::Relaxed), prepared::VERSIONS[1].swap(0, std::sync::atomic::Ordering::Relaxed), searches[0], searches[1], lazy.0, lazy.1);
        }
        compose::clear();
        // Every job that entered them has ended: the run's thread holds the last handle.
        let plans = PLANS.with(|p| p.borrow_mut().take());
        keeps::observing(false);
        let published = PUBLISHED.iter().map(|c| c.swap(0, std::sync::atomic::Ordering::Relaxed)).collect::<Vec<_>>();
        let folds = keeps::FOLDS.swap(0, std::sync::atomic::Ordering::Relaxed);
        let deferred = super::native::setfix::DEFERRED_WALKS.iter().map(|c| c.swap(0, std::sync::atomic::Ordering::Relaxed)).collect::<Vec<_>>();
        super::profile::decision("a walk's new texts interned at once (decision 0315)", || format!("{} walks in one piece interned {} new texts, one write of the arena each", deferred[0], deferred[1]));
        super::profile::decision("kept plans shared by RCU (decision 0301)", || format!("{} publications of {} new plans or orders; observed sums folded {folds} times", published[0], published[1]));
        if let (Some(g), Some(plans)) = (self.0, plans) {
            // Every job that entered them has ended: the run's thread holds the last handle, or a copy is taken.
            let (kept, orders) = match Arc::try_unwrap(plans.run) {
                Ok(p) => p.into_kept(),
                Err(p) => (p.kept.load().iter().map(|(k, e)| (*k, e.kept())).collect(), rustc_hash::FxHashMap::clone(&p.orders.load())),
            };
            SESSION_PLANS.with(|s| *s.borrow_mut() = Some((g, kept, orders)));
        }
    }
}

thread_local! {
    /// Whether a plan is explained step by step whether or not the run is profiled (`explaining`): the choice of
    /// what a relation keeps re-prices each step of its plans (decision 0099).
    static EXPLAIN_ALWAYS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Plans explained step by step while `on` (`EXPLAIN_ALWAYS`), the setting before returned.
pub fn explaining(on: bool) -> bool {
    EXPLAIN_ALWAYS.with(|e| e.replace(on))
}

thread_local! {
    /// Set while a plan's record is made for its steps' rows alone (`Model::plan_rule_step_rows`).
    static STEP_ROWS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

thread_local! {
    /// Whether the planner reads its statistics skewed (`Model::plan_inner`, `RERAIL_STATS_SKEW`).
    static SKEWING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Carries the run's kept plans and the planner's settings to a job on another thread (`run_state`): the plans are
/// shared (what the job plans is taken into them as it ends, `enter_plans`), the settings copied.
pub(in crate::code_rule_dsl) fn carried(c: &mut super::run_state::Capture) {
    c.part(&["PLANS", "OWN_PLANS", "OWN_PLANNED"], super::run_state::Shared(share_plans(), enter_plans));
    c.copy(&["PLAN_SITE"], &PLAN_SITE);
    c.copy(&["EXPLAIN_ALWAYS"], &EXPLAIN_ALWAYS);
    c.copy(&["NO_SHAPES"], &NO_SHAPES);
    c.copy(&["SKEWING"], &SKEWING);
    c.copy(&["STEP_ROWS"], &STEP_ROWS);
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
pub type Plan = (Vec<Step>, f64, f64);

impl Model {
    /// The logical key and scalar analysis (`analysis`) of `v`, the program this model plans, made the first time
    /// it is asked for.
    pub fn analysis(&self, v: &Version) -> std::sync::Arc<super::analysis::ProgramAnalysis> {
        std::sync::Arc::clone(self.analysis.get_or_init(|| {
            let _analysis = super::profile::span(|| "logical key and scalar analysis".into());
            let analysis = program_analysis(v);
            let _explain = super::profile::span(|| "explain analysis".into());
            analysis.explain();
            analysis
        }))
    }

    /// `analysis` of program `p`.
    pub fn analysis_of(&self, p: &body::Placed) -> Result<std::sync::Arc<super::analysis::ProgramAnalysis>> {
        Ok(self.analysis(&p.v))
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
            // Any relation empty when it plans is not empty where its plans are reused, for the same reason: at least
            // one row. At 0, every step after it was priced 0 rows and every order was free: annotation demand's
            // `rel_base` planned before its guest round made `rr_ast_1_1`, scanned `pkg` whole and then probed
            // `docof` (10,728 -> 25.8 M -> 10,728 rows); `impmod`, `ruff_qual` and `frag_str` the same.
            let card = self.stats.card.get(rel).copied();
            if card.is_none() && (rel.ends_with("__plus") || rel.ends_with("__minus")) && !self.stats.lazy.0.contains_key(rel) {
                UNSIZED_DELTAS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            card.unwrap_or(1000.0).max(1.0)
        }
    }

    /// Cardinality of a kind posting in the population currently being planned.
    pub fn kind_frequency(&self, kind: &Constant) -> Option<i64> {
        // A complete current histogram is authoritative, including absent values.
        // Sampled corpus priors describe a different population and are a fallback.
        if let Some(freq) = self.stats.dfreq.at("kind", 1) {
            return Some(freq.get(kind).copied().unwrap_or(0));
        }
        let key = match kind { Constant::Str(s) => s.clone(), Constant::Int(i) => i.to_string() };
        self.stats.kfreq.get(&key).copied()
    }

    /// Expected postings for a literal value whose frequency has not been measured.
    pub fn unknown_constant_rows(&self, rel:&str, column:usize)->f64 {
        // A literal key is not sampled from this relation's rows. sq / rows
        // overweights hot keys; the value-uniform prior is rows / distinct.
        self.card(rel)/self.ndv_or_default(rel,column)
    }

    /// Column `c` of `rel`'s heavy values (decision 0209 §4), where they were made over its rows as they are now
    /// (within a tenth): a relation still filling, or changed since, is priced from its sketches.
    pub fn heavy_of(&self, rel: &str, c: usize) -> Option<&table::HeavyCol> {
        let h = self.stats.heavy.at(rel, c)?;
        let card = self.stats.card.get(rel).copied()?;
        ((h.rows - card).abs() <= 0.1 * card.max(1.0)).then_some(&**h)
    }

    /// Column `c` of `rel`'s distinct values: measured, else one; for a relation of the component being solved,
    /// which no statistics have seen yet, PostgreSQL's prior for a column it has not analyzed
    /// (`DEFAULT_NUM_DISTINCT`, 200 values, at most the relation's rows). Taken as one value, a lookup of such a
    /// relation by a column was priced at the whole relation a key: `p188_parsed_within`'s self-join by `$p` on
    /// hogql, 4,877 rows a key planned and none made, its stratum planned at 2.6 G units for 0.7 M done (Q44). Other
    /// relations keep one: what is kept or inlined (`keeps`) is chosen on these estimates, and the prior there
    /// inlined kept `pre_` relations (`rr_ctx282` read one at 0.8 M units, and inlined did 39 M).
    fn ndv_or_default(&self, rel: &str, c: usize) -> f64 {
        const DEFAULT_NUM_DISTINCT: f64 = 200.0;
        // Unmeasured: its layout's estimate (decision 0361 step 4), else the prior.
        let unseen = || {
            self.stats.shapes.ndv(rel, c, self.card(rel), self.stats.counts()).unwrap_or_else(|| if self.stats.solving.contains(rel) { DEFAULT_NUM_DISTINCT.min(self.card(rel)) } else { 1.0 })
        };
        let ndv = self.ndv.at(&rel, c).copied().unwrap_or_else(unseen);
        (ndv * skew(rel, c, 0)).max(1.0)
    }

    /// Bound a binding's distinct values by measured rows surviving its constant filters. The full column's
    /// domain is too wide after, for example, `kind(n, function_definition)`: those bindings are functions,
    /// not random nodes. Treating them as random misses underprices expansion from each function to its body.
    pub fn binding_domains(&self, b: &Body<'_>, i: usize) -> Vec<Option<f64>> {
        let rel = b.rel(b.atom(i).expect("an atom"));
        let mut cap = self.stats.card.get(rel).copied().unwrap_or(f64::INFINITY);
        for (c, t) in b.args(i).iter().enumerate() {
            let Some(k) = b.constant(*t) else { continue };
            let measured = if rel == "kind" && c == 1 {
                self.kind_frequency(&k).map(|n| n as f64)
            } else if let Some(freq) = self.stats.dfreq.at(&rel, c) {
                Some(freq.get(&k).copied().unwrap_or(0) as f64)
            } else {
                self.stats.memo.get(rel).and_then(|m| m.get(&(self.col(rel, c), k))).map(|n| *n as f64)
            };
            if let Some(n) = measured { cap = cap.min(n); }
        }
        // `kind($s, $k)` with `$k` read by an atom whose values there are counted (`scope($k)`'s four kinds): `$s`
        // ranges over those kinds' nodes (decision 0299). As every node, a probe by `$s` of a relation of scopes
        // (`rr_nearest_ks10($t, $s)`) was priced as if most scopes missed, and `tscope` read every scope's nodes
        // (1.7 M rows for 0.07 M).
        if rel == "kind"
            && let Some(TermRef::Var(k)) = b.args(i).get(1).copied()
            && let Some(n) = (0..b.len()).filter(|&j| j != i).find_map(|j| {
                let o = b.plain_atom(j).filter(|o| !o.neg())?;
                let c = b.args(j).iter().position(|t| matches!(t, TermRef::Var(x) if *x == k))?;
                let values = self.stats.dfreq.at(b.rel(o), c)?;
                values.keys().map(|v| self.kind_frequency(v).map(|n| n as f64)).sum::<Option<f64>>()
            })
        {
            cap = cap.min(n.max(1.0));
        }
        (0..b.args(i).len()).map(|c| self.ndv.at(&rel, c).map(|n| n.min(cap))).collect()
    }

    /// Complete small value sets already in statistics. SQL can load only the constants requested by a rule,
    /// so an absent memo entry proves nothing unless both its distinct count and total cover the column.
    fn small_values(&self, rel: &str, c: usize) -> Option<Vec<&Constant>> {
        if let Some(freq) = self.stats.dfreq.at(&rel, c) {
            return Some(freq.iter().filter(|(_, n)| **n > 0).map(|(v, _)| v).collect());
        }
        if self.stats.solving.contains(rel) { return None; }
        let col = self.col(rel, c);
        let [rows, ndv, _, _] = *self.stats.col.get(&(rel.to_owned(), col.clone()))?;
        if ndv > SMALL_NDV { return None; }
        let values: Vec<_> = self.stats.memo.get(rel)?.iter().filter(|((name, _), n)| *name == col && **n > 0).collect();
        if values.len() as i64 != ndv || values.iter().map(|(_, n)| **n).sum::<i64>() != rows { return None; }
        Some(values.into_iter().map(|((_, v), _)| v).collect())
    }

    /// A maintenance delta (`__ins`, `__redo`, `__del`) of the component being solved: it fills in the same waves
    /// as the delta that probes it, from the same change, so its keys follow the delta's, and what is measured of
    /// it before the fixpoint is a snapshot. Neither an absence proof nor a containment share holds against it.
    fn changes_with_the_delta(&self, rel: &str) -> bool {
        ["__ins", "__redo", "__del"].iter().any(|s| rel.ends_with(s)) && self.stats.solving.contains(rel)
    }

    /// Positive joins whose keys cannot occur in the forced first atom's values. Not a maintenance delta of the
    /// component being solved (`changes_with_the_delta`). Only that atom is a binding
    /// source: a later predicate whose variables happen to be bound has not yet restricted the demand.
    pub fn delta_misses(&self, b: &Body<'_>, first: Option<usize>) -> Vec<usize> {
        let first = first.or_else(|| b.demand_first());
        let Some(first) = first else { return Vec::new() };
        let Some(delta) = b.atom(first) else { return Vec::new() };
        let drel = b.rel(delta);
        let dargs = b.args(first);
        // A delta column's values, made only where another atom shares its variable.
        let values: Vec<std::cell::OnceCell<Option<Vec<&Constant>>>> = (0..dargs.len()).map(|_| std::cell::OnceCell::new()).collect();
        (0..b.len()).filter_map(|i| {
            let a = b.plain_atom(i)?;
            if i == first || a.neg() { return None; }
            let args = b.args(i);
            // Only an atom sharing a variable with the delta can miss its keys: tested by the terms first, the
            // relation's name after.
            let shares = |v| args.iter().any(|t| matches!(t, TermRef::Var(x) if *x == v));
            if !dargs.iter().any(|t| matches!(*t, TermRef::Var(v) if shares(v))) || self.changes_with_the_delta(b.rel(a)) { return None; }
            dargs.iter().enumerate().any(|(c, t)| {
                let TermRef::Var(v) = *t else { return false };
                if !shares(v) { return false; }
                let Some(from) = values[c].get_or_init(|| self.small_values(drel, c)) else { return false };
                if from.is_empty() { return false; }
                args.iter().enumerate().any(|(at, t)| matches!(t, TermRef::Var(x) if *x == v)
                    && self.small_values(b.rel(a), at).is_some_and(|to| !to.is_empty() && from.iter().all(|v| !to.contains(v))))
            }).then_some(i)
        }).collect()
    }

    /// Orders a rule body (T's `plan_rule`). Dynamic programming over subsets of the join literals (Selinger): the
    /// cheapest order of each subset, with the filters applied as early as they can run. Work counts rows produced per
    /// step plus tape reads for `nearest` (climb: steps per row; descent: mean subtree size per row). With `first`
    /// (a body position), that atom reads its semi-naive delta. A smaller connected changed-input demand may
    /// precede its indexed probe.
    pub fn plan_rule(&self, pf: Fields<'_>, b: &Body<'_>, first: Option<usize>) -> Result<Plan> {
        Ok(self.plan_rule_explained(pf, b, first)?.0)
    }

    /// `plan_rule` for its price alone: no EXPLAIN is made of it, even when profiling (§4's pricing of a stratum's
    /// two forms planned 10,092 rules at O125 step 08 to read their totals, 0.6 s of explain records).
    pub fn plan_rule_priced(&self, pf: Fields<'_>, b: &Body<'_>, first: Option<usize>) -> Result<Plan> {
        Ok(self.plan_inner(pf, b, first, None, false)?.0)
    }

    /// `plan_rule`, and when profiling, the plan as EXPLAIN shows it (`profile::RulePlan`): the search's size, and per
    /// step its access method, source and the rows expected after it.
    /// Decision 0354 (tests): the orders a search priced with a batch pricer on, each with its steps as the search
    /// made them and as the plan record of the same order gives them.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn search_steps_against_records(&self, pf: Fields<'_>, b: &Body<'_>, first: Option<usize>) -> Result<Vec<(Vec<super::profile::StepPlan>, Option<super::profile::RulePlan>)>> {
        let mut cx = joins::SearchCx::default();
        self.plan_inner_in(pf, b, first, None, false, &mut cx)?;
        Ok(cx.steps_of.into_iter().map(|(order, st)| (st, self.explain_order(pf, b, first, &order))).collect())
    }

    pub fn plan_rule_explained(
        &self,
        pf: Fields<'_>,
        b: &Body<'_>,
        first: Option<usize>,
    ) -> Result<(Plan, Option<super::profile::RulePlan>)> {
        self.plan_inner(pf, b, first, None, true)
    }

    /// `plan_rule_explained` for the rows expected after each step of the plan alone, as what a relation keeps
    /// calibrates them (decision 0099): the record is made without its strings, its steps' bases and the joins each
    /// step passed over, priced again for EXPLAIN.
    pub fn plan_rule_step_rows(&self, pf: Fields<'_>, b: &Body<'_>, first: Option<usize>) -> Result<(Plan, Vec<f64>)> {
        let (was, rows_was) = (EXPLAIN_ALWAYS.with(|e| e.replace(true)), STEP_ROWS.with(|e| e.replace(true)));
        let got = self.plan_inner(pf, b, first, None, true);
        EXPLAIN_ALWAYS.with(|e| e.set(was));
        STEP_ROWS.with(|e| e.set(rows_was));
        let (plan, x) = got?;
        let rows = x.map(|x| x.steps.iter().map(|s| s.rows).collect()).unwrap_or_default();
        Ok((plan, rows))
    }

    /// `plan_rule_explained`, reusing this rule's plan from an earlier chunk (decision 0038): the kept order is
    /// re-priced with this chunk's statistics, and searched again only where its work per node grew more than
    /// `REPLAN` times, or it no longer runs (a relation it reads is missing). The planner's cache lives for a
    /// corpus run (`reset_plans`).
    /// The plan a kept one gives as it is under the planning budget (decision 0122), or `None` if it is to be priced:
    /// its line did less work than a search when it last ran, scaled by how much what it reads grew (`sizes`, this
    /// plan's `live_sizes`). It reads the relations' rows only, never their columns' statistics (decision 0123).
    /// Decision 0203 §3: a rule whose exact pair statistics were measured is priced at each plan, never taken as it
    /// was kept: its kept price was made without them. (Q80: not by its plan key, which then changed at the flag that
    /// measured them, and the line forgot what it measured.)
    fn exact_reprices(b: &Body<'_>) -> bool {
        joins::any_exact() && joins::epoch(b.observed_key()) != 0
    }

    fn budget_takes(&self, kept: &Kept, b: &Body<'_>, first: Option<usize>, sizes: &[f64]) -> Option<(Plan, Option<super::profile::RulePlan>)> {
        let (order, _, _, _, Some(previous), Some(ran)) = kept else { return None };
        if !plan_budget() || Self::exact_reprices(b) {
            return None;
        }
        // The rows of what it reads (not the corpus's nodes, nor distributions): their product bounds its growth.
        let cards = 1 + usize::from(first.is_some()) + b.read_count();
        let grew = match (previous.sizes.get(1..cards), sizes.get(1..cards)) {
            (Some(a), Some(b)) => growth(a, b),
            _ => f64::INFINITY,
        };
        // A literal with a constant reads the constant's rows, not its relation's: `doc_role(d, 7)` with one row
        // for 7 when planned and 2,455 now grew 2,455 times, though `doc_role` grew 9 (smoking gun 10, t34).
        let grew = grew * Self::const_slots(b, first).into_iter().map(|i| match (previous.sizes.get(i), sizes.get(i)) {
            (Some(&a), Some(&b)) if a != -1.0 && b != -1.0 => {
                let n = |x: f64| if x == -2.0 { 0.0 } else { x };
                (n(b).max(1.0) / n(a).max(1.0)).max(1.0)
            }
            _ => 1.0,
        }).product::<f64>();
        // A line that did nothing tells nothing of its order's cost but that one row of input costs about that:
        // counted as one row of work (an order kept from an empty input, with 5,000 rows now, is searched).
        (ran.max(1.0) * grew < SEARCH_WORK && self.valid_live_driver(order, b, first))
            .then(|| if same_order(&previous.got.0.0, order) { previous.got.clone() } else { ((order.to_vec(), *ran, ran.max(1.0)), None) })
    }

    /// Whether planning this line (`plan_rule_reused` of the rule with `first` its delta) reads statistics: every line
    /// but one whose kept order is taken as it is (`budget_takes`). The component the line is
    /// planned in is the one this model is solving (`Stats::solve`): its digest is in the key.
    fn needs_statistics(&self, b: &Body<'_>, first: Option<usize>) -> bool {
        self.priced_why(b, first).is_some()
    }

    /// Why a line will be priced (`needs_statistics`), or `None` where its kept order is taken as it is.
    fn priced_why(&self, b: &Body<'_>, first: Option<usize>) -> Option<&'static str> {
        if plan_fresh() || !plan_budget() {
            return Some("no budget");
        }
        let misses = self.delta_misses(b, first);
        let key = self.plan_key_at("", b, first, &misses);
        let Some(kept) = plans_get(&key) else { return Some("no kept plan") };
        if self.budget_takes(&kept, b, first, &self.live_sizes_at(b, first, &misses)).is_some() {
            return None;
        }
        Some(match (&kept.4, kept.5) {
            (None, _) => "no price",
            (_, None) if planned_now(&key) => "never ran, planned before in this run",
            (_, None) => "never ran",
            (_, Some(r)) if r >= SEARCH_WORK => "ran over a search",
            _ => "inputs grew",
        })
    }

    /// Whether any statistics are still to be made (`deferred`): a round asks nothing where none is.
    pub(in crate::code_rule_dsl) fn has_deferred(&self) -> bool {
        !self.stats.lazy.0.is_empty() || !self.stats.prior_later.is_empty()
    }

    /// A relation whose statistics are still to be made: bound lazily (`Stats::lazily`), or quiet and taking its
    /// kept rows' (`prior_stats`).
    /// A relation its layout answers whole (`shapes::Shapes::answers`) is never one: its statistics are not made.
    fn deferred(&self, rel: &str) -> bool {
        self.stats.lazy.0.contains_key(rel) && !self.shaped(rel) || self.stats.prior_later.contains(rel)
    }

    /// Whether `rel`'s layout answers every column's distinct values (decision 0361 step 4).
    fn shaped(&self, rel: &str) -> bool {
        self.stats.shapes.answers(rel, self.stats.counts())
    }

    /// Decision 0123, step 1: the statistics a planning round requests. Of the lines it will plan (a rule and its
    /// delta atom; `None`: its seed or whole plan), those that will be priced (`needs_statistics`) request the
    /// relations they read whose statistics are still to be made. `plain` lines read none (`read_by`).
    pub(in crate::code_rule_dsl) fn wants<'r, 'v: 'r>(&self, lines: impl IntoIterator<Item = (&'r Body<'v>, Option<usize>)>, into: &mut BTreeSet<String>) {
        if self.stats.lazy.0.is_empty() && self.stats.prior_later.is_empty() {
            return;
        }
        for (b, first) in lines {
            let rels = b.reads_statistics();
            if rels.iter().all(|x| !self.deferred(x) || into.contains(*x)) {
                continue;
            }
            if let Some(why) = self.priced_why(b, first) {
                let before = into.len();
                into.extend(rels.into_iter().filter(|x| self.deferred(x)).map(str::to_owned));
                let n = into.len() - before;
                if n > 0 {
                    super::profile::decision("statistics requested (decision 0123)", || format!("by a line priced: {why}, {} delta ({} relations)", if first.is_some() { "with a" } else { "no" }, if n > 3 { "4+".to_owned() } else { n.to_string() }));
                }
            }
        }
    }

    /// `wants` for the strata a level or a scoreboard runs next (each a component and its rules), before any of them
    /// runs: each stratum's seeds (a rule with no atom of the component, or one reading a change) and probes (each
    /// atom of the component), asked as solving that component, whose digest is in their keys and whose relations
    /// their delta tests read. A stratum's own rounds make what these miss (its whole plans; a line whose rows moved).
    pub(in crate::code_rule_dsl) fn wants_strata<'r>(&mut self, vs: &body::Versions, strata: impl IntoIterator<Item = (&'r BTreeSet<String>, Vec<RuleKey>)>, into: &mut BTreeSet<String>) -> Result<()> {
        if self.stats.lazy.0.is_empty() && self.stats.prior_later.is_empty() {
            return Ok(());
        }
        let was = std::mem::take(&mut self.stats.solving);
        for (scc, rules) in strata {
            let bodies: Vec<Body<'_>> = rules.iter().map(|&r| vs.at(r)).collect();
            let lines: Vec<(&Body<'_>, Option<usize>)> = bodies.iter().flat_map(|b| {
                let rec = recursive(b, scc);
                let seed = rec.is_empty() || reads_change(b, scc);
                rec.into_iter().map(|(ai, _)| (b, Some(ai))).chain(seed.then_some((b, None))).collect::<Vec<_>>()
            }).collect();
            self.stats.solve(solving_of(vs, scc, &rules).unwrap_or_default());
            self.wants(lines, into);
        }
        self.stats.solve(was);
        Ok(())
    }

    /// Whether the line has no kept plan to start from (`plan_rule_reused` would search it: its first sight).
    /// Its key, when so.
    pub(in crate::code_rule_dsl) fn first_sight(&self, b: &Body<'_>, first: Option<usize>) -> Option<PlanKey> {
        if plan_fresh() {
            return None;
        }
        let key = self.reused_key(b, first);
        matches!(plans_find(&key, b.len(), self.stats.nodes.max(1.0)), Some(None)).then_some(key)
    }

    /// Decision 0123, step 2: a round's requested statistics, made in one batch before any of its plans is made
    /// (step 3). Each is made as a kept relation's are (`resolve`): counted from its postings, or measured. This is
    /// where one batch goes to another maker (a classifier on the device) as a whole.
    pub(in crate::code_rule_dsl) fn make_statistics(&mut self, f: Fields<'_>, wanted: &BTreeSet<String>) -> Result<()> {
        let wanted: Vec<&str> = wanted.iter().map(String::as_str).filter(|r| self.deferred(r)).collect();
        if wanted.is_empty() {
            return Ok(());
        }
        STATISTICS[0].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        STATISTICS[1].fetch_add(wanted.len() as u64, std::sync::atomic::Ordering::Relaxed);
        let _span = super::profile::span(|| "statistics batch".into());
        super::profile::decision("statistics (decision 0123)", || format!("one batch of {} relations: {}", wanted.len(), wanted.join(", ")));
        self.resolve(f, wanted)
    }

    /// A line about to be planned whose statistics no round requested (its key moved once the round's batch was made,
    /// or it was not foreseen): they are made now, one line at a time, and counted (`RERAIL_PLAN_CACHE_COUNTS`:
    /// `late`), as what the rounds should have asked. `priced`: a line that is searched whatever was kept
    /// (`plan_rule_live`).
    pub(in crate::code_rule_dsl) fn late(&mut self, f: Fields<'_>, b: &Body<'_>, first: Option<usize>, priced: bool) -> Result<()> {
        let _span = super::profile::span(|| "late statistics".into());
        let rels: Vec<&str> = b.reads_statistics().into_iter().filter(|x| self.deferred(x)).collect();
        if rels.is_empty() || !priced && !self.needs_statistics(b, first) {
            return Ok(());
        }
        STATISTICS[2].fetch_add(rels.len() as u64, std::sync::atomic::Ordering::Relaxed);
        super::profile::decision("statistics (decision 0123)", || format!("made late, for one line: {}", rels.join(", ")));
        self.resolve(f, rels)
    }

    pub fn plan_rule_reused(
        &self,
        pf: Fields<'_>,
        b: &Body<'_>,
        first: Option<usize>,
    ) -> Result<(Plan, Option<super::profile::RulePlan>)> {
        self.plan_rule_reused_in(pf, b, first, None)
    }

    /// `plan_rule_reused`; in an open round (decision 0354, `round`), a searched line's choice is recorded there and
    /// the search's best kept, the round's choices made together after and kept in its place (`keep_choice`).
    pub fn plan_rule_reused_in(
        &self,
        pf: Fields<'_>,
        b: &Body<'_>,
        first: Option<usize>,
        mut round: Option<&mut Vec<joins::Choice>>,
    ) -> Result<(Plan, Option<super::profile::RulePlan>)> {
        if plan_fresh() {
            return self.plan_inner(pf, b, first, None, true);
        }
        let misses = self.delta_misses(b, first);
        let key = self.plan_key_at("", b, first, &misses);
        let nodes = self.stats.nodes.max(1.0);
        let Some(kept) = plans_find(&key, b.len(), nodes) else {
            plan_cache(0);
            let mut cx = joins::SearchCx::default();
            return self.plan_inner_in(pf, b, first, None, true, &mut cx).map(|got| self.decide(pf, b, first, key, got, cx.runners_up, cx.steps_of, round.take()));
        };
        // A line with work queued for its next plan (decision 0203: a wider search, a proposal not tried) is searched.
        let pending = joins::pending(&key);
        let kept = if pending { None } else { kept };
        let sizes = {
            let _span = super::profile::span(|| "kept plan: its sizes".into());
            self.live_sizes_at(b, first, &misses)
        };
        if super::profile::on() {
            note_planned(key);
        }
        let remember = |key: &PlanKey, got: &(Plan, Option<super::profile::RulePlan>)| {
            plans_learnt(key, Learnt::Priced(Arc::new(Priced { sizes: sizes.clone(), got: got.clone() })));
        };
        // A rule whose line did less work than a search when it last ran (scaled by how much what it reads grew)
        // keeps its order as it is: neither searched nor priced (decision 0122).
        let budget = super::profile::span(|| "kept plan: the budget".into());
        if let Some(k) = &kept && let Some(got) = self.budget_takes(k, b, first, &sizes) {
            drop(budget);
            let _span = super::profile::span(|| "kept plan: taken, remembered".into());
            plan_cache(8);
            super::profile::decision("plan cache (decision 0038)", || format!("a kept plan taken as it is: its rule did {} rows of work when it last ran, less than a search costs (decision 0122)", k.5.unwrap_or(0.0)));
            // Its sizes now, against which the next run's growth is measured.
            remember(&key, &got);
            return Ok(got);
        }
        drop(budget);
        // Why a kept plan was not taken as it is (`RERAIL_PLAN_CACHE_COUNTS`).
        if plan_budget() && let Some(k) = &kept {
            let why = match (&k.4, k.5) {
                (None, _) => 0,
                (_, None) => 1,
                (_, Some(r)) if r >= SEARCH_WORK => 2,
                // Ran under a search's work: its inputs grew past what that allows, or its order is no longer led by
                // its delta.
                _ => 3,
            };
            BUDGET_MISS[why].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if why == 1 && super::profile::on() {
                let again = planned_now(&key);
                super::profile::decision("planning budget missed: never ran (decision 0122)", || format!("planned by {}, {}", PLAN_SITE.with(std::cell::Cell::get), if again { "planned before in this run" } else { "kept from an earlier run" }));
            }
            // What grew, by kind of size and how far (the profile's decisions).
            if why == 3 && super::profile::on() && let Some(previous) = &k.4 {
                let cards = 1 + usize::from(first.is_some()) + b.read_count();
                let atoms: Vec<&str> = (0..b.len()).filter_map(|i| b.atom(i)).map(|a| b.rel(a)).collect();
                let driver = self.valid_live_driver(&k.0, b, first);
                let (a, b) = (&previous.sizes[1..cards.min(previous.sizes.len())], &sizes[1..cards.min(sizes.len())]);
                let (at, g) = a.iter().zip(b).enumerate().map(|(i, (x, y))| (i, (y.max(1.0) / x.max(1.0)).max(1.0))).fold((0, 1.0), |m, (i, g)| if g > m.1 { (i, g) } else { m });
                let what = if first.is_some() && at == atoms.len() { "the delta".to_owned() } else { atoms.get(at).map_or("?".into(), |r| {
                    let r = *r;
                    if r.ends_with("__old") { "an old relation".to_owned() } else if self.stats.solving.contains(r) { "a relation of the component".to_owned() } else if r.contains("__") { format!("a change ({})", r.rsplit_once("__").map_or("", |x| x.1)) } else { "a relation".to_owned() }
                }) };
                let from = a.get(at).copied().unwrap_or(0.0);
                let bucket = |x: f64| if x <= 1.0 { "≤1" } else if x <= 10.0 { "≤10" } else if x <= 100.0 { "≤100" } else if x <= 1e4 { "≤10k" } else { ">10k" };
                let product = if a.len() == b.len() { growth(a, b) } else { f64::INFINITY };
                super::profile::decision("planning budget missed: inputs grew (decision 0122)", || format!("{}product ×{}; most by {what}, from {} rows, ×{}; ran {}", if driver { "" } else { "kept order not led by its delta; " }, bucket(product), bucket(from), bucket(g), bucket(k.5.unwrap_or(0.0))));
            }
        }
        // A kept order priced under what a search costs, whose line did a search's work or more when it last ran
        // (decision 0123 §7): its price was wrong by enough that a search can pay, and costs less than the line ran.
        // So an order taken without a search (`greedy first`, where its price is under a search's) is held to what it
        // did. Until the line runs again, the new plan is not held to the old work (`ran` is cleared).
        let mispriced = kept.as_ref().is_some_and(|k| mispriced(k.1 * nodes, k.5, b, first));
        let kept = if mispriced {
            plan_cache(9);
            super::profile::decision("plan cache (decision 0038)", || "searched: a kept order priced under a search ran over a search's work (decision 0123 §7)".into());
            None
        } else {
            kept
        };
        if let Some((_, _, _, _, Some(priced), _)) = &kept
            && near(&priced.sizes, &sizes)
            && self.valid_live_driver(&priced.got.0.0, b, first)
        {
            super::profile::decision("plan cache (decision 0038)", || format!("a kept plan reused at its kept price: no size it reads moved over {PRICE_DRIFT}x"));
            plan_cache(2);
            return Ok(priced.got.clone());
        }
        // Seen first: no kept plan, nor one dropped as mispriced (that is searched).
        let first_sight = kept.is_none() && !mispriced;
        if let Some((order, per_node, searched, gap, previous, _)) = kept {
            let priced = {
                let _span = super::profile::span(|| "re-price".into());
                // The search's EXPLAIN (its steps' rows as the search priced them), with this pricing's work, rows and
                // steps priced: not built again per chunk.
                self.plan_inner(pf, b, first, Some(&order), false).map(|(got, _)| {
                    let x = searched.map(|x| super::profile::RulePlan { work: got.1, rows: got.2, subsets: 0, candidates: order.len() as u64, ..x });
                    (got, x)
                })
            };
            // A start much cheaper than when the order was searched (a text test that matched thousands of strings
            // there and dozens here): the kept order may now scan what that start would look up, and pricing the
            // kept order alone never shows it.
            let cheaper_start = |work: f64| work / SEED.with(|c| c.get()).max(1.0) > REPLAN * gap.max(1.0) && work > REPLAN_ROWS;
            match priced {
                Ok(got) if cheaper_start(got.0.1) => {
                    plan_cache(4);
                    super::profile::decision("plan cache (decision 0038)", || "replanned: a start much cheaper than when the kept order was searched".into())
                }
                Ok(got) => {
                    // Why the kept order is not kept, if it is not: `None` keeps it.
                    let live = self.valid_live_driver(&got.0.0, b, first);
                    let cards = 1 + usize::from(first.is_some()) + b.read_count();
                    let dist_near = previous.as_ref().is_some_and(|p| {
                        let scale = sizes[0] / p.sizes.first().copied().unwrap_or(1.0).max(1.0);
                        p.sizes.len() >= cards && sizes.len() >= cards && near_distributions(&p.sizes[cards..], &sizes[cards..], scale, b.len())
                    });
                    let grew = !(got.0.1 / nodes <= REPLAN * per_node.max(1e-12) || got.0.1 - per_node * nodes <= REPLAN_ROWS);
                    // A re-priced order whose whole work is under `REPLAN_ROWS` is kept whatever its distributions:
                    // a search can save at most that work, and the search costs more than it here (O-replan-churn:
                    // small relations whose distinct values differ between a chunk and the cross part).
                    let why = if !live {
                        Some("the kept order's driver is not live here")
                    } else if got.0.1 <= REPLAN_ROWS {
                        None
                    } else if !dist_near {
                        Some("the distributions it reads moved, in proportion to the nodes")
                    } else if grew {
                        Some("the kept order's work per node grew over 4x, by over 10000 rows")
                    } else {
                        None
                    };
                    let Some(why) = why else {
                        super::profile::decision("plan cache (decision 0038)", || "a kept plan reused, re-priced with this chunk's statistics".into());
                        plan_cache(3);
                        remember(&key, &got);
                        return Ok(got);
                    };
                    plan_cache(5);
                    static SHOWN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                    if std::env::var_os("RERAIL_PLAN_CACHE_WHY").is_some() && nodes >= 1000.0 && SHOWN.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 12 {
                        eprintln!("replanned {} :- {} literals, first {first:?}: {why}: nodes {nodes}, work {} (kept per node {per_node} -> {}), sizes {:?}, kept sizes {:?}", b.show_head(), b.len(), got.0.1, per_node * nodes, sizes, previous.as_ref().map(|p| p.sizes.clone()));
                    }
                    super::profile::decision("plan cache (decision 0038)", || format!("replanned: {why}"))
                }
                Err(_) => {
                    plan_cache(6);
                    super::profile::decision("plan cache (decision 0038)", || "replanned: the kept order no longer runs".into())
                }
            }
        }
        // A line seen first starts from an order kept before, where a search could not pay (decision 0258). The
        // search tries it first, under its own set-up.
        let start = if first_sight && !pending { self.start_order(b, first) } else { None };
        let mut cx = joins::SearchCx { start: start.as_ref().map(|s| s.0.clone()), ..Default::default() };
        let got = {
            let _span = super::profile::span(|| "search".into());
            self.plan_inner_in(pf, b, first, None, true, &mut cx)?
        };
        if cx.started && let Some((_, how)) = start {
            plan_cache(10 + how);
            super::profile::decision("plan cache (decision 0038)", || format!("started from {} (decision 0258)", ["its line's order kept under another key", "its source rule's order"][how]));
            PLANS.with(|p| {
                if let Some(p) = p.borrow().as_ref() {
                    plans_put(p, key, (got.0.0.clone().into(), got.0.1 / nodes, None, got.0.1 / SEED.with(|c| c.get()).max(1.0), None, None));
                }
            });
            orders_put(line_key(b, first), &got.0.0);
            remember(&key, &got);
            return Ok(got);
        }
        if start.is_some() {
            super::profile::decision("source orders (decision 0258)", || "searched: the order does not run, is not led by its delta, or prices over a search's cost".into());
        }
        if first_sight {
            plan_cache(1);
            super::profile::decision("plan cache (decision 0038)", || "planned (first sight of the rule)".into());
        }
        let got = self.decide(pf, b, first, key, got, cx.runners_up, cx.steps_of, round.take());
        PLANS.with(|p| {
            if let Some(p) = p.borrow().as_ref() {
                let gap = got.0.1 / SEED.with(|c| c.get()).max(1.0);
                let ran = if mispriced { None } else { plans_get(&key).and_then(|k| k.5) };
                plans_put(p, key.clone(), (got.0.0.clone().into(), got.0.1 / nodes, got.1.clone(), gap, None, ran));
            }
        });
        if source_orders() {
            orders_put(line_key(b, first), &got.0.0);
        }
        remember(&key, &got);
        Ok(got)
    }

    /// Decision 0354: a round's choice for a line planned in it (`plan_rule_reused_in`), kept under its key in place
    /// of the search's best, as `plan_rule_reused` keeps a line's plan: the line's next planning takes it.
    fn keep_choice(&self, pf: Fields<'_>, b: &Body<'_>, first: Option<usize>, key: PlanKey, order: &[Step]) -> Result<(Plan, Option<super::profile::RulePlan>)> {
        let got = self.plan_inner(pf, b, first, Some(order), true)?;
        let nodes = self.stats.nodes.max(1.0);
        PLANS.with(|p| {
            if let Some(p) = p.borrow().as_ref() {
                let gap = got.0.1 / SEED.with(|c| c.get()).max(1.0);
                let ran = plans_get(&key).and_then(|k| k.5);
                plans_put(p, key, (got.0.0.clone().into(), got.0.1 / nodes, got.1.clone(), gap, None, ran));
            }
        });
        if source_orders() {
            orders_put(line_key(b, first), &got.0.0);
        }
        // Its sizes now, as `plan_rule_reused` keeps them with its plan.
        let sizes = self.live_sizes_at(b, first, &self.delta_misses(b, first));
        plans_learnt(&key, Learnt::Priced(Arc::new(Priced { sizes, got: got.clone() })));
        Ok(got)
    }

    /// Decision 0354: with a batch pricer, a stratum's lines planned before its loop, their choices made in one
    /// round (one batch prediction for all of them): every line, not only those seen first, since a line with a
    /// kept plan may be searched again (work pending, its sizes grown), and those were two in three of the rounds a
    /// line at a time. Each line's plan is handed to the loop (`Ahead`), which plans only a line missing from it
    /// (its planning failed here: the loop reports it). Without a pricer, or without a kept plans' store, nothing.
    pub(in crate::code_rule_dsl) fn plan_ahead(&mut self, pf: Fields<'_>, lines: &[(&Body<'_>, Option<usize>)]) -> Ahead {
        let mut ahead = Ahead::default();
        if !joins::pricing() || PLANS.with(|p| p.borrow().is_none()) || lines.is_empty() {
            return ahead;
        }
        let _span = super::profile::span(|| "join model: a stratum's plans in one round".into());
        let mut round: Vec<joins::Choice> = Vec::new();
        let mut asked = Vec::new();
        for &(b, first) in lines {
            if self.late(pf, b, first, false).is_err() {
                continue;
            }
            let at = round.len();
            if let Ok(got) = self.plan_rule_reused_in(pf, b, first, Some(&mut round)) {
                if round.len() > at {
                    asked.push((at, b, first));
                }
                ahead.insert(line_id(b, first), got);
            }
        }
        let choices = joins::finish_round(round);
        for (i, b, first) in asked {
            let Some(c) = choices.get(i).filter(|c| c.chosen != 0) else { continue };
            match self.keep_choice(pf, b, first, c.key, &c.candidates[c.chosen].order) {
                Ok(got) => {
                    super::profile::decision("join choices (decision 0203)", || format!("{}: {}", b.head_text(), c.why));
                    joins::log(|| format!("choice {} (in a round): {}", b.head_text(), c.why));
                    ahead.insert(line_id(b, first), got);
                }
                // The loop plans it, and takes the kept plan there is.
                Err(_) => {
                    ahead.remove(&line_id(b, first));
                }
            }
        }
        ahead
    }

    /// Decision 0258: an order for a line seen first, kept before, for the search to try first (`SearchCx::start`),
    /// which takes it where it runs, is led by the line's delta and prices under what a search costs (a search could
    /// not pay for itself: decisions 0122, 0123 §7): the line's own order kept under another key (its component
    /// differs), else, for a maintenance rule (`Body::origin`), its source rule's order (`source_order`). Which of the
    /// two (0, 1) with it. None where a search prices fewer than `START_CANDIDATES` orders: it prices the start's.
    fn start_order(&self, b: &Body<'_>, first: Option<usize>) -> Option<(Vec<Step>, usize)> {
        if !source_orders() {
            return None;
        }
        // Why a line is searched, not started (the profile's decisions).
        let not = |why: &str| {
            super::profile::decision("source orders (decision 0258)", || format!("searched: {why}"));
            None
        };
        if search_rows(b, first) - SEARCH_FIXED_ROWS < START_CANDIDATES {
            return not("a search prices as few orders as a start");
        }
        match orders_get(line_key(b, first)) {
            Some(o) => Some((o.to_vec(), 0)),
            None => {
                let Some(src) = b.origin() else { return not(if b.rule.source().is_some() { "a source of another relation" } else { "no source rule" }) };
                let map = b.source_map(&src);
                // The source's line with the same delta, else its line with none.
                let at = first.and_then(|f| map[f]);
                let Some(kept) = orders_get(line_key(&src, at)).or_else(|| at.and_then(|_| orders_get(line_key(&src, None)))) else { return not("its source rule's line has no order kept") };
                let Some(order) = source_order(b, &src, &map, &kept, first) else { return not("a filter of the source's order never runs") };
                Some((order, 1))
            }
        }
    }

    /// A kept plan's key: the rule as text, the delta (`first`) and the component solved, after `kind`.
    fn plan_key(&self, kind: &str, b: &Body<'_>, first: Option<usize>) -> PlanKey {
        self.plan_key_at(kind, b, first, &self.delta_misses(b, first))
    }

    /// `plan_key` with the line's `delta_misses`, which a plan call finds once for its keys and its sizes
    /// (`live_sizes_at`). The rule is written by its digest (`Body::key`), made once per body.
    fn plan_key_at(&self, kind: &str, b: &Body<'_>, first: Option<usize>, misses: &[usize]) -> PlanKey {
        let _span = super::profile::span(|| "key".into());
        super::derived::digest(|d| {
            d.str(kind);
            d.u64(b.key() as u64);
            d.u64((b.key() >> 64) as u64);
            d.u64(first.map_or(u64::MAX, |f| f as u64));
            d.u64(self.stats.solving_digest as u64);
            d.u64((self.stats.solving_digest >> 64) as u64);
            // A changed key domain can make another order cheaper while row and distinct counts stay fixed.
            for &i in misses { d.u64(i as u64); }
            // A join shape directive narrows the search: an order kept without it is not this rule's.
            for shape in self.shapes_for(b.rel(b.head())) { d.str(&shape.text); }
            // Exact statistics measured for the rule (decision 0203 §3) are not in the key (Q80): they re-key what
            // the line measured, and a slow line's flag, which measures them, forgot its own line. A rule with exact
            // statistics is priced at each plan instead, never taken as it is kept (`exact_reprices`).
        })
    }

    /// The sizes a price reads (`Priced`): the corpus's nodes and the rows of each relation the body reads.
    fn read_sizes(&self, b: &Body<'_>) -> Vec<f64> {
        std::iter::once(self.stats.nodes.max(1.0)).chain((0..b.len()).filter_map(|i| b.atom(i)).map(|a| self.card(b.rel(a)))).collect()
    }

    /// Where `live_sizes` holds the count of each constant the body's atoms test: the planning budget reads them
    /// as growth (`budget_takes`).
    fn const_slots(b: &Body<'_>, first: Option<usize>) -> Vec<usize> {
        // `read_sizes` (the nodes, then each atom's rows), then the delta's rows.
        let mut at = 1 + b.read_count() + usize::from(first.and_then(|f| b.atom(f)).is_some());
        let mut slots = Vec::new();
        for i in (0..b.len()).filter(|&i| b.atom(i).is_some()) {
            for t in b.args(i) {
                if matches!(t, TermRef::Wild) { continue; }
                at += 2;
                if b.constant(*t).is_some() {
                    slots.push(at);
                    at += 1;
                }
            }
        }
        slots
    }

    /// Live cardinalities followed by the distributions the price reads. A delta's size affects whether a
    /// search pays for itself; its distinct values and fanouts can change the cheapest order even at fixed size.
    pub fn live_sizes(&self, b: &Body<'_>, first: Option<usize>) -> Vec<f64> {
        self.live_sizes_at(b, first, &self.delta_misses(b, first))
    }

    /// `live_sizes` with the line's `delta_misses` (`plan_key_at`'s). Each atom's relation is found once, and its
    /// columns read by its number (decision 0209 §3), not by name a column at a time.
    fn live_sizes_at(&self, b: &Body<'_>, first: Option<usize>, misses: &[usize]) -> Vec<f64> {
        let mut sizes = Vec::with_capacity(2 + 4 * b.len());
        sizes.extend(self.read_sizes(b));
        if let Some(a) = first.and_then(|f| b.atom(f)) {
            sizes.push(self.stats.delta_rows.get(b.rel(a)).copied().unwrap_or(-1.0));
        }
        for i in (0..b.len()).filter(|&i| b.atom(i).is_some()) {
            let rel = b.rel(b.atom(i).expect("an atom"));
            let id = table::find(rel);
            for (c,t) in b.args(i).iter().enumerate() {
                if matches!(t, TermRef::Wild) { continue; }
                sizes.push(id.and_then(|r| self.ndv.at_rel(r, c)).copied().unwrap_or(1.0));
                sizes.push(id.and_then(|r| self.stats.fan.at_rel(r, c)).copied().unwrap_or(1.0));
                if let Some(k) = b.constant(*t) {
                    let n = if rel == "kind" && c == 1 {
                        self.kind_frequency(&k)
                    } else {
                        id.and_then(|r| self.stats.dfreq.at_rel(r, c)).map(|f|f.get(&k).copied().unwrap_or(0))
                            .or_else(||self.stats.const_freq(rel,&self.col(rel,c),&k))
                    };
                    sizes.push(n.map_or(-1.0, |n| if n == 0 { -2.0 } else { n as f64 }));
                }
            }
        }
        // One bit per literal; 1 -> 4 crosses PRICE_DRIFT and also changes the driver's exact signature.
        sizes.extend((0..b.len()).map(|i| if misses.contains(&i) { 4.0 } else { 1.0 }));
        sizes
    }

    fn smaller_change(&self, b: &Body<'_>, delta: usize, candidate: usize) -> bool {
        let (Some(d), Some(a)) = (b.atom(delta), b.atom(candidate)) else { return false };
        let (dr, ar) = (b.rel(d), b.rel(a));
        !a.neg() && self.stats.solving.contains(dr) && !self.stats.solving.contains(ar)
            && (ar.ends_with("__plus") || ar.ends_with("__minus"))
            && !body::atom_vars(d).is_disjoint(&body::atom_vars(a))
            && self.card(ar) < self.stats.delta_rows.get(dr).copied()
                .unwrap_or_else(|| if small_batches(dr) { 0.0 } else { self.card(dr) })
    }

    /// Whether `order` starts from the line's driver: its delta (or a smaller connected change), else a
    /// demand-restricted aggregate's demand (`Body::demand_first`); and a demanded relation's rule from a lead its
    /// demand bounds (`Body::demand_bounds_lead`), so a kept order led by a scan is searched again (Q59).
    pub fn valid_live_driver(&self, order: &[Step], b: &Body<'_>, first: Option<usize>) -> bool {
        let driven = first.or_else(|| b.demand_first()).is_none_or(|f| order.first().is_some_and(|s| s.at == f || self.smaller_change(b, f, s.at)));
        driven && (first.is_some() || order.first().is_none_or(|s| b.demand_bounds_lead(s.at)))
    }

    /// Records the work a rule's line did in the engine (`plan_work`, every order it ran in) on its kept plan, for
    /// the next run's planning budget (decision 0122).
    pub(in crate::code_rule_dsl) fn ran(key: PlanKey, work: f64) {
        plans_learnt(&key, Learnt::Ran(work));
    }

    /// Tests: `ran`, under the key `plan_rule_reused` keeps the rule's plan under now.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn ran_as_planned(&self, b: &Body<'_>, first: Option<usize>, work: f64) {
        Self::ran(self.plan_key("", b, first), work);
    }

    /// The key `plan_rule_reused` keeps a rule's plan under now (it reads the statistics: taken when planned).
    pub(in crate::code_rule_dsl) fn reused_key(&self, b: &Body<'_>, first: Option<usize>) -> PlanKey {
        self.plan_key("", b, first)
    }

    /// A stratum's plan searched again from its live counts between waves (`plan_rule_explained`), kept with the
    /// sizes it saw: while no size moved over `PRICE_DRIFT` times, the same search is not made again (decision
    /// 0090 step 5: in a session's run these searches were most of its planning). Outside a corpus run every one
    /// is searched.
    pub fn plan_rule_live(
        &self,
        pf: Fields<'_>,
        b: &Body<'_>,
        first: Option<usize>,
    ) -> Result<(Plan, Option<super::profile::RulePlan>)> {
        self.plan_rule_live_in(pf, b, first, None)
    }

    /// `plan_rule_live`; in an open round (decision 0203 §2, `round`), a searched line's choice is recorded there
    /// and the search's best kept, the round's choices made together after (`joins::finish_round`).
    pub fn plan_rule_live_in(
        &self,
        pf: Fields<'_>,
        b: &Body<'_>,
        first: Option<usize>,
        round: Option<&mut Vec<joins::Choice>>,
    ) -> Result<(Plan, Option<super::profile::RulePlan>)> {
        if PLANS.with(|p| p.borrow().is_none()) {
            return self.plan_inner(pf, b, first, None, true);
        }
        // A line planned with the program (decision 0361 step 5): no key, no cache, no price.
        if let Some(plan) = self.stats.prepared.take(line_key(b, first), self.stats.nodes) {
            super::profile::decision("prepared plans (decision 0361 step 5)", || "a live plan taken as prepared: not searched or priced".to_owned());
            return Ok((plan, None));
        }
        let misses = self.delta_misses(b, first);
        let key = self.plan_key_at("live ", b, first, &misses);
        // The state kept before the last (O-live-replan item 4): sizes that alternate between waves (a distinct
        // count of 1, then 3, then 1) find their earlier price here instead of searching again.
        let earlier = key ^ (1u128 << 127);
        let sizes = self.live_sizes_at(b, first, &misses);
        let cardinalities = 1 + usize::from(first.is_some()) + b.read_count();
        // The last price, and the work of the order when it was searched.
        let kept = plans_find(&key, b.len(), self.stats.nodes.max(1.0)).flatten().and_then(|k| Some((k.4.clone()?, k.1)));
        // Keeps `got` priced at `sizes` under `key`, the state it replaces under `earlier`.
        let keep = |got: &(Plan, Option<super::profile::RulePlan>), sizes: Vec<f64>, searched: Option<f64>| {
            PLANS.with(|p| {
                if let Some(p) = p.borrow().as_ref() {
                    let then = plans_get(&key);
                    // The work its line did when it last ran (decision 0122), kept with the plan.
                    let ran = then.as_ref().and_then(|k| k.5);
                    // A search's order, at its work; or the kept order re-priced, as it was kept.
                    let entry: (Arc<[Step]>, f64, Option<super::profile::RulePlan>, f64) = match (searched, &then) {
                        (Some(work), _) => (got.0.0.clone().into(), work, None, 0.0),
                        (None, Some(k)) => (k.0.clone(), k.1, k.2.clone(), k.3),
                        (None, None) => return,
                    };
                    if let Some(then) = then {
                        plans_put(p, earlier, then);
                    }
                    plans_put(p, key, (entry.0, entry.1, entry.2, entry.3, Some(Arc::new(Priced { sizes, got: got.clone() })), ran));
                }
            });
        };
        // A line with work queued for its next plan (decision 0203) is searched.
        let line = (joins::any_noted() || joins::pricing()).then(|| self.plan_key_at("", b, first, &misses));
        let kept = if line.is_some_and(|l| joins::pending(&l)) { None } else { kept };
        if let Some((priced, searched)) = &kept {
            let exact = Self::exact_reprices(b);
            if !exact && near(&priced.sizes, &sizes) && self.valid_live_driver(&priced.got.0.0, b, first) {
                super::profile::decision("plan cache (decision 0038)", || format!("a live replan kept: no size it reads moved over {PRICE_DRIFT}x"));
                return Ok(priced.got.clone());
            }
            let alternate = plans_get(&earlier).and_then(|k| k.4);
            if let Some(alt) = alternate
                && !exact
                && near(&alt.sizes, &sizes)
                && self.valid_live_driver(&alt.got.0.0, b, first)
            {
                super::profile::decision("plan cache (decision 0038)", || format!("a live replan kept: the state before the last, no size moved over {PRICE_DRIFT}x"));
                return Ok(alt.got.clone());
            }
            // A size moved: the kept order is priced again (no search). It is kept:
            // - when its work is under what a search would cost (`search_rows`): a search could save less than it
            //   costs (O-live-replan item 1; 89% of live searches found the order they had);
            // - when the body has one positive atom, whose order is forced (item 5);
            // - otherwise while the distributions it reads are near in proportion to the nodes (item 3) and its work
            //   grew less than `REPLAN` times or by fewer than `REPLAN_ROWS` rows over its work when searched.
            let then = *searched;
            let repriced = {
                let _span = super::profile::span(|| "re-price".into());
                self.plan_inner(pf, b, first, Some(&priced.got.0.0), false)
            };
            let positive = (0..b.len()).filter(|&i| matches!(b.view(i), Lit::Atom(a) if !a.neg()) || matches!(b.view(i), Lit::Agg { .. })).count();
            let distributions_near = || {
                let scale = sizes[0] / priced.sizes.first().copied().unwrap_or(1.0).max(1.0);
                priced.sizes.len() >= cardinalities && sizes.len() >= cardinalities && near_distributions(&priced.sizes[cardinalities..], &sizes[cardinalities..], scale, b.len())
            };
            let why = match &repriced {
                Err(_) => Some("the kept order no longer runs"),
                Ok((plan, _)) if !self.valid_live_driver(&plan.0, b, first) => Some("the kept order's driver is not live here"),
                Ok(_) if positive <= 1 => None,
                Ok((plan, _)) if plan.1 <= search_rows(b, first) => None,
                Ok(_) if !distributions_near() => Some("the distributions it reads moved, in proportion to the nodes"),
                Ok((plan, _)) if !(plan.1 <= REPLAN * then.max(1.0) || plan.1 - then <= REPLAN_ROWS) => Some("its work grew over 4x, by over 10000 rows"),
                Ok(_) => None,
            };
            if let Some(why) = why {
                super::profile::decision("live replans searched (decision 0038)", || format!("searched again: {why}"));
                static SHOWN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                if std::env::var_os("RERAIL_PLAN_CACHE_WHY").is_some() && SHOWN.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 40 {
                    let work = repriced.as_ref().map(|(p, _)| p.1).unwrap_or(-1.0);
                    eprintln!("live searched {} :- {} literals, first {first:?}: {why}: work {work} (searched {then}, a search {}), sizes {sizes:?}, kept sizes {:?}", b.show_head(), b.len(), search_rows(b, first), priced.sizes);
                }
            }
            if let Ok((plan, _)) = repriced
                && why.is_none()
            {
                super::profile::decision("plan cache (decision 0038)", || {
                    if positive <= 1 { "a live replan kept, re-priced: one positive atom, its order forced".into() }
                    else if plan.1 <= search_rows(b, first) { "a live replan kept, re-priced: its work is under a search's".into() }
                    else { "a live replan kept, re-priced: its work grew little".into() }
                });
                let got = (plan, priced.got.1.clone());
                keep(&got, sizes, None);
                return Ok(got);
            }
        }
        if kept.is_none() {
            super::profile::decision("live replans searched (decision 0038)", || "searched: no plan kept under its key".into());
        }
        // Without its EXPLAIN (item 2): the caller asks for it (`explain_order`) only when it takes a new order.
        let mut cx = joins::SearchCx::default();
        let got = {
            let _span = super::profile::span(|| "search".into());
            self.plan_inner_in(pf, b, first, None, false, &mut cx)?
        };
        let got = match line {
            Some(line) => self.decide(pf, b, first, line, got, cx.runners_up, cx.steps_of, round),
            None => got,
        };
        // Its work as searched (for live replans, the second field), against which a re-pricing is judged.
        keep(&got, sizes, Some(got.0.1));
        Ok(got)
    }

    /// Decision 0203 §2: the choice of a searched line's order. `got` is the search's best; its runners-up
    /// (`alternatives`, handed back in its `joins::SearchCx`) and the line's proposals re-priced now are its candidates. Decision 0243: a wider
    /// search is not run here: the line asks for it at its end, and the background serves it. A line with no note and no batch pricer takes
    /// the search's best at once. In an open round (`round`) the choice is recorded and the best kept.
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    fn decide(&self, pf: Fields<'_>, b: &Body<'_>, first: Option<usize>, key: PlanKey, got: (Plan, Option<super::profile::RulePlan>), alternatives: Vec<(Vec<Step>, f64)>, steps_of: Vec<(Vec<Step>, Vec<super::profile::StepPlan>)>, round: Option<&mut Vec<joins::Choice>>) -> (Plan, Option<super::profile::RulePlan>) {
        if !joins::on() {
            return got;
        }
        let note = joins::note(&key);
        if note.is_none() && !joins::pricing() {
            return got;
        }
        let mut candidates = vec![joins::Candidate { order: got.0.0.clone(), est: got.0.1, steps: Vec::new() }];
        candidates.extend(alternatives.into_iter().map(|(order, est)| joins::Candidate { order, est, steps: Vec::new() }));
        if let Some(n) = &note {
            for (o, _) in &n.proposals {
                if let Ok(((order, est, _), _)) = self.plan_inner(pf, b, first, Some(&joins::unstored(o)), false)
                    && self.valid_live_driver(&order, b, first)
                {
                    candidates.push(joins::Candidate { order, est, steps: Vec::new() });
                }
            }
        }
        let mut seen: Vec<Vec<Step>> = Vec::new();
        candidates.retain(|c| if seen.contains(&c.order) { false } else { seen.push(c.order.clone()); true });
        // Decision 0354: with a batch pricer (the learned join model), each candidate's steps as its features.
        if joins::pricing() {
            let _span = super::profile::span(|| "join model: features".into());
            // A candidate the statistics price over `joins::feature_span` times the best's is not in play (the model's
            // corrections are a few times, not hundreds): it keeps its price, and is not explained.
            let best = candidates.iter().map(|c| c.est).fold(f64::INFINITY, f64::min).max(1.0);
            for (i, c) in candidates.iter_mut().enumerate() {
                if c.est > joins::feature_span() * best {
                    continue;
                }
                // The search's own steps where it priced the order (the best, the runners-up); else the search's best
                // record, or one explained here (a proposal re-priced).
                let made;
                let x = match (steps_of.iter().find(|(o, _)| *o == c.order), i, &got.1) {
                    (Some((_, st)), _, _) => {
                        made = super::profile::RulePlan { steps: st.clone(), ..Default::default() };
                        &made
                    }
                    (None, 0, Some(x)) => x,
                    _ => match self.explain_order(pf, b, first, &c.order) {
                        Some(x) => {
                            made = x;
                            &made
                        }
                        None => continue,
                    },
                };
                c.steps = step_samples(self, b, &c.order, x, None, first.is_none(), "model").unwrap_or_default();
            }
        }
        let mut choice = joins::Choice { key, candidates, chosen: 0, why: "" };
        if let Some(round) = round {
            round.push(choice);
            return got;
        }
        joins::choose(std::slice::from_mut(&mut choice));
        if choice.chosen == 0 {
            return got;
        }
        super::profile::decision("join choices (decision 0203)", || format!("{}: {}", b.head_text(), choice.why));
        joins::log(|| format!("choice {}: {} ({} candidates)", b.head_text(), choice.why, choice.candidates.len()));
        self.plan_inner(pf, b, first, Some(&choice.candidates[choice.chosen].order), got.1.is_some()).unwrap_or(got)
    }

    /// Decision 0203 §5: a wider search of a line, best first: the exact search up to `joins::WIDE_GREEDY` joins
    /// with its runners-up, and random connected orders (`joins::random_orders`), each priced with the statistics
    /// as they are (the exact pair statistics among them).
    pub(in crate::code_rule_dsl) fn explore(&self, pf: Fields<'_>, b: &Body<'_>, first: Option<usize>) -> Vec<(Vec<Step>, f64)> {
        let mut out: Vec<(Vec<Step>, f64)> = Vec::new();
        let mut cx = joins::SearchCx { wide: true, ..Default::default() };
        if let Ok(((order, work, _), _)) = self.plan_inner_in(pf, b, first, None, false, &mut cx) {
            out.push((order, work));
            out.extend(cx.runners_up);
        }
        let js: Vec<usize> = (0..b.len()).filter(|&i| b.is_join(i)).collect();
        let vars: Vec<u128> = js.iter().map(|&j| b.vars(j).iter().fold(0u128, |m, v| m | 1u128 << (v.index() % 128))).collect();
        let start = first.and_then(|f| js.iter().position(|&j| j == f));
        let mut rng = joins::Rng::new(b.observed_key());
        for o in joins::random_orders(&vars, start, joins::RANDOM_ORDERS, &mut rng) {
            let Some(order) = Self::with_filters(b, o.iter().map(|&k| js[k])) else { continue };
            if let Ok(((order, work, _), _)) = self.plan_inner(pf, b, first, Some(&order), false)
                && self.valid_live_driver(&order, b, first)
            {
                out.push((order, work));
            }
        }
        out.sort_by(|a, b| a.1.total_cmp(&b.1));
        let mut seen: Vec<Vec<Step>> = Vec::new();
        out.retain(|(o, _)| if seen.contains(o) { false } else { seen.push(o.clone()); true });
        out
    }

    /// The body's literals in the join order `joins`, each filter placed as soon as it is ready; `None` where one
    /// never is.
    fn with_filters(b: &Body<'_>, joins: impl Iterator<Item = usize>) -> Option<Vec<Step>> {
        let mut filters: Vec<usize> = (0..b.len()).filter(|&i| !b.is_join(i)).collect();
        let (mut out, mut bound) = (Vec::with_capacity(b.len()), VarSet::new());
        let place = |out: &mut Vec<Step>, bound: &mut VarSet, filters: &mut Vec<usize>| {
            let mut progressed = true;
            while progressed {
                progressed = false;
                if let Some(k) = filters.iter().position(|&f| b.ready(f, bound)) {
                    let f = filters.remove(k);
                    out.push(Step { at: f, mode: None });
                    bound.extend(b.vars(f));
                    progressed = true;
                }
            }
        };
        place(&mut out, &mut bound, &mut filters);
        for j in joins {
            out.push(Step { at: j, mode: None });
            bound.extend(b.vars(j));
            place(&mut out, &mut bound, &mut filters);
        }
        filters.is_empty().then_some(out)
    }

    /// Decision 0203 §3: `order`'s price for the `delta` rows its delta atom (`first`) read, the delta's rows set to
    /// them while priced. A seed's (`first` none) is its price.
    /// Decision 0206: the best order of `b` reading `rows` rows of atom `at` (the rows a cut join left), as a delta
    /// is read.
    pub(in crate::code_rule_dsl) fn plan_for_rows(&mut self, pf: Fields<'_>, b: &Body<'_>, at: usize, rows: f64) -> Option<Vec<Step>> {
        self.plan_for_rows_in(pf, b, at, rows, &mut joins::SearchCx::default())
    }

    /// `plan_for_rows` under `cx`: its runners-up handed back there (decision 0207's trials).
    fn plan_for_rows_in(&mut self, pf: Fields<'_>, b: &Body<'_>, at: usize, rows: f64, cx: &mut joins::SearchCx) -> Option<Vec<Step>> {
        let rel = b.atom(at).map(|a| b.rel(a).to_owned())?;
        let was = self.stats.delta_rows.insert(rel.clone(), rows.max(1.0));
        let got = self.plan_inner_in(pf, b, Some(at), None, false, cx).ok().map(|((order, _, _), _)| order);
        match was {
            Some(v) => self.stats.delta_rows.insert(rel, v),
            None => self.stats.delta_rows.remove(&rel),
        };
        got
    }

    pub(in crate::code_rule_dsl) fn price_for_delta(&mut self, pf: Fields<'_>, b: &Body<'_>, first: Option<usize>, order: &[Step], delta: f64) -> Option<f64> {
        let rel = first.and_then(|f| b.atom(f)).map(|a| b.rel(a).to_owned());
        let was = rel.as_ref().map(|r| self.stats.delta_rows.insert(r.clone(), delta.max(1.0)));
        let got = self.plan_inner(pf, b, first, Some(order), false).ok().map(|((_, w, _), _)| w);
        if let (Some(r), Some(was)) = (rel, was) {
            match was {
                Some(v) => self.stats.delta_rows.insert(r, v),
                None => self.stats.delta_rows.remove(&r),
            };
        }
        got
    }

    /// A live order's EXPLAIN, priced as the search priced it: made only when a replan takes a new order.
    pub fn explain_order(&self, pf: Fields<'_>, b: &Body<'_>, first: Option<usize>, order: &[Step]) -> Option<super::profile::RulePlan> {
        let _span = super::profile::span(|| "explain order".into());
        self.plan_inner(pf, b, first, Some(order), true).ok().and_then(|(_, x)| x)
    }

    /// `plan_search`, under `RERAIL_STATS_SKEW` (a measurement aid, decision 0096): planned with each column's
    /// fanout and distinct values skewed by a fixed factor in [1/f, f]; a search is also made with the statistics as
    /// they are, and the skewed order priced with them, so `RERAIL_STATS_SKEW_LOG` gets the regret: the skewed
    /// order's work over the best order's, by the true statistics.
    /// The join shape directives on `head`'s rules and their forms (`front::shape_base`).
    fn shapes_for(&self, head: &str) -> Vec<&crate::code_rule_dsl::logic::front::Shape> {
        if self.shapes.is_empty() {
            return Vec::new();
        }
        let base = crate::code_rule_dsl::logic::front::shape_base(head);
        self.shapes.iter().filter(|s| s.head == base).collect()
    }

    /// The rule's join shape directives as waits of its joins (`.shape`, O-join-shape-directives): `(join, any of the
    /// joins it waits for, the variables it waits for, the directive's text)`. A join is held back until a literal of
    /// the relation it waits for is joined, or the variables it waits for are bound; the delta that drives the rule
    /// (`first`) never is. Each wait is on the subset joined and what it binds, as a gate's is, so the search stays
    /// exact within them. `joins`: the body's join literals, in order.
    fn shape_waits<'s>(&'s self, b: &Body<'_>, first: Option<usize>, joins: &[usize]) -> Vec<(usize, Vec<usize>, Vec<VarSlot>, &'s str)> {
        use crate::code_rule_dsl::logic::front::{ShapeWait, shape_base};
        let mut waits = Vec::new();
        for shape in self.shapes_for(b.rel(b.head())) {
            for (i, &bi) in joins.iter().enumerate() {
                if first == Some(bi) || !b.atom(bi).is_some_and(|a| shape_base(b.rel(a)) == shape.lit) {
                    continue;
                }
                match &shape.wait {
                    ShapeWait::After(x) => {
                        let any: Vec<usize> = joins.iter().enumerate().filter(|&(j, &bj)| j != i && b.atom(bj).is_some_and(|a| shape_base(b.rel(a)) == x)).map(|(j, _)| j).collect();
                        if !any.is_empty() {
                            waits.push((i, any, Vec::new(), shape.text.as_str()));
                        }
                    }
                    ShapeWait::Bound(vars) => {
                        let vars: Vec<VarSlot> = vars.iter().filter_map(|v| b.slot(v)).filter(|&s| (0..b.len()).any(|l| b.vars(l).contains(s))).collect();
                        if !vars.is_empty() {
                            waits.push((i, Vec::new(), vars, shape.text.as_str()));
                        }
                    }
                }
            }
        }
        waits
    }

    /// The search within the join shape directives on the rule (`.shape`, O-join-shape-directives). Where they leave no
    /// order, the search without them, said in the profile. Profiled, the search without them too: a directive whose
    /// order is the planner's own is no longer needed (estimates cannot judge it: a directive is written where they
    /// are wrong).
    fn plan_inner(
        &self,
        pf: Fields<'_>,
        b: &Body<'_>,
        first: Option<usize>,
        fixed: Option<&[Step]>,
        explain: bool,
    ) -> Result<(Plan, Option<super::profile::RulePlan>)> {
        self.plan_inner_in(pf, b, first, fixed, explain, &mut joins::SearchCx::default())
    }

    /// `plan_inner` under `cx` (decisions 0203, 0104): how wide the search is, and its runners-up, handed back.
    fn plan_inner_in(
        &self,
        pf: Fields<'_>,
        b: &Body<'_>,
        first: Option<usize>,
        fixed: Option<&[Step]>,
        explain: bool,
        cx: &mut joins::SearchCx,
    ) -> Result<(Plan, Option<super::profile::RulePlan>)> {
        // A line planned with the program from its layouts (decision 0361 step 5): taken as kept, not searched.
        if fixed.is_none()
            && let Some(plan) = self.stats.prepared.take(line_key(b, first), self.stats.nodes)
        {
            super::profile::decision("prepared plans (decision 0361 step 5)", || "a line's plan taken as prepared: not searched".to_owned());
            let explained = if explain { self.explain_order(pf, b, first, &plan.0) } else { None };
            return Ok((plan, explained));
        }
        if self.shapes_for(b.rel(b.head())).is_empty() || NO_SHAPES.with(std::cell::Cell::get) {
            return self.plan_unshaped(pf, b, first, fixed, explain, cx);
        }
        let free = |this: &Self, explain: bool, cx: &mut joins::SearchCx| {
            NO_SHAPES.with(|n| n.set(true));
            let got = this.plan_unshaped(pf, b, first, None, explain, cx);
            NO_SHAPES.with(|n| n.set(false));
            got
        };
        match self.plan_unshaped(pf, b, first, fixed, explain, cx) {
            Ok(got) => {
                // The directives that hold back a step of this search: those of the rule's body as searched.
                let joins: Vec<usize> = (0..b.len()).filter(|&i| b.is_join(i)).collect();
                let used: Vec<String> = {
                    let mut t: Vec<String> = self.shape_waits(b, first, &joins).iter().map(|w| w.3.to_owned()).collect();
                    t.dedup();
                    t
                };
                if fixed.is_none()
                    && super::profile::on()
                    && !used.is_empty()
                    && let Ok(own) = free(self, false, &mut joins::SearchCx { wide: cx.wide, ..Default::default() })
                {
                    let head = b.head_text();
                    if b.shown(&own.0.0) == b.shown(&got.0.0) {
                        super::profile::decision("a join shape directive no longer needed (O-join-shape-directives)", || format!("{head}: the planner's own order is the directive's ({})", used.join("; ")));
                    } else {
                        super::profile::decision("join shape directives (O-join-shape-directives)", || format!("{head}: est work {:.0}, the planner's own {:.0} ({})", got.0.1, own.0.1, used.join("; ")));
                    }
                }
                Ok(got)
            }
            // A kept order the directives do not admit is searched again by the caller.
            Err(e) if fixed.is_some() => Err(e),
            Err(e) => {
                let head = b.head_text();
                super::profile::decision("a join shape directive that cannot hold (O-join-shape-directives)", || format!("{head}: no order within its directives ({e}); planned without them"));
                free(self, explain, cx)
            }
        }
    }

    fn plan_unshaped(
        &self,
        pf: Fields<'_>,
        b: &Body<'_>,
        first: Option<usize>,
        fixed: Option<&[Step]>,
        explain: bool,
        cx: &mut joins::SearchCx,
    ) -> Result<(Plan, Option<super::profile::RulePlan>)> {
        // A demand-restricted aggregate enumerates witnesses from its requested keys. Statistics
        // must not reorder that boundary into a scan of the global witness universe. A recursive
        // delta, when supplied, remains the driver. The prefix is reserved by RR lowering.
        let first = first.or_else(|| b.demand_first());
        if skew_setting().is_none() {
            let got = self.plan_search(pf, b, first, fixed, explain, cx)?;
            // A demanded relation's rule led by a scan its demand does not bound is searched again from the demand
            // (Q59; `Body::demand_bounds_lead`).
            if first.is_none()
                && fixed.is_none()
                && let Some(d) = b.own_demand()
                && got.0.0.first().is_some_and(|s| !b.demand_bounds_lead(s.at))
            {
                return self.plan_search(pf, b, Some(d), None, explain, cx);
            }
            return Ok(got);
        }
        SKEWING.with(|s| s.set(true));
        let got = self.plan_search(pf, b, first, fixed, explain, cx);
        SKEWING.with(|s| s.set(false));
        let got = got?;
        if fixed.is_none()
            && std::env::var_os("RERAIL_STATS_SKEW_LOG").is_some()
            && let Ok((truth, _)) = self.plan_search(pf, b, first, None, false, &mut joins::SearchCx::default())
            && let Ok((priced, _)) = self.plan_search(pf, b, first, Some(&got.0.0), false, &mut joins::SearchCx::default())
        {
            use std::io::Write as _;
            static LOG: std::sync::OnceLock<Option<std::sync::Mutex<std::fs::File>>> = std::sync::OnceLock::new();
            let log = LOG.get_or_init(|| std::env::var_os("RERAIL_STATS_SKEW_LOG").and_then(|p| std::fs::OpenOptions::new().create(true).append(true).open(p).ok()).map(std::sync::Mutex::new));
            if let Some(f) = log {
                let same = b.shown(&got.0.0) == b.shown(&truth.0);
                let head = b.head_text();
                let _ = writeln!(f.lock().unwrap(), "{:.6}\t{}\t{:.1}\t{:.1}\t{head}", priced.1 / truth.1.max(1e-9), u8::from(same), priced.1, truth.1);
            }
        }
        Ok(got)
    }

    /// The search, or with `fixed`, the pricing of that order (an error where it cannot run).
    fn plan_search(
        &self,
        pf: Fields<'_>,
        b: &Body<'_>,
        first: Option<usize>,
        fixed: Option<&[Step]>,
        explain: bool,
        cx: &mut joins::SearchCx,
    ) -> Result<(Plan, Option<super::profile::RulePlan>)> {
        let stats = &self.stats;
        let head = b.head_text();
        let rule = b.observed_key();
        let exact = joins::exact_for(rule);
        if fixed.is_none() {
            cx.runners_up.clear();
            cx.steps_of.clear();
        }
        // var → the constant an equality binds it to (`$k = #k1207`): read as the constant where an atom reads it.
        let mut consts: HashMap<VarSlot, Constant> = HashMap::new();
        for l in 0..b.len() {
            if let Lit::Cmp { l: x, op: CmpOp::Eq, r: y } = b.view(l) {
                match (x, y) {
                    (TermRef::Var(v), t) | (t, TermRef::Var(v)) if b.constant(t).is_some() => {
                        consts.insert(v, b.constant(t).expect("a constant"));
                    }
                    _ => {}
                }
            }
        }
        let constant_of = |t: TermRef<'_>| b.constant(t).or_else(|| t.var().and_then(|v| consts.get(&v).cloned()));
        // Those variables: bound, each is a constant, so it joins nothing. An atom that shares only such a variable
        // with what is joined is a cross product (Q183: `p299_attempt`'s `kind($m, $k)` after `$k = #k1249` read
        // every node of the kind for each of 15 k texts, 1.16 M rows for none kept, where `text($n, $t)` joined).
        let pinned = VarSet::of(consts.keys().copied());
        let joins_with = |t: TermRef<'_>, bound: &VarSet| matches!(t, TermRef::Var(v) if bound.contains(v) && !pinned.contains(v));
        let binds_some = |bound: &VarSet| !bound.minus(&pinned).is_empty();
        // var → constant kind, from kind($v, "k") atoms (the kind a constant, or a variable an equality binds to one).
        let mut kinds: HashMap<VarSlot, String> = HashMap::new();
        for l in 0..b.len() {
            if let Some(a) = b.plain_atom(l)
                && !a.neg()
                && b.rel(a) == "kind"
                && let (TermRef::Var(v), Some(k)) = (b.args(l)[0], constant_of(b.args(l)[1]))
            {
                // Lowered programs' kinds are kind atoms, and their statistics are keyed by the atoms' text.
                kinds.insert(v, match k {
                    Constant::Str(s) => s,
                    Constant::Int(i) => i.to_string(),
                });
            }
        }
        // The same binding domains are used by many candidate prefixes. Read the existing statistics once
        // per search, with no input recount and no additional SQL.
        let binding_domains: Vec<_> = (0..b.len()).filter(|&i| b.plain_atom(i).is_some_and(|a| !a.neg()))
            .map(|i| (i, b.vars(i).clone(), self.binding_domains(b, i))).collect();
        let delta_misses = self.delta_misses(b, first);
        let delta_vars = first.and_then(|f| b.atom(f)).map(body::atom_vars);
        let delta_size = first.and_then(|f| b.atom(f)).and_then(|a| self.stats.delta_rows.get(b.rel(a))).copied();
        let starts = |i: usize| first.is_none_or(|f| f == i || self.smaller_change(b, f, i));
        // Which rule priced the atom last (`atom_rows`), and the containment share it applied: the explain record
        // names it per step, so a misprediction shows the rule that made it.
        let basis: std::cell::Cell<&'static str> = std::cell::Cell::new("");
        let basis_hit = std::cell::Cell::new(1.0f64);
        let probed_by_delta = first.is_some();
        // Each body atom's column statistics, read once per search (O-live-replan, bulk planning): the lookups below
        // allocate a key per read (`(String, usize)`, a column's name), and a search reads them once per bound pattern.
        // None depends on what is bound.
        struct Col {
            /// A constant's postings (`dfreq`, the memo, an input's constant frequency), where counted.
            konst: Option<f64>,
            /// The rows a value holds: `fan`, else the relation over its distinct values.
            fan: f64,
            ndv: Option<f64>,
            ndv1: f64,
            /// A constant's rows counted for a column other than the most selective.
            counted: Option<f64>,
            /// Distinct values at least the rows: each row a value of its own.
            distinct: bool,
        }
        struct AtomCols {
            card: f64,
            input: bool,
            /// The relation's number, where it has statistics (`table::find`).
            id: Option<table::RelId>,
            /// Each column's, read when first asked: a re-price reads only the columns its order binds.
            cols: Vec<std::cell::OnceCell<Col>>,
        }
        impl AtomCols {
            fn col(&self, c: usize, b: &Body<'_>, i: usize, m: &Model) -> &Col {
                self.cols[c].get_or_init(|| {
                    let rel = b.rel(b.atom(i).expect("an atom"));
                    let (stats, card) = (&m.stats, self.card);
                    // The relation's number, once an atom (decision 0209 §3): its columns are array slots.
                    let id = self.id;
                    let ndv = id.and_then(|r| m.ndv.at_rel(r, c)).copied();
                    let ndv1 = m.ndv_or_default(rel, c);
                    let fan = id.and_then(|r| stats.fan.at_rel(r, c)).map(|f| (f * skew(rel, c, 1)).max(1.0)).unwrap_or_else(|| card / ndv1);
                    let (mut konst, mut counted) = (None, None);
                    if let Some(k) = b.constant(b.args(i)[c]) {
                        let dfreq = id.and_then(|r| stats.dfreq.at_rel(r, c));
                        let name = m.col(rel, c);
                        let freq = if self.input { stats.const_freq(rel, &name, &k) } else { None };
                        // A count is at least one row, as `card`: a constant absent from the chunk that plans is not
                        // absent from the chunks that reuse the plan (`frag_str`'s `doc_role(d, 17505)` priced 0.01
                        // rows, 2,455 found, each joined with every string: 7.1 M rows, none kept). The memo's count
                        // is exact (a delta's complete constant memo: an absence proof), and stays as it is.
                        konst = if let Some(d) = dfreq {
                            Some((d.get(&k).copied().unwrap_or(0) as f64).max(1.0))
                        } else if let Some(&f) = stats.memo.get(rel).and_then(|m| m.get(&(name.clone(), k.clone()))) {
                            Some(f as f64)
                        } else {
                            freq.map(|f| (f as f64).max(1.0))
                        };
                        counted = freq.map(|f| f as f64).or_else(|| dfreq.map(|d| d.get(&k).copied().unwrap_or(0) as f64));
                    }
                    Col { konst, fan, ndv, ndv1, counted, distinct: ndv.is_some_and(|n| n >= card) }
                })
            }
        }
        let cols_of = |i: usize| -> AtomCols {
            let a = b.atom(i).expect("an atom");
            let (rel, args) = (b.rel(a), b.args(i));
            static COUNT: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
            if *COUNT.get_or_init(|| std::env::var_os("RERAIL_PLAN_GREEDY_PROBE").is_some()) {
                // Bulk planning's sharing (a measurement aid): column tables made per search, against the relations
                // and (relation, constants) they are of.
                static SEEN: parking_lot::Mutex<(u64, Option<std::collections::HashSet<String>>, Option<std::collections::HashSet<String>>)> = parking_lot::Mutex::new((0, None, None));
                let mut g = SEEN.lock();
                g.0 += 1;
                g.1.get_or_insert_default().insert(rel.to_string());
                let with_constants = format!("{}{:?}", rel, args.iter().map(|t| b.constant(*t)).collect::<Vec<_>>());
                g.2.get_or_insert_default().insert(with_constants);
                if g.0.is_power_of_two() || g.0 % 10_000 == 0 {
                    eprintln!("column tables: {} made, {} relations, {} relations with their constants", g.0, g.1.as_ref().map_or(0, |s| s.len()), g.2.as_ref().map_or(0, |s| s.len()));
                }
            }
            AtomCols { card: self.card(rel), input: self.is_input(pf, rel), id: table::find(rel), cols: (0..args.len()).map(|_| std::cell::OnceCell::new()).collect() }
        };
        let atom_cols: std::cell::RefCell<Vec<Option<std::rc::Rc<AtomCols>>>> = std::cell::RefCell::new(vec![None; b.len()]);
        let cols_at = |i: usize| -> std::rc::Rc<AtomCols> {
            if let Some(t) = &atom_cols.borrow()[i] {
                return t.clone();
            }
            let t = std::rc::Rc::new(cols_of(i));
            atom_cols.borrow_mut()[i] = Some(t.clone());
            t
        };
        let atom_rows_ = |i: usize, bound: &VarSet| -> f64 {
            let a = b.atom(i).expect("an atom");
            let (rel, args) = (b.rel(a), b.args(i));
            basis_hit.set(1.0);
            // An atom that shares no bound variable, read after bindings exist, is a cross product: its fanout
            // observed in one wave or chunk is the relation's size then (none before a guest round makes the rows),
            // not a property of the join, so it is priced from the relation's own estimate (below). What was
            // observed comes before any rule that estimates: an absence proof from a snapshot was kept over 733,692
            // rows seen (`pfn`'s `pcall__ins`).
            let crossed = binds_some(bound) && !args.iter().any(|t| joins_with(*t, bound));
            // Exact pair statistics measured for a slow line (decision 0203 §3) come first: they were measured on
            // every row, where an observation is of the bindings one order saw.
            if !crossed
                && let Some(seen) = exact.as_ref().and_then(|e| e.get(&b.sig(i, bound)))
                && seen[0] >= OBS_MIN
            {
                basis.set("exact pair");
                return (seen[1] as f64 / seen[0] as f64).max(0.01);
            }
            if !crossed
                && let Some(seen) = stats.obs.get(&rule).and_then(|o| o.get(&b.sig(i, bound)))
                && seen[0] >= OBS_MIN
            {
                basis.set("observed");
                return (seen[1] as f64 / seen[0] as f64).max(0.01);
            }
            if delta_vars.as_ref().is_some_and(|v| v.is_subset(bound))
                && delta_misses.iter().any(|&j| j == i || b.same_atom(j, i)) {
                basis.set("absence proof");
                return 0.01;
            }
            // A variable an equality binds to a constant is that constant once bound (`$k = #k1207` placed): the
            // executor reads the atom with it then. Before, a scan reads every kind.
            if rel == "kind"
                && let Some(k) = args.get(1).and_then(|t| b.constant(*t).or_else(|| t.var().filter(|v| bound.contains(*v)).and_then(|_| constant_of(*t))))
            {
                basis.set("kind frequency");
                let key = match &k { Constant::Str(s) => s.clone(), Constant::Int(i) => i.to_string() };
                // A node bound by a field (`field($n, f, _)` its parent, `field(_, f, $n)` its child, placed) is of
                // kind `k` as often as `f`'s parents or children are (Q44: an attribute's `object` field's parent is
                // an attribute every time, where the kinds' shares of the nodes gave it 6%).
                if let TermRef::Var(v) = args[0]
                    && bound.contains(v)
                    && let Some(share) = (0..b.len())
                        .filter(|&j| j != i)
                        .filter_map(|j| {
                            b.plain_atom(j).filter(|o| !o.neg() && b.rel(*o) == "field")?;
                            let a = b.args(j);
                            if a.len() != 3 || !b.vars(j).is_subset(bound) {
                                return None;
                            }
                            let f = match b.constant(a[1])? { Constant::Str(s) => s, Constant::Int(i) => i.to_string() };
                            // With the other end's kind known (a kind atom names it), the share among that kind's
                            // edges by `f`: an aliased import's `name` is an identifier, though most `name`s'
                            // parents are functions and classes, whose names are too.
                            let other = |t: TermRef<'_>| t.var().and_then(|x| kinds.get(&x));
                            match (a[0], a[2]) {
                                (TermRef::Var(x), c) if x == v => other(c).and_then(|ck| stats.fields.joint_share(&key, ck, &f, true)).or_else(|| stats.fields.parent_share(&key, &f)),
                                (p, TermRef::Var(x)) if x == v => other(p).and_then(|pk| stats.fields.joint_share(pk, &key, &f, false)).or_else(|| stats.fields.child_share(&key, &f)),
                                _ => None,
                            }
                        })
                        .reduce(f64::min)
                {
                    basis.set("field statistics");
                    return share.clamp(0.001, 1.0);
                }
                if matches!(args[0], TermRef::Var(v) if bound.contains(v)) {
                    // A bound node's kind check passes the kind's share of the nodes: each node has one kind. At 1.0
                    // no check filtered, and a join's later steps were priced over every binding (decision 0209:
                    // the self-join seed's `kind($k, identifier)` passed 5,526 bindings where it keeps a fifth).
                    let nodes = self.card("kind");
                    return match self.kind_frequency(&k) {
                        Some(n) if nodes >= 1.0 => (n as f64 / nodes).clamp(0.001, 1.0),
                        _ => 1.0,
                    };
                }
                return self.kind_frequency(&k).map_or(0.5, |n| (n as f64).max(0.01));
            }
            // `kind($c, $ck)` with `$ck` bound by an atom whose values in that column are counted (`dfreq`: a
            // relation of facts, or a derived one of few values): the nodes of those kinds, each weighted by how
            // often it binds (decision 0299). `region`'s `comp_kind($ck, $k), kind($c, $ck)` reads four rare kinds,
            // priced at every node over 88 kinds.
            if rel == "kind"
                && !matches!(args[0], TermRef::Var(v) if bound.contains(v))
                && b.constant(args[0]).is_none()
                && let TermRef::Var(k) = args[1]
                && bound.contains(k)
                && let Some(rows) = (0..b.len()).filter(|&j| j != i).find_map(|j| {
                    let o = b.plain_atom(j).filter(|o| !o.neg())?;
                    let c = b.args(j).iter().position(|t| matches!(t, TermRef::Var(x) if *x == k))?;
                    let values = stats.dfreq.at(b.rel(o), c)?;
                    let (mut nodes, mut draws) = (0.0f64, 0.0f64);
                    for (value, &count) in values {
                        nodes += count as f64 * self.kind_frequency(value)? as f64;
                        draws += count as f64;
                    }
                    (draws > 0.0).then(|| nodes / draws)
                })
            {
                basis.set("kind frequency");
                return rows.max(0.01);
            }
            let cols: Vec<usize> = (0..args.len())
                .filter(|&c| b.constant(args[c]).is_some() || matches!(args[c], TermRef::Var(v) if bound.contains(v)))
                .collect();
            // `within(n, a)` read from its container: `a`'s descendants, its subtree's size (decision 0299). The view
            // is made per node looked up, so its rows' statistics are of the lookups made so far, mostly from a node
            // up to its few ancestors: `pth_deferred` read a scope's descendants priced at 14.5 rows, 30 M rows for
            // 0.24 M priced. The container's kind's mean size where a kind atom names it, else the mean size of a
            // node with children (`Stats::subtree_fan`).
            if rel == "within"
                && cols == [1]
                && stats.nodes > 0.0
                && let Some(size) = match args[1] {
                    TermRef::Var(v) => kinds.get(&v).and_then(|k| stats.size.get(k)).copied().or_else(|| stats.subtree_fan()),
                    _ => stats.subtree_fan(),
                }
            {
                basis.set("subtree size");
                return size.max(1.0);
            }
            // `field($p, f, $c)` by its parent, of a kind a kind atom names: that kind's children by `f` a node
            // (Q44: a function's `name`, one each, was priced at `name`'s share of every node, 0.05). By its child:
            // the share of the child's kind that is a child by `f`.
            if rel == "field"
                && args.len() == 3
                && let Some(f) = b.constant(args[1])
                && let Some((at, parent)) = match (args[0], args[2]) {
                    (TermRef::Var(p), TermRef::Var(c)) if bound.contains(p) && !bound.contains(c) => Some((p, true)),
                    (TermRef::Var(p), TermRef::Var(c)) if bound.contains(c) && !bound.contains(p) => Some((c, false)),
                    _ => None,
                }
                && let Some(k) = kinds.get(&at)
                && let Some(&nodes) = stats.kfreq.get(k)
                && nodes > 0
                && let Some(edges) = stats.fields.edges(k, &match f { Constant::Str(s) => s, Constant::Int(i) => i.to_string() }, parent)
            {
                basis.set("field statistics");
                let per = edges / nodes as f64;
                return if parent { per.max(0.01) } else { per.clamp(0.01, 1.0) };
            }
            if cols.is_empty() {
                basis.set("relation size");
                return self.card(rel);
            }
            basis.set("postings");
            // The most selective bound column gives its expected postings (exact for a constant on an input); the
            // other bound columns divide by their NDV.
            let t = cols_at(i);
            let postings = |c: usize| -> f64 {
                // Retained derived inputs and maintenance deltas have measured postings too (`dfreq`, the memo).
                // A missing entry is not an absence proof unless this is an input's complete constant memo.
                if let Some(f) = t.col(c, b, i, self).konst {
                    return f;
                }
                // A literal whose rows are not counted: one value's share, rows / distinct (`unknown_constant_rows`).
                // Its fanout (rows per value, weighted by rows) overweights hot keys, and a literal is not drawn
                // from the relation's rows (async-family: the embed seed's estimate 141,009 -> 59,625 work).
                if b.constant(args[c]).is_some() {
                    return t.card / t.col(c, b, i, self).ndv1;
                }
                if let TermRef::Var(v) = args[c] {
                    let distinct_demand = binding_domains.iter().any(|(d, vars, _)| {
                        let dr = b.rel(b.atom(*d).expect("an atom"));
                        *d != i && vars.is_subset(bound)
                            && (dr.ends_with("__plus") || dr.ends_with("__minus"))
                            && !stats.solving.contains(dr)
                            && { let dt = cols_at(*d); b.args(*d).iter().enumerate().any(|(dc, t)| matches!(t, TermRef::Var(x) if *x == v) && dt.col(dc, b, *d, self).distinct) }
                    });
                    if distinct_demand { return t.card / t.col(c, b, i, self).ndv1; }
                    // Decision 0209 §4: where this column and each placed atom's column holding `v` keep their heavy
                    // values, the binding is drawn value by value through those atoms, so a value common in all is
                    // drawn as often as it is. A fanout (sq / rows) is that for one atom placed; a three-way
                    // self-join on a skewed key wants the third moment (0.51 of its size from sq on the fixture).
                    if let Some(here) = self.heavy_of(rel, c) {
                        let mut by = Vec::new();
                        let mut all = true;
                        for (d, vars, _) in &binding_domains {
                            if *d == i || !vars.is_subset(bound) { continue; }
                            let dr = b.rel(b.atom(*d).expect("an atom"));
                            // Its rows reach `v` weighted by their own counts only where no other of its variables
                            // joins another atom: one reached through another column (`kind($m, return)`, then
                            // `inf($f, $m)`) draws `v` by that join, a few returns a function whatever its size,
                            // which only measured pairs see (0203 §3).
                            let joined_else = b.args(*d).iter().any(|t| matches!(t, TermRef::Var(x) if *x != v
                                && binding_domains.iter().any(|(e, evars, _)| *e != *d && evars.contains(*x))));
                            if joined_else { all = false; }
                            for (dc, t) in b.args(*d).iter().enumerate() {
                                if matches!(t, TermRef::Var(x) if *x == v) {
                                    match self.heavy_of(dr, dc) { Some(h) => by.push(h), None => all = false }
                                }
                            }
                        }
                        if all && !by.is_empty() && let Some(f) = table::heavy_fan(here, &by) {
                            basis.set("heavy values");
                            return f.max(0.01);
                        }
                    }
                }
                t.col(c, b, i, self).fan
            };
            let mut first = cols[0];
            for &c in &cols[1..] {
                if postings(c) < postings(first) {
                    first = c;
                }
            }
            let mut rows = postings(first);
            // A constant's share of the rows where its postings are counted, else one value's share: a hot value is
            // not one of `ndv`. `pv($o, 218, $k)` by `$k`, 644 rows a value, was priced at 92 a binding (`218` one
            // of 7 values) where it holds 43% of the rows: 382 were read, and the plan probed it before `icall`.
            let card = t.card;
            for &c in &cols {
                if c != first {
                    match t.col(c, b, i, self).counted {
                        Some(f) if card > 0.0 => rows *= (f / card).min(1.0),
                        _ => rows /= t.col(c, b, i, self).ndv1,
                    }
                }
            }
            // Several columns bound by variables, not all: at least the rows of one combination of their values the
            // relation holds (decision 0299), where the containment below says how many bindings hit. Dividing by
            // each column's distinct values takes the columns as independent, and one often determines another:
            // `walk($n, $x, $s)` by `$n` and `$x`, `$x` being `$n`'s name, was priced at 0.0007 rows a binding for
            // about 2 (`annotated_on_chain`: 6.8 M rows of work for 0.03 M priced).
            if cols.len() >= 2 && cols.len() < args.len() && cols.iter().all(|&c| b.constant(args[c]).is_none()) && card > 0.0 {
                let combinations = cols.iter().map(|&c| t.col(c, b, i, self).ndv1).product::<f64>().clamp(1.0, card);
                rows = rows.max(card / combinations);
            }
            // A relation probed by a value an earlier step bound: a column's fanout
            // is the rows per value it holds (`fan`), so a probe is priced as if it always hits. Most do not where
            // the binding ranges over more values than the column holds: `fbind`'s `fcall($c, $f, $i, $a)` by `$a`,
            // 399 rows, probed by `pv($a, "inst", $k)`'s 280,000, was priced at a row a probe and planned after
            // `tfix` by `$k` (19 M rows of work for 220). The probes that hit are the column's share of the binding's
            // values (containment): its distinct values over the binding's, where both are measured (live counts),
            // the binding's by the fewest distinct values of an atom of bound variables that holds it, capped
            // by its constant filters. This also applies to restrictive inputs outside the recursive component.
            // Not for a maintenance delta of the component being solved (`changes_with_the_delta`): its rows are the
            // ones this change makes, as the delta's are, so the delta's values are the ones it holds, and its
            // distinct values measured mid-fixpoint are a snapshot of a relation still filling. `pfn`'s
            // `pcall__ins` by `$o`, probed by `pv`'s delta, was priced at 0.01 rows a binding and gave 20.8.
            if !probed_by_delta || !self.changes_with_the_delta(rel) {
                let mut hit = 1.0f64;
                for &c in &cols {
                    let TermRef::Var(v) = args[c] else { continue };
                    let Some(here) = t.col(c, b, i, self).ndv else { continue };
                    let from = binding_domains
                        .iter()
                        .filter_map(|(d, vars, domains)| {
                            if *d == i || !vars.is_subset(bound) { return None; }
                            b.args(*d).iter().position(|t| matches!(t, TermRef::Var(x) if *x == v)).and_then(|c2| domains[c2])
                        })
                        .fold(f64::INFINITY, f64::min);
                    if from.is_finite() && from > here.max(1.0) {
                        hit = hit.min(here.max(1.0) / from);
                    }
                }
                rows *= hit;
                basis_hit.set(hit);
            }
            // Every column bound: a membership test, at most one row of a set.
            // A tree child is also unique by (parent, ordinal). Marginal hot-value
            // frequencies cannot multiply this lookup into several children.
            if rel == "child" && self.is_input(pf, rel) && cols.contains(&0) && cols.contains(&2) {
                rows = rows.min(1.0);
            }
            if cols.len() == args.len() && !pf.lattice(rel) {
                if rows > 1.0 { basis.set("membership"); }
                rows = rows.min(1.0);
                // A text test (decision 0037) probed by a string an input binds keeps the share of that column's
                // distinct strings it holds: both are measured per chunk. Priced at 1, a test of every identifier's
                // text looked free after the scan, and the planner never started from the test (notes T10, P6).
                if rel.starts_with(super::strings::RELATION)
                    && let [TermRef::Var(v)] = *args
                {
                    let domain = (0..b.len())
                        .filter_map(|j| match b.plain_atom(j) {
                            Some(o) if !o.neg() && b.rel(o) != rel && self.is_input(pf, b.rel(o)) => {
                                b.args(j).iter().position(|t| matches!(t, TermRef::Var(x) if *x == v)).map(|c| self.ndv_or_default(b.rel(o), c))
                            }
                            _ => None,
                        })
                        .fold(f64::INFINITY, f64::min);
                    if domain.is_finite() {
                        rows = rows.min(self.card(rel) / domain);
                    }
                }
            }
            rows.max(0.01)
        };
        let atom_rows = |i: usize, bound: &VarSet| -> f64 {
            let a = b.atom(i).expect("an atom");
            let (rel, args) = (b.rel(a), b.args(i));
            let mut rows = atom_rows_(i, bound);
            if first == Some(i) {
                if let Some(n) = delta_size && n.max(0.01) < rows { rows = n.max(0.01); basis.set("delta size"); }
            }
            // A cross product never filters a binding: each one reads every matching row, and a relation empty
            // where the plan was priced is not empty where it is reused. At least one row per binding, so a connected
            // atom of fanout one is never ordered after it.
            let delta_atom = first == Some(i) && b.plain_atom(i).is_some();
            if !delta_atom && !a.neg() && binds_some(bound) && !args.iter().any(|t| joins_with(*t, bound)) && rows < 1.0 {
                rows = 1.0;
                basis.set("cross product");
            }
            // No variable bound on a relation of the component being solved: a scan of it per binding, priced as a
            // pass over the corpus (shared among its constants' values) at least.
            // The delta atom of a probe is not such a scan: it reads the last wave's rows, priced by the relation's
            // live size (an upper bound), not the corpus.
            let delta = first == Some(i) && b.plain_atom(i).is_some();
            // A first scan (nothing bound yet) of a member that had rows when the component opened reads those rows
            // once: its card, measured then (`Stats::sized`). A whole plan priced by the floor took `pv` (65 k rows)
            // first, tied with `decfn` (1 k) at the corpus's 180 k, and read it in full (O125 step 07).
            let sized = bound.is_empty() && stats.sized.contains(rel);
            if !delta && !sized && stats.solving.contains(rel) && !args.iter().any(|t| matches!(t, TermRef::Var(v) if bound.contains(*v))) {
                let mut floor = stats.nodes;
                for (c, t) in args.iter().enumerate() {
                    if b.constant(*t).is_some() {
                        floor /= self.ndv_or_default(rel, c);
                    }
                }
                if floor > rows { rows = floor; basis.set("scan of the component"); }
            }
            rows
        };
        // The rule `atom_rows` priced `a` by, as the explain record prints it.
        let basis_of = |i: usize, bound: &VarSet| -> String {
            let _ = atom_rows(i, bound);
            let hit = basis_hit.get();
            if hit < 1.0 { format!("{}, containment {hit:.3}", basis.get()) } else { basis.get().to_owned() }
        };
        // (column, rows) if `a` can be read as a range inside one document: a body literal `docof($v, $d)` has $d
        // bound and $v unbound, $v is `a`'s column, and an input index on `a` is the bound column then that column.
        let doc_range = |i: usize, bound: &VarSet| -> Option<(usize, f64)> {
            let a = b.atom(i).expect("an atom");
            let (rel, args) = (b.rel(a), b.args(i));
            if stats.docs.is_none() || !self.is_input(pf, rel) {
                return None;
            }
            for j in 0..b.len() {
                let Some(d) = b.plain_atom(j) else { continue };
                if d.neg() {
                    continue;
                }
                let Some(&(nc, dc)) = self.docof.get(b.rel(d)) else { continue };
                let (TermRef::Var(v), dv) = (b.args(j)[nc], b.args(j)[dc]) else { continue };
                if bound.contains(v) || !(b.constant(dv).is_some() || matches!(dv, TermRef::Var(x) if bound.contains(x))) {
                    continue;
                }
                let rcols: Vec<usize> = (0..args.len()).filter(|&c| matches!(args[c], TermRef::Var(x) if x == v)).collect();
                let bcols: Vec<usize> = (0..args.len())
                    .filter(|&c| b.constant(args[c]).is_some() || matches!(args[c], TermRef::Var(x) if bound.contains(x)))
                    .collect();
                if rcols.len() != 1 || bcols.len() != 1 {
                    continue;
                }
                let (rname, bname) = (self.col(rel, rcols[0]), self.col(rel, bcols[0]));
                if !stats.indexes.get(rel).into_iter().flatten().any(|ix| ix.len() >= 2 && ix[0] == bname && ix[1] == rname) {
                    continue;
                }
                let mut rows = stats.fan_doc(rel, &bname);
                if let Some(k) = b.constant(args[bcols[0]]) {
                    let f = stats.const_freq(rel, &bname, &k).filter(|&f| f != 0).map_or(rows, |f| f as f64);
                    rows = rows.min(f);
                }
                return Some((rcols[0], rows));
            }
            None
        };
        // (work, rows out, mode) of adding join literal `l`, or None if it can't run yet. A step's work is the rows it
        // passes on, as the executor counts a line's (`profile::plan_work`: the rows entering each step, and the last
        // step's out): the rows a join probes with are the step before's (decision 0297).
        let long = (0..b.len()).filter(|&i| b.is_join(i)).count() > joins::greedy_limit(GREEDY, cx.wide);
        // Past the step that binds the head's last variable, the native walk needs one witness (`setfix::tail`): a
        // step there passes on at most the bindings it gets, and reads no further rows of them than it passes on. A
        // factorized part's key, `fzk($d) :- demand($d), node($d, $v, …), col($v, $c)`, was planned from `col` (31,191
        // rows est., 4 M read) where `demand` first reads one node a document.
        let head_vars = body::atom_vars(b.head());
        // What is bound before the first step: a recheck's head (`Body::with_head_bound`, Q208), one candidate row.
        let pre = b.bound_before();
        let step_all = |l: usize, rows: f64, bound: &VarSet| -> Option<(f64, f64, Option<Mode>)> {
            // In a long body's greedy order, a step that reads with no bound variable (a scan, a cross product) is
            // priced from at least one binding (Postgres's `clamp_row_est`): an estimate near 0 would make every next
            // step look free (decision 0038). The search keeps T's pricing, which takes a cross product after an
            // unfilled relation (`magic_returns`) on purpose; a kept plan made where a relation was empty is re-priced
            // in the next chunk and replanned where it grew.
            let unbound = matches!(b.view(l), Lit::Atom(_)) && !b.args(l).iter().any(|t| matches!(t, TermRef::Var(v) if bound.contains(*v)));
            let rows = if long && unbound && !bound.is_empty() { rows.max(1.0) } else { rows };
            match b.view(l) {
                Lit::Atom(_) => {
                    let out = rows * atom_rows(l, bound);
                    if let Some((_, r)) = doc_range(l, bound)
                        && rows * r < out
                    {
                        let out = rows * r;
                        return Some((out, out, Some(Mode::Range)));
                    }
                    Some((out, out, None))
                }
                Lit::Nearest { n, a, kindset } => {
                    if !b.ready(l, bound) {
                        return None;
                    }
                    if bound.contains(n) {
                        // Climb from $n: one parent read per expected step; a bound $a just filters.
                        return Some((rows * stats.steps(kinds.get(&n).map(String::as_str), b.v.text(kindset)), rows, Some(Mode::Climb)));
                    }
                    // Descend from $a: every node walked.
                    let size = kinds.get(&a).and_then(|k| stats.size.get(k)).copied().unwrap_or(stats.nodes / 1000.0);
                    Some((rows * size, rows * size, Some(Mode::Descend)))
                }
                _ => None,
            }
        };
        let step = |l: usize, rows: f64, bound: &VarSet| -> Option<(f64, f64, Option<Mode>)> {
            let (w, out, mode) = step_all(l, rows, bound)?;
            if out > rows && head_vars.is_subset(bound) {
                let atom = matches!(b.view(l), Lit::Atom(_));
                return Some((if atom { w - out + rows } else { w }, rows, mode));
            }
            Some((w, out, mode))
        };
        // A computed value (`$s = @rr_node_text($n)`) waits while a positive atom still to join reads its arguments
        // and not the value: that atom only narrows the rows, so computing it first computes (and interns) values for
        // rows it drops. `def_name`'s long-text twin seeded every node past the text bound and read each one's text
        // before `field($f, "name", $n)` kept none: 77 MB of module, class and function texts in a posthog store.
        let call_waits = |f: usize, bound: &dyn Fn(VarSlot) -> bool, open: &dyn Fn(usize) -> bool| -> bool {
            let Lit::Cmp { l, op: CmpOp::Eq, r } = b.view(f) else { return false };
            let (out, call) = match (l, r) {
                (TermRef::Var(v), c @ TermRef::Call(..)) | (c @ TermRef::Call(..), TermRef::Var(v)) if !bound(v) => (v, c),
                _ => return false,
            };
            let mut ins = VarSet::new();
            body::term_vars(call, &mut ins);
            (0..b.len()).filter(|&j| b.is_join(j)).any(|j| {
                open(j) && matches!(b.view(j), Lit::Atom(a) if !a.neg()) && {
                    let vs = b.vars(j);
                    !vs.contains(out) && !vs.is_disjoint(&ins)
                }
            })
        };
        // Applies every filter (negation, comparison, aggregate) whose variables are bound: the rows out, and the
        // filters' work, each the rows it passes on (decision 0297).
        let close = |order: &mut Vec<Step>, mut rows: f64, bound: &mut VarSet, rest: &mut Vec<usize>| -> (f64, f64) {
            let mut work = 0.0;
            let mut progressed = true;
            while progressed {
                progressed = false;
                for i in rest.clone() {
                    let l = i;
                    if !b.is_join(l) && b.ready(l, bound) && !call_waits(i, &|v| bound.contains(v), &|j: usize| !order.iter().any(|s| s.at == j)) {
                        order.push(Step { at: i, mode: None });
                        rest.retain(|&j| j != i);
                        bound.extend(b.vars(l));
                        progressed = true;
                        rows *= b.filter_pass(l);
                        work += rows;
                    }
                }
            }
            (rows, work)
        };
        if let Some(l) = (0..b.len()).find(|&i| matches!(b.view(i), Lit::Ast { .. })) {
            return Err(Error::Plan(format!("rule for {head}: no executor for {} yet", b.show(l))));
        }
        let joins: Vec<usize> = (0..b.len()).filter(|&i| b.is_join(i)).collect();
        SEED.with(|c| c.set(joins.iter().filter_map(|&i| step(i, 1.0, &VarSet::new()).map(|w| w.0)).fold(f64::INFINITY, f64::min).min(1e300)));
        if !joins.is_empty() && !joins.iter().any(|&i| matches!(b.view(i), Lit::Atom(_))) {
            return Err(Error::Plan(format!("rule for {head} has no positive atom")));
        }
        let waits = if NO_SHAPES.with(std::cell::Cell::get) { Vec::new() } else { self.shape_waits(b, first, &joins) };
        let shaped: Vec<String> = { let mut t: Vec<String> = waits.iter().map(|w| w.3.to_owned()).collect(); t.dedup(); t };
        let shape_waits = |i: usize, placed: u64, bound: &dyn Fn(VarSlot) -> bool| -> bool {
            waits.iter().any(|(k, any, vars, _)| *k == i && ((!any.is_empty() && !any.iter().any(|j| placed & (1 << j) != 0)) || vars.iter().any(|v| !bound(*v))))
        };
        let mut candidates = 0u64;
        // A given order, priced now: each join step as the search prices it, each filter once ready. A kept order
        // (`fixed`), or a start the search tries first (`SearchCx::start`, decision 0258).
        let price = |fixed: &[Step], candidates: &mut u64| -> Result<(f64, f64, Vec<Step>)> {
            let (mut work, mut rows, mut bound, mut order) = (0.0, 1.0, pre.clone(), Vec::new());
            if fixed.len() != b.len() {
                return Err(Error::Plan("a kept plan of another body".into()));
            }
            let mut placed: u64 = 0;
            for s in fixed {
                let l = s.at;
                if l >= b.len() {
                    return Err(Error::Plan("a kept plan of another body".into()));
                }
                if let Some(i) = joins.iter().position(|&j| j == s.at) {
                    if shape_waits(i, placed, &|v: VarSlot| bound.contains(v)) {
                        return Err(Error::Plan("a kept plan its join shape directives do not admit".into()));
                    }
                    placed |= 1 << i;
                }
                if b.is_join(l) {
                    let (w, out, mode) = step(l, rows, &bound).ok_or_else(|| Error::Plan("a kept step cannot run".into()))?;
                    *candidates += 1;
                    work += w;
                    rows = out;
                    order.push(Step { at: s.at, mode });
                } else {
                    if !b.ready(l, &bound) {
                        return Err(Error::Plan("a kept filter cannot run".into()));
                    }
                    rows *= b.filter_pass(l);
                    work += rows;
                    order.push(s.clone());
                }
                bound.extend(b.vars(l));
            }
            Ok((work, rows, order))
        };
        let (work, rows, order, subsets) = if joins.is_empty() {
            // A body of filters and aggregates alone (`n = count : { … }`) runs over the relational unit, one empty
            // binding: ground negations and constant expressions filter it; an unbound variable still fails.
            let (mut order, mut bound, mut rest) = (Vec::new(), pre.clone(), (0..b.len()).collect());
            let (rows, _) = close(&mut order, 1.0, &mut bound, &mut rest);
            if !rest.is_empty() {
                let rest: Vec<String> = rest.iter().map(|&i: &usize| b.show(i)).collect();
                let all: Vec<String> = (0..b.len()).map(|i| b.show(i)).collect();
                return Err(Error::Plan(format!("unbound literals in {head}: {rest:?} (body {all:?})")));
            }
            (1.0, rows, order, 1)
        } else if let Some(fixed) = fixed {
            let (work, rows, order) = price(fixed, &mut candidates)?;
            (work, rows, order, 0)
        } else {
        type Best = (f64, f64, Vec<Step>, VarSet, Vec<usize>);
        let mut subsets_reached = 0u64;
        let mut best_of: HashMap<u64, Best> = HashMap::new();
        best_of.insert(0, {
            let (mut order, mut bound, mut rest) = (Vec::new(), pre.clone(), (0..b.len()).filter(|&i| !b.is_join(i)).collect());
            // A recheck's tests of its head alone run before any join (Q208).
            let (rows, work) = if pre.is_empty() { (1.0, 0.0) } else { close(&mut order, 1.0, &mut bound, &mut rest) };
            (work, rows, order, bound, rest)
        });
        // The greedy order's next step shares a variable with what is joined, while one can (no cross product while a
        // connected step remains, decision 0038). The search prices cross products itself: a cheap one with a tiny
        // relation is T's choice on its programs (`dec1` before `pv`: work 67, not 7,001).
        let connected = |l: usize, bound: &VarSet| { let live = bound.minus(&pinned); live.is_empty() || !b.vars(l).is_disjoint(&live) };
        let full_mask: u64 = (0..joins.len()).map(|i| 1u64 << i).sum();
        // A gate (`answers::gate`'s `done_N_ad(t)` before `!N(t)`) holds only keys its rule's own literals give
        // (its `ask` is the body before the negation), so it never yields a row they would not: it is a test of a
        // bound key, joined once its key is bound and never crossed in to bind it. Its size is the size of what is
        // answered so far, which the stats of an earlier wave do not know. As the delta, it anchors as any atom.
        let gate_waits = |i: usize, bound: &dyn Fn(VarSlot) -> bool| -> bool {
            matches!(b.view(joins[i]), Lit::Atom(a) if super::answers::is_gate(b.rel(a)))
                && (0..b.len()).any(|l| matches!(b.view(l), Lit::Atom(n) if n.neg() && n.gated()))
                && first != Some(joins[i])
                && b.vars(joins[i]).iter().any(|v| !bound(v))
                && (0..joins.len()).any(|j| j != i)
        };
        let disconnected = |i: usize, prev: u64, bound: &VarSet| {
            !connected(joins[i], bound) && (0..joins.len()).any(|j| j != i && prev & (1 << j) == 0 && connected(joins[j], bound))
        };
        // The search's variables as bits: a body with more than 128 is ordered greedily, as a long one is.
        let mut var_ids: Vec<Option<usize>> = vec![None; b.rule.var_count()];
        let mut n_vars = 0usize;
        for l in 0..b.len() {
            for v in b.vars(l).iter() {
                if var_ids[v.index()].is_none() {
                    var_ids[v.index()] = Some(n_vars);
                    n_vars += 1;
                }
            }
        }
        // No cross product priced from a default size (O-join-shape-directives, item 4): while a connected step remains,
        // a step sharing no variable with what is joined is not taken where it or a connected step has no size of its
        // own (`card`'s 1,000 rows): `pv__ins` crossed `pv` with `retparam` before an unsized 15-row `posarg__plus`.
        let jmask: Vec<u128> = joins.iter().map(|&j| b.vars(j).iter().filter(|&v| !pinned.contains(v)).filter_map(|v| var_ids[v.index()]).fold(0u128, |m, k| if k < 128 { m | 1u128 << k } else { m })).collect();
        // The joins with no size of their own, read from the statistics once per search.
        let no_size: u64 = (0..joins.len()).filter(|&i| matches!(b.view(joins[i]), Lit::Atom(a) if !a.neg() && { let r = b.rel(a); r != "node" && !self.stats.card.contains_key(r) && !self.stats.lazy.0.contains_key(r) && !self.stats.solving.contains(r) })).fold(0, |m, i| m | 1 << i);
        // The joins of a relation with no rows of its own when priced: unsized, or empty (its rows come later, as a
        // guest round's do). Not the delta, whose rows are its wave's. Read once per search, as `no_size` is.
        let no_rows: u64 = (0..joins.len())
            .filter(|&i| {
                first != Some(joins[i])
                    && (no_size >> i & 1 == 1 || matches!(b.view(joins[i]), Lit::Atom(a) if !a.neg() && self.stats.card.get(b.rel(a)).is_some_and(|&n| n <= 0.0)))
            })
            .fold(0, |m, i| m | 1 << i);
        let no_rows_when_priced = |i: usize| no_rows >> i & 1 == 1;
        let unfounded_cross = |i: usize, prev: u64, bound: u128| -> bool { unfounded_cross(i, prev, bound, &jmask, no_size) };
        // The greedy order (Postgres's `geqo_threshold`): the cheapest connected step next. Long bodies take it; under
        // `RERAIL_PLAN_GREEDY_PROBE` (a measurement aid) every searched body is also ordered so, and compared.
        let greedy = |candidates: &mut u64| -> Option<Best> {
            let (mut work, mut rows, mut order, mut bound, mut rest) = best_of.get(&0).cloned().expect("the empty order");
            let mut done: u64 = 0;
            while done != full_mask {
                let mut best: Option<(usize, f64, f64, Option<Mode>)> = None;
                for i in 0..joins.len() {
                    let l = joins[i];
                    if done & (1 << i) != 0 || order.is_empty() && (!matches!(b.view(l), Lit::Atom(_)) || !starts(joins[i])) {
                        continue;
                    }
                    if disconnected(i, done, &bound) || gate_waits(i, &|v: VarSlot| bound.contains(v)) || shape_waits(i, done, &|v: VarSlot| bound.contains(v)) {
                        continue;
                    }
                    let Some((w, out, mode)) = step(l, rows, &bound) else { continue };
                    *candidates += 1;
                    if best.is_none_or(|b| w < b.1) {
                        best = Some((i, w, out, mode));
                    }
                }
                let Some((i, w, out, mode)) = best else { break };
                order.push(Step { at: joins[i], mode });
                bound.extend(b.vars(joins[i]));
                let (closed, filtered) = close(&mut order, out, &mut bound, &mut rest);
                rows = closed;
                work += w + filtered;
                done |= 1 << i;
            }
            (done == full_mask).then_some((work, rows, order, bound, rest))
        };
        static PROBE: std::sync::OnceLock<(bool, bool)> = std::sync::OnceLock::new();
        let (probe, greedy_first) = *PROBE.get_or_init(|| (std::env::var_os("RERAIL_PLAN_GREEDY_PROBE").is_some(), std::env::var("RERAIL_PLAN_GREEDY_FIRST").map_or(true, |v| v != "off")));
        let greedy_probe = (probe && joins.len() >= 3).then(|| { let mut c = 0; let g = greedy(&mut c); (g, c) });
        // Greedy first (decision 0123 §7; `RERAIL_PLAN_GREEDY_FIRST=off` searches as before): its order where its work
        // is under what the search would cost (`search_rows`), so the order a search finds could save less than the
        // search costs. An order so taken that ran over a search's work is searched at its next plan
        // (`plan_rule_reused`, `mispriced`).
        // A start (decision 0258): the order it was given, where it runs, is led by the line's delta and prices under
        // what the search would cost. Priced under this search's set-up, so a start refused costs its steps alone.
        let started = cx.start.take().and_then(|o| price(&o, &mut candidates).ok())
            .filter(|(work, _, order)| *work <= search_rows(b, first) && self.valid_live_driver(order, b, first))
            .map(|(work, rows, order)| {
                let mut bound = VarSet::new();
                for l in 0..b.len() { bound.extend(b.vars(l)); }
                (work, rows, order, bound, Vec::new())
            });
        cx.started = started.is_some();
        let cheap_greedy = (started.is_none() && greedy_first && !cx.wide && fixed.is_none() && joins.len() >= 3 && joins.len() <= GREEDY)
            .then(|| greedy(&mut candidates))
            .flatten()
            .filter(|g| g.0 <= search_rows(b, first));
        if started.is_none() {
            SEARCHES[usize::from(cheap_greedy.is_some())].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        if let Some(g) = started {
            best_of.insert(full_mask, g);
        } else if let Some(g) = cheap_greedy {
            super::profile::decision("plan cache (decision 0038)", || "greedy first: its work under a search's".into());
            best_of.insert(full_mask, g);
        } else if joins.len() > joins::greedy_limit(GREEDY, cx.wide) || n_vars > 128 {
            if let Some(g) = greedy(&mut candidates) {
                best_of.insert(full_mask, g);
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
            /// Its rows rest only on relations with no rows of their own when priced (empty, or unsized): a
            /// cross product taken from it is priced from at least a search's rows (`SEARCH_WORK`), not theirs.
            unfounded: bool,
            /// Its last join's rows out a row in, and the rule that priced them (decision 0354's features).
            fan: f64,
            basis: &'static str,
        }
        let filters: Vec<usize> = (0..b.len()).filter(|&i| !b.is_join(i)).collect();
        // Each filter's share of rows passed on (decision 0297); a body's first 128 filters are priced.
        let pass: Vec<f64> = filters.iter().map(|&f| b.filter_pass(f)).collect();
        // Variables as bits (`var_ids`): a subset's bound variables are a mask, and a set of names is made only to
        // price a join or test a filter the first time its own variables are bound so.
        let names: Vec<VarSlot> = {
            let mut n: Vec<(usize, VarSlot)> = var_ids.iter().enumerate().filter_map(|(s, k)| k.map(|k| (k, VarSlot(s as u32)))).collect();
            n.sort();
            n.into_iter().map(|(_, v)| v).collect()
        };
        let bits_of = |vs: &VarSet| vs.iter().fold(0u128, |m, v| m | 1u128 << var_ids[v.index()].expect("a body variable"));
        let set_of = |bits: u128| -> VarSet { VarSet::of(names.iter().enumerate().filter(|(k, _)| bits >> k & 1 == 1).map(|(_, v)| *v)) };
        let lit_bits: Vec<u128> = (0..b.len()).map(|l| bits_of(b.vars(l))).collect();
        // Whether a filter is ready reads only its own variables (`ready`, kept by them: `Readiness`).
        let mut readiness = Readiness::new(&lit_bits);
        let mut ready_at = |f: usize, bound: u128| -> bool { readiness.ready(f, bound, |own| b.ready(f, &set_of(own))) };
        // A subset's bound variables and the filters it has placed (bit `k`: `filters[k]`).
        // A recheck's head (`pre`), bound before every subset: its variables that the body reads.
        let pre_bits = pre.iter().filter_map(|v| var_ids[v.index()]).fold(0u128, |m, k| if k < 128 { m | 1u128 << k } else { m });
        let mut closure = |mask: u64| -> (u128, u128) {
            let mut bound = pre_bits;
            if mask == 0 && pre_bits == 0 {
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
                    if !placed[k] && ready_at(f, bound) && !call_waits(f, &|v: VarSlot| var_ids[v.index()].is_some_and(|k| bound >> k & 1 == 1), &|j: usize| joins.iter().position(|&x| x == j).is_none_or(|i| mask & (1 << i) == 0)) {
                        placed[k] = true;
                        bound |= lit_bits[f];
                        progressed = true;
                    }
                }
            }
            let placed = placed.iter().enumerate().take(128).filter(|(_, p)| **p).fold(0u128, |m, (k, _)| m | 1 << k);
            (bound, placed)
        };
        // A join's pricing depends on which of its variables are bound (and, for a range read, those of the body's
        // `docof` atoms), not on the rows coming in: its factors are worked out once per such pattern, and each
        // candidate applies them to its rows exactly as `step` does.
        #[derive(Clone, Copy)]
        enum Price {
            No,
            /// Rows a binding, a range's where it reads one, and the rule `atom_rows` priced it by.
            Atom(f64, Option<f64>, &'static str),
            Climb(f64),
            Descend(f64),
        }
        let mut docof_vars = VarSet::new();
        for d in 0..b.len() {
            if let Lit::Atom(a) = b.view(d)
                && !a.neg()
                && self.docof.contains_key(b.rel(a))
            {
                docof_vars.extend(b.vars(d));
            }
        }
        let relevant: Vec<u128> = joins
            .iter()
            .map(|&j| {
                let mut v = b.vars(j).clone();
                if matches!(b.view(j), Lit::Atom(_)) {
                    v.extend(&docof_vars);
                }
                bits_of(&v)
            })
            .collect();
        let mut prices: rustc_hash::FxHashMap<(usize, u128), Price> = Default::default();
        let mut price = |i: usize, bits: u128| -> Price {
            let pattern = bits & relevant[i];
            let compute = || {
                let bound = &set_of(bits);
                let j = joins[i];
                match b.view(j) {
                Lit::Atom(_) => {
                    let rows = atom_rows(j, bound);
                    let by = basis.get();
                    Price::Atom(rows, doc_range(j, bound).map(|(_, r)| r), by)
                }
                Lit::Nearest { n, a, kindset } => {
                    if !b.ready(j, bound) {
                        Price::No
                    } else if bound.contains(n) {
                        Price::Climb(stats.steps(kinds.get(&n).map(String::as_str), b.v.text(kindset)))
                    } else {
                        Price::Descend(kinds.get(&a).and_then(|k| stats.size.get(k)).copied().unwrap_or(stats.nodes / 1000.0))
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
                Price::Atom(ar, range, _) => {
                    let out = rows * ar;
                    if let Some(r) = range
                        && rows * r < out
                    {
                        let out = rows * r;
                        return Some((out, out, Some(Mode::Range)));
                    }
                    Some((out, out, None))
                }
                Price::Climb(steps) => Some((rows * steps, rows, Some(Mode::Climb))),
                Price::Descend(size) => Some((rows * size, rows * size, Some(Mode::Descend))),
            }
        };
        // By subset mask (at most `GREEDY` joins, so 4,096 subsets).
        let mut cells: Vec<Option<Cell>> = (0..1usize << joins.len()).map(|_| None).collect();
        let mut closed: Vec<(u128, u128)> = vec![(0, 0); 1 << joins.len()];
        closed[0] = closure(0);
        // The filters a recheck's head makes ready before any join (Q208), each passing its share on.
        let (rows0, work0) = {
            let (mut rows, mut work, mut placed) = (1.0, 0.0, closed[0].1);
            while placed != 0 {
                let k = placed.trailing_zeros() as usize;
                placed &= placed - 1;
                rows *= pass[k];
                work += rows;
            }
            (rows, work)
        };
        cells[0] = Some(Cell { work: work0, rows: rows0, prev: 0, last: usize::MAX, mode: None, unfounded: false, fan: 1.0, basis: "" });
        // Only the subsets one join more than a subset reached (with the delta first, half of them or fewer): a
        // subset no order reaches has no cell, and its closure is not worked out. Each is solved as before, its last
        // joins tried in the same order, so the plan is the same.
        let mut reached: Vec<u64> = vec![0];
        // The candidates of the full subset, each the best order ending in its last join: the search's runners-up
        // (decision 0203 §1), kept for a later choice.
        let mut finals: Vec<(f64, u64, usize, Option<Mode>, f64, &'static str)> = Vec::new();
        // A subset is solved from smaller ones only, so the order its size's subsets are solved in is free: each is
        // taken once (`listed`), not sorted (a sort of each size's subsets was 14% of a cold run's searches).
        let mut listed = vec![false; 1usize << joins.len()];
        for _size in 1..=joins.len() {
            let mut next: Vec<u64> = Vec::new();
            for &m in &reached {
                for j in 0..joins.len() {
                    let n = m | 1 << j;
                    if m & (1 << j) == 0 && !listed[n as usize] {
                        listed[n as usize] = true;
                        next.push(n);
                    }
                }
            }
            reached.clear();
            for mask in next {
                let subset: Vec<usize> = (0..joins.len()).filter(|&i| mask & (1 << i) != 0).collect();
                let mut best: Option<Cell> = None;
                let mine = closure(mask);
                for i in py_set_order(&subset) {
                    let prev = mask & !(1 << i);
                    let Some(from) = cells[prev as usize].as_ref() else { continue };
                    if prev == 0 && !matches!(b.view(joins[i]), Lit::Atom(_)) {
                        continue; // an anchor is an atom
                    }
                    if prev == 0 && !starts(joins[i]) {
                        continue; // The delta or a connected, fixed changed-input demand.
                    }
                    let (bound, placed_prev) = &closed[prev as usize];
                    if gate_waits(i, &|v: VarSlot| var_ids[v.index()].is_some_and(|k| bound >> k & 1 == 1)) || shape_waits(i, prev, &|v: VarSlot| var_ids[v.index()].is_some_and(|k| bound >> k & 1 == 1)) || unfounded_cross(i, prev, *bound) {
                        continue;
                    }
                    // Smoking gun 10's cold case (t34): `frag_str` planned before the guest round made `doc_role`'s
                    // rows took `doc_role(d, 17505)` (empty, priced at one row) first, then `kind($s, 1255)` crossed
                    // with it (7.1 M rows where 2,455 came), where `kind` first and `doc_role` probed by `@doc` was
                    // as cheap as priced. A cross product multiplies rows no row of the relations it follows vouched
                    // for: priced from a search's rows at least, it is taken only where it is cheap even so.
                    let crossed = prev != 0 && jmask[i] & *bound == 0;
                    let left = if crossed && from.unfounded { from.rows.max(SEARCH_WORK) } else { from.rows };
                    let p = price(i, *bound);
                    let by = match p { Price::Atom(_, _, by) => by, _ => "" };
                    let Some((w, mut out, mode)) = priced(p, left) else { continue };
                    let fan = if left > 0.0 { out / left } else { 0.0 };
                    let unfounded = (prev == 0 || from.unfounded) && no_rows_when_priced(i);
                    candidates += 1;
                    // The filters this join makes ready, as `close` places them: each passes its share on, and its work
                    // is the rows it passes on (decision 0297).
                    let mut filtered = 0.0;
                    let mut new = mine.1 & !placed_prev;
                    while new != 0 {
                        let k = new.trailing_zeros() as usize;
                        new &= new - 1;
                        out *= pass[k];
                        filtered += out;
                    }
                    let cand = Cell { work: from.work + w + filtered, rows: out, prev, last: i, mode, unfounded, fan, basis: by };
                    if mask == full_mask {
                        finals.push((cand.work, prev, i, mode, fan, by));
                    }
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
            let (mut order, mut bound, mut rest) = (Vec::new(), pre.clone(), filters.clone());
            if !pre.is_empty() {
                close(&mut order, 1.0, &mut bound, &mut rest);
            }
            for (i, mode) in path.into_iter().rev() {
                order.push(Step { at: joins[i], mode });
                bound.extend(b.vars(joins[i]));
                close(&mut order, 1.0, &mut bound, &mut rest);
            }
            Some((cell.work, cell.rows, order, bound, rest))
        };
        // `RERAIL_PLAN_SHOW=<head>` (dev): the search's cells of one and two joins, and the full order's, as priced.
        if std::env::var("RERAIL_PLAN_SHOW").is_ok_and(|r| r.split(',').any(|name| name == b.rel(b.head()))) {
            eprintln!("plan show {} first {first:?} fixed {} no_size {no_size:b} no_rows {no_rows:b}", b.show_head(), fixed.is_some());
            for &j in &joins {
                let Some(a) = b.atom(j) else { continue };
                let rel = b.rel(a);
                let cols: Vec<String> = (0..b.args(j).len()).map(|c| format!("ndv {:?} fan {:?}", self.ndv.get(&(rel.to_owned(), c)), self.stats.fan.get(&(rel.to_owned(), c)))).collect();
                let old = format!("{rel}__old");
                eprintln!(
                    "plan show   atom {}: card {:?} lazy {} solving {} prior_later {} old card {:?} old lazy {} col {:?} old col {:?} | {}",
                    b.show_atom(a),
                    self.stats.card.get(rel),
                    self.stats.lazy.0.contains_key(rel),
                    self.stats.solving.contains(rel),
                    self.stats.prior_later.contains(rel),
                    self.stats.card.get(&old),
                    self.stats.lazy.0.contains_key(old.as_str()),
                    self.stats.col.get(&(rel.to_owned(), sql_col(rel, 0))),
                    self.stats.col.get(&(old.clone(), sql_col(&old, 0))),
                    cols.join("; ")
                );
            }
            for (m, c) in cells.iter().enumerate().filter_map(|(m, c)| c.as_ref().map(|c| (m as u64, c))) {
                if m.count_ones() <= 5 || m == full_mask {
                    let lits: Vec<String> = (0..joins.len()).filter(|&i| m >> i & 1 == 1).map(|i| b.atom(joins[i]).map_or_else(|| format!("#{}", joins[i]), |a| b.show_atom(a))).collect();
                    eprintln!("plan show   {m:b} last {} work {:.1} rows {:.1} unfounded {}: {}", c.last, c.work, c.rows, c.unfounded, lits.join(" , "));
                }
            }
        }
        let explain = std::env::var("RDL_EXPLAIN").is_ok_and(|r| r.split(',').any(|name| name == b.rel(b.head())));
        for mask in (0..cells.len() as u64).filter(|&m| cells[m as usize].is_some()) {
            if mask == full_mask || (explain && mask.count_ones() == 2) {
                if let Some(b) = rebuild(mask) {
                    best_of.insert(mask, b);
                }
            }
        }
        subsets_reached = cells.iter().filter(|c| c.is_some()).count() as u64;
        // Decision 0354: an order's steps as this search priced them, for the learned model's features: each join's
        // rows a row in and basis from its cell (`last`: a runner-up's final join, which is not its subset's cell),
        // a filter passing its share; access and source as the plan record names them, their first words.
        let source_of = |rel: &str| -> &'static str {
            if rel.starts_with(super::strings::RELATION) {
                "text test"
            } else if self.is_input(pf, rel) {
                "input"
            } else if self.view_src.contains_key(rel) {
                "view of an input"
            } else {
                "derived"
            }
        };
        let planned = |order: &[Step], last: Option<(usize, f64, &'static str)>| -> Option<Vec<super::profile::StepPlan>> {
            let (mut mask, mut r, mut bound) = (0u64, 1.0f64, VarSet::new());
            let n = order.iter().filter(|s| joins.contains(&s.at)).count();
            let (mut seen, mut out) = (0, Vec::with_capacity(order.len()));
            for s in order {
                let (access, source, by): (&str, &str, &str) = match joins.iter().position(|&j| j == s.at) {
                    Some(i) => {
                        seen += 1;
                        mask |= 1 << i;
                        let (fan, by) = match last {
                            Some((li, fan, by)) if seen == n && li == i => (fan, by),
                            _ => {
                                let c = cells[mask as usize].as_ref()?;
                                if c.last != i {
                                    return None;
                                }
                                (c.fan, c.basis)
                            }
                        };
                        r *= fan;
                        match b.view(s.at) {
                            Lit::Atom(a) => {
                                let args = b.args(s.at);
                                let c = (0..args.len()).filter(|&c| b.constant(args[c]).is_some() || matches!(args[c], TermRef::Var(v) if bound.contains(v))).count();
                                let access = match s.mode {
                                    Some(Mode::Range) => "range",
                                    _ if c == 0 => "scan",
                                    _ if c == args.len() => "membership",
                                    _ => "lookup",
                                };
                                (access, source_of(b.rel(a)), by)
                            }
                            Lit::Nearest { .. } => (s.mode.map_or("nearest", Mode::name), "tree", ""),
                            _ => ("filter", "", ""),
                        }
                    }
                    None => match b.view(s.at) {
                        Lit::Atom(a) => {
                            r *= b.filter_pass(s.at);
                            ("anti-join", source_of(b.rel(a)), "")
                        }
                        Lit::Cmp { op, .. } => {
                            r *= b.filter_pass(s.at);
                            (if op == CmpOp::Eq { "assign" } else { "filter" }, "", "")
                        }
                        Lit::Agg { .. } => ("aggregate", "", ""),
                        _ => {
                            r *= b.filter_pass(s.at);
                            ("filter", "", "")
                        }
                    },
                };
                bound.extend(b.vars(s.at));
                out.push(super::profile::StepPlan { literal: String::new(), access: access.to_owned(), source: source.to_owned(), rows: r, all: r, passed: Vec::new(), basis: by.to_owned() });
            }
            Some(out)
        };
        // Made for a batch pricer's features.
        let pricing = joins::pricing();
        if fixed.is_none() && joins::on() && finals.len() > 1 {
            finals.sort_by(|a, b| a.0.total_cmp(&b.0));
            let best_last = cells[full_mask as usize].as_ref().map(|c| c.last);
            let alternatives: Vec<(Vec<Step>, f64)> = finals
                .iter()
                .filter(|f| Some(f.2) != best_last)
                .take(joins::ALTERNATIVES)
                .filter_map(|&(work, prev, last, mode, fan, by)| {
                    let (_, _, mut order, mut bound, mut rest) = rebuild(prev)?;
                    order.push(Step { at: joins[last], mode });
                    bound.extend(b.vars(joins[last]));
                    close(&mut order, 1.0, &mut bound, &mut rest);
                    if pricing && rest.is_empty() && let Some(st) = planned(&order, Some((last, fan, by))) {
                        cx.steps_of.push((order.clone(), st));
                    }
                    rest.is_empty().then_some((order, work))
                })
                .collect();
            cx.runners_up = alternatives;
        }
        if pricing && let Some(best) = best_of.get(&full_mask) && let Some(st) = planned(&best.2, None) {
            cx.steps_of.push((best.2.clone(), st));
        }
        }
        if let Some((g, gc)) = &greedy_probe
            && joins.len() <= GREEDY
            && let Some(best) = best_of.get(&full_mask)
        {
            let same = g.as_ref().is_some_and(|g| g.2.iter().map(|s| (s.at, s.mode)).eq(best.2.iter().map(|s| (s.at, s.mode))));
            let regret = g.as_ref().map_or(f64::INFINITY, |g| g.0 - best.0);
            let ratio = g.as_ref().map_or(f64::INFINITY, |g| g.0 / best.0.max(1e-9));
            let bucket = if same { "same order".to_string() } else if ratio <= 1.01 { "other order, same work".into() } else if regret <= SEARCH_FIXED_ROWS + candidates as f64 { "greedy worse by less than the search costs".into() } else if ratio <= 2.0 { "greedy worse, under 2x".into() } else { "greedy worse, over 2x".into() };
            super::profile::decision("greedy against the search (RERAIL_PLAN_GREEDY_PROBE)", || format!("{} joins: {bucket}", joins.len().min(9)));
            // Greedy first: its order taken where its own work is under what the search costs, so a better order
            // could save less than the search would cost.
            let cost = SEARCH_FIXED_ROWS + candidates as f64;
            let taken = g.as_ref().is_some_and(|g| g.0 <= cost);
            static SUMS: parking_lot::Mutex<[f64; 8]> = parking_lot::Mutex::new([0.0; 8]);
            let mut s = SUMS.lock();
            let at = if taken { 0 } else { 4 };
            s[at] += 1.0;
            s[at + 1] += candidates as f64;
            s[at + 2] += *gc as f64;
            s[at + 3] += if same { 0.0 } else { regret.min(1e12).max(0.0) };
            let n = (s[0] + s[4]) as u64;
            if n.is_power_of_two() || n % 1000 == 0 {
                eprintln!("greedy probe: {n} searches; greedy taken {} (candidates {}, greedy {}, regret {} rows); searched {} (candidates {}, greedy {}, regret {} rows)", s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]);
            }
        }
        let full: u64 = (0..joins.len()).map(|i| 1u64 << i).sum();
        if std::env::var("RDL_EXPLAIN").is_ok_and(|r| r.split(',').any(|name| name == b.rel(b.head()))) {
            // The chosen order's estimates step by step, and every two-step prefix's, for this head's rules.
            // Report the provenance of estimates, without recounting inputs. This
            // distinguishes a missing histogram from a bad cost calculation.
            for i in (0..b.len()).filter(|&i| b.atom(i).is_some()) {
                let rel = b.rel(b.atom(i).expect("an atom"));
                let columns:Vec<_>=b.args(i).iter().enumerate().map(|(c,t)| {
                    let key=(rel.to_string(),c);
                    let histogram=stats.dfreq.get(&key);
                    let wanted=b.constant(*t);
                    format!("c{c}: ndv={:?} fan={:?} exact_values={:?} wanted={wanted:?} exact_count={:?} prior={:?}",
                        self.ndv.get(&key),stats.fan.get(&key),histogram.map(HashMap::len),
                        wanted.as_ref().and_then(|v|histogram.map(|f|f.get(v).copied().unwrap_or(0))),
                        wanted.as_ref().and_then(|v|stats.const_freq(rel,&self.col(rel,c),v)))
                }).collect();
                eprintln!("    statistics {} card={} [{}]",rel,self.card(rel),columns.join("; "));
            }
            if let Some((work, rows, order, _, _)) = best_of.get(&full) {
                eprintln!("EXPLAIN {head} (delta {first:?}): work {work:.0} rows {rows:.1}");
                for &i in &delta_misses { eprintln!("    disjoint delta key domains: {}", b.show(i)); }
                let mut bound = VarSet::new();
                let mut r = 1.0;
                for s in order {
                    if let Lit::Atom(a) = b.view(s.at) && !a.neg() {
                        let per = atom_rows(s.at, &bound);
                        r *= per;
                        // What every positive atom would cost here instead.
                        let others: Vec<String> = (0..b.len())
                            .filter_map(|l| if let Lit::Atom(o) = b.view(l) && !o.neg() && !order.iter().take_while(|t| !std::ptr::eq(*t, s)).any(|t| b.show(t.at) == b.show(l)) { Some(format!("{}={:.2}", b.rel(o), atom_rows(l, &bound))) } else { None })
                            .collect();
                        eprintln!("    {:<40} per binding {per:.2}, rows {r:.1}   [{}]", b.show(s.at), others.join(" "));
                    } else {
                        eprintln!("    {}", b.show(s.at));
                    }
                    bound.extend(b.vars(s.at));
                }
            }
            for (mask, best) in &best_of {
                if mask.count_ones() == 2 {
                    eprintln!("  prefix {:?}: work {:.0} rows {:.1}", b.shown(&best.2), best.0, best.1);
                }
            }
        }
        let subsets = if subsets_reached > 0 { subsets_reached } else { best_of.len() as u64 };
        let Some((work, rows, order, _, rest)) = best_of.remove(&full) else {
            return Err(Error::Plan(format!("cannot order the body of {head}")));
        };
        if !rest.is_empty() {
            let rest: Vec<String> = rest.iter().map(|&i| b.show(i)).collect();
            return Err(Error::Plan(format!("unplaced literals {rest:?} in {head}")));
        }
        (work, rows, order, subsets)
        };
        // The orders priced, charged to the planning span they were priced in (the roofline's candidate work).
        super::profile::work(super::profile::Unit::Candidate, candidates);
        // Also when the run may use the device: placement prices each join step from these (decision 0095); and when
        // the body reads a kind-set axis, which takes its expected lookups from them (decision 0143).
        let axis = || (0..b.len()).any(|l| matches!(b.view(l), Lit::Atom(a) if !a.neg() && super::front::is_transduced(b.rel(a))));
        let explained = (explain && (super::profile::on() || placement::device_on() || EXPLAIN_ALWAYS.with(std::cell::Cell::get) || axis())).then(|| {
            let _explain = super::profile::span(|| "explain record".into());
            // The chosen order step by step, priced as the search priced it.
            let mut bound = VarSet::new();
            let mut r = 1.0;
            let mut steps = Vec::new();
            let mut placed: BTreeSet<usize> = BTreeSet::new();
            // Each step's rows alone (`plan_rule_step_rows`): as below, with nothing made for EXPLAIN to show.
            if STEP_ROWS.with(std::cell::Cell::get) {
                for s in &order {
                    match b.view(s.at) {
                        Lit::Atom(a) if !a.neg() => {
                            if let Some((_, out, _)) = step(s.at, r, &bound) {
                                r = out;
                            }
                        }
                        Lit::Nearest { .. } => {
                            if let Some((_, out, _)) = step(s.at, r, &bound) {
                                r = out;
                            }
                        }
                        Lit::Agg { .. } => {}
                        // A filter passes its share, as the search priced it (decision 0297).
                        _ => r *= b.filter_pass(s.at),
                    }
                    bound.extend(b.vars(s.at));
                    steps.push(super::profile::StepPlan { literal: String::new(), access: String::new(), source: String::new(), rows: r, all: r, passed: Vec::new(), basis: String::new() });
                }
                return super::profile::RulePlan { work, rows, subsets, candidates, steps, shaped: shaped.clone() };
            }
            // The rows each step makes when every row is walked, past the bound head too (no witness cut, decision
            // 0229): what the device's breadth-first stages make (decision 0277).
            let mut r_all = 1.0;
            for s in &order {
                let before = r;
                // The joins the search could have taken here instead, each priced as it was: rows a binding. A probe's
                // delta comes first by construction, not by choice: nothing was passed over for it.
                let passed: Vec<(String, f64, String)> = if b.is_join(s.at) && first != Some(s.at) {
                    joins
                        .iter()
                        .filter(|&&j| j != s.at && !placed.contains(&j))
                        .filter_map(|&j| match b.view(j) {
                            Lit::Atom(a) if !a.neg() => Some((b.show_atom(a), atom_rows(j, &bound), basis_of(j, &bound))),
                            _ => None,
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                placed.insert(s.at);
                let cols = |i: usize, bound: &VarSet| -> Vec<String> {
                    let (rel, args) = (b.rel(b.atom(i).expect("an atom")), b.args(i));
                    (0..args.len())
                        .filter(|&c| b.constant(args[c]).is_some() || matches!(args[c], TermRef::Var(v) if bound.contains(v)))
                        .map(|c| self.col(rel, c))
                        .collect()
                };
                let source = |rel: &str| -> String {
                    if rel.starts_with(super::strings::RELATION) {
                        "text test".into()
                    } else if self.is_input(pf, rel) {
                        "input".into()
                    } else if self.view_src.contains_key(rel) {
                        "view of an input".into()
                    } else {
                        "derived".into()
                    }
                };
                let basis = match b.view(s.at) {
                    Lit::Atom(a) if !a.neg() => basis_of(s.at, &bound),
                    _ => String::new(),
                };
                let (access, src) = match b.view(s.at) {
                    Lit::Atom(a) if !a.neg() => {
                        if let Some((_, out, _)) = step(s.at, r, &bound) {
                            r = out;
                        }
                        let c = cols(s.at, &bound);
                        let access = match s.mode {
                            Some(Mode::Range) => format!("range in document on ({})", c.join(", ")),
                            _ if c.is_empty() => "scan".to_owned(),
                            _ if c.len() == b.args(s.at).len() => format!("membership on ({})", c.join(", ")),
                            _ => format!("lookup on ({})", c.join(", ")),
                        };
                        (access, source(b.rel(a)))
                    }
                    Lit::Atom(a) => {
                        r *= b.filter_pass(s.at);
                        (format!("anti-join on ({})", cols(s.at, &bound).join(", ")), source(b.rel(a)))
                    }
                    Lit::Nearest { .. } => {
                        if let Some((_, out, _)) = step(s.at, r, &bound) {
                            r = out;
                        }
                        (s.mode.map_or("nearest", Mode::name).to_owned() + " the tree", "tree".to_owned())
                    }
                    // A filter passes its share, as the search priced it (decision 0297): a bound of an id's range
                    // nearly every row, an equality that binds every one. The record halved them (Q44).
                    Lit::Cmp { op, .. } => {
                        r *= b.filter_pass(s.at);
                        (if op == CmpOp::Eq { "assign or test" } else { "filter" }.to_owned(), String::new())
                    }
                    Lit::Agg { .. } => ("aggregate".to_owned(), String::new()),
                    _ => {
                        r *= b.filter_pass(s.at);
                        ("filter".to_owned(), String::new())
                    }
                };
                let walked = match b.view(s.at) {
                    Lit::Atom(a) if !a.neg() => step_all(s.at, r_all, &bound).map(|x| x.1),
                    Lit::Nearest { .. } => step_all(s.at, r_all, &bound).map(|x| x.1),
                    _ => None,
                };
                r_all = walked.unwrap_or(if before > 0.0 { r_all * r / before } else { r_all });
                bound.extend(b.vars(s.at));
                steps.push(super::profile::StepPlan { literal: b.show(s.at), access, source: src, rows: r, all: r_all.max(r), passed, basis });
            }
            super::profile::RulePlan { work, rows, subsets, candidates, steps, shaped: shaped.clone() }
        });
        Ok(((order, work, rows), explained))
    }

    /// A view over one input atom takes its statistics from the input's: a constant filters by its exact postings
    /// fraction, and each column keeps the input column's NDV (at most the view's rows) and fanout (scaled by the
    /// filter, assuming independence).
    /// `b` the view's rule, of one atom.
    pub fn view_stats(&mut self, rel: &str, b: &Body<'_>) {
        let a = b.plain_atom(0).expect("a view's atom");
        let arel = b.rel(a);
        let args = b.args(0);
        let base = self.stats.card[arel];
        let mut rows = base;
        for (c, t) in args.iter().enumerate() {
            if let Some(k) = b.constant(*t) {
                rows *= self.stats.const_freq(arel, &self.col(arel, c), &k).unwrap_or(0) as f64 / base.max(1.0);
            }
        }
        let card = rows.round_ties_even().max(1.0);
        self.stats.card.insert(rel.to_owned(), card);
        let src = first_columns(args);
        // Exact constant counts survive a projection that retains a measured key and filters no rows.
        // Marginal counts cannot be copied through a filter, or through a projection that deduplicates rows.
        let variables: Vec<VarSlot> = args.iter().filter_map(|t| t.var()).collect();
        let no_filter = args.iter().all(|t| matches!(t, TermRef::Var(_) | TermRef::Wild)) && VarSet::of(variables.iter().copied()).len() == variables.len();
        let preserves_key = b.head().args().any(|t| match t {
            TermRef::Var(v) => src.get(&v).is_some_and(|bc| self.ndv.at(arel, *bc) == Some(&base)),
            _ => false,
        });
        for (c, t) in b.head().args().enumerate() {
            let TermRef::Var(v) = t else { continue };
            let Some(&bc) = src.get(&v) else { continue };
            let n = self.ndv[&(arel.to_owned(), bc)].min(card).max(1.0);
            self.ndv.insert((rel.to_owned(), c), n);
            let fan = (self.stats.fan[&(arel.to_owned(), bc)] * rows / base.max(1.0)).max(1.0);
            self.stats.fan.insert((rel.to_owned(), c), fan);
            if no_filter && preserves_key {
                let from = self.col(arel, bc);
                let to = self.col(rel, c);
                let values: Vec<_> = self.stats.memo.get(arel).into_iter().flatten()
                    .filter(|((name, _), _)| name == &from).map(|((_, v), n)| ((to.clone(), v.clone()), *n)).collect();
                self.stats.memo.entry(rel.to_owned()).or_default().extend(values);
            }
        }
    }
}

/// Each variable's first column in an atom's arguments.
fn first_columns(args: &[TermRef<'_>]) -> HashMap<VarSlot, usize> {
    let mut src = HashMap::new();
    for (c, t) in args.iter().enumerate().rev() {
        if let TermRef::Var(v) = t {
            src.insert(*v, c);
        }
    }
    src
}

/// A digest of what a program's analysis reads: the version's digest (its declarations, inputs, facts and `.about`
/// fields, and its rules as a sum), and the rules in order.
fn program_digest(v: &Version) -> u128 {
    super::derived::digest(|d| {
        d.u64(v.digest());
        d.u64(v.rules().count() as u64);
        v.rules().for_each(|r| d.u64(r.hash()));
    })
}

/// `ProgramAnalysis::analyze`'s result for `prog`, kept for the programs analysed last: a session's runs plan the
/// same programs again (its file-local part every run, a maintained part while the same strata change), and the
/// analysis is a function of what the digest reads (each relation's columns and lattice keys, the inputs, the
/// relations with facts, the natives, and the rules in order).
fn program_analysis(prog: &Version) -> std::sync::Arc<super::analysis::ProgramAnalysis> {
    type Kept = Vec<(u128, std::sync::Arc<super::analysis::ProgramAnalysis>)>;
    /// The programs analysed last, newest at the end: a few, as each holds a result per rule.
    const KEEP: usize = 4;
    static KEPT: std::sync::Mutex<Kept> = std::sync::Mutex::new(Vec::new());
    let key = program_digest(prog);
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

/// An input's cardinality, distinct counts and fanouts, from its measured columns: `width` its declared columns
/// (`None`, not declared).
fn input_stats(stats: &mut Stats, ndv: &mut table::ColMap<f64>, width: Option<usize>, rel: &str) -> Result<()> {
    let width = width.ok_or_else(|| Error::Statistics(format!("the input {rel} is not declared")))?;
    for col in 0..width {
        let name = sql_col(rel, col);
        let Some(&[rows, n, sq, _]) = stats.col.get(&(rel.to_owned(), name.clone())) else {
            return Err(Error::Statistics(format!("no statistics for {rel}.{name}")));
        };
        stats.card.insert(rel.to_owned(), rows as f64);
        ndv.insert((rel.to_owned(), col), if n == 0 { 1.0 } else { n as f64 });
        stats.fan.insert((rel.to_owned(), col), if rows != 0 { sq as f64 / rows as f64 } else { 1.0 });
    }
    Ok(())
}

/// The program's join shape directives (`.shape`), by text, as the model reads them.
pub fn shapes_of(f: &fields::Fields<'_>) -> std::sync::Arc<Vec<super::front::Shape>> {
    std::sync::Arc::new(f.shapes().iter().map(|s| super::front::Shape {
        head: f.text(s.head).to_owned(),
        lit: f.text(s.lit).to_owned(),
        wait: match &s.wait {
            super::program_image::ShapeWaitRef::After(a) => super::front::ShapeWait::After(f.text(*a).to_owned()),
            super::program_image::ShapeWaitRef::Bound(vs) => super::front::ShapeWait::Bound(vs.iter().map(|&a| f.text(a).to_owned()).collect()),
        },
        text: f.text(s.text).to_owned(),
    }).collect::<Vec<_>>())
}

/// Reads the statistics and the input relations' cardinalities, distinct counts, fanouts and indexes, for the
/// program whose whole version `p` is (its rules by handle, its fields).
pub fn make_plan(p: &body::Placed, db: &Connection, mut inputs: Inputs) -> Result<Model> {
    let analysis = std::sync::OnceLock::new();
    let f = p.fields()?;
    // Each relation's layout, declared or inferred (decision 0361 step 3), for EXPLAIN: a report, so a program the
    // analysis refuses (its strata) plans as it would unprofiled.
    if super::profile::on()
        && let Ok(layouts) = super::program_image::manager::analysis::<super::program_image::layouts::Layouts>(&p.v)
    {
        for line in layouts.show(&p.v).lines() {
            super::profile::decision("relation layouts (decision 0361)", || line.to_owned());
        }
    }
    let rules = || (0..p.len()).map(|k| p.body(k));
    // A relation of the program's facts is its table too (`run`: `c0`, `c1`, …), though an input of that name has
    // named columns (the converter's `option_item`, core.rerail's `option_item(name, i, v)`).
    let derived: HashSet<String> = rules()
        .map(|b| b.rel(b.head()).to_owned())
        .chain(f.fact_rels().into_iter().map(|r| f.text(r.name())).filter(|rel| !f.is_input(rel)).map(str::to_owned))
        .collect();
    let mut stats = Stats::new(db, p, &mut inputs)?;
    stats.shapes = shapes::Shapes::of_version(&p.v);
    stats.prepared = prepared::Prepared::of_version(&p.v);
    let mut ndv = ColMap::default();
    for (rel, (_, rows)) in &inputs.batch {
        let rows = rows.max(1.0);
        stats.card.insert(rel.clone(), rows);
        // A base relation bound as a batch source is its vtab, which answers T's indexes' probes; any other
        // batch source is scanned.
        Arc::make_mut(&mut stats.indexes).insert(rel.clone(), vtab_indexes(rel));
        let names = super::sql::select::input_columns(rel);
        for col in 0..f.arity(rel).unwrap_or(0) {
            // A base relation's distinct values by column: from its layout where a fact answers the column
            // (decision 0361 step 4: `child`'s `c` a key, its `p` a few fan-out, its `i` dense within `p`).
            // Otherwise, by the column's name (the columns no fact answers: a chunk's documents, kinds, fields
            // and depths are far fewer than its rows); other columns are taken as keys.
            let distinct = stats.shapes.ndv(rel, col, rows, stats.counts()).unwrap_or_else(|| match names.and_then(|names| names.get(col)).copied() {
                Some("d") => rows.min(300.0),
                Some("kind") => rows.min(200.0),
                Some("idx") => rows.min(20.0),
                Some("f") => rows.min(50.0),
                Some("depth") => rows.min(30.0),
                Some("p" | "par") => (rows / 3.0).max(1.0),
                Some("s") => (rows / 5.0).max(1.0),
                _ => rows,
            });
            ndv.insert((rel.clone(), col), distinct);
            stats.fan.insert((rel.clone(), col), (rows / distinct).max(1.0));
        }
    }
    // An input no rule reads needs no statistics: a `.rerail` program declares every `core.rerail` relation (its prelude).
    let read: BTreeSet<String> = rules().flat_map(|b| (0..b.len()).filter_map(|i| b.atom(i)).map(|a| b.rel(a).to_owned()).collect::<Vec<_>>()).collect();
    for rel in f.inputs().filter(|rel| !inputs.batch.contains_key(*rel) && read.contains(*rel)) {
        // A relation whose statistics are made when first planned with (`Model::resolve`).
        if stats.lazy.0.contains_key(rel) {
            continue;
        }
        input_stats(&mut stats, &mut ndv, f.arity(rel), rel)?;
    }
    // A relation of the program's facts alone is known before it is made (decision 0299): its rows, each column's
    // distinct values, fanout and counts. Unsized, it was priced at the default 1,000 rows until its stratum ran:
    // `region`'s four `comp_kind` facts, each joined with every node of a kind, were priced at 10 M rows for 0.17 M.
    let heads: HashSet<String> = rules().map(|b| b.rel(b.head()).to_owned()).collect();
    for fr in f.fact_rels() {
        let rel = f.text(fr.name());
        if f.is_input(rel) || heads.contains(rel) || stats.card.contains_key(rel) || fr.rows().len() == 0 {
            continue;
        }
        let rows: Vec<Vec<Constant>> = fr
            .rows()
            .map(|r| r.map(|c| match c {
                super::program_image::Cell::Int(i) => Constant::Int(i),
                super::program_image::Cell::Str(a) => Constant::Str(f.text(a).to_owned()),
                // A kind, field or name (decision 0270): its atom, as `constant` reads a branded term.
                super::program_image::Cell::Branded(a) => Constant::Int(a.value()),
            }).collect())
            .collect();
        let n = rows.len() as f64;
        stats.card.insert(rel.to_owned(), n);
        for col in 0..rows[0].len() {
            let mut counts: HashMap<Constant, i64> = HashMap::new();
            for r in &rows {
                if let Some(v) = r.get(col) {
                    *counts.entry(v.clone()).or_default() += 1;
                }
            }
            let sq: i64 = counts.values().map(|c| c * c).sum();
            ndv.insert((rel.to_owned(), col), counts.len() as f64);
            stats.fan.insert((rel.to_owned(), col), sq as f64 / n);
            stats.dfreq.insert((rel.to_owned(), col), counts);
        }
    }
    let shapes = shapes_of(&f);
    // Input indexes as column lists, each ending in the rowid column if the table has one (SQLite appends it). The
    // vtabs answer the same probes as T's tables' indexes (`facts::vtab::syntax`), so they are listed as T's.
    if inputs.atoms.is_some() {
        for rel in f.inputs().filter(|rel| !inputs.batch.contains_key(*rel)) {
            Arc::make_mut(&mut stats.indexes).insert(rel.to_owned(), vtab_indexes(rel));
        }
        return Ok(Model { shapes, analysis, stats, ndv, docof: Default::default(), view_src: Default::default(), inputs: inputs.into(), docs_by_shift: by_shift(db), derived: derived.into() });
    }
    for rel in f.inputs() {
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
        Arc::make_mut(&mut stats.indexes).insert(rel.to_owned(), indexes);
    }
    Ok(Model { shapes, analysis, stats, ndv, docof: Default::default(), view_src: Default::default(), inputs: inputs.into(), docs_by_shift: by_shift(db), derived: derived.into() })
}

/// T's facts file's indexes, which the base-relation vtabs' lookups mirror.
fn vtab_indexes(rel: &str) -> Vec<Vec<String>> {
    let lists: &[&[&str]] = match rel {
        "node" => &[&["n"], &["kind", "n"]],
        "child" => &[&["c"], &["p", "idx"]],
        "field" => &[&["p", "f"], &["c"]],
        "text" | "name" | "str" | "strprefix" | "node_text" | "comment" | "structural_before" | "structural_after" => &[&["n"], &["s", "n"]],
        // W's values (`facts::vtab::syntax::Extra`).
        "value" | "static_value" | "rr_value_lit" => &[&["n"], &["v", "n"]],
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
        "intval" | "op" | "op_t" | "tape" | "line" | "end_line" | "col" | "docof" | "visit_index" | "ast_depth" | "spread" | "reference_open" | "rr_long_text_node" => &[&["n"]],
        "node_order" => &[&["n"], &["k"]],
        "first_token" | "last_token" => &[&["n"]],
        "token_before" | "token_after" => &[&["x"]],
        "end_point" => &[&["n"], &["p"]],
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
        "token" | "rr_token_anon" => &[&["p", "i"], &["start"]],
        "rr_token_named" => &[&["p", "i"], &["c"], &["start"]],
        "path_part" | "file_name" => &[&["d"], &["s", "d"]],
        // The transducer's `nearest` rows (`facts::vtab::syntax::with_nearest`), looked up either way.
        rel if crate::code_rule_dsl::logic::front::is_transduced(rel) => &[&["n"], &["a"]],
        // A text test's symbols (decision 0037, `preparation::text::TextRelations`).
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
    fn relation(&self, pf: Fields<'_>, rel: &str, delta: bool) -> Relation {
        if let Some((source, _)) = self.inputs.batch.get(rel) {
            Relation::Batch { rel: rel.to_owned(), source: source.clone() }
        } else if pf.is_input(rel) {
            Relation::Input { schema: self.inputs.facts.clone(), rel: rel.to_owned() }
        } else {
            Relation::Derived { rel: rel.to_owned(), delta }
        }
    }

    /// Whether a statement of `prog` may read `relation`: its inputs, batch inputs and derived relations, and
    /// the tree relations the `nearest` walks read, and views the run planned from.
    fn declared(&self, pf: Fields<'_>, relation: &Relation) -> bool {
        match relation {
            Relation::Input { rel, .. } => pf.is_input(rel) || matches!(rel.as_str(), "tape" | "node" | "child" | "node_order"),
            Relation::Batch { rel, .. } => self.inputs.batch.contains_key(rel),
            Relation::Derived { rel, .. } | Relation::Bound { rel, .. } => {
                // Or a view the run made and planned from (`rdl_kinds`: its statistics are the model's).
                pf.declared(rel) || pf.derives(rel) || self.stats.card.contains_key(rel)
            }
            _ => true,
        }
    }
}

/// A rule's chosen plan, as T's `plan.json` lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct Chosen {
    /// The rule's index in the program, and its body positions in plan order with their modes.
    pub rule: usize,
    pub steps: Vec<(usize, Option<Mode>)>,
    pub head: String,
    pub anchor: String,
    pub walks: Vec<(String, &'static str)>,
    pub work: i64,
}

/// What a run derived: the chosen plans, each output's rows (sorted, tab-separated), and the rounds per recursive
/// component.
#[derive(Debug, Default)]
pub struct Outcome {
    pub chosen: Vec<Chosen>,
    /// Each output's rows, as the run holds them (decision 0239; `text` gives them as lines).
    pub outputs: BTreeMap<String, std::sync::Arc<output::Rows>>,
    pub iterations: BTreeMap<String, usize>,
    /// The components a native mode ran (T's `iterations` value `"native"`).
    pub native: BTreeSet<String>,
    /// Each output's node columns (atoms mode: node ids are the vtabs', `d << 32 | index`).
    pub nodes: BTreeMap<String, Vec<usize>>,
    /// Outputs held as resident facts (`native::derived`), including decoded composed outputs kept for sessions.
    pub kept: BTreeSet<String>,
    /// The kept relations handed to the caller during the run (`run_atoms_native_handing`), no longer held.
    pub handed: BTreeSet<String>,
    /// The relations recomputed in their readers (decision 0099): no table and no engine rows hold them.
    pub inlined: BTreeSet<String>,
    /// The reached strata derived again, by the class of reason (`maintain::reason_class`): the maintain mode did not
    /// take them.
    pub again: BTreeMap<&'static str, usize>,
    /// A maintained run's relations whose rows changed by a delta (decision 0117 step 4): the rows gone and added.
    pub deltas: HashMap<String, Delta>,
    /// A maintained run's relations whose rows are as stored and were never read (decision 0117 step 4): no reached
    /// stratum read them. They are not held; their frames stand.
    pub unread: BTreeSet<String>,
    /// A maintained run's relations whose rows are as they were (decision 0117 step 4): held as kept (no read of
    /// them moved), or maintained to an empty delta. Their kept frames are not written again.
    pub unchanged: BTreeSet<String>,
    /// `RERAIL_SPIKE_DELTA` (the O125 spike): each maintained relation's rows as its form left them: [old,
    /// over-deleted (`__del`, or a level's `__gone`), inserted (`__ins`, or rederived `__redo`), truly gone
    /// (`__minus`), truly new (`__plus`), after]. Read by the session's report.
    pub spike_forms: Vec<(String, [usize; 6])>,
    /// `RERAIL_SPIKE_DELTA`: each maintained stratum reached, whether its own relations changed, and its changed
    /// reads with their deltas' rows (gone + new; `usize::MAX` where not held). Read by the session's report.
    pub spike_reached: Vec<(String, bool, Vec<(String, usize)>)>,
    /// The strata passed on as quiet (decision 0101 §1), and the relations they held with no column measured
    /// (`QuietTally`).
    pub quiet: (usize, usize),
}

impl Outcome {
    /// Each output's rows, each value as shown (`Rows::shown`): sorted, each once, a node as its number. For
    /// comparing outcomes, never read back.
    pub fn shown(&self) -> BTreeMap<String, Vec<Vec<String>>> {
        self.outputs.iter().map(|(rel, rows)| (rel.clone(), rows.shown())).collect()
    }
}

/// The set engine's component calls. Over atoms they are made in process (`native::component`), with no statement
/// each; over text symbols (T's extension loaded on the connection, which keeps its own symbols) through its SQL
/// functions.
struct Fx<'a> {
    db: crate::code_rule_dsl::facts::vtab::Db<'a>,
    direct: bool,
}
impl Fx<'_> {
    /// The planner's spec goes to the engine as values in process. Over text symbols the SQL functions are T's
    /// extension's own engine (`native/extension`, a copy with its own components), which reads the spec's text.
    fn open(&self, spec: &ComponentSpec) -> Result<()> {
        let _span = super::profile::span(|| "engine open (load inputs, build indexes)".into());
        if self.direct {
            super::native::component::open_spec(self.db, spec)?;
        } else {
            self.db.query_row("SELECT set_fixpoint_open(?1)", [spec.to_text()], |_| Ok(()))?;
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
    /// Profiled: per probe, its steps' CPU seconds since the last call, and last its head's rows made. T's extension
    /// engine does not time its steps: none.
    fn step_cpu(&self, name: &str) -> Result<Vec<Vec<f64>>> {
        if !self.direct {
            return Ok(Vec::new());
        }
        Ok(super::native::component::step_cpu(self.db, name)?)
    }
    /// Each relation's live statistics, in spec order.
    fn stats(&self, name: &str, symbols_text: &dyn Fn(usize, usize) -> bool) -> Result<Vec<LiveRel>> {
        let _span = super::profile::span(|| "engine live statistics".into());
        if self.direct {
            return Ok(super::native::component::live(self.db, name)?
                .into_iter()
                .map(|l| LiveRel { n: l.n, delta: l.delta, ndv: l.ndv, sq: l.sq, freq: l.freq.into_iter().map(|(c, v)| (c, v.into_iter().map(|(v, m)| (Constant::Int(v), m)).collect())).collect() })
                .collect());
        }
        let text: String = self.db.query_row("SELECT set_fixpoint_stats(?1)", [name], |r| r.get(0))?;
        Ok(text.split('\n').enumerate().map(|(i, line)| LiveRel::parse(line, &|c| symbols_text(i, c))).collect())
    }
    /// `stats`, of only the relations whose rows moved since `seen` (`live_changed`): `None` for the others. Through
    /// SQL, every relation's.
    fn stats_changed(&self, name: &str, seen: &mut Vec<[usize; 3]>, symbols_text: &dyn Fn(usize, usize) -> bool) -> Result<Vec<Option<LiveRel>>> {
        if !self.direct {
            return Ok(self.stats(name, symbols_text)?.into_iter().map(Some).collect());
        }
        let _span = super::profile::span(|| "engine live statistics".into());
        // `RERAIL_LIVE_ALL=on`: every relation's, each time (to compare).
        static ALL: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| matches!(std::env::var("RERAIL_LIVE_ALL").as_deref(), Ok("on")));
        if *ALL {
            seen.clear();
        }
        Ok(super::native::component::live_changed(self.db, name, seen)?
            .into_iter()
            .map(|l| l.map(|l| LiveRel { n: l.n, delta: l.delta, ndv: l.ndv, sq: l.sq, freq: l.freq.into_iter().map(|(c, v)| (c, v.into_iter().map(|(v, m)| (Constant::Int(v), m)).collect())).collect() }))
            .collect())
    }
    fn reprobe(&self, name: &str, i: usize, new: &Reprobe) -> Result<()> {
        let _span = super::profile::span(|| "engine reprobe".into());
        if self.direct {
            super::native::component::reprobe_spec(self.db, name, i, new)?;
        } else {
            let (kind, body) = new.text();
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
            super::native::component::materialize(&self.db, table)?;
        } else {
            self.db.query_row("SELECT set_fixpoint_materialize(?1)", [table], |_| Ok(()))?;
        }
        Ok(())
    }
}

/// A run's evaluation state on its connection.
/// A native stratum's outcome.
enum Native {
    /// A rule has a literal with no native step: nothing ran.
    Declined,
    Ran,
    /// Its run waits on its device wave (`lanes::wave`): the driver's state, to resume once the wave ran.
    Waiting(Box<Driving>),
    /// Its component posted requests (`Run::asking`, decision 0114 §3): the driver's state and the requests'
    /// tickets, to resume once every one is answered. Nothing waits for them meanwhile.
    Asking(Box<Driving>, Vec<u64>),
}

/// A native stratum between its component's open and its close: what its driver (`Run::drive`) needs to run the
/// component's chunks of waves, replan between them and record what it did. Held while the stratum waits on its
/// device wave.
struct Driving {
    scc: BTreeSet<String>,
    solving: bool,
    name: String,
    spec: ComponentSpec,
    rels: Vec<String>,
    ri: BTreeMap<String, usize>,
    views: HashMap<String, (String, String)>,
    sql_views: bool,
    sql_probe: Vec<bool>,
    /// Each seed's order, with its rule (a seed seeded by SQL has none).
    orders: Vec<(body::RuleKey, Vec<Step>)>,
    probes: Vec<(body::RuleKey, usize, Vec<Step>)>,
    order_rules: Vec<u64>,
    nseeds: usize,
    seeds: Vec<body::RuleKey>,
    ngates: usize,
    explained: Vec<(String, Option<usize>, Option<super::profile::RulePlan>, f64, u64)>,
    counted: Vec<Vec<(u64, u64)>>,
    /// Per probe line, the orders it ran in before a replan changed it, each with its own step counts: `counted`
    /// holds the current order's alone, so a step's counts are never another order's step at the same place.
    earlier: Vec<Vec<(Vec<Step>, Vec<(u64, u64)>)>>,
    timed: Vec<f64>,
    /// Profiled: per probe line, its current order's CPU seconds per step and last its head's rows made, over the
    /// walks `counted` counts (both start again when a replan changes the order).
    stepped: Vec<Vec<f64>>,
    chunk: i64,
    /// Relation/column versions and their dependent probe plans.
    replanned_at: replanning::Watch,
    /// Per probe, the searches made for it in this run: its work pays for each before the next (decision 0122).
    searched: Vec<u32>,
    /// Per line (seeds, then probes), the key its plan is kept under, for its work to be recorded (decision 0122).
    keys: Vec<Option<PlanKey>>,
    started: std::time::Instant,
    placed: placement::Placement,
    shape: placement::Shape,
    /// This thread's CPU the stratum has spent so far (while profiling).
    cpu_own: f64,
    lab: Option<super::native::component::lab::Capture>,
    /// Placed on the device but a shape its run does not take: why, reported once the stratum closes.
    not_taken: Option<&'static str>,
    /// Per probe: not planned yet, until its delta has rows (decision 0123 §9).
    unplanned: Vec<bool>,
    /// Per line (seeds, then probes): a replacement running over the rows a slow order left at its cut (decision
    /// 0206).
    handoff: Vec<Option<joins::Handoff>>,
    /// Per line, the seconds its probes ran (the engine's per-probe wall time), for the measured critical path
    /// (decision 0208).
    walled: Vec<f64>,
    /// Per probe, its whole plan's order (O168).
    wholes: HashMap<usize, Vec<Step>>,
}

pub struct Run<'a, 'b> {
    /// The run's connection; only its scope in a stratum run off the run's thread (`strand`, decision 0113).
    db: crate::code_rule_dsl::facts::vtab::Db<'a>,
    model: Model,
    views: Arc<HashSet<String>>,
    made: HashSet<(String, Vec<usize>)>,
    /// Each derived table's secondary indexes, as column orders (`index_for`).
    orders: HashMap<String, Vec<Vec<usize>>>,
    /// Derived tables with no key: views materialized as plain tables (`ensure_indexes`).
    keyless: HashSet<String>,
    /// Relations held only by the native set engine (`program`) until `materialize`.
    native_rels: BTreeSet<String>,
    /// The inputs this run's strata placed on the device read (decision 0325 step 6): the views the device made of
    /// them stay resident on this thread for the run's later strata, which placement then prices as found, not made.
    /// A strand's run is on another thread, with residency of its own: it starts with none.
    device_views: HashSet<String>,
    /// Immutable unary demand snapshots, shared only within this run.
    demands: native_demand::Demands,
    /// Why the native engine declined the last stratum it was offered (EXPLAIN's `why`).
    why_not: Option<String>,
    /// What kind of literal the native engine declined (`decline_class`), for the profile's SQLite fallbacks.
    why_class: Option<String>,
    /// While the maintain mode writes its joins (decision 0364): a body literal read from this source, all its
    /// columns (a changed input's version, `Version(r, …)`), not as its input's query.
    mode_reads: HashMap<usize, JoinSource>,
    /// Derived relations whose table is made only when SQL first needs it (`ensure`): its `CREATE` statement. A
    /// relation the native engine derives and keeps never has one.
    pending: HashMap<String, String>,
    /// Relations whose one rule projects one atom (`kind($n, $k) :- node(_, $n, $k, _, _)`, variables distinct), each
    /// with the atom's integer constants as (column, value) filters (`isasync($n) :- op($n, 126)`): a native join reads
    /// such a view from its input in place, filtered, and its rows are counted there (`rows_of`), so no SQL view is
    /// made. Found once per program, not per atom of each spec.
    projections: Arc<HashMap<String, Vec<(usize, i64)>>>,
    /// The program's inputs, as a set: asked per atom of each spec (`prog.inputs` is a list); and those of the
    /// maintained forms placed since (`RunInputs`).
    inputs: Arc<RunInputs<'a>>,
    /// The run's programs' rules as versions (`body::Versions`), which the planner reads them by: the program's,
    /// and each maintained form's as it is placed.
    versions: Arc<body::Versions>,
    /// The version among `versions` whose fields the run reads (`fields`): 0, the program's; a maintained form's
    /// while the run plans or runs its pieces (`set_version`).
    version: usize,
    /// Answers a request (decision 0065 §2a): the request relation and its new rows, to the produced relations'
    /// rows. A component with a request pauses for it (`setfix`'s `request`).
    answer: Option<Answer<'b>>,
    /// The run's text tests (`Hooks::texts`), or its own, made from the program's tests when one first runs.
    texts: Option<Arc<super::strings::Vocabulary>>,
    /// Posts requests and gives their answers as they arrive (decision 0114 §3): where the strata loop retires
    /// strata from a scoreboard, a component with a request suspends on it instead of waiting in `answer`.
    asking: Option<&'b mut dyn Asking>,
    /// Each produced relation's rows answered so far, for a stratum above its request's (its table's rows).
    answered: BTreeMap<String, Vec<Vec<i64>>>,
    /// The stratum being evaluated whose answers are logged (`logging`), and each such stratum's answers by produced
    /// relation (`answer_logs`): a native step's answers are among what its stratum read, kept with its frames.
    logging: Option<usize>,
    answer_logs: HashMap<usize, BTreeMap<String, BTreeSet<Vec<i64>>>>,
    /// The run's quiet strata, shared with its strands (`QuietTally`).
    quiet: Arc<QuietTally>,
}

/// The runner's answer to requests (`Run::answer`): a request relation and its new rows (atoms and integers), to the
/// rows each produced relation gains.
pub type Answer<'a> = &'a mut dyn FnMut(&str, &[Vec<i64>]) -> Result<Vec<(String, Vec<Vec<i64>>)>>;

/// A complete request relation has no pending recursive work before its consumer. Resume held keys at once,
/// grouped by request family, as a quiet recursive component does. Never publish the protocol rows as facts.
/// (Ported from Data-layer-rule-expressivity d659df1c4: held-back keys of a complete request were lost before its
/// non-recursive consumer ran.)
fn answer_complete(answer: Answer<'_>, request: &str, keys: &[Vec<i64>]) -> Result<BTreeMap<String, Vec<Vec<i64>>>> {
    let mut pending = BTreeMap::from([(request.to_owned(), keys.to_vec())]);
    let mut produced: BTreeMap<String, Vec<Vec<i64>>> = BTreeMap::new();
    while !pending.is_empty() {
        let mut held: BTreeMap<String, Vec<Vec<i64>>> = BTreeMap::new();
        for (request, keys) in pending {
            for (rel, rows) in answer(&request, &keys)? {
                if let Some(request) = rel.strip_prefix(super::answers::HELD_BACK) {
                    if !rows.is_empty() { held.entry(request.to_owned()).or_default().extend(rows); }
                } else { produced.entry(rel).or_default().extend(rows); }
            }
        }
        pending = held;
    }
    Ok(produced)
}

#[cfg(test)]
mod complete_answer_tests {
    use super::*;

    #[test]
    fn complete_answers_resume_deferred_batches_before_consumers() {
        // Source witness: plan_scope_side_effects.scan_repo must load an external file link.
        // Its load requests are already complete, so loaded is in a later stratum, not their SCC.
        // Exercise that execution path with a whole demand batch, both all-held and mixed answers.
        for immediate in [0, 1] {
            use super::super::program_image::fields::FieldsMut as _;
            let read = super::super::rr::parse_version(".decl seed(x:int)\n.decl req(x:int)\n.decl got(x:int)\n.decl shown(x:int)\nreq(x) :- seed(x).\nshown(x) :- got(x).\n.output shown\n").unwrap();
            let mut b = super::super::program_image::Builder::on(&read);
            b.fields().set_facts("seed", &(0..1000).map(|n| super::super::front::List::from(vec![Value::Int(n)])).collect::<Vec<_>>());
            b.fields().set_field(super::super::program_facts::Field::Answers(&BTreeMap::from([("req".to_owned(), vec!["got".to_owned()])])));
            let (program, _) = b.finish().unwrap();
            let lowered = lower::Lowered::of_placed(body::Placed::whole(program.clone()), lower::domains_of(&program).unwrap(), [0; 3], crate::code_rule_dsl::language::GrammarDialect::Python);
            let path = crate::code_rule_dsl::fixtures::temp();
            let _store = crate::content_store::ContentStore::open(&path).unwrap();
            let db = Connection::open(&path).unwrap();
            let mut batches = Vec::new();
            let mut answer = |rel: &str, keys: &[Vec<i64>]| -> Result<Vec<(String, Vec<Vec<i64>>)>> {
                assert_eq!(rel, "req");
                batches.push(keys.to_vec());
                if batches.len() == 1 {
                    Ok(vec![("got".into(), keys[..immediate].to_vec()),
                        (format!("{}req", super::super::answers::HELD_BACK), keys[immediate..].to_vec())])
                } else { Ok(vec![("got".into(), keys.to_vec())]) }
            };
            let result = run_atoms_native_answering(&lowered, &db, "main", Stats::empty_of_lowered(&lowered).unwrap(), BTreeSet::new(), "program", &mut answer, None, None, None).unwrap();
            assert_eq!(result.outputs["shown"].len(), 1000, "later consumers must see all deferred positives");
            assert_eq!(result.outputs["shown"].shown().iter().map(|r| r[0].parse::<i64>().unwrap()).collect::<BTreeSet<_>>(), (0..1000).collect(), "every returned value stays exact");
            assert_eq!(batches.len(), 2, "held keys resume as one batch, never one callback per key");
            assert_eq!(batches[0].len(), 1000);
            assert_eq!(batches[1].len(), 1000-immediate);
            assert_eq!(batches[1], batches[0][immediate..], "the complete deferred payload resumes unchanged");
        }
    }
}

/// The runner's answers to requests, without a wait (decision 0114 §3): a request is posted and its answer is an
/// event, taken when it has come.
pub trait Asking {
    /// Posts a request relation's new rows: the ticket its answer comes back with.
    fn post(&mut self, rel: &str, rows: &[Vec<i64>]) -> Result<u64>;
    /// The answers that have come, each with its ticket; when none has, waits up to `wait` for one. The rows
    /// each produced relation gains, as `Answer` gives them.
    fn arrived(&mut self, wait: std::time::Duration) -> Result<Vec<(u64, Vec<(String, Vec<Vec<i64>>)>)>>;
}

/// Answers a paused component's requests and adds the answers to its produced relations (decision 0065 §2a).
/// Rows for a produced relation outside it are kept for the stratum that holds that relation.
/// Time spent in answer hooks, and their calls (`RERAIL_STRATA_COUNTS` prints them): a measurement.
pub static ANSWER_WAIT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static ANSWER_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn answer_pending(db: &Connection, direct: bool, answer: &mut Option<Answer<'_>>, answered: &mut BTreeMap<String, Vec<Vec<i64>>>, log: Option<&mut BTreeMap<String, BTreeSet<Vec<i64>>>>, name: &str, scc: &BTreeSet<String>) -> Result<()> {
    if !direct {
        return Err(Error::Plan(format!("{name}: requests are answered only over atoms (the in-process engine)")));
    }
    let asked = super::native::component::pending(db, name)?;
    let answer = answer.as_deref_mut().ok_or_else(|| Error::Plan(format!("{name}: a request, and no one to answer it")))?;
    let mut given = Vec::new();
    for (rel, rows) in &asked {
        let _span = super::profile::span(|| format!("answer {rel} ({} rows)", rows.len()));
        let t0 = std::time::Instant::now();
        given.extend(answer(rel, rows)?);
        ANSWER_WAIT.fetch_add(t0.elapsed().as_nanos() as u64, std::sync::atomic::Ordering::Relaxed);
        ANSWER_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    add_answers(db, answered, log, name, scc, given)
}

/// Adds a component's answers to its produced relations; rows for a produced relation outside it are kept for the
/// stratum that holds that relation.
fn add_answers(db: &Connection, answered: &mut BTreeMap<String, Vec<Vec<i64>>>, log: Option<&mut BTreeMap<String, BTreeSet<Vec<i64>>>>, name: &str, scc: &BTreeSet<String>, given: Vec<(String, Vec<Vec<i64>>)>) -> Result<()> {
    let mut by: BTreeMap<String, Vec<Vec<i64>>> = BTreeMap::new();
    for (p, got) in given {
        by.entry(p).or_default().extend(got);
    }
    if let Some(log) = log {
        for (p, rows) in by.iter().filter(|(p, _)| !p.starts_with(super::answers::HELD_BACK)) {
            log.entry(p.clone()).or_default().extend(rows.iter().cloned());
        }
    }
    for (p, rows) in by {
        if let Some(req) = p.strip_prefix(super::answers::HELD_BACK) {
            super::native::component::defer(db, name, req, rows)?;
        } else if scc.contains(&p) {
            super::native::component::add(db, name, &p, &rows)?;
        } else {
            answered.entry(p).or_default().extend(rows);
        }
    }
    Ok(())
}

/// Rule `rule` (of version `v`) as a view of an input's columns (Q37), the input bound or not: its body one positive
/// atom of an input (`input`), no constants and each variable once, its head distinct variables of it, and its rows
/// the input's, one each: the head keeps the input's key (`node`'s node, `doc`'s and `tape`'s first column) or every
/// column. The input, and the atom's column of each head column. A run holds such a relation in place
/// (`Run::input_view`); a session neither keeps nor counts it run (`retain::cut`).
pub(crate) fn input_view_shape(v: &super::program_image::Version, rule: super::program_image::RuleRef<'_>, input: impl Fn(&str) -> bool) -> Option<(String, Vec<usize>)> {
    if std::env::var_os("RERAIL_COPY_VIEWS").is_some() {
        return None;
    }
    let mut body = rule.body();
    let (Some(only), None) = (body.next(), body.next()) else { return None };
    let Lit::Atom(a) = only.view() else { return None };
    let base = v.rel_name(a.rel());
    if a.neg() || !input(base) {
        return None;
    }
    let args: Vec<TermRef<'_>> = a.args().collect();
    let mut vars = HashSet::new();
    if !args.iter().all(|t| match t {
        TermRef::Var(x) => vars.insert(*x),
        TermRef::Wild => true,
        _ => false,
    }) {
        return None;
    }
    let at: Vec<usize> = rule.head().args().map(|t| t.var().and_then(|x| args.iter().position(|y| y.var() == Some(x)))).collect::<Option<_>>()?;
    if at.iter().collect::<HashSet<_>>().len() != at.len() {
        return None;
    }
    let key = match base {
        "node" if args.len() == 5 => Some(1),
        "doc" | "tape" => Some(0),
        _ => None,
    };
    (key.is_some_and(|k| at.contains(&k)) || at.len() == args.len()).then(|| (base.to_owned(), at))
}

/// The strata the process held as views of a bound input's columns (`Run::input_view`, Q37), not derived.
static INPUT_VIEWS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How many strata the process held as views of an input's columns, for tests and profiles.
pub fn input_views() -> u64 {
    INPUT_VIEWS.load(std::sync::atomic::Ordering::Relaxed)
}

/// A run's inputs: the program's, borrowed from its version, and those its maintained forms added (their old rows
/// and the deltas they read), owned, since a form's version is the run's and moves with it.
#[derive(Clone, Default)]
struct RunInputs<'a> {
    program: rustc_hash::FxHashSet<&'a str>,
    added: rustc_hash::FxHashSet<Box<str>>,
}

impl RunInputs<'_> {
    fn contains(&self, rel: &str) -> bool {
        self.program.contains(rel) || self.added.contains(rel)
    }
}

/// A recursive atom's delta variant: the rule's head, its plan with that atom first, and the atom's plan position.

impl<'a> Run<'a, '_> {
    /// The current program's fields (`fields::Fields`), read from its version.
    fn fields(&self) -> Fields<'_> {
        self.versions.whole(self.version)
    }

    /// A composite index on each derived relation for every bound-column pattern the plan probes it with.
    fn ensure_indexes(&mut self, order: Order<'_, '_>) -> Result<()> {
        let b = order.b;
        let mut bound = VarSet::new();
        for s in order.steps {
            let a = b.atom(s.at);
            let args = b.args(s.at);
            let probed_col = |t: &TermRef<'_>, bound: &VarSet| b.constant(*t).is_some() || matches!(t, TermRef::Var(v) if bound.contains(*v));
            if let Some(a) = a
                && self.views.contains(b.rel(a))
                && let Some((base, cols)) = self.model.view_src.get(b.rel(a))
            {
                // A view over an input is probed through the input's indexes. If none serves this probe, SQLite
                // builds an automatic index per statement, which for a filtered view scans every node, every round.
                // Materialize it once instead; it then gets indexes like any derived relation.
                let probed: HashSet<&String> = args.iter().enumerate().filter(|(_, t)| probed_col(t, &bound)).filter_map(|(c, _)| cols[c].as_ref()).collect();
                let served = self.model.stats.indexes.get(base).into_iter().flatten().any(|ix| ix.first().is_some_and(|c| probed.contains(c)));
                if !probed.is_empty() && !served {
                    let r = b.rel(a);
                    let q = crate::code_rule_dsl::logic::sql::ident(r);
                    self.db.execute_batch(&format!(
                        "CREATE TEMP TABLE {r}__m AS SELECT * FROM {q}; DROP VIEW {q}; ALTER TABLE {r}__m RENAME TO {q};"
                    ))?;
                    Arc::make_mut(&mut self.views).remove(r);
                    Arc::make_mut(&mut self.model.view_src).remove(r);
                    self.keyless.insert(r.to_string());
                }
            }
            if let Some(a) = a
                && !self.inputs.contains(b.rel(a))
                && !self.views.contains(b.rel(a))
            {
                let cols: Vec<usize> = (0..args.len()).filter(|&c| probed_col(&args[c], &bound)).collect();
                if !cols.is_empty() && self.made.insert((b.rel(a).to_string(), cols.clone())) {
                    self.index_for(b.rel(a), &cols)?;
                }
            }
            bound.extend(b.vars(s.at));
        }
        Ok(())
    }

    /// An index serving an equality search on `cols` of derived table `rel`, with as few indexes as serve every
    /// search: each index is one column order, and serves a search whose columns are its first ones. The table's
    /// key (all columns in order, or a lattice's key columns; `WITHOUT ROWID`, so the table is that order) serves
    /// its prefixes. A search containing an index's columns extends that index (rebuilt once, the old one dropped)
    /// rather than adding one, so the indexes form chains of searches (Soufflé's index selection: a minimum chain
    /// cover, here greedy as searches arrive). Each secondary index of a `WITHOUT ROWID` table holds the whole row,
    /// a copy of the relation that every insert maintains: on `calls.rerail` this makes 22 indexes where one per search
    /// made 96.
    fn index_for(&mut self, rel: &str, cols: &[usize]) -> Result<()> {
        let want: BTreeSet<usize> = cols.iter().copied().collect();
        let key = if self.keyless.contains(rel) {
            0
        } else {
            let f = self.fields();
            f.lattice_decl(rel).map_or_else(|| f.arity(rel).unwrap_or(0), |l| l.keys)
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

    /// `r`'s rows, read where they are held: the native engine's relation, or the source bound to `r`'s table. SQL's
    /// `count(*)` only for a table SQL holds: any SQL makes the run wait for the store (write-back cannot lag).
    fn rows_of(&self, r: &str) -> Result<i64> {
        if let Some(h) = super::native::component::held(self.db, r) {
            return Ok(h.rows() as i64);
        }
        if let Some(source) = crate::code_rule_dsl::facts::vtab::bound_source(self.db, r) {
            return Ok(source.len() as i64);
        }
        if let Some(n) = self.view_rows(r) {
            return Ok(n);
        }
        self.count(&format!("SELECT count(*) FROM {}", crate::code_rule_dsl::logic::sql::ident(r)))
    }
    /// The rows of `r`, a view projecting one bound input (`projections`), counted from the input: its rows that pass
    /// the view's filters, each projection once (the view is a set when it drops columns).
    fn view_rows(&self, r: &str) -> Option<i64> {
        let view = self.unmade_view(r)?;
        let src = crate::code_rule_dsl::facts::vtab::bound_source(self.db, &view.base)?;
        let names = src.columns();
        let (eqs, kept) = (view.eqs, view.cols);
        let ids = src.filter_eq(&eqs);
        let whole = (0..names.len()).all(|c| kept.contains(&c) || eqs.iter().any(|e| e.0 == c));
        if whole {
            return Some(ids.len() as i64);
        }
        let mut seen: HashSet<Vec<i64>> = HashSet::with_capacity(ids.len());
        for id in ids {
            seen.insert(kept.iter().map(|&c| src.value(id as usize, c)).collect());
        }
        Some(seen.len() as i64)
    }
    /// The view `r` (`projections`) on its bound input's columns: what a reader after the run reads in place of SQL's.
    fn unmade_view(&self, r: &str) -> Option<crate::code_rule_dsl::facts::vtab::UnmadeView> {
        let eqs = self.projections.get(r)?;
        let (base, cols) = self.model.view_src.get(r)?;
        let src = crate::code_rule_dsl::facts::vtab::bound_source(self.db, base)?;
        let names = src.columns();
        let at = |c: &str| names.iter().position(|n| n == c);
        let eqs: Vec<(usize, i64)> = eqs.iter().map(|&(c, v)| Some((at(&self.model.col(base, c))?, v))).collect::<Option<_>>()?;
        let cols: Vec<usize> = cols.iter().map(|c| at(c.as_deref()?)).collect::<Option<_>>()?;
        Some(crate::code_rule_dsl::facts::vtab::UnmadeView { base: base.clone(), cols, eqs })
    }
    fn count(&self, sql: &str) -> Result<i64> {
        Ok(self.db.query_row(sql, [], |r| r.get(0))?)
    }




    /// The rows `select` added to `head`'s relation (keys changed, for a lattice).
    fn insert_rows(&self, head: &str, select: &str) -> Result<usize> {
        Ok(if self.fields().lattice(head) {
            sql_kernel::merge(self, head, select)?
        } else {
            let statement = format!("INSERT OR IGNORE INTO {} {select}", crate::code_rule_dsl::logic::sql::ident(head));
            super::profile::plan(&self.db, &statement);
            self.db.execute(&statement, [])?
        })
    }

    /// Makes `rel`'s rows natively from the tables of the relations it reads (`front::Native`, `native` the run's
    /// program's); returns them.
    fn native_relation(&mut self, rel: &str, native: &super::program_image::NativeRef) -> Result<usize> {
        use super::program_image::NativeSpec;
        let vs = Arc::clone(&self.versions);
        let f = vs.whole(self.version);
        let reads: Vec<String> = native.reads.iter().map(|&r| f.rel_name(r).to_owned()).collect();
        self.ensure(&reads)?;
        self.materialize(&reads)?;
        let (edge, target, roots) = match &native.spec {
            NativeSpec::DfsFirst { edge, target, roots } => (f.rel_name(*edge), f.rel_name(*target), roots.map(|r| f.rel_name(r))),
            NativeSpec::MatchAll { input, pattern, groups, positions } => {
                let groups: Vec<String> = groups.iter().map(|&g| f.text(g).to_owned()).collect();
                return self.match_all(rel, f.rel_name(*input), f.text(*pattern), &groups, *positions);
            }
            NativeSpec::TextTest { input, column, op, patterns } => {
                let patterns: Vec<String> = patterns.iter().map(|&p| f.text(p).to_owned()).collect();
                return self.text_test(rel, f.rel_name(*input), *column, f.text(*op), &patterns);
            }
            NativeSpec::Components { edge, own } => return self.components(rel, f.rel_name(*edge), f.rel_name(*own)),
            NativeSpec::Rank { query, counted, group, op } => return self.rank_rows(rel, f.rel_name(*query), f.rel_name(*counted), *group, f.text(*op)),
            NativeSpec::Reach { .. } | NativeSpec::ReachThrough { .. } => return self.reach_rows(rel, &native.spec),
            NativeSpec::Python { kind, input, targets } => return self.python_relation(rel, f.text(*kind), f.rel_name(*input), targets.map(|t| f.rel_name(t))),
            NativeSpec::PythonCandidates { input, targets, .. } => return self.python_relation(rel, "candidates", f.rel_name(*input), Some(f.rel_name(*targets))),
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
                let symbol = f.types(roots)[1] == "sym" && self.model.inputs.atoms.is_some();
                let key = if symbol { format!("sym_text({})", r[1]) } else { r[1].clone() };
                let mut seen = BTreeSet::new();
                let ordered = read(&format!("SELECT {}, {key} FROM {} ORDER BY 2, 1", r[0], crate::code_rule_dsl::logic::sql::ident(roots)), 1)?;
                Some(ordered.into_iter().map(|row| row[0]).filter(|r| seen.insert(*r)).collect())
            }
            None => None,
        };
        let rows = crate::code_rule_dsl::logic::native::dfs_first::dfs_first(&edges, &targets, roots.as_deref());
        let n = rows.len();
        insert_many(&self.db, &format!("INSERT OR IGNORE INTO {}", crate::code_rule_dsl::logic::sql::ident(rel)), rows.into_iter().map(|(a, b, c)| vec![SqlValue::Integer(a), SqlValue::Integer(b), SqlValue::Integer(c)]).collect())?;
        Ok(n)
    }

    /// Read a completed unary relation where its facts live. SQL is only a fallback for SQL-owned facts.
    fn native_demand(&mut self, input: &str) -> Result<std::sync::Arc<Vec<i64>>> {
        let source = if self.native_rels.contains(input) {
            Some(super::native::component::derived(self.db, input).ok_or_else(|| Error::Plan(format!("missing retained demand {input}")))?)
        } else if self.fields().is_input(input) {
            crate::code_rule_dsl::facts::vtab::bound_source(self.db, input)
        } else { None };
        if source.is_none() { self.ensure(&[input.to_owned()])?; }
        self.demands.read(&self.db, input, &self.model.col(input, 0), source)
    }

    /// One producer invocation for a completed demand relation; native rows never take a SQL round trip.
    fn python_relation(&mut self, rel: &str, kind: &str, input: &str, targets: Option<&str>) -> Result<usize> {
        use super::native::component;
        use crate::code_rule_dsl::facts::vtab;
        let ident = crate::code_rule_dsl::logic::sql::ident;
        let requests = self.native_demand(input)?;
        let mut rows = if requests.is_empty() { Vec::new() } else {
            let mut wanted = BTreeSet::new();
            if let Some(targets) = targets {
                if self.model.inputs.atoms.is_some() {
                    let keys = self.native_demand(targets)?;
                    wanted = keys.iter().map(|&key| super::native::symbol_text(&self.db, key).map_err(Error::from)).collect::<Result<_>>()?;
                } else {
                    self.ensure(&[targets.to_owned()])?;
                    let col = self.model.col(targets, 0);
                    let mut s = self.db.prepare(&format!("SELECT {col} FROM {}", ident(targets)))?;
                    wanted = s.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<_>>()?;
                }
            }
            let source = vtab::bound_source(self.db, "node").ok_or_else(|| Error::Plan("Python producer requires bound syntax facts".into()))?;
            let batch = source.python_batch().ok_or_else(|| Error::Plan("Python producer requires a syntax batch".into()))?;
            let producer = crate::code_rule_dsl::facts::vtab::syntax::semantic_producer().map_err(Error::Plan)?;
            let f = self.fields();
            if let Some(super::program_image::NativeSpec::PythonCandidates { context, member, .. }) = f.native(rel).map(|n| &n.spec) {
                producer.candidates(&batch, &requests, &wanted, f.text(*context), f.text(*member)).map_err(Error::Plan)?
            }else{producer.project(&batch, kind, &requests, &wanted).map_err(Error::Plan)?}
        };
        if self.model.inputs.atoms.is_some() {
            let strings = rows.iter().flatten().filter_map(|v| match v { SqlValue::Text(s) => Some(s.as_str()), _ => None });
            let mut ids = super::native::intern_batch(&self.db, strings)?.into_iter();
            for value in rows.iter_mut().flatten() {
                if matches!(value, SqlValue::Text(_)) { *value = SqlValue::Integer(ids.next().unwrap()); }
            }
            let values: Vec<Vec<i64>> = rows.into_iter().map(|row| row.into_iter().map(|v| match v {
                SqlValue::Integer(i) => Ok(i),
                _ => Err(Error::Plan("Python fact producer returned a non-integer after interning".into())),
            }).collect()).collect::<Result<_>>()?;
            component::retain(self.db, rel, values, vec![b'i'; self.fields().width(rel)])?;
            self.native_rels.insert(rel.to_owned());
            let source = component::derived(self.db, rel).expect("just retained");
            let stats = source.statistics().expect("retained relations own their statistics");
            live_stats(&mut self.model, rel, LiveRel {
                n: source.len() as i64,
                delta: source.len() as i64,
                ndv: stats.iter().map(|s| s.2).collect(),
                sq: stats.iter().map(|s| s.3).collect(),
                freq: Vec::new(),
            });
            super::profile::decision("native fact result", || format!("{rel}: retained rows={}", source.len()));
            return Ok(source.len());
        }
        // The legacy text-symbol backend reads SQL tables. Atom-backed production retains the typed rows above.
        self.ensure(&[rel.to_owned()])?;
        let before = self.db.total_changes();
        insert_many(&self.db, &format!("INSERT OR IGNORE INTO {}", ident(rel)), rows)?;
        Ok((self.db.total_changes() - before) as usize)
    }

    /// `components` (`front::Native::Components`): each document's component and key from `edge`'s and `own`'s
    /// rows. Digests and keys are symbols: atoms (interned by `sym_atom`) or text, as the run stores them.
    /// `components` on the native engine: `edge`'s and `own`'s rows read where the engine holds them (relations it ran,
    /// or the run's bound inputs), the result retained as its own relation. No statement (decision 0104): a wave
    /// reads no table. `None` when the run holds symbols as text, or either input only as a table.
    fn native_components(&mut self, rel: &str, edge: &str, own: &str) -> Result<Option<usize>> {
        use super::native::component;
        if self.model.inputs.atoms.is_none() {
            return Ok(None);
        }
        let held = |this: &Self, r: &str| -> Option<Arc<dyn crate::code_rule_dsl::facts::vtab::Source>> {
            if this.native_rels.contains(r) {
                component::derived(this.db, r)
            } else if this.fields().is_input(r) {
                crate::code_rule_dsl::facts::vtab::bound_source(this.db, r)
            } else {
                None
            }
        };
        let (Some(e), Some(o)) = (held(self, edge), held(self, own)) else { return Ok(None) };
        let edges: Vec<(i64, i64)> = (0..e.len()).map(|r| (e.value(r, 0), e.value(r, 1))).collect::<BTreeSet<_>>().into_iter().collect();
        let mut digests: Vec<(i64, String)> = Vec::with_capacity(o.len());
        for (d, h) in (0..o.len()).map(|r| (o.value(r, 0), o.value(r, 1))).collect::<BTreeSet<_>>() {
            digests.push((d, super::native::symbol_text(&self.db, h)?));
        }
        let rows = crate::code_rule_dsl::logic::native::components::components(&edges, &digests);
        let ids = super::native::intern_batch(&self.db, rows.iter().map(|(_, _, k)| k.as_str()))?;
        let values: Vec<Vec<i64>> = rows.iter().zip(ids).map(|((d, c, _), k)| vec![*d, *c, k]).collect();
        component::retain(self.db, rel, values, vec![b'i'; self.fields().width(rel)])?;
        self.native_rels.insert(rel.to_owned());
        let source = component::derived(self.db, rel).expect("just retained");
        let stats = source.statistics().expect("retained relations own their statistics");
        live_stats(&mut self.model, rel, LiveRel {
            n: source.len() as i64,
            delta: source.len() as i64,
            ndv: stats.iter().map(|s| s.2).collect(),
            sq: stats.iter().map(|s| s.3).collect(),
            freq: Vec::new(),
        });
        Ok(Some(source.len()))
    }

    /// `dfs_first` (`front::Native::DfsFirst`, `_memo` with roots) from the rows the engine holds, kept as a held
    /// relation: no table read or written (decision 0104). `None`: an input is not held (`native_relation` then
    /// makes the rows from tables).
    fn native_dfs_first(&mut self, rel: &str, edge: &str, target: &str, roots: Option<&str>) -> Result<Option<usize>> {
        use super::native::component;
        if self.model.inputs.atoms.is_none() {
            return Ok(None);
        }
        let held = |this: &Self, r: &str| -> Option<Arc<dyn crate::code_rule_dsl::facts::vtab::Source>> {
            if this.native_rels.contains(r) {
                component::derived(this.db, r)
            } else if this.fields().is_input(r) {
                crate::code_rule_dsl::facts::vtab::bound_source(this.db, r)
            } else {
                None
            }
        };
        let (Some(e), Some(t)) = (held(self, edge), held(self, target)) else { return Ok(None) };
        let roots = match roots {
            Some(r) => match held(self, r) {
                Some(s) => Some((r, s)),
                None => return Ok(None),
            },
            None => None,
        };
        let edges: Vec<(i64, i64, i64)> = (0..e.len()).map(|r| (e.value(r, 0), e.value(r, 1), e.value(r, 2))).collect::<BTreeSet<_>>().into_iter().collect();
        let targets: Vec<i64> = (0..t.len()).map(|r| t.value(r, 0)).collect::<BTreeSet<_>>().into_iter().collect();
        // `dfs_first_memo`'s roots in increasing order of their second column (a number, or a symbol by its text),
        // each root once, as `native_relation` orders them.
        let roots: Option<Vec<i64>> = match roots {
            Some((name, s)) => {
                let symbol = self.fields().types(name)[1] == "sym";
                let pairs: BTreeSet<(i64, i64)> = (0..s.len()).map(|r| (s.value(r, 0), s.value(r, 1))).collect();
                let mut keyed: Vec<((i64, String), i64)> = Vec::with_capacity(pairs.len());
                for (root, k) in pairs {
                    let key = if symbol { (0, super::native::symbol_text(&self.db, k)?) } else { (k, String::new()) };
                    keyed.push((key, root));
                }
                keyed.sort();
                let mut seen = BTreeSet::new();
                Some(keyed.into_iter().map(|(_, r)| r).filter(|r| seen.insert(*r)).collect())
            }
            None => None,
        };
        let rows = crate::code_rule_dsl::logic::native::dfs_first::dfs_first(&edges, &targets, roots.as_deref());
        let values: Vec<Vec<i64>> = rows.into_iter().map(|(a, b, c)| vec![a, b, c]).collect();
        component::retain(self.db, rel, values, vec![b'i'; self.fields().width(rel)])?;
        self.native_rels.insert(rel.to_owned());
        let source = component::derived(self.db, rel).expect("just retained");
        let stats = source.statistics().expect("retained relations own their statistics");
        live_stats(&mut self.model, rel, LiveRel {
            n: source.len() as i64,
            delta: source.len() as i64,
            ndv: stats.iter().map(|s| s.2).collect(),
            sq: stats.iter().map(|s| s.3).collect(),
            freq: Vec::new(),
        });
        Ok(Some(source.len()))
    }

    /// `match_all` (`front::Native::MatchAll`) over the input the engine holds, kept as a held relation: no table read
    /// or written (decision 0104). `None`: the input is not held, or the run has no atoms (`match_all` then reads
    /// the table).
    fn native_match_all(&mut self, rel: &str, input: &str, pattern: &str, groups: &[String], positions: bool) -> Result<Option<usize>> {
        use super::native::component;
        if self.model.inputs.atoms.is_none() {
            return Ok(None);
        }
        let held = if self.native_rels.contains(input) {
            component::derived(self.db, input)
        } else if self.fields().is_input(input) {
            crate::code_rule_dsl::facts::vtab::bound_source(self.db, input)
        } else {
            None
        };
        let Some(s) = held else { return Ok(None) };
        let re = regex::Regex::new(pattern).map_err(|e| Error::Plan(format!("match_all: bad regex {pattern:?}: {e}")))?;
        let values: BTreeSet<i64> = (0..s.len()).map(|r| s.value(r, 0)).collect();
        let mut texts: Vec<(i64, String)> = Vec::with_capacity(values.len());
        for v in values {
            texts.push((v, super::native::symbol_text(&self.db, v)?));
        }
        use crate::code_rule_dsl::logic::native::match_all;
        let captures: Vec<(i64, Vec<&str>, Option<[i64; 3]>)> = texts
            .iter()
            .flat_map(|(v, t)| {
                let rows: Vec<(Vec<&str>, Option<[i64; 3]>)> = if positions {
                    match_all::occurrences(&re, groups, t).into_iter().map(|(row, at)| (row, Some(at))).collect()
                } else {
                    match_all::rows(&re, groups, t).into_iter().map(|row| (row, None)).collect()
                };
                rows.into_iter().map(move |(row, at)| (*v, row, at))
            })
            .collect();
        let mut ids = super::native::intern_batch(&self.db, captures.iter().flat_map(|(_, row, _)| row.iter().copied()))?.into_iter();
        let rows: BTreeSet<Vec<i64>> = captures
            .into_iter()
            .map(|(v, row, at)| {
                std::iter::once(v)
                    .chain(row.iter().map(|_| ids.next().expect("one atom per capture")))
                    .chain(at.into_iter().flatten())
                    .collect()
            })
            .collect();
        component::retain(self.db, rel, rows.into_iter().collect(), vec![b'i'; self.fields().width(rel)])?;
        self.native_rels.insert(rel.to_owned());
        let source = component::derived(self.db, rel).expect("just retained");
        let stats = source.statistics().expect("retained relations own their statistics");
        live_stats(&mut self.model, rel, LiveRel {
            n: source.len() as i64,
            delta: source.len() as i64,
            ndv: stats.iter().map(|s| s.2).collect(),
            sq: stats.iter().map(|s| s.3).collect(),
            freq: Vec::new(),
        });
        Ok(Some(source.len()))
    }

    /// A view of `node`'s columns `cols` (`input_view`) held in place (decision 0329, `hold_view_in_place`), where `src`
    /// is a batch's: its statistics each column's exact ones, counted from the batch without reading its rows
    /// (`syntax::node_column_counts`), as a copy's sketches gave them (`live_of`). `None`: `src` is not a batch's, and
    /// the view is read into flat rows (`hold_view`).
    fn hold_view_in_place(&mut self, rel: &str, src: &Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, cols: &[usize], types: Vec<u8>) -> Option<LiveRel> {
        if src.columns().len() != 5 {
            return None;
        }
        let batch = src.python_batch()?;
        let n = src.len() as i64;
        let mut ndv = Vec::new();
        let mut sq = Vec::new();
        let mut freq: Vec<(usize, HashMap<Constant, i64>)> = Vec::new();
        for (j, &c) in cols.iter().enumerate() {
            let (d, s, counts) = crate::code_rule_dsl::facts::vtab::syntax::node_column_counts(&batch, c)?;
            ndv.push(d);
            sq.push(s);
            if let Some(counts) = counts {
                freq.push((j, counts.into_iter().map(|(v, m)| (Constant::Int(v), m)).collect()));
            }
        }
        super::native::component::hold_view_in_place(self.db, rel, Arc::clone(src), cols.to_vec(), types);
        Some(LiveRel { n, delta: n, ndv, sq, freq })
    }

    /// The reach index (decision 0119, `front::Native::Reach` and `ReachThrough`) over relations the engine holds:
    /// held as the index, read in place by its readers, no row made. `None`: an input is not held natively (the rows
    /// are then made by `reach_rows`).
    fn native_reach(&mut self, rel: &str, native: &super::program_image::NativeSpec) -> Result<Option<(usize, String)>> {
        use super::native::{component, reach::Closed};
        use crate::code_rule_dsl::facts::vtab::Source as _;
        use super::program_image::NativeSpec as Native;
        let vs = Arc::clone(&self.versions);
        let f = vs.whole(self.version);
        if self.model.inputs.atoms.is_none() {
            return Ok(None);
        }
        let held = |this: &Self, r: &str| -> Option<Arc<dyn crate::code_rule_dsl::facts::vtab::Source>> {
            if this.native_rels.contains(r) {
                component::derived(this.db, r)
            } else if this.fields().is_input(r) {
                crate::code_rule_dsl::facts::vtab::bound_source(this.db, r)
            } else {
                None
            }
        };
        let pairs = |s: &dyn crate::code_rule_dsl::facts::vtab::Source| -> Vec<(i64, i64)> {
            let mut v = [0i64; 2];
            (0..s.len()).map(|r| { s.row_at(r, &[0, 1], &mut v); (v[0], v[1]) }).collect()
        };
        let closed = match native {
            Native::Reach { seed, edge } => {
                let (Some(s), Some(e)) = (held(self, f.rel_name(*seed)), held(self, f.rel_name(*edge))) else { return Ok(None) };
                Closed::reach(&pairs(s.as_ref()), &pairs(e.as_ref()))
            }
            Native::ReachThrough { reach, map } => {
                let (Some(r), Some(m)) = (component::held_source(self.db, f.rel_name(*reach)), held(self, f.rel_name(*map))) else { return Ok(None) };
                let Some(r) = r.as_any().and_then(|a| a.downcast_ref::<Closed>()) else { return Ok(None) };
                r.through(&pairs(m.as_ref()))
            }
            _ => return Ok(None),
        };
        let n = closed.len();
        let bytes = closed.bytes();
        let over = match native { Native::ReachThrough { reach, .. } => Some(f.rel_name(*reach)), _ => None };
        let why = if super::profile::on() { self.reach_shape(rel, &closed, over) } else { String::new() };
        let stats = closed.statistics().expect("the index counts its statistics");
        component::hold_source(self.db, rel, Arc::new(closed), vec![b'i'; f.width(rel)]);
        self.native_rels.insert(rel.to_owned());
        live_stats(&mut self.model, rel, LiveRel {
            n: n as i64,
            delta: n as i64,
            ndv: stats.iter().map(|s| s.2).collect(),
            sq: stats.iter().map(|s| s.3).collect(),
            freq: Vec::new(),
        });
        super::profile::note(format!("{rel} index bytes"), || bytes.to_string());
        Ok(Some((n, why)))
    }

    /// The explain output's account of a reach index (decision 0119): the graph's cycles and its connected parts,
    /// the largest named (a document by its path, where the nodes are documents), as a decision each; and the
    /// stratum's one-line summary, returned. A relation through a reach (`over`) closes over that reach's graph,
    /// whose shape its own lines give.
    fn reach_shape(&self, rel: &str, closed: &super::native::reach::Closed, over: Option<&str>) -> String {
        let shape = closed.shape();
        let (starts, distinct, widest) = closed.spread();
        let doc = crate::code_rule_dsl::facts::vtab::bound_source(self.db, "doc");
        let name = |v: i64| -> String {
            let found = doc.as_ref().and_then(|d| {
                let cols = d.columns();
                let (dc, pc) = (cols.iter().position(|c| c == "d")?, cols.iter().position(|c| c == "path")?);
                let row = *d.lookup(dc, &[v])?.first()?;
                super::native::symbol_text(&self.db, d.value(row, pc)).ok()
            });
            found.unwrap_or_else(|| v.to_string())
        };
        let list = |parts: &[(usize, Vec<i64>)]| -> String {
            parts
                .iter()
                .map(|(size, first)| {
                    let mut names: Vec<String> = first.iter().map(|&v| name(v)).collect();
                    names.sort();
                    let more = if *size > first.len() { ", …" } else { "" };
                    format!("{size} ({}{more})", names.join(", "))
                })
                .collect::<Vec<_>>()
                .join("; ")
        };
        let topic = "reach index: cycles and connected graphs (decision 0119)";
        if let Some(reach) = over {
            super::profile::decision(topic, || format!("{rel}: through {reach}, over its graph; {starts} starts share {distinct} closures; the widest reaches {widest}"));
            return format!("reach index (decision 0119) through {reach}: {starts} starts, {distinct} closures, widest {widest}");
        }
        super::profile::decision(topic, || {
            format!(
                "{rel}: {} nodes, {} edges; {} strongly connected components, {} of them cycles of two or more nodes, {} nodes with an edge to themselves; {} connected parts",
                shape.nodes, shape.edges, shape.components, shape.cyclic, shape.self_loops, shape.connected
            )
        });
        if !shape.largest_cycles.is_empty() {
            super::profile::decision(topic, || format!("{rel}: largest cycles: {}", list(&shape.largest_cycles)));
        }
        if !shape.largest_connected.is_empty() {
            super::profile::decision(topic, || format!("{rel}: largest connected parts: {}", list(&shape.largest_connected)));
        }
        super::profile::decision(topic, || format!("{rel}: {starts} starts share {distinct} closures; the widest reaches {widest}"));
        format!(
            "reach index (decision 0119): {} nodes, {} edges, {} cycles (largest {}), {} connected parts (largest {}); {starts} starts, {distinct} closures, widest {widest}",
            shape.nodes,
            shape.edges,
            shape.cyclic,
            shape.largest_cycles.first().map_or(0, |c| c.0),
            shape.connected,
            shape.largest_connected.first().map_or(0, |c| c.0)
        )
    }

    /// The reach index's rows written to `rel`'s table, from its inputs' tables: for a run that holds no relation
    /// natively.
    fn reach_rows(&mut self, rel: &str, native: &super::program_image::NativeSpec) -> Result<usize> {
        use super::native::reach::Closed;
        use crate::code_rule_dsl::facts::vtab::Source as _;
        use super::program_image::NativeSpec as Native;
        let vs = Arc::clone(&self.versions);
        let f = vs.whole(self.version);
        let ident = crate::code_rule_dsl::logic::sql::ident;
        // The tables' values, text or numbers, coded as integers in their order for the index, and decoded after.
        let mut codes: BTreeMap<SqlKey, i64> = BTreeMap::new();
        let pairs = |this: &Self, r: &str| -> Result<Vec<(SqlKey, SqlKey)>> {
            let (c0, c1) = (this.model.col(r, 0), this.model.col(r, 1));
            let mut st = this.db.prepare(&format!("SELECT DISTINCT {c0}, {c1} FROM {}", ident(r)))?;
            let rows = st.query_map([], |r| Ok((SqlKey::of(r.get(0)?), SqlKey::of(r.get(1)?))))?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        };
        let (seed, edge, map) = match native {
            Native::Reach { seed, edge } => (f.rel_name(*seed), f.rel_name(*edge), None),
            Native::ReachThrough { reach, map } => {
                let reach = f.rel_name(*reach);
                let Some(Native::Reach { seed, edge }) = f.native(reach).map(|n| &n.spec) else { return Err(Error::Plan(format!("{rel}: {reach} is not a reach"))) };
                (f.rel_name(*seed), f.rel_name(*edge), Some(f.rel_name(*map)))
            }
            _ => unreachable!("a reach index"),
        };
        let read = [Some(pairs(self, seed)?), Some(pairs(self, edge)?), map.map(|m| pairs(self, m)).transpose()?];
        for (a, b) in read.iter().flatten().flatten() {
            codes.entry(a.clone()).or_insert(0);
            codes.entry(b.clone()).or_insert(0);
        }
        let mut values_of: Vec<SqlKey> = Vec::with_capacity(codes.len());
        for (i, (k, code)) in codes.iter_mut().enumerate() {
            *code = i as i64;
            values_of.push(k.clone());
        }
        let coded = |p: &Option<Vec<(SqlKey, SqlKey)>>| -> Vec<(i64, i64)> { p.iter().flatten().map(|(a, b)| (codes[a], codes[b])).collect() };
        let reach = Closed::reach(&coded(&read[0]), &coded(&read[1]));
        let closed = if map.is_some() { reach.through(&coded(&read[2])) } else { reach };
        let values = (0..closed.len()).map(|r| vec![values_of[closed.value(r, 0) as usize].value(), values_of[closed.value(r, 1) as usize].value()]).collect();
        let before = self.db.total_changes();
        insert_many(&self.db, &format!("INSERT OR IGNORE INTO {}", ident(rel)), values)?;
        Ok((self.db.total_changes() - before) as usize)
    }

    /// `Native::Rank` over the tables of its two relations: each asked tuple's count below its bound
    /// (`native::rank`).
    fn rank_rows(&mut self, rel: &str, query: &str, counted: &str, group: usize, op: &str) -> Result<usize> {
        let op = super::native::rank::Op::parse(op).ok_or_else(|| Error::Plan(format!("{rel}: a rank by {op}")))?;
        let read = |r: &str| -> Result<Vec<Vec<i64>>> {
            let w = self.fields().width(r);
            let cols: Vec<String> = (0..w).map(|c| self.model.col(r, c)).collect();
            let mut st = self.db.prepare(&format!("SELECT DISTINCT {} FROM {}", cols.join(", "), crate::code_rule_dsl::logic::sql::ident(r)))?;
            let rows = st.query_map([], |row| (0..w).map(|i| row.get::<_, i64>(i)).collect::<rusqlite::Result<Vec<i64>>>())?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        };
        let rows = super::native::rank::rank(&read(query)?, &read(counted)?, group, op);
        let values = rows.into_iter().map(|r| r.into_iter().map(SqlValue::Integer).collect()).collect();
        let before = self.db.total_changes();
        insert_many(&self.db, &format!("INSERT OR IGNORE INTO {}", crate::code_rule_dsl::logic::sql::ident(rel)), values)?;
        Ok((self.db.total_changes() - before) as usize)
    }

    /// `Native::Rank` on the native engine: its two relations read where the engine holds them, the result retained
    /// as its own relation, no table read or written. `None` when either is held only as a table.
    fn native_rank(&mut self, rel: &str, query: &str, counted: &str, group: usize, op: &str) -> Result<Option<usize>> {
        use super::native::component;
        let Some(op) = super::native::rank::Op::parse(op) else { return Ok(None) };
        let held = |this: &Self, r: &str| -> Option<Arc<dyn crate::code_rule_dsl::facts::vtab::Source>> {
            if this.native_rels.contains(r) {
                component::derived(this.db, r)
            } else if this.fields().is_input(r) {
                crate::code_rule_dsl::facts::vtab::bound_source(this.db, r)
            } else {
                None
            }
        };
        let (Some(q), Some(c)) = (held(self, query), held(self, counted)) else { return Ok(None) };
        let rows = |s: &dyn crate::code_rule_dsl::facts::vtab::Source| -> Vec<Vec<i64>> {
            let w = s.columns().len();
            (0..s.len()).map(|r| (0..w).map(|k| s.value(r, k)).collect()).collect()
        };
        let values = super::native::rank::rank(&rows(q.as_ref()), &rows(c.as_ref()), group, op);
        component::retain(self.db, rel, values, vec![b'i'; self.fields().width(rel)])?;
        self.native_rels.insert(rel.to_owned());
        let source = component::derived(self.db, rel).expect("just retained");
        let stats = source.statistics().expect("retained relations own their statistics");
        live_stats(&mut self.model, rel, LiveRel {
            n: source.len() as i64,
            delta: source.len() as i64,
            ndv: stats.iter().map(|s| s.2).collect(),
            sq: stats.iter().map(|s| s.3).collect(),
            freq: Vec::new(),
        });
        Ok(Some(source.len()))
    }

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
        let mut ids = if atoms { super::native::intern_batch(&self.db, rows.iter().map(|(_, _, k)| k.as_str()))?.into_iter() } else { Vec::new().into_iter() };
        let values = rows
            .into_iter()
            .map(|(d, c, k)| {
                let k = if atoms { SqlValue::Integer(ids.next().expect("one atom per key")) } else { SqlValue::Text(k) };
                vec![SqlValue::Integer(d), SqlValue::Integer(c), k]
            })
            .collect();
        let before = self.db.total_changes();
        insert_many(&self.db, &format!("INSERT OR IGNORE INTO {}", ident(rel)), values)?;
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
            super::native::intern_batch(&self.db, captures.iter().flat_map(|(_, row, _)| row.iter().copied()))?.into_iter()
        } else { Vec::new().into_iter() };
        let rows = captures.into_iter().map(|(v, row, at)| {
            std::iter::once(v.clone()).chain(row.into_iter().map(|g| {
                if atoms { SqlValue::Integer(ids.next().expect("one atom per capture")) }
                else { SqlValue::Text(g.to_owned()) }
            })).chain(at.into_iter().flatten().map(SqlValue::Integer)).collect()
        }).collect();
        let before = self.db.total_changes();
        insert_many(&self.db, &format!("INSERT OR IGNORE INTO {}", crate::code_rule_dsl::logic::sql::ident(rel)), rows)?;
        Ok((self.db.total_changes() - before) as usize)
    }

    /// The run's vocabulary and test `op` of `patterns`' place in it (`Hooks::texts`), or the run's own, made from
    /// the program's text tests the first time one runs.
    fn texts_for(&mut self, op: &str, patterns: &[String]) -> Result<(Arc<super::strings::Vocabulary>, usize)> {
        let spec = (op.to_owned(), patterns.to_vec());
        let given = self.texts.as_ref().and_then(|v| v.specs.iter().position(|s| *s == spec).map(|k| (Arc::clone(v), k)));
        let (texts, k) = match given {
            Some(found) => found,
            None => {
                // The program's tests, as `preparation::text::TextRelations` numbered them: one vocabulary for the run's every test.
                let f = self.fields();
                let mut specs: Vec<(String, Vec<String>)> = f.natives().iter().filter_map(|n| match &n.spec {
                    super::program_image::NativeSpec::TextTest { op, patterns, .. } => Some((f.text(*op).to_owned(), patterns.iter().map(|&p| f.text(p).to_owned()).collect())),
                    _ => None,
                }).collect();
                specs.sort();
                specs.dedup();
                let names: Vec<String> = (0..specs.len()).map(|k| format!("{}{k}", super::strings::DURING)).collect();
                let own = Arc::new(super::strings::Vocabulary::from_specs(&names, specs).map_err(Error::Plan)?);
                let k = own.specs.iter().position(|s| *s == spec).expect("the test is the program's");
                self.texts = Some(Arc::clone(&own));
                (own, k)
            }
        };
        Ok((texts, k))
    }

    /// A text test over strings the run answered (`front::Native::TextTest`) over a relation the engine holds: the
    /// rows of `input` whose `column` passes `op` of `patterns`, its distinct strings tested by the run's vocabulary
    /// with its other tests at once, each once a run; held as the engine's relation. `None`: the input is not held.
    /// `natively`: whether the engine made the input (`native_rels`, which a stratum is lent, `Run::lent`).
    fn native_text_test(&mut self, rel: &str, input: &str, natively: bool, column: usize, op: &str, patterns: &[String]) -> Result<Option<usize>> {
        use super::native::component;
        if self.model.inputs.atoms.is_none() {
            return Ok(None);
        }
        let held = if natively {
            component::derived(self.db, input)
        } else if self.fields().is_input(input) {
            crate::code_rule_dsl::facts::vtab::bound_source(self.db, input)
        } else {
            None
        };
        let Some(source) = held else { return Ok(None) };
        let (texts, k) = self.texts_for(op, patterns)?;
        let width = self.fields().width(input);
        let all: Vec<usize> = (0..width).collect();
        let mut row = vec![0i64; width];
        let rows: Vec<Vec<i64>> = (0..source.len()).map(|r| { source.row_at(r, &all, &mut row); row.clone() }).collect();
        // Each distinct string once, by its atom.
        let atoms: Vec<i64> = rows.iter().map(|r| r[column]).collect::<BTreeSet<_>>().into_iter().collect();
        let strings = super::native::symbol_texts(&self.db, &atoms)?;
        let symbols: Option<Vec<u32>> = atoms.iter().map(|&a| u32::try_from(a).ok()).collect();
        let passed: HashSet<i64> = texts.passing(k, &strings, symbols.as_deref()).into_iter().map(|i| atoms[i]).collect();
        let values: Vec<Vec<i64>> = rows.into_iter().filter(|r| passed.contains(&r[column])).collect();
        component::retain(self.db, rel, values, vec![b'i'; width])?;
        self.native_rels.insert(rel.to_owned());
        let made = component::derived(self.db, rel).expect("just retained");
        let stats = made.statistics().expect("retained relations own their statistics");
        live_stats(&mut self.model, rel, LiveRel {
            n: made.len() as i64,
            delta: made.len() as i64,
            ndv: stats.iter().map(|s| s.2).collect(),
            sq: stats.iter().map(|s| s.3).collect(),
            freq: Vec::new(),
        });
        Ok(Some(made.len()))
    }

    /// A text test over strings the run answered (`front::Native::TextTest`), in SQL: the rows of `input` whose
    /// `column` passes `op` of `patterns`, its distinct strings tested by the run's vocabulary with its other
    /// tests at once. Where the engine does not hold the input (`native_text_test`), or symbols are text.
    fn text_test(&mut self, rel: &str, input: &str, column: usize, op: &str, patterns: &[String]) -> Result<usize> {
        let (texts, k) = self.texts_for(op, patterns)?;
        let atoms = self.model.inputs.atoms.is_some();
        let width = self.fields().width(input);
        let cols: Vec<String> = (0..width).map(|c| self.model.col(input, c)).collect();
        let text = if atoms { format!("sym_text({})", cols[column]) } else { cols[column].clone() };
        // Its rows and each one's string, read once; each distinct string tested once.
        let rows: Vec<(Vec<SqlValue>, Option<String>)> = {
            let mut st = self.db.prepare(&format!("SELECT DISTINCT {}, {text} FROM {}", cols.join(", "), crate::code_rule_dsl::logic::sql::ident(input)))?;
            let rows = st.query_map([], |row| Ok(((0..width).map(|i| row.get::<_, SqlValue>(i)).collect::<rusqlite::Result<Vec<_>>>()?, row.get::<_, Option<String>>(width)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let mut at: HashMap<&str, usize> = HashMap::new();
        let (mut strings, mut symbols, mut of_row) = (Vec::new(), Vec::new(), Vec::with_capacity(rows.len()));
        for (row, t) in &rows {
            of_row.push(t.as_deref().map(|t| *at.entry(t).or_insert_with(|| {
                strings.push(t.to_owned());
                symbols.push(match row[column] { SqlValue::Integer(i) => u32::try_from(i).ok(), _ => None });
                strings.len() - 1
            })));
        }
        let symbols: Option<Vec<u32>> = if atoms { symbols.into_iter().collect() } else { None };
        let mut passes = vec![false; strings.len()];
        for i in texts.passing(k, &strings, symbols.as_deref()) {
            passes[i] = true;
        }
        let kept: Vec<Vec<SqlValue>> = rows.into_iter().zip(of_row).filter(|(_, i)| i.is_some_and(|i| passes[i])).map(|((row, _), _)| row).collect();
        let before = self.db.total_changes();
        insert_many(&self.db, &format!("INSERT OR IGNORE INTO {}", crate::code_rule_dsl::logic::sql::ident(rel)), kept)?;
        Ok((self.db.total_changes() - before) as usize)
    }

    fn refresh(&mut self, r: &str, at_least_one: bool) -> Result<()> {
        let n = self.rows_of(r)? as f64;
        self.model.stats.card.insert(r.to_owned(), if at_least_one { n.max(1.0) } else { n });
        Ok(())
    }
}

/// Whether a text-test stratum's rules are its native's definition (`h(x…) :- input(x…), test`), so the native makes
/// its rows: in a maintained form its relation's rules are its old rows and its new ones (`maintain`), made as rules,
/// while its `h__ins` and a stratum derived again are the definition.
fn text_test_defined(vs: &body::Versions, rules: &[body::RuleKey], input: &str) -> bool {
    matches!(rules, [k] if { let (v, r) = vs.rule_at(*k); r.body().filter_map(|l| l.atom()).map(|a| v.rel_name(a.rel())).eq([input]) })
}

/// Decision 0354: the samples of `order`'s steps after its first, from its plan record `x` and the counts it ran with
/// (`None` where the record is not of this order). A step's bound key is its relation's most distinct values among
/// the columns bound when it reads. Without `counts`, each step's features for a prediction (the model's choice):
/// every step after the first, its rows in and out the statistics'.
/// Decision 0354: a stratum's plans made ahead of its loop (`Model::plan_ahead`), by line (`line_id`).
pub(in crate::code_rule_dsl) type Ahead = HashMap<(usize, Option<usize>), (Plan, Option<super::profile::RulePlan>)>;

/// A line of a stratum's loop: its body (where it lives for the loop) and its delta.
pub(in crate::code_rule_dsl) fn line_id(b: &Body<'_>, first: Option<usize>) -> (usize, Option<usize>) {
    (std::ptr::from_ref(b) as usize, first)
}

fn step_samples(model: &Model, b: &Body<'_>, order: &[Step], x: &super::profile::RulePlan, counts: Option<&[(u64, u64)]>, seed: bool, origin: &str) -> Option<Vec<super::costs::join_steps::StepSample>> {
    use super::costs::join_steps::{StepSample, category};
    if x.steps.len() != order.len() || counts.is_some_and(|c| c.len() != order.len()) {
        return None;
    }
    let planned: Vec<(u64, u64)>;
    let (counts, predicting) = match counts {
        Some(c) => (c, false),
        None => {
            planned = (0..order.len()).map(|i| ((if i == 0 { 1.0 } else { x.steps[i - 1].rows }).max(0.0) as u64, x.steps[i].rows.max(0.0) as u64)).collect();
            (&planned[..], true)
        }
    };
    let rule = b.observed_key();
    let mut bound = VarSet::new();
    let mut out = Vec::new();
    for (i, (s, &(n_in, n_out))) in x.steps.iter().zip(counts).enumerate() {
        let at = order[i].at;
        // A record kept from a run may be of another order than the counts'; one made for a prediction is this
        // order's (and the literal's text is not made again).
        if !predicting && s.literal != b.show(at) {
            return None;
        }
        let before = if i == 0 { 1.0 } else { x.steps[i - 1].rows };
        if i > 0 && (predicting || n_in > 0) && before > 0.0 {
            let rel = b.atom(at).map(|a| b.rel(a));
            let args = if rel.is_some() { b.args(at) } else { &[] };
            let bound_cols: Vec<usize> = (0..args.len()).filter(|&c| b.constant(args[c]).is_some() || matches!(args[c], TermRef::Var(v) if bound.contains(v))).collect();
            let key_ndv = rel.map_or(0.0, |r| bound_cols.iter().filter_map(|&c| model.ndv.at(&r, c).copied()).fold(0.0, f64::max));
            // The relation's name, and its bound columns' count, from the body: a record the search makes for a
            // prediction has neither the literal's text nor the columns' names.
            let relation = rel.map_or_else(String::new, str::to_owned);
            out.push(StepSample {
                basis: category(&s.basis, 4),
                access: category(&s.access, 1),
                source: category(&s.source, 4),
                bound: bound_cols.len() as u32,
                step: i as u32,
                steps: order.len() as u32,
                est_in: before,
                est_fan: s.rows / before,
                card: rel.map_or(0.0, |r| model.card(r)),
                solving: rel.is_some_and(|r| model.stats.solving.contains(r)),
                seed,
                literal: format!("{rule:x}:{at}"),
                key_ndv,
                relation,
                bound_mask: bound_cols.iter().filter(|&&c| c < 64).fold(0, |m, &c| m | 1 << c),
                cols: args.len() as u32,
                actual_in: if predicting { before } else { n_in as f64 },
                actual_out: n_out as f64,
                origin: origin.to_owned(),
            });
        }
        if !b.plain_atom(at).is_some_and(|a| a.neg()) {
            bound.extend(b.vars(at));
        }
    }
    Some(out)
}

/// Adds a native join's step counts ("in:out" per step, `set_fixpoint_steps`) to the observed fanouts of the binding
/// patterns its positive atoms were read with (T's `observe`). The first step (the delta, or a seed's anchor) is
/// skipped.
fn observe(stats: &mut Stats, rule: u64, order: Order<'_, '_>, counts: &[(i64, i64)]) {
    keeps::observe_steps(rule, order, counts);
    // A probe that ran no step past its first (its delta empty this chunk) observed nothing.
    if counts.iter().skip(1).all(|&(n_in, _)| n_in == 0) {
        return;
    }
    let b = order.b;
    let mut bound = VarSet::new();
    for (i, (s, &(n_in, n_out))) in order.steps.iter().zip(counts).enumerate() {
        let neg = b.plain_atom(s.at).is_some_and(|a| a.neg());
        if i > 0 && b.plain_atom(s.at).is_some() && !neg && n_in != 0 {
            let o = stats.obs.entry(rule).or_default().entry(b.sig(s.at, &bound)).or_insert([0, 0]);
            o[0] += n_in;
            o[1] += n_out;
        }
        if !neg {
            bound.extend(b.vars(s.at));
        }
    }
}

/// The fanouts observed for each step of `order` (`observe`), where at least `OBS_MIN` bindings were counted:
/// the prices `atom_rows` takes first, so a probe is replanned when one appears or moves (`replanning::Watch`).
fn observed_fans(stats: &Stats, rule: u64, order: Order<'_, '_>) -> Vec<Option<f64>> {
    let seen = stats.obs.get(&rule);
    let b = order.b;
    let mut bound = VarSet::new();
    order
        .steps
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let neg = b.plain_atom(s.at).is_some_and(|a| a.neg());
            let fan = if i > 0 && b.plain_atom(s.at).is_some() && !neg {
                seen.and_then(|o| o.get(&b.sig(s.at, &bound))).filter(|n| n[0] >= OBS_MIN).map(|n| n[1] as f64 / n[0] as f64)
            } else {
                None
            };
            if !neg {
                bound.extend(b.vars(s.at));
            }
            fan
        })
        .collect()
}

/// The changed inputs a stratum in the maintain mode reads (decision 0364): each one's old rows, rows gone and rows
/// added, as sources.
/// A view of an input's columns (`input_view_shape`, Q37) has its input's, read through its columns (`Some`).
/// A changed read's versions for the maintain mode, each read in place: `[old, minus, plus, new]`, and the columns of
/// its source each holds (an input view's), or all.
type ModeInputs = BTreeMap<String, ([Arc<dyn crate::code_rule_dsl::facts::vtab::Source>; 4], Option<Vec<usize>>)>;

/// The engine's list a maintain mode's joined line runs in (`setfix`): its probes (seeds and delta probes), then its
/// removals, checked adds and rechecks (`maint_step_counts`), each in spec order.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ModeLine {
    Probe,
    Remove,
    Add,
    Recheck,
}

/// Strata the evaluator's maintain mode maintained (decision 0364 stage 3), over the process.
static MODE_STRATA: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
#[cfg_attr(not(any(test, feature = "test-hooks")), allow(dead_code))]
pub fn mode_strata() -> u64 {
    MODE_STRATA.load(std::sync::atomic::Ordering::Relaxed)
}

/// A column's type in the native store's spec: `s`, a symbol it reads as text, only where symbols are text. Over
/// atoms every column is an integer (a name, kind or field atom), as the planner's SQL holds it (O13).
fn sym_char(inputs: &Inputs, t: &str) -> char {
    if t == "sym" && inputs.atoms.is_none() { 's' } else { 'i' }
}

/// A relation's column types as the store keeps its rows: `s` where it declares a symbol, whatever the engine holds
/// the column as (over atoms, an integer). The store maps a forked process's symbols by them (decision 0170); rows of
/// another width than declared keep the engine's types.
pub(crate) fn stored_types(f: Fields<'_>, rel: &str, engine: Vec<u8>) -> Vec<u8> {
    let declared = f.types(rel);
    if declared.len() != engine.len() {
        return engine;
    }
    declared.iter().map(|t| if *t == "sym" { b's' } else { b'i' }).collect()
}


/// A relation's statistics from the native store (`set_fixpoint_stats`, T's `live_stats`): rows, then per column
/// the distinct count and the sum of squared postings, whose ratio to rows is the skew-weighted fanout ingestion
/// gives inputs.
/// A native relation's live statistics (`setfix::live`): rows, per column distinct values and the sum of squared
/// postings, and each value's postings in a column of few values.
pub struct LiveRel {
    pub n: i64,
    pub delta: i64,
    pub ndv: Vec<i64>,
    pub sq: Vec<i64>,
    pub freq: Vec<(usize, HashMap<Constant, i64>)>,
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
        LiveRel { n, delta: n, ndv: rest[..k].to_vec(), sq: rest[k..].to_vec(), freq }
    }
}

/// A recursive relation's statistics measured before this run, when the caller gave them under its name
/// (`Stats::measured`: the cross-file rounds give each hop relation's rows of the round before): its rows, and its
/// columns' distinct counts and fanouts, which otherwise start at 1 until the first replan.
fn prior_stats(model: &mut Model, f: Fields<'_>, r: &str) -> Option<f64> {
    // Old rows whose statistics are made when first planned with: their count now, their columns' when a plan reads
    // `r` (`Model::resolve`, `prior_now`).
    let old = format!("{}__old", r.strip_suffix("__live").unwrap_or(r));
    if f.declared(r) && !model.stats.knows_now(r) && model.stats.lazy.0.contains_key(old.as_str()) {
        model.stats.prior_later.insert(r.to_owned());
        return model.stats.card.get(&old).map(|n| n.max(1.0));
    }
    prior_now(model, f, r, false)
}

/// `prior_stats`, its old rows' statistics made now. Deferred (`fill`), they fill only what nothing set since, as
/// a later write overrides them when they are made at once.
fn prior_now(model: &mut Model, f: Fields<'_>, r: &str, fill: bool) -> Option<f64> {
    let width = f.arity(r)?;
    let old = format!("{}__old", r.strip_suffix("__live").unwrap_or(r));
    model.resolve(f, [r, old.as_str()]).ok()?;
    let measured = |of: &str| (0..width).map(|c| model.stats.col.get(&(of.to_owned(), sql_col(of, c))).copied()).collect::<Option<Vec<[i64; 4]>>>();
    // A maintained relation starts as its old rows (`rel r … base r__old r__del`), which are measured under that
    // name: without them every column read as one value, so a bound column narrowed nothing (`cfn($c, $e, $f)` with
    // `$c` bound priced as all 2,139 rows, and the plan took cross products of `rpvar` and `attr_of__plus` instead).
    // A levels stratum's `h__live` (decision 0090) is `h__old`'s rows too.
    let old = format!("{}__old", r.strip_suffix("__live").unwrap_or(r));
    let cols = measured(r).or_else(|| measured(&old))?;
    let rows = cols.first().map_or(0, |m| m[0]);
    for (c, [n, ndv, sq, _]) in cols.into_iter().enumerate() {
        let (ndv, fan) = (ndv.max(1) as f64, if n != 0 { sq as f64 / n as f64 } else { 1.0 });
        if fill {
            model.ndv.or_insert((r.to_owned(), c), ndv);
            model.stats.fan.or_insert((r.to_owned(), c), fan);
        } else {
            model.ndv.insert((r.to_owned(), c), ndv);
            model.stats.fan.insert((r.to_owned(), c), fan);
        }
    }
    Some((rows as f64).max(1.0))
}

pub fn live_stats(model: &mut Model, r: &str, live: LiveRel) -> Vec<usize> {
    // Level-held rows are represented by their prior statistics, not their temporarily visible subset.
    if r.ends_with("__live") { return Vec::new(); }
    model.stats.prior_later.remove(r);
    let mut changed = Vec::new();
    let card = (live.n as f64).max(1.0);
    if model.stats.card.insert(r.to_owned(), card) != Some(card) { changed.push(0); }
    let mut freq: HashMap<_, _> = live.freq.into_iter().collect();
    for (c, (&ndv, &sq)) in live.ndv.iter().zip(&live.sq).enumerate() {
        let key = (r.to_owned(), c);
        let ndv = ndv.max(1) as f64;
        let fan = if live.n != 0 { sq as f64 / live.n as f64 } else { 1.0 };
        let new_freq = freq.remove(&c);
        let moved = model.ndv.get(&key) != Some(&ndv)
            || model.stats.fan.get(&key) != Some(&fan)
            || model.stats.dfreq.get(&key) != new_freq.as_ref();
        if moved {
            changed.push(c + 1);
            model.ndv.insert(key.clone(), ndv);
            model.stats.fan.insert(key.clone(), fan);
            match new_freq {
                Some(f) => { model.stats.dfreq.insert(key, f); }
                None => { model.stats.dfreq.remove(&key); }
            }
        }
    }
    changed
}

/// A plan's step strings, which say whether a replan changed it.
/// A stratum's name in the profile: its relations, cut at 80 characters.
/// The kind of literal the native set engine has no step for, as the profile groups SQLite's fallbacks: an
/// aggregate by its operator, a function by its name (the first one in the literal), a comparison by its operator,
/// a read by its relation when it is an input, else by who holds it (a relation an earlier SQLite stratum made is
/// a fallback that brings on the next).
/// The kind of literal `i` of `b` that has no native step (`decline_class` of the coverage ledger).
fn decline_class(b: &Body<'_>, i: usize, f: Fields<'_>, native: &BTreeSet<String>) -> String {
    fn first_call<'t>(b: &'t Body<'_>, t: TermRef<'_>) -> Option<&'t str> {
        match t {
            TermRef::Call(f, args) => {
                let f = b.v.text(f);
                Some(args.clone().find_map(|a| first_call(b, a)).filter(|_| f == "concat" || f == "add").unwrap_or(f))
            }
            _ => None,
        }
    }
    let neg = |a: super::program_image::AtomRef<'_>| if a.neg() { "negated " } else { "" };
    match b.view(i) {
        Lit::Agg { op, .. } => format!(
            "aggregate {}",
            match op {
                super::program_image::AggRef::Count => "count",
                super::program_image::AggRef::Min => "min",
                super::program_image::AggRef::Max => "max",
                super::program_image::AggRef::Sum => "sum",
                super::program_image::AggRef::Product => "product",
                super::program_image::AggRef::Join { .. } => "join (string concatenation)",
            }
        ),
        Lit::Atom(a) if f.is_input(b.rel(a)) => format!("{}read of input {}", neg(a), b.rel(a)),
        Lit::Atom(a) if native.contains(b.rel(a)) => format!("{}read of a relation the native engine holds", neg(a)),
        Lit::Atom(a) => format!("{}read of a relation an earlier SQLite stratum made", neg(a)),
        Lit::Cmp { l, op, r } => match first_call(b, l).or_else(|| first_call(b, r)) {
            Some(f) => format!("function @{f}"),
            None => format!("comparison {}", op.as_str()),
        },
        _ => b.show(i).split('(').next().unwrap_or("literal").to_owned(),
    }
}

/// A stratum's key as its span names it: at most 80 bytes of it.
fn stratum_label(scc: &BTreeSet<String>) -> String {
    let key = stratum_key(scc);
    if key.len() > 80 { format!("{}…({} relations)", &key[..key.floor_char_boundary(80)], scc.len()) } else { key }
}

/// A reach index made again (decision 0119) against its old rows kept in memory (decision 0331): its delta held as
/// `rel__minus` and `rel__plus`, the rows of each sorted and compared, so its readers are maintained rather than
/// derived again. Returns whether it changed.
/// The delta of `rel`, derived again (a reach index made again, or a stratum the maintain mode does not take), against
/// its kept rows `old`: held as `rel__minus` and `rel__plus`, its readers' versions. Whether any row moved.
/// A relation's change in a run, which its frame's delta is written from (decision 0117 step 4): its version marks,
/// read when the delta is written (0364 R2: no row is copied to state a change), or the rows a stratum derived again
/// made against its kept rows (`kept_delta`).
#[derive(Clone, Debug)]
pub enum Delta {
    Marks { gone: usize, added: usize },
    Rows { gone: Vec<i64>, added: Vec<i64> },
}

impl Delta {
    /// Its rows gone and added, counted.
    pub fn counts(&self, width: usize) -> (usize, usize) {
        match self {
            Delta::Marks { gone, added } => (*gone, *added),
            Delta::Rows { gone, added } => (gone.len() / width.max(1), added.len() / width.max(1)),
        }
    }
    /// Its rows gone and added, row-major: a mark's read from `rel` now. `None` where `rel` no longer has its version.
    pub fn rows(&self, rel: &super::native::component::InRel) -> Option<(Vec<i64>, Vec<i64>)> {
        match self {
            Delta::Marks { .. } => rel.version_delta().map(|(added, gone)| (gone, added)),
            Delta::Rows { gone, added } => Some((gone.clone(), added.clone())),
        }
    }
}

fn kept_delta(db: &Connection, rel: &str, old: &Arc<super::native::component::InRel>, native_rels: &mut BTreeSet<String>) -> Result<bool> {
    let _span = super::profile::span(|| format!("{rel}: its delta against its kept rows"));
    // Where it can be, the relation derived again is made a version of its kept rows in place (0364 R2): its change
    // is then its marks, and nothing is copied or held beside it.
    if let Some(new) = super::native::component::held(db, rel)
        && let Some(rebased) = super::native::component::InRel::rebased(rel, old, &new)
    {
        let (gone, added) = rebased.version_counts().expect("started from its kept rows");
        super::native::component::hold_as(db, rel, Arc::new(rebased));
        return Ok(gone + added > 0);
    }
    if std::env::var_os("RERAIL_SHAPE_TRACE").is_some() {
        let new = super::native::component::held(db, rel);
        eprintln!("RERAIL_SHAPE_TRACE {rel}: not rebased (kept: view {}, levels {}, {} rows; new: {})", old.view_kind().is_some(), old.has_levels(), old.rows(),
                  new.map_or("not held".to_owned(), |n| format!("view {}, levels {}, {} rows", n.view_kind().is_some(), n.has_levels(), n.rows())));
    }
    let types = old.types().to_vec();
    let k = types.len();
    let new = super::native::component::derived(db, rel).ok_or_else(|| Error::Plan(format!("{rel}: derived again, but not held")))?;
    let rows = |s: &dyn crate::code_rule_dsl::facts::vtab::Source| -> Vec<Vec<i64>> {
        let cols: Vec<usize> = (0..k).collect();
        let mut v: Vec<Vec<i64>> = (0..s.len()).map(|r| { let mut row = vec![0i64; k]; s.row_at(r, &cols, &mut row); row }).collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    let (was, now) = (rows(old.source((0..k).map(|c| sql_col(rel, c)).collect()).as_ref()), rows(new.as_ref()));
    crate::code_rule_dsl::logic::plan::corpus::maintain::shape::rows_copied(was.len() + now.len());
    let (mut minus, mut plus) = (Vec::new(), Vec::new());
    let (mut i, mut j) = (0, 0);
    while i < was.len() || j < now.len() {
        match (was.get(i), now.get(j)) {
            (Some(a), Some(b)) if a == b => (i, j) = (i + 1, j + 1),
            (Some(a), Some(b)) if a < b => { minus.extend_from_slice(a); i += 1; }
            (Some(_), Some(b)) | (None, Some(b)) => { plus.extend_from_slice(b); j += 1; }
            (Some(a), None) => { minus.extend_from_slice(a); i += 1; }
            (None, None) => unreachable!(),
        }
    }
    let changed = !minus.is_empty() || !plus.is_empty();
    for (suffix, flat) in [("minus", minus), ("plus", plus)] {
        let name = format!("{rel}__{suffix}");
        let n = flat.len() / k.max(1);
        super::native::component::hold_rows(db, &name, types.clone(), flat, n);
        native_rels.insert(name);
    }
    Ok(changed)
}

fn stratum_key(scc: &BTreeSet<String>) -> String {
    let key = scc.iter().cloned().collect::<Vec<_>>().join("/");
    if key.len() > 80 { format!("{}…({} relations)", &key[..key.floor_char_boundary(80)], scc.len()) } else { key }
}

/// A delta probe's plan with the stratum's own relations named as read: the last wave's delta at `at`, else the
/// rows so far.
fn delta_sources(mut x: super::profile::RulePlan, order: Order<'_, '_>, at: usize, scc: &BTreeSet<String>) -> super::profile::RulePlan {
    for (i, st) in order.steps.iter().enumerate() {
        if let Some(l) = order.b.plain_atom(st.at)
            && scc.contains(order.b.rel(l))
        {
            x.steps[i].source = if i == at { "delta (last wave)".into() } else { "this stratum".into() };
        }
    }
    x
}

fn shown(order: Order<'_, '_>) -> Vec<String> {
    order.b.shown(order.steps)
}

/// Whether two orders of one body are the same: each step's position and mode (what `shown` compared as text,
/// without writing either out).
fn same_order(a: &[Step], b: &[Step]) -> bool {
    a == b
}

/// The relations rule `k`'s atoms and aggregates read, in body order, by its handles in `vs`.
fn reads_of(vs: &body::Versions, k: RuleKey) -> impl Iterator<Item = &str> + '_ {
    let (v, rule) = vs.rule_at(k);
    rule.body().filter_map(|l| l.atom()).map(move |a| v.rel_name(a.rel()))
}

/// The relations of `scc` that reach themselves through `rules` (any read: positive, negated or aggregated), each
/// read by its handles.
fn cyclic_relations(vs: &body::Versions, scc: &BTreeSet<String>, rules: &[RuleKey]) -> BTreeSet<String> {
    cyclic_in(scc, rules.iter().flat_map(|&r| {
        let head = vs.head_at(r);
        reads_of(vs, r).map(move |a| (head, a))
    }))
}

/// The relations of `scc` that reach themselves through `edges`, each `(head, a relation its rule reads)`.
fn cyclic_in<'a>(scc: &BTreeSet<String>, edges: impl Iterator<Item = (&'a str, &'a str)>) -> BTreeSet<String> {
    // A relation reaches itself where it is in a strongly connected part of more than one relation, or reads itself:
    // one pass over the edges (Tarjan's), not a walk from each relation (a 220-relation stratum walked 48,000 times).
    let names: Vec<&str> = scc.iter().map(String::as_str).collect();
    let at = |r: &str| names.binary_search(&r).ok();
    let mut next: Vec<Vec<usize>> = vec![Vec::new(); names.len()];
    let mut own = vec![false; names.len()];
    for (head, read) in edges {
        let (Some(h), Some(b)) = (at(head), at(read)) else { continue };
        next[b].push(h);
        own[b] |= b == h;
    }
    let n = names.len();
    let (mut index, mut low, mut on) = (vec![usize::MAX; n], vec![0; n], vec![false; n]);
    let (mut stack, mut count, mut cyclic) = (Vec::new(), 0, vec![false; n]);
    for root in 0..n {
        if index[root] != usize::MAX {
            continue;
        }
        // Iteratively: (node, its next edge).
        let mut work: Vec<(usize, usize)> = vec![(root, 0)];
        index[root] = count;
        low[root] = count;
        count += 1;
        stack.push(root);
        on[root] = true;
        while let Some(&mut (v, ref mut e)) = work.last_mut() {
            if let Some(&w) = next[v].get(*e) {
                *e += 1;
                if index[w] == usize::MAX {
                    index[w] = count;
                    low[w] = count;
                    count += 1;
                    stack.push(w);
                    on[w] = true;
                    work.push((w, 0));
                } else if on[w] {
                    low[v] = low[v].min(index[w]);
                }
                continue;
            }
            work.pop();
            if let Some(&(u, _)) = work.last() {
                low[u] = low[u].min(low[v]);
            }
            if low[v] == index[v] {
                let mut part = Vec::new();
                while let Some(w) = stack.pop() {
                    on[w] = false;
                    part.push(w);
                    if w == v {
                        break;
                    }
                }
                let multi = part.len() > 1;
                for w in part {
                    cyclic[w] = multi || own[w];
                }
            }
        }
    }
    names.iter().zip(cyclic).filter(|(_, c)| *c).map(|(r, _)| (*r).to_owned()).collect()
}

/// Whether `b` reads, positively, a change of a relation outside `scc`: rows a lower stratum lost, gained or
/// decided (decisions 0077, 0090).
fn reads_change(b: &Body<'_>, scc: &BTreeSet<String>) -> bool {
    const CHANGES: [&str; 10] = ["__plus", "__minus", "__ins", "__del", "__gone", "__lost", "__redo", "__cand", "__chk", "__held"];
    // An aggregate's changed groups (`h__grpR_K`, `maintain`): a row of the head that read one is derived again.
    let groups = |rel: &str| rel.rsplit_once("__").is_some_and(|(_, s)| s.starts_with("grp"));
    (0..b.len()).filter_map(|i| b.plain_atom(i)).filter(|a| !a.neg()).map(|a| b.rel(a)).any(|rel| !scc.contains(rel) && (CHANGES.iter().any(|c| rel.ends_with(c)) || groups(rel)))
}

/// The component a stratum's plans are made as solving (`Stats::solve`, in their keys): itself when a rule reads it,
/// else none. Its seeds planned ahead (`seed_specs`, `Model::wants_strata`) are keyed as the stratum keys them.
fn solving_of(vs: &body::Versions, scc: &BTreeSet<String>, rules: &[RuleKey]) -> Option<BTreeSet<String>> {
    rules.iter().any(|&r| {
        let (v, rule) = vs.rule_at(r);
        rule.body().any(|l| matches!(l.view(), Lit::Atom(a) if !a.neg() && scc.contains(v.rel_name(a.rel()))))
    }).then(|| scc.clone())
}

/// The positive atoms of `scc` in `b` (its recursive atoms): each one's place in the body and its relation.
fn recursive<'v>(b: &Body<'v>, scc: &BTreeSet<String>) -> Vec<(usize, &'v str)> {
    (0..b.len())
        .filter_map(|i| b.plain_atom(i).filter(|a| !a.neg()).map(|a| (i, b.rel(a))))
        .filter(|(_, rel)| scc.contains(*rel))
        .collect()
}

/// Term `t` of `v`'s rule `r`, to write into a builder of another version: its names by their text.
fn term_text<'v>(v: &'v Version, r: super::program_image::RuleRef<'v>, t: TermRef<'v>) -> super::program_image::T<'v> {
    use super::program_image::T;
    match t {
        TermRef::Var(s) => T::Var(v.text(r.var(s))),
        TermRef::Wild => T::Wild,
        TermRef::Top => T::Top,
        TermRef::Int(i) => T::Int(i),
        TermRef::Branded(b) => T::Branded(b),
        TermRef::Str(a) => T::Str(v.text(a)),
        TermRef::Call(f, args) => T::Call(v.text(f), args.map(|a| term_text(v, r, a)).collect()),
    }
}

/// A rule a fixpoint makes of a part of one of the program's (`native_fixpoint`'s parts, an edge table's rest): its
/// head, the atoms written before its body (`pre`), and its body, literals of `from`'s rule by place.
struct Part<'v> {
    rel: String,
    head: Vec<super::program_image::T<'v>>,
    pre: Vec<(&'static str, Vec<super::program_image::T<'v>>)>,
    from: (&'v Version, super::program_image::RuleRef<'v>),
    lits: Vec<usize>,
}

/// `parts`, rules of a version of their own (their literals copied by their names), in order: each planned alone.
fn made_parts<'a, 'v: 'a>(parts: impl IntoIterator<Item = &'a Part<'v>>) -> Result<Arc<Version>> {
    let mut b = super::program_image::Builder::base();
    for p in parts {
        let rel = b.rel(&p.rel);
        let mut rule = b.rule().head(rel, &p.head);
        for (pre, args) in &p.pre {
            let pre = rule.builder().rel(pre);
            rule = rule.atom(pre, args);
        }
        let (v, r) = p.from;
        let lits: Vec<_> = r.body().collect();
        for &i in &p.lits {
            rule = rule.lit(v, r, lits[i]);
        }
        rule.finish().map_err(Error::Plan)?;
    }
    b.finish().map(|(v, _)| v).map_err(Error::Plan)
}

/// A native spec's token for a constant term of `v`.
fn const_tok(v: &Version, t: TermRef<'_>) -> Option<Tok> {
    match t {
        TermRef::Int(i) => Some(Tok::Int(i)),
        TermRef::Branded(a) => Some(Tok::Int(a.value())),
        TermRef::Str(s) => Some(Tok::Sym(v.text(s).to_owned())),
        _ => None,
    }
}

/// A native stratum, run on its own model off the run's thread (decision 0113 §4): a `Run` with the connection's
/// scope alone (`Db::detached`), no answer hook, its own model (`Model::fork`) and the native relations it reads.
/// What it shares with the run (the program, the views, the inputs, the rules' numbers) is read-only while the strata
/// run. The engine's relations and components it makes are the run's (they are kept by scope), and what it learnt
/// goes back to the run when it retires (`Run::absorb`), in program order.
/// A stratum's run off the run's thread, and the atoms' arena of its scope (whose new atoms it reserves).
/// Its third part: the device waves its job flushed and the launches they held (`lanes::wave`), credited to the run's
/// thread when the strand is taken back, as its relations are: a level's launches a job ran are the level's wave.
struct Strand<'a>(Run<'a, 'static>, Arc<crate::content_store::atoms::Atoms>, (u64, u64));

// SAFETY: a strand holds no connection (`Db::detached`, whose SQL is an error) and no answer hook (`strand` makes it
// with neither). Its model, relations and maps are its own. What it shares with the run is read-only while it runs: the
// program, the views, the projections, the inputs, and the rules' numbers, whose addresses are keys only.
unsafe impl Send for Strand<'_> {}

/// What a stratum's job takes from the run's thread (decision 0113 §4): the run's context (`run_state::Carry`): its
/// kept plans, derived values and observations, which it shares, and the thread's settings, which it copies.
struct Ambient(super::run_state::Carry);

impl Ambient {
    fn here() -> Self {
        Ambient(super::run_state::Carry::here())
    }

    /// While the guard lives, this thread works in the run's ambient.
    fn enter(&self) -> impl Drop {
        struct Entered(super::run_state::Entered, super::profile::Span);
        impl Drop for Entered {
            fn drop(&mut self) {}
        }
        let span = super::profile::span_at(self.0.under(), || "issued components (stream job)".into());
        Entered(self.0.enter(), span)
    }
}

impl<'a> Run<'a, '_> {
    /// The relations a native stratum's own model holds (`Model::fork`), when it may run off the run's thread: it
    /// reads no SQL (every relation its rules read is the engine's, its own, or an input bound in place), answers no
    /// request, and runs no native operator. The relations `held_too` (a component's earlier members', decision 0113
    /// §2) are taken as held. `Err`: why it runs here.
    fn strand_reads_with(&self, scc: &BTreeSet<String>, rules: &[RuleKey], held_too: &BTreeSet<String>) -> std::result::Result<BTreeSet<String>, &'static str> {
        if std::env::var_os("RERAIL_RELATIONAL_LAB").is_some() {
            return Err("the relational lab");
        }
        if self.model.inputs.atoms.is_none() {
            return Err("no atoms (a SQL run)");
        }
        let f = self.fields();
        for r in scc {
            let why = if f.lattice(r) { "a lattice" }
                else if f.native(r).is_some() { "a native operator" }
                else if f.request(r) { "a request answered" }
                else if f.produced(r) { "an answer produced" }
                else if f.has_facts(r) { "facts" }
                else if self.views.contains(r) { "a view" }
                else { continue };
            return Err(why);
        }
        if rules.iter().any(|&r| self.versions.rule_at(r).1.body().any(|l| matches!(l.view(), Lit::Ast { .. }))) {
            return Err("an Ast literal");
        }
        if self.input_view_stratum(scc, rules).is_some() {
            return Err("a view of an input's columns, held on the run's thread");
        }
        let mut reads: BTreeSet<String> = scc.clone();
        for &r in rules {
            let (v, rule) = self.versions.rule_at(r);
            for a in rule.body().filter_map(|l| l.atom()) {
                let rel = v.rel_name(a.rel());
                if scc.contains(rel) {
                    continue;
                }
                let held = self.native_rels.contains(rel) || held_too.contains(rel);
                let bound = self.inputs.contains(rel) && crate::code_rule_dsl::facts::vtab::bound_source(self.db, rel).is_some();
                let viewed = self.in_place(rel) && self.model.view_src.get(rel).is_some_and(|(base, _)| crate::code_rule_dsl::facts::vtab::bound_source(self.db, base).is_some());
                if !(held || bound || viewed) {
                    return Err(if self.inputs.contains(rel) { "reads an input not bound in place" } else { "reads a relation not held (SQL's)" });
                }
                reads.insert(rel.to_owned());
                if let Some((base, _)) = self.model.view_src.get(rel) {
                    reads.insert(base.clone());
                }
            }
        }
        for r in scc {
            reads.insert(format!("{}__old", r.strip_suffix("__live").unwrap_or(r)));
        }
        Ok(reads)
    }

    /// A strand for a stratum that reads `reads` (`strand_reads_with`) and whose rules' digests are `rules`.
    fn strand(&mut self, reads: &BTreeSet<String>, rules: &[u64], resolve: &[&str]) -> Result<Strand<'a>> {
        let arena = super::native::component::prime(&self.db)?;
        let model = self.model.fork(self.versions.whole(self.version), reads, rules, resolve.iter().copied())?;
        Ok(Strand(Run {
            db: self.db.detached(),
            model,
            views: Arc::clone(&self.views),
            made: HashSet::new(),
            device_views: HashSet::new(),
            orders: HashMap::new(),
            keyless: HashSet::new(),
            native_rels: reads.iter().filter(|r| self.native_rels.contains(*r)).cloned().collect(),
            demands: Default::default(),
            pending: HashMap::new(),
            why_not: None,
            projections: Arc::clone(&self.projections),
            inputs: Arc::clone(&self.inputs),
            versions: Arc::clone(&self.versions),
            version: self.version,
            answer: None,
            texts: self.texts.clone(),
            asking: None,
            why_class: None, mode_reads: Default::default(),
            answered: BTreeMap::new(),
            logging: None,
            answer_logs: HashMap::new(),
            quiet: Arc::clone(&self.quiet),
        }, arena, (0, 0)))
    }



    /// Takes back a strand of the stratum `scc` that ran: its relations, and what its model learnt of them and of
    /// its rules (`rules`, their digests).
    fn absorb(&mut self, s: Strand<'a>, scc: &BTreeSet<String>, rules: &[u64]) {
        let Strand(run, _, waves) = s;
        super::lanes::wave::credit(waves);
        self.model.absorb(self.versions.whole(self.version), run.model, scc, rules);
        self.native_rels.extend(run.native_rels.into_iter().filter(|r| scc.contains(r)));
    }
}


impl<'a> Strand<'a> {

    /// Runs a component's members (decision 0113 §2) in program order on this thread, each to its fixpoint: how many
    /// ran natively, and what stopped the next, if one did (a decline and its reason, or an error). A member that
    /// reads one waiting on its device wave has that wave flushed first. The members still waiting when the last has
    /// run are left in `park` (decision 0123 §6): their launches go out with the level's wave, on the run's thread
    /// (`resume_parked`). With no `park`, the component flushes them as its own wave.
    fn run_members(&mut self, members: &[(BTreeSet<String>, Vec<RuleKey>)], times: &mut Vec<(std::time::Instant, std::time::Instant)>, reserved: &mut crate::content_store::atoms::Reserved, park: Option<&mut Vec<(usize, Box<Driving>)>>) -> (usize, Option<Result<(Option<String>, Option<String>)>>) {
        let t0 = std::time::Instant::now();
        times.clear();
        times.resize(members.len(), (t0, t0));
        // The atoms it makes are its own until it retires (decision 0114 §4): reserved, then published.
        struct End<'r>(Option<crate::content_store::atoms::Reservation>, &'r mut crate::content_store::atoms::Reserved);
        impl Drop for End<'_> {
            fn drop(&mut self) {
                if let Some(r) = self.0.take() {
                    *self.1 = r.end();
                }
            }
        }
        let _end = End(Some(crate::content_store::atoms::reserving(&self.1)), reserved);
        // Members that wait on their device wave (decision 0113 §3): the next members run meanwhile, and their
        // launches go out as one wave, when a member reads a waiting one or the component ends.
        let mut waiting: Vec<(usize, Box<Driving>)> = Vec::new();
        let finish = |run: &mut Run<'a, 'static>, waiting: &mut Vec<(usize, Box<Driving>)>, times: &mut Vec<(std::time::Instant, std::time::Instant)>| -> Result<()> {
            let ks: Vec<usize> = waiting.iter().map(|w| w.0).collect();
            while !waiting.is_empty() {
                super::lanes::wave::flush(waiting.len()).map_err(Error::Plan)?;
                for (k, d) in std::mem::take(waiting) {
                    let _span = super::profile::span(|| format!("stratum {} (in a job, after its device wave)", stratum_label(&members[k].0)));
                    match run.native_resume(d)? {
                        Native::Ran => {}
                        Native::Waiting(d) => waiting.push((k, d)),
                        Native::Declined => return Err(Error::Plan(format!("{:?}: a stratum that waited on its wave declined", members[k].0))),
                        Native::Asking(..) => return Err(Error::Plan(format!("{:?}: requests posted off the run's thread", members[k].0))),
                    }
                }
            }
            let now = std::time::Instant::now();
            for k in ks {
                times[k].1 = now;
            }
            Ok(())
        };
        for (k, (scc, rules)) in members.iter().enumerate() {
            let reads_waiting = waiting.iter().any(|(w, _)| rules.iter().any(|&r| {
                let (v, rule) = self.0.versions.rule_at(r);
                rule.body().filter_map(|l| l.atom()).any(|a| members[*w].0.contains(v.rel_name(a.rel())))
            }));
            if reads_waiting && let Err(e) = finish(&mut self.0, &mut waiting, times) {
                return (0, Some(Err(e)));
            }
            times[k].0 = std::time::Instant::now();
            let stop = match { let _span = super::profile::span(|| format!("stratum {} (in a job)", stratum_label(scc))); self.0.native_stratum(scc, rules) } {
                Ok(Native::Ran) => {
                    times[k].1 = std::time::Instant::now();
                    continue;
                }
                Ok(Native::Waiting(d)) => {
                    waiting.push((k, d));
                    continue;
                }
                Ok(Native::Asking(..)) => Err(Error::Plan(format!("{scc:?}: requests posted off the run's thread"))),
                Ok(Native::Declined) => Ok((self.0.why_not.take(), self.0.why_class.take())),
                Err(e) => Err(e),
            };
            return match finish(&mut self.0, &mut waiting, times) {
                Ok(()) => (k, Some(stop)),
                Err(e) => (0, Some(Err(e))),
            };
        }
        if let Some(park) = park {
            park.append(&mut waiting);
            return (members.len(), None);
        }
        match finish(&mut self.0, &mut waiting, times) {
            Ok(()) => (members.len(), None),
            Err(e) => (0, Some(Err(e))),
        }
    }

    /// Runs `f` in the strand's reservation (decision 0114 §4), as `run_members` runs its members: what it made is added
    /// to `reserved`.
    fn reserving<R>(&mut self, reserved: &mut crate::content_store::atoms::Reserved, f: impl FnOnce(&mut Self) -> R) -> R {
        let r = crate::content_store::atoms::reserving(&self.1);
        let out = f(self);
        reserved.extend(r.end());
        out
    }

    /// Flushes the wave of the members `run_members` parked, on this thread, until none waits.
    fn flush_parked(&mut self, members: &[(BTreeSet<String>, Vec<RuleKey>)], waiting: &mut Vec<(usize, Box<Driving>)>, times: &mut [(std::time::Instant, std::time::Instant)]) -> Result<()> {
        while !waiting.is_empty() {
            super::lanes::wave::flush(waiting.len()).map_err(Error::Plan)?;
            self.resume_parked(members, waiting, times)?;
        }
        Ok(())
    }

    /// Resumes the members `run_members` parked (`waiting`) once the run's thread flushed their wave: those that wait
    /// on a wave again stay in `waiting`, for the next flush.
    fn resume_parked(&mut self, members: &[(BTreeSet<String>, Vec<RuleKey>)], waiting: &mut Vec<(usize, Box<Driving>)>, times: &mut [(std::time::Instant, std::time::Instant)]) -> Result<()> {
        for (k, d) in std::mem::take(waiting) {
            let _span = super::profile::span(|| format!("stratum {} (in a job, after its device wave)", stratum_label(&members[k].0)));
            match self.0.native_resume(d)? {
                Native::Ran => times[k].1 = std::time::Instant::now(),
                Native::Waiting(d) => waiting.push((k, d)),
                Native::Declined => return Err(Error::Plan(format!("{:?}: a stratum that waited on its wave declined", members[k].0))),
                Native::Asking(..) => return Err(Error::Plan(format!("{:?}: requests posted off the run's thread", members[k].0))),
            }
        }
        Ok(())
    }
}

impl<'a> Run<'a, '_> {
    /// Decision 0209 §4 and §6: makes the heavy values of `rules`' join columns, in one batch (`table::heavy_batch`):
    /// a column of a variable that two positive atoms or more hold, of a relation not being solved, whose heavy
    /// values are not made over its rows now. With `raise`, a column whose values are skewed past `table::HEAVY`
    /// (ndv over it, and sq · ndv / rows² over 2) is raised; otherwise only raised columns are made. Returns the
    /// columns made.
    fn heavy_for(&mut self, rules: impl IntoIterator<Item = RuleKey>, raise: bool, key: Option<PlanKey>) -> usize {
        static ON: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var("RERAIL_STATS_HEAVY").map_or(true, |v| v != "off"));
        if !*ON || !raise && !joins::any_raised() {
            return 0;
        }
        let mut want: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
        let vs = Arc::clone(&self.versions);
        let raised = joins::raised_all();
        for r in rules {
            let b = vs.at(r);
            // Not raising: only a raised column is made, so a rule that reads no raised relation has none.
            if !raise && !(0..b.len()).any(|i| matches!(b.view(i), Lit::Atom(a) if !a.neg() && raised.iter().any(|(rel, _)| rel == b.rel(a)))) {
                continue;
            }
            let mut held: HashMap<VarSlot, Vec<(&str, usize)>> = HashMap::new();
            for i in 0..b.len() {
                let Lit::Atom(a) = b.view(i) else { continue };
                if a.neg() { continue }
                for (c, t) in b.args(i).iter().enumerate() {
                    if let Some(v) = t.var() { held.entry(v).or_default().push((b.rel(a), c)); }
                }
            }
            for cols in held.values().filter(|cols| cols.len() >= 2) {
                for &(rel, c) in cols {
                    if self.model.stats.solving.contains(rel) || self.heavy_current(rel, c) {
                        continue;
                    }
                    let wanted = raised.iter().any(|(r, k)| r == rel && *k == c) || raise && {
                        let (rows, ndv) = (self.model.card(rel), self.model.ndv.at(rel, c).copied().unwrap_or(1.0));
                        let fan = self.model.stats.fan.at(rel, c).copied().unwrap_or(rows / ndv.max(1.0));
                        ndv > table::HEAVY as f64 && fan * ndv / rows.max(1.0) > 2.0
                    };
                    if wanted { want.entry(rel.to_owned()).or_default().insert(c); }
                }
            }
        }
        if want.is_empty() {
            return 0;
        }
        let _span = super::profile::span(|| "heavy values (decision 0209)".into());
        use super::native::component as engine;
        // O172: a relation whose heavy values were made over it as it was (`heavy_mark`) merges its change since;
        // the others are counted, in one batch.
        let mut n = 0;
        let mut full: Vec<(String, Vec<usize>)> = Vec::new();
        for (rel, cols) in want {
            let cols: Vec<usize> = cols.into_iter().collect();
            let now = engine::held(self.db, &rel);
            let change = now.as_ref().zip(self.model.stats.heavy_mark.get(&rel))
                .filter(|_| cols.iter().all(|&c| self.model.stats.heavy.at(&rel, c).is_some()))
                .and_then(|(now, mark)| now.delta_ids_since_mark(mark).map(|_| ()));
            let (Some(now), Some(())) = (now, change) else { full.push((rel, cols)); continue };
            // Its change read where its rows lie, none copied (0364 R2).
            let mark = self.model.stats.heavy_mark.get(&rel).expect("checked");
            let delta = now.delta_ids_since_mark(mark).expect("checked");
            for &c in &cols {
                let count = |sign: i64| -> rustc_hash::FxHashMap<i64, u64> {
                    let mut m = rustc_hash::FxHashMap::default();
                    delta.each(sign, &mut |row| *m.entry(row[c]).or_default() += 1);
                    m
                };
                let old = self.model.stats.heavy.at(&rel, c).cloned().expect("checked");
                let ndv = self.model.ndv.at(&rel, c).copied().unwrap_or(old.ndv);
                let h = old.merge(&count(1), &count(-1), ndv);
                if raise { joins::raise(key, &rel, c); }
                joins::log(|| format!("heavy values {rel}.{c}, merged: {} rows added, {} removed; {} rows, {} values", delta.counts().1, delta.counts().0, h.rows, h.ndv));
                self.model.stats.heavy.insert((rel.as_str(), c), std::sync::Arc::new(h));
                n += 1;
            }
            self.model.stats.heavy_mark.insert(rel.as_str(), std::sync::Arc::new(now.mark()));
        }
        let held: Vec<_> = full.iter().map(|(rel, _)| engine::held(self.db, rel)).collect();
        let made = table::heavy_batch(full.iter().zip(&held).map(|((rel, cols), now)| (engine::derived(self.db, rel), now.clone(), cols.clone())).collect());
        for (((rel, cols), got), now) in full.iter().zip(made).zip(held) {
            let Some(got) = got else { continue };
            for (&c, h) in cols.iter().zip(got) {
                if raise { joins::raise(key, rel, c); }
                joins::log(|| format!("heavy values {rel}.{c}: {} rows, {} values, top {} hold {:.0}% (batches: {:?} CPU, GPU; {} columns read in place)", h.rows, h.ndv, h.top.len(), 100.0 * (1.0 - h.tail_rows / h.rows.max(1.0)), table::batches(), table::read_in_place()));
                self.model.stats.heavy.insert((rel.as_str(), c), std::sync::Arc::new(h));
                n += 1;
            }
            if let Some(now) = now { self.model.stats.heavy_mark.insert(rel.as_str(), std::sync::Arc::new(now.mark())); }
        }
        n
    }

    /// Whether column `c` of `rel`'s heavy values are of its rows now: made or merged over the engine's relation as
    /// it is (its mark), else, with no relation to mark, within a tenth of its rows (`Model::heavy_of`).
    fn heavy_current(&self, rel: &str, c: usize) -> bool {
        if self.model.stats.heavy.at(rel, c).is_none() {
            return false;
        }
        match (super::native::component::held(self.db, rel), self.model.stats.heavy_mark.get(rel)) {
            (Some(now), Some(mark)) => now.delta_ids_since_mark(mark).is_some_and(|d| d.counts() == (0, 0)),
            (Some(_), None) => false,
            (None, _) => self.model.heavy_of(rel, c).is_some(),
        }
    }

    /// The rows a line's trials draw from (`trial_ids`), split by whether their key is one of a heavy column's values
    /// (O171, and decision 0216's spike): the key is a variable at a column whose heavy values are made, held by the
    /// lead itself, or by the one atom holding both it and a variable of the lead (`inf(f, m)` from
    /// `kind(m, return)`). Returns the heavy column, and the rows of heavy keys and of the rest.
    fn heavy_parts(&self, name: &str, line: usize, b: &Body<'_>, lead: usize) -> Result<Option<HeavyParts>> {
        use super::native::component as engine;
        let Lit::Atom(la) = b.view(lead) else { return Ok(None) };
        // (the linking relation, its key column, its column of the lead's variable and that variable's lead column)
        let found = heavy_link(b, lead, &|rel, c| self.model.heavy_of(rel, c).is_some());
        let Some((link, cv, via)) = found else { return Ok(None) };
        // The heavy values reported: the `HEAVY` largest of those kept.
        let heavy: HashSet<i64> = {
            let h = self.model.heavy_of(&link, cv).expect("found");
            let mut top = h.top.clone();
            top.sort_by(|a, b| b.1.total_cmp(&a.1));
            top.into_iter().take(table::HEAVY).map(|t| t.0).collect()
        };
        // The trial's ids are the relation's own (`trial_ids`), read through it: the SQL source (`engine::derived`)
        // numbers a relation with dead rows by its live rows instead.
        let (Some(ids), Some(lrel)) = (engine::trial_ids(self.db, name, line)?, engine::held(self.db, b.rel(la))) else { return Ok(None) };
        let lsrc = |id: u32, c: usize| lrel.at(id as usize).at(c);
        let mut parts = HeavyParts { link: link.clone(), cv, hot: Vec::new(), cold: Vec::new() };
        let put = |p: &mut HeavyParts, id: u32, vs: &[i64]| if vs.iter().any(|v| heavy.contains(v)) { p.hot.push(id) } else { p.cold.push(id) };
        match via {
            None => {
                for &id in &ids {
                    put(&mut parts, id, &[lsrc(id, cv)]);
                }
            }
            Some((cu, lc)) => {
                // Each lead row's keys: the link probed by its value (decision 0232), not the link read whole.
                let Some(linked) = engine::held(self.db, &link) else { return Ok(None) };
                let mut keys: HashMap<i64, Vec<i64>> = HashMap::new();
                for &id in &ids {
                    let u = lsrc(id, lc);
                    let vs = keys.entry(u).or_insert_with(|| { let mut vs = Vec::new(); linked.probe_each(&[cu], &[u], &mut |row| vs.push(row(cv))); vs });
                    put(&mut parts, id, vs);
                }
            }
        }
        Ok(Some(parts))
    }

    /// Decision 0209 §5's spike: whether a line's best order differs between its heavy key values and the rest.
    /// The key is a variable a raised column holds whose heavy values are made; the lead's rows reach it through
    /// the one atom holding both (`inf(f, m)` from `kind(m, return)`). Each candidate runs to the end over each part,
    /// cut at 4 times the part's least work so far. Logged only.
    #[allow(clippy::too_many_arguments)]
    fn split_spike(&mut self, name: &str, line: usize, b: &Body<'_>, orders: &[Vec<Step>], lead: usize, first: Option<usize>, cut_order: &[Step], ri: &BTreeMap<String, usize>, scc: &BTreeSet<String>, native_rels: &BTreeSet<String>) -> Result<()> {
        let Some(HeavyParts { link, cv, hot, cold, .. }) = self.heavy_parts(name, line, b, lead)? else { joins::log(|| format!("split spike {}: no heavy key reached from the lead", b.head_text())); return Ok(()) };
        use super::native::component as engine;
        let mut report = Vec::new();
        for (part, picked) in [("heavy", &hot), ("rest", &cold)] {
            let mut least: Option<u64> = None;
            let mut runs = Vec::new();
            for (i, o) in orders.iter().enumerate() {
                let budget = least.map_or(20_000_000, |w| (4 * w).max(1_000));
                let run = if i == 0 {
                    engine::trial_over(self.db, name, line, None, None, Ok(picked), budget)?
                } else {
                    let at = first.map(|ai| o.iter().position(|s| s.at == ai));
                    if at == Some(None) { continue }
                    let Some(spec) = self.whole_join_spec(b, o, at.flatten(), ri, scc, native_rels) else { continue };
                    let step = (o[0].at == cut_order[0].at).then_some(0);
                    engine::trial_over(self.db, name, line, Some(&spec), step, Ok(picked), budget)?
                };
                let Some(run) = run else { continue };
                if !run.cut { least = Some(least.map_or(run.work, |w| w.min(run.work))); }
                runs.push(format!("{} {}{}", o.iter().map(|s| s.at.to_string()).collect::<Vec<_>>().join("."), run.work, if run.cut { "+" } else { "" }));
            }
            report.push(format!("{part} ({} rows): {}", picked.len(), runs.join(", ")));
        }
        joins::log(|| format!("split spike {} by {link}.{cv}: {}", b.head_text(), report.join("; ")));
        Ok(())
    }

    /// Decision 0203 §3: the exact pair statistics of rule `r`'s line `line` in component `name`, run in `order`:
    /// for each two positive atoms that share a variable, the rows of the first (past its constants) and the rows
    /// of the second those join, counted from the engine's histograms of both on the shared variables. Each is an
    /// observation of the second atom bound by the first's variables, as `observe` keeps them. `whole`: `order` is the
    /// line's whole plan (O168), and its steps are read there.
    ///
    /// Decision 0354, step 2: not counted where the join model prices (`joins::pricing`): the slow line is planned
    /// again in a round the model prices, which corrects its estimates per step; the counts were the largest
    /// planning cost of a model-planned run (3.2 to 3.8 s of CPU on hogql, a stratum's 1.8 s at once).
    fn exact_statistics(&self, name: &str, line: usize, whole: bool, b: &Body<'_>, order: &[Step]) -> Vec<(Sig, [i64; 2])> {
        if self.model.inputs.atoms.is_none() {
            joins::log(|| format!("exact {}: symbols as text, no histograms", b.head_text()));
            return Vec::new();
        }
        let _span = super::profile::span(|| "exact pair statistics (decision 0203)".into());
        // The order's positive plain atoms: their step and literal.
        let atoms: Vec<(usize, usize)> = order.iter().enumerate().filter_map(|(s, st)| match b.view(st.at) {
            Lit::Atom(a) if !a.neg() => Some((s, st.at)),
            _ => None,
        }).collect();
        let var_at = |i: usize, v: VarSlot| b.args(i).iter().position(|t| t.var() == Some(v));
        let mut pairs: Vec<(usize, usize, Vec<usize>, Vec<usize>)> = Vec::new();
        let mut wanted: Vec<(usize, Vec<usize>)> = Vec::new();
        for (p, &(sp, a)) in atoms.iter().enumerate() {
            for (q, &(sq, c)) in atoms.iter().enumerate() {
                if p == q {
                    continue;
                }
                let mut shared: Vec<VarSlot> = b.args(a).iter().filter_map(|t| t.var()).filter(|&v| var_at(c, v).is_some()).collect();
                shared.sort_by_key(|v| v.index());
                shared.dedup();
                if shared.is_empty() {
                    continue;
                }
                let ca: Vec<usize> = shared.iter().filter_map(|&v| var_at(a, v)).collect();
                let cc: Vec<usize> = shared.iter().filter_map(|&v| var_at(c, v)).collect();
                for w in [(sp, ca.clone()), (sq, cc.clone())] {
                    if !wanted.contains(&w) {
                        wanted.push(w);
                    }
                }
                pairs.push((p, q, ca, cc));
            }
        }
        if pairs.is_empty() {
            joins::log(|| format!("exact {}: no two positive atoms share a variable ({} atoms: {})", b.head_text(), atoms.len(), (0..b.len()).map(|i| b.show(i)).collect::<Vec<_>>().join(", ")));
            return Vec::new();
        }
        let hist = match super::native::component::step_histograms(self.db, name, line, whole, &wanted, joins::HISTOGRAM_ROWS) {
            Ok(h) => h,
            Err(e) => {
                joins::log(|| format!("exact {}: no histograms: {e}", b.head_text()));
                return Vec::new();
            }
        };
        joins::log(|| format!("exact {}: {} pairs, histograms {:?}", b.head_text(), pairs.len(), hist.iter().map(|h| h.as_ref().map(|(n, m)| (*n, m.len()))).collect::<Vec<_>>()));
        let of = |s: usize, cols: &Vec<usize>| wanted.iter().position(|w| w.0 == s && &w.1 == cols).and_then(|i| hist.get(i)?.as_ref());
        let mut out = Vec::new();
        for (p, q, ca, cc) in pairs {
            let ((sp, a), (sq, c)) = (atoms[p], atoms[q]);
            let (Some((rows, ha)), Some((_, hc))) = (of(sp, &ca), of(sq, &cc)) else { continue };
            if (*rows as i64) < OBS_MIN {
                continue;
            }
            let joined: u64 = ha.iter().map(|(k, n)| n * hc.get(k).copied().unwrap_or(0)).sum();
            out.push((b.sig(c, b.vars(a)), [*rows as i64, joined as i64]));
        }
        out
    }

    /// Decision 0203 §5: asks the `join-search` upkeep job for a wider search of rule `r`'s line `key` (its delta
    /// `first`), on a model of the relations it reads.
    ///
    /// Decision 0243: where the line ran (`ran`: its component, its line and the order it ran) and few requests hold
    /// relations already (`joins::may_hold`), the request keeps what a walk reads (`WalkSnap`), and the search walks
    /// its best-priced orders and the one that ran: its proposals are ranked by walked work.
    fn request_search(&mut self, k: RuleKey, first: Option<usize>, rule: u64, key: PlanKey, weight: f64, ran: Option<(&str, usize, &[Step])>) -> Result<()> {
        if !joins::wants_request(&key) {
            return Ok(());
        }
        let heavy = |rel: &str, c: usize| self.model.heavy_of(rel, c).map(|h| {
            let mut top = h.top.clone();
            top.sort_by(|a, b| b.1.total_cmp(&a.1));
            top.into_iter().take(table::HEAVY).map(|t| t.0).collect::<Vec<i64>>()
        });
        let vs = Arc::clone(&self.versions);
        let b = vs.at(k);
        let snap = match ran { Some((name, line, order)) if joins::may_hold() => WalkSnap::take(self.db, name, line, &b, first, &heavy).map(|s| (s, order.to_vec())), _ => None };
        let holds = snap.is_some();
        let rels: BTreeSet<String> = (0..b.len()).filter_map(|i| b.atom(i)).map(|a| b.rel(a).to_owned()).collect();
        // The run's versions, shared: the search prices the rule by its handle, under the fields of its version.
        let (versions, version) = (Arc::clone(&self.versions), self.version);
        let mut model = self.model.fork(versions.whole(version), &rels, &[rule], rels.iter().map(String::as_str))?;
        model.stats.solve(self.model.stats.solving.clone());
        joins::request(key, weight, holds, Box::new(move |over: &dyn Fn() -> bool| {
            let b = versions.at(k);
            let priced = model.explore(versions.whole(version), &b, first);
            match snap {
                Some((snap, ran)) => snap.walk(&b, first, &ran, priced, over),
                None => joins::Searched { orders: priced.into_iter().map(|(o, w)| (joins::steps(&o), w)).collect(), walked: None },
            }
        }));
        Ok(())
    }

    fn fx(&self) -> Fx<'_> {
        Fx { db: self.db, direct: self.model.inputs.atoms.is_some() }
    }

    /// Makes the tables of `rels` not yet made (`pending`).
    fn ensure<'r>(&mut self, rels: impl IntoIterator<Item = &'r String>) -> Result<()> {
        for r in rels {
            if let Some(create) = self.pending.remove(r) {
                crate::code_rule_dsl::facts::vtab::setup_now(&self.db)?;
                // A view's SQL may read another view not yet made, or a relation the native engine holds (a library's
                // relation derived once for the run): each is made, or moved to its table, before the view is.
                let named = |n: &String| sql_names(&create, &crate::code_rule_dsl::logic::sql::ident(n));
                let views: Vec<String> = self.pending.keys().filter(|n| named(n)).cloned().collect();
                self.ensure(&views)?;
                // Not `r` itself: its own table's statement names it, and it is moved there once the table is made.
                let held: Vec<String> = self.native_rels.iter().filter(|n| *n != r && named(n)).cloned().collect();
                self.materialize(&held)?;
                self.db.execute_batch(&create)?;
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

    /// The rules of a stratum not known now to derive nothing (decision 0123: what a level's batch asks statistics
    /// for): none joins a lower relation known to hold no rows, one run natively and held empty or an input delta
    /// bound empty. A relation not run yet is not known; the stratum leaves out what it finds empty then
    /// (`native_stratum_`), and a quiet stratum is one left with no rule.
    fn runnable(&self, scc: &BTreeSet<String>, rules: &[RuleKey]) -> Vec<RuleKey> {
        let empty = |rel: &str| {
            if scc.contains(rel) {
                return false;
            }
            if self.inputs.contains(rel) && (rel.ends_with("__minus") || rel.ends_with("__plus")) {
                return crate::code_rule_dsl::facts::vtab::bound_source(self.db, rel).is_some_and(|s| s.len() == 0);
            }
            self.native_rels.contains(rel) && super::native::component::held(self.db, rel).is_some_and(|h| h.rows() == 0)
        };
        rules.iter().copied().filter(|&r| {
            let (v, rule) = self.versions.rule_at(r);
            !rule.body().any(|l| matches!(l.view(), Lit::Atom(a) if !a.neg() && empty(v.rel_name(a.rel()))))
        }).collect()
    }

    /// T's `native_stratum`: plans with the stratum as the component being solved when a rule reads it.
    /// The join specs of a stratum's seed rules whose every atom reads a relation already run natively (decision
    /// 0100 §5): what a level's batch can pair before the stratum runs. Planned as the stratum will plan them
    /// (`plan_rule_reused` keeps the plan); a spec the stratum ends up planning otherwise is not found, and that seed
    /// runs by the host's walk.
    fn seed_specs(&mut self, scc: &BTreeSet<String>, rules: &[RuleKey]) -> Vec<Arc<JoinSpec>> {
        let versions = Arc::clone(&self.versions);
        let pf = versions.whole(self.version);
        let f = self.fields();
        if scc.iter().any(|r| f.lattice(r) || f.native(r).is_some() || f.gate(r)) {
            return Vec::new();
        }
        let ri: BTreeMap<String, usize> = scc.iter().enumerate().map(|(i, r)| (r.clone(), i)).collect();
        let mut out = Vec::new();
        // One that joins a relation holding no rows derives nothing, and the stratum leaves it out (its rules over
        // empty relations): not planned here either, nor its relations' statistics made (`runnable`).
        let seeds: Vec<RuleKey> = self.runnable(scc, rules).into_iter().filter(|&r| {
            let b = versions.at(r);
            recursive(&b, scc).is_empty() && (0..b.len()).filter_map(|i| b.atom(i)).all(|a| self.native_rels.contains(b.rel(a)))
        }).collect();
        let vs = Arc::clone(&self.versions);
        let bodies: Vec<Body<'_>> = seeds.iter().map(|&r| vs.at(r)).collect();
        // Planned as solving what the stratum will (`solving_of`): their keys are the stratum's.
        let was = std::mem::take(&mut self.model.stats.solving);
        self.model.stats.solve(solving_of(&versions, scc, rules).unwrap_or_default());
        // A round (decision 0123): the statistics of the seeds that will be priced, in one batch.
        let mut wanted = BTreeSet::new();
        self.model.wants(bodies.iter().map(|b| (b, None)), &mut wanted);
        if self.model.make_statistics(pf, &wanted).is_err() {
            self.model.stats.solve(was);
            return out;
        }
        plan_site("a level's seed joins");
        for b in &bodies {
            if self.model.late(pf, b, None, false).is_err() {
                continue;
            }
            let Ok(((order, _, _), _)) = self.model.plan_rule_reused(pf, b, None) else { continue };
            // The level batch keys its pairs by the spec's digest, as the stratum's join does.
            if let Some(spec) = self.whole_join_spec(b, &order, None, &ri, scc, &self.native_rels) {
                out.push(spec);
            }
        }
        self.model.stats.solve(was);
        out
    }

    /// Rule `r` as a view of a bound input (Q37): its body one positive atom of an input bound in place (no
    /// constants, each variable once), its head distinct variables of it, and its rows the input's, one each: the
    /// head keeps the input's key (`node`'s node, `doc`'s and `tape`'s first column) or every column. The input, its
    /// source, and the source's column of each head column.
    fn input_view(&self, r: RuleKey) -> Option<(String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, Vec<usize>)> {
        let (v, rule) = self.versions.rule_at(r);
        let (base, at) = input_view_shape(v, rule, |rel| self.inputs.contains(rel))?;
        let base = base.as_str();
        let args_len = rule.body().next().and_then(|l| l.atom()).map_or(0, |a| a.args().count());
        let src = crate::code_rule_dsl::facts::vtab::bound_source(self.db, base)?;
        let src = src.frozen().unwrap_or(src);
        let names = crate::code_rule_dsl::facts::vtab::named(base, src.as_ref());
        if names.len() != args_len {
            return None;
        }
        let cols: Vec<usize> = at.iter().map(|&c| names.iter().position(|n| *n == self.model.col(base, c))).collect::<Option<_>>()?;
        Some((base.to_owned(), src, cols))
    }

    /// Stratum `scc` (`rules`) as a view of an input's columns (`input_view`): its relation, and what it views.
    fn input_view_stratum(&self, scc: &BTreeSet<String>, rules: &[RuleKey]) -> Option<(String, String)> {
        let f = self.fields();
        let (Some(rel), 1, [r]) = (scc.iter().next(), scc.len(), rules) else { return None };
        if f.has_facts(rel) || f.native(rel).is_some() || f.lattice(rel) || f.request(rel) || f.produced(rel) || f.gate(rel) {
            return None;
        }
        let (base, _, cols) = self.input_view(*r)?;
        Some((rel.clone(), format!("{rel} = {base}{cols:?}")))
    }

    /// T's `native_stratum`: plans with the stratum as the component being solved when a rule reads it. The stratum
    /// may come back waiting on its device wave (`Native::Waiting`); `native_resume` finishes it once the wave ran.
    /// Whether a native join reads the view `r` from its input in place (`projections`): no SQL view is made for it
    /// unless SQL reads it.
    fn in_place(&self, r: &str) -> bool {
        self.projections.contains_key(r) && self.model.view_src.get(r).is_some_and(|(_, vc)| vc.iter().all(Option::is_some))
    }

    /// Why a stratum cannot take the maintain mode (decision 0364 stage 3), or `None` when it can: one relation, no
    /// rule of it reading it (not recursive), each rule's body joins, negations and comparisons with a head of
    /// variables and constants, its kept rows in memory, and each read that changed (`moved`) a native relation
    /// that started from its kept one this run (it has a version).
    fn maintain_mode_blocks(&self, scc: &BTreeSet<String>, rules: &[RuleKey], moved: &BTreeSet<String>, olds: &BTreeMap<String, Arc<super::native::component::InRel>>, ins: &ModeInputs) -> Option<String> {
        if std::env::var("RERAIL_MAINTAIN_MODE").is_ok_and(|v| v == "off") {
            return Some("RERAIL_MAINTAIN_MODE=off".into());
        }
        // Nothing kept: its part's first run (a cold run keeps nothing, and its part is read from no frame), derived.
        if olds.is_empty() {
            return Some(crate::code_rule_dsl::logic::plan::corpus::maintain::FIRST_RUN.into());
        }
        if olds.len() != scc.len() {
            return Some("its kept rows are not in memory".into());
        }
        let f = self.versions.whole(self.version);
        for h in scc {
            if f.native(h).is_some() || f.lattice_decl(h).is_some() || f.gate(h) || f.request(h) || f.produced(h) || f.has_facts(h) || f.is_input(h) {
                return Some("a native, lattice, gate, request, produced, input or fact relation".into());
            }
        }
        // Recursive: by levels (decision 0364 §2), every relation's kept rows with theirs.
        let recursive = rules.iter().any(|&r| { let b = self.versions.at(r); (0..b.len()).any(|i| matches!(b.view(i), Lit::Atom(a) if scc.contains(b.rel(a)))) });
        if recursive && !olds.values().all(|o| o.has_levels() || o.rows() == 0) {
            return Some("recursive, its kept rows without levels".into());
        }
        if rules.is_empty() {
            return Some("no rules".into());
        }
        for &r in rules {
            let b = self.versions.at(r);
            for i in 0..b.len() {
                match b.view(i) {
                    Lit::Atom(a) if scc.contains(b.rel(a)) => {}
                    Lit::Atom(a) if moved.contains(b.rel(a)) && ins.contains_key(b.rel(a)) => {}
                    Lit::Atom(a) if moved.contains(b.rel(a)) => {
                        let held = super::native::component::held(self.db, b.rel(a));
                        if !held.is_some_and(|x| x.start().is_some()) {
                            return Some(format!("{} changed without a version", b.rel(a)));
                        }
                    }
                    Lit::Atom(_) | Lit::Cmp { .. } => {}
                    // An aggregate is maintained by its groups (`maintain_mode`): over a lower stratum's relation with
                    // a version, or one a form maintained; an input's rows are read projected, its versions whole.
                    Lit::Agg { atom: a, .. } if a.gated() => return Some("a gated aggregate".into()),
                    Lit::Agg { atom: a, .. } if scc.contains(b.rel(a)) => return Some("an aggregate over its own stratum".into()),
                    Lit::Agg { atom: a, .. } if moved.contains(b.rel(a)) => {
                        let rel = b.rel(a);
                        // A changed input's, or input view's, versions are given whole, its rows now among them
                        // (`give_versions`), so its aggregate joins read whole columns throughout.
                        match ins.get(rel) {
                            Some(_) => {}
                            None if super::native::component::held(self.db, rel).is_some_and(|x| x.start().is_some()) => {}
                            None => return Some(format!("{rel} changed without a version")),
                        }
                    }
                    Lit::Agg { .. } => {}
                    Lit::Nearest { .. } | Lit::Ast { .. } => return Some("a nearest or ast literal: no native step".into()),
                }
            }
        }
        None
    }

    /// A non-recursive stratum maintained by the evaluator's mode (decision 0364 stage 3): its relation starts as its
    /// kept one (`old`) in place; per rule and changed atom, a removal join (the atom's rows gone, the other changed
    /// atoms as they were) whose rows are candidates, and an add join (the atom's rows added, the others now); per
    /// rule a recheck with the head bound. A negated atom drives the other way (its rows added remove, its rows gone
    /// add) and its adds are checked. Nothing but the relation is made: its versions are the change. `None`: declined;
    /// `Some(false)`: no join to run (every changed read's version that drives one is empty), the kept relation held
    /// as it is (quiet); `Some(true)`: run, its change its versions.
    fn maintain_mode(&mut self, scc: &BTreeSet<String>, rules: &[RuleKey], moved: &BTreeSet<String>, olds: BTreeMap<String, Arc<super::native::component::InRel>>, ins: &ModeInputs) -> Result<Option<bool>> {
        let vs = Arc::clone(&self.versions);
        let pf = vs.whole(self.version);
        let h = scc.iter().next().expect("a relation").clone();
        let ri: BTreeMap<String, usize> = scc.iter().enumerate().map(|(i, r)| (r.clone(), i)).collect();
        let native_rels = self.native_rels.clone();
        let types: BTreeMap<String, String> = scc.iter().map(|r| (r.clone(), pf.types(r).iter().map(|t| sym_char(&self.model.inputs, t)).collect())).collect();
        let bodies: Vec<Body<'_>> = rules.iter().map(|&r| vs.at(r)).collect();
        let mut wanted = BTreeSet::new();
        self.model.wants(bodies.iter().map(|b| (b, None)), &mut wanted);
        self.model.make_statistics(pf, &wanted)?;
        let held = |rel: &str| super::native::component::held(self.db, rel);
        let mut lines = Vec::new();
        // Each joined line's kept-plan key, by the engine's list it runs in (`ModeLine`), in spec order: its work goes
        // to the next run's planning budget (decision 0122, Q208).
        let mut mode_keys: Vec<(ModeLine, PlanKey)> = Vec::new();
        let mut input_held: Vec<String> = Vec::new();
        // Each relation starts from its kept rows, held under its own name when the stratum opens (below).
        for r in scc {
            lines.push(SpecLine::Rel { name: r.clone(), types: types[r].clone(), start: Start::Base { old: r.clone(), gone: "-".into(), seeded: false } });
        }
        lines.push(SpecLine::NoWrite);
        let recursive = bodies.iter().any(|b| (0..b.len()).any(|i| matches!(b.view(i), Lit::Atom(a) if scc.contains(b.rel(a)))));
        if recursive {
            // Its rows keep levels, and its kept rows are no wave's delta: the first wave's is what the removals put back.
            lines.push(SpecLine::Leveled((0..scc.len()).collect()));
            lines.push(SpecLine::KeptOld);
        }
        // The changed inputs' versions, given in place over their sources beside each input (`give_versions`) while
        // the stratum runs: its joins read `Version(r, …)`, as they read a derived relation's.
        for (r, ([o, mi, pl, now], cols)) in ins {
            let t: Vec<u8> = pf.types(r).iter().map(|t| sym_char(&self.model.inputs, t) as u8).collect();
            let given = [(Arc::clone(o), cols.clone()), (Arc::clone(mi), cols.clone()), (Arc::clone(pl), cols.clone()), (Arc::clone(now), cols.clone())];
            super::native::component::give_versions(self.db, r, t, given);
            input_held.push(r.clone());
        }
        let release = |given: &[String]| super::native::component::take_versions(self.db, given);
        for (bi, b) in bodies.iter().enumerate() {
            let head_i = ri[vs.head_at(rules[bi])];
            self.model.late(pf, b, None, false)?;
            // The items of `spec` made for literal `at` of the plan `order` (`JoinSpec::from`: its step).
            let lit_of = |spec: &JoinSpec, order: &[Step], item: usize| -> Option<usize> {
                let step = if spec.from.is_empty() { item } else { *spec.from.get(item)? as usize };
                order.get(step).map(|s| s.at)
            };
            // `spec` with each changed atom read in its version: `drive` at `k` (as `dver`, positive), the others `rest`.
            let versioned = |spec: &JoinSpec, order: &[Step], k: usize, dver: super::native::component::spec::Ver, rest: super::native::component::spec::Ver| -> JoinSpec {
                let mut out = spec.clone();
                for (i, item) in out.items.iter_mut().enumerate() {
                    let Some(at) = lit_of(spec, order, i) else { continue };
                    let src = match item {
                        JoinItem::Atom { neg, src, .. } => {
                            if at == k {
                                *neg = false;
                            }
                            src
                        }
                        // An aggregate reads its relation as the join's other atoms do (it never drives: `k` is an atom).
                        JoinItem::Count { src, .. } | JoinItem::Extreme { src, .. } | JoinItem::StrJoin { src, .. } => src,
                        _ => continue,
                    };
                    // A changed input read in its version already (`mode_reads`) is read as it is.
                    let JoinSource::Native(t) = src.clone() else { continue };
                    if at == k {
                        *src = JoinSource::Version(t, dver);
                    } else if moved.contains(&t) && rest != super::native::component::spec::Ver::New {
                        *src = JoinSource::Version(t, rest);
                    }
                }
                out
            };
            use super::native::component::spec::Ver as V;
            for k in 0..b.len() {
                let Lit::Atom(a) = b.view(k) else { continue };
                let rel = b.rel(a);
                if !moved.contains(rel) {
                    continue;
                }
                let (gone, added) = match ins.get(rel) {
                    Some(([_, mi, pl, _], _)) => (mi.len(), pl.len()),
                    None => {
                        let x = held(rel).expect("a changed read with a version");
                        (x.version_ids(super::native::component::Ver::Minus).map_or(0, |v| v.len()), x.version_ids(super::native::component::Ver::Plus).map_or(0, |v| v.len()))
                    }
                };
                // A negated atom: its rows added remove derivations, its rows gone add them (checked: a row may be back).
                let (remove_by, add_by) = if a.neg() { (V::Plus, V::Minus) } else { (V::Minus, V::Plus) };
                let (remove_n, add_n) = if a.neg() { (added, gone) } else { (gone, added) };
                // A positive driver is planned first, as a delta is; a negated one stays where its order tests it.
                let first = (!a.neg()).then_some(k);
                for (by, n, removing) in [(remove_by, remove_n, true), (add_by, add_n, false)] {
                    if n == 0 {
                        continue;
                    }
                    self.model.stats.delta_rows.insert(rel.to_owned(), n as f64);
                    // The program's kept plan of this line, as a semi-naive probe's (decision 0364 stage 4): searched
                    // on its first sight, then taken or priced again as its sizes move (decisions 0038, 0122).
                    plan_site("the maintain mode's join");
                    let ((order, _, _), _) = self.model.plan_rule_reused(pf, b, first)?;
                    let key = self.model.reused_key(b, first);
                    // The changed inputs' literals read their versions: the driver's, and as they were in a removal.
                    self.mode_reads.clear();
                    // An aggregate's atom too: read where the rule's other atoms are, as it was in a removal. Unnamed here, it
                    // read the relation's own versions, which a relation derived again past θ (R4) does not have:
                    // its delta is `r__minus`/`r__plus`, against its kept rows (`kept_delta`).
                    for i in 0..b.len() {
                        let (Lit::Atom(x) | Lit::Agg { atom: x, .. }) = b.view(i) else { continue };
                        let xr = b.rel(x);
                        if !ins.contains_key(xr) {
                            continue;
                        }
                        let ver = if i == k { by } else if removing { V::Old } else { continue };
                        self.mode_reads.insert(i, JoinSource::Version(xr.to_owned(), ver));
                    }
                    let made = self.whole_join_spec(b, &order, None, &ri, scc, &native_rels);
                    self.mode_reads.clear();
                    let Some(spec) = made else {
                        self.why_not = Some(format!("the maintain mode: no native join for a rule of {h}"));
                        return Ok(None);
                    };
                    let spec = Arc::new(versioned(&spec, &order, k, by, if removing { V::Old } else { V::New }));
                    let (line, kind) = match (removing, a.neg()) {
                        (true, _) => (SpecLine::Remove { head: head_i, join: spec }, ModeLine::Remove),
                        (false, false) => (SpecLine::Seed { head: head_i, join: spec }, ModeLine::Probe),
                        (false, true) => (SpecLine::AddChecked { head: head_i, join: spec }, ModeLine::Add),
                    };
                    lines.push(line);
                    mode_keys.push((kind, key));
                }
            }
            // An aggregate over a changed relation (as the forms' `__grp` relations): its groups whose key is in the
            // relation's minus or plus are the ones whose value may move. Per such delta, a removal join (the rule over
            // what was, its aggregate over the old rows) and an add join (the rule now), each led by an atom over that
            // delta with the aggregate's group slots (its bound arguments) and its own columns wild.
            for k in 0..b.len() {
                let Lit::Agg { atom: a, .. } = b.view(k) else { continue };
                let rel = b.rel(a);
                if !moved.contains(rel) {
                    continue;
                }
                let (gone, added) = match ins.get(rel) {
                    Some(([_, mi, pl, _], _)) => (mi.len(), pl.len()),
                    None => {
                        let x = held(rel).expect("a changed read with a version");
                        (x.version_ids(super::native::component::Ver::Minus).map_or(0, |v| v.len()), x.version_ids(super::native::component::Ver::Plus).map_or(0, |v| v.len()))
                    }
                };
                for (key_ver, n) in [(V::Minus, gone), (V::Plus, added)] {
                    if n == 0 {
                        continue;
                    }
                    for removing in [true, false] {
                        plan_site("the maintain mode's aggregate join");
                        let ((order, _, _), _) = self.model.plan_rule_reused(pf, b, None)?;
                        let key = self.model.reused_key(b, None);
                        self.mode_reads.clear();
                        // A changed input read in its versions, whole: what was in a removal, and in an add the
                        // aggregate's own atom now, so its columns are the delta atom's that leads the join.
                        for i in 0..b.len() {
                            let (Lit::Atom(x) | Lit::Agg { atom: x, .. }) = b.view(i) else { continue };
                            if !ins.contains_key(b.rel(x)) {
                                continue;
                            }
                            if removing {
                                self.mode_reads.insert(i, JoinSource::Version(b.rel(x).to_owned(), V::Old));
                            } else if i == k {
                                self.mode_reads.insert(i, JoinSource::Version(b.rel(x).to_owned(), V::New));
                            }
                        }
                        let made = self.whole_join_spec(b, &order, None, &ri, scc, &native_rels);
                        self.mode_reads.clear();
                        let Some(spec) = made else {
                            self.why_not = Some(format!("the maintain mode: no native aggregate join for a rule of {h}"));
                            return Ok(None);
                        };
                        let mut spec = versioned(&spec, &order, usize::MAX, V::New, if removing { V::Old } else { V::New });
                        let Some(at) = (0..spec.items.len()).find(|&i| lit_of(&spec, &order, i) == Some(k) && matches!(spec.items[i], JoinItem::Count { .. } | JoinItem::Extreme { .. } | JoinItem::StrJoin { .. })) else {
                            self.why_not = Some(format!("the maintain mode: no aggregate item for a rule of {h}"));
                            return Ok(None);
                        };
                        let (JoinItem::Count { args, .. } | JoinItem::Extreme { args, .. } | JoinItem::StrJoin { args, .. }) = &spec.items[at] else { unreachable!() };
                        let args: Vec<super::native::component::spec::Arg> = args.iter().map(|x| match x {
                            super::native::component::spec::Arg::Local(_) => super::native::component::spec::Arg::Tok(super::native::component::spec::Tok::Wild),
                            x => x.clone(),
                        }).collect();
                        let src = JoinSource::Version(rel.to_owned(), key_ver);
                        if !spec.from.is_empty() {
                            let step = spec.from[at];
                            spec.from.insert(0, step);
                        }
                        spec.items.insert(0, JoinItem::Atom { neg: false, src, args });
                        let spec = Arc::new(spec);
                        lines.push(if removing { SpecLine::Remove { head: head_i, join: spec } } else { SpecLine::Seed { head: head_i, join: spec } });
                        mode_keys.push((if removing { ModeLine::Remove } else { ModeLine::Probe }, key));
                    }
                }
            }
            // Its recheck: the rule run once per candidate row, its head's variables bound before its first step. The
            // planner prices its orders from that binding and one row, and keeps the order chosen under the recheck's
            // own key (`Body::with_head_bound`, Q208), taken or priced again as the other lines' are.
            // Its spec is written with the head's variables bound, as the order was priced (`join_spec_made`): a step
            // that runs one way only (`docof` from its node) reads its bound side. Where no native join is made, the
            // rule's own order: correct with the head bound too, its binds then tests (0364 R7 counts it).
            let bound = b.with_head_bound();
            self.model.late(pf, &bound, None, false)?;
            plan_site("the maintain mode's recheck");
            let priced = match self.model.plan_rule_reused(pf, &bound, None) {
                // The spec of the order as priced: its head's variables bound before its first step (`with_head_bound`).
                Ok(((order, _, _), _)) => self.whole_join_spec(&bound, &order, None, &ri, scc, &native_rels).map(|spec| (spec, self.model.reused_key(&bound, None))),
                Err(_) => None,
            };
            let (spec, key) = match priced {
                Some(got) => got,
                None => {
                    plan_site("the maintain mode's recheck, the rule's order");
                    // Not priced with its head bound (0364 R7 counts it).
                    crate::code_rule_dsl::logic::plan::corpus::maintain::shape::recheck_unpriced();
                    let ((order, _, _), _) = self.model.plan_rule_reused(pf, b, None)?;
                    let Some(spec) = self.whole_join_spec(b, &order, None, &ri, scc, &native_rels) else {
                        self.why_not = Some(format!("the maintain mode: no native recheck for a rule of {h}"));
                        return Ok(None);
                    };
                    (spec, self.model.reused_key(b, None))
                }
            };
            lines.push(SpecLine::Recheck { head: head_i, join: spec });
            mode_keys.push((ModeLine::Recheck, key));
        }
        let quiet = !lines.iter().any(|l| matches!(l, SpecLine::Remove { .. } | SpecLine::Seed { .. } | SpecLine::AddChecked { .. }));
        // A recursive rule's delta probes: the waves that carry the rows put back and added, and the drivers that find
        // the rows reading a row gone. Planned only when something changed, from the program's kept plans.
        for (bi, b) in bodies.iter().enumerate().filter(|_| !quiet) {
            let head_i = ri[vs.head_at(rules[bi])];
            for i in 0..b.len() {
                let Lit::Atom(a) = b.view(i) else { continue };
                if a.neg() || !scc.contains(b.rel(a)) {
                    continue;
                }
                self.model.late(pf, b, Some(i), false)?;
                plan_site("the maintain mode's probe");
                let ((order, _, _), _) = self.model.plan_rule_reused(pf, b, Some(i))?;
                let key = self.model.reused_key(b, Some(i));
                let at = order.iter().position(|st| st.at == i).expect("the delta atom");
                let Some(spec) = self.whole_join_spec(b, &order, Some(at), &ri, scc, &native_rels) else {
                    self.why_not = Some(format!("the maintain mode: no native probe for a rule of {h}"));
                    return Ok(None);
                };
                lines.push(SpecLine::Join { head: head_i, delta: ri[b.rel(a)], join: spec, whole: None });
                mode_keys.push((ModeLine::Probe, key));
            }
        }
        // Quiet: no join to run. The relation is its kept one, its statistics as kept (`quiet_card`).
        if quiet {
            release(&input_held);
            for (r, o) in olds {
                let card = quiet_card(&mut self.model, &self.quiet, pf, &r, &o);
                self.model.stats.card.insert(r.clone(), card);
                super::native::component::hold_as(self.db, &r, o);
                self.native_rels.insert(r);
            }
            self.quiet.strata.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            super::profile::stratum(&stratum_key(scc), "native set engine", || "quiet: the maintain mode has no join to run; the relation is its kept one".to_owned());
            return Ok(Some(false));
        }
        let name = format!("mm_{h}");
        let spec = ComponentSpec { name: name.clone(), lines };
        let fx = Fx { db: self.db, direct: true };
        for r in scc {
            super::native::component::hold_as(self.db, r, Arc::clone(&olds[r]));
        }
        fx.open(&spec)?;
        fx.run(&name, 0)?;
        for (r, live) in scc.iter().zip(fx.stats(&name, &|_, _| false)?) {
            live_stats(&mut self.model, r, live);
        }
        // Each line's work, under its kept plan's key, for the next run's planning budget (decision 0122): a removal
        // and its add, planned alike, under one key, their work summed (Q208). A recursive component's driver (its
        // probe's order less the delta atom, finding the rows that read a row gone) adds its work to its probe's.
        {
            let probes = super::native::component::step_counts(self.db, &name)?;
            let [removes, adds, rechecks] = super::native::component::maint_step_counts(self.db, &name)?;
            let drivers = super::native::component::driver_step_counts(self.db, &name)?;
            let work_of = |line: &[(i64, i64)]| super::profile::plan_work(&line.iter().map(|&(a, b)| (a as u64, b as u64)).collect::<Vec<_>>()) as f64;
            let mut lists: [std::vec::IntoIter<Vec<(i64, i64)>>; 4] = [probes.into_iter(), removes.into_iter(), adds.into_iter(), rechecks.into_iter()];
            let mut work: Vec<(PlanKey, f64)> = Vec::new();
            let mut probe = 0;
            for (kind, key) in &mode_keys {
                let Some(line) = lists[*kind as usize].next() else { continue };
                let mut w = work_of(&line);
                if *kind == ModeLine::Probe {
                    w += drivers.iter().filter(|(p, _)| *p == probe).map(|(_, d)| work_of(d)).sum::<f64>();
                    probe += 1;
                }
                match work.iter_mut().find(|(k, _)| k == key) {
                    Some((_, at)) => *at += w,
                    None => work.push((*key, w)),
                }
            }
            for (key, w) in work {
                Model::ran(key, w);
            }
        }
        fx.close(&name)?;
        release(&input_held);
        self.native_rels.extend(scc.iter().cloned());
        // Nothing deleted, nothing derived: quiet, as a stratum whose joins all read empty deltas is.
        if scc.iter().all(|r| super::native::component::held(self.db, r).and_then(|x| x.version_counts()) == Some((0, 0))) {
            self.quiet.strata.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        MODE_STRATA.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        super::profile::stratum(&stratum_key(scc), "native set engine", || "the maintain mode (decision 0364)".to_owned());
        Ok(Some(true))
    }

    fn native_stratum(&mut self, scc: &BTreeSet<String>, rules: &[RuleKey]) -> Result<Native> {
        // `RERAIL_STRATA_LIVE`: each native stratum as it starts and ends, so a stalled run names its stratum (the
        // trace, `RERAIL_STRATA_TRACE`, is written when the loop ends).
        static LIVE: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var_os("RERAIL_STRATA_LIVE").is_some());
        let t0 = LIVE.then(|| {
            eprintln!("RERAIL_STRATA_LIVE start {scc:?} ({} rules)", rules.len());
            std::time::Instant::now()
        });
        let solving = solving_of(&self.versions, scc, rules);
        let out = self.lent(solving, |this, native_rels| this.native_stratum_(scc, rules, native_rels));
        if let Some(t0) = t0 {
            eprintln!("RERAIL_STRATA_LIVE end {scc:?} {:.3} s", t0.elapsed().as_secs_f64());
        }
        out
    }

    /// Resumes a stratum that waited on its device wave, after the wave ran: its driver goes on from the run that
    /// suspended it.
    fn native_resume(&mut self, d: Box<Driving>) -> Result<Native> {
        let solving = d.solving.then(|| d.scc.clone());
        self.lent(solving, |this, native_rels| this.drive(d, native_rels))
    }

    /// `native_stratum`, finished here: a stratum that waits on its device wave has the wave flushed (with whatever
    /// else it holds, whose outputs stay for their strata) and is resumed until it ran. Whether it ran natively. For
    /// a stratum its caller needs at once (one derived again, 0090 §4).
    fn native_stratum_now(&mut self, scc: &BTreeSet<String>, rules: &[RuleKey]) -> Result<bool> {
        // Needed at once: its requests are answered in place (`answer`), not posted.
        let asking = self.asking.take();
        let got = self.native_stratum_now_(scc, rules);
        self.asking = asking;
        got
    }

    fn native_stratum_now_(&mut self, scc: &BTreeSet<String>, rules: &[RuleKey]) -> Result<bool> {
        let mut got = self.native_stratum(scc, rules)?;
        loop {
            match got {
                Native::Ran => return Ok(true),
                Native::Declined => return Ok(false),
                Native::Waiting(d) => {
                    super::lanes::wave::flush(1).map_err(Error::Plan)?;
                    got = self.native_resume(d)?;
                }
                Native::Asking(..) => return Err(Error::Plan(format!("{scc:?}: requests posted by a stratum needed at once"))),
            }
        }
    }

    /// Runs `f` with the relations run natively so far lent to it, and `solving` as the component being solved.
    fn lent<T>(&mut self, solving: Option<BTreeSet<String>>, f: impl FnOnce(&mut Self, &BTreeSet<String>) -> Result<T>) -> Result<T> {
        self.model.stats.solve(solving.unwrap_or_default());
        // The relations run natively so far, lent to the stratum (it reads them, and adds its own only at the end):
        // cloning them per stratum was quadratic in strata (3% of a combined run's main thread).
        let native_rels = std::mem::take(&mut self.native_rels);
        let out = f(self, &native_rels);
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
    fn native_stratum_(&mut self, scc: &BTreeSet<String>, rules: &[RuleKey], native_rels: &BTreeSet<String>) -> Result<Native> {
        let versions = Arc::clone(&self.versions);
        let pf = versions.whole(self.version);
        let vs = Arc::clone(&self.versions);
        let f = vs.whole(self.version);
        // The relations that reach themselves through the stratum's rules as written, before any is left out
        // (`SpecLine::Leveled`).
        let cyclic = cyclic_relations(&vs, scc, rules);
        self.why_not = None;
        self.why_class = None;
        // A lattice relation runs here as one row a key, folded by its lattice's join (decision 0124), except where
        // the component's other machinery keeps sets: a gate, or a request answered (its SQL seeds and probes insert
        // by key, not by join). A session's `levels` are kept as any relation's: a lattice stratum is not maintained
        // (`maintain::unmaintainable`), it is derived again each run.
        if let Some(why) = scc.iter().filter_map(|r| f.lattice_decl(r).map(|l| (r, l))).find_map(|(r, l)| {
            let lattice = match super::native::lattice(f.text(l.lattice)) {
                Ok(lattice) => lattice,
                Err(e) => return Some(format!("a lattice relation ({r}): {e}")),
            };
            if !lattice.finite() && l.widen.is_none() {
                return Some(format!("a lattice relation ({r}) of unbounded height without `widen`"));
            }
            if scc.iter().any(|g| f.gate(g) || f.request(g)) {
                return Some(format!("a lattice relation ({r}) in a component with gates or requests"));
            }
            None
        }) {
            self.why_not = Some(why);
            self.why_class = Some("a lattice relation".into());
            return Ok(Native::Declined);
        }
        // A text test over answered strings (a maintained form's `h__ins` among them): its rows from the run's
        // vocabulary where the engine holds its input. With the device on, its rule (the test row by row) runs as
        // placed, the device's text-test form.
        if let [rel] = scc.iter().collect::<Vec<_>>().as_slice()
            && let Some(super::program_image::NativeSpec::TextTest { input, column, op, patterns }) = f.native(rel).map(|n| &n.spec)
            && text_test_defined(&self.versions, rules, f.rel_name(*input))
        {
            if !placement::device_on() {
                let (input, op, patterns) = (f.rel_name(*input).to_owned(), f.text(*op).to_owned(), patterns.iter().map(|&p| f.text(p).to_owned()).collect::<Vec<_>>());
                if self.native_text_test(rel, &input, native_rels.contains(&input), *column, &op, &patterns)?.is_some() {
                    return Ok(Native::Ran);
                }
                self.why_not = Some("a text test whose input the engine does not hold".into());
                self.why_class = Some("native fact operator".into());
                return Ok(Native::Declined);
            }
        // A text test whose rules are not its definition (a maintained form's relation: its old rows and its new
        // ones) is those rules, run below.
        } else if scc.iter().any(|r| f.native(r).is_some_and(|n| !matches!(n.spec, super::program_image::NativeSpec::TextTest { .. }))) {
            self.why_not = Some("a native fact operator over input relations".into());
            self.why_class = Some("native fact operator".into());
            return Ok(Native::Declined);
        }
        let rels: Vec<&String> = scc.iter().collect();
        let ri: BTreeMap<String, usize> = rels.iter().enumerate().map(|(i, r)| ((*r).clone(), i)).collect();
        let name = format!("prog_{}", rels[0]);
        let mut gates: Vec<(&str, &super::program_image::GateRef)> = f.gates().iter().map(|g| (f.text(g.key), g)).filter(|(g, _)| scc.contains(*g)).collect();
        gates.sort_by(|a, b| (a.1.level, a.0).cmp(&(b.1.level, b.0)));
        // A gate's rule only placed it here.
        let rules: Vec<RuleKey> = rules.iter().copied().filter(|&r| !gates.iter().any(|(g, _)| *g == vs.head_at(r))).collect();
        // A levels stratum (decision 0090): per relation `h`, `h__chk` and `h__gone` are written by the engine's
        // driver; their rules only placed them here.
        let levels: Vec<[String; 5]> = scc.iter().filter_map(|r| r.strip_suffix("__cand")).filter_map(|h| levels_of(h, scc)).collect();
        let placed: BTreeSet<&str> = levels.iter().flat_map(|l| [l[1].as_str(), l[3].as_str()]).collect();
        let rules: Vec<RuleKey> = rules.into_iter().filter(|&r| !placed.contains(vs.head_at(r))).collect();
        // A maintained stratum's survivors (`h :- h__old, !h__del`, decision 0077): the relation starts as the kept
        // `h__old` with `h__del`'s rows dead in place (decision 0090 step 4), not derived row by row.
        // `RERAIL_DRED_COPY`: derive them by the rule (to measure what starting in place saves).
        let based: BTreeMap<String, (String, String)> = if std::env::var_os("RERAIL_DRED_COPY").is_some() {
            BTreeMap::new()
        } else {
            rules.iter().filter_map(|&r| survivors(&vs.at(r), scc, native_rels).map(|od| (vs.head_at(r).to_owned(), od))).collect()
        };
        let rules: Vec<RuleKey> = rules.into_iter().filter(|&r| !(based.contains_key(vs.head_at(r)) && survivors(&vs.at(r), scc, native_rels).is_some())).collect();
        // A rule that joins a relation of a lower stratum holding no rows derives nothing, so it is neither planned
        // nor given to the engine. In a maintained run most delta rules read a delta that is empty this time, and
        // planning, keying and specifying them was work in the program's size, not the delta's (0090 step 5,
        // 0094 §4). Only a positive atom counts: a negation of it holds, and an aggregate over no rows is a row.
        // A relation a request may fill during the stratum is never taken as empty. `RERAIL_KEEP_EMPTY_JOINS` runs
        // them all (to measure what leaving them out saves).
        let rules: Vec<RuleKey> = if std::env::var_os("RERAIL_KEEP_EMPTY_JOINS").is_some() { rules } else {
            let fillable: BTreeSet<&str> = f.answers().iter().map(|&(_, p)| f.rel_name(p)).flat_map(|p| [Some(p), f.request_of(p)]).flatten().collect();
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
            let kept: Vec<RuleKey> = rules.into_iter().filter(|&r| {
                let (v, rule) = vs.rule_at(r);
                !rule.body().any(|l| matches!(l.view(), Lit::Atom(a) if !a.neg() && is_empty(v.rel_name(a.rel()))))
            }).collect();
            if before > kept.len() {
                super::profile::decision("rules over empty relations", || format!("{} of {before} rules not run: each joins a relation of a lower stratum that has no rows", before - kept.len()));
            }
            // The same inside the component: a relation of it that starts empty and that nothing fills (no rule left
            // derives it; no facts, base, request, answer, level or gate writes it) stays empty, so a rule joining it
            // derives nothing either; until none is left out. In a maintained run `pcall__ins`'s rules all joined
            // empty input deltas, and the probes of `pfn__ins :- pcall__ins(…), pv(…), fv(…)` by `pv`'s and `fv`'s
            // deltas read 341,837 rows each before the empty `pcall__ins` rejected them (O125 step 06).
            let written: BTreeSet<&str> = levels.iter().flatten().map(String::as_str)
                .chain(gates.iter().map(|(g, _)| *g))
                .chain(based.keys().map(String::as_str))
                .chain(fillable.iter().copied())
                .chain(scc.iter().map(String::as_str).filter(|r| f.has_facts(r)))
                .collect();
            let mut kept = kept;
            loop {
                let heads: BTreeSet<&str> = kept.iter().map(|&r| vs.head_at(r)).collect();
                let unfilled = |rel: &str| scc.contains(rel) && !heads.contains(rel) && !written.contains(rel);
                let n = kept.len();
                kept.retain(|&r| {
                    let (v, rule) = vs.rule_at(r);
                    !rule.body().any(|l| matches!(l.view(), Lit::Atom(a) if !a.neg() && unfilled(v.rel_name(a.rel()))))
                });
                if kept.len() == n { break; }
                super::profile::decision("rules over empty relations", || format!("{} rules not run: each joins a relation of its component that starts empty and that nothing fills", n - kept.len()));
            }
            kept
        };
        let typed: HashMap<String, String> = rels.iter().map(|r| ((*r).clone(), f.types(r).iter().map(|t| sym_char(&self.model.inputs, t)).collect::<String>())).collect();
        let types = |r: &str| typed[r].clone();
        // Nothing left to run (every rule joined an empty relation, and no relation of the stratum has facts): the
        // stratum's relations are held with no rows, and no component is specified or opened. A relation with facts
        // (`d("a"). d(y) :- d(x), e(x, y).` with `e` empty) holds its facts, which the component seeds.
        // `RERAIL_KEEP_EMPTY_JOINS` opens it as before.
        let asks = scc.iter().any(|r| f.request(r) || f.produced(r));
        let facts = scc.iter().any(|r| f.has_facts(r));
        if rules.is_empty() && based.is_empty() && gates.is_empty() && levels.is_empty() && !asks && !facts && std::env::var_os("RERAIL_KEEP_EMPTY_JOINS").is_none() {
            for r in &rels {
                super::native::component::hold_empty(self.db, r, types(r).into_bytes());
            }
            super::profile::stratum(&stratum_key(scc), "native set engine", || "every rule joins an empty relation: held empty, no component opened".to_owned());
            self.native_rels.extend(scc.iter().cloned());
            return Ok(Native::Ran);
        }
        // A copy of a bound input's columns (Q37: `kind(n, k) :- node(_, n, k, _, _)`, 3.5 M rows a cold run of
        // hogql): its relation is the input's columns read once into flat rows (`hold_view`), and no component
        // copies it. `RERAIL_COPY_VIEWS` derives it as before (to measure what the view saves).
        if let ([rel], [r]) = (rels.as_slice(), rules.as_slice())
            && based.is_empty() && gates.is_empty() && levels.is_empty() && !asks && !facts
            && let Some((base, src, cols)) = self.input_view(*r)
        {
            // Held in place where the source answers the view's probes and counts its columns itself (a batch's
            // `node`, decision 0329): no row read. Else read once into flat rows (`hold_view`). Either way its
            // statistics are a copy's, exact or sketched: its readers plan as they did when a stratum copied it.
            let (form, live) = match self.hold_view_in_place(rel, &src, &cols, types(rel).into_bytes()) {
                Some(live) => ("held in place", live),
                None => {
                    let _span = super::profile::span(|| "a view of an input's columns: its rows".into());
                    let l = super::native::component::hold_view(self.db, rel, src, cols.clone(), types(rel).into_bytes());
                    ("read into flat rows", LiveRel { n: l.n, delta: l.delta, ndv: l.ndv, sq: l.sq, freq: l.freq.into_iter().map(|(c, v)| (c, v.into_iter().map(|(v, m)| (Constant::Int(v), m)).collect())).collect() })
                }
            };
            live_stats(&mut self.model, rel, live);
            INPUT_VIEWS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            super::profile::stratum(&stratum_key(scc), "native set engine", || format!("a view of {base}'s columns {cols:?}: {form}, no component opened"));
            self.native_rels.extend(scc.iter().cloned());
            return Ok(Native::Ran);
        }
        // A quiet stratum (decision 0101 §1): only survivors are left, and each `h__del` is empty. Each `h` is then
        // its kept `h__old`, the same relation with its indexes; nothing is specified, planned or opened. A kept
        // relation whose rows are partly in a table (`stored`) is opened as before, so its readers see them all.
        // `RERAIL_KEEP_QUIET_STRATA` opens it as before (to measure what the skip saves).
        if rules.is_empty() && !based.is_empty() && gates.is_empty() && !asks && std::env::var_os("RERAIL_KEEP_QUIET_STRATA").is_none() {
            let held = |rel: &str| super::native::component::held(self.db, rel);
            let quiet: Option<Vec<(&String, Arc<super::native::component::InRel>)>> = rels.iter().map(|r| match based.get(*r) {
                Some((old, gone)) => {
                    // The rows gone are a native relation's, an input delta's, or a table's that a SQL stratum wrote.
                    let gone_empty = match held(gone) {
                        Some(g) => g.rows() == 0,
                        None if self.inputs.contains(gone.as_str()) => crate::code_rule_dsl::facts::vtab::bound_source(self.db, gone).is_some_and(|s| s.len() == 0),
                        None => self.db.query_row(&format!("SELECT NOT EXISTS (SELECT 1 FROM {})", super::sql::table(gone)), [], |row| row.get::<_, bool>(0)).unwrap_or(false),
                    };
                    let unseeded = !f.has_facts(r);
                    held(old).filter(|o| gone_empty && unseeded && o.none_stored()).map(|o| (*r, o))
                }
                None => None,
            }).collect();
            if let Some(quiet) = quiet {
                for (r, o) in quiet {
                    // Its readers are planned from its rows, as an opened stratum's would be (`prior_stats`).
                    let card = quiet_card(&mut self.model, &self.quiet, pf, r, &o);
                    self.model.stats.card.insert(r.clone(), card);
                    super::native::component::hold_as(self.db, r, o);
                }
                self.quiet.strata.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                super::profile::stratum(&stratum_key(scc), "native set engine", || "quiet: nothing deleted, nothing to derive; each relation is its kept one".to_owned());
                self.native_rels.extend(scc.iter().cloned());
                return Ok(Native::Ran);
            }
        }
        // The statistics its plans read are requested in two rounds, each made in one batch before its plans
        // (decision 0123): its seeds', then, once its relations' rows are known, its probes' and whole plans'.
        // The component's spec: its relations, and each rule planned and written as a native join (`spec`'s own CPU
        // is the joins' specs and their checks; planning is in its `plan #…` spans).
        let spec_span = super::profile::span(|| "spec".into());
        let mut lines: Vec<SpecLine> = Vec::new();
        // The relations' lines go here, once the seeds SQL derives are known (below).
        let rel_at = lines.len();
        lines.push(SpecLine::NoWrite);
        // Its rows keep levels where a session maintains it by them (decision 0090, the `levels` qualifier).
        if scc.iter().any(|r| f.qualified(r, "levels")) {
            lines.push(SpecLine::Leveled(rels.iter().enumerate().filter(|(_, r)| cyclic.contains(r.as_str())).map(|(i, _)| i).collect()));
        }
        // A component that answers requests (decision 0065 §2a) cannot fall back to SQL whole: a rule with no native
        // step is a SQL seed, run before the component opens, or a SQL probe over its relations' `set_rel` views.
        let answering = scc.iter().any(|r| f.request(r));
        let views: HashMap<String, (String, String)> = rels.iter().map(|r| ((*r).clone(), (format!("nsx_f_{r}"), format!("nsx_d_{r}")))).collect();
        let mut sql_seeded: BTreeSet<String> = BTreeSet::new();
        let mut sql_probe: Vec<bool> = Vec::new();
        // Requests and what answers them (decision 0065 §2a): the run pauses for the first, and adds to the second.
        for r in &rels {
            if f.request(r) && scc.iter().any(|x| f.request_of(x) == Some(r.as_str())) {
                lines.push(SpecLine::Request(ri[*r]));
            }
            if f.produced(r) {
                lines.push(SpecLine::Produced(ri[*r]));
            }
            // Stored for a session's next run (decision 0099): its statistics kept as its rows come, so that run
            // does not count them from its rows (decision 0096 step 4).
            if f.qualified(r, "no_inline") {
                lines.push(SpecLine::Stats(ri[*r]));
            }
        }
        let mut est: HashMap<&str, f64> = HashMap::new();
        for (h, (old, _)) in &based {
            let n = super::native::component::held(self.db, old).map_or(0, |o| o.rows());
            *est.entry(h.as_str()).or_default() += n as f64;
        }
        let mut orders = Vec::new();
        // Each line's kept-plan key (`Driving::keys`).
        let mut keys: Vec<Option<PlanKey>> = Vec::new();
        // Each order's rule, for the fanouts its runs observe (`obs_rule`).
        let mut order_rules: Vec<u64> = Vec::new();
        // `RERAIL_WAVE0=changes` (a spike): a maintained component's first wave is driven by its input changes, not by
        // its kept rows. A rule that reads a change of a lower stratum (`x__plus`, `x__gone`, …) also runs once as a
        // seed, and the kept rows start as no wave's delta (`native::setfix::wave0_changes`).
        let wave0 = !based.is_empty() && super::native::wave0_changes();
        if wave0 {
            lines.push(SpecLine::KeptOld);
        }
        // Only a rule whose relations of the component all start from kept rows: one that reads a relation starting
        // empty (`walk__ins`) derives nothing in the first wave, and its delta's probe takes its rows as they come.
        let from_kept = |b: &Body<'_>| recursive(b, scc).iter().all(|(_, a)| based.contains_key(*a));
        let seeds: Vec<RuleKey> = rules.iter().copied().filter(|&r| {
            let b = vs.at(r);
            recursive(&b, scc).is_empty() || (wave0 && reads_change(&b, scc) && from_kept(&b))
        }).collect();
        // A rule's planning, as a span named by its number and body size (`plan #0009 violation, 15 literals`).
        let rule_no = |r: RuleKey| r.number();
        let planning = |r: RuleKey, what: &str| {
            super::profile::span(|| {
                let no = rule_no(r).map_or("?".to_owned(), |k| format!("{k:04}"));
                let (v, rule) = vs.rule_at(r);
                format!("{what} #{no} {}, {} literals", v.rel_name(rule.head().rel()), rule.body().count())
            })
        };
        let planning_key = |k: body::RuleKey, what: &str| {
            super::profile::span(|| {
                let b = vs.at(k);
                let no = k.number().map_or("?".to_owned(), |k| format!("{k:04}"));
                format!("{what} #{no} {}, {} literals", b.rel(b.head()), b.len())
            })
        };
        let profiling = super::profile::on();
        // Per seed, then per probe: (kind, rule, plan as EXPLAIN shows it, planning CPU, replans).
        let mut explained: Vec<(String, Option<usize>, Option<super::profile::RulePlan>, f64, u64)> = Vec::new();
        // Its shape and planned work, for where it runs (`placement`, decision 0095).
        let mut shape = placement::Shape::default();
        // The first step of `order` without a native step (EXPLAIN's reason the engine declined).
        // Prefixes are probed with a head of no terms: the rule's own head names variables a prefix has not bound yet,
        // so every prefix would fail at its first step. `None`: every step has one, and the head is what has none.
        let first_missing = |this: &Self, b: &Body<'_>, steps: &[Step], at: Option<usize>| -> Option<usize> {
            (1..=steps.len()).find(|&k| this.join_spec(b, &steps[..k], true, at.filter(|&a| a < k), &ri, scc, &native_rels).is_none())
        };
        let missing = |this: &Self, b: &Body<'_>, steps: &[Step], at: Option<usize>| -> String {
            match first_missing(this, b, steps, at) {
                Some(k) => {
                    let s = &steps[k - 1];
                    format!("{}{} has no native step", b.show(s.at), if s.mode == Some(Mode::Range) { " (a range read inside a document)" } else { "" })
                }
                None => format!("the head {} has no native form", b.head_text()),
            }
        };
        // The same step's kind (`decline_class`).
        let missing_class = |this: &Self, b: &Body<'_>, steps: &[Step], at: Option<usize>| -> String {
            match first_missing(this, b, steps, at) {
                Some(k) if steps[k - 1].mode == Some(Mode::Range) => "a range read inside a document".to_owned(),
                Some(k) => decline_class(b, steps[k - 1].at, f, native_rels),
                None => "a head term".to_owned(),
            }
        };
        // A seed that reads the component's own relations (`wave0`'s) prices them by their kept rows' statistics, as
        // the probes do: unpriced, each read of one was a scan of the component, and the search took cross products.
        if wave0 {
            for r in &rels {
                if let Some(card) = prior_stats(&mut self.model, pf, r) {
                    self.model.stats.card.insert((*r).clone(), card);
                }
            }
        }
        // Round 1 (decision 0123): the seeds' statistics, those that will be priced.
        let seed_bodies: Vec<Body<'_>> = seeds.iter().map(|&r| vs.at(r)).collect();
        let mut wanted = BTreeSet::new();
        {
            let _span = super::profile::span(|| "statistics requested".into());
            self.model.wants(seed_bodies.iter().map(|b| (b, None)), &mut wanted);
            self.model.make_statistics(pf, &wanted)?;
        }
        self.heavy_for(seeds.iter().copied(), false, None);
        plan_site("a stratum's seed");
        let mut ahead = self.model.plan_ahead(pf, &seed_bodies.iter().map(|b| (b, None)).collect::<Vec<_>>());
        for (&r, b) in seeds.iter().zip(&seed_bodies) {
            let head = vs.head_at(r);
            self.model.late(pf, b, None, false)?;
            let span = planning(r, "plan");
            let t0 = if profiling { super::profile::now() } else { 0.0 };
            let ((order, work, rows), x) = match ahead.remove(&line_id(b, None)) {
                Some(got) => got,
                None => self.model.plan_rule_reused(pf, b, None)?,
            };
            let took = if profiling { super::profile::now() - t0 } else { 0.0 };
            drop(span);
            shape.add(Order::new(b, &order), work, false, false);
            shape.seeded(head, rows);
            if let Some(x) = &x {
                shape.add_joins(Order::new(b, &order), x, |rel| self.model.stats.card.get(rel).copied().unwrap_or(0.0), |rel, col| self.model.stats.lookup_cost(rel, col), |rel| !self.model.derived.contains(rel));
            }
            let Some(mut spec) = self.whole_join_spec(b, &order, None, &ri, scc, &native_rels) else {
                if answering {
                    let reads: Vec<String> = (0..b.len()).filter_map(|i| b.atom(i)).map(|a| b.rel(a)).filter(|a| !scc.contains(*a)).map(str::to_owned).collect();
                    self.materialize(&reads)?;
                    self.ensure_indexes(Order::new(b, &order))?;
                    let select = self.model.sql_select(pf, b, &order, None, None)?;
                    self.insert_rows(head, &select)?;
                    sql_seeded.insert(head.to_owned());
                    continue;
                }
                self.why_not = Some(format!("rule #{:04}: {}", rule_no(r).unwrap_or(0), missing(self, b, &order, None)));
                self.why_class = Some(missing_class(self, b, &order, None));
                return Ok(Native::Declined);
            };
            if let Some(x) = &x {
                expect_axis_lookups(self.db, x);
                std::sync::Arc::make_mut(&mut spec).est = step_estimates(x);
            }
            explained.push((format!("seed {head}"), rule_no(r), x, took, 0));
            keys.push(Some(self.model.reused_key(b, None)));
            *est.entry(head).or_default() += rows;
            lines.push(SpecLine::Seed { head: ri[head], join: spec });
            orders.push((r, order));
            order_rules.push(b.observed_key());
        }
        // A stratum's relations were made empty by `run` unless the program states facts of them or a SQL seed derived
        // some. A produced input (decision 0065 §2a) starts from its rows where it is.
        let mut rel_lines = Vec::new();
        for r in &rels {
            if f.produced(r) && self.inputs.contains(r.as_str()) {
                let cols = (0..f.width(r)).map(|c| self.model.col(r, c)).collect::<Vec<_>>().join(", ");
                let sql = format!("SELECT {cols} FROM {}", self.model.relation(pf, r, false).render());
                rel_lines.push(SpecLine::Rel { name: (*r).clone(), types: types(r), start: Start::From(sql) });
            } else if let Some((old, gone)) = based.get(*r) {
                // Its survivors rule is gone: it starts from the kept relation, plus what facts or SQL seeds wrote.
                let seeded = !(!f.has_facts(r) && !sql_seeded.contains(*r));
                rel_lines.push(SpecLine::Rel { name: (*r).clone(), types: types(r), start: Start::Base { old: old.clone(), gone: gone.clone(), seeded } });
            } else {
                let empty = !f.has_facts(r) && !sql_seeded.contains(*r);
                rel_lines.push(SpecLine::Rel { name: (*r).clone(), types: types(r), start: if empty { Start::Empty } else { Start::Rows } });
            }
        }
        // A lattice relation's key and join, after the relations (decision 0124). One that starts from a kept
        // relation (a maintained session's survivors) is a set there: it is not folded in place yet.
        for r in &rels {
            let Some(l) = f.lattice_decl(r) else { continue };
            if based.contains_key(*r) {
                self.why_not = Some(format!("a lattice relation ({r}) maintained from its kept rows"));
                self.why_class = Some("a lattice relation".into());
                return Ok(Native::Declined);
            }
            rel_lines.push(SpecLine::Lattice { rel: ri[*r], keys: l.keys, name: f.text(l.lattice).to_owned(), widen: l.widen });
        }
        // The relations the run keeps past it (a chunk's exports): in blocks of their own, so a reader that holds
        // them where they lie does not keep the run's other relations' blocks.
        let kept_rows: Vec<usize> = rels.iter().filter(|r| self.model.inputs.kept.contains(r.as_str())).map(|r| ri[*r]).collect();
        if !kept_rows.is_empty() {
            rel_lines.push(SpecLine::KeptRows(kept_rows));
        }
        // The relations declared sorted (decision 0456 K7): laid out in canonical order when done.
        let sorted: Vec<usize> = rels.iter().filter(|r| f.qualified(r, "sorted")).map(|r| ri[*r]).collect();
        if !sorted.is_empty() {
            rel_lines.push(SpecLine::Sorted(sorted));
        }
        lines.splice(rel_at..rel_at, rel_lines);
        let nseeds = orders.len();
        for r in &rels {
            let seeded = est.get(r.as_str()).copied().unwrap_or(0.0).max(1.0);
            let card = prior_stats(&mut self.model, pf, r).map_or(seeded, |n| n.max(seeded));
            self.model.stats.card.insert((*r).clone(), card);
        }
        // Round 2 (decision 0123): the probes' statistics, and the whole plans' where one is made (below).
        let whole_wanted = |this: &Self, a: &str| !(wave0 && based.contains_key(a) || plan_budget() && this.model.card(a) < SEARCH_WORK);
        // Decision 0123 §9: a probe with no kept plan is planned when its delta first has rows, not here. Not where a
        // closure may be the component's form (one relation: `closure::recognize` reads its probe at open), nor where
        // the probes run in SQL, nor a probe with a whole plan (it runs over the relation's first delta).
        // The probes so left, with their keys: decided once.
        let rule_bodies: Vec<Body<'_>> = rules.iter().map(|&r| vs.at(r)).collect();
        let mut lazy: HashMap<(RuleKey, usize), PlanKey> = HashMap::new();
        if lazy_probes() && !answering && rels.len() > 1 {
            for (&r, b) in rules.iter().zip(&rule_bodies) {
                for (ai, a) in recursive(b, scc) {
                    if !whole_wanted(self, a) && let Some(key) = self.model.first_sight(b, Some(ai)) {
                        lazy.insert((r, ai), key);
                    }
                }
            }
        }
        let mut wanted = BTreeSet::new();
        let round: Vec<(&Body<'_>, Option<usize>)> = rules.iter().zip(&rule_bodies).flat_map(|(&r, b)| {
            let rec = recursive(b, scc);
            let whole = rec.iter().any(|(_, a)| whole_wanted(self, a));
            rec.into_iter().filter(|(ai, _)| !lazy.contains_key(&(r, *ai))).map(|(ai, _)| (b, Some(ai))).chain(whole.then_some((b, None))).collect::<Vec<_>>()
        }).collect();
        {
            let _span = super::profile::span(|| "statistics requested".into());
            self.model.wants(round.iter().copied(), &mut wanted);
            self.model.make_statistics(pf, &wanted)?;
        }
        self.heavy_for(rules.iter().copied(), false, None);
        let mut ahead = self.model.plan_ahead(pf, &round);
        let mut probes = Vec::new();
        // Each probe's whole plan's order, where it has one (O168: a whole plan cut is handed off as a probe is).
        let mut wholes: HashMap<usize, Vec<Step>> = HashMap::new();
        let mut unplanned: Vec<bool> = Vec::new();
        for (&r, b) in rules.iter().zip(&rule_bodies) {
            let head = vs.head_at(r);
            // The rule's whole plan, and its spec once one is wanted: one per rule, whichever atom is its delta.
            let mut whole_plan: Option<Vec<Step>> = None;
            let mut whole_spec: Option<Option<Arc<JoinSpec>>> = None;
            for (ai, a) in recursive(b, scc) {
                if let Some(key) = lazy.remove(&(r, ai)) {
                    explained.push((format!("join {head} (delta {a})"), rule_no(r), None, 0.0, 0));
                    lines.push(SpecLine::Unplanned { head: ri[head], delta: ri[a] });
                    keys.push(Some(key));
                    probes.push((r, ai, Vec::new()));
                    sql_probe.push(false);
                    unplanned.push(true);
                    LAZY[0].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    continue;
                }
                unplanned.resize(probes.len() + 1, false);
                self.model.late(pf, b, Some(ai), false)?;
                plan_site("a stratum's probe");
                let _span = planning(r, "plan");
                let t0 = if profiling { super::profile::now() } else { 0.0 };
                let ((order, work, rows), x) = match ahead.remove(&line_id(b, Some(ai))) {
                    Some(got) => got,
                    None => self.model.plan_rule_reused(pf, b, Some(ai))?,
                };
                let key = self.model.reused_key(b, Some(ai));
                let took = if profiling { super::profile::now() - t0 } else { 0.0 };
                // Its delta as the plan priced it (`atom_rows`: the relation's rows, at most its last delta's).
                let delta = self.model.stats.delta_rows.get(a).copied().map_or(self.model.card(a), |d| d.min(self.model.card(a)));
                shape.add(Order::new(b, &order), 0.0, true, pf.lattice_name(head) == Some("shortlex_min"));
                shape.probe(a, work, rows, delta);
                if let Some(x) = &x {
                    shape.add_joins(Order::new(b, &order), x, |rel| self.model.stats.card.get(rel).copied().unwrap_or(0.0), |rel, col| self.model.stats.lookup_cost(rel, col), |rel| !self.model.derived.contains(rel));
                }
                let at = order.iter().position(|s| s.at == ai).expect("the delta atom");
                let Some(mut spec) = self.whole_join_spec(b, &order, Some(at), &ri, scc, &native_rels) else {
                    if answering {
                        let reads: Vec<String> = (0..b.len()).filter_map(|i| b.atom(i)).map(|a| b.rel(a)).filter(|a| !scc.contains(*a)).map(str::to_owned).collect();
                        // SQL reads the views too (`native_stratum` made only those no native join reads in place).
                        self.ensure(&reads)?;
                        self.materialize(&reads)?;
                        // The component's own relations are read through its views; only the others take indexes.
                        let outside: Vec<Step> = order.iter().filter(|st| b.atom(st.at).is_none_or(|a| !scc.contains(b.rel(a)))).copied().collect();
                        self.ensure_indexes(Order::new(b, &outside))?;
                        let sql = self.model.sql_select(pf, b, &order, Some(at), Some(&views))?;
                        explained.push((format!("probe {head} (delta {a})"), rule_no(r), None, took, 0));
                        lines.push(SpecLine::Probe { head: ri[head], delta: ri[a], sql });
                        keys.push(None);
                        probes.push((r, ai, order));
                        sql_probe.push(true);
                        continue;
                    }
                    self.why_not = Some(format!("rule #{:04}: {}", rule_no(r).unwrap_or(0), missing(self, b, &order, Some(at))));
                    self.why_class = Some(missing_class(self, b, &order, Some(at)));
                    return Ok(Native::Declined);
                };
                let x = x.map(|x| delta_sources(x, Order::new(b, &order), at, scc));
                if let Some(x) = &x {
                    expect_axis_lookups(self.db, x);
                    std::sync::Arc::make_mut(&mut spec).est = step_estimates(x);
                }
                explained.push((format!("join {head} (delta {a})"), rule_no(r), x, took, 0));
                // The wave whose delta is all of the relation (it was empty before): any order, the atom read in full.
                // Under `wave0` a relation started from kept rows never has that wave (its kept rows are no delta).
                // Nor, under the planning budget (decision 0122), one whose relation is expected to start with fewer
                // rows than a search costs: the probe's own order reads them.
                if !whole_wanted(self, a) {
                    lines.push(SpecLine::Join { head: ri[head], delta: ri[a], join: spec, whole: None });
                    keys.push(Some(key));
                    probes.push((r, ai, order));
                    sql_probe.push(false);
                    continue;
                }
                if whole_plan.is_none() {
                    plan_site("a stratum's whole plan");
                    self.model.late(pf, b, None, false)?;
                    whole_plan = Some(match ahead.remove(&line_id(b, None)) {
                        Some(got) => got.0.0,
                        None => self.model.plan_rule_reused(pf, b, None)?.0.0,
                    });
                }
                let whole = whole_plan.as_ref().expect("planned");
                let wspec = if whole[0].at != ai && !small_batches(a) {
                    whole_spec.get_or_insert_with(|| self.whole_join_spec(b, whole, None, &ri, scc, &native_rels)).clone()
                } else {
                    None
                };
                if wspec.is_some() { wholes.insert(probes.len(), whole.clone()); }
                lines.push(SpecLine::Join { head: ri[head], delta: ri[a], join: spec, whole: wspec });
                keys.push(Some(key));
                probes.push((r, ai, order));
                sql_probe.push(false);
            }
        }
        for l in &levels {
            lines.push(SpecLine::Levels { cand: ri[&l[0]], chk: ri[&l[1]], held: ri[&l[2]], gone: ri[&l[3]], live: ri[&l[4]] });
        }
        for (g, d) in &gates {
            let quiet: Vec<usize> = d.quiet.iter().filter_map(|&r| ri.get(f.rel_name(r))).copied().collect();
            let after: Vec<usize> = d.deps.iter().filter_map(|&x| gates.iter().position(|(g2, _)| *g2 == f.rel_name(x))).collect();
            let src = d.src.map(|s| f.text(s));
            if let Some(src) = src.filter(|s| !ri.contains_key(*s)) {
                return Err(Error::Plan(format!("gate {g}: its source {src} is not in its component {scc:?}")));
            }
            lines.push(SpecLine::Gate { done: ri[*g], src: src.map(|s| ri[s]), quiet, after });
        }
        let mut spec = ComponentSpec { name: name.clone(), lines };
        drop(spec_span);
        // The O125 spike (`RERAIL_SPIKE_SPECS=<dir>`): each stratum named as it starts, its spec written out.
        if let Some(dir) = std::env::var_os("RERAIL_SPIKE_SPECS") {
            let cpu = { let (u, s) = corpus::thread_usage(); u + s };
            eprintln!("O125 stratum {name} ({} relations, {} rules) at {:.1} s CPU", rels.len(), rules.len(), cpu);
            let _ = std::fs::write(std::path::Path::new(&dir).join(format!("{name}.spec")), format!("{}\n\n{}", rules.iter().map(|&r| { let (v, rule) = self.versions.rule_at(r); v.show_rule(rule) }).collect::<Vec<_>>().join("\n"), spec.to_text()));
        }
        let started = std::time::Instant::now();
        // Where it runs (decision 0095 §7, §8). A stratum the placement puts on the device says so in its spec
        // (`device`): its seed joins run there, in one submission, and a recursive one's rounds to its fixpoint
        // (`device::fixpoint`), unless the device refuses one of them, which the profile then names. Gates and
        // requests are forms the device lacks.
        shape.close_rounds();
        // An input whose view a device stratum of this run made already is found resident: no view made for it.
        for r in &mut shape.reads {
            r.3 &= !self.device_views.contains(&r.0);
        }
        let placed = placement::place(&shape);
        if placed.gpu {
            self.device_views.extend(shape.reads.iter().filter(|r| r.3).map(|r| r.0.clone()));
        }
        // Placed there but a shape the device's run does not take: named, as a refusal.
        let not_taken = if !placed.gpu {
            None
        } else if sql_probe.iter().any(|p| *p) {
            Some("a recursive component with a SQL probe")
        } else if !gates.is_empty() || answering {
            Some("a component with gates or requests")
        } else if orders.is_empty() {
            Some("no native seed join")
        } else if placement::device_joins_off() {
            Some("device joins off (RERAIL_DEVICE_JOINS=off, a measurement aid)")
        } else {
            // A probe not planned yet is planned now: the device compiles every probe at open.
            let mut planned = Vec::new();
            let due: Vec<usize> = (0..probes.len()).filter(|&k| unplanned[k]).collect();
            for k in due {
                let (r, ai) = (probes[k].0, probes[k].1);
                let (order, spec, x) = self.plan_probe_now(r, ai, &ri, scc, native_rels, &planning_key)?;
                explained[nseeds + k].2 = x;
                probes[k].2 = order;
                unplanned[k] = false;
                let b = vs.at(r);
                planned.push((ri[b.rel(b.head())], ri[b.rel(b.atom(ai).expect("a delta atom"))], spec));
            }
            let mut planned = planned.into_iter();
            for l in &mut spec.lines {
                if let SpecLine::Unplanned { head, delta } = l {
                    let (h, d, join) = planned.next().expect("a plan per probe not planned");
                    debug_assert_eq!((*head, *delta), (h, d));
                    *l = SpecLine::Join { head: h, delta: d, join, whole: None };
                }
            }
            spec.lines.insert(0, SpecLine::Device);
            None
        };
        let cpu_at = if profiling { let (u, s) = corpus::thread_usage(); u + s } else { 0.0 };
        let counted: Vec<Vec<(u64, u64)>> = Vec::new();
        // The relational lab (decision 0095, step 0): the stratum's sources and seeds kept to run it again alone.
        let lab = super::native::component::lab::wanted(&name, shape.work)
            .and_then(|path| super::native::component::lab::capture_inputs(&self.db, &name, &spec, &path));
        // Per probe (spec order), its CPU seconds, while profiling: the run's per-probe `ms:rows:cpu_ms` field.
        let timed: Vec<f64> = Vec::new();
        self.fx().open(&spec)?;
        // Members that start with rows (a maintained stratum's survivors, rows SQL wrote): the whole plans run in the
        // first wave, before the first chunk's live counts replan them, so they are planned again here from the
        // members' rows as the component opened (`Stats::sized`). `RERAIL_SIZED_AT_OPEN=off` plans them as before.
        if !(placed.gpu && not_taken.is_none()) && std::env::var("RERAIL_SIZED_AT_OPEN").map_or(true, |v| v != "off") {
            let text_symbols = |i: usize, c: usize| self.model.inputs.atoms.is_none() && f.types(rels[i]).get(c) == Some(&"sym");
            let live = self.fx().stats(&name, &text_symbols)?;
            let started: Vec<(String, LiveRel)> = rels.iter().zip(live).filter(|(r, l)| l.n > 0 && !small_batches(r)).map(|(r, l)| ((*r).clone(), l)).collect();
            if !started.is_empty() {
                let names: Vec<String> = started.iter().map(|(r, _)| r.clone()).collect();
                for (r, l) in started {
                    live_stats(&mut self.model, &r, l);
                }
                self.model.stats.sized(names);
                let mut whole: Vec<(body::RuleKey, Arc<JoinSpec>, Vec<Step>)> = Vec::new();
                for k in 0..probes.len() {
                    let r = probes[k].0;
                    let b = vs.at(r);
                    if sql_probe[k] || unplanned[k] || b.atom(probes[k].1).is_some_and(|a| small_batches(b.rel(a))) {
                        continue;
                    }
                    let (spec, order) = match whole.iter().find(|(p, _, _)| *p == r) {
                        Some((_, w, o)) => (w.clone(), o.clone()),
                        None => {
                            let _span = planning_key(r, "plan whole at open");
                            let ((new, _, _), _) = self.model.plan_rule_live(pf, &b, None)?;
                            let Some(w) = self.whole_join_spec(&b, &new, None, &ri, scc, &native_rels) else { continue };
                            whole.push((r, w.clone(), new.clone()));
                            (w, new)
                        }
                    };
                    self.fx().reprobe(&name, nseeds + k, &Reprobe::Whole(spec))?;
                    wholes.insert(k, order);
                }
            }
        }
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
            let line_bodies: Vec<Body<'_>> = orders.iter().map(|(r, _)| vs.at(*r)).chain(probes.iter().map(|p| vs.at(p.0))).collect();
            for (k, o) in orders.iter().map(|(_, o)| o).chain(probes.iter().map(|p| &p.2)).zip(&line_bodies).map(|(o, b)| Order::new(b, o)).enumerate() {
                eprintln!("    {name} probe {k}: {}", shown(o).join(" , "));
            }
        }
        order_rules.extend(probes.iter().map(|p| vs.at(p.0).observed_key()));
        unplanned.resize(probes.len(), false);
        let replanned_at = replanning::Watch::new(&ri, &probes.iter().map(|p| (vs.at(p.0), p.1)).collect::<Vec<_>>());
        let d = Driving {
            scc: scc.clone(),
            solving: !self.model.stats.solving.is_empty(),
            name,
            spec,
            rels: rels.iter().map(|r| (*r).clone()).collect(),
            ri,
            views,
            sql_views,
            sql_probe,
            orders,
            probes,
            order_rules,
            nseeds,
            seeds,
            ngates: gates.len(),
            explained,
            counted,
            earlier: Vec::new(),
            timed,
            stepped: Vec::new(),
            chunk: 1,
            replanned_at,
            searched: Vec::new(),
            handoff: Vec::new(),
            walled: Vec::new(),
            wholes,
            keys,
            started,
            placed,
            shape,
            cpu_own: if profiling { let (u, s) = corpus::thread_usage(); u + s - cpu_at } else { 0.0 },
            lab,
            not_taken,
            unplanned,
        };
        self.drive(Box::new(d), native_rels)
    }

    /// A probe planned when its delta first has rows (decision 0123 §9), or when the device takes its stratum: its
    /// order, its spec and its explanation. A probe with no native form for its order fails the run, as a replanned
    /// probe does.
    #[allow(clippy::type_complexity)]
    fn plan_probe_now(&mut self, r: body::RuleKey, ai: usize, ri: &BTreeMap<String, usize>, scc: &BTreeSet<String>, native_rels: &BTreeSet<String>, planning: &dyn Fn(body::RuleKey, &str) -> super::profile::Span) -> Result<(Vec<Step>, Arc<JoinSpec>, Option<super::profile::RulePlan>)> {
        let versions = Arc::clone(&self.versions);
        let pf = versions.whole(self.version);
        let vs = Arc::clone(&self.versions);
        let b = vs.at(r);
        self.model.late(pf, &b, Some(ai), false)?;
        plan_site("a stratum's probe, when its delta came");
        let span = planning(r, "plan when its delta came");
        let ((order, _, _), x) = self.model.plan_rule_reused(pf, &b, Some(ai))?;
        drop(span);
        let at = order.iter().position(|s| s.at == ai).expect("the delta atom");
        let spec = self
            .whole_join_spec(&b, &order, Some(at), ri, scc, native_rels)
            .ok_or_else(|| Error::Plan(format!("a probe of {} planned when its delta came has no native step", b.head_text())))?;
        let x = x.map(|x| delta_sources(x, Order::new(&b, &order), at, scc));
        LAZY[1].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok((order, spec, x))
    }

    /// A native stratum's driver: its component's chunks of waves to the fixpoint, replanned from live counts
    /// between them, then what it did recorded and the component closed. A run that waits on its device wave
    /// (`lanes::wave`) suspends the stratum: the driver's state goes back to the strata loop, which resumes it once
    /// the wave ran.
    fn drive(&mut self, d: Box<Driving>, native_rels: &BTreeSet<String>) -> Result<Native> {
        let vs = Arc::clone(&self.versions);
        let versions = Arc::clone(&self.versions);
        let pf = versions.whole(self.version);
        let fields = vs.whole(self.version);
        let Driving { scc: scc_owned, solving, name, spec, rels: rels_owned, ri, views, sql_views, sql_probe, mut orders, mut probes, order_rules, nseeds, seeds, ngates, mut explained, mut counted, mut earlier, mut timed, mut stepped, mut chunk, mut replanned_at, mut searched, keys, started, placed, shape, cpu_own, lab, not_taken, mut unplanned, mut handoff, mut walled, mut wholes } = *d;
        let scc = &scc_owned;
        let rels: Vec<&String> = rels_owned.iter().collect();
        // Each line's rule as the planner reads it: the seeds' orders, then the probes' (their rules fixed, their
        // orders replanned in place).
        let line_bodies: Vec<Body<'_>> = orders.iter().map(|(r, _)| vs.at(*r)).chain(probes.iter().map(|p| vs.at(p.0))).collect();
        let planning = |k: body::RuleKey, what: &str| {
            super::profile::span(|| {
                let b = vs.at(k);
                let no = k.number().map_or("?".to_owned(), |k| format!("{k:04}"));
                format!("{what} #{no} {}, {} literals", b.rel(b.head()), b.len())
            })
        };
        let profiling = super::profile::on();
        // This thread's CPU at the stratum's start, less what other strata spent while it waited.
        let cpu_at = if profiling { let (u, s) = corpus::thread_usage(); u + s - cpu_own } else { 0.0 };
        handoff.resize_with(nseeds + probes.len(), || None);
        // Each relation's rows, delta start and end as its statistics were last read (`stats_changed`).
        let mut live_seen: Vec<[usize; 3]> = Vec::new();
        // `RERAIL_JOIN_CUT_ONLY=<text>` (a measurement aid): only components whose name holds it are cut.
        let cut_only = std::env::var("RERAIL_JOIN_CUT_ONLY").ok();
        loop {
            // Decision 0206: each join's budget (a seed's or a probe's), the work left before its line is next checked
            // against its price (`joins::next_due`), or before a replacement's own limit. A join that spends it is cut
            // at the row of its first atom it is at, and the run returns here (`5`).
            if joins::on() && self.fx().direct && cut_only.as_deref().is_none_or(|s| name.contains(s)) {
                let budgets: Vec<(usize, Option<u64>)> = (0..nseeds + probes.len()).filter(|&line| line < nseeds || !sql_probe[line - nseeds] && !unplanned[line - nseeds]).map(|line| {
                    let work = counted.get(line).map_or(0, |c| super::profile::plan_work(c)) as f64;
                    let due = match &handoff[line] {
                        Some(h) => Some(h.limit),
                        None => keys.get(line).copied().flatten().and_then(|key| {
                            let k = line.wrapping_sub(nseeds);
                            joins::next_due(key, if line < nseeds { &orders[line].1 } else { wholes.get(&k).unwrap_or(&probes[k].2) })
                        }),
                    };
                    (line, due.map(|d| (d - work).max(1.0) as u64))
                }).collect();
                super::native::component::set_budgets(self.db, &name, &budgets)?;
            }
            let f: String = {
                let _wave = super::lanes::wave::suspendable();
                self.fx().run(&name, chunk as u32)?
            };
            let f: Vec<&str> = f.split_whitespace().collect();
            // Each probe's milliseconds in this run, its first figure (always measured).
            if let Some(per) = f.get(7).filter(|_| f.len() == 9) {
                for (k, item) in per.split(',').enumerate() {
                    if walled.len() <= k { walled.resize(k + 1, 0.0); }
                    walled[k] += item.split(':').next().and_then(|t| t.parse::<f64>().ok()).unwrap_or(0.0) / 1e3;
                }
            }
            let cut = if f.last() == Some(&"5") { super::native::component::take_cut(self.db, &name)? } else { None };
            if let Some(c) = &cut {
                joins::log(|| format!("cut {name} line {}: {} rows of work, {} rows of its first atom done, {} left{}", c.probe, c.work, c.done, c.left, if c.led { "" } else { " (not known: all)" }));
            }
            if let Some(per) = f.get(7).filter(|_| f.len() == 9 && super::profile::on()) {
                for (k, item) in per.split(',').enumerate() {
                    if timed.len() <= k {
                        timed.resize(k + 1, 0.0);
                    }
                    timed[k] += item.rsplit(':').next().and_then(|c| c.parse::<f64>().ok()).unwrap_or(0.0) / 1e3;
                }
            }
            macro_rules! suspended {
                () => {{
                    drop(rels);
                    Box::new(Driving {
                        scc: scc_owned, solving, name, spec, rels: rels_owned, ri, views, sql_views, sql_probe, orders, probes, order_rules, nseeds, seeds, ngates,
                        explained, counted, earlier, timed, stepped, chunk, replanned_at, searched, keys, started, placed, shape, lab, not_taken, unplanned, handoff, walled, wholes,
                        cpu_own: if profiling { let (u, s) = corpus::thread_usage(); u + s - cpu_at } else { 0.0 },
                    })
                }};
            }
            // Requests, where they are posted (decision 0114 §3): the stratum suspends with its tickets.
            if f.last() == Some(&"2") && self.asking.is_some() && self.model.inputs.atoms.is_some() {
                let asked = super::native::component::pending(&self.db, &name)?;
                let asking = self.asking.as_deref_mut().expect("checked");
                let tickets = asked.iter().map(|(rel, rows)| asking.post(rel, rows)).collect::<Result<Vec<u64>>>()?;
                ANSWER_CALLS.fetch_add(tickets.len() as u64, std::sync::atomic::Ordering::Relaxed);
                return Ok(Native::Asking(suspended!(), tickets));
            }
            if f.last() == Some(&"2") {
                let log = self.logging.map(|at| self.answer_logs.entry(at).or_default());
                answer_pending(&self.db, self.model.inputs.atoms.is_some(), &mut self.answer, &mut self.answered, log, &name, scc)?;
                continue;
            }
            // Waiting on its device wave: the stratum is suspended with its driver's state, and resumed here.
            if f.last() == Some(&"3") {
                return Ok(Native::Waiting(suspended!()));
            }
            let steps = self.fx().steps(&name)?;
            if profiling {
                for (k, line) in self.fx().step_cpu(&name)?.into_iter().enumerate() {
                    if stepped.len() <= k { stepped.resize(k + 1, Vec::new()); }
                    if stepped[k].len() < line.len() { stepped[k].resize(line.len(), 0.0); }
                    for (a, b) in stepped[k].iter_mut().zip(line) { *a += b; }
                }
            }
            // A line that did nothing this chunk observed nothing (decision 0122: no planning work for it).
            let idle = |line: &[(i64, i64)]| line.iter().all(|&(a, b)| a == 0 && b == 0);
            for ((o, &rule), line) in orders.iter().map(|(_, o)| o).chain(probes.iter().map(|p| &p.2)).zip(&line_bodies).map(|(o, b)| Order::new(b, o)).zip(&order_rules).zip(&steps) {
                if !idle(line) {
                    observe(&mut self.model.stats, rule, o, line);
                }
            }
            // A step whose fanout was observed, or moved, is priced by it now: its probe is planned again.
            for (k, p) in probes.iter().enumerate() {
                if steps.get(nseeds + k).is_some_and(|l| !idle(l)) {
                    replanned_at.observed(k, observed_fans(&self.model.stats, order_rules[nseeds + k], Order::new(&line_bodies[nseeds + k], &p.2)));
                }
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
            let text_symbols = |i: usize, c: usize| self.model.inputs.atoms.is_none() && fields.types(rels[i]).get(c) == Some(&"sym");
            // Only the relations whose rows moved since the last chunk: the others' statistics are as recorded.
            let live = self.fx().stats_changed(&name, &mut live_seen, &text_symbols)?;
            for (i, (r, l)) in rels.iter().zip(live).enumerate() {
                let Some(l) = l else { continue };
                replanned_at.rows(i, l.n);
                replanned_at.delta(i, l.delta);
                self.model.stats.delta_rows.insert((*r).clone(), l.delta as f64);
                for slot in live_stats(&mut self.model, r, l) {
                    replanned_at.record(i, slot, &self.model, r);
                }
            }
            // Decision 0203 §3: a probe whose work so far passed its price, for the delta rows it read, by
            // `slow_factor` is flagged while it runs. Its rule's exact pair statistics are measured on the engine's
            // rows, it is planned again now with them, and a wider search is asked for.
            if joins::on() {
                for k in 0..probes.len() {
                    let line = nseeds + k;
                    if sql_probe[k] || unplanned[k] {
                        continue;
                    }
                    let (Some(Some(key)), Some(c)) = (keys.get(line).copied(), counted.get(line)) else { continue };
                    let work = super::profile::plan_work(c) as f64;
                    if !joins::check(key, work, &probes[k].2) {
                        continue;
                    }
                    let (r, ai) = (probes[k].0, probes[k].1);
                    let b = vs.at(r);
                    // A body of one positive atom has one order: nothing to plan again.
                    if positive_atoms(&b) < 2 {
                        continue;
                    }
                    let delta = c.first().map_or(0, |s| s.1) as f64;
                    let Some(est) = self.model.price_for_delta(pf, &b, Some(ai), &probes[k].2, delta) else { continue };
                    if work <= joins::slow_factor() * est.max(1.0) {
                        continue;
                    }
                    let rule = order_rules[line];
                    joins::flag_slow(key, rule, &probes[k].2, est, work);
                    super::profile::decision("slow joins (decision 0203)", || format!("{} (delta at {ai}): {work:.0} rows of work, priced {est:.0} for its {delta:.0} delta rows", b.head_text()));
                    let exact = if joins::pricing() { Vec::new() } else { self.exact_statistics(&name, line, false, &b, &probes[k].2) };
                    joins::log(|| format!("slow {} (delta at {ai}): {work:.0} rows of work, priced {est:.0} for its {delta:.0} delta rows; {} exact pair statistics", b.head_text(), exact.len()));
                    joins::put_exact(key, rule, exact);
                    self.heavy_for([r], true, Some(key));
                    replanned_at.force(k);
                    self.request_search(r, Some(ai), rule, key, work, Some((&name, line, &probes[k].2)))?;
                }
            }
            // Decision 0123 §9: probes not planned yet whose delta has rows; planned now, as one round, and the run
            // goes on. A probe whose delta never has rows is never planned.
            if f.last() == Some(&"4") {
                let due: Vec<usize> = (0..probes.len()).filter(|&k| unplanned[k] && replanned_at.delta_rows(k) > 0).collect();
                if due.is_empty() {
                    return Err(Error::Plan(format!("{name}: the run returned for a probe's plan, and none not planned has rows")));
                }
                let mut wanted = BTreeSet::new();
                self.model.wants(due.iter().map(|&k| (&line_bodies[nseeds + k], Some(probes[k].1))), &mut wanted);
                self.model.make_statistics(pf, &wanted)?;
                for k in due {
                    let (r, ai) = (probes[k].0, probes[k].1);
                    let t0 = if profiling { super::profile::now() } else { 0.0 };
                    let (order, spec, x) = self.plan_probe_now(r, ai, &ri, scc, native_rels, &planning)?;
                    self.fx().reprobe(&name, nseeds + k, &Reprobe::Join(spec))?;
                    let e = &mut explained[nseeds + k];
                    e.2 = x;
                    e.3 += if profiling { super::profile::now() - t0 } else { 0.0 };
                    probes[k].2 = order;
                    unplanned[k] = false;
                }
                continue;
            }
            if f.last() == Some(&"1") || (chunk == 0 && cut.is_none()) {
                break;
            }
            // A cut probe's order before this chunk's replans, to tell whether it was replaced.
            let before: Option<Vec<Step>> = cut.as_ref().filter(|c| c.probe >= nseeds && c.probe < nseeds + probes.len()).map(|c| probes[c.probe - nseeds].2.clone());
            {
                // The whole plans, planned from seed estimates (a derived relation empty, its columns without
                // distinct counts), again from the live counts: once per rule, the same plan for each of its probes
                // (`setfix` runs it once a wave).
                let mut whole: Vec<(body::RuleKey, Arc<JoinSpec>, Vec<Step>)> = Vec::new();
                let wants_whole = |k: usize, this: &Self| {
                    let b = &line_bodies[nseeds + k];
                    let delta = b.atom(probes[k].1).map(|a| b.rel(a));
                    !sql_probe[k]
                        && !delta.is_some_and(small_batches)
                        // A whole plan runs over its relation's delta, all its rows: searched again only where that
                        // delta costs more than the search (decision 0122).
                        && !(plan_budget() && delta.is_some_and(|a| this.model.stats.delta_rows.get(a).is_none_or(|&d| d < SEARCH_WORK)))
                };
                // A round (decision 0123): the whole plans' statistics in one batch, then the plans.
                // Searched whatever was kept (`plan_rule_live`): every deferred relation they read.
                let wanted: BTreeSet<String> = (0..probes.len()).filter(|&k| wants_whole(k, self)).flat_map(|k| Model::read_by_body(&line_bodies[nseeds + k])).map(str::to_owned).collect();
                self.model.make_statistics(pf, &wanted)?;
                // Decision 0354: the rules' whole plans searched first, their choices made together after one batch
                // prediction (`joins::choose`), as the probes' below: a round a chunk, not one a rule.
                let mut round: Vec<joins::Choice> = Vec::new();
                let mut searched_whole: Vec<(usize, Vec<Step>, Option<usize>)> = Vec::new();
                for k in 0..probes.len() {
                    if unplanned[k] || !wants_whole(k, self) {
                        continue;
                    }
                    let r = probes[k].0;
                    let b = &line_bodies[nseeds + k];
                    if !replanned_at.take_whole(k) { continue; }
                    if whole.iter().any(|(p, _, _)| *p == r) || searched_whole.iter().any(|(j, _, _)| probes[*j].0 == r) {
                        searched_whole.push((k, Vec::new(), None));
                        continue;
                    }
                    self.model.late(pf, b, None, true)?;
                    let _span = planning(r, "replan whole");
                    let asked = round.len();
                    let ((new, _, _), _) = self.model.plan_rule_live_in(pf, b, None, Some(&mut round))?;
                    searched_whole.push((k, new, (round.len() > asked).then_some(asked)));
                }
                let choices = joins::finish_round(round);
                for (k, mut new, asked) in searched_whole {
                    let r = probes[k].0;
                    let b = &line_bodies[nseeds + k];
                    let (spec, order) = match whole.iter().find(|(p, _, _)| *p == r) {
                        Some((_, w, o)) => (w.clone(), o.clone()),
                        None => {
                            if let Some(c) = asked.and_then(|i| choices.get(i)).filter(|c| c.chosen != 0)
                                && let Ok(((order, _, _), _)) = self.model.plan_inner(pf, b, None, Some(&c.candidates[c.chosen].order), false)
                            {
                                super::profile::decision("join choices (decision 0203)", || format!("{}: {}", b.head_text(), c.why));
                                joins::log(|| format!("choice {} (whole, round): {}", b.head_text(), c.why));
                                new = order;
                            }
                            let Some(w) = self.whole_join_spec(b, &new, None, &ri, scc, &native_rels) else { continue };
                            whole.push((r, w.clone(), new.clone()));
                            (w, new)
                        }
                    };
                    self.fx().reprobe(&name, nseeds + k, &Reprobe::Whole(spec))?;
                    wholes.insert(k, order);
                }
                // Maintenance probes deliberately have no whole plan. They must also use the seed wave's
                // measured domains before their first delta runs, rather than waiting two further waves.
            }
            // The probes searched again this chunk, then a round (decision 0123): their statistics in one batch.
            let mut replan: Vec<usize> = Vec::new();
            for k in 0..probes.len() {
                if sql_probe[k] || unplanned[k] {
                    continue;
                }
                // Decision 0122: a probe is searched again only once its work so far, and the delta it is about to
                // read, pay for its searches; until then a dirty plan stays dirty.
                // A line flagged slow (decision 0203 §3) is searched again now, whatever its budget.
                let forced = joins::any_noted() && keys.get(nseeds + k).copied().flatten().is_some_and(|key| joins::take_forced(&key));
                if plan_budget() && !forced {
                    if searched.len() <= k { searched.resize(probes.len(), 0); }
                    let line = nseeds + k;
                    let done = counted.get(line).map_or(0, |c| super::profile::plan_work(c)) + earlier.get(line).map_or(0, |e| e.iter().map(|(_, c)| super::profile::plan_work(c)).sum());
                    if (done as f64 + replanned_at.delta_rows(k) as f64) < SEARCH_WORK * f64::from(searched[k] + 1) { continue; }
                }
                // The dependency index invalidates only affected plans. An empty delta leaves its plan dirty
                // until it can run; unchanged probes never rebuild keys or statistics signatures.
                if !replanned_at.take(k) { continue; }
                if let Some(n) = searched.get_mut(k) { *n += 1; }
                replan.push(k);
            }
            let mut wanted = BTreeSet::new();
            for &k in &replan {
                // Searched whatever was kept (`plan_rule_live`): every deferred relation it reads.
                wanted.extend(Model::read_by_body(&line_bodies[nseeds + k]).into_iter().map(str::to_owned));
            }
            self.model.make_statistics(pf, &wanted)?;
            // Decision 0203 §2: the round's lines are searched first, their choices made together after one batch
            // prediction (`joins::choose`), then applied.
            let mut round: Vec<joins::Choice> = Vec::new();
            let mut planned = Vec::with_capacity(replan.len());
            for k in replan {
                let (r, ai) = (probes[k].0, probes[k].1);
                let b = &line_bodies[nseeds + k];
                self.model.late(pf, b, Some(ai), true)?;
                let span = planning(r, "replan");
                let t0 = if profiling { super::profile::now() } else { 0.0 };
                let asked = round.len();
                let ((new, _, _), x) = self.model.plan_rule_live_in(pf, b, Some(ai), Some(&mut round))?;
                let asked = (round.len() > asked).then_some(asked);
                drop(span);
                let e = &mut explained[nseeds + k];
                e.3 += if profiling { super::profile::now() - t0 } else { 0.0 };
                planned.push((k, new, x, asked));
            }
            let choices = joins::finish_round(round);
            for (k, mut new, mut x, asked) in planned {
                let ai = probes[k].1;
                let b = &line_bodies[nseeds + k];
                if let Some(c) = asked.and_then(|i| choices.get(i)).filter(|c| c.chosen != 0)
                    && let Ok(((order, _, _), _)) = self.model.plan_inner(pf, b, Some(ai), Some(&c.candidates[c.chosen].order), false)
                    && self.model.valid_live_driver(&order, b, Some(ai))
                {
                    super::profile::decision("join choices (decision 0203)", || format!("{}: {}", b.head_text(), c.why));
                    joins::log(|| format!("choice {} (round): {}", b.head_text(), c.why));
                    new = order;
                    x = None;
                }
                let e = &mut explained[nseeds + k];
                if same_order(&new, &probes[k].2) {
                    continue;
                }
                e.4 += 1;
                let at = new.iter().position(|s| s.at == ai).expect("the delta atom");
                let x = x.or_else(|| self.model.explain_order(pf, b, Some(ai), &new));
                let e = &mut explained[nseeds + k];
                e.2 = x.map(|x| delta_sources(x, Order::new(b, &new), at, scc));
                let at = new.iter().position(|s| s.at == ai).expect("the delta atom");
                let spec = self
                    .whole_join_spec(b, &new, Some(at), &ri, scc, &native_rels)
                    .ok_or_else(|| Error::Plan(format!("a replanned probe of {} has no native step", b.head_text())))?;
                self.fx().reprobe(&name, nseeds + k, &Reprobe::Join(spec))?;
                // The counts so far are the old order's: kept with it, and the new order's counted from nothing. Summed
                // by place, a step's counts were another order's step there (`pfn`'s `fv` rows shown at `pcall__ins`).
                let line = nseeds + k;
                if earlier.len() <= line { earlier.resize_with(line + 1, Vec::new); }
                let ran = counted.get_mut(line).map(std::mem::take).unwrap_or_default();
                // Its CPU by step went with the old order (`native`'s earlier orders are shown without it).
                if let Some(c) = stepped.get_mut(line) { c.clear(); }
                if ran.iter().any(|&(a, _)| a > 0) { earlier[line].push((std::mem::replace(&mut probes[k].2, new), ran)); }
                else { probes[k].2 = new; }
            }
            // Decision 0206, the handoff: a join cut at its check, then replaced, reads the rows of its first atom its
            // old order left, under a limit of `slow_factor` times its price for them. Cut again, the order with the
            // less work per row goes on from there, unlimited. A probe's whole plan (O168) is cut and handed off as a
            // seed is: it leads with its first atom, and reads no delta.
            if let Some(c) = &cut && c.probe < nseeds + probes.len() {
                let line = c.probe;
                let k = line.wrapping_sub(nseeds);
                let whole = c.whole && line >= nseeds && wholes.contains_key(&k);
                let seed = line < nseeds;
                let led = seed || whole;
                let (r, first) = if seed { (orders[line].0, None) } else if whole { (probes[k].0, None) } else { (probes[k].0, Some(probes[k].1)) };
                let b = vs.at(r);
                let order_now = |orders: &Vec<(RuleKey, Vec<Step>)>, probes: &Vec<(RuleKey, usize, Vec<Step>)>, wholes: &HashMap<usize, Vec<Step>>| -> Vec<Step> {
                    if seed { orders[line].1.clone() } else if whole { wholes[&k].clone() } else { probes[k].2.clone() }
                };
                let mut old = if led { Some(order_now(&orders, &probes, &wholes)) } else { before.clone() };
                let cut_lead = old.as_ref().map_or_else(|| order_now(&orders, &probes, &wholes)[0].at, |o| o[0].at);
                // Puts `new` in the engine for the line, and in the driver's place for it; its counts so far stay
                // with the order they were the old one's (a whole plan's are not counted).
                macro_rules! place {
                    ($new:expr) => {{
                        let new: Vec<Step> = $new;
                        let at = first.map(|ai| new.iter().position(|s| s.at == ai).expect("the delta atom"));
                        match self.whole_join_spec(&b, &new, at, &ri, scc, native_rels) {
                            Some(spec) => {
                                self.fx().reprobe(&name, line, &if whole { Reprobe::Whole(spec) } else { Reprobe::Join(spec) })?;
                                if new[0].at == cut_lead { super::native::component::cut_step(self.db, &name, 0)?; }
                                if whole {
                                    wholes.insert(k, new);
                                } else {
                                    if earlier.len() <= line { earlier.resize_with(line + 1, Vec::new); }
                                    let ran = counted.get_mut(line).map(std::mem::take).unwrap_or_default();
                                    if let Some(c) = stepped.get_mut(line) { c.clear(); }
                                    let slot = if seed { &mut orders[line].1 } else { &mut probes[k].2 };
                                    if ran.iter().any(|&(a, _)| a > 0) { earlier[line].push((std::mem::replace(slot, new), ran)); }
                                    else { *slot = new; }
                                }
                                true
                            }
                            None => false,
                        }
                    }};
                }
                // The line's work: its counts, and a whole plan's work as its cut reported it.
                let work = counted.get(line).map_or(0, |c| super::profile::plan_work(c)) as f64 + if whole { c.work as f64 } else { 0.0 };
                // Decision 0207, trials: at a line's first check in the process (and once more after a flag), its
                // orders run over a sample of the rows left (decision 0274: only a line observed on a run's critical
                // path, `joins::take_trial`), each cut at `trial_cutoff` times the least work measured
                // so far and at `trial_share` of the line's work. What each measured over its price is kept as an
                // order tried; the least measured goes on where it beats the running order by `trial_margin`.
                // The sample is what the share pays for at the running order's work per row, at most `trial_rows`;
                // under `TRIAL_LEAST` rows, the trials wait for a later check.
                let cap = (joins::trial_share() * work).max(1_000.0) as u64;
                let sample = if c.led && c.done > 0 { ((cap as f64 * c.done as f64 / c.work.max(1) as f64) as usize).min(joins::trial_rows() as usize) } else { 0 };
                const TRIAL_LEAST: usize = 8;
                let joined = positive_atoms(&b) >= 2;
                // A seed or whole plan is checked here, at its cut (a probe was, above).
                if led && let Some(key) = keys.get(line).copied().flatten() {
                    let running = order_now(&orders, &probes, &wholes);
                    // The walk cut now is judged by its own work over its own rows (a line's work so far may be an
                    // earlier walk's too: then a few rows read under a short budget stood for all of it).
                    let ran = c.work as f64;
                    let est = if joins::check(key, work, &running) { self.model.price_for_delta(pf, &b, Some(cut_lead), &running, c.done.max(1) as f64) } else { None };
                    if let Some(est) = est && ran > joins::slow_factor() * est.max(1.0) && positive_atoms(&b) >= 2 {
                        let rule = order_rules[line];
                        joins::flag_slow(key, rule, &running, est, ran);
                        super::profile::decision("slow joins (decision 0203)", || format!("{} {}: {ran:.0} rows of work, priced {est:.0} for its {} rows read", if whole { "whole plan" } else { "seed" }, b.head_text(), c.done));
                        let exact = if joins::pricing() { Vec::new() } else { self.exact_statistics(&name, line, whole, &b, &running) };
                        joins::log(|| format!("slow {} {}: {ran:.0} rows of work, priced {est:.0} for its {} rows read; {} exact pair statistics; its share of a critical path {:.3}{}", if whole { "whole plan" } else { "seed" }, b.head_text(), c.done, exact.len(), joins::note(&key).map_or(0.0, |n| n.critical), if joins::trial_due(&key) { ", trialled" } else { "" }));
                        joins::put_exact(key, rule, exact);
                        self.heavy_for([r], true, Some(key));
                        joins::take_forced(&key);
                        self.request_search(r, None, rule, key, work, Some((&name, line, &running)))?;
                        // With trials (decision 0207) the orders are measured below instead, where they can be. Decision
                        // 0271: a line no trial measures is planned again now, with the exact statistics just taken,
                        // and its replacement runs under the handoff's limit. That is a line whose sample is under
                        // `TRIAL_LEAST` (its few lead rows each cost more than its check's budget: `pre_570` did 0 of
                        // 423 in 201,431 rows of work, then ran 22 s unbounded). A line in a handoff is judged by it.
                        // Q80: where the exact statistics plan the order it runs, they did not explain its work, and
                        // its price stays as wrong: the least priced of the search's runners-up under what the order
                        // measured goes in its place, under the handoff's limit (`joins::over_measured`).
                        if !joins::measured_below(sample >= TRIAL_LEAST && joined && joins::trial_due(&key), handoff[line].is_some()) {
                            let mut cx = joins::SearchCx::default();
                            let new = self.model.plan_for_rows_in(pf, &b, cut_lead, c.left.max(1) as f64, &mut cx).and_then(|new| {
                                if !same_order(&new, &running) {
                                    return Some((new, "planned again now"));
                                }
                                let others: Vec<(Vec<Step>, f64)> = cx.runners_up.into_iter().filter(|(o, _)| o.len() == b.len() && !same_order(o, &running)).collect();
                                joins::over_measured(&others, ran, c.done, c.left).map(|o| (o, "planned again into the order it runs: a runner-up priced under what it measured goes on"))
                            });
                            if let Some((new, why)) = new {
                                if joins::trials_on() {
                                    joins::log(|| format!("slow {}: {} of its lead rows done, under a trial's {TRIAL_LEAST}: {why}", b.head_text(), c.done));
                                }
                                place!(new);
                            }
                        }
                    }
                }
                if let Some(key) = keys.get(line).copied().flatten() && joins::trials_on() && joined && sample >= TRIAL_LEAST && handoff[line].is_none() && joins::take_trial(key) {
                    // Decision 0209: the candidates are priced with the line's skewed join columns' heavy values.
                    self.heavy_for([r], true, Some(key));
                    let running = order_now(&orders, &probes, &wholes);
                    let cut_order = old.clone().unwrap_or_else(|| running.clone());
                    let lead = first.unwrap_or(cut_order[0].at);
                    let left = if c.led { c.left } else { c.done + c.left };
                    let mut cands: Vec<Vec<Step>> = vec![running.clone(), cut_order.clone()];
                    // The background's walked work of the proposals tried, which size an own-lead trial's sample.
                    let mut walked_props: Vec<(Vec<Step>, f64)> = Vec::new();
                    // Decision 0243: where a background search walked the line's orders, the line searches nothing here:
                    // it tries the proposals whose walked work beats the order it ran, the best two. Else, the
                    // search as before (decision 0207), and its request waits for the background.
                    match joins::walked(&key) {
                        Some((ran_w, props)) => {
                            // Decision 0247: past the walks' spread (`WALK_MARGIN`).
                            let better: Vec<(Vec<Step>, f64)> = props.into_iter().filter(|(_, w)| *w < joins::WALK_MARGIN * ran_w).take(2).collect();
                            joins::log(|| format!("trials {}: the background's walked proposals, the order it ran at {ran_w:.0}: {}", b.head_text(), if better.is_empty() { "none better".to_owned() } else { better.iter().map(|(o, w)| format!("{} {w:.0}", o.iter().map(|s| s.at.to_string()).collect::<Vec<_>>().join("."))).collect::<Vec<_>>().join(", ") }));
                            walked_props = better.clone();
                            cands.extend(better.into_iter().map(|(o, _)| o));
                        }
                        None => {
                            let mut cx = joins::SearchCx::default();
                            if let Some(best) = self.model.plan_for_rows_in(pf, &b, lead, left.max(1) as f64, &mut cx) {
                                cands.push(best);
                                cands.extend(cx.runners_up.into_iter().map(|(o, _)| o));
                            }
                            cands.extend(joins::proposals(&key));
                        }
                    }
                    let mut distinct: Vec<Vec<Step>> = Vec::new();
                    for o in cands {
                        if o.len() == b.len() && !distinct.iter().any(|d| same_order(d, &o)) { distinct.push(o); }
                    }
                    distinct.truncate(6);
                    // O171: where the line's key has heavy values, the sample is drawn from the heavy keys' rows and
                    // the rest apart, a quarter of it from the heavy, and each order's work per row is their mean
                    // weighted by the parts' rows. At a stride alone, a key whose heavy values hold most of the work
                    // was measured on its light rows (`call_ret`: 23% of the rows did 96% of the work).
                    // Decision 0261: each part's work is estimated by ratio to its rows' sizes (`heavy_parts`), not
                    // by its rows: a heavy row's work grows with its key's rows, and a part's few sampled rows
                    // stood for all of its rows at their mean.
                    // Decision 0233, two-phase sampling: an order's work over a part is its trial's, scaled by the
                    // ratio of two walks (`Walker`): over a uniform sample of the part's rows (the first phase), and
                    // over the rows its trial ran (the second). The walk measures a row's work at its key through
                    // filters, chains and the atoms closing cycles and triangles; its cost is fixed by the sample.
                    let t_model = std::time::Instant::now();
                    let hp = self.heavy_parts(&name, line, &b, lead)?.filter(|hp| !hp.hot.is_empty() && !hp.cold.is_empty());
                    let parts_s = t_model.elapsed().as_secs_f64();
                    // The parts are the relation's own row ids (`trial_ids`), read through it.
                    let lead_src = match b.view(lead) { Lit::Atom(a) if hp.is_some() => super::native::component::held(self.db, b.rel(a)), _ => None };
                    let walker = lead_src.as_ref().map(|_| {
                        let db = self.db;
                        Walker::new(&b, lead, joins::walk_width(), &|rel| super::native::component::held(db, rel), &|x| super::native::component::symbol_id(db, x).ok().flatten())
                    });
                    let lead_cols = match b.view(lead) { Lit::Atom(_) => b.args(lead).len(), _ => 0 };
                    let values = |ids: &[u32]| -> Vec<Vec<i64>> {
                        let src = lead_src.as_ref().expect("a walker");
                        ids.iter().map(|&id| (0..lead_cols).map(|c| src.at(id as usize).at(c)).collect()).collect()
                    };
                    // The first phase's rows: a uniform sample of each part (seeded), as lead rows.
                    let phase1: Option<[Vec<Vec<i64>>; 2]> = hp.as_ref().filter(|_| walker.is_some()).map(|hp| {
                        let pick = |ids: &[u32], salt: u64| -> Vec<u32> {
                            let mut v: Vec<(u64, u32)> = ids.iter().map(|&id| ((id as u64 + salt).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 7, id)).collect();
                            v.sort_unstable();
                            v.into_iter().take(joins::walk_rows()).map(|x| x.1).collect()
                        };
                        [values(&pick(&hp.hot, 1)), values(&pick(&hp.cold, 2))]
                    });
                    // The running order's first-phase walks, per part: they size each part's sample, and are its x1.
                    let x1_run: Option<[f64; 2]> = walker.as_ref().zip(phase1.as_ref()).map(|(w, ph)| [w.work(&running, &ph[0]), w.work(&running, &ph[1])]);
                    let parts = hp.as_ref().map(|hp| {
                        // (the rows picked, the part's rows)
                        let stride = |ids: &[u32], k: usize| -> (Vec<u32>, usize) {
                            let st = ids.len().div_ceil(k.max(1)).max(1);
                            (ids.iter().step_by(st).copied().collect(), ids.len())
                        };
                        let most = joins::trial_rows() as usize;
                        let (kh, kc) = match x1_run {
                            // Decision 0236: each part takes the rows that fit half its budget at the running order's
                            // work per row there, its walk in the units of the line's work so far (the line's work
                            // per row done, over the walks' mean). A quarter of a sample sized at the line's mean
                            // work, drawn from rows of heavy keys, passed the budget on every order.
                            Some(x) if c.done > 0 => {
                                let (h, n) = (hp.hot.len() as f64, hp.cold.len() as f64);
                                let mean = (h * x[0] + n * x[1]) / (h + n);
                                let unit = c.work as f64 / c.done as f64 / mean.max(1e-9);
                                let fit = |x: f64| (cap as f64 / 2.0 * 0.5 / (x * unit).max(1.0)) as usize;
                                (fit(x[0]).min(most).max(2).min(hp.hot.len()), fit(x[1]).min(most).max(1).min(hp.cold.len()))
                            }
                            _ => {
                                let kh = (sample / 4).max(2).min(hp.hot.len());
                                (kh, sample.saturating_sub(kh).max(1).min(hp.cold.len()))
                            }
                        };
                        [stride(&hp.hot, kh), stride(&hp.cold, kc)]
                    });
                    let walk_s = std::cell::Cell::new(0.0f64);
                    // Decision 0236: every candidate is walked over the first phase's rows, per part; an order whose
                    // walked work over the line passes `trial_cutoff` times the least walked is not tried. The walks
                    // cost probes, bounded by the sample; a trial of a poor order cost its whole cut budget (on the
                    // data-layer files four such trials did as much work as the line).
                    let t_walk = super::native::component::thread_cpu();
                    // Decision 0245: an order leading with another atom is not walked from the cut lead's rows (on the
                    // data-layer files such a walk measured 248 M for a line of 5.7 M, and left the order untried).
                    let walked: Vec<Option<[f64; 2]>> = distinct.iter().map(|o| match (&walker, &phase1, x1_run) {
                        (_, _, Some(x)) if same_order(o, &running) => Some(x),
                        (Some(w), Some(ph), _) if o[0].at == lead => Some([w.work(o, &ph[0]), w.work(o, &ph[1])]),
                        _ => None,
                    }).collect();
                    walk_s.set(super::native::component::thread_cpu() - t_walk);
                    let line_walk: Vec<Option<f64>> = walked.iter().map(|x| x.zip(hp.as_ref()).map(|(x, hp)| hp.hot.len() as f64 * x[0] + hp.cold.len() as f64 * x[1])).collect();
                    let least_walk = line_walk.iter().flatten().copied().fold(f64::INFINITY, f64::min);
                    let walked_out = |i: usize| i > 0 && line_walk[i].is_some_and(|w| w > joins::trial_cutoff() * least_walk);
                    // The running order first (the others' specs lead from its cut), then the rest by walked work.
                    let mut by_walk: Vec<usize> = (1..distinct.len()).filter(|&i| !walked_out(i)).collect();
                    by_walk.sort_by(|&x, &y| line_walk[x].unwrap_or(f64::INFINITY).total_cmp(&line_walk[y].unwrap_or(f64::INFINITY)));
                    // Where the walks leave only the running order, its trial would decide nothing: none runs (on 328
                    // data-layer files the trial alone was cut after 8.6 s).
                    let tried: Vec<usize> = if by_walk.is_empty() && line_walk[0].is_some() { Vec::new() } else { std::iter::once(0).chain(by_walk).collect() };
                    if line_walk.iter().any(|w| w.is_some()) {
                        joins::log(|| format!("walks {}: each order's walked work over the line: {}", b.head_text(), distinct.iter().zip(&line_walk).enumerate().map(|(i, (o, w))| format!("{}{} {:.0}{}", if same_order(o, &running) { "*" } else { "" }, o.iter().map(|s| s.at.to_string()).collect::<Vec<_>>().join("."), w.unwrap_or(f64::NAN), if walked_out(i) { " (not tried)" } else { "" })).collect::<Vec<_>>().join(", ")));
                        if tried.is_empty() { joins::log(|| format!("walks {}: only the running order is within reach; no trial runs", b.head_text())); }
                    }
                    // An order's estimated work over part `p`, from its trial `run` over the rows `pick` (in order: a
                    // cut trial ran their first `run.done`): `None` without a walker.
                    let two_phase = |o: &[Step], p: usize, run: &super::native::component::TrialRun, pick: &[u32], size: usize| -> Option<(f64, f64, f64)> {
                        let (w, ph) = (walker.as_ref()?, phase1.as_ref()?);
                        let t = super::native::component::thread_cpu();
                        let ran = &pick[..(run.done as usize).clamp(1, pick.len())];
                        let x1 = match distinct.iter().zip(&walked).find(|(d, _)| same_order(d, o)) { Some((_, Some(x))) => x[p], _ => w.work(o, &ph[p]) };
                        let x2 = w.work_at(o, &values(ran), 4 * joins::walk_width());
                        walk_s.set(walk_s.get() + super::native::component::thread_cpu() - t);
                        let y = run.work as f64 / run.rows.max(1) as f64;
                        Some((size as f64 * y * if x2 > 0.0 { x1 / x2 } else { 1.0 }, x1, x2))
                    };
                    // The running order's walks, for the log: (first phase, second) per part.
                    let ratios: std::cell::RefCell<Vec<(usize, f64, f64)>> = Default::default();
                    let t_trials = std::time::Instant::now();
                    let mut measured: Vec<(Vec<Step>, super::native::component::TrialRun)> = Vec::new();
                    let mut least: [Option<u64>; 2] = [None; 2];
                    for &i in &tried {
                        let o = &distinct[i];
                        let spec = if i == 0 { None } else {
                            let at = first.map(|ai| o.iter().position(|s| s.at == ai));
                            if at == Some(None) { continue; }
                            let Some(spec) = self.whole_join_spec(&b, o, at.flatten(), &ri, scc, native_rels) else { continue };
                            Some((spec, (o[0].at == cut_order[0].at).then_some(0)))
                        };
                        let budget = |least: Option<u64>, cap: u64| least.map_or(cap, |w| ((joins::trial_cutoff() * w as f64) as u64).clamp(1, cap));
                        let one = |this: &mut Self, pick: std::result::Result<&[u32], usize>, budget: u64| match &spec {
                            None => super::native::component::trial_over(this.db, &name, line, None, None, pick, budget),
                            Some((spec, step)) => super::native::component::trial_over(this.db, &name, line, Some(spec), *step, pick, budget),
                        };
                        // Decision 0245: an order the cut's rows cannot be placed in (it leads with another atom than
                        // the cut, and no one step of it reads those rows) is run over a sample of its own lead's
                        // rows instead. Its work over the line, over its price for the line, is kept as an order
                        // tried, so the next plan's choice reads it; it is not placed here, as it would read the
                        // rows the cut finished again.
                        let own = |this: &mut Self| -> Result<()> {
                            let Some((spec, None)) = &spec else { return Ok(()) };
                            let w = walked_props.iter().find(|(p, _)| same_order(p, o)).map(|p| p.1);
                            let size = |all: usize| match w {
                                Some(w) if w > 0.0 => ((cap as f64 / 2.0 * all as f64 / w) as usize).clamp(TRIAL_LEAST, joins::trial_rows() as usize),
                                _ => sample,
                            };
                            let Some((run, all, picked)) = super::native::component::trial_own(this.db, &name, spec, &size, cap)? else { return Ok(()) };
                            // Two-phase (decision 0233): the background's walk over the lead's rows is the first
                            // phase, a walk over the trial's rows the second, and the trial's work per row is scaled by
                            // their ratio. At its rows alone a stride of 250 of 22,926 rows, past a check passing 2%,
                            // measured the fixture's line at 49,612 rows of work for 388,673.
                            let lead_at = o[0].at;
                            let x2 = match (b.view(lead_at), w) {
                                (Lit::Atom(a), Some(_)) => super::native::component::held(this.db, b.rel(a)).map(|src| {
                                    let db = this.db;
                                    let walker = Walker::new(&b, lead_at, joins::walk_width(), &|rel| super::native::component::held(db, rel), &|x| super::native::component::symbol_id(db, x).ok().flatten());
                                    // The walk starts from lead rows: those of the sample matching the lead's constants and
                                    // repeated variables. Per row sampled, as the trial's work is (the fixture's `kind`
                                    // rows walked as calls measured 15 times the trial).
                                    let largs = b.args(lead_at);
                                    let consts: Vec<(usize, Option<i64>)> = largs.iter().enumerate().filter_map(|(c, t)| match *t {
                                        TermRef::Int(i) => Some((c, Some(i))),
                                        TermRef::Branded(a) => Some((c, Some(a.value()))),
                                        TermRef::Str(x) => Some((c, super::native::component::symbol_id(db, b.v.text(x)).ok().flatten())),
                                        _ => None,
                                    }).collect();
                                    let same: Vec<(usize, usize)> = largs.iter().enumerate().filter_map(|(c, t)| { let v = t.var()?; largs.iter().position(|u| u.var() == Some(v)).filter(|&d| d < c).map(|d| (d, c)) }).collect();
                                    let rows: Vec<Vec<i64>> = picked.iter().map(|&id| (0..largs.len()).map(|c| src.at(id as usize).at(c)).collect::<Vec<i64>>())
                                        .filter(|row| consts.iter().all(|&(c, v)| v == Some(row[c])) && same.iter().all(|&(d, c)| row[d] == row[c])).collect();
                                    if rows.is_empty() { return 0.0 }
                                    walker.work_at(o, &rows, 4 * joins::walk_width()) * rows.len() as f64 / picked.len().max(1) as f64
                                }),
                                _ => None,
                            };
                            let y = run.work as f64 / run.rows.max(1) as f64;
                            let line_work = match (w, x2) {
                                (Some(w), Some(x2)) if x2 > 0.0 => w * y / x2,
                                (Some(_), Some(_)) => f64::NAN,
                                _ => y * all as f64,
                            };
                            let price = this.model.price_for_delta(pf, &b, first, o, (c.done + c.left).max(1) as f64);
                            let name_o = o.iter().map(|s| s.at.to_string()).collect::<Vec<_>>().join(".");
                            if !run.cut && line_work.is_finite() && let Some(p) = price {
                                joins::put_tried(key, order_rules[line], o, line_work / p.max(1.0));
                            }
                            joins::log(|| format!("trials {}: {name_o} over its own lead, {} of its {all} rows: {}{} rows of work, {line_work:.0} over the line (walked {}, {} a row over its rows, priced {}){}", b.head_text(), run.rows, run.work, if run.cut { "+" } else { "" }, w.map_or("-".to_owned(), |w| format!("{w:.0}")), x2.map_or("-".to_owned(), |x| format!("{x:.1}")), price.map_or("-".to_owned(), |p| format!("{p:.0}")), if run.cut { "; cut, not kept" } else { "" }));
                            Ok(())
                        };
                        let run = match &parts {
                            None => {
                                let Some(run) = one(self, Err(sample), budget(least[0], cap))? else { own(self)?; continue };
                                if !run.cut { least[0] = Some(least[0].map_or(run.work, |w| w.min(run.work))); }
                                run
                            }
                            Some(ps) => {
                                let mut got = Vec::new();
                                for (p, (pick, _)) in ps.iter().enumerate() {
                                    let Some(run) = one(self, Ok(pick), budget(least[p], cap / 2))? else { break };
                                    if !run.cut { least[p] = Some(least[p].map_or(run.work, |w| w.min(run.work))); }
                                    got.push(run);
                                }
                                if got.is_empty() { own(self)?; }
                                if got.len() < 2 { continue; }
                                let all = (ps[0].1 + ps[1].1) as f64;
                                // A cut order cannot win: it is weighted by its parts' rows (decision 0221), and
                                // not walked.
                                let cut = got.iter().any(|run| run.cut);
                                let total: f64 = got.iter().zip(ps).enumerate().map(|(p, (run, (pick, size)))| {
                                    let by_rows = *size as f64 * run.work as f64 / run.rows.max(1) as f64;
                                    if cut { return by_rows }
                                    match two_phase(o, p, run, pick, *size) {
                                        Some((est, x1, x2)) => { if same_order(o, &running) { ratios.borrow_mut().push((p, x1, x2)); } est }
                                        None => by_rows,
                                    }
                                }).sum();
                                let per_row = total / all;
                                let rows = got[0].rows + got[1].rows;
                                super::native::component::TrialRun { work: (per_row * rows as f64).round() as u64, rows, done: got[0].done + got[1].done, made: got[0].made + got[1].made, cut: got[0].cut || got[1].cut, steps: got[0].steps.iter().zip(&got[1].steps).map(|(a, b)| (a.0 + b.0, a.1 + b.1)).collect() }
                            }
                        };
                        measured.push((o.clone(), run));
                    }
                    if let Some(ps) = parts.as_ref().filter(|_| !tried.is_empty()) {
                        let rs = ratios.borrow().iter().map(|(p, x1, x2)| format!("{} {x1:.0} / {x2:.0}", if *p == 0 { "heavy" } else { "rest" })).collect::<Vec<_>>().join(", ");
                        joins::log(|| format!("trials {}: stratified, {} of {} rows of heavy keys and {} of {} of the rest; the running order's walks per row, a sample over the trial's rows: {} ({:.1} ms to split, {:.1} ms CPU to walk; {} probes)", b.head_text(), ps[0].0.len(), ps[0].1, ps[1].0.len(), ps[1].1, if rs.is_empty() { "none (cut)".to_owned() } else { rs }, parts_s * 1e3, walk_s.get() * 1e3, walker.as_ref().map_or(0, |w| w.probes())));
                        if let Some(w) = &walker { joins::log(|| format!("walks {}: {}", b.head_text(), w.spent())); }
                    }
                    let rule = order_rules[line];
                    let mut prices = Vec::new();
                    for (o, run) in &measured {
                        let est = self.model.price_for_delta(pf, &b, Some(lead), o, run.rows as f64);
                        if let Some(est) = est {
                            joins::put_trial(key, rule, o, run.work as f64, est, run.cut);
                        }
                        prices.push(est.unwrap_or(f64::NAN));
                        // Decision 0354: a trial that ran to its end gives its order's steps, kept or not.
                        if !run.cut
                            && super::costs::join_steps::enabled()
                            && let Some(x) = self.model.explain_order(pf, &b, first, o)
                            && let Some(steps) = step_samples(&self.model, &b, o, &x, Some(&run.steps), false, "trial")
                        {
                            super::costs::join_steps::submit(steps);
                        }
                    }
                    // Each order's price from statistics alone, without the running order's observations: what a
                    // plan made before the line ran is priced at (decision 0209's measure).
                    let stat_prices: Vec<f64> = if joins::logging() {
                        let obs = self.model.stats.obs.remove(&rule);
                        let p = measured.iter().map(|(o, run)| self.model.price_for_delta(pf, &b, Some(lead), o, run.rows as f64).unwrap_or(f64::NAN)).collect();
                        if let Some(obs) = obs { self.model.stats.obs.insert(rule, obs); }
                        p
                    } else { Vec::new() };
                    // Decision 0209 §5, the split spike (`RERAIL_SPLIT_SPIKE`): the rows left, split by whether
                    // their key is one of a heavy column's values, and every candidate run over each part.
                    if std::env::var_os("RERAIL_SPLIT_SPIKE").is_some() {
                        self.split_spike(&name, line, &b, &distinct, lead, first, &cut_order, &ri, scc, native_rels)?;
                    }
                    // O169: the rows a finished trial made over its sample are the wave's; the order that goes on
                    // reads the rest.
                    let kept = super::native::component::keep_trial_rows(self.db, &name)?;
                    let trials_ms = t_trials.elapsed().as_secs_f64() * 1e3;
                    if !tried.is_empty() { joins::log(|| format!("trials {}{} over {} rows ({} rows of work in all, {trials_ms:.1} ms, {kept} rows kept): {}", if whole { "whole plan " } else { "" }, b.head_text(), measured.first().map_or(0, |m| m.1.rows), measured.iter().map(|m| m.1.work).sum::<u64>(), measured.iter().zip(&prices).enumerate().map(|(i, ((o, t), p))| { let sp = stat_prices.get(i).copied().unwrap_or(f64::NAN); format!("{}{} {}{} (priced {p:.0}, from statistics {sp:.0})", if same_order(o, &running) { "*" } else { "" }, o.iter().map(|s| s.at.to_string()).collect::<Vec<_>>().join("."), t.work, if t.cut { "+" } else { "" }) }).collect::<Vec<_>>().join(", "))); }
                    let mine = measured.iter().find(|(o, _)| same_order(o, &running)).map(|(_, t)| t.clone());
                    let best = measured.iter().filter(|(_, t)| !t.cut).min_by_key(|(_, t)| t.work);
                    if let (Some(mine), Some((best, won))) = (mine, best)
                        && !same_order(best, &running)
                        && (mine.cut || won.work as f64 * joins::trial_margin() < mine.work as f64)
                    {
                        let (best, won) = (best.clone(), won.clone());
                        if place!(best) {
                            super::profile::decision("join trials (decision 0207)", || format!("{}: {} rows of work over the sample, the running order {}{}", b.head_text(), won.work, mine.work, if mine.cut { " or more" } else { "" }));
                            if old.is_none() { old = Some(running); }
                        }
                    }
                }
                if !led && old.as_ref().is_some_and(|o| same_order(o, &probes[k].2)) { old = None; }
                let now = order_now(&orders, &probes, &wholes);
                let per_row = |work: u64, done: u64| if done > 0 { work as f64 / done as f64 } else { f64::INFINITY };
                match handoff[line].take() {
                    None => {
                        if let Some(old) = old.filter(|o| !same_order(o, &now)) {
                            let left = if c.led { c.left } else { c.done + c.left };
                            let est = self.model.price_for_delta(pf, &b, first.or(Some(old[0].at)), &now, left.max(1) as f64).unwrap_or(f64::MAX / 4.0);
                            let limit = joins::slow_factor() * est.max(1.0);
                            joins::log(|| format!("handoff {}: cut at {} of {} rows after {} rows of work; its replacement reads the {left} left, limit {limit:.0}", b.head_text(), c.done, c.done + c.left, c.work));
                            super::profile::decision("join handoff (decision 0206)", || format!("{}: replaced at row {} of {}", b.head_text(), c.done, c.done + c.left));
                            joins::count_handoff(false);
                            handoff[line] = Some(joins::Handoff { old, per_row: per_row(c.work, c.done), limit });
                        }
                    }
                    Some(h) => {
                        let ran = per_row(c.work, c.done);
                        let back = ran > h.per_row;
                        joins::count_handoff(back);
                        let (other, other_rate) = if back { (now.clone(), ran) } else { (h.old.clone(), h.per_row) };
                        if back && !place!(h.old) {
                            return Err(Error::Plan(format!("a join of {} has no native step", b.head_text())));
                        }
                        // Decision 0271: the order that goes on keeps a limit, and the two are compared again at it.
                        let base = counted.get(line).map_or(0, |c| super::profile::plan_work(c)) as f64;
                        let limit = joins::limit_again(if back { h.per_row } else { ran }, base, c.left as f64, c.work as f64);
                        joins::log(|| format!("handoff {}: the replacement was cut too, {ran:.1} rows of work per row against the old order's {:.1}: {} goes on, limit {limit:.0}", b.head_text(), h.per_row, if back { "the old order" } else { "the replacement" }));
                        handoff[line] = Some(joins::Handoff { old: other, per_row: other_rate, limit });
                    }
                }
            }
            // Statistics can now end a chunk early. Bound its wave budget even in a long sequence of such
            // checkpoints; casting an overflowing budget to u32 would accidentally request an unbounded run.
            if cut.is_some() { continue; }
            chunk = (chunk * 2).min(u32::MAX as i64);
        }
        // What each rule's steps produced and each relation holds, for a later chunk's choice of what a relation
        // keeps (decision 0099, `keeps::observe`).
        {
            let ruled: Vec<(u64, &[(u64, u64)])> = order_rules.iter().zip(&counted).map(|(&r, c)| (r, c.as_slice())).collect();
            // Each relation's rows, and per column its distinct values and fanout as the live counts left them.
            let sizes: Vec<keeps::Seen> = rels
                .iter()
                .filter_map(|r| {
                    let rows = *self.model.stats.card.get(*r)?;
                    let width = fields.arity(r).unwrap_or(0);
                    let cols = (0..width).map(|c| (self.model.ndv.at(r, c).copied(), self.model.stats.fan.at(r, c).copied())).collect();
                    Some(keeps::Seen { rel: r.as_str(), rows, cols })
                })
                .collect();
            keeps::observe(&ruled, &sizes, self.model.stats.nodes);
        }
        // What the device did, or why it refused (taken whether or not profiling, so a report never outlives its
        // stratum).
        let device = super::native::component::device_report().or_else(|| {
            not_taken.map(|why| super::native::component::DeviceReport { refused: Some(why.to_owned()), ..Default::default() })
        });
        placement::record(&stratum_key(scc), &placed, device.as_ref().map(|d| d.refused.clone()));
        if profiling {
            let (u, s) = corpus::thread_usage();
            let work: u64 = counted.iter().chain(earlier.iter().flatten().map(|(_, c)| c)).map(|c| super::profile::plan_work(c)).sum();
            super::profile::report::placed(&stratum_key(scc), &placed, &shape, work, u + s - cpu_at, device.as_ref());
        }
        if super::profile::on() {
            let empty = Vec::new();
            let kinds = seeds.iter().map(|&r| format!("seed {}", vs.at(r).head_text())).chain(probes.iter().map(|&(r, ai, _)| format!("join {} (delta at {ai})", vs.at(r).head_text())));
            for ((k, o), kind) in orders.iter().map(|(_, o)| o).chain(probes.iter().map(|p| &p.2)).zip(&line_bodies).map(|(o, b)| Order::new(b, o)).enumerate().zip(kinds) {
                // Each earlier order with its own counts (its time is in the current order's).
                for (was, c) in earlier.get(k).into_iter().flatten() {
                    super::profile::native(format!("{kind}: {}", shown(Order::new(o.b, was)).join(" , ")), c, 0.0, &[]);
                }
                super::profile::native(format!("{kind}: {}", shown(o).join(" , ")), counted.get(k).unwrap_or(&empty), timed.get(k).copied().unwrap_or(0.0), stepped.get(k).map_or(&[][..], Vec::as_slice));
            }
            // After the native plans: they are charged to this stratum's `engine run (joins)` by the current span.
            let _record = super::profile::span(|| "profile record".into());
            let key = stratum_key(scc);
            super::profile::stratum(&key, "native set engine", || {
                format!(
                    "every rule has a native step: component {name}, {} seed joins, {} delta probes, {} gates; inputs read in place, derived relations held in memory",
                    seeds.len(),
                    probes.len(),
                    ngates
                )
            });
            for (k, (kind, rule, x, took, replans)) in explained.iter().enumerate() {
                if let Some(x) = x {
                    super::profile::planned(&key, kind, *rule, x, *took, *replans, Some(counted.get(k).unwrap_or(&empty)), None);
                    super::profile::planned_cpu(&key, kind, x, stepped.get(k).map_or(&[][..], Vec::as_slice));
                }
            }
        }
        if let Some(file) = std::env::var_os("RDL_COMPONENTS") {
            // The component's plans and each step's observed bindings in and out (`set_fixpoint_steps`), appended.
            let mut text = format!("COMPONENT {} rounds={} ms={:.1}\n", rels.iter().map(|r| r.as_str()).collect::<Vec<_>>().join("/"), chunk.trailing_zeros(), started.elapsed().as_secs_f64() * 1e3);
            for l in &spec.lines {
                let (kind, join) = match l {
                    SpecLine::Seed { join, .. } => ("seed", join),
                    SpecLine::Join { join, .. } => ("join", join),
                    _ => continue,
                };
                {
                    let spec = join.to_text();
                    let spec: Vec<String> = spec.split('\n').map(|step| step.split(' ').map(|t| t.strip_prefix("t:").map_or(t.to_owned(), |rest| {
                        let f: Vec<&str> = rest.splitn(4, ':').collect();
                        format!("t:{}:[{}]", f.first().unwrap_or(&""), f.get(1).and_then(|h| unhex(h)).unwrap_or_default())
                    })).collect::<Vec<_>>().join(" ")).collect();
                    text += &format!("  SPEC {kind} {}\n", spec.join(" | "));
                }
            }
            let empty = Vec::new();
            for (k, o) in orders.iter().map(|(_, o)| o).chain(probes.iter().map(|p| &p.2)).zip(&line_bodies).map(|(o, b)| Order::new(b, o)).enumerate() {
                for (was, c) in earlier.get(k).into_iter().flatten() {
                    let work: u64 = super::profile::plan_work(c);
                    text += &format!("  work={work} (an earlier order) {}\n    {}\n", c.iter().map(|(a, b)| format!("{a}:{b}")).collect::<Vec<_>>().join(","), shown(Order::new(o.b, was)).join(" , "));
                }
                let line: Vec<String> = counted.get(k).unwrap_or(&empty).iter().map(|(a, b)| format!("{a}:{b}")).collect();
                let work: u64 = super::profile::plan_work(counted.get(k).unwrap_or(&empty));
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
        // Each line's work, for the next run's planning budget (decision 0122): a seed's under its rule, a probe's
        // under its rule and delta.
        {
            let empty = Vec::new();
            let work = |line: usize| (super::profile::plan_work(counted.get(line).unwrap_or(&empty)) + earlier.get(line).map_or(0, |e| e.iter().map(|(_, c)| super::profile::plan_work(c)).sum())) as f64;
            for (line, key) in keys.iter().enumerate() {
                if let Some(key) = key {
                    Model::ran(*key, work(line));
                }
            }
            // Decision 0354: each joined step's estimate and what it made, for the learned correction of the
            // estimates. A line that ran one order gives its steps by its plan record; a line whose order was
            // replaced (decision 0206) gives each order's steps by its own record and counts: orders the planner did
            // not keep, which a line that kept its order never shows.
            if super::costs::join_steps::enabled() {
                let mut sampled = Vec::new();
                let lines = orders.iter().map(|(_, o)| (None, o.as_slice())).chain(probes.iter().map(|p| (Some(p.1), p.2.as_slice())));
                for (line, (first, order)) in lines.enumerate() {
                    let seed = line < nseeds;
                    if !seed && wholes.contains_key(&(line - nseeds)) {
                        continue;
                    }
                    let (Some(c), Some(b)) = (counted.get(line), line_bodies.get(line)) else { continue };
                    match earlier.get(line).filter(|e| !e.is_empty()) {
                        None => {
                            if let Some(Some(x)) = explained.get(line).map(|e| e.2.as_ref())
                                && let Some(mut steps) = step_samples(&self.model, b, order, x, Some(c), seed, if joins::pricing() { "model" } else { "ran" })
                            {
                                sampled.append(&mut steps);
                            }
                        }
                        Some(e) => {
                            for (o, counts) in e.iter().map(|(o, n)| (o.as_slice(), n.as_slice())).chain(std::iter::once((order, c.as_slice()))) {
                                if let Some(x) = self.model.explain_order(pf, b, first, o)
                                    && let Some(mut steps) = step_samples(&self.model, b, o, &x, Some(counts), seed, "replaced")
                                {
                                    sampled.append(&mut steps);
                                }
                            }
                        }
                    }
                }
                super::costs::join_steps::submit(sampled);
            }
            // Decision 0203 §4: each line's work, its order and its price for the rows it read, for the run's
            // critical path; a line heavy enough is searched wider in the background.
            if joins::on() && PLANS.with(|p| p.borrow().is_some()) {
                let stratum = scc.iter().next().cloned().unwrap_or_default();
                let lines: Vec<(RuleKey, Option<usize>, &[Step])> = orders.iter().map(|(r, o)| (*r, None, o.as_slice())).chain(probes.iter().map(|p| (p.0, Some(p.1), p.2.as_slice()))).collect();
                let mut runs = Vec::new();
                for (line, (r, first, order)) in lines.into_iter().enumerate() {
                    let (Some(Some(key)), Some(c)) = (keys.get(line).copied(), counted.get(line)) else { continue };
                    // Every order the line ran: an order handed off (decision 0206) did its share of the line's work.
                    let w = work(line);
                    if w < SEARCH_WORK {
                        continue;
                    }
                    let rule = order_rules.get(line).copied().unwrap_or(0);
                    let est = if w >= joins::slow_rows().min(1e4) || line_log().is_some() {
                        let b = vs.at(r);
                        let delta = c.first().map_or(0, |s| s.1) as f64;
                        self.model.price_for_delta(pf, &b, first, order, delta).unwrap_or(0.0)
                    } else {
                        0.0
                    };
                    if w >= joins::slow_rows() && joins::wants_request(&key) {
                        self.request_search(r, first, rule, key, w, Some((&name, line, order)))?;
                    }
                    runs.push(joins::LineRun { stratum: stratum.clone(), key, rule, order: joins::steps(order), work: w, est, wall: walled.get(line).copied().unwrap_or(0.0) });
                    if let Some(log) = line_log() {
                        let b = vs.at(r);
                        let text = format!("{} :- {}", b.show_head(), (0..b.len()).map(|i| b.show(i)).collect::<Vec<_>>().join(", "));
                        let shown = order.iter().map(|s| s.at.to_string()).collect::<Vec<_>>().join(".");
                        let wall = walled.get(line).copied().unwrap_or(0.0);
                        use std::io::Write as _;
                        let _ = writeln!(log.lock(), "{stratum}\t{first:?}\t{shown}\t{w:.0}\t{est:.0}\t{wall:.6}\t{:x}\t{text}", key as u64);
                    }
                }
                lines_push(runs);
            }
        }
        {
            let _close = super::profile::span(|| "engine close".into());
            self.fx().close(&name)?;
            for r in &rels { self.model.stats.delta_rows.remove(r.as_str()); }
        }
        if let Some(capture) = lab {
            super::native::component::lab::capture_outputs(&self.db, capture);
        }
        self.native_rels.extend(scc.iter().cloned());
        Ok(Native::Ran)
    }

    /// Runs a one-relation flat-lattice component natively (sqlite-ext `lattice_fixpoint`, T's `native_fixpoint`)
    /// if every rule is a seed (no recursive atom), a copy (`R($n, $v) :- ..., R($x, $v)`) or a binary transfer
    /// (`R($n, @flat_bin($o, $a, $b)) :- ..., R($l, $a), R($r, $b)`). The rest of each body is non-recursive, so SQL
    /// evaluates it once, planned as usual, into the component's seed and edge tables. Returns false to fall back.
    fn native_fixpoint(&mut self, scc: &BTreeSet<String>, rules: &[RuleKey]) -> Result<bool> {
        let versions = Arc::clone(&self.versions);
        let pf = versions.whole(self.version);
        let vs = Arc::clone(&self.versions);
        let fields = vs.whole(self.version);
        let native = &self.model.inputs.native;
        let [rel] = scc.iter().collect::<Vec<_>>()[..] else { return Ok(false) };
        if !fields.lattice(rel) || fields.width(rel) != 2 {
            return Ok(false);
        }
        let mut parts: Vec<(&str, Part<'_>)> = Vec::new();
        for &k in rules {
            let (v, r) = vs.rule_at(k);
            let lits: Vec<super::program_image::LitRef<'_>> = r.body().collect();
            let rec: Vec<AtomRef<'_>> = lits.iter().filter_map(|l| match l.view() {
                Lit::Atom(a) if v.rel_name(a.rel()) == rel.as_str() => Some(a),
                _ => None,
            }).collect();
            let rest: Vec<usize> = (0..lits.len()).filter(|&i| !matches!(lits[i].view(), Lit::Atom(a) if v.rel_name(a.rel()) == rel.as_str())).collect();
            if rec.iter().any(|a| a.neg()) {
                return Ok(false);
            }
            fn arg(a: AtomRef<'_>, c: usize) -> TermRef<'_> {
                a.args().nth(c).expect("a column")
            }
            let rvals: BTreeSet<VarSlot> = rec.iter().filter_map(|&a| arg(a, 1).var()).collect();
            if rvals.len() != rec.len() || rest.iter().any(|&i| rvals.iter().any(|&s| body::lit_vars(lits[i]).contains(s))) {
                return Ok(false);
            }
            let mut head = r.head().args();
            let (key, val) = (head.next().expect("a key"), head.next().expect("a value"));
            let kinds: Vec<usize> = (0..lits.len())
                .filter(|&i| matches!(lits[i].view(), Lit::Atom(a) if !a.neg() && v.rel_name(a.rel()) == "kind" && key.var().is_some() && arg(a, 0).var() == key.var()))
                .collect();
            let mut val_vars = VarSet::new();
            body::term_vars(val, &mut val_vars);
            let kseed = match (key.var(), kinds.as_slice()) {
                (Some(kn), [ki]) if rec.is_empty() && native.contains("kseed") => {
                    let Lit::Atom(ka) = lits[*ki].view() else { unreachable!() };
                    arg(ka, 1).var().is_some() && (0..lits.len()).all(|i| i == *ki || !body::lit_vars(lits[i]).contains(kn)) && !val_vars.contains(kn)
                }
                _ => false,
            };
            let t = |t| term_text(v, r, t);
            let part = |rel: String, head, lits: Vec<usize>| Part { rel, head, pre: Vec::new(), from: (v, r), lits };
            if kseed {
                // A per-kind default (`R($n, v) :- kind($n, $k), <tests of $k>`): evaluated over the distinct kinds,
                // and the native side seeds every node of those kinds from the tape. No row per node.
                let Lit::Atom(ka) = lits[kinds[0]].view() else { unreachable!() };
                let k = t(arg(ka, 1));
                let mut seed = part(format!("lfx_{rel}_kseed"), vec![k.clone(), t(val)], (0..lits.len()).filter(|&i| i != kinds[0]).collect());
                seed.pre.push(("rdl_kinds", vec![k]));
                parts.push(("kseed", seed));
            } else if rec.is_empty() {
                parts.push(("seed", part(format!("lfx_{rel}_seed"), vec![t(key), t(val)], (0..lits.len()).collect())));
            } else if rec.len() == 1 && val.var().is_some() && val.var() == arg(rec[0], 1).var() {
                parts.push(("copy", part(format!("lfx_{rel}_copy"), vec![t(key), t(arg(rec[0], 0))], rest)));
            } else if let TermRef::Call(f, a) = val
                && rec.len() == 2
                && v.text(f) == "flat_bin"
                && let [op, l, right, ..] = a.collect::<Vec<_>>()[..]
                && l.var().is_some()
                && l.var() == arg(rec[0], 1).var()
                && right.var().is_some()
                && right.var() == arg(rec[1], 1).var()
            {
                parts.push(("bin", part(format!("lfx_{rel}_bin"), vec![t(key), t(op), t(arg(rec[0], 0)), t(arg(rec[1], 0))], rest)));
            } else {
                return Ok(false);
            }
        }
        let schema = self.model.inputs.facts.trim_end_matches('.').to_owned();
        let has_kseed = parts.iter().any(|(k, _)| *k == "kseed");
        if has_kseed && !self.views.contains("rdl_kinds") {
            self.db.execute(&format!("CREATE TEMP VIEW rdl_kinds AS SELECT kind AS c0 FROM {}stat_kind", self.model.inputs.stats), [])?;
            Arc::make_mut(&mut self.views).insert("rdl_kinds".to_owned());
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
        // Each part a rule of its own, planned as any rule is.
        let made = made_parts(parts.iter().map(|(_, p)| p))?;
        for (k, (_, part)) in parts.iter().enumerate() {
            let b = body::nth(&made, k);
            let (order, _, _) = self.model.plan_rule(pf, &b, None)?;
            self.ensure_indexes(Order::new(&b, &order))?;
            let select = self.model.sql_select(pf, &b, &order, None, None)?;
            self.db.execute(&format!("INSERT INTO {} {select}", crate::code_rule_dsl::logic::sql::ident(&part.rel)), [])?;
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
            Arc::make_mut(&mut self.views).insert(rel.clone());
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
    fn native_set_fixpoint(&mut self, scc: &BTreeSet<String>, rules: &[RuleKey]) -> Result<bool> {
        let versions = Arc::clone(&self.versions);
        let pf = versions.whole(self.version);
        let versions = Arc::clone(&self.versions);
        let fields = versions.whole(self.version);
        if scc.iter().any(|r| fields.lattice(r)) {
            return Ok(false);
        }
        let rels: Vec<&String> = scc.iter().collect();
        let ri: BTreeMap<String, usize> = rels.iter().enumerate().map(|(i, r)| ((*r).clone(), i)).collect();
        let name = format!("sfx{}_{}", self.made.len(), rels[0]);
        let mut seed_plans = Vec::new();
        let vs = Arc::clone(&self.versions);
        for &r in rules.iter().filter(|&&r| recursive(&vs.at(r), scc).is_empty()) {
            let b = vs.at(r);
            let plan = self.model.plan_rule(pf, &b, None)?;
            seed_plans.push((vs.head_at(r), b, plan));
        }
        for r in &rels {
            let rows: f64 = seed_plans.iter().filter(|(h, _, _)| *h == r.as_str()).map(|(_, _, p)| p.2).sum();
            let card = prior_stats(&mut self.model, pf, r).map_or(rows.max(1.0), |n| n.max(rows.max(1.0)));
            self.model.stats.card.insert((*r).clone(), card);
        }
        // The type the first positive atom of literals `rest` of `v`'s rule `r` that names variable `x` gives it.
        let var_type = |v: &Version, r: super::program_image::RuleRef<'_>, x: &str, rest: &[usize]| -> Option<char> {
            let lits: Vec<_> = r.body().collect();
            rest.iter().find_map(|&i| match lits[i].view() {
                Lit::Atom(a) if !a.neg() => a
                    .args()
                    .position(|t| t.var().is_some_and(|s| v.text(r.var(s)) == x))
                    .map(|c| sym_char(&self.model.inputs, fields.types(v.rel_name(a.rel()))[c])),
                _ => None,
            })
        };
        let tables: HashMap<String, (String, String)> = rels.iter().map(|r| ((*r).clone(), (format!("sfx_f_{r}"), format!("sfx_d_{r}")))).collect();
        let types = |r: &str| fields.types(r).iter().map(|t| sym_char(&self.model.inputs, t)).collect::<String>();
        let mut lines: Vec<SpecLine> = rels.iter().map(|r| SpecLine::Rel { name: (*r).clone(), types: types(r), start: Start::Rows }).collect();
        // Each edge table: its name, its width, the plan of the rest that fills it, and the rest as a version.
        let mut edges: Vec<(String, usize, Vec<Step>, Arc<Version>)> = Vec::new();
        let mut probes: Vec<(RuleKey, Vec<Step>, usize, usize)> = Vec::new();
        for (j, &k) in rules.iter().enumerate() {
            let rec = recursive(&vs.at(k), scc);
            if rec.is_empty() {
                continue;
            }
            let (v, r) = vs.rule_at(k);
            let lits: Vec<super::program_image::LitRef<'_>> = r.body().collect();
            if let [(ai, _)] = rec[..]
                && let Lit::Atom(a) = lits[ai].view()
            {
                let rest: Vec<usize> = (0..lits.len()).filter(|&i| i != ai).collect();
                let name = |s: VarSlot| v.text(r.var(s));
                let names = |vars: &VarSet| vars.iter().map(name).map(str::to_owned).collect::<BTreeSet<String>>();
                // The spec line and edge plan of a linear rule, or the reason it is a probe.
                let edge_rule = || -> Result<std::result::Result<(SpecLine, Option<(String, usize, Vec<Step>, Arc<Version>)>), String>> {
                    let mut rv = VarSet::new();
                    rest.iter().for_each(|&i| rv.extend(&body::lit_vars(lits[i])));
                    let (mut hv, mut av) = (VarSet::new(), VarSet::new());
                    r.head().args().for_each(|t| body::term_vars(t, &mut hv));
                    a.args().for_each(|t| body::term_vars(t, &mut av));
                    let (restvars, hvars, avars) = (names(&rv), names(&hv), names(&av));
                    let key: Vec<String> = avars.intersection(&restvars).cloned().collect();
                    let ecols: Vec<String> = hvars.intersection(&restvars).filter(|v| !avars.contains(*v)).cloned().collect();
                    if hvars.iter().any(|v| !avars.contains(v) && !restvars.contains(v)) {
                        return Ok(Err("head variable from nowhere".to_owned()));
                    }
                    let slot: HashMap<&str, usize> = avars.iter().chain(&ecols).enumerate().map(|(i, v)| (v.as_str(), i)).collect();
                    let mut pat = Vec::new();
                    for t in a.args() {
                        pat.push(match t {
                            TermRef::Wild => Tok::Wild,
                            TermRef::Var(s) => Tok::Slot(slot[name(s)]),
                            TermRef::Int(_) | TermRef::Branded(_) | TermRef::Str(_) => const_tok(v, t).expect("a constant"),
                            _ => return Ok(Err("pattern term".to_owned())),
                        });
                    }
                    let mut head = Vec::new();
                    for t in r.head().args() {
                        head.push(match t {
                            TermRef::Var(s) => HeadTok::Tok(Tok::Slot(slot[name(s)])),
                            TermRef::Int(_) | TermRef::Branded(_) | TermRef::Str(_) => HeadTok::Tok(const_tok(v, t).expect("a constant")),
                            TermRef::Call(f, xs) if v.text(f) == "cat" => match xs.collect::<Vec<_>>()[..] {
                                [TermRef::Var(x0), TermRef::Var(x1), ..] => HeadTok::Cat(slot[name(x0)], slot[name(x1)]),
                                _ => return Ok(Err("head term".to_owned())),
                            },
                            _ => return Ok(Err("head term".to_owned())),
                        });
                    }
                    let mut edge = None;
                    let src = if !rest.is_empty() {
                        if !rest.iter().any(|&i| matches!(lits[i].view(), Lit::Atom(x) if !x.neg())) {
                            return Ok(Err("rest has no atom".to_owned()));
                        }
                        // A rest sharing no variable (a constant test) has no edge table to keep.
                        if key.is_empty() && ecols.is_empty() {
                            return Ok(Err("rest binds no variable".to_owned()));
                        }
                        let ehead = format!("sfx_{}_{j}", v.rel_name(r.head().rel()));
                        let cols: Vec<&str> = key.iter().chain(&ecols).map(String::as_str).collect();
                        // The rest, a rule of its own under the edge table's head.
                        let made = made_parts([&Part { rel: ehead.clone(), head: cols.iter().map(|c| super::program_image::T::Var(c)).collect(), pre: Vec::new(), from: (v, r), lits: rest.clone() }])?;
                        let Ok((order, work, _)) = self.model.plan_rule(pf, &body::nth(&made, 0), None) else {
                            return Ok(Err("rest cannot be planned alone".to_owned()));
                        };
                        if work > self.model.stats.nodes {
                            return Ok(Err(format!("rest costs {work:.0} > {} nodes alone", self.model.stats.nodes)));
                        }
                        let delta = self.model.plan_rule(pf, &vs.at(k), Some(ai))?.1;
                        if work > delta {
                            return Ok(Err(format!("edges cost {work:.0} > one delta pass {delta:.0}")));
                        }
                        let t: String = cols.iter().map(|x| var_type(v, r, x, &rest).unwrap_or('i')).collect();
                        let src = Some((ehead.clone(), t));
                        edge = Some((ehead, cols.len(), order, made));
                        src
                    } else {
                        None
                    };
                    let vs = |vs: &[String]| vs.iter().map(|v| slot[v.as_str()]).collect::<Vec<_>>();
                    let line = SpecLine::Edge(EdgeSpec { head: ri[v.rel_name(r.head().rel())], src: ri[v.rel_name(a.rel())], edge: src, key: vs(&key), ecols: vs(&ecols), pat, out: head });
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
                let (order, _, _) = self.model.plan_rule(pf, &vs.at(k), Some(ai))?;
                let at = order.iter().position(|s| s.at == ai).expect("the delta atom");
                probes.push((k, order, at, ai));
            }
        }
        for (h, b, (order, _, _)) in &seed_plans {
            self.ensure_indexes(Order::new(b, order))?;
            let select = self.model.sql_select(pf, b, order, None, None)?;
            self.insert_rows(h, &select)?;
        }
        for (ehead, width, order, made) in &edges {
            let b = body::nth(made, 0);
            self.ensure_indexes(Order::new(&b, order))?;
            let cols = (0..*width).map(|i| format!("c{i}")).collect::<Vec<_>>().join(", ");
            self.db.execute_batch(&format!("DROP TABLE IF EXISTS {0}; CREATE TEMP TABLE {0}({cols});", crate::code_rule_dsl::logic::sql::ident(ehead)))?;
            if *width > 0 {
                let select = self.model.sql_select(pf, &body::nth(made, 0), order, None, None)?;
                self.db.execute(&format!("INSERT INTO {} {select}", crate::code_rule_dsl::logic::sql::ident(ehead)), [])?;
            }
        }
        let join = self.model.inputs.native.contains("join");
        let none = BTreeSet::new();
        for (r, order, at, ai) in &probes {
            let b = vs.at(*r);
            let rel = b.rel(b.plain_atom(*ai).expect("the probe's recursive atom"));
            if join && let Some(spec) = self.whole_join_spec(&b, order, Some(*at), &ri, scc, &none) {
                lines.push(SpecLine::Join { head: ri[b.rel(b.head())], delta: ri[rel], join: spec, whole: None });
                continue;
            }
            let reads: Vec<String> = (0..b.len()).filter_map(|i| b.atom(i)).map(|a| b.rel(a).to_owned()).collect();
            self.ensure(&reads)?;
            self.ensure_indexes(Order::new(&b, order))?;
            let sql = self.model.sql_select(pf, &b, order, Some(*at), Some(&tables))?;
            lines.push(SpecLine::Probe { head: ri[b.rel(b.head())], delta: ri[rel], sql });
        }
        self.fx().open(&ComponentSpec { name: name.clone(), lines })?;
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
            let text_symbols = |i: usize, c: usize| self.model.inputs.atoms.is_none() && fields.types(rels[i]).get(c) == Some(&"sym");
            let live = self.fx().stats(&name, &text_symbols)?;
            for (r, l) in rels.iter().zip(live) {
                live_stats(&mut self.model, r, l);
            }
            for k in 0..probes.len() {
                let (r, ai) = (probes[k].0, probes[k].3);
                let (new, _, _) = self.model.plan_rule(pf, &vs.at(r), Some(ai))?;
                if same_order(&new, &probes[k].1) {
                    continue;
                }
                let nat = new.iter().position(|s| s.at == ai).expect("the delta atom");
                let spec = if join { self.whole_join_spec(&vs.at(r), &new, Some(nat), &ri, scc, &none) } else { None };
                let new_body = match spec {
                    Some(spec) => Reprobe::Join(spec),
                    None => {
                        let b = vs.at(r);
                        self.ensure_indexes(Order::new(&b, &new))?;
                        Reprobe::Sql(self.model.sql_select(pf, &b, &new, Some(nat), Some(&tables))?)
                    }
                };
                self.fx().reprobe(&name, k, &new_body)?;
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

/// Evaluates program `v` over the facts database at `facts` (T's `run_sql`, pure SQL mode): views for projections of
/// one input, typed tables for the rest, then each stratum planned from the statistics and the counts of the strata
/// before it, non-recursive ones once and recursive ones semi-naively.
pub fn run_sql(v: Arc<crate::code_rule_dsl::logic::program_image::Version>, facts: &Path) -> Result<Outcome> {
    let db = Connection::open_in_memory()?;
    db.execute("ATTACH ?1 AS f", [format!("file:{}?mode=ro", facts.display())])?;
    // Map the facts file, so probes of the input relations' indexes are memory reads, and keep ephemeral b-trees
    // (DISTINCT, sorts, materialized subqueries) in memory.
    db.execute_batch("PRAGMA f.mmap_size = 4294967296; PRAGMA temp_store = MEMORY;")?;
    run(&body::Placed::whole(v), &db, Inputs::file(), None, Hooks::default())
}

/// `run_sql` with T's `RDL_NATIVE` modes (`native_modes`). `load` registers T's extension functions on the run's
/// connection before anything runs, as T's `run_sql` loads `RDL_EXT`.
pub fn run_sql_native(
    v: Arc<crate::code_rule_dsl::logic::program_image::Version>,
    facts: &Path,
    modes: &str,
    load: impl FnOnce(&Connection) -> rusqlite::Result<()>,
) -> Result<Outcome> {
    let db = Connection::open_in_memory()?;
    db.execute("ATTACH ?1 AS f", [format!("file:{}?mode=ro", facts.display())])?;
    db.execute_batch("PRAGMA f.mmap_size = 4294967296; PRAGMA temp_store = MEMORY;")?;
    load(&db)?;
    run(&body::Placed::whole(v), &db, Inputs { native: native_modes(modes), ..Inputs::file() }, None, Hooks::default())
}

/// Evaluates a lowered program (decision 0009) over the base relations bound on `db` (`facts::vtab::syntax::bind`),
/// with the planner's statistics in the attached schema `stats`. `db` is a connection to the content store: the
/// symbols the run makes are the store's atoms (pending until a flush, decision 0049), and derived relations are
/// `temp` tables, dropped by the next run. Outputs are decoded: names and kinds as text, node ids as the vtabs'.
pub fn run_atoms(lowered: &lower::Lowered, db: &Connection, stats: &str) -> Result<Outcome> {
    temp_in_memory(db)?;
    connection::run(db,2,||super::native::register_symbols(db))?;
    let inputs = Inputs {
        facts: "main.".to_owned(),
        stats: format!("{stats}."),
        atoms: Some(lowered.ops),
        native: BTreeSet::new(),
        batch: HashMap::new(),
        given: None,
        kept: BTreeSet::new(),
    };
    let out = run(&lowered.placed()?, db, inputs, Some(lowered.lowering()), Hooks::default());
    super::native::flush_symbols(db)?;
    out
}

/// `run_atoms` with T's native modes (O13): the components a mode takes run in `logic::native` (registered on
/// `db`) over atoms, the rest in SQL. T's `nearest` and `kseed` modes read a per-run copy of the tree
/// (`tape_load`), which is not kept: `nearest` is the transducer's (`preparation::nearest`), so those two
/// modes are dropped from `modes`.
pub fn run_atoms_native(lowered: &lower::Lowered, db: &Connection, stats: &str, modes: &str) -> Result<Outcome> {
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
pub fn run_atoms_native_over(
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
pub struct Hooks<'a> {
    /// A kept relation no later stratum reads: the caller writes it where it goes, and the run drops it
    /// (`Outcome::handed`); one the caller does not take (`false`) is held to the run's end as before.
    pub hand_off: Option<&'a mut dyn FnMut(&str) -> Result<bool>>,
    /// Inputs a stratum is about to read for the first time: the caller binds their rows now.
    pub load: Option<&'a mut dyn FnMut(&[String]) -> Result<()>>,
    /// Inputs no later stratum reads: the caller may drop their rows (the run has dropped its views of them).
    pub unload: Option<&'a mut dyn FnMut(&[String]) -> Result<()>>,
    /// Answers the requests a component makes, inside the fixpoint (decision 0065 §2a).
    pub answer: Option<Answer<'a>>,
    /// The run's text tests (`preparation::text::TextRelations`): a test over strings the run answers (`front::Native::TextTest`)
    /// tests them with every other test at once, each string once a run, as the batches do. Without it the run
    /// makes its own from the program's tests.
    pub texts: Option<Arc<super::strings::Vocabulary>>,
    /// The program normalized (`lower::Lowered::normalized`), for `apart` as `run` decides it: read, not made again.
    pub normalized: Option<Arc<body::Normalized>>,
    /// Posts the requests a component makes, their answers taken as they come (decision 0114 §3); used where the
    /// strata loop retires from a scoreboard, `answer` elsewhere.
    pub asking: Option<&'a mut dyn Asking>,
    /// The strata an earlier run of the program kept (decision 0077 §4): a stratum whose reads are unchanged is not run.
    pub strata: Option<&'a mut dyn KeptStrata>,
    /// A session's maintained run, driven by its deltas (decision 0117 step 3): the program is the base one, and
    /// each stratum is maintained, if a read of it changed, by a form made when the run reaches it.
    pub maintain: Option<&'a mut dyn Maintain>,
}

/// The driver's side of a maintained run (decision 0117 step 3): what was kept, and each reached stratum's form.
pub trait Maintain {
    /// Relation `rel`'s rows as the last run kept them, read in place, if they are in memory: a stratum no changed read
    /// reaches holds them.
    fn kept(&mut self, rel: &str) -> Option<Arc<super::native::component::InRel>>;
    /// Whether `rel`'s kept rows are known without reading them (in memory, or stored: `need` reads them).
    fn keeps(&self, rel: &str) -> bool;
    /// The kept rows of `rels` read where they are stored, and those of them that are the part's clean reads bound,
    /// with their statistics in `model` (decision 0117 step 4: what the strata a level reaches read, and no more).
    fn need(&mut self, rels: &[String], model: &mut Model) -> Result<()>;
    /// Whether the caller reads `rel` after the run (besides the outputs): a relation not read is then held.
    fn at_end(&self, rel: &str) -> bool;
    /// Whether `rel`'s deltas (`rel__minus`, `rel__plus`) are relations of the run now: given, or made by its form.
    fn has_delta(&self, rel: &str) -> bool;
    /// `rel` changed and has no deltas (derived again, and its old rows not kept): its readers derive again.
    fn no_delta(&mut self, rel: &str);
    /// `rel`'s delta was made by the run itself (`reach_delta`): its readers read `rel__minus` and `rel__plus`.
    fn delta_made(&mut self, rel: &str);
    /// The run's end: how many strata were reached, of those the driver walked.
    fn walked(&mut self, reached: usize, quiet: usize, of: usize);
    /// The inputs that changed with a delta not empty, or with none: the walk's first moved reads.
    fn seeds(&mut self) -> Vec<String>;
    /// Stratum `scc` was maintained by the evaluator's mode (decision 0364 stage 3), not by a form: each relation's
    /// delta is its versions (held as `rel__minus`, `rel__plus`), and it counts as maintained (`recursive`: by levels).
    fn mode_ran(&mut self, scc: &BTreeSet<String>, recursive: bool);
    /// Input `rel`'s rows gone and added, `[minus, plus]`, read in place: the maintain mode's versions of it.
    fn given(&self, rel: &str) -> Option<[Arc<dyn crate::code_rule_dsl::facts::vtab::Source>; 2]>;
    /// Input `rel`'s rows as the last run had them, read in place (the maintain mode's `old` of an input).
    fn old_input(&self, rel: &str) -> Option<Arc<dyn crate::code_rule_dsl::facts::vtab::Source>>;
}



/// Holds `rel`'s kept rows `o` as the run's relation, with its cardinality (a quiet stratum's, decision 0117 step 3).
fn hold_quiet(run: &mut Run, f: Fields<'_>, db: &Connection, rel: &String, o: Arc<super::native::component::InRel>) {
    let card = quiet_card(&mut run.model, &run.quiet, f, rel, &o);
    run.model.stats.card.insert(rel.clone(), card);
    super::native::component::hold_as(db, rel, o);
    run.native_rels.insert(rel.clone());
}

/// The rows a relation held as kept (`o`) is planned at, with its columns' statistics: its own or its old rows'
/// where measured (`prior_stats`); else the statistics the engine kept of its rows as they came
/// (`InRel::kept_stats`), read when a plan first prices it (decision 0101 §4). Without them each column read as one
/// value, and every probe of it as all its rows: `aknown__sup4__del` scanned `aleaf` (12,779 rows) before its 3-row
/// delta (t39). Only where every column's were kept: the planner reads a relation's columns all or none.
fn quiet_card(model: &mut Model, tally: &QuietTally, f: Fields<'_>, rel: &str, o: &Arc<super::native::component::InRel>) -> f64 {
    let seeded = (o.rows() as f64).max(1.0);
    if let Some(n) = prior_stats(model, f, rel) {
        return n.max(seeded);
    }
    if !model.stats.lazy.0.contains_key(rel)
        && let Some(width) = f.arity(rel)
        && let Some(kept) = o.kept_stats((0..width).map(|c| sql_col(rel, c)).collect())
    {
        let cols: Vec<(String, [i64; 4])> = kept.statistics(&|_| true).into_iter().map(|(c, rows, ndv, sq, sqd)| (c, [rows, ndv, sq, sqd])).collect();
        if cols.len() == width {
            model.stats.lazily(rel, o.rows(), LazyStats::new(move || (cols, Vec::new(), ())));
        }
    }
    if !model.stats.lazy.0.contains_key(rel) {
        tally.unmeasured.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    seeded
}
/// A stratum of a maintained run, as its level's start placed it (decision 0117 step 3).
enum Slot {
    /// Run as the base program's stratum (its reads unchanged and its rows not kept), or not placed yet.
    Base,
    /// Held as kept.
    Quiet,
    /// Maintained by the evaluator's mode (decision 0364 stage 3): its rules, the reads that changed, its kept rows.
    Mode { rules: Vec<RuleKey>, moved: BTreeSet<String>, olds: BTreeMap<String, Arc<super::native::component::InRel>>, ins: ModeInputs },
}


/// What an earlier run of a program kept of its strata, and what this run gives back (decision 0077 §4).
pub trait KeptStrata {
    /// Each relation of the program the run evaluates, by its identity (`identities`): what was kept of a relation
    /// is of the relation of that identity.
    fn program(&mut self, identities: &HashMap<String, [u8; 32]>);
    /// Input `rel`'s digest in this run (`None`: not known).
    fn input(&mut self, rel: &str) -> Option<u64>;
    /// The digest relation `rel` had when it was kept.
    fn was(&self, rel: &str) -> Option<u64>;
    /// The digest `read` had when the kept `rel` was derived from it (`None`: `rel` did not read it, or is not kept).
    fn read_was(&self, rel: &str, read: &str) -> Option<u64>;
    /// Whether the earlier run's rows of `rel` are kept.
    fn has_rows(&self, rel: &str) -> bool;
    /// The earlier run's rows of `rel`: its columns' types as kept, and its rows as a relation the engine holds under
    /// `types` (its own), bound where they lie (decision 0401 F3).
    fn rows(&mut self, rel: &str, types: &[u8]) -> Option<(Vec<u8>, Arc<super::native::component::InRel>)>;
    /// This run derived `rel`: its digest, and its rows (types, flat, count), with the digest each relation its
    /// stratum read had; `None` when SQL derived it, or a read's digest is not known.
    fn ran(&mut self, rel: &str, got: Option<(u64, Vec<u8>, Vec<i64>, usize, Vec<(String, u64)>)>);
    /// The run's end: the strata it evaluated, of all.
    fn counted(&mut self, evaluated: usize, of: usize);
}

/// The relations a stratum's rules read: atoms (an aggregate's too), `node` and `child` for a `nearest` climb, a
/// native operator's inputs.
fn stratum_reads(f: Fields<'_>, vs: &body::Versions, scc: &BTreeSet<String>, rules: &[RuleKey]) -> BTreeSet<String> {
    let mut read: BTreeSet<&str> = BTreeSet::new();
    let mut nearest = false;
    for &r in rules {
        let (v, rule) = vs.rule_at(r);
        for l in rule.body() {
            if let Some(a) = l.atom() {
                read.insert(v.rel_name(a.rel()));
            }
            nearest |= matches!(l.view(), Lit::Nearest { .. });
        }
    }
    let mut out: BTreeSet<String> = read.into_iter().map(str::to_owned).collect();
    if nearest {
        out.extend(["node".to_owned(), "child".to_owned()]);
    }
    for rel in scc {
        if let Some(native) = f.native(rel) {
            out.extend(native.reads.iter().map(|&r| f.rel_name(r).to_owned()));
        }
    }
    // A produced relation reads its request (decision 0065 §2a): it is the request's answers.
    for &(req, p) in f.answers() {
        if scc.contains(f.rel_name(p)) {
            out.insert(f.rel_name(req).to_owned());
        }
    }
    out
}

fn retainable_stratum(f: Fields<'_>, vs: &body::Versions, scc: &BTreeSet<String>, rules: &[RuleKey], views: &HashSet<String>) -> bool {
    !rules.is_empty()
        && !scc.iter().any(|r| f.native(r).is_some() || f.lattice(r) || views.contains(r) || f.produced(r))
        && !rules.iter().any(|&r| vs.rule_at(r).1.body().any(|l| matches!(l.view(), Lit::Ast { .. })))
}

/// Old frames could record a nonempty answer digest beside an unmaterialized, empty payload.
fn answer_read(p: &str) -> String {
    format!("answer:v2:{p}")
}

/// A set of answered rows' digest (`rows_digest`, of no rows when none), of relation `p`'s content (`Cells`).
fn answers_digest(f: Fields<'_>, db: &Connection, p: &str, rows: Option<&BTreeSet<Vec<i64>>>) -> u64 {
    let Some(rows) = rows.filter(|r| !r.is_empty()) else { return rows_digest(&Cells::plain(0), &[], 0) };
    let width = rows.iter().next().map_or(0, Vec::len);
    let flat: Vec<i64> = rows.iter().flatten().copied().collect();
    let atoms = crate::content_store::atoms::Atoms::for_connection(db);
    rows_digest(&Cells::new(&atoms, &stored_types(f, p, vec![b'i'; width])), &flat, rows.len())
}

/// A stratum's reads with its native step's answers (one digest per produced relation), when
/// it makes requests of its own.
fn with_answers(f: Fields<'_>, db: &Connection, scc: &BTreeSet<String>, reads: Option<Vec<(String, u64)>>, log: Option<BTreeMap<String, BTreeSet<Vec<i64>>>>) -> Option<Vec<(String, u64)>> {
    let mut reads = reads?;
    for req in scc {
        let ps = f.answers_of(req);
        if !ps.iter().any(|p| scc.contains(*p)) {
            continue;
        }
        for p in ps {
            reads.push((answer_read(p), answers_digest(f, db, p, log.as_ref().and_then(|l| l.get(p)))));
        }
    }
    for p in scc.iter().filter(|p| f.request_of(p).is_some_and(|q| !scc.contains(q))) {
        reads.push((answer_read(p), answers_digest(f, db, p, log.as_ref().and_then(|l| l.get(p)))));
    }
    Some(reads)
}

/// After a stratum ran: each of its relations' digest and rows, given to `ks` with what the stratum read
/// (`reads`, each with its digest then), and its digest to this run's `fresh`.
fn kept_strata_ran(ks: &mut dyn KeptStrata, db: &Connection, run: &Run, scc: &BTreeSet<String>, reads: Option<&Vec<(String, u64)>>, fresh: &mut HashMap<String, Option<u64>>) {
    if scc.contains("source_language_classification") {
        crate::code_rule_dsl::prof::count("source_language.evaluations", 1);
    }
    let mut members = Vec::new();
    let f = run.fields();
    for r in scc.iter().filter(|r| !run.views.contains(*r)) {
        let got = if run.native_rels.contains(r) {
            super::native::component::kept_rows(db, r)
        } else if f.produced(r) && !f.is_input(r) {
            // SQL holds the answers when the relation is outside the native component.
            produced_rows(db, run, r)
        } else {
            None
        };
        let atoms = crate::content_store::atoms::Atoms::for_connection(db);
        let digest = got.as_ref().map(|(types, rows, n)| rows_digest(&Cells::new(&atoms, &stored_types(f, r, types.clone())), rows, *n));
        fresh.insert(r.clone(), digest);
        members.push((r.clone(), digest));
        ks.ran(r, got.zip(reads).map(|((types, rows, n), reads)| (digest.expect("rows give a digest"), stored_types(f, r, types), rows, n, reads.clone())));
    }
    for v in scc.iter().filter(|r| run.views.contains(*r)) {
        fresh.insert(v.clone(), view_digest(reads.map(Vec::as_slice), &members));
    }
}

/// A produced relation's rows as its table holds them, typed as the program declares it (no rows without a table).
fn produced_rows(db: &Connection, run: &Run, rel: &str) -> Option<(Vec<u8>, Vec<i64>, usize)> {
    let types: Vec<u8> = run.fields().types(rel).iter().map(|t| sym_char(&run.model.inputs, t)).collect::<String>().into_bytes();
    let width = types.len();
    let cols = (0..width).map(|c| format!("c{c}")).collect::<Vec<_>>().join(", ");
    let mut flat = Vec::new();
    let mut n = 0;
    if let Ok(mut stmt) = db.prepare(&format!("SELECT DISTINCT {cols} FROM {}", crate::code_rule_dsl::logic::sql::ident(rel))) {
        let mut rows = stmt.query([]).ok()?;
        while let Some(row) = rows.next().ok()? {
            for c in 0..width {
                flat.push(row.get::<_, i64>(c).ok()?);
            }
            n += 1;
        }
    }
    Some((types, flat, n))
}

/// A view's digest: of what its stratum read and of its stratum's other relations (`None` when one is not known).
/// The same whether the stratum ran or was clean.
fn view_digest(reads: Option<&[(String, u64)]>, members: &[(String, Option<u64>)]) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let mut reads: Vec<&(String, u64)> = reads?.iter().filter(|(r, _)| !r.starts_with("answer:")).collect();
    reads.sort();
    reads.hash(&mut h);
    for (r, d) in members {
        (r, (*d)?).hash(&mut h);
    }
    Some(h.finish())
}

/// Each relation's identity (decision 0103; docs/final-shape/notes/t33, K1): a digest of the engine, the rules of its
/// stratum, its declaration and facts, and the identities of what its stratum reads, so a relation's identity
/// changes when any rule it is derived from does. A relation no rule derives (an input, the program's facts) is
/// named by its name, its declaration and its facts. `sccs` in the order the strata run.
///
/// Read from the program's version (0173 step 6): `f` its fields, its rules (`by_stratum`'s) by their handles in `vs`,
/// each in the digest by its image hash (`RuleRef::hash`, of its texts and values, the same in every process), and
/// each relation by its parts' (`identity_parts`).
pub fn identities(f: Fields<'_>, vs: &body::Versions, sccs: &[BTreeSet<String>], by_stratum: &[Vec<RuleKey>]) -> HashMap<String, [u8; 32]> {
    use sha2::{Digest as _, Sha256};
    let leaf = |rel: &str| leaf_identity(f, rel);
    let mut out: HashMap<String, [u8; 32]> = HashMap::new();
    for (scc, rules) in sccs.iter().zip(by_stratum) {
        let mut h = Sha256::new();
        h.update(b"rerail.relation/v2\0");
        h.update(crate::code_rule_dsl::build_ids::engine().as_bytes());
        for rel in scc {
            h.update(rel.as_bytes());
            h.update(b"\0");
            identity_parts::put(&mut h, &f, rel, true);
        }
        let mut hashes: Vec<u64> = rules.iter().map(|&r| vs.at(r).rule.hash()).collect();
        hashes.sort_unstable();
        for x in &hashes {
            h.update(x.to_le_bytes());
        }
        for r in stratum_reads(f, vs, scc, rules) {
            if !scc.contains(&r) {
                h.update(r.as_bytes());
                h.update(out.get(&r).copied().unwrap_or_else(|| leaf(&r)));
            }
        }
        let stratum: [u8; 32] = h.finalize().into();
        for rel in scc {
            let mut h = Sha256::new();
            h.update(stratum);
            h.update(rel.as_bytes());
            out.insert(rel.clone(), h.finalize().into());
        }
    }
    out
}

/// The identity of a relation no rule of `prog` derives (an input, the program's facts): its name, its declaration
/// and its facts (`identities`).
pub fn leaf_identity(f: Fields<'_>, rel: &str) -> [u8; 32] {
    use sha2::{Digest as _, Sha256};
    let mut h = Sha256::new();
    h.update(b"rerail.relation/v2\0leaf\0");
    h.update(rel.as_bytes());
    h.update(b"\0");
    identity_parts::put(&mut h, &f, rel, false);
    h.finalize().into()
}

/// `leaf_identity` of the relations of a program, read from its version.
pub struct Leaves {
    placed: body::Placed,
}

impl Leaves {
    pub fn of_placed(placed: body::Placed) -> Leaves {
        Leaves { placed }
    }
    /// `identities` of the program, each stratum's rules found by their heads in the version (the order of each stratum's
    /// rules in the program).
    pub fn identities_by_heads(&self, sccs: &[BTreeSet<String>]) -> Result<HashMap<String, [u8; 32]>> {
        let p = &self.placed;
        let at: HashMap<&str, usize> = sccs.iter().enumerate().flat_map(|(i, scc)| scc.iter().map(move |r| (r.as_str(), i))).collect();
        let mut keys: Vec<Vec<body::RuleKey>> = vec![Vec::new(); sccs.len()];
        for k in 0..p.len() {
            if let Some(&i) = at.get(p.v.rel_name(p.rule(k).head().rel())) {
                keys[i].push(body::RuleKey::new(0, k));
            }
        }
        let vs = body::Versions::of_placed(p.clone());
        let f = vs.fields(0).ok_or_else(|| Error::Plan("the program has no image: its identities are read from its version".into()))?;
        Ok(identities(f, &vs, sccs, &keys))
    }
    pub fn identity(&self, rel: &str) -> Option<[u8; 32]> {
        Some(leaf_identity(self.placed.fields().ok()?, rel))
    }
}

/// Holds clean relation `rel` in the run as the earlier run kept it, with its statistics; its rows' count.
/// A column's distinct values and its skew-weighted fanout: the sum of squared postings over the rows, the rows a
/// probe by a value drawn from the rows reads, as live statistics give it (`live_stats`). The mean, rows over
/// distinct values, priced a probe of `pv` by `$k` from `fv__ins` at 83 rows where the delta's values held 382.
fn postings(values: impl Iterator<Item = i64>) -> (f64, f64) {
    let mut count: rustc_hash::FxHashMap<i64, u64> = rustc_hash::FxHashMap::default();
    let mut n = 0u64;
    for v in values {
        *count.entry(v).or_insert(0) += 1;
        n += 1;
    }
    let sq: u64 = count.values().map(|m| m * m).sum();
    ((count.len().max(1)) as f64, if n != 0 { (sq as f64 / n as f64).max(1.0) } else { 1.0 })
}

fn kept_strata_hold(ks: &mut dyn KeptStrata, db: &Connection, run: &mut Run, rel: &str) -> Result<usize> {
    // Held as this run's engine holds the relation's columns (over atoms a symbol is an integer column).
    let types: Vec<u8> = run.fields().types(rel).iter().map(|t| sym_char(&run.model.inputs, t) as u8).collect();
    let (stored, held) = ks.rows(rel, &types).ok_or_else(|| Error::Plan(format!("{rel}: kept as clean, but its rows are not kept")))?;
    if types.len() != stored.len() {
        return Err(Error::Plan(format!("{rel}: kept {} columns wide, declared {}", stored.len(), types.len())));
    }
    let k = types.len();
    let n = held.rows();
    let rel_ = &*held;
    let cols: Vec<(f64, f64)> = (0..k).map(|c| postings((0..rel_.len()).filter(|&id| !super::native::component::is_dead(rel_.dead_bits(), id as u32)).map(|id| rel_.value(id, c)))).collect();
    super::native::component::hold_bound(db, rel, Arc::clone(&held));
    run.native_rels.insert(rel.to_owned());
    run.model.stats.card.insert(rel.to_owned(), (n as f64).max(1.0));
    for (c, (d, fan)) in cols.into_iter().enumerate() {
        run.model.ndv.insert((rel.to_owned(), c), d);
        run.model.stats.fan.insert((rel.to_owned(), c), fan);
        run.model.stats.dfreq.remove(&(rel.to_owned(), c));
    }
    Ok(n)
}

/// A clean relation's kept rows (`KeptStrata::rows`), as the requests it answers again take them.
fn kept_strata_rows(ks: &mut dyn KeptStrata, f: Fields<'_>, rel: &str) -> Result<Vec<Vec<i64>>> {
    let types = vec![b'i'; f.types(rel).len()];
    let (_, held) = ks.rows(rel, &types).ok_or_else(|| Error::Plan(format!("{rel}: kept as clean, but its rows are not kept")))?;
    let k = types.len();
    if k == 0 {
        return Ok(Vec::new());
    }
    // copy: not block data: the requests a replayed stratum asks again, as its answer function takes them.
    Ok(held.live_rows().unwrap_or_default().chunks_exact(k).map(<[i64]>::to_vec).collect())
}

/// How a digest reads a relation's cells (decision 0212): an atom column (`b's'`, as frames type it) by its atoms'
/// content keys (`Atoms::content_keys`: their strings' hashes), any other as it is. So a kept frame's digest, and the
/// read digests its readers record, are of its rows' content: the same in every process, whichever ids it gave the
/// strings. By id, a process whose atoms forked from the store's (decision 0170) digested the same rows otherwise, and
/// its frames, keyed by its own numbering, could not be written at all: every next run derived them again.
pub struct Cells<'a> {
    atoms: Option<&'a crate::content_store::atoms::Atoms>,
    atom: Vec<usize>,
    width: usize,
}

impl<'a> Cells<'a> {
    /// The cells of rows of column `types` (`b's'` an atom), whose atoms are `atoms`'.
    pub fn new(atoms: &'a crate::content_store::atoms::Atoms, types: &[u8]) -> Self {
        let atom: Vec<usize> = types.iter().enumerate().filter(|(_, t)| **t == b's').map(|(c, _)| c).collect();
        Cells { atoms: (!atom.is_empty()).then_some(atoms), atom, width: types.len() }
    }

    /// The cells of rows of `width` columns that hold no atom.
    pub fn plain(width: usize) -> Self {
        Cells { atoms: None, atom: Vec::new(), width }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    /// `rows` (whole rows) with each atom cell replaced by its content key.
    pub fn key(&self, rows: &mut [i64]) {
        let Some(atoms) = self.atoms else { return };
        let w = self.width.max(1);
        let mut cells: Vec<i64> = Vec::with_capacity(rows.len() / w * self.atom.len());
        for row in rows.chunks_exact(w) {
            cells.extend(self.atom.iter().map(|&c| row[c]));
        }
        atoms.content_keys(&mut cells);
        let mut it = cells.into_iter();
        for row in rows.chunks_exact_mut(w) {
            for &c in &self.atom {
                row[c] = it.next().expect("a key per atom cell");
            }
        }
    }

    /// The sum of `rows`' row digests (`row_digest`), keyed (`key`) a chunk at a time: no copy of all of them. Many
    /// rows are summed in parts at once (a wrapping sum: the same in any order).
    fn sum(&self, rows: &[i64]) -> u64 {
        let w = self.width;
        if w == 0 {
            return 0;
        }
        const PART: usize = 1 << 15;
        if rows.len() / w >= 2 * PART {
            use rayon::prelude::*;
            return rows.par_chunks(PART * w).map(|part| self.sum_one(part)).reduce(|| 0, u64::wrapping_add);
        }
        self.sum_one(rows)
    }

    /// `rows_digest` of `n` rows read a batch at a time where they lie (`batches(lo, hi, f)`: the rows of ids
    /// `lo..hi` of `ids`, `f` per batch), with no copy of them all: many ids are summed in parts at once.
    pub fn digest_batches(&self, ids: usize, n: usize, batches: &(dyn Fn(usize, usize, &mut dyn FnMut(&[i64])) + Sync)) -> u64 {
        const PART: usize = 1 << 15;
        let part = |lo: usize| {
            let mut s = 0u64;
            batches(lo, (lo + PART).min(ids), &mut |rows| s = s.wrapping_add(self.sum_one(rows)));
            s
        };
        let sum = if self.width == 0 {
            0
        } else if ids >= 2 * PART {
            use rayon::prelude::*;
            (0..ids).into_par_iter().step_by(PART).map(part).reduce(|| 0, u64::wrapping_add)
        } else {
            part(0)
        };
        sum ^ (n as u64).wrapping_mul(0xff51_afd7_ed55_8ccd)
    }

    /// `sum` on this thread.
    fn sum_one(&self, rows: &[i64]) -> u64 {
        let w = self.width;
        if self.atoms.is_none() {
            return rows.chunks_exact(w).fold(0u64, |s, r| s.wrapping_add(row_digest(r)));
        }
        let mut sum = 0u64;
        let mut buf: Vec<i64> = Vec::new();
        for chunk in rows.chunks(w * 4096) {
            buf.clear();
            buf.extend_from_slice(&chunk[..chunk.len() / w * w]);
            self.key(&mut buf);
            sum = buf.chunks_exact(w).fold(sum, |s, r| s.wrapping_add(row_digest(r)));
        }
        sum
    }

    /// One row's digest (`row_digest`) of its content.
    pub fn row(&self, row: &[i64]) -> u64 {
        if self.atoms.is_none() {
            return row_digest(row);
        }
        let mut r = row.to_vec();
        self.key(&mut r);
        row_digest(&r)
    }

    /// Each of `rows`' digests (`row`), keyed under one read of the atoms.
    pub fn rows(&self, rows: &[i64]) -> Vec<u64> {
        let w = self.width.max(1);
        if self.atoms.is_none() {
            return rows.chunks_exact(w).map(row_digest).collect();
        }
        let mut r = rows.to_vec();
        self.key(&mut r);
        r.chunks_exact(w).map(row_digest).collect()
    }
}

/// The most distinct values a column may have for its whole value set to be read from the statistics
/// (`Model::small_values`): the corpus's totals keep every value of such a column, and of others only those tested.
pub const SMALL_NDV: i64 = 64;

/// An order-independent digest of rows (a set) of their content (`Cells`), as `corpus::retain::digest` takes a
/// source's.
pub fn rows_digest(cells: &Cells, rows: &[i64], n: usize) -> u64 {
    cells.sum(rows) ^ (n as u64).wrapping_mul(0xff51_afd7_ed55_8ccd)
}

/// One row's part of `rows_digest`, a sum of them: a relation's digest is patched by its rows gone and added.
pub(crate) fn row_digest(row: &[i64]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for v in row {
        h = (h ^ *v as u64).wrapping_mul(0x0000_0100_0000_01b3);
        h ^= h >> 29;
    }
    h.wrapping_mul(0x9e37_79b9_7f4a_7c15)
}

/// `rows_digest` of a relation of `n` rows whose digest was `digest`, after its rows `gone` went and `added` came
/// (decision 0117 step 4): its rows are not read.
pub fn rows_digest_patched(cells: &Cells, digest: u64, n: usize, gone: &[i64], added: &[i64]) -> u64 {
    let w = cells.width().max(1);
    let (ng, na) = (gone.len() / w, added.len() / w);
    let sum = (digest ^ (n as u64).wrapping_mul(0xff51_afd7_ed55_8ccd)).wrapping_sub(cells.sum(gone)).wrapping_add(cells.sum(added));
    sum ^ ((n + na - ng) as u64).wrapping_mul(0xff51_afd7_ed55_8ccd)
}

/// `run_atoms_native_over`, telling `hooks` of each kept relation and input as its last reader runs, and of each
/// input before its first.
pub fn run_atoms_native_hooked(
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
pub fn run_atoms_native_answering<'a>(
    lowered: &lower::Lowered,
    db: &Connection,
    facts: &str,
    given: Stats,
    kept: BTreeSet<String>,
    modes: &str,
    answer: Answer<'a>,
    asking: Option<&'a mut dyn Asking>,
    strata: Option<&'a mut dyn KeptStrata>,
    texts: Option<Arc<super::strings::Vocabulary>>,
) -> Result<Outcome> {
    let mut native = native_modes(modes);
    native.remove("nearest");
    native.remove("kseed");
    let inputs = Inputs { facts: format!("{facts}."), stats: "s.".to_owned(), atoms: Some(lowered.ops), native, batch: HashMap::new(), given: Some(given), kept };
    run_native_inputs(lowered, db, inputs, Hooks { answer: Some(answer), asking, strata, texts, ..Default::default() })
}

/// Readies the run's connection for native runs before its first wave (decision 0104): the engine's relations and
/// the store's symbols registered (which reads the store's identity), its settings deferred. Once per connection.
pub fn ready_native(db: &Connection) -> Result<()> {
    temp_in_memory(db)?;
    connection::run(db,1,||super::native::register(db))?;
    connection::run(db,2,||super::native::register_symbols(db))?;
    Ok(())
}

fn run_native_inputs(lowered: &lower::Lowered, db: &Connection, inputs: Inputs, hooks: Hooks<'_>) -> Result<Outcome> {
    temp_in_memory(db)?;
    connection::run(db,1,||super::native::register(db))?;
    connection::run(db,2,||super::native::register_symbols(db))?;
    super::native::forget_inputs(db);
    // The program normalized once (`Lowered::normalized`), for `apart` as `run` decides it.
    let mut hooks = hooks;
    let apart = inputs.atoms.is_some() && !inputs.native.is_empty();
    if hooks.normalized.is_none() {
        hooks.normalized = Some(lowered.normalized(apart)?);
    }
    let out = run(&lowered.placed()?, db, inputs, Some(lowered.lowering()), hooks);
    // The run's symbols are pending in the store's arena; they are written when a flush is due or with the next
    // write transaction (decision 0049).
    let flush = super::profile::span(|| "flush symbols".into());
    super::native::flush_symbols(db)?;
    drop(flush);
    out
}

/// Inserts `rows` (each of one width) with `insert` (`INSERT [OR …] INTO t`), many rows per statement: a statement
/// per row would be a host loop over data.
pub fn insert_many(db: &Connection, insert: &str, rows: Vec<Vec<SqlValue>>) -> Result<()> {
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

/// The relations a single rule projects from one input atom, as views (`CREATE TEMP VIEW`), with
/// their statistics, input columns and document columns recorded in `model`.
/// A view's statement.
pub fn view_sql(rel: &str, query: &Query) -> String {
    format!("CREATE TEMP VIEW {} AS {}", crate::code_rule_dsl::logic::sql::ident(rel), query.render())
}

/// The views `run` makes of one-atom projections of an input, each as its query (`view_sql`).
fn plan_views(f: Fields<'_>, vs: &body::Versions, model: &mut Model, rules_by: &IndexMap<&str, Vec<RuleKey>>) -> Result<Vec<(String, Query)>> {
    use super::program_image::NativeSpec;
    let mut views = Vec::new();
    // Resident consumers need facts, not a SQL view that must be read back at each chunk.
    // Other native operators still use SQL-owned inputs and keep their projection views.
    let native_reads: HashSet<&str> = if model.inputs.atoms.is_some() && model.inputs.native.contains("program") {
        f.natives().iter().filter(|n| matches!(n.spec, NativeSpec::Python { .. } | NativeSpec::PythonCandidates { .. })).flat_map(|n| n.reads.iter().map(|&r| f.rel_name(r))).collect()
    } else { HashSet::new() };
    // A gate's relations and a request are tables: the fixpoint writes them as a stratum's (its gate opens, its
    // requests are answered), which a view cannot take.
    let written: HashSet<&str> = f.gates().iter().flat_map(|g| std::iter::once(f.text(g.key)).chain(g.src.map(|s| f.text(s)))).chain(f.answers().iter().map(|&(q, _)| f.rel_name(q))).collect();
    for (&rel, rs) in rules_by {
        if native_reads.contains(rel) || written.contains(rel) { continue; }
        let [rule] = rs.as_slice() else { continue };
        // A gate's relations (a request asked, `ask_…`, and what it feeds) are written as the run decides them: a
        // table, not a view.
        let gated = f.gates().iter().any(|g| {
            f.text(g.key) == rel || g.rel.is_some_and(|r| f.rel_name(r) == rel) || g.src.is_some_and(|s| f.text(s) == rel) || g.quiet.iter().chain(&g.deps).any(|&r| f.rel_name(r) == rel)
        });
        if f.lattice(rel) || f.facts(rel).is_some() || gated {
            continue;
        }
        let b = vs.at(*rule);
        let a = match b.plain_atom(0) {
            Some(a) if b.len() == 1 => a,
            _ => continue,
        };
        let arel = b.rel(a);
        if a.neg() || !f.is_input(arel) {
            continue;
        }
        let order = vec![Step { at: 0, mode: None }];
        let mut query = model.checked_query(f, &b, &order, None, None)?;
        // A projection is a set. Omit DISTINCT only when the head retains an input key;
        // otherwise alternate input supports must not multiply aggregate counts.
        let head: Vec<TermRef<'_>> = b.head().args().collect();
        let args = b.args(0);
        let preserves = |t: &TermRef<'_>| match t {
            TermRef::Var(v) => head.iter().any(|x| x.var() == Some(*v)),
            TermRef::Wild => false,
            _ => true,
        };
        let key = match arel {
            "node" if args.len() == 5 => Some(1),
            "doc" | "tape" => Some(0),
            _ => None,
        };
        query.body.distinct = !(key.is_some_and(|k| args.get(k).is_some_and(preserves)) || args.iter().all(preserves));
        views.push((rel.to_owned(), query));
        model.resolve(f, [arel])?;
        model.view_stats(rel, &b);
        let src = first_columns(args);
        let cols = head.iter().map(|t| t.var().and_then(|v| src.get(&v)).map(|&c| model.col(arel, c))).collect();
        Arc::make_mut(&mut model.view_src).insert(rel.to_owned(), (arel.to_owned(), cols));
        if let ([TermRef::Var(h0), TermRef::Var(h1)], [TermRef::Var(d), TermRef::Var(n), rest @ ..]) = (head.as_slice(), args)
            && arel == "node"
            && rest.iter().all(|t| matches!(t, TermRef::Wild))
            && d != n
        {
            let names = [*h0, *h1];
            let (mut sorted_h, mut sorted_a) = (vec![h0.0, h1.0], vec![d.0, n.0]);
            sorted_h.sort();
            sorted_a.sort();
            if sorted_h == sorted_a {
                let pos = |x: VarSlot| names.iter().position(|v| *v == x).expect("a head variable");
                Arc::make_mut(&mut model.docof).insert(rel.to_owned(), (pos(*n), pos(*d)));
            }
        }
    }
    Ok(views)
}

/// What the runs on a connection (by `setfix_scope`) left with it, so the next run there asks SQL nothing it knows
/// (decision 0104: no SQL in a run's waves that nothing needs). Dropped when the connection closes (`forget`).
#[derive(Clone, Copy, Default)]
struct Left {
    /// The last run left tables or views in SQL: the next run asks which, and drops them.
    temp: bool,
    /// `temp_store` is set to memory.
    configured: bool,
    /// The documents of its runs are node-id intervals by their keys (`docs_by_shift`).
    by_shift: bool,
}
static TEMP_LEFT: std::sync::Mutex<BTreeMap<usize, Left>> = std::sync::Mutex::new(BTreeMap::new());

/// Forgets what the runs on the connection at `at` left (`setfix::forget_scope`, when it closes).
pub fn forget_connection(at: usize) {
    if let Ok(mut m) = TEMP_LEFT.lock() {
        m.remove(&at);
    }
}

/// States that the runs on `db` have documents whose nodes are `d << 32 | index` (the corpus runner's): a range read
/// computes a document's interval, so no table of them is written or read in the run's waves.
pub fn docs_by_shift(db: &Connection) {
    super::native::setfix::forgotten_on_close();
    TEMP_LEFT.lock().expect("temp_left").entry(super::native::setfix_scope(db)).or_default().by_shift = true;
}

pub fn by_shift_of(db: &Connection) -> bool {
    by_shift(db)
}

fn by_shift(db: &Connection) -> bool {
    TEMP_LEFT.lock().expect("temp_left").get(&super::native::setfix_scope(db)).is_some_and(|l| l.by_shift)
}

/// `temp_store` in memory, set once a connection.
fn temp_in_memory(db: &Connection) -> Result<()> {
    let at = super::native::setfix_scope(db);
    if TEMP_LEFT.lock().expect("temp_left").get(&at).is_some_and(|l| l.configured) {
        return Ok(());
    }
    // Set when SQL first runs on the connection (`vtab::setup_now`): a run all native runs no statement.
    crate::code_rule_dsl::facts::vtab::defer_setup(db, "PRAGMA temp_store = MEMORY;");
    super::native::setfix::forgotten_on_close();
    TEMP_LEFT.lock().expect("temp_left").entry(at).or_default().configured = true;
    Ok(())
}

/// At a run's end: its connection is in `TEMP_LEFT` when the run made a table or a view, or kept one. While the run
/// lasts, SQLite's authorizer on its connection notes each table or view made (`TempLeft::watch`), with no statement
/// of its own; the run's end takes it away.
struct TempLeft {
    scope: usize,
    made: std::sync::Arc<std::sync::atomic::AtomicBool>,
    db: *mut rusqlite::ffi::sqlite3,
}

/// The authorizer's callback: notes a table or view made in `made` (an `AtomicBool`); every action is allowed.
unsafe extern "C" fn note_made(made: *mut std::ffi::c_void, action: std::ffi::c_int, _: *const std::ffi::c_char, _: *const std::ffi::c_char,
                               _: *const std::ffi::c_char, _: *const std::ffi::c_char) -> std::ffi::c_int {
    use rusqlite::ffi::*;
    if matches!(action, SQLITE_CREATE_TEMP_TABLE | SQLITE_CREATE_TEMP_VIEW | SQLITE_CREATE_TEMP_INDEX | SQLITE_CREATE_TABLE | SQLITE_CREATE_VIEW) {
        // SAFETY: `made` is the `AtomicBool` of the `TempLeft` that installed this callback, which removes it before
        // its `Arc` is dropped.
        unsafe { (*(made as *const std::sync::atomic::AtomicBool)).store(true, std::sync::atomic::Ordering::Relaxed) };
    }
    SQLITE_OK
}

impl TempLeft {
    /// Notes the tables and views `db` makes until the guard drops. rusqlite 0.41's `authorizer` takes the connection
    /// mutably (a run holds it shared), so the callback is SQLite's own, set and taken away here.
    fn watch(db: &Connection, scope: usize) -> Self {
        let made = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        // SAFETY: the handle is the open connection's, which outlives the run (the guard is the run's local).
        let handle = unsafe { db.handle() };
        unsafe { rusqlite::ffi::sqlite3_set_authorizer(handle, Some(note_made), std::sync::Arc::as_ptr(&made) as *mut std::ffi::c_void) };
        TempLeft { scope, made, db: handle }
    }
}

impl Drop for TempLeft {
    fn drop(&mut self) {
        // SAFETY: as in `watch`; no callback reads `made` once this returns.
        unsafe { rusqlite::ffi::sqlite3_set_authorizer(self.db, None, std::ptr::null_mut()) };
        let made = self.made.load(std::sync::atomic::Ordering::Relaxed);
        if let Ok(mut m) = TEMP_LEFT.lock() {
            m.entry(self.scope).or_default().temp = made;
        }
    }
}

/// Runs program `program`, its version (a whole image: decision 0173, t36 item 7 step 8), over `db`: `lowering` the
/// lowering's domains where the program is lowered (decision 0009).
pub fn run(program: &body::Placed, db: &Connection, inputs: Inputs, lowering: Option<lower::Lowering<'_>>, mut hooks: Hooks<'_>) -> Result<Outcome> {
    // A relation of rules and stated facts: its facts are a relation of facts alone, read by one rule, so the engine
    // holds them as it holds every relation, and no table is loaded with them in the run's waves (decision 0104).
    let apart = inputs.atoms.is_some() && !inputs.native.is_empty();
    // The program's version as the run reads it (t36 item 7, step 2): normalized by the version passes
    // (`passes::normalize`, `stated_apart`), which the planner reads its rules and fields from, and the passes below
    // edit.
    // Normalized once a program (`lower::Lowered::normalized`) where the caller holds it, so every run of it (a
    // corpus run's chunks) reads one version and the analyses it holds (Q203 item 3 (ii)).
    let ((sentinels, stated), normal_v) = match hooks.normalized.take() {
        Some(n) => ((n.sentinels.clone(), n.stated.clone()), n.placed.clone()),
        None => body::pass("plan.normalize", "the program's version", program, |p| {
            let body::Normalized { placed, sentinels, stated } = p.normalized(apart)?;
            Ok::<_, Error>(Some(((sentinels, stated), placed)))
        })?
        .expect("normalizing makes a version"),
    };
    // The lowering's domains, with the columns normalizing added: a sentinel's presence column first, and a relation's
    // stated facts apart under its own domains.
    let domains = lowering.map(|l| {
        let f = normal_v.fields().expect("a whole version");
        let mut domains = l.domains.clone();
        for rel in &sentinels {
            if let Some(columns) = domains.get_mut(rel) {
                while columns.len() < f.width(rel) {
                    columns.insert(0, lower::Domain::Int);
                }
            }
        }
        for (rel, apart) in &stated {
            if let Some(columns) = domains.get(rel).cloned() {
                domains.insert(apart.clone(), columns);
            }
        }
        domains
    });
    let lowered = lowering.zip(domains.as_ref()).map(|(l, domains)| lower::Lowering { domains, dialect: l.dialect });
    let setup = super::profile::span(|| "setup".into());
    connection::run(db,4,||super::strings::register(db))?;
    connection::run(db,8,||super::native::aggregation::register_sum(db))?;
    if inputs.atoms.is_some() && inputs.given.is_some() && PLANS.with(|p|p.borrow().is_some())
        && hooks.answer.is_none()
        && compose::enabled() && compose::accepts(&normal_v)? {
        let p=&normal_v;
        let f=p.fields()?;
        let needed=compose::needed(f,f.outputs().chain(inputs.kept.iter().map(String::as_str)));
        let reads:Vec<String>=f.inputs().filter(|r|needed.contains(*r)).map(str::to_owned).collect();
        if let Some(load)=hooks.load.as_deref_mut(){load(&reads)?;}
        for rel in super::native::component::adopted(db).into_iter().filter(|r|reads.contains(r)) {
            let held=super::native::component::held(db,&rel).ok_or_else(||Error::Plan(format!("missing retained input {rel}")))?;
            let source=held.source((0..f.width(&rel)).map(|c|sql_col(&rel,c)).collect());
            if !crate::code_rule_dsl::facts::vtab::replace_bound_source(db,&rel,source.clone())? {
                connection::bind(db,&rel,source)?;
            }
        }
        // The composed query is one statement over every needed rule. Where SQLite cannot prepare it (a relation it
        // reads that only the engine holds, or SQLite's limits on a statement's references and nesting), the program
        // runs by strata instead, as one the composer does not accept does: composition is a plan, not a semantics.
        // Keyed by the program's digest (`Placed::digest`).
        let plan=compose::planned(p,db,&mut inputs.clone())?;
        if let Err(e)=db.prepare_cached(&plan.sql) {
            super::profile::decision("composed relational query", || format!("declined: SQLite cannot prepare it ({e}); the program runs by strata"));
            if let Some(unload)=hooks.unload.as_deref_mut(){unload(&reads)?;}
        } else {
        let mut out=plan.execute(db,f,lowered,&mut hooks)?;
        if let Some(unload)=hooks.unload.as_deref_mut(){unload(&reads)?;}
        for rel in &sentinels {
            if let Some(rows)=out.outputs.get_mut(rel){*rows=std::sync::Arc::new(rows.without_first());}
            if let Some(cols)=out.nodes.get_mut(rel){*cols=cols.iter().filter_map(|i|i.checked_sub(1)).collect();}
        }
        return Ok(out);
        }
    }
    // The previous run's tables are dropped; its views are kept where this run plans the same (`plan_views`). A
    // connection whose runs left nothing in SQL is not asked what they left (`temp_left`).
    let scope = super::native::setfix_scope(db);
    let left = TEMP_LEFT.lock().expect("temp_left").get(&scope).is_some_and(|l| l.temp);
    let temp: Vec<(String, String, String)> = if !left {
        Vec::new()
    } else {
        db.prepare("SELECT type, name, coalesce(sql, '') FROM temp.sqlite_master WHERE type IN ('table', 'view')")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?
    };
    let _left_after = TempLeft::watch(db, scope);
    let mut old_views: HashMap<String, String> = HashMap::new();
    for (ty, name, sql) in temp {
        if ty == "view" {
            old_views.insert(name, sql);
        } else if !crate::code_rule_dsl::facts::vtab::is_bound(db, &name) {
            // A table of the connection's own (a writable one's `temp` virtual table, `vtab::register`), bound now
            // or kept for this run's bind, is not what an earlier run left.
            db.execute_batch(&format!("DROP TABLE IF EXISTS temp.{}", crate::code_rule_dsl::logic::sql::ident(&name)))?;
            crate::code_rule_dsl::facts::vtab::dropped_temp(db, &name);
        }
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
        make_plan(&normal_v, db, inputs)?
    };
    let demand_span = super::profile::span(|| "demand".into());
    // The literals a whole-program transformation reads, charged to it (here the demand split, then the strata).
    if super::profile::on() {
        super::profile::work(super::profile::Unit::Literal, literals(&normal_v));
    }
    let corpus = PLANS.with(|p| p.borrow().is_some());
    // Both rewrites below keep the program's meaning and read every rule: a corpus run takes the choices an earlier
    // run or chunk made for the same program at its scale or more (`choices`, decision 0090 §5).
    // A program of no rules (a cross part whose every stratum is kept) has no choice to make, so none is kept for it:
    // keeping one wrote to the store in the run after a cold one, for a program no other run plans (decision 0250).
    // So too a program whose every rule is a view of an input's columns (Q37): held in place, never kept, it is what
    // is left of a cross part kept whole otherwise.
    let choosing = program.len() > 0 && normal_v.fields().ok().is_none_or(|f| normal_v.v.rules().any(|r| input_view_shape(&normal_v.v, r, |x| f.is_input(x)).is_none()));
    let chosen_key = (corpus && choosing).then(|| choices::key(&normal_v));
    let reused = chosen_key.as_ref().and_then(|k| choices::lookup(k, model.stats.nodes));
    // The program's version as the passes chose it (t36 item 6): their overlay, or the kept choice's, which the run
    // reads its rules from as it is; the program's own where nothing was rewritten.
    let placed: body::Placed;
    // The relations inlined, for the outcome (a session stores, never inlines, what it keeps: decision 0099).
    let inlined_rels: BTreeSet<String>;
    match reused {
        Some(c) => {
            let nodes = model.stats.nodes;
            super::profile::decision("planning choices (decision 0090 §5)", || format!("kept: inline or store and demand as chosen at {:.0} nodes, taken at {nodes:.0}", c.nodes));
            inlined_rels = c.inlined.iter().cloned().collect();
            if !c.inlined.is_empty() {
                super::profile::decision("inline or store (decision 0099)", || format!("{} relations recomputed in their readers: {}", c.inlined.len(), c.inlined.join(", ")));
            }
            if !c.demanded.is_empty() {
                super::profile::decision("demand (decision 0048)", || format!("{} relations computed only for the keys their readers reach: {}", c.demanded.len(), c.demanded.join(", ")));
            }
            placed = match c.version {
                // The program run is the rewritten one: its analysis is that program's, made when asked.
                Some(v) => {
                    crate::threads::drop_behind(std::mem::take(&mut model.analysis));
                    v
                }
                // Not rewritten: the program's own version.
                None => normal_v,
            };
        }
        None => {
            // What each derived relation keeps (decision 0099): stored, or inlined into its readers, from this plan's
            // estimates; before demand, which then sees the readers as inlining left them. Corpus runs only, as demand.
            let mut inlined_names = Vec::new();
            // The program's version: each pass edits the one before as an overlay.
            let choice = if corpus { body::pass("plan.inline", "inline or store", &normal_v, |p| keeps::choose(p, &mut model)).map_err(Error::Plan)? } else { None };
            let inlined = match choice {
                Some((done, v)) => {
                    super::profile::decision("inline or store (decision 0099)", || format!("{} relations recomputed in their readers: {}", done.len(), done.join(", ")));
                    // A measurement aid: `RERAIL_INLINE_DUMP=<file>` appends the rules as inlining left them.
                    if let Some(path) = std::env::var_os("RERAIL_INLINE_DUMP") {
                        use std::io::Write;
                        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                            for r in v.v.rules() {
                                let _ = writeln!(f, "{}", v.v.show_rule(r));
                            }
                        }
                    }
                    // The program run is the inlined one: its analysis is that program's, made when asked.
                    crate::threads::drop_behind(std::mem::take(&mut model.analysis));
                    inlined_names = done;
                    Some(v)
                }
                None => None,
            };
            // Relations computed for every node that rules only negate, computed for the keys those rules reach
            // (decision 0048); in a corpus run only (kept plans), so T's golden plans are unchanged.
            let mut demanded_names = Vec::new();
            let Ok(split_neg) = body::pass("plan.demand", "negation split", inlined.as_ref().unwrap_or(&normal_v), |p| Ok::<_, std::convert::Infallible>(if corpus { demand::split_negations(p, &model) } else { None }));
            let split = match split_neg {
                Some((demanded, v)) => {
                    super::profile::decision("demand (decision 0048)", || {
                        format!("{} relations computed only for the keys their readers reach: {}", demanded.len(), demanded.join(", "))
                    });
                    // The program run is the split one: its analysis is that program's, made when asked.
                    crate::threads::drop_behind(std::mem::take(&mut model.analysis));
                    demanded_names = demanded;
                    Some(v)
                }
                None => None,
            };
            let rewritten = split.or(inlined);
            inlined_rels = inlined_names.iter().cloned().collect();
            if let Some(k) = chosen_key {
                choices::remember(k, choices::Chosen { version: rewritten.clone(), inlined: inlined_names, demanded: demanded_names, nodes: model.stats.nodes });
            }
            // Not rewritten: the program's own version, made above.
            placed = rewritten.unwrap_or(normal_v);
        }
    }
    drop(demand_span);
    let strata_span = super::profile::span(|| "strata".into());
    if super::profile::on() {
        super::profile::work(super::profile::Unit::Literal, literals(&placed));
    }
    // The run's version 0, whole (t36 item 7): the run reads the program's rules and fields from it (`f0` here,
    // `Run::fields` in its helpers).
    let Some(_) = Fields::of(&placed) else { return Err(Error::Plan("the program has no image: its fields are read from its version".into())) };
    // Read once (above), and shared by this clone.
    let p0 = placed.clone();
    let f0 = Fields::of(&p0).expect("a whole version");
    let sccs = p0.components()?;
    let deps = super::program_image::analyses::edges_by_name(f0.v, f0.about);
    // A debugging aid: `RERAIL_RUN_DUMP=<file>` appends the rules each run is given, by stratum.
    if let Some(path) = std::env::var_os("RERAIL_RUN_DUMP") {
        use std::io::Write as _;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let v = &p0.v;
            let _ = writeln!(f, "== run of {} rules", v.rules().count());
            for scc in sccs.iter() {
                let _ = writeln!(f, "-- stratum {scc:?}");
                for r in v.rules().filter(|r| scc.contains(v.rel_name(r.head().rel()))) {
                    let _ = writeln!(f, "{}", v.show_rule(r));
                }
            }
        }
    }
    // Device waves (decision 0095 §5, 0098 §6): the strata in order of their levels. A stratum's level is one more
    // than the deepest level it depends on, so a level's strata do not depend on each other, and in this order each
    // level's strata are adjacent: a level is a device wave. The sort is stable, so within a level the strata keep
    // the dependency order's. Only strata run natively make waves (SQL launches nothing), so a run all in SQL keeps
    // the dependency order, and T's plans in it.
    // `by_level` reorders them: its own copy, the version's kept as it is.
    let (sccs, level_of) = by_level(sccs.to_vec(), &deps, model.inputs.native.contains("program"));
    // Each relation's rules, as keys of the program's version (its rule `k` is the program's rule `k`), in the
    // program's order.
    let mut rules_by: IndexMap<&str, Vec<RuleKey>> = IndexMap::new();
    for k in 0..p0.len() {
        rules_by.entry(p0.v.rel_name(p0.rule(k).head().rel())).or_default().push(RuleKey::program(k));
    }
    drop(strata_span);
    drop(planning);
    let tables = super::profile::span(|| "views and tables".into());
    let native = model.inputs.native.clone();
    // Inputs kept in the engine from an earlier run (a session's, decision 0090 step 3) are read in place, as native
    // relations, with the indexes they have.
    let adopted: BTreeSet<String> = super::native::component::adopted(db).into_iter().filter(|r| f0.is_input(r)).collect();
    // A run that is not native runs SQL from its start: the connection's deferred set-up first (`vtab::setup_now`).
    if !(atoms && !native.is_empty()) {
        crate::code_rule_dsl::facts::vtab::setup_now(db)?;
    }
    // The maintained run's driver (decision 0117 step 3).
    let mut maintaining = hooks.maintain.take();
    let mut moved_rels: HashSet<String> = maintaining.as_deref_mut().map(|m| m.seeds().into_iter().collect()).unwrap_or_default();
    // Reach indexes made again whose old rows are kept in memory: their deltas made once they are (`reach_delta`).
    let mut reach_olds: HashMap<String, Arc<super::native::component::InRel>> = HashMap::new();
    // The strata derived again in place of the maintain mode, by relation: its kept rows, which its delta is made
    // against once its level has run (at the next level's placement).
    let mut again_olds: BTreeMap<String, Arc<super::native::component::InRel>> = BTreeMap::new();
    let (mut reached, mut quiet_slots) = (0usize, 0usize);
    // Each stratum as its level's start placed it (decision 0117 step 3), the strata placed so far, and the forms issued
    // as jobs and the pieces they ran.
    let mut slots: Vec<Slot> = Vec::new();
    // The relations of quiet strata left stored, not read (decision 0117 step 4).
    let mut unread: BTreeSet<String> = BTreeSet::new();
    let mut prepared_to = 0usize;
    let projections0: HashMap<String, Vec<(usize, i64)>> = super::program_image::analyses::projections(&p0.v).into_iter().map(|(r, eqs)| (p0.v.rel_name(r).to_owned(), eqs)).collect();
    let mut run = Run { db: crate::code_rule_dsl::facts::vtab::Db::of(db), model, views: Default::default(), made: HashSet::new(), orders: HashMap::new(), keyless: HashSet::new(), native_rels: adopted, device_views: HashSet::new(), mode_reads: Default::default(), demands: Default::default(), pending: HashMap::new(), why_not: None, projections: Arc::new(projections0), inputs: Arc::new(RunInputs { program: f0.inputs().collect(), added: Default::default() }), versions: Arc::new(body::Versions::of_placed(placed)), version: 0, answer: hooks.answer.take(), texts: hooks.texts.take(), asking: None, why_class: None, answered: BTreeMap::new(), logging: None, answer_logs: HashMap::new(), quiet: Default::default() };
    // A native run makes no views: a projection of an input is a stratum like any, run from the input where it is
    // held (decision 0104: no SQL in the run's waves). A SQL run reads its inputs through views in place.
    let planned_views = if atoms && !native.is_empty() { Vec::new() } else { plan_views(f0, &Arc::clone(&run.versions), &mut run.model, &rules_by)? };
    for (rel, view) in planned_views {
        let view = view_sql(&rel, &view);
        // SQLite keeps a temp view's statement without `TEMP`.
        if old_views.remove(&rel).as_deref() == Some(&view.replacen("CREATE TEMP VIEW", "CREATE VIEW", 1)) {
            // Kept as the last run made it: still in SQL, so the next run asks what is left.
            _left_after.made.store(true, std::sync::atomic::Ordering::Relaxed);
        } else {
            db.execute_batch(&format!("DROP VIEW IF EXISTS temp.{}; {view}", crate::code_rule_dsl::logic::sql::ident(&rel)))?;
        }
        Arc::make_mut(&mut run.views).insert(rel);
    }
    for name in old_views.keys() {
        db.execute_batch(&format!("DROP VIEW IF EXISTS temp.{}", crate::code_rule_dsl::logic::sql::ident(&name)))?;
    }
    // The last run's views read in place are not this one's.
    crate::code_rule_dsl::facts::vtab::forget_views(db);
    // Relations the program's rules or outputs name (a part of a split program declares the whole's).
    let vs0 = Arc::clone(&run.versions);
    let named: HashSet<&str> = body::Versions::keys(0, p0.len())
        .flat_map(|r| std::iter::once(vs0.head_at(r)).chain(reads_of(&vs0, r)))
        .chain(f0.outputs())
        .chain(f0.fact_rels().into_iter().map(|f| f0.text(f.name())))
        .collect();
    // In the declarations' order (by name).
    for (rel, cols) in f0.decls_by_name() {
        if f0.is_input(rel) || run.views.contains(rel) || !named.contains(rel) {
            continue;
        }
        // Typed columns: an untyped column compared with an INTEGER one gets numeric affinity applied to it, and
        // SQLite then can't use its index (lessons ledger L11, narrow typed tables).
        let names = cols
            .iter()
            .enumerate()
            .map(|(i, t)| match *t {
                "node" | "int" | "flat" | "min" | "max" => Ok(format!("c{i} INTEGER")),
                "sym" if atoms => Ok(format!("c{i} INTEGER")),
                "sym" => Ok(format!("c{i} TEXT")),
                _ => Err(Error::Plan(format!("{rel}: no SQL type for {t}"))),
            })
            .collect::<Result<Vec<_>>>()?
            .join(", ");
        // Clustered on the key (WITHOUT ROWID), so an insert writes one b-tree, and every secondary index carries the
        // whole row.
        let key = match f0.lattice_decl(rel) {
            Some(l) => (0..l.keys).map(|i| format!("c{i}")).collect::<Vec<_>>().join(", "),
            None => (0..cols.len()).map(|i| format!("c{i}")).collect::<Vec<_>>().join(", "),
        };
        let create = format!("CREATE TEMP TABLE {}({names}, PRIMARY KEY ({key})) WITHOUT ROWID", crate::code_rule_dsl::logic::sql::ident(rel));
        // A rule's head is made when SQL first needs it; a relation of facts alone now.
        // (Over atoms: T's extension, over text, reads its relations' tables itself.)
        // A request's produced relation too: its answers go to the engine's component, or to its table (made then).
        // A relation of neither in a run with no native reader too (the cone's `load`): a run that reads it in SQL
        // has it made then; one that does not makes no table its next run on the connection drops. (With a native
        // reader it is held empty in the engine, below.)
        let neither = native.is_empty() && !rules_by.contains_key(rel);
        if atoms && (rules_by.contains_key(rel) || f0.produced(rel) || neither) && !f0.has_facts(rel) {
            run.pending.insert(rel.to_string(), create);
            continue;
        }
        // A relation of stated facts alone (or of none) is held in the engine, as any relation is, when the run has it: its
        // facts are not loaded into a SQL table inside the run's waves (decision 0104). SQL that reads it has it made
        // then (`materialize`).
        if atoms && !native.is_empty() && !cols.is_empty() && !rules_by.contains_key(rel) && !f0.lattice(rel) {
            let facts: Vec<Vec<super::program_image::Cell>> = f0.facts(rel).map(|f| f.rows().map(|row| row.collect()).collect()).unwrap_or_default();
            let texts: Vec<&str> = facts.iter().flatten().filter_map(|v| if let super::program_image::Cell::Str(s) = v { Some(f0.text(*s)) } else { None }).collect();
            let mut ids = super::native::intern_batch(db, texts.iter().copied())?.into_iter();
            let mut rows: Vec<i64> = Vec::with_capacity(facts.len() * cols.len());
            for row in &facts {
                for v in row {
                    rows.push(match v {
                        super::program_image::Cell::Int(i) => *i,
                        super::program_image::Cell::Str(_) => ids.next().expect("one atom per text"),
                        super::program_image::Cell::Branded(b) => b.value(),
                    });
                }
            }
            let types: Vec<u8> = cols.iter().map(|t| sym_char(&run.model.inputs, t) as u8).collect();
            let mut set: rustc_hash::FxHashSet<&[i64]> = rustc_hash::FxHashSet::default();
            let mut unique: Vec<i64> = Vec::with_capacity(rows.len());
            if cols.is_empty() {
                // A relation of no columns holds its one fact or none.
            } else {
                for row in rows.chunks_exact(cols.len()) {
                    if set.insert(row) {
                        unique.extend_from_slice(row);
                    }
                }
            }
            let n = if cols.is_empty() { usize::from(!facts.is_empty()) } else { unique.len() / cols.len() };
            for c in 0..cols.len() {
                let (d, fan) = postings(unique.iter().skip(c).step_by(cols.len()).copied());
                run.model.ndv.insert((rel.to_string(), c), d);
                run.model.stats.fan.insert((rel.to_string(), c), fan);
            }
            run.model.stats.card.insert(rel.to_string(), (n as f64).max(1.0));
            super::native::component::hold_rows(db, rel, types, unique, n);
            run.native_rels.insert(rel.to_string());
            run.pending.insert(rel.to_string(), create);
            continue;
        }
        db.execute(&create, [])?;
        let rows: Vec<Vec<SqlValue>> = f0
            .facts(rel)
            .into_iter()
            .flat_map(|f| f.rows())
            .map(|row| {
                row.map(|v| match v {
                    super::program_image::Cell::Int(i) => SqlValue::Integer(i),
                    super::program_image::Cell::Str(s) => SqlValue::Text(f0.text(s).to_string()),
                    super::program_image::Cell::Branded(b) => SqlValue::Integer(b.value()),
                })
                .collect()
            })
            .collect();
        if f0.lattice(rel) && !rows.is_empty(){
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
    // Each stratum's rules (an index into `sccs`), as keys of the program's version, read by their handles.
    let by_stratum = keys_by_stratum(&p0, &sccs);
    // The program's version alone (version 0, as in `run.versions`, so its keys are the same): what the strata's
    // readers below read the program's rules by, apart from the forms the run adds to its own.
    let run_vs = body::Versions::of_placed(p0.clone());
    let last = last_reads(f0, &run_vs, &sccs, &by_stratum, &run.views);
    let first = first_reads(f0, &run_vs, &sccs, &by_stratum, &run.views);
    let stratum_of: HashMap<&str, usize> = sccs.iter().enumerate().flat_map(|(j, scc)| scc.iter().map(move |r| (r.as_str(), j))).collect();
    let mut unloaded: BTreeSet<String> = BTreeSet::new();
    // A gate that orders a maintained stratum's pieces (decision 0101 §3) holds nothing past its component.
    let gated: BTreeSet<String> = f0
        .gates()
        .iter()
        .flat_map(|g| [f0.text(g.key)].into_iter().chain(g.src.map(|s| f0.text(s))).chain(g.rel.iter().chain(&g.quiet).chain(&g.deps).map(|&r| f0.rel_name(r))))
        .map(str::to_owned)
        .collect();
    let mut stays: BTreeSet<String> = f0.outputs().map(str::to_owned).chain(run.model.inputs.kept.iter().cloned()).chain(gated.iter().cloned()).collect();
    // The dump (`RERAIL_REL_DIR`, a debugging aid) keeps every derived relation to the run's end, so it holds the
    // pieces a run releases as it goes (`h__ins`, `h__del`, …).
    if std::env::var_os("RERAIL_REL_DIR").is_some() {
        stays.extend(sccs.iter().flatten().cloned());
    }
    // What stays, and what the views among it read (through views): SQL reads a view's inputs once the run ends.
    let mut pinned: BTreeSet<String> = stays.clone();
    // Of them, the relations a view that stays reads (through views), kept ones too: not handed off (`hand_off`), as
    // the view is read after the run.
    let mut view_read: BTreeSet<String> = BTreeSet::new();
    loop {
        let reads: Vec<String> = body::Versions::keys(0, p0.len())
            .filter(|&r| pinned.contains(run.versions.head_at(r)) && run.views.contains(run.versions.head_at(r)))
            .flat_map(|r| reads_of(&run.versions, r).map(str::to_owned).collect::<Vec<_>>())
            .collect();
        view_read.extend(reads.iter().cloned());
        let more: Vec<String> = reads.into_iter().filter(|r| !pinned.contains(r.as_str())).collect();
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
    for r in f0.inputs() {
        loads[first.get(r).copied().unwrap_or(0).min(sccs.len().max(1) - 1)].push(r.to_owned());
        if let Some(&l) = last.get(r)
            && !pinned.contains(r)
        {
            unloads[(l + 1).min(sccs.len())].push(r.to_owned());
        }
    }
    // Each relation by the stratum after its last reader: a native one is released or read out there. A native
    // relation is one adopted before the first stratum or one a stratum made (`native_stratum`), so a stratum looks at
    // those of its bucket and the one before's own unread relations, not every native relation (quadratic in strata).
    let mut after_last: Vec<Vec<String>> = vec![Vec::new(); sccs.len() + 1];
    for (r, &l) in &last {
        after_last[(l + 1).min(sccs.len())].push(r.clone());
    }
    // Device waves (decision 0095 §5, 0098 §6), by level (`by_level`):
    // - Each level's count tables that an earlier chunk found worth building (`count table`) are built as one batch
    //   when the level's first stratum runs. A table whose relation is not held yet is built by its step, as before.
    // - A stratum whose form launches kernels (the closure's bit rows) prepares them and waits (`Native::Waiting`),
    //   and the next strata of its level run. When the level ends, or a stratum depends on a waiting one, every
    //   waiting launch goes to the executor as one batch (`lanes::wave::flush`), and the waiting strata finish in
    //   order. So each lane gets one submission per level for each, not one per stratum.
    // A waiting stratum has read its inputs: what it does after its wave reads only its own component and the
    // launches' outputs. So the inputs it read may be dropped, and the relations it read released, while it waits.
    let wants_of: Vec<BTreeSet<(String, Vec<usize>)>> = by_stratum
        .iter()
        .zip(&sccs)
        .map(|(rules, scc)| {
            rules
                .iter()
                .flat_map(|&r| {
                    let b = run_vs.at(r);
                    (0..b.len())
                        .filter_map(|i| match b.view(i) {
                            Lit::Agg { op: super::program_image::AggRef::Count, atom, .. } if !scc.contains(b.rel(atom)) => {
                                Some((b.rel(atom).to_owned(), atom.args().enumerate().filter(|(_, t)| !matches!(t, TermRef::Wild)).map(|(c, _)| c).collect()))
                            }
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<BTreeSet<(String, Vec<usize>)>>()
        })
        .collect();
    let mut waved = vec![false; sccs.len()];
    // Strata waiting on their device wave: position, key, driver's state.
    let mut waiting: Vec<(usize, String, Box<Driving>)> = Vec::new();
    // Strata suspended on their requests (decision 0114 §3), and the answers come for them so far, by ticket.
    let mut asked: Vec<Asked> = Vec::new();
    let mut arrived: HashMap<u64, Vec<(String, Vec<Vec<i64>>)>> = HashMap::new();
    // Relations no stratum reads, of strata that finished after the stratum following them ran: released or read
    // out at the next stratum, as they would have been at the one following them.
    let mut carry: Vec<String> = Vec::new();
    // Strata kept from an earlier run of the program (`Hooks::strata`, decision 0077 §4): a stratum whose every read
    // has the rows it had then is not run, its relations' rows taken from what was kept when a stratum that runs
    // reads them (or the run's end keeps them); a stratum that runs gives its rows' digest, so one whose rows came
    // out the same leaves its readers clean (early cutoff).
    let mut strata_kept = hooks.strata.take();
    if let Some(ks) = strata_kept.as_deref_mut() {
        let ids = identities(f0, &run_vs, &sccs, &by_stratum);
        ks.program(&ids);
    }
    // Each evaluated stratum's reads, with the digest each had when it ran (`None`: one was not known).
    let mut stratum_read_digests: HashMap<usize, Option<Vec<(String, u64)>>> = HashMap::new();
    let input_set: BTreeSet<&str> = f0.inputs().collect();
    // This run's digest of each derived relation (`None`: not known, as for a stratum SQL ran); clean relations not
    // yet held; the strata evaluated whose rows' digests are not taken yet. A stratum waiting on its device wave has
    // no rows yet: its digest is taken once its wave ran (a stratum that reads it finishes the wave first).
    let mut fresh: HashMap<String, Option<u64>> = HashMap::new();
    let mut unheld: BTreeSet<String> = BTreeSet::new();
    let mut evaluated: Vec<usize> = Vec::new();
    let (mut clean_strata, mut held_again, mut held_rows) = (0usize, 0usize, 0usize);
    // Decision 0113: native strata issued ahead, as components on the executor's host stream, retired here in order.
    let mut board = (issue::on() && native.contains("program") && strata_kept.is_none() && maintaining.is_none()).then(|| issue::Board::new(&sccs, &deps));
    if maintaining.is_some() {
        slots = (0..sccs.len()).map(|_| Slot::Base).collect();
    }
    let ambient = Ambient::here();
    // Requests are posted, not waited for, where strata retire from the scoreboard (`RERAIL_ASK_ASYNC=off`: waited).
    if board.is_some() && std::env::var("RERAIL_ASK_ASYNC").as_deref() != Ok("off") {
        run.asking = hooks.asking.take();
    }
    // What the strata suspended on their requests read: kept until they finish (their fixpoints read them again).
    let held_by_asked = |asked: &[Asked]| -> HashSet<String> { asked.iter().flat_map(|a| stratum_reads(f0, &run_vs, &sccs[a.at], &by_stratum[a.at])).collect() };
    // Decision 0208: each stratum's start and end where no board traces them (every stratum then runs on the loop's
    // thread), for the run's measured critical path.
    let strata_t0 = std::time::Instant::now();
    let mut loop_times: Vec<Option<(f64, f64, u8)>> = if board.is_none() { vec![None; sccs.len()] } else { Vec::new() };
    let mut loop_open: Option<(usize, f64)> = None;
    super::lanes::executor().stream("strata", |stream| -> Result<()> {
    for (at, scc) in sccs.iter().enumerate() {
        if let Some(b) = board.as_mut() {
            b.close();
        }
        if let Some((p, a)) = loop_open.take() {
            loop_times[p] = Some((a, strata_t0.elapsed().as_secs_f64(), 0));
        }
        // Answers come as events: their strata resume. One that reads a stratum still asking waits for its answers
        // (helping nothing: the board's components go on, and are taken as they end).
        if !asked.is_empty() {
            take_answers(&mut run, &mut asked, &mut arrived, &mut waiting, &mut outcome, &mut carry, &sccs, &last, std::time::Duration::ZERO)?;
            let reads_asked = |asked: &[Asked]| {
                let theirs: HashSet<&str> = asked.iter().flat_map(|a| sccs[a.at].iter().map(String::as_str)).collect();
                scc.iter().flat_map(|r| deps.get(r).into_iter().flatten()).any(|(d, _)| theirs.contains(d.as_str()))
            };
            while reads_asked(&asked) {
                if let Some(b) = board.as_mut() {
                    b.collect(&mut run, stream)?;
                }
                take_answers(&mut run, &mut asked, &mut arrived, &mut waiting, &mut outcome, &mut carry, &sccs, &last, std::time::Duration::from_millis(1))?;
            }
        }
        let held = if asked.is_empty() { HashSet::new() } else { held_by_asked(&asked) };
        if !waiting.is_empty() {
            let reads_waiting = || {
                let theirs: HashSet<&str> = waiting.iter().flat_map(|w| sccs[w.0].iter().map(String::as_str)).collect();
                scc.iter().flat_map(|r| deps.get(r).into_iter().flatten()).any(|(d, _)| theirs.contains(d.as_str()))
            };
            if level_of[at] != level_of[waiting[0].0] || reads_waiting() {
                finish_wave(&mut run, &mut waiting, &mut outcome, &mut carry, &sccs, &last, &mut asked)?;
            }
        }
        if let Some(b) = board.as_mut() {
            b.advance(at, &waiting.iter().map(|w| w.0).chain(asked.iter().map(|a| a.at)).collect::<Vec<_>>());
        } else {
            loop_open = Some((at, strata_t0.elapsed().as_secs_f64()));
        }
        super::lanes::prof::level(Some(level_of[at]));
        if let Some(ks) = strata_kept.as_deref_mut() {
            let (done, still): (Vec<usize>, Vec<usize>) = std::mem::take(&mut evaluated).into_iter().partition(|k| !waiting.iter().any(|w| w.0 == *k));
            evaluated = still;
            for k in done {
                let reads = with_answers(f0, db, &sccs[k], stratum_read_digests.remove(&k).flatten(), run.answer_logs.remove(&k));
                kept_strata_ran(ks, db, &run, &sccs[k], reads.as_ref(), &mut fresh);
            }
        }
        // A relation a stratum still asking reads is kept to a later position.
        if !held.is_empty() {
            let (later, now): (Vec<String>, Vec<String>) = std::mem::take(&mut after_last[at]).into_iter().partition(|r| held.contains(r));
            after_last[at] = now;
            after_last[at + 1].extend(later);
            let (later, now): (Vec<String>, Vec<String>) = std::mem::take(&mut unloads[at]).into_iter().partition(|r| held.contains(r));
            unloads[at] = now;
            unloads[at + 1].extend(later);
        }
        for r in &after_last[at] { run.demands.forget(r); }
        let candidates: Vec<String> = if at == 0 {
            run.native_rels.iter().cloned().collect()
        } else {
            let mut c = std::mem::take(&mut after_last[at]);
            c.extend(sccs[at - 1].iter().filter(|r| !last.contains_key(*r)).cloned());
            c.append(&mut carry);
            let mut seen = HashSet::new();
            c.retain(|r| run.native_rels.contains(r) && seen.insert(r.clone()));
            c
        };
        let candidates: Vec<String> = if held.is_empty() {
            candidates
        } else {
            let (later, now): (Vec<String>, Vec<String>) = candidates.into_iter().partition(|r| held.contains(r));
            carry.extend(later);
            now
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
            let mut give: Vec<String> = Vec::new();
            for r in read_out.iter().filter(|r| run.model.inputs.kept.contains(*r) && !gated.contains(*r) && !view_read.contains(*r)) {
                if hand(r)? {
                    give.push(r.clone());
                }
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
        // Decision 0117 step 3: a maintained run walks the base program's strata, a level at a time. At a level's
        // first stratum, every stratum of the level is placed: its producers are all below it, so whether a changed
        // read reaches it is known. One no changed read reaches holds its kept rows; one that a changed read reaches
        // gets its form, maintained or derived again (decision 0090 §4, by its input deltas' size). The level's
        // count tables and first joins go out as one device batch (decision 0095 §5, 0100 §5), and each form whose
        // pieces a strand can run is issued as a job on the host stream (decision 0113), retired at its position.
        if let Some(m) = maintaining.as_deref_mut() {
            if prepared_to <= at {
                // The deltas of the relations derived again below: each against its kept rows, where the engine holds
                // what was derived (one SQL derived has none: its readers derive again).
                for (r, old) in std::mem::take(&mut again_olds) {
                    if super::native::component::derived(db, &r).is_none() {
                        m.no_delta(&r);
                        continue;
                    }
                    if kept_delta(db, &r, &old, &mut run.native_rels)? {
                        m.delta_made(&r);
                        // Made a version of its kept rows (`InRel::rebased`): its change is its marks, as a maintained
                        // relation's. Else its delta's rows.
                        let rebased = super::native::component::held(db, &r).filter(|h| h.start().is_some());
                        if let Some((gone, added)) = rebased.and_then(|h| h.version_counts()) {
                            outcome.deltas.insert(r.clone(), Delta::Marks { gone, added });
                        } else {
                            let rows = |s: &str| super::native::component::held(db, &format!("{r}__{s}")).and_then(|h| h.live_rows());
                            if let (Some(gone), Some(added)) = (rows("minus"), rows("plus")) {
                                outcome.deltas.insert(r.clone(), Delta::Rows { gone, added });
                            }
                        }
                    } else {
                        moved_rels.remove(&r);
                        outcome.unchanged.insert(r);
                    }
                }
                let end = (at..sccs.len()).find(|&j| level_of[j] != level_of[at]).unwrap_or(sccs.len());
                prepared_to = end;
                let _level = super::profile::span(|| format!("a level of {} strata placed", end - at));
                if let Some(load) = hooks.load.as_deref_mut() {
                    for j in at + 1..end {
                        let now: Vec<String> = std::mem::take(&mut loads[j]);
                        if !now.is_empty() {
                            load(&now)?;
                        }
                    }
                }
                let mut wants: BTreeSet<(String, Vec<usize>)> = BTreeSet::new();
                let mut specs: Vec<Arc<JoinSpec>> = Vec::new();
                // Placed: held as kept (in memory, or left stored, unread), run as the base program's, or reached.
                let mut base_strata: Vec<usize> = Vec::new();
                let mut reached_strata: Vec<(usize, Vec<RuleKey>, BTreeSet<String>)> = Vec::new();
                let mut need: BTreeSet<String> = BTreeSet::new();
                for j in at..end {
                    let scc = &sccs[j];
                    let rules: Vec<RuleKey> = scc.iter().flat_map(|rel| rules_by.get(rel.as_str()).into_iter().flatten().copied()).collect();
                    let reads = stratum_reads(f0, &run_vs, scc, &rules);
                    let moved: BTreeSet<String> = reads.iter().filter(|r| !scc.contains(*r) && moved_rels.contains(r.as_str())).cloned().collect();
                    waved[j] = true;
                    if moved.is_empty() {
                        let no_facts = scc.iter().all(|r| !f0.has_facts(r));
                        let in_memory: Option<Vec<(&String, Arc<super::native::component::InRel>)>> = scc.iter().map(|r| m.kept(r).map(|k| (r, k))).collect();
                        let placed = match in_memory.filter(|_| no_facts) {
                            Some(kept) => {
                                for (r, o) in kept {
                                    hold_quiet(&mut run, f0, db, r, o);
                                }
                                true
                            }
                            // Stored: read only if a stratum that runs reads it, or the caller does.
                            None if no_facts && scc.iter().all(|r| m.keeps(r)) => {
                                unread.extend(scc.iter().cloned());
                                true
                            }
                            None => false,
                        };
                        if placed {
                            quiet_slots += 1;
                            outcome.unchanged.extend(scc.iter().cloned());
                            super::profile::stratum(&stratum_key(scc), "native set engine", || "quiet: no read changed; each relation is its kept one (decision 0117 step 3)".to_owned());
                            slots[j] = Slot::Quiet;
                            continue;
                        }
                        // Not kept: derived from its reads, as they were, so its rows are as they were.
                        need.extend(reads.iter().filter(|r| !scc.contains(*r)).cloned());
                        base_strata.push(j);
                        continue;
                    }
                    // A native operator's stratum (the reach index, decision 0119) has no maintained form: it is made again
                    // from its reads as they are now, as the base program makes it, and its readers derive again. A text
                    // test's has (`maintain`: only the new strings tested). A reach index whose old rows are kept in
                    // memory (a chunk's guest rounds, decision 0331) is made the same way, and its delta is then made
                    // against them (`reach_delta`), so its readers are maintained.
                    if scc.iter().any(|r| f0.native(r).is_some_and(|n| !matches!(n.spec, super::program_image::NativeSpec::TextTest { .. }))) {
                        need.extend(reads.iter().filter(|r| !scc.contains(*r)).cloned());
                        base_strata.push(j);
                        for r in scc {
                            if let Some(old) = m.kept(r).filter(|_| matches!(f0.native(r).map(|n| &n.spec), Some(super::program_image::NativeSpec::Reach { .. } | super::program_image::NativeSpec::ReachThrough { .. }))) {
                                reach_olds.insert(r.clone(), old);
                            } else {
                                m.no_delta(r);
                                moved_rels.insert(r.clone());
                            }
                        }
                        continue;
                    }
                    need.extend(reads.iter().cloned());
                    need.extend(scc.iter().cloned());
                    reached_strata.push((j, rules, moved));
                }
                // What the level's strata read of what is kept, read now in one batch, and held where unread.
                let need: Vec<String> = need.into_iter().collect();
                m.need(&need, &mut run.model)?;
                // A kept relation that is no stratum of this program (an earlier round's, decision 0331) is held too:
                // a stratum derived again reads it as it was. Only the program's own quiet strata were held (`unread`),
                // so `chain_below`'s reach, derived again in a later guest round, found `chain_step` and its seed read
                // but not held, and declined to SQL whenever the round chose to derive it again.
                let hold: Vec<&String> = need.iter().filter(|r| unread.contains(*r) || (!stratum_of.contains_key(r.as_str()) && !run.native_rels.contains(*r) && !f0.is_input(r))).collect();
                for r in hold {
                    let Some(o) = m.kept(r) else {
                        if unread.contains(r) {
                            return Err(Error::Plan(format!("{r}: its kept rows were not read")));
                        }
                        continue;
                    };
                    hold_quiet(&mut run, f0, db, r, o);
                }
                unread.retain(|r| !need.contains(r));
                for &j in &base_strata {
                    if native.contains("program") && !sccs[j].iter().all(|r| run.views.contains(r)) {
                        wants.extend(wants_of[j].iter().cloned());
                        specs.extend(run.seed_specs(&sccs[j], &by_stratum[j]));
                    }
                }
                for (j, rules, moved) in reached_strata {
                    let scc = &sccs[j];
                    if scc.contains("source_language_classification") {
                        crate::code_rule_dsl::prof::count("source_language.evaluations", 1);
                    }
                    let key = stratum_key(scc);
                    reached += 1;
                    // The evaluator's maintain mode (decision 0364 stage 3), where the stratum can take it: no form.
                    let olds: BTreeMap<String, Arc<super::native::component::InRel>> = scc.iter().filter_map(|r| m.kept(r).map(|k| (r.clone(), k))).collect();
                    // Its changed inputs' versions: the old rows the last run kept, and the deltas bound from the
                    // changed documents (`input_delta`).
                    // A view of an input's columns has its input's, through its columns. A relation a form maintained
                    // (or the mode, whose versions are its own) has its kept rows as old, and the form's net deltas
                    // (`r__minus :- r__del, !r__redo, !r__ins`, `r__plus :- r__ins, !r__old`) as minus and plus.
                    let ins: ModeInputs = moved.iter().filter_map(|r| {
                        let formed = |s: &str| super::native::component::held(db, &format!("{r}__{s}"));
                        // Only deltas this run made (`has_delta`): the engine may hold an earlier run's `r__minus`.
                        if !f0.is_input(r)
                            && m.has_delta(r)
                            && super::native::component::held(db, r).is_some_and(|x| x.start().is_none())
                            && let (Some(o), Some(mi), Some(pl)) = (m.kept(r), formed("minus"), formed("plus"))
                        {
                            let w = o.k;
                            let cols = |x: &Arc<super::native::component::InRel>| x.source((0..w).map(|c| sql_col(r, c)).collect());
                            let now = super::native::component::held(db, r)?;
                            return Some((r.clone(), ([cols(&o), cols(&mi), cols(&pl), cols(&now)], None)));
                        }
                        let (base, cols) = if f0.is_input(r) {
                            (r.clone(), None)
                        } else {
                            let [k] = rules_by.get(r.as_str()).map(Vec::as_slice).unwrap_or_default() else { return None };
                            let (v, rule) = run_vs.rule_at(*k);
                            let (base, cols) = input_view_shape(v, rule, |x| f0.is_input(x))?;
                            (base, Some(cols))
                        };
                        let [mi, pl] = m.given(&base)?;
                        let now = crate::code_rule_dsl::facts::vtab::bound_source(db, &base)?;
                        Some((r.clone(), ([m.old_input(&base)?, mi, pl, now], cols)))
                    }).collect();
                    // 0090 §4's threshold (0364 R4): a stratum whose changed reads' deltas reach past θ of its kept rows
                    // is derived again, its levels fresh from that derive. The deltas are their versions' sizes.
                    let delta_rows: usize = moved.iter().map(|r| match ins.get(r) {
                        Some((srcs, _)) => srcs[1].len() + srcs[2].len(),
                        None => super::native::component::held(db, r).map_or(0, |x| {
                            [super::native::component::Ver::Minus, super::native::component::Ver::Plus].into_iter().filter_map(|v| x.version_view(v)).map(|v| v.rows()).sum()
                        }),
                    }).sum();
                    let kept_rows: usize = olds.values().map(|o| o.rows()).sum();
                    let blocks = if crate::code_rule_dsl::logic::plan::corpus::maintain::past_threshold(delta_rows, kept_rows) {
                        Some(crate::code_rule_dsl::logic::plan::corpus::maintain::THRESHOLD.to_owned())
                    } else {
                        run.maintain_mode_blocks(scc, &rules, &moved, &olds, &ins)
                    };
                    if std::env::var_os("RERAIL_MODE_TRACE").is_some() {
                        eprintln!("RERAIL_MODE_TRACE {key}: {} ({delta_rows} delta rows, {kept_rows} kept)", blocks.as_deref().unwrap_or("the maintain mode"));
                    }
                    match blocks {
                        None => {
                            slots[j] = Slot::Mode { rules, moved, olds, ins };
                            continue;
                        }
                        Some(why) => {
                            *outcome.again.entry(crate::code_rule_dsl::logic::plan::corpus::maintain::reason_class(&why)).or_insert(0) += 1;
                            super::profile::decision("not the maintain mode (decision 0364)", || format!("{key}: {why}"))
                        }
                    }
                    // Derived again from its reads as they are now, as the base program derives it (decision 0364: no
                    // form). Its relations move; a kept one's delta is its diff against its kept rows, made once the
                    // level has run (`again_olds`), so its readers are maintained, not derived again in turn.
                    for r in scc {
                        moved_rels.insert(r.clone());
                        match m.kept(r) {
                            Some(old) => {
                                again_olds.insert(r.clone(), old);
                            }
                            None => m.no_delta(r),
                        }
                    }
                    if native.contains("program") {
                        specs.extend(run.seed_specs(scc, &rules));
                    }
                }
                if !wants.is_empty() {
                    let _wave = super::profile::span(|| "device wave (count tables, a maintained level)".into());
                    let (launches, rels) = super::native::component::prefetch_tables(db, &wants)?;
                    if launches > 0 {
                        super::profile::decision("device waves (decision 0095 §5)", || format!("a maintained level of {} strata: {launches} count tables over {rels} relations in one batch", end - at));
                    }
                }
                if !specs.is_empty() {
                    let _wave = super::profile::span(|| "level wave (joins, a maintained level)".into());
                    let sent = super::native::component::prefetch_joins(db, &specs)?;
                    if sent > 0 {
                        super::profile::decision("level joins (decision 0100 §5)", || format!("a maintained level of {} strata: {sent} of {} seed joins sent as one batch", end - at, specs.len()));
                    }
                }
            }
            match std::mem::replace(&mut slots[at], Slot::Base) {
                Slot::Quiet => continue,
                Slot::Base => {}
                Slot::Mode { rules, moved, olds, ins } => {
                    let key = stratum_key(scc);
                    let Some(ran) = run.maintain_mode(scc, &rules, &moved, olds, &ins)? else {
                        return Err(Error::Plan(format!("{key}: the maintain mode declined: {}", run.why_not.take().unwrap_or_default())));
                    };
                    if !ran {
                        outcome.unchanged.extend(scc.iter().cloned());
                        outcome.native.insert(key);
                        continue;
                    }
                    let recursive = rules.iter().any(|&r| reads_of(&run.versions, r).any(|x| scc.contains(x)));
                    m.mode_ran(scc, recursive);
                    for r in scc {
                        let rel = super::native::component::held(db, r).ok_or_else(|| Error::Plan(format!("{r}: not held after its maintain mode")))?;
                        let (gone, added) = rel.version_counts().ok_or_else(|| Error::Plan(format!("{r}: no version after its maintain mode")))?;
                        if added == 0 && gone == 0 {
                            outcome.unchanged.insert(r.clone());
                        } else {
                            moved_rels.insert(r.clone());
                            // Read from its marks when its frame's delta is written (`Delta::rows`), not copied now.
                            outcome.deltas.insert(r.clone(), Delta::Marks { gone, added });
                        }
                    }
                    outcome.native.insert(key);
                    continue;
                }
            }
        }
        let rules: Vec<RuleKey> = scc.iter().flat_map(|rel| rules_by.get(rel.as_str()).into_iter().flatten().copied()).collect();
        let key = scc.iter().cloned().collect::<Vec<_>>().join("/");
        if let Some(ks) = strata_kept.as_deref_mut() {
            let reads = stratum_reads(f0, &run_vs, scc, &rules);
            // A view of an input's columns (Q37) is never kept: it is held again in place, which is no work, so it
            // is clean as its input is. Its digest is its reads' with what it views (as a SQL view's, `view_digest`).
            if let Some((rel, view)) = run.input_view_stratum(scc, &rules) {
                let nows: Option<Vec<(String, u64)>> = reads
                    .iter()
                    .map(|r| {
                        let now = if fresh.contains_key(r) { fresh[r] } else if input_set.contains(r.as_str()) { ks.input(r) } else { None };
                        now.map(|d| (r.clone(), d))
                    })
                    .collect();
                if run.native_stratum_now(scc, &rules)? {
                    fresh.insert(rel, view_digest(nows.as_deref(), &[(view, Some(0))]));
                    clean_strata += 1;
                    continue;
                }
            }
            // A stratum the engine makes otherwise (a native operator, a lattice, a view, a produced request, a
            // tree pattern read off the documents) always runs.
            // A request this stratum makes and answers inside its fixpoint (decision 0065 §2a) is its native step:
            // Its reads include the observed answers. A changed answer invalidates the stratum.
            // (A request whose answers are all above the stratum is answered there, once its rows are whole.)
            let asks: Vec<&String> = scc.iter().filter(|r| f0.answers_of(r).iter().any(|p| scc.contains(*p))).collect();
            // A stratum of produced relations above their requests is their native step: their rows are the answers.
            let above: Vec<(&String, &str)> = scc.iter().filter_map(|r| f0.request_of(r).filter(|q| !scc.contains(*q)).map(|q| (r, q))).collect();
            let produced_only = rules.is_empty() && !above.is_empty() && above.len() == scc.len();
            let answering = (!asks.is_empty() || produced_only) && run.answer.is_some();
            // A view is made over what its stratum holds, so it is as its stratum's other relations and reads are.
            let plain = !scc.iter().any(|r| f0.native(r).is_some() || f0.lattice(r) || f0.request_of(r).is_some_and(|q| !(answering && (scc.contains(q) || produced_only))))
                && !rules.iter().any(|&r| run.versions.rule_at(r).1.body().any(|l| matches!(l.view(), Lit::Ast { .. })));
            let mut clean = plain && (!rules.is_empty() || produced_only) && scc.iter().all(|r| run.views.contains(r) || ks.has_rows(r));
            let mut nows: Option<Vec<(String, u64)>> = Some(Vec::new());
            // A stratum's reads of its own relations (its recursion) are what it derives from its other reads; one
            // that is an input too (rows given, and answers added to them) is read as an input.
            for r in reads.iter().filter(|r| !scc.contains(*r) || input_set.contains(r.as_str())) {
                let now = if fresh.contains_key(r) {
                    fresh[r]
                } else if input_set.contains(r.as_str()) {
                    ks.input(r)
                } else if rules_by.contains_key(r.as_str()) {
                    None
                } else {
                    // Neither derived nor an input: the program's own facts, which the reader's identity holds.
                    Some(0)
                };
                clean = clean && now.is_some() && scc.iter().filter(|x| !run.views.contains(*x)).all(|x| ks.read_was(x, r) == now);
                match (now, nows.as_mut()) {
                    (Some(d), Some(v)) => v.push((r.clone(), d)),
                    _ => nows = None,
                }
            }
            // The stratum's own requests, answered again from its kept rows: a native step's answers are a function of
            // its requests, so they are as they were exactly when the step's leaves (the files it read) are.
            // A request below the stratum that no stratum derives is the program's own facts (a constant world
            // request, `rr_dir_req("sub")`): not kept, so its answers cannot be replayed, and the stratum runs.
            if clean && answering && above.iter().any(|(p, q)| !ks.has_rows(q) && !run.answered.contains_key(p.as_str())) {
                clean = false;
            }
            let mut replayed: BTreeMap<String, BTreeSet<Vec<i64>>> = BTreeMap::new();
            if clean && answering {
                let _span = super::profile::span(|| "the stratum's requests answered again".into());
                for req in &asks {
                    let rows = kept_strata_rows(ks, f0, req)?;
                    let answer = run.answer.as_deref_mut().expect("answering");
                    for (p, got) in answer(req, &rows)? {
                        replayed.entry(p).or_default().extend(got);
                    }
                }
                // Produced relations above their requests: answered by the stratum below, when it ran or was clean,
                // else asked here, as their stratum asks when it runs.
                let mut by_req: BTreeMap<&str, Vec<&String>> = BTreeMap::new();
                for (p, q) in &above {
                    by_req.entry(*q).or_default().push(*p);
                }
                for (q, ps) in by_req {
                    if ps.iter().all(|p| run.answered.contains_key(p.as_str())) {
                        for p in ps {
                            replayed.entry(p.clone()).or_default().extend(run.answered[p.as_str()].iter().cloned());
                        }
                        continue;
                    }
                    let rows = kept_strata_rows(ks, f0, q)?;
                    let answer = run.answer.as_deref_mut().expect("answering");
                    for (p, got) in answer(q, &rows)? {
                        replayed.entry(p).or_default().extend(got);
                    }
                }
                for (p, _) in &above {
                    let now = answers_digest(f0, db, p, replayed.get(p.as_str()));
                    clean = clean && ks.read_was(p, &answer_read(p)) == Some(now);
                }
                for req in &asks {
                    for p in f0.answers_of(req) {
                        let now = answers_digest(f0, db, p, replayed.get(p));
                        clean = clean && scc.iter().filter(|x| !run.views.contains(*x)).all(|x| ks.read_was(x, &answer_read(p)) == Some(now));
                    }
                }
                if !clean {
                    super::profile::decision("a stratum's answers changed (t33 K3)", || format!("{key}: its requests answered again differ; it runs"));
                }
            }
            if clean {
                // Answers for a relation above this stratum go on to it, as they do when the stratum runs.
                for (p, rows) in replayed.into_iter().filter(|(p, _)| !scc.contains(p)) {
                    run.answered.entry(p).or_default().extend(rows);
                }
                let viewed = scc.iter().any(|r| run.views.contains(r));
                let kept_here: Vec<&String> = scc.iter().filter(|r| !run.views.contains(*r)).collect();
                for r in kept_here {
                    fresh.insert(r.clone(), ks.was(r));
                    // A view's relations are held now: the view reads them in place.
                    if viewed {
                        held_rows += kept_strata_hold(ks, db, &mut run, r)?;
                    } else {
                        unheld.insert(r.clone());
                    }
                }
                let members: Vec<(String, Option<u64>)> = scc.iter().filter(|r| !run.views.contains(*r)).map(|r| (r.clone(), ks.was(r))).collect();
                for v in scc.iter().filter(|r| run.views.contains(*r)) {
                    fresh.insert(v.clone(), view_digest(nows.as_deref(), &members));
                }
                clean_strata += 1;
                continue;
            }
            // A clean relation this stratum reads is held now, as the last run derived it.
            let wanted: Vec<String> = reads.iter().filter(|r| unheld.contains(*r)).cloned().collect();
            for r in wanted {
                held_rows += kept_strata_hold(ks, db, &mut run, &r)?;
                unheld.remove(&r);
                held_again += 1;
            }
            stratum_read_digests.insert(at, nows);
            run.logging = answering.then_some(at);
            evaluated.push(at);
        }
        let _stratum = super::profile::span(|| format!("stratum {}", if key.len() > 80 { format!("{}…({} relations)", &key[..80], scc.len()) } else { key.clone() }));
        // A produced relation above its request (decision 0065 §2a): its request is whole, so it is answered once, or
        // holds what a component's pauses answered. An input's rows are the caller's to write.
        if scc.iter().all(|r| f0.produced(r)) {
            for rel in scc {
                let rows = match run.answered.remove(rel) {
                    Some(rows) => rows,
                    None => {
                        let req = f0.request_of(rel).expect("a produced relation's request").to_owned();
                        let width = f0.width(&req);
                        // The request's rows where they are: the engine's (a relation it holds), none (one still to be
                        // made: no table, no rows), or its table's.
                        let native = run.native_rels.contains(&req).then(|| super::native::component::kept_rows(db, &req)).flatten();
                        let asked: Vec<Vec<i64>> = if let Some((_, flat, _)) = native.filter(|_| width > 0) {
                            flat.chunks(width).map(<[i64]>::to_vec).collect()
                        } else if run.pending.contains_key(&req) && !run.native_rels.contains(&req) {
                            Vec::new()
                        } else {
                            run.materialize(std::slice::from_ref(&req))?;
                            let cols = (0..width).map(|c| format!("c{c}")).collect::<Vec<_>>().join(", ");
                            db.prepare(&format!("SELECT {cols} FROM {}", crate::code_rule_dsl::logic::sql::ident(&req)))?
                                .query_map([], |r| (0..width).map(|c| r.get::<_, i64>(c)).collect::<rusqlite::Result<Vec<_>>>())?
                                .collect::<rusqlite::Result<_>>()?
                        };
                        let answer = run.answer.as_deref_mut().ok_or_else(|| Error::Plan(format!("{req}: a request, and no one to answer it")))?;
                        let mut mine = Vec::new();
                        for (p, got) in answer_complete(answer, &req, &asked)? {
                            if p == *rel { mine.extend(got) } else { run.answered.entry(p).or_default().extend(got) }
                        }
                        mine
                    }
                };
                if let Some(at) = run.logging {
                    run.answer_logs.entry(at).or_default().entry(rel.clone()).or_default().extend(rows.iter().cloned());
                }
                // A later program can reuse this producer even when the current cone has no local reader.
                // Retain its actual answers alongside their observed digest whenever this stratum publishes frames.
                if f0.is_input(rel) || rows.is_empty() {
                    continue;
                }
                if strata_kept.is_none()
                    && !body::Versions::keys(0, p0.len()).any(|r| reads_of(&run.versions, r).any(|a| a == rel.as_str()))
                {
                    continue;
                }
                // Held by the native engine, as a component's own answers are (`answer_pending`): a native stratum above
                // reads it there, and a SQL one has it moved into its table (`Run::materialize`).
                let types: Vec<u8> = f0.types(rel).iter().map(|t| sym_char(&run.model.inputs, t)).collect::<String>().into_bytes();
                // `retain` replaces what the engine holds: rows it already holds (the relation's program facts) stay,
                // so its readers read the facts and the answers both.
                let mut rows = rows;
                if run.native_rels.contains(rel) {
                    if let Some((_, flat, k)) = super::native::component::kept_rows(db, rel).filter(|(_, _, k)| *k == types.len() && *k > 0) {
                        rows.extend(flat.chunks(k).map(<[i64]>::to_vec));
                    }
                }
                super::native::component::retain(db, rel, rows, types)?;
                run.native_rels.insert(rel.clone());
            }
            continue;
        }
        let rule_no = |r: RuleKey| r.number().expect("a rule of the program");
        let skey = stratum_key(scc);
        let mut why = if native.contains("program") { String::new() } else { format!("the native engine is off (modes: {native:?})") };
        if super::profile::on() {
            for &r in &rules {
                // The rule's text, as its version shows it.
                super::profile::note(format!("rule #{:04}", rule_no(r)), || { let (v, rule) = run.versions.rule_at(r); v.show_rule(rule) });
            }
        }
        // Python operators consume completed demands, not their synthetic locality-proof rules. Keep native
        // inputs and outputs in the set store before the SQL fallback can materialize them.
        use super::program_image::NativeSpec;
        if atoms && scc.iter().any(|r| matches!(f0.native(r).map(|n| &n.spec), Some(NativeSpec::Python { .. } | NativeSpec::PythonCandidates { .. }))) {
            let rel = scc.iter().next().unwrap();
            let (kind, input, targets) = match f0.native(rel).map(|n| &n.spec) {
                Some(NativeSpec::Python { kind, input, targets }) => (f0.text(*kind), f0.rel_name(*input), targets.map(|t| f0.rel_name(t))),
                Some(NativeSpec::PythonCandidates { input, targets, .. }) => ("candidates", f0.rel_name(*input), Some(f0.rel_name(*targets))),
                _=>return Err(Error::Plan("recursive Python semantic demands are not supported".into())),
            };
            if scc.len() != 1 || scc.contains(input) || targets.is_some_and(|t| scc.contains(t)) {
                return Err(Error::Plan("recursive Python semantic demands are not supported".into()));
            }
            let n = run.python_relation(rel, kind, input, targets)?;
            super::profile::stratum(&skey, "native fact operator over retained demand relations", String::new);
            super::profile::note(format!("{rel} rows"), || n.to_string());
            outcome.native.insert(key);
            continue;
        }
        // The reach index (decision 0119) over the relations the engine holds: no row made, read in place.
        if let [rel] = scc.iter().collect::<Vec<_>>().as_slice()
            && let Some(native @ (NativeSpec::Reach { .. } | NativeSpec::ReachThrough { .. })) = f0.native(rel).map(|n| &n.spec)
            && let Some((n, why)) = run.native_reach(rel, native)?
        {
            super::profile::stratum(&skey, "reach index over held relations (decision 0119)", || why);
            super::profile::note(format!("{rel} rows"), || n.to_string());
            outcome.native.insert(key);
            if let Some(old) = reach_olds.remove(rel.as_str())
                && let Some(m) = maintaining.as_deref_mut()
            {
                if kept_delta(db, rel, &old, &mut run.native_rels)? {
                    m.delta_made(rel);
                    moved_rels.insert((*rel).clone());
                } else {
                    outcome.unchanged.insert((*rel).clone());
                }
            }
            continue;
        }
        // A reach index not made in place (`reach_rows`) has no delta: its readers derive again.
        if let Some(m) = maintaining.as_deref_mut() {
            for r in scc.iter().filter(|r| reach_olds.remove(r.as_str()).is_some()) {
                m.no_delta(r);
                moved_rels.insert(r.clone());
            }
        }
        // A text test over strings the run answered, from the rows the engine holds: no table read or written. With
        // the device on, its rule runs as placed (the device's text-test form, `native_stratum_`), never on the host
        // in the device's place.
        if let [rel] = scc.iter().collect::<Vec<_>>().as_slice()
            && !placement::device_on()
            && let Some(NativeSpec::TextTest { input, column, op, patterns }) = f0.native(rel).map(|n| &n.spec)
            && text_test_defined(&run.versions, &rules, f0.rel_name(*input))
            && let Some(n) = run.native_text_test(rel, f0.rel_name(*input), run.native_rels.contains(f0.rel_name(*input)), *column, f0.text(*op), &patterns.iter().map(|&p| f0.text(p).to_owned()).collect::<Vec<_>>())?
        {
            super::profile::stratum(&skey, "text test over a held relation (the run's vocabulary)", String::new);
            super::profile::note(format!("{rel} rows"), || n.to_string());
            outcome.native.insert(key);
            continue;
        }
        // The import components (`components.rerail`) from the rows the engine holds: no table read or written.
        if let [rel] = scc.iter().collect::<Vec<_>>().as_slice()
            && let Some(NativeSpec::Components { edge, own }) = f0.native(rel).map(|n| &n.spec)
            && let Some(n) = run.native_components(rel, f0.rel_name(*edge), f0.rel_name(*own))?
        {
            super::profile::stratum(&skey, "native fact operator over held relations", String::new);
            super::profile::note(format!("{rel} rows"), || n.to_string());
            outcome.native.insert(key);
            continue;
        }
        // A count below a bound (`rank_form`) from the rows the engine holds, likewise.
        if let [rel] = scc.iter().collect::<Vec<_>>().as_slice()
            && let Some(NativeSpec::Rank { query, counted, group, op }) = f0.native(rel).map(|n| &n.spec)
            && let Some(n) = run.native_rank(rel, f0.rel_name(*query), f0.rel_name(*counted), *group, f0.text(*op))?
        {
            super::profile::stratum(&skey, "count below a bound over held relations (decision 0361 step 7)", String::new);
            super::profile::note(format!("{rel} rows"), || n.to_string());
            outcome.native.insert(key);
            continue;
        }
        // `match_all` over the input the engine holds, likewise.
        if let [rel] = scc.iter().collect::<Vec<_>>().as_slice()
            && let Some(NativeSpec::MatchAll { input, pattern, groups, positions }) = f0.native(rel).map(|n| &n.spec)
            && let Some(n) = {
                let groups: Vec<String> = groups.iter().map(|&g| f0.text(g).to_owned()).collect();
                run.native_match_all(rel, f0.rel_name(*input), f0.text(*pattern), &groups, *positions)?
            }
        {
            super::profile::stratum(&skey, "native fact operator over held relations", String::new);
            super::profile::note(format!("{rel} rows"), || n.to_string());
            outcome.native.insert(key);
            continue;
        }
        // `dfs_first` (and `_memo`) from the rows the engine holds, likewise.
        if let [rel] = scc.iter().collect::<Vec<_>>().as_slice()
            && let Some(NativeSpec::DfsFirst { edge, target, roots }) = f0.native(rel).map(|n| &n.spec)
            && let Some(n) = run.native_dfs_first(rel, f0.rel_name(*edge), f0.rel_name(*target), roots.map(|r| f0.rel_name(r)))?
        {
            super::profile::stratum(&skey, "native fact operator over held relations", String::new);
            super::profile::note(format!("{rel} rows"), || n.to_string());
            outcome.native.insert(key);
            continue;
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
            // Not planned at all where the batch is off (`RERAIL_LEVEL_JOINS=off`), nor in a maintained run (a
            // session's part, `maintaining`): its seeds join deltas of a few rows, below the batch's `MIN_LEFT`, so
            // planning and specifying them ahead only doubled their planning (O125 steps 01-03: -5% cycles without
            // it). `RERAIL_LEVEL_JOINS=on` batches them as before.
            let batched = super::native::setfix::level::on() && (maintaining.is_none() || std::env::var("RERAIL_LEVEL_JOINS").is_ok_and(|v| v == "on"));
            let native_ones: Vec<usize> = if !batched { Vec::new() } else { level.iter().copied().filter(|&j| !sccs[j].iter().all(|r| run.views.contains(r))).collect() };
            // Chained (decision 0100 §11): the next level's seed joins whose relations are already held go in this
            // batch too, so their launch and its latency run under this level's strata, not before the next's.
            // `seed_specs` keeps only those whose every atom is a relation run natively so far.
            let later: Vec<usize> = if !batched { Vec::new() } else { (at..sccs.len()).map(|j| level_of[j]).filter(|&l| l > level_of[at]).min().map_or_else(Vec::new, |next| {
                (at..sccs.len()).filter(|&j| level_of[j] == next && !waved[j] && !sccs[j].iter().all(|r| run.views.contains(r))).collect()
            }) };
            // Decision 0123: the statistics the level's plans will read (and the next level's, chained, where its joins
            // are batched), requested by each line that will be priced and made in one batch, before any of them is
            // planned. Made whether or not the level's seed joins are batched: its strata plan with them either way.
            if run.model.has_deferred() {
                let mut wanted = BTreeSet::new();
                let strata: Vec<(&BTreeSet<String>, Vec<RuleKey>)> = level.iter().chain(&later).map(|&j| (&sccs[j], run.runnable(&sccs[j], &by_stratum[j]))).collect();
                let vs = Arc::clone(&run.versions);
                run.model.wants_strata(&vs, strata, &mut wanted)?;
                run.model.make_statistics(f0, &wanted)?;
            }
            let mut specs: Vec<Arc<JoinSpec>> = native_ones.iter().flat_map(|&j| run.seed_specs(&sccs[j], &by_stratum[j])).collect();
            let own = specs.len();
            specs.extend(later.iter().flat_map(|&j| run.seed_specs(&sccs[j], &by_stratum[j])));
            if !specs.is_empty() {
                let _wave = super::profile::span(|| "level wave (joins)".into());
                let sent = super::native::component::prefetch_joins(db, &specs)?;
                if sent > 0 {
                    super::profile::decision("level joins (decision 0100 §5)", || {
                        format!("a level of {} strata: {sent} of {} seed joins sent as one batch ({} of them the next level's, chained)", level.len(), specs.len(), specs.len() - own)
                    });
                }
            }
        }
        if native.contains("program") && !scc.iter().all(|r| run.views.contains(r)) {
            let native_span = super::profile::span(|| "native".into());
            let t0 = level_trace().map(|_| thread_cpu());
            // Decision 0113: the strata ready after this one are issued as components; this one, if a component's,
            // is retired from it (its first member waits for the component's job, or runs it here).
            let got = match board.as_mut() {
                Some(b) => {
                    let rules_of = |j: usize| -> Vec<RuleKey> { sccs[j].iter().flat_map(|rel| rules_by.get(rel.as_str()).into_iter().flatten().copied()).collect() };
                    let mut load_early = |j: usize| -> Result<()> {
                        if let Some(load) = hooks.load.as_deref_mut() {
                            let now: Vec<String> = std::mem::take(&mut loads[j]);
                            if !now.is_empty() {
                                load(&now)?;
                            }
                        }
                        Ok(())
                    };
                    b.collect(&mut run, stream)?;
                    b.issue(at, &mut run, &sccs, &rules_of, &wants_of, &mut waved, &mut load_early, stream, &ambient)?;
                    b.retire_at(at, &mut run, stream, &ambient, &mut |b, run| b.issue(at, run, &sccs, &rules_of, &wants_of, &mut waved, &mut load_early, stream, &ambient))?
                }
                None => issue::Got::Not,
            };
            let ran = match got {
                issue::Got::Ran => Native::Ran,
                issue::Got::Declined(why, class) => {
                    run.why_not = why;
                    run.why_class = class;
                    Native::Declined
                }
                issue::Got::Not | issue::Got::Here => run.native_stratum(scc, &rules)?,
            };
            drop(native_span);
            // `RERAIL_LEVEL_TRACE=<file>`: each native stratum's level and thread CPU (decision 0100 step 6).
            if let (Some(path), Some(t0)) = (level_trace(), t0) {
                use std::io::Write as _;
                if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                    let _ = writeln!(f, "stratum\t{}\t{at}\t{:.6}\t{}\t{}", level_of[at], thread_cpu() - t0, match ran { Native::Ran => "ran", Native::Waiting(_) => "waiting", Native::Asking(..) => "asking", Native::Declined => "declined" }, rules.len());
                }
            }
            match ran {
                Native::Ran => {
                    outcome.native.insert(key);
                    continue;
                }
                Native::Waiting(d) => {
                    waiting.push((at, key, d));
                    continue;
                }
                Native::Asking(d, tickets) => {
                    asked.push(Asked { at, key, d, tickets, since: std::time::Instant::now() });
                    continue;
                }
                Native::Declined => {}
            }
            why = format!("the native engine declined: {}", run.why_not.take().unwrap_or_else(|| "no reason given".into()));
            if scc.iter().any(|g| f0.gate(g)) {
                return Err(Error::Plan(format!("gated negations in {scc:?} need native steps for every rule")));
            }
            if scc.iter().any(|r| f0.request(r)) && scc.iter().any(|r| f0.produced(r)) {
                return Err(Error::Plan(format!("requests in {scc:?} are answered only inside the native fixpoint: {why}")));
            }
        }
        // This stratum runs in SQL, which reads the native relations it joins from their tables.
        super::profile::strict_declined(&skey, &why, run.why_class.as_deref());
        let read: Vec<String> = rules.iter().flat_map(|&r| reads_of(&run.versions, r).map(str::to_owned).collect::<Vec<_>>()).collect();
        run.materialize(&read)?;
        // This stratum runs in SQL: its relations' and those it reads' tables.
        let read: Vec<String> = rules.iter().flat_map(|&r| reads_of(&run.versions, r).map(str::to_owned).collect::<Vec<_>>()).chain(scc.iter().cloned()).collect();
        run.ensure(&read)?;
        // What it reads that the engine holds (a relation of facts alone, decision 0104) is moved to its table first.
        run.materialize(&read)?;
        // A relation made natively (`dfs_first`): its inputs' rows in, the operator's rows out; its rule only
        // ordered it after them.
        if let [rel] = scc.iter().collect::<Vec<_>>().as_slice()
            && let Some(native) = f0.native(rel)
        {
            super::profile::stratum(&skey, "native fact operator over demanded input relations", || why.clone());
            let n = run.native_relation(rel, native)?;
            super::profile::note(format!("{rel} rows"), || n.to_string());
            run.refresh(rel, false)?;
            for c in 0..f0.width(rel) {
                let d = run.count(&format!("SELECT count(DISTINCT c{c}) FROM {}", crate::code_rule_dsl::logic::sql::ident(rel)))?;
                run.model.ndv.insert(((*rel).clone(), c), if d == 0 { 1.0 } else { d as f64 });
            }
            continue;
        }
        let mut planned = Vec::new();
        let mut explained = Vec::new();
        // A round (decision 0123): the statistics its priced plans read, in one batch.
        let mut wanted = BTreeSet::new();
        let vs = Arc::clone(&run.versions);
        let bodies: Vec<Body<'_>> = rules.iter().map(|&r| vs.at(r)).collect();
        run.model.wants(bodies.iter().map(|b| (b, None)), &mut wanted);
        run.model.make_statistics(f0, &wanted)?;
        let planning = super::profile::span(|| "plan".into());
        plan_site("SQL");
        for (&rule, b) in rules.iter().zip(&bodies) {
            run.model.late(f0, b, None, false)?;
            let t0 = if super::profile::on() { super::profile::now() } else { 0.0 };
            let ((order, work, _), x) = run.model.plan_rule_reused(f0, b, None)?;
            explained.push((x, if super::profile::on() { super::profile::now() - t0 } else { 0.0 }));
            run.ensure_indexes(Order::new(b, &order))?;
            outcome.chosen.push(Chosen {
                rule: rule.number().expect("a rule of the program"),
                steps: order.iter().map(|s| (s.at, s.mode)).collect(),
                // The head's text is the tree's (`Display`), written from its handles.
                head: b.show_head(),
                anchor: order.first().map_or_else(|| "scalar".into(), |s| b.show(s.at)),
                walks: order.iter().filter_map(|s| s.mode.map(|m| (b.show(s.at), m.name()))).collect(),
                work: work.round_ties_even() as i64,
            });
            planned.push((rule, order));
        }
        drop(planning);
        let planned_bodies = &bodies;
        if scc.iter().all(|r| run.views.contains(r)) {
            super::profile::stratum(&skey, "SQL view", || "a view over the inputs: SQLite reads it in place where a rule joins it".into());
            continue;
        }
        let recursive =
            scc.len() > 1 || planned.iter().zip(planned_bodies).any(|((_, o), b)| Order::new(b, o).lits().any(|l| matches!(l, Lit::Atom(a) if !a.neg() && scc.contains(b.rel(a)))));
        let mut ran_native=false;
        if recursive {
            for r in scc {db.execute(&format!("CREATE TEMP TABLE delta_{r} AS SELECT * FROM {} WHERE 0", crate::code_rule_dsl::logic::sql::ident(r)),[])?;}
            if native.contains("fixpoint") && run.native_fixpoint(scc,&rules)? {ran_native=true;outcome.native.insert(key.clone());}
            else if native.contains("setfix") && run.native_set_fixpoint(scc,&rules)? {ran_native=true;outcome.native.insert(key.clone());}
        }
        if !ran_native {
            let n=sql_kernel::component(&mut run,scc,&planned,recursive)?;
            super::profile::stratum(&skey,"SQL wave kernel",||format!("{why}; {n} waves"));
            let class = run.why_class.take().unwrap_or_else(|| if native.contains("program") { "no reason given".into() } else { "the native engine is off".into() });
            // The device's hazards too: a stratum the native engine declines is never placed, so its shape is read here.
            let lacking = if super::profile::on() {
                let mut shape = placement::Shape::default();
                for ((_, order), b) in planned.iter().zip(planned_bodies) {
                    shape.add(Order::new(b, order), 0.0, recursive, f0.lattice_name(b.rel(b.head())) == Some("shortlex_min"));
                }
                shape.lacking()
            } else {
                Vec::new()
            };
            placement::record_fallback(&skey, &class, &why, lacking);
            super::profile::fallback(&skey, class);
            if recursive {outcome.iterations.insert(key,n as usize);}
            for ((r,_),(x,took)) in planned.iter().zip(&explained){
                if let Some(x)=x{super::profile::planned(&skey,&format!("sql wave {}",vs.head_at(*r)),r.number(),x,*took,0,None,None);}
            }
        }
        let _refresh = super::profile::span(|| "refresh and distinct counts".into());
        for r in scc {
            run.refresh(r, false)?;
            for c in 0..f0.width(r) {
                let d = run.count(&format!("SELECT count(DISTINCT c{c}) FROM {}", crate::code_rule_dsl::logic::sql::ident(r)))?;
                run.model.ndv.insert((r.clone(), c), if d == 0 { 1.0 } else { d as f64 });
            }
        }
    }
    // The strata still asking, and those waiting on their waves, finish (each may wait on the other's).
    while !asked.is_empty() || !waiting.is_empty() {
        finish_wave(&mut run, &mut waiting, &mut outcome, &mut carry, &sccs, &last, &mut asked)?;
        while !asked.is_empty() {
            if let Some(b) = board.as_mut() {
                b.collect(&mut run, stream)?;
            }
            take_answers(&mut run, &mut asked, &mut arrived, &mut waiting, &mut outcome, &mut carry, &sccs, &last, std::time::Duration::from_millis(1))?;
        }
    }
    // A debugging aid: `RERAIL_REL_ROWS=<file>` appends each derived relation's rows; `RERAIL_REL_DIR=<dir>` writes
    // every relation the run reads or derives, one directory per run. Its reads are behind the run (strict mode's
    // waves are over), so a strict run can be dumped.
    if let Some(path) = std::env::var_os("RERAIL_REL_ROWS") {
        super::profile::behind(|| -> Result<()> {
        use std::io::Write as _;
        static RUNS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let at = RUNS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).map_err(|e| Error::Plan(e.to_string()))?;
        let _ = writeln!(f, "== run {at}");
        let mut all: BTreeSet<String> = sccs.iter().flatten().cloned().collect();
        all.extend(body::Versions::keys(0, p0.len()).flat_map(|r| reads_of(&run.versions, r).map(str::to_owned).collect::<Vec<_>>()));
        let dir = std::env::var_os("RERAIL_REL_DIR").map(|d| std::path::Path::new(&d).join(format!("run{at}")));
        if let Some(d) = &dir { let _ = std::fs::create_dir_all(d); }
        for r in &all {
            let h = super::native::component::held(run.db, r);
            let n = h.as_ref().map(|h| h.rows() as i64).unwrap_or_else(|| run.count(&format!("SELECT count(*) FROM {}", crate::code_rule_dsl::logic::sql::ident(r))).unwrap_or(-1));
            let _ = writeln!(f, "{r}\t{n}");
            if let Some(d) = &dir {
                let text: String = match h {
                    // Its live rows: flat when it holds them, else read through its view (a view holds no row flat).
                    Some(h) => {
                        let k = h.types().len().max(1);
                        let line = |r: &[i64]| r.iter().map(|v| v.to_string()).collect::<Vec<_>>().join("\t") + "\n";
                        match h.live_rows() {
                            Some(flat) => flat.chunks(k).map(line).collect(),
                            None => (0..h.len()).map(|i| line(&h.values(i))).collect(),
                        }
                    }
                    None => {
                        let k = f0.arity(r).unwrap_or(0);
                        let sql = format!("SELECT * FROM {}", crate::code_rule_dsl::logic::sql::ident(r));
                        let mut out = String::new();
                        if let Ok(mut st) = run.db.prepare(&sql) {
                            let k = if k == 0 { st.column_count() } else { k.min(st.column_count()) };
                            if let Ok(mut rows) = st.query([]) {
                                while let Ok(Some(row)) = rows.next() {
                                    let vals: Vec<String> = (0..k).map(|i| row.get::<_, rusqlite::types::Value>(i).map(|v| match v { rusqlite::types::Value::Integer(n) => n.to_string(), rusqlite::types::Value::Text(t) => t, rusqlite::types::Value::Real(x) => x.to_string(), _ => String::new() }).unwrap_or_default()).collect();
                                    out.push_str(&vals.join("\t"));
                                    out.push('\n');
                                }
                            }
                        }
                        out
                    }
                };
                let _ = std::fs::write(d.join(format!("{r}.tsv")), text);
            }
        }
        Ok(())
        })?;
    }
    Ok(())
    })?;
    if let Some((p, a)) = loop_open.take() {
        loop_times[p] = Some((a, strata_t0.elapsed().as_secs_f64(), 0));
    }
    if let Some(b) = board.as_mut() {
        b.mark(sccs.len());
    }
    // Decision 0203 §4: the run's critical path by its lines' work, its heavy lines noted; decision 0208: the path as
    // the strata ran, by their measured times, which notes the lines.
    if joins::on() {
        let lines = lines_take();
        if !lines.is_empty() {
            let of: HashMap<&str, usize> = sccs.iter().enumerate().flat_map(|(j, scc)| scc.iter().map(move |r| (r.as_str(), j))).collect();
            let producers: Vec<Vec<usize>> = sccs.iter().enumerate().map(|(j, scc)| {
                let from: BTreeSet<usize> = scc.iter().flat_map(|r| deps.get(r).into_iter().flatten()).filter_map(|(d, _)| of.get(d.as_str()).copied()).filter(|&p| p != j).collect();
                from.into_iter().collect()
            }).collect();
            let names: Vec<String> = sccs.iter().map(|s| s.iter().next().cloned().unwrap_or_default()).collect();
            let timeline = board.as_ref().map_or_else(|| loop_times.clone(), |b| b.timeline());
            let path = joins::critical_timed(&names, &producers, &lines, Some(&timeline));
            super::profile::decision("critical path (decision 0203)", || format!("{} strata, {:.0} rows of work; {} lines noted", path.strata.len(), path.work, path.lines.len()));
            joins::log(|| format!("critical path: {} of {} strata, {:.0} rows of work ({} lines, {:.0} in all); {} lines noted; {} searches waiting", path.strata.len(), sccs.len(), path.work, lines.len(), lines.iter().map(|l| l.work).sum::<f64>(), path.lines.len(), joins::requests()));
            joins::log(|| format!("lines: {}", lines.iter().filter(|l| l.work >= 1_000.0).map(|l| format!("{} {:.0} (priced {:.0}, {})", l.stratum, l.work, l.est, l.order.iter().map(|s| s.0.to_string()).collect::<Vec<_>>().join("."))).collect::<Vec<_>>().join(", ")));
            if let Some(m) = &path.measured {
                super::profile::decision("measured critical path (decision 0208)", || format!("{} strata, {:.3} s busy, {:.3} s waiting; {} of them on the predicted path", m.strata.len(), m.busy, m.waited, m.shared));
                joins::log(|| format!("measured critical path: {} strata, {:.3} s busy, {:.3} s waiting ({:.3} s from its first start to its end); {} of the {} predicted strata on it: {}", m.strata.len(), m.busy, m.waited, m.span, m.shared, path.strata.len(), m.strata.iter().map(|&j| names[j].as_str()).collect::<Vec<_>>().join(" > ")));
            }
        }
    }
    if let Some(b) = board.as_mut() {
        b.dump(&level_of, &sccs);
    }
    if let Some(b) = &board {
        super::profile::decision("strata issued ahead (decision 0113)", || format!("{:?}", b.counts));
        if std::env::var_os("RERAIL_STRATA_COUNTS").is_some() {
            let r = std::sync::atomic::Ordering::Relaxed;
            eprintln!(
                "RERAIL_STRATA_COUNTS {:?} answers {} in {:.3} s; suspended on requests {} times, {:.3} s in all",
                b.counts,
                ANSWER_CALLS.load(r),
                ANSWER_WAIT.load(r) as f64 * 1e-9,
                ASK_SUSPENSIONS.load(r),
                ASK_SUSPENDED.load(r) as f64 * 1e-9
            );
        }
    }
    finish_wave(&mut run, &mut waiting, &mut outcome, &mut carry, &sccs, &last, &mut asked)?;
    super::lanes::prof::level(None);
    if let Some(m) = maintaining.as_deref_mut() {
        m.walked(reached, quiet_slots, sccs.len());
    }
    if let Some(ks) = strata_kept.as_deref_mut() {
        for k in std::mem::take(&mut evaluated) {
            let reads = with_answers(f0, db, &sccs[k], stratum_read_digests.remove(&k).flatten(), run.answer_logs.remove(&k));
            kept_strata_ran(ks, db, &run, &sccs[k], reads.as_ref(), &mut fresh);
        }
        // What stays to the run's end (outputs, relations kept for the caller) is held again if it was clean.
        let wanted: Vec<String> = unheld.iter().filter(|r| stays.contains(*r)).cloned().collect();
        for r in wanted {
            held_rows += kept_strata_hold(ks, db, &mut run, &r)?;
            unheld.remove(&r);
            held_again += 1;
        }
        ks.counted(sccs.len() - clean_strata, sccs.len());
        super::profile::decision("cross-file strata kept across runs (decision 0077 §4)", || {
            format!("{clean_strata} of {} strata not run: their reads had the rows they had; {held_again} of their relations held again ({held_rows} rows)", sccs.len())
        });
    }
    // The chunk's end: pairs no seed took, and what the lanes kept for the chunk's joins; the executor's scoreboard
    // so far, to its trace (`RERAIL_SCOREBOARD`).
    super::native::component::level_clear();
    super::lanes::executor().note("a chunk's end");
    if let Some(path) = level_trace() {
        use std::io::Write as _;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let (sent, used) = super::native::component::level_counts();
            let _ = writeln!(f, "chunk\t{sent}\t{used}");
        }
    }
    // What the caller reads of the relations left stored (decision 0117 step 4): read and held; the rest stand.
    if let Some(m) = maintaining.as_deref_mut() {
        let wanted: Vec<String> = unread.iter().filter(|r| f0.is_output(r) || m.at_end(r)).cloned().collect();
        if !wanted.is_empty() {
            m.need(&wanted, &mut run.model)?;
            for r in &wanted {
                let Some(o) = m.kept(r) else { return Err(Error::Plan(format!("{r}: its kept rows were not read"))) };
                hold_quiet(&mut run, f0, db, r, o);
                unread.remove(r);
            }
        }
        outcome.unread = std::mem::take(&mut unread);
    }
    let _outputs = super::profile::span(|| "outputs".into());
    let kept = run.model.inputs.kept.clone();
    outcome.kept = kept.iter().filter(|r| run.native_rels.contains(*r)).cloned().collect();
    let written: Vec<String> = f0.outputs().filter(|r| !outcome.kept.contains(*r) && !handed.contains(*r)).map(str::to_owned).collect();
    // An output the native engine holds is read where it is: writing it to a table to read it back is a wait on
    // SQLite between this run and its reader (the guests' requests start the next wave). `finding` joins the nodes'
    // and documents' inputs, in SQL.
    let native_out: BTreeSet<String> = if atoms {
        // `finding` too, when the nodes and documents it is decoded with are bound in place (`finding_rows`).
        // Only the findings form (`finding(key, n)`) is decoded with them: a `finding` of another shape is an output
        // like any other.
        let findings_form = |r: &str| r == "finding" && f0.columns(r).is_none_or(|c| c.len() == 2);
        let in_place = |r: &str| !findings_form(r) || ["node", "doc"].iter().all(|t| crate::code_rule_dsl::facts::vtab::bound_source(db, t).is_some());
        written.iter().filter(|r| in_place(r) && run.native_rels.contains(*r) && !run.inputs.contains(r.as_str()) && super::native::component::held(db, r).is_some()).cloned().collect()
    } else {
        BTreeSet::new()
    };
    let to_sql: Vec<String> = written.iter().filter(|r| !native_out.contains(*r)).cloned().collect();
    run.materialize(&to_sql)?;
    let outputs_of: Vec<&str> = f0.outputs().collect();
    let (outputs, nodes) = decoded_outputs(&outputs_of, &|r| f0.is_input(r), &|r| f0.columns(r).map(|c| c.into_iter().map(|(_, t)| t.to_owned()).collect()), db, lowered, &facts, atoms, &kept, &native_out)?;
    let mut outputs=outputs;let mut nodes=nodes;
    for rel in &sentinels {
        if let Some(rows)=outputs.get_mut(rel){*rows=std::sync::Arc::new(rows.without_first());}
        if let Some(cols)=nodes.get_mut(rel){*cols=cols.iter().filter_map(|i|i.checked_sub(1)).collect();}
    }
    outcome.outputs = outputs;
    outcome.nodes = nodes;
    outcome.handed = handed;
    outcome.inlined = inlined_rels;
    outcome.quiet = (run.quiet.strata.load(std::sync::atomic::Ordering::Relaxed), run.quiet.unmeasured.load(std::sync::atomic::Ordering::Relaxed));
    // The program's analysis is freed on the pool: on a pack's program, its free was 3% of the main thread.
    crate::threads::drop_behind(std::mem::take(&mut run.model.analysis));
    Ok(outcome)
}

/// Whether `e` is a strand's read of SQL (`Db::sql`): the stratum runs on the run's thread instead.
fn off_thread(e: &Error) -> bool {
    e.to_string().contains("off the run's thread")
}

/// The strata in order of their levels (stable; with `sort`), and each one's level: one more than the deepest level of a stratum
/// it depends on (`deps`: reads, requests answered, a native operator's inputs). A level's strata do not depend on
/// each other, so it is a device wave (decision 0095 §5).
fn by_level(sccs: Vec<BTreeSet<String>>, deps: &super::front::Edges, sort: bool) -> (Vec<BTreeSet<String>>, Vec<usize>) {
    let of: HashMap<&str, usize> = sccs.iter().enumerate().flat_map(|(j, scc)| scc.iter().map(move |r| (r.as_str(), j))).collect();
    let mut level = vec![0usize; sccs.len()];
    for j in 0..sccs.len() {
        level[j] = sccs[j]
            .iter()
            .flat_map(|r| deps.get(r).into_iter().flatten())
            .filter_map(|(d, _)| of.get(d.as_str()).copied())
            .filter(|&d| d < j)
            .map(|d| level[d] + 1)
            .max()
            .unwrap_or(0);
    }
    let mut order: Vec<usize> = (0..sccs.len()).collect();
    if sort {
        order.sort_by_key(|&j| level[j]);
    }
    let levels = order.iter().map(|&j| level[j]).collect();
    let mut sccs: Vec<Option<BTreeSet<String>>> = sccs.into_iter().map(Some).collect();
    (order.into_iter().map(|j| sccs[j].take().expect("each stratum once")).collect(), levels)
}

/// Runs the device wave the waiting strata wait on (`lanes::wave::flush`: their launches as one batch), then
/// resumes each in order; one that waits again (a second form's launches) waits on the next wave.
fn finish_wave<'a>(run: &mut Run<'a, '_>, waiting: &mut Vec<(usize, String, Box<Driving>)>, outcome: &mut Outcome, carry: &mut Vec<String>, sccs: &[BTreeSet<String>], last: &HashMap<String, usize>, asked: &mut Vec<Asked>) -> Result<()> {
    while !waiting.is_empty() {
        {
            let _wave = super::profile::span(|| "device wave (kernel launches)".into());
            super::lanes::wave::flush(waiting.len()).map_err(Error::Plan)?;
        }
        for (j, key, d) in std::mem::take(waiting) {
            let _stratum = super::profile::span(|| format!("stratum {} (after its device wave)", if key.len() > 80 { format!("{}…", &key[..80]) } else { key.clone() }));
            match run.native_resume(d)? {
                Native::Ran => {
                    outcome.native.insert(key);
                    carry.extend(sccs[j].iter().filter(|r| !last.contains_key(*r)).cloned());
                }
                Native::Waiting(d) => waiting.push((j, key, d)),
                Native::Asking(d, tickets) => asked.push(Asked { at: j, key, d, tickets, since: std::time::Instant::now() }),
                Native::Declined => return Err(Error::Plan(format!("{key}: a stratum that waited on its wave declined"))),
            }
        }
    }
    Ok(())
}

/// A stratum suspended on its requests (decision 0114 §3): its position and key, its driver's state, the tickets
/// of the requests it posted, and when it posted them.
struct Asked {
    at: usize,
    key: String,
    d: Box<Driving>,
    tickets: Vec<u64>,
    since: std::time::Instant,
}

/// How often strata suspended on requests, and for how long in all (`RERAIL_STRATA_COUNTS`, the profile).
static ASK_SUSPENSIONS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static ASK_SUSPENDED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Takes the answers that have come (waiting up to `wait` for one when none has), and resumes each stratum whose
/// requests are all answered: it runs on, finishes, waits on its wave, or asks again.
#[allow(clippy::too_many_arguments)]
fn take_answers<'a>(
    run: &mut Run<'a, '_>,
    asked: &mut Vec<Asked>,
    arrived: &mut HashMap<u64, Vec<(String, Vec<Vec<i64>>)>>,
    waiting: &mut Vec<(usize, String, Box<Driving>)>,
    outcome: &mut Outcome,
    carry: &mut Vec<String>,
    sccs: &[BTreeSet<String>],
    last: &HashMap<String, usize>,
    wait: std::time::Duration,
) -> Result<()> {
    let Some(asking) = run.asking.as_deref_mut() else {
        return Err(Error::Plan("requests posted, and no one to answer them".into()));
    };
    let t0 = std::time::Instant::now();
    let came = asking.arrived(wait)?;
    if !wait.is_zero() {
        ANSWER_WAIT.fetch_add(t0.elapsed().as_nanos() as u64, std::sync::atomic::Ordering::Relaxed);
    }
    arrived.extend(came);
    let mut k = 0;
    while k < asked.len() {
        if !asked[k].tickets.iter().all(|t| arrived.contains_key(t)) {
            k += 1;
            continue;
        }
        let a = asked.remove(k);
        ASK_SUSPENSIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        ASK_SUSPENDED.fetch_add(a.since.elapsed().as_nanos() as u64, std::sync::atomic::Ordering::Relaxed);
        let given: Vec<(String, Vec<Vec<i64>>)> = a.tickets.iter().flat_map(|t| arrived.remove(t).unwrap_or_default()).collect();
        let _stratum = super::profile::span(|| format!("stratum {} (after its answers)", if a.key.len() > 80 { format!("{}…", &a.key[..80]) } else { a.key.clone() }));
        add_answers(&run.db, &mut run.answered, None, &a.d.name, &a.d.scc, given)?;
        match run.native_resume(a.d)? {
            Native::Ran => {
                outcome.native.insert(a.key);
                carry.extend(sccs[a.at].iter().filter(|r| !last.contains_key(*r)).cloned());
            }
            Native::Waiting(d) => waiting.push((a.at, a.key, d)),
            Native::Asking(d, tickets) => asked.push(Asked { at: a.at, key: a.key, d, tickets, since: std::time::Instant::now() }),
            Native::Declined => return Err(Error::Plan(format!("{}: a stratum that asked declined", a.key))),
        }
    }
    Ok(())
}

/// Per relation, the first stratum whose rules read it; a view is read where its first reader runs. `by` each
/// stratum's rules (`rules_by_stratum`), read by their handles in `vs`; `f` the program's fields.
pub fn first_reads(f: Fields<'_>, vs: &body::Versions, sccs: &[BTreeSet<String>], by: &[Vec<RuleKey>], views: &HashSet<String>) -> HashMap<String, usize> {
    let mut first: HashMap<String, usize> = HashMap::new();
    for (j, scc) in sccs.iter().enumerate().rev() {
        let at = if scc.iter().all(|r| views.contains(r)) { scc.iter().filter_map(|r| first.get(r)).copied().min().unwrap_or(j) } else { j };
        for rel in stratum_reads(f, vs, scc, &by[j]) {
            let e = first.entry(rel).or_insert(at);
            *e = (*e).min(at);
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

/// The literals of `p`'s rules, each head one: what a whole-program pass over them reads (the profile's work).
fn literals(p: &body::Placed) -> u64 {
    (0..p.len()).map(|k| 1 + p.rule(k).body().count() as u64).sum()
}

/// Each stratum's rules, as keys of the program's version `p` (`RuleKey::program`: the program's rule `k`), in the
/// program's order.
pub fn keys_by_stratum(p: &body::Placed, sccs: &[BTreeSet<String>]) -> Vec<Vec<RuleKey>> {
    let of: HashMap<&str, usize> = sccs.iter().enumerate().flat_map(|(j, scc)| scc.iter().map(move |r| (r.as_str(), j))).collect();
    let mut by: Vec<Vec<RuleKey>> = vec![Vec::new(); sccs.len()];
    for k in 0..p.len() {
        if let Some(&j) = of.get(p.v.rel_name(p.rule(k).head().rel())) {
            by[j].push(RuleKey::program(k));
        }
    }
    by
}

/// Per relation, the last stratum (an index into `sccs`) whose rules read it (`first_reads`' reading).
pub fn last_reads(f: Fields<'_>, vs: &body::Versions, sccs: &[BTreeSet<String>], by: &[Vec<RuleKey>], views: &HashSet<String>) -> HashMap<String, usize> {
    let mut last: HashMap<String, usize> = HashMap::new();
    for (j, scc) in sccs.iter().enumerate().rev() {
        let at = if scc.iter().all(|r| views.contains(r)) { scc.iter().filter_map(|r| last.get(r)).copied().max().unwrap_or(j).max(j) } else { j };
        for rel in stratum_reads(f, vs, scc, &by[j]) {
            let e = last.entry(rel).or_insert(at);
            *e = (*e).max(at);
        }
    }
    last
}

/// Kept relations `rels` (outputs of `lowered`, read back from the store, t32 4.1) as `decoded_outputs` gives an
/// output: held in `db`'s run as the engine's relations and decoded from there. Not `finding`, which SQL decodes.
#[allow(clippy::type_complexity)]
pub fn decode_kept(
    db: &Connection,
    lowered: &lower::Lowered,
    rels: Vec<(String, std::sync::Arc<super::native::component::InRel>)>,
) -> Result<(BTreeMap<String, std::sync::Arc<output::Rows>>, BTreeMap<String, Vec<usize>>)> {
    let names: BTreeSet<String> = rels.iter().map(|(r, _)| r.clone()).filter(|r| r != "finding").collect();
    for (rel, held) in rels.into_iter().filter(|(r, _)| names.contains(r)) {
        super::native::component::hold_as(db, &rel, held);
    }
    // The program's inputs and declarations, read from its version.
    let placed = lowered.placed()?;
    let f = placed.fields()?;
    let outputs: Vec<&str> = names.iter().map(String::as_str).collect();
    decoded_outputs(&outputs, &|r| f.is_input(r), &|r| f.columns(r).map(|c| c.into_iter().map(|(_, t)| t.to_owned()).collect()), db, Some(lowered.lowering()), "", true, &BTreeSet::new(), &names)
}

/// The program's outputs as rows (decision 0239: symbols, kinds and fields as their text, every other value as it
/// is; no row printed), and per output the columns that hold nodes.
#[allow(clippy::type_complexity)]
/// `outputs` the outputs to decode; `is_input` and `arity` the program's (its inputs, its declarations' widths).
#[allow(clippy::too_many_arguments)]
fn decoded_outputs(
    outputs_of: &[&str],
    is_input: &dyn Fn(&str) -> bool,
    types: &dyn Fn(&str) -> Option<Vec<String>>,
    db: &Connection,
    lowered: Option<lower::Lowering<'_>>,
    facts: &str,
    atoms: bool,
    kept: &BTreeSet<String>,
    native: &BTreeSet<String>,
) -> Result<(BTreeMap<String, std::sync::Arc<output::Rows>>, BTreeMap<String, Vec<usize>>)> {
    let (mut outputs, mut all_nodes) = (BTreeMap::new(), BTreeMap::new());
    let text = |e: &str| if atoms { format!("sym_text({e})") } else { e.to_owned() };
    for &rel in outputs_of.iter().filter(|r| !kept.contains(**r)) {
        // The findings form `finding(key, n)` is written as `finding_at` (key, path, start); a `finding` of another
        // shape (a data-layer program's `finding(clause, code, n)`) is an output like any other.
        let declared = types(rel);
        let findings_form = rel == "finding" && declared.as_ref().is_none_or(|t| t.len() == 2);
        // Each column's decoding: symbols as text (names in SQL, kinds and fields from the dialect's grammar).
        let domains: Vec<lower::Domain> = match lowered {
            Some(l) if !findings_form => l.domains.get(rel).cloned().ok_or_else(|| Error::Plan(format!("the output {rel} has no columns' domains")))?,
            _ => Vec::new(),
        };
        let sql = if findings_form {
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
        let name = if findings_form { "finding_at".to_owned() } else { rel.to_owned() };
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
        // A value as the row holds it: an integer as it is; a kind or a field by its name; NULL as none
        // (`output::NULL`, shown `None`); a text as it is, in a text column. No integer column holds a text, a real
        // or a blob: the engine has no such value there.
        // Each column typed by its domain (a node; a text: a name, a kind, a field; else an integer), or where the run
        // was not lowered, by its declared type. A table's rows are a set; a row seeded from SQL may also be one the
        // engine derived: each row is once where it is read (the image).
        let types: Vec<u8> = if findings_form {
            vec![output::col::TEXT, output::col::TEXT, output::col::INT]
        } else if lowered.is_some() {
            domains
                .iter()
                .map(|d| match d {
                    lower::Domain::Node => output::col::NODE,
                    lower::Domain::Name | lower::Domain::Kind | lower::Domain::Field => output::col::TEXT,
                    _ => output::col::INT,
                })
                .collect()
        } else {
            declared
                .as_ref()
                .map(|cols| cols.iter().map(|t| match t.as_str() { "node" => output::col::NODE, "number" | "int" | "unsigned" => output::col::INT, _ => output::col::TEXT }).collect())
                .unwrap_or_default()
        };
        let text_domain = |i: usize| types.get(i).is_none_or(|&t| t == output::col::TEXT);
        let cell = |i: usize, v: SqlValue| -> Result<Shown> {
            Ok(match v {
                SqlValue::Integer(v) => decode(i, v).map_or(Shown::Int(v), Shown::Text),
                SqlValue::Null if text_domain(i) => Shown::Text("None".to_owned()),
                SqlValue::Null => Shown::Int(output::NULL),
                SqlValue::Text(s) if text_domain(i) => Shown::Text(s),
                SqlValue::Real(f) if text_domain(i) => Shown::Text(format!("{f:?}")),
                other => return Err(Error::Plan(format!("{rel}: {other:?} in integer column {i}"))),
            })
        };
        let text_cols: Vec<bool> = domains.iter().map(|d| *d == lower::Domain::Name).collect();
        let held = if native.contains(rel) && findings_form {
            super::native::component::finding_rows(db)?.map(|rows| rows.into_iter().map(|(c, p, at)| vec![SqlValue::Text(c), SqlValue::Text(p), SqlValue::Integer(at)]).collect())
        } else if native.contains(rel) {
            super::native::component::output_rows(db, rel, &text_cols)?
        } else if atoms && rel != "finding" && is_input(rel) && let Some(src) = crate::code_rule_dsl::facts::vtab::bound_source(db, rel) {
            // An input handed on as an output is read where it is bound.
            Some(super::native::component::source_rows(db, src.as_ref(), &text_cols)?)
        } else {
            None
        };
        let mut rows = output::Rows::new(types.clone());
        let mut row: Vec<Shown> = Vec::new();
        let push = |rows: &mut output::Rows, row: &[Shown]| rows.push(&row.iter().map(Shown::cell).collect::<Vec<_>>());
        match held {
            Some(held) => {
                for r in held {
                    row.clear();
                    for (i, v) in r.into_iter().enumerate() {
                        row.push(cell(i, v)?);
                    }
                    push(&mut rows, &row);
                }
            }
            None => {
                let mut st = db.prepare(&sql)?;
                let width = st.column_count();
                let mut q = st.query([])?;
                while let Some(r) = q.next()? {
                    row.clear();
                    for i in 0..width {
                        row.push(cell(i, r.get::<_, SqlValue>(i)?)?);
                    }
                    push(&mut rows, &row);
                }
            }
        }
        let nodes = domains.iter().enumerate().filter(|(_, d)| **d == lower::Domain::Node).map(|(i, _)| i).collect();
        all_nodes.insert(name.clone(), nodes);
        outputs.insert(name, std::sync::Arc::new(rows));
    }
    Ok((outputs, all_nodes))
}

/// A value of a decoded row before it is held (`decoded_outputs`).
enum Shown {
    Int(i64),
    Text(String),
}

impl Shown {
    fn cell(&self) -> output::Cell<'_> {
        match self {
            Shown::Int(v) => output::Cell::Int(*v),
            Shown::Text(s) => output::Cell::Text(s),
        }
    }
}

pub mod emit;
pub mod fields;
pub mod acyclic;
pub mod estimates;
pub mod sql_kernel;
pub mod identity_parts;

// cratify: `tests` is rerail-engine's (it mounts it)

/// Whether the SQL text `sql` names the identifier `name` (as a whole word, outside string literals).
fn sql_names(sql: &str, name: &str) -> bool {
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut in_str = false;
    let bytes: Vec<(usize, char)> = sql.char_indices().collect();
    let mut i = 0;
    while i < bytes.len() {
        let (at, c) = bytes[i];
        if c == '\'' {
            in_str = !in_str;
        } else if !in_str && sql[at..].starts_with(name) {
            let before = sql[..at].chars().next_back().is_none_or(|b| !word(b));
            let after = sql[at + name.len()..].chars().next().is_none_or(|a| !word(a));
            if before && after {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// A table's value as an ordered key (`reach_rows`): integers before text, as SQLite orders them; a real or a blob
/// by its text.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum SqlKey {
    Null,
    Int(i64),
    Text(String),
}

impl SqlKey {
    fn of(v: SqlValue) -> SqlKey {
        match v {
            SqlValue::Null => SqlKey::Null,
            SqlValue::Integer(i) => SqlKey::Int(i),
            SqlValue::Text(t) => SqlKey::Text(t),
            SqlValue::Real(f) => SqlKey::Text(f.to_string()),
            SqlValue::Blob(b) => SqlKey::Text(String::from_utf8_lossy(&b).into_owned()),
        }
    }
    fn value(&self) -> SqlValue {
        match self {
            SqlKey::Null => SqlValue::Null,
            SqlKey::Int(i) => SqlValue::Integer(*i),
            SqlKey::Text(t) => SqlValue::Text(t.clone()),
        }
    }
}

/// The DP's guard against a cross product priced from a default size (decision 0137): step `i` shares no variable
/// with what `prev` joined (`bound`), a connected step remains, and `i` or such a step has no size of its own
/// (`no_size`, by join). `jmask`: each join's variables as bits.
fn unfounded_cross(i: usize, prev: u64, bound: u128, jmask: &[u128], no_size: u64) -> bool {
    if prev == 0 || jmask[i] & bound != 0 {
        return false;
    }
    let mut connected = 0u64;
    for (j, &m) in jmask.iter().enumerate() {
        if j != i && prev >> j & 1 == 0 && m & bound != 0 {
            connected |= 1 << j;
        }
    }
    connected != 0 && (no_size >> i & 1 == 1 || connected & no_size != 0)
}

/// Whether each literal of a body is ready (`ready`), kept by which of its own variables are bound: a table of its
/// variables' patterns, made when the literal is first asked about, or a map for one of more than `Readiness::WIDE`
/// variables.
struct Readiness {
    own: Vec<u128>,
    tables: Vec<Option<Vec<u8>>>,
    wide: rustc_hash::FxHashMap<(usize, u128), bool>,
}

impl Readiness {
    const WIDE: u32 = 12;

    /// For literals whose variables are `own` (as bits).
    fn new(own: &[u128]) -> Readiness {
        Readiness { own: own.to_vec(), tables: vec![None; own.len()], wide: Default::default() }
    }

    /// Whether literal `f` is ready with `bound` bound: `test` of its own variables bound, run once per pattern.
    fn ready(&mut self, f: usize, bound: u128, test: impl FnOnce(u128) -> bool) -> bool {
        let own = self.own[f];
        let pattern = bound & own;
        if own.count_ones() > Self::WIDE {
            return *self.wide.entry((f, pattern)).or_insert_with(|| test(pattern));
        }
        // The pattern's bits packed in the order of the literal's variables.
        let (mut at, mut rest, mut k) = (0usize, own, 0);
        while rest != 0 {
            let low = rest & rest.wrapping_neg();
            if pattern & low != 0 {
                at |= 1 << k;
            }
            rest &= rest - 1;
            k += 1;
        }
        let table = self.tables[f].get_or_insert_with(|| vec![0; 1 << own.count_ones()]);
        if table[at] == 0 {
            table[at] = 1 + u8::from(test(pattern));
        }
        table[at] == 2
    }
}

#[cfg(test)]
mod search_tests {
    use super::*;
    /// `cyclic_relations` by one pass over the edges is the walk from each relation: every relation that reaches
    /// itself through the given rules' atoms (a self-read, or a strongly connected part of more than one), over
    /// generated programs.
    #[test]
    fn cyclic_relations_are_those_that_reach_themselves() {
        let walked = |scc: &BTreeSet<String>, v: &Version| -> BTreeSet<String> {
            let mut next: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
            for r in v.rules() {
                for l in r.body() {
                    if let super::super::program_image::Lit::Atom(a) = l.view()
                        && scc.contains(v.rel_name(a.rel()))
                    {
                        next.entry(v.rel_name(a.rel())).or_default().insert(v.rel_name(r.head().rel()));
                    }
                }
            }
            scc.iter().filter(|start| {
                let (mut seen, mut todo): (BTreeSet<&str>, Vec<&str>) = (BTreeSet::new(), next.get(start.as_str()).into_iter().flatten().copied().collect());
                while let Some(x) = todo.pop() {
                    if x == start.as_str() { return true; }
                    if seen.insert(x) { todo.extend(next.get(x).into_iter().flatten().copied()); }
                }
                false
            }).cloned().collect()
        };
        let mut seed = 7u64;
        let mut rand = |n: u64| { seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17; seed % n };
        for _ in 0..60 {
            let n = 2 + rand(12) as usize;
            let mut text: String = (0..n).map(|i| format!(".decl r{i}(x:number)\n")).collect();
            text.push_str(".decl base(x:number)\nbase(1).\n");
            for i in 0..n {
                text.push_str(&format!("r{i}(x) :- base(x).\n"));
                for _ in 0..rand(3) {
                    let j = rand(n as u64);
                    let neg = if rand(5) == 0 { format!(", !r{}(x)", rand(n as u64)) } else { String::new() };
                    text.push_str(&format!("r{i}(x) :- r{j}(x), base(x){neg}.\n"));
                }
            }
            text.push_str(&format!(".output {}\n", (0..n).map(|i| format!("r{i}")).collect::<Vec<_>>().join(",")));
            let p = super::super::rr::parse_version(&text).unwrap();
            // The relations taken as one part (a superset of each strongly connected part), with base outside it.
            let scc: BTreeSet<String> = (0..n).map(|i| format!("r{i}")).collect();
            let vs = body::Versions::of_placed(body::Placed::whole(p.clone()));
            let keys: Vec<RuleKey> = body::Versions::keys(0, p.rules().count()).collect();
            assert_eq!(cyclic_relations(&vs, &scc, &keys), walked(&scc, &p), "{text}");
        }
    }

    /// The guard as decision 0137 wrote it, over the same masks: a step sharing no variable with what is joined is
    /// refused while a connected step remains, where it or that connected step has no size of its own.
    fn cross_by_definition(i: usize, prev: u64, bound: u128, jmask: &[u128], no_size: &[bool]) -> bool {
        prev != 0 && jmask[i] & bound == 0 && (0..jmask.len()).any(|j| j != i && prev & (1 << j) == 0 && jmask[j] & bound != 0 && (no_size[i] || no_size[j]))
    }

    /// `unfounded_cross` (the DP's guard against a cross product priced from a default size) gives the definition's
    /// answer for every step, subset joined and set of bound variables, from masks made once per search: it read
    /// three statistics maps by relation name for every pair of joins at every candidate, 12% of `keeps::choose`.
    #[test]
    fn the_cross_product_guard_is_its_definition_over_masks() {
        // Four joins over variables 0..5: a(0,1) b(1,2) c(3) d(2,3,4).
        let jmask: Vec<u128> = vec![0b11, 0b110, 0b1000, 0b11100];
        for sized in 0u64..16 {
            let no_size: Vec<bool> = (0..4).map(|j| sized >> j & 1 == 0).collect();
            let mask: u64 = (0..4).filter(|&j| no_size[j]).map(|j| 1u64 << j).sum();
            for prev in 0u64..16 {
                let bound = (0..4).filter(|&j| prev >> j & 1 == 1).fold(0u128, |b, j| b | jmask[j]);
                for i in (0..4).filter(|&i| prev >> i & 1 == 0) {
                    assert_eq!(unfounded_cross(i, prev, bound, &jmask, mask), cross_by_definition(i, prev, bound, &jmask, &no_size), "step {i} after {prev:04b}, unsized joins {mask:04b}");
                }
            }
        }
    }

    /// Whether a filter is ready, kept per filter by which of its own variables are bound (`Readiness`): the answer
    /// is the test's for every set bound, and the test runs once per pattern of its variables. Kept in a hash map by
    /// (filter, variables), its lookups were two thirds of the time a search spent placing filters.
    #[test]
    fn a_filters_readiness_is_kept_by_its_own_variables() {
        // Three filters over variables 0..8: {1, 3}, {0, 2, 5, 7}, none.
        let own: Vec<u128> = vec![0b1010, 0b1010_0101, 0];
        let test = |f: usize, b: u128| match f {
            0 => b & 0b10 != 0,
            1 => b & 0b1000_0001 == 0b1000_0001,
            _ => true,
        };
        let mut runs = 0usize;
        let mut kept = Readiness::new(&own);
        for b in 0u128..256 {
            for f in 0..own.len() {
                let got = kept.ready(f, b, |x| {
                    runs += 1;
                    assert_eq!(x & !own[f], 0, "only the filter's own variables are passed");
                    test(f, x)
                });
                assert_eq!(got, test(f, b & own[f]), "filter {f} with {b:08b} bound");
            }
        }
        assert_eq!(runs, 4 + 16 + 1, "once per pattern of each filter's variables");
    }

    /// A model of `prog` whose inputs have the rows and distinct values by column given.
    fn model_with(prog: &body::Placed, inputs: &[(&str, f64, &[f64])]) -> Model {
        let given = Inputs { facts: "main.".to_owned(), stats: "main.".to_owned(), atoms: Some([0; 3]), native: BTreeSet::new(), batch: HashMap::new(), given: None, kept: BTreeSet::new() };
        let v = &prog.v;
        let derived: HashSet<String> = v.rules().map(|r| v.rel_name(r.head().rel()).to_owned()).collect();
        let mut model = Model { shapes: Default::default(), docs_by_shift: false, analysis: Default::default(), stats: Stats::empty_of(&prog.fields().unwrap()), ndv: Default::default(), docof: Default::default(), view_src: Default::default(), inputs: given.into(), derived: derived.into() };
        model.stats.nodes = 1_000_000.0;
        for &(rel, rows, ndv) in inputs {
            model.stats.card.insert(rel.to_owned(), rows);
            for (c, &n) in ndv.iter().enumerate() {
                model.ndv.insert((rel.to_owned(), c), n);
                model.stats.fan.insert((rel.to_owned(), c), rows / n);
            }
        }
        model
    }

    /// The rows after each step of a plan, as calibration reads them (`keeps::calibrated_rows_at`), are the EXPLAIN
    /// record's, and the plan is the same: made without the record's strings, bases and passed-over joins, each of
    /// which priced every unplaced join again (15% of `keeps::choose`).
    #[test]
    fn a_plans_step_rows_are_its_explain_records() {
        let prog = crate::code_rule_dsl::logic::rr::parse_version(
            ".decl a(x: number)\n.decl b(x: number, y: number)\n.decl c(y: number, z: number)\n.decl d(z: number, w: number)\n\
             .input a\n.input b\n.input c\n.input d\n.decl o(x: number, w: number)\n\
             o(x, w) :- a(x), b(x, y), c(y, z), d(z, w), !a(w), z != 3.\n\
             o(x, w) :- b(x, y), c(y, 7), d(7, w), w = x.\n\
             o(x, w) :- d(z, w), c(y, z), b(x, y), a(x).\n.output o\n",
        )
        .unwrap();
        let placed = body::Placed::whole(prog.clone());
        let model = model_with(&placed, &[("a", 50.0, &[50.0]), ("b", 100_000.0, &[1_000.0, 5_000.0]), ("c", 20_000.0, &[5_000.0, 300.0]), ("d", 9_000.0, &[300.0, 700.0])]);
        let was = explaining(true);
        for (k, rule) in prog.rules().enumerate() {
            let b = body::nth(&prog, k);
            let r = prog.show_rule(rule);
            for first in [None, Some(1)] {
                let (plan, x) = model.plan_rule_explained(placed.fields().unwrap(), &b, first).unwrap();
                let (lean, rows) = model.plan_rule_step_rows(placed.fields().unwrap(), &b, first).unwrap();
                let recorded: Vec<f64> = x.expect("explained").steps.iter().map(|s| s.rows).collect();
                assert_eq!((b.shown(&lean.0), lean.1, lean.2), (b.shown(&plan.0), plan.1, plan.2), "{r}");
                assert_eq!(rows, recorded, "{r} from {first:?}");
            }
        }
        explaining(was);
    }
}

/// A checked line's rows left, split by its key's heavy values (`Run::heavy_parts`): the linking relation and its key
/// column; the rows of heavy keys and the rest.
struct HeavyParts {
    link: String,
    cv: usize,
    hot: Vec<u32>,
    cold: Vec<u32>,
}

/// Decision 0243: what a background search's walks read, kept by its request while the engine holds it: the body's
/// relations (shared, not copied), the rule's text constants' symbols, and for a probe's line a sample of its delta's
/// rows with their count.
struct WalkSnap {
    rels: HashMap<String, std::sync::Arc<super::native::component::InRel>>,
    syms: HashMap<String, i64>,
    delta: Option<(Vec<Vec<i64>>, f64)>,
    /// Decision 0245: each body atom's columns whose heavy values were made, by relation and column: the `HEAVY`
    /// most common, as the inline trials split by them (`Run::heavy_parts`).
    heavy: HashMap<(String, usize), Vec<i64>>,
}

/// Orders a background search walks, at most: its best-priced, and the one that ran.
const WALK_ORDERS: usize = 16;

thread_local! {
    /// Mixed into the walks' seeds: 0 but while `RERAIL_WALK_REPEAT` measures their spread.
    static WALK_SALT: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// The salt for a walk's `part` (`starts` for the lead's draw, `walker` for its steps); `RERAIL_WALK_SALT_ONLY`
/// keeps the other part's seeds as they are, to measure each part's spread.
fn walk_salt(part: &str) -> u64 {
    let only = std::env::var("RERAIL_WALK_SALT_ONLY").ok();
    if only.as_deref().is_some_and(|o| o != part) { return 0 }
    WALK_SALT.with(|s| s.get()).wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

/// How many lead rows past the checks a background walk draws, at most (`RERAIL_WALK_STARTS`), within the draw's
/// cap of `64 * walk_rows` read. Decision 0248: 4 times `walk_rows`. The walker thins them by each step's size, so
/// more cost little: at 512 the orders led by the calls varied by 23% and 33% (sd, 8 seeds), at 128 by 31% and 39%,
/// for the same search CPU.
fn walk_starts() -> usize {
    std::env::var("RERAIL_WALK_STARTS").ok().and_then(|v| v.parse().ok()).unwrap_or_else(|| 4 * joins::walk_rows())
}

/// `RERAIL_WALK_REPEAT`: a measurement aid. A background search walks each order this many times more, each under
/// other seeds, and logs their mean and spread. The ranking keeps the first walk.
fn walk_repeat() -> usize {
    std::env::var("RERAIL_WALK_REPEAT").ok().and_then(|v| v.parse().ok()).unwrap_or(0)
}

impl WalkSnap {
    /// `None` where a positive atom's relation is not the engine's (the walk could not probe it).
    /// Read by the scope alone (`Db`): a stratum off the run's thread asks too, and reads no SQL (decision 0113).
    fn take(db: crate::code_rule_dsl::facts::vtab::Db<'_>, name: &str, line: usize, b: &Body<'_>, first: Option<usize>, heavy_of: &dyn Fn(&str, usize) -> Option<Vec<i64>>) -> Option<WalkSnap> {
        use super::native::component as engine;
        let mut rels = HashMap::new();
        let mut syms = HashMap::new();
        let mut heavy = HashMap::new();
        for i in 0..b.len() {
            let Lit::Atom(a) = b.view(i) else { continue };
            let rel = b.rel(a);
            if !a.neg() {
                for (c, t) in b.args(i).iter().enumerate() {
                    if t.var().is_some() && let Some(h) = heavy_of(rel, c) { heavy.insert((rel.to_string(), c), h); }
                }
            }
            match engine::held(db, rel) {
                Some(held) => { rels.insert(rel.to_string(), held); }
                None if !a.neg() => return None,
                None => {}
            }
            for t in b.args(i) {
                if let TermRef::Str(x) = *t && let Ok(Some(id)) = engine::symbol_id(db, b.v.text(x)) { syms.insert(b.v.text(x).to_string(), id); }
            }
        }
        // A probe's line leads with its delta: a seeded uniform sample of the rows it read, as the lead's values.
        let delta = first.and_then(|f| {
            let Lit::Atom(a) = b.view(f) else { return None };
            let ids = engine::trial_ids(db, name, line).ok()??;
            // The relation's own ids (`trial_ids`), read through it, not through its SQL source's live numbering.
            let src = engine::held(db, b.rel(a))?;
            let mut v: Vec<(u64, u32)> = ids.iter().map(|&id| ((id as u64 + 3).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 7, id)).collect();
            v.sort_unstable();
            // As wide as a lead's draw (`starts`): its checks may pass few.
            let width = b.args(f).len();
            let rows = v.into_iter().take(16 * joins::walk_rows()).map(|(_, id)| (0..width).map(|c| src.at(id as usize).at(c)).collect()).collect();
            Some((rows, ids.len() as f64))
        });
        Some(WalkSnap { rels, syms, delta, heavy })
    }

    /// The checks right after order `o`'s lead, as body literals: atoms over the lead's variables and constants alone.
    fn checks(b: &Body<'_>, o: &[Step]) -> Vec<usize> {
        let Some(lead) = o.first().map(|s| s.at) else { return Vec::new() };
        let Lit::Atom(_) = b.view(lead) else { return Vec::new() };
        let lead_vars: Vec<VarSlot> = b.args(lead).iter().filter_map(|t| t.var()).collect();
        o[1..].iter().map_while(|s| match b.view(s.at) {
            Lit::Atom(_) if b.args(s.at).iter().all(|t| t.var().is_none_or(|v| lead_vars.contains(&v))) => Some(s.at),
            _ => None,
        }).collect()
    }

    /// Decision 0245: the heavy key a walk from `lead` splits by, as `Run::heavy_parts` finds it: a variable of the
    /// lead at a column with heavy values (`None` for the link's route), else the one atom holding both a variable at
    /// such a column and a variable of the lead (its key's column, and its column of that variable with the lead's).
    fn link(&self, b: &Body<'_>, lead: usize) -> Option<(String, usize, Option<(usize, usize)>)> {
        heavy_link(b, lead, &|rel, c| self.heavy.contains_key(&(rel.to_string(), c)))
    }

    /// The rows a walk of order `o` starts from, past its lead and the checks right after it: the lead's rows that
    /// pass them, drawn until `walk_rows` pass, at most `16 * walk_rows` at a time and `64 * walk_rows` in all (a
    /// check passing 2% of 2,048 left 40 rows, each a function's work, and the walk underestimated a skewed key
    /// 5 to 70 times). With the lead's estimated rows, the checks' work per lead row (each reads what the checks
    /// before it passed) and the share passing them all. `None` where too few pass a sample to measure (16).
    fn starts(&self, b: &Body<'_>, o: &[Step], first: Option<usize>) -> Option<Starts> {
        let lead = o.first()?.at;
        let Lit::Atom(a) = b.view(lead) else { return None };
        if a.neg() { return None }
        let lead_rel = b.rel(a);
        let largs = b.args(lead);
        let width = largs.len();
        let checks = Self::checks(b, o);
        // A check's term in a lead row: a constant, or the lead's value of the variable. A wildcard binds nothing,
        // and is no column of the probe (`None`).
        let value = |t: TermRef<'_>, row: &[i64]| -> Option<Option<i64>> {
            match t {
                TermRef::Int(i) => Some(Some(i)),
                TermRef::Branded(a) => Some(Some(a.value())),
                TermRef::Str(x) => self.syms.get(b.v.text(x)).copied().map(Some),
                TermRef::Var(v) => largs.iter().position(|u| u.var() == Some(v)).map(|c| Some(row[c])),
                TermRef::Wild => Some(None),
                _ => None,
            }
        };
        // The lead's constants, as a probe's columns and key, and its columns that repeat a variable.
        let (mut cols, mut key) = (Vec::new(), Vec::new());
        for (c, t) in largs.iter().enumerate() {
            match *t {
                TermRef::Int(i) => { cols.push(c); key.push(i) }
                TermRef::Branded(a) => { cols.push(c); key.push(a.value()) }
                TermRef::Str(x) => { cols.push(c); key.push(*self.syms.get(b.v.text(x))?) }
                _ => {}
            }
        }
        let same: Vec<(usize, usize)> = largs.iter().enumerate().filter_map(|(c, t)| { let v = t.var()?; largs.iter().position(|u| u.var() == Some(v)).filter(|&d| d < c).map(|d| (d, c)) }).collect();
        // Each check's rows reached and passed, and the lead rows passing them all.
        let mut reached = vec![0u64; checks.len()];
        let mut passed = vec![0u64; checks.len()];
        let mut survivors: Vec<Vec<i64>> = Vec::new();
        // Whether a lead row passes the checks, counting each check's rows reached and passed where `counts`.
        let passes = |row: &[i64], mut counts: Option<(&mut [u64], &mut [u64])>| -> bool {
            for (i, &ci) in checks.iter().enumerate() {
                if let Some((r, _)) = counts.as_mut() { r[i] += 1 }
                let Lit::Atom(c) = b.view(ci) else { continue };
                let probe: Option<Vec<(usize, i64)>> = b.args(ci).iter().enumerate().filter_map(|(k, &t)| match value(t, row) { Some(Some(v)) => Some(Some((k, v))), Some(None) => None, None => Some(None) }).collect();
                let found = probe.is_some_and(|probe| {
                    let (pcols, pkey): (Vec<usize>, Vec<i64>) = probe.into_iter().unzip();
                    let mut found = false;
                    if let Some(rel) = self.rels.get(b.rel(c)) { rel.probe_each(&pcols, &pkey, &mut |_| found = true) }
                    found
                });
                if found == c.neg() { return false }
                if let Some((_, p)) = counts.as_mut() { p[i] += 1 }
            }
            true
        };
        let mut test = |row: Vec<i64>, survivors: &mut Vec<Vec<i64>>| {
            if passes(&row, Some((&mut reached, &mut passed))) { survivors.push(row) }
        };
        let (rows, exact) = if Some(lead) == first {
            // The delta's sample, as taken.
            let (sample, rows) = self.delta.clone()?;
            let exact = sample.len() as f64 >= rows;
            for row in sample { test(row, &mut survivors) }
            (rows, exact)
        } else {
            let rel = self.rels.get(lead_rel)?;
            let mut seed = 0x0243_5eed_u64 ^ walk_salt("starts");
            let mut draw = || { seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17; seed };
            let batch = 16 * joins::walk_rows();
            let (mut est, mut read, mut kept, mut exact) = (0.0, 0u64, 0u64, false);
            while survivors.len() < walk_starts() && (read as usize) < 64 * joins::walk_rows() {
                let mut got = Vec::new();
                let n = rel.probe_sample(&cols, &key, batch, &mut draw, &mut |row| got.push((0..width).map(row).collect::<Vec<i64>>()));
                est = n;
                read += got.len() as u64;
                // Read whole: every candidate once, and no more draws.
                exact = n <= batch as f64 && got.len() as f64 == n;
                for row in got {
                    if same.iter().all(|&(d, c)| row[d] == row[c]) { kept += 1; test(row, &mut survivors) }
                }
                if exact || read == 0 { break }
            }
            (if read > 0 { est * kept as f64 / read as f64 } else { 0.0 }, exact)
        };
        // Decision 0248: where the draw stopped short of its rows, past a check that passes few, the survivors are
        // found through the check: its rows by its constants, where they are within the draw's cap, each probing
        // the lead by the variables they share. They are then every lead row past the checks, not a sample. A
        // check passing 0.3% left 49 rows of 8,192 drawn, and `2.3.0.1.4.5`'s walks varied by 31% from the draw.
        let (mut survivors, mut exact, mut exact_share) = (survivors, exact, None);
        if !exact && Some(lead) != first && survivors.len() < walk_starts() && rows > 0.0 && let Some(lrel) = self.rels.get(lead_rel) {
            let cap = 64 * joins::walk_rows();
            for &ci in &checks {
                let Lit::Atom(c) = b.view(ci) else { continue };
                if c.neg() { continue }
                let Some(crel) = self.rels.get(b.rel(c)) else { continue };
                let cargs = b.args(ci);
                let (mut ccols, mut ckey, mut known) = (Vec::new(), Vec::new(), true);
                for (i, t) in cargs.iter().enumerate() {
                    match *t {
                        TermRef::Int(v) => { ccols.push(i); ckey.push(v) }
                        TermRef::Branded(a) => { ccols.push(i); ckey.push(a.value()) }
                        TermRef::Str(x) => match self.syms.get(b.v.text(x)) { Some(&v) => { ccols.push(i); ckey.push(v) } None => known = false },
                        _ => {}
                    }
                }
                if !known || ccols.is_empty() || crel.probe_size(&ccols, &ckey) > cap as f64 { continue }
                // (the check's column, the lead's) for each variable they share
                let links: Vec<(usize, usize)> = cargs.iter().enumerate().filter_map(|(i, t)| { let v = t.var()?; largs.iter().position(|u| u.var() == Some(v)).map(|lc| (i, lc)) }).collect();
                if links.is_empty() { continue }
                let (mut found, mut over): (Vec<Vec<i64>>, bool) = (Vec::new(), false);
                crel.probe_each(&ccols, &ckey, &mut |row| {
                    if over { return }
                    let (mut lcols, mut lkey) = (cols.clone(), key.clone());
                    for &(i, lc) in &links { if !lcols.contains(&lc) { lcols.push(lc); lkey.push(row(i)); } }
                    lrel.probe_each(&lcols, &lkey, &mut |l| {
                        let r: Vec<i64> = (0..width).map(l).collect();
                        if same.iter().all(|&(d, c)| r[d] == r[c]) && passes(&r, None) { found.push(r) }
                    });
                    over = found.len() > cap;
                });
                if over { continue }
                found.sort_unstable();
                found.dedup();
                exact_share = Some(found.len() as f64 / rows);
                survivors = found;
                exact = true;
                break;
            }
        }
        if !exact && survivors.len() < 16 { return None }
        // Per lead row: each check reads what passed the ones before it.
        let (mut work, mut share) = (0.0, 1.0);
        for i in 0..checks.len() {
            work += share;
            share *= if reached[i] > 0 { passed[i] as f64 / reached[i] as f64 } else { 0.0 };
        }
        let share = exact_share.unwrap_or(share);
        let mut st = Starts { rows: survivors, lead_rows: rows, checks: checks.len(), checks_work: work, share, cold: 1.0, hot: None };
        // Decision 0245: split by a heavy key, as the inline trials do (0221, 0233). A delta's sample is not split:
        // its rows are not a relation to draw by key.
        if !exact && Some(lead) != first && let Some((link, cv, via)) = self.link(b, lead) && let Some(lrel) = self.rels.get(link.as_str()) {
            let heavy: HashSet<i64> = self.heavy[&(link.clone(), cv)].iter().copied().collect();
            let hot_row = |row: &[i64]| match via {
                None => heavy.contains(&row[cv]),
                Some((cu, lc)) => { let mut hit = false; lrel.probe_each(&[cu], &[row[lc]], &mut |l| hit |= heavy.contains(&l(cv))); hit }
            };
            let (hot_u, cold_u): (Vec<Vec<i64>>, Vec<Vec<i64>>) = std::mem::take(&mut st.rows).into_iter().partition(|row| hot_row(row));
            let cold_share = cold_u.len() as f64 / (hot_u.len() + cold_u.len()).max(1) as f64;
            // The hot lead rows drawn by key: each heavy value's link rows, sampled in proportion to its rows, then
            // the lead's rows they reach, kept where they pass the checks. Each heavy value's survivors are scaled
            // by its rows over those drawn. A lead row two heavy values reach is counted twice.
            let batch = 16 * joins::walk_rows();
            let ordered: Vec<i64> = self.heavy[&(link.clone(), cv)].clone();
            let sizes: Vec<f64> = ordered.iter().map(|&h| { let mut n = 0.0; lrel.probe_each(&[cv], &[h], &mut |_| n += 1.0); n }).collect();
            let all: f64 = sizes.iter().sum();
            let mut seed = 0x0245_5eed_u64 ^ walk_salt("starts");
            let mut draw = || { seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17; seed };
            // Per heavy value: its rows (with the lead's constants, where the lead holds the key), and the rows
            // drawn and passing over all rounds. Rounds go on until `walk_rows` pass, at most `64 * walk_rows`
            // drawn, as the uniform draw: a check passing 0.3% left 6 rows of one round's 2,080.
            let mut by: Vec<(f64, usize, usize)> = vec![(0.0, 0, 0); ordered.len()];
            let (mut hot_rows, mut drawn) = (Vec::new(), 0usize);
            while hot_rows.len() < walk_starts() && drawn < 64 * joins::walk_rows() && all > 0.0 {
                let before = drawn;
                for (i, (&h, &size)) in ordered.iter().zip(&sizes).enumerate() {
                    if size == 0.0 { continue }
                    let limit = ((batch as f64 * size / all).ceil() as usize).max(1);
                    let mut links: Vec<i64> = Vec::new();
                    let mut leads: Vec<Vec<i64>> = Vec::new();
                    let n = match via {
                        None => {
                            // The lead holds the key: its rows with the key and its constants.
                            let (mut c2, mut k2) = (cols.clone(), key.clone());
                            c2.push(cv); k2.push(h);
                            lrel.probe_sample(&c2, &k2, limit, &mut draw, &mut |row| leads.push((0..width).map(row).collect()))
                        }
                        Some((cu, _)) => { lrel.probe_sample(&[cv], &[h], limit, &mut draw, &mut |row| links.push(row(cu))); size }
                    };
                    let got = match via { None => leads.len(), Some(_) => links.len() };
                    if got == 0 { continue }
                    drawn += got;
                    if let Some((_, lc)) = via {
                        let lr = self.rels.get(lead_rel)?;
                        let (mut c2, mut k2) = (cols.clone(), key.clone());
                        c2.push(lc);
                        k2.push(0);
                        for u in links {
                            *k2.last_mut().expect("pushed") = u;
                            lr.probe_each(&c2, &k2, &mut |row| leads.push((0..width).map(row).collect()));
                        }
                    }
                    let mut kept = 0usize;
                    for row in leads {
                        if same.iter().all(|&(d, c)| row[d] == row[c]) && passes(&row, None) { kept += 1; hot_rows.push(row) }
                    }
                    by[i] = (n, by[i].1 + got, by[i].2 + kept);
                }
                if drawn == before { break }
            }
            // Each heavy value's survivors scaled by its rows over those drawn. A lead row two heavy values reach is
            // counted twice.
            let hot_n: f64 = by.iter().filter(|b| b.1 > 0).map(|&(n, got, kept)| n * kept as f64 / got as f64).sum();
            let drawn_pass = hot_rows.len();
            // The uniform draw's hot rows are a uniform sample of the same rows: walked with them.
            hot_rows.extend(hot_u.iter().cloned());
            if WALK_SALT.with(|s| s.get()) == 0 { joins::log(|| format!("background walk split by {link}.{cv}: {} of {} lead rows past the checks hot by value; {} drawn by key, {} pass, estimated {hot_n:.0}", hot_u.len(), hot_u.len() + cold_u.len(), drawn, drawn_pass)); }
            st.rows = cold_u;
            st.cold = cold_share;
            st.hot = Some((hot_rows, hot_n));
        }
        Some(st)
    }

    /// The search's orders, the `WALK_ORDERS` best-priced and the one that ran, each walked from its own lead's rows
    /// that pass the checks after it (`starts`): their walked work over the line. An order not measured is left out
    /// (a price and a walked work are not one measure); where the order that ran is not walked, the priced orders are
    /// kept instead.
    ///
    /// Decision 0245: the order that ran is walked first, then the rest by price, each only while `over` says the
    /// time is not up.
    fn walk(&self, b: &Body<'_>, first: Option<usize>, ran: &[Step], mut priced: Vec<(Vec<Step>, f64)>, over: &dyn Fn() -> bool) -> joins::Searched {
        priced.sort_by(|a, b| a.1.total_cmp(&b.1));
        priced.truncate(WALK_ORDERS);
        let ran_price = priced.iter().position(|(o, _)| same_order(o, ran)).map_or(f64::NAN, |i| priced.remove(i).1);
        priced.insert(0, (ran.to_vec(), ran_price));
        let mut walkers: HashMap<usize, Walker<'_, '_>> = HashMap::new();
        // The starts by the lead and its checks: orders that share them share their rows.
        let mut starts: HashMap<Vec<usize>, Option<Starts>> = HashMap::new();
        let mut walked = Vec::new();
        let mut ran_w = None;
        for (i, (o, price)) in priced.into_iter().enumerate() {
            // A measurement of the walks' spread (`walk_repeat`) walks every order.
            if i > 0 && over() && walk_repeat() == 0 { break }
            let Some(lead) = o.first().map(|s| s.at) else { continue };
            let prefix: Vec<usize> = o.iter().take(1 + Self::checks(b, &o).len()).map(|s| s.at).collect();
            let st = starts.entry(prefix).or_insert_with(|| self.starts(b, &o, first)).as_ref();
            let walker = walkers.entry(lead).or_insert_with(|| Walker::new(b, lead, joins::walk_width(), &|rel| self.rels.get(rel).cloned(), &|x| self.syms.get(x).copied()));
            // From the rows past the checks: the walk tests them again, each passing, and goes on. Its work past the
            // checks is the share passing them times its work less theirs. A walk whose sample ran out before its
            // last step measured nothing past it, and is not ranked.
            // Split by a heavy key (decision 0245), the rest's rows are their share of those past the checks, and the
            // hot rows are as drawn by key.
            let est = st.and_then(|st| {
                let k = st.checks as f64;
                let part = |rows: &[Vec<i64>]| -> Option<f64> {
                    if rows.is_empty() { return Some(0.0) }
                    let per = walker.work(&o, rows);
                    (!walker.emptied()).then(|| (per - k).max(0.0))
                };
                let mut est = st.lead_rows * (st.checks_work + st.share * st.cold * part(&st.rows)?);
                if let Some((rows, n)) = &st.hot { est += n * part(rows)?; }
                Some(est)
            });
            if same_order(&o, ran) { ran_w = est; }
            let reps = walk_repeat();
            if reps > 0 && joins::logging() {
                let mut got = Vec::new();
                for k in 1..=reps as u64 {
                    WALK_SALT.with(|s| s.set(k));
                    let w2 = Walker::new(b, lead, joins::walk_width(), &|rel| self.rels.get(rel).cloned(), &|x| self.syms.get(x).copied());
                    let e = self.starts(b, &o, first).and_then(|st| {
                        let k = st.checks as f64;
                        let part = |rows: &[Vec<i64>]| -> Option<f64> { if rows.is_empty() { return Some(0.0) } let per = w2.work(&o, rows); (!w2.emptied()).then(|| (per - k).max(0.0)) };
                        let mut e = st.lead_rows * (st.checks_work + st.share * st.cold * part(&st.rows)?);
                        if let Some((rows, n)) = &st.hot { e += n * part(rows)?; }
                        Some(e)
                    });
                    got.extend(e);
                }
                WALK_SALT.with(|s| s.set(0));
                let m = got.iter().sum::<f64>() / got.len().max(1) as f64;
                let sd = (got.iter().map(|x| (x - m).powi(2)).sum::<f64>() / got.len().max(1) as f64).sqrt();
                let (lo, hi) = got.iter().fold((f64::INFINITY, 0.0f64), |(a, b), &x| (a.min(x), b.max(x)));
                joins::log(|| format!("walk spread {}: {} of {reps} measured, mean {m:.0}, sd {sd:.0} ({:.0}%), {lo:.0} to {hi:.0}", o.iter().map(|s| s.at.to_string()).collect::<Vec<_>>().join("."), got.len(), 100.0 * sd / m.max(1.0)));
            }
            walked.push((joins::steps(&o), est, price));
        }
        // Without the order that ran walked, the walks have nothing to be compared with: priced, as before.
        let Some(ran_w) = ran_w.filter(|w| *w > 0.0) else {
            return joins::Searched { orders: walked.into_iter().filter(|w| w.2.is_finite()).map(|(o, _, p)| (o, p)).collect(), walked: None };
        };
        // The order that ran stays among the proposals, so the next check compares the rest with it.
        let ran_steps = joins::steps(ran);
        let mut orders: Vec<(Vec<StoredStep>, f64)> = walked.into_iter().filter_map(|(o, est, _)| Some((o, est?))).collect();
        orders.sort_by(|a, b| a.1.total_cmp(&b.1));
        if let Some(at) = orders.iter().position(|(o, _)| *o == ran_steps) && at >= joins::PROPOSALS {
            orders.swap(joins::PROPOSALS - 1, at);
        }
        joins::Searched { orders, walked: Some(ran_w) }
    }
}

/// A background walk's start (`WalkSnap::starts`): the lead's rows that pass the checks right after it, the lead's
/// estimated rows, how many checks, their work per lead row, and the share passing them all.
struct Starts {
    rows: Vec<Vec<i64>>,
    lead_rows: f64,
    checks: usize,
    checks_work: f64,
    share: f64,
    /// Decision 0245, where split by a heavy key: `rows` are the rest's, `cold` their share of the rows past the
    /// checks, and `hot` the rows drawn by the key's heavy values with their estimated rows.
    cold: f64,
    hot: Option<(Vec<Vec<i64>>, f64)>,
}

/// Decision 0245: the heavy key a walk or trial from `lead` splits by (`has`: whether a relation's column has heavy
/// values): a variable of the lead at such a column (`None` for the link's route), else the one atom holding both a
/// variable at such a column and a variable of the lead (`inf(f, m)` from `kind(m, return)`): its key's column, and
/// Decision 0258: maintenance rule `b`'s order from its source rule `src`'s kept order `order` (`map`: each literal's
/// literal of the source, `Body::source_map`). Led by the line's delta (`first`) where it has one; else as the source
/// was, each of the rule's literals where its source literal stood (a seed's start is the search's to choose, and the
/// source's was chosen for that body: a change led by itself ran a selective constant probe after it, 5x the work).
/// The source's joins follow in their order, then the rule's other joins, each next the first of them that shares a
/// bound variable while one does (no cross product while a connected join remains, as the search); each filter once
/// it can run, as a search places them. `None` where a filter never can.
fn source_order(b: &Body<'_>, src: &Body<'_>, map: &[Option<usize>], order: &[Step], first: Option<usize>) -> Option<Vec<Step>> {
    let mut of = vec![None; src.len()];
    for (i, m) in map.iter().enumerate() {
        if let Some(j) = *m {
            of[j] = Some(i);
        }
    }
    let mut joins: Vec<usize> = Vec::with_capacity(b.len());
    for s in order {
        if let Some(i) = of.get(s.at).copied().flatten()
            && b.is_join(i)
            && Some(i) != first
            && !joins.contains(&i)
        {
            joins.push(i);
        }
    }
    joins.extend((0..b.len()).filter(|&i| b.is_join(i) && Some(i) != first).collect::<Vec<_>>().into_iter().filter(|i| !joins.contains(i)).collect::<Vec<_>>());
    let mut out: Vec<Step> = Vec::with_capacity(b.len());
    let mut bound = VarSet::new();
    let mut rest: Vec<usize> = (0..b.len()).filter(|&i| !b.is_join(i)).collect();
    let close = |out: &mut Vec<Step>, bound: &mut VarSet, rest: &mut Vec<usize>| {
        while let Some(p) = rest.iter().position(|&l| b.ready(l, bound)) {
            let l = rest.remove(p);
            out.push(Step { at: l, mode: None });
            bound.extend(b.vars(l));
        }
    };
    close(&mut out, &mut bound, &mut rest);
    let take = |i: usize, out: &mut Vec<Step>, bound: &mut VarSet, rest: &mut Vec<usize>| {
        out.push(Step { at: i, mode: None });
        bound.extend(b.vars(i));
        close(out, bound, rest);
    };
    if let Some(f) = first {
        take(f, &mut out, &mut bound, &mut rest);
    }
    while !joins.is_empty() {
        let next = joins.iter().position(|&i| bound.is_empty() || !b.vars(i).is_disjoint(&bound)).unwrap_or(0);
        let i = joins.remove(next);
        take(i, &mut out, &mut bound, &mut rest);
    }
    rest.is_empty().then_some(out)
}

/// its column of that variable with the lead's.
fn heavy_link(b: &Body<'_>, lead: usize, has: &dyn Fn(&str, usize) -> bool) -> Option<(String, usize, Option<(usize, usize)>)> {
    let Lit::Atom(la) = b.view(lead) else { return None };
    let largs = b.args(lead);
    for (cv, t) in largs.iter().enumerate() {
        if t.var().is_some() && has(b.rel(la), cv) { return Some((b.rel(la).to_string(), cv, None)) }
    }
    for d in 0..b.len() {
        let Lit::Atom(a) = b.view(d) else { continue };
        if d == lead || a.neg() { continue }
        let args = b.args(d);
        for (cv, t) in args.iter().enumerate() {
            let Some(v) = t.var() else { continue };
            if !has(b.rel(a), cv) { continue }
            for (cu, t2) in args.iter().enumerate() {
                let Some(u) = t2.var() else { continue };
                if u == v { continue }
                if let Some(lc) = largs.iter().position(|t| t.var() == Some(u)) {
                    return Some((b.rel(a).to_string(), cv, Some((cu, lc))));
                }
            }
        }
    }
    None
}

/// Decision 0232: an order's work per lead row, sampled. The walk starts from lead rows (their bindings), and carries
/// a sample of at most `width` bindings through the order's steps after the lead:
/// - an atom binding new variables is probed with the bound ones (its relation's own index, `InRel::probe_sample`, drawn by position past the reservoir); the
///   bindings multiply by the sample's mean matches, and the matches are sampled again;
/// - an atom whose variables are all bound (a filter, or the atom closing a cycle or a triangle) is tested on each
///   binding; the bindings multiply by the share that pass;
/// - a negated atom passes the bindings it does not match;
/// - any other literal is taken at 1.
/// Each step adds the bindings it reads. Exact where the sample holds every binding. A binding is the rule's
/// variables by their slots.
pub(super) struct Walker<'a, 'v> {
    b: &'a Body<'v>,
    lead: usize,
    /// The rule's variables: a binding's width.
    vars: usize,
    /// Each body literal's relation, where the engine holds it.
    rels: Vec<Option<std::sync::Arc<super::native::component::InRel>>>,
    /// Each body atom's constants, by column: `Some(id)`, or `None` for a text no symbol has (no row holds it).
    consts: Vec<Vec<(usize, Option<i64>)>>,
    width: usize,
    probes: std::cell::Cell<u64>,
    /// The sampler's state: each thinning draws from it, so walks are unbiased and repeatable.
    seed: std::cell::Cell<u64>,
    /// Per body literal: its probes, the candidate rows they read, and their CPU seconds (the log's measure).
    spent: std::cell::RefCell<HashMap<usize, (u64, u64, f64)>>,
    /// Decision 0243: whether the last walk's sample ran out of bindings before its last step.
    emptied: std::cell::Cell<bool>,
}

impl<'a, 'v> Walker<'a, 'v> {
    pub(super) fn new(b: &'a Body<'v>, lead: usize, width: usize, rel: &dyn Fn(&str) -> Option<std::sync::Arc<super::native::component::InRel>>, sym: &dyn Fn(&str) -> Option<i64>) -> Walker<'a, 'v> {
        let (mut rels, mut consts) = (Vec::new(), Vec::new());
        for i in 0..b.len() {
            let Lit::Atom(a) = b.view(i) else { rels.push(None); consts.push(Vec::new()); continue };
            rels.push(rel(b.rel(a)));
            consts.push(b.args(i).iter().enumerate().filter_map(|(c, t)| match *t {
                TermRef::Int(i) => Some((c, Some(i))),
                TermRef::Branded(a) => Some((c, Some(a.value()))),
                TermRef::Str(x) => Some((c, sym(b.v.text(x)))),
                _ => None,
            }).collect());
        }
        Walker { b, lead, vars: b.rule.var_count(), rels, consts, width: width.max(1), probes: std::cell::Cell::new(0), seed: std::cell::Cell::new(0x0227_5eed ^ walk_salt("walker")), spent: Default::default(), emptied: std::cell::Cell::new(false) }
    }

    /// Whether the last walk's sample ran out of bindings before its last step: where it was a sample, its zero is
    /// only a bound (decision 0243).
    pub(super) fn emptied(&self) -> bool {
        self.emptied.get()
    }

    /// The probes made so far.
    pub(super) fn probes(&self) -> u64 {
        self.probes.get()
    }

    /// Per body literal probed: its relation, probes and milliseconds.
    pub(super) fn spent(&self) -> String {
        let mut v: Vec<_> = self.spent.borrow().iter().map(|(&d, &(n, r, t))| (d, n, r, t)).collect();
        v.sort_by_key(|x| x.0);
        v.iter().map(|(d, n, r, t)| format!("{d} {}: {n} probes reading {r} rows in {:.1} ms CPU", match self.b.view(*d) { Lit::Atom(a) => self.b.rel(a).to_string(), _ => "?".into() }, t * 1e3)).collect::<Vec<_>>().join(", ")
    }

    /// Order `o`'s work per lead row, from the lead rows `starts` (each its values by the lead's columns).
    pub(super) fn work(&self, o: &[Step], starts: &[Vec<i64>]) -> f64 {
        self.work_at(o, starts, self.width)
    }

    /// `work` carrying `width` bindings a step.
    pub(super) fn work_at(&self, o: &[Step], starts: &[Vec<i64>], width: usize) -> f64 {
        let b = self.b;
        let Lit::Atom(_) = b.view(self.lead) else { return 1.0 };
        if starts.is_empty() { return 0.0 }
        let n = self.vars;
        let mut bound = vec![false; n];
        // Decision 0247: each binding carries the share of a lead row's bindings it stands for; a step's work is
        // their sum.
        let w0 = 1.0 / starts.len() as f64;
        let mut sample: Vec<(f64, Vec<i64>)> = Vec::with_capacity(starts.len());
        for row in starts {
            let mut bind = vec![0i64; n];
            for (c, t) in b.args(self.lead).iter().enumerate() {
                if let Some(v) = t.var() { bind[v.index()] = row[c]; bound[v.index()] = true; }
            }
            sample.push((w0, bind));
        }
        let mut work = 0.0f64;
        // A text constant no symbol has: no row holds it, an exact zero (not a sample run out).
        let mut none_hold = false;
        self.emptied.set(false);
        for (si, s) in o.iter().enumerate() {
            if s.at == self.lead { continue }
            let total: f64 = sample.iter().map(|x| x.0).sum();
            if sample.is_empty() || total == 0.0 {
                self.emptied.set(!none_hold);
                break
            }
            work += total;
            let Lit::Atom(a) = b.view(s.at) else { continue };
            let args = b.args(s.at);
            let Some(rel) = &self.rels[s.at] else {
                for t in args { if let Some(v) = t.var() { bound[v.index()] = true } }
                continue;
            };
            let consts = &self.consts[s.at];
            if consts.iter().any(|c| c.1.is_none()) {
                // A text constant no symbol has: no row holds it.
                if !a.neg() { sample.clear(); none_hold = true; }
                continue;
            }
            // The key: the constants and the bound variables; the free variables, first places and repeats.
            let mut kcols: Vec<usize> = consts.iter().map(|c| c.0).collect();
            let mut kfrom: Vec<std::result::Result<i64, usize>> = consts.iter().map(|c| Ok(c.1.expect("checked"))).collect();
            let mut free: Vec<(usize, usize)> = Vec::new();
            let mut repeat: Vec<(usize, usize)> = Vec::new();
            for (c, t) in args.iter().enumerate() {
                let Some(i) = t.var().map(|v| v.index()) else { continue };
                if bound[i] { kcols.push(c); kfrom.push(Err(i)); }
                else if free.iter().any(|f| f.1 == i) { repeat.push((c, i)); }
                else { free.push((c, i)); }
            }
            // An expansion samples `width` of its matches, or 16 times that where checks follow it (a filter, the
            // atom closing a cycle): a check passing few rows is measured on enough of them. The sample is thinned
            // back to `width` before the next expansion.
            let expands = !a.neg() && !free.is_empty();
            let reservoir = if expands {
                // Decision 0247: thinned by the next step's size. At most `16 * width` by weight first, then `width`
                // by weight times each binding's candidates here (`probe_size`): a uniform thinning of equal weights
                // dropped a large function's rows as often as a small one's, and walks varied by 27% to 41%.
                if sample.len() > 16 * width { sample = self.priority(sample, 16 * width); }
                if sample.len() > width {
                    let sizes: Vec<f64> = sample.iter().map(|(w, bind)| {
                        let key: Vec<i64> = kfrom.iter().map(|k| match *k { Ok(v) => v, Err(i) => bind[i] }).collect();
                        w * rel.probe_size(&kcols, &key)
                    }).collect();
                    self.probes.set(self.probes.get() + sample.len() as u64);
                    sample = self.priority_sized(sample, &sizes, width);
                }
                let mut after = bound.clone();
                for &(_, i) in &free { after[i] = true; }
                let checked = o[si + 1..].iter().find(|x| x.at != self.lead).is_some_and(|x| match b.view(x.at) {
                    Lit::Atom(_) => b.args(x.at).iter().all(|t| t.var().is_none_or(|v| after[v.index()])),
                    _ => false,
                });
                if checked { 16 * width } else { width }
            } else { width };
            // Decision 0247: the matches carried on, each with the share it stands for: its binding's, times the
            // probe's estimate over the rows it read (a probe reads at most `reservoir` of its candidates). They
            // are thinned by priority sampling (`priority`). A uniform draw over the rows read gave a function of
            // 20,000 nodes the weight of one of 1,024, and walked the data-layer line `1.0.2.3.4.5` at 0.39 of
            // its work.
            let mut next: Vec<(f64, Vec<i64>)> = Vec::new();
            let t_step = super::native::component::thread_cpu();
            let n_step = sample.len() as u64;
            let mut r_step = 0u64;
            for (wb, bind) in &sample {
                let key: Vec<i64> = kfrom.iter().map(|k| match *k { Ok(v) => v, Err(i) => bind[i] }).collect();
                let (mut found, mut read) = (false, 0u64);
                let mut mine: Vec<Vec<i64>> = Vec::new();
                self.probes.set(self.probes.get() + 1);
                // Decision 0236: a probe with more candidates than the reservoir reads that many of them, drawn by
                // position, and its matches are estimated from their share (an order probing `kind` by its
                // constant alone read every call).
                let est = rel.probe_sample(&kcols, &key, reservoir, &mut || self.draw(), &mut |row| {
                    read += 1;
                    let ok = repeat.iter().all(|&(c, i)| free.iter().find(|f| f.1 == i).is_some_and(|f| row(f.0) == row(c)));
                    if !ok { return }
                    found = true;
                    if expands {
                        let mut e = bind.clone();
                        for &(c, i) in &free { e[i] = row(c); }
                        mine.push(e);
                    }
                });
                r_step += read;
                if expands {
                    let w = if read > 0 { wb * est / read as f64 } else { 0.0 };
                    next.extend(mine.into_iter().map(|e| (w, e)));
                } else if a.neg() != found {
                    next.push((*wb, bind.clone()));
                }
            }
            { let mut sp = self.spent.borrow_mut(); let e = sp.entry(s.at).or_default(); e.0 += n_step; e.1 += r_step; e.2 += super::native::component::thread_cpu() - t_step; }
            if expands {
                for &(_, i) in &free { bound[i] = true; }
                // Carried at most `16 * width`: the next expansion thins them by its own size.
                if next.len() > 16 * width { next = self.priority(next, 16 * width); }
            }
            sample = next;
        }
        // Decision 0273: the rows the last step makes are work too, as the engine counts a line's work
        // (`profile::plan_work`: each step's rows in, and the last step's rows out). Without them, a walk exact by its
        // own count read 0.87 of the engine's work on the heavy lines of both packs, and a common term of every
        // order's walk was missing from their ratios.
        work + sample.iter().map(|x| x.0).sum::<f64>()
    }
}

impl Walker<'_, '_> {
    /// Decision 0247: priority sampling (Duffield, Lund and Thorup): `k` of `items`, each a weight and a binding,
    /// whose weights estimate any subset's sum without bias. Each item's priority is its weight over a uniform
    /// draw; the `k` highest are kept. An item heavier than the next priority is always among them, and keeps its
    /// own weight: a large function's bindings are carried, not drawn. The others share what the items not that
    /// heavy weighed, so a thinning keeps the total (with each at the next priority, as the estimator has it, the
    /// total varied by an eighth at 64, and a walk that kept it to 1.04 of its work went to 1.22).
    fn priority(&self, mut items: Vec<(f64, Vec<i64>)>, k: usize) -> Vec<(f64, Vec<i64>)> {
        if items.len() <= k || k == 0 { return items }
        let all: f64 = items.iter().map(|x| x.0).sum();
        let mut keyed: Vec<(f64, usize)> = items.iter().enumerate().map(|(i, x)| {
            let u = ((self.draw() >> 11) as f64 + 0.5) / (1u64 << 53) as f64;
            (x.0 / u, i)
        }).collect();
        keyed.select_nth_unstable_by(k, |a, b| b.0.total_cmp(&a.0));
        let tau = keyed[k].0;
        let keep: Vec<usize> = keyed[..k].iter().map(|x| x.1).collect();
        let heavy: f64 = keep.iter().map(|&i| items[i].0).filter(|&w| w > tau).sum();
        let light = keep.iter().filter(|&&i| items[i].0 <= tau).count();
        let share = if light > 0 { ((all - heavy) / light as f64).max(0.0) } else { 0.0 };
        let mut out = Vec::with_capacity(k);
        for i in keep {
            let (w, b) = std::mem::take(&mut items[i]);
            out.push((if w > tau { w } else { share }, b));
        }
        out
    }

    /// `priority` by a size: each item's priority is its size over a uniform draw, the `k` highest kept. An item
    /// larger than the next priority keeps its weight; the others' sizes share what the items not that large
    /// sized, and each weight is scaled by its share over its size. An item of no size is dropped (it reaches no
    /// row). Unbiased for sums that grow with the size, and exact for those proportional to it.
    fn priority_sized(&self, mut items: Vec<(f64, Vec<i64>)>, sizes: &[f64], k: usize) -> Vec<(f64, Vec<i64>)> {
        let live: Vec<usize> = (0..items.len()).filter(|&i| sizes[i] > 0.0).collect();
        if live.len() <= k { return live.into_iter().map(|i| std::mem::take(&mut items[i])).collect() }
        let all: f64 = live.iter().map(|&i| sizes[i]).sum();
        let mut keyed: Vec<(f64, usize)> = live.iter().map(|&i| {
            let u = ((self.draw() >> 11) as f64 + 0.5) / (1u64 << 53) as f64;
            (sizes[i] / u, i)
        }).collect();
        keyed.select_nth_unstable_by(k, |a, b| b.0.total_cmp(&a.0));
        let tau = keyed[k].0;
        let keep: Vec<usize> = keyed[..k].iter().map(|x| x.1).collect();
        let big: f64 = keep.iter().map(|&i| sizes[i]).filter(|&z| z > tau).sum();
        let small = keep.iter().filter(|&&i| sizes[i] <= tau).count();
        let share = if small > 0 { ((all - big) / small as f64).max(0.0) } else { 0.0 };
        keep.into_iter().map(|i| {
            let (w, b) = std::mem::take(&mut items[i]);
            (if sizes[i] > tau { w } else { w * share / sizes[i] }, b)
        }).collect()
    }

    /// The sampler's next draw (xorshift): walks are unbiased, and repeatable from the seed.
    fn draw(&self) -> u64 {
        let mut x = self.seed.get();
        x ^= x << 13; x ^= x >> 7; x ^= x << 17;
        self.seed.set(x);
        x
    }
}

#[cfg(test)]
mod ordered_tests {
    use super::*;

    /// Q88: a chunk plans from the run's plans as filed once the chunks `window` before it were, never from a chunk
    /// beside it: what each sees is fixed by the chunks' order, whatever finishes or is filed first. What it plans and
    /// learns is its own until it is filed.
    #[test]
    fn a_chunk_sees_the_plans_filed_before_its_window_never_a_neighbours() {
        let _plans = KeepPlans::start(None);
        let mut ordered = OrderedPlans::begin(2).expect("a corpus run's plans");
        let put = |key: PlanKey, w: f64| PLANS.with(|p| plans_put(p.borrow().as_ref().unwrap(), key, (Arc::from(Vec::<Step>::new()), w, None, 1.0, None, None)));
        let get = |key: PlanKey| plans_get(&key).map(|k| k.1);
        let (v0, v1) = (ordered.next(), ordered.next());
        let ((), m0) = in_view(&v0, || put(1, 10.0));
        // Chunk 1 began beside chunk 0: chunk 0's plan is not in its view, though chunk 0 has finished.
        let (seen, m1) = in_view(&v1, || {
            let seen = get(1);
            put(2, 20.0);
            seen
        });
        assert_eq!(seen, None, "a neighbour's plan, finished first");
        assert_eq!(get(1), None, "the run's plans hold no chunk's until it is filed");
        ordered.file(m0);
        assert_eq!(get(1), Some(10.0), "chunk 0 filed");
        assert_eq!(in_view(&v1, || get(1)).0, None, "a view stays as its chunk began");
        ordered.file(m1);
        // Chunk 2 sees chunk 0's (`window` back), and not chunk 1's though it was filed before chunk 2 began.
        let v2 = ordered.next();
        assert_eq!(in_view(&v2, || (get(1), get(2))).0, (Some(10.0), None));
        // What it learns of a plan it read is its own copy's: the run's entry is unchanged until it is filed.
        let ((), m2) = in_view(&v2, || plans_learnt(&1, Learnt::Ran(5.0)));
        assert_eq!(plans_get(&1).and_then(|k| k.5), None, "unfiled");
        ordered.file(m2);
        assert_eq!(plans_get(&1).and_then(|k| k.5), Some(5.0), "filed");
    }
}
