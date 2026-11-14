//! A JSON file's tape from the Bitlex parser's node table (`json_parser::Columns`), node for node tree-sitter-json's.
//! `Bitlex` is a `tape::TreeParser`, though not the engine's: JSON is read with simdjson (`json_parser::simdjson`, 0134).
use super::{Columns, NO_PARENT};
use crate::code_rule_dsl::tape::{
    ERROR_CODE, FIELD_LIMIT, FileImage, Grammar, KIND_LIMIT, Pending, TapeImage, TemplateError, TreeParser, checked,
};

/// JSON's tree parser: the Bitlex parser, standing in for tree-sitter-json.
pub struct Bitlex;

impl TreeParser for Bitlex {
    fn parse(&self, grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Option<Result<FileImage, TemplateError>> {
        if !grammar.lang.embeds().is_empty()
            || !matches!(crate::code_rule_dsl::language::base(crate::code_rule_dsl::language::Family::of(grammar.lang)).parse_input(source), std::borrow::Cow::Borrowed(_))
        {
            return None;
        }
        let parse = crate::code_rule_dsl::logic::profile::span(|| "bitlex parse".into());
        let columns = super::columns(source);
        drop(parse);
        let Some(columns) = columns else {
            crate::code_rule_dsl::logic::profile::decision("JSON parse", || "a file the Bitlex parser refused: tree-sitter-json".to_owned());
            return None;
        };
        crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::SourceByte, source.len() as u64);
        let _encode = crate::code_rule_dsl::logic::profile::span(|| "tape encode".into());
        Some(encode_columns(grammar, source, &columns, &budget).map(FileImage::single).map_err(TemplateError::Encode))
    }
}

/// The tape of a node table a Bitlex parser wrote (`transducer::packed::json_parser::Columns`), in the
/// grammar's tree-sitter ids: kinds, fields, spans, parents, extras and supertypes, as `encode_tree` reads them
/// off a tree-sitter tree.
pub fn encode_columns(
    grammar: &Grammar,
    source: &str,
    columns: &Columns,
    budget: &impl Fn() -> bool,
) -> Result<TapeImage, String> {
    checked(budget)?;
    if source.len() > u32::MAX as usize {
        return Err("source exceeds tape range".into());
    }
    let mut nodes = Vec::with_capacity(columns.rows.len());
    for r in &columns.rows {
        let (kind, field, extra) = (u32::from(r.kind), u32::from(r.field), r.extra != 0);
        let error = kind == u32::from(u16::MAX);
        if !error && (kind as usize >= KIND_LIMIT || field as usize >= FIELD_LIMIT || kind == u32::from(ERROR_CODE)) {
            return Err(format!("kind {kind} or field {field} exceeds the tape's codes"));
        }
        // An extra hangs from no field, as `encode_tree` leaves it.
        let field = if extra { 0 } else { field as u16 };
        nodes.push(Pending {
            kf: if error { ERROR_CODE } else { kind as u16 | (field << 10) },
            parent: (r.parent != NO_PARENT).then_some(r.parent),
            start: r.start,
            length: r.end.checked_sub(r.start).ok_or("inverted node span")?,
            size: 1,
            extra,
            error,
            missing: false,
            kind_alias: None,
        });
    }
    let mut supertypes = Vec::with_capacity(columns.super_node.len());
    for (&node, &class) in columns.super_node.iter().zip(&columns.super_kind) {
        if class as usize >= KIND_LIMIT {
            return Err("supertype symbol exceeds tape limit".into());
        }
        supertypes.push((node, class as u16));
    }
    TapeImage::finish(nodes, supertypes, grammar, source, budget)
}
