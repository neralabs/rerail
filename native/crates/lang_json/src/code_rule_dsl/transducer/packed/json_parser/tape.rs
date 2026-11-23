//! A JSON file's tape as the Bitlex parser's tier 2 writes it, its bytes (`json_parser::image`), tree-sitter-json's.
//! `Bitlex` is a `tape::TreeParser`, though not the engine's: JSON is read with simdjson (`json_parser::simdjson`, 0134).
use crate::code_rule_dsl::tape::{FileImage, Grammar, TapeImage, TemplateError, TreeParser};

/// JSON's tree parser: the Bitlex parser, standing in for tree-sitter-json.
pub struct Bitlex;

impl TreeParser for Bitlex {
    fn parse(&self, grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Option<Result<FileImage, TemplateError>> {
        if !matches!(crate::code_rule_dsl::language::base(crate::code_rule_dsl::language::Family::of(grammar.lang)).parse_input(source), std::borrow::Cow::Borrowed(_))
        {
            return None;
        }
        let parse = crate::code_rule_dsl::logic::profile::span(|| "bitlex parse".into());
        let image = super::image(source);
        drop(parse);
        let Some(image) = image else {
            crate::code_rule_dsl::logic::profile::decision("JSON parse", || "a file the Bitlex parser refused: tree-sitter-json".to_owned());
            return None;
        };
        crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::SourceByte, source.len() as u64);
        // The tape is tier 2's, as it wrote it: nothing is packed again (decisions 0140, 0434 §6).
        Some(TapeImage::written_16(image, grammar.lang, source, None, &budget).map(|image| FileImage::single(image, source)).map_err(TemplateError::Encode))
    }
}
