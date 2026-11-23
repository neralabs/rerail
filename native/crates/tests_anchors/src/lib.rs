//! The engine's tests of the anchors' SQL: the D and P anchors, their lowering, preparation, corpus planning, statistics and retention (docs/final-shape/proposals/crate-split/incremental-plan.md, step 7i).
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
    #[cfg_attr(test, path = "matcher")]
    mod matcher {
        pub use rerail_boot::code_rule_dsl::matcher::*;
        #[cfg(test)]
        #[path = "d_anchors"]
        pub mod d_anchors {
            pub use rerail_boot::code_rule_dsl::matcher::d_anchors::*;
            pub mod sql {
                pub use rerail_boot::code_rule_dsl::matcher::d_anchors::sql::*;
                // Its profiling spikes, which read `tests` (`document`).
                #[cfg(all(test, feature = "code-rule-profile"))]
                pub mod retention_spikes;
                // Its tests' file is beside it: no directory of its own here.
                #[path = "."]
                pub mod lower {
                    pub use rerail_boot::code_rule_dsl::matcher::d_anchors::sql::lower::*;
                    #[cfg(test)]
                    #[path = "lower_tests.rs"]
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
                // Its profiling spikes, which read `tests` (`document`).
                #[cfg(all(test, feature = "code-rule-profile"))]
                pub mod retention_spikes;
                // Its tests' file is beside it: no directory of its own here.
                #[path = "."]
                pub mod lower {
                    pub use rerail_boot::code_rule_dsl::matcher::p_anchors::sql::lower::*;
                    #[cfg(test)]
                    #[path = "lower_tests.rs"]
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
        #[allow(unused_imports)]
        use util::{ MAX_UTIL_STACK_BYTES, UTIL_DEPTH, UTIL_EXCEEDED, UTIL_STACK_BASE, UTIL_STEPS_LEFT, stack_address, };
        #[allow(unused_imports)]
        use util::{THREE_VALUED};
    }
    #[cfg(test)]
    #[path = "../../../../src/code_rule_dsl/test_support.rs"]
    mod test_support;
}

// The engine's allocator (`rerail_prof::heap::Sampled`, as `rerail-engine`'s `lib.rs` declares it): without it the
// test binary ran on the system allocator, which no build of the engine runs on.
#[cfg(not(any(windows, target_family = "wasm", feature = "system-allocator")))]
#[global_allocator]
static GLOBAL: rerail_prof::heap::Sampled = rerail_prof::heap::Sampled;
