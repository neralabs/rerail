//! The transducer's base: the packed programs, their interpreter, Bitlex, the batch ABI, the device and CPU executor
//! interfaces, the query front end and the label sets (their node space is rerail-node-space's, which it re-exports).
//! It reads none of W (rerail-rules): the compiled CPU executor
//! (rerail-transducer-cpu) and W's matchers and waves lowered, with `demand` (rerail-transducer), are above it.
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
pub use rerail_observe::content_store;
pub use rerail_observe::git_access;
pub use rerail_observe::git_workspace;
extern crate self as rerail_transducer;
extern crate self as rerail_transducer_base;
#[path = "code_rule_dsl"]
pub mod code_rule_dsl {
    pub use rerail_lang::code_rule_dsl::budget;
    pub use rerail_lang::code_rule_dsl::build_ids;
    pub use rerail_lang::code_rule_dsl::embed;
    pub mod facts {
        pub use rerail_facts::code_rule_dsl::facts::*;
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
        use crate::content_store::{Binding, Derivation, DerivationRecord, FactStore, StoreResult};
    }
    #[cfg(any(test, feature = "test-hooks"))]
    pub use rerail_lang::code_rule_dsl::fixtures;
    pub use rerail_lang::code_rule_dsl::json;
    pub use rerail_lang::code_rule_dsl::regexes;
    pub mod language {
        pub use rerail_lang::code_rule_dsl::language::*;
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
    pub mod logic {
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
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::front::ast::{AXIS, AggOp, DESCENDANTS, EVERY_KIND, GLOBAL, LIBRARY, LatticeDecl, NEAREST, Native, OPTION_GLOB, QUALIFIERS, RESERVED, SCOPE_NOT, SpanIn, Value, axis, globals, is_axis, is_transduced};
            #[allow(unused_imports)]
            pub(crate) use crate::code_rule_dsl::facts::core::PYTHON_CONTEXTS;
        }
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
    pub use rerail_lang::code_rule_dsl::module_path;
    pub use rerail_prof::code_rule_dsl::phase_clock;
    pub use rerail_facts::code_rule_dsl::producers;
    pub use rerail_prof::code_rule_dsl::prof;
    pub use rerail_facts::code_rule_dsl::search_index;
    pub use rerail_lang::code_rule_dsl::semver;
    pub use rerail_node_space::code_rule_dsl::space;
    pub use rerail_facts::code_rule_dsl::sql_profile;
    pub use rerail_facts::code_rule_dsl::sql_admission;
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
    pub mod transducer;
    pub use rerail_lang::code_rule_dsl::tree;
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
// cratify: mounts end
