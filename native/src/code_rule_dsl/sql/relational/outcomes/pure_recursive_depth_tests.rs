//! Entered-depth peaks include an entry whose work charge fails, and exclude rejected entries.
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
fn oracle(
    node: Node<'_, TapeDoc>,
    shape: usize,
    depth: u32,
    work: &mut i64,
    peak: &mut u32,
) -> i64 {
    if depth >= 256 {
        return -1;
    }
    *peak = (*peak).max(depth + 1);
    if *work == 0 {
        return -2;
    }
    *work -= 1;
    if node.kind() == "identifier" && node.text() == "a" {
        return 1;
    }
    if shape == 1 {
        return match node
            .parent()
            .filter(|p| p.kind() == "parenthesized_expression")
        {
            Some(parent) => oracle(parent, shape, depth + 1, work, peak),
            None => 0,
        };
    }
    if node.kind() != "array" {
        return 0;
    }
    let mut saw = false;
    for child in node.children() {
        if child.kind() != "array" && child.kind() != "identifier" {
            continue;
        }
        saw = true;
        let answer = oracle(child, shape, depth + 1, work, peak);
        if answer < 0 {
            return answer;
        }
        if shape == 0 && answer == 0 {
            return 0;
        }
        if shape == 2 && answer == 1 {
            return 1;
        }
    }
    if shape == 0 {
        1
    } else {
        let _ = saw;
        0
    }
}
#[test]
fn pure_machine_and_shared_fast_components_report_exact_entered_peaks() {
    let universal = json!({"any":[{"kind":"identifier","regex":"^a$"},{"kind":"array","not":{"has":{"stopBy":"neighbor","not":{"any":[{"regex":"^[\\[\\],]$"},{"kind":"comment"},{"matches":"u"}]}}}}]});
    let chain = json!({"any":[{"kind":"identifier","regex":"^a$"},{"inside":{"stopBy":"neighbor","kind":"parenthesized_expression","matches":"u"}}]});
    let downward = json!({"any":[{"kind":"identifier","regex":"^a$"},{"kind":"array","has":{"stopBy":"neighbor","matches":"u"}}]});
    for (shape, body, source) in [
        (0, universal, "f([a,[a,a],a]);"),
        (1, chain, "f((((b))));"),
        (2, downward, "f([b,[b,a],b]);"),
    ] {
        let p = policy_utils(&[json!({"matches":"u"})], json!({"u":body}));
        let grammar = &grammar_matchers(&p)[0];
        let mut lower = Lower::new(grammar.language, grammar.three_valued);
        let plan = lower.plan(&grammar.matcher).unwrap();
        exec::with_terminals(
            vec![(tape(grammar.language, source), Default::default())],
            lower.pure.program.terminals,
            |db, batch, _| {
                super::super::super::leaves::register(db, batch).unwrap();
                let file = &batch.files[0];
                let root = (0..file.facts.len())
                    .find(|&id| {
                        let node = node_at(&file.root, id);
                        if shape == 1 {
                            node.kind() == "identifier" && node.text() == "b"
                        } else {
                            node.kind() == "array"
                        }
                    })
                    .unwrap();
                for dynamic in [false, true] {
                    for incoming in [0, 1, 252, 253, 254, 255, 256] {
                        for allowance in [0, 1, 2, 3, 4, 8, 16, 64] {
                            let mut sql = Sql {
                                definitions: vec![],
                                profile: vec![],
                                correlated: false,
                                generation: Generation {
                                    peak_tracking: true,
                                    utility_depth: if dynamic { 0 } else { incoming },
                                    dynamic_depth: dynamic.then(|| "?1".into()),
                                    ..Generation::default()
                                },
                            };
                            let input = Demand {
                                name: sql.relation(
                                    "test reached invocation",
                                    format!(
                                        "SELECT 1 AS k,0 AS file,{root} AS node,{allowance} AS work"
                                    ),
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
                            let row = |r: &rusqlite::Row<'_>| {
                                Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
                            };
                            let actual = if dynamic {
                                db.query_row(&query, [incoming], row)
                            } else {
                                db.query_row(&query, [], row)
                            }
                            .unwrap();
                            let peak: i64 = db
                                .query_row("SELECT relation_peak_close(?1)", [token], |r| r.get(0))
                                .unwrap();
                            let mut left = allowance;
                            let mut expected_peak = 0;
                            let expected = oracle(
                                node_at(&file.root, root),
                                shape,
                                incoming,
                                &mut left,
                                &mut expected_peak,
                            );
                            assert_eq!(
                                (actual, peak),
                                ((expected, left), i64::from(expected_peak)),
                                "shape={shape} dynamic={dynamic} depth={incoming} work={allowance}"
                            );
                            assert_eq!(
                                db.query_row::<i64, _, _>(
                                    "SELECT relation_continuation_rows()",
                                    [],
                                    |r| r.get(0)
                                )
                                .unwrap(),
                                0
                            );
                        }
                    }
                }
            },
        );
    }
}
