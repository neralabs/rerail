//! The engine's tests of the matchers' lowering and admission: the composite matcher's lowering, the admission experiments, greedy contracts and cycle progress (docs/final-shape/proposals/crate-split/incremental-plan.md, step 7i).
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
#[path = "../../../src/test_private_folders.rs"]
mod test_private_folders;

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
    #[cfg_attr(test, path = "transducer")]
    mod transducer {
        pub use rerail_boot::code_rule_dsl::transducer::*;
        #[cfg_attr(test, path = "packed")]
        pub mod packed {
            pub use rerail_boot::code_rule_dsl::transducer::packed::*;
            #[cfg(test)]
            #[path = "compile/matcher_tests.rs"]
            pub mod compile_matcher_tests;
            // Read by simdjson's registration (not wasm: `install_languages`) and the tests.
            #[cfg(any(test, feature = "test-hooks", not(target_family = "wasm")))]
            #[cfg_attr(not(test), allow(unused_imports))]
            pub use rerail_lang_json::code_rule_dsl::transducer::packed::json_parser;
            #[cfg(all(any(test, feature = "test-hooks"), any(target_os = "macos", target_os = "linux", windows)))]
            #[allow(unused_imports)]
            pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_join;
            #[cfg(all(test, any(target_os = "macos", target_os = "linux", windows)))]
            pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_rows;
            #[cfg(all(test, any(target_os = "macos", target_os = "linux", windows)))]
            pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_sort;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::tape::TapeDoc;
            #[allow(unused_imports)]
            use ast_grep_core::MatchStrictness;
            #[allow(unused_imports)]
            use ast_grep_core::matcher::{Pattern, PatternNode};
            #[allow(unused_imports)]
            use ast_grep_core::meta_var::{MetaVarEnv, MetaVariable};
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use regex::Regex;
            #[allow(unused_imports)]
            use sha2::{Digest as _, Sha256};
            #[allow(unused_imports)]
            use std::borrow::Cow;
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
            #[allow(unused_imports)]
            use std::fmt::Write as _;
            #[allow(unused_imports)]
            use std::ops::Range;
            #[allow(unused_imports)]
            use std::sync::Arc;
            #[cfg(test)]
            #[allow(unused_imports)]
            use crate::code_rule_dsl::transducer::packed::literal_anchors::*;
            #[allow(unused_imports)]
            #[cfg(all(any(test, feature = "test-hooks"), any(target_os = "macos", target_os = "linux", windows)))]
            pub(super) use crate::code_rule_dsl::transducer::packed::metal_backend::launch::LaunchScope as MetalTestScope;
            #[allow(unused_imports)]
            #[cfg(any(target_os = "macos", target_os = "linux", windows))]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::metal_program_backend::set_steps;
            #[allow(unused_imports)]
            #[cfg(any(target_os = "macos", target_os = "linux", windows))]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::metal_program_backend::lattice as metal_lattice;
            #[allow(unused_imports)]
            #[cfg(any(target_os = "macos", target_os = "linux", windows))]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::metal_program_backend::nearest as metal_nearest;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::device::{RootJobs, RootsFound};
            #[allow(unused_imports)]
            #[cfg(any(target_os = "macos", target_os = "linux", windows))]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::compile_matcher::matcher_metal;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::compile_matcher::{Backend as MatcherBackend, LoweredMatcher, LoweredWave, WaveRun};
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::compile_micro::{Emit, Frames, Micro, RootRecord, steps as micro_steps};
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::compilation::PlanId;
            #[allow(unused_imports)]
            use rerail_compiler::native_cpu_codegen;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use rerail_cpu_block::native_cpu_rows;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::packed_batch::PackedExecution;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use rerail_compiler::block;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use rerail_rr_parser as rr_parser;
            #[allow(unused_imports)]
            use rerail_compiler::program::*;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use rerail_compiler::program::{NAMED, Words, word};
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use rerail_compiler::Error;
            #[allow(unused_imports)]
            #[cfg(any(test, feature = "test-hooks"))]
            use literal_anchors::*;
            #[cfg(test)]
            #[path = "verify/cycle_progress_tests.rs"]
            pub mod cycle_progress_tests;
            #[cfg(test)]
            #[path = "verify/greedy_contract_tests.rs"]
            pub mod greedy_contract_tests;
            // The admission spike's tests parse Python, which the engine registers: they run here, and the transducer
            // keeps the spike only for its test hooks (`metal_frontier_conformance` reads a program of it).
            #[cfg(test)]
            #[path = "admission_spike.rs"]
            pub mod admission_spike;
        }
        #[allow(unused_imports)]
        #[cfg(test)]
        use crate::code_rule_dsl::tape::TapeDoc;
        #[allow(unused_imports)]
        #[cfg(test)]
        use ast_grep_core::MatchStrictness;
        #[allow(unused_imports)]
        #[cfg(test)]
        use ast_grep_core::matcher::{Pattern, PatternNode};
        #[allow(unused_imports)]
        #[cfg(test)]
        use regex::Regex;
        #[allow(unused_imports)]
        #[cfg(test)]
        use std::collections::{BTreeMap, BTreeSet};
        #[allow(unused_imports)]
        #[cfg(test)]
        use std::ops::Range;
        #[allow(unused_imports)]
        #[cfg(test)]
        use crate::code_rule_dsl::search_index::persist_search_index_batch;
    }
}
