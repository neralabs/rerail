#[path = "blade"]
mod blade {
    pub use rerail_lang_php::code_rule_dsl::blade::*;
    #[cfg(test)]
    pub mod tests;
}
pub use rerail_lang::code_rule_dsl::budget;
// The engine's build identities, which it sets as it loads (`crate::install_build_ids`).
pub use rerail_lang::code_rule_dsl::build_ids;
#[path = "calls"]
mod calls {
    pub use rerail_w::code_rule_dsl::calls::*;
    #[path = "closure"]
    pub mod closure {
        pub use rerail_w::code_rule_dsl::calls::closure::*;
        pub use rerail_lang_php::code_rule_dsl::calls::closure::lang_php;
        pub use rerail_lang_python::code_rule_dsl::calls::closure::lang_python;
        pub use rerail_lang_rust::code_rule_dsl::calls::closure::lang_rust;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::calls::names::Name;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::calls::php;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::calls::summary::{BASE, CLASS, DECORATED, METACLASS, OPEN, Summary, UNKNOWN_BASE};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::Family;
        #[allow(unused_imports)]
        use rayon::prelude::*;
        #[allow(unused_imports)]
        use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
        #[allow(unused_imports)]
        use std::sync::Arc;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::adapter::pack as language_pack;
    }
    pub use rerail_lang_php::code_rule_dsl::calls::php;
    pub use rerail_lang_python::code_rule_dsl::calls::python;
    pub use rerail_lang_rust::code_rule_dsl::calls::rust;
    #[path = "summary"]
    pub mod summary {
        pub use rerail_w::code_rule_dsl::calls::summary::*;
        pub use rerail_lang_python::code_rule_dsl::calls::summary::python;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::tree::Tree;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::calls::names::Name;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::Family;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::module_path::ModulePath;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::scope::{Binding, Form, Import, ScopeKind, ScopeTable};
        #[allow(unused_imports)]
        use ast_grep_core::{Doc, Node};
        #[allow(unused_imports)]
        use foldhash::{HashMap, HashMapExt};
    }
    #[allow(unused_imports)]
    use crate::code_rule_dsl::json::{invalid, object};
    #[allow(unused_imports)]
    use crate::code_rule_dsl::language::is_stub_file;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::language::{Family, GrammarDialect};
    #[allow(unused_imports)]
    use crate::code_rule_dsl::lines::Span;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::module_path::ModulePath;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::scope::{Reading, ScopeTable};
    #[allow(unused_imports)]
    use crate::content_store::HashKey;
    #[allow(unused_imports)]
    use ast_grep_core::{Doc, Node};
    #[allow(unused_imports)]
    use crate::code_rule_dsl::calls::closure::{DefRef, Graph, Hop};
    #[allow(unused_imports)]
    use crate::code_rule_dsl::calls::names::Name;
    #[allow(unused_imports)]
    use napi::Result;
    #[allow(unused_imports)]
    use serde_json::{Map, Value};
    #[allow(unused_imports)]
    use sha2::{Digest, Sha256};
    #[allow(unused_imports)]
    use std::cell::RefCell;
    #[allow(unused_imports)]
    use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
    #[allow(unused_imports)]
    use std::sync::{Arc, OnceLock};
    #[allow(unused_imports)]
    use crate::code_rule_dsl::calls::summary::{ASYNC, CLASS, Callee, Summary};
    #[allow(unused_imports)]
    use crate::code_rule_dsl::language::adapter::pack as language_pack;
}
#[path = "compile"]
mod compile {
    pub use rerail_w::code_rule_dsl::compile::*;
    #[path = "binds"]
    pub mod binds {
        pub use rerail_w::code_rule_dsl::compile::binds::*;
        // ESLint's binds lab: it reads the engine's oracles and test kit.
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
    }
    #[path = "format_args"]
    pub mod format_args {
        pub use rerail_w::code_rule_dsl::compile::format_args::*;
        // PHP's format-args lab: it reads the engine's oracles and test kit.
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
    }
    pub use rerail_lang_ecma::code_rule_dsl::compile::import_target;
    #[path = "parse"]
    pub mod parse {
        pub use rerail_w::code_rule_dsl::compile::parse::*;
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
    }
    #[path = "resolve"]
    pub mod resolve {
        pub use rerail_w::code_rule_dsl::compile::resolve::*;
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
}
pub use rerail_w::code_rule_dsl::crypto;
pub use rerail_lang_python::code_rule_dsl::cpython;
#[path = "demand"]
mod demand {
    pub use rerail_transducer::code_rule_dsl::demand::*;
    #[path = "driver"]
    pub mod driver {
        pub use rerail_transducer::code_rule_dsl::demand::driver::*;
        // The catalog is test-only and reads `relational`, the engine's: mounted here whole.
        #[cfg(any(test, feature = "test-hooks"))]
        #[path = "catalog.rs"]
        pub mod catalog;
        #[cfg(test)]
        pub mod tests;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::demand::vtab::{MatchRegistry, PackedDemand, register_module};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::transducer::packed::ExecutionBackend;
        #[allow(unused_imports)]
        use crate::content_store::{ContentStore, StoreResult};
        #[allow(unused_imports)]
        use rusqlite::{Connection, OptionalExtension as _, Row};
        #[allow(unused_imports)]
        use serde_json::{Value, json};
        #[allow(unused_imports)]
        use std::collections::BTreeMap;
        #[allow(unused_imports)]
        use std::sync::{Arc, LazyLock};
    }
}
// Metal as the engine's device (`rerail-device-metal`): registered as the library loads (`install_devices`).
#[cfg(target_os="macos")]
pub use rerail_device_metal::code_rule_dsl::device_metal;
#[path = "embed"]
mod embed {
    pub use rerail_lang::code_rule_dsl::embed::*;
    pub use rerail_lang_php::code_rule_dsl::embed::compiled_view;
    pub use rerail_lang_ecma::code_rule_dsl::embed::script_block;
    #[allow(unused_imports)]
    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) use crate::code_rule_dsl::embed::test_script::TestScript;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::language::GrammarDialect;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::embed::segments::SegmentMap;
    #[allow(unused_imports)]
    use std::ops::Range;
    #[allow(unused_imports)]
    use tree_sitter::Node as TsNode;
}
#[path = "eval"]
mod eval {
    pub use rerail_w::code_rule_dsl::eval::*;
    pub use rerail_lang_ecma::code_rule_dsl::eval::ecma;
    #[cfg(test)]
    pub mod tests;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::calls;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::json::{invalid, json_error, required_str};
    #[allow(unused_imports)]
    use crate::code_rule_dsl::language::adapter::package_manifests;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::model::{ClauseKind, CompiledClause, CompiledCodeRule, FactQuery, FileInput, Metrics, Snapshot, package_marker_directory, python_module_in};
    #[allow(unused_imports)]
    use crate::code_rule_dsl::value::names;
    #[allow(unused_imports)]
    use napi::Result;
    #[allow(unused_imports)]
    pub(super) use crate::code_rule_dsl::eval::output::{
        coverage, evaluation_json, failure, overflow_evaluation, result_order, violation_from_witness,
        violation_id, violation_ids,
    };
    #[allow(unused_imports)]
    pub(super) use crate::code_rule_dsl::eval::query::Output;
    #[allow(unused_imports)]
    pub(super) use crate::code_rule_dsl::eval::query::{
        TapeCacheSlot, WitnessError, document_matcher, evaluate_query, file_language, file_tape,
        grammar_matcher, witness_from_match, witnesses_from_match,
    };
    #[allow(unused_imports)]
    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) use crate::code_rule_dsl::eval::query::{reset_shared_group_lookups, shared_group_lookups};
    #[allow(unused_imports)]
    use serde_json::Value;
    #[allow(unused_imports)]
    use std::collections::{BTreeMap, HashSet};
    #[allow(unused_imports)]
    use std::sync::Arc;
}
#[path = "facts"]
mod facts {
    pub use rerail_facts::code_rule_dsl::facts::*;
    pub use rerail_w::code_rule_dsl::facts::facts_a;
    #[path = "producers"]
    pub mod producers {
        pub use rerail_facts::code_rule_dsl::facts::producers::*;
        pub use rerail_lang_python::code_rule_dsl::facts::producers::python;
    }
    #[path = "vtab"]
    pub mod vtab {
        pub use rerail_facts::code_rule_dsl::facts::vtab::*;
        pub use rerail_lang_python::code_rule_dsl::facts::vtab::python;
        #[path = "syntax"]
        pub mod syntax {
            pub use rerail_facts::code_rule_dsl::facts::vtab::syntax::*;
            pub use rerail_lang_python::code_rule_dsl::facts::vtab::syntax::annotation;
            pub mod ast_wave;
            pub use rerail_lang_python::code_rule_dsl::facts::vtab::syntax::candidates;
            pub use rerail_lang_python::code_rule_dsl::facts::vtab::syntax::cpython;
            pub use rerail_lang_ecma::code_rule_dsl::facts::vtab::syntax::ecma_core;
            pub use rerail_lang_python::code_rule_dsl::facts::vtab::syntax::fstring;
            pub use rerail_lang_python::code_rule_dsl::facts::vtab::syntax::py_families;
            pub use rerail_lang_python::code_rule_dsl::facts::vtab::syntax::python_core;
            pub use rerail_lang_python::code_rule_dsl::facts::vtab::syntax::python_demand;
            pub use rerail_lang_python::code_rule_dsl::facts::vtab::syntax::python_parts;
            pub use rerail_lang_python::code_rule_dsl::facts::vtab::syntax::python_syntax;
            pub use rerail_logic::code_rule_dsl::facts::vtab::syntax::rr_core;
            #[cfg(test)]
            pub mod tests;
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
// Wired to the findings layer in a later slice; until then only tests call it.
#[path = "findings"]
mod findings {
    pub use rerail_w::code_rule_dsl::findings::*;
    #[cfg(any(test, feature = "test-hooks"))]
    pub use rerail_lang_python::code_rule_dsl::findings::async_reference;
}
// The relational pools' hook slots, compiled wherever the pools are (the hub's relational stage is in the library).
pub use rerail_w::code_rule_dsl::hooks;
pub use rerail_w::code_rule_dsl::identifier;
pub use rerail_lang_ecma::code_rule_dsl::import_target;
pub use rerail_lang::code_rule_dsl::json;
#[path = "language"]
mod language {
    pub use rerail_lang::code_rule_dsl::language::*;
    #[path = "adapter"]
    pub mod adapter {
        pub use rerail_w::code_rule_dsl::language::adapter::*;
        #[path = "arguments"]
        pub mod arguments {
            pub use rerail_w::code_rule_dsl::language::adapter::arguments::*;
            pub use rerail_lang_ecma::code_rule_dsl::language::adapter::arguments::lang_ecma;
            pub use rerail_lang_php::code_rule_dsl::language::adapter::arguments::lang_php;
            pub use rerail_lang_python::code_rule_dsl::language::adapter::arguments::lang_python;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::{Refusal, Refused};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::{Arguments, Passed};
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use std::borrow::Cow;
        }
        pub use rerail_lang_php::code_rule_dsl::language::adapter::blade;
        #[path = "calls"]
        pub mod calls {
            pub use rerail_w::code_rule_dsl::language::adapter::calls::*;
            pub use rerail_lang_php::code_rule_dsl::language::adapter::calls::lang_blade;
            pub use rerail_lang_php::code_rule_dsl::language::adapter::calls::lang_php;
            pub use rerail_lang_python::code_rule_dsl::language::adapter::calls::lang_python;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::{Refusal, Refused};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::calls::closure::{Graph, Link};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::calls::names::Name;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::calls::summary::{Callee, SHADOWED, Summary};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::calls::{FileTables, php};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::{Family, GrammarDialect};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::module_path::ModulePath;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::scope::ScopeTable;
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use ast_grep_language::LanguageExt;
            #[allow(unused_imports)]
            use std::borrow::Cow;
            #[allow(unused_imports)]
            use std::sync::Arc;
        }
        pub use rerail_lang_ecma::code_rule_dsl::language::adapter::ecma;
        #[path = "format_args"]
        pub mod format_args {
            pub use rerail_w::code_rule_dsl::language::adapter::format_args::*;
            pub use rerail_lang_php::code_rule_dsl::language::adapter::format_args::lang_php;
            pub use rerail_lang_python::code_rule_dsl::language::adapter::format_args::lang_python;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::{Refusal, Refused};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::Style;
            #[allow(unused_imports)]
            use std::borrow::Cow;
        }
        pub use rerail_lang_json::code_rule_dsl::language::adapter::json;
        #[cfg(test)]
        pub mod kit;
        pub use rerail_lang_php::code_rule_dsl::language::adapter::php;
        pub use rerail_lang_python::code_rule_dsl::language::adapter::python;
        pub use rerail_lang_rust::code_rule_dsl::language::adapter::rust;
        #[path = "scope"]
        pub mod scope {
            pub use rerail_w::code_rule_dsl::language::adapter::scope::*;
            pub use rerail_lang_ecma::code_rule_dsl::language::adapter::scope::lang_ecma;
            pub use rerail_lang_php::code_rule_dsl::language::adapter::scope::lang_php;
            pub use rerail_lang_python::code_rule_dsl::language::adapter::scope::lang_python;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::{Adapter, Refusal};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::{ScopeName, Unresolved, file_module, file_source_type_for};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::module_path::ModulePath;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::scope::options::{
                DEFAULT_PYTHON_MINOR, EsGlobals, LanguageOptions, PythonOptions,
            };
            #[allow(unused_imports)]
            use crate::code_rule_dsl::scope::flow::Defined;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::scope::usage::{Steps, Usage};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::scope::{
                Access, Binding, BuildError, Builder, Directives, Form, Lookup, Reading, Reference, ScopeKind,
                ScopeTable, ecma, php, python,
            };
            #[allow(unused_imports)]
            use crate::code_rule_dsl::value::NodeRef;
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use std::borrow::Cow;
            #[allow(unused_imports)]
            use std::sync::Arc;
        }
        pub use rerail_lang_sql::code_rule_dsl::language::adapter::sql;
        #[path = "syntax"]
        pub mod syntax {
            pub use rerail_w::code_rule_dsl::language::adapter::syntax::*;
            pub use rerail_lang_php::code_rule_dsl::language::adapter::syntax::lang_blade;
            pub use rerail_lang_ecma::code_rule_dsl::language::adapter::syntax::lang_ecma;
            pub use rerail_lang_php::code_rule_dsl::language::adapter::syntax::lang_php;
            pub use rerail_lang_python::code_rule_dsl::language::adapter::syntax::lang_python;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::{Adapter, Refusal};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::compile::config::Options;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::compile::options;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::SourceType;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::literal::Grammar;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::scope::options::{LanguageOptions, PythonOptions};
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use serde_json::Value;
            #[allow(unused_imports)]
            use std::borrow::Cow;
            #[allow(unused_imports)]
            use std::sync::Arc;
        }
        #[path = "value"]
        pub mod value {
            pub use rerail_w::code_rule_dsl::language::adapter::value::*;
            pub use rerail_lang_ecma::code_rule_dsl::language::adapter::value::lang_ecma;
            pub use rerail_lang_php::code_rule_dsl::language::adapter::value::lang_php;
            pub use rerail_lang_python::code_rule_dsl::language::adapter::value::lang_python;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::Refusal;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::Family;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::SemanticText;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::value::structure::{Equality, Tok};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::value::{
                self, BigInt, Context, Ordered, TypeNames, Value, key, php_compare, php_structure,
            };
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use std::cmp::Ordering;
        }
        pub use rerail_lang_ecma::code_rule_dsl::language::adapter::vue;
        #[allow(unused_imports)]
        pub(crate) use crate::code_rule_dsl::language::adapter::arguments::{ArgumentFeature, ArgumentRules};
        #[allow(unused_imports)]
        pub(crate) use crate::code_rule_dsl::language::adapter::calls::{CallFeature, CallRules};
        #[allow(unused_imports)]
        pub(crate) use crate::code_rule_dsl::language::adapter::format_args::{FormatArgFeature, FormatArgRules};
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        #[cfg_attr(not(feature = "lang-json"), allow(unused_imports))]
        pub(crate) use crate::code_rule_dsl::language::adapter::json::Json;
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        #[cfg_attr(not(feature = "lang-rr"), allow(unused_imports))]
        pub(crate) use crate::code_rule_dsl::language::adapter::rr::Rr;
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        #[cfg_attr(not(feature = "lang-rust"), allow(unused_imports))]
        pub(crate) use crate::code_rule_dsl::language::adapter::rust::Rust;
        #[allow(unused_imports)]
        pub(crate) use crate::code_rule_dsl::language::adapter::scope::{ScopeFeature, ScopeRules};
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        #[cfg_attr(not(feature = "lang-sql"), allow(unused_imports))]
        pub(crate) use crate::code_rule_dsl::language::adapter::sql::Sql;
        #[allow(unused_imports)]
        pub(crate) use crate::code_rule_dsl::language::adapter::syntax::{QueryWhen, SyntaxFeature, SyntaxRules};
        #[allow(unused_imports)]
        pub(crate) use crate::code_rule_dsl::language::adapter::value::{ValueFeature, ValueRules};
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        #[cfg_attr(not(feature = "lang-vue"), allow(unused_imports))]
        pub(crate) use crate::code_rule_dsl::language::adapter::vue::Vue;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::{
            Family, GrammarDialect, PackageType, SourceType, is_declaration_file, language_not_supported,
        };
        #[allow(unused_imports)]
        use crate::code_rule_dsl::identifier::{ecma_name, python_name};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::json::invalid;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::lines::{LineTable, Terminators};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::matcher::{file_index};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::model::{ViolationSpec, python_module_path_on_disk};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::suppress::{Suppress, root};
        #[allow(unused_imports)]
        use ast_grep_core::{Doc, Node};
        #[allow(unused_imports)]
        use std::borrow::Cow;
        #[allow(unused_imports)]
        use std::ops::Range;
        #[allow(unused_imports)]
        use std::path::Path;
        #[allow(unused_imports)]
        use std::sync::LazyLock;
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        pub(in crate::code_rule_dsl) use crate::code_rule_dsl::language::adapter::python::{Python};
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        pub(in crate::code_rule_dsl) use crate::code_rule_dsl::language::adapter::ecma::{Ecma};
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        pub(in crate::code_rule_dsl) use crate::code_rule_dsl::language::adapter::php::{Php};
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        pub(in crate::code_rule_dsl) use crate::code_rule_dsl::language::adapter::blade::{Blade};
        #[allow(unused_imports)]
        pub(crate) use crate::code_rule_dsl::language::{ast_grep, parse_input};
    }
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
#[path = "lines"]
mod lines {
    pub use rerail_lang::code_rule_dsl::lines::*;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::language::Family;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::language::GrammarDialect;
    #[allow(unused_imports)]
    use serde::{Deserialize, Serialize};
}
#[path = "literal"]
mod literal {
    pub use rerail_w::code_rule_dsl::literal::*;
    pub use rerail_lang_python::code_rule_dsl::literal::annotation;
    pub use rerail_lang_python::code_rule_dsl::literal::brace;
    pub use rerail_lang_ecma::code_rule_dsl::literal::eslint_directive;
    pub use rerail_lang_python::code_rule_dsl::literal::identifier_words;
    pub use rerail_lang_python::code_rule_dsl::literal::percent;
    pub use rerail_lang_php::code_rule_dsl::literal::php_string;
    pub use rerail_lang_php::code_rule_dsl::literal::phpdoc;
    pub use rerail_lang_php::code_rule_dsl::literal::printf;
    pub use rerail_lang_python::code_rule_dsl::literal::python_fstring;
    pub use rerail_lang_python::code_rule_dsl::literal::python_string;
    pub use rerail_lang_rust::code_rule_dsl::literal::rust_format;
    pub use rerail_lang_php::code_rule_dsl::literal::scanf;
    pub use rerail_lang_python::code_rule_dsl::literal::semantic;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::embed::segments::FileRanges;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::language::GrammarDialect;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::value::Value;
    #[allow(unused_imports)]
    use ast_grep_core::Position;
    #[allow(unused_imports)]
    use ast_grep_core::language::Language;
    #[allow(unused_imports)]
    use ast_grep_core::matcher::{Pattern, PatternBuilder, PatternError};
    #[allow(unused_imports)]
    use ast_grep_core::source::{Content, Doc, Edit, SgNode};
    #[allow(unused_imports)]
    use crate::code_rule_dsl::literal::offsets::Decoded;
    #[allow(unused_imports)]
    use std::borrow::Cow;
    #[allow(unused_imports)]
    use std::ops::Range;
    #[allow(unused_imports)]
    use std::sync::Arc;
    #[allow(unused_imports)]
    use std::sync::atomic::{AtomicUsize, Ordering};
}
#[path = "logic"]
pub(crate) mod logic {
    pub use rerail_logic::code_rule_dsl::logic::*;
    pub mod convert;
    #[path = "front"]
    pub mod front {
        pub use rerail_rr::code_rule_dsl::logic::front::*;
        #[path = "caches"]
        pub mod caches {
            pub use rerail_rr::code_rule_dsl::logic::front::caches::*;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::logic::front::{Error, Literal, Program, Term};
            #[allow(unused_imports)]
            use std::collections::BTreeSet;
        }
        #[path = "yaml"]
        pub mod yaml {
            pub use rerail_rr::code_rule_dsl::logic::front::yaml::*;
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
    #[path = "magic"]
    pub mod magic {
        pub use rerail_rr::code_rule_dsl::logic::magic::*;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::logic::front::{AggOp, Atom, Error, Literal, Program, Rule, Term, Value, cmp_binds, join, strata, tarjan};
        #[allow(unused_imports)]
        use std::collections::{BTreeMap, BTreeSet};
        #[allow(unused_imports)]
        pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::front::ast::{Gate, Report};
    }
    #[path = "native"]
    pub mod native {
        pub use rerail_logic::code_rule_dsl::logic::native::*;
        #[path = "reach"]
        pub mod reach {
            pub use rerail_logic::code_rule_dsl::logic::native::reach::*;
            #[cfg(test)]
            pub mod tests;
        }
        #[path = "setfix"]
        pub mod setfix {
            pub use rerail_logic::code_rule_dsl::logic::native::setfix::*;
            #[cfg(target_os="macos")]
            pub use rerail_device_metal::code_rule_dsl::logic::native::setfix::device;
            // The lab's tests run strata on the Metal device (the transducer's), which the engine alone sees.
            #[path = "lab"]
            pub mod lab {
                pub use rerail_logic::code_rule_dsl::logic::native::setfix::lab::*;
                #[cfg(test)]
                pub mod tests;
            }
            #[allow(unused_imports)]
            use crate::code_rule_dsl::logic::analysis::scalars::Kind as ScalarKind;
            #[allow(unused_imports)]
            use rusqlite::ffi;
            #[allow(unused_imports)]
            use rusqlite::types::{Value, ValueRef};
            #[allow(unused_imports)]
            use rusqlite::vtab::{Context as VContext, CreateVTab, Filters, IndexConstraintOp, IndexInfo, VTab, VTabConnection, VTabCursor, VTabKind};
            #[allow(unused_imports)]
            use rusqlite::{Connection, Error, Result};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::facts::vtab::Scoped;
            #[allow(unused_imports)]
            use std::borrow::Cow;
            #[allow(unused_imports)]
            use hashbrown::HashTable;
            #[allow(unused_imports)]
            use rustc_hash::{FxHashMap as HashMap, FxHasher};
            #[allow(unused_imports)]
            use std::ffi::{CStr, CString};
            #[allow(unused_imports)]
            use std::hash::Hasher;
            #[allow(unused_imports)]
            use std::os::raw::c_int;
            #[allow(unused_imports)]
            use std::sync::{Arc, LazyLock, Mutex, RwLock};
            #[allow(unused_imports)]
            use std::time::Instant;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::logic::native::setfix::spec::{ComponentSpec, InputKey, JoinItem, JoinSpec, Line, Reprobe, Source, Start, Tok};
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::device::{Report as DeviceReport, take_report as device_report};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::logic::device::{JoinPlan};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::logic::profile::{Unit, work as charge};
        }
        #[cfg(test)]
        pub mod tests;
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        pub(crate) use crate::code_rule_dsl::logic::native::setfix::DERIVATIONS;
        #[allow(unused_imports)]
        pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::native::setfix::{forget_engine, forget_scope};
        #[allow(unused_imports)]
        pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::native::setfix::{flush_symbols, forget_inputs, intern_batch, keep_inputs, register_symbols, sym_text_of};
        #[allow(unused_imports)]
        pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::native::setfix::symbol_text;
        #[allow(unused_imports)]
        use rusqlite::ffi;
        #[allow(unused_imports)]
        use rusqlite::functions::{Aggregate, Context, FunctionFlags};
        #[allow(unused_imports)]
        use rusqlite::vtab::{Context as VContext, CreateVTab, Filters, IndexConstraintOp, IndexInfo, Module, VTab, VTabConnection, VTabCursor, VTabKind};
        #[allow(unused_imports)]
        use rusqlite::{Connection, Error, Result};
        #[allow(unused_imports)]
        use std::borrow::Cow;
        #[allow(unused_imports)]
        use std::ffi::CStr;
        #[allow(unused_imports)]
        use std::sync::Mutex;
        #[allow(unused_imports)]
        use std::os::raw::c_int;
        #[allow(unused_imports)]
        use std::sync::{Arc, LazyLock};
        #[allow(unused_imports)]
        use std::time::Instant;
    }
    #[path = "plan"]
    pub mod plan {
        pub use rerail_logic::code_rule_dsl::logic::plan::*;
        // Native demand's integration tests read the content store's documents through the engine's facts.
        #[path = "native_demand"]
        pub mod native_demand {
            pub use rerail_logic::code_rule_dsl::logic::plan::native_demand::*;
            #[cfg(test)]
            #[path = "tests"]
            pub mod tests {
                pub mod integration_tests;
            }
        }
        #[path = "corpus"]
        pub mod corpus {
            pub use rerail_logic::code_rule_dsl::logic::plan::corpus::*;
            pub mod references;
            // The corpus's tests run whole rule sets (the engine's): its own, a session's and the AST matchers'.
            #[cfg(test)]
            pub mod tests;
            #[path = "session"]
            pub mod session {
                pub use rerail_logic::code_rule_dsl::logic::plan::corpus::session::*;
                #[cfg(test)]
                pub mod tests;
            }
            #[path = "matchers"]
            pub mod matchers {
                pub use rerail_logic::code_rule_dsl::logic::plan::corpus::matchers::*;
                #[cfg(test)]
                pub mod tests;
            }
            #[allow(unused_imports)]
            use crate::code_rule_dsl::logic::plan::{Error, Outcome, Result, lower};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::facts::core::{self, Built};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::facts::vtab::{Modules, syntax};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::GrammarDialect;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::logic::front;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::logic::rr::WORLD;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::typing::cross_rounds;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::plan::corpus::hiding::qualified;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::plan::corpus::inputs::parameters as input_parameters;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::plan::corpus::matchers::columns as ast_columns;
            #[allow(unused_imports)]
            pub(crate) use crate::code_rule_dsl::logic::plan::corpus::typing::{Typing, TypingReport};
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
            use std::path::Path;
            #[allow(unused_imports)]
            use std::sync::Arc;
            #[allow(unused_imports)]
            pub(crate) use crate::code_rule_dsl::prof::thread_usage;
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, BTreeSet};
        }
        #[cfg(test)]
        pub mod tests;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::logic::front::{AggOp, Atom, Literal, Program, Rule, Term, Value, cmp_binds, strata};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::logic::native::TOP;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::logic::native::component::spec::{Arg, ComponentSpec, EdgeSpec, Extreme, HeadTok, InputKey, JoinItem, JoinSpec, Line as SpecLine, Reprobe, Source as JoinSource, Start, Tok};
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
        use crate::code_rule_dsl::logic::sql::select::{Cte, FromItem, Join, Query, Relation, Select, Source};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::logic::sql::sql_ast::{SqlBinaryOperator as Op, SqlExpression as Expr, SqlUnaryOperator, SqlValue as Value_};
    }
    #[path = "profile"]
    pub mod profile {
        pub use rerail_prof::code_rule_dsl::logic::profile::*;
        pub use rerail_logic::code_rule_dsl::logic::profile::report;
        pub use rerail_logic::code_rule_dsl::logic::profile::watch;
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
    #[path = "rr"]
    pub mod rr {
        pub use rerail_logic::code_rule_dsl::logic::rr::*;
        #[path = "aggregate_demand"]
        pub mod aggregate_demand {
            pub use rerail_rr::code_rule_dsl::logic::rr::aggregate_demand::*;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::logic::front::{self, AggOp, Atom, Literal, Program, Rule, Term};
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, BTreeSet};
        }
        #[cfg(test)]
        pub mod tape_tests;
        #[path = "tape_type"]
        pub mod tape_type {
            pub use rerail_rr::code_rule_dsl::logic::rr::tape_type::*;
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, BTreeSet};
            #[allow(unused_imports)]
            use rerail_rr_syntax::syntax::{Head, Item, Lit, Statement, Term};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::logic::front::Error;
        }
        #[cfg(test)]
        pub mod tests;
        #[allow(unused_imports)]
        use rerail_rr_syntax::syntax;
        #[allow(unused_imports)]
        pub(in crate::code_rule_dsl) use rerail_rr_syntax::tape;
        #[allow(unused_imports)]
        use std::collections::BTreeSet;
        #[allow(unused_imports)]
        use std::path::{Path, PathBuf};
        #[allow(unused_imports)]
        pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::front::Error;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::logic::front::Program;
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::rr::lower::drop_unread_core;
        #[allow(unused_imports)]
        pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::rr::aggregate_demand::retype_helpers;
        #[allow(unused_imports)]
        pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::rr::lower::is_core;
        #[allow(unused_imports)]
        pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::rr::lower::WORLD;
        #[allow(unused_imports)]
        pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::rr::print::print;
        #[allow(unused_imports)]
        pub(in crate::code_rule_dsl) use syntax::quote;
        #[allow(unused_imports)]
        pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::rr::translate::translate;
        #[allow(unused_imports)]
        pub(in crate::code_rule_dsl) use crate::code_rule_dsl::facts::vtab::syntax::core_rr::single_metavars;
    }
    #[path = "strings"]
    pub mod strings {
        pub use rerail_facts::code_rule_dsl::logic::strings::*;
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
}
/// A pool job's span under the span that started it (`crate::threads`), so a job's work is charged to its caller.
/// The profile's SQL hazards, which the content store's connections and writer report (`content_store::hazards`, set as
/// the library loads; decision 0104).
pub(crate) mod sql_hazards {
    pub(crate) use super::logic::profile::{behind_the_run, sql_in_wave, strict_statement, strict_on_every_connection, tracing as on, wait_in_wave};
}
#[path = "sql_executor"]
mod sql_executor {
    pub use rerail_sessions::code_rule_dsl::sql_executor::*;
    #[cfg(test)]
    pub mod tests;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::facts::core::{self, Built};
    #[allow(unused_imports)]
    use crate::code_rule_dsl::facts::vtab::{Relation, Source, syntax};
    #[allow(unused_imports)]
    use crate::code_rule_dsl::logic::sql::select;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::logic::{front, plan, rr, strings};
    #[allow(unused_imports)]
    use crate::code_rule_dsl::waves::{owner::Database, schedule};
    #[allow(unused_imports)]
    use crate::content_store::ContentStore;
    #[allow(unused_imports)]
    use napi::{Error, Result};
    #[allow(unused_imports)]
    use napi_derive::napi;
    #[allow(unused_imports)]
    use rusqlite::OptionalExtension;
    #[allow(unused_imports)]
    use rusqlite::types::{Value as SqlValue, ValueRef};
    #[allow(unused_imports)]
    use serde_json::{Value, json};
    #[allow(unused_imports)]
    use std::collections::{BTreeMap, BTreeSet, HashMap};
    #[allow(unused_imports)]
    use std::path::Path;
    #[allow(unused_imports)]
    use std::sync::Arc;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::logic::sql::lattices;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::logic::sql::program;
}
#[path = "matcher"]
mod matcher {
    pub use rerail_w::code_rule_dsl::matcher::*;
    #[path = "anchors"]
    pub mod anchors {
        pub use rerail_w::code_rule_dsl::matcher::anchors::*;
        #[cfg(test)]
        pub mod tests;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::matcher::{AstMatcher, StopBy, UtilRef};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::budget;
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use crate::code_rule_dsl::walks::{Walk, record_walk};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::GrammarDialect;
        #[allow(unused_imports)]
        use ast_grep_core::matcher::{Matcher, PatternNode, kind_utils};
        #[allow(unused_imports)]
        use ast_grep_core::source::SgNode;
        #[allow(unused_imports)]
        use ast_grep_core::{AstGrep, Doc, MatchStrictness, Node};
        #[allow(unused_imports)]
        use foldhash::{HashMap, HashMapExt, HashSet};
        #[allow(unused_imports)]
        use regex::Regex;
        #[allow(unused_imports)]
        use std::borrow::Cow;
        #[allow(unused_imports)]
        use std::cell::RefCell;
        #[allow(unused_imports)]
        use std::collections::{BTreeMap, BTreeSet};
        #[allow(unused_imports)]
        use std::num::NonZeroUsize;
        #[allow(unused_imports)]
        use std::sync::Arc;
        #[allow(unused_imports)]
        use rerail_store::probes::TextProbe;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::adapter::pack as language_pack;
    }
    #[path = "arguments"]
    pub mod arguments {
        pub use rerail_w::code_rule_dsl::matcher::arguments::*;
        pub use rerail_lang_php::code_rule_dsl::matcher::arguments::lang_php;
        pub use rerail_lang_python::code_rule_dsl::matcher::arguments::lang_python;
        pub use rerail_lang_rust::code_rule_dsl::matcher::arguments::rust;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::tree::Tree;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::matcher::util::UNKNOWN;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::matcher::value::spend_step;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::identifier::python_identifier;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::{Family, GrammarDialect};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::value::{Context, Value};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::value::{php, python};
        #[allow(unused_imports)]
        use ast_grep_core::meta_var::MetaVarEnv;
        #[allow(unused_imports)]
        use ast_grep_core::{Doc, Node};
        #[allow(unused_imports)]
        use std::borrow::Cow;
        #[allow(unused_imports)]
        use std::collections::BTreeSet;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::adapter::pack as language_pack;
    }
    pub use rerail_lang_sql::code_rule_dsl::matcher::arguments_sql;
    #[cfg(any(test, feature = "test-hooks"))]
    #[path = "d_anchors"]
    pub mod d_anchors {
        pub use rerail_w::code_rule_dsl::matcher::d_anchors::*;
        #[cfg(test)]
        pub mod tests;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::matcher::{AstMatcher, StopBy, UtilRef};
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use crate::code_rule_dsl::budget;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::adapter::{Adapter, ValueRules};
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use crate::code_rule_dsl::walks::{Walk, record_walk};
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use crate::code_rule_dsl::language::GrammarDialect;
        #[allow(unused_imports)]
        use ast_grep_core::MatchStrictness;
        #[allow(unused_imports)]
        use ast_grep_core::matcher::{Matcher, PatternNode, kind_utils};
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use ast_grep_core::source::SgNode;
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use ast_grep_core::{AstGrep, Doc, Node};
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use foldhash::{HashMap, HashMapExt, HashSet};
        #[allow(unused_imports)]
        use regex::Regex;
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use std::borrow::Cow;
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use std::cell::RefCell;
        #[allow(unused_imports)]
        use std::collections::{BTreeMap, BTreeSet};
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use std::num::NonZeroUsize;
        #[allow(unused_imports)]
        use std::sync::Arc;
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use crate::code_rule_dsl::matcher::d_anchors::text::TextProbe;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::adapter::pack as language_pack;
    }
    #[path = "format_args"]
    pub mod format_args {
        pub use rerail_w::code_rule_dsl::matcher::format_args::*;
        pub use rerail_lang_php::code_rule_dsl::matcher::format_args::printf;
        pub use rerail_lang_python::code_rule_dsl::matcher::format_args::python;
        pub use rerail_lang_rust::code_rule_dsl::matcher::format_args::rust;
        pub use rerail_lang_php::code_rule_dsl::matcher::format_args::scanf;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::tree::Tree;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::matcher::AstMatcher;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::matcher::each::{self};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::matcher::parse::parsed_tree;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::matcher::util::{UNKNOWN};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::matcher::value::Reading;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::literal::{Grammar, Parsed, SubDoc, offsets};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::origin::Origin;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::transducer::semantic_facts::{Facts, Target, slot};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::value::Value;
        #[allow(unused_imports)]
        use ast_grep_core::Matcher;
        #[allow(unused_imports)]
        use ast_grep_core::meta_var::MetaVarEnv;
        #[allow(unused_imports)]
        use ast_grep_core::{AstGrep, Doc, Node};
        #[allow(unused_imports)]
        use std::borrow::Cow;
        #[allow(unused_imports)]
        use std::collections::BTreeSet;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::GrammarDialect;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::tape::TapeDoc;
        #[allow(unused_imports)]
        use ast_grep_core::tree_sitter::StrDoc;
    }
    #[cfg(any(test, feature = "test-hooks"))]
    #[path = "p_anchors"]
    pub mod p_anchors {
        pub use rerail_w::code_rule_dsl::matcher::p_anchors::*;
        #[path = "sql"]
        pub mod sql {
            pub use rerail_w::code_rule_dsl::matcher::p_anchors::sql::*;
            #[cfg(test)]
            pub mod lab;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::matcher::p_anchors::sql::audit::candidate_frontier_audit;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::matcher::p_anchors::sql::corpus::{Corpus, CorpusSlot};
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::matcher::p_anchors::sql::plans::{
                for_file as anchor_plans_for_file, plans as anchor_plans,
            };
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::p_anchors::AnchorPlan;
            #[allow(unused_imports)]
            #[cfg(any(test, feature = "test-hooks"))]
            use crate::code_rule_dsl::matcher::p_anchors::Selection;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::{language, tape::TapeDoc};
            #[allow(unused_imports)]
            #[cfg(any(test, feature = "test-hooks"))]
            use ast_grep_core::Node;
            #[allow(unused_imports)]
            use ast_grep_core::{AstGrep, Doc};
            #[allow(unused_imports)]
            #[cfg(any(test, feature = "test-hooks"))]
            use std::borrow::Cow;
        }
        #[cfg(test)]
        pub mod tests;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::matcher::{AstMatcher, StopBy, UtilRef};
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use crate::code_rule_dsl::budget;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::adapter::{Adapter, ValueRules};
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use crate::code_rule_dsl::walks::{Walk, record_walk};
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use crate::code_rule_dsl::language::GrammarDialect;
        #[allow(unused_imports)]
        use ast_grep_core::MatchStrictness;
        #[allow(unused_imports)]
        use ast_grep_core::matcher::{Matcher, PatternNode, kind_utils};
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use ast_grep_core::source::SgNode;
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use ast_grep_core::{AstGrep, Doc, Node};
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use foldhash::{HashMap, HashMapExt, HashSet};
        #[allow(unused_imports)]
        use regex::Regex;
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use std::borrow::Cow;
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use std::cell::RefCell;
        #[allow(unused_imports)]
        use std::collections::{BTreeMap, BTreeSet};
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use std::num::NonZeroUsize;
        #[allow(unused_imports)]
        use std::sync::Arc;
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use crate::code_rule_dsl::matcher::p_anchors::text::TextProbe;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::adapter::pack as language_pack;
    }
    #[cfg(any(test, feature = "test-hooks"))]
    pub use rerail_lang_python::code_rule_dsl::matcher::python_facts;
    #[path = "resolve"]
    pub mod resolve {
        pub use rerail_w::code_rule_dsl::matcher::resolve::*;
        pub use rerail_lang_php::code_rule_dsl::matcher::resolve::php;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::tree::Tree;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::matcher::GrammarMatcher;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::matcher::util::{UNKNOWN, UtilLimit};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::budget;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::identifier::{python_identifier};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::{Family, GrammarDialect};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::{
            JsxRuntime, SourceType, has_module_syntax, is_stub_file, jsx_runtime_in_source,
        };
        #[allow(unused_imports)]
        use crate::code_rule_dsl::lines::{LineTable, Terminators};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::literal::{Grammar, SubTree};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::model::FileInput;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::module_path::ModulePath;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::scope::options::{DEFAULT_PYTHON_MINOR, LanguageOptions, PythonOptions};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::scope::usage::Usage;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::scope::{Binding, BuildError, Import, Reading, ScopeTable, TYPE, VALUE};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::suppress::RerailIgnores;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::value::NodeRef;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::value::structure::{Equality, Tok};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::value::{Followed, Value};
        #[allow(unused_imports)]
        use ast_grep_core::{Doc, Node};
        #[allow(unused_imports)]
        use foldhash::{HashMap, HashMapExt};
        #[allow(unused_imports)]
        use std::cell::{Cell, OnceCell, RefCell};
        #[allow(unused_imports)]
        use std::collections::BTreeSet;
        #[allow(unused_imports)]
        use std::ops::Range;
        #[allow(unused_imports)]
        use std::rc::Rc;
        #[allow(unused_imports)]
        use std::sync::Arc;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::adapter::pack as language_pack;
    }
    #[allow(unused_imports)]
    #[cfg(any(test, feature = "test-hooks"))]
    pub(in crate::code_rule_dsl) use crate::code_rule_dsl::matcher::p_anchors::sql::{Corpus, CorpusSlot};
    #[allow(unused_imports)]
    #[cfg(all(any(test, feature = "test-hooks"), feature = "p-anchor-corpus"))]
    pub(in crate::code_rule_dsl) use crate::code_rule_dsl::matcher::p_anchors::sql::{anchor_plans, anchor_plans_for_file};
    #[allow(unused_imports)]
    use crate::code_rule_dsl::budget;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::language::{GrammarDialect, SourceType};
    #[allow(unused_imports)]
    pub(super) use crate::code_rule_dsl::matcher::analysis::RequiredText;
    #[allow(unused_imports)]
    #[cfg_attr(not(test), allow(unused_imports))]
    pub(super) use crate::code_rule_dsl::matcher::lower::{Axis, Fact, FactFound, MatchTree, Stop, DocumentGate, gated_match_tree, match_tree};
    #[allow(unused_imports)]
    pub(crate) use crate::code_rule_dsl::matcher::analysis::{calls_positive_only, reads_import_targets};
    #[allow(unused_imports)]
    pub(super) use crate::code_rule_dsl::matcher::anchors::{AnchorPlan, FileAnchorIndex, Selection};
    #[allow(unused_imports)]
    pub(super) use crate::code_rule_dsl::matcher::anchors::{Condition, Direction, Required, Requirement};
    #[allow(unused_imports)]
    pub(super) use crate::code_rule_dsl::matcher::arguments::{Arguments, Passed, SlotKind};
    #[allow(unused_imports)]
    use ast_grep_core::matcher::Matcher;
    #[allow(unused_imports)]
    use ast_grep_core::matcher::{KindMatcher, Pattern};
    #[allow(unused_imports)]
    use ast_grep_core::matcher::{MatcherExt, NodeMatch};
    #[allow(unused_imports)]
    use ast_grep_core::meta_var::{MetaVarEnv, MetaVariable};
    #[allow(unused_imports)]
    use ast_grep_core::{Doc, Node};
    #[allow(unused_imports)]
    pub(super) use crate::code_rule_dsl::matcher::binding::BindingTest;
    #[allow(unused_imports)]
    pub(super) use crate::code_rule_dsl::matcher::binds::{BindingCount, BindingFilter, Binds, Redefines, ScopeName};
    #[allow(unused_imports)]
    pub(super) use crate::code_rule_dsl::matcher::captures::{Capture, CaptureAnalysis, collect_variables, parse_exported};
    #[allow(unused_imports)]
    pub(super) use crate::code_rule_dsl::matcher::compare::{Collation, CompareOp, CompareText, Comparison, EqClass, SemanticText};
    #[allow(unused_imports)]
    pub(super) use crate::code_rule_dsl::matcher::count::{Count, CountBinding};
    #[allow(unused_imports)]
    pub(crate) use crate::code_rule_dsl::matcher::each::binding_range;
    #[allow(unused_imports)]
    pub(crate) use crate::code_rule_dsl::matcher::embedded::EmbeddedTest;
    #[allow(unused_imports)]
    pub(crate) use crate::code_rule_dsl::matcher::exported::{
        WITNESS_NODE, export_text, exported_span, exported_text, is_span_variable,
    };
    #[allow(unused_imports)]
    pub(crate) use crate::code_rule_dsl::matcher::format_args::{FormatArgs, Style};
    #[allow(unused_imports)]
    pub(crate) use crate::code_rule_dsl::matcher::layout::{Gap, Layout, LineContext, LineCount, Neighbour};
    #[allow(unused_imports)]
    pub(super) use crate::code_rule_dsl::matcher::memo::with_file as with_file_memo;
    #[allow(unused_imports)]
    pub(super) use crate::code_rule_dsl::matcher::nth_child::{NthChild, Position as NthPosition, parse_position};
    #[allow(unused_imports)]
    pub(in crate::code_rule_dsl) use crate::code_rule_dsl::matcher::nth_child::SiblingPositions;
    #[allow(unused_imports)]
    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) use crate::code_rule_dsl::matcher::nth_child::{named_sibling_visits, reset_named_sibling_visits};
    #[allow(unused_imports)]
    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) use crate::code_rule_dsl::matcher::parse::parse_fact_tree;
    #[allow(unused_imports)]
    pub(crate) use crate::code_rule_dsl::matcher::parse::{HostTest, ParseTest};
    #[allow(unused_imports)]
    use regex::Regex;
    #[allow(unused_imports)]
    pub(super) use crate::code_rule_dsl::matcher::relation::{FieldRelation, Related, StopBy, stop_rule};
    #[allow(unused_imports)]
    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) use crate::code_rule_dsl::matcher::relation::{set_answers_off, set_same_key_off};
    #[allow(unused_imports)]
    pub(crate) use crate::code_rule_dsl::matcher::resolve::{
        FileGates, calls_tables, file_index, file_lines, file_module, file_rerail_ignores, file_source_type_for,
        table_reading,
    };
    #[allow(unused_imports)]
    pub(super) use crate::code_rule_dsl::matcher::resolve::{FileScopeSlot, Unresolved, matched_file_path};
    #[allow(unused_imports)]
    pub(crate) use crate::code_rule_dsl::matcher::resolve::Resolves;
    #[allow(unused_imports)]
    pub(crate) use crate::code_rule_dsl::matcher::resolve::php as resolve_php;
    #[allow(unused_imports)]
    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) use crate::code_rule_dsl::matcher::resolve::{JSX_RUNTIME_DERIVATIONS, SOURCE_TYPE_DERIVATIONS, TABLE_BUILDS};
    #[allow(unused_imports)]
    pub(super) use crate::code_rule_dsl::matcher::run::{Stopped, enumerate, node_at, run_clause};
    #[allow(unused_imports)]
    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) use crate::code_rule_dsl::matcher::run::{reset_roots_matched, roots_matched};
    #[allow(unused_imports)]
    pub(super) use crate::code_rule_dsl::matcher::semantic::Semantic;
    #[allow(unused_imports)]
    use std::borrow::Cow;
    #[allow(unused_imports)]
    use std::collections::BTreeSet;
    #[allow(unused_imports)]
    use std::sync::Arc;
    #[allow(unused_imports)]
    pub(super) use crate::code_rule_dsl::matcher::transform::{Digits, Separator, Source, StringCase, Trans, Transform};
    #[allow(unused_imports)]
    pub(crate) use crate::code_rule_dsl::matcher::transform::{NumberText, install_number_text, number_to_string};
    #[allow(unused_imports)]
    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) use crate::code_rule_dsl::matcher::transform::{
        line_table_constructions, note_line_table_construction, reset_line_table_constructions,
    };
    #[allow(unused_imports)]
    pub(super) use crate::code_rule_dsl::matcher::util::{
        CallArgs, MAX_UTIL_DEPTH, MAX_UTIL_INSTANCES, UtilCall, UtilLimit, UtilOrigin, UtilRef,
        UtilTable, UtilitySummary,
    };
    #[allow(unused_imports)]
    use crate::code_rule_dsl::matcher::util::{UNKNOWN};
    #[allow(unused_imports)]
    pub(super) use crate::code_rule_dsl::matcher::value::{Bound, Reading, ValueTest};
    #[allow(unused_imports)]
    pub(crate) use crate::code_rule_dsl::matcher::value::Values;
    #[allow(unused_imports)]
    pub(crate) use crate::code_rule_dsl::matcher::term::{LanguageTerm, TermCompiler, install_terms};
}
pub use rerail_w::code_rule_dsl::model;
#[path = "module_graph"]
mod module_graph {
    pub use rerail_lang_ecma::code_rule_dsl::module_graph::*;
    #[cfg(test)]
    pub mod tests;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::tree::Tree;
    #[allow(unused_imports)]
    use ast_grep_core::{Doc, Node};
    #[allow(unused_imports)]
    use ast_grep_language::LanguageExt;
    #[allow(unused_imports)]
    use indexmap::IndexSet;
    #[allow(unused_imports)]
    use parking_lot::Mutex;
    #[allow(unused_imports)]
    use regex::Regex;
    #[allow(unused_imports)]
    use serde_json::Value;
    #[allow(unused_imports)]
    use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
    #[allow(unused_imports)]
    use std::path::{Path, PathBuf};
    #[allow(unused_imports)]
    use std::sync::{Arc, LazyLock};
    #[allow(unused_imports)]
    use std::time::{Duration, SystemTime};
    #[allow(unused_imports)]
    use crate::code_rule_dsl::language::adapter::package_manifests;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::language::{dialect_by_label, label_of_path};
}
pub use rerail_lang::code_rule_dsl::module_path;
pub use rerail_lang_ecma::code_rule_dsl::npm_ignore;
pub use rerail_w::code_rule_dsl::origin;
pub use rerail_lang_ecma::code_rule_dsl::package_manifest;
// P's phase clocks; read by profiling once stages are named (05 Phase 11 row 2).
pub use rerail_prof::code_rule_dsl::phase_clock;
pub use rerail_lang_php::code_rule_dsl::php_literal;
#[path = "php_view"]
mod php_view {
    pub use rerail_lang_php::code_rule_dsl::php_view::*;
    #[cfg(test)]
    pub mod tests;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::blade::forte::scan;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::php_literal;
    #[allow(unused_imports)]
    use std::borrow::Cow;
    #[allow(unused_imports)]
    use std::ops::Range;
}
pub use rerail_w::code_rule_dsl::placement;
pub use rerail_facts::code_rule_dsl::producers;
// Published from the scan in a later slice; until then only tests call it.
#[allow(dead_code)]
pub(crate) mod prefilter;
pub use rerail_transducer::code_rule_dsl::space;
// Read by the prefilter's lowering; the compiled-plan cache is wired with the plans row.
#[path = "stats"]
mod stats {
    pub use rerail_w::code_rule_dsl::stats::*;
    #[path = "ingest"]
    pub mod ingest {
        pub use rerail_w::code_rule_dsl::stats::ingest::*;
        #[cfg(test)]
        pub mod tests;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::facts::core::KIND_STRIDE as LANG_STRIDE;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::search_index::kind_code;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::tape::TapeDoc;
        #[allow(unused_imports)]
        use rusqlite::{Connection, Result, params};
        #[allow(unused_imports)]
        use std::collections::BTreeMap;
    }
    #[path = "inputs"]
    pub mod inputs {
        pub use rerail_w::code_rule_dsl::stats::inputs::*;
        #[cfg(test)]
        pub mod tests;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::matcher::{AnchorPlan, Condition, Required, Requirement};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::facts::core::KIND_STRIDE as LANG_STRIDE;
        #[allow(unused_imports)]
        use rusqlite::{Connection, Result, limits::Limit, params_from_iter, types::Value};
        #[allow(unused_imports)]
        use std::collections::{BTreeMap, BTreeSet, HashSet};
        #[allow(unused_imports)]
        use std::sync::Arc;
    }
    #[path = "plans"]
    pub mod plans {
        pub use rerail_w::code_rule_dsl::stats::plans::*;
        #[cfg(test)]
        pub mod tests;
        #[allow(unused_imports)]
        use rusqlite::{Connection, OptionalExtension, Result, params};
        #[allow(unused_imports)]
        use std::collections::{BTreeMap, btree_map};
        #[allow(unused_imports)]
        use std::sync::Arc;
    }
}
// Wall time by stage, with RERAIL_PROF set (O28).
pub use rerail_prof::code_rule_dsl::prof;
// P's hooks are called from its matcher and relational code; W wires them with 03's stage names.
#[cfg(feature="code-rule-profile")]
pub use rerail_w::code_rule_dsl::profile;
// The same SQL diagnostics serve standalone execution and the workspace profiler.
pub use rerail_facts::code_rule_dsl::sql_profile;
#[path = "query"]
mod query {
    pub use rerail_w::code_rule_dsl::query::*;
    #[cfg(any(test, feature = "test-hooks"))]
    pub mod plan;
    #[cfg(test)]
    pub mod tests;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::model::RegexOptions;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::model::{FileInput, RegexSource};
    #[allow(unused_imports)]
    use regex::Regex;
    #[allow(unused_imports)]
    use std::ops::Range;
}
// Admission reads these once the batch budget is wired; until then only tests call it.
pub use rerail_w::code_rule_dsl::resolution;
pub use rerail_facts::code_rule_dsl::search_index;
#[cfg(any(test, feature = "test-hooks"))]
pub use rerail_w::code_rule_dsl::walks;
mod functors;
pub use rerail_w::code_rule_dsl::rule_api;
#[path = "residency"]
mod residency {
    pub use rerail_transducer::code_rule_dsl::residency::*;
    #[path = "admission"]
    pub mod admission {
        pub use rerail_w::code_rule_dsl::residency::admission::*;
        #[path = "relational"]
        pub mod relational {
            pub use rerail_w::code_rule_dsl::residency::admission::relational::*;
            #[path = "batch"]
            pub mod batch {
                pub use rerail_w::code_rule_dsl::residency::admission::relational::batch::*;
                #[cfg(test)]
                pub mod tests;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::residency::admission::relational::{Inputs, Job, Preparation, failure, producer, sql_error};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::model::{CompiledCodeRule, Metrics};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::{budget::Context, model::CompiledClause};
                #[allow(unused_imports)]
                use std::collections::VecDeque;
            }
            #[path = "inputs"]
            pub mod inputs {
                pub use rerail_w::code_rule_dsl::residency::admission::relational::inputs::*;
                #[cfg(test)]
                pub mod tests;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::residency::admission::relational::{FactQuery, insert};
                #[allow(unused_imports)]
                use crate::code_rule_dsl::{
                    eval,
                    model::{CompiledClause, CompiledCodeRule, Metrics, Snapshot},
                    violations::{Witness, anchored_span, projection::Row},
                };
                #[allow(unused_imports)]
                use napi::Result;
                #[allow(unused_imports)]
                use rusqlite::{Connection, types::ValueRef};
                #[allow(unused_imports)]
                use std::{
                    cell::RefCell,
                    sync::{Arc, atomic::AtomicUsize},
                };
            }
            pub use rerail_lang_python::code_rule_dsl::residency::admission::relational::python_ast;
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
            use rusqlite::functions::FunctionFlags;
            #[allow(unused_imports)]
            use rusqlite::hooks::Authorization;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::residency::admission::relational::database::Database;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::residency::admission::relational::reachability::Reachability;
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
    }
    #[cfg(target_os="macos")]
    pub use rerail_device_metal::code_rule_dsl::residency::metal;
    #[cfg(target_os="macos")]
    pub use rerail_device_metal::code_rule_dsl::residency::slabs;
    #[allow(unused_imports)]
    pub(in crate::code_rule_dsl) use crate::code_rule_dsl::residency::admission::relational::ast_stage::install_ast_producer;
}
// P's paths for P's relational stage, under test (open item O18): P's workspace code reads it here.
use residency::admission::relational;
#[path = "scope"]
mod scope {
    pub use rerail_w::code_rule_dsl::scope::*;
    pub use rerail_lang_ecma::code_rule_dsl::scope::ecma;
    pub use rerail_lang_php::code_rule_dsl::scope::php;
    pub use rerail_lang_php::code_rule_dsl::scope::php_names;
    pub use rerail_lang_python::code_rule_dsl::scope::python;
    pub use rerail_lang_rust::code_rule_dsl::scope::rust;
    pub use rerail_lang_sql::code_rule_dsl::scope::sql;
    #[path = "usage"]
    pub mod usage {
        pub use rerail_w::code_rule_dsl::scope::usage::*;
        pub use rerail_lang_ecma::code_rule_dsl::scope::usage::ecma;
        pub use rerail_lang_ecma::code_rule_dsl::scope::usage::lang_ecma;
        pub use rerail_lang_php::code_rule_dsl::scope::usage::lang_php;
        pub use rerail_lang_python::code_rule_dsl::scope::usage::lang_python;
        pub use rerail_lang_rust::code_rule_dsl::scope::usage::lang_rust;
        pub use rerail_lang_sql::code_rule_dsl::scope::usage::lang_sql;
        pub use rerail_lang_php::code_rule_dsl::scope::usage::php;
        pub use rerail_lang_rust::code_rule_dsl::scope::usage::rust;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::scope::{
            Binding, BuildError, DEADLINE_INTERVAL, Form, ImportKind, NodeRef, ScopeKind, ScopeTable,
        };
        #[allow(unused_imports)]
        use crate::code_rule_dsl::tree::Tree;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::identifier::python_identifier;
        #[allow(unused_imports)]
        use ast_grep_core::{Doc, Node};
        #[allow(unused_imports)]
        use foldhash::{HashMap, HashMapExt};
        #[allow(unused_imports)]
        use std::borrow::Cow;
        #[allow(unused_imports)]
        use std::collections::BTreeSet;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::adapter::pack as language_pack;
    }
    #[allow(unused_imports)]
    pub(crate) use crate::code_rule_dsl::scope::lookup::Lookup;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::identifier::{nfkc};
    #[allow(unused_imports)]
    use crate::code_rule_dsl::language::{Family, GrammarDialect};
    #[allow(unused_imports)]
    use crate::code_rule_dsl::module_path::ModulePath;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::value::{Followed, NodeRef};
    #[allow(unused_imports)]
    use ast_grep_core::{Doc, Node};
    #[allow(unused_imports)]
    use ast_grep_language::LanguageExt;
    #[allow(unused_imports)]
    use foldhash::{HashMap, HashMapExt};
    #[allow(unused_imports)]
    use std::borrow::Cow;
    #[allow(unused_imports)]
    use std::collections::BTreeSet;
    #[allow(unused_imports)]
    use std::ops::Range;
    #[allow(unused_imports)]
    use std::sync::{Arc, OnceLock};
    #[allow(unused_imports)]
    use crate::code_rule_dsl::language::adapter::pack as language_pack;
}
pub use rerail_lang::code_rule_dsl::semver;
// The first slice's entry points (06 S6), called by hosts through `slice::napi::CodeRuleSlice`.
mod slice;
pub use rerail_w::code_rule_dsl::soundness;
#[path = "suppress"]
mod suppress {
    pub use rerail_w::code_rule_dsl::suppress::*;
    pub use rerail_lang_ecma::code_rule_dsl::suppress::biome;
    pub use rerail_lang_rust::code_rule_dsl::suppress::clippy;
    pub use rerail_lang_ecma::code_rule_dsl::suppress::eslint;
    pub use rerail_lang_python::code_rule_dsl::suppress::noqa;
    pub use rerail_lang_python::code_rule_dsl::suppress::nosec;
    pub use rerail_lang_ecma::code_rule_dsl::suppress::oxlint;
    pub use rerail_lang_php::code_rule_dsl::suppress::phpcs;
    pub use rerail_lang_php::code_rule_dsl::suppress::phpstan;
    pub use rerail_lang_php::code_rule_dsl::suppress::psalm;
    pub use rerail_lang_sql::code_rule_dsl::suppress::sqlfluff;
    pub use rerail_lang_ecma::code_rule_dsl::suppress::vue;
    #[allow(unused_imports)]
    use ast_grep_core::{Doc, Node};
    #[allow(unused_imports)]
    pub(crate) use crate::code_rule_dsl::suppress::rerail::RerailIgnores;
    #[allow(unused_imports)]
    pub(crate) use crate::code_rule_dsl::suppress::kinds::*;
}
#[path = "tape"]
mod tape {
    pub use rerail_lang::code_rule_dsl::tape::*;
    #[cfg(test)]
    pub mod tests;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::embed::segments::SegmentMap;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::embed::Program;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::language::{GrammarDialect, KindCatalog, Via};
    #[allow(unused_imports)]
    use crate::code_rule_dsl::lines::LineTable;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::lines::Terminators;
    #[allow(unused_imports)]
    use ast_grep_core::Position;
    #[allow(unused_imports)]
    use ast_grep_core::source::{Doc, Edit, SgNode};
    #[allow(unused_imports)]
    use ast_grep_language::LanguageExt;
    #[allow(unused_imports)]
    use sha2::{Digest, Sha256};
    #[allow(unused_imports)]
    use std::borrow::Cow;
    #[allow(unused_imports)]
    use std::ops::Range;
    #[allow(unused_imports)]
    use std::sync::{Arc, Weak};
    #[allow(unused_imports)]
    use tree_sitter::{Language, Node as TsNode, TreeCursor};
    #[allow(unused_imports)]
    #[allow(unused_imports)]
    pub(in crate::code_rule_dsl) use crate::code_rule_dsl::tape::index::IndexImage;
    #[allow(unused_imports)]
    pub(super) use crate::code_rule_dsl::tape::wire::index_span;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::tape::wire::{CHECKPOINT_STRIDE, WIDTH};
}
pub use rerail_lang::code_rule_dsl::tree;

/// The language tag a stored tape image names (`tape::stored_tape_tag`): the store picks a
/// frame's dictionary by it.
pub(crate) fn stored_tape_tag(bytes: &[u8]) -> Option<u8> {
    tape::stored_tape_tag(bytes)
}

/// The language tag of the dialect a file at `path` is parsed as.
pub(crate) fn path_tape_tag(path: &str) -> Option<u8> {
    language::tape_tag(language::dialect_by_label(language::label_of_path(path)?)?)
}
pub use rerail_facts::code_rule_dsl::text;
#[cfg(any(test, feature = "test-hooks"))]
mod test_support;
pub mod tools;
#[path = "transducer"]
mod transducer {
    pub use rerail_transducer::code_rule_dsl::transducer::*;
    #[path = "packed"]
    pub mod packed {
        pub use rerail_transducer::code_rule_dsl::transducer::packed::*;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::bounded_repeat_proof;
        #[path = "compile"]
        pub mod compile_matcher {
            pub use rerail_transducer::code_rule_dsl::transducer::packed::compile_matcher::*;
            #[cfg(target_os="macos")]
            pub use rerail_device_metal::code_rule_dsl::transducer::packed::compile_matcher::matcher_metal;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::transducer::packed::*;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::{Axis, MatchTree, Stop};
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, BTreeSet, HashMap};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::tape::DERIVED_CLASS_BASE as LABEL_BASE;
        }
        #[cfg(test)]
        #[path = "compile/matcher_tests.rs"]
        pub mod compile_matcher_tests;
        pub use rerail_lang_json::code_rule_dsl::transducer::packed::json_parser;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_backend;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_bulk_closure;
        #[cfg(all(any(test, feature = "test-hooks"),target_os="macos",feature="metal-tests"))]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_bulk_closure_tests;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_candidate_batch;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_capture_arguments;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_capture_groups;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_capture_literals;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_capture_predicates;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_closure_continuation;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_closure_prepare;
        #[cfg(all(any(test, feature = "test-hooks"),target_os="macos",feature="metal-tests"))]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_closure_prepare_tests;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_constant_output;
        #[cfg(all(any(test, feature = "test-hooks"),target_os="macos",feature="metal-tests"))]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_cutover_architecture;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_dynamic_execution;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_failure_records;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_frame_identity;
        #[cfg(all(any(test, feature = "test-hooks"),target_os="macos",feature="metal-tests"))]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_frame_identity_tests;
        #[cfg(all(any(test, feature = "test-hooks"),target_os="macos",feature="metal-tests"))]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_frontier_conformance;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_frontier_reduce;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_frontier_seed;
        #[cfg(all(any(test, feature = "test-hooks"),target_os="macos",feature="metal-tests"))]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_frontier_seed_tests;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_frontier_validation;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_grid_backend;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_grid_contract;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_grid_executor;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_grid_plan;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_initial_prefix;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_initial_range_codegen;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_inline_predicates;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_join;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_named_order;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_output_contract;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_output_dispatch;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_output_mime;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_output_pages;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_output_selection;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_output_strip;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_parallel;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_position_order;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_program_backend;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_ready_batch;
        #[cfg(all(any(test, feature = "test-hooks"),target_os="macos",feature="metal-tests"))]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_ready_batch_tests;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_ready_proof;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_regex_code;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_regex_frontier;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_register_codegen;
        #[cfg(all(any(test, feature = "test-hooks"),target_os="macos",feature="metal-tests"))]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_register_conformance;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_register_execution;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_rows;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_semantic_columns;
        #[cfg(all(any(test, feature = "test-hooks"),target_os="macos",feature="metal-tests"))]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_semantic_conformance;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_sort;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_state_values;
        #[cfg(all(any(test, feature = "test-hooks"),target_os="macos",feature="metal-tests"))]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_state_values_tests;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_successor;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_successor_branch_codegen;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_successor_codegen;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_successor_prepare;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_successor_repeat_codegen;
        #[cfg(all(any(test, feature = "test-hooks"),target_os="macos",feature="metal-tests"))]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_successor_tests;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_terminal;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_terminal_capture_order;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_terminal_repeat;
        #[cfg(target_os="macos")]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_typed_output;
        #[cfg(all(any(test, feature = "test-hooks"),target_os="macos",feature="metal-tests"))]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_unique_child_tests;
        #[cfg(all(any(test, feature = "test-hooks"),target_os="macos",feature="metal-tests"))]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::metal_value_prefix_tests;
        #[cfg(all(any(test, feature = "test-hooks"),target_os="macos",target_arch="aarch64"))]
        pub use rerail_device_metal::code_rule_dsl::transducer::packed::regex_relocation_tests;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::tape::TapeDoc;
        #[allow(unused_imports)]
        use ast_grep_core::MatchStrictness;
        #[allow(unused_imports)]
        use ast_grep_core::matcher::{Pattern, PatternNode};
        #[allow(unused_imports)]
        use ast_grep_core::meta_var::{MetaVarEnv, MetaVariable};
        #[allow(unused_imports)]
        use ast_grep_core::{Doc, Node};
        #[allow(unused_imports)]
        use regex::Regex;
        #[allow(unused_imports)]
        use sha2::{Digest as _, Sha256};
        #[allow(unused_imports)]
        use std::borrow::Cow;
        #[allow(unused_imports)]
        use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
        #[allow(unused_imports)]
        use std::fmt::Write as _;
        #[allow(unused_imports)]
        use std::ops::Range;
        #[allow(unused_imports)]
        use std::sync::Arc;
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        #[allow(unused_imports)]
        use crate::code_rule_dsl::transducer::packed::literal_anchors::*;
        #[allow(unused_imports)]
        #[cfg(all(any(test, feature = "test-hooks"), target_os = "macos"))]
        pub(super) use crate::code_rule_dsl::transducer::packed::metal_backend::launch::LaunchScope as MetalTestScope;
        #[allow(unused_imports)]
        #[cfg(target_os = "macos")]
        pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::metal_program_backend::set_steps;
        #[allow(unused_imports)]
        #[cfg(target_os = "macos")]
        pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::metal_program_backend::lattice as metal_lattice;
        #[allow(unused_imports)]
        #[cfg(target_os = "macos")]
        pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::metal_program_backend::nearest as metal_nearest;
        #[allow(unused_imports)]
        pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::device::{RootJobs, RootsFound};
        #[allow(unused_imports)]
        #[cfg(target_os = "macos")]
        pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::compile_matcher::matcher_metal;
        #[allow(unused_imports)]
        pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::compile_matcher::{Backend as MatcherBackend, LoweredMatcher, LoweredWave, WaveRun};
        #[allow(unused_imports)]
        pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::compile_micro::{Emit, Frames, Micro, RootRecord, steps as micro_steps};
        #[allow(unused_imports)]
        pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::compilation::PlanId;
        #[allow(unused_imports)]
        use rerail_compiler::native_cpu_codegen;
        #[allow(unused_imports)]
        #[allow(unused_imports)]
        pub(in crate::code_rule_dsl) use rerail_compiler::native_cpu_rows;
        #[allow(unused_imports)]
        pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::packed_batch::PackedExecution;
        #[allow(unused_imports)]
        pub(in crate::code_rule_dsl) use rerail_compiler::block;
        #[allow(unused_imports)]
        #[allow(unused_imports)]
        pub(in crate::code_rule_dsl) use rerail_rr_parser as rr_parser;
        #[allow(unused_imports)]
        use rerail_compiler::program::*;
        #[allow(unused_imports)]
        pub(in crate::code_rule_dsl) use rerail_compiler::program::{NAMED, Words, word};
        #[allow(unused_imports)]
        pub(in crate::code_rule_dsl) use rerail_compiler::Error;
    }
    #[allow(unused_imports)]
    #[cfg(any(test, feature = "test-hooks"))]
    use crate::code_rule_dsl::tape::TapeDoc;
    #[allow(unused_imports)]
    #[cfg(any(test, feature = "test-hooks"))]
    use ast_grep_core::MatchStrictness;
    #[allow(unused_imports)]
    #[cfg(any(test, feature = "test-hooks"))]
    use ast_grep_core::matcher::{Pattern, PatternNode};
    #[allow(unused_imports)]
    #[cfg(any(test, feature = "test-hooks"))]
    use regex::Regex;
    #[allow(unused_imports)]
    #[cfg(any(test, feature = "test-hooks"))]
    use std::collections::{BTreeMap, BTreeSet};
    #[allow(unused_imports)]
    #[cfg(any(test, feature = "test-hooks"))]
    use std::ops::Range;
    #[allow(unused_imports)]
    #[cfg(any(test, feature = "test-hooks"))]
    use crate::code_rule_dsl::search_index::persist_search_index_batch;
}
// Typing's rounds run over T's facts in tests only until Phase 10 wires them.
#[path = "typing"]
mod typing {
    pub use rerail_logic::code_rule_dsl::typing::*;
    #[path = "file_local"]
    pub mod file_local {
        pub use rerail_logic::code_rule_dsl::typing::file_local::*;
        #[allow(unused_imports)]
        use crate::content_store::{StoreError, StoreResult};
        #[allow(unused_imports)]
        pub(crate) use crate::code_rule_dsl::facts::nearest::{Binding, NONE, Trees};
        #[allow(unused_imports)]
        pub(crate) use crate::code_rule_dsl::typing::file_local::split_memo::forget as forget_splits;
        #[allow(unused_imports)]
        pub(crate) use crate::code_rule_dsl::logic::front::document_column;
        #[allow(unused_imports)]
        use std::collections::BTreeMap;
    }
    #[cfg(test)]
    pub mod oracle_tests;
}
#[path = "value"]
mod value {
    pub use rerail_w::code_rule_dsl::value::*;
    pub use rerail_lang_ecma::code_rule_dsl::value::ecma;
    pub use rerail_lang_ecma::code_rule_dsl::value::key;
    pub use rerail_lang_php::code_rule_dsl::value::php;
    pub use rerail_lang_php::code_rule_dsl::value::php_compare;
    pub use rerail_lang_php::code_rule_dsl::value::php_structure;
    pub use rerail_lang_python::code_rule_dsl::value::python;
    pub use rerail_lang_python::code_rule_dsl::value::python_ast;
    pub use rerail_lang_python::code_rule_dsl::value::python_structure;
    pub use rerail_lang_rust::code_rule_dsl::value::rust;
    pub use rerail_lang_sql::code_rule_dsl::value::sql;
    #[cfg(test)]
    pub mod tests;
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
    use crate::code_rule_dsl::language::adapter::pack as language_pack;
    #[allow(unused_imports)]
    pub(crate) use crate::code_rule_dsl::value::text_regex::MAX_STRING_BYTES;
}
pub use rerail_w::code_rule_dsl::violations;
pub use rerail_lang_ecma::code_rule_dsl::vue;
// Wired to the logic planner and the push path in a later slice; until then only tests call it.
#[allow(dead_code)]
mod waves;
#[path = "workspace"]
pub(crate) mod workspace {
    pub use rerail_w::code_rule_dsl::workspace::*;
    #[cfg(test)]
    pub mod context_tests;
    #[cfg(test)]
    pub mod crossfile_tests;
    #[path = "ingest"]
    pub mod ingest {
        pub use rerail_w::code_rule_dsl::workspace::ingest::*;
        pub use rerail_lang_ecma::code_rule_dsl::workspace::ingest::ecma;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::workspace::crossfile::{CrossFileScheduler};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::workspace::documents;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::workspace::{FileState, WorkspaceState};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::eval::{file_language, grammar_matcher};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::json::{invalid, json_error};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::GrammarDialect;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::adapter::{package_manifests};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::model::{
            ClauseKind, CompiledClause, CompiledCodeRule, FileInput, Query, package_marker_directory,
            python_module_in,
        };
        #[allow(unused_imports)]
        use crate::code_rule_dsl::module_graph;
        #[allow(unused_imports)]
        use crate::content_store::StoreError;
        #[allow(unused_imports)]
        use napi::Result;
        #[allow(unused_imports)]
        use serde::Deserialize;
        #[allow(unused_imports)]
        use serde_json::Value;
        #[allow(unused_imports)]
        use sha2::{Digest, Sha256};
        #[allow(unused_imports)]
        use std::collections::{BTreeMap, BTreeSet, HashSet};
        #[allow(unused_imports)]
        use std::sync::Arc;
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use std::sync::atomic::Ordering;
    }
    #[cfg(test)]
    pub mod placement_tests;
    #[cfg(test)]
    pub mod storage_tests;
    #[cfg(test)]
    pub mod tests;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::json::invalid;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::model::{CompiledClause, CompiledCodeRule, FileInput};
    #[allow(unused_imports)]
    use crate::code_rule_dsl::scope::ScopeTable;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::tape;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::violations::Witness;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::language::GrammarDialect;
    #[allow(unused_imports)]
    use crate::content_store::StoredFile;
    #[allow(unused_imports)]
    use napi::Result;
    #[allow(unused_imports)]
    #[cfg(any(test, feature = "test-hooks"))]
    use std::cell::{Cell, RefCell};
    #[allow(unused_imports)]
    use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
    #[allow(unused_imports)]
    use std::sync::atomic::{AtomicBool, Ordering};
    #[allow(unused_imports)]
    use std::sync::{Arc, OnceLock};
    #[allow(unused_imports)]
    use std::time::{Duration, Instant};
    #[allow(unused_imports)]
    use crate::code_rule_dsl::workspace::storage::StorageContext;
}

