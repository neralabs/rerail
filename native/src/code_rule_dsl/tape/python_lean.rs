//! A Python file's tape from the lean parser (`rerail_python_lean`): tree-sitter-python's tape,
//! written as the parse goes, with nothing packed again (`TapeImage::written`). The tape is the one
//! tree-sitter-python's tree encodes to, node for node (`python_tapes_from_the_lean_parser_are_tree_sitters`).
//!
//! The parser's keys name kinds, fields and supertypes by their names in tree-sitter-python
//! (`rerail_python_lean::{KINDS, FIELDS, CLASSES}`); here they are its ids. A file the parser refuses
//! (any syntax error, the text's end included, which tree-sitter recovers from; a construct it does
//! not read) goes to tree-sitter-python (decision 0163, amended).

use crate::code_rule_dsl::tape::{CODE_EXTRA, Code, ERROR_CODE, FIELD_LIMIT, FileImage, Grammar, KIND_LIMIT, TapeImage, TemplateError, TreeParser};

/// tree-sitter-python's ids of the parser's kinds, fields and supertype sets.
struct Ids {
    kinds: Vec<u16>,
    fields: Vec<u16>,
    classes: Vec<Vec<u16>>,
}

fn ids() -> Option<&'static Ids> {
    static IDS: std::sync::OnceLock<Option<Ids>> = std::sync::OnceLock::new();
    IDS.get_or_init(|| {
        use crate::code_rule_dsl::language::GrammarDialect;
        use ast_grep_language::LanguageExt;
        let language = GrammarDialect::Python.get_ts_language();
        let id = |name: &str, named: bool| Some(language.id_for_node_kind(name, named)).filter(|&k| k != 0 && (k as usize) < KIND_LIMIT);
        // `ERROR` is tree-sitter's error symbol, past the grammar's: the tape's own code.
        let kinds = rerail_python_lean::KINDS.iter().map(|&(name, named)| if name == "ERROR" { Some(ERROR_CODE) } else { id(name, named) }).collect::<Option<Vec<_>>>()?;
        let fields = rerail_python_lean::FIELDS
            .iter()
            .map(|&name| if name.is_empty() { Some(0) } else { language.field_id_for_name(name).map(|f| f.get()).filter(|&f| (f as usize) < FIELD_LIMIT) })
            .collect::<Option<Vec<_>>>()?;
        let classes = rerail_python_lean::CLASSES
            .iter()
            .map(|set| {
                let mut ids = set.iter().map(|name| id(name, true)).collect::<Option<Vec<_>>>()?;
                ids.sort_unstable();
                Some(ids)
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Ids { kinds, fields, classes })
    })
    .as_ref()
}

/// `source`'s tape from the lean parser: `Err` where it refuses the file; `Ok(Err)` where the tape
/// cannot be made.
pub fn encode(grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Result<Result<TapeImage, String>, rerail_python_lean::Refusal> {
    let tape = rerail_python_lean::tape(source.as_bytes())?;
    let Some(ids) = ids() else {
        return Ok(Err("tree-sitter-python lacks a kind or field the lean parser names".into()));
    };
    let codes: Result<Vec<Code>, String> = tape
        .keys
        .iter()
        .map(|&key| {
            let (kind, field, class, extra) = rerail_python_lean::unkey(key);
            let kind = ids.kinds[usize::from(kind)];
            let mut code = if kind == ERROR_CODE { Code::error(ids.fields[usize::from(field)])? } else { Code::plain(kind, ids.fields[usize::from(field)], &ids.classes[usize::from(class)])? };
            if extra {
                code.flags |= CODE_EXTRA;
            }
            Ok(code)
        })
        .collect();
    Ok(codes.and_then(|codes| TapeImage::written(tape.code, tape.size, tape.base, tape.off, tape.tail, tape.widths, codes, None, grammar, source, budget)))
}

/// Python's tree parser: the lean parser, standing in for tree-sitter-python where it reads the file.
pub struct Lean;

impl TreeParser for Lean {
    fn parse(&self, grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Option<Result<FileImage, TemplateError>> {
        if !grammar.lang.embeds().is_empty()
            || !matches!(crate::code_rule_dsl::language::base(crate::code_rule_dsl::language::Family::of(grammar.lang)).parse_input(source), std::borrow::Cow::Borrowed(_))
        {
            return None;
        }
        let span = crate::code_rule_dsl::logic::profile::span(|| "lean Python parse and tape".into());
        match encode(grammar, source, &budget) {
            Ok(image) => {
                drop(span);
                crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::SourceByte, source.len() as u64);
                Some(image.map(FileImage::single).map_err(TemplateError::Encode))
            }
            Err(why) => {
                crate::code_rule_dsl::logic::profile::decision("Python parse", || format!("a file the lean parser refused ({why:?}): tree-sitter-python"));
                None
            }
        }
    }
}
