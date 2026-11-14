//! What a run's joins teach the next plans (decision 0203).
//!
//! The planner prices orders from statistics, and a price can be wrong by orders of magnitude where two relations'
//! values are correlated, or where a fanout was observed for another order than the one priced. This module keeps
//! what the runs measured, and feeds it back:
//!
//! - **Alternatives** (`searched`): the search's runners-up, the best order ending in each other join, with their
//!   prices. They are what a later choice weighs, and what exploration tries.
//! - **A memo** (`Note`, per line: a rule, its delta and its component, by the line's plan key): how often the line
//!   ran slow, its share of the run's critical path, each order tried with its measured work over its price, the
//!   orders a wider search proposed, and the exact pair statistics measured for it. Kept across runs in the store,
//!   beside the kept plans (`choices.rs`).
//! - **The choice** (`choose`): for a round of lines at once, after one batch prediction (`Pricer`), the order of
//!   least price corrected by what its order measured; now and then an untried order within a margin of it, for a
//!   line that matters (`explore`).
//! - **Slow joins** (`Run::drive`): a line whose work passes its price by `slow_factor` is flagged while it runs.
//!   Its rule's exact pair statistics are measured on the engine's rows (`put_exact`), and it is planned again.
//! - **The critical path** (`critical`): at a run's end, the longest chain of strata by work; its lines with at
//!   least `critical_share` of it are noted, and asked to be searched wider.
//! - **The wider search** (`request`, the `join-search` upkeep job, decision 0190): the exact search past the
//!   greedy bound, and random connected orders, priced with the exact statistics; the best are the line's proposals.

use super::{Constant, Mode, PlanKey, Sig, Step, StoredStep};
use parking_lot::{Mutex, RwLock};
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};

/// The search's runners-up kept per line.
pub(super) const ALTERNATIVES: usize = 4;
/// Orders tried kept per line.
const TRIED: usize = 8;
/// Orders a wider search proposes per line.
const PROPOSALS: usize = 6;
/// The wider search's exact bound, in joins (the run's is `GREEDY`).
pub(super) const WIDE_GREEDY: usize = 16;
/// Random connected orders a wider search prices.
pub(super) const RANDOM_ORDERS: usize = 48;
/// Lines noted, at most: the least important go first.
const NOTES_MAX: usize = 4096;
/// Rows a step's histogram reads at most (a stride beyond).
pub(super) const HISTOGRAM_ROWS: usize = 1 << 20;

fn setting(name: &str, or: f64) -> f64 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(or)
}

macro_rules! cached {
    ($name:ident, $env:literal, $or:expr) => {
        pub(super) fn $name() -> f64 {
            static V: std::sync::LazyLock<f64> = std::sync::LazyLock::new(|| setting($env, $or));
            *V
        }
    };
}
// A line is slow past this many times its price, over `slow_rows` rows of work.
cached!(slow_factor, "RERAIL_JOIN_SLOW", 8.0);
cached!(slow_rows, "RERAIL_JOIN_SLOW_ROWS", 100_000.0);
// A line on the critical path with this share of it is noted.
cached!(critical_share, "RERAIL_JOIN_CRITICAL", 0.05);
// The chance a noted line tries an untried order, and how far over the best's price that order may be.
cached!(explore_rate, "RERAIL_JOIN_EXPLORE", 0.1);
cached!(margin, "RERAIL_JOIN_MARGIN", 2.0);
// Decision 0207's trials: the rows of a sample, an order's cutoff as a multiple of the best measured so far, the
// share of the line's work so far each may spend, and by how much a trialled order must beat the one running.
cached!(trial_rows, "RERAIL_JOIN_TRIAL_ROWS", 256.0);
cached!(trial_cutoff, "RERAIL_JOIN_TRIAL_CUTOFF", 2.0);
cached!(trial_share, "RERAIL_JOIN_TRIAL_SHARE", 0.1);
cached!(trial_margin, "RERAIL_JOIN_TRIAL_MARGIN", 1.25);

/// `RERAIL_JOIN_TRIALS=off`: a line's orders are never trialled (decision 0207).
pub(super) fn trials_on() -> bool {
    static ON: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var("RERAIL_JOIN_TRIALS").map_or(true, |v| v != "off"));
    on() && *ON
}

/// Whether line `key`'s orders are trialled at this check: the first time in this process, and once after it was
/// flagged slow.
pub(super) fn take_trial(key: PlanKey) -> bool {
    MEMO.write().trialled.insert(key)
}

/// A trial measured `ratio` (work over price) for `order` of line `key`: kept as a run's is (`critical`).
pub(super) fn put_tried(key: PlanKey, rule: u64, order: &[Step], ratio: f64) {
    COUNTS[TRIALS].fetch_add(1, Relaxed);
    let mut m = MEMO.write();
    let n = noted_mut(&mut m, key, rule);
    merge_tried(n, steps(order), ratio.clamp(1e-6, 1e9));
}

