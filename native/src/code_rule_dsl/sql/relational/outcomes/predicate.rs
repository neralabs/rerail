//! Total atoms on an existing node need no existence subquery. Node kinds and
//! node_regex are non-null; surrounding CASE expressions still own reachability.
use super::Set;

pub(super) fn condition(set: &Set, alias: &str) -> Result<String, String> {
    Ok(match set {
        Set::AllNodes => "1".into(),
        Set::Empty => "0".into(),
        Set::Kinds(kinds) => format!(
            "{alias}.kind IN ({})",
            kinds
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(",")
        ),
        Set::Regex(term) => format!("node_regex({term},{alias}.file,{alias}.id)"),
        // Patterns, set composition, references, and walks keep their relational
        // witnesses and joins. This is only an atomic expression specialization.
        _ => super::super::super::emit::condition(set, alias)?,
    })
}

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
        let prior = super::super::super::emit::condition(&set, "n").unwrap();
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
    // Non-atomic expressions must keep the relational witness path.
    for set in [
        Set::Pattern(0),
        Set::Reference(0),
        Set::Preimage(
            super::super::super::ir::Edge::Parent,
            Box::new(Set::Kinds(vec![110])),
        ),
    ] {
        assert_eq!(
            condition(&set, "n").unwrap(),
            super::super::super::emit::condition(&set, "n").unwrap()
        );
    }
}
