//! The engine's tests of rule resolving: the compiler's resolution, binds and bindings, and its order (docs/final-shape/proposals/crate-split/incremental-plan.md, step 7i).
//!
//! A test crate above boot: it mounts the test files in place, under the facades the engine hosted them through,
//! and reads every other module through boot. Its test binary is booted as it loads (`boot::install_for_tests`), as
//! the engine's is. Written by tools/cratify/split_test_crates.py, and kept by hand since.
#![cfg(test)]
#![allow(dead_code, unused_imports, hidden_glob_reexports, ambiguous_glob_reexports, private_interfaces, private_bounds)]
#![allow(clippy::len_without_is_empty, clippy::result_unit_err, clippy::new_without_default)]
#![allow(clippy::type_complexity, clippy::too_many_arguments)]
pub use rerail_boot::{content_store, git_access, git_ignore, git_workspace, panic_boundary, source_tree, threads, tree_watch};
pub(crate) use rerail_boot::code_rule_dsl::prof::{lower_hex, purge_allocator_pages};

// The engine's test-binary rules: each test's time limit (`RERAIL_TEST_TIMEOUT`), and stores that flush nothing
// (`RERAIL_FLUSH=none`, decision 0481), both set before `main`.
#[path = "../../../src/test_watchdog.rs"]
mod test_watchdog;
#[path = "../../../src/test_flush.rs"]
mod test_flush;

/// The program, booted as the engine boots it (`boot::install`), with this binary's own build identities.
pub(crate) fn install_store_functions() {
    rerail_boot::code_rule_dsl::boot::install_for_tests();
}

mod test_load {
    extern "C" fn load() {
        super::install_store_functions();
    }
    #[used]
    #[cfg_attr(target_vendor = "apple", unsafe(link_section = "__DATA,__mod_init_func"))]
    #[cfg_attr(windows, unsafe(link_section = ".CRT$XCU"))]
    #[cfg_attr(not(any(target_vendor = "apple", windows)), unsafe(link_section = ".init_array"))]
    static LOAD: extern "C" fn() = load;
}

