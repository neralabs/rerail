use super::relational::{SqlCatalogPlan, SqlShape, TerminalKind};
use crate::code_rule_dsl::CodeRuleDslRuntime;
use crate::code_rule_dsl::test_support::CAPABILITIES;
use rusqlite::{Connection, params};
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

fn normalized(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn cte(sql: &str, name: &str) -> String {
    let start = sql.find(name).expect("generated SQL contains the CTE");
    let rest = &sql[start..];
    let end = rest
        .find("\n),\n")
        .map_or(rest.len(), |index| index + "\n)".len());
    normalized(&rest[..end])
}

fn execute_boolean_sql(
    plan: &SqlCatalogPlan,
    batch_complete: bool,
    producers: &[(usize, bool, Option<bool>, &[i64])],
) -> Vec<(i64, i64)> {
    let connection = Connection::open_in_memory().expect("in-memory SQLite");
    connection
        .execute_batch(
            "CREATE TABLE rerail_matches(
                batch INTEGER, row_kind INTEGER, producer_id INTEGER, file_index INTEGER,
                task_index INTEGER, match_index INTEGER, capture_index INTEGER,
                capture_occurrence INTEGER, capture_sequence_start INTEGER,
                capture_sequence_count INTEGER, node INTEGER, related_node INTEGER,
                axis_kind INTEGER, axis_order INTEGER, fact_kind INTEGER, document_id INTEGER,
                mapped_document_id INTEGER, start_ordinal INTEGER, output_label TEXT,
                capture_label TEXT, text_value TEXT, start_byte INTEGER, end_byte INTEGER,
                verdict INTEGER, complete INTEGER, work_used INTEGER, error_code TEXT
            );",
        )
        .expect("typed packed relation");
    connection
        .execute(
            "INSERT INTO rerail_matches(batch, row_kind, complete) VALUES (1, 1, ?1)",
            [i64::from(batch_complete)],
        )
        .expect("batch status");
    for (producer, complete, verdict, nodes) in producers {
        connection
            .execute(
                "INSERT INTO rerail_matches(batch, row_kind, producer_id, file_index, verdict, complete)
                 VALUES (1, 2, ?1, 0, ?2, ?3)",
                params![
                    *producer as i64,
                    verdict.map(i64::from),
                    i64::from(*complete)
                ],
            )
            .expect("producer status");
        for (match_index, node) in nodes.iter().enumerate() {
            connection
                .execute(
                    "INSERT INTO rerail_matches(
                        batch, row_kind, producer_id, file_index, task_index, match_index,
                        node, start_ordinal, complete
                     ) VALUES (1, 3, ?1, 0, ?2, ?2, ?3, ?3, 1)",
                    params![*producer as i64, match_index as i64, node],
                )
                .expect("terminal match");
        }
    }
    let mut statement = connection.prepare(plan.sql()).expect("executable SQL plan");
    statement
        .query_map([1_i64], |row| Ok((row.get(2)?, row.get(10)?)))
        .expect("query relational rows")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("typed output rows")
}

#[test]
fn compiled_ordered_all_has_exact_relational_sql() {
    let compiled = code_rule(json!({
        "all": [
            { "kind": "identifier" },
            { "regex": "^x" }
        ]
    }));
    let plan = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
        .expect("ordered conjunction lowers into SQL");

    assert_eq!(plan.terminals().len(), 2);
    assert_eq!(
        plan.shape(),
        &[
            SqlShape::PackedMaterialized,
            SqlShape::StatusJoin,
            SqlShape::ScanTerminal,
            SqlShape::InnerJoin,
            SqlShape::CaseTruth,
            SqlShape::Projection,
            SqlShape::OrderBy,
        ]
    );
    assert_eq!(
        cte(plan.sql(), "clause_0_relation_2 AS"),
        "clause_0_relation_2 AS ( SELECT left_input.* FROM clause_0_term_0 left_input INNER JOIN clause_0_term_1 right_input ON right_input.file_index = left_input.file_index AND right_input.node = left_input.node )"
    );
    assert_eq!(
        execute_boolean_sql(
            &plan,
            true,
            &[
                (0, true, Some(true), &[10, 20]),
                (1, true, Some(true), &[20]),
            ],
        ),
        vec![(0, 20)]
    );
}

