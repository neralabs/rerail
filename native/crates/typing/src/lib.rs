//! The logic language's typing (`code_rule_dsl::typing`): its approximate inference by phases, the file-local
//! passes, the dependencies and the rounds. It is over the planner's search and base (`rerail-plan-search`), whose
//! programs and passes it reads, beside the planner's run (`rerail-planner`, which it does not read: its tests
//! register its carrier), and under the corpus runner, which runs it (`plan::corpus::typing`), in `rerail-logic`.
//!
//! Its sources are in its own `src/`, at their module paths in the engine's tree (the crate split, step 9).
// A mounted module re-exports its dependency's by glob, and its sources import the same items under the same names:
// the glob's name and the private one are one item. The mounts also widen what the crates above read (the split's
// facades, not a design).
#![allow(dead_code, unused_imports, hidden_glob_reexports, private_interfaces, private_bounds)]
#![allow(clippy::len_without_is_empty, clippy::result_unit_err, clippy::new_without_default)]
#![allow(clippy::type_complexity, clippy::too_many_arguments)]
// The browser build has no JIT: wasm/cranelift-jit stands in under the same name (wasm/README.md).
#[cfg(target_family = "wasm")]
extern crate cranelift_jit_wasm as cranelift_jit;
pub use rerail_plan_search::{content_store, git_access, git_workspace, threads};
extern crate self as rerail_typing;
#[path = "code_rule_dsl"]
pub mod code_rule_dsl {
    pub use rerail_plan_search::code_rule_dsl::*;
    pub mod typing;
}

/// This crate's own test binary, which no program composes: what a job carries of its run is the run's, the
/// planner's and setfix's, as the planner's tests register.
#[cfg(test)]
mod test_load {
    const CARRIERS: &[fn(&mut crate::code_rule_dsl::logic::run_state::Capture)] = &[
        crate::code_rule_dsl::logic::run::carried,
        rerail_planner::code_rule_dsl::logic::plan::carried,
        crate::code_rule_dsl::logic::plan::keeps::carried,
        crate::code_rule_dsl::logic::native::setfix::carried,
    ];
    extern "C" fn load() {
        crate::code_rule_dsl::logic::run_state::CARRIERS.register("rerail-typing's tests", CARRIERS);
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
