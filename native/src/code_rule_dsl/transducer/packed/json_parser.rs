//! JSON's parser: the ladder's JSONC grammar (`crates/compiler/bitlex_ladder/15-jsonc.bitlex`, JSON with `//` and
//! `/* */` comments, as tree-sitter-json reads it), compiled once per process through the Bitlex
//! compiler to tier 1 and tier 2 on Cranelift, and shared by every thread.
//!
//! The grammar's kinds, fields and supertype take tree-sitter-json's ids by name, so its node
//! table is the tree-sitter tree's, node for node (`bitlex_json_tests`): the tape of a JSON file is
//! encoded from it (`tape::TapeImage::parse`). A file the grammar refuses (invalid JSON, an empty
//! file, a comment holding a quote) goes to tree-sitter-json.

pub(in crate::code_rule_dsl) use rerail_compiler::native_cpu_stream::{Columns, NO_PARENT};
use super::{bitlex_compile, native_cpu_stream, native_ir, rir};

/// The grammar: the ladder's JSONC step.
const GRAMMAR: &str = include_str!("../../../../crates/compiler/bitlex_ladder/15-jsonc.bitlex");

/// The compiled parser.
struct Compiled {
    parser: native_cpu_stream::Parser,
}

// SAFETY: as `rr_parser::Compiled`: nothing changes a compiled parser once it is made; `parse`
// takes `&self` and writes only buffers it allocates for the call.
unsafe impl Send for Compiled {}
unsafe impl Sync for Compiled {}

static PARSER: std::sync::OnceLock<Result<Compiled, String>> = std::sync::OnceLock::new();

fn compiled() -> Result<&'static Compiled, &'static str> {
    PARSER.get_or_init(compile).as_ref().map_err(String::as_str)
}

/// tree-sitter-json's id of a kind: a named kind's, else an anonymous token's, else a supertype's
/// (named and hidden). `None` for a name it does not have, which the compiler refuses.
fn kind_id(language: &tree_sitter::Language, name: &str) -> Option<u16> {
    if name == "ERROR" {
        return Some(u16::MAX);
    }
    [true, false].into_iter().map(|named| language.id_for_node_kind(name, named)).find(|&id| id != 0)
}

fn compile() -> Result<Compiled, String> {
    use crate::code_rule_dsl::language::GrammarDialect;
    use ast_grep_language::LanguageExt;
    let language = GrammarDialect::Json.get_ts_language();
    let stmts = ::bitlex::syntax::parse(GRAMMAR)?;
    let kind = |n: &str| kind_id(&language, n);
    let field = |n: &str| language.field_id_for_name(n.trim_start_matches("field ")).map(|f| f.get());
    let symbols = bitlex_compile::Symbols { kind: &kind, field: &field };
    let compiled = bitlex_compile::compile(&stmts, &[bitlex_compile::FALLBACK], Some(&symbols))?;
    let program = rir::assemble(&compiled.text())?;
    let native = native_ir::lower(&program).map_err(|e| format!("lowers: {e:?}"))?;
    let parser = native_cpu_stream::Parser::new(&native).map_err(|e| format!("Cranelift: {e:?}"))?;
    Ok(Compiled { parser })
}

/// `src`'s node table, with tree-sitter-json's ids; `None` where the grammar refuses the file (or
/// did not compile: the reason is [`compile_error`]).
pub(in crate::code_rule_dsl) fn columns(src: &str) -> Option<Columns> {
    compiled().ok()?.parser.columns(src.as_bytes()).ok()
}

/// Why the grammar did not compile, if it did not.
pub(in crate::code_rule_dsl) fn compile_error() -> Option<&'static str> {
    compiled().err()
}
