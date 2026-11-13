//! Algebra shared by native lattice joins and SQL aggregate composition.
//! Opaque callbacks retain their evaluation order and multiplicity.

/// The scalar sum contract is independent of input order. Both evaluators check the final integer range.
pub fn sum_value(sum: i128) -> rusqlite::Result<i64> {
    i64::try_from(sum).map_err(|_| rusqlite::Error::UserFunctionError("integer overflow".into()))
}
pub(in crate::code_rule_dsl) fn register_sum(db: &rusqlite::Connection) -> rusqlite::Result<()> {
    use rusqlite::functions::{Aggregate, Context, FunctionFlags};
    struct Sum;
    impl Aggregate<i128, i64> for Sum {
        fn init(&self, _: &mut Context<'_>) -> rusqlite::Result<i128> {
            Ok(0)
        }
        fn step(&self, ctx: &mut Context<'_>, sum: &mut i128) -> rusqlite::Result<()> {
            if let Some(value) = ctx.get::<Option<i64>>(0)? {
                *sum = sum.checked_add(i128::from(value)).ok_or_else(|| {
                    rusqlite::Error::UserFunctionError("scalar sum exceeds 128 bits".into())
                })?;
            }
            Ok(())
        }
        fn finalize(&self, _: &mut Context<'_>, sum: Option<i128>) -> rusqlite::Result<i64> {
            sum_value(sum.unwrap_or(0))
        }
    }
    db.create_aggregate_function(
        c"rr_exact_sum",
        1,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        Sum,
    )
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Reduction {
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

    pub fn duplicate_insensitive(self) -> bool {
        self != Self::Opaque
    }
}

/// Each input has at most one non-null value per prefix key, after this reduction.
/// This description applies before merging the previous wave or applying widening.
#[derive(Clone, Copy, Debug)]
pub struct Grouping {
    pub keys: usize,
    pub reduction: Reduction,
}

impl Grouping {
    /// A single partial result needs no second aggregate at the same keys.
    pub fn accepts(self, input: Self) -> bool {
        self.keys == input.keys
            && self.reduction == input.reduction
            && self.reduction.duplicate_insensitive()
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::code_rule_dsl::logic::native::{Flat, Lattice, Max, Min, TOP};

    #[test]
    fn exact_scalar_sum_keeps_cancellation_empty_groups_and_final_overflow() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        register_sum(&db).unwrap();
        db.execute_batch("CREATE TABLE value(v INTEGER)").unwrap();
        for values in [
            vec![],
            vec![i64::MAX, 1, -1],
            vec![1, i64::MAX, -1],
            vec![i64::MIN, -1, 1],
            vec![7, 7, 3],
        ] {
            db.execute("DELETE FROM value", []).unwrap();
            for v in &values {
                db.execute("INSERT INTO value VALUES(?1)", [v]).unwrap();
            }
            let actual: i64 = db
                .query_row("SELECT rr_exact_sum(v) FROM value", [], |r| r.get(0))
                .unwrap();
            assert_eq!(
                actual,
                sum_value(values.iter().map(|&v| i128::from(v)).sum()).unwrap()
            );
        }
        db.execute_batch("DELETE FROM value; INSERT INTO value VALUES(9223372036854775807),(1)")
            .unwrap();
        assert!(
            db.query_row("SELECT rr_exact_sum(v) FROM value", [], |r| r
                .get::<_, i64>(0))
                .is_err()
        );
    }

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
