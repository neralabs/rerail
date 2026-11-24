//! The adapter frameworks the language packs fill (scopes, values, literals, calls, suppressions), the language
//! adapter traits they implement (`language::adapter`) and what those traits name: a request's file
//! (`file_input`), the matcher's file context and argument slots (`matcher::{resolve, arguments}`) with the clause
//! steps they spend (`matcher::steps`), and the documents' options (`compile::{config, options}`). Below the matcher,
//! the rule compiler and the workspace (decision 0353's blocks are read in place throughout).
//!
//! Its sources are in its own `src/`, at their module paths in the engine's tree (the crate split, step 9).
// A mounted module re-exports its dependency's by glob, and its sources import the same items
// under the same names: the glob's name and the private one are one item. The mounts also widen what the crates
// above read (the split's facades, not a design).
#![allow(dead_code, unused_imports, hidden_glob_reexports, private_interfaces, private_bounds)]
// What the split widened for the crates above is no public API (`len`, `()` errors, `new`); the long
// signatures and tuple types are the sources' own.
#![allow(clippy::len_without_is_empty, clippy::result_unit_err, clippy::new_without_default)]
#![allow(clippy::type_complexity, clippy::too_many_arguments)]
pub use rerail_observe::content_store;
pub use rerail_observe::git_access;
pub use rerail_observe::git_workspace;
extern crate self as rerail_adapters;
#[path = "code_rule_dsl"]
pub mod code_rule_dsl {
    pub use rerail_lang::code_rule_dsl::budget;
    pub use rerail_lang::code_rule_dsl::build_ids;
    pub mod calls;
    #[path = "compile"]
    pub mod compile {
        pub mod config;
        pub mod options;
    }
    pub use rerail_lang::code_rule_dsl::embed;
    pub use rerail_facts_base::code_rule_dsl::facts;
    pub mod file_input;
    #[cfg(any(test, feature = "test-hooks"))]
    pub use rerail_lang::code_rule_dsl::fixtures;
    pub mod identifier;
    pub use rerail_lang::code_rule_dsl::json;
    pub use rerail_lang::code_rule_dsl::regexes;
    #[path = "language"]
    pub mod language {
        pub use rerail_lang::code_rule_dsl::language::*;
        pub mod adapter;
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use crate::code_rule_dsl::tree::Tree;
        #[allow(unused_imports)]
        use rerail_rr_syntax::tape as rr_tree;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::module_path::ModulePath;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::semver::Range;
        #[allow(unused_imports)]
        use ast_grep_core::{
            AstGrep,
            language::Language,
            matcher::{Pattern, PatternBuilder, PatternError},
            tree_sitter::{StrDoc, TSLanguage},
        };
        #[allow(unused_imports)]
        use ast_grep_language::{LanguageExt, SupportLang};
        #[allow(unused_imports)]
        use std::borrow::Cow;
        #[allow(unused_imports)]
        use std::sync::LazyLock;
    }
    pub use rerail_lang::code_rule_dsl::lines;
    pub mod literal;
    pub mod logic {
        pub mod front {
            pub use rerail_rr_syntax::code_rule_dsl::logic::front::ast;
            pub use rerail_rr_syntax::code_rule_dsl::logic::front::name;
        }
        pub use rerail_prof::code_rule_dsl::logic::profile;
        pub use rerail_facts_base::code_rule_dsl::logic::strings;
    }
    /// What of the matcher the adapters name: the file context names resolve in, the argument slots, a clause's
    /// steps. The matcher (rerail-matcher) re-exports them.
    #[path = "matcher"]
    pub mod matcher {
        pub mod arguments;
        pub mod resolve;
        pub mod steps;
        pub use arguments::{Arguments, Passed, SlotKind};
        pub use resolve::Resolves;
        pub use resolve::file_path;
        pub use resolve::{
            FileGates, calls_tables, file_index, file_lines, file_module, file_rerail_ignores,
            file_source_type_for, table_reading,
        };
        pub use resolve::{FileScopeSlot, Unresolved, matched_file_path};
        #[cfg(any(test, feature = "test-hooks"))]
        pub use resolve::{JSX_RUNTIME_DERIVATIONS, SOURCE_TYPE_DERIVATIONS, TABLE_BUILDS};
        pub use steps::{clause_state_reads, fill_steps, filling, read_clause_state};
    }
    pub use rerail_lang::code_rule_dsl::module_path;
    pub mod origin;
    pub use rerail_prof::code_rule_dsl::phase_clock;
    pub use rerail_facts_base::code_rule_dsl::producers;
    pub use rerail_prof::code_rule_dsl::prof;
    pub mod scope;
    pub use rerail_facts_base::code_rule_dsl::search_index;
    pub use rerail_lang::code_rule_dsl::semver;
    pub mod suppress;
    pub use rerail_lang::code_rule_dsl::tape;
    pub use rerail_facts_base::code_rule_dsl::text;
    pub use rerail_lang::code_rule_dsl::tree;
    pub mod value;
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
