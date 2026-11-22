//! The frontier audit's test: a compiled policy's plan permutations compile to one statement.
use crate::code_rule_dsl::test_support::compiled;
use serde_json::json;

#[test]
fn plan_permutations_compile_to_the_same_statement() {
    let rule = compiled(json!({ "rule": { "any": [
        { "kind": "call_expression", "regex": "fetch" },
        { "kind": "string", "regex": "secret" },
        { "kind": "identifier", "regex": "^eval$" }
    ] } }));
    let report = super::candidate_frontier_audit(&rule.inner);
    assert_eq!(report["format"], "rerail.frontier-audit/v1");
    assert!(!report["languages"].as_array().unwrap().is_empty(), "{report}");
    assert_eq!(report["deterministic"], true, "{report}");
}
