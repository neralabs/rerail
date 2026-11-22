//! The relational admission (`residency::admission`): P's relational clauses compiled (`relational::compile`) and
//! evaluated over SQLite sessions (`relational::evaluate_batch`, `batch`, `syntax`, the producers), and batch
//! admission's measurements (`census`, `memory`). Over the code rules (rerail-rules-base), whose compiler and
//! evaluator call it through the registered `Admission` (`relational::ADMISSION`, which boot registers); below the
//! workspace (rerail-workspace). It re-exports the code rules by glob, so the crates above read them here, and with
//! them P's and D's test-only engines (`matcher::p_anchors`, `matcher::d_anchors`, `waves::relational`:
//! rerail-p-anchors, rerail-d-relational, rerail-d-anchors).
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
extern crate self as rerail_rules;
#[path = "../../../src/code_rule_dsl"]
pub mod code_rule_dsl {
    pub use rerail_rules_base::code_rule_dsl::*;
    /// The matcher, with P's and D's SQL anchor engines (test-only copies: rerail-p-anchors, rerail-d-anchors).
    pub mod matcher {
        pub use rerail_rules_base::code_rule_dsl::matcher::*;
        #[cfg(feature = "test-hooks")]
        pub use rerail_p_anchors::code_rule_dsl::matcher::{Corpus, CorpusSlot, p_anchors};
        #[cfg(all(feature = "test-hooks", feature = "p-anchor-corpus"))]
        pub use rerail_p_anchors::code_rule_dsl::matcher::{anchor_plans, anchor_plans_for_file};
        #[cfg(feature = "test-hooks")]
        pub use rerail_d_anchors::code_rule_dsl::matcher::d_anchors;
    }
    /// D's relational waves (test-only, rerail-d-relational).
    pub mod waves {
        #[cfg(feature = "test-hooks")]
        pub use rerail_d_relational::code_rule_dsl::waves::relational;
    }
    #[path = "residency"]
    pub mod residency {
        pub use rerail_rules_base::code_rule_dsl::residency::*;
        pub mod admission;
        #[allow(unused_imports)]
        pub(in crate::code_rule_dsl) use crate::code_rule_dsl::residency::admission::relational::ast_stage::install_ast_producer;
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
