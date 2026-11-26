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
//! Rule bodies are typed queries (`sql::select`, over `sql_ast`'s expressions), checked before they are rendered; the wave keeps its planned program as a compiled plan (`stats::plans`, `waves::matches`). Left out of the
//! port: T's measurement switches (`RDL_PLAN0`, `RDL_SQL_REPLAN`, `RDL_CLUSTER=0`, `RDL_WHOLE=0`, `RDL_REPLAN`, traces
//! and profiles, with `timed_plan`, T's planning timer) and its 1 GB page caches (over the 500 MB budget; SQLite's
//! defaults instead).

use rerail_blocks::vector::Vector;
use crate::code_rule_dsl::facts::vtab::syntax::PythonBatch as _;
use super::front::Value;
use super::program_image::{AtomRef, CmpOp, Lit, Slot as VarSlot, TermRef, Version};
use body::{Body, RuleKey, VarSet};
use fields::Fields;
use super::native::TOP;
use super::native::component::spec::{Arg, ComponentSpec, EdgeSpec, Extreme, HeadTok, InputKey, JoinItem, Join as PlanJoin, JoinSpec, Line as SpecLine, Reprobe, Source as JoinSource, Start, Tok};
use std::sync::Arc;
use indexmap::IndexMap;
use rusqlite::Connection;
use rusqlite::types::Value as SqlValue;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;
use std::path::Path;
use super::sql::select::{Cte, FromItem, Join, Query, Relation, Select, Source};
use super::sql::sql_ast::{SqlBinaryOperator as Op, SqlExpression as Expr, SqlUnaryOperator, SqlValue as Value_};

// cratify: the planner's base is rerail-plan-base's (its facade of `plan` mounts the vocabulary, `base`, and the
// children the search reads: the forms, joins, outputs, placement, tables and emission), and its search is
// rerail-plan-search's (its facade mounts `search`, with `choices`, `demand`, `keeps`, `prepared`, `replanning` and
// `select`, the passes that read the model, and `kept_entry`, the kept facts' entries they and the corpus keep); the run re-exports both here.
pub use rerail_plan_search::code_rule_dsl::logic::plan::*;
// cratify: `corpus` is rerail-logic's (it mounts it: the corpus runner reads the language, above the planner)
pub mod join_spec;
pub mod compose;
pub mod emit;
pub mod issue;
/// A source's rows where they lie, digested and lent to a keep (`SourceRows`), the component's and the corpus's.
pub mod source_rows;
pub mod sql_kernel;
/// SPIKE (throw-away): W's algorithms as the executors of the reference library's strata.
pub mod spike_strata;
pub mod spike_eval;

/// Plans kept across chunks on this thread while held (a corpus run), none kept at the start: a run's plans from
/// earlier runs are the store's (`compiled_plans`' kept facts), a session's runs' as a new process's (t33 step 3).
pub struct KeepPlans(Option<rerail_prof::code_rule_dsl::logic::profile::Bound>);

impl KeepPlans {
    pub fn start() -> Self {
        compose::clear();
        let plans = rerail_prof::code_rule_dsl::logic::profile::bind(KeptPlans(Some(Box::new(SharedPlans { run: Arc::new(RunPlans::new(Default::default(), Default::default(), Default::default())), view: None }))));
        keeps::observing(true);
        KeepPlans(Some(plans))
    }
}

