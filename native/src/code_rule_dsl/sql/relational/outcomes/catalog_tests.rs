//! Explicit atomic contracts needed by catalog rules. Composition stays in SQL.
use crate::code_rule_dsl::{
    CodeRuleDslRuntime,
    matcher::{self, Stopped},
    sql::{exec, profiling, tests::differential::tape},
    test_support::{CAPABILITIES, grammar_matchers},
};
use ast_grep_core::AstGrep;
use serde_json::{Value, json};

fn policy(language: &str, rule: Value, utils: Value) -> crate::code_rule_dsl::CompiledCodeRuleDsl {
    CodeRuleDslRuntime::compile(
        json!({"format":"rerail.policy/v1","id":"catalog-atoms","revision":1,"mode":"steer",
        "clauses":[{"id":"c0","forbid":{"ast":{"language":language,"rule":rule,"utils":utils}},
        "violation":{"code":"C0","message":"match","anchor":"witness.span"}}]})
        .to_string(),
        "json".into(),
        CAPABILITIES.into(),
    )
    .unwrap()
}

/// Compare the clause's roots, remaining allowance and failure publication on every file.
/// These rules bind nothing. Unlike the Value-only probe helper, this accepts other leaf kinds.
fn check_atomic_budget(
    language: &str,
    rule: Value,
    utils: Value,
    sources: &[&str],
    allowance: usize,
) -> Value {
    let policy = policy(language, rule.clone(), utils);
    let grammar = &grammar_matchers(&policy)[0];
    let docs: Vec<_> = sources.iter().map(|s| tape(grammar.language, s)).collect();
    let mut expected = vec![];
    for (file, doc) in docs.iter().enumerate() {
        let tree = AstGrep::doc(doc.clone());
        let _scopes =
            matcher::FileScopeSlot::install(Default::default(), Default::default(), false);
        let mut work = allowance;
        let mut rows = vec![];
        let result = matcher::with_file_memo(&tree, || {
            matcher::run_clause(
                &tree,
                grammar,
                None,
                doc.transducer_len(),
                &mut work,
                |node, _, left| {
                    rows.push(json!([file, node, 0, *left]));
                    Ok::<_, ()>(true)
                },
            )
            .unwrap()
        });
        match result {
            Ok(()) => expected.extend(rows),
            Err(Stopped::Limit(limit)) => expected.push(json!([
                file,
                null,
                super::super::failure::encode(&format!("{limit:?}")).unwrap(),
                work
            ])),
            Err(Stopped::Deadline) => panic!("unexpected deadline"),
        }
    }
    let compiled = super::compile_job(&[grammar], allowance as i64).unwrap();
    let ((actual, executions), frames) = profiling::frame_counts(|| {
        exec::with_terminals(
            docs.into_iter()
                .map(|doc| (doc, Default::default()))
                .collect(),
            compiled.terminals,
            |db, batch, _| {
                super::super::leaves::register(db, batch).unwrap();
                let mut stmt = db.prepare(&compiled.sql).unwrap();
                let trace = super::super::tests::ExecutionTrace::start(db);
                let rows = stmt
                    .query_map([], |row| {
                        let tag = row.get::<_, i64>(4)?;
                        let captures: Value =
                            serde_json::from_str(&row.get::<_, String>(3)?).unwrap();
                        assert_eq!(captures, json!([]));
                        Ok(json!([
                            row.get::<_, u32>(1)?,
                            if tag < 0 {
                                None
                            } else {
                                Some(row.get::<_, u32>(2)?)
                            },
                            tag,
                            row.get::<_, i64>(5)?
                        ]))
                    })
                    .unwrap()
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .unwrap();
                (rows, trace.count())
            },
        )
    });
    let mut actual = actual;
    actual.sort_by_key(|row| (row[0].as_u64(), row[1].as_u64()));
    expected.sort_by_key(|row| (row[0].as_u64(), row[1].as_u64()));
    assert_eq!(
        actual, expected,
        "rule={rule}, allowance={allowance}, sources={sources:?}"
    );
    assert_eq!(executions, 1);
    assert_eq!(frames[4], 0);
    json!(actual)
}

