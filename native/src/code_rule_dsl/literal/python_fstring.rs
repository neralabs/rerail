//! A JoinedStr's direct literal parts and opaque interpolation slots. Expressions and format
//! specifications are never evaluated. Debug-field source contributes literal text. A format_specifier can be
//! parsed separately, so its literal parts retain their own source positions.

use super::offsets::{self, Decoded};
use super::{Grammar, HostMap, PNode, Parsed, SubTree};
use crate::code_rule_dsl::value::python::{Prefix, prefix};
use crate::code_rule_dsl::value::{MAX_STRING_BYTES, Value};
use ast_grep_core::{Doc, Node};
use std::sync::Arc;

pub(crate) const KINDS: &[&str] = &["template", "literal", "interpolation"];
const G: Grammar = Grammar::PythonFString;

struct Builder {
    decoded: Decoded,
    children: Vec<PNode>,
    literal_start: usize,
}

impl Builder {
    fn flush(&mut self) {
        let end = self.decoded.text.len();
        if self.literal_start < end {
            self.children.push(
                PNode::new(G, "literal", self.literal_start, end).with_value(Value::Str(
                    self.decoded.text[self.literal_start..end].to_owned(),
                )),
            );
        }
        self.literal_start = end;
    }

    fn literal(&mut self, text: &str, start: usize, prefix: &Prefix) -> bool {
        offsets::decode_body(text, start, prefix, &mut self.decoded)
            && self.decoded.text.len() <= MAX_STRING_BYTES
    }

    fn field<D: Doc>(&mut self, field: &Node<'_, D>, marker: &str) -> bool {
        // Python's debug form adds the exact expression source, '=' and trailing whitespace as
        // a Constant before its FormattedValue, even when the expression itself is unknown.
        if let Some(end) = debug_end(field) {
            let range = field.range();
            let text = field.text();
            if let Some(debug) = text.get(1..end - range.start) {
                if self.decoded.text.len().saturating_add(debug.len()) > MAX_STRING_BYTES {
                    return false;
                }
                for (at, ch) in debug.char_indices() {
                    offsets::push(
                        &mut self.decoded,
                        ch,
                        range.start + 1 + at,
                        range.start + 1 + at + ch.len_utf8(),
                    );
                }
            }
        }
        self.flush();
        let start = self.decoded.text.len();
        if start.saturating_add(marker.len()) > MAX_STRING_BYTES {
            return false;
        }
        for ch in marker.chars() {
            offsets::push(
                &mut self.decoded,
                ch,
                field.range().start,
                field.range().end,
            );
        }
        self.children.push(
            PNode::new(G, "interpolation", start, self.decoded.text.len())
                .with_value(Value::Str(marker.to_owned())),
        );
        self.literal_start = self.decoded.text.len();
        true
    }
}

/// The source pieces a JoinedStr's value is read from, in order: a concatenated_string's strings, else the node.
fn parts<'r, D: Doc>(node: &Node<'r, D>) -> Vec<Node<'r, D>> {
    if node.kind() == "concatenated_string" {
        node.children()
            .filter(|part| part.kind() == "string")
            .collect()
    } else {
        vec![node.clone()]
    }
}

/// A part's replacement fields, in order: a format_specifier's format_expressions, a string's interpolations.
fn fields<'r, D: Doc>(part: &Node<'r, D>) -> impl Iterator<Item = Node<'r, D>> {
    let kind = if part.kind() == "format_specifier" {
        "format_expression"
    } else {
        "interpolation"
    };
    part.children().filter(move |child| child.kind() == kind)
}

/// Where a debug field's literal text (`{x = }`'s `x = `, from after the `{`) ends, as a source byte: before its
/// conversion, its format spec or its `}`. `None` for a field with no `=`.
fn debug_end<D: Doc>(field: &Node<'_, D>) -> Option<usize> {
    field.children().any(|child| child.kind() == "=").then(|| {
        field
            .field("type_conversion")
            .or_else(|| field.field("format_specifier"))
            .map_or(field.range().end - 1, |node| node.range().start)
    })
}

/// What a piece of a JoinedStr is (`pieces`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PieceKind {
    /// A literal run of the root: its decoded text, a debug field's `x = ` and a spec's literal text included.
    Literal,
    /// A replacement field of the root: its expression's source.
    Interpolation,
    /// A debug field's literal text (`x = ` in `{x = }`), source as written, at its interpolation's index.
    Debug,
    /// A field's format spec (`>10` in `{x:>10}`), source as written without its `:`, at its interpolation's index.
    FormatSpec,
}

impl PieceKind {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Literal => "literal",
            Self::Interpolation => "interpolation",
            Self::Debug => "debug",
            Self::FormatSpec => "format_spec",
        }
    }
}

