//! `call-argument`: the strings a language's registry entry names as code by their place (decision 0470), each a
//! program of its file. Python's, for HTML:
//!
//! ```text
//! from markupsafe import Markup
//! Markup("<b>x</b>")                      HTML: the callee resolves to `markupsafe.Markup`, argument 0
//! format_html('<a href="{}">{}</a>', u)   HTML, where `format_html` is `django.utils.html.format_html`
//! Markup(f"<b>{x}</b>")                   no program: an f-string's value is not its text
//! ```
//!
//! A call is at a place where its callee resolves, in the file's own scope table (Ruff's `resolve_qualified_name`),
//! to the place's qualified name. Its positional argument there is a program where it is a plain string: no
//! interpolation, no bytes, no escape (an implicit concatenation of such strings is one program). The program's text
//! is the strings' content, each part mapped to its bytes in the file, so its nodes stand at the file's lines.
//!
//! The embedder reads the host through the scope table, so it needs the file's tape document: the batch's, over the
//! host's tape (`TapeDoc::over`, nothing copied). A file whose text names no place's last segment builds no table.

use super::segments::SegmentMap;
use super::{Embedder, Program};
use crate::code_rule_dsl::language::{GrammarDialect, Via};
use crate::code_rule_dsl::scope::{Reading, ScopeTable};
use crate::code_rule_dsl::tape::TapeDoc;
use crate::code_rule_dsl::tree::Tree as _;
use ast_grep_core::{AstGrep, Doc, Node};

/// The embedder of `Via::CallArgument`.
pub struct CallArgument;

impl Embedder for CallArgument {
    fn build(&self, _root: super::host::HostNode<'_>, _source: &str, _budget: &dyn Fn() -> bool) -> Option<Program> {
        None
    }

    fn per_place(&self) -> bool {
        true
    }

    fn reads_scope(&self) -> bool {
        true
    }

    fn build_in_doc(&self, doc: &TapeDoc, budget: &dyn Fn() -> bool) -> Vec<Program> {
        let host = doc.grammar().lang;
        let places: Vec<(GrammarDialect, Vec<&str>, usize)> = host
            .embed_places()
            .iter()
            .filter(|(via, ..)| *via == Via::CallArgument)
            .map(|&(_, dialect, callee, argument)| (dialect, callee.split('.').collect(), argument as usize))
            .collect();
        // A file whose text names no place's callee holds none: no scope table is built for it.
        let source = doc.source.as_bytes();
        if !places.iter().any(|(_, callee, _)| callee.last().is_some_and(|last| memchr::memmem::find(source, last.as_bytes()).is_some())) {
            return Vec::new();
        }
        let grep = AstGrep::doc(doc.clone());
        let Ok(table) = ScopeTable::build_in(grep.root(), host, usize::MAX, budget, Reading::default(), None) else { return Vec::new() };
        let mut programs = Vec::new();
        let mut stack = vec![grep.root()];
        while let Some(node) = stack.pop() {
            if budget() {
                return Vec::new();
            }
            stack.extend(node.named());
            if node.kind() != "call" {
                continue;
            }
            let Some(callee) = node.field("function") else { continue };
            let Some(name) = crate::code_rule_dsl::scope::python::typedefs::qualified_name(&table, None, &callee) else { continue };
            for (dialect, place, argument) in &places {
                if name.len() != place.len() || !name.iter().zip(place).all(|(a, b)| a == b) {
                    continue;
                }
                let Some(value) = node.field("arguments").and_then(|args| positional(&args, *argument)) else { continue };
                if let Some(program) = program(*dialect, &value, &doc.source, budget) {
                    programs.push(program);
                }
            }
        }
        // In the file's order: the stack walks it backwards among siblings.
        programs.sort_by_key(|p: &Program| p.at.as_ref().map_or(0, |at| at.start));
        programs
    }
}

/// The call's positional argument `index`: `None` past the last, or after a splat (its place is not known).
fn positional<'r, D: Doc>(args: &Node<'r, D>, index: usize) -> Option<Node<'r, D>> {
    let mut at = 0;
    for arg in args.named_code() {
        match arg.kind().as_ref() {
            "keyword_argument" | "dictionary_splat" => continue,
            "list_splat" => return None,
            _ if at == index => return Some(arg),
            _ => at += 1,
        }
    }
    None
}

/// The program of a plain string `value` (or an implicit concatenation of plain strings): its content, each part
/// mapped to its bytes in the file.
fn program<D: Doc>(dialect: GrammarDialect, value: &Node<'_, D>, source: &str, budget: &dyn Fn() -> bool) -> Option<Program> {
    let parts: Vec<Node<'_, D>> = match value.kind().as_ref() {
        "string" => vec![value.clone()],
        "concatenated_string" => value.named_code().collect(),
        _ => return None,
    };
    let mut text = String::new();
    let mut segments = SegmentMap::new();
    for part in &parts {
        let content = plain(part, source)?;
        let at = text.len();
        text.push_str(&source[content.clone()]);
        if !content.is_empty() {
            segments.push(at, text.len(), content, true, false);
        }
    }
    if text.trim().is_empty() {
        return None;
    }
    segments.index();
    let parse = super::Parse::read(dialect, &text, budget)?;
    // It stands at the argument, the string or the concatenation, as a rule names it (`parsed(s, "html", p)`).
    Some(Program { dialect, parse, text, segments, host_errors: Vec::new(), at: Some(value.range()) })
}

/// The bytes of a plain string's content: a `string` whose prefix is none, `r` or `u`, with no interpolation, and,
/// unless raw, no backslash (an escape's value is not its bytes). `None` for any other.
fn plain<D: Doc>(string: &Node<'_, D>, source: &str) -> Option<std::ops::Range<usize>> {
    if string.kind() != "string" {
        return None;
    }
    let start = string.range().start;
    let quote = source[start..].find(['"', '\''])?;
    let prefix = source[start..start + quote].to_ascii_lowercase();
    let raw = match prefix.as_str() {
        "" | "u" => false,
        "r" => true,
        _ => return None,
    };
    let range = string.range();
    let delimiter = if source[start + quote..].starts_with("\"\"\"") || source[start + quote..].starts_with("'''") { 3 } else { 1 };
    let content = start + quote + delimiter..range.end.checked_sub(delimiter)?;
    if content.start > content.end || string.named().any(|child| child.kind() == "interpolation") {
        return None;
    }
    if !raw && source[content.clone()].contains('\\') {
        return None;
    }
    Some(content)
}
