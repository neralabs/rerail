//! The engine's composition root (docs/final-shape/proposals/crate-split/incremental-plan.md, step 1).
//!
//! It registers every provider the build selects (the language packs, Metal, the engine's glue) into the core's
//! registries, in one order, once a process (`code_rule_dsl::boot::install`). It holds the glue those registrations
//! name: the prefilter, the functors, the `ast` wave compiler and the waves' glue, and the syntax-only adapters.
//! The engine (napi and the CLI) stands on it, and so can any test binary that needs a booted program.
//!
//! Its sources stay where they were (native/src): this crate mounts them in place, under the facades the engine
//! mounted them through (tools/cratify/boot_mounts.py wrote them from the engine's `code_rule_dsl/mod.rs`; kept by
//! hand since step 2). A facade's re-exports are `pub`, and its tests' helpers are behind `test-hooks`: the test
//! crates above (`native/crates/tests_*`) read every module through it.
// A mounted module re-exports its dependency's by glob, and its sources import the same items
// under the same names: the glob's name and the private one are one item. The mounts also widen what the crates
// above read (the split's facades, not a design).
#![allow(dead_code, unused_imports, hidden_glob_reexports, ambiguous_glob_reexports, private_interfaces, private_bounds)]
#![allow(clippy::len_without_is_empty, clippy::result_unit_err, clippy::new_without_default)]
#![allow(clippy::type_complexity, clippy::too_many_arguments)]
// The browser build has no JIT: wasm/cranelift-jit stands in under the same name (wasm/README.md).
#[cfg(target_family = "wasm")]
extern crate cranelift_jit_wasm as cranelift_jit;
// jemalloc's per-thread counters, which the composition hands the facts' accounting (`boot::heap_net`).
#[cfg(not(any(windows, target_family = "wasm", feature = "system-allocator")))]
extern crate tikv_jemalloc_ctl;
pub use rerail_store::content_store;
pub use rerail_store::git_access;
pub use rerail_store::git_workspace;
pub use rerail_store::source_tree;
pub use rerail_workspace::git_ignore;
pub use rerail_workspace::panic_boundary;
pub use rerail_logic::threads;
pub use rerail_tree_watch::tree_watch;
extern crate self as rerail_boot;
// The CLI's tools, at the root as the engine has them (`crate::tools`).
pub use code_rule_dsl::tools;