#[cfg(any(test, feature = "test-hooks"))]
pub use rerail_lang::code_rule_dsl::fixtures;
#[cfg(any(test, feature = "test-hooks"))]
pub(crate) use fixtures::{fixtures_dir, oracle};

use compile::{compile_document, parse_capabilities, parse_document};
use json::{canonicalize_value, hex_digest, invalid, json_error};
use model::CompiledCodeRule;
use napi::Result;
use napi_derive::napi;
use serde_json::Value;
use std::sync::Arc;

#[napi]
pub struct CodeRuleDslRuntime;

#[napi]
impl CodeRuleDslRuntime {
    /// Canonical front-end facts for one complete demand batch, including invalid syntax.
    /// This only parses source; it never imports or executes a source module.
    #[napi(catch_unwind)]
    pub fn python_ast_batch(documents_json: String) -> Result<String> {
        let documents: Vec<value::python_ast::Document> =
            serde_json::from_str(&documents_json).map_err(json_error)?;
        let response =
            value::python_ast::batch_with_metrics(&documents).map_err(|error| invalid(error.0))?;
        serde_json::to_string(&response).map_err(json_error)
    }

    #[napi(catch_unwind)]
    pub fn compile(
        source: String,
        format: String,
        capabilities_json: String,
    ) -> Result<CompiledCodeRuleDsl> {
        let capabilities = parse_capabilities(&capabilities_json)?;
        let document = parse_document(&source, &format)?;
        let canonical_value = canonicalize_value(document);
        let canonical = serde_json::to_string(&canonical_value).map_err(json_error)?;
        let digest = format!("sha256:{}", hex_digest(canonical.as_bytes()));
        let (id, rule_mode, scope, clauses, required, report_count_gt) =
            compile_document(&canonical_value, &capabilities)?;
        Ok(CompiledCodeRuleDsl {
            inner: Arc::new(CompiledCodeRule {
                id,
                report_count_gt,
                rule_mode,
                canonical,
                digest,
                required_capabilities: serde_json::to_string(&required).map_err(json_error)?,
                clauses,
                scope,
                capabilities,
            }),
        })
    }
}

