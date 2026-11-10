//! The engine's `code_rule_dsl` paths the production sources name, and the relational connection
//! its authorizer takes.
use rusqlite::{
    Connection, Result,
    hooks::{AuthContext, Authorization},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Deref,
    sync::{Arc, atomic::AtomicBool},
};

pub(crate) mod profile {
    pub(crate) use crate::clock::thread_cpu_ns;
    pub(crate) use crate::memory;
}
pub(crate) mod tape {
    pub(crate) use crate::wire::index_span;
}
pub(crate) mod budget {
    // Captured SQL has no active rule context. The controller supplies the hard deadline.
    pub(crate) fn step_budget_exhausted() -> bool {
        false
    }
}

// The engine's sources keep their own layout.
#[rustfmt::skip]
#[allow(dead_code)]
#[path = "../../../../native/src/code_rule_dsl/residency/admission/relational/authorizer.rs"]
mod authorizer;
#[rustfmt::skip]
#[allow(dead_code)]
#[path = "../../../../native/src/code_rule_dsl/hooks.rs"]
mod hooks;
#[cfg(test)]
#[allow(dead_code)]
#[path = "../../../../native/src/code_rule_dsl/residency/admission/relational/storage.rs"]
mod storage;

/// A relational connection with the engine's hook dispatchers, as its authorizer requires.
pub(crate) struct Database {
    connection: Connection,
    hooks: hooks::Hooks,
}

/// The tables and columns the authorized statements read, and whether one read the source.
pub(crate) struct Reads {
    pub(crate) fields: Arc<parking_lot::Mutex<BTreeMap<String, BTreeSet<String>>>>,
    // Read by the scoped-reader contract test.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) source: Arc<AtomicBool>,
}

impl Database {
    pub(crate) fn new(mut connection: Connection) -> Result<Self> {
        let hooks = hooks::Hooks::register(&mut connection)?;
        Ok(Self { connection, hooks })
    }

    /// Authorizes later statements with the production authorizer over `tables`.
    pub(crate) fn authorize(&self, tables: &BTreeSet<String>) -> Result<Reads> {
        let reads = authorizer::install(self, tables)?;
        Ok(Reads {
            fields: reads.fields,
            source: reads.source,
        })
    }

    /// Removes the authorizer: every action is allowed again.
    pub(crate) fn allow_all(&self) -> Result<()> {
        self.hooks
            .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
    }
}

impl Deref for Database {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        &self.connection
    }
}
