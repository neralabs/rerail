//! A PHP file's tape from the lean parser (`rerail_php_lean`): tree-sitter-php's tape (its `php_only`
//! grammar), written as the parse goes, with nothing packed again (`formats::lean_named`: the run's format, written in one pass). The tape is
//! the one tree-sitter-php's tree encodes to, node for node (`php_tapes_from_the_lean_parser_are_tree_sitters`).
//!
//! The parser reads the file's view (`parse_input`: the text outside `<?php … ?>` blanked), the
//! bytes tree-sitter-php parses; the view keeps the file's length and every offset, so the tape
//! reads the file. The parser's keys name kinds, fields and supertypes by their names in
//! tree-sitter-php (`rerail_php_lean::{KINDS, FIELDS, CLASSES}`); its language's table numbers them by those names
//! (`names`, decision 0631). A file the
//! parser refuses (any syntax error, which tree-sitter recovers from; a construct it does not read)
//! goes to tree-sitter-php.

use rerail_blocks::vector::Vector;
use crate::code_rule_dsl::tape::{FileImage, Grammar, TapeImage, TemplateError, TreeParser};

/// `source`'s tape from the lean parser: `Err` where it refuses the file; `Ok(Err)` where the tape
/// cannot be made.
pub fn encode(grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Result<Result<TapeImage, String>, rerail_php_lean::Refusal> {
    let view = crate::code_rule_dsl::language::parse_input(crate::code_rule_dsl::language::GrammarDialect::Php, source);
    debug_assert_eq!(view.len(), source.len());
    encode_input(grammar, view.as_bytes(), source, Vector::new(), budget)
}

/// The tape of a program's text as it is (no view): a Blade template's compiled program, which
/// tree-sitter-php reads with no `<?php` tag (`blade::view`).
/// Its placeholders `omit` (`SegmentMap::omissions`) are left out as it is written (`Writer::omitting`).
pub fn encode_program(grammar: &Grammar, text: &str, omit: Vector<[u32; 2]>, budget: &dyn Fn() -> bool) -> Result<Result<TapeImage, String>, rerail_php_lean::Refusal> {
    encode_input(grammar, text.as_bytes(), text, omit, budget)
}

/// The tape of `input`, which has `source`'s length and offsets, reading `source`.
fn encode_input(grammar: &Grammar, input: &[u8], source: &str, omit: Vector<[u32; 2]>, budget: &dyn Fn() -> bool) -> Result<Result<TapeImage, String>, rerail_php_lean::Refusal> {
    rerail_php_lean::tape_into(input, &mut super::formats::lean_named(grammar, source, budget, rerail_php_lean::names).omitting(omit))
}

/// PHP's tree parser: the lean parser, standing in for tree-sitter-php where it reads the file.
pub struct Lean;

impl TreeParser for Lean {
    /// An embedded program's text as it is, with no `<?php` tag and no view (`encode_program`: a Blade template's
    /// compiled program).
    fn program(&self, grammar: &Grammar, text: &str, omit: Vector<[u32; 2]>, budget: &dyn Fn() -> bool) -> Option<Result<TapeImage, String>> {
        encode_program(grammar, text, omit, budget).ok()
    }

    fn parse(&self, grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Option<Result<FileImage, TemplateError>> {
        let span = crate::code_rule_dsl::logic::profile::span(|| "lean PHP parse and tape".into());
        match encode(grammar, source, &budget) {
            Ok(image) => {
                drop(span);
                crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::SourceByte, source.len() as u64);
                Some(image.map(|image| FileImage::single(image, source)).map_err(TemplateError::Encode))
            }
            Err(why) => {
                crate::code_rule_dsl::logic::profile::decision("PHP parse", || format!("a file the lean parser refused ({why:?}): tree-sitter-php"));
                None
            }
        }
    }
}
