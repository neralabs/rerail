//! Real catalog grammar parity. Every fixture belongs to the same SQL file batch.
use super::{super::Compiled, digest};
use crate::code_rule_dsl::{
    matcher::{self, GrammarMatcher, Stopped},
    soundness::SyntaxErrors,
    sql::{exec, profiling, tests::differential::tape},
    tape::TapeDoc,
};
use ast_grep_core::{
    AstGrep,
    meta_var::{MetaVarEnv, MetaVariable},
};
use serde_json::{Map, Value, json};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

pub(in crate::code_rule_dsl::sql::relational) struct Fixture {
    pub(in crate::code_rule_dsl::sql::relational) path: String,
    pub(in crate::code_rule_dsl::sql::relational) source: String,
}

pub(in crate::code_rule_dsl::sql::relational) fn fixtures(root: &Path) -> Vec<Fixture> {
    fn collect(path: &Path, paths: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                collect(&path, paths);
            } else if path.extension().is_some_and(|e| e == "py" || e == "pyi") {
                paths.push(path);
            }
        }
    }
    let mut paths = vec![];
    for tool in ["ruff", "bandit"] {
        collect(
            &root.join(format!("packs/python/upstream/{tool}/fixtures")),
            &mut paths,
        );
    }
    paths.sort();
    paths
        .into_iter()
        .map(|path| Fixture {
            source: std::fs::read_to_string(&path).unwrap(),
            path: path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        })
        .collect()
}

pub(super) fn inventory(fixtures: &[Fixture]) -> Value {
    json!({
        "selection":"Every .py and .pyi file recursively under packs/python/upstream/{ruff,bandit}/fixtures; no per-rule or source exclusions.",
        "bytes":fixtures.iter().map(|f|f.source.len()).sum::<usize>(),
        "files":fixtures.iter().map(|f|json!({"path":f.path,"bytes":f.source.len(),"sha256":digest(f.source.as_bytes())})).collect::<Vec<_>>()
    })
}

pub(in crate::code_rule_dsl::sql::relational) fn captures(env: &MetaVarEnv<'_, TapeDoc>, names: &[String]) -> Result<Value, String> {
    let mut list_names = BTreeSet::new();
    for variable in env.get_matched_variables() {
        let name = match variable {
            MetaVariable::Capture(name, _) => name,
            MetaVariable::MultiCapture(name) => {
                list_names.insert(name.clone());
                name
            }
            _ => return Err("native environment exposed an unsupported capture kind".into()),
        };
        if matcher::is_span_variable(&name) {
            continue;
        }
        if !names.contains(&name) {
            return Err(format!(
                "native capture {name:?} is absent from compiler output schema"
            ));
        }
    }
    let mut values = Map::new();
    for name in names {
        let value = if list_names.contains(name) {
            json!(
                env.get_multiple_matches(name)
                    .iter()
                    .map(|node| node.node_id() as u32)
                    .collect::<Vec<_>>()
            )
        } else if let Some(node) = env.get_match(name) {
            json!(node.node_id() as u32)
        } else if let (Some(text), Some(span)) = (
            matcher::exported_text(env, name),
            matcher::exported_span(env, name),
        ) {
            json!({"text":text,"span":[span.start,span.end]})
        } else if let Some(bytes) = env.get_transformed(name) {
            json!(
                String::from_utf8(bytes.to_vec())
                    .map_err(|error| format!("non-UTF8 transformed capture {name}: {error}"))?
            )
        } else {
            Value::Null
        };
        values.insert(name.clone(), value);
    }
    Ok(Value::Object(values))
}

