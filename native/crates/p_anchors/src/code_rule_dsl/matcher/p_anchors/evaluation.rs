//! P's anchor corpus in a rule's stateless evaluation (the `p-anchor-corpus` path, open item O20): the code rules'
//! evaluator (rerail-rules-base, below this crate) reads it through `eval::ANCHOR_CORPUS`, which [`register`] fills.
//! Moved from `eval` (`prepare_candidates`) and `eval::query` (`anchor_roots`, `selected_roots`) unchanged.

use crate::code_rule_dsl::eval;
use crate::code_rule_dsl::matcher::GrammarMatcher;
use crate::code_rule_dsl::model::{CompiledCodeRule, Snapshot};
use std::collections::BTreeMap;
use std::sync::Arc;

/// P's corpus, as the evaluator reads it.
struct Anchors;

impl eval::AnchorCorpus for Anchors {
    fn enter(&self) -> Box<dyn std::any::Any> {
        Box::new(super::sql::CorpusSlot::enter())
    }

    fn prepare(&self, code_rule: &CompiledCodeRule, snapshot: &Snapshot) {
        prepare_candidates(code_rule, snapshot)
    }

    fn roots(
        &self,
        target: &ast_grep_core::AstGrep<crate::code_rule_dsl::tape::TapeDoc>,
        grammar: &GrammarMatcher,
        bound: usize,
        path: &str,
    ) -> Option<Arc<[u32]>> {
        anchor_roots(target, grammar, bound, path)
    }
}

/// Registers P's corpus with the evaluator (`eval::ANCHOR_CORPUS`): boot, as it installs, and this crate's tests.
pub fn register() {
    eval::ANCHOR_CORPUS.register("rerail-p-anchors", &Anchors);
}

/// P's corpus publication and candidate preparation for one rule's stateless evaluation (the
/// `p-anchor-corpus` path, open item O20).
pub(super) fn prepare_candidates(code_rule: &CompiledCodeRule, snapshot: &Snapshot) {
    use crate::code_rule_dsl::matcher::{CorpusSlot, anchor_plans};
    let plans = anchor_plans(std::iter::once(code_rule));
    if plans.is_empty() {
        return;
    }
    let corpus = CorpusSlot::current();
    let mut demands = BTreeMap::new();
    for file in snapshot
        .files
        .iter()
        .filter(|file| code_rule.scope.matches(&file.path))
    {
        let Some(lang) = eval::file_language(file) else {
            continue;
        };
        if !plans.contains_key(&u32::from(
            crate::code_rule_dsl::language::tape_tag(lang).expect("tape language"),
        )) {
            continue;
        }
        match eval::file_tape(lang, file) {
            Ok(None) => {}
            Ok(Some((doc, _))) => {
                demands.insert(
                    file.path.clone(),
                    crate::code_rule_dsl::matcher::anchor_plans_for_file(code_rule, file, &doc),
                );
                if let Err(error) = corpus.put(
                    &file.path,
                    &doc,
                    &crate::code_rule_dsl::budget::matcher_budget_exhausted,
                ) {
                    eprintln!("SQL AST batch publication unavailable: {error}");
                }
            }
            Err(error) => eprintln!("SQL AST batch source unavailable: {error}"),
        }
    }
    #[cfg(feature = "code-rule-profile")]
    corpus.record_origins(std::iter::once(code_rule));
    if let Err(error) = corpus.prepare_files(&demands) {
        eprintln!("SQL AST corpus batch unavailable; retaining authoritative candidates: {error}");
    }
}

/// Selects candidate ordinals through the same postings used by workspace ingestion. Incomplete
/// indexes retain the existing bounded root walk. Kind-only queries also use SQL postings.
pub(super) fn anchor_roots(
    target: &ast_grep_core::AstGrep<crate::code_rule_dsl::tape::TapeDoc>,
    grammar: &GrammarMatcher,
    bound: usize,
    path: &str,
) -> Option<Arc<[u32]>> {
    use crate::code_rule_dsl::matcher::p_anchors::{plan_for, sql::FileAnchorIndex};
    let anchors = plan_for(grammar);
    if !anchors.needs_index() {
        return None;
    }
    #[cfg(feature = "code-rule-profile")]
    let timer = crate::code_rule_dsl::profile::PhaseTimer::start(
        crate::code_rule_dsl::profile::Phase::Index,
    );
    let index = FileAnchorIndex::build_at(
        target,
        &[&anchors],
        bound,
        crate::code_rule_dsl::budget::matcher_budget_exhausted,
        path,
    );
    #[cfg(feature = "code-rule-profile")]
    {
        drop(timer);
        crate::code_rule_dsl::profile::record_index(
            index.len(),
            index.estimated_bytes(),
            index.complete(),
        );
    }
    selected_roots(&index, &anchors)
}

fn selected_roots(
    index: &crate::code_rule_dsl::matcher::p_anchors::sql::FileAnchorIndex<
        '_,
        crate::code_rule_dsl::tape::TapeDoc,
    >,
    anchors: &Arc<crate::code_rule_dsl::matcher::p_anchors::AnchorPlan>,
) -> Option<Arc<[u32]>> {
    #[cfg(feature = "code-rule-profile")]
    let _timer = crate::code_rule_dsl::profile::PhaseTimer::start(
        crate::code_rule_dsl::profile::Phase::Selection,
    );
    index.selected(anchors)
}

