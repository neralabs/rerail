//! `braceFormat`: `str.format` templates, a port of Ruff 0.16.9's `ruff_python_literal::format` as
//! Pyflakes' F52x rules read it (`FormatSummary`: the template's parts, each field's name, and the
//! parts of its format spec, one level down), with the format-spec mini-language.
//!
//! The tree is a `format` root holding `literal` and `field` parts. A field holds its
//! `field_name` (whose value is the argument it reads: the automatic number, the index, or the
//! keyword), its `conversion` and its `format_spec`. The field name holds `auto` (zero width),
//! `index` or `name`, then its `attribute` and `element_index` parts. A spec with fields holds
//! `literal` and `field` parts (a second level of fields is an error, as in Ruff); a spec without
//! them holds the mini-language's `conversion`, `fill`, `align`, `sign`, `alternate`, `zero`,
//! `width`, `grouping`, `precision` and `type`, or a `spec_error`. A template Ruff refuses is a
//! `format` root with one `error` node, whose value is Ruff's F521 message.

use super::{Grammar, PNode};
use crate::code_rule_dsl::value::Value;

pub(crate) const KINDS: &[&str] = &[
    "format",
    "literal",
    "field",
    "field_name",
    "auto",
    "index",
    "name",
    "attribute",
    "element_index",
    "conversion",
    "format_spec",
    "fill",
    "align",
    "sign",
    "alternate",
    "zero",
    "width",
    "grouping",
    "precision",
    "type",
    "spec_error",
    "error",
];

const G: Grammar = Grammar::Brace;

/// Ruff's `FormatParseError`.
#[derive(Clone, Copy, Debug, PartialEq)]
enum ParseError {
    UnmatchedBracket,
    MissingStartBracket,
    UnescapedStartBracketInLiteral,
    PlaceholderRecursionExceeded,
    UnknownConversion,
    EmptyAttribute,
    MissingRightBracket,
    InvalidCharacterAfterRightBracket,
}

impl ParseError {
    /// Ruff's `error_to_string`, F521's message.
    fn message(self) -> &'static str {
        match self {
            Self::EmptyAttribute => "Empty attribute in format string",
            Self::InvalidCharacterAfterRightBracket => {
                "Only '.' or '[' may follow ']' in format field specifier"
            }
            Self::PlaceholderRecursionExceeded => "Max format placeholder recursion exceeded",
            Self::MissingStartBracket => "Single '}' encountered in format string",
            Self::MissingRightBracket => "Expected '}' before end of string",
            Self::UnmatchedBracket => "Single '{' encountered in format string",
            Self::UnescapedStartBracketInLiteral | Self::UnknownConversion => {
                "Unexpected error parsing format string"
            }
        }
    }
}

/// A part of a template, as slices of the decoded text.
enum Part<'t> {
    Literal { raw: &'t str, value: String },
    Field(Field<'t>),
}

struct Field<'t> {
    /// From the `{` to the `}`.
    whole: &'t str,
    name: &'t str,
    /// The conversion character, after `!`.
    conversion: Option<&'t str>,
    /// After the first `:`, when there is one.
    spec: Option<&'t str>,
}

/// Where `part` starts in `text`: `part` is a sub-slice of it.
fn offset(text: &str, part: &str) -> usize {
    part.as_ptr() as usize - text.as_ptr() as usize
}

/// Parses `text` as Ruff's `FormatSummary::try_from` reads it.
pub(crate) fn parse(text: &str) -> PNode {
    let mut root = PNode::new(G, "format", 0, text.len());
    let mut autos = 0;
    match template(text, text, &mut autos, true) {
        Ok(parts) => root.children = parts,
        Err(error) => {
            let mut node = PNode::new(G, "error", 0, text.len());
            node.error = true;
            node.value = Some(Value::Str(error.message().to_owned()));
            root.children = vec![node];
        }
    }
    root
}

