use super::publication_tests::{check_work, policy_utils};
use serde_json::json;

#[test]
fn atomic_predicate_shared_unknown_capture_and_failure_boundaries() {
    for atom in [
        json!({"kind":"number"}),
        json!({"regex":"^[12]$"}),
        json!({"not":{"regex":"^2$"}}),
        json!({"pattern":"1"}),
    ] {
        for boolean in ["all", "any"] {
            let mut branch = json!({});
            branch[boolean] = json!([atom,{"matches":"v"}]);
            let expression = json!({"any":[branch,{"all":[{"not":{"matches":"v"}},{"pattern":"$KEY"}]},{"matches":"v"}]});
            let rules = [
                json!({"kind":"number","all":[expression.clone()]}),
                json!({"kind":"identifier","all":[expression]}),
            ];
            let p = policy_utils(&rules, json!({"v":{"value":{"gt":0}}}));
            for allowance in [0, 1, 2, 3, 4, 8] {
                for fault in [None, Some("2")] {
                    check_work(&p, &rules, &["1;2;x;", "2;x;1;"], fault, allowance);
                }
            }
        }
    }
}

#[test]
fn atomic_predicate_ordered_walks_stops_and_pattern_witnesses() {
    for atom in [
        json!({"kind":"number"}),
        json!({"regex":"^[12]$"}),
        json!({"not":{"regex":"^2$"}}),
        json!({"pattern":"1"}),
    ] {
        let body = json!({"all":[atom,{"value":{"gt":0}}]});
        for search in [
            json!({"has":{"all":[body.clone()],"stopBy":"end"}}),
            json!({"has":{"all":[body.clone()],"stopBy":{"kind":"identifier","testUnknown":".*"}}}),
            json!({"count":{"has":{"all":[body],"stopBy":"end"},"min":1,"as":"N"}}),
        ] {
            let rules = [
                json!({"kind":"arguments","all":[search]}),
                json!({"kind":"number","pattern":"$OUT"}),
            ];
            let p = policy_utils(&rules, json!({}));
            for allowance in [0, 1, 2, 3, 8, 100] {
                for fault in [None, Some("2")] {
                    check_work(
                        &p,
                        &rules,
                        &["f(1,x,2); g(2);", "f(x,1);"],
                        fault,
                        allowance,
                    );
                }
            }
        }
    }
}
