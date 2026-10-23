use super::publication_tests::{check_work, policy_utils};
use crate::code_rule_dsl::model::{ClauseKind, Query};
use serde_json::{Value, json};

fn fixture(boolean: &str, verdict: &str, size: usize) -> (Value, Value, &'static str) {
    let kind = if verdict == "unknown" {
        "identifier"
    } else {
        "number"
    };
    let search = json!({"matches":"v"});
    let mut rule = json!({"kind":kind});
    rule[boolean] = json!(vec![search; size]);
    let predicates = [".*", "^", "$", ".", "^.*$", ".+", "[0-9xy]", "[^z]"];
    let mut body = predicates
        .iter()
        .map(|p| json!({"regex":p}))
        .collect::<Vec<_>>();
    body.push(json!({"matches":"inner"}));
    let utils = json!({"v":{"all":body},"inner":{"value":if verdict=="false" {json!({"eq":99})}else{json!({"gt":0})}}});
    (rule, utils, if verdict == "unknown" { "x;" } else { "1;" })
}

#[test]
fn composition_budget_native_and_admission_boundaries() {
    use crate::code_rule_dsl::{
        matcher::{self, value_probe},
        sql::tests::differential::tape,
    };
    use ast_grep_core::AstGrep;
    let mut evidence = vec![];
    let mut rejected = vec![];
    for boolean in ["all", "any"] {
        for verdict in ["true", "false", "unknown"] {
            let (rule, utils, source) = fixture(boolean, verdict, 32);
            let p = policy_utils(&[rule], utils);
            let ClauseKind::Forbid(Query::Ast { matchers, .. }) = &p.inner.clauses[0].kind else {
                panic!("AST")
            };
            let grammar = &matchers[0];
            let boundary = match (boolean, verdict) {
                ("all", "true") => 96,
                ("any", "true") => 3,
                ("all", "false") => 3,
                _ => 96,
            };
            for allowance in [boundary - 1, boundary] {
                let doc = tape(grammar.language, source);
                let tree = AstGrep::doc(doc.clone());
                let _scope =
                    matcher::FileScopeSlot::install(Default::default(), Default::default(), false);
                let mut work = allowance;
                let mut matches = 0;
                let (result, calls) = value_probe::run_reached(None, || {
                    matcher::with_file_memo(&tree, || {
                        matcher::run_clause(
                            &tree,
                            grammar,
                            None,
                            doc.transducer_len(),
                            &mut work,
                            |_, _, _| {
                                matches += 1;
                                Ok::<_, ()>(true)
                            },
                        )
                        .unwrap()
                    })
                });
                assert_eq!(result.is_ok(), allowance == boundary);
                assert_eq!(work, 0);
                assert_eq!(
                    calls,
                    if verdict != "unknown" {
                        vec!["1"]
                    } else {
                        vec!["x"]
                    }
                );
                if allowance == boundary {
                    assert_eq!(matches, usize::from(verdict == "true"));
                }
                evidence.push(json!({"boolean":boolean,"verdict":verdict,"allowance":allowance,"success":result.is_ok(),"remaining":work,"calls":calls,"matches":matches}));
            }
            let compiled = match super::compile_job(&[grammar], 10000) {
                Ok(compiled) => compiled,
                Err(error) => {
                    rejected.push(json!({"boolean":boolean,"verdict":verdict,"error":error}));
                    continue;
                }
            };
            assert_eq!(compiled.terminals.len(), 9);
            assert_eq!(
                compiled.relations["lowering"][0]["compositionOperators"],
                46
            );
            assert_eq!(compiled.relations["lowering"][0]["utilityBodyLowerings"], 2);
            let (rule, utils, _) = fixture(boolean, verdict, 32);
            let rules = [rule];
            let p = policy_utils(&rules, utils);
            for allowance in [0, boundary - 1, boundary, boundary + 1] {
                check_work(&p, &rules, &[source, source], None, allowance as i64);
            }
        }
    }
    println!("COMPOSITION_BUDGET_NATIVE {}", json!(evidence));
    println!("COMPOSITION_BUDGET_REJECTIONS {}", json!(rejected));
    assert!(rejected.is_empty());
}

