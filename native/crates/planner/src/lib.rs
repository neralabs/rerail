//! The logic language's planner's run (`logic::plan`, `plan.rs`): the run of a program's strata (`Run`,
//! `run_within`), the walker, the answers' reads, the composed form (`compose`), the issue of a stratum's work
//! (`issue`), the join specs (`join_spec`), the emitted SQL and its kernels (`emit`, `sql_kernel`), and the plans a
//! corpus run keeps (`KeepPlans`). Over its search (`rerail-plan-search`: the model and the search for a rule's
//! plan) and its base (`rerail-plan-base`), whose `plan` it re-exports; under the corpus runner and the language's
//! root, which are `rerail-logic`'s.
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
pub use rerail_plan_search::content_store;
pub use rerail_plan_search::git_access;
pub use rerail_plan_search::git_workspace;
extern crate self as rerail_planner;
#[path = "code_rule_dsl"]
pub mod code_rule_dsl {
    pub use rerail_plan_search::code_rule_dsl::*;
    #[path = "logic"]
    pub mod logic {
        pub use rerail_plan_search::code_rule_dsl::logic::*;
        /// The planner's search, its model and runs (`plan.rs`), with the passes that read the model; it re-exports
        /// its base's `plan` (rerail-plan-base's).
        pub mod plan;
    }
}
pub use rerail_plan_search::threads;

/// This crate's own test binary, which no program composes: what a job carries of its run is the run's, the
/// planner's and setfix's (no corpus, whose test store `logic::CARRIERS` adds).
#[cfg(test)]
mod test_load {
    const CARRIERS: &[fn(&mut crate::code_rule_dsl::logic::run_state::Capture)] = &[
        crate::code_rule_dsl::logic::run::carried,
        crate::code_rule_dsl::logic::plan::carried,
        crate::code_rule_dsl::logic::plan::keeps::carried,
        crate::code_rule_dsl::logic::native::setfix::carried,
    ];
    extern "C" fn load() {
        crate::code_rule_dsl::logic::run_state::CARRIERS.register("rerail-planner's tests", CARRIERS);
    }
    #[used]
    #[cfg_attr(target_vendor = "apple", unsafe(link_section = "__DATA,__mod_init_func"))]
    #[cfg_attr(windows, unsafe(link_section = ".CRT$XCU"))]
    #[cfg_attr(not(any(target_vendor = "apple", windows)), unsafe(link_section = ".init_array"))]
    static LOAD: extern "C" fn() = load;
}