fn merge_tried(n: &mut Note, order: Vec<StoredStep>, ratio: f64) {
    match n.tried.iter_mut().find(|t| t.order == order) {
        Some(t) => {
            t.ratio = (t.ratio * ratio).sqrt();
            t.runs += 1;
        }
        None => {
            if n.tried.len() >= TRIED {
                n.tried.remove(0);
            }
            n.tried.push(Tried { order, ratio, runs: 1 });
        }
    }
}

/// Line `key`'s proposals from a wider search, as orders.
pub(super) fn proposals(key: &PlanKey) -> Vec<Vec<Step>> {
    MEMO.read().notes.get(key).map(|n| n.proposals.iter().map(|(o, _)| o.iter().map(|&(at, mode)| Step { at: at as usize, mode }).collect()).collect()).unwrap_or_default()
}

/// `RERAIL_JOINS=off`: no memo, no flags, no exploration (the planner as before).
pub(super) fn on() -> bool {
    static ON: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var("RERAIL_JOINS").map_or(true, |v| v != "off"));
    *ON
}

/// `RERAIL_JOINS_LOG`: each flag, choice and critical path to standard error.
pub(super) fn log(what: impl FnOnce() -> String) {
    static ON: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var_os("RERAIL_JOINS_LOG").is_some());
    if *ON {
        eprintln!("joins: {}", what());
    }
}

/// An order a line ran, and how its measured work compared with its price.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tried {
    pub order: Vec<StoredStep>,
    /// Measured work over priced work, a geometric mean over its runs.
    pub ratio: f64,
    pub runs: u32,
}

/// What the runs learnt of one line.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Note {
    /// The rule's observation key (`Body::observed_key`), which its exact statistics are kept under.
    pub rule: u64,
    pub runs: u32,
    /// Times it was flagged slow while it ran.
    pub slow: u32,
    /// Its share of the critical path, a moving average over runs.
    pub critical: f64,
    /// Its price and measured work when it last ran.
    pub est: f64,
    pub ran: f64,
    /// A wider search is wanted (`request`), not made yet.
    pub wants_search: bool,
    pub tried: Vec<Tried>,
    /// Orders a wider search found, with their prices then.
    pub proposals: Vec<(Vec<StoredStep>, f64)>,
    /// The rule's exact pair statistics (`put_exact`): per binding pattern, bindings in and rows out.
    pub exact: Vec<(Sig, [i64; 2])>,
}

impl Note {
    fn weight(&self) -> f64 {
        (self.critical + 0.01 * f64::from(self.slow)) * self.ran.max(1.0)
    }
    fn tried(&self, order: &[StoredStep]) -> Option<&Tried> {
        self.tried.iter().find(|t| t.order == order)
    }
}

#[derive(Default)]
struct Memo {
    notes: FxHashMap<PlanKey, Note>,
    /// Per rule, its exact statistics, as the search reads them.
    exact: FxHashMap<u64, Arc<HashMap<Sig, [i64; 2]>>>,
    /// Per rule, how many times its exact statistics were measured (in the plan key: a new measure is a new plan).
    epochs: FxHashMap<u64, u32>,
    /// Lines flagged in this process, and the work at which each is checked next (`check`).
    flagged: FxHashSet<PlanKey>,
    next_check: FxHashMap<PlanKey, f64>,
    /// Lines whose next replan skips the planning budget (a flag's).
    forced: FxHashSet<PlanKey>,
    /// Lines whose orders were trialled in this process (decision 0207): once, and once more after a flag.
    trialled: FxHashSet<PlanKey>,
    dirty: bool,
}

static MEMO: std::sync::LazyLock<RwLock<Memo>> = std::sync::LazyLock::new(Default::default);
/// Whether any line is noted, or any rule has exact statistics: the planner's fast paths read these alone.
static NOTED: AtomicBool = AtomicBool::new(false);
static EXACT: AtomicBool = AtomicBool::new(false);
/// Counts for the profile and tests: flags, exact statistics measured, choices corrected, explored, proposals,
/// handoffs and handbacks (decision 0206), orders trialled (0206 §2).
pub(super) static COUNTS: [AtomicU64; 8] = [const { AtomicU64::new(0) }; 8];
pub(super) const FLAGS: usize = 0;
pub(super) const MEASURED: usize = 1;
pub(super) const CORRECTED: usize = 2;
pub(super) const EXPLORED: usize = 3;
pub(super) const PROPOSED: usize = 4;
pub(super) const HANDOFFS: usize = 5;
pub(super) const HANDBACKS: usize = 6;
pub(super) const TRIALS: usize = 7;

