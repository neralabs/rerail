//! ESLint's directive comments as ESLint 10.11.0 reads them: `ConfigCommentParser` (in
//! `@eslint/plugin-kit`) and `SourceCode#getInlineConfigNodes`. Ranges are byte offsets into the
//! comment's text, its `//`, `/*`, `<!--` or `-->` included. An HTML-like comment (Annex B.1.1),
//! which espree (acorn's `skipLineComment`) reads as a `Line` comment where the source type is
//! `script` or `commonjs` and does not parse in a module, is a `//` comment's equal: checked
//! against ESLint 9.39.1 and 10.11.0, whose `-line` and `-next-line` directives in one silence
//! `no-debugger` and whose `eslint-disable` in one is no directive.

use super::PNode;
use crate::code_rule_dsl::value::Value;
use std::ops::Range;

/// The kinds of the `eslintDirective` grammar's trees.
pub(crate) const KINDS: &[&str] = &["directive", "item", "setting", "description"];

/// JavaScript's `\s` (and what `String#trim` removes): white space and line terminators.
pub(crate) fn js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

/// `range` of `text` without the JavaScript white space around it.
fn trimmed(text: &str, range: Range<usize>) -> Range<usize> {
    let part = &text[range.clone()];
    let start = range.start + (part.len() - part.trim_start_matches(js_space).len());
    let end = range.end - (part.len() - part.trim_end_matches(js_space).len());
    start..end.max(start)
}

/// The labels `getInlineConfigNodes` keeps (`shared/directives.js`).
const LABELS: &[&str] = &[
    "eslint",
    "eslint-env",
    "eslint-enable",
    "eslint-disable",
    "eslint-disable-line",
    "eslint-disable-next-line",
    "exported",
    "global",
    "globals",
];

/// A directive comment as ESLint parses it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Directive {
    /// `eslint-disable`, `global`, ...
    pub(crate) label: String,
    pub(crate) label_range: Range<usize>,
    /// The text after the label, trimmed (`value`), before any description.
    pub(crate) value_range: Range<usize>,
    /// The text after ` -- `, trimmed (`justification`), when there is one.
    pub(crate) description: Option<Range<usize>>,
    /// Whether the comment is a line comment: `//`, `<!--` or `-->`.
    pub(crate) line: bool,
}

/// An item of a directive's list: a rule id of `eslint-disable`, a name of `global` or `exported`,
/// with its setting (`global a:off`) where it has one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Item {
    pub(crate) name: String,
    pub(crate) range: Range<usize>,
    pub(crate) setting: Option<(String, Range<usize>)>,
}

impl Directive {
    /// A comment's directive, as `getInlineConfigNodes` keeps it: a known label, and in a line
    /// comment only `eslint-disable-line` and `eslint-disable-next-line`. `comment` is the whole
    /// comment text, `//`, `/*`, `<!--` or `-->` included.
    pub(crate) fn parse(comment: &str) -> Option<Self> {
        let line = ["//", "<!--", "-->"]
            .into_iter()
            .find(|opener| comment.starts_with(opener));
        let (line, value) = if let Some(opener) = line {
            (true, opener.len()..comment.len())
        } else {
            let rest = comment.strip_prefix("/*")?.strip_suffix("*/")?;
            (false, 2..2 + rest.len())
        };
        // `#extractDirectiveComment`: the first `\s-{2,}\s` starts the justification.
        let text = &comment[value.clone()];
        let mut split = None;
        for (at, c) in text.char_indices() {
            if !js_space(c) {
                continue;
            }
            let after = &text[at + c.len_utf8()..];
            let dashes = after.len() - after.trim_start_matches('-').len();
            if dashes >= 2 && after[dashes..].chars().next().is_some_and(js_space) {
                let space = after[dashes..].chars().next().map_or(0, char::len_utf8);
                split = Some((at, at + c.len_utf8() + dashes + space));
                break;
            }
        }
        let (directive, description) = match split {
            Some((end, start)) => (
                trimmed(comment, value.start..value.start + end),
                Some(trimmed(comment, value.start + start..value.end)),
            ),
            None => (trimmed(comment, value.clone()), None),
        };
        // `directivesPattern`: `^([a-z]+(?:-[a-z]+)*)(?:\s|$)`.
        let part = &comment[directive.clone()];
        let mut end = 0;
        let bytes = part.as_bytes();
        loop {
            let run = bytes[end..]
                .iter()
                .take_while(|byte| byte.is_ascii_lowercase())
                .count();
            if run == 0 {
                break;
            }
            end += run;
            if bytes.get(end) == Some(&b'-')
                && bytes.get(end + 1).is_some_and(u8::is_ascii_lowercase)
            {
                end += 1;
            } else {
                break;
            }
        }
        if end == 0 || part[end..].chars().next().is_some_and(|c| !js_space(c)) {
            return None;
        }
        let label = &part[..end];
        if !LABELS.contains(&label)
            || (line && !matches!(label, "eslint-disable-line" | "eslint-disable-next-line"))
        {
            return None;
        }
        Some(Self {
            label: label.to_owned(),
            label_range: directive.start..directive.start + end,
            value_range: trimmed(comment, directive.start + end..directive.end),
            description,
            line,
        })
    }

