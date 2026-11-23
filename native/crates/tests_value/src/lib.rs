//! The engine's tests of values: the value readers and their oracles, Blade and its PHP view, and the argument binding checked against them (docs/final-shape/proposals/crate-split/incremental-plan.md, step 7i).
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
    // P's paths for P's relational stage, under test (open item O18): P's workspace code reads it here.
    #[cfg_attr(test, path = "scope")]
    mod scope {
        pub use rerail_boot::code_rule_dsl::scope::*;
        pub use rerail_lang_ecma::code_rule_dsl::scope::ecma;
        pub mod php {
            pub use rerail_boot::code_rule_dsl::scope::php::*;
            #[cfg(test)]
                    pub mod flow_tests;
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
        pub use rerail_lang_python::code_rule_dsl::scope::python;
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
        #[allow(unused_imports)]
        use crate::code_rule_dsl::identifier::{name_in};
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

// The engine's allocator (`rerail_prof::heap::Sampled`, as `rerail-engine`'s `lib.rs` declares it): without it the
// test binary ran on the system allocator, which no build of the engine runs on.
#[cfg(not(any(windows, target_family = "wasm", feature = "system-allocator")))]
#[global_allocator]
static GLOBAL: rerail_prof::heap::Sampled = rerail_prof::heap::Sampled;
