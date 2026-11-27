//! A run of one dialect of a language of several (decision 0337): `.language typescript` runs once for each of `.ts`,
//! `.tsx` and `.js`, each lowered for its own grammar (`tools::run_program`, the `dialect` option). Each run reads
//! the other dialects' files in scope as text, and reports in its own dialect's files alone.
//!
//! What the run's dialect decides is the run's own, held for as long as the run lasts (`RunDialect`): the memo's space
//! names it (`memo::space_digest`), the findings it writes are its own dialect's (`reporter`), and its `files` leave
//! out the other dialects' it read as text.

use super::CorpusFiles;
use crate::code_rule_dsl::language::{label, label_of_path, CodeRuleLanguage};
use std::cell::RefCell;

/// A run under way on this thread: its dialect, and how many of the other dialects' files it read as text.
struct Run {
    dialect: Option<String>,
    read_as_text: usize,
}

thread_local! {
    /// The runs under way on this thread, innermost last (`RunDialect`).
    static RUNS: RefCell<Vec<Run>> = const { RefCell::new(Vec::new()) };
}

/// The dialect a run was given (its `dialect` option), for as long as the run lasts. A run inside it (a phase's) is
/// under its own options until it ends.
pub(super) struct RunDialect;

impl RunDialect {
    /// The run of `options` begins.
    pub(super) fn enter(options: &serde_json::Value) -> RunDialect {
        let dialect = options.get("dialect").and_then(serde_json::Value::as_str).map(str::to_owned);
        RUNS.with(|r| r.borrow_mut().push(Run { dialect, read_as_text: 0 }));
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

/// `found` with the in-scope files of the other dialects of the run's (`options`' `dialect`) read as text, their number
/// kept for the run (`RunDialect::read_as_text`).
pub(super) fn with_other_dialects(mut found: CorpusFiles, options: &serde_json::Value) -> CorpusFiles {
    let Some(dialect) = options.get("dialect").and_then(serde_json::Value::as_str) else { return found };
    let before = found.text.len();
    let others: Vec<String> = text_dialects(dialect).into_iter().filter_map(|l| found.by_label.get(l)).flatten().cloned().collect();
    found.text.extend(others);
    found.text.sort();
    found.text.dedup();
    let added = found.text.len() - before;
    RUNS.with(|r| r.borrow_mut().last_mut().into_iter().for_each(|run| run.read_as_text = added));
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

/// Whether the innermost run writes a finding in the file at a path: any, where it was given no dialect; else not one in
/// a file of another dialect of its language, which it read as text only and which that dialect's own run reports.
pub(super) fn reporter() -> impl Fn(&str) -> bool {
    let others = RunDialect::current().map(|dialect| text_dialects(&dialect)).unwrap_or_default();
    move |path| label_of_path(path).is_none_or(|l| !others.contains(&l))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A run's dialect holds while it lasts, and a run inside it holds its own until it ends; a run of no dialect
    /// writes every finding.
    #[test]
    fn a_runs_dialect_holds_while_it_lasts() {
        assert!(reporter()("a.tsx"));
        let outer = RunDialect::enter(&serde_json::json!({"dialect": "typescript"}));
        assert_eq!(RunDialect::current().as_deref(), Some("typescript"));
        let reported = reporter();
        assert!(reported("a.ts") && reported("README.md") && !reported("b.tsx"));
        {
            let _inner = RunDialect::enter(&serde_json::Value::Null);
            assert_eq!(RunDialect::current(), None);
            assert!(reporter()("b.tsx"));
        }
        assert_eq!(RunDialect::current().as_deref(), Some("typescript"));
        assert_eq!(outer.read_as_text(), 0);
        drop(outer);
        assert_eq!(RunDialect::current(), None);
    }
}
