//! The logic language (docs/final-shape/03-architecture.md §12; 05 Phases 5–6): rules as named
//! facts and Datalog over them, planned from statistics and lowered to SQL or transducer groups.
//! `sql` is the typed SQL its plans emit. The core modules (`sql`, `analysis`, `costs`, …) are
//! `rerail-logic-core`'s, and the native solvers with the run, its lanes and its state (`native`, `run`, `lanes`,
//! `run_state`, `device`) `rerail-setfix`'s, under the planner; this module re-exports them under their paths.

pub use rerail_logic_core::code_rule_dsl::logic::crypto;
pub use rerail_logic_core::code_rule_dsl::logic::derived;
pub use rerail_setfix::code_rule_dsl::logic::run;
/// What a job carries of the run's context, in the order it is entered (`run_state::CARRIERS`, which the program's
/// composition registers): the run first, entered first and put back last, so every other part's guard drops inside
/// it (a plans guard's drop counts into the run); then the planner's kept plans and observations; then a job's scope
/// of the run's chunk (Q203 item 3); and in a test build the test's store.
pub const CARRIERS: &[fn(&mut run_state::Capture)] = &[
    run::carried,
    plan::carried,
    plan::keeps::carried,
    native::setfix::carried,
    #[cfg(any(test, feature = "test-hooks"))]
    plan::corpus::test_store::carried,
];
/// setfix's, with its tests, which read what the whole program carries (`logic::CARRIERS`).
pub mod run_state {
    pub use rerail_setfix::code_rule_dsl::logic::run_state::*;
    #[cfg(test)]
    mod tests;
}
#[cfg(test)]
pub mod facts_only;
/// This crate's own test binary, which the engine does not load: strict mode on every connection, as the engine's
/// loader sets it (the store sends its SQL hazards to the profile itself, `content_store::hazards`), what a job
/// carries of its run, and the corpus runner (`plan::corpus::RUNNER`), as boot registers them.
#[cfg(test)]
mod test_load {
    extern "C" fn load() {
        crate::code_rule_dsl::logic::profile::strict_on_every_connection();
        crate::code_rule_dsl::logic::run_state::CARRIERS.register("rerail-logic's tests", crate::code_rule_dsl::logic::CARRIERS);
        crate::code_rule_dsl::logic::plan::corpus::RUNNER.register("rerail-logic's tests", &crate::code_rule_dsl::logic::plan::corpus::Corpus);
        crate::code_rule_dsl::logic::plan::keeps::CORE_NAMES.register("rerail-logic's tests", crate::code_rule_dsl::logic::rr::core_names);
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
    // Its files are its test modules alone.
    #[cfg_attr(any(test, feature = "test-hooks"), path = ".")]
    pub mod answers {
        pub use rerail_rr::code_rule_dsl::logic::front::answers::*;
        #[cfg(any(test, feature = "test-hooks"))]
        #[path = "answers_tests.rs"]
        pub mod tests;
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
/// The planner's, with its tests, which read the language (`rr::parse_version`).
pub mod answers {
    pub use rerail_planner::code_rule_dsl::logic::answers::*;
    // `answers.rs`'s imports, for its tests' `use super::*`.
    #[allow(unused_imports)]
    use super::front::{Edges, tarjan};
    #[allow(unused_imports)]
    use super::front::ast::Gate;
    #[allow(unused_imports)]
    use super::plan::body::Placed;
    #[allow(unused_imports)]
    use super::plan::fields::{Fields, FieldsData};
    #[allow(unused_imports)]
    use super::program_image::manager::{Cx, Pipeline, ProgramPass, ProgramW, RuleOut, RulePass, Select, Step};
    #[allow(unused_imports)]
    use super::program_image::passes::{PartOf, Retain, SetAnswers};
    #[allow(unused_imports)]
    use super::program_image::{AtomRef, Builder, Lit, LitRef, PartFields, Rewrite, RuleBuilder, RuleRef, T, TermRef, Version};
    #[allow(unused_imports)]
    use std::collections::{BTreeMap, BTreeSet};
    #[allow(unused_imports)]
    use std::sync::Arc;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::logic::program_image::passes::Parts as _;
    #[cfg(test)]
    pub mod tests;
}
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
/// The planner (`rerail-planner`), with the corpus runner, which is this crate's (it reads the language's root), and
/// the planner's tests that read the language (`rr::parse_version`) or the corpus.
#[allow(dead_code)]
pub mod plan {
    pub use rerail_planner::code_rule_dsl::logic::plan::*;
    // `plan.rs`'s imports, for the corpus's `use super::*` (as it saw them in one crate).
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
    pub mod corpus;
    #[cfg(test)]
    mod complete_answer_tests;
    #[cfg(test)]
    mod search_tests;
    pub mod body {
        pub use rerail_planner::code_rule_dsl::logic::plan::body::*;
        // `plan/body.rs`'s imports, for its tests' `use super::*`.
        #[allow(unused_imports)]
        use super::super::program_image::{AggRef, AtomRef, Builder, CmpOp, Lit, LitRef, RelId, RuleId, RuleRef, Slot, TermRef, Version};
        #[allow(unused_imports)]
        use super::{Constant, Error, Result};
        #[allow(unused_imports)]
        use std::cell::OnceCell;
        #[allow(unused_imports)]
        use std::sync::Arc;
        #[cfg(test)]
        mod tests;
    }
    pub mod choices {
        pub use rerail_planner::code_rule_dsl::logic::plan::choices::*;
        // `plan/choices.rs`'s imports, for its tests' `use super::*`.
        #[allow(unused_imports)]
        use super::super::front::Value;
        #[allow(unused_imports)]
        use super::super::program_facts::{self, Facts};
        #[allow(unused_imports)]
        use super::super::program_text;
        #[allow(unused_imports)]
        use crate::content_store::{ContentStore, FactsMemoKey, FactsPlanSet, FactsResult, HashKey, StoreResult};
        #[allow(unused_imports)]
        use sha2::{Digest as _, Sha256};
        #[allow(unused_imports)]
        use std::collections::{BTreeSet, HashMap};
        #[allow(unused_imports)]
        use std::sync::{Arc, Mutex};
        #[cfg(test)]
        pub mod tests;
    }
    pub mod compose {
        pub use rerail_planner::code_rule_dsl::logic::plan::compose::*;
        // `plan/compose.rs`'s imports, for its tests' `use super::*`.
        #[allow(unused_imports)]
        use super::super::program_image::{Builder, Lit, NativeKind, NativeSpec, RelId, analyses};
        #[allow(unused_imports)]
        use super::super::sql::ident;
        #[allow(unused_imports)]
        use super::body::{Placed, Versions};
        #[allow(unused_imports)]
        use super::*;
        #[cfg(test)]
        pub mod tests;
    }
    pub mod demand {
        pub use rerail_planner::code_rule_dsl::logic::plan::demand::*;
        // `plan/demand.rs`'s imports, for its tests' `use super::*`.
        #[allow(unused_imports)]
        use super::super::program_image::{Builder, CmpOp, RuleId, RuleRef, T, TermRef, Version};
        #[allow(unused_imports)]
        use super::keeps::inline::named;
        #[allow(unused_imports)]
        use std::collections::{BTreeMap, BTreeSet};
        #[allow(unused_imports)]
        use super::body::{Freeze as _, Placed};
        #[allow(unused_imports)]
        use super::fields::Fields;
        #[cfg(test)]
        mod tests;
        pub mod over {
            pub use rerail_planner::code_rule_dsl::logic::plan::demand::over::*;
            // `plan/demand/over.rs`'s imports, for its tests' `use super::*`.
            #[allow(unused_imports)]
            use super::super::super::program_image::{AtomRef, CmpOp, Lit, LitRef, RuleRef, Slot, TermRef, Version};
            #[allow(unused_imports)]
            use super::super::body::{Placed, lit_vars, term_vars, VarSet};
            #[allow(unused_imports)]
            use super::super::fields::Fields;
            #[allow(unused_imports)]
            use super::Split;
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, BTreeSet};
            #[cfg(test)]
            mod tests;
        }
    }
    pub mod joins {
        pub use rerail_planner::code_rule_dsl::logic::plan::joins::*;
        // `plan/joins.rs`'s imports, for its tests' `use super::*`.
        #[allow(unused_imports)]
        use super::{Constant, Mode, PlanKey, Sig, Step, StoredStep};
        #[allow(unused_imports)]
        use parking_lot::{Mutex, RwLock};
        #[allow(unused_imports)]
        use rustc_hash::{FxHashMap, FxHashSet};
        #[allow(unused_imports)]
        use std::collections::HashMap;
        #[allow(unused_imports)]
        use std::sync::Arc;
        #[allow(unused_imports)]
        use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
        #[cfg(test)]
        mod tests;
    }
    pub mod keeps {
        pub use rerail_planner::code_rule_dsl::logic::plan::keeps::*;
        // `plan/keeps.rs`'s imports, for its tests' `use super::*`.
        #[allow(unused_imports)]
        use super::Model;
        #[allow(unused_imports)]
        use super::body::{Body, Placed};
        #[allow(unused_imports)]
        use super::fields::Fields;
        #[allow(unused_imports)]
        use super::super::program_image::{RuleId, Version};
        #[allow(unused_imports)]
        use std::cell::RefCell;
        #[allow(unused_imports)]
        use std::sync::Arc;
        #[allow(unused_imports)]
        use std::collections::{BTreeMap, BTreeSet};
        #[cfg(test)]
        pub mod tests;
    }
    pub mod lower {
        pub use rerail_planner::code_rule_dsl::logic::plan::lower::*;
        // `plan/lower.rs`'s imports, for its tests' `use super::*`.
        #[allow(unused_imports)]
        use super::super::front::{Branded, Class, Value};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::logic::program_image::Version;
        #[allow(unused_imports)]
        use super::{Error, Result};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::facts::core::kind_atom;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::facts::vtab::syntax::field_atom;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::{GrammarDialect, KindCatalog, tape_tag};
        #[allow(unused_imports)]
        use std::collections::BTreeMap;
        #[allow(unused_imports)]
        use rustc_hash::FxHashMap as HashMap;
        #[cfg(test)]
        pub mod tests;
    }
    pub mod native_demand {
        pub use rerail_planner::code_rule_dsl::logic::plan::native_demand::*;
        // `plan/native_demand.rs`'s imports, for its tests' `use super::*`.
        #[allow(unused_imports)]
        use super::{Error, Result};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::{facts::vtab::Source, logic::profile};
        #[allow(unused_imports)]
        use rusqlite::Connection;
        #[allow(unused_imports)]
        use std::{collections::HashMap, sync::Arc};
        // Its tests' file holds the planner's tests; the integration tests under it read the corpus.
        #[cfg(test)]
        mod tests {
            #[allow(unused_imports)]
            use super::*;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::logic::native::component;
            mod integration_tests;
        }
    }
    pub mod replanning {
        pub use rerail_planner::code_rule_dsl::logic::plan::replanning::*;
        // `plan/replanning.rs`'s imports, for its tests' `use super::*`.
        #[allow(unused_imports)]
        use super::body::Body;
        #[allow(unused_imports)]
        use super::{Constant, Model, Step, near};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::logic::program_image::TermRef;
        #[allow(unused_imports)]
        use std::collections::{BTreeMap, HashMap};
        #[cfg(test)]
        pub mod tests;
    }
    pub mod wellfounded {
        pub use rerail_planner::code_rule_dsl::logic::plan::wellfounded::*;
        // `plan/wellfounded.rs`'s imports, for its tests' `use super::*`.
        #[allow(unused_imports)]
        use crate::code_rule_dsl::logic::plan::fields::{Fields, FieldsData};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::logic::program_image::analyses::Components;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::logic::program_image::manager::{Analysis, Cx, Reads};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::logic::program_image::{CmpOp, Lit, TermRef, Version};
        #[allow(unused_imports)]
        use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
        #[cfg(test)]
        mod tests;
    }
}
// The profile's report and watch read the planner's placements: the planner's facade of the profile.
pub use rerail_planner::code_rule_dsl::logic::profile;
// The program image's core is a crate of its own, under the lowering (decision 0377); the pass manager, the passes and
// the analyses, which read the planner, are the planner's, added to the core's module.
pub use rerail_program_image::code_rule_dsl::logic::program_facts;
pub use rerail_planner::code_rule_dsl::logic::program_image;
pub use rerail_planner::code_rule_dsl::logic::program_text;
pub use rerail_logic_core::code_rule_dsl::logic::version_trace;
/// Main's JSON rules translated to `.rerail` on read (decision 0175): `rerail-json-rules`'s.
pub use rerail_json_rules::code_rule_dsl::logic::json_rules;
#[allow(dead_code)]
// cratify: the language's root (`rr`: reading, the libraries, lowering's tail) and the settings are rerail-rr-root's,
// with the corpus's base they read (the corpus runner is behind its registered `plan::corpus::RUNNER`).
pub use rerail_rr_root::code_rule_dsl::logic::rr;
pub use rerail_rr_root::code_rule_dsl::logic::settings;
pub use rerail_logic_core::code_rule_dsl::logic::sql;
/// The reach index's form found in a program (decision 0119).
pub use rerail_logic_core::code_rule_dsl::logic::reach_form;
pub use rerail_planner::code_rule_dsl::logic::rank_form;
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
