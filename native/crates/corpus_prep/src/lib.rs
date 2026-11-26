//! The corpus runner's preparation (`plan::corpus::{preparation, product_demand}`): what a program
//! becomes before it runs (simplified, its parts and lowering's tail, hidden, its demand by magic sets, merged,
//! pruned, ranked), and the deferred products' demand. It is over the memo
//! (`rerail-corpus-memo`), whose memo and guests it reads, and under the corpus's root, `rerail-logic`'s.
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
pub use rerail_corpus_memo::{content_store, git_access, git_workspace, threads};
extern crate self as rerail_corpus_prep;
#[path = "code_rule_dsl"]
pub mod code_rule_dsl {
    pub use rerail_corpus_memo::code_rule_dsl::*;
    #[path = "logic"]
    pub mod logic {
        pub use rerail_corpus_memo::code_rule_dsl::logic::*;
        pub mod plan {
            pub use rerail_corpus_memo::code_rule_dsl::logic::plan::*;
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
            /// The corpus's preparation, over the memo's layer; the root (`corpus.rs`) is `rerail-logic`'s.
            pub mod corpus {
                pub use rerail_corpus_memo::code_rule_dsl::logic::plan::corpus::*;
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
                use rerail_corpus_memo::code_rule_dsl::logic::plan::corpus::shape;
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
                pub mod preparation;
                pub mod product_demand;
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
        crate::code_rule_dsl::logic::run_state::CARRIERS.register("rerail-corpus-prep's tests", CARRIERS);
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