/// A template's parts as nodes; with `top`, each field's spec is parsed one level down.
fn template(base: &str, text: &str, autos: &mut i64, top: bool) -> Result<Vec<PNode>, ParseError> {
    let mut nodes = Vec::new();
    for part in parts(text)? {
        match part {
            Part::Literal { raw, value } => {
                let start = offset(base, raw);
                nodes.push(
                    PNode::new(G, "literal", start, start + raw.len())
                        .with_value(Value::Str(value)),
                );
            }
            Part::Field(field) => nodes.push(field_node(base, &field, autos, top)?),
        }
    }
    Ok(nodes)
}

fn field_node(
    base: &str,
    field: &Field<'_>,
    autos: &mut i64,
    top: bool,
) -> Result<PNode, ParseError> {
    let start = offset(base, field.whole);
    let mut node = PNode::new(G, "field", start, start + field.whole.len());
    let name = field_name(base, field.name, autos)?;
    node.value = name.value.clone();
    node.children.push(name);
    if let Some(conversion) = field.conversion {
        let at = offset(base, conversion);
        node.children.push(
            PNode::new(G, "conversion", at, at + conversion.len())
                .with_value(Value::Str(conversion.to_owned())),
        );
    }
    if let Some(spec) = field.spec {
        let at = offset(base, spec);
        let mut spec_node = PNode::new(G, "format_spec", at, at + spec.len())
            .with_value(Value::Str(spec.to_owned()));
        if top {
            let nested = template(base, spec, autos, false)?;
            if nested.iter().any(|part| part.kind == G.kind_id("field")) {
                spec_node.children = nested;
            } else {
                spec_node.children = mini_language(base, spec);
            }
        }
        node.children.push(spec_node);
    }
    Ok(node)
}

/// Ruff's `FieldName::parse`: the first part (automatic, an index, or a keyword) and the parts
/// after it. The node's value is the argument the field reads.
fn field_name(base: &str, name: &str, autos: &mut i64) -> Result<PNode, ParseError> {
    let start = offset(base, name);
    let mut node = PNode::new(G, "field_name", start, start + name.len());
    let first_len = name.find(['.', '[']).unwrap_or(name.len());
    let first = &name[..first_len];
    let head = if first.is_empty() {
        let value = Value::int(*autos);
        *autos += 1;
        PNode::new(G, "auto", start, start).with_value(value)
    } else if let Ok(index) = first.parse::<usize>() {
        PNode::new(G, "index", start, start + first_len).with_value(Value::int(index as i64))
    } else {
        PNode::new(G, "name", start, start + first_len).with_value(Value::Str(first.to_owned()))
    };
    node.value = head.value.clone();
    node.children.push(head);
    let mut rest = &name[first_len..];
    while let Some(c) = rest.chars().next() {
        match c {
            '.' => {
                let body = &rest[1..];
                let len = body.find(['.', '[']).unwrap_or(body.len());
                if len == 0 {
                    return Err(ParseError::EmptyAttribute);
                }
                let at = offset(base, body);
                node.children.push(
                    PNode::new(G, "attribute", at, at + len)
                        .with_value(Value::Str(body[..len].to_owned())),
                );
                rest = &body[len..];
            }
            '[' => {
                let body = &rest[1..];
                let Some(close) = body.find(']') else {
                    return Err(ParseError::MissingRightBracket);
                };
                let index = &body[..close];
                if index.is_empty() {
                    return Err(ParseError::EmptyAttribute);
                }
                let value = match index.parse::<usize>() {
                    Ok(number) => Value::int(number as i64),
                    Err(_) => Value::Str(index.to_owned()),
                };
                let at = offset(base, index);
                node.children
                    .push(PNode::new(G, "element_index", at, at + index.len()).with_value(value));
                rest = &body[close + 1..];
            }
            _ => return Err(ParseError::InvalidCharacterAfterRightBracket),
        }
    }
    Ok(node)
}

/// Ruff's `FormatString::parse`: literals and bracketed fields, in turn, until the text ends.
fn parts(text: &str) -> Result<Vec<Part<'_>>, ParseError> {
    let mut rest = text;
    let mut parts = Vec::new();
    while !rest.is_empty() {
        let (part, next) = match literal(rest) {
            Ok(found) => found,
            Err(_) => spec(rest, true)?,
        };
        parts.push(part);
        rest = next;
    }
    Ok(parts)
}