/// The root children of `parse`'s tree, in order, as `(index, kind, text)`: each `literal` child with its value
/// and each `interpolation` child with its field's expression source; with an interpolation's index, its field's
/// `debug` text and `format_spec` source where it has them. `None` where `parse` gives no tree (not a JoinedStr,
/// bytes, or an unknown value). The literals and their order are the marker's alike: the marker is only the
/// interpolations' own text.
pub(crate) fn pieces<D: Doc>(node: &Node<'_, D>) -> Option<Vec<(usize, PieceKind, String)>> {
    let Parsed::Tree(tree) = parse(node, "{}", &mut |_| true)? else {
        return None;
    };
    let mut slots = parts(node)
        .into_iter()
        .flat_map(|part| fields(&part).collect::<Vec<_>>());
    let mut out = Vec::new();
    for (i, (kind, value)) in tree.root_children().enumerate() {
        if kind != "interpolation" {
            let Some(Value::Str(text)) = value else {
                continue;
            };
            out.push((i, PieceKind::Literal, text.clone()));
            continue;
        }
        // `parse` makes one interpolation per field, in this order, or no tree.
        let field = slots.next().expect("an interpolation per field");
        let expression = field
            .field("expression")
            .map_or_else(String::new, |e| e.text().into_owned());
        out.push((i, PieceKind::Interpolation, expression));
        let (source, start) = (field.text(), field.range().start);
        if let Some(debug) = debug_end(&field).and_then(|end| source.get(1..end - start)) {
            out.push((i, PieceKind::Debug, debug.to_owned()));
        }
        if let Some(spec) = field.field("format_specifier") {
            out.push((
                i,
                PieceKind::FormatSpec,
                spec.text().get(1..).unwrap_or_default().to_owned(),
            ));
        }
    }
    Some(out)
}

/// The root value of `parse`'s tree with interpolation slot text `marker`: the decoded literal text, each
/// interpolation replaced by `marker`. `None` where `parse` gives no tree.
pub(crate) fn template<D: Doc>(node: &Node<'_, D>, marker: &str) -> Option<String> {
    let Parsed::Tree(tree) = parse(node, marker, &mut |_| true)? else {
        return None;
    };
    match tree.root_value() {
        Some(Value::Str(text)) => Some(text.clone()),
        _ => None,
    }
}

pub(crate) fn parse<D: Doc>(
    node: &Node<'_, D>,
    marker: &str,
    spend: &mut dyn FnMut(usize) -> bool,
) -> Option<Parsed> {
    if !spend(node.range().len() + 1) {
        return None;
    }
    let mut builder = Builder {
        decoded: Decoded {
            text: String::new(),
            spans: Vec::new(),
            host: node.range(),
            simple: None,
        },
        children: Vec::new(),
        literal_start: 0,
    };
    let parts = parts(node);
    let mut formatted = node.kind() == "format_specifier";
    for part in parts {
        if part.kind() == "format_specifier" {
            // The spec's raw slices between format_expression nodes are literal text. The ':'
            // belongs to the JoinedStr's span but not its value.
            let text = part.text();
            let mut start = part.range().start + 1;
            let raw = part
                .ancestors()
                .find(|node| node.kind() == "string")
                .and_then(|node| node.children().find(|child| child.kind() == "string_start"))
                .is_some_and(|node| node.text().to_ascii_lowercase().contains('r'));
            let pref = prefix(if raw { "rf" } else { "f" }).expect("f prefix");
            for field in part
                .children()
                .filter(|child| child.kind() == "format_expression")
            {
                if !builder.literal(
                    &text[start - part.range().start..field.range().start - part.range().start],
                    start,
                    &pref,
                ) {
                    return Some(Parsed::Unknown);
                }
                if !builder.field(&field, marker) {
                    return Some(Parsed::Unknown);
                }
                start = field.range().end;
            }
            if !builder.literal(&text[start - part.range().start..], start, &pref) {
                return Some(Parsed::Unknown);
            }
            continue;
        }
        let start = part
            .children()
            .find(|child| child.kind() == "string_start")?;
        let text = start.text();
        let quote = text.find(['\'', '"'])?;
        let pref = prefix(&text[..quote])?;
        if pref.bytes || pref.template {
            return Some(Parsed::NotString);
        }
        formatted |= pref.format;
        for child in part.children() {
            match child.kind().as_ref() {
                "string_content" if !builder.literal(&child.text(), child.range().start, &pref) => {
                    return Some(Parsed::Unknown);
                }
                "interpolation" if !builder.field(&child, marker) => {
                    return Some(Parsed::Unknown);
                }
                _ => {}
            }
        }
        if builder.decoded.text.len() > MAX_STRING_BYTES {
            return Some(Parsed::Unknown);
        }
    }
    if !formatted {
        return Some(Parsed::NotString);
    }
    builder.flush();
    let mut root = PNode::new(G, "template", 0, builder.decoded.text.len())
        .with_value(Value::Str(builder.decoded.text.clone()));
    root.children = builder.children;
    if !spend(root.size() + builder.decoded.text.len()) {
        return None;
    }
    Some(Parsed::Tree(Arc::new(SubTree::new(
        G,
        builder.decoded.text.clone(),
        0,
        root,
        HostMap::Spans(builder.decoded),
    ))))
}
