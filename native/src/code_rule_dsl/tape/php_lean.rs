//! A PHP file's tape from the lean parser (`rerail_php_lean`): tree-sitter-php's tape (its `php_only`
//! grammar), written as the parse goes, with nothing packed again (`TapeImage::written`). The tape is
//! the one tree-sitter-php's tree encodes to, node for node (`php_tapes_from_the_lean_parser_are_tree_sitters`).
//!
//! The parser reads the file's view (`parse_input`: the text outside `<?php … ?>` blanked), the
//! bytes tree-sitter-php parses; the view keeps the file's length and every offset, so the tape
//! reads the file. The parser's keys name kinds, fields and supertypes by their names in
//! tree-sitter-php (`rerail_php_lean::{KINDS, FIELDS, CLASSES}`); here they are its ids. A file the
//! parser refuses (any syntax error, which tree-sitter recovers from; a construct it does not read)
//! goes to tree-sitter-php.

use crate::code_rule_dsl::tape::{CODE_EXTRA, Code, FIELD_LIMIT, FileImage, Grammar, KIND_LIMIT, TapeImage, TemplateError, TreeParser};

/// tree-sitter-php's ids of the parser's kinds, fields and supertype sets (`None`: not in the
/// language: a token only the grammar's text names, which the parser never writes).
struct Ids {
    kinds: Vec<Option<u16>>,
    fields: Vec<Option<u16>>,
    classes: Vec<Option<Vec<u16>>>,
}

fn ids() -> &'static Ids {
    static IDS: std::sync::OnceLock<Ids> = std::sync::OnceLock::new();
    IDS.get_or_init(|| {
        use crate::code_rule_dsl::language::GrammarDialect;
        use ast_grep_language::LanguageExt;
        let language = GrammarDialect::Php.get_ts_language();
        let id = |name: &str, named: bool| Some(language.id_for_node_kind(name, named)).filter(|&k| k != 0 && (k as usize) < KIND_LIMIT);
        let kinds = rerail_php_lean::KINDS.iter().map(|&(name, named)| id(name, named)).collect();
        let fields = rerail_php_lean::FIELDS
            .iter()
            .map(|&name| if name.is_empty() { Some(0) } else { language.field_id_for_name(name).map(|f| f.get()).filter(|&f| (f as usize) < FIELD_LIMIT) })
            .collect();
        let classes = rerail_php_lean::CLASSES
            .iter()
            .map(|set| {
                let mut ids = set.iter().map(|name| id(name, true)).collect::<Option<Vec<_>>>()?;
                ids.sort_unstable();
                Some(ids)
            })
            .collect();
        Ids { kinds, fields, classes }
    })
}

/// `source`'s tape from the lean parser: `Err` where it refuses the file; `Ok(Err)` where the tape
/// cannot be made.
pub fn encode(grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Result<Result<TapeImage, String>, rerail_php_lean::Refusal> {
    let view = crate::code_rule_dsl::language::parse_input(crate::code_rule_dsl::language::GrammarDialect::Php, source);
    debug_assert_eq!(view.len(), source.len());
    let tape = rerail_php_lean::tape(view.as_bytes())?;
    let ids = ids();
    let lacks = |what: &str, i: usize| format!("tree-sitter-php lacks the lean parser's {what} {i}");
    let codes: Result<Vec<Code>, String> = tape
        .keys
        .iter()
        .map(|&key| {
            let (kind, field, class, extra) = rerail_php_lean::unkey(key);
            let kind = ids.kinds[usize::from(kind)].ok_or_else(|| lacks("kind", usize::from(kind)))?;
            let field = ids.fields[usize::from(field)].ok_or_else(|| lacks("field", usize::from(field)))?;
            let classes: &[u16] = if extra { &[] } else { ids.classes[usize::from(class)].as_deref().ok_or_else(|| lacks("supertype set", usize::from(class)))? };
            let mut code = Code::plain(kind, field, classes)?;
            if extra {
                code.flags |= CODE_EXTRA;
            }
            Ok(code)
        })
        .collect();
    Ok(codes.and_then(|codes| TapeImage::written(tape.code, tape.size, tape.base, tape.off, tape.tail, tape.widths, codes, None, grammar, source, budget)))
}

/// PHP's tree parser: the lean parser, standing in for tree-sitter-php where it reads the file.
pub struct Lean;

impl TreeParser for Lean {
    fn parse(&self, grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Option<Result<FileImage, TemplateError>> {
        if !grammar.lang.embeds().is_empty() {
            return None;
        }
        let span = crate::code_rule_dsl::logic::profile::span(|| "lean PHP parse and tape".into());
        match encode(grammar, source, &budget) {
            Ok(image) => {
                drop(span);
                crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::SourceByte, source.len() as u64);
                Some(image.map(FileImage::single).map_err(TemplateError::Encode))
            }
            Err(why) => {
                crate::code_rule_dsl::logic::profile::decision("PHP parse", || format!("a file the lean parser refused ({why:?}): tree-sitter-php"));
                None
            }
        }
    }
}
