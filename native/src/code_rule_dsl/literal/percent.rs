//! `percentFormat`: printf-style formats, a port of Ruff 0.16.9's `ruff_python_literal::cformat`
//! (the parser Pyflakes' F50x rules read), over `str` values.
//!
//! The tree is a `format` root holding `literal` and `placeholder` parts in order. A placeholder
//! holds, when present, its `mapping_key`, `flags`, `width`, `precision` (a `star` inside each of
//! the last two for `*`), `length` and `conversion`. A string Ruff refuses is a `format` root with
//! one `error` node, whose value is Ruff's message.

use super::{Grammar, PNode};
use crate::code_rule_dsl::value::Value;

pub(crate) const KINDS: &[&str] = &[
    "format",
    "literal",
    "placeholder",
    "mapping_key",
    "flags",
    "width",
    "precision",
    "star",
    "length",
    "conversion",
    "error",
];

const G: Grammar = Grammar::Percent;

/// Ruff's `CFormatErrorType`, with the character index it reports.
enum ErrorType {
    UnmatchedKeyParentheses,
    UnsupportedFormatChar(char),
    IncompleteFormat,
    IntTooBig,
}

struct Error {
    kind: ErrorType,
    /// A character index, as Ruff's `CFormatError::index`.
    index: usize,
}

impl Error {
    /// Ruff's `Display` of the error (the text of F501's message; F509 reads the character).
    fn message(&self) -> String {
        match self.kind {
            ErrorType::UnmatchedKeyParentheses => "incomplete format key".to_owned(),
            ErrorType::IncompleteFormat => "incomplete format".to_owned(),
            ErrorType::UnsupportedFormatChar(c) => format!(
                "unsupported format character '{c}' ({:#x}) at index {}",
                c as u32, self.index
            ),
            ErrorType::IntTooBig => "width/precision too big".to_owned(),
        }
    }
}

/// The decoded text's characters with their byte offsets.
struct Chars<'t> {
    text: &'t str,
    chars: Vec<(usize, char)>,
}

impl Chars<'_> {
    fn get(&self, index: usize) -> Option<char> {
        self.chars.get(index).map(|(_, c)| *c)
    }
    /// The byte offset of the character at `index`, or the text's end past the last.
    fn byte(&self, index: usize) -> usize {
        self.chars.get(index).map_or(self.text.len(), |(at, _)| *at)
    }
}

/// Parses `text` as Ruff's `CFormatString::from_str` does.
pub(crate) fn parse(text: &str) -> PNode {
    let chars = Chars {
        text,
        chars: text.char_indices().collect(),
    };
    let mut root = PNode::new(G, "format", 0, text.len());
    match parts(&chars) {
        Ok(parts) => root.children = parts,
        Err(error) => {
            let start = chars.byte(error.index);
            let end = match error.kind {
                ErrorType::UnsupportedFormatChar(c) => start + c.len_utf8(),
                _ => start,
            };
            let mut node = PNode::new(G, "error", start, end);
            node.error = true;
            node.value = Some(Value::Str(error.message()));
            root.children = vec![node];
        }
    }
    root
}

fn parts(chars: &Chars<'_>) -> Result<Vec<PNode>, Error> {
    let mut parts = Vec::new();
    // The literal being accumulated: its first character's index and its text.
    let mut literal: Option<(usize, String)> = None;
    let mut positional = 0;
    let mut index = 0;
    let flush = |literal: &mut Option<(usize, String)>, end: usize, parts: &mut Vec<PNode>| {
        if let Some((start, text)) = literal.take() {
            parts.push(
                PNode::new(G, "literal", chars.byte(start), chars.byte(end))
                    .with_value(Value::Str(text)),
            );
        }
    };
    while let Some(c) = chars.get(index) {
        if c != '%' {
            literal
                .get_or_insert_with(|| (index, String::new()))
                .1
                .push(c);
            index += 1;
            continue;
        }
        match chars.get(index + 1) {
            Some('%') => {
                literal
                    .get_or_insert_with(|| (index, String::new()))
                    .1
                    .push('%');
                index += 2;
            }
            Some(_) => {
                flush(&mut literal, index, &mut parts);
                let (placeholder, next) = spec(chars, index, &mut positional)?;
                parts.push(placeholder);
                index = next;
            }
            None => {
                return Err(Error {
                    kind: ErrorType::IncompleteFormat,
                    index: index + 1,
                });
            }
        }
    }
    flush(&mut literal, index, &mut parts);
    Ok(parts)
}

