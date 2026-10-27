//! Fixed kernel-launch budget for one complete Metal evaluation.

// Temporary user-approved ceiling. Complete evaluations must return below 100
// through execution-shape improvements, without resetting this shared budget.
pub(super) const KERNEL_LAUNCH_LIMIT: usize = 600;

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
    fn permits_exactly_600_launches_and_refuses_every_later_launch() {
        let mut budget = LaunchBudget::default();
        for _ in 0..600 {
            assert!(budget.reserve());
        }
        assert!(!budget.reserve());
        assert!(budget.refused());
        assert!(!budget.reserve());
        assert_eq!(budget.used, 600);
    }
}
