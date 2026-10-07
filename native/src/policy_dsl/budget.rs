//! The per-file match budget: a file evaluation's deadline. `MatchBudget::start` installs it on
//! the evaluating thread, where the matcher and anchor index poll it; a query batch registered
//! there carries the same deadline, which its SQLite progress handler and its terminals (on
//! whatever thread runs them) poll. Every poll reads one shared `MatchDeadline`.

use std::cell::{Cell, RefCell};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Matcher calls between two reads of the clock; a deadline is noticed at most this many calls late.
const BUDGET_CHECK_INTERVAL: u32 = 64;

thread_local! {
    static MATCH_BUDGET: RefCell<Option<MatchDeadline>> = const { RefCell::new(None) };
}

thread_local! {
    /// Matcher calls until the next clock read.
    static BUDGET_COUNTDOWN: Cell<u32> = const { Cell::new(0) };
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

/// Whether some poll found this thread's budget exhausted; never reads the clock. What decides,
/// after work ends, whether it was cut short: work that finished unpolled is complete, however
/// late it finished.
pub(super) fn matcher_budget_observed() -> bool {
    MATCH_BUDGET.with(|budget| {
        budget
            .borrow()
            .as_ref()
            .is_some_and(MatchDeadline::observed)
    })
}

/// The deadline of this thread's budget, for work that polls it elsewhere.
pub(super) fn current_deadline() -> Option<MatchDeadline> {
    MATCH_BUDGET.with(|budget| budget.borrow().clone())
}

/// Passes this thread's deadline without polling it: what a last step that runs past the deadline
/// and never polls leaves behind.
#[cfg(test)]
pub(super) fn pass_deadline_unpolled() {
    MATCH_BUDGET.with(|budget| {
        if let Some(deadline) = budget.borrow_mut().as_mut() {
            deadline.deadline = Instant::now();
        }
    });
}

/// `matcher_budget_exhausted` for matcher entry points: reads the clock every
/// `BUDGET_CHECK_INTERVAL` calls, so a deadline is noticed at most that many calls late.
pub(super) fn matcher_call_budget_exhausted() -> bool {
    if matcher_budget_observed() {
        return true;
    }
    let countdown = BUDGET_COUNTDOWN.get();
    if countdown > 0 {
        BUDGET_COUNTDOWN.set(countdown - 1);
        return false;
    }
    BUDGET_COUNTDOWN.set(BUDGET_CHECK_INTERVAL);
    matcher_budget_exhausted()
}

/// One file evaluation's deadline, shared by every clone: once a poll finds it passed (or the
/// workspace closed), it stays exhausted for all of them.
#[derive(Clone)]
pub(super) struct MatchDeadline {
    deadline: Instant,
    closed: Arc<AtomicBool>,
    spent: Arc<AtomicBool>,
    /// Writes of `spent`, shared by every clone.
    #[cfg(test)]
    spent_stores: Arc<std::sync::atomic::AtomicUsize>,
}

impl MatchDeadline {
    fn start(closed: &Arc<AtomicBool>, duration: Duration) -> Self {
        Self {
            deadline: Instant::now() + duration,
            closed: Arc::clone(closed),
            spent: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            spent_stores: Arc::default(),
        }
    }

    /// Reads the clock unless exhaustion was already observed, and records it when it is.
    pub(super) fn exhausted(&self) -> bool {
        if self.observed() {
            return true;
        }
        if !self.closed.load(Ordering::Relaxed) && Instant::now() < self.deadline {
            return false;
        }
        self.spent.store(true, Ordering::Relaxed);
        #[cfg(test)]
        self.spent_stores.fetch_add(1, Ordering::Relaxed);
        true
    }

    /// Whether some poll found the deadline exhausted; never reads the clock.
    pub(super) fn observed(&self) -> bool {
        self.spent.load(Ordering::Relaxed)
    }
}

pub(super) struct MatchBudget {
    deadline: MatchDeadline,
}
impl MatchBudget {
    pub(super) fn start(closed: &Arc<AtomicBool>, duration: Duration) -> Self {
        let deadline = MatchDeadline::start(closed, duration);
        MATCH_BUDGET.with(|budget| *budget.borrow_mut() = Some(deadline.clone()));
        #[cfg(test)]
        BUDGET_COUNTDOWN.set(0);
        Self { deadline }
    }

    /// Whether a poll on any thread found the deadline exhausted: an evaluation that finished
    /// without one is complete, however late it finished.
    pub(super) fn observed(&self) -> bool {
        self.deadline.observed()
    }

    #[cfg(test)]
    pub(super) fn deadline(&self) -> MatchDeadline {
        self.deadline.clone()
    }

    #[cfg(test)]
    pub(super) fn expired_for_test() -> Self {
        Self::start(&Arc::new(AtomicBool::new(false)), Duration::ZERO)
    }
}
impl Drop for MatchBudget {
    fn drop(&mut self) {
        MATCH_BUDGET.with(|budget| *budget.borrow_mut() = None);
        #[cfg(test)]
        BUDGET_COUNTDOWN.set(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_deadline_records_spent_only_when_it_becomes_exhausted() {
        let closed = Arc::new(AtomicBool::new(false));
        let deadline = MatchDeadline::start(&closed, Duration::from_secs(3_600));
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
}
