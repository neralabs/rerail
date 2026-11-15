//! A Markdown file's tape from the lean parser (`rerail_markdown_lean`): tree-sitter-md's block
//! tape, written as the parse goes, with nothing packed again (`TapeImage::written`). The tape is
//! the one tree-sitter-md's tree encodes to, node for node
//! (`markdown_tapes_from_the_lean_parser_are_tree_sitters`).
//!
//! The parser's keys name kinds and fields by their names in tree-sitter-md
//! (`rerail_markdown_lean::{KINDS, FIELDS}`); here they are its ids. A file the parser refuses (any
//! syntax error, which tree-sitter recovers from; a construct it does not read) goes to
//! tree-sitter-md.

use crate::code_rule_dsl::tape::{CODE_EXTRA, Code, FIELD_LIMIT, FileImage, Grammar, KIND_LIMIT, TapeImage, TemplateError, TreeParser};

/// tree-sitter-md's ids of the parser's kinds and fields (`None`: not in the language).
struct Ids {
    kinds: Vec<Option<u16>>,
    fields: Vec<Option<u16>>,
}

fn ids() -> &'static Ids {
    static IDS: std::sync::OnceLock<Ids> = std::sync::OnceLock::new();
    IDS.get_or_init(|| {
        use crate::code_rule_dsl::language::GrammarDialect;
        use ast_grep_language::LanguageExt;
        let language = GrammarDialect::Markdown.get_ts_language();
        let kinds = rerail_markdown_lean::KINDS
            .iter()
            .map(|&(name, named)| Some(language.id_for_node_kind(name, named)).filter(|&k| k != 0 && (k as usize) < KIND_LIMIT))
            .collect();
        let fields = rerail_markdown_lean::FIELDS
            .iter()
            .map(|&name| if name.is_empty() { Some(0) } else { language.field_id_for_name(name).map(|f| f.get()).filter(|&f| (f as usize) < FIELD_LIMIT) })
            .collect();
        Ids { kinds, fields }
    })
}

/// `source`'s tape from the lean parser: `Err` where it refuses the file; `Ok(Err)` where the tape
/// cannot be made.
pub fn encode(grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Result<Result<TapeImage, String>, rerail_markdown_lean::Refusal> {
    let tape = rerail_markdown_lean::tape(source.as_bytes())?;
    let ids = ids();
    let lacks = |what: &str, i: usize| format!("tree-sitter-md lacks the lean parser's {what} {i}");
    let codes: Result<Vec<Code>, String> = tape
        .keys
        .iter()
        .map(|&key| {
            let (kind, field, _, extra) = rerail_markdown_lean::unkey(key);
            let kind = ids.kinds[usize::from(kind)].ok_or_else(|| lacks("kind", usize::from(kind)))?;
            let field = ids.fields[usize::from(field)].ok_or_else(|| lacks("field", usize::from(field)))?;
            let mut code = Code::plain(kind, field, &[])?;
            if extra {
                code.flags |= CODE_EXTRA;
            }
            Ok(code)
        })
        .collect();
    Ok(codes.and_then(|codes| TapeImage::written(tape.code, tape.size, tape.base, tape.off, tape.tail, tape.widths, codes, None, grammar, source, budget)))
}

/// Markdown's tree parser: the lean parser, standing in for tree-sitter-md where it reads the file.
pub struct Lean;

impl TreeParser for Lean {
    fn parse(&self, grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Option<Result<FileImage, TemplateError>> {
        if !grammar.lang.embeds().is_empty() {
            return None;
        }
        let span = crate::code_rule_dsl::logic::profile::span(|| "lean Markdown parse and tape".into());
        match encode(grammar, source, &budget) {
            Ok(image) => {
                drop(span);
                crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::SourceByte, source.len() as u64);
                Some(image.map(FileImage::single).map_err(TemplateError::Encode))
            }
            Err(why) => {
                crate::code_rule_dsl::logic::profile::decision("Markdown parse", || format!("a file the lean parser refused ({why:?}): tree-sitter-md"));
                None
            }
        }
    }
}
