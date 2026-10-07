//! The per-file match budget. `MatchBudget::work` (or `until`) installs it on the evaluating
//! thread, where the matcher, the anchor index, regex scans and cross-file checks poll it. Every
//! poll reads one shared `MatchDeadline`.
//!
//! A file's and a global check's budget is work, not time: each poll spends one unit of a fixed
//! allowance, and every poll site polls once per bounded amount of work (every 1,024 matcher
//! calls, every 256 nodes of a walk or posting build, every 1,024 cross-file rows, once per regex
//! match), so whether an evaluation runs out depends on its input alone, not on the machine's
//! load. Only the inline fast path, whose miss falls back to a background evaluation, keeps a
//! wall-clock deadline.

use std::cell::{Cell, RefCell};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Matcher calls between two polls; a budget is noticed spent at most this many calls late.
/// `FILE_WORK_BUDGET` is calibrated at this interval.
const BUDGET_CHECK_INTERVAL: u32 = 1024;

thread_local! {
    static MATCH_BUDGET: RefCell<Option<MatchDeadline>> = const { RefCell::new(None) };
}

thread_local! {
    /// Matcher calls until the next poll; reset when a budget starts, so an evaluation polls at
    /// the same calls whatever the thread evaluated before.
    static BUDGET_COUNTDOWN: Cell<u32> = const { Cell::new(0) };
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

/// Whether the work-skipping shortcuts run: always, outside tests.
pub(super) fn shortcuts_on() -> bool {
    #[cfg(test)]
    {
        !SHORTCUTS_OFF.get()
    }
    #[cfg(not(test))]
    {
        true
    }
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
            #[cfg(test)]
            spent_stores: Arc::default(),
        }
    }

    /// Spends a unit of work (or reads the clock) unless exhaustion was already observed, and
    /// records exhaustion when it is.
    pub(super) fn exhausted(&self) -> bool {
        if self.observed() {
            return true;
        }
        let open = !self.closed.load(Ordering::Relaxed)
            && match self.limit {
                Limit::Work(limit) => self.work.fetch_add(1, Ordering::Relaxed) < limit,
                Limit::Until(deadline) => Instant::now() < deadline,
            };
        if open {
            return false;
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

pub(super) struct MatchBudget {
    deadline: MatchDeadline,
}
impl MatchBudget {
    /// A budget of `units` polls: the same work runs out at the same poll, on any machine and load.
    pub(super) fn work(closed: &Arc<AtomicBool>, units: u64) -> Self {
        Self::install(MatchDeadline::new(closed, Limit::Work(units)))
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
        Self { deadline }
    }

    /// Whether a poll found the budget exhausted: an evaluation that finished without one is
    /// complete.
    pub(super) fn observed(&self) -> bool {
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
}
