//! The logic language's planner's search (`logic::plan::search`): the planner's inputs and statistics, its model of
//! the data (`Model`), the search for a rule's plan, the plans kept across a run's chunks and strata, and the passes
//! that read the model (`keeps`, `select`, `prepared`, `choices`, `demand`, `replanning`). Over the planner's base
//! (`rerail-plan-base`), under its run (`rerail-planner`: `plan.rs`), which re-exports its `plan`.
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
pub use rerail_plan_base::content_store;
pub use rerail_plan_base::git_access;
pub use rerail_plan_base::git_workspace;
extern crate self as rerail_plan_search;
#[path = "code_rule_dsl"]
pub mod code_rule_dsl {
    pub use rerail_plan_base::code_rule_dsl::*;
    #[path = "logic"]
    pub mod logic {
        pub use rerail_plan_base::code_rule_dsl::logic::*;
        /// The planner's search under the planner's path (`plan::search`), with the passes that read the model, over
        /// its base's `plan` (rerail-plan-base's). The run (`plan.rs`) is rerail-planner's, and re-exports this.
        #[path = "plan"]
        pub mod plan {
            pub use rerail_plan_base::code_rule_dsl::logic::plan::*;
            pub mod search;
            pub use search::*;
            /// The tree's statistics, its kinds' and edges', in a section (decision 0628 §4).
            pub mod tree_stats;
            pub mod choices;
            /// A memo entry of program facts kept as kept facts, held by its source (t33 step 3).
            pub mod kept_entry;
            pub mod demand;
            pub mod keeps;
            pub mod prepared;
            /// The analyses of a version kept by its identity, with the kept plans (t33 K4).
            pub mod kept_analyses;
            pub mod replanning;
            pub mod select;
            /// A rule read as a filter of one relation (decision 0611).
            pub mod filters;
            pub use filters::{filter_of, plain_filter};
            // `plan.rs`'s imports, for the children' `use super::*`.
            #[allow(unused_imports)]
            use super::front::Value;
            #[allow(unused_imports)]
            use super::program_image::{AtomRef, CmpOp, Lit, Slot as VarSlot, TermRef, Version};
            #[allow(unused_imports)]
            use body::{Body, RuleKey, VarSet};
            #[allow(unused_imports)]
            use fields::Fields;
            #[allow(unused_imports)]
            use super::native::TOP;
            #[allow(unused_imports)]
            use super::native::component::spec::{Arg, ComponentSpec, EdgeSpec, Extreme, HeadTok, InputKey, JoinItem, JoinSpec, Line as SpecLine, Reprobe, Source as JoinSource, Start, Tok};
            #[allow(unused_imports)]
            use std::sync::Arc;
            #[allow(unused_imports)]
            use indexmap::IndexMap;
            #[allow(unused_imports)]
            use rusqlite::Connection;
            #[allow(unused_imports)]
            use rusqlite::types::Value as SqlValue;
            #[allow(unused_imports)]
            use std::cell::RefCell;
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
            #[allow(unused_imports)]
            use std::fmt;
            #[allow(unused_imports)]
            use std::path::Path;
            #[allow(unused_imports)]
            use super::sql::select::{Cte, FromItem, Join, Query, Relation, Select, Source};
            #[allow(unused_imports)]
            use super::sql::sql_ast::{SqlBinaryOperator as Op, SqlExpression as Expr, SqlUnaryOperator, SqlValue as Value_};
        }
    }
}
pub use rerail_plan_base::threads;

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
        crate::code_rule_dsl::logic::run_state::CARRIERS.register("rerail-plan-search's tests", CARRIERS);
    }
    #[used]
    #[cfg_attr(target_vendor = "apple", unsafe(link_section = "__DATA,__mod_init_func"))]
    #[cfg_attr(windows, unsafe(link_section = ".CRT$XCU"))]
    #[cfg_attr(not(any(target_vendor = "apple", windows)), unsafe(link_section = ".init_array"))]
    static LOAD: extern "C" fn() = load;
}
