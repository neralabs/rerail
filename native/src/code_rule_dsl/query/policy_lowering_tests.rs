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

fn execute_navigation_sql(
    plan: &SqlCatalogPlan,
    matches: &[(usize, &[i64])],
    edges: &[(i64, i64, i64, i64, bool)],
) -> Vec<i64> {
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
                verdict INTEGER, complete INTEGER, work_used INTEGER, error_code TEXT,
                field_id INTEGER, related_named INTEGER
            );
            INSERT INTO rerail_matches(batch, row_kind, complete) VALUES (1, 1, 1);",
        )
        .expect("typed packed relation");
    for producer in 0..plan.terminals().len() {
        connection
            .execute(
                "INSERT INTO rerail_matches(batch, row_kind, producer_id, file_index, verdict, complete)
                 VALUES (1, 2, ?1, 0, 1, 1)",
                [producer as i64],
            )
            .expect("producer status");
    }
    for (producer, nodes) in matches {
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
    let navigation = plan
        .terminals()
        .iter()
        .position(|terminal| terminal.kind() == TerminalKind::Navigation)
        .expect("navigation producer");
    for (order, (parent, child, child_order, field, named)) in edges.iter().enumerate() {
        connection
            .execute(
                "INSERT INTO rerail_matches(
                    batch, row_kind, producer_id, file_index, task_index, match_index,
                    node, related_node, axis_kind, axis_order, fact_kind, complete,
                    field_id, related_named
                 ) VALUES (1, 5, ?1, 0, 0, ?2, ?3, ?4, 1, ?5, NULL, 1, ?6, ?7)",
                params![
                    navigation as i64,
                    order as i64,
                    parent,
                    child,
                    child_order,
                    field,
                    i64::from(*named)
                ],
            )
            .expect("navigation edge");
    }
    let mut statement = connection.prepare(plan.sql()).expect("executable SQL plan");
    statement
        .query_map([1_i64], |row| row.get(10))
        .expect("query navigation rows")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("typed output rows")
}

fn compared_field_id(sql: &str) -> i64 {
    let suffix = sql
        .split("edge.field_id = ")
        .nth(1)
        .expect("field relation compares its field id");
    suffix
        .split(|character: char| !character.is_ascii_digit())
        .next()
        .expect("field id")
        .parse()
        .expect("numeric field id")
}

#[test]
fn compiled_ancestor_and_descendant_relations_execute_stop_semantics() {
    let inside = code_rule(json!({
        "inside": { "kind": "function_definition", "stopBy": "end" }
    }));
    let inside = SqlCatalogPlan::compile(&[Arc::clone(&inside.inner)])
        .expect("ancestor navigation lowers into SQL");
    assert_eq!(
        execute_navigation_sql(
            &inside,
            &[(0, &[3, 7]), (2, &[1, 5])],
            &[
                (0, 1, 0, 0, true),
                (1, 3, 0, 0, true),
                (0, 5, 1, 0, true),
                (5, 7, 0, 0, true)
            ]
        ),
        vec![3, 7]
    );

    let has = code_rule(json!({
        "has": {
            "kind": "return_statement",
            "stopBy": { "kind": "function_definition" }
        }
    }));
    let has = SqlCatalogPlan::compile(&[Arc::clone(&has.inner)])
        .expect("descendant navigation with a stop rule lowers into SQL");
    assert_eq!(
        execute_navigation_sql(
            &has,
            &[(0, &[0, 1]), (2, &[3]), (3, &[1])],
            &[(0, 1, 0, 0, true), (1, 2, 0, 0, true), (2, 3, 0, 0, true)]
        ),
        vec![1]
    );
}

#[test]
fn compiled_sibling_relations_execute_in_both_directions() {
    for (relation, source, related) in [("precedes", 10, 12), ("follows", 12, 10)] {
        let compiled = code_rule(json!({
            (relation): { "kind": "identifier", "stopBy": "end" }
        }));
        let plan = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
            .expect("sibling navigation lowers into SQL");
        assert_eq!(
            execute_navigation_sql(
                &plan,
                &[(0, &[source]), (2, &[related])],
                &[
                    (9, 10, 0, 0, true),
                    (9, 11, 1, 0, true),
                    (9, 12, 2, 0, true)
                ]
            ),
            vec![source],
            "{relation}"
        );
    }
}

#[test]
fn compiled_field_relation_uses_the_grammar_field_identity() {
    let compiled = code_rule(json!({
        "has": { "field": "body", "kind": "block", "stopBy": "neighbor" }
    }));
    let plan = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
        .expect("field navigation lowers into SQL");
    let field = compared_field_id(plan.sql());
    assert_eq!(
        execute_navigation_sql(&plan, &[(0, &[1]), (2, &[2])], &[(1, 2, 0, field, true)]),
        vec![1]
    );
    assert!(
        execute_navigation_sql(
            &plan,
            &[(0, &[1]), (2, &[2])],
            &[(1, 2, 0, field + 1, true)]
        )
        .is_empty()
    );
}

#[test]
fn compiled_nth_child_uses_named_sibling_order_and_reverse_order() {
    for (nth_child, expected) in [
        (json!("2n+1"), vec![10, 12]),
        (json!({ "position": 1, "reverse": true }), vec![12]),
    ] {
        let compiled = code_rule(json!({ "nthChild": nth_child }));
        let plan = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
            .expect("nth-child navigation lowers into SQL");
        assert_eq!(
            execute_navigation_sql(
                &plan,
                &[(0, &[10, 11, 12])],
                &[
                    (9, 10, 0, 0, true),
                    (9, 11, 1, 0, true),
                    (9, 12, 2, 0, true)
                ]
            ),
            expected
        );
    }
}

#[test]
fn unsupported_inventory_reports_every_operation_in_source_order() {
    let compiled = code_rule(json!({
        "count": { "has": { "kind": "if_statement", "stopBy": "end" }, "min": 2 }
    }));
    let error = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
        .expect_err("the compiler reports unsupported relational operators");

    assert_eq!(
        error.unsupported(),
        &["lowering/ast-match/python/rule: matcher operation is not lowered"]
    );
}
