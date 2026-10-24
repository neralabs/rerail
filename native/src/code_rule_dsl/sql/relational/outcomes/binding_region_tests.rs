//! Binding regions accept single captures; parsed spans expose only their optional node shadow.
use super::publication_tests::check_work;
use crate::code_rule_dsl::{CodeRuleDslRuntime, CompiledCodeRuleDsl, test_support::CAPABILITIES};
use serde_json::{Value, json};
fn compile_policy(rules: &[Value], utils: Value) -> napi::Result<CompiledCodeRuleDsl> {
    let clauses:Vec<_>=rules.iter().enumerate().map(|(i,rule)|json!({"id":format!("c{i}"),"forbid":{"ast":{"language":"python","rule":rule,"utils":utils}},"violation":{"code":format!("C{i}"),"message":"match","anchor":"witness.span"}})).collect();
    CodeRuleDslRuntime::compile(json!({"format":"rerail.policy/v1","id":"binding-region","revision":1,"mode":"steer","clauses":clauses}).to_string(),"json".into(),CAPABILITIES.into())
}
fn policy(rules: &[Value], utils: Value) -> CompiledCodeRuleDsl {
    compile_policy(rules, utils).unwrap()
}

fn predicate() -> Value {
    json!({"binds":{"form":"augmented","bindings":{"within":"REGION","max":1}}})
}
#[test]
fn binding_region_inputs_change_memo_keys_and_preserve_rollback_and_work() {
    let rules = [
        json!({"kind":"identifier","regex":"^n$","any":[{"inside":{"kind":"module","pattern":"$REGION","stopBy":"end"},"matches":"u"},{"inside":{"kind":"for_statement","pattern":"$REGION","stopBy":"end"},"matches":"u"}]}),
        json!({"kind":"identifier","regex":"^n$","inside":{"kind":"for_statement","pattern":"$REGION","stopBy":"end"},"all":[{"matches":"u"},{"matches":"u"}]}),
        json!({"kind":"identifier","inside":{"kind":"for_statement","pattern":"$REGION","stopBy":"end"},"any":[{"pattern":"$TEMP","binds":{"bindings":{"within":"REGION","min":99}}},{"pattern":"$F"}]}),
        json!({"kind":"identifier","regex":"^n$","inside":{"kind":"for_statement","pattern":"$REGION","stopBy":"end"},"all":[predicate(),{"any":[{"value":{"constant":true}},{"regex":".*"}]}]}),
    ];
    let p = policy(&rules, json!({"u":predicate()}));
    let sources = [
        "n = 0\nfor x in xs:\n    n += 1\nprint(n)\n",
        "n = 0\nfor x in xs:\n    n += 1\n    n += 2\n",
    ];
    for work in [0, 1, 2, 3, 4, 8, 16, 32, 64, 128, 256, 1024, i64::MAX] {
        let result = check_work(&p, &rules, &sources, None, work);
        if work == i64::MAX {
            for clause in 0..rules.len() {
                assert!(
                    result["output"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|r| r[0] == clause && r[4] == 0),
                    "missing positive clause {clause}"
                );
            }
        }
    }
    check_work(&p, &rules, &sources, Some("n"), i64::MAX);
}
#[test]
fn binding_region_count_and_list_channels_are_rejected_before_lowering() {
    for (rule, reason) in [
        (
            json!({"kind":"function_definition","all":[{"count":{"has":{"kind":"identifier","stopBy":"end"},"min":1,"as":"REGION"}},{"has":{"kind":"identifier","stopBy":"end","binds":{"form":"parameter","bindings":{"within":"REGION","max":0}}}}]}),
            "never bound where this binding filter runs",
        ),
        (
            json!({"kind":"assignment","pattern":"[$$$REGION] = $V","has":{"kind":"identifier","stopBy":"end","binds":{"form":"assignment","bindings":{"within":"REGION","max":0}}}}),
            "within reads single-node captures only",
        ),
    ] {
        let error = compile_policy(&[rule], json!({}))
            .err()
            .expect("region input must be a single capture");
        assert!(error.reason.contains(reason), "{}", error.reason);
    }
}
#[test]
fn binding_region_span_without_node_is_absent_but_shadow_node_bounds_the_count() {
    let parse = json!({"has":{"field":"right","parse":{"grammar":"testChars","has":{"kind":"char","capture":"REGION","regex":"^n$"}}}});
    let rules = [
        json!({"kind":"assignment","all":[parse,{"has":{"field":"left","kind":"identifier","binds":{"bindings":{"within":"REGION","max":0}}}}]}),
        json!({"kind":"assignment","all":[parse,{"has":{"field":"left","kind":"identifier","pattern":"$REGION","binds":{"bindings":{"within":"REGION","min":1,"max":1}}}}]}),
    ];
    let p = policy(&rules, json!({}));
    for work in [0, 1, 2, 4, 8, 16, 32, 64, 128, 1024, i64::MAX] {
        let result = check_work(&p, &rules, &["n = 'n'\nm = 'n'\n"], None, work);
        if work == i64::MAX {
            for clause in 0..rules.len() {
                assert!(
                    result["output"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|r| r[0] == clause && r[4] == 0),
                    "missing span channel clause {clause}"
                );
            }
        }
    }
}
