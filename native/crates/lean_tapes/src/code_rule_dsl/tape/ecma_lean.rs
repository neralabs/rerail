//! A JavaScript, TypeScript or TSX file's tape from the lean parser (`rerail_ecma_lean`):
//! tree-sitter-javascript's, -typescript's or -tsx's tape, written as the parse goes, with nothing
//! packed again (`TapeImage::written`). The tape is the one the dialect's tree-sitter tree encodes
//! to, node for node (the crate's oracle; `ecma_tapes_from_the_lean_parser_are_tree_sitters`).
//!
//! The parser's keys name kinds, fields and supertypes by their names in the grammars
//! (`rerail_ecma_lean::{KINDS, FIELDS, CLASSES}`), one list for the three; its language's table numbers them by those names
//! (`names`, decision 0631). A kind one grammar lacks (TypeScript's in JavaScript) has none, and the parser
//! never writes it there. A file the parser refuses (any syntax error, which tree-sitter recovers
//! from; a construct it does not read) goes to tree-sitter.

use rerail_blocks::vector::Vector;
use crate::code_rule_dsl::tape::{FileImage, Grammar, TapeImage, TemplateError, TreeParser};
use rerail_ecma_lean::Dialect;

/// `source`'s tape from the lean parser in `dialect`: `Err` where it refuses the file; `Ok(Err)`
/// where the tape cannot be made.
pub fn encode(dialect: Dialect, grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Result<Result<TapeImage, String>, rerail_ecma_lean::Refusal> {
    encode_from(dialect, grammar, source, "program", budget).expect("a program is a start")
}

/// [`encode`] from the node kind `start` (decision 0466): `None` for a kind the parser does not start at.
pub fn encode_from(dialect: Dialect, grammar: &Grammar, source: &str, start: &str, budget: &dyn Fn() -> bool) -> Option<Result<Result<TapeImage, String>, rerail_ecma_lean::Refusal>> {
    encode_omitting(dialect, grammar, source, start, Vector::new(), budget)
}

/// [`encode_from`] of an embedded program, its placeholders `omit` (`SegmentMap::omissions`) left out as it is
/// written (`Writer::omitting`).
pub fn encode_omitting(dialect: Dialect, grammar: &Grammar, source: &str, start: &str, omit: Vector<[u32; 2]>, budget: &dyn Fn() -> bool) -> Option<Result<Result<TapeImage, String>, rerail_ecma_lean::Refusal>> {
    rerail_ecma_lean::tape_from_into(source.as_bytes(), dialect, start, &mut super::formats::lean_named(grammar, source, budget, rerail_ecma_lean::names).omitting(omit))
}

/// A dialect's tree parser: the lean parser, standing in for tree-sitter where it reads the file.
pub struct Lean(pub Dialect);

pub static JAVASCRIPT: Lean = Lean(Dialect::JavaScript);
pub static TYPESCRIPT: Lean = Lean(Dialect::TypeScript);
pub static TSX: Lean = Lean(Dialect::Tsx);

impl TreeParser for Lean {
    /// A JSX element (decision 0466), in the dialects that read JSX: not TypeScript's.
    fn starts(&self) -> &'static [&'static str] {
        if self.0 == Dialect::TypeScript { &[] } else { rerail_ecma_lean::STARTS }
    }

    fn parse_from(&self, grammar: &Grammar, source: &str, start: &str, budget: &dyn Fn() -> bool) -> Option<Result<TapeImage, String>> {
        let _span = crate::code_rule_dsl::logic::profile::span(|| format!("lean {:?} parse of a fragment", self.0));
        encode_from(self.0, grammar, source, start, budget)?.ok()
    }

    /// An embedded program's text, read as it is (a Vue component's script, an HTML page's).
    fn program(&self, grammar: &Grammar, text: &str, omit: Vector<[u32; 2]>, budget: &dyn Fn() -> bool) -> Option<Result<TapeImage, String>> {
        encode_omitting(self.0, grammar, text, "program", omit, budget).expect("a program is a start").ok()
    }

    fn parse(&self, grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Option<Result<FileImage, TemplateError>> {
        if !matches!(crate::code_rule_dsl::language::base(crate::code_rule_dsl::language::Family::of(grammar.lang)).parse_input(source), std::borrow::Cow::Borrowed(_))
        {
            return None;
        }
        let span = crate::code_rule_dsl::logic::profile::span(|| format!("lean {:?} parse and tape", self.0));
        match encode(self.0, grammar, source, &budget) {
            Ok(image) => {
                drop(span);
                crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::SourceByte, source.len() as u64);
                Some(image.map(|image| FileImage::single(image, source)).map_err(TemplateError::Encode))
            }
            Err(why) => {
                crate::code_rule_dsl::logic::profile::decision("ECMAScript parse", || format!("a file the lean parser refused ({why:?}): tree-sitter-{:?}", self.0));
                None
            }
        }
    }
}
