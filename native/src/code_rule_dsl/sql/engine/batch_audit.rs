//! Diagnostic experiments; none changes the production compiler or its execution budget.
use super::*;
use serde_json::json;

fn same(a: &[Vec<FileOutcome>], b: &[Vec<FileOutcome>]) {
    assert_eq!(a.len(), b.len());
    for (a, b) in a.iter().zip(b) {
        assert_eq!(a.len(), b.len());
        for (a, b) in a.iter().zip(b) {
            assert_eq!(a.unknown, b.unknown);
            assert_eq!(a.matches, b.matches);
        }
    }
}

fn kind_truth(p: &structural::Predicate, kind: u16) -> Option<bool> {
    use structural::Predicate as P;
    Some(match p {
        P::False => false,
        P::Kinds(k) => k.contains(&kind),
        P::All(v) => v
            .iter()
            .map(|p| kind_truth(p, kind))
            .collect::<Option<Vec<_>>>()?
            .into_iter()
            .all(|b| b),
        P::Any(v) => v
            .iter()
            .map(|p| kind_truth(p, kind))
            .collect::<Option<Vec<_>>>()?
            .into_iter()
            .any(|b| b),
        P::Not(p) => !kind_truth(p, kind)?,
        _ => return None,
    })
}

#[test]
#[ignore = "diagnostic: isolated SQL arms, memo lifetime, and shared async producer"]
fn fragment_batch_optimization_audit() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut paths = std::fs::read_dir(root.join("packs/python/rules"))
        .unwrap()
        .map(|p| p.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect::<Vec<_>>();
    paths.sort();
    let catalog = rules(paths.clone());
    let fixtures = crate::code_rule_dsl::sql::relational::cutover_tests::execution::fixtures(root);
    let docs = fixtures
        .iter()
        .map(|f| {
            (
                crate::code_rule_dsl::sql::tests::differential::tape(
                    catalog[0].grammar.language,
                    &f.source,
                ),
                FileContext {
                    path: Some(f.path.clone()),
                    ..Default::default()
                },
            )
        })
        .collect::<Vec<_>>();
    let all_files = (0..docs.len()).map(|f| (f, None)).collect::<Vec<_>>();
    let mut memo = vec![];
    let expected = native(&catalog, &fixtures);
    for iteration in 0..3 {
        for share in if iteration % 2 == 0 {
            [false, true]
        } else {
            [true, false]
        } {
            let t = Started::now();
            let actual = native_memo(&catalog, &fixtures, share);
            let elapsed = t.elapsed();
            same(&actual, &expected);
            memo.push(json!({"shared":share,"cpuNs":elapsed.cpu,"wallNs":elapsed.wall}));
        }
    }
    let mut arms = vec![];
    for (id, rule) in catalog.iter().enumerate() {
        let mut compiler = ProgramCompiler::default();
        compiler
            .clause(&rule.grammar, rule.each.as_deref())
            .unwrap();
        let mut compiled = compiler.finish().0;
        let Some(predicate) = compiled.clauses[0].template.predicate.as_ref() else {
            continue;
        };
        let implied = rule.grammar.anchors.root_kinds().is_some_and(|kinds| {
            kinds
                .iter()
                .all(|k| kind_truth(predicate, *k) == Some(true))
        });
        let mut session = compiled.session().unwrap();
        let runs = [ClauseRun {
            statement: 0,
            files: all_files.clone(),
        }];
        let actual = session.run(docs.clone(), &runs).unwrap();
        same(&actual, &expected[id..id + 1]);
        let mut result = json!({"id":id,"rule":rule.name,"kindPredicateImpliedByAnchors":implied,
            "sql":*compiled.queries,"plans":session.plans().unwrap(),"baseline":session.last_profile});
        if implied {
            let prefix = compiled.queries[0].split(" FROM ").next().unwrap();
            compiled.queries = Arc::new(vec![if compiled.clauses[0].template.native.is_some() {
                format!("{prefix} FROM matcher_0 WHERE truth=1 OR truth<0")
            } else {
                format!("{prefix} FROM candidates_0")
            }]);
            let mut optimized = compiled.session().unwrap();
            let found = optimized.run(docs.clone(), &runs).unwrap();
            same(&found, &actual);
            result["withoutRedundantFilter"] = optimized.last_profile;
            result["optimizedSql"] = json!(*compiled.queries);
        }
        arms.push(result);
    }

    // Six authored rules start with the same two capture-free utilities. This experiment
    // factors those exact definitions, preserving each consumer's original anchor domain.
    let mut originals = ProgramCompiler::default();
    let mut residuals = ProgramCompiler::default();
    let mut gate = None;
    let mut async_names = vec![];
    let mut utility_definitions = None;
    for path in paths {
        let mut source: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let ast = &mut source["clauses"][0]["forbid"]["ast"];
        let Some(all) = ast["rule"]["all"].as_array() else {
            continue;
        };
        if all.len() != 3
            || all[0] != json!({"matches":"callee-position"})
            || all[1] != json!({"matches":"async-context"})
        {
            continue;
        }
        let definitions = [
            "callee-position",
            "async-context",
            "scope-body",
            "lambda-body",
        ]
        .map(|n| ast["utils"][n].clone());
        assert!(definitions.iter().all(|d| !d.is_null()));
        if let Some(prior) = &utility_definitions {
            assert_eq!(prior, &definitions);
        }
        utility_definitions = Some(definitions);
        let compile = |source: &serde_json::Value| {
            let p =
                CodeRuleDslRuntime::compile(source.to_string(), "json".into(), CAPABILITIES.into())
                    .unwrap();
            crate::code_rule_dsl::test_support::grammar_matchers(&p)[0].clone()
        };
        let original = compile(&source);
        originals.clause(&original, None).unwrap();
        async_names.push(source["id"].clone());
        let residual = source["clauses"][0]["forbid"]["ast"]["rule"]["all"][2].clone();
        if gate.is_none() {
            let mut producer = source.clone();
            producer["clauses"][0]["forbid"]["ast"]["rule"] =
                json!({"all":[{"matches":"callee-position"},{"matches":"async-context"}]});
            gate = Some(compile(&producer));
        }
        source["clauses"][0]["forbid"]["ast"]["rule"] = residual;
        let mut remainder = compile(&source);
        remainder.anchors = original.anchors.clone();
        residuals.clause(&remainder, None).unwrap();
    }
    assert_eq!(async_names.len(), 6);
    let originals = originals.finish().0;
    let residuals = residuals.finish().0;
    let gate = compile_enumerating(&[&gate.unwrap()], None).unwrap();
    assert!(gate.clauses[0].template.names.is_empty());
    let batch = Batch::of(docs.clone(), Arc::new(Program { terminals: vec![] }));
    let mut domains = vec![];
    let mut union = vec![BTreeSet::new(); docs.len()];
    for c in originals.clauses.iter() {
        let f = Fragment::new(&c.grammar, &batch, usize::MAX);
        let domain = f
            .requests
            .iter()
            .map(|r| (r.file, r.node))
            .collect::<Vec<_>>();
        for &(file, node) in &domain {
            union[file as usize].insert(node);
        }
        domains.push(domain);
    }
    let union_rows: usize = union.iter().map(BTreeSet::len).sum();
    let gate_runs = [ClauseRun {
        statement: 0,
        files: union
            .into_iter()
            .enumerate()
            .map(|(file, nodes)| (file, Some(Arc::from(nodes.into_iter().collect::<Vec<_>>()))))
            .collect(),
    }];
    let runs = (0..6)
        .map(|statement| ClauseRun {
            statement,
            files: all_files.clone(),
        })
        .collect::<Vec<_>>();
    let mut baseline_session = originals.session().unwrap();
    let mut gate_session = gate.session().unwrap();
    let mut residual_session = residuals.session().unwrap();
    let mut sharing = vec![];
    for iteration in 0..3 {
        let mut baseline_output = None;
        let mut shared_output = None;
        for shared in if iteration % 2 == 0 {
            [false, true]
        } else {
            [true, false]
        } {
            let t = Started::now();
            let (output, metrics) = if shared {
                let admitted = gate_session.run(docs.clone(), &gate_runs).unwrap();
                assert!(admitted.iter().flatten().all(|f| f.unknown.is_none()));
                let rows: usize = admitted[0].iter().map(|f| f.len()).sum();
                let filtered = (0..6)
                    .map(|statement| ClauseRun {
                        statement,
                        files: admitted[0]
                            .iter()
                            .enumerate()
                            .map(|(file, f)| {
                                (
                                    file,
                                    Some(Arc::from(
                                        f.matches.iter().map(|m| m.root).collect::<Vec<_>>(),
                                    )),
                                )
                            })
                            .collect(),
                    })
                    .collect::<Vec<_>>();
                let output = residual_session.run(docs.clone(), &filtered).unwrap();
                (
                    output,
                    json!({"admitted":rows,"producer":gate_session.last_profile,"consumers":residual_session.last_profile}),
                )
            } else {
                (
                    baseline_session.run(docs.clone(), &runs).unwrap(),
                    baseline_session.last_profile.clone(),
                )
            };
            let elapsed = t.elapsed();
            assert!(output.iter().flatten().all(|f| f.unknown.is_none()));
            sharing.push(
                json!({"shared":shared,"cpuNs":elapsed.cpu,"wallNs":elapsed.wall,"detail":metrics}),
            );
            if shared {
                shared_output = Some(output)
            } else {
                baseline_output = Some(output)
            }
        }
        same(&baseline_output.unwrap(), &shared_output.unwrap());
    }
    let report = json!({"scope":"597 catalog fixtures; matching diagnostics, not whole-job speedups",
        "files":fixtures.len(),"rules":catalog.len(),"memo":memo,"arms":arms,
        "async":{"rules":async_names,"inputRowsPerRule":domains.iter().map(Vec::len).collect::<Vec<_>>(),
            "identicalDomainPairs":(0..6).flat_map(|a|(a+1..6).map(move|b|(a,b))).filter(|(a,b)|domains[*a]==domains[*b]).collect::<Vec<_>>(),
            "unionRows":union_rows,"runs":sharing}});
    std::fs::write(
        std::env::var("RERAIL_FRAGMENT_BATCH_AUDIT").expect("output path"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
}