#[test]
fn catalog_atomic_resolution_preserves_scope_and_utility_work() {
    for (language, atom, sources) in [
        (
            "python",
            json!({"resolves":{"global":["print"]}}),
            vec![
                "print(1)\ndef f(print):\n    print(2)\nprint(3)\n",
                "print = 1\nprint(2)\n",
            ],
        ),
        (
            "python",
            json!({"resolves":{"import":["requests.get"]}}),
            vec![
                "import requests as r\nr.get('a')\ndef f(r):\n    r.get('b')\n",
                "from requests import get\nget('c')\n",
            ],
        ),
        (
            "typescript",
            json!({"resolves":{"global":["console"]}}),
            vec![
                "console.log(1); function f(console) { console.log(2); }",
                "const console = x; console.log(3);",
            ],
        ),
    ] {
        for rule in [
            json!({"all":[{"matches":"atom"},{"matches":"atom"}]}),
            json!({"any":[{"matches":"atom"},{"matches":"atom"}]}),
            json!({"not":{"matches":"atom"}}),
        ] {
            for allowance in [0, 1, 2, 3, 8, 64, 1024] {
                check_atomic_budget(
                    language,
                    rule.clone(),
                    json!({"atom":atom}),
                    &sources,
                    allowance,
                );
            }
        }
    }
}

#[test]
fn catalog_atomic_layout_preserves_reached_neighbors_and_work() {
    for atom in [
        json!({"layout":{"span":{"eq":0}}}),
        json!({"layout":{"before":{"to":"token","comments":false,"breaks":{"eq":1}}}}),
        json!({"layout":{"after":{"to":"sibling","comments":false,"blank":{"eq":0}}}}),
    ] {
        for body in [
            json!({"all":[{"matches":"atom"},{"matches":"atom"}]}),
            json!({"any":[{"matches":"atom"},{"matches":"atom"}]}),
            json!({"not":{"matches":"atom"}}),
        ] {
            let rule = json!({"kind":"expression_statement","all":[body]});
            for allowance in [0, 1, 2, 3, 8, 64, 1024] {
                check_atomic_budget(
                    "typescript",
                    rule.clone(),
                    json!({"atom":atom}),
                    &["a;\n// comment\n// second\nb;\n\nc;", "x; y;\nz;"],
                    allowance,
                );
            }
        }
    }
}

#[test]
fn catalog_atomic_absent_arguments_preserve_spreads_and_utility_work() {
    for (language, kind, atom, sources) in [
        (
            "python",
            "call",
            json!({"arguments":{"signature":["x","*","strict"],"absent":["strict"]}}),
            vec![
                "f(1)\nf(1, strict=True)\nf(*xs)\nf(**kw)\nf(**{'strict': True})\nf(**{'x': 1})\n",
                "f(1, **{'strict': False}, **kw)\nf(1, strict=True, **{'strict': False})\nf(x for x in xs)\n",
            ],
        ),
        (
            "python",
            "call",
            json!({"arguments":{"signature":["x","*","strict"],"absent":["strict"],"mode":"syntactic"}}),
            vec![
                "f(1)\nf(1, strict=True)\nf(*xs)\nf(**kw)\nf(**{'strict': True})\n",
                "f(strict=False, **kw)\n",
            ],
        ),
        (
            "typescript",
            "call_expression",
            json!({"arguments":{"signature":["x","y"],"absent":["y"]}}),
            vec![
                "f(); f(1); f(1,2); f(...xs); f(1,...xs);",
                "f(...[1]); f(...[1,2]);",
            ],
        ),
    ] {
        for expression in [
            json!({"all":[{"matches":"atom"},{"matches":"atom"}]}),
            json!({"any":[{"matches":"atom"},{"matches":"atom"}]}),
            json!({"not":{"matches":"atom"}}),
        ] {
            for allowance in [0, 1, 2, 3, 8, 64, 1024] {
                check_atomic_budget(
                    language,
                    json!({"kind":kind,"all":[expression.clone()]}),
                    json!({"atom":atom}),
                    &sources,
                    allowance,
                );
            }
        }
    }
}

#[test]
fn catalog_atomic_followed_values_preserve_unknown_failure_and_memo() {
    use super::publication_tests::{check_work, policy_utils};
    let atom = json!({"value":{"follow":true,"eq":2}});
    for expression in [
        json!({"all":[{"matches":"v"},{"matches":"v"}]}),
        json!({"any":[{"matches":"v"},{"matches":"v"}]}),
        json!({"not":{"matches":"v"}}),
        json!({"any":[{"regex":"^x$"},{"matches":"v"}]}),
    ] {
        let rules = [json!({"kind":"identifier","all":[expression]})];
        let policy = policy_utils(&rules, json!({"v":atom}));
        for allowance in [0, 1, 2, 3, 4, 8, 32, 1024] {
            for fault in [None, Some("x")] {
                check_work(
                    &policy,
                    &rules,
                    &["const x = 2; const y = x; f(x,y,z);", "const x = 3; f(x);"],
                    fault,
                    allowance,
                );
            }
        }
    }
}

