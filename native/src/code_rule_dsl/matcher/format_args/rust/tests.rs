use super::*;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::literal::rust_format::{
    Align, Argument, Count, Piece, Sign, Slot, Trait, bind, parse,
};
use ast_grep_core::tree_sitter::LanguageExt;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

/// The macro invocation starting at byte `offset` of the source.
fn invocation_at<'r, D: Doc>(node: &Node<'r, D>, offset: usize) -> Option<Node<'r, D>> {
    if node.kind() == "macro_invocation" && node.range().start == offset {
        return Some(node.clone());
    }
    node.children()
        .find_map(|child| invocation_at(&child, offset))
}

/// Every `let name = literal;` of the source: the variables a case's captures read.
fn let_values<D: Doc>(node: &Node<'_, D>, found: &mut BTreeMap<String, RustValue>) {
    if node.kind() == "let_declaration"
        && let (Some(pattern), Some(value)) = (node.field("pattern"), node.field("value"))
        && pattern.kind() == "identifier"
        && matches!(
            value.kind().as_ref(),
            "integer_literal" | "string_literal" | "raw_string_literal"
        )
    {
        found.insert(pattern.text().to_string(), literal(&value));
    }
    for child in node.children() {
        let_values(&child, found);
    }
}

/// The source offset (in the literal token's text) of every byte of the decoded string, and one
/// more for its end: an escape maps to where it starts, a line continuation's skipped whitespace
/// to nothing.
fn source_offsets(token: &str) -> Vec<usize> {
    let bytes = token.as_bytes();
    if bytes[0] == b'r' {
        let hashes = bytes[1..].iter().take_while(|b| **b == b'#').count();
        let start = hashes + 2;
        let end = token.len() - hashes - 1;
        let mut offsets: Vec<usize> = (start..end).collect();
        offsets.push(end);
        return offsets;
    }
    let content_end = token.len() - 1;
    let mut offsets = Vec::new();
    let mut at = 1;
    while at < content_end {
        let ch = token[at..].chars().next().expect("char");
        if ch != '\\' {
            offsets.extend(std::iter::repeat_n(at, ch.len_utf8()));
            at += ch.len_utf8();
            continue;
        }
        let escape_start = at;
        let decoded: Option<char> = match bytes[at + 1] {
            b'n' => Some('\n'),
            b'r' => Some('\r'),
            b't' => Some('\t'),
            b'\\' => Some('\\'),
            b'0' => Some('\0'),
            b'\'' => Some('\''),
            b'"' => Some('"'),
            b'x' => {
                let value = u32::from_str_radix(&token[at + 2..at + 4], 16).expect("hex");
                at += 2;
                char::from_u32(value)
            }
            b'u' => {
                let close = token[at..].find('}').expect("closing brace") + at;
                let value = u32::from_str_radix(&token[at + 3..close], 16).expect("hex");
                at = close - 1;
                char::from_u32(value)
            }
            b'\n' | b'\r' => {
                // A line continuation drops the newline and the whitespace after it.
                at += 2;
                while token[at..].starts_with(char::is_whitespace) && at < content_end {
                    at += token[at..].chars().next().expect("char").len_utf8();
                }
                continue;
            }
            other => panic!("escape \\{} not in the fixtures", other as char),
        };
        let ch = decoded.expect("scalar value");
        offsets.extend(std::iter::repeat_n(escape_start, ch.len_utf8()));
        at += 2;
    }
    offsets.push(content_end);
    offsets
}

/// What a value prints as, when the test can render it.
fn display(value: &RustValue, debug: bool) -> Option<String> {
    match value {
        RustValue::Int { value, .. } => Some(value.to_string()),
        RustValue::Str(text) if debug => Some(format!("{text:?}")),
        RustValue::Str(text) => Some(text.clone()),
        _ => None,
    }
}

/// A count's value from the argument it reads.
fn count_value(value: &RustValue) -> Option<usize> {
    match value {
        RustValue::Int { value, .. } => value.to_string().parse().ok(),
        _ => None,
    }
}

/// Pads `text` to `width` as `{:fill align width}` does.
fn pad(text: String, width: usize, fill: char, align: Align) -> String {
    let length = text.chars().count();
    if length >= width {
        return text;
    }
    let total = width - length;
    let (left, right) = match align {
        Align::Left => (0, total),
        Align::Right => (total, 0),
        Align::Center => (total / 2, total - total / 2),
    };
    let side = |n: usize| fill.to_string().repeat(n);
    format!("{}{text}{}", side(left), side(right))
}

