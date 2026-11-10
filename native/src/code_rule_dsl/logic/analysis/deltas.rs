//! Coverage of positive join changes, independent of storage and execution.
use super::Decision;

/// Every current join witness contains a row from each positive input.
/// If one insertion set equals its complete current input, that branch covers
/// every added witness. This says nothing about deletions or scalar invalidation.
pub(in crate::code_rule_dsl) fn complete_positive_input(
    total_context: bool,
    same_current_input: bool,
) -> Decision {
    Decision::new(
        total_context && same_current_input,
        if !total_context {
            "Checked or opaque context expressions retain their evaluation branches"
        } else if same_current_input {
            "A complete positive insertion input covers every current join witness; other insertion branches are redundant"
        } else {
            "A partial positive insertion input does not cover other changed inputs"
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn complete_input_covers_join_insertions_including_simultaneous_deletions() {
        assert!(!complete_positive_input(false, true).proven);
        assert!(!complete_positive_input(true, false).proven);
        for bits in 0..4096u32 {
            let set = |offset| {
                (0..3)
                    .filter(|n| bits & (1u32 << (offset + n)) != 0)
                    .collect::<BTreeSet<_>>()
            };
            let old_a = set(0);
            let new_a = set(3);
            let old_b = set(6);
            let new_b = set(9);
            let plus_a = new_a.difference(&old_a).copied().collect::<BTreeSet<_>>();
            let plus_b = new_b.difference(&old_b).copied().collect::<BTreeSet<_>>();
            let join = |a: &BTreeSet<u32>, b: &BTreeSet<u32>| {
                a.iter()
                    .flat_map(|x| b.iter().map(move |y| (*x, *y)))
                    .filter(|(x, y)| x <= y)
                    .collect::<BTreeSet<_>>()
            };
            let changed = join(&plus_a, &new_b)
                .union(&join(&new_a, &plus_b))
                .copied()
                .collect::<BTreeSet<_>>();
            for (complete, branch) in [
                (plus_a == new_a, join(&plus_a, &new_b)),
                (plus_b == new_b, join(&new_a, &plus_b)),
            ] {
                if complete_positive_input(true, complete).proven {
                    assert_eq!(branch, changed);
                }
            }
        }
    }
}