pub(super) fn steps(order: &[Step]) -> Vec<StoredStep> {
    order.iter().map(|s| (s.at as u16, s.mode)).collect()
}

pub(super) fn unstored(order: &[StoredStep]) -> Vec<Step> {
    order.iter().map(|&(at, mode)| Step { at: at.into(), mode }).collect()
}

pub(super) fn noted(key: &PlanKey) -> bool {
    NOTED.load(Relaxed) && MEMO.read().notes.contains_key(key)
}

pub(super) fn any_noted() -> bool {
    NOTED.load(Relaxed)
}

pub fn note(key: &PlanKey) -> Option<Note> {
    if !NOTED.load(Relaxed) {
        return None;
    }
    MEMO.read().notes.get(key).cloned()
}

/// Whether the line has work queued for its next plan: a wider search wanted, or a proposal not tried yet. Such a
/// line is searched, not taken at its kept price.
pub(super) fn pending(key: &PlanKey) -> bool {
    NOTED.load(Relaxed) && MEMO.read().notes.get(key).is_some_and(|n| n.wants_search || n.proposals.iter().any(|(o, _)| n.tried(o).is_none()))
}

/// The epoch of `rule`'s exact statistics (0: none), digested into its plan keys.
pub(super) fn epoch(rule: u64) -> u32 {
    if !EXACT.load(Relaxed) {
        return 0;
    }
    MEMO.read().epochs.get(&rule).copied().unwrap_or(0)
}

pub(super) fn any_exact() -> bool {
    EXACT.load(Relaxed)
}

/// `rule`'s exact statistics, read once per search.
pub(super) fn exact_for(rule: u64) -> Option<Arc<HashMap<Sig, [i64; 2]>>> {
    if !EXACT.load(Relaxed) {
        return None;
    }
    MEMO.read().exact.get(&rule).cloned()
}

fn noted_mut<'m>(memo: &'m mut Memo, key: PlanKey, rule: u64) -> &'m mut Note {
    NOTED.store(true, Relaxed);
    memo.dirty = true;
    if memo.notes.len() >= NOTES_MAX && !memo.notes.contains_key(&key) {
        // The least important tenth goes.
        let mut w: Vec<(f64, PlanKey)> = memo.notes.iter().map(|(k, n)| (n.weight(), *k)).collect();
        w.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (_, k) in w.into_iter().take(NOTES_MAX / 10) {
            memo.notes.remove(&k);
        }
    }
    let n = memo.notes.entry(key).or_default();
    n.rule = rule;
    n
}

/// Whether a line's work so far, `work`, is due to be checked against its price: the first time past `slow_rows`,
/// then each time it doubles (a price per check, so checking costs a fraction of the work, decision 0122). Never
/// once flagged.
pub(super) fn check(key: PlanKey, work: f64) -> bool {
    if work < slow_rows() {
        return false;
    }
    let mut m = MEMO.write();
    if m.flagged.contains(&key) {
        return false;
    }
    let next = m.next_check.entry(key).or_insert(0.0);
    if work < *next {
        return false;
    }
    *next = 2.0 * work;
    true
}

/// The work at which line `key` is next checked (`check`), so the engine cuts its probe there (decision 0206);
/// none once flagged.
pub(super) fn next_due(key: PlanKey) -> Option<f64> {
    let m = MEMO.read();
    if m.flagged.contains(&key) {
        return None;
    }
    Some(m.next_check.get(&key).copied().unwrap_or(0.0).max(slow_rows()))
}

/// Decision 0206: a slow order replaced at its cut. `old`, its order; `per_row`, its work per delta row finished;
/// `limit`, the work its replacement may do over the rows left before the two are compared.
pub(in crate::code_rule_dsl::logic) struct Handoff { pub old: Vec<Step>, pub per_row: f64, pub limit: f64 }

/// A handoff made, or ended with the old order going on (`back`).
pub(super) fn count_handoff(back: bool) {
    COUNTS[if back { HANDBACKS } else { HANDOFFS }].fetch_add(1, Relaxed);
}

/// A line ran slow: `ran` rows of work where its order was priced at `est`. Its next replan skips the budget.
pub(super) fn flag_slow(key: PlanKey, rule: u64, est: f64, ran: f64) {
    COUNTS[FLAGS].fetch_add(1, Relaxed);
    let mut m = MEMO.write();
    m.flagged.insert(key);
    m.forced.insert(key);
    m.trialled.remove(&key);
    let n = noted_mut(&mut m, key, rule);
    n.slow += 1;
    n.est = est;
    n.ran = ran;
    n.wants_search = true;
}

/// Whether the line's replan skips the planning budget, once.
pub(super) fn take_forced(key: &PlanKey) -> bool {
    NOTED.load(Relaxed) && MEMO.write().forced.remove(key)
}

