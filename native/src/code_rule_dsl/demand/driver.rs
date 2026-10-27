//! Runs lowered SQL over `demand(batch)` relations on the content store's connection (05 Phase 5
//! row 2; docs/final-shape/03-architecture.md §3).
//!
//! From N's `query/mod.rs`. A batch's packed programs and tapes are registered with the
//! process's registry, SQLite demands them through the typed relations of `vtab`, whose `xFilter`
//! executes the batch once, and the statements' rows come back as violations and failures. N
//! lowered whole rule catalogs to that SQL itself (`SqlPlans::compile` over `relational_ir`, tree
//! axes in SQL), which is not copied; the statements come from the logic planner (Phase 6) and the
//! batches from the waves over `prefilter/` candidate sets (Phase 7). N ran on a private in-memory
//! connection per thread; here the statements run in one transaction on the store, where the
//! fact relations and their statistics live.

use super::vtab::{HardPartition, MatchRegistry, PackedDemand, PackedRow, register_module};
use crate::code_rule_dsl::transducer::packed::ExecutionBackend;
use crate::content_store::{ContentStore, StoreResult};
use rusqlite::{Connection, Row};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock};

/// The batches registered by every driver; each lives for one `drive`.
static REGISTRY: LazyLock<Arc<MatchRegistry>> = LazyLock::new(Arc::default);

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct SqlFixRow {
    span: crate::code_rule_dsl::lines::Span,
    text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::code_rule_dsl) struct SqlViolationRow {
    rule_id: String,
    clause_id: String,
    code: String,
    message: String,
    path: String,
    span: crate::code_rule_dsl::lines::Span,
    mode: String,
    stable_order: String,
    captures: BTreeMap<String, Vec<crate::code_rule_dsl::lines::Span>>,
    variables: BTreeMap<String, bool>,
    fix: Option<SqlFixRow>,
    witness_start_byte: usize,
    witness_end_byte: usize,
    each_range: Option<(usize, usize)>,
}

