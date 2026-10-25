//! The per-file match budget. `MatchBudget::work` (or `until`) installs it on the evaluating
//! thread, where the matcher, the anchor index, regex scans and cross-file clauses poll it. Every
//! poll reads one shared `MatchDeadline`.
//!
//! A file's and a global clause's budget is work, not time: each poll spends one unit of a fixed
//! allowance, and every poll site polls once per bounded amount of work (every 1,024 matcher
//! calls, every 256 steps of a walk, posting build or anchor selection, every 1,024 cross-file
//! rows, once per regex match), so whether an evaluation runs out depends on its input alone, not
//! on the machine's load. Only the inline fast path, whose miss falls back to a background
//! evaluation, keeps a wall-clock deadline.
//!
//! A file's budget covers the work every clause shares (parsing, the tape, the anchor index);
//! each clause then runs under an allowance of its own (`with_allowance`), so one clause's
//! matching never spends another's. Lazily built per-file tables (scope tables, PHP names, anchor
//! text) are the exception: the first clause that needs one builds it under its own allowance,
//! and the clauses after it read it for free, so the work a clause spends depends on the clauses
//! run before it in the same pass. Over all of a file's clauses, from nothing built, that is fixed
//! by the content and the catalog (`workspace::evaluate` stores only such passes' exhaustion). A
//! ceiling on the allowances a file's clauses spend together bounds its total work
//! (`workspace::ClauseWork`). Running out of work is the input's verdict; only the workspace
//! closing or a wall-clock deadline (`MatchBudget::interrupted`) cuts an evaluation short in a
//! way that is not.

use std::cell::{Cell, RefCell};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Matcher calls between two polls; a budget is noticed spent at most this many calls late.
/// `CLAUSE_WORK_BUDGET` is calibrated at this interval.
const BUDGET_CHECK_INTERVAL: u32 = 1024;

/// Steps of a loop between two polls, for `step_budget_exhausted`.
const STEP_CHECK_INTERVAL: u32 = 256;

thread_local! {
    static MATCH_BUDGET: RefCell<Option<MatchDeadline>> = const { RefCell::new(None) };
}

thread_local! {
    /// Matcher calls until the next poll; reset when a budget starts, so an evaluation polls at
    /// the same calls whatever the thread evaluated before.
    static BUDGET_COUNTDOWN: Cell<u32> = const { Cell::new(0) };
    /// Loop steps until the next poll (`step_budget_exhausted`); reset as `BUDGET_COUNTDOWN` is.
    static STEP_COUNTDOWN: Cell<u32> = const { Cell::new(0) };
}

#[cfg(test)]
thread_local! {
    /// Tests turn off, on their own thread, the shortcuts that only skip work (the anchor-set
    /// shortcuts), to check they change no result.
    static SHORTCUTS_OFF: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Compiles and evaluates on this thread without the work-skipping shortcuts while `off`.
#[cfg(test)]
pub(super) fn set_shortcuts_off(off: bool) {
    SHORTCUTS_OFF.set(off);
}

/// Whether the test oracle's work-skipping shortcuts run.
#[cfg(test)]
pub(super) fn shortcuts_on() -> bool {
    !SHORTCUTS_OFF.get()
}

/// Whether this thread's budget is exhausted (never, without one). Callers that poll in a loop
/// throttle themselves (every 256 steps); per-matcher-call polling uses `matcher_call_budget_exhausted`.
pub(super) fn matcher_budget_exhausted() -> bool {
    MATCH_BUDGET.with(|budget| {
        budget
            .borrow()
            .as_ref()
            .is_some_and(MatchDeadline::exhausted)
    })
}

/// Whether some poll found this thread's budget exhausted; never polls. What decides, after work
/// ends, whether it was cut short: work that finished unpolled is complete.
pub(super) fn matcher_budget_observed() -> bool {
    MATCH_BUDGET.with(|budget| {
        budget
            .borrow()
            .as_ref()
            .is_some_and(MatchDeadline::observed)
    })
}

/// Spends this thread's budget without polling it: what a last step that runs past the budget and
/// never polls leaves behind.
#[cfg(test)]
pub(super) fn pass_deadline_unpolled() {
    MATCH_BUDGET.with(|budget| {
        if let Some(deadline) = budget.borrow_mut().as_mut() {
            deadline.limit = match deadline.limit {
                Limit::Work(_) => Limit::Work(0),
                Limit::Until(_) => Limit::Until(Instant::now()),
            };
        }
    });
}

/// `matcher_budget_exhausted` for matcher entry points: polls every `BUDGET_CHECK_INTERVAL`
/// calls, so a spent budget is noticed at most that many calls late.
pub(super) fn matcher_call_budget_exhausted() -> bool {
    let countdown = BUDGET_COUNTDOWN.get();
    if countdown > 0 {
        BUDGET_COUNTDOWN.set(countdown - 1);
        return false;
    }
    if matcher_budget_exhausted() {
        return true;
    }
    BUDGET_COUNTDOWN.set(BUDGET_CHECK_INTERVAL);
    false
}

/// `matcher_budget_exhausted` for one step of a loop: polls every `STEP_CHECK_INTERVAL` steps
/// counted across calls, so many short loops spend what one long loop of their steps does.
pub(super) fn step_budget_exhausted() -> bool {
    let countdown = STEP_COUNTDOWN.get();
    if countdown > 0 {
        STEP_COUNTDOWN.set(countdown - 1);
        return false;
    }
    if matcher_budget_exhausted() {
        return true;
    }
    STEP_COUNTDOWN.set(STEP_CHECK_INTERVAL - 1);
    false
}

/// What an allowance run inside this thread's budget used (`with_allowance`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Allowance {
    /// Whether a poll found the allowance exhausted (or the workspace closed, or the clock passed).
    pub(super) exhausted: bool,
    /// The units its polls spent, at most its size.
    pub(super) spent: u64,
}

