//! The logic language (docs/final-shape/03-architecture.md §12; 05 Phases 5–6): rules as named
//! facts and Datalog over them, planned from statistics and lowered to SQL or transducer groups.
//! `sql` is the typed SQL its plans emit.

pub mod crypto;
pub mod derived;
#[cfg(test)]
pub mod facts_only;
pub mod analysis;
// Nothing emits SQL through them until the planner (Phase 6) does.
#[path = "logic/front"]
pub mod front {
    pub use rerail_rr::code_rule_dsl::logic::front::*;
    #[path = "caches"]
    pub mod caches {
        pub use rerail_rr::code_rule_dsl::logic::front::caches::*;
        #[cfg(test)]
        #[path = "../caches_tests.rs"]
        pub mod tests;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::logic::front::{Error, Literal, Program, Term};
        #[allow(unused_imports)]
        use std::collections::BTreeSet;
    }
    #[cfg(test)]
    pub mod data_layer_tests;
    #[cfg(test)]
    pub mod tests;
    #[path = "yaml"]
    pub mod yaml {
        pub use rerail_rr::code_rule_dsl::logic::front::yaml::*;
        #[cfg(test)]
        pub mod tests;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::logic::front::{Atom, Error, Literal, Program, Rule, Term};
        #[allow(unused_imports)]
        use serde_json::Value;
    }
    #[allow(unused_imports)]
    use std::collections::{BTreeMap, BTreeSet};
    #[allow(unused_imports)]
    use std::fmt;
    #[allow(unused_imports)]
    pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::front::name::Name;
    #[allow(unused_imports)]
    pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::front::ast::{AXIS, Again, AggOp, Atom, DESCENDANTS, EVERY_KIND, GLOBAL, LIBRARY, LatticeDecl, Literal, NEAREST, Native, OPTION_GLOB, Program, QUALIFIERS, RESERVED, Rule, SCOPE_NOT, SpanIn, Term, Value, axis, cmp_binds, globals, is_axis, is_transduced, join};
    #[allow(unused_imports)]
    pub(crate) use crate::code_rule_dsl::facts::core::PYTHON_CONTEXTS;
}
pub mod costs;
// The logic's device interface; the engine registers a device as it loads (`crate::install_devices`).
pub mod device;
pub mod lanes;
#[allow(dead_code)]
pub mod answers;
pub mod liveness;
#[path = "logic/magic"]
pub mod magic {
    pub use rerail_rr::code_rule_dsl::logic::magic::*;
    #[cfg(test)]
    pub mod tests;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::logic::front::{AggOp, Atom, Error, Literal, Program, Rule, Term, Value, cmp_binds, join, strata, tarjan};
    #[allow(unused_imports)]
    use std::collections::{BTreeMap, BTreeSet};
    #[allow(unused_imports)]
    pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::front::ast::{Gate, Report};
}
#[allow(dead_code)]
pub mod native;
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
    #[allow(unused_imports)]
    pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::profile::roofline::Unit;
}
pub mod program_facts;
#[allow(dead_code)]
pub mod rr;
pub mod settings;
#[allow(dead_code)]
pub mod sql;
pub mod simplify;
#[path = "logic/strings"]
pub mod strings {
    pub use rerail_facts::code_rule_dsl::logic::strings::*;
    #[cfg(test)]
    pub mod tests;
    #[allow(unused_imports)]
    use std::collections::{BTreeMap, HashMap, HashSet};
    #[allow(unused_imports)]
    use std::sync::{Arc, LazyLock, Mutex};
    #[allow(unused_imports)]
    use crate::code_rule_dsl::text::{Domain, Lowered, Source, Spelling, lower};
    #[allow(unused_imports)]
    use rusqlite::Connection;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::logic::front::ast::{Atom, Literal, Program, Term};
}
// cratify: `convert` is rerail-engine's (it mounts it)