impl SqlViolationRow {
    fn json_column<T: serde::de::DeserializeOwned>(
        row: &Row<'_>,
        column: &str,
        text: String,
    ) -> rusqlite::Result<T> {
        serde_json::from_str(&text).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                row.as_ref().column_index(column).unwrap_or_default(),
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })
    }

    fn from_sql_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        let span: crate::code_rule_dsl::lines::Span =
            Self::json_column(row, "span", row.get("span")?)?;
        let captures: BTreeMap<String, Vec<crate::code_rule_dsl::lines::Span>> =
            Self::json_column(row, "captures", row.get("captures")?)?;
        let variables: BTreeMap<String, bool> =
            Self::json_column(row, "variables", row.get("variables")?)?;
        let fix: Option<SqlFixRow> = row
            .get::<_, Option<String>>("fix")?
            .map(|text| Self::json_column(row, "fix", text))
            .transpose()?;
        let required = |column: &str| -> rusqlite::Result<String> {
            let value: String = row.get(column)?;
            if value.is_empty() {
                return Err(rusqlite::Error::InvalidColumnType(
                    row.as_ref().column_index(column)?,
                    column.to_owned(),
                    rusqlite::types::Type::Null,
                ));
            }
            Ok(value)
        };
        let each_start: Option<i64> = row.get("each_start_byte")?;
        let each_end: Option<i64> = row.get("each_end_byte")?;
        let each_range = match (each_start, each_end) {
            (Some(start), Some(end)) if start >= 0 && start <= end => Some((
                usize::try_from(start)
                    .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, start))?,
                usize::try_from(end)
                    .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, end))?,
            )),
            (None, None) => None,
            _ => return Err(rusqlite::Error::IntegralValueOutOfRange(0, 0)),
        };
        if span.start_byte > span.end_byte
            || captures
                .values()
                .flatten()
                .any(|span| span.start_byte > span.end_byte)
            || fix
                .as_ref()
                .is_some_and(|fix| fix.span.start_byte > fix.span.end_byte)
        {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let mode = required("mode")?;
        if !matches!(mode.as_str(), "log" | "steer" | "block") {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let witness_start_byte = usize::try_from(row.get::<_, i64>("witness_start_byte")?)
            .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, 0))?;
        let witness_end_byte = usize::try_from(row.get::<_, i64>("witness_end_byte")?)
            .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, 0))?;
        if witness_start_byte > witness_end_byte {
            return Err(rusqlite::Error::InvalidQuery);
        }
        Ok(Self {
            rule_id: required("rule_id")?,
            clause_id: required("clause_id")?,
            code: required("code")?,
            message: required("message")?,
            path: required("path")?,
            span,
            mode,
            stable_order: required("stable_order")?,
            captures,
            variables,
            fix,
            witness_start_byte,
            witness_end_byte,
            each_range,
        })
    }

    pub(in crate::code_rule_dsl) fn publication_key(&self) -> (&str, &str, &str) {
        (&self.rule_id, &self.clause_id, &self.path)
    }

    pub(in crate::code_rule_dsl) fn into_witness(self) -> crate::code_rule_dsl::violations::Witness {
        crate::code_rule_dsl::violations::Witness {
            path: self.path,
            span: self.span,
            captures: crate::code_rule_dsl::violations::Captures::from_spans(self.captures),
            message: Some(self.message),
            fix: self.fix.map(|fix| {
                serde_json::to_value(fix).expect("a typed SQL fix serializes as a witness fix")
            }),
            ignored: false,
        }
    }

    pub(in crate::code_rule_dsl) fn to_value(&self) -> Value {
        let mut id = crate::code_rule_dsl::eval::violation_id(
            &self.rule_id,
            &self.clause_id,
            &self.code,
            &self.path,
            self.witness_start_byte as u64,
            self.witness_end_byte as u64,
        );
        if let Some((start, end)) = self.each_range {
            let source = format!("{id}\0each\0{start}\0{end}");
            id = format!("sha256:{}", crate::code_rule_dsl::json::hex_digest(source.as_bytes()));
        }
        let mut value = json!({
            "id": id,
            "ruleId": self.rule_id,
            "clauseId": self.clause_id,
            "code": self.code,
            "mode": self.mode,
            "message": self.message,
            "path": self.path,
            "span": self.span,
            "captures": self.captures,
            "variables": self.variables,
        });
        if let Some(fix) = &self.fix {
            value
                .as_object_mut()
                .expect("violation is an object")
                .insert(
                    "fix".to_owned(),
                    serde_json::to_value(fix).expect("typed fix serializes"),
                );
        }
        value
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::code_rule_dsl) struct SqlFailureRow {
    pub(in crate::code_rule_dsl) rule_id: String,
    pub(in crate::code_rule_dsl) clause_id: String,
    pub(in crate::code_rule_dsl) path: String,
    pub(in crate::code_rule_dsl) code: String,
    pub(in crate::code_rule_dsl) message: Option<String>,
}

/// Rows returned by SQL after completion filtering and stable ordering.
pub(in crate::code_rule_dsl) struct QueryOutcome {
    pub(in crate::code_rule_dsl) rows: Vec<PackedRow>,
    pub(in crate::code_rule_dsl) violations: Vec<SqlViolationRow>,
    pub(in crate::code_rule_dsl) failures: Vec<SqlFailureRow>,
    pub(in crate::code_rule_dsl) counters: QueryCounters,
}

