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
//!   greedy bound, and random connected orders, priced with the exact statistics. Decision 0243: where the request
//!   holds the line's relations (`may_hold`), its best-priced orders and the one that ran are walked over them, and
//!   the proposals are ranked by walked work. The choice costs a walked order not measured against the walked order
//!   that ran (decision 0245). It is the only wider search: the plan runs none. A one-shot process serves the
//!   waiting searches at its run's end (`drain_at_end`), within a share of its CPU; each search stops at its budget.

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
pub(super) const PROPOSALS: usize = 6;
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
            rerail_prof::code_rule_dsl::logic::profile::setting(|| setting($env, $or))
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
// Decision 0354: the learned model's choices explore only on lines it prices under this many rows of work (a
// wrong order there costs little; on hogql an explored 59 M-row line ran 230 M), and price only the candidates
// the statistics price within this many times the best's.
cached!(explore_rows, "RERAIL_JOIN_EXPLORE_ROWS", 1_000_000.0);
cached!(feature_span, "RERAIL_JOIN_FEATURE_SPAN", 16.0);
// Decision 0207's trials: the rows of a sample, an order's cutoff as a multiple of the best measured so far, the
// share of the line's work so far each may spend, and by how much a trialled order must beat the one running.
cached!(trial_rows, "RERAIL_JOIN_TRIAL_ROWS", 256.0);
cached!(trial_cutoff, "RERAIL_JOIN_TRIAL_CUTOFF", 2.0);
cached!(trial_share, "RERAIL_JOIN_TRIAL_SHARE", 0.1);
cached!(trial_margin, "RERAIL_JOIN_TRIAL_MARGIN", 1.25);

/// `RERAIL_JOIN_TRIALS=off`: a line's orders are never trialled (decision 0207).
pub(super) fn trials_on() -> bool {
    on() && rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var("RERAIL_JOIN_TRIALS").map_or(true, |v| v != "off"))
}

/// Decision 0274: whether line `key` was observed on a run's critical path: its note's share of the measured path
/// (0208), a moving average over runs, is at least `critical_share`. A line that leaves the path halves its share
/// each run, and drops below in a few: the next path's lines are found as each is made faster.
pub(super) fn on_critical_path(key: &PlanKey) -> bool {
    joins().noted.load(Relaxed) && joins().memo.read().notes.get(key).is_some_and(|n| n.critical >= critical_share())
}

/// Decision 0274: whether line `key`'s orders would be trialled at this check (`take_trial`), without taking it.
pub(super) fn trial_due(key: &PlanKey) -> bool {
    trials_on() && on_critical_path(key) && !joins().memo.read().trialled.contains(key)
}

/// Whether line `key`'s orders are trialled at this check: decision 0274, only a line observed on a run's critical
/// path, and as 0207, once in this process and once more after it was flagged slow. A slow line off the path is
/// planned again instead, under the handoff's limit (decision 0271).
pub(super) fn take_trial(key: PlanKey) -> bool {
    on_critical_path(&key) && joins().memo.write().trialled.insert(key)
}

/// A trial measured `ratio` (work over price) for `order` of line `key`: kept as a run's is (`critical`).
pub(super) fn put_tried(key: PlanKey, rule: u64, order: &[Step], ratio: f64) {
    run_counts().counts[TRIALS].fetch_add(1, Relaxed);
    let st_ = joins();
    let mut m = st_.memo.write();
    let n = noted_mut(&mut m, key, rule);
    merge_tried(n, steps(order), ratio.clamp(1e-6, 1e9), TRIAL_WEIGHT);
}

/// Decision 0247: how far a trial's estimate moves an order's measured ratio (in logs), against a run's
/// `RUN_WEIGHT`. A trial measures a sample, scaled by walks; a run measures the line.
const TRIAL_WEIGHT: f64 = 0.25;
const RUN_WEIGHT: f64 = 0.5;

/// A trial of `order` of line `key` (decision 0207): `work` rows of work against a price of `est` for the same
/// sample, `cut` where it stopped at its budget.
///
/// A cut trial measured part of its order's work, so its work over its price is only a floor: the order is at least
/// its price. Kept as the ratio, it made the order look as cheap as its cut: `attr_of`'s cross product on PostHog,
/// cut at 3,581 rows of a price of 9.4 M (0.0004), was then chosen over orders measured at 3.5 times their price.
///
/// So a cut trial's ratio is a floor (decision 0267): one under 1 says nothing and is not kept, so the order keeps its
/// price or what it measured, and one over 1 raises the order's kept ratio to it, never lowers it. A measure merged
/// with the trial's weight would move a kept ratio toward the floor from either side. `pre_570`'s order on the
/// data-layer candidates, priced 12.7 M and cut at 7,601 (0.0006), ran 22 s against 6 ms.
pub(super) fn put_trial(key: PlanKey, rule: u64, order: &[Step], work: f64, est: f64, cut: bool) {
    let ratio = (work / est.max(1.0)).clamp(1e-6, 1e9);
    if !cut {
        return put_tried(key, rule, order, ratio);
    }
    if ratio <= 1.0 {
        return;
    }
    run_counts().counts[TRIALS].fetch_add(1, Relaxed);
    let st_ = joins();
    let mut m = st_.memo.write();
    let n = noted_mut(&mut m, key, rule);
    let order = steps(order);
    match n.tried.iter_mut().find(|t| t.order == order) {
        Some(t) => t.ratio = t.ratio.max(ratio),
        None => merge_tried(n, order, ratio, TRIAL_WEIGHT),
    }
}