#[path = "../../../src/code_rule_dsl"]
pub mod code_rule_dsl {
    pub use rerail_boot::code_rule_dsl::*;
    pub use rerail_boot::code_rule_dsl::rule_api::CodeRuleDslEditor;
    #[cfg_attr(test, path = "compile")]
    mod compile {
        pub use rerail_boot::code_rule_dsl::compile::*;
        pub mod order {
            pub use rerail_boot::code_rule_dsl::compile::order::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::invalid;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::{AstMatcher, CaptureAnalysis};
            #[allow(unused_imports)]
            use napi::Result;
            #[allow(unused_imports)]
            use std::collections::BTreeSet;
        }
        #[cfg_attr(test, path = "binds")]
        pub mod binds {
            pub use rerail_boot::code_rule_dsl::compile::binds::*;
            #[cfg(test)]
            pub mod eslint;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::compile::ast_rule::{compile_ast_rule};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::compile::utils::{UtilScope};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::{invalid, object, reject_unknown};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::{ScopeFeature};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::kind_ids;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::{Family, GrammarDialect};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::Semantic;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::{AstMatcher, BindingCount, BindingFilter, Binds, Redefines};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::model::Limits;
            #[allow(unused_imports)]
            use napi::Result;
            #[allow(unused_imports)]
            use serde_json::{Map, Value};
            #[allow(unused_imports)]
            use std::collections::BTreeSet;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::pack as language_pack;
            #[cfg(test)]
                    pub mod ruff;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use super::ast_rule::{validate_ast_rule};
            #[allow(unused_imports)]
            use super::resolve::{BINDING_KEYS, OCCURRENCE_KEY};
            #[allow(unused_imports)]
            use super::utils::{UtilIds};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::{gate};
        }
        #[allow(unused_imports)] // boot's mount: only boot's files read it
        pub use rerail_lang_ecma::code_rule_dsl::compile::import_target;
        #[cfg_attr(test, path = "parse")]
        pub mod parse {
            pub use rerail_boot::code_rule_dsl::compile::parse::*;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::compile::ast_rule::{compile_ast_rule};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::compile::utils::{UtilScope};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::{bounded_text, invalid, non_empty_str, object};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::{SyntaxFeature};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::kind_ids;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::{Family, GrammarDialect};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::literal::Grammar;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::Semantic;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::{
                AstMatcher, CaptureAnalysis, CompareText, Count, HostTest, ParseTest, Reading, Related,
                SemanticText, StopBy, Transform, parse_exported, table_reading,
            };
            #[allow(unused_imports)]
            use crate::code_rule_dsl::model::Limits;
            #[allow(unused_imports)]
            use ast_grep_core::matcher::KindMatcher;
            #[allow(unused_imports)]
            use napi::Result;
            #[allow(unused_imports)]
            use regex::Regex;
            #[allow(unused_imports)]
            use serde_json::{Map, Value};
            #[allow(unused_imports)]
            use std::collections::BTreeSet;
            #[allow(unused_imports)]
            use std::sync::Arc;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::pack as language_pack;
            #[cfg(test)]
                    pub mod ruff;
            #[allow(unused_imports)]
            use super::ast_rule::{AST_RELATIONS, StopSpec, relation_rule, validate_ast_rule};
            #[allow(unused_imports)]
            use super::compare::{count_bounds, parse_comparison};
            #[allow(unused_imports)]
            use super::order::order_by_captures;
            #[allow(unused_imports)]
            use super::resolve::{compile_resolves, validate_resolves};
            #[allow(unused_imports)]
            use super::utils::{UtilIds};
            #[allow(unused_imports)]
            use super::value::{compile_value_with, validate_value};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::{gate};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::soundness::sound_under_errors;
        }
        #[cfg_attr(test, path = "resolve")]
        pub mod resolve {
            pub use rerail_boot::code_rule_dsl::compile::resolve::*;
            #[allow(unused_imports)] // boot's mount: only boot's files read it
            pub use rerail_lang_php::code_rule_dsl::compile::resolve::php;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::compile::utils::UtilScope;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::{invalid, object, reject_unknown};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::{Refusal, ScopeFeature};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::kind_ids;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::{Family, GrammarDialect, language_not_supported};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::Semantic;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::{AstMatcher, BindingTest, Resolves, Unresolved};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::scope::options::{LanguageOptions, PythonOptions};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::scope::Lookup;
            #[allow(unused_imports)]
            use napi::Result;
            #[allow(unused_imports)]
            use serde_json::{Map, Value};
            #[allow(unused_imports)]
            use std::collections::BTreeSet;
            #[allow(unused_imports)]
            use std::sync::Arc;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::pack as language_pack;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use super::binds::{FILTER_KEYS, compile_filter, validate_filter};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::{gate};
        }
        #[allow(unused_imports)]
        use crate::code_rule_dsl::json::{
            bounded_text, invalid, json_error, non_empty_str, object, reject_unknown, required_str,
        };
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::adapter::{Refusal};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::{CodeRuleLanguage, Semantic, Support};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::matcher::{AnchorPlan, AstMatcher, Capture, Constrained, GrammarMatcher, Reading, RequiredText, UtilOrigin, UtilTable, collect_variables, reads_import_targets};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::model::{
            Anchor, Capabilities, ClauseKind, CodeRuleScope, CompiledClause, Limits, Query, RegexLines,
            RegexOptions, RegexSource, RuleMode, ViolationSpec,
        };
        #[allow(unused_imports)]
        use crate::code_rule_dsl::violations;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::{GrammarDialect, PatternSource, rule_pattern};
        #[allow(unused_imports)]
        pub(super) use crate::code_rule_dsl::compile::ast_rule::compile_ast_rule;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::compile::documents::Side;
        #[allow(unused_imports)]
        use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
        #[allow(unused_imports)]
        use napi::Result;
        #[allow(unused_imports)]
        use regex::Regex;
        #[allow(unused_imports)]
        use serde::de::{self, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
        #[allow(unused_imports)]
        use serde_json::{Map, Value, json};
        #[allow(unused_imports)]
        use std::collections::{BTreeMap, BTreeSet};
        #[allow(unused_imports)]
        use std::sync::Arc;
        #[allow(unused_imports)]
        pub(super) use crate::code_rule_dsl::compile::utils::UtilScope;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::adapter::pack as language_pack;
        #[allow(unused_imports)]
        use super::language::adapter::{gate};
        #[allow(unused_imports)]
        use super::matcher::{reads_values};
        #[allow(unused_imports)]
        use super::soundness::sound_under_errors;
        #[allow(unused_imports)]
        use ast_rule::validate_ast_rule;
        #[allow(unused_imports)]
        use compare::{binding_site_index, bound_elsewhere, compare_sites, frame_scope};
        #[allow(unused_imports)]
        use define::{CLAUSE_KINDS, Defines, collect_defines, is_define};
        #[allow(unused_imports)]
        use dialect::{embedded_dialects, language_capability};
        #[allow(unused_imports)]
        use shared::SharedTables;
        #[allow(unused_imports)]
        use utils::{UtilDef, UtilIds, reachable, reject_util_cycles, rule_map, util_map};
        #[cfg(test)]
            pub mod binding_tests;
    }
    #[cfg(test)]
    mod test_support;
}

// The engine's allocator (`rerail_prof::heap::Sampled`, as `rerail-engine`'s `lib.rs` declares it): without it the
// test binary ran on the system allocator, which no build of the engine runs on.
#[cfg(not(any(windows, target_family = "wasm", feature = "system-allocator")))]
#[global_allocator]
static GLOBAL: rerail_prof::heap::Sampled = rerail_prof::heap::Sampled;
