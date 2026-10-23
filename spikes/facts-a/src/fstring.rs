//! `parse: { grammar: pythonFString }` (native `literal/python_fstring.rs` + `literal/offsets.rs`):
//! a JoinedStr's literal parts and opaque interpolation slots, each with its decoded text and the
//! host bytes it stands for. A derived per-node fact (census category c). The decoder
//! (`Decoded`, `decode_body`) is shared with `identifierWords` (`ast/parse_ext.rs`).
use crate::facts::{self, Facts};
use crate::pyvalue::{MAX_STRING_BYTES, Prefix, prefix};

#[derive(Clone, Copy)]
pub(crate) struct Span {
    pub(crate) at: u32,
    pub(crate) start: u32,
    pub(crate) end: u32,
}

pub(crate) struct Decoded {
    pub(crate) text: String,
    pub(crate) spans: Vec<Span>,
    /// The literal node's first source byte (native `Decoded::host.start`).
    pub(crate) host_start: usize,
}

impl Decoded {
    pub(crate) fn push(&mut self, c: char, start: usize, end: usize) {
        self.spans.push(Span { at: self.text.len() as u32, start: start as u32, end: end as u32 });
        self.text.push(c);
    }
    pub(crate) fn host_range(&self, start: usize, end: usize) -> (u32, u32) {
        let index = |at: usize| self.spans.partition_point(|s| (s.at as usize) < at);
        let first = index(start);
        let s = match self.spans.get(first) {
            Some(span) => span.start as usize,
            None => self.spans.last().map_or(self.host_start, |span| span.end as usize),
        };
        if end <= start {
            return (s as u32, s as u32);
        }
        let last = index(end).saturating_sub(1).max(first);
        let e = self.spans.get(last).map_or(s, |span| span.end as usize);
        (s as u32, e.max(s) as u32)
    }
}

pub struct Piece {
    pub kind: &'static str,
    pub start: u32,
    pub end: u32,
    pub text: String,
}

pub(crate) fn decode_body(body: &str, base: usize, prefix: &Prefix, d: &mut Decoded) -> Result<bool, String> {
    let bytes = body.as_bytes();
    let mut index = 0;
    while index < body.len() {
        let c = body[index..].chars().next().unwrap();
        let at = base + index;
        match c {
            '\r' => {
                let w = if bytes.get(index + 1) == Some(&b'\n') { 2 } else { 1 };
                d.push('\n', at, at + w);
                index += w;
            }
            '{' | '}' if prefix.format && bytes.get(index + 1) == Some(&(c as u8)) => {
                d.push(c, at, at + 2);
                index += 2;
            }
            '\\' if !prefix.raw => match escape(body, index, at, d)? {
                Some(next) => index = next,
                None => return Ok(false),
            },
            _ => {
                d.push(c, at, at + c.len_utf8());
                index += c.len_utf8();
            }
        }
    }
    Ok(true)
}

