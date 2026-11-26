//! JSON's Bitlex parser: the simdjson form's grammar (`crates/json_parser/json-simdjson.bitlex`), as Rust the Bitlex
//! compiler wrote from it (`rerail_json_aot`, checked in): tier 1 as simdjson's stage 1, tier 2 as its stage 2. It
//! reads the files simdjson refuses for a number past a double's range (`fallback`).
//!
//! Tier 2 writes the engine's tape of the file itself, in format 16 through the writer's cursor as it parses
//! (decisions 0140, 0434 §6: no node table, no conversion), each of its codes numbered in JSON's table by its kind's and
//! field's names in tree-sitter-json, and records each node as it writes it, so the tape's auxiliary index is laid from
//! the record (decision 0703). A file the grammar refuses (invalid JSON, a comment, nesting past 1,024) goes to
//! tree-sitter-json.

/// tree-sitter-json's id of a kind: a named kind's, else an anonymous token's, else a supertype's
/// (named and hidden). `None` for a name it does not have.
fn kind_id(language: &tree_sitter::Language, name: &str) -> Option<u16> {
    if name == "ERROR" {
        return Some(u16::MAX);
    }
    [true, false].into_iter().map(|named| language.id_for_node_kind(name, named)).find(|&id| id != 0)
}

/// JSON's number in `lang` of each of the parser's codes (`rerail_json_aot::ENTRIES`), by their kinds' and fields'
/// names in tree-sitter-json.
fn numbers(lang: &crate::code_rule_dsl::tape::f16::Lang) -> Result<Vec<u32>, String> {
    use crate::code_rule_dsl::language::GrammarDialect;
    use ast_grep_language::LanguageExt;
    let language = GrammarDialect::Json.get_ts_language();
    let name = |table: &[(u16, &'static str)], id: u16| table.iter().find(|&&(i, _)| i == id).map(|&(_, n)| n).ok_or_else(|| format!("no name for the parser's id {id}"));
    let kind = |id: u16| name(rerail_json_aot::KIND_NAMES, id).and_then(|n| kind_id(&language, n).ok_or_else(|| format!("tree-sitter-json has no kind {n}")));
    rerail_json_aot::ENTRIES
        .iter()
        .map(|&(k, f, classes)| {
            let field = match f {
                0 => 0,
                f => name(rerail_json_aot::FIELD_NAMES, f).and_then(|n| language.field_id_for_name(n).map(|f| f.get()).ok_or_else(|| format!("tree-sitter-json has no field {n}")))?,
            };
            let classes = classes.iter().map(|&c| kind(c)).collect::<Result<Vec<u16>, String>>()?;
            let code = crate::code_rule_dsl::tape::Code::plain(kind(k)?, field, &classes)?;
            lang.number(&code.entry()).ok_or_else(|| format!("code {:?} is not in JSON's table", code.entry()))
        })
        .collect()
}

// Its tape (`tape::TreeParser`).
pub mod tape;
// The engine's JSON parser: simdjson's stage 1 (the source hashed in it) and a stage 2 of ours, which
// writes the tape in place (0134). On MSVC its C++ is compiled by clang-cl (`rerail-simdjson-native`).
#[cfg(not(target_family = "wasm"))]
pub mod simdjson;
// What simdjson refuses: the Bitlex parser, then tree-sitter-json past trailing commas.
#[cfg(not(target_family = "wasm"))]
pub mod fallback;

/// `source`'s tape as tier 2 wrote it, in the run's format (16, or 16z), with tree-sitter-json's ids, and the record of
/// its nodes; `None` where the grammar refuses the file. `sha256`: the source's digest where its reader took it.
pub fn image(source: &str, sha256: Option<[u8; 32]>) -> Option<crate::code_rule_dsl::tape::f16::Finished> {
    use crate::code_rule_dsl::language::GrammarDialect;
    use crate::code_rule_dsl::tape::f16;
    let (lang, tag, z) = crate::code_rule_dsl::tape::formats::run_16(GrammarDialect::Json).ok()?;
    let numbers = numbers(lang).ok()?;
    let src = source.as_bytes();
    let doc = f16::Document { lang, source: src, tag, z, blocks: true, nodes: src.len() / 2 + 16, sha256 };
    let mut w = f16::Writer::empty();
    w.begin_with(&doc, |out| crate::code_rule_dsl::lines::index_into(out, src, crate::code_rule_dsl::lines::terminators(GrammarDialect::Json)));
    // The parser reads past the file's end, as simdjson's does: the file padded, as simdjson's shim pads it.
    rerail_json_aot::parse_numbered(&rerail_json_aot::padded(src), src.len(), &mut w, &numbers).ok()?;
    w.finish().ok()
}

/// Why JSON's codes cannot be numbered in its table, if they cannot.
pub fn compile_error() -> Option<String> {
    let (lang, _, _) = match crate::code_rule_dsl::tape::formats::run_16(crate::code_rule_dsl::language::GrammarDialect::Json) {
        Ok(l) => l,
        Err(e) => return Some(e),
    };
    numbers(lang).err()
}