/// Ruff's `CFormatSpec::parse`, from the `%` at `percent`; returns the placeholder and the index
/// after it. `positional` counts the values unkeyed specifiers and stars consume so far.
fn spec(
    chars: &Chars<'_>,
    percent: usize,
    positional: &mut usize,
) -> Result<(PNode, usize), Error> {
    let mut index = percent + 1;
    let mut children = Vec::new();
    let mut key = None;
    // Mapping key: parenthesized text, parentheses nesting.
    if chars.get(index) == Some('(') {
        let open = index;
        index += 1;
        let mut depth = 1;
        let mut text = String::new();
        loop {
            let Some(c) = chars.get(index) else {
                return Err(Error {
                    kind: ErrorType::UnmatchedKeyParentheses,
                    index: open,
                });
            };
            index += 1;
            match c {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => {}
            }
            if depth > 0 {
                text.push(c);
            } else {
                break;
            }
        }
        children.push(
            PNode::new(
                G,
                "mapping_key",
                chars.byte(open + 1),
                chars.byte(index - 1),
            )
            .with_value(Value::Str(text.clone())),
        );
        key = Some(text);
    }
    // Flags.
    let flags_start = index;
    while matches!(chars.get(index), Some('#' | '0' | '-' | ' ' | '+')) {
        index += 1;
    }
    if index > flags_start {
        let text: String = (flags_start..index)
            .filter_map(|at| chars.get(at))
            .collect();
        children.push(
            PNode::new(G, "flags", chars.byte(flags_start), chars.byte(index))
                .with_value(Value::Str(text)),
        );
    }
    // Width.
    if let Some(width) = quantity(chars, &mut index, "width", positional)? {
        children.push(width);
    }
    // Precision.
    if chars.get(index) == Some('.') {
        let dot = index;
        index += 1;
        let quantity = quantity(chars, &mut index, "precision", positional)?;
        match quantity {
            Some(mut precision) => {
                precision.start = chars.byte(dot);
                children.push(precision);
            }
            None => children.push(PNode::new(
                G,
                "precision",
                chars.byte(dot),
                chars.byte(index),
            )),
        }
    }
    // Length modifier.
    if matches!(chars.get(index), Some('h' | 'l' | 'L')) {
        children.push(PNode::new(
            G,
            "length",
            chars.byte(index),
            chars.byte(index + 1),
        ));
        index += 1;
    }
    // Conversion.
    let Some(c) = chars.get(index) else {
        return Err(Error {
            kind: ErrorType::IncompleteFormat,
            index: 0,
        });
    };
    if !matches!(
        c,
        'd' | 'i'
            | 'u'
            | 'o'
            | 'x'
            | 'X'
            | 'e'
            | 'E'
            | 'f'
            | 'F'
            | 'g'
            | 'G'
            | 'c'
            | 'r'
            | 's'
            | 'a'
    ) {
        return Err(Error {
            kind: ErrorType::UnsupportedFormatChar(c),
            index,
        });
    }
    children.push(
        PNode::new(G, "conversion", chars.byte(index), chars.byte(index + 1))
            .with_value(Value::Str(c.to_string())),
    );
    index += 1;
    let mut placeholder = PNode::new(G, "placeholder", chars.byte(percent), chars.byte(index));
    placeholder.value = Some(match key {
        Some(key) => Value::Str(key),
        None => {
            *positional += 1;
            Value::int(*positional as i64 - 1)
        }
    });
    placeholder.children = children;
    Ok((placeholder, index))
}