/// Ruff's `parse_literal`: characters up to an unescaped brace; `\N{...}` after a backslash is one
/// piece, braces and all.
fn literal(text: &str) -> Result<(Part<'_>, &str), ParseError> {
    let mut rest = text;
    let mut value = String::new();
    let mut pending_escape = false;
    while !rest.is_empty() {
        if pending_escape
            && let Some(inner) = rest.strip_prefix("N{")
            && let Some(close) = inner.find('}')
        {
            let end = close + 3;
            value.push_str(&rest[..end]);
            rest = &rest[end..];
            pending_escape = false;
            continue;
        }
        let mut chars = rest.chars();
        let first = chars.next().expect("not empty");
        if first == '{' || first == '}' {
            if chars.next() == Some(first) {
                value.push(first);
                rest = chars.as_str();
                pending_escape = false;
                continue;
            }
            let consumed = text.len() - rest.len();
            return if value.is_empty() {
                Err(ParseError::UnescapedStartBracketInLiteral)
            } else {
                Ok((
                    Part::Literal {
                        raw: &text[..consumed],
                        value,
                    },
                    rest,
                ))
            };
        }
        value.push(first);
        rest = chars.as_str();
        pending_escape = first == '\\' && !pending_escape;
    }
    Ok((Part::Literal { raw: text, value }, ""))
}

/// Ruff's `parse_spec`: a bracketed field, with one level of nested brackets when `nesting`.
fn spec(text: &str, nesting: bool) -> Result<(Part<'_>, &str), ParseError> {
    let Some(inner) = text.strip_prefix('{') else {
        return Err(ParseError::MissingStartBracket);
    };
    let mut nested = false;
    for (index, c) in inner.char_indices() {
        if c == '{' {
            if nested || !nesting {
                return Err(ParseError::PlaceholderRecursionExceeded);
            }
            nested = true;
        } else if c == '}' {
            if nested {
                nested = false;
                continue;
            }
            let left = &inner[..index];
            let field = in_brackets(left, &text[..index + 2])?;
            return Ok((Part::Field(field), &inner[index + 1..]));
        }
    }
    Err(ParseError::UnmatchedBracket)
}

/// Ruff's `parse_part_in_brackets`: the name before the first `:` (and before its `!`), the
/// conversion after `!` (exactly one character), the spec after the `:`.
fn in_brackets<'t>(left: &'t str, whole: &'t str) -> Result<Field<'t>, ParseError> {
    let (argument, spec) = match left.find(':') {
        Some(colon) => (&left[..colon], Some(&left[colon + 1..])),
        None => (left, None),
    };
    let (name, conversion) = match argument.find('!') {
        Some(bang) => {
            let conversion = &argument[bang + 1..];
            if conversion.chars().count() != 1 {
                return Err(ParseError::UnknownConversion);
            }
            (&argument[..bang], Some(conversion))
        }
        None => (argument, None),
    };
    Ok(Field {
        whole,
        name,
        conversion,
        spec,
    })
}