/// Runs `run` under an allowance of `units` polls of its own, inside this thread's budget: the
/// allowance is spent and runs out on its own, while the workspace's closing and a wall-clock
/// deadline still end it. Under a wall-clock budget (the inline fast path) the allowance is that
/// deadline alone. Returns `run`'s result and what the allowance used. Without a budget, `run`
/// runs unbounded.
pub(super) fn with_allowance<T>(units: u64, run: impl FnOnce() -> T) -> (T, Allowance) {
    let Some(outer) = MATCH_BUDGET.with_borrow(Option::clone) else {
        return (
            run(),
            Allowance {
                exhausted: false,
                spent: 0,
            },
        );
    };
    let limit = match outer.limit {
        Limit::Work(_) => Limit::Work(units),
        until @ Limit::Until(_) => until,
    };
    let allowance = MatchDeadline {
        nested: Arc::clone(&outer.nested),
        interrupted_seen: Arc::clone(&outer.interrupted_seen),
        ..MatchDeadline::new(&outer.closed, limit)
    };
    /// Reinstates the enclosing budget and its poll countdowns, however `run` ends.
    struct Restore(Option<MatchDeadline>, u32, u32);
    impl Drop for Restore {
        fn drop(&mut self) {
            MATCH_BUDGET.set(self.0.take());
            BUDGET_COUNTDOWN.set(self.1);
            STEP_COUNTDOWN.set(self.2);
        }
    }
    let _restore = Restore(
        MATCH_BUDGET.replace(Some(allowance.clone())),
        BUDGET_COUNTDOWN.replace(0),
        STEP_COUNTDOWN.replace(0),
    );
    let result = run();
    let exhausted = allowance.observed();
    if exhausted {
        allowance.nested.store(true, Ordering::Relaxed);
    }
    let spent = allowance.work.load(Ordering::Relaxed).min(units);
    (result, Allowance { exhausted, spent })
}

/// What a budget allows.
#[derive(Clone, Copy)]
enum Limit {
    /// Polls: each spends one unit of this many.
    Work(u64),
    /// A wall-clock deadline.
    Until(Instant),
}

/// One evaluation's budget, shared by every clone: once a poll finds it spent (or the workspace
/// closed), it stays exhausted for all of them.
#[derive(Clone)]
pub(super) struct MatchDeadline {
    limit: Limit,
    /// Units the polls spent, shared by every clone.
    work: Arc<AtomicU64>,
    closed: Arc<AtomicBool>,
    spent: Arc<AtomicBool>,
    /// Whether an allowance run inside this budget (`with_allowance`) ran out; shared with them.
    nested: Arc<AtomicBool>,
    /// Whether a poll of this budget or an allowance inside it found the workspace closed or the
    /// wall-clock deadline passed (`MatchBudget::interrupted`); shared with them.
    interrupted_seen: Arc<AtomicBool>,
    /// Writes of `spent`, shared by every clone.
    #[cfg(test)]
    spent_stores: Arc<std::sync::atomic::AtomicUsize>,
}

impl MatchDeadline {
    fn new(closed: &Arc<AtomicBool>, limit: Limit) -> Self {
        Self {
            limit,
            work: Arc::default(),
            closed: Arc::clone(closed),
            spent: Arc::new(AtomicBool::new(false)),
            nested: Arc::default(),
            interrupted_seen: Arc::default(),
            #[cfg(test)]
            spent_stores: Arc::default(),
        }
    }

