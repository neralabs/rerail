use super::relational::{SqlCatalogPlan, SqlShape, TerminalKind};
use super::relational_ir::{normalized_binding_compatibility, qualified_row_columns};
use crate::code_rule_dsl::CodeRuleDslRuntime;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::model::Anchor;
use crate::code_rule_dsl::packed::ExecutionBackend;
use crate::code_rule_dsl::tape::{Grammar, TapeDoc, TapeImage};
use crate::code_rule_dsl::test_support::{CAPABILITIES, evaluate_file};
use crate::code_rule_dsl::transducer::sql_backend::{
    MatchRegistry, PackedDemand, ProducerKind, register_module,
};
use rusqlite::{Connection, params};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;

fn code_rule(rule: serde_json::Value) -> crate::code_rule_dsl::CompiledCodeRuleDsl {
    code_rule_ast(json!({ "language": "python", "rule": rule }))
}

fn code_rule_with_message(
    rule: serde_json::Value,
    message: &str,
) -> crate::code_rule_dsl::CompiledCodeRuleDsl {
    compile_code_rule(json!({ "language": "python", "rule": rule }), message)
}

fn code_rule_ast(ast: serde_json::Value) -> crate::code_rule_dsl::CompiledCodeRuleDsl {
    compile_code_rule(ast, "x")
}

