//! The matcher (W's): a clause's ast rules over a document's tape (`matcher`): its anchors, values, relations,
//! counts, captures and memo; the syntax errors' soundness (`soundness`), the code-rule profile (`profile`, with
//! `code-rule-profile`) and the semantic facts its binds read (`transducer::semantic_facts`). Over the adapters
//! (`rerail-adapters`), below the rule compiler and the workspace (rerail-classic).
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
pub use rerail_adapters::content_store;
pub use rerail_adapters::git_access;
pub use rerail_adapters::git_workspace;
extern crate self as rerail_matcher;
#[path = "../../../src/code_rule_dsl"]
pub mod code_rule_dsl {
    pub use rerail_adapters::code_rule_dsl::*;
    pub mod matcher;
    #[cfg(feature = "code-rule-profile")]
    pub mod profile;
    pub mod soundness;
    #[path = "transducer"]
    pub mod transducer {
        pub mod semantic_facts;
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use crate::code_rule_dsl::tape::TapeDoc;
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use ast_grep_core::MatchStrictness;
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use ast_grep_core::matcher::{Pattern, PatternNode};
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use regex::Regex;
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use std::collections::{BTreeMap, BTreeSet};
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use std::ops::Range;
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use crate::code_rule_dsl::search_index::persist_search_index_batch;
    }
    #[cfg(any(test, feature = "test-hooks"))]
    pub mod walks;
}