#[test]
fn compiled_ordered_any_preserves_branch_identity_in_exact_sql() {
    let compiled = code_rule(json!({
        "any": [
            { "kind": "identifier" },
            { "regex": "^x" }
        ]
    }));
    let plan = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
        .expect("ordered alternatives lower into SQL");

    assert_eq!(
        plan.shape(),
        &[
            SqlShape::PackedMaterialized,
            SqlShape::StatusJoin,
            SqlShape::ScanTerminal,
            SqlShape::OrderedUnion,
            SqlShape::WindowRank,
            SqlShape::CaseTruth,
            SqlShape::Projection,
            SqlShape::OrderBy,
        ]
    );
    assert_eq!(
        cte(plan.sql(), "clause_0_relation_2 AS"),
        "clause_0_relation_2 AS ( SELECT candidate.* FROM clause_0_relation_2_candidates candidate WHERE NOT EXISTS ( SELECT 1 FROM clause_0_relation_2_candidates earlier WHERE earlier.file_index = candidate.file_index AND earlier.node = candidate.node AND earlier.branch_order < candidate.branch_order ) )"
    );
    assert!(plan.sql().contains("0 AS branch_order"));
    assert!(plan.sql().contains("1 AS branch_order"));
    assert_eq!(
        execute_boolean_sql(
            &plan,
            true,
            &[
                (0, true, Some(true), &[10]),
                (1, true, Some(true), &[10, 20]),
            ],
        ),
        vec![(0, 10), (1, 20)]
    );
}

#[test]
fn compiled_not_uses_completion_guarded_relational_antijoin() {
    let compiled = code_rule(json!({ "not": { "regex": "^x" } }));
    let plan =
        SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)]).expect("negation lowers into SQL");

    assert_eq!(plan.terminals().len(), 2);
    assert_eq!(
        plan.shape(),
        &[
            SqlShape::PackedMaterialized,
            SqlShape::StatusJoin,
            SqlShape::ScanTerminal,
            SqlShape::AntiJoin,
            SqlShape::CaseTruth,
            SqlShape::Projection,
            SqlShape::OrderBy,
        ]
    );
    assert_eq!(
        cte(plan.sql(), "clause_0_relation_2 AS"),
        "clause_0_relation_2 AS ( SELECT universe.* FROM clause_0_term_0 universe WHERE NOT EXISTS ( SELECT 1 FROM clause_0_term_1 excluded WHERE excluded.file_index = universe.file_index AND excluded.node = universe.node ) )"
    );
    assert!(plan.sql().contains("batch.complete = 1"));
    assert_eq!(
        execute_boolean_sql(
            &plan,
            true,
            &[
                (0, true, Some(true), &[10, 20]),
                (1, true, Some(true), &[20]),
            ],
        ),
        vec![(0, 10)]
    );
    assert!(
        execute_boolean_sql(
            &plan,
            true,
            &[(0, true, Some(true), &[10, 20]), (1, false, None, &[]),],
        )
        .is_empty()
    );
    assert!(
        execute_boolean_sql(
            &plan,
            true,
            &[(0, true, Some(true), &[10, 20]), (1, true, None, &[]),],
        )
        .is_empty()
    );
}

#[test]
fn unsupported_inventory_reports_every_operation_in_source_order() {
    let compiled = code_rule(json!({
        "has": { "kind": "call", "stopBy": "end" }
    }));
    let error = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
        .expect_err("the compiler reports unsupported relational operators");

    assert_eq!(
        error.unsupported(),
        &["lowering/ast-match/python/rule/has: descendant relation"]
    );
}
