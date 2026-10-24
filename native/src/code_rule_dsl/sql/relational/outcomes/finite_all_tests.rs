//! Conjunction fusion must preserve whole-child capture withholding.
use super::{
    lower::Lower,
    publication_tests::{check_work_compiler, policy_utils},
};
use crate::code_rule_dsl::test_support::grammar_matchers;
use serde_json::json;
#[test]
fn total_nested_all_cannot_cross_propagated_held_capture_names() {
    let rules = [json!({"kind":"identifier","regex":"^x$","all":[
        {"all":[{"pattern":"$X"},{"testUnknown":".*"}]},
        {"all":[{"pattern":"$Y"},{"compare":{"left":"X","op":"eq","right":"Y","text":"node"}}]},
        {"all":[{"matches":"u"},{"pattern":"$Y"}]}
    ]})];
    let p = policy_utils(&rules, json!({"u":{"kind":"identifier"}}));
    for work in [0, 1, 2, 3, 8, 64] {
        let result =
            check_work_compiler(&p, &rules, &["x;"], None, work, super::compile_job_finite);
        assert_eq!(result["output"], json!([]));
    }
}
#[test]
fn total_nested_all_fuses_real_frames_and_keeps_capture_fault_order() {
    let rules = [json!({"kind":"number","all":[
        {"all":[{"regex":"^[0-9]+$"},{"pattern":"$X"}]},
        {"compare":{"left":"X","op":"eq","right":"X","text":"node"}},
        {"matches":"u"}
    ]})];
    let p = policy_utils(&rules, json!({"u":{"value":{"gt":0}}}));
    let grammar = &grammar_matchers(&p)[0];
    let mut lower = Lower::new_finite(grammar.language, grammar.three_valued);
    lower.plan(&grammar.matcher).unwrap();
    assert!(lower.profile(0)["finiteFlattenedAll"].as_u64().unwrap() > 0);
    for work in [0, 1, 2, 3, 4, 8, 32, 64, i64::MAX] {
        check_work_compiler(
            &p,
            &rules,
            &["f(1,0,2);"],
            None,
            work,
            super::compile_job_finite,
        );
    }
    check_work_compiler(
        &p,
        &rules,
        &["f(1,0,2);"],
        Some("2"),
        64,
        super::compile_job_finite,
    );
}
