//! Rust's `format_args!` templates, as rustc 1.99.0 reads them: `parse` is `rustc_parse_format`
//! (the template's pieces, every spec validated as `rustc_builtin_macros` does), `bind` is the
//! argument resolution of `rustc_builtin_macros::format` (which argument each field and count
//! reads, which arguments no field reads, which references name an argument that is not there).
//!
//! Ranges are byte offsets in the decoded template (the string literal's value); mapping them
//! back to the source is the caller's, as for any escaped literal. Word characters (argument and
//! count names) are read as `char::is_alphabetic`/`is_alphanumeric` plus `_`, an approximation of
//! rustc's XID tables (the two differ on combining marks and a few format characters).

use super::offsets::{Decode, Decoded, Span};
use super::{Grammar, PNode};
use crate::code_rule_dsl::value::rust::IntTy;
use crate::code_rule_dsl::value::{BigInt, MAX_STRING_BYTES, Value};
use ast_grep_core::{Doc, Node};
use std::ops::Range;

/// A template rustc refuses: what it says and the byte offset it points at.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FormatError {
    pub(crate) message: String,
    pub(crate) at: usize,
}

/// Which argument a field formats.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Argument {
    /// `{}`: the next positional argument.
    Next,
    /// `{0}`.
    Index(usize),
    /// `{name}`: a named argument, or else a captured variable.
    Name(String),
}

/// A width or a precision.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Count {
    /// `{:5}`.
    Literal(usize),
    /// `{:1$}`.
    Index(usize),
    /// `{:w$}`.
    Name(String),
    /// `{:.*}`: the next positional argument, taken before the field's own value.
    Star,
}

/// A field's alignment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Align {
    Left,
    Right,
    Center,
}

/// A field's sign flag.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Sign {
    Plus,
    Minus,
}

/// The formatting trait a field's type names.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Trait {
    /// No type: `Display`.
    Display,
    /// `?`.
    Debug,
    /// `x?`.
    DebugLowerHex,
    /// `X?`.
    DebugUpperHex,
    /// `e`.
    LowerExp,
    /// `E`.
    UpperExp,
    /// `o`.
    Octal,
    /// `p`.
    Pointer,
    /// `b`.
    Binary,
    /// `x`.
    LowerHex,
    /// `X`.
    UpperHex,
}

/// A field's fill, alignment, flags and type; its width and precision are the field's.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Spec {
    pub(crate) fill: Option<char>,
    pub(crate) align: Option<Align>,
    pub(crate) sign: Option<Sign>,
    /// The `#` flag.
    pub(crate) alternate: bool,
    /// The `0` flag.
    pub(crate) zero: bool,
    pub(crate) ty: Trait,
}

impl Default for Spec {
    fn default() -> Self {
        Self {
            fill: None,
            align: None,
            sign: None,
            alternate: false,
            zero: false,
            ty: Trait::Display,
        }
    }
}

/// A template's part.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Piece {
    /// Text to print. After a `{{` or `}}` escape the range starts at the escape's second brace,
    /// so the range is the text printed.
    Literal { range: Range<usize> },
    /// A `{..}` field.
    Field {
        /// From the opening brace through the closing one.
        range: Range<usize>,
        argument: Argument,
        width: Option<Count>,
        precision: Option<Count>,
        /// What follows the `:` up to the closing brace's leading whitespace; empty, after the
        /// argument's own whitespace, when there is no `:`.
        spec_range: Range<usize>,
        spec: Spec,
    },
}