#[test]
fn catalog_atomic_refuses_parsed_exports_without_a_typed_contract() {
    let p = policy(
        "python",
        json!({"parse":{"grammar":"testChars","has":{"kind":"char","capture":"C"}}}),
        json!({}),
    );
    let grammar = &grammar_matchers(&p)[0];
    let error = super::compile(grammar)
        .err()
        .expect("no parsed export contract");
    assert!(
        error.contains("exported spans need a typed contract"),
        "{error}"
    );
}

#[test]
fn catalog_followed_values_nested_and_shared_memo_exclude_cache_fills() {
    use super::publication_tests::{check_work, policy_utils};
    let utilities = json!({
        "v":{"value":{"follow":true,"eq":2}},
        "nested":{"all":[{"matches":"v"},{"matches":"v"}]}
    });
    for expression in [
        json!({"all":[{"matches":"nested"},{"matches":"nested"}]}),
        json!({"any":[{"all":[{"matches":"v"},{"regex":"^never$"}]},
                        {"all":[{"matches":"v"},{"matches":"v"}]}]}),
    ] {
        let rules = [json!({"kind":"identifier","all":[expression]})];
        let p = policy_utils(&rules, utilities.clone());
        for allowance in 0..=48 {
            check_work(
                &p,
                &rules,
                &["const x = 2; const y = x; f(x,y,z);", "const x = 3; f(x);"],
                None,
                allowance,
            );
        }
        for fault in [Some("x"), Some("y")] {
            check_work(
                &p,
                &rules,
                &["const x = 2; const y = x; f(x,y,z);", "const x = 3; f(x);"],
                fault,
                1024,
            );
        }
    }
}

#[test]
fn catalog_followed_values_repeated_search_preserves_one_time_fill_costs() {
    use super::publication_tests::{check_work, policy_utils};
    let search = json!({"has":{"stopBy":"end","kind":"identifier","matches":"v"}});
    let rules = [json!({"kind":"arguments","all":[search.clone(), search.clone(), search]})];
    let p = policy_utils(&rules, json!({"v":{"value":{"follow":true,"eq":2}}}));
    for allowance in 0..=64 {
        check_work(
            &p,
            &rules,
            &["const x = 2; f(x,y); g(x);", "const x = 3; f(x,y);"],
            None,
            allowance,
        );
    }
    for fault in [Some("x"), Some("y")] {
        check_work(
            &p,
            &rules,
            &["const x = 2; f(x,y); g(x);", "const x = 3; f(x,y);"],
            fault,
            1024,
        );
    }
}

#[test]
fn catalog_followed_value_scope_charges_are_per_clause_and_file() {
    use super::publication_tests::{check_work, policy_utils};
    let rule = json!({"kind":"identifier","all":[{"matches":"v"},{"matches":"v"}]});
    let rules = [rule.clone(), rule];
    let p = policy_utils(&rules, json!({"v":{"value":{"follow":true,"eq":2}}}));
    for allowance in 0..=64 {
        check_work(
            &p,
            &rules,
            &["const x = 2; f(x,y);", "const x = 3; f(x);"],
            None,
            allowance,
        );
    }
}

#[test]
fn catalog_followed_scope_costs_reset_for_each_prepared_job() {
    use super::publication_tests::policy_utils;
    use crate::code_rule_dsl::sql::terminals::BatchScopes;
    let rule = json!({"kind":"identifier","all":[{"matches":"v"},{"matches":"v"}]});
    let p = policy_utils(&[rule], json!({"v":{"value":{"follow":true,"eq":2}}}));
    let grammar = &grammar_matchers(&p)[0];
    let compiled = super::compile_job(&[grammar], 64).unwrap();
    exec::with_terminals(
        vec![(
            tape(grammar.language, "const x = 2; f(x,y);"),
            Default::default(),
        )],
        compiled.terminals,
        |db, batch, _| {
            super::super::leaves::register(db, batch).unwrap();
            let mut stmt = db.prepare(&compiled.sql).unwrap();
            let mut outputs = Vec::new();
            for _ in 0..3 {
                // This is the execution boundary used by Session::run. A prepared statement
                // reused within one BatchScopes lifetime still belongs to that same job.
                let _job = BatchScopes::begin();
                outputs.push(
                    stmt.query_map([], |r| {
                        Ok(json!([
                            r.get::<_, u32>(0)?,
                            r.get::<_, u32>(1)?,
                            r.get::<_, Option<u32>>(2)?,
                            r.get::<_, String>(3)?,
                            r.get::<_, i64>(4)?,
                            r.get::<_, i64>(5)?
                        ]))
                    })
                    .unwrap()
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .unwrap(),
                );
            }
            assert!(!outputs[0].is_empty());
            assert_eq!(outputs[0], outputs[1]);
            assert_eq!(outputs[1], outputs[2]);
        },
    );
}
