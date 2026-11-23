//! The engine's tests of rule compiling: the compiler's parse, format arguments, configuration and values (docs/final-shape/proposals/crate-split/incremental-plan.md, step 7i).
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

#[path = "code_rule_dsl"]
pub mod code_rule_dsl {
    pub use rerail_boot::code_rule_dsl::*;
    pub use rerail_boot::code_rule_dsl::rule_api::CodeRuleDslEditor;
    #[cfg_attr(test, path = "compile")]
    mod compile {
        pub use rerail_boot::code_rule_dsl::compile::*;
        pub mod value {
            pub use rerail_boot::code_rule_dsl::compile::value::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::{bounded_text, invalid, object, reject_unknown};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::{ ValueFeature, gate, };
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::{Family, GrammarDialect};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::{AstMatcher, Bound, Reading, ValueTest};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::{Semantic, SemanticText};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::model::Limits;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::scope::options::{LanguageOptions, PythonOptions};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::value::Value;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::value::text_regex::TextRegex;
            #[allow(unused_imports)]
            use napi::Result;
            #[allow(unused_imports)]
            use serde_json::Value as Json;
            #[allow(unused_imports)]
            use std::sync::Arc;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::pack as language_pack;
        }
        pub mod arguments {
            pub use rerail_boot::code_rule_dsl::compile::arguments::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use super::resolve::is_identifier;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::{invalid, object, reject_unknown};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::{ ArgumentFeature, gate, };
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::{Family, GrammarDialect, kind_ids};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::Semantic;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::{Arguments, AstMatcher, SlotKind};
            #[allow(unused_imports)]
            use napi::Result;
            #[allow(unused_imports)]
            use serde_json::Value;
            #[allow(unused_imports)]
            use std::collections::BTreeSet;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::pack as language_pack;
        }
        pub mod utils {
            pub use rerail_boot::code_rule_dsl::compile::utils::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use super::documents::{DocumentScopes, Side};
            #[allow(unused_imports)]
            use super::order::AllOrderConflict;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::{invalid, object, reject_unknown};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::{ CallArgs, CaptureAnalysis, MAX_UTIL_INSTANCES, Transform, UtilTable, };
            #[allow(unused_imports)]
            use crate::code_rule_dsl::scope::options::{LanguageOptions, PythonOptions};
            #[allow(unused_imports)]
            use napi::Result;
            #[allow(unused_imports)]
            use serde_json::Value;
            #[allow(unused_imports)]
            use std::cell::RefCell;
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, BTreeSet};
            #[allow(unused_imports)]
            use std::sync::{Arc, Weak};
        }
        pub mod shared {
            pub use rerail_boot::code_rule_dsl::compile::shared::*;
            #[cfg(all(test, not(windows)))]
            pub mod tests;
        }
        pub mod layout {
            pub use rerail_boot::code_rule_dsl::compile::layout::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use super::parse::capture_name;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::{invalid, object, reject_unknown};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::Semantic;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::{AstMatcher, Gap, Layout, LineContext, LineCount, Neighbour};
            #[allow(unused_imports)]
            use napi::Result;
            #[allow(unused_imports)]
            use serde_json::Value;
        }
        pub mod evidence {
            pub use rerail_boot::code_rule_dsl::compile::evidence::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use super::define::Defines;
            #[allow(unused_imports)]
            use super::{compile_glob_set, compile_query, string_array};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::{invalid, non_empty_str, object, reject_unknown};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::model::{ AstRelation, Capabilities, CodeRuleScope, FactQuery, NameClosure, Projection, ProjectionSource, Query, };
            #[allow(unused_imports)]
            use napi::Result;
            #[allow(unused_imports)]
            use serde_json::Value;
            #[allow(unused_imports)]
            use std::collections::BTreeSet;
            #[allow(unused_imports)]
            use std::sync::Arc;
        }
        pub mod dialect {
            pub use rerail_boot::code_rule_dsl::compile::dialect::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::invalid;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::GrammarDialect;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::{self, CodeRuleLanguage};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::AstMatcher;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::model::Capabilities;
            #[allow(unused_imports)]
            use napi::Result;
            #[allow(unused_imports)]
            use serde_json::Value;
        }
        pub mod define {
            pub use rerail_boot::code_rule_dsl::compile::define::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use super::ast_rule::{compile_ast_rule, validate_ast_rule};
            #[allow(unused_imports)]
            use super::config::Options;
            #[allow(unused_imports)]
            use super::dialect::language_capability;
            #[allow(unused_imports)]
            use super::documents::{DocumentScopes, Side};
            #[allow(unused_imports)]
            use super::refuse_unavailable_semantics;
            #[allow(unused_imports)]
            use super::shared::SharedTables;
            #[allow(unused_imports)]
            use super::utils::{UtilDef, UtilIds, UtilScope, reject_util_cycles, util_map};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::{invalid, object, reject_unknown, required_str};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::{CodeRuleLanguage, GrammarDialect};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::{AstMatcher, UtilTable};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::model::Capabilities;
            #[allow(unused_imports)]
            use napi::Result;
            #[allow(unused_imports)]
            use serde_json::Value;
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, BTreeSet};
            #[allow(unused_imports)]
            use std::sync::Arc;
        }
        pub mod config {
            pub use rerail_boot::code_rule_dsl::compile::config::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::{invalid, object, reject_unknown};
            #[allow(unused_imports)]
            use napi::Result;
            #[allow(unused_imports)]
            use serde_json::{Map, Number, Value};
            #[allow(unused_imports)]
            use std::collections::BTreeMap;
        }
        pub mod compare {
            pub use rerail_boot::code_rule_dsl::compile::compare::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use super::ast_rule::AST_RELATIONS;
            #[allow(unused_imports)]
            use super::order::AllOrderConflict;
            #[allow(unused_imports)]
            use super::resolve::{BINDING_KEYS, OCCURRENCE_KEY};
            #[allow(unused_imports)]
            use super::utils::UtilDef;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::{invalid, non_empty_str, object, reject_unknown};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::{ Collation, CompareOp, CompareText, Comparison, CountBinding, UtilitySummary, };
            #[allow(unused_imports)]
            #[cfg(any(test, feature = "test-hooks"))]
            use crate::code_rule_dsl::walks::{Walk, record_walk};
            #[allow(unused_imports)]
            use napi::Result;
            #[allow(unused_imports)]
            use serde_json::Value;
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, BTreeSet};
        }
        pub mod ast_rule {
            pub use rerail_boot::code_rule_dsl::compile::ast_rule::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use super::compare::{count_bounds, parse_comparison};
            #[allow(unused_imports)]
            use super::dialect::grammar_gap;
            #[allow(unused_imports)]
            use super::documents::Side;
            #[allow(unused_imports)]
            use super::nth_child::{compile_nth_child, validate_nth_child};
            #[allow(unused_imports)]
            use super::order::{all_order_conflicts, order_by_captures};
            #[allow(unused_imports)]
            use super::utils::{UtilIds, UtilScope, is_metavariable_name};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::{ bounded_text, invalid, non_empty_str, object, reject_unknown, required_str, };
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::{GrammarDialect, PatternSource, rule_pattern};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::{ AstMatcher, CallArgs, CompareText, Count, EmbeddedTest, FieldRelation, HostTest, Reading, Related, Semantic, StopBy, UtilCall, UtilRef, parse_exported, };
            #[allow(unused_imports)]
            use crate::code_rule_dsl::model::Limits;
            #[allow(unused_imports)]
            use ast_grep_core::Language;
            #[allow(unused_imports)]
            use ast_grep_core::matcher::{KindMatcher, Pattern, PatternNode};
            #[allow(unused_imports)]
            use napi::Result;
            #[allow(unused_imports)]
            use regex::Regex;
            #[allow(unused_imports)]
            use serde_json::Value;
            #[allow(unused_imports)]
            use std::collections::BTreeSet;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::pack as language_pack;
        }
        #[cfg_attr(test, path = "format_args")]
        pub mod format_args {
            pub use rerail_boot::code_rule_dsl::compile::format_args::*;
            #[cfg(test)]
            pub mod php;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::compile::ast_rule::{compile_ast_rule};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::compile::utils::{UtilScope};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::{invalid, non_empty_str, object, reject_unknown};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::{FormatArgFeature};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::{Family, GrammarDialect};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::Semantic;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::{
                AstMatcher, Capture, FormatArgs, Reading, Style, parse_exported,
            };
            #[allow(unused_imports)]
            use crate::code_rule_dsl::model::Limits;
            #[allow(unused_imports)]
            use napi::Result;
            #[allow(unused_imports)]
            use serde_json::Value;
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
            use super::parse::{capture_name, compile_sub_rule, validate_sub_rule};
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
                    pub mod annotation_tests;
            #[cfg(test)]
                    pub mod host_tests;
            #[cfg(test)]
                    pub mod m12b_tests;
            #[cfg(test)]
                    pub mod tests;
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
        #[cfg(test)]
            pub mod tests;
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
            pub mod examples_tests;
        #[cfg(test)]
            pub mod params_tests;
    }
    #[cfg(test)]
    #[path = "../../../../src/code_rule_dsl/test_support.rs"]
    mod test_support;
}
