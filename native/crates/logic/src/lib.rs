//! The Datalog engine: plans, the native executor, SQL, typing, the waves core, the solver device interface.
//!
//! Its sources stay where they were (native/src): this crate mounts them in place, as the engine did.
// The mounts glob their owners' modules and widen what the crates above read: the split's facades, not a design.
#![allow(dead_code, unused_imports, hidden_glob_reexports, private_interfaces, private_bounds)]
// What the split widened for the crates above is no public API (`len`, `()` errors, `new`); the long
// signatures and tuple types are the sources' own.
#![allow(clippy::len_without_is_empty, clippy::result_unit_err, clippy::new_without_default)]
#![allow(clippy::type_complexity, clippy::too_many_arguments)]
// cratify: mounts (tools/cratify/mounts.py; generated, do not edit) begin
pub use rerail_store::content_store;
pub use rerail_store::git_workspace;
extern crate self as rerail_logic;
#[path = "../../../src/code_rule_dsl"]
pub mod code_rule_dsl {
    pub use rerail_lang::code_rule_dsl::budget;
    pub use rerail_lang::code_rule_dsl::build_ids;
    pub use rerail_lang::code_rule_dsl::embed;
    #[path = "facts"]
    pub mod facts {
        pub use rerail_facts::code_rule_dsl::facts::*;
        #[path = "vtab"]
        pub mod vtab {
            pub use rerail_facts::code_rule_dsl::facts::vtab::*;
            #[path = "syntax"]
            pub mod syntax {
                pub use rerail_facts::code_rule_dsl::facts::vtab::syntax::*;
                pub mod rr_core;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::facts::vtab::{Modules, Relation, Source};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::facts::Graph;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::facts::core::{Facts, KIND_STRIDE, NONE, TEXT, kind_atom, kind_code};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::language::{KindCatalog, from_tape_tag};
                #[allow(unused_imports)]
                use crate::content_store::{ContentStore, StoreError, StoreResult};
                #[allow(unused_imports)]
                use rusqlite::Connection;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::logic::front::ast::Value;
                #[allow(unused_imports)]
                use std::collections::{BTreeSet, HashMap};
                #[allow(unused_imports)]
                use std::sync::{Arc, OnceLock};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::facts::vtab::syntax::core_rr::Family;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::facts::vtab::syntax::guests::GuestRows;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::facts::vtab::syntax::granules::INDEX_BITS;
            }
            #[cfg(test)]
            pub mod tests;
            #[allow(unused_imports)]
            use fallible_iterator::FallibleIterator;
            #[allow(unused_imports)]
            use rusqlite::{
                Connection, Result,
                types::Value,
                vtab::{
                    Context, Filters, IndexConstraintOp, IndexInfo, Module, VTab, VTabConfig, VTabConnection,
                    VTabCursor,
                },
            };
            #[allow(unused_imports)]
            use std::{
                borrow::Cow,
                cell::RefCell,
                collections::{BTreeMap, HashMap},
                ffi::{CStr, CString, c_int},
                ops::Range,
                sync::{
                    Arc, OnceLock,
                    atomic::{AtomicU64, Ordering},
                },
            };
        }
        #[allow(unused_imports)]
        use crate::content_store::{Binding, ContentStore, Derivation, DerivationRecord, StoreResult};
    }
    #[cfg(any(test, feature = "test-hooks"))]
    pub use rerail_lang::code_rule_dsl::fixtures;
    pub use rerail_lang::code_rule_dsl::json;
    pub use rerail_lang::code_rule_dsl::language;
    pub use rerail_lang::code_rule_dsl::lines;
    pub mod logic;
    pub use rerail_lang::code_rule_dsl::module_path;
    pub use rerail_prof::code_rule_dsl::phase_clock;
    pub use rerail_facts::code_rule_dsl::producers;
    pub use rerail_prof::code_rule_dsl::prof;
    pub use rerail_facts::code_rule_dsl::search_index;
    pub use rerail_lang::code_rule_dsl::semver;
    pub use rerail_facts::code_rule_dsl::sql_profile;
    pub use rerail_facts::code_rule_dsl::sql_admission;
    #[path = "stats"]
    pub mod stats {
        pub use rerail_facts::code_rule_dsl::stats::identity;
    }
    #[path = "suppress"]
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
    pub mod typing;
    #[path = "value"]
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
    #[path = "waves"]
    pub mod waves {
        pub mod owner;
        pub mod schedule;
        pub mod storage;
    }
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
#[path = "../../../src/threads.rs"]
pub mod threads;
// cratify: mounts end
