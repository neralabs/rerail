//! Native oracle for composed recursive answers and exact failed entry prefixes.
use super::super::{Slots, finite_lower, lower::Lower, publication_tests::policy_utils};
use super::{Demand, Generation, Sql};
use crate::code_rule_dsl::{
    matcher::{self, node_at},
    sql::{exec, tests::differential::tape},
    test_support::grammar_matchers,
};
use serde_json::json;
#[test]
fn recursive_caller_regions_match_native_depth_peak_and_warm_memos() {
    let policy = policy_utils(
        &[json!({"all":[{"matches":"wrapper"},{"matches":"wrapper"}]})],
        json!({
            "wrapper":{"all":[{"not":{"regex":"^never$"}},{"any":[{"matches":"outer"},{"not":{"matches":"outer"}}]},{"matches":"outer"}]},
            "outer":{"any":[{"matches":"left"},{"matches":"right"},{"kind":"parenthesized_expression","has":{"stopBy":"neighbor","matches":"outer"}}]},
            "left":{"kind":"identifier","regex":"^a$"},
            "right":{"kind":"identifier","matches":"leaf"},
            "leaf":{"kind":"identifier","regex":"^b$"}
        }),
    );
    let g = &grammar_matchers(&policy)[0];
    let mut lower = Lower::new_finite(g.language, g.three_valued);
    let plan = lower.plan(&g.matcher).unwrap();
    let program = finite_lower::lower(&plan).unwrap();
    let layout = lower.memo_layout(true);
    exec::with_terminals(
        vec![(
            tape(g.language, "f(((a)),((b)),((c)));"),
            Default::default(),
        )],
        lower.pure.program.terminals,
        |db, batch, _| {
            super::super::super::leaves::register(db, batch).unwrap();
            let file = &batch.files[0];
            let _scope = matcher::FileScopeSlot::install(Default::default(), None, false);
            let roots = (0..file.facts.len())
                .filter(|id| {
                    matches!(
                        node_at(&file.root, *id).kind().as_ref(),
                        ";" | "identifier" | "parenthesized_expression"
                    )
                })
                .collect::<Vec<_>>();
            let mut total_vm = 0i64;
            let mut executions = 0;
            let mut queries = std::collections::BTreeSet::new();
            let mut prepare_ns = 0;
            let mut execute_ns = 0;
            db.set_prepared_statement_cache_capacity(16);
            for root in roots {
                for dynamic in [false, true] {
                    db.query_row::<i64, _, _>("SELECT relation_memo_reset(0)", [], |r| r.get(0))
                        .unwrap();
                    matcher::with_file_memo(&file.root, || {
                        // Start cold with failures, fill, then replay at small work budgets.
                        for incoming in [254, 255, 256, 0, 253, 254, 0] {
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
                                    ..Default::default()
                                },
                            };
                            let input = Demand {
                                name: sql.relation(
                                    "recursive caller depth input",
                                    "SELECT 1 k,0 file,?3 node,?2 work".into(),
                                    true,
                                    "single reached root",
                                ),
                                slots: Slots::new(),
                            };
                            sql.generation.components.borrow_mut().roots = input.name.clone();
                            let out = sql.finite_program(&program, &input).unwrap();
                            fn shared_recursive_input(v: &serde_json::Value) -> bool {
                                match v {
                                    serde_json::Value::Object(fields) => {
                                        (v["operator"] == "closed recursive caller"
                                            && v["recursiveInputs"] == 1
                                            && v["logicalOccurrences"] == 3)
                                            || fields.values().any(shared_recursive_input)
                                    }
                                    serde_json::Value::Array(values) => {
                                        values.iter().any(shared_recursive_input)
                                    }
                                    _ => false,
                                }
                            }
                            assert!(
                                shared_recursive_input(&json!(sql.profile)),
                                "one producer must retain all three logical occurrences"
                            );
                            sql.publish_components();
                            let query = format!(
                                "WITH RECURSIVE {} SELECT truth,work FROM {}",
                                sql.definitions.join(","),
                                out.name
                            );
                            queries.insert(query.clone());
                            let start = std::time::Instant::now();
                            let mut statement = db.prepare_cached(&query).unwrap();
                            prepare_ns += start.elapsed().as_nanos();
                            for work in [0, 1, 2, 3, 4, 8, 32] {
                                let token: i64 = db
                                    .query_row("SELECT relation_peak_open(0)", [], |r| r.get(0))
                                    .unwrap();
                                let read = |r: &rusqlite::Row<'_>| {
                                    Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
                                };
                                let start = std::time::Instant::now();
                                let actual = statement
                                    .query_row(
                                        [i64::from(incoming), work as i64, root as i64],
                                        read,
                                    )
                                    .unwrap();
                                execute_ns += start.elapsed().as_nanos();
                                total_vm += i64::from(
                                    statement.reset_status(rusqlite::StatementStatus::VmStep),
                                );
                                executions += 1;
                                let peak: i64 = db
                                    .query_row("SELECT relation_peak_close(?1)", [token], |r| {
                                        r.get(0)
                                    })
                                    .unwrap();
                                let expected = matcher::match_at_depth(
                                    g,
                                    node_at(&file.root, root),
                                    incoming,
                                    work,
                                );
                                assert_eq!(
                                    (actual, (peak - i64::from(incoming)).max(0)),
                                    (
                                        (expected.truth, expected.work as i64),
                                        i64::from(expected.peak.saturating_sub(incoming))
                                    ),
                                    "node={root},dynamic={dynamic},incoming={incoming},work={work}"
                                );
                                let leaks:(i64,i64,i64)=db.query_row("SELECT relation_continuation_rows(),relation_state_rows(),relation_state_scopes()",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
                                assert_eq!(leaks, (0, 0, 0));
                            }
                        }
                    });
                }
            }
            assert_eq!(
                queries.len(),
                6,
                "five static depths and one dynamic statement"
            );
            eprintln!(
                "RECURSIVE_CALLER_PROFILE {}",
                json!({"executions":executions,"vm":total_vm,"distinctStatements":queries.len(),"maxSqlBytes":queries.iter().map(String::len).max(),"prepareWallNs":prepare_ns,"executionWallNs":execute_ns})
            );
        },
    );
}
