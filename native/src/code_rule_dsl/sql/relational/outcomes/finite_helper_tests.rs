//! Native-oracle coverage for finite structural and binding continuations.
use super::publication_tests::{check_work_compiler, policy_utils};
use crate::code_rule_dsl::{CodeRuleDslRuntime, CompiledCodeRuleDsl, test_support::CAPABILITIES};
use serde_json::{Value, json};
fn python(rules: &[Value], utils: Value) -> CompiledCodeRuleDsl {
    let clauses=rules.iter().enumerate().map(|(i,rule)|json!({"id":format!("c{i}"),"forbid":{"ast":{"language":"python","rule":rule,"utils":utils}},"violation":{"code":format!("C{i}"),"message":"match","anchor":"witness.span"}})).collect::<Vec<_>>();
    CodeRuleDslRuntime::compile(json!({"format":"rerail.policy/v1","id":"finite-binding","revision":1,"mode":"steer","clauses":clauses}).to_string(),"json".into(),CAPABILITIES.into()).unwrap()
}
#[test]
fn finite_binding_targets_charge_once_and_restore_failed_exports() {
    let rules = [
        json!({"kind":"identifier","all":[{"matches":"resolve"},{"matches":"resolve"}]}),
        json!({"kind":"identifier","any":[{"resolves":{"binding":{"site":{"pattern":"$S"},"value":{"kind":"string","matches":"v"}}}},{"pattern":"$F"}]}),
        json!({"kind":"identifier","resolves":{"binding":{"site":{"pattern":"$S"},"value":{"matches":"v"},"annotation":{"kind":"type"}}}}),
        json!({"kind":"identifier","resolves":{"binding":{"site":{"pattern":"$S","testUnknown":".*"},"value":{"matches":"v"}}}}),
    ];
    let p = python(
        &rules,
        json!({"resolve":{"resolves":{"binding":{"site":{"pattern":"$S"},"value":{"all":[{"matches":"v"},{"matches":"v"}]}}}},"v":{"pattern":"$V","value":{"constant":true}}}),
    );
    let sources = [
        "a: int = 1\nb = 'two'\nprint(a, b)\n",
        "def f(x: int):\n    y = 3\n    return x+y\n",
    ];
    for work in [0, 1, 2, 3, 4, 5, 8, 16, 32, 64, 256, 1024, i64::MAX] {
        check_work_compiler(&p, &rules, &sources, None, work, super::compile_job_finite);
    }
    check_work_compiler(
        &p,
        &rules,
        &sources,
        Some("1"),
        1024,
        super::compile_job_finite,
    );
}
#[test]
fn finite_search_visits_later_siblings_and_keeps_parent_field_roles() {
    let rules = [
        json!({"kind":"arguments","has":{"stopBy":"neighbor","matches":"u"}}),
        json!({"kind":"arguments","not":{"has":{"stopBy":"neighbor","not":{"any":[{"regex":"^[(),]$"},{"matches":"u"}]}}}}),
        json!({"kind":"identifier","regex":"^a$","inside":{"field":"object","matches":"parent"}}),
    ];
    let p = policy_utils(
        &rules,
        json!({"u":{"kind":"number","value":{"gt":0}},"parent":{"kind":"member_expression","pattern":"$P"}}),
    );
    for work in [0, 1, 2, 3, 4, 8, 16, 64, 1024, i64::MAX] {
        check_work_compiler(
            &p,
            &rules,
            &["f(0,1);f(1,0);f(1,2);a.b.c;b.a.c;a;"],
            None,
            work,
            super::compile_job_finite,
        );
    }
}

#[test]
fn shared_plain_utility_keeps_distinct_binding_following_origins() {
    let binding = json!({"resolves":{"binding":{"value":{"matches":"u"}}}});
    let rules = [json!({"kind":"identifier","regex":"^a$","all":[binding.clone(),binding]})];
    let p = python(
        &rules,
        json!({"u":{"any":[{"value":{"eq":1}},{"kind":"identifier"}]}}),
    );
    for work in [0, 1, 2, 3, 4, 8, 16, 32, 64, 1000] {
        check_work_compiler(
            &p,
            &rules,
            &["a = a\nprint(a)\n"],
            None,
            work,
            super::compile_job_finite,
        );
    }
}
