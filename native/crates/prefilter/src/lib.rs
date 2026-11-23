//! The prefilter (`prefilter`): a tape's terms as candidate sets, the plans that read them, the gates, the readers and
//! the lowering. It reads the rule API and the stats (rerail-workspace) and the label sets and `space`
//! (rerail-transducer); boot re-exports it, and registers its tapes' functions and its image (`boot::install_with`).
//!
//! Its sources stay where they were (native/src): this crate mounts them in place, as the engine did.
// A mounted module re-exports its dependency's by glob, and its sources import the same items
// under the same names: the glob's name and the private one are one item.
#![allow(dead_code, unused_imports, hidden_glob_reexports, private_interfaces, private_bounds)]
#![allow(clippy::len_without_is_empty, clippy::result_unit_err, clippy::new_without_default)]
#![allow(clippy::type_complexity, clippy::too_many_arguments)]
pub use rerail_workspace::{content_store, git_access, git_workspace};
#[path = "../../../src/code_rule_dsl"]
pub mod code_rule_dsl {
    pub use rerail_workspace::code_rule_dsl::*;
    pub use rerail_transducer::code_rule_dsl::{space, transducer};
    pub mod logic {
        pub use rerail_workspace::code_rule_dsl::logic::*;
        pub use rerail_run::code_rule_dsl::logic::lanes;
    }
    pub mod prefilter;
}
