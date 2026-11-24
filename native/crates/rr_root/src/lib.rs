//! The logic language's root (`logic::rr`: reading .rr texts and their libraries, lowering's tail, the language's
//! settings, `logic::settings`) and the corpus's base it reads (`plan::corpus`: `base`, the store and its selection,
//! phase 0's observer, the programs combined, the report; phase 0's import graph, the phases' stores, the
//! constants, reuse, the world, git's inputs, the links). Over the corpus's preparation (`rerail-corpus-prep`), under
//! the corpus runner (`corpus.rs`, `rerail-logic`'s), which phase 0 runs its bootstrap program through: the runner is
//! behind a trait the program's composition registers (`plan::corpus::RUNNER`).
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
pub use rerail_corpus_prep::{content_store, git_access, git_workspace, threads};
extern crate self as rerail_rr_root;
#[path = "code_rule_dsl"]
pub mod code_rule_dsl {
    pub use rerail_corpus_prep::code_rule_dsl::*;
    #[path = "logic"]
    pub mod logic {
        pub use rerail_corpus_prep::code_rule_dsl::logic::*;
        /// The language's root (`rr.rs`), with its children.
        pub mod rr;
        pub mod settings;
        /// Contexts: code inside code analysed as a pass of its own rules (decision 0597), which the root's reading
        /// (`rr::parse_version`) runs.
        pub mod contexts;
        pub use rerail_json_rules::code_rule_dsl::logic::json_rules;
        pub mod plan {
            pub use rerail_corpus_prep::code_rule_dsl::logic::plan::*;
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
            /// The corpus's base and the modules the language's root reads with it; the root (`corpus.rs`, the
            /// runner) is `rerail-logic`'s, and re-exports this.
            pub mod corpus {
                pub use rerail_corpus_prep::code_rule_dsl::logic::plan::corpus::*;
                pub mod base;
                pub use base::*;
                pub mod analysis_history;
                pub mod combined;
                pub mod constants;
                pub mod frame;
                pub mod git;
                pub mod git_content;
                pub mod import_closure;
                pub mod inputs;
                pub mod kept_link;
                pub mod leaves;
                pub mod kept_texts;
                pub mod link;
                pub mod phase0;
                pub mod phases;
                pub mod reuse;
                pub mod source_languages;
                pub mod switches;
                #[cfg(any(test, feature = "test-hooks"))]
                pub mod test_store;
                pub mod world;
                // `corpus.rs`'s imports and its re-exports of the lower crates' items, for the children's `super::`.
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
                pub use rerail_corpus_memo::code_rule_dsl::logic::plan::corpus::own;
                #[allow(unused_imports)]
                pub use rerail_corpus_memo::code_rule_dsl::logic::plan::corpus::guests;
                #[allow(unused_imports)]
                pub use rerail_corpus_memo::code_rule_dsl::logic::plan::corpus::hiding;
                #[allow(unused_imports)]
                pub use rerail_corpus_memo::code_rule_dsl::logic::plan::corpus::exports;
                #[allow(unused_imports)]
                pub use rerail_corpus_memo::code_rule_dsl::logic::plan::corpus::scoping;
                #[allow(unused_imports)]
                pub use hiding::qualified;
                #[allow(unused_imports)]
                pub use rerail_corpus_memo::code_rule_dsl::logic::plan::corpus::sizing;
                #[allow(unused_imports)]
                pub use rerail_corpus_memo::code_rule_dsl::logic::plan::corpus::maintain;
                #[allow(unused_imports)]
                pub use rerail_corpus_memo::code_rule_dsl::logic::plan::corpus::keptstats;
                #[allow(unused_imports)]
                pub use rerail_corpus_memo::code_rule_dsl::logic::plan::corpus::texts;
                #[allow(unused_imports)]
                use rerail_corpus_memo::code_rule_dsl::logic::plan::corpus::shape;
                #[allow(unused_imports)]
                pub use rerail_corpus_memo::code_rule_dsl::logic::plan::corpus::matchers;
                #[allow(unused_imports)]
                pub use matchers::columns as ast_columns;
                #[allow(unused_imports)]
                pub use rerail_corpus_memo::code_rule_dsl::logic::plan::corpus::typing;
                #[allow(unused_imports)]
                pub use typing::{Typing, TypingReport};
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
                pub use crate::code_rule_dsl::prof::thread_usage;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::logic::program_image::passes::Parts as _;
                #[allow(unused_imports)]
                use std::collections::{BTreeMap, BTreeSet};
                #[allow(unused_imports)]
                pub use guests::Late;
                #[allow(unused_imports)]
                pub use own::{OWN_DOCS, OWN_EDGES, OWN_FIELDS, OWN_KINDS, OWN_POSTINGS, Own, STAT_SAMPLE_ROWS, constants_tested_in};
            }
        }
    }
}

/// This crate's own test binary, which no program composes: what a job carries of its run is the run's, the
/// planner's and setfix's, with the test's store, as the planner's tests register; no corpus runner is registered.
#[cfg(test)]
mod test_load {
    const CARRIERS: &[fn(&mut crate::code_rule_dsl::logic::run_state::Capture)] = &[
        crate::code_rule_dsl::logic::run::carried,
        crate::code_rule_dsl::logic::plan::carried,
        crate::code_rule_dsl::logic::plan::keeps::carried,
        crate::code_rule_dsl::logic::native::setfix::carried,
        crate::code_rule_dsl::logic::plan::corpus::test_store::carried,
    ];
    extern "C" fn load() {
        crate::code_rule_dsl::logic::run_state::CARRIERS.register("rerail-rr-root's tests", CARRIERS);
    }
    #[used]
    #[cfg_attr(target_vendor = "apple", unsafe(link_section = "__DATA,__mod_init_func"))]
    #[cfg_attr(windows, unsafe(link_section = ".CRT$XCU"))]
    #[cfg_attr(not(any(target_vendor = "apple", windows)), unsafe(link_section = ".init_array"))]
    static LOAD: extern "C" fn() = load;
}

// The crate's own tests make their folders the user's alone, as the engine's test binaries do (decision 0574): a
// host's umask of `022` made a fixture's folder `755`, which the mapped files' rules refuse.
#[cfg(test)]
#[path = "../../../src/test_private_folders.rs"]
mod test_private_folders;