/// Profile counters for one SQL-demanded workspace batch.
pub(in crate::code_rule_dsl) struct QueryCounters {
    pub(in crate::code_rule_dsl) statements: usize,
    pub(in crate::code_rule_dsl) rows: usize,
    pub(in crate::code_rule_dsl) terminal_executions: usize,
    pub(in crate::code_rule_dsl) virtual_table_filters: usize,
    pub(in crate::code_rule_dsl) registrations: usize,
    pub(in crate::code_rule_dsl) backend_invocations: usize,
    pub(in crate::code_rule_dsl) hard_partitions: usize,
    pub(in crate::code_rule_dsl) partitions: Vec<HardPartition>,
    pub(in crate::code_rule_dsl) demanded_producers: usize,
    pub(in crate::code_rule_dsl) terminal_tasks: usize,
    pub(in crate::code_rule_dsl) command_buffers: usize,
    pub(in crate::code_rule_dsl) packed_dispatches: usize,
    pub(in crate::code_rule_dsl) anchor_dispatches: usize,
    pub(in crate::code_rule_dsl) matcher_calls: usize,
    pub(in crate::code_rule_dsl) host_callbacks: usize,
}

fn split_measured_statement_packets(
    ordinals: &[usize],
    byte_limit: usize,
    mut render: impl FnMut(&[usize]) -> Result<String, String>,
) -> Result<Vec<(Vec<usize>, String)>, String> {
    fn split(
        ordinals: &[usize],
        byte_limit: usize,
        render: &mut impl FnMut(&[usize]) -> Result<String, String>,
        packets: &mut Vec<(Vec<usize>, String)>,
    ) -> Result<(), String> {
        if ordinals.is_empty() {
            return Ok(());
        }
        let sql = render(ordinals)?;
        if sql.len() <= byte_limit {
            packets.push((ordinals.to_vec(), sql));
            return Ok(());
        }
        if ordinals.len() == 1 {
            return Err(format!(
                "relational SQL exceeds bounded statement capacity: {} bytes > {byte_limit} bytes for clause {}",
                sql.len(),
                ordinals[0]
            ));
        }
        let middle = ordinals.len() / 2;
        split(&ordinals[..middle], byte_limit, render, packets)?;
        split(&ordinals[middle..], byte_limit, render, packets)
    }

    let mut packets = Vec::new();
    split(ordinals, byte_limit, &mut render, &mut packets)?;
    Ok(packets)
}

/// One lowered statement: its SQL, the (rule, clause) keys it answers as a JSON array (bound to
/// `:clauses`), and the temporary table it fills, if it fills one.
pub(in crate::code_rule_dsl) struct ExecutionStatement {
    pub(in crate::code_rule_dsl) sql: String,
    pub(in crate::code_rule_dsl) clauses: String,
    pub(in crate::code_rule_dsl) materialization: Option<String>,
}

fn production_backend() -> ExecutionBackend {
    #[cfg(target_os = "macos")]
    {
        ExecutionBackend::MetalJit
    }
    #[cfg(not(target_os = "macos"))]
    {
        ExecutionBackend::CpuJit
    }
}

/// Registers `demand` and runs `statements` over it in one transaction on `store`. Each
/// statement either fills a temporary table (`materialization`) or returns output rows
/// (`output_kind` 1: violation, 2: failure) or, failing those, relation rows.
pub(in crate::code_rule_dsl) fn drive(
    store: &ContentStore,
    demand: PackedDemand,
    statements: &[ExecutionStatement],
    file_metadata: &str,
) -> StoreResult<QueryOutcome> {
    let token = REGISTRY.register(demand);
    let outcome = store.candidate_transaction(false, |db| {
        // Registering again replaces the modules and re-reads their row estimates from
        // `fact_stats`, which the previous batch may have changed.
        register_module(db, Arc::clone(&REGISTRY))?;
        let mut materialized = Vec::new();
        let result = run(db, token, statements, file_metadata, &mut materialized);
        db.flush_prepared_statement_cache();
        for table in materialized.into_iter().rev() {
            let _ = db.execute_batch(&format!("DROP TABLE IF EXISTS temp.\"{table}\""));
        }
        result
    });
    REGISTRY.remove_batch(token);
    outcome
}