// This crate's test binary keeps the engine's test-binary rules: each test's time limit (`RERAIL_TEST_TIMEOUT`),
// and stores that flush nothing (`RERAIL_FLUSH=none`, decision 0481), both set before `main`.
#[cfg(test)]
#[path = "../../../src/test_watchdog.rs"]
mod test_watchdog;
#[cfg(test)]
#[path = "../../../src/test_flush.rs"]
mod test_flush;
#[cfg(test)]
#[path = "../../../src/test_private_folders.rs"]
mod test_private_folders;
pub use code_rule_dsl::prof::{lower_hex, purge_allocator_pages};
#[path = "../../../src/code_rule_dsl"]
pub mod code_rule_dsl {
    // The facades re-export their dependency's module by glob, and the mounted sources import the same items under
    // the same names: the glob's name and the private one are one item.
    #![allow(hidden_glob_reexports)]
    pub mod blade {
        pub mod forte {
            pub mod tree {
                #[cfg(any(test, feature = "test-hooks"))]
                pub use rerail_lang_php::code_rule_dsl::blade::forte::tree::*;
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
            pub use rerail_lang_php::code_rule_dsl::blade::forte::*;
            #[allow(unused_imports)]
            use directives::Directives;
            #[allow(unused_imports)]
            use node::FlatNode;
            #[allow(unused_imports)]
            use token::{LexerError, Token, TokenType};
        }
        #[cfg_attr(not(test), allow(unused_imports))]
        pub use rerail_lang_php::code_rule_dsl::blade::*;
    }
    pub use rerail_lang::code_rule_dsl::budget;
    // The engine's build identities, which it sets as it loads (`crate::install_build_ids`).
    pub use rerail_lang::code_rule_dsl::build_ids;
    pub mod calls {
        pub use rerail_workspace::code_rule_dsl::calls::*;
        pub mod closure {
            pub use rerail_workspace::code_rule_dsl::calls::closure::*;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::calls::names::Name;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::calls::php;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::calls::summary::{BASE, CLASS, DECORATED, METACLASS, OPEN, Summary, UNKNOWN_BASE};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::Family;
            #[allow(unused_imports)]
            use rayon::prelude::*;
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
            #[allow(unused_imports)]
            use std::sync::Arc;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::pack as language_pack;
        }
        pub use rerail_lang_php::code_rule_dsl::calls::php;
        #[cfg(any(test, feature = "test-hooks"))]
        pub use rerail_lang_python::code_rule_dsl::calls::python;
        pub mod rust {
            pub use rerail_lang_rust::code_rule_dsl::calls::rust::*;
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
        pub mod summary {
            pub use rerail_workspace::code_rule_dsl::calls::summary::*;
            #[cfg(any(test, feature = "test-hooks"))]
            pub use rerail_lang_python::code_rule_dsl::calls::summary::python;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::tree::Tree;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::calls::names::Name;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::Family;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::module_path::ModulePath;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::scope::{Binding, Form, Import, ScopeKind, ScopeTable};
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use foldhash::{HashMap, HashMapExt};
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
    }
    pub mod compile {
        pub mod value {
            pub use rerail_workspace::code_rule_dsl::compile::value::*;
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
            pub use rerail_workspace::code_rule_dsl::compile::utils::*;
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
            pub use rerail_workspace::code_rule_dsl::compile::shared::*;
        }
        pub mod order {
            pub use rerail_workspace::code_rule_dsl::compile::order::*;
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
            pub use rerail_workspace::code_rule_dsl::compile::dialect::*;
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
            pub use rerail_workspace::code_rule_dsl::compile::define::*;
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
            pub use rerail_workspace::code_rule_dsl::compile::config::*;
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
            pub use rerail_workspace::code_rule_dsl::compile::compare::*;
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
            pub use rerail_workspace::code_rule_dsl::compile::ast_rule::*;
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
        pub use rerail_workspace::code_rule_dsl::compile::*;
        pub mod binds {
            pub use rerail_workspace::code_rule_dsl::compile::binds::*;
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
            #[allow(unused_imports)]
            use super::ast_rule::{validate_ast_rule};
            #[allow(unused_imports)]
            use super::resolve::{BINDING_KEYS, OCCURRENCE_KEY};
            #[allow(unused_imports)]
            use super::utils::{UtilIds};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::{gate};
        }
        pub mod format_args {
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
        pub mod parse {
            pub use rerail_workspace::code_rule_dsl::compile::parse::*;
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
        pub mod resolve {
            pub use rerail_workspace::code_rule_dsl::compile::resolve::*;
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
        pub use crate::code_rule_dsl::compile::ast_rule::compile_ast_rule;
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
        pub use crate::code_rule_dsl::compile::utils::UtilScope;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::adapter::pack as language_pack;
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
    }
    #[cfg_attr(not(test), allow(unused_imports))]
    pub use rerail_lang_python::code_rule_dsl::cpython;
    pub mod demand {
        pub mod vtab {
            pub use rerail_transducer::code_rule_dsl::demand::vtab::*;
            #[allow(unused_imports)]
            use super::super::language::Family;
            #[allow(unused_imports)]
            use super::super::transducer::packed::{ Error as PackedError, ExecutionBackend, PlanId, Program, execute_planned, packed_program_counts, packed_workspace_fits, packed_workspace_size, };
            #[allow(unused_imports)]
            use super::super::tape::TapeDoc;
            #[allow(unused_imports)]
            use super::super::value::structure::Equality;
            #[allow(unused_imports)]
            use super::super::value::Context as ValueContext;
            #[allow(unused_imports)]
            use ast_grep_core::{AstGrep, Doc};
            #[allow(unused_imports)]
            use fallible_iterator::FallibleIterator as _;
            #[allow(unused_imports)]
            use rusqlite::ffi;
            #[allow(unused_imports)]
            use rusqlite::vtab::{ Context, Filters, IndexConstraintOp, IndexInfo, Module, VTab, VTabConfig, VTabConnection, VTabCursor, };
            #[allow(unused_imports)]
            use rusqlite::{Connection, Result as SqlResult};
            #[allow(unused_imports)]
            use std::borrow::Cow;
            #[allow(unused_imports)]
            use std::collections::{BTreeSet, HashMap};
            #[allow(unused_imports)]
            use std::ffi::{CStr, c_int};
            #[allow(unused_imports)]
            use std::marker::PhantomData;
            #[allow(unused_imports)]
            use std::ops::Range;
            #[allow(unused_imports)]
            use std::panic::{AssertUnwindSafe, catch_unwind};
            #[allow(unused_imports)]
            use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
            #[allow(unused_imports)]
            use std::sync::{Arc, Mutex, OnceLock};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::pack as language_pack;
        }
        pub mod driver {
            #[cfg_attr(not(test), allow(unused_imports))]
            pub use rerail_transducer::code_rule_dsl::demand::driver::*;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::demand::vtab::{MatchRegistry, PackedDemand, register_module};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::transducer::packed::ExecutionBackend;
            #[allow(unused_imports)]
            use crate::content_store::{ContentStore, StoreResult};
            #[allow(unused_imports)]
            use rusqlite::{Connection, OptionalExtension as _, Row};
            #[allow(unused_imports)]
            use serde_json::{Value, json};
            #[allow(unused_imports)]
            use std::collections::BTreeMap;
            #[allow(unused_imports)]
            use std::sync::{Arc, LazyLock};
        }
    }
    // Metal as the engine's device (`rerail-device-metal`): registered as the library loads (`install_devices`).
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    #[allow(unused_imports)] // boot's mount: only boot's files read it
    pub use rerail_device_metal::code_rule_dsl::device_metal;
    pub mod embed {
        pub use rerail_lang::code_rule_dsl::embed::*;
        #[allow(unused_imports)] // boot's mount: only boot's files read it
        pub use rerail_lang_php::code_rule_dsl::embed::compiled_view;
        #[allow(unused_imports)] // boot's mount: only boot's files read it
        pub use rerail_lang_ecma::code_rule_dsl::embed::script_block;
        #[cfg_attr(not(test), allow(unused_imports))]
        pub use rerail_lang_python::code_rule_dsl::embed::call_argument;
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        pub use crate::code_rule_dsl::embed::test_script::TestScript;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::GrammarDialect;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::embed::segments::SegmentMap;
        #[allow(unused_imports)]
        use std::ops::Range;
        #[allow(unused_imports)]
        use tree_sitter::Node as TsNode;
    }
    pub mod eval {
        pub mod query {
            pub use rerail_workspace::code_rule_dsl::eval::query::*;
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
            pub use rerail_workspace::code_rule_dsl::eval::output::*;
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
            #[cfg(any(test, feature = "test-hooks"))]
            pub use rerail_workspace::code_rule_dsl::eval::evidence::*;
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
        pub use rerail_workspace::code_rule_dsl::eval::*;
        #[cfg_attr(not(test), allow(unused_imports))]
        pub use rerail_lang_ecma::code_rule_dsl::eval::ecma;
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
        pub use crate::code_rule_dsl::eval::output::{
            coverage, evaluation_json, failure, overflow_evaluation, result_order, violation_from_witness,
            violation_id, violation_ids,
        };
        #[allow(unused_imports)]
        pub use crate::code_rule_dsl::eval::query::Output;
        #[allow(unused_imports)]
        pub use crate::code_rule_dsl::eval::query::{
            TapeCacheSlot, WitnessError, document_matcher, evaluate_query, file_language, file_tape,
            grammar_matcher, witness_from_match, witnesses_from_match,
        };
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        pub use crate::code_rule_dsl::eval::query::{reset_shared_group_lookups, shared_group_lookups};
        #[allow(unused_imports)]
        use serde_json::Value;
        #[allow(unused_imports)]
        use std::collections::{BTreeMap, HashSet};
        #[allow(unused_imports)]
        use std::sync::Arc;
    }
    #[cfg_attr(test, path = "facts")]
    pub mod facts {
        pub mod families {
            #[cfg_attr(not(test), allow(unused_imports))]
            pub use rerail_facts::code_rule_dsl::facts::families::*;
            #[allow(unused_imports)]
            use super::Graph;
            #[allow(unused_imports)]
            use super::core::{self, Facts};
            #[allow(unused_imports)]
            use super::observe::{Push, observe_root, path_name};
            #[allow(unused_imports)]
            use super::propagate::propagate;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::{GrammarDialect, dialect_by_label, label_of_path, tape_tag};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::lines::{LineTable, Terminators};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::facts::vtab::syntax::{Families as LazyFamilies, families};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::tape::{self, FileImage, TAPE_FORMAT_VERSION, TapeDoc, TapeImage};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::build_ids::grammar_version;
            #[allow(unused_imports)]
            use crate::content_store::prefilter::{PREFILTER_VERSION, TapePrefilter};
            #[allow(unused_imports)]
            use crate::content_store::{ ABSENT, Derivation, DerivationInput, DerivationRecord, HashKey, SourceId, StoreError, StoreResult, TapeId, };
            #[allow(unused_imports)]
            use sha2::{Digest as _, Sha256};
            #[allow(unused_imports)]
            use std::collections::{HashMap, HashSet};
            #[allow(unused_imports)]
            use std::sync::Arc;
        }
        pub mod core {
            pub use rerail_facts::code_rule_dsl::facts::core::*;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::tape_tag;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::lines::LineTable;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::tape::TapeDoc;
            #[allow(unused_imports)]
            use std::sync::Arc;
        }
        pub use rerail_facts::code_rule_dsl::facts::*;
        pub mod producers {
            pub mod python {
                pub mod python_facts {
                    pub use rerail_lang_python::code_rule_dsl::facts::producers::python::python_facts::*;
                    #[allow(unused_imports)]
                    use crate::code_rule_dsl::{ identifier::python_identifier, language::{ Family, GrammarDialect, adapter::{python::Python, ScopeRules}, }, module_path::ModulePath, scope::{ Binding, Form, Import, Lookup, Origin, ScopeTable, options::{PythonOptions, python_builtins}, }, value::NodeRef, };
                    #[allow(unused_imports)]
                    use crate::code_rule_dsl::tree::Tree;
                    #[allow(unused_imports)]
                    use ast_grep_core::{Doc, Node};
                    #[allow(unused_imports)]
                    use std::collections::{BTreeMap, HashSet};
                }
                pub mod import_screen {
                    #[cfg(any(test, feature = "test-hooks"))]
                    pub use rerail_lang_python::code_rule_dsl::facts::producers::python::import_screen::*;
                    #[allow(unused_imports)]
                    use crate::code_rule_dsl::{ identifier::{nfkc, python_identifier}, facts::producers::python::python_facts::{Fact, qualification_chain}, module_path::ModulePath, tape::TapeDoc, };
                    #[allow(unused_imports)]
                    use ast_grep_core::{Doc, Node};
                    #[allow(unused_imports)]
                    use std::collections::{BTreeMap, BTreeSet};
                }
                pub mod families {
                    pub mod candidates {
                        #[cfg(any(test, feature = "test-hooks"))]
                        pub use rerail_lang_python::code_rule_dsl::facts::producers::python::families::candidates::*;
                        #[allow(unused_imports)]
                        use super::*;
                        #[allow(unused_imports)]
                        use std::collections::BTreeSet;
                    }
                    #[cfg(any(test, feature = "test-hooks"))]
                    pub use rerail_lang_python::code_rule_dsl::facts::producers::python::families::*;
                    #[allow(unused_imports)]
                    use crate::code_rule_dsl::facts::core::{Facts, NONE, kind_atom};
                    #[allow(unused_imports)]
                    use crate::code_rule_dsl::language::KindCatalog;
                }
                pub mod call_batch {
                    #[cfg(any(test, feature = "test-hooks"))]
                    pub use rerail_lang_python::code_rule_dsl::facts::producers::python::call_batch::*;
                    #[allow(unused_imports)]
                    use super::python_structure::{ Argument, Attribute, CallSite, ContextKind, ExecutionContext, NodeRef, PythonStructure, Spread, };
                    #[allow(unused_imports)]
                    use crate::code_rule_dsl::tape::TapeDoc;
                    #[allow(unused_imports)]
                    use ast_grep_core::{Doc, Node, language::Language};
                    #[allow(unused_imports)]
                    use rustc_hash::FxHashSet;
                    #[allow(unused_imports)]
                    use std::{cmp::Reverse, collections::BinaryHeap};
                }
                pub use rerail_lang_python::code_rule_dsl::facts::producers::python::*;
            }
        }
        #[cfg_attr(test, path = "vtab")]
        pub mod vtab {
            pub use rerail_facts::code_rule_dsl::facts::vtab::*;
            #[allow(unused_imports)] // boot's mount: only boot's files read it
            pub use rerail_lang_python::code_rule_dsl::facts::vtab::python;
            #[cfg_attr(test, path = "syntax")]
            pub mod syntax {
                pub mod core_rr {
                    pub use rerail_facts::code_rule_dsl::facts::vtab::syntax::core_rr::*;
                    #[allow(unused_imports)]
                    use super::{Dialect, Projected, Relation};
                    #[allow(unused_imports)]
                    use crate::code_rule_dsl::facts::core::Facts;
                    #[allow(unused_imports)]
                    use std::collections::{BTreeSet, HashSet};
                }
                pub use rerail_facts::code_rule_dsl::facts::vtab::syntax::*;
                #[allow(unused_imports)] // boot's mount: only boot's files read it
                pub use rerail_lang_python::code_rule_dsl::facts::vtab::syntax::annotation;
                #[allow(unused_imports)] // boot's mount: only boot's files read it
                pub mod ast_wave;
                // The syntax tests' fixtures (the planner corpus), which the tests of the crates above read too.
                #[cfg(any(test, feature = "test-hooks"))]
                pub mod fixtures;
                #[cfg(any(test, feature = "test-hooks"))]
                pub use rerail_lang_python::code_rule_dsl::facts::vtab::syntax::candidates;
                #[allow(unused_imports)] // boot's mount: only boot's files read it
                pub use rerail_lang_python::code_rule_dsl::facts::vtab::syntax::cpython;
                #[allow(unused_imports)] // boot's mount: only boot's files read it
                pub use rerail_lang_ecma::code_rule_dsl::facts::vtab::syntax::ecma_core;
                #[allow(unused_imports)] // boot's mount: only boot's files read it
                pub use rerail_lang_python::code_rule_dsl::facts::vtab::syntax::fstring;
                #[cfg_attr(not(test), allow(unused_imports))]
                pub use rerail_lang_python::code_rule_dsl::facts::vtab::syntax::py_families;
                #[allow(unused_imports)] // boot's mount: only boot's files read it
                pub use rerail_lang_python::code_rule_dsl::facts::vtab::syntax::python_core;
                pub mod python_demand {
                    #[cfg_attr(not(test), allow(unused_imports))]
                    pub use rerail_lang_python::code_rule_dsl::facts::vtab::syntax::python_demand::*;
                    #[allow(unused_imports)]
                    use super::{Batch, NONE, Projected};
                    #[allow(unused_imports)]
                    use crate::code_rule_dsl::facts::producers::python::families as py;
                    #[allow(unused_imports)]
                    use crate::code_rule_dsl::{ facts::producers::python::{ python_facts::{self, Fact, NodeFacts, Qualification}, }, language::GrammarDialect, scope::{ Reading, ScopeTable, options::{DEFAULT_PYTHON_MINOR, PythonOptions}, }, tape::{Grammar, TapeDoc, TapeImage}, };
                    #[allow(unused_imports)]
                    use ast_grep_core::{AstGrep, Node, tree_sitter::StrDoc};
                    #[allow(unused_imports)]
                    use rusqlite::types::Value;
                    #[allow(unused_imports)]
                    use std::{ collections::{BTreeMap, BTreeSet}, sync::Arc, };
                }
                pub mod python_parts {
                    #[cfg(any(test, feature = "test-hooks"))]
                    pub use rerail_lang_python::code_rule_dsl::facts::vtab::syntax::python_parts::*;
                    #[allow(unused_imports)]
                    use super::core_rr::py_unescape;
                    #[allow(unused_imports)]
                    use super::{Dialect, Projected};
                }
                #[allow(unused_imports)] // boot's mount: only boot's files read it
                pub use rerail_logic::code_rule_dsl::facts::vtab::syntax::rr_core;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::facts::vtab::{Modules, Relation, Source};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::facts::Graph;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::facts::core::{Facts, KIND_STRIDE, NONE, TEXT, kind_atom, kind_code};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::language::{KindCatalog, from_tape_tag};
                #[allow(unused_imports)]
                use crate::content_store::{ContentStore, StoreError, StoreResult};
                #[allow(unused_imports)]
                use rusqlite::Connection;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::logic::front::ast::Value;
                #[allow(unused_imports)]
                use std::collections::{BTreeSet, HashMap};
                #[allow(unused_imports)]
                use std::sync::{Arc, OnceLock};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::facts::vtab::syntax::core_rr::Family;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::facts::vtab::syntax::guests::GuestRows;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::facts::vtab::syntax::granules::INDEX_BITS;
            }
            #[allow(unused_imports)]
            use fallible_iterator::FallibleIterator;
            #[allow(unused_imports)]
            use rusqlite::{
                Connection, Result,
                types::Value,
                vtab::{
                    Context, Filters, IndexConstraintOp, IndexInfo, Module, VTab, VTabConfig, VTabConnection,
                    VTabCursor,
                },
            };
            #[allow(unused_imports)]
            use std::{
                borrow::Cow,
                cell::RefCell,
                collections::{BTreeMap, HashMap},
                ffi::{CStr, CString, c_int},
                ops::Range,
                sync::{
                    Arc, OnceLock,
                    atomic::{AtomicU64, Ordering},
                },
            };
        }
        #[allow(unused_imports)]
        use crate::content_store::{Binding, ContentStore, Derivation, DerivationRecord, StoreResult};
    }
    pub use rerail_workspace::code_rule_dsl::file_input;
    // Wired to the findings layer in a later slice; until then only tests call it.
    pub mod findings {
        pub use rerail_workspace::code_rule_dsl::findings::*;
        #[cfg(any(test, feature = "test-hooks"))]
        pub mod async_reference {
            pub use rerail_lang_python::code_rule_dsl::findings::async_reference::*;
            // Its timings are read by the p-anchor-corpus tests alone.
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
        }
    }
    // The relational pools' hook slots, compiled wherever the pools are (the hub's relational stage is in the library).
    pub mod identifier {
        pub use rerail_workspace::code_rule_dsl::identifier::*;
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
    #[allow(unused_imports)] // boot's mount: only boot's files read it
    pub use rerail_lang_ecma::code_rule_dsl::import_target;
    pub use rerail_lang::code_rule_dsl::json;
    #[cfg_attr(test, path = "language")]
    pub mod language {
        pub use rerail_lang::code_rule_dsl::language::*;
        #[cfg_attr(test, path = "adapter")]
        pub mod adapter {
            pub use rerail_workspace::code_rule_dsl::language::adapter::*;
            pub mod arguments {
                pub use rerail_workspace::code_rule_dsl::language::adapter::arguments::*;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::language::adapter::{Refusal, Refused};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::matcher::{Arguments, Passed};
                #[allow(unused_imports)]
                use ast_grep_core::{Doc, Node};
                #[allow(unused_imports)]
                use std::borrow::Cow;
            }
            #[cfg_attr(not(test), allow(unused_imports))]
            pub use rerail_lang_php::code_rule_dsl::language::adapter::blade;
            pub mod calls {
                pub use rerail_workspace::code_rule_dsl::language::adapter::calls::*;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::language::adapter::{Refusal, Refused};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::calls::closure::{Graph, Link};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::calls::names::Name;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::calls::summary::{Callee, SHADOWED, Summary};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::calls::{FileTables, php};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::language::{Family, GrammarDialect};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::module_path::ModulePath;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::scope::ScopeTable;
                #[allow(unused_imports)]
                use ast_grep_core::{Doc, Node};
                #[allow(unused_imports)]
                use ast_grep_language::LanguageExt;
                #[allow(unused_imports)]
                use std::borrow::Cow;
                #[allow(unused_imports)]
                use std::sync::Arc;
            }
            pub use rerail_lang_ecma::code_rule_dsl::language::adapter::ecma;
            pub mod format_args {
                pub use rerail_workspace::code_rule_dsl::language::adapter::format_args::*;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::language::adapter::{Refusal, Refused};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::literal::Grammar;
                #[allow(unused_imports)]
                use std::borrow::Cow;
            }
            // Java's and C#'s families: syntax only so far, their tapes from the lean parsers.
            #[cfg_attr(not(test), allow(unused_imports))]
            pub mod csharp;
            #[cfg_attr(not(test), allow(unused_imports))]
            pub mod java;
            // Go's, C's and C++'s families: syntax only so far, their tapes from the lean parsers.
            #[cfg_attr(not(test), allow(unused_imports))]
            pub mod c;
            #[allow(unused_imports)] // boot's mount: only boot's files read it
            pub mod c_pp;
            #[cfg_attr(not(test), allow(unused_imports))]
            pub mod go;
            // Kotlin's, Swift's and Scala's families: syntax only so far, their tapes from the lean parsers.
            #[cfg_attr(not(test), allow(unused_imports))]
            pub mod kotlin;
            #[cfg_attr(not(test), allow(unused_imports))]
            pub mod scala;
            #[cfg_attr(not(test), allow(unused_imports))]
            pub mod swift;
            // HTML's, CSS's and Markdown's families: syntax only so far, read by tree-sitter.
            #[cfg_attr(not(test), allow(unused_imports))]
            pub mod css;
            #[cfg_attr(not(test), allow(unused_imports))]
            pub mod html;
            #[cfg_attr(not(test), allow(unused_imports))]
            pub mod markdown;
            #[cfg_attr(not(test), allow(unused_imports))]
            pub use rerail_lang_json::code_rule_dsl::language::adapter::json;
            #[cfg_attr(not(test), allow(unused_imports))]
            pub use rerail_lang_php::code_rule_dsl::language::adapter::php;
            pub use rerail_lang_python::code_rule_dsl::language::adapter::python;
            #[cfg_attr(not(test), allow(unused_imports))]
            pub use rerail_lang_rust::code_rule_dsl::language::adapter::rust;
            pub mod scope {
                pub use rerail_workspace::code_rule_dsl::language::adapter::scope::*;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::language::adapter::{Adapter, Refusal};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::scope::ScopeName;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::matcher::{Unresolved, file_module, file_source_type_for};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::module_path::ModulePath;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::scope::options::{
                    DEFAULT_PYTHON_MINOR, EsGlobals, LanguageOptions, PythonOptions,
                };
                #[allow(unused_imports)]
                use crate::code_rule_dsl::scope::flow::Defined;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::scope::usage::{Steps, Usage};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::scope::{
                    Access, Binding, BuildError, Builder, Directives, Form, Lookup, Reading, Reference, ScopeKind,
                    ScopeTable, ecma, php, python,
                };
                #[allow(unused_imports)]
                use crate::code_rule_dsl::value::NodeRef;
                #[allow(unused_imports)]
                use ast_grep_core::{Doc, Node};
                #[allow(unused_imports)]
                use std::borrow::Cow;
                #[allow(unused_imports)]
                use std::sync::Arc;
            }
            #[cfg_attr(not(test), allow(unused_imports))]
            pub use rerail_lang_sql::code_rule_dsl::language::adapter::sql;
            pub mod syntax {
                pub use rerail_workspace::code_rule_dsl::language::adapter::syntax::*;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::language::adapter::{Adapter, Refusal};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::compile::config::Options;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::compile::options;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::language::SourceType;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::literal::Grammar;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::scope::options::{LanguageOptions, PythonOptions};
                #[allow(unused_imports)]
                use ast_grep_core::{Doc, Node};
                #[allow(unused_imports)]
                use serde_json::Value;
                #[allow(unused_imports)]
                use std::borrow::Cow;
                #[allow(unused_imports)]
                use std::sync::Arc;
            }
            pub mod value {
                pub use rerail_workspace::code_rule_dsl::language::adapter::value::*;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::language::adapter::Refusal;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::language::Family;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::value::structure::{Equality, Tok};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::value::{
                    self, BigInt, Context, Ordered, TypeNames, Value, key, php_compare, php_structure,
                };
                #[allow(unused_imports)]
                use ast_grep_core::{Doc, Node};
                #[allow(unused_imports)]
                use std::cmp::Ordering;
            }
            pub mod vue {
                #[cfg_attr(not(test), allow(unused_imports))]
                pub use rerail_lang_ecma::code_rule_dsl::language::adapter::vue::*;
                #[allow(unused_imports)]
                use super::QueryWhen;
                #[allow(unused_imports)]
                use super::syntax::QueryOptions;
                #[allow(unused_imports)]
                use super::ecma::Ecma;
                #[allow(unused_imports)]
                use super::{Adapter, Refusal, Refused, SyntaxFeature, SyntaxRules};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::compile::config::Options;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::language::{Family, SourceType};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::lines::{LineTable, Terminators};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::matcher::{file_index};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::model::ViolationSpec;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::scope::options::{LanguageOptions, PythonOptions};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::suppress::Suppress;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::suppress::vue::{VueDirectives, script_directives as vue_script_directives};
                #[allow(unused_imports)]
                use ast_grep_core::{Doc, Node};
                #[allow(unused_imports)]
                use std::borrow::Cow;
                #[allow(unused_imports)]
                use std::ops::Range;
                #[allow(unused_imports)]
                use std::path::Path;
            }
            #[allow(unused_imports)]
            pub use crate::code_rule_dsl::language::adapter::arguments::{ArgumentFeature, ArgumentRules};
            #[allow(unused_imports)]
            pub use crate::code_rule_dsl::language::adapter::calls::{CallFeature, CallRules};
            #[allow(unused_imports)]
            pub use crate::code_rule_dsl::language::adapter::format_args::{FormatArgFeature, FormatArgRules};
            #[allow(unused_imports)]
            pub use crate::code_rule_dsl::language::adapter::scope::{ScopeFeature, ScopeRules};
            #[allow(unused_imports)]
            pub use crate::code_rule_dsl::language::adapter::syntax::{QueryWhen, SyntaxFeature, SyntaxRules};
            #[allow(unused_imports)]
            pub use crate::code_rule_dsl::language::adapter::value::{ValueFeature, ValueRules};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::{
                Family, GrammarDialect, PackageType, SourceType, is_declaration_file, language_not_supported,
            };
            #[allow(unused_imports)]
            use crate::code_rule_dsl::identifier::{ecma_name, python_name};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::invalid;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::lines::{LineTable, Terminators};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::{file_index};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::file_input::python_module_path_on_disk;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::suppress::{Suppress, root};
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use std::borrow::Cow;
            #[allow(unused_imports)]
            use std::ops::Range;
            #[allow(unused_imports)]
            use std::path::Path;
            #[allow(unused_imports)]
            use std::sync::LazyLock;
            #[allow(unused_imports)]
            pub use crate::code_rule_dsl::language::{ast_grep, parse_input};
        }
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use crate::code_rule_dsl::tree::Tree;
        #[allow(unused_imports)]
        use rerail_rr_syntax::tape as rr_tree;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::module_path::ModulePath;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::semver::Range;
        #[allow(unused_imports)]
        use ast_grep_core::{
            AstGrep,
            language::Language,
            matcher::{Pattern, PatternBuilder, PatternError},
            tree_sitter::{StrDoc, TSLanguage},
        };
        #[allow(unused_imports)]
        use ast_grep_language::{LanguageExt, SupportLang};
        #[allow(unused_imports)]
        use std::borrow::Cow;
        #[allow(unused_imports)]
        use std::sync::LazyLock;
    }
    pub mod lines {
        pub use rerail_lang::code_rule_dsl::lines::*;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::Family;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::GrammarDialect;
        #[allow(unused_imports)]
        use serde::{Deserialize, Serialize};
    }
    pub mod literal {
        pub mod offsets {
            pub use rerail_workspace::code_rule_dsl::literal::offsets::*;
            #[allow(unused_imports)]
            use std::ops::Range;
        }
        pub use rerail_workspace::code_rule_dsl::literal::*;
        pub mod annotation {
            #[cfg_attr(not(test), allow(unused_imports))]
            pub use rerail_lang_python::code_rule_dsl::literal::annotation::*;
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
            #[cfg_attr(not(test), allow(unused_imports))]
            pub use rerail_lang_ecma::code_rule_dsl::literal::eslint_directive::*;
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
            #[cfg_attr(not(test), allow(unused_imports))]
            pub use rerail_lang_php::code_rule_dsl::literal::phpdoc::*;
            #[allow(unused_imports)]
            use super::{Grammar, PNode};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::value::Value;
        }
        #[allow(unused_imports)] // boot's mount: only boot's files read it
        pub use rerail_lang_php::code_rule_dsl::literal::printf;
        #[allow(unused_imports)] // boot's mount: only boot's files read it
        pub use rerail_lang_python::code_rule_dsl::literal::python_fstring;
        #[cfg(any(test, feature = "test-hooks"))]
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
    }
    pub mod logic {
        pub use rerail_logic::code_rule_dsl::logic::*;
        // The converters from other tools' rules to `.rr` (the CLI's `convert`).
        pub mod convert;
        // Tests that read what the engine registers as it loads (the languages, their `.rerail` libraries) run here.
        pub mod program_image {
            // Its items, beside its tests here; the engine reads it through `logic`'s own re-export.
            #[allow(unused_imports)]
            pub use rerail_logic::code_rule_dsl::logic::program_image::*;
        }
        pub mod program_text {
            // Its items, beside its tests here; the engine reads it through `logic`'s own re-export.
            #[allow(unused_imports)]
            pub use rerail_logic::code_rule_dsl::logic::program_text::*;
        }
        // Tests that read what the engine registers as it loads (the languages, their `.rerail` libraries) run here.
        pub mod liveness {
            #[cfg(any(test, feature = "test-hooks"))]
            pub use rerail_logic::code_rule_dsl::logic::liveness::*;
        }
        // The cost statistics' read of each lane and form (3a42aefcf).
        // The JSON translator's tests (decision 0175) run in the engine's test binary.
        pub mod json_rules {
            pub use rerail_logic::code_rule_dsl::logic::json_rules::*;
        }
        pub mod front {
            pub use rerail_rr::code_rule_dsl::logic::front::*;
            pub mod caches {
                #[allow(unused_imports)]
                use crate::code_rule_dsl::logic::front::Error;
                #[allow(unused_imports)]
                use std::collections::BTreeSet;
            }
            pub mod yaml {
                #[allow(unused_imports)] // boot's mount: only boot's files read it
                pub use rerail_rr::code_rule_dsl::logic::front::yaml::*;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::logic::front::Error;
                #[allow(unused_imports)]
                use serde_json::Value;
            }
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, BTreeSet};
            #[allow(unused_imports)]
            use std::fmt;
            #[allow(unused_imports)]
            pub use crate::code_rule_dsl::logic::front::name::Name;
            #[allow(unused_imports)]
            pub use crate::code_rule_dsl::logic::front::ast::{AXIS, AggOp, DESCENDANTS, EVERY_KIND, GLOBAL, LIBRARY, LatticeDecl, NEAREST, Native, OPTION_GLOB, QUALIFIERS, RESERVED, SCOPE_NOT, SpanIn, Value, axis, globals, is_axis, is_transduced};
            #[allow(unused_imports)]
            pub use crate::code_rule_dsl::facts::core::PYTHON_CONTEXTS;
        }
        pub mod native {
            pub use rerail_logic::code_rule_dsl::logic::native::*;
            pub mod reach {
                #[cfg(any(test, feature = "test-hooks"))]
                pub use rerail_logic::code_rule_dsl::logic::native::reach::*;
            }
            pub mod setfix {
                pub use rerail_logic::code_rule_dsl::logic::native::setfix::*;
                // The lab's tests run strata on the Metal device (the transducer's), which the engine alone sees.
                pub mod lab {
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
                pub use crate::code_rule_dsl::logic::device::{Report as DeviceReport, take_report as device_report};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::logic::device::{JoinPlan};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::logic::profile::{Unit, work as charge};
            }
            #[allow(unused_imports)]
            pub use crate::code_rule_dsl::logic::native::setfix::{forget_engine, forget_scope};
            #[allow(unused_imports)]
            pub use crate::code_rule_dsl::logic::native::setfix::{flush_symbols, forget_inputs, intern_batch, keep_inputs, register_symbols, sym_text_of};
            #[allow(unused_imports)]
            pub use crate::code_rule_dsl::logic::native::setfix::symbol_text;
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
        pub mod plan {
            pub use rerail_logic::code_rule_dsl::logic::plan::*;
            // Tests that read what the engine registers as it loads (the languages, their `.rerail` libraries) run here.
            pub mod compose {
            }
            // Tests that read what the engine registers as it loads (the languages, their `.rerail` libraries) run here.
            pub mod native_demand {
            }
            // Only its tests are files of their own.
            pub mod corpus {
                pub use rerail_logic::code_rule_dsl::logic::plan::corpus::*;
                // The base tests compile and evaluate rule documents through the engine (`test_support`).
                // Decision 0225 §2: partial memo hits hand on what a full evaluation does.
                // Decision 0364's required shape (R1–R10): what a session's edit does, one test per requirement.
                // Decision 0458 (R1–R5): the cross part reads the base relations off every file's tape.
                // Decision 0494: a batch makes a document when first read.
                // Tests that read what the engine registers as it loads (the languages, their `.rerail` libraries) run here.
                pub mod phase0 {
                    pub use rerail_logic::code_rule_dsl::logic::plan::corpus::phase0::*;
                }
                // Tests that read what the engine registers as it loads (the languages, their `.rerail` libraries) run here.
                pub mod phases {
                    pub use rerail_logic::code_rule_dsl::logic::plan::corpus::phases::*;
                }
                // Tests that read what the engine registers as it loads (the languages, their `.rerail` libraries) run here.
                pub mod prepared {
                    #[cfg(any(test, feature = "test-hooks"))]
                    pub use rerail_logic::code_rule_dsl::logic::plan::corpus::prepared::*;
                }
                // Tests that read what the engine registers as it loads (the languages, their `.rerail` libraries) run here.
                pub mod reuse {
                    #[cfg(any(test, feature = "test-hooks"))]
                    pub use rerail_logic::code_rule_dsl::logic::plan::corpus::reuse::*;
                }
                // Tests that read what the engine registers as it loads (the languages, their `.rerail` libraries) run here.
                pub mod session {
                    pub use rerail_logic::code_rule_dsl::logic::plan::corpus::session::*;
                }
                // The corpus runner's tests: they parse files, so they run where the languages are registered.
                // Markdown's inline guests against tree-sitter-md's inline trees: they parse files.
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
                pub use crate::code_rule_dsl::logic::plan::corpus::hiding::qualified;
                #[allow(unused_imports)]
                pub use crate::code_rule_dsl::logic::plan::corpus::inputs::parameters as input_parameters;
                #[allow(unused_imports)]
                pub use crate::code_rule_dsl::logic::plan::corpus::matchers::columns as ast_columns;
                #[allow(unused_imports)]
                pub use crate::code_rule_dsl::logic::plan::corpus::typing::{Typing, TypingReport};
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
                pub use crate::code_rule_dsl::prof::thread_usage;
                #[allow(unused_imports)]
                use std::collections::{BTreeMap, BTreeSet};
            }
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
        pub mod profile {
            pub use rerail_prof::code_rule_dsl::logic::profile::*;
            // The profile's account of every thread's CPU, under its spans and between them (3af4e92a3, 2f9181e59).
            // Tests that read what the engine registers as it loads (the languages, their `.rerail` libraries) run here.
            pub mod report {
                pub use rerail_logic::code_rule_dsl::logic::profile::report::*;
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
            pub use crate::code_rule_dsl::logic::profile::roofline::Unit;
        }
        pub mod rr {
            pub use rerail_logic::code_rule_dsl::logic::rr::*;
            pub mod tape_type {
                #[allow(unused_imports)]
                use std::collections::{BTreeMap, BTreeSet};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::logic::front::Error;
            }
            #[allow(unused_imports)]
            use rerail_rr_syntax::syntax;
            #[allow(unused_imports)]
            pub use rerail_rr_syntax::tape;
            #[allow(unused_imports)]
            use std::collections::BTreeSet;
            #[allow(unused_imports)]
            use std::path::{Path, PathBuf};
            #[allow(unused_imports)]
            pub use crate::code_rule_dsl::logic::front::Error;
            #[allow(unused_imports)]
            #[cfg(any(test, feature = "test-hooks"))]
            pub use crate::code_rule_dsl::logic::rr::lower::drop_unread_core;
            #[allow(unused_imports)]
            pub use crate::code_rule_dsl::logic::rr::lower::is_core;
            #[allow(unused_imports)]
            pub use crate::code_rule_dsl::logic::rr::lower::WORLD;
            #[allow(unused_imports)]
            pub use syntax::quote;
            #[allow(unused_imports)]
            pub use crate::code_rule_dsl::logic::rr::translate::translate;
            #[allow(unused_imports)]
            pub use crate::code_rule_dsl::facts::vtab::syntax::core_rr::single_metavars;
            // The rule tests' runner (`.rr` cases), which the CLI's tools and the tests read.
            pub mod testing;
        }
        pub mod strings {
            pub use rerail_facts::code_rule_dsl::logic::strings::*;
            // `to_precision` is the ECMA crate's (`TO_PRECISION`), registered as the engine loads.
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
    /// The profile's SQL hazards, which the content store's connections and writer report (`content_store::hazards`, set as
    /// the library loads; decision 0104).
    pub mod sql_hazards {
        #[allow(unused_imports)] // boot's mount: only boot's files read it
        pub use super::logic::profile::strict_on_every_connection;
    }
    pub mod sql_executor {
        // Only in builds that have them in rerail-sessions (its own `cfg`): its tests, `test-hooks`, `headless`.
        #[cfg(any(test, feature = "test-hooks", feature = "headless"))]
        pub mod verification {
            #[allow(unused_imports)]
            use super::*;
        }
        pub mod versions {
            pub use rerail_sessions::code_rule_dsl::sql_executor::versions::*;
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
        }
        pub mod work_graph {
            #[cfg(any(test, feature = "test-hooks"))]
            pub use rerail_sessions::code_rule_dsl::sql_executor::work_graph::*;
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
            pub mod cold_loop {
                #[cfg(any(test, feature = "test-hooks"))]
                pub use rerail_sessions::code_rule_dsl::sql_executor::temporal::cold_loop::*;
                #[allow(unused_imports)]
                use super::*;
            }
            pub mod batch {
                pub use rerail_sessions::code_rule_dsl::sql_executor::temporal::batch::*;
                #[allow(unused_imports)]
                use super::*;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::facts::vtab::Modules;
                #[allow(unused_imports)]
                use rusqlite::functions::{Aggregate, Context, FunctionFlags};
                #[allow(unused_imports)]
                use state::{Arena, Wave};
            }
            pub use rerail_sessions::code_rule_dsl::sql_executor::temporal::*;
            #[allow(unused_imports)]
            use super::*;
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
            // Read by the tests alone: a headless build's comparison is rerail-sessions' own (`verification`).
            // The test crates read it through `test-hooks`.
            #[cfg(any(test, feature = "test-hooks"))]
            pub use rerail_sessions::code_rule_dsl::sql_executor::component_loop::*;
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
        pub use rerail_sessions::code_rule_dsl::sql_executor::*;
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
    pub mod matcher {
        pub mod value {
            pub use rerail_workspace::code_rule_dsl::matcher::value::*;
            #[allow(unused_imports)]
            use super::resolve::{ ValueKey, binding_value, constant_value, followed_table, memoize_structure, memoize_value, memoized_structure, memoized_value, unshadowed_global, };
            #[allow(unused_imports)]
            use super::util::{UTIL_EXCEEDED, UTIL_STEPS_LEFT, UtilLimit};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::budget;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::{Family, GrammarDialect, kind_ids};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::literal;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::scope::options::{LanguageOptions, PythonOptions};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::value::structure::{Equality, Tok};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::value::text_regex::TextRegex;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::value::{BigInt, Context, Value};
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use num_traits::Zero;
            #[allow(unused_imports)]
            use std::cmp::Ordering;
            #[allow(unused_imports)]
            use std::collections::BTreeSet;
            #[allow(unused_imports)]
            use std::rc::Rc;
            #[allow(unused_imports)]
            use std::sync::Arc;
            #[allow(unused_imports)]
            use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::pack as language_pack;
        }
        pub mod embedded {
            pub use rerail_workspace::code_rule_dsl::matcher::embedded::*;
            #[allow(unused_imports)]
            use super::AstMatcher;
            #[allow(unused_imports)]
            use super::each::{self, HostRanges};
            #[allow(unused_imports)]
            use super::exported::{export, exported_span, exported_text, is_span_variable};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::embed::segments::holds;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::{GrammarDialect, Via};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::origin::Origin;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::tape::{TapeDoc, tape_doc};
            #[allow(unused_imports)]
            use ast_grep_core::meta_var::{MetaVarEnv, MetaVariable};
            #[allow(unused_imports)]
            use ast_grep_core::{AstGrep, Doc, Matcher, Node};
            #[allow(unused_imports)]
            use std::borrow::Cow;
            #[allow(unused_imports)]
            use std::cell::RefCell;
            #[allow(unused_imports)]
            use std::ops::Range;
            #[allow(unused_imports)]
            use std::sync::Arc;
        }
        pub mod util {
            pub use rerail_workspace::code_rule_dsl::matcher::util::*;
            #[allow(unused_imports)]
            use super::AstMatcher;
            #[allow(unused_imports)]
            use std::cell::Cell;
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, BTreeSet};
            #[allow(unused_imports)]
            use std::sync::atomic::{AtomicBool, Ordering};
            #[allow(unused_imports)]
            use std::sync::{Arc, Mutex, OnceLock, Weak};
        }
        pub mod transform {
            pub use rerail_workspace::code_rule_dsl::matcher::transform::*;
            #[allow(unused_imports)]
            use super::exported::exported_span;
            #[allow(unused_imports)]
            use super::value::Reading;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::origin::InFile;
            #[allow(unused_imports)]
            use ast_grep_core::meta_var::{MetaVarEnv, MetaVariable};
            #[allow(unused_imports)]
            use ast_grep_core::source::Content;
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use regex::Regex;
            #[allow(unused_imports)]
            use semantic::value_text;
            #[allow(unused_imports)]
            #[cfg(any(test, feature = "test-hooks"))]
            use std::cell::Cell;
            #[allow(unused_imports)]
            use std::collections::BTreeSet;
            #[allow(unused_imports)]
            use std::ops::Range;
        }
        pub mod relation {
            pub use rerail_workspace::code_rule_dsl::matcher::relation::*;
            #[allow(unused_imports)]
            use super::AstMatcher;
            #[allow(unused_imports)]
            use super::CaptureAnalysis;
            #[allow(unused_imports)]
            use super::RequiredText;
            #[allow(unused_imports)]
            use super::Semantic;
            #[allow(unused_imports)]
            use super::memo::Truth;
            #[allow(unused_imports)]
            use super::util::{THREE_VALUED, UNKNOWN};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::budget;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::tape::TapeDoc;
            #[allow(unused_imports)]
            use ast_grep_core::matcher::{Matcher, MatcherExt, PatternNode};
            #[allow(unused_imports)]
            use ast_grep_core::meta_var::{MetaVarEnv, MetaVariable};
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use std::borrow::Cow;
            #[allow(unused_imports)]
            use std::cell::Cell;
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, BTreeSet};
            #[allow(unused_imports)]
            use std::ops::ControlFlow;
            #[allow(unused_imports)]
            use std::sync::{Arc, OnceLock};
        }
        pub mod nth_child {
            pub use rerail_workspace::code_rule_dsl::matcher::nth_child::*;
            #[allow(unused_imports)]
            use super::AstMatcher;
            #[allow(unused_imports)]
            use super::CaptureAnalysis;
            #[allow(unused_imports)]
            use super::spend_util_step;
            #[allow(unused_imports)]
            use super::util::{THREE_VALUED, UNKNOWN};
            #[allow(unused_imports)]
            use ast_grep_core::Matcher;
            #[allow(unused_imports)]
            use ast_grep_core::meta_var::MetaVarEnv;
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use std::borrow::Cow;
        }
        pub mod memo {
            pub use rerail_workspace::code_rule_dsl::matcher::memo::*;
            #[allow(unused_imports)]
            use super::nth_child::Siblings;
            #[allow(unused_imports)]
            use super::relation::WalkFilter;
            #[allow(unused_imports)]
            use super::util::{THREE_VALUED, UNKNOWN, UtilCall, UtilRef};
            #[allow(unused_imports)]
            use super::{Comparison, EqClass};
            #[allow(unused_imports)]
            use super::{ clause_state_reads, clauses_begun, each, fill_steps, filling, read_clause_state, spend_util_steps, swap_util_peak, util_level, util_levels_fit, util_limit_hit, utility_steps_left, };
            #[allow(unused_imports)]
            use crate::code_rule_dsl::budget;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::tape::TapeDoc;
            #[allow(unused_imports)]
            use ast_grep_core::meta_var::{MetaVarEnv, MetaVariable};
            #[allow(unused_imports)]
            use ast_grep_core::{AstGrep as Root, Doc, Node};
            #[allow(unused_imports)]
            use std::any::TypeId;
            #[allow(unused_imports)]
            use std::borrow::Cow;
            #[allow(unused_imports)]
            use std::cell::RefCell;
            #[allow(unused_imports)]
            use std::ops::Range;
            #[allow(unused_imports)]
            use std::sync::Arc;
        }
        pub mod layout {
            pub use rerail_workspace::code_rule_dsl::matcher::layout::*;
            #[allow(unused_imports)]
            use super::each::binding_range;
            #[allow(unused_imports)]
            use super::spend_util_step;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::origin::InFile;
            #[allow(unused_imports)]
            use ast_grep_core::meta_var::MetaVarEnv;
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use std::borrow::Cow;
            #[allow(unused_imports)]
            use std::ops::Range;
        }
        pub mod each {
            pub use rerail_workspace::code_rule_dsl::matcher::each::*;
            #[allow(unused_imports)]
            use super::exported::exported_span;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::origin::Origin;
            #[allow(unused_imports)]
            use ast_grep_core::Doc;
            #[allow(unused_imports)]
            use ast_grep_core::meta_var::MetaVarEnv;
            #[allow(unused_imports)]
            use std::cell::{Cell, RefCell};
            #[allow(unused_imports)]
            use std::collections::HashSet;
            #[allow(unused_imports)]
            use std::ops::Range;
            #[allow(unused_imports)]
            use std::sync::Arc;
        }
        pub mod count {
            pub use rerail_workspace::code_rule_dsl::matcher::count::*;
            #[allow(unused_imports)]
            use super::AstMatcher;
            #[allow(unused_imports)]
            use super::relation::descend_until;
            #[allow(unused_imports)]
            use super::util::{THREE_VALUED, UNKNOWN, UTIL_EXCEEDED};
            #[allow(unused_imports)]
            use super::{StopBy, spend_util_step};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::budget;
            #[allow(unused_imports)]
            use ast_grep_core::matcher::Matcher;
            #[allow(unused_imports)]
            use ast_grep_core::meta_var::{MetaVarEnv, MetaVariable};
            #[allow(unused_imports)]
            use ast_grep_core::source::Content;
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use std::borrow::Cow;
            #[allow(unused_imports)]
            use std::cell::Cell;
            #[allow(unused_imports)]
            use std::ops::ControlFlow;
        }
        pub mod compare {
            pub use rerail_workspace::code_rule_dsl::matcher::compare::*;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::tree::Tree;
            #[allow(unused_imports)]
            use super::collate::natural_compare;
            #[allow(unused_imports)]
            use super::exported::exported_text;
            #[allow(unused_imports)]
            use super::transform::Transform;
            #[allow(unused_imports)]
            use super::value::Reading;
            #[allow(unused_imports)]
            use ast_grep_core::meta_var::MetaVarEnv;
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use std::cmp::Ordering;
            #[allow(unused_imports)]
            use std::sync::Arc;
        }
        pub mod call {
            #[allow(unused_imports)]
            use super::{UtilCall, each, spend_util_step, unify_capture};
            #[allow(unused_imports)]
            use ast_grep_core::meta_var::MetaVarEnv;
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use std::collections::BTreeSet;
        }
        pub mod analysis {
            pub mod required_text {
                #[cfg(any(test, feature = "test-hooks"))]
                pub use rerail_workspace::code_rule_dsl::matcher::analysis::required_text::*;
                #[allow(unused_imports)]
                use super::super::{AstMatcher, UtilRef};
                #[allow(unused_imports)]
                use ast_grep_core::MatchStrictness;
                #[allow(unused_imports)]
                use ast_grep_core::matcher::PatternNode;
                #[allow(unused_imports)]
                use regex_syntax::hir::literal::Extractor;
                #[allow(unused_imports)]
                use std::collections::{BTreeMap, BTreeSet};
            }
            pub use rerail_workspace::code_rule_dsl::matcher::analysis::*;
        }
        pub use rerail_workspace::code_rule_dsl::matcher::*;
        pub mod anchors {
            pub use rerail_workspace::code_rule_dsl::matcher::anchors::*;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::{AstMatcher, StopBy, UtilRef};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::budget;
            #[allow(unused_imports)]
            #[cfg(any(test, feature = "test-hooks"))]
            use crate::code_rule_dsl::walks::{Walk, record_walk};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::GrammarDialect;
            #[allow(unused_imports)]
            use ast_grep_core::matcher::{Matcher, PatternNode, kind_utils};
            #[allow(unused_imports)]
            use ast_grep_core::source::SgNode;
            #[allow(unused_imports)]
            use ast_grep_core::{AstGrep, Doc, MatchStrictness, Node};
            #[allow(unused_imports)]
            use foldhash::{HashMap, HashMapExt, HashSet};
            #[allow(unused_imports)]
            use regex::Regex;
            #[allow(unused_imports)]
            use std::borrow::Cow;
            #[allow(unused_imports)]
            use std::cell::RefCell;
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, BTreeSet};
            #[allow(unused_imports)]
            use std::num::NonZeroUsize;
            #[allow(unused_imports)]
            use std::sync::Arc;
            #[allow(unused_imports)]
            use rerail_store::probes::TextProbe;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::pack as language_pack;
        }
        pub mod arguments {
            pub use rerail_workspace::code_rule_dsl::matcher::arguments::*;
            #[cfg(any(test, feature = "test-hooks"))]
            pub use rerail_lang_php::code_rule_dsl::matcher::arguments::lang_php;
            #[cfg(any(test, feature = "test-hooks"))]
            pub use rerail_lang_python::code_rule_dsl::matcher::arguments::lang_python;
            #[cfg(any(test, feature = "test-hooks"))]
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
        }
        #[cfg(any(test, feature = "test-hooks"))]
        pub mod d_anchors {
            pub mod sql {
                pub mod lower {
                    pub use rerail_workspace::code_rule_dsl::matcher::d_anchors::sql::lower::*;
                    #[allow(unused_imports)]
                    use super::super::{AnchorPlan, Condition, Direction, Required};
                    #[allow(unused_imports)]
                    use rusqlite::types::Value;
                    #[allow(unused_imports)]
                    use std::collections::{BTreeSet, HashMap};
                    #[allow(unused_imports)]
                    use decode::Property;
                    #[allow(unused_imports)]
                    use render::Definitions;
                }
                pub mod corpus {
                    pub use rerail_workspace::code_rule_dsl::matcher::d_anchors::sql::corpus::*;
                    #[allow(unused_imports)]
                    use super::super::AnchorPlan;
                    #[allow(unused_imports)]
                    use super::{ candidates::{Builder, Roots}, store::Store, };
                    #[allow(unused_imports)]
                    use crate::code_rule_dsl::tape::TapeDoc;
                    #[allow(unused_imports)]
                    use ast_grep_core::Doc;
                    #[allow(unused_imports)]
                    use parking_lot::Mutex;
                    #[allow(unused_imports)]
                    use rusqlite::Result;
                    #[allow(unused_imports)]
                    use std::collections::{BTreeMap, HashMap, HashSet};
                    #[allow(unused_imports)]
                    use std::{cell::RefCell, sync::Arc};
                }
                pub mod statistics {
                    pub use rerail_workspace::code_rule_dsl::matcher::d_anchors::sql::statistics::*;
                    #[allow(unused_imports)]
                    use super::super::{AnchorPlan, Condition, Required, Requirement};
                    #[allow(unused_imports)]
                    use rusqlite::{Connection, Result, limits::Limit, params_from_iter, types::Value};
                    #[allow(unused_imports)]
                    use std::collections::{BTreeMap, BTreeSet, HashSet};
                    #[allow(unused_imports)]
                    use std::sync::Arc;
                }
                pub use rerail_workspace::code_rule_dsl::matcher::d_anchors::sql::*;
                #[allow(unused_imports)]
                use super::AnchorPlan;
                #[allow(unused_imports)]
                #[cfg(any(test, feature = "test-hooks"))]
                use super::Selection;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::{language, tape::TapeDoc};
                #[allow(unused_imports)]
                #[cfg(any(test, feature = "test-hooks"))]
                use ast_grep_core::Node;
                #[allow(unused_imports)]
                use ast_grep_core::{AstGrep, Doc};
                #[allow(unused_imports)]
                #[cfg(any(test, feature = "test-hooks"))]
                use std::borrow::Cow;
            }
            pub use rerail_workspace::code_rule_dsl::matcher::d_anchors::*;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::{AstMatcher, StopBy, UtilRef};
            #[allow(unused_imports)]
            #[cfg(any(test, feature = "test-hooks"))]
            use crate::code_rule_dsl::budget;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::{Adapter, ValueRules};
            #[allow(unused_imports)]
            #[cfg(any(test, feature = "test-hooks"))]
            use crate::code_rule_dsl::walks::{Walk, record_walk};
            #[allow(unused_imports)]
            #[cfg(any(test, feature = "test-hooks"))]
            use crate::code_rule_dsl::language::GrammarDialect;
            #[allow(unused_imports)]
            use ast_grep_core::MatchStrictness;
            #[allow(unused_imports)]
            use ast_grep_core::matcher::{Matcher, PatternNode, kind_utils};
            #[allow(unused_imports)]
            #[cfg(any(test, feature = "test-hooks"))]
            use ast_grep_core::source::SgNode;
            #[allow(unused_imports)]
            #[cfg(any(test, feature = "test-hooks"))]
            use ast_grep_core::{AstGrep, Doc, Node};
            #[allow(unused_imports)]
            #[cfg(any(test, feature = "test-hooks"))]
            use foldhash::{HashMap, HashMapExt, HashSet};
            #[allow(unused_imports)]
            use regex::Regex;
            #[allow(unused_imports)]
            #[cfg(any(test, feature = "test-hooks"))]
            use std::borrow::Cow;
            #[allow(unused_imports)]
            #[cfg(any(test, feature = "test-hooks"))]
            use std::cell::RefCell;
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, BTreeSet};
            #[allow(unused_imports)]
            #[cfg(any(test, feature = "test-hooks"))]
            use std::num::NonZeroUsize;
            #[allow(unused_imports)]
            use std::sync::Arc;
            #[allow(unused_imports)]
            #[cfg(any(test, feature = "test-hooks"))]
            use crate::code_rule_dsl::matcher::d_anchors::text::TextProbe;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::pack as language_pack;
        }
        pub mod format_args {
            pub use rerail_workspace::code_rule_dsl::matcher::format_args::*;
            #[allow(unused_imports)] // boot's mount: only boot's files read it
            pub use rerail_lang_php::code_rule_dsl::matcher::format_args::printf;
            #[allow(unused_imports)] // boot's mount: only boot's files read it
            pub use rerail_lang_python::code_rule_dsl::matcher::format_args::python;
            #[allow(unused_imports)] // boot's mount: only boot's files read it
            pub use rerail_lang_rust::code_rule_dsl::matcher::format_args::rust;
            #[allow(unused_imports)] // boot's mount: only boot's files read it
            pub use rerail_lang_php::code_rule_dsl::matcher::format_args::scanf;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::tree::Tree;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::AstMatcher;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::each::{self};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::parse::parsed_tree;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::util::{UNKNOWN};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::value::Reading;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::literal::{Grammar, Parsed, SubDoc, offsets};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::origin::Origin;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::transducer::semantic_facts::{Facts, Target, slot};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::value::Value;
            #[allow(unused_imports)]
            use ast_grep_core::Matcher;
            #[allow(unused_imports)]
            use ast_grep_core::meta_var::MetaVarEnv;
            #[allow(unused_imports)]
            use ast_grep_core::{AstGrep, Doc, Node};
            #[allow(unused_imports)]
            use std::borrow::Cow;
            #[allow(unused_imports)]
            use std::collections::BTreeSet;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::GrammarDialect;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::tape::TapeDoc;
            #[allow(unused_imports)]
            use ast_grep_core::tree_sitter::StrDoc;
        }
        #[cfg(any(test, feature = "test-hooks"))]
        pub mod p_anchors {
            pub use rerail_workspace::code_rule_dsl::matcher::p_anchors::*;
            pub mod sql {
                pub mod lower {
                    pub use rerail_workspace::code_rule_dsl::matcher::p_anchors::sql::lower::*;
                    #[allow(unused_imports)]
                    use super::super::{AnchorPlan, Condition, Direction, Required};
                    #[allow(unused_imports)]
                    use rusqlite::types::Value;
                    #[allow(unused_imports)]
                    use std::collections::{BTreeSet, HashMap};
                    #[allow(unused_imports)]
                    use decode::Property;
                    #[allow(unused_imports)]
                    use render::Definitions;
                }
                pub use rerail_workspace::code_rule_dsl::matcher::p_anchors::sql::*;
                #[allow(unused_imports)]
                pub use crate::code_rule_dsl::matcher::p_anchors::sql::audit::candidate_frontier_audit;
                #[allow(unused_imports)]
                pub use crate::code_rule_dsl::matcher::p_anchors::sql::corpus::{Corpus, CorpusSlot};
                #[allow(unused_imports)]
                pub use crate::code_rule_dsl::matcher::p_anchors::sql::plans::{
                    for_file as anchor_plans_for_file, plans as anchor_plans,
                };
                #[allow(unused_imports)]
                use crate::code_rule_dsl::matcher::p_anchors::AnchorPlan;
                #[allow(unused_imports)]
                #[cfg(any(test, feature = "test-hooks"))]
                use crate::code_rule_dsl::matcher::p_anchors::Selection;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::{language, tape::TapeDoc};
                #[allow(unused_imports)]
                #[cfg(any(test, feature = "test-hooks"))]
                use ast_grep_core::Node;
                #[allow(unused_imports)]
                use ast_grep_core::{AstGrep, Doc};
                #[allow(unused_imports)]
                #[cfg(any(test, feature = "test-hooks"))]
                use std::borrow::Cow;
            }
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::{AstMatcher, StopBy, UtilRef};
            #[allow(unused_imports)]
            #[cfg(any(test, feature = "test-hooks"))]
            use crate::code_rule_dsl::budget;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::{Adapter, ValueRules};
            #[allow(unused_imports)]
            #[cfg(any(test, feature = "test-hooks"))]
            use crate::code_rule_dsl::walks::{Walk, record_walk};
            #[allow(unused_imports)]
            #[cfg(any(test, feature = "test-hooks"))]
            use crate::code_rule_dsl::language::GrammarDialect;
            #[allow(unused_imports)]
            use ast_grep_core::MatchStrictness;
            #[allow(unused_imports)]
            use ast_grep_core::matcher::{Matcher, PatternNode, kind_utils};
            #[allow(unused_imports)]
            #[cfg(any(test, feature = "test-hooks"))]
            use ast_grep_core::source::SgNode;
            #[allow(unused_imports)]
            #[cfg(any(test, feature = "test-hooks"))]
            use ast_grep_core::{AstGrep, Doc, Node};
            #[allow(unused_imports)]
            #[cfg(any(test, feature = "test-hooks"))]
            use foldhash::{HashMap, HashMapExt, HashSet};
            #[allow(unused_imports)]
            use regex::Regex;
            #[allow(unused_imports)]
            #[cfg(any(test, feature = "test-hooks"))]
            use std::borrow::Cow;
            #[allow(unused_imports)]
            #[cfg(any(test, feature = "test-hooks"))]
            use std::cell::RefCell;
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, BTreeSet};
            #[allow(unused_imports)]
            #[cfg(any(test, feature = "test-hooks"))]
            use std::num::NonZeroUsize;
            #[allow(unused_imports)]
            use std::sync::Arc;
            #[allow(unused_imports)]
            #[cfg(any(test, feature = "test-hooks"))]
            use crate::code_rule_dsl::matcher::p_anchors::text::TextProbe;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::pack as language_pack;
        }
        #[cfg(any(test, feature = "test-hooks"))]
        pub mod python_facts {
            pub use rerail_lang_python::code_rule_dsl::matcher::python_facts::*;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::{ identifier::python_identifier, language::{ Family, GrammarDialect, adapter::{python::Python, ScopeRules}, }, module_path::ModulePath, scope::{ Binding, Form, Import, Lookup, Origin, ScopeTable, options::{PythonOptions, python_builtins}, }, value::NodeRef, };
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, HashSet};
        }
        pub mod resolve {
            pub use rerail_workspace::code_rule_dsl::matcher::resolve::*;
            pub use rerail_lang_php::code_rule_dsl::matcher::resolve::php;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::tree::Tree;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::GrammarMatcher;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::util::{UNKNOWN, UtilLimit};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::budget;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::identifier::{python_identifier};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::{Family, GrammarDialect};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::{
                JsxRuntime, SourceType, has_module_syntax, is_stub_file, jsx_runtime_in_source,
            };
            #[allow(unused_imports)]
            use crate::code_rule_dsl::lines::{LineTable, Terminators};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::literal::{Grammar, SubTree};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::model::FileInput;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::module_path::ModulePath;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::scope::options::{DEFAULT_PYTHON_MINOR, LanguageOptions, PythonOptions};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::scope::usage::Usage;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::scope::{Binding, BuildError, Import, Reading, ScopeTable, TYPE, VALUE};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::suppress::RerailIgnores;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::value::NodeRef;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::value::structure::{Equality, Tok};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::value::{Followed, Value};
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use foldhash::{HashMap, HashMapExt};
            #[allow(unused_imports)]
            use std::cell::{Cell, OnceCell, RefCell};
            #[allow(unused_imports)]
            use std::collections::BTreeSet;
            #[allow(unused_imports)]
            use std::ops::Range;
            #[allow(unused_imports)]
            use std::rc::Rc;
            #[allow(unused_imports)]
            use std::sync::Arc;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::pack as language_pack;
        }
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        pub use crate::code_rule_dsl::matcher::p_anchors::sql::{Corpus, CorpusSlot};
        #[allow(unused_imports)]
        #[cfg(all(any(test, feature = "test-hooks"), feature = "p-anchor-corpus"))]
        pub use crate::code_rule_dsl::matcher::p_anchors::sql::{anchor_plans, anchor_plans_for_file};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::budget;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::{GrammarDialect, SourceType};
        #[allow(unused_imports)]
        pub use crate::code_rule_dsl::matcher::analysis::RequiredText;
        #[allow(unused_imports)]
        pub use crate::code_rule_dsl::matcher::lower::{Axis, Fact, FactFound, MatchTree, Stop, DocumentGate, gated_match_tree, match_tree};
        #[allow(unused_imports)]
        pub use crate::code_rule_dsl::matcher::analysis::{calls_positive_only, reads_import_targets};
        #[allow(unused_imports)]
        pub use crate::code_rule_dsl::matcher::anchors::{AnchorPlan, FileAnchorIndex, Selection};
        #[allow(unused_imports)]
        pub use crate::code_rule_dsl::matcher::anchors::{Condition, Direction, Required, Requirement};
        #[allow(unused_imports)]
        pub use crate::code_rule_dsl::matcher::arguments::{Arguments, Passed, SlotKind};
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
        pub use crate::code_rule_dsl::matcher::binding::BindingTest;
        #[allow(unused_imports)]
        pub use crate::code_rule_dsl::matcher::binds::{BindingCount, BindingFilter, Binds, Redefines, ScopeName};
        #[allow(unused_imports)]
        pub use crate::code_rule_dsl::matcher::captures::{Capture, CaptureAnalysis, collect_variables, parse_exported};
        #[allow(unused_imports)]
        pub use crate::code_rule_dsl::matcher::compare::{Collation, CompareOp, CompareText, Comparison, EqClass, SemanticText};
        #[allow(unused_imports)]
        pub use crate::code_rule_dsl::matcher::count::{Count, CountBinding};
        #[allow(unused_imports)]
        pub use crate::code_rule_dsl::matcher::each::binding_range;
        #[allow(unused_imports)]
        pub use crate::code_rule_dsl::matcher::embedded::EmbeddedTest;
        #[allow(unused_imports)]
        pub use crate::code_rule_dsl::matcher::exported::{
            WITNESS_NODE, export_text, exported_span, exported_text, is_span_variable,
        };
        #[allow(unused_imports)]
        pub use crate::code_rule_dsl::matcher::format_args::{FormatArgs, Style};
        #[allow(unused_imports)]
        pub use crate::code_rule_dsl::matcher::layout::{Gap, Layout, LineContext, LineCount, Neighbour};
        #[allow(unused_imports)]
        pub use crate::code_rule_dsl::matcher::memo::with_file as with_file_memo;
        #[allow(unused_imports)]
        pub use crate::code_rule_dsl::matcher::nth_child::{NthChild, Position as NthPosition, parse_position};
        #[allow(unused_imports)]
        pub use crate::code_rule_dsl::matcher::nth_child::SiblingPositions;
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        pub use crate::code_rule_dsl::matcher::nth_child::{named_sibling_visits, reset_named_sibling_visits};
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        pub use crate::code_rule_dsl::matcher::parse::parse_fact_tree;
        #[allow(unused_imports)]
        pub use crate::code_rule_dsl::matcher::parse::{HostTest, ParseTest};
        #[allow(unused_imports)]
        use regex::Regex;
        #[allow(unused_imports)]
        pub use crate::code_rule_dsl::matcher::relation::{FieldRelation, Related, StopBy, stop_rule};
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        pub use crate::code_rule_dsl::matcher::relation::{set_answers_off, set_same_key_off};
        #[allow(unused_imports)]
        pub use crate::code_rule_dsl::matcher::resolve::{
            FileGates, calls_tables, file_index, file_lines, file_module, file_rerail_ignores, file_source_type_for,
            table_reading,
        };
        #[allow(unused_imports)]
        pub use crate::code_rule_dsl::matcher::resolve::{FileScopeSlot, Unresolved, matched_file_path};
        #[allow(unused_imports)]
        pub use crate::code_rule_dsl::matcher::resolve::Resolves;
        #[allow(unused_imports)]
        pub use crate::code_rule_dsl::matcher::resolve::php as resolve_php;
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        pub use crate::code_rule_dsl::matcher::resolve::{JSX_RUNTIME_DERIVATIONS, SOURCE_TYPE_DERIVATIONS, TABLE_BUILDS};
        #[allow(unused_imports)]
        pub use crate::code_rule_dsl::matcher::run::{Stopped, enumerate, node_at, run_clause};
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        pub use crate::code_rule_dsl::matcher::run::{reset_roots_matched, roots_matched};
        #[allow(unused_imports)]
        pub use crate::code_rule_dsl::matcher::semantic::Semantic;
        #[allow(unused_imports)]
        use std::borrow::Cow;
        #[allow(unused_imports)]
        use std::collections::BTreeSet;
        #[allow(unused_imports)]
        use std::sync::Arc;
        #[allow(unused_imports)]
        pub use crate::code_rule_dsl::matcher::transform::{Digits, Separator, Source, StringCase, Trans, Transform};
        #[allow(unused_imports)]
        pub use crate::code_rule_dsl::matcher::transform::{NumberText, install_number_text, number_to_string};
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        pub use crate::code_rule_dsl::matcher::transform::{
            line_table_constructions, note_line_table_construction, reset_line_table_constructions,
        };
        #[allow(unused_imports)]
        pub use crate::code_rule_dsl::matcher::util::{
            CallArgs, MAX_UTIL_DEPTH, MAX_UTIL_INSTANCES, UtilCall, UtilLimit, UtilOrigin, UtilRef,
            UtilTable, UtilitySummary,
        };
        #[allow(unused_imports)]
        use crate::code_rule_dsl::matcher::util::{UNKNOWN};
        #[allow(unused_imports)]
        pub use crate::code_rule_dsl::matcher::value::{Bound, Reading, ValueTest};
        #[allow(unused_imports)]
        pub use crate::code_rule_dsl::matcher::value::Values;
        #[allow(unused_imports)]
        pub use crate::code_rule_dsl::matcher::term::{LanguageTerm, TermCompiler, install_terms};
        #[allow(unused_imports)]
        use util::{ MAX_UTIL_STACK_BYTES, UTIL_DEPTH, UTIL_EXCEEDED, UTIL_STACK_BASE, UTIL_STEPS_LEFT, stack_address, };
        #[allow(unused_imports)]
        use util::{THREE_VALUED};
    }
    pub use rerail_workspace::code_rule_dsl::model;
    pub mod module_graph {
        pub use rerail_lang_ecma::code_rule_dsl::module_graph::*;
        // The module graph tests' fixtures, which the engine's napi tests read too.
        #[cfg(any(test, feature = "test-hooks"))]
        pub mod fixtures;
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
    pub use rerail_lang::code_rule_dsl::module_path;
    pub use rerail_workspace::code_rule_dsl::origin;
    pub use rerail_lang_ecma::code_rule_dsl::package_manifest;
    // P's phase clocks; read by profiling once stages are named (05 Phase 11 row 2).
    #[cfg(any(test, feature = "test-hooks"))]
    pub use rerail_prof::code_rule_dsl::phase_clock;
    pub use rerail_lang_php::code_rule_dsl::php_literal;
    pub mod php_view {
        #[cfg(any(test, feature = "test-hooks"))]
        pub use rerail_lang_php::code_rule_dsl::php_view::*;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::blade::forte::scan;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::php_literal;
        #[allow(unused_imports)]
        use std::borrow::Cow;
        #[allow(unused_imports)]
        use std::ops::Range;
    }
    #[allow(unused_imports)] // boot's mount: only boot's files read it
    pub use rerail_facts::code_rule_dsl::producers;
    // The prefilter (rerail-prefilter's), with the tests of it that read what boot registers (its languages).
    pub mod prefilter {
        pub use rerail_prefilter::code_rule_dsl::prefilter::*;
        // `prefilter.rs`'s own imports, which its tests read (`use super::*`).
        #[allow(unused_imports)]
        use crate::code_rule_dsl::{language, tape::TapeDoc};
        #[allow(unused_imports)]
        use crate::content_store::{ContentStore, StoreError, StoreResult, TapeId, TapePrefilter};
        #[allow(unused_imports)]
        use ast_grep_core::Doc;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::search_index::kind_token;
        #[cfg(test)]
        mod tests;
        #[cfg(test)]
        #[path = "lower/audit_tests.rs"]
        mod audit_tests;
        #[cfg(test)]
        #[path = "lab_tests.rs"]
        mod lab_tests;
        pub mod lower {
            pub use rerail_prefilter::code_rule_dsl::prefilter::lower::*;
            // `lower.rs`'s own imports, which its tests read (`use super::*`).
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::{AnchorPlan, Condition, Required, Requirement};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::stats::inputs::Statistics;
            #[allow(unused_imports)]
            use rusqlite::types::Value;
            #[allow(unused_imports)]
            use std::collections::{BTreeSet, HashMap};
            #[cfg(test)]
            #[path = "../lower_tests.rs"]
            mod tests;
        }
        pub mod tree {
            pub use rerail_prefilter::code_rule_dsl::prefilter::tree::*;
            #[cfg(test)]
            mod tests;
        }
    }
    #[allow(unused_imports)] // boot's mount: only boot's files read it
    pub use rerail_transducer::code_rule_dsl::space;
    // Read by the prefilter's lowering; the compiled-plan cache is wired with the plans row.
    pub mod stats {
        #[cfg_attr(not(test), allow(unused_imports))]
        pub use rerail_workspace::code_rule_dsl::stats::*;
        pub mod ingest {
            #[cfg_attr(not(test), allow(unused_imports))]
            pub use rerail_workspace::code_rule_dsl::stats::ingest::*;
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
        pub mod inputs {
            #[cfg_attr(not(test), allow(unused_imports))]
            pub use rerail_workspace::code_rule_dsl::stats::inputs::*;
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
        pub mod plans {
            #[cfg_attr(not(test), allow(unused_imports))]
            pub use rerail_workspace::code_rule_dsl::stats::plans::*;
            #[allow(unused_imports)]
            use rusqlite::{Connection, OptionalExtension, Result, params};
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, btree_map};
            #[allow(unused_imports)]
            use std::sync::Arc;
        }
    }
    // Wall time by stage, with RERAIL_PROF set (O28).
    pub use rerail_prof::code_rule_dsl::prof;
    // P's hooks are called from its matcher and relational code; W wires them with 03's stage names.
    #[cfg(feature="code-rule-profile")]
    pub use rerail_workspace::code_rule_dsl::profile;
    // The same SQL diagnostics serve standalone execution and the workspace profiler.
    #[cfg(any(test, feature = "test-hooks"))]
    pub use rerail_facts::code_rule_dsl::sql_profile;
    pub mod sql_admission {
        #[cfg(any(test, feature = "test-hooks"))]
        pub use rerail_facts::code_rule_dsl::sql_admission::*;
        #[allow(unused_imports)]
        use parking_lot::Mutex;
        #[allow(unused_imports)]
        use std::cell::RefCell;
        #[allow(unused_imports)]
        use std::sync::Arc;
        #[allow(unused_imports)]
        use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
    }
    pub mod query {
        pub use rerail_workspace::code_rule_dsl::query::*;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::model::RegexOptions;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::model::{FileInput, RegexSource};
        #[allow(unused_imports)]
        use regex::Regex;
        #[allow(unused_imports)]
        use std::ops::Range;
    }
    // Admission reads these once the batch budget is wired; until then only tests call it.
    #[allow(unused_imports)] // boot's mount: only boot's files read it
    pub use rerail_workspace::code_rule_dsl::resolution;
    pub use rerail_facts::code_rule_dsl::search_index;
    #[cfg(any(test, feature = "test-hooks"))]
    pub use rerail_workspace::code_rule_dsl::walks;
    #[allow(unused_imports)] // boot's mount: only boot's files read it
    pub mod functors;
    pub use rerail_workspace::code_rule_dsl::rule_api;
    pub mod residency {
        pub mod admission {
            pub mod relational {
                pub mod syntax {
                    pub use rerail_workspace::code_rule_dsl::residency::admission::relational::syntax::*;
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
                    pub mod layout {
                        #[cfg(any(test, feature = "test-hooks"))]
                        pub use rerail_workspace::code_rule_dsl::residency::admission::relational::producer::layout::*;
                        #[allow(unused_imports)]
                        use super::{Demand, Resident};
                        #[allow(unused_imports)]
                        use std::{collections::HashSet, sync::Arc};
                    }
                    pub use rerail_workspace::code_rule_dsl::residency::admission::relational::producer::*;
                }
                pub mod pool {
                    #[cfg(any(test, feature = "test-hooks"))]
                    pub use rerail_workspace::code_rule_dsl::residency::admission::relational::pool::*;
                    #[allow(unused_imports)]
                    use super::{Database, Result, configure, connection, sql_error};
                    #[allow(unused_imports)]
                    use std::{ cell::{Cell, RefCell}, ops::Deref, rc::Rc, sync::{Arc, atomic::AtomicUsize}, };
                }
                pub mod admission {
                    #[cfg(any(test, feature = "test-hooks"))]
                    pub use rerail_workspace::code_rule_dsl::residency::admission::relational::admission::*;
                    #[allow(unused_imports)]
                    use super::{Inputs, Result, invalid};
                    #[allow(unused_imports)]
                    use crate::code_rule_dsl::{ eval, model::{ClauseKind, CompiledClause, CompiledCodeRule, Metrics, Snapshot}, };
                }
                pub use rerail_workspace::code_rule_dsl::residency::admission::relational::*;
                pub mod batch {
                    #[cfg(any(test, feature = "test-hooks"))]
                    pub use rerail_workspace::code_rule_dsl::residency::admission::relational::batch::*;
                    #[allow(unused_imports)]
                    use crate::code_rule_dsl::residency::admission::relational::{Inputs, Job, Preparation, failure, producer, sql_error};
                    #[allow(unused_imports)]
                    use crate::code_rule_dsl::model::{CompiledCodeRule, Metrics};
                    #[allow(unused_imports)]
                    use crate::code_rule_dsl::{budget::Context, model::CompiledClause};
                    #[allow(unused_imports)]
                    use std::collections::VecDeque;
                }
                pub mod inputs {
                    #[cfg(any(test, feature = "test-hooks"))]
                    pub use rerail_workspace::code_rule_dsl::residency::admission::relational::inputs::*;
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
                pub use crate::code_rule_dsl::residency::admission::relational::reachability::Reachability;
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
            #[cfg_attr(not(test), allow(unused_imports))]
            pub use rerail_device_metal::code_rule_dsl::residency::slabs::*;
        }
        #[allow(unused_imports)]
        pub use crate::code_rule_dsl::residency::admission::relational::ast_stage::install_ast_producer;
    }
    // P's paths for P's relational stage, under test (open item O18): P's workspace code reads it here.
    pub mod scope {
        pub use rerail_workspace::code_rule_dsl::scope::*;
        pub use rerail_lang_ecma::code_rule_dsl::scope::ecma;
        pub mod php {
            #[cfg(any(test, feature = "test-hooks"))]
            pub use rerail_lang_php::code_rule_dsl::scope::php::*;
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
            pub use rerail_lang_php::code_rule_dsl::scope::php_names::*;
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
            pub use rerail_lang_rust::code_rule_dsl::scope::rust::*;
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
            #[cfg(any(test, feature = "test-hooks"))]
            pub use rerail_lang_sql::code_rule_dsl::scope::sql::*;
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
        pub mod usage {
            pub use rerail_workspace::code_rule_dsl::scope::usage::*;
            #[cfg(any(test, feature = "test-hooks"))]
            pub use rerail_lang_python::code_rule_dsl::scope::usage::lang_python;
            #[cfg(any(test, feature = "test-hooks"))]
            pub use rerail_lang_rust::code_rule_dsl::scope::usage::lang_rust;
            pub mod rust {
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
        }
        #[allow(unused_imports)]
        pub use crate::code_rule_dsl::scope::lookup::Lookup;
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
        #[allow(unused_imports)]
        use crate::code_rule_dsl::identifier::{name_in};
    }
    pub mod regexes {
        #[cfg_attr(not(test), allow(unused_imports))]
        pub use rerail_lang::code_rule_dsl::regexes::*;
        #[allow(unused_imports)]
        use std::sync::Arc;
    }
    pub use rerail_lang::code_rule_dsl::semver;
    // The first slice's entry points (06 S6), called by hosts through `slice::napi::CodeRuleSlice`.
    pub mod slice;
    pub mod soundness {
        pub use rerail_workspace::code_rule_dsl::soundness::*;
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
    pub mod suppress {
        pub mod rerail {
            pub use rerail_workspace::code_rule_dsl::suppress::rerail::*;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::embed::segments::FileRanges;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::{Family, GrammarDialect};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::origin::InFile;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::tape::{TapeDoc, tape_doc};
            #[allow(unused_imports)]
            use ast_grep_core::{AstGrep, Doc, Node};
            #[allow(unused_imports)]
            use std::ops::Range;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::pack as language_pack;
        }
        pub use rerail_workspace::code_rule_dsl::suppress::*;
        pub mod biome {
            #[cfg_attr(not(test), allow(unused_imports))]
            pub use rerail_lang_ecma::code_rule_dsl::suppress::biome::*;
            #[allow(unused_imports)]
            use super::names;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::{object, reject_unknown};
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use serde_json::Value;
            #[allow(unused_imports)]
            use std::ops::Range;
        }
        #[allow(unused_imports)] // boot's mount: only boot's files read it
        pub use rerail_lang_rust::code_rule_dsl::suppress::clippy;
        #[cfg_attr(not(test), allow(unused_imports))]
        pub use rerail_lang_ecma::code_rule_dsl::suppress::eslint;
        #[cfg_attr(not(test), allow(unused_imports))]
        pub use rerail_lang_python::code_rule_dsl::suppress::noqa;
        pub mod nosec {
            #[cfg_attr(not(test), allow(unused_imports))]
            pub use rerail_lang_python::code_rule_dsl::suppress::nosec::*;
            #[allow(unused_imports)]
            use super::names;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::{object, reject_unknown};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::lines::LineTable;
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use regex::Regex;
            #[allow(unused_imports)]
            use serde_json::Value;
            #[allow(unused_imports)]
            use std::collections::BTreeMap;
            #[allow(unused_imports)]
            use std::sync::LazyLock;
        }
        pub mod oxlint {
            #[cfg_attr(not(test), allow(unused_imports))]
            pub use rerail_lang_ecma::code_rule_dsl::suppress::oxlint::*;
            #[allow(unused_imports)]
            use super::names;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::{object, reject_unknown};
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use serde_json::Value;
            #[allow(unused_imports)]
            use std::collections::HashMap;
            #[allow(unused_imports)]
            use std::ops::Range;
        }
        #[cfg_attr(not(test), allow(unused_imports))]
        pub use rerail_lang_php::code_rule_dsl::suppress::phpcs;
        pub mod phpstan {
            #[cfg_attr(not(test), allow(unused_imports))]
            pub use rerail_lang_php::code_rule_dsl::suppress::phpstan::*;
            #[allow(unused_imports)]
            use super::psalm::{doc_comment, pcre_space};
            #[allow(unused_imports)]
            use super::{names, php_comment_end};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::{object, reject_unknown};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::lines::LineTable;
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use serde_json::Value;
            #[allow(unused_imports)]
            use std::cell::RefCell;
            #[allow(unused_imports)]
            use std::collections::HashMap;
            #[allow(unused_imports)]
            use std::ops::Range;
        }
        pub use rerail_lang_php::code_rule_dsl::suppress::psalm;
        #[allow(unused_imports)] // boot's mount: only boot's files read it
        pub use rerail_lang_sql::code_rule_dsl::suppress::sqlfluff;
        pub use rerail_lang_ecma::code_rule_dsl::suppress::vue;
        #[allow(unused_imports)]
        use ast_grep_core::{Doc, Node};
        #[allow(unused_imports)]
        pub use crate::code_rule_dsl::suppress::rerail::RerailIgnores;
        #[allow(unused_imports)]
        pub use crate::code_rule_dsl::suppress::kinds::*;
    }
    pub mod tape {
        pub mod resident {
            #[cfg(any(test, feature = "test-hooks"))]
            pub use rerail_lang::code_rule_dsl::tape::resident::*;
            #[allow(unused_imports)]
            use super::*;
        }
        pub use rerail_lang::code_rule_dsl::tape::*;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::embed::segments::SegmentMap;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::embed::Program;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::{GrammarDialect, KindCatalog, Via};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::lines::LineTable;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::lines::Terminators;
        #[allow(unused_imports)]
        use ast_grep_core::Position;
        #[allow(unused_imports)]
        use ast_grep_core::source::{Doc, Edit, SgNode};
        #[allow(unused_imports)]
        use ast_grep_language::LanguageExt;
        #[allow(unused_imports)]
        use sha2::{Digest, Sha256};
        #[allow(unused_imports)]
        use std::borrow::Cow;
        #[allow(unused_imports)]
        use std::ops::Range;
        #[allow(unused_imports)]
        use std::sync::{Arc, Weak};
        #[allow(unused_imports)]
        use tree_sitter::{Language, Node as TsNode, TreeCursor};
        #[allow(unused_imports)]
        pub use crate::code_rule_dsl::tape::index::IndexImage;
        #[allow(unused_imports)]
        pub use crate::code_rule_dsl::tape::wire::index_span;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::tape::wire::{CHECKPOINT_STRIDE, WIDTH};
    }
    pub mod tree {
        pub use rerail_lang::code_rule_dsl::tree::*;
        #[allow(unused_imports)]
        use ast_grep_core::{Doc, Node};
        #[allow(unused_imports)]
        use std::borrow::Cow;
        #[allow(unused_imports)]
        use std::ops::Range;
    }

