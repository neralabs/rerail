//! SQL-owned workspace execution.
//!
//! The compiler lowers a catalog to verified packed programs. The workspace supplies immutable
//! tapes and a sparse task grid. SQLite demands producer work through narrow typed relations;
//! `xFilter` executes each admitted batch once, and materialized SQL relations perform completion
//! filtering and deterministic extraction. No host callback or semantic matcher is an execution
//! dependency.

#[cfg(test)]
#[path = "plan/render/compact_output_sql.rs"]
mod compact_output_sql;
pub(in crate::code_rule_dsl) mod crossfile;
mod plan;
#[path = "plan/render/relational.rs"]
mod relational;
#[path = "plan/render/relational_ir.rs"]
mod relational_ir;
pub(in crate::code_rule_dsl) mod select_when;

use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::lines::{LineTable, Terminators};
use crate::code_rule_dsl::model::CompiledCodeRule;
use crate::code_rule_dsl::packed::ExecutionBackend;
use crate::code_rule_dsl::tape::TapeDoc;
use crate::code_rule_dsl::transducer::sql_backend::{
    DocumentMapSpan, HardPartition, MatchRegistry, PackedDemand, PackedRow, ProducerKind,
    register_module,
};
use rusqlite::{Connection, Row};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;

type ClauseId = (usize, usize);

