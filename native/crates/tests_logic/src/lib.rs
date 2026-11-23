//! The engine's tests of logic: the planner, the native set kernels, the profile, typing, the waves and the statistics (docs/final-shape/proposals/crate-split/incremental-plan.md, step 7i).
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

#[path = "code_rule_dsl"]
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
        pub mod liveness {
            pub use rerail_boot::code_rule_dsl::logic::liveness::*;
            #[cfg(test)]
            pub mod tests;
        }
        // The cost statistics' read of each lane and form (3a42aefcf).
        #[cfg(test)]
        mod costs_tests;
        #[cfg_attr(test, path = "native")]
        pub mod native {
            pub use rerail_boot::code_rule_dsl::logic::native::*;
            #[cfg_attr(test, path = "reach")]
            pub mod reach {
                pub use rerail_boot::code_rule_dsl::logic::native::reach::*;
                #[cfg(test)]
                pub mod tests;
            }
            // rerail-setfix's directory: the lab's tests are its files (a path from here, which this crate's tree has).
            #[cfg_attr(test, path = "../../../../../setfix/src/code_rule_dsl/logic/native/setfix")]
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
            #[cfg(test)]
            pub mod tests;
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
    #[cfg(test)]
    #[path = "../../../../src/code_rule_dsl/test_support.rs"]
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