    pub use rerail_facts::code_rule_dsl::text;
    #[cfg(test)]
    pub mod test_support;
    /// The engine's text tools without Node (the CLI's commands); the engine adds their napi wrappers.
    pub mod tools;
    pub mod transducer {
        pub use rerail_transducer::code_rule_dsl::transducer::*;
        pub mod packed {
            pub mod set_mode {
                #[cfg(any(test, feature = "test-hooks"))]
                pub use rerail_transducer::code_rule_dsl::transducer::packed::set_mode::*;
                #[allow(unused_imports)]
                #[cfg(any(test, feature = "test-hooks"))]
                use super::*;
            }
            // `.`: no directory `rir/` exists to resolve `..` through.
            pub mod rir {
                pub use rerail_transducer::code_rule_dsl::transducer::packed::rir::*;
                // The grammar's names; the compiler's `program::Names` comes in by `super::*` too.
                #[cfg_attr(not(test), allow(unused_imports))]
                pub use rerail_transducer::code_rule_dsl::transducer::packed::rir::Names;
                #[allow(unused_imports)]
                use rerail_rir::rir_asm::{Line, NameTable, Token, block_programs_rows, head_rows, index, tokens, word_row, word_rows};
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
            pub mod compilation {
                pub use rerail_transducer::code_rule_dsl::transducer::packed::compilation::*;
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
            pub mod capture_run {
                #[cfg(any(test, feature = "test-hooks"))]
                pub use rerail_transducer::code_rule_dsl::transducer::packed::capture_run::*;
                #[allow(unused_imports)]
                use super::*;
            }
            pub mod regex_block {
                #[cfg_attr(not(test), allow(unused_imports))]
                pub use rerail_transducer::code_rule_dsl::transducer::packed::regex_block::*;
                #[allow(unused_imports)]
                use super::block::{BlockOp, BlockProgram, ClassTable};
                #[allow(unused_imports)]
                use regex_syntax::hir::{Class, Hir, HirKind, Look};
                #[allow(unused_imports)]
                use std::collections::HashMap;
            }
            pub mod native_cpu {
                #[cfg(any(test, feature = "test-hooks"))]
                pub use rerail_transducer::code_rule_dsl::transducer::packed::native_cpu::*;
                // The compiled CPU executor's registration, which boot makes in every build (`install_devices`).
                pub use rerail_transducer::code_rule_dsl::transducer::packed::native_cpu::register;
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
            pub use rerail_transducer::code_rule_dsl::transducer::packed::*;
            pub mod compile_matcher {
                pub use rerail_transducer::code_rule_dsl::transducer::packed::compile_matcher::*;
                #[cfg(any(target_os = "macos", target_os = "linux", windows))]
                pub use rerail_device_metal::code_rule_dsl::transducer::packed::compile_matcher::matcher_metal;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::transducer::packed::*;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::matcher::{Axis, MatchTree, Stop};
                #[allow(unused_imports)]
                use std::collections::{BTreeMap, BTreeSet, HashMap};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::tape::DERIVED_CLASS_BASE as LABEL_BASE;
            }
            // Read by simdjson's registration (not wasm: `install_languages`) and the tests.
            #[cfg(any(test, feature = "test-hooks", not(target_family = "wasm")))]
            #[cfg_attr(not(test), allow(unused_imports))]
            pub use rerail_lang_json::code_rule_dsl::transducer::packed::json_parser;
            #[cfg(any(target_os = "macos", target_os = "linux", windows))]
            pub mod metal_backend {
                #[cfg(any(test, feature = "test-hooks"))]
                pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_backend::*;
                // A launch-budget test that parses a document: it runs where the languages are registered.
            }
            #[cfg(all(any(test, feature = "test-hooks"), any(target_os = "macos", target_os = "linux", windows)))]
            #[allow(unused_imports)]
            pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_join;
            #[cfg(any(target_os = "macos", target_os = "linux", windows))]
            pub mod metal_program_backend {
                pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_program_backend::*;
                // The tape tests store parsed documents: they run where the languages are registered.
            }
            // Metal's semantic fixtures parse documents: they run where the languages are registered.
            #[cfg(all(any(test, feature = "test-hooks"), any(target_os = "macos", target_os = "linux", windows)))]
            pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_rows;
            #[cfg(all(any(test, feature = "test-hooks"), any(target_os = "macos", target_os = "linux", windows)))]
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
            #[cfg(any(test, feature = "test-hooks"))]
            #[allow(unused_imports)]
            use crate::code_rule_dsl::transducer::packed::literal_anchors::*;
            #[allow(unused_imports)]
            #[cfg(all(any(test, feature = "test-hooks"), any(target_os = "macos", target_os = "linux", windows)))]
            pub use crate::code_rule_dsl::transducer::packed::metal_backend::launch::LaunchScope as MetalTestScope;
            #[allow(unused_imports)]
            #[cfg(any(target_os = "macos", target_os = "linux", windows))]
            pub use crate::code_rule_dsl::transducer::packed::metal_program_backend::set_steps;
            #[allow(unused_imports)]
            #[cfg(any(target_os = "macos", target_os = "linux", windows))]
            pub use crate::code_rule_dsl::transducer::packed::metal_program_backend::lattice as metal_lattice;
            #[allow(unused_imports)]
            #[cfg(any(target_os = "macos", target_os = "linux", windows))]
            pub use crate::code_rule_dsl::transducer::packed::metal_program_backend::nearest as metal_nearest;
            #[allow(unused_imports)]
            pub use crate::code_rule_dsl::transducer::packed::device::{RootJobs, RootsFound};
            #[allow(unused_imports)]
            #[cfg(any(target_os = "macos", target_os = "linux", windows))]
            pub use crate::code_rule_dsl::transducer::packed::compile_matcher::matcher_metal;
            #[allow(unused_imports)]
            pub use crate::code_rule_dsl::transducer::packed::compile_matcher::{Backend as MatcherBackend, LoweredMatcher, LoweredWave, WaveRun};
            #[allow(unused_imports)]
            pub use crate::code_rule_dsl::transducer::packed::compile_micro::{Emit, Frames, Micro, RootRecord, steps as micro_steps};
            #[allow(unused_imports)]
            pub use crate::code_rule_dsl::transducer::packed::compilation::PlanId;
            #[allow(unused_imports)]
            use rerail_compiler::native_cpu_codegen;
            #[allow(unused_imports)]
            pub use rerail_cpu_block::native_cpu_rows;
            #[allow(unused_imports)]
            pub use crate::code_rule_dsl::transducer::packed::packed_batch::PackedExecution;
            #[allow(unused_imports)]
            pub use rerail_compiler::block;
            #[allow(unused_imports)]
            pub use rerail_rr_parser as rr_parser;
            #[allow(unused_imports)]
            use rerail_compiler::program::*;
            #[allow(unused_imports)]
            pub use rerail_compiler::program::{NAMED, Words, word};
            #[allow(unused_imports)]
            pub use rerail_compiler::Error;
            #[allow(unused_imports)]
            #[cfg(any(test, feature = "test-hooks"))]
            use literal_anchors::*;
            // The admission spike's tests parse Python, which the engine registers: they run here, and the transducer
            // keeps the spike only for its test hooks (`metal_frontier_conformance` reads a program of it).
        }
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use crate::code_rule_dsl::tape::TapeDoc;
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use ast_grep_core::MatchStrictness;
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use ast_grep_core::matcher::{Pattern, PatternNode};
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use regex::Regex;
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use std::collections::{BTreeMap, BTreeSet};
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use std::ops::Range;
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use crate::code_rule_dsl::search_index::persist_search_index_batch;
    }
    // Typing's rounds run over T's facts in tests only until Phase 10 wires them.
    pub mod typing {
        #[allow(unused_imports)]
        pub use rerail_logic::code_rule_dsl::typing::*;
        // Tests that read what the engine registers as it loads (the languages, their `.rerail` libraries) run here.
        pub mod file_local {
            #[cfg_attr(not(test), allow(unused_imports))]
            pub use rerail_logic::code_rule_dsl::typing::file_local::*;
            // A test that reads T's programs as text: it runs where the Rr language is registered.
            #[allow(unused_imports)]
            use crate::content_store::{StoreError, StoreResult};
            #[allow(unused_imports)]
            pub use crate::code_rule_dsl::facts::nearest::{Binding, NONE, Trees};
            #[allow(unused_imports)]
            pub use crate::code_rule_dsl::logic::front::document_column;
            #[allow(unused_imports)]
            use std::collections::BTreeMap;
        }
    }
    pub mod value {
        pub use rerail_workspace::code_rule_dsl::value::*;
        pub mod ecma {
            pub use rerail_lang_ecma::code_rule_dsl::value::ecma::*;
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
            #[cfg(any(test, feature = "test-hooks"))]
            pub use rerail_lang_ecma::code_rule_dsl::value::key::*;
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
            pub use rerail_lang_php::code_rule_dsl::value::php::*;
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
            pub use rerail_lang_php::code_rule_dsl::value::php_compare::*;
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
            #[cfg(any(test, feature = "test-hooks"))]
            pub use rerail_lang_python::code_rule_dsl::value::python::*;
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
            #[cfg(any(test, feature = "test-hooks"))]
            pub use rerail_lang_rust::code_rule_dsl::value::rust::*;
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
            #[cfg(any(test, feature = "test-hooks"))]
            pub use rerail_lang_sql::code_rule_dsl::value::sql::*;
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
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::{Family, GrammarDialect};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::tape::TapeDoc;
        #[allow(unused_imports)]
        use ast_grep_core::tree_sitter::StrDoc;
        #[allow(unused_imports)]
        use ast_grep_core::{Doc, Node};
        #[allow(unused_imports)]
        pub use num_bigint::BigInt;
        #[allow(unused_imports)]
        use num_traits::{FromPrimitive, Signed, ToPrimitive, Zero};
        #[allow(unused_imports)]
        use std::cmp::Ordering;
        #[allow(unused_imports)]
        use std::ops::Range;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::adapter::pack as language_pack;
        #[allow(unused_imports)]
        pub use crate::code_rule_dsl::value::text_regex::MAX_STRING_BYTES;
    }
    pub mod violations {
        pub use rerail_workspace::code_rule_dsl::violations::*;
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
    pub mod vue {
        pub mod view {
            #[cfg(any(test, feature = "test-hooks"))]
            pub use rerail_lang_ecma::code_rule_dsl::vue::view::*;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::embed::{Program, segments::SegmentMap};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::GrammarDialect;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::vue::sfc;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::vue::template::{self, Arg, Directive, Parse};
            #[allow(unused_imports)]
            use ast_grep_language::LanguageExt;
            #[allow(unused_imports)]
            use std::borrow::Cow;
            #[allow(unused_imports)]
            use std::ops::Range;
            #[allow(unused_imports)]
            use tree_sitter::{Node, Parser, Tree};
        }
        pub use rerail_lang_ecma::code_rule_dsl::vue::*;
    }
    // Wired to the logic planner and the push path in a later slice; until then only tests call it.
    pub mod workspace {
        // The workspace tests' fixtures, which the tests of the crates above read too.
        #[cfg(any(test, feature = "test-hooks"))]
        pub mod fixtures;
        pub mod results {
            #[cfg(any(test, feature = "test-hooks"))]
            pub use rerail_workspace::code_rule_dsl::workspace::results::*;
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
        pub use rerail_workspace::code_rule_dsl::workspace::*;
        pub mod ingest {
            pub use rerail_workspace::code_rule_dsl::workspace::ingest::*;
            #[cfg_attr(not(test), allow(unused_imports))]
            pub use rerail_lang_ecma::code_rule_dsl::workspace::ingest::ecma;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::workspace::crossfile::{CrossFileScheduler};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::workspace::documents;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::workspace::{FileState, WorkspaceState};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::eval::{file_language, grammar_matcher};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::{invalid, json_error};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::GrammarDialect;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::{package_manifests};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::model::{
                ClauseKind, CompiledClause, CompiledCodeRule, FileInput, Query, package_marker_directory,
                python_module_in,
            };
            #[allow(unused_imports)]
            use crate::code_rule_dsl::module_graph;
            #[allow(unused_imports)]
            use crate::content_store::StoreError;
            #[allow(unused_imports)]
            use napi::Result;
            #[allow(unused_imports)]
            use serde::Deserialize;
            #[allow(unused_imports)]
            use serde_json::Value;
            #[allow(unused_imports)]
            use sha2::{Digest, Sha256};
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, BTreeSet, HashSet};
            #[allow(unused_imports)]
            use std::sync::Arc;
            #[allow(unused_imports)]
            #[cfg(any(test, feature = "test-hooks"))]
            use std::sync::atomic::Ordering;
        }
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
        #[cfg(any(test, feature = "test-hooks"))]
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

