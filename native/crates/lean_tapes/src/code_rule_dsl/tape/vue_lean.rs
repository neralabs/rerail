//! A Vue file's tape from the lean parser (`rerail_vue_lean`): tree-sitter-vue's tape, written as
//! the parse goes, with nothing packed again (`formats::lean_named`: the run's format, written in one pass). The tape is the one
//! tree-sitter-vue's tree encodes to, node for node (`vue_tapes_from_the_lean_parser_are_tree_sitters`).
//! Its embedded program (the component's scripts and template expressions, `script-block`) is
//! built from the tape (`FileImage::embedding`), as it is from tree-sitter's.
//!
//! The parser's keys name kinds by their names in tree-sitter-vue (`rerail_vue_lean::KINDS`); its language's table numbers them by those names
//! (`names`, decision 0631). A file the parser refuses (any syntax error, which tree-sitter recovers from;
//! a construct it does not read) goes to tree-sitter-vue.

use crate::code_rule_dsl::tape::{FileImage, Grammar, TapeImage, TemplateError, TreeParser};

/// `source`'s tape from the lean parser: `Err` where it refuses the file; `Ok(Err)` where the tape
/// cannot be made.
pub fn encode(grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Result<Result<TapeImage, String>, rerail_vue_lean::Refusal> {
    rerail_vue_lean::tape_into(source.as_bytes(), &mut super::formats::lean_named(grammar, source, budget, rerail_vue_lean::names))
}

/// Vue's tree parser: the lean parser, standing in for tree-sitter-vue where it reads the file.
pub struct Lean;

impl TreeParser for Lean {
    fn parse(&self, grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Option<Result<FileImage, TemplateError>> {
        let span = crate::code_rule_dsl::logic::profile::span(|| "lean Vue parse and tape".into());
        match encode(grammar, source, &budget) {
            Ok(image) => {
                drop(span);
                crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::SourceByte, source.len() as u64);
                // Its scripts and template expressions are embedded by the router (`TapeImage::parse`).
                Some(image.map(|image| FileImage::single(image, source)).map_err(TemplateError::Encode))
            }
            Err(why) => {
                crate::code_rule_dsl::logic::profile::decision("Vue parse", || format!("a file the lean parser refused ({why:?}): tree-sitter-vue"));
                None
            }
        }
    }
}
