use super::super::publication_tests::{check_work_compiler, policy_utils};
use super::*;
use crate::code_rule_dsl::{
    sql::{exec, tests::differential::tape},
    test_support::grammar_matchers,
};
use serde_json::json;

#[test]
fn pure_cost_relations_preserve_native_order_unknown_and_finite_root_publication() {
    let utilities = json!({"a":{"kind":"identifier","regex":"^a$"},"b":{"any":[{"matches":"a"},{"all":[{"kind":"number"},{"not":{"regex":"^0$"}}]}]},"u":{"all":[{"testUnknown":"^x$"},{"any":[{"matches":"b"},{"kind":"identifier","regex":"^x$"}]}]}});
    for rule in [
        json!({"matches":"u"}),
        json!({"kind":"array","has":{"stopBy":"neighbor","matches":"u"}}),
        json!({"any":[{"all":[{"matches":"a"},{"not":{"matches":"a"}}]},{"matches":"b"}]}),
    ] {
        let rules = [rule];
        let policy = policy_utils(&rules, utilities.clone());
        for allowance in [0, 1, 2, 3, 4, 5, 8, 16, 1000, i64::MAX] {
            let r = check_work_compiler(
                &policy,
                &rules,
                &["f([x,0,a,1]); b; a;", "0; x; a;"],
                None,
                allowance,
                compile,
            );
            assert_eq!(r["metrics"]["execution"]["matchingStatements"], 1);
            assert_eq!(r["metrics"]["framePublications"], 0);
            assert!(!r["sql"].as_str().unwrap().contains("relation_continuation"));
            assert!(!r["sql"].as_str().unwrap().contains("relation_state_"));
        }
    }
}
#[test]
fn summary_depth_failure_prefix_precedes_later_work_failure() {
    let rules = [json!({"matches":"u"})];
    let policy = policy_utils(
        &rules,
        json!({"u":{"matches":"v"},"v":{"kind":"identifier"}}),
    );
    let grammars = grammar_matchers(&policy);
    let projected = compile_summary(
        &grammars[0],
        254,
        "SELECT file,id node FROM node WHERE id=0",
    )
    .unwrap();
    exec::with_terminals(
        vec![(tape(grammars[0].language, "a;"), Default::default())],
        projected.terminals,
        |db, batch, _| {
            super::super::super::leaves::register(db, batch).unwrap();
            let row: (i64, i64) = db
                .query_row(&projected.sql, [], |r| Ok((r.get(3)?, r.get(4)?)))
                .unwrap();
            assert_eq!(row, (2, 2));
        },
    );
    for incoming in [0, 253, 254, 255, 256] {
        for allowance in [0, 1, 2, 1000] {
            let c = compile_at_depth(&[&grammars[0]], allowance, incoming).unwrap();
            exec::with_terminals(
                vec![(tape(grammars[0].language, "a;"), Default::default())],
                c.terminals,
                |db, batch, _| {
                    super::super::super::leaves::register(db, batch).unwrap();
                    let rows = db
                        .prepare(&c.sql)
                        .unwrap()
                        .query_map([], |r| {
                            Ok((
                                r.get::<_, Option<i64>>(2)?,
                                r.get::<_, i64>(4)?,
                                r.get::<_, i64>(5)?,
                            ))
                        })
                        .unwrap()
                        .collect::<rusqlite::Result<Vec<_>>>()
                        .unwrap();
                    // Every candidate spends two utility entries unless the incoming
                    // level rejects entry first. Failed pairs discard all matches.
                    let depth_cost = if incoming >= 256 {
                        0
                    } else if incoming == 255 {
                        1
                    } else {
                        2
                    };
                    let depth_failed = incoming >= 255;
                    let first_failure = rows.iter().find(|r| r.1 < 0);
                    if depth_failed || allowance < 2 {
                        let failure = first_failure.expect("failure row");
                        assert_eq!(failure.1, if allowance < depth_cost { -2 } else { -1 });
                        assert_eq!(
                            failure.2,
                            if allowance < depth_cost {
                                0
                            } else {
                                allowance - depth_cost
                            }
                        );
                    } else if let Some(failure) = first_failure {
                        // Later roots can exhaust a shared file allowance.
                        assert_eq!(failure.1, -2);
                        assert_eq!(failure.2, 0);
                    }
                    assert_eq!(
                        db.query_row::<i64, _, _>("SELECT relation_continuation_rows()", [], |r| r
                            .get(0))
                            .unwrap(),
                        0
                    );
                },
            );
        }
    }
}
#[test]
fn scalar_contract_refuses_throwing_cache_fill_and_capture_effects() {
    for rule in [
        json!({"value":{"gt":0}}),
        json!({"pattern":"$X"}),
        json!({"matches":"u"}),
    ] {
        let p = policy_utils(
            &[rule],
            json!({"u":{"all":[{"value":{"gt":0}},{"value":{"gt":0}}]}}),
        );
        let g = grammar_matchers(&p);
        assert!(compile(&[&g[0]], 1000).is_err());
    }
    // A utility's first Value evaluation fills native state; replaying its
    // scalar truth twice would erase the once-only callback/cache distinction.
    let repeated = [json!({"kind":"number","all":[{"matches":"q"},{"matches":"q"}]})];
    let cached = policy_utils(&repeated, json!({"q":{"value":{"gt":0}}}));
    let cached_report = check_work_compiler(
        &cached,
        &repeated,
        &["1;"],
        None,
        1000,
        super::super::compile_job,
    );
    assert_eq!(cached_report["nativeTraces"][0]["calls"], json!(["1"]));
    let cached_grammars = grammar_matchers(&cached);
    assert!(compile(&[&cached_grammars[0]], 1000).is_err());
    // The reached callback is observable even if a later predicate is false;
    // computing only the final false truth would erase a throwing operation.
    let rules = [json!({"kind":"number","all":[{"value":{"gt":0}},{"regex":"^never$"}]})];
    let p = policy_utils(&rules, json!({}));
    let reached = check_work_compiler(
        &p,
        &rules,
        &["1;2;"],
        Some("1"),
        1000,
        super::super::compile_job,
    );
    assert!(
        !reached["nativeTraces"][0]["calls"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        reached["output"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r[4].as_i64().unwrap() < 0)
    );
}
#[test]
fn batched_cost_summary_scaling_has_constant_sql_and_no_control_frames() {
    let rules = [json!({"kind":"array","has":{"stopBy":"neighbor","matches":"u"}})];
    let p = policy_utils(
        &rules,
        json!({"u":{"any":[{"all":[{"kind":"identifier"},{"regex":"^z$"}]},{"kind":"number"}]}}),
    );
    let mut samples = vec![];
    for width in [8, 16, 32] {
        let source = format!("f([{},z]);", vec!["a"; width].join(","));
        let r = check_work_compiler(&p, &rules, &[&source], None, i64::MAX, compile);
        samples.push(json!({"width":width,"sqlBytes":r["sql"].as_str().unwrap().len(),"metrics":r["metrics"],"nativeTraces":r["nativeTraces"]}));
    }
    assert!(
        samples
            .iter()
            .all(|s| s["sqlBytes"] == samples[0]["sqlBytes"])
    );
    let v0 = samples[0]["metrics"]["execution"]["vmSteps"]
        .as_u64()
        .unwrap();
    let v2 = samples[2]["metrics"]["execution"]["vmSteps"]
        .as_u64()
        .unwrap();
    assert!(v2 < 5 * v0, "bounded batched relational growth");
    if let Ok(path) = std::env::var("RERAIL_COST_RELATION_SPIKE_EVIDENCE") {
        std::fs::write(path,serde_json::to_vec_pretty(&json!({"contract":{"captures":false,"callbacks":false,"cacheFills":false,"continuationCells":0,"unknown":"explicit Kleene truth with reached prefix","depth":"incoming-depth-indexed failure prefixes","work":"file-root prefix cost publication"},"limitations":["acyclic utilities only","single grammar","immediate child search without stop rules","no captures/value/cache-fill effects","all-node summary construction; projected-demand optimization remains future work"],"samples":samples})).unwrap()).unwrap();
    }
}

#[test]
fn pure_summary_incoming_depth_matches_original_native_matcher() {
    use crate::code_rule_dsl::matcher::{self, node_at};
    let rules = [json!({"matches":"u"})];
    let p = policy_utils(
        &rules,
        json!({"u":{"matches":"v"},"v":{"kind":"identifier"}}),
    );
    let grammars = grammar_matchers(&p);
    let g = &grammars[0];
    let initial = compile_summary(g, 0, "SELECT file,id node FROM node").unwrap();
    exec::with_terminals(
        vec![(tape(g.language, "a;"), Default::default())],
        initial.terminals,
        |db, batch, _| {
            super::super::super::leaves::register(db, batch).unwrap();
            let file = &batch.files[0];
            let id = (0..file.facts.len())
                .find(|id| node_at(&file.root, *id).kind() == "identifier")
                .unwrap();
            let _scope =
                matcher::FileScopeSlot::install(Default::default(), Default::default(), false);
            matcher::with_file_memo(&file.root, || {
                // Warm the native completed-answer cache, then test fitting replay,
                // over-deep misses, and rejected entry without physical recursion.
                matcher::match_at_depth(g, node_at(&file.root, id), 0, 1000);
                for incoming in [0, 253, 254, 255, 256] {
                    let c =
                        compile_summary(g, incoming, &format!("SELECT 0 file,{id} node")).unwrap();
                    let summary: (i64, i64, i64) = db
                        .query_row(&c.sql, [], |r| Ok((r.get(2)?, r.get(3)?, r.get(4)?)))
                        .unwrap();
                    for allowance in [0, 1, 2, 1000] {
                        let native = matcher::match_at_depth(
                            g,
                            node_at(&file.root, id),
                            incoming,
                            allowance,
                        );
                        let expected_truth = if allowance < summary.1 as usize {
                            -2
                        } else {
                            summary.0
                        };
                        let remaining = allowance.saturating_sub(summary.1 as usize);
                        assert_eq!(
                            (native.truth, native.work),
                            (expected_truth, remaining),
                            "incoming={incoming}, work={allowance}"
                        );
                        assert_eq!(native.peak.saturating_sub(incoming), summary.2 as u32);
                        assert_eq!(native.limit.is_some(), native.truth < 0);
                    }
                }
            });
        },
    );
}

#[test]
fn restricted_summary_matches_broad_answers_and_avoids_unrelated_nodes() {
    let rules = [json!({"matches":"u"})];
    let p = policy_utils(
        &rules,
        json!({"u":{"any":[{"kind":"identifier","regex":"^a$"},{"has":{"stopBy":"neighbor","matches":"v"}}]},"v":{"kind":"identifier","regex":"^a$"}}),
    );
    let grammars = grammar_matchers(&p);
    let g = &grammars[0];
    let source = format!("[a,b];\n{}", "unrelated;\n".repeat(1000));
    let demand = format!(
        "SELECT file,id node FROM node WHERE kind={}",
        g.language.kinds().kind_id("array")
    );
    let mut answers = vec![];
    let mut metrics = vec![];
    for restricted in [false, true] {
        let c = if restricted {
            compile_summary_demanded(g, 0, &demand)
        } else {
            compile_summary(g, 0, &demand)
        }
        .unwrap();
        let (rows, vm) = exec::with_terminals(
            vec![(tape(g.language, &source), Default::default())],
            c.terminals,
            |db, batch, _| {
                super::super::super::leaves::register(db, batch).unwrap();
                let mut stmt = db.prepare(&c.sql).unwrap();
                let rows = stmt
                    .query_map([], |r| {
                        Ok([
                            r.get::<_, i64>(0)?,
                            r.get(1)?,
                            r.get(2)?,
                            r.get(3)?,
                            r.get(4)?,
                        ])
                    })
                    .unwrap()
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .unwrap();
                (rows, stmt.get_status(rusqlite::StatementStatus::VmStep))
            },
        );
        answers.push(rows);
        metrics.push(vm);
    }
    assert!(!answers[0].is_empty());
    assert_eq!(answers[0], answers[1]);
    assert!(metrics[1] * 10 < metrics[0], "{metrics:?}");
    if let Ok(path) = std::env::var("RERAIL_COST_RELATION_SPIKE_EVIDENCE") {
        std::fs::write(format!("{path}.demand.json"),serde_json::to_vec_pretty(&json!({"broadVm":metrics[0],"restrictedVm":metrics[1],"rows":answers[0].len(),"passed":true,"contract":"same pure summaries; closure excludes unrelated nodes"})).unwrap()).unwrap();
    }
}

#[test]
fn dynamic_incoming_levels_match_constant_depth_producers_without_sql_clones() {
    let rules = [json!({"matches":"u"})];
    let p = policy_utils(
        &rules,
        json!({"u":{"any":[{"matches":"v"},{"has":{"stopBy":"neighbor","matches":"v"}}]},"v":{"kind":"identifier","regex":"^a$"}}),
    );
    let g = &grammar_matchers(&p)[0];
    let demanded = "SELECT file,id node FROM node";
    let dynamic=compile_summary_dynamic(g,"SELECT n.file,n.id node,l.column1 incoming FROM node n CROSS JOIN(VALUES(0),(253),(254),(255),(256)) l","dynamic_control").unwrap();
    let sql = format!(
        "WITH RECURSIVE {} {} ORDER BY 1,2,3",
        dynamic.definitions.join(","),
        dynamic.select
    );
    exec::with_terminals(
        vec![(tape(g.language, "[a,b];"), Default::default())],
        dynamic.terminals,
        |db, batch, _| {
            super::super::super::leaves::register(db, batch).unwrap();
            let actual = db
                .prepare(&sql)
                .unwrap()
                .query_map([], |r| {
                    Ok([
                        r.get::<_, i64>(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ])
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            let mut expected = vec![];
            let mut cloned_bytes = 0;
            for incoming in [0, 253, 254, 255, 256] {
                let c = compile_summary_demanded(g, incoming, demanded).unwrap();
                cloned_bytes += c.sql.len();
                let rows = db
                    .prepare(&c.sql)
                    .unwrap()
                    .query_map([], |r| {
                        Ok([
                            r.get::<_, i64>(0)?,
                            r.get(1)?,
                            i64::from(incoming),
                            r.get(2)?,
                            r.get(3)?,
                            r.get(4)?,
                        ])
                    })
                    .unwrap()
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .unwrap();
                expected.extend(rows);
            }
            expected.sort();
            assert_eq!(actual, expected);
            assert!(sql.len() * 2 < cloned_bytes);
            if let Ok(path) = std::env::var("RERAIL_COST_RELATION_SPIKE_EVIDENCE") {
                std::fs::write(format!("{path}.dynamic.json"),serde_json::to_vec_pretty(&json!({"passed":true,"rows":actual.len(),"dynamicSqlBytes":sql.len(),"clonedSqlBytes":cloned_bytes,"contract":"incoming level is a data key; identical depth failure prefixes"})).unwrap()).unwrap();
            }
        },
    );
}
