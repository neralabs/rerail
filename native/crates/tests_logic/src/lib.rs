//! The engine's tests of logic and the sessions: the planner, the .rr lowering, the program image, the native set kernels, typing, the SQL executor, the statistics and the residency (docs/final-shape/proposals/crate-split/incremental-plan.md, step 2).
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
    // The waves' tests (the owner's, the relational executor's, the schedule's), under the facade they were
    // mounted through (`waves.rs`).
    #[allow(dead_code)]
    mod waves;
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
        // Tests that read what the engine registers as it loads (the languages, their `.rerail` libraries) run here.
        pub mod liveness {
            pub use rerail_boot::code_rule_dsl::logic::liveness::*;
            #[cfg(test)]
            pub mod tests;
        }
        // The cost statistics' read of each lane and form (3a42aefcf).
        #[cfg(test)]
        mod costs_tests;
        // The JSON translator's tests (decision 0175) run in the engine's test binary.
        #[cfg_attr(test, path = "json_rules")]
        pub mod json_rules {
            pub use rerail_boot::code_rule_dsl::logic::json_rules::*;
            #[cfg(test)]
            pub mod tests;
        }
        #[cfg_attr(test, path = "native")]
        pub mod native {
            pub use rerail_boot::code_rule_dsl::logic::native::*;
            #[cfg_attr(test, path = "reach")]
            pub mod reach {
                pub use rerail_boot::code_rule_dsl::logic::native::reach::*;
                #[cfg(test)]
                pub mod tests;
            }
            #[cfg_attr(test, path = "setfix")]
            pub mod setfix {
                pub use rerail_boot::code_rule_dsl::logic::native::setfix::*;
                // The lab's tests run strata on the Metal device (the transducer's), which the engine alone sees.
                #[cfg_attr(test, path = "lab")]
                pub mod lab {
                    pub use rerail_boot::code_rule_dsl::logic::native::setfix::lab::*;
                    #[cfg(test)]
                    pub mod tests;
                }
                #[allow(unused_imports)]
                use crate::code_rule_dsl::logic::analysis::scalars::Kind as ScalarKind;
                #[allow(unused_imports)]
                use rusqlite::ffi;
                #[allow(unused_imports)]
                use rusqlite::types::{Value, ValueRef};
                #[allow(unused_imports)]
                use rusqlite::vtab::{Context as VContext, CreateVTab, Filters, IndexConstraintOp, IndexInfo, VTab, VTabConnection, VTabCursor, VTabKind};
                #[allow(unused_imports)]
                use rusqlite::{Connection, Error, Result};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::facts::vtab::Scoped;
                #[allow(unused_imports)]
                use std::borrow::Cow;
                #[allow(unused_imports)]
                use hashbrown::HashTable;
                #[allow(unused_imports)]
                use rustc_hash::{FxHashMap as HashMap, FxHasher};
                #[allow(unused_imports)]
                use std::ffi::{CStr, CString};
                #[allow(unused_imports)]
                use std::hash::Hasher;
                #[allow(unused_imports)]
                use std::os::raw::c_int;
                #[allow(unused_imports)]
                use std::sync::{Arc, LazyLock, Mutex, RwLock};
                #[allow(unused_imports)]
                use std::time::Instant;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::logic::native::setfix::spec::{ComponentSpec, InputKey, JoinItem, JoinSpec, Line, Reprobe, Source, Start, Tok};
                #[allow(unused_imports)]
                pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::device::{Report as DeviceReport, take_report as device_report};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::logic::device::{JoinPlan};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::logic::profile::{Unit, work as charge};
            }
            #[cfg(test)]
            pub mod tests;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::native::setfix::{forget_engine, forget_scope};
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::native::setfix::{flush_symbols, forget_inputs, intern_batch, keep_inputs, register_symbols, sym_text_of};
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::native::setfix::symbol_text;
            #[allow(unused_imports)]
            use rusqlite::ffi;
            #[allow(unused_imports)]
            use rusqlite::functions::{Aggregate, Context, FunctionFlags};
            #[allow(unused_imports)]
            use rusqlite::vtab::{Context as VContext, CreateVTab, Filters, IndexConstraintOp, IndexInfo, Module, VTab, VTabConnection, VTabCursor, VTabKind};
            #[allow(unused_imports)]
            use rusqlite::{Connection, Error, Result};
            #[allow(unused_imports)]
            use std::borrow::Cow;
            #[allow(unused_imports)]
            use std::ffi::CStr;
            #[allow(unused_imports)]
            use std::sync::Mutex;
            #[allow(unused_imports)]
            use std::os::raw::c_int;
            #[allow(unused_imports)]
            use std::sync::{Arc, LazyLock};
            #[allow(unused_imports)]
            use std::time::Instant;
        }
        #[cfg_attr(test, path = "plan")]
        pub mod plan {
            pub use rerail_boot::code_rule_dsl::logic::plan::*;
            // Tests that read what the engine registers as it loads (the languages, their `.rerail` libraries) run here.
            pub mod compose {
                pub use rerail_boot::code_rule_dsl::logic::plan::compose::*;
                #[cfg(test)]
                pub mod engine_tests;
            }
            // Tests that read what the engine registers as it loads (the languages, their `.rerail` libraries) run here.
            pub mod native_demand {
                pub use rerail_boot::code_rule_dsl::logic::plan::native_demand::*;
                #[cfg(test)]
                pub mod engine_tests;
            }
            // Only its tests are files of their own.
            #[cfg_attr(test, path = "corpus")]
            pub mod corpus {
                pub use rerail_boot::code_rule_dsl::logic::plan::corpus::*;
                // The base tests compile and evaluate rule documents through the engine (`test_support`).
                #[cfg(test)]
                pub mod base_tests;
                // Decision 0225 §2: partial memo hits hand on what a full evaluation does.
                #[cfg(test)]
                pub mod partial_tests;
                // Decision 0364's required shape (R1–R10): what a session's edit does, one test per requirement.
                #[cfg(test)]
                pub mod maintain_tests;
                // Decision 0458 (R1–R5): the cross part reads the base relations off every file's tape.
                #[cfg(test)]
                pub mod cross_views_tests;
                // Decision 0494: a batch makes a document when first read.
                #[cfg(test)]
                pub mod made_batch_tests;
                // Q332: what a warm run does to the files that did not change, by count (the `warm` nextest profile).
                #[cfg(test)]
                pub mod warm_tests;
                #[cfg(test)]
                pub mod cache_tests;
                // Tests that read what the engine registers as it loads (the languages, their `.rerail` libraries) run here.
                pub mod phase0 {
                    pub use rerail_boot::code_rule_dsl::logic::plan::corpus::phase0::*;
                    #[cfg(test)]
                    pub mod tests;
                }
                // Tests that read what the engine registers as it loads (the languages, their `.rerail` libraries) run here.
                pub mod phases {
                    pub use rerail_boot::code_rule_dsl::logic::plan::corpus::phases::*;
                    #[cfg(test)]
                    pub mod tests;
                }
                // Tests that read what the engine registers as it loads (the languages, their `.rerail` libraries) run here.
                pub mod prepared {
                    pub use rerail_boot::code_rule_dsl::logic::plan::corpus::prepared::*;
                    #[cfg(test)]
                    pub mod tests;
                    #[cfg(test)]
                    pub mod identity_tests;
                }
                // Tests that read what the engine registers as it loads (the languages, their `.rerail` libraries) run here.
                pub mod reuse {
                    pub use rerail_boot::code_rule_dsl::logic::plan::corpus::reuse::*;
                    #[cfg(test)]
                    pub mod tests;
                }
                // Tests that read what the engine registers as it loads (the languages, their `.rerail` libraries) run here.
                pub mod session {
                    pub use rerail_boot::code_rule_dsl::logic::plan::corpus::session::*;
                    #[cfg(test)]
                    pub mod tests;
                }
                // The corpus runner's tests: they parse files, so they run where the languages are registered.
                #[cfg(test)]
                pub mod tests;
                // Markdown's inline guests against tree-sitter-md's inline trees: they parse files.
                #[cfg(test)]
                pub mod guests_tests;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::logic::plan::{Error, Outcome, Result, lower};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::facts::core::{self, Built};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::facts::vtab::{Modules, syntax};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::language::GrammarDialect;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::logic::front;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::logic::rr::WORLD;
                #[allow(unused_imports)]
                pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::plan::corpus::hiding::qualified;
                #[allow(unused_imports)]
                pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::plan::corpus::inputs::parameters as input_parameters;
                #[allow(unused_imports)]
                pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::plan::corpus::matchers::columns as ast_columns;
                #[allow(unused_imports)]
                pub(crate) use crate::code_rule_dsl::logic::plan::corpus::typing::{Typing, TypingReport};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::typing::file_local::Binding;
                #[allow(unused_imports)]
                use crate::content_store::ContentStore;
                #[allow(unused_imports)]
                use rusqlite::Connection;
                #[allow(unused_imports)]
                use rusqlite::types::Value as SqlValue;
                #[allow(unused_imports)]
                use std::collections::HashMap;
                #[allow(unused_imports)]
                use std::path::Path;
                #[allow(unused_imports)]
                use std::sync::Arc;
                #[allow(unused_imports)]
                pub(crate) use crate::code_rule_dsl::prof::thread_usage;
                #[allow(unused_imports)]
                use std::collections::{BTreeMap, BTreeSet};
            }
            #[cfg(test)]
            pub mod tests;
            #[cfg(test)]
            pub mod coverage;
            #[cfg(test)]
            pub mod first_tests;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::logic::front::{AggOp, Value};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::logic::native::TOP;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::logic::native::component::spec::{Arg, ComponentSpec, EdgeSpec, Extreme, HeadTok, InputKey, JoinItem, JoinSpec, Line as SpecLine, Reprobe, Source as JoinSource, Start, Tok};
            #[allow(unused_imports)]
            use std::sync::Arc;
            #[allow(unused_imports)]
            use indexmap::IndexMap;
            #[allow(unused_imports)]
            use rusqlite::Connection;
            #[allow(unused_imports)]
            use rusqlite::types::Value as SqlValue;
            #[allow(unused_imports)]
            use std::cell::RefCell;
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
            #[allow(unused_imports)]
            use std::fmt;
            #[allow(unused_imports)]
            use std::path::Path;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::logic::sql::select::{Cte, FromItem, Join, Query, Relation, Select, Source};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::logic::sql::sql_ast::{SqlBinaryOperator as Op, SqlExpression as Expr, SqlUnaryOperator, SqlValue as Value_};
        }
        #[cfg_attr(test, path = "profile")]
        pub mod profile {
            pub use rerail_boot::code_rule_dsl::logic::profile::*;
            // The profile's account of every thread's CPU, under its spans and between them (3af4e92a3, 2f9181e59).
            #[cfg(test)]
            mod threads_tests;
            // Tests that read what the engine registers as it loads (the languages, their `.rerail` libraries) run here.
            pub mod report {
                pub use rerail_boot::code_rule_dsl::logic::profile::report::*;
                #[cfg(test)]
                pub mod tests;
            }
            pub use rerail_logic::code_rule_dsl::logic::profile::watch;
            #[allow(unused_imports)]
            use std::collections::BTreeMap;
            #[allow(unused_imports)]
            use std::fmt::Write as _;
            #[allow(unused_imports)]
            use std::path::Path;
            #[allow(unused_imports)]
            use std::sync::atomic::{AtomicBool, Ordering};
            #[allow(unused_imports)]
            use std::sync::{LazyLock, Mutex};
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::profile::roofline::Unit;
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
        #[cfg_attr(test, path = "strings")]
        pub mod strings {
            pub use rerail_boot::code_rule_dsl::logic::strings::*;
            // `to_precision` is the ECMA crate's (`TO_PRECISION`), registered as the engine loads.
            #[cfg(test)]
            pub mod engine_tests;
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, HashMap, HashSet};
            #[allow(unused_imports)]
            use std::sync::{Arc, LazyLock, Mutex};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::text::{Domain, Lowered, Source, Spelling, lower};
            #[allow(unused_imports)]
            use rusqlite::Connection;
        }
    }
    #[cfg_attr(test, path = "sql_executor")]
    mod sql_executor {
        pub use rerail_boot::code_rule_dsl::sql_executor::*;
        // Only in builds that have them in rerail-sessions (its own `cfg`): its tests, `test-hooks`, `headless`.
        #[cfg(any(test, feature = "test-hooks", feature = "headless"))]
        pub mod verification {
            pub use rerail_boot::code_rule_dsl::sql_executor::verification::*;
            #[cfg(test)]
            #[path = "tests.rs"]
            pub mod tests;
            #[allow(unused_imports)]
            use super::*;
        }
        pub mod versions {
            pub use rerail_boot::code_rule_dsl::sql_executor::versions::*;
            #[cfg(test)]
                    pub mod operators;
            #[allow(unused_imports)]
            use super::super::facts::vtab::Modules;
            #[allow(unused_imports)]
            use super::*;
            #[allow(unused_imports)]
            use parking_lot::RwLock;
            #[allow(unused_imports)]
            use std::cell::RefCell;
            #[allow(unused_imports)]
            use tree::{Key, Link, Node};
            #[cfg(test)]
                    pub mod tests;
        }
        pub mod work_graph {
            pub use rerail_boot::code_rule_dsl::sql_executor::work_graph::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use super::*;
            #[allow(unused_imports)]
            use rerail_accel::kernel::{Kernel, Program, ProgramKernel, ProgramOutput};
            #[allow(unused_imports)]
            use rerail_accel::{Batch, Lane, Launch};
            #[allow(unused_imports)]
            use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
            #[allow(unused_imports)]
            use std::time::{Duration, Instant};
        }
        pub mod temporal {
            pub use rerail_boot::code_rule_dsl::sql_executor::temporal::*;
            pub mod cold_loop {
                pub use rerail_boot::code_rule_dsl::sql_executor::temporal::cold_loop::*;
                #[cfg(test)]
                            pub mod tests;
                #[allow(unused_imports)]
                use super::*;
            }
            pub mod batch {
                pub use rerail_boot::code_rule_dsl::sql_executor::temporal::batch::*;
                #[cfg(test)]
                            pub mod tests;
                #[allow(unused_imports)]
                use super::*;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::facts::vtab::Modules;
                #[allow(unused_imports)]
                use rusqlite::functions::{Aggregate, Context, FunctionFlags};
                #[allow(unused_imports)]
                use state::{Arena, Wave};
            }
            #[allow(unused_imports)]
            use super::*;
            #[cfg(test)]
                    pub mod replay_tests;
            #[allow(unused_imports)]
            use super::super::logic::sql::select::{Query, Relation as Rel};
            #[allow(unused_imports)]
            use super::maintenance::{Delta, Work};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::logic::native::aggregation::{Grouping, Reduction};
            #[allow(unused_imports)]
            use frame::Frame;
        }
        // Only in builds that have them in rerail-sessions (its own `cfg`): its tests, `test-hooks`, `headless`.
        #[cfg(any(test, feature = "test-hooks", feature = "headless"))]
        pub mod component_loop {
            pub use rerail_boot::code_rule_dsl::sql_executor::component_loop::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use super::*;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::facts::vtab::Modules;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::logic::analysis::stages::{Components, loops::Analysis};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::logic::sql::select::Query;
            #[allow(unused_imports)]
            use rusqlite::functions::FunctionFlags;
        }
        #[cfg(test)]
        pub mod tests;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::facts::core::{self, Built};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::facts::vtab::{Relation, Source, syntax};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::logic::sql::select;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::logic::{front, plan, rr, strings};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::waves::{owner::Database, schedule};
        #[allow(unused_imports)]
        use crate::content_store::ContentStore;
        #[allow(unused_imports)]
        use napi::{Error, Result};
        #[allow(unused_imports)]
        use napi_derive::napi;
        #[allow(unused_imports)]
        use rusqlite::OptionalExtension;
        #[allow(unused_imports, hidden_glob_reexports)]
        use rusqlite::types::{Value as SqlValue, ValueRef};
        #[allow(unused_imports, hidden_glob_reexports)]
        use serde_json::{Value, json};
        #[allow(unused_imports, hidden_glob_reexports)]
        use std::collections::{BTreeMap, BTreeSet, HashMap};
        #[allow(unused_imports)]
        use std::path::Path;
        #[allow(unused_imports, hidden_glob_reexports)]
        use std::sync::Arc;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::logic::native::lattices;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::logic::sql::program;
    }
    // Read by the prefilter's lowering; the compiled-plan cache is wired with the plans row.
    #[cfg_attr(test, path = "stats")]
    mod stats {
        pub use rerail_boot::code_rule_dsl::stats::*;
        #[cfg_attr(test, path = "ingest")]
        pub mod ingest {
            pub use rerail_boot::code_rule_dsl::stats::ingest::*;
            #[cfg(test)]
            pub mod tests;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::facts::core::KIND_STRIDE as LANG_STRIDE;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::search_index::kind_code;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::tape::TapeDoc;
            #[allow(unused_imports)]
            use rusqlite::{Connection, Result, params};
            #[allow(unused_imports)]
            use std::collections::BTreeMap;
        }
        #[cfg_attr(test, path = "inputs")]
        pub mod inputs {
            pub use rerail_boot::code_rule_dsl::stats::inputs::*;
            #[cfg(test)]
            pub mod tests;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::{AnchorPlan, Condition, Required, Requirement};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::facts::core::KIND_STRIDE as LANG_STRIDE;
            #[allow(unused_imports)]
            use rusqlite::{Connection, Result, limits::Limit, params_from_iter, types::Value};
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, BTreeSet, HashSet};
            #[allow(unused_imports)]
            use std::sync::Arc;
        }
        #[cfg_attr(test, path = "plans")]
        pub mod plans {
            pub use rerail_boot::code_rule_dsl::stats::plans::*;
            #[cfg(test)]
            pub mod tests;
            #[allow(unused_imports)]
            use rusqlite::{Connection, OptionalExtension, Result, params};
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, btree_map};
            #[allow(unused_imports)]
            use std::sync::Arc;
        }
    }
    #[cfg_attr(test, path = "sql_admission")]
    pub mod sql_admission {
        pub use rerail_boot::code_rule_dsl::sql_admission::*;
        #[cfg(test)]
        pub mod tests;
        #[allow(unused_imports)]
        use parking_lot::Mutex;
        #[allow(unused_imports)]
        use std::cell::RefCell;
        #[allow(unused_imports)]
        use std::sync::Arc;
        #[allow(unused_imports)]
        use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
    }
    #[cfg_attr(test, path = "residency")]
    mod residency {
        pub use rerail_boot::code_rule_dsl::residency::*;
        #[cfg_attr(test, path = "admission")]
        pub mod admission {
            pub use rerail_boot::code_rule_dsl::residency::admission::*;
            #[cfg_attr(test, path = "relational")]
            pub mod relational {
                pub use rerail_boot::code_rule_dsl::residency::admission::relational::*;
                pub mod syntax {
                    pub use rerail_boot::code_rule_dsl::residency::admission::relational::syntax::*;
                    #[cfg(test)]
                    #[path = "scoped_index_tests.rs"]
                    pub mod scoped_index_tests;
                    #[allow(unused_imports)]
                    use super::{Result, invalid, source, sql_error};
                    #[allow(unused_imports)]
                    use crate::code_rule_dsl::{ eval, language::GrammarDialect, lines::LineTable, model::FileInput, tape::TapeDoc, violations::Span, };
                    #[allow(unused_imports)]
                    use ast_grep_core::source::SgNode;
                    #[allow(unused_imports)]
                    use packed::{Columns, Fields, Postings};
                    #[allow(unused_imports)]
                    use rusqlite::{Connection, types::Value, vtab::Context};
                    #[allow(unused_imports)]
                    use std::{ cell::RefCell, collections::{BTreeMap, HashMap}, sync::{ Arc, atomic::{AtomicU64, AtomicUsize}, }, };
                }
                pub mod producer {
                    pub use rerail_boot::code_rule_dsl::residency::admission::relational::producer::*;
                    pub mod layout {
                        pub use rerail_boot::code_rule_dsl::residency::admission::relational::producer::layout::*;
                        #[cfg(test)]
                        #[path = "tests.rs"]
                        pub mod tests;
                        #[allow(unused_imports)]
                        use super::{Demand, Resident};
                        #[allow(unused_imports)]
                        use std::{collections::HashSet, sync::Arc};
                    }
                }
                pub mod pool {
                    pub use rerail_boot::code_rule_dsl::residency::admission::relational::pool::*;
                    #[cfg(test)]
                    #[path = "tests.rs"]
                    pub mod tests;
                    #[allow(unused_imports)]
                    use super::{Database, Result, configure, connection, sql_error};
                    #[allow(unused_imports)]
                    use std::{ cell::{Cell, RefCell}, ops::Deref, rc::Rc, sync::{Arc, atomic::AtomicUsize}, };
                }
                pub mod admission {
                    pub use rerail_boot::code_rule_dsl::residency::admission::relational::admission::*;
                    #[cfg(test)]
                                    pub mod tests;
                    #[allow(unused_imports)]
                    use super::{Inputs, Result, invalid};
                    #[allow(unused_imports)]
                    use crate::code_rule_dsl::{ eval, model::{ClauseKind, CompiledClause, CompiledCodeRule, Metrics, Snapshot}, };
                }
                #[cfg_attr(test, path = "batch")]
                pub mod batch {
                    pub use rerail_boot::code_rule_dsl::residency::admission::relational::batch::*;
                    #[cfg(test)]
                    pub mod tests;
                    #[allow(unused_imports)]
                    use crate::code_rule_dsl::residency::admission::relational::{Inputs, Job, Preparation, failure, producer, sql_error};
                    #[allow(unused_imports)]
                    use crate::code_rule_dsl::model::{CompiledCodeRule, Metrics};
                    #[allow(unused_imports)]
                    use crate::code_rule_dsl::{budget::Context, model::CompiledClause};
                    #[allow(unused_imports)]
                    use std::collections::VecDeque;
                }
                #[cfg_attr(test, path = "inputs")]
                pub mod inputs {
                    pub use rerail_boot::code_rule_dsl::residency::admission::relational::inputs::*;
                    #[cfg(test)]
                    pub mod tests;
                    #[allow(unused_imports)]
                    use crate::code_rule_dsl::residency::admission::relational::{FactQuery, insert};
                    #[allow(unused_imports)]
                    use crate::code_rule_dsl::{
                        eval,
                        model::{CompiledClause, CompiledCodeRule, Metrics, Snapshot},
                        violations::{Witness, anchored_span, projection::Row},
                    };
                    #[allow(unused_imports)]
                    use napi::Result;
                    #[allow(unused_imports)]
                    use rusqlite::{Connection, types::ValueRef};
                    #[allow(unused_imports)]
                    use std::{
                        cell::RefCell,
                        sync::{Arc, atomic::AtomicUsize},
                    };
                }
                #[allow(unused_imports)] // boot's mount: only boot's files read it
                pub use rerail_lang_python::code_rule_dsl::residency::admission::relational::python_ast;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::eval::{failure, violation_from_witness};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::json::{hex_digest, invalid};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::model::{CompiledClause, CompiledCodeRule, FactQuery, Snapshot};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::violations::{Captures, Witness};
                #[allow(unused_imports)]
                use napi::Result;
                #[allow(unused_imports)]
                use rusqlite::Connection;
                #[allow(unused_imports)]
                use rusqlite::functions::FunctionFlags;
                #[allow(unused_imports)]
                use rusqlite::hooks::Authorization;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::residency::admission::relational::database::Database;
                #[allow(unused_imports)]
                pub(in crate::code_rule_dsl) use crate::code_rule_dsl::residency::admission::relational::reachability::Reachability;
                #[allow(unused_imports)]
                use serde_json::{Value, json};
                #[allow(unused_imports)]
                use std::collections::{BTreeMap, BTreeSet, HashMap};
                #[allow(unused_imports)]
                use std::sync::{
                    Arc,
                    atomic::{AtomicUsize, Ordering},
                };
            }
        }
        // The slabs' tests store parsed documents: they run where the languages are registered.
        #[cfg(any(target_os = "macos", target_os = "linux", windows))]
        pub mod slabs {
            pub use rerail_boot::code_rule_dsl::residency::slabs::*;
            #[cfg(test)]
            pub mod tests;
        }
        #[allow(unused_imports)]
        pub(in crate::code_rule_dsl) use crate::code_rule_dsl::residency::admission::relational::ast_stage::install_ast_producer;
    }
    #[cfg(test)]
    mod test_support;
    // Typing's rounds run over T's facts in tests only until Phase 10 wires them.
    #[cfg_attr(test, path = "typing")]
    mod typing {
        pub use rerail_boot::code_rule_dsl::typing::*;
        // Tests that read what the engine registers as it loads (the languages, their `.rerail` libraries) run here.
        #[cfg(test)]
        pub mod phase_tests;
        #[cfg_attr(test, path = "file_local")]
        pub mod file_local {
            pub use rerail_boot::code_rule_dsl::typing::file_local::*;
            // A test that reads T's programs as text: it runs where the Rr language is registered.
            #[cfg(test)]
            pub mod engine_tests;
            #[allow(unused_imports)]
            use crate::content_store::{StoreError, StoreResult};
            #[allow(unused_imports)]
            pub(crate) use crate::code_rule_dsl::facts::nearest::{Binding, NONE, Trees};
            #[allow(unused_imports)]
            pub(crate) use crate::code_rule_dsl::logic::front::document_column;
            #[allow(unused_imports)]
            use std::collections::BTreeMap;
        }
        #[cfg(test)]
        pub mod oracle_tests;
    }
}
