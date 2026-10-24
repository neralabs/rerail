//! Shared definitions preserve evaluations, rollback and native call identities.
use super::{
    finite_lower,
    lower::Lower,
    publication_tests::{check_work_compiler, policy_utils},
};
use crate::code_rule_dsl::test_support::grammar_matchers;
use serde_json::json;
#[test]
fn shared_source_keeps_distinct_utility_entries_with_one_capture_body() {
    let rules = [
        json!({"kind":"arguments","has":{"stopBy":"neighbor","all":[{"matches":"u"},{"matches":"v"}]}}),
    ];
    let p = policy_utils(&rules, json!({"u":{"pattern":"$X"},"v":{"pattern":"$X"}}));
    let grammar = &grammar_matchers(&p)[0];
    let mut lower = Lower::new_finite(grammar.language, grammar.three_valued);
    let plan = lower.plan(&grammar.matcher).unwrap();
    let program = finite_lower::lower(&plan).unwrap();
    assert_eq!(program.functions.len(), 2);
    assert_ne!(program.functions[0].memo, program.functions[1].memo);
    assert_eq!(program.functions[0].entry, program.functions[1].entry);
    for work in [0, 1, 2, 3, 4, 8, 16, 64, i64::MAX] {
        check_work_compiler(
            &p,
            &rules,
            &["f(1,2);f(x);"],
            None,
            work,
            super::compile_job_finite,
        );
    }
}
#[test]
fn shared_capture_subtrees_run_again_after_failed_and_unknown_branches() {
    let shared = json!({"all":[{"pattern":"$X"},{"regex":"^[12]$"}]});
    let rules = [
        json!({"kind":"arguments","has":{"stopBy":"neighbor","any":[{"all":[shared.clone(),{"value":{"gt":10}}]},{"all":[shared,{"pattern":"$Y"}]},{"all":[{"testUnknown":"^x$"},{"pattern":"$Z"}]}]}}),
    ];
    let p = policy_utils(&rules, json!({}));
    for work in [0, 1, 2, 3, 4, 8, 16, 64, 1024, i64::MAX] {
        check_work_compiler(
            &p,
            &rules,
            &["f(1,2);f(0,2);f(x,2);"],
            None,
            work,
            super::compile_job_finite,
        );
    }
    check_work_compiler(
        &p,
        &rules,
        &["f(1,2);"],
        Some("2"),
        1024,
        super::compile_job_finite,
    );
}
#[test]
fn shared_contextual_patterns_preserve_selected_kind_and_capture_spans() {
    let pattern = json!({"pattern":{"context":"f($X + $Y)","selector":"binary_expression"}});
    let rules =
        [json!({"kind":"arguments","has":{"stopBy":"neighbor","all":[pattern.clone(),pattern]}})];
    let p = policy_utils(&rules, json!({}));
    for work in [0, 1, 2, 4, 8, 64, i64::MAX] {
        check_work_compiler(
            &p,
            &rules,
            &["f(1+2,3*4);g(x+y);g(x);"],
            None,
            work,
            super::compile_job_finite,
        );
    }
}

#[test]
fn one_shared_call_instruction_still_spends_eight_native_entries() {
    let rules = [json!({"kind":"number","all":vec![json!({"matches":"u"});8]})];
    let p = policy_utils(&rules, json!({"u":{"pattern":"$X"}}));
    for work in 0..=9 {
        let result =
            check_work_compiler(&p, &rules, &["1;"], None, work, super::compile_job_finite);
        assert_eq!(result["output"][0][4], if work < 8 { -2 } else { 0 });
        assert_eq!(result["output"][0][5], if work < 8 { 0 } else { work - 8 });
    }
}

#[test]
fn shared_walks_with_same_utility_identity_reexecute_each_parent_edge() {
    let search = json!({"has":{"stopBy":"neighbor","matches":"u"}});
    let rules = [json!({"kind":"arguments","all":[search.clone(),search]})];
    let p = policy_utils(&rules, json!({"u":{"pattern":"$X","value":{"gt":0}}}));
    for work in [0, 1, 2, 3, 4, 5, 8, 16, 32, 1000, i64::MAX] {
        check_work_compiler(
            &p,
            &rules,
            &["f(1,2);f(0,2);f(x,2);"],
            None,
            work,
            super::compile_job_finite,
        );
    }
    check_work_compiler(
        &p,
        &rules,
        &["f(1,2);"],
        Some("1"),
        1000,
        super::compile_job_finite,
    );
}
