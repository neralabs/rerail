//! The owner's connection and what it carries (D's `relational/database.rs`; decision 0010): the
//! base-relation vtabs, the symbol functions, the `demand` modules and the temp schema's storage.
//! D's source and regex bindings, program cache and profiler stay with Phase 8's producers.
use super::super::storage::Storage;
use crate::code_rule_dsl::demand::vtab::{MatchRegistry, register_module};
use crate::code_rule_dsl::facts::vtab::{Modules, Source, plan_signature};
use crate::code_rule_dsl::logic::native::register_symbols;
use crate::content_store::{ContentStore, StoreError, StoreResult};
use rusqlite::Connection;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::Arc;

pub(in crate::code_rule_dsl) struct Database {
    connection: Connection,
    /// The store file the connection is on; a recycled owner serves only that store.
    pub(super) path: PathBuf,
    pub(super) healthy: Cell<bool>,
    pub(super) active: Cell<usize>,
    pub(super) leases: Cell<usize>,
    pub(super) plan_signature: Cell<u64>,
    pub(in crate::code_rule_dsl::waves) storage: RefCell<Storage>,
    pub(super) modules: Modules,
    /// The sources bound now, by table: the plan signature's input.
    pub(super) bound: RefCell<BTreeMap<String, Arc<dyn Source>>>,
    pub(in crate::code_rule_dsl::waves) registry: Arc<MatchRegistry>,
}

impl Database {
    pub(super) fn ensure_idle(&self) -> rusqlite::Result<()> {
        // SAFETY: this connection and its statements belong to the current worker.
        // SQLite retains the statement list until statements are finalized here.
        unsafe {
            let mut statement = rusqlite::ffi::sqlite3_next_stmt(self.handle(), std::ptr::null_mut());
            while !statement.is_null() {
                if rusqlite::ffi::sqlite3_stmt_busy(statement) != 0 {
                    return Err(rusqlite::Error::SqliteFailure(
                        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
                        Some("execution binding has an open SQL cursor".into()),
                    ));
                }
                statement = rusqlite::ffi::sqlite3_next_stmt(self.handle(), statement);
            }
        }
        Ok(())
    }
    pub(in crate::code_rule_dsl) fn prepare_execution(&self, query: &str) -> rusqlite::Result<rusqlite::CachedStatement<'_>> {
        // Cached plans depend on native access paths and estimates, as well as SQL
        // schema. Zero-row bindings must not freeze scan plans for populated data.
        self.prepare_cached(&format!("{query}\n/* native estimates {:x} */", self.plan_signature.get()))
    }
    /// Binds `source` to the vtab `name` and refreshes the plan signature.
    pub(super) fn bind(&self, name: &str, source: Arc<dyn Source>) -> rusqlite::Result<()> {
        self.modules.bind(self, name, Arc::clone(&source))?;
        let mut bound = self.bound.borrow_mut();
        bound.insert(name.to_owned(), source);
        self.plan_signature.set(plan_signature(&bound));
        Ok(())
    }
    /// Empties every vtab: a later reader sees no rows rather than another lease's.
    pub(super) fn clear_bindings(&self) {
        for name in std::mem::take(&mut *self.bound.borrow_mut()).into_keys() {
            if let Some(binding) = self.modules.get(&name) {
                binding.clear();
            }
        }
        self.plan_signature.set(0);
    }
    /// SQLite-owned storage held by a suspended consumer, without scanning rows.
    pub(in crate::code_rule_dsl) fn resident_bytes(&self) -> rusqlite::Result<usize> {
        let mut total = 0usize;
        for kind in [
            rusqlite::ffi::SQLITE_DBSTATUS_CACHE_USED,
            rusqlite::ffi::SQLITE_DBSTATUS_SCHEMA_USED,
            rusqlite::ffi::SQLITE_DBSTATUS_STMT_USED,
        ] {
            let (mut current, mut high) = (0i64, 0i64);
            // SAFETY: this owned connection stays live and is used only on its evaluating thread.
            let code = unsafe { rusqlite::ffi::sqlite3_db_status64(self.connection.handle(), kind, &mut current, &mut high, 0) };
            if code != rusqlite::ffi::SQLITE_OK {
                return Err(rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(code), None));
            }
            total = total.saturating_add(usize::try_from(current.max(0)).unwrap_or(usize::MAX));
        }
        Ok(total)
    }
    /// A connection on `store`'s file with the symbol functions and the `demand` modules.
    pub(in crate::code_rule_dsl) fn open(store: &ContentStore) -> StoreResult<Self> {
        let connection = store.open_connection()?;
        // Derived relations and their `new_`/`delta_` tables live in memory; the store's pages stay
        // its own (03 §4).
        connection.execute_batch("PRAGMA temp_store = MEMORY")?;
        register_symbols(&connection).map_err(|e| StoreError::Invalid(e.to_string()))?;
        let registry = Arc::new(MatchRegistry::default());
        register_module(&connection, Arc::clone(&registry))?;
        Ok(Self {
            connection,
            path: store.database_path().to_path_buf(),
            healthy: Cell::new(true),
            active: Cell::new(0),
            leases: Cell::new(0),
            plan_signature: Cell::new(0),
            storage: Default::default(),
            modules: Modules::default(),
            bound: Default::default(),
            registry,
        })
    }
}

impl Deref for Database {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        &self.connection
    }
}
