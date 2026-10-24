//! Ordered recursive functions reuse bodies without changing native entry events.
use super::{
    finite_lower,
    lower::Lower,
    publication_tests::{check_work_compiler, policy_utils},
};
use crate::code_rule_dsl::test_support::grammar_matchers;
use serde_json::{Value, json};
fn check(rule: Value, utils: Value, sources: &[&str]) {
    let rules = [rule];
    let p = policy_utils(&rules, utils);
    let grammar = &grammar_matchers(&p)[0];
    let mut lower = Lower::new_finite(grammar.language, grammar.three_valued);
    let plan = lower.plan(&grammar.matcher).unwrap();
    let program = finite_lower::lower(&plan).unwrap();
    assert!(
        program.functions.iter().any(|f| f
            .memo
            .is_some_and(|id| (2_000_000..3_000_000).contains(&id))),
        "proven recursive function must retain its native memo identity"
    );
    assert!(super::compile_job_finite(&[grammar], -1).is_err());
    for work in [0, 1, 2, 3, 4, 5, 8, 16, 32, 64, 1024, i64::MAX] {
        check_work_compiler(&p, &rules, sources, None, work, super::compile_job_finite);
    }
    check_work_compiler(
        &p,
        &rules,
        sources,
        Some("1"),
        1024,
        super::compile_job_finite,
    );
}
#[test]
fn finite_recursive_ordered_arms_keep_unknown_capture_provenance() {
    let recurse = json!({"kind":"array","has":{"stopBy":"neighbor","matches":"u"}});
    let base = json!({"kind":"number","matches":"leaf"});
    for first in [false, true] {
        let arms = if first {
            vec![
                recurse.clone(),
                json!({"kind":"array","testUnknown":".*"}),
                base.clone(),
            ]
        } else {
            vec![
                json!({"kind":"array","testUnknown":".*"}),
                base.clone(),
                recurse.clone(),
            ]
        };
        check(
            json!({"kind":"array","matches":"u"}),
            json!({"u":{"any":arms},"leaf":{"pattern":"$X","value":{"gt":0}}}),
            &["f([1]);f([[0,1]]);f([]);f([x,1]);"],
        );
    }
    check(
        json!({"kind":"array","matches":"u"}),
        json!({"u":{"any":[{"kind":"identifier","matches":"leaf"},recurse,{"kind":"number","matches":"leaf"}]},"leaf":{"pattern":"$X","testUnknown":"^x$","value":{"gt":0}}}),
        &["f([x,1]);f([0,1]);f([[1]]);"],
    );
}
#[test]
fn finite_recursive_child_prefix_rejects_before_work_zero_entry() {
    let rules = [json!({"kind":"array","matches":"u"})];
    let p = policy_utils(
        &rules,
        json!({"u":{"any":[{"kind":"number","matches":"leaf"},{"kind":"array","has":{"stopBy":"neighbor","not":{"regex":"^(?:\\[|\\]|,|0)$"},"matches":"u"}}]},"leaf":{"pattern":"$X","value":{"gt":0}}}),
    );
    for work in [0, 1, 2, 3, 4, 8, 32, 1000] {
        let result = check_work_compiler(
            &p,
            &rules,
            &["f([0]);"],
            None,
            work,
            super::compile_job_finite,
        );
        if work == 1 {
            assert_eq!(result["output"], json!([]));
            assert_eq!(result["nativeTraces"][0]["remaining"], 0);
        }
    }
}