/// Why a case's output is not rendered.
#[derive(Debug, PartialEq)]
enum Skip {
    /// An argument that is no integer or string literal.
    Value,
    /// A field type other than `Display` and `Debug`, or a float's precision.
    Trait,
}

/// The call's output, from its binding and the values the arguments and variables have.
fn render(
    call: &FormatCall<'_, impl Doc>,
    pieces: &[Piece],
    binding: &crate::code_rule_dsl::literal::rust_format::Binding,
    values: &BTreeMap<String, RustValue>,
) -> Result<String, Skip> {
    let arguments: Vec<Option<RustValue>> = call
        .arguments
        .iter()
        .map(|argument| {
            (argument.first.node_id() == argument.last.node_id()).then(|| literal(&argument.first))
        })
        .collect();
    let read = |slot: &Slot| -> Result<RustValue, Skip> {
        match slot {
            Slot::Argument(index) => arguments[*index].clone().ok_or(Skip::Value),
            Slot::Capture(name) => values.get(name).cloned().ok_or(Skip::Value),
            Slot::Missing(_) => Err(Skip::Value),
        }
    };
    let mut out = String::new();
    let mut fields = binding.fields.iter();
    for piece in pieces {
        match piece {
            Piece::Literal { range } => out.push_str(&call.value[range.clone()]),
            Piece::Field {
                spec,
                width: width_count,
                precision: precision_count,
                ..
            } => {
                let field = fields.next().expect("one binding per field");
                let value = read(&field.value)?;
                let width = field.width.as_ref().map(&read).transpose()?;
                let precision = field.precision.as_ref().map(&read).transpose()?;
                let debug = match spec.ty {
                    Trait::Display => false,
                    Trait::Debug => true,
                    _ => return Err(Skip::Trait),
                };
                let is_int = matches!(value, RustValue::Int { .. });
                let mut text = display(&value, debug).ok_or(Skip::Value)?;
                let precision = match (&precision, precision_count) {
                    (Some(value), _) => Some(count_value(value).ok_or(Skip::Value)?),
                    (None, Some(Count::Literal(precision))) => Some(*precision),
                    _ => None,
                };
                if let Some(precision) = precision
                    && !is_int
                {
                    text = text.chars().take(precision).collect();
                }
                if spec.sign == Some(Sign::Plus) && is_int {
                    text = format!("+{text}");
                }
                let width = match (&width, width_count) {
                    (Some(value), _) => count_value(value).ok_or(Skip::Value)?,
                    (None, Some(Count::Literal(literal))) => *literal,
                    _ => 0,
                };
                if spec.zero && is_int {
                    let (sign, digits) = match text.strip_prefix('+') {
                        Some(digits) => ("+", digits.to_string()),
                        None => ("", text.clone()),
                    };
                    let digits_width = width.saturating_sub(sign.len());
                    text = format!("{sign}{}", pad(digits, digits_width, '0', Align::Right));
                } else {
                    let default = if is_int { Align::Right } else { Align::Left };
                    text = pad(
                        text,
                        width,
                        spec.fill.unwrap_or(' '),
                        spec.align.unwrap_or(default),
                    );
                }
                out.push_str(&text);
            }
        }
    }
    Ok(out)
}

/// The numbers a "invalid reference to positional argument(s) N and M (there ..)" message names.
fn referenced_indices(message: &str) -> BTreeSet<usize> {
    let list = message
        .split(" (")
        .next()
        .expect("message")
        .trim_start_matches("invalid reference to positional argument");
    list.split(|c: char| !c.is_ascii_digit())
        .filter(|digits| !digits.is_empty())
        .map(|digits| digits.parse().expect("number"))
        .collect()
}

/// The name in "cannot find value `x` in this scope".
fn unresolved_name(message: &str) -> String {
    message.split('`').nth(1).expect("quoted name").to_string()
}

