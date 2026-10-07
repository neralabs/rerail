//! Python values (tree-sitter-python 0.25): literals as CPython reads them, and the operators and
//! builtin constructors over them folded as CPython evaluates them.

mod ops;

use super::{BigInt, ContainerKind, Context, MAX_INT_BITS, MAX_STRING_BYTES, Value, int_to_float};
use crate::policy_dsl::identifier::python_identifier;
use ast_grep_core::{Doc, Node};

/// Decodes `node` to its value; `Unknown` for anything that is not a literal this knows.
pub(crate) fn decode<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>) -> Value {
    if !context.enter() {
        return Value::Unknown;
    }
    let value = decode_node(node, context);
    context.leave();
    if context.exhausted() {
        return Value::Unknown;
    }
    value
}

fn decode_node<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>) -> Value {
    match node.kind().as_ref() {
        "integer" => integer(&node.text()),
        "float" => float(&node.text()),
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        "none" => Value::None,
        "ellipsis" => Value::Ellipsis,
        "string" => string(node),
        "concatenated_string" => concatenated(node),
        "parenthesized_expression" => match single_child(node) {
            Some(inner) => decode(&inner, context),
            None => Value::Unknown,
        },
        "unary_operator" => unary(node, context),
        "not_operator" => match node.field("argument") {
            Some(operand) => ops::unary("not", &decode(&operand, context)),
            None => Value::Unknown,
        },
        "binary_operator" => binary(node, context),
        "boolean_operator" => boolean(node, context),
        "comparison_operator" => comparison(node, context),
        "conditional_expression" => conditional(node, context),
        "call" => call(node, context),
        "tuple" => sequence(node, ContainerKind::Tuple, context),
        "list" => sequence(node, ContainerKind::List, context),
        "set" => sequence(node, ContainerKind::Set, context),
        "dictionary" => sequence(node, ContainerKind::Dict, context),
        "identifier" => follow(node, context),
        _ => Value::Unknown,
    }
}

/// A name's value through its binding (`value.follow`). A list, set or dict may be mutated
/// anywhere, so only its kind carries over, not its length or emptiness.
fn follow<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>) -> Value {
    let Some(followed) = context.follow(node.range(), &python_identifier(node)) else {
        return Value::Unknown;
    };
    let Some(target) = followed.value.find_near(node.root().root(), node) else {
        return Value::Unknown;
    };
    match decode(&target, context) {
        Value::Container { kind, .. }
            if matches!(
                kind,
                ContainerKind::List | ContainerKind::Set | ContainerKind::Dict
            ) =>
        {
            Value::Container {
                kind,
                len: None,
                nonempty: None,
            }
        }
        value => value,
    }
}

/// Expressions: the subtypes of the grammar's `expression` and `primary_expression` supertypes
/// (tree-sitter-python 0.25.0 `node-types.json`), and bare tuples. Only an expression has a value;
/// on any other node (a statement, a keyword argument, punctuation) every value predicate fails.
pub(crate) const EXPRESSION_KINDS: &[&str] = &[
    "as_pattern",
    "attribute",
    "await",
    "binary_operator",
    "boolean_operator",
    "call",
    "comparison_operator",
    "concatenated_string",
    "conditional_expression",
    "dictionary",
    "dictionary_comprehension",
    "ellipsis",
    "expression_list",
    "false",
    "float",
    "generator_expression",
    "identifier",
    "integer",
    "lambda",
    "list",
    "list_comprehension",
    "list_splat",
    "named_expression",
    "none",
    "not_operator",
    "parenthesized_expression",
    "set",
    "set_comprehension",
    "string",
    "subscript",
    "true",
    "tuple",
    "unary_operator",
    "yield",
];

/// Kinds whose value `decode` can know; any other expression is `Unknown`.
pub(crate) const VALUE_KINDS: &[&str] = &[
    "integer",
    "float",
    "true",
    "false",
    "none",
    "ellipsis",
    "string",
    "concatenated_string",
    "parenthesized_expression",
    "unary_operator",
    "not_operator",
    "binary_operator",
    "boolean_operator",
    "comparison_operator",
    "conditional_expression",
    "call",
    "tuple",
    "list",
    "set",
    "dictionary",
];

/// Named children other than comments.
fn operands<'r, D: Doc>(node: &Node<'r, D>) -> Vec<Node<'r, D>> {
    node.children()
        .filter(|child| child.is_named() && child.kind() != "comment")
        .collect()
}

