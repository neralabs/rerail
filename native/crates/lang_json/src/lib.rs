//! JSON: its Bitlex parser.
//!
//! Its sources stay where they were (native/src): this crate mounts them in place, as the engine did.
#![allow(dead_code, unused_imports)]
// cratify: mounts (tools/cratify/mounts.py; generated, do not edit) begin
pub use rerail_store::content_store;
pub use rerail_store::git_workspace;
extern crate self as rerail_lang_json;
#[path = "../../../src/code_rule_dsl"]
pub mod code_rule_dsl {
    pub use rerail_lang::code_rule_dsl::budget;
    pub use rerail_lang::code_rule_dsl::build_ids;
    pub use rerail_w::code_rule_dsl::calls;
    pub use rerail_w::code_rule_dsl::compile;
    pub use rerail_w::code_rule_dsl::crypto;
    pub use rerail_transducer::code_rule_dsl::demand;
    pub use rerail_lang::code_rule_dsl::embed;
    pub use rerail_w::code_rule_dsl::eval;
    #[path = "facts"]
    pub mod facts {
        pub use rerail_facts::code_rule_dsl::facts::*;
        pub use rerail_w::code_rule_dsl::facts::facts_a;
        #[path = "vtab"]
        pub mod vtab {
            pub use rerail_facts::code_rule_dsl::facts::vtab::*;
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
    pub use rerail_lang::code_rule_dsl::json;
    #[path = "language"]
    pub mod language {
        pub use rerail_lang::code_rule_dsl::language::*;
        #[path = "adapter"]
        pub mod adapter {
            pub use rerail_w::code_rule_dsl::language::adapter::*;
            pub mod json;
            #[allow(unused_imports)]
            pub(crate) use crate::code_rule_dsl::language::adapter::arguments::{ArgumentFeature, ArgumentRules};
            #[allow(unused_imports)]
            pub(crate) use crate::code_rule_dsl::language::adapter::calls::{CallFeature, CallRules};
            #[allow(unused_imports)]
            pub(crate) use crate::code_rule_dsl::language::adapter::format_args::{FormatArgFeature, FormatArgRules};
            #[allow(unused_imports)]
            #[cfg(any(test, feature = "test-hooks"))]
            #[cfg_attr(not(feature = "lang-json"), allow(unused_imports))]
            pub(crate) use crate::code_rule_dsl::language::adapter::json::Json;
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
    pub use rerail_w::code_rule_dsl::literal;
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
    pub use rerail_lang::code_rule_dsl::module_path;
    pub use rerail_w::code_rule_dsl::origin;
    pub use rerail_prof::code_rule_dsl::phase_clock;
    pub use rerail_w::code_rule_dsl::placement;
    pub use rerail_facts::code_rule_dsl::producers;
    pub use rerail_prof::code_rule_dsl::prof;
    #[cfg(feature="code-rule-profile")]
    pub use rerail_w::code_rule_dsl::profile;
    pub use rerail_w::code_rule_dsl::query;
    pub use rerail_transducer::code_rule_dsl::residency;
    pub use rerail_w::code_rule_dsl::resolution;
    pub use rerail_w::code_rule_dsl::rule_api;
    pub use rerail_w::code_rule_dsl::scope;
    pub use rerail_facts::code_rule_dsl::search_index;
    pub use rerail_lang::code_rule_dsl::semver;
    pub use rerail_w::code_rule_dsl::soundness;
    pub use rerail_transducer::code_rule_dsl::space;
    pub use rerail_facts::code_rule_dsl::sql_profile;
    pub use rerail_w::code_rule_dsl::stats;
    pub use rerail_w::code_rule_dsl::suppress;
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
        pub use rerail_transducer::code_rule_dsl::transducer::*;
        #[path = "packed"]
        pub mod packed {
            pub use rerail_transducer::code_rule_dsl::transducer::packed::*;
            pub mod json_parser;
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
            #[allow(unused_imports)]
            #[cfg(any(test, feature = "test-hooks"))]
            #[allow(unused_imports)]
            use crate::code_rule_dsl::transducer::packed::literal_anchors::*;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::device::{RootJobs, RootsFound};
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::compile_matcher::{Backend as MatcherBackend, LoweredMatcher, LoweredWave, WaveRun};
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::compile_micro::{Emit, Frames, Micro, RootRecord, steps as micro_steps};
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::compilation::PlanId;
            #[allow(unused_imports)]
            use rerail_compiler::native_cpu_codegen;
            #[allow(unused_imports)]
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use rerail_compiler::native_cpu_rows;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::packed_batch::PackedExecution;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use rerail_compiler::block;
            #[allow(unused_imports)]
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use rerail_rr_parser as rr_parser;
            #[allow(unused_imports)]
            use rerail_compiler::program::*;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use rerail_compiler::program::{NAMED, Words, word};
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use rerail_compiler::Error;
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
    pub use rerail_lang::code_rule_dsl::tree;
    pub use rerail_w::code_rule_dsl::value;
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
