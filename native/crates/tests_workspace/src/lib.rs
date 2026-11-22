//! The engine's tests of the workspace: its runs over a repository and the context they read (docs/final-shape/proposals/crate-split/incremental-plan.md, step 7i).
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
    #[cfg(test)]
    mod test_support;
    #[cfg_attr(test, path = "workspace")]
    pub(crate) mod workspace {
        pub use rerail_boot::code_rule_dsl::workspace::*;
        #[cfg(test)]
        pub mod context_tests;
        #[cfg(test)]
        pub mod tests;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::json::invalid;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::model::{CompiledClause, CompiledCodeRule, FileInput};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::scope::ScopeTable;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::tape;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::violations::Witness;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::GrammarDialect;
        #[allow(unused_imports)]
        use crate::content_store::StoredFile;
        #[allow(unused_imports)]
        use napi::Result;
        #[allow(unused_imports)]
        #[cfg(test)]
        use std::cell::{Cell, RefCell};
        #[allow(unused_imports)]
        use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
        #[allow(unused_imports)]
        use std::sync::atomic::{AtomicBool, Ordering};
        #[allow(unused_imports)]
        use std::sync::{Arc, OnceLock};
        #[allow(unused_imports)]
        use std::time::{Duration, Instant};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::workspace::storage::StorageContext;
    }
}
