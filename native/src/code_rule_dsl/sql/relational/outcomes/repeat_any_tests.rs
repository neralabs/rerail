use super::publication_tests::{check_work, policy_utils};
use serde_json::{Value, json};

fn search(kind: &str, utility: &str) -> Value {
    json!({"has":{"all":[{"kind":kind},{"matches":utility}],"stopBy":"end"}})
}
fn repeated(kind: &str, count: usize) -> Value {
    json!({"kind":"arguments","any":vec![search(kind,"v"); count]})
}
fn check(rule: Value, utils: Value, sources: &[&str], maximum: i64, fault: Option<&str>) {
    let rules = [rule, json!({"kind":"number","pattern":"$OTHER"})];
    let p = policy_utils(&rules, utils);
    for allowance in 0..=maximum {
        check_work(&p, &rules, sources, fault, allowance);
    }
}

#[test]
fn repeat_any_native_boundaries() {
    use crate::code_rule_dsl::{
        matcher::{self, value_probe},
        sql::tests::differential::tape,
    };
    use ast_grep_core::AstGrep;
    for (kind, source, value, boundary, reads, matched) in [
        ("number", "f(1,2);", json!({"gt":0}), 2, vec!["1"], 1),
        ("number", "f(1,2);", json!({"eq":99}), 12, vec!["1", "2"], 0),
        (
            "identifier",
            "f(x,y);",
            json!({"gt":0}),
            12,
            vec!["x", "y"],
            0,
        ),
        ("number", "f(g(1));", json!({"gt":0}), 4, vec!["1"], 2),
    ] {
        let rules = [repeated(kind, 3)];
        let p = policy_utils(&rules, json!({"v":{"value":value}}));
        let crate::code_rule_dsl::model::ClauseKind::Forbid(
            crate::code_rule_dsl::model::Query::Ast { matchers, .. },
        ) = &p.inner.clauses[0].kind
        else {
            panic!("AST")
        };
        let grammar = &matchers[0];
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
            println!(
                "REPEAT_ANY_NATIVE_GATE {}",
                json!({"source":source,"value":value,"allowance":allowance,"remaining":work,"matches":matches,"calls":calls,"success":result.is_ok()})
            );
            assert_eq!(result.is_ok(), allowance == boundary);
            assert_eq!(work, 0);
            assert_eq!(calls, reads);
            if allowance == boundary {
                assert_eq!(matches, matched);
            }
        }
        for allowance in 0..=boundary + 1 {
            check_work(&p, &rules, &[source], None, allowance as i64);
        }
    }
}

#[test]
fn repeat_any_profile() {
    let mut samples = vec![];
    for dimension in [
        "true",
        "false",
        "unknown",
        "true-fallback",
        "false-fallback",
        "unknown-fallback",
    ] {
        for size in [2, 8, 32] {
            let unknown = dimension.starts_with("unknown");
            let mut rule = repeated(if unknown { "identifier" } else { "number" }, size);
            if dimension.ends_with("fallback") {
                rule["any"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"regex":".*"}));
            }
            let source = if unknown { "f(x,y);" } else { "f(1,2);" };
            let rules = [rule];
            let p = policy_utils(
                &rules,
                json!({"v":{"value": if dimension.starts_with("false") {json!({"eq":99})} else {json!({"gt":0})}}}),
            );
            let mut r = check_work(&p, &rules, &[source, source], None, 10000);
            r["dimension"] = json!(dimension);
            r["size"] = json!(size);
            samples.push(r);
        }
    }
    if let Ok(path) = std::env::var("RERAIL_RELATIONAL_REPEAT_ANY_EVIDENCE") {
        std::fs::write(path, serde_json::to_string_pretty(&samples).unwrap()).unwrap();
    }
}
fn groups(value: &Value) -> Vec<usize> {
    match value {
        Value::Object(o) => o
            .get("repeatTraversal")
            .map(|g| g["calls"].as_u64().unwrap() as usize)
            .into_iter()
            .chain(o.values().flat_map(groups))
            .collect(),
        Value::Array(a) => a.iter().flat_map(groups).collect(),
        _ => vec![],
    }
}

#[test]
fn repeat_any_preserves_open_unknown_and_fallbacks() {
    for (kind, source, value) in [
        ("number", "f(1,2);", json!({"gt":0})),
        ("number", "f(1,2);", json!({"eq":99})),
        ("identifier", "f(x,y);", json!({"gt":0})),
    ] {
        let s = search(kind, "v");
        let sequence = json!({"any":vec![s.clone();3]});
        for term in [
            sequence.clone(),
            json!({"any":[s.clone(),s.clone(),s.clone(),{"regex":".*"}]}),
            json!({"any":[s.clone(),s.clone(),s.clone(),{"regex":"^never$"}]}),
            json!({"any":[{"testUnknown":".*"},s.clone(),s.clone(),s.clone()]}),
            json!({"any":[{"testUnknown":".*"},s.clone(),s.clone(),s.clone(),{"pattern":"$WINNER"}]}),
            json!({"any":[{"regex":".*"},s.clone(),s.clone(),s.clone()]}),
            json!({"any":[{"all":[{"testUnknown":".*"},{"pattern":"$BLOCKED"}]},s.clone(),s.clone(),s.clone()]}),
            json!({"not":sequence.clone()}),
            json!({"all":[{"testUnknown":".*"},sequence.clone()]}),
            json!({"all":[sequence,{"pattern":"$AFTER"}]}),
        ] {
            check(
                json!({"kind":"arguments","all":[term]}),
                json!({"v":{"value":value}}),
                &[source, source],
                14,
                None,
            );
        }
    }
}

