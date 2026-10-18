//! SQL-owned workspace execution.
#![expect(
    dead_code,
    reason = "the packed lowering layer calls this boundary; the deleted matcher adapter must not keep it artificially live"
)]
//!
//! The host registers one immutable packed batch. SQLite opens `rerail_matches(batch)`, whose
//! `xFilter` executes that batch once and materializes its typed relation. No host execution
//! callback, semantic matcher, scalar callback, node-walk table, or nested statement exists.

pub(in crate::code_rule_dsl) mod crossfile;
pub(in crate::code_rule_dsl) mod select_when;

use crate::code_rule_dsl::transducer::sql_backend::{MatchRegistry, PackedDemand, register_module};
use rusqlite::Connection;
use std::sync::Arc;

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

/// The immutable SQL route for a catalog. Rule compilation remains in Rerail's verified compiler.
pub(super) struct SqlPlans;

impl SqlPlans {
    pub(super) fn compile(
        _code_rules: &[Arc<crate::code_rule_dsl::model::CompiledCodeRule>],
    ) -> Self {
        Self
    }

    /// Executes one inert batch only when SQLite opens its virtual table.
    pub(super) fn drive(&self, demand: PackedDemand) -> rusqlite::Result<QueryCounters> {
        let slot = CONNECTION.with(Arc::clone);
        let mut slot = slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if slot.is_none() {
            *slot = Some(QueryConnection::open()?);
        }
        slot.as_ref().expect("opened connection").drive(demand)
    }
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

    fn drive(&self, demand: PackedDemand) -> rusqlite::Result<QueryCounters> {
        let token = self.registry.register(demand);
        let result = (|| {
            let mut statement = self.connection.prepare_cached(
                "WITH terminal AS MATERIALIZED (\
                   SELECT * FROM rerail_matches WHERE batch = ?1\
                 ) SELECT file_index, program_index, match_index, complete, error_code \
                   FROM terminal ORDER BY file_index, program_index, match_index",
            )?;
            let mut rows = statement.query([token])?;
            let mut count = 0;
            while rows.next()?.is_some() {
                count += 1;
            }
            let (terminal_executions, virtual_table_filters) = self
                .registry
                .query_execution_counters(token)
                .unwrap_or_default();
            Ok(QueryCounters {
                statements: 1,
                rows: count,
                scalar_callbacks: 0,
                node_rows: 0,
                environments: 0,
                fact_builds: 0,
                fact_hits: 0,
                terminal_executions,
                virtual_table_filters,
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
mod tests;
