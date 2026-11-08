//! Connection-local scratch schemas. Batches keep rows transactional and reuse empty tables.
use super::*;

#[derive(Clone, Default)]
pub(super) struct Pool {
    tables: BTreeMap<(String, bool), Vec<String>>,
    next: usize,
}

pub(super) struct Work<'a> {
    pub db: &'a rusqlite::Connection,
    pub pool: Pool,
    used: Vec<String>,
    positions: BTreeMap<(String, bool), usize>,
    pub created: usize,
}
pub(super) struct Mark {
    used: usize,
    positions: BTreeMap<(String, bool), usize>,
}

impl Pool {
    pub fn clear(&mut self, db: &rusqlite::Connection) -> Result<()> {
        for name in self.tables.values().flatten() {
            db.execute_batch(&format!("DROP TABLE IF EXISTS temp.{}", quote(name)))
                .map_err(error)?;
        }
        *self = Self::default();
        Ok(())
    }
}

impl<'a> Work<'a> {
    /// Sequential temporal operators reuse scratch slots while the caller's deltas stay live.
    pub fn mark(&self) -> Mark {
        Mark {
            used: self.used.len(),
            positions: self.positions.clone(),
        }
    }
    pub fn rewind(&mut self, mark: Mark) -> Result<()> {
        for name in &self.used[mark.used..] {
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
            pool: pool.clone(),
            used: vec![],
            positions: BTreeMap::new(),
            created: 0,
        }
    }
    pub fn allocate(&mut self, definition: String, without_rowid: bool) -> Result<String> {
        let shape = (definition, without_rowid);
        let position = self.positions.entry(shape.clone()).or_default();
        let tables = self.pool.tables.entry(shape.clone()).or_default();
        if *position == tables.len() {
            let name = format!("__rr_update_{}", self.pool.next);
            self.pool.next += 1;
            self.db
                .execute_batch(&format!(
                    "CREATE TEMP TABLE {}({}){}",
                    quote(&name),
                    shape.0,
                    if without_rowid { " WITHOUT ROWID" } else { "" }
                ))
                .map_err(error)?;
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
            self.db
                .execute(&format!("DELETE FROM {}", quote(name)), [])
                .map_err(error)?;
        }
        Ok(())
    }
}