#[test]
fn composition_budget_profile() {
    let mut samples = vec![];
    for boolean in ["all", "any"] {
        for verdict in ["true", "false", "unknown"] {
            for size in [2, 8, 32] {
                let (rule, utils, source) = fixture(boolean, verdict, size);
                let rules = [rule];
                let p = policy_utils(&rules, utils);
                let mut r = check_work(&p, &rules, &[source, source], None, 10000);
                r["dimension"] = json!(format!("{boolean}-{verdict}-wide"));
                r["size"] = json!(size);
                samples.push(r);
            }
        }
    }
    if let Ok(path) = std::env::var("RERAIL_RELATIONAL_COMPOSITION_BUDGET_EVIDENCE") {
        std::fs::write(path, serde_json::to_string_pretty(&samples).unwrap()).unwrap();
    }
}

#[test]
fn composition_budget_captures_fallbacks_and_failures() {
    for boolean in ["all", "any"] {
        for verdict in ["true", "false", "unknown"] {
            let (rule, utils, source) = fixture(boolean, verdict, 3);
            let kind = if verdict == "unknown" {
                "identifier"
            } else {
                "number"
            };
            for variant in [
                rule.clone(),
                json!({"kind":kind,"pattern":"$INPUT","all":[rule.clone()]}),
                json!({"kind":kind,"any":[rule.clone(),{"pattern":"$FALLBACK"}]}),
                json!({"kind":kind,"not":rule}),
            ] {
                let rules = [variant, json!({"kind":"number","pattern":"$OTHER"})];
                let p = policy_utils(&rules, utils.clone());
                for allowance in 0..=12 {
                    check_work(&p, &rules, &[source, "1;2;"], None, allowance);
                }
                for allowance in [0, 2, 3, 8, 9, 100] {
                    check_work(&p, &rules, &[source, "1;2;"], Some("2"), allowance);
                }
            }
        }
    }
}

#[test]
fn composition_budget_compact_nested_graph_and_key_changes() {
    // This graph has 127 logical utility calls, but the same planner emits only
    // seven miss bodies. It used to fail conservative admission at 256 operators.
    let mut utils = json!({"u0":{"value":{"gt":0}}});
    for n in 1..=6 {
        utils[format!("u{n}")] =
            json!({"all":[{"matches":format!("u{}",n-1)},{"matches":format!("u{}",n-1)}]});
    }
    let rules = [json!({"kind":"number","matches":"u6"})];
    let p = policy_utils(&rules, utils);
    for allowance in [0, 1, 2, 190, 191, 192, 382] {
        check_work(&p, &rules, &["1;", "1;2;"], None, allowance);
        check_work(&p, &rules, &["1;", "1;2;"], Some("2"), allowance);
    }
    let (_, utils, _) = fixture("all", "true", 2);
    let rules = [
        json!({"kind":"number","all":[{"matches":"v"},{"pattern":"$KEY"},{"matches":"v"}]}),
        json!({"kind":"number","matches":"v"}),
    ];
    let p = policy_utils(&rules, utils);
    for allowance in 0..=12 {
        check_work(&p, &rules, &["1;2;", "1;"], None, allowance);
        check_work(&p, &rules, &["1;2;", "1;"], Some("2"), allowance);
    }
}

#[test]
fn composition_budget_does_not_assume_a_capture_replay() {
    // Merely repeating the same utility ID does not prove replay under the
    // existing shared-region planner when captures are live.
    let (rule, utils, _) = fixture("all", "true", 32);
    let rules = [json!({"kind":"number","pattern":"$KEY","all":[rule]})];
    let p = policy_utils(&rules, utils);
    let ClauseKind::Forbid(Query::Ast { matchers, .. }) = &p.inner.clauses[0].kind else {
        panic!("AST")
    };
    let error = super::compile_job(&[&matchers[0]], 10000)
        .err()
        .expect("unproved replays expand");
    assert!(
        error.contains("eight key versions")
            || error.contains("emitted composition operator count"),
        "{error}"
    );
}