/// The module a file's `module` carries, from the package markers on disk, for the file at
/// `absolute` whose dialect's label is `label`: a dotted string, a list of names when one keeps
/// a dot, or null outside a package and for a family without modules (`Adapter::module_path`).
/// Hosts and the native repository ingestion paths share this one derivation.
#[napi(catch_unwind)]
pub fn module_path(label: String, absolute: String) -> Option<napi::Either<String, Vec<String>>> {
    match language::adapter::module_path(&label, std::path::Path::new(&absolute))? {
        Value::String(module) => Some(napi::Either::A(module)),
        Value::Array(names) => Some(napi::Either::B(
            names
                .into_iter()
                .filter_map(|name| name.as_str().map(str::to_owned))
                .collect(),
        )),
        _ => None,
    }
}

/// Whether files of the dialect labelled `label` carry a `package` (`Adapter::PACKAGE_MANIFESTS`):
/// hosts resend such a file when a `package.json` edit changes its nearest manifest.
#[napi]
pub fn reads_package_manifests(label: String) -> bool {
    language::adapter::package_manifests(&label)
}

/// A file's `package` fact: what the engine reads of its nearest `package.json`.
#[napi(object)]
pub struct PackageFacts {
    /// The manifest's directory, relative to the workspace root (`""` at the root).
    pub folder: String,
    /// `"module"`, `"commonjs"`, or `"unset"` for a manifest without a valid `type`.
    #[napi(js_name = "type")]
    pub package_type: String,
    /// The `bin` targets, relative to `folder`.
    pub bin: Vec<String>,
    /// The configured Node range (`engines.node`, else `devEngines.runtime`); none without one.
    pub node_version: Option<String>,
    /// Every dependency name, of any kind.
    pub dependencies: Vec<String>,
    /// The package's `tsconfig.json` (else `jsconfig.json`) `compilerOptions.jsxImportSource`.
    pub jsx_import_source: Option<String>,
    /// `true` where the manifest's `private` is `true`; none otherwise.
    pub private: Option<bool>,
    /// Whether npm publishes the file, as eslint-plugin-n's `getNpmignore` reads the manifest's
    /// `files` and `main` and its folder's `.npmignore` or `.gitignore`: the file's own fact,
    /// which `package_facts` sends and `nearest_package` does not.
    pub published: Option<bool>,
}

