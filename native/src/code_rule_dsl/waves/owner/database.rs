//! The owner's connection and what it carries (D's `relational/database.rs`; decision 0010): the
//! base-relation vtabs, the symbol functions, the `demand` modules and the temp schema's storage.
//! D's source and regex bindings, program cache and profiler stay with Phase 8's producers.
use super::super::storage::Storage;
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
    // Drop the callback before closing its connection.
    profiler: Option<Box<crate::code_rule_dsl::sql_profile::Profiler>>,
    connection: Connection,
    /// The store file the connection is on; a recycled owner serves only that store.
    pub(super) path: PathBuf,
    pub(super) healthy: Cell<bool>,
    pub(super) active: Cell<usize>,
    pub(super) leases: Cell<usize>,
    pub(super) plan_signature: Cell<u64>,
    pub storage: RefCell<Storage>,
    pub(super) modules: Modules,
    /// The sources bound now, by table: the plan signature's input.
    pub(super) bound: RefCell<BTreeMap<String, Arc<dyn Source>>>,
    /// What the layers above set up on the connection (the transducer's demand module and its match registry,
    /// `ConnectionSetup`).
    pub(in crate::code_rule_dsl) attached: Option<Box<dyn ConnectionSetup>>,
}

impl Database {
    pub(in crate::code_rule_dsl) fn profiler(
        &self,
    ) -> Option<&crate::code_rule_dsl::sql_profile::Profiler> {
        self.profiler.as_deref()
    }
    pub(in crate::code_rule_dsl) fn profile_scope(
        &self,
        fields: impl FnOnce() -> serde_json::Value,
    ) -> Option<crate::code_rule_dsl::sql_profile::Context> {
        self.profiler().map(|p| p.scope(fields()))
    }
    pub(in crate::code_rule_dsl) fn profile_checkpoint(&self) {
        if let Some(profiler) = &self.profiler {
            if let Err(error) = profiler.checkpoint(self) {
                eprintln!("SQL profile checkpoint failed: {error}");
            }
        }
    }
    /// Forget execution bookkeeping after a standalone SQL session drops its generated objects.
    pub(in crate::code_rule_dsl) fn reset_execution(&self) {
        self.reset_execution_storage();
        self.flush_prepared_statement_cache();
    }
    /// Positive retained phases have fixed delta-first probes and stable native schemas.
    /// Their owner can forget storage bookkeeping while keeping those prepared plans.
    pub(in crate::code_rule_dsl) fn reset_execution_storage(&self) {
        *self.storage.borrow_mut() = Storage::default();
    }
    pub(in crate::code_rule_dsl) fn ensure_idle(&self) -> rusqlite::Result<()> {
        // SAFETY: this connection and its statements belong to the current worker.
        // SQLite retains the statement list until statements are finalized here.
        unsafe {
            let mut statement =
                rusqlite::ffi::sqlite3_next_stmt(self.handle(), std::ptr::null_mut());
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
    pub(in crate::code_rule_dsl) fn prepare_execution(
        &self,
        query: &str,
    ) -> rusqlite::Result<rusqlite::CachedStatement<'_>> {
        // Cached plans depend on native access paths and estimates, as well as SQL
        // schema. Zero-row bindings must not freeze scan plans for populated data.
        self.prepare_cached(&format!(
            "{query}\n/* native estimates {:x} */",
            self.plan_signature.get()
        ))
    }
    /// Binds `source` to the vtab `name` and refreshes the plan signature.
    pub(in crate::code_rule_dsl) fn bind(
        &self,
        name: &str,
        source: Arc<dyn Source>,
    ) -> rusqlite::Result<()> {
        self.modules.bind(self, name, Arc::clone(&source))?;
        let mut bound = self.bound.borrow_mut();
        bound.insert(name.to_owned(), source);
        self.plan_signature.set(plan_signature(&bound));
        Ok(())
    }
    /// Rebind one fact batch and calculate its planner signature once.
    pub(in crate::code_rule_dsl) fn bind_many<'a>(
        &self,
        sources: impl IntoIterator<Item = (&'a str, Arc<dyn Source>)>,
    ) -> rusqlite::Result<()> {
        let mut bound = self.bound.borrow_mut();
        for (name, source) in sources {
            self.modules.bind(self, name, Arc::clone(&source))?;
            bound.insert(name.to_owned(), source);
        }
        self.plan_signature.set(plan_signature(&bound));
        Ok(())
    }
    /// Empties every vtab: a later reader sees no rows rather than another lease's.
    pub(in crate::code_rule_dsl) fn clear_bindings(&self) {
        for name in std::mem::take(&mut *self.bound.borrow_mut()).into_keys() {
            if let Some(binding) = self.modules.get(&name) {
                binding.clear();
            }
        }
        self.plan_signature.set(0);
    }

    /// Remove session-owned modules as well as their rows when an extension session closes.
    pub(in crate::code_rule_dsl) fn remove_bindings(&self) -> rusqlite::Result<()> {
        self.ensure_idle()?;
        self.flush_prepared_statement_cache();
        let names: Vec<String> = self.bound.borrow().keys().cloned().collect();
        self.clear_bindings();
        for name in names {
            self.modules.unregister(self, &name)?;
        }
        Ok(())
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
            let code = unsafe {
                rusqlite::ffi::sqlite3_db_status64(
                    self.connection.handle(),
                    kind,
                    &mut current,
                    &mut high,
                    0,
                )
            };
            if code != rusqlite::ffi::SQLITE_OK {
                return Err(rusqlite::Error::SqliteFailure(
                    rusqlite::ffi::Error::new(code),
                    None,
                ));
            }
            total = total.saturating_add(usize::try_from(current.max(0)).unwrap_or(usize::MAX));
        }
        Ok(total)
    }
    /// A connection on `store`'s file with the symbol functions and the `demand` modules.
    pub(in crate::code_rule_dsl) fn open(store: &ContentStore) -> StoreResult<Self> {
        let connection = store.open_connection()?;
        // SAFETY: this field is declared before the owned connection.
        let profiler =
            unsafe { crate::code_rule_dsl::sql_profile::Profiler::configured(&connection)? };
        if let Some(p) = &profiler {
            p.context(serde_json::json!({"scope":"sql-owner","stage":"connection setup","executionPhase":"connectionSetup"}));
        }
        // Derived relations and their `new_`/`delta_` tables live in memory; the store's pages stay
        // its own (03 §4).
        connection.execute_batch("PRAGMA temp_store = MEMORY")?;
        register_symbols(&connection).map_err(|e| StoreError::Invalid(e.to_string()))?;
        let attached = match SETUP.get() {
            Some(setup) => Some(setup(&connection)?),
            None => None,
        };
        Ok(Self {
            profiler,
            connection,
            path: store.database_path().to_path_buf(),
            healthy: Cell::new(true),
            active: Cell::new(0),
            leases: Cell::new(0),
            plan_signature: Cell::new(0),
            storage: Default::default(),
            modules: Modules::default(),
            bound: Default::default(),
            attached,
        })
    }
    /// The owned connection, to set a hook of its own (an SQL session's step and time budget).
    pub(in crate::code_rule_dsl) fn connection_mut(&mut self) -> &mut Connection {
        &mut self.connection
    }
}

impl Deref for Database {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        &self.connection
    }
}

pub(crate) use crate::code_rule_dsl::facts::connection::{ConnectionSetup, SetUp};

static SETUP: std::sync::OnceLock<SetUp> = std::sync::OnceLock::new();

/// Registers what sets up each owned connection (once, at load).
pub(crate) fn install_connection_setup(setup: SetUp) {
    let _ = SETUP.set(setup);
}
