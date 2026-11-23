//! The engine's tests of scopes: the scope tables and their usage, the module graph, identifiers and lines (docs/final-shape/proposals/crate-split/incremental-plan.md, step 7i).
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
    #[cfg(test)]
    #[cfg_attr(test, path = "module_graph")]
    mod module_graph {
        pub use rerail_boot::code_rule_dsl::module_graph::*;
        #[cfg(test)]
        pub mod tests;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::tree::Tree;
        #[allow(unused_imports)]
        use ast_grep_core::{Doc, Node};
        #[allow(unused_imports)]
        use ast_grep_language::LanguageExt;
        #[allow(unused_imports)]
        use indexmap::IndexSet;
        #[allow(unused_imports)]
        use parking_lot::Mutex;
        #[allow(unused_imports)]
        use regex::Regex;
        #[allow(unused_imports)]
        use serde_json::Value;
        #[allow(unused_imports)]
        use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
        #[allow(unused_imports)]
        use std::path::{Path, PathBuf};
        #[allow(unused_imports)]
        use std::sync::{Arc, LazyLock};
        #[allow(unused_imports)]
        use std::time::{Duration, SystemTime};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::adapter::package_manifests;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::{dialect_by_label, label_of_path};
    }
    pub use rerail_boot::code_rule_dsl::rule_api::CodeRuleDslEditor;
    // The relational pools' hook slots, compiled wherever the pools are (the hub's relational stage is in the library).
    pub mod identifier {
        pub use rerail_boot::code_rule_dsl::identifier::*;
        #[cfg(test)]
        pub mod tests;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::{Family, GrammarDialect};
        #[allow(unused_imports)]
        use ast_grep_core::{Doc, Node};
        #[allow(unused_imports)]
        use std::borrow::Cow;
        #[allow(unused_imports)]
        use unicode_normalization::{IsNormalized, UnicodeNormalization, is_nfkc_quick};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::adapter::pack as language_pack;
    }
    #[cfg_attr(test, path = "lines")]
    mod lines {
        pub use rerail_boot::code_rule_dsl::lines::*;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::Family;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::GrammarDialect;
        #[allow(unused_imports)]
        use serde::{Deserialize, Serialize};
        #[cfg(test)]
            pub mod eslint;
    }
    // P's paths for P's relational stage, under test (open item O18): P's workspace code reads it here.
    #[cfg_attr(test, path = "scope")]
    mod scope {
        pub use rerail_boot::code_rule_dsl::scope::*;
        pub use rerail_lang_ecma::code_rule_dsl::scope::ecma;
        pub mod php {
            pub use rerail_boot::code_rule_dsl::scope::php::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::tree::Tree;
            #[allow(unused_imports)]
            use super::php_names::{self, PhpNames, Resolved};
            #[allow(unused_imports)]
            use super::{Binding, BuildError, Builder, Form, NodeRef, ScopeKind, Site, VALUE};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::value::{Context as Decoding, php as php_value};
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use foldhash::{HashMap, HashMapExt};
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, BTreeSet};
            #[allow(unused_imports)]
            use std::sync::Arc;
        }
        pub mod php_names {
            pub use rerail_boot::code_rule_dsl::scope::php_names::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use std::sync::Arc;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::tree::Tree;
            #[allow(unused_imports)]
            use super::{BuildError, NodeRef};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::value::php_compare::{ImportKind, import_kind};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::value::{Context, Value, php as php_value};
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use serde::Deserialize;
            #[allow(unused_imports)]
            use std::collections::{HashMap, HashSet};
            #[allow(unused_imports)]
            use std::ops::Range;
            #[allow(unused_imports)]
            use std::sync::LazyLock;
        }
        pub use rerail_lang_python::code_rule_dsl::scope::python;
        pub mod rust {
            pub use rerail_boot::code_rule_dsl::scope::rust::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use super::{ Binding, BuildError, Builder, Form, Import, NodeRef, Reference, ScopeKind, ScopeTable, Site, TYPE, VALUE, };
            #[allow(unused_imports)]
            use crate::code_rule_dsl::tree::Tree;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::calls::rust::{LIBRARY_ROOTS, crate_module, std_macro};
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
        }
        pub mod sql {
            pub use rerail_boot::code_rule_dsl::scope::sql::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use super::{Binding as TableBinding, BuildError, Builder, Form, ScopeKind, Site, VALUE};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::value::NodeRef;
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use foldhash::{HashMap, HashMapExt};
            #[allow(unused_imports)]
            use std::ops::Range;
        }
        #[cfg_attr(test, path = "usage")]
        pub mod usage {
            pub use rerail_boot::code_rule_dsl::scope::usage::*;
            #[cfg(test)]
            pub use rerail_lang_python::code_rule_dsl::scope::usage::lang_python;
            #[cfg(test)]
            pub use rerail_lang_rust::code_rule_dsl::scope::usage::lang_rust;
            pub mod rust {
                pub use rerail_boot::code_rule_dsl::scope::usage::rust::*;
                #[cfg(test)]
                #[path = "tests.rs"]
                pub mod tests;
                #[allow(unused_imports)]
                use super::super::rust::{MACRO, resolve_index};
                #[allow(unused_imports)]
                use super::super::{Access, Import, NodeRef, Reference, ScopeKind, ScopeTable, TYPE, VALUE};
                #[allow(unused_imports)]
                use super::{Facts, Steps};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::scope::BuildError;
                #[allow(unused_imports)]
                #[cfg(any(test, feature = "test-hooks"))]
                use crate::code_rule_dsl::walks::{Walk, record_walk};
                #[allow(unused_imports)]
                use ast_grep_core::{Doc, Node};
                #[allow(unused_imports)]
                use foldhash::HashMap;
            }
            #[allow(unused_imports)]
            use crate::code_rule_dsl::scope::{
                Binding, BuildError, DEADLINE_INTERVAL, Form, ImportKind, NodeRef, ScopeKind, ScopeTable,
            };
            #[allow(unused_imports)]
            use crate::code_rule_dsl::tree::Tree;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::identifier::python_identifier;
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use foldhash::{HashMap, HashMapExt};
            #[allow(unused_imports)]
            use std::borrow::Cow;
            #[allow(unused_imports)]
            use std::collections::BTreeSet;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::pack as language_pack;
            #[cfg(test)]
                    pub mod tests;
        }
        #[allow(unused_imports)]
        pub(crate) use crate::code_rule_dsl::scope::lookup::Lookup;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::identifier::{nfkc};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::{Family, GrammarDialect};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::module_path::ModulePath;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::value::{Followed, NodeRef};
        #[allow(unused_imports)]
        use ast_grep_core::{Doc, Node};
        #[allow(unused_imports)]
        use ast_grep_language::LanguageExt;
        #[allow(unused_imports)]
        use foldhash::{HashMap, HashMapExt};
        #[allow(unused_imports)]
        use std::borrow::Cow;
        #[allow(unused_imports)]
        use std::collections::BTreeSet;
        #[allow(unused_imports)]
        use std::ops::Range;
        #[allow(unused_imports)]
        use std::sync::{Arc, OnceLock};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::adapter::pack as language_pack;
        #[cfg(test)]
        #[path = "tests.rs"]
        pub mod tests;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::identifier::{name_in};
    }
    #[cfg(test)]
    #[path = "../../../../src/code_rule_dsl/test_support.rs"]
    mod test_support;
}

// The engine's allocator (`rerail_prof::heap::Sampled`, as `rerail-engine`'s `lib.rs` declares it): without it the
// test binary ran on the system allocator, which no build of the engine runs on.
#[cfg(not(any(windows, target_family = "wasm", feature = "system-allocator")))]
#[global_allocator]
static GLOBAL: rerail_prof::heap::Sampled = rerail_prof::heap::Sampled;