/// Reads `template` as `format_args!` does.
pub(crate) fn parse(template: &str) -> Result<Vec<Piece>, FormatError> {
    let mut parser = Parser {
        chars: template.char_indices().collect(),
        at: 0,
        len: template.len(),
    };
    let mut pieces = Vec::new();
    while let Some(c) = parser.peek() {
        match c {
            '{' => {
                let open = parser.offset();
                parser.at += 1;
                if parser.peek() == Some('{') {
                    pieces.push(parser.escaped_literal());
                } else {
                    pieces.push(parser.field(open)?);
                }
            }
            '}' => {
                let at = parser.offset();
                parser.at += 1;
                if parser.peek() == Some('}') {
                    pieces.push(parser.escaped_literal());
                } else {
                    return Err(FormatError {
                        message: "invalid format string: unmatched `}` found".to_string(),
                        at,
                    });
                }
            }
            _ => pieces.push(parser.literal()),
        }
    }
    Ok(pieces)
}

/// A recursive-descent reader of the template's characters.
struct Parser {
    chars: Vec<(usize, char)>,
    at: usize,
    len: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).map(|(_, c)| *c)
    }

    fn peek_ahead(&self) -> Option<char> {
        self.chars.get(self.at + 1).map(|(_, c)| *c)
    }

    /// The byte offset of the next character (the template's length at its end).
    fn offset(&self) -> usize {
        self.chars.get(self.at).map_or(self.len, |(at, _)| *at)
    }

    fn consume(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn ws(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.at += 1;
        }
    }

    fn error<T>(&self, message: String) -> Result<T, FormatError> {
        Err(FormatError {
            message: format!("invalid format string: {message}"),
            at: self.offset(),
        })
    }

    /// Text up to the next brace, starting at the current character.
    fn literal(&mut self) -> Piece {
        let start = self.offset();
        while self.peek().is_some_and(|c| c != '{' && c != '}') {
            self.at += 1;
        }
        Piece::Literal {
            range: start..self.offset(),
        }
    }

    /// The text from the second brace of an escape (the current character) up to the next brace.
    fn escaped_literal(&mut self) -> Piece {
        let start = self.offset();
        self.at += 1;
        while self.peek().is_some_and(|c| c != '{' && c != '}') {
            self.at += 1;
        }
        Piece::Literal {
            range: start..self.offset(),
        }
    }

    /// A field, after its opening brace at `open`.
    fn field(&mut self, open: usize) -> Result<Piece, FormatError> {
        let argument = self.position()?;
        self.ws();
        let mut width = None;
        let mut precision = None;
        let mut spec = Spec::default();
        let spec_start;
        if self.consume(':') {
            spec_start = self.offset();
            self.format(&mut spec, &mut width, &mut precision)?;
        } else {
            spec_start = self.offset();
        }
        let spec_end = self.offset();
        self.ws();
        if !self.consume('}') {
            return match self.peek() {
                Some(c) => self.error(format!("expected `}}`, found `{}`", c.escape_debug())),
                None => self.error("expected `}` but string was terminated".to_string()),
            };
        }
        Ok(Piece::Field {
            range: open..self.offset(),
            argument,
            width,
            precision,
            spec_range: spec_start..spec_end,
            spec,
        })
    }

    /// An index, a name, or (when neither) the next argument.
    fn position(&mut self) -> Result<Argument, FormatError> {
        if let Some(index) = self.integer()? {
            return Ok(Argument::Index(index));
        }
        if !self.peek().is_some_and(is_id_start) {
            return Ok(Argument::Next);
        }
        let word = self.word()?;
        if word == "r" && self.peek() == Some('#') && self.peek_ahead().is_some_and(is_id_start) {
            return self.error("raw identifiers are not supported".to_string());
        }
        Ok(Argument::Name(word))
    }

    /// The part after `:`: `[[fill]align][sign]['#']['0'][width]['.' precision][type]`.
    fn format(
        &mut self,
        spec: &mut Spec,
        width: &mut Option<Count>,
        precision: &mut Option<Count>,
    ) -> Result<(), FormatError> {
        if let (Some(c), Some('<' | '>' | '^')) = (self.peek(), self.peek_ahead()) {
            self.at += 1;
            spec.fill = Some(c);
        }
        if self.consume('<') {
            spec.align = Some(Align::Left);
        } else if self.consume('>') {
            spec.align = Some(Align::Right);
        } else if self.consume('^') {
            spec.align = Some(Align::Center);
        }
        if self.consume('+') {
            spec.sign = Some(Sign::Plus);
        } else if self.consume('-') {
            spec.sign = Some(Sign::Minus);
        }
        spec.alternate = self.consume('#');
        let mut have_width = false;
        if self.consume('0') {
            // `0$` is the width argument 0, not the flag followed by a stray `$`.
            if self.consume('$') {
                *width = Some(Count::Index(0));
                have_width = true;
            } else {
                spec.zero = true;
            }
        }
        if !have_width {
            *width = self.count()?;
        }
        if self.consume('.') {
            *precision = if self.consume('*') {
                Some(Count::Star)
            } else {
                self.count()?
            };
        }
        spec.ty = self.type_name()?;
        Ok(())
    }

    /// The trait letter: `x`, `X` (alone or before `?`), `?`, or a word that must be a trait.
    fn type_name(&mut self) -> Result<Trait, FormatError> {
        if self.consume('x') {
            return Ok(if self.consume('?') {
                Trait::DebugLowerHex
            } else {
                Trait::LowerHex
            });
        }
        if self.consume('X') {
            return Ok(if self.consume('?') {
                Trait::DebugUpperHex
            } else {
                Trait::UpperHex
            });
        }
        if self.consume('?') {
            return Ok(Trait::Debug);
        }
        let at = self.offset();
        match self.word()?.as_str() {
            "" => Ok(Trait::Display),
            "e" => Ok(Trait::LowerExp),
            "E" => Ok(Trait::UpperExp),
            "o" => Ok(Trait::Octal),
            "p" => Ok(Trait::Pointer),
            "b" => Ok(Trait::Binary),
            other => Err(FormatError {
                message: format!("unknown format trait `{other}`"),
                at,
            }),
        }
    }

    /// A width or precision: an integer, `integer$`, `name$`; `None` when there is none (a word
    /// without `$` is left for the type).
    fn count(&mut self) -> Result<Option<Count>, FormatError> {
        if let Some(value) = self.integer()? {
            return Ok(Some(if self.consume('$') {
                Count::Index(value)
            } else {
                Count::Literal(value)
            }));
        }
        let start = self.at;
        let word = self.word()?;
        if word.is_empty() {
            return Ok(None);
        }
        if self.consume('$') {
            return Ok(Some(Count::Name(word)));
        }
        self.at = start;
        Ok(None)
    }

    /// An identifier or keyword; empty when none starts here. A lone `_` is refused.
    fn word(&mut self) -> Result<String, FormatError> {
        let mut word = String::new();
        if !self.peek().is_some_and(is_id_start) {
            return Ok(word);
        }
        let start = self.offset();
        while let Some(c) = self.peek().filter(|c| is_id_continue(*c)) {
            word.push(c);
            self.at += 1;
        }
        if word == "_" {
            return Err(FormatError {
                message: "invalid format string: invalid argument name `_`".to_string(),
                at: start,
            });
        }
        Ok(word)
    }

    /// Decimal digits as rustc's `u16`: a larger number is refused.
    fn integer(&mut self) -> Result<Option<usize>, FormatError> {
        let start = self.offset();
        let mut value: Option<u16> = None;
        let mut overflow = false;
        while let Some(digit) = self.peek().and_then(|c| c.to_digit(10)) {
            self.at += 1;
            let so_far = value.unwrap_or(0);
            match so_far
                .checked_mul(10)
                .and_then(|n| n.checked_add(digit as u16))
            {
                Some(next) => value = Some(next),
                None => {
                    overflow = true;
                    value = Some(so_far);
                }
            }
        }
        if overflow {
            return Err(FormatError {
                message: "invalid format string: integer does not fit into the type `u16` whose range is `0..=65535`"
                    .to_string(),
                at: start,
            });
        }
        Ok(value.map(usize::from))
    }
}

