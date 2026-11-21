//! The kernel launches of one complete Metal evaluation (list step 13; t1 155: the 600-dispatch
//! guard replaced by a plan-time bound). A plan counts its launches before it encodes one
//! (`metal_label_passes::run`), and one over this bound is refused there as `BackendLaunchLimit`;
//! this ledger is the backstop that refuses submission after an uncounted launch.
/// 200 since list step 26 (user decision, 2026-11-03): the wave doubled to 158 clauses with fact labels.
pub const KERNEL_LAUNCH_LIMIT: usize = 200;

/// The reruns' launches (user decision, 2026-11-21): a rerun of an evaluation (a run again with more levels, more
/// budget, wider lanes, more rounds) charges a budget apart from the evaluation's own `KERNEL_LAUNCH_LIMIT`, which
/// the evaluations of a run share. Each evaluation adds one rerun's worth (`KERNEL_LAUNCH_LIMIT`) as it begins, so the
/// reruns are one an evaluation on average: one that needs more takes what others did not. Outside a run an
/// evaluation's is its own.
#[derive(Default)]
pub struct Reruns {
    granted: std::sync::atomic::AtomicUsize,
    spent: std::sync::atomic::AtomicUsize,
}

impl Reruns {
    /// One evaluation's share.
    pub fn grant(&self) {
        self.granted.fetch_add(KERNEL_LAUNCH_LIMIT, std::sync::atomic::Ordering::Relaxed);
    }

    /// Takes one launch: false where the shares are spent.
    pub fn take(&self) -> bool {
        use std::sync::atomic::Ordering::Relaxed;
        self.spent
            .try_update(Relaxed, Relaxed, |spent| (spent < self.granted.load(Relaxed)).then_some(spent + 1))
            .is_ok()
    }

    /// The launches left.
    pub fn available(&self) -> usize {
        use std::sync::atomic::Ordering::Relaxed;
        self.granted.load(Relaxed).saturating_sub(self.spent.load(Relaxed))
    }
}

#[derive(Default)]
pub struct LaunchBudget {
    used: usize,
    refused: bool,
}

impl LaunchBudget {
    pub fn reserve(&mut self) -> bool {
        if self.used == KERNEL_LAUNCH_LIMIT {
            self.refused = true;
            return false;
        }
        self.used += 1;
        true
    }

    pub fn refused(&self) -> bool {
        self.refused
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permits_exactly_the_limit_and_refuses_every_later_launch() {
        let mut budget = LaunchBudget::default();
        for _ in 0..KERNEL_LAUNCH_LIMIT {
            assert!(budget.reserve());
        }
        assert!(!budget.reserve());
        assert!(budget.refused());
        assert!(!budget.reserve());
        assert_eq!(budget.used, KERNEL_LAUNCH_LIMIT);
    }
}
