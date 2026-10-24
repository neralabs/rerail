use super::super::publication_tests::check_work_compiler;
use super::*;
use crate::code_rule_dsl::{
    CodeRuleDslRuntime, CompiledCodeRuleDsl,
    sql::{exec, tests::differential::tape},
    test_support::{CAPABILITIES, grammar_matchers},
};
use serde_json::{Value, json};
fn policy(helper: &str) -> (CompiledCodeRuleDsl, [Value; 1]) {
    let mut p: Value = serde_json::from_str(include_str!(
        "../../../../../../packs/python/rules/ruff-sim905-split-static-string.json"
    ))
    .unwrap();
    let rule = json!({"matches":helper});
    p["clauses"][0]["id"] = json!("c0");
    p["clauses"][0]["forbid"]["ast"]["rule"] = rule.clone();
    (
        CodeRuleDslRuntime::compile(p.to_string(), "json".into(), CAPABILITIES.into()).unwrap(),
        [rule],
    )
}
#[test]
fn numeric_effect_frontier_preserves_reached_values_work_and_completed_memo() {
    for (name, helper) in [("int32", 1), ("neg-int32", 2), ("maxsplit", 3)] {
        let (p, rules) = policy(name);
        for work in [0, 1, 2, 3, 4, 5, 8, 16, 32, 1000, i64::MAX] {
            let report = check_work_compiler(
                &p,
                &rules,
                &[
                    "a = 0\nb = 2147483647\nc = 2147483648\nd = -1\ne = ((-2147483648))\nf = -(2147483648)\ng = (0x10)\nh = x\n",
                ],
                None,
                work,
                |g, w| compile_helper(g, w, helper),
            );
            assert_eq!(report["metrics"]["execution"]["matchingStatements"], 1);
        }
        for fault in ["2147483647", "2147483648"] {
            check_work_compiler(
                &p,
                &rules,
                &["0\n2147483647\n2147483648\n"],
                Some(fault),
                1000,
                |g, w| compile_helper(g, w, helper),
            );
        }
    }
}
#[test]
fn numeric_effect_frontier_warm_replay_depth_fit_and_cost_delta() {
    let (p, _) = policy("maxsplit");
    let grammars = grammar_matchers(&p);
    let g = &grammars[0];
    let f = frontier(g).unwrap();
    exec::with_terminals(
        vec![(tape(g.language, "((1))\n"), Default::default())],
        f.terminals,
        |db, batch, _| {
            super::super::super::leaves::register(db, batch).unwrap();
            let integer = g.language.kinds().kind_id("integer");
            let node: i64 = db
                .query_row("SELECT id FROM node WHERE kind=?1", [integer], |r| r.get(0))
                .unwrap();
            db.query_row::<i64, _, _>("SELECT relation_memo_reset(0)", [], |r| r.get(0))
                .unwrap();
            let mut rows = vec![];
            for (helper, depth, work) in [
                (1, 0, 1000),
                (1, 0, 1000),
                (3, 0, 1000),
                (1, 255, 1000),
                (1, 256, 1000),
                (1, 0, 0),
                (1, 0, 1),
            ] {
                let query = bind(
                    &f.query,
                    "0",
                    &node.to_string(),
                    &helper.to_string(),
                    &depth.to_string(),
                    &work.to_string(),
                );
                let answer: (i64, i64, i64, Option<i64>, i64) = db
                    .query_row(&query, [], |r| {
                        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
                    })
                    .unwrap();
                if depth == 256 {
                    assert_eq!((answer.0, answer.1), (-1, work));
                }
                if work == 0 {
                    assert_eq!((answer.0, answer.1), (-2, 0));
                }
                assert_eq!(
                    db.query_row::<i64, _, _>("SELECT relation_continuation_rows()", [], |r| r
                        .get(0))
                        .unwrap(),
                    0
                );
                rows.push(json!({"helper":helper,"depth":depth,"work":work,"answer":answer}));
            }
            assert!(
                rows[0]["answer"][2].as_i64().unwrap() >= rows[1]["answer"][2].as_i64().unwrap()
            );
            if let Ok(path) = std::env::var("RERAIL_SIM905_EFFECT_SPIKE_EVIDENCE") {
                std::fs::write(path,serde_json::to_vec_pretty(&json!({"contract":{"utilityOnlyFrontier":true,"pureBooleanPCs":0,"executionCost":"spent on reached miss/hit","replayCost":"spent minus native fill delta","unknown":"maxsplit falls through int32 Unknown before neg-int32","publication":"completed per utility subtree only"},"samples":rows,"limitations":["specific explicit helper graph","single clause memo namespace","caller must serialize frontier demands in native order","pure family integration pending"]})).unwrap()).unwrap();
            }
        },
    );
}

