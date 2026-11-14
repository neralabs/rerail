//! A JavaScript, TypeScript or TSX file's tape from the lean parser (`rerail_ecma_lean`):
//! tree-sitter-javascript's, -typescript's or -tsx's tape, written as the parse goes, with nothing
//! packed again (`TapeImage::written`). The tape is the one the dialect's tree-sitter tree encodes
//! to, node for node (the crate's oracle; `ecma_tapes_from_the_lean_parser_are_tree_sitters`).
//!
//! The parser's keys name kinds, fields and supertypes by their names in the grammars
//! (`rerail_ecma_lean::{KINDS, FIELDS, CLASSES}`), one list for the three; here they are the
//! dialect's ids. A kind one grammar lacks (TypeScript's in JavaScript) has none, and the parser
//! never writes it there. A file the parser refuses (any syntax error, which tree-sitter recovers
//! from; a construct it does not read) goes to tree-sitter.

use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::tape::{CODE_EXTRA, Code, FIELD_LIMIT, FileImage, Grammar, KIND_LIMIT, TapeImage, TemplateError, TreeParser};
use rerail_ecma_lean::Dialect;

/// A dialect's ids of the parser's kinds, fields and supertype sets (`None`: not in its grammar).
struct Ids {
    kinds: Vec<Option<u16>>,
    fields: Vec<Option<u16>>,
    classes: Vec<Option<Vec<u16>>>,
}

fn grammar_dialect(dialect: Dialect) -> GrammarDialect {
    match dialect {
        Dialect::JavaScript => GrammarDialect::JavaScript,
        Dialect::TypeScript => GrammarDialect::TypeScript,
        Dialect::Tsx => GrammarDialect::Tsx,
    }
}

fn ids(dialect: Dialect) -> &'static Ids {
    static IDS: [std::sync::OnceLock<Ids>; 3] = [const { std::sync::OnceLock::new() }; 3];
    IDS[dialect as usize].get_or_init(|| {
        use ast_grep_language::LanguageExt;
        let language = grammar_dialect(dialect).get_ts_language();
        let id = |name: &str, named: bool| Some(language.id_for_node_kind(name, named)).filter(|&k| k != 0 && (k as usize) < KIND_LIMIT);
        let kinds = rerail_ecma_lean::KINDS.iter().map(|&(name, named)| id(name, named)).collect();
        let fields = rerail_ecma_lean::FIELDS
            .iter()
            .map(|&name| if name.is_empty() { Some(0) } else { language.field_id_for_name(name).map(|f| f.get()).filter(|&f| (f as usize) < FIELD_LIMIT) })
            .collect();
        let classes = rerail_ecma_lean::CLASSES
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

/// `source`'s tape from the lean parser in `dialect`: `Err` where it refuses the file; `Ok(Err)`
/// where the tape cannot be made.
pub fn encode(dialect: Dialect, grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Result<Result<TapeImage, String>, rerail_ecma_lean::Refusal> {
    let tape = rerail_ecma_lean::tape(source.as_bytes(), dialect)?;
    let ids = ids(dialect);
    let lacks = |what: &str, i: usize| format!("{dialect:?}'s grammar lacks the lean parser's {what} {i}");
    let codes: Result<Vec<Code>, String> = tape
        .keys
        .iter()
        .map(|&key| {
            let (kind, field, class, extra) = rerail_ecma_lean::unkey(key);
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

/// A dialect's tree parser: the lean parser, standing in for tree-sitter where it reads the file.
pub struct Lean(pub Dialect);

pub static JAVASCRIPT: Lean = Lean(Dialect::JavaScript);
pub static TYPESCRIPT: Lean = Lean(Dialect::TypeScript);
pub static TSX: Lean = Lean(Dialect::Tsx);

impl TreeParser for Lean {
    fn parse(&self, grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Option<Result<FileImage, TemplateError>> {
        if !grammar.lang.embeds().is_empty()
            || !matches!(crate::code_rule_dsl::language::base(crate::code_rule_dsl::language::Family::of(grammar.lang)).parse_input(source), std::borrow::Cow::Borrowed(_))
        {
            return None;
        }
        let span = crate::code_rule_dsl::logic::profile::span(|| format!("lean {:?} parse and tape", self.0));
        match encode(self.0, grammar, source, &budget) {
            Ok(image) => {
                drop(span);
                crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::SourceByte, source.len() as u64);
                Some(image.map(FileImage::single).map_err(TemplateError::Encode))
            }
            Err(why) => {
                crate::code_rule_dsl::logic::profile::decision("ECMAScript parse", || format!("a file the lean parser refused ({why:?}): tree-sitter-{:?}", self.0));
                None
            }
        }
    }
}
