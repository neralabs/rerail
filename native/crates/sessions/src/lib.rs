//! The SQL executor (`sql_executor::CodeRuleSqlExecutor`): a session's connection, inputs and executions, its
//! documents, producers, catalog and updates, and the retained traces (`temporal`), over the executor's parts below it
//! (rerail-sessions-base), which it re-exports, so a reader above reads every executor path here, as before. The
//! parts below read the traces through `session::Trace` and their entry points through `maintenance::TEMPORAL`, which
//! boot registers (`temporal::register`).
//!
//! Its sources are in its own `src/`, at their module paths in the engine's tree (the crate split, step 9).
// A mounted module re-exports its dependency's by glob, and its sources import the same items
// under the same names: the glob's name and the private one are one item. The mounts also widen what the crates
// above read (the split's facades, not a design).
#![allow(dead_code, unused_imports, hidden_glob_reexports, private_interfaces, private_bounds)]
// What the split widened for the crates above is no public API (`len`, `()` errors, `new`); the long
// signatures and tuple types are the sources' own.
#![allow(clippy::len_without_is_empty, clippy::result_unit_err, clippy::new_without_default)]
#![allow(clippy::type_complexity, clippy::too_many_arguments)]
// The browser build has no JIT: wasm/cranelift-jit stands in under the same name (wasm/README.md).
#[cfg(target_family = "wasm")]
extern crate cranelift_jit_wasm as cranelift_jit;
pub use rerail_sessions_base::{content_store, git_access, git_workspace, threads};
extern crate self as rerail_sessions;
#[path = "code_rule_dsl"]
pub mod code_rule_dsl {
    pub use rerail_sessions_base::code_rule_dsl::*;
    // The logic language with its rr root and corpus (rerail-rr-root), which the executor reads (`rr::parse_version`,
    // `plan::corpus::preparation`); the parts below read the planner's (rerail-corpus-memo).
    pub use rerail_rr_root::code_rule_dsl::logic;
    #[path = "sql_executor"]
    pub mod sql_executor {
        pub use rerail_sessions_base::code_rule_dsl::sql_executor::*;
        pub mod catalog;
        pub mod compiler;
        pub mod documents;
        pub mod executor;
        pub mod explain;
        pub mod producers;
        pub mod profiling;
        pub mod temporal;
        pub mod updates;
        #[cfg(any(any(test, feature = "test-hooks"), feature = "headless"))]
        pub mod verification;
        pub use executor::CodeRuleSqlExecutor;
        // The executor's own names, which its parts here read (`use super::*`).
        use executor::{reserved, session_program};
        // `sql_executor.rs`'s own imports, which its children read (`use super::*`).
        use super::facts::core::{self, Built};
        use super::facts::vtab::{Relation, Source, syntax};
        use super::logic::native::lattices;
        use super::logic::program_image::{Builder, Cell, Version, fields::ProgramFields, manager::Pipeline};
        use super::logic::sql::program;
        use super::logic::sql::select;
        use super::logic::{plan, rr, strings};
        use super::waves::{owner::Database, schedule};
        use crate::content_store::ContentStore;
        use napi::{Error, Result};
        use napi_derive::napi;
        use rusqlite::OptionalExtension;
        use std::path::Path;
    }
}

/// This crate's own test binary, which boot does not load: the retained traces its tests run are registered as the
/// binary loads, as boot registers them (`install_with`).
#[cfg(test)]
mod test_load {
    extern "C" fn load() {
        crate::code_rule_dsl::sql_executor::temporal::register();
    }
    #[used]
    #[cfg_attr(target_vendor = "apple", unsafe(link_section = "__DATA,__mod_init_func"))]
    #[cfg_attr(not(target_vendor = "apple"), unsafe(link_section = ".init_array"))]
    static LOAD: extern "C" fn() = load;
}
