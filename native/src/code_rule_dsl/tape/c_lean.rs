//! A C or C++ file's tape from the lean parser (`rerail_c_lean`): tree-sitter-c's or
//! tree-sitter-cpp's tape, written as the parse goes, with nothing packed again
//! (`formats::lean_named`: the run's format, written in one pass). The tape is the one tree-sitter's tree encodes to, node for node
//! (`c_tapes_from_the_lean_parser_are_tree_sitters`, `cpp_tapes_from_the_lean_parser_are_tree_sitters`).
//!
//! One parser reads both languages (`rerail_c_lean::Dialect`). Its keys name kinds, fields and
//! supertypes by their names in tree-sitter-cpp, whose grammar extends tree-sitter-c's
//! (`rerail_c_lean::{KINDS, FIELDS, CLASSES}`); its language's table numbers them by those names
//! (`names`, decision 0626). A name the
//! dialect's grammar lacks is a kind only the other language writes. A file the parser refuses (any
//! syntax error, which tree-sitter recovers from; a construct it does not read exactly) goes to
//! tree-sitter.

use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::tape::{FileImage, Grammar, TapeImage, TemplateError, TreeParser};

/// The parser's dialect for `grammar`'s language (`None`: neither C nor C++).
fn dialect(grammar: &Grammar) -> Option<rerail_c_lean::Dialect> {
    match grammar.lang {
        GrammarDialect::C => Some(rerail_c_lean::Dialect::C),
        GrammarDialect::Cpp => Some(rerail_c_lean::Dialect::Cpp),
        _ => None,
    }
}

/// `source`'s tape from the lean parser, in `grammar`'s dialect (C or C++): `Err` where it refuses
/// the file; `Ok(Err)` where the tape cannot be made.
pub fn encode(grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Result<Result<TapeImage, String>, rerail_c_lean::Refusal> {
    let Some(dialect) = dialect(grammar) else { return Ok(Err(format!("the lean C parser reads no {:?}", grammar.lang))) };
    rerail_c_lean::tape_into(source.as_bytes(), dialect, &mut super::formats::lean_named(grammar, source, budget, rerail_c_lean::names))
}

/// C's and C++'s tree parser: the lean parser, standing in for tree-sitter where it reads the file.
pub struct Lean;

impl TreeParser for Lean {
    fn parse(&self, grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Option<Result<FileImage, TemplateError>> {
        if !matches!(crate::code_rule_dsl::language::base(crate::code_rule_dsl::language::Family::of(grammar.lang)).parse_input(source), std::borrow::Cow::Borrowed(_))
        {
            return None;
        }
        let dialect = dialect(grammar)?;
        let span = crate::code_rule_dsl::logic::profile::span(|| format!("lean {dialect:?} parse and tape"));
        match encode(grammar, source, &budget) {
            Ok(image) => {
                drop(span);
                crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::SourceByte, source.len() as u64);
                Some(image.map(|image| FileImage::single(image, source)).map_err(TemplateError::Encode))
            }
            Err(why) => {
                crate::code_rule_dsl::logic::profile::decision("C/C++ parse", || format!("a file the lean parser refused ({why:?}): tree-sitter-{dialect:?}"));
                None
            }
        }
    }
}
