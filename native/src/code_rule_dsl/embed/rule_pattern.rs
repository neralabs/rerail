//! `pattern`: a rule file's patterns (decision 0495 §3), each a program in the file's `.language`, read from that
//! language's pattern fragment (the one starting at `pattern_module`: a module whose metavariables are names).
//!
//! ```text
//! .language python
//! call(c) :- c ~ `subprocess.run($CMD, shell=True)`.     a Python program: the pattern, at its bytes
//! .language typescript
//! call(c) :- c ~ `eval($X)`.                               none: TypeScript registers no pattern fragment
//! ```
//!
//! A program's text is the backtick literal's body as the rule file's language reads it: between its fences, and,
//! for a fence of two backticks or more, without one space on each side. Its bytes are the file's.

use super::host::HostNode;
use super::segments::SegmentMap;
use super::{Embedder, Program};
use crate::code_rule_dsl::language::GrammarDialect;

/// The start a language's pattern fragment parses from (decision 0495 §3).
pub const START: &str = "pattern_module";

/// The embedder of `Via::Pattern`.
pub struct RulePattern;

impl Embedder for RulePattern {
    fn build(&self, root: HostNode<'_>, source: &str, budget: &dyn Fn() -> bool) -> Option<Program> {
        self.build_all(root, source, budget).into_iter().next()
    }

    fn build_all(&self, root: HostNode<'_>, source: &str, budget: &dyn Fn() -> bool) -> Vec<Program> {
        let Some(dialect) = pattern_dialect(root, source) else { return Vec::new() };
        let Ok(grammar) = crate::code_rule_dsl::tape::Grammar::embedded(dialect) else { return Vec::new() };
        let mut programs = Vec::new();
        let mut stack = vec![root];
        while let Some(node) = stack.pop() {
            if node.kind() == "pattern_match"
                && let Some(literal) = node.named_children().find(|c| c.kind() == "code_literal")
                && let Some(program) = program(dialect, &grammar, literal, source, budget)
            {
                programs.push(program);
            }
            let children = node.named_children().collect::<Vec<_>>();
            stack.extend(children.into_iter().rev());
        }
        programs
    }

    fn per_place(&self) -> bool {
        true
    }

    /// A pattern is no file of its language: CPython refuses `$CMD`, so its verdict is its tape's.
    fn tape_verdict(&self) -> bool {
        true
    }
}

/// The dialect the file's `.language` names, where it registers a pattern fragment.
fn pattern_dialect(root: HostNode<'_>, source: &str) -> Option<GrammarDialect> {
    let language = root.named_children().filter(|c| c.kind() == "setting").find_map(|s| {
        let mut words = source[s.byte_range()].split_whitespace();
        (words.next() == Some(".language")).then(|| words.next()).flatten()
    })?;
    let dialect = crate::code_rule_dsl::language::dialect_by_label(language)?;
    crate::code_rule_dsl::language::fragments().iter().any(|&(_, d, start, _)| d == dialect && start == START).then_some(dialect)
}

/// The pattern program of a backtick literal: its body, at its bytes.
fn program(dialect: GrammarDialect, grammar: &crate::code_rule_dsl::tape::Grammar, literal: HostNode<'_>, source: &str, budget: &dyn Fn() -> bool) -> Option<Program> {
    let range = literal.byte_range();
    let quoted = &source[range.clone()];
    let fence = quoted.bytes().take_while(|&b| b == b'`').count();
    if fence == 0 || quoted.len() < 2 * fence {
        return None;
    }
    let (mut start, mut end) = (range.start + fence, range.end - fence);
    if fence > 1 {
        if source[start..end].starts_with(' ') {
            start += 1;
        }
        if end > start && source[start..end].ends_with(' ') {
            end -= 1;
        }
    }
    let text = source[start..end].to_owned();
    if text.trim().is_empty() {
        return None;
    }
    let mut segments = SegmentMap::new();
    segments.push(0, text.len(), start..end, true, false);
    segments.index();
    let image = crate::code_rule_dsl::tape::TapeImage::parse_from(grammar, &text, START, &budget)?.ok()?;
    Some(Program { dialect, parse: super::Parse::Tape(image.primary), text, segments, host_errors: Vec::new(), at: Some(range) })
}
