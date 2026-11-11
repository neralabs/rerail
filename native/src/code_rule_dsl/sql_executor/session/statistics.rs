//! Read-only row-count metadata for retained schedules. Cold schedules keep their SQL table.
use crate::code_rule_dsl::facts::vtab::Modules;
use super::*;
use std::cell::RefCell;

#[derive(Default)]
pub(in crate::code_rule_dsl::sql_executor) struct Statistics {
    modules: Modules,
    current: RefCell<Option<Arc<Rows>>>,
}
pub struct Rows(Vec<(String, i64)>);
impl Source for Rows {
    fn columns(&self) -> Vec<String> {
        vec!["tbl".into(), "rows".into()]
    }
    fn text_column(&self, c: usize) -> bool {
        c == 0
    }
    fn len(&self) -> usize {
        self.0.len()
    }
    fn estimated_matches(&self, _: usize, _: bool) -> usize {
        self.len()
    }
    fn indexed(&self, _: usize) -> bool {
        false
    }
    fn lookup(&self, _: usize, _: &[i64]) -> Option<Vec<usize>> {
        None
    }
    fn value(&self, r: usize, c: usize) -> i64 {
        if c == 1 { self.0[r].1 } else { 0 }
    }
    fn cell(&self, r: usize, c: usize) -> SqlValue {
        if c == 0 {
            SqlValue::Text(self.0[r].0.clone())
        } else {
            SqlValue::Integer(self.0[r].1)
        }
    }
}
impl Statistics {
    pub fn snapshot(&self) -> Option<Arc<Rows>> {
        self.current.borrow().clone()
    }
    pub fn restore(&self, db: &rusqlite::Connection, old: Option<Arc<Rows>>) -> Result<()> {
        if let Some(rows) = old {
            self.modules
                .bind(db, "wave_stats", rows.clone())
                .map_err(error)?;
            *self.current.borrow_mut() = Some(rows);
        } else if self.current.borrow().is_some() {
            self.clear(db)?;
        }
        Ok(())
    }
    pub fn patch(&self, db: &rusqlite::Connection, rows: &[Vec<SqlValue>]) -> Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        let mut values = self
            .current
            .borrow()
            .as_ref()
            .unwrap()
            .0
            .iter()
            .cloned()
            .collect::<BTreeMap<_, _>>();
        for row in rows {
            let [SqlValue::Text(name), SqlValue::Integer(n)] = row.as_slice() else {
                return Err(error("invalid wave statistics row"));
            };
            values.insert(name.clone(), *n);
        }
        self.restore(db, Some(Arc::new(Rows(values.into_iter().collect()))))
    }
    pub fn enable(&self, db: &rusqlite::Connection, rows: Vec<Vec<SqlValue>>) -> Result<()> {
        let mut values = BTreeMap::new();
        for row in rows {
            let [SqlValue::Text(name), SqlValue::Integer(n)] = row.as_slice() else {
                return Err(error("invalid wave statistics row"));
            };
            values.insert(name.clone(), *n);
        }
        db.execute_batch("DROP TABLE temp.wave_stats")
            .map_err(error)?;
        self.restore(db, Some(Arc::new(Rows(values.into_iter().collect()))))?;
        db.execute_batch("CREATE TEMP VIEW wave_stats AS SELECT * FROM main.wave_stats")
            .map_err(error)
    }
    pub fn clear(&self, db: &rusqlite::Connection) -> Result<()> {
        if self.current.borrow().is_some() {
            db.execute_batch("DROP VIEW IF EXISTS temp.wave_stats")
                .map_err(error)?;
            self.forget(db)?;
        }
        Ok(())
    }
    pub fn forget(&self, db: &rusqlite::Connection) -> Result<()> {
        if self.current.borrow().is_some() {
            self.modules.unregister(db, "wave_stats").map_err(error)?;
            self.current.borrow_mut().take();
        }
        Ok(())
    }
    /// A later program can use a cold scheduler which writes its row counts through SQL.
    pub fn materialize(&self, db: &rusqlite::Connection) -> Result<()> {
        let Some(rows) = self.snapshot() else {
            return Ok(());
        };
        db.execute_batch("SAVEPOINT sql_statistics")
            .map_err(error)?;
        let result = (|| {
            self.clear(db)?;
            db.execute_batch("CREATE TEMP TABLE wave_stats(tbl TEXT PRIMARY KEY,rows INTEGER NOT NULL) WITHOUT ROWID").map_err(error)?;
            plan::insert_many(
                db,
                "INSERT INTO wave_stats(tbl,rows)",
                rows.0
                    .iter()
                    .map(|(r, n)| vec![SqlValue::Text(r.clone()), SqlValue::Integer(*n)])
                    .collect(),
            )
            .map_err(error)?;
            db.execute_batch("RELEASE sql_statistics").map_err(error)
        })();
        if result.is_err() {
            let _ = db.execute_batch("ROLLBACK TO sql_statistics; RELEASE sql_statistics");
            self.restore(db, Some(rows))?;
        }
        result
    }
}
