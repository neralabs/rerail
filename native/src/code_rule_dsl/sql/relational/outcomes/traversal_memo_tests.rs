use super::publication_tests::{check_work, policy_utils};
use serde_json::{Value, json};

#[test]
fn traversal_memo_native_boundary_and_admission() {
    use crate::code_rule_dsl::{
        matcher::{self, value_probe},
        sql::tests::differential::tape,
    };
    use ast_grep_core::AstGrep;
    let search = json!({"has":{"all":[{"kind":"number"},{"matches":"v"}],"stopBy":"end"}});
    let rule = json!({"kind":"arguments","all":[search.clone(),search]});
    let utils = json!({"v":{"value":{"gt":0}}});
    let rules = [rule];
    let p = policy_utils(&rules, utils);
    let crate::code_rule_dsl::model::ClauseKind::Forbid(crate::code_rule_dsl::model::Query::Ast {
        matchers,
        ..
    }) = &p.inner.clauses[0].kind
    else {
        panic!("AST")
    };
    let grammar = &matchers[0];
    for (source, boundary) in [("f(1,2);", 4), ("f(g(1));", 8)] {
        for allowance in [boundary - 1, boundary] {
            let doc = tape(grammar.language, source);
            let tree = AstGrep::doc(doc.clone());
            let _scopes =
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
            println!(
                "TRAVERSAL_NATIVE_GATE {}",
                json!({"source":source,"allowance":allowance,"remaining":work,"matches":matches,"result":match &result {Ok(())=>"ok".into(),Err(matcher::Stopped::Limit(limit))=>format!("{limit:?}"),Err(matcher::Stopped::Deadline)=>"deadline".into()},"calls":calls})
            );
            assert_eq!(result.is_ok(), allowance == boundary);
            assert_eq!(work, 0);
            assert_eq!(calls, vec!["1"]);
        }
    }
    for source in ["f(1,2);", "f(g(1));"] {
        for allowance in 0..=9 {
            check_work(&p, &rules, &[source], None, allowance);
        }
    }
}

fn search(body: Value) -> Value {
    json!({"has":{"all":[{"kind":"number"},body],"stopBy":"end"}})
}
fn check(rule: Value, utils: Value, sources: &[&str], max: i64, fault: Option<&str>) {
    let rules = [rule, json!({"kind":"number","pattern":"$OTHER"})];
    let p = policy_utils(&rules, utils);
    for allowance in 0..=max {
        check_work(&p, &rules, sources, fault, allowance);
    }
}

#[test]
fn traversal_memo_false_unknown_and_decisive_stops() {
    for (kind, source) in [("number", "f(g(1),2,3);"), ("identifier", "f(g(x),y,z);")] {
        for value in [json!({"gt":0}), json!({"eq":99})] {
            for captured in [false, true] {
                let mut body = json!({"all":[{"kind":kind},{"matches":"v"}],"stopBy":"end"});
                if captured {
                    body["pattern"] = json!("$X");
                }
                let search = json!({"has":body});
                for term in [
                    json!({"all":[search.clone(),search.clone()]}),
                    json!({"not":{"any":[search.clone(),search.clone()]}}),
                ] {
                    check(
                        json!({"kind":"arguments","all":[term]}),
                        json!({"v":{"value":value}}),
                        &[source, source],
                        20,
                        None,
                    );
                }
            }
        }
    }
}

#[test]
fn traversal_memo_capture_keys_rollback_and_nested_utilities() {
    for (rule, utils) in [
        (
            json!({"kind":"arguments","all":[search(json!({"matches":"v"})),{"pattern":"$X"},search(json!({"matches":"v"})),search(json!({"matches":"v"}))]}),
            json!({"v":{"value":{"gt":0}}}),
        ),
        (
            json!({"kind":"arguments","all":[{"not":{"not":search(json!({"matches":"v"}))}},search(json!({"matches":"v"})),search(json!({"matches":"v"}))]}),
            json!({"v":{"pattern":"$X","value":{"gt":0}}}),
        ),
        (
            json!({"kind":"arguments","all":[search(json!({"matches":"outer"})),search(json!({"matches":"v"})),search(json!({"matches":"outer"}))]}),
            json!({"v":{"value":{"gt":0}},"outer":{"all":[{"matches":"v"},{"matches":"v"}]}}),
        ),
        (
            json!({"kind":"arguments","all":[{"matches":"outer"},search(json!({"matches":"v"})),{"matches":"outer"}]}),
            json!({"v":{"value":{"gt":0}},"outer":search(json!({"matches":"v"}))}),
        ),
        (
            json!({"kind":"arguments","all":[search(json!({"matches":"left"})),search(json!({"matches":"right"})),search(json!({"matches":"left"}))]}),
            json!({"left":{"value":{"gt":0}},"right":{"value":{"gt":0}}}),
        ),
    ] {
        check(rule, utils, &["f(g(1),2);", "f(3,4);"], 24, None);
    }
}