/// `rule`'s exact pair statistics, measured for line `key`: the rule's plans are keyed again (`epoch`).
pub(super) fn put_exact(key: PlanKey, rule: u64, stats: Vec<(Sig, [i64; 2])>) {
    if stats.is_empty() {
        return;
    }
    COUNTS[MEASURED].fetch_add(stats.len() as u64, Relaxed);
    let mut m = MEMO.write();
    let n = noted_mut(&mut m, key, rule);
    for (sig, c) in stats {
        match n.exact.iter_mut().find(|(s, _)| *s == sig) {
            Some(e) => e.1 = c,
            None => n.exact.push((sig, c)),
        }
    }
    let map: HashMap<Sig, [i64; 2]> = n.exact.iter().cloned().collect();
    m.exact.insert(rule, Arc::new(map));
    *m.epochs.entry(rule).or_insert(0) += 1;
    EXACT.store(true, Relaxed);
}

/// A wider search's orders for line `key`, best first: its proposals.
pub(super) fn propose(key: PlanKey, mut orders: Vec<(Vec<StoredStep>, f64)>) {
    orders.sort_by(|a, b| a.1.total_cmp(&b.1));
    orders.dedup_by(|a, b| a.0 == b.0);
    orders.truncate(PROPOSALS);
    COUNTS[PROPOSED].fetch_add(orders.len() as u64, Relaxed);
    log(|| format!("proposals for line {:x}: {} orders, prices {:?}", key as u64, orders.len(), orders.iter().map(|o| o.1.round()).collect::<Vec<_>>()));
    let mut m = MEMO.write();
    let Some(n) = m.notes.get_mut(&key) else { return };
    n.proposals = orders;
    n.wants_search = false;
    m.dirty = true;
}

thread_local! {
    /// The last search's runners-up on this thread (`plan_search`), taken by its caller.
    static SEARCHED: std::cell::RefCell<Vec<(Vec<Step>, f64)>> = const { std::cell::RefCell::new(Vec::new()) };
    /// While set, searches are exact up to `WIDE_GREEDY` joins and never take the greedy order first.
    static WIDE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// While a round is open (`Round`), the choices its lines asked for.
    static ROUND: std::cell::RefCell<Option<Vec<Choice>>> = const { std::cell::RefCell::new(None) };
}

pub(super) fn searched_put(alternatives: Vec<(Vec<Step>, f64)>) {
    SEARCHED.with(|s| *s.borrow_mut() = alternatives);
}

pub(super) fn searched_take() -> Vec<(Vec<Step>, f64)> {
    SEARCHED.with(|s| std::mem::take(&mut *s.borrow_mut()))
}

pub(super) fn wide() -> bool {
    WIDE.with(std::cell::Cell::get)
}

/// The greedy bound of this thread's searches.
pub(super) fn greedy_limit(run: usize) -> usize {
    if wide() { WIDE_GREEDY.max(run) } else { run }
}

/// Runs `f` with searches wide (`WIDE`).
pub(super) fn widely<T>(f: impl FnOnce() -> T) -> T {
    let was = WIDE.with(|w| w.replace(true));
    let got = f();
    WIDE.with(|w| w.set(was));
    got
}

/// An order a choice weighs, at its price now.
#[derive(Clone, Debug)]
pub struct Candidate {
    pub order: Vec<Step>,
    pub est: f64,
}

/// One line's choice in a round: its candidates (the search's best first), and what was chosen.
#[derive(Clone, Debug)]
pub struct Choice {
    pub key: PlanKey,
    pub candidates: Vec<Candidate>,
    pub chosen: usize,
    pub why: &'static str,
}

/// A batch prediction of the candidates' work (decision 0203 §2): one call per round, a price per candidate. The
/// planner's estimates stand where none is set, or where it answers `None`.
pub trait Pricer: Send + Sync {
    fn price(&self, round: &[Choice]) -> Option<Vec<Vec<f64>>>;
}

static PRICER: RwLock<Option<Arc<dyn Pricer>>> = RwLock::new(None);

pub fn set_pricer(p: Option<Arc<dyn Pricer>>) {
    PRICING.store(p.is_some(), Relaxed);
    *PRICER.write() = p;
}

static PRICING: AtomicBool = AtomicBool::new(false);

/// Whether a batch pricer is set: every searched line's choice is then made in its round.
pub(super) fn pricing() -> bool {
    PRICING.load(Relaxed)
}

