use super::*;
fn grammar(gate: serde_json::Value, tail: &str) -> GrammarMatcher {
    let p=CodeRuleDslRuntime::compile(json!({"format":"rerail.policy/v1","id":"prefix","revision":1,"mode":"steer","clauses":[{"id":"c","forbid":{"ast":{"language":"javascript","utils":{"gate":gate,"tail":{"pattern":tail}},"rule":{"all":[{"matches":"gate"},{"matches":"tail"}]}}},"violation":{"code":"P","message":"found","anchor":"witness.span"}}]}).to_string(),"json".into(),CAPABILITIES.into()).unwrap();
    grammar_matchers(&p)[0].clone()
}
fn docs(g: &GrammarMatcher, sources: &[&str]) -> Vec<(TapeDoc, FileContext)> {
    sources
        .iter()
        .map(|s| {
            (
                crate::code_rule_dsl::sql::tests::differential::tape(g.language, s),
                Default::default(),
            )
        })
        .collect()
}
fn runs(count: usize, files: usize) -> Vec<ClauseRun> {
    (0..count)
        .map(|statement| ClauseRun {
            statement,
            files: (0..files).map(|f| (f, None)).collect(),
        })
        .collect()
}
#[test]
fn fragment_batch_factors_complete_definitions_and_preserves_capture_outputs() {
    let gate = json!({"kind":"call_expression","has":{"stopBy":"end","kind":"number"}});
    let a = grammar(gate.clone(), "f($X)");
    let b = grammar(gate, "g($Y)");
    let compiled = compile_enumerating(&[&a, &b], None).unwrap();
    let mut session = compiled.session().unwrap();
    for source in ["f(1);g(2);h(x);", "f(x);g(3);", "f(4);g(x);"] {
        let found = session.run(docs(&a, &[source]), &runs(2, 1)).unwrap();
        for (id, g) in [&a, &b].iter().enumerate() {
            let single = compile_enumerating(&[g], None).unwrap();
            let expected = single
                .session()
                .unwrap()
                .run(docs(g, &[source]), &runs(1, 1))
                .unwrap();
            assert_eq!(found[id][0].matches, expected[0][0].matches);
            assert_eq!(found[id][0].unknown, expected[0][0].unknown);
        }
        assert!(
            session.last_profile["nativeBatches"][0]["prefix_stages"]
                .as_u64()
                .unwrap()
                > 0
        );
        assert_eq!(session.last_profile["nativeBatches"][0]["filters"], 1);
    }
    assert_eq!(session.prepared_statements(), 1);
}
#[test]
fn fragment_batch_never_unions_distinct_candidate_domains() {
    let a = grammar(json!({"kind":"call_expression"}), "f($X)");
    let compiled = compile_enumerating(&[&a, &a], None).unwrap();
    let doc = docs(&a, &["f(1);f(2);"]);
    let first = compiled
        .session()
        .unwrap()
        .run(doc.clone(), &runs(2, 1))
        .unwrap()[0][0][0]
        .root;
    let mut demands = runs(2, 1);
    demands[1].files[0].1 = Some(Arc::from([first]));
    let mut session = compiled.session().unwrap();
    let found = session.run(doc, &demands).unwrap();
    assert_eq!(found[0][0].len(), 2);
    assert_eq!(found[1][0].len(), 1);
    assert_eq!(session.last_profile["nativeBatches"][0]["prefix_stages"], 0);
}
#[test]
fn fragment_batch_different_definitions_and_unknowns_remain_private() {
    for other in [
        json!({"kind":"call_expression","has":{"kind":"identifier","stopBy":"end"}}),
        json!({"testUnknown":"f"}),
    ] {
        let a = grammar(
            json!({"kind":"call_expression","has":{"kind":"number","stopBy":"end"}}),
            "f($X)",
        );
        let b = grammar(other, "g($Y)");
        let compiled = compile_enumerating(&[&a, &b], None).unwrap();
        let mut session = compiled.session().unwrap();
        let found = session.run(docs(&a, &["f(1);g(x);"]), &runs(2, 1)).unwrap();
        for (id, g) in [&a, &b].iter().enumerate() {
            let single = compile_enumerating(&[g], None).unwrap();
            let expected = single
                .session()
                .unwrap()
                .run(docs(g, &["f(1);g(x);"]), &runs(1, 1))
                .unwrap();
            assert_eq!(found[id][0].matches, expected[0][0].matches);
            assert_eq!(found[id][0].unknown, expected[0][0].unknown);
        }
        assert_eq!(session.last_profile["nativeBatches"][0]["prefix_stages"], 0);
        assert_eq!(session.last_profile["nativeBatches"][0]["grouping_roots"], 0);
    }
}
#[test]
fn fragment_batch_prefix_failure_is_file_local_and_empty_results_complete() {
    let a = grammar(json!({"kind":"call_expression"}), "f($X)");
    let mut compiler = ProgramCompiler::default();
    for _ in 0..2 {
        let id = compiler.clause(&a, None).unwrap();
        compiler.clauses[id].allowance = Some((2, 0));
    }
    let compiled = compiler.finish().0;
    let mut session = compiled.session().unwrap();
    let found = session
        .run(docs(&a, &["f(1);f(2);", "g(1);", "1;"]), &runs(2, 3))
        .unwrap();
    for rule in found {
        assert!(rule[0].unknown.is_some());
        assert!(rule[0].is_empty());
        assert!(rule[1].unknown.is_none());
        assert!(rule[1].is_empty());
        assert!(rule[2].unknown.is_none());
        assert!(rule[2].is_empty());
    }
}

