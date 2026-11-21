//! The engine's tests of the transducer: the packed programs, their backends and the Bitlex lowering (docs/final-shape/proposals/crate-split/incremental-plan.md, step 2).
//!
//! A test crate above boot: it mounts the test files in place, under the facades the engine hosted them through,
//! and reads every other module through boot. Its test binary is booted as it loads (`boot::install_for_tests`), as
//! the engine's is. Written by tools/cratify/test_crates.py, and kept by hand since.
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
    #[cfg_attr(test, path = "transducer")]
    mod transducer {
        pub use rerail_boot::code_rule_dsl::transducer::*;
        #[cfg_attr(test, path = "packed")]
        pub mod packed {
            pub use rerail_boot::code_rule_dsl::transducer::packed::*;
            pub mod set_mode {
                pub use rerail_boot::code_rule_dsl::transducer::packed::set_mode::*;
                #[cfg(test)]
                #[path = "tests.rs"]
                pub mod tests;
                #[allow(unused_imports)]
                #[cfg(any(test, feature = "test-hooks"))]
                use super::*;
            }
            // `.`: no directory `rir/` exists to resolve `..` through.
            #[cfg_attr(test, path = ".")]
            pub mod rir {
                pub use rerail_boot::code_rule_dsl::transducer::packed::rir::*;
                // The grammar's names; the compiler's `program::Names` comes in by `super::*` too.
                #[cfg_attr(not(test), allow(unused_imports))]
                pub use rerail_transducer::code_rule_dsl::transducer::packed::rir::Names;
                #[cfg(test)]
                #[path = "rir_tests.rs"]
                pub mod tests;
                #[allow(unused_imports)]
                use rerail_compiler::rir_asm::{Line, NameTable, Token, block_programs_rows, head_rows, index, tokens, word_row, word_rows};
                #[allow(unused_imports)]
                use super::*;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::language::{GrammarDialect, KindCatalog};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::tape::DERIVED_CLASS_BASE;
                #[allow(unused_imports)]
                use std::collections::HashMap as Map;
            }
            // `.`: no directory `compilation/` exists to resolve `..` through.
            #[cfg_attr(test, path = ".")]
            pub mod compilation {
                pub use rerail_boot::code_rule_dsl::transducer::packed::compilation::*;
                #[cfg(test)]
                #[path = "compilation_tests.rs"]
                pub mod tests;
                #[allow(unused_imports)]
                use super::{Error, ExecutionBackend, Program, native_cpu};
                #[allow(unused_imports)]
                use sha2::{Digest as _, Sha256};
                #[allow(unused_imports)]
                use std::collections::BTreeSet;
                #[allow(unused_imports)]
                use std::sync::{LazyLock, Mutex};
            }
            // `.`: no directory `capture_run/` exists to resolve `..` through.
            #[cfg_attr(test, path = ".")]
            pub mod capture_run {
                pub use rerail_boot::code_rule_dsl::transducer::packed::capture_run::*;
                #[cfg(test)]
                #[path = "capture_run_stream_tests.rs"]
                pub mod stream_tests;
                #[allow(unused_imports)]
                use super::*;
            }
            pub mod regex_block {
                pub use rerail_boot::code_rule_dsl::transducer::packed::regex_block::*;
                #[cfg(test)]
                            pub mod tests;
                #[allow(unused_imports)]
                use super::block::{BlockOp, BlockProgram, ClassTable};
                #[allow(unused_imports)]
                use regex_syntax::hir::{Class, Hir, HirKind, Look};
                #[allow(unused_imports)]
                use std::collections::HashMap;
            }
            pub mod native_cpu {
                pub use rerail_boot::code_rule_dsl::transducer::packed::native_cpu::*;
                #[cfg(test)]
                            pub mod tests;
                #[allow(unused_imports)]
                use super::{ BatchOutcome, Error, Output, Program, metal_batch_abi::{ ARENA_KIND_COUNT, FORMAT_VERSION, FinalCaptureRecord, GreedyEntryRecord, LinkRecord, LuaInstructionRecord, LuaSetMemberRecord, MetalNode, MetalSupertype, NONE, NfaRangeRecord, NfaStateRecord, PENDING_FOLLOWING, PENDING_GREEDY, PENDING_PATH, PathRecord, PendingRecord, PropertyRecord, QueryBucketRecord, QueryMemberRecord, QueuedStateRecord, RepeatRecord, ResultRecord, TaskStatus, ValueRecord, }, native_cpu_capture, native_cpu_output, native_cpu_reduction, native_cpu_scheduler, native_cpu_value, native_ir::{ArenaKind, FunctionRef, NativeModule, StructuralOperation}, packed_batch::{PackedBatch, PackedExecution, PackedResultPage}, value_packing::PackedValueTables, };
                #[allow(unused_imports)]
                use crate::code_rule_dsl::tape::TapeDoc;
                #[allow(unused_imports)]
                use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlags, condcodes::IntCC, types};
                #[allow(unused_imports)]
                use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Switch};
                #[allow(unused_imports)]
                use cranelift_jit::{JITBuilder, JITModule};
                #[allow(unused_imports)]
                use cranelift_module::{Linkage, Module, default_libcall_names};
                #[allow(unused_imports)]
                use rayon::prelude::*;
                #[allow(unused_imports)]
                use std::collections::BTreeMap;
                #[allow(unused_imports)]
                use std::marker::PhantomData;
                #[allow(unused_imports)]
                use std::mem;
                #[allow(unused_imports)]
                use std::ops::{Deref, DerefMut};
                #[allow(unused_imports)]
                use std::ptr::NonNull;
                #[allow(unused_imports)]
                use std::sync::{Arc, LazyLock, Mutex};
            }
            #[cfg(test)]
            #[path = "compile/matcher_tests.rs"]
            pub mod compile_matcher_tests;
            // Read by simdjson's registration (not wasm: `install_languages`) and the tests.
            #[cfg(any(test, feature = "test-hooks", not(target_family = "wasm")))]
            #[cfg_attr(not(test), allow(unused_imports))]
            pub use rerail_lang_json::code_rule_dsl::transducer::packed::json_parser;
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            pub mod metal_backend {
                pub use rerail_boot::code_rule_dsl::transducer::packed::metal_backend::*;
                // A launch-budget test that parses a document: it runs where the languages are registered.
                #[cfg(test)]
                pub mod engine_tests;
            }
            #[cfg(all(any(test, feature = "test-hooks"), any(target_os = "macos", target_os = "linux")))]
            #[allow(unused_imports)]
            pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_join;
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            #[cfg_attr(test, path = ".")]
            pub mod metal_program_backend {
                pub use rerail_boot::code_rule_dsl::transducer::packed::metal_program_backend::*;
                // The tape tests store parsed documents: they run where the languages are registered.
                #[cfg(test)]
                #[path = "metal_tape_tests.rs"]
                pub mod tape_tests;
            }
            // Metal's semantic fixtures parse documents: they run where the languages are registered.
            #[cfg(all(test, any(target_os = "macos", target_os = "linux"), feature = "metal-tests"))]
            pub mod metal_semantic_conformance;
            #[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
            pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_rows;
            #[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
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
            #[cfg(all(any(test, feature = "test-hooks"), any(target_os = "macos", target_os = "linux")))]
            pub(super) use crate::code_rule_dsl::transducer::packed::metal_backend::launch::LaunchScope as MetalTestScope;
            #[allow(unused_imports)]
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::metal_program_backend::set_steps;
            #[allow(unused_imports)]
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::metal_program_backend::lattice as metal_lattice;
            #[allow(unused_imports)]
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::metal_program_backend::nearest as metal_nearest;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::device::{RootJobs, RootsFound};
            #[allow(unused_imports)]
            #[cfg(any(target_os = "macos", target_os = "linux"))]
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
            pub(in crate::code_rule_dsl) use rerail_compiler::native_cpu_rows;
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
            #[cfg(test)]
                    pub mod capture_run_entry_tests;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            #[cfg(any(test, feature = "test-hooks"))]
            use literal_anchors::*;
            #[cfg(test)]
            #[path = "capture_run_tests.rs"]
            pub mod capture_run_tests;
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
        #[cfg(test)]
        #[path = "tests.rs"]
        pub mod tests;
    }
}