    /// Whether the directive enables or disables rules.
    pub(crate) fn disables(&self) -> bool {
        self.label.starts_with("eslint-disable") || self.label == "eslint-enable"
    }

    /// The directive's list: `parseListConfig` (rule ids, `exported` names) or `parseStringConfig`
    /// (`global` names and settings); none for `eslint` and `eslint-env`.
    pub(crate) fn items(&self, comment: &str) -> Vec<Item> {
        match self.label.as_str() {
            "global" | "globals" => string_config(comment, self.value_range.clone()),
            "eslint" | "eslint-env" => Vec::new(),
            _ => list_config(comment, self.value_range.clone()),
        }
    }
}

/// `parseListConfig`: the value split on commas, each trimmed and without one pair of enclosing
/// quotes; empty ones dropped. A name listed twice is one item, at its first place.
fn list_config(comment: &str, value: Range<usize>) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::new();
    let mut start = value.start;
    for piece in comment[value.clone()].split(',') {
        let mut range = trimmed(comment, start..start + piece.len());
        start += piece.len() + 1;
        let text = &comment[range.clone()];
        if text.len() >= 2
            && (text.starts_with('\'') || text.starts_with('"'))
            && text.ends_with(&text[..1])
        {
            range = range.start + 1..range.end - 1;
        }
        let name = &comment[range.clone()];
        if name.is_empty() || items.iter().any(|item| item.name == name) {
            continue;
        }
        items.push(Item {
            name: name.to_owned(),
            range,
            setting: None,
        });
    }
    items
}

/// `parseStringConfig`: white space around `:` and `,` removed, then split on white space and
/// runs of commas; each `name` or `name:setting` (a later part after another `:` is dropped). A
/// name listed twice is one item, at its first place, with its last setting.
fn string_config(comment: &str, value: Range<usize>) -> Vec<Item> {
    // The collapsed text, with each character's offset in the comment.
    let text = &comment[value.clone()];
    let mut collapsed: Vec<(char, usize)> = Vec::new();
    for (at, c) in text.char_indices() {
        if js_space(c) {
            let before = text[..at].trim_end_matches(js_space).chars().next_back();
            let after = text[at..].trim_start_matches(js_space).chars().next();
            if matches!(before, Some(':' | ',')) || matches!(after, Some(':' | ',')) {
                continue;
            }
        }
        collapsed.push((c, value.start + at));
    }
    let mut items: Vec<Item> = Vec::new();
    let mut chunk: Vec<(char, usize)> = Vec::new();
    let mut flush = |chunk: &mut Vec<(char, usize)>| {
        if chunk.is_empty() {
            return;
        }
        let parts = chunk
            .split(|(c, _)| *c == ':')
            .map(|part| {
                let range = part.first().map_or(0..0, |(_, at)| {
                    *at..part.last().map_or(0, |(c, at)| at + c.len_utf8())
                });
                (part.iter().map(|(c, _)| c).collect::<String>(), range)
            })
            .collect::<Vec<_>>();
        chunk.clear();
        let (name, range) = parts[0].clone();
        let setting = parts.get(1).cloned();
        match items.iter_mut().find(|item| item.name == name) {
            Some(item) => item.setting = setting,
            None => items.push(Item {
                name,
                range,
                setting,
            }),
        }
    };
    for (c, at) in collapsed {
        if js_space(c) || c == ',' {
            flush(&mut chunk);
        } else {
            chunk.push((c, at));
        }
    }
    flush(&mut chunk);
    items
}

