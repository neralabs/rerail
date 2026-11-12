//! File-backed positional FTS, following ast-index-spike. SQL selects candidates;
//! the existing matcher validates them and constructs captures.
#[path = "lower/audit.rs"]
pub mod audit;
pub use audit::candidate_frontier_audit;
pub mod candidates;
#[cfg(feature = "code-rule-profile")]
pub mod capture;
pub mod corpus;
pub mod execute;
pub use corpus::{Corpus, CorpusSlot};
pub mod functions;
pub mod graph;
#[path = "store/kinds.rs"]
pub mod kind_postings;
pub mod lower;
pub mod nodes;
pub mod plans;
pub mod postings;
pub use plans::{
    for_file as anchor_plans_for_file, plans as anchor_plans,
};
pub mod reader;
#[cfg(all(any(test, feature = "test-hooks"), feature = "code-rule-profile"))]
pub mod replay;
pub mod session;
#[path = "functions/source_batch.rs"]
pub mod source_batch;
// cratify: `source_batch_tests` is rerail-engine's (it mounts it)
pub mod statistics;
pub mod store;
pub mod values;

use super::AnchorPlan;
#[cfg(any(test, feature = "test-hooks"))]
use super::Selection;
use crate::code_rule_dsl::{language, tape::TapeDoc};
#[cfg(any(test, feature = "test-hooks"))]
use ast_grep_core::Node;
use ast_grep_core::{AstGrep, Doc};
#[cfg(any(test, feature = "test-hooks"))]
use std::borrow::Cow;

pub struct FileAnchorIndex<'tree, D: Doc> {
    _root: &'tree AstGrep<D>,
    file: Option<std::sync::Arc<corpus::File>>,
    #[cfg(feature = "code-rule-profile")]
    nodes: usize,
    visited: usize,
}

impl<'tree> FileAnchorIndex<'tree, TapeDoc> {
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn build(
        root: &'tree AstGrep<TapeDoc>,
        _plans: &[&AnchorPlan],
        max_nodes: usize,
        interrupted: impl FnMut() -> bool,
    ) -> Self {
        let corpus = std::sync::Arc::new(Corpus::default());
        corpus
            .put("input", root.root().get_doc(), &|| false)
            .unwrap();
        // Test callers may borrow plans, so preserve their pointer keys while lowering a batch.
        corpus
            .prepare_borrowed(_plans, root.root().get_doc())
            .unwrap();
        let _scope = CorpusSlot::install(corpus);
        Self::build_at(root, _plans, max_nodes, interrupted, "input")
    }

    pub fn build_at(
        root: &'tree AstGrep<TapeDoc>,
        _plans: &[&AnchorPlan],
        max_nodes: usize,
        interrupted: impl FnMut() -> bool,
        path: &str,
    ) -> Self {
        let doc = root.root().get_doc();
        let nodes = doc.transducer_len();
        let lang =
            u32::from(language::tape_tag(*doc.get_lang()).expect("registered tape language"));
        let interrupted = std::cell::RefCell::new(interrupted);
        let poll = || interrupted.borrow_mut()();
        let file = if nodes <= max_nodes && !poll() {
            CorpusSlot::current().prepared_file(path, lang, &doc.index_source_digest())
        } else {
            None
        };
        // Refusing an oversized build must still expose the exhausted node allowance.
        // A subsequent kind lookup must not turn incomplete traversal into a clean absence.
        let visited = if nodes > max_nodes {
            max_nodes.saturating_add(1)
        } else if file.is_some() {
            nodes
        } else {
            0
        };
        Self {
            _root: root,
            file,
            #[cfg(feature = "code-rule-profile")]
            nodes,
            visited,
        }
    }

    #[cfg(feature = "code-rule-profile")]
    pub(in crate::code_rule_dsl) fn len(&self) -> usize {
        self.nodes
    }
    pub fn visited_nodes(&self) -> usize {
        self.visited
    }
    #[cfg(any(any(test, feature = "test-hooks"), feature = "code-rule-profile"))]
    pub fn complete(&self) -> bool {
        self.file.is_some()
    }
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn node(&self, pre: usize) -> Node<'tree, TapeDoc> {
        self._root
            .adopt(self._root.root().get_doc().node(pre as u32))
    }
    #[cfg(feature = "code-rule-profile")]
    pub(in crate::code_rule_dsl) fn estimated_bytes(&self) -> usize {
        self.file.as_ref().map_or(0, |file| file.bytes)
    }

    pub fn selected(
        &self,
        plan: &std::sync::Arc<AnchorPlan>,
    ) -> Option<std::sync::Arc<[u32]>> {
        if crate::code_rule_dsl::budget::matcher_budget_exhausted() {
            return None;
        }
        let roots = self
            .file
            .as_ref()
            .and_then(|file| file.roots.get(&(std::sync::Arc::as_ptr(plan) as usize)));
        let Some(roots) = roots else {
            #[cfg(feature = "code-rule-profile")]
            crate::code_rule_dsl::profile::record_anchor_cache_miss();
            return None;
        };
        let roots = roots.ordinals();
        if crate::code_rule_dsl::budget::matcher_budget_exhausted() {
            return None;
        }
        roots
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn select(&self, plan: &AnchorPlan) -> Selection<'_> {
        let id = plan as *const AnchorPlan as usize;
        match self
            .file
            .as_ref()
            .and_then(|file| file.roots.get(&id))
            .and_then(|roots| roots.ordinals())
        {
            Some(roots) => Selection::Roots(Cow::Owned(
                roots.iter().map(|&root| root as usize).collect(),
            )),
            None => Selection::All,
        }
    }
}

// cratify: `preparation_tests` is rerail-engine's (it mounts it)
// cratify: `retention_spikes` is rerail-engine's (it mounts it)
// cratify: `tests` is rerail-engine's (it mounts it)

#[path = "corpus/cache.rs"]
pub mod cache;
