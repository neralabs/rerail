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
        if field.children().any(|child| child.kind() == "=") {
            let range = field.range();
            let end = field
                .field("type_conversion")
                .or_else(|| field.field("format_specifier"))
                .map_or(range.end - 1, |node| node.range().start);
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
    let parts = if node.kind() == "concatenated_string" {
        node.children()
            .filter(|part| part.kind() == "string")
            .collect::<Vec<_>>()
    } else {
        vec![node.clone()]
    };
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
