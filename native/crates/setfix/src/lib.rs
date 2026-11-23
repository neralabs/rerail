//! The logic language's set engine (`logic::native::setfix`), with what of a device it runs (`device::solver`: its
//! joins and fixpoints over setfix's store, and the columns' statistics the planner reads) and the waves' owner, whose
//! database registers setfix's symbols. Over the run (`rerail-run`: its state, lanes, device interface, the other
//! solvers), under the planner, which reads it. Its module tree is the run's, with setfix's modules mounted in it.
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
// The store's records and interface; the tests open the store itself (crate split 10b-2).
/// The store's records and interface (rerail-store-base, through observe), and its atoms (rerail-atoms).
#[cfg(not(test))]
pub mod content_store {
    pub use rerail_atoms as atoms;
    pub use rerail_observe::content_store::*;
}
#[cfg(test)]
pub use rerail_store::content_store;
pub use rerail_run::git_access;
pub use rerail_run::git_workspace;
pub use rerail_run::threads;
extern crate self as rerail_setfix;
#[path = "code_rule_dsl"]
pub mod code_rule_dsl {
    pub use rerail_run::code_rule_dsl::*;
    // The facts with the syntax table (rerail-facts): the run reads facts-base's.
    pub use rerail_facts::code_rule_dsl::facts;
    // The SQL profiler and admission ledger the waves' owner installs (rerail-sql-profile, crate split 10c).
    pub use rerail_sql_profile::code_rule_dsl::{sql_admission, sql_profile};
    #[path = "logic"]
    pub mod logic {
        pub use rerail_run::code_rule_dsl::logic::*;
        // The logic language's core (rerail-logic-core), which the run does not read.
        pub use rerail_logic_core::code_rule_dsl::logic::{analysis, bloom, crypto, derived, join_size, liveness, reach_form, sql, version_trace};
        #[path = "device"]
        pub mod device {
            pub use rerail_run::code_rule_dsl::logic::device::*;
            pub mod solver;
            pub use solver::{ColumnStats, JoinPlan, SolverDevice, StatColumn, register, solver};
        }
        pub mod plan {
            pub use rerail_run::code_rule_dsl::logic::plan::*;
            pub mod statkeep {
                pub use rerail_run::code_rule_dsl::logic::plan::statkeep::*;
                /// The statistics' tests, which measure setfix's column sources.
                #[cfg(test)]
                pub mod tests;
            }
        }
        #[path = "native"]
        pub mod native {
            pub use rerail_run::code_rule_dsl::logic::native::*;
            pub mod setfix;
            /// T's loadable extension as copied (8934168d `lib.rs`); this module is its adaptation (O19).
            #[cfg(any(test, feature = "test-hooks"))]
            #[allow(dead_code)]
            pub mod extension;
            #[cfg(any(test, feature = "test-hooks"))]
            pub use setfix::Derivations;
            pub use setfix::{forget_engine, forget_scope};
            pub use setfix::{flush_symbols, wave0_changes, forget_inputs, intern_batch, keep_inputs, register_symbols, sym_text_of};
            /// A connection's identity for the state a run leaves with it (`setfix::scope`).
            pub use setfix::scope as setfix_scope;
            pub use setfix::{symbol_text, symbol_texts};
            /// The logic's native functions and virtual tables: the lattices', then the set engine's.
            pub use setfix::register;
            /// The set engine's component calls, made in process by the planner (no SQL statement each).
            pub mod component {
                pub use super::setfix::lab;
                pub use super::setfix::spec;
                pub use super::setfix::{DeviceReport, device_report};
                pub use super::setfix::probed;
                pub use super::setfix::{CutReport, TrialRun, symbol_id, cut_counts, cut_step, keep_trial_rows, set_budgets, take_cut, thread_cpu, trial, trial_ids, trial_over, trial_own};
                pub use super::setfix::{InRel, Live, Ver, LIVE_BATCH, RelMark, add, cat_symbols, prime, adopt, adopted, close, columns_source, columns_source_measured, derived, forget_derived, forget_inputs_of, held, held_rows, each_held_batch, hold_as, hold_maintained, hold_empty, hold_source, hold_view_in_place, held_source, hold_bound, is_dead, live, give_versions, take_versions, materialize, open_spec, pending, defer, prefetch_joins, prefetch_tables, level_clear, level_counts, release, reprobe_spec, retain, run, step_counts, maint_step_counts, driver_step_counts, step_cpu, step_histograms, unindex, hold_rows, kept_source, live_changed, table_source, tables_built, output_rows, source_rows, finding_rows, key_breaks};
            }
        }
    }
    #[path = "waves"]
    pub mod waves {
        pub use rerail_run::code_rule_dsl::waves::*;
        pub mod owner;
        pub mod schedule;
    }
}

/// This crate's own test binary, which no program composes: what a job carries of its run is the run's and setfix's
/// (`setfix::RUN_CARRIERS`), with no planner.
#[cfg(test)]
mod test_load {
    extern "C" fn load() {
        crate::code_rule_dsl::logic::run_state::CARRIERS.register("rerail-setfix's tests", crate::code_rule_dsl::logic::native::setfix::RUN_CARRIERS);
    }
    #[used]
    #[cfg_attr(target_vendor = "apple", unsafe(link_section = "__DATA,__mod_init_func"))]
    #[cfg_attr(windows, unsafe(link_section = ".CRT$XCU"))]
    #[cfg_attr(not(any(target_vendor = "apple", windows)), unsafe(link_section = ".init_array"))]
    static LOAD: extern "C" fn() = load;
}
