//! Compiled plans, the statistics their decisions read, run-time feedback and SQLite's own
//! statistics (docs/final-shape/03 §6, 02 "stat-driven compiler"; 05 Phase 3 row 8). Written new:
//! no source has these. The rules come from PIAT spec §4.9 and §11 (notes/t1-piat-spec.md):
//! - **`compiled_plans`** keeps each plan under its identity (`identity::CompiledPlanId`, N's
//!   `SemanticId` in its own domain) with its decisions and the program they produced.
//!   Every statistic a decision read is stored with its validity interval: the log2 bucket of the
//!   value read (§4.9's "log2 buckets of every statistic read"). A plan stays valid while every
//!   statistic stays in its bucket, so a refresh re-plans only when a decision could change (§11
//!   :273), and nothing counts rows at run time (DLRE trap T18).
//! - **Validity is re-checked by one batched lookup** (§11 :283): [`Plans::valid`] reads the
//!   intervals of many plans in one statement and compares them with the values the caller read
//!   in its own one batched statement (`inputs::Statistics::reads`).
//! - **`plan_feedback`** holds, per plan and backend, an EWMA of the observed selectivity and cost
//!   per root (§2.5b, §4.8: "sel from `plan_feedback` EWMA, else the prior"). It also answers H4:
//!   a regex's matched-term count enters the estimate after its first fill.
//! - **`sqlite_stat1`** is written from `fact_stats` for fact relations held as SQL tables, never
//!   by `ANALYZE` over the data (02 "SQL join order"); `ANALYZE sqlite_schema` only creates the
//!   table and reloads the planner's copy.
//!
//! Compiled programs also live in memory, in the bounded cache copied from Data-layer-rule-
//! expressivity (`cache.rs`), keyed the same way; the store is their durable copy.

use super::cache::Cache;
use rusqlite::{Connection, OptionalExtension, Result, params};
use std::collections::{BTreeMap, btree_map};
use std::sync::Arc;

/// The version of the decisions and programs `compiled_plans` holds; a plan compiled by another is
/// not read.
pub(in crate::code_rule_dsl) const COMPILER_VERSION: i64 = 1;
/// Bytes of compiled programs kept in memory.
const MEMORY: usize = 16 * 1024 * 1024;
/// Weight of the newest observation in `plan_feedback`.
const FEEDBACK_WEIGHT: f64 = 0.25;

/// The bytes of a plan's `identity::CompiledPlanId`.
pub(in crate::code_rule_dsl) type PlanId = [u8; 32];

/// The interval of values over which a decision that read `value` stays valid: its log2 bucket.
pub(in crate::code_rule_dsl) fn interval(value: u64) -> (u64, u64) {
    if value == 0 {
        return (0, 0);
    }
    let low = 1u64 << value.ilog2();
    (low, low.saturating_sub(1).saturating_add(low))
}

/// A compiled plan: its decisions (with the statistic values they read, for `explain`) and the
/// program they produced.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::code_rule_dsl) struct Compiled {
    pub decisions: String,
    pub program: Vec<u8>,
}

struct Held {
    compiled: Arc<Compiled>,
    bounds: Vec<(String, u64, u64)>,
}

fn holds(bounds: &[(String, u64, u64)], current: &BTreeMap<String, u64>) -> bool {
    bounds.iter().all(|(stat, low, high)| current.get(stat).is_some_and(|value| (low..=high).contains(&value)))
}

/// Compiled plans, in memory and in the store.
pub(in crate::code_rule_dsl) struct Plans {
    memory: Cache<PlanId, Held>,
}

impl Default for Plans {
    fn default() -> Self {
        Self { memory: Cache::new(MEMORY) }
    }
}

impl Plans {
    /// The plans among `plans` whose stored decisions still hold for `current` (statistic →
    /// value, as read now), with their programs. One statement reads every stored interval.
    pub(in crate::code_rule_dsl) fn valid(
        &mut self,
        db: &Connection,
        plans: &[PlanId],
        current: &BTreeMap<String, u64>,
    ) -> Result<BTreeMap<PlanId, Arc<Compiled>>> {
        let mut valid = BTreeMap::new();
        let mut missing = Vec::new();
        for plan in plans {
            match self.memory.get(plan) {
                Some(entry) if holds(&entry.bounds, current) => {
                    valid.insert(*plan, Arc::clone(&entry.compiled));
                }
                Some(_) => {}
                None => missing.push(*plan),
            }
        }
        if missing.is_empty() {
            return Ok(valid);
        }
        let wanted = serde_json::to_string(&missing.iter().map(hex).collect::<Vec<_>>()).expect("hex strings serialise");
        let mut loaded = BTreeMap::<PlanId, Held>::new();
        let mut statement = db.prepare_cached(
            "SELECT p.plan, p.decisions, p.program, r.stat, r.lo, r.hi FROM json_each(?1) w \
             JOIN compiled_plans p ON p.plan = unhex(w.value) AND p.compiler = ?2 \
             LEFT JOIN compiled_plan_reads r ON r.plan = p.plan",
        )?;
        let mut rows = statement.query(params![wanted, COMPILER_VERSION])?;
        while let Some(row) = rows.next()? {
            let plan: PlanId = row.get(0)?;
            let entry = match loaded.entry(plan) {
                btree_map::Entry::Occupied(entry) => entry.into_mut(),
                btree_map::Entry::Vacant(entry) => entry.insert(Held {
                    compiled: Arc::new(Compiled { decisions: row.get(1)?, program: row.get(2)? }),
                    bounds: Vec::new(),
                }),
            };
            if let Some(stat) = row.get::<_, Option<String>>(3)? {
                entry.bounds.push((stat, row.get::<_, i64>(4)? as u64, row.get::<_, i64>(5)? as u64));
            }
        }
        for (plan, entry) in loaded {
            if holds(&entry.bounds, current) {
                valid.insert(plan, Arc::clone(&entry.compiled));
            }
            let bytes = entry.compiled.program.len() + entry.compiled.decisions.len();
            self.memory.insert(plan, entry, bytes);
        }
        Ok(valid)
    }

