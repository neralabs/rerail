use super::relational::{SqlCatalogPlan, SqlShape, TerminalKind};
use crate::code_rule_dsl::test_support::compile_typescript_ast;
use serde_json::json;
use std::sync::Arc;

fn code_rule(rule: serde_json::Value) -> crate::code_rule_dsl::CompiledCodeRuleDsl {
    compile_typescript_ast(json!({ "rule": rule })).expect("valid test policy")
}

#[test]
fn compiled_kind_policy_lowers_to_sql_and_one_terminal() {
    let compiled = code_rule(json!({ "kind": "identifier" }));
    let plan = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
        .expect("kind policy has complete relational lowering");

    assert_eq!(plan.terminals().len(), 1);
    assert_eq!(plan.terminals()[0].id(), 0);
    assert_eq!(plan.terminals()[0].kind(), TerminalKind::Kind);
    assert_eq!(plan.terminals()[0].language(), "typescript");
    assert_eq!(plan.terminals()[0].source(), "kind(identifier)");
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
    assert!(plan.unsupported().is_empty());
    assert!(!plan.sql().contains("tq_"));
    assert!(!plan.sql().contains("rerail_nodes"));
    assert!(!plan.sql().contains("rerail_facts"));
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
        "has": { "kind": "call_expression", "stopBy": "end" }
    }));
    let error = SqlCatalogPlan::compile(&[Arc::clone(&first.inner), Arc::clone(&second.inner)])
        .expect_err("phase-one compiler reports unsupported relational operators");

    assert_eq!(
        error.unsupported(),
        &[
            "dialects/ast-match/typescript/rule/all: ordered conjunction",
            "dialects/ast-match/typescript/rule/all/1: regex terminal",
            "dialects/ast-match/typescript/rule/has: descendant relation",
        ]
    );
}