impl Drop for KeepPlans {
    fn drop(&mut self) {
        if std::env::var_os("RERAIL_PLAN_CACHE_COUNTS").is_some() {
            let counts: Vec<String> = run_counts().plan_cache.iter().zip(PLAN_CACHE_NAMES).map(|(c, n)| format!("{n} {}", c.swap(0, std::sync::atomic::Ordering::Relaxed))).collect();
            let miss: Vec<String> = run_counts().budget_miss.iter().zip(["no price", "never ran", "ran over a search", "inputs grew"]).map(|(c, n)| format!("{n} {}", c.swap(0, std::sync::atomic::Ordering::Relaxed))).collect();
            let st: Vector<u64> = run_counts().statistics.iter().map(|c| c.swap(0, std::sync::atomic::Ordering::Relaxed)).collect();
            let searches: Vector<u64> = run_counts().searches.iter().map(|c| c.swap(0, std::sync::atomic::Ordering::Relaxed)).collect();
            let lazy = lazy_probes_counted();
            eprintln!("plan cache: {}; budget missed: {}; statistics: {} batches of {} relations, {} late, {} answered by layouts; {} lines prepared, {} searched once heavy ({} versions' plans read, {} made); mispredicted: {} blown up, {} over-estimated; orders: {} searched, {} greedy (under a search's cost); probes: {} left unplanned at open, {} planned later", counts.join(", "), miss.join(", "), st[0], st[1], st[2], run_counts().shaped.swap(0, std::sync::atomic::Ordering::Relaxed), prepared::run_counts().taken.swap(0, std::sync::atomic::Ordering::Relaxed), prepared::run_counts().searched.swap(0, std::sync::atomic::Ordering::Relaxed), prepared::run_counts().versions[0].swap(0, std::sync::atomic::Ordering::Relaxed), prepared::run_counts().versions[1].swap(0, std::sync::atomic::Ordering::Relaxed), joins::run_counts().misses[0].swap(0, std::sync::atomic::Ordering::Relaxed), joins::run_counts().misses[1].swap(0, std::sync::atomic::Ordering::Relaxed), searches[0], searches[1], lazy.0, lazy.1);
        }
        // The layout facts the run's rows broke (decision 0361 step 6), each named in the profile as it broke.
        if std::env::var_os("RERAIL_PLAN_CACHE_COUNTS").is_some() {
            let b = shapes::run_broken();
            let broken = b.facts();
            eprintln!("layout facts: {} probe steps checked; broken: {}", b.1.swap(0, std::sync::atomic::Ordering::Relaxed), if broken.is_empty() { "none".to_owned() } else { broken.iter().map(|(r, f)| format!("{r} {f}")).collect::<Vec<_>>().join(", ") });
        }
        compose::clear();
        drop(self.0.take());
        keeps::observing(false);
        let published = run_counts().published.iter().map(|c| c.swap(0, std::sync::atomic::Ordering::Relaxed)).collect::<Vec<_>>();
        let folds = keeps::run_counts().folds.swap(0, std::sync::atomic::Ordering::Relaxed);
        let deferred = super::native::setfix::run_counts().deferred_walks.iter().map(|c| c.swap(0, std::sync::atomic::Ordering::Relaxed)).collect::<Vec<_>>();
        super::profile::decision("a walk's new texts interned at once (decision 0315)", || format!("{} walks in one piece interned {} new texts, one write of the arena each", deferred[0], deferred[1]));
        super::profile::decision("kept plans shared by RCU (decision 0301)", || format!("{} publications of {} new plans or orders; observed sums folded {folds} times", published[0], published[1]));
    }
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
    pub nodes: BTreeMap<String, Vector<usize>>,
    /// Outputs held as resident facts (`native::derived`), including decoded composed outputs kept for sessions.
    pub kept: BTreeSet<String>,
    /// The kept relations handed to the caller during the run (`run_atoms_native_handing`), no longer held.
    pub handed: BTreeSet<String>,
    /// The run's own record (decision 0441: what it met of its limits, the trees it decoded), where it was the
    /// outermost run; a run nested in another leaves it to that run.
    pub run: Option<super::run::Record>,
    /// The relations recomputed in their readers (decision 0099): no table and no engine rows hold them.
    pub inlined: BTreeSet<String>,
    /// A maintained run's strata derived again, each by its key, with the class of its reason (`maintain::reason_class`).
    /// With `maintained` and `quiet`, each stratum the run walked is counted once (0364 R9).
    pub again: Vec<(String, &'static str)>,
    /// A maintained run's strata the maintain mode ran, and of them the recursive.
    pub maintained: (usize, usize),
    /// A maintained run's walk: the strata a changed read reached, those skipped (no read changed: each relation its
    /// kept one), and all it walked.
    pub walked: (usize, usize, usize),
    /// A maintained run's relations whose rows changed by a delta (decision 0117 step 4): the rows gone and added.
    pub deltas: HashMap<String, Delta>,
    /// A maintained run's relations whose rows are as stored and were never read (decision 0117 step 4): no reached
    /// stratum read them. They are not held; their frames stand.
    pub unread: BTreeSet<String>,
    /// A maintained run's relations whose rows are as they were (decision 0117 step 4): held as kept (no read of
    /// them moved), or maintained to an empty delta. Their kept frames are not written again.
    pub unchanged: BTreeSet<String>,
    /// The strata passed on as quiet (decision 0101 §1), and the relations they held with no column measured
    /// (`QuietTally`).
    pub quiet: (usize, usize),
    /// A maintained run's strata a changed read reached that no rule of saw a row of its delta, by kind or key
    /// (decision 0563 B), by their keys (`stratum_key`): passed on as kept, and counted with the skipped (`walked`).
    pub unseen: Vec<String>,
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
    fn open(&self, spec: &super::native::component::spec::Spec) -> Result<()> {
        let _span = super::profile::span(|| "engine open (load inputs, build indexes)".into());
        if self.direct {
            super::native::component::open_spec(self.db, spec)?;
        } else {
            self.db.query_row("SELECT set_fixpoint_open(?1)", [super::native::component::spec::spec_text(spec)], |_| Ok(()))?;
        }
        Ok(())
    }
    fn run(&self, name: &str, waves: u32) -> Result<String> {
        let _span = super::profile::span(|| "engine run (joins)".into());
        Ok(if self.direct { super::native::component::run(self.db, name, waves)? } else { self.db.query_row("SELECT set_fixpoint_run(?1, ?2)", rusqlite::params![name, waves], |r| r.get(0))? })
    }
    /// The declared keys a second row broke since the last call (decision 0361 step 6): each relation's place and the
    /// key's columns. T's extension engine checks none.
    fn key_breaks(&self, name: &str) -> Result<Vec<(usize, Vector<usize>)>> {
        if self.direct { Ok(super::native::component::key_breaks(self.db, name)?) } else { Ok(Vec::new()) }
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
    fn step_cpu(&self, name: &str) -> Result<Vec<Vector<f64>>> {
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
                .map(LiveRel::of_native)
                .collect());
        }
        let text: String = self.db.query_row("SELECT set_fixpoint_stats(?1)", [name], |r| r.get(0))?;
        Ok(text.split('\n').enumerate().map(|(i, line)| LiveRel::parse(line, &|c| symbols_text(i, c))).collect())
    }
    /// `stats`, of only the relations whose rows moved since `seen` (`live_changed`): `None` for the others. Through
    /// SQL, every relation's.
    fn stats_changed(&self, name: &str, seen: &mut Vector<[usize; 3]>, symbols_text: &dyn Fn(usize, usize) -> bool) -> Result<Vec<Option<LiveRel>>> {
        if !self.direct {
            return Ok(self.stats(name, symbols_text)?.into_iter().map(Some).collect());
        }
        let _span = super::profile::span(|| "engine live statistics".into());
        // `RERAIL_LIVE_ALL=on`: every relation's, each time (to compare).
        if rerail_prof::code_rule_dsl::logic::profile::setting(|| matches!(std::env::var("RERAIL_LIVE_ALL").as_deref(), Ok("on"))) {
            seen.clear();
        }
        Ok(super::native::component::live_changed(self.db, name, seen)?
            .into_iter()
            .map(|l| l.map(LiveRel::of_native))
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
    Asking(Box<Driving>, Vector<u64>),
}

/// A native stratum between its component's open and its close: what its driver (`Run::drive`) needs to run the
/// component's chunks of waves, replan between them and record what it did. Held while the stratum waits on its
/// device wave.
struct Driving {
    scc: super::front::CompAt,
    solving: bool,
    /// Its component's column sources (`column_sources`), set again when it resumes.
    sources: Sources,
    name: String,
    spec: super::native::component::spec::Spec,
    rels: Vec<String>,
    ri: BTreeMap<String, usize>,
    views: HashMap<String, (String, String)>,
    sql_views: bool,
    sql_probe: Vec<bool>,
    /// Each seed's order, with its rule (a seed seeded by SQL has none).
    orders: Vec<(body::RuleKey, Vec<Step>)>,
    probes: Vec<(body::RuleKey, usize, Vec<Step>)>,
    order_rules: Vector<u64>,
    nseeds: usize,
    seeds: Vec<body::RuleKey>,
    ngates: usize,
    explained: Vec<(String, Option<usize>, Option<super::profile::RulePlan>, f64, u64)>,
    counted: Vec<Vec<(u64, u64)>>,
    /// Per probe line, the orders it ran in before a replan changed it, each with its own step counts: `counted`
    /// holds the current order's alone, so a step's counts are never another order's step at the same place.
    earlier: Vec<Vec<(Vec<Step>, Vec<(u64, u64)>)>>,
    timed: Vector<f64>,
    /// Profiled: per probe line, its current order's CPU seconds per step and last its head's rows made, over the
    /// walks `counted` counts (both start again when a replan changes the order).
    stepped: Vec<Vector<f64>>,
    chunk: i64,
    /// Relation/column versions and their dependent probe plans.
    replanned_at: replanning::Watch,
    /// Per probe, the searches made for it in this run: its work pays for each before the next (decision 0122).
    searched: Vector<u32>,
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
    /// Per line, its running order's rows after each step as priced, the widest among them (`Model::line_miss`).
    expected: Vec<Option<(Vec<Step>, Vector<f64>)>>,
    /// Per line, the seconds its probes ran (the engine's per-probe wall time), for the measured critical path
    /// (decision 0208).
    walled: Vector<f64>,
    /// Per probe, its whole plan's order (O168).
    wholes: HashMap<usize, Vec<Step>>,
}

pub struct Run<'a, 'b> {
    /// The run's connection; only its scope in a stratum run off the run's thread (`strand`, decision 0113).
    db: crate::code_rule_dsl::facts::vtab::Db<'a>,
    model: Model,
    views: Arc<HashSet<String>>,
    made: HashSet<(String, Vector<usize>)>,
    /// Each derived table's secondary indexes, as column orders (`index_for`).
    orders: HashMap<String, Vec<Vector<usize>>>,
    /// Derived tables with no key: views materialized as plain tables (`ensure_indexes`).
    keyless: HashSet<String>,
    /// Relations held only by the native set engine (`program`) until `materialize`.
    native_rels: BTreeSet<String>,
    /// SPIKE: the session's tree where it holds no syntax batch (`spike_strata::Tree::bare`): the relations
    /// `spike_eval` holds for its strata, its strands' too.
    spike_bare: Arc<std::sync::OnceLock<Arc<spike_strata::Tree>>>,
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
    answered: BTreeMap<String, Vec<Vector<i64>>>,
    /// The stratum being evaluated whose answers are logged (`logging`), and each such stratum's answers by produced
    /// relation (`answer_logs`): a native step's answers are among what its stratum read, kept with its frames.
    logging: Option<usize>,
    answer_logs: HashMap<usize, BTreeMap<String, BTreeSet<Vector<i64>>>>,
    /// The run's quiet strata, shared with its strands (`QuietTally`).
    quiet: Arc<QuietTally>,
    /// The static check of the program the run's part is of (decision 0364 §5): given by the caller
    /// (`Maintain::proofs`), else made of the run's own version when first asked.
    proofs: std::sync::OnceLock<Arc<wellfounded::Proofs>>,
}

/// The runner's answer to requests (`Run::answer`): a request relation and its new rows (atoms and integers), to the
/// rows each produced relation gains.
/// A request's answerer: given a request relation's rows (read where they lie, `component::Asked`), the rows each
/// produced relation gains.
pub type Answer<'a> = &'a mut dyn FnMut(&str, &super::native::component::Asked) -> Result<Vec<(String, Vec<Vector<i64>>)>>;

/// A complete request relation has no pending recursive work before its consumer. Resume held keys at once,
/// grouped by request family, as a quiet recursive component does. Never publish the protocol rows as facts.
/// (Ported from Data-layer-rule-expressivity d659df1c4: held-back keys of a complete request were lost before its
/// non-recursive consumer ran.)
fn answer_complete(answer: Answer<'_>, request: &str, keys: super::native::component::Asked) -> Result<BTreeMap<String, Vec<Vector<i64>>>> {
    use super::native::component::Asked;
    let mut pending = BTreeMap::from([(request.to_owned(), keys)]);
    let mut produced: BTreeMap<String, Vec<Vector<i64>>> = BTreeMap::new();
    while !pending.is_empty() {
        let mut held: BTreeMap<String, Vec<Vector<i64>>> = BTreeMap::new();
        for (request, keys) in pending {
            for (rel, rows) in answer(&request, &keys)? {
                if let Some(request) = rel.strip_prefix(super::answers::HELD_BACK) {
                    if !rows.is_empty() { held.entry(request.to_owned()).or_default().extend(rows); }
                } else { produced.entry(rel).or_default().extend(rows); }
            }
        }
        // The rows an answer held back, asked again: rows the answers made.
        pending = held.into_iter().map(|(r, rows)| (r, Asked::rows(rows))).collect();
    }
    Ok(produced)
}

// cratify: `complete_answer_tests` is rerail-logic's (it mounts it: it reads the language, `rr::parse_version`, or the corpus)

/// The runner's answers to requests, without a wait (decision 0114 §3): a request is posted and its answer is an
/// event, taken when it has come.
pub trait Asking {
    /// Posts a request relation's new rows: the ticket its answer comes back with.
    fn post(&mut self, rel: &str, rows: &super::native::component::Asked) -> Result<u64>;
    /// The answers that have come, each with its ticket; when none has, waits up to `wait` for one. The rows
    /// each produced relation gains, as `Answer` gives them.
    fn arrived(&mut self, wait: std::time::Duration) -> Result<Vec<(u64, Vec<(String, Vec<Vector<i64>>)>)>>;
}

fn answer_pending(db: &Connection, direct: bool, answer: &mut Option<Answer<'_>>, answered: &mut BTreeMap<String, Vec<Vector<i64>>>, log: Option<&mut BTreeMap<String, BTreeSet<Vector<i64>>>>, name: &str, scc: &super::front::Comp) -> Result<()> {
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
        run_counts().answer_wait.fetch_add(t0.elapsed().as_nanos() as u64, std::sync::atomic::Ordering::Relaxed);
        run_counts().answer_calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    add_answers(db, answered, log, name, scc, given)
}

/// Adds a component's answers to its produced relations; rows for a produced relation outside it are kept for the
/// stratum that holds that relation.
fn add_answers(db: &Connection, answered: &mut BTreeMap<String, Vec<Vector<i64>>>, log: Option<&mut BTreeMap<String, BTreeSet<Vector<i64>>>>, name: &str, scc: &super::front::Comp, given: Vec<(String, Vec<Vector<i64>>)>) -> Result<()> {
    let mut by: BTreeMap<String, Vec<Vector<i64>>> = BTreeMap::new();
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

/// A changed read's versions as a stratum reads them: a row gone and a row added that agree on every column it reads
/// (`used`, its source's) are no change to it, and neither is in its delta. Each is a set of rows picked where they lie
/// in the versions (0364 R2). `None` when no pair agrees. A line inserted moves the `start` and `end` of every node
/// after it alone: ep2's one-line edit gave `node` 1,637 rows added and 1,633 gone, and a stratum that read nodes'
/// kinds saw each as a change, past the threshold (0090 §4).
///
/// Each version's rows are found by a hash of their read columns, each row's columns read once: a row agrees with
/// the other version's rows of its hash, compared where they lie. Sorted by a comparison that read both rows'
/// columns through the source at each step, the data layer's 27,000-row `node` and `visit_index` versions took 0.4 s
/// of an edit at 334 files (t43 step 21).
pub(crate) fn unchanged_as_read(minus: &Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, plus: &Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, used: &[usize]) -> Option<[Arc<dyn crate::code_rule_dsl::facts::vtab::Source>; 2]> {
    use crate::code_rule_dsl::facts::vtab::Source;
    use std::hash::Hasher;
    let hash = |s: &dyn Source, r: usize| {
        let mut h = rustc_hash::FxHasher::default();
        for &c in used {
            h.write_i64(s.value(r, c));
        }
        h.finish()
    };
    let agree = |s: &dyn Source, r: usize, t: &dyn Source, x: usize| used.iter().all(|&c| s.value(r, c) == t.value(x, c));
    // Each version's rows by hash: the rows of the other version that may agree with one.
    let by_hash = |s: &dyn Source| {
        let mut by: rustc_hash::FxHashMap<u64, Vec<u32>> = rustc_hash::FxHashMap::default();
        let hashes: Vec<u64> = (0..s.len()).map(|r| hash(s, r)).collect();
        for (r, &h) in hashes.iter().enumerate() {
            by.entry(h).or_default().push(r as u32);
        }
        (by, hashes)
    };
    let ((by_minus, minus_hashes), (by_plus, plus_hashes)) = (by_hash(minus.as_ref()), by_hash(plus.as_ref()));
    // The rows of `s` whose columns read agree with no row of `t`.
    let alone = |s: &dyn Source, hashes: &[u64], t: &dyn Source, by: &rustc_hash::FxHashMap<u64, Vec<u32>>| -> Vector<u32> {
        (0..s.len()).filter(|&r| !by.get(&hashes[r]).is_some_and(|xs| xs.iter().any(|&x| agree(s, r, t, x as usize)))).map(|r| r as u32).collect()
    };
    let m = alone(minus.as_ref(), &minus_hashes, plus.as_ref(), &by_plus);
    if m.len() == minus.len() {
        return None;
    }
    let p = alone(plus.as_ref(), &plus_hashes, minus.as_ref(), &by_minus);
    let picked = |s: &Arc<dyn Source>, ids| super::native::component::InRel::picked(Arc::clone(s), ids, s.columns());
    Some([picked(minus, m), picked(plus, p)])
}

/// How many strata the process held as views of an input's columns, for tests and profiles.
pub fn input_views() -> u64 {
    run_counts().input_views.load(std::sync::atomic::Ordering::Relaxed)
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
                && let Some(rerail_section::Tuple2(base, cols)) = self.model.view_src.get(b.rel(a))
            {
                let base = base.as_str();
                // A view over an input is probed through the input's indexes. If none serves this probe, SQLite
                // builds an automatic index per statement, which for a filtered view scans every node, every round.
                // Materialize it once instead; it then gets indexes like any derived relation.
                let probed: HashSet<&str> = args.iter().enumerate().filter(|(_, t)| probed_col(t, &bound)).filter_map(|(c, _)| cols[c].as_ref().map(|c| c.as_str())).collect();
                let served = self.model.stats.indexes.get(base).into_iter().flatten().any(|ix| ix.first().is_some_and(|c| probed.contains(c.as_str())));
                if !probed.is_empty() && !served {
                    let r = b.rel(a);
                    let q = crate::code_rule_dsl::logic::sql::ident(r);
                    self.db.execute_batch(&format!(
                        "CREATE TEMP TABLE {r}__m AS SELECT * FROM {q}; DROP VIEW {q}; ALTER TABLE {r}__m RENAME TO {q};"
                    ))?;
                    Arc::make_mut(&mut self.views).remove(r);
                    self.model.view_src = self.model.view_src.filtered(|k, _| k != r);
                    self.keyless.insert(r.to_string());
                }
            }
            if let Some(a) = a
                && !self.inputs.contains(b.rel(a))
                && !self.views.contains(b.rel(a))
            {
                let cols: Vector<usize> = (0..args.len()).filter(|&c| probed_col(&args[c], &bound)).collect();
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
        let mut seen: HashSet<Vector<i64>> = HashSet::with_capacity(ids.len());
        for id in ids {
            seen.insert(kept.iter().map(|&c| src.value(id as usize, c)).collect());
        }
        Some(seen.len() as i64)
    }
    /// The view `r` (`projections`) on its bound input's columns: what a reader after the run reads in place of SQL's.
    fn unmade_view(&self, r: &str) -> Option<crate::code_rule_dsl::facts::vtab::UnmadeView> {
        let eqs = self.projections.get(r)?;
        let rerail_section::Tuple2(base, cols) = self.model.view_src.get(r)?;
        let base = base.as_str();
        let src = crate::code_rule_dsl::facts::vtab::bound_source(self.db, base)?;
        let names = src.columns();
        let at = |c: &str| names.iter().position(|n| n == c);
        let eqs: Vec<(usize, i64)> = eqs.iter().map(|&(c, v)| Some((at(&self.model.col(base, c))?, v))).collect::<Option<_>>()?;
        let cols: Vector<usize> = cols.iter().map(|c| at(c.as_ref()?.as_str())).collect::<Option<_>>()?;
        Some(crate::code_rule_dsl::facts::vtab::UnmadeView { base: base.to_owned(), cols, eqs })
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
    fn native_relation(&mut self, rel: &str, native: &super::program_image::ArchivedNativeRef) -> Result<usize> {
        use super::program_image::{ArchivedNativeSpec, NativeSpec};
        let vs = Arc::clone(&self.versions);
        let f = vs.whole(self.version);
        let reads: Vec<String> = native.reads.iter().map(|&r| f.rel_name(r).to_owned()).collect();
        self.ensure(&reads)?;
        self.materialize(&reads)?;
        let (edge, target, roots) = match &native.spec {
            ArchivedNativeSpec::DfsFirst { edge, target, roots } => (f.rel_name(*edge), f.rel_name(*target), roots.map(|r| f.rel_name(r))),
            ArchivedNativeSpec::MatchAll { input, pattern, groups, positions } => {
                let groups: Vec<String> = groups.iter().map(|&g| f.text(g).to_owned()).collect();
                return self.match_all(rel, f.rel_name(*input), f.text(*pattern), &groups, *positions);
            }
            ArchivedNativeSpec::TextTest { input, column, op, patterns } => {
                let patterns: Vec<String> = patterns.iter().map(|&p| f.text(p).to_owned()).collect();
                return self.text_test(rel, f.rel_name(*input), *column as usize, f.text(*op), &patterns);
            }
            ArchivedNativeSpec::Components { edge, own } => return self.components(rel, f.rel_name(*edge), f.rel_name(*own)),
            ArchivedNativeSpec::Rank { query, counted, group, op } => return self.rank_rows(rel, f.rel_name(*query), f.rel_name(*counted), *group as usize, f.text(*op)),
            ArchivedNativeSpec::Reach { .. } | ArchivedNativeSpec::ReachThrough { .. } => return self.reach_rows(rel, &native.spec),
            ArchivedNativeSpec::Python { kind, input, targets } => return self.python_relation(rel, f.text(*kind), f.rel_name(*input), targets.map(|t| f.rel_name(t))),
            ArchivedNativeSpec::PythonCandidates { input, targets, .. } => return self.python_relation(rel, "candidates", f.rel_name(*input), Some(f.rel_name(*targets))),
        };
        let cols = |r: &str, n: usize| (0..n).map(|c| self.model.col(r, c)).collect::<Vec<_>>();
        let read = |sql: &str, width: usize| -> Result<Vec<Vector<i64>>> {
            let mut st = self.db.prepare(sql)?;
            let rows = st.query_map([], |row| (0..width).map(|i| row.get::<_, i64>(i)).collect::<rusqlite::Result<Vector<i64>>>())?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        };
        let e = cols(edge, 3);
        let edges: Vec<(i64, i64, i64)> = read(&format!("SELECT DISTINCT {}, {}, {} FROM {}", e[0], e[1], e[2], crate::code_rule_dsl::logic::sql::ident(edge)), 3)?.into_iter().map(|r| (r[0], r[1], r[2])).collect();
        let t = cols(target, 1);
        let targets: Vector<i64> = read(&format!("SELECT DISTINCT {} FROM {}", t[0], crate::code_rule_dsl::logic::sql::ident(target)), 1)?.into_iter().map(|r| r[0]).collect();
        // `dfs_first_memo`'s roots in increasing order of their second column: a number, or a symbol as text.
        let roots: Option<Vector<i64>> = match roots {
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
    fn native_demand(&mut self, input: &str) -> Result<std::sync::Arc<Vector<i64>>> {
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
            if let Some(super::program_image::ArchivedNativeSpec::PythonCandidates { context, member, .. }) = f.native(rel).map(|n| &n.spec) {
                producer.candidates(&batch, &requests, &wanted, f.text(*context), f.text(*member)).map_err(Error::Plan)?
            }else{producer.project(&batch, kind, &requests, &wanted).map_err(Error::Plan)?}
        };
        if self.model.inputs.atoms.is_some() {
            let strings = rows.iter().flatten().filter_map(|v| match v { SqlValue::Text(s) => Some(s.as_str()), _ => None });
            let mut ids = super::native::intern_batch(&self.db, strings)?.into_iter();
            for value in rows.iter_mut().flatten() {
                if matches!(value, SqlValue::Text(_)) { *value = SqlValue::Integer(ids.next().unwrap()); }
            }
            let values: Vec<Vector<i64>> = rows.into_iter().map(|row| row.into_iter().map(|v| match v {
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
                keys: Vec::new(),
                counts: Vec::new(),
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
        let values: Vec<Vector<i64>> = rows.iter().zip(ids).map(|((d, c, _), k)| rerail_blocks::vector![*d, *c, k]).collect();
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
            keys: Vec::new(),
            counts: Vec::new(),
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
        let targets: Vector<i64> = (0..t.len()).map(|r| t.value(r, 0)).collect::<BTreeSet<_>>().into_iter().collect();
        // `dfs_first_memo`'s roots in increasing order of their second column (a number, or a symbol by its text),
        // each root once, as `native_relation` orders them.
        let roots: Option<Vector<i64>> = match roots {
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
        let values: Vec<Vector<i64>> = rows.into_iter().map(|(a, b, c)| rerail_blocks::vector![a, b, c]).collect();
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
            keys: Vec::new(),
            counts: Vec::new(),
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
        let rows: BTreeSet<Vector<i64>> = captures
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
            keys: Vec::new(),
            counts: Vec::new(),
        });
        Ok(Some(source.len()))
    }

    /// A view of `base`'s columns `cols` (`input_view`) held in place (decisions 0329, 0353: a view is a read of its
    /// source, nothing makes its rows), and its rows' count. Its statistics are each column's exact ones: `base`'s,
    /// where the run holds them (`view_stats_held`); else counted when a plan first reads them: a batch's `node` from
    /// the batch without reading its rows (`syntax::node_column_counts`, `InPlaceView`), any other source off its
    /// columns (`SourceView`). Counted at once, a view no plan reads made every document of a batch's relation.
    fn hold_view_in_place(&mut self, base: &str, rel: &str, src: &Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, cols: &[usize], types: Vec<u8>) -> i64 {
        let batch = src.python_batch().filter(|_| src.columns().len() == 5 && cols.iter().all(|&c| crate::code_rule_dsl::facts::vtab::syntax::node_column_counted(c)));
        // The view's rows are its base's: the count the run holds of them where it holds one (an axis's length walks
        // every document's tree; 10 ms of a warm run that ran no rule), else the source's.
        let names = src.columns();
        let held: Option<Vec<i64>> = cols.iter().map(|&c| Some(self.model.stats.col.get(base, names.get(c)?)?[0])).collect();
        let n = match held.as_deref() {
            Some([first, rest @ ..]) if rest.iter().all(|r| r == first) => *first,
            _ => src.len() as i64,
        };
        super::native::component::hold_view_in_place(self.db, rel, Arc::clone(src), cols.iter().copied().collect::<rerail_blocks::vector::Vector<_>>(), types);
        if let Some(live) = self.view_stats_held(base, &names, cols, n, batch.is_some()) {
            self.model.stats.lazy.0.remove(rel);
            live_stats(&mut self.model, rel, live);
            super::profile::decision("a view held in place: its statistics (Q332)", || format!("{rel}: {base}'s, held"));
            return n;
        }
        super::profile::decision("a view held in place: its statistics (Q332)", || format!("{rel}: counted off {base} when first planned with ({base}'s are not held)"));
        let deferred: Arc<dyn Deferred> = match batch {
            Some(batch) => Arc::new(InPlaceView { rel: rel.to_owned(), batch, cols: cols.to_vec(), rows: n }),
            None => Arc::new(SourceView { rel: rel.to_owned(), src: Arc::clone(src), cols: cols.to_vec(), rows: n }),
        };
        self.model.stats.lazily(rel, n.max(1) as usize, deferred);
        n
    }

    /// The statistics of a view of `base`'s columns `cols` (`names`, its columns), `n` rows, from those the run holds
    /// of `base` itself (Q332): its columns' counts, kept and patched by the changed documents (decision 0458), and
    /// the kind column's values, the tree's (`Stats::tree`: by the same kind atoms the column holds). What counting
    /// the batch's documents gives (`InPlaceView`), none made. `None` where one is not held, or holds other rows than
    /// the view's (a node column's counts of `n` rows, the tree's kinds `n` nodes of as many kinds). Of another
    /// source than a batch's `node` (`node` false), any column's counts, with no values' counts.
    fn view_stats_held(&self, base: &str, names: &[String], cols: &[usize], n: i64, node: bool) -> Option<LiveRel> {
        let st = &self.model.stats;
        let (mut ndv, mut sq, mut freq) = (Vector::new(), Vector::new(), Vec::new());
        for (j, &c) in cols.iter().enumerate() {
            let &[rows, d, s, _] = st.col.get(base, names.get(c)?)?;
            if rows != n {
                return None;
            }
            match c {
                _ if !node => {}
                1 => {}
                2 => {
                    let kinds: table::Freq = st.tree.kinds().filter(|&(_, m, _)| m > 0).map(|(k, m, _)| Some((Constant::Int(k.parse().ok()?), m))).collect::<Option<_>>()?;
                    if kinds.values().sum::<i64>() != n || kinds.len() as i64 != d {
                        return None;
                    }
                    freq.push((j, std::sync::Arc::new(kinds)));
                }
                _ => return None,
            }
            ndv.push(d);
            sq.push(s);
        }
        Some(LiveRel { n, delta: n, ndv, sq, freq, keys: Vec::new(), counts: Vec::new() })
    }

    /// The reach index (decision 0119, `front::Native::Reach` and `ReachThrough`) over relations the engine holds:
    /// held as the index, read in place by its readers, no row made. `None`: an input is not held natively (the rows
    /// are then made by `reach_rows`).
    fn native_reach(&mut self, rel: &str, native: &super::program_image::ArchivedNativeSpec) -> Result<Option<(usize, String)>> {
        use super::native::{component, reach::Closed};
        use crate::code_rule_dsl::facts::vtab::Source as _;
        use super::program_image::ArchivedNativeSpec as Native;
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
            keys: Vec::new(),
            counts: Vec::new(),
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
        let list = |parts: &[(usize, Vector<i64>)]| -> String {
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
    fn reach_rows(&mut self, rel: &str, native: &super::program_image::ArchivedNativeSpec) -> Result<usize> {
        use super::native::reach::Closed;
        use crate::code_rule_dsl::facts::vtab::Source as _;
        use super::program_image::ArchivedNativeSpec as Native;
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
        let read = |r: &str| -> Result<Vec<Vector<i64>>> {
            let w = self.fields().width(r);
            let cols: Vec<String> = (0..w).map(|c| self.model.col(r, c)).collect();
            let mut st = self.db.prepare(&format!("SELECT DISTINCT {} FROM {}", cols.join(", "), crate::code_rule_dsl::logic::sql::ident(r)))?;
            let rows = st.query_map([], |row| (0..w).map(|i| row.get::<_, i64>(i)).collect::<rusqlite::Result<Vector<i64>>>())?;
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
        let rows = |s: &dyn crate::code_rule_dsl::facts::vtab::Source| -> Vec<Vector<i64>> {
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
            keys: Vec::new(),
            counts: Vec::new(),
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
        let mut ids = if atoms { super::native::intern_batch(&self.db, rows.iter().map(|(_, _, k)| k.as_str()))?.into_iter() } else { Vector::new().into_iter() };
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
        } else { Vector::new().into_iter() };
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
                    super::program_image::ArchivedNativeSpec::TextTest { op, patterns, .. } => Some((f.text(*op).to_owned(), patterns.iter().map(|&p| f.text(p).to_owned()).collect())),
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

    /// SPIKE: the rows of a stratum of the reference library by W's algorithm (`spike_strata`), from its inputs and
    /// the batch's tree; none where the chunk holds no batch.
    fn spike_rows(&self, scc: &super::front::CompAt) -> Option<Vec<(String, spike_strata::Flat)>> {
        let _span = super::profile::span(|| "spike executor".into());
        let (batch, tree) = self.spike_tree()?;
        let chunk = spike_strata::Chunk::new(&tree, Some(&batch));
        Some(spike_strata::run(&scc.iter().collect::<Vec<_>>(), &chunk, self))
    }

    /// SPIKE: the chunk's batch (the `child` view's) and its tree (`spike_strata::Tree`), made at its first spiked
    /// stratum and held in the batch's slot (`Batch::spike`).
    fn spike_tree(&self) -> Option<(Arc<crate::code_rule_dsl::facts::vtab::syntax::Batch>, Arc<spike_strata::Tree>)> {
        use crate::code_rule_dsl::facts::vtab::syntax::Batch;
        let batch = ["child", "node", "kind"].iter().find_map(|rel| self.spike_source(rel)?.syntax_batch())?.downcast::<Batch>().ok()?;
        let slot = Arc::clone(batch.spike.get_or_init(|| -> Arc<dyn std::any::Any + Send + Sync> {
            if std::env::var_os("RERAIL_SPIKE_WHY").is_some() {
                let rels: Vec<String> = ["node", "kind", "child", "field", "text", "within"]
                    .iter()
                    .map(|rel| match self.spike_source(rel) {
                        None => format!("{rel}: none"),
                        Some(src) => {
                            let same = src.syntax_batch().and_then(|b| b.downcast::<Batch>().ok()).map(|b| Arc::ptr_eq(&b, &batch));
                            let derived = super::native::component::derived(self.db, rel).is_some();
                            format!("{rel}: {} rows, batch {same:?}, derived {derived}", src.len())
                        }
                    })
                    .collect();
                eprintln!("SPIKE BATCH {} rows; {}", batch.nodes().total(), rels.join("; "));
            }
            Arc::new(spike_strata::Slot::default())
        }))
        .downcast::<spike_strata::Slot>()
        .ok()?;
        // The tape values admitted into the chunk so far (decision 0431 §8): their nodes are the `node` view's too.
        let values = self
            .spike_source("node")
            .and_then(|src| src.as_any().and_then(|a| a.downcast_ref::<crate::code_rule_dsl::facts::vtab::syntax::values::Layered>()).map(|l| l.value_batches()))
            .unwrap_or_default();
        let tree = slot.tree(&batch, values);
        Some((batch, tree))
    }

    /// SPIKE: the texts of symbol atoms, by the connection's scope (a stratum may run off the run's thread).
    fn spike_texts(&self, atoms: &[i64]) -> Option<Vec<String>> {
        let symbols = super::native::setfix::symbols(self.db).ok()?;
        let s = symbols.read().ok()?;
        Some(atoms.iter().map(|&a| s.name(a)).collect())
    }

    /// SPIKE: a relation the chunk holds: its bound view, or a derived relation.
    fn spike_source(&self, rel: &str) -> Option<Arc<dyn crate::code_rule_dsl::facts::vtab::Source>> {
        crate::code_rule_dsl::facts::vtab::bound_source(self.db, rel).or_else(|| super::native::component::derived(self.db, rel))
    }

    /// SPIKE: stratum `scc` evaluated over the chunk's tree and held relations (`spike_eval`): its relations' rows,
    /// held by the chunk for the strata after it; `None` where it is declined (logged with `RERAIL_SPIKE_WHY`).
    fn spike_eval(&mut self, scc: &super::front::CompAt, rules: &[RuleKey]) -> Result<Option<Vec<(String, spike_eval::HRel)>>> {
        let names: Vec<&str> = scc.iter().collect();
        let vs = Arc::clone(&self.versions);
        let f = vs.whole(self.version);
        if names.iter().any(|r| f.lattice_decl(r).is_some() || f.gate(r) || f.request(r) || f.native(r).is_some()) {
            spike_eval::declined(&names, "a lattice, gate, request or native relation");
            return Ok(None);
        }
        let (batch, tree) = match self.spike_tree() {
            Some((b, t)) => (Some(b), t),
            None => (None, self.spike_holder()),
        };
        let pairs: Vec<(&Version, super::program_image::RuleRef<'_>)> = rules.iter().map(|&k| vs.rule_at(k)).collect();
        let inputs = Arc::clone(&self.inputs);
        let key = self.spike_key();
        let run_names = SpikeNames { db: self.db, f, tree: &tree, inputs: &inputs, key };
        let prog = match spike_eval::compile(&names, &pairs, &|r| f.width(r), &run_names) {
            Ok(p) => p,
            Err(why) => {
                spike_eval::declined(&names, &why);
                return Ok(None);
            }
        };
        // A projection of one tree atom (`kind(n, k) :- node(_, n, k, _, _)`) is read off the tree by the strata here
        // after it; its rows, which those the engine runs read, are the engine's to make (no join, a view's rows).
        if batch.is_none() && spike_eval::reads_tree(&prog) {
            spike_eval::declined(&names, "reads the tree, no syntax batch");
            return Ok(None);
        }
        if let Some(al) = prog.alias() {
            tree.aliases.lock().unwrap().insert((key, prog.rels[0].clone()), al);
            spike_eval::declined(&names, "a projection of the tree, read off it here");
            return Ok(None);
        }
        // A relation a request fills (its answers') is whole once its stratum has run: answers fill the produced
        // relations of the stratum that asks or of one after it, never one that has run.
        let mut read: Vec<Option<Arc<spike_eval::HRel>>> = vec![None; prog.rels.len()];
        for (i, rel) in spike_eval::reads(&prog) {
            let held = tree.held.lock().unwrap().get(&(key, rel.to_owned())).cloned();
            let h = match held {
                Some(h) => h,
                None => {
                    let _span = super::profile::span(|| format!("spike eval: read {rel}"));
                    let Some(src) = super::native::component::derived(self.db, rel).or_else(|| crate::code_rule_dsl::facts::vtab::bound_source(self.db, rel)) else {
                        spike_eval::declined(&names, &format!("{rel} not held"));
                        return Ok(None);
                    };
                    // The engine's relation read where it lies; another source's rows copied flat.
                    let width = f.width(rel);
                    let h = Arc::new(spike_eval::HRel::of_engine(&src, width, src.len()).unwrap_or_else(|| {
                        if std::env::var_os("RERAIL_SPIKE_WHY").is_some() {
                            let held = super::native::component::held_rows(&*src).map(|(_, n, dead, _)| (n, dead.iter().any(|&w| w != 0)));
                            eprintln!("SPIKE COPY {rel}: {} rows copied flat: engine's (rows, dead) {held:?}", src.len());
                        }
                        let flat = spike_strata::rows(&*src);
                        spike_eval::HRel::of_rows(flat.k, flat.v, src.len())
                    }));
                    tree.held.lock().unwrap().insert((key, rel.to_owned()), Arc::clone(&h));
                    h
                }
            };
            if h.k != prog.widths[i] {
                spike_eval::declined(&names, &format!("{rel} held with {} columns, declared {}", h.k, prog.widths[i]));
                return Ok(None);
            }
            read[i] = Some(h);
        }
        let chunk = spike_strata::Chunk::new(&tree, batch.as_deref());
        let atoms = Arc::clone(super::native::setfix::symbols(self.db)?.atoms());
        let calls = SpikeCalls { db: self.db, atoms, long: std::sync::OnceLock::new() };
        let own = match spike_eval::run(&prog, &chunk, read, &calls) {
            Ok(own) => own,
            Err(why) => {
                spike_eval::declined(&names, &why);
                return Ok(None);
            }
        };
        Ok(Some(prog.rels.iter().cloned().zip(own).collect()))
    }

    /// SPIKE: a relation `spike_eval` made (its rows distinct), held as the engine's relation `rel`: copied once, in
    /// its order, with no sort.
    fn spike_retain_distinct(&mut self, rel: &str, h: &spike_eval::HRel) -> Result<()> {
        use super::native::component;
        let _span = super::profile::span(|| format!("spike: retain {rel}"));
        component::retain_distinct(self.db, rel, h.own(), vec![b'i'; h.k])?;
        self.native_rels.insert(rel.to_owned());
        // Its statistics counted when a plan first reads them (`SourceView`): most are read only by strata the
        // evaluator runs, which plan with none.
        let made = component::derived(self.db, rel).expect("just retained");
        self.model.stats.lazily(rel, h.n.max(1), Arc::new(SourceView { rel: rel.to_owned(), src: made, cols: (0..h.k).collect(), rows: h.n as i64 }));
        Ok(())
    }

    /// SPIKE: a relation `spike_eval` made, held by the chunk for the strata after it.
    fn spike_hold(&self, rel: &str, h: spike_eval::HRel) -> Option<Arc<spike_eval::HRel>> {
        let tree = self.spike_holder();
        let h = Arc::new(h);
        tree.held.lock().unwrap().insert((self.spike_key(), rel.to_owned()), Arc::clone(&h));
        Some(h)
    }

    /// SPIKE: the chunk's held copies of `rels` (`spike_strata::Tree::held`) let go, as the engine releases them once
    /// no stratum after reads them.
    fn spike_release(&self, rels: &[String]) {
        if !spike_eval::enabled() || rels.is_empty() {
            return;
        }
        let tree = self.spike_holder();
        let key = self.spike_key();
        let mut held = tree.held.lock().unwrap();
        for r in rels {
            held.remove(&(key, r.clone()));
        }
    }

    /// SPIKE: the tree that holds the session's relations for `spike_eval`: its batch's, else its own bare one.
    fn spike_holder(&self) -> Arc<spike_strata::Tree> {
        match self.spike_tree() {
            Some((_, t)) => t,
            None => Arc::clone(self.spike_bare.get_or_init(|| Arc::new(spike_strata::Tree::bare()))),
        }
    }

    /// SPIKE: the run's key of the chunk's held relations (its versions and version).
    fn spike_key(&self) -> usize {
        Arc::as_ptr(&self.versions) as *const () as usize ^ self.version.rotate_left(48)
    }

    /// SPIKE: flat `rows` held as the engine's relation `rel`, as a stratum's.
    fn spike_retain(&mut self, rel: &str, rows: &spike_strata::Flat) -> Result<()> {
        self.spike_retain_flat(rel, &rows.v)
    }

    /// SPIKE (check mode): which `node` and `kind` the engine holds now (their lengths, whether their batch is the
    /// tree's) and their rows of node `n`.
    fn spike_whence(&self, n: i64) -> String {
        use crate::code_rule_dsl::facts::vtab::syntax::Batch;
        let batch = self.spike_tree().map(|(b, _)| b);
        ["node", "kind"]
            .iter()
            .map(|&rel| {
                let Some(src) = self.spike_source(rel) else { return format!("{rel}: none") };
                let same = src.syntax_batch().and_then(|b| b.downcast::<Batch>().ok()).map(|b| batch.as_ref().is_some_and(|t| Arc::ptr_eq(&b, t)));
                let col = usize::from(rel == "node");
                let flat = spike_strata::rows(&*src);
                let hit: Vec<&[i64]> = flat.v.chunks_exact(flat.k.max(1)).filter(|r| r.get(col) == Some(&n)).take(2).collect();
                let derived = super::native::component::derived(self.db, rel).is_some();
                format!("{rel}: {} rows, batch {same:?}, derived {derived}, rows of it {hit:?}", src.len())
            })
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// SPIKE: rows held as the engine's relation `rel`, flat, as a stratum's.
    fn spike_retain_flat(&mut self, rel: &str, rows: &[i64]) -> Result<()> {
        use super::native::component;
        let _span = super::profile::span(|| format!("spike: retain {rel}"));
        let width = self.fields().width(rel);
        component::retain_flat(self.db, rel, rows, vec![b'i'; width])?;
        self.native_rels.insert(rel.to_owned());
        let made = component::derived(self.db, rel).expect("just retained");
        let stats = made.statistics().expect("retained relations own their statistics");
        live_stats(&mut self.model, rel, LiveRel {
            n: made.len() as i64,
            delta: made.len() as i64,
            ndv: stats.iter().map(|s| s.2).collect(),
            sq: stats.iter().map(|s| s.3).collect(),
            freq: Vec::new(),
            keys: Vec::new(),
            counts: Vec::new(),
        });
        Ok(())
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
        let all: Vector<usize> = (0..width).collect();
        let mut row = rerail_blocks::vector![0i64; width];
        let rows: Vec<Vector<i64>> = (0..source.len()).map(|r| { source.row_at(r, &all, &mut row); row.clone() }).collect();
        // Each distinct string once, by its atom.
        let atoms: Vector<i64> = rows.iter().map(|r| r[column]).collect::<BTreeSet<_>>().into_iter().collect();
        let strings = super::native::symbol_texts(&self.db, &atoms)?;
        let symbols: Option<Vector<u32>> = atoms.iter().map(|&a| u32::try_from(a).ok()).collect();
        let passed: HashSet<i64> = texts.passing(k, &strings, symbols.as_deref()).into_iter().map(|i| atoms[i]).collect();
        let values: Vec<Vector<i64>> = rows.into_iter().filter(|r| passed.contains(&r[column])).collect();
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
            keys: Vec::new(),
            counts: Vec::new(),
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
        let symbols: Option<Vector<u32>> = if atoms { symbols.into_iter().collect() } else { None };
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
        self.model.stats.card.insert(r, if at_least_one { n.max(1.0) } else { n });
        Ok(())
    }
}

/// SPIKE: what `spike_eval` compiles a stratum by: the run's symbols, kind sets and inputs, the chunk's tree.
struct SpikeNames<'a> {
    db: crate::code_rule_dsl::facts::vtab::Db<'a>,
    f: Fields<'a>,
    tree: &'a spike_strata::Tree,
    inputs: &'a RunInputs<'a>,
    /// The run's key in the chunk's held relations.
    key: usize,
}

impl spike_eval::Names for SpikeNames<'_> {
    fn intern(&self, text: &str) -> i64 {
        let syms = super::native::setfix::symbols(self.db).expect("the run's symbols");
        let mut s = syms.write().unwrap();
        s.id(text)
    }

    fn kindset(&self, set: &str) -> Option<Vec<i64>> {
        let kinds = self.f.kindset(set)?;
        // A set's kinds are their atoms (`#k` constants, as text), or their names; `-1` is a kind no dialect has.
        let mut out: Vec<i64> = kinds
            .iter()
            .filter_map(|&k| {
                let t = self.f.text(k);
                match t.parse::<i64>() {
                    Ok(a) => (a >= 0).then_some(a),
                    Err(_) => self.tree.atoms.get(t).copied(),
                }
            })
            .collect();
        out.sort_unstable();
        out.dedup();
        if std::env::var_os("RERAIL_SPIKE_WHY").is_some() {
            eprintln!("SPIKE KINDSET {set}: {:?} -> {out:?}", kinds.iter().map(|&k| self.f.text(k)).collect::<Vec<_>>());
        }
        Some(out)
    }

    fn tree_relation(&self, rel: &str) -> bool {
        let tree = matches!(rel, "kind" | "node" | "child" | "field" | "text" | "within") || ["rr_nearest_", "rr_inside_", "rr_has_"].iter().any(|p| rel.starts_with(p));
        tree && (self.inputs.contains(rel) || self.f.is_input(rel))
    }

    fn alias(&self, rel: &str) -> Option<spike_eval::Alias> {
        self.tree.aliases.lock().unwrap().get(&(self.key, rel.to_owned())).cloned()
    }
}

/// SPIKE: what `spike_eval`'s calls read of the run: its symbols, and its long-text nodes for node reads.
struct SpikeCalls<'a> {
    db: crate::code_rule_dsl::facts::vtab::Db<'a>,
    /// The run's atoms, read and interned in place (no lock of the run's interner).
    atoms: Arc<crate::content_store::atoms::Atoms>,
    long: std::sync::OnceLock<Option<Arc<dyn crate::code_rule_dsl::facts::vtab::Source>>>,
}

impl spike_eval::Calls for SpikeCalls<'_> {
    fn name(&self, sym: i64) -> Option<String> {
        let atom = crate::content_store::atoms::Sym::try_from(sym).ok()?;
        self.atoms.with_str(atom, str::to_owned).or_else(|| self.atoms.value(atom).ok().flatten())
    }

    fn intern(&self, text: &str) -> i64 {
        self.atoms.intern(text).map_or(-1, i64::from)
    }

    fn atom(&self, text: &str) -> i64 {
        self.atoms.existing(text).map_or_else(|| self.intern(text), i64::from)
    }

    /// `a.b`; `"" . b` is `b`.
    fn cat(&self, a: i64, b: i64) -> i64 {
        match (self.name(a), self.name(b)) {
            (Some(x), _) if x.is_empty() => b,
            (Some(x), Some(y)) => self.intern(&format!("{x}.{y}")),
            _ => super::native::component::cat_symbols(self.db, a, b),
        }
    }

    fn node_read(&self, name: &str, n: i64) -> Option<String> {
        let long = self.long.get_or_init(|| crate::code_rule_dsl::facts::vtab::bound_source(self.db, crate::code_rule_dsl::facts::vtab::LONG_TEXT_NODES));
        crate::code_rule_dsl::logic::strings::functions::node_read(name, long.as_deref(), n)
    }
}

/// SPIKE: a stratum's inputs as the chunk holds them.
impl spike_strata::Inputs for Run<'_, '_> {
    fn read(&self, rel: &str) -> spike_strata::Flat {
        let _span = super::profile::span(|| "spike: read an input".into());
        super::native::component::derived(self.db, rel).or_else(|| crate::code_rule_dsl::facts::vtab::bound_source(self.db, rel)).map(|s| spike_strata::rows(&*s)).unwrap_or_default()
    }

    fn texts(&self, atoms: &[i64]) -> Vec<String> {
        self.spike_texts(atoms).unwrap_or_default()
    }
}

/// Whether a text-test stratum's rules are its native's definition (`h(x…) :- input(x…), test`), so the native makes
/// its rows: in a maintained form its relation's rules are its old rows and its new ones (`maintain`), made as rules,
/// while its `h__ins` and a stratum derived again are the definition.
fn text_test_defined(vs: &body::Versions, rules: &[body::RuleKey], input: &str) -> bool {
    matches!(rules, [k] if { let (v, r) = vs.rule_at(*k); r.body().filter_map(|l| l.atom()).map(|a| v.rel_name(a.rel())).eq([input]) })
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
            let o = stats.obs.rule_mut(rule).entry(b.sig_key(s.at, &bound)).or_insert([0, 0]);
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
                seen.and_then(|o| o.get(&b.sig_key(s.at, &bound))).filter(|n| n[0] >= OBS_MIN).map(|n| n[1] as f64 / n[0] as f64)
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
type ModeInputs = BTreeMap<String, ([Arc<dyn crate::code_rule_dsl::facts::vtab::Source>; 4], Option<rerail_blocks::vector::Vector<usize>>)>;

/// The engine's list a maintain mode's joined line runs in (`setfix`): its probes (seeds and delta probes), then its
/// removals, checked adds and rechecks (`maint_step_counts`), each in spec order.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ModeLine {
    Probe,
    Remove,
    Add,
    Recheck,
}

#[cfg_attr(not(any(test, feature = "test-hooks")), allow(dead_code))]
pub fn mode_strata() -> u64 {
    run_counts().mode_strata.load(std::sync::atomic::Ordering::Relaxed)
}

/// A column's type in the native store's spec: `s`, a symbol it reads as text, only where symbols are text. Over
/// atoms every column is an integer (a name, kind or field atom), as the planner's SQL holds it (O13).
fn sym_char(inputs: &Inputs, t: &str) -> char {
    if t == "sym" && inputs.atoms.is_none() { 's' } else { 'i' }
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
                super::program_image::AggRef::Bounded { .. } => "bounded min or max (an as-of search)",
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
fn stratum_label(scc: &super::front::Comp) -> String {
    let key = stratum_key(scc);
    if key.len() > 80 { format!("{}…({} relations)", &key[..key.floor_char_boundary(80)], scc.len()) } else { key }
}

/// A relation's change in a run, which its frame's delta is written from (decision 0117 step 4): its version marks,
/// read when the delta is written (0364 R2: no row is copied to state a change), or the rows a stratum derived again
/// made against its kept rows (`kept_delta`).
#[derive(Clone, Debug)]
pub enum Delta {
    Marks { gone: usize, added: usize },
    Rows { gone: Vector<i64>, added: Vector<i64> },
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
    pub fn rows(&self, rel: &super::native::component::InRel) -> Option<(Vector<i64>, Vector<i64>)> {
        match self {
            Delta::Marks { .. } => rel.version_delta().map(|(added, gone)| (gone, added)),
            Delta::Rows { gone, added } => Some((gone.clone(), added.clone())),
        }
    }
}

/// How a relation derived again moved against its kept rows (`kept_delta`).
enum Moved {
    /// No row gone or added.
    Not,
    /// It is now a version of its kept rows: its change is its marks.
    Marks,
    /// Its rows gone and added, row-major: a relation that cannot be made a version of its kept rows (a reach index
    /// held over its source, decision 0119).
    Rows(Vector<i64>, Vector<i64>),
}

/// The delta of `rel`, derived again (a reach index made again against its old rows kept in memory, decision 0331, or
/// a stratum the maintain mode does not take), against its kept rows `old`, so its readers are maintained rather than
/// derived again: where it can be, `rel` made a version of `old` in place; else its rows gone and added, the rows of
/// each side sorted and compared (copied: 0364 R2's one open case, counted by `rows_copied`). Neither is held under a
/// name: its readers are given them as its versions.
fn kept_delta(db: &Connection, rel: &str, old: &Arc<super::native::component::InRel>) -> Result<Moved> {
    let _span = super::profile::span(|| format!("{rel}: its delta against its kept rows"));
    // Where it can be, the relation derived again is made a version of its kept rows in place (0364 R2): its change
    // is then its marks, and nothing is copied or held beside it.
    if let Some(new) = super::native::component::held(db, rel)
        && let Some(rebased) = super::native::component::InRel::rebased(rel, old, &new)
    {
        let (gone, added) = rebased.version_counts().expect("started from its kept rows");
        super::native::component::hold_as(db, rel, Arc::new(rebased));
        return Ok(if gone + added > 0 { Moved::Marks } else { Moved::Not });
    }
    if std::env::var_os("RERAIL_SHAPE_TRACE").is_some() {
        let new = super::native::component::held(db, rel);
        eprintln!("RERAIL_SHAPE_TRACE {rel}: not rebased (kept: view {}, levels {}, {} rows; new: {})", old.view_kind().is_some(), old.has_levels(), old.rows(),
                  new.map_or("not held".to_owned(), |n| format!("view {}, levels {}, {} rows", n.view_kind().is_some(), n.has_levels(), n.rows())));
    }
    let types = old.types().to_vec();
    let k = types.len();
    let new = super::native::component::derived(db, rel).ok_or_else(|| Error::Plan(format!("{rel}: derived again, but not held")))?;
    let rows = |s: &dyn crate::code_rule_dsl::facts::vtab::Source| -> Vec<Vector<i64>> {
        let cols: Vector<usize> = (0..k).collect();
        let mut v: Vec<Vector<i64>> = (0..s.len()).map(|r| { let mut row = rerail_blocks::vector![0i64; k]; s.row_at(r, &cols, &mut row); row }).collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    let (was, now) = (rows(old.source((0..k).map(|c| sql_col(rel, c)).collect()).as_ref()), rows(new.as_ref()));
    crate::code_rule_dsl::logic::run::shape::rows_copied(was.len() + now.len());
    let (mut minus, mut plus) = (Vector::new(), Vector::new());
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
    Ok(if minus.is_empty() && plus.is_empty() { Moved::Not } else { Moved::Rows(minus, plus) })
}

/// A relation's rows gone and added (`Moved::Rows`) as its minus and plus versions, beside its kept rows `old`.
fn diff_versions(old: &super::native::component::InRel, gone: &[i64], added: &[i64]) -> [Arc<super::native::component::InRel>; 2] {
    let k = old.k.max(1);
    [gone, added].map(|rows| super::native::component::InRel::from_flat(rows.iter().copied().collect::<rerail_blocks::vector::Vector<_>>(), rows.len() / k, old.types().to_vec()))
}

fn stratum_key(scc: &super::front::Comp) -> String {
    let key = scc.iter().collect::<Vec<_>>().join("/");
    if key.len() > 80 { format!("{}…({} relations)", &key[..key.floor_char_boundary(80)], scc.len()) } else { key }
}

/// A delta probe's plan with the stratum's own relations named as read: the last wave's delta at `at`, else the
/// rows so far.
fn delta_sources(mut x: super::profile::RulePlan, order: Order<'_, '_>, at: usize, scc: &super::front::Comp) -> super::profile::RulePlan {
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

/// The relations rule `k`'s atoms and aggregates read, in body order, by its handles in `vs`.
fn reads_of(vs: &body::Versions, k: RuleKey) -> impl Iterator<Item = &str> + '_ {
    let (v, rule) = vs.rule_at(k);
    rule.body().filter_map(|l| l.atom()).map(move |a| v.rel_name(a.rel()))
}

/// The relations of `scc` that reach themselves through `rules` (any read: positive, negated or aggregated), each
/// read by its handles.
pub fn cyclic_relations(vs: &body::Versions, scc: &super::front::Comp, rules: &[RuleKey]) -> BTreeSet<String> {
    cyclic_in(scc, rules.iter().flat_map(|&r| {
        let head = vs.head_at(r);
        reads_of(vs, r).map(move |a| (head, a))
    }))
}

/// The relations of `scc` that reach themselves through `edges`, each `(head, a relation its rule reads)`.
fn cyclic_in<'a>(scc: &super::front::Comp, edges: impl Iterator<Item = (&'a str, &'a str)>) -> BTreeSet<String> {
    // A relation reaches itself where it is in a strongly connected part of more than one relation, or reads itself:
    // one pass over the edges (Tarjan's), not a walk from each relation (a 220-relation stratum walked 48,000 times).
    let names: Vec<&str> = scc.iter().collect();
    let at = |r: &str| names.binary_search(&r).ok();
    let mut next: Vec<Vector<usize>> = vec![rerail_blocks::vector::Vector::new(); names.len()];
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
    lits: Vector<usize>,
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
/// Its last field keeps the connection's data (`vtab::ConnData`) while the strand's detached `Db` reads it: dropped
/// after the run (fields drop in order).
struct Strand<'a>(Run<'a, 'static>, Arc<crate::content_store::atoms::Atoms>, (u64, u64), Arc<crate::code_rule_dsl::facts::vtab::ConnData>);

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
    fn strand_reads_with(&self, scc: &super::front::Comp, rules: &[RuleKey], held_too: &BTreeSet<String>) -> std::result::Result<BTreeSet<String>, &'static str> {
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
        let mut reads: BTreeSet<String> = scc.names();
        for &r in rules {
            let (v, rule) = self.versions.rule_at(r);
            for a in rule.body().filter_map(|l| l.atom()) {
                let rel = v.rel_name(a.rel());
                if scc.contains(rel) {
                    continue;
                }
                let held = self.native_rels.contains(rel) || held_too.contains(rel);
                let bound = self.inputs.contains(rel) && crate::code_rule_dsl::facts::vtab::bound_source(self.db, rel).is_some();
                let viewed = self.in_place(rel) && self.model.view_src.get(rel).is_some_and(|v| crate::code_rule_dsl::facts::vtab::bound_source(self.db, &v.0).is_some());
                if !(held || bound || viewed) {
                    return Err(if self.inputs.contains(rel) { "reads an input not bound in place" } else { "reads a relation not held (SQL's)" });
                }
                reads.insert(rel.to_owned());
                if let Some(rerail_section::Tuple2(base, _)) = self.model.view_src.get(rel) {
                    reads.insert(base.as_str().to_owned());
                }
            }
        }
        Ok(reads)
    }

    /// A strand for a stratum that reads `reads` (`strand_reads_with`) and whose rules' digests are `rules`.
    fn strand(&mut self, reads: &BTreeSet<String>, rules: &[u64], resolve: &[&str]) -> Result<Strand<'a>> {
        let arena = super::native::component::prime(&self.db)?;
        let model = self.model.fork(self.versions.whole(self.version), reads, rules, resolve.iter().copied())?;
        // SAFETY: the strand holds `keep` while its `db` is used (`Strand`'s last field, dropped after its run).
        let (db, keep) = unsafe { self.db.detached() };
        Ok(Strand(Run {
            db,
            model,
            views: Arc::clone(&self.views),
            made: HashSet::new(),
            device_views: HashSet::new(),
            orders: HashMap::new(),
            keyless: HashSet::new(),
            native_rels: reads.iter().filter(|r| self.native_rels.contains(*r)).cloned().collect(),
            spike_bare: Arc::clone(&self.spike_bare),
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
            proofs: self.proofs.clone(),
        }, arena, (0, 0), keep))
    }

    /// Takes back a strand of the stratum `scc` that ran: its relations, and what its model learnt of them and of
    /// its rules (`rules`, their digests).
    fn absorb(&mut self, s: Strand<'a>, scc: &super::front::Comp, rules: &[u64]) {
        let Strand(run, _, waves, _) = s;
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
    fn run_members(&mut self, members: &[(super::front::CompAt, Vec<RuleKey>)], times: &mut Vec<(std::time::Instant, std::time::Instant)>, reserved: &mut crate::content_store::atoms::Reserved, park: Option<&mut Vec<(usize, Box<Driving>)>>) -> (usize, Option<Result<(Option<String>, Option<String>)>>) {
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
            let ks: Vector<usize> = waiting.iter().map(|w| w.0).collect();
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
            let ran = {
                let _span = super::profile::span(|| format!("stratum {} (in a job)", stratum_label(scc)));
                self.0.native_stratum(scc, rules)
            };
            let stop = match ran {
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
    fn flush_parked(&mut self, members: &[(super::front::CompAt, Vec<RuleKey>)], waiting: &mut Vec<(usize, Box<Driving>)>, times: &mut [(std::time::Instant, std::time::Instant)]) -> Result<()> {
        while !waiting.is_empty() {
            super::lanes::wave::flush(waiting.len()).map_err(Error::Plan)?;
            self.resume_parked(members, waiting, times)?;
        }
        Ok(())
    }

    /// Resumes the members `run_members` parked (`waiting`) once the run's thread flushed their wave: those that wait
    /// on a wave again stay in `waiting`, for the next flush.
    fn resume_parked(&mut self, members: &[(super::front::CompAt, Vec<RuleKey>)], waiting: &mut Vec<(usize, Box<Driving>)>, times: &mut [(std::time::Instant, std::time::Instant)]) -> Result<()> {
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
        if !rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var("RERAIL_STATS_HEAVY").map_or(true, |v| v != "off")) || !raise && !joins::any_raised() {
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
                    if self.model.stats.solving.contains(rel) || self.heavy_current(rel, c) || self.heavy_from_shared(rel, c) {
                        continue;
                    }
                    let wanted = raised.iter().any(|(r, k)| r == rel && *k == c) || raise && {
                        let (rows, ndv) = (self.model.card(rel), self.model.ndv.at(rel, c).copied().unwrap_or(1.0));
                        let fan = self.model.stats.fan.at(rel, c).copied().unwrap_or(rows / ndv.max(1.0));
                        ndv > table::HEAVY as f64 && fan * ndv / rows.max(1.0) > 2.0
                    };
                    if wanted {
                        joins::log(|| format!("heavy values wanted {rel}.{c}: kept {:?} rows, card {:?}, held {}, mark {}", self.model.stats.heavy.at(rel, c).map(|h| h.rows), self.model.stats.card.get(rel), super::native::component::held(self.db, rel).is_some(), self.model.stats.heavy_mark.get(rel).is_some()));
                        want.entry(rel.to_owned()).or_default().insert(c);
                    }
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
        let mut full: Vec<(String, Vector<usize>)> = Vec::new();
        for (rel, cols) in want {
            let cols: Vector<usize> = cols.into_iter().collect();
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
            let mark = std::sync::Arc::new(now.mark());
            self.share_heavy(&rel, &cols, &mark);
            self.model.stats.heavy_mark.insert(rel.as_str(), mark);
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
            if let Some(now) = now {
                let mark = std::sync::Arc::new(now.mark());
                self.share_heavy(rel, cols, &mark);
                self.model.stats.heavy_mark.insert(rel.as_str(), mark);
            }
        }
        n
    }

    /// Heavy values of column `c` of `rel` another fork of this model made over the relation as it is now
    /// (`Stats::heavy_shared`), taken as this model's own: whether there were any.
    fn heavy_from_shared(&mut self, rel: &str, c: usize) -> bool {
        let Some(now) = super::native::component::held(self.db, rel) else { return false };
        let found = self.model.stats.heavy_shared.lock().at(rel, c).cloned();
        let Some((mark, h)) = found else { return false };
        if !now.delta_ids_since_mark(&mark).is_some_and(|d| d.counts() == (0, 0)) {
            return false;
        }
        self.model.stats.heavy.insert((rel, c), h);
        self.model.stats.heavy_mark.insert(rel, mark);
        true
    }

    /// Shares the heavy values of `rel`'s columns `cols` this model holds, made over the relation `mark` saw, with the
    /// model's other forks (`Stats::heavy_shared`).
    fn share_heavy(&self, rel: &str, cols: &[usize], mark: &std::sync::Arc<super::native::component::RelMark>) {
        let mut shared = self.model.stats.heavy_shared.lock();
        for &c in cols {
            if let Some(h) = self.model.stats.heavy.at(rel, c) {
                shared.insert((rel, c), (std::sync::Arc::clone(mark), std::sync::Arc::clone(h)));
            }
        }
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
            top.iter().take(table::HEAVY).map(|t| t.0).collect()
        };
        // The trial's ids are the relation's own (`trial_ids`), read through it: the SQL source (`engine::derived`)
        // numbers a relation with dead rows by its live rows instead.
        let (Some(ids), Some(lrel)) = (engine::trial_ids(self.db, name, line)?, engine::held(self.db, b.rel(la))) else { return Ok(None) };
        let lsrc = |id: u32, c: usize| lrel.at(id as usize).at(c);
        let mut parts = HeavyParts { link: link.clone(), cv, hot: rerail_blocks::vector::Vector::new(), cold: rerail_blocks::vector::Vector::new() };
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
                let mut keys: HashMap<i64, Vector<i64>> = HashMap::new();
                for &id in &ids {
                    let u = lsrc(id, lc);
                    let vs = keys.entry(u).or_insert_with(|| { let mut vs = Vector::new(); linked.probe_each(&[cu], &[u], &mut |row| vs.push(row(cv))); vs });
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
    fn split_spike(&mut self, name: &str, line: usize, b: &Body<'_>, orders: &[Vec<Step>], lead: usize, first: Option<usize>, cut_order: &[Step], ri: &BTreeMap<String, usize>, scc: &super::front::Comp, native_rels: &BTreeSet<String>) -> Result<()> {
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
                    engine::trial_over(self.db, name, line, Some(spec.get()), step, Ok(picked), budget)?
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
        let mut pairs: Vec<(usize, usize, Vector<usize>, Vector<usize>)> = Vec::new();
        let mut wanted: Vec<(usize, Vector<usize>)> = Vec::new();
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
                let ca: Vector<usize> = shared.iter().filter_map(|&v| var_at(a, v)).collect();
                let cc: Vector<usize> = shared.iter().filter_map(|&v| var_at(c, v)).collect();
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
        let of = |s: usize, cols: &Vector<usize>| wanted.iter().position(|w| w.0 == s && &w.1 == cols).and_then(|i| hist.get(i)?.as_ref());
        let mut out = Vec::new();
        for (p, q, ca, cc) in pairs {
            let ((sp, a), (sq, c)) = (atoms[p], atoms[q]);
            let (Some((rows, ha)), Some((_, hc))) = (of(sp, &ca), of(sq, &cc)) else { continue };
            if (*rows as i64) < OBS_MIN {
                continue;
            }
            let joined: u64 = ha.iter().map(|(k, n)| n * hc.get(k)).sum();
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
            top.iter().take(table::HEAVY).map(|t| t.0).collect::<Vector<i64>>()
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
    fn runnable(&self, scc: &super::front::Comp, rules: &[RuleKey]) -> Vec<RuleKey> {
        let empty = |rel: &str| {
            if scc.contains(rel) {
                return false;
            }
            self.native_rels.contains(rel) && super::native::component::held(self.db, rel).is_some_and(|h| h.holds_none())
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
    fn seed_specs(&mut self, scc: &super::front::Comp, rules: &[RuleKey]) -> Vec<PlanJoin> {
        let versions = Arc::clone(&self.versions);
        let pf = versions.whole(self.version);
        let f = self.fields();
        if scc.iter().any(|r| f.lattice(r) || f.native(r).is_some() || f.gate(r)) {
            return Vec::new();
        }
        let ri: BTreeMap<String, usize> = scc.iter().enumerate().map(|(i, r)| (r.to_owned(), i)).collect();
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
        self.model.stats.sourced(column_sources(&versions, scc, rules));
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
    fn input_view(&self, r: RuleKey) -> Option<(String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, Vector<usize>)> {
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
        let cols: Vector<usize> = at.iter().map(|&c| names.iter().position(|n| *n == self.model.col(base, c))).collect::<Option<_>>()?;
        Some((base.to_owned(), src, cols))
    }

    /// Stratum `scc` (`rules`) as a view of an input's columns (`input_view`): its relation, and what it views.
    fn input_view_stratum(&self, scc: &super::front::Comp, rules: &[RuleKey]) -> Option<(String, String)> {
        let rel = self.view_shaped(scc, rules)?;
        let (base, _, cols) = self.input_view(rules[0])?;
        Some((rel.to_owned(), format!("{rel} = {base}{cols:?}")))
    }

    /// Stratum `scc` (`rules`) shaped as a view of an input's columns (`input_view_shape`), whether or not the input
    /// is bound yet (a file-local part's export is bound when its first reader runs): its relation.
    fn view_shaped<'s>(&self, scc: &'s super::front::Comp, rules: &[RuleKey]) -> Option<&'s str> {
        let f = self.fields();
        let (Some(rel), 1, [r]) = (scc.iter().next(), scc.len(), rules) else { return None };
        if f.has_facts(rel) || f.native(rel).is_some() || f.lattice(rel) || f.request(rel) || f.produced(rel) || f.gate(rel) {
            return None;
        }
        let (v, rule) = self.versions.rule_at(*r);
        input_view_shape(v, rule, |x| self.inputs.contains(x)).map(|_| rel)
    }

    /// T's `native_stratum`: plans with the stratum as the component being solved when a rule reads it. The stratum
    /// may come back waiting on its device wave (`Native::Waiting`); `native_resume` finishes it once the wave ran.
    /// Whether a native join reads the view `r` from its input in place (`projections`): no SQL view is made for it
    /// unless SQL reads it.
    fn in_place(&self, r: &str) -> bool {
        self.projections.contains_key(r) && self.model.view_src.get(r).is_some_and(|v| v.1.iter().all(|c| c.is_some()))
    }

    /// Why a stratum cannot take the maintain mode (decision 0364 stage 3), or `None` when it can: one relation, no
    /// rule of it reading it (not recursive), each rule's body joins, negations and comparisons with a head of
    /// variables and constants, its kept rows in memory, and each read that changed (`moved`) a native relation
    /// that started from its kept one this run (it has a version).
    /// The static check's verdict on recursive component `scc` (decision 0364 §5).
    fn proof(&self, scc: &super::front::Comp) -> std::result::Result<wellfounded::Measure, String> {
        let proofs = match self.proofs.get() {
            Some(p) => p,
            None => {
                let made = super::program_image::manager::analysis::<wellfounded::Proofs>(&self.versions.placed(self.version).v)?;
                self.proofs.get_or_init(|| made)
            }
        };
        proofs.of(scc)
    }
    /// The columns of changed read `rel` that stratum `rules` reads: of each atom over it, the arguments but `_`. `None`
    /// when an aggregate reads it, whose groups count its rows.
    fn read_columns(&self, rules: &[RuleKey], rel: &str) -> Option<BTreeSet<usize>> {
        let mut used = BTreeSet::new();
        for &r in rules {
            let b = self.versions.at(r);
            for i in 0..b.len() {
                match b.view(i) {
                    Lit::Atom(a) if b.rel(a) == rel => used.extend(a.args().enumerate().filter(|(_, t)| !matches!(t, TermRef::Wild)).map(|(c, _)| c)),
                    Lit::Agg { atom: a, .. } if b.rel(a) == rel => return None,
                    _ => {}
                }
            }
        }
        Some(used)
    }

    fn maintain_mode_blocks(&self, scc: &super::front::Comp, rules: &[RuleKey], moved: &BTreeSet<String>, olds: &BTreeMap<String, Arc<super::native::component::InRel>>, ins: &ModeInputs) -> Option<String> {
        if std::env::var("RERAIL_MAINTAIN_MODE").is_ok_and(|v| v == "off") {
            return Some("RERAIL_MAINTAIN_MODE=off".into());
        }
        // Nothing kept: its part's first run (a cold run keeps nothing, and its part is read from no frame), derived.
        if olds.is_empty() {
            return Some(crate::code_rule_dsl::logic::plan::again::FIRST_RUN.into());
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
        // Recursive: by levels (decision 0364 §2), every relation's kept rows with theirs; or proved by the static
        // check (§5), which keeps none.
        let recursive = rules.iter().any(|&r| { let b = self.versions.at(r); (0..b.len()).any(|i| matches!(b.view(i), Lit::Atom(a) if scc.contains(b.rel(a)))) });
        if recursive && !olds.values().all(|o| o.has_levels() || o.rows() == 0) && self.proof(scc).is_err() {
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
                    // a version, or a changed input's versions, given whole.
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
    /// 0090 §4's second clause (0364 R4): what maintaining stratum `rules` would cost against deriving it again, as the
    /// planner prices them. The mode's lines are each rule led by each changed read's delta (`deltas`: its rows gone
    /// and added), a removal and an add; a derive is each rule's seed plan over its reads whole. Both plans are the
    /// program's kept plans of those lines (`plan_rule_reused`), which the mode or the derive then takes as they are.
    fn mode_costs(&mut self, rules: &[RuleKey], deltas: &BTreeMap<String, usize>) -> Result<(f64, f64)> {
        let vs = Arc::clone(&self.versions);
        let pf = vs.whole(self.version);
        let bodies: Vec<Body<'_>> = rules.iter().map(|&r| vs.at(r)).collect();
        let mut wanted = BTreeSet::new();
        self.model.wants(bodies.iter().map(|b| (b, None)), &mut wanted);
        self.model.make_statistics(pf, &wanted)?;
        let (mut maintain, mut derive) = (0.0, 0.0);
        for b in &bodies {
            plan_site("the threshold's price of a derive (0090 §4)");
            let derived = self.model.plan_rule_reused(pf, b, None)?.0.1;
            derive += derived;
            for k in 0..b.len() {
                let Lit::Atom(a) = b.view(k) else { continue };
                let rel = b.rel(a);
                let Some(&n) = deltas.get(rel).filter(|n| **n > 0) else { continue };
                let was = self.model.stats.delta_rows.insert(rel, n as f64);
                plan_site("the threshold's price of the mode (0090 §4)");
                // Priced as the mode plans it (`maintain_mode`): a positive driver past the floor free to start
                // anywhere.
                let first = (!a.neg()).then_some(k);
                self.model.stats.lead_free = first.is_some() && crate::code_rule_dsl::logic::plan::again::past_floor(n);
                let mut priced = self.model.plan_rule_reused(pf, b, first).map(|p| p.0.1);
                // A kept order is run as it is while its work re-priced is under `REPLAN_ROWS`: a search could save
                // no more than that. Its price may still be far over the line's best, and here it is summed with
                // the stratum's others against the derive: a line priced over its rule's derive is searched, and
                // priced at its best. After ep2's append, three lines kept from its few moved lines were each
                // priced 9,836 at the import's 9,350 (their best, 10 to 60), and the mode's 30,377 against the
                // derive's 3,505 derived `violation` again, its finding's places making 33 documents.
                if let Ok(kept) = priced
                    && kept > derived
                    && let Ok(((_, best, _), _)) = self.model.plan_inner(pf, b, first, None, false)
                {
                    priced = Ok(kept.min(best));
                }
                self.model.stats.lead_free = false;
                match was {
                    Some(w) => self.model.stats.delta_rows.insert(rel, w),
                    None => self.model.stats.delta_rows.remove(rel),
                };
                // A removal and an add, each over its side of the delta: priced once over the whole delta.
                maintain += priced?;
            }
        }
        Ok((maintain, derive))
    }

    fn maintain_mode(&mut self, scc: &super::front::Comp, rules: &[RuleKey], moved: &BTreeSet<String>, olds: BTreeMap<String, Arc<super::native::component::InRel>>, ins: &ModeInputs) -> Result<Option<bool>> {
        let vs = Arc::clone(&self.versions);
        let pf = vs.whole(self.version);
        let h = scc.iter().next().expect("a relation");
        let ri: BTreeMap<String, usize> = scc.iter().enumerate().map(|(i, r)| (r.to_owned(), i)).collect();
        let native_rels = self.native_rels.clone();
        let types: BTreeMap<String, String> = scc.iter().map(|r| (r.to_owned(), pf.types(r).iter().map(|t| sym_char(&self.model.inputs, t)).collect())).collect();
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
            lines.push(SpecLine::Rel { name: r.to_owned(), types: types[r].to_owned(), start: Start::Base { old: r.to_owned(), gone: "-".into(), seeded: false } });
        }
        lines.push(SpecLine::NoWrite);
        let recursive = bodies.iter().any(|b| (0..b.len()).any(|i| matches!(b.view(i), Lit::Atom(a) if scc.contains(b.rel(a)))));
        if recursive {
            // The static check (0364 §5, R6): a component that cannot hold a row up by itself keeps no levels, its
            // candidates rechecked once each in its measure's order: by the depth or document order of the node at
            // each relation's measure column (read from `ast_depth`, `node_order`), or by its index.
            let proved = self.proof(scc);
            let shown = proved.as_ref().map_or_else(|w| format!("not proved: {w}"), |m| format!("proved, {:?}", m.order));
            super::profile::decision("the static check (decision 0364 §5)", || format!("{}: {shown}", stratum_key(scc)));
            if std::env::var_os("RERAIL_MODE_TRACE").is_some() {
                eprintln!("RERAIL_MODE_TRACE {}: the static check: {shown}", stratum_key(scc));
            }
            match proved {
                Ok(m) => {
                    use wellfounded::Order;
                    // A node's key is read where the run has it bound: a part that holds no tree (a cross part
                    // whose tree steps were pushed into the chunks, decision 0051) has not, and its candidates go in
                    // id order, each checked again when a row it may read goes (`Measured::keyed`).
                    let read = |rel: &str| crate::code_rule_dsl::facts::vtab::bound_source(self.db, rel).map(|_| rel.to_owned());
                    let (desc, key) = match m.order {
                        Order::Down => (false, read("ast_depth")),
                        Order::Up => (true, read("ast_depth")),
                        Order::Back => (true, read("node_order")),
                        Order::Climbs | Order::Same => (false, None),
                        Order::Falls => (true, None),
                    };
                    let keyed = key.is_some() || matches!(m.order, Order::Climbs | Order::Falls | Order::Same);
                    if !keyed {
                        crate::code_rule_dsl::logic::run::shape::measure_unkeyed();
                    }
                    let cols = scc.iter().map(|r| m.col.get(r).copied().unwrap_or(0)).collect();
                    let ranks = scc.iter().map(|r| m.rank.get(r).copied().unwrap_or(0) as usize).collect();
                    lines.push(SpecLine::Measured { desc, key, keyed, cols, ranks });
                }
                // Its rows keep levels.
                Err(_) => lines.push(SpecLine::Leveled((0..scc.len()).collect())),
            }
            // Its kept rows are no wave's delta: the first wave's is what the removals put back.
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
            let lit_of = |from: &[u32], order: &[Step], item: usize| -> Option<usize> {
                let step = if from.is_empty() { item } else { *from.get(item)? as usize };
                order.get(step).map(|s| s.at)
            };
            // `spec` with each changed atom read in its version: `drive` at `k` (as `dver`, positive), the others `rest`.
            // A rewrite of the join (decision 0628 §4): its view made a value again, then encoded as the new join.
            let versioned = |spec: &PlanJoin, order: &[Step], k: usize, dver: super::native::component::spec::Ver, rest: super::native::component::spec::Ver| -> JoinSpec {
                let mut out = <JoinSpec as rerail_section::Decode>::decode(spec, &rerail_section::Names::NONE);
                for (i, item) in out.items.iter_mut().enumerate() {
                    let Some(at) = lit_of(&spec.from, order, i) else { continue };
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
                // A positive driver is planned first, as a delta is. A negated one leads its line too, its delta read
                // positively: the body is planned with its variables bound (`Body::with_bound`), and its own item
                // goes to the front. Left where the order tests it, the line walked its other atoms over the whole
                // corpus to meet a delta of a few rows: on the data layer, 113 removal joins took 10 s of a 2-file
                // edit's 31 s at 334 files, and as much as a first run at 2,410.
                let first = (!a.neg()).then_some(k);
                for (by, n, removing) in [(remove_by, remove_n, true), (add_by, add_n, false)] {
                    if n == 0 {
                        continue;
                    }
                    self.model.stats.delta_rows.insert(rel, n as f64);
                    // The program's kept plan of this line, as a semi-naive probe's (decision 0364 stage 4): searched
                    // on its first sight, then taken or priced again as its sizes move (decisions 0038, 0122).
                    plan_site("the maintain mode's join");
                    // A negated driver leads its line too: the body planned with its variables bound by the delta's rows
                    // (`Body::with_bound`), kept apart as a recheck's is; its item goes first (below).
                    let led_body = a.neg().then(|| b.with_bound(b.vars(k).clone()));
                    let led = match &led_body {
                        Some(lb) => self.model.plan_rule_reused(pf, lb, None).ok().map(|((o, _, _), _)| o),
                        None => None,
                    };
                    // The line's order: the driver first, then the rest as planned from its variables bound.
                    let led = led.map(|o| std::iter::once(Step { at: k, mode: None }).chain(o.into_iter().filter(|st| st.at != k)).collect::<Vec<_>>());
                    // A positive driver's line is a whole join (`Line::Remove`, `Line::Seed`), its delta probed
                    // through its own index where another atom binds it (`Source::Version`): past 0090 §4's floor,
                    // sized by its rows, it leads only where it is the cheapest start (`Stats::lead_free`). Led by
                    // it, a one-row finding read after 4,672 moved lines was each of 163 rules' lines in ep2's import
                    // edit, and their price (1.9 M) kept `violation` derived again: 33 documents made for their
                    // places (0.19 s). Under the floor a line costs little whatever its order, and keeps the
                    // program's (its prepared and kept plans): planned free, ep2's appends' lines cost 1 G more.
                    let (order, key) = match (&led, &led_body) {
                        (Some(o), Some(lb)) => (o.clone(), self.model.reused_key(lb, None)),
                        _ => {
                            // The relation's change, both sides, as the clause's price reads it (`mode_costs`).
                            self.model.stats.lead_free = crate::code_rule_dsl::logic::plan::again::past_floor(gone + added);
                            let planned = self.model.plan_rule_reused(pf, b, first).map(|p| (p.0.0, self.model.reused_key(b, first)));
                            self.model.stats.lead_free = false;
                            planned?
                        }
                    };
                    // The changed inputs' literals read their versions: the driver's, and as they were in a removal.
                    self.mode_reads.clear();
                    // An aggregate's atom too: read where the rule's other atoms are, as it was in a removal. Unnamed here, it
                    // read the relation's own versions, which a relation derived again past θ (R4) may not have: its
                    // delta against its kept rows is given (`kept_delta`).
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
                    // The negated driver's item, first in the order, reads its delta positively (`versioned`).
                    let spec = versioned(&spec, &order, k, by, if removing { V::Old } else { V::New });
                    match (&led, a.neg()) {
                        (Some(_), _) if matches!(spec.items.first(), Some(JoinItem::Atom { neg: false, .. })) && lit_of(&spec.from, &order, 0) == Some(k) => {
                            crate::code_rule_dsl::logic::run::shape::negated_led()
                        }
                        (_, true) => crate::code_rule_dsl::logic::run::shape::driver_unled(),
                        _ => {}
                    }
                    let spec = spec.encode();
                    let (line, kind) = match (removing, a.neg()) {
                        (true, _) => (SpecLine::Remove { head: head_i, join: spec }, ModeLine::Remove),
                        (false, false) => (SpecLine::Seed { head: head_i, join: spec, est: Default::default() }, ModeLine::Probe),
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
                    // The group key: the aggregate atom's variables the rule's other literals share. The delta atom
                    // that leads the line binds them, so the rest is planned from them bound (`Body::with_bound`), and
                    // the spec holds their slots before its first item, which that atom then fills.
                    let mut group = VarSet::new();
                    for v in body::atom_vars(a).iter() {
                        if (0..b.len()).any(|i| i != k && b.vars(i).contains(v)) {
                            group.insert(v);
                        }
                    }
                    let grouped = b.with_bound(group);
                    for removing in [true, false] {
                        plan_site("the maintain mode's aggregate join");
                        let led = self.model.plan_rule_reused(pf, &grouped, None).ok().map(|((o, _, _), _)| o);
                        let (order, key, body) = match led {
                            Some(o) => (o, self.model.reused_key(&grouped, None), &grouped),
                            None => (self.model.plan_rule_reused(pf, b, None)?.0.0, self.model.reused_key(b, None), b),
                        };
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
                        let made = self.whole_join_spec(body, &order, None, &ri, scc, &native_rels);
                        if made.is_none() || !std::ptr::eq(body, &grouped) {
                            crate::code_rule_dsl::logic::run::shape::driver_unled();
                        }
                        self.mode_reads.clear();
                        let Some(spec) = made else {
                            self.why_not = Some(format!("the maintain mode: no native aggregate join for a rule of {h}"));
                            return Ok(None);
                        };
                        let mut spec = versioned(&spec, &order, usize::MAX, V::New, if removing { V::Old } else { V::New });
                        let Some(at) = (0..spec.items.len()).find(|&i| lit_of(&spec.from, &order, i) == Some(k) && matches!(spec.items[i], JoinItem::Count { .. } | JoinItem::Extreme { .. } | JoinItem::StrJoin { .. })) else {
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
                        let spec = spec.encode();
                        lines.push(if removing { SpecLine::Remove { head: head_i, join: spec } } else { SpecLine::Seed { head: head_i, join: spec, est: Default::default() } });
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
                    crate::code_rule_dsl::logic::run::shape::recheck_unpriced();
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
                lines.push(SpecLine::Join { head: head_i, delta: ri[b.rel(a)], join: spec, whole: None, est: Default::default() });
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
        let spec = ComponentSpec { name: name.clone(), lines }.encode();
        let fx = Fx { db: self.db, direct: true };
        for r in scc {
            super::native::component::hold_maintained(self.db, r, Arc::clone(&olds[r]));
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
        self.native_rels.extend(scc.iter().map(str::to_owned));
        // Nothing deleted, nothing derived: quiet, as a stratum whose joins all read empty deltas is.
        if scc.iter().all(|r| super::native::component::held(self.db, r).and_then(|x| x.version_counts()) == Some((0, 0))) {
            self.quiet.strata.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        run_counts().mode_strata.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        super::profile::stratum(&stratum_key(scc), "native set engine", || "the maintain mode (decision 0364)".to_owned());
        Ok(Some(true))
    }

    fn native_stratum(&mut self, scc: &super::front::CompAt, rules: &[RuleKey]) -> Result<Native> {
        // `RERAIL_STRATA_LIVE`: each native stratum as it starts and ends, so a stalled run names its stratum (the
        // trace, `RERAIL_STRATA_TRACE`, is written when the loop ends).
        let t0 = rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var_os("RERAIL_STRATA_LIVE").is_some()).then(|| {
            eprintln!("RERAIL_STRATA_LIVE start {scc:?} ({} rules)", rules.len());
            std::time::Instant::now()
        });
        // SPIKE: a stratum evaluated over the chunk's tree and held relations (`spike_eval`).
        let evaluated = if spike_eval::enabled() { self.spike_eval(scc, rules)? } else { None };
        if evaluated.is_some() && !spike_strata::check() {
            for (rel, mut h) in evaluated.into_iter().flatten() {
                // The engine's copy made, the chunk reads it where it lies: one copy of its rows.
                self.spike_retain_distinct(&rel, &h)?;
                if let Some(src) = super::native::component::derived(self.db, &rel)
                    && !h.lend_to(&src)
                    && std::env::var_os("RERAIL_SPIKE_WHY").is_some()
                {
                    let held = super::native::component::held_rows(&*src).map(|(rows, n, dead, _)| (n, dead.len(), (0..h.k).map(|c| rows.range_reader(0, n as u32, c).is_some()).collect::<Vec<_>>()));
                    eprintln!("SPIKE LEND {rel}: {} rows not lent: engine's {held:?}", h.n);
                }
                self.spike_hold(&rel, h);
            }
            return Ok(Native::Ran);
        }
        // SPIKE: a stratum of the reference library run by W's algorithm (`spike_strata`).
        let spiked = if spike_strata::enabled() && spike_strata::handles(&scc.iter().collect::<Vec<_>>()) { self.spike_rows(scc) } else { None };
        if spiked.is_some() && !spike_strata::check() {
            for (rel, rows) in spiked.iter().flatten() {
                self.spike_retain(rel, rows)?;
            }
            return Ok(Native::Ran);
        }
        let solving = solving_of(&self.versions, scc, rules);
        let sources = if solving.is_some() { column_sources(&self.versions, scc, rules) } else { Sources::new() };
        let out = self.lent(solving, sources, |this, native_rels| this.native_stratum_(scc, rules, native_rels));
        if let (Some(made), Ok(Native::Ran)) = (&evaluated, &out) {
            let tree = self.spike_tree();
            let chunk = tree.as_ref().map(|(b, t)| spike_strata::Chunk::new(t, Some(b)));
            for (rel, h) in made {
                let generic = super::native::component::derived(self.db, rel).map(|s| spike_strata::rows(&*s)).unwrap_or_default();
                spike_strata::compare(rel, &spike_strata::Flat { k: h.k, v: h.own().to_vec() }, &generic, chunk.as_ref());
                // Where the engine found a node the spike's tree does not hold: the `node` or `kind` it read.
                if generic.k > 0 && h.k == generic.k {
                    let ours: HashSet<&[i64]> = h.own().chunks_exact(h.k).collect();
                    let missing = generic.v.chunks_exact(generic.k).filter(|r| !ours.contains(r)).take(2);
                    for row in missing {
                        for &v in row.iter().filter(|&&v| v > 1 << 32) {
                            eprintln!("SPIKE WHENCE {rel} {v}: {}", self.spike_whence(v));
                        }
                    }
                }
            }
        }
        // Check mode: the spike's rows held for the strata after it, as they are.
        if matches!(out, Ok(Native::Ran)) {
            for (rel, h) in evaluated.into_iter().flatten() {
                self.spike_hold(&rel, h);
            }
        }
        if let (Some(made), Ok(Native::Ran)) = (&spiked, &out) {
            let tree = self.spike_tree();
            let chunk = tree.as_ref().map(|(b, t)| spike_strata::Chunk::new(t, Some(b)));
            for (rel, rows) in made {
                let generic = super::native::component::derived(self.db, rel).map(|s| spike_strata::rows(&*s)).unwrap_or_default();
                spike_strata::compare(rel, rows, &generic, chunk.as_ref());
            }
        }
        if let Some(t0) = t0 {
            eprintln!("RERAIL_STRATA_LIVE end {scc:?} {:.3} s", t0.elapsed().as_secs_f64());
        }
        out
    }

    /// Resumes a stratum that waited on its device wave, after the wave ran: its driver goes on from the run that
    /// suspended it.
    fn native_resume(&mut self, d: Box<Driving>) -> Result<Native> {
        let solving = d.solving.then(|| rerail_section::TextSet::of(d.scc.iter()));
        let sources = d.sources.clone();
        self.lent(solving, sources, |this, native_rels| this.drive(d, native_rels))
    }

    /// `native_stratum`, finished here: a stratum that waits on its device wave has the wave flushed (with whatever
    /// else it holds, whose outputs stay for their strata) and is resumed until it ran. Whether it ran natively. For
    /// a stratum its caller needs at once (one derived again, 0090 §4).
    fn native_stratum_now(&mut self, scc: &super::front::CompAt, rules: &[RuleKey]) -> Result<bool> {
        // Needed at once: its requests are answered in place (`answer`), not posted.
        let asking = self.asking.take();
        let got = self.native_stratum_now_(scc, rules);
        self.asking = asking;
        got
    }

    fn native_stratum_now_(&mut self, scc: &super::front::CompAt, rules: &[RuleKey]) -> Result<bool> {
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

    /// Runs `f` with the relations run natively so far lent to it, and `solving` as the component being solved, its
    /// columns' `sources` (decision 0658).
    fn lent<T>(&mut self, solving: Option<rerail_section::TextSet>, sources: Sources, f: impl FnOnce(&mut Self, &BTreeSet<String>) -> Result<T>) -> Result<T> {
        self.model.stats.solve(solving.unwrap_or_default());
        self.model.stats.sourced(sources);
        // The relations run natively so far, lent to the stratum (it reads them, and adds its own only at the end):
        // cloning them per stratum was quadratic in strata (3% of a combined run's main thread).
        let native_rels = std::mem::take(&mut self.native_rels);
        let out = f(self, &native_rels);
        let added = std::mem::replace(&mut self.native_rels, native_rels);
        self.native_rels.extend(added);
        self.model.stats.solve(Default::default());
        out
    }

    /// One stratum on the native set engine, as a component of seed joins (rules without a recursive atom, run once)
    /// and join probes (one per recursive atom, planned with its delta first), all planned by `plan_rule` and
    /// replanned from live counts between chunks of waves (T's `native_stratum_`). Lower strata it reads natively
    /// come from the native store (`r:`), others from their tables or the facts. Nothing is written back: the
    /// relations join `native_rels`. False (and nothing run) where a rule has a literal with no native step.
    fn native_stratum_(&mut self, scc: &super::front::CompAt, rules: &[RuleKey], native_rels: &BTreeSet<String>) -> Result<Native> {
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
            && let Some(super::program_image::ArchivedNativeSpec::TextTest { input, column, op, patterns }) = f.native(rel).map(|n| &n.spec)
            && text_test_defined(&self.versions, rules, f.rel_name(*input))
        {
            if !placement::device_on() {
                let (input, op, patterns) = (f.rel_name(*input).to_owned(), f.text(*op).to_owned(), patterns.iter().map(|&p| f.text(p).to_owned()).collect::<Vec<_>>());
                if self.native_text_test(rel, &input, native_rels.contains(&input), *column as usize, &op, &patterns)?.is_some() {
                    return Ok(Native::Ran);
                }
                self.why_not = Some("a text test whose input the engine does not hold".into());
                self.why_class = Some("native fact operator".into());
                return Ok(Native::Declined);
            }
        // A text test whose rules are not its definition (a maintained form's relation: its old rows and its new
        // ones) is those rules, run below.
        } else if scc.iter().any(|r| f.native(r).is_some_and(|n| !matches!(n.spec, super::program_image::ArchivedNativeSpec::TextTest { .. }))) {
            self.why_not = Some("a native fact operator over input relations".into());
            self.why_class = Some("native fact operator".into());
            return Ok(Native::Declined);
        }
        let rels: Vec<&str> = scc.iter().collect();
        let ri: BTreeMap<String, usize> = rels.iter().enumerate().map(|(i, r)| ((*r).to_owned(), i)).collect();
        let name = format!("prog_{}", rels[0]);
        let mut gates: Vec<(&str, &super::program_image::ArchivedGateRef)> = f.gates().iter().map(|g| (f.text(g.key), g)).filter(|(g, _)| scc.contains(*g)).collect();
        gates.sort_by(|a, b| (a.1.level, a.0).cmp(&(b.1.level, b.0)));
        // A gate's rule only placed it here.
        let rules: Vec<RuleKey> = rules.iter().copied().filter(|&r| !gates.iter().any(|(g, _)| *g == vs.head_at(r))).collect();
        // The rules as the program writes them, before a run leaves out those over empty relations: a view of an input
        // is one by its program, not by a run's rows.
        let written = rules.len();
        // A rule that joins a relation of a lower stratum holding no rows derives nothing, so it is neither planned
        // nor given to the engine. In a maintained run most delta rules read a delta that is empty this time, and
        // planning, keying and specifying them was work in the program's size, not the delta's (0090 step 5,
        // 0094 §4). Only a positive atom counts: a negation of it holds, and an aggregate over no rows is a row.
        // A relation a request may fill during the stratum is never taken as empty. `RERAIL_KEEP_EMPTY_JOINS` runs
        // them all (to measure what leaving them out saves).
        let rules: Vec<RuleKey> = if std::env::var_os("RERAIL_KEEP_EMPTY_JOINS").is_some() { rules } else {
            let fillable: BTreeSet<&str> = f.answers().iter().map(|&(_, p)| f.rel_name(p)).flat_map(|p| [Some(p), f.request_of(p)]).flatten().collect();
            // A program that answers no request grows no source while it runs (a tape value's rows are admitted as a
            // request asks, decision 0431 §8): an input it has bound already holds all its rows. The cross-file part's
            // inputs are such, the file-local part's exports (a context's, with no root: 0597 rung 2.10).
            let asks_nothing = f.answers().is_empty();
            let mut empty: HashMap<String, bool> = HashMap::new();
            let mut is_empty = |rel: &str| -> bool {
                if scc.contains(rel) || fillable.contains(rel) {
                    return false;
                }
                // An input may be bound when its first reader runs, so it is not taken as empty unless it is bound
                // already, in a program that asks nothing.
                if !native_rels.contains(rel) {
                    return asks_nothing && self.inputs.contains(rel) && crate::code_rule_dsl::facts::vtab::bound_source(self.db, rel).is_some_and(|s| !s.may_hold_rows());
                }
                if let Some(&e) = empty.get(rel) {
                    return e;
                }
                let e = match super::native::component::held(self.db, rel) {
                    Some(h) => h.holds_none(),
                    None => !native_rels.contains(rel) && crate::code_rule_dsl::facts::vtab::bound_source(self.db, rel).is_some_and(|s| !s.may_hold_rows()),
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
            // derives it; no facts, request, answer or gate writes it) stays empty, so a rule joining it
            // derives nothing either; until none is left out. In a maintained run `pcall__ins`'s rules all joined
            // empty input deltas, and the probes of `pfn__ins :- pcall__ins(…), pv(…), fv(…)` by `pv`'s and `fv`'s
            // deltas read 341,837 rows each before the empty `pcall__ins` rejected them (O125 step 06).
            let written: BTreeSet<&str> = gates.iter().map(|(g, _)| *g)
                .chain(fillable.iter().copied())
                .chain(scc.iter().filter(|r| f.has_facts(r)))
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
        let typed: HashMap<String, String> = rels.iter().map(|r| ((*r).to_owned(), f.types(r).iter().map(|t| sym_char(&self.model.inputs, t)).collect::<String>())).collect();
        let types = |r: &str| typed[r].clone();
        // Nothing left to run (every rule joined an empty relation, and no relation of the stratum has facts): the
        // stratum's relations are held with no rows, and no component is specified or opened. A relation with facts
        // (`d("a"). d(y) :- d(x), e(x, y).` with `e` empty) holds its facts, which the component seeds.
        // `RERAIL_KEEP_EMPTY_JOINS` opens it as before.
        let asks = scc.iter().any(|r| f.request(r) || f.produced(r));
        let facts = scc.iter().any(|r| f.has_facts(r));
        if rules.is_empty() && gates.is_empty() && !asks && !facts && std::env::var_os("RERAIL_KEEP_EMPTY_JOINS").is_none() {
            for r in &rels {
                super::native::component::hold_empty(self.db, r, types(r).into_bytes());
            }
            super::profile::stratum(&stratum_key(scc), "native set engine", || "every rule joins an empty relation: held empty, no component opened".to_owned());
            self.native_rels.extend(scc.iter().map(str::to_owned));
            return Ok(Native::Ran);
        }
        // A copy of a bound input's columns (Q37: `kind(n, k) :- node(_, n, k, _, _)`, 3.5 M rows a cold run of
        // hogql): its relation is a view of the input's columns, read in place (`hold_view_in_place`), and no
        // component copies it. `RERAIL_COPY_VIEWS` derives it as before (to measure what the view saves). Only a stratum whose
        // program makes it a view (`written`): one whose other rules join relations empty this run (`cparam`: a
        // file-local export's copy and a cross-file rule) is derived, kept, and maintained.
        if let ([rel], [r]) = (rels.as_slice(), rules.as_slice())
            && written == 1
            && gates.is_empty() && !asks && !facts
            && let Some((base, src, cols)) = self.input_view(*r)
        {
            // Held in place (decisions 0329, 0353): no row read, none copied. Its statistics are counted when a plan
            // first reads them (`InPlaceView`, `SourceView`), unless its base's are held.
            let n = self.hold_view_in_place(&base, rel, &src, &cols, types(rel).into_bytes());
            let form = "held in place";
            if std::env::var_os("RERAIL_SHAPE_TRACE").is_some() {
                let (v, rule) = vs.rule_at(*r);
                let body: Vec<&str> = rule.body().filter_map(|l| l.atom()).map(|a| v.rel_name(a.rel())).collect();
                eprintln!("RERAIL_SHAPE_TRACE {rel}: a view of {base}{cols:?} ({form}; its rule reads {body:?}, {n} rows)");
            }
            run_counts().input_views.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            super::profile::stratum(&stratum_key(scc), "native set engine", || format!("a view of {base}'s columns {cols:?}: {form}, no component opened"));
            self.native_rels.extend(scc.iter().map(str::to_owned));
            return Ok(Native::Ran);
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
        // Its rows keep levels where a session maintains it by them (decision 0090, the `levels` qualifier), unless
        // the static check proves it needs none (decision 0364 §5).
        if scc.iter().any(|r| f.qualified(r, "levels")) && self.proof(scc).is_err() {
            lines.push(SpecLine::Leveled(rels.iter().enumerate().filter(|(_, r)| cyclic.contains(**r)).map(|(i, _)| i).collect()));
        }
        // A component that answers requests (decision 0065 §2a) cannot fall back to SQL whole: a rule with no native
        // step is a SQL seed, run before the component opens, or a SQL probe over its relations' `set_rel` views.
        let answering = scc.iter().any(|r| f.request(r));
        let views: HashMap<String, (String, String)> = rels.iter().map(|r| ((*r).to_owned(), (format!("nsx_f_{r}"), format!("nsx_d_{r}")))).collect();
        let mut sql_seeded: BTreeSet<String> = BTreeSet::new();
        let mut sql_probe: Vec<bool> = Vec::new();
        // Requests and what answers them (decision 0065 §2a): the run pauses for the first, and adds to the second.
        for r in &rels {
            if f.request(r) && scc.iter().any(|x| f.request_of(x) == Some(r)) {
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
        // Each relation's rows as its seeds are estimated to derive them.
        let mut est: HashMap<&str, f64> = HashMap::new();
        let mut orders = Vec::new();
        // Each line's kept-plan key (`Driving::keys`).
        let mut keys: Vec<Option<PlanKey>> = Vec::new();
        // Each order's rule, for the fanouts its runs observe (`obs_rule`).
        let mut order_rules: Vector<u64> = rerail_blocks::vector::Vector::new();
        let seeds: Vec<RuleKey> = rules.iter().copied().filter(|&r| {
            let b = vs.at(r);
            recursive(&b, scc).is_empty()
        }).collect();
        // A relation that is another less a few rows (`n(xs) :- x(xs), !g(xs)`, both held by the engine, `g` the
        // smaller by far: a narrow copy and its rows gone, decision 0575) starts as `x`'s rows and orders, shared, `g`'s
        // marked dead in place (`Rel::based`); its other rules add to it. Its readers read it in one literal, and no
        // row of `x` is copied or derived again.
        let based: Option<(String, String, String, RuleKey)> = (rels.len() == 1 && !f.has_facts(rels[0]) && !f.produced(rels[0])
            && std::env::var_os("RERAIL_NO_BASED").is_none())
            .then(|| seeds.iter().find_map(|&r| based_rule(&vs, r, &|rel: &str| native_rels.contains(rel) && !scc.contains(rel), &|rel: &str| super::native::component::held(self.db, rel).map(|h| h.rows()))))
            .flatten();
        let seeds: Vec<RuleKey> = seeds.into_iter().filter(|&r| based.as_ref().is_none_or(|b| b.3 != r)).collect();
        if let Some((h, x, g, _)) = &based {
            super::profile::decision("a relation started from another's rows", || format!("{h}: {x}'s rows and orders, less {g}'s, marked dead in place"));
        }
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
            (1..=steps.len()).find(|&k| this.join_spec(b, &steps[..k], true, at.filter(|&a| a < k), &ri, scc, native_rels).is_none())
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
            if std::env::var("RERAIL_PLAN_SHOW").is_ok_and(|w| scc.iter().any(|r| w.split(',').any(|n| n == r))) {
                eprintln!("plan show seed of {scc:?} head {}: work {work:.1} rows {rows:.1}: {}", b.show_head(), b.shown(&order).join(" , "));
            }
            let took = if profiling { super::profile::now() - t0 } else { 0.0 };
            drop(span);
            shape.add(Order::new(b, &order), work, false);
            shape.seeded(head, rows);
            if let Some(x) = &x {
                shape.add_joins(Order::new(b, &order), x, |rel| self.model.stats.card.get(rel).copied().unwrap_or(0.0), |rel, col| self.model.stats.lookup_cost(rel, col), |rel| !self.model.derived.contains(rel));
            }
            let Some(spec) = self.whole_join_spec(b, &order, None, &ri, scc, native_rels) else {
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
            let mut priced = super::native::setfix::spec::Est::default();
            if let Some(x) = &x {
                expect_axis_lookups(self.db, x);
                priced = step_estimates(x);
            }
            explained.push((format!("seed {head}"), rule_no(r), x, took, 0));
            keys.push(Some(self.model.reused_key(b, None)));
            *est.entry(head).or_default() += rows;
            lines.push(SpecLine::Seed { head: ri[head], join: spec, est: priced });
            orders.push((r, order));
            order_rules.push(b.observed_key());
        }
        // A stratum's relations were made empty by `run` unless the program states facts of them or a SQL seed derived
        // some. A produced input (decision 0065 §2a) starts from its rows where it is.
        let mut rel_lines = Vec::new();
        for r in &rels {
            if f.produced(r) && self.inputs.contains(r) {
                let cols = (0..f.width(r)).map(|c| self.model.col(r, c)).collect::<Vec<_>>().join(", ");
                let sql = format!("SELECT {cols} FROM {}", self.model.relation(pf, r, false).render());
                rel_lines.push(SpecLine::Rel { name: (*r).to_owned(), types: types(r), start: Start::From(sql) });
            } else if let Some((_, x, g, _)) = based.as_ref().filter(|b| b.0 == **r) {
                rel_lines.push(SpecLine::Rel { name: (*r).to_owned(), types: types(r), start: Start::Base { old: x.clone(), gone: g.clone(), seeded: sql_seeded.contains(*r) } });
            } else {
                let empty = !f.has_facts(r) && !sql_seeded.contains(*r);
                rel_lines.push(SpecLine::Rel { name: (*r).to_owned(), types: types(r), start: if empty { Start::Empty } else { Start::Rows } });
            }
        }
        // A lattice relation's key and join, after the relations (decision 0124).
        for r in &rels {
            let Some(l) = f.lattice_decl(r) else { continue };
            rel_lines.push(SpecLine::Lattice { rel: ri[*r], keys: l.keys, name: f.text(l.lattice).to_owned(), widen: l.widen });
        }
        // The keys the layouts give the stratum's own relations (decision 0361 step 6): checked where the rows are
        // written, a second row of a key reported (`key_breaks`). A relation the run found breaking a fact has no
        // facts left in the model's shapes, so none.
        for r in rels.iter().filter(|r| scc.contains(r)) {
            let Some(facts) = self.model.stats.shapes.facts(r) else { continue };
            for fact in facts.iter() {
                if let crate::code_rule_dsl::logic::front::ArchivedLayoutFact::Key(cols) = fact {
                    rel_lines.push(SpecLine::Key { rel: ri[*r], cols: Vector::from(cols.as_slice()) });
                }
            }
        }
        // The relations the run keeps past it (a chunk's exports): in blocks of their own, so a reader that holds
        // them where they lie does not keep the run's other relations' blocks.
        let kept_rows: Vector<usize> = rels.iter().filter(|r| self.model.inputs.kept.contains(**r)).map(|r| ri[*r]).collect();
        if !kept_rows.is_empty() {
            rel_lines.push(SpecLine::KeptRows(kept_rows));
        }
        // The relations declared sorted (decision 0456 K7): laid out in canonical order when done.
        let sorted: Vector<usize> = rels.iter().filter(|r| f.qualified(r, "sorted")).map(|r| ri[*r]).collect();
        if !sorted.is_empty() {
            rel_lines.push(SpecLine::Sorted(sorted));
        }
        lines.splice(rel_at..rel_at, rel_lines);
        let nseeds = orders.len();
        for r in &rels {
            let seeded = est.get(r).copied().unwrap_or(0.0).max(1.0);
            let card = prior_stats(&mut self.model, pf, r).map_or(seeded, |n| n.max(seeded));
            self.model.stats.card.insert(*r, card);
        }
        // Round 2 (decision 0123): the probes' statistics, and the whole plans' where one is made (below).
        let whole_wanted = |this: &Self, a: &str| !(plan_budget() && this.model.card(a) < SEARCH_WORK);
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
            let mut whole_spec: Option<Option<PlanJoin>> = None;
            for (ai, a) in recursive(b, scc) {
                if let Some(key) = lazy.remove(&(r, ai)) {
                    explained.push((format!("join {head} (delta {a})"), rule_no(r), None, 0.0, 0));
                    lines.push(SpecLine::Unplanned { head: ri[head], delta: ri[a] });
                    keys.push(Some(key));
                    probes.push((r, ai, Vec::new()));
                    sql_probe.push(false);
                    unplanned.push(true);
                    run_counts().lazy[0].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
                shape.add(Order::new(b, &order), 0.0, true);
                shape.probe(a, work, rows, delta);
                if let Some(x) = &x {
                    shape.add_joins(Order::new(b, &order), x, |rel| self.model.stats.card.get(rel).copied().unwrap_or(0.0), |rel, col| self.model.stats.lookup_cost(rel, col), |rel| !self.model.derived.contains(rel));
                }
                let at = order.iter().position(|s| s.at == ai).expect("the delta atom");
                let Some(spec) = self.whole_join_spec(b, &order, Some(at), &ri, scc, native_rels) else {
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
                let mut priced = super::native::setfix::spec::Est::default();
                if let Some(x) = &x {
                    expect_axis_lookups(self.db, x);
                    priced = step_estimates(x);
                }
                explained.push((format!("join {head} (delta {a})"), rule_no(r), x, took, 0));
                // The wave whose delta is all of the relation (it was empty before): any order, the atom read in full.
                // Not, under the planning budget (decision 0122), one whose relation is expected to start with fewer
                // rows than a search costs: the probe's own order reads them.
                if !whole_wanted(self, a) {
                    lines.push(SpecLine::Join { head: ri[head], delta: ri[a], join: spec, whole: None, est: priced });
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
                let wspec = if whole[0].at != ai {
                    whole_spec.get_or_insert_with(|| self.whole_join_spec(b, whole, None, &ri, scc, native_rels)).clone()
                } else {
                    None
                };
                if wspec.is_some() { wholes.insert(probes.len(), whole.clone()); }
                lines.push(SpecLine::Join { head: ri[head], delta: ri[a], join: spec, whole: wspec, est: priced });
                keys.push(Some(key));
                probes.push((r, ai, order));
                sql_probe.push(false);
            }
        }
        for (g, d) in &gates {
            let quiet: Vector<usize> = d.quiet.iter().filter_map(|&r| ri.get(f.rel_name(r))).copied().collect();
            let after: Vector<usize> = d.deps.iter().filter_map(|&x| gates.iter().position(|(g2, _)| *g2 == f.rel_name(x))).collect();
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
            let cpu = { let (u, s) = crate::code_rule_dsl::prof::thread_usage(); u + s };
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
            let due: Vector<usize> = (0..probes.len()).filter(|&k| unplanned[k]).collect();
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
                    *l = SpecLine::Join { head: h, delta: d, join, whole: None, est: Default::default() };
                }
            }
            spec.lines.insert(0, SpecLine::Device);
            None
        };
        // The spec, encoded once now that its lines are final (decision 0628 §4): what the engine opens, the lab
        // captures and the driver holds.
        let spec = spec.encode();
        let cpu_at = if profiling { let (u, s) = crate::code_rule_dsl::prof::thread_usage(); u + s } else { 0.0 };
        let counted: Vec<Vec<(u64, u64)>> = Vec::new();
        // The relational lab (decision 0095, step 0): the stratum's sources and seeds kept to run it again alone.
        let lab = super::native::component::lab::wanted(&name, shape.work)
            .and_then(|path| super::native::component::lab::capture_inputs(&self.db, &name, &spec, &path));
        // Per probe (spec order), its CPU seconds, while profiling: the run's per-probe `ms:rows:cpu_ms` field.
        let timed: Vector<f64> = rerail_blocks::vector::Vector::new();
        self.fx().open(&spec)?;
        // Members that start with rows (a maintained stratum's survivors, rows SQL wrote): the whole plans run in the
        // first wave, before the first chunk's live counts replan them, so they are planned again here from the
        // members' rows as the component opened (`Stats::sized`). `RERAIL_SIZED_AT_OPEN=off` plans them as before.
        if !(placed.gpu && not_taken.is_none()) && std::env::var("RERAIL_SIZED_AT_OPEN").map_or(true, |v| v != "off") {
            let text_symbols = |i: usize, c: usize| self.model.inputs.atoms.is_none() && f.types(rels[i]).get(c) == Some(&"sym");
            let live = self.fx().stats(&name, &text_symbols)?;
            let started: Vec<(String, LiveRel)> = rels.iter().zip(live).filter(|(_, l)| l.n > 0).map(|(r, l)| ((*r).to_owned(), l)).collect();
            if !started.is_empty() {
                let names: Vec<String> = started.iter().map(|(r, _)| r.clone()).collect();
                for (r, l) in started {
                    live_stats(&mut self.model, &r, l);
                }
                self.model.stats.sized(names);
                let mut whole: Vec<(body::RuleKey, PlanJoin, Vec<Step>)> = Vec::new();
                for k in 0..probes.len() {
                    let r = probes[k].0;
                    let b = vs.at(r);
                    if sql_probe[k] || unplanned[k] {
                        continue;
                    }
                    let (spec, order) = match whole.iter().find(|(p, _, _)| *p == r) {
                        Some((_, w, o)) => (w.clone(), o.clone()),
                        None => {
                            let _span = planning_key(r, "plan whole at open");
                            let ((new, _, _), _) = self.model.plan_rule_live(pf, &b, None)?;
                            let Some(w) = self.whole_join_spec(&b, &new, None, &ri, scc, native_rels) else { continue };
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
            sources: self.model.stats.sources.clone(),
            name,
            spec,
            rels: rels.iter().map(|r| (*r).to_owned()).collect(),
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
            searched: rerail_blocks::vector::Vector::new(),
            handoff: Vec::new(),
            expected: Vec::new(),
            walled: rerail_blocks::vector::Vector::new(),
            wholes,
            keys,
            started,
            placed,
            shape,
            cpu_own: if profiling { let (u, s) = crate::code_rule_dsl::prof::thread_usage(); u + s - cpu_at } else { 0.0 },
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
    fn plan_probe_now(&mut self, r: body::RuleKey, ai: usize, ri: &BTreeMap<String, usize>, scc: &super::front::Comp, native_rels: &BTreeSet<String>, planning: &dyn Fn(body::RuleKey, &str) -> super::profile::Span) -> Result<(Vec<Step>, PlanJoin, Option<super::profile::RulePlan>)> {
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
        run_counts().lazy[1].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
        let Driving { scc: scc_owned, solving, sources, name, spec, rels: rels_owned, ri, views, sql_views, sql_probe, mut orders, mut probes, order_rules, nseeds, seeds, ngates, mut explained, mut counted, mut earlier, mut timed, mut stepped, mut chunk, mut replanned_at, mut searched, keys, started, placed, shape, cpu_own, lab, not_taken, mut unplanned, mut handoff, mut expected, mut walled, mut wholes } = *d;
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
        let cpu_at = if profiling { let (u, s) = crate::code_rule_dsl::prof::thread_usage(); u + s - cpu_own } else { 0.0 };
        handoff.resize_with(nseeds + probes.len(), || None);
        expected.resize_with(nseeds + probes.len(), || None);
        // Each relation's rows, delta start and end as its statistics were last read (`stats_changed`).
        let mut live_seen: Vector<[usize; 3]> = rerail_blocks::vector::Vector::new();
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
                        // A probe with a whole plan (O168) runs one or the other in a wave, under one budget: due at
                        // the later of the two orders' checks, none where either is flagged. Its probe order flagged
                        // and its budget taken from the whole order, it was due at once at each chunk, never checked
                        // again (the probe's check skips a flagged order), and cut at each row: SLF001's `qualname`
                        // on 1,644 PostHog files, 405,000 cuts of 1 row of work in 60 s.
                        None => keys.get(line).copied().flatten().and_then(|key| {
                            let k = line.wrapping_sub(nseeds);
                            if line < nseeds { return joins::next_due(key, &orders[line].1); }
                            let probe = joins::next_due(key, &probes[k].2);
                            match wholes.get(&k) { Some(w) => probe.zip(joins::next_due(key, w)).map(|(a, b)| a.max(b)), None => probe }
                        }),
                    };
                    (line, due.map(|d| (d - work).max(1.0) as u64))
                }).collect();
                super::native::component::set_budgets(self.db, &name, &budgets)?;
            }
            let f: String = {
                let _wave = super::lanes::wave::suspendable();
                // Decision 0462: a relation at the width cap stops the run; its error names each rule deriving it, in
                // the order it ran, with the rows into and out of each step.
                self.fx().run(&name, chunk as u32).map_err(|e| match e {
                    Error::Sql(e) => Error::Sql(super::native::width::with_rules(e, |table| {
                        line_bodies.iter().enumerate().filter(|(_, b)| b.rel(b.head()) == table).map(|(line, b)| {
                            let order = if line < nseeds { &orders[line].1 } else { &probes[line - nseeds].2 };
                            let steps = counted.get(line).map_or(String::new(), |c| format!(" (rows into, out of each step: {})", c.iter().map(|(i, o)| format!("{i}→{o}")).collect::<Vec<_>>().join(", ")));
                            format!("{} :- {}.{steps}", b.head_text(), b.shown(order).join(", "))
                        }).collect()
                    })),
                    e => e,
                })?
            };
            // Decision 0462: a probe cut at the width cap, named by its rule in the order it ran.
            if let Some(run) = super::run::current().filter(|r| r.width.any_cut()) {
                run.width.name_cuts(&name, |line| {
                    let b = line_bodies.get(line)?;
                    let order = if line < nseeds { orders.get(line).map(|o| &o.1)? } else { probes.get(line - nseeds).map(|p| &p.2)? };
                    Some(format!("{} :- {}.", b.head_text(), b.shown(order).join(", ")))
                });
            }
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
                        scc: scc_owned, solving, sources, name, spec, rels: rels_owned, ri, views, sql_views, sql_probe, orders, probes, order_rules, nseeds, seeds, ngates,
                        explained, counted, earlier, timed, stepped, chunk, replanned_at, searched, keys, started, placed, shape, lab, not_taken, unplanned, handoff, expected, walled, wholes,
                        cpu_own: if profiling { let (u, s) = crate::code_rule_dsl::prof::thread_usage(); u + s - cpu_at } else { 0.0 },
                    })
                }};
            }
            // Requests, where they are posted (decision 0114 §3): the stratum suspends with its tickets.
            if f.last() == Some(&"2") && self.asking.is_some() && self.model.inputs.atoms.is_some() {
                let asked = super::native::component::pending(self.db, &name)?;
                let asking = self.asking.as_deref_mut().expect("checked");
                let tickets = asked.iter().map(|(rel, rows)| asking.post(rel, rows)).collect::<Result<Vector<u64>>>()?;
                run_counts().answer_calls.fetch_add(tickets.len() as u64, std::sync::atomic::Ordering::Relaxed);
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
            // A declared key a second row broke where the rows were written (decision 0361 step 6): the run's, as a
            // probe's counts break a fact (`Model::layout_broken`), planned from statistics from then on.
            let mut key_broke = false;
            for (i, cols) in self.fx().key_breaks(&name)? {
                let Some(rel) = rels.get(i).map(|r| r.to_string()) else { continue };
                let shapes = &self.model.stats.shapes;
                let fact = crate::code_rule_dsl::logic::front::LayoutFact::Key(cols).show(&|c| shapes.column(rel.as_str(), c).map_or_else(|| format!("${c}"), str::to_owned));
                if shapes::run_broken().note(&rel, fact.clone()) {
                    let text = format!("{rel}: {fact} broken: a second row of a key, where its rows were written; planned from statistics for the rest of the run");
                    if super::profile::strict() {
                        eprintln!("rerail: a layout fact broken (decision 0361 step 6): {text}");
                    }
                    super::profile::decision("broken layout facts (decision 0361 step 6)", || text);
                }
                key_broke = true;
            }
            if key_broke {
                self.model.stats.shapes = shapes::Shapes::for_run(std::sync::Arc::clone(&self.model.stats.shapes));
            }
            if profiling {
                for (k, line) in self.fx().step_cpu(&name)?.into_iter().enumerate() {
                    if stepped.len() <= k { stepped.resize(k + 1, rerail_blocks::vector::Vector::new()); }
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
            // Decision 0361 step 6: every line's rows so far against the facts its order was priced by, in every run
            // (not only the lines the joins check below): a probe past its fact's bound breaks the fact, and its
            // relation is planned from statistics from then on. A line's counts are its running order's (they are
            // taken from it when the order is replaced): a seed's `orders`, a probe's `probes` (a whole plan's rows
            // are not counted). The checks below replan by what broke here.
            let mut layout_broke = vec![false; counted.len()];
            for (line, c) in counted.iter().enumerate() {
                let order = if line < nseeds { orders.get(line).map(|o| &o.1) } else { probes.get(line - nseeds).map(|p| &p.2) };
                if let Some(order) = order
                    && !c.is_empty()
                {
                    layout_broke[line] = self.model.layout_broken(&line_bodies[line], order, c);
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
                    let (r, ai) = (probes[k].0, probes[k].1);
                    // Its prepared order, horribly slow (past `slow_factor` of what it was prepared at) or heavy (the
                    // check below): planned again next round with the run's statistics (`prepared_heavy`).
                    let checked = joins::check(key, work, &probes[k].2);
                    // Its rows against its order's step by step: the widest blown up, or a step far under its price.
                    let miss = self.model.line_miss(pf, &vs.at(r), Some(ai), &probes[k].2, key, c, &mut expected[line]);
                    // Its probes' rows broke a fact they were priced by (decision 0361 step 6, `layout_broke` above):
                    // its relation is left to the statistics, and a prepared order priced by it is searched.
                    let broken = layout_broke[line];
                    if (checked || miss.is_some() || broken || self.model.prepared_horribly_slow(&vs.at(r), &[Some(ai)], &probes[k].2, work))
                        && self.model.prepared_heavy(&vs.at(r), &[Some(ai)], &probes[k].2).is_some()
                    {
                        replanned_at.force(k);
                        continue;
                    }
                    if !checked && miss.is_none() {
                        continue;
                    }
                    let b = vs.at(r);
                    // A body of one positive atom has one order: nothing to plan again.
                    if positive_atoms(&b) < 2 {
                        continue;
                    }
                    let delta = c.first().map_or(0, |s| s.1) as f64;
                    let Some(est) = self.model.price_for_delta(pf, &b, Some(ai), &probes[k].2, delta) else { continue };
                    if miss.is_none() && work <= joins::slow_factor() * est.max(1.0) {
                        continue;
                    }
                    let rule = order_rules[line];
                    if let Some(m) = &miss {
                        joins::count_miss(m);
                        super::profile::decision("mispredicted joins (decision 0203)", || format!("{} (delta at {ai}): {}", b.head_text(), m.describe()));
                        joins::log(|| format!("mispredicted {} (delta at {ai}): {}; {}", b.head_text(), m.describe(), self.model.describe_miss(pf, &b, Some(ai), &probes[k].2, c)));
                    }
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
                let due: Vector<usize> = (0..probes.len()).filter(|&k| unplanned[k] && replanned_at.delta_rows(k) > 0).collect();
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
                let mut whole: Vec<(body::RuleKey, PlanJoin, Vec<Step>)> = Vec::new();
                let wants_whole = |k: usize, this: &Self| {
                    let b = &line_bodies[nseeds + k];
                    let delta = b.atom(probes[k].1).map(|a| b.rel(a));
                    !sql_probe[k]
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
                            let Some(w) = self.whole_join_spec(b, &new, None, &ri, scc, native_rels) else { continue };
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
            let mut replan: Vector<usize> = rerail_blocks::vector::Vector::new();
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
                    .whole_join_spec(b, &new, Some(at), &ri, scc, native_rels)
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
                    // Its rows were checked against the facts its order was priced by as they were counted (decision 0361
                    // step 6, `layout_broke`): a fact broken there prices this search from statistics.
                    // Its prepared order, heavy: searched now with the run's statistics, and a better order goes on
                    // under the handoff (decision 0206), which goes back to this one where it runs slower a row.
                    if self.model.prepared_heavy(&b, &[first, None], &running).is_some() {
                        let mut cx = joins::SearchCx::default();
                        if let Some(new) = self.model.plan_for_rows_in(pf, &b, cut_lead, c.left.max(1) as f64, &mut cx)
                            && !same_order(&new, &running)
                        {
                            place!(new);
                        }
                    }
                    // The walk cut now is judged by its own work over its own rows (a line's work so far may be an
                    // earlier walk's too: then a few rows read under a short budget stood for all of it).
                    let ran = c.work as f64;
                    // Its rows against its order's step by step (`line_miss`): the widest blown up, or far under.
                    let miss = counted.get(line).and_then(|steps| self.model.line_miss(pf, &b, Some(cut_lead), &running, key, steps, &mut expected[line]));
                    let est = if joins::check(key, work, &running) || miss.is_some() { self.model.price_for_delta(pf, &b, Some(cut_lead), &running, c.done.max(1) as f64) } else { None };
                    if let Some(est) = est && (miss.is_some() || ran > joins::slow_factor() * est.max(1.0)) && positive_atoms(&b) >= 2 {
                        let rule = order_rules[line];
                        if let Some(m) = &miss {
                            joins::count_miss(m);
                            super::profile::decision("mispredicted joins (decision 0203)", || format!("{}: {}", b.head_text(), m.describe()));
                            if let Some(steps) = counted.get(line) {
                                joins::log(|| format!("mispredicted {}: {}; {}", b.head_text(), m.describe(), self.model.describe_miss(pf, &b, Some(cut_lead), &running, steps)));
                            }
                            self.model.prepared_heavy(&b, &[first, None], &running);
                        }
                        joins::flag_slow(key, rule, &running, est, ran);
                        super::profile::decision("slow joins (decision 0203)", || format!("{} {}: {ran:.0} rows of work, priced {est:.0} for its {} rows read", if whole { "whole plan" } else { "seed" }, b.head_text(), c.done));
                        let exact = if joins::pricing() { Vec::new() } else { self.exact_statistics(&name, line, whole, &b, &running) };
                        joins::log(|| format!("slow {} {}: {ran:.0} rows of work, priced {est:.0} for its {} rows read; {} exact pair statistics; its share of a critical path {:.3}{}", if whole { "whole plan" } else { "seed" }, b.head_text(), c.done, exact.len(), joins::critical_share_of(&key), if joins::trial_due(&key) { ", trialled" } else { "" }));
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
                    let values = |ids: &[u32]| -> Vec<Vector<i64>> {
                        let src = lead_src.as_ref().expect("a walker");
                        ids.iter().map(|&id| (0..lead_cols).map(|c| src.at(id as usize).at(c)).collect()).collect()
                    };
                    // The first phase's rows: a uniform sample of each part (seeded), as lead rows.
                    let phase1: Option<[Vec<Vector<i64>>; 2]> = hp.as_ref().filter(|_| walker.is_some()).map(|hp| {
                        let pick = |ids: &[u32], salt: u64| -> Vector<u32> {
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
                        let stride = |ids: &[u32], k: usize| -> (Vector<u32>, usize) {
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
                    let mut by_walk: Vector<usize> = (1..distinct.len()).filter(|&i| !walked_out(i)).collect();
                    by_walk.sort_by(|&x, &y| line_walk[x].unwrap_or(f64::INFINITY).total_cmp(&line_walk[y].unwrap_or(f64::INFINITY)));
                    // Where the walks leave only the running order, its trial would decide nothing: none runs (on 328
                    // data-layer files the trial alone was cut after 8.6 s).
                    let tried: Vector<usize> = if by_walk.is_empty() && line_walk[0].is_some() { rerail_blocks::vector::Vector::new() } else { std::iter::once(0).chain(by_walk).collect() };
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
                            Some((spec, step)) => super::native::component::trial_over(this.db, &name, line, Some(spec.get()), *step, pick, budget),
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
                            let Some((run, all, picked)) = super::native::component::trial_own(this.db, &name, spec.get(), &size, cap)? else { return Ok(()) };
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
                                    let rows: Vec<Vector<i64>> = picked.iter().map(|&id| (0..largs.len()).map(|c| src.at(id as usize).at(c)).collect::<Vector<i64>>())
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
                    let stat_prices: Vector<f64> = if joins::logging() {
                        let obs = self.model.stats.obs.remove(&rule);
                        let p = measured.iter().map(|(o, run)| self.model.price_for_delta(pf, &b, Some(lead), o, run.rows as f64).unwrap_or(f64::NAN)).collect();
                        if let Some(obs) = obs { self.model.stats.obs.insert(rule, obs); }
                        p
                    } else { rerail_blocks::vector::Vector::new() };
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
            let (u, s) = crate::code_rule_dsl::prof::thread_usage();
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
                super::profile::native(format!("{kind}: {}", shown(o).join(" , ")), counted.get(k).unwrap_or(&empty), timed.get(k).copied().unwrap_or(0.0), stepped.get(k).map_or(&[][..], |v| v.as_slice()));
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
                    super::profile::planned_cpu(&key, kind, x, stepped.get(k).map_or(&[][..], |v| v.as_slice()));
                }
            }
        }
        if let Some(file) = std::env::var_os("RDL_COMPONENTS") {
            // The component's plans and each step's observed bindings in and out (`set_fixpoint_steps`), appended.
            let mut text = format!("COMPONENT {} rounds={} ms={:.1}\n", rels.iter().map(|r| r.as_str()).collect::<Vec<_>>().join("/"), chunk.trailing_zeros(), started.elapsed().as_secs_f64() * 1e3);
            for l in spec.lines.iter() {
                use super::native::component::spec::ArchivedLine as L;
                let (kind, join) = match l {
                    L::Seed { join, .. } => ("seed", spec.bound(join)),
                    L::Join { join, .. } => ("join", spec.bound(join)),
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
            if joins::on() && with_plans(|p| p.is_some()) {
                let stratum = scc.iter().next().map(str::to_owned).unwrap_or_default();
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
                        let steps = c.iter().map(|(i, o)| format!("{i}/{o}")).collect::<Vec<_>>().join(" ");
                        let _ = writeln!(log.lock(), "{stratum}\t{first:?}\t{shown}\t{w:.0}\t{est:.0}\t{wall:.6}\t{:x}\t{steps}\t{text}", key as u64);
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
        self.native_rels.extend(scc.iter().map(str::to_owned));
        Ok(Native::Ran)
    }

    /// Runs a one-relation flat-lattice component natively (sqlite-ext `lattice_fixpoint`, T's `native_fixpoint`)
    /// if every rule is a seed (no recursive atom), a copy (`R($n, $v) :- ..., R($x, $v)`) or a binary transfer
    /// (`R($n, @flat_bin($o, $a, $b)) :- ..., R($l, $a), R($r, $b)`). The rest of each body is non-recursive, so SQL
    /// evaluates it once, planned as usual, into the component's seed and edge tables. Returns false to fall back.
    fn native_fixpoint(&mut self, scc: &super::front::Comp, rules: &[RuleKey]) -> Result<bool> {
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
                Lit::Atom(a) if v.rel_name(a.rel()) == rel => Some(a),
                _ => None,
            }).collect();
            let rest: Vector<usize> = (0..lits.len()).filter(|&i| !matches!(lits[i].view(), Lit::Atom(a) if v.rel_name(a.rel()) == rel)).collect();
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
            let kinds: Vector<usize> = (0..lits.len())
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
            let part = |rel: String, head, lits: Vector<usize>| Part { rel, head, pre: Vec::new(), from: (v, r), lits };
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
            let n = self.model.stats.tree.len() as f64;
            self.model.stats.card.insert("rdl_kinds", n);
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
            Arc::make_mut(&mut self.views).insert(rel.to_owned());
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
    fn native_set_fixpoint(&mut self, scc: &super::front::Comp, rules: &[RuleKey]) -> Result<bool> {
        let versions = Arc::clone(&self.versions);
        let pf = versions.whole(self.version);
        let versions = Arc::clone(&self.versions);
        let fields = versions.whole(self.version);
        if scc.iter().any(|r| fields.lattice(r)) {
            return Ok(false);
        }
        let rels: Vec<&str> = scc.iter().collect();
        let ri: BTreeMap<String, usize> = rels.iter().enumerate().map(|(i, r)| ((*r).to_owned(), i)).collect();
        let name = format!("sfx{}_{}", self.made.len(), rels[0]);
        let mut seed_plans = Vec::new();
        let vs = Arc::clone(&self.versions);
        for &r in rules.iter().filter(|&&r| recursive(&vs.at(r), scc).is_empty()) {
            let b = vs.at(r);
            let plan = self.model.plan_rule(pf, &b, None)?;
            seed_plans.push((vs.head_at(r), b, plan));
        }
        for r in &rels {
            let rows: f64 = seed_plans.iter().filter(|(h, _, _)| *h == *r).map(|(_, _, p)| p.2).sum();
            let card = prior_stats(&mut self.model, pf, r).map_or(rows.max(1.0), |n| n.max(rows.max(1.0)));
            self.model.stats.card.insert(*r, card);
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
        let tables: HashMap<String, (String, String)> = rels.iter().map(|r| ((*r).to_owned(), (format!("sfx_f_{r}"), format!("sfx_d_{r}")))).collect();
        let types = |r: &str| fields.types(r).iter().map(|t| sym_char(&self.model.inputs, t)).collect::<String>();
        let mut lines: Vec<SpecLine> = rels.iter().map(|r| SpecLine::Rel { name: (*r).to_owned(), types: types(r), start: Start::Rows }).collect();
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
                let rest: Vector<usize> = (0..lits.len()).filter(|&i| i != ai).collect();
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
                    let vs = |vs: &[String]| vs.iter().map(|v| slot[v.as_str()]).collect::<Vector<_>>();
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
                lines.push(SpecLine::Join { head: ri[b.rel(b.head())], delta: ri[rel], join: spec, whole: None, est: Default::default() });
                continue;
            }
            let reads: Vec<String> = (0..b.len()).filter_map(|i| b.atom(i)).map(|a| b.rel(a).to_owned()).collect();
            self.ensure(&reads)?;
            self.ensure_indexes(Order::new(&b, order))?;
            let sql = self.model.sql_select(pf, &b, order, Some(*at), Some(&tables))?;
            lines.push(SpecLine::Probe { head: ri[b.rel(b.head())], delta: ri[rel], sql });
        }
        self.fx().open(&ComponentSpec { name: name.clone(), lines }.encode())?;
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
            for (k, probe) in probes.iter_mut().enumerate() {
                let (r, ai) = (probe.0, probe.3);
                let (new, _, _) = self.model.plan_rule(pf, &vs.at(r), Some(ai))?;
                if same_order(&new, &probe.1) {
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
                probe.1 = new;
                probe.2 = nat;
            }
            chunk *= 2;
        }
        for r in &rels {
            let (full, delta) = &tables[*r];
            self.db.execute_batch(&format!("DROP TABLE temp.{full}; DROP TABLE temp.{delta};"))?;
        }
        self.fx().close(&name)?;
        self.made.insert((format!("sfx:{name}"), rerail_blocks::vector::Vector::new()));
        Ok(true)
    }

}

/// Evaluates program `v` over the facts database at `facts` (T's `run_sql`, pure SQL mode): views for projections of
/// one input, typed tables for the rest, then each stratum planned from the statistics and the counts of the strata
/// before it, non-recursive ones once and recursive ones semi-naively.
pub fn run_sql(v: Arc<crate::code_rule_dsl::logic::program_image::Version>, facts: &Path) -> Result<Outcome> {
    // The facts file is mapped (`mmap_size` below): it and its folder the user's alone (`rerail_blocks::mapped`).
    rerail_blocks::mapped::open_read(facts, false).map_err(|e| Error::Plan(e.to_string()))?;
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
    // The facts file is mapped (`mmap_size` below): it and its folder the user's alone (`rerail_blocks::mapped`).
    rerail_blocks::mapped::open_read(facts, false).map_err(|e| Error::Plan(e.to_string()))?;
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
    /// Whether `rel` has a delta this run: given, or made by the run.
    fn has_delta(&self, rel: &str) -> bool;
    /// `rel` changed and has no deltas (derived again, and its old rows not kept): its readers derive again.
    fn no_delta(&mut self, rel: &str);
    /// `rel`'s delta was made by the run itself (`kept_delta`): its readers read its versions.
    fn delta_made(&mut self, rel: &str);
    /// The inputs that changed with a delta not empty, or with none: the walk's first moved reads.
    fn seeds(&mut self) -> Vec<String>;
    /// Stratum `scc` was maintained by the evaluator's mode (decision 0364 stage 3) and its rows changed: each
    /// relation's change is its version, read by its readers (`recursive`: maintained by levels). The run counts it
    /// (`Outcome::maintained`).
    fn mode_ran(&mut self, scc: &super::front::Comp, recursive: bool);
    /// Input `rel`'s rows gone and added, `[minus, plus]`, read in place: the maintain mode's versions of it.
    fn given(&self, rel: &str) -> Option<[Arc<dyn crate::code_rule_dsl::facts::vtab::Source>; 2]>;
    /// Input `rel`'s rows as the last run had them, read in place (the maintain mode's `old` of an input).
    fn old_input(&self, rel: &str) -> Option<Arc<dyn crate::code_rule_dsl::facts::vtab::Source>>;
    /// The static check of the whole program the run's part was split from (decision 0364 §5); `None`: the run
    /// checks its own version, which holds only the part's rules.
    fn proofs(&self) -> Option<Arc<wellfounded::Proofs>> {
        None
    }
}

/// Holds `rel`'s kept rows `o` as the run's relation, with its cardinality (a quiet stratum's, decision 0117 step 3).
fn hold_quiet(run: &mut Run, f: Fields<'_>, db: &Connection, rel: &str, o: Arc<super::native::component::InRel>) {
    let card = quiet_card(&mut run.model, &run.quiet, f, rel, &o);
    run.model.stats.card.insert(rel, card);
    super::native::component::hold_as(db, rel, o);
    run.native_rels.insert(rel.to_owned());
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
    // A kept relation read from the store (t33 K4: a kept entry with no statistics part, whose rows no run measured
    // yet): its columns' statistics where the planner first asks, read as its source answers them (`Derived`: kept as
    // its rows came, else measured over its rows where they lie).
    if !model.stats.lazy.0.contains_key(rel)
        && let Some(width) = f.arity(rel)
    {
        let names: Vec<String> = (0..width).map(|c| sql_col(rel, c)).collect();
        let src = o.source(names);
        model.stats.lazily(rel, o.rows(), LazyStats::new(move || {
            let cols = crate::code_rule_dsl::facts::vtab::syntax::statistics(src.as_ref()).into_iter().map(|(c, rows, ndv, sq, sqd)| (c, [rows, ndv, sq, sqd])).collect();
            (cols, Vec::new(), ())
        }));
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
    /// This run derived `rel`: its digest, and its rows (types, rows, count), with the digest each relation its
    /// stratum read had; `None` when SQL derived it, or a read's digest is not known.
    fn ran(&mut self, rel: &str, got: Option<(u64, Vec<u8>, RanRows, usize, Vec<(String, u64)>)>);
    /// The run's end: the strata it evaluated, of all.
    fn counted(&mut self, evaluated: usize, of: usize);
}

/// The relations a stratum's rules read: atoms (an aggregate's too), `node` and `child` for a `nearest` climb, a
/// native operator's inputs.
/// A changed read's versions as the maintain mode reads them (decision 0364): its kept rows, its rows gone and added,
/// and its rows now, each a source read where it lies, with the columns of the input it is a view of, where it is
/// one. A relation derived again that could not be made a version of its kept rows (`kept_delta`) has its kept rows
/// as old and its rows gone and added as minus and plus (`diffs`); a changed input, its given deltas; a view of an
/// input the engine holds with its change marked where its rows lie (a file-local part's export, `fl…_cparam`), its
/// input's versions through its columns. Shared by the mode's inputs and the reach by kind and key (decision 0563 B).
fn moved_versions(r: &str, diffs: &HashMap<String, [Arc<super::native::component::InRel>; 2]>, m: &mut dyn Maintain, db: &Connection, f0: Fields<'_>, rules_by: &IndexMap<&str, Vec<RuleKey>>, run_vs: &body::Versions) -> Option<([Arc<dyn crate::code_rule_dsl::facts::vtab::Source>; 4], Option<rerail_blocks::vector::Vector<usize>>)> {
    if let (Some([mi, pl]), Some(o), Some(now)) = (diffs.get(r), m.kept(r), super::native::component::held(db, r)) {
        let cols = |x: &Arc<super::native::component::InRel>| x.source((0..o.k).map(|c| sql_col(r, c)).collect());
        return Some(([cols(&o), cols(mi), cols(pl), cols(&now)], None));
    }
    let (base, cols) = if f0.is_input(r) {
        (r.to_owned(), None)
    } else {
        let [k] = rules_by.get(r).map(Vec::as_slice).unwrap_or_default() else { return None };
        let (v, rule) = run_vs.rule_at(*k);
        let (base, cols) = input_view_shape(v, rule, |x| f0.is_input(x))?;
        (base, Some(cols))
    };
    if let Some([mi, pl]) = m.given(&base) {
        let now = crate::code_rule_dsl::facts::vtab::bound_source(db, &base)?;
        return Some(([m.old_input(&base)?, mi, pl, now], cols));
    }
    cols.as_ref()?;
    let h = super::native::component::held(db, &base).filter(|h| h.start().is_some())?;
    let src = |x: &Arc<super::native::component::InRel>| x.source((0..h.k).map(|c| sql_col(&base, c)).collect());
    use super::native::component::Ver;
    let [o, mi, pl] = [Ver::Old, Ver::Minus, Ver::Plus].map(|v| h.version_view(v));
    Some(([src(&o?), src(&mi?), src(&pl?), src(&h)], cols))
}

/// What the reach by kind and key reads of a stratum's changed reads (decision 0563 B): each one's versions
/// (`moved_versions`), and how a joined relation is read. Made at the driver, for one stratum.
struct Reach<'r, 'f> {
    diffs: &'r HashMap<String, [Arc<super::native::component::InRel>; 2]>,
    db: &'r Connection,
    f0: Fields<'f>,
    rules_by: &'r IndexMap<&'f str, Vec<RuleKey>>,
    run_vs: &'r body::Versions,
    moved: &'r BTreeSet<String>,
    scc: &'r super::front::Comp,
    /// A changed read's delta as the atoms that read the same columns of it see it (`Net`): its rows gone and added
    /// that count, the same for every stratum that reads it so, so made once a run.
    live: &'r std::cell::RefCell<NetDeltas>,
}

/// The net deltas a run's reach has made (`Reach::live`), by relation and the columns read.
type NetDeltas = HashMap<(String, Vec<usize>), [Arc<dyn crate::code_rule_dsl::facts::vtab::Source>; 2]>;

/// A column's test in an atom: the values it may hold (none: the atom holds a constant no row has).
type Allowed = Option<Vec<i64>>;

/// Whether `values` (one a test, in order) pass `tests`.
fn allowed(values: &[i64], tests: &[(usize, Allowed)]) -> bool {
    tests.iter().zip(values).all(|((_, allowed), x)| allowed.as_ref().is_some_and(|vals| vals.contains(x)))
}

/// Whether some row of `d` passes `tests` (each a column of `d` and its values), read where it lies.
fn passes(d: &Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, tests: &[(usize, Allowed)]) -> bool {
    let cols: Vec<usize> = tests.iter().map(|(c, _)| *c).collect();
    let mut any = false;
    d.for_rows_in(0..d.len(), &cols, &mut |_, row| {
        any = allowed(row, tests);
        !any
    });
    any
}

/// One side of a changed read's delta as an atom sees it (decision 0563 B): a row gone (or added) counts only where
/// no row added (or gone) holds the same values in the columns the atom reads (`read`). An insert's rows past it
/// keep their ids (`granules`) and change only in their places (`node`'s start and end, `line`, `node_order`): to
/// `node(d, v, _, _, _)` they did not change. Read in place: each row's read columns, looked up in the other side
/// through its own index. With no columns read, or no index to look up, every row counts.
struct Net<'s> {
    this: &'s Arc<dyn crate::code_rule_dsl::facts::vtab::Source>,
    other: &'s Arc<dyn crate::code_rule_dsl::facts::vtab::Source>,
    /// The columns read, the one looked up first: the one the other side's index finds fewest rows for (a node's id,
    /// not its document, whose rows are every row of the edited file).
    read: &'s [usize],
}

impl Net<'_> {
    /// Whether row `row` of this side counts.
    fn live(&self, row: usize, mine: &mut [i64], theirs: &mut [i64]) -> bool {
        let Some(&key) = self.read.first() else { return true };
        self.this.row_at(row, self.read, mine);
        let mut same = |r: usize| {
            self.other.row_at(r, self.read, theirs);
            theirs == mine
        };
        if let Some(rows) = self.other.sorted_rows(key, mine[0]) {
            return !rows.iter().any(|&r| same(r as usize));
        }
        match self.other.lookup(key, &[mine[0]]) {
            Some(rows) => !rows.into_iter().any(same),
            None => true,
        }
    }

    /// This side's rows that count, as a source read where they lie (`InRel::picked`, as an input's delta is named,
    /// 0364 R2): this side itself where every row counts.
    fn rows(&self) -> Arc<dyn crate::code_rule_dsl::facts::vtab::Source> {
        let (mut mine, mut theirs) = (vec![0i64; self.read.len()], vec![0i64; self.read.len()]);
        let ids: Vector<u32> = (0..self.this.len()).filter(|&r| self.live(r, &mut mine, &mut theirs)).map(|r| r as u32).collect();
        if ids.len() == self.this.len() {
            return Arc::clone(self.this);
        }
        super::native::component::InRel::picked(Arc::clone(self.this), ids, self.this.columns())
    }
}

impl Reach<'_, '_> {
    /// Whether no rule of `rules` sees a row of any changed read's delta (decision 0563 B). Each atom over a changed
    /// read shares a variable with a positive atom of another relation, earlier in the order of strata, and no row
    /// gone or added of the read holds, in that column, a value any row of the other relation holds there under the
    /// atom's other tests (a kind's constant, a kind set's facts, a field's name). The delta is read where it lies,
    /// and the other relation, a derived one the run holds, through its own index (`Source::lookup`): its rows now,
    /// and where it changed too, its rows gone. A read no rule names as an atom (a native's, a request's, a nearest ancestor's, a matcher's) is never
    /// cut; nor is an aggregate's.
    fn cuts(&self, m: &mut dyn Maintain, model: &mut Model, rules: &[RuleKey]) -> bool {
        let mut named: BTreeSet<&str> = BTreeSet::new();
        for &k in rules {
            let (v, rule) = self.run_vs.rule_at(k);
            let uses = var_uses(rule);
            let mut sets: HashMap<VarSlot, Vec<i64>> = HashMap::new();
            for l in rule.body() {
                match l.view() {
                    Lit::Nearest { .. } | Lit::Ast { .. } => return false,
                    Lit::Agg { atom, .. } if self.moved.contains(v.rel_name(atom.rel())) => return false,
                    Lit::Atom(a) if !a.neg() && a.args().count() == 1 => {
                        // A kind set: a relation of facts alone, each a value.
                        let name = v.rel_name(a.rel());
                        if let (Some(x), Some(facts)) = (a.args().next().and_then(TermRef::var), v.facts(v.rel_atom(a.rel())))
                            && v.heads(a.rel()).next().is_none()
                            && !self.f0.is_input(name)
                        {
                            let mut values: Vec<i64> = Vec::new();
                            for row in facts.rows() {
                                for c in row {
                                    match c {
                                        super::program_image::Cell::Int(i) => values.push(i),
                                        super::program_image::Cell::Branded(b) => values.push(b.value()),
                                        super::program_image::Cell::Str(t) => values.extend(super::native::setfix::symbol_id(self.db, v.text(t)).ok().flatten()),
                                    }
                                }
                            }
                            sets.insert(x, values);
                        }
                    }
                    Lit::Cmp { l, op: CmpOp::Eq, r } => {
                        let held = match (l.var(), r.var()) {
                            (Some(x), None) => self.constant(v, r).map(|c| (x, c)),
                            (None, Some(x)) => self.constant(v, l).map(|c| (x, c)),
                            _ => None,
                        };
                        if let Some((x, c)) = held {
                            sets.insert(x, c.into_iter().collect());
                        }
                    }
                    _ => {}
                }
            }
            for l in rule.body() {
                let Lit::Atom(a) = l.view() else { continue };
                let name = v.rel_name(a.rel());
                let Some(moved) = self.moved.get(name) else { continue };
                named.insert(moved.as_str());
                if !self.atom_cut(m, model, v, rule, a, (&sets, uses.as_ref())) {
                    return false;
                }
            }
        }
        self.moved.iter().all(|r| named.contains(r.as_str()))
    }

    /// A constant term's value as the run holds it; `Some(None)`: a symbol the run has none of, which no row holds.
    fn constant(&self, v: &Version, t: TermRef<'_>) -> Option<Option<i64>> {
        match t {
            TermRef::Int(i) => Some(Some(i)),
            TermRef::Branded(b) => Some(Some(b.value())),
            TermRef::Str(x) => Some(super::native::setfix::symbol_id(self.db, v.text(x)).ok().flatten()),
            _ => None,
        }
    }

    /// Whether atom `a` (over a changed read) sees no row of the read's delta: some positive atom of another
    /// relation shares a variable with it and holds none of the delta's values there.
    fn atom_cut(&self, m: &mut dyn Maintain, model: &mut Model, v: &Version, rule: super::program_image::RuleRef<'_>, a: AtomRef<'_>, (sets, uses): (&HashMap<VarSlot, Vec<i64>>, Option<&HashMap<VarSlot, usize>>)) -> bool {
        let r = v.rel_name(a.rel());
        let Some(([_, minus, plus, _], cols)) = moved_versions(r, self.diffs, m, self.db, self.f0, self.rules_by, self.run_vs) else { return false };
        let at = |c: usize, cols: &Option<Vector<usize>>| cols.as_ref().map_or(Some(c), |m| m.get(c).copied());
        // The columns the atom reads: a constant's, or a variable's used elsewhere in the rule (all but `_`'s where
        // the rule's variables are not all counted, `var_uses`); through the columns of the input it is a view of.
        let read: Vec<usize> = a.args().enumerate().filter(|(_, u)| match u {
            TermRef::Wild => false,
            TermRef::Var(x) => uses.is_none_or(|n| n.get(x).copied().unwrap_or(0) > 1),
            _ => true,
        }).map(|(i, _)| at(i, &cols)).collect::<Option<Vec<usize>>>().unwrap_or_default();
        let [minus, plus] = if read.is_empty() {
            [minus, plus]
        } else {
            let key = (r.to_owned(), read.clone());
            let made = self.live.borrow().get(&key).cloned();
            made.unwrap_or_else(|| {
                let by = |other: &Arc<dyn crate::code_rule_dsl::facts::vtab::Source>| {
                    let mut by = read.clone();
                    if let Some(i) = (0..by.len()).min_by_key(|&i| other.estimated_matches(by[i], false)) {
                        by.swap(0, i);
                    }
                    by
                };
                let (to_plus, to_minus) = (by(&plus), by(&minus));
                let net = [Net { this: &minus, other: &plus, read: &to_plus }.rows(), Net { this: &plus, other: &minus, read: &to_minus }.rows()];
                self.live.borrow_mut().insert(key, net.clone());
                net
            })
        };
        // No row of the delta counts: the atom sees no change (an insert's rows past it, moved in place alone).
        if minus.len() == 0 && plus.len() == 0 {
            return true;
        }
        // The atom's own tests (a kind's constant, a kind set's facts): a row of the delta none of them passes is no
        // row the rule sees.
        let own: Vec<(usize, Allowed)> = a.args().enumerate().filter_map(|(i, u)| {
            let test = match u {
                TermRef::Var(y) => sets.get(&y).map(|vals| Some(vals.clone())),
                other => self.constant(v, other).map(|c| c.map(|c| vec![c])),
            }?;
            Some((at(i, &cols)?, test))
        }).collect();
        if !own.is_empty() && [&minus, &plus].iter().all(|d| !passes(d, &own)) {
            return true;
        }
        for (i, t) in a.args().enumerate() {
            let (Some(x), Some(ri)) = (t.var(), at(i, &cols)) else { continue };
            for o in rule.body() {
                let Lit::Atom(b) = o.view() else { continue };
                let q = v.rel_name(b.rel());
                if b.neg() || q == r || self.scc.contains(q) {
                    continue;
                }
                let Some(j) = b.args().position(|u| u.var() == Some(x)) else { continue };
                // The other columns' tests: a constant, or a variable a kind set or a comparison holds.
                let tests: Vec<(usize, Allowed)> = b.args().enumerate().filter(|(jj, _)| *jj != j).filter_map(|(jj, u)| match u {
                    TermRef::Var(y) => sets.get(&y).map(|vals| (jj, Some(vals.clone()))),
                    other => self.constant(v, other).map(|c| (jj, c.map(|c| vec![c]))),
                }).collect();
                let Some(sources) = self.joined(m, model, q) else { continue };
                if self.joins_none(&[&minus, &plus], ri, &own, &sources, j, &tests) == Some(true) {
                    return true;
                }
            }
        }
        false
    }

    /// Joined relation `q` as the stratum reads it, where it lies: its rows now, and where it changed, its rows gone
    /// too. `None` where the run holds no source of it, and for an input or a view of one: an input is read through
    /// its view of the batch's documents, where a lookup builds the documents' rows (420 ms for `docof` on ep2's
    /// insert), which a run that keeps its strata never builds. An input's delta is still tested in its own rows.
    fn joined(&self, m: &mut dyn Maintain, model: &mut Model, q: &str) -> Option<Vec<(Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, Option<Vector<usize>>)>> {
        // An input the run holds no rows of itself (neither held nor kept) is read through a view of the documents,
        // and so is a changed input's rows now: only a relation this program derives is read changed. A relation held
        // as kept is an input of the maintained program too.
        let derived = self.rules_by.contains_key(q);
        if self.moved.contains(q) {
            if !derived {
                return None;
            }
            let ([_, minus, _, now], cols) = moved_versions(q, self.diffs, m, self.db, self.f0, self.rules_by, self.run_vs)?;
            if cols.is_some() {
                return None;
            }
            return Some(vec![(now, None), (minus, None)]);
        }
        if self.f0.is_input(q) && !derived && super::native::component::held(self.db, q).is_none() && !m.keeps(q) {
            return None;
        }
        // Held, kept in memory, or bound as an input is (a kept relation `Maintain::need` bound where it lies); kept
        // and not read yet, bound so first, as a stratum that reads it binds it.
        let bound = |m: &mut dyn Maintain| -> Option<Arc<dyn crate::code_rule_dsl::facts::vtab::Source>> {
            if let Some(h) = super::native::component::held(self.db, q).or_else(|| m.kept(q)) {
                return Some(h.source((0..h.k).map(|c| sql_col(q, c)).collect()));
            }
            crate::code_rule_dsl::facts::vtab::bound_source(self.db, q)
        };
        if let Some(src) = bound(m) {
            return Some(vec![(src, None)]);
        }
        if m.keeps(q) {
            m.need(&[q.to_owned()], model).ok()?;
            return Some(vec![(bound(m)?, None)]);
        }
        None
    }

    /// Whether no row of `deltas` holds in column `ri` a value some row of `sources` holds in column `j` under `tests`;
    /// `None` where a source has no lookup of that column.
    fn joins_none(&self, deltas: &[&Arc<dyn crate::code_rule_dsl::facts::vtab::Source>], ri: usize, own: &[(usize, Allowed)], sources: &[(Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, Option<Vector<usize>>)], j: usize, tests: &[(usize, Allowed)]) -> Option<bool> {
        self.joins_none_in(deltas, ri, own, sources, (j, tests), true)
    }

    /// `joins_none`, with the other relation's joined column and its tests (`at`); `turn`: whether the walk may turn
    /// to the other relation's rows (not where a delta has no lookup of the column).
    fn joins_none_in(&self, deltas: &[&Arc<dyn crate::code_rule_dsl::facts::vtab::Source>], ri: usize, own: &[(usize, Allowed)], sources: &[(Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, Option<Vector<usize>>)], (j, tests): (usize, &[(usize, Allowed)]), turn: bool) -> Option<bool> {
        let at = |c: usize, cols: &Option<Vector<usize>>| cols.as_ref().map_or(Some(c), |m| m.get(c).copied());
        // Each source's joined column and tested columns, through the columns of the input it is a view of.
        let probes: Vec<(&Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, usize, Vec<usize>)> = sources.iter().map(|(s, cols)| Some((s, at(j, cols)?, tests.iter().map(|(c, _)| at(*c, cols)).collect::<Option<Vec<usize>>>()?))).collect::<Option<_>>()?;
        let read: Vec<usize> = std::iter::once(ri).chain(own.iter().map(|(c, _)| *c)).collect();
        let mut got = vec![0i64; tests.len()];
        // The delta first, for as many rows as the other relation has: a stratum the delta reaches finds its witness
        // in its first rows. Where none is found by then and the other relation is the smaller, its rows are walked
        // and each looked up in the delta, through the delta's own index (a stratum's few rows against an insert's
        // renumbered nodes). Each probe reads at most twice the smaller side.
        let delta_rows: usize = deltas.iter().map(|d| d.len()).sum();
        let other_rows: usize = probes.iter().map(|(s, ..)| s.len()).sum();
        let budget = if turn && other_rows < delta_rows { other_rows } else { usize::MAX };
        let mut walked = 0usize;
        let mut none = Some(true);
        for d in deltas {
            d.for_rows_in(0..d.len(), &read, &mut |_, row| {
                walked += 1;
                if walked > budget {
                    return false;
                }
                if !allowed(&row[1..], own) {
                    return true;
                }
                let value = row[0];
                for (s, sj, tcols) in &probes {
                    // A view of the tape gives each match's values from what found it; a kept index, its rows' slice;
                    // else the source's own lookup.
                    let mut seen = false;
                    if s.lookup_read(*sj, value, tcols, &mut |_, vals| seen |= allowed(vals, tests)) {
                        if seen {
                            none = Some(false);
                            return false;
                        }
                        continue;
                    }
                    let hit = |row: usize, got: &mut [i64]| {
                        s.row_at(row, tcols, got);
                        allowed(got, tests)
                    };
                    let found = if let Some(rows) = s.sorted_rows(*sj, value) {
                        rows.iter().any(|&r| tests.is_empty() || hit(r as usize, &mut got))
                    } else if let Some(rows) = s.lookup(*sj, &[value]) {
                        rows.into_iter().any(|r| tests.is_empty() || hit(r, &mut got))
                    } else {
                        none = None;
                        return false;
                    };
                    if found {
                        none = Some(false);
                        return false;
                    }
                }
                true
            });
            if none != Some(true) {
                return none;
            }
        }
        if walked <= budget {
            return none;
        }
        let mut own_got = vec![0i64; own.len()];
        let own_cols: Vec<usize> = own.iter().map(|(c, _)| *c).collect();
        for (s, sj, tcols) in &probes {
            let cols: Vec<usize> = std::iter::once(*sj).chain(tcols.iter().copied()).collect();
            s.for_rows_in(0..s.len(), &cols, &mut |_, row| {
                if !allowed(&row[1..], tests) {
                    return true;
                }
                for d in deltas {
                    let mut hit = |r: usize| {
                        d.row_at(r, &own_cols, &mut own_got);
                        allowed(&own_got, own)
                    };
                    let found = if let Some(rows) = d.sorted_rows(ri, row[0]) {
                        rows.iter().any(|&r| hit(r as usize))
                    } else if let Some(rows) = d.lookup(ri, &[row[0]]) {
                        rows.into_iter().any(hit)
                    } else {
                        none = None;
                        return false;
                    };
                    if found {
                        none = Some(false);
                        return false;
                    }
                }
                true
            });
            if none != Some(true) {
                break;
            }
        }
        if none.is_none() {
            return self.joins_none_in(deltas, ri, own, sources, (j, tests), false);
        }
        none
    }
}

/// How many times each variable of `rule` occurs, in its head and its body's atoms and comparisons (decision 0563 B:
/// a variable used once is read as `_`). `None` where a literal holds variables not counted here (an aggregate, a
/// nearest ancestor, an AST match, a function term): then every column but a `_` is read.
fn var_uses(rule: super::program_image::RuleRef<'_>) -> Option<HashMap<VarSlot, usize>> {
    let mut uses: HashMap<VarSlot, usize> = HashMap::new();
    let count = |t: TermRef<'_>, uses: &mut HashMap<VarSlot, usize>| match t {
        TermRef::Var(x) => {
            *uses.entry(x).or_default() += 1;
            true
        }
        TermRef::Call(..) => false,
        _ => true,
    };
    for t in rule.head().args() {
        if !count(t, &mut uses) {
            return None;
        }
    }
    for l in rule.body() {
        match l.view() {
            Lit::Atom(a) => {
                for t in a.args() {
                    if !count(t, &mut uses) {
                        return None;
                    }
                }
            }
            Lit::Cmp { l, r, .. } => {
                if !count(l, &mut uses) || !count(r, &mut uses) {
                    return None;
                }
            }
            _ => return None,
        }
    }
    Some(uses)
}

fn stratum_reads(f: Fields<'_>, vs: &body::Versions, scc: &super::front::Comp, rules: &[RuleKey]) -> BTreeSet<String> {
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

fn retainable_stratum(f: Fields<'_>, vs: &body::Versions, scc: &super::front::Comp, rules: &[RuleKey], views: &HashSet<String>) -> bool {
    !rules.is_empty()
        && !scc.iter().any(|r| f.native(r).is_some() || f.lattice(r) || views.contains(r) || f.produced(r))
        && !rules.iter().any(|&r| vs.rule_at(r).1.body().any(|l| matches!(l.view(), Lit::Ast { .. })))
}

/// Old frames could record a nonempty answer digest beside an unmaterialized, empty payload.
fn answer_read(p: &str) -> String {
    format!("answer:v2:{p}")
}

/// A set of answered rows' digest (`rows_digest`, of no rows when none), of relation `p`'s content (`Cells`).
fn answers_digest(f: Fields<'_>, db: &Connection, p: &str, rows: Option<&BTreeSet<Vector<i64>>>) -> u64 {
    let Some(rows) = rows.filter(|r| !r.is_empty()) else { return rows_digest(&Cells::plain(0), &[], 0) };
    let width = rows.iter().next().map_or(0, |v| v.len());
    let flat: Vector<i64> = rows.iter().flatten().copied().collect();
    let atoms = crate::content_store::atoms::Atoms::for_connection(db);
    rows_digest(&Cells::new(&atoms, &stored_types(f, p, vec![b'i'; width])), &flat, rows.len())
}

/// A stratum's reads with its native step's answers (one digest per produced relation), when
/// it makes requests of its own.
fn with_answers(f: Fields<'_>, db: &Connection, scc: &super::front::Comp, reads: Option<Vec<(String, u64)>>, log: Option<BTreeMap<String, BTreeSet<Vector<i64>>>>) -> Option<Vec<(String, u64)>> {
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

/// A stratum's relation's rows as `KeptStrata::ran` takes them: the engine's relation, read where its rows lie
/// (decision 0353 step 8), or SQL's answers in a block of their own (rows the engine does not hold).
pub enum RanRows {
    /// Rows read where they lie (SQL's in a block of their own, `produced_rows`).
    Picked(Box<dyn crate::content_store::kept_facts::KeptPicked>),
    /// The engine's relation, lent whole where its rows lie.
    Held(Arc<super::native::component::InRel>),
}

/// After a stratum ran: each of its relations' digest and rows, given to `ks` with what the stratum read
/// (`reads`, each with its digest then), and its digest to this run's `fresh`.
fn kept_strata_ran(ks: &mut dyn KeptStrata, db: &Connection, run: &Run, scc: &super::front::Comp, reads: Option<&Vec<(String, u64)>>, fresh: &mut HashMap<String, Option<u64>>) {
    if scc.contains("source_language_classification") {
        crate::code_rule_dsl::prof::count("source_language.evaluations", 1);
    }
    let mut members = Vec::new();
    let f = run.fields();
    for r in scc.iter().filter(|r| !run.views.contains(*r)) {
        let atoms = crate::content_store::atoms::Atoms::for_connection(db);
        // Its rows where they lie, and their digest: the engine's relation lent whole where its rows lie, its digest
        // taken there (`RanRows::Held`, decision 0353 step 8); SQL's written into a block of their own as the query
        // gives them (`produced_rows`).
        let got = if run.native_rels.contains(r) {
            super::native::component::kept_rel(db, r).and_then(|rel| {
                let cells = Cells::new(&atoms, &stored_types(f, r, rel.types().to_vec()));
                Some((rel.types().to_vec(), source_rows::held_digest(&cells, &rel)?, rel.rows(), RanRows::Held(rel)))
            })
        } else if f.produced(r) && !f.is_input(r) {
            // SQL holds the answers when the relation is outside the native component.
            produced_rows(db, run, r).map(|(types, rows)| {
                let cells = Cells::new(&atoms, &stored_types(f, r, types.clone()));
                let (n, w) = (rows.len(), types.len());
                let mut sum = 0u64;
                let mut row = vec![0i64; w];
                for i in 0..n {
                    for (c, v) in row.iter_mut().enumerate() {
                        *v = rows.get(i, c);
                    }
                    sum = sum.wrapping_add(cells.sum(&row));
                }
                (types, sum ^ (n as u64).wrapping_mul(0xff51_afd7_ed55_8ccd), n, RanRows::Picked(Box::new(rows)))
            })
        } else {
            None
        };
        let digest = got.as_ref().map(|g| g.1);
        fresh.insert(r.to_owned(), digest);
        members.push((r.to_owned(), digest));
        ks.ran(r, got.zip(reads).map(|((types, digest, n, rows), reads)| (digest, stored_types(f, r, types), rows, n, reads.clone())));
    }
    for v in scc.iter().filter(|r| run.views.contains(*r)) {
        fresh.insert(v.to_owned(), view_digest(reads.map(|v| v.as_slice()), &members));
    }
}

/// A produced relation's rows as its table holds them, typed as the program declares it (no rows without a table).
fn produced_rows(db: &Connection, run: &Run, rel: &str) -> Option<(Vec<u8>, crate::content_store::kept_facts::BlockFlat)> {
    let types: Vec<u8> = run.fields().types(rel).iter().map(|t| sym_char(&run.model.inputs, t)).collect::<String>().into_bytes();
    let width = types.len();
    let cols = (0..width).map(|c| format!("c{c}")).collect::<Vec<_>>().join(", ");
    let from = format!("SELECT DISTINCT {cols} FROM {}", crate::code_rule_dsl::logic::sql::ident(rel));
    // Counted first, so its rows go straight into a block of their size, none gathered on the heap.
    let Ok(n) = db.query_row(&format!("SELECT COUNT(*) FROM ({from})"), [], |r| r.get::<_, i64>(0)) else {
        return Some((types, crate::content_store::kept_facts::BlockFlat::new(0, width, false)));
    };
    let mut flat = crate::content_store::kept_facts::BlockFlat::new(usize::try_from(n).ok()?, width, false);
    let mut stmt = db.prepare(&from).ok()?;
    let mut rows = stmt.query([]).ok()?;
    let mut i = 0;
    while let Some(row) = rows.next().ok()? {
        for c in 0..width {
            flat.put(i, c, row.get::<_, i64>(c).ok()?);
        }
        i += 1;
    }
    Some((types, flat))
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
pub fn identities(f: Fields<'_>, vs: &body::Versions, sccs: &crate::code_rule_dsl::logic::front::Components, by_stratum: &[Vec<RuleKey>]) -> HashMap<String, [u8; 32]> {
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
        let mut hashes: Vector<u64> = rules.iter().map(|&r| vs.at(r).rule.hash()).collect();
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
            out.insert(rel.to_owned(), h.finalize().into());
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
    pub fn identities_by_heads(&self, sccs: &crate::code_rule_dsl::logic::front::Components) -> Result<HashMap<String, [u8; 32]>> {
        let p = &self.placed;
        let at: HashMap<&str, usize> = sccs.iter().enumerate().flat_map(|(i, scc)| scc.iter().map(move |r| (r, i))).collect();
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
    run.model.stats.card.insert(rel, (n as f64).max(1.0));
    for (c, (d, fan)) in cols.into_iter().enumerate() {
        run.model.ndv.insert((rel.to_owned(), c), d);
        run.model.stats.fan.insert((rel.to_owned(), c), fan);
        run.model.stats.dfreq.remove(&(rel.to_owned(), c));
    }
    Ok(n)
}

/// A clean relation's kept rows (`KeptStrata::rows`), as the requests it answers again take them.
fn kept_strata_rows(ks: &mut dyn KeptStrata, f: Fields<'_>, rel: &str) -> Result<super::native::component::Asked> {
    let types = vec![b'i'; f.types(rel).len()];
    let (_, held) = ks.rows(rel, &types).ok_or_else(|| Error::Plan(format!("{rel}: kept as clean, but its rows are not kept")))?;
    if types.is_empty() {
        return Ok(super::native::component::Asked::default());
    }
    // The requests a replayed stratum asks again, read where the kept rows lie (decision 0353 step 8).
    Ok(super::native::component::Asked::of_rel(&held))
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
    let Some(width) = rows.first().map(|v| v.len()) else { return Ok(()) };
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
    use super::program_image::{ArchivedNativeSpec, NativeSpec};
    let mut views = Vec::new();
    // Resident consumers need facts, not a SQL view that must be read back at each chunk.
    // Other native operators still use SQL-owned inputs and keep their projection views.
    let native_reads: HashSet<&str> = if model.inputs.atoms.is_some() && model.inputs.native.contains("program") {
        f.natives().iter().filter(|n| matches!(n.spec, ArchivedNativeSpec::Python { .. } | ArchivedNativeSpec::PythonCandidates { .. })).flat_map(|n| n.reads.iter().map(|&r| f.rel_name(r))).collect()
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
        model.view_src = model.view_src.with([(rel.to_owned(), (arel.to_owned(), cols))]);
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
                model.docof = model.docof.with([(rel.to_owned(), (pos(*n), pos(*d)))]);
            }
        }
    }
    Ok(views)
}

/// States that the runs on `db` have documents whose nodes are `d << 32 | index` (the corpus runner's): a range read
/// computes a document's interval, so no table of them is written or read in the run's waves.
pub fn docs_by_shift(db: &Connection) {
    left_on(db).lock().by_shift = true;
}

pub fn by_shift_of(db: &Connection) -> bool {
    by_shift(db)
}

/// `temp_store` in memory, set once a connection.
fn temp_in_memory(db: &Connection) -> Result<()> {
    let left = left_on(db);
    if left.lock().configured {
        return Ok(());
    }
    // Set when SQL first runs on the connection (`vtab::setup_now`): a run all native runs no statement.
    crate::code_rule_dsl::facts::vtab::defer_setup(db, "PRAGMA temp_store = MEMORY;");
    left.lock().configured = true;
    Ok(())
}

/// At a run's end: its connection's `Left::temp` is set when the run made a table or a view, or kept one. While the run
/// lasts, SQLite's authorizer on its connection notes each table or view made (`TempLeft::watch`), with no statement
/// of its own; the run's end takes it away.
struct TempLeft {
    left: std::sync::Arc<parking_lot::Mutex<Left>>,
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
    fn watch(db: &Connection) -> Self {
        let made = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        // SAFETY: the handle is the open connection's, which outlives the run (the guard is the run's local).
        let handle = unsafe { db.handle() };
        unsafe { rusqlite::ffi::sqlite3_set_authorizer(handle, Some(note_made), std::sync::Arc::as_ptr(&made) as *mut std::ffi::c_void) };
        TempLeft { left: left_on(db), made, db: handle }
    }
}

impl Drop for TempLeft {
    fn drop(&mut self) {
        // SAFETY: as in `watch`; no callback reads `made` once this returns.
        unsafe { rusqlite::ffi::sqlite3_set_authorizer(self.db, None, std::ptr::null_mut()) };
        let made = self.made.load(std::sync::atomic::Ordering::Relaxed);
        self.left.lock().temp = made;
    }
}

/// Runs program `program`, its version (a whole image: decision 0173, t36 item 7 step 8), over `db`: `lowering` the
/// lowering's domains where the program is lowered (decision 0009).
pub fn run(program: &body::Placed, db: &Connection, inputs: Inputs, lowering: Option<lower::Lowering<'_>>, hooks: Hooks<'_>) -> Result<Outcome> {
    // The run's own state (decision 0441): the caller's run's if one is under way here, else this run's own, returned
    // with its outcome.
    let own = super::run::scope(super::native::width::Settings::from_env);
    let mut outcome = run_within(program, db, inputs, lowering, hooks)?;
    outcome.run = own.finish();
    Ok(outcome)
}

/// `run`, within the run's own state (`run::scope`).
fn run_within(program: &body::Placed, db: &Connection, inputs: Inputs, lowering: Option<lower::Lowering<'_>>, mut hooks: Hooks<'_>) -> Result<Outcome> {
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
    if inputs.atoms.is_some() && inputs.given.is_some() && with_plans(|p|p.is_some())
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
    let left = left_on(db).lock().temp;
    let temp: Vec<(String, String, String)> = if !left {
        Vec::new()
    } else {
        db.prepare("SELECT type, name, coalesce(sql, '') FROM temp.sqlite_master WHERE type IN ('table', 'view')")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?
    };
    let _left_after = TempLeft::watch(db);
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
    let corpus = with_plans(|p| p.is_some());
    // Both rewrites below keep the program's meaning and read every rule: a corpus run takes the choices an earlier
    // run or chunk made for the same program at its scale or more (`choices`, decision 0090 §5).
    // A program of no rules (a cross part whose every stratum is kept) has no choice to make, so none is kept for it:
    // keeping one wrote to the store in the run after a cold one, for a program no other run plans (decision 0250).
    // So too a program whose every rule is a view of an input's columns (Q37): held in place, never kept, it is what
    // is left of a cross part kept whole otherwise.
    let choosing = program.len() > 0 && normal_v.fields().ok().is_none_or(|f| normal_v.v.rules().any(|r| input_view_shape(&normal_v.v, r, |x| f.is_input(x)).is_none()));
    let chosen_key = (corpus && choosing).then(|| choices::key(&normal_v));
    let reused = chosen_key.as_ref().and_then(|k| choices::lookup(k, model.stats.nodes));
    // The relations inlined, for the outcome (a session stores, never inlines, what it keeps: decision 0099).
    let inlined_rels: BTreeSet<String>;
    // The program's version as the passes chose it (t36 item 6): their overlay, or the kept choice's, which the run
    // reads its rules from as it is; the program's own where nothing was rewritten.
    let placed: body::Placed = match reused {
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
            match c.version {
                // The program run is the rewritten one: its analysis is that program's, made when asked.
                Some(v) => {
                    crate::threads::drop_behind(std::mem::take(&mut model.analysis));
                    v
                }
                // Not rewritten: the program's own version.
                None => normal_v,
            }
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
            rewritten.unwrap_or(normal_v)
        }
    };
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
    // A maintained run keeps the strata as the version has them (its kept rows and their levels are by stratum).
    let joined = (hooks.maintain.is_none() && model.inputs.native.contains("program")).then(|| climb_ends(&p0, &sccs, &deps)).flatten();
    let sccs = match joined {
        Some(joined) => Arc::new(joined),
        None => sccs,
    };
    // The ordering edges hold for the whole run: a stratum that asks suspends (decision 0114 §3), and a stratum is
    // run meanwhile only where it does not depend on one asking (`issue::Board`, the waiting checks). After the
    // climb's ends are joined: a stratum moved into its walk's is in a request's cone with it or outside with it.
    let (first, ordered) = requests_first(&sccs, &deps, &f0);
    let sccs = first.map_or(sccs, Arc::new);
    let deps = ordered.unwrap_or(deps);
    let (order, level_of) = by_level(&sccs, &deps, model.inputs.native.contains("program"));
    let sccs = match order {
        Some(o) => Arc::new(sccs.reordered(o.iter().copied())),
        None => sccs,
    };
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
    // The reach by kind and key (decision 0563 B); `RERAIL_REACH_BY_KEY=off` reaches by relation alone, to compare.
    let by_key = std::env::var("RERAIL_REACH_BY_KEY").map_or(true, |x| x != "off");
    let live = std::cell::RefCell::new(NetDeltas::new());
    // Reach indexes made again whose old rows are kept in memory: their deltas made once they are (`kept_delta`).
    let mut reach_olds: HashMap<String, Arc<super::native::component::InRel>> = HashMap::new();
    // A relation derived again whose delta is its rows gone and added (`Moved::Rows`): its minus and plus versions,
    // given to its readers (`ins`).
    let mut diffs: HashMap<String, [Arc<super::native::component::InRel>; 2]> = HashMap::new();
    // A changed read's versions as strata read them (`unchanged_as_read`), by its minus version and the columns read:
    // made once for all the strata that read the same columns. The minus version is held by its entry, so its address
    // is no other's while the entry lives.
    type AsRead = (Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, Option<[Arc<dyn crate::code_rule_dsl::facts::vtab::Source>; 2]>);
    let mut as_read: HashMap<(usize, Vec<usize>), AsRead> = HashMap::new();
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
    let mut run = Run { db: crate::code_rule_dsl::facts::vtab::Db::of(db), model, views: Default::default(), made: HashSet::new(), orders: HashMap::new(), keyless: HashSet::new(), native_rels: adopted, spike_bare: Default::default(), device_views: HashSet::new(), proofs: Default::default(), mode_reads: Default::default(), demands: Default::default(), pending: HashMap::new(), why_not: None, projections: Arc::new(projections0), inputs: Arc::new(RunInputs { program: f0.inputs().collect(), added: Default::default() }), versions: Arc::new(body::Versions::of_placed(placed)), version: 0, answer: hooks.answer.take(), texts: hooks.texts.take(), asking: None, why_class: None, answered: BTreeMap::new(), logging: None, answer_logs: HashMap::new(), quiet: Default::default() };
    if let Some(p) = maintaining.as_deref().and_then(|m| m.proofs()) {
        let _ = run.proofs.set(p);
    }
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
        db.execute_batch(&format!("DROP VIEW IF EXISTS temp.{}", crate::code_rule_dsl::logic::sql::ident(name)))?;
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
            let mut rows: Vector<i64> = rerail_blocks::vector::Vector::with_capacity(facts.len() * cols.len());
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
            let mut unique: Vector<i64> = rerail_blocks::vector::Vector::with_capacity(rows.len());
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
            run.model.stats.card.insert(rel, (n as f64).max(1.0));
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
    let stratum_of: HashMap<&str, usize> = sccs.iter().enumerate().flat_map(|(j, scc)| scc.iter().map(move |r| (r, j))).collect();
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
        stays.extend(sccs.iter().flatten().map(str::to_owned));
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
    let wants_of: Vec<BTreeSet<(String, Vector<usize>)>> = by_stratum
        .iter()
        .zip(sccs.iter())
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
                .collect::<BTreeSet<(String, Vector<usize>)>>()
        })
        .collect();
    let mut waved = vec![false; sccs.len()];
    // Strata waiting on their device wave: position, key, driver's state.
    let mut waiting: Vec<(usize, String, Box<Driving>)> = Vec::new();
    // Strata suspended on their requests (decision 0114 §3), and the answers come for them so far, by ticket.
    let mut asked: Vec<Asked> = Vec::new();
    let mut arrived: HashMap<u64, Vec<(String, Vec<Vector<i64>>)>> = HashMap::new();
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
    let mut evaluated: Vector<usize> = rerail_blocks::vector::Vector::new();
    let (mut clean_strata, mut held_again, mut held_rows) = (0usize, 0usize, 0usize);
    // Decision 0113: native strata issued ahead, as components on the executor's host stream, retired here in order.
    // Not in a run in mode `here`: a nested run whose caller is already one of the pool's jobs (a rules pass, decision
    // 0679 §5.1) runs its strata on its own thread, paying no job, ambient or board a stratum.
    let mut board = (issue::on() && native.contains("program") && !native.contains("here") && strata_kept.is_none() && maintaining.is_none()).then(|| issue::Board::new(&sccs, &deps));
    // Decision 0592: with strata issued ahead, a relation goes when its last reader to finish does.
    let mut reads_left = board.as_ref().filter(|_| release_finished()).map(|_| ReadsLeft::new(f0, &run_vs, &sccs, &by_stratum, &run.views));
    let mut early = Early::default();
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
                let theirs: HashSet<&str> = asked.iter().flat_map(|a| sccs[a.at].iter()).collect();
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
        if let Some(rl) = reads_left.as_mut() {
            let mut gone = rl.passed(at, &|j| asked.iter().any(|a| a.at == j));
            if let Some(b) = board.as_mut() {
                gone.extend(rl.finished(b.take_finished()));
            }
            release_unread(db, &mut run, gone, &pinned, &held, &input_set, &last, at, &mut early);
        }
        if !waiting.is_empty() {
            let reads_waiting = || {
                let theirs: HashSet<&str> = waiting.iter().flat_map(|w| sccs[w.0].iter()).collect();
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
            let (done, still): (Vector<usize>, Vector<usize>) = std::mem::take(&mut evaluated).into_iter().partition(|k| !waiting.iter().any(|w| w.0 == *k));
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
            c.extend(sccs[at - 1].iter().filter(|r| !last.contains_key(*r)).map(str::to_owned));
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
        // SPIKE: the chunk's held copies of those no later stratum reads, kept for the caller or not.
        run.spike_release(&candidates.iter().filter(|r| last.get(*r).is_none_or(|&l| l < at)).cloned().collect::<Vec<_>>());
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
                    match kept_delta(db, &r, &old)? {
                        // Made a version of its kept rows (`InRel::rebased`): its change is its marks, as a maintained
                        // relation's.
                        Moved::Marks => {
                            m.delta_made(&r);
                            if let Some((gone, added)) = super::native::component::held(db, &r).and_then(|h| h.version_counts()) {
                                outcome.deltas.insert(r.clone(), Delta::Marks { gone, added });
                            }
                        }
                        Moved::Rows(gone, added) => {
                            m.delta_made(&r);
                            diffs.insert(r.clone(), diff_versions(&old, &gone, &added));
                            outcome.deltas.insert(r.clone(), Delta::Rows { gone, added });
                        }
                        Moved::Not => {
                            moved_rels.remove(&r);
                            outcome.unchanged.insert(r);
                        }
                    }
                }
                let end = (at..sccs.len()).find(|&j| level_of[j] != level_of[at]).unwrap_or(sccs.len());
                prepared_to = end;
                let _level = super::profile::span(|| format!("a level of {} strata placed", end - at));
                if let Some(load) = hooks.load.as_deref_mut() {
                    for slot in &mut loads[at + 1..end] {
                        let now: Vec<String> = std::mem::take(slot);
                        if !now.is_empty() {
                            load(&now)?;
                        }
                    }
                }
                let mut wants: BTreeSet<(String, Vector<usize>)> = BTreeSet::new();
                let mut specs: Vec<PlanJoin> = Vec::new();
                // Placed: held as kept (in memory, or left stored, unread), run as the base program's, or reached.
                let mut base_strata: Vector<usize> = rerail_blocks::vector::Vector::new();
                let mut reached_strata: Vec<(usize, Vec<RuleKey>, BTreeSet<String>)> = Vec::new();
                let mut need: BTreeSet<String> = BTreeSet::new();
                for j in at..end {
                    let scc = &sccs[j];
                    let rules: Vec<RuleKey> = scc.iter().flat_map(|rel| rules_by.get(rel).into_iter().flatten().copied()).collect();
                    let reads = stratum_reads(f0, &run_vs, scc, &rules);
                    let moved: BTreeSet<String> = reads.iter().filter(|r| !scc.contains(*r) && moved_rels.contains(r.as_str())).cloned().collect();
                    // Decision 0563 B: a stratum none of whose rules sees a row of its changed reads' deltas, by kind
                    // or by key, is passed on as one no read of which changed. A native's stratum and a view of an
                    // input are left to their own paths.
                    let unseen = by_key
                        && !moved.is_empty()
                        && !scc.iter().any(|r| f0.native(r).is_some())
                        && run.view_shaped(scc, &rules).is_none()
                        && Reach { diffs: &diffs, db, f0, rules_by: &rules_by, run_vs: &run_vs, moved: &moved, scc, live: &live }.cuts(&mut *m, &mut run.model, &rules);
                    let moved = if unseen { BTreeSet::new() } else { moved };
                    if unseen {
                        outcome.unseen.push(stratum_key(scc));
                    }
                    waved[j] = true;
                    if moved.is_empty() {
                        let no_facts = scc.iter().all(|r| !f0.has_facts(r));
                        let in_memory: Option<Vec<(&str, Arc<super::native::component::InRel>)>> = scc.iter().map(|r| m.kept(r).map(|k| (r, k))).collect();
                        let placed = match in_memory.filter(|_| no_facts) {
                            Some(kept) => {
                                for (r, o) in kept {
                                    hold_quiet(&mut run, f0, db, r, o);
                                }
                                true
                            }
                            // Stored: read only if a stratum that runs reads it, or the caller does.
                            None if no_facts && scc.iter().all(|r| m.keeps(r)) => {
                                unread.extend(scc.iter().map(str::to_owned));
                                true
                            }
                            None => false,
                        };
                        if placed {
                            quiet_slots += 1;
                            outcome.unchanged.extend(scc.iter().map(str::to_owned));
                            super::profile::stratum(&stratum_key(scc), "native set engine", || if unseen { "quiet: no rule sees a row its changed reads' deltas gained or lost, by kind or key (decision 0563 B); each relation is its kept one".to_owned() } else { "quiet: no read changed; each relation is its kept one (decision 0117 step 3)".to_owned() });
                            slots[j] = Slot::Quiet;
                            continue;
                        }
                        // Not kept: derived from its reads, as they were, so its rows are as they were.
                        need.extend(reads.iter().filter(|r| !scc.contains(*r)).cloned());
                        base_strata.push(j);
                        outcome.again.push((stratum_key(scc), crate::code_rule_dsl::logic::plan::again::NOT_KEPT));
                        continue;
                    }
                    // A native operator's stratum (the reach index, decision 0119) has no maintained form: it is made again
                    // from its reads as they are now, as the base program makes it, and its readers derive again. A text
                    // test's has (`maintain`: only the new strings tested). A reach index whose old rows are kept in
                    // memory (a chunk's guest rounds, decision 0331) is made the same way, and its delta is then made
                    // against them (`reach_delta`), so its readers are maintained.
                    if scc.iter().any(|r| f0.native(r).is_some_and(|n| !matches!(n.spec, super::program_image::ArchivedNativeSpec::TextTest { .. }))) {
                        need.extend(reads.iter().filter(|r| !scc.contains(*r)).cloned());
                        base_strata.push(j);
                        outcome.again.push((stratum_key(scc), crate::code_rule_dsl::logic::plan::again::NATIVE_OPERATOR));
                        for r in scc {
                            if let Some(old) = m.kept(r).filter(|_| matches!(f0.native(r).map(|n| &n.spec), Some(super::program_image::ArchivedNativeSpec::Reach { .. } | super::program_image::ArchivedNativeSpec::ReachThrough { .. }))) {
                                reach_olds.insert(r.to_owned(), old);
                            } else {
                                m.no_delta(r);
                                moved_rels.insert(r.to_owned());
                            }
                        }
                        continue;
                    }
                    need.extend(reads.iter().cloned());
                    need.extend(scc.iter().map(str::to_owned));
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
                    // A view of an input's columns (Q37) keeps nothing: it is held again in place, which is no work,
                    // and its version is its input's, read through its columns (its readers' `ins`, below). The mode's
                    // trivial case: maintained, not derived again, whatever its delta.
                    if run.view_shaped(scc, &rules).is_some() {
                        if std::env::var_os("RERAIL_MODE_TRACE").is_some() {
                            eprintln!("RERAIL_MODE_TRACE {key}: a view of its input, held again in place");
                        }
                        outcome.maintained.0 += 1;
                        moved_rels.extend(scc.iter().map(str::to_owned));
                        continue;
                    }
                    // The evaluator's maintain mode (decision 0364 stage 3), where the stratum can take it: no form.
                    let olds: BTreeMap<String, Arc<super::native::component::InRel>> = scc.iter().filter_map(|r| m.kept(r).map(|k| (r.to_owned(), k))).collect();
                    // Its changed inputs' versions: the old rows the last run kept, and the deltas bound from the
                    // changed documents (`input_delta`).
                    // A view of an input's columns has its input's, through its columns. A relation derived again that
                    // could not be made a version of its kept rows (`kept_delta`) has its kept rows as old, and its rows
                    // gone and added as minus and plus (`diffs`).
                    let ins: ModeInputs = moved.iter().filter_map(|r| moved_versions(r, &diffs, &mut *m, db, f0, &rules_by, &run_vs).map(|got| (r.clone(), got))).collect();
                    // Each as the stratum reads it: its rows gone and added that agree on the columns its rules read are
                    // no change to it (`unchanged_as_read`). A view's columns are its source's through `cols`.
                    let ins: ModeInputs = ins.into_iter().map(|(r, ([o, mi, pl, now], cols))| {
                        let used: Option<Vec<usize>> = run.read_columns(&rules, &r).and_then(|u| u.into_iter().map(|c| cols.as_ref().map_or(Some(c), |m| m.get(c).copied())).collect());
                        let Some(used) = used.filter(|u| u.len() < mi.columns().len()) else { return (r, ([o, mi, pl, now], cols)) };
                        let (_, made) = as_read.entry((Arc::as_ptr(&mi) as *const u8 as usize, used.clone())).or_insert_with(|| {
                            let made = unchanged_as_read(&mi, &pl, &used);
                            if let Some([m2, p2]) = &made {
                                super::profile::decision("a changed read's rows that agree on the columns its strata read (0090 §4)", || format!("{r} by columns {used:?}: {} of {} gone, {} of {} added", m2.len(), mi.len(), p2.len(), pl.len()));
                            }
                            (Arc::clone(&mi), made)
                        });
                        match made.as_ref() {
                            Some([m2, p2]) => (r, ([o, Arc::clone(m2), Arc::clone(p2), now], cols)),
                            None => (r, ([o, mi, pl, now], cols)),
                        }
                    }).collect();
                    // 0090 §4's threshold (0364 R4): a stratum whose changed reads' deltas are past θ of those reads'
                    // stored rows is derived again, its levels fresh from that derive. A derive reads its inputs whole
                    // and the mode reads their deltas, so the share of the reads that changed is what the two cost
                    // apart. (Measured against the stratum's own kept rows, a selective stratum over the whole tree,
                    // `rr_cover41` with 644 rows over `child`, `docof`, `col`, was derived again at every 2-file edit
                    // of the data layer: the edit cost the first run's 500 s at 2,410 files.)
                    // An input's rows read are its rows now, measured as its delta is: a changed document's every row
                    // (`input_delta`), against every document's. A semi-join's planned rows (`model.stats.card`, at the
                    // nodes the exports hold) are not its delta's measure: an import that moved every line of a file
                    // was 508 of `line`'s 744 rows at the held nodes, and derived again. Its `len` makes no document:
                    // the corpus batch answers it by its counted total (`give_totals`, `held_total`, Q332).
                    let read_rows = |_: &str, now: &Arc<dyn crate::code_rule_dsl::facts::vtab::Source>| now.len();
                    let sizes: BTreeMap<String, (usize, usize)> = moved.iter().map(|r| (r.clone(), match ins.get(r) {
                        Some((srcs, _)) => (srcs[1].len() + srcs[2].len(), read_rows(r, &srcs[3])),
                        None => super::native::component::held(db, r).map_or((0, 0), |x| {
                            ([super::native::component::Ver::Minus, super::native::component::Ver::Plus].into_iter().filter_map(|v| x.version_view(v)).map(|v| v.rows()).sum(), x.rows())
                        }),
                    })).collect();
                    let (delta_rows, read_rows) = sizes.values().fold((0, 0), |(d, n), (a, b)| (d + a, n + b));
                    let kept_rows: usize = olds.values().map(|o| o.rows()).sum();
                    // §4's second clause: the mode's lines priced against a derive's. Priced only where the first
                    // clause and the mode's own blocks leave it maintained, and only past the floor (below it, a
                    // maintain costs little whatever the plans say).
                    let mut priced = None;
                    let blocks = if crate::code_rule_dsl::logic::plan::again::past_threshold(delta_rows, read_rows) {
                        Some(crate::code_rule_dsl::logic::plan::again::THRESHOLD.to_owned())
                    } else {
                        match run.maintain_mode_blocks(scc, &rules, &moved, &olds, &ins) {
                            None if crate::code_rule_dsl::logic::plan::again::past_floor(delta_rows) => {
                                let deltas: BTreeMap<String, usize> = sizes.iter().map(|(r, (d, _))| (r.clone(), *d)).collect();
                                let (mw, dw) = run.mode_costs(&rules, &deltas)?;
                                priced = Some((mw, dw));
                                crate::code_rule_dsl::logic::plan::again::costs_more(mw, dw).then(|| crate::code_rule_dsl::logic::plan::again::THRESHOLD.to_owned())
                            }
                            other => other,
                        }
                    };
                    if std::env::var_os("RERAIL_MODE_TRACE").is_some() {
                        let costs = priced.map_or_else(String::new, |(m, d)| format!(", mode {m:.0} against derive {d:.0}"));
                        let mut by: Vec<_> = sizes.iter().filter(|(_, (d, _))| *d > 0).map(|(r, (d, n))| format!("{r} {d}/{n}")).collect();
                        by.truncate(4);
                        eprintln!("RERAIL_MODE_TRACE {key}: {} ({delta_rows} delta rows of {read_rows} read, {kept_rows} kept{costs}; {})", blocks.as_deref().unwrap_or("the maintain mode"), by.join(", "));
                    }
                    match blocks {
                        None => {
                            slots[j] = Slot::Mode { rules, moved, olds, ins };
                            continue;
                        }
                        Some(why) => {
                            outcome.again.push((key.clone(), crate::code_rule_dsl::logic::plan::again::reason_class(&why)));
                            super::profile::decision("not the maintain mode (decision 0364)", || format!("{key}: {why}"))
                        }
                    }
                    // Derived again from its reads as they are now, as the base program derives it (decision 0364: no
                    // form). Its relations move; a kept one's delta is its diff against its kept rows, made once the
                    // level has run (`again_olds`), so its readers are maintained, not derived again in turn.
                    for r in scc {
                        moved_rels.insert(r.to_owned());
                        match m.kept(r) {
                            Some(old) => {
                                again_olds.insert(r.to_owned(), old);
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
                    // Maintained, whether or not its rows changed.
                    let recursive = rules.iter().any(|&r| reads_of(&run.versions, r).any(|x| scc.contains(x)));
                    outcome.maintained.0 += 1;
                    outcome.maintained.1 += usize::from(recursive);
                    if !ran {
                        outcome.unchanged.extend(scc.iter().map(str::to_owned));
                        outcome.native.insert(key);
                        continue;
                    }
                    m.mode_ran(scc, recursive && run.proof(scc).is_err());
                    for r in scc {
                        let rel = super::native::component::held(db, r).ok_or_else(|| Error::Plan(format!("{r}: not held after its maintain mode")))?;
                        let (gone, added) = rel.version_counts().ok_or_else(|| Error::Plan(format!("{r}: no version after its maintain mode")))?;
                        if added == 0 && gone == 0 {
                            outcome.unchanged.insert(r.to_owned());
                        } else {
                            moved_rels.insert(r.to_owned());
                            // Read from its marks when its frame's delta is written (`Delta::rows`), not copied now.
                            outcome.deltas.insert(r.to_owned(), Delta::Marks { gone, added });
                        }
                    }
                    outcome.native.insert(key);
                    continue;
                }
            }
        }
        let rules: Vec<RuleKey> = scc.iter().flat_map(|rel| rules_by.get(rel).into_iter().flatten().copied()).collect();
        let key = scc.iter().collect::<Vec<_>>().join("/");
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
                if run.native_stratum_now(&super::front::CompAt::of(&sccs, at), &rules)? {
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
            let asks: Vec<&str> = scc.iter().filter(|r| f0.answers_of(r).iter().any(|p| scc.contains(*p))).collect();
            // A stratum of produced relations above their requests is their native step: their rows are the answers.
            let above: Vec<(&str, &str)> = scc.iter().filter_map(|r| f0.request_of(r).filter(|q| !scc.contains(*q)).map(|q| (r, q))).collect();
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
            if clean && answering && above.iter().any(|(p, q)| !ks.has_rows(q) && !run.answered.contains_key(*p)) {
                clean = false;
            }
            let mut replayed: BTreeMap<String, BTreeSet<Vector<i64>>> = BTreeMap::new();
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
                let mut by_req: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
                for (p, q) in &above {
                    by_req.entry(*q).or_default().push(*p);
                }
                for (q, ps) in by_req {
                    if ps.iter().all(|p| run.answered.contains_key(*p)) {
                        for p in ps {
                            replayed.entry(p.to_owned()).or_default().extend(run.answered[p].iter().cloned());
                        }
                        continue;
                    }
                    let rows = kept_strata_rows(ks, f0, q)?;
                    let answer = run.answer.as_deref_mut().expect("answering");
                    for (p, got) in answer(q, &rows)? {
                        replayed.entry(p).or_default().extend(got);
                    }
                }
                for &(p, _) in &above {
                    let now = answers_digest(f0, db, p, replayed.get(p));
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
                let kept_here: Vec<&str> = scc.iter().filter(|r| !run.views.contains(*r)).collect();
                for r in kept_here {
                    fresh.insert(r.to_owned(), ks.was(r));
                    // A view's relations are held now: the view reads them in place.
                    if viewed {
                        held_rows += kept_strata_hold(ks, db, &mut run, r)?;
                    } else {
                        unheld.insert(r.to_owned());
                    }
                }
                let members: Vec<(String, Option<u64>)> = scc.iter().filter(|r| !run.views.contains(*r)).map(|r| (r.to_owned(), ks.was(r))).collect();
                for v in scc.iter().filter(|r| run.views.contains(*r)) {
                    fresh.insert(v.to_owned(), view_digest(nows.as_deref(), &members));
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
                        // The engine's relation read where its rows lie (decision 0353 step 8).
                        let native = run.native_rels.contains(&req).then(|| super::native::component::kept_rel(db, &req)).flatten();
                        let asked = if let Some(rel) = native.filter(|_| width > 0) {
                            super::native::component::Asked::of_rel(&rel)
                        } else if run.pending.contains_key(&req) && !run.native_rels.contains(&req) {
                            super::native::component::Asked::default()
                        } else {
                            run.materialize(std::slice::from_ref(&req))?;
                            let cols = (0..width).map(|c| format!("c{c}")).collect::<Vec<_>>().join(", ");
                            // SQL's rows (a relation the engine does not hold).
                            super::native::component::Asked::rows(db.prepare(&format!("SELECT {cols} FROM {}", crate::code_rule_dsl::logic::sql::ident(&req)))?
                                .query_map([], |r| (0..width).map(|c| r.get::<_, i64>(c)).collect::<rusqlite::Result<Vector<_>>>())?
                                .collect::<rusqlite::Result<_>>()?)
                        };
                        let answer = run.answer.as_deref_mut().ok_or_else(|| Error::Plan(format!("{req}: a request, and no one to answer it")))?;
                        let mut mine = Vec::new();
                        for (p, got) in answer_complete(answer, &req, asked)? {
                            if p == *rel { mine.extend(got) } else { run.answered.entry(p).or_default().extend(got) }
                        }
                        mine
                    }
                };
                if let Some(at) = run.logging {
                    run.answer_logs.entry(at).or_default().entry(rel.to_owned()).or_default().extend(rows.iter().cloned());
                }
                // A later program can reuse this producer even when the current cone has no local reader.
                // Retain its actual answers alongside their observed digest whenever this stratum publishes frames.
                if f0.is_input(rel) || rows.is_empty() {
                    continue;
                }
                if strata_kept.is_none()
                    && !body::Versions::keys(0, p0.len()).any(|r| reads_of(&run.versions, r).any(|a| a == rel))
                {
                    continue;
                }
                // Held by the native engine, as a component's own answers are (`answer_pending`): a native stratum above
                // reads it there, and a SQL one has it moved into its table (`Run::materialize`).
                let types: Vec<u8> = f0.types(rel).iter().map(|t| sym_char(&run.model.inputs, t)).collect::<String>().into_bytes();
                // `retain` replaces what the engine holds: rows it already holds (the relation's program facts) stay,
                // so its readers read the facts and the answers both.
                // Held with the rows the engine holds now, those shared where they lie (decision 0353 step 8).
                if run.native_rels.contains(rel) {
                    super::native::component::retain_with(db, rel, rows, types)?;
                } else {
                    super::native::component::retain(db, rel, rows, types)?;
                }
                run.native_rels.insert(rel.to_owned());
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
        use super::program_image::{ArchivedNativeSpec, NativeSpec};
        if atoms && scc.iter().any(|r| matches!(f0.native(r).map(|n| &n.spec), Some(ArchivedNativeSpec::Python { .. } | ArchivedNativeSpec::PythonCandidates { .. }))) {
            let rel = scc.iter().next().unwrap();
            let (kind, input, targets) = match f0.native(rel).map(|n| &n.spec) {
                Some(ArchivedNativeSpec::Python { kind, input, targets }) => (f0.text(*kind), f0.rel_name(*input), targets.map(|t| f0.rel_name(t))),
                Some(ArchivedNativeSpec::PythonCandidates { input, targets, .. }) => ("candidates", f0.rel_name(*input), Some(f0.rel_name(*targets))),
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
            && let Some(native @ (ArchivedNativeSpec::Reach { .. } | ArchivedNativeSpec::ReachThrough { .. })) = f0.native(rel).map(|n| &n.spec)
            && let Some((n, why)) = run.native_reach(rel, native)?
        {
            super::profile::stratum(&skey, "reach index over held relations (decision 0119)", || why);
            super::profile::note(format!("{rel} rows"), || n.to_string());
            outcome.native.insert(key);
            if let Some(old) = reach_olds.remove(*rel)
                && let Some(m) = maintaining.as_deref_mut()
            {
                match kept_delta(db, rel, &old)? {
                    Moved::Not => {
                        outcome.unchanged.insert((*rel).to_owned());
                    }
                    moved => {
                        if let Moved::Rows(gone, added) = &moved {
                            diffs.insert((*rel).to_owned(), diff_versions(&old, gone, added));
                        }
                        m.delta_made(rel);
                        moved_rels.insert((*rel).to_owned());
                    }
                }
            }
            continue;
        }
        // A filter every row passes is its relation (t52 step 3): `h(v…) :- x(v…), $v < b, …` (`filter_of`), where each
        // live row of `x` holds every compared column under its bound, as hiding's narrow twins in a run with no
        // annotation document. Its rows are `x`'s, held under both names: read where they lie to check, nothing
        // derived or copied. Maintained, its change is `x`'s: the same rows pass the same bounds. Not under kept
        // strata, whose kept rows are a stratum's own, nor on the device, whose strata bind their relations themselves.
        if let [rel] = scc.iter().collect::<Vec<_>>().as_slice()
            && let [r] = rules.as_slice()
            && strata_kept.is_none()
            && !placement::device_on()
            && let Some((of, bounds)) = { let (v, rule) = run.versions.rule_at(*r); plain_filter(&f0, rel, v, rule) }
            && run.native_rels.contains(&of)
            && let Some(n) = super::native::component::hold_as_where_all(db, rel, &of, |src, row| bounds.iter().all(|&(c, b)| src.value(row, c) < b))
        {
            super::profile::stratum(&skey, "a filter every row passes: its relation's rows (t52 step 3)", || format!("{of}: {n} rows"));
            super::profile::decision("filters every row passes, read as their relation", || format!("{rel} as {of}"));
            run.native_rels.insert(rel.to_string());
            if let Some(&card) = run.model.stats.card.get(of.as_str()) {
                run.model.stats.card.insert(rel.to_string(), card);
            }
            for c in 0..f0.types(rel).len() {
                let k = (of.clone(), c);
                if let Some(&d) = run.model.ndv.get(&k) {
                    run.model.ndv.insert((rel.to_string(), c), d);
                }
                if let Some(&fan) = run.model.stats.fan.get(&k) {
                    run.model.stats.fan.insert((rel.to_string(), c), fan);
                }
            }
            if let Some(m) = maintaining.as_deref_mut() {
                if !moved_rels.contains(&of) {
                    outcome.unchanged.insert(rel.to_string());
                } else if let Some(d) = diffs.get(&of).cloned() {
                    diffs.insert(rel.to_string(), d);
                    m.delta_made(rel);
                    moved_rels.insert(rel.to_string());
                } else {
                    m.no_delta(rel);
                    moved_rels.insert(rel.to_string());
                }
            }
            outcome.native.insert(key);
            continue;
        }
        if super::profile::on()
            && let [rel] = scc.iter().collect::<Vec<_>>().as_slice()
            && let [r] = rules.as_slice()
            && let Some((of, _)) = { let (v, rule) = run.versions.rule_at(*r); plain_filter(&f0, rel, v, rule) }
        {
            let why = if strata_kept.is_some() {
                "kept strata"
            } else if placement::device_on() {
                "on the device"
            } else if !run.native_rels.contains(&of) {
                "its relation not held by the engine"
            } else {
                "a row of its relation fails a bound"
            };
            super::profile::decision("filters derived, not read as their relation", || format!("{rel} of {of}: {why}"));
        }
        // A reach index not made in place (`reach_rows`) has no delta: its readers derive again.
        if let Some(m) = maintaining.as_deref_mut() {
            for r in scc.iter().filter(|r| reach_olds.remove(*r).is_some()) {
                m.no_delta(r);
                moved_rels.insert(r.to_owned());
            }
        }
        // A text test over strings the run answered, from the rows the engine holds: no table read or written. With
        // the device on, its rule runs as placed (the device's text-test form, `native_stratum_`), never on the host
        // in the device's place.
        if let [rel] = scc.iter().collect::<Vec<_>>().as_slice()
            && !placement::device_on()
            && let Some(ArchivedNativeSpec::TextTest { input, column, op, patterns }) = f0.native(rel).map(|n| &n.spec)
            && text_test_defined(&run.versions, &rules, f0.rel_name(*input))
            && let Some(n) = run.native_text_test(rel, f0.rel_name(*input), run.native_rels.contains(f0.rel_name(*input)), *column as usize, f0.text(*op), &patterns.iter().map(|&p| f0.text(p).to_owned()).collect::<Vec<_>>())?
        {
            super::profile::stratum(&skey, "text test over a held relation (the run's vocabulary)", String::new);
            super::profile::note(format!("{rel} rows"), || n.to_string());
            outcome.native.insert(key);
            continue;
        }
        // The import components (`components.rerail`) from the rows the engine holds: no table read or written.
        if let [rel] = scc.iter().collect::<Vec<_>>().as_slice()
            && let Some(ArchivedNativeSpec::Components { edge, own }) = f0.native(rel).map(|n| &n.spec)
            && let Some(n) = run.native_components(rel, f0.rel_name(*edge), f0.rel_name(*own))?
        {
            super::profile::stratum(&skey, "native fact operator over held relations", String::new);
            super::profile::note(format!("{rel} rows"), || n.to_string());
            outcome.native.insert(key);
            continue;
        }
        // A count below a bound (`rank_form`) from the rows the engine holds, likewise.
        if let [rel] = scc.iter().collect::<Vec<_>>().as_slice()
            && let Some(ArchivedNativeSpec::Rank { query, counted, group, op }) = f0.native(rel).map(|n| &n.spec)
            && let Some(n) = run.native_rank(rel, f0.rel_name(*query), f0.rel_name(*counted), *group as usize, f0.text(*op))?
        {
            super::profile::stratum(&skey, "count below a bound over held relations (decision 0361 step 7)", String::new);
            super::profile::note(format!("{rel} rows"), || n.to_string());
            outcome.native.insert(key);
            continue;
        }
        // `match_all` over the input the engine holds, likewise.
        if let [rel] = scc.iter().collect::<Vec<_>>().as_slice()
            && let Some(ArchivedNativeSpec::MatchAll { input, pattern, groups, positions }) = f0.native(rel).map(|n| &n.spec)
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
            && let Some(ArchivedNativeSpec::DfsFirst { edge, target, roots }) = f0.native(rel).map(|n| &n.spec)
            && let Some(n) = run.native_dfs_first(rel, f0.rel_name(*edge), f0.rel_name(*target), roots.map(|r| f0.rel_name(r)))?
        {
            super::profile::stratum(&skey, "native fact operator over held relations", String::new);
            super::profile::note(format!("{rel} rows"), || n.to_string());
            outcome.native.insert(key);
            continue;
        }
        if native.contains("program") && !waved[at] {
            let level: Vector<usize> = (at..sccs.len()).filter(|&j| level_of[j] == level_of[at] && !waved[j]).collect();
            let wants: BTreeSet<(String, Vector<usize>)> = level.iter().flat_map(|&j| wants_of[j].iter().cloned()).collect();
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
            let native_ones: Vector<usize> = if !batched { rerail_blocks::vector::Vector::new() } else { level.iter().copied().filter(|&j| !sccs[j].iter().all(|r| run.views.contains(r))).collect() };
            // Chained (decision 0100 §11): the next level's seed joins whose relations are already held go in this
            // batch too, so their launch and its latency run under this level's strata, not before the next's.
            // `seed_specs` keeps only those whose every atom is a relation run natively so far.
            let later: Vector<usize> = if !batched { rerail_blocks::vector::Vector::new() } else { (at..sccs.len()).map(|j| level_of[j]).filter(|&l| l > level_of[at]).min().map_or_else(rerail_blocks::vector::Vector::new, |next| {
                (at..sccs.len()).filter(|&j| level_of[j] == next && !waved[j] && !sccs[j].iter().all(|r| run.views.contains(r))).collect()
            }) };
            // Decision 0123: the statistics the level's plans will read (and the next level's, chained, where its joins
            // are batched), requested by each line that will be priced and made in one batch, before any of them is
            // planned. Made whether or not the level's seed joins are batched: its strata plan with them either way.
            if run.model.has_deferred() {
                let mut wanted = BTreeSet::new();
                let strata: Vec<(&super::front::Comp, Vec<RuleKey>)> = level.iter().chain(&later).map(|&j| (&sccs[j], run.runnable(&sccs[j], &by_stratum[j]))).collect();
                let vs = Arc::clone(&run.versions);
                run.model.wants_strata(&vs, strata, &mut wanted)?;
                run.model.make_statistics(f0, &wanted)?;
            }
            let mut specs: Vec<PlanJoin> = native_ones.iter().flat_map(|&j| run.seed_specs(&sccs[j], &by_stratum[j])).collect();
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
                    let rules_of = |j: usize| -> Vec<RuleKey> { sccs[j].iter().flat_map(|rel| rules_by.get(rel).into_iter().flatten().copied()).collect() };
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
                    b.retire_at(at, &mut run, stream, &ambient, &mut |b, run| {
                        // What the jobs that ended while the loop waits finished reading goes now (decision 0592).
                        if let Some(rl) = reads_left.as_mut() {
                            let gone = rl.finished(b.take_finished());
                            release_unread(db, run, gone, &pinned, &held, &input_set, &last, at, &mut early);
                        }
                        b.issue(at, run, &sccs, &rules_of, &wants_of, &mut waved, &mut load_early, stream, &ambient)
                    })?
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
                issue::Got::Not | issue::Got::Here => run.native_stratum(&super::front::CompAt::of(&sccs, at), &rules)?,
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
        let read: Vec<String> = rules.iter().flat_map(|&r| reads_of(&run.versions, r).map(str::to_owned).collect::<Vec<_>>()).chain(scc.iter().map(str::to_owned)).collect();
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
                run.model.ndv.insert((*rel, c), if d == 0 { 1.0 } else { d as f64 });
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
            if (native.contains("fixpoint") && run.native_fixpoint(scc,&rules)?) || (native.contains("setfix") && run.native_set_fixpoint(scc,&rules)?) {ran_native=true;outcome.native.insert(key.clone());}
        }
        if !ran_native {
            let n=sql_kernel::component(&mut run,scc,&planned,recursive)?;
            super::profile::stratum(&skey,"SQL wave kernel",||format!("{why}; {n} waves"));
            let class = run.why_class.take().unwrap_or_else(|| if native.contains("program") { "no reason given".into() } else { "the native engine is off".into() });
            // The device's hazards too: a stratum the native engine declines is never placed, so its shape is read here.
            let lacking = if super::profile::on() {
                let mut shape = placement::Shape::default();
                for ((_, order), b) in planned.iter().zip(planned_bodies) {
                    shape.add(Order::new(b, order), 0.0, recursive);
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
                run.model.ndv.insert((r, c), if d == 0 { 1.0 } else { d as f64 });
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
        let mut all: BTreeSet<String> = sccs.iter().flatten().map(str::to_owned).collect();
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
            let of: HashMap<&str, usize> = sccs.iter().enumerate().flat_map(|(j, scc)| scc.iter().map(move |r| (r, j))).collect();
            let producers: Vec<Vector<usize>> = sccs.iter().enumerate().map(|(j, scc)| {
                let from: BTreeSet<usize> = scc.iter().flat_map(|r| deps.get(r).into_iter().flatten()).filter_map(|(d, _)| of.get(d.as_str()).copied()).filter(|&p| p != j).collect();
                from.into_iter().collect()
            }).collect();
            let names: Vec<String> = sccs.iter().map(|s| s.iter().next().map(str::to_owned).unwrap_or_default()).collect();
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
        if reads_left.is_some() {
            super::profile::decision("relations released when their last reader finished (decision 0592)", || {
                format!("{} released so, {} of them before the loop passed their last reader, {} strata ahead in all", early.released, early.ahead, early.strata_ahead)
            });
        }
        if std::env::var_os("RERAIL_STRATA_COUNTS").is_some() {
            let r = std::sync::atomic::Ordering::Relaxed;
            eprintln!(
                "RERAIL_STRATA_COUNTS {:?} answers {} in {:.3} s; suspended on requests {} times, {:.3} s in all",
                b.counts,
                run_counts().answer_calls.load(r),
                run_counts().answer_wait.load(r) as f64 * 1e-9,
                run_counts().ask_suspensions.load(r),
                run_counts().ask_suspended.load(r) as f64 * 1e-9
            );
        }
    }
    finish_wave(&mut run, &mut waiting, &mut outcome, &mut carry, &sccs, &last, &mut asked)?;
    super::lanes::prof::level(None);
    if maintaining.is_some() {
        outcome.walked = (reached, quiet_slots, sccs.len());
    }
    if let Some(ks) = strata_kept {
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
    if let Some(m) = maintaining {
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
/// The climb's ends (decision 0561): a stratum whose rules each read a keyed walk's rows once, positively, and
/// otherwise only relations made before the walk (`look` and `miss` over `walk`) joins the walk's stratum, where the
/// walk's run answers it at each row as the climb makes it (`closure::keyed`), so the walk's rows are not read again
/// by a join of their own. `RERAIL_CLIMB_ENDS=off` keeps the strata apart.
/// `None`: the strata as they are.
fn climb_ends(p: &body::Placed, sccs: &super::front::Components, deps: &super::front::Edges) -> Option<super::front::Components> {
    if std::env::var("RERAIL_CLIMB_ENDS").is_ok_and(|v| v == "off") {
        return None;
    }
    let mut rules: HashMap<&str, Vec<usize>> = HashMap::new();
    for k in 0..p.len() {
        rules.entry(p.v.rel_name(p.rule(k).head().rel())).or_default().push(k);
    }
    let at: HashMap<&str, usize> = sccs.iter().enumerate().flat_map(|(j, scc)| scc.iter().map(move |r| (r, j))).collect();
    let only = |scc: &super::front::Comp| (scc.len() == 1).then(|| scc.first().expect("one").to_owned());
    let mut into: Vec<Option<usize>> = vec![None; sccs.len()];
    for j in 0..sccs.len() {
        let Some(w) = only(&sccs[j]) else { continue };
        if into[j].is_some() || !rules.get(w.as_str()).is_some_and(|ks| keyed_walk(p, &w, ks)) {
            continue;
        }
        for i in j + 1..sccs.len() {
            let Some(t) = only(&sccs[i]) else { continue };
            let Some(d) = deps.get(&t) else { continue };
            let reads = d.contains(&(w.clone(), false)) && !d.contains(&(w.clone(), true)) && !d.iter().any(|(r, _)| *r == t);
            let before = d.iter().all(|(r, _)| *r == w || at.get(r.as_str()).is_none_or(|&s| s < j));
            if into[i].is_none() && reads && before && rules.get(t.as_str()).is_some_and(|ks| ks.iter().all(|&k| reads_once(p, k, &w) && of_the_row(p, k, &w))) {
                into[i] = Some(j);
            }
        }
    }
    if into.iter().all(Option::is_none) {
        return None;
    }
    // Each stratum kept, with the ends moved into it; `Components::of` sorts each one's members.
    let mut out: Vec<Vec<&str>> = sccs.iter().map(|c| c.iter().collect()).collect();
    for (i, to) in into.iter().enumerate() {
        if let Some(j) = *to {
            let moved: Vec<&str> = sccs[i].iter().collect();
            out[j].extend(moved);
        }
    }
    Some(super::front::Components::of_names(out.into_iter().enumerate().filter(|(i, _)| into[*i].is_none()).map(|(_, s)| s)))
}

/// Whether rule `k` reads relation `w` once, as a positive atom of its own (not an aggregate's).
fn reads_once(p: &body::Placed, k: usize, w: &str) -> bool {
    let r = p.rule(k);
    let mut n = 0;
    for l in r.body() {
        match l.view() {
            Lit::Atom(a) if p.v.rel_name(a.rel()) == w => {
                if a.neg() {
                    return false;
                }
                n += 1;
            }
            Lit::Agg { atom, .. } if p.v.rel_name(atom.rel()) == w => return false,
            _ => {}
        }
    }
    n == 1
}

/// Whether rule `k` binds nothing past its row of `w` (read once, `reads_once`): every variable of its other literals
/// is one of the row's. Its rest is then tests of the row (`bound_in(s, x)`, `kind(m, "module")`), which the climb asks
/// once a row (decision 0561, 3). A rest that joins out from the row (`annotated_on_chain`'s annotations of the row's
/// scope, a scope's nodes a row) is planned from its own side, a stratum of its own: driven by every row of the climb,
/// it scanned a scope's nodes for each (43 s of corpus0's normal packs).
fn of_the_row(p: &body::Placed, k: usize, w: &str) -> bool {
    let r = p.rule(k);
    let Some(row) = r.body().find_map(|l| match l.view() {
        Lit::Atom(a) if p.v.rel_name(a.rel()) == w => Some(body::lit_vars(l)),
        _ => None,
    }) else {
        return false;
    };
    r.body().filter(|l| !matches!(l.view(), Lit::Atom(a) if p.v.rel_name(a.rel()) == w)).all(|l| body::lit_vars(l).is_subset(&row))
}

/// Whether relation `w`'s rules (`ks`) are a keyed walk (decision 0570) as written: each recursive rule reads `w`
/// once and makes the row it read with one column replaced, and at least one reads another of the row's columns
/// elsewhere in its body (`!bound_in(s, x)`). The engine's `closure::recognize` decides the form; this is its shape in
/// the rules, so that a plain closure's readers stay apart.
fn keyed_walk(p: &body::Placed, w: &str, ks: &[usize]) -> bool {
    let vars = |t: TermRef<'_>, out: &mut Vec<VarSlot>| {
        if let Some(s) = t.var() {
            out.push(s);
        }
    };
    let mut keyed = false;
    let mut recursive = false;
    for &k in ks {
        let r = p.rule(k);
        let own: Vec<AtomRef<'_>> = r.body().filter_map(|l| match l.view() { Lit::Atom(a) if p.v.rel_name(a.rel()) == w => Some(a), _ => None }).collect();
        if own.is_empty() {
            continue;
        }
        recursive = true;
        let [a] = own[..] else { return false };
        if a.neg() {
            return false;
        }
        let (head, row): (Vec<_>, Vec<_>) = (r.head().args().map(TermRef::var).collect(), a.args().map(TermRef::var).collect());
        if head.len() != row.len() || head.iter().chain(&row).any(Option::is_none) {
            return false;
        }
        let moved: Vec<usize> = (0..row.len()).filter(|&c| head[c] != row[c]).collect();
        let [c] = moved[..] else { return false };
        let others: Vec<VarSlot> = (0..row.len()).filter(|&o| o != c).filter_map(|o| row[o]).collect();
        let mut used = Vec::new();
        for l in r.body() {
            match l.view() {
                Lit::Atom(b) if b.node() != a.node() => b.args().for_each(|t| vars(t, &mut used)),
                Lit::Cmp { l, r, .. } => {
                    vars(l, &mut used);
                    vars(r, &mut used);
                }
                _ => {}
            }
        }
        keyed |= used.iter().any(|s| others.contains(s));
    }
    recursive && keyed
}

/// The strata with each request that produces an input first (decision 0431 §8): its answers grow the run's bound
/// sources (a tape value's rows are a part of each, `syntax::values::Layered`), so a stratum that scans a source
/// sees them only after the request has answered. A shown guest's nodes (one in the run's own language, its copies
/// folded, decision 0058) are a plain view's rows, reached by no guest relation. The request's cone (what it reads,
/// transitively) is closed under its dependencies, so the cone first and then the rest, each in its order, is a
/// dependency order still; each stratum outside the cone is given an edge to the request, which the run's scheduling
/// keeps (the edges returned, `None` where no request produces an input).
fn requests_first(sccs: &super::front::Components, deps: &super::front::Edges, f: &Fields<'_>) -> (Option<super::front::Components>, Option<super::front::Edges>) {
    let requests: BTreeSet<String> = f.answers().iter().filter(|&&(_, by)| f.is_input(f.v.rel_name(by))).map(|&(q, _)| f.v.rel_name(q).to_owned()).collect();
    if requests.is_empty() {
        return (None, None);
    }
    let mut cone: BTreeSet<String> = requests.clone();
    let mut todo: Vec<String> = requests.iter().cloned().collect();
    while let Some(r) = todo.pop() {
        for (d, _) in deps.get(&r).into_iter().flatten() {
            if cone.insert(d.clone()) {
                todo.push(d.clone());
            }
        }
    }
    let (inside, outside): (Vec<usize>, Vec<usize>) = (0..sccs.len()).partition(|&j| sccs[j].iter().any(|r| cone.contains(r)));
    let mut edges = deps.clone();
    for r in outside.iter().flat_map(|&j| sccs[j].iter()) {
        let e = edges.entry(r.to_owned()).or_default();
        e.extend(requests.iter().map(|q| (q.clone(), false)));
    }
    (Some(sccs.reordered(inside.into_iter().chain(outside))), Some(edges))
}

/// Each stratum's level, and the order the strata run in: by level when `sort`, the dependency order's otherwise
/// (`None`: the order kept).
fn by_level(sccs: &super::front::Components, deps: &super::front::Edges, sort: bool) -> (Option<Vector<usize>>, Vector<usize>) {
    let of: HashMap<&str, usize> = sccs.iter().enumerate().flat_map(|(j, scc)| scc.iter().map(move |r| (r, j))).collect();
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
    let mut order: Vector<usize> = (0..sccs.len()).collect();
    if !sort {
        return (None, order.iter().map(|&j| level[j]).collect());
    }
    order.sort_by_key(|&j| level[j]);
    let levels = order.iter().map(|&j| level[j]).collect();
    (Some(order), levels)
}

/// Runs the device wave the waiting strata wait on (`lanes::wave::flush`: their launches as one batch), then
/// resumes each in order; one that waits again (a second form's launches) waits on the next wave.
fn finish_wave<'a>(run: &mut Run<'a, '_>, waiting: &mut Vec<(usize, String, Box<Driving>)>, outcome: &mut Outcome, carry: &mut Vec<String>, sccs: &crate::code_rule_dsl::logic::front::Components, last: &HashMap<String, usize>, asked: &mut Vec<Asked>) -> Result<()> {
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
                    carry.extend(sccs[j].iter().filter(|r| !last.contains_key(*r)).map(str::to_owned));
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
    tickets: Vector<u64>,
    since: std::time::Instant,
}

/// Takes the answers that have come (waiting up to `wait` for one when none has), and resumes each stratum whose
/// requests are all answered: it runs on, finishes, waits on its wave, or asks again.
#[allow(clippy::too_many_arguments)]
fn take_answers<'a>(
    run: &mut Run<'a, '_>,
    asked: &mut Vec<Asked>,
    arrived: &mut HashMap<u64, Vec<(String, Vec<Vector<i64>>)>>,
    waiting: &mut Vec<(usize, String, Box<Driving>)>,
    outcome: &mut Outcome,
    carry: &mut Vec<String>,
    sccs: &crate::code_rule_dsl::logic::front::Components,
    last: &HashMap<String, usize>,
    wait: std::time::Duration,
) -> Result<()> {
    let Some(asking) = run.asking.as_deref_mut() else {
        return Err(Error::Plan("requests posted, and no one to answer them".into()));
    };
    let t0 = std::time::Instant::now();
    let came = asking.arrived(wait)?;
    if !wait.is_zero() {
        run_counts().answer_wait.fetch_add(t0.elapsed().as_nanos() as u64, std::sync::atomic::Ordering::Relaxed);
    }
    arrived.extend(came);
    let mut k = 0;
    while k < asked.len() {
        if !asked[k].tickets.iter().all(|t| arrived.contains_key(t)) {
            k += 1;
            continue;
        }
        let a = asked.remove(k);
        run_counts().ask_suspensions.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        run_counts().ask_suspended.fetch_add(a.since.elapsed().as_nanos() as u64, std::sync::atomic::Ordering::Relaxed);
        let given: Vec<(String, Vec<Vector<i64>>)> = a.tickets.iter().flat_map(|t| arrived.remove(t).unwrap_or_default()).collect();
        let _stratum = super::profile::span(|| format!("stratum {} (after its answers)", if a.key.len() > 80 { format!("{}…", &a.key[..80]) } else { a.key.clone() }));
        add_answers(&run.db, &mut run.answered, None, &a.d.name, &a.d.scc, given)?;
        match run.native_resume(a.d)? {
            Native::Ran => {
                outcome.native.insert(a.key);
                carry.extend(sccs[a.at].iter().filter(|r| !last.contains_key(*r)).map(str::to_owned));
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
pub fn first_reads(f: Fields<'_>, vs: &body::Versions, sccs: &crate::code_rule_dsl::logic::front::Components, by: &[Vec<RuleKey>], views: &HashSet<String>) -> HashMap<String, usize> {
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
fn level_trace() -> Option<String> {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var("RERAIL_LEVEL_TRACE").ok())
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
pub fn keys_by_stratum(p: &body::Placed, sccs: &crate::code_rule_dsl::logic::front::Components) -> Vec<Vec<RuleKey>> {
    let of: HashMap<&str, usize> = sccs.iter().enumerate().flat_map(|(j, scc)| scc.iter().map(move |r| (r, j))).collect();
    let mut by: Vec<Vec<RuleKey>> = vec![Vec::new(); sccs.len()];
    for k in 0..p.len() {
        if let Some(&j) = of.get(p.v.rel_name(p.rule(k).head().rel())) {
            by[j].push(RuleKey::program(k));
        }
    }
    by
}

/// Each relation's readers that have not finished (decision 0592): a relation is released when the last of its readers
/// to finish does, not when the in-order loop passes its last reader by position. Strata are issued ahead and their
/// jobs end out of order (decision 0114 §1), so a slow stratum at the loop's position held every relation made after
/// it, though their readers had all run.
pub struct ReadsLeft {
    /// Per stratum, the relations it is counted a reader of.
    reads: Vec<Vec<String>>,
    left: HashMap<String, usize>,
    done: Vec<bool>,
    /// The strata below it the loop passed.
    upto: usize,
    /// Passed, but suspended on their requests: they read again when they resume.
    deferred: Vec<usize>,
}

impl ReadsLeft {
    fn new(f: Fields<'_>, vs: &body::Versions, sccs: &super::front::Components, by: &[Vec<RuleKey>], views: &HashSet<String>) -> Self {
        let reads: Vec<BTreeSet<String>> = sccs.iter().enumerate().map(|(j, scc)| stratum_reads(f, vs, scc, &by[j])).collect();
        // A stratum of views is read where its readers read it (`last_reads`): what it reads stays to the in-order loop.
        let lazy: HashSet<&String> = sccs.iter().zip(&reads).filter(|(scc, _)| scc.iter().all(|r| views.contains(r))).flat_map(|(_, rs)| rs).collect();
        Self::of(reads.iter().map(|rs| rs.iter().filter(|r| !lazy.contains(r)).cloned().collect()).collect())
    }

    /// Over each stratum's counted reads.
    pub fn of(reads: Vec<Vec<String>>) -> Self {
        let mut left: HashMap<String, usize> = HashMap::new();
        for r in reads.iter().flatten() {
            *left.entry(r.clone()).or_default() += 1;
        }
        ReadsLeft { done: vec![false; reads.len()], reads, left, upto: 0, deferred: Vec::new() }
    }

    /// Strata `js` read all they read: the relations no reader is left for.
    pub fn finished(&mut self, js: impl IntoIterator<Item = usize>) -> Vec<String> {
        let mut out = Vec::new();
        for j in js {
            if std::mem::replace(&mut self.done[j], true) {
                continue;
            }
            for r in &self.reads[j] {
                let n = self.left.get_mut(r).expect("a counted read");
                *n -= 1;
                if *n == 0 {
                    out.push(r.clone());
                }
            }
        }
        out
    }

    /// The loop is at `at`: each stratum before it finished, but one still asking (`asking`), which finishes once it is
    /// not.
    pub fn passed(&mut self, at: usize, asking: &dyn Fn(usize) -> bool) -> Vec<String> {
        let mut now: Vec<usize> = Vec::new();
        for j in self.upto..at {
            if asking(j) { self.deferred.push(j) } else { now.push(j) }
        }
        self.upto = self.upto.max(at);
        let (still, back): (Vec<usize>, Vec<usize>) = std::mem::take(&mut self.deferred).into_iter().partition(|&j| asking(j));
        self.deferred = still;
        now.extend(back);
        self.finished(now)
    }
}

/// How many relations went when their readers finished, and how many of them before the loop at `at` passed their
/// last reader (`last`), by how many strata in all.
#[derive(Default)]
struct Early {
    released: usize,
    ahead: usize,
    strata_ahead: usize,
}

/// Whether relations go when their last reader to finish does (decision 0592): on, unless `RERAIL_RELEASE_FINISHED=off`
/// (for comparison: the loop's in-order release alone).
fn release_finished() -> bool {
    std::env::var("RERAIL_RELEASE_FINISHED").as_deref() != Ok("off")
}

/// Releases `gone`, the relations whose readers all finished (`ReadsLeft`), where the in-order loop would at their last
/// reader: a native relation the run made, not one that stays (`pinned`), an input, a view, or one a stratum still
/// asking reads (`held`). Each is then not native, so the loop's own release passes it by.
#[allow(clippy::too_many_arguments)]
fn release_unread(db: &Connection, run: &mut Run<'_, '_>, gone: Vec<String>, pinned: &BTreeSet<String>, held: &HashSet<String>, inputs: &BTreeSet<&str>, last: &HashMap<String, usize>, at: usize, early: &mut Early) {
    let done: Vec<String> = gone
        .into_iter()
        .filter(|r| run.native_rels.contains(r) && !pinned.contains(r) && !held.contains(r) && !inputs.contains(r.as_str()) && !run.views.contains(r))
        .collect();
    if done.is_empty() {
        return;
    }
    super::native::component::release(db, &done);
    run.spike_release(&done);
    for r in &done {
        run.native_rels.remove(r);
        run.demands.forget(r);
        if let Some(&l) = last.get(r)
            && l >= at
        {
            early.ahead += 1;
            early.strata_ahead += l + 1 - at;
        }
    }
    early.released += done.len();
}

/// Per relation, the last stratum (an index into `sccs`) whose rules read it (`first_reads`' reading).
pub fn last_reads(f: Fields<'_>, vs: &body::Versions, sccs: &crate::code_rule_dsl::logic::front::Components, by: &[Vec<RuleKey>], views: &HashSet<String>) -> HashMap<String, usize> {
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
) -> Result<(BTreeMap<String, std::sync::Arc<output::Rows>>, BTreeMap<String, Vector<usize>>)> {
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

/// Outputs `rels` of `lowered` that are its inputs, as a run with nothing to derive gives them (t33 K4): read where
/// the run binds them in `db`, decoded as `decoded_outputs` decodes an input's rows.
#[allow(clippy::type_complexity)]
pub fn decode_bound(
    db: &Connection,
    lowered: &lower::Lowered,
    rels: &[&str],
) -> Result<(BTreeMap<String, std::sync::Arc<output::Rows>>, BTreeMap<String, Vector<usize>>)> {
    let placed = lowered.placed()?;
    let f = placed.fields()?;
    decoded_outputs(rels, &|r| f.is_input(r), &|r| f.columns(r).map(|c| c.into_iter().map(|(_, t)| t.to_owned()).collect()), db, Some(lowered.lowering()), "", true, &BTreeSet::new(), &BTreeSet::new())
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
) -> Result<(BTreeMap<String, std::sync::Arc<output::Rows>>, BTreeMap<String, Vector<usize>>)> {
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

// cratify: `search_tests` is rerail-logic's (it mounts it: it reads the language, `rr::parse_version`, or the corpus)

/// A checked line's rows left, split by its key's heavy values (`Run::heavy_parts`): the linking relation and its key
/// column; the rows of heavy keys and the rest.
struct HeavyParts {
    link: String,
    cv: usize,
    hot: Vector<u32>,
    cold: Vector<u32>,
}

/// Decision 0243: what a background search's walks read, kept by its request while the engine holds it: the body's
/// relations (shared, not copied), the rule's text constants' symbols, and for a probe's line a sample of its delta's
/// rows with their count.
pub struct WalkSnap {
    pub rels: HashMap<String, std::sync::Arc<super::native::component::InRel>>,
    pub syms: HashMap<String, i64>,
    pub delta: Option<(Vec<Vector<i64>>, f64)>,
    /// Decision 0245: each body atom's columns whose heavy values were made, by relation and column: the `HEAVY`
    /// most common, as the inline trials split by them (`Run::heavy_parts`).
    pub heavy: HashMap<(String, usize), Vector<i64>>,
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
    /// `None` where a positive atom's relation is not the engine's (the walk could not probe it), or is a view held in
    /// place (`hold_view_in_place`): a view's rows are its source's, so holding it holds the source whole, and a
    /// batch's view a chunk's every relation, in its block of sources, until the search runs. Decision 0243 bounds
    /// what a request holds by its line's relations; the line is priced instead. A negated view is not held (a walk
    /// takes a negated relation it does not hold as passing).
    /// Read by the scope alone (`Db`): a stratum off the run's thread asks too, and reads no SQL (decision 0113).
    pub fn take(db: crate::code_rule_dsl::facts::vtab::Db<'_>, name: &str, line: usize, b: &Body<'_>, first: Option<usize>, heavy_of: &dyn Fn(&str, usize) -> Option<Vector<i64>>) -> Option<WalkSnap> {
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
            match engine::held(db, rel).filter(|held| held.view.is_none()) {
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
    fn checks(b: &Body<'_>, o: &[Step]) -> Vector<usize> {
        let Some(lead) = o.first().map(|s| s.at) else { return rerail_blocks::vector::Vector::new() };
        let Lit::Atom(_) = b.view(lead) else { return rerail_blocks::vector::Vector::new() };
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
    pub fn starts(&self, b: &Body<'_>, o: &[Step], first: Option<usize>) -> Option<Starts> {
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
        let mut survivors: Vec<Vector<i64>> = Vec::new();
        // Whether a lead row passes the checks, counting each check's rows reached and passed where `counts`.
        let passes = |row: &[i64], mut counts: Option<(&mut [u64], &mut [u64])>| -> bool {
            for (i, &ci) in checks.iter().enumerate() {
                if let Some((r, _)) = counts.as_mut() { r[i] += 1 }
                let Lit::Atom(c) = b.view(ci) else { continue };
                let probe: Option<Vec<(usize, i64)>> = b.args(ci).iter().enumerate().filter_map(|(k, &t)| match value(t, row) { Some(Some(v)) => Some(Some((k, v))), Some(None) => None, None => Some(None) }).collect();
                let found = probe.is_some_and(|probe| {
                    let (pcols, pkey): (Vector<usize>, Vector<i64>) = probe.into_iter().unzip();
                    let mut found = false;
                    if let Some(rel) = self.rels.get(b.rel(c)) { rel.probe_each(&pcols, &pkey, &mut |_| found = true) }
                    found
                });
                if found == c.neg() { return false }
                if let Some((_, p)) = counts.as_mut() { p[i] += 1 }
            }
            true
        };
        let mut test = |row: Vector<i64>, survivors: &mut Vec<Vector<i64>>| {
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
                let n = rel.probe_sample(&cols, &key, batch, &mut draw, &mut |row| got.push((0..width).map(row).collect::<Vector<i64>>()));
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
                let (mut found, mut over): (Vec<Vector<i64>>, bool) = (Vec::new(), false);
                crel.probe_each(&ccols, &ckey, &mut |row| {
                    if over { return }
                    let (mut lcols, mut lkey) = (cols.clone(), key.clone());
                    for &(i, lc) in &links { if !lcols.contains(&lc) { lcols.push(lc); lkey.push(row(i)); } }
                    lrel.probe_each(&lcols, &lkey, &mut |l| {
                        let r: Vector<i64> = (0..width).map(l).collect();
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
            let (hot_u, cold_u): (Vec<Vector<i64>>, Vec<Vector<i64>>) = std::mem::take(&mut st.rows).into_iter().partition(|row| hot_row(row));
            let cold_share = cold_u.len() as f64 / (hot_u.len() + cold_u.len()).max(1) as f64;
            // The hot lead rows drawn by key: each heavy value's link rows, sampled in proportion to its rows, then
            // the lead's rows they reach, kept where they pass the checks. Each heavy value's survivors are scaled
            // by its rows over those drawn. A lead row two heavy values reach is counted twice.
            let batch = 16 * joins::walk_rows();
            let ordered: Vector<i64> = self.heavy[&(link.clone(), cv)].clone();
            let sizes: Vector<f64> = ordered.iter().map(|&h| { let mut n = 0.0; lrel.probe_each(&[cv], &[h], &mut |_| n += 1.0); n }).collect();
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
                    let mut links: Vector<i64> = rerail_blocks::vector::Vector::new();
                    let mut leads: Vec<Vector<i64>> = Vec::new();
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
    pub fn walk(&self, b: &Body<'_>, first: Option<usize>, ran: &[Step], mut priced: Vec<(Vec<Step>, f64)>, over: &dyn Fn() -> bool) -> joins::Searched {
        priced.sort_by(|a, b| a.1.total_cmp(&b.1));
        priced.truncate(WALK_ORDERS);
        let ran_price = priced.iter().position(|(o, _)| same_order(o, ran)).map_or(f64::NAN, |i| priced.remove(i).1);
        priced.insert(0, (ran.to_vec(), ran_price));
        let mut walkers: HashMap<usize, Walker<'_, '_>> = HashMap::new();
        // The starts by the lead and its checks: orders that share them share their rows.
        let mut starts: HashMap<Vector<usize>, Option<Starts>> = HashMap::new();
        let mut walked = Vec::new();
        let mut ran_w = None;
        for (i, (o, price)) in priced.into_iter().enumerate() {
            // A measurement of the walks' spread (`walk_repeat`) walks every order.
            if i > 0 && over() && walk_repeat() == 0 { break }
            let Some(lead) = o.first().map(|s| s.at) else { continue };
            let prefix: Vector<usize> = o.iter().take(1 + Self::checks(b, &o).len()).map(|s| s.at).collect();
            let st = starts.entry(prefix).or_insert_with(|| self.starts(b, &o, first)).as_ref();
            let walker = walkers.entry(lead).or_insert_with(|| Walker::new(b, lead, joins::walk_width(), &|rel| self.rels.get(rel).cloned(), &|x| self.syms.get(x).copied()));
            // From the rows past the checks: the walk tests them again, each passing, and goes on. Its work past the
            // checks is the share passing them times its work less theirs. A walk whose sample ran out before its
            // last step measured nothing past it, and is not ranked.
            // Split by a heavy key (decision 0245), the rest's rows are their share of those past the checks, and the
            // hot rows are as drawn by key.
            let est = st.and_then(|st| {
                let k = st.checks as f64;
                let part = |rows: &[Vector<i64>]| -> Option<f64> {
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
                        let part = |rows: &[Vector<i64>]| -> Option<f64> { if rows.is_empty() { return Some(0.0) } let per = w2.work(&o, rows); (!w2.emptied()).then(|| (per - k).max(0.0)) };
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
pub struct Starts {
    rows: Vec<Vector<i64>>,
    lead_rows: f64,
    checks: usize,
    checks_work: f64,
    share: f64,
    /// Decision 0245, where split by a heavy key: `rows` are the rest's, `cold` their share of the rows past the
    /// checks, and `hot` the rows drawn by the key's heavy values with their estimated rows.
    cold: f64,
    pub hot: Option<(Vec<Vector<i64>>, f64)>,
}

/// `n(xs) :- x(xs), !g(xs)`: the rule's head, `x` and `g`, where `xs` are distinct variables in one order, `x` and `g`
/// relations the engine holds (`held_rel`) and `g` has at most an eighth of `x`'s rows (`rows`; `Rel::based`'s bound
/// on the rows it marks dead). `None` for any other rule.
fn based_rule(vs: &body::Versions, r: RuleKey, held_rel: &dyn Fn(&str) -> bool, rows: &dyn Fn(&str) -> Option<usize>) -> Option<(String, String, String, RuleKey)> {
    let (v, rule) = vs.rule_at(r);
    let head: Vec<TermRef<'_>> = rule.head().args().collect();
    let mut vars = HashSet::new();
    if head.is_empty() || !head.iter().all(|t| matches!(t, TermRef::Var(x) if vars.insert(*x))) {
        return None;
    }
    let mut body = rule.body();
    let (Some(p), Some(q), None) = (body.next(), body.next(), body.next()) else { return None };
    let (Lit::Atom(p), Lit::Atom(q)) = (p.view(), q.view()) else { return None };
    let (x, g) = match (p.neg(), q.neg()) { (false, true) => (p, q), (true, false) => (q, p), _ => return None };
    let vars_of = |a: AtomRef<'_>| a.args().map(|t| t.var()).collect::<Vec<_>>();
    let want: Vec<_> = head.iter().map(|t| t.var()).collect();
    if vars_of(x) != want || vars_of(g) != want {
        return None;
    }
    let (xn, gn) = (v.rel_name(x.rel()), v.rel_name(g.rel()));
    if xn == gn || !held_rel(xn) || !held_rel(gn) {
        return None;
    }
    let (Some(xr), Some(gr)) = (rows(xn), rows(gn)) else { return None };
    (xr > 0 && gr * 8 <= xr).then(|| (v.rel_name(rule.head().rel()).to_owned(), xn.to_owned(), gn.to_owned(), r))
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
///
/// Each step adds the bindings it reads. Exact where the sample holds every binding. A binding is the rule's
/// variables by their slots.
pub struct Walker<'a, 'v> {
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
    pub fn new(b: &'a Body<'v>, lead: usize, width: usize, rel: &dyn Fn(&str) -> Option<std::sync::Arc<super::native::component::InRel>>, sym: &dyn Fn(&str) -> Option<i64>) -> Walker<'a, 'v> {
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
    pub fn work(&self, o: &[Step], starts: &[Vector<i64>]) -> f64 {
        self.work_at(o, starts, self.width)
    }

    /// `work` carrying `width` bindings a step.
    pub(super) fn work_at(&self, o: &[Step], starts: &[Vector<i64>], width: usize) -> f64 {
        let b = self.b;
        let Lit::Atom(_) = b.view(self.lead) else { return 1.0 };
        if starts.is_empty() { return 0.0 }
        let n = self.vars;
        let mut bound = vec![false; n];
        // Decision 0247: each binding carries the share of a lead row's bindings it stands for; a step's work is
        // their sum.
        let w0 = 1.0 / starts.len() as f64;
        let mut sample: Vec<(f64, Vector<i64>)> = Vec::with_capacity(starts.len());
        for row in starts {
            let mut bind = rerail_blocks::vector![0i64; n];
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
            let mut kcols: Vector<usize> = consts.iter().map(|c| c.0).collect();
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
                    let sizes: Vector<f64> = sample.iter().map(|(w, bind)| {
                        let key: Vector<i64> = kfrom.iter().map(|k| match *k { Ok(v) => v, Err(i) => bind[i] }).collect();
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
            let mut next: Vec<(f64, Vector<i64>)> = Vec::new();
            let t_step = super::native::component::thread_cpu();
            let n_step = sample.len() as u64;
            let mut r_step = 0u64;
            for (wb, bind) in &sample {
                let key: Vector<i64> = kfrom.iter().map(|k| match *k { Ok(v) => v, Err(i) => bind[i] }).collect();
                let (mut found, mut read) = (false, 0u64);
                let mut mine: Vec<Vector<i64>> = Vec::new();
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
    fn priority(&self, mut items: Vec<(f64, Vector<i64>)>, k: usize) -> Vec<(f64, Vector<i64>)> {
        if items.len() <= k || k == 0 { return items }
        let all: f64 = items.iter().map(|x| x.0).sum();
        let mut keyed: Vec<(f64, usize)> = items.iter().enumerate().map(|(i, x)| {
            let u = ((self.draw() >> 11) as f64 + 0.5) / (1u64 << 53) as f64;
            (x.0 / u, i)
        }).collect();
        keyed.select_nth_unstable_by(k, |a, b| b.0.total_cmp(&a.0));
        let tau = keyed[k].0;
        let keep: Vector<usize> = keyed[..k].iter().map(|x| x.1).collect();
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
    fn priority_sized(&self, mut items: Vec<(f64, Vector<i64>)>, sizes: &[f64], k: usize) -> Vec<(f64, Vector<i64>)> {
        let live: Vector<usize> = (0..items.len()).filter(|&i| sizes[i] > 0.0).collect();
        if live.len() <= k { return live.into_iter().map(|i| std::mem::take(&mut items[i])).collect() }
        let all: f64 = live.iter().map(|&i| sizes[i]).sum();
        let mut keyed: Vec<(f64, usize)> = live.iter().map(|&i| {
            let u = ((self.draw() >> 11) as f64 + 0.5) / (1u64 << 53) as f64;
            (sizes[i] / u, i)
        }).collect();
        keyed.select_nth_unstable_by(k, |a, b| b.0.total_cmp(&a.0));
        let tau = keyed[k].0;
        let keep: Vector<usize> = keyed[..k].iter().map(|x| x.1).collect();
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

    /// 0090 §4: a changed read's rows gone and added that agree on the columns its strata read are no change to it;
    /// the rest of each version is its delta, as read where they lie, in their order. With none agreeing, `None`.
    #[test]
    fn a_changed_reads_rows_that_agree_on_the_read_columns_are_no_change() {
        use crate::code_rule_dsl::facts::vtab::Source;
        let cols = || ["a", "b", "c"].map(String::from).to_vec();
        let rel = |rows: &[[i64; 3]]| crate::code_rule_dsl::logic::native::component::columns_source(cols(), (0..3).map(|c| Some(rows.iter().map(|r| r[c]).collect())).collect());
        let minus = rel(&[[1, 10, 5], [2, 20, 6], [3, 30, 7], [2, 20, 1]]);
        let plus = rel(&[[1, 10, 9], [2, 21, 6], [4, 40, 7], [1, 10, 8]]);
        let rows = |s: &Arc<dyn Source>| (0..s.len()).map(|r| (0..3).map(|c| s.value(r, c)).collect::<Vec<i64>>()).collect::<Vec<_>>();
        let [m, p] = unchanged_as_read(&minus, &plus, &[0, 1]).expect("a pair agrees on (a, b)");
        assert_eq!(rows(&m), vec![vec![2, 20, 6], vec![3, 30, 7], vec![2, 20, 1]], "the rows gone that no row added agrees with");
        assert_eq!(rows(&p), vec![vec![2, 21, 6], vec![4, 40, 7]], "the rows added that no row gone agrees with");
        // On the last column alone: 5 and 1 are added by none; 9 and 8 gone by none.
        let [m, p] = unchanged_as_read(&minus, &plus, &[2]).expect("6 and 7 agree");
        assert_eq!(rows(&m), vec![vec![1, 10, 5], vec![2, 20, 1]]);
        assert_eq!(rows(&p), vec![vec![1, 10, 9], vec![1, 10, 8]]);
        assert!(unchanged_as_read(&rel(&[[5, 5, 5]]), &plus, &[0, 1]).is_none(), "no pair agrees");
        assert!(unchanged_as_read(&rel(&[]), &plus, &[0, 1]).is_none(), "nothing gone");
    }

    /// Q88: a chunk plans from the run's plans as filed once the chunks `window` before it were, never from a chunk
    /// beside it: what each sees is fixed by the chunks' order, whatever finishes or is filed first. What it plans and
    /// learns is its own until it is filed.
    #[test]
    fn a_chunk_sees_the_plans_filed_before_its_window_never_a_neighbours() {
        let _plans = KeepPlans::start();
        let mut ordered = OrderedPlans::begin(2).expect("a corpus run's plans");
        let put = |key: PlanKey, w: f64| with_plans(|p| plans_put(p.unwrap(), key, (Arc::from(Vec::<Step>::new()), w, None, 1.0, None, None)));
        let get = |key: PlanKey| plans_get(&key).map(|k| k.1);
        let (v0, v1) = (ordered.next_view(), ordered.next_view());
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
        let v2 = ordered.next_view();
        assert_eq!(in_view(&v2, || (get(1), get(2))).0, (Some(10.0), None));
        // What it learns of a plan it read is its own copy's: the run's entry is unchanged until it is filed.
        let ((), m2) = in_view(&v2, || plans_learnt(&1, Learnt::Ran(5.0)));
        assert_eq!(plans_get(&1).and_then(|k| k.5), None, "unfiled");
        ordered.file(m2);
        assert_eq!(plans_get(&1).and_then(|k| k.5), Some(5.0), "filed");
    }

    /// Q314: the joins' memo follows the chunks' order as the plans do. A note chunk 0 writes is in no view of a
    /// chunk beside it, nor the run's until chunk 0 is filed; chunk 2 (a window of 2) begins with it.
    #[test]
    fn a_chunk_sees_the_joins_notes_filed_before_its_window_never_a_neighbours() {
        // The test's own joins memo (decision 0442), empty and no other test's.
        let _own = crate::code_rule_dsl::logic::profile::within(crate::code_rule_dsl::logic::profile::Shared::new());
        let _plans = KeepPlans::start();
        let mut ordered = OrderedPlans::begin(2).expect("a corpus run's plans");
        let key: PlanKey = 0x0261_0001;
        let order = [Step { at: 0, mode: None }, Step { at: 1, mode: None }];
        let noted = || joins::note(&key).is_some();
        let (v0, v1) = (ordered.next_view(), ordered.next_view());
        let ((), m0) = in_view(&v0, || joins::put_tried(key, 7, &order, 3.0));
        assert!(in_view(&v0, noted).0, "a chunk reads its own notes");
        assert!(!in_view(&v1, noted).0, "a neighbour's, finished first");
        assert!(!noted(), "the run's memo holds no chunk's until it is filed");
        ordered.file(m0);
        assert!(noted(), "chunk 0 filed");
        assert!(!in_view(&v1, noted).0, "a view stays as its chunk began");
        let ((), m1) = in_view(&v1, || ());
        ordered.file(m1);
        assert!(in_view(&ordered.next_view(), noted).0, "chunk 2 begins with chunk 0's");
    }

    /// Decision 0478 step 7: a device stage's growth (rows out per row in, which sizes its next run's space) is read as
    /// filed before its chunk's window began, never as a chunk beside it or the chunk's own strata ran, by the
    /// threads' timing; a chunk's growths are filed whatever order its threads wrote them in, each its most.
    #[test]
    fn a_stages_growth_is_read_as_filed_never_as_the_threads_ran() {
        // The test's own joins memo (decision 0442), empty and no other test's.
        let _own = crate::code_rule_dsl::logic::profile::within(crate::code_rule_dsl::logic::profile::Shared::new());
        let _plans = KeepPlans::start();
        let mut ordered = OrderedPlans::begin(2).expect("a corpus run's plans");
        let (rule, stage) = (0x0450_0007u64, 3);
        let seen = || joins::growth(rule, stage);
        let (v0, v1) = (ordered.next_view(), ordered.next_view());
        let ((), m0) = in_view(&v0, || {
            joins::note_growth(rule, stage, 2.0);
            joins::note_growth(rule, stage, 5.0);
            joins::note_growth(rule, stage, 3.0);
        });
        assert_eq!(in_view(&v0, seen).0, None, "a chunk's own strata read its growth as filed when it began");
        assert_eq!(in_view(&v1, seen).0, None, "a neighbour's");
        assert_eq!(seen(), None, "the run's holds no chunk's until it is filed");
        let ((), m1) = in_view(&v1, || {
            joins::note_growth(rule, stage, 4.0);
            joins::note_growth(rule, stage, 1.0);
        });
        ordered.file(m0);
        assert_eq!(seen(), Some(5.0), "a chunk's most");
        ordered.file(m1);
        assert_eq!(seen(), Some(4.0), "filed in the chunks' order: the later chunk's stands");
        assert_eq!(in_view(&ordered.next_view(), seen).0, Some(5.0), "chunk 2 begins with chunk 0's");
    }

    /// Q314: so do the planning choices (inline or store, demand). A program chunk 0 chose for is chosen again by a
    /// chunk beside it, not taken from chunk 0 by whichever planned first; chunk 2 takes chunk 0's.
    #[test]
    fn a_chunk_takes_the_planning_choices_filed_before_its_window_never_a_neighbours() {
        let _plans = KeepPlans::start();
        let mut ordered = OrderedPlans::begin(2).expect("a corpus run's plans");
        let key = [0x61u8; 32];
        let chosen = |rel: &str| choices::Chosen { version: None, inlined: vec![rel.to_owned()], demanded: Vec::new(), nodes: 1000.0 };
        let taken = || choices::lookup(&key, 1000.0).map(|c| c.inlined);
        let (v0, v1) = (ordered.next_view(), ordered.next_view());
        let ((), m0) = in_view(&v0, || choices::remember(key, chosen("a")));
        assert_eq!(in_view(&v0, taken).0, Some(vec!["a".to_owned()]), "a chunk takes its own");
        assert_eq!(in_view(&v1, taken).0, None, "a neighbour's, made first");
        assert_eq!(taken(), None, "the process's hold no chunk's until it is filed");
        let ((), m1) = in_view(&v1, || choices::remember(key, chosen("b")));
        ordered.file(m0);
        assert_eq!(taken(), Some(vec!["a".to_owned()]));
        ordered.file(m1);
        assert_eq!(taken(), Some(vec!["b".to_owned()]), "filed in the chunks' order: the later chunk's stands");
        assert_eq!(in_view(&ordered.next_view(), taken).0, Some(vec!["a".to_owned()]), "chunk 2 begins with chunk 0's");
    }
}

/// The climb's ends (decision 0561) are the readers the climb answers from its row alone: a reader whose rules bind
/// nothing past the walk's row (`look`'s `bound_in(s, x)`) joins the walk's stratum; one that joins out from the row
/// (`chained`'s `ann(a, x, s)`, as `annotated_on_chain` joins a scope's annotations) stays a stratum of its own,
/// planned from its own side, not driven by every row of the climb.
#[cfg(test)]
mod climb_tests {
    use super::*;

    #[test]
    fn a_reader_that_joins_out_from_the_walks_row_is_not_a_climb_end() {
        use crate::code_rule_dsl::logic::program_image::{Builder, T};
        // walk(n, x, s) :- nameref(n, x), wstart(n, s).
        // walk(n, x, p) :- walk(n, x, s), !bound_in(s, x), up(s, p).
        // look(n, x, s) :- walk(n, x, s), bound_in(s, x).
        // chained(n, x) :- walk(n, x, s), ann(a, x, s).
        let mut b = Builder::base();
        for (rel, cols) in [("nameref", &[("n", "node"), ("x", "symbol")][..]), ("wstart", &[("n", "node"), ("s", "node")]), ("bound_in", &[("s", "node"), ("x", "symbol")]), ("up", &[("s", "node"), ("p", "node")]), ("ann", &[("a", "node"), ("x", "symbol"), ("s", "node")]), ("walk", &[("n", "node"), ("x", "symbol"), ("s", "node")]), ("look", &[("n", "node"), ("x", "symbol"), ("s", "node")]), ("chained", &[("n", "node"), ("x", "symbol")])] {
            b.fields().declare(rel, cols);
        }
        let v = |x: &'static str| T::Var(x);
        b.rule().head("walk", &[v("n"), v("x"), v("s")]).atom("nameref", &[v("n"), v("x")]).atom("wstart", &[v("n"), v("s")]).finish().unwrap();
        b.rule().head("walk", &[v("n"), v("x"), v("p")]).atom("walk", &[v("n"), v("x"), v("s")]).not("bound_in", &[v("s"), v("x")]).atom("up", &[v("s"), v("p")]).finish().unwrap();
        b.rule().head("look", &[v("n"), v("x"), v("s")]).atom("walk", &[v("n"), v("x"), v("s")]).atom("bound_in", &[v("s"), v("x")]).finish().unwrap();
        b.rule().head("chained", &[v("n"), v("x")]).atom("walk", &[v("n"), v("x"), v("s")]).atom("ann", &[v("a"), v("x"), v("s")]).finish().unwrap();
        let v = b.finish().unwrap().0;
        let p = body::Placed::whole(v);
        let f = Fields::of(&p).expect("a whole version");
        let deps = crate::code_rule_dsl::logic::program_image::analyses::edges_by_name(f.v, f.about);
        let sccs = climb_ends(&p, &p.components().unwrap(), &deps).expect("the climb's end joins the walk's stratum");
        let walk = sccs.iter().find(|s| s.contains("walk")).expect("walk's stratum");
        assert!(walk.contains("look"), "a reader of the row alone is answered in the climb: {sccs:?}");
        assert!(!walk.contains("chained"), "a reader that joins out from the row is a stratum of its own: {sccs:?}");
    }
}