    #[cfg(any(test, feature = "test-hooks"))]
    #[cfg_attr(not(test), allow(unused_imports))] // boot's fixtures (`prefilter::fixtures`) read it under `test-hooks`
    pub use rerail_lang::code_rule_dsl::fixtures;

    /// The waves: the core's (`rerail-logic`), and the glue boot owns (the first slice's wave, findings and
    /// policies, the schedule's batches).
    pub mod waves {
        pub use rerail_logic::code_rule_dsl::waves::{owner, storage};
        #[cfg(any(test, feature = "test-hooks"))]
        pub use rerail_workspace::code_rule_dsl::waves::relational;
        pub mod schedule {
            pub use rerail_logic::code_rule_dsl::waves::schedule::*;
            pub mod batches;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::waves::owner::{Database, Lease};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::waves::owner::sql::SqlOwner;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::facts::Graph;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::facts::vtab::Source;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::facts::vtab::syntax::{Document, fresh, sources};
            #[allow(unused_imports)]
            use crate::content_store::{DerivationInput, DerivationRecord, StoreError, StoreResult};
            #[allow(unused_imports)]
            use sha2::{Digest as _, Sha256};
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, BTreeSet};
            #[allow(unused_imports)]
            use std::sync::Arc;
            #[allow(unused_imports)]
            pub use crate::code_rule_dsl::waves::schedule::lattice::Operations as LatticeOperations;
        }
        pub mod matches;
        pub mod findings;
        pub mod policies;
    }
    /// The composition (`install`).
    pub mod boot;
    pub use rule_api::CompiledCodeRuleDsl;
    #[cfg(any(test, feature = "test-hooks"))]
    pub use rule_api::CodeRuleDslEditor;
}
