//! The engine's tests of matching: the matcher, the workspace, findings, suppressions and violations (docs/final-shape/proposals/crate-split/incremental-plan.md, step 2).
//!
//! A test crate above boot: it mounts the test files in place, under the facades the engine hosted them through,
//! and reads every other module through boot. Its test binary is booted as it loads (`boot::install_for_tests`), as
//! the engine's is. Written by tools/cratify/test_crates.py, and kept by hand since.
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
    // Wired to the findings layer in a later slice; until then only tests call it.
    #[cfg_attr(test, path = "findings")]
    mod findings {
        pub use rerail_boot::code_rule_dsl::findings::*;
        #[cfg(test)]
        #[path = "tests"]
        pub mod async_reference {
            pub use rerail_boot::code_rule_dsl::findings::async_reference::*;
            // Its timings are read by the p-anchor-corpus tests alone.
            #[cfg(test)]
            #[cfg_attr(not(feature = "p-anchor-corpus"), allow(dead_code))]
            pub mod native_oracle;
            #[cfg(test)]
            pub mod s_native_oracle;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::findings::{
                contract::{Coverage, FindingKey, Findings},
                shape::generated_shape,
            };
            #[allow(unused_imports)]
            use crate::code_rule_dsl::facts::producers::python::{call_batch, python_facts::{self, Qualification}, python_structure::{ContextKind, NodeRef}};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::facts::vtab::{self, Modules, Relation, Source};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::{
                language::GrammarDialect,
                module_path::ModulePath,
                scope::{
                    Reading, ScopeTable,
                    options::{DEFAULT_PYTHON_MINOR, PythonOptions},
                },
                tape::TapeDoc,
            };
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use rusqlite::{Connection, StatementStatus};
            #[allow(unused_imports)]
            use std::{
                collections::{BTreeMap, BTreeSet},
                time::Instant,
            };
            #[cfg(test)]
                    pub mod rr_comparison;
            #[cfg(test)]
                    pub mod tests;
            #[cfg(test)]
            pub mod compiler_admission;
        }
    }
    #[cfg_attr(test, path = "matcher")]
    mod matcher {
        pub use rerail_boot::code_rule_dsl::matcher::*;
        pub mod value {
            pub use rerail_boot::code_rule_dsl::matcher::value::*;
            #[cfg(test)]
            #[path = "tests.rs"]
            pub mod tests;
            #[allow(unused_imports)]
            use super::resolve::{ ValueKey, binding_value, constant_value, followed_table, memoize_structure, memoize_value, memoized_structure, memoized_value, unshadowed_global, };
            #[allow(unused_imports)]
            use super::util::{UTIL_EXCEEDED, UTIL_STEPS_LEFT, UtilLimit};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::budget;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::{Family, GrammarDialect, kind_ids};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::literal;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::scope::options::{LanguageOptions, PythonOptions};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::value::structure::{Equality, Tok};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::value::text_regex::TextRegex;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::value::{BigInt, Context, Value};
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use num_traits::Zero;
            #[allow(unused_imports)]
            use std::cmp::Ordering;
            #[allow(unused_imports)]
            use std::collections::BTreeSet;
            #[allow(unused_imports)]
            use std::rc::Rc;
            #[allow(unused_imports)]
            use std::sync::Arc;
            #[allow(unused_imports)]
            use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::pack as language_pack;
        }
        pub mod embedded {
            pub use rerail_boot::code_rule_dsl::matcher::embedded::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use super::AstMatcher;
            #[allow(unused_imports)]
            use super::each::{self, HostRanges};
            #[allow(unused_imports)]
            use super::exported::{export, exported_span, exported_text, is_span_variable};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::embed::segments::holds;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::{GrammarDialect, Via};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::origin::Origin;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::tape::{TapeDoc, tape_doc};
            #[allow(unused_imports)]
            use ast_grep_core::meta_var::{MetaVarEnv, MetaVariable};
            #[allow(unused_imports)]
            use ast_grep_core::{AstGrep, Doc, Matcher, Node};
            #[allow(unused_imports)]
            use std::borrow::Cow;
            #[allow(unused_imports)]
            use std::cell::RefCell;
            #[allow(unused_imports)]
            use std::ops::Range;
            #[allow(unused_imports)]
            use std::sync::Arc;
        }
        pub mod util {
            pub use rerail_boot::code_rule_dsl::matcher::util::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use super::AstMatcher;
            #[allow(unused_imports)]
            use std::cell::Cell;
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, BTreeSet};
            #[allow(unused_imports)]
            use std::sync::atomic::{AtomicBool, Ordering};
            #[allow(unused_imports)]
            use std::sync::{Arc, Mutex, OnceLock, Weak};
        }
        pub mod transform {
            pub use rerail_boot::code_rule_dsl::matcher::transform::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use super::exported::exported_span;
            #[allow(unused_imports)]
            use super::value::Reading;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::origin::InFile;
            #[allow(unused_imports)]
            use ast_grep_core::meta_var::{MetaVarEnv, MetaVariable};
            #[allow(unused_imports)]
            use ast_grep_core::source::Content;
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use regex::Regex;
            #[allow(unused_imports)]
            use semantic::value_text;
            #[allow(unused_imports)]
            #[cfg(any(test, feature = "test-hooks"))]
            use std::cell::Cell;
            #[allow(unused_imports)]
            use std::collections::BTreeSet;
            #[allow(unused_imports)]
            use std::ops::Range;
        }
        pub mod relation {
            pub use rerail_boot::code_rule_dsl::matcher::relation::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use super::AstMatcher;
            #[allow(unused_imports)]
            use super::CaptureAnalysis;
            #[allow(unused_imports)]
            use super::RequiredText;
            #[allow(unused_imports)]
            use super::Semantic;
            #[allow(unused_imports)]
            use super::memo::Truth;
            #[allow(unused_imports)]
            use super::util::{THREE_VALUED, UNKNOWN};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::budget;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::tape::TapeDoc;
            #[allow(unused_imports)]
            use ast_grep_core::matcher::{Matcher, MatcherExt, PatternNode};
            #[allow(unused_imports)]
            use ast_grep_core::meta_var::{MetaVarEnv, MetaVariable};
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use std::borrow::Cow;
            #[allow(unused_imports)]
            use std::cell::Cell;
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, BTreeSet};
            #[allow(unused_imports)]
            use std::ops::ControlFlow;
            #[allow(unused_imports)]
            use std::sync::{Arc, OnceLock};
        }
        pub mod nth_child {
            pub use rerail_boot::code_rule_dsl::matcher::nth_child::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use super::AstMatcher;
            #[allow(unused_imports)]
            use super::CaptureAnalysis;
            #[allow(unused_imports)]
            use super::spend_util_step;
            #[allow(unused_imports)]
            use super::util::{THREE_VALUED, UNKNOWN};
            #[allow(unused_imports)]
            use ast_grep_core::Matcher;
            #[allow(unused_imports)]
            use ast_grep_core::meta_var::MetaVarEnv;
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use std::borrow::Cow;
        }
        pub mod memo {
            pub use rerail_boot::code_rule_dsl::matcher::memo::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use super::nth_child::Siblings;
            #[allow(unused_imports)]
            use super::relation::WalkFilter;
            #[allow(unused_imports)]
            use super::util::{THREE_VALUED, UNKNOWN, UtilCall, UtilRef};
            #[allow(unused_imports)]
            use super::{Comparison, EqClass};
            #[allow(unused_imports)]
            use super::{ clause_state_reads, clauses_begun, each, fill_steps, filling, read_clause_state, spend_util_steps, swap_util_peak, util_level, util_levels_fit, util_limit_hit, utility_steps_left, };
            #[allow(unused_imports)]
            use crate::code_rule_dsl::budget;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::tape::TapeDoc;
            #[allow(unused_imports)]
            use ast_grep_core::meta_var::{MetaVarEnv, MetaVariable};
            #[allow(unused_imports)]
            use ast_grep_core::{AstGrep as Root, Doc, Node};
            #[allow(unused_imports)]
            use std::any::TypeId;
            #[allow(unused_imports)]
            use std::borrow::Cow;
            #[allow(unused_imports)]
            use std::cell::RefCell;
            #[allow(unused_imports)]
            use std::ops::Range;
            #[allow(unused_imports)]
            use std::sync::Arc;
        }
        pub mod layout {
            pub use rerail_boot::code_rule_dsl::matcher::layout::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use super::each::binding_range;
            #[allow(unused_imports)]
            use super::spend_util_step;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::origin::InFile;
            #[allow(unused_imports)]
            use ast_grep_core::meta_var::MetaVarEnv;
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use std::borrow::Cow;
            #[allow(unused_imports)]
            use std::ops::Range;
        }
        pub mod each {
            pub use rerail_boot::code_rule_dsl::matcher::each::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use super::exported::exported_span;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::origin::Origin;
            #[allow(unused_imports)]
            use ast_grep_core::Doc;
            #[allow(unused_imports)]
            use ast_grep_core::meta_var::MetaVarEnv;
            #[allow(unused_imports)]
            use std::cell::{Cell, RefCell};
            #[allow(unused_imports)]
            use std::collections::HashSet;
            #[allow(unused_imports)]
            use std::ops::Range;
            #[allow(unused_imports)]
            use std::sync::Arc;
        }
        pub mod count {
            pub use rerail_boot::code_rule_dsl::matcher::count::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use super::AstMatcher;
            #[allow(unused_imports)]
            use super::relation::descend_until;
            #[allow(unused_imports)]
            use super::util::{THREE_VALUED, UNKNOWN, UTIL_EXCEEDED};
            #[allow(unused_imports)]
            use super::{StopBy, spend_util_step};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::budget;
            #[allow(unused_imports)]
            use ast_grep_core::matcher::Matcher;
            #[allow(unused_imports)]
            use ast_grep_core::meta_var::{MetaVarEnv, MetaVariable};
            #[allow(unused_imports)]
            use ast_grep_core::source::Content;
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use std::borrow::Cow;
            #[allow(unused_imports)]
            use std::cell::Cell;
            #[allow(unused_imports)]
            use std::ops::ControlFlow;
        }
        pub mod compare {
            pub use rerail_boot::code_rule_dsl::matcher::compare::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::tree::Tree;
            #[allow(unused_imports)]
            use super::collate::natural_compare;
            #[allow(unused_imports)]
            use super::exported::exported_text;
            #[allow(unused_imports)]
            use super::transform::Transform;
            #[allow(unused_imports)]
            use super::value::Reading;
            #[allow(unused_imports)]
            use ast_grep_core::meta_var::MetaVarEnv;
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use std::cmp::Ordering;
            #[allow(unused_imports)]
            use std::sync::Arc;
        }
        pub mod call {
            pub use rerail_boot::code_rule_dsl::matcher::call::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use super::{UtilCall, each, spend_util_step, unify_capture};
            #[allow(unused_imports)]
            use ast_grep_core::meta_var::MetaVarEnv;
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use std::collections::BTreeSet;
        }
        pub mod analysis {
            pub use rerail_boot::code_rule_dsl::matcher::analysis::*;
            pub mod required_text {
                pub use rerail_boot::code_rule_dsl::matcher::analysis::required_text::*;
                #[cfg(test)]
                            pub mod tests;
                #[allow(unused_imports)]
                use super::super::{AstMatcher, UtilRef};
                #[allow(unused_imports)]
                use ast_grep_core::MatchStrictness;
                #[allow(unused_imports)]
                use ast_grep_core::matcher::PatternNode;
                #[allow(unused_imports)]
                use regex_syntax::hir::literal::Extractor;
                #[allow(unused_imports)]
                use std::collections::{BTreeMap, BTreeSet};
            }
        }
        #[cfg_attr(test, path = "anchors")]
        pub mod anchors {
            pub use rerail_boot::code_rule_dsl::matcher::anchors::*;
            #[cfg(test)]
            pub mod tests;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::{AstMatcher, StopBy, UtilRef};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::budget;
            #[allow(unused_imports)]
            #[cfg(test)]
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
        #[cfg(test)]
        #[path = "d_anchors"]
        pub mod d_anchors {
            pub use rerail_boot::code_rule_dsl::matcher::d_anchors::*;
            pub mod sql {
                pub use rerail_boot::code_rule_dsl::matcher::d_anchors::sql::*;
                pub mod lower {
                    pub use rerail_boot::code_rule_dsl::matcher::d_anchors::sql::lower::*;
                    #[cfg(test)]
                    #[path = "../lower_tests.rs"]
                    pub mod tests;
                    #[allow(unused_imports)]
                    use super::super::{AnchorPlan, Condition, Direction, Required};
                    #[allow(unused_imports)]
                    use rusqlite::types::Value;
                    #[allow(unused_imports)]
                    use std::collections::{BTreeSet, HashMap};
                    #[allow(unused_imports)]
                    use decode::Property;
                    #[allow(unused_imports)]
                    use render::Definitions;
                }
                pub mod corpus {
                    pub use rerail_boot::code_rule_dsl::matcher::d_anchors::sql::corpus::*;
                    #[cfg(test)]
                                    pub mod owner_tests;
                    #[cfg(test)]
                                    pub mod planning_tests;
                    #[allow(unused_imports)]
                    use super::super::AnchorPlan;
                    #[allow(unused_imports)]
                    use super::{ candidates::{Builder, Roots}, store::Store, };
                    #[allow(unused_imports)]
                    use crate::code_rule_dsl::tape::TapeDoc;
                    #[allow(unused_imports)]
                    use ast_grep_core::Doc;
                    #[allow(unused_imports)]
                    use parking_lot::Mutex;
                    #[allow(unused_imports)]
                    use rusqlite::Result;
                    #[allow(unused_imports)]
                    use std::collections::{BTreeMap, HashMap, HashSet};
                    #[allow(unused_imports)]
                    use std::{cell::RefCell, sync::Arc};
                }
                pub mod statistics {
                    pub use rerail_boot::code_rule_dsl::matcher::d_anchors::sql::statistics::*;
                    #[cfg(test)]
                                    pub mod tests;
                    #[allow(unused_imports)]
                    use super::super::{AnchorPlan, Condition, Required, Requirement};
                    #[allow(unused_imports)]
                    use rusqlite::{Connection, Result, limits::Limit, params_from_iter, types::Value};
                    #[allow(unused_imports)]
                    use std::collections::{BTreeMap, BTreeSet, HashSet};
                    #[allow(unused_imports)]
                    use std::sync::Arc;
                }
                #[cfg(test)]
                            pub mod preparation_tests;
                #[cfg(test)]
                            pub mod tests;
                #[allow(unused_imports)]
                use super::AnchorPlan;
                #[allow(unused_imports)]
                #[cfg(any(test, feature = "test-hooks"))]
                use super::Selection;
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
                #[cfg(test)]
                #[path = "functions/source_batch_tests.rs"]
                pub mod source_batch_tests;
            }
            #[cfg(test)]
            pub mod tests;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::{AstMatcher, StopBy, UtilRef};
            #[allow(unused_imports)]
            #[cfg(test)]
            use crate::code_rule_dsl::budget;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::{Adapter, ValueRules};
            #[allow(unused_imports)]
            #[cfg(test)]
            use crate::code_rule_dsl::walks::{Walk, record_walk};
            #[allow(unused_imports)]
            #[cfg(test)]
            use crate::code_rule_dsl::language::GrammarDialect;
            #[allow(unused_imports)]
            use ast_grep_core::MatchStrictness;
            #[allow(unused_imports)]
            use ast_grep_core::matcher::{Matcher, PatternNode, kind_utils};
            #[allow(unused_imports)]
            #[cfg(test)]
            use ast_grep_core::source::SgNode;
            #[allow(unused_imports)]
            #[cfg(test)]
            use ast_grep_core::{AstGrep, Doc, Node};
            #[allow(unused_imports)]
            #[cfg(test)]
            use foldhash::{HashMap, HashMapExt, HashSet};
            #[allow(unused_imports)]
            use regex::Regex;
            #[allow(unused_imports)]
            #[cfg(test)]
            use std::borrow::Cow;
            #[allow(unused_imports)]
            #[cfg(test)]
            use std::cell::RefCell;
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, BTreeSet};
            #[allow(unused_imports)]
            #[cfg(test)]
            use std::num::NonZeroUsize;
            #[allow(unused_imports)]
            use std::sync::Arc;
            #[allow(unused_imports)]
            #[cfg(test)]
            use crate::code_rule_dsl::matcher::d_anchors::text::TextProbe;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::pack as language_pack;
        }
        #[cfg(test)]
        #[path = "p_anchors"]
        pub mod p_anchors {
            pub use rerail_boot::code_rule_dsl::matcher::p_anchors::*;
            #[path = "sql"]
            pub mod sql {
                pub use rerail_boot::code_rule_dsl::matcher::p_anchors::sql::*;
                pub mod lower {
                    pub use rerail_boot::code_rule_dsl::matcher::p_anchors::sql::lower::*;
                    #[cfg(test)]
                    #[path = "../lower_tests.rs"]
                    pub mod tests;
                    #[allow(unused_imports)]
                    use super::super::{AnchorPlan, Condition, Direction, Required};
                    #[allow(unused_imports)]
                    use rusqlite::types::Value;
                    #[allow(unused_imports)]
                    use std::collections::{BTreeSet, HashMap};
                    #[allow(unused_imports)]
                    use decode::Property;
                    #[allow(unused_imports)]
                    use render::Definitions;
                }
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
                #[cfg(test)]
                use crate::code_rule_dsl::matcher::p_anchors::Selection;
                #[allow(unused_imports)]
                use crate::code_rule_dsl::{language, tape::TapeDoc};
                #[allow(unused_imports)]
                #[cfg(test)]
                use ast_grep_core::Node;
                #[allow(unused_imports)]
                use ast_grep_core::{AstGrep, Doc};
                #[allow(unused_imports)]
                #[cfg(test)]
                use std::borrow::Cow;
                #[cfg(test)]
                            pub mod tests;
                #[cfg(test)]
                            pub mod preparation_tests;
            }
            #[cfg(test)]
            pub mod tests;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::matcher::{AstMatcher, StopBy, UtilRef};
            #[allow(unused_imports)]
            #[cfg(test)]
            use crate::code_rule_dsl::budget;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::{Adapter, ValueRules};
            #[allow(unused_imports)]
            #[cfg(test)]
            use crate::code_rule_dsl::walks::{Walk, record_walk};
            #[allow(unused_imports)]
            #[cfg(test)]
            use crate::code_rule_dsl::language::GrammarDialect;
            #[allow(unused_imports)]
            use ast_grep_core::MatchStrictness;
            #[allow(unused_imports)]
            use ast_grep_core::matcher::{Matcher, PatternNode, kind_utils};
            #[allow(unused_imports)]
            #[cfg(test)]
            use ast_grep_core::source::SgNode;
            #[allow(unused_imports)]
            #[cfg(test)]
            use ast_grep_core::{AstGrep, Doc, Node};
            #[allow(unused_imports)]
            #[cfg(test)]
            use foldhash::{HashMap, HashMapExt, HashSet};
            #[allow(unused_imports)]
            use regex::Regex;
            #[allow(unused_imports)]
            #[cfg(test)]
            use std::borrow::Cow;
            #[allow(unused_imports)]
            #[cfg(test)]
            use std::cell::RefCell;
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, BTreeSet};
            #[allow(unused_imports)]
            #[cfg(test)]
            use std::num::NonZeroUsize;
            #[allow(unused_imports)]
            use std::sync::Arc;
            #[allow(unused_imports)]
            #[cfg(test)]
            use crate::code_rule_dsl::matcher::p_anchors::text::TextProbe;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::pack as language_pack;
        }
        #[cfg(test)]
        pub mod python_facts {
            pub use rerail_boot::code_rule_dsl::matcher::python_facts::*;
            #[cfg(test)]
            #[path = "tests.rs"]
            pub mod tests;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::{ identifier::python_identifier, language::{ Family, GrammarDialect, adapter::{python::Python, ScopeRules}, }, module_path::ModulePath, scope::{ Binding, Form, Import, Lookup, Origin, ScopeTable, options::{PythonOptions, python_builtins}, }, value::NodeRef, };
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, HashSet};
        }
        #[allow(unused_imports)]
        #[cfg(test)]
        pub(in crate::code_rule_dsl) use crate::code_rule_dsl::matcher::p_anchors::sql::{Corpus, CorpusSlot};
        #[allow(unused_imports)]
        #[cfg(all(test, feature = "p-anchor-corpus"))]
        pub(in crate::code_rule_dsl) use crate::code_rule_dsl::matcher::p_anchors::sql::{anchor_plans, anchor_plans_for_file};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::budget;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::{GrammarDialect, SourceType};
        #[allow(unused_imports)]
        pub(super) use crate::code_rule_dsl::matcher::analysis::RequiredText;
        #[allow(unused_imports)]
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
        #[cfg(test)]
        pub(crate) use crate::code_rule_dsl::matcher::nth_child::{named_sibling_visits, reset_named_sibling_visits};
        #[allow(unused_imports)]
        #[cfg(test)]
        pub(crate) use crate::code_rule_dsl::matcher::parse::parse_fact_tree;
        #[allow(unused_imports)]
        pub(crate) use crate::code_rule_dsl::matcher::parse::{HostTest, ParseTest};
        #[allow(unused_imports)]
        use regex::Regex;
        #[allow(unused_imports)]
        pub(super) use crate::code_rule_dsl::matcher::relation::{FieldRelation, Related, StopBy, stop_rule};
        #[allow(unused_imports)]
        #[cfg(test)]
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
        #[cfg(test)]
        pub(super) use crate::code_rule_dsl::matcher::resolve::{JSX_RUNTIME_DERIVATIONS, SOURCE_TYPE_DERIVATIONS, TABLE_BUILDS};
        #[allow(unused_imports)]
        pub(super) use crate::code_rule_dsl::matcher::run::{Stopped, enumerate, node_at, run_clause};
        #[allow(unused_imports)]
        #[cfg(test)]
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
        #[cfg(test)]
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
        #[cfg(test)]
        #[path = "tests/mod.rs"]
        pub mod tests;
        #[allow(unused_imports)]
        use util::{ MAX_UTIL_STACK_BYTES, UTIL_DEPTH, UTIL_EXCEEDED, UTIL_STACK_BASE, UTIL_STEPS_LEFT, stack_address, };
        #[allow(unused_imports)]
        use util::{THREE_VALUED};
    }
    #[cfg_attr(test, path = "suppress")]
    mod suppress {
        pub use rerail_boot::code_rule_dsl::suppress::*;
        pub mod rerail {
            pub use rerail_boot::code_rule_dsl::suppress::rerail::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::embed::segments::FileRanges;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::{Family, GrammarDialect};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::origin::InFile;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::tape::{TapeDoc, tape_doc};
            #[allow(unused_imports)]
            use ast_grep_core::{AstGrep, Doc, Node};
            #[allow(unused_imports)]
            use std::ops::Range;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::language::adapter::pack as language_pack;
        }
        pub mod biome {
            pub use rerail_boot::code_rule_dsl::suppress::biome::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use super::names;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::{object, reject_unknown};
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use serde_json::Value;
            #[allow(unused_imports)]
            use std::ops::Range;
        }
        #[allow(unused_imports)] // boot's mount: only boot's files read it
        pub use rerail_lang_rust::code_rule_dsl::suppress::clippy;
        #[cfg_attr(not(test), allow(unused_imports))]
        pub use rerail_lang_ecma::code_rule_dsl::suppress::eslint;
        #[cfg_attr(not(test), allow(unused_imports))]
        pub use rerail_lang_python::code_rule_dsl::suppress::noqa;
        pub mod nosec {
            pub use rerail_boot::code_rule_dsl::suppress::nosec::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use super::names;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::{object, reject_unknown};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::lines::LineTable;
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use regex::Regex;
            #[allow(unused_imports)]
            use serde_json::Value;
            #[allow(unused_imports)]
            use std::collections::BTreeMap;
            #[allow(unused_imports)]
            use std::sync::LazyLock;
        }
        pub mod oxlint {
            pub use rerail_boot::code_rule_dsl::suppress::oxlint::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use super::names;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::{object, reject_unknown};
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use serde_json::Value;
            #[allow(unused_imports)]
            use std::collections::HashMap;
            #[allow(unused_imports)]
            use std::ops::Range;
        }
        #[cfg_attr(not(test), allow(unused_imports))]
        pub use rerail_lang_php::code_rule_dsl::suppress::phpcs;
        pub mod phpstan {
            pub use rerail_boot::code_rule_dsl::suppress::phpstan::*;
            #[cfg(test)]
            #[path = "tests.rs"]
            pub mod tests;
            #[allow(unused_imports)]
            use super::psalm::{doc_comment, pcre_space};
            #[allow(unused_imports)]
            use super::{names, php_comment_end};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::{object, reject_unknown};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::lines::LineTable;
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use serde_json::Value;
            #[allow(unused_imports)]
            use std::cell::RefCell;
            #[allow(unused_imports)]
            use std::collections::HashMap;
            #[allow(unused_imports)]
            use std::ops::Range;
        }
        pub use rerail_lang_php::code_rule_dsl::suppress::psalm;
        #[allow(unused_imports)] // boot's mount: only boot's files read it
        pub use rerail_lang_sql::code_rule_dsl::suppress::sqlfluff;
        pub use rerail_lang_ecma::code_rule_dsl::suppress::vue;
        #[allow(unused_imports)]
        use ast_grep_core::{Doc, Node};
        #[allow(unused_imports)]
        pub(crate) use crate::code_rule_dsl::suppress::rerail::RerailIgnores;
        #[allow(unused_imports)]
        pub(crate) use crate::code_rule_dsl::suppress::kinds::*;
        #[cfg(test)]
            pub mod eslint_tests;
        #[cfg(test)]
            pub mod tests;
        #[cfg(test)]
            pub mod ruff;
    }
    #[cfg(test)]
    mod test_support;
    pub mod violations {
        pub use rerail_boot::code_rule_dsl::violations::*;
        #[cfg(test)]
            pub mod each_limit_tests;
        #[cfg(test)]
            pub mod tests;
        #[allow(unused_imports)]
        use super::json::invalid;
        #[allow(unused_imports)]
        use super::matcher::{ AstMatcher, Capture, CaptureAnalysis, GrammarMatcher, UtilTable, UtilitySummary, binding_range, collect_variables, };
        #[allow(unused_imports)]
        use super::model::{ Anchor, ClauseKind, FIX_TEXT_PREFIX, FixPlan, Query, RegexSource, Template, ViolationSpec, };
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use crate::code_rule_dsl::walks::{Walk, record_walk};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::{Family, GrammarDialect};
        #[allow(unused_imports)]
        use crate::code_rule_dsl::origin::InFile;
        #[allow(unused_imports)]
        use ast_grep_core::matcher::NodeMatch;
        #[allow(unused_imports)]
        use ast_grep_core::meta_var::{MetaVarEnv, MetaVariable};
        #[allow(unused_imports)]
        use ast_grep_core::replacer::{Content, Replacer, TemplateFix};
        #[allow(unused_imports)]
        use ast_grep_core::{Doc, Node};
        #[allow(unused_imports)]
        use napi::Result;
        #[allow(unused_imports)]
        use regex::Regex;
        #[allow(unused_imports)]
        use serde::{Deserialize, Serialize};
        #[allow(unused_imports)]
        use serde_json::{Value, json};
        #[allow(unused_imports)]
        use std::borrow::Cow;
        #[allow(unused_imports)]
        #[cfg(any(test, feature = "test-hooks"))]
        use std::cell::Cell;
        #[allow(unused_imports)]
        use std::collections::{BTreeMap, BTreeSet};
        #[allow(unused_imports)]
        use std::ops::Range;
        #[allow(unused_imports)]
        use std::sync::Arc;
        #[allow(unused_imports)]
        use crate::code_rule_dsl::language::adapter::pack as language_pack;
    }
    #[cfg_attr(test, path = "workspace")]
    pub(crate) mod workspace {
        pub use rerail_boot::code_rule_dsl::workspace::*;
        pub mod results {
            pub use rerail_boot::code_rule_dsl::workspace::results::*;
            #[cfg(test)]
                    pub mod tests;
            #[allow(unused_imports)]
            use super::crossfile::CrossFileScheduler;
            #[allow(unused_imports)]
            use super::ingest::{candidate_dispatch, make_file};
            #[allow(unused_imports)]
            use super::persisted::{candidate_catalog, restore_complete_results};
            #[allow(unused_imports)]
            use super::{CandidateSets, ClauseId, FileState, WorkspaceState};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::eval::{ coverage, evaluation_json, failure, file_language, result_order, violation_from_witness, };
            #[allow(unused_imports)]
            use crate::code_rule_dsl::json::{invalid, json_error};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::model::{ClauseKind, CompiledCodeRule, Evaluation, Metrics, Query};
            #[allow(unused_imports)]
            #[cfg(feature = "code-rule-profile")]
            use crate::code_rule_dsl::profile;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::soundness::{SYNTAX_ERRORS, UNREAD};
            #[allow(unused_imports)]
            use napi::Result;
            #[allow(unused_imports)]
            use rayon::prelude::*;
            #[allow(unused_imports)]
            use serde_json::{Value, json};
            #[allow(unused_imports)]
            #[cfg(any(test, feature = "test-hooks"))]
            use std::cell::Cell;
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, BTreeSet};
            #[allow(unused_imports)]
            use std::hash::{DefaultHasher, Hash, Hasher};
            #[allow(unused_imports)]
            use std::sync::Arc;
            #[allow(unused_imports)]
            use std::time::Duration;
        }
        #[cfg(test)]
        pub mod crossfile_tests;
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
        #[cfg(test)]
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
}