fn escape(body: &str, index: usize, at: usize, d: &mut Decoded) -> Result<Option<usize>, String> {
    let rest = &body[index + 1..];
    let Some(esc) = rest.chars().next() else { return Ok(None) };
    let after = index + 1 + esc.len_utf8();
    let whole = |end: usize| (at, at + (end - index));
    let simple = match esc {
        '\n' => return Ok(Some(after)),
        '\r' => return Ok(Some(if rest.as_bytes().get(1) == Some(&b'\n') { after + 1 } else { after })),
        '\\' => Some('\\'),
        '\'' => Some('\''),
        '"' => Some('"'),
        'a' => Some('\u{7}'),
        'b' => Some('\u{8}'),
        'f' => Some('\u{c}'),
        'n' => Some('\n'),
        'r' => Some('\r'),
        't' => Some('\t'),
        'v' => Some('\u{b}'),
        _ => None,
    };
    if let Some(c) = simple {
        let (s, e) = whole(after);
        d.push(c, s, e);
        return Ok(Some(after));
    }
    let hex = |count: usize| -> Option<(u32, usize)> {
        let digits = rest.get(1..1 + count)?;
        (digits.len() == count && digits.chars().all(|c| c.is_ascii_hexdigit()))
            .then(|| u32::from_str_radix(digits, 16).ok())
            .flatten()
            .map(|code| (code, after + count))
    };
    let (code, end) = match esc {
        '0'..='7' => {
            let mut code = esc.to_digit(8).unwrap();
            let mut end = after;
            for digit in rest[1..].chars().take(2) {
                match digit.to_digit(8) {
                    Some(dg) => {
                        code = code * 8 + dg;
                        end += 1;
                    }
                    None => break,
                }
            }
            (code, end)
        }
        'x' => match hex(2) {
            Some(v) => v,
            None => return Ok(None),
        },
        'u' => match hex(4) {
            Some(v) => v,
            None => return Ok(None),
        },
        'U' => match hex(8) {
            Some(v) => v,
            None => return Ok(None),
        },
        'N' => return Err("`\\N{...}` escape in an f-string needs the Unicode name table".into()),
        other => {
            d.push('\\', at, at + 1);
            d.push(other, at + 1, at + 1 + other.len_utf8());
            return Ok(Some(after));
        }
    };
    let Some(c) = char::from_u32(code) else { return Ok(None) };
    let (s, e) = whole(end);
    d.push(c, s, e);
    Ok(Some(end))
}

pub enum FParse {
    Tree(Vec<Piece>),
    /// Not an f-string this grammar reads (no `f` prefix, bytes, a t-string): no match.
    NotString,
    /// A value the native parse does not know (a refused escape, past 64 KiB): unknown verdict.
    Unknown,
}

fn is(f: &Facts, n: u32, kind: &str) -> bool {
    &*facts::name(f.kind[n as usize]) == kind
}

/// Native `python_fstring::Builder`.
struct Builder {
    d: Decoded,
    children: Vec<(&'static str, usize, usize)>,
    literal_start: usize,
}

impl Builder {
    fn flush(&mut self) {
        let end = self.d.text.len();
        if self.literal_start < end {
            self.children.push(("literal", self.literal_start, end));
        }
        self.literal_start = end;
    }

    fn literal(&mut self, text: &str, start: usize, prefix: &Prefix) -> Result<bool, String> {
        Ok(decode_body(text, start, prefix, &mut self.d)? && self.d.text.len() <= MAX_STRING_BYTES)
    }