/// A small generator for exploration and random orders: `RERAIL_JOIN_SEED` fixes it.
pub(super) struct Rng(u64);
impl Rng {
    pub(super) fn new(salt: u64) -> Rng {
        static N: AtomicU64 = AtomicU64::new(0);
        let seed = std::env::var("RERAIL_JOIN_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or_else(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64));
        Rng((seed ^ salt.rotate_left(17) ^ N.fetch_add(0x9e37_79b9_7f4a_7c15, Relaxed)) | 1)
    }
    pub(super) fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    pub(super) fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    pub(super) fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Chooses each line's order in `round`, after one batch prediction of every candidate's work.
///
/// - A line with no note takes the search's best (or the prediction's least, where a pricer is set).
/// - A noted line's candidates are priced by what their orders measured: price × the order's measured ratio. An
///   order never tried keeps its price.
/// - A noted line that matters (on the critical path, or slow) tries, at `explore_rate`, an order not tried yet
///   whose corrected price is within `margin` of the best's.
pub fn choose(round: &mut [Choice]) {
    let predicted = PRICER.read().clone().and_then(|p| p.price(round));
    let memo = if NOTED.load(Relaxed) { Some(MEMO.read()) } else { None };
    for (k, c) in round.iter_mut().enumerate() {
        c.chosen = 0;
        c.why = "the search's best";
        if c.candidates.len() < 2 {
            continue;
        }
        let price = |i: usize| predicted.as_ref().and_then(|p| p.get(k)?.get(i).copied()).unwrap_or(c.candidates[i].est);
        let note = memo.as_ref().and_then(|m| m.notes.get(&c.key));
        let corrected = |i: usize| {
            let ratio = note.and_then(|n| n.tried(&steps(&c.candidates[i].order))).map_or(1.0, |t| t.ratio);
            price(i) * ratio
        };
        let costs: Vec<f64> = (0..c.candidates.len()).map(corrected).collect();
        let best = (0..costs.len()).min_by(|&a, &b| costs[a].total_cmp(&costs[b])).unwrap_or(0);
        if best != 0 {
            c.chosen = best;
            c.why = if note.is_some() { "corrected by what its orders measured" } else { "the batch prediction's least" };
            COUNTS[CORRECTED].fetch_add(1, Relaxed);
        }
        let Some(n) = note else { continue };
        if n.critical < critical_share() && n.slow == 0 {
            continue;
        }
        let untried: Vec<usize> = (0..costs.len()).filter(|&i| i != best && costs[i] <= margin() * costs[best].max(1.0) && n.tried(&steps(&c.candidates[i].order)).is_none()).collect();
        if untried.is_empty() {
            continue;
        }
        let mut rng = Rng::new(c.key as u64);
        if rng.unit() < explore_rate() {
            c.chosen = untried[rng.below(untried.len())];
            c.why = "explored: an order not tried, within the margin";
            COUNTS[EXPLORED].fetch_add(1, Relaxed);
        }
    }
}

/// A round of choices (decision 0203 §2): while it is open, a line planned on this thread records its choice here
/// and keeps the search's best; `finish` chooses them all at once.
pub(super) struct Round(Option<Vec<Choice>>);

impl Round {
    pub(super) fn open() -> Round {
        Round(ROUND.with(|r| r.borrow_mut().replace(Vec::new())))
    }
    /// The choices recorded so far.
    pub(super) fn len() -> usize {
        ROUND.with(|r| r.borrow().as_ref().map_or(0, Vec::len))
    }
    pub(super) fn finish(mut self) -> Vec<Choice> {
        let mut got = ROUND.with(|r| std::mem::replace(&mut *r.borrow_mut(), self.0.take())).unwrap_or_default();
        choose(&mut got);
        got
    }
}

impl Drop for Round {
    fn drop(&mut self) {
        if let Some(was) = self.0.take() {
            ROUND.with(|r| *r.borrow_mut() = Some(was));
        }
    }
}

/// Records `choice` in the open round, if one is: `true` where it was (the caller keeps the search's best).
pub(super) fn deferred(choice: &Choice) -> bool {
    ROUND.with(|r| match r.borrow_mut().as_mut() {
        Some(v) => {
            v.push(choice.clone());
            true
        }
        None => false,
    })
}

/// One line as it ran: its stratum, key, rule, the order it ran last, its measured work, and its price for the
/// rows it read (0 where not priced).
#[derive(Clone, Debug)]
pub struct LineRun {
    pub stratum: String,
    pub key: PlanKey,
    pub rule: u64,
    pub order: Vec<StoredStep>,
    pub work: f64,
    pub est: f64,
}

/// What `critical` found, for the profile.
#[derive(Debug, Default, PartialEq)]
pub struct Path {
    /// The strata on it, in order, and its work.
    pub strata: Vec<usize>,
    pub work: f64,
    /// Its lines noted, with their shares.
    pub lines: Vec<(PlanKey, f64)>,
}

/// The run's critical path (decision 0203 §4): strata `producers` (each stratum's, by index; strata in dependency
/// order, as a run takes them), weighted by the work of their lines `runs`. The longest chain is the path. Each of
/// its lines with `critical_share` of it is noted: its share, its order tried, and a wider search asked for while
/// its order measured over twice its price or none was made. Lines noted before have their order recorded too.
pub fn critical(strata: &[String], producers: &[Vec<usize>], runs: &[LineRun]) -> Path {
    let at: HashMap<&str, usize> = strata.iter().enumerate().map(|(j, s)| (s.as_str(), j)).collect();
    let mut weight = vec![0.0f64; strata.len()];
    for r in runs {
        if let Some(&j) = at.get(r.stratum.as_str()) {
            weight[j] += r.work;
        }
    }
    let mut dist = vec![0.0f64; strata.len()];
    let mut prev: Vec<Option<usize>> = vec![None; strata.len()];
    for j in 0..strata.len() {
        let from = producers.get(j).into_iter().flatten().filter(|&&p| p < j).max_by(|&&a, &&b| dist[a].total_cmp(&dist[b])).copied();
        dist[j] = weight[j] + from.map_or(0.0, |p| dist[p]);
        prev[j] = from;
    }
    let Some(end) = (0..strata.len()).max_by(|&a, &b| dist[a].total_cmp(&dist[b])) else { return Path::default() };
    let mut path = Vec::new();
    let mut at_ = Some(end);
    while let Some(j) = at_ {
        path.push(j);
        at_ = prev[j];
    }
    path.reverse();
    let total = dist[end];
    let on: FxHashSet<usize> = path.iter().copied().collect();
    let mut lines = Vec::new();
    if total <= 0.0 {
        return Path { strata: path, work: total, lines };
    }
    let mut m = MEMO.write();
    for r in runs {
        let share = at.get(r.stratum.as_str()).filter(|j| on.contains(j)).map_or(0.0, |_| r.work / total);
        let critical = share >= critical_share() && r.work >= slow_rows().min(1e4);
        if !critical && !m.notes.contains_key(&r.key) {
            continue;
        }
        let n = noted_mut(&mut m, r.key, r.rule);
        n.runs += 1;
        n.critical = if n.runs == 1 { share } else { 0.5 * n.critical + 0.5 * share };
        n.ran = r.work;
        if r.est > 0.0 {
            n.est = r.est;
            let ratio = (r.work.max(1.0) / r.est.max(1.0)).clamp(1e-6, 1e9);
            merge_tried(n, r.order.clone(), ratio);
            if critical && (ratio > 2.0 || n.proposals.is_empty()) {
                n.wants_search = true;
            }
        }
        if critical {
            lines.push((r.key, share));
        }
    }
    drop(m);
    prioritize(&lines, total);
    Path { strata: path, work: total, lines }
}

/// Random connected orders of joins `vars` (each join's variables, as bits), from `start` where given, else from any
/// join: each next join shares a variable with those placed while one can. Distinct, at most `n`.
pub(super) fn random_orders(vars: &[u128], start: Option<usize>, n: usize, rng: &mut Rng) -> Vec<Vec<usize>> {
    let mut out: Vec<Vec<usize>> = Vec::new();
    let mut seen: FxHashSet<Vec<usize>> = FxHashSet::default();
    for _ in 0..n * 4 {
        if out.len() >= n || vars.is_empty() {
            break;
        }
        let first = start.unwrap_or_else(|| rng.below(vars.len()));
        let mut order = vec![first];
        let mut bound = vars[first];
        let mut left: Vec<usize> = (0..vars.len()).filter(|&i| i != first).collect();
        while !left.is_empty() {
            let connected: Vec<usize> = (0..left.len()).filter(|&k| vars[left[k]] & bound != 0).collect();
            let k = if connected.is_empty() { rng.below(left.len()) } else { connected[rng.below(connected.len())] };
            let j = left.swap_remove(k);
            bound |= vars[j];
            order.push(j);
        }
        if seen.insert(order.clone()) {
            out.push(order);
        }
    }
    out
}

/// Wider searches waiting for the upkeep job, most important first: each a line's key, its weight, and the search,
/// which returns its orders with their prices.
type Search = Box<dyn FnOnce() -> Vec<(Vec<StoredStep>, f64)> + Send>;
static REQUESTS: Mutex<Vec<(PlanKey, f64, Search)>> = Mutex::new(Vec::new());
/// Requests waiting at most (the least important are dropped).
const REQUESTS_MAX: usize = 64;

/// Asks for a wider search of line `key` (`search`), run by the `join-search` upkeep job at idle.
pub(super) fn request(key: PlanKey, weight: f64, search: Search) {
    job();
    let mut q = REQUESTS.lock();
    if q.iter().any(|(k, _, _)| *k == key) {
        return;
    }
    q.push((key, weight, search));
    q.sort_by(|a, b| b.1.total_cmp(&a.1));
    q.truncate(REQUESTS_MAX);
}

/// Whether a wider search of line `key` is worth asking for: none waits for it, and it has no proposals yet.
pub(super) fn wants_request(key: &PlanKey) -> bool {
    if REQUESTS.lock().iter().any(|(k, _, _)| k == key) {
        return false;
    }
    !(NOTED.load(Relaxed) && MEMO.read().notes.get(key).is_some_and(|n| !n.proposals.is_empty() && !n.wants_search))
}

/// Raises the waiting searches of the critical path's lines to its work: they are searched first.
fn prioritize(lines: &[(PlanKey, f64)], total: f64) {
    let mut q = REQUESTS.lock();
    for (k, w, _) in q.iter_mut() {
        if let Some((_, share)) = lines.iter().find(|(l, _)| l == k) {
            *w = w.max(share * total) + total;
        }
    }
    q.sort_by(|a, b| b.1.total_cmp(&a.1));
}

/// Requests waiting, for tests and the profile.
pub fn requests() -> usize {
    REQUESTS.lock().len()
}

/// Runs the most important waiting search now, on this thread: `false` where none waits.
pub fn search_one() -> bool {
    let Some((key, _, search)) = ({
        let mut q = REQUESTS.lock();
        (!q.is_empty()).then(|| q.remove(0))
    }) else {
        return false;
    };
    let orders = search();
    propose(key, orders);
    true
}

struct JoinSearch;

impl rerail_accel::upkeep::Job for JoinSearch {
    fn name(&self) -> String {
        "join-search".into()
    }
    fn due(&mut self) -> rerail_accel::upkeep::Due {
        if REQUESTS.lock().is_empty() {
            rerail_accel::upkeep::Due::No
        } else {
            rerail_accel::upkeep::Due::Idle { starve: std::time::Duration::from_secs(300) }
        }
    }
    fn step(&mut self, slice: &rerail_accel::upkeep::Slice) -> Result<rerail_accel::upkeep::Step, String> {
        while !slice.over() {
            if !search_one() {
                return Ok(rerail_accel::upkeep::Step::Done);
            }
        }
        Ok(rerail_accel::upkeep::Step::More)
    }
}

/// The job, registered once per process when the first search is asked for.
fn job() {
    static JOB: std::sync::OnceLock<Mutex<Option<rerail_accel::upkeep::Registered>>> = std::sync::OnceLock::new();
    JOB.get_or_init(|| Mutex::new(Some(rerail_accel::upkeep::register(Box::new(JoinSearch)))));
}

/// Clears the memo (tests).
#[cfg(any(test, feature = "test-hooks"))]
pub fn clear() {
    *MEMO.write() = Memo::default();
    NOTED.store(false, Relaxed);
    EXACT.store(false, Relaxed);
    REQUESTS.lock().clear();
}

// Persistence (`choices.rs`): the notes as rows, kept beside the plans.

fn float(x: f64) -> i64 {
    x.to_bits() as i64
}

fn unfloat(x: i64) -> f64 {
    f64::from_bits(x as u64)
}

fn push_steps(row: &mut Vec<i64>, order: &[StoredStep]) {
    row.push(order.len() as i64);
    for four in order.chunks(4) {
        let mut x = 0u64;
        for (i, &(at, mode)) in four.iter().enumerate() {
            let m = match mode { None => 0u64, Some(Mode::Range) => 1, Some(Mode::Climb) => 2, Some(Mode::Descend) => 3 };
            x |= (u64::from(at) << 2 | m) << (16 * i);
        }
        row.push(x as i64);
    }
}

fn read_steps(next: &mut dyn FnMut() -> Option<i64>) -> Option<Vec<StoredStep>> {
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
}

/// A note as integers: `[key high, key low, rule, runs, slow, critical, est, ran, wants, tried: n, (ratio, runs,
/// steps)…, proposals: n, (price, steps)…]` (floats by their bits).
pub fn note_row(key: PlanKey, n: &Note) -> Vec<i64> {
    let mut row = vec![(key >> 64) as i64, key as i64, n.rule as i64, i64::from(n.runs), i64::from(n.slow), float(n.critical), float(n.est), float(n.ran), i64::from(n.wants_search)];
    row.push(n.tried.len() as i64);
    for t in &n.tried {
        row.push(float(t.ratio));
        row.push(i64::from(t.runs));
        push_steps(&mut row, &t.order);
    }
    row.push(n.proposals.len() as i64);
    for (o, p) in &n.proposals {
        row.push(float(*p));
        push_steps(&mut row, o);
    }
    row
}

pub fn of_note_row(row: &[i64]) -> Option<(PlanKey, Note)> {
    let mut at = row.iter().copied();
    let mut next = || at.next();
    let key = (next()? as u64 as u128) << 64 | next()? as u64 as u128;
    let mut n = Note { rule: next()? as u64, runs: u32::try_from(next()?).ok()?, slow: u32::try_from(next()?).ok()?, critical: unfloat(next()?), est: unfloat(next()?), ran: unfloat(next()?), wants_search: next()? != 0, ..Default::default() };
    for _ in 0..usize::try_from(next()?).ok()?.min(TRIED) {
        let ratio = unfloat(next()?);
        let runs = u32::try_from(next()?).ok()?;
        n.tried.push(Tried { order: read_steps(&mut next)?, ratio, runs });
    }
    for _ in 0..usize::try_from(next()?).ok()?.min(PROPOSALS) {
        let p = unfloat(next()?);
        n.proposals.push((read_steps(&mut next)?, p));
    }
    Some((key, n))
}

/// A constant as text, for an exact statistic's row: `i5`, or `s` and the string.
fn constant_text(c: &Constant) -> String {
    match c {
        Constant::Int(i) => format!("i{i}"),
        Constant::Str(s) => format!("s{s}"),
    }
}

/// An exact statistic as values: `[key high, key low, rule, relation, columns, in, out]`; the columns as
/// `c` or `c=constant`, joined by `\u{1}`.
pub fn exact_row(key: PlanKey, rule: u64, (rel, cols): &Sig, c: [i64; 2]) -> (Vec<i64>, String, String, [i64; 2]) {
    let text: Vec<String> = cols.iter().map(|(col, k)| match k { None => col.to_string(), Some(k) => format!("{col}={}", constant_text(k)) }).collect();
    (vec![(key >> 64) as i64, key as i64, rule as i64], rel.clone(), text.join("\u{1}"), c)
}

pub fn of_exact_row(ints: &[i64], rel: &str, cols: &str, c: [i64; 2]) -> Option<(PlanKey, u64, Sig, [i64; 2])> {
    let [hi, lo, rule] = *ints else { return None };
    let key = (hi as u64 as u128) << 64 | lo as u64 as u128;
    let mut parsed = Vec::new();
    for part in cols.split('\u{1}').filter(|p| !p.is_empty()) {
        let (col, k) = match part.split_once('=') {
            None => (part.parse().ok()?, None),
            Some((col, k)) => (col.parse().ok()?, Some(match k.split_at(1) {
                ("i", v) => Constant::Int(v.parse().ok()?),
                ("s", v) => Constant::Str(v.to_owned()),
                _ => return None,
            })),
        };
        parsed.push((col, k));
    }
    Some((key, rule as u64, (rel.to_owned(), parsed), c))
}

/// The memo's notes and exact statistics, for the store, where any changed since the last call.
pub fn to_store() -> Option<(Vec<(PlanKey, Note)>, Vec<(PlanKey, u64, Sig, [i64; 2])>)> {
    let mut m = MEMO.write();
    if !std::mem::take(&mut m.dirty) {
        return None;
    }
    let notes: Vec<(PlanKey, Note)> = m.notes.iter().map(|(k, n)| (*k, n.clone())).collect();
    let exact = notes.iter().flat_map(|(k, n)| n.exact.iter().map(move |(s, c)| (*k, n.rule, s.clone(), *c))).collect();
    Some((notes, exact))
}

/// Takes the store's notes: a line this process noted already keeps its own (memory is ahead of the store).
pub fn from_store(notes: Vec<(PlanKey, Note)>, exact: Vec<(PlanKey, u64, Sig, [i64; 2])>) {
    if notes.is_empty() {
        return;
    }
    let mut m = MEMO.write();
    let mut taken: FxHashSet<PlanKey> = FxHashSet::default();
    for (k, n) in notes {
        if !m.notes.contains_key(&k) {
            taken.insert(k);
            m.notes.insert(k, n);
        }
    }
    for (k, rule, sig, c) in exact {
        if taken.contains(&k)
            && let Some(n) = m.notes.get_mut(&k)
        {
            n.exact.push((sig, c));
            let _ = rule;
        }
    }
    let rules: Vec<(u64, HashMap<Sig, [i64; 2]>)> = m.notes.values().filter(|n| !n.exact.is_empty()).map(|n| (n.rule, n.exact.iter().cloned().collect())).collect();
    for (rule, map) in rules {
        m.epochs.entry(rule).or_insert(1);
        m.exact.insert(rule, Arc::new(map));
        EXACT.store(true, Relaxed);
    }
    NOTED.store(!m.notes.is_empty(), Relaxed);
}

#[cfg(test)]
mod tests;