/// A JavaScript string value.
fn js_string(text: &str) -> Value {
    Value::Utf16(text.encode_utf16().collect())
}

/// The tree `parse: { grammar: "eslintDirective" }` reads from a comment ESLint reads as a
/// directive: a `directive` root over the whole comment, valued by its label, holding an `item`
/// per name of its list (valued by the name, over the name; a `global` name's `setting` child,
/// valued by the setting, over the setting after its `:`), then a `description` for the text after
/// ` -- `. `None` for any other comment.
pub(crate) fn tree(comment: &str) -> Option<PNode> {
    let directive = Directive::parse(comment)?;
    let grammar = GRAMMAR;
    let mut root =
        PNode::new(grammar, "directive", 0, comment.len()).with_value(js_string(&directive.label));
    for item in directive.items(comment) {
        let mut node = PNode::new(grammar, "item", item.range.start, item.range.end)
            .with_value(js_string(&item.name));
        if let Some((setting, range)) = item.setting {
            node.children.push(
                PNode::new(grammar, "setting", range.start, range.end)
                    .with_value(js_string(&setting)),
            );
        }
        root.children.push(node);
    }
    if let Some(range) = directive.description {
        root.children.push(
            PNode::new(grammar, "description", range.start, range.end)
                .with_value(js_string(&comment[range])),
        );
    }
    Some(root)
}

#[cfg(test)]
mod tests;

/// The `eslintDirective` grammar (`literal::LiteralGrammar`).
pub(crate) struct EslintDirectiveGrammar;

/// Its handle, which the engine registers (`literal::install`).
pub(crate) const GRAMMAR: super::Grammar = super::Grammar::new(&EslintDirectiveGrammar);

impl super::LiteralGrammar for EslintDirectiveGrammar {
    fn name(&self) -> &'static str {
        "eslintDirective"
    }
    fn kinds(&self) -> &'static [&'static str] {
        KINDS
    }
    fn host_language(&self) -> crate::code_rule_dsl::language::GrammarDialect {
        crate::code_rule_dsl::language::GrammarDialect::TypeScript
    }
    fn host(&self) -> &'static str {
        "TypeScript comments"
    }
    fn host_kinds(&self) -> &'static [&'static str] {
        &["comment"]
    }
    fn reads_comments(&self) -> bool {
        true
    }
    fn item_resolves(&self) -> bool {
        true
    }
    fn parse(&self, grammar: super::Grammar, text: &str) -> super::PNode {
        // A comment that is no directive has no tree (`parse_comment`).
        tree(text).unwrap_or_else(|| super::PNode::new(grammar, "directive", 0, text.len()))
    }
}

/// Parses a TypeScript comment ESLint reads as a directive (`eslintDirective`): its text, mapped
/// to the source by a shift. Spends one step of `spend` per byte and tree node; `None` when the
/// budget ran out.
pub(crate) fn parse_comment<D: ast_grep_core::Doc>(
    node: &ast_grep_core::Node<'_, D>,
    spend: &mut dyn FnMut(usize) -> bool,
) -> Option<super::Parsed> {
    let text = node.text();
    if !spend(text.len() + 1) {
        return None;
    }
    let Some(root) = tree(&text) else {
        return Some(super::Parsed::NotString);
    };
    if !spend(root.size()) {
        return None;
    }
    Some(super::Parsed::Tree(std::sync::Arc::new(super::SubTree::new(
        GRAMMAR,
        text.into_owned(),
        0,
        root,
        super::HostMap::Exact(node.range().start as isize),
    ))))
}