fn compile_code_rule(
    ast: serde_json::Value,
    message: &str,
) -> crate::code_rule_dsl::CompiledCodeRuleDsl {
    CodeRuleDslRuntime::compile(
        json!({
            "format": "rerail.policy/v1",
            "id": "lowering",
            "revision": 1,
            "mode": "block",
            "clauses": [{
                "id": "ast-match",
                "forbid": { "ast": ast },
                "violation": { "anchor": "witness.span", "code": "X", "message": message }
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
                verdict INTEGER, complete INTEGER, work_used INTEGER, error_code TEXT,
                field_id INTEGER, related_named INTEGER, source_text TEXT,
                structural_key TEXT, named_leaf INTEGER, capture_kind INTEGER, capture_named INTEGER, equality_occurrence INTEGER
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
    let verdict = "CASE WHEN left_input.verdict = 0 OR right_input.verdict = 0 THEN 0 WHEN left_input.verdict IS NULL OR right_input.verdict IS NULL THEN NULL ELSE 1 END";
    let columns = qualified_row_columns("left_input")
        .replace("left_input.verdict", &format!("{verdict} AS verdict"));
    assert_eq!(
        cte(plan.sql(), "clause_0_relation_2 AS"),
        format!(
            "clause_0_relation_2 AS ( SELECT {columns}, left_input.bindings, left_input.choice_order FROM clause_0_term_0 left_input INNER JOIN clause_0_term_1 right_input ON right_input.file_index = left_input.file_index AND right_input.node = left_input.node )"
        )
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
    let columns = qualified_row_columns("branch");
    assert_eq!(
        cte(plan.sql(), "clause_0_relation_2_branches AS"),
        format!(
            "clause_0_relation_2_branches AS ( SELECT {columns}, branch.bindings, printf('%08d/', 0) || branch.choice_order AS choice_order, 0 AS branch_order, 0 AS unknown_blocks FROM clause_0_term_0 branch UNION ALL SELECT {columns}, branch.bindings, printf('%08d/', 1) || branch.choice_order AS choice_order, 1 AS branch_order, 0 AS unknown_blocks FROM clause_0_term_1 branch )"
        )
    );
    let chosen = qualified_row_columns("chosen");
    assert_eq!(
        cte(plan.sql(), "clause_0_relation_2 AS"),
        format!(
            "clause_0_relation_2 AS ( SELECT {chosen}, chosen.bindings, chosen.choice_order FROM ( SELECT branch.*, row_number() OVER (PARTITION BY file_index, node ORDER BY CASE WHEN verdict = 1 OR (verdict IS NULL AND unknown_blocks = 1) THEN 0 WHEN verdict IS NULL THEN 1 ELSE 2 END, branch_order, choice_order, producer_id, match_index) AS truth_order FROM clause_0_relation_2_branches branch ) chosen WHERE chosen.truth_order = 1 AND chosen.verdict IS NOT 0 )"
        )
    );
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
    let compatible = normalized_binding_compatibility("universe", "excluded");
    let verdict = format!(
        "CASE WHEN EXISTS (SELECT 1 FROM clause_0_term_1 excluded WHERE excluded.file_index = universe.file_index AND excluded.node = universe.node AND excluded.verdict IS NULL AND {compatible}) THEN NULL ELSE 1 END"
    );
    let columns = qualified_row_columns("universe")
        .replace("universe.verdict", &format!("{verdict} AS verdict"));
    assert_eq!(
        cte(plan.sql(), "clause_0_relation_2 AS"),
        format!(
            "clause_0_relation_2 AS ( SELECT {columns}, universe.bindings, universe.choice_order FROM clause_0_term_0 universe WHERE universe.verdict = 1 AND NOT EXISTS ( SELECT 1 FROM clause_0_term_1 excluded WHERE excluded.file_index = universe.file_index AND excluded.node = universe.node AND excluded.verdict = 1 AND {compatible} ) )"
        )
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
                field_id INTEGER, related_named INTEGER, source_text TEXT,
                structural_key TEXT, named_leaf INTEGER, capture_kind INTEGER, capture_named INTEGER, equality_occurrence INTEGER
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
    assert_eq!(
        execute_navigation_sql(
            &has,
            &[(0, &[0]), (2, &[4]), (3, &[1])],
            &[(0, 1, 0, 0, true), (1, 2, 0, 0, true), (0, 4, 1, 0, true)]
        ),
        vec![0],
        "a stop node cuts only its own descendant branch"
    );
    assert_eq!(
        execute_navigation_sql(
            &has,
            &[(0, &[0]), (2, &[1]), (3, &[1])],
            &[(0, 1, 0, 0, true), (0, 4, 1, 0, true)]
        ),
        vec![0],
        "the stop node is an inclusive relation candidate"
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

    let compiled = code_rule(json!({
        "inside": { "field": "body", "kind": "function_definition", "stopBy": "end" }
    }));
    let plan = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
        .expect("inverse field navigation lowers into SQL");
    let field = compared_field_id(plan.sql());
    assert_eq!(
        execute_navigation_sql(
            &plan,
            &[(0, &[3]), (2, &[1])],
            &[(1, 2, 0, field, true), (2, 3, 0, field + 1, true)]
        ),
        vec![3]
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

    let compiled = code_rule(json!({
        "nthChild": { "position": 2, "ofRule": { "kind": "identifier" } }
    }));
    let plan = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
        .expect("filtered nth-child navigation lowers into SQL");
    assert_eq!(
        execute_navigation_sql(
            &plan,
            &[(0, &[10, 11, 12]), (2, &[11, 12])],
            &[
                (9, 10, 0, 0, true),
                (9, 11, 1, 0, true),
                (9, 12, 2, 0, true)
            ]
        ),
        vec![12]
    );

    let compiled = code_rule(json!({
        "nthChild": { "position": 1, "ofRule": { "kind": "identifier" } }
    }));
    let plan = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
        .expect("filtered first-child navigation lowers into SQL");
    assert_eq!(
        execute_navigation_sql(
            &plan,
            &[(0, &[10, 11, 12]), (2, &[11, 12])],
            &[
                (9, 10, 0, 0, true),
                (9, 11, 1, 0, true),
                (9, 12, 2, 0, true)
            ]
        ),
        vec![11]
    );
}

type CaptureFixture<'a> = (usize, i64, i64, &'a str, i64, i64, &'a str, &'a str);

fn execute_capture_sql(
    plan: &SqlCatalogPlan,
    matches: &[(usize, i64, i64)],
    captures: &[CaptureFixture<'_>],
    edges: &[(i64, i64, i64)],
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
                field_id INTEGER, related_named INTEGER, source_text TEXT,
                structural_key TEXT, named_leaf INTEGER, capture_kind INTEGER, capture_named INTEGER, equality_occurrence INTEGER
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
    for (producer, match_index, node) in matches {
        connection
            .execute(
                "INSERT INTO rerail_matches(
                    batch, row_kind, producer_id, file_index, task_index, match_index,
                    node, start_ordinal, complete
                 ) VALUES (1, 3, ?1, 0, ?2, ?2, ?3, ?3, 1)",
                params![*producer as i64, match_index, node],
            )
            .expect("terminal match");
    }
    for (producer, match_index, capture_index, label, occurrence, node, text, structure) in captures
    {
        connection
            .execute(
                "INSERT INTO rerail_matches(
                    batch, row_kind, producer_id, file_index, match_index, capture_index,
                    capture_occurrence, capture_sequence_start, capture_sequence_count,
                    node, capture_label, source_text, structural_key, named_leaf, capture_kind, complete
                 ) VALUES (1, 4, ?1, 0, ?2, ?3, ?4, 0, 1, ?5, ?6, ?7, ?8, 1, 1, 1)",
                params![
                    *producer as i64,
                    match_index,
                    capture_index,
                    occurrence,
                    node,
                    label,
                    text,
                    structure
                ],
            )
            .expect("capture row");
    }
    if !edges.is_empty() {
        let navigation = plan
            .terminals()
            .iter()
            .position(|terminal| terminal.kind() == TerminalKind::Navigation)
            .expect("navigation producer");
        for (match_index, (parent, child, child_order)) in edges.iter().enumerate() {
            connection
                .execute(
                    "INSERT INTO rerail_matches(
                        batch, row_kind, producer_id, file_index, match_index, node,
                        related_node, axis_kind, axis_order, field_id, related_named, complete
                     ) VALUES (1, 5, ?1, 0, ?2, ?3, ?4, 1, ?5, 0, 1, 1)",
                    params![
                        navigation as i64,
                        match_index as i64,
                        parent,
                        child,
                        child_order
                    ],
                )
                .expect("navigation edge");
        }
    }
    let mut statement = connection.prepare(plan.sql()).expect("executable SQL plan");
    statement
        .query_map([1_i64], |row| row.get(10))
        .expect("query capture rows")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("typed output rows")
}

#[test]
fn compiled_capture_constraints_run_on_the_bound_node() {
    let compiled = code_rule_ast(json!({
        "language": "python",
        "rule": { "pattern": "$A" },
        "constraints": { "A": { "kind": "identifier" } }
    }));
    let plan = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
        .expect("a capture constraint lowers into SQL");

    assert_eq!(
        execute_capture_sql(
            &plan,
            &[(0, 0, 10), (1, 0, 20)],
            &[(0, 0, 0, "A", 0, 20, "item", "identifier:item")],
            &[]
        ),
        vec![10]
    );
    assert!(
        execute_capture_sql(
            &plan,
            &[(0, 0, 10), (1, 0, 21)],
            &[(0, 0, 0, "A", 0, 20, "item", "identifier:item")],
            &[]
        )
        .is_empty()
    );
}

#[test]
fn navigation_preserves_the_first_related_capture_environment() {
    let compiled = code_rule_ast(json!({
        "language": "python",
        "rule": { "has": { "pattern": "$A", "stopBy": "end" } },
        "constraints": { "A": { "kind": "identifier" } }
    }));
    let plan = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
        .expect("a related capture environment lowers into SQL");

    assert_eq!(
        execute_capture_sql(
            &plan,
            &[(0, 0, 0), (2, 0, 2), (3, 0, 2)],
            &[(2, 0, 0, "A", 0, 2, "item", "identifier:item")],
            &[(0, 2, 0)]
        ),
        vec![0]
    );
}

#[test]
fn repeated_pattern_captures_require_structural_equality() {
    let compiled = code_rule(json!({ "pattern": "$A + $A" }));
    let plan = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
        .expect("a repeated capture pattern lowers into SQL");

    let matches = [(0, 0, 10)];
    assert_eq!(
        execute_capture_sql(
            &plan,
            &matches,
            &[
                (0, 0, 0, "A", 0, 20, "item", "identifier:item"),
                (0, 0, 1, "A", 1, 21, "item", "identifier:item")
            ],
            &[]
        ),
        vec![10]
    );
    assert!(
        execute_capture_sql(
            &plan,
            &matches,
            &[
                (0, 0, 0, "A", 0, 20, "item", "identifier:item"),
                (0, 0, 1, "A", 1, 21, "other", "identifier:other")
            ],
            &[]
        )
        .is_empty()
    );
}

#[test]
fn packed_repeated_capture_pattern_and_sql_equality_execute_together() {
    let compiled = code_rule(json!({ "pattern": "$A + $A" }));
    let plan = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
        .expect("a repeated capture pattern lowers into SQL");
    let source = Arc::new("x + x\ny + z\n".to_owned());
    let grammar = Grammar::new(GrammarDialect::Python).expect("Python grammar");
    let image = TapeImage::parse(&grammar, &source, &|| false)
        .expect("parser completed")
        .expect("tape image");
    let tape =
        TapeDoc::restore(Arc::clone(&source), grammar, &image, &|| false).expect("restored tape");
    let roots = (0..tape.transducer_len() as u32).collect::<Vec<_>>();
    let demand = PackedDemand::new(
        ExecutionBackend::CpuJit,
        plan.terminals()
            .iter()
            .map(|terminal| terminal.program().clone())
            .collect(),
        vec![tape],
        vec![vec![roots]],
        vec![vec![true]],
        vec![u32::MAX],
    )
    .and_then(|demand| demand.with_producer_kinds(vec![ProducerKind::Pattern]))
    .expect("immutable pattern demand");
    let connection = Connection::open_in_memory().expect("in-memory SQLite");
    let registry = Arc::new(MatchRegistry::default());
    register_module(&connection, Arc::clone(&registry)).expect("packed virtual table");
    let token = registry.register(demand);
    let nodes = connection
        .prepare(plan.sql())
        .expect("executable SQL plan")
        .query_map([token], |row| row.get::<_, i64>(10))
        .expect("SQL-only repeated capture query")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("typed results");
    assert_eq!(nodes.len(), 1, "only x + x has equal repeated captures");
    assert_eq!(registry.query_execution_counters(token), Some((1, 1)));
}

#[test]
fn phase_two_comparison_and_value_have_relational_sql_and_ordered_catalogs() {
    let comparison = code_rule(json!({
        "all": [
            { "pattern": "$A + $B" },
            { "compare": { "left": "A", "op": "eq", "right": "B", "text": "source" } }
        ]
    }));
    let comparison = SqlCatalogPlan::compile(&[Arc::clone(&comparison.inner)])
        .expect("source comparison lowers");
    assert_eq!(comparison.terminals().len(), 1);
    assert_eq!(comparison.terminals()[0].kind(), TerminalKind::Pattern);
    let compare_sql = cte(comparison.sql(), "clause_0_compare_1 AS");
    assert!(compare_sql.contains("json_extract(left_value.value, '$.label') = 'A'"));
    assert!(compare_sql.contains("json_extract(right_value.value, '$.label') = 'B'"));
    assert!(compare_sql.contains(
        "json_extract(left_value.value, '$.text') = json_extract(right_value.value, '$.text')"
    ));
    assert!(compare_sql.contains("THEN NULL"));

    let value = code_rule(json!({
        "kind": "identifier",
        "value": { "follow": true, "eq": 1 }
    }));
    let value =
        SqlCatalogPlan::compile(&[Arc::clone(&value.inner)]).expect("flow-dependent value lowers");
    assert_eq!(
        value
            .terminals()
            .iter()
            .map(|terminal| terminal.kind())
            .collect::<Vec<_>>(),
        vec![TerminalKind::Kind, TerminalKind::Value]
    );
    assert_eq!(
        value.terminals()[1].producer_kind(),
        ProducerKind::ValueFact
    );
    let descriptor = value.terminals()[1]
        .value_fact()
        .expect("value catalog entry has one immutable descriptor");
    assert_eq!(descriptor.language, GrammarDialect::Python);
    assert!(descriptor.follow);
    assert!(!descriptor.expression_kinds.is_empty());
    let value_sql = cte(value.sql(), "clause_0_value_1 AS");
    assert!(value_sql.contains("JOIN fact value_fact"));
    assert!(value_sql.contains("value_fact.fact_kind = 1"));
    assert!(value_sql.contains("value_fact.text_value = 'int:1'"));
    assert!(value_sql.contains("THEN NULL"));
}

#[test]
fn phase_two_comparison_value_and_truth_sql_equal_the_oracle() {
    let cases = [
        (
            json!({
                "all": [
                    { "pattern": "$A + $B" },
                    { "compare": { "left": "A", "op": "eq", "right": "B", "text": "source" } }
                ]
            }),
            "x + x\nx + y\n",
        ),
        (json!({ "kind": "integer", "value": { "eq": 1 } }), "1\n2\n"),
        (json!({ "value": { "eq": 1 } }), "1 + 0\nTrue\n1.0\n2\n"),
        (
            json!({ "kind": "identifier", "value": { "follow": true, "eq": 1 } }),
            "known = 1\nknown\nunknown\n",
        ),
        (
            json!({
                "all": [
                    { "kind": "identifier", "value": { "follow": true, "eq": 1 } },
                    { "kind": "comment" }
                ]
            }),
            "unknown\n",
        ),
        (
            json!({
                "any": [
                    { "kind": "identifier", "value": { "follow": true, "eq": 1 } },
                    { "kind": "identifier" }
                ]
            }),
            "known = 1\nknown\nunknown\n",
        ),
        (
            json!({
                "all": [
                    { "pattern": "$A", "value": { "follow": true, "eq": 1 } },
                    { "compare": { "left": "A", "op": "eq", "right": "A", "text": "source" } }
                ]
            }),
            "known = 1\nknown\nunknown\n",
        ),
        (
            json!({
                "any": [
                    { "pattern": "$A", "value": { "follow": true, "eq": 1 } },
                    { "pattern": "$A" }
                ]
            }),
            "known = 1\nknown\nunknown\n",
        ),
        (
            json!({ "not": { "kind": "identifier", "value": { "follow": true, "eq": 1 } } }),
            "known = 1\nknown\nunknown\n",
        ),
    ];
    for (case_index, (rule, source)) in cases.into_iter().enumerate() {
        let compiled = code_rule(rule);
        assert_eq!(
            sql_ranges(&compiled, source),
            oracle_ranges(&compiled, source),
            "case {case_index}: {source}"
        );
    }
}

#[test]
fn disabled_when_is_an_executable_empty_relation_without_terminal_work() {
    let compiled = code_rule_ast(json!({
        "language": "python",
        "languageOptions": { "pythonVersion": "3.15" },
        "when": { "pythonVersion": { "max": "3.7" } },
        "rule": { "kind": "identifier" }
    }));
    let plan = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
        .expect("disabled when lowers to an executable empty relation");
    assert!(plan.terminals().is_empty());
    assert_eq!(sql_ranges(&compiled, "x\n"), Vec::<(usize, usize)>::new());
}

pub(super) struct SqlOutcome {
    pub(super) ranges: Vec<(usize, usize)>,
    pub(super) errors: Vec<String>,
}

pub(super) fn sql_outcome(
    compiled: &crate::code_rule_dsl::CompiledCodeRuleDsl,
    source: &str,
) -> SqlOutcome {
    let plan = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
        .expect("policy has complete structural SQL lowering");
    let source = Arc::new(source.to_owned());
    let grammar = Grammar::new(GrammarDialect::Python).expect("Python grammar");
    let image = TapeImage::parse(&grammar, &source, &|| false)
        .expect("parser completed")
        .expect("tape image");
    let tape =
        TapeDoc::restore(Arc::clone(&source), grammar, &image, &|| false).expect("restored tape");
    let spans = (0..tape.transducer_len() as u32)
        .map(|node| tape.transducer_span(node))
        .collect::<Vec<_>>();
    let roots = (0..tape.transducer_len() as u32).collect::<Vec<_>>();
    let anchors = plan
        .terminals()
        .iter()
        .map(|terminal| {
            if terminal.kind() == TerminalKind::Navigation {
                vec![0]
            } else {
                roots.clone()
            }
        })
        .collect::<Vec<_>>();
    let producer_kinds = plan
        .terminals()
        .iter()
        .map(|terminal| terminal.producer_kind())
        .collect::<Vec<_>>();
    let demand = PackedDemand::new(
        ExecutionBackend::CpuJit,
        plan.terminals()
            .iter()
            .map(|terminal| terminal.program().clone())
            .collect(),
        vec![tape],
        vec![anchors],
        vec![vec![true; plan.terminals().len()]],
        plan.terminals()
            .iter()
            .map(|terminal| terminal.result_limit())
            .collect(),
    )
    .and_then(|demand| demand.with_producer_kinds(producer_kinds))
    .and_then(|demand| {
        demand.with_value_facts(
            plan.terminals()
                .iter()
                .map(|terminal| terminal.value_fact())
                .collect(),
        )
    })
    .expect("immutable structural demand");
    let connection = Connection::open_in_memory().expect("in-memory SQLite");
    let registry = Arc::new(MatchRegistry::default());
    register_module(&connection, Arc::clone(&registry)).expect("packed virtual table");
    let token = registry.register(demand);
    let rows = connection
        .prepare(plan.sql())
        .expect("executable SQL plan")
        .query_map([token], |row| {
            Ok((
                row.get::<_, i64>(1)?,
                row.get::<_, Option<i64>>(10)?,
                row.get::<_, Option<String>>(27)?,
                row.get::<_, String>(35)?,
            ))
        })
        .expect("SQL structural query")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("typed SQL nodes");
    let expected_executions = if plan.terminals().is_empty() {
        (0, 0)
    } else {
        (1, 1)
    };
    assert_eq!(
        registry.query_execution_counters(token),
        Some(expected_executions)
    );
    let anchor = &compiled.inner.clauses[0].violation.anchor;
    let mut errors = rows
        .iter()
        .filter(|(row_kind, _, _, _)| *row_kind == 2)
        .map(|(_, _, error, _)| error.clone().expect("a failure row has an error code"))
        .collect::<Vec<_>>();
    errors.sort();
    errors.dedup();
    let mut ranges = rows
        .into_iter()
        .filter(|(row_kind, _, _, _)| *row_kind == 3)
        .map(|(_, node, _, bindings)| {
            let node = node.expect("a violation row has a node");
            let node = match anchor {
                Anchor::Capture(label) => serde_json::from_str::<serde_json::Value>(&bindings)
                    .expect("JSON binding environment")
                    .as_array()
                    .expect("binding array")
                    .iter()
                    .find(|binding| binding["label"] == *label && binding["kind"] == 1)
                    .and_then(|binding| binding["node"].as_i64())
                    .unwrap_or(node),
                Anchor::Span | Anchor::Start | Anchor::End | Anchor::FileStart => node,
            };
            let span = &spans[node as usize];
            (span.start, span.end)
        })
        .collect::<Vec<_>>();
    ranges.sort_unstable();
    SqlOutcome { ranges, errors }
}

pub(super) fn sql_ranges(
    compiled: &crate::code_rule_dsl::CompiledCodeRuleDsl,
    source: &str,
) -> Vec<(usize, usize)> {
    sql_outcome(compiled, source).ranges
}

fn oracle_ranges(
    compiled: &crate::code_rule_dsl::CompiledCodeRuleDsl,
    source: &str,
) -> Vec<(usize, usize)> {
    let result = evaluate_file(compiled, "python", "a.py", source);
    assert_eq!(result["failures"], json!([]), "{result}");
    let mut ranges = result["violations"]
        .as_array()
        .expect("violations")
        .iter()
        .map(|violation| {
            (
                violation["span"]["startByte"].as_u64().expect("start") as usize,
                violation["span"]["endByte"].as_u64().expect("end") as usize,
            )
        })
        .collect::<Vec<_>>();
    ranges.sort_unstable();
    ranges
}

fn sql_single_bindings(
    compiled: &crate::code_rule_dsl::CompiledCodeRuleDsl,
    source: &str,
) -> Vec<((usize, usize), BTreeMap<String, String>)> {
    let plan = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
        .expect("policy has complete structural SQL lowering");
    let source = Arc::new(source.to_owned());
    let grammar = Grammar::new(GrammarDialect::Python).expect("Python grammar");
    let image = TapeImage::parse(&grammar, &source, &|| false)
        .expect("parser completed")
        .expect("tape image");
    let tape =
        TapeDoc::restore(Arc::clone(&source), grammar, &image, &|| false).expect("restored tape");
    let spans = (0..tape.transducer_len() as u32)
        .map(|node| tape.transducer_span(node))
        .collect::<Vec<_>>();
    let roots = (0..tape.transducer_len() as u32).collect::<Vec<_>>();
    let anchors = plan
        .terminals()
        .iter()
        .map(|terminal| {
            if terminal.kind() == TerminalKind::Navigation {
                vec![0]
            } else {
                roots.clone()
            }
        })
        .collect::<Vec<_>>();
    let demand = PackedDemand::new(
        ExecutionBackend::CpuJit,
        plan.terminals()
            .iter()
            .map(|terminal| terminal.program().clone())
            .collect(),
        vec![tape],
        vec![anchors],
        vec![vec![true; plan.terminals().len()]],
        plan.terminals()
            .iter()
            .map(|terminal| terminal.result_limit())
            .collect(),
    )
    .and_then(|demand| {
        demand.with_producer_kinds(
            plan.terminals()
                .iter()
                .map(|terminal| terminal.producer_kind())
                .collect(),
        )
    })
    .expect("immutable structural demand");
    let connection = Connection::open_in_memory().expect("in-memory SQLite");
    let registry = Arc::new(MatchRegistry::default());
    register_module(&connection, Arc::clone(&registry)).expect("packed virtual table");
    let token = registry.register(demand);
    connection
        .prepare(plan.sql())
        .expect("executable SQL plan")
        .query_map([token], |row| {
            Ok((row.get::<_, i64>(10)?, row.get::<_, String>(35)?))
        })
        .expect("SQL structural query")
        .map(|row| {
            let (node, bindings) = row.expect("typed SQL environment");
            let bindings: serde_json::Value =
                serde_json::from_str(&bindings).expect("JSON binding environment");
            let mut singles = BTreeMap::new();
            for binding in bindings.as_array().expect("binding array") {
                if binding["kind"] == 1 {
                    let label = binding["label"].as_str().expect("capture label");
                    let text = binding["text"].as_str().expect("capture text");
                    if let Some(previous) = singles.insert(label.to_owned(), text.to_owned()) {
                        assert_eq!(previous, text, "one single capture has one value");
                    }
                }
            }
            let span = &spans[node as usize];
            ((span.start, span.end), singles)
        })
        .collect()
}

#[test]
fn sql_ordered_alternatives_publish_the_oracle_capture_environment() {
    let compiled = code_rule_with_message(
        json!({
            "all": [
                { "pattern": "$A + $B" },
                { "any": [
                    { "pattern": "$X + $A" },
                    { "pattern": "$A + $X" }
                ] }
            ]
        }),
        "A=$A X=$X",
    );
    let sql = sql_single_bindings(&compiled, "x + y\n");
    let oracle = evaluate_file(&compiled, "python", "a.py", "x + y\n");
    assert_eq!(oracle["failures"], json!([]), "{oracle}");
    assert_eq!(oracle["violations"][0]["message"], "A=x X=y");
    assert_eq!(
        sql,
        vec![(
            (0, 5),
            BTreeMap::from([
                ("A".to_owned(), "x".to_owned()),
                ("B".to_owned(), "y".to_owned()),
                ("X".to_owned(), "y".to_owned()),
            ])
        )]
    );
}

#[test]
fn sql_nested_alternatives_preserve_lexical_choice_order() {
    let compiled = code_rule_with_message(
        json!({
            "all": [
                { "any": [
                    { "pattern": "$A + $X" },
                    { "pattern": "$X + $A" }
                ] },
                { "any": [
                    { "pattern": "$Z + $A" },
                    { "pattern": "$A + $Z" }
                ] }
            ]
        }),
        "A=$A",
    );
    let source = "x + y\n";
    let sql = sql_single_bindings(&compiled, source);
    let oracle = evaluate_file(&compiled, "python", "a.py", source);
    assert_eq!(oracle["failures"], json!([]), "{oracle}");
    assert_eq!(oracle["violations"][0]["message"], "A=x");
    assert_eq!(sql[0].1["A"], "x");
}

#[test]
fn sql_constraints_publish_new_oracle_bindings() {
    let compiled = compile_code_rule(
        json!({
            "language": "python",
            "rule": { "pattern": "$A + 1" },
            "constraints": { "A": { "pattern": "$B" } }
        }),
        "A=$A B=$B",
    );
    let source = "x + 1\n";
    let sql = sql_single_bindings(&compiled, source);
    let oracle = evaluate_file(&compiled, "python", "a.py", source);
    assert_eq!(oracle["failures"], json!([]), "{oracle}");
    assert_eq!(oracle["violations"][0]["message"], "A=x B=x");
    assert_eq!(
        sql,
        vec![(
            (0, 5),
            BTreeMap::from([
                ("A".to_owned(), "x".to_owned()),
                ("B".to_owned(), "x".to_owned()),
            ])
        )]
    );
}

#[test]
fn sql_count_bindings_publish_the_oracle_value() {
    let compiled = code_rule_with_message(
        json!({
            "kind": "function_definition",
            "count": {
                "has": { "kind": "return_statement", "stopBy": "end" },
                "min": 2,
                "as": "CCN",
                "offset": 1
            }
        }),
        "CCN=$CCN",
    );
    let source = "def f():\n return 1\n return 2\n";
    let sql = sql_single_bindings(&compiled, source);
    let oracle = evaluate_file(&compiled, "python", "a.py", source);
    assert_eq!(oracle["failures"], json!([]), "{oracle}");
    assert_eq!(oracle["violations"][0]["message"], "CCN=3");
    assert_eq!(
        sql,
        vec![(
            (0, source.len() - 1),
            BTreeMap::from([("CCN".to_owned(), "3".to_owned())])
        )]
    );
}

#[test]
fn sql_structural_core_matches_the_matcher_oracle() {
    let cases = [
        (
            code_rule(json!({ "pattern": "f($$$A)" })),
            "f(a, b)\ng(c)\nf(# comment\n  d, e)\n",
        ),
        (code_rule(json!({ "pattern": "$A + $A" })), "x + x\ny + z\n"),
        (
            code_rule(json!({ "pattern": "f($A)" })),
            "f(# before\n x # after\n)\nf(1)\n",
        ),
        (
            code_rule(json!({ "pattern": "f($$$A, $$$B)" })),
            "f(a, b, c)\nf(a)\nf()\n",
        ),
        (
            code_rule(json!({
                "pattern": {
                    "context": "def $F():\n    $$$BODY",
                    "selector": "function_definition"
                }
            })),
            "def f():\n    return 1\n\nx = 1\n",
        ),
        (
            code_rule_ast(json!({
                "language": "python",
                "rule": { "pattern": "$A" },
                "constraints": { "A": { "kind": "identifier" } }
            })),
            "x\n1\n",
        ),
        (
            code_rule(json!({
                "kind": "function_definition",
                "count": {
                    "has": { "kind": "return_statement", "stopBy": "end" },
                    "min": 2,
                    "max": 2
                }
            })),
            "def two():\n return 1\n return 2\n\ndef one():\n return 1\n",
        ),
        (
            code_rule(json!({
                "pattern": "$F($$$)",
                "count": {
                    "has": {
                        "pattern": "$F",
                        "kind": "identifier",
                        "stopBy": "end"
                    },
                    "min": 3
                }
            })),
            "f(g, g)\nf(f, f)\ng(g)\n",
        ),
        (
            code_rule(json!({
                "kind": "if_statement",
                "count": {
                    "inside": { "kind": "if_statement", "stopBy": "end" },
                    "min": 2
                }
            })),
            "if a:\n if b:\n  if c:\n   pass\n",
        ),
        (
            code_rule_ast(json!({
                "language": "python",
                "rule": { "has": { "pattern": "$A", "stopBy": "end" } },
                "constraints": { "A": { "kind": "identifier" } }
            })),
            "f(1)\ng(x)\n",
        ),
    ];
    for (compiled, source) in cases {
        assert_eq!(
            sql_ranges(&compiled, source),
            oracle_ranges(&compiled, source),
            "{source}"
        );
    }
}

#[test]
fn sql_structural_composition_matrix_matches_the_matcher_oracle() {
    let cases = vec![
        (
            code_rule(json!({
                "all": [
                    { "pattern": "$A + $B" },
                    { "any": [
                        { "pattern": "$X + $A" },
                        { "all": [
                            { "pattern": "$A + $Y" },
                            { "not": { "pattern": "$Y + $A" } }
                        ] }
                    ] }
                ]
            })),
            "x + y\ny + y\n",
        ),
        (
            code_rule(json!({
                "all": [
                    { "pattern": "$F($$$)" },
                    { "has": { "pattern": "$F", "stopBy": "end" } }
                ]
            })),
            "f(x)\ng(f)\n",
        ),
        (
            code_rule(json!({
                "all": [
                    { "pattern": "$F($$$)" },
                    { "not": { "has": { "pattern": "$F", "stopBy": "end" } } }
                ]
            })),
            "f(x)\ng(f)\n",
        ),
        (
            code_rule(json!({
                "all": [
                    { "pattern": "$F($$$)" },
                    { "any": [
                        { "has": { "pattern": "$X", "stopBy": "end" } },
                        { "has": { "pattern": "$F", "stopBy": "end" } }
                    ] }
                ]
            })),
            "f(x)\ng(f)\n",
        ),
        (
            code_rule(json!({
                "pattern": "$F($$$)",
                "count": {
                    "has": { "pattern": "$F", "stopBy": "end" },
                    "min": 1,
                    "max": 1
                }
            })),
            "f(x)\ng(f)\nf(f)\n",
        ),
        (
            code_rule(json!({
                "all": [
                    { "pattern": "$F($$$)" },
                    { "count": {
                        "has": { "pattern": "$F", "stopBy": "end" },
                        "min": 1,
                        "max": 1
                    } }
                ]
            })),
            "f(x)\ng(f)\nf(f)\n",
        ),
        (
            code_rule(json!({
                "all": [
                    { "pattern": "check($F($$$))" },
                    { "has": {
                        "kind": "argument_list",
                        "count": {
                            "has": { "pattern": "$F", "stopBy": "end" },
                            "min": 2
                        },
                        "stopBy": "end"
                    } }
                ]
            })),
            "check(f(g, g))\ncheck(f(f, f))\n",
        ),
        (
            code_rule(json!({
                "pattern": "check($F($$$))",
                "count": {
                    "has": {
                        "kind": "argument_list",
                        "count": {
                            "has": { "pattern": "$F", "stopBy": "end" },
                            "min": 2
                        },
                        "stopBy": "end"
                    },
                    "min": 1
                }
            })),
            "check(f(g, g))\ncheck(f(f, f))\n",
        ),
        (
            code_rule(json!({
                "kind": "identifier",
                "inside": {
                    "any": [
                        { "pattern": "def $A():\n    $$$" },
                        { "kind": "function_definition" }
                    ],
                    "stopBy": "end"
                }
            })),
            "def f():\n x = 1\n\ny = 2\n",
        ),
        (
            code_rule(json!({
                "kind": "function_definition",
                "has": {
                    "kind": "return_statement",
                    "stopBy": { "kind": "function_definition" }
                }
            })),
            "def outer():\n def inner():\n  return 1\n return 2\n",
        ),
        (
            code_rule(json!({
                "kind": "function_definition",
                "count": {
                    "has": {
                        "kind": "return_statement",
                        "stopBy": { "kind": "function_definition" }
                    },
                    "min": 1,
                    "max": 1
                }
            })),
            "def outer():\n def inner():\n  return 1\n return 2\n",
        ),
        (
            code_rule(json!({
                "kind": "expression_statement",
                "precedes": {
                    "kind": "expression_statement",
                    "stopBy": { "pattern": "stop()" }
                }
            })),
            "a()\nb()\nstop()\nc()\nd()\n",
        ),
        (
            code_rule(json!({
                "kind": "expression_statement",
                "follows": {
                    "kind": "expression_statement",
                    "stopBy": { "pattern": "stop()" }
                }
            })),
            "a()\nb()\nstop()\nc()\nd()\n",
        ),
        (
            code_rule(json!({
                "kind": "function_definition",
                "has": { "field": "body", "kind": "block", "stopBy": "neighbor" }
            })),
            "def f():\n x = 1\n",
        ),
        (
            code_rule(json!({
                "kind": "identifier",
                "inside": { "field": "body", "kind": "function_definition", "stopBy": "end" }
            })),
            "def f():\n x = 1\n",
        ),
        (
            code_rule(json!({
                "kind": "identifier",
                "nthChild": { "position": "2n+1", "reverse": true }
            })),
            "f(a, b, c, d)\n",
        ),
        (
            code_rule(json!({
                "kind": "identifier",
                "nthChild": { "position": 2, "ofRule": { "kind": "identifier" } }
            })),
            "f(a, b, c, d)\n",
        ),
        (
            code_rule(json!({
                "all": [
                    { "pattern": "$A" },
                    { "nthChild": { "position": 1, "ofRule": { "pattern": "$A" } } }
                ]
            })),
            "f(a, b)\n",
        ),
        (
            code_rule(json!({ "pattern": "[$$$A, 0, $$$A]" })),
            "[x, y, 0, x, y]\n[x, 0, y]\n",
        ),
    ];
    for (compiled, source) in cases {
        assert_eq!(
            sql_ranges(&compiled, source),
            oracle_ranges(&compiled, source),
            "{source}"
        );
    }
}

#[test]
fn sql_boolean_cross_product_matches_the_matcher_oracle() {
    let atoms = [
        json!({ "kind": "identifier" }),
        json!({ "regex": "^[fgx]$" }),
        json!({ "pattern": "$A" }),
        json!({ "has": { "kind": "identifier", "stopBy": "neighbor" } }),
    ];
    let source = "def f(x):\n return g(x)\n";
    for left in &atoms {
        for right in &atoms {
            for rule in [
                json!({ "all": [left.clone(), right.clone()] }),
                json!({ "any": [left.clone(), right.clone()] }),
                json!({ "all": [left.clone(), { "not": right.clone() }] }),
            ] {
                let compiled = code_rule(rule);
                assert_eq!(
                    sql_ranges(&compiled, source),
                    oracle_ranges(&compiled, source),
                    "left={left} right={right}"
                );
            }
        }
    }
}

#[test]
fn sql_composition_preserves_input_environments() {
    let cases = [
        code_rule(json!({
            "all": [
                { "pattern": "$A + $B" },
                { "any": [
                    { "pattern": "$X + $A" },
                    { "pattern": "$A + $Y" }
                ] }
            ]
        })),
        code_rule(json!({
            "all": [
                { "pattern": "$A + $B" },
                { "not": { "pattern": "$X + $A" } }
            ]
        })),
    ];
    let outcomes = cases
        .iter()
        .map(|compiled| {
            let source = "x + y\n";
            (
                sql_ranges(compiled, source),
                oracle_ranges(compiled, source),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        outcomes.iter().map(|pair| &pair.0).collect::<Vec<_>>(),
        outcomes.iter().map(|pair| &pair.1).collect::<Vec<_>>()
    );
}

#[test]
fn compiled_count_applies_exact_bounds_to_the_complete_axis() {
    let compiled = code_rule(json!({
        "count": {
            "has": { "kind": "if_statement", "stopBy": "end" },
            "min": 2,
            "max": 2
        }
    }));
    let plan = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
        .expect("a bounded descendant count lowers into SQL");

    assert_eq!(
        execute_navigation_sql(
            &plan,
            &[(0, &[0, 10]), (2, &[2, 3, 12])],
            &[
                (0, 1, 0, 0, true),
                (1, 2, 0, 0, true),
                (1, 3, 1, 0, true),
                (10, 11, 0, 0, true),
                (11, 12, 0, 0, true)
            ]
        ),
        vec![0]
    );
    assert!(
        execute_boolean_sql(
            &plan,
            true,
            &[
                (0, true, Some(true), &[0]),
                (1, false, None, &[]),
                (2, true, Some(true), &[2, 3]),
            ]
        )
        .is_empty(),
        "an incomplete axis cannot prove a count bound"
    );
}

#[test]
fn compiled_count_uses_each_axis_direction_and_inclusive_stop_order() {
    for (relation, source, related, edges) in [
        (
            "inside",
            3,
            vec![1, 2],
            vec![(1, 2, 0, 0, true), (2, 3, 0, 0, true)],
        ),
        (
            "precedes",
            10,
            vec![11, 12],
            vec![
                (9, 10, 0, 0, true),
                (9, 11, 1, 0, true),
                (9, 12, 2, 0, true),
            ],
        ),
        (
            "follows",
            12,
            vec![10, 11],
            vec![
                (9, 10, 0, 0, true),
                (9, 11, 1, 0, true),
                (9, 12, 2, 0, true),
            ],
        ),
    ] {
        let compiled = code_rule(json!({
            "count": {
                (relation): { "kind": "identifier", "stopBy": "end" },
                "min": 2
            }
        }));
        let plan = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
            .expect("axis count lowers into SQL");
        assert_eq!(
            execute_navigation_sql(&plan, &[(0, &[source]), (2, &related)], &edges),
            vec![source],
            "{relation}"
        );
    }

    let compiled = code_rule(json!({
        "count": {
            "has": {
                "kind": "return_statement",
                "stopBy": { "kind": "function_definition" }
            },
            "max": 0
        }
    }));
    let plan = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
        .expect("completion-aware stopped count lowers into SQL");
    assert!(
        execute_navigation_sql(
            &plan,
            &[(0, &[0]), (2, &[4]), (3, &[1])],
            &[(0, 1, 0, 0, true), (1, 2, 0, 0, true), (0, 4, 1, 0, true)]
        )
        .is_empty(),
        "the stop at node 1 does not hide a match in its sibling branch"
    );
}

#[test]
fn unsupported_inventory_reports_every_operation_in_source_order() {
    let compiled = code_rule(json!({
        "kind": "integer",
        "value": { "constant": true }
    }));
    let error = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
        .expect_err("the compiler reports unsupported relational operators");

    assert_eq!(
        error.unsupported(),
        &["lowering/ast-match/python/rule/all/1: value predicate has no SQL comparison"]
    );
}
