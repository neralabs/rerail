//! A SQL file's tape from the lean parser (`rerail_sql_lean`): tree-sitter-sql's
//! tape, written as the parse goes, with nothing packed again (`formats::lean_named`: the run's format, written in one pass). The tape is
//! the one tree-sitter-sql's tree encodes to, node for node
//! (`sql_tapes_from_the_lean_parser_are_tree_sitters`).
//!
//! The parser's keys name kinds and fields by their names in tree-sitter-sql
//! (`rerail_sql_lean::{KINDS, FIELDS}`); its language's table numbers them by those names
//! (`names`, decision 0626). A file the parser refuses (any
//! syntax error, which tree-sitter recovers from; a construct it does not read) goes to
//! tree-sitter-sql.

use crate::code_rule_dsl::tape::{FileImage, Grammar, TapeImage, TemplateError, TreeParser};

/// `source`'s tape from the lean parser: `Err` where it refuses the file; `Ok(Err)` where the tape
/// cannot be made.
pub fn encode(grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Result<Result<TapeImage, String>, rerail_sql_lean::Refusal> {
    rerail_sql_lean::tape_into(source.as_bytes(), &mut super::formats::lean_named(grammar, source, budget, rerail_sql_lean::names))
}

/// SQL's tree parser: the lean parser, standing in for tree-sitter-sql where it reads the file.
pub struct Lean;

impl TreeParser for Lean {
    fn parse(&self, grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Option<Result<FileImage, TemplateError>> {
        let span = crate::code_rule_dsl::logic::profile::span(|| "lean SQL parse and tape".into());
        match encode(grammar, source, &budget) {
            Ok(image) => {
                drop(span);
                crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::SourceByte, source.len() as u64);
                Some(image.map(|image| FileImage::single(image, source)).map_err(TemplateError::Encode))
            }
            Err(why) => {
                crate::code_rule_dsl::logic::profile::decision("SQL parse", || format!("a file the lean parser refused ({why:?}): tree-sitter-sql"));
                None
            }
        }
    }
}
