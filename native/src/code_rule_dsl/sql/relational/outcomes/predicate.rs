//! Bound predicates share the relational emitter. Surrounding CASE expressions
//! continue to own reached calls and publication boundaries.
use super::Set;

pub(super) use super::super::super::emit::condition;

#[test]
fn atomic_predicate_truth_negation_and_reached_calls() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let db = rusqlite::Connection::open_in_memory().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    db.create_scalar_function(
        "node_regex",
        3,
        rusqlite::functions::FunctionFlags::SQLITE_UTF8,
        move |ctx| {
            observed.fetch_add(1, Ordering::Relaxed);
            Ok(ctx.get::<i64>(0)? != 0)
        },
    )
    .unwrap();
    for set in [
        Set::AllNodes,
        Set::Empty,
        Set::Kinds(vec![]),
        Set::Kinds(vec![0, 110, u16::MAX]),
        Set::Regex(0),
        Set::Regex(1),
    ] {
        let direct = condition(&set, "n").unwrap();
        assert!(!direct.contains("SELECT"));
        let prior = format!("NOT (NOT EXISTS (SELECT 1 WHERE {direct}))");
        for kind in [0, 109, 110, 65535] {
            for guard in [false, true] {
                for negate in [false, true] {
                    let evaluate = |expression: &str| {
                        let expression = if negate {
                            format!("NOT ({expression})")
                        } else {
                            expression.into()
                        };
                        let query = format!(
                            "SELECT CASE WHEN ?1 THEN CASE WHEN {expression} THEN 1 ELSE 0 END END FROM (SELECT 0 AS file,1 AS id,?2 AS kind) n"
                        );
                        calls.store(0, Ordering::Relaxed);
                        let answer: Option<i64> = db
                            .query_row(&query, rusqlite::params![guard, kind], |r| r.get(0))
                            .unwrap();
                        (answer, calls.load(Ordering::Relaxed))
                    };
                    let result = evaluate(&direct);
                    assert_eq!(result, evaluate(&prior));
                    assert_eq!(result.1, usize::from(guard && matches!(set, Set::Regex(_))));
                }
            }
        }
    }
}
