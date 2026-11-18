//! A Swift file's tape from the lean parser (`rerail_swift_lean`): tree-sitter-swift's tape,
//! written as the parse goes, with nothing packed again (`TapeImage::written`). The tape is the one
//! tree-sitter-swift's tree encodes to, node for node (`swift_tapes_from_the_lean_parser_are_tree_sitters`).
//!
//! The parser's keys name kinds, fields and supertypes by their names in tree-sitter-swift
//! (`rerail_swift_lean::{KINDS, FIELDS, CLASSES}`); here they are its ids. A file the parser refuses
//! (any syntax error, which tree-sitter recovers from; a construct it does not read) goes to
//! tree-sitter-swift.

use crate::code_rule_dsl::tape::{CODE_EXTRA, Code, FIELD_LIMIT, FileImage, Grammar, KIND_LIMIT, TapeImage, TemplateError, TreeParser};

/// tree-sitter-swift's ids of the parser's kinds, fields and supertype sets (`None`: not in the
/// language: a token only the grammar's text names, which the parser never writes).
struct Ids {
    kinds: Vec<Option<u16>>,
    /// Per kind, the contextual alias tree-sitter-swift's tape gives it: where the symbol its name maps to
    /// is spelled or named otherwise (as `encode_node` finds it).
    aliases: Vec<Option<(String, bool)>>,
    fields: Vec<Option<u16>>,
    classes: Vec<Option<Vec<u16>>>,
}

fn ids() -> &'static Ids {
    static IDS: std::sync::OnceLock<Ids> = std::sync::OnceLock::new();
    IDS.get_or_init(|| {
        use crate::code_rule_dsl::language::GrammarDialect;
        use ast_grep_language::LanguageExt;
        let language = GrammarDialect::Swift.get_ts_language();
        let id = |name: &str, named: bool| Some(language.id_for_node_kind(name, named)).filter(|&k| k != 0 && (k as usize) < KIND_LIMIT);
        let kinds: Vec<Option<u16>> = rerail_swift_lean::KINDS.iter().map(|&(name, named)| id(name, named)).collect();
        let aliases = rerail_swift_lean::KINDS
            .iter()
            .zip(&kinds)
            .map(|(&(name, named), &kind)| {
                let kind = kind?;
                (language.node_kind_for_id(kind) != Some(name) || language.node_kind_is_named(kind) != named).then(|| (name.to_owned(), named))
            })
            .collect();
        let fields = rerail_swift_lean::FIELDS
            .iter()
            .map(|&name| if name.is_empty() { Some(0) } else { language.field_id_for_name(name).map(|f| f.get()).filter(|&f| (f as usize) < FIELD_LIMIT) })
            .collect();
        let classes = rerail_swift_lean::CLASSES
            .iter()
            .map(|set| {
                let mut ids = set.iter().map(|name| id(name, true)).collect::<Option<Vec<_>>>()?;
                ids.sort_unstable();
                Some(ids)
            })
            .collect();
        Ids { kinds, aliases, fields, classes }
    })
}

/// `source`'s tape from the lean parser: `Err` where it refuses the file; `Ok(Err)` where the tape
/// cannot be made.
pub fn encode(grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Result<Result<TapeImage, String>, rerail_swift_lean::Refusal> {
    let tape = rerail_swift_lean::tape(source.as_bytes())?;
    let ids = ids();
    let lacks = |what: &str, i: usize| format!("tree-sitter-swift lacks the lean parser's {what} {i}");
    // The aliases by first use: the codes are numbered by first use, so in code order.
    let mut aliases: Vec<(String, bool)> = Vec::new();
    let codes: Result<Vec<Code>, String> = tape
        .keys
        .iter()
        .map(|&key| {
            let (kind, field, class, extra) = rerail_swift_lean::unkey(key);
            let alias = &ids.aliases[usize::from(kind)];
            let kind = ids.kinds[usize::from(kind)].ok_or_else(|| lacks("kind", usize::from(kind)))?;
            let field = ids.fields[usize::from(field)].ok_or_else(|| lacks("field", usize::from(field)))?;
            let classes: &[u16] = if extra { &[] } else { ids.classes[usize::from(class)].as_deref().ok_or_else(|| lacks("supertype set", usize::from(class)))? };
            let mut code = Code::plain(kind, field, classes)?;
            if extra {
                code.flags |= CODE_EXTRA;
            }
            if let Some(alias) = alias {
                let at = aliases.iter().position(|a| a == alias).unwrap_or_else(|| {
                    aliases.push(alias.clone());
                    aliases.len() - 1
                });
                code.alias = u16::try_from(at).map_err(|_| "too many contextual aliases")?;
            }
            Ok(code)
        })
        .collect();
    Ok(codes.and_then(|codes| {
        TapeImage::written(tape.code, tape.size, tape.base, tape.off, tape.tail, tape.widths, codes, None, grammar, source, budget)?.with_kind_aliases(aliases)
    }))
}

/// Swift's tree parser: the lean parser, standing in for tree-sitter-swift where it reads the file.
pub struct Lean;

impl TreeParser for Lean {
    fn parse(&self, grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Option<Result<FileImage, TemplateError>> {
        if !grammar.lang.embeds().is_empty()
            || !matches!(crate::code_rule_dsl::language::base(crate::code_rule_dsl::language::Family::of(grammar.lang)).parse_input(source), std::borrow::Cow::Borrowed(_))
        {
            return None;
        }
        let span = crate::code_rule_dsl::logic::profile::span(|| "lean Swift parse and tape".into());
        match encode(grammar, source, &budget) {
            Ok(image) => {
                drop(span);
                crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::SourceByte, source.len() as u64);
                Some(image.map(|image| FileImage::single(image, source)).map_err(TemplateError::Encode))
            }
            Err(why) => {
                crate::code_rule_dsl::logic::profile::decision("Swift parse", || format!("a file the lean parser refused ({why:?}): tree-sitter-swift"));
                None
            }
        }
    }
}
