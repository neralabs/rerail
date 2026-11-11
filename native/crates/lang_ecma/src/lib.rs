//! JavaScript, TypeScript and Vue: manifests, imports, values.
//!
//! Its sources stay where they were (native/src): this crate mounts them in place, as the engine did.
#![allow(dead_code, unused_imports)]
// cratify: mounts (tools/cratify/mounts.py; generated, do not edit) begin
pub use rerail_store::content_store;
pub use rerail_store::git_workspace;
extern crate self as rerail_lang_ecma;
#[path = "../../../src/code_rule_dsl"]
pub mod code_rule_dsl {
    pub use rerail_lang::code_rule_dsl::budget;
    pub use rerail_lang::code_rule_dsl::build_ids;
    pub use rerail_w::code_rule_dsl::calls;
    #[path = "compile"]
    pub mod compile {
        pub use rerail_w::code_rule_dsl::compile::*;
        #[path = "binds"]
        pub mod binds {
            pub use rerail_w::code_rule_dsl::compile::binds::*;
            // ESLint's binds lab (`eslint.rs`) reads the engine's oracles and test kit: the engine mounts it.
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
        }
        pub mod import_target;
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
    }
    pub use rerail_w::code_rule_dsl::crypto;
    #[path = "embed"]
    pub mod embed {
        pub use rerail_lang::code_rule_dsl::embed::*;
        pub mod script_block;
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        pub(crate) use crate::code_rule_dsl::embed::test_script::TestScript;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::GrammarDialect;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::embed::segments::SegmentMap;
        #[allow(unused_imports)]
        use std::ops::Range;
        #[allow(unused_imports)]
        use tree_sitter::Node as TsNode;
    }
    #[path = "eval"]
    pub mod eval {
        pub use rerail_w::code_rule_dsl::eval::*;
        pub mod ecma;
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
        #[cfg(any(test, feature = "test-hooks"))]
        pub(super) use crate::code_rule_dsl::eval::query::{reset_shared_group_lookups, shared_group_lookups};
        #[allow(unused_imports)]
        use serde_json::Value;
        #[allow(unused_imports)]
        use std::collections::{BTreeMap, HashSet};
        #[allow(unused_imports)]
        use std::sync::Arc;
    }
    #[path = "facts"]
    pub mod facts {
        pub use rerail_facts::code_rule_dsl::facts::*;
        pub use rerail_w::code_rule_dsl::facts::facts_a;
        #[path = "vtab"]
        pub mod vtab {
            pub use rerail_facts::code_rule_dsl::facts::vtab::*;
            #[path = "syntax"]
            pub mod syntax {
                pub use rerail_facts::code_rule_dsl::facts::vtab::syntax::*;
                pub mod ecma_core;
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
    pub use rerail_w::code_rule_dsl::findings;
    #[cfg(any(test, feature = "test-hooks"))]
    pub use rerail_lang::code_rule_dsl::fixtures;
    pub use rerail_w::code_rule_dsl::hooks;
    pub use rerail_w::code_rule_dsl::identifier;
    pub mod import_target;
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
                pub mod lang_ecma;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::language::adapter::{Refusal, Refused};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::matcher::{Arguments, Passed};
                #[allow(unused_imports)]
                use ast_grep_core::{Doc, Node};
                #[allow(unused_imports)]
                use std::borrow::Cow;
            }
            pub mod ecma;
            #[path = "scope"]
            pub mod scope {
                pub use rerail_w::code_rule_dsl::language::adapter::scope::*;
                pub mod lang_ecma;
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
                use crate::code_rule_dsl::scope::{Access, Binding, BuildError, Builder, Directives, Form, Lookup, Reading, Reference, ScopeKind, ScopeTable, ecma};
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
                pub mod lang_ecma;
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
                pub mod lang_ecma;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::language::adapter::Refusal;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::language::Family;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::matcher::SemanticText;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::value::structure::{Equality, Tok};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::value::{self, BigInt, Context, Ordered, TypeNames, Value, key};
                #[allow(unused_imports)]
                use ast_grep_core::{Doc, Node};
                #[allow(unused_imports)]
                use std::cmp::Ordering;
            }
            pub mod vue;
            #[allow(unused_imports)]
            pub(crate) use crate::code_rule_dsl::language::adapter::arguments::{ArgumentFeature, ArgumentRules};
            #[allow(unused_imports)]
            pub(crate) use crate::code_rule_dsl::language::adapter::calls::{CallFeature, CallRules};
            #[allow(unused_imports)]
            pub(crate) use crate::code_rule_dsl::language::adapter::format_args::{FormatArgFeature, FormatArgRules};
            #[allow(unused_imports)]
            #[cfg(any(test, feature = "test-hooks"))]
            #[cfg_attr(not(feature = "lang-rr"), allow(unused_imports))]
            pub(crate) use crate::code_rule_dsl::language::adapter::rr::Rr;
            #[allow(unused_imports)]
            pub(crate) use crate::code_rule_dsl::language::adapter::scope::{ScopeFeature, ScopeRules};
            #[allow(unused_imports)]
            pub(crate) use crate::code_rule_dsl::language::adapter::syntax::{QueryWhen, SyntaxFeature, SyntaxRules};
            #[allow(unused_imports)]
            pub(crate) use crate::code_rule_dsl::language::adapter::value::{ValueFeature, ValueRules};
            #[allow(unused_imports)]
            #[cfg(any(test, feature = "test-hooks"))]
            #[cfg_attr(not(feature = "lang-vue"), allow(unused_imports))]
            pub(crate) use crate::code_rule_dsl::language::adapter::vue::Vue;
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
            #[cfg(any(test, feature = "test-hooks"))]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::language::adapter::ecma::{Ecma};
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
    #[path = "lines"]
    pub mod lines {
        pub use rerail_lang::code_rule_dsl::lines::*;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::Family;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::GrammarDialect;
        #[allow(unused_imports)]
        use serde::{Deserialize, Serialize};
    }
    #[path = "literal"]
    pub mod literal {
        pub use rerail_w::code_rule_dsl::literal::*;
        pub mod eslint_directive;
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
    pub use rerail_w::code_rule_dsl::matcher;
    pub use rerail_w::code_rule_dsl::model;
    pub mod module_graph;
    pub use rerail_lang::code_rule_dsl::module_path;
    pub mod npm_ignore;
    pub use rerail_w::code_rule_dsl::origin;
    pub mod package_manifest;
    pub use rerail_prof::code_rule_dsl::phase_clock;
    pub use rerail_w::code_rule_dsl::placement;
    pub use rerail_facts::code_rule_dsl::producers;
    pub use rerail_prof::code_rule_dsl::prof;
    #[cfg(feature="code-rule-profile")]
    pub use rerail_w::code_rule_dsl::profile;
    pub use rerail_w::code_rule_dsl::query;
    #[path = "residency"]
    pub mod residency {
        pub use rerail_w::code_rule_dsl::residency::admission;
        #[allow(unused_imports)]
        pub(in crate::code_rule_dsl) use crate::code_rule_dsl::residency::admission::relational::ast_stage::install_ast_producer;
    }
    pub use rerail_w::code_rule_dsl::resolution;
    pub use rerail_w::code_rule_dsl::rule_api;
    #[path = "scope"]
    pub mod scope {
        pub use rerail_w::code_rule_dsl::scope::*;
        pub mod ecma;
        #[path = "usage"]
        pub mod usage {
            pub use rerail_w::code_rule_dsl::scope::usage::*;
            pub mod ecma;
            pub mod lang_ecma;
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
    }
    pub use rerail_facts::code_rule_dsl::search_index;
    pub use rerail_lang::code_rule_dsl::semver;
    pub use rerail_w::code_rule_dsl::soundness;
    pub use rerail_facts::code_rule_dsl::sql_profile;
    pub use rerail_w::code_rule_dsl::stats;
    #[path = "suppress"]
    pub mod suppress {
        pub use rerail_w::code_rule_dsl::suppress::*;
        pub mod biome;
        pub mod eslint;
        pub mod oxlint;
        pub mod vue;
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
        #[allow(unused_imports)]
        use crate::code_rule_dsl::tape::wire::{CHECKPOINT_STRIDE, WIDTH};
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
        pub mod ecma;
        #[cfg(test)]
        pub mod eslint_rules;
        pub mod key;
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
    pub mod vue;
    #[cfg(any(test, feature = "test-hooks"))]
    pub use rerail_w::code_rule_dsl::walks;
    #[path = "waves"]
    pub mod waves {
        #[cfg(any(test, feature = "test-hooks"))]
        pub use rerail_w::code_rule_dsl::waves::relational;
    }
    #[path = "workspace"]
    pub mod workspace {
        pub use rerail_w::code_rule_dsl::workspace::*;
        #[path = "ingest"]
        pub mod ingest {
            pub use rerail_w::code_rule_dsl::workspace::ingest::*;
            pub mod ecma;
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