    /// Spends a unit of work (or reads the clock) unless exhaustion was already observed, and
    /// records exhaustion when it is, and whether closing or the clock caused it.
    pub(super) fn exhausted(&self) -> bool {
        if self.observed() {
            return true;
        }
        let closed = self.closed.load(Ordering::Relaxed);
        let open = !closed
            && match self.limit {
                Limit::Work(limit) => self.work.fetch_add(1, Ordering::Relaxed) < limit,
                Limit::Until(deadline) => Instant::now() < deadline,
            };
        if open {
            return false;
        }
        if closed || matches!(self.limit, Limit::Until(_)) {
            self.interrupted_seen.store(true, Ordering::Relaxed);
        }
        self.spent.store(true, Ordering::Relaxed);
        #[cfg(test)]
        self.spent_stores.fetch_add(1, Ordering::Relaxed);
        true
    }

    /// Whether some poll found the budget exhausted; never polls.
    pub(super) fn observed(&self) -> bool {
        self.spent.load(Ordering::Relaxed)
    }
}

/// An evaluation's suspended budget. Resuming restores its exact poll countdowns.
pub(super) struct Context {
    deadline: Option<MatchDeadline>,
    matcher: u32,
    steps: u32,
}
impl Context {
    pub(super) fn capture() -> Self {
        Self {
            deadline: MATCH_BUDGET.with_borrow(Clone::clone),
            matcher: BUDGET_COUNTDOWN.get(),
            steps: STEP_COUNTDOWN.get(),
        }
    }
    pub(super) fn work(closed: &Arc<AtomicBool>, units: u64) -> Self {
        Self {
            deadline: Some(MatchDeadline::new(closed, Limit::Work(units))),
            matcher: 0,
            steps: 0,
        }
    }
    pub(super) fn observed(&self) -> bool {
        self.deadline
            .as_ref()
            .is_some_and(|d| d.observed() || d.nested.load(Ordering::Relaxed))
    }
    pub(super) fn run<T>(&mut self, run: impl FnOnce() -> T) -> T {
        let old = Self::capture();
        MATCH_BUDGET.set(self.deadline.clone());
        BUDGET_COUNTDOWN.set(self.matcher);
        STEP_COUNTDOWN.set(self.steps);
        struct Restore<'a>(&'a mut Context, Context);
        impl Drop for Restore<'_> {
            fn drop(&mut self) {
                self.0.matcher = BUDGET_COUNTDOWN.get();
                self.0.steps = STEP_COUNTDOWN.get();
                MATCH_BUDGET.set(self.1.deadline.clone());
                BUDGET_COUNTDOWN.set(self.1.matcher);
                STEP_COUNTDOWN.set(self.1.steps);
            }
        }
        let _restore = Restore(self, old);
        run()
    }
}

pub(super) struct MatchBudget {
    deadline: MatchDeadline,
}
impl MatchBudget {
    /// A budget of `units` polls: the same work runs out at the same poll, on any machine and load.
    pub(super) fn work(closed: &Arc<AtomicBool>, units: u64) -> Self {
        Self::install(MatchDeadline::new(closed, Limit::Work(units)))
    }

    pub(super) fn resume(context: Context) -> Self {
        let deadline = context.deadline.expect("prepared file budget");
        MATCH_BUDGET.set(Some(deadline.clone()));
        BUDGET_COUNTDOWN.set(context.matcher);
        STEP_COUNTDOWN.set(context.steps);
        Self { deadline }
    }

    /// A wall-clock budget, for a fast path whose miss only falls back.
    pub(super) fn until(closed: &Arc<AtomicBool>, duration: Duration) -> Self {
        Self::install(MatchDeadline::new(
            closed,
            Limit::Until(Instant::now() + duration),
        ))
    }

    fn install(deadline: MatchDeadline) -> Self {
        MATCH_BUDGET.with(|budget| *budget.borrow_mut() = Some(deadline.clone()));
        BUDGET_COUNTDOWN.set(0);
        STEP_COUNTDOWN.set(0);
        Self { deadline }
    }

    /// Whether a poll found the budget, or an allowance run inside it, exhausted: an evaluation
    /// that finished without one is complete.
    pub(super) fn observed(&self) -> bool {
        self.deadline.observed() || self.deadline.nested.load(Ordering::Relaxed)
    }

    /// Whether a poll found the workspace closed or the wall-clock deadline passed: what cut the
    /// evaluation short depends on when it ran, not on the file, so it is not the file's verdict.
    pub(super) fn interrupted(&self) -> bool {
        self.deadline.interrupted_seen.load(Ordering::Relaxed)
    }

