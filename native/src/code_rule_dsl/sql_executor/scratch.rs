//! Connection-local scratch schemas. Batches keep rows transactional and reuse empty tables.
use super::*;

#[derive(Clone, Default)]
pub(super) struct Pool {
    tables: BTreeMap<(String, bool, bool), Vec<String>>,
    next: usize,
}

pub(super) struct Work<'a> {
    pub db: &'a rusqlite::Connection,
    pub versions: Option<&'a versions::Store>,
    pub profiler: Option<&'a super::super::sql_profile::Profiler>,
    pub pool: Pool,
    used: Vec<String>,
    positions: BTreeMap<(String, bool, bool), usize>,
    pub created: usize,
    pub version_nodes: usize,
    pub preparing: bool,
    pub summaries: reductions::pool::Cache,
    pub summary_transitions: Vec<reductions::Transition>,
    pub summary_reuses: usize,
    pub reducer_groups: u64,
    pub context_rows: u64,
    pub summary_initializations: Vec<reductions::Initial>,
}
pub(super) struct Mark {
    used: usize,
    positions: BTreeMap<(String, bool, bool), usize>,
}

impl Pool {
    #[cfg(test)]
    pub(in crate::code_rule_dsl::sql_executor) fn names(&self) -> Vec<String> {
        self.tables.values().flatten().cloned().collect::<BTreeSet<_>>().into_iter().collect()
    }
    pub fn clear(&mut self, db: &rusqlite::Connection, store: &versions::Store) -> Result<()> {
        for name in self.tables.values().flatten() {
            if store.owns(name) {
                store.remove_all(db, name)?;
                continue;
            }
            db.execute_batch(&format!("DROP TABLE IF EXISTS temp.{}", quote(name)))
                .map_err(error)?;
        }
        *self = Self::default();
        Ok(())
    }
}

impl<'a> Work<'a> {
    pub fn profile_scope(
        &self,
        fields: impl FnOnce() -> Value,
    ) -> Option<super::super::sql_profile::Context> {
        self.profiler.map(|p| p.scope(fields()))
    }
    /// Sequential temporal operators reuse scratch slots while the caller's deltas stay live.
    pub fn mark(&self) -> Mark {
        Mark {
            used: self.used.len(),
            positions: self.positions.clone(),
        }
    }
    pub fn rewind(&mut self, mark: Mark) -> Result<()> {
        for name in &self.used[mark.used..] {
            if let Some(store) = self.versions.filter(|s| s.owns(name)) {
                store.clear_scratch(name)?;
                continue;
            }
            self.db
                .execute(&format!("DELETE FROM {}", quote(name)), [])
                .map_err(error)?;
        }
        self.used.truncate(mark.used);
        self.positions = mark.positions;
        Ok(())
    }
    pub fn new(db: &'a rusqlite::Connection, pool: &Pool) -> Self {
        Self {
            db,
            versions: None,
            profiler: None,
            pool: pool.clone(),
            used: vec![],
            positions: BTreeMap::new(),
            created: 0,
            version_nodes: 0,
            preparing: false,
            summaries: BTreeMap::new(),
            summary_transitions: vec![],
            summary_reuses: 0,
            reducer_groups: 0,
            context_rows: 0,
            summary_initializations: vec![],
        }
    }
    pub fn allocate(&mut self, definition: String, without_rowid: bool) -> Result<String> {
        self.allocate_access(definition, without_rowid, true)
    }
    pub fn allocate_readonly(&mut self, definition: String) -> Result<String> {
        self.allocate_access(definition, true, false)
    }
    fn allocate_access(
        &mut self,
        definition: String,
        without_rowid: bool,
        writable: bool,
    ) -> Result<String> {
        let shape = (definition, without_rowid, writable);
        let position = self.positions.entry(shape.clone()).or_default();
        let tables = self.pool.tables.entry(shape.clone()).or_default();
        if *position == tables.len() {
            let name = format!("__rr_update_{}", self.pool.next);
            self.pool.next += 1;
            if let Some(store) = self.versions.filter(|_| shape.1) {
                store.create_scratch_access(
                    self.db,
                    &name,
                    versions::Schema::generated(&shape.0)?,
                    shape.2,
                )?;
            } else {
                self.db
                    .execute_batch(&format!(
                        "CREATE TEMP TABLE {}({}){}",
                        quote(&name),
                        shape.0,
                        if without_rowid { " WITHOUT ROWID" } else { "" }
                    ))
                    .map_err(error)?;
            }
            tables.push(name);
            self.created += 1;
        }
        let name = tables[*position].clone();
        *position += 1;
        self.used.push(name.clone());
        Ok(name)
    }
    pub fn cleanup(&self) -> Result<()> {
        for name in &self.used {
            if let Some(store) = self.versions.filter(|s| s.owns(name)) {
                store.clear_scratch(name)?;
                continue;
            }
            self.db
                .execute(&format!("DELETE FROM {}", quote(name)), [])
                .map_err(error)?;
        }
        Ok(())
    }
}