/// The one named child of a parenthesized expression, comments aside; a parenthesized yield or a
/// generator has other shapes.
pub(super) fn single_child<'r, D: Doc>(node: &Node<'r, D>) -> Option<Node<'r, D>> {
    let mut named = node
        .children()
        .filter(|child| child.is_named() && child.kind() != "comment");
    match (named.next(), named.next()) {
        (Some(inner), None) => Some(inner),
        _ => None,
    }
}

/// An `int` literal: `0x`, `0o`, `0b` or decimal digits with `_` separators; with `j`, an imaginary
/// `complex`. Python 2's `L` suffix and leading zeros are syntax errors.
pub(crate) fn integer(text: &str) -> Value {
    let digits = text.replace('_', "");
    if let Some(imaginary) = digits.strip_suffix(['j', 'J']) {
        return match imaginary.parse::<f64>() {
            Ok(value) => Value::Complex(0.0, value),
            Err(_) => Value::Unknown,
        };
    }
    let lower = digits.to_ascii_lowercase();
    let (radix, body) = if let Some(body) = lower.strip_prefix("0x") {
        (16, body)
    } else if let Some(body) = lower.strip_prefix("0o") {
        (8, body)
    } else if let Some(body) = lower.strip_prefix("0b") {
        (2, body)
    } else {
        if lower.len() > 1 && lower.starts_with('0') && lower.bytes().any(|byte| byte != b'0') {
            return Value::Unknown;
        }
        (10, lower.as_str())
    };
    if body.is_empty() || !body.chars().all(|c| c.is_digit(radix)) {
        return Value::Unknown;
    }
    // Every digit after the first nonzero one adds at least `floor(log2(radix))` bits, so a
    // literal far past the cap is refused before it is parsed.
    let significant = body.trim_start_matches('0');
    let least_bits = match radix {
        16 => 4,
        8 => 3,
        10 => 3,
        _ => 1,
    };
    if (significant.len().saturating_sub(1) as u64).saturating_mul(least_bits) > MAX_INT_BITS {
        return Value::Unknown;
    }
    match BigInt::parse_bytes(body.as_bytes(), radix) {
        Some(value) => Value::int(value),
        None => Value::Unknown,
    }
}

/// A `float` literal (`1.`, `.5`, `1e5`, `_` separators); with `j`, an imaginary `complex`.
pub(crate) fn float(text: &str) -> Value {
    let digits = text.replace('_', "");
    let (imaginary, digits) = match digits.strip_suffix(['j', 'J']) {
        Some(digits) => (true, digits),
        None => (false, digits.as_str()),
    };
    if !digits
        .bytes()
        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'.' | b'e' | b'E' | b'+' | b'-'))
    {
        return Value::Unknown;
    }
    match digits.parse::<f64>() {
        Ok(value) if imaginary => Value::Complex(0.0, value),
        Ok(value) => Value::Float(value),
        Err(_) => Value::Unknown,
    }
}

/// What a string literal's prefix makes it.
pub(crate) struct Prefix {
    pub(crate) raw: bool,
    pub(crate) bytes: bool,
    pub(crate) format: bool,
    pub(crate) template: bool,
}

/// A literal's prefix letters, in any case; `None` for a combination Python refuses.
pub(crate) fn prefix(letters: &str) -> Option<Prefix> {
    let lower = letters.to_ascii_lowercase();
    let valid = matches!(
        lower.as_str(),
        "" | "r" | "u" | "b" | "br" | "rb" | "f" | "fr" | "rf" | "t" | "tr" | "rt"
    );
    valid.then(|| Prefix {
        raw: lower.contains('r'),
        bytes: lower.contains('b'),
        format: lower.contains('f'),
        template: lower.contains('t'),
    })
}

/// A decoded piece of a string literal.
pub(super) enum Piece {
    Str(String),
    Bytes(Vec<u8>),
}

/// A `string` node: its prefix and quotes from the source text, its body decoded. An f-string with
/// a replacement field is not known.
fn string<D: Doc>(node: &Node<'_, D>) -> Value {
    if node.children().any(|child| child.kind() == "interpolation") {
        return Value::Unknown;
    }
    match piece(&node.text()) {
        Some(Piece::Str(value)) => Value::Str(value),
        Some(Piece::Bytes(value)) => Value::Bytes(value),
        None => Value::Unknown,
    }
}