fn is_id_start(c: char) -> bool {
    c == '_' || c.is_alphabetic()
}

fn is_id_continue(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

/// What a field's value or a count reads.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Slot {
    /// The argument at this index (positional ones first, then the named ones in order).
    Argument(usize),
    /// A variable in scope: `{name}` or `{:name$}` that no named argument is called.
    Capture(String),
    /// An index past the last argument.
    Missing(usize),
}

/// What one field reads.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FieldBinding {
    /// The field's index in the pieces.
    pub(crate) piece: usize,
    pub(crate) value: Slot,
    /// `None` without a width, or with a literal one.
    pub(crate) width: Option<Slot>,
    /// `None` without a precision, or with a literal one.
    pub(crate) precision: Option<Slot>,
}

/// A template resolved against a call's arguments.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Binding {
    /// Every field's slots, in template order.
    pub(crate) fields: Vec<FieldBinding>,
    /// Indices of the arguments nothing reads (rustc's "argument never used").
    pub(crate) unused: Vec<usize>,
    /// Piece indices of the fields with a slot past the last argument (rustc's "invalid
    /// reference to positional argument").
    pub(crate) missing: Vec<usize>,
}

/// Resolves `pieces` against a call with `positional` unnamed arguments followed by the
/// arguments called `named`. A precision `*` takes the next implicit index before the field's own
/// value does; `{name}` and `{:name$}` that no named argument is called capture a variable.
pub(crate) fn bind(pieces: &[Piece], positional: usize, named: &[String]) -> Binding {
    let total = positional + named.len();
    let slot_for_index = |index: usize| {
        if index < total {
            Slot::Argument(index)
        } else {
            Slot::Missing(index)
        }
    };
    let slot_for_name = |name: &String| match named.iter().position(|n| n == name) {
        Some(at) => Slot::Argument(positional + at),
        None => Slot::Capture(name.clone()),
    };
    let mut next = 0usize;
    let mut take_next = || {
        let index = next;
        next += 1;
        index
    };
    let mut fields = Vec::new();
    for (piece, part) in pieces.iter().enumerate() {
        let Piece::Field {
            argument,
            width,
            precision,
            ..
        } = part
        else {
            continue;
        };
        let count = |count: &Count, take_next: &mut dyn FnMut() -> usize| match count {
            Count::Literal(_) => None,
            Count::Index(index) => Some(slot_for_index(*index)),
            Count::Name(name) => Some(slot_for_name(name)),
            Count::Star => Some(slot_for_index(take_next())),
        };
        let width = width.as_ref().and_then(|c| count(c, &mut take_next));
        let precision = precision.as_ref().and_then(|c| count(c, &mut take_next));
        let value = match argument {
            Argument::Next => slot_for_index(take_next()),
            Argument::Index(index) => slot_for_index(*index),
            Argument::Name(name) => slot_for_name(name),
        };
        fields.push(FieldBinding {
            piece,
            value,
            width,
            precision,
        });
    }
    let mut used = vec![false; total];
    let mut missing = Vec::new();
    for field in &fields {
        let slots = [
            Some(&field.value),
            field.width.as_ref(),
            field.precision.as_ref(),
        ];
        let mut is_missing = false;
        for slot in slots.into_iter().flatten() {
            match slot {
                Slot::Argument(index) => used[*index] = true,
                Slot::Missing(_) => is_missing = true,
                Slot::Capture(_) => {}
            }
        }
        if is_missing {
            missing.push(field.piece);
        }
    }
    let unused = used
        .iter()
        .enumerate()
        .filter(|(_, used)| !**used)
        .map(|(index, _)| index)
        .collect();
    Binding {
        fields,
        unused,
        missing,
    }
}

