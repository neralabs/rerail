//! The program image's core (decision 0377): `Image`, its versions (`Version`) and the `Builder` that writes them,
//! with `program_facts`, the rows the image's header is. Under the lowering (`rerail-rr`), so the lowering writes a
//! module's rules as versions; the pass manager, the passes and the analyses stay in `rerail-logic`, which mounts
//! this crate's `program_image` with them added.
//!
//! Its sources are in its own `src/`, at their module paths in the engine's tree (the crate split, step 9).
// A mounted module re-exports its dependency's by glob, and its sources import the same items
// under the same names: the glob's name and the private one are one item.
#![allow(dead_code, unused_imports, hidden_glob_reexports)]
// cratify: mounts (tools/cratify/mounts.py; generated, do not edit) begin
extern crate self as rerail_program_image;
#[path = "code_rule_dsl"]
pub mod code_rule_dsl {
    #[path = "logic"]
    pub mod logic {
        pub mod front {
            pub use rerail_rr_syntax::code_rule_dsl::logic::front::{ast, name};
        }
        pub use rerail_prof::code_rule_dsl::logic::profile;
        pub mod program_facts;
        pub mod program_image;
    }
}
// cratify: mounts end
