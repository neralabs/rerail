//! Occurrence candidates retain entry resolution, ordering, unknowns, and per-visit charges.
use super::publication_tests::check_work;
use crate::code_rule_dsl::{CodeRuleDslRuntime, CompiledCodeRuleDsl, test_support::CAPABILITIES};
use serde_json::{Value, json};
fn policy(rules: &[Value], utils: Value) -> CompiledCodeRuleDsl {
    let clauses: Vec<_> = rules.iter().enumerate().map(|(i, rule)| json!({"id":format!("c{i}"),"forbid":{"ast":{"language":"typescript","rule":rule,"utils":utils}},"violation":{"code":format!("C{i}"),"message":"match","anchor":"witness.span"}})).collect();
    CodeRuleDslRuntime::compile(json!({"format":"rerail.policy/v1","id":"binding-occurrence-outcomes","revision":1,"mode":"steer","clauses":clauses}).to_string(),"json".into(),CAPABILITIES.into()).unwrap()
}
#[test]
fn binding_occurrence_order_captures_nested_rules_and_budget_boundaries() {
    let rules = [
        json!({"kind":"identifier","resolves":{"binding":{"occurrence":{"pattern":"$N","inside":{"kind":"array_pattern"}}}}}),
        json!({"kind":"identifier","resolves":{"binding":{"site":{"pattern":"$S"},"occurrence":{"pattern":"$N","inside":{"kind":"arguments"}}}}}),
        json!({"kind":"identifier","resolves":{"binding":{"value":{"kind":"number"},"occurrence":{"inside":{"kind":"variable_declarator"}}}}}),
        json!({"kind":"identifier","any":[{"resolves":{"binding":{"occurrence":{"pattern":"$N","inside":{"kind":"string"}}}}},{"pattern":"$F"}]}),
    ];
    let p = policy(&rules, json!({}));
    let sources = [
        "let a = 1, b; foo(() => { [a,b] = xs; b; }); foo(a,b);",
        "let a = 1; let b = a; foo(a,b); foo(b);",
    ];
    for work in [0, 1, 2, 3, 4, 8, 16, 32, 64, 128, 256, 1024, i64::MAX] {
        check_work(&p, &rules, &sources, None, work);
    }
}
#[test]
fn binding_occurrence_unknown_search_is_strict_only_when_exporting() {
    let location = json!({"any":[{"inside":{"kind":"variable_declarator","testUnknown":".*"}},{"inside":{"kind":"arguments"}}]});
    let rules = [
        json!({"kind":"identifier","resolves":{"binding":{"occurrence":location}}}),
        json!({"kind":"identifier","resolves":{"binding":{"occurrence":{"pattern":"$N","all":[location]}}}}),
        json!({"kind":"identifier","all":[{"matches":"u"},{"matches":"u"}]}),
    ];
    let p = policy(
        &rules,
        json!({"u":{"resolves":{"binding":{"occurrence":location}}}}),
    );
    for work in [0, 1, 2, 4, 8, 16, 32, 64, 128, 256, 1024, i64::MAX] {
        check_work(&p, &rules, &["let a = 1; foo(a); foo(a);"], None, work);
    }
}
#[test]
fn binding_occurrence_failure_stops_later_candidates_and_roots() {
    let rules = [
        json!({"kind":"identifier","resolves":{"binding":{"occurrence":{"value":{"constant":true}}}}}),
    ];
    let p = policy(&rules, json!({}));
    check_work(
        &p,
        &rules,
        &["let a = 1; foo(a); foo(a);"],
        Some("a"),
        i64::MAX,
    );
}

#[test]
fn binding_following_memo_context_differs_from_occurrence_context() {
    let body = json!({"all":[{"inside":{"field":"value","kind":"variable_declarator"}},{"any":[{"value":{"eq":1}},{"kind":"identifier"}]}]});
    let rules = [
        json!({"kind":"identifier","resolves":{"binding":{"value":{"matches":"u"},"occurrence":{"matches":"u"}}}}),
    ];
    let p = policy(&rules, json!({"u":body}));
    for work in [0, 1, 2, 4, 8, 16, 32, 64, 128, 256, 1024, i64::MAX] {
        check_work(&p, &rules, &["var a = a; foo(a);"], None, work);
    }
    let rules = [
        json!({"kind":"identifier","resolves":{"binding":{"value":{"matches":{"u":{"P":"$X"}}},"occurrence":{"matches":{"u":{"P":"$X"}}}}}}),
    ];
    let p = policy(&rules, json!({"u":{"params":["P"],"rule":body}}));
    for work in [0, 1, 2, 4, 8, 16, 32, 64, 128, 256, 1024, i64::MAX] {
        check_work(&p, &rules, &["var a = a; foo(a);"], None, work);
    }
}