    /// Whether the file's own budget, which its shared work spends (parsing, the tape, the anchor
    /// index, the call summary), was found spent; an allowance running out inside it is not. How
    /// much shared work an evaluation does depends on what an earlier one stored (a stored tape
    /// skips the parse), so running out of it is not the file's verdict either.
    pub(super) fn shared_spent(&self) -> bool {
        self.deadline.observed()
    }

    #[cfg(test)]
    /// Units the polls have spent so far.
    pub(super) fn spent(&self) -> u64 {
        self.deadline.work.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    pub(super) fn expired_for_test() -> Self {
        Self::work(&Arc::new(AtomicBool::new(false)), 0)
    }
}
impl Drop for MatchBudget {
    fn drop(&mut self) {
        MATCH_BUDGET.with(|budget| *budget.borrow_mut() = None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suspended_context_preserves_poll_countdowns_and_cancellation() {
        let closed = Arc::new(AtomicBool::new(false));
        let mut first = Context::work(&closed, 1);
        let mut second = Context::work(&closed, 1);
        assert!(!first.run(step_budget_exhausted));
        assert!(!second.run(step_budget_exhausted));
        for _ in 0..STEP_CHECK_INTERVAL - 1 {
            assert!(!first.run(step_budget_exhausted));
        }
        assert!(first.run(step_budget_exhausted));
        assert!(first.observed());
        assert!(!second.observed());
        closed.store(true, Ordering::Relaxed);
        assert!(second.run(matcher_budget_exhausted));
        assert!(second.observed());
    }

    #[test]
    fn a_deadline_records_spent_only_when_it_becomes_exhausted() {
        let closed = Arc::new(AtomicBool::new(false));
        let deadline = MatchDeadline::new(
            &closed,
            Limit::Until(Instant::now() + Duration::from_secs(3_600)),
        );
        let worker = deadline.clone();
        for _ in 0..3 {
            assert!(!deadline.exhausted());
            assert!(!worker.exhausted());
        }
        assert_eq!(
            deadline.spent_stores.load(Ordering::Relaxed),
            0,
            "an open deadline is polled without writing the shared flag"
        );
        closed.store(true, Ordering::Relaxed);
        for _ in 0..3 {
            assert!(deadline.exhausted());
            assert!(worker.exhausted());
        }
        assert_eq!(
            deadline.spent_stores.load(Ordering::Relaxed),
            1,
            "exhaustion is recorded once, when it is first observed"
        );
    }

    #[test]
    fn a_work_budget_runs_out_at_the_same_poll_across_clones_however_long_it_takes() {
        let closed = Arc::new(AtomicBool::new(false));
        let budget = MatchDeadline::new(&closed, Limit::Work(5));
        let terminal = budget.clone();
        for poll in 0..5 {
            let polled = if poll % 2 == 0 { &budget } else { &terminal };
            assert!(!polled.exhausted(), "poll {poll} is within the budget");
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(
            terminal.exhausted(),
            "the sixth poll, on either clone, spends past it"
        );
        assert!(budget.observed() && budget.exhausted());
    }

    #[test]
    fn an_allowance_runs_out_on_its_own_and_leaves_the_file_budget_unspent() {
        let closed = Arc::new(AtomicBool::new(false));
        let file = MatchBudget::work(&closed, 3);
        let ((), costly) = with_allowance(10, || while !matcher_budget_exhausted() {});
        assert_eq!(
            costly,
            Allowance {
                exhausted: true,
                spent: 10
            },
            "the costly allowance ran out, having spent all of it"
        );
        assert_eq!(file.spent(), 0, "an allowance's polls are its own");
        assert!(
            !matcher_budget_observed(),
            "the file budget is reinstated unspent"
        );
        let ((), cheap) = with_allowance(10, || {
            for _ in 0..5 {
                assert!(!matcher_budget_exhausted());
            }
        });
        assert_eq!(
            cheap,
            Allowance {
                exhausted: false,
                spent: 5
            },
            "a later allowance starts whole"
        );
        assert!(
            file.observed(),
            "the file reports that one of its allowances ran out"
        );
        assert!(
            !file.interrupted(),
            "running out of work is no interruption"
        );
        closed.store(true, Ordering::Relaxed);
        let (_, after_close) = with_allowance(10, matcher_budget_exhausted);
        assert!(
            after_close.exhausted,
            "closing the workspace ends every allowance"
        );
        assert!(file.interrupted(), "and the file sees it interrupted");
    }

    #[test]
    fn step_polls_count_steps_across_calls() {
        let budget = MatchBudget::work(&Arc::new(AtomicBool::new(false)), u64::MAX);
        for _ in 0..1_000 {
            // A loop of one step per call, as a per-root helper runs.
            assert!(!step_budget_exhausted());
        }
        assert_eq!(
            budget.spent(),
            4,
            "1,000 steps poll once per 256, not once per call"
        );
    }
}
