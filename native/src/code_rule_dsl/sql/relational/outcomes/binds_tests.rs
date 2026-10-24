//! Scope and usage predicates retain native work and clause-owned cache fills.
use super::publication_tests::check_work;
use crate::code_rule_dsl::{
    CodeRuleDslRuntime, CompiledCodeRuleDsl,
    test_support::{CAPABILITIES, grammar_matchers},
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
        json!({"format":"rerail.policy/v1","id":"binds-outcomes","revision":1,"mode":"steer","clauses":clauses}).to_string(),
        "json".into(), CAPABILITIES.into(),
    ).unwrap()
}

fn unused() -> Value {
    json!({"binds":{"form":"annotation","scope":["function","lambda"],"latest":true,"used":false}})
}

#[test]
fn binds_scopes_usage_captures_failures_and_clause_ownership() {
    let rules = [
        json!({"kind":"identifier","pattern":"$N","all":[unused()]}),
        json!({"kind":"identifier","all":[unused()]}),
    ];
    let p = policy(&rules, json!({}));
    let sources = [
        "top: int\nclass C:\n    member: int\ndef f():\n    unused: int\n    used: int\n    print(used)\n    assigned: int\n    assigned = 1\n",
        "def g():\n    unused: int\n    unused: str\n    used: int\n    return used\n",
    ];
    for work in [0, 1, 2, 4, 8, 16, 32, 64, 128, 1024] {
        check_work(&p, &rules, &sources, None, work);
    }
    let result = check_work(&p, &rules, &sources, None, i64::MAX);
    assert!(!result["output"].as_array().unwrap().is_empty());
}

#[test]
fn binds_utility_replay_excludes_scope_and_usage_fill_work() {
    let rules = [json!({"kind":"identifier","all":[{"matches":"u"},{"matches":"u"}]})];
    let p = policy(&rules, json!({"u":unused()}));
    // Sweep through both initial cache construction and repeated utility entries.
    for work in 0..=64 {
        check_work(
            &p,
            &rules,
            &[
                "def f():\n    a: int\n    b: str\n",
                "def g():\n    c: int\n",
            ],
            None,
            work,
        );
    }
    let result = check_work(
        &p,
        &rules,
        &["def f():\n    a: int\n    b: str\n"],
        None,
        i64::MAX,
    );
    assert!(
        result["metrics"]["execution"]["memoStorage"]["hits"]
            .as_u64()
            .unwrap()
            > 0
    );
}

#[test]
fn binds_unreached_predicate_spends_no_work_or_leaf_demands() {
    let rules = [json!({"kind":"identifier","any":[{"regex":".*"},unused()]})];
    let p = policy(&rules, json!({}));
    let result = check_work(&p, &rules, &["def f():\n    unused: int\n"], None, 0);
    assert_eq!(result["budgetCalls"], json!([]));
    assert!(
        result["output"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row[4] == 0)
    );
}

#[test]
fn binds_kind_selection_bounds_leaf_demands_under_irrelevant_growth() {
    let rules = [json!({"kind":"identifier","all":[unused()]})];
    let p = policy(&rules, json!({}));
    let source = "def f():\n    unused: int\n";
    let small = check_work(&p, &rules, &[source], None, i64::MAX);
    let large_source = format!("{source}{}", "1 + 2\n".repeat(256));
    let large = check_work(&p, &rules, &[&large_source], None, i64::MAX);
    assert_eq!(
        small["budgetCalls"].as_array().unwrap().len(),
        large["budgetCalls"].as_array().unwrap().len()
    );
}

#[test]
fn binds_refuses_subrules_and_redefinitions() {
    for binds in [
        json!({"site":{"kind":"assignment"}}),
        json!({"value":{"kind":"integer"}}),
        json!({"annotation":{"kind":"identifier"}}),
        json!({"redefines":{"form":"assignment"}}),
    ] {
        let p = policy(
            &[json!({"kind":"identifier","pattern":"$REGION","binds":binds})],
            json!({}),
        );
        let grammars = grammar_matchers(&p);
        let reason = super::compile_job(&[&grammars[0]], i64::MAX)
            .err()
            .expect("composite binds must be refused");
        assert!(reason.starts_with("outcome binds:"), "{reason}");
    }
    let p = policy_language(
        "typescript",
        &[json!({"kind":"identifier","binds":{"occurrence":{"kind":"identifier"}}})],
        json!({}),
    );
    let grammars = grammar_matchers(&p);
    let reason = super::compile_job(&[&grammars[0]], i64::MAX)
        .err()
        .expect("occurrence composition must be refused");
    assert!(reason.starts_with("outcome binds:"), "{reason}");
}

#[test]
fn f842_catalog_rule_matches_upstream_fixture_without_frames() {
    let p = CodeRuleDslRuntime::compile(
        include_str!("../../../../../../packs/python/rules/ruff-f842-unused-annotation.json")
            .into(),
        "json".into(),
        CAPABILITIES.into(),
    )
    .unwrap();
    let fixture =
        include_str!("../../../../../../packs/python/upstream/ruff/fixtures/pyflakes/F842.py");
    let result = super::super::tests::measure(
        &grammar_matchers(&p)[0],
        &[("F842.py".into(), fixture.into())],
    );
    assert_eq!(result["matchingStatements"], 1);
    assert_eq!(result["framePublications"], 0);
}
