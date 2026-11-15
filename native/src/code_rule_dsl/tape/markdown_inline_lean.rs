//! An inline text's tape from the lean parser (`rerail_markdown_inline_lean`): tree-sitter-md's
//! inline grammar's tape, with nothing packed again (`TapeImage::written`). The tape is the one
//! tree-sitter-md-inline's tree encodes to, node for node
//! (`markdown_inline_tapes_from_the_lean_parser_are_tree_sitters`). The texts are the guests a
//! Markdown file's `inline` and `pipe_table_cell` nodes embed (`guests::markdown_inline_text`).
//!
//! The parser's keys name kinds by their names in the inline grammar
//! (`rerail_markdown_inline_lean::KINDS`; it has no fields); here they are its ids. A text the
//! parser refuses (an ambiguity tree-sitter settles by its own measure, a construct it does not
//! read) goes to tree-sitter-md-inline.

use crate::code_rule_dsl::tape::{Code, FileImage, Grammar, KIND_LIMIT, TapeImage, TemplateError, TreeParser};

/// The inline grammar's ids of the parser's kinds (`None`: not in the language).
fn kinds() -> &'static [Option<u16>] {
    static KINDS: std::sync::OnceLock<Vec<Option<u16>>> = std::sync::OnceLock::new();
    KINDS.get_or_init(|| {
        use crate::code_rule_dsl::language::GrammarDialect;
        use ast_grep_language::LanguageExt;
        let language = GrammarDialect::MarkdownInline.get_ts_language();
        rerail_markdown_inline_lean::KINDS
            .iter()
            .map(|&(name, named)| Some(language.id_for_node_kind(name, named)).filter(|&k| k != 0 && (k as usize) < KIND_LIMIT))
            .collect()
    })
}

/// `source`'s tape from the lean parser: `Err` where it refuses the text; `Ok(Err)` where the tape
/// cannot be made.
pub fn encode(grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Result<Result<TapeImage, String>, rerail_markdown_inline_lean::Refusal> {
    let tape = rerail_markdown_inline_lean::tape(source.as_bytes())?;
    let kinds = kinds();
    let codes: Result<Vec<Code>, String> = tape
        .keys
        .iter()
        .map(|&key| {
            let (kind, _, _, _) = rerail_markdown_inline_lean::unkey(key);
            let kind = kinds[usize::from(kind)].ok_or_else(|| format!("tree-sitter-md-inline lacks the lean parser's kind {kind}"))?;
            Code::plain(kind, 0, &[])
        })
        .collect();
    Ok(codes.and_then(|codes| TapeImage::written(tape.code, tape.size, tape.base, tape.off, tape.tail, tape.widths, codes, None, grammar, source, budget)))
}

/// The inline grammar's tree parser: the lean parser, standing in for tree-sitter-md-inline where it
/// reads the text.
pub struct Lean;

impl TreeParser for Lean {
    fn parse(&self, grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Option<Result<FileImage, TemplateError>> {
        if !grammar.lang.embeds().is_empty() {
            return None;
        }
        let span = crate::code_rule_dsl::logic::profile::span(|| "lean Markdown inline parse and tape".into());
        match encode(grammar, source, &budget) {
            Ok(image) => {
                drop(span);
                crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::SourceByte, source.len() as u64);
                Some(image.map(FileImage::single).map_err(TemplateError::Encode))
            }
            Err(why) => {
                crate::code_rule_dsl::logic::profile::decision("Markdown inline parse", || format!("a text the lean parser refused ({why:?}): tree-sitter-md-inline"));
                None
            }
        }
    }
}
