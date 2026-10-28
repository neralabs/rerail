use super::relational::{SqlCatalogPlan, SqlShape, TerminalKind, TerminalPlan};
use super::relational_ir::qualified_row_columns;
use super::{PackedFileInput, SqlPlans};
use crate::code_rule_dsl::CodeRuleDslRuntime;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::lines::{LineTable, Terminators};
use crate::code_rule_dsl::model::Anchor;
use crate::code_rule_dsl::packed::ExecutionBackend;
use crate::code_rule_dsl::tape::{Grammar, TapeDoc, TapeImage};
use crate::code_rule_dsl::test_support::{CAPABILITIES, evaluate_file};
use crate::code_rule_dsl::transducer::sql_backend::{
    DocumentMapSpan, MatchRegistry, PackedDemand, ProducerKind, register_module,
};
use rusqlite::{Connection, params};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;

fn file_metadata(files: &[(String, String)], dialect: GrammarDialect) -> String {
    serde_json::Value::Array(
        files
            .iter()
            .map(|(path, source)| {
                serde_json::json!({
                    "path": path,
                    "source": source,
                    "lineStarts": LineTable::new(source, Terminators::of(dialect)).starts(),
                })
            })
            .collect(),
    )
    .to_string()
}

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
    compile_code_rule_with_capabilities(ast, message, CAPABILITIES.to_owned())
}

fn compile_code_rule_with_capabilities(
    ast: serde_json::Value,
    message: &str,
    capabilities: String,
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
        capabilities,
    )
    .expect("valid test policy")
}

#[test]
fn abstract_kind_terminals_execute_from_tape_class_memberships() {
    let compiled = code_rule(json!({ "kind": "expression" }));
    assert_eq!(sql_ranges(&compiled, "x = 1\n"), [(4, 5)]);
}

#[test]
fn recursive_utility_components_lower_when_reached_through_an_outer_utility() {
    let compiled = code_rule_ast(json!({
        "language": "python",
        "utils": {
            "recursive": {
                "any": [
                    { "kind": "identifier" },
                    { "has": { "matches": "recursive", "stopBy": "end" } }
                ]
            },
            "outer": {
                "all": [
                    { "kind": "module" },
                    { "has": { "matches": "recursive", "stopBy": "end" } }
                ]
            }
        },
        "rule": { "matches": "outer" }
    }));
    let plan = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
        .expect("a nested recursive component has bounded relational lowering");

    assert!(plan.structural_sql().contains("WITH RECURSIVE"));
    assert!(plan.structural_sql().contains("utility_recursive_"));
}

#[test]
fn parameterized_recursive_utility_identity_frames_lower_once() {
    let compiled = code_rule_ast(json!({
        "language": "python",
        "utils": {
            "recursive": {
                "params": ["N"],
                "rule": {
                    "any": [
                        { "kind": "identifier", "pattern": "$N" },
                        {
                            "all": [
                                { "kind": "attribute" },
                                {
                                    "has": {
                                        "field": "object",
                                        "matches": { "recursive": { "N": "$N" } }
                                    }
                                }
                            ]
                        }
                    ]
                }
            }
        },
        "rule": { "matches": { "recursive": { "N": "$FOUND" } } }
    }));
    let plan = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
        .expect("an identity-framed recursive component has one fixed point");
    let sql = plan.structural_sql();

    assert!(sql.contains("WITH RECURSIVE"));
    assert_eq!(
        sql.lines()
            .filter(|line| line.contains("_fix_") && line.contains(" AS ("))
            .count(),
        1,
        "one parameterized recursive fixed-point definition"
    );
    assert!(sql.contains("@caller:"));

    let source = "root.leaf\n";
    assert_eq!(
        sql_ranges(&compiled, source),
        oracle_ranges(&compiled, source),
        "the recursive frame returns its parameter binding to the caller"
    );
}

#[test]
fn mutually_recursive_utility_frames_relabel_component_parameters() {
    let compiled = code_rule_ast(json!({
        "language": "python",
        "utils": {
            "left": {
                "params": ["LEFT"],
                "rule": {
                    "any": [
                        { "kind": "identifier", "pattern": "$LEFT" },
                        {
                            "all": [
                                { "kind": "attribute" },
                                {
                                    "has": {
                                        "field": "object",
                                        "matches": { "right": { "RIGHT": "$LEFT" } }
                                    }
                                }
                            ]
                        }
                    ]
                }
            },
            "right": {
                "params": ["RIGHT"],
                "rule": {
                    "all": [
                        { "kind": "parenthesized_expression" },
                        {
                            "has": {
                                "matches": { "left": { "LEFT": "$RIGHT" } }
                            }
                        }
                    ]
                }
            }
        },
        "rule": { "matches": { "left": { "LEFT": "$FOUND" } } }
    }));
    let plan = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
        .expect("mutually recursive frames relabel component parameters");

    assert!(plan.structural_sql().contains("WITH RECURSIVE"));
    let source = "(root).leaf\n";
    assert_eq!(
        sql_ranges(&compiled, source),
        oracle_ranges(&compiled, source)
    );
}

#[test]
fn recursive_utility_paths_keep_nested_navigation_guards_in_sql() {
    let compiled = code_rule_ast(json!({
        "language": "python",
        "utils": {
            "recursive": {
                "any": [
                    { "kind": "identifier" },
                    {
                        "all": [
                            { "kind": "attribute" },
                            {
                                "has": {
                                    "field": "object",
                                    "all": [
                                        { "kind": "attribute" },
                                        { "has": { "matches": "recursive" } }
                                    ]
                                }
                            }
                        ]
                    }
                ]
            }
        },
        "rule": { "matches": "recursive" }
    }));
    let plan = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
        .expect("nested recursive paths lower through guarded navigation steps");
    let sql = plan.structural_sql();

    assert!(sql.contains("WITH RECURSIVE"));
    assert!(sql.contains("field_id ="));
    assert!(sql.contains("guard0"));
    assert_eq!(
        sql.lines()
            .filter(|line| line.contains("_fix_") && line.contains(" AS ("))
            .count(),
        1,
        "one recursive fixed-point definition"
    );
}

#[test]
fn recursive_utility_paths_lower_rule_stops() {
    let compiled = code_rule_ast(json!({
        "language": "python",
        "utils": {
            "recursive": {
                "any": [
                    { "kind": "identifier" },
                    {
                        "has": {
                            "stopBy": { "kind": "function_definition" },
                            "matches": "recursive"
                        }
                    }
                ]
            }
        },
        "rule": { "matches": "recursive" }
    }));
    let plan = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
        .expect("recursive paths lower rule stops into the fixed point");

    assert!(plan.structural_sql().contains("WITH RECURSIVE"));
    for source in [
        "outer(inner)\n",
        "def boundary():\n    return outer(inner)\n",
    ] {
        assert_eq!(
            sql_ranges(&compiled, source),
            oracle_ranges(&compiled, source)
        );
    }
}