fn schema(compiled: &Compiled, grammar: &GrammarMatcher) -> Result<Vec<String>, String> {
    let names = if let Some(captures) = compiled.relations["captures"].as_array() {
        captures
            .iter()
            .map(|capture| {
                capture["name"]
                    .as_str()
                    .map(str::to_owned)
                    .ok_or("capture schema lacks name".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?
    } else if let Some(capture) = compiled.relations["capture"].as_str() {
        vec![capture.to_owned()]
    } else {
        vec![]
    };
    let mut declared = matcher::CaptureAnalysis::default().binds(&grammar.matcher);
    declared.extend(matcher::CaptureAnalysis::default().counts(&grammar.matcher));
    let published: BTreeSet<_> = names.iter().cloned().collect();
    if published != declared || published.len() != names.len() {
        return Err(format!(
            "capture schema differs: compiler {names:?}, native declarations {declared:?}"
        ));
    }
    Ok(names)
}

pub(in crate::code_rule_dsl::sql::relational) fn check(
    grammar: &GrammarMatcher,
    compiled: Compiled,
    fixtures: &[Fixture],
) -> Result<Value, String> {
    if fixtures.is_empty() {
        return Err("empty fixture batch".into());
    }
    if grammar.transform.is_some() {
        return Err(
            "catalog runner needs transform publication before this grammar can pass".into(),
        );
    }
    let names = schema(&compiled, grammar)?;
    let docs: Vec<_> = fixtures
        .iter()
        .map(|fixture| tape(grammar.language, &fixture.source))
        .collect();
    let mut expected = vec![];
    let mut native_traces = vec![];
    let mut positive_files = vec![];
    let mut native_matches = 0;
    for (file, (doc, fixture)) in docs.iter().zip(fixtures).enumerate() {
        let tree = AstGrep::doc(doc.clone());
        let errors = SyntaxErrors::of_doc(doc);
        let _scopes =
            matcher::FileScopeSlot::install(Default::default(), None, errors.hide_bindings())
                .path(&fixture.path);
        let mut provisional = vec![];
        let mut work = i64::MAX as usize;
        let outcome = if grammar.applies
            && grammar
                .file
                .as_ref()
                .is_none_or(|condition| condition.holds(&fixture.path, None))
        {
            matcher::with_file_memo(&tree, || {
                matcher::run_clause(
                    &tree,
                    grammar,
                    None,
                    doc.transducer_len(),
                    &mut work,
                    |root, env, _| {
                        provisional.push(json!([file, root, captures(&env, &names)?, 0]));
                        Ok::<_, String>(true)
                    },
                )
            })?
        } else {
            Ok(())
        };
        let provisional_count = provisional.len();
        let failure = match outcome {
            Ok(()) => {
                if provisional_count > 0 {
                    positive_files.push(file);
                    native_matches += provisional_count;
                }
                expected.extend(provisional);
                0
            }
            Err(Stopped::Limit(limit)) => {
                let tag = super::super::failure::encode(&format!("{limit:?}"))
                    .ok_or_else(|| format!("unsupported native failure {limit:?}"))?;
                expected.push(json!([file, null, {}, tag]));
                tag
            }
            Err(Stopped::Deadline) => {
                return Err("native deadline interrupted fixture validation".into());
            }
        };
        native_traces.push(json!({"file":file,"provisionalMatches":provisional_count,"failure":failure,"remainingWork":work}));
    }
    let sql_sha = digest(compiled.sql.as_bytes());
    let has_failure = compiled.relations["failureColumn"].as_u64() == Some(4);
    let (execution, frames) = profiling::frame_counts(|| {
        exec::with_terminals(
            docs.into_iter()
                .zip(fixtures)
                .map(|(doc, fixture)| {
                    (
                        doc,
                        exec::FileContext {
                            path: Some(fixture.path.clone()),
                            ..Default::default()
                        },
                    )
                })
                .collect(),
            compiled.terminals,
            |db, batch, _| -> Result<Value, String> {
                super::super::leaves::register(db, batch).map_err(|e| e.to_string())?;
                let return_rows = super::super::row::ROWS.get();
                let return_cells = super::super::row::CELLS.get();
                let continuation_start = super::super::continuation_store::traffic();
                let stages =
                    super::super::spike_stages::run(db, &compiled.relations, &compiled.sql)?;
                let prepare_started = crate::code_rule_dsl::sql::timing::Started::now();
                let mut stmt = db
                    .prepare(&compiled.sql)
                    .map_err(|e| format!("prepare: {e}"))?;
                let prepare = prepare_started.elapsed();

                let trace = super::super::tests::ExecutionTrace::start(db);
                let mut actual = vec![];
                let started = crate::code_rule_dsl::sql::timing::Started::now();
                {
                    let mut rows = stmt.query([]).map_err(|e| format!("query: {e}"))?;
                    while let Some(row) = rows.next().map_err(|e| format!("step: {e}"))? {
                        let clause = row.get::<_, i64>(0).map_err(|e| e.to_string())?;
                        if clause != 0 {
                            return Err(format!("unexpected clause index {clause}"));
                        }
                        let file = row.get::<_, u32>(1).map_err(|e| e.to_string())? as usize;
                        if file >= fixtures.len() {
                            return Err(format!("unexpected file index {file}"));
                        }
                        let failure = if has_failure {
                            row.get::<_, i64>(4).map_err(|e| e.to_string())?
                        } else {
                            0
                        };
                        if failure > 0
                            || (failure < 0 && super::super::failure::decode(failure).is_none())
                        {
                            return Err(format!("invalid publication failure {failure}"));
                        }
                        let payload = row.get::<_, String>(3).map_err(|e| e.to_string())?;
                        let values: Vec<Value> = serde_json::from_str(&payload)
                            .map_err(|e| format!("capture JSON: {e}"))?;
                        let root = if failure < 0 {
                            None
                        } else {
                            Some(row.get::<_, u32>(2).map_err(|e| e.to_string())?)
                        };
                        let mut captures = Map::new();
                        if failure < 0 {
                            if !values.is_empty() {
                                return Err("failed file published capture values".into());
                            }
                        } else {
                            if values.len() != names.len() {
                                return Err(format!(
                                    "capture arity {}, expected {}",
                                    values.len(),
                                    names.len()
                                ));
                            }
                            captures.extend(names.iter().cloned().zip(values));
                        }
                        actual.push(json!([file, root, captures, failure]));
                    }
                }
                let elapsed = started.elapsed();
                let scan = trace.scan();
                let final_executions = trace.count();
                let executions = final_executions + stages.executions;
                drop(trace);
                // Explain and diagnostic reads are outside matching counters and timing.
                let explain = db
                    .prepare(&format!("EXPLAIN QUERY PLAN {}", compiled.sql))
                    .map_err(|e| format!("explain prepare: {e}"))?
                    .query_map([], |r| {
                        Ok(json!([
                            r.get::<_, i64>(0)?,
                            r.get::<_, i64>(1)?,
                            r.get::<_, String>(3)?
                        ]))
                    })
                    .map_err(|e| e.to_string())?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| e.to_string())?;
                let first_difference = expected
                    .iter()
                    .zip(&actual)
                    .position(|(a, b)| a != b)
                    .or_else(|| {
                        (expected.len() != actual.len()).then_some(expected.len().min(actual.len()))
                    });
                let passed =
                    first_difference.is_none() && executions == stages.expected_statements();
                let mut report = json!({
                    "passed":passed,"matchingStatements":executions,"vm":i64::from(stmt.get_status(rusqlite::StatementStatus::VmStep))+stages.vm,
                    "fullscanSteps":i64::from(stmt.get_status(rusqlite::StatementStatus::FullscanStep))+stages.fullscan,
                    "automaticIndexRows":i64::from(stmt.get_status(rusqlite::StatementStatus::AutoIndex))+stages.automatic_index,
                    "sorts":i64::from(stmt.get_status(rusqlite::StatementStatus::Sort))+stages.sorts,
                    "prepareCpuNs":prepare.cpu+stages.prepare.cpu,"prepareWallNs":prepare.wall+stages.prepare.wall,
                    "executionCpuNs":elapsed.cpu+stages.execution.cpu,"executionWallNs":elapsed.wall+stages.execution.wall,
                    "typedReturnRows":super::super::row::ROWS.get()-return_rows,
                    "typedReturnCells":super::super::row::CELLS.get()-return_cells,
                    "continuationTraffic": {
                        "pushes":super::super::continuation_store::traffic()[0]-continuation_start[0],
                        "pops":super::super::continuation_store::traffic()[1]-continuation_start[1],
                        "peekRequests":super::super::continuation_store::traffic()[2]-continuation_start[2],
                        "reservedCellsWritten":super::super::continuation_store::traffic()[3]-continuation_start[3]
                    },
                    "explain":explain,
                    "nativeMatches":native_matches,"nativeFailures":native_traces.iter().filter(|r|r["failure"]!=0).count(),
                    "nativeRows":expected.len(),"sqlRows":actual.len(),"positiveFiles":positive_files,
                    "firstDifference":first_difference.map(|i|json!({"index":i,"expected":expected.get(i),"actual":actual.get(i)})),
                    "nativeOutputSha256":digest(&serde_json::to_vec(&expected).unwrap()),"sqlOutputSha256":digest(&serde_json::to_vec(&actual).unwrap()),
                    "captureNames":names,"nativeFileTraces":native_traces,"sqlSha256":sql_sha,
                    "coverageGap":if native_matches==0 { Some("No positive native witness in the complete fixture batch; empty-result parity does not cover successful publication.") } else { None },
                    "failureContract":"Native clause/file failures discard provisional matches. SQL failure tags and surviving outputs are compared. No fault injection or finite allowance sweep in this gate."
                });
                report["scan"] = scan.clone();
                if stages.active() {
                    report["stageExecution"] = stages.profile();
                    report["finalStatement"] = json!({"scan":scan,"matchingStatements":final_executions,"vmSteps":stmt.get_status(rusqlite::StatementStatus::VmStep),"automaticIndexRows":stmt.get_status(rusqlite::StatementStatus::AutoIndex),"fullscanSteps":stmt.get_status(rusqlite::StatementStatus::FullscanStep),"sorts":stmt.get_status(rusqlite::StatementStatus::Sort),"prepareCpuNs":prepare.cpu,"prepareWallNs":prepare.wall,"executionCpuNs":elapsed.cpu,"executionWallNs":elapsed.wall});
                }
                Ok(report)
            },
        )
    });
    let mut report = execution?;
    report["framePublications"] = json!(frames[4]);
    report["nodeAdvances"] = json!(frames[11]);
    report["nodeColumns"] = json!(frames[12]);
    report["nodeFilters"] = json!(frames[13]);
    if frames[4] != 0 {
        report["passed"] = json!(false);
    }
    Ok(report)
}

#[test]
fn catalog_capture_oracle_keeps_exported_ranges_and_ignores_reserved_metadata() {
    let doc = tape(
        crate::code_rule_dsl::language::GrammarDialect::Python,
        "abc",
    );
    let tree = AstGrep::doc(doc);
    let mut env = MetaVarEnv::new();
    matcher::export_text(&mut env, "C", "decoded", 1..3);
    assert_eq!(
        captures(&env, &["C".into()]).unwrap(),
        json!({"C":{"text":"decoded","span":[1,3]}})
    );
    env.insert("C", tree.root());
    assert_eq!(captures(&env, &["C".into()]).unwrap(), json!({"C":0}));
    assert_eq!(matcher::exported_span(&env, "C"), Some(1..3));
    env.insert("UNDECLARED", tree.root());
    assert!(
        captures(&env, &["C".into()])
            .unwrap_err()
            .contains("UNDECLARED")
    );
}
