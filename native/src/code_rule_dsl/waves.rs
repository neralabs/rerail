//! SQL-driven waves (docs/final-shape/03-architecture.md §4, §7; decision 0010): one owner
//! connection on the content store, the temp schema's lifecycle, and the frontier loop that SQL
//! steers, wave by wave, until no relation changes.
// Tests that read what the engine registers as it loads (the languages, their `.rr` libraries) run here.
pub mod owner {
    pub use rerail_logic::code_rule_dsl::waves::owner::*;
    #[cfg(test)]
    pub mod tests;
}
#[cfg(test)]
#[path = "waves/relational"]
pub(in crate::code_rule_dsl) mod relational {
    pub mod shared {
        pub use rerail_w::code_rule_dsl::waves::relational::shared::*;
        #[cfg(test)]
        pub mod identity_tests;
            #[allow(unused_imports)]
            use super::{Plan, Stage, syntax};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::model::{CompiledCodeRule, FileInput, Snapshot};
            #[allow(unused_imports)]
            use sha2::{Digest, Sha256};
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, HashMap, HashSet};
            #[allow(unused_imports)]
            use std::hash::{Hash, Hasher};
            #[allow(unused_imports)]
            use std::sync::Arc;
    }
    pub mod producer {
        pub mod layout {
            pub use rerail_w::code_rule_dsl::waves::relational::producer::layout::*;
            #[cfg(test)]
            pub mod tests;
                #[allow(unused_imports)]
                use super::{Demand, Resident};
                #[allow(unused_imports)]
                use std::{collections::HashSet, sync::Arc};
        }
        pub use rerail_w::code_rule_dsl::waves::relational::producer::*;
    }
    pub mod pool {
        pub use rerail_w::code_rule_dsl::waves::relational::pool::*;
        #[cfg(test)]
        pub mod tests;
            #[allow(unused_imports)]
            use super::{Database, Result, configure_in_place, connection};
            #[allow(unused_imports)]
            use parking_lot::{ArcMutexGuard, Mutex, RawMutex};
            #[allow(unused_imports)]
            use std::{ cell::{OnceCell, RefCell}, ops::Deref, rc::Rc, sync::{Arc, atomic::AtomicUsize}, };
    }
    pub use rerail_w::code_rule_dsl::waves::relational::*;
    #[path = "batch"]
    pub mod batch {
        pub use rerail_w::code_rule_dsl::waves::relational::batch::*;
        #[cfg(test)]
        pub mod tests;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::waves::relational::{Inputs, Job, Preparation, Session, Step, failure, producer};
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
        use crate::code_rule_dsl::waves::relational::{FactQuery, insert};
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
    use crate::code_rule_dsl::eval::{failure, violation_from_witness};
    #[allow(unused_imports)]
    use crate::code_rule_dsl::json::{hex_digest, invalid};
    #[allow(unused_imports)]
    use crate::code_rule_dsl::model::{CompiledClause, CompiledCodeRule, FactQuery, Snapshot};
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
    #[cfg(test)]
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
#[allow(unused_imports)]
pub use rerail_logic::code_rule_dsl::waves::storage;
/// The policies through the logic language: the wave's clauses, rules and demand (list step 21).
pub(in crate::code_rule_dsl) mod policies;
