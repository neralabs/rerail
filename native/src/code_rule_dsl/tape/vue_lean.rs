//! A Vue file's tape from the lean parser (`rerail_vue_lean`): tree-sitter-vue's tape, written as
//! the parse goes, with nothing packed again (`formats::lean`: format 15's columns, or format 16 written in one pass). The tape is the one
//! tree-sitter-vue's tree encodes to, node for node (`vue_tapes_from_the_lean_parser_are_tree_sitters`).
//! Its embedded program (the component's scripts and template expressions, `script-block`) is
//! built from the tape (`FileImage::embedding`), as it is from tree-sitter's.
//!
//! The parser's keys name kinds by their names in tree-sitter-vue (`rerail_vue_lean::KINDS`); here
//! they are its ids. A file the parser refuses (any syntax error, which tree-sitter recovers from;
//! a construct it does not read) goes to tree-sitter-vue.

use crate::code_rule_dsl::tape::{CODE_EXTRA, Code, FileImage, Grammar, KIND_LIMIT, TapeImage, TemplateError, TreeParser};

/// tree-sitter-vue's ids of the parser's kinds (`None`: not in the language).
fn kind_ids() -> &'static [Option<u16>] {
    static IDS: std::sync::OnceLock<Vec<Option<u16>>> = std::sync::OnceLock::new();
    IDS.get_or_init(|| {
        use crate::code_rule_dsl::language::GrammarDialect;
        use ast_grep_language::LanguageExt;
        let language = GrammarDialect::Vue.get_ts_language();
        rerail_vue_lean::KINDS
            .iter()
            .map(|&(name, named)| Some(language.id_for_node_kind(name, named)).filter(|&k| k != 0 && (k as usize) < KIND_LIMIT))
            .collect()
    })
}

/// `source`'s tape from the lean parser: `Err` where it refuses the file; `Ok(Err)` where the tape
/// cannot be made.
pub fn encode(grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Result<Result<TapeImage, String>, rerail_vue_lean::Refusal> {
    let ids = kind_ids();
    let code_of = move |&key: &u32| -> Result<Code, String> {
        let (kind, _, _, extra) = rerail_vue_lean::unkey(key);
        let kind = ids[usize::from(kind)].ok_or_else(|| format!("tree-sitter-vue lacks the lean parser's kind {kind}"))?;
        let mut code = Code::plain(kind, 0, &[])?;
        if extra {
            code.flags |= CODE_EXTRA;
        }
        Ok(code)
    };
    rerail_vue_lean::tape_into(source.as_bytes(), &mut super::formats::lean(grammar, source, budget, code_of))
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
