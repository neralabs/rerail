//! Own the optional trace callback before its connection, including failed sessions.
use rusqlite::Connection;
use std::ops::Deref;

pub(in crate::code_rule_dsl) struct Database {
    #[cfg(feature = "code-rule-profile")]
    profiler: Option<Box<crate::code_rule_dsl::profile::sql::Profiler>>,
    connection: Connection,
    pub(in crate::code_rule_dsl) storage: std::cell::RefCell<super::storage::Storage>,
}

impl Database {
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
            storage: Default::default(),
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
