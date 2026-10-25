use super::*;
use crate::code_rule_dsl::{
    CodeRuleDslRuntime,
    test_support::{CAPABILITIES, grammar_matchers},
};
use serde_json::json;
fn policy(rule: serde_json::Value) -> crate::code_rule_dsl::CompiledCodeRuleDsl {
    CodeRuleDslRuntime::compile(json!({"id":"fragment-integration","format":"rerail.policy/v1","revision":1,"mode":"steer","clauses":[{"id":"check","forbid":{"ast":{"language":"javascript","rule":rule}},"violation":{"code":"check","message":"found","anchor":"witness.span"}}]}).to_string(),"json".into(),CAPABILITIES.into()).unwrap()
}
#[test]
fn fragment_integration_prepares_once_and_replaces_batch() {
    let p = policy(json!({"pattern":"f($X)"}));
    let g = &grammar_matchers(&p)[0];
    let mut builder = ProgramCompiler::default();
    builder.clause(g, None).unwrap();
    builder.clause(g, None).unwrap();
    let compiled = builder.finish().0;
    assert!(compiled.hits >= 1);
    let mut session = compiled.session().unwrap();
    let runs = [
        ClauseRun {
            statement: 0,
            files: vec![(0, None)],
        },
        ClauseRun {
            statement: 1,
            files: vec![(0, None)],
        },
    ];
    for (source, count) in [("f(a); f(b);", 2), ("g(a);", 0), ("f(c);", 1)] {
        let docs = vec![(
            crate::code_rule_dsl::sql::tests::differential::tape(g.language, source),
            FileContext::default(),
        )];
        let found = session.run(docs, &runs).unwrap();
        assert_eq!(found[0][0].len(), count);
        assert_eq!(found[1][0].len(), count);
        assert!(found.iter().flatten().all(|f| f.unknown.is_none()));
        assert_eq!(session.prepared_statements(), 1);
        assert_eq!(session.last_profile["boundaries"][0]["filters"], 1);
    }
    assert_eq!(session.stats().statements, 3);
}

#[test]
fn fragment_integration_structural_sql_native_binding_parity() {
    let rules = [
        json!({"kind":"call_expression","has":{"kind":"identifier","stopBy":"end"}}),
        json!({"kind":"identifier","inside":{"kind":"call_expression","stopBy":"end"}}),
        json!({"kind":"identifier","not":{"inside":{"kind":"arguments","stopBy":"end"}}}),
        json!({"kind":"identifier","follows":{"kind":"identifier","stopBy":"end"}}),
        json!({"kind":"identifier","precedes":{"kind":"identifier","stopBy":"neighbor"}}),
        json!({"kind":"call_expression","count":{"has":{"kind":"identifier","stopBy":"end"},"min":2}}),
        json!({"pattern":"f($X)","has":{"kind":"number","stopBy":"end"}}),
    ];
    for rule in rules {
        let p = policy(rule.clone());
        let g = &grammar_matchers(&p)[0];
        let source = "f(1); f(x); g(a,b); const z = f(2);";
        let doc = crate::code_rule_dsl::sql::tests::differential::tape(g.language, source);
        let tree = ast_grep_core::AstGrep::doc(doc.clone());
        let mut expected = vec![];
        let mut work = usize::MAX;
        matcher::with_file_memo(&tree, || {
            matcher::run_clause(
                &tree,
                g,
                None,
                doc.transducer_len(),
                &mut work,
                |node, _, _| {
                    expected.push(node);
                    Ok::<_, ()>(true)
                },
            )
            .unwrap()
            .ok()
            .expect("native completes");
        });
        let compiled = compile_enumerating(&[g], None).unwrap();
        assert!(compiled.clauses[0].template.predicate.is_some(), "{rule}");
        let mut session = compiled.session().unwrap();
        let actual = session
            .run(
                vec![(doc, FileContext::default())],
                &[ClauseRun {
                    statement: 0,
                    files: vec![(0, None)],
                }],
            )
            .unwrap();
        assert_eq!(
            actual[0][0].iter().map(|m| m.root).collect::<Vec<_>>(),
            expected,
            "{rule}"
        );
        assert_eq!(session.stats().statements, 1);
        assert!(actual[0][0].unknown.is_none());
    }
}