/// The value of a string literal as Ruff's `ExprStringLiteral` reads it: a `string`, or an implicit
/// concatenation of them, none of them an f-, t- or bytes literal; escapes decoded.
pub(crate) fn string_literal<D: Doc>(node: &Node<'_, D>) -> Option<String> {
    let parts = match node.kind().as_ref() {
        "string" => vec![node.clone()],
        "concatenated_string" => node
            .children()
            .filter(|child| child.is_named() && child.kind() != "comment")
            .collect(),
        _ => return None,
    };
    let mut value = String::new();
    for part in parts {
        let text = part.text();
        let quote_at = text.find(['\'', '"'])?;
        let letters = prefix(&text[..quote_at])?;
        if part.kind() != "string" || letters.format || letters.template || letters.bytes {
            return None;
        }
        match piece(&text)? {
            Piece::Str(piece) => value.push_str(&piece),
            Piece::Bytes(_) => return None,
        }
        if value.len() > MAX_STRING_BYTES {
            return None;
        }
    }
    Some(value)
}

/// Adjacent literals join; `str` and `bytes` do not mix.
fn concatenated<D: Doc>(node: &Node<'_, D>) -> Value {
    let mut joined: Option<Piece> = None;
    for part in node.children().filter(|child| child.kind() == "string") {
        if part.children().any(|child| child.kind() == "interpolation") {
            return Value::Unknown;
        }
        let Some(next) = piece(&part.text()) else {
            return Value::Unknown;
        };
        joined = Some(match (joined, next) {
            (None, next) => next,
            (Some(Piece::Str(mut left)), Piece::Str(right)) => {
                left.push_str(&right);
                Piece::Str(left)
            }
            (Some(Piece::Bytes(mut left)), Piece::Bytes(right)) => {
                left.extend(right);
                Piece::Bytes(left)
            }
            _ => return Value::Unknown,
        });
        let size = match &joined {
            Some(Piece::Str(value)) => value.len(),
            Some(Piece::Bytes(value)) => value.len(),
            None => 0,
        };
        if size > MAX_STRING_BYTES {
            return Value::Unknown;
        }
    }
    match joined {
        Some(Piece::Str(value)) => Value::Str(value),
        Some(Piece::Bytes(value)) => Value::Bytes(value),
        None => Value::Unknown,
    }
}

/// Decodes one literal's source text: prefix, quotes, body. A template (`t`) string is no `str`.
fn piece(text: &str) -> Option<Piece> {
    let quote_at = text.find(['\'', '"'])?;
    let prefix = prefix(&text[..quote_at])?;
    if prefix.template {
        return None;
    }
    let rest = &text[quote_at..];
    let quote = if rest.starts_with("'''") || rest.starts_with("\"\"\"") {
        &rest[..3]
    } else {
        &rest[..1]
    };
    if rest.len() < 2 * quote.len() || !rest.ends_with(quote) {
        return None;
    }
    decode_body(&rest[quote.len()..rest.len() - quote.len()], &prefix)
}

/// Decodes a literal's body (or an f-string's literal text) under its prefix: newlines read as
/// universal newlines, `{{` and `}}` in an f-string, escapes unless raw.
pub(super) fn decode_body(body: &str, prefix: &Prefix) -> Option<Piece> {
    // The tokenizer reads source with universal newlines.
    let mut body = body.replace("\r\n", "\n").replace('\r', "\n");
    if prefix.format || prefix.template {
        body = body.replace("{{", "{").replace("}}", "}");
    }
    if prefix.bytes {
        if !body.is_ascii() {
            return None;
        }
        let value = if prefix.raw {
            body.into_bytes()
        } else {
            decode_escapes(&body, true)?
                .into_iter()
                .map(|unit| u8::try_from(unit).ok())
                .collect::<Option<Vec<_>>>()?
        };
        return (value.len() <= MAX_STRING_BYTES).then_some(Piece::Bytes(value));
    }
    let value = if prefix.raw {
        body
    } else {
        decode_escapes(&body, false)?
            .into_iter()
            .map(char::from_u32)
            .collect::<Option<String>>()?
    };
    (value.len() <= MAX_STRING_BYTES).then_some(Piece::Str(value))
}

