use super::{SqlViolationRow, drive, regex_matches, split_measured_statement_packets};
use crate::code_rule_dsl::budget::{MatchBudget, set_shortcuts_off};
use crate::code_rule_dsl::packed::ExecutionBackend;
use crate::code_rule_dsl::test_support::{CAPABILITIES, evaluate_file};
use crate::code_rule_dsl::transducer::sql_backend::{
    MatchRegistry, PackedDemand, RowKind, register_module,
};
use crate::code_rule_dsl::{CodeRuleDslRuntime, CompiledCodeRuleDsl};
use regex::Regex;
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

#[test]
fn packed_sql_backend_registers_narrow_typed_relations() {
    let connection = rusqlite::Connection::open_in_memory().expect("SQLite");
    register_module(&connection, Arc::new(MatchRegistry::default())).expect("relation modules");

    let statistics = connection
        .query_row(
            "SELECT count(*) FROM rerail_relation_statistics WHERE estimated_rows > 0",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("stored SQL planning statistics");
    assert_eq!(statistics, 10);

    for (relation, columns) in [
        ("rerail_batch_status", 3),
        ("rerail_producer_status", 7),
        ("rerail_terminal_match", 13),
        ("rerail_capture", 23),
        ("rerail_navigation", 17),
        ("rerail_semantic_fact", 34),
        ("rerail_value_fact", 9),
        ("rerail_parse_fact", 16),
        ("rerail_transform_fact", 12),
        ("rerail_document_map", 9),
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
    let expected = row.to_value();
    let witness = row.into_witness();
    assert_eq!(witness.sql_violation, Some(expected));
}

#[test]
fn sqlite_demands_the_complete_packed_batch_once() {
    let demand = PackedDemand::new(
        ExecutionBackend::CpuJit,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    )
    .expect("verified empty packed batch");
    let outcome = drive(demand).expect("SQL-demanded packed batch");
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
    assert_eq!(counters.scalar_callbacks, 0);
    assert_eq!(counters.node_rows, 0);
}

/// Matches keep their byte ranges past multibyte text; a match past the limit leaves the scan
/// unfinished.
#[test]
fn regex_matches_keep_byte_ranges_and_stop_past_the_limit() {
    let regex = Regex::new("é").unwrap();
    let (first, finished) = regex_matches(&regex, "éxé.ts", 1);
    assert_eq!((first.len(), first[0].clone(), finished), (1, 0..2, false));
    let all = regex_matches(&regex, "éxé.ts", 2);
    assert_eq!((all.0.as_slice(), all.1), (&[0..2, 3..5][..], true));
}

/// An exhausted match deadline leaves the scan unfinished before it keeps any match.
#[test]
fn regex_matches_stop_at_the_deadline() {
    let _budget = crate::code_rule_dsl::budget::MatchBudget::expired_for_test();
    let regex = Regex::new("needle").unwrap();
    assert_eq!(regex_matches(&regex, "needle\n", 2), (Vec::new(), false));
}

/// A Python pack code rule (a path under `packs/python/`: `rules/` or `candidates/rules/`), as
/// the conformance runner registers it.
fn python_pack_code_rule(rule: &str) -> CompiledCodeRuleDsl {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../packs/python")
        .join(rule);
    CodeRuleDslRuntime::compile(
        std::fs::read_to_string(&path).expect("pack policy"),
        "json".to_owned(),
        CAPABILITIES.to_owned(),
    )
    .unwrap_or_else(|error| panic!("{}: {}", path.display(), error.reason))
}

/// A code rule's result on one file, compiled and evaluated with the work-skipping shortcuts on and
/// off (`set_shortcuts_off`): they must agree, violations and failures alike.
fn agreeing(compiled: &CompiledCodeRuleDsl, path: &str, source: &str) -> Value {
    set_shortcuts_off(true);
    let without = evaluate_file(compiled, "python", path, source);
    set_shortcuts_off(false);
    let with = evaluate_file(compiled, "python", path, source);
    assert_eq!(with, without, "{path}: the shortcuts changed the result");
    with
}

/// The shortcuts that only skip work (the anchor-set shortcuts) change no result of F811, F821 or
/// F841: on every Ruff fixture, and on a module that mixes unpacking, assignment chains, walruses,
/// `with` targets, redefinitions and dotted imports.
#[test]
fn python_shortcuts_change_no_pack_result() {
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../packs/python/upstream/ruff/fixtures");
    let mut files = Vec::new();
    let mut pending = vec![fixtures];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory).expect("fixtures") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                pending.push(path);
            } else if path
                .extension()
                .is_some_and(|extension| extension == "py" || extension == "pyi")
            {
                files.push(path);
            }
        }
    }
    files.sort();
    assert!(files.len() > 50, "{} fixtures", files.len());
    let mixed = "\
import os.path
import os
from typing import overload
import json as json
def f(items):
    a, b = c = (1, 2)
    d = e = f = [3, 4]
    (g, h) = i = j = items
    k = (l := 5)
    m, n = o = p = 6, 7
    with open('x') as (q, r), open('y') as s:
        pass
    t = 1
    t = 2
    u: int = 3
    v += 1
    for w in items:
        pass
    import json
    def os():
        pass
    return [x for x in items if (y := x)]
class C:
    import sys
    z = sys
";
    for rule in [
        "candidates/rules/ruff-f811-redefined-while-unused.json",
        "candidates/rules/ruff-f821-undefined-name.json",
        "candidates/rules/ruff-f841-unused-variable.json",
    ] {
        let compiled = python_pack_code_rule(rule);
        for file in &files {
            let source = std::fs::read_to_string(file).expect("fixture");
            let path = if file.extension().is_some_and(|extension| extension == "pyi") {
                "fixture.pyi"
            } else {
                "fixture.py"
            };
            agreeing(&compiled, path, &source);
        }
        let result = agreeing(&compiled, "mixed.py", mixed);
        assert_eq!(result["failures"], json!([]), "{rule}: {result}");
    }
}

/// F841's work grows linearly with a function's assignments, each an unpacked chain whose
/// identifiers the capture-free utilities are asked about: doubling them at most about doubles
/// the budget units spent.
#[test]
fn f841_work_grows_linearly_with_assignments() {
    let code_rule = python_pack_code_rule("candidates/rules/ruff-f841-unused-variable.json");
    let mut units = Vec::new();
    for assignments in [500, 1000, 2000] {
        let mut source = "def f(items):\n".to_owned();
        for index in 0..assignments {
            source.push_str(&format!(
                "    a{index}, b{index} = c{index} = d{index} = (items, {index}, (e{index}, items))\n"
            ));
        }
        source.push_str("    return a0, b0, c0, d0\n");
        let budget = MatchBudget::work(&Arc::new(AtomicBool::new(false)), u64::MAX);
        let result = evaluate_file(&code_rule, "python", "chains.py", &source);
        let spent = budget.spent();
        drop(budget);
        assert_eq!(result["failures"], json!([]), "{result}");
        // `c` and `d` of every assignment but the first are unused; `a` and `b` are unpacked.
        assert_eq!(
            result["violations"].as_array().map(Vec::len),
            Some(2 * (assignments - 1)),
            "{assignments} assignments"
        );
        units.push(spent);
    }
    assert!(units[0] > 0, "the evaluation polled its budget: {units:?}");
    for pair in units.windows(2) {
        assert!(
            pair[1] * 4 <= pair[0] * 9,
            "units {units:?}: doubling the assignments more than doubled the work"
        );
    }
}
