use super::{ExecutionStatement, SqlViolationRow, drive, split_measured_statement_packets};
use crate::code_rule_dsl::demand::vtab::{MatchRegistry, PackedDemand, RowKind, register_module};
use crate::code_rule_dsl::prefilter::tests::{open, temp};
use crate::code_rule_dsl::transducer::packed::ExecutionBackend;
use std::sync::Arc;

/// N's `legacy_test_scan_sql` (`query/plan/render/workspace.rs`): every row of a batch.
const SCAN_SQL: &str = "SELECT batch, row_kind, producer_id, file_index, task_index, match_index, capture_index, capture_occurrence, capture_sequence_start, capture_sequence_count, node, related_node, axis_kind, axis_order, fact_kind, document_id, mapped_document_id, start_ordinal, output_label, capture_label, text_value, start_byte, end_byte, verdict, complete, work_used, error_code, field_id, related_named, source_text, structural_key, named_leaf, capture_kind, capture_named, equality_occurrence FROM rerail_matches WHERE batch = ?1 ORDER BY rowid";

#[test]
fn packed_sql_backend_registers_narrow_typed_relations() {
    let path = temp();
    drop(open(&path));
    let connection = rusqlite::Connection::open(path).expect("content store");
    register_module(&connection, Arc::new(MatchRegistry::default())).expect("relation modules");

    // Estimates come from `fact_stats`; no seeded statistics table.
    let seeded = connection
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name = 'rerail_relation_statistics'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("schema lookup");
    assert_eq!(seeded, 0);

    for (relation, columns) in [
        ("rerail_batch_status", 3),
        ("rerail_producer_status", 7),
        ("rerail_terminal_match", 13),
        ("rerail_capture", 23),
        ("rerail_document_map", 9),
        // Removed with the CPU fact post-pass and the navigation producer.
        ("rerail_navigation", 0),
        ("rerail_semantic_fact", 0),
        ("rerail_value_fact", 0),
        ("rerail_parse_fact", 0),
        ("rerail_transform_fact", 0),
    ] {
        let count = connection
            .query_row(
                "SELECT count(*) FROM pragma_table_xinfo(?1)",
                [relation],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or_else(|error| panic!("{relation} schema: {error}"));
        assert_eq!(count, columns, "{relation} has a narrow typed schema");
    }

    let legacy_fact_columns = connection
        .query_row(
            "SELECT count(*) FROM pragma_table_xinfo('rerail_fact')",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("legacy fact schema lookup");
    assert_eq!(legacy_fact_columns, 0);

    let plan = connection
        .prepare(
            "EXPLAIN QUERY PLAN SELECT node FROM rerail_terminal_match \
             WHERE batch = 1 AND producer_id = 2 AND file_index = 3 \
             AND node >= 4 AND node <= 9",
        )
        .expect("pushed-down relation plan")
        .query_map([], |row| row.get::<_, String>(3))
        .expect("query plan rows")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("query plan");
    assert!(
        plan.iter().any(|detail| detail.contains("INDEX 31:")),
        "producer, file, and node-range constraints are pushed down: {plan:?}"
    );
    let in_plan = connection
        .prepare(
            "EXPLAIN QUERY PLAN SELECT node FROM rerail_terminal_match \
             WHERE batch = 1 AND producer_id IN (2, 5) AND file_index = 3 \
             AND node BETWEEN 4 AND 9",
        )
        .expect("batched producer plan")
        .query_map([], |row| row.get::<_, String>(3))
        .expect("batched query plan rows")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("batched query plan");
    assert!(
        in_plan.iter().any(|detail| detail.contains("INDEX 61:")),
        "producer IN is passed to sqlite3_vtab_in: {in_plan:?}"
    );
    let ordered_plan = connection
        .prepare(
            "EXPLAIN QUERY PLAN SELECT file_index, producer_id, task_index, match_index, node \
             FROM rerail_terminal_match WHERE batch = 1 \
             ORDER BY file_index, producer_id, task_index, match_index, node",
        )
        .expect("declared relation ordering")
        .query_map([], |row| row.get::<_, String>(3))
        .expect("ordered query plan rows")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("ordered query plan");
    assert!(
        ordered_plan
            .iter()
            .all(|detail| !detail.contains("USE TEMP B-TREE")),
        "only guaranteed native ordering is declared: {ordered_plan:?}"
    );
}

#[test]
fn measured_statement_packets_split_deterministically_at_the_resource_boundary() {
    let mut rendered = Vec::new();
    let packets = split_measured_statement_packets(&[0, 1, 2, 3], 10, |ordinals| {
        rendered.push(ordinals.to_vec());
        Ok("xxxx".repeat(ordinals.len()))
    })
    .expect("bounded packets");

    assert_eq!(
        packets
            .iter()
            .map(|(ordinals, _)| ordinals.clone())
            .collect::<Vec<_>>(),
        vec![vec![0, 1], vec![2, 3]]
    );
    assert_eq!(rendered, vec![vec![0, 1, 2, 3], vec![0, 1], vec![2, 3]]);
}

#[test]
fn typed_sql_violation_rows_reject_malformed_nested_spans() {
    let connection = rusqlite::Connection::open_in_memory().expect("SQLite");
    let error = connection
        .query_row(
            "SELECT 'rule' AS rule_id, 'clause' AS clause_id, 'CODE' AS code, 'message' AS message, 'test.py' AS path, '{\"endByte\":1,\"endColumn\":1,\"endLine\":0,\"startByte\":0,\"startColumn\":0,\"startLine\":0}' AS span, 'log' AS mode, 'order' AS stable_order, '{\"A\":[{\"endByte\":1,\"endColumn\":1,\"endLine\":0,\"startByte\":0,\"startColumn\":0,\"startLine\":0,\"unexpected\":true}]}' AS captures, '{}' AS variables, NULL AS fix, 0 AS witness_start_byte, 1 AS witness_end_byte, NULL AS each_start_byte, NULL AS each_end_byte",
            [],
            SqlViolationRow::from_sql_row,
        )
        .expect_err("unknown nested span field must fail closed");
    assert!(matches!(
        error,
        rusqlite::Error::FromSqlConversionFailure(..)
    ));

    let missing = connection
        .query_row(
            "SELECT 'rule' AS rule_id, 'clause' AS clause_id, 'CODE' AS code, 'message' AS message, 'test.py' AS path, '{\"endByte\":1,\"endColumn\":1,\"endLine\":0,\"startByte\":0,\"startColumn\":0,\"startLine\":0}' AS span, 'log' AS mode, '{}' AS captures, '{}' AS variables, NULL AS fix, 0 AS witness_start_byte, 1 AS witness_end_byte, NULL AS each_start_byte, NULL AS each_end_byte",
            [],
            SqlViolationRow::from_sql_row,
        )
        .expect_err("missing stable order must fail closed");
    assert!(matches!(missing, rusqlite::Error::InvalidColumnName(_)));

    let inconsistent_witness = connection
        .query_row(
            "SELECT 'rule' AS rule_id, 'clause' AS clause_id, 'CODE' AS code, 'message' AS message, 'test.py' AS path, '{\"endByte\":1,\"endColumn\":1,\"endLine\":0,\"startByte\":0,\"startColumn\":0,\"startLine\":0}' AS span, 'log' AS mode, 'order' AS stable_order, '{}' AS captures, '{}' AS variables, NULL AS fix, 2 AS witness_start_byte, 1 AS witness_end_byte, NULL AS each_start_byte, NULL AS each_end_byte",
            [],
            SqlViolationRow::from_sql_row,
        )
        .expect_err("an inverted witness range must fail closed");
    assert!(matches!(
        inconsistent_witness,
        rusqlite::Error::InvalidQuery
    ));
}

#[test]
fn typed_sql_violation_publication_keeps_the_complete_projected_value() {
    let connection = rusqlite::Connection::open_in_memory().expect("SQLite");
    let row = connection
        .query_row(
            "SELECT 'rule' AS rule_id, 'clause' AS clause_id, 'CODE' AS code, 'projected' AS message, 'test.py' AS path, '{\"endByte\":1,\"endColumn\":1,\"endLine\":0,\"startByte\":0,\"startColumn\":0,\"startLine\":0}' AS span, 'steer' AS mode, 'order' AS stable_order, '{}' AS captures, '{\"COUNT\":true}' AS variables, NULL AS fix, 0 AS witness_start_byte, 1 AS witness_end_byte, NULL AS each_start_byte, NULL AS each_end_byte",
            [],
            SqlViolationRow::from_sql_row,
        )
        .expect("typed SQL violation");
    // The published value carries the finding key (`id`); the witness keeps what W's output
    // needs.
    let value = row.to_value();
    assert!(value["id"].as_str().is_some_and(|id| id.starts_with("sha256:")), "{value}");
    assert_eq!(value["variables"], serde_json::json!({"COUNT": true}));
    let witness = row.into_witness();
    assert_eq!((witness.path.as_str(), witness.message.as_deref()), ("test.py", Some("projected")));
}

/// On the content store, twice: the second drive registers the modules again on the same
/// connection.
#[test]
fn sqlite_demands_the_complete_packed_batch_once() {
    let store = open(&temp());
    for _ in 0..2 {
        let demand = PackedDemand::new(
            ExecutionBackend::CpuJit,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .expect("verified empty packed batch");
        let statement = ExecutionStatement {
            sql: SCAN_SQL.to_owned(),
            clauses: "[]".to_owned(),
            materialization: None,
        };
        let outcome = drive(&store, demand, &[statement], "[]").expect("SQL-demanded packed batch");
        let counters = outcome.counters;
        assert_eq!(outcome.rows.len(), 1);
        assert_eq!(outcome.rows[0].row_kind(), RowKind::BatchStatus);
        assert!(outcome.rows[0].is_complete());
        assert_eq!(outcome.rows[0].error_code(), None);
        assert_eq!(counters.rows, 1);
        assert_eq!(counters.statements, 1);
        assert_eq!(counters.registrations, 1);
        assert_eq!(counters.backend_invocations, 1);
        assert_eq!(counters.terminal_executions, 1);
        assert_eq!(counters.virtual_table_filters, 1);
    }
}
