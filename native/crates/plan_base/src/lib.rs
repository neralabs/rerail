//! The logic language's planner's base, below its search (`rerail-planner`): the planner's vocabulary
//! (`plan::base`: its errors, steps, modes, orders, cells), a program's forms and fields (`plan::{body, fields}`), the
//! joins and their memo, the outputs, placement, tables, the emitted SQL and its kernels, the shapes, the lowering
//! and the text demand, wellfoundedness, and the pass manager with the program's passes and analyses
//! (`program_image::{manager, passes, analyses, layouts}`), the answers' gates (`answers`), the program's text, the
//! rank form and the profile's report and watch. Over setfix. Its `plan` is a facade of the planner's paths;
//! `plan.rs` (rerail-planner's) re-exports it.
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
pub use rerail_store::content_store;
pub use rerail_store::git_access;
pub use rerail_store::git_workspace;
extern crate self as rerail_plan_base;
#[path = "code_rule_dsl"]
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
        pub use rerail_program_image::code_rule_dsl::logic::program_facts;
        pub use rerail_facts::code_rule_dsl::logic::strings;
        pub use rerail_logic_core::code_rule_dsl::logic::{analysis, bloom, costs, crypto, derived, join_size, liveness, reach_form, sql, version_trace};
        pub use rerail_setfix::code_rule_dsl::logic::{device, lanes, native, run, run_state};
        pub mod answers;
        /// The passes a program was made into (decision 0597): what the contexts (rerail-rr-root's) and the
        /// combine's hiding (rerail-corpus-prep's) read.
        pub mod passes_made;
        /// The planner's base under the planner's path: what its search (`plan.rs`, rerail-planner's) reads, and
        /// the root's vocabulary (`base`). The search, its model and runs, and the passes that read the model are
        /// rerail-planner's.
        #[path = "plan"]
        pub mod plan {
            pub use base::*;
            /// Why a stratum an edit reaches is derived again (decision 0364 R3, R4).
            pub mod again;
            pub mod acyclic;
            pub mod base;
            pub mod body;
            pub mod connection;
            pub mod estimates;
            pub mod fields;
            pub mod identity_parts;
            pub mod joins;
            pub mod lower;
            pub mod native_demand;
            pub mod output;
            pub mod phased;
            pub mod placement;
            pub mod shapes;
            pub mod table;
            pub mod text_demand;
            pub mod wellfounded;
            pub use rerail_facts::code_rule_dsl::logic::plan::sketch;
            /// setfix's: its kept relations carry their statistics in the planner's form (`setfix::KeptStats`).
            pub use rerail_setfix::code_rule_dsl::logic::plan::statkeep;
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
        pub mod program_text;
        pub mod rank_form;
        // The program image's core is a crate of its own, under the lowering (decision 0377); the pass manager, the
        // passes and the analyses, which read the planner, are this crate's, added to the core's module.
        #[path = "program_image"]
        pub mod program_image {
            pub use rerail_program_image::code_rule_dsl::logic::program_image::*;
            pub mod analyses;
            /// Each relation's layout, declared or inferred (decision 0361 step 3).
            pub mod layouts;
            pub mod manager;
            pub mod passes;
            /// The image as the engine's inputs (decision 0662 §5.1).
            pub mod view;
            pub use analyses::{Edges, Strata};
        }
        // The profile's report and watch read the planner's placements; the rest is `rerail-prof`'s.
        #[path = "profile"]
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
        /// What the planner reads of the language: `rerail-rr`'s lowering and the syntax's quoting. The language's
        /// root (`rr.rs`: reading, lowering's tail, the libraries) is `rerail-logic`'s.
        pub mod rr {
            pub use rerail_rr::code_rule_dsl::logic::rr::lower;
            pub use rerail_rr::code_rule_dsl::logic::rr::lower::{FILE_SYSTEM, WORLD};
            pub use rerail_rr::code_rule_dsl::logic::rr::lower::findings::UNPLACED;
            pub use rerail_rr::code_rule_dsl::logic::rr::aggregate_demand;
            pub use rerail_rr::code_rule_dsl::logic::rr::print::{self, print_version};
            pub use rerail_rr_syntax::syntax::quote;
            /// The shipped images and their export (decision 0264), which `program_text` keeps.
            pub mod images;
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
    pub use rerail_setfix::code_rule_dsl::waves;
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
pub use rerail_setfix::threads;

/// This crate's own test binary, which no program composes: what a job carries of its run is the run's and setfix's
/// (`setfix::RUN_CARRIERS`), with no planner.
#[cfg(test)]
mod test_load {
    extern "C" fn load() {
        crate::code_rule_dsl::logic::run_state::CARRIERS.register("rerail-plan-base's tests", crate::code_rule_dsl::logic::native::setfix::RUN_CARRIERS);
    }
    #[used]
    #[cfg_attr(target_vendor = "apple", unsafe(link_section = "__DATA,__mod_init_func"))]
    #[cfg_attr(windows, unsafe(link_section = ".CRT"))]
    #[cfg_attr(not(any(target_vendor = "apple", windows)), unsafe(link_section = ".init_array"))]
    static LOAD: extern "C" fn() = load;
}