/// The kinds of a `rustFormat` tree: a `format` root holding `literal` and `field` parts. A field's
/// value is the argument it names (an explicit index, or a name; none for `{}`); it holds `auto`
/// (zero width, for `{}`), `index` or `name`, then its `format_spec` (whose value is the spec's
/// text) with the `type` of its formatting trait (`?`, `x`, `e`, ...) where it names one. A
/// template rustc refuses is a `format` root with one `error` node, whose value is rustc's message.
pub(crate) const KINDS: &[&str] = &[
    "format",
    "literal",
    "field",
    "auto",
    "index",
    "name",
    "format_spec",
    "type",
    "error",
];

const G: Grammar = Grammar::RustFormat;

/// The `rustFormat` tree of a decoded template.
pub(crate) fn tree(text: &str) -> PNode {
    let mut root = PNode::new(G, "format", 0, text.len());
    match parse(text) {
        Ok(pieces) => root.children = pieces.iter().map(|piece| piece_node(text, piece)).collect(),
        Err(error) => {
            let mut node = PNode::new(G, "error", error.at.min(text.len()), text.len());
            node.error = true;
            node.value = Some(Value::Str(error.message));
            root.children = vec![node];
        }
    }
    root
}

fn piece_node(text: &str, piece: &Piece) -> PNode {
    let Piece::Field {
        range,
        argument,
        spec_range,
        spec,
        ..
    } = piece
    else {
        let Piece::Literal { range } = piece else {
            unreachable!("a piece is a literal or a field")
        };
        return PNode::new(G, "literal", range.start, range.end)
            .with_value(Value::Str(text[range.clone()].to_owned()));
    };
    // The argument runs from after the `{` to the `:` before the spec, or to the closing brace.
    let argument_end = if text[..spec_range.start].ends_with(':') {
        spec_range.start - 1
    } else {
        range.end - 1
    };
    let written = &text[range.start + 1..argument_end.max(range.start + 1)];
    let start = range.start + 1 + (written.len() - written.trim_start().len());
    let end = start + written.trim().len();
    let (kind, value) = match argument {
        Argument::Next => ("auto", None),
        Argument::Index(index) => ("index", Some(usize_value(*index))),
        Argument::Name(name) => ("name", Some(Value::Str(name.clone()))),
    };
    let mut field = PNode::new(G, "field", range.start, range.end);
    field.value = value.clone();
    let mut name = PNode::new(G, kind, start, end);
    name.value = value;
    field.children.push(name);
    if !spec_range.is_empty() {
        let mut node = PNode::new(G, "format_spec", spec_range.start, spec_range.end)
            .with_value(Value::Str(text[spec_range.clone()].to_owned()));
        let written = trait_text(spec.ty);
        if !written.is_empty() && text[spec_range.clone()].ends_with(written) {
            node.children.push(
                PNode::new(G, "type", spec_range.end - written.len(), spec_range.end)
                    .with_value(Value::Str(written.to_owned())),
            );
        }
        field.children.push(node);
    }
    field
}

