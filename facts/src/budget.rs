//! Algebraic budgets (docs/facts/PHASE-3.md § P3b, G3b.3): the kill switch of a per-content plan
//! is a limit on its **algebraic work** on the content, never time or an engine's steps. The work
//! is what `facts-a`'s `crate::counts` calls algebraic and `crate::reference` reports alike: the
//! matcher's node visits and emitted rows (`match_nodes + input_rows`), the leaf rows a relational
//! clause reads, its stages' rows and its top-level fixpoints' admitted rows. The limit is
//! `A · m(class)` over the content's `n` nodes and depth `d`, where `class` is the plan's class
//! (`phys::PlanCost`), or its fallback where a per-content check failed (`bounds_unproven`), and
//! `m(unbounded) = n²`.
//!
//! Counts only grow, so the verdict "work > limit" does not depend on where it is checked: an
//! executor checks at stage boundaries and per fixpoint round (`Meter`), against the fallback
//! limit while the run is under way (no check has failed yet that could raise it further) and
//! against the content's own limit at the end ([`verdict`]). The engine and the reference count
//! the same rows, so they reach the same verdict with the same message: a fact about
//! `(content, plan digest)`, memoised with the plan's results.
use crate::phys::Class;
use std::cell::Cell;

/// `A`: the margin over `m(class)`. The corpus maximum of `work / m(class)` over every
/// `(content, clause)` of the 58 compiling rules (9,010 files, 1,449,505 rows of `facts-a repo
/// --file-counts`; report `docs/facts/reports/phase-3-p3b.md` § G3b.3) is 51, on one-node files,
/// where a clause's fixed rows outweigh `m`; at 100 nodes and more it is below 1. `A` is at least
/// four times that, rounded up to a power of two.
pub const A: u64 = 256;

thread_local! {
    static TEST_A: Cell<Option<u64>> = const { Cell::new(None) };
    static METER: Cell<Option<Meter>> = const { Cell::new(None) };
}

/// Tests: `A` on this thread (`None`: the constant).
pub fn set_test_a(a: Option<u64>) {
    TEST_A.with(|t| t.set(a));
}

pub fn a() -> u64 {
    TEST_A.with(Cell::get).unwrap_or(A)
}

/// `m(class)` at `n` nodes and depth `d` (both at least 1); `O(n log n)` as `n·⌈log₂(n+1)⌉`.
pub fn scale(class: Class, n: u64, d: u64) -> u64 {
    let (n, d) = (n.max(1), d.max(1));
    match class {
        Class::N => n,
        Class::NLogN => n.saturating_mul(u64::from(64 - n.leading_zeros())),
        Class::ND => n.saturating_mul(d),
        Class::N2 | Class::Unbounded => n.saturating_mul(n),
    }
}

/// The limit for `class` on a content of `n` nodes and depth `d`.
pub fn limit(class: Class, n: u64, d: u64) -> u64 {
    a().saturating_mul(scale(class, n, d))
}

/// A plan's limits on one content: its class's, and its fallback class's (`proven ≤ fallback`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub proven: u64,
    pub fallback: u64,
}

impl Limits {
    pub fn new(class: Class, fallback: Class, n: u64, d: u64) -> Limits {
        let (p, f) = (limit(class, n, d), limit(fallback, n, d));
        Limits { proven: p, fallback: f.max(p) }
    }

    /// The content's limit: the fallback's where a per-content check failed.
    pub fn of(self, unproven: bool) -> u64 {
        if unproven { self.fallback } else { self.proven }
    }
}

/// The failure message (`budget-exceeded`, as native's code for a spent allowance).
pub fn exceeded(limit: u64) -> String {
    format!("budget-exceeded: algebraic work over {limit}")
}

/// The verdict on a finished run: `work` against the content's limit.
pub fn verdict(work: u64, limits: Limits, unproven: bool) -> Result<(), String> {
    let l = limits.of(unproven);
    if work > l { Err(exceeded(l)) } else { Ok(()) }
}

/// A run under way on this thread: the work committed by finished stages (and the matcher and
/// leaf rows before them), and the rows of the current stage's top-level fixpoints so far.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Meter {
    pub limit: u64,
    pub spent: u64,
    pub live: u64,
}

/// Starts metering this thread's run: `spent` is the work before the stages, `limit` the
/// fallback limit (the most any content of the plan may spend).
pub fn start(limit: u64, spent: u64) {
    METER.with(|m| m.set(Some(Meter { limit, spent, live: 0 })));
}

/// Ends metering; the meter as it stood.
pub fn stop() -> Option<Meter> {
    METER.with(|m| m.take())
}

/// The meter as it stands.
pub fn meter() -> Option<Meter> {
    METER.with(Cell::get)
}

fn check(m: Meter) -> Result<(), String> {
    if m.spent.saturating_add(m.live) > m.limit { Err(exceeded(m.limit)) } else { Ok(()) }
}

/// One round of a top-level fixpoint admitted `rows`.
pub fn round(rows: u64) -> Result<(), String> {
    METER.with(|c| match c.get() {
        Some(mut m) => {
            m.live = m.live.saturating_add(rows);
            c.set(Some(m));
            check(m)
        }
        None => Ok(()),
    })
}

/// The current stage's fixpoint rows so far (an operator that gives up mid-way restores them).
pub fn live() -> u64 {
    meter().map_or(0, |m| m.live)
}

pub fn set_live(live: u64) {
    METER.with(|c| {
        if let Some(mut m) = c.get() {
            m.live = live;
            c.set(Some(m));
        }
    });
}

/// A finished stage: its rows and its top-level fixpoints' admitted rows replace what its rounds
/// counted so far.
pub fn stage(rows: u64, fix: u64) -> Result<(), String> {
    METER.with(|c| match c.get() {
        Some(mut m) => {
            m.spent = m.spent.saturating_add(rows).saturating_add(fix);
            m.live = 0;
            c.set(Some(m));
            check(m)
        }
        None => Ok(()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_limits_scale_with_the_class() {
        set_test_a(Some(2));
        assert_eq!(limit(Class::N, 100, 7), 200);
        assert_eq!(limit(Class::NLogN, 100, 7), 2 * 100 * 7);
        assert_eq!(limit(Class::ND, 100, 7), 1_400);
        assert_eq!(limit(Class::N2, 100, 7), 20_000);
        assert_eq!(limit(Class::Unbounded, 100, 7), 20_000);
        let l = Limits::new(Class::ND, Class::Unbounded, 100, 7);
        assert_eq!((l.of(false), l.of(true)), (1_400, 20_000));
        assert_eq!(verdict(1_400, l, false), Ok(()));
        assert_eq!(verdict(1_401, l, false), Err(exceeded(1_400)));
        assert_eq!(verdict(1_401, l, true), Ok(()));
        set_test_a(None);
    }

    /// Rounds count against the limit while a stage runs; a finished stage's rows and fixpoint
    /// rows replace its rounds; whatever order the checks run in, the work over the limit fails.
    #[test]
    fn budget_meter_counts_rounds_then_stages() {
        start(10, 3);
        assert_eq!(round(4), Ok(()));
        assert_eq!(live(), 4);
        set_live(0);
        assert_eq!(round(4), Ok(()));
        assert_eq!(stage(2, 4), Ok(()));
        assert_eq!(meter(), Some(Meter { limit: 10, spent: 9, live: 0 }));
        assert_eq!(round(2), Err(exceeded(10)));
        assert_eq!(stop().map(|m| m.spent + m.live), Some(11));
        assert_eq!(round(1_000), Ok(()), "no meter, no limit");
    }
}