    /// Records `plan`, compiled from statistics `read` (statistic → value), replacing any earlier
    /// compilation.
    pub(in crate::code_rule_dsl) fn put(
        &mut self,
        db: &Connection,
        plan: PlanId,
        compiled: Compiled,
        read: &BTreeMap<String, u64>,
    ) -> Result<Arc<Compiled>> {
        db.execute(
            "INSERT OR REPLACE INTO compiled_plans(plan, compiler, decisions, program) VALUES (?1, ?2, ?3, ?4)",
            params![plan, COMPILER_VERSION, compiled.decisions, compiled.program],
        )?;
        db.execute("DELETE FROM compiled_plan_reads WHERE plan = ?1", [plan])?;
        let mut insert = db.prepare_cached("INSERT INTO compiled_plan_reads(plan, stat, lo, hi) VALUES (?1, ?2, ?3, ?4)")?;
        let mut bounds = Vec::with_capacity(read.len());
        for (stat, &value) in read {
            let (low, high) = interval(value);
            insert.execute(params![plan, stat, low as i64, high.min(i64::MAX as u64) as i64])?;
            bounds.push((stat.clone(), low, high));
        }
        let bytes = compiled.program.len() + compiled.decisions.len();
        let compiled = Arc::new(compiled);
        self.memory.insert(plan, Held { compiled: Arc::clone(&compiled), bounds }, bytes);
        Ok(compiled)
    }
}

fn hex(plan: &PlanId) -> String {
    plan.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// A plan's observed behaviour on one backend.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::code_rule_dsl) struct Feedback {
    pub selectivity: f64,
    pub ns_per_root: f64,
    pub samples: i64,
}

/// Folds one run of `plan` on `backend` into its feedback.
pub(in crate::code_rule_dsl) fn observe(db: &Connection, plan: PlanId, backend: &str, selectivity: f64, ns_per_root: f64) -> Result<()> {
    db.execute(
        "INSERT INTO plan_feedback(plan, backend, selectivity, ns_per_root, samples, updated_at) \
         VALUES (?1, ?2, ?3, ?4, 1, unixepoch()) \
         ON CONFLICT(plan, backend) DO UPDATE SET \
         selectivity = selectivity + ?5 * (excluded.selectivity - selectivity), \
         ns_per_root = ns_per_root + ?5 * (excluded.ns_per_root - ns_per_root), \
         samples = samples + 1, updated_at = excluded.updated_at",
        params![plan, backend, selectivity, ns_per_root, FEEDBACK_WEIGHT],
    )?;
    Ok(())
}

pub(in crate::code_rule_dsl) fn feedback(db: &Connection, plan: PlanId, backend: &str) -> Result<Option<Feedback>> {
    db.prepare_cached("SELECT selectivity, ns_per_root, samples FROM plan_feedback WHERE plan = ?1 AND backend = ?2")?
        .query_row(params![plan, backend], |row| {
            Ok(Feedback { selectivity: row.get(0)?, ns_per_root: row.get(1)?, samples: row.get(2)? })
        })
        .optional()
}

/// Writes `sqlite_stat1` rows for `indexes` ((table, index, columns)) from `fact_stats`, and makes
/// the planner read them. An index's k-th number is the rows an equality on its first k columns
/// selects: the smallest postings-weighted fanout (`sq / rows`) among them, at least 1.
pub(in crate::code_rule_dsl) fn write_stat1(db: &Connection, indexes: &[(&str, &str, &[&str])]) -> Result<()> {
    db.execute_batch("ANALYZE sqlite_schema")?;
    let mut column = db.prepare_cached("SELECT rows, sq FROM fact_stats WHERE tbl = ?1 AND col = ?2")?;
    for (table, index, columns) in indexes {
        let mut rows = 0;
        let mut stat = Vec::with_capacity(columns.len());
        let mut fanout = u64::MAX;
        for col in *columns {
            let (n, sq) = column
                .query_row(params![table, col], |row| Ok((row.get::<_, i64>(0)? as u64, row.get::<_, i64>(1)? as u64)))
                .optional()?
                .unwrap_or((0, 0));
            rows = rows.max(n);
            fanout = fanout.min(sq.div_ceil(n.max(1)).max(1));
            stat.push(fanout.to_string());
        }
        db.execute("DELETE FROM sqlite_stat1 WHERE tbl = ?1 AND idx = ?2", params![table, index])?;
        db.execute(
            "INSERT INTO sqlite_stat1(tbl, idx, stat) VALUES (?1, ?2, ?3)",
            params![table, index, format!("{} {}", rows.max(1), stat.join(" "))],
        )?;
    }
    // Reloads every table's statistics; reads no table but the schema.
    db.execute_batch("ANALYZE sqlite_schema")
}

#[cfg(test)]
mod tests;