#[test]
fn traversal_memo_counts_stops_and_axes() {
    for rule in [
        json!({"kind":"arguments","all":[{"count":{"has":{"all":[{"kind":"number"},{"matches":"v"}],"stopBy":"end"},"min":0,"max":8,"as":"N"}},search(json!({"matches":"v"}))]}),
        json!({"kind":"arguments","has":{"kind":"number","regex":"^99$","stopBy":{"all":[{"kind":"number"},{"matches":"v"}]}}}),
        json!({"kind":"number","all":[{"follows":{"all":[{"kind":"number"},{"matches":"v"}],"stopBy":"end"}},{"follows":{"all":[{"kind":"number"},{"matches":"v"}],"stopBy":"end"}}]}),
        json!({"kind":"number","all":[{"precedes":{"all":[{"kind":"number"},{"matches":"v"}],"stopBy":"end"}},{"precedes":{"all":[{"kind":"number"},{"matches":"v"}],"stopBy":"end"}}]}),
        json!({"kind":"number","all":[{"inside":{"all":[{"kind":"arguments"},{"matches":"v"}],"stopBy":"end"}},{"inside":{"all":[{"kind":"arguments"},{"matches":"v"}],"stopBy":"end"}}]}),
    ] {
        check(
            rule,
            json!({"v":{"value":{"gt":0}}}),
            &["f(g(1),2,3);", "f(4,5);"],
            30,
            None,
        );
    }
}

#[test]
fn traversal_memo_reached_failure_and_clause_isolation() {
    let s = search(json!({"matches":"v"}));
    for terms in [
        json!({"all":[s.clone(),s.clone()]}),
        json!({"any":[s.clone(),s.clone()]}),
        json!({"any":[{"regex":".*"},s.clone()]}),
        json!({"any":[{"all":[{"regex":"^never$"},s.clone()]},s.clone()]}),
    ] {
        let rule = json!({"kind":"arguments","all":[terms]});
        let rules = [rule.clone(), rule];
        let p = policy_utils(&rules, json!({"v":{"value":{"eq":99}}}));
        for fault in [None, Some("2")] {
            for allowance in [0, 1, 2, 3, 4, 5, 6, 10, 100] {
                check_work(&p, &rules, &["f(g(1),2,3);", "f(4,5,6);"], fault, allowance);
            }
        }
    }
}

#[test]
fn traversal_memo_profile() {
    let mut samples = vec![];
    for dimension in [
        "repeats",
        "overlap",
        "disjoint",
        "changed-captures",
        "early-stop",
    ] {
        for size in if dimension == "changed-captures" {
            [2, 4, 8]
        } else {
            [2, 8, 32]
        } {
            let s = search(json!({"matches":"v"}));
            let calls = match dimension {
                "repeats" => vec![s; size],
                "changed-captures" => {
                    let mut calls = vec![s.clone()];
                    for version in 0..size {
                        calls.push(json!({"pattern":format!("$X{version}")}));
                        calls.extend([s.clone(), s.clone()]);
                    }
                    calls
                }
                _ => vec![s.clone(), s],
            };
            let source = match dimension {
                "overlap" => format!("{}1{};", "f(".repeat(size), ")".repeat(size)),
                "disjoint" => "f(1);".repeat(size),
                "early-stop" => format!("f({});", vec!["1"; size].join(",")),
                _ => "f(1,2);".into(),
            };
            let rules = [json!({"kind":"arguments","all":calls})];
            let p = policy_utils(&rules, json!({"v":{"value":{"gt":0}}}));
            let mut r = check_work(&p, &rules, &[&source, &source], None, 10000);
            r["dimension"] = json!(dimension);
            r["size"] = json!(size);
            samples.push(r);
        }
    }
    if let Ok(path) = std::env::var("RERAIL_RELATIONAL_TRAVERSAL_MEMO_EVIDENCE") {
        std::fs::write(path, serde_json::to_string_pretty(&samples).unwrap()).unwrap();
    }
}
