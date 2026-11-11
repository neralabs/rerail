//! SQL-driven waves (docs/final-shape/03-architecture.md §4, §7; decision 0010): one owner
//! connection on the content store, the temp schema's lifecycle, and the frontier loop that SQL
//! steers, wave by wave, until no relation changes.
pub use rerail_logic::code_rule_dsl::waves::owner;
#[cfg(any(test, feature = "test-hooks"))]
#[path = "waves/relational"]
pub(in crate::code_rule_dsl) mod relational {
    pub use rerail_w::code_rule_dsl::waves::relational::*;
    #[path = "batch"]
    pub mod batch {
        pub use rerail_w::code_rule_dsl::waves::relational::batch::*;
        #[cfg(test)]
        pub mod tests;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::waves::relational::{Evaluation, Inputs, Job, Preparation, Session, Step, failure, producer, sql_error};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::model::{CompiledCodeRule, Metrics};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::{budget::Context, model::CompiledClause};
        #[allow(unused_imports)]
        use std::collections::VecDeque;
    }
    pub mod execution;
    #[path = "inputs"]
    pub mod inputs {
        pub use rerail_w::code_rule_dsl::waves::relational::inputs::*;
        #[cfg(test)]
        pub mod tests;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::waves::relational::{FactQuery, Rows, insert};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::{
            eval,
            model::{CompiledClause, CompiledCodeRule, Metrics, Snapshot},
            violations::{Witness, anchored_span, projection::Row},
        };
        #[allow(unused_imports)]
        use napi::Result;
        #[allow(unused_imports)]
        use rusqlite::types::ValueRef;
        #[allow(unused_imports)]
        use std::{
            cell::RefCell,
            sync::{Arc, atomic::AtomicUsize},
        };
    }
    #[allow(unused_imports)]
    use crate::code_rule_dsl::eval::{evidence::Rows, failure, violation_from_witness};
    #[allow(unused_imports)]
    use crate::code_rule_dsl::json::{hex_digest, invalid};
    #[allow(unused_imports)]
    use crate::code_rule_dsl::model::{
        CompiledClause, CompiledCodeRule, Evaluation, FactQuery, Snapshot,
    };
    #[allow(unused_imports)]
    use crate::code_rule_dsl::violations::{Captures, Witness};
    #[allow(unused_imports)]
    use napi::Result;
    #[allow(unused_imports)]
    use rusqlite::Connection;
    #[allow(unused_imports)]
    use rusqlite::hooks::Authorization;
    #[allow(unused_imports)]
    pub(in crate::code_rule_dsl) use crate::code_rule_dsl::waves::relational::shared::Versions;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::waves::relational::database::Database;
    #[allow(unused_imports)]
    #[cfg(any(test, feature = "test-hooks"))]
    pub(in crate::code_rule_dsl) use crate::code_rule_dsl::waves::relational::functions::install_unbounded as install_regex_functions;
    #[allow(unused_imports)]
    pub(in crate::code_rule_dsl) use crate::code_rule_dsl::waves::relational::reachability::Reachability;
    #[allow(unused_imports)]
    use serde_json::{Value, json};
    #[allow(unused_imports)]
    use std::collections::{BTreeMap, BTreeSet, HashMap};
    #[allow(unused_imports)]
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
}
/// The first slice's wave over lowered matchers (06 S4).
pub(in crate::code_rule_dsl) mod matches;
/// The first slice's findings layer over the wave's matches (06 S5).
pub(in crate::code_rule_dsl) mod findings;
#[path = "waves/schedule"]
pub(in crate::code_rule_dsl) mod schedule {
    pub use rerail_logic::code_rule_dsl::waves::schedule::*;
    pub mod batches;
    #[cfg(test)]
    pub mod tests;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::waves::owner::{Database, Lease};
    #[allow(unused_imports)]
    use crate::code_rule_dsl::waves::owner::sql::{SqlOwner};
    #[allow(unused_imports)]
    use crate::code_rule_dsl::facts::Graph;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::facts::vtab::Source;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::facts::vtab::syntax::{Document, fresh, sources};
    #[allow(unused_imports)]
    use crate::content_store::{DerivationInput, DerivationRecord, StoreError, StoreResult};
    #[allow(unused_imports)]
    use sha2::{Digest as _, Sha256};
    #[allow(unused_imports)]
    use std::collections::{BTreeMap, BTreeSet};
    #[allow(unused_imports)]
    use std::sync::Arc;
    #[allow(unused_imports)]
    pub(in crate::code_rule_dsl) use crate::code_rule_dsl::waves::schedule::lattice::Operations as LatticeOperations;
}
pub use rerail_logic::code_rule_dsl::waves::storage;
/// The policies through the logic language: the wave's clauses, rules and demand (list step 21).
pub(in crate::code_rule_dsl) mod policies;
