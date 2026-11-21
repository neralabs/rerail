//! The logic language (docs/final-shape/03-architecture.md §12; 05 Phases 5–6): rules as named
//! facts and Datalog over them, planned from statistics and lowered to SQL or transducer groups.
//! `sql` is the typed SQL its plans emit. The core modules (`sql`, `analysis`, `costs`, …) are
//! `rerail-logic-core`'s, and the native solvers with the run, its lanes and its state (`native`, `run`, `lanes`,
//! `run_state`, `device`) `rerail-setfix`'s, under the planner; this module re-exports them under their paths.

pub use rerail_logic_core::code_rule_dsl::logic::crypto;
pub use rerail_logic_core::code_rule_dsl::logic::derived;
pub use rerail_setfix::code_rule_dsl::logic::run;
/// setfix's, with its tests, which read what the whole program carries (`plan::CARRIERS`).
pub mod run_state {
    pub use rerail_setfix::code_rule_dsl::logic::run_state::*;
    #[cfg(test)]
    mod tests;
}
#[cfg(test)]
pub mod facts_only;
/// This crate's own test binary, which the engine does not load: strict mode on every connection, as the engine's
/// loader sets it (the store sends its SQL hazards to the profile itself, `content_store::hazards`), and what a job
/// carries of its run, as boot registers it.
#[cfg(test)]
mod test_load {
    extern "C" fn load() {
        crate::code_rule_dsl::logic::profile::strict_on_every_connection();
        crate::code_rule_dsl::logic::run_state::CARRIERS.register("rerail-logic's tests", crate::code_rule_dsl::logic::plan::CARRIERS);
    }
    // Run as the binary loads, before any test opens a store (as the engine's `module_init` runs).
    #[used]
    #[cfg_attr(target_vendor = "apple", unsafe(link_section = "__DATA,__mod_init_func"))]
    #[cfg_attr(not(target_vendor = "apple"), unsafe(link_section = ".init_array"))]
    static LOAD: extern "C" fn() = load;
}
/// `rerail-logic-core`'s, with its tests, which read the whole language (`rr::parse_version`).
pub mod analysis {
    pub use rerail_logic_core::code_rule_dsl::logic::analysis::*;
    #[cfg(test)]
    pub mod tests;
}
pub use rerail_logic_core::code_rule_dsl::logic::bloom;
pub use rerail_logic_core::code_rule_dsl::logic::join_size;
// Nothing emits SQL through them until the planner (Phase 6) does.
// Its files are its test modules alone.
#[cfg_attr(any(test, feature = "test-hooks"), path = "logic/front")]
pub mod front {
    pub use rerail_rr::code_rule_dsl::logic::front::*;
    // Its files are its test modules alone.
    #[cfg_attr(any(test, feature = "test-hooks"), path = ".")]
    pub mod caches {
        pub use rerail_rr::code_rule_dsl::logic::front::caches::*;
        #[cfg(any(test, feature = "test-hooks"))]
        #[path = "caches_tests.rs"]
        pub mod tests;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::logic::front::Error;
        #[allow(unused_imports)]
        use std::collections::BTreeSet;
    }
    #[cfg(test)]
    pub mod data_layer_tests;
    #[cfg(any(test, feature = "test-hooks"))]
    pub mod tests;
    // Its files are its test modules alone.
    #[cfg_attr(any(test, feature = "test-hooks"), path = "yaml")]
    pub mod yaml {
        pub use rerail_rr::code_rule_dsl::logic::front::yaml::*;
        #[cfg(any(test, feature = "test-hooks"))]
        pub mod tests;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::logic::front::Error;
        #[allow(unused_imports)]
        use serde_json::Value;
    }
    #[allow(unused_imports)]
    use std::collections::{BTreeMap, BTreeSet};
    #[allow(unused_imports)]
    use std::fmt;
}
pub use rerail_logic_core::code_rule_dsl::logic::costs;
// The logic's device interface; the engine registers a device as it loads (`crate::install_devices`).
pub use rerail_setfix::code_rule_dsl::logic::device;
pub use rerail_setfix::code_rule_dsl::logic::lanes;
#[allow(dead_code)]
pub mod answers;
pub use rerail_logic_core::code_rule_dsl::logic::liveness;
// The demand passes' tests (`plan::corpus::preparation::magic`) against T's goldens; its files are those alone.
#[cfg(any(test, feature = "test-hooks"))]
#[path = "logic/magic"]
pub mod magic {
    pub mod tests;
}
#[allow(dead_code)]
/// setfix's, with the tests of it that read the language and the planner (`rr::parse_version`, `plan::corpus::retain`).
pub mod native {
    pub use rerail_setfix::code_rule_dsl::logic::native::*;
    pub mod setfix {
        pub use rerail_setfix::code_rule_dsl::logic::native::setfix::*;
        #[allow(unused_imports)]
        use rusqlite::Connection;
        #[allow(unused_imports)]
        use std::sync::Arc;
        #[cfg(test)]
        mod bounded_tests;
        #[cfg(test)]
        mod retain_tests;
    }
}
#[allow(dead_code)]
pub mod plan;
#[path = "logic/profile"]
pub mod profile {
    pub use rerail_prof::code_rule_dsl::logic::profile::*;
    pub mod report;
    pub mod watch;
    #[allow(unused_imports)]
    use std::collections::BTreeMap;
    #[allow(unused_imports)]
    use std::fmt::Write as _;
    #[allow(unused_imports)]
    use std::path::Path;
    #[allow(unused_imports)]
    use std::sync::atomic::{AtomicBool, Ordering};
    #[allow(unused_imports)]
    use std::sync::{LazyLock, Mutex};
}
// The program image's core is a crate of its own, under the lowering (decision 0377); the pass manager, the passes and
// the analyses, which read the planner, are this crate's, added to the core's module.
pub use rerail_program_image::code_rule_dsl::logic::program_facts;
#[path = "logic/program_image"]
pub mod program_image {
    pub use rerail_program_image::code_rule_dsl::logic::program_image::*;
    pub mod analyses;
    /// Each relation's layout, declared or inferred (decision 0361 step 3).
    pub mod layouts;
    pub mod manager;
    pub mod passes;
    pub use analyses::{Edges, Strata};
}
pub mod program_text;
pub use rerail_logic_core::code_rule_dsl::logic::version_trace;
/// Main's JSON rules translated to `.rerail` on read (decision 0175).
pub mod json_rules;
#[allow(dead_code)]
pub mod rr;
pub mod settings;
pub use rerail_logic_core::code_rule_dsl::logic::sql;
/// The reach index's form found in a program (decision 0119).
pub use rerail_logic_core::code_rule_dsl::logic::reach_form;
pub mod rank_form;
// Its files are its test modules alone.
#[cfg_attr(any(test, feature = "test-hooks"), path = "logic/strings")]
pub mod strings {
    pub use rerail_facts::code_rule_dsl::logic::strings::*;
    #[cfg(any(test, feature = "test-hooks"))]
    pub mod tests;
    #[allow(unused_imports)]
    use std::collections::{BTreeMap, HashMap, HashSet};
    #[allow(unused_imports)]
    use std::sync::{Arc, LazyLock, Mutex};
    #[allow(unused_imports)]
    use crate::code_rule_dsl::text::{Domain, Lowered, Source, Spelling, lower};
    #[allow(unused_imports)]
    use rusqlite::Connection;
}
// cratify: `convert` is rerail-engine's (it mounts it)
