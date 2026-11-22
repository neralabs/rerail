//! M's Y1 reference graph as copied (`facts::ygraph`; 05 Phase 1 row 2's source; open item O19), under the facts'
//! base (rerail-facts-base), which re-exports it. Unix only (not the browser build, not Windows): it forks its checks
//! and reads macOS' and Linux' memory counters.
//!
//! Its sources stay where they were (native/src): this crate mounts them in place, as the engine did.
#![allow(dead_code, unused_imports, clippy::all)]
#[path = "../../../src/code_rule_dsl"]
pub mod code_rule_dsl {
    pub mod logic {
        pub use rerail_prof::code_rule_dsl::logic::profile;
    }
    #[path = "facts"]
    pub mod facts {
        #[cfg(unix)]
        pub mod ygraph;
    }
}
