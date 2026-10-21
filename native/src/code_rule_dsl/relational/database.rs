//! Own the optional trace callback before its connection, including failed sessions.
use rusqlite::Connection;
use std::ops::Deref;

pub(in crate::code_rule_dsl) struct Database {
    #[cfg(feature = "code-rule-profile")]
    profiler: Option<Box<crate::code_rule_dsl::profile::sql::Profiler>>,
    connection: Connection,
}

impl Database {
    pub(super) fn open() -> rusqlite::Result<Self> {
        let connection = Connection::open_in_memory()?;
        #[cfg(feature = "code-rule-profile")]
        // SAFETY: the stable boxed callback drops before this owned connection.
        let profiler =
            unsafe { crate::code_rule_dsl::profile::sql::Profiler::configured(&connection)? };
        Ok(Self {
            #[cfg(feature = "code-rule-profile")]
            profiler,
            connection,
        })
    }

    #[cfg(feature = "code-rule-profile")]
    pub(super) fn context(&self, value: serde_json::Value) {
        if let Some(profiler) = &self.profiler {
            profiler.context(value);
        }
    }

    #[cfg(feature = "code-rule-profile")]
    pub(super) fn stage(&self, name: &str) {
        if let Some(profiler) = &self.profiler {
            profiler.stage(name);
        }
    }

    #[cfg(feature = "code-rule-profile")]
    pub(super) fn operation(
        &self,
        name: &str,
        index: usize,
    ) -> Option<crate::code_rule_dsl::profile::sql::Operation<'_>> {
        self.profiler
            .as_ref()
            .map(|p| p.operation(name, ("relational", index), 0, Vec::new()))
    }

    #[cfg(feature = "code-rule-profile")]
    pub(super) fn checkpoint(&self) -> rusqlite::Result<()> {
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