/// One file's immutable input to the packed batch. `tasks` contains only demanded clauses; an
/// empty roots vector is a demanded task with a complete negative result.
pub(super) struct PackedFileInput {
    pub(super) tape: TapeDoc,
    pub(super) language: GrammarDialect,
    pub(super) path: String,
    pub(super) module: Option<crate::code_rule_dsl::module_path::ModulePath>,
    pub(super) tasks: BTreeMap<ClauseId, Vec<u32>>,
    pub(super) document_maps: Vec<DocumentMapSpan>,
    pub(super) syntax_failure: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct SqlFixRow {
    span: crate::code_rule_dsl::lines::Span,
    text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SqlViolationRow {
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

    pub(super) fn publication_key(&self) -> (&str, &str, &str) {
        (&self.rule_id, &self.clause_id, &self.path)
    }

    pub(super) fn into_witness(self) -> super::violations::Witness {
        let sql_violation = Some(self.to_value());
        super::violations::Witness {
            path: self.path,
            span: self.span,
            captures: super::violations::Captures::from_spans(self.captures),
            message: Some(self.message),
            fix: self.fix.map(|fix| {
                serde_json::to_value(fix).expect("a typed SQL fix serializes as a witness fix")
            }),
            sql_violation,
        }
    }

    pub(super) fn to_value(&self) -> Value {
        let mut id = super::eval::violation_id(
            &self.rule_id,
            &self.clause_id,
            &self.code,
            &self.path,
            self.witness_start_byte as u64,
            self.witness_end_byte as u64,
        );
        if let Some((start, end)) = self.each_range {
            let source = format!("{id}\0each\0{start}\0{end}");
            id = format!("sha256:{}", super::json::hex_digest(source.as_bytes()));
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
pub(super) struct SqlFailureRow {
    pub(super) rule_id: String,
    pub(super) clause_id: String,
    pub(super) path: String,
    pub(super) code: String,
    pub(super) message: Option<String>,
}

/// Rows returned by SQL after completion filtering and stable ordering.
pub(super) struct QueryOutcome {
    pub(super) rows: Vec<PackedRow>,
    pub(super) violations: Vec<SqlViolationRow>,
    pub(super) failures: Vec<SqlFailureRow>,
    pub(super) counters: QueryCounters,
}

/// Profile counters for one SQL-demanded workspace batch.
pub(super) struct QueryCounters {
    pub(super) statements: usize,
    pub(super) rows: usize,
    pub(super) scalar_callbacks: usize,
    pub(super) node_rows: usize,
    pub(super) environments: usize,
    pub(super) fact_builds: usize,
    pub(super) fact_hits: usize,
    pub(super) terminal_executions: usize,
    pub(super) virtual_table_filters: usize,
    pub(super) registrations: usize,
    pub(super) backend_invocations: usize,
    pub(super) hard_partitions: usize,
    pub(super) partitions: Vec<HardPartition>,
    pub(super) demanded_producers: usize,
    pub(super) terminal_tasks: usize,
    pub(super) command_buffers: usize,
    pub(super) packed_dispatches: usize,
    pub(super) anchor_dispatches: usize,
    pub(super) matcher_calls: usize,
    pub(super) host_callbacks: usize,
}

/// The immutable SQL route for one catalog. Lowering failures are retained in source order and
/// fail the apply before registration; they never select another executor.
pub(super) struct SqlPlans {
    plan: Result<relational::SqlCatalogPlan, String>,
    clause_keys: BTreeMap<ClauseId, (String, String)>,
    #[cfg(test)]
    failure_injection: std::sync::Mutex<Option<(usize, String)>>,
}

impl SqlPlans {
    pub(super) fn compile(code_rules: &[Arc<CompiledCodeRule>]) -> Self {
        let clause_keys = code_rules
            .iter()
            .enumerate()
            .flat_map(|(rule_index, rule)| {
                rule.clauses
                    .iter()
                    .enumerate()
                    .map(move |(clause_index, clause)| {
                        (
                            (rule_index, clause_index),
                            (rule.id.clone(), clause.id.clone()),
                        )
                    })
            })
            .collect();
        Self {
            plan: relational::SqlCatalogPlan::compile(code_rules).map_err(|error| {
                format!(
                    "SQL relational lowering failed: {}",
                    error.unsupported().join("; ")
                )
            }),
            clause_keys,
            #[cfg(test)]
            failure_injection: std::sync::Mutex::new(None),
        }
    }

    #[cfg(test)]
    pub(super) fn inject_failure(&self, producer: usize, code: &str) {
        *self.failure_injection.lock().unwrap() = Some((producer, code.to_owned()));
    }

    pub(super) fn verify_lowered(&self) -> Result<(), String> {
        self.plan.as_ref().map_err(Clone::clone)?.verify()
    }

    /// Executes a completed relational catalog plan. Phase 1 creates the plan but does not yet
    /// pack its terminal descriptors into the expanded virtual-table relation.
    pub(super) fn execute(&self, files: Vec<PackedFileInput>) -> Result<QueryOutcome, String> {
        self.execute_with_statement(files, None)
    }

    #[cfg(test)]
    pub(super) fn execute_statement(
        &self,
        files: Vec<PackedFileInput>,
        sql: &str,
    ) -> Result<QueryOutcome, String> {
        self.execute_with_statement(files, Some(sql))
    }

    fn execute_with_statement(
        &self,
        files: Vec<PackedFileInput>,
        statement: Option<&str>,
    ) -> Result<QueryOutcome, String> {
        const MAX_SQL_STATEMENT_BYTES: usize = 8 * 1024 * 1024;

        let plan = self.plan.as_ref().map_err(Clone::clone)?;
        let all_clause_keys = || {
            (0..plan.clause_count())
                .map(|ordinal| {
                    let (rule, clause) = plan
                        .clause_output_key(ordinal)
                        .ok_or_else(|| "SQL plan has no output key for a clause".to_owned())?;
                    Ok(serde_json::json!({"ruleId": rule, "clauseId": clause}))
                })
                .collect::<Result<Vec<_>, String>>()
        };
        let statements = match statement {
            Some(statement) => vec![ExecutionStatement {
                sql: statement.to_owned(),
                clauses: serde_json::Value::Array(all_clause_keys()?).to_string(),
                materialization: None,
            }],
            None => {
                let mut statements = Vec::new();
                for ordinals in plan::exact_scope_groups(&files, plan.clause_logical_plans())? {
                    let packets = split_measured_statement_packets(
                        &ordinals,
                        MAX_SQL_STATEMENT_BYTES,
                        |packet| {
                            plan.execution_sql_for_clauses(packet)
                                .map(|plan| plan::workspace_execution_sql(&plan.query))
                        },
                    )?;
                    for (packet, _) in packets {
                        let rendered = plan.execution_sql_for_clauses(&packet)?;
                        for (table, sql) in rendered.materializations {
                            statements.push(ExecutionStatement {
                                sql,
                                clauses: "[]".to_owned(),
                                materialization: Some(table),
                            });
                        }
                        let sql = plan::workspace_execution_sql(&rendered.query);
                        let clauses = packet
                            .iter()
                            .map(|ordinal| {
                                let (rule, clause) = plan
                                    .clause_output_key(*ordinal)
                                    .expect("verified packet ordinal has an output key");
                                serde_json::json!({"ruleId": rule, "clauseId": clause})
                            })
                            .collect();
                        statements.push(ExecutionStatement {
                            sql,
                            clauses: serde_json::Value::Array(clauses).to_string(),
                            materialization: None,
                        });
                    }
                }
                statements
            }
        };
        if statements.is_empty() {
            return Err("SQL catalog produced no exact-scope statements".to_owned());
        }
        for statement in &statements {
            if statement.sql.len() > MAX_SQL_STATEMENT_BYTES {
                return Err(format!(
                    "relational SQL exceeds bounded statement capacity: {} bytes > {MAX_SQL_STATEMENT_BYTES} bytes",
                    statement.sql.len()
                ));
            }
        }
        let programs = plan
            .terminals()
            .iter()
            .map(|terminal| terminal.program().clone())
            .collect::<Vec<_>>();
        let result_limits = plan
            .terminals()
            .iter()
            .map(relational::TerminalPlan::result_limit)
            .collect::<Vec<_>>();
        let mut tapes = Vec::with_capacity(files.len());
        let mut anchors = Vec::with_capacity(files.len());
        let mut task_grid = Vec::with_capacity(files.len());
        let mut document_maps = Vec::new();
        let mut file_paths = Vec::new();
        let mut file_metadata = Vec::new();
        for file in files {
            let mut file_anchors = Vec::with_capacity(programs.len());
            let mut file_tasks = Vec::with_capacity(programs.len());
            let mut syntax_clauses = std::collections::BTreeSet::new();
            for terminal in plan.terminals() {
                let mut roots = terminal
                    .clauses()
                    .iter()
                    .filter_map(|clause| file.tasks.get(clause))
                    .flatten()
                    .copied()
                    .collect::<Vec<_>>();
                roots.sort_unstable();
                roots.dedup();
                let demanded = (terminal.producer_kind() == ProducerKind::FileFact
                    || terminal.dialect() == file.language)
                    && terminal.applies_to_file(
                        &file.path,
                        std::str::from_utf8(file.tape.transducer_source()).unwrap_or_default(),
                        file.module.as_ref(),
                    )
                    && terminal
                        .clauses()
                        .iter()
                        .any(|clause| file.tasks.contains_key(clause));
                if !demanded {
                    roots.clear();
                } else if file.syntax_failure.is_some()
                    && terminal.producer_kind() != ProducerKind::FileFact
                {
                    syntax_clauses.extend(
                        terminal
                            .clauses()
                            .iter()
                            .filter(|clause| file.tasks.contains_key(clause))
                            .copied(),
                    );
                }
                if demanded && terminal.producer_kind() == ProducerKind::Navigation {
                    roots.clear();
                    roots.push(0);
                } else if matches!(
                    terminal.producer_kind(),
                    ProducerKind::FileFact | ProducerKind::TransformFact
                ) {
                    roots.clear();
                }
                file_tasks.push(demanded);
                file_anchors.push(roots);
            }
            document_maps.extend(file.document_maps);
            let source = std::str::from_utf8(file.tape.transducer_source())
                .map_err(|error| format!("packed source is not UTF-8: {error}"))?;
            let syntax_failures = syntax_clauses
                .into_iter()
                .filter_map(|clause| {
                    self.clause_keys.get(&clause).map(|(rule_id, clause_id)| {
                        serde_json::json!({
                            "ruleId": rule_id,
                            "clauseId": clause_id,
                            "message": file.syntax_failure.as_deref(),
                        })
                    })
                })
                .collect::<Vec<_>>();
            file_metadata.push(serde_json::json!({
                "path": file.path,
                "source": source,
                "lineStarts": LineTable::new(source, Terminators::of(file.language)).starts(),
                "syntaxFailures": syntax_failures,
            }));
            file_paths.push(file.path);
            tapes.push(file.tape);
            anchors.push(file_anchors);
            task_grid.push(file_tasks);
        }
        let producer_kinds = plan
            .terminals()
            .iter()
            .map(relational::TerminalPlan::producer_kind)
            .collect();
        let value_facts = plan
            .terminals()
            .iter()
            .map(relational::TerminalPlan::value_fact)
            .collect();
        let semantic_facts = plan
            .terminals()
            .iter()
            .map(relational::TerminalPlan::semantic_fact)
            .collect();
        let transform_facts = plan
            .terminals()
            .iter()
            .map(relational::TerminalPlan::transform_fact)
            .collect();
        let file_facts = plan
            .terminals()
            .iter()
            .map(relational::TerminalPlan::file_fact)
            .collect();
        let parse_facts = plan
            .terminals()
            .iter()
            .map(relational::TerminalPlan::parse_fact)
            .collect();
        let output_order = (0..plan.clause_count())
            .filter_map(|ordinal| {
                plan.clause_output_key(ordinal)
                    .map(|(rule, clause)| ((rule.to_owned(), clause.to_owned()), ordinal))
            })
            .collect::<BTreeMap<_, _>>();
        let file_order = file_paths
            .iter()
            .enumerate()
            .map(|(ordinal, path)| (path.clone(), ordinal))
            .collect::<BTreeMap<_, _>>();
        let demand = PackedDemand::new(
            production_backend(),
            programs,
            tapes,
            anchors,
            task_grid,
            result_limits,
        )
        .and_then(|demand| {
            demand.with_sound_under_errors(
                plan.terminals()
                    .iter()
                    .map(relational::TerminalPlan::sound_under_errors)
                    .collect(),
            )
        })
        .and_then(|demand| demand.with_producer_kinds(producer_kinds))
        .and_then(|demand| demand.with_value_facts(value_facts))
        .and_then(|demand| demand.with_semantic_facts(semantic_facts))
        .and_then(|demand| demand.with_parse_facts(parse_facts))
        .and_then(|demand| demand.with_transform_facts(transform_facts))
        .and_then(|demand| demand.with_file_facts(file_facts, file_paths))
        .and_then(|demand| demand.with_document_maps(document_maps, plan.requires_document_maps()))
        .map_err(|error| format!("packed batch verification failed: {error:?}"))?;
        #[cfg(test)]
        let demand = match self.failure_injection.lock().unwrap().take() {
            Some((producer, code)) => demand
                .with_failure_injection(producer, code)
                .map_err(|error| format!("packed failure injection failed: {error:?}"))?,
            None => demand,
        };
        let mut outcome = drive_sql(
            demand,
            &statements,
            &serde_json::Value::Array(file_metadata).to_string(),
        )
        .map_err(|error| error.to_string())?;
        outcome.violations.sort_by(|left, right| {
            output_order
                .get(&(left.rule_id.clone(), left.clause_id.clone()))
                .cmp(&output_order.get(&(right.rule_id.clone(), right.clause_id.clone())))
                .then_with(|| file_order.get(&left.path).cmp(&file_order.get(&right.path)))
                .then_with(|| left.stable_order.cmp(&right.stable_order))
        });
        outcome.failures.sort_by(|left, right| {
            output_order
                .get(&(left.rule_id.clone(), left.clause_id.clone()))
                .cmp(&output_order.get(&(right.rule_id.clone(), right.clause_id.clone())))
                .then_with(|| file_order.get(&left.path).cmp(&file_order.get(&right.path)))
                .then_with(|| left.code.cmp(&right.code))
        });
        Ok(outcome)
    }
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

struct ExecutionStatement {
    sql: String,
    clauses: String,
    materialization: Option<String>,
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

#[cfg(test)]
fn drive(demand: PackedDemand) -> rusqlite::Result<QueryOutcome> {
    drive_sql(
        demand,
        &[ExecutionStatement {
            sql: plan::legacy_test_scan_sql().to_owned(),
            clauses: "[]".to_owned(),
            materialization: None,
        }],
        "[]",
    )
}

fn drive_sql(
    demand: PackedDemand,
    statements: &[ExecutionStatement],
    file_metadata: &str,
) -> rusqlite::Result<QueryOutcome> {
    let slot = CONNECTION.with(Arc::clone);
    let mut slot = slot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if slot.is_none() {
        *slot = Some(QueryConnection::open()?);
    }
    slot.as_ref()
        .expect("opened connection")
        .drive(demand, statements, file_metadata)
}

struct QueryConnection {
    connection: Connection,
    registry: Arc<MatchRegistry>,
}

impl QueryConnection {
    fn open() -> rusqlite::Result<Self> {
        let connection = Connection::open_in_memory()?;
        let registry = Arc::new(MatchRegistry::default());
        register_module(&connection, Arc::clone(&registry))?;
        Ok(Self {
            connection,
            registry,
        })
    }

    fn drive(
        &self,
        demand: PackedDemand,
        statements: &[ExecutionStatement],
        file_metadata: &str,
    ) -> rusqlite::Result<QueryOutcome> {
        let token = self.registry.register(demand);
        let mut materialized = Vec::new();
        let result = (|| {
            let mut rows = Vec::new();
            let mut violations = Vec::new();
            let mut failures = Vec::new();
            for execution in statements {
                if let Some(table) = &execution.materialization {
                    self.connection
                        .execute_batch(&format!("DROP TABLE IF EXISTS \"{table}\""))?;
                    let mut statement = self.connection.prepare(&execution.sql)?;
                    statement.execute(rusqlite::named_params! {
                        ":batch": token,
                        ":files": file_metadata,
                    })?;
                    materialized.push(table.clone());
                    continue;
                }
                let mut statement = self.connection.prepare_cached(&execution.sql)?;
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
            let packed = self
                .registry
                .query_execution_report(token)
                .expect("the registered batch remains available until publication");
            Ok(QueryOutcome {
                counters: QueryCounters {
                    statements: statements.len(),
                    rows: row_count,
                    scalar_callbacks: 0,
                    node_rows: 0,
                    environments: 0,
                    fact_builds: 0,
                    fact_hits: 0,
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
        })();
        self.connection.flush_prepared_statement_cache();
        for table in materialized.into_iter().rev() {
            let _ = self
                .connection
                .execute_batch(&format!("DROP TABLE IF EXISTS \"{table}\""));
        }
        self.registry.remove_batch(token);
        result
    }
}

/// One thread's SQLite connection. Workspace eviction can release every idle connection.
type Slot = Arc<std::sync::Mutex<Option<QueryConnection>>>;
type SlotRegistry = Vec<std::sync::Weak<std::sync::Mutex<Option<QueryConnection>>>>;

static SLOTS: std::sync::LazyLock<std::sync::Mutex<SlotRegistry>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(Vec::new()));

thread_local! {
    static CONNECTION: Slot = {
        let slot = Slot::default();
        SLOTS
            .lock()
            .expect("query connection registry poisoned")
            .push(Arc::downgrade(&slot));
        slot
    };
}

pub(super) fn release_connections() {
    let mut slots = SLOTS.lock().expect("query connection registry poisoned");
    slots.retain(|slot| {
        let Some(slot) = slot.upgrade() else {
            return false;
        };
        if let Ok(mut connection) = slot.try_lock() {
            connection.take();
        }
        true
    });
}

/// A regex clause's ordered byte ranges, bounded by `limit` and the active deadline.
pub(in crate::code_rule_dsl) fn regex_matches(
    regex: &regex::Regex,
    haystack: &str,
    limit: usize,
) -> (Vec<std::ops::Range<usize>>, bool) {
    let mut matches = Vec::new();
    for found in regex.find_iter(haystack) {
        if crate::code_rule_dsl::budget::matcher_budget_exhausted() || matches.len() >= limit {
            return (matches, false);
        }
        matches.push(found.range());
    }
    (matches, true)
}

#[cfg(test)]
mod policy_lowering_tests;
#[cfg(test)]
mod production_contract_tests;
#[cfg(test)]
mod tests;