#[test]
fn repeat_any_proof_excludes_effects_and_nonidentical_searches() {
    let plain = search("number", "v");
    let value = json!({"has":{"kind":"number","value":{"gt":0},"stopBy":"end"}});
    let stopped =
        json!({"has":{"all":[{"kind":"number"},{"matches":"v"}],"stopBy":{"regex":"^never$"}}});
    let counted =
        json!({"count":{"has":{"all":[{"kind":"number"},{"matches":"v"}],"stopBy":"end"},"min":1}});
    for terms in [
        vec![value.clone(), value],
        vec![stopped.clone(), stopped],
        vec![counted.clone(), counted],
        vec![plain.clone(), json!({"pattern":"$X"}), plain.clone()],
        vec![plain.clone(), search("number", "other")],
        vec![plain.clone(), search("identifier", "v")],
        vec![json!({"has":{"all":[{"testUnknown":".*"},{"matches":"v"}],"stopBy":"end"}}); 2],
        vec![json!({"has":{"all":[{"kind":"number"},{"matches":"v"}]}}); 2],
        vec![search("number", "captured"); 2],
    ] {
        let rules = [json!({"kind":"arguments","any":terms})];
        let p = policy_utils(
            &rules,
            json!({"v":{"value":{"eq":99}},"other":{"value":{"eq":99}},"captured":{"pattern":"$X","value":{"eq":99}}}),
        );
        for allowance in 0..=10 {
            let r = check_work(&p, &rules, &["f(1,x,2);"], None, allowance);
            assert!(groups(&r["relations"]).is_empty());
        }
    }
}

#[test]
fn repeat_any_nested_costs_and_stable_incoming_captures() {
    let s = search("number", "v");
    for value in [json!({"gt":0}), json!({"eq":99})] {
        for rule in [
            json!({"kind":"arguments","pattern":"$X","any":[s.clone(),s.clone(),s.clone(),{"pattern":"$WINNER"}]}),
            json!({"kind":"arguments","any":[s.clone(),s.clone(),{"regex":"^never$"},s.clone(),s.clone(),{"regex":".*"}]}),
            json!({"kind":"arguments","matches":"outer"}),
        ] {
            check(
                rule,
                json!({"inner":{"value":value},"v":{"any":[{"matches":"inner"},{"matches":"inner"}]},"outer":{"any":[s.clone(),s.clone(),s.clone(),{"regex":".*"}]}}),
                &["f(g(1),2);", "f(3,4);"],
                32,
                None,
            );
        }
    }
}

#[test]
fn repeat_any_reached_failures_and_provisional_publication() {
    for value in [json!({"gt":0}), json!({"eq":99})] {
        let mut rule = repeated("number", 4);
        rule["any"]
            .as_array_mut()
            .unwrap()
            .push(json!({"regex":".*"}));
        let rules = [rule.clone(), rule];
        let p = policy_utils(&rules, json!({"v":{"value":value}}));
        for source in ["f(1);g(2);h(3);", "f(g(1),2);h(3);"] {
            for allowance in [0, 1, 2, 3, 5, 7, 8, 9, 12, 16, 100] {
                for fault in [None, Some("2")] {
                    check_work(&p, &rules, &[source, "f(4,5);"], fault, allowance);
                }
            }
        }
        // A later value leaf must be reached after a false group, but not after true.
        let s = search("number", "v");
        check(
            json!({"kind":"arguments","any":[s.clone(),s.clone(),s,{"value":{"gt":0}}]}),
            json!({"v":{"value":value}}),
            &["f(1,2);"],
            16,
            Some("(1,2)"),
        );
    }
}

#[test]
fn repeat_any_zero_cost_and_signed_limit() {
    for source in ["f();", "f(1);", "f(x);"] {
        for kind in ["number", "identifier"] {
            let mut rule = repeated(kind, 32);
            rule["any"]
                .as_array_mut()
                .unwrap()
                .push(json!({"pattern":"$FALLBACK"}));
            let rules = [rule];
            let p = policy_utils(&rules, json!({"v":{"value":{"eq":99}}}));
            for allowance in [0, 1, 2, 62, 63, 64, 65, i64::MAX] {
                let r = check_work(&p, &rules, &[source], None, allowance);
                assert_eq!(groups(&r["relations"]), vec![32]);
                assert!(r["sql"].as_str().unwrap().len() < 16000);
            }
        }
    }
}