impl PackageFacts {
    fn of(manifest: &package_manifest::Manifest) -> Self {
        Self {
            folder: manifest.folder().to_owned(),
            package_type: manifest.package_type().as_str().to_owned(),
            bin: manifest.bin().map(str::to_owned).collect(),
            node_version: manifest.node_version().map(str::to_owned),
            dependencies: manifest.dependencies().map(str::to_owned).collect(),
            jsx_import_source: manifest.jsx_import_source().map(str::to_owned),
            private: manifest.is_private().then_some(true),
            published: None,
        }
    }
}

/// The `package` a file carries, for the file at `absolute` whose dialect's label is `label`: its
/// nearest `package.json` at or above it, never above `root` (the workspace's), with whether npm
/// publishes the file, or null without one and for a family that reads none
/// (`Adapter::PACKAGE_MANIFESTS`). Hosts and the native repository ingestion paths share this one
/// derivation; a host reading many files reads them through one `PackageFactsReader`.
#[napi(catch_unwind)]
pub fn package_facts(label: String, absolute: String, root: String) -> Option<PackageFacts> {
    PackageFactsReader::new(root).facts(label, absolute)
}

/// The `package` facts of the files below one root, each manifest and its ignore files read once
/// (`package_manifest::Manifests`): one walk's reader, as files on disk do not change during it.
#[napi]
pub struct PackageFactsReader {
    manifests: parking_lot::Mutex<package_manifest::Manifests>,
}

