//! Reached atomic binding/cache-fill boundary for actual B615 imported classes.
//! SQL owns the original import/module structural predicate. Scope discovery is
//! Incoming logical depth is fixed at zero; no depth qualification is claimed.
//! the ORIGINAL atomic resolver; its first-fill work never enters a pure memo.
use super::{Op, lower::Lower};
use crate::code_rule_dsl::{
    matcher::{AstMatcher, GrammarMatcher, Semantic},
    sql::terminals::Terminal,
};
use std::collections::HashSet;
fn binding_atoms(m: &AstMatcher, seen: &mut HashSet<(usize, usize)>, found: &mut Vec<AstMatcher>) {
    match m {
        AstMatcher::Semantic(Semantic::Binding(test))
            if test
                .site
                .as_ref()
                .is_some_and(|s| matches!(s, AstMatcher::Kind(_) | AstMatcher::Inside(_, _))) =>
        {
            found.push(m.clone())
        }
        AstMatcher::All(v) | AstMatcher::Any(v) => {
            for m in v {
                binding_atoms(m, seen, found)
            }
        }
        AstMatcher::Not(m) => binding_atoms(m, seen, found),
        AstMatcher::Util(u) if seen.insert((u.table.as_ptr() as usize, u.index)) => {
            let _ = u.with_target(|m| binding_atoms(m, seen, found));
        }
        AstMatcher::Has(r, _)
        | AstMatcher::Inside(r, _)
        | AstMatcher::Follows(r, _)
        | AstMatcher::Precedes(r, _) => binding_atoms(r.rule(), seen, found),
        _ => {}
    }
}
struct Frontier {
    query: String,
    pure: String,
    terminals: Vec<Terminal>,
}
fn frontier(g: &GrammarMatcher) -> Result<Frontier, String> {
    let mut atoms = vec![];
    binding_atoms(&g.matcher, &mut HashSet::new(), &mut atoms);
    let alias=atoms.iter().find(|m|matches!(m,AstMatcher::Semantic(Semantic::Binding(t)) if t.site.as_ref().is_some_and(|s|matches!(s,AstMatcher::Kind(_))))).ok_or("original aliased-import binding atom absent")?;
    let import=atoms.iter().find(|m|matches!(m,AstMatcher::Semantic(Semantic::Binding(t)) if t.site.as_ref().is_some_and(|s|matches!(s,AstMatcher::Inside(_, _))))).ok_or("original module binding atom absent")?;
    let mut lower = Lower::new(g.language, g.three_valued);
    lower.scope_owner = 1;
    let mut terms = vec![];
    for atom in [alias, import] {
        let plan = lower.plan(atom)?;
        let Op::Binding { core, .. } = plan.op else {
            return Err("expected original atomic binding plan".into());
        };
        let Op::BoundLeaf { term, inputs, .. } = core.op else {
            return Err("expected target facts".into());
        };
        if inputs.len() != 4 {
            return Err("unexpected live capture input".into());
        }
        terms.push(term);
    }
    let regex = lower.pure.program.terminals.len();
    lower.pure.program.terminals.push(Terminal::Regex(
        regex::Regex::new("^transformers$").unwrap(),
    ));
    let kinds = g.language.kinds();
    let imports = kinds.kind_id("import_from_statement");
    let identifier = kinds.kind_id("identifier");
    let aliased = kinds.kind_id("aliased_import");
    let module = g
        .language
        .field_id("module_name")
        .ok_or("module_name field absent")?;
    let pure = format!(
        "CREATE TEMP TABLE b615_import_pure AS SELECT i.file,i.id,i.\"last\" FROM node i JOIN node module ON module.file=i.file AND module.parent=i.id AND module.field={module} AND module.ffirst=1 WHERE i.kind={imports} AND EXISTS(SELECT 1 FROM node name WHERE name.file=module.file AND name.id>module.id AND name.id<=module.\"last\" AND name.kind={identifier} AND node_regex({regex},name.file,name.id));CREATE INDEX b615_import_pure_file ON b615_import_pure(file,id);"
    );
    let query = format!(
        r#"WITH input AS MATERIALIZED(SELECT ?1 file,?2 node,?3 allowance,?4 reached,relation_cache_fill_steps() before),
lookup AS MATERIALIZED(SELECT d.*,m.truth remembered,m.cost remembered_cost,m.peak remembered_peak FROM input d LEFT JOIN relation_memo m ON m.mode IS CASE WHEN d.reached THEN 0 END AND m.clause IS 9100000 AND m.utility IS 1 AND m.file IS d.file AND m.node IS d.node),
first AS MATERIALIZED(SELECT d.*,r.truth t1,r.work w1,r.c0 site1 FROM lookup d LEFT JOIN relation_binding_budget r ON r.term={} AND r.file=d.file AND r.node=d.node AND r.allowance=CASE WHEN d.reached AND d.remembered IS NULL AND d.allowance>0 THEN d.allowance-1 END AND r.i0 IS NULL AND r.i1 IS NULL AND r.i2 IS NULL AND r.i3 IS NULL),
first_child AS MATERIALIZED(SELECT d.*,n.kind first_kind,CASE WHEN d.t1=1 AND d.site1 IS NOT NULL THEN CASE WHEN d.w1>0 THEN d.w1-1 ELSE 0 END ELSE d.w1 END after_first FROM first d LEFT JOIN node n ON n.file=d.file AND n.id=d.site1),
second AS MATERIALIZED(SELECT d.*,r.truth t2,r.work w2,r.c0 site2 FROM first_child d LEFT JOIN relation_binding_budget r ON r.term={} AND r.file=d.file AND r.node=d.node AND r.allowance=CASE WHEN d.reached AND d.remembered IS NULL AND d.t1=1 AND d.site1 IS NOT NULL AND d.w1>0 AND d.first_kind<>{aliased} THEN d.after_first END AND r.i0 IS NULL AND r.i1 IS NULL AND r.i2 IS NULL AND r.i3 IS NULL),
answer AS MATERIALIZED(SELECT d.*,CASE WHEN NOT d.reached THEN 0 WHEN d.remembered IS NOT NULL THEN CASE WHEN d.allowance>=d.remembered_cost THEN d.remembered ELSE -2 END WHEN d.allowance=0 THEN -2 WHEN d.t1<0 THEN d.t1 WHEN d.t1=1 AND d.w1=0 THEN -2 WHEN d.t2<0 THEN d.t2 WHEN d.t2=1 AND d.w2=0 THEN -2 WHEN d.t2=1 AND EXISTS(SELECT 1 FROM b615_import_pure i WHERE i.file=d.file AND d.site2>i.id AND d.site2<=i."last") THEN 1 ELSE 0 END truth,CASE WHEN NOT d.reached THEN d.allowance WHEN d.remembered IS NOT NULL THEN MAX(0,d.allowance-d.remembered_cost) WHEN d.allowance=0 THEN 0 WHEN d.t1<0 THEN d.w1 WHEN d.t1=1 AND d.w1=0 THEN 0 WHEN d.t2<0 THEN d.w2 WHEN d.t2=1 THEN MAX(0,d.w2-1) ELSE COALESCE(d.w2,d.after_first,d.allowance-1) END remaining FROM second d),
measured AS MATERIALIZED(SELECT d.*,relation_cache_fill_delta(d.before) fill FROM answer d)
SELECT d.truth,d.remaining,d.allowance-d.remaining execution_cost,d.allowance-d.remaining-d.fill replay_cost,d.fill,CASE WHEN d.reached THEN 1 ELSE 0 END peak FROM measured d LEFT JOIN relation_memo m ON m.mode IS CASE WHEN d.reached AND d.remembered IS NULL AND d.truth=1 THEN 1 END AND m.clause IS 9100000 AND m.utility IS 1 AND m.file IS d.file AND m.node IS d.node AND m.vtruth IS 1 AND m.vcost IS d.allowance-d.remaining-d.fill AND m.vpeak IS 1"#,
        terms[0], terms[1]
    );
    Ok(Frontier {
        query,
        pure,
        terminals: lower.pure.program.terminals,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::{
        CodeRuleDslRuntime, matcher,
        sql::{exec, relational::budget_leaf, tests::differential::tape},
        test_support::{CAPABILITIES, grammar_matchers},
    };
    use ast_grep_core::AstGrep;
    use serde_json::{Value, json};
    fn policy(reject: bool) -> crate::code_rule_dsl::CompiledCodeRuleDsl {
        let mut p: Value = serde_json::from_str(include_str!(
            "../../../../../../packs/python/rules/bandit-b615-huggingface-unsafe-download.json"
        ))
        .unwrap();
        p["clauses"][0]["forbid"]["ast"]["rule"] = if reject {
            json!({"all":[{"regex":"^not_reached$"},{"matches":"segment-transformers"}]})
        } else {
            json!({"matches":"segment-transformers"})
        };
        p["clauses"][0]["violation"]["message"] = json!("binding boundary");
        CodeRuleDslRuntime::compile(p.to_string(), "json".into(), CAPABILITIES.into()).unwrap()
    }
    #[test]
    fn structural_spike_b615_import_binding_cold_fill_warm_replay_and_rejection() {
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
            for reject in [false, true] {
                let p = policy(reject);
                let g = &grammar_matchers(&p)[0];
                let doc = tape(g.language, source);
                let tree = AstGrep::doc(doc.clone());
                let objects = tree
                    .root()
                    .dfs()
                    .filter(|n| n.kind() == "call")
                    .filter_map(|n| n.field("function"))
                    .filter(|n| {
                        n.kind() == "attribute"
                            && n.field("attribute")
                                .is_some_and(|n| n.text() == "from_pretrained")
                    })
                    .filter_map(|n| n.field("object"))
                    .filter(|n| matches!(n.text().as_ref(), "AutoModel" | "AutoTokenizer"))
                    .collect::<Vec<_>>();
                assert_eq!(objects.len(), if file == 0 { 13 } else { 6 });
                let _scope =
                    matcher::FileScopeSlot::install(Default::default(), Default::default(), false);
                let mut expected = vec![];
                matcher::with_file_memo(&tree, || {
                    for node in objects.iter().chain(objects.iter()) {
                        let mut work = 10000;
                        let matched = g.match_root(node.clone(), &mut work).unwrap().is_some();
                        expected.push((i64::from(matched), work as i64));
                    }
                });
                let f = frontier(g).unwrap();
                exec::with_terminals(
                    vec![(doc, Default::default())],
                    f.terminals,
                    |db, batch, _| {
                        crate::code_rule_dsl::sql::relational::leaves::register(db, batch).unwrap();
                        db.execute_batch(&f.pure).unwrap();
                        let mut cold_fill = 0;
                        let mut total_probes = 0;
                        for ((index, node), (truth, work)) in objects
                            .iter()
                            .chain(objects.iter())
                            .enumerate()
                            .zip(expected)
                        {
                            let ((actual, left, cost, replay, fill), probes) =
                                budget_leaf::trace(|| {
                                    db.query_row(
                                        &f.query,
                                        rusqlite::params![
                                            0,
                                            node.node_id() as i64,
                                            10000,
                                            i64::from(!reject)
                                        ],
                                        |r| {
                                            Ok((
                                                r.get::<_, i64>(0)?,
                                                r.get::<_, i64>(1)?,
                                                r.get::<_, i64>(2)?,
                                                r.get::<_, i64>(3)?,
                                                r.get::<_, i64>(4)?,
                                            ))
                                        },
                                    )
                                    .unwrap()
                                });
                            assert_eq!(
                                (actual, left),
                                (truth, work),
                                "file={file} reject={reject} index={index}"
                            );
                            if reject {
                                assert_eq!((cost, fill, probes.len()), (0, 0, 0));
                            } else {
                                assert_eq!(actual, 1);
                                assert_eq!(replay, 3);
                                if index == 0 {
                                    assert!(fill > 0);
                                    cold_fill = fill;
                                } else {
                                    assert_eq!(fill, 0);
                                }
                                if index >= objects.len() {
                                    assert!(probes.is_empty());
                                }
                            }
                            total_probes += probes.len();
                            evidence.push(json!({"file":file,"priorRejection":reject,"invocation":index,"node":node.node_id(),"truth":actual,"work":left,"executionCost":cost,"replayCost":replay,"scopeFill":fill,"atomicBindingProbes":probes.len(),"pureProducer":"one batch import/module interval relation; no scope work in shared result"}));
                        }
                        if !reject {
                            assert!(cold_fill > 0);
                            assert_eq!(total_probes, objects.len() * 2);
                        }
                    },
                );
            }
        }
        if let Ok(path) = std::env::var("RERAIL_B615_BINDING_BOUNDARY_SPIKE_EVIDENCE") {
            std::fs::write(path, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
        }
    }
    #[test]
    fn structural_spike_b615_binding_finite_fill_and_replay_boundaries() {
        let p = policy(false);
        let g = &grammar_matchers(&p)[0];
        let source = include_str!(
            "../../../../../../packs/python/upstream/bandit/fixtures/huggingface_unsafe_download.py"
        );
        let doc = tape(g.language, source);
        let tree = AstGrep::doc(doc.clone());
        let node = tree
            .root()
            .dfs()
            .find(|n| {
                n.kind() == "identifier"
                    && n.text() == "AutoModel"
                    && n.parent().is_some_and(|p| p.kind() == "attribute")
            })
            .unwrap();
        let native_call = |allowance: usize| {
            let mut left = allowance;
            let truth = match g.match_root(node.clone(), &mut left) {
                Ok(Some(_)) => 1,
                Ok(None) => 0,
                Err(limit) => {
                    crate::code_rule_dsl::sql::relational::failure::encode(&format!("{limit:?}"))
                        .unwrap()
                }
            };
            (truth, left as i64)
        };
        let fill = {
            let _scope =
                matcher::FileScopeSlot::install(Default::default(), Default::default(), false);
            matcher::with_file_memo(&tree, || {
                let (truth, left) = native_call(10000);
                assert_eq!(truth, 1);
                10000 - left - 3
            })
        };
        assert!(fill > 0);
        let mut schedules = vec![];
        for allowance in [0, 1, 2, 3, fill + 2, fill + 3, fill + 4] {
            schedules.push(vec![allowance]);
        }
        for allowance in [0, 1, 2, 3, 4] {
            schedules.push(vec![10000, allowance]);
        }
        schedules.push(vec![fill + 2, 10000, 3]);
        let mut evidence = vec![];
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
            let f = frontier(g).unwrap();
            exec::with_terminals(
                vec![(doc.clone(), Default::default())],
                f.terminals,
                |db, batch, _| {
                    crate::code_rule_dsl::sql::relational::leaves::register(db, batch).unwrap();
                    db.execute_batch(&f.pure).unwrap();
                    for (index, (&allowance, native)) in schedule.iter().zip(expected).enumerate() {
                        let ((truth, left, observed_fill), probes) = budget_leaf::trace(|| {
                            db.query_row(
                                &f.query,
                                rusqlite::params![0, node.node_id() as i64, allowance, 1],
                                |r| {
                                    Ok((
                                        r.get::<_, i64>(0)?,
                                        r.get::<_, i64>(1)?,
                                        r.get::<_, i64>(4)?,
                                    ))
                                },
                            )
                            .unwrap()
                        });
                        assert_eq!((truth, left), native, "schedule={schedule:?} index={index}");
                        if index > 0 && schedule[0] == 10000 {
                            assert_eq!(observed_fill, 0);
                            assert!(probes.is_empty());
                        }
                        if allowance == 0 {
                            assert!(probes.is_empty());
                        }
                        evidence.push(json!({"schedule":schedule,"invocation":index,"allowance":allowance,"truth":truth,"work":left,"scopeFill":observed_fill,"atomicBindingProbes":probes.len(),"incomingDepth":0,"measuredColdScopeFill":fill}));
                    }
                },
            );
        }
        if let Ok(path) = std::env::var("RERAIL_B615_BINDING_BOUNDARY_SPIKE_EVIDENCE") {
            std::fs::write(
                format!("{path}.finite.json"),
                serde_json::to_vec_pretty(&evidence).unwrap(),
            )
            .unwrap();
        }
    }
}

#[path = "b615_import_callee_spike.rs"]
mod import_callee;
