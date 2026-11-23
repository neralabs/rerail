//! The engine's tests of the corpus runs: the planner's corpus, its sessions, kept stores and warm runs, and its coverage (docs/final-shape/proposals/crate-split/incremental-plan.md, step 7i).
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
    pub use rerail_boot::code_rule_dsl::rule_api::CodeRuleDslEditor;
    #[path = "logic"]
    pub(crate) mod logic {
        pub use rerail_boot::code_rule_dsl::logic::*;
        #[cfg_attr(test, path = "plan")]
        pub mod plan {
            pub use rerail_boot::code_rule_dsl::logic::plan::*;
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
                // The matchers' tests: they compile rules through the engine (`rule_api`).
                pub mod matchers {
                    pub use rerail_boot::code_rule_dsl::logic::plan::corpus::matchers::*;
                    #[cfg(test)]
                    pub mod tests;
                    #[allow(unused_imports)]
                    use crate::code_rule_dsl::logic::program_image::{Lit, TermRef, Version};
                    #[allow(unused_imports)]
                    use std::collections::{BTreeSet, HashMap};
                    #[allow(unused_imports)]
                    use crate::code_rule_dsl::facts::grammar_shape::{field_kinds, leaf_kind};
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
            pub mod coverage;
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
    }
    #[cfg(test)]
    #[path = "../../../../src/code_rule_dsl/test_support.rs"]
    mod test_support;
}
