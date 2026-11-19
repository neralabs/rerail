//! A CSS file's tape from the lean parser (`rerail_css_lean`): tree-sitter-css's tape, written as
//! the parse goes, with nothing packed again (`formats::lean`: format 15's columns, or format 16 written in one pass). The tape is the one
//! tree-sitter-css's tree encodes to, node for node (`css_tapes_from_the_lean_parser_are_tree_sitters`).
//!
//! The parser's keys name kinds by their names in tree-sitter-css (`rerail_css_lean::KINDS`); here
//! they are its ids. A file the parser refuses (any syntax error, which tree-sitter recovers from; a
//! construct it does not read) goes to tree-sitter-css.

use crate::code_rule_dsl::tape::{CODE_EXTRA, Code, FileImage, Grammar, KIND_LIMIT, TapeImage, TemplateError, TreeParser};

/// tree-sitter-css's ids of the parser's kinds (`None`: not in the language).
fn kind_ids() -> &'static [Option<u16>] {
    static IDS: std::sync::OnceLock<Vec<Option<u16>>> = std::sync::OnceLock::new();
    IDS.get_or_init(|| {
        use crate::code_rule_dsl::language::GrammarDialect;
        use ast_grep_language::LanguageExt;
        let language = GrammarDialect::Css.get_ts_language();
        rerail_css_lean::KINDS
            .iter()
            .map(|&(name, named)| Some(language.id_for_node_kind(name, named)).filter(|&k| k != 0 && (k as usize) < KIND_LIMIT))
            .collect()
    })
}

/// `source`'s tape from the lean parser: `Err` where it refuses the file; `Ok(Err)` where the tape
/// cannot be made.
pub fn encode(grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Result<Result<TapeImage, String>, rerail_css_lean::Refusal> {
    let ids = kind_ids();
    let code_of = |&key: &u32| -> Result<Code, String> {
        let (kind, _, _, extra) = rerail_css_lean::unkey(key);
        let kind = ids[usize::from(kind)].ok_or_else(|| format!("tree-sitter-css lacks the lean parser's kind {kind}"))?;
        let mut code = Code::plain(kind, 0, &[])?;
        if extra {
            code.flags |= CODE_EXTRA;
        }
        Ok(code)
    };
    rerail_css_lean::tape_into(source.as_bytes(), &mut super::formats::lean(grammar, source, budget, &code_of))
}

/// CSS's tree parser: the lean parser, standing in for tree-sitter-css where it reads the file.
pub struct Lean;

impl TreeParser for Lean {
    fn parse(&self, grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Option<Result<FileImage, TemplateError>> {
        if !grammar.lang.embeds().is_empty() {
            return None;
        }
        let span = crate::code_rule_dsl::logic::profile::span(|| "lean CSS parse and tape".into());
        match encode(grammar, source, &budget) {
            Ok(image) => {
                drop(span);
                crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::SourceByte, source.len() as u64);
                Some(image.map(FileImage::single).map_err(TemplateError::Encode))
            }
            Err(why) => {
                crate::code_rule_dsl::logic::profile::decision("CSS parse", || format!("a file the lean parser refused ({why:?}): tree-sitter-css"));
                None
            }
        }
    }
}
