//! Captured positive hf-callee transfer for the actual imported class fixtures.
//! This child composes SQL-owned attribute fields/API binding with the reached
//! atomic resolver frontier. API is absent on entry; depth is fixed at zero.
use super::*;
use crate::code_rule_dsl::sql::relational::outcomes::emit::bridge;
// hf-callee invokes named-from-pretrained, then segment-transformers on
// the callee ATTRIBUTE. That segment's object arm invokes the identifier
// segment covered by the frontier. These three prefix entries precede it.
const PREFIX_UTILITIES: [&str; 3] = [
    "hf-callee",
    "named-from-pretrained",
    "segment-transformers(attribute)",
];
fn captured(g: &GrammarMatcher) -> Result<Frontier, String> {
    let prefix = PREFIX_UTILITIES.len();
    let mut f = frontier(g)?;
    let api_regex = f.terminals.len();
    f.terminals.push(Terminal::Regex(
        regex::Regex::new("^from_pretrained$").unwrap(),
    ));
    let object_regex = f.terminals.len();
    f.terminals.push(Terminal::Regex(
        regex::Regex::new("^(AutoModel|AutoTokenizer)$").unwrap(),
    ));
    let kinds = g.language.kinds();
    let attribute = kinds.kind_id("attribute");
    let identifier = kinds.kind_id("identifier");
    let attrfield = g.language.field_id("attribute").unwrap();
    let objectfield = g.language.field_id("object").unwrap();
    // These are SQL argument expressions, not terminal rebasing or SQL parsing.
    let child = f
        .query
        .replace("?1", "d.file")
        .replace("?2", "d.object")
        .replace("?3", &format!("MAX(0,d.allowance-{prefix})"))
        .replace(
            "?4",
            &format!("d.reached AND d.remembered IS NULL AND d.allowance>={prefix}"),
        );
    f.query = format!(
        r#"WITH demand AS MATERIALIZED(SELECT ?1 file,?2 node,?3 allowance,?4 reached,?5 incoming_api),
domain AS MATERIALIZED(SELECT d.*,object.id object,api.id api FROM demand d JOIN node n ON n.file=d.file AND n.id=d.node AND n.kind={attribute} JOIN node object ON object.file=n.file AND object.parent=n.id AND object.field={objectfield} AND object.ffirst=1 AND object.kind={identifier} JOIN node api ON api.file=n.file AND api.parent=n.id AND api.field={attrfield} AND api.ffirst=1 AND api.kind={identifier} WHERE d.incoming_api IS NULL AND node_regex({api_regex},api.file,api.id) AND node_regex({object_regex},object.file,object.id)),
lookup AS MATERIALIZED(SELECT d.*,m.truth remembered,m.cost remembered_cost,m.o0 remembered_capture FROM domain d LEFT JOIN relation_memo m ON m.mode IS CASE WHEN d.reached THEN 0 END AND m.clause IS 9100000 AND m.utility IS 2 AND m.file IS d.file AND m.node IS d.node),
body AS MATERIALIZED(SELECT d.*,r.c0 body_truth,r.c1 body_work,r.c4 fill FROM lookup d {}),
answer AS MATERIALIZED(SELECT d.*,CASE WHEN NOT d.reached THEN 0 WHEN d.remembered IS NOT NULL THEN CASE WHEN d.allowance>=d.remembered_cost THEN 1 ELSE -2 END WHEN d.allowance<{prefix} THEN -2 ELSE d.body_truth END truth,CASE WHEN NOT d.reached THEN d.allowance WHEN d.remembered IS NOT NULL THEN MAX(0,d.allowance-d.remembered_cost) WHEN d.allowance<{prefix} THEN 0 ELSE d.body_work END remaining FROM body d)
SELECT d.truth,d.remaining,CASE WHEN d.truth=1 THEN COALESCE(d.remembered_capture,d.api) END capture,d.allowance-d.remaining execution_cost,d.allowance-d.remaining-d.fill replay_cost,d.fill,CASE WHEN NOT d.reached OR d.truth<0 OR d.truth=1 THEN 1 ELSE 0 END qualified FROM answer d LEFT JOIN relation_memo m ON m.mode IS CASE WHEN d.reached AND d.remembered IS NULL AND d.truth=1 THEN 1 END AND m.clause IS 9100000 AND m.utility IS 2 AND m.file IS d.file AND m.node IS d.node AND m.vtruth IS 1 AND m.vcost IS d.allowance-d.remaining-d.fill AND m.vpeak IS 3 AND m.v0 IS d.api"#,
        bridge(&child, 6)
    );
    Ok(f)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::{
        CodeRuleDslRuntime, CompiledCodeRuleDsl, matcher,
        sql::{exec, relational::budget_leaf, tests::differential::tape},
        test_support::{CAPABILITIES, grammar_matchers},
    };
    use ast_grep_core::AstGrep;
    use serde_json::{Value, json};
    fn policy(reject: bool) -> CompiledCodeRuleDsl {
        let mut p: Value = serde_json::from_str(include_str!(
            "../../../../../../packs/python/rules/bandit-b615-huggingface-unsafe-download.json"
        ))
        .unwrap();
        p["clauses"][0]["forbid"]["ast"]["rule"] = if reject {
            json!({"all":[{"regex":"^not_reached$"},{"matches":"hf-callee"}]})
        } else {
            json!({"matches":"hf-callee"})
        };
        p["clauses"][0]["violation"]["message"] = json!("hf helper");
        CodeRuleDslRuntime::compile(p.to_string(), "json".into(), CAPABILITIES.into()).unwrap()
    }
    #[test]
    fn structural_spike_b615_actual_imported_callee_api_work_cold_warm_and_budgets() {
        let mut evidence = vec![];
        let sources = [
            include_str!(
                "../../../../../../packs/python/upstream/bandit/fixtures/huggingface_unsafe_download.py"
            ),
            include_str!(
                "../../../../../../packs/python/upstream/bandit/fixtures/rerail/nosec/B615.py"
            ),
        ];
        for (file, source) in sources.into_iter().enumerate() {
            for reject in [false, true] {
                let p = policy(reject);
                let g = &grammar_matchers(&p)[0];
                let doc = tape(g.language, source);
                let tree = AstGrep::doc(doc.clone());
                let nodes = tree
                    .root()
                    .dfs()
                    .filter(|n| n.kind() == "call")
                    .filter_map(|n| n.field("function"))
                    .filter(|n| {
                        n.kind() == "attribute"
                            && n.field("attribute")
                                .is_some_and(|n| n.text() == "from_pretrained")
                            && n.field("object").is_some_and(|n| {
                                matches!(n.text().as_ref(), "AutoModel" | "AutoTokenizer")
                            })
                    })
                    .collect::<Vec<_>>();
                assert_eq!(nodes.len(), if file == 0 { 13 } else { 6 });
                let _scope =
                    matcher::FileScopeSlot::install(Default::default(), Default::default(), false);
                let expected = matcher::with_file_memo(&tree, || {
                    nodes
                        .iter()
                        .chain(nodes.iter())
                        .map(|n| {
                            let mut left = 10000;
                            let found = g.match_root(n.clone(), &mut left).unwrap();
                            (
                                i64::from(found.is_some()),
                                left as i64,
                                found.map(|f| {
                                    f.get_env().get_match("API").unwrap().node_id() as i64
                                }),
                            )
                        })
                        .collect::<Vec<_>>()
                });
                if !reject {
                    let logical = matcher::with_file_memo(&tree, || {
                        matcher::match_at_depth(g, nodes[0].clone(), 0, 10000)
                    });
                    assert_eq!((logical.truth, logical.peak), (1, 3));
                }
                let f = captured(g).unwrap();
                exec::with_terminals(
                    vec![(doc, Default::default())],
                    f.terminals,
                    |db, batch, _| {
                        crate::code_rule_dsl::sql::relational::leaves::register(db, batch).unwrap();
                        db.execute_batch(&f.pure).unwrap();
                        for ((index, node), native) in
                            nodes.iter().chain(nodes.iter()).enumerate().zip(expected)
                        {
                            let ((truth, left, api, replay, fill, qualified), probes) =
                                budget_leaf::trace(|| {
                                    db.query_row(
                                        &f.query,
                                        rusqlite::params![
                                            0,
                                            node.node_id() as i64,
                                            10000,
                                            i64::from(!reject),
                                            rusqlite::types::Null
                                        ],
                                        |r| {
                                            Ok((
                                                r.get::<_, i64>(0)?,
                                                r.get::<_, i64>(1)?,
                                                r.get::<_, Option<i64>>(2)?,
                                                r.get::<_, i64>(4)?,
                                                r.get::<_, i64>(5)?,
                                                r.get::<_, i64>(6)?,
                                            ))
                                        },
                                    )
                                    .unwrap()
                                });
                            assert_eq!(qualified, 1);
                            assert_eq!(
                                (truth, left, api),
                                native,
                                "file={file} reject={reject} invocation={index}"
                            );
                            if reject {
                                assert_eq!((replay, fill, probes.len()), (0, 0, 0));
                            } else {
                                assert_eq!(replay, 6);
                                if index >= nodes.len() {
                                    assert_eq!(fill, 0);
                                    assert!(probes.is_empty());
                                }
                            }
                            evidence.push(json!({"file":file,"invocation":index,"rejectedBeforeHelper":reject,"truth":truth,"work":left,"API":api,"completedReplayCost":replay,"coldFill":fill,"atomicBindingProbes":probes.len(),"qualified":qualified,"incomingDepth":0,"domain":"original hf-callee positive AutoModel/AutoTokenizer imported-from-transformers branch; absent API input"}));
                        }
                    },
                );
            }
        }
        if let Ok(path) = std::env::var("RERAIL_B615_IMPORTED_CALLEE_SPIKE_EVIDENCE") {
            std::fs::write(path, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
        }
    }
    #[test]
    fn structural_spike_b615_actual_callee_finite_and_outside_domain_refusals() {
        let p = policy(false);
        let g = &grammar_matchers(&p)[0];
        let source = "from transformers import AutoModel\nAutoModel.from_pretrained('model')\n";
        let doc = tape(g.language, source);
        let tree = AstGrep::doc(doc.clone());
        let node = tree
            .root()
            .dfs()
            .find(|n| n.kind() == "call")
            .unwrap()
            .field("function")
            .unwrap();
        let native_call = |allowance: usize| {
            let mut left = allowance;
            let found = g.match_root(node.clone(), &mut left);
            match found {
                Ok(found) => (
                    i64::from(found.is_some()),
                    left as i64,
                    found.map(|f| f.get_env().get_match("API").unwrap().node_id() as i64),
                ),
                Err(limit) => (
                    crate::code_rule_dsl::sql::relational::failure::encode(&format!("{limit:?}"))
                        .unwrap(),
                    left as i64,
                    None,
                ),
            }
        };
        let fill = {
            let _scope =
                matcher::FileScopeSlot::install(Default::default(), Default::default(), false);
            matcher::with_file_memo(&tree, || {
                let (t, w, _) = native_call(10000);
                assert_eq!(t, 1);
                10000 - w - 6
            })
        };
        assert!(fill > 0);
        let mut schedules = vec![];
        for allowance in [0, 1, 2, 3, 4, 5, 6, fill + 5, fill + 6, fill + 7] {
            schedules.push(vec![allowance]);
        }
        for allowance in [0, 1, 2, 3, 4, 5, 6] {
            schedules.push(vec![10000, allowance]);
        }
        schedules.push(vec![fill + 5, 10000, 6]);
        for schedule in schedules {
            let expected = {
                let _scope =
                    matcher::FileScopeSlot::install(Default::default(), Default::default(), false);
                matcher::with_file_memo(&tree, || {
                    schedule
                        .iter()
                        .map(|w| native_call(*w as usize))
                        .collect::<Vec<_>>()
                })
            };
            let f = captured(g).unwrap();
            exec::with_terminals(
                vec![(doc.clone(), Default::default())],
                f.terminals,
                |db, batch, _| {
                    crate::code_rule_dsl::sql::relational::leaves::register(db, batch).unwrap();
                    db.execute_batch(&f.pure).unwrap();
                    for (index, (&work, native)) in schedule.iter().zip(expected).enumerate() {
                        let (row, probes) = budget_leaf::trace(|| {
                            db.query_row(
                                &f.query,
                                rusqlite::params![
                                    0,
                                    node.node_id() as i64,
                                    work,
                                    1,
                                    rusqlite::types::Null
                                ],
                                |r| {
                                    Ok((
                                        r.get::<_, i64>(0)?,
                                        r.get::<_, i64>(1)?,
                                        r.get::<_, Option<i64>>(2)?,
                                        r.get::<_, i64>(6)?,
                                    ))
                                },
                            )
                            .unwrap()
                        });
                        assert_eq!(row.3, 1);
                        assert_eq!(
                            (row.0, row.1, row.2),
                            native,
                            "schedule={schedule:?} index={index}"
                        );
                        if work < 4 || index > 0 && schedule[0] == 10000 {
                            assert!(probes.is_empty());
                        }
                    }
                },
            );
        }
        for source in [
            "from transformers import AutoModel as M\nM.from_pretrained('model')\n",
            "from datasets import load_dataset\nload_dataset('model')\n",
            "from huggingface_hub import hf_hub_download as download\ndownload('model',filename='weights')\n",
        ] {
            let doc = tape(g.language, source);
            let tree = AstGrep::doc(doc.clone());
            let n = tree
                .root()
                .dfs()
                .find(|n| n.kind() == "call")
                .unwrap()
                .field("function")
                .unwrap();
            let _scope =
                matcher::FileScopeSlot::install(Default::default(), Default::default(), false);
            let original = matcher::with_file_memo(&tree, || {
                let mut work = 10000;
                g.match_root(n.clone(), &mut work).unwrap().is_some()
            });
            assert!(
                original,
                "outside-domain refusal must not be mistaken for native false"
            );
            let f = captured(g).unwrap();
            exec::with_terminals(
                vec![(doc, Default::default())],
                f.terminals,
                |db, batch, _| {
                    crate::code_rule_dsl::sql::relational::leaves::register(db, batch).unwrap();
                    db.execute_batch(&f.pure).unwrap();
                    let (count, probes) = budget_leaf::trace(|| {
                        db.query_row(
                            &format!("SELECT COUNT(*) FROM ({})", f.query),
                            rusqlite::params![
                                0,
                                n.node_id() as i64,
                                10000,
                                1,
                                rusqlite::types::Null
                            ],
                            |r| r.get::<_, i64>(0),
                        )
                        .unwrap()
                    });
                    assert_eq!(count, 0);
                    assert!(probes.is_empty());
                },
            );
        }
    }
    fn preserved_original_context() -> crate::code_rule_dsl::matcher::GrammarMatcher {
        let p = CodeRuleDslRuntime::compile(
            include_str!(
                "../../../../../../packs/python/rules/bandit-b615-huggingface-unsafe-download.json"
            )
            .into(),
            "json".into(),
            CAPABILITIES.into(),
        )
        .unwrap();
        let mut g = grammar_matchers(&p)[0].clone();
        assert!(g.three_valued);
        let index = g.utils.names.iter().position(|n| n == "hf-callee").unwrap();
        g.matcher = AstMatcher::Util(matcher::UtilRef {
            table: std::sync::Arc::downgrade(&g.utils),
            index,
        });
        assert!(g.three_valued);
        g
    }
    #[test]
    fn structural_spike_b615_actual_imports_preserve_original_three_context() {
        let sources = [
            include_str!(
                "../../../../../../packs/python/upstream/bandit/fixtures/huggingface_unsafe_download.py"
            ),
            include_str!(
                "../../../../../../packs/python/upstream/bandit/fixtures/rerail/nosec/B615.py"
            ),
        ];
        let mut evidence = vec![];
        for (file, source) in sources.into_iter().enumerate() {
            let g = preserved_original_context();
            let doc = tape(g.language, source);
            let tree = AstGrep::doc(doc.clone());
            let nodes = tree
                .root()
                .dfs()
                .filter(|n| n.kind() == "call")
                .filter_map(|n| n.field("function"))
                .filter(|n| {
                    n.kind() == "attribute"
                        && n.field("attribute")
                            .is_some_and(|n| n.text() == "from_pretrained")
                        && n.field("object").is_some_and(|n| {
                            matches!(n.text().as_ref(), "AutoModel" | "AutoTokenizer")
                        })
                })
                .collect::<Vec<_>>();
            assert_eq!(nodes.len(), if file == 0 { 13 } else { 6 });
            let mut calls = nodes
                .iter()
                .chain(nodes.iter())
                .cloned()
                .map(|n| (n, 10000_usize))
                .collect::<Vec<_>>();
            for work in [0, 1, 5, 6] {
                calls.push((nodes[0].clone(), work));
            }
            let _scope =
                matcher::FileScopeSlot::install(Default::default(), Default::default(), false);
            let (expected, values) =
                crate::code_rule_dsl::matcher::value_probe::run_reached(None, || {
                    matcher::with_file_memo(&tree, || {
                        calls
                            .iter()
                            .map(|(n, allowance)| {
                                let mut left = *allowance;
                                match g.match_root(n.clone(), &mut left) {
                                    Ok(found) => (
                                        i64::from(found.is_some()),
                                        left as i64,
                                        found.map(|f| {
                                            f.get_env().get_match("API").unwrap().node_id() as i64
                                        }),
                                    ),
                                    Err(limit) => (
                                        crate::code_rule_dsl::sql::relational::failure::encode(
                                            &format!("{limit:?}"),
                                        )
                                        .unwrap(),
                                        left as i64,
                                        None,
                                    ),
                                }
                            })
                            .collect::<Vec<_>>()
                    })
                });
            assert!(values.is_empty());
            let f = captured(&g).unwrap();
            exec::with_terminals(
                vec![(doc, Default::default())],
                f.terminals,
                |db, batch, _| {
                    crate::code_rule_dsl::sql::relational::leaves::register(db, batch).unwrap();
                    db.execute_batch(&f.pure).unwrap();
                    for (index, ((node, work), native)) in calls.iter().zip(expected).enumerate() {
                        let (row, probes) = budget_leaf::trace(|| {
                            db.query_row(
                                &f.query,
                                rusqlite::params![
                                    0,
                                    node.node_id() as i64,
                                    *work as i64,
                                    1,
                                    rusqlite::types::Null
                                ],
                                |r| {
                                    Ok((
                                        r.get::<_, i64>(0)?,
                                        r.get::<_, i64>(1)?,
                                        r.get::<_, Option<i64>>(2)?,
                                        r.get::<_, i64>(4)?,
                                        r.get::<_, i64>(5)?,
                                        r.get::<_, i64>(6)?,
                                    ))
                                },
                            )
                            .unwrap()
                        });
                        assert_eq!(
                            (row.0, row.1, row.2),
                            native,
                            "original3v file={file} invocation={index}"
                        );
                        assert_eq!(row.5, 1);
                        if row.0 == 1 {
                            assert_eq!(row.3, 6);
                        }
                        if index == 0 {
                            assert!(row.4 > 0);
                        }
                        if index >= nodes.len() {
                            assert_eq!(row.4, 0);
                            assert!(probes.is_empty());
                        }
                        evidence.push(json!({"file":file,"invocation":index,"ambientThreeValued":g.three_valued,"originalGrammarContextPreserved":true,"allowance":work,"truth":row.0,"remainingWork":row.1,"API":row.2,"replayWork":row.3,"scopeFillWork":row.4,"bindingProbes":probes.len(),"valueProbes":values.len(),"scope":"19 actual imported-class hf-callee demands; direct attributes; absent API input; depth0; aliases/nth outside"}));
                    }
                },
            );
        }
        if let Ok(path) = std::env::var("RERAIL_B615_IMPORTED_CALLEE_SPIKE_EVIDENCE") {
            std::fs::write(
                format!("{path}.original-three.json"),
                serde_json::to_vec_pretty(&evidence).unwrap(),
            )
            .unwrap();
        }
    }
}
