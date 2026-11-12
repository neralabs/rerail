//! Python: its facts, scopes, values and suppressions.
//!
//! Its sources stay where they were (native/src): this crate mounts them in place, as the engine did.
#![allow(dead_code, unused_imports)]
// cratify: mounts (tools/cratify/mounts.py; generated, do not edit) begin
pub use rerail_store::content_store;
pub use rerail_store::git_workspace;
extern crate self as rerail_lang_python;
#[path = "../../../src/code_rule_dsl"]
pub mod code_rule_dsl {
    pub use rerail_lang::code_rule_dsl::budget;
    pub use rerail_lang::code_rule_dsl::build_ids;
    #[path = "calls"]
    pub mod calls {
        pub use rerail_w::code_rule_dsl::calls::*;
        #[path = "closure"]
        pub mod closure {
            pub use rerail_w::code_rule_dsl::calls::closure::*;
            pub mod lang_python;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::calls::names::Name;
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
        pub mod python;
        #[cfg(test)]
        pub mod python_tests;
        #[path = "summary"]
        pub mod summary {
            pub use rerail_w::code_rule_dsl::calls::summary::*;
            pub mod python;
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
    pub use rerail_w::code_rule_dsl::compile;
    pub mod cpython;
    pub use rerail_w::code_rule_dsl::crypto;
    pub use rerail_lang::code_rule_dsl::embed;
    pub use rerail_w::code_rule_dsl::eval;
    #[path = "facts"]
    pub mod facts {
        pub use rerail_facts::code_rule_dsl::facts::*;
        pub use rerail_w::code_rule_dsl::facts::facts_a;
        #[path = "producers"]
        pub mod producers {
            pub use rerail_facts::code_rule_dsl::facts::producers::*;
            pub mod python;
        }
        #[path = "vtab"]
        pub mod vtab {
            pub use rerail_facts::code_rule_dsl::facts::vtab::*;
            #[cfg(any(test, feature = "test-hooks"))]
            pub use rerail_w::code_rule_dsl::facts::vtab::p_table;
            pub mod python;
            #[path = "syntax"]
            pub mod syntax {
                pub use rerail_facts::code_rule_dsl::facts::vtab::syntax::*;
                pub mod annotation;
                #[cfg(any(test, feature = "test-hooks"))]
                pub mod candidate_spike;
                pub mod candidates;
                pub mod cpython;
                pub mod fstring;
                pub mod py_families;
                pub mod python_core;
                pub mod python_demand;
                pub mod python_parts;
                pub mod python_syntax;
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
    #[path = "findings"]
    pub mod findings {
        pub use rerail_w::code_rule_dsl::findings::*;
        #[cfg(any(test, feature = "test-hooks"))]
        #[path = "tests/async_reference.rs"]
        pub mod async_reference;
    }
    #[cfg(any(test, feature = "test-hooks"))]
    pub use rerail_lang::code_rule_dsl::fixtures;
    pub use rerail_w::code_rule_dsl::hooks;
    pub use rerail_w::code_rule_dsl::identifier;
    pub use rerail_lang::code_rule_dsl::json;
    #[path = "language"]
    pub mod language {
        pub use rerail_lang::code_rule_dsl::language::*;
        #[path = "adapter"]
        pub mod adapter {
            pub use rerail_w::code_rule_dsl::language::adapter::*;
            #[path = "arguments"]
            pub mod arguments {
                pub use rerail_w::code_rule_dsl::language::adapter::arguments::*;
                pub mod lang_python;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::language::adapter::{Refusal, Refused};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::matcher::{Arguments, Passed};
                #[allow(unused_imports)]
                use ast_grep_core::{Doc, Node};
                #[allow(unused_imports)]
                use std::borrow::Cow;
            }
            #[path = "calls"]
            pub mod calls {
                pub use rerail_w::code_rule_dsl::language::adapter::calls::*;
                pub mod lang_python;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::language::adapter::{Refusal, Refused};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::calls::closure::{Graph, Link};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::calls::names::Name;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::calls::summary::{Callee, SHADOWED, Summary};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::calls::{FileTables};
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
            #[path = "format_args"]
            pub mod format_args {
                pub use rerail_w::code_rule_dsl::language::adapter::format_args::*;
                pub mod lang_python;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::language::adapter::{Refusal, Refused};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::matcher::Style;
                #[allow(unused_imports)]
                use std::borrow::Cow;
            }
            pub mod python;
            #[path = "scope"]
            pub mod scope {
                pub use rerail_w::code_rule_dsl::language::adapter::scope::*;
                pub mod lang_python;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::language::adapter::{Adapter, Refusal};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::matcher::{ScopeName, Unresolved, file_module, file_source_type_for};
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
                use crate::code_rule_dsl::scope::{Access, Binding, BuildError, Builder, Directives, Form, Lookup, Reading, Reference, ScopeKind, ScopeTable, python};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::value::NodeRef;
                #[allow(unused_imports)]
                use ast_grep_core::{Doc, Node};
                #[allow(unused_imports)]
                use std::borrow::Cow;
                #[allow(unused_imports)]
                use std::sync::Arc;
            }
            #[path = "syntax"]
            pub mod syntax {
                pub use rerail_w::code_rule_dsl::language::adapter::syntax::*;
                pub mod lang_python;
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
            #[path = "value"]
            pub mod value {
                pub use rerail_w::code_rule_dsl::language::adapter::value::*;
                pub mod lang_python;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::language::adapter::Refusal;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::language::Family;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::matcher::SemanticText;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::value::structure::{Equality, Tok};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::value::{self, BigInt, Context, Ordered, TypeNames, Value};
                #[allow(unused_imports)]
                use ast_grep_core::{Doc, Node};
                #[allow(unused_imports)]
                use std::cmp::Ordering;
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
            use crate::code_rule_dsl::model::{ViolationSpec, python_module_path_on_disk};
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
    pub use rerail_lang::code_rule_dsl::lines;
    #[path = "literal"]
    pub mod literal {
        pub use rerail_w::code_rule_dsl::literal::*;
        pub mod annotation;
        pub mod brace;
        pub mod identifier_words;
        #[path = "offsets"]
        pub mod offsets {
            pub use rerail_w::code_rule_dsl::literal::offsets::*;
            #[cfg(test)]
            pub mod tests;
            #[allow(unused_imports)]
            use std::ops::Range;
        }
        pub mod percent;
        pub mod python_fstring;
        pub mod python_string;
        pub mod semantic;
        #[cfg(test)]
        pub mod tests;
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
    #[path = "logic"]
    pub mod logic {
        #[path = "front"]
        pub mod front {
            pub use rerail_rr_syntax::code_rule_dsl::logic::front::ast;
            pub use rerail_rr_syntax::code_rule_dsl::logic::front::name;
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, BTreeSet};
            #[allow(unused_imports)]
            use std::fmt;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::front::name::Name;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::front::ast::{AXIS, Again, AggOp, Atom, DESCENDANTS, EVERY_KIND, GLOBAL, LIBRARY, LatticeDecl, Literal, NEAREST, Native, OPTION_GLOB, Program, QUALIFIERS, RESERVED, Rule, SCOPE_NOT, SpanIn, Term, Value, axis, cmp_binds, globals, is_axis, is_transduced, join};
            #[allow(unused_imports)]
            pub(crate) use crate::code_rule_dsl::facts::core::PYTHON_CONTEXTS;
        }
        #[path = "plan"]
        pub mod plan {
            pub use rerail_facts::code_rule_dsl::logic::plan::sketch;
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
        }
        pub use rerail_prof::code_rule_dsl::logic::profile;
        pub use rerail_facts::code_rule_dsl::logic::strings;
    }
    #[path = "matcher"]
    pub mod matcher {
        pub use rerail_w::code_rule_dsl::matcher::*;
        #[path = "arguments"]
        pub mod arguments {
            pub use rerail_w::code_rule_dsl::matcher::arguments::*;
            pub mod lang_python;
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
            use crate::code_rule_dsl::value::{python};
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
        #[path = "format_args"]
        pub mod format_args {
            pub use rerail_w::code_rule_dsl::matcher::format_args::*;
            pub mod python;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::tree::Tree;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::AstMatcher;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::each::{self, HostRanges};
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
        pub mod python_facts;
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        pub(in crate::code_rule_dsl) use crate::code_rule_dsl::matcher::p_anchors::sql::{Corpus, CorpusSlot};
        #[allow(unused_imports)]
        #[cfg(all(any(test, feature = "test-hooks"), feature = "p-anchor-corpus"))]
        pub(in crate::code_rule_dsl) use crate::code_rule_dsl::matcher::p_anchors::sql::{anchor_plans, anchor_plans_for_file};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::budget;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::{GrammarDialect, SourceType};
        #[allow(unused_imports)]
        pub(super) use crate::code_rule_dsl::matcher::analysis::RequiredText;
        #[allow(unused_imports)]
        #[cfg_attr(not(test), allow(unused_imports))]
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
        #[cfg(any(test, feature = "test-hooks"))]
        pub(crate) use crate::code_rule_dsl::matcher::nth_child::{named_sibling_visits, reset_named_sibling_visits};
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        pub(crate) use crate::code_rule_dsl::matcher::parse::parse_fact_tree;
        #[allow(unused_imports)]
        pub(crate) use crate::code_rule_dsl::matcher::parse::{HostTest, ParseTest};
        #[allow(unused_imports)]
        use regex::Regex;
        #[allow(unused_imports)]
        pub(super) use crate::code_rule_dsl::matcher::relation::{FieldRelation, Related, StopBy, stop_rule};
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
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
        #[cfg(any(test, feature = "test-hooks"))]
        pub(super) use crate::code_rule_dsl::matcher::resolve::{JSX_RUNTIME_DERIVATIONS, SOURCE_TYPE_DERIVATIONS, TABLE_BUILDS};
        #[allow(unused_imports)]
        pub(super) use crate::code_rule_dsl::matcher::run::{Stopped, enumerate, node_at, run_clause};
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
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
        #[cfg(any(test, feature = "test-hooks"))]
        pub(crate) use crate::code_rule_dsl::matcher::transform::{
            line_table_constructions, note_line_table_construction, reset_line_table_constructions,
        };
        #[allow(unused_imports)]
        pub(super) use crate::code_rule_dsl::matcher::util::{
            CallArgs, MAX_UTIL_DEPTH, MAX_UTIL_INSTANCES, UtilCall, UtilLimit, UtilOrigin, UtilRef,
            UtilTable, UtilitySummary,
        };
        #[allow(unused_imports)]
        use crate::code_rule_dsl::matcher::util::{MAX_UTIL_STACK_BYTES, UTIL_EXCEEDED, UTIL_STEPS_LEFT};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::matcher::util::{UNKNOWN};
        #[allow(unused_imports)]
        pub(super) use crate::code_rule_dsl::matcher::value::{Bound, Reading, ValueTest};
        #[allow(unused_imports)]
        pub(crate) use crate::code_rule_dsl::matcher::value::Values;
        #[allow(unused_imports)]
        pub(crate) use crate::code_rule_dsl::matcher::term::{LanguageTerm, TermCompiler, install_terms};
    }
    pub use rerail_w::code_rule_dsl::model;
    pub use rerail_lang::code_rule_dsl::module_path;
    pub use rerail_w::code_rule_dsl::origin;
    pub use rerail_prof::code_rule_dsl::phase_clock;
    pub use rerail_w::code_rule_dsl::placement;
    pub use rerail_facts::code_rule_dsl::producers;
    pub use rerail_prof::code_rule_dsl::prof;
    #[cfg(feature="code-rule-profile")]
    pub use rerail_w::code_rule_dsl::profile;
    pub use rerail_w::code_rule_dsl::query;
    #[path = "residency"]
    pub mod residency {
        #[path = "admission"]
        pub mod admission {
            pub use rerail_w::code_rule_dsl::residency::admission::*;
            #[path = "relational"]
            pub mod relational {
                pub use rerail_w::code_rule_dsl::residency::admission::relational::*;
                pub mod python_ast;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::eval::{evidence::Rows, failure, violation_from_witness};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::json::{hex_digest, invalid};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::model::{
                    CompiledClause, CompiledCodeRule, Evaluation, FactQuery, Snapshot,
                };
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
        #[allow(unused_imports)]
        pub(in crate::code_rule_dsl) use crate::code_rule_dsl::residency::admission::relational::ast_stage::install_ast_producer;
    }
    pub use rerail_w::code_rule_dsl::resolution;
    pub use rerail_w::code_rule_dsl::rule_api;
    #[path = "scope"]
    pub mod scope {
        pub use rerail_w::code_rule_dsl::scope::*;
        pub mod python;
        #[path = "usage"]
        pub mod usage {
            pub use rerail_w::code_rule_dsl::scope::usage::*;
            pub mod lang_python;
            #[cfg(test)]
            pub mod tests;
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
        pub(crate) use crate::code_rule_dsl::scope::lookup::Lookup;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::identifier::{name_in, nfkc};
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
    }
    pub use rerail_facts::code_rule_dsl::search_index;
    pub use rerail_lang::code_rule_dsl::semver;
    pub use rerail_w::code_rule_dsl::soundness;
    pub use rerail_facts::code_rule_dsl::sql_profile;
    pub use rerail_w::code_rule_dsl::stats;
    #[path = "suppress"]
    pub mod suppress {
        pub use rerail_w::code_rule_dsl::suppress::*;
        pub mod noqa;
        pub mod nosec;
        #[allow(unused_imports)]
        use ast_grep_core::{Doc, Node};
        #[allow(unused_imports)]
        pub(crate) use crate::code_rule_dsl::suppress::rerail::RerailIgnores;
        #[allow(unused_imports)]
        pub(crate) use crate::code_rule_dsl::suppress::kinds::*;
    }
    #[path = "tape"]
    pub mod tape {
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
        #[allow(unused_imports)]
        pub(in crate::code_rule_dsl) use crate::code_rule_dsl::tape::index::IndexImage;
        #[allow(unused_imports)]
        pub(super) use crate::code_rule_dsl::tape::wire::index_span;
    }
    pub use rerail_facts::code_rule_dsl::text;
    #[path = "transducer"]
    pub mod transducer {
        pub use rerail_w::code_rule_dsl::transducer::semantic_facts;
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
    pub use rerail_lang::code_rule_dsl::tree;
    #[path = "value"]
    pub mod value {
        pub use rerail_w::code_rule_dsl::value::*;
        pub mod python;
        pub mod python_ast;
        pub mod python_structure;
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
    }
    pub use rerail_w::code_rule_dsl::violations;
    #[cfg(any(test, feature = "test-hooks"))]
    pub use rerail_w::code_rule_dsl::walks;
    #[path = "waves"]
    pub mod waves {
        #[cfg(any(test, feature = "test-hooks"))]
        pub use rerail_w::code_rule_dsl::waves::relational;
    }
    pub use rerail_w::code_rule_dsl::workspace;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::residency::admission::relational;
    #[allow(unused_imports)]
    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) use crate::code_rule_dsl::fixtures::{fixtures_dir, oracle};
    #[allow(unused_imports)]
    use crate::code_rule_dsl::compile::{compile_document, parse_capabilities, parse_document};
    #[allow(unused_imports)]
    use crate::code_rule_dsl::json::{canonicalize_value, hex_digest, invalid, json_error};
    #[allow(unused_imports)]
    use crate::code_rule_dsl::model::CompiledCodeRule;
    #[allow(unused_imports)]
    use napi::Result;
    #[allow(unused_imports)]
    use napi_derive::napi;
    #[allow(unused_imports)]
    use serde_json::Value;
    #[allow(unused_imports)]
    use std::sync::Arc;
    #[allow(unused_imports)]
    pub use crate::code_rule_dsl::rule_api::CompiledCodeRuleDsl;
    #[allow(unused_imports)]
    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) use crate::code_rule_dsl::rule_api::CodeRuleDslEditor;
}
pub use rerail_w::git_ignore;
pub use rerail_w::panic_boundary;
// cratify: mounts end