#[napi]
impl PackageFactsReader {
    #[napi(constructor)]
    pub fn new(root: String) -> Self {
        Self {
            manifests: parking_lot::Mutex::new(package_manifest::Manifests::new(root)),
        }
    }

    /// `package_facts` for the file at `absolute` below the reader's root.
    #[napi(catch_unwind)]
    pub fn facts(&self, label: String, absolute: String) -> Option<PackageFacts> {
        if !language::adapter::package_manifests(&label) {
            return None;
        }
        let mut manifests = self.manifests.lock();
        let file = std::path::Path::new(&absolute);
        let manifest = manifests.of_file(file)?;
        let path = manifests.relative(file);
        Some(PackageFacts {
            published: manifest.publishes(&path),
            ..PackageFacts::of(&manifest)
        })
    }
}

/// The nearest `package.json` at or above `absolute` (a file or a folder's `package.json`),
/// never above `root`, whatever reads it: the package an edit of any file belongs to, so a host
/// can tell which applied sources it may change. Null without one.
#[napi(catch_unwind)]
// Only JavaScript calls it.
#[cfg_attr(any(test, feature = "test-hooks"), allow(dead_code))]
pub fn nearest_package(absolute: String, root: String) -> Option<PackageFacts> {
    let manifest =
        package_manifest::Manifests::new(root).of_file(std::path::Path::new(&absolute))?;
    Some(PackageFacts::of(&manifest))
}

