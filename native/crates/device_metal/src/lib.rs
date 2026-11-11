//! Metal, as the logic's and the transducer's device (macOS only; registered by the engine).
//!
//! Its sources stay where they were (native/src): this crate mounts them in place, as the engine did.
#![allow(dead_code, unused_imports)]
// cratify: mounts (tools/cratify/mounts.py; generated, do not edit) begin
pub use rerail_store::content_store;
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
    #[cfg(target_os="macos")]
    pub mod device_metal;
    pub use rerail_lang::code_rule_dsl::embed;
    pub use rerail_w::code_rule_dsl::eval;
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
    #[path = "language"]
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
        pub use rerail_logic::code_rule_dsl::logic::*;
        #[path = "front"]
        pub mod front {
            pub use rerail_rr::code_rule_dsl::logic::front::*;
            #[path = "caches"]
            pub mod caches {
                pub use rerail_rr::code_rule_dsl::logic::front::caches::*;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::logic::front::{Error, Literal, Program, Term};
                #[allow(unused_imports)]
                use std::collections::BTreeSet;
            }
            #[path = "yaml"]
            pub mod yaml {
                pub use rerail_rr::code_rule_dsl::logic::front::yaml::*;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::logic::front::{Atom, Error, Literal, Program, Rule, Term};
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
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::front::ast::{AXIS, Again, AggOp, Atom, DESCENDANTS, EVERY_KIND, GLOBAL, LIBRARY, LatticeDecl, Literal, NEAREST, Native, OPTION_GLOB, Program, QUALIFIERS, RESERVED, Rule, SCOPE_NOT, SpanIn, Term, Value, axis, cmp_binds, globals, is_axis, is_transduced, join};
            #[allow(unused_imports)]
            pub(crate) use crate::code_rule_dsl::facts::core::PYTHON_CONTEXTS;
        }
        #[path = "magic"]
        pub mod magic {
            pub use rerail_rr::code_rule_dsl::logic::magic::*;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::logic::front::{AggOp, Atom, Error, Literal, Program, Rule, Term, Value, cmp_binds, join, strata, tarjan};
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, BTreeSet};
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::front::ast::{Gate, Report};
        }
        #[path = "native"]
        pub mod native {
            pub use rerail_logic::code_rule_dsl::logic::native::*;
            #[path = "setfix"]
            pub mod setfix {
                pub use rerail_logic::code_rule_dsl::logic::native::setfix::*;
                #[cfg(target_os="macos")]
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
            #[cfg(any(test, feature = "test-hooks"))]
            pub(crate) use crate::code_rule_dsl::logic::native::setfix::DERIVATIONS;
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
        #[path = "plan"]
        pub mod plan {
            pub use rerail_logic::code_rule_dsl::logic::plan::*;
            #[path = "corpus"]
            pub mod corpus {
                pub use rerail_logic::code_rule_dsl::logic::plan::corpus::*;
                #[cfg(test)]
                pub mod base_tests;
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
                use crate::code_rule_dsl::typing::cross_rounds;
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
            use crate::code_rule_dsl::logic::front::{AggOp, Atom, Literal, Program, Rule, Term, Value, cmp_binds, strata};
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
        #[path = "profile"]
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
        #[path = "rr"]
        pub mod rr {
            pub use rerail_logic::code_rule_dsl::logic::rr::*;
            #[path = "aggregate_demand"]
            pub mod aggregate_demand {
                pub use rerail_rr::code_rule_dsl::logic::rr::aggregate_demand::*;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::logic::front::{self, AggOp, Atom, Literal, Program, Rule, Term};
                #[allow(unused_imports)]
                use std::collections::{BTreeMap, BTreeSet};
            }
            #[path = "tape_type"]
            pub mod tape_type {
                pub use rerail_rr::code_rule_dsl::logic::rr::tape_type::*;
                #[allow(unused_imports)]
                use std::collections::{BTreeMap, BTreeSet};
                #[allow(unused_imports)]
                use rerail_rr_syntax::syntax::{Head, Item, Lit, Statement, Term};
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
            use crate::code_rule_dsl::logic::front::Program;
            #[allow(unused_imports)]
            #[cfg(any(test, feature = "test-hooks"))]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::rr::lower::drop_unread_core;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::rr::aggregate_demand::retype_helpers;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::rr::lower::is_core;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::rr::lower::WORLD;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::rr::print::print;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use syntax::quote;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::rr::translate::translate;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::facts::vtab::syntax::core_rr::single_metavars;
        }
        #[path = "strings"]
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
            #[allow(unused_imports)]
            use crate::code_rule_dsl::logic::front::ast::{Atom, Literal, Program, Term};
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
    #[path = "residency"]
    pub mod residency {
        pub use rerail_transducer::code_rule_dsl::residency::*;
        #[cfg(target_os="macos")]
        pub mod metal;
        #[cfg(target_os="macos")]
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
            #[cfg(test)]
            pub mod block_rows_tests;
            #[cfg(target_os="macos")]
            #[path = "verify/bounded_repeat_proof.rs"]
            pub mod bounded_repeat_proof;
            #[path = "compile"]
            pub mod compile_matcher {
                pub use rerail_transducer::code_rule_dsl::transducer::packed::compile_matcher::*;
                #[cfg(target_os="macos")]
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
            #[cfg(target_os="macos")]
            pub mod metal_backend;
            #[cfg(target_os="macos")]
            pub mod metal_bulk_closure;
            #[cfg(all(test,target_os="macos",feature="metal-tests"))]
            pub mod metal_bulk_closure_tests;
            #[cfg(target_os="macos")]
            pub mod metal_candidate_batch;
            #[cfg(target_os="macos")]
            pub mod metal_capture_arguments;
            #[cfg(target_os="macos")]
            pub mod metal_capture_groups;
            #[cfg(target_os="macos")]
            pub mod metal_capture_literals;
            #[cfg(target_os="macos")]
            pub mod metal_capture_predicates;
            #[cfg(target_os="macos")]
            pub mod metal_closure_continuation;
            #[cfg(target_os="macos")]
            pub mod metal_closure_prepare;
            #[cfg(all(test,target_os="macos",feature="metal-tests"))]
            pub mod metal_closure_prepare_tests;
            #[cfg(target_os="macos")]
            pub mod metal_constant_output;
            #[cfg(all(test,target_os="macos",feature="metal-tests"))]
            #[path = "metal_cutover_architecture.rs"]
            pub mod metal_cutover_architecture;
            #[cfg(target_os="macos")]
            pub mod metal_dynamic_execution;
            #[cfg(target_os="macos")]
            pub mod metal_failure_records;
            #[cfg(target_os="macos")]
            pub mod metal_frame_identity;
            #[cfg(all(test,target_os="macos",feature="metal-tests"))]
            pub mod metal_frame_identity_tests;
            #[cfg(all(test,target_os="macos",feature="metal-tests"))]
            pub mod metal_frontier_conformance;
            #[cfg(target_os="macos")]
            pub mod metal_frontier_reduce;
            #[cfg(target_os="macos")]
            pub mod metal_frontier_seed;
            #[cfg(all(test,target_os="macos",feature="metal-tests"))]
            pub mod metal_frontier_seed_tests;
            #[cfg(target_os="macos")]
            pub mod metal_frontier_validation;
            #[cfg(target_os="macos")]
            pub mod metal_grid_backend;
            #[cfg(target_os="macos")]
            pub mod metal_grid_contract;
            #[cfg(target_os="macos")]
            pub mod metal_grid_executor;
            #[cfg(target_os="macos")]
            pub mod metal_grid_plan;
            #[cfg(target_os="macos")]
            pub mod metal_initial_prefix;
            #[cfg(target_os="macos")]
            pub mod metal_initial_range_codegen;
            #[cfg(target_os="macos")]
            pub mod metal_inline_predicates;
            #[cfg(target_os="macos")]
            pub mod metal_join;
            #[cfg(target_os="macos")]
            pub mod metal_named_order;
            #[cfg(target_os="macos")]
            pub mod metal_output_contract;
            #[cfg(target_os="macos")]
            pub mod metal_output_dispatch;
            #[cfg(target_os="macos")]
            pub mod metal_output_mime;
            #[cfg(target_os="macos")]
            pub mod metal_output_pages;
            #[cfg(target_os="macos")]
            pub mod metal_output_selection;
            #[cfg(target_os="macos")]
            pub mod metal_output_strip;
            #[cfg(target_os="macos")]
            pub mod metal_parallel;
            #[cfg(target_os="macos")]
            pub mod metal_position_order;
            #[cfg(target_os="macos")]
            pub mod metal_program_backend;
            #[cfg(target_os="macos")]
            pub mod metal_ready_batch;
            #[cfg(all(test,target_os="macos",feature="metal-tests"))]
            pub mod metal_ready_batch_tests;
            #[cfg(target_os="macos")]
            pub mod metal_ready_proof;
            #[cfg(target_os="macos")]
            pub mod metal_regex_code;
            #[cfg(target_os="macos")]
            pub mod metal_regex_frontier;
            #[cfg(target_os="macos")]
            pub mod metal_register_codegen;
            #[cfg(all(test,target_os="macos",feature="metal-tests"))]
            pub mod metal_register_conformance;
            #[cfg(target_os="macos")]
            pub mod metal_register_execution;
            #[cfg(target_os="macos")]
            pub mod metal_rows;
            #[cfg(target_os="macos")]
            pub mod metal_semantic_columns;
            #[cfg(all(test,target_os="macos",feature="metal-tests"))]
            pub mod metal_semantic_conformance;
            #[cfg(target_os="macos")]
            pub mod metal_sort;
            #[cfg(target_os="macos")]
            pub mod metal_state_values;
            #[cfg(all(test,target_os="macos",feature="metal-tests"))]
            pub mod metal_state_values_tests;
            #[cfg(target_os="macos")]
            pub mod metal_successor;
            #[cfg(target_os="macos")]
            pub mod metal_successor_branch_codegen;
            #[cfg(target_os="macos")]
            pub mod metal_successor_codegen;
            #[cfg(target_os="macos")]
            pub mod metal_successor_prepare;
            #[cfg(target_os="macos")]
            pub mod metal_successor_repeat_codegen;
            #[cfg(all(test,target_os="macos",feature="metal-tests"))]
            pub mod metal_successor_tests;
            #[cfg(target_os="macos")]
            pub mod metal_terminal;
            #[cfg(target_os="macos")]
            pub mod metal_terminal_capture_order;
            #[cfg(target_os="macos")]
            pub mod metal_terminal_repeat;
            #[cfg(target_os="macos")]
            pub mod metal_typed_output;
            #[cfg(all(test,target_os="macos",feature="metal-tests"))]
            pub mod metal_unique_child_tests;
            #[cfg(all(test,target_os="macos",feature="metal-tests"))]
            pub mod metal_value_prefix_tests;
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
            #[allow(unused_imports)]
            use crate::code_rule_dsl::transducer::packed::literal_anchors::*;
            #[allow(unused_imports)]
            #[cfg(all(any(test, feature = "test-hooks"), target_os = "macos"))]
            pub(super) use crate::code_rule_dsl::transducer::packed::metal_backend::launch::LaunchScope as MetalTestScope;
            #[allow(unused_imports)]
            #[cfg(target_os = "macos")]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::metal_program_backend::set_steps;
            #[allow(unused_imports)]
            #[cfg(target_os = "macos")]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::metal_program_backend::lattice as metal_lattice;
            #[allow(unused_imports)]
            #[cfg(target_os = "macos")]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::metal_program_backend::nearest as metal_nearest;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::device::{RootJobs, RootsFound};
            #[allow(unused_imports)]
            #[cfg(target_os = "macos")]
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
    #[path = "typing"]
    pub mod typing {
        pub use rerail_logic::code_rule_dsl::typing::*;
        #[path = "file_local"]
        pub mod file_local {
            pub use rerail_logic::code_rule_dsl::typing::file_local::*;
            #[cfg(test)]
            pub mod tests;
            #[allow(unused_imports)]
            use crate::content_store::{StoreError, StoreResult};
            #[allow(unused_imports)]
            pub(crate) use crate::code_rule_dsl::facts::nearest::{Binding, NONE, Trees};
            #[allow(unused_imports)]
            pub(crate) use crate::code_rule_dsl::typing::file_local::split_memo::forget as forget_splits;
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
    #[path = "waves"]
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