/// An argument index as Rust reads it: a `usize`.
fn usize_value(index: usize) -> Value {
    Value::RustInt {
        value: BigInt::from(index),
        ty: Some(IntTy::Usize),
    }
}

/// How a spec writes its formatting trait.
fn trait_text(ty: Trait) -> &'static str {
    match ty {
        Trait::Display => "",
        Trait::Debug => "?",
        Trait::DebugLowerHex => "x?",
        Trait::DebugUpperHex => "X?",
        Trait::LowerExp => "e",
        Trait::UpperExp => "E",
        Trait::Octal => "o",
        Trait::Pointer => "p",
        Trait::Binary => "b",
        Trait::LowerHex => "x",
        Trait::UpperHex => "X",
    }
}

/// Decodes a tree-sitter-rust `string_literal` or `raw_string_literal` as the `str` rustc reads,
/// with the source span of every decoded character: what a format template is. A byte or C string
/// is no template (`NotString`); an escape rustc refuses, and a value past 64 KiB, are `Unknown`.
pub(crate) fn decode<D: Doc>(node: &Node<'_, D>) -> Decode {
    let text = node.text();
    let base = node.range().start;
    let mut decoded = Decoded {
        text: String::new(),
        spans: Vec::new(),
        host: node.range(),
        simple: None,
    };
    match node.kind().as_ref() {
        "string_literal" if text.starts_with('"') => {
            if text.len() < 2 || !text.ends_with('"') {
                return Decode::Unknown;
            }
            if !unescape(&text[1..text.len() - 1], base + 1, &mut decoded) {
                return Decode::Unknown;
            }
        }
        "raw_string_literal" if text.starts_with('r') => {
            let hashes = text[1..].bytes().take_while(|byte| *byte == b'#').count();
            let open = 2 + hashes;
            if text.len() < open + 1 + hashes || text.as_bytes().get(open - 1) != Some(&b'"') {
                return Decode::Unknown;
            }
            let body = &text[open..text.len() - 1 - hashes];
            for (at, c) in body.char_indices() {
                let start = base + open + at;
                push(&mut decoded, c, start..start + c.len_utf8());
            }
        }
        _ => return Decode::NotString,
    }
    if decoded.text.len() > MAX_STRING_BYTES {
        return Decode::Unknown;
    }
    Decode::Known(decoded)
}