/// The server graph of the package whose folder (relative to `root`) is `folder`, read on disk
/// off the JS thread: its files in Next's App Router server module graph, relative to `folder`
/// (`module_graph::on_disk`); null where the package has none (a source or folder cannot be read,
/// or it is past the walk's file or byte bounds), so the host sends its files no membership. A
/// host asks for it once per walk for a package whose `package` names `next` when a code rule
/// reads `file.package.serverGraph`, and sends each file of the package its own membership
/// (`package.serverGraph`). With `overlay_path`, that source reads as `overlay_source` (deleted
/// without one): an edit or a sample disk does not hold. The task is not cancellable; the walk's
/// bounds limit it.
#[napi]
// Only JavaScript calls it; tests drive `ServerGraphTask::members`.
#[cfg_attr(any(test, feature = "test-hooks"), allow(dead_code))]
pub fn server_graph_async(
    root: String,
    folder: String,
    overlay_path: Option<String>,
    overlay_source: Option<String>,
) -> napi::bindgen_prelude::AsyncTask<ServerGraphTask> {
    napi::bindgen_prelude::AsyncTask::new(ServerGraphTask::new(
        root,
        folder,
        overlay_path,
        overlay_source,
    ))
}

/// `server_graph_async`'s work.
pub struct ServerGraphTask {
    root: String,
    folder: String,
    overlay_path: Option<String>,
    overlay_source: Option<String>,
}

