//! Whole eleven-grammar job: fresh tapes, native file/rule execution versus one SQL publication.
use super::super::super::cutover_tests::execution::Fixture;
use super::*;
use ast_grep_core::AstGrep;

fn native(catalog: &[(String, GrammarMatcher)], fixtures: &[Fixture]) -> Vec<Value> {
    let names = catalog
        .iter()
        .map(|(_, g)| {
            let mut a = matcher::CaptureAnalysis::default();
            let mut n = a.binds(&g.matcher);
            n.extend(a.counts(&g.matcher));
            n.into_iter().collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let mut output = vec![];
    for (file, f) in fixtures.iter().enumerate() {
        let doc = tape(catalog[0].1.language, &f.source);
        let errors = crate::code_rule_dsl::soundness::SyntaxErrors::of_doc(&doc);
        let tree = AstGrep::doc(doc.clone());
        let _scopes =
            matcher::FileScopeSlot::install(Default::default(), None, errors.hide_bindings())
                .path(&f.path);
        matcher::with_file_memo(&tree, || {
            for (id, (_, grammar)) in catalog.iter().enumerate() {
                if !grammar.applies
                    || grammar
                        .file
                        .as_ref()
                        .is_some_and(|c| !c.holds(&f.path, None))
                {
                    continue;
                }
                let mut work = usize::MAX;
                let mut provisional = vec![];
                let result = matcher::run_clause(
                    &tree,
                    grammar,
                    None,
                    doc.transducer_len(),
                    &mut work,
                    |node, env, _| {
                        let captures = super::super::super::cutover_tests::execution::captures(
                            &env, &names[id],
                        )?;
                        provisional.push(json!([id, file, node, captures, 0]));
                        Ok::<_, String>(true)
                    },
                )
                .unwrap();
                match result {
                    Ok(()) => output.extend(provisional),
                    Err(Stopped::Limit(limit)) => output.push(json!([
                        id,
                        file,
                        null,
                        {},
                        super::super::super::failure::encode(&format!("{limit:?}")).unwrap()
                    ])),
                    Err(Stopped::Deadline) => panic!("native deadline"),
                }
            }
        });
    }
    output.sort_by_key(|r| (r[0].as_u64(), r[1].as_u64(), r[2].as_u64()));
    output
}

fn hybrid(catalog: &[(String, GrammarMatcher)], fixtures: &[Fixture]) -> (Vec<Value>, Value) {
    let setup = Started::now();
    let docs = fixtures
        .iter()
        .map(|f| {
            (
                tape(catalog[0].1.language, &f.source),
                exec::FileContext {
                    path: Some(f.path.clone()),
                    ..Default::default()
                },
            )
        })
        .collect();
    exec::with_terminals(docs, vec![], |db, batch, _| {
        let mut fragments = vec![];
        for (id, (_, grammar)) in catalog.iter().enumerate() {
            let fragment = Arc::new(Fragment::new(grammar, batch, usize::MAX));
            register(
                db,
                &CString::new(format!("fragment_{id}")).unwrap(),
                batch.clone(),
                fragment.clone(),
            )
            .unwrap();
            fragments.push(fragment);
        }
        let width = fragments.iter().map(|f| f.names.len()).max().unwrap();
        // Only final publication has a common union schema. Producers remain narrow and separately scoped.
        let sql=fragments.iter().enumerate().map(|(id,f)| {
            let captures=(0..width).map(|n| if n<f.names.len(){format!(",c{n}")}else{format!(",NULL AS c{n}")}).collect::<String>();
            format!("SELECT {id} AS producer,file,node,truth,ordinal{captures} FROM fragment_{id} WHERE truth=1 OR truth<0")
        }).collect::<Vec<_>>().join(" UNION ALL ")+" ORDER BY producer,file,node,ordinal";
        let setup = setup.elapsed();
        let prepare = Started::now();
        let mut stmt = db.prepare(&sql).unwrap();
        let prepare = prepare.elapsed();
        let counts = fragments
            .iter()
            .map(|f| matcher::CaptureAnalysis::default().counts(&f.grammar.matcher))
            .collect::<Vec<_>>();
        let trace = super::super::super::tests::ExecutionTrace::start(db);
        let timer = Started::now();
        let mut output = vec![];
        {
            let mut rows = stmt.query([]).unwrap();
            while let Some(row) = rows.next().unwrap() {
                let id = row.get::<_, u32>(0).unwrap() as usize;
                if let Some(value) = read_row(row, 1, &fragments[id], batch, &counts[id]) {
                    let mut record = vec![json!(id)];
                    record.extend(value.as_array().unwrap().iter().cloned());
                    output.push(Value::Array(record));
                }
            }
        }
        let execution = timer.elapsed();
        let statements = trace.count();
        drop(trace);
        assert_eq!(statements, 1);
        let boundaries = fragments
            .iter()
            .map(|f| f.metrics.lock().unwrap().clone())
            .collect::<Vec<_>>();
        assert!(boundaries.iter().all(|b| b.filters == 1));
        let explain = db
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .unwrap()
            .query_map([], |r| r.get::<_, String>(3))
            .unwrap()
            .collect::<Result<Vec<_>>>()
            .unwrap();
        (
            output,
            json!({"sql":sql,"sqlBytes":sql.len(),"statements":statements,"vm":stmt.get_status(rusqlite::StatementStatus::VmStep),"setupCpuNs":setup.cpu,"setupWallNs":setup.wall,"prepareCpuNs":prepare.cpu,"prepareWallNs":prepare.wall,"executionCpuNs":execution.cpu,"executionWallNs":execution.wall,"boundaries":boundaries,"explain":explain}),
        )
    })
}

pub(super) fn measure(catalog: &[(String, GrammarMatcher)], fixtures: &[Fixture]) -> Value {
    let mut runs = vec![];
    for iteration in 0..3 {
        // Alternate order. Both paths parse fresh tapes and assemble their full result.
        let native_run = || {
            let t = Started::now();
            let r = native(catalog, fixtures);
            (r, t.elapsed())
        };
        let hybrid_run = || {
            let t = Started::now();
            let r = hybrid(catalog, fixtures);
            (r, t.elapsed())
        };
        let ((expected, n), ((actual, detail), h)) = if iteration % 2 == 0 {
            (native_run(), hybrid_run())
        } else {
            let h = hybrid_run();
            (native_run(), h)
        };
        assert_eq!(actual, expected, "whole eleven-grammar job");
        eprintln!(
            "whole fragment job native={}ms hybrid={}ms rows={} statements={}",
            n.cpu as f64 / 1e6,
            h.cpu as f64 / 1e6,
            actual.len(),
            detail["statements"]
        );
        runs.push(json!({"nativeCpuNs":n.cpu,"nativeWallNs":n.wall,"hybridCpuNs":h.cpu,"hybridWallNs":h.wall,"rows":actual.len(),"detail":detail}));
    }
    json!({"files":fixtures.len(),"grammars":catalog.len(),"scope":"cold job includes tape parsing, candidate discovery, batch and SQL setup, preparation, matching, result decoding and teardown; policy compilation reported separately","runs":runs})
}
