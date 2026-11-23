//! A C or C++ file's tape from the lean parser (`rerail_c_lean`): tree-sitter-c's or
//! tree-sitter-cpp's tape, written as the parse goes, with nothing packed again
//! (`TapeImage::written`). The tape is the one tree-sitter's tree encodes to, node for node
//! (`c_tapes_from_the_lean_parser_are_tree_sitters`, `cpp_tapes_from_the_lean_parser_are_tree_sitters`).
//!
//! One parser reads both languages (`rerail_c_lean::Dialect`). Its keys name kinds, fields and
//! supertypes by their names in tree-sitter-cpp, whose grammar extends tree-sitter-c's
//! (`rerail_c_lean::{KINDS, FIELDS, CLASSES}`); here they are the dialect's ids. A name the
//! dialect's grammar lacks is a kind only the other language writes. A file the parser refuses (any
//! syntax error, which tree-sitter recovers from; a construct it does not read exactly) goes to
//! tree-sitter.

use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::tape::{CODE_EXTRA, Code, FIELD_LIMIT, FileImage, Grammar, KIND_LIMIT, TapeImage, TemplateError, TreeParser};

/// A dialect's ids of the parser's kinds, fields and supertype sets (`None`: not in the dialect's
/// grammar: a kind of the other language, or a token only the grammar's text names).
struct Ids {
    kinds: Vec<Option<u16>>,
    fields: Vec<Option<u16>>,
    classes: Vec<Option<Vec<u16>>>,
}

fn ids_for(dialect: GrammarDialect) -> Ids {
    use ast_grep_language::LanguageExt;
    let language = dialect.get_ts_language();
    let id = |name: &str, named: bool| Some(language.id_for_node_kind(name, named)).filter(|&k| k != 0 && (k as usize) < KIND_LIMIT);
    let kinds = rerail_c_lean::KINDS.iter().map(|&(name, named)| id(name, named)).collect();
    let fields = rerail_c_lean::FIELDS
        .iter()
        .map(|&name| if name.is_empty() { Some(0) } else { language.field_id_for_name(name).map(|f| f.get()).filter(|&f| (f as usize) < FIELD_LIMIT) })
        .collect();
    let classes = rerail_c_lean::CLASSES
        .iter()
        .map(|set| {
            let mut ids = set.iter().map(|name| id(name, true)).collect::<Option<Vec<_>>>()?;
            ids.sort_unstable();
            Some(ids)
        })
        .collect();
    Ids { kinds, fields, classes }
}

fn ids(dialect: rerail_c_lean::Dialect) -> &'static Ids {
    static C: std::sync::OnceLock<Ids> = std::sync::OnceLock::new();
    static CPP: std::sync::OnceLock<Ids> = std::sync::OnceLock::new();
    match dialect {
        rerail_c_lean::Dialect::C => C.get_or_init(|| ids_for(GrammarDialect::C)),
        rerail_c_lean::Dialect::Cpp => CPP.get_or_init(|| ids_for(GrammarDialect::Cpp)),
    }
}

/// The parser's dialect for `grammar`'s language (`None`: neither C nor C++).
fn dialect(grammar: &Grammar) -> Option<rerail_c_lean::Dialect> {
    match grammar.lang {
        GrammarDialect::C => Some(rerail_c_lean::Dialect::C),
        GrammarDialect::Cpp => Some(rerail_c_lean::Dialect::Cpp),
        _ => None,
    }
}

/// `source`'s tape from the lean parser, in `grammar`'s dialect (C or C++): `Err` where it refuses
/// the file; `Ok(Err)` where the tape cannot be made.
pub fn encode(grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Result<Result<TapeImage, String>, rerail_c_lean::Refusal> {
    let Some(dialect) = dialect(grammar) else { return Ok(Err(format!("the lean C parser reads no {:?}", grammar.lang))) };
    let tape = rerail_c_lean::tape(source.as_bytes(), dialect)?;
    let ids = ids(dialect);
    let lacks = |what: &str, i: usize| format!("tree-sitter-{dialect:?} lacks the lean parser's {what} {i}");
    let codes: Result<Vec<Code>, String> = tape
        .keys
        .iter()
        .map(|&key| {
            let (kind, field, class, extra) = rerail_c_lean::unkey(key);
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

/// C's and C++'s tree parser: the lean parser, standing in for tree-sitter where it reads the file.
pub struct Lean;

impl TreeParser for Lean {
    fn parse(&self, grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Option<Result<FileImage, TemplateError>> {
        if !matches!(crate::code_rule_dsl::language::base(crate::code_rule_dsl::language::Family::of(grammar.lang)).parse_input(source), std::borrow::Cow::Borrowed(_))
        {
            return None;
        }
        let dialect = dialect(grammar)?;
        let span = crate::code_rule_dsl::logic::profile::span(|| format!("lean {dialect:?} parse and tape"));
        match encode(grammar, source, &budget) {
            Ok(image) => {
                drop(span);
                crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::SourceByte, source.len() as u64);
                Some(image.map(|image| FileImage::single(image, source)).map_err(TemplateError::Encode))
            }
            Err(why) => {
                crate::code_rule_dsl::logic::profile::decision("C/C++ parse", || format!("a file the lean parser refused ({why:?}): tree-sitter-{dialect:?}"));
                None
            }
        }
    }
}
