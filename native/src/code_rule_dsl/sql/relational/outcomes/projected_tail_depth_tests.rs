use super::super::{Slots, lower::Lower, publication_tests::policy_utils};
use super::{Demand, Generation, Sql};
use crate::code_rule_dsl::{
    matcher::{self, node_at},
    sql::{exec, tests::differential::tape},
    test_support::grammar_matchers,
};
use serde_json::json;
#[test]
fn projected_tail_and_metadata_prefixes_match_native_depth_work_and_peak() {
    let p = policy_utils(
        &[json!({"matches":"outer"})],
        json!({
            "outer":{"any":[{"matches":"left"},{"matches":"right"},{"kind":"parenthesized_expression","has":{"stopBy":"neighbor","matches":"outer"}}]},
            "left":{"kind":"identifier","regex":"^a$"},
            "right":{"kind":"identifier","matches":"leaf"},
            "leaf":{"kind":"identifier","regex":"^b$"}
        }),
    );
    let g = &grammar_matchers(&p)[0];
    let mut lower = Lower::new(g.language, g.three_valued);
    let plan = lower.plan(&g.matcher).unwrap();
    assert!(matches!(&plan.op,super::super::Op::PureRecursive(p) if p.projection.is_some()));
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
                .filter(|i| {
                    let n = node_at(&file.root, *i);
                    n.kind() == "parenthesized_expression" || n.text() == "a" || n.text() == ";"
                })
                .take(4)
                .collect::<Vec<_>>();
            for root in roots {
                for dynamic in [false, true] {
                    for incoming in [0, 252, 254, 255, 256] {
                        for work in [0, 1, 2, 3, 4, 8, 32] {
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
                                    "depth oracle input",
                                    format!("SELECT 1 k,0 file,{root} node,{work} work"),
                                    true,
                                    "single reached input",
                                ),
                                slots: Slots::new(),
                            };
                            sql.generation.components.borrow_mut().roots = input.name.clone();
                            let out = sql.emit(&plan, &input).unwrap();
                            sql.publish_components();
                            let query = format!(
                                "WITH RECURSIVE {} SELECT truth,work FROM {}",
                                sql.definitions.join(","),
                                out.name
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
                            let expected = matcher::with_file_memo(&file.root, || {
                                matcher::match_at_depth(
                                    g,
                                    node_at(&file.root, root),
                                    incoming,
                                    work,
                                )
                            });
                            assert_eq!(
                                (actual, peak),
                                (
                                    (expected.truth, expected.work as i64),
                                    i64::from(expected.peak)
                                ),
                                "node={root},dynamic={dynamic},incoming={incoming},work={work}"
                            );
                        }
                    }
                }
            }
        },
    );
}