impl ServerGraphTask {
    pub(crate) fn new(
        root: String,
        folder: String,
        overlay_path: Option<String>,
        overlay_source: Option<String>,
    ) -> Self {
        Self {
            root,
            folder,
            overlay_path,
            overlay_source,
        }
    }

    /// The members, as the binding resolves them; `None` without a graph.
    pub(crate) fn members(&self) -> Option<Vec<String>> {
        let root = std::path::Path::new(&self.root);
        let overlay = self
            .overlay_path
            .as_deref()
            .map(|path| (path, self.overlay_source.as_deref()));
        module_graph::on_disk(
            root,
            &self.folder,
            &module_graph::config_on_disk(root, &self.folder),
            overlay,
        )
        .map(|members| members.into_iter().collect())
    }
}

impl napi::Task for ServerGraphTask {
    type Output = Option<Vec<String>>;
    type JsValue = Option<Vec<String>>;

    fn compute(&mut self) -> Result<Option<Vec<String>>> {
        crate::panic_boundary::contain_panic(std::any::type_name::<Self>(), || Ok(self.members()))
    }

    fn resolve(
        &mut self,
        _env: napi::Env,
        output: Option<Vec<String>>,
    ) -> Result<Option<Vec<String>>> {
        Ok(output)
    }
}

