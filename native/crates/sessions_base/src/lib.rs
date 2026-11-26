//! The SQL executor's parts below the executor (`sql_executor`): its sessions' runs, tuple maintenance, reductions,
//! versions, phases, walks and work graph. The executor (`CodeRuleSqlExecutor`) and the retained traces (`temporal`)
//! are rerail-sessions', which the parts here read through `session::Trace` and `maintenance::TEMPORAL`.
//!
//! Its sources are in its own `src/`, at their module paths in the engine's tree (the crate split, step 9).
// A mounted module re-exports its dependency's by glob, and its sources import the same items
// under the same names: the glob's name and the private one are one item. The mounts also widen what the crates
// above read (the split's facades, not a design).
#![allow(dead_code, unused_imports, hidden_glob_reexports, private_interfaces, private_bounds)]
// What the split widened for the crates above is no public API (`len`, `()` errors, `new`); the long
// signatures and tuple types are the sources' own.
#![allow(clippy::len_without_is_empty, clippy::result_unit_err, clippy::new_without_default)]
#![allow(clippy::type_complexity, clippy::too_many_arguments)]
// The browser build has no JIT: wasm/cranelift-jit stands in under the same name (wasm/README.md).
#[cfg(target_family = "wasm")]
extern crate cranelift_jit_wasm as cranelift_jit;
// cratify: mounts (tools/cratify/mounts.py; generated, do not edit) begin
pub use rerail_store::content_store;
pub use rerail_store::git_access;
pub use rerail_store::git_workspace;

extern crate self as rerail_sessions_base;
#[path = "code_rule_dsl"]
pub mod code_rule_dsl {
    pub use rerail_lang::code_rule_dsl::budget;
    pub use rerail_lang::code_rule_dsl::build_ids;
    pub use rerail_lang::code_rule_dsl::embed;
    
    pub mod facts {
        pub use rerail_facts::code_rule_dsl::facts::*;
        pub mod vtab {
            pub use rerail_facts::code_rule_dsl::facts::vtab::*;
            pub mod syntax {
                pub use rerail_facts::code_rule_dsl::facts::vtab::syntax::*;
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
    #[cfg(any(test, feature = "test-hooks"))]
    pub use rerail_lang::code_rule_dsl::fixtures;
    pub use rerail_lang::code_rule_dsl::json;
    pub use rerail_lang::code_rule_dsl::regexes;
    pub use rerail_lang::code_rule_dsl::language;
    pub use rerail_lang::code_rule_dsl::lines;
    
    pub mod logic {
        pub use rerail_corpus_memo::code_rule_dsl::logic::*;
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
        pub mod profile {
            pub use rerail_prof::code_rule_dsl::logic::profile::*;
            pub use rerail_corpus_memo::code_rule_dsl::logic::profile::report;
            pub use rerail_corpus_memo::code_rule_dsl::logic::profile::watch;
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
            pub use rerail_corpus_memo::code_rule_dsl::logic::rr::*;
            pub use rerail_rr::code_rule_dsl::logic::rr::translate;
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
    pub use rerail_lang::code_rule_dsl::module_path;
    pub use rerail_prof::code_rule_dsl::phase_clock;
    pub use rerail_facts::code_rule_dsl::producers;
    pub use rerail_prof::code_rule_dsl::prof;
    pub use rerail_facts::code_rule_dsl::search_index;
    pub use rerail_lang::code_rule_dsl::semver;
    pub mod sql_executor;
    pub use rerail_sql_profile::code_rule_dsl::sql_profile;
    pub use rerail_sql_profile::code_rule_dsl::sql_admission;
    pub mod stats {
        pub use rerail_facts::code_rule_dsl::stats::identity;
    }
    pub mod suppress {
        pub use rerail_lang::code_rule_dsl::suppress::kinds;
        #[allow(unused_imports)]
        use ast_grep_core::{Doc, Node};
        #[allow(unused_imports)]
        pub(crate) use crate::code_rule_dsl::suppress::kinds::*;
    }
    pub use rerail_lang::code_rule_dsl::tape;
    pub use rerail_facts::code_rule_dsl::text;
    pub use rerail_lang::code_rule_dsl::tree;
    pub use rerail_corpus_memo::code_rule_dsl::typing;
    pub mod value {
        pub use rerail_lang::code_rule_dsl::value::text_regex;
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
        pub(crate) use crate::code_rule_dsl::value::text_regex::MAX_STRING_BYTES;
    }
    pub mod waves {
        pub use rerail_corpus_memo::code_rule_dsl::waves::owner;
        pub use rerail_corpus_memo::code_rule_dsl::waves::schedule;
        pub use rerail_corpus_memo::code_rule_dsl::waves::storage;
    }
    #[allow(unused_imports)]
    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) use crate::code_rule_dsl::fixtures::{fixtures_dir, oracle};
    #[allow(unused_imports)]
    use crate::code_rule_dsl::json::{canonicalize_value, hex_digest, invalid, json_error};
    #[allow(unused_imports)]
    use napi::Result;
    #[allow(unused_imports)]
    use napi_derive::napi;
    #[allow(unused_imports)]
    use serde_json::Value;
    #[allow(unused_imports)]
    use std::sync::Arc;
}
pub use rerail_corpus_memo::threads;
// cratify: mounts end
