//! The kernel launches of one complete Metal evaluation (list step 13; t1 155: the 600-dispatch
//! guard replaced by a plan-time bound). A plan counts its launches before it encodes one
//! (`metal_label_passes::run`), and one over this bound is refused there as `BackendLaunchLimit`;
//! this ledger is the backstop that refuses submission after an uncounted launch.
pub(crate) const KERNEL_LAUNCH_LIMIT: usize = 99;

#[derive(Default)]
pub(super) struct LaunchBudget {
    used: usize,
    refused: bool,
}

impl LaunchBudget {
    pub(super) fn reserve(&mut self) -> bool {
        if self.used == KERNEL_LAUNCH_LIMIT {
            self.refused = true;
            return false;
        }
        self.used += 1;
        true
    }

    pub(super) fn refused(&self) -> bool {
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
