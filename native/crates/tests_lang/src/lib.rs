//! The engine's tests of the languages: tapes, the language registry, facts, trees, Blade and Vue (docs/final-shape/proposals/crate-split/incremental-plan.md, step 2).
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
    #[cfg_attr(test, path = "facts")]
    mod facts {
        pub use rerail_boot::code_rule_dsl::facts::*;
        pub mod families {
            pub use rerail_boot::code_rule_dsl::facts::families::*;
            #[cfg(test)]
            #[path = "tests.rs"]
            pub mod tests;
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
            pub use rerail_boot::code_rule_dsl::facts::core::*;
            #[cfg(test)]
            #[path = "tests.rs"]
            pub mod tests;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::tape_tag;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::lines::LineTable;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::tape::TapeDoc;
            #[allow(unused_imports)]
            use std::sync::Arc;
        }
        #[cfg_attr(test, path = "producers")]
        pub mod producers {
            pub use rerail_boot::code_rule_dsl::facts::producers::*;
            pub mod python {
                pub use rerail_boot::code_rule_dsl::facts::producers::python::*;
                pub mod python_facts {
                    pub use rerail_boot::code_rule_dsl::facts::producers::python::python_facts::*;
                    #[cfg(test)]
                    #[path = "tests.rs"]
                    pub mod tests;
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
                    pub use rerail_boot::code_rule_dsl::facts::producers::python::import_screen::*;
                    #[cfg(test)]
                    #[path = "tests.rs"]
                    pub mod tests;
                    #[allow(unused_imports)]
                    use crate::code_rule_dsl::{ identifier::{nfkc, python_identifier}, facts::producers::python::python_facts::{Fact, qualification_chain}, module_path::ModulePath, tape::TapeDoc, };
                    #[allow(unused_imports)]
                    use ast_grep_core::{Doc, Node};
                    #[allow(unused_imports)]
                    use std::collections::{BTreeMap, BTreeSet};
                }
                pub mod families {
                    pub use rerail_boot::code_rule_dsl::facts::producers::python::families::*;
                    pub mod candidates {
                        pub use rerail_boot::code_rule_dsl::facts::producers::python::families::candidates::*;
                        #[cfg(test)]
                        #[path = "tests.rs"]
                        pub mod tests;
                        #[allow(unused_imports)]
                        use super::*;
                        #[allow(unused_imports)]
                        use std::collections::BTreeSet;
                    }
                    #[cfg(test)]
                    #[path = "tests.rs"]
                    pub mod tests;
                    #[allow(unused_imports)]
                    use crate::code_rule_dsl::facts::core::{Facts, NONE, kind_atom};
                    #[allow(unused_imports)]
                    use crate::code_rule_dsl::language::KindCatalog;
                }
                pub mod call_batch {
                    pub use rerail_boot::code_rule_dsl::facts::producers::python::call_batch::*;
                    #[cfg(test)]
                    #[path = "tests.rs"]
                    pub mod tests;
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
            }
        }
        #[cfg_attr(test, path = "vtab")]
        pub mod vtab {
            pub use rerail_boot::code_rule_dsl::facts::vtab::*;
            #[allow(unused_imports)] // boot's mount: only boot's files read it
            pub use rerail_lang_python::code_rule_dsl::facts::vtab::python;
            #[cfg_attr(test, path = "syntax")]
            pub mod syntax {
                pub use rerail_boot::code_rule_dsl::facts::vtab::syntax::*;
                pub mod core_rr {
                    pub use rerail_boot::code_rule_dsl::facts::vtab::syntax::core_rr::*;
                    #[cfg(test)]
                    #[path = "tests.rs"]
                    pub mod tests;
                    #[allow(unused_imports)]
                    use super::{Dialect, Projected, Relation};
                    #[allow(unused_imports)]
                    use crate::code_rule_dsl::facts::core::Facts;
                    #[allow(unused_imports)]
                    use std::collections::{BTreeSet, HashSet};
                }
                #[allow(unused_imports)] // boot's mount: only boot's files read it
                pub use rerail_lang_python::code_rule_dsl::facts::vtab::syntax::annotation;
                #[allow(unused_imports)] // boot's mount: only boot's files read it
                pub use rerail_boot::code_rule_dsl::facts::vtab::syntax::ast_wave;
                #[cfg(test)]
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
                    pub use rerail_boot::code_rule_dsl::facts::vtab::syntax::python_demand::*;
                    #[cfg(test)]
                    #[path = "tests.rs"]
                    pub mod tests;
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
                    pub use rerail_boot::code_rule_dsl::facts::vtab::syntax::python_parts::*;
                    #[cfg(test)]
                    #[path = "tests.rs"]
                    pub mod tests;
                    #[allow(unused_imports)]
                    use super::core_rr::py_unescape;
                    #[allow(unused_imports)]
                    use super::{Dialect, Projected};
                }
                #[allow(unused_imports)] // boot's mount: only boot's files read it
                pub use rerail_logic::code_rule_dsl::facts::vtab::syntax::rr_core;
                #[cfg(test)]
                pub mod tests;
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
    #[cfg_attr(test, path = "language")]
    mod language {
        pub use rerail_boot::code_rule_dsl::language::*;
        #[cfg_attr(test, path = "adapter")]
        pub mod adapter {
            pub use rerail_boot::code_rule_dsl::language::adapter::*;
            // The adapter kit's checks of every family, by `with_adapter!` (this crate's build script).
            pub mod kit;
            #[cfg_attr(not(test), allow(unused_imports))]
            pub use rerail_lang_php::code_rule_dsl::language::adapter::blade;
            pub use rerail_lang_ecma::code_rule_dsl::language::adapter::ecma;
            // Java's and C#'s families: syntax only so far, their tapes from the lean parsers.
            #[cfg_attr(not(test), allow(unused_imports))]
            pub use rerail_boot::code_rule_dsl::language::adapter::csharp;
            #[cfg_attr(not(test), allow(unused_imports))]
            pub use rerail_boot::code_rule_dsl::language::adapter::java;
            // Go's, C's and C++'s families: syntax only so far, their tapes from the lean parsers.
            #[cfg_attr(not(test), allow(unused_imports))]
            pub use rerail_boot::code_rule_dsl::language::adapter::c;
            #[allow(unused_imports)] // boot's mount: only boot's files read it
            pub use rerail_boot::code_rule_dsl::language::adapter::c_pp;
            #[cfg_attr(not(test), allow(unused_imports))]
            pub use rerail_boot::code_rule_dsl::language::adapter::go;
            // Kotlin's, Swift's and Scala's families: syntax only so far, their tapes from the lean parsers.
            #[cfg_attr(not(test), allow(unused_imports))]
            pub use rerail_boot::code_rule_dsl::language::adapter::kotlin;
            #[cfg_attr(not(test), allow(unused_imports))]
            pub use rerail_boot::code_rule_dsl::language::adapter::scala;
            #[cfg_attr(not(test), allow(unused_imports))]
            pub use rerail_boot::code_rule_dsl::language::adapter::swift;
            // HTML's, CSS's and Markdown's families: syntax only so far, read by tree-sitter.
            #[cfg_attr(not(test), allow(unused_imports))]
            pub use rerail_boot::code_rule_dsl::language::adapter::css;
            #[cfg_attr(not(test), allow(unused_imports))]
            pub use rerail_boot::code_rule_dsl::language::adapter::html;
            #[cfg_attr(not(test), allow(unused_imports))]
            pub use rerail_boot::code_rule_dsl::language::adapter::markdown;
            #[cfg_attr(not(test), allow(unused_imports))]
            pub use rerail_lang_json::code_rule_dsl::language::adapter::json;
            #[cfg_attr(not(test), allow(unused_imports))]
            pub use rerail_lang_php::code_rule_dsl::language::adapter::php;
            pub use rerail_lang_python::code_rule_dsl::language::adapter::python;
            #[cfg_attr(not(test), allow(unused_imports))]
            pub use rerail_lang_rust::code_rule_dsl::language::adapter::rust;
            #[cfg_attr(not(test), allow(unused_imports))]
            pub use rerail_lang_sql::code_rule_dsl::language::adapter::sql;
            pub mod vue {
                pub use rerail_boot::code_rule_dsl::language::adapter::vue::*;
                #[cfg(test)]
                #[path = "tests.rs"]
                pub mod tests;
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
            pub(crate) use crate::code_rule_dsl::language::adapter::arguments::{ArgumentFeature, ArgumentRules};
            #[allow(unused_imports)]
            pub(crate) use crate::code_rule_dsl::language::adapter::calls::{CallFeature, CallRules};
            #[allow(unused_imports)]
            pub(crate) use crate::code_rule_dsl::language::adapter::format_args::{FormatArgFeature, FormatArgRules};
            #[allow(unused_imports)]
            pub(crate) use crate::code_rule_dsl::language::adapter::scope::{ScopeFeature, ScopeRules};
            #[allow(unused_imports)]
            pub(crate) use crate::code_rule_dsl::language::adapter::syntax::{QueryWhen, SyntaxFeature, SyntaxRules};
            #[allow(unused_imports)]
            pub(crate) use crate::code_rule_dsl::language::adapter::value::{ValueFeature, ValueRules};
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
            pub(crate) use crate::code_rule_dsl::language::{ast_grep, parse_input};
        }
        #[allow(unused_imports)]
        #[cfg(test)]
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
    #[cfg_attr(test, path = "tape")]
    mod tape {
        pub use rerail_boot::code_rule_dsl::tape::*;
        pub mod resident {
            pub use rerail_boot::code_rule_dsl::tape::resident::*;
            #[cfg(test)]
            #[path = "tests.rs"]
            pub mod tests;
            #[allow(unused_imports)]
            use super::*;
        }
        #[cfg(test)]
        pub mod tests;
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
        pub(in crate::code_rule_dsl) use crate::code_rule_dsl::tape::index::IndexImage;
        #[allow(unused_imports)]
        pub(super) use crate::code_rule_dsl::tape::wire::index_span;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::tape::wire::{CHECKPOINT_STRIDE, WIDTH};
        #[cfg(test)]
            pub mod corpus_parity;
        #[cfg(test)]
        #[path = "tokens_tests.rs"]
        pub mod tokens_tests;
    }
    pub mod tree {
        pub use rerail_boot::code_rule_dsl::tree::*;
        #[cfg(test)]
        #[path = "tests.rs"]
        pub mod tests;
        #[allow(unused_imports)]
        use ast_grep_core::{Doc, Node};
        #[allow(unused_imports)]
        use std::borrow::Cow;
        #[allow(unused_imports)]
        use std::ops::Range;
    }
    #[cfg(test)]
    mod test_support;
    pub mod vue {
        pub use rerail_boot::code_rule_dsl::vue::*;
        pub mod view {
            pub use rerail_boot::code_rule_dsl::vue::view::*;
            #[cfg(test)]
                    pub mod tests;
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
        #[cfg(test)]
        #[path = "oracle_tests.rs"]
        pub mod oracle_tests;
        #[cfg(test)]
        #[path = "tests.rs"]
        pub mod tests;
    }
}
