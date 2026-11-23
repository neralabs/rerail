//! The engine's tests of the .rr lowering and the program image: rr, its tapes, the program image and its text, and the JSON rules (docs/final-shape/proposals/crate-split/incremental-plan.md, step 7i).
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
    #[path = "logic"]
    pub(crate) mod logic {
        pub use rerail_boot::code_rule_dsl::logic::*;
        // Tests that read what the engine registers as it loads (the languages, their `.rerail` libraries) run here.
        pub mod program_image {
            pub use rerail_boot::code_rule_dsl::logic::program_image::*;
            // Its items, beside its tests here; the engine reads it through `logic`'s own re-export.
            #[cfg(test)]
            pub mod tests;
        }
        pub mod program_text {
            pub use rerail_boot::code_rule_dsl::logic::program_text::*;
            // Its items, beside its tests here; the engine reads it through `logic`'s own re-export.
            #[cfg(test)]
            pub mod tests;
        }
        // The JSON translator's tests (decision 0175) run in the engine's test binary.
        #[cfg_attr(test, path = "json_rules")]
        pub mod json_rules {
            pub use rerail_boot::code_rule_dsl::logic::json_rules::*;
            #[cfg(test)]
            pub mod tests;
        }
        #[path = "rr"]
        pub mod rr {
            pub use rerail_boot::code_rule_dsl::logic::rr::*;
            #[cfg(test)]
            pub mod tape_tests;
            #[cfg(test)]
            pub mod tests;
            #[allow(unused_imports)]
            use rerail_rr_syntax::syntax;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use rerail_rr_syntax::tape;
            #[allow(unused_imports)]
            use std::collections::BTreeSet;
            #[allow(unused_imports)]
            use std::path::{Path, PathBuf};
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::front::Error;
            #[allow(unused_imports)]
            #[cfg(test)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::rr::lower::drop_unread_core;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::rr::lower::is_core;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::rr::lower::WORLD;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use syntax::quote;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::rr::translate::translate;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::facts::vtab::syntax::core_rr::single_metavars;
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
