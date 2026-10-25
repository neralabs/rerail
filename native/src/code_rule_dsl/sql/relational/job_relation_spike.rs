//! Bounded statement packing, not cross-rule work sharing. Terminals receive
//! final IDs before lowering; each closed MATERIALIZED branch has private memo
//! and scope IDs. All files remain in every statement's batch.

use super::job::{Arm, Pack};
use crate::code_rule_dsl::matcher::{self, GrammarMatcher};
use crate::code_rule_dsl::sql::terminals::Terminal;
use serde_json::{Value, json};
fn pack(grammars: &[&GrammarMatcher]) -> Result<Vec<Pack>, String> {
    super::job::compile(grammars, &super::planner_stats::PlannerStats::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::{
        CodeRuleDslRuntime, CompiledCodeRuleDsl,
        matcher::Stopped,
        soundness::SyntaxErrors,
        sql::{
            exec,
            relational::{
                cutover_tests::execution::{Fixture, fixtures},
                tests::ExecutionTrace,
            },
            tests::differential::tape,
        },
        tape::TapeDoc,
        test_support::CAPABILITIES,
    };
    use ast_grep_core::{
        AstGrep,
        meta_var::{MetaVarEnv, MetaVariable},
    };
    use std::collections::{BTreeMap, BTreeSet};
    fn captures(env: &MetaVarEnv<'_, TapeDoc>, names: &[String]) -> Result<Value, String> {
        let lists = env
            .get_matched_variables()
            .filter_map(|v| {
                if let MetaVariable::MultiCapture(name) = v {
                    Some(name)
                } else {
                    None
                }
            })
            .collect::<BTreeSet<_>>();
        let mut values = serde_json::Map::new();
        for name in names {
            let value = if lists.contains(name) {
                json!(
                    env.get_multiple_matches(name)
                        .iter()
                        .map(|n| n.node_id() as u32)
                        .collect::<Vec<_>>()
                )
            } else if let Some(n) = env.get_match(name) {
                json!(n.node_id() as u32)
            } else if let (Some(text), Some(span)) = (
                matcher::exported_text(env, name),
                matcher::exported_span(env, name),
            ) {
                json!({"text":text,"span":[span.start,span.end]})
            } else if let Some(bytes) = env.get_transformed(name) {
                json!(String::from_utf8(bytes.to_vec()).map_err(|e| e.to_string())?)
            } else {
                Value::Null
            };
            values.insert(name.clone(), value);
        }
        Ok(Value::Object(values))
    }
    fn validate(
        grammars: &[&GrammarMatcher],
        packs: Vec<Pack>,
        batch: &[Fixture],
    ) -> Result<Value, String> {
        validate_at_work(grammars, packs, batch, i64::MAX)
    }
    fn validate_at_work(
        grammars: &[&GrammarMatcher],
        packs: Vec<Pack>,
        batch: &[Fixture],
        allowance: i64,
    ) -> Result<Value, String> {
        validate_observed(grammars, packs, batch, allowance, false)
    }
    fn validate_observed(
        grammars: &[&GrammarMatcher],
        packs: Vec<Pack>,
        batch: &[Fixture],
        allowance: i64,
        probes: bool,
    ) -> Result<Value, String> {
        let mut native_probes = vec![];
        let mut sql_probes = vec![];
        let mut expected = vec![];
        let mut actual = vec![];
        let mut reports = vec![];
        let mut all_names = BTreeMap::new();
        for pack in &packs {
            for arm in &pack.arms {
                all_names.insert(arm.clause, arm.names.clone());
            }
        }
        for (clause, g) in grammars.iter().enumerate() {
            for (file, fixture) in batch.iter().enumerate() {
                let doc = tape(g.language, &fixture.source);
                let tree = AstGrep::doc(doc.clone());
                let errors = SyntaxErrors::of_doc(&doc);
                let _scopes = matcher::FileScopeSlot::install(
                    Default::default(),
                    None,
                    errors.hide_bindings(),
                )
                .path(&fixture.path);
                let mut provisional = vec![];
                let mut work = usize::try_from(allowance).map_err(|_| "negative allowance")?;
                let mut run = || -> Result<Result<(), Stopped>, String> {
                    if g.applies && g.file.as_ref().is_none_or(|c| c.holds(&fixture.path, None)) {
                        matcher::with_file_memo(&tree, || {
                            matcher::run_clause(
                                &tree,
                                g,
                                None,
                                doc.transducer_len(),
                                &mut work,
                                |node, env, left| {
                                    provisional.push(json!([
                                        clause,
                                        file,
                                        node,
                                        captures(&env, &all_names[&clause])?,
                                        0,
                                        *left
                                    ]));
                                    Ok::<_, String>(true)
                                },
                            )
                        })
                    } else {
                        Ok(Ok(()))
                    }
                };
                let result = if probes {
                    let (result, calls) = matcher::value_probe::run_reached(None, run);
                    native_probes.extend(calls);
                    result
                } else {
                    run()
                }?;
                match result {
                    Ok(()) => expected.extend(provisional),
                    Err(Stopped::Limit(limit)) => {
                        let tag = super::super::failure::encode(&format!("{limit:?}"))
                            .ok_or("unknown native limit")?;
                        expected.push(json!([clause, file, null, {}, tag, work]));
                    }
                    Err(Stopped::Deadline) => return Err("native deadline".into()),
                }
            }
        }
        for pack in packs {
            if pack.sql.len() > 256_000 {
                return Err("oversized matching statement".into());
            }
            let language = grammars[0].language;
            if grammars.iter().any(|g| g.language != language) {
                return Err("mixed language file tape requires distinct host batches".into());
            }
            let docs = batch
                .iter()
                .map(|f| {
                    (
                        tape(language, &f.source),
                        exec::FileContext {
                            path: Some(f.path.clone()),
                            ..Default::default()
                        },
                    )
                })
                .collect();
            let execute = || {
                exec::with_terminals(
                    docs,
                    pack.terminals,
                    |db, batch, _| -> Result<Value, String> {
                        super::super::leaves::register(db, batch).map_err(|e| e.to_string())?;
                        let stages = super::super::spike_stages::run(
                            db,
                            &json!({"physicalStages":pack.stages}),
                            &pack.sql,
                        )?;
                        let mut stmt =
                            db.prepare(&pack.sql).map_err(|e| format!("prepare: {e}"))?;
                        let trace = ExecutionTrace::start(db);
                        {
                            let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
                            while let Some(row) = rows.next().map_err(|e| e.to_string())? {
                                let clause =
                                    row.get::<_, u32>(0).map_err(|e| e.to_string())? as usize;
                                let file: u32 = row.get(1).map_err(|e| e.to_string())?;
                                let node: Option<u32> = row.get(2).map_err(|e| e.to_string())?;
                                let payload: String = row.get(3).map_err(|e| e.to_string())?;
                                let failure: i64 = row.get(4).map_err(|e| e.to_string())?;
                                let remaining: i64 = row.get(5).map_err(|e| e.to_string())?;
                                let values: Vec<Value> =
                                    serde_json::from_str(&payload).map_err(|e| e.to_string())?;
                                let mut captures = serde_json::Map::new();
                                if failure < 0 {
                                    if !values.is_empty() {
                                        return Err("failed file exported capture".into());
                                    }
                                } else {
                                    let names = all_names.get(&clause).ok_or("foreign clause")?;
                                    if values.len() != names.len() {
                                        return Err("capture width".into());
                                    }
                                    captures.extend(names.iter().cloned().zip(values));
                                }
                                actual.push(json!([
                                    clause, file, node, captures, failure, remaining
                                ]));
                            }
                        }
                        let executions = trace.count();
                        drop(trace);
                        if executions != 1 {
                            return Err(format!(
                                "expected one matching statement, got {executions}"
                            ));
                        }
                        Ok(
                            json!({"bytes":pack.sql.len(),"arms":pack.arms.len(),"matchingExecutions":executions+stages.executions,"physicalStages":stages.profile(),"vmSteps":i64::from(stmt.get_status(rusqlite::StatementStatus::VmStep))+stages.vm,"allFiles":batch.files.len()}),
                        )
                    },
                )
            };
            let report = if probes {
                let (report, calls) = matcher::value_probe::run_reached(None, execute);
                sql_probes.extend(calls);
                report
            } else {
                execute()
            }?;
            reports.push(report);
        }
        let sort = |rows: &mut Vec<Value>| {
            rows.sort_by_key(|r| (r[0].as_u64(), r[1].as_u64(), r[2].as_u64(), r[4].as_i64()))
        };
        sort(&mut expected);
        sort(&mut actual);
        let difference = expected
            .iter()
            .zip(&actual)
            .position(|(a, b)| a != b)
            .or_else(|| {
                (expected.len() != actual.len()).then_some(expected.len().min(actual.len()))
            });
        let probe_parity = !probes || native_probes == sql_probes;
        let executions: u64 = reports
            .iter()
            .map(|r| r["matchingExecutions"].as_u64().unwrap())
            .sum();
        if executions > 100 {
            return Err(format!(
                "job exceeded hard execution budget: {executions}/100"
            ));
        }
        let mut report = json!({"passed":difference.is_none() && probe_parity,"grammars":grammars.len(),"files":batch.len(),"matchingExecutions":executions,"statements":reports,"nativeRows":expected.len(),"sqlRows":actual.len(),"difference":difference.map(|i|json!({"index":i,"expected":expected.get(i),"actual":actual.get(i)})),"contract":"work-aware statement packing; native outputs and published remaining work; no cross-rule VM sharing claim"});
        if probes {
            report["nativeValueProbes"] = json!(native_probes);
            report["sqlValueProbes"] = json!(sql_probes);
            report["probeParity"] = json!(probe_parity);
            report["nativeWorkRows"] = json!(
                expected
                    .iter()
                    .map(|r| json!([r[0], r[4], r[5]]))
                    .collect::<Vec<_>>()
            );
        }
        Ok(report)
    }
    fn all_grammars(policy: &CompiledCodeRuleDsl) -> Vec<&GrammarMatcher> {
        use crate::code_rule_dsl::model::{ClauseKind, Query};
        policy
            .inner
            .clauses
            .iter()
            .flat_map(|clause| match &clause.kind {
                ClauseKind::Forbid(Query::Ast { matchers, .. }) => {
                    matchers.iter().collect::<Vec<_>>()
                }
                _ => vec![],
            })
            .collect()
    }
    fn catalog() -> Vec<CompiledCodeRuleDsl> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap();
        let mut paths = std::fs::read_dir(root.join("packs/python/rules"))
            .unwrap()
            .map(|p| p.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "json"))
            .collect::<Vec<_>>();
        paths.sort();
        paths
            .into_iter()
            .map(|p| {
                CodeRuleDslRuntime::compile(
                    std::fs::read_to_string(p).unwrap(),
                    "json".into(),
                    CAPABILITIES.into(),
                )
                .unwrap()
            })
            .collect()
    }
    #[test]
    fn structural_spike_job_full_catalog_packed_native_parity() {
        let policies = catalog();
        let grammars = policies.iter().flat_map(all_grammars).collect::<Vec<_>>();
        assert_eq!(grammars.len(), 146);
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap();
        let batch = fixtures(root);
        let language = grammars[0].language;
        let indexed = exec::Batch::new(
            batch
                .iter()
                .map(|f| (tape(language, &f.source), Default::default()))
                .collect(),
            vec![],
        );
        let stats = super::super::planner_stats::PlannerStats::from_batch(&indexed, language);
        let packs = super::super::job::compile(&grammars, &stats).unwrap();
        assert!(packs.len() <= 100);
        let report = validate(&grammars, packs, &batch).unwrap();
        if let Ok(path) = std::env::var("RERAIL_JOB_RELATION_SPIKE_EVIDENCE") {
            std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        }
        assert_eq!(report["passed"], true, "{report}");
        assert!(report["matchingExecutions"].as_u64().unwrap() <= 100);
    }
    #[test]
    fn structural_spike_job_terminal_memo_and_scope_namespaces_do_not_collide() {
        // The full catalog gate covers all namespace interactions. This focused
        // control uses explicit different regex/Value meanings at local terminal0.
        let rules = [
            json!({"kind":"identifier","regex":"^alpha$"}),
            json!({"kind":"identifier","regex":"^beta$"}),
            json!({"kind":"integer","value":{"gt":2}}),
            json!({"kind":"integer","value":{"lt":2}}),
            json!({"kind":"integer","matches":"u"}),
            json!({"kind":"integer","matches":"u"}),
        ];
        let mut controls = vec![];
        for (i, rule) in rules.iter().enumerate() {
            controls.push(CodeRuleDslRuntime::compile(json!({"format":"rerail.policy/v1","id":format!("namespace{i}"),"revision":1,"mode":"steer","clauses":[{"id":"c","forbid":{"ast":{"language":"python","rule":rule,"utils":{"u":{"value":if i%2==0 {json!({"gt":2})} else {json!({"lt":2})}}}}},"violation":{"code":"C","message":"match","anchor":"witness.span"}}]}).to_string(),"json".into(),CAPABILITIES.into()).unwrap());
        }
        let grammars = controls.iter().flat_map(all_grammars).collect::<Vec<_>>();
        let batch = vec![Fixture {
            path: "collision.py".into(),
            source: "alpha\nbeta\n1\n3\n".into(),
        }];
        let report = validate(&grammars, pack(&grammars).unwrap(), &batch).unwrap();
        assert_eq!(report["passed"], true, "{report}");
    }
    #[test]
    fn structural_spike_job_closed_pure_producer_shared_across_rules() {
        use crate::code_rule_dsl::language::GrammarDialect;
        use std::sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
        };
        let source = "alpha\nbeta\nother\n";
        let doc = tape(GrammarDialect::Python, source);
        let kind = GrammarDialect::Python.kinds().kind_id("identifier");
        let counter = Arc::new(AtomicU64::new(0));
        let mut results = vec![];
        let mut traffic = vec![];
        for shared in [false, true] {
            let counter = counter.clone();
            counter.store(0, Ordering::Relaxed);
            let terms = ["^alpha$", "^beta$"]
                .into_iter()
                .map(|r| Terminal::Regex(regex::Regex::new(r).unwrap()))
                .collect();
            let (rows, visits) = exec::with_terminals(
                vec![(doc.clone(), exec::FileContext::default())],
                terms,
                |db, _, _| {
                    let observed = counter.clone();
                    db.create_scalar_function(
                        "job_producer_visit",
                        1,
                        rusqlite::functions::FunctionFlags::SQLITE_UTF8,
                        move |ctx| {
                            observed.fetch_add(1, Ordering::Relaxed);
                            ctx.get::<i64>(0)
                        },
                    )
                    .unwrap();
                    let body = format!(
                        "SELECT n.file,n.id node,job_producer_visit(n.id) visited FROM node n WHERE n.kind={kind}"
                    );
                    let declarations = if shared {
                        format!("producer AS MATERIALIZED ({body})")
                    } else {
                        format!(
                            "producer0 AS MATERIALIZED ({body}),producer1 AS MATERIALIZED ({body})"
                        )
                    };
                    let p0 = if shared { "producer" } else { "producer0" };
                    let p1 = if shared { "producer" } else { "producer1" };
                    let sql = format!(
                        "WITH {declarations} SELECT 0 clause,p.file,p.node FROM {p0} p WHERE p.visited>=0 AND node_regex(0,p.file,p.node) UNION ALL SELECT 1,p.file,p.node FROM {p1} p WHERE p.visited>=0 AND node_regex(1,p.file,p.node) ORDER BY clause,file,node"
                    );
                    let rows = db
                        .prepare(&sql)
                        .unwrap()
                        .query_map([], |r| {
                            Ok([
                                r.get::<_, i64>(0)?,
                                r.get::<_, i64>(1)?,
                                r.get::<_, i64>(2)?,
                            ])
                        })
                        .unwrap()
                        .collect::<Result<Vec<_>, _>>()
                        .unwrap();
                    (rows, counter.load(Ordering::Relaxed))
                },
            );
            results.push(rows);
            traffic.push(visits);
        }
        assert_eq!(results[0], results[1]);
        assert_eq!(results[0].len(), 2);
        assert_eq!(traffic[0], traffic[1] * 2);
        assert_eq!(traffic[1], 3);
    }
    fn cache_policy(rules: &[Value]) -> CompiledCodeRuleDsl {
        let clauses=rules.iter().enumerate().map(|(i,rule)|json!({"id":format!("c{i}"),"forbid":{"ast":{"language":"python","rule":rule}},"violation":{"code":"C","message":"match","anchor":"witness.span"}})).collect::<Vec<_>>();
        CodeRuleDslRuntime::compile(json!({"format":"rerail.policy/v1","id":"parse-cache-owner","revision":1,"mode":"steer","clauses":clauses}).to_string(),"json".into(),CAPABILITIES.into()).unwrap()
    }
    fn finite_control(grammars: &[&GrammarMatcher], work: i64) -> Pack {
        let compiled = super::super::outcomes::compile_job(grammars, work).unwrap();
        let arms = grammars
            .iter()
            .enumerate()
            .map(|(clause, g)| Arm {
                clause,
                sql: String::new(),
                columns: 6,
                names: matcher::CaptureAnalysis::default()
                    .binds(&g.matcher)
                    .into_iter()
                    .collect(),
            })
            .collect();
        Pack {
            stages: compiled.relations["physicalStages"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|s| s.as_str())
                .map(str::to_owned)
                .collect(),
            sql: compiled.sql,
            terminals: compiled.terminals,
            arms,
        }
    }
    #[test]
    fn structural_spike_job_parse_first_fill_is_cold_per_owner_and_warm_within_owner() {
        let parse = json!({"grammar":"percentFormat","has":{"kind":"placeholder"}});
        let single = json!({"kind":"string","pattern":"$S","parse":parse});
        let repeated =
            json!({"kind":"string","pattern":"$S","all":[{"parse":parse},{"parse":parse}]});
        let policy = cache_policy(&[single, repeated]);
        let grammars = all_grammars(&policy);
        let batch = vec![Fixture {
            path: "cache.py".into(),
            source: "s = '%(key)s'\n".into(),
        }];
        // Within an owner, a repeated direct parse has no second fill. Across
        // owners, both rows must retain the same cold first-fill work.
        let native_left = grammars
            .iter()
            .map(|g| {
                let doc = tape(g.language, &batch[0].source);
                let tree = AstGrep::doc(doc.clone());
                let _scope = matcher::FileScopeSlot::install(Default::default(), None, false);
                let mut left = i64::MAX as usize;
                let mut published = vec![];
                matcher::with_file_memo(&tree, || {
                    matcher::run_clause(
                        &tree,
                        g,
                        None,
                        doc.transducer_len(),
                        &mut left,
                        |_, _, work| {
                            published.push(*work);
                            Ok::<_, String>(true)
                        },
                    )
                })
                .unwrap()
                .unwrap_or_else(|_| panic!("native repeated-parse control exceeded its allowance"));
                assert_eq!(published.len(), 1);
                published[0]
            })
            .collect::<Vec<_>>();
        assert_eq!(native_left[0], native_left[1]);
        assert!(native_left[0] < (i64::MAX as usize));
        for allowance in [0, 1, 2, 4, 8, 16, 32, 1000] {
            let report = validate_at_work(
                &grammars,
                vec![finite_control(&grammars, allowance)],
                &batch,
                allowance,
            )
            .unwrap();
            assert_eq!(report["passed"], true, "allowance={allowance}: {report}");
        }
        let report = validate(&grammars, pack(&grammars).unwrap(), &batch).unwrap();
        assert_eq!(report["passed"], true, "{report}");
    }
    #[test]
    fn structural_spike_job_format_analysis_cannot_reuse_another_clause_fill() {
        let format = json!({"style":"percent","string":"S","args":"R","unused":{"regex":".*"}});
        let single = json!({"pattern":"$S % $R","formatArgs":format});
        let repeated =
            json!({"pattern":"$S % $R","all":[{"formatArgs":format},{"formatArgs":format}]});
        let policy = cache_policy(&[single, repeated]);
        let grammars = all_grammars(&policy);
        let batch = vec![Fixture {
            path: "formats.py".into(),
            source: "'%s' % (1,2)\n'%(x)s' % {'y':3}\n'%' % 4\n".into(),
        }];
        for allowance in [0, 1, 2, 4, 8, 16, 32, 1000] {
            let report = validate_at_work(
                &grammars,
                vec![finite_control(&grammars, allowance)],
                &batch,
                allowance,
            )
            .unwrap();
            assert_eq!(report["passed"], true, "allowance={allowance}: {report}");
        }
    }
    fn repeated_original_reading(g: &GrammarMatcher) -> GrammarMatcher {
        fn atom(m: &matcher::AstMatcher) -> Option<&matcher::AstMatcher> {
            match m {
                matcher::AstMatcher::Semantic(
                    matcher::Semantic::Value(_) | matcher::Semantic::FormatArgs(_),
                ) => Some(m),
                matcher::AstMatcher::All(items) => items.iter().find_map(atom),
                _ => None,
            }
        }
        let mut repeated = g.clone();
        repeated.matcher = matcher::AstMatcher::All(vec![
            g.matcher.clone(),
            atom(&g.matcher).expect("reading atom").clone(),
        ]);
        repeated
    }
    #[test]
    fn structural_spike_job_cloned_scalar_reading_is_cold_across_owners_warm_within_owner() {
        let policy = cache_policy(&[json!({"kind":"integer","pattern":"$X","value":{"gt":0}})]);
        let original = all_grammars(&policy)[0];
        let first = repeated_original_reading(original);
        let second = first.clone(); // Reading::clone deliberately preserves cache_id.
        let grammars = vec![&first, &second];
        let batch = vec![Fixture {
            path: "reading.py".into(),
            source: "7\n".into(),
        }];
        for allowance in [0, 1, 2, 8] {
            let report = validate_observed(
                &grammars,
                vec![finite_control(&grammars, allowance)],
                &batch,
                allowance,
                true,
            )
            .unwrap();
            assert_eq!(report["passed"], true, "allowance={allowance}: {report}");
            assert_eq!(report["probeParity"], true);
            assert_eq!(
                report["nativeValueProbes"].as_array().unwrap().len(),
                if allowance == 0 { 2 } else { 4 }
            );
            // At allowance1, both owner rows publish remaining0 even though
            // each evaluates the exact same Reading twice (second is warm).
            assert_eq!(report["nativeRows"], json!(2));
            if allowance > 0 {
                assert_eq!(
                    report["nativeWorkRows"],
                    json!([[0, 0, allowance - 1], [1, 0, allowance - 1]]),
                    "each owner decodes once, repeated same-reading call is warm"
                );
            }
        }
    }
    #[test]
    fn structural_spike_job_cloned_php_format_value_reading_preserves_cold_decode_work() {
        let rule = json!({"pattern":"sprintf($F, $$$A)","formatArgs":{"style":"printf","string":"F","args":"A","unused":{"regex":".*"}}});
        let policy=CodeRuleDslRuntime::compile(json!({"format":"rerail.policy/v1","id":"shared-php-reading","revision":1,"mode":"steer","clauses":[{"id":"c","forbid":{"ast":{"language":"php","rule":rule}},"violation":{"code":"C","message":"match","anchor":"witness.span"}}]}).to_string(),"json".into(),CAPABILITIES.into()).unwrap();
        let original = all_grammars(&policy)[0];
        let first = repeated_original_reading(original);
        let second = first.clone();
        let grammars = vec![&first, &second];
        // The format is a decoded concatenation, not a direct literal parse.
        let batch = vec![Fixture {
            path: "reading.php".into(),
            source: "<?php sprintf('%' . 's', 1, 2);\n".into(),
        }];
        for allowance in [0, 1, 2, 3, 4, 8, 16, 32, 1000] {
            let report = validate_observed(
                &grammars,
                vec![finite_control(&grammars, allowance)],
                &batch,
                allowance,
                true,
            )
            .unwrap();
            assert_eq!(report["passed"], true, "allowance={allowance}: {report}");
            assert_eq!(
                report["nativeValueProbes"],
                json!([]),
                "FormatArgs reads decoded values directly, without a ValueTest callback"
            );
            assert_eq!(report["sqlValueProbes"], json!([]));
            if allowance == 1000 {
                assert_eq!(report["nativeRows"], json!(2));
            }
        }
    }
}
