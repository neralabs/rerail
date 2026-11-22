//! The engine's tests of the SQL executor: its sessions, maintenance, retained traces and versions, the SQL admission and the residency (docs/final-shape/proposals/crate-split/incremental-plan.md, step 7i).
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
}
