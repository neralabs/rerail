//! The code rules (W's): their model (`model`), their compiler (`compile`) and their evaluation over a file
//! (`eval`, `query`, `violations`, `resolution`), with the SQLite hooks and functions (`hooks`, `crypto`) and the
//! relational admission's base (`residency::admission::relational`: its plans, its database, and the admission
//! behind a registered trait, `ADMISSION`). Over the matcher (`rerail-matcher`), below the admission
//! (rerail-rules). The matcher's facade here mounts P's and D's SQL anchor engines and `waves::relational`
//! (test-only copies), which read the model and the admission's database.
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
pub use rerail_matcher::content_store;
pub use rerail_matcher::git_access;
pub use rerail_matcher::git_workspace;
extern crate self as rerail_rules_base;
#[path = "../../../src/code_rule_dsl"]
pub mod code_rule_dsl {
    pub use rerail_matcher::code_rule_dsl::*;
    pub mod compile;
    pub mod crypto;
    pub mod eval;
    pub mod findings;
    pub mod hooks;
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
    #[path = "matcher"]
    pub mod matcher {
        pub use rerail_matcher::code_rule_dsl::matcher::*;
        /// P's anchors module and SQL anchor engine as copied (2814d875), test-only (O20).
        #[cfg(any(test, feature = "test-hooks"))]
        #[allow(dead_code, unused_imports)]
        pub mod p_anchors;
        // P's anchor corpus at P's paths, for P's relational workspace code and the `p-anchor-corpus` path.
        #[cfg(any(test, feature = "test-hooks"))]
        pub use p_anchors::sql::{Corpus, CorpusSlot};
        #[cfg(all(any(test, feature = "test-hooks"), feature = "p-anchor-corpus"))]
        pub use p_anchors::sql::{anchor_plans, anchor_plans_for_file};
        /// D's anchors module and SQL anchor engine as copied (bb79b40a), test-only: the closure of D's
        /// plan-input statistics (O19).
        #[cfg(any(test, feature = "test-hooks"))]
        #[allow(dead_code, unused_imports)]
        pub mod d_anchors;
        // `matcher/mod.rs`'s imports, which the anchor copies read (`use super::*`).
        #[allow(unused_imports)]
        use super::budget;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::{GrammarDialect, SourceType};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::regexes::Regex;
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
        use std::borrow::Cow;
        #[allow(unused_imports)]
        use std::collections::BTreeSet;
        #[allow(unused_imports)]
        use std::sync::Arc;
        #[allow(unused_imports)]
        use util::{
            MAX_UTIL_STACK_BYTES, UTIL_DEPTH, UTIL_EXCEEDED, UTIL_STACK_BASE, UTIL_STEPS_LEFT,
            stack_address,
        };
        #[allow(unused_imports)]
        use util::{THREE_VALUED, UNKNOWN};
    }
    pub mod model;
    pub mod query;
    #[path = "residency"]
    pub mod residency {
        pub mod admission {
            /// The relational admission's base: its plans, the admission behind a registered trait (`ADMISSION`),
            /// and its database. The admission itself is rerail-rules' (`relational.rs`).
            pub mod relational {
                pub mod base;
                pub use base::*;
                pub mod database;
                pub mod storage;
            }
        }
    }
    pub mod resolution;
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
    pub mod violations;
    #[cfg_attr(any(test, feature = "test-hooks"), path = "waves")]
    pub mod waves {
        #[cfg(any(test, feature = "test-hooks"))]
        pub mod relational;
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
}
