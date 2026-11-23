//! The logic language's core, under the planner: the typed SQL its plans emit (`sql`), the program's analyses
//! (`analysis`), the cost model (`costs`), the bloom filters and join sizes, the liveness rules, and the derived
//! relations' names. setfix (with the run, the lanes, the device interface, the waves and the thread pool), the planner
//! and the corpus are the crates above.
//!
//! Its sources are in its own `src/`, at their module paths in the engine's tree (the crate split, step 9).
// A mounted module re-exports its dependency's by glob, and its sources import the same items under the same names:
// the glob's name and the private one are one item. The mounts also widen what the crates above read (the split's
// facades, not a design).
#![allow(dead_code, unused_imports, hidden_glob_reexports, private_interfaces, private_bounds)]
// What the split widened for the crates above is no public API (`len`, `()` errors, `new`); the long signatures and
// tuple types are the sources' own.
#![allow(clippy::len_without_is_empty, clippy::result_unit_err, clippy::new_without_default)]
#![allow(clippy::type_complexity, clippy::too_many_arguments)]
// The browser build has no JIT: wasm/cranelift-jit stands in under the same name (wasm/README.md).
#[cfg(target_family = "wasm")]
extern crate cranelift_jit_wasm as cranelift_jit;
pub use rerail_observe::content_store;
pub use rerail_observe::git_access;
pub use rerail_observe::git_workspace;
extern crate self as rerail_logic_core;
#[path = "code_rule_dsl"]
pub mod code_rule_dsl {
    pub use rerail_lang::code_rule_dsl::budget;
    pub use rerail_lang::code_rule_dsl::build_ids;
    pub use rerail_lang::code_rule_dsl::embed;
    pub use rerail_facts::code_rule_dsl::facts;
    #[cfg(any(test, feature = "test-hooks"))]
    pub use rerail_lang::code_rule_dsl::fixtures;
    pub use rerail_lang::code_rule_dsl::json;
    pub use rerail_lang::code_rule_dsl::regexes;
    pub use rerail_lang::code_rule_dsl::language;
    pub use rerail_lang::code_rule_dsl::lines;
    #[path = "logic"]
    pub mod logic {
        // The lower crates' modules of the logic language, under its paths.
        pub use rerail_rr::code_rule_dsl::logic::front;
        pub use rerail_prof::code_rule_dsl::logic::profile;
        pub use rerail_program_image::code_rule_dsl::logic::{program_facts, program_image};
        pub use rerail_facts::code_rule_dsl::logic::strings;
        pub mod analysis;
        pub mod bloom;
        pub mod costs;
        #[path = "../../../../facts_base/src/code_rule_dsl/logic/crypto.rs"]
        pub mod crypto;
        pub mod derived;
        pub mod join_size;
        pub mod liveness;
        pub mod reach_form;
        pub mod sql;
        pub mod version_trace;
    }
    pub use rerail_lang::code_rule_dsl::module_path;
    pub use rerail_prof::code_rule_dsl::phase_clock;
    pub use rerail_facts::code_rule_dsl::producers;
    pub use rerail_prof::code_rule_dsl::prof;
    pub use rerail_facts::code_rule_dsl::search_index;
    pub use rerail_lang::code_rule_dsl::semver;
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
