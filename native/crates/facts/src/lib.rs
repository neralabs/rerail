//! The engine's facts: the syntax table (`facts::vtab::syntax`: a batch's documents, their views, their families'
//! relations and the kind-set axes) and the families' store reads (`facts::families`), over the facts' base
//! (rerail-facts-base: the other virtual tables, text, strings, producers, SQL profiles), which it re-exports, so a
//! reader above reads every facts path here, as before.
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
pub use rerail_facts_base::{content_store, git_access, git_workspace};
#[path = "code_rule_dsl"]
pub mod code_rule_dsl {
    pub use rerail_facts_base::code_rule_dsl::*;
    #[path = "facts"]
    pub mod facts {
        pub use rerail_facts_base::code_rule_dsl::facts::*;
        #[allow(dead_code)]
        pub mod families;
        #[path = "vtab"]
        pub mod vtab {
            pub use rerail_facts_base::code_rule_dsl::facts::vtab::*;
            pub mod syntax;
        }
    }
}