/// Python's escape sequences, as code points (bytes for a `bytes` literal). `None` for a malformed
/// or out-of-range escape, a name `\N{...}` does not know, or a lone surrogate (which `String`
/// cannot hold).
fn decode_escapes(body: &str, bytes: bool) -> Option<Vec<u32>> {
    let mut out = Vec::with_capacity(body.len());
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(u32::from(c));
            continue;
        }
        let escape = chars.next()?;
        let simple = match escape {
            '\n' => continue,
            '\\' => Some(u32::from('\\')),
            '\'' => Some(u32::from('\'')),
            '"' => Some(u32::from('"')),
            'a' => Some(7),
            'b' => Some(8),
            'f' => Some(12),
            'n' => Some(10),
            'r' => Some(13),
            't' => Some(9),
            'v' => Some(11),
            _ => None,
        };
        if let Some(code) = simple {
            out.push(code);
            continue;
        }
        let hex = |chars: &mut std::iter::Peekable<std::str::Chars<'_>>, count: usize| {
            let digits = chars.by_ref().take(count).collect::<String>();
            (digits.len() == count && digits.chars().all(|c| c.is_ascii_hexdigit()))
                .then(|| u32::from_str_radix(&digits, 16).ok())
                .flatten()
        };
        match escape {
            '0'..='7' => {
                let mut code = escape.to_digit(8)?;
                for _ in 0..2 {
                    match chars.peek().and_then(|c| c.to_digit(8)) {
                        Some(digit) => {
                            code = code * 8 + digit;
                            chars.next();
                        }
                        None => break,
                    }
                }
                // In bytes an escape past 0o377 keeps its low byte (a SyntaxWarning).
                out.push(if bytes { code & 0xff } else { code });
            }
            'x' => out.push(hex(&mut chars, 2)?),
            'u' if !bytes => out.push(scalar(hex(&mut chars, 4)?)?),
            'U' if !bytes => out.push(scalar(hex(&mut chars, 8)?)?),
            // `\N{name}`: a character by its Unicode name or alias.
            'N' if !bytes => {
                if chars.next()? != '{' {
                    return None;
                }
                let mut name = String::new();
                loop {
                    match chars.next()? {
                        '}' => break,
                        c => name.push(c),
                    }
                }
                out.push(u32::from(super::names::lookup(&name)?));
            }
            // Anything else keeps its backslash.
            other => {
                out.push(u32::from('\\'));
                out.push(u32::from(other));
            }
        }
    }
    Some(out)
}

/// A code point a Rust `char` can hold: not a surrogate, at most U+10FFFF.
fn scalar(code: u32) -> Option<u32> {
    char::from_u32(code).map(u32::from)
}

/// `+x`, `-x`, `~x`.
fn unary<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>) -> Value {
    let (Some(operator), Some(operand)) = (node.field("operator"), node.field("argument")) else {
        return Value::Unknown;
    };
    let operand = decode(&operand, context);
    ops::unary(&operator.text(), &operand)
}

/// Arithmetic, bitwise and sequence operators; `1+2j` is how a complex constant is spelled.
fn binary<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>) -> Value {
    let (Some(left), Some(operator), Some(right)) = (
        node.field("left"),
        node.field("operator"),
        node.field("right"),
    ) else {
        return Value::Unknown;
    };
    let left = decode(&left, context);
    if !left.is_known() {
        return Value::Unknown;
    }
    let right = decode(&right, context);
    ops::binary(&operator.text(), &left, &right)
}

/// `a and b`, `a or b`: the operand that decides, which need not be a `bool`.
fn boolean<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>) -> Value {
    let (Some(left), Some(operator), Some(right)) = (
        node.field("left"),
        node.field("operator"),
        node.field("right"),
    ) else {
        return Value::Unknown;
    };
    let left_value = decode(&left, context);
    let Some(truthy) = left_value.truthy(super::Lang::Python) else {
        return Value::Unknown;
    };
    match (operator.text().as_ref(), truthy) {
        ("or", true) | ("and", false) => left_value,
        ("or", false) | ("and", true) => decode(&right, context),
        _ => Value::Unknown,
    }
}

/// A chain `a < b <= c`: each operand once, False at the first comparison that fails.
fn comparison<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>) -> Value {
    let mut operands = Vec::new();
    let mut operators = Vec::new();
    for child in node.children() {
        if child.kind() == "comment" {
            continue;
        }
        if child.is_named() {
            operands.push(child);
        } else {
            operators.push(child.kind().to_string());
        }
    }
    if operands.len() != operators.len() + 1 || operators.is_empty() {
        return Value::Unknown;
    }
    let mut left = decode(&operands[0], context);
    for (operator, right) in operators.iter().zip(&operands[1..]) {
        let right = decode(right, context);
        match ops::compare(operator, &left, &right) {
            Some(true) => left = right,
            Some(false) => return Value::Bool(false),
            None => return Value::Unknown,
        }
    }
    Value::Bool(true)
}

