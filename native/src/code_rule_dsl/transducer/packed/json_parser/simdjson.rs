//! A JSON file's tape from simdjson (`rerail_simdjson`, the C++ library): its DOM parser validates the
//! whole file, and the tape's columns are written straight from its structural index
//! (`rerail_simdjson::tape`), tree-sitter-json's nodes in preorder. Nothing is packed again
//! (`TapeImage::written`). The tape is the one tree-sitter-json's tree encodes to, byte for byte
//! (`json_tapes_from_simdjson_are_tree_sitters`).
//!
//! simdjson reads strict JSON. A file it refuses (comments, a trailing comma, a number past its range,
//! nesting past 1,024, invalid JSON) goes to tree-sitter-json.
use crate::code_rule_dsl::tape::{Code, FIELD_LIMIT, FileImage, Grammar, KIND_LIMIT, TapeImage, TemplateError, TreeParser};
pub use rerail_simdjson::Refusal;

/// tree-sitter-json's ids of the tree's kinds and fields, and of the `_value` supertype.
pub fn kinds() -> Option<&'static (rerail_simdjson::Kinds, u16)> {
    static KINDS: std::sync::OnceLock<Option<(rerail_simdjson::Kinds, u16)>> = std::sync::OnceLock::new();
    KINDS
        .get_or_init(|| {
            use crate::code_rule_dsl::language::GrammarDialect;
            use ast_grep_language::LanguageExt;
            let language = GrammarDialect::Json.get_ts_language();
            let kind = |n: &str| super::kind_id(&language, n).filter(|&k| (k as usize) < KIND_LIMIT && k != 0);
            let field = |n: &str| language.field_id_for_name(n).map(|f| f.get()).filter(|&f| (f as usize) < FIELD_LIMIT);
            let kinds = rerail_simdjson::Kinds {
                document: kind("document")?,
                object: kind("object")?,
                pair: kind("pair")?,
                array: kind("array")?,
                string: kind("string")?,
                string_content: kind("string_content")?,
                escape_sequence: kind("escape_sequence")?,
                number: kind("number")?,
                true_: kind("true")?,
                false_: kind("false")?,
                null: kind("null")?,
                key_field: field("key")?,
                value_field: field("value")?,
            };
            Some((kinds, kind("_value")?))
        })
        .as_ref()
}

/// `source`'s tape, as simdjson's writer wrote its columns (`rerail_simdjson::tape`): `Err` where simdjson
/// refuses the file; `Ok(Err)` where the tape cannot be made.
pub fn encode(grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Result<Result<TapeImage, String>, Refusal> {
    let Some((kinds, value)) = kinds() else {
        return Ok(Err("tree-sitter-json lacks a kind or field simdjson's tape names".into()));
    };
    let t = rerail_simdjson::tape(source.as_bytes(), kinds)?;
    let codes: Result<Vec<Code>, String> =
        t.codes.iter().map(|&(kind, field, in_value)| Code::plain(kind, field, if in_value { std::slice::from_ref(value) } else { &[] })).collect();
    Ok(codes.and_then(|codes| TapeImage::written(t.code, t.size, t.base, t.off, t.tail, t.widths, codes, grammar, source, budget)))
}

/// JSON's tree parser: simdjson, standing in for tree-sitter-json.
pub struct Simdjson;

impl TreeParser for Simdjson {
    fn parse(&self, grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Option<Result<FileImage, TemplateError>> {
        if !grammar.lang.embeds().is_empty()
            || !matches!(crate::code_rule_dsl::language::base(crate::code_rule_dsl::language::Family::of(grammar.lang)).parse_input(source), std::borrow::Cow::Borrowed(_))
        {
            return None;
        }
        let span = crate::code_rule_dsl::logic::profile::span(|| "simdjson parse and tape".into());
        match encode(grammar, source, &budget) {
            Ok(image) => {
                drop(span);
                crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::SourceByte, source.len() as u64);
                Some(image.map(FileImage::single).map_err(TemplateError::Encode))
            }
            Err(why) => {
                crate::code_rule_dsl::logic::profile::decision("JSON parse", || format!("a file simdjson refused ({why}): tree-sitter-json"));
                None
            }
        }
    }
}
