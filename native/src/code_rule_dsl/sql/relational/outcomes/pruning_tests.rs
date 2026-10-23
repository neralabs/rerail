//! Candidate pruning is observable through native utility charges, including on failed searches.
use super::publication_tests::{check_work, policy_utils};
use serde_json::{Value, json};

fn check(rule: Value, utils: Value, source: &str, maximum: i64) -> Vec<Value> {
    // Negation publishes the remaining allowance when the search is false.
    let rules = [rule.clone(), json!({"not":rule})];
    let policy = policy_utils(&rules, utils);
    (0..=maximum)
        .map(|work| check_work(&policy, &rules, &[source, source], None, work))
        .collect()
}

#[test]
fn pruning_preserves_one_charge_and_three_valued_visitation() {
    let rule = json!({"kind":"arguments","has":{"matches":"number"}});
    let utils = json!({"number":{"kind":"number"}});
    for three in [false, true] {
        let rule = if three {
            json!({"all":[{"testUnknown":"^never$"},rule.clone()]})
        } else {
            rule.clone()
        };
        let rules = [rule];
        let policy = policy_utils(&rules, utils.clone());
        for work in 0..=3 {
            let r = check_work(&policy, &rules, &["f(1,2,3);"], None, work);
            assert_eq!(
                r["output"][0][4],
                if work < if three { 2 } else { 1 } {
                    -2
                } else {
                    0
                }
            );
            if work == 1 && !three {
                assert_eq!(r["output"][0][5], 0);
                assert_eq!(r["nativeTraces"][0]["candidateVisits"], 2);
            }
        }
    }
}

#[test]
fn pruning_preserves_native_kind_and_text_analysis() {
    for util in [
        json!({"kind":"number","regex":"^7$"}),
        json!({"pattern":"7"}),
        json!({"any":[{"kind":"number","regex":"^7$"},{"kind":"identifier","regex":"^goal$"}]}),
        json!({"not":{"not":{"kind":"number","regex":"^7$"}}}),
        json!({"all":[{"kind":"number"},{"kind":"identifier"}]}),
        json!({"regex":"^7$"}), // No kind bound: native does not use text postings.
    ] {
        check(
            json!({"kind":"arguments","has":{"matches":"u","stopBy":"end"}}),
            json!({"u":util}),
            "f(a,1,2,7,goal);",
            12,
        );
    }
    for regex in ["^(alpha|goal)$", "^é$", "^a\\.b$", "^a.*z$"] {
        check(
            json!({"kind":"arguments","has":{"matches":"u","stopBy":"end"}}),
            json!({"u":{"kind":"identifier","regex":regex}}),
            "f(x,alpha,goal,é,abz);",
            6,
        );
    }
}

#[test]
fn pruning_text_bounds_apply_only_to_descendant_end() {
    // A seek must not borrow a candidate from a later root or outside a nested subtree.
    for source in ["f(x);g(7);h(y);", "f(g(x),7);"] {
        check(
            json!({"kind":"arguments","has":{"matches":"u","stopBy":"end"}}),
            json!({"u":{"kind":"number","pattern":"$X","regex":"^7$"}}),
            source,
            4,
        );
    }
    for (stop, cost) in [
        (json!("end"), 1),
        (json!("neighbor"), 3),
        (json!({"regex":"^never$"}), 3),
    ] {
        let rules = [json!({"kind":"arguments","has":{"matches":"n","stopBy":stop}})];
        let p = policy_utils(&rules, json!({"n":{"kind":"number","regex":"^7$"}}));
        for work in 0..=4 {
            let r = check_work(&p, &rules, &["f(1,2,7);"], None, work);
            assert_eq!(r["output"][0][4], if work < cost { -2 } else { 0 });
            if work >= cost {
                assert_eq!(r["output"][0][5], work - cost);
            }
        }
    }
    check(
        json!({"kind":"arguments","has":{"matches":"u","stopBy":"end"}}),
        json!({"u":{"kind":"identifier","all":[{"regex":"^a"},{"regex":"z$"}]}}),
        "f(alpha,zzz,abz);",
        4,
    );
}

