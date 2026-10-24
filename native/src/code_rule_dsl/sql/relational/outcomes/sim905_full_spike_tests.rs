use super::super::publication_tests::check_work_compiler;
use super::*;
use crate::code_rule_dsl::{
    CodeRuleDslRuntime, CompiledCodeRuleDsl,
    sql::{exec, tests::differential::tape},
    test_support::{CAPABILITIES, grammar_matchers},
};
use serde_json::{Value, json};
fn policy() -> (CompiledCodeRuleDsl, [Value; 1]) {
    let mut p: Value = serde_json::from_str(include_str!(
        "../../../../../../packs/python/rules/ruff-sim905-split-static-string.json"
    ))
    .unwrap();
    p["clauses"][0]["id"] = json!("c0");
    let rule = p["clauses"][0]["forbid"]["ast"]["rule"].clone();
    (
        CodeRuleDslRuntime::compile(p.to_string(), "json".into(), CAPABILITIES.into()).unwrap(),
        [rule],
    )
}
#[test]
fn full_sim905_spike_preserves_native_work_reached_order_and_rejected_suffix() {
    let (p, rules) = policy();
    let huge = format!("0x1{}", "0".repeat(1025));
    let unknown = format!("'a'.split(maxsplit={huge})\n'a'.split(maxsplit={huge},maxsplit=1)\n");
    let sources = [
        "'a'.split()\n'a'.unknown(maxsplit=1)\n'a'.split(sep=x,maxsplit=2)\n'a'.split(maxsplit=2147483648,maxsplit=1)\n'a'.split(None,(-2))\n'a'.split(maxsplit=x)\n'a'.split(unknown=1)\n'a'.split(1, 'x')\n'a'.split(sep=2, wrong=3)\n'a'.split(1,maxsplit=x)\n'a'.split(1,sep=x)\n",
        "x.split(maxsplit=1)\n'a'.unknown(maxsplit=2)\n",
        "x = 1\n",
        &unknown,
    ];
    let grammars = grammar_matchers(&p);
    let g = &grammars[0];
    for work in [0, 1, 2, 3, 4, 8, 16, 32, 64, 256, 10000, i64::MAX] {
        let report = check_work_compiler(&p, &rules, &sources, None, work, compile);
        let diagnostic = compile_final(&[g], work).unwrap();
        exec::with_terminals(
            sources
                .iter()
                .map(|s| (tape(g.language, s), Default::default()))
                .collect(),
            diagnostic.terminals,
            |db, batch, _| {
                super::super::super::leaves::register(db, batch).unwrap();
                let _stages = crate::code_rule_dsl::sql::relational::spike_stages::run(
                    db,
                    &diagnostic.relations,
                    &diagnostic.sql,
                )
                .unwrap();
                let actual = db
                    .prepare(&diagnostic.sql)
                    .unwrap()
                    .query_map([], |r| {
                        Ok((
                            r.get::<_, u32>(0)? as usize,
                            r.get::<_, i64>(1)?,
                            r.get::<_, i64>(2)?,
                        ))
                    })
                    .unwrap()
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .unwrap();
                assert_eq!(
                    actual.len(),
                    sources.len(),
                    "diagnostic must retain files with no roots"
                );
                for (file, _truth, remaining) in actual {
                    assert_eq!(
                        remaining as u64,
                        report["nativeTraces"][file]["remaining"].as_u64().unwrap(),
                        "final rejected suffix work={work},file={file}"
                    );
                }
            },
        );
    }
    for fault in ["1", "2", "2147483648"] {
        check_work_compiler(&p, &rules, &sources, Some(fault), 10000, compile);
    }
}
#[test]
fn full_sim905_spike_all597_native_parity_and_measured_structural_gate() {
    use crate::code_rule_dsl::sql::relational::cutover_tests::execution::{check, fixtures};
    let (p, _) = policy();
    let grammars = grammar_matchers(&p);
    let g = &grammars[0];
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let batch = fixtures(root);
    assert_eq!(batch.len(), 597);
    let mut compiled = compile(&[g], i64::MAX).unwrap();
    if let Ok(path) = std::env::var("RERAIL_SIM905_REFERENCE_SQL_OVERRIDE") {
        compiled.sql = std::fs::read_to_string(path).unwrap();
    }
    let stage_bytes = compiled.relations["spikeStages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().len())
        .collect::<Vec<_>>();
    let bytes = std::iter::once(compiled.sql.len())
        .chain(stage_bytes.iter().copied())
        .max()
        .unwrap();
    let total_bytes = compiled.sql.len() + stage_bytes.iter().sum::<usize>();
    let report = check(g, compiled, &batch).unwrap();
    let evidence = json!({"contract":"full SIM905 truth/native work/effect order reference; focused tests include final rejected suffix remaining work","maxStatementSqlBytes":bytes,"totalSqlBytes":total_bytes,"stageSqlBytes":stage_bytes,"catalog":report,"structuralGate":{"maxVm":44908386,"maxTypedReturnRows":158186,"maxSqlBytes":256000},"productionSwitch":false});
    if let Ok(path) = std::env::var("RERAIL_SIM905_FULL_SPIKE_EVIDENCE") {
        std::fs::write(path, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
    }
    assert_eq!(report["passed"], true, "{evidence}");
    assert!(
        report["matchingStatements"].as_u64().unwrap() <= 100,
        "all stages count toward matching budget: {evidence}"
    );
    assert!(
        report["vm"].as_u64().unwrap() <= 44908386,
        "measured structural spike rejected: {evidence}"
    );
    assert!(
        report["typedReturnRows"].as_u64().unwrap() <= 158186,
        "measured typed-return gate rejected: {evidence}"
    );
    assert!(bytes <= 256000, "measured SQL budget rejected: {evidence}");
}

#[test]
fn pure_helper_summaries_prepare_and_match_original_native_depth_oracle() {
    use crate::code_rule_dsl::matcher::{self, AstMatcher, UtilRef, node_at};
    use std::sync::Arc;
    let (p, _) = policy();
    let grammars = grammar_matchers(&p);
    let g = &grammars[0];
    for name in ["receiver", "sep"] {
        let initial = pure::compile(g, name, "SELECT file,id node FROM node", 0).unwrap();
        exec::with_terminals(
            vec![(tape(g.language, "('a')\n(None)\n(x)\n"), Default::default())],
            initial.terminals,
            |db, batch, _| {
                super::super::super::leaves::register(db, batch).unwrap();
                db.prepare(&initial.sql).unwrap();
                let file = &batch.files[0];
                let ids = ["parenthesized_expression", "identifier", "none"]
                    .iter()
                    .map(|kind| {
                        (0..file.facts.len())
                            .find(|id| node_at(&file.root, *id).kind() == *kind)
                            .unwrap()
                    })
                    .collect::<Vec<_>>();
                let mut native_g = g.clone();
                let index = g.utils.names.iter().position(|n| n == name).unwrap();
                native_g.matcher = AstMatcher::Util(UtilRef {
                    table: Arc::downgrade(&g.utils),
                    index,
                });
                let _scope =
                    matcher::FileScopeSlot::install(Default::default(), Default::default(), false);
                matcher::with_file_memo(&file.root, || {
                    for id in ids {
                        for incoming in [0, 253, 254, 255, 256] {
                            let c = pure::compile(
                                g,
                                name,
                                &format!("SELECT 0 file,{id} node"),
                                incoming,
                            )
                            .unwrap();
                            let summary: (i64, i64, i64) = db
                                .query_row(&c.sql, [], |r| Ok((r.get(2)?, r.get(3)?, r.get(4)?)))
                                .unwrap();
                            let native = matcher::match_at_depth(
                                &native_g,
                                node_at(&file.root, id),
                                incoming,
                                10000,
                            );
                            assert_eq!(
                                (summary.0, 10000 - summary.1, summary.2),
                                (
                                    native.truth,
                                    native.work as i64,
                                    native.peak.saturating_sub(incoming) as i64
                                ),
                                "helper={name}, incoming={incoming}"
                            );
                        }
                    }
                });
            },
        );
    }
}

#[test]
fn literal_projection_matches_original_generic_dag_for_prefixes_parts_and_depth() {
    let (p, _) = policy();
    let grammars = grammar_matchers(&p);
    let g = &grammars[0];
    let source = "('a')\n(('x'))\nb'a'\nf'a'\nrb'a'\nfr'a'\nt'a'\n'a' 'b'\n'a' f'b'\n(b'a' 'b')\n(None)\nx\n";
    for name in ["receiver", "sep"] {
        for incoming in [0, 250, 252, 253, 254, 255, 256] {
            let projected =
                pure::compile(g, name, "SELECT file,id node FROM node", incoming).unwrap();
            let control =
                pure::compile_control(g, name, "SELECT file,id node FROM node", incoming).unwrap();
            let control_sql = pure::offset(
                &control.sql,
                projected.terminals.len(),
                control.terminals.len(),
            );
            let projected_sql = projected.sql;
            let mut terminals = projected.terminals;
            terminals.extend(control.terminals);
            exec::with_terminals(
                vec![(tape(g.language, source), Default::default())],
                terminals,
                |db, batch, _| {
                    super::super::super::leaves::register(db, batch).unwrap();
                    let read = |sql: &str| {
                        let mut rows = db
                            .prepare(sql)
                            .unwrap()
                            .query_map([], |r| {
                                Ok((
                                    r.get::<_, i64>(0)?,
                                    r.get::<_, i64>(1)?,
                                    r.get::<_, i64>(2)?,
                                    r.get::<_, i64>(3)?,
                                    r.get::<_, i64>(4)?,
                                ))
                            })
                            .unwrap()
                            .collect::<rusqlite::Result<Vec<_>>>()
                            .unwrap();
                        rows.sort();
                        rows
                    };
                    assert_eq!(
                        read(&projected_sql),
                        read(&control_sql),
                        "source-derived generic control helper={name},incoming={incoming}"
                    );
                },
            );
        }
    }
}

#[test]
fn fused_pure_producers_are_measured_against_staged_reference() {
    use crate::code_rule_dsl::sql::relational::cutover_tests::execution::{check, fixtures};
    let (p, rules) = policy();
    let grammars = grammar_matchers(&p);
    let g = &grammars[0];
    let candidate = compile_fused(&[g], i64::MAX).unwrap();
    let bytes = candidate.sql.len();
    let prepare = exec::with_terminals(
        vec![(
            tape(g.language, "'a'.split(maxsplit=1)"),
            Default::default(),
        )],
        candidate.terminals,
        |db, batch, _| {
            super::super::super::leaves::register(db, batch).unwrap();
            db.prepare(&candidate.sql)
                .map(drop)
                .map_err(|e| e.to_string())
        },
    );
    let evidence = if let Err(error) = prepare {
        json!({"candidate":"fused pure producers","accepted":false,"prepareFailure":error,"sqlBytes":bytes,"productionSwitch":false,"stagedReferenceRetained":true})
    } else {
        let huge = format!("0x1{}", "0".repeat(1025));
        let unknown =
            format!("'a'.split(maxsplit={huge},maxsplit=1)\n'a'.split(maxsplit={huge})\n");
        let sources = [
            "'a'.split()\n'a'.unknown(maxsplit=1)\n'a'.split(1,maxsplit=x)\n'a'.split(maxsplit=2147483648,maxsplit=1)\n'a'.split(None,(-2))\n",
            unknown.as_str(),
        ];
        for work in [0, 1, 2, 4, 8, 32, 64, 256, 10000, i64::MAX] {
            let report = check_work_compiler(&p, &rules, &sources, None, work, compile_fused);
            let diagnostic = compile_fused_mode(&[g], work, true).unwrap();
            exec::with_terminals(
                sources
                    .iter()
                    .map(|s| (tape(g.language, s), Default::default()))
                    .collect(),
                diagnostic.terminals,
                |db, batch, _| {
                    super::super::super::leaves::register(db, batch).unwrap();
                    let rows = db
                        .prepare(&diagnostic.sql)
                        .unwrap()
                        .query_map([], |r| {
                            Ok((r.get::<_, u32>(0)? as usize, r.get::<_, i64>(2)?))
                        })
                        .unwrap()
                        .collect::<rusqlite::Result<Vec<_>>>()
                        .unwrap();
                    assert_eq!(rows.len(), sources.len());
                    for (file, remaining) in rows {
                        assert_eq!(
                            remaining as u64,
                            report["nativeTraces"][file]["remaining"].as_u64().unwrap(),
                            "fused final rejected suffix work={work}"
                        );
                    }
                },
            );
        }
        for fault in ["1", "2147483648"] {
            check_work_compiler(&p, &rules, &sources, Some(fault), 10000, compile_fused);
        }
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap();
        let batch = fixtures(root);
        assert_eq!(batch.len(), 597);
        let report = check(g, compile_fused(&[g], i64::MAX).unwrap(), &batch).unwrap();
        assert_eq!(report["passed"], true, "fused native parity: {report}");
        assert_eq!(report["matchingStatements"], 1);
        assert_eq!(report["framePublications"], 0);
        let accepted = bytes <= 256000
            && report["vm"].as_u64().unwrap() <= 44908386
            && report["typedReturnRows"].as_u64().unwrap() <= 158186;
        json!({"candidate":"fused pure producers","accepted":accepted,"sqlBytes":bytes,"catalog":report,"productionSwitch":false,"selection":"compare measured VM to staged reference; do not relax fixed gates"})
    };
    if let Ok(path) = std::env::var("RERAIL_SIM905_FULL_SPIKE_EVIDENCE") {
        std::fs::write(
            format!("{path}.fused.json"),
            serde_json::to_vec_pretty(&evidence).unwrap(),
        )
        .unwrap();
    }
    if let Some(error) = evidence["prepareFailure"].as_str() {
        assert!(
            error.contains("too many references")
                || error.contains("parser stack overflow")
                || error.contains("Expression tree is too large")
                || error.contains("at most 64 tables"),
            "unclosed or invalid fused generator is a defect, not a staging proof: {error}"
        );
    }
    println!("SIM905_FUSED_CONTROL {}", evidence);
}
