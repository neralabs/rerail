//! SQL-owned workspace execution.
//!
//! The compiler lowers a catalog to verified packed programs. The workspace supplies immutable
//! tapes and a sparse task grid. SQLite demands that batch through `rerail_matches(:batch)`;
//! `xFilter` executes it once, and the materialized SQL relation performs completion filtering and
//! deterministic extraction. No host callback or semantic matcher is an execution dependency.

pub(in crate::code_rule_dsl) mod crossfile;
pub(in crate::code_rule_dsl) mod select_when;

use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::model::{Anchor, ClauseKind, CompiledCodeRule, Query};
use crate::code_rule_dsl::packed::{ExecutionBackend, Program};
use crate::code_rule_dsl::tape::TapeDoc;
use crate::code_rule_dsl::transducer::sql_backend::{
    MatchRegistry, PackedDemand, TerminalRow, register_module,
};
use rusqlite::Connection;
use std::collections::BTreeMap;
use std::sync::Arc;

type ClauseId = (usize, usize);

#[derive(Clone)]
struct PackedProgramPlan {
    clause: ClauseId,
    language: GrammarDialect,
    program: Program,
    result_limit: u32,
}

/// One file's immutable input to the packed batch. `tasks` contains only demanded clauses; an
/// empty roots vector is a demanded task with a complete negative result.
pub(super) struct PackedFileInput {
    pub(super) tape: TapeDoc,
    pub(super) language: GrammarDialect,
    pub(super) tasks: BTreeMap<ClauseId, Vec<u32>>,
}

/// Rows returned by SQL after completion filtering and stable ordering.
pub(super) struct QueryOutcome {
    pub(super) rows: Vec<TerminalRow>,
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
    programs: Result<Vec<PackedProgramPlan>, String>,
}

impl SqlPlans {
    pub(super) fn compile(code_rules: &[Arc<CompiledCodeRule>]) -> Self {
        Self {
            programs: lower_catalog(code_rules),
        }
    }

    pub(super) fn verify_lowered(&self) -> Result<(), String> {
        self.programs.as_ref().map(|_| ()).map_err(Clone::clone)
    }

    /// Builds one immutable sparse batch and executes the one SQL statement that owns it.
    pub(super) fn execute(&self, files: Vec<PackedFileInput>) -> Result<QueryOutcome, String> {
        let plans = self.programs.as_ref().map_err(Clone::clone)?;
        let programs = plans
            .iter()
            .map(|plan| plan.program.clone())
            .collect::<Vec<_>>();
        let result_limits = plans
            .iter()
            .map(|plan| plan.result_limit)
            .collect::<Vec<_>>();
        let mut tapes = Vec::with_capacity(files.len());
        let mut anchors = Vec::with_capacity(files.len());
        let mut task_grid = Vec::with_capacity(files.len());
        for file in files {
            let mut file_anchors = Vec::with_capacity(plans.len());
            let mut demanded = Vec::with_capacity(plans.len());
            for plan in plans {
                let roots = (plan.language == file.language)
                    .then(|| file.tasks.get(&plan.clause))
                    .flatten();
                demanded.push(roots.is_some());
                file_anchors.push(roots.cloned().unwrap_or_default());
            }
            tapes.push(file.tape);
            anchors.push(file_anchors);
            task_grid.push(demanded);
        }
        let demand = PackedDemand::new(
            production_backend(),
            programs,
            tapes,
            anchors,
            task_grid,
            result_limits,
        )
        .map_err(|error| format!("packed batch verification failed: {error:?}"))?;
        drive(demand).map_err(|error| error.to_string())
    }
}

fn lower_catalog(code_rules: &[Arc<CompiledCodeRule>]) -> Result<Vec<PackedProgramPlan>, String> {
    let mut programs = Vec::new();
    for (rule_index, code_rule) in code_rules.iter().enumerate() {
        for (clause_index, clause) in code_rule.clauses.iter().enumerate() {
            let ClauseKind::Forbid(Query::Ast { matchers, calls }) = &clause.kind else {
                return Err(format!(
                    "{} / {}: packed lowering does not support this clause relation",
                    code_rule.id, clause.id
                ));
            };
            let violation = &clause.violation;
            if !calls.is_empty()
                || clause.synthetic
                || clause.decision
                || violation.anchor != Anchor::Span
                || violation.message_template.is_some()
                || violation.fix.is_some()
                || !violation.node_regexes.is_empty()
                || violation.each.is_some()
                || violation.fix_range.is_some()
                || violation.suppress.is_some()
                || violation.group_transform.is_some()
                || !violation.project.is_empty()
            {
                return Err(format!(
                    "{} / {}: packed lowering does not support this clause output contract",
                    code_rule.id, clause.id
                ));
            }
            let result_limit =
                u32::try_from(code_rule.capabilities.limits.max_witnesses).map_err(|_| {
                    format!(
                        "{} / {}: packed result limit exceeds u32",
                        code_rule.id, clause.id
                    )
                })?;
            for matcher in matchers {
                if matcher.file.is_some()
                    || !matcher.applies
                    || matcher.three_valued
                    || matcher.transform.is_some()
                {
                    return Err(format!(
                        "{} / {} / {:?}: packed lowering does not support this matcher contract",
                        code_rule.id,
                        clause.id,
                        crate::code_rule_dsl::language::label(matcher.language)
                    ));
                }
                let program = Program::lower_matcher(&matcher.matcher).map_err(|error| {
                    format!(
                        "{} / {} / {:?}: packed lowering failed: {error:?}",
                        code_rule.id,
                        clause.id,
                        crate::code_rule_dsl::language::label(matcher.language)
                    )
                })?;
                programs.push(PackedProgramPlan {
                    clause: (rule_index, clause_index),
                    language: matcher.language,
                    program,
                    result_limit,
                });
            }
        }
    }
    Ok(programs)
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

fn drive(demand: PackedDemand) -> rusqlite::Result<QueryOutcome> {
    let slot = CONNECTION.with(Arc::clone);
    let mut slot = slot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if slot.is_none() {
        *slot = Some(QueryConnection::open()?);
    }
    slot.as_ref().expect("opened connection").drive(demand)
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

    fn drive(&self, demand: PackedDemand) -> rusqlite::Result<QueryOutcome> {
        let token = self.registry.register(demand);
        let result = (|| {
            let mut statement = self.connection.prepare_cached(
                "WITH terminal AS MATERIALIZED (\
                   SELECT batch, file_index, program_index, match_index, start_ordinal, \
                          output_label, capture_index, capture_label, capture_node, start_byte, \
                          end_byte, complete, error_code \
                   FROM rerail_matches WHERE batch = ?1\
                 ), complete_pairs AS MATERIALIZED (\
                   SELECT file_index, program_index, min(complete) AS complete \
                   FROM terminal GROUP BY file_index, program_index\
                 ), extracted AS MATERIALIZED (\
                   SELECT t.* FROM terminal t \
                   JOIN complete_pairs c USING (file_index, program_index) \
                   WHERE t.match_index IS NULL OR c.complete = 1\
                 ) \
                 SELECT batch, file_index, program_index, match_index, start_ordinal, \
                        output_label, capture_index, capture_label, capture_node, start_byte, \
                        end_byte, complete, error_code \
                 FROM extracted \
                 ORDER BY file_index, program_index, match_index, capture_index",
            )?;
            let rows = statement
                .query_map([token], TerminalRow::from_sql_row)?
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
mod lowering_contract;
#[cfg(test)]
mod tests;
