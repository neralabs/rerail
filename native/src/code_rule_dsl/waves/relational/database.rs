//! Own the optional trace callback before its connection, including failed sessions.
use rusqlite::Connection;
use std::ops::Deref;

type ProgramCache =
    std::collections::BTreeMap<Vec<(String, String)>, std::sync::Arc<Vec<super::Stage>>>;
pub(in crate::code_rule_dsl) struct Database {
    #[cfg(feature = "code-rule-profile")]
    profiler: Option<Box<crate::code_rule_dsl::profile::sql::Profiler>>,
    connection: Connection,
    pub(super) regex_binding: super::functions::SharedBinding,
    pub(super) source_binding: super::source::SharedBinding,
    pub(super) regex_installed: std::cell::Cell<bool>,
    pub(super) source_installed: std::cell::Cell<bool>,
    pub(super) healthy: std::cell::Cell<bool>,
    pub(super) active: std::cell::Cell<usize>,
    pub(super) plan_signature: std::cell::Cell<u64>,
    pub(super) storage: std::cell::RefCell<super::storage::Storage>,
    pub(super) bindings: std::cell::RefCell<super::shared::Bindings>,
    pub(super) program_bytes: std::cell::Cell<usize>,
    pub(super) descriptor_bytes: std::cell::Cell<usize>,
    pub(super) programs: std::cell::RefCell<ProgramCache>,
    pub(super) record_relations: bool,
    pub(super) modules:
        std::cell::RefCell<std::collections::BTreeMap<String, super::table::Binding>>,
}

impl Database {
    #[cfg(feature = "code-rule-profile")]
    pub(super) fn observe_context(&self, boundary: &str) {
        let mut span =
            crate::code_rule_dsl::profile::trace::Span::start("sql-context-inventory", boundary);
        span.detail(||{
            let schema=(||->rusqlite::Result<std::collections::BTreeMap<String,i64>> {
                self.prepare("SELECT type,count(*) FROM temp.sqlite_schema GROUP BY type")?.query_map([],|r|Ok((r.get(0)?,r.get(1)?)))?.collect()
            })();
            let mut prepared=0;
            // SAFETY: the connection's statement list stays live on this worker.
            unsafe {
                let mut statement=rusqlite::ffi::sqlite3_next_stmt(self.handle(),std::ptr::null_mut());
                while !statement.is_null() {prepared+=1;statement=rusqlite::ffi::sqlite3_next_stmt(self.handle(),statement);}
            }
            serde_json::json!({"schema":schema.ok(),"preparedStatements":prepared,"nativeModules":self.modules.borrow().len(),"physicalPrograms":self.programs.borrow().len(),
                "programBytes":self.program_bytes.get(),"regexRegistrations":usize::from(self.regex_installed.get())*3,"sourceRegistrations":usize::from(self.source_installed.get()),
                "scopedStorage":self.storage.borrow().inventory(),"residentSqlBytes":self.resident_bytes().ok()})
        });
    }
    pub(super) fn ensure_idle(&self) -> rusqlite::Result<()> {
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
    pub(super) fn prepare_execution(
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
        Ok(total
            .saturating_add(self.program_bytes.get())
            .saturating_add(self.descriptor_bytes.get()))
    }
    pub(in crate::code_rule_dsl) fn open() -> rusqlite::Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    pub(in crate::code_rule_dsl) fn from_connection(
        connection: Connection,
    ) -> rusqlite::Result<Self> {
        #[cfg(feature = "code-rule-profile")]
        // SAFETY: the stable boxed callback drops before this owned connection.
        let profiler =
            unsafe { crate::code_rule_dsl::profile::sql::Profiler::configured(&connection)? };
        Ok(Self {
            #[cfg(feature = "code-rule-profile")]
            profiler,
            connection,
            regex_binding: super::functions::binding(),
            source_binding: super::source::binding(),
            regex_installed: std::cell::Cell::new(false),
            source_installed: std::cell::Cell::new(false),
            healthy: std::cell::Cell::new(true),
            active: std::cell::Cell::new(0),
            plan_signature: std::cell::Cell::new(0),
            storage: Default::default(),
            bindings: Default::default(),
            modules: Default::default(),
            programs: Default::default(),
            record_relations: cfg!(feature = "code-rule-profile")
                && std::env::var_os("RERAIL_SQL_PROFILE_DIR").is_some(),
            program_bytes: std::cell::Cell::new(0),
            descriptor_bytes: std::cell::Cell::new(0),
        })
    }

    #[cfg(feature = "code-rule-profile")]
    pub(in crate::code_rule_dsl) fn context(&self, value: serde_json::Value) {
        if let Some(profiler) = &self.profiler {
            profiler.context(value);
        }
    }

    #[cfg(feature = "code-rule-profile")]
    pub(in crate::code_rule_dsl) fn stage(&self, name: &str) {
        if let Some(profiler) = &self.profiler {
            profiler.stage(name);
        }
    }

    #[cfg(feature = "code-rule-profile")]
    pub(in crate::code_rule_dsl) fn operation(
        &self,
        name: &str,
        index: usize,
    ) -> Option<crate::code_rule_dsl::profile::sql::Operation<'_>> {
        self.profiler
            .as_ref()
            .map(|p| p.operation(name, ("relational", index), 0, Vec::new()))
    }

    #[cfg(feature = "code-rule-profile")]
    pub(in crate::code_rule_dsl) fn explain_pending(&self) {
        if let Some(profiler) = &self.profiler {
            profiler.explain_pending(&self.connection);
        }
    }

    #[cfg(feature = "code-rule-profile")]
    pub(in crate::code_rule_dsl) fn checkpoint(&self) -> rusqlite::Result<()> {
        if let Some(profiler) = &self.profiler {
            profiler.checkpoint(&self.connection)?;
        }
        Ok(())
    }
}

impl Deref for Database {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        &self.connection
    }
}

#[cfg(feature = "code-rule-profile")]
impl Drop for Database {
    fn drop(&mut self) {
        if let Err(error) = self.checkpoint() {
            eprintln!("SQL profile checkpoint failed: {error}");
        }
    }
}
