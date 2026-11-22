//! The planner's search, below its run (`plan.rs`, rerail-planner's): the planner's inputs and statistics
//! (`Inputs`, `Stats`, `LiveRel`), its model of the data (`Model`), the search for a rule's plan and the plans kept
//! across a run's chunks and its strata (`KeptPlans`, `OrderedPlans`, `in_view`), `make_plan` and the SQL of its
//! expressions. rerail-plan-search's `plan` mounts it, with the passes that read the model; `plan.rs` re-exports it.

use super::*;

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
pub fn native_modes(modes: &str) -> BTreeSet<String> {
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
pub fn unhex(s: &str) -> Option<String> {
    let bytes = (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect::<Option<Vec<u8>>>()?;
    String::from_utf8(bytes).ok()
}

impl Inputs {
    /// T's facts file attached as `f`, statistics in it.
    pub fn file() -> Self {
        Inputs { facts: "f.".to_owned(), stats: "f.".to_owned(), atoms: None, native: BTreeSet::new(), batch: HashMap::new(), given: None, kept: BTreeSet::new() }
    }
}

fn sql_constant(v: &SqlValue) -> Option<Constant> {
    match v {
        SqlValue::Integer(i) => Some(Constant::Int(*i)),
        SqlValue::Text(s) => Some(Constant::Str(s.clone())),
        _ => None,
    }
}

pub fn int(v: &SqlValue) -> i64 {
    match v {
        SqlValue::Integer(i) => *i,
        _ => 0,
    }
}

pub fn text(v: &SqlValue) -> String {
    match v {
        SqlValue::Text(s) => s.clone(),
        SqlValue::Integer(i) => i.to_string(),
        _ => String::new(),
    }
}

/// A plan's rows after each of its steps, every row walked (`StepPlan::all`): what the device sizes its stages' tables
/// by before they have run (`JoinSpec::est`).
pub fn step_estimates(x: &super::super::profile::RulePlan) -> super::super::native::setfix::spec::Est {
    super::super::native::setfix::spec::Est(x.steps.iter().map(|s| s.all).collect())
}

/// Tells each kind-set axis a planned order reads by its `n` how many keys come into that step (the rows the planner
/// expects after the step before): the axis chooses between its batch's solve and lookups per node (decision 0143).
pub fn expect_axis_lookups(db: impl crate::code_rule_dsl::facts::vtab::Scoped, plan: &super::super::profile::RulePlan) {
    let mut into = 1.0;
    for step in &plan.steps {
        let rel = step.literal.split('(').next().unwrap_or("").trim();
        if super::super::front::is_transduced(rel) && (step.access.starts_with("lookup on (n") || step.access.starts_with("membership on (n"))
            && let Some(source) = crate::code_rule_dsl::facts::vtab::bound_source(&db, rel) {
                source.expect_lookups(0, into);
            }
        into = step.rows;
    }
}

/// The positive atoms of a body: a body of one has one order.
pub fn positive_atoms(b: &Body<'_>) -> usize {
    (0..b.len()).filter(|&i| matches!(b.view(i), Lit::Atom(a) if !a.neg())).count()
}

/// Bindings a step must have seen before its observed fanout replaces the estimate.
pub const OBS_MIN: i64 = 32;

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
    /// A view's statistics counted where its rows lie, as a copy's are (`LiveRel`, `Run::hold_view_in_place`):
    /// `Model::resolve` applies them as `live_stats` does, exact, and never a layout's answer (decision 0361) in
    /// their place. `None`: they are `statistics`', as measured.
    fn live(&self) -> Option<LiveRel> {
        None
    }
    /// Whether they are `live`'s: known without counting them.
    fn is_live(&self) -> bool {
        false
    }
}

/// The statistics of a view of a batch's `node` held in place (decision 0329), counted off the batch's documents
/// (`syntax::node_column_counts`) when a plan first reads them (Q332): counting them makes every document's
/// projection, which a run whose plans never read the view (a warm run's, its strata kept) has no need of.
pub struct InPlaceView {
    pub rel: String,
    pub batch: Arc<crate::code_rule_dsl::facts::vtab::syntax::Batch>,
    pub cols: Vec<usize>,
    pub rows: i64,
}
impl InPlaceView {
    fn counted(&self) -> LiveRel {
        let _span = super::super::profile::span(|| format!("statistics of {}, a view held in place, counted when first planned with", self.rel));
        let (mut ndv, mut sq, mut freq) = (Vec::new(), Vec::new(), Vec::new());
        for (j, &c) in self.cols.iter().enumerate() {
            let (d, s, counts) = crate::code_rule_dsl::facts::vtab::syntax::node_column_counts(&self.batch, c).expect("a column node_column_counts counts");
            ndv.push(d);
            sq.push(s);
            if let Some(counts) = counts {
                freq.push((j, counts.into_iter().map(|(v, m)| (Constant::Int(v), m)).collect()));
            }
        }
        LiveRel { n: self.rows, delta: self.rows, ndv, sq, freq, keys: Vec::new(), counts: Vec::new() }
    }
}
impl Deferred for InPlaceView {
    fn statistics(&self) -> (Vec<(String, [i64; 4])>, Vec<(String, i64, i64)>) {
        let live = self.counted();
        let cols = (0..self.cols.len()).map(|j| (sql_col(&self.rel, j), [live.n, live.ndv[j], live.sq[j], live.sq[j]])).collect();
        let vals = live.freq.iter().flat_map(|(j, f)| f.iter().filter_map(|(v, &m)| match v { Constant::Int(v) => Some((sql_col(&self.rel, *j), *v, m)), _ => None })).collect();
        (cols, vals)
    }
    fn live(&self) -> Option<LiveRel> {
        Some(self.counted())
    }
    fn is_live(&self) -> bool {
        true
    }
}
/// The statistics of a view of any other source's columns held in place (`Run::hold_view_in_place`): its columns'
/// counts, the source's own or measured off its rows, when a plan first reads them. Counted at once, a view no plan
/// read (a quiet stratum's reader, a warm run's) made every document of a batch's relation (`string_annotation`).
pub struct SourceView {
    pub rel: String,
    pub src: Arc<dyn crate::code_rule_dsl::facts::vtab::Source>,
    pub cols: Vec<usize>,
    pub rows: i64,
}
impl SourceView {
    fn counted(&self) -> LiveRel {
        let _span = super::super::profile::span(|| format!("statistics of {}, a view held in place, counted when first planned with", self.rel));
        let all = crate::code_rule_dsl::facts::vtab::syntax::statistics(self.src.as_ref());
        let at = |c: usize| all.get(c).map_or((self.rows, self.rows), |s| (s.2, s.3));
        let (ndv, sq) = self.cols.iter().map(|&c| at(c)).unzip();
        LiveRel { n: self.rows, delta: self.rows, ndv, sq, freq: Vec::new(), keys: Vec::new(), counts: Vec::new() }
    }
}
impl Deferred for SourceView {
    fn statistics(&self) -> (Vec<(String, [i64; 4])>, Vec<(String, i64, i64)>) {
        let live = self.counted();
        let cols = (0..self.cols.len()).map(|j| (sql_col(&self.rel, j), [live.n, live.ndv[j], live.sq[j], live.sq[j]])).collect();
        (cols, Vec::new())
    }
    fn live(&self) -> Option<LiveRel> {
        Some(self.counted())
    }
    fn is_live(&self) -> bool {
        true
    }
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
pub struct LazyMap(pub HashMap<String, std::sync::Arc<dyn Deferred>>);
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

/// (`stat_*` in the facts database) in one statement. Nothing is counted at plan time (SQL index prototype S174, S178).
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
    /// The mean subtree size of the child by field `f` of a node of kind `pk`: its child kinds' sizes (`size`),
    /// weighed by how often each is that child. `None` without such edges, or without a size for any of their kinds.
    pub fn part_size(&self, pk: &str, f: &str, size: &HashMap<String, f64>) -> Option<f64> {
        let (mut edges, mut nodes) = (0.0, 0.0);
        for ((p, c, field), &n) in &self.joint {
            if p == pk && field == f && let Some(&s) = size.get(c) {
                edges += n;
                nodes += n * s;
            }
        }
        (edges > 0.0).then(|| nodes / edges)
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
    /// A line whose delta is read through its own index (a maintain mode's removal or add join, decision 0364): the
    /// delta (`first`) is sized by its rows but need not lead, and a probe of it is the relation's probe at the
    /// delta's share of its rows. A semi-naive probe's delta leads its wave (`false`).
    pub lead_free: bool,
    /// (rel, col) → posting-weighted fanout of an equality probe: sq / rows.
    pub fan: ColMap<f64>,
    /// (derived rel, col) → {value: postings}, for a column of few values (native statistics).
    pub dfreq: ColMap<HashMap<Constant, i64>>,
    /// (rel, col) → its heavy values (decision 0209 §4), made for the columns a line raised (`Run::heavy_for`).
    /// Shared, not copied, by a component's model.
    pub heavy: ColMap<std::sync::Arc<table::HeavyCol>>,
    /// (derived rel, col) → its keys (decision 0488: the native relation's sketch's few values, or its filter), read
    /// where the engine keeps them: the keys two columns share price a probe's hits (`atom_rows`).
    pub keys: ColMap<super::super::bloom::Keys>,
    /// (derived rel, col) → its counts for join sizes (decision 0488: the native relation's sketch's counters,
    /// `join_size`): a probe by one atom's column is priced by the rows the two columns join (`atom_rows`).
    pub counts: ColMap<std::sync::Arc<super::super::join_size::JoinCounts>>,
    /// rel → the engine's relation as its heavy values were made or last merged over (`InRel::mark`): its change
    /// since is merged into them, not counted again (O172).
    pub heavy_mark: table::RelMap<std::sync::Arc<super::super::native::component::RelMark>>,
    /// (rel, col) → heavy values a model or any of its forks made, with the engine's relation as they were made
    /// over (`InRel::mark`): shared by a model's forks (`fork`), which plan a wave's components apart and take their
    /// own back only as each retires. A fork takes an entry whose mark finds the relation unchanged (decision 0488:
    /// corpus0's components counted each chunk's `kind` view again, 107 times). Held by the model, not the process.
    pub heavy_shared: std::sync::Arc<parking_lot::Mutex<HashMap<(String, usize), (std::sync::Arc<super::super::native::component::RelMark>, std::sync::Arc<table::HeavyCol>)>>>,
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
    pub indexes: std::sync::Arc<HashMap<String, Vec<Vec<String>>>>,
    /// The inputs the run binds as rows it holds, not a view of a source (`setfix::input_of`'s `held_rel`): the
    /// cross part's base relations held and the file-local part's exports. A probe of one reads its sorted order, so a
    /// range of its next column is read as a range (`setfix::ranges`, `Model::plan_rule`'s `range_pass`).
    pub held: std::sync::Arc<BTreeSet<String>>,
    /// Documents (node-id intervals, `stat_docrange`), if the facts have them.
    pub docs: Option<i64>,
    /// Exact postings of the program's constants: table → (column, value) → n. By table first: a relation's are
    /// read without a pass over every table's.
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
    pub lazy: LazyMap,
    /// Quiet strata's relations, whose columns take their `__old`'s statistics when first planned with
    /// (`prior_stats`, `Model::resolve`): their rows are known without them.
    pub prior_later: BTreeSet<String>,
}

impl Stats {
    /// The recursive component being planned (`solving`), and its digest.
    pub fn solve(&mut self, scc: BTreeSet<String>) {
        self.solving_digest = super::super::derived::digest(|d| {
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
        self.solving_digest = super::super::derived::digest(|d| {
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
            lead_free: false,
            fan: Default::default(),
            dfreq: Default::default(),
            heavy: Default::default(),
            keys: Default::default(),
            counts: Default::default(),
            heavy_mark: Default::default(),
            heavy_shared: Default::default(),
            obs: HashMap::new(),
            solving: BTreeSet::new(),
            held: Default::default(),
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

    /// The tree's nodes by kind atom, exactly: `node`'s kind column's value counts (decision 0458: kept and patched
    /// by document, as every base relation's are). They replace `tree`'s, whose counts are a sample's, scaled. The
    /// planner reads `kind`'s values against `node`'s statistics (`view_stats_held`), so the two must be counted
    /// together. A kind's mean subtree size stays `tree`'s: it is a mean, not a count.
    pub fn kinds_counted(&mut self, counts: impl IntoIterator<Item = (i64, i64)>) {
        let mut counts: Vec<(i64, i64)> = counts.into_iter().filter(|&(_, n)| n > 0).collect();
        counts.sort_unstable();
        self.nodes = counts.iter().map(|&(_, n)| n).sum::<i64>() as f64;
        self.kfreq = Arc::new(counts.into_iter().map(|(k, n)| (k.to_string(), n)).collect());
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
        self.card.insert(rel, rows as f64);
        self.lazy.0.insert(rel.to_owned(), deferred);
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
    pub fn read(db: &Connection, p: &body::Placed, inputs: &Inputs, fingerprint: bool) -> Result<Self> {
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
            lead_free: false,
            fan: Default::default(),
            dfreq: Default::default(),
            heavy: Default::default(),
            keys: Default::default(),
            counts: Default::default(),
            heavy_mark: Default::default(),
            heavy_shared: Default::default(),
            obs: HashMap::new(),
            solving: BTreeSet::new(),
            held: Default::default(),
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
    pub analysis: std::sync::OnceLock<std::sync::Arc<super::super::analysis::ProgramAnalysis>>,
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

/// Bound while a rule is searched without its join shape directives (`Model::plan_inner`).
#[derive(Clone, Copy, Default)]
struct NoShapes(bool);

#[cfg_attr(not(any(test, feature = "test-hooks")), allow(dead_code))]
pub fn shaped_statistics() -> u64 {
    run_counts().shaped.load(std::sync::atomic::Ordering::Relaxed)
}

impl Model {
    /// Whether `rel` is one of `prog`'s inputs. A program may have thousands, and the planner asks for every atom it
    /// prices: a scan of the list was 40% of `plan_search` in a session step. A long list is hashed once per model;
    /// the programs a model plans are alive while it is, so an address and length name one list.
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
            // A view held in place: its exact statistics, as when they were counted at once (`InPlaceView`).
            if let Some(live) = deferred.live() {
                live_stats(self, rel, live);
                continue;
            }
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
                run_counts().shaped.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                super::super::profile::decision("statistics answered by a layout (decision 0361)", || rel.to_owned());
                continue;
            }
            let _span = super::super::profile::span(|| format!("statistics of {rel}, made when first planned with"));
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
            lead_free: false,
            fan: Default::default(),
            dfreq: Default::default(),
            heavy: Default::default(),
            keys: Default::default(),
            counts: Default::default(),
            heavy_mark: Default::default(),
            heavy_shared: std::sync::Arc::clone(&st.heavy_shared),
            obs: rules.iter().filter_map(|r| Some((*r, st.obs.get(r)?.clone()))).collect(),
            solving: BTreeSet::new(),
            held: st.held.clone(),
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
            stats.keys.share_from(&st.keys, r, w);
            stats.counts.share_from(&st.counts, r, w);
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
            self.stats.keys.take_from(&mut fork.stats.keys, r, w);
            self.stats.counts.take_from(&mut fork.stats.counts, r, w);
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
    pub fn read_by_body<'v>(b: &Body<'v>) -> Vec<&'v str> {
        let atoms = || (0..b.len()).filter_map(|i| b.atom(i));
        let pos: Vec<AtomRef<'v>> = atoms().filter(|a| !a.neg()).collect();
        let plain = pos.len() == 1 && {
            let mut seen = VarSet::default();
            pos[0].args().all(|t| t.var().is_some_and(|x| seen.insert(x)))
        };
        if plain { Vec::new() } else { atoms().map(|a| b.rel(a)).collect() }
    }

    /// Whether `rel` is one of the program's inputs (`f`, its version's: a set read once per version).
    pub fn is_input(&self, f: Fields<'_>, rel: &str) -> bool {
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

pub use super::super::sql::select::sql_col;

pub const PLAN_CACHE_NAMES: [&str; 12] = ["no cache", "first sight", "kept price", "re-priced", "replanned: cheaper start", "replanned: work grew", "replanned: no longer runs", "taken from the store", "kept: ran below a search", "searched: ran over its price", "started: its line's order", "started: its source rule's order"];
/// `RERAIL_PLAN_FRESH`: every plan searched, none reused (a measurement: what kept orders cost).
fn plan_fresh() -> bool {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var_os("RERAIL_PLAN_FRESH").is_some())
}
/// Where a plan is being made, for the profile's account of the planning budget's misses (in the run's context).
#[derive(Clone, Copy)]
struct PlanSite(&'static str);
impl Default for PlanSite {
    fn default() -> Self {
        PlanSite("elsewhere")
    }
}
pub fn plan_site(site: &'static str) {
    rerail_prof::code_rule_dsl::logic::profile::bind_here(PlanSite(site));
}
#[cfg_attr(not(any(test, feature = "test-hooks")), allow(dead_code))]
pub fn lazy_probes_counted() -> (u64, u64) {
    (run_counts().lazy[0].load(std::sync::atomic::Ordering::Relaxed), run_counts().lazy[1].load(std::sync::atomic::Ordering::Relaxed))
}

fn plan_cache(outcome: usize) {
    run_counts().plan_cache[outcome].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// A run's quiet strata (decision 0101 §1), counted across its strands (`Run::strand`) and reported in its
/// `Outcome`: the strata passed on unchanged, and the relations held as kept with their rows' count alone, no
/// column measured (`quiet_card`). Each column of those reads as one value, and every probe of one as all its rows.
/// None should be (the guard in `a_session_passes_on_a_quiet_stratum`). Per run, not per process: a process-wide
/// count let one test's runs move another's.
#[derive(Debug, Default)]
pub struct QuietTally {
    pub strata: std::sync::atomic::AtomicUsize,
    pub unmeasured: std::sync::atomic::AtomicUsize,
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
pub fn lazy_probes() -> bool {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var("RERAIL_LAZY_PROBES").map_or(true, |v| v != "off"))
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

/// `f` of the run's kept plans (`KeptPlans`, the base's binding of them): this search's (`SharedPlans`), the one
/// kind there is.
pub fn with_plans<R>(f: impl FnOnce(Option<&SharedPlans>) -> R) -> R {
    kept_plans(|p| f(p.map(|p| p.as_any().downcast_ref::<SharedPlans>().expect("the run's kept plans are the search's"))))
}

impl KeptView for SharedPlans {
    fn chunk_memo(&self) -> Option<Arc<joins::ChunkMemo>> {
        self.view.as_ref().map(|v| Arc::clone(&v.memo))
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

thread_local! {
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
    with_plans(|p| {
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
pub fn plans_get(key: &PlanKey) -> Option<Kept> {
    with_plans(|p| {
        let shared = p.as_ref()?;
        OWN_PLANS.with(|o| o.borrow().as_ref().and_then(|m| m.get(key).cloned())).or_else(|| shared.now().get(key).map(|e| e.kept()))
    })
}

/// Keeps `kept` for `key` (decision 0301): where the run holds `key` with the same order, in its entry's cell, seen
/// by every job at once; a new key or a new order in this thread's job's plans (published as it ends), else published
/// now.
pub fn plans_put(shared: &SharedPlans, key: PlanKey, kept: Kept) {
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
pub fn plans_learnt(key: &PlanKey, learnt: Learnt) {
    with_plans(|p| {
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
pub enum Learnt {
    Priced(Arc<Priced>),
    Ran(f64),
}

/// A kept plan: its order, its work per node when searched, when profiling its EXPLAIN, its work over its body's
/// cheapest start (`SEED`) when searched, its last price with the sizes it was priced at (`Priced`), and the work
/// its rule's line did in the engine when it last ran (`ran`, decision 0122), if it has run.
pub type Kept = (Arc<[Step]>, f64, Option<super::super::profile::RulePlan>, f64, Option<Arc<Priced>>, Option<f64>);

/// Decision 0122: planning costs no more than running. A search costs about as much as this many rows of the
/// engine's join work (O125 step 07: 0.16 ms a search, 0.74 µs a row of work), so a rule whose line did less work
/// when it last ran, its inputs grown by no more than that allows, keeps its order with no search and no pricing;
/// in a run, a probe is searched again only once its work has paid for its searches. `RERAIL_PLAN_BUDGET=off`
/// plans as before (a measurement aid).
pub const SEARCH_WORK: f64 = 250.0;

pub fn plan_budget() -> bool {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| !std::env::var("RERAIL_PLAN_BUDGET").is_ok_and(|v| v == "off"))
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
pub struct SharedPlans {
    pub run: Arc<RunPlans>,
    pub view: Option<Arc<ChunkView>>,
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

/// A chunk's view of the run's plans (Q88): the run's as filed when it began (`OrderedPlans::next_view`), with what it
/// planned and learnt over them, its own entries (`own`), handed back with it (`in_view`).
pub struct ChunkView {
    kept: arc_swap::ArcSwap<Plans>,
    own: parking_lot::Mutex<rustc_hash::FxHashSet<PlanKey>>,
    observed: Option<keeps::SharedObserved>,
    /// The joins' memo as the run's was filed when it began, with its own writes (Q314, `joins::ChunkMemo`).
    memo: Arc<joins::ChunkMemo>,
    /// The planning choices (inline or store, demand) likewise (Q314, `choices::ChunkChoices`).
    choices: Arc<choices::ChunkChoices>,
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
pub struct ChunkPlans(Vec<(PlanKey, Kept)>, Option<keeps::Observed>, Vec<joins::Op>, Vec<([u8; 32], choices::Chosen, bool)>);

/// Q88: the run's plans and observations as its chunks are filed, in their order. Chunk `n` (from 0) sees them as they
/// were once chunks `0..=n - window` were filed; what it made is the run's when it is filed.
pub struct OrderedPlans {
    run: SharedPlans,
    observed: Option<keeps::SharedObserved>,
    window: usize,
    /// The run's plans, observations and joins' memo once `first + i` chunks were filed, for those a chunk to begin
    /// may see.
    snapshots: std::collections::VecDeque<(Arc<Plans>, Option<Arc<keeps::Observed>>, Arc<joins::Memo>, ChoicesAt)>,
    first: usize,
    begun: usize,
    filed: usize,
}

impl OrderedPlans {
    /// Over this thread's run's plans: `None` outside a corpus run.
    pub fn begin(window: usize) -> Option<Self> {
        let run = share_plans()?;
        let observed = keeps::share_observed();
        let at = (run.kept.load_full(), keeps::observed_now(&observed), joins::snapshot(), choices::snapshot());
        let observed = at.1.is_some().then_some(observed);
        Some(OrderedPlans { run, observed, window: window.max(1), snapshots: [at].into(), first: 0, begun: 0, filed: 0 })
    }

    /// The next chunk's view.
    pub fn next_view(&mut self) -> Arc<ChunkView> {
        let n = self.begun;
        self.begun += 1;
        let at = (n + 1).saturating_sub(self.window);
        assert!(at <= self.filed, "Q88: chunk {n} begins with {} filed, under {at}", self.filed);
        while self.first < at {
            self.snapshots.pop_front();
            self.first += 1;
        }
        let (plans, observed, memo, chosen) = self.snapshots[at - self.first].clone();
        Arc::new(ChunkView {
            kept: arc_swap::ArcSwap::new(plans),
            own: Default::default(),
            observed: self.observed.as_ref().map(|_| keeps::chunk_observed(observed)),
            memo: joins::chunk_memo(&memo),
            choices: choices::chunk_choices(&chosen),
        })
    }

    /// Files the next chunk's plans and observations into the run's, in the chunks' order.
    pub fn file(&mut self, made: ChunkPlans) {
        let ChunkPlans(plans, observed, ops, chosen) = made;
        self.run.publish(plans);
        if let (Some(run), Some(o)) = (&self.observed, observed) {
            keeps::file_observed(run, o);
        }
        joins::file_ops(ops);
        choices::file(chosen);
        self.filed += 1;
        let at = (self.run.kept.load_full(), self.observed.as_ref().and_then(keeps::observed_now), joins::snapshot(), choices::snapshot());
        self.snapshots.push_back(at);
    }
}

/// `f` planning within a chunk's view (Q88): what it plans, learns and observes stays in the view (and the jobs it
/// starts, which enter it), and is handed back with its result for the run's (`OrderedPlans::file`).
pub fn in_view<R>(view: &Arc<ChunkView>, f: impl FnOnce() -> R) -> (R, ChunkPlans) {
    let Some(run) = share_plans() else { return (f(), ChunkPlans::default()) };
    let shared = SharedPlans { run: Arc::clone(&run.run), view: Some(Arc::clone(view)) };
    // The view is the plans its extent binds (its jobs carry it), and its own (`enter_plans`) go back to it.
    let r = {
        // The chunk's plans are the run's context's while `f` runs (and its jobs', which carry the context), and
        // what it plans is put into the view as the job's own end (`enter_plans`), before the binding goes.
        let _bound = rerail_prof::code_rule_dsl::logic::profile::bind(KeptPlans(Some(Box::new(shared.clone()))));
        let _plans = enter_plans(Some(shared));
        let _observed_at = view.observed.clone().map(keeps::bind_observed);
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
    (r, ChunkPlans(plans, view.observed.as_ref().map(keeps::observed_made), joins::ops_of(&view.memo), choices::made_in(&view.choices)))
}

/// The planning choices kept in the process, as a chunk begins from them (`choices::snapshot`).
type ChoicesAt = Arc<std::collections::HashMap<[u8; 32], choices::Chosen>>;

/// The planning choices a chunk's planning reads and makes (Q261): its view's, where the run is in one (`in_view`).
pub fn chunk_choices() -> Option<Arc<choices::ChunkChoices>> {
    with_plans(|p| p.and_then(|s| s.view.as_ref()).map(|v| Arc::clone(&v.choices)))
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
    explain: Option<super::super::profile::RulePlan>,
    gap: f64,
}

impl Entry {
    fn of(k: Kept) -> Arc<Entry> {
        let found = arc_swap::ArcSwap::from_pointee(Found { per_node: k.1, explain: k.2, gap: k.3 });
        Arc::new(Entry { order: k.0, found, priced: parking_lot::Mutex::new(k.4), ran: std::sync::atomic::AtomicU64::new(k.5.map_or(NOT_RAN, f64::to_bits)) })
    }

    /// The plan as `Kept`.
    pub fn kept(&self) -> Kept {
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
pub struct Plans {
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
    pub fn iter(&self) -> impl Iterator<Item = (&PlanKey, &Arc<Entry>)> {
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

/// A run's plans: those it made or took (`kept`), and those the store kept from earlier processes (`stored`,
/// decision 0116 step 2), taken into `kept` when a rule is first planned. Decision 0301: each part shared as it is
/// used. `kept` is read with no lock (RCU: an immutable map, a new one published for new plans), its entries'
/// learning in cells; the lines run and the keys planned are collected apart, under locks no planner reads under.
pub struct RunPlans {
    pub kept: arc_swap::ArcSwap<Plans>,
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
    pub orders: arc_swap::ArcSwap<rustc_hash::FxHashMap<u128, Arc<[Step]>>>,
    /// The prepared plans of the versions planned (decision 0361 step 5, Q228): the store's, read with the kept
    /// plans, and those this run made, by the version's identity; and whether it made any.
    prepared: arc_swap::ArcSwap<prepared::ByVersion>,
    prepared_new: std::sync::atomic::AtomicBool,
    /// The lines this run found heavy while they ran their prepared plan: searched from then on (`prepared_heavy`).
    prepared_searched: parking_lot::Mutex<HashSet<u128>>,
}

impl Default for RunPlans {
    fn default() -> Self {
        RunPlans::new(Default::default(), Default::default(), Default::default())
    }
}

/// Decision 0258: a line's key apart from what solves it: its body and its delta.
#[doc(hidden)]
pub fn line_key(b: &Body<'_>, first: Option<usize>) -> u128 {
    super::super::derived::digest(|d| {
        d.str("line");
        d.u64(b.key() as u64);
        d.u64((b.key() >> 64) as u64);
        d.u64(first.map_or(u64::MAX, |f| f as u64));
    })
}

/// The order last kept for line `line` (`RunPlans::orders`).
fn orders_get(line: u128) -> Option<Arc<[Step]>> {
    with_plans(|p| p.and_then(|s| s.orders.load().get(&line).cloned()))
}

/// Keeps `order` as line `line`'s last (`RunPlans::orders`).
fn orders_put(line: u128, order: &[Step]) {
    with_plans(|p| {
        if let Some(s) = p.as_ref() {
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
    rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var("RERAIL_SOURCE_ORDERS").is_ok_and(|v| v == "on"))
}

/// Whether `key` was planned in this thread's run (`RunPlans::planned_now`).
fn planned_now(key: &PlanKey) -> bool {
    OWN_PLANNED.with(|o| o.borrow().as_ref().is_some_and(|o| o.contains(key)))
        || with_plans(|p| p.is_some_and(|s| s.planned_now.lock().contains(key)))
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
        with_plans(|p| {
            if let Some(s) = p.as_ref() {
                s.planned_now.lock().insert(key);
            }
        });
    }
}

impl RunPlans {
    pub fn new(kept: rustc_hash::FxHashMap<PlanKey, Kept>, orders: rustc_hash::FxHashMap<u128, Arc<[Step]>>, stored: Arc<StoredPlans>) -> Self {
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
            prepared_searched: Default::default(),
        }
    }

    /// Tests: plans with `stored` as the store's.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn with_stored(stored: Arc<StoredPlans>) -> Self {
        RunPlans::new(Default::default(), Default::default(), stored)
    }

    /// Publishes `plans`, new keys or new orders (decision 0301): a new map, the old one's entries shared.
    fn publish(&self, plans: Vec<(PlanKey, Kept)>) {
        if plans.is_empty() {
            return;
        }
        let entries: Vec<(PlanKey, Arc<Entry>)> = plans.into_iter().map(|(k, v)| (k, Entry::of(v))).collect();
        run_counts().published[0].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        run_counts().published[1].fetch_add(entries.len() as u64, std::sync::atomic::Ordering::Relaxed);
        self.kept.rcu(|m| {
            let mut m = Plans::clone(m);
            m.extend(entries.iter().map(|(k, e)| (*k, Arc::clone(e))));
            m
        });
    }

    /// The run's plans as `Kept`, and its orders: a session keeps them for its next run.
    pub fn into_kept(self) -> (rustc_hash::FxHashMap<PlanKey, Kept>, rustc_hash::FxHashMap<u128, Arc<[Step]>>) {
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
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn find(&self, key: &PlanKey, len: usize, nodes: f64) -> Option<Kept> {
        if let Some(e) = self.kept.load().get(key) {
            return Some(e.kept());
        }
        let k = self.stored_near(key, len, nodes)?;
        self.publish(vec![(*key, k.clone())]);
        Some(k)
    }
}

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
pub fn scale(nodes: f64) -> u8 {
    (nodes.max(1.0).log2() / 2.0) as u8
}

impl StoredPlan {
    /// Whether `other` need not be written over this one: the same but for the work its line last ran (`ran`), where
    /// that is within 2x (the budget compares it with a search's cost, so an edit's few rows more or less are the
    /// same answer, and every plan is not written again each run).
    pub fn same(&self, other: &StoredPlan) -> bool {
        let ran = match (self.ran, other.ran) {
            (None, None) => true,
            (Some(a), Some(b)) => a.max(1.0) <= 2.0 * b.max(1.0) && b.max(1.0) <= 2.0 * a.max(1.0),
            _ => false,
        };
        ran && self.order == other.order && self.per_node == other.per_node && self.gap == other.gap && self.priced == other.priced
    }

    pub fn of(k: &Kept) -> Option<StoredPlan> {
        let steps = |order: &[Step]| order.iter().map(|s| Some((u16::try_from(s.at).ok().filter(|&a| a < 1 << 14)?, s.mode))).collect::<Option<Vec<_>>>();
        let priced = match &k.4 {
            Some(p) => Some((p.sizes.clone(), steps(&p.got.0.0)?, p.got.0.1, p.got.0.2)),
            None => None,
        };
        Some(StoredPlan { order: steps(&k.0)?, per_node: k.1, gap: k.3, ran: k.5.filter(|_| priced.is_some()), priced })
    }

    pub fn kept(&self, len: usize) -> Option<Kept> {
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
    with_plans(|p| {
        if let Some(p) = p.as_ref() {
            p.stored.store(stored);
        }
    });
}

/// Whether this thread runs with kept plans (`KeptPlans`): only then are prepared plans read from or kept for the store.
#[doc(hidden)]
/// Whether this thread's run searches line `line` rather than take its prepared plan: it ran heavy (`prepared_heavy`).
pub fn prepared_searched(line: u128) -> bool {
    with_plans(|p| p.is_some_and(|p| p.prepared_searched.lock().contains(&line)))
}

pub fn run_holds_plans() -> bool {
    with_plans(|p| p.is_some())
}

/// The prepared plans the store kept of the version whose identity is `id` (Q228).
#[doc(hidden)]
pub fn prepared_stored(id: &[u8; 32]) -> Option<Arc<prepared::Prepared>> {
    with_plans(|p| p.and_then(|p| p.prepared.load().get(id).cloned()))
}

/// Keeps `made`, the prepared plans this run made of the version whose identity is `id`, for the store. A version
/// with no prepared line is held but not new: the store keeps a row a line (`prepared::store_rows`), so it has none
/// of it, and every run makes it again; counted new, a warm run gathered a write of the kept plans as read
/// (`the_run_after_a_cold_one_writes_nothing`, Q243).
#[doc(hidden)]
pub fn prepared_made(id: [u8; 32], made: Arc<prepared::Prepared>) {
    let new = !made.0.is_empty();
    with_plans(|p| {
        if let Some(p) = p.as_ref() {
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
    with_plans(|p| {
        if let Some(p) = p.as_ref() {
            p.prepared.store(Arc::new(stored));
        }
    });
}

/// This thread's run's prepared plans for the store, the stored with the made: `None` where it made none.
pub fn prepared_to_store() -> Option<Arc<prepared::ByVersion>> {
    with_plans(|p| p.filter(|p| p.prepared_new.load(std::sync::atomic::Ordering::Relaxed)).map(|p| p.prepared.load_full()))
}

/// The prepared plans held for this thread's run, as read and made.
pub fn prepared_held() -> Arc<prepared::ByVersion> {
    with_plans(|p| p.map(|p| p.prepared.load_full()).unwrap_or_default())
}

/// This thread's run's plans for the store: the stored ones with the run's over them, or `None` where the run
/// changed none.
pub fn plans_to_store() -> Option<StoredPlans> {
    let plans = with_plans(|p| p.cloned())?;
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
/// read, its wall seconds, its plan key, each step's bindings in and out (`in/out`, by step, in the order run) and
/// its rule.
/// The run's own (opened once a run, decision 0442).
pub fn line_log() -> Option<std::sync::Arc<parking_lot::Mutex<std::fs::File>>> {
    crate::code_rule_dsl::logic::run::setting(|| {
        let p = std::env::var_os("RERAIL_JOIN_LINES")?;
        std::fs::OpenOptions::new().create(true).append(true).open(p).ok().map(|f| std::sync::Arc::new(parking_lot::Mutex::new(f)))
    })
}

/// Adds a stratum's lines as they ran to the run's (decision 0203 §4): sent to its log (decision 0301: a channel,
/// whose sends take no lock, that only `lines_take` reads).
pub fn lines_push(runs: Vec<joins::LineRun>) {
    if runs.is_empty() {
        return;
    }
    with_plans(|p| {
        if let Some(p) = p.as_ref() {
            let _ = p.lines.0.send(runs);
        }
    });
}

/// The lines run since the last call.
pub fn lines_take() -> Vec<joins::LineRun> {
    with_plans(|p| p.map(|p| p.lines.1.lock().try_iter().flatten().collect()).unwrap_or_default())
}

/// The plans the store kept for this thread's run, as read (the join memo's write, where no plan changed).
pub fn stored_plans() -> Option<StoredPlans> {
    with_plans(|p| p.map(|p| StoredPlans::clone(&p.stored.load())))
}

/// This thread's kept plans, for a job on another thread to enter.
fn share_plans() -> Option<SharedPlans> {
    with_plans(|p| p.cloned())
}

/// While the guard lives, this thread's job plans into plans of its own (`OWN_PLANS`: new keys and new orders),
/// published into the run's (`plans`, the job's caller's, `share_plans`) as it ends, its keys planned handed on then
/// too. The run's plans themselves are in the run's context, which the job's run carries.
fn enter_plans(plans: Option<SharedPlans>) -> impl Drop {
    type Own = (Option<rustc_hash::FxHashMap<PlanKey, Kept>>, Option<rustc_hash::FxHashSet<PlanKey>>);
    struct Entered(Option<SharedPlans>, Own);
    impl Drop for Entered {
        fn drop(&mut self) {
            let own = OWN_PLANS.with(|o| std::mem::replace(&mut *o.borrow_mut(), self.1.0.take()));
            let planned = OWN_PLANNED.with(|o| std::mem::replace(&mut *o.borrow_mut(), self.1.1.take()));
            let Some(shared) = self.0.take() else { return };
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
        plans,
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
    pub sizes: Vec<f64>,
    pub got: (Plan, Option<super::super::profile::RulePlan>),
}

/// How far a size may move before a kept price is worked out again (the cost model's, which setfix's waves read too).
pub use super::super::costs::PRICE_DRIFT;

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

/// Whether a plan is explained step by step whether or not the run is profiled (`explaining`): the choice of what a
/// relation keeps re-prices each step of its plans (decision 0099).
#[derive(Clone, Copy, Default)]
struct ExplainAlways(bool);

/// Plans explained step by step, `on` or not, while the guard lives.
pub fn explaining(on: bool) -> rerail_prof::code_rule_dsl::logic::profile::Bound {
    rerail_prof::code_rule_dsl::logic::profile::bind(ExplainAlways(on))
}

/// Bound while a plan's record is made for its steps' rows alone (`Model::plan_rule_step_rows`).
#[derive(Clone, Copy, Default)]
struct StepRows(bool);

/// Bound while the planner reads its statistics skewed (`Model::plan_inner`, `RERAIL_STATS_SKEW`).
#[derive(Clone, Copy, Default)]
struct Skewing(bool);

/// Carries the run's kept plans and the planner's settings to a job on another thread (`run_state`): the plans are
/// shared (what the job plans is taken into them as it ends, `enter_plans`), the settings copied.
pub fn carried(c: &mut super::super::run_state::Capture) {
    c.part(&["OWN_PLANS", "OWN_PLANNED"], super::super::run_state::Shared(share_plans(), enter_plans));
}

/// `RERAIL_STATS_SKEW=f[:seed]`: the largest factor a statistic is skewed by, and the seed of the factors.
fn skew_setting() -> Option<(f64, u64)> {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| {
        let v = std::env::var("RERAIL_STATS_SKEW").ok()?;
        let (f, seed) = v.split_once(':').map_or((v.as_str(), "0"), |(a, b)| (a, b));
        Some((f.parse().ok()?, seed.parse().ok()?))
    })
}

/// The factor statistic `what` of column `c` of `rel` is skewed by while skewing: fixed per (seed, rel, c, what),
/// log-uniform in [1/f, f]; 1 otherwise.
fn skew(rel: &str, c: usize, what: u64) -> f64 {
    let Some((f, seed)) = skew_setting().filter(|_| rerail_prof::code_rule_dsl::logic::profile::bound::<Skewing>().0) else { return 1.0 };
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
    pub fn analysis(&self, v: &Version) -> std::sync::Arc<super::super::analysis::ProgramAnalysis> {
        std::sync::Arc::clone(self.analysis.get_or_init(|| {
            let _analysis = super::super::profile::span(|| "logical key and scalar analysis".into());
            let analysis = program_analysis(v);
            let _explain = super::super::profile::span(|| "explain analysis".into());
            analysis.explain();
            analysis
        }))
    }

    /// `analysis` of program `p`.
    pub fn analysis_of(&self, p: &body::Placed) -> Result<std::sync::Arc<super::super::analysis::ProgramAnalysis>> {
        Ok(self.analysis(&p.v))
    }

    pub fn card(&self, rel: &str) -> f64 {
        // `node` is the corpus's nodes, except where a batch binds it: then its estimate is the batch's.
        if rel == "node" && !self.inputs.batch.contains_key(rel) {
            self.stats.nodes
        } else if rel.starts_with(super::super::strings::RELATION) {
            // A text test empty in the chunk that plans is not empty in the chunks that reuse its plans (decision
            // 0038): at least one string.
            self.stats.card.get(rel).copied().unwrap_or(1000.0).max(1.0)
        } else {
            // Any relation empty when it plans is not empty where its plans are reused, for the same reason: at least
            // one row. At 0, every step after it was priced 0 rows and every order was free: annotation demand's
            // `rel_base` planned before its guest round made `rr_ast_1_1`, scanned `pkg` whole and then probed
            // `docof` (10,728 -> 25.8 M -> 10,728 rows); `impmod`, `ruff_qual` and `frag_str` the same.
            self.stats.card.get(rel).copied().unwrap_or(1000.0).max(1.0)
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
    pub(crate) fn ndv_or_default(&self, rel: &str, c: usize) -> f64 {
        const DEFAULT_NUM_DISTINCT: f64 = 200.0;
        // Unmeasured: its layout's estimate (decision 0361 step 4), else the prior.
        let unseen = || {
            self.stats.shapes.ndv(rel, c, self.card(rel), self.stats.counts()).unwrap_or_else(|| if self.stats.solving.contains(rel) { DEFAULT_NUM_DISTINCT.min(self.card(rel)) } else { 1.0 })
        };
        let ndv = self.ndv.at(rel, c).copied().unwrap_or_else(unseen);
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
            } else if let Some(freq) = self.stats.dfreq.at(rel, c) {
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
        (0..b.args(i).len()).map(|c| self.ndv.at(rel, c).map(|n| n.min(cap))).collect()
    }

    /// Complete small value sets already in statistics. SQL can load only the constants requested by a rule,
    /// so an absent memo entry proves nothing unless both its distinct count and total cover the column.
    fn small_values(&self, rel: &str, c: usize) -> Option<Vec<&Constant>> {
        if let Some(freq) = self.stats.dfreq.at(rel, c) {
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

    /// Positive joins whose keys cannot occur in the forced first atom's values. Only that atom is a binding
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
            if !dargs.iter().any(|t| matches!(*t, TermRef::Var(v) if shares(v))) { return None; }
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
    pub fn search_steps_against_records(&self, pf: Fields<'_>, b: &Body<'_>, first: Option<usize>) -> Result<Vec<(Vec<super::super::profile::StepPlan>, Option<super::super::profile::RulePlan>)>> {
        let mut cx = joins::SearchCx::default();
        self.plan_inner_in(pf, b, first, None, false, &mut cx)?;
        Ok(cx.steps_of.into_iter().map(|(order, st)| (st, self.explain_order(pf, b, first, &order))).collect())
    }

    pub fn plan_rule_explained(
        &self,
        pf: Fields<'_>,
        b: &Body<'_>,
        first: Option<usize>,
    ) -> Result<(Plan, Option<super::super::profile::RulePlan>)> {
        self.plan_inner(pf, b, first, None, true)
    }

    /// `plan_rule_explained` for the rows expected after each step of the plan alone, as what a relation keeps
    /// calibrates them (decision 0099): the record is made without its strings, its steps' bases and the joins each
    /// step passed over, priced again for EXPLAIN.
    pub fn plan_rule_step_rows(&self, pf: Fields<'_>, b: &Body<'_>, first: Option<usize>) -> Result<(Plan, Vec<f64>)> {
        let explain = explaining(true);
        let step_rows = rerail_prof::code_rule_dsl::logic::profile::bind(StepRows(true));
        let got = self.plan_inner(pf, b, first, None, true);
        drop(step_rows);
        drop(explain);
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

    fn budget_takes(&self, kept: &Kept, b: &Body<'_>, first: Option<usize>, sizes: &[f64]) -> Option<(Plan, Option<super::super::profile::RulePlan>)> {
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
    pub fn has_deferred(&self) -> bool {
        !self.stats.lazy.0.is_empty() || !self.stats.prior_later.is_empty()
    }

    /// A relation whose statistics are still to be made: bound lazily (`Stats::lazily`), or quiet and taking its
    /// kept rows' (`prior_stats`).
    /// A relation its layout answers whole (`shapes::Shapes::answers`) is never one: its statistics are not made. A
    /// view held in place always is (`Deferred::is_live`): its statistics are counted, never a layout's.
    fn deferred(&self, rel: &str) -> bool {
        self.stats.lazy.0.get(rel).is_some_and(|d| d.is_live() || !self.shaped(rel)) || self.stats.prior_later.contains(rel)
    }

    /// Whether `rel`'s layout answers every column's distinct values (decision 0361 step 4).
    fn shaped(&self, rel: &str) -> bool {
        self.stats.shapes.answers(rel, self.stats.counts())
    }

    /// Decision 0123, step 1: the statistics a planning round requests. Of the lines it will plan (a rule and its
    /// delta atom; `None`: its seed or whole plan), those that will be priced (`needs_statistics`) request the
    /// relations they read whose statistics are still to be made. `plain` lines read none (`read_by`).
    pub fn wants<'r, 'v: 'r>(&self, lines: impl IntoIterator<Item = (&'r Body<'v>, Option<usize>)>, into: &mut BTreeSet<String>) {
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
                    super::super::profile::decision("statistics requested (decision 0123)", || format!("by a line priced: {why}, {} delta ({} relations)", if first.is_some() { "with a" } else { "no" }, if n > 3 { "4+".to_owned() } else { n.to_string() }));
                }
            }
        }
    }

    /// `wants` for the strata a level or a scoreboard runs next (each a component and its rules), before any of them
    /// runs: each stratum's seeds (a rule with no atom of the component) and probes (each
    /// atom of the component), asked as solving that component, whose digest is in their keys and whose relations
    /// their delta tests read. A stratum's own rounds make what these miss (its whole plans; a line whose rows moved).
    pub fn wants_strata<'r>(&mut self, vs: &body::Versions, strata: impl IntoIterator<Item = (&'r BTreeSet<String>, Vec<RuleKey>)>, into: &mut BTreeSet<String>) -> Result<()> {
        if self.stats.lazy.0.is_empty() && self.stats.prior_later.is_empty() {
            return Ok(());
        }
        let was = std::mem::take(&mut self.stats.solving);
        for (scc, rules) in strata {
            let bodies: Vec<Body<'_>> = rules.iter().map(|&r| vs.at(r)).collect();
            let lines: Vec<(&Body<'_>, Option<usize>)> = bodies.iter().flat_map(|b| {
                let rec = recursive(b, scc);
                let seed = rec.is_empty();
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
    pub fn first_sight(&self, b: &Body<'_>, first: Option<usize>) -> Option<PlanKey> {
        if plan_fresh() {
            return None;
        }
        let key = self.reused_key(b, first);
        matches!(plans_find(&key, b.len(), self.stats.nodes.max(1.0)), Some(None)).then_some(key)
    }

    /// Decision 0123, step 2: a round's requested statistics, made in one batch before any of its plans is made
    /// (step 3). Each is made as a kept relation's are (`resolve`): counted from its postings, or measured. This is
    /// where one batch goes to another maker (a classifier on the device) as a whole.
    pub fn make_statistics(&mut self, f: Fields<'_>, wanted: &BTreeSet<String>) -> Result<()> {
        let wanted: Vec<&str> = wanted.iter().map(String::as_str).filter(|r| self.deferred(r)).collect();
        if wanted.is_empty() {
            return Ok(());
        }
        run_counts().statistics[0].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        run_counts().statistics[1].fetch_add(wanted.len() as u64, std::sync::atomic::Ordering::Relaxed);
        let _span = super::super::profile::span(|| "statistics batch".into());
        super::super::profile::decision("statistics (decision 0123)", || format!("one batch of {} relations: {}", wanted.len(), wanted.join(", ")));
        self.resolve(f, wanted)
    }

    /// A line about to be planned whose statistics no round requested (its key moved once the round's batch was made,
    /// or it was not foreseen): they are made now, one line at a time, and counted (`RERAIL_PLAN_CACHE_COUNTS`:
    /// `late`), as what the rounds should have asked. `priced`: a line that is searched whatever was kept
    /// (`plan_rule_live`).
    pub fn late(&mut self, f: Fields<'_>, b: &Body<'_>, first: Option<usize>, priced: bool) -> Result<()> {
        let _span = super::super::profile::span(|| "late statistics".into());
        let rels: Vec<&str> = b.reads_statistics().into_iter().filter(|x| self.deferred(x)).collect();
        if rels.is_empty() || !priced && !self.needs_statistics(b, first) {
            return Ok(());
        }
        run_counts().statistics[2].fetch_add(rels.len() as u64, std::sync::atomic::Ordering::Relaxed);
        super::super::profile::decision("statistics (decision 0123)", || format!("made late, for one line: {}", rels.join(", ")));
        self.resolve(f, rels)
    }

    pub fn plan_rule_reused(
        &self,
        pf: Fields<'_>,
        b: &Body<'_>,
        first: Option<usize>,
    ) -> Result<(Plan, Option<super::super::profile::RulePlan>)> {
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
    ) -> Result<(Plan, Option<super::super::profile::RulePlan>)> {
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
            let _span = super::super::profile::span(|| "kept plan: its sizes".into());
            self.live_sizes_at(b, first, &misses)
        };
        if super::super::profile::on() {
            note_planned(key);
        }
        let remember = |key: &PlanKey, got: &(Plan, Option<super::super::profile::RulePlan>)| {
            plans_learnt(key, Learnt::Priced(Arc::new(Priced { sizes: sizes.clone(), got: got.clone() })));
        };
        // A rule whose line did less work than a search when it last ran (scaled by how much what it reads grew)
        // keeps its order as it is: neither searched nor priced (decision 0122).
        let budget = super::super::profile::span(|| "kept plan: the budget".into());
        if let Some(k) = &kept && let Some(got) = self.budget_takes(k, b, first, &sizes) {
            drop(budget);
            let _span = super::super::profile::span(|| "kept plan: taken, remembered".into());
            plan_cache(8);
            super::super::profile::decision("plan cache (decision 0038)", || format!("a kept plan taken as it is: its rule did {} rows of work when it last ran, less than a search costs (decision 0122)", k.5.unwrap_or(0.0)));
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
            run_counts().budget_miss[why].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if why == 1 && super::super::profile::on() {
                let again = planned_now(&key);
                super::super::profile::decision("planning budget missed: never ran (decision 0122)", || format!("planned by {}, {}", rerail_prof::code_rule_dsl::logic::profile::bound::<PlanSite>().0, if again { "planned before in this run" } else { "kept from an earlier run" }));
            }
            // What grew, by kind of size and how far (the profile's decisions).
            if why == 3 && super::super::profile::on() && let Some(previous) = &k.4 {
                let cards = 1 + usize::from(first.is_some()) + b.read_count();
                let atoms: Vec<&str> = (0..b.len()).filter_map(|i| b.atom(i)).map(|a| b.rel(a)).collect();
                let driver = self.valid_live_driver(&k.0, b, first);
                let (a, b) = (&previous.sizes[1..cards.min(previous.sizes.len())], &sizes[1..cards.min(sizes.len())]);
                let (at, g) = a.iter().zip(b).enumerate().map(|(i, (x, y))| (i, (y.max(1.0) / x.max(1.0)).max(1.0))).fold((0, 1.0), |m, (i, g)| if g > m.1 { (i, g) } else { m });
                let what = if first.is_some() && at == atoms.len() { "the delta".to_owned() } else { atoms.get(at).map_or("?".into(), |r| {
                    let r = *r;
                    if self.stats.solving.contains(r) { "a relation of the component".to_owned() } else if r.contains("__") { format!("a change ({})", r.rsplit_once("__").map_or("", |x| x.1)) } else { "a relation".to_owned() }
                }) };
                let from = a.get(at).copied().unwrap_or(0.0);
                let bucket = |x: f64| if x <= 1.0 { "≤1" } else if x <= 10.0 { "≤10" } else if x <= 100.0 { "≤100" } else if x <= 1e4 { "≤10k" } else { ">10k" };
                let product = if a.len() == b.len() { growth(a, b) } else { f64::INFINITY };
                super::super::profile::decision("planning budget missed: inputs grew (decision 0122)", || format!("{}product ×{}; most by {what}, from {} rows, ×{}; ran {}", if driver { "" } else { "kept order not led by its delta; " }, bucket(product), bucket(from), bucket(g), bucket(k.5.unwrap_or(0.0))));
            }
        }
        // A kept order priced under what a search costs, whose line did a search's work or more when it last ran
        // (decision 0123 §7): its price was wrong by enough that a search can pay, and costs less than the line ran.
        // So an order taken without a search (`greedy first`, where its price is under a search's) is held to what it
        // did. Until the line runs again, the new plan is not held to the old work (`ran` is cleared).
        let mispriced = kept.as_ref().is_some_and(|k| mispriced(k.1 * nodes, k.5, b, first));
        let kept = if mispriced {
            plan_cache(9);
            super::super::profile::decision("plan cache (decision 0038)", || "searched: a kept order priced under a search ran over a search's work (decision 0123 §7)".into());
            None
        } else {
            kept
        };
        // A kept price is a prediction, its line's last run what the order cost: a line that ran far over its kept
        // price (`REPLAN` times it, by over `REPLAN_ROWS` rows) is not reused at that price, however near its sizes
        // (corpus0, 2026-11-21: `open_tuple`'s seed kept at 753 rows, run at 41 M, reused at 753 by 7 strata).
        let ran_over = |priced: &Priced, ran: Option<f64>| ran.is_some_and(|ran| ran > REPLAN * priced.got.0.1 && ran - priced.got.0.1 > REPLAN_ROWS);
        if let Some((_, _, _, _, Some(priced), ran)) = &kept
            && near(&priced.sizes, &sizes)
            && !ran_over(priced, *ran)
            && self.valid_live_driver(&priced.got.0.0, b, first)
        {
            super::super::profile::decision("plan cache (decision 0038)", || format!("a kept plan reused at its kept price: no size it reads moved over {PRICE_DRIFT}x"));
            plan_cache(2);
            return Ok(priced.got.clone());
        }
        // Seen first: no kept plan, nor one dropped as mispriced (that is searched).
        let first_sight = kept.is_none() && !mispriced;
        if let Some((order, per_node, searched, gap, previous, _)) = kept {
            let priced = {
                let _span = super::super::profile::span(|| "re-price".into());
                // The search's EXPLAIN (its steps' rows as the search priced them), with this pricing's work, rows and
                // steps priced: not built again per chunk.
                self.plan_inner(pf, b, first, Some(&order), false).map(|(got, _)| {
                    let x = searched.map(|x| super::super::profile::RulePlan { work: got.1, rows: got.2, subsets: 0, candidates: order.len() as u64, ..x });
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
                    super::super::profile::decision("plan cache (decision 0038)", || "replanned: a start much cheaper than when the kept order was searched".into())
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
                        super::super::profile::decision("plan cache (decision 0038)", || "a kept plan reused, re-priced with this chunk's statistics".into());
                        plan_cache(3);
                        remember(&key, &got);
                        return Ok(got);
                    };
                    plan_cache(5);
                    #[derive(Default)]
                    struct CacheMissesShown(std::sync::atomic::AtomicU64);
                    if std::env::var_os("RERAIL_PLAN_CACHE_WHY").is_some() && nodes >= 1000.0 && super::super::profile::local::<CacheMissesShown>().0.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 12 {
                        eprintln!("replanned {} :- {} literals, first {first:?}: {why}: nodes {nodes}, work {} (kept per node {per_node} -> {}), sizes {:?}, kept sizes {:?}", b.show_head(), b.len(), got.0.1, per_node * nodes, sizes, previous.as_ref().map(|p| p.sizes.clone()));
                    }
                    super::super::profile::decision("plan cache (decision 0038)", || format!("replanned: {why}"))
                }
                Err(_) => {
                    plan_cache(6);
                    super::super::profile::decision("plan cache (decision 0038)", || "replanned: the kept order no longer runs".into())
                }
            }
        }
        // A line seen first starts from an order kept before, where a search could not pay (decision 0258). The
        // search tries it first, under its own set-up.
        let start = if first_sight && !pending { self.start_order(b, first) } else { None };
        let mut cx = joins::SearchCx { start: start.as_ref().map(|s| s.0.clone()), ..Default::default() };
        let got = {
            let _span = super::super::profile::span(|| "search".into());
            self.plan_inner_in(pf, b, first, None, true, &mut cx)?
        };
        if cx.started && let Some((_, how)) = start {
            plan_cache(10 + how);
            super::super::profile::decision("plan cache (decision 0038)", || format!("started from {} (decision 0258)", ["its line's order kept under another key", "its source rule's order"][how]));
            with_plans(|p| {
                if let Some(p) = p.as_ref() {
                    plans_put(p, key, (got.0.0.clone().into(), got.0.1 / nodes, None, got.0.1 / SEED.with(|c| c.get()).max(1.0), None, None));
                }
            });
            orders_put(line_key(b, first), &got.0.0);
            remember(&key, &got);
            return Ok(got);
        }
        if start.is_some() {
            super::super::profile::decision("source orders (decision 0258)", || "searched: the order does not run, is not led by its delta, or prices over a search's cost".into());
        }
        if first_sight {
            plan_cache(1);
            super::super::profile::decision("plan cache (decision 0038)", || "planned (first sight of the rule)".into());
        }
        let got = self.decide(pf, b, first, key, got, cx.runners_up, cx.steps_of, round.take());
        with_plans(|p| {
            if let Some(p) = p.as_ref() {
                let gap = got.0.1 / SEED.with(|c| c.get()).max(1.0);
                let ran = if mispriced { None } else { plans_get(&key).and_then(|k| k.5) };
                plans_put(p, key, (got.0.0.clone().into(), got.0.1 / nodes, got.1.clone(), gap, None, ran));
            }
        });
        // A lead-free line's order is not its delta-led line's (`line_key` is the body and the delta).
        if source_orders() && !self.stats.lead_free {
            orders_put(line_key(b, first), &got.0.0);
        }
        remember(&key, &got);
        Ok(got)
    }

    /// Decision 0354: a round's choice for a line planned in it (`plan_rule_reused_in`), kept under its key in place
    /// of the search's best, as `plan_rule_reused` keeps a line's plan: the line's next planning takes it.
    fn keep_choice(&self, pf: Fields<'_>, b: &Body<'_>, first: Option<usize>, key: PlanKey, order: &[Step]) -> Result<(Plan, Option<super::super::profile::RulePlan>)> {
        let got = self.plan_inner(pf, b, first, Some(order), true)?;
        let nodes = self.stats.nodes.max(1.0);
        with_plans(|p| {
            if let Some(p) = p.as_ref() {
                let gap = got.0.1 / SEED.with(|c| c.get()).max(1.0);
                let ran = plans_get(&key).and_then(|k| k.5);
                plans_put(p, key, (got.0.0.clone().into(), got.0.1 / nodes, got.1.clone(), gap, None, ran));
            }
        });
        // A lead-free line's order is not its delta-led line's (`line_key` is the body and the delta).
        if source_orders() && !self.stats.lead_free {
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
    pub fn plan_ahead(&mut self, pf: Fields<'_>, lines: &[(&Body<'_>, Option<usize>)]) -> Ahead {
        let mut ahead = Ahead::default();
        if !joins::pricing() || with_plans(|p| p.is_none()) || lines.is_empty() {
            return ahead;
        }
        let _span = super::super::profile::span(|| "join model: a stratum's plans in one round".into());
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
                    super::super::profile::decision("join choices (decision 0203)", || format!("{}: {}", b.head_text(), c.why));
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
        if !source_orders() || self.stats.lead_free {
            return None;
        }
        // Why a line is searched, not started (the profile's decisions).
        let not = |why: &str| {
            super::super::profile::decision("source orders (decision 0258)", || format!("searched: {why}"));
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
        let _span = super::super::profile::span(|| "key".into());
        super::super::derived::digest(|d| {
            d.str(kind);
            d.u64(b.key() as u64);
            d.u64((b.key() >> 64) as u64);
            d.u64(first.map_or(u64::MAX, |f| f as u64));
            // Its delta free to start anywhere: another line than the one its delta leads.
            if self.stats.lead_free {
                d.str("lead free");
            }
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

    /// Whether `order` starts from the line's driver: its delta, else a
    /// demand-restricted aggregate's demand (`Body::demand_first`); and a demanded relation's rule from a lead its
    /// demand bounds (`Body::demand_bounds_lead`), so a kept order led by a scan is searched again (Q59).
    pub fn valid_live_driver(&self, order: &[Step], b: &Body<'_>, first: Option<usize>) -> bool {
        if self.stats.lead_free {
            return true;
        }
        let driven = first.or_else(|| b.demand_first()).is_none_or(|f| order.first().is_some_and(|s| s.at == f));
        driven && (first.is_some() || order.first().is_none_or(|s| b.demand_bounds_lead(s.at)))
    }

    /// Records the work a rule's line did in the engine (`plan_work`, every order it ran in) on its kept plan, for
    /// the next run's planning budget (decision 0122).
    pub fn ran(key: PlanKey, work: f64) {
        plans_learnt(&key, Learnt::Ran(work));
    }

    /// Tests: `ran`, under the key `plan_rule_reused` keeps the rule's plan under now.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn ran_as_planned(&self, b: &Body<'_>, first: Option<usize>, work: f64) {
        Self::ran(self.plan_key("", b, first), work);
    }

    /// The key `plan_rule_reused` keeps a rule's plan under now (it reads the statistics: taken when planned).
    pub fn reused_key(&self, b: &Body<'_>, first: Option<usize>) -> PlanKey {
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
    ) -> Result<(Plan, Option<super::super::profile::RulePlan>)> {
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
    ) -> Result<(Plan, Option<super::super::profile::RulePlan>)> {
        if with_plans(|p| p.is_none()) {
            return self.plan_inner(pf, b, first, None, true);
        }
        // A line planned with the program (decision 0361 step 5): no key, no cache, no price. Delta-led: not a
        // lead-free line's.
        if !self.stats.lead_free && let Some(plan) = self.stats.prepared.take(line_key(b, first), self.stats.nodes, b, &self.stats.shapes) {
            self.log_assumed(line_key(b, first));
            super::super::profile::decision("prepared plans (decision 0361 step 5)", || format!("{}: a live plan taken as prepared, not searched or priced", b.rel(b.head())));
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
        let keep = |got: &(Plan, Option<super::super::profile::RulePlan>), sizes: Vec<f64>, searched: Option<f64>| {
            with_plans(|p| {
                if let Some(p) = p.as_ref() {
                    let then = plans_get(&key);
                    // The work its line did when it last ran (decision 0122), kept with the plan.
                    let ran = then.as_ref().and_then(|k| k.5);
                    // A search's order, at its work; or the kept order re-priced, as it was kept.
                    let entry: (Arc<[Step]>, f64, Option<super::super::profile::RulePlan>, f64) = match (searched, &then) {
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
                super::super::profile::decision("plan cache (decision 0038)", || format!("a live replan kept: no size it reads moved over {PRICE_DRIFT}x"));
                return Ok(priced.got.clone());
            }
            let alternate = plans_get(&earlier).and_then(|k| k.4);
            if let Some(alt) = alternate
                && !exact
                && near(&alt.sizes, &sizes)
                && self.valid_live_driver(&alt.got.0.0, b, first)
            {
                super::super::profile::decision("plan cache (decision 0038)", || format!("a live replan kept: the state before the last, no size moved over {PRICE_DRIFT}x"));
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
                let _span = super::super::profile::span(|| "re-price".into());
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
                super::super::profile::decision("live replans searched (decision 0038)", || format!("searched again: {why}"));
                #[derive(Default)]
                struct ReplansShown(std::sync::atomic::AtomicU64);
                if std::env::var_os("RERAIL_PLAN_CACHE_WHY").is_some() && super::super::profile::local::<ReplansShown>().0.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 40 {
                    let work = repriced.as_ref().map(|(p, _)| p.1).unwrap_or(-1.0);
                    eprintln!("live searched {} :- {} literals, first {first:?}: {why}: work {work} (searched {then}, a search {}), sizes {sizes:?}, kept sizes {:?}", b.show_head(), b.len(), search_rows(b, first), priced.sizes);
                }
            }
            if let Ok((plan, _)) = repriced
                && why.is_none()
            {
                super::super::profile::decision("plan cache (decision 0038)", || {
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
            super::super::profile::decision("live replans searched (decision 0038)", || "searched: no plan kept under its key".into());
        }
        // Without its EXPLAIN (item 2): the caller asks for it (`explain_order`) only when it takes a new order.
        let mut cx = joins::SearchCx::default();
        let got = {
            let _span = super::super::profile::span(|| "search".into());
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
    fn decide(&self, pf: Fields<'_>, b: &Body<'_>, first: Option<usize>, key: PlanKey, got: (Plan, Option<super::super::profile::RulePlan>), alternatives: Vec<(Vec<Step>, f64)>, steps_of: Vec<(Vec<Step>, Vec<super::super::profile::StepPlan>)>, round: Option<&mut Vec<joins::Choice>>) -> (Plan, Option<super::super::profile::RulePlan>) {
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
            let _span = super::super::profile::span(|| "join model: features".into());
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
                        made = super::super::profile::RulePlan { steps: st.clone(), ..Default::default() };
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
        super::super::profile::decision("join choices (decision 0203)", || format!("{}: {}", b.head_text(), choice.why));
        joins::log(|| format!("choice {}: {} ({} candidates)", b.head_text(), choice.why, choice.candidates.len()));
        self.plan_inner(pf, b, first, Some(&choice.candidates[choice.chosen].order), got.1.is_some()).unwrap_or(got)
    }

    /// Decision 0203 §5: a wider search of a line, best first: the exact search up to `joins::WIDE_GREEDY` joins
    /// with its runners-up, and random connected orders (`joins::random_orders`), each priced with the statistics
    /// as they are (the exact pair statistics among them).
    pub fn explore(&self, pf: Fields<'_>, b: &Body<'_>, first: Option<usize>) -> Vec<(Vec<Step>, f64)> {
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
    /// Decision 0361 step 5 with 0203: a line running the order prepared for it (planned with the program, from its
    /// layouts' estimates), checked as heavy (`joins::check`: past a search's work, then at each doubling), is searched
    /// from now on in this run: its key returned, for the caller to plan it again with the run's statistics. A prepared
    /// order that is only worse than another is priced right by those statistics and so is never slow (§3); this
    /// finds it at a search's price, once a line, where its work pays for one (decision 0122). The order the search
    /// gives is the one kept (`decide`), so a later run starts from it. Its assumed sizes against the run's, in the
    /// profile: the estimate and what it met.
    /// Whether a line running the order prepared for it has done `work` past `joins::slow_factor` times what that plan
    /// priced it at (at least a search's work, `SEARCH_WORK`: a search is never made for less than it costs). Checked
    /// at every chunk, as it costs a lookup: the prepared price is the plan's, not one worked out again.
    #[doc(hidden)]
    pub fn prepared_horribly_slow(&self, b: &Body<'_>, firsts: &[Option<usize>], running: &[Step], work: f64) -> bool {
        firsts.iter().filter_map(|f| self.stats.prepared.0.get(&line_key(b, *f))).find(|p| same_order(&p.order, running)).is_some_and(|p| {
            work > joins::slow_factor() * (p.work_per_node * self.stats.nodes.max(1.0)).max(SEARCH_WORK)
        })
    }

    /// Q290's measurement (`prepared::sizes_log`): a prepared plan taken for `line`, each relation it assumed a size of
    /// against what this model holds of it, rows per node. A relation still being solved is left out (its rows so far).
    fn log_assumed(&self, line: u128) {
        let Some(log) = prepared::sizes_log() else { return };
        use std::io::Write as _;
        let n = self.stats.nodes.max(1.0);
        let mut log = log.lock();
        for (r, per) in self.stats.prepared.assumed(line) {
            if !self.stats.solving.contains(r) {
                let _ = writeln!(log, "seen\t{r}\t{per}\t{}\t{n}", self.card(r) / n);
            }
        }
    }

    /// Decision 0361 step 6: a running line's rows step by step (`steps`, the executors' counts, decision 0203) against
    /// the facts its probes were priced by. A probe returning past `shapes::BROKEN_BY` times the most its relation's key
    /// or fan-out lets it, a probe on average, breaks that fact: the run notes it (`shapes::Broken`), names it in the
    /// profile (and on stderr in a strict run), and this model's shapes leave the relation out, as every later model's
    /// do (`Shapes::for_run`): planned from statistics for the rest of the run. Whether a fact broke here.
    #[doc(hidden)]
    pub fn layout_broken(&mut self, b: &Body<'_>, order: &[Step], steps: &[(u64, u64)]) -> bool {
        if self.stats.shapes.0.is_empty() || order.len() != steps.len() {
            return false;
        }
        let mut bound = VarSet::new();
        let mut broke = false;
        for (k, s) in order.iter().enumerate() {
            if k > 0
                && s.mode.is_none()
                && let Some(a) = b.atom(s.at).filter(|a| !a.neg())
            {
                let rel = b.rel(a);
                let cols: Vec<usize> = b.args(s.at).iter().enumerate().filter(|(_, t)| t.var().is_some_and(|v| bound.contains(v)) || b.constant(**t).is_some()).map(|(c, _)| c).collect();
                let (into, made) = steps[k];
                if let Some(fact) = self.stats.shapes.broken_at(rel, &cols, into, made) {
                    if shapes::run_broken().note(rel, fact.clone()) {
                        let text = format!("{rel}: {fact} broken: {into} probes made {made} rows, in {}; planned from statistics for the rest of the run", b.head_text());
                        if super::super::profile::strict() {
                            eprintln!("rerail: a layout fact broken (decision 0361 step 6): {text}");
                        }
                        super::super::profile::decision("broken layout facts (decision 0361 step 6)", || text);
                    }
                    broke = true;
                }
            }
            bound.extend(b.vars(s.at));
        }
        if broke {
            self.stats.shapes = shapes::Shapes::for_run(std::sync::Arc::clone(&self.stats.shapes));
        }
        broke
    }

    #[doc(hidden)]
    pub fn prepared_heavy(&self, b: &Body<'_>, firsts: &[Option<usize>], running: &[Step]) -> Option<u128> {
        let (line, p) = firsts.iter().map(|f| line_key(b, *f)).find_map(|k| self.stats.prepared.0.get(&k).filter(|p| same_order(&p.order, running)).map(|p| (k, p)))?;
        let fresh = with_plans(|s| s.is_some_and(|s| s.prepared_searched.lock().insert(line)));
        if !fresh {
            return None;
        }
        prepared::run_counts().searched.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        super::super::profile::decision("prepared plans (decision 0361 step 5)", || {
            let n = self.stats.nodes.max(1.0);
            let met: Vec<String> = p.assumed.iter().map(|(r, per)| format!("{r} assumed {:.0} rows, holds {:.0}", per * n, self.card(r))).collect();
            format!("{}: heavy on its prepared order, searched{}{}", b.head_text(), if met.is_empty() { "" } else { "; " }, met.join(", "))
        });
        Some(line)
    }

    /// The rows after each step of `order`, as the search prices it (`StepRows`): what a running line's counts are
    /// checked against (`joins::step_miss`), its widest step among them. A pricing, not a search.
    pub(in crate::code_rule_dsl) fn order_step_rows(&self, pf: Fields<'_>, b: &Body<'_>, first: Option<usize>, order: &[Step]) -> Option<Vec<f64>> {
        let explain = explaining(true);
        let step_rows = rerail_prof::code_rule_dsl::logic::profile::bind(StepRows(true));
        let got = self.plan_inner(pf, b, first, Some(order), true);
        drop(step_rows);
        drop(explain);
        got.ok().and_then(|(_, x)| x).map(|x| x.steps.iter().map(|s| s.rows).collect())
    }

    /// A running line's miss against its order's step rows (`joins::step_miss`), once it has done a few searches'
    /// work and is not flagged in that order. The step rows are priced once an order (`expected`), so a line too
    /// small to matter is never priced (decision 0122). `RERAIL_JOIN_MISSES=off` checks none (a measurement aid).
    #[allow(clippy::too_many_arguments)]
    pub fn line_miss(&self, pf: Fields<'_>, b: &Body<'_>, first: Option<usize>, order: &[Step], key: PlanKey, steps: &[(u64, u64)], expected: &mut Option<(Vec<Step>, Vec<f64>)>) -> Option<joins::Miss> {
        let work = super::super::profile::plan_work(steps) as f64;
        if !rerail_prof::code_rule_dsl::logic::profile::setting(|| !std::env::var("RERAIL_JOIN_MISSES").is_ok_and(|v| v == "off")) || work < joins::slow_factor() * SEARCH_WORK || positive_atoms(b) < 2 || joins::flagged(key, order) {
            return None;
        }
        if expected.as_ref().is_none_or(|(o, _)| !same_order(o, order)) {
            *expected = Some((order.to_vec(), self.order_step_rows(pf, b, first, order).unwrap_or_default()));
        }
        expected.as_ref().and_then(|(_, e)| joins::step_miss(e, steps, SEARCH_WORK))
    }

    /// A mispredicted line, step by step (`RERAIL_JOINS_LOG`, decision 0488's measure of what the counters do not
    /// price): each step's literal, the rule it was priced by, its rows priced, and the bindings in and rows out it
    /// made so far (`steps`).
    pub fn describe_miss(&self, pf: Fields<'_>, b: &Body<'_>, first: Option<usize>, order: &[Step], steps: &[(u64, u64)]) -> String {
        let explain = explaining(true);
        let got = self.explain_order(pf, b, first, order);
        drop(explain);
        let Some(x) = got else { return "no explain".into() };
        x.steps
            .iter()
            .enumerate()
            .map(|(k, st)| {
                let (i, o) = steps.get(k).copied().unwrap_or_default();
                format!("[{k}] {} by {} ({}): priced {:.1} rows, made {o} of {i} in", st.literal, st.access, if st.basis.is_empty() { "-" } else { &st.basis }, st.rows)
            })
            .chain((0..b.len()).filter_map(|i| b.plain_atom(i).map(|a| b.rel(a))).collect::<std::collections::BTreeSet<_>>().into_iter().map(|r| {
                let w = b.args((0..b.len()).find(|&i| b.plain_atom(i).is_some_and(|a| b.rel(a) == r)).unwrap()).len();
                let ndv: Vec<String> = (0..w).map(|c| self.ndv.at(r, c).map_or("-".into(), |n| format!("{n:.0}"))).collect();
                format!("stats {r}: {:.0} rows (card {:?}), ndv [{}]{}{}", self.card(r), self.stats.card.get(r), ndv.join(", "), if self.stats.solving.contains(r) { ", solving" } else { "" }, if self.stats.counts.at(r, 0).is_some() { ", counted" } else { "" })
            }))
            .collect::<Vec<_>>()
            .join(" | ")
    }

    pub fn plan_for_rows_in(&mut self, pf: Fields<'_>, b: &Body<'_>, at: usize, rows: f64, cx: &mut joins::SearchCx) -> Option<Vec<Step>> {
        let rel = b.atom(at).map(|a| b.rel(a).to_owned())?;
        let was = self.stats.delta_rows.insert(rel.clone(), rows.max(1.0));
        let got = self.plan_inner_in(pf, b, Some(at), None, false, cx).ok().map(|((order, _, _), _)| order);
        match was {
            Some(v) => self.stats.delta_rows.insert(rel, v),
            None => self.stats.delta_rows.remove(&rel),
        };
        got
    }

    pub fn price_for_delta(&mut self, pf: Fields<'_>, b: &Body<'_>, first: Option<usize>, order: &[Step], delta: f64) -> Option<f64> {
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
    pub fn explain_order(&self, pf: Fields<'_>, b: &Body<'_>, first: Option<usize>, order: &[Step]) -> Option<super::super::profile::RulePlan> {
        let _span = super::super::profile::span(|| "explain order".into());
        self.plan_inner(pf, b, first, Some(order), true).ok().and_then(|(_, x)| x)
    }

    /// `plan_search`, under `RERAIL_STATS_SKEW` (a measurement aid, decision 0096): planned with each column's
    /// fanout and distinct values skewed by a fixed factor in [1/f, f]; a search is also made with the statistics as
    /// they are, and the skewed order priced with them, so `RERAIL_STATS_SKEW_LOG` gets the regret: the skewed
    /// order's work over the best order's, by the true statistics.
    /// The join shape directives on `head`'s rules.
    fn shapes_for(&self, head: &str) -> Vec<&crate::code_rule_dsl::logic::front::Shape> {
        self.shapes.iter().filter(|s| s.head == head).collect()
    }

    /// The rule's join shape directives as waits of its joins (`.shape`, O-join-shape-directives): `(join, any of the
    /// joins it waits for, the variables it waits for, the directive's text)`. A join is held back until a literal of
    /// the relation it waits for is joined, or the variables it waits for are bound; the delta that drives the rule
    /// (`first`) never is. Each wait is on the subset joined and what it binds, as a gate's is, so the search stays
    /// exact within them. `joins`: the body's join literals, in order.
    fn shape_waits<'s>(&'s self, b: &Body<'_>, first: Option<usize>, joins: &[usize]) -> Vec<(usize, Vec<usize>, Vec<VarSlot>, &'s str)> {
        use crate::code_rule_dsl::logic::front::ShapeWait;
        let mut waits = Vec::new();
        for shape in self.shapes_for(b.rel(b.head())) {
            for (i, &bi) in joins.iter().enumerate() {
                if first == Some(bi) || !b.atom(bi).is_some_and(|a| b.rel(a) == shape.lit) {
                    continue;
                }
                match &shape.wait {
                    ShapeWait::After(x) => {
                        let any: Vec<usize> = joins.iter().enumerate().filter(|&(j, &bj)| j != i && b.atom(bj).is_some_and(|a| b.rel(a) == x)).map(|(j, _)| j).collect();
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
    pub fn plan_inner(
        &self,
        pf: Fields<'_>,
        b: &Body<'_>,
        first: Option<usize>,
        fixed: Option<&[Step]>,
        explain: bool,
    ) -> Result<(Plan, Option<super::super::profile::RulePlan>)> {
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
    ) -> Result<(Plan, Option<super::super::profile::RulePlan>)> {
        // A line planned with the program from its layouts (decision 0361 step 5): taken as kept, not searched.
        if fixed.is_none()
            && !self.stats.lead_free
            && let Some(plan) = self.stats.prepared.take(line_key(b, first), self.stats.nodes, b, &self.stats.shapes)
        {
            self.log_assumed(line_key(b, first));
            super::super::profile::decision("prepared plans (decision 0361 step 5)", || format!("{}: a line's plan taken as prepared, not searched", b.rel(b.head())));
            if std::env::var("RERAIL_PLAN_SHOW").is_ok_and(|r| r.split(',').any(|name| name == b.rel(b.head()))) {
                eprintln!("plan show {} prepared: work {:.1} rows {:.1}: {}", b.show_head(), plan.1, plan.2, b.shown(&plan.0).join(" , "));
            }
            let explained = if explain { self.explain_order(pf, b, first, &plan.0) } else { None };
            return Ok((plan, explained));
        }
        if self.shapes_for(b.rel(b.head())).is_empty() || rerail_prof::code_rule_dsl::logic::profile::bound::<NoShapes>().0 {
            return self.plan_unshaped(pf, b, first, fixed, explain, cx);
        }
        let free = |this: &Self, explain: bool, cx: &mut joins::SearchCx| {
            let _unshaped = rerail_prof::code_rule_dsl::logic::profile::bind(NoShapes(true));
            this.plan_unshaped(pf, b, first, None, explain, cx)
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
                    && super::super::profile::on()
                    && !used.is_empty()
                    && let Ok(own) = free(self, false, &mut joins::SearchCx { wide: cx.wide, ..Default::default() })
                {
                    let head = b.head_text();
                    if b.shown(&own.0.0) == b.shown(&got.0.0) {
                        super::super::profile::decision("a join shape directive no longer needed (O-join-shape-directives)", || format!("{head}: the planner's own order is the directive's ({})", used.join("; ")));
                    } else {
                        super::super::profile::decision("join shape directives (O-join-shape-directives)", || format!("{head}: est work {:.0}, the planner's own {:.0} ({})", got.0.1, own.0.1, used.join("; ")));
                    }
                }
                Ok(got)
            }
            // A kept order the directives do not admit is searched again by the caller.
            Err(e) if fixed.is_some() => Err(e),
            Err(e) => {
                let head = b.head_text();
                super::super::profile::decision("a join shape directive that cannot hold (O-join-shape-directives)", || format!("{head}: no order within its directives ({e}); planned without them"));
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
    ) -> Result<(Plan, Option<super::super::profile::RulePlan>)> {
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
        let skewing = rerail_prof::code_rule_dsl::logic::profile::bind(Skewing(true));
        let got = self.plan_search(pf, b, first, fixed, explain, cx);
        drop(skewing);
        let got = got?;
        if fixed.is_none()
            && std::env::var_os("RERAIL_STATS_SKEW_LOG").is_some()
            && let Ok((truth, _)) = self.plan_search(pf, b, first, None, false, &mut joins::SearchCx::default())
            && let Ok((priced, _)) = self.plan_search(pf, b, first, Some(&got.0.0), false, &mut joins::SearchCx::default())
        {
            use std::io::Write as _;
            // The run's own (opened once a run, decision 0442).
            let log = crate::code_rule_dsl::logic::run::setting(|| std::env::var_os("RERAIL_STATS_SKEW_LOG").and_then(|p| std::fs::OpenOptions::new().create(true).append(true).open(p).ok()).map(|f| std::sync::Arc::new(std::sync::Mutex::new(f))));
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
    ) -> Result<(Plan, Option<super::super::profile::RulePlan>)> {
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
        // with what is joined is a cross product (Q220: `p299_attempt`'s `kind($m, $k)` after `$k = #k1249` read
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
        // var → the subtree size of a part: `field($p, f, $c)` or `child($p, $c, i)` with $p's kind known and $c's
        // not. A part's subtree is at most its parent's, so `within($n, $c)` read from $c is priced at the parent's
        // size (t50 step 4: a function's body, `field($g, "body", $b)`, was priced as a node with children, 3 to 5
        // nodes, and the plan enumerated every function's body from it). A field's part is priced closer: the mean
        // size of the kinds that field's children have under a parent of that kind (`Fielded::part_size`). A
        // function's `body` is about the function; its `parameters` are a few nodes, and priced at the function's
        // size they tied with the function's every node by `nearest`, which the plan of `def_outside` then read.
        let mut part_of: HashMap<VarSlot, f64> = HashMap::new();
        for l in 0..b.len() {
            if let Some(a) = b.plain_atom(l)
                && !a.neg()
            {
                let args = b.args(l);
                let (p, c, f) = match (b.rel(a), args.len()) {
                    ("field", 3) => (args[0], args[2], b.constant(args[1])),
                    ("child", 3) => (args[0], args[1], None),
                    _ => continue,
                };
                if let (TermRef::Var(p), TermRef::Var(c)) = (p, c)
                    && !kinds.contains_key(&c)
                    && let Some(k) = kinds.get(&p)
                {
                    let field = f.map(|f| match f { Constant::Str(s) => s, Constant::Int(i) => i.to_string() });
                    let size = field.and_then(|f| self.stats.fields.part_size(k, &f, &self.stats.size)).or_else(|| self.stats.size.get(k).copied());
                    if let Some(size) = size {
                        part_of.insert(c, size);
                    }
                }
            }
        }
        // The same binding domains are used by many candidate prefixes. Read the existing statistics once
        // per search, with no input recount and no additional SQL.
        let binding_domains: Vec<_> = (0..b.len()).filter(|&i| b.plain_atom(i).is_some_and(|a| !a.neg()))
            .map(|i| (i, b.vars(i).clone(), self.binding_domains(b, i))).collect();
        let delta_misses = self.delta_misses(b, first);
        let delta_vars = first.and_then(|f| b.atom(f)).map(body::atom_vars);
        let delta_size = first.and_then(|f| b.atom(f)).and_then(|a| self.stats.delta_rows.get(b.rel(a))).copied();
        let starts = |i: usize| self.stats.lead_free || first.is_none_or(|f| f == i);
        // Which rule priced the atom last (`atom_rows`), and the containment share it applied: the explain record
        // names it per step, so a misprediction shows the rule that made it.
        let basis: std::cell::Cell<&'static str> = std::cell::Cell::new("");
        let basis_hit = std::cell::Cell::new(1.0f64);
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
            if rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var_os("RERAIL_PLAN_GREEDY_PROBE").is_some()) {
                // Bulk planning's sharing (a measurement aid): column tables made per search, against the relations
                // and (relation, constants) they are of.
                // The run's own (decision 0442).
                #[derive(Default)]
                struct Seen(parking_lot::Mutex<(u64, Option<std::collections::HashSet<String>>, Option<std::collections::HashSet<String>>)>);
                let seen = crate::code_rule_dsl::logic::run::local::<Seen>();
                let mut g = seen.0.lock();
                g.0 += 1;
                g.1.get_or_insert_default().insert(rel.to_string());
                let with_constants = format!("{}{:?}", rel, args.iter().map(|t| b.constant(*t)).collect::<Vec<_>>());
                g.2.get_or_insert_default().insert(with_constants);
                if g.0.is_power_of_two() || g.0.is_multiple_of(10_000) {
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
            // The columns whose postings count the bindings that miss (heavy values, a join's size): the share of
            // the bindings that hit is not taken of them again.
            let misses_counted = std::cell::Cell::new(0u64);
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
                    TermRef::Var(v) => kinds.get(&v).and_then(|k| stats.size.get(k)).copied().or_else(|| part_of.get(&v).copied()).or_else(|| stats.subtree_fan()),
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
                    // Decision 0209 §4: where this column and each placed atom's column holding `v` keep their heavy
                    // values, the binding is drawn value by value through those atoms, so a value common in all is
                    // drawn as often as it is. A fanout (sq / rows) is that for one atom placed; a three-way
                    // self-join on a skewed key wants the third moment (0.51 of its size from sq on the fixture).
                    // The placed atoms' columns that hold `v`, the bindings drawn through them; `all`: each draws
                    // `v` by its own rows. Its rows reach `v` weighted by their own counts only where no other of
                    // its variables joins another atom: one reached through another column (`kind($m, return)`,
                    // then `inf($f, $m)`) draws `v` by that join, a few returns a function whatever its size, which
                    // only measured pairs see (0203 §3).
                    let mut by_cols: Vec<(&str, usize)> = Vec::new();
                    let mut all = true;
                    for (d, vars, _) in &binding_domains {
                        if *d == i || !vars.is_subset(bound) { continue; }
                        let dr = b.rel(b.atom(*d).expect("an atom"));
                        let joined_else = b.args(*d).iter().any(|t| matches!(t, TermRef::Var(x) if *x != v
                            && binding_domains.iter().any(|(e, evars, _)| *e != *d && evars.contains(*x))));
                        if joined_else { all = false; }
                        by_cols.extend(b.args(*d).iter().enumerate().filter(|(_, t)| matches!(t, TermRef::Var(x) if *x == v)).map(|(dc, _)| (dr, dc)));
                    }
                    // Each column priced with its kept keys (decision 0488), read where the engine keeps them.
                    let priced = |r: &str, c: usize| self.heavy_of(r, c).map(|h| table::PricedCol::new(h, self.stats.keys.at(r, c)));
                    if all && !by_cols.is_empty() && let Some(here) = priced(rel, c)
                        && let Some(by) = by_cols.iter().map(|&(dr, dc)| priced(dr, dc)).collect::<Option<Vec<_>>>()
                        && let Some(f) = table::heavy_fan(here, &by)
                    {
                        basis.set("heavy values");
                        misses_counted.set(misses_counted.get() | 1 << c.min(63));
                        return f.max(0.01);
                    }
                    // Decision 0488: drawn through one atom's column, the rows the two columns join (their sketches'
                    // counters, `join_size`) over that column's rows are this column's rows a binding, the bindings
                    // that miss counted in. Only where the counters tell the join's size.
                    if all && let [(dr, dc)] = by_cols[..]
                        && let (Some(mine), Some(theirs)) = (self.stats.counts.at(rel, c), self.stats.counts.at(dr, dc))
                        && theirs.rows >= 1.0
                        && let joined = mine.join(theirs)
                        && joined.tells()
                    {
                        basis.set("join sketch");
                        misses_counted.set(misses_counted.get() | 1 << c.min(63));
                        return (joined.rows / theirs.rows).max(0.01);
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
            // Only where the placed atoms join the columns' variables (one reached from another), or a constant fixes one
            // (`pinned`: one value, no product): two variables bound by
            // atoms sharing none are a cross product's pairs, independent, and most pairs are none of the relation's.
            // `open_tuple`'s `child($t, $c, _)` after `kind($t, tuple)` and a scan of `unparen($c, $m)` was priced at a row
            // a pair (10.7 M) for 790, and the tail's witness pricing (Q146) took the cross product as nearly free:
            // 61 M rows, 19.7 s of ep2's cold run.
            let joined_by_placed = || {
                let vars: Vec<_> = cols.iter().filter_map(|&c| args[c].var()).collect();
                let Some(&start) = vars.first() else { return false };
                let mut reached = VarSet::new();
                reached.insert(start);
                let mut grew = true;
                while grew {
                    grew = false;
                    for (d, dvars, _) in &binding_domains {
                        if *d != i && dvars.is_subset(bound) && !dvars.is_disjoint(&reached) && !dvars.is_subset(&reached) {
                            reached.extend(dvars);
                            grew = true;
                        }
                    }
                }
                vars.iter().all(|v| reached.contains(*v) || pinned.contains(*v))
            };
            if cols.len() >= 2 && cols.len() < args.len() && cols.iter().all(|&c| b.constant(args[c]).is_none()) && card > 0.0 && joined_by_placed() {
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
            {
                let mut hit = 1.0f64;
                for &c in &cols {
                    let TermRef::Var(v) = args[c] else { continue };
                    if c == first && misses_counted.get() >> c.min(63) & 1 != 0 { continue }
                    // A column holds no more distinct values than its relation's rows: where its own are not measured,
                    // the rows measured bound them (a relation of the component being solved is still growing). Left
                    // out, a one-row finding probed by a moved line's node kept every binding: ep2's import edit
                    // ordered `p49_ASYNC100($v0, _)` after four old versions' probes of 4,672 rows, in each of 163
                    // rules' lines (`violation` maintained: 34.8 s; read after the delta, under 0.02 s).
                    let measured = || stats.card.get(rel).copied().filter(|_| !stats.solving.contains(rel)).map(|n| n.max(1.0));
                    let Some(here) = t.col(c, b, i, self).ndv.or_else(measured) else { continue };
                    let (from, at) = binding_domains
                        .iter()
                        .filter_map(|(d, vars, domains)| {
                            if *d == i || !vars.is_subset(bound) { return None; }
                            b.args(*d).iter().position(|t| matches!(t, TermRef::Var(x) if *x == v)).and_then(|c2| Some((domains[c2]?, (*d, c2))))
                        })
                        .fold((f64::INFINITY, None), |m, (n, at)| if n < m.0 { (n, Some(at)) } else { m });
                    // Where both columns' keys are kept (decision 0488), the bindings that hit are the share of the
                    // domain's keys this column holds too, whichever has more: two columns of disjoint values
                    // (a node's ids against another kind's) hit nothing, which distinct counts cannot tell.
                    let kept = at.and_then(|(d, c2)| {
                        let (mine, theirs) = (self.stats.keys.at(rel, c)?, self.stats.keys.at(b.rel(b.atom(d)?), c2)?);
                        let of = theirs.distinct();
                        (of >= 1.0).then(|| Some((mine.shared(theirs)? / of).clamp(0.5 / of, 1.0))).flatten()
                    });
                    if let Some(share) = kept {
                        hit = hit.min(share);
                    } else if from.is_finite() && from > here.max(1.0) {
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
                if rel.starts_with(super::super::strings::RELATION)
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
            // A delta read through its own index past the lead (`Stats::lead_free`): the relation's probe, at the
            // delta's share of its rows (measured). Led by its delta, a line reads the delta's rows at most.
            let share = (self.stats.lead_free && args.iter().any(|t| joins_with(*t, bound)))
                .then(|| delta_size.zip(stats.card.get(rel).copied().filter(|&c| c > 0.0)))
                .flatten();
            if first == Some(i) && let Some((n, card)) = share {
                rows = (rows * (n / card).min(1.0)).max(1e-6);
                basis.set("delta share");
            } else if first == Some(i)
                && let Some(n) = delta_size && n.max(0.01) < rows { rows = n.max(0.01); basis.set("delta size"); }
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
        // The work of an order whose tail (from the first join made with every head variable bound: `tail`, the work
        // and rows then) ends with `rows`: the tail's reads stop at a binding's first witness (`setfix::tail`, a fused
        // step too), so a binding with `rows / h` witnesses reads about that share of them. Every candidate a witness:
        // one read a step a binding (`fzk($d)`, then a document's nodes, 19 M rows read as 14,631, work queue Q240).
        // One in 15,000 a witness (`loaded` by a key every row holds, then `doc_module(p, m)`): read in full, as the
        // walk reads them (443 M rows).
        fn witnessed(raw: f64, tail: Option<(f64, f64)>, rows: f64) -> f64 {
            match tail {
                Some((base, h)) if rows > h && h > 0.0 => base + (raw - base) * h / rows,
                _ => raw,
            }
        }
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
        // A step as every order prices it: the rows it walks, past the head's binding too. The walk's stop at a
        // binding's first witness is the whole order's (`witnessed`, over its tail), not a step's: priced a step at a
        // time, as one read a binding, the greedy order read a document's every node as one where none of them
        // passed (decision 0431 R9's `exists_10($d) :- demand($d), node($d, $v, …), !placed($v), finding($v)`,
        // 118 rows of work priced, 100 k read a call), and took it unsearched.
        let step = |l: usize, rows: f64, bound: &VarSet| step_all(l, rows, bound);
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
        let waits = if rerail_prof::code_rule_dsl::logic::profile::bound::<NoShapes>().0 { Vec::new() } else { self.shape_waits(b, first, &joins) };
        let shaped: Vec<String> = { let mut t: Vec<String> = waits.iter().map(|w| w.3.to_owned()).collect(); t.dedup(); t };
        let shape_waits = |i: usize, placed: u64, bound: &dyn Fn(VarSlot) -> bool| -> bool {
            waits.iter().any(|(k, any, vars, _)| *k == i && ((!any.is_empty() && !any.iter().any(|j| placed & (1 << j) != 0)) || vars.iter().any(|v| !bound(*v))))
        };
        let mut candidates = 0u64;
        // A given order, priced now: each join step as the search prices it, each filter once ready. A kept order
        // (`fixed`), or a start the search tries first (`SearchCx::start`, decision 0258).
        let price = |fixed: &[Step], candidates: &mut u64| -> Result<(f64, f64, Vec<Step>)> {
            let (mut work, mut rows, mut bound, mut order) = (0.0, 1.0, pre.clone(), Vec::new());
            // The work and rows where the head's variables are all bound before a join (`witnessed`).
            let mut tail: Option<(f64, f64)> = None;
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
                    if tail.is_none() && head_vars.is_subset(&bound) {
                        tail = Some((work, rows));
                    }
                    let (w, out, mode) = step_all(l, rows, &bound).ok_or_else(|| Error::Plan("a kept step cannot run".into()))?;
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
                    order.push(*s);
                }
                bound.extend(b.vars(l));
            }
            Ok((witnessed(work, tail, rows), rows, order))
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
        // A gate (`answers::gate`'s `done_N_ad(t)` before `!N(t)` or an aggregate) holds only keys its rule's own literals give
        // (its `ask` is the body before the negation), so it never yields a row they would not: it is a test of a
        // bound key, joined once its key is bound and never crossed in to bind it. Its size is the size of what is
        // answered so far, which the stats of an earlier wave do not know. As the delta, it anchors as any atom.
        let gate_waits = |i: usize, bound: &dyn Fn(VarSlot) -> bool| -> bool {
            matches!(b.view(joins[i]), Lit::Atom(a) if super::super::answers::is_gate(b.rel(a)))
                && (0..b.len()).any(|l| matches!(b.view(l), Lit::Atom(n) if n.neg() && n.gated()) || matches!(b.view(l), Lit::Agg { atom: n, .. } if n.gated()))
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
        // The head's variables as bits, where each is a body variable numbered under 128 (`step`'s head-bound rule).
        let head_mask: Option<u128> = head_vars.iter().try_fold(0u128, |m, v| var_ids[v.index()].filter(|&k| k < 128).map(|k| m | 1u128 << k));
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
            // The work and rows where the head's variables are all bound before a join (`witnessed`), as `price`'s.
            let mut tail: Option<(f64, f64)> = None;
            while done != full_mask {
                let mut best: Option<(usize, f64, f64, Option<Mode>)> = None;
                for (i, &l) in joins.iter().enumerate() {
                    if done & (1 << i) != 0 || order.is_empty() && (!matches!(b.view(l), Lit::Atom(_)) || !starts(l)) {
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
                if tail.is_none() && head_vars.is_subset(&bound) {
                    tail = Some((work, rows));
                }
                order.push(Step { at: joins[i], mode });
                bound.extend(b.vars(joins[i]));
                let (closed, filtered) = close(&mut order, out, &mut bound, &mut rest);
                rows = closed;
                work += w + filtered;
                done |= 1 << i;
            }
            (done == full_mask).then_some((witnessed(work, tail, rows), rows, order, bound, rest))
        };
        // `RERAIL_PLAN_SHOW=<head>` (dev): what the planner knows of each atom where it orders the head's rules: the
        // search's, and a greedy order's taken without a search.
        let shown = std::env::var("RERAIL_PLAN_SHOW").is_ok_and(|r| r.split(',').any(|name| name == b.rel(b.head())));
        let show_atoms = |what: &str| {
            eprintln!("plan show {} {what} first {first:?} fixed {} no_size {no_size:b} no_rows {no_rows:b}", b.show_head(), fixed.is_some());
            for &j in &joins {
                let Some(a) = b.atom(j) else { continue };
                let rel = b.rel(a);
                let cols: Vec<String> = (0..b.args(j).len()).map(|c| format!("ndv {:?} fan {:?}", self.ndv.get(&(rel.to_owned(), c)), self.stats.fan.get(&(rel.to_owned(), c)))).collect();
                eprintln!(
                    "plan show   atom {}: derived {} held {} view {} card {:?} lazy {} solving {} prior_later {} col {:?} | {}",
                    b.show_atom(a),
                    self.derived.contains(rel),
                    self.stats.held.contains(rel),
                    self.view_src.contains_key(rel),
                    self.stats.card.get(rel),
                    self.stats.lazy.0.contains_key(rel),
                    self.stats.solving.contains(rel),
                    self.stats.prior_later.contains(rel),
                    self.stats.col.get(&(rel.to_owned(), sql_col(rel, 0))),
                    cols.join("; ")
                );
            }
        };
        let (probe, greedy_first) = rerail_prof::code_rule_dsl::logic::profile::setting(|| (std::env::var_os("RERAIL_PLAN_GREEDY_PROBE").is_some(), std::env::var("RERAIL_PLAN_GREEDY_FIRST").map_or(true, |v| v != "off")));
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
            // Priced as the walk runs it (`price`: `witnessed` over the tail past the bound head), as a start is, not
            // at `step`'s one witness a step: a tail whose rows hold no witness reads every one of them. ep2's hiding
            // bound, `fzk($d) :- demand($d), node($d, $s, …), guest_place(_, $s, …)`, must join `node` before the
            // unconnected `guest_place`; with no guest place, priced at a row a document (80), it read every node of
            // each (400,983 rows a chunk, 131 M in the run's 329 programs).
            .and_then(|g| {
                let (work, rows, _) = price(&g.2, &mut candidates).ok()?;
                Some((work, rows, g.2, g.3, g.4))
            })
            .filter(|g| g.0 <= search_rows(b, first));
        if started.is_none() {
            run_counts().searches[usize::from(cheap_greedy.is_some())].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        if let Some(g) = started {
            best_of.insert(full_mask, g);
        } else if let Some(g) = cheap_greedy {
            super::super::profile::decision("plan cache (decision 0038)", || "greedy first: its work under a search's".into());
            if shown {
                show_atoms("greedy first");
                eprintln!("plan show   greedy work {:.1} rows {:.1}: {}", g.0, g.1, b.shown(&g.2).join(" , "));
            }
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
            /// Its work as each step reads (`work` is that past the head's binding scaled by `witnessed`), and,
            /// once the head is bound, the work and rows at that point: the tail (`witnessed`).
            raw: f64,
            tail: Option<(f64, f64)>,
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
        // The share of a join's rows a range read leaves (`setfix::ranges`): a positive atom of a relation rules make,
        // not this component, probed by a bound key, whose new variable later comparisons bound by values bound
        // already, reads only the rows between them, in the order on its key then that column (`index_then`). The
        // comparisons still pass their share on as filters; this prices the rows read. `bcgp_line($p, $l, _)` probed by
        // `$p` with `$l > $a, $l < $b` reads a pair's lines between its decorator and its def, not its file's.
        let range_pass = |i: usize, bound: u128| -> f64 {
            let j = joins[i];
            let Lit::Atom(a) = b.view(j) else { return 1.0 };
            let rel = b.rel(a);
            // Rows a rule made here, or an input the run holds (`Stats::held`): rows with an order to range over. A
            // view of a source reads its own indexes.
            if a.neg() || jmask[i] & bound == 0 || !(self.derived.contains(rel) || self.stats.held.contains(rel)) || self.view_src.contains_key(rel) || self.stats.solving.contains(rel) {
                return 1.0;
            }
            let mut new = jmask[i] & !bound;
            let mut best = 1.0f64;
            while new != 0 {
                let v = new.trailing_zeros();
                new &= new - 1;
                let mut share = 1.0;
                for (k, &f) in filters.iter().enumerate() {
                    if matches!(b.view(f), Lit::Cmp { op: CmpOp::Lt | CmpOp::Le | CmpOp::Gt | CmpOp::Ge, .. }) && lit_bits[f] & !bound == 1u128 << v {
                        share *= pass[k];
                    }
                }
                best = best.min(share);
            }
            best
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
        cells[0] = Some(Cell { work: work0, rows: rows0, prev: 0, last: usize::MAX, mode: None, unfounded: false, fan: 1.0, basis: "", raw: work0, tail: None });
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
                    let w = w * range_pass(i, *bound);
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
                    // Past the head's binding, priced as the walk runs it (`witnessed`), as a fixed order is (`price`);
                    // the tail starts at the first join made with every head variable bound before it. The walk passes
                    // on one witness a binding (`setfix::tail`), and a step reads a binding's rows until its first:
                    // all of them where none passes, so `rr_agg15($a, $b, $p) :- rr_ctx16($a, $b, $p),
                    // bcgp_line($p, $l, _), !bcgp_at_line($p, $l), $l > $a, $l < $b` led from the pairs (64.8 K)
                    // read every line of each pair's file (28.6 s against 5.4 s led from the lines). The rows here are
                    // every row walked, as the fixed order's are: a second cut of the rows passed on to the bindings
                    // (Q245 item 5, made apart from this one) left the share of witnesses always all, and Q240's
                    // documents no longer led (1 M rows where 1,200 come, Q411).
                    let raw = from.raw + w + filtered;
                    // A cross product in the tail is not discounted by its witnesses: its rows are not witnesses (no row
                    // of what it follows vouched for them), and a binding with none reads all of them. It is paid in
                    // full, and the tail starts again after it, its rows the bindings. Discounted, RUF007's climb seed
                    // in main's pack crossed `child($p12, $c11, _)` and `child($p8, $c7, _)` after the head was bound
                    // (71 K bindings, 2·10¹⁰ crossed rows, priced as 4,194) and ran past 90 s on one file.
                    let head_bound = head_mask.is_some_and(|h| *bound & h == h);
                    let tail = if crossed && (from.tail.is_some() || head_bound) {
                        Some((raw, out))
                    } else {
                        from.tail.or_else(|| head_bound.then_some((from.raw, left)))
                    };
                    let cand = Cell { work: witnessed(raw, tail, out), rows: out, prev, last: i, mode, unfounded, fan, basis: by, raw, tail };
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
        if shown {
            show_atoms("search");
            for (m, c) in cells.iter().enumerate().filter_map(|(m, c)| c.as_ref().map(|c| (m as u64, c))) {
                if m.count_ones() <= 5 || m == full_mask {
                    let lits: Vec<String> = (0..joins.len()).filter(|&i| m >> i & 1 == 1).map(|i| b.atom(joins[i]).map_or_else(|| format!("#{}", joins[i]), |a| b.show_atom(a))).collect();
                    eprintln!("plan show   {m:b} last {} work {:.1} rows {:.1} unfounded {}: {}", c.last, c.work, c.rows, c.unfounded, lits.join(" , "));
                }
            }
        }
        let explain = std::env::var("RDL_EXPLAIN").is_ok_and(|r| r.split(',').any(|name| name == b.rel(b.head())));
        for mask in (0..cells.len() as u64).filter(|&m| cells[m as usize].is_some()) {
            if (mask == full_mask || (explain && mask.count_ones() == 2))
                && let Some(b) = rebuild(mask) {
                    best_of.insert(mask, b);
                }
        }
        subsets_reached = cells.iter().filter(|c| c.is_some()).count() as u64;
        // Decision 0354: an order's steps as this search priced them, for the learned model's features: each join's
        // rows a row in and basis from its cell (`last`: a runner-up's final join, which is not its subset's cell),
        // a filter passing its share; access and source as the plan record names them, their first words.
        let source_of = |rel: &str| -> &'static str {
            if rel.starts_with(super::super::strings::RELATION) {
                "text test"
            } else if self.is_input(pf, rel) {
                "input"
            } else if self.view_src.contains_key(rel) {
                "view of an input"
            } else {
                "derived"
            }
        };
        let planned = |order: &[Step], last: Option<(usize, f64, &'static str)>| -> Option<Vec<super::super::profile::StepPlan>> {
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
                out.push(super::super::profile::StepPlan { literal: String::new(), access: access.to_owned(), source: source.to_owned(), rows: r, all: r, passed: Vec::new(), basis: by.to_owned() });
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
            super::super::profile::decision("greedy against the search (RERAIL_PLAN_GREEDY_PROBE)", || format!("{} joins: {bucket}", joins.len().min(9)));
            // Greedy first: its order taken where its own work is under what the search costs, so a better order
            // could save less than the search would cost.
            let cost = SEARCH_FIXED_ROWS + candidates as f64;
            let taken = g.as_ref().is_some_and(|g| g.0 <= cost);
            // The run's own (decision 0442).
            #[derive(Default)]
            struct Sums(parking_lot::Mutex<[f64; 8]>);
            let sums = crate::code_rule_dsl::logic::run::local::<Sums>();
            let mut s = sums.0.lock();
            let at = if taken { 0 } else { 4 };
            s[at] += 1.0;
            s[at + 1] += candidates as f64;
            s[at + 2] += *gc as f64;
            s[at + 3] += if same { 0.0 } else { regret.clamp(0.0, 1e12) };
            let n = (s[0] + s[4]) as u64;
            if n.is_power_of_two() || n.is_multiple_of(1000) {
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
        super::super::profile::work(super::super::profile::Unit::Candidate, candidates);
        // Also when the run may use the device: placement prices each join step from these (decision 0095); and when
        // the body reads a kind-set axis, which takes its expected lookups from them (decision 0143).
        let axis = || (0..b.len()).any(|l| matches!(b.view(l), Lit::Atom(a) if !a.neg() && super::super::front::is_transduced(b.rel(a))));
        let explained = (explain && (super::super::profile::on() || placement::device_on() || rerail_prof::code_rule_dsl::logic::profile::bound::<ExplainAlways>().0 || axis())).then(|| {
            let _explain = super::super::profile::span(|| "explain record".into());
            // The chosen order step by step, priced as the search priced it.
            let mut bound = VarSet::new();
            let mut r = 1.0;
            let mut steps = Vec::new();
            let mut placed: BTreeSet<usize> = BTreeSet::new();
            // Each step's rows alone (`plan_rule_step_rows`): as below, with nothing made for EXPLAIN to show.
            if rerail_prof::code_rule_dsl::logic::profile::bound::<StepRows>().0 {
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
                    steps.push(super::super::profile::StepPlan { literal: String::new(), access: String::new(), source: String::new(), rows: r, all: r, passed: Vec::new(), basis: String::new() });
                }
                return super::super::profile::RulePlan { work, rows, subsets, candidates, steps, shaped: shaped.clone() };
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
                    if rel.starts_with(super::super::strings::RELATION) {
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
                steps.push(super::super::profile::StepPlan { literal: b.show(s.at), access, source: src, rows: r, all: r_all.max(r), passed, basis });
            }
            super::super::profile::RulePlan { work, rows, subsets, candidates, steps, shaped: shaped.clone() }
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
        self.stats.card.insert(rel, card);
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
pub fn first_columns(args: &[TermRef<'_>]) -> HashMap<VarSlot, usize> {
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
    super::super::derived::digest(|d| {
        d.u64(v.digest());
        d.u64(v.rules().count() as u64);
        v.rules().for_each(|r| d.u64(r.hash()));
    })
}

/// `ProgramAnalysis::analyze`'s result for `prog`, kept for the programs analysed last: a session's runs plan the
/// same programs again (its file-local part every run, a maintained part while the same strata change), and the
/// analysis is a function of what the digest reads (each relation's columns and lattice keys, the inputs, the
/// relations with facts, the natives, and the rules in order).
fn program_analysis(prog: &Version) -> std::sync::Arc<super::super::analysis::ProgramAnalysis> {
    type Kept = Vec<(u128, std::sync::Arc<super::super::analysis::ProgramAnalysis>)>;
    /// The programs analysed last, newest at the end: a few, as each holds a result per rule.
    const KEEP: usize = 4;
    #[derive(Default)]
    struct KeptAnalyses(std::sync::Mutex<Kept>);
    let key = program_digest(prog);
    let held_ = crate::code_rule_dsl::logic::profile::shared::<KeptAnalyses>();
    let mut kept = held_.0.lock().unwrap();
    if let Some(at) = kept.iter().position(|(k, _)| *k == key) {
        let hit = kept.remove(at);
        let analysis = std::sync::Arc::clone(&hit.1);
        kept.push(hit);
        super::super::profile::decision("program analysis", || "kept: the same program analysed earlier".into());
        return analysis;
    }
    drop(kept);
    super::super::profile::decision("program analysis", || "analysed".into());
    let analysis = std::sync::Arc::new(super::super::analysis::ProgramAnalysis::analyze(prog));
    let held_ = crate::code_rule_dsl::logic::profile::shared::<KeptAnalyses>();
    let mut kept = held_.0.lock().unwrap();
    if kept.len() >= KEEP {
        kept.remove(0);
    }
    kept.push((key, std::sync::Arc::clone(&analysis)));
    analysis
}

/// An input's cardinality, distinct counts and fanouts, from its measured columns: `width` its declared columns
/// (`None`, not declared).
pub fn input_stats(stats: &mut Stats, ndv: &mut table::ColMap<f64>, width: Option<usize>, rel: &str) -> Result<()> {
    let width = width.ok_or_else(|| Error::Statistics(format!("the input {rel} is not declared")))?;
    for col in 0..width {
        let name = sql_col(rel, col);
        let Some(&[rows, n, sq, _]) = stats.col.get(&(rel.to_owned(), name.clone())) else {
            return Err(Error::Statistics(format!("no statistics for {rel}.{name}")));
        };
        stats.card.insert(rel, rows as f64);
        ndv.insert((rel.to_owned(), col), if n == 0 { 1.0 } else { n as f64 });
        stats.fan.insert((rel.to_owned(), col), if rows != 0 { sq as f64 / rows as f64 } else { 1.0 });
    }
    Ok(())
}

/// The program's join shape directives (`.shape`), by text, as the model reads them.
pub fn shapes_of(f: &fields::Fields<'_>) -> std::sync::Arc<Vec<super::super::front::Shape>> {
    std::sync::Arc::new(f.shapes().iter().map(|s| super::super::front::Shape {
        head: f.text(s.head).to_owned(),
        lit: f.text(s.lit).to_owned(),
        wait: match &s.wait {
            super::super::program_image::ShapeWaitRef::After(a) => super::super::front::ShapeWait::After(f.text(*a).to_owned()),
            super::super::program_image::ShapeWaitRef::Bound(vs) => super::super::front::ShapeWait::Bound(vs.iter().map(|&a| f.text(a).to_owned()).collect()),
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
    if super::super::profile::on()
        && let Ok(layouts) = super::super::program_image::manager::analysis::<super::super::program_image::layouts::Layouts>(&p.v)
    {
        for line in layouts.show(&p.v).lines() {
            super::super::profile::decision("relation layouts (decision 0361)", || line.to_owned());
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
    // Without the relations whose facts the run's rows broke (decision 0361 step 6).
    stats.shapes = shapes::Shapes::for_run(shapes::Shapes::of_version(&p.v));
    stats.prepared = prepared::Prepared::of_version(&p.v);
    let mut ndv = ColMap::default();
    for (rel, (_, rows)) in &inputs.batch {
        let rows = rows.max(1.0);
        stats.card.insert(rel.clone(), rows);
        // A base relation bound as a batch source is its vtab, which answers T's indexes' probes; any other
        // batch source is scanned.
        Arc::make_mut(&mut stats.indexes).insert(rel.clone(), vtab_indexes(rel));
        let names = super::super::sql::select::input_columns(rel);
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
                super::super::program_image::Cell::Int(i) => Constant::Int(i),
                super::super::program_image::Cell::Str(a) => Constant::Str(f.text(a).to_owned()),
                // A kind, field or name (decision 0270): its atom, as `constant` reads a branded term.
                super::super::program_image::Cell::Branded(a) => Constant::Int(a.value()),
            }).collect())
            .collect();
        let n = rows.len() as f64;
        stats.card.insert(rel, n);
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
pub fn vtab_indexes(rel: &str) -> Vec<Vec<String>> {
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
        "intval" | "op" | "op_t" | "tape" | "line" | "end_line" | "col" | "docof" | "visit_index" | "ast_depth" | "spread" | "rr_long_text_node" => &[&["n"]],
        "node_order" => &[&["n"], &["k"]],
        "first_token" | "last_token" => &[&["n"]],
        "token_before" | "token_after" => &[&["x"]],
        "end_point" => &[&["n"], &["p"]],
        "next_sibling" => &[&["a"], &["b"]],
        "str_part" | "str_part_start" | "str_part_word" => &[&["lit"]],
        "core_node" => &[&["n"], &["t"], &["d"]],
        // Containment, made per node looked up either way (`facts::vtab::syntax::within`, decision 0050).
        "within" => &[&["n"], &["a"]],
        "span_within" => &[&["inner"], &["outer"]],
        "doc" | "pkg" | "unpackaged" | "tape_error" | "syntax_error" | "source" | "line_count" => &[&["d"]],
        "src_line" => &[&["d", "l"], &["sp"]],
        "op_at" | "prev_token" => &[&["n"], &["s", "n"]],
        "token" | "rr_token_anon" => &[&["p", "i"], &["start"]],
        "rr_token_named" => &[&["p", "i"], &["c"], &["start"]],
        "path_part" | "file_name" => &[&["d"], &["s", "d"]],
        // The transducer's `nearest` rows (`facts::vtab::syntax::with_nearest`), looked up either way.
        rel if crate::code_rule_dsl::logic::front::is_transduced(rel) => &[&["n"], &["a"]],
        // A text test's symbols (decision 0037, `preparation::text::TextRelations`).
        rel if rel.starts_with(super::super::strings::RELATION) => &[&["s"]],
        // A `match_all` regex's matches, by span and by match (`facts::vtab::syntax::Extra`).
        rel if rel.starts_with(super::super::strings::MATCHES) => &[&["sp"], &["m"]],
        // A `match_group` regex's groups, by span, by match and by group span.
        rel if rel.starts_with(super::super::strings::GROUPS) => &[&["sp"], &["m"], &["gsp"]],
        _ => &[],
    };
    lists.iter().map(|l| l.iter().map(|c| (*c).to_owned()).collect()).collect()
}

// ---- SQL backend -------------------------------------------------------------------------------------------------
//
// A rule body is built as a typed query (`sql::select`), checked against the program's relations and the
// columns in scope, then rendered (list step 20).

pub fn col(alias: &str, column: &str) -> Expr {
    Expr::Qualified { alias: alias.to_owned(), column: column.to_owned() }
}
pub fn bin(operator: Op, left: Expr, right: Expr) -> Expr {
    Expr::Binary { operator, left: Box::new(left), right: Box::new(right) }
}
pub fn eq(left: Expr, right: Expr) -> Expr {
    bin(Op::Equal, left, right)
}
pub fn call(name: &'static str, arguments: Vec<Expr>) -> Expr {
    Expr::Call { name, arguments }
}
pub fn length(e: &Expr) -> Expr {
    call("length", vec![e.clone()])
}
pub fn substr(arguments: Vec<Expr>) -> Expr {
    call("substr", arguments)
}
/// A symbol's text: `sym_text` of it in atoms mode.
pub fn text_of(atoms: Option<[i64; 3]>, e: &Expr) -> Expr {
    if atoms.is_some() { call("sym_text", vec![e.clone()]) } else { e.clone() }
}
/// A text's symbol: `sym_atom` of it in atoms mode.
pub fn symbol_of(atoms: Option<[i64; 3]>, e: Expr) -> Expr {
    if atoms.is_some() { call("sym_atom", vec![e]) } else { e }
}

/// A kind set's members as SQL values: kind atoms, or kind names.
pub fn kinds(members: &[String]) -> Vec<Expr> {
    members.iter().map(|k| k.parse::<i64>().map_or_else(|_| string(k), integer)).collect()
}

impl Inputs {
    /// A symbol expression as text: `sym_text` of it in atoms mode.
    pub fn text_of(&self, e: &Expr) -> Expr {
        if self.atoms.is_some() { call("sym_text", vec![e.clone()]) } else { e.clone() }
    }
    /// `sym_atom` of a text expression in atoms mode.
    pub fn symbol_of(&self, e: Expr) -> Expr {
        if self.atoms.is_some() { call("sym_atom", vec![e]) } else { e }
    }
}

impl Model {
    /// Where a rule reads `rel`: its batch source, the facts' input, or the derived table (or its delta).
    pub fn relation(&self, pf: Fields<'_>, rel: &str, delta: bool) -> Relation {
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
    pub fn declared(&self, pf: Fields<'_>, relation: &Relation) -> bool {
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
/// Rule `rule` (of version `v`) as a view of an input's columns (Q37), the input bound or not: its body one positive
/// atom of an input (`input`), no constants and each variable once, its head distinct variables of it, and its rows
/// the input's, one each: the head keeps the input's key (`node`'s node, `doc`'s and `tape`'s first column) or every
/// column. The input, and the atom's column of each head column. A run holds such a relation in place
/// (`Run::input_view`); a session neither keeps nor counts it run (`retain::cut`).
pub fn input_view_shape(v: &super::super::program_image::Version, rule: super::super::program_image::RuleRef<'_>, input: impl Fn(&str) -> bool) -> Option<(String, Vec<usize>)> {
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
    /// Each column's keys, where the native engine keeps them (decision 0488), else none.
    pub keys: Vec<super::super::bloom::Keys>,
    /// Each column's counts for join sizes, where the native engine keeps them (`join_size`), else none.
    pub counts: Vec<std::sync::Arc<super::super::join_size::JoinCounts>>,
}

impl LiveRel {
    /// The native engine's (`setfix::live`): its values are integers.
    pub fn of_native(l: super::super::native::component::Live) -> LiveRel {
        LiveRel { n: l.n, delta: l.delta, ndv: l.ndv, sq: l.sq, freq: l.freq.into_iter().map(|(c, v)| (c, v.into_iter().map(|(v, m)| (Constant::Int(v), m)).collect())).collect(), keys: l.keys, counts: l.counts }
    }
    /// From T's extension's text line (`set_fixpoint_stats`); `text(c)`: column `c` holds symbols as text.
    pub fn parse(line: &str, text: &dyn Fn(usize) -> bool) -> LiveRel {
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
        LiveRel { n, delta: n, ndv: rest[..k].to_vec(), sq: rest[k..].to_vec(), freq, keys: Vec::new(), counts: Vec::new() }
    }
}

/// Decision 0354: the samples of `order`'s steps after its first, from its plan record `x` and the counts it ran with
/// (`None` where the record is not of this order). A step's bound key is its relation's most distinct values among
/// the columns bound when it reads. Without `counts`, each step's features for a prediction (the model's choice):
/// every step after the first, its rows in and out the statistics'.
/// Decision 0354: a stratum's plans made ahead of its loop (`Model::plan_ahead`), by line (`line_id`).
pub type Ahead = HashMap<(usize, Option<usize>), (Plan, Option<super::super::profile::RulePlan>)>;

/// A line of a stratum's loop: its body (where it lives for the loop) and its delta.
pub fn line_id(b: &Body<'_>, first: Option<usize>) -> (usize, Option<usize>) {
    (std::ptr::from_ref(b) as usize, first)
}

pub fn step_samples(model: &Model, b: &Body<'_>, order: &[Step], x: &super::super::profile::RulePlan, counts: Option<&[(u64, u64)]>, seed: bool, origin: &str) -> Option<Vec<super::super::costs::join_steps::StepSample>> {
    use super::super::costs::join_steps::{StepSample, category};
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
            let key_ndv = rel.map_or(0.0, |r| bound_cols.iter().filter_map(|&c| model.ndv.at(r, c).copied()).fold(0.0, f64::max));
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

/// `prior_stats`, its statistics made now. Deferred (`fill`), they fill only what nothing set since, as a later
/// write overrides them when they are made at once.
pub fn prior_now(model: &mut Model, f: Fields<'_>, r: &str, fill: bool) -> Option<f64> {
    let width = f.arity(r)?;
    model.resolve(f, [r]).ok()?;
    let cols = (0..width).map(|c| model.stats.col.get(&(r.to_owned(), sql_col(r, c))).copied()).collect::<Option<Vec<[i64; 4]>>>()?;
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
    model.stats.prior_later.remove(r);
    let mut changed = Vec::new();
    let card = (live.n as f64).max(1.0);
    if model.stats.card.insert(r, card) != Some(card) { changed.push(0); }
    let mut freq: HashMap<_, _> = live.freq.into_iter().collect();
    // Its keys change with its rows, as its distinct values do: a change of them alone replans nothing.
    for (c, keys) in live.keys.into_iter().enumerate().filter(|_| super::super::bloom::kept_keys_read()) {
        model.stats.keys.insert((r.to_owned(), c), keys);
    }
    for (c, counts) in live.counts.into_iter().enumerate().filter(|_| super::super::bloom::kept_keys_read()) {
        model.stats.counts.insert((r.to_owned(), c), counts);
    }
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

/// Whether two orders of one body are the same: each step's position and mode (what `shown` compared as text,
/// without writing either out).
pub fn same_order(a: &[Step], b: &[Step]) -> bool {
    a == b
}

/// The component a stratum's plans are made as solving (`Stats::solve`, in their keys): itself when a rule reads it,
/// else none. Its seeds planned ahead (`seed_specs`, `Model::wants_strata`) are keyed as the stratum keys them.
pub fn solving_of(vs: &body::Versions, scc: &BTreeSet<String>, rules: &[RuleKey]) -> Option<BTreeSet<String>> {
    rules.iter().any(|&r| {
        let (v, rule) = vs.rule_at(r);
        rule.body().any(|l| matches!(l.view(), Lit::Atom(a) if !a.neg() && scc.contains(v.rel_name(a.rel()))))
    }).then(|| scc.clone())
}

/// The positive atoms of `scc` in `b` (its recursive atoms): each one's place in the body and its relation.
pub fn recursive<'v>(b: &Body<'v>, scc: &BTreeSet<String>) -> Vec<(usize, &'v str)> {
    (0..b.len())
        .filter_map(|i| b.plain_atom(i).filter(|a| !a.neg()).map(|a| (i, b.rel(a))))
        .filter(|(_, rel)| scc.contains(*rel))
        .collect()
}

/// The most distinct values a column may have for its whole value set to be read from the statistics
/// (`Model::small_values`): the corpus's totals keep every value of such a column, and of others only those tested.
pub const SMALL_NDV: i64 = 64;

pub fn by_shift(db: &Connection) -> bool {
    left_on(db).lock().by_shift
}

/// The DP's guard against a cross product priced from a default size (decision 0137): step `i` shares no variable
/// with what `prev` joined (`bound`), a connected step remains, and `i` or such a step has no size of its own
/// (`no_size`, by join). `jmask`: each join's variables as bits.
pub fn unfounded_cross(i: usize, prev: u64, bound: u128, jmask: &[u128], no_size: u64) -> bool {
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
pub struct Readiness {
    own: Vec<u128>,
    tables: Vec<Option<Vec<u8>>>,
    wide: rustc_hash::FxHashMap<(usize, u128), bool>,
}

impl Readiness {
    const WIDE: u32 = 12;

    /// For literals whose variables are `own` (as bits).
    pub fn new(own: &[u128]) -> Readiness {
        Readiness { own: own.to_vec(), tables: vec![None; own.len()], wide: Default::default() }
    }

    /// Whether literal `f` is ready with `bound` bound: `test` of its own variables bound, run once per pattern.
    pub fn ready(&mut self, f: usize, bound: u128, test: impl FnOnce(u128) -> bool) -> bool {
        let own = self.own[f];
        let pattern = bound & own;
        if own.count_ones() > Self::WIDE {
            return *self.wide.entry((f, pattern)).or_insert_with(|| test(pattern));
        }
        // The pattern's bits packed in the order of the literal's variables.
        let (mut at, mut rest, mut k) = (0usize, own, 0);
        while rest != 0 {
            let low = rest.isolate_lowest_one();
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
pub fn source_order(b: &Body<'_>, src: &Body<'_>, map: &[Option<usize>], order: &[Step], first: Option<usize>) -> Option<Vec<Step>> {
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

/// What the runs on a connection left with it, so the next run there asks SQL nothing it knows (decision 0104: no SQL
/// in a run's waves that nothing needs). On the connection (decision 0442, `vtab::ConnData`), dropped as it closes.
#[derive(Clone, Copy, Default)]
pub struct Left {
    /// The last run left tables or views in SQL: the next run asks which, and drops them.
    pub temp: bool,
    /// `temp_store` is set to memory.
    pub configured: bool,
    /// The documents of its runs are node-id intervals by their keys (`docs_by_shift`).
    pub by_shift: bool,
}

/// What the runs on `db` left with it (`Left`).
pub fn left_on(db: &Connection) -> std::sync::Arc<parking_lot::Mutex<Left>> {
    crate::code_rule_dsl::facts::vtab::connection_data(db).get::<parking_lot::Mutex<Left>>()
}

