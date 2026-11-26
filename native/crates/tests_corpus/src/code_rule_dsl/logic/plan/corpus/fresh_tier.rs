//! Which of a test's states are checked against a run over an empty store (a fresh run).
//!
//! A test that edits its corpus step by step checks its kept run's outputs at each state against a fresh run's. A
//! fresh run is a cold run: in the reach index's test, 12 of its 24 runs. By default a scenario checks its last
//! state only, which every edit before it led to; under `RERAIL_TEST_FRESH=every` (the farm's and a merge's full
//! tier) it checks every state, as an edit put wrong and then right again by a later one shows only there.

/// Whether every state is checked (`RERAIL_TEST_FRESH=every`).
pub fn every() -> bool {
    std::env::var("RERAIL_TEST_FRESH").is_ok_and(|v| v == "every")
}

/// Whether state `i` of a scenario's `n` (0-based, in order) is checked against a fresh run: the last always, the
/// rest under `every`.
pub fn compared(i: usize, n: usize) -> bool {
    i + 1 >= n || every()
}

#[cfg(test)]
mod tests {
    /// The last state is checked in every tier; the earlier ones only in the full tier.
    #[test]
    fn the_last_state_is_always_checked_and_the_rest_in_the_full_tier() {
        assert!(super::compared(2, 3));
        assert!(super::compared(0, 1));
        assert_eq!(super::compared(0, 3), super::every());
        assert_eq!(super::compared(1, 3), super::every());
    }
}