#[test]
fn fragment_batch_does_not_materialize_anchor_implied_kind_prefixes() {
    let a = policy(json!({"kind":"identifier","pattern":"$A"}));
    let b = policy(json!({"kind":"identifier","pattern":"$B"}));
    let a = grammar_matchers(&a)[0].clone();
    let b = grammar_matchers(&b)[0].clone();
    let compiled = compile_enumerating(&[&a, &b], None).unwrap();
    let mut session = compiled.session().unwrap();
    let found = session.run(docs(&a, &["x;y;"]), &runs(2, 1)).unwrap();
    assert_eq!(found[0][0].len(), 2);
    assert_eq!(found[1][0].len(), 2);
    assert_eq!(session.last_profile["nativeBatches"][0]["prefix_roots"], 0);
}

#[test]
fn fragment_batch_parent_seed_intersects_sparse_domain_and_survives_edit() {
    let p = policy(json!({"regex":"^[xy]$","inside":{"stopBy":"neighbor","kind":"arguments"}}));
    let g = grammar_matchers(&p)[0].clone();
    let compiled = compile_enumerating(&[&g], None).unwrap();
    assert!(compiled.queries[0].contains("candidate_files_0"));
    let mut session = compiled.session().unwrap();
    for source in ["f(x,y);g(x);", "g(y,x);f(y);"] {
        let all = session.run(docs(&g, &[source]), &runs(1, 1)).unwrap();
        assert_eq!(all[0][0].len(), 3);
        let selected = all[0][0][1].root;
        let sparse = [ClauseRun {
            statement: 0,
            files: vec![(0, Some(Arc::from([selected])))],
        }];
        let found = session.run(docs(&g, &[source]), &sparse).unwrap();
        assert_eq!(found[0][0].len(), 1);
        assert_eq!(found[0][0][0].root, selected);
    }
}

#[test]
fn fragment_batch_qualifies_parents_before_child_expansion() {
    let source =
        include_str!("../../../../../packs/python/rules/ruff-f707-default-except-not-last.json");
    let policy =
        CodeRuleDslRuntime::compile(source.into(), "json".into(), CAPABILITIES.into()).unwrap();
    let g = grammar_matchers(&policy)[0].clone();
    let mut compiled = compile_enumerating(&[&g], None).unwrap();
    let current = planning::demand(&compiled.clauses[0], 0, 2);
    assert!(current.contains("AS MATERIALIZED"));
    let mut optimized = compiled.session().unwrap();
    let parent = compiled.clauses[0]
        .template
        .predicate
        .as_ref()
        .unwrap()
        .parent_seed()
        .unwrap();
    let prior = format!(
        "SELECT c.request,c.file,c.node FROM candidate_files_0 f CROSS JOIN node p ON p.file=f.file CROSS JOIN node n ON n.file=p.file AND n.parent=p.id CROSS JOIN candidates_0 c ON c.file=n.file AND c.node=n.id WHERE {}",
        parent.sql("p", 0)
    );
    let query = compiled.queries[0].replace(&current, &prior);
    assert_ne!(query, compiled.queries[0]);
    compiled.queries = Arc::new(vec![query]);
    let mut baseline = compiled.session().unwrap();
    let source = "try:\n    pass\nexcept:\n    f(0)\nexcept Exception:\n    pass\n".repeat(256);
    let documents = docs(&g, &[&source]);
    let old = baseline.run(documents.clone(), &runs(1, 1)).unwrap();
    let new = optimized.run(documents, &runs(1, 1)).unwrap();
    assert_eq!(new[0][0].matches, old[0][0].matches);
    assert_eq!(new[0][0].unknown, old[0][0].unknown);
    assert!(!new[0][0].matches.is_empty());
    let old_vm = baseline.last_profile["statements"][0]["vm"]
        .as_u64()
        .unwrap();
    let new_vm = optimized.last_profile["statements"][0]["vm"]
        .as_u64()
        .unwrap();
    eprintln!(
        "qualifying parents: VM {old_vm} -> {new_vm}; CPU {} -> {}",
        baseline.last_profile["statements"][0]["cpuNs"],
        optimized.last_profile["statements"][0]["cpuNs"]
    );
    assert!(
        new_vm < old_vm,
        "parent materialization must reduce repeated work"
    );
    assert!(
        serde_json::to_string(&optimized.plans().unwrap())
            .unwrap()
            .contains("MATERIALIZE qualifying_parent")
    );
}
