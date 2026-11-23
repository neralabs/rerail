//! The engine's tests of calls and literals: the call graphs, literals, the evaluator, queries, soundness, embedding and regexes (docs/final-shape/proposals/crate-split/incremental-plan.md, step 7i).
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
#[path = "../../../src/test_private_folders.rs"]
mod test_private_folders;

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
}