/// Ruff's `parse_quantity`: `*` (a `star` consuming the next positional value) or decimal digits
/// that fit an `i32`; `kind` names the node.
fn quantity(
    chars: &Chars<'_>,
    index: &mut usize,
    kind: &str,
    positional: &mut usize,
) -> Result<Option<PNode>, Error> {
    let start = *index;
    match chars.get(start) {
        Some('*') => {
            *index += 1;
            let star = PNode::new(G, "star", chars.byte(start), chars.byte(start + 1))
                .with_value(Value::int(*positional as i64));
            *positional += 1;
            let mut node = PNode::new(G, kind, chars.byte(start), chars.byte(start + 1));
            node.children.push(star);
            Ok(Some(node))
        }
        Some(c) if c.is_ascii_digit() => {
            let mut number: i32 = 0;
            while let Some(digit) = chars.get(*index).and_then(|c| c.to_digit(10)) {
                number = number
                    .checked_mul(10)
                    .and_then(|number| number.checked_add(digit as i32))
                    .ok_or(Error {
                        kind: ErrorType::IntTooBig,
                        index: *index,
                    })?;
                *index += 1;
            }
            Ok(Some(
                PNode::new(G, kind, chars.byte(start), chars.byte(*index))
                    .with_value(Value::int(i64::from(number))),
            ))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::oracle;

    /// The tree as `kind(value)[children]`, one line per part.
    fn shape(text: &str) -> Vec<String> {
        fn node(text: &str, node: &PNode) -> String {
            let kind = KINDS[usize::from(node.kind) - 1];
            let value = match &node.value {
                Some(Value::Str(value)) => format!("={value:?}"),
                Some(Value::Int(value)) => format!("={value}"),
                _ => String::new(),
            };
            let children = node
                .children
                .iter()
                .map(|child| node_fn(text, child))
                .collect::<Vec<_>>();
            let inner = if children.is_empty() {
                String::new()
            } else {
                format!("[{}]", children.join(" "))
            };
            format!("{kind}{value}:{:?}{inner}", &text[node.start..node.end])
        }
        fn node_fn(text: &str, child: &PNode) -> String {
            node(text, child)
        }
        parse(text)
            .children
            .iter()
            .map(|part| node(text, part))
            .collect()
    }

    #[test]
    fn ruffs_examples() {
        assert_eq!(
            shape("Hello, my name is %s and I'm %d years old"),
            [
                "literal=\"Hello, my name is \":\"Hello, my name is \"",
                "placeholder=0:\"%s\"[conversion=\"s\":\"s\"]",
                "literal=\" and I'm \":\" and I'm \"",
                "placeholder=1:\"%d\"[conversion=\"d\":\"d\"]",
                "literal=\" years old\":\" years old\"",
            ]
        );
        assert_eq!(
            shape("%(m((u(((l((((ti))))p)))l))e)d"),
            [
                "placeholder=\"m((u(((l((((ti))))p)))l))e\":\"%(m((u(((l((((ti))))p)))l))e)d\"[mapping_key=\"m((u(((l((((ti))))p)))l))e\":\"m((u(((l((((ti))))p)))l))e\" conversion=\"d\":\"d\"]"
            ]
        );
        assert_eq!(
            shape("%  0   -+++###10d"),
            [
                "placeholder=0:\"%  0   -+++###10d\"[flags=\"  0   -+++###\":\"  0   -+++###\" width=10:\"10\" conversion=\"d\":\"d\"]"
            ]
        );
        assert_eq!(shape("%(aged"), ["error=\"incomplete format key\":\"\""]);
        assert_eq!(
            shape("Hello %n"),
            ["error=\"unsupported format character 'n' (0x6e) at index 7\":\"n\""]
        );
        assert_eq!(shape("Hello %"), ["error=\"incomplete format\":\"\""]);
    }

    #[test]
    fn stars_consume_positional_values() {
        assert_eq!(
            shape("%*.*f %% %s"),
            [
                "placeholder=2:\"%*.*f\"[width:\"*\"[star=0:\"*\"] precision:\".*\"[star=1:\"*\"] conversion=\"f\":\"f\"]",
                "literal=\" % \":\" %% \"",
                "placeholder=3:\"%s\"[conversion=\"s\":\"s\"]",
            ]
        );
        assert_eq!(
            shape("%(a).3lx%.f"),
            [
                "placeholder=\"a\":\"%(a).3lx\"[mapping_key=\"a\":\"a\" precision=3:\".3\" length:\"l\" conversion=\"x\":\"x\"]",
                "placeholder=0:\"%.f\"[precision:\".\" conversion=\"f\":\"f\"]",
            ]
        );
        // `%b` is for bytes only; `%5` ends without a conversion.
        assert_eq!(
            shape("%b"),
            ["error=\"unsupported format character 'b' (0x62) at index 1\":\"b\""]
        );
        assert_eq!(shape("a%5"), ["error=\"incomplete format\":\"\""]);
        assert_eq!(
            shape("%99999999999d"),
            ["error=\"width/precision too big\":\"\""]
        );
        // Character indices, not bytes.
        assert_eq!(
            shape("é%ü"),
            ["error=\"unsupported format character 'ü' (0xfc) at index 2\":\"ü\""]
        );
    }

    /// Every case of `fixtures/python/literal/percent.expected.json`: errors and messages as Ruff 0.16.9 reports them
    /// (F501, F509), keys and positional values as CPython 3.13 reads them.
    #[test]
    fn percent_formats_match_ruff_and_cpython() {
        let oracle: serde_json::Value =
            serde_json::from_str(oracle!("python/literal/percent.expected.json")).expect("oracle");
        let cases = oracle["cases"].as_array().expect("cases");
        assert!(cases.len() > 4_000);
        for case in cases {
            let text = case[0].as_str().expect("text");
            let (python, ruff) = (&case[1], &case[2]);
            let root = parse(text);
            let error = root
                .children
                .iter()
                .find(|part| part.error)
                .and_then(|part| match &part.value {
                    Some(Value::Str(message)) => Some(message.clone()),
                    _ => None,
                });
            if let Some(expected) = ruff["F501"].as_str() {
                assert_eq!(error.as_deref(), Some(expected), "{text:?}");
                continue;
            }
            if let Some(character) = ruff["F509"].as_str() {
                let error = error.unwrap_or_default();
                assert!(
                    error.starts_with(&format!("unsupported format character '{character}' ")),
                    "{text:?}: {error}"
                );
                continue;
            }
            assert_eq!(error, None, "{text:?}");
            let placeholders = || {
                root.children
                    .iter()
                    .filter(|part| KINDS[usize::from(part.kind) - 1] == "placeholder")
            };
            let keys: Vec<String> = placeholders()
                .filter_map(|part| match &part.value {
                    Some(Value::Str(key)) => Some(key.clone()),
                    _ => None,
                })
                .collect();
            let stars = |node: &PNode| {
                node.children
                    .iter()
                    .filter(|child| {
                        child
                            .children
                            .iter()
                            .any(|star| KINDS[usize::from(star.kind) - 1] == "star")
                    })
                    .count()
            };
            let positional: usize = placeholders()
                .map(|part| usize::from(!matches!(part.value, Some(Value::Str(_)))) + stars(part))
                .sum();
            if let Some(expected) = python["positional"].as_u64() {
                assert!(keys.is_empty(), "{text:?}");
                assert_eq!(positional as u64, expected, "{text:?}");
            }
            let expected_keys: Vec<String> = python["keys"]
                .as_array()
                .map(|keys| {
                    keys.iter()
                        .filter_map(|key| key.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            if !expected_keys.is_empty() {
                assert_eq!(keys, expected_keys, "{text:?}");
            }
        }
    }
}
