use super::publication_tests::{check_work_compiler, policy_utils};
use serde_json::json;
#[test]
fn metadata_utility_segments_preserve_work_unknown_callbacks_and_failure_followers() {
    let rules = [
        json!({"kind":"arguments","has":{"stopBy":"end","any":[{"matches":"outer"},{"not":{"matches":"outer"}}]}}),
    ];
    let policy = policy_utils(
        &rules,
        json!({
            "outer":{"any":[{"matches":"left"},{"matches":"right"}]},
            "left":{"any":[{"kind":"number","value":{"gt":0}},{"kind":"identifier","value":{"gt":0}},{"kind":"parenthesized_expression","has":{"stopBy":"neighbor","matches":"left"}}]},
            "right":{"any":[{"kind":"number","value":{"eq":0}},{"kind":"parenthesized_expression","has":{"stopBy":"neighbor","matches":"right"}}]}
        }),
    );
    let sources = ["f(a,0,1);f(((0)));f(((1)),a);g();", "f(z,0);f(-1,1);"];
    for work in [0, 1, 2, 3, 4, 5, 8, 16, 32, 128, 1024, i64::MAX] {
        for fault in [None, Some("1")] {
            check_work_compiler(&policy, &rules, &sources, fault, work, |g, w| {
                let c = super::compile_job_finite(g, w)?;
                assert!(
                    c.relations.to_string().contains("metadata utility segment"),
                    "metadata proof must be exercised"
                );
                Ok(c)
            });
        }
    }
}

#[test]
fn metadata_candidate_selection_preserves_unknown_work_faults_and_search_boundaries() {
    let rules = [
        json!({"kind":"arguments","has":{"stopBy":"end","all":[{"kind":"number"},{"matches":"v"}]}}),
        json!({"kind":"arguments","has":{"stopBy":"neighbor","all":[{"kind":"number"},{"matches":"v"}]}}),
        json!({"kind":"arguments","has":{"stopBy":{"kind":"parenthesized_expression"},"all":[{"kind":"number"},{"matches":"v"}]}}),
    ];
    let policy = policy_utils(&rules, json!({"v":{"testUnknown":"^2$","value":{"gt":0}}}));
    let sources = [
        "f(a,b,(0),((2)),c,3);f(a,b);f(0,1);",
        "f(((0)));g(2,0);g(3);",
    ];
    for work in [0, 1, 2, 3, 4, 8, 32, i64::MAX] {
        for fault in [None, Some("3")] {
            check_work_compiler(&policy, &rules, &sources, fault, work, |g, w| {
                let compiled = super::compile_job_finite(g, w)?;
                fn skipped(value: &serde_json::Value) -> u64 {
                    match value {
                        serde_json::Value::Object(map) => {
                            map.get("zeroWorkCandidateSearches")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0)
                                + map.values().map(skipped).sum::<u64>()
                        }
                        serde_json::Value::Array(values) => values.iter().map(skipped).sum(),
                        _ => 0,
                    }
                }
                assert!(
                    skipped(&compiled.relations) >= 2,
                    "both neighbor and descendant proofs must be exercised"
                );
                Ok(compiled)
            });
        }
    }
}
