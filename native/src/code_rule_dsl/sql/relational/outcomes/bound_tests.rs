//! Atomic argument binding keeps native truth, unification, and work semantics.
use super::publication_tests::check_work;
use crate::code_rule_dsl::{
    CodeRuleDslRuntime, CompiledCodeRuleDsl, sql::relational::bound_leaf::trace,
    test_support::CAPABILITIES,
};
use serde_json::{Value, json};

fn policy(rules: &[Value], utils: Value) -> CompiledCodeRuleDsl {
    policy_language("python", rules, utils)
}

fn policy_language(language: &str, rules: &[Value], utils: Value) -> CompiledCodeRuleDsl {
    let clauses: Vec<_> = rules.iter().enumerate().map(|(i, rule)| json!({
        "id":format!("c{i}"),"forbid":{"ast":{"language":language,"rule":rule,"utils":utils}},
        "violation":{"code":format!("C{i}"),"message":"match","anchor":"witness.span"}
    })).collect();
    CodeRuleDslRuntime::compile(
        json!({"format":"rerail.policy/v1","id":"bound-outcomes","revision":1,"mode":"steer","clauses":clauses}).to_string(),
        "json".into(), CAPABILITIES.into(),
    ).unwrap()
}

fn arguments() -> Value {
    json!({"arguments":{"signature":["x","*","y"],"bind":{"X":"x","Y":"y"}}})
}

#[test]
fn arguments_bound_leaf_keywords_spreads_missing_and_native_budget() {
    for argument in [
        arguments(),
        json!({"arguments":{"signature":["x","*","y"],"bind":{"Y":"y"},"absent":["x"]}}),
        json!({"arguments":{"signature":["x","y"],"bind":{"X":"x","Y":"y"},"mode":"syntactic"}}),
        json!({"arguments":{"signature":["x","/","y"],"bind":{"X":"x","Y":"y"}}}),
    ] {
        let rules = [json!({"kind":"call","all":[argument]})];
        let p = policy(&rules, json!({}));
        let sources = [
            "f(a,y=b)\nf(x=a,y=b)\nf(a,b)\nf()\nf(*items)\nf(*items,y=b)\nf(**kw)\nf(**{'x': a, 'y': b})\nf(x=a,**kw)\n",
            "g(a,y=b)\ng(y=b)\ng(**{'y': b})\ng(x for x in items)\n",
        ];
        for work in [0, 1, 2, 3, 4, 8, 16, 64, 256] {
            check_work(&p, &rules, &sources, None, work);
        }
        check_work(&p, &rules, &sources, None, i64::MAX);
    }
}

#[test]
fn arguments_bound_leaf_unknown_before_false_binds_no_captures() {
    // x is unknown after *items, but absent keyword-only y makes binding false.
    let rules = [json!({"kind":"call","all":[arguments()]})];
    let p = policy(&rules, json!({}));
    let result = check_work(&p, &rules, &["f(*items)\n"], None, i64::MAX);
    assert_eq!(result["output"], json!([]));
    // The same ordering applies after an absent assertion accumulated unknown.
    let rules = [
        json!({"kind":"call","arguments":{"signature":["x","*","y"],"absent":["x"],"bind":{"Y":"y"}}}),
    ];
    let p = policy(&rules, json!({}));
    assert_eq!(
        check_work(&p, &rules, &["f(*items)\n"], None, i64::MAX)["output"],
        json!([])
    );
}

#[test]
fn arguments_bound_leaf_unifies_typed_inputs_and_isolates_clauses_files() {
    let rules: Vec<_> = ["x", "y"].iter().map(|slot| json!({"kind":"call","all":[
        {"has":{"field":"arguments","has":{"kind":"identifier","pattern":"$X","stopBy":"neighbor"}}},
        {"arguments":{"signature":["x","y"],"bind":{"X":slot}}}
    ]})).collect();
    let p = policy(&rules, json!({}));
    for work in [0, 1, 2, 4, 8, 32, 128] {
        check_work(
            &p,
            &rules,
            &["f(a,b)\nf(a,a)\nf(**{'x': a,'y': b})\n", "g(c,d)\ng(c,c)\n"],
            None,
            work,
        );
    }
}

#[test]
fn arguments_bound_leaf_utility_replay_and_unreached_demand() {
    let rules =
        [json!({"kind":"call","all":[{"matches":"args"},{"matches":"args"},{"matches":"args"}]})];
    let p = policy(&rules, json!({"args":arguments()}));
    for work in 0..=20 {
        check_work(
            &p,
            &rules,
            &["f(a,y=b)\nf(**{'x': a,'y': b})\n", "g(c,y=d)\n"],
            None,
            work,
        );
    }
    // First call binds X/Y. The second stores the answer for those bound inputs;
    // the third must actually recall that answer, rather than merely recompute.
    let result = check_work(&p, &rules, &["f(a,y=b)\n"], None, i64::MAX);
    assert!(
        result["metrics"]["execution"]["memoStorage"]["hits"]
            .as_u64()
            .unwrap()
            > 0
    );
    let rules = [json!({"kind":"call","any":[{"regex":".*"},arguments()]})];
    let p = policy(&rules, json!({}));
    let (result, calls) = trace(|| check_work(&p, &rules, &["f(a,y=b)\n"], None, 0));
    assert!(calls.is_empty());
    assert!(
        result["output"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row[4] == 0)
    );
}

