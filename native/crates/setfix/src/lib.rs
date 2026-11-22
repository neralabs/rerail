//! The logic language's native solvers (`logic::native`: setfix and the others) with what they and their runs read
//! of each other: the run and its state (`run`, `run_state`), its lanes and executors (`lanes`), the solver device
//! interface (`device`, with the run's binding), the waves' owner, schedule and storage, and the thread pool
//! (`threads`). It is under the planner, which reads it, and over `rerail-logic-core`. Two of the planner's modules it
//! reads are mounted here, under the planner's paths (`plan::sketch`, `plan::statkeep`: a kept relation's
//! statistics in the planner's form).
//!
//! Its sources stay where they were (native/src): this crate mounts them in place, as the engine did.
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
pub use rerail_store::content_store;
pub use rerail_store::git_access;
pub use rerail_store::git_workspace;
extern crate self as rerail_setfix;
#[path = "../../../src/code_rule_dsl"]
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
        pub use rerail_logic_core::code_rule_dsl::logic::{analysis, bloom, costs, crypto, derived, join_size, liveness, reach_form, sql, version_trace};
        pub mod device;
        pub mod lanes;
        pub mod native;
        pub mod run;
        pub mod run_state;
        /// The planner's modules setfix reads, under the planner's paths.
        #[path = "plan"]
        pub mod plan {
            pub use rerail_facts::code_rule_dsl::logic::plan::sketch;
            pub mod statkeep;
        }
    }
    pub use rerail_lang::code_rule_dsl::module_path;
    pub use rerail_prof::code_rule_dsl::phase_clock;
    pub use rerail_facts::code_rule_dsl::producers;
    pub use rerail_prof::code_rule_dsl::prof;
    pub use rerail_facts::code_rule_dsl::search_index;
    pub use rerail_lang::code_rule_dsl::semver;
    pub use rerail_facts::code_rule_dsl::sql_profile;
    pub use rerail_facts::code_rule_dsl::sql_admission;
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
    #[path = "waves"]
    pub mod waves {
        pub mod owner;
        pub mod schedule;
        pub mod storage;
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
#[path = "../../../src/threads.rs"]
pub mod threads;

/// This crate's own test binary, which no program composes: what a job carries of its run is the run's and setfix's
/// (`run_state::RUN_CARRIERS`), with no planner.
#[cfg(test)]
mod test_load {
    extern "C" fn load() {
        crate::code_rule_dsl::logic::run_state::CARRIERS.register("rerail-setfix's tests", crate::code_rule_dsl::logic::run_state::RUN_CARRIERS);
    }
    #[used]
    #[cfg_attr(target_vendor = "apple", unsafe(link_section = "__DATA,__mod_init_func"))]
    #[cfg_attr(windows, unsafe(link_section = ".CRT$XCU"))]
    #[cfg_attr(not(any(target_vendor = "apple", windows)), unsafe(link_section = ".init_array"))]
    static LOAD: extern "C" fn() = load;
}

// The crate's own tests make their folders the user's alone, as the engine's test binaries do (decision 0574): a
// host's umask of `002` made a fixture's folder `775`, which the mapped files' rules refuse.
#[cfg(test)]
#[path = "../../../src/test_private_folders.rs"]
mod test_private_folders;
