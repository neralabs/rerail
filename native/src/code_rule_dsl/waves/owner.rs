//! One SQL owner on the content store (decision 0010), from Data-layer-rule-expressivity's
//! `relational/pool.rs`. Consumers keep private aliases and independent leases; the connection is
//! recycled across executions only when it is clean.
//!
//! D's owner was an in-memory connection, rebound per lease to a source snapshot, regex functions
//! and a work budget. Here the connection is on the store's file, so `fact_stats`,
//! `compiled_plans` and `atoms` are `main`, and a lease binds the base-relation vtabs instead
//! (`facts::vtab::syntax`). `statistics_boundary` returns the counts the compiled plans' validity
//! reads (`stats::plans`), and `trim` evicts completed relations to a byte capacity.
mod database;

pub(in crate::code_rule_dsl) use database::Database;
use crate::code_rule_dsl::facts::vtab::Source;
use crate::content_store::{ContentStore, StoreResult};
use parking_lot::{ArcMutexGuard, Mutex, RawMutex};
use std::{
    cell::{OnceCell, RefCell},
    collections::BTreeMap,
    ops::Deref,
    rc::Rc,
    sync::Arc,
};

/// Bytes of schema, statements and bookkeeping a recycled owner may keep.
const RETAINED_BYTES: usize = 4 * 1024 * 1024;

/// The owner kept between executions.
#[derive(Default, Clone)]
pub(in crate::code_rule_dsl) struct Backing(Arc<Mutex<Option<Database>>>);
struct Inner<'s> {
    store: &'s ContentStore,
    db: OnceCell<Database>,
}
#[derive(Clone)]
pub(in crate::code_rule_dsl) struct Pool<'s>(Rc<Inner<'s>>);
/// One execution's hold on the backing: the owner returns to it on drop, if clean.
pub(in crate::code_rule_dsl) struct Scope<'s> {
    pool: Option<Pool<'s>>,
    backing: ArcMutexGuard<RawMutex, Option<Database>>,
}
impl Backing {
    pub(in crate::code_rule_dsl) fn enter<'s>(&self, store: &'s ContentStore) -> Scope<'s> {
        let mut backing = self.0.lock_arc();
        let pool = Pool(Rc::new(Inner { store, db: OnceCell::new() }));
        if let Some(db) = backing.take()
            && db.path == store.database_path()
        {
            let _ = pool.0.db.set(db);
        }
        Scope { pool: Some(pool), backing }
    }
}
impl<'s> Scope<'s> {
    pub(in crate::code_rule_dsl) fn pool(&self) -> Pool<'s> {
        self.pool.as_ref().unwrap().clone()
    }
}
impl Drop for Scope<'_> {
    fn drop(&mut self) {
        let pool = self.pool.take().unwrap();
        if let Ok(inner) = Rc::try_unwrap(pool.0)
            && let Some(db) = inner.db.into_inner()
        {
            db.clear_bindings();
            let clean = db.storage.borrow_mut().evict(&db, RETAINED_BYTES).is_ok();
            if clean && db.storage.borrow().bounded() && db.healthy.get() && db.is_autocommit() {
                *self.backing = Some(db);
            }
        }
    }
}
pub(in crate::code_rule_dsl) struct Lease<'s> {
    pool: Pool<'s>,
    pub id: usize,
    pins: RefCell<Vec<String>>,
}
impl<'s> Pool<'s> {
    pub(in crate::code_rule_dsl) fn store(&self) -> &'s ContentStore {
        self.0.store
    }
    pub(in crate::code_rule_dsl) fn invalidate(&self) {
        if let Some(db) = self.0.db.get() {
            db.healthy.set(false);
        }
    }
    /// Writes the counts of the relations changed since the last boundary to `temp.wave_stats`
    /// and returns every relation's `rows.<name>`.
    pub(in crate::code_rule_dsl) fn statistics_boundary(&self) -> rusqlite::Result<BTreeMap<String, u64>> {
        match self.0.db.get() {
            Some(db) => db.storage.borrow_mut().statistics_boundary(db),
            None => Ok(BTreeMap::new()),
        }
    }
    /// Evicts completed, unpinned relations until the owner's bookkeeping fits `capacity`.
    pub(in crate::code_rule_dsl) fn trim(&self, capacity: usize) {
        if let Some(db) = self.0.db.get()
            && db.storage.borrow_mut().evict(db, capacity).is_err()
        {
            db.healthy.set(false);
        }
    }
    pub(in crate::code_rule_dsl) fn take(&self) -> StoreResult<Lease<'s>> {
        if self.0.db.get().is_none() {
            let _ = self.0.db.set(Database::open(self.0.store)?);
        }
        let db = self.0.db.get().unwrap();
        db.ensure_idle()?;
        let id = db.leases.get() + 1;
        db.leases.set(id);
        db.clear_bindings();
        db.active.set(id);
        db.storage.borrow_mut().owner = Some(id);
        Ok(Lease { pool: self.clone(), id, pins: RefCell::default() })
    }
}
impl Lease<'_> {
    pub(in crate::code_rule_dsl) fn private(&self, name: &str) -> String {
        format!("__private_{}_{}", self.id, name)
    }
    /// Binds `source` to the vtab `name` for this lease.
    pub(in crate::code_rule_dsl) fn bind(&self, name: &str, source: Arc<dyn Source>) -> rusqlite::Result<()> {
        self.activate()?;
        self.deref().bind(name, source)
    }
    pub(in crate::code_rule_dsl) fn pin(&self, name: &str) {
        self.storage.borrow_mut().pin(name);
        self.pins.borrow_mut().push(name.to_owned());
    }
    /// Makes this lease the connection's active one; another lease's vtab bindings are cleared.
    pub(in crate::code_rule_dsl) fn activate(&self) -> rusqlite::Result<()> {
        if self.active.get() != self.id {
            self.ensure_idle()?;
            self.clear_bindings();
            self.active.set(self.id);
        }
        self.storage.borrow_mut().owner = Some(self.id);
        Ok(())
    }
    pub(in crate::code_rule_dsl) fn resident_bytes(&self) -> rusqlite::Result<usize> {
        Ok(self.deref().resident_bytes()?.saturating_add(self.storage.borrow().bytes()))
    }
}
impl Deref for Lease<'_> {
    type Target = Database;
    fn deref(&self) -> &Database {
        self.pool.0.db.get().unwrap()
    }
}
impl Drop for Lease<'_> {
    fn drop(&mut self) {
        if !self.is_autocommit() {
            let _ = self.execute_batch("ROLLBACK");
        }
        if self.active.get() == self.id {
            self.clear_bindings();
        }
        let _ = self.storage.borrow_mut().clear_owner(self, self.id);
        let pins = std::mem::take(self.pins.get_mut());
        for name in pins {
            self.storage.borrow_mut().unpin(&name);
        }
    }
}

#[cfg(test)]
mod tests;
