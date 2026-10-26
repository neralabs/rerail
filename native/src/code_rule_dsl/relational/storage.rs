//! Reusable SQL schemas. A connection lease owns all rows; only empty backing is reusable.
use rusqlite::{Connection, Result};
use std::collections::BTreeMap;
#[path = "storage/contexts.rs"]
mod contexts;
#[cfg(test)]
#[path = "storage/contexts_tests.rs"]
mod contexts_tests;
#[derive(Default)]
pub(super) struct Storage {
    contexts: contexts::Contexts,
    objects: BTreeMap<String, Object>,
    pub(super) owner: Option<usize>,
    pins: BTreeMap<String, usize>,
}
struct Object {
    definition: String,
    table: bool,
    dirty: bool,
    indexes: BTreeMap<String, String>,
    analyzed: Option<usize>,
    owner: Option<usize>,
    ready: bool,
}
fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}
impl Storage {
    pub fn bind_scope(
        &mut self,
        db: &Connection,
        owner: usize,
        bindings: BTreeMap<String, usize>,
    ) -> Result<()> {
        self.owner = Some(owner);
        self.contexts.bind(db, owner, bindings)
    }
    pub fn write_target(&self, name: &str) -> (String, Option<i64>) {
        self.contexts
            .target(name)
            .unwrap_or_else(|| (name.to_owned(), None))
    }
    pub fn declare(
        &mut self,
        db: &Connection,
        name: &str,
        definition: String,
        table: bool,
    ) -> Result<()> {
        if contexts::managed(name) && table {
            return self.contexts.declare(db, name, &definition);
        }
        let name = &name.to_ascii_lowercase();
        if self
            .objects
            .get(name)
            .is_some_and(|o| o.definition == definition && o.table == table)
        {
            if let Some(o) = self.objects.get_mut(name)
                && !o.ready
            {
                o.owner = self.owner;
                if o.table && o.dirty {
                    db.execute(&format!("DELETE FROM {}", quote(name)), [])?;
                    o.dirty = false;
                }
            }
            return Ok(());
        }
        self.discard(db, name)?;
        db.execute_batch(&definition)?;
        self.objects.insert(
            name.to_owned(),
            Object {
                definition,
                table,
                dirty: false,
                indexes: BTreeMap::new(),
                analyzed: None,
                owner: self.owner,
                ready: false,
            },
        );
        Ok(())
    }
    pub fn index(&mut self, db: &Connection, table: &str, name: &str, sql: String) -> Result<()> {
        if contexts::managed(table) {
            return self.contexts.index(db, table, name, &sql);
        }
        let table = &table.to_ascii_lowercase();
        let o = self
            .objects
            .get_mut(table)
            .expect("declared physical table");
        if o.indexes.get(name) == Some(&sql) {
            return Ok(());
        }
        if o.indexes.contains_key(name) {
            db.execute_batch(&format!("DROP INDEX {}", quote(name)))?;
        }
        db.execute_batch(&sql)?;
        o.indexes.insert(name.into(), sql);
        Ok(())
    }
    pub fn analyze(&mut self, db: &Connection, name: &str, rows: usize) -> Result<()> {
        if contexts::managed(name) {
            return self.contexts.analyze(db, name, rows);
        }
        let name = &name.to_ascii_lowercase();
        let o = self.objects.get_mut(name).expect("declared table");
        if rows > 0 || o.analyzed.is_some_and(|old| old > 0) {
            db.execute_batch(&format!("ANALYZE temp.{}", quote(name)))?;
            o.analyzed = Some(rows);
        }
        Ok(())
    }
    pub fn written(&mut self, name: &str, rows: bool) {
        if contexts::managed(name) {
            self.contexts.written(name, rows);
            return;
        }
        if let Some(o) = self.objects.get_mut(&name.to_ascii_lowercase()) {
            o.dirty |= rows;
        }
    }
    pub fn retire(&mut self, db: &Connection, name: &str) -> Result<bool> {
        if contexts::managed(name) {
            return self.contexts.retire(db, name);
        }
        let Some(o) = self.objects.get_mut(&name.to_ascii_lowercase()) else {
            return Ok(false);
        };
        if o.ready {
            return Ok(true);
        }
        if o.table && o.dirty {
            db.execute(&format!("DELETE FROM {}", quote(name)), [])?;
            o.dirty = false;
        }
        Ok(true)
    }
    pub fn discard(&mut self, db: &Connection, name: &str) -> Result<()> {
        if self.contexts.contains(name) {
            return self.contexts.discard(db, name);
        }
        if let Some(o) = self.objects.remove(&name.to_ascii_lowercase()) {
            db.execute_batch(&format!(
                "DROP {} {}",
                if o.table { "TABLE" } else { "VIEW" },
                quote(name)
            ))?;
        }
        Ok(())
    }
    #[cfg(test)]
    pub fn clear(&mut self, db: &Connection) -> Result<()> {
        for (name, o) in &mut self.objects {
            if o.table && o.dirty && !o.ready {
                db.execute(&format!("DELETE FROM {}", quote(name)), [])?;
                o.dirty = false;
            }
        }
        Ok(())
    }
    pub fn ready(&self, name: &str) -> bool {
        if contexts::managed(name) {
            return self.contexts.ready(name);
        }
        self.objects
            .get(&name.to_ascii_lowercase())
            .is_some_and(|o| o.ready)
    }
    pub fn publish(&mut self, name: &str) {
        if contexts::managed(name) {
            self.contexts.publish(name);
            return;
        }
        if let Some(object) = self.objects.get_mut(&name.to_ascii_lowercase()) {
            object.ready = true;
            object.owner = None;
        }
    }
    pub fn pin(&mut self, name: &str) {
        *self.pins.entry(name.to_owned()).or_default() += 1;
    }
    pub fn unpin(&mut self, name: &str) {
        if let Some(n) = self.pins.get_mut(name) {
            *n -= 1;
            if *n == 0 {
                self.pins.remove(name);
            }
        }
    }
    pub fn clear_owner(&mut self, db: &Connection, owner: usize) -> Result<()> {
        self.contexts.clear_owner(db, owner)?;
        let owned = self
            .objects
            .iter()
            .filter(|(_, o)| o.owner == Some(owner))
            .map(|(n, _)| n.clone())
            .collect::<Vec<_>>();
        for name in owned {
            if name.starts_with("__private_") || name.starts_with("__stage___private_") {
                if let Some(o) = self.objects.remove(&name) {
                    db.execute_batch(&format!(
                        "DROP {} IF EXISTS {}",
                        if o.table { "TABLE" } else { "VIEW" },
                        quote(&name)
                    ))?;
                }
            } else {
                self.retire(db, &name)?;
            }
        }
        Ok(())
    }
    pub fn evict(&mut self, db: &Connection, capacity: usize) -> Result<()> {
        self.contexts.evict(db, capacity)?;
        // Admission owns the capacity. Only completed, unpinned producers can leave it.
        let names = self
            .objects
            .iter()
            .filter(|(n, o)| o.ready && !self.pins.contains_key(*n))
            .map(|(n, _)| n.clone())
            .collect::<Vec<_>>();
        for name in names {
            if self.bounded() && self.bytes() <= capacity.min(4 * 1024 * 1024) {
                break;
            }
            self.discard(db, &name)?;
        }
        Ok(())
    }
    pub fn bytes(&self) -> usize {
        self.contexts.bytes()
            + self
                .objects
                .iter()
                .map(|(name, o)| {
                    std::mem::size_of::<Object>()
                        + name.capacity()
                        + o.definition.capacity()
                        + o.indexes
                            .iter()
                            .map(|(n, sql)| n.capacity() + sql.capacity() + 64)
                            .sum::<usize>()
                        + 64
                })
                .sum::<usize>()
    }
    pub fn bounded(&self) -> bool {
        self.objects.len() <= 128 && self.bytes() <= 4 * 1024 * 1024
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reuses_empty_schemas_preserves_bags_and_affinity_and_separates_successive_bindings()
    -> Result<()> {
        let db = Connection::open_in_memory()?;
        let mut s = Storage::default();
        let definition = "CREATE TEMP TABLE keys(fid INTEGER,pre INTEGER,value)".to_owned();
        for _ in 0..2 {
            s.declare(&db, "keys", definition.clone(), true)?;
        }
        let count = db.execute("INSERT INTO keys VALUES('1','7',NULL),('1','7',NULL)", [])?;
        s.written("keys", count != 0);
        let rows: Vec<(i64, i64, Option<String>)> = db
            .prepare("SELECT fid,pre,value FROM keys")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<Result<_>>()?;
        assert_eq!(rows, vec![(1, 7, None), (1, 7, None)]);
        assert!(s.retire(&db, "keys")?);
        assert!(s.retire(&db, "keys")?);
        assert_eq!(
            db.query_row("SELECT count(*) FROM keys", [], |r| r.get::<_, i64>(0))?,
            0
        );
        s.declare(
            &db,
            "keys",
            "CREATE TEMP TABLE keys(value TEXT)".into(),
            true,
        )?;
        db.execute("INSERT INTO keys VALUES(7)", [])?;
        s.written("keys", true);
        assert_eq!(
            db.query_row("SELECT typeof(value) FROM keys", [], |r| r
                .get::<_, String>(0))?,
            "text"
        );
        s.clear(&db)?;
        assert_eq!(
            db.query_row("SELECT count(*) FROM keys", [], |r| r.get::<_, i64>(0))?,
            0
        );
        Ok(())
    }
    #[test]
    fn keeps_indexes_and_deferred_views_with_empty_backing_between_leases() -> Result<()> {
        let db = Connection::open_in_memory()?;
        let mut s = Storage::default();
        s.declare(
            &db,
            "backing",
            "CREATE TEMP TABLE backing(fid INTEGER,pre INTEGER)".into(),
            true,
        )?;
        for _ in 0..2 {
            s.index(
                &db,
                "backing",
                "by_key",
                "CREATE INDEX by_key ON backing(fid,pre)".into(),
            )?;
            s.declare(
                &db,
                "visible",
                "CREATE TEMP VIEW visible AS SELECT fid,pre+1 AS pre FROM backing".into(),
                false,
            )?;
        }
        db.execute("INSERT INTO backing VALUES(1,7)", [])?;
        s.written("backing", true);
        assert_eq!(
            db.query_row("SELECT pre FROM visible", [], |r| r.get::<_, i64>(0))?,
            8
        );
        s.retire(&db, "backing")?;
        assert_eq!(
            db.query_row("SELECT count(*) FROM visible", [], |r| r.get::<_, i64>(0))?,
            0
        );
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM temp.sqlite_schema WHERE type='index'",
                [],
                |r| r.get::<_, i64>(0)
            )?,
            1
        );
        Ok(())
    }
}
