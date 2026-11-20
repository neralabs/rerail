//! Metal, as the logic's and the transducer's device (macOS only; registered by the engine).
//!
//! Its sources stay where they were (native/src): this crate mounts them in place, as the engine did.
// A mounted module re-exports its dependency's by glob, and its sources import the same items
// under the same names: the glob's name and the private one are one item. The mounts also widen what the crates
// above read (the split's facades, not a design).
#![allow(dead_code, unused_imports, hidden_glob_reexports, private_interfaces, private_bounds)]
// What the split widened for the crates above is no public API (`len`, `()` errors, `new`); the long
// signatures and tuple types are the sources' own.
#![allow(clippy::len_without_is_empty, clippy::result_unit_err, clippy::new_without_default)]
#![allow(clippy::type_complexity, clippy::too_many_arguments)]
// The mounts' `#[path]`s name the directories of the Metal modules they hold, which only macOS builds.
#![cfg_attr(not(target_os = "macos"), allow(unused_attributes))]
// The browser build has no JIT: wasm/cranelift-jit stands in under the same name (wasm/README.md).
#[cfg(target_family = "wasm")]
extern crate cranelift_jit_wasm as cranelift_jit;
// cratify: mounts (tools/cratify/mounts.py; generated, do not edit) begin
pub use rerail_store::content_store;
pub use rerail_store::git_access;
pub use rerail_store::git_workspace;
extern crate self as rerail_device_metal;
#[path = "../../../src/code_rule_dsl"]
pub mod code_rule_dsl {
    pub use rerail_lang::code_rule_dsl::budget;
    pub use rerail_lang::code_rule_dsl::build_ids;
    pub use rerail_w::code_rule_dsl::calls;
    pub use rerail_w::code_rule_dsl::compile;
    pub use rerail_w::code_rule_dsl::crypto;
    pub use rerail_transducer::code_rule_dsl::demand;
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub mod device_metal;
    pub use rerail_lang::code_rule_dsl::embed;
    pub use rerail_w::code_rule_dsl::eval;
    
