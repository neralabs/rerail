use super::relational::{SqlCatalogPlan, SqlShape, TerminalKind};
use crate::code_rule_dsl::CodeRuleDslRuntime;
use crate::code_rule_dsl::test_support::CAPABILITIES;
use serde_json::json;
use std::sync::Arc;

fn code_rule(rule: serde_json::Value) -> crate::code_rule_dsl::CompiledCodeRuleDsl {
    CodeRuleDslRuntime::compile(
        json!({
            "format": "rerail.policy/v1",
            "id": "lowering",
            "revision": 1,
            "mode": "block",
            "clauses": [{
                "id": "ast-match",
                "forbid": { "ast": { "language": "python", "rule": rule } },
                "violation": { "anchor": "witness.span", "code": "X", "message": "x" }
            }]
        })
        .to_string(),
        "json".to_owned(),
        CAPABILITIES.to_owned(),
    )
    .expect("valid test policy")
}

#[test]
fn compiled_kind_policy_lowers_to_sql_and_one_terminal() {
    let compiled = code_rule(json!({ "kind": "identifier" }));
    let plan = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
        .expect("kind policy has complete relational lowering");

    assert_eq!(plan.terminals().len(), 1);
    assert_eq!(plan.terminals()[0].kind(), TerminalKind::Kind);
    assert_eq!(plan.terminals()[0].language(), "python");
    assert_eq!(
        plan.shape(),
        &[
            SqlShape::PackedMaterialized,
            SqlShape::StatusJoin,
            SqlShape::ScanTerminal,
            SqlShape::Projection,
            SqlShape::OrderBy,
        ]
    );
    assert_eq!(plan.clause_count(), 1);
    assert!(!plan.sql().contains("tq_"));
    assert!(!plan.sql().contains("rerail_nodes"));
    assert!(!plan.sql().contains("rerail_facts"));
}

#[test]
fn compiled_literal_pattern_lowers_to_a_verified_terminal() {
    let compiled = code_rule(json!({ "pattern": "foo" }));
    let plan = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
        .expect("literal pattern has complete relational lowering");

    assert_eq!(plan.terminals().len(), 1);
    assert_eq!(plan.terminals()[0].kind(), TerminalKind::Pattern);
    assert_eq!(plan.terminals()[0].language(), "python");
    assert_eq!(
        plan.shape(),
        &[
            SqlShape::PackedMaterialized,
            SqlShape::StatusJoin,
            SqlShape::ScanTerminal,
            SqlShape::ScanCapture,
            SqlShape::Projection,
            SqlShape::OrderBy,
        ]
    );
}

#[test]
fn compiled_regex_lowers_to_a_verified_terminal() {
    let compiled = code_rule(json!({ "regex": "^unsafe$" }));
    let plan = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
        .expect("node regex has complete relational lowering");

    assert_eq!(plan.terminals().len(), 1);
    assert_eq!(plan.terminals()[0].kind(), TerminalKind::Regex);
    assert_eq!(plan.terminals()[0].language(), "python");
    assert_eq!(
        plan.shape(),
        &[
            SqlShape::PackedMaterialized,
            SqlShape::StatusJoin,
            SqlShape::ScanTerminal,
            SqlShape::Projection,
            SqlShape::OrderBy,
        ]
    );
}

#[test]
fn equal_terminals_share_identity_without_merging_clauses() {
    let first = code_rule(json!({ "kind": "identifier" }));
    let second = code_rule(json!({ "kind": "identifier" }));
    let plan = SqlCatalogPlan::compile(&[Arc::clone(&first.inner), Arc::clone(&second.inner)])
        .expect("equal kind terminals lower");

    assert_eq!(plan.terminals().len(), 1);
    assert_eq!(plan.clause_count(), 2);
    assert_eq!(plan.clause_terminal_ids(), &[vec![0], vec![0]]);
}

#[test]
fn unsupported_inventory_reports_every_operation_in_source_order() {
    let first = code_rule(json!({
        "all": [
            { "kind": "identifier" },
            { "regex": "^x" }
        ]
    }));
    let second = code_rule(json!({
        "has": { "kind": "call", "stopBy": "end" }
    }));
    let error = SqlCatalogPlan::compile(&[Arc::clone(&first.inner), Arc::clone(&second.inner)])
        .expect_err("phase-one compiler reports unsupported relational operators");

    assert_eq!(
        error.unsupported(),
        &[
            "lowering/ast-match/python/rule/all: ordered conjunction",
            "lowering/ast-match/python/rule/has: descendant relation",
        ]
    );
}
