//! Output ownership and transitive invalidation. Persistent user tables are never owned here.
use super::*;
mod statistics;
mod planning;
pub(super) use planning::normalize_views;
pub(super) use statistics::Statistics;

#[derive(Clone)]
pub(super) struct Execution {
    pub inputs: BTreeSet<String>,
    pub objects: Vec<(String, bool)>,
    pub emitted: Arc<plan::emit::Emitted>,
    /// Physical arities at the compiler boundary, before executor-private walk lowering.
    pub logical_relations: Arc<Vec<(String,usize)>>,
    /// Logical component proofs are built once, before physical helper lowering.
    pub loop_analyses: Arc<Vec<Value>>,
    /// The program as declared (pruned, before lowering), for the catalog.
    pub program: Arc<front::Program>,
    pub planning_digest: Option<String>,
    /// Relations made by SQL producers, each with the session inputs its query reads. Tuple
    /// maintenance recomputes a producer's stratum when what it reads changes.
    pub producers: BTreeMap<String, Vec<String>>,
    pub counts: BTreeMap<String, u64>,
    pub outputs: Vec<String>,
    pub max_waves: u32,
    pub reductions: Option<reductions::pool::Plans>,
    pub recursive: BTreeMap<u32, recursive::Partitioned>,
    pub temporal: BTreeMap<u32, temporal::Trace>,
}

impl CodeRuleSqlExecutor {
    pub(super) fn affected(&self, changed: &BTreeSet<String>) -> BTreeSet<usize> {
        let mut changed = changed.clone();
        let mut affected = BTreeSet::new();
        loop {
            let before = affected.len();
            for (i, execution) in self.executions.iter().enumerate() {
                if !affected.contains(&i)
                    && (execution.inputs.iter().any(|input| changed.contains(input))
                        || execution
                            .objects
                            .iter()
                            .any(|(name, _)| changed.contains(name)))
                {
                    affected.insert(i);
                    changed.extend(execution.objects.iter().map(|(name, _)| name.clone()));
                }
            }
            if before == affected.len() {
                return affected;
            }
        }
    }

    pub(super) fn invalidate(&mut self, changed: &BTreeSet<String>) -> Result<()> {
        let affected = self.affected(changed);
        self.remove_executions(&affected)
    }

    pub(super) fn remove_executions(&mut self, affected: &BTreeSet<usize>) -> Result<()> {
        if affected.is_empty() {
            return Ok(());
        }
        let objects: BTreeSet<_> = affected
            .iter()
            .flat_map(|i| self.executions[*i].objects.iter().cloned())
            .collect();
        self.db.admission().require_owner(|| {
            self.control_drop_bound() + objects.iter().filter(|(n, _)| !self.versions.owns(n)).count()
        }).map_err(error)?;
        for table in [false, true] {
            for (name, is_table) in objects.iter().filter(|(_, is_table)| *is_table == table) {
                if self.versions.owns(name) {
                    self.versions.remove_all(&self.db, name)?;
                    continue;
                }
                self.db
                    .execute_batch(&format!(
                        "DROP {} IF EXISTS temp.{}",
                        if *is_table { "TABLE" } else { "VIEW" },
                        quote(name)
                    ))
                    .map_err(error)?;
            }
        }
        let mut at = 0;
        self.executions.retain(|_| {
            let keep = !affected.contains(&at);
            at += 1;
            keep
        });
        self.generated.retain(|object| !objects.contains(object));
        self.clear_control()?;
        self.db.reset_execution();
        self.refresh_catalog()
    }

    pub(super) fn result_index(&self, relation: &str, columns: &[u32]) -> Result<()> {
        if !self
            .generated
            .iter()
            .any(|(name, table)| name == relation && *table)
        {
            return Err(error("indexes require a session-owned result table"));
        }
        index(&self.db, relation, columns).map_err(error)
    }
    pub(super) fn output_objects(
        &self,
        emitted: &plan::emit::Emitted,
    ) -> Result<Vec<(String, bool)>> {
        let mut objects = Vec::new();
        let projections = emitted.views.iter().map(|(name,q)| (name.clone(),q.body.columns.len())).collect::<Vec<_>>();
        for (name, arity) in emitted.program.relations.iter().chain(&projections) {
            if (name.starts_with("__") && !projections.iter().any(|(r,_)|r==name)) || reserved(name) {
                return Err(error(format!("{name}: reserved SQL wave relation")));
            }
            if *arity == 0 {
                return Err(error("SQL wave relations need at least one column"));
            }
            objects.extend([
                (name.clone(), true),
                (format!("new_{name}"), true),
                (format!("delta_{name}"), true),
            ]);
            if emitted
                .program
                .lattice_operations
                .get(name)
                .is_some_and(|ops| ops.after.is_some())
            {
                objects.push((format!("__sql_changes_{name}"), true));
            }
        }
        let existing = self.db.prepare("SELECT lower(name) FROM sqlite_schema UNION SELECT lower(name) FROM temp.sqlite_schema")
            .map_err(error)?.query_map([], |r| r.get::<_, String>(0)).map_err(error)?
            .collect::<rusqlite::Result<BTreeSet<_>>>().map_err(error)?;
        let mut unique = BTreeSet::new();
        let owned: BTreeSet<_> = self
            .generated
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        for (name, _) in &objects {
            if !identifier(name)
                || name.to_ascii_lowercase().starts_with("wave_")
                || name.to_ascii_lowercase().starts_with("rdl_")
                || name.eq_ignore_ascii_case("atoms")
                || !unique.insert(name.to_ascii_lowercase())
            {
                return Err(error(format!(
                    "invalid or overlapping wave relation: {name}"
                )));
            }
            let exists = existing.contains(&name.to_ascii_lowercase());
            if (exists && !owned.contains(name.as_str()))
                || self
                    .inputs
                    .keys()
                    .any(|input| input.eq_ignore_ascii_case(name))
            {
                return Err(error(format!(
                    "{name}: SQL output already exists; rebind inputs to clear derived results"
                )));
            }
        }
        Ok(objects)
    }
    pub(super) fn control_drop_bound(&self) -> usize {
        // One view drop for native statistics, then one inventory and up to seven table drops.
        usize::from(self.statistics.snapshot().is_some()) + if self.control_ready.get() { 8 } else { 0 }
    }
    pub(super) fn clear_control(&self) -> Result<()> {
        self.statistics.clear(&self.db)?;
        self.stats_ready.set(false);
        if !self.control_ready.get() {
            return Ok(());
        }
        let names = self.db.prepare("SELECT name FROM temp.sqlite_schema WHERE type='table' AND name IN('wave_step','wave_read','wave_delta','wave_pending','wave_after','wave_document','wave_stats')")
            .map_err(error)?.query_map([], |r| r.get::<_, String>(0)).map_err(error)?
            .collect::<rusqlite::Result<Vec<_>>>().map_err(error)?;
        for name in names {
            self.db
                .execute_batch(&format!("DROP TABLE temp.{}", quote(&name)))
                .map_err(error)?;
        }
        self.control_ready.set(false);
        Ok(())
    }
}

pub(super) fn index(
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

impl Drop for CodeRuleSqlExecutor {
    fn drop(&mut self) {
        let _scope = profiling::scope(&self.db, "reset");
        self.db.profile_checkpoint();
        let _ = self.db.remove_bindings();
        if let Ok(_admission)=self.db.admission().enter("cleanup") {
            let _ = crate::content_store::atoms::Atoms::for_connection(&self.db).flush(&self.db);
        }
        self.db.profile_checkpoint();
    }
}
