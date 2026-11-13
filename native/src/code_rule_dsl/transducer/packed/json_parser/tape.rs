//! A JSON file's tape from what the Bitlex parser's tier 2 wrote (`json_parser::Written`), node for node tree-sitter-json's.
//! `Bitlex` is a `tape::TreeParser`, though not the engine's: JSON is read with simdjson (`json_parser::simdjson`, 0134).
use super::Written;
use crate::code_rule_dsl::tape::wire::CHECKPOINT_STRIDE;
use crate::code_rule_dsl::tape::{
    ERROR_CODE, FIELD_LIMIT, FileImage, Grammar, KIND_LIMIT, NONE, Pending, TapeImage, TemplateError, TreeParser, checked,
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
        let written = super::written(source);
        drop(parse);
        let Some(written) = written else {
            crate::code_rule_dsl::logic::profile::decision("JSON parse", || "a file the Bitlex parser refused: tree-sitter-json".to_owned());
            return None;
        };
        crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::SourceByte, source.len() as u64);
        let _encode = crate::code_rule_dsl::logic::profile::span(|| "tape encode".into());
        Some(encode_written(grammar, source, &written, &budget).map(FileImage::single).map_err(TemplateError::Encode))
    }
}

/// The tape of what the Bitlex parser's tier 2 wrote (`json_parser::Written`): its image's nodes
/// (codes, sizes, spans, read with the engine's readers, `tape::resident`) with the supertypes
/// recorded beside it, in the grammar's tree-sitter ids, as `encode_tree` reads them off a
/// tree-sitter tree. The image's codes carry no supertype classes, so the tape is made again
/// with them (`TapeImage::finish`).
pub fn encode_written(grammar: &Grammar, source: &str, written: &Written, budget: &impl Fn() -> bool) -> Result<TapeImage, String> {
    use crate::code_rule_dsl::tape::resident;
    checked(budget)?;
    if source.len() > u32::MAX as usize {
        return Err("source exceeds tape range".into());
    }
    let bytes = &written.image;
    let layout = resident::layout(bytes)?;
    let codes = resident::device_codes(bytes, &layout, grammar)?;
    let n = layout.nodes as usize;
    let mut parents = vec![0; n];
    resident::parents(bytes, &layout, &mut parents)?;
    let [_, wo, wt] = layout.widths;
    let starts: Vec<u32> = (0..n)
        .map(|i| resident::cell_at(bytes, layout.base, 4, i / CHECKPOINT_STRIDE) + resident::cell_at(bytes, layout.off, wo, i))
        .collect();
    let mut nodes = Vec::with_capacity(n);
    for i in 0..n {
        let code = codes.get(resident::cell_at(bytes, layout.code, 2, i) as usize).ok_or("a node's code past the table")?;
        let (extra, error) = (code.flags & 2 != 0, code.flags & 8 != 0);
        let (kind, field) = (u32::from(code.kind), u32::from(code.field));
        if !error && (kind as usize >= KIND_LIMIT || field as usize >= FIELD_LIMIT || kind == u32::from(ERROR_CODE)) {
            return Err(format!("kind {kind} or field {field} exceeds the tape's codes"));
        }
        let size = resident::size_at(bytes, &layout, i);
        // A tail is measured from the node's own start, or from its last descendant's.
        let from = if layout.from_start { starts[i] } else { starts[i + size as usize - 1] };
        let end = from.checked_add(resident::cell_at(bytes, layout.tail, wt, i)).ok_or("a node's end past the source")?;
        // An extra hangs from no field, as `encode_tree` leaves it.
        let field = if extra { 0 } else { field as u16 };
        nodes.push(Pending {
            kf: if error { ERROR_CODE } else { kind as u16 | (field << 10) },
            parent: (parents[i] != NONE).then_some(parents[i]),
            start: starts[i],
            length: end.checked_sub(starts[i]).ok_or("inverted node span")?,
            size: 1,
            extra,
            error,
            missing: false,
            kind_alias: None,
        });
    }
    let mut supertypes = Vec::with_capacity(written.supers.len());
    for &(node, class) in &written.supers {
        if class as usize >= KIND_LIMIT {
            return Err("supertype symbol exceeds tape limit".into());
        }
        supertypes.push((node, class));
    }
    TapeImage::finish(nodes, supertypes, grammar, source, budget)
}