#[test]
fn universal_recursive_utility_paths_lower_as_failure_fixed_points() {
    let compiled = code_rule_ast(json!({
        "language": "python",
        "utils": {
            "recursive": {
                "any": [
                    { "kind": "identifier" },
                    {
                        "all": [
                            { "kind": "list" },
                            {
                                "not": {
                                    "has": {
                                        "all": [
                                            { "nthChild": "n+1" },
                                            { "not": { "matches": "recursive" } }
                                        ]
                                    }
                                }
                            }
                        ]
                    }
                ]
            }
        },
        "rule": { "matches": "recursive" }
    }));
    let plan = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
        .expect("universal recursion lowers as a bounded failure fixed point");
    let sql = plan.structural_sql();

    assert!(sql.contains("failed_match.utility_component"));
    assert!(sql.contains("NOT EXISTS"));
}

#[test]
fn mutually_recursive_universal_branches_share_one_failure_fixed_point() {
    let compiled = code_rule_ast(json!({
        "language": "python",
        "utils": {
            "item": {
                "any": [
                    { "kind": "identifier" },
                    { "matches": "container" }
                ]
            },
            "container": {
                "all": [
                    { "kind": "list" },
                    {
                        "not": {
                            "has": {
                                "all": [
                                    { "nthChild": "n+1" },
                                    { "not": { "matches": "item" } }
                                ]
                            }
                        }
                    },
                    {
                        "not": {
                            "has": {
                                "all": [
                                    { "kind": "dictionary_splat" },
                                    { "not": { "matches": "item" } }
                                ]
                            }
                        }
                    }
                ]
            }
        },
        "rule": { "matches": "item" }
    }));
    let plan = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
        .expect("mutual universal recursion lowers as one failure fixed point");
    let sql = plan.structural_sql();

    assert!(sql.contains("failed_match.utility_component"));
    assert_eq!(
        sql.lines()
            .filter(|line| line.contains("_fix_") && line.contains(" AS ("))
            .count(),
        1
    );
}

#[test]
fn many_universal_recursive_branches_do_not_expand_to_boolean_dnf() {
    let universal_branches = (0..12)
        .map(|_| {
            json!({
                "not": {
                    "has": {
                        "all": [
                            { "kind": "dictionary_splat" },
                            { "not": { "matches": "item" } }
                        ]
                    }
                }
            })
        })
        .collect::<Vec<_>>();
    let mut container = vec![json!({ "kind": "list" })];
    container.extend(universal_branches);
    let compiled = code_rule_ast(json!({
        "language": "python",
        "utils": {
            "item": {
                "any": [
                    { "kind": "identifier" },
                    { "matches": "container" }
                ]
            },
            "container": { "all": container }
        },
        "rule": { "matches": "item" }
    }));
    let plan = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
        .expect("universal branches remain a factored recursive fixed point");

    assert!(
        plan.structural_sql().len() < 64 * 1024,
        "factored recursive SQL is {} bytes",
        plan.structural_sql().len()
    );
}