/// `a if test else b`.
fn conditional<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>) -> Value {
    let [chosen, test, alternative] = &operands(node)[..] else {
        return Value::Unknown;
    };
    let (chosen, test, alternative) = (chosen.clone(), test.clone(), alternative.clone());
    match decode(&test, context).truthy(super::Lang::Python) {
        Some(true) => decode(&chosen, context),
        Some(false) => decode(&alternative, context),
        None => Value::Unknown,
    }
}

/// Builtin constructors called with no argument or one positional argument, where the name is the
/// builtin (not shadowed in the file): `bool(x)`, `int()`, `float()`, `str()`, `bytes()`,
/// `tuple(x)`, `list(x)`, `set(x)` and `dict(x)`, so far as the result is known.
fn call<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>) -> Value {
    let (Some(function), Some(arguments)) = (node.field("function"), node.field("arguments"))
    else {
        return Value::Unknown;
    };
    if function.kind() != "identifier" || arguments.kind() != "argument_list" {
        return Value::Unknown;
    }
    let name = python_identifier(&function);
    let builtin = [
        "bool", "int", "float", "str", "bytes", "tuple", "list", "set", "dict",
    ];
    if !builtin.contains(&name.as_ref()) {
        return Value::Unknown;
    }
    let arguments = operands(&arguments);
    let argument = match arguments[..] {
        [] => None,
        [ref argument]
            if !matches!(
                argument.kind().as_ref(),
                "keyword_argument" | "list_splat" | "dictionary_splat" | "parenthesized_list_splat"
            ) =>
        {
            Some(argument.clone())
        }
        _ => return Value::Unknown,
    };
    if !context.is_global(function.range(), &name) {
        return Value::Unknown;
    }
    let Some(argument) = argument else {
        return match name.as_ref() {
            "bool" => Value::Bool(false),
            "int" => Value::int(0),
            "float" => Value::Float(0.0),
            "str" => Value::Str(String::new()),
            "bytes" => Value::Bytes(Vec::new()),
            "tuple" => Value::Tuple(Vec::new()),
            kind => Value::Container {
                kind: match kind {
                    "list" => ContainerKind::List,
                    "set" => ContainerKind::Set,
                    _ => ContainerKind::Dict,
                },
                len: Some(0),
                nonempty: Some(false),
            },
        };
    };
    let value = decode(&argument, context);
    match (name.as_ref(), &value) {
        (_, Value::Unknown) => Value::Unknown,
        ("bool", value) => value
            .truthy(super::Lang::Python)
            .map_or(Value::Unknown, Value::Bool),
        ("int", Value::Int(_)) | ("float", Value::Float(_)) | ("str", Value::Str(_)) => value,
        ("int", Value::Bool(value)) => Value::int(u8::from(*value)),
        ("float", Value::Int(value)) => int_to_float(value).map_or(Value::Unknown, Value::Float),
        ("float", Value::Bool(value)) => Value::Float(f64::from(u8::from(*value))),
        ("tuple", Value::Tuple(_)) => value,
        ("tuple" | "list" | "set" | "dict", value) => iterable(&name, value),
        _ => Value::Unknown,
    }
}

/// `tuple(x)`, `list(x)`, `set(x)`, `dict(x)` of a known iterable: its emptiness carries over.
fn iterable(name: &str, value: &Value) -> Value {
    let (len, nonempty) = match value {
        Value::Str(text) => (Some(text.chars().count()), Some(!text.is_empty())),
        Value::Bytes(bytes) => (Some(bytes.len()), Some(!bytes.is_empty())),
        Value::Tuple(elements) => (Some(elements.len()), Some(!elements.is_empty())),
        Value::Container {
            kind:
                ContainerKind::Tuple | ContainerKind::List | ContainerKind::Set | ContainerKind::Dict,
            len,
            nonempty,
        } => (*len, *nonempty),
        _ => return Value::Unknown,
    };
    // `dict` needs pairs, and a string's characters are not.
    if name == "dict"
        && !matches!(
            value,
            Value::Container {
                kind: ContainerKind::Dict,
                ..
            }
        )
    {
        return Value::Unknown;
    }
    let kind = match name {
        "tuple" => ContainerKind::Tuple,
        "list" => ContainerKind::List,
        "set" => ContainerKind::Set,
        _ => ContainerKind::Dict,
    };
    let exact = matches!(kind, ContainerKind::Tuple | ContainerKind::List)
        || len.is_some_and(|len| len < 2);
    Value::Container {
        kind,
        len: len.filter(|_| exact),
        nonempty,
    }
}

