//! Output ownership and transitive invalidation. Persistent user tables are never owned here.
use super::*;
pub mod statistics;
pub mod planning;
pub use planning::normalize_views;
pub use statistics::Statistics;

#[derive(Clone)]
pub struct Execution {
    pub inputs: BTreeSet<String>,
    pub objects: Vec<(String, bool)>,
    pub emitted: Arc<plan::emit::Emitted>,
    /// Physical arities at the compiler boundary, before executor-private walk lowering.
    pub logical_relations: Arc<Vec<(String,usize)>>,
    /// Logical component proofs are built once, before physical helper lowering.
    pub loop_analyses: Arc<Vec<Value>>,
    /// The program as declared (pruned, before lowering), for the catalog.
    pub program: Arc<Version>,
    pub planning_digest: Option<String>,
    /// Relations made by SQL producers, each with the session inputs its query reads. Tuple
    /// maintenance recomputes a producer's stratum when what it reads changes.
    pub producers: BTreeMap<String, Vec<String>>,
    pub counts: BTreeMap<String, u64>,
    pub outputs: Vec<String>,
    pub max_waves: u32,
    pub reductions: Option<reductions::pool::Plans>,
    pub recursive: BTreeMap<u32, recursive::Partitioned>,
    /// The retained traces, by stratum (`temporal::Trace`, rerail-sessions').
    pub temporal: BTreeMap<u32, Box<dyn Trace>>,
}

/// A stratum's retained trace (`temporal::Trace`, rerail-sessions'), as the executor's parts below it read it: they
/// replay it over a change, and read what it holds and how it plans. The traces' own code reads its type
/// (`temporal::Trace::of`).
pub trait Trace: Send + Sync {
    fn clone_box(&self) -> Box<dyn Trace>;
    /// Replays the trace over `changes` (at most `limit` waves): each relation's delta, and the waves it ran.
    fn apply(
        &mut self,
        w: &mut scratch::Work<'_>,
        store: &versions::Store,
        changes: &BTreeMap<String, maintenance::Delta>,
        limit: u32,
    ) -> Result<(BTreeMap<String, maintenance::Delta>, u32)>;
    /// The objects it owns: each name, and whether it is a table.
    fn objects(&self) -> Box<dyn Iterator<Item = (String, bool)> + '_>;
    fn planning_statistics(&self) -> Option<Value>;
    fn planning_strategy(&self) -> super::planning::Strategy;
    /// Whether it is the whole program's, in phases that stay as they are.
    fn stable_phases(&self, program: &super::super::waves::schedule::Program) -> bool;
    fn explain(&self) -> Value;
    fn frames(&self) -> usize;
    fn as_any(&self) -> &dyn std::any::Any;
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any>;
}

impl Clone for Box<dyn Trace> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

// `impl CodeRuleSqlExecutor` (its executions: `affected`, `invalidate`, `remove_executions`, ...) is the executor's
// (`executor.rs`, rerail-sessions').

pub fn index(
    db: &rusqlite::Connection,
    relation: &str,
    columns: &[u32],
) -> rusqlite::Result<()> {
    let count = db
        .prepare(&format!("SELECT * FROM {} LIMIT 0", quote(relation)))?
        .column_count();
    if columns.is_empty()
        || columns.iter().any(|i| *i as usize >= count)
        || columns.iter().collect::<BTreeSet<_>>().len() != columns.len()
    {
        return Err(rusqlite::Error::InvalidParameterName(
            "index columns must be distinct valid positions".into(),
        ));
    }
    if let Some(source) = super::super::facts::vtab::bound_source(db, relation) {
        source.prepare_index(&columns.iter().map(|c| *c as usize).collect::<Vec<_>>());
        return Ok(());
    }
    let suffix = columns
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("_");
    let name = format!("__sql_index_{relation}_{suffix}");
    let cols = columns
        .iter()
        .map(|i| format!("c{i}"))
        .collect::<Vec<_>>()
        .join(",");
    db.execute_batch(&format!(
        "CREATE INDEX IF NOT EXISTS temp.{} ON {}({cols})",
        quote(&name),
        quote(relation)
    ))
}

// `impl Drop for CodeRuleSqlExecutor` is the executor's (`executor.rs`).
