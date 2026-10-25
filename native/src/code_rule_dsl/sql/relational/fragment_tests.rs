use super::*;
use crate::code_rule_dsl::{
    CodeRuleDslRuntime,
    model::{ClauseKind, Query},
    sql::{exec, tests::differential::tape},
    test_support::{CAPABILITIES, grammar_matchers},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::Path;

const REFUSED: [&str; 11] = [
    "b615",
    "loop-variable-overrides-iterator",
    "undefined-local-with-import-star-usage",
    "is-literal",
    "forward-annotation-syntax-error",
    "forward-annotation-semantic-syntax-error",
    "forward-annotation-await-outside-function",
    "multiple-starts-ends-with",
    "duplicate-isinstance-call",
    "enumerate-for-loop",
    "split-static-string",
];

fn oracle(fragment: &Fragment, batch: &Batch) -> (Vec<Value>, BTreeMap<u32, usize>) {
    let mut rows = vec![];
    let mut remaining = BTreeMap::new();
    let mut groups: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for r in &fragment.requests {
        groups.entry(r.file).or_default().push(r.node);
    }
    for (file, selected) in groups {
        let file = file as usize;
        let entry = &batch.files[file];
        let mut work = fragment.work;
        let mut provisional = vec![];
        let result = with_scopes(file, entry, || {
            matcher::with_file_memo(&entry.root, || {
                matcher::run_clause(
                    &entry.root,
                    &fragment.grammar,
                    Some(&selected),
                    entry.facts.len() as usize,
                    &mut work,
                    |node, env, _| {
                        provisional.push(json!([
                            file,
                            node,
                            super::super::cutover_tests::execution::captures(
                                &env,
                                &fragment.names
                            )?,
                            0
                        ]));
                        Ok::<_, String>(true)
                    },
                )
            })
        })
        .unwrap();
        remaining.insert(file as u32, work);
        match result {
            Ok(()) => rows.extend(provisional),
            Err(Stopped::Limit(limit)) => rows.push(json!([
                file,
                null,
                {},
                super::super::failure::encode(&format!("{limit:?}")).unwrap()
            ])),
            Err(Stopped::Deadline) => panic!("native deadline"),
        }
    }
    (rows, remaining)
}

fn read_row(
    row: &rusqlite::Row<'_>,
    offset: usize,
    fragment: &Fragment,
    batch: &Batch,
    counts: &std::collections::BTreeSet<String>,
) -> Option<Value> {
    let file = row.get::<_, u32>(offset).unwrap();
    let node = row.get::<_, u32>(offset + 1).unwrap();
    let truth = row.get::<_, i64>(offset + 2).unwrap();
    if truth < 0 {
        return Some(json!([file, null, {}, truth]));
    }
    if truth != 1 {
        return None;
    }
    let mut captures = serde_json::Map::new();
    for (i, name) in fragment.names.iter().enumerate() {
        let value = match row.get::<_, Cell>(offset + i + 4).unwrap() {
            Cell::Null => Value::Null,
            Cell::Integer(v) if counts.contains(name) => json!(v.to_string()),
            Cell::Integer(v) => json!(v),
            Cell::List(bytes) => json!(
                super::super::capture_cell::decode_list(
                    &bytes,
                    file,
                    batch.files[file as usize].facts.len()
                )
                .unwrap()
            ),
            Cell::Span(bytes) => {
                let s = span_capture::decode(&bytes).unwrap();
                json!({"text":s.text,"span":[s.range.start,s.range.end]})
            }
        };
        captures.insert(name.clone(), value);
    }
    Some(json!([file, node, captures, 0]))
}
fn read(stmt: &mut rusqlite::Statement<'_>, fragment: &Fragment, batch: &Batch) -> Vec<Value> {
    let counts = matcher::CaptureAnalysis::default().counts(&fragment.grammar.matcher);
    let mut rows = stmt.query([]).unwrap();
    let mut output = vec![];
    while let Some(row) = rows.next().unwrap() {
        if let Some(value) = read_row(row, 0, fragment, batch, &counts) {
            output.push(value);
        }
    }
    output
}
#[path = "fragment_job.rs"]
mod job;

#[test]
fn fragment_spike_original_eleven_refusals() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let fixtures = super::super::cutover_tests::execution::fixtures(root);
    let mut paths = std::fs::read_dir(root.join("packs/python/rules"))
        .unwrap()
        .map(|p| p.unwrap().path())
        .collect::<Vec<_>>();
    paths.sort();
    let mut reports = vec![];
    let mut catalog = vec![];
    let mut compilation = vec![];
    for path in paths {
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let source = std::fs::read_to_string(&path).unwrap();
        if !REFUSED
            .iter()
            .any(|name| source.contains(&format!("\"{name}\"")))
        {
            continue;
        }
        let compile = Started::now();
        let policy =
            CodeRuleDslRuntime::compile(source, "json".into(), CAPABILITIES.into()).unwrap();
        let compile = compile.elapsed();
        compilation.push(json!({"policy":path.file_name().unwrap().to_string_lossy(),"cpuNs":compile.cpu,"wallNs":compile.wall}));
        for clause in &policy.inner.clauses {
            if !REFUSED.contains(&clause.id.as_str()) {
                continue;
            }
            let ClauseKind::Forbid(Query::Ast { matchers, .. }) = &clause.kind else {
                panic!("not AST")
            };
            for grammar in matchers {
                catalog.push((clause.id.clone(), grammar.clone()));
                let setup = Started::now();
                let docs = fixtures
                    .iter()
                    .map(|f| {
                        (
                            tape(grammar.language, &f.source),
                            exec::FileContext {
                                path: Some(f.path.clone()),
                                ..Default::default()
                            },
                        )
                    })
                    .collect();
                let report = exec::with_terminals(docs, vec![], |db, batch, _| {
                    let fragment = Arc::new(Fragment::new(grammar, batch, usize::MAX));
                    register(db, c"matcher_fragment", batch.clone(), fragment.clone()).unwrap();
                    let setup = setup.elapsed();
                    let sql = "SELECT * FROM matcher_fragment WHERE truth=1 OR truth<0 ORDER BY file,node";
                    let prepare = Started::now();
                    let mut stmt = db.prepare(sql).unwrap();
                    let prepare = prepare.elapsed();
                    let explain = db
                        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
                        .unwrap()
                        .query_map([], |r| r.get::<_, String>(3))
                        .unwrap()
                        .collect::<Result<Vec<_>>>()
                        .unwrap();
                    let mut runs = vec![];
                    let mut matches = 0;
                    for _ in 0..3 {
                        let _native_scopes =
                            crate::code_rule_dsl::sql::terminals::BatchScopes::begin();
                        let native = Started::now();
                        let (expected, remaining) = oracle(&fragment, batch);
                        let native = native.elapsed();
                        *fragment.metrics.lock().unwrap() = Metrics::default();
                        stmt.reset_status(rusqlite::StatementStatus::VmStep);
                        let _hybrid_scopes =
                            crate::code_rule_dsl::sql::terminals::BatchScopes::begin();
                        let trace = super::super::tests::ExecutionTrace::start(db);
                        let timer = Started::now();
                        let actual = read(&mut stmt, &fragment, batch);
                        let elapsed = timer.elapsed();
                        let executions = trace.count();
                        drop(trace);
                        assert_eq!(executions, 1);
                        assert_eq!(actual, expected, "{}", clause.id);
                        matches = actual.len();
                        let metrics = fragment.metrics.lock().unwrap().clone();
                        assert_eq!(metrics.filters, 1, "one complete batch, {}", clause.id);
                        let differences: Vec<_> = remaining
                            .iter()
                            .filter(|(file, work)| metrics.remaining_work.get(file) != Some(work))
                            .take(5)
                            .collect();
                        assert!(
                            differences.is_empty(),
                            "work accounting {}: {differences:?}",
                            clause.id
                        );
                        runs.push(json!({"nativeCpuNs":native.cpu,"nativeWallNs":native.wall,"hybridCpuNs":elapsed.cpu,"hybridWallNs":elapsed.wall,"vm":stmt.get_status(rusqlite::StatementStatus::VmStep),"boundary":metrics}));
                    }
                    assert!(matches > 0, "positive witness {}", clause.id);
                    json!({"clause":clause.id,"files":fixtures.len(),"roots":fragment.requests.len(),"matches":matches,"sql":sql,"sqlBytes":sql.len(),"explain":explain,"setupCpuNs":setup.cpu,"setupWallNs":setup.wall,"prepareCpuNs":prepare.cpu,"prepareWallNs":prepare.wall,"runs":runs,"nativeComposition":true,"scope":"whole matcher region binding; independent SQL composition is a separate control"})
                });
                eprintln!(
                    "fragment {} roots={} matches={} hybridCpu={} nativeCpu={}",
                    clause.id,
                    report["roots"],
                    report["matches"],
                    report["runs"][1]["hybridCpuNs"],
                    report["runs"][1]["nativeCpuNs"]
                );
                reports.push(report);
            }
        }
    }
    assert_eq!(reports.len(), 11);
    let whole_job = job::measure(&catalog, &fixtures);
    let inventory=fixtures.iter().map(|f|json!({"path":f.path,"bytes":f.source.len(),"sha256":format!("{:x}",Sha256::digest(f.source.as_bytes()))})).collect::<Vec<_>>();
    if let Ok(path) = std::env::var("RERAIL_FRAGMENT_EVIDENCE") {
        std::fs::write(path,serde_json::to_vec_pretty(&json!({"scope":"eleven originally refused grammars, all catalog fixtures; not a full repository benchmark","grammars":reports,"wholeJob":whole_job,"policyCompilation":compilation,"fixtures":inventory})).unwrap()).unwrap();
    }
}

