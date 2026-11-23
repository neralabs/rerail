//! The corpus runner's memo (`plan::corpus::{memo, guests, hiding, exports, keptstats, …}`): the entries a run keeps
//! of its chunks and parts and reads back, the guests' rows, the relations' hiding, the exports' counts, a chunk's own
//! statistics, and the small leaves under them. It is over typing and the planner and under the preparation
//! (`plan::corpus::preparation`), which reads the memo and the guests; the corpus's root is `rerail-logic`'s.
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
pub use rerail_typing::{content_store, git_access, git_workspace, threads};
extern crate self as rerail_corpus_memo;
#[path = "code_rule_dsl"]
pub mod code_rule_dsl {
    pub use rerail_typing::code_rule_dsl::*;
    #[path = "logic"]
    pub mod logic {
        pub use rerail_typing::code_rule_dsl::logic::*;
        pub mod plan {
            pub use rerail_typing::code_rule_dsl::logic::plan::*;
            // `plan.rs`'s imports, for the corpus root' `use super::*`.
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
            /// The corpus's memo layer; the root (`corpus.rs`) and the rest are `rerail-logic`'s.
            pub mod corpus {
                // `plan/corpus.rs`'s imports, for its children' `use super::*`.
                #[allow(unused_imports)]
                use rerail_store::Canonical as _;
                #[allow(unused_imports)]
                use super::{Error, Outcome, Result, lower, output};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::facts::core::{self, Built};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::facts::vtab::{Modules, syntax};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::language::GrammarDialect;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::logic::front;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::logic::program_image::fields::{Edit, ProgramFields};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::logic::rr::WORLD;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::typing::file_local::Binding;
                #[allow(unused_imports)]
                use crate::content_store::ContentStore;
                #[allow(unused_imports)]
                use rusqlite::Connection;
                #[allow(unused_imports)]
                use rusqlite::types::Value as SqlValue;
                #[allow(unused_imports)]
                use std::collections::HashMap;
                #[allow(unused_imports)]
                use std::path::{Path, PathBuf};
                #[allow(unused_imports)]
                use std::sync::Arc;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::logic::program_image::passes::Parts as _;
                #[allow(unused_imports)]
                use std::collections::{BTreeMap, BTreeSet};
                // The root's re-exports of this layer's items, which the layer reads under the root's names.
                pub use guests::Late;
                pub use hiding::qualified;
                pub use matchers::columns as ast_columns;
                pub use own::{OWN_DOCS, OWN_EDGES, OWN_FIELDS, OWN_KINDS, OWN_POSTINGS, Own, STAT_SAMPLE_ROWS, constants_tested_in};
                pub use typing::{Typing, TypingReport};
                pub mod added;
                pub mod exports;
                pub mod guests;
                pub mod hiding;
                pub mod keptstats;
                pub mod maintain;
                pub mod matchers;
                pub mod memo;
                pub mod own;
                pub mod owner;
                pub mod scoping;
                pub mod shape;
                pub mod sizing;
                pub mod texts;
                pub mod typing;
            }
        }
    }
}

/// This crate's own test binary, which no program composes: what a job carries of its run is the run's, the
/// planner's and setfix's, as the planner's tests register.
#[cfg(test)]
mod test_load {
    const CARRIERS: &[fn(&mut crate::code_rule_dsl::logic::run_state::Capture)] = &[
        crate::code_rule_dsl::logic::run::carried,
        crate::code_rule_dsl::logic::plan::carried,
        crate::code_rule_dsl::logic::plan::keeps::carried,
        crate::code_rule_dsl::logic::native::setfix::carried,
    ];
    extern "C" fn load() {
        crate::code_rule_dsl::logic::run_state::CARRIERS.register("rerail-corpus-memo's tests", CARRIERS);
    }
    #[used]
    #[cfg_attr(target_vendor = "apple", unsafe(link_section = "__DATA,__mod_init_func"))]
    #[cfg_attr(windows, unsafe(link_section = ".CRT$XCU"))]
    #[cfg_attr(not(any(target_vendor = "apple", windows)), unsafe(link_section = ".init_array"))]
    static LOAD: extern "C" fn() = load;
}