#[test]
fn pruning_keeps_stops_and_neighbor_semantics() {
    let utils = json!({"n":{"kind":"number","regex":"^7$"},"stop":{"kind":"array"},"comma":{"regex":"^,$"},"args":{"kind":"arguments"}});
    for rule in [
        json!({"kind":"arguments","has":{"matches":"n","stopBy":"neighbor"}}),
        json!({"kind":"arguments","has":{"matches":"n","stopBy":{"matches":"stop"}}}),
        json!({"kind":"number","inside":{"matches":"args","stopBy":"end"}}),
        json!({"kind":"number","inside":{"matches":"args","stopBy":"neighbor"}}),
        json!({"kind":"number","precedes":{"matches":"n","stopBy":"end"}}),
        json!({"kind":"number","follows":{"matches":"n","stopBy":"end"}}),
        json!({"kind":"number","precedes":{"matches":"n","stopBy":{"matches":"comma"}}}),
        json!({"kind":"number","follows":{"matches":"n","stopBy":{"matches":"comma"}}}),
    ] {
        check(rule, utils.clone(), "f([1,7],2,7,3);", 14);
    }
}

#[test]
fn pruning_preserves_captures_counts_and_nested_searches() {
    for (rule, utils) in [
        (
            json!({"kind":"arguments","has":{"matches":"n"}}),
            json!({"n":{"kind":"number","pattern":"$X","regex":"^7$"}}),
        ),
        (
            json!({"kind":"arguments","has":{"matches":"n"}}),
            json!({"n":{"kind":"array","has":{"matches":"number"}} ,"number":{"kind":"number"}}),
        ),
        (
            json!({"kind":"arguments","has":{"matches":"n"}}),
            json!({"n":{"kind":"array","count":{"has":{"kind":"number"},"min":1,"as":"N"}}}),
        ),
        (
            json!({"kind":"arguments","count":{"has":{"matches":"number"},"min":1}}),
            json!({"number":{"kind":"number"}}),
        ),
    ] {
        check(rule, utils, "f([1,7],2,7);", 18);
    }
}

#[test]
fn pruning_profile() {
    let mut samples = vec![];
    for dimension in [
        "sparse-kinds",
        "sparse-text",
        "dense-false",
        "known-first",
        "stop",
    ] {
        for size in [4, 32, 256] {
            let (source, util, stop) = match dimension {
                "sparse-text" => (
                    format!("f({},goal);", vec!["other"; size].join(",")),
                    json!({"kind":"identifier","regex":"^goal$"}),
                    json!("end"),
                ),
                "dense-false" => (
                    format!("f({});", vec!["1"; size].join(",")),
                    json!({"kind":"number","not":{"regex":".*"}}),
                    json!("end"),
                ),
                "known-first" => (
                    format!("f(1,{});", vec!["2"; size].join(",")),
                    json!({"kind":"number"}),
                    json!("end"),
                ),
                "stop" => (
                    format!("f([{}],1);", vec!["x"; size].join(",")),
                    json!({"kind":"number"}),
                    json!({"kind":"array"}),
                ),
                _ => (
                    format!("f({},1);", vec!["x"; size].join(",")),
                    json!({"kind":"number"}),
                    json!("end"),
                ),
            };
            let search = json!({"matches":"u","stopBy":stop});
            let rule = if dimension == "dense-false" {
                json!({"kind":"arguments","not":{"has":search}})
            } else {
                json!({"kind":"arguments","has":search})
            };
            let rules = [rule];
            let p = policy_utils(&rules, json!({"u":util}));
            let mut r = check_work(&p, &rules, &[&source, &source], None, 10000);
            r["dimension"] = json!(dimension);
            r["size"] = json!(size);
            samples.push(r);
        }
    }
    if let Ok(path) = std::env::var("RERAIL_RELATIONAL_PRUNING_EVIDENCE") {
        std::fs::write(path, serde_json::to_string_pretty(&samples).unwrap()).unwrap();
    }
}
