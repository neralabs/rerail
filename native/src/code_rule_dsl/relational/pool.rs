//! One SQL owner per execution; consumers keep private aliases and independent budgets.
use super::{Database, Result, configure_in_place, connection};
use parking_lot::{ArcMutexGuard, Mutex, RawMutex};
use std::{
    cell::{OnceCell, RefCell},
    ops::Deref,
    rc::Rc,
    sync::{Arc, atomic::AtomicUsize},
};

#[derive(Default, Clone)]
pub(in crate::code_rule_dsl) struct Backing(Arc<Mutex<Option<Database>>>);
struct Inner {
    db: OnceCell<Database>,
}
#[derive(Clone)]
pub(super) struct Pool(Rc<Inner>);
impl Default for Pool {
    fn default() -> Self {
        Self(Rc::new(Inner {
            db: OnceCell::new(),
        }))
    }
}
pub(super) struct Scope {
    pool: Option<Pool>,
    backing: ArcMutexGuard<RawMutex, Option<Database>>,
}
impl Backing {
    pub(super) fn enter(&self) -> Scope {
        let mut backing = self.0.lock_arc();
        let pool = Pool::default();
        if let Some(db) = backing.take() {
            let _ = pool.0.db.set(db);
        }
        Scope {
            pool: Some(pool),
            backing,
        }
    }
}
impl Scope {
    pub(super) fn pool(&self) -> Pool {
        self.pool.as_ref().unwrap().clone()
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        let pool = self.pool.take().unwrap();
        if let Ok(inner) = Rc::try_unwrap(pool.0)
            && let Some(db) = inner.db.into_inner()
        {
            let _ = db.progress_handler(0, None::<fn() -> bool>);
            let _ = super::source::bind(&db, None, Arc::new(AtomicUsize::new(0)));
            super::functions::rebind(&db.regex_binding, 0, Arc::new(AtomicUsize::new(0)));
            let bounded = db.bindings.borrow().data_bytes() <= 4 * 1024 * 1024;
            let clean = bounded || db.storage.borrow_mut().evict(&db, 0).is_ok();
            if !bounded {
                db.bindings.borrow_mut().forget();
            }
            if clean && db.healthy.get() && db.is_autocommit() {
                *self.backing = Some(db);
            }
        }
    }
}
pub(super) struct Lease {
    pool: Pool,
    pub id: usize,
    modules: RefCell<Vec<String>>,
    pins: RefCell<Vec<String>>,
}
impl Pool {
    pub fn trim(&self, capacity: usize) {
        #[cfg(feature = "code-rule-profile")]
        let _span = crate::code_rule_dsl::profile::trace::Span::start("sql-pool-trim", "admission");
        if let Some(db) = self.0.db.get() {
            if db.bindings.borrow().data_bytes() > 4 * 1024 * 1024 {
                db.bindings.borrow_mut().forget();
            }
            let _ = db.storage.borrow_mut().evict(db, capacity);
        }
    }
    pub fn take(&self, max_pattern: usize, work: Arc<AtomicUsize>) -> Result<Lease> {
        if self.0.db.get().is_none() {
            let _ = self.0.db.set(connection(max_pattern, work.clone())?);
        }
        let db = self.0.db.get().unwrap();
        configure_in_place(db, max_pattern, work)?;
        let id = db.bindings.borrow_mut().lease();
        for binding in db.modules.borrow().values() {
            binding.clear();
        }
        db.active.set(id);
        db.storage.borrow_mut().owner = Some(id);
        Ok(Lease {
            pool: self.clone(),
            id,
            modules: RefCell::default(),
            pins: RefCell::default(),
        })
    }
}
impl Lease {
    fn modules_registry(
        &self,
    ) -> std::cell::Ref<'_, std::collections::BTreeMap<String, super::table::Binding>> {
        self.deref().modules.borrow()
    }
    pub fn private(&self, name: &str) -> String {
        format!("__private_{}_{}", self.id, name)
    }
    pub fn module(&self, name: String) {
        self.modules.borrow_mut().push(name);
    }
    pub fn pin(&self, name: &str) {
        self.storage.borrow_mut().pin(name);
        self.pins.borrow_mut().push(name.to_owned());
    }
    pub fn activate(&self, max: usize, work: Arc<AtomicUsize>) -> Result<()> {
        if self.active.get() != self.id {
            for binding in self.modules_registry().values() {
                binding.clear();
            }
            configure_in_place(self, max, work)?;
            self.active.set(self.id);
        }
        self.storage.borrow_mut().owner = Some(self.id);
        Ok(())
    }
    pub fn resident_bytes(&self) -> rusqlite::Result<usize> {
        #[cfg(feature = "code-rule-profile")]
        let _span = crate::code_rule_dsl::profile::trace::Span::start(
            "sql-resident-accounting",
            "admission",
        );
        Ok(self.deref().resident_bytes()?.saturating_add(
            self.storage
                .borrow()
                .bytes()
                .saturating_add(self.bindings.borrow().bytes()),
        ))
    }
}
impl Deref for Lease {
    type Target = Database;
    fn deref(&self) -> &Database {
        self.pool.0.db.get().unwrap()
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        #[cfg(feature = "code-rule-profile")]
        let mut span =
            crate::code_rule_dsl::profile::trace::Span::start("sql-consumer-release", "lease");
        #[cfg(feature = "code-rule-profile")]
        span.detail(|| serde_json::json!({"lease":self.id}));
        let _ = self.progress_handler(0, None::<fn() -> bool>);
        if !self.is_autocommit() {
            let _ = self.execute_batch("ROLLBACK");
        }
        let modules = std::mem::take(self.modules.get_mut());
        for module in modules {
            if let Some(binding) = self.modules_registry().get(&module) {
                if self.active.get() == self.id {
                    binding.clear();
                }
            } else {
                let _ = super::table::unregister(self, &module);
            }
        }
        let _ = self.storage.borrow_mut().clear_owner(self, self.id);
        let pins = std::mem::take(self.pins.get_mut());
        for name in pins {
            self.storage.borrow_mut().unpin(&name);
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
            super::super::source::bind(
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
            let name = db.private("unfinished");
            db.storage
                .borrow_mut()
                .declare(&db, &name, format!("CREATE TABLE {name}(value)"), true)
                .unwrap();
            db.execute_batch(&format!("INSERT INTO {name} VALUES(1)"))
                .unwrap();
            db.storage.borrow_mut().written(&name, true);
        }
        let db = pool.take(1024, Arc::new(AtomicUsize::new(100))).unwrap();
        assert!(db.prepare("SELECT * FROM __private_1_unfinished").is_err());
    }
}
