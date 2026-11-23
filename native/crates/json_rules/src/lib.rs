//! Main's JSON rules (`rerail.policy/v1`, decision 0175): the translator of a JSON policy to `.rerail` text, which the
//! language's reader (`logic::rr`, `rerail-logic`'s) runs on read. It reads only the language's lowering and library
//! (`rerail-rr`) and the languages, so it is under the planner's crates and builds beside them.
//!
//! Its sources are in its own `src/`, at their module paths in the engine's tree (the crate split, step 9).
// A mounted module re-exports its dependency's by glob, and its sources import the same items under the same names.
#![allow(dead_code, unused_imports, hidden_glob_reexports)]
// The long signatures and tuple types are the sources' own.
#![allow(clippy::type_complexity, clippy::too_many_arguments)]
extern crate self as rerail_json_rules;
#[path = "code_rule_dsl"]
pub mod code_rule_dsl {
    pub use rerail_rr::code_rule_dsl::*;
    #[path = "logic"]
    pub mod logic {
        pub use rerail_rr::code_rule_dsl::logic::*;
        pub mod json_rules;
    }
}