#[test]
fn numeric_frontier_matches_native_synthetic_incoming_depth_and_warm_order() {
    use crate::code_rule_dsl::matcher::{self, node_at, value_probe};
    use ast_grep_core::AstGrep;
    for (name, helper) in [("int32", 1), ("neg-int32", 2), ("maxsplit", 3)] {
        let (p, _) = policy(name);
        let grammars = grammar_matchers(&p);
        let g = &grammars[0];
        let source = "((1))\n";
        let tree = AstGrep::doc(tape(g.language, source));
        let native_id = (0..u32::try_from(tree.root().dfs().count()).unwrap())
            .find(|id| node_at(&tree, *id).kind() == "parenthesized_expression")
            .unwrap();
        let cases = [
            (0, 1000),
            (0, 0),
            (0, 1),
            (252, 1000),
            (253, 1000),
            (254, 1000),
            (255, 1000),
            (256, 1000),
            (0, 4),
            (0, 1000),
        ];
        let _scope = matcher::FileScopeSlot::install(Default::default(), Default::default(), false);
        let expected = matcher::with_file_memo(&tree, || {
            cases
                .iter()
                .map(|&(depth, work)| {
                    let (answer, calls) = value_probe::run_reached(None, || {
                        matcher::match_at_depth(g, node_at(&tree, native_id), depth, work)
                    });
                    (answer, calls)
                })
                .collect::<Vec<_>>()
        });
        let f = frontier(g).unwrap();
        exec::with_terminals(
            vec![(tape(g.language, source), Default::default())],
            f.terminals,
            |db, batch, _| {
                super::super::super::leaves::register(db, batch).unwrap();
                db.query_row::<i64, _, _>("SELECT relation_memo_reset(0)", [], |r| r.get(0))
                    .unwrap();
                for ((depth, work), (native, calls)) in cases.iter().zip(&expected) {
                    let query = bind(
                        &f.query,
                        "0",
                        &native_id.to_string(),
                        &helper.to_string(),
                        &depth.to_string(),
                        &work.to_string(),
                    );
                    let (actual, sql_calls) = value_probe::run_reached(None, || {
                        db.query_row(&query, [], |r| {
                            Ok((
                                r.get::<_, i64>(0)?,
                                r.get::<_, i64>(1)?,
                                r.get::<_, i64>(4)?,
                            ))
                        })
                        .unwrap()
                    });
                    assert_eq!(
                        (actual.0, actual.1),
                        (native.truth, native.work as i64),
                        "helper={name} depth={depth} work={work}"
                    );
                    assert_eq!(
                        &sql_calls, calls,
                        "reached atom order, helper={name}, depth={depth}, work={work}"
                    );
                    assert_eq!(
                        actual.2 as u32,
                        native.peak.saturating_sub(*depth),
                        "relative peak helper={name} depth={depth} work={work}"
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

#[test]
fn original_integer_limit_produces_unknown_and_warm_unknown_replay() {
    use crate::code_rule_dsl::matcher::{self, value_probe};
    use ast_grep_core::AstGrep;
    let (p, _) = policy("maxsplit");
    let grammars = grammar_matchers(&p);
    let g = &grammars[0];
    let source = format!("0x1{}", "0".repeat(1025));
    let tree = AstGrep::doc(tape(g.language, &source));
    let node = tree.root().dfs().find(|n| n.kind() == "integer").unwrap();
    let id = node.node_id();
    let _scope = matcher::FileScopeSlot::install(Default::default(), Default::default(), false);
    let native = matcher::with_file_memo(&tree, || {
        (0..2)
            .map(|_| {
                value_probe::run_reached(None, || {
                    matcher::match_at_depth(g, node.clone(), 0, 10000)
                })
            })
            .collect::<Vec<_>>()
    });
    assert_eq!(
        native[0].0.truth, 2,
        "original 4096-bit evaluator bound must produce real Unknown"
    );
    assert_eq!(native[1].0.truth, 2);
    assert_eq!(native[0].1, vec![source.clone()]);
    assert!(native[1].1.is_empty());
    let f = frontier(g).unwrap();
    exec::with_terminals(
        vec![(tape(g.language, &source), Default::default())],
        f.terminals,
        |db, batch, _| {
            super::super::super::leaves::register(db, batch).unwrap();
            db.query_row::<i64, _, _>("SELECT relation_memo_reset(0)", [], |r| r.get(0))
                .unwrap();
            let query = bind(&f.query, "0", &id.to_string(), "3", "0", "10000");
            for (expected, calls) in &native {
                let (row, actual_calls) = value_probe::run_reached(None, || {
                    db.query_row(&query, [], |r| {
                        Ok((
                            r.get::<_, i64>(0)?,
                            r.get::<_, i64>(1)?,
                            r.get::<_, i64>(2)?,
                            r.get::<_, i64>(3)?,
                        ))
                    })
                    .unwrap()
                });
                assert_eq!((row.0, row.1), (2, expected.work as i64));
                assert_eq!(&actual_calls, calls);
                assert_eq!(
                    row.3,
                    10000 - native[1].0.work as i64,
                    "warm replay cost follows original native observed work, not a guessed utility count"
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

#[test]
fn pure_false_maxsplit_stratum_has_three_logical_entries_and_no_value() {
    use crate::code_rule_dsl::matcher::{self, value_probe};
    use ast_grep_core::AstGrep;
    let (p, _) = policy("maxsplit");
    let grammars = grammar_matchers(&p);
    let g = &grammars[0];
    let tree = AstGrep::doc(tape(g.language, "x\n"));
    let node = tree
        .root()
        .dfs()
        .find(|n| n.kind() == "identifier")
        .unwrap();
    let _scope = matcher::FileScopeSlot::install(Default::default(), Default::default(), false);
    for work in [0, 1, 2, 3, 4, 1000] {
        let (answer, calls) = value_probe::run_reached(None, || {
            matcher::with_file_memo(&tree, || matcher::match_at_depth(g, node.clone(), 0, work))
        });
        assert!(calls.is_empty());
        assert_eq!(
            (answer.truth, answer.work),
            (if work < 3 { -2 } else { 0 }, work.saturating_sub(3))
        );
    }
}

#[test]
fn sticky_failure_peeks_reached_cached_any_and_child_followers_without_execution() {
    use crate::code_rule_dsl::matcher::{self, AstMatcher, UtilRef, value_probe};
    use ast_grep_core::AstGrep;
    use std::sync::Arc;
    let (p, _) = policy("maxsplit");
    let grammars = grammar_matchers(&p);
    let g = &grammars[0];
    for scenario in ["any", "children"] {
        for incoming in [0, 250, 251, 252, 253, 255, 256] {
            for work in [0usize, 1, 2, 3, 10000] {
                let source = "-((1))\n";
                let tree = AstGrep::doc(tape(g.language, source));
                let unary = tree
                    .root()
                    .dfs()
                    .find(|n| n.kind() == "unary_operator")
                    .unwrap();
                let operand = unary
                    .children()
                    .find(|n| n.kind() == "parenthesized_expression")
                    .unwrap();
                let (warm_name, warm_helper, warm_node, cold_name, cold_helper) =
                    if scenario == "any" {
                        ("neg-int32", 2, unary.clone(), "maxsplit", 3)
                    } else {
                        ("int32", 1, operand, "neg-int32", 2)
                    };
                let make_rule = |name: &str| {
                    let index = g.utils.names.iter().position(|n| n == name).unwrap();
                    AstMatcher::Util(UtilRef {
                        table: Arc::downgrade(&g.utils),
                        index,
                    })
                };
                // Value reads make completed utility answers clause-owner keyed.
                // Both invocations belong to ONE owner; only the selected root
                // rule changes. Box guarantees an unchanged owner address.
                let mut owner = Box::new(g.clone());
                let _scope =
                    matcher::FileScopeSlot::install(Default::default(), Default::default(), false);
                let (warm, cold) = matcher::with_file_memo(&tree, || {
                    owner.matcher = make_rule(warm_name);
                    let warm = value_probe::run_reached(None, || {
                        matcher::match_at_depth(&owner, warm_node.clone(), 0, 10000)
                    });
                    owner.matcher = make_rule(cold_name);
                    let cold = value_probe::run_reached(None, || {
                        matcher::match_at_depth(&owner, unary.clone(), incoming, work)
                    });
                    (warm, cold)
                });
                assert_eq!(
                    warm.0.truth, 1,
                    "warm follower must publish a completed matching answer"
                );
                assert!(
                    cold.1.is_empty(),
                    "no Value body runs after warm lookup/sticky failure"
                );
                if incoming == 0 && work == 1 {
                    assert_eq!(cold.0.truth, -2);
                    assert!(
                        cold.0.peak > 2,
                        "must exercise cached follower raising the native peak"
                    );
                }
                let f = frontier(g).unwrap();
                exec::with_terminals(
                    vec![(tape(g.language, source), Default::default())],
                    f.terminals,
                    |db, batch, _| {
                        super::super::super::leaves::register(db, batch).unwrap();
                        db.query_row::<i64, _, _>("SELECT relation_memo_reset(0)", [], |r| {
                            r.get(0)
                        })
                        .unwrap();
                        for (helper, node, depth, allowance, (native, calls)) in [
                            (warm_helper, warm_node.node_id(), 0, 10000, &warm),
                            (cold_helper, unary.node_id(), incoming, work, &cold),
                        ] {
                            let token: i64 = db
                                .query_row("SELECT relation_peak_open(0)", [], |r| r.get(0))
                                .unwrap();
                            let query = bind(
                                &f.query,
                                "0",
                                &node.to_string(),
                                &helper.to_string(),
                                &depth.to_string(),
                                &allowance.to_string(),
                            );
                            let publications_before = super::super::super::memo_store::counts()[3];
                            let (row, actual_calls) = value_probe::run_reached(None, || {
                                db.query_row(&query, [], |r| {
                                    Ok((
                                        r.get::<_, i64>(0)?,
                                        r.get::<_, i64>(1)?,
                                        r.get::<_, i64>(4)?,
                                    ))
                                })
                                .unwrap()
                            });
                            let outer_peak: i64 = db
                                .query_row("SELECT relation_peak_close(?1)", [token], |r| r.get(0))
                                .unwrap();
                            if helper == cold_helper && work == 1 {
                                assert_eq!(
                                    super::super::super::memo_store::counts()[3],
                                    publications_before,
                                    "no answer is published after sticky failure"
                                );
                            }
                            let relative = native.peak.saturating_sub(depth);
                            let entered = if relative == 0 { 0 } else { depth + relative };
                            assert_eq!(
                                (row.0, row.1, row.2),
                                (native.truth, native.work as i64, relative as i64),
                                "scenario={scenario},helper={helper},depth={depth},work={allowance}"
                            );
                            assert_eq!(outer_peak, entered as i64);
                            assert_eq!(&actual_calls, calls);
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
                    },
                );
            }
        }
    }
}