#[test]
fn large_utility_catalogs_use_compact_stable_cte_names() {
    let utils = (0..80)
        .map(|ordinal| {
            (
                format!("component_{ordinal:03}"),
                json!({
                    "all": [
                        { "pattern": format!("call_{ordinal:03}($A)") },
                        { "pattern": format!("call_{ordinal:03}($B)") },
                        { "pattern": format!("call_{ordinal:03}($C)") }
                    ]
                }),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    let calls = (0..80)
        .flat_map(|ordinal| {
            let name = format!("component_{ordinal:03}");
            [json!({ "matches": name }), json!({ "matches": name })]
        })
        .collect::<Vec<_>>();
    let compiled = code_rule_ast(json!({
        "language": "python",
        "utils": utils,
        "rule": { "any": calls }
    }));
    let plan = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
        .expect("a large utility catalog has bounded SQL names");

    assert!(
        !plan
            .structural_sql()
            .contains("clause_0_utility_component_000_")
    );
    assert_eq!(
        plan.structural_sql().matches("FROM tm m JOIN").count(),
        1,
        "the utility catalog normalizes terminal rows once"
    );
    assert_eq!(
        plan.structural_sql()
            .matches(
                "tm AS MATERIALIZED(SELECT*FROM terminal_match UNION ALL SELECT*FROM tm WHERE 0)",
            )
            .count(),
        1,
        "a large utility catalog fences its one terminal base alias"
    );
    assert!(
        !plan.structural_sql().contains("m.batch"),
        "the normalized utility row omits dead packed columns"
    );
    assert_eq!(
        plan.structural_sql()
            .matches("cp AS MATERIALIZED(SELECT*FROM capture UNION ALL SELECT*FROM cp WHERE 0)")
            .count(),
        1,
        "wide utility catalogs fence one cached physical capture binding"
    );
    assert_eq!(
        plan.structural_sql()
            .matches("utility_component_000_")
            .count(),
        2,
        "the public utility name has one definition and one dead recursive fence"
    );
    assert!(!plan.structural_sql().contains("input_2"));
    assert!(
        plan.structural_sql().matches(" AS MATERIALIZED").count() >= 80,
        "wide utility plans need structural materialization boundaries"
    );
    let connection = Connection::open_in_memory().unwrap();
    register_module(&connection, Arc::new(MatchRegistry::default())).unwrap();
    connection
        .prepare(plan.structural_sql())
        .expect("a wide utility catalog prepares without CTE reference expansion");
}

#[test]
fn transformed_comparisons_are_batched_as_typed_facts() {
    let compiled = code_rule_ast(json!({
        "language": "python",
        "rule": {
            "all": [
                { "pattern": "pair($ASYNC, $BASE)" },
                { "compare": { "left": "NORMALIZED", "op": "eq", "right": "BASE" } }
            ]
        },
        "transform": {
            "WITHOUT_SUFFIX": {
                "replace": { "source": "$ASYNC", "replace": "_async$", "by": "" }
            },
            "NORMALIZED": {
                "replace": { "source": "$WITHOUT_SUFFIX", "replace": "^prefix_", "by": "" }
            }
        }
    }));
    let matched = sql_outcome(&compiled, "pair(prefix_foo_async, foo)\n");
    assert_eq!(matched.ranges.len(), 1);
    assert!(matched.facts.iter().any(|fact| fact.slot == 19));
    assert_eq!(matched.execution_counters.0, 1);
    assert!((1..=10).contains(&matched.execution_counters.1));
    assert!(
        sql_outcome(&compiled, "pair(prefix_foo_async, bar)\n")
            .ranges
            .is_empty()
    );
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
    assert!(!plan.structural_sql().contains("tq_"));
    assert!(!plan.structural_sql().contains("rerail_nodes"));
    assert!(!plan.structural_sql().contains("rerail_facts"));
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
fn expanded_clause_navigation_uses_typed_materialization_packets() {
    let mut rule = json!({ "kind": "identifier" });
    for _ in 0..12 {
        rule = json!({ "has": rule });
    }
    let compiled = code_rule(rule);
    let plan = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
        .expect("expanded navigation lowers");
    let execution = plan
        .execution_sql_for_clauses(&[0])
        .expect("expanded navigation packetizes");

    assert!(!execution.materializations.is_empty());
    assert!(execution.query.contains("rerail_packet_0_"));
    for (table, sql) in execution.materializations {
        assert!(sql.starts_with(&format!("CREATE TEMP TABLE \"{table}\" AS WITH RECURSIVE")));
        assert!(sql.contains("rerail_navigation"));
    }
}

#[test]
fn exact_scope_statements_render_only_their_selected_clauses() {
    let first = code_rule(json!({ "kind": "identifier" }));
    let second = code_rule(json!({ "kind": "string" }));
    let plan = SqlCatalogPlan::compile(&[Arc::clone(&first.inner), Arc::clone(&second.inner)])
        .expect("two clauses lower");

    let first_sql = plan
        .execution_sql_for_clauses(&[0])
        .expect("first exact-scope statement");
    let second_sql = plan
        .execution_sql_for_clauses(&[1])
        .expect("second exact-scope statement");

    assert!(first_sql.query.contains("clause_0_output"));
    assert!(!first_sql.query.contains("clause_1"));
    assert!(second_sql.query.contains("clause_1_output"));
    assert!(!second_sql.query.contains("clause_0"));
    assert!(!first_sql.query.contains("rerail_matches"));
    assert!(!second_sql.query.contains("rerail_matches"));
    assert_eq!(first_sql.query.matches("rerail_terminal_match").count(), 1);
    assert_eq!(second_sql.query.matches("rerail_terminal_match").count(), 1);
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

fn create_typed_relation_views(connection: &Connection) {
    connection
        .execute_batch(
            "CREATE VIEW rerail_batch_status AS
               SELECT batch, complete, error_code FROM rerail_matches WHERE row_kind = 1;
             CREATE VIEW rerail_producer_status AS
               SELECT batch, producer_id, file_index, verdict, complete, work_used, error_code FROM rerail_matches WHERE row_kind = 2;
             CREATE VIEW rerail_terminal_match AS
               SELECT batch, producer_id, file_index, task_index, match_index, node, document_id, mapped_document_id, start_ordinal, output_label, start_byte, end_byte, verdict FROM rerail_matches WHERE row_kind = 3;
             CREATE VIEW rerail_capture AS
               SELECT batch, producer_id, file_index, task_index, match_index, capture_index, capture_occurrence, capture_sequence_start, capture_sequence_count, node, output_label, capture_label, text_value, start_byte, end_byte, verdict, field_id, source_text, structural_key, named_leaf, capture_kind, capture_named, equality_occurrence FROM rerail_matches WHERE row_kind = 4;
             CREATE VIEW rerail_navigation AS
               SELECT batch, producer_id, file_index, task_index, match_index, node, related_node, axis_kind, axis_order, start_byte, end_byte, verdict, complete, work_used, error_code, field_id, related_named FROM rerail_matches WHERE row_kind = 5;
             CREATE VIEW rerail_fact AS
               SELECT batch, producer_id, file_index, task_index, match_index, capture_index, capture_occurrence, capture_sequence_start, capture_sequence_count, node, related_node, axis_kind, axis_order, fact_kind, document_id, mapped_document_id, start_ordinal, output_label, capture_label, text_value, start_byte, end_byte, verdict, complete, work_used, error_code, field_id, related_named, source_text, structural_key, named_leaf, capture_kind, capture_named, equality_occurrence FROM rerail_matches WHERE row_kind = 6;
             CREATE VIEW rerail_semantic_fact AS SELECT * FROM rerail_fact WHERE fact_kind NOT IN (1, 18, 19) OR fact_kind IS NULL;
             CREATE VIEW rerail_value_fact AS SELECT * FROM rerail_fact WHERE fact_kind = 1;
             CREATE VIEW rerail_parse_fact AS SELECT * FROM rerail_fact WHERE fact_kind = 18;
             CREATE VIEW rerail_transform_fact AS SELECT * FROM rerail_fact WHERE fact_kind = 19;
             CREATE VIEW rerail_document_map AS
               SELECT batch, file_index, match_index, node, related_node, document_id, mapped_document_id, start_byte, end_byte FROM rerail_matches WHERE row_kind = 7;",
        )
        .expect("ordinary typed relation views");
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
    create_typed_relation_views(&connection);
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
    let mut statement = connection
        .prepare(plan.structural_sql())
        .expect("executable SQL plan");
    statement
        .query_map([1_i64], |row| {
            Ok((
                row.get::<_, i64>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<i64>>(10)?,
            ))
        })
        .expect("query relational rows")
        .filter_map(|row| match row {
            Ok((8, _, _)) => None,
            Ok((_, Some(file), Some(node))) => Some(Ok((file, node))),
            Ok(_) => Some(Err(rusqlite::Error::InvalidQuery)),
            Err(error) => Some(Err(error)),
        })
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
        cte(plan.structural_sql(), "clause_0_relation_2 AS"),
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
        cte(plan.structural_sql(), "clause_0_relation_2_branches AS"),
        format!(
            "clause_0_relation_2_branches AS ( SELECT {columns}, branch.bindings, printf('%08d/', 0) || branch.choice_order AS choice_order, 0 AS branch_order, 0 AS unknown_blocks FROM clause_0_term_0 branch UNION ALL SELECT {columns}, branch.bindings, printf('%08d/', 1) || branch.choice_order AS choice_order, 1 AS branch_order, 0 AS unknown_blocks FROM clause_0_term_1 branch )"
        )
    );
    let chosen = qualified_row_columns("chosen");
    assert_eq!(
        cte(plan.structural_sql(), "clause_0_relation_2 AS"),
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
fn utility_negation_summarizes_exclusion_truth_with_one_candidate_scan() {
    let compiled = code_rule_ast(json!({
        "language": "python",
        "utils": {
            "capturing": { "pattern": "call($ARG)" },
            "negative": { "not": { "matches": "capturing" } }
        },
        "rule": { "any": [{ "matches": "negative" }, { "matches": "negative" }] }
    }));
    let plan = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
        .expect("utility negation lowers into SQL");
    let sql = plan.structural_sql();

    assert!(sql.contains("has_true"));
    assert!(sql.contains("has_unknown"));
}

#[test]
fn compiled_capture_free_not_uses_completion_guarded_relational_antijoin() {
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
    let compatible = "1 = 1";
    let verdict = format!(
        "CASE WHEN EXISTS (SELECT 1 FROM clause_0_term_1 excluded WHERE excluded.file_index = universe.file_index AND excluded.node = universe.node AND excluded.verdict IS NULL AND {compatible}) THEN NULL ELSE 1 END"
    );
    let columns = qualified_row_columns("universe")
        .replace("universe.verdict", &format!("{verdict} AS verdict"));
    assert_eq!(
        cte(plan.structural_sql(), "clause_0_relation_2 AS"),
        format!(
            "clause_0_relation_2 AS ( SELECT {columns}, universe.bindings, universe.choice_order FROM clause_0_term_0 universe WHERE universe.verdict = 1 AND NOT EXISTS ( SELECT 1 FROM clause_0_term_1 excluded WHERE excluded.file_index = universe.file_index AND excluded.node = universe.node AND excluded.verdict = 1 AND {compatible} ) )"
        )
    );
    assert!(plan.structural_sql().contains("batch.complete = 1"));
    assert!(!cte(plan.structural_sql(), "clause_0_relation_2 AS").contains("json_each"));
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
    create_typed_relation_views(&connection);
    let mut statement = connection
        .prepare(plan.structural_sql())
        .expect("executable SQL plan");
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
fn utility_neighbor_navigation_executes_in_one_physical_region() {
    for (relation, sources, targets, edges, expected) in [
        ("has", vec![0], vec![1], vec![(0, 1, 0, 0, true)], vec![0]),
        (
            "inside",
            vec![1],
            vec![0],
            vec![(0, 1, 0, 0, true)],
            vec![1],
        ),
        (
            "precedes",
            vec![1],
            vec![2],
            vec![(0, 1, 0, 0, true), (0, 2, 1, 0, true)],
            vec![1],
        ),
        (
            "follows",
            vec![2],
            vec![1],
            vec![(0, 1, 0, 0, true), (0, 2, 1, 0, true)],
            vec![2],
        ),
    ] {
        let compiled = code_rule_ast(json!({
            "language": "python",
            "utils": {
                "direct_neighbor": { (relation): { "kind": "identifier" } }
            },
            "rule": {
                "any": [
                    { "matches": "direct_neighbor" },
                    { "matches": "direct_neighbor" }
                ]
            }
        }));
        let plan = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
            .expect("utility neighbor navigation lowers into SQL");
        let universe = plan
            .terminals()
            .iter()
            .position(|terminal| terminal.kind() == TerminalKind::Universe)
            .unwrap();
        let identifier = plan
            .terminals()
            .iter()
            .position(|terminal| terminal.kind() == TerminalKind::Kind)
            .unwrap();

        assert_eq!(
            execute_navigation_sql(
                &plan,
                &[(universe, &sources), (identifier, &targets)],
                &edges
            ),
            expected,
            "{relation}"
        );
        assert!(!plan.structural_sql().contains("_accepted AS"));
    }
}

#[test]
fn clause_navigation_carries_its_source_frame_without_rejoining_it() {
    let compiled = code_rule(json!({
        "all": [
            { "kind": "function_definition" },
            { "has": { "kind": "identifier" } }
        ]
    }));
    let plan = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
        .expect("navigation source frames lower into SQL");

    assert_eq!(
        plan.structural_sql()
            .matches("clause_0_relation_2 source")
            .count(),
        1,
        "the navigation seed is the only scan of its source relation"
    );
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
    let field = compared_field_id(plan.structural_sql());
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
    let field = compared_field_id(plan.structural_sql());
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
    create_typed_relation_views(&connection);
    let mut statement = connection
        .prepare(plan.structural_sql())
        .expect("executable SQL plan");
    statement
        .query_map([1_i64], |row| row.get(10))
        .expect("query capture rows")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("typed output rows")
}

#[test]
fn recursive_materialization_fences_preserve_wide_utility_results() {
    let utils = (0..9)
        .map(|ordinal| {
            (
                format!("utility_{ordinal}"),
                json!({ "pattern": format!("name_{ordinal}") }),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    let calls = (0..9)
        .map(|ordinal| json!({ "matches": format!("utility_{ordinal}") }))
        .collect::<Vec<_>>();
    let compiled = code_rule_ast(json!({
        "language": "python",
        "utils": utils,
        "rule": { "any": calls }
    }));

    let violations = sql_violation_values(&compiled, "wide.py", "name_8\n", GrammarDialect::Python);
    assert_eq!(violations.len(), 1);
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
        .prepare(plan.structural_sql())
        .expect("executable SQL plan")
        .query_map([token], |row| row.get::<_, i64>(10))
        .expect("SQL-only repeated capture query")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("typed results");
    assert_eq!(nodes.len(), 1, "only x + x has equal repeated captures");
    let counters = registry
        .query_execution_counters(token)
        .expect("registered packed demand");
    assert_eq!(counters.0, 1);
    assert!((1..=10).contains(&counters.1));
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
    let compare_sql = cte(comparison.structural_sql(), "clause_0_compare_1 AS");
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
    let value_sql = cte(value.structural_sql(), "clause_0_value_1 AS");
    assert!(value_sql.contains("JOIN fact value_fact"));
    assert!(value_sql.contains("value_fact.fact_kind = 1"));
    assert!(value_sql.contains("value_fact.text_value = 'int:1'"));
    assert!(value_sql.contains("THEN NULL"));
}

#[test]
fn compound_value_predicates_lower_to_batched_typed_facts() {
    let compiled = code_rule(json!({
        "kind": "string",
        "value": { "type": "str", "regex": "^(sql|virtual)$" }
    }));
    let plan = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
        .expect("compound value predicates lower to one typed fact producer");

    assert_eq!(
        plan.terminals()
            .iter()
            .map(|terminal| terminal.kind())
            .collect::<Vec<_>>(),
        vec![TerminalKind::Kind, TerminalKind::Value]
    );
    assert!(
        plan.terminals()[1]
            .value_fact()
            .expect("value predicate has a typed descriptor")
            .predicate
            .is_some()
    );
    let value_sql = cte(plan.structural_sql(), "clause_0_value_1 AS");
    assert!(value_sql.contains("value_fact.verdict = 0"));
    assert!(!value_sql.contains("value_fact.text_value ="));
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
        (
            json!({
                "all": [
                    { "pattern": "$A + $B" },
                    { "compare": { "left": "A", "op": "eq", "right": "B", "text": "structure" } }
                ]
            }),
            "x + x\nx + y\n",
        ),
        (
            json!({
                "all": [
                    { "pattern": "$A + $B" },
                    { "compare": { "left": "A", "op": "ne", "right": "B", "text": "structure" } }
                ]
            }),
            "x + x\nx + y\n",
        ),
        (
            json!({
                "all": [
                    { "pattern": "$A + $B" },
                    { "compare": { "left": "A", "op": "eq", "right": "B", "text": "structure" } }
                ]
            }),
            "1 + 0x1\n1 + 2\n",
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
fn utility_limits_cover_acyclic_components_without_partial_rows() {
    let mut limited: serde_json::Value =
        serde_json::from_str(CAPABILITIES).expect("test capabilities are JSON");
    limited["limits"]["maxVisitedNodes"] = 1.into();
    let work_limited = compile_code_rule_with_capabilities(
        json!({
            "language": "python",
            "utils": { "u": { "kind": "identifier" } },
            "rule": { "matches": "u" }
        }),
        "x",
        limited.to_string(),
    );
    let oracle = evaluate_file(&work_limited, "python", "a.py", "x\n");
    assert_eq!(oracle["violations"], json!([]));
    assert_eq!(oracle["failures"][0]["code"], "budget-exceeded");
    let outcome = sql_outcome(&work_limited, "x\n");
    assert!(outcome.ranges.is_empty());
    assert_eq!(outcome.errors, ["utility-work-limit"]);

    let mut utils = serde_json::Map::new();
    utils.insert("u0".to_owned(), json!({ "kind": "identifier" }));
    for depth in 1..=256 {
        utils.insert(
            format!("u{depth}"),
            json!({ "matches": format!("u{}", depth - 1) }),
        );
    }
    let depth_limited = code_rule_ast(
        json!({ "language": "python", "utils": utils, "rule": { "matches": "u256" } }),
    );
    let oracle = evaluate_file(&depth_limited, "python", "a.py", "x\n");
    assert_eq!(oracle["violations"], json!([]));
    assert_eq!(oracle["failures"][0]["code"], "budget-exceeded");
    let outcome = sql_outcome(&depth_limited, "x\n");
    assert!(outcome.ranges.is_empty());
    assert_eq!(outcome.errors, ["utility-depth-limit"]);
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SqlFactRow {
    pub(super) producer: i64,
    pub(super) node: (usize, usize),
    pub(super) related: Option<(usize, usize)>,
    pub(super) slot: i64,
    pub(super) key: i64,
    pub(super) position: i64,
    pub(super) flag: Option<i64>,
    pub(super) target_kind: Option<i64>,
    pub(super) label: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SqlProducerStatusRow {
    pub(super) producer: i64,
    pub(super) file: i64,
    pub(super) complete: i64,
    pub(super) verdict: Option<i64>,
    pub(super) work_used: Option<i64>,
    pub(super) error: Option<String>,
}

pub(super) struct SqlOutcome {
    pub(super) ranges: Vec<(usize, usize)>,
    pub(super) file_indices: Vec<i64>,
    pub(super) document_ids: Vec<(i64, i64)>,
    pub(super) errors: Vec<String>,
    pub(super) facts: Vec<SqlFactRow>,
    pub(super) statuses: Vec<SqlProducerStatusRow>,
    pub(super) columns: Vec<String>,
    pub(super) execution_counters: (usize, usize),
}

pub(super) struct SqlWorkspaceOutcome {
    pub(super) ranges: Vec<(usize, usize, usize)>,
    pub(super) errors: Vec<String>,
    pub(super) statuses: Vec<(usize, usize)>,
}

pub(super) fn sql_workspace_ranges(
    compiled: &crate::code_rule_dsl::CompiledCodeRuleDsl,
    files: &[(String, String)],
) -> SqlWorkspaceOutcome {
    sql_workspace_ranges_for_statement(compiled, files, None)
}

pub(super) fn sql_workspace_ranges_for_statement(
    compiled: &crate::code_rule_dsl::CompiledCodeRuleDsl,
    files: &[(String, String)],
    sql: Option<&str>,
) -> SqlWorkspaceOutcome {
    let plan =
        SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)]).expect("workspace SQL lowering");
    let grammar = Grammar::new(GrammarDialect::Python).expect("Python grammar");
    let mut tapes = Vec::new();
    let mut anchors: Vec<Vec<Vec<u32>>> = Vec::new();
    for (_, source) in files {
        let source = Arc::new(source.clone());
        let image = TapeImage::parse(&grammar, &source, &|| false)
            .expect("parser completed")
            .expect("tape image");
        let tape = TapeDoc::restore(Arc::clone(&source), grammar.clone(), &image, &|| false)
            .expect("restored tape");
        let roots = (0..tape.transducer_len() as u32).collect::<Vec<_>>();
        anchors.push(
            plan.terminals()
                .iter()
                .map(|terminal| {
                    if matches!(
                        terminal.kind(),
                        TerminalKind::File | TerminalKind::Transform
                    ) {
                        Vec::new()
                    } else if terminal.kind() == TerminalKind::Navigation {
                        vec![0]
                    } else {
                        roots.clone()
                    }
                })
                .collect(),
        );
        tapes.push(tape);
    }
    let task_grid = files
        .iter()
        .map(|(path, source)| {
            plan.terminals()
                .iter()
                .map(|terminal| terminal.applies_to_file(path, source, None))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    for (file_anchors, file_tasks) in anchors.iter_mut().zip(&task_grid) {
        for (roots, demanded) in file_anchors.iter_mut().zip(file_tasks) {
            if !demanded {
                roots.clear();
            }
        }
    }
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
        tapes,
        anchors,
        task_grid,
        plan.terminals()
            .iter()
            .map(|terminal| terminal.result_limit())
            .collect(),
    )
    .and_then(|demand| {
        demand.with_sound_under_errors(
            plan.terminals()
                .iter()
                .map(TerminalPlan::sound_under_errors)
                .collect(),
        )
    })
    .and_then(|demand| demand.with_producer_kinds(producer_kinds))
    .and_then(|demand| {
        demand.with_value_facts(
            plan.terminals()
                .iter()
                .map(|terminal| terminal.value_fact())
                .collect(),
        )
    })
    .and_then(|demand| {
        demand.with_semantic_facts(
            plan.terminals()
                .iter()
                .map(|terminal| terminal.semantic_fact())
                .collect(),
        )
    })
    .and_then(|demand| {
        demand.with_transform_facts(
            plan.terminals()
                .iter()
                .map(TerminalPlan::transform_fact)
                .collect(),
        )
    })
    .and_then(|demand| {
        demand.with_file_facts(
            plan.terminals()
                .iter()
                .map(|terminal| terminal.file_fact())
                .collect(),
            files.iter().map(|(path, _)| path.clone()).collect(),
        )
    })
    .and_then(|demand| {
        demand.with_parse_facts(
            plan.terminals()
                .iter()
                .map(|terminal| terminal.parse_fact())
                .collect(),
        )
    })
    .and_then(|demand| demand.with_document_maps(Vec::new(), false))
    .expect("immutable workspace demand");
    let connection = Connection::open_in_memory().expect("in-memory SQLite");
    let registry = Arc::new(MatchRegistry::default());
    register_module(&connection, Arc::clone(&registry)).expect("packed virtual table");
    let token = registry.register(demand);
    let mut statement = connection
        .prepare(sql.unwrap_or_else(|| plan.structural_sql()))
        .expect("workspace SQL statement");
    let columns = statement
        .column_names()
        .iter()
        .map(|name| name.to_string())
        .collect::<Vec<_>>();
    let column = |name: &str| columns.iter().position(|column| column == name);
    let file = column("file_index").expect("file column");
    let start = column("start_byte").expect("start column");
    let end = column("end_byte").expect("end column");
    let error = column("error_code");
    let metadata = file_metadata(files, GrammarDialect::Python);
    let parameters = if statement
        .parameter_index(":files")
        .expect("valid file metadata parameter")
        .is_some()
    {
        vec![
            rusqlite::types::Value::Integer(token),
            rusqlite::types::Value::Text(metadata),
        ]
    } else {
        vec![rusqlite::types::Value::Integer(token)]
    };
    let rows = statement
        .query_map(rusqlite::params_from_iter(parameters), |row| {
            Ok((
                row.get::<_, Option<i64>>(file)?,
                row.get::<_, Option<i64>>(start)?,
                row.get::<_, Option<i64>>(end)?,
                error
                    .map(|column| row.get::<_, Option<String>>(column))
                    .transpose()?
                    .flatten(),
            ))
        })
        .expect("workspace query")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("workspace rows");
    let mut ranges = rows
        .iter()
        .filter_map(|(file, start, end, error)| {
            if error.is_none() {
                Some((
                    usize::try_from((*file)?).ok()?,
                    usize::try_from((*start)?).ok()?,
                    usize::try_from((*end)?).ok()?,
                ))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    let mut errors = rows
        .into_iter()
        .filter_map(|(_, _, _, error)| error)
        .collect::<Vec<_>>();
    ranges.sort_unstable();
    errors.sort();
    errors.dedup();
    let mut statuses = registry
        .query_producer_status_rows(token)
        .into_iter()
        .map(|row| (row.file as usize, row.producer as usize))
        .collect::<Vec<_>>();
    statuses.sort_unstable();
    SqlWorkspaceOutcome {
        ranges,
        errors,
        statuses,
    }
}

pub(super) fn sql_violation_values(
    compiled: &crate::code_rule_dsl::CompiledCodeRuleDsl,
    path: &str,
    source: &str,
    dialect: GrammarDialect,
) -> Vec<serde_json::Value> {
    sql_violation_values_for_statement(compiled, path, source, dialect, None)
}

pub(super) fn sql_violation_values_for_statement(
    compiled: &crate::code_rule_dsl::CompiledCodeRuleDsl,
    path: &str,
    source: &str,
    dialect: GrammarDialect,
    statement: Option<&str>,
) -> Vec<serde_json::Value> {
    let source = Arc::new(source.to_owned());
    let grammar = Grammar::new(dialect).expect("contract grammar");
    let image = TapeImage::parse(&grammar, &source, &|| false)
        .expect("parser completed")
        .expect("tape image");
    let tape =
        TapeDoc::restore(Arc::clone(&source), grammar, &image, &|| false).expect("restored tape");
    let roots = (0..tape.transducer_len() as u32).collect::<Vec<_>>();
    let plans = SqlPlans::compile(&[Arc::clone(&compiled.inner)]);
    let files = vec![PackedFileInput {
        tape,
        language: dialect,
        path: path.to_owned(),
        module: None,
        tasks: BTreeMap::from([((0, 0), roots)]),
        document_maps: Vec::new(),
        syntax_failure: None,
    }];
    let outcome = match statement {
        Some(statement) => plans.execute_statement(files, statement),
        None => plans.execute(files),
    }
    .expect("typed SQL violation output");
    assert!(outcome.rows.is_empty(), "production returned packed rows");
    assert!(outcome.failures.is_empty(), "production returned failures");
    outcome
        .violations
        .iter()
        .map(super::SqlViolationRow::to_value)
        .collect()
}

pub(super) fn sql_outcome(
    compiled: &crate::code_rule_dsl::CompiledCodeRuleDsl,
    source: &str,
) -> SqlOutcome {
    sql_outcome_for_dialect(compiled, source, GrammarDialect::Python)
}

pub(super) fn sql_outcome_for_dialect(
    compiled: &crate::code_rule_dsl::CompiledCodeRuleDsl,
    source: &str,
    dialect: GrammarDialect,
) -> SqlOutcome {
    sql_outcome_with_failure(compiled, source, dialect, None, None, false, Vec::new())
}

pub(super) fn sql_outcome_for_statement(
    compiled: &crate::code_rule_dsl::CompiledCodeRuleDsl,
    source: &str,
    dialect: GrammarDialect,
    sql: &str,
) -> SqlOutcome {
    sql_outcome_with_failure(
        compiled,
        source,
        dialect,
        None,
        Some(sql),
        false,
        Vec::new(),
    )
}

enum InjectedFailureProducer {
    SemanticFact(usize),
    Exact(usize),
}

pub(super) fn sql_outcome_for_statement_with_fact_failure(
    compiled: &crate::code_rule_dsl::CompiledCodeRuleDsl,
    source: &str,
    dialect: GrammarDialect,
    fact_producer: usize,
    code: &str,
    sql: &str,
) -> SqlOutcome {
    sql_outcome_with_failure(
        compiled,
        source,
        dialect,
        Some((
            InjectedFailureProducer::SemanticFact(fact_producer),
            code.to_owned(),
        )),
        Some(sql),
        false,
        Vec::new(),
    )
}

pub(super) fn sql_outcome_for_statement_with_producer_failure(
    compiled: &crate::code_rule_dsl::CompiledCodeRuleDsl,
    source: &str,
    dialect: GrammarDialect,
    producer: usize,
    code: &str,
    sql: &str,
) -> SqlOutcome {
    sql_outcome_with_failure(
        compiled,
        source,
        dialect,
        Some((InjectedFailureProducer::Exact(producer), code.to_owned())),
        Some(sql),
        false,
        Vec::new(),
    )
}

pub(super) fn sql_outcome_with_producer_failure(
    compiled: &crate::code_rule_dsl::CompiledCodeRuleDsl,
    source: &str,
    dialect: GrammarDialect,
    producer: usize,
    code: &str,
) -> SqlOutcome {
    sql_outcome_with_failure(
        compiled,
        source,
        dialect,
        Some((InjectedFailureProducer::Exact(producer), code.to_owned())),
        None,
        true,
        Vec::new(),
    )
}

pub(super) fn sql_outcome_with_fact_failure(
    compiled: &crate::code_rule_dsl::CompiledCodeRuleDsl,
    source: &str,
    dialect: GrammarDialect,
    fact_producer: usize,
    code: &str,
) -> SqlOutcome {
    sql_outcome_with_failure(
        compiled,
        source,
        dialect,
        Some((
            InjectedFailureProducer::SemanticFact(fact_producer),
            code.to_owned(),
        )),
        None,
        false,
        Vec::new(),
    )
}

fn sql_outcome_with_failure(
    compiled: &crate::code_rule_dsl::CompiledCodeRuleDsl,
    source: &str,
    dialect: GrammarDialect,
    failure: Option<(InjectedFailureProducer, String)>,
    statement: Option<&str>,
    use_execution_sql: bool,
    document_maps: Vec<DocumentMapSpan>,
) -> SqlOutcome {
    let plan = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
        .expect("policy has complete structural SQL lowering");
    let execution_sql = use_execution_sql.then(|| plan.execution_sql());
    let statement = execution_sql.or(statement);
    let source = Arc::new(source.to_owned());
    let grammar = Grammar::new(dialect).expect("contract grammar");
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
            if matches!(
                terminal.kind(),
                TerminalKind::File | TerminalKind::Transform
            ) {
                Vec::new()
            } else if terminal.kind() == TerminalKind::Navigation {
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
    let injected = failure.map(|(source, code)| {
        let producer = match source {
            InjectedFailureProducer::Exact(producer) => producer,
            InjectedFailureProducer::SemanticFact(ordinal) => producer_kinds
                .iter()
                .enumerate()
                .filter(|(_, kind)| **kind == ProducerKind::SemanticFact)
                .nth(ordinal)
                .map(|(producer, _)| producer)
                .expect("injected semantic fact producer exists"),
        };
        (producer, code)
    });
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
        demand.with_sound_under_errors(
            plan.terminals()
                .iter()
                .map(TerminalPlan::sound_under_errors)
                .collect(),
        )
    })
    .and_then(|demand| demand.with_producer_kinds(producer_kinds))
    .and_then(|demand| {
        demand.with_value_facts(
            plan.terminals()
                .iter()
                .map(|terminal| terminal.value_fact())
                .collect(),
        )
    })
    .and_then(|demand| {
        demand.with_semantic_facts(
            plan.terminals()
                .iter()
                .map(|terminal| terminal.semantic_fact())
                .collect(),
        )
    })
    .and_then(|demand| {
        demand.with_transform_facts(
            plan.terminals()
                .iter()
                .map(TerminalPlan::transform_fact)
                .collect(),
        )
    })
    .and_then(|demand| {
        demand.with_file_facts(
            plan.terminals()
                .iter()
                .map(|terminal| terminal.file_fact())
                .collect(),
            vec!["test_contract.py".to_owned()],
        )
    })
    .and_then(|demand| {
        demand.with_parse_facts(
            plan.terminals()
                .iter()
                .map(|terminal| terminal.parse_fact())
                .collect(),
        )
    })
    .and_then(|demand| {
        demand.with_document_maps(document_maps, plan.shape().contains(&SqlShape::MapDocument))
    })
    .and_then(|demand| match injected {
        Some((producer, code)) => demand.with_failure_injection(producer, code),
        None => Ok(demand),
    })
    .expect("immutable structural demand");
    let connection = Connection::open_in_memory().expect("in-memory SQLite");
    let registry = Arc::new(MatchRegistry::default());
    register_module(&connection, Arc::clone(&registry)).expect("packed virtual table");
    let token = registry.register(demand);
    let mut statement = connection
        .prepare(statement.unwrap_or_else(|| plan.structural_sql()))
        .expect("executable SQL plan");
    let columns = statement
        .column_names()
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let metadata = file_metadata(
        &[("test_contract.py".to_owned(), source.to_string())],
        dialect,
    );
    let parameters = if statement
        .parameter_index(":files")
        .expect("valid file metadata parameter")
        .is_some()
    {
        vec![
            rusqlite::types::Value::Integer(token),
            rusqlite::types::Value::Text(metadata),
        ]
    } else {
        vec![rusqlite::types::Value::Integer(token)]
    };
    let rows = statement
        .query_map(rusqlite::params_from_iter(parameters), |row| {
            let named = |name| row.as_ref().column_index(name).ok();
            let get_optional = |name| {
                named(name)
                    .map(|index| row.get::<_, Option<i64>>(index))
                    .transpose()
                    .map(Option::flatten)
            };
            Ok((
                if let Some(index) = named("row_kind") {
                    row.get::<_, i64>(index)?
                } else if let Some(index) = named("output_kind") {
                    match row.get::<_, i64>(index)? {
                        2 => 8,
                        _ => 3,
                    }
                } else {
                    3
                },
                get_optional("file_index")?,
                get_optional("document_id")?,
                get_optional("mapped_document_id")?,
                get_optional("node")?,
                get_optional("start_byte")?,
                get_optional("end_byte")?,
                named("error_code")
                    .map(|index| row.get::<_, Option<String>>(index))
                    .transpose()?
                    .flatten(),
                named("bindings")
                    .map(|index| row.get::<_, String>(index))
                    .transpose()?
                    .unwrap_or_else(|| "[]".to_owned()),
                get_optional("argument_node")?.or(get_optional("target_node")?),
            ))
        })
        .expect("SQL structural query")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("typed SQL nodes");
    let execution_counters = registry
        .query_execution_counters(token)
        .expect("the packed batch remains registered");
    let statuses = registry
        .query_producer_status_rows(token)
        .into_iter()
        .map(|row| SqlProducerStatusRow {
            producer: row.producer,
            file: row.file,
            complete: row.complete,
            verdict: row.verdict,
            work_used: row.work_used,
            error: row.error,
        })
        .collect::<Vec<_>>();
    let mut facts = registry
        .query_fact_rows(token)
        .into_iter()
        .map(|row| SqlFactRow {
            producer: row.producer,
            node: {
                let span = &spans[row.node as usize];
                (span.start, span.end)
            },
            related: row.related.map(|node| {
                let span = &spans[node as usize];
                (span.start, span.end)
            }),
            slot: row.slot,
            key: row.key,
            position: row.position,
            flag: row.flag,
            target_kind: row.target_kind,
            label: row.label,
        })
        .collect::<Vec<_>>();
    facts.sort_by_key(|row| {
        (
            row.producer,
            row.node,
            row.slot,
            row.key,
            row.position,
            row.related,
        )
    });
    if plan.terminals().is_empty() {
        assert_eq!(execution_counters, (0, 0));
    } else {
        assert_eq!(execution_counters.0, 1, "the packed demand executes once");
        assert!(
            (1..=10).contains(&execution_counters.1),
            "only typed relation scans are counted: {execution_counters:?}"
        );
    }
    let anchor = &compiled.inner.clauses[0].violation.anchor;
    let mut errors = rows
        .iter()
        .filter(|(row_kind, _, _, _, _, _, _, _, _, _)| *row_kind == 8)
        .map(|(_, _, _, _, _, _, _, error, _, _)| {
            error.clone().expect("a failure row has an error code")
        })
        .collect::<Vec<_>>();
    errors.sort();
    errors.dedup();
    let mut file_indices = rows
        .iter()
        .filter(|(row_kind, _, _, _, _, _, _, _, _, _)| *row_kind != 8)
        .map(|(_, file, _, _, _, _, _, _, _, _)| file.expect("a violation row has a file"))
        .collect::<Vec<_>>();
    file_indices.sort_unstable();
    let mut document_ids = rows
        .iter()
        .filter(|(row_kind, _, _, _, _, _, _, _, _, _)| *row_kind != 8)
        .filter_map(|(_, _, document, mapped, _, _, _, _, _, _)| {
            Some((document.as_ref().copied()?, mapped.as_ref().copied()?))
        })
        .collect::<Vec<_>>();
    document_ids.sort_unstable();
    let mut ranges = rows
        .into_iter()
        .filter(|(row_kind, _, _, _, _, _, _, _, _, _)| *row_kind != 8)
        .map(
            |(_, _, _, _, node, start, end, _, bindings, argument_node)| {
                let node = node.expect("a violation row has a node");
                if let Anchor::Capture(label) = anchor {
                    let bindings = serde_json::from_str::<serde_json::Value>(&bindings)
                        .expect("JSON binding environment");
                    let binding = bindings
                        .as_array()
                        .expect("binding array")
                        .iter()
                        .find(|binding| binding["label"] == *label && binding["kind"] == 1);
                    if let Some(binding) = binding
                        && let (Some(start), Some(end)) =
                            (binding["start_byte"].as_u64(), binding["end_byte"].as_u64())
                    {
                        return (start as usize, end as usize);
                    }
                    let node = binding
                        .and_then(|binding| binding["node"].as_i64())
                        .or(argument_node)
                        .unwrap_or(node);
                    let span = &spans[node as usize];
                    return (span.start, span.end);
                }
                match (start, end) {
                    (Some(start), Some(end)) => (start as usize, end as usize),
                    _ => {
                        let span = &spans[node as usize];
                        (span.start, span.end)
                    }
                }
            },
        )
        .collect::<Vec<_>>();
    ranges.sort_unstable();
    SqlOutcome {
        ranges,
        file_indices,
        document_ids,
        errors,
        facts,
        statuses,
        columns,
        execution_counters,
    }
}

fn validated_document_maps(
    source: &str,
    maps: &[(usize, usize, usize, usize)],
) -> Result<Vec<DocumentMapSpan>, String> {
    if maps.is_empty() {
        return Err("document-map-missing".to_owned());
    }
    let mut ordered = maps.to_vec();
    ordered.sort_unstable_by_key(|span| (span.2, span.3, span.0, span.1));
    for (index, span) in ordered.iter().enumerate() {
        if span.0 >= span.1 || span.2 >= span.3 || span.1 > source.len() {
            return Err("document-map-invalid-span".to_owned());
        }
        if ordered[..index].iter().any(|earlier| {
            (span.0 < earlier.1 && earlier.0 < span.1) || (span.2 < earlier.3 && earlier.2 < span.3)
        }) {
            return Err("document-map-overlap".to_owned());
        }
    }
    Ok(ordered
        .into_iter()
        .map(
            |(source_start, source_end, generated_start, generated_end)| DocumentMapSpan {
                source_document: 0,
                generated_document: 1,
                source_start,
                source_end,
                generated_start,
                generated_end,
            },
        )
        .collect())
}

pub(super) fn sql_outcome_with_document_maps(
    compiled: &crate::code_rule_dsl::CompiledCodeRuleDsl,
    source: &str,
    maps: &[(usize, usize, usize, usize)],
) -> Result<SqlOutcome, String> {
    Ok(sql_outcome_with_failure(
        compiled,
        source,
        GrammarDialect::Blade,
        None,
        None,
        false,
        validated_document_maps(source, maps)?,
    ))
}

pub(super) fn sql_outcome_for_statement_with_document_maps(
    compiled: &crate::code_rule_dsl::CompiledCodeRuleDsl,
    source: &str,
    maps: &[(usize, usize, usize, usize)],
    sql: &str,
) -> Result<SqlOutcome, String> {
    Ok(sql_outcome_with_failure(
        compiled,
        source,
        GrammarDialect::Blade,
        None,
        Some(sql),
        false,
        validated_document_maps(source, maps)?,
    ))
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
            if matches!(
                terminal.kind(),
                TerminalKind::File | TerminalKind::Transform
            ) {
                Vec::new()
            } else if terminal.kind() == TerminalKind::Navigation {
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
        .prepare(plan.structural_sql())
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
        "all": [
            { "pattern": "$A + $B" },
            { "compare": { "left": "A", "op": "eq", "right": "B", "text": "tokens" } }
        ]
    }));
    let error = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
        .expect_err("the compiler reports unsupported relational operators");

    assert_eq!(
        error.unsupported(),
        &["generated relational plan: comparison text mode is not lowered in phase two"]
    );
}
