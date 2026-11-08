//! Algebra shared by native lattice joins and SQL aggregate composition.
//! Opaque callbacks retain their evaluation order and multiplicity.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::code_rule_dsl) enum Reduction {
    #[default]
    Opaque,
    Flat {
        top: i64,
    },
    Min,
    Max,
}

impl Reduction {
    /// Combine two present values. Absence and widening belong to the caller.
    pub(in crate::code_rule_dsl) fn combine(self, a: i64, b: i64) -> Option<i64> {
        match self {
            Self::Opaque => None,
            Self::Flat { top } => Some(if a == b { a } else { top }),
            Self::Min => Some(a.min(b)),
            Self::Max => Some(a.max(b)),
        }
    }

    pub(in crate::code_rule_dsl) fn duplicate_insensitive(self) -> bool {
        self != Self::Opaque
    }
}

/// Each input has at most one non-null value per prefix key, after this reduction.
/// This description applies before merging the previous wave or applying widening.
#[derive(Clone, Copy, Debug)]
pub(in crate::code_rule_dsl) struct Grouping {
    pub keys: usize,
    pub reduction: Reduction,
}

impl Grouping {
    /// A single partial result needs no second aggregate at the same keys.
    pub(in crate::code_rule_dsl) fn accepts(self, input: Self) -> bool {
        self.keys == input.keys
            && self.reduction == input.reduction
            && self.reduction.duplicate_insensitive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::logic::native::{Flat, Lattice, Max, Min, TOP};

    #[test]
    fn compatibility_preserves_native_folds_and_rejects_different_keys_or_algebras() {
        let lattices: [&dyn Lattice; 3] = [&Flat, &Min, &Max];
        let values = [-TOP, -7, 0, 7, TOP];
        for lattice in lattices {
            let reduction = lattice.reduction();
            let output = Grouping { keys: 2, reduction };
            assert!(output.accepts(output));
            assert!(!output.accepts(Grouping { keys: 1, reduction }));
            for other in [
                Reduction::Opaque,
                Reduction::Flat { top: 123 },
                Reduction::Min,
                Reduction::Max,
            ] {
                assert_eq!(
                    output.accepts(Grouping {
                        keys: 2,
                        reduction: other
                    }),
                    reduction == other
                );
            }
            for a in values {
                assert_eq!(reduction.combine(a, a), Some(a));
                for b in values {
                    for c in values {
                        let direct = lattice.lub(lattice.lub(a, b), c);
                        let partial = reduction
                            .combine(a, reduction.combine(b, c).unwrap())
                            .unwrap();
                        assert_eq!(partial, direct);
                        assert_eq!(lattice.lub(a, b), reduction.combine(a, b).unwrap());
                    }
                }
            }
        }
        assert!(
            !Grouping {
                keys: 0,
                reduction: Reduction::Opaque
            }
            .accepts(Grouping {
                keys: 0,
                reduction: Reduction::Opaque
            })
        );
        assert_eq!(Reduction::Opaque.combine(1, 2), None);
    }

    #[test]
    fn flat_sql_matches_native_for_empty_duplicate_conflicting_and_top_inputs() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE input(v INTEGER NOT NULL)")
            .unwrap();
        let sql = format!(
            "SELECT {} FROM input",
            crate::code_rule_dsl::logic::sql::lattices::flat_aggregate("v", TOP)
        );
        for values in [
            vec![],
            vec![7],
            vec![7, 7],
            vec![7, 9],
            vec![TOP],
            vec![TOP, 7],
            vec![-TOP, TOP],
            vec![i64::MIN, i64::MAX],
        ] {
            db.execute("DELETE FROM input", []).unwrap();
            for value in &values {
                db.execute("INSERT INTO input VALUES (?1)", [value])
                    .unwrap();
            }
            let expected = values.into_iter().reduce(|a, b| Flat.lub(a, b));
            let actual: Option<i64> = db.query_row(&sql, [], |r| r.get(0)).unwrap();
            assert_eq!(actual, expected);
        }
        // Callback registrations cannot obtain a builtin proof through JSON.
        let ops: crate::code_rule_dsl::waves::schedule::LatticeOperations = serde_json::from_value(serde_json::json!({"aggregate":"min","merge":"max","top":TOP,"widen":null,"after":null,"reduction":"Min"})).unwrap();
        assert_eq!(ops.reduction, Reduction::Opaque);
    }
}