    /// An interpolation (or a spec's `format_expression`) as one `marker` slot; false where the
    /// text would pass 64 KiB (Unknown).
    fn field(&mut self, f: &Facts, field: u32, marker: &str) -> bool {
        let (rs, re) = (f.start[field as usize] as usize, f.end[field as usize] as usize);
        // Debug form `{x=}`: the expression source, '=' and trailing space are literal.
        if f.children(field).iter().any(|&c| is(f, c, "=")) {
            let field_end = f
                .children(field)
                .iter()
                .copied()
                .find(|&c| {
                    let fname = field_name(f, c);
                    fname == "type_conversion" || fname == "format_specifier"
                })
                .map_or(re - 1, |c| f.start[c as usize] as usize);
            if let Some(debug) = f.text(field).get(1..field_end - rs) {
                if self.d.text.len().saturating_add(debug.len()) > MAX_STRING_BYTES {
                    return false;
                }
                for (at, ch) in debug.char_indices() {
                    self.d.push(ch, rs + 1 + at, rs + 1 + at + ch.len_utf8());
                }
            }
        }
        self.flush();
        let s = self.d.text.len();
        if s.saturating_add(marker.len()) > MAX_STRING_BYTES {
            return false;
        }
        for ch in marker.chars() {
            self.d.push(ch, rs, re);
        }
        self.children.push(("interpolation", s, self.d.text.len()));
        self.literal_start = self.d.text.len();
        true
    }
}

/// The parsed pieces of `n`, a `string`, `concatenated_string` or `format_specifier` (native
/// pythonFString host kinds, compile/parse.rs `host_kinds`), each interpolation slot's text
/// `marker` (the `interpolation` option, default `{}`; matcher/parse.rs `parsed_tree_with`).
pub fn parse(f: &Facts, n: u32, marker: &str) -> Result<FParse, String> {
    let parts: Vec<u32> = if is(f, n, "concatenated_string") {
        f.children(n).iter().copied().filter(|&c| is(f, c, "string")).collect()
    } else {
        vec![n]
    };
    let mut b = Builder {
        d: Decoded { text: String::new(), spans: Vec::new(), host_start: f.start[n as usize] as usize },
        children: Vec::new(),
        literal_start: 0,
    };
    let mut formatted = is(f, n, "format_specifier");
    for part in parts {
        if is(f, part, "format_specifier") {
            // python_fstring.rs:112-142: the spec's raw slices between format_expression nodes
            // are literal text; the ':' is not part of the value. Raw iff the enclosing string's
            // prefix has an `r`.
            let (ps, text) = (f.start[part as usize] as usize, f.text(part));
            let mut start = ps + 1;
            let mut up = f.parent(part);
            while let Some(a) = up.filter(|&a| !is(f, a, "string")) {
                up = f.parent(a);
            }
            let raw = up
                .and_then(|s| f.children(s).iter().copied().find(|&c| is(f, c, "string_start")))
                .is_some_and(|ss| f.text(ss).to_ascii_lowercase().contains('r'));
            let pref = prefix(if raw { "rf" } else { "f" }).expect("f prefix");
            for &field in f.children(part).iter().filter(|&&c| is(f, c, "format_expression")) {
                let fs = f.start[field as usize] as usize;
                if !b.literal(&text[start - ps..fs - ps], start, &pref)? || !b.field(f, field, marker) {
                    return Ok(FParse::Unknown);
                }
                start = f.end[field as usize] as usize;
            }
            if !b.literal(&text[start - ps..], start, &pref)? {
                return Ok(FParse::Unknown);
            }
            continue;
        }
        // python_fstring.rs:144-149: a part without `string_start`, quote or known prefix makes
        // the native parse return `None`, which `parsed_tree_with` (matcher/parse.rs:281-284)
        // reads as Unknown.
        let Some(start) = f.children(part).iter().copied().find(|&c| is(f, c, "string_start")) else {
            return Ok(FParse::Unknown);
        };
        let text = f.text(start);
        let Some(quote) = text.find(['\'', '"']) else { return Ok(FParse::Unknown) };
        let Some(pref) = prefix(&text[..quote]) else { return Ok(FParse::Unknown) };
        if pref.bytes || pref.template {
            return Ok(FParse::NotString);
        }
        formatted |= pref.format;
        for &child in f.children(part) {
            match &*facts::name(f.kind[child as usize]) {
                "string_content" => {
                    if !b.literal(f.text(child), f.start[child as usize] as usize, &pref)? {
                        return Ok(FParse::Unknown);
                    }
                }
                "interpolation" => {
                    if !b.field(f, child, marker) {
                        return Ok(FParse::Unknown);
                    }
                }
                _ => {}
            }
        }
        if b.d.text.len() > MAX_STRING_BYTES {
            return Ok(FParse::Unknown);
        }
    }
    if !formatted {
        return Ok(FParse::NotString);
    }
    b.flush();
    let d = &b.d;
    Ok(FParse::Tree(
        b.children
            .iter()
            .map(|&(kind, s, e)| {
                let (hs, he) = d.host_range(s, e);
                Piece { kind, start: hs, end: he, text: d.text[s..e].to_owned() }
            })
            .collect(),
    ))
}

fn field_name(f: &Facts, c: u32) -> String {
    let code = f.field[c as usize];
    crate::facts::field_name(code)
}