    pub mod facts {
        pub use rerail_facts::code_rule_dsl::facts::*;
        pub use rerail_w::code_rule_dsl::facts::facts_a;
        pub mod vtab {
            pub use rerail_facts::code_rule_dsl::facts::vtab::*;
            #[cfg(any(test, feature = "test-hooks"))]
            pub use rerail_w::code_rule_dsl::facts::vtab::p_table;
            pub mod syntax {
                pub use rerail_facts::code_rule_dsl::facts::vtab::syntax::*;
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
    pub use rerail_w::code_rule_dsl::findings;
    #[cfg(any(test, feature = "test-hooks"))]
    pub use rerail_lang::code_rule_dsl::fixtures;
    pub use rerail_w::code_rule_dsl::hooks;
    pub use rerail_w::code_rule_dsl::identifier;
    pub use rerail_lang::code_rule_dsl::json;
    pub use rerail_lang::code_rule_dsl::regexes;
    pub mod language {
        pub use rerail_lang::code_rule_dsl::language::*;
        pub use rerail_w::code_rule_dsl::language::adapter;
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
    pub use rerail_w::code_rule_dsl::literal;
    #[cfg_attr(target_os = "macos", path = "logic")]
    pub mod logic {
        pub use rerail_logic::code_rule_dsl::logic::*;
        pub mod front {
            pub use rerail_rr::code_rule_dsl::logic::front::*;
            pub mod caches {
                pub use rerail_rr::code_rule_dsl::logic::front::caches::*;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::logic::front::Error;
                #[allow(unused_imports)]
                use std::collections::BTreeSet;
            }
            pub mod yaml {
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
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::front::name::Name;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::front::ast::{AXIS, AggOp, DESCENDANTS, EVERY_KIND, GLOBAL, LIBRARY, LatticeDecl, NEAREST, Native, OPTION_GLOB, QUALIFIERS, RESERVED, SCOPE_NOT, SpanIn, Value, axis, globals, is_axis, is_transduced};
            #[allow(unused_imports)]
            pub(crate) use crate::code_rule_dsl::facts::core::PYTHON_CONTEXTS;
        }
        #[cfg_attr(target_os = "macos", path = "native")]
        pub mod native {
            pub use rerail_logic::code_rule_dsl::logic::native::*;
            #[cfg_attr(target_os = "macos", path = "setfix")]
            pub mod setfix {
                pub use rerail_logic::code_rule_dsl::logic::native::setfix::*;
                #[cfg(any(target_os = "macos", target_os = "linux"))]
                pub mod device;
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
                pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::device::{Report as DeviceReport, take_report as device_report};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::logic::device::{JoinPlan};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::logic::profile::{Unit, work as charge};
            }
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::native::setfix::{forget_engine, forget_scope};
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::native::setfix::{flush_symbols, forget_inputs, intern_batch, keep_inputs, register_symbols, sym_text_of};
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::native::setfix::symbol_text;
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
            pub mod corpus {
                pub use rerail_logic::code_rule_dsl::logic::plan::corpus::*;
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
                pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::plan::corpus::hiding::qualified;
                #[allow(unused_imports)]
                pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::plan::corpus::inputs::parameters as input_parameters;
                #[allow(unused_imports)]
                pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::plan::corpus::matchers::columns as ast_columns;
                #[allow(unused_imports)]
                pub(crate) use crate::code_rule_dsl::logic::plan::corpus::typing::{Typing, TypingReport};
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
                pub(crate) use crate::code_rule_dsl::prof::thread_usage;
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
            pub use rerail_logic::code_rule_dsl::logic::profile::report;
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
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::profile::roofline::Unit;
        }
        pub mod rr {
            pub use rerail_logic::code_rule_dsl::logic::rr::*;
            pub mod aggregate_demand {
                pub use rerail_rr::code_rule_dsl::logic::rr::aggregate_demand::*;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::logic::front::{self, AggOp};
                #[allow(unused_imports)]
                use std::collections::{BTreeMap, BTreeSet};
            }
            pub mod tape_type {
                pub use rerail_rr::code_rule_dsl::logic::rr::tape_type::*;
                #[allow(unused_imports)]
                use std::collections::{BTreeMap, BTreeSet};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::logic::front::Error;
            }
            #[allow(unused_imports)]
            use rerail_rr_syntax::syntax;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use rerail_rr_syntax::tape;
            #[allow(unused_imports)]
            use std::collections::BTreeSet;
            #[allow(unused_imports)]
            use std::path::{Path, PathBuf};
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::front::Error;
            #[allow(unused_imports)]
            #[cfg(any(test, feature = "test-hooks"))]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::rr::lower::drop_unread_core;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::rr::lower::is_core;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::rr::lower::WORLD;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use syntax::quote;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::rr::translate::translate;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::facts::vtab::syntax::core_rr::single_metavars;
        }
        pub mod strings {
            pub use rerail_facts::code_rule_dsl::logic::strings::*;
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
    #[cfg_attr(target_os = "macos", path = "residency")]
    pub mod residency {
        pub use rerail_transducer::code_rule_dsl::residency::*;
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        pub mod slabs;
        #[allow(unused_imports)]
        pub(in crate::code_rule_dsl) use crate::code_rule_dsl::residency::admission::relational::ast_stage::install_ast_producer;
    }
    pub use rerail_w::code_rule_dsl::resolution;
    pub use rerail_w::code_rule_dsl::rule_api;
    pub use rerail_w::code_rule_dsl::scope;
    pub use rerail_facts::code_rule_dsl::search_index;
    pub use rerail_lang::code_rule_dsl::semver;
    pub use rerail_w::code_rule_dsl::soundness;
    pub use rerail_transducer::code_rule_dsl::space;
    pub use rerail_facts::code_rule_dsl::sql_profile;
    pub use rerail_facts::code_rule_dsl::sql_admission;
    pub use rerail_w::code_rule_dsl::stats;
    pub use rerail_w::code_rule_dsl::suppress;
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
        pub(in crate::code_rule_dsl) use crate::code_rule_dsl::tape::index::IndexImage;
        #[allow(unused_imports)]
        pub(super) use crate::code_rule_dsl::tape::wire::index_span;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::tape::wire::{CHECKPOINT_STRIDE, WIDTH};
    }
    pub use rerail_facts::code_rule_dsl::text;
    #[cfg_attr(target_os = "macos", path = "transducer")]
    pub mod transducer {
        pub use rerail_transducer::code_rule_dsl::transducer::*;
        #[cfg_attr(target_os = "macos", path = "packed")]
        pub mod packed {
            pub use rerail_transducer::code_rule_dsl::transducer::packed::*;
            #[cfg(test)]
            pub mod block_rows_tests;
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            #[path = "verify/bounded_repeat_proof.rs"]
            pub mod bounded_repeat_proof;
            #[path = "compile"]
            pub mod compile_matcher {
                pub use rerail_transducer::code_rule_dsl::transducer::packed::compile_matcher::*;
                #[cfg(any(target_os = "macos", target_os = "linux"))]
                pub mod matcher_metal;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::transducer::packed::*;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::matcher::{Axis, MatchTree, Stop};
                #[allow(unused_imports)]
                use std::collections::{BTreeMap, BTreeSet, HashMap};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::tape::DERIVED_CLASS_BASE as LABEL_BASE;
            }
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            pub mod metal_backend;
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            pub mod metal_join;
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            pub mod metal_program_backend;
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            pub mod metal_rows;
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            pub mod metal_sort;
            #[cfg(all(any(test, feature = "test-hooks"),target_os="macos",target_arch="aarch64"))]
            pub mod regex_relocation_tests;
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
            use crate::code_rule_dsl::transducer::packed::literal_anchors::*;
            #[allow(unused_imports)]
            #[cfg(all(any(test, feature = "test-hooks"), any(target_os = "macos", target_os = "linux")))]
            pub(super) use crate::code_rule_dsl::transducer::packed::metal_backend::launch::LaunchScope as MetalTestScope;
            #[allow(unused_imports)]
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::metal_program_backend::set_steps;
            #[allow(unused_imports)]
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::metal_program_backend::lattice as metal_lattice;
            #[allow(unused_imports)]
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::metal_program_backend::nearest as metal_nearest;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::device::{RootJobs, RootsFound};
            #[allow(unused_imports)]
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::compile_matcher::matcher_metal;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::compile_matcher::{Backend as MatcherBackend, LoweredMatcher, LoweredWave, WaveRun};
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::compile_micro::{Emit, Frames, Micro, RootRecord, steps as micro_steps};
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::compilation::PlanId;
            #[allow(unused_imports)]
            use rerail_compiler::native_cpu_codegen;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use rerail_compiler::native_cpu_rows;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::packed_batch::PackedExecution;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use rerail_compiler::block;
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
    #[cfg_attr(any(test, feature = "test-hooks"), path = "typing")]
    pub mod typing {
        pub use rerail_logic::code_rule_dsl::typing::*;
        #[cfg_attr(any(test, feature = "test-hooks"), path = "file_local")]
        pub mod file_local {
            pub use rerail_logic::code_rule_dsl::typing::file_local::*;
            #[cfg(any(test, feature = "test-hooks"))]
            pub mod tests;
            #[allow(unused_imports)]
            use crate::content_store::{StoreError, StoreResult};
            #[allow(unused_imports)]
            pub(crate) use crate::code_rule_dsl::facts::nearest::{Binding, NONE, Trees};
            #[allow(unused_imports)]
            pub(crate) use crate::code_rule_dsl::logic::front::document_column;
            #[allow(unused_imports)]
            use std::collections::BTreeMap;
        }
    }
    pub use rerail_w::code_rule_dsl::value;
    pub use rerail_w::code_rule_dsl::violations;
    #[cfg(any(test, feature = "test-hooks"))]
    pub use rerail_w::code_rule_dsl::walks;
    pub mod waves {
        pub use rerail_logic::code_rule_dsl::waves::owner;
        #[cfg(any(test, feature = "test-hooks"))]
        pub use rerail_w::code_rule_dsl::waves::relational;
        pub use rerail_logic::code_rule_dsl::waves::schedule;
        pub use rerail_logic::code_rule_dsl::waves::storage;
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
pub use rerail_logic::threads;
// cratify: mounts end

/// The language packs this crate's tests read through the load-time registry (`language::base`), registered once:
/// the engine registers them as it loads (`install_languages`), and a test of this crate runs without it (work
/// queue Q23). The first registration holds, so a test may call it at any point. `.rr` text is read too (the parity
/// cases' programs, on the CPU pool's workers): without its pack a worker panicked, intermittently, by which test ran
/// first. And Python's embedder, as the engine's load registers it.
#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
pub(crate) fn test_languages() {
    use crate::code_rule_dsl::language::{self, Family, adapter::pack::{Pack, register}};
    use crate::code_rule_dsl::language::adapter::rr::Rr;
    use rerail_lang_python::code_rule_dsl::language::adapter::python::Python;
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        register(Family::Python, &Pack::<Python>::NEW);
        language::register_base(Family::Python, &Pack::<Python>::NEW);
        register(Family::Rr, &Pack::<Rr>::NEW);
        language::register_base(Family::Rr, &Pack::<Rr>::NEW);
        // Python's strings embed code at the places it declares (decision 0470): a Python file's tape reads its
        // embedder, which the engine registers with the rest as it loads.
        crate::code_rule_dsl::embed::install_embedders(vec![(language::Via::CallArgument, &rerail_lang_python::code_rule_dsl::embed::call_argument::CallArgument)]);
    });
}