fn push(decoded: &mut Decoded, c: char, source: Range<usize>) {
    decoded.spans.push(Span {
        at: decoded.text.len() as u32,
        start: source.start as u32,
        end: source.end as u32,
    });
    decoded.text.push(c);
}

/// Decodes a non-raw string's body starting at source byte `base`; `false` for an escape rustc
/// refuses.
fn unescape(body: &str, base: usize, decoded: &mut Decoded) -> bool {
    let bytes = body.as_bytes();
    let mut at = 0;
    while at < body.len() {
        let c = body[at..].chars().next().expect("a char");
        if c != '\\' {
            // A bare carriage return is refused in a string literal.
            if c == '\r' && bytes.get(at + 1) != Some(&b'\n') {
                return false;
            }
            push(decoded, c, base + at..base + at + c.len_utf8());
            at += c.len_utf8();
            continue;
        }
        let start = at;
        let Some(&escape) = bytes.get(at + 1) else {
            return false;
        };
        let (value, next) = match escape {
            b'n' => (Some('\n'), at + 2),
            b'r' => (Some('\r'), at + 2),
            b't' => (Some('\t'), at + 2),
            b'\\' => (Some('\\'), at + 2),
            b'0' => (Some('\0'), at + 2),
            b'\'' => (Some('\''), at + 2),
            b'"' => (Some('"'), at + 2),
            b'x' => {
                let Some(code) = body
                    .get(at + 2..at + 4)
                    .and_then(|hex| u8::from_str_radix(hex, 16).ok())
                    .filter(|code| *code <= 0x7f)
                else {
                    return false;
                };
                (Some(char::from(code)), at + 4)
            }
            b'u' => {
                let Some(close) = body[at..].find('}').map(|close| at + close) else {
                    return false;
                };
                let Some(digits) = body.get(at + 2..close).and_then(|d| d.strip_prefix('{')) else {
                    return false;
                };
                let digits: String = digits.chars().filter(|c| *c != '_').collect();
                let Some(c) = (!digits.is_empty() && digits.len() <= 6)
                    .then(|| u32::from_str_radix(&digits, 16).ok())
                    .flatten()
                    .and_then(char::from_u32)
                else {
                    return false;
                };
                (Some(c), close + 1)
            }
            // A line continuation skips the newline and the whitespace after it.
            b'\n' | b'\r' => {
                let rest = &body[at + 1..];
                let skipped = rest.len()
                    - rest
                        .trim_start_matches(|c: char| matches!(c, ' ' | '\t' | '\n' | '\r'))
                        .len();
                (None, at + 1 + skipped)
            }
            _ => return false,
        };
        if let Some(value) = value {
            push(decoded, value, base + start..base + next);
        }
        at = next;
    }
    true
}

#[cfg(test)]
mod tests;
