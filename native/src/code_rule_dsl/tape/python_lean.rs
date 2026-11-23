//! A Python file's tape from the lean parser (`rerail_python_lean`): tree-sitter-python's tape,
//! written as the parse goes, with nothing packed again (`formats::lean_named`: the run's format, written in one pass). The tape is the one
//! tree-sitter-python's tree encodes to, node for node (`python_tapes_from_the_lean_parser_are_tree_sitters`).
//!
//! The parser's keys name kinds, fields and supertypes by their names in tree-sitter-python
//! (`rerail_python_lean::{KINDS, FIELDS, CLASSES}`); its language's table numbers them by those names
//! (`names`, decision 0626). A file the parser refuses
//! (any syntax error, the text's end included, which tree-sitter recovers from; a construct it does
//! not read) goes to tree-sitter-python (decision 0163, amended).

use crate::code_rule_dsl::tape::{FileImage, Grammar, TapeImage, TemplateError, TreeParser};

/// `source`'s tape from the lean parser: `Err` where it refuses the file; `Ok(Err)` where the tape
/// cannot be made.
pub fn encode(grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Result<Result<TapeImage, String>, rerail_python_lean::Refusal> {
    encode_from(grammar, source, "module", budget)
}

/// [`encode`] from the node kind `start` (decision 0466).
pub fn encode_from(grammar: &Grammar, source: &str, start: &str, budget: &dyn Fn() -> bool) -> Result<Result<TapeImage, String>, rerail_python_lean::Refusal> {
    rerail_python_lean::tape_from_into(source.as_bytes(), start, &mut super::formats::lean_named(grammar, source, budget, rerail_python_lean::names))
}

/// Python's tree parser: the lean parser, standing in for tree-sitter-python where it reads the file.
pub struct Lean;

impl TreeParser for Lean {
    fn starts(&self) -> &'static [&'static str] {
        rerail_python_lean::STARTS
    }

    fn parse_from(&self, grammar: &Grammar, source: &str, start: &str, budget: &dyn Fn() -> bool) -> Option<Result<TapeImage, String>> {
        let _span = crate::code_rule_dsl::logic::profile::span(|| "lean Python parse of a fragment".into());
        encode_from(grammar, source, start, budget).ok()
    }

    fn parse(&self, grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Option<Result<FileImage, TemplateError>> {
        if !matches!(crate::code_rule_dsl::language::base(crate::code_rule_dsl::language::Family::of(grammar.lang)).parse_input(source), std::borrow::Cow::Borrowed(_))
        {
            return None;
        }
        let span = crate::code_rule_dsl::logic::profile::span(|| "lean Python parse and tape".into());
        match encode(grammar, source, &budget) {
            Ok(image) => {
                drop(span);
                crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::SourceByte, source.len() as u64);
                Some(image.map(|image| FileImage::single(image, source)).map_err(TemplateError::Encode))
            }
            Err(why) => {
                crate::code_rule_dsl::logic::profile::decision("Python parse", || format!("a file the lean parser refused ({why:?}): tree-sitter-python"));
                None
            }
        }
    }
}