#[test]
fn format_call_matches_rustc_on_every_case() {
    let oracle: serde_json::Value = serde_json::from_str(crate::code_rule_dsl::oracle!(
        "rust/matcher/format_args/rust.expected.json"
    ))
    .expect("oracle JSON");
    let cases = oracle["cases"].as_array().expect("cases");
    let originals: serde_json::Value = serde_json::from_str(crate::code_rule_dsl::oracle!(
        "rust/matcher/format_args/rust.cases.json"
    ))
    .expect("cases JSON");
    let items_of: BTreeMap<&str, &str> = originals["cases"]
        .as_array()
        .expect("cases")
        .iter()
        .map(|case| {
            (
                case["name"].as_str().expect("name"),
                case["items"].as_str().unwrap_or(""),
            )
        })
        .collect();
    let (mut checked, mut unknown, mut rendered) = (0usize, 0usize, 0usize);
    let mut skipped: Vec<(String, Skip)> = Vec::new();
    for case in cases {
        let name = case["name"].as_str().expect("name");
        let src = case["src"].as_str().expect("src");
        let items = items_of[name];
        let prefix = format!(
            "fn main() {{ {items}{}",
            if items.is_empty() { "" } else { " " }
        );
        let source = format!("{prefix}{src}; }}");
        let root = GrammarDialect::Rust.ast_grep(&source);
        let invocation =
            invocation_at(&root.root(), prefix.len()).unwrap_or_else(|| panic!("{name}: no call"));
        let call = format_call(&invocation);
        if case["calls"] == false {
            assert!(call.is_none(), "{name}: expected an unknown format string");
            unknown += 1;
            continue;
        }
        let call = call.unwrap_or_else(|| panic!("{name}: no format call in {src}"));
        checked += 1;

        let errors = case["errors"].as_array().expect("errors");
        let messages: Vec<&str> = errors
            .iter()
            .map(|error| error["message"].as_str().expect("message"))
            .collect();
        let offset = prefix.len();
        let span_of = |span: &serde_json::Value| -> Range<usize> {
            span["start"].as_u64().expect("start") as usize + offset
                ..span["end"].as_u64().expect("end") as usize + offset
        };

        // rustc refuses calls whose positional arguments follow a named one, before anything else.
        let order_error = messages
            .iter()
            .any(|m| m.starts_with("positional arguments cannot follow named arguments"));
        assert_eq!(call.well_ordered, !order_error, "{name}: argument order");
        if order_error {
            continue;
        }

        let parsed = parse(&call.value);
        let template_error = messages.iter().any(|m| {
            m.starts_with("invalid format string") || m.starts_with("unknown format trait")
        });
        assert_eq!(parsed.is_err(), template_error, "{name}: template parse");
        let Ok(pieces) = parsed else { continue };

        let binding = bind(&pieces, call.positional, &call.named);

        // Unused arguments: the spans of the argument values rustc says are never used.
        let argument_spans: Vec<Range<usize>> = call
            .arguments
            .iter()
            .map(|a| a.first.range().start..a.last.range().end)
            .collect();
        let mut rustc_unused = BTreeSet::new();
        let mut rustc_missing_pieces = BTreeSet::new();
        let mut implicit_message = None;
        let mut invalid_reference = None;
        let mut rustc_captures = BTreeSet::new();
        let token_start = call.template.range().start;
        let offsets = source_offsets(&call.template.text());
        let piece_span = |piece: &Piece| -> Range<usize> {
            let (Piece::Literal { range } | Piece::Field { range, .. }) = piece;
            token_start + offsets[range.start]..token_start + offsets[range.end]
        };
        for error in errors {
            let message = error["message"].as_str().expect("message");
            let spans: Vec<Range<usize>> = error["spans"]
                .as_array()
                .expect("spans")
                .iter()
                .map(span_of)
                .collect();
            if message.ends_with("argument never used")
                || message.starts_with("multiple unused formatting arguments")
            {
                for span in &spans {
                    let index = argument_spans
                        .iter()
                        .position(|argument| argument == span)
                        .unwrap_or_else(|| panic!("{name}: span {span:?} is no argument"));
                    rustc_unused.insert(index);
                }
            } else if message.starts_with("invalid reference to positional argument")
                || message.contains("positional argument") && message.contains("in format string")
            {
                for span in &spans {
                    let at = pieces
                        .iter()
                        .position(|piece| {
                            let range = piece_span(piece);
                            matches!(piece, Piece::Field { .. })
                                && range.start <= span.start
                                && span.end <= range.end
                        })
                        .unwrap_or_else(|| panic!("{name}: span {span:?} is in no field"));
                    rustc_missing_pieces.insert(at);
                }
                if message.starts_with("invalid reference") {
                    invalid_reference = Some(referenced_indices(message));
                } else {
                    implicit_message = Some(
                        message
                            .split(' ')
                            .next()
                            .expect("count")
                            .parse::<usize>()
                            .expect("number of positional arguments"),
                    );
                }
            } else if error["code"] == "E0425" {
                rustc_captures.insert(unresolved_name(message));
            } else if message.starts_with("format argument must be a string literal") {
                panic!("{name}: a literal template is no literal-less call");
            } else {
                panic!("{name}: unclassified rustc error {message:?}");
            }
        }
        assert_eq!(
            binding.unused.iter().copied().collect::<BTreeSet<_>>(),
            rustc_unused,
            "{name}: unused arguments"
        );

        // Missing fields.
        let missing: BTreeSet<usize> = binding.missing.iter().copied().collect();
        if let Some(indices) = invalid_reference {
            assert_eq!(missing, rustc_missing_pieces, "{name}: missing fields");
            let ours: BTreeSet<usize> = binding
                .fields
                .iter()
                .flat_map(|f| [Some(&f.value), f.width.as_ref(), f.precision.as_ref()])
                .flatten()
                .filter_map(|slot| match slot {
                    Slot::Missing(index) => Some(*index),
                    _ => None,
                })
                .collect();
            assert_eq!(ours, indices, "{name}: invalid indices");
        } else if let Some(needed) = implicit_message {
            // rustc lists every field that takes an implicit index, whether or not it is the
            // one past the end, and counts those indices.
            let implicit: BTreeSet<usize> = pieces
                .iter()
                .enumerate()
                .filter(|(_, piece)| {
                    matches!(
                        piece,
                        Piece::Field {
                            argument: Argument::Next,
                            ..
                        }
                    ) || matches!(
                        piece,
                        Piece::Field {
                            precision: Some(Count::Star),
                            ..
                        }
                    )
                })
                .map(|(at, _)| at)
                .collect();
            let count: usize = pieces
                .iter()
                .map(|piece| match piece {
                    Piece::Field {
                        argument,
                        precision,
                        ..
                    } => {
                        usize::from(*argument == Argument::Next)
                            + usize::from(*precision == Some(Count::Star))
                    }
                    Piece::Literal { .. } => 0,
                })
                .sum();
            assert_eq!(rustc_missing_pieces, implicit, "{name}: implicit fields");
            assert_eq!(needed, count, "{name}: implicit argument count");
            assert!(
                !missing.is_empty() && missing.is_subset(&rustc_missing_pieces),
                "{name}: missing {missing:?} within {rustc_missing_pieces:?}"
            );
        } else {
            assert!(missing.is_empty(), "{name}: spurious missing {missing:?}");
        }

        // Variables the call captures but the case does not define.
        let mut values = BTreeMap::new();
        let items_root = GrammarDialect::Rust.ast_grep(format!("fn main() {{ {items} }}"));
        let_values(&items_root.root(), &mut values);
        let defined: BTreeSet<String> = items_root
            .root()
            .dfs()
            .filter(|node| node.kind() == "let_declaration")
            .filter_map(|node| node.field("pattern"))
            .filter(|pattern| pattern.kind() == "identifier")
            .map(|pattern| pattern.text().to_string())
            .collect();
        let ours: BTreeSet<String> = binding
            .fields
            .iter()
            .flat_map(|f| [Some(&f.value), f.width.as_ref(), f.precision.as_ref()])
            .flatten()
            .filter_map(|slot| match slot {
                Slot::Capture(variable) if !defined.contains(variable) => Some(variable.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(ours, rustc_captures, "{name}: unresolved captures");

        if !case["accepted"].as_bool().expect("accepted") {
            continue;
        }
        // An accepted call has nothing unused or missing, and its fields fill with the
        // arguments that make rustc's recorded output.
        assert!(
            binding.unused.is_empty() && binding.missing.is_empty(),
            "{name}"
        );
        match render(&call, &pieces, &binding, &values) {
            Ok(text) => {
                let expected = format!(
                    "{}{}",
                    case["stdout"].as_str().expect("stdout"),
                    case["stderr"].as_str().expect("stderr")
                );
                let (expected, text) =
                    (expected.trim_end_matches('\n'), text.trim_end_matches('\n'));
                let exact = !matches!(
                    call.macro_name.as_str(),
                    "assert_eq"
                        | "assert_ne"
                        | "debug_assert_eq"
                        | "debug_assert_ne"
                        | "unreachable"
                        | "todo"
                        | "unimplemented"
                );
                if exact {
                    assert_eq!(text, expected, "{name}: output");
                } else {
                    assert!(
                        expected.contains(text),
                        "{name}: {text:?} not in {expected:?}"
                    );
                }
                rendered += 1;
            }
            Err(reason) => skipped.push((name.to_string(), reason)),
        }
    }
    assert!(checked >= 140, "only {checked} calls checked");
    assert_eq!(unknown, 6, "calls with an unknown format string");
    assert!(
        rendered >= 76,
        "only {rendered} outputs rendered, skipped {skipped:?}"
    );
    assert!(
        skipped.len() <= 8,
        "too many unrendered outputs: {skipped:?}"
    );
}

/// Parses `src` as a statement and returns its first macro call.
fn call_of(src: &str) -> Option<(String, usize, Vec<String>, Vec<String>)> {
    let prefix = "fn main() { ";
    let root = GrammarDialect::Rust.ast_grep(format!("{prefix}{src}; }}"));
    let call = format_call(&invocation_at(&root.root(), prefix.len())?)?;
    let arguments = call
        .arguments
        .iter()
        .map(|a| {
            let node_text = |node: &Node<'_, _>| node.text().to_string();
            format!("{}..{}", node_text(&a.first), node_text(&a.last))
        })
        .collect();
    Some((call.value, call.positional, call.named, arguments))
}

#[test]
fn arguments_keep_their_first_and_last_token() {
    let (value, positional, named, arguments) =
        call_of(r#"write!(out, "{} {x}", f(1, 2), x = a.b + 1,)"#).expect("a format call");
    assert_eq!(value, "{} {x}");
    assert_eq!(positional, 1);
    assert_eq!(named, vec!["x".to_string()]);
    assert_eq!(arguments, vec!["f..(1, 2)".to_string(), "a..1".to_string()]);
}

#[test]
fn turbofish_and_cast_generics_keep_their_commas() {
    let (_, positional, _, arguments) =
        call_of(r#"println!("{}", HashMap::<K, V>::new().len())"#).expect("a format call");
    assert_eq!((positional, arguments.len()), (1, 1));
    let (_, positional, _, arguments) =
        call_of(r#"println!("{} {}", x as Foo<A, B>, y)"#).expect("a format call");
    assert_eq!((positional, arguments.len()), (2, 2));
}

#[test]
fn std_paths_and_other_macros() {
    assert!(call_of(r#"std::println!("{}", 1)"#).is_some());
    assert!(call_of(r#"vec!["{}", 1]"#).is_none());
    assert!(call_of(r#"println!(concat!("{}", "x"), 1)"#).is_none());
    assert!(call_of(r#"println!(b"{}", 1)"#).is_none());
}

/// The binding of the call `src` makes: its unused argument indices, its missing pieces and the
/// variables it captures.
fn bound(src: &str) -> (Vec<usize>, Vec<usize>, Vec<String>) {
    let prefix = "fn main() { ";
    let root = GrammarDialect::Rust.ast_grep(format!("{prefix}{src}; }}"));
    let call = format_call(&invocation_at(&root.root(), prefix.len()).expect("a call"))
        .expect("a format call");
    let pieces = parse(&call.value).expect("a template");
    let binding = bind(&pieces, call.positional, &call.named);
    let captures = binding
        .fields
        .iter()
        .flat_map(|f| [Some(&f.value), f.width.as_ref(), f.precision.as_ref()])
        .flatten()
        .filter_map(|slot| match slot {
            Slot::Capture(variable) => Some(variable.clone()),
            _ => None,
        })
        .collect();
    (binding.unused.clone(), binding.missing.clone(), captures)
}

#[test]
fn comments_are_no_arguments() {
    // rustc 1.99.0: "invalid reference to positional argument 1 (there is 1 argument)", and
    // "argument never used" for the `0`.
    let (unused, missing, _) = bound(r#"println!(/* explanation */ "{1}", 0)"#);
    assert_eq!((unused, missing), (vec![0], vec![0]));
    // rustc 1.99.0 prints 2 and 3.
    let (_, positional, named, arguments) =
        call_of(r#"println!("{x}", /* c */ x = 2)"#).expect("a format call");
    assert_eq!((positional, named), (0, vec!["x".to_string()]));
    assert_eq!(arguments, vec!["2..2".to_string()]);
    assert_eq!(
        bound(r#"println!("{x}", /* c */ x = 2)"#),
        (vec![], vec![], vec![])
    );
    assert_eq!(
        bound("println!(\"{}\", // why\n 3)"),
        (vec![], vec![], vec![])
    );
}

#[test]
fn raw_identifier_arguments_are_named_without_their_prefix() {
    // rustc 1.99.0 accepts the call and prints 1.
    let (_, _, named, arguments) =
        call_of(r#"format!("{type}", r#type = 1)"#).expect("a format call");
    assert_eq!(named, vec!["type".to_string()]);
    assert_eq!(arguments, vec!["1..1".to_string()]);
    assert_eq!(
        bound(r#"format!("{type}", r#type = 1)"#),
        (vec![], vec![], vec![])
    );
}