fn merge_tried(n: &mut Note, order: Vec<StoredStep>, ratio: f64, weight: f64) {
    match n.tried.iter_mut().find(|t| t.order == order) {
        Some(t) => {
            t.ratio = t.ratio.powf(1.0 - weight) * ratio.powf(weight);
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

/// Decision 0243: line `key`'s walked proposals, with their walked work, and the walked work of the order it ran;
/// `None` where its search priced them, or made none.
pub(super) fn walked(key: &PlanKey) -> Option<(f64, Vec<(Vec<Step>, f64)>)> {
    let st_ = joins();
    let m = st_.memo.read();
    let n = m.notes.get(key).filter(|n| n.walked > 0.0 && !n.proposals.is_empty())?;
    Some((n.walked, n.proposals.iter().map(|(o, w)| (o.iter().map(|&(at, mode)| Step { at: at as usize, mode }).collect(), *w)).collect()))
}

/// Line `key`'s proposals from a wider search, as orders.
pub(super) fn proposals(key: &PlanKey) -> Vec<Vec<Step>> {
    joins().memo.read().notes.get(key).map(|n| n.proposals.iter().map(|(o, _)| o.iter().map(|&(at, mode)| Step { at: at as usize, mode }).collect()).collect()).unwrap_or_default()
}

/// `RERAIL_JOINS=off`: no memo, no flags, no exploration (the planner as before).
pub(super) fn on() -> bool {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var("RERAIL_JOINS").map_or(true, |v| v != "off"))
}

/// `RERAIL_JOINS_LOG`: each flag, choice and critical path to standard error.
pub(super) fn logging() -> bool {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var_os("RERAIL_JOINS_LOG").is_some())
}

pub(in crate::code_rule_dsl) fn log(what: impl FnOnce() -> String) {
    if logging() {
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
    /// Orders a wider search found, with their prices then; or, where the search walked them (`walked` above
    /// 0), their walked work over the line.
    pub proposals: Vec<(Vec<StoredStep>, f64)>,
    /// Decision 0243: the walked work over the line of the order it ran, where the search walked its orders (0:
    /// they were priced).
    pub walked: f64,
    /// The rule's exact pair statistics (`put_exact`): per binding pattern, bindings in and rows out.
    pub exact: Vec<(Sig, [i64; 2])>,
    /// The columns the line raised to keep their heavy values (decision 0209 §4): (relation, column). Kept with the
    /// note, so a later process makes them from its first plan (O172).
    pub raised: Vec<(String, usize)>,
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
    /// Per rule, how many times its exact statistics were measured (a rule with any is priced at each plan).
    epochs: FxHashMap<u64, u32>,
    /// Lines flagged in this process, by the order each was flagged in, and the work at which each is checked next
    /// (`check`).
    flagged: FxHashMap<PlanKey, Vec<StoredStep>>,
    next_check: FxHashMap<PlanKey, f64>,
    /// Lines whose next replan skips the planning budget (a flag's).
    forced: FxHashSet<PlanKey>,
    /// Lines whose orders were trialled in this process (decision 0207): once, and once more after a flag.
    trialled: FxHashSet<PlanKey>,
    /// Columns raised to keep their heavy values (decision 0209 §4): a checked line's skewed join columns, and those
    /// of lines on the measured path that ran over twice their price. Made from the start of each later plan.
    raised: FxHashSet<(String, usize)>,
    dirty: bool,
    /// Decision 0247: each note's decision fingerprint as last written or read (`fingerprint`): a run that changed
    /// nothing the planner reads writes nothing.
    written: FxHashMap<PlanKey, u64>,
}

/// Decision 0247: what of a note a plan's choice reads, as a hash: its orders tried and proposals with their
/// measures to a quarter octave, its walked work, whether a search is wanted, its exact statistics and raised
/// columns, and whether it is critical or slow. Its runs, moving shares and last measures are not: a warm run that
/// ran its joins again rewrote every note and, with them, the kept plans (django-bugs decision 0228).
fn fingerprint(n: &Note) -> u64 {
    use std::hash::{Hash, Hasher};
    let q = |x: f64| if x > 0.0 { (x.log2() * 4.0).round() as i64 } else { i64::MIN };
    let mut h = std::collections::hash_map::DefaultHasher::new();
    n.rule.hash(&mut h);
    for t in &n.tried { format!("{:?}", t.order).hash(&mut h); q(t.ratio).hash(&mut h); }
    for (o, w) in &n.proposals { format!("{o:?}").hash(&mut h); q(*w).hash(&mut h); }
    q(n.walked).hash(&mut h);
    n.wants_search.hash(&mut h);
    format!("{:?}", n.exact).hash(&mut h);
    n.raised.hash(&mut h);
    (n.critical >= critical_share()).hash(&mut h);
    (n.slow > 0).hash(&mut h);
    h.finish()
}

/// The join statistics of the caller's programs (decision 0442: the shared object's): the memo, its flags (whether
/// any line is noted, any rule has exact statistics, any column is raised: the planner's fast paths read these
/// alone), the searches waiting, and the upkeep job that runs them.
#[derive(Default)]
struct Joins {
    memo: RwLock<Memo>,
    noted: AtomicBool,
    exact: AtomicBool,
    raised: AtomicBool,
    requests: Mutex<Vec<(PlanKey, f64, bool, Search)>>,
    job: std::sync::OnceLock<Mutex<Option<rerail_accel::upkeep::Registered>>>,
}

fn joins() -> Arc<Joins> {
    super::super::profile::shared::<Joins>()
}
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

/// Raises column `c` of `rel` to keep its heavy values (decision 0209 §4), for line `key` where a line raised it
/// (kept with its note); whether it was not yet.
pub(super) fn raise(key: Option<PlanKey>, rel: &str, c: usize) -> bool {
    joins().raised.store(true, Relaxed);
    let st_ = joins();
    let mut m = st_.memo.write();
    if let Some(key) = key {
        // A trial raises before its line is noted (`put_tried` notes it, with its rule, right after).
        joins().noted.store(true, Relaxed);
        let n = m.notes.entry(key).or_default();
        if !n.raised.iter().any(|(r, k)| r == rel && *k == c) {
            n.raised.push((rel.to_owned(), c));
            m.dirty = true;
        }
    }
    m.raised.insert((rel.to_owned(), c))
}

/// Whether column `c` of `rel` keeps its heavy values.
#[cfg(test)]
pub(super) fn raised(rel: &str, c: usize) -> bool {
    joins().raised.load(Relaxed) && joins().memo.read().raised.contains(&(rel.to_owned(), c))
}

/// The columns that keep their heavy values, read once (`Run::heavy_for` checks every shared column of every rule
/// of a stratum, in every chunk: a read of the memo's lock and a key allocated for each was 0.4 s of a cold run).
pub(super) fn raised_all() -> Vec<(String, usize)> {
    if !joins().raised.load(Relaxed) {
        return Vec::new();
    }
    joins().memo.read().raised.iter().cloned().collect()
}

pub(super) fn any_raised() -> bool {
    joins().raised.load(Relaxed)
}


pub(super) fn noted(key: &PlanKey) -> bool {
    joins().noted.load(Relaxed) && joins().memo.read().notes.contains_key(key)
}

pub(super) fn any_noted() -> bool {
    joins().noted.load(Relaxed)
}

pub fn note(key: &PlanKey) -> Option<Note> {
    if !joins().noted.load(Relaxed) {
        return None;
    }
    joins().memo.read().notes.get(key).cloned()
}

/// Whether the line has work queued for its next plan: a wider search wanted, or a proposal not tried yet. Such a
/// line is searched, not taken at its kept price.
pub(super) fn pending(key: &PlanKey) -> bool {
    joins().noted.load(Relaxed) && joins().memo.read().notes.get(key).is_some_and(|n| n.wants_search || n.proposals.iter().any(|(o, _)| n.tried(o).is_none()))
}

/// The epoch of `rule`'s exact statistics (0: none), digested into its plan keys.
pub(super) fn epoch(rule: u64) -> u32 {
    if !joins().exact.load(Relaxed) {
        return 0;
    }
    joins().memo.read().epochs.get(&rule).copied().unwrap_or(0)
}

pub(super) fn any_exact() -> bool {
    joins().exact.load(Relaxed)
}

/// `rule`'s exact statistics, read once per search.
pub(super) fn exact_for(rule: u64) -> Option<Arc<HashMap<Sig, [i64; 2]>>> {
    if !joins().exact.load(Relaxed) {
        return None;
    }
    joins().memo.read().exact.get(&rule).cloned()
}

fn noted_mut<'m>(memo: &'m mut Memo, key: PlanKey, rule: u64) -> &'m mut Note {
    joins().noted.store(true, Relaxed);
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
/// once flagged in the order it runs (`order`): another order of the line is checked again.
pub(super) fn check(key: PlanKey, work: f64, order: &[Step]) -> bool {
    if work < slow_rows() {
        return false;
    }
    let st_ = joins();
    let mut m = st_.memo.write();
    if m.flagged.get(&key).is_some_and(|o| *o == steps(order)) {
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
/// none once flagged in the order it runs (`order`).
pub(super) fn next_due(key: PlanKey, order: &[Step]) -> Option<f64> {
    let st_ = joins();
    let m = st_.memo.read();
    if m.flagged.get(&key).is_some_and(|o| *o == steps(order)) {
        return None;
    }
    Some(m.next_check.get(&key).copied().unwrap_or(0.0).max(slow_rows()))
}

/// Decision 0206: a slow order replaced at its cut. `old`, its order; `per_row`, its work per delta row finished;
/// `limit`, the work its replacement may do over the rows left before the two are compared.
pub(in crate::code_rule_dsl::logic) struct Handoff { pub old: Vec<Step>, pub per_row: f64, pub limit: f64 }

/// Decision 0271: whether a slow seed's or whole plan's orders are measured after its check, by a trial (`sampled`: its
/// sample has `TRIAL_LEAST` rows, and decision 0274's trial is due) or by the handoff it is in. Where not, it is
/// planned again at the check.
pub(super) fn measured_below(sampled: bool, in_handoff: bool) -> bool {
    trials_on() && (sampled || in_handoff)
}

/// Decision 0271: the limit of the order that goes on at a replacement's cut, before the two are compared again: past
/// the work `base` it has done (none, for the old order placed back), `slow_factor` times its work per row `rate`
/// over the `left` rows left. An order that has done no row (`rate` infinite) may double the work, the larger of what
/// it has done and the cut's `work`, before the next comparison: a replacement placed starts its counts again (`base`
/// 0), and `base` plus the larger of the two gave it its cut's work again: the limit it was just cut at, so a row
/// costing more than that was cut with none done at each limit for ever. Always finite: no order goes on unlimited.
pub(super) fn limit_again(rate: f64, base: f64, left: f64, work: f64) -> f64 {
    if rate.is_finite() { base + slow_factor() * rate * left.max(1.0) } else { 2.0 * base.max(work).max(1.0) }
}

/// A handoff made, or ended with the old order going on (`back`).
pub(super) fn count_handoff(back: bool) {
    run_counts().counts[if back { HANDBACKS } else { HANDOFFS }].fetch_add(1, Relaxed);
}

/// A line ran slow: `ran` rows of work where its order `order` was priced at `est`. Its next replan skips the
/// budget.
///
/// Q80: what the order measured is kept as an order tried, so that the line's next choice (`choose`) prices the
/// order at it. Its work over its price is a floor, as a cut trial's is (decision 0267): it was cut while it ran, and
/// it raises the order's kept ratio, never lowers it. Not kept, the next chunk's plan took the same order at the same
/// price (`pre_546` on hogql: priced 360 to 513 for 102 K rows of work, at each of its trips).
/// Whether line `key` is flagged in the order it runs (`flag_slow`): checked no more in it.
pub(super) fn flagged(key: PlanKey, order: &[Step]) -> bool {
    joins().memo.read().flagged.get(&key).is_some_and(|o| *o == steps(order))
}

/// How a running line's rows missed its order's estimate, step by step (`step_miss`).
#[derive(Clone, Debug, PartialEq)]
pub enum Miss {
    /// Its widest step made `ran` rows where the widest was priced at `est`: the expansion blew up.
    Wide { est: f64, ran: f64 },
    /// Step `at` made `ran` rows a binding entering it where it was priced at `est`: the estimate that chose the order
    /// was far too high there (an atom priced to multiply its bindings that cuts them, read late for it).
    Over { at: usize, est: f64, ran: f64 },
}

impl Miss {
    pub fn describe(&self) -> String {
        match self {
            Miss::Wide { est, ran } => format!("its widest step made {ran:.0} rows, priced {est:.0}: blew up"),
            Miss::Over { at, est, ran } => format!("step {at} made {ran:.3} rows a binding, priced {est:.3}: over-estimated"),
        }
    }
}


pub(super) fn count_miss(m: &Miss) {
    run_counts().misses[usize::from(matches!(m, Miss::Over { .. }))].fetch_add(1, Relaxed);
}

/// Bindings a step is entered with before its estimate is judged too high: under them its count is noise.
pub const OVER_LEAST_LEAD: f64 = 64.0;

/// A running line's miss against its order's estimate (`est`: the rows after each step, as the search priced the
/// order; `ran`: each step's bindings in and rows out so far), or `None`. Both are taken per row the lead step made,
/// so a line part way through its rows is judged as fairly as a finished one. Past `slow_factor` either way:
///
/// - **Wide:** the widest step made more than that times the widest priced, and more than a search's work. The
///   expansion is the cost a wrong estimate makes; the work check (`check`) sees it only once it is the whole
///   line's, and only against a price from the same statistics.
/// - **Over:** a step priced at that times a search's work or more made rows a binding under its price's share. That
///   is the step's own estimate, apart from the steps before it: an atom priced to multiply what it is entered with
///   that cuts it is read late, after the expansion it would have cut (the orders it ruled out were priced with it).
///
/// The steps must be the order's, one for one (the profile pairs them so); else `None`.
pub fn step_miss(est: &[f64], ran: &[(u64, u64)], search_work: f64) -> Option<Miss> {
    if est.is_empty() || est.len() != ran.len() {
        return None;
    }
    let lead = ran[0].1 as f64;
    if lead <= 0.0 {
        return None;
    }
    let f = slow_factor();
    let scale = lead / est[0].max(1e-9);
    let priced: Vec<f64> = est.iter().map(|e| e * scale).collect();
    let widest_est = priced.iter().copied().fold(0.0, f64::max);
    let widest_ran = ran.iter().map(|r| r.1 as f64).fold(0.0, f64::max);
    if widest_ran > f * widest_est.max(search_work) {
        return Some(Miss::Wide { est: widest_est, ran: widest_ran });
    }
    for at in 1..priced.len() {
        let (into, made) = (ran[at].0 as f64, ran[at].1 as f64);
        if into < OVER_LEAST_LEAD || priced[at] < f * search_work {
            continue;
        }
        let (est, got) = (priced[at] / priced[at - 1].max(1e-9), made / into);
        if est > f * got.max(1.0 / into) {
            return Some(Miss::Over { at, est, ran: got });
        }
    }
    None
}

pub(super) fn flag_slow(key: PlanKey, rule: u64, order: &[Step], est: f64, ran: f64) {
    run_counts().counts[FLAGS].fetch_add(1, Relaxed);
    let st_ = joins();
    let mut m = st_.memo.write();
    let order = steps(order);
    m.flagged.insert(key, order.clone());
    m.forced.insert(key);
    m.trialled.remove(&key);
    let n = noted_mut(&mut m, key, rule);
    n.slow += 1;
    n.est = est;
    n.ran = ran;
    n.wants_search = true;
    let ratio = (ran / est.max(1.0)).clamp(1e-6, 1e9);
    if ratio > 1.0 {
        match n.tried.iter_mut().find(|t| t.order == order) {
            Some(t) => t.ratio = t.ratio.max(ratio),
            None => merge_tried(n, order, ratio, RUN_WEIGHT),
        }
    }
}

/// Q80: the order to put in place of a slow line's running order, where planning it again with the exact statistics
/// found the order it runs. Of the search's runners-up `others` (each priced for the rows left), the least priced,
/// where its price is under what the running order measured: `ran` rows of work over its first `done` lead rows, at
/// that rate over the `left` rows left (an order that finished no row costs at least its work again). A replacement
/// runs under the handoff's limit (decision 0206), and where it is cut too, the order with less work a row goes on:
/// a runner-up priced as wrongly costs a limit's work. `None`: no runner-up is priced under the running order.
pub(super) fn over_measured(others: &[(Vec<Step>, f64)], ran: f64, done: u64, left: u64) -> Option<Vec<Step>> {
    let measured = if done > 0 { ran / done as f64 * left.max(1) as f64 } else { ran.max(1.0) };
    others.iter().filter(|(_, est)| est.is_finite() && *est < measured).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(o, _)| o.clone())
}

/// Whether the line's replan skips the planning budget, once.
pub(super) fn take_forced(key: &PlanKey) -> bool {
    joins().noted.load(Relaxed) && joins().memo.write().forced.remove(key)
}

/// `rule`'s exact pair statistics, measured for line `key`: the rule's plans are keyed again (`epoch`).
pub(super) fn put_exact(key: PlanKey, rule: u64, stats: Vec<(Sig, [i64; 2])>) {
    if stats.is_empty() {
        return;
    }
    run_counts().counts[MEASURED].fetch_add(stats.len() as u64, Relaxed);
    let st_ = joins();
    let mut m = st_.memo.write();
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
    joins().exact.store(true, Relaxed);
}

/// A wider search's orders for line `key`, best first: its proposals. `walked`: the walked work of the order the
/// line ran, where the search walked them (decision 0243).
pub(super) fn propose(key: PlanKey, mut orders: Vec<(Vec<StoredStep>, f64)>, walked: Option<f64>) {
    orders.sort_by(|a, b| a.1.total_cmp(&b.1));
    orders.dedup_by(|a, b| a.0 == b.0);
    orders.truncate(PROPOSALS);
    run_counts().counts[PROPOSED].fetch_add(orders.len() as u64, Relaxed);
    log(|| format!("proposals for line {:x}: {} orders, {} {:?}{}", key as u64, orders.len(), if walked.is_some() { "walked" } else { "prices" }, orders.iter().map(|o| format!("{} {:.0}", o.0.iter().map(|s| s.0.to_string()).collect::<Vec<_>>().join("."), o.1)).collect::<Vec<_>>(), walked.map_or(String::new(), |w| format!("; the order it ran walked {w:.0}"))));
    let st_ = joins();
    let mut m = st_.memo.write();
    let Some(n) = m.notes.get_mut(&key) else { return };
    n.proposals = orders;
    n.walked = walked.unwrap_or(0.0);
    n.wants_search = false;
    m.dirty = true;
}

/// A search's context (decision 0203), handed down to it and read back by its caller: whether it is wide (exact up
/// to `WIDE_GREEDY` joins, never the greedy order first), and its runners-up, best first.
#[derive(Default)]
pub(in crate::code_rule_dsl) struct SearchCx {
    pub wide: bool,
    pub runners_up: Vec<(Vec<Step>, f64)>,
    /// With a batch pricer (decision 0354): the best's and the runners-up's steps as the search priced them, each
    /// step's rows, basis, access and source from the search's own cells (no plan record made for them).
    pub steps_of: Vec<(Vec<Step>, Vec<super::super::profile::StepPlan>)>,
    /// An order to try first (decision 0258): taken where it runs, is led by the delta and prices under a search's
    /// cost, before greedy-first and the search.
    pub start: Option<Vec<Step>>,
    /// Whether `start` was taken.
    pub started: bool,
}

/// The greedy bound of a search, `wide` or not.
pub(super) fn greedy_limit(run: usize, wide: bool) -> usize {
    if wide { WIDE_GREEDY.max(run) } else { run }
}

/// An order a choice weighs, at its price now; with a batch pricer, its steps' features (decision 0354: each step
/// after the first, its rows in and out the statistics'; empty where they could not be read).
#[derive(Clone, Debug)]
pub struct Candidate {
    pub order: Vec<Step>,
    pub est: f64,
    pub steps: Vec<rerail_accel::join_model::StepSample>,
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

/// The machine's batch pricer, the learned model's (decision 0442: the machine's), and whether one is set.
#[derive(Default)]
struct Pricing(RwLock<Option<Arc<dyn Pricer>>>, AtomicBool);

fn pricer() -> Arc<Pricing> {
    super::super::profile::machine::<Pricing>()
}

pub fn set_pricer(p: Option<Arc<dyn Pricer>>) {
    let held = pricer();
    held.1.store(p.is_some(), Relaxed);
    *held.0.write() = p;
}


/// Whether a batch pricer is set: every searched line's choice is then made in its round.
pub(super) fn pricing() -> bool {
    pricer().1.load(Relaxed)
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
    // Nothing to choose between: the pricer is not asked (decision 0354: rounds of no choice were three in four).
    let choosing = round.iter().any(|c| c.candidates.len() >= 2);
    let st_ = pricer();
    let predicted = st_.0.read().clone().filter(|_| choosing).and_then(|p| {
        let _span = super::super::profile::span(|| "join model: prediction".into());
        p.price(round)
    });
    let held = joins();
    let memo = if held.noted.load(Relaxed) { Some(held.memo.read()) } else { None };
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
        let mut costs: Vec<f64> = (0..c.candidates.len()).map(corrected).collect();
        // Decision 0245: an order the background walked and no run measured is costed in the walks' units, against
        // the order the line ran: its walked work times that order's measured cost over its walked work. A walk's
        // ratio to a price made then, times a price made now, mixed two models' prices (on the fixture it took an
        // order walked 1.35 times the one that ran).
        if let Some(n) = note.filter(|n| n.walked > 0.0) {
            let walked_of = |i: usize| { let o = steps(&c.candidates[i].order); n.proposals.iter().find(|(p, _)| *p == o).map(|p| p.1) };
            let measured = |i: usize| n.tried(&steps(&c.candidates[i].order)).is_some();
            let ran = (0..c.candidates.len()).find(|&i| measured(i) && walked_of(i) == Some(n.walked));
            if let Some(ran) = ran {
                // Decision 0247: a walk varies by 20% to 30% (sd, 8 seeds on the data-layer files), so a walked
                // order is costed as if it walked `WALK_MARGIN` higher: it displaces a measured order only where
                // it walks about two deviations below (a walk 0.61 of the one that ran took a line from 5.66 M to
                // 6.89 M).
                let unit = costs[ran] / n.walked;
                for (i, cost) in costs.iter_mut().enumerate() {
                    if !measured(i) && let Some(w) = walked_of(i) { *cost = w * unit / WALK_MARGIN; }
                }
            }
        }
        let best = (0..costs.len()).min_by(|&a, &b| costs[a].total_cmp(&costs[b])).unwrap_or(0);
        if best != 0 {
            c.chosen = best;
            c.why = if note.is_some() { "corrected by what its orders measured" } else { "the batch prediction's least" };
            run_counts().counts[CORRECTED].fetch_add(1, Relaxed);
        }
        // Decision 0354: where the learned model chose, a share of its choices take another candidate within the
        // margin, so that the steps it learns from are not only the orders it picks.
        if note.is_none() && predicted.is_some() {
            let near: Vec<usize> = (0..costs.len()).filter(|&i| i != best && costs[i] <= margin() * costs[best].max(1.0)).collect();
            let mut rng = Rng::new(c.key as u64 ^ 0x6a6f_696e);
            if !near.is_empty() && costs[best] <= explore_rows() && rng.unit() < explore_rate() {
                c.chosen = near[rng.below(near.len())];
                c.why = "explored: a candidate within the margin of the model's least";
                run_counts().counts[EXPLORED].fetch_add(1, Relaxed);
            }
            continue;
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
            run_counts().counts[EXPLORED].fetch_add(1, Relaxed);
        }
    }
}

/// A round of choices (decision 0203 §2): the lines planned while it was open recorded their choices (each kept
/// the search's best, `Model::plan_rule_live_in`); chosen here all at once, after one batch prediction.
pub(super) fn finish_round(mut round: Vec<Choice>) -> Vec<Choice> {
    choose(&mut round);
    round
}

/// One line as it ran: its stratum, key, rule, the order it ran last, its measured work, its price for the rows it
/// read (0 where not priced), and the seconds its probes ran (decision 0208; 0 where not known).
#[derive(Clone, Debug, Default)]
pub struct LineRun {
    pub stratum: String,
    pub key: PlanKey,
    pub rule: u64,
    pub order: Vec<StoredStep>,
    pub work: f64,
    pub est: f64,
    pub wall: f64,
}

/// What `critical` found, for the profile.
#[derive(Debug, Default, PartialEq)]
pub struct Path {
    /// The strata on it, in order, and its work: the path the lines' work predicts.
    pub strata: Vec<usize>,
    pub work: f64,
    /// Its lines noted, with their shares (of the measured path where there is one).
    pub lines: Vec<(PlanKey, f64)>,
    /// The path as the strata ran (decision 0208).
    pub measured: Option<Measured>,
}

/// The run's measured critical path (decision 0208): from the stratum that ended last, back through what each
/// waited for. Its strata in order; the seconds they ran, and those spent between them (each stratum's wait for what
/// it waited for); from the first's start to the last's end; and how many are on the predicted path.
#[derive(Debug, Default, PartialEq)]
pub struct Measured {
    pub strata: Vec<usize>,
    pub busy: f64,
    pub waited: f64,
    pub span: f64,
    pub shared: usize,
}

/// Decision 0208: the measured path over `timeline` (each stratum's start, end and where it ran: 0 the loop's
/// thread). From the stratum that ended last, each step back is to what the stratum could not start before: the
/// latest to end, before its start, of its producers and (on the loop's thread) the stratum the loop ran before it.
pub fn measured_path(producers: &[Vec<usize>], timeline: &[Option<(f64, f64, u8)>]) -> Option<Measured> {
    const EPS: f64 = 1e-6;
    let end = (0..timeline.len()).filter(|&j| timeline[j].is_some()).max_by(|&a, &b| timeline[a].unwrap().1.total_cmp(&timeline[b].unwrap().1))?;
    let mut path = vec![end];
    let (mut busy, mut waited) = (0.0, 0.0);
    let mut at = end;
    loop {
        let (start, stop, place) = timeline[at].expect("a stratum that ran");
        busy += stop - start;
        let before = |j: usize| timeline[j].filter(|t| t.1 <= start + EPS);
        let by_data = producers.get(at).into_iter().flatten().copied().filter(|&p| p != at && before(p).is_some());
        // The loop's thread runs its strata one after another: the one before on that thread, too.
        let by_thread = (place == 0).then(|| (0..timeline.len()).filter(|&j| j != at && timeline[j].is_some_and(|t| t.2 == 0) && before(j).is_some()).max_by(|&a, &b| timeline[a].unwrap().1.total_cmp(&timeline[b].unwrap().1))).flatten();
        let Some(prev) = by_data.chain(by_thread).filter(|p| !path.contains(p)).max_by(|&a, &b| timeline[a].unwrap().1.total_cmp(&timeline[b].unwrap().1)) else { break };
        waited += (start - timeline[prev].unwrap().1).max(0.0);
        path.push(prev);
        at = prev;
    }
    path.reverse();
    let span = timeline[end].unwrap().1 - timeline[path[0]].unwrap().0;
    Some(Measured { strata: path, busy, waited, span, shared: 0 })
}

/// `critical_timed` with no times: the predicted path notes the lines.
pub fn critical(strata: &[String], producers: &[Vec<usize>], runs: &[LineRun]) -> Path {
    critical_timed(strata, producers, runs, None)
}

/// The run's critical path (decision 0203 §4): strata `producers` (each stratum's, by index; strata in dependency
/// order, as a run takes them), weighted by the work of their lines `runs`. The longest chain is the path. Each of
/// its lines with `critical_share` of it is noted: its share, its order tried, and a wider search asked for while
/// its order measured over twice its price or none was made. Lines noted before have their order recorded too.
///
/// Decision 0208: where the strata's times are given, the path as they ran (`measured_path`) notes the lines
/// instead: a line's share is its seconds over the path's busy seconds (its work over the path's, where its seconds
/// are not known).
pub fn critical_timed(strata: &[String], producers: &[Vec<usize>], runs: &[LineRun], timeline: Option<&[Option<(f64, f64, u8)>]>) -> Path {
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
    let mut measured = timeline.and_then(|t| measured_path(producers, t));
    if let Some(m) = &mut measured {
        m.shared = m.strata.iter().filter(|j| path.contains(j)).count();
    }
    // The path that notes: the measured one where there is one, by seconds where they are known.
    let on: FxHashSet<usize> = measured.as_ref().map_or(&path, |m| &m.strata).iter().copied().collect();
    let on_path = |r: &LineRun| at.get(r.stratum.as_str()).is_some_and(|j| on.contains(j));
    let walls: f64 = runs.iter().filter(|r| on_path(r)).map(|r| r.wall).sum();
    let works: f64 = runs.iter().filter(|r| on_path(r)).map(|r| r.work).sum();
    let by_wall = measured.is_some() && walls > 0.0;
    let mut lines = Vec::new();
    if total <= 0.0 {
        return Path { strata: path, work: total, lines, measured };
    }
    let st_ = joins();
    let mut m = st_.memo.write();
    for r in runs {
        let key = r.key;
        let share = if !on_path(r) { 0.0 } else if by_wall { r.wall / walls } else if measured.is_some() { r.work / works.max(1.0) } else { r.work / total };
        let critical = share >= critical_share() && r.work >= slow_rows().min(1e4);
        if !critical && !m.notes.contains_key(&key) {
            continue;
        }
        let n = noted_mut(&mut m, key, r.rule);
        n.runs += 1;
        n.critical = if n.runs == 1 { share } else { 0.5 * n.critical + 0.5 * share };
        n.ran = r.work;
        if r.est > 0.0 {
            n.est = r.est;
            let ratio = (r.work.max(1.0) / r.est.max(1.0)).clamp(1e-6, 1e9);
            merge_tried(n, r.order.clone(), ratio, RUN_WEIGHT);
            if critical && (ratio > 2.0 || n.proposals.is_empty()) {
                n.wants_search = true;
            }
        }
        if critical {
            lines.push((key, share));
        }
    }
    drop(m);
    prioritize(&lines, total);
    Path { strata: path, work: total, lines, measured }
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

/// Decision 0247: how far below the order that ran a walked order must walk to be taken on its walk alone.
pub(super) const WALK_MARGIN: f64 = 0.625;

/// A wider search's result: its orders with their prices, or their walked work over the line, and the walked work of
/// the order the line ran where it walked them (decision 0243).
pub(super) struct Searched {
    pub orders: Vec<(Vec<StoredStep>, f64)>,
    pub walked: Option<f64>,
}

/// Wider searches waiting for the upkeep job, most important first: each a line's key, its weight, whether it holds
/// the line's relations to walk (decision 0243), and the search. The search is given whether its time is up: it
/// walks no further order once it is (decision 0245), and returns what it has.
type Search = Box<dyn FnOnce(&dyn Fn() -> bool) -> Searched + Send>;
/// Requests waiting at most (the least important are dropped).
const REQUESTS_MAX: usize = 64;
/// Decision 0243: requests that hold their line's relations, at most. A held relation stays alive until its search
/// runs, after the engine has moved on; the rest are priced.
const HOLDING_MAX: usize = 4;

/// Whether a new request may hold its line's relations to walk them (`HOLDING_MAX`).
pub(super) fn may_hold() -> bool {
    joins().requests.lock().iter().filter(|r| r.2).count() < HOLDING_MAX
}

/// Asks for a wider search of line `key` (`search`), run by the `join-search` upkeep job at idle.
pub(super) fn request(key: PlanKey, weight: f64, holds: bool, search: Search) {
    job();
    let st_ = joins();
    let mut q = st_.requests.lock();
    if q.iter().any(|r| r.0 == key) {
        return;
    }
    q.push((key, weight, holds, search));
    q.sort_by(|a, b| b.1.total_cmp(&a.1));
    q.truncate(REQUESTS_MAX);
}

/// Whether a wider search of line `key` is worth asking for: none waits for it, and it has no proposals yet.
pub(super) fn wants_request(key: &PlanKey) -> bool {
    let key = key;
    if joins().requests.lock().iter().any(|r| &r.0 == key) {
        return false;
    }
    !(joins().noted.load(Relaxed) && joins().memo.read().notes.get(key).is_some_and(|n| !n.proposals.is_empty() && !n.wants_search))
}

/// Raises the waiting searches of the critical path's lines to its work: they are searched first.
fn prioritize(lines: &[(PlanKey, f64)], total: f64) {
    let st_ = joins();
    let mut q = st_.requests.lock();
    for (k, w, _, _) in q.iter_mut() {
        if let Some((_, share)) = lines.iter().find(|(l, _)| l == k) {
            *w = w.max(share * total) + total;
        }
    }
    q.sort_by(|a, b| b.1.total_cmp(&a.1));
}

/// Requests waiting, for tests and the profile.
pub fn requests() -> usize {
    joins().requests.lock().len()
}

/// Runs the most important waiting search now, on this thread, until `over` says its time is up: `false` where none
/// waits.
pub fn search_one(over: &dyn Fn() -> bool) -> bool {
    let Some((key, _, _, search)) = ({
        let st_ = joins();
        let mut q = st_.requests.lock();
        (!q.is_empty()).then(|| q.remove(0))
    }) else {
        return false;
    };
    let found = search(over);
    propose(key, found.orders, found.walked);
    true
}

/// Decision 0243: whether a run's end serves the waiting searches (`drain_at_end`): a one-shot process ends before the
/// machine is idle, and its requests, with the relations they hold, end with it. Decision 0272: every command of the
/// CLI (`tools::cli`) sets it; a long-lived caller (the reactor) leaves the searches to the idle job. The caller's
/// (decision 0442), in the run's context, so its runs' jobs see it too.
#[derive(Clone, Copy, Default)]
struct Drain(bool);
/// The share of the process's CPU a run's end may spend on them.
const DRAIN_SHARE: f64 = 0.05;

/// Sets whether the ends of this caller's runs serve the waiting searches.
pub fn drain_at_end(on: bool) {
    crate::code_rule_dsl::logic::profile::bind_here(Drain(on));
}

/// At a run's end, where `drain_at_end` is set: the waiting searches, most important first, until this thread has
/// spent `DRAIN_SHARE` of the process's CPU so far on them. The searches the budget leaves wait, as before.
pub(super) fn drain() {
    if !crate::code_rule_dsl::logic::profile::bound::<Drain>().0 || joins().requests.lock().is_empty() {
        return;
    }
    let budget = DRAIN_SHARE * super::super::profile::process_cpu();
    let start = super::super::native::component::thread_cpu();
    // Decision 0245: each search stops walking at the budget, not only between searches.
    let over = || super::super::native::component::thread_cpu() - start >= budget;
    let mut served = 0;
    while !over() && search_one(&over) {
        served += 1;
    }
    let spent = super::super::native::component::thread_cpu() - start;
    log(|| format!("searches at the run's end: {served} served in {:.1} ms CPU (budget {:.1} ms), {} wait", spent * 1e3, budget * 1e3, requests()));
}

/// The searches of the shared object that registered it: its step runs on that object.
struct JoinSearch(Arc<crate::code_rule_dsl::logic::profile::Shared>);

impl rerail_accel::upkeep::Job for JoinSearch {
    fn name(&self) -> String {
        "join-search".into()
    }
    fn due(&mut self) -> rerail_accel::upkeep::Due {
        let _in_run = crate::code_rule_dsl::logic::profile::within(Arc::clone(&self.0));
        if joins().requests.lock().is_empty() {
            rerail_accel::upkeep::Due::No
        } else {
            rerail_accel::upkeep::Due::Idle { starve: std::time::Duration::from_secs(300) }
        }
    }
    fn step(&mut self, slice: &rerail_accel::upkeep::Slice) -> Result<rerail_accel::upkeep::Step, String> {
        let _in_run = crate::code_rule_dsl::logic::profile::within(Arc::clone(&self.0));
        while !slice.over() {
            if !search_one(&|| slice.over()) {
                return Ok(rerail_accel::upkeep::Step::Done);
            }
        }
        Ok(rerail_accel::upkeep::Step::More)
    }
}

/// The job, registered once per shared object when its first search is asked for.
fn job() {
    joins().job.get_or_init(|| Mutex::new(Some(rerail_accel::upkeep::register(Box::new(JoinSearch(crate::code_rule_dsl::logic::profile::shared_here()))))));
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
    // Decision 0243, last: a row stored before it ends here, and reads as priced.
    row.push(float(n.walked));
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
    n.walked = next().map_or(0.0, unfloat);
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

/// The memo's rows for the store: its notes, their exact statistics, and the columns they raised.
pub type Stored = (Vec<(PlanKey, Note)>, Vec<(PlanKey, u64, Sig, [i64; 2])>, Vec<(PlanKey, String, usize)>);

/// The memo's notes and exact statistics, for the store, where any changed since the last call.
pub fn to_store() -> Option<Stored> {
    let st_ = joins();
    let mut m = st_.memo.write();
    if !std::mem::take(&mut m.dirty) {
        return None;
    }
    let now: FxHashMap<PlanKey, u64> = m.notes.iter().map(|(k, n)| (*k, fingerprint(n))).collect();
    if now == m.written {
        return None;
    }
    m.written = now;
    let notes: Vec<(PlanKey, Note)> = m.notes.iter().map(|(k, n)| (*k, n.clone())).collect();
    let exact = notes.iter().flat_map(|(k, n)| n.exact.iter().map(move |(s, c)| (*k, n.rule, s.clone(), *c))).collect();
    let raised = notes.iter().flat_map(|(k, n)| n.raised.iter().map(move |(r, c)| (*k, r.clone(), *c))).collect();
    Some((notes, exact, raised))
}

/// Takes the store's notes: a line this process noted already keeps its own (memory is ahead of the store).
pub fn from_store((notes, exact, raised): Stored) {
    if notes.is_empty() {
        return;
    }
    let st_ = joins();
    let mut m = st_.memo.write();
    let mut taken: FxHashSet<PlanKey> = FxHashSet::default();
    for (k, n) in notes {
        m.notes.entry(k).or_insert_with(|| {
            taken.insert(k);
            n
        });
    }
    for (k, rule, sig, c) in exact {
        if taken.contains(&k)
            && let Some(n) = m.notes.get_mut(&k)
        {
            n.exact.push((sig, c));
            let _ = rule;
        }
    }
    for (k, rel, c) in raised {
        if taken.contains(&k)
            && let Some(n) = m.notes.get_mut(&k)
        {
            n.raised.push((rel, c));
        }
    }
    // The columns every note raised: made from a stratum's first plan (`Run::heavy_for`).
    let all: Vec<(String, usize)> = m.notes.values().flat_map(|n| n.raised.iter().cloned()).collect();
    if !all.is_empty() {
        joins().raised.store(true, Relaxed);
        m.raised.extend(all);
    }
    // As the store holds them: a run that changes none of them writes none.
    for k in &taken {
        let f = fingerprint(&m.notes[k]);
        m.written.insert(*k, f);
    }
    let rules: Vec<(u64, HashMap<Sig, [i64; 2]>)> = m.notes.values().filter(|n| !n.exact.is_empty()).map(|n| (n.rule, n.exact.iter().cloned().collect())).collect();
    for (rule, map) in rules {
        m.epochs.entry(rule).or_insert(1);
        m.exact.insert(rule, Arc::new(map));
        joins().exact.store(true, Relaxed);
    }
    joins().noted.store(!m.notes.is_empty(), Relaxed);
}

#[cfg(test)]
mod tests;

/// Decision 0232: the bindings a key's walk carries through each step (`RERAIL_WALK_WIDTH`, 64).
pub fn walk_width() -> usize {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var("RERAIL_WALK_WIDTH").ok().and_then(|v| v.parse().ok()).unwrap_or(64))
}

/// Decision 0233: the rows of each part a line's first-phase walk samples (`RERAIL_WALK_ROWS`, 128).
pub fn walk_rows() -> usize {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var("RERAIL_WALK_ROWS").ok().and_then(|v| v.parse().ok()).unwrap_or(128))
}

/// The run's counts of this module's work (decision 0442: a run's own, `run::local`).
#[derive(Default)]
pub struct JoinCounts {
    /// Counts for the profile and tests: flags, exact statistics measured, choices corrected, explored, proposals,
    /// handoffs and handbacks (decision 0206), orders trialled (0206 §2).
    pub counts: [AtomicU64; 8],
    /// Lines found mispredicted step by step in this process: blown up, over-estimated (`RERAIL_PLAN_CACHE_COUNTS`).
    pub misses: [AtomicU64; 2],
}

/// The run's `JoinCounts`.
#[inline]
pub fn run_counts() -> std::sync::Arc<JoinCounts> {
    crate::code_rule_dsl::logic::run::local::<JoinCounts>()
}
