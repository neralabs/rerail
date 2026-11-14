//! The tapes of the dialects a literal grammar parses (decisions 0176 and 0194): a string's or a comment's tree as
//! the engine's literal parser of its grammar gives it, so that main's rules that parse a value with a grammar
//! (`parsed(s, "percentFormat", r)`, `parsed(c, "eslintDirective", r)`) read the tree they were written for. The
//! dialects are `percent-format` and `brace-format` (Ruff's parsers' ports, `literal::percent` and `literal::brace`:
//! a `format` root, its `literal`, `placeholder` and `field` parts, and an `error` node holding Ruff's message) and
//! `eslint-directive` (`literal::eslint_directive`: a `directive` root, its `item`s and their `setting`s, and a
//! `description`).
//!
//! A dialect's grammar is the registered literal grammar its label names in camel case (`eslint-directive` is
//! `eslintDirective`), so this parser names no grammar: a dialect of a grammar another crate holds needs no edit here.
//! The parsers are hand-written ports, which rank 0 allows until each grammar is a transducer program. A document of
//! these dialects is a guest (a string's value, a comment's text), never a file of the corpus.
//!
//! - Kinds: the grammar's kinds, by id from 1 (`literal::Grammar::kind_id`), all named; no fields.
//! - Nodes: the tree in preorder, so node `i` of the tape is node `i` of the parser's preorder ([`values`]).
//! - Values: a node's value (an `error`'s message, a `field_name`'s argument, a directive's label, an item's name),
//!   read by [`values`] off a parse of the document's text (`core_rr`'s literal facts).

use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::literal::{Grammar as Literal, PNode};
use crate::code_rule_dsl::tape::{FileImage, Grammar, OwnTable, Pending, TapeImage, TemplateError, TreeParser, checked};
use crate::code_rule_dsl::value::Value;
use std::sync::OnceLock;

/// The tree parser of one dialect: the engine's parser of its literal grammar. Its kinds table is made once.
pub struct FormatStrings(pub GrammarDialect, pub OnceLock<OwnTable>);

impl FormatStrings {
    pub const fn new(lang: GrammarDialect) -> Self {
        Self(lang, OnceLock::new())
    }
}

/// The literal grammar a dialect's documents are parsed with: the one its label names in camel case.
fn grammar(lang: GrammarDialect) -> Option<Literal> {
    let label = crate::code_rule_dsl::language::label(lang)?;
    let mut name = String::with_capacity(label.len());
    let mut upper = false;
    for c in label.chars() {
        if c == '-' {
            upper = true;
        } else {
            name.push(if upper { c.to_ascii_uppercase() } else { c });
            upper = false;
        }
    }
    Literal::named(&name)
}

/// A grammar's kinds as an own table names them: `""` at 0, then its kinds (all named), so an id is the grammar's.
fn table(kinds: &'static [&'static str]) -> OwnTable {
    let names: Vec<&'static str> = std::iter::once("").chain(kinds.iter().copied()).collect();
    let names: &'static [&'static str] = Box::leak(names.into_boxed_slice());
    (names, names.len() as u16, &[""])
}

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
        let literal = grammar(self.0)?;
        Some(*self.1.get_or_init(|| table(literal.kinds())))
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