/// Ruff's `FormatSpec::parse` for a spec without fields: the mini-language's pieces, or a
/// `spec_error` holding Ruff's error.
fn mini_language(base: &str, spec: &str) -> Vec<PNode> {
    let mut nodes = Vec::new();
    let mut rest = spec;
    let piece =
        |kind: &str, text: &str, rest: &str, value: Option<Value>, nodes: &mut Vec<PNode>| {
            let at = offset(base, text);
            let len = text.len() - rest.len();
            let mut node = PNode::new(G, kind, at, at + len);
            node.value = value;
            nodes.push(node);
        };
    let error = |message: String| {
        let at = offset(base, spec);
        let mut node = PNode::new(G, "spec_error", at, at + spec.len());
        node.error = true;
        node.value = Some(Value::Str(message));
        vec![node]
    };
    // A conversion (`!r`) at the start of a spec.
    let mut chars = rest.chars();
    if chars.next() == Some('!')
        && let Some(c) = chars.next()
        && matches!(c, 's' | 'r' | 'a' | 'b')
    {
        let after = chars.as_str();
        piece(
            "conversion",
            rest,
            after,
            Some(Value::Str(c.to_string())),
            &mut nodes,
        );
        rest = after;
    }
    // Fill and align.
    let mut leading = rest.char_indices();
    let first = leading.next();
    let second = leading.next();
    let align_at = |c: char| matches!(c, '<' | '>' | '=' | '^');
    match (first, second) {
        (Some((_, fill)), Some((at, align))) if align_at(align) => {
            let after_fill = &rest[at..];
            piece(
                "fill",
                rest,
                after_fill,
                Some(Value::Str(fill.to_string())),
                &mut nodes,
            );
            let after = &after_fill[align.len_utf8()..];
            piece(
                "align",
                after_fill,
                after,
                Some(Value::Str(align.to_string())),
                &mut nodes,
            );
            rest = after;
        }
        (Some((_, align)), _) if align_at(align) => {
            let after = &rest[align.len_utf8()..];
            piece(
                "align",
                rest,
                after,
                Some(Value::Str(align.to_string())),
                &mut nodes,
            );
            rest = after;
        }
        _ => {}
    }
    // Sign, `#`, `0`.
    for (kind, accepted) in [
        ("sign", &['-', '+', ' '][..]),
        ("alternate", &['#'][..]),
        ("zero", &['0'][..]),
    ] {
        if let Some(c) = rest.chars().next()
            && accepted.contains(&c)
        {
            let after = &rest[1..];
            let value = (kind == "sign").then(|| Value::Str(c.to_string()));
            piece(kind, rest, after, value, &mut nodes);
            rest = after;
        }
    }
    // Width.
    let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits > 0 {
        let Ok(width) = rest[..digits].parse::<usize>() else {
            return error("DecimalDigitsTooMany".to_owned());
        };
        let after = &rest[digits..];
        piece(
            "width",
            rest,
            after,
            Some(Value::int(width as i64)),
            &mut nodes,
        );
        rest = after;
    }
    // Grouping.
    if let Some(c) = rest.chars().next()
        && matches!(c, '_' | ',')
    {
        let after = &rest[1..];
        piece(
            "grouping",
            rest,
            after,
            Some(Value::Str(c.to_string())),
            &mut nodes,
        );
        rest = after;
    }
    // Precision: a `.` followed by digits; a bare `.` is left for the type, which refuses it.
    if let Some(after_dot) = rest.strip_prefix('.') {
        let digits = after_dot.len()
            - after_dot
                .trim_start_matches(|c: char| c.is_ascii_digit())
                .len();
        if digits > 0 {
            let Ok(precision) = after_dot[..digits].parse::<usize>() else {
                return error("DecimalDigitsTooMany".to_owned());
            };
            if precision > i32::MAX as usize {
                return error("PrecisionTooBig".to_owned());
            }
            let after = &after_dot[digits..];
            piece(
                "precision",
                rest,
                after,
                Some(Value::int(precision as i64)),
                &mut nodes,
            );
            rest = after;
        }
    }
    // Type, which must end the spec.
    if let Some(c) = rest.chars().next() {
        if !matches!(
            c,
            's' | 'b'
                | 'c'
                | 'd'
                | 'o'
                | 'n'
                | 'N'
                | 'x'
                | 'X'
                | 'e'
                | 'E'
                | 'f'
                | 'F'
                | 'g'
                | 'G'
                | '%'
        ) {
            return error(format!("InvalidFormatType({c:?})"));
        }
        let after = &rest[c.len_utf8()..];
        piece(
            "type",
            rest,
            after,
            Some(Value::Str(c.to_string())),
            &mut nodes,
        );
        rest = after;
    }
    if !rest.is_empty() {
        return error("InvalidFormatSpecifier".to_owned());
    }
    nodes
}

#[cfg(test)]
mod tests;
