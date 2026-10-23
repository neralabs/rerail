use super::publication_tests::{check_work, policy_utils};
use serde_json::{Value, json};

fn fixture(boolean: &str, verdict: &str, size: usize) -> (Value, Value, &'static str) {
    let kind = if verdict == "unknown" {
        "identifier"
    } else {
        "number"
    };
    let mut rule = json!({"kind":kind});
    rule[boolean] = json!(vec![json!({"matches":"v"}); size]);
    let utils =
        json!({"v":{"value":if verdict=="false" {json!({"eq":99})} else {json!({"gt":0})}}});
    (rule, utils, if verdict == "unknown" { "x;" } else { "1;" })
}

fn groups(value: &Value) -> usize {
    match value {
        Value::Object(map) => {
            usize::from(map.contains_key("repeatUtility")) + map.values().map(groups).sum::<usize>()
        }
        Value::Array(items) => items.iter().map(groups).sum(),
        _ => 0,
    }
}

#[test]
fn repeated_utility_simple_profiles_and_boundaries() {
    let mut samples = vec![];
    for boolean in ["all", "any"] {
        for verdict in ["true", "false", "unknown"] {
            for size in [2, 8, 32] {
                let (rule, utils, source) = fixture(boolean, verdict, size);
                let rules = [rule];
                let p = policy_utils(&rules, utils);
                let boundary = if (boolean == "any" && verdict == "true")
                    || (boolean == "all" && verdict == "false")
                {
                    2
                } else {
                    2 * size as i64
                };
                for allowance in [0, 1, boundary - 1, boundary, boundary + 1] {
                    check_work(&p, &rules, &[source, source], None, allowance);
                }
                let mut r = check_work(&p, &rules, &[source, source], None, 10000);
                assert_eq!(groups(&r["relations"]), 1);
                r["dimension"] = json!(format!("{boolean}-{verdict}-simple"));
                r["size"] = json!(size);
                samples.push(r);
            }
        }
    }
    if let Ok(path) = std::env::var("RERAIL_RELATIONAL_REPEAT_UTILITY_EVIDENCE") {
        std::fs::write(path, serde_json::to_string_pretty(&samples).unwrap()).unwrap();
    }
}

#[test]
fn repeated_utility_unknown_fallbacks_unreached_calls_and_failures() {
    for boolean in ["all", "any"] {
        for verdict in ["true", "false", "unknown"] {
            let (group, utils, source) = fixture(boolean, verdict, 32);
            let kind = if verdict == "unknown" {
                "identifier"
            } else {
                "number"
            };
            for variant in [
                json!({"kind":kind,"any":[group.clone(),{"regex":".*"}]}),
                json!({"kind":kind,"all":[{"regex":"^never$"},group.clone()]}),
                json!({"kind":kind,"any":[{"regex":".*"},group.clone()]}),
                json!({"kind":kind,"not":group}),
            ] {
                let rules = [variant, json!({"kind":"number","pattern":"$OTHER"})];
                let p = policy_utils(&rules, utils.clone());
                for allowance in [0, 1, 2, 3, 63, 64, 65, 128] {
                    check_work(&p, &rules, &[source, "1;2;", "x;"], None, allowance);
                    check_work(&p, &rules, &[source, "1;2;", "x;"], Some("2"), allowance);
                }
            }
        }
    }
}

#[test]
fn repeated_utility_nested_answers_survive_later_uses() {
    let utils = json!({"v":{"all":[{"regex":".*"},{"matches":"inner"}]},"inner":{"value":{"gt":0}},"w":{"value":{"eq":99}}});
    let call = json!({"matches":"v"});
    let first = json!({"all":vec![call.clone();4]});
    let second = json!({"any":vec![call.clone();4]});
    for combination in [
        json!({"all":[first.clone(),second.clone(),{"matches":"inner"},call.clone()]}),
        json!({"any":[{"all":[first.clone(),{"matches":"w"}]},second.clone(),{"matches":"inner"}]}),
        json!({"all":[{"not":{"not":first.clone()}},second.clone(),call.clone()]}),
        json!({"any":[{"not":first},second,{"matches":"inner"}]}),
    ] {
        // Different clauses use separate utility identities and answer lifetimes.
        let rules = [
            json!({"kind":"number","all":[combination.clone()]}),
            json!({"kind":"identifier","all":[combination]}),
        ];
        let p = policy_utils(&rules, utils.clone());
        for allowance in [0, 1, 2, 3, 11, 12, 13, 24, 27, 30, 100] {
            check_work(&p, &rules, &["1;2;x;", "3;x;"], None, allowance);
            check_work(&p, &rules, &["1;2;x;", "3;x;"], Some("2"), allowance);
        }
    }
}

#[test]
fn repeated_utility_excludes_changed_keys_distinct_calls_and_branch_boundaries() {
    let utils = json!({"v":{"value":{"gt":0}},"w":{"value":{"gt":0}},"capture":{"pattern":"$KEY","value":{"gt":0}},"walk":{"has":{"kind":"number","value":{"gt":0},"stopBy":"end"}}});
    for rule in [
        json!({"kind":"number","all":[{"matches":"v"},{"matches":"w"},{"matches":"v"}]}),
        json!({"kind":"number","all":[{"matches":"v"},{"pattern":"$KEY"},{"matches":"v"}]}),
        json!({"kind":"number","pattern":"$KEY","all":[{"matches":"v"},{"matches":"v"}]}),
        json!({"kind":"number","all":[{"matches":"capture"},{"matches":"capture"}]}),
        json!({"kind":"number","any":[{"all":[{"regex":"^2$"},{"matches":"v"}]},{"matches":"v"}]}),
        json!({"kind":"number","all":[{"matches":"v"},{"not":{"matches":"v"}}]}),
        json!({"kind":"arguments","all":[{"matches":"walk"},{"matches":"walk"}]}),
    ] {
        let rules = [rule];
        let p = policy_utils(&rules, utils.clone());
        for allowance in [0, 1, 2, 3, 4, 6, 10, 100] {
            let r = check_work(&p, &rules, &["f(1,2,x);", "f(2);"], None, allowance);
            assert_eq!(groups(&r["relations"]), 0);
        }
    }
}

#[test]
fn repeated_utility_measures_prefix_rejection_without_value_reads() {
    for boolean in ["all", "any"] {
        for nested in [false, true] {
            let leaf = if nested {
                json!({"matches":"inner"})
            } else {
                json!({"value":{"gt":0}})
            };
            let utils = json!({"v":{"all":[{"regex":"^never$"},leaf]},"inner":{"value":{"gt":0}}});
            let mut repeated = json!({});
            repeated[boolean] = json!(vec![json!({"matches":"v"}); 8]);
            let rules = [json!({"kind":"number","any":[repeated,{"regex":".*"}]})];
            let p = policy_utils(&rules, utils);
            for allowance in (0..=9).chain([16, 17]) {
                let r = check_work(&p, &rules, &["1;2;", "1;"], Some("1"), allowance);
                assert_eq!(groups(&r["relations"]), 1);
                assert!(r["sqlCalls"].as_array().unwrap().is_empty());
            }
        }
    }
}