fn policy(rule: Value, utils: Value) -> crate::code_rule_dsl::CompiledCodeRuleDsl {
    CodeRuleDslRuntime::compile(json!({"format":"rerail.policy/v1","id":"fragment","revision":1,"mode":"steer","clauses":[{"id":"c","forbid":{"ast":{"language":"typescript","rule":rule,"utils":utils}},"violation":{"code":"F","message":"found","anchor":"witness.span"}}]}).to_string(),"json".into(),CAPABILITIES.into()).unwrap()
}

#[test]
fn fragment_spike_composes_typed_outputs_with_join_antijoin_and_aggregate() {
    let a = policy(json!({"pattern":"f($X)"}), json!({}));
    let b = policy(json!({"kind":"number","regex":"^1$"}), json!({}));
    let ga = &grammar_matchers(&a)[0];
    let gb = &grammar_matchers(&b)[0];
    exec::with_terminals(
        vec![(tape(ga.language, "f(1);f(2);f(1);"), Default::default())],
        vec![],
        |db, batch, _| {
            let a = Arc::new(Fragment::new(ga, batch, usize::MAX));
            let b = Arc::new(Fragment::new(gb, batch, usize::MAX));
            register(db, c"calls_fragment", batch.clone(), a.clone()).unwrap();
            register(db, c"ones_fragment", batch.clone(), b.clone()).unwrap();
            let sql = "WITH calls AS MATERIALIZED (SELECT file,node,c0 FROM calls_fragment WHERE truth=1), ones AS MATERIALIZED (SELECT file,node FROM ones_fragment WHERE truth=1) SELECT (SELECT count(*) FROM calls c JOIN ones o ON o.file=c.file AND o.node=c.c0),(SELECT count(*) FROM calls c WHERE NOT EXISTS (SELECT 1 FROM ones o WHERE o.file=c.file AND o.node=c.c0))";
            let actual = db
                .query_row(sql, [], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))
                .unwrap();
            assert_eq!(actual, (2, 1));
            assert_eq!(a.metrics.lock().unwrap().filters, 1);
            assert_eq!(b.metrics.lock().unwrap().filters, 1);
        },
    );
}

