//! The `percent-format` and `brace-format` dialects' tapes (decision 0176): a Python format string's tree as Ruff's
//! parsers give it, through their ports (`literal::percent`, `literal::brace`), so that main's rules that parse a
//! string as `percentFormat` or `braceFormat` (`parsed(s, "percentFormat", r)`) read the tree they were written
//! for: a `format` root, its `literal`, `placeholder` and `field` parts, and an `error` node holding Ruff's message.
//!
//! The parsers are hand-written ports, which rank 0 allows until each grammar is a transducer program. A document of
//! either dialect is a guest (its text a string's value), never a file of the corpus.
//!
//! - Kinds: the grammar's `KINDS`, by id from 1 (`literal::Grammar::kind_id`), all named; no fields.
//! - Nodes: the tree in preorder, so node `i` of the tape is node `i` of the parser's preorder ([`values`]).
//! - Values: a node's value (an `error`'s message, a `field_name`'s argument), read by [`values`] off a parse of the
//!   document's text (`core_rr`'s literal facts).

use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::literal::{Grammar as Literal, PNode};
use crate::code_rule_dsl::tape::{FileImage, Grammar, OwnTable, Pending, TapeImage, TemplateError, TreeParser, checked};
use crate::code_rule_dsl::value::Value;
use std::sync::LazyLock;

/// The format strings' tree parser for one dialect: Ruff's parser of its grammar.
pub struct FormatStrings(pub GrammarDialect);

/// The literal grammar a dialect's documents are parsed with.
fn grammar(lang: GrammarDialect) -> Option<Literal> {
    match lang {
        GrammarDialect::PercentFormat => Some(super::percent::GRAMMAR),
        GrammarDialect::BraceFormat => Some(super::brace::GRAMMAR),
        _ => None,
    }
}

/// A grammar's kinds as an own table names them: `""` at 0, then its kinds (all named), so an id is the grammar's.
fn table(kinds: &'static [&'static str]) -> OwnTable {
    let names: Vec<&'static str> = std::iter::once("").chain(kinds.iter().copied()).collect();
    let names: &'static [&'static str] = Box::leak(names.into_boxed_slice());
    (names, names.len() as u16, &[""])
}

static PERCENT: LazyLock<OwnTable> = LazyLock::new(|| table(super::percent::KINDS));
static BRACE: LazyLock<OwnTable> = LazyLock::new(|| table(super::brace::KINDS));

/// The tree's nodes in preorder.
fn preorder(root: &PNode) -> Vec<(&PNode, Option<u32>)> {
    let mut out = Vec::new();
    let mut stack = vec![(root, None)];
    while let Some((node, parent)) = stack.pop() {
        let id = out.len() as u32;
        out.push((node, parent));
        stack.extend(node.children.iter().rev().map(|c| (c, Some(id))));
    }
    out
}

impl TreeParser for FormatStrings {
    fn parse(&self, tape: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Option<Result<FileImage, TemplateError>> {
        let literal = grammar(tape.lang)?;
        let encode = || -> Result<TapeImage, String> {
            checked(&budget)?;
            if source.len() > u32::MAX as usize {
                return Err("source exceeds tape range".into());
            }
            let root = literal.parse(source);
            let nodes = preorder(&root)
                .into_iter()
                .map(|(n, parent)| {
                    let start = n.start.min(source.len()) as u32;
                    let end = n.end.clamp(n.start, source.len()) as u32;
                    Pending { kf: n.kind, parent, start, length: end - start, size: 1, extra: n.extra, error: false, missing: false, kind_alias: None }
                })
                .collect();
            TapeImage::finish(nodes, Vec::new(), tape, source, &budget)
        };
        Some(encode().map(FileImage::single).map_err(TemplateError::Encode))
    }

    fn kinds(&self) -> Option<OwnTable> {
        match self.0 {
            GrammarDialect::PercentFormat => Some(*PERCENT),
            GrammarDialect::BraceFormat => Some(*BRACE),
            _ => None,
        }
    }
}

/// Each node's value, in preorder (the tape's ids), as the core relations hold it: its text and its type.
pub fn values(lang: GrammarDialect, source: &str) -> Vec<Option<(String, &'static str)>> {
    let Some(literal) = grammar(lang) else { return Vec::new() };
    let root = literal.parse(source);
    preorder(&root)
        .into_iter()
        .map(|(n, _)| match &n.value {
            Some(Value::Str(s)) => Some((s.clone(), "str")),
            Some(Value::Int(i)) => Some((i.to_string(), "int")),
            Some(Value::Bool(b)) => Some(((if *b { "True" } else { "False" }).to_owned(), "bool")),
            _ => None,
        })
        .collect()
}

/// The format strings' core facts (`core_rr::DialectFacts`): their nodes' values.
pub struct FormatFacts;

impl crate::code_rule_dsl::facts::vtab::syntax::core_rr::DialectFacts for FormatFacts {
    fn node_values(&self, tag: u8, source: &str) -> Vec<Option<(String, &'static str)>> {
        crate::code_rule_dsl::language::from_tape_tag(tag).map_or_else(Vec::new, |lang| values(lang, source))
    }
}
