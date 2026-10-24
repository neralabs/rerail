//! Independent utility-entry oracle for completed subtree memo peaks and replay costs.
use super::super::{Slots, lower::Lower, publication_tests::policy_utils};
use super::{Demand, Generation, Sql};
use crate::code_rule_dsl::{
    matcher::node_at,
    sql::{exec, tests::differential::tape},
    tape::TapeDoc,
    test_support::grammar_matchers,
};
use ast_grep_core::Node;
use serde_json::json;
use std::collections::BTreeMap;
#[derive(Clone, Copy)]
struct Answer {
    truth: i64,
    cost: i64,
    levels: u32,
}
fn oracle(
    node: Node<'_, TapeDoc>,
    depth: u32,
    work: &mut i64,
    peak: &mut u32,
    memo: &mut BTreeMap<usize, Answer>,
) -> i64 {
    if let Some(answer) = memo
        .get(&node.node_id())
        .copied()
        .filter(|answer| depth + answer.levels <= 256)
    {
        *peak = (*peak).max(depth + answer.levels);
        if *work < answer.cost {
            *work = 0;
            return -2;
        }
        *work -= answer.cost;
        return answer.truth;
    }
    if depth >= 256 {
        return -1;
    }
    let start = *work;
    let mut entered = depth + 1;
    let truth = (|| {
        if *work == 0 {
            return -2;
        }
        *work -= 1;
        if node.kind() == "number" {
            if *work == 0 {
                return -4;
            }
            *work -= 1;
            if node.text() == "1" {
                return 1;
            }
        }
        if node.kind() != "array" {
            return 0;
        }
        for child in node.children() {
            let truth = oracle(child, depth + 1, work, &mut entered, memo);
            if truth < 0 || truth == 1 {
                return truth;
            }
        }
        0
    })();
    *peak = (*peak).max(entered);
    if truth >= 0 {
        memo.insert(
            node.node_id(),
            Answer {
                truth,
                cost: start - *work,
                levels: entered - depth,
            },
        );
    }
    truth
}
#[test]
fn recursive_memo_replays_only_depth_fitting_completed_answers() {
    let body = json!({"any":[{"kind":"number","value":{"gt":0}},{"kind":"array","has":{"stopBy":"neighbor","matches":"u"}}]});
    let p = policy_utils(&[json!({"matches":"u"})], json!({"u":body}));
    let grammar = &grammar_matchers(&p)[0];
    // Value decoding is a file-scope cache, independent of relation_memo_reset.
    // Each incoming-depth mode needs the same cold scope as the independent oracle.
    for dynamic in [false, true] {
        let mut lower = Lower::new(grammar.language, grammar.three_valued);
        let plan = lower.plan(&grammar.matcher).unwrap();
        let layout = lower.memo_layout(true);
        exec::with_terminals(
            vec![(
                tape(grammar.language, "f([0,[0,1],0]);"),
                Default::default(),
            )],
            lower.pure.program.terminals,
            |db, batch, _| {
                super::super::super::leaves::register(db, batch).unwrap();
                let file = &batch.files[0];
                let root = (0..file.facts.len())
                    .find(|id| node_at(&file.root, *id).kind() == "array")
                    .unwrap();
                db.query_row::<i64, _, _>("SELECT relation_memo_reset(0)", [], |r| r.get(0))
                    .unwrap();
                let mut memo = BTreeMap::new();
                for (incoming, allowance) in [
                    (0, 3),
                    (0, 1000),
                    (253, 1000),
                    (252, 1000),
                    (255, 0),
                    (256, 0),
                    (0, 2),
                    (0, 1000),
                ] {
                    let mut sql = Sql {
                        definitions: vec![],
                        profile: vec![],
                        correlated: false,
                        generation: Generation {
                            peak_tracking: true,
                            memo_clause: Some(0),
                            memo_layout: layout.clone(),
                            utility_depth: if dynamic { 0 } else { incoming },
                            dynamic_depth: dynamic.then(|| "?1".into()),
                            ..Generation::default()
                        },
                    };
                    let input = Demand {
                        name: sql.relation(
                            "test invocation",
                            format!("SELECT 1 AS k,0 AS file,{root} AS node,{allowance} AS work"),
                            true,
                            "single input",
                        ),
                        slots: Slots::new(),
                    };
                    sql.generation.components.borrow_mut().roots = input.name.clone();
                    let output = sql.emit(&plan, &input).unwrap();
                    sql.publish_components();
                    let query = format!(
                        "WITH RECURSIVE {} SELECT truth,work FROM {}",
                        sql.definitions.join(",\n"),
                        output.name
                    );
                    let token: i64 = db
                        .query_row("SELECT relation_peak_open(0)", [], |r| r.get(0))
                        .unwrap();
                    let row =
                        |r: &rusqlite::Row<'_>| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?));
                    let actual = if dynamic {
                        db.query_row(&query, [incoming], row)
                    } else {
                        db.query_row(&query, [], row)
                    }
                    .unwrap();
                    let peak: i64 = db
                        .query_row("SELECT relation_peak_close(?1)", [token], |r| r.get(0))
                        .unwrap();
                    let mut work = allowance;
                    let mut expected_peak = 0;
                    let truth = oracle(
                        node_at(&file.root, root),
                        incoming,
                        &mut work,
                        &mut expected_peak,
                        &mut memo,
                    );
                    assert_eq!(
                        (actual, peak),
                        ((truth, work), i64::from(expected_peak)),
                        "dynamic={dynamic} depth={incoming} work={allowance}"
                    );
                    assert_eq!(
                        db.query_row::<i64, _, _>("SELECT relation_continuation_rows()", [], |r| r
                            .get(0))
                            .unwrap(),
                        0
                    );
                }
            },
        );
    }
}