fn run(
    db: &Connection,
    token: i64,
    statements: &[ExecutionStatement],
    file_metadata: &str,
    materialized: &mut Vec<String>,
) -> rusqlite::Result<QueryOutcome> {
    let mut rows = Vec::new();
    let mut violations = Vec::new();
    let mut failures = Vec::new();
    for execution in statements {
        if let Some(table) = &execution.materialization {
            db.execute_batch(&format!("DROP TABLE IF EXISTS temp.\"{table}\""))?;
            let mut statement = db.prepare(&execution.sql)?;
            statement.execute(rusqlite::named_params! {
                ":batch": token,
                ":files": file_metadata,
            })?;
            materialized.push(table.clone());
            continue;
        }
        let mut statement = db.prepare_cached(&execution.sql)?;
        let output = statement.column_index("output_kind").is_ok();
        if output {
            enum OutputRow {
                Violation(Box<SqlViolationRow>),
                Failure(SqlFailureRow),
            }
            let decode = |row: &Row<'_>| -> rusqlite::Result<OutputRow> {
                match row.get::<_, i64>("output_kind")? {
                    1 => SqlViolationRow::from_sql_row(row)
                        .map(Box::new)
                        .map(OutputRow::Violation),
                    2 => {
                        let required = |column: &str| -> rusqlite::Result<String> {
                            let value: String = row.get(column)?;
                            if value.is_empty() {
                                return Err(rusqlite::Error::InvalidColumnType(
                                    row.as_ref().column_index(column)?,
                                    column.to_owned(),
                                    rusqlite::types::Type::Null,
                                ));
                            }
                            Ok(value)
                        };
                        let message = row
                            .get::<_, Option<String>>("message")?
                            .filter(|message| !message.is_empty());
                        Ok(OutputRow::Failure(SqlFailureRow {
                            rule_id: required("rule_id")?,
                            clause_id: required("clause_id")?,
                            path: required("path")?,
                            code: required("error_code")?,
                            message,
                        }))
                    }
                    _ => Err(rusqlite::Error::InvalidQuery),
                }
            };
            let decoded = if statement.parameter_index(":clauses")?.is_some() {
                statement
                    .query_map(
                        rusqlite::named_params! {":batch": token, ":files": file_metadata, ":clauses": execution.clauses},
                        decode,
                    )?
                    .collect::<rusqlite::Result<Vec<_>>>()?
            } else {
                statement
                    .query_map(
                        rusqlite::named_params! {":batch": token, ":files": file_metadata},
                        decode,
                    )?
                    .collect::<rusqlite::Result<Vec<_>>>()?
            };
            for row in decoded {
                match row {
                    OutputRow::Violation(row) => violations.push(*row),
                    OutputRow::Failure(row) => failures.push(row),
                }
            }
        } else {
            rows.extend(
                statement
                    .query_map([token], PackedRow::from_sql_row)?
                    .collect::<rusqlite::Result<Vec<_>>>()?,
            );
        }
    }
    let row_count = rows.len() + violations.len() + failures.len();
    let packed = REGISTRY
        .query_execution_report(token)
        .expect("the registered batch remains available until publication");
    Ok(QueryOutcome {
        counters: QueryCounters {
            statements: statements.len(),
            rows: row_count,
            terminal_executions: packed.demand_executions,
            virtual_table_filters: packed.filters,
            registrations: packed.registrations,
            backend_invocations: packed.backend_invocations,
            hard_partitions: packed.hard_partitions,
            partitions: packed.partitions,
            demanded_producers: packed.demanded_producers,
            terminal_tasks: packed.terminal_tasks,
            command_buffers: packed.command_buffers,
            packed_dispatches: packed.packed_dispatches,
            anchor_dispatches: packed.anchor_dispatches,
            matcher_calls: packed.matcher_calls,
            host_callbacks: packed.host_callbacks,
        },
        rows,
        violations,
        failures,
    })
}

#[cfg(test)]
mod tests;
