//! An inline text's tape from the lean parser (`rerail_markdown_inline_lean`): tree-sitter-md's
//! inline grammar's tape, with nothing packed again (`formats::lean_named`: the run's format, written in one pass). The tape is the one
//! tree-sitter-md-inline's tree encodes to, node for node
//! (`markdown_inline_tapes_from_the_lean_parser_are_tree_sitters`). The texts are the guests a
//! Markdown file's `inline` and `pipe_table_cell` nodes embed (`embed::inline_content`).
//!
//! The parser's keys name kinds by their names in the inline grammar
//! (`rerail_markdown_inline_lean::KINDS`; it has no fields); its language's table numbers them by those names
//! (`names`, decision 0626). A text the
//! parser refuses (an ambiguity tree-sitter settles by its own measure, a construct it does not
//! read) goes to tree-sitter-md-inline.

use crate::code_rule_dsl::tape::{FileImage, Grammar, TapeImage, TemplateError, TreeParser};

/// `source`'s tape from the lean parser: `Err` where it refuses the text; `Ok(Err)` where the tape
/// cannot be made.
pub fn encode(grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Result<Result<TapeImage, String>, rerail_markdown_inline_lean::Refusal> {
    rerail_markdown_inline_lean::tape_into(source.as_bytes(), &mut super::formats::lean_named(grammar, source, budget, rerail_markdown_inline_lean::names))
}

/// The inline grammar's tree parser: the lean parser, standing in for tree-sitter-md-inline where it
/// reads the text.
pub struct Lean;

impl TreeParser for Lean {
    fn parse(&self, grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Option<Result<FileImage, TemplateError>> {
        let span = crate::code_rule_dsl::logic::profile::span(|| "lean Markdown inline parse and tape".into());
        match encode(grammar, source, &budget) {
            Ok(image) => {
                drop(span);
                crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::SourceByte, source.len() as u64);
                Some(image.map(|image| FileImage::single(image, source)).map_err(TemplateError::Encode))
            }
            Err(why) => {
                crate::code_rule_dsl::logic::profile::decision("Markdown inline parse", || format!("a text the lean parser refused ({why:?}): tree-sitter-md-inline"));
                None
            }
        }
    }
}
