//! Binding children use SQL sequencing, target charges, and transactional captures.
use super::publication_tests::check_work;
use crate::code_rule_dsl::{CodeRuleDslRuntime, CompiledCodeRuleDsl, test_support::CAPABILITIES};
use serde_json::{Value, json};
fn policy(rules: &[Value], utils: Value) -> CompiledCodeRuleDsl {
    let clauses: Vec<_> = rules.iter().enumerate().map(|(i, rule)| json!({"id":format!("c{i}"),"forbid":{"ast":{"language":"python","rule":rule,"utils":utils}},"violation":{"code":format!("C{i}"),"message":"match","anchor":"witness.span"}})).collect();
    CodeRuleDslRuntime::compile(json!({"format":"rerail.policy/v1","id":"binding-outcomes","revision":1,"mode":"steer","clauses":clauses}).to_string(),"json".into(),CAPABILITIES.into()).unwrap()
}
#[test]
fn binding_targets_order_capture_and_exact_work() {
    let rules = [
        json!({"kind":"identifier","resolves":{"binding":{"value":{"pattern":"$V"}}}}),
        json!({"kind":"identifier","inside":{"kind":"function_definition","pattern":"$REGION"},"resolves":{"binding":{"bindings":{"within":"REGION","min":1},"value":{"pattern":"$V"}}}}),
        json!({"kind":"identifier","resolves":{"binding":{"site":{"pattern":"$S"},"value":{"pattern":"$V"},"annotation":{"kind":"type"}}}}),
        json!({"kind":"identifier","any":[{"resolves":{"binding":{"site":{"pattern":"$S"},"value":{"kind":"string"}}}},{"pattern":"$F"}]}),
    ];
    let p = policy(&rules, json!({}));
    let sources = [
        "a: int = 1\nb = 'two'\nprint(a, b)\n",
        "def f(x: int):\n    y = 3\n    return x + y\n",
    ];
    for work in [0, 1, 2, 3, 4, 8, 16, 32, 64, 128, 1024, i64::MAX] {
        check_work(&p, &rules, &sources, None, work);
    }
}
#[test]
fn binding_utility_cache_fill_replay_and_unreached_children() {
    let binding = json!({"resolves":{"binding":{"value":{"pattern":"$V"}}}});
    let rules = [
        json!({"kind":"identifier","all":[{"matches":"u"},{"matches":"u"}]}),
        json!({"kind":"identifier","any":[{"regex":".*"},binding]}),
    ];
    let p = policy(&rules, json!({"u":binding}));
    for work in 0..=40 {
        check_work(&p, &rules, &["x = 1\nprint(x)\n"], None, work);
    }
}
#[test]
fn binding_nested_acyclic_following_preserves_finite_rule_identities() {
    let rules = [
        json!({"kind":"identifier","resolves":{"binding":{"value":{"resolves":{"binding":{"value":{"kind":"integer","pattern":"$V"}}}}}}}),
    ];
    let p = policy(&rules, json!({}));
    for work in [0, 1, 2, 4, 8, 16, 32, 64, 128, 1024, i64::MAX] {
        check_work(
            &p,
            &rules,
            &["a = 1\nb = a\nprint(b)\n", "x = x\nprint(x)\n"],
            None,
            work,
        );
    }
}