/// The project configuration files the package whose folder (relative to `root`) is `folder`
/// reads on disk (`module_graph::Config`): its `tsconfig.json` (else `jsconfig.json`), then the
/// `extends` bases followed, relative to `root`. Conformance forwards them with a corpus, as it
/// forwards its `package.json` files.
#[napi(catch_unwind)]
pub fn module_config_files(root: String, folder: String) -> Vec<String> {
    module_graph::config_on_disk(std::path::Path::new(&root), &folder)
        .files()
        .to_vec()
}

pub use rule_api::CompiledCodeRuleDsl;
#[cfg(any(test, feature = "test-hooks"))]
pub(crate) use rule_api::CodeRuleDslEditor;



/// The engine's language packs, into the core's registry (`language::adapter::pack`). Called as the library
/// loads, before any run: the core asks a family's questions of the pack registered here.
pub(crate) fn install_languages() {
    use language::Family;
    use language::adapter::pack::{Pack, pack, register};
    use language::adapter::{blade::Blade, ecma::Ecma, json::Json, php::Php, python::Python, rr::Rr, rust::Rust, sql::Sql, vue::Vue};
    register(Family::Python, &Pack::<Python>::NEW);
    language::register_base(Family::Python, &Pack::<Python>::NEW);
    register(Family::Ecma, &Pack::<Ecma>::NEW);
    language::register_base(Family::Ecma, &Pack::<Ecma>::NEW);
    register(Family::Php, &Pack::<Php>::NEW);
    language::register_base(Family::Php, &Pack::<Php>::NEW);
    register(Family::Blade, &Pack::<Blade>::NEW);
    language::register_base(Family::Blade, &Pack::<Blade>::NEW);
    register(Family::Rust, &Pack::<Rust>::NEW);
    language::register_base(Family::Rust, &Pack::<Rust>::NEW);
    register(Family::Sql, &Pack::<Sql>::NEW);
    language::register_base(Family::Sql, &Pack::<Sql>::NEW);
    register(Family::Vue, &Pack::<Vue>::NEW);
    language::register_base(Family::Vue, &Pack::<Vue>::NEW);
    register(Family::Rr, &Pack::<Rr>::NEW);
    language::register_base(Family::Rr, &Pack::<Rr>::NEW);
    register(Family::Json, &Pack::<Json>::NEW);
    language::register_base(Family::Json, &Pack::<Json>::NEW);
    // The linters' suppressions, in the order `suppress` is parsed and checked in.
    {
        use suppress::{eslint, noqa, nosec, oxlint, biome, phpstan, psalm, phpcs, clippy, sqlfluff, kind};
        suppress::install(vec![
            kind::<eslint::Suppressions>(),
            kind::<noqa::Suppressions>(),
            kind::<nosec::Suppressions>(),
            kind::<oxlint::Suppressions>(),
            kind::<biome::Suppressions>(),
            kind::<phpstan::Suppressions>(),
            kind::<psalm::Suppressions>(),
            kind::<phpcs::Suppressions>(),
            kind::<clippy::Suppressions>(),
            kind::<sqlfluff::Suppressions>(),
        ]);
    }
    // The embedders, by `Via` (`embed::Embedder`), and the program scopes an embedding gives a meaning of its own.
    embed::install_embedders(vec![
        #[cfg(feature = "lang-blade")]
        (language::Via::CompiledView, &embed::compiled_view::CompiledView),
        #[cfg(feature = "lang-vue")]
        (language::Via::ScriptBlock, &embed::script_block::ScriptBlock),
        #[cfg(any(test, feature = "test-hooks"))]
        (language::Via::TestScript, &embed::TestScript),
    ]);
    scope::install_program_scopes(vec![
        #[cfg(feature = "lang-blade")]
        (language::Via::CompiledView, embed::compiled_view::scope),
    ]);
    // What reads W's values off a file's tape for facts' `value` relations.
    facts::core::install_value_reader(&matcher::Values);
    // The families' producers of a batch's relations: Python's f-string trees, CPython's syntax, string annotations.
    facts::vtab::syntax::install_producers(vec![
        &facts::vtab::syntax::fstring::Producer,
        &facts::vtab::syntax::cpython::Producer,
        &facts::vtab::syntax::annotation::Producer,
    ]);
    // The families' lazy relations over a batch (Python's `py.*`).
    facts::vtab::syntax::install_families(vec![&facts::vtab::syntax::py_families::Families]);
    // The families' tables over a bound `node` relation (Python's semantic producers).
    facts::vtab::install_node_tables(vec![facts::vtab::python::register]);
    // The semantic facts a program's native steps demand (Python's).
    facts::vtab::syntax::install_semantic_producer(&facts::vtab::syntax::python_demand::Semantic);
    // What each family reads of its own core.rr facts.
    {
        use crate::code_rule_dsl::facts::vtab::syntax::core_rr::{self, Family};
        use crate::code_rule_dsl::facts::vtab::syntax::{ecma_core, python_core, rr_core};
        core_rr::register_dialect_facts(Family::Python, &python_core::PythonFacts);
        core_rr::register_dialect_facts(Family::Ecma, &ecma_core::EcmaFacts);
        core_rr::register_dialect_facts(Family::Rr, &rr_core::RrFacts);
    }
    // A workspace's project context of ECMAScript's files (manifests, project configurations, server graphs).
    workspace::ingest::install_project(|| Box::new(workspace::ingest::ecma::EcmaProject::default()));
    // A snapshot's context of ECMAScript's files (its manifests).
    eval::install_snapshot_context(eval::ecma::derive);
    // The numbers `toPrecision` reads (ECMAScript's).
    matcher::install_number_text(&value::ecma::Numbers);
    // `@to_precision`'s formatting (ECMAScript's), for the string functions below the languages.
    logic::strings::functions::install_to_precision(value::ecma::to_precision);
    // The canonical AST stage's producer (Python's CPython `ast`).
    residency::install_ast_producer(&residency::admission::relational::python_ast::PythonAst);
    // What ECMAScript's import targets resolve among (`resolution`).
    resolution::install(import_target::held::RESOLUTIONS);
    // A repository's context as its index reads it (ECMAScript's `package.json` files).
    workspace::ingest::install_repository_context(workspace::ingest::ecma::repository_context);
    // The language terms (ECMAScript's `importTarget`).
    matcher::install_terms(vec![compile::import_target::TERM]);
    // Each format style's preparer (`formatArgs`): Python's, PHP's and Rust's.
    {
        use matcher::format_args::{Style, printf, python, rust, scanf};
        matcher::format_args::install_format_preparers(vec![
            (Style::Brace, &python::BRACE),
            (Style::Percent, &python::PERCENT),
            (Style::Printf, &printf::Printf),
            (Style::Scanf, &scanf::Scanf),
            (Style::Rust, &rust::Rust),
        ]);
    }
    // What compiles `resolves` over a flow (PHP's).
    compile::resolve::install_flow_resolves(compile::resolve::php::compile_php_resolves);
    // The containment relations' `nearest` solve (the file-local FIX's).
    facts::nearest::install_nearest(typing::file_local::nearest);
    // `core.rr`'s names, as the rule language reads them.
    facts::core::install_core_names(logic::rr::core_names);
    // What the evaluator keeps per connection, dropped with it.
    facts::vtab::install_forget(facts::vtab::Forget { scope: logic::native::forget_scope, engine: logic::native::forget_engine });
    // core.rr's functors whose meaning is W's or a language's.
    logic::strings::install_functors(functors::register_functors);
    // The run's executors, by binding.
    facts::core::install_executors(logic::lanes::executor_for);
    // A tape's prefilter rows, which the push path derives.
    facts::families::install_prefilter_image(|tape, doc, interrupted| prefilter::image(tape, doc, &interrupted));
    // An owned connection's demand module.
    waves::owner::database::install_connection_setup(demand::vtab::set_up_connection);
    // Python's reference graph, a program's cross-file relations.
    logic::plan::corpus::install_cross_file(logic::plan::corpus::references::make);
    // A snapshot's files, as an SQL session binds them.
    sql_executor::documents::install_snapshot_files(|json| {
        let (snapshot, _) = eval::parse_snapshot(json, false)?;
        Ok(snapshot.files.into_iter().map(|f| sql_executor::documents::File { path: f.path, language: f.language, source: f.source }).collect())
    });
    // The candidate compiler's audit of a compiled rule.
    let _ = rule_api::FRONTIER_AUDIT.set(|rule| prefilter::candidate_frontier_audit(rule).into());
    // What compiles and launches a span pattern's prefilter (the transducer's block programs).
    facts::span_prefilter::install_span_prefilter(transducer::packed::regex_block::compile_span, transducer::packed::regex_block::launch_spans);
    // What compiles a program's `ast` matchers into the transducer wave facts runs (decision 0106).
    facts::vtab::syntax::install_ast_compiler(&facts::vtab::syntax::ast_wave::Compiler);
    // The literal grammars `parse` applies, in the order messages list them (`literal::LiteralGrammar`).
    literal::install(vec![
        literal::percent::GRAMMAR,
        literal::identifier_words::GRAMMAR,
        literal::python_fstring::GRAMMAR,
        literal::brace::GRAMMAR,
        literal::annotation::GRAMMAR,
        literal::eslint_directive::GRAMMAR,
        literal::printf::GRAMMAR,
        literal::scanf::GRAMMAR,
        literal::phpdoc::GRAMMAR,
        literal::rust_format::GRAMMAR,
        #[cfg(test)]
        literal::CHARS,
    ]);
    // What a `package` file condition reads of a file without a manifest (ECMA's default Node range).
    language::set_no_manifest(&package_manifest::NoManifest);
    language::set_manifest_reader(package_manifest::read_wire);
    // The dialects their languages parse themselves (`tape::TreeParser`). A build without a language's
    // feature routes no file to it, so its parser is not linked.
    {
        let mut parsers: Vec<(language::GrammarDialect, &'static dyn tape::TreeParser)> = Vec::new();
        if cfg!(feature = "lang-blade") {
            parsers.push((language::GrammarDialect::Blade, &blade::tape::Forte));
        }
        if cfg!(feature = "lang-json") {
            parsers.push((language::GrammarDialect::Json, &transducer::packed::json_parser::tape::Bitlex));
        }
        tape::install_parsers(parsers);
    }
    // Every family the build has, registered (`pack` refuses one that is not).
    Family::ALL.into_iter().for_each(|family| _ = pack(family));
}

/// The engine's devices, into the logic's and the transducer's registries (`logic::device`,
/// `transducer::packed::device`). Called as the library loads, before any run.
pub(crate) fn install_devices() {
    #[cfg(target_os = "macos")]
    device_metal::register();
}
