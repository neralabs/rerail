//! Native-oracle gates for shared physical utility bodies.
use super::publication_tests::{check_work_compiler, policy_utils};
use serde_json::{Value, json};
fn check(rule: Value, utils: Value, sources: &[&str]) {
    let rules = [rule];
    let p = policy_utils(&rules, utils);
    for work in [0, 1, 2, 3, 4, 5, 8, 16, 32, 100, 1000, i64::MAX] {
        check_work_compiler(&p, &rules, sources, None, work, super::compile_job_finite);
    }
    check_work_compiler(
        &p,
        &rules,
        sources,
        Some("2"),
        1000,
        super::compile_job_finite,
    );
}
#[test]
fn shared_calls_retain_work_memo_depth_captures_and_order() {
    check(
        json!({"kind":"arguments","has":{"stopBy":"neighbor","all":[{"matches":"u"},{"matches":"u"}]}}),
        json!({"u":{"pattern":"$X","value":{"gt":0}}}),
        &["f(1,2); f(0,1); f(x,2);"],
    );
}
#[test]
fn shared_booleans_keep_unknown_withholding_and_rollback() {
    check(
        json!({"kind":"arguments","has":{"stopBy":"neighbor","all":[{"any":[{"matches":"a"},{"matches":"b"}]},{"not":{"not":{"matches":"b"}}}]}}),
        json!({"a":{"pattern":"$X","testUnknown":"^x$","value":{"gt":0}},"b":{"pattern":"$Y","value":{"gt":0}}}),
        &["f(x,2);f(0,1);f(1);"],
    );
    check(
        json!({"kind":"arguments","has":{"stopBy":"neighbor","all":[{"matches":"a"},{"matches":"a"},{"regex":"^never$"}]}}),
        json!({"a":{"pattern":"$X","testUnknown":"^x$","value":{"gt":0}}}),
        &["f(x,2);f(1);"],
    );
}
#[test]
fn shared_search_stop_count_and_field_follow_native_visits() {
    for axis in ["has", "inside", "follows", "precedes"] {
        check(
            json!({"kind":"identifier",axis:{"stopBy":{"regex":"^barrier$"},"matches":"u"}}),
            json!({"u":{"any":[{"kind":"number","value":{"gt":0}},{"testUnknown":"^x$","regex":"^never$"}]}}),
            &["f(x,0,1);g(y);barrier;z;"],
        );
    }
    check(
        json!({"kind":"arguments","count":{"has":{"stopBy":"neighbor","matches":"u"},"min":2,"as":"N"}}),
        json!({"u":{"kind":"number","value":{"gt":0}}}),
        &["f(1,2,3);f(1);f(x,2);"],
    );
}

#[test]
fn shared_control_tables_have_statement_lifetime_and_linear_root_cost() {
    let rules = [json!({"kind":"number","all":vec![json!({"matches":"u"});8]})];
    let p = policy_utils(&rules, json!({"u":{"value":{"gt":0}}}));
    let mut measurements = vec![];
    for width in [8, 32, 128] {
        let source = "f(1,0,x);".repeat(width);
        let result = check_work_compiler(
            &p,
            &rules,
            &[&source],
            None,
            i64::MAX,
            super::compile_job_finite,
        );
        let sql = result["sql"].as_str().unwrap();
        for id in 0..3 {
            assert_eq!(
                sql.matches(&format!("constant0_{id} AS MATERIALIZED"))
                    .count(),
                1
            );
        }
        measurements.push(json!({"width":width,"sqlBytes":sql.len(),"metrics":result["metrics"]}));
    }
    for pair in measurements.windows(2) {
        assert_eq!(pair[0]["sqlBytes"], pair[1]["sqlBytes"]);
        assert!(
            pair[1]["metrics"]["execution"]["vmSteps"].as_u64().unwrap()
                <= 5 * pair[0]["metrics"]["execution"]["vmSteps"].as_u64().unwrap()
        );
    }
    if let Ok(path) = std::env::var("RERAIL_FINITE_TABLES_EVIDENCE") {
        std::fs::write(path, serde_json::to_vec_pretty(&measurements).unwrap()).unwrap();
    }
}
