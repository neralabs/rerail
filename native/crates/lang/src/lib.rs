//! The language registry and what every crate reads of a language: tapes, trees, lines, JSON, budgets.
//!
//! Its sources stay where they were (native/src): this crate mounts them in place, as the engine did.
// The mounts glob their owners' modules and widen what the crates above read: the split's facades, not a design.
#![allow(dead_code, unused_imports, hidden_glob_reexports, private_interfaces, private_bounds)]
// What the split widened for the crates above is no public API (`len`, `()` errors, `new`); the long
// signatures and tuple types are the sources' own.
#![allow(clippy::len_without_is_empty, clippy::result_unit_err, clippy::new_without_default)]
#![allow(clippy::type_complexity, clippy::too_many_arguments)]
// cratify: mounts (tools/cratify/mounts.py; generated, do not edit) begin
pub use rerail_store::content_store;
pub use rerail_store::git_workspace;
extern crate self as rerail_lang;
#[path = "../../../src/code_rule_dsl"]
pub mod code_rule_dsl {
    pub mod budget;
    pub mod build_ids;
    pub mod embed;
    #[cfg(any(test, feature = "test-hooks"))]
    pub mod fixtures;
    pub mod json;
    pub mod language;
    pub mod lines;
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
        }
        pub use rerail_prof::code_rule_dsl::logic::profile;
    }
    pub mod module_path;
    pub use rerail_prof::code_rule_dsl::phase_clock;
    pub use rerail_prof::code_rule_dsl::prof;
    pub mod semver;
    #[path = "suppress"]
    pub mod suppress {
        pub mod kinds;
        #[allow(unused_imports)]
        use ast_grep_core::{Doc, Node};
        #[allow(unused_imports)]
        pub(crate) use crate::code_rule_dsl::suppress::kinds::*;
    }
    pub mod tape;
    pub mod tree;
    #[path = "value"]
    pub mod value {
        pub mod text_regex;
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
