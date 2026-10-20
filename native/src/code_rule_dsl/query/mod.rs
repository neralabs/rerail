//! SQL-owned workspace execution.
//!
//! The compiler lowers a catalog to verified packed programs. The workspace supplies immutable
//! tapes and a sparse task grid. SQLite demands that batch through `rerail_matches(:batch)`;
//! `xFilter` executes it once, and the materialized SQL relation performs completion filtering and
//! deterministic extraction. No host callback or semantic matcher is an execution dependency.

pub(in crate::code_rule_dsl) mod crossfile;
mod relational;
mod relational_ir;
pub(in crate::code_rule_dsl) mod select_when;

use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::model::CompiledCodeRule;
use crate::code_rule_dsl::packed::ExecutionBackend;
use crate::code_rule_dsl::tape::TapeDoc;
use crate::code_rule_dsl::transducer::sql_backend::{
    DocumentMapSpan, MatchRegistry, PackedDemand, PackedRow, ProducerKind, register_module,
};
use rusqlite::Connection;
use std::collections::BTreeMap;
use std::sync::Arc;

type ClauseId = (usize, usize);

/// One file's immutable input to the packed batch. `tasks` contains only demanded clauses; an
/// empty roots vector is a demanded task with a complete negative result.
pub(super) struct PackedFileInput {
    pub(super) tape: TapeDoc,
    pub(super) language: GrammarDialect,
    pub(super) path: String,
    pub(super) tasks: BTreeMap<ClauseId, Vec<u32>>,
    pub(super) document_maps: Vec<DocumentMapSpan>,
}

/// Rows returned by SQL after completion filtering and stable ordering.
pub(super) struct QueryOutcome {
    pub(super) rows: Vec<PackedRow>,
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
}

/// The immutable SQL route for one catalog. Lowering failures are retained in source order and
/// fail the apply before registration; they never select another executor.
pub(super) struct SqlPlans {
    plan: Result<relational::SqlCatalogPlan, String>,
}

impl SqlPlans {
    pub(super) fn compile(code_rules: &[Arc<CompiledCodeRule>]) -> Self {
        Self {
            plan: relational::SqlCatalogPlan::compile(code_rules).map_err(|error| {
                format!(
                    "SQL relational lowering failed: {}",
                    error.unsupported().join("; ")
                )
            }),
        }
    }

    pub(super) fn verify_lowered(&self) -> Result<(), String> {
        self.plan.as_ref().map_err(Clone::clone)?.verify()
    }

    /// Executes a completed relational catalog plan. Phase 1 creates the plan but does not yet
    /// pack its terminal descriptors into the expanded virtual-table relation.
    pub(super) fn execute(&self, files: Vec<PackedFileInput>) -> Result<QueryOutcome, String> {
        let plan = self.plan.as_ref().map_err(Clone::clone)?;
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
        for file in files {
            let mut file_anchors = Vec::with_capacity(programs.len());
            let mut file_tasks = Vec::with_capacity(programs.len());
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
                let demanded = terminal.dialect() == file.language
                    && terminal.applies_to_file(
                        &file.path,
                        std::str::from_utf8(file.tape.transducer_source()).unwrap_or_default(),
                    )
                    && terminal
                        .clauses()
                        .iter()
                        .any(|clause| file.tasks.contains_key(clause));
                if demanded && terminal.producer_kind() == ProducerKind::Navigation {
                    roots.clear();
                    roots.push(0);
                } else if terminal.producer_kind() == ProducerKind::FileFact {
                    roots.clear();
                }
                file_tasks.push(demanded);
                file_anchors.push(roots);
            }
            document_maps.extend(file.document_maps);
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
        .and_then(|demand| demand.with_file_facts(file_facts, file_paths))
        .and_then(|demand| demand.with_document_maps(document_maps, plan.requires_document_maps()))
        .map_err(|error| format!("packed batch verification failed: {error:?}"))?;
        drive_sql(demand, plan.sql()).map_err(|error| error.to_string())
    }
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
        "SELECT batch, row_kind, producer_id, file_index, task_index, match_index, capture_index, capture_occurrence, capture_sequence_start, capture_sequence_count, node, related_node, axis_kind, axis_order, fact_kind, document_id, mapped_document_id, start_ordinal, output_label, capture_label, text_value, start_byte, end_byte, verdict, complete, work_used, error_code, field_id, related_named, source_text, structural_key, named_leaf, capture_kind, capture_named, equality_occurrence FROM rerail_matches WHERE batch = ?1 ORDER BY rowid",
    )
}

fn drive_sql(demand: PackedDemand, sql: &str) -> rusqlite::Result<QueryOutcome> {
    let slot = CONNECTION.with(Arc::clone);
    let mut slot = slot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if slot.is_none() {
        *slot = Some(QueryConnection::open()?);
    }
    slot.as_ref().expect("opened connection").drive(demand, sql)
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

    fn drive(&self, demand: PackedDemand, sql: &str) -> rusqlite::Result<QueryOutcome> {
        let token = self.registry.register(demand);
        let result = (|| {
            let mut statement = self.connection.prepare_cached(sql)?;
            let rows = statement
                .query_map([token], PackedRow::from_sql_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let (terminal_executions, virtual_table_filters) = self
                .registry
                .query_execution_counters(token)
                .unwrap_or_default();
            Ok(QueryOutcome {
                counters: QueryCounters {
                    statements: 1,
                    rows: rows.len(),
                    scalar_callbacks: 0,
                    node_rows: 0,
                    environments: 0,
                    fact_builds: 0,
                    fact_hits: 0,
                    terminal_executions,
                    virtual_table_filters,
                },
                rows,
            })
        })();
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