#[test]
fn arguments_bound_leaf_typescript_modes_and_call_shapes() {
    let rules = [
        json!({"arguments":{"signature":["x","y"],"bind":{"X":"x","Y":"y"}}}),
        json!({"arguments":{"signature":["x","y"],"bind":{"X":"x","Y":"y"},"mode":"estree"}}),
    ];
    let p = policy_language("typescript", &rules, json!({}));
    let sources = [
        "f(a,b); f(...items,b); f(a); new C(a,b); tag`text`; f((a),(b));",
        "g(c,d); new D(c,d); g(...items); g();",
    ];
    for work in [0, 1, 2, 4, 8, 32] {
        check_work(&p, &rules, &sources, None, work);
    }
    let result = check_work(&p, &rules, &sources, None, i64::MAX);
    assert!(!result["output"].as_array().unwrap().is_empty());
    let (_, calls) = trace(|| {
        check_work(
            &p,
            &rules,
            &["f(a,b); new C(c,d);", "g(e,f);"],
            None,
            i64::MAX,
        )
    });
    assert_eq!(calls.len(), 6, "three reached calls in each of two clauses");
    let mut keys = std::collections::BTreeSet::new();
    assert!(
        calls
            .iter()
            .all(|&(term, file, node, _, _)| keys.insert((term, file, node)))
    );
}

#[test]
fn arguments_bound_leaf_php_named_unpacking_and_finite_work() {
    let rules = [
        json!({"arguments":{"signature":["x","y"],"bind":{"X":"x","Y":"y"}}}),
        json!({"arguments":{"signature":["x","y"],"bind":{"X":"x","Y":"y"},"mode":"syntactic"}}),
    ];
    let p = policy_language("php", &rules, json!({}));
    let sources = [
        "<?php f(1,2); f(y:2,x:1); f(...['x'=>1,'y'=>2]); f(...$items); f(...); f(1);",
        "<?php g(...[3,4]); g(...[3,'y'=>4]); g(x:3,...$items); g();",
    ];
    for work in [0, 1, 2, 3, 4, 8, 16, 32, 64, 128] {
        check_work(&p, &rules, &sources, None, work);
    }
    let result = check_work(&p, &rules, &sources, None, i64::MAX);
    assert!(!result["output"].as_array().unwrap().is_empty());
    let (_, calls) = trace(|| {
        check_work(
            &p,
            &rules,
            &["<?php f(1,2); f(y:4,x:3);", "<?php g(5,6);"],
            None,
            i64::MAX,
        )
    });
    assert_eq!(calls.len(), 6, "three reached calls in each of two clauses");
    let mut keys = std::collections::BTreeSet::new();
    assert!(
        calls
            .iter()
            .all(|&(term, file, node, _, _)| keys.insert((term, file, node)))
    );
    // Array unpacking reads literal keys/values through the native finite-work decoder.
    let rules = [rules[0].clone()];
    let p = policy_language("php", &rules, json!({}));
    let source = "<?php f(...['x'=>1,'y'=>2]);";
    let exhausted = check_work(&p, &rules, &[source], None, 0);
    assert!(
        exhausted["output"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row[4].as_i64().unwrap() < 0)
    );
    let complete = check_work(&p, &rules, &[source], None, i64::MAX);
    assert!(
        complete["output"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row[4] == 0)
    );
}

#[test]
fn arguments_bound_leaf_one_native_call_per_reached_demand() {
    let rules = [json!({"kind":"call","all":[arguments()]})];
    let p = policy(&rules, json!({}));
    let (_, calls) = trace(|| {
        check_work(
            &p,
            &rules,
            &["f(a,y=b)\nf(c,y=d)\n", "g(e,y=f)\n"],
            None,
            i64::MAX,
        )
    });
    assert_eq!(calls.len(), 3);
    let mut keys = std::collections::BTreeSet::new();
    assert!(
        calls
            .iter()
            .all(|&(term, file, node, _, _)| keys.insert((term, file, node)))
    );
}

#[test]
fn arguments_bound_leaf_arity_width_and_irrelevant_growth_keep_one_demand() {
    for width in [1, 8, 32] {
        let signature: Vec<_> = (0..width).map(|i| format!("a{i}")).collect();
        let bindings: serde_json::Map<String, Value> = signature
            .iter()
            .enumerate()
            .map(|(i, name)| (format!("X{i}"), json!(name)))
            .collect();
        let rules = [json!({"kind":"call","arguments":{"signature":signature,"bind":bindings}})];
        let p = policy(&rules, json!({}));
        let source = format!(
            "f({})\n",
            (0..width)
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join(",")
        );
        for growth in [0, 128] {
            let source = format!("{source}{}", "1 + 2\n".repeat(growth));
            let (_, calls) = trace(|| check_work(&p, &rules, &[&source], None, i64::MAX));
            assert_eq!(calls.len(), 1, "width={width} growth={growth}");
        }
    }
}
