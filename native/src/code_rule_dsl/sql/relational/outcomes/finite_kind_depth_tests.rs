//! Original matcher oracle for complete metadata proofs and retained failed prefixes.
use super::super::{Slots, finite_lower, lower::Lower, publication_tests::policy_utils};
use super::{Demand, Generation, Sql};
use crate::code_rule_dsl::{
    matcher::{self, node_at},
    sql::{exec, tests::differential::tape},
    test_support::grammar_matchers,
};
use serde_json::json;
#[test]
fn metadata_utility_segments_match_native_depth_peak_and_warm_replay() {
    let policy = policy_utils(
        &[json!({"matches":"outer"})],
        json!({
            "outer":{"any":[{"matches":"left"},{"matches":"right"}]},
            "left":{"any":[{"kind":"number","value":{"gt":0}},{"kind":"parenthesized_expression","has":{"stopBy":"neighbor","matches":"left"}}]},
            "right":{"any":[{"kind":"number","value":{"eq":0}},{"kind":"parenthesized_expression","has":{"stopBy":"neighbor","matches":"right"}}]}
        }),
    );
    let g = &grammar_matchers(&policy)[0];
    let mut lower = Lower::new_finite(g.language, g.three_valued);
    let plan = lower.plan(&g.matcher).unwrap();
    let program = finite_lower::lower(&plan).unwrap();
    let layout = lower.memo_layout(true);
    exec::with_terminals(
        vec![(tape(g.language, "f(((a)),b);"), Default::default())],
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
            for root in roots {
                for dynamic in [false, true] {
                    db.query_row::<i64, _, _>("SELECT relation_memo_reset(0)", [], |r| r.get(0))
                        .unwrap();
                    matcher::with_file_memo(&file.root, || {
                        // Start cold with failures, fill, then replay at small work budgets.
                        for incoming in [254, 255, 256, 0, 253, 254, 0] {
                            for work in [0, 1, 2, 3, 4, 8, 32] {
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
                                        "metadata depth input",
                                        format!("SELECT 1 k,0 file,{root} node,{work} work"),
                                        true,
                                        "single reached root",
                                    ),
                                    slots: Slots::new(),
                                };
                                sql.generation.components.borrow_mut().roots = input.name.clone();
                                let out = sql.finite_program(&program, &input).unwrap();
                                assert!(
                                    serde_json::to_string(&sql.profile)
                                        .unwrap()
                                        .contains("metadata utility segment")
                                );
                                sql.publish_components();
                                let query = format!(
                                    "WITH RECURSIVE {} SELECT truth,work FROM {}",
                                    sql.definitions.join(","),
                                    out.name
                                );
                                let token: i64 = db
                                    .query_row("SELECT relation_peak_open(0)", [], |r| r.get(0))
                                    .unwrap();
                                let read = |r: &rusqlite::Row<'_>| {
                                    Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
                                };
                                let actual = if dynamic {
                                    db.query_row(&query, [incoming], read)
                                } else {
                                    db.query_row(&query, [], read)
                                }
                                .unwrap();
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
        },
    );
}
