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
pub(in crate::code_rule_dsl) struct Pool(Rc<RefCell<Vec<Database>>>);
pub(in crate::code_rule_dsl) struct Lease {
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
pub mod tests;
