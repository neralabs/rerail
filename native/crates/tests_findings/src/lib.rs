//! The engine's tests of findings: the workspace's storage, results, placement and cross-file findings, the async reference and the violations (docs/final-shape/proposals/crate-split/incremental-plan.md, step 7i).
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
    // Wired to the findings layer in a later slice; until then only tests call it.
    #[cfg_attr(test, path = "findings")]
    mod findings {
        pub use rerail_boot::code_rule_dsl::findings::*;
        #[cfg(test)]
        #[path = "tests"]
        pub mod async_reference {
            pub use rerail_boot::code_rule_dsl::findings::async_reference::*;
            // Its timings are read by the p-anchor-corpus tests alone.
            #[cfg(test)]
            #[cfg_attr(not(feature = "p-anchor-corpus"), allow(dead_code))]
            pub mod native_oracle;
            #[cfg(test)]
            pub mod s_native_oracle;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::findings::{
                contract::{Coverage, FindingKey, Findings},
                shape::generated_shape,
            };
            #[allow(unused_imports)]
            use crate::code_rule_dsl::facts::producers::python::{call_batch, python_facts::{self, Qualification}, python_structure::{ContextKind, NodeRef}};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::facts::vtab::{self, Modules, Relation, Source};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::{
                language::GrammarDialect,
                module_path::ModulePath,
                scope::{
                    Reading, ScopeTable,
                    options::{DEFAULT_PYTHON_MINOR, PythonOptions},
                },
                tape::TapeDoc,
            };
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use rusqlite::{Connection, StatementStatus};
            #[allow(unused_imports)]
            use std::{
                collections::{BTreeMap, BTreeSet},
                time::Instant,
            };
            #[cfg(test)]
                    pub mod rr_comparison;
            #[cfg(test)]
                    pub mod tests;
            #[cfg(test)]
            pub mod compiler_admission;
        }
    }
    #[cfg(test)]
    mod test_support;
    pub mod violations {
        pub use rerail_boot::code_rule_dsl::violations::*;
        #[cfg(test)]
            pub mod each_limit_tests;
        #[cfg(test)]
            pub mod tests;
        #[allow(unused_imports)]
        use super::json::invalid;
        #[allow(unused_imports)]
        use super::matcher::{ AstMatcher, Capture, CaptureAnalysis, GrammarMatcher, UtilTable, UtilitySummary, binding_range, collect_variables, };
        #[allow(unused_imports)]
        use super::model::{ Anchor, ClauseKind, FIX_TEXT_PREFIX, FixPlan, Query, RegexSource, Template, ViolationSpec, };
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use crate::code_rule_dsl::walks::{Walk, record_walk};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::{Family, GrammarDialect};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::origin::InFile;
        #[allow(unused_imports)]
        use ast_grep_core::matcher::NodeMatch;
        #[allow(unused_imports)]
        use ast_grep_core::meta_var::{MetaVarEnv, MetaVariable};
        #[allow(unused_imports)]
        use ast_grep_core::replacer::{Content, Replacer, TemplateFix};
        #[allow(unused_imports)]
        use ast_grep_core::{Doc, Node};
        #[allow(unused_imports)]
        use napi::Result;
        #[allow(unused_imports)]
        use regex::Regex;
        #[allow(unused_imports)]
        use serde::{Deserialize, Serialize};
        #[allow(unused_imports)]
        use serde_json::{Value, json};
        #[allow(unused_imports)]
        use std::borrow::Cow;
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use std::cell::Cell;
        #[allow(unused_imports)]
        use std::collections::{BTreeMap, BTreeSet};
        #[allow(unused_imports)]
        use std::ops::Range;
        #[allow(unused_imports)]
        use std::sync::Arc;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::adapter::pack as language_pack;
    }
    #[cfg_attr(test, path = "workspace")]
    pub(crate) mod workspace {
        pub use rerail_boot::code_rule_dsl::workspace::*;
        pub mod results {
            pub use rerail_boot::code_rule_dsl::workspace::results::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use super::crossfile::CrossFileScheduler;
            #[allow(unused_imports)]
            use super::ingest::{candidate_dispatch, make_file};
            #[allow(unused_imports)]
            use super::persisted::{candidate_catalog, restore_complete_results};
            #[allow(unused_imports)]
            use super::{CandidateSets, ClauseId, FileState, WorkspaceState};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::eval::{ coverage, evaluation_json, failure, file_language, result_order, violation_from_witness, };
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::{invalid, json_error};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::model::{ClauseKind, CompiledCodeRule, Evaluation, Metrics, Query};
            #[allow(unused_imports)]
            #[cfg(feature = "code-rule-profile")]
            use crate::code_rule_dsl::profile;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::soundness::{SYNTAX_ERRORS, UNREAD};
            #[allow(unused_imports)]
            use napi::Result;
            #[allow(unused_imports)]
            use rayon::prelude::*;
            #[allow(unused_imports)]
            use serde_json::{Value, json};
            #[allow(unused_imports)]
            #[cfg(any(test, feature = "test-hooks"))]
            use std::cell::Cell;
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, BTreeSet};
            #[allow(unused_imports)]
            use std::hash::{DefaultHasher, Hash, Hasher};
            #[allow(unused_imports)]
            use std::sync::Arc;
            #[allow(unused_imports)]
            use std::time::Duration;
        }
        #[cfg(test)]
        pub mod crossfile_tests;
        #[cfg(test)]
        pub mod placement_tests;
        #[cfg(test)]
        pub mod storage_tests;
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