#[test]
fn fragment_integration_statistics_flip_shape_not_findings() {
    let p = policy(json!({"kind":"call_expression","has":{"kind":"identifier","stopBy":"end"}}));
    let g = &grammar_matchers(&p)[0];
    let mut builder = ProgramCompiler::default();
    builder.clause(g, None).unwrap();
    let roots = g
        .anchors
        .root_kinds()
        .unwrap()
        .iter()
        .copied()
        .collect::<Vec<_>>();
    let leaves = builder.clauses[0]
        .template
        .predicate
        .as_ref()
        .unwrap()
        .rare_descendant()
        .unwrap()
        .to_vec();
    for (r, l, want_leaf) in [(10000, 1, true), (1, 10000, false)] {
        let mut stats = statistics::PlannerStats::default();
        for k in &roots {
            stats.kinds.insert(*k, r);
        }
        for k in &leaves {
            stats.kinds.insert(*k, l);
        }
        assert_eq!(
            planning::decisions(&builder.clauses, &stats),
            vec![want_leaf]
        );
        let queries = planning::queries(&builder.clauses, &stats);
        let compiled = Compiled {
            clauses: Arc::new(builder.clauses.clone()),
            queries: Arc::new(queries),
            hits: 0,
            registry: serde_json::Value::Null,
            statistics: stats,
            statistics_error: None,
        };
        let mut session = compiled.session().unwrap();
        let found = session
            .run(
                vec![(
                    crate::code_rule_dsl::sql::tests::differential::tape(g.language, "f(x);g(1);"),
                    Default::default(),
                )],
                &[ClauseRun {
                    statement: 0,
                    files: vec![(0, None)],
                }],
            )
            .unwrap();
        assert_eq!(found[0][0].len(), 2); // Both function identifiers are descendants.
    }
}

#[test]
fn fragment_integration_isolates_budget_failures_and_recovers() {
    let mut p:serde_json::Value=serde_json::from_str(&json!({"format":"rerail.policy/v1","id":"limits","revision":1,"mode":"steer","clauses":[{"id":"c","forbid":{"ast":{"language":"typescript","rule":{"kind":"number","matches":"u"},"utils":{"u":{"kind":"number"}}}},"violation":{"code":"L","message":"found","anchor":"witness.span"}}]}).to_string()).unwrap();
    let p = CodeRuleDslRuntime::compile(p.take().to_string(), "json".into(), CAPABILITIES.into())
        .unwrap();
    let g = &grammar_matchers(&p)[0];
    let mut builder = ProgramCompiler::default();
    builder.clause(g, None).unwrap();
    builder.clauses[0].allowance = Some((1, 0));
    let compiled = builder.finish().0;
    let mut session = compiled.session().unwrap();
    let found = session
        .run(
            ["1;2;", "1;"]
                .iter()
                .map(|s| {
                    (
                        crate::code_rule_dsl::sql::tests::differential::tape(g.language, s),
                        Default::default(),
                    )
                })
                .collect(),
            &[ClauseRun {
                statement: 0,
                files: vec![(0, None), (1, None)],
            }],
        )
        .unwrap();
    assert!(found[0][0].unknown.is_some());
    assert!(found[0][0].is_empty());
    assert_eq!(found[0][1].len(), 1);
    assert!(found[0][1].unknown.is_none());
    let again = session
        .run(
            vec![(
                crate::code_rule_dsl::sql::tests::differential::tape(g.language, "1;"),
                Default::default(),
            )],
            &[ClauseRun {
                statement: 0,
                files: vec![(0, None)],
            }],
        )
        .unwrap();
    assert!(again[0][0].unknown.is_none());
    assert_eq!(again[0][0].len(), 1);
}

#[path = "qualification.rs"]
mod qualification;

#[test]
fn fragment_integration_logical_unknown_is_not_incomplete_coverage() {
    let p = policy(json!({"kind":"identifier","testUnknown":"^x$"}));
    let g = &grammar_matchers(&p)[0];
    let compiled = compile_enumerating(&[g], None).unwrap();
    let mut session = compiled.session().unwrap();
    let found = session
        .run(
            vec![(
                crate::code_rule_dsl::sql::tests::differential::tape(g.language, "x;y;"),
                Default::default(),
            )],
            &[ClauseRun {
                statement: 0,
                files: vec![(0, None)],
            }],
        )
        .unwrap();
    assert_eq!(found[0][0].len(), 1);
    assert!(found[0][0].unknown.is_none());
    assert_eq!(
        session.last_profile["boundaries"][0]["logical_unknown_roots"],
        1
    );
}
