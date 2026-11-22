//! The engine's tests of rule compiling: the compiler, scopes, values, calls, literals and the evaluator (docs/final-shape/proposals/crate-split/incremental-plan.md, step 2).
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
    #[cfg_attr(test, path = "blade")]
    mod blade {
        pub use rerail_boot::code_rule_dsl::blade::*;
        pub mod forte {
            pub use rerail_boot::code_rule_dsl::blade::forte::*;
            pub mod tree {
                pub use rerail_boot::code_rule_dsl::blade::forte::tree::*;
                #[cfg(test)]
                            pub mod tests;
                #[allow(unused_imports)]
                use super::ast::{Ast, ElementClass};
                #[allow(unused_imports)]
                use super::directives::Directives;
                #[allow(unused_imports)]
                use super::node::{FlatNode, NodeKind};
                #[allow(unused_imports)]
                use super::token::{Token, TokenType};
                #[allow(unused_imports)]
                use super::{Budget, DepthLimits, ParseError, Parsed};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::literal::{ID_FLAG, NEXT_TREE, TREE_MASK};
                #[allow(unused_imports)]
                use ast_grep_core::Position;
                #[allow(unused_imports)]
                use ast_grep_core::language::Language;
                #[allow(unused_imports)]
                use ast_grep_core::matcher::{Pattern, PatternBuilder, PatternError};
                #[allow(unused_imports)]
                use ast_grep_core::source::{Content, Doc, Edit, SgNode};
                #[allow(unused_imports)]
                use std::borrow::Cow;
                #[allow(unused_imports)]
                use std::ops::Range;
                #[allow(unused_imports)]
                use std::sync::Arc;
                #[allow(unused_imports)]
                use std::sync::atomic::Ordering;
            }
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use directives::Directives;
            #[allow(unused_imports)]
            use node::FlatNode;
            #[allow(unused_imports)]
            use token::{LexerError, Token, TokenType};
        }
        #[cfg(test)]
        pub mod tests;
        #[cfg(test)]
            pub mod oracle_tests;
    }
    #[cfg_attr(test, path = "calls")]
    mod calls {
        pub use rerail_boot::code_rule_dsl::calls::*;
        pub use rerail_lang_php::code_rule_dsl::calls::php;
        #[cfg(test)]
        pub use rerail_lang_python::code_rule_dsl::calls::python;
        pub mod rust {
            pub use rerail_boot::code_rule_dsl::calls::rust::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::tree::Tree;
            #[allow(unused_imports)]
            use super::names::Name;
            #[allow(unused_imports)]
            use super::summary::{self, Def, OPEN, Summary};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::Family;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::scope::rust::{UseHead, UsePath, glob_visible, resolve_index, use_path};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::scope::{Binding, Form, Reference, ScopeTable};
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use std::cell::RefCell;
            #[allow(unused_imports)]
            use std::collections::{HashMap, HashSet};
            #[allow(unused_imports)]
            use std::rc::Rc;
            #[allow(unused_imports)]
            use std::sync::Arc;
        }
        #[allow(unused_imports)]
        use crate::code_rule_dsl::json::{invalid, object};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::is_stub_file;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::{Family, GrammarDialect};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::lines::Span;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::module_path::ModulePath;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::scope::{Reading, ScopeTable};
        #[allow(unused_imports)]
        use crate::content_store::HashKey;
        #[allow(unused_imports)]
        use ast_grep_core::{Doc, Node};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::calls::closure::{DefRef, Graph, Hop};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::calls::names::Name;
        #[allow(unused_imports)]
        use napi::Result;
        #[allow(unused_imports)]
        use serde_json::{Map, Value};
        #[allow(unused_imports)]
        use sha2::{Digest, Sha256};
        #[allow(unused_imports)]
        use std::cell::RefCell;
        #[allow(unused_imports)]
        use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
        #[allow(unused_imports)]
        use std::sync::{Arc, OnceLock};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::calls::summary::{ASYNC, CLASS, Callee, Summary};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::adapter::pack as language_pack;
        #[cfg(test)]
            pub mod tests;
        #[cfg(test)]
        #[path = "php_tests.rs"]
        pub mod php_tests;
        #[cfg(test)]
        #[path = "python_tests.rs"]
        pub mod python_tests;
    }
    #[cfg_attr(test, path = "compile")]
    mod compile {
        pub use rerail_boot::code_rule_dsl::compile::*;
        pub mod value {
            pub use rerail_boot::code_rule_dsl::compile::value::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::{bounded_text, invalid, object, reject_unknown};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::{ ValueFeature, gate, };
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::{Family, GrammarDialect};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::{AstMatcher, Bound, Reading, ValueTest};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::{Semantic, SemanticText};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::model::Limits;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::scope::options::{LanguageOptions, PythonOptions};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::value::Value;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::value::text_regex::TextRegex;
            #[allow(unused_imports)]
            use napi::Result;
            #[allow(unused_imports)]
            use serde_json::Value as Json;
            #[allow(unused_imports)]
            use std::sync::Arc;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::pack as language_pack;
        }
        pub mod arguments {
            pub use rerail_boot::code_rule_dsl::compile::arguments::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use super::resolve::is_identifier;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::{invalid, object, reject_unknown};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::{ ArgumentFeature, gate, };
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::{Family, GrammarDialect, kind_ids};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::Semantic;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::{Arguments, AstMatcher, SlotKind};
            #[allow(unused_imports)]
            use napi::Result;
            #[allow(unused_imports)]
            use serde_json::Value;
            #[allow(unused_imports)]
            use std::collections::BTreeSet;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::pack as language_pack;
        }
        pub mod utils {
            pub use rerail_boot::code_rule_dsl::compile::utils::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use super::documents::{DocumentScopes, Side};
            #[allow(unused_imports)]
            use super::order::AllOrderConflict;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::{invalid, object, reject_unknown};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::{ CallArgs, CaptureAnalysis, MAX_UTIL_INSTANCES, Transform, UtilTable, };
            #[allow(unused_imports)]
            use crate::code_rule_dsl::scope::options::{LanguageOptions, PythonOptions};
            #[allow(unused_imports)]
            use napi::Result;
            #[allow(unused_imports)]
            use serde_json::Value;
            #[allow(unused_imports)]
            use std::cell::RefCell;
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, BTreeSet};
            #[allow(unused_imports)]
            use std::sync::{Arc, Weak};
        }
        pub mod shared {
            pub use rerail_boot::code_rule_dsl::compile::shared::*;
            #[cfg(all(test, not(windows)))]
            pub mod tests;
        }
        pub mod order {
            pub use rerail_boot::code_rule_dsl::compile::order::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::invalid;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::{AstMatcher, CaptureAnalysis};
            #[allow(unused_imports)]
            use napi::Result;
            #[allow(unused_imports)]
            use std::collections::BTreeSet;
        }
        pub mod layout {
            pub use rerail_boot::code_rule_dsl::compile::layout::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use super::parse::capture_name;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::{invalid, object, reject_unknown};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::Semantic;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::{AstMatcher, Gap, Layout, LineContext, LineCount, Neighbour};
            #[allow(unused_imports)]
            use napi::Result;
            #[allow(unused_imports)]
            use serde_json::Value;
        }
        pub mod evidence {
            pub use rerail_boot::code_rule_dsl::compile::evidence::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use super::define::Defines;
            #[allow(unused_imports)]
            use super::{compile_glob_set, compile_query, string_array};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::{invalid, non_empty_str, object, reject_unknown};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::model::{ AstRelation, Capabilities, CodeRuleScope, FactQuery, NameClosure, Projection, ProjectionSource, Query, };
            #[allow(unused_imports)]
            use napi::Result;
            #[allow(unused_imports)]
            use serde_json::Value;
            #[allow(unused_imports)]
            use std::collections::BTreeSet;
            #[allow(unused_imports)]
            use std::sync::Arc;
        }
        pub mod dialect {
            pub use rerail_boot::code_rule_dsl::compile::dialect::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::invalid;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::GrammarDialect;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::{self, CodeRuleLanguage};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::AstMatcher;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::model::Capabilities;
            #[allow(unused_imports)]
            use napi::Result;
            #[allow(unused_imports)]
            use serde_json::Value;
        }
        pub mod define {
            pub use rerail_boot::code_rule_dsl::compile::define::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use super::ast_rule::{compile_ast_rule, validate_ast_rule};
            #[allow(unused_imports)]
            use super::config::Options;
            #[allow(unused_imports)]
            use super::dialect::language_capability;
            #[allow(unused_imports)]
            use super::documents::{DocumentScopes, Side};
            #[allow(unused_imports)]
            use super::refuse_unavailable_semantics;
            #[allow(unused_imports)]
            use super::shared::SharedTables;
            #[allow(unused_imports)]
            use super::utils::{UtilDef, UtilIds, UtilScope, reject_util_cycles, util_map};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::{invalid, object, reject_unknown, required_str};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::{CodeRuleLanguage, GrammarDialect};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::{AstMatcher, UtilTable};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::model::Capabilities;
            #[allow(unused_imports)]
            use napi::Result;
            #[allow(unused_imports)]
            use serde_json::Value;
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, BTreeSet};
            #[allow(unused_imports)]
            use std::sync::Arc;
        }
        pub mod config {
            pub use rerail_boot::code_rule_dsl::compile::config::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::{invalid, object, reject_unknown};
            #[allow(unused_imports)]
            use napi::Result;
            #[allow(unused_imports)]
            use serde_json::{Map, Number, Value};
            #[allow(unused_imports)]
            use std::collections::BTreeMap;
        }
        pub mod compare {
            pub use rerail_boot::code_rule_dsl::compile::compare::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use super::ast_rule::AST_RELATIONS;
            #[allow(unused_imports)]
            use super::order::AllOrderConflict;
            #[allow(unused_imports)]
            use super::resolve::{BINDING_KEYS, OCCURRENCE_KEY};
            #[allow(unused_imports)]
            use super::utils::UtilDef;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::{invalid, non_empty_str, object, reject_unknown};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::{ Collation, CompareOp, CompareText, Comparison, CountBinding, UtilitySummary, };
            #[allow(unused_imports)]
            #[cfg(any(test, feature = "test-hooks"))]
            use crate::code_rule_dsl::walks::{Walk, record_walk};
            #[allow(unused_imports)]
            use napi::Result;
            #[allow(unused_imports)]
            use serde_json::Value;
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, BTreeSet};
        }
        pub mod ast_rule {
            pub use rerail_boot::code_rule_dsl::compile::ast_rule::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use super::compare::{count_bounds, parse_comparison};
            #[allow(unused_imports)]
            use super::dialect::grammar_gap;
            #[allow(unused_imports)]
            use super::documents::Side;
            #[allow(unused_imports)]
            use super::nth_child::{compile_nth_child, validate_nth_child};
            #[allow(unused_imports)]
            use super::order::{all_order_conflicts, order_by_captures};
            #[allow(unused_imports)]
            use super::utils::{UtilIds, UtilScope, is_metavariable_name};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::{ bounded_text, invalid, non_empty_str, object, reject_unknown, required_str, };
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::{GrammarDialect, PatternSource, rule_pattern};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::{ AstMatcher, CallArgs, CompareText, Count, EmbeddedTest, FieldRelation, HostTest, Reading, Related, Semantic, StopBy, UtilCall, UtilRef, parse_exported, };
            #[allow(unused_imports)]
            use crate::code_rule_dsl::model::Limits;
            #[allow(unused_imports)]
            use ast_grep_core::Language;
            #[allow(unused_imports)]
            use ast_grep_core::matcher::{KindMatcher, Pattern, PatternNode};
            #[allow(unused_imports)]
            use napi::Result;
            #[allow(unused_imports)]
            use regex::Regex;
            #[allow(unused_imports)]
            use serde_json::Value;
            #[allow(unused_imports)]
            use std::collections::BTreeSet;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::pack as language_pack;
        }
        #[cfg_attr(test, path = "binds")]
        pub mod binds {
            pub use rerail_boot::code_rule_dsl::compile::binds::*;
            #[cfg(test)]
            pub mod eslint;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::compile::ast_rule::{compile_ast_rule};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::compile::utils::{UtilScope};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::{invalid, object, reject_unknown};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::{ScopeFeature};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::kind_ids;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::{Family, GrammarDialect};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::Semantic;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::{AstMatcher, BindingCount, BindingFilter, Binds, Redefines};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::model::Limits;
            #[allow(unused_imports)]
            use napi::Result;
            #[allow(unused_imports)]
            use serde_json::{Map, Value};
            #[allow(unused_imports)]
            use std::collections::BTreeSet;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::pack as language_pack;
            #[cfg(test)]
                    pub mod ruff;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use super::ast_rule::{validate_ast_rule};
            #[allow(unused_imports)]
            use super::resolve::{BINDING_KEYS, OCCURRENCE_KEY};
            #[allow(unused_imports)]
            use super::utils::{UtilIds};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::{gate};
        }
        #[cfg_attr(test, path = "format_args")]
        pub mod format_args {
            pub use rerail_boot::code_rule_dsl::compile::format_args::*;
            #[cfg(test)]
            pub mod php;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::compile::ast_rule::{compile_ast_rule};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::compile::utils::{UtilScope};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::{invalid, non_empty_str, object, reject_unknown};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::{FormatArgFeature};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::{Family, GrammarDialect};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::Semantic;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::{
                AstMatcher, Capture, FormatArgs, Reading, Style, parse_exported,
            };
            #[allow(unused_imports)]
            use crate::code_rule_dsl::model::Limits;
            #[allow(unused_imports)]
            use napi::Result;
            #[allow(unused_imports)]
            use serde_json::Value;
            #[allow(unused_imports)]
            use std::collections::BTreeSet;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::pack as language_pack;
            #[cfg(test)]
                    pub mod ruff;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use super::ast_rule::{validate_ast_rule};
            #[allow(unused_imports)]
            use super::parse::{capture_name, compile_sub_rule, validate_sub_rule};
            #[allow(unused_imports)]
            use super::utils::{UtilIds};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::{gate};
        }
        #[allow(unused_imports)] // boot's mount: only boot's files read it
        pub use rerail_lang_ecma::code_rule_dsl::compile::import_target;
        #[cfg_attr(test, path = "parse")]
        pub mod parse {
            pub use rerail_boot::code_rule_dsl::compile::parse::*;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::compile::ast_rule::{compile_ast_rule};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::compile::utils::{UtilScope};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::{bounded_text, invalid, non_empty_str, object};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::{SyntaxFeature};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::kind_ids;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::{Family, GrammarDialect};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::literal::Grammar;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::Semantic;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::{
                AstMatcher, CaptureAnalysis, CompareText, Count, HostTest, ParseTest, Reading, Related,
                SemanticText, StopBy, Transform, parse_exported, table_reading,
            };
            #[allow(unused_imports)]
            use crate::code_rule_dsl::model::Limits;
            #[allow(unused_imports)]
            use ast_grep_core::matcher::KindMatcher;
            #[allow(unused_imports)]
            use napi::Result;
            #[allow(unused_imports)]
            use regex::Regex;
            #[allow(unused_imports)]
            use serde_json::{Map, Value};
            #[allow(unused_imports)]
            use std::collections::BTreeSet;
            #[allow(unused_imports)]
            use std::sync::Arc;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::pack as language_pack;
            #[cfg(test)]
                    pub mod ruff;
            #[cfg(test)]
                    pub mod annotation_tests;
            #[cfg(test)]
                    pub mod host_tests;
            #[cfg(test)]
                    pub mod m12b_tests;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use super::ast_rule::{AST_RELATIONS, StopSpec, relation_rule, validate_ast_rule};
            #[allow(unused_imports)]
            use super::compare::{count_bounds, parse_comparison};
            #[allow(unused_imports)]
            use super::order::order_by_captures;
            #[allow(unused_imports)]
            use super::resolve::{compile_resolves, validate_resolves};
            #[allow(unused_imports)]
            use super::utils::{UtilIds};
            #[allow(unused_imports)]
            use super::value::{compile_value_with, validate_value};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::{gate};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::soundness::sound_under_errors;
        }
        #[cfg_attr(test, path = "resolve")]
        pub mod resolve {
            pub use rerail_boot::code_rule_dsl::compile::resolve::*;
            #[allow(unused_imports)] // boot's mount: only boot's files read it
            pub use rerail_lang_php::code_rule_dsl::compile::resolve::php;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::compile::utils::UtilScope;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::{invalid, object, reject_unknown};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::{Refusal, ScopeFeature};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::kind_ids;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::{Family, GrammarDialect, language_not_supported};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::Semantic;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::{AstMatcher, BindingTest, Resolves, Unresolved};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::scope::options::{LanguageOptions, PythonOptions};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::scope::Lookup;
            #[allow(unused_imports)]
            use napi::Result;
            #[allow(unused_imports)]
            use serde_json::{Map, Value};
            #[allow(unused_imports)]
            use std::collections::BTreeSet;
            #[allow(unused_imports)]
            use std::sync::Arc;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::pack as language_pack;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use super::binds::{FILTER_KEYS, compile_filter, validate_filter};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::{gate};
        }
        #[allow(unused_imports)]
        use crate::code_rule_dsl::json::{
            bounded_text, invalid, json_error, non_empty_str, object, reject_unknown, required_str,
        };
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::adapter::{Refusal};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::{CodeRuleLanguage, Semantic, Support};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::matcher::{AnchorPlan, AstMatcher, Capture, Constrained, GrammarMatcher, Reading, RequiredText, UtilOrigin, UtilTable, collect_variables, reads_import_targets};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::model::{
            Anchor, Capabilities, ClauseKind, CodeRuleScope, CompiledClause, Limits, Query, RegexLines,
            RegexOptions, RegexSource, RuleMode, ViolationSpec,
        };
        #[allow(unused_imports)]
        use crate::code_rule_dsl::violations;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::{GrammarDialect, PatternSource, rule_pattern};
        #[allow(unused_imports)]
        pub(super) use crate::code_rule_dsl::compile::ast_rule::compile_ast_rule;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::compile::documents::Side;
        #[allow(unused_imports)]
        use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
        #[allow(unused_imports)]
        use napi::Result;
        #[allow(unused_imports)]
        use regex::Regex;
        #[allow(unused_imports)]
        use serde::de::{self, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
        #[allow(unused_imports)]
        use serde_json::{Map, Value, json};
        #[allow(unused_imports)]
        use std::collections::{BTreeMap, BTreeSet};
        #[allow(unused_imports)]
        use std::sync::Arc;
        #[allow(unused_imports)]
        pub(super) use crate::code_rule_dsl::compile::utils::UtilScope;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::adapter::pack as language_pack;
        #[cfg(test)]
            pub mod tests;
        #[allow(unused_imports)]
        use super::language::adapter::{gate};
        #[allow(unused_imports)]
        use super::matcher::{reads_values};
        #[allow(unused_imports)]
        use super::soundness::sound_under_errors;
        #[allow(unused_imports)]
        use ast_rule::validate_ast_rule;
        #[allow(unused_imports)]
        use compare::{binding_site_index, bound_elsewhere, compare_sites, frame_scope};
        #[allow(unused_imports)]
        use define::{CLAUSE_KINDS, Defines, collect_defines, is_define};
        #[allow(unused_imports)]
        use dialect::{embedded_dialects, language_capability};
        #[allow(unused_imports)]
        use shared::SharedTables;
        #[allow(unused_imports)]
        use utils::{UtilDef, UtilIds, reachable, reject_util_cycles, rule_map, util_map};
        #[cfg(test)]
            pub mod binding_tests;
        #[cfg(test)]
            pub mod examples_tests;
        #[cfg(test)]
            pub mod params_tests;
    }
    #[cfg_attr(test, path = "embed")]
    mod embed {
        pub use rerail_boot::code_rule_dsl::embed::*;
        #[allow(unused_imports)] // boot's mount: only boot's files read it
        pub use rerail_lang_php::code_rule_dsl::embed::compiled_view;
        #[allow(unused_imports)] // boot's mount: only boot's files read it
        pub use rerail_lang_ecma::code_rule_dsl::embed::script_block;
        #[cfg_attr(not(test), allow(unused_imports))]
        pub use rerail_lang_python::code_rule_dsl::embed::call_argument;
        #[allow(unused_imports)]
        #[cfg(test)]
        pub(crate) use crate::code_rule_dsl::embed::test_script::TestScript;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::GrammarDialect;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::embed::segments::SegmentMap;
        #[allow(unused_imports)]
        use std::ops::Range;
        #[allow(unused_imports)]
        use tree_sitter::Node as TsNode;
        #[cfg(test)]
        #[path = "tests.rs"]
        pub mod tests;
    }
    #[cfg_attr(test, path = "eval")]
    mod eval {
        pub use rerail_boot::code_rule_dsl::eval::*;
        pub mod query {
            pub use rerail_boot::code_rule_dsl::eval::query::*;
            #[cfg(test)]
            #[path = "tape_cache_tests.rs"]
            pub mod tape_cache_tests;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::invalid;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::meta_name;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::{GrammarDialect, SourceType};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::lines::{LineTable, Terminators};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::{ FileGates, FileScopeSlot, GrammarMatcher, UtilLimit, WITNESS_NODE, export_text, exported_span, is_span_variable, };
            #[allow(unused_imports)]
            use crate::code_rule_dsl::model::{ CodeRuleScope, FileInput, Limits, Metrics, Query, RegexSource, Snapshot, ViolationSpec, };
            #[allow(unused_imports)]
            use crate::code_rule_dsl::origin::InFile;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::query::select_when::{self, LeafRow, Leaves, Plan};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::soundness::{SyntaxErrors, SyntaxReading};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::violations::{self, Captures, Witness};
            #[allow(unused_imports)]
            use ast_grep_core::matcher::NodeMatch;
            #[allow(unused_imports)]
            use ast_grep_core::meta_var::{MetaVarEnv, MetaVariable};
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use napi::Result;
            #[allow(unused_imports)]
            use serde_json::{Map, Value};
            #[allow(unused_imports)]
            use std::cell::RefCell;
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, HashSet};
            #[allow(unused_imports)]
            use std::ops::Range;
            #[allow(unused_imports)]
            use std::sync::Arc;
        }
        pub mod output {
            pub use rerail_boot::code_rule_dsl::eval::output::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::hex_digest;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::model::{CompiledClause, CompiledCodeRule, Evaluation};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::violations::{self, Span, Witness};
            #[allow(unused_imports)]
            use serde_json::{Value, json};
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, BTreeSet};
        }
        pub mod evidence {
            pub use rerail_boot::code_rule_dsl::eval::evidence::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use super::output::{failure, violation_from_witness, violation_id};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::{exported_span, exported_text};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::model::{ ClauseKind, CompiledClause, CompiledCodeRule, Evaluation, FactQuery, FileInput, Projection, ProjectionSource, Query, ViolationSpec, };
            #[allow(unused_imports)]
            use crate::code_rule_dsl::origin::InFile;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::query::crossfile::{self, Relation, Requirement};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::tape::tape_doc;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::violations::{self, NodeGroup, Witness};
            #[allow(unused_imports)]
            use ast_grep_core::Doc;
            #[allow(unused_imports)]
            use ast_grep_core::matcher::NodeMatch;
            #[allow(unused_imports)]
            use ast_grep_core::replacer::Content;
            #[allow(unused_imports)]
            use serde_json::{Value, json};
            #[allow(unused_imports)]
            use std::collections::BTreeSet;
        }
        #[cfg_attr(not(test), allow(unused_imports))]
        pub use rerail_lang_ecma::code_rule_dsl::eval::ecma;
        #[cfg(test)]
        pub mod tests;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::calls;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::json::{invalid, json_error, required_str};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::adapter::package_manifests;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::model::{ClauseKind, CompiledClause, CompiledCodeRule, FactQuery, FileInput, Metrics, Snapshot, package_marker_directory, python_module_in};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::value::names;
        #[allow(unused_imports)]
        use napi::Result;
        #[allow(unused_imports)]
        pub(super) use crate::code_rule_dsl::eval::output::{
            coverage, evaluation_json, failure, overflow_evaluation, result_order, violation_from_witness,
            violation_id, violation_ids,
        };
        #[allow(unused_imports)]
        pub(super) use crate::code_rule_dsl::eval::query::Output;
        #[allow(unused_imports)]
        pub(super) use crate::code_rule_dsl::eval::query::{
            TapeCacheSlot, WitnessError, document_matcher, evaluate_query, file_language, file_tape,
            grammar_matcher, witness_from_match, witnesses_from_match,
        };
        #[allow(unused_imports)]
        #[cfg(test)]
        pub(super) use crate::code_rule_dsl::eval::query::{reset_shared_group_lookups, shared_group_lookups};
        #[allow(unused_imports)]
        use serde_json::Value;
        #[allow(unused_imports)]
        use std::collections::{BTreeMap, HashSet};
        #[allow(unused_imports)]
        use std::sync::Arc;
    }
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
    #[cfg_attr(test, path = "literal")]
    mod literal {
        pub use rerail_boot::code_rule_dsl::literal::*;
        pub mod offsets {
            pub use rerail_boot::code_rule_dsl::literal::offsets::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use std::ops::Range;
        }
        pub mod annotation {
            pub use rerail_boot::code_rule_dsl::literal::annotation::*;
            #[cfg(test)]
            #[path = "tests.rs"]
            pub mod tests;
            #[allow(unused_imports)]
            use super::offsets::Decode;
            #[allow(unused_imports)]
            use super::{HostMap, PNode, SubTree, semantic};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::GrammarDialect;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::value::Value;
            #[allow(unused_imports)]
            use ast_grep_language::LanguageExt;
            #[allow(unused_imports)]
            use std::ops::Range;
            #[allow(unused_imports)]
            use std::sync::OnceLock;
        }
        #[allow(unused_imports)] // boot's mount: only boot's files read it
        pub use rerail_lang_python::code_rule_dsl::literal::brace;
        pub mod eslint_directive {
            pub use rerail_boot::code_rule_dsl::literal::eslint_directive::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use super::PNode;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::value::Value;
            #[allow(unused_imports)]
            use std::ops::Range;
        }
        #[allow(unused_imports)] // boot's mount: only boot's files read it
        pub use rerail_lang_python::code_rule_dsl::literal::identifier_words;
        #[allow(unused_imports)] // boot's mount: only boot's files read it
        pub use rerail_lang_python::code_rule_dsl::literal::percent;
        #[allow(unused_imports)] // boot's mount: only boot's files read it
        pub use rerail_lang_python::code_rule_dsl::literal::format_tape;
        pub mod phpdoc {
            pub use rerail_boot::code_rule_dsl::literal::phpdoc::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use super::{Grammar, PNode};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::value::Value;
        }
        #[allow(unused_imports)] // boot's mount: only boot's files read it
        pub use rerail_lang_php::code_rule_dsl::literal::printf;
        #[allow(unused_imports)] // boot's mount: only boot's files read it
        pub use rerail_lang_python::code_rule_dsl::literal::python_fstring;
        #[cfg(test)]
        pub use rerail_lang_python::code_rule_dsl::literal::python_string;
        #[allow(unused_imports)] // boot's mount: only boot's files read it
        pub use rerail_lang_rust::code_rule_dsl::literal::rust_format;
        #[allow(unused_imports)] // boot's mount: only boot's files read it
        pub use rerail_lang_php::code_rule_dsl::literal::scanf;
        pub use rerail_lang_python::code_rule_dsl::literal::semantic;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::embed::segments::FileRanges;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::GrammarDialect;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::value::Value;
        #[allow(unused_imports)]
        use ast_grep_core::Position;
        #[allow(unused_imports)]
        use ast_grep_core::language::Language;
        #[allow(unused_imports)]
        use ast_grep_core::matcher::{Pattern, PatternBuilder, PatternError};
        #[allow(unused_imports)]
        use ast_grep_core::source::{Content, Doc, Edit, SgNode};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::literal::offsets::Decoded;
        #[allow(unused_imports)]
        use std::borrow::Cow;
        #[allow(unused_imports)]
        use std::ops::Range;
        #[allow(unused_imports)]
        use std::sync::Arc;
        #[allow(unused_imports)]
        use std::sync::atomic::{AtomicUsize, Ordering};
        #[cfg(test)]
            pub mod tests;
    }
    #[cfg_attr(test, path = "matcher")]
    mod matcher {
        pub use rerail_boot::code_rule_dsl::matcher::*;
        #[cfg_attr(test, path = "arguments")]
        pub mod arguments {
            pub use rerail_boot::code_rule_dsl::matcher::arguments::*;
            #[cfg(test)]
            pub use rerail_lang_php::code_rule_dsl::matcher::arguments::lang_php;
            #[cfg(test)]
            pub use rerail_lang_python::code_rule_dsl::matcher::arguments::lang_python;
            #[cfg(test)]
            pub use rerail_lang_rust::code_rule_dsl::matcher::arguments::rust;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::tree::Tree;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::util::UNKNOWN;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::value::spend_step;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::identifier::python_identifier;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::{Family, GrammarDialect};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::value::{Context, Value};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::value::{php, python};
            #[allow(unused_imports)]
            use ast_grep_core::meta_var::MetaVarEnv;
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use std::borrow::Cow;
            #[allow(unused_imports)]
            use std::collections::BTreeSet;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::pack as language_pack;
            #[cfg(test)]
                    pub mod tests;
            #[cfg(test)]
            #[path = "php_syntactic_tests.rs"]
            pub mod php_syntactic_tests;
            #[cfg(test)]
            #[path = "rust_tests.rs"]
            pub mod rust_tests;
        }
        #[allow(unused_imports)]
        #[cfg(test)]
        pub(in crate::code_rule_dsl) use crate::code_rule_dsl::matcher::p_anchors::sql::{Corpus, CorpusSlot};
        #[allow(unused_imports)]
        #[cfg(all(test, feature = "p-anchor-corpus"))]
        pub(in crate::code_rule_dsl) use crate::code_rule_dsl::matcher::p_anchors::sql::{anchor_plans, anchor_plans_for_file};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::budget;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::{GrammarDialect, SourceType};
        #[allow(unused_imports)]
        pub(super) use crate::code_rule_dsl::matcher::analysis::RequiredText;
        #[allow(unused_imports)]
        pub(super) use crate::code_rule_dsl::matcher::lower::{Axis, Fact, FactFound, MatchTree, Stop, DocumentGate, gated_match_tree, match_tree};
        #[allow(unused_imports)]
        pub(crate) use crate::code_rule_dsl::matcher::analysis::{calls_positive_only, reads_import_targets};
        #[allow(unused_imports)]
        pub(super) use crate::code_rule_dsl::matcher::anchors::{AnchorPlan, FileAnchorIndex, Selection};
        #[allow(unused_imports)]
        pub(super) use crate::code_rule_dsl::matcher::anchors::{Condition, Direction, Required, Requirement};
        #[allow(unused_imports)]
        pub(super) use crate::code_rule_dsl::matcher::arguments::{Arguments, Passed, SlotKind};
        #[allow(unused_imports)]
        use ast_grep_core::matcher::Matcher;
        #[allow(unused_imports)]
        use ast_grep_core::matcher::{KindMatcher, Pattern};
        #[allow(unused_imports)]
        use ast_grep_core::matcher::{MatcherExt, NodeMatch};
        #[allow(unused_imports)]
        use ast_grep_core::meta_var::{MetaVarEnv, MetaVariable};
        #[allow(unused_imports)]
        use ast_grep_core::{Doc, Node};
        #[allow(unused_imports)]
        pub(super) use crate::code_rule_dsl::matcher::binding::BindingTest;
        #[allow(unused_imports)]
        pub(super) use crate::code_rule_dsl::matcher::binds::{BindingCount, BindingFilter, Binds, Redefines, ScopeName};
        #[allow(unused_imports)]
        pub(super) use crate::code_rule_dsl::matcher::captures::{Capture, CaptureAnalysis, collect_variables, parse_exported};
        #[allow(unused_imports)]
        pub(super) use crate::code_rule_dsl::matcher::compare::{Collation, CompareOp, CompareText, Comparison, EqClass, SemanticText};
        #[allow(unused_imports)]
        pub(super) use crate::code_rule_dsl::matcher::count::{Count, CountBinding};
        #[allow(unused_imports)]
        pub(crate) use crate::code_rule_dsl::matcher::each::binding_range;
        #[allow(unused_imports)]
        pub(crate) use crate::code_rule_dsl::matcher::embedded::EmbeddedTest;
        #[allow(unused_imports)]
        pub(crate) use crate::code_rule_dsl::matcher::exported::{
            WITNESS_NODE, export_text, exported_span, exported_text, is_span_variable,
        };
        #[allow(unused_imports)]
        pub(crate) use crate::code_rule_dsl::matcher::format_args::{FormatArgs, Style};
        #[allow(unused_imports)]
        pub(crate) use crate::code_rule_dsl::matcher::layout::{Gap, Layout, LineContext, LineCount, Neighbour};
        #[allow(unused_imports)]
        pub(super) use crate::code_rule_dsl::matcher::memo::with_file as with_file_memo;
        #[allow(unused_imports)]
        pub(super) use crate::code_rule_dsl::matcher::nth_child::{NthChild, Position as NthPosition, parse_position};
        #[allow(unused_imports)]
        pub(in crate::code_rule_dsl) use crate::code_rule_dsl::matcher::nth_child::SiblingPositions;
        #[allow(unused_imports)]
        #[cfg(test)]
        pub(crate) use crate::code_rule_dsl::matcher::nth_child::{named_sibling_visits, reset_named_sibling_visits};
        #[allow(unused_imports)]
        #[cfg(test)]
        pub(crate) use crate::code_rule_dsl::matcher::parse::parse_fact_tree;
        #[allow(unused_imports)]
        pub(crate) use crate::code_rule_dsl::matcher::parse::{HostTest, ParseTest};
        #[allow(unused_imports)]
        use regex::Regex;
        #[allow(unused_imports)]
        pub(super) use crate::code_rule_dsl::matcher::relation::{FieldRelation, Related, StopBy, stop_rule};
        #[allow(unused_imports)]
        #[cfg(test)]
        pub(super) use crate::code_rule_dsl::matcher::relation::{set_answers_off, set_same_key_off};
        #[allow(unused_imports)]
        pub(crate) use crate::code_rule_dsl::matcher::resolve::{
            FileGates, calls_tables, file_index, file_lines, file_module, file_rerail_ignores, file_source_type_for,
            table_reading,
        };
        #[allow(unused_imports)]
        pub(super) use crate::code_rule_dsl::matcher::resolve::{FileScopeSlot, Unresolved, matched_file_path};
        #[allow(unused_imports)]
        pub(crate) use crate::code_rule_dsl::matcher::resolve::Resolves;
        #[allow(unused_imports)]
        pub(crate) use crate::code_rule_dsl::matcher::resolve::php as resolve_php;
        #[allow(unused_imports)]
        #[cfg(test)]
        pub(super) use crate::code_rule_dsl::matcher::resolve::{JSX_RUNTIME_DERIVATIONS, SOURCE_TYPE_DERIVATIONS, TABLE_BUILDS};
        #[allow(unused_imports)]
        pub(super) use crate::code_rule_dsl::matcher::run::{Stopped, enumerate, node_at, run_clause};
        #[allow(unused_imports)]
        #[cfg(test)]
        pub(crate) use crate::code_rule_dsl::matcher::run::{reset_roots_matched, roots_matched};
        #[allow(unused_imports)]
        pub(super) use crate::code_rule_dsl::matcher::semantic::Semantic;
        #[allow(unused_imports)]
        use std::borrow::Cow;
        #[allow(unused_imports)]
        use std::collections::BTreeSet;
        #[allow(unused_imports)]
        use std::sync::Arc;
        #[allow(unused_imports)]
        pub(super) use crate::code_rule_dsl::matcher::transform::{Digits, Separator, Source, StringCase, Trans, Transform};
        #[allow(unused_imports)]
        pub(crate) use crate::code_rule_dsl::matcher::transform::{NumberText, install_number_text, number_to_string};
        #[allow(unused_imports)]
        #[cfg(test)]
        pub(crate) use crate::code_rule_dsl::matcher::transform::{
            line_table_constructions, note_line_table_construction, reset_line_table_constructions,
        };
        #[allow(unused_imports)]
        pub(super) use crate::code_rule_dsl::matcher::util::{
            CallArgs, MAX_UTIL_DEPTH, MAX_UTIL_INSTANCES, UtilCall, UtilLimit, UtilOrigin, UtilRef,
            UtilTable, UtilitySummary,
        };
        #[allow(unused_imports)]
        use crate::code_rule_dsl::matcher::util::{UNKNOWN};
        #[allow(unused_imports)]
        pub(super) use crate::code_rule_dsl::matcher::value::{Bound, Reading, ValueTest};
        #[allow(unused_imports)]
        pub(crate) use crate::code_rule_dsl::matcher::value::Values;
        #[allow(unused_imports)]
        pub(crate) use crate::code_rule_dsl::matcher::term::{LanguageTerm, TermCompiler, install_terms};
        #[allow(unused_imports)]
        use util::{ MAX_UTIL_STACK_BYTES, UTIL_DEPTH, UTIL_EXCEEDED, UTIL_STACK_BASE, UTIL_STEPS_LEFT, stack_address, };
        #[allow(unused_imports)]
        use util::{THREE_VALUED};
    }
    #[cfg_attr(test, path = "php_view")]
    mod php_view {
        pub use rerail_boot::code_rule_dsl::php_view::*;
        #[cfg(test)]
        pub mod tests;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::blade::forte::scan;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::php_literal;
        #[allow(unused_imports)]
        use std::borrow::Cow;
        #[allow(unused_imports)]
        use std::ops::Range;
    }
    #[cfg_attr(test, path = "query")]
    mod query {
        pub use rerail_boot::code_rule_dsl::query::*;
        #[cfg(test)]
        pub mod tests;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::model::RegexOptions;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::model::{FileInput, RegexSource};
        #[allow(unused_imports)]
        use regex::Regex;
        #[allow(unused_imports)]
        use std::ops::Range;
    }
    // P's paths for P's relational stage, under test (open item O18): P's workspace code reads it here.
    #[cfg_attr(test, path = "scope")]
    mod scope {
        pub use rerail_boot::code_rule_dsl::scope::*;
        pub use rerail_lang_ecma::code_rule_dsl::scope::ecma;
        pub mod php {
            pub use rerail_boot::code_rule_dsl::scope::php::*;
            #[cfg(test)]
                    pub mod flow_tests;
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
    pub mod regexes {
        pub use rerail_boot::code_rule_dsl::regexes::*;
        #[cfg(test)]
        pub mod tests;
        #[allow(unused_imports)]
        use std::sync::Arc;
    }
    pub mod soundness {
        pub use rerail_boot::code_rule_dsl::soundness::*;
        #[cfg(test)]
            pub mod tests;
        #[allow(unused_imports)]
        use super::embed::segments::FileRanges;
        #[allow(unused_imports)]
        use super::lines::LineTable;
        #[allow(unused_imports)]
        use super::matcher::{AstMatcher, GrammarMatcher, UtilRef, UtilitySummary, stop_rule};
        #[allow(unused_imports)]
        use super::origin::Origin;
        #[allow(unused_imports)]
        use super::tape::TapeDoc;
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use crate::code_rule_dsl::walks::{Walk, record_walk};
        #[allow(unused_imports)]
        use std::collections::BTreeSet;
        #[allow(unused_imports)]
        use std::ops::Range;
    }
    #[cfg(test)]
    mod test_support;
    #[cfg_attr(test, path = "value")]
    mod value {
        pub use rerail_boot::code_rule_dsl::value::*;
        pub mod ecma {
            pub use rerail_boot::code_rule_dsl::value::ecma::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::tree::Tree;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::Family;
            #[allow(unused_imports)]
            use super::{ContainerKind, Context, MAX_ELEMENTS, MAX_STRING_BYTES, Value};
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
        }
        pub mod key {
            pub use rerail_boot::code_rule_dsl::value::key::*;
            #[cfg(test)]
            #[path = "tests.rs"]
            pub mod tests;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::tree::Tree;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::Family;
            #[allow(unused_imports)]
            use super::ecma::{cooked, number, operand, ops, string_node};
            #[allow(unused_imports)]
            use super::{BigInt, Value};
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use num_traits::Zero;
        }
        pub mod php {
            pub use rerail_boot::code_rule_dsl::value::php::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::tree::Tree;
            #[allow(unused_imports)]
            use super::{BigInt, ContainerKind, Context, Value};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::value::NodeRef;
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use num_traits::ToPrimitive;
            #[allow(unused_imports)]
            use std::cmp::Ordering;
        }
        pub mod php_compare {
            pub use rerail_boot::code_rule_dsl::value::php_compare::*;
            #[cfg(test)]
            #[path = "tests.rs"]
            pub mod tests;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::tree::Tree;
            #[allow(unused_imports)]
            use super::php::{self, Key};
            #[allow(unused_imports)]
            use super::{Context, Value};
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
        }
        pub use rerail_lang_php::code_rule_dsl::value::php_structure;
        pub mod python {
            pub use rerail_boot::code_rule_dsl::value::python::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::tree::Tree;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::Family;
            #[allow(unused_imports)]
            use super::{BigInt, ContainerKind, Context, MAX_INT_BITS, MAX_STRING_BYTES, Value, int_to_float};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::identifier::python_identifier;
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
        }
        pub use rerail_lang_python::code_rule_dsl::value::python_ast;
        pub mod rust {
            pub use rerail_boot::code_rule_dsl::value::rust::*;
            #[cfg(test)]
            #[path = "tests.rs"]
            pub mod tests;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::tree::Tree;
            #[allow(unused_imports)]
            use super::Context;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::scope::rust::resolve_index;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::scope::{Form, ScopeTable};
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use num_bigint::BigInt;
            #[allow(unused_imports)]
            use num_traits::{FromPrimitive, Num, ToPrimitive, Zero};
            #[allow(unused_imports)]
            use std::cell::Cell;
        }
        pub mod sql {
            pub use rerail_boot::code_rule_dsl::value::sql::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::tree::Tree;
            #[allow(unused_imports)]
            use super::{Value, int_float_order};
            #[allow(unused_imports)]
            use num_bigint::BigInt;
            #[allow(unused_imports)]
            use num_traits::ToPrimitive;
            #[allow(unused_imports)]
            use std::borrow::Cow;
            #[allow(unused_imports)]
            use std::cmp::Ordering;
        }
        #[cfg(test)]
        pub mod tests;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::{Family, GrammarDialect};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::tape::TapeDoc;
        #[allow(unused_imports)]
        use ast_grep_core::tree_sitter::StrDoc;
        #[allow(unused_imports)]
        use ast_grep_core::{Doc, Node};
        #[allow(unused_imports)]
        pub(crate) use num_bigint::BigInt;
        #[allow(unused_imports)]
        use num_traits::{FromPrimitive, Signed, ToPrimitive, Zero};
        #[allow(unused_imports)]
        use std::cmp::Ordering;
        #[allow(unused_imports)]
        use std::ops::Range;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::adapter::pack as language_pack;
        #[allow(unused_imports)]
        pub(crate) use crate::code_rule_dsl::value::text_regex::MAX_STRING_BYTES;
        #[cfg(test)]
            pub mod rules;
        #[cfg(test)]
            pub mod eslint_rules;
    }
}
