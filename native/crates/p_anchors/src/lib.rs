//! P's anchors module and SQL anchor engine as copied (2814d875), test-only (O20): `matcher::p_anchors`, its anchor
//! corpus (`Corpus`, `CorpusSlot`) at P's paths, and the corpus's part in a rule's stateless evaluation
//! (`p_anchors::evaluation`, the `p-anchor-corpus` path), which the code rules' evaluator reads through
//! `eval::ANCHOR_CORPUS`. Over the code rules (rerail-rules-base), whose model and compiler it reads; below the
//! admission (rerail-rules), which re-exports it. A build without `test-hooks` mounts nothing.
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
pub use rerail_rules_base::{content_store, git_access, git_workspace};
extern crate self as rerail_p_anchors;
#[path = "../../../src/code_rule_dsl"]
pub mod code_rule_dsl {
    pub use rerail_rules_base::code_rule_dsl::*;
    pub mod matcher {
        pub use rerail_rules_base::code_rule_dsl::matcher::*;
        /// P's anchors module and SQL anchor engine as copied (2814d875), test-only (O20).
        #[cfg(any(test, feature = "test-hooks"))]
        #[allow(dead_code, unused_imports)]
        pub mod p_anchors;
        // P's anchor corpus at P's paths, for P's relational workspace code and the `p-anchor-corpus` path.
        #[cfg(any(test, feature = "test-hooks"))]
        pub use p_anchors::sql::{Corpus, CorpusSlot};
        #[cfg(all(any(test, feature = "test-hooks"), feature = "p-anchor-corpus"))]
        pub use p_anchors::sql::{anchor_plans, anchor_plans_for_file};
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
    pub mod tape {
        pub use rerail_rules_base::code_rule_dsl::tape::*;
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

/// This crate's own test binary, which the engine does not load: P's corpus registered with the evaluator, as boot
/// registers it.
#[cfg(all(test, feature = "p-anchor-corpus"))]
mod test_load {
    extern "C" fn load() {
        crate::code_rule_dsl::matcher::p_anchors::evaluation::register();
    }
    #[used]
    #[cfg_attr(target_vendor = "apple", unsafe(link_section = "__DATA,__mod_init_func"))]
    #[cfg_attr(windows, unsafe(link_section = ".CRT$XCU"))]
    #[cfg_attr(not(any(target_vendor = "apple", windows)), unsafe(link_section = ".init_array"))]
    static LOAD: extern "C" fn() = load;
}
