//! Coverage of positive join changes, independent of storage and execution.
use super::Decision;

/// Every changed witness of a positive rule contains a changed input row.
/// Recheck affected heads against the current inputs to preserve alternative supports.
pub(in crate::code_rule_dsl) fn reconcile_projection(
    total: bool,
    positive: bool,
    mapped_inputs: bool,
) -> Decision {
    let reason = if !total {
        "Checked or opaque expressions retain their original evaluation"
    } else if !positive {
        "Scalar, aggregate, negative, and CTE queries retain full evaluation"
    } else if !mapped_inputs {
        "Every changing input needs a complete positional delta and its current schema"
    } else {
        "Changed positive witnesses cover every affected head; current support probes preserve duplicate derivations"
    };
    Decision::new(total && positive && mapped_inputs, reason)
}

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
    fn changed_witnesses_cover_projected_supports_with_simultaneous_deletions() {
        assert!(reconcile_projection(true, true, true).proven);
        assert!(!reconcile_projection(false, true, true).proven);
        assert!(!reconcile_projection(true, false, true).proven);
        assert!(!reconcile_projection(true, true, false).proven);
        for bits in 0..4096u32 {
            let set = |offset| {
                (0..3)
                    .filter(|n| bits & (1u32 << (offset + n)) != 0)
                    .collect::<BTreeSet<u32>>()
            };
            let (old_a, new_a, old_b, new_b) = (set(0), set(3), set(6), set(9));
            let touched_a = old_a.symmetric_difference(&new_a).copied().collect();
            let touched_b = old_b.symmetric_difference(&new_b).copied().collect();
            let superset_a = old_a.union(&new_a).copied().collect();
            let superset_b = old_b.union(&new_b).copied().collect();
            let project = |a: &BTreeSet<u32>, b: &BTreeSet<u32>| {
                a.iter()
                    .flat_map(|x| b.iter().filter(move |y| x <= *y).map(move |y| (x + y) % 2))
                    .collect::<BTreeSet<_>>()
            };
            let affected = project(&touched_a, &superset_b)
                .union(&project(&superset_a, &touched_b))
                .copied()
                .collect::<BTreeSet<_>>();
            let current = project(&new_a, &new_b);
            let patched = project(&old_a, &old_b)
                .difference(&affected)
                .copied()
                .chain(current.intersection(&affected).copied())
                .collect::<BTreeSet<_>>();
            assert_eq!(patched, current);
        }
    }

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