/// A display: a tuple whose elements are all known is a value; otherwise, and for lists, sets and
/// dicts, only its kind, length and emptiness are, so far as splats allow.
fn sequence<D: Doc>(node: &Node<'_, D>, kind: ContainerKind, context: &mut Context<'_>) -> Value {
    let elements = node
        .children()
        .filter(|child| child.is_named() && child.kind() != "comment")
        .collect::<Vec<_>>();
    let splat = |element: &Node<'_, D>| {
        matches!(
            element.kind().as_ref(),
            "list_splat" | "dictionary_splat" | "parenthesized_list_splat"
        )
    };
    let splats = elements.iter().filter(|element| splat(element)).count();
    // A comprehension has other kinds; these displays hold expressions, pairs and splats only.
    if kind == ContainerKind::Tuple && splats == 0 {
        let mut values = Vec::with_capacity(elements.len());
        for element in &elements {
            match decode(element, context) {
                Value::Unknown => {
                    values.clear();
                    break;
                }
                value => values.push(value),
            }
        }
        if values.len() == elements.len() {
            return Value::Tuple(values);
        }
    }
    let exact = splats == 0
        && (matches!(kind, ContainerKind::Tuple | ContainerKind::List) || elements.len() < 2);
    Value::Container {
        kind,
        len: exact.then_some(elements.len()),
        nonempty: if splats < elements.len() {
            Some(true)
        } else if elements.is_empty() {
            Some(false)
        } else {
            None
        },
    }
}

/// Python's `str(v)` of a known scalar: a `str` itself, an `int` in decimal, a `float` as `repr`
/// writes it, `True`, `False`, `None` and `Ellipsis`; `None` for any other value.
pub(crate) fn to_str(value: &Value) -> Option<String> {
    Some(match value {
        Value::Str(text) => text.clone(),
        Value::Int(int) => int.to_string(),
        Value::Float(float) => float_repr(*float),
        Value::Bool(true) => "True".to_owned(),
        Value::Bool(false) => "False".to_owned(),
        Value::None => "None".to_owned(),
        Value::Ellipsis => "Ellipsis".to_owned(),
        _ => return None,
    })
}

/// CPython's `float_repr` (`repr` style `'r'`): the shortest digits that round-trip, positional
/// where the decimal point falls within 16 digits of them (then with `.0` if integral), else
/// scientific with a signed exponent of at least two digits.
pub(crate) fn float_repr(value: f64) -> String {
    if value.is_nan() {
        return "nan".to_owned();
    }
    if value.is_infinite() {
        return if value > 0.0 { "inf" } else { "-inf" }.to_owned();
    }
    let sign = if value.is_sign_negative() { "-" } else { "" };
    if value == 0.0 {
        return format!("{sign}0.0");
    }
    // Rust's shortest round-trip digits, as `d.ddde[-]x`.
    let scientific = format!("{:e}", value.abs());
    let (mantissa, exponent) = scientific
        .split_once('e')
        .expect("LowerExp writes an exponent");
    let exponent: i32 = exponent.parse().expect("an integer exponent");
    let digits = mantissa.replace('.', "");
    let point = exponent + 1;
    let body = if -4 < point && point <= 16 {
        if point <= 0 {
            format!("0.{}{digits}", "0".repeat(point.unsigned_abs() as usize))
        } else if point as usize >= digits.len() {
            format!("{digits}{}.0", "0".repeat(point as usize - digits.len()))
        } else {
            format!(
                "{}.{}",
                &digits[..point as usize],
                &digits[point as usize..]
            )
        }
    } else {
        let head = if digits.len() == 1 {
            digits.clone()
        } else {
            format!("{}.{}", &digits[..1], &digits[1..])
        };
        let exponent_sign = if exponent < 0 { '-' } else { '+' };
        format!("{head}e{exponent_sign}{:02}", exponent.unsigned_abs())
    };
    format!("{sign}{body}")
}

#[cfg(test)]
mod tests;
