use super::publication_tests::{check_work, policy_utils};
use serde_json::{Value, json};

fn runs(value: &Value) -> Vec<usize> {
    match value {
        Value::Object(map) => {
            let mut result = map.values().flat_map(runs).collect::<Vec<_>>();
            if let Some(run) = map.get("predicateRun") {
                result.push(run["predicates"].as_u64().unwrap() as usize);
            }
            result
        }
        Value::Array(items) => items.iter().flat_map(runs).collect(),
        _ => vec![],
    }
}

fn predicates(boolean: &str, count: usize) -> Vec<Value> {
    (0..count)
        .map(|i| json!({"regex":if boolean == "all" {format!("(?:.*){{{i}}}")} else {format!("^never{i}$")}}))
        .collect()
}

#[test]
fn predicate_run_profiles_width_and_repetition() {
    let mut samples = vec![];
    for boolean in ["all", "any"] {
        for verdict in ["true", "false", "unknown"] {
            for width in [2, 8, 32] {
                for repeats in [2, 8, 32] {
                    let mut body = predicates(boolean, width);
                    body.push(json!({"matches":"inner"}));
                    let mut utility = json!({});
                    utility[boolean] = json!(body);
                    let utils = json!({"v":utility,"inner":{"value":if verdict=="false" {json!({"eq":99})} else {json!({"gt":0})}}});
                    let mut rule =
                        json!({"kind":if verdict=="unknown" {"identifier"} else {"number"}});
                    rule[boolean] = json!(vec![json!({"matches":"v"}); repeats]);
                    let rules = [rule];
                    let p = policy_utils(&rules, utils);
                    let source = if verdict == "unknown" { "x;" } else { "1;" };
                    let mut r = check_work(&p, &rules, &[source, source], None, 10000);
                    assert_eq!(runs(&r["relations"]), vec![width]);
                    assert_eq!(r["relations"]["lowering"][0]["atomicTerminals"], width + 1);
                    r["dimension"] = json!(format!("{boolean}-{verdict}"));
                    r["width"] = json!(width);
                    r["size"] = json!(repeats);
                    samples.push(r);
                }
            }
        }
    }
    if let Ok(path) = std::env::var("RERAIL_RELATIONAL_PREDICATE_RUN_EVIDENCE") {
        std::fs::write(path, serde_json::to_string_pretty(&samples).unwrap()).unwrap();
    }
}

#[test]
fn predicate_run_decisive_prefix_and_unreached_value_faults() {
    for boolean in ["all", "any"] {
        for width in [2, 8, 32] {
            for at in [0, width - 1] {
                let mut body = predicates(boolean, width);
                body[at] = json!({"regex":if boolean=="all" {"^never$"} else {".*"}});
                body.push(json!({"matches":"inner"}));
                let mut utility = json!({});
                utility[boolean] = json!(body);
                let utils = json!({"v":utility,"inner":{"value":{"gt":0}}});
                let mut repeated = json!({});
                repeated[boolean] = json!(vec![json!({"matches":"v"}); 8]);
                let rules = [json!({"kind":"number","any":[repeated,{"regex":".*"}]})];
                let p = policy_utils(&rules, utils);
                for allowance in [0, 1, 2, 7, 8, 9, 100] {
                    let r = check_work(&p, &rules, &["1;2;", "1;"], Some("1"), allowance);
                    assert_eq!(runs(&r["relations"]), vec![width]);
                    assert!(r["sqlCalls"].as_array().unwrap().is_empty());
                }
            }
        }
    }
}

#[test]
fn predicate_run_preserves_effect_boundaries_and_publication() {
    for boolean in ["all", "any"] {
        for boundary in [
            json!({"testUnknown":".*"}),
            json!({"pattern":"$KEY"}),
            json!({"value":{"gt":0}}),
            json!({"matches":"inner"}),
            json!({"inside":{"kind":"expression_statement","stopBy":"end"}}),
            json!({"not":{"regex":"^never$"}}),
        ] {
            let mut body = predicates(boolean, 2);
            body.push(boundary);
            body.extend(predicates(boolean, 3));
            body.push(json!({"matches":"inner"}));
            let mut utility = json!({});
            utility[boolean] = json!(body);
            let utils = json!({"v":utility,"inner":{"value":{"gt":0}}});
            // Nested use, capture rollback, later memo reuse, independent clauses/files.
            let expression = json!({"any":[{"not":{"matches":"v"}},{"all":[{"matches":"v"},{"pattern":"$OUT"}]},{"matches":"inner"}]});
            let rules = [
                json!({"kind":"number","all":[expression.clone()]}),
                json!({"kind":"identifier","all":[expression]}),
            ];
            let p = policy_utils(&rules, utils);
            for allowance in [0, 1, 2, 3, 4, 6, 8, 12, 100] {
                for fault in [None, Some("2")] {
                    let r = check_work(&p, &rules, &["1;2;x;", "x;3;"], fault, allowance);
                    let grouped = runs(&r["relations"]);
                    assert!(!grouped.is_empty());
                    assert!(grouped.iter().all(|n| [2, 3].contains(n)), "{grouped:?}");
                }
            }
        }
    }
}

#[test]
fn predicate_run_unknown_and_failure_before_and_after_group() {
    let utils = json!({"v":{"value":{"gt":0}}});
    for boolean in ["all", "any"] {
        for before in [false, true] {
            for operand in [json!({"testUnknown":".*"}), json!({"matches":"v"})] {
                let mut children = predicates(boolean, 8);
                if before {
                    children.insert(0, operand);
                } else {
                    children.push(operand);
                }
                // This later nested call requires the shared region and makes
                // cached failures/unknown answers visible after the predicate run.
                children.push(json!({"not":{"not":{"matches":"v"}}}));
                let mut group = json!({});
                group[boolean] = json!(children);
                for variant in [
                    group.clone(),
                    json!({"not":group}),
                    json!({"any":[{"regex":".*"},group]}),
                ] {
                    let rules = [json!({"kind":"number","all":[variant]})];
                    let p = policy_utils(&rules, utils.clone());
                    for allowance in [0, 1, 2, 3, 4, 100] {
                        check_work(&p, &rules, &["1;2;", "2;"], None, allowance);
                        check_work(&p, &rules, &["1;2;", "2;"], Some("2"), allowance);
                    }
                }
            }
        }
    }
}
