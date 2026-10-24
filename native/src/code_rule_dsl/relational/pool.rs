//! Keep four idle SQL readers. Resident admission bounds active consumers and preserves broad producer waves.
use super::{Database, Result, configure, connection, sql_error};
use std::{
    cell::{Cell, RefCell},
    ops::Deref,
    rc::Rc,
    sync::{Arc, atomic::AtomicUsize},
};
const READERS: usize = 4;
#[derive(Default, Clone)]
pub(super) struct Pool(Rc<RefCell<Vec<Database>>>);
pub(super) struct Lease {
    db: Option<Database>,
    pool: Pool,
    pub reusable: Cell<bool>,
}
impl Pool {
    pub fn trim(&self, capacity: usize) {
        let mut idle = self.0.borrow_mut();
        while idle
            .iter()
            .map(|db| {
                db.resident_bytes()
                    .unwrap_or(usize::MAX)
                    .saturating_add(db.storage.borrow().bytes())
            })
            .fold(0usize, usize::saturating_add)
            > capacity
        {
            if idle.pop().is_none() {
                break;
            }
        }
    }
    pub fn take(&self, max_pattern: usize, work: Arc<AtomicUsize>) -> Result<Lease> {
        let db = match self.0.borrow_mut().pop() {
            Some(db) => configure(db, max_pattern, work)?,
            None => connection(max_pattern, work)?,
        };
        Ok(Lease {
            db: Some(db),
            pool: self.clone(),
            reusable: Cell::new(false),
        })
    }
}
impl Lease {
    pub fn resident_bytes(&self) -> rusqlite::Result<usize> {
        Ok(self
            .deref()
            .resident_bytes()?
            .saturating_add(self.storage.borrow().bytes()))
    }
}
impl Deref for Lease {
    type Target = Database;
    fn deref(&self) -> &Database {
        self.db.as_ref().unwrap()
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        let Some(db) = self.db.take() else { return };
        if !self.reusable.get() || !db.is_autocommit() {
            return;
        }
        if db.storage.borrow_mut().clear(&db).is_err() || !db.storage.borrow().bounded() {
            return;
        }
        for (name, arity) in [
            ("source_text", 3),
            ("rx_matches", 2),
            ("rx_extract", 3),
            ("rx_replace", 3),
        ] {
            if db.remove_function(name, arity).is_err() {
                return;
            }
        }
        if db
            .resident_bytes()
            .map(|n| n.saturating_add(db.storage.borrow().bytes()))
            .map_err(sql_error)
            .is_ok_and(|n| n <= 16 * 1024 * 1024)
            && self.pool.0.borrow().len() < READERS
        {
            self.pool.0.borrow_mut().push(db);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_successful_lease_returns_empty_backing_and_resets_source_and_function_budgets() {
        let pool = Pool::default();
        let old = Arc::new(AtomicUsize::new(0));
        {
            let db = pool.take(1024, old.clone()).unwrap();
            db.storage
                .borrow_mut()
                .declare(&db, "input", "CREATE TABLE input(value)".into(), true)
                .unwrap();
            db.execute("INSERT INTO input VALUES(7)", []).unwrap();
            db.storage.borrow_mut().written("input", true);
            let file = crate::code_rule_dsl::model::FileInput::new(
                "old.py".into(),
                "python".into(),
                Arc::new("old".into()),
                None,
                None,
                "test",
            )
            .unwrap();
            super::super::source::install(
                &db,
                Some(&crate::code_rule_dsl::model::Snapshot {
                    revision: 1,
                    files: vec![file],
                }),
                Arc::new(AtomicUsize::new(100)),
            )
            .unwrap();
            assert_eq!(
                db.query_row("SELECT source_text('old.py',0,3)", [], |r| r
                    .get::<_, String>(0))
                    .unwrap(),
                "old"
            );
            db.reusable.set(true);
        }
        let work = Arc::new(AtomicUsize::new(100));
        let db = pool.take(1024, work.clone()).unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM input", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            db.query_row("SELECT rx_matches('a','a')", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            db.query_row("SELECT source_text('old.py',0,3)", [], |r| r
                .get::<_, Option<String>>(0))
                .unwrap(),
            None
        );
        assert_eq!(old.load(std::sync::atomic::Ordering::Relaxed), 0);
        assert!(work.load(std::sync::atomic::Ordering::Relaxed) < 100);
    }
    #[test]
    fn incomplete_or_failed_consumers_cannot_recycle_partial_rows() {
        let pool = Pool::default();
        {
            let db = pool.take(1024, Arc::new(AtomicUsize::new(100))).unwrap();
            db.execute_batch("CREATE TABLE unfinished(value);INSERT INTO unfinished VALUES(1)")
                .unwrap();
        }
        let db = pool.take(1024, Arc::new(AtomicUsize::new(100))).unwrap();
        assert!(db.prepare("SELECT * FROM unfinished").is_err());
    }
}