#[test]
fn fragment_spike_bound_inputs_unknown_and_file_failure() {
    let p = policy(json!({"kind":"identifier","pattern":"$X"}), json!({}));
    let grammar = &grammar_matchers(&p)[0];
    exec::with_terminals(
        vec![(tape(grammar.language, "a;b;a;"), Default::default())],
        vec![],
        |db, batch, _| {
            let mut f = Fragment::new(grammar, batch, usize::MAX);
            let bound = f.requests[0].node;
            for r in &mut f.requests {
                r.inputs = vec![("X".into(), SlotKind::Node, Cell::Integer(bound as i64))];
            }
            let f = Arc::new(f);
            register(db, c"bound_fragment", batch.clone(), f.clone()).unwrap();
            assert_eq!(
                db.query_row(
                    "SELECT count(*) FROM bound_fragment WHERE truth=1",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                2
            );
            *f.metrics.lock().unwrap() = Metrics::default();
            assert_eq!(db.query_row("SELECT count(*) FROM bound_fragment WHERE request IN (SELECT 0 UNION ALL SELECT 2) AND truth=1",[],|r|r.get::<_,i64>(0)).unwrap(),2);
            assert_eq!(f.metrics.lock().unwrap().filters, 1);
            assert_eq!(f.metrics.lock().unwrap().roots, 2);
            assert_eq!(
                db.query_row(
                    "SELECT count(*) FROM bound_fragment WHERE request IN (SELECT 0 WHERE 0)",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                0
            );
        },
    );
    let p = policy(
        json!({"kind":"number","matches":"u"}),
        json!({"u":{"kind":"number"}}),
    );
    let grammar = &grammar_matchers(&p)[0];
    exec::with_terminals(
        vec![
            (tape(grammar.language, "1;2;"), Default::default()),
            (tape(grammar.language, "1;"), Default::default()),
        ],
        vec![],
        |db, batch, _| {
            let f = Arc::new(Fragment::new(grammar, batch, 1));
            register(db, c"limited_fragment", batch.clone(), f.clone()).unwrap();
            let (expected, remaining) = oracle(&f, batch);
            let actual=read(&mut db.prepare("SELECT * FROM limited_fragment WHERE truth=1 OR truth<0 ORDER BY file,node").unwrap(),&f,batch);
            assert_eq!(actual, expected);
            assert_eq!(f.metrics.lock().unwrap().remaining_work, remaining);
            assert!(
                actual
                    .iter()
                    .any(|r| r[0] == 0 && r[3].as_i64().unwrap() < 0)
            );
            assert!(actual.iter().any(|r| r[0] == 1 && r[3] == 0));
        },
    );
    let p = policy(json!({"kind":"identifier","testUnknown":"^x$"}), json!({}));
    let grammar = &grammar_matchers(&p)[0];
    exec::with_terminals(
        vec![(tape(grammar.language, "x;y;"), Default::default())],
        vec![],
        |db, batch, _| {
            let f = Arc::new(Fragment::new(grammar, batch, usize::MAX));
            register(db, c"unknown_fragment", batch.clone(), f).unwrap();
            assert_eq!(
                db.query_row(
                    "SELECT sum(truth=2),sum(truth=1) FROM unknown_fragment",
                    [],
                    |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
                )
                .unwrap(),
                (1, 1)
            );
        },
    );
}

#[test]
fn fragment_spike_capture_enumeration_stays_inside_one_batch() {
    let p = policy(
        json!({"kind":"call_expression","has":{"kind":"arguments","has":{"kind":"identifier","pattern":"$X"}}}),
        json!({}),
    );
    let grammar = &grammar_matchers(&p)[0];
    exec::with_terminals(
        vec![(tape(grammar.language, "f(a,b,a);"), Default::default())],
        vec![],
        |db, batch, _| {
            let mut fragment = Fragment::new(grammar, batch, usize::MAX);
            fragment.each = Some("X".into());
            let fragment = Arc::new(fragment);
            register(db, c"enumerated_fragment", batch.clone(), fragment.clone()).unwrap();
            let rows = db
                .prepare(
                    "SELECT ordinal,c0 FROM enumerated_fragment WHERE truth=1 ORDER BY ordinal",
                )
                .unwrap()
                .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, u32>(1)?)))
                .unwrap()
                .collect::<Result<Vec<_>>>()
                .unwrap();
            // Native each enumerates distinct source ranges, including equal text at different occurrences.
            let texts = rows
                .iter()
                .map(|(_, id)| {
                    matcher::node_at(&batch.files[0].root, *id)
                        .text()
                        .into_owned()
                })
                .collect::<Vec<_>>();
            assert_eq!(texts, ["a", "b", "a"]);
            assert_eq!(rows.iter().map(|(o, _)| *o).collect::<Vec<_>>(), [0, 1, 2]);
            assert_eq!(fragment.metrics.lock().unwrap().filters, 1);
        },
    );
}

#[test]
fn fragment_spike_sql_interpreter_is_not_an_emission_option() {
    let p = policy(
        json!({"kind":"arguments","has":{"stopBy":"end","matches":"u"}}),
        json!({"u":{"any":[{"pattern":"$X"},{"has":{"matches":"u"}}]}}),
    );
    // Legacy explicit requests cannot resurrect the removed function-call emitter.
    let grammars = grammar_matchers(&p);
    let refs = grammars.iter().collect::<Vec<_>>();
    let result = super::super::outcomes::compile_job_finite(&refs, 100);
    assert!(result.is_err());
}
