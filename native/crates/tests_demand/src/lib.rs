//! The engine's tests of demand: the demand-driven matcher, its virtual tables and its driver, and the contract and
//! policy suites of the reference catalog lowering (rerail-demand-catalog) (docs/final-shape/proposals/crate-split/
//! incremental-plan.md, steps 2 and 7i).
//!
//! A test crate above boot: it mounts the test files in place, under the facades the engine hosted them through,
//! and reads every other module through boot. Its test binary is booted as it loads (`boot::install_for_tests`), as
//! the engine's is. Written by tools/cratify/split_test_crates.py, and kept by hand since.
#![cfg(test)]
#![allow(dead_code, unused_imports, hidden_glob_reexports, ambiguous_glob_reexports, private_interfaces, private_bounds)]
#![allow(clippy::len_without_is_empty, clippy::result_unit_err, clippy::new_without_default)]
#![allow(clippy::type_complexity, clippy::too_many_arguments)]
pub use rerail_boot::{content_store, git_access, git_ignore, git_workspace, panic_boundary, source_tree, threads, tree_watch};
pub(crate) use rerail_boot::code_rule_dsl::prof::{lower_hex, purge_allocator_pages};

// The engine's test-binary rules: each test's time limit (`RERAIL_TEST_TIMEOUT`), and stores that flush nothing
// (`RERAIL_FLUSH=none`, decision 0481), both set before `main`.
#[path = "../../../src/test_watchdog.rs"]
mod test_watchdog;
#[path = "../../../src/test_flush.rs"]
mod test_flush;

/// The program, booted as the engine boots it (`boot::install`), with this binary's own build identities.
pub(crate) fn install_store_functions() {
    rerail_boot::code_rule_dsl::boot::install_for_tests();
}

mod test_load {
    extern "C" fn load() {
        super::install_store_functions();
    }
    #[used]
    #[cfg_attr(target_vendor = "apple", unsafe(link_section = "__DATA,__mod_init_func"))]
    #[cfg_attr(windows, unsafe(link_section = ".CRT$XCU"))]
    #[cfg_attr(not(any(target_vendor = "apple", windows)), unsafe(link_section = ".init_array"))]
    static LOAD: extern "C" fn() = load;
}

#[path = "../../../src/code_rule_dsl"]
pub mod code_rule_dsl {
    pub use rerail_boot::code_rule_dsl::*;
    pub use rerail_boot::code_rule_dsl::rule_api::CodeRuleDslEditor;
    #[cfg_attr(test, path = "demand")]
    mod demand {
        pub use rerail_boot::code_rule_dsl::demand::*;
        pub mod vtab {
            pub use rerail_boot::code_rule_dsl::demand::vtab::*;
            #[cfg(test)]
            #[path = "tests.rs"]
            pub mod tests;
            #[allow(unused_imports)]
            use super::super::language::Family;
            #[allow(unused_imports)]
            use super::super::transducer::packed::{ Error as PackedError, ExecutionBackend, PlanId, Program, execute_planned, packed_program_counts, packed_workspace_fits, packed_workspace_size, };
            #[allow(unused_imports)]
            use super::super::tape::TapeDoc;
            #[allow(unused_imports)]
            use super::super::value::structure::Equality;
            #[allow(unused_imports)]
            use super::super::value::Context as ValueContext;
            #[allow(unused_imports)]
            use ast_grep_core::{AstGrep, Doc};
            #[allow(unused_imports)]
            use fallible_iterator::FallibleIterator as _;
            #[allow(unused_imports)]
            use rusqlite::ffi;
            #[allow(unused_imports)]
            use rusqlite::vtab::{ Context, Filters, IndexConstraintOp, IndexInfo, Module, VTab, VTabConfig, VTabConnection, VTabCursor, };
            #[allow(unused_imports)]
            use rusqlite::{Connection, Result as SqlResult};
            #[allow(unused_imports)]
            use std::borrow::Cow;
            #[allow(unused_imports)]
            use std::collections::{BTreeSet, HashMap};
            #[allow(unused_imports)]
            use std::ffi::{CStr, c_int};
            #[allow(unused_imports)]
            use std::marker::PhantomData;
            #[allow(unused_imports)]
            use std::ops::Range;
            #[allow(unused_imports)]
            use std::panic::{AssertUnwindSafe, catch_unwind};
            #[allow(unused_imports)]
            use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
            #[allow(unused_imports)]
            use std::sync::{Arc, Mutex, OnceLock};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::pack as language_pack;
        }
        #[cfg_attr(test, path = "driver")]
        pub mod driver {
            pub use rerail_boot::code_rule_dsl::demand::driver::*;
            // The reference catalog lowering (rerail-demand-catalog), with the contract and policy suites that
            // drive it.
            #[cfg(test)]
            #[path = "catalog"]
            pub mod catalog {
                pub use rerail_demand_catalog::code_rule_dsl::demand::driver::catalog::*;
                mod policy_lowering_tests;
                mod production_contract_tests;
            }
            #[cfg(test)]
            pub mod tests;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::demand::vtab::{MatchRegistry, PackedDemand, register_module};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::transducer::packed::ExecutionBackend;
            #[allow(unused_imports)]
            use crate::content_store::{ContentStore, StoreResult};
            #[allow(unused_imports)]
            use rusqlite::{Connection, OptionalExtension as _, Row};
            #[allow(unused_imports)]
            use serde_json::{Value, json};
            #[allow(unused_imports)]
            use std::collections::BTreeMap;
            #[allow(unused_imports)]
            use std::sync::{Arc, LazyLock};
        }
    }
    #[cfg(test)]
    mod test_support;
}

// The engine's allocator (`rerail_prof::heap::Sampled`, as `rerail-engine`'s `lib.rs` declares it): without it the
// test binary ran on the system allocator, which no build of the engine runs on.
#[cfg(not(any(windows, target_family = "wasm", feature = "system-allocator")))]
#[global_allocator]
static GLOBAL: rerail_prof::heap::Sampled = rerail_prof::heap::Sampled;
