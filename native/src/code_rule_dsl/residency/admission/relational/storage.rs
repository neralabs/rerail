//! Reusable SQL schemas. A connection lease owns all rows; only empty backing is reusable.
use rusqlite::{Connection, Result};
use std::collections::BTreeMap;
#[derive(Default)]
pub(in crate::code_rule_dsl) struct Storage {
    objects: BTreeMap<String, Object>,
}
struct Object {
    definition: String,
    table: bool,
    dirty: bool,
    indexes: BTreeMap<String, String>,
    analyzed: Option<usize>,
}
fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}
impl Storage {
    pub fn declare(
        &mut self,
        db: &Connection,
        name: &str,
        definition: String,
        table: bool,
    ) -> Result<()> {
        let name = &name.to_ascii_lowercase();
        if self
            .objects
            .get(name)
            .is_some_and(|o| o.definition == definition && o.table == table)
        {
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
            },
        );
        Ok(())
    }
    pub fn index(&mut self, db: &Connection, table: &str, name: &str, sql: String) -> Result<()> {
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
        let name = &name.to_ascii_lowercase();
        let o = self.objects.get_mut(name).expect("declared table");
        if rows > 0 || o.analyzed.is_some_and(|old| old > 0) {
            db.execute_batch(&format!("ANALYZE temp.{}", quote(name)))?;
            o.analyzed = Some(rows);
        }
        Ok(())
    }
    pub fn written(&mut self, name: &str, rows: bool) {
        if let Some(o) = self.objects.get_mut(&name.to_ascii_lowercase()) {
            o.dirty |= rows;
        }
    }
    pub fn retire(&mut self, db: &Connection, name: &str) -> Result<bool> {
        let Some(o) = self.objects.get_mut(&name.to_ascii_lowercase()) else {
            return Ok(false);
        };
        if o.table && o.dirty {
            db.execute(&format!("DELETE FROM {}", quote(name)), [])?;
            o.dirty = false;
        }
        Ok(true)
    }
    pub fn discard(&mut self, db: &Connection, name: &str) -> Result<()> {
        if let Some(o) = self.objects.remove(&name.to_ascii_lowercase()) {
            db.execute_batch(&format!(
                "DROP {} {}",
                if o.table { "TABLE" } else { "VIEW" },
                quote(name)
            ))?;
        }
        Ok(())
    }
    pub fn clear(&mut self, db: &Connection) -> Result<()> {
        for (name, o) in &mut self.objects {
            if o.table && o.dirty {
                db.execute(&format!("DELETE FROM {}", quote(name)), [])?;
                o.dirty = false;
            }
        }
        Ok(())
    }
    pub fn bytes(&self) -> usize {
        self.objects
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
            .sum()
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
