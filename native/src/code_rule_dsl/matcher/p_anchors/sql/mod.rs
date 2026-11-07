//! File-backed positional FTS, following ast-index-spike. SQL selects candidates;
//! the existing matcher validates them and constructs captures.
#[path = "lower/audit.rs"]
mod audit;
pub(in crate::code_rule_dsl) use audit::candidate_frontier_audit;
mod candidates;
#[cfg(feature = "code-rule-profile")]
mod capture;
mod corpus;
mod execute;
pub(in crate::code_rule_dsl) use corpus::{Corpus, CorpusSlot};
mod functions;
mod graph;
#[path = "store/kinds.rs"]
mod kind_postings;
mod lower;
mod nodes;
mod plans;
mod postings;
pub(in crate::code_rule_dsl) use plans::{
    for_file as anchor_plans_for_file, plans as anchor_plans,
};
mod reader;
mod tree;
#[cfg(all(test, feature = "code-rule-profile"))]
mod replay;
mod session;
mod store;
mod values;

use super::AnchorPlan;
#[cfg(test)]
use super::Selection;
use crate::code_rule_dsl::{language, tape::TapeDoc};
#[cfg(test)]
use ast_grep_core::Node;
use ast_grep_core::{AstGrep, Doc};
#[cfg(test)]
use std::borrow::Cow;

pub(in crate::code_rule_dsl) struct FileAnchorIndex<'tree, D: Doc> {
    _root: &'tree AstGrep<D>,
    file: Option<std::sync::Arc<corpus::File>>,
    #[cfg(feature = "code-rule-profile")]
    nodes: usize,
    visited: usize,
}

impl<'tree> FileAnchorIndex<'tree, TapeDoc> {
    #[cfg(test)]
    pub(in crate::code_rule_dsl) fn build(
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

    pub(in crate::code_rule_dsl) fn build_at(
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
    pub(in crate::code_rule_dsl) fn visited_nodes(&self) -> usize {
        self.visited
    }
    #[cfg(any(test, feature = "code-rule-profile"))]
    pub(in crate::code_rule_dsl) fn complete(&self) -> bool {
        self.file.is_some()
    }
    #[cfg(test)]
    pub(in crate::code_rule_dsl) fn node(&self, pre: usize) -> Node<'tree, TapeDoc> {
        self._root
            .adopt(self._root.root().get_doc().node(pre as u32))
    }
    #[cfg(feature = "code-rule-profile")]
    pub(in crate::code_rule_dsl) fn estimated_bytes(&self) -> usize {
        self.file.as_ref().map_or(0, |file| file.bytes)
    }

    pub(in crate::code_rule_dsl) fn selected(
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

    #[cfg(test)]
    pub(in crate::code_rule_dsl) fn select(&self, plan: &AnchorPlan) -> Selection<'_> {
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

#[cfg(test)]
mod preparation_tests;
#[cfg(all(test, feature = "code-rule-profile"))]
mod retention_spikes;
#[cfg(test)]
mod lab;
#[cfg(test)]
mod tests;
