//! A run of one dialect of a language of several (decision 0337): `.language typescript` runs once for each of `.ts`,
//! `.tsx` and `.js`, each lowered for its own grammar (`tools::run_program`, the `dialect` option). Each run reads
//! the other dialects' files in scope as text, and reports in its own dialect's files alone.
//!
//! What the run's dialect decides is the run's own, held for as long as the run lasts (`RunDialect`): the memo's space
//! names it (`memo::space_digest`), the findings it writes are its own dialect's (`reporter`), and its `files` leave
//! out the other dialects' it read as text.

use super::CorpusFiles;
use crate::code_rule_dsl::language::{label, CodeRuleLanguage};
use std::cell::RefCell;

/// A run under way on this thread: its dialect, the other dialects' files it read as text (sorted), and how many of
/// them it added to its documents.
#[derive(Clone)]
struct Run {
    dialect: Option<String>,
    text: std::sync::Arc<Vec<String>>,
    read_as_text: usize,
}

thread_local! {
    /// The runs under way on this thread, innermost last (`RunDialect`): the run's context, which a job on another
    /// thread carries (`carried`).
    static RUNS: RefCell<Vec<Run>> = const { RefCell::new(Vec::new()) };
}

/// The runs under way, for a job on another thread (`run_state::Carry`).
pub(in crate::code_rule_dsl) fn carried(c: &mut crate::code_rule_dsl::logic::run_state::Capture) {
    c.cloned(&["RUNS"], &RUNS);
}

/// The dialect a run was given (its `dialect` option), for as long as the run lasts. A run inside it (a phase's) is
/// under its own options until it ends.
pub(super) struct RunDialect;

impl RunDialect {
    /// The run of `options` begins.
    pub(super) fn enter(options: &serde_json::Value) -> RunDialect {
        let dialect = options.get("dialect").and_then(serde_json::Value::as_str).map(str::to_owned);
        RUNS.with(|r| r.borrow_mut().push(Run { dialect, text: Default::default(), read_as_text: 0 }));
        RunDialect
    }

    /// The innermost run's dialect, if it was given one.
    pub(super) fn current() -> Option<String> {
        RUNS.with(|r| r.borrow().last().and_then(|run| run.dialect.clone()))
    }

    /// How many of the other dialects' files the innermost run read as text (`with_other_dialects`).
    pub(super) fn read_as_text(&self) -> usize {
        RUNS.with(|r| r.borrow().last().map_or(0, |run| run.read_as_text))
    }
}

impl Drop for RunDialect {
    fn drop(&mut self) {
        RUNS.with(|r| r.borrow_mut().pop());
    }
}

/// `found` with the in-scope files of the other dialects of the run's (`options`' `dialect`) read as text, kept for the
/// run with their number (`RunDialect::read_as_text`, `reporter`).
pub(super) fn with_other_dialects(mut found: CorpusFiles, options: &serde_json::Value) -> CorpusFiles {
    let Some(dialect) = options.get("dialect").and_then(serde_json::Value::as_str) else { return found };
    let before = found.text.len();
    let mut others: Vec<String> = text_dialects(dialect).into_iter().filter_map(|l| found.by_label.get(l)).flatten().cloned().collect();
    others.sort();
    others.dedup();
    found.text.extend(others.iter().cloned());
    found.text.sort();
    found.text.dedup();
    let added = found.text.len() - before;
    let others = std::sync::Arc::new(others);
    RUNS.with(|r| {
        r.borrow_mut().last_mut().into_iter().for_each(|run| {
            run.text = others.clone();
            run.read_as_text = added;
        })
    });
    found
}

/// The dialects whose files a run of `dialect` reads as text only: where `dialect` is one of a language of several
/// (TypeScript's `.ts`, `.tsx` and `.js` files), the language's others. Each of the program's runs is lowered for its
/// own dialect, and parses and reports that dialect's files alone (`reporter`); a cross-file rule of one sees every
/// file's `doc` and source text.
fn text_dialects(dialect: &str) -> Vec<&'static str> {
    let mut several = CodeRuleLanguage::ALL.into_iter().filter(|l| l.dialects().len() > 1);
    let Some(language) = several.find(|l| l.dialects().iter().any(|d| label(*d) == Some(dialect))) else { return Vec::new() };
    language.dialects().iter().filter_map(|d| label(*d)).filter(|l| *l != dialect).collect()
}

/// Whether the innermost run writes a finding in the file at a path: any but one of the other dialects' files it read
/// as text only (`with_other_dialects`), which that dialect's own run parses and reports.
pub(super) fn reporter() -> impl Fn(&str) -> bool {
    let text = RUNS.with(|r| r.borrow().last().map(|run| run.text.clone())).unwrap_or_default();
    move |path| text.binary_search_by(|p| p.as_str().cmp(path)).is_err()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A run's dialect holds while it lasts, and a run inside it holds its own until it ends. A run writes no finding in
    /// another dialect's file it read as text, and any other: one of another dialect it did not read, a file of no
    /// dialect. A run of no dialect writes every finding.
    #[test]
    fn a_runs_dialect_holds_while_it_lasts() {
        assert!(reporter()("a.tsx"));
        let options = serde_json::json!({"dialect": "typescript"});
        let outer = RunDialect::enter(&options);
        assert_eq!(RunDialect::current().as_deref(), Some("typescript"));
        let found = CorpusFiles {
            by_label: [("typescript", vec!["a.ts".to_owned()]), ("tsx", vec!["b.tsx".to_owned()])].into(),
            text: vec!["README.md".to_owned()],
        };
        let found = with_other_dialects(found, &options);
        assert_eq!(found.text, ["README.md", "b.tsx"]);
        assert_eq!(outer.read_as_text(), 1);
        let reported = reporter();
        assert!(reported("a.ts") && reported("README.md") && reported("c.tsx") && !reported("b.tsx"));
        {
            let _inner = RunDialect::enter(&serde_json::Value::Null);
            assert_eq!(RunDialect::current(), None);
            assert!(reporter()("b.tsx"));
        }
        assert_eq!(RunDialect::current().as_deref(), Some("typescript"));
        drop(outer);
        assert_eq!(RunDialect::current(), None);
    }
}
