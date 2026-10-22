//! `parse: { grammar: pythonFString }` (native `literal/python_fstring.rs` + `literal/offsets.rs`):
//! a JoinedStr's literal parts and opaque interpolation slots, each with its decoded text and the
//! host bytes it stands for. A derived per-node fact (census category c).
use crate::facts::{self, Facts};
use crate::pyvalue::{MAX_STRING_BYTES, Prefix, prefix};

#[derive(Clone, Copy)]
struct Span {
    at: u32,
    start: u32,
    end: u32,
}

struct Decoded {
    text: String,
    spans: Vec<Span>,
    host_end: usize,
    host_start: usize,
}

impl Decoded {
    fn push(&mut self, c: char, start: usize, end: usize) {
        self.spans.push(Span { at: self.text.len() as u32, start: start as u32, end: end as u32 });
        self.text.push(c);
    }
    fn host_range(&self, start: usize, end: usize) -> (u32, u32) {
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

fn decode_body(body: &str, base: usize, prefix: &Prefix, d: &mut Decoded) -> Result<bool, String> {
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

/// The parsed pieces of `n` (a `string` or `concatenated_string`).
pub fn parse(f: &Facts, n: u32) -> Result<FParse, String> {
    let kind = facts::name(f.kind[n as usize]);
    let parts: Vec<u32> = if kind == "concatenated_string" {
        f.children(n).iter().copied().filter(|&c| facts::name(f.kind[c as usize]) == "string").collect()
    } else {
        vec![n]
    };
    let mut d = Decoded { text: String::new(), spans: Vec::new(), host_start: f.start[n as usize] as usize, host_end: f.end[n as usize] as usize };
    let _ = d.host_end;
    let mut children: Vec<(&'static str, usize, usize)> = Vec::new();
    let mut literal_start = 0usize;
    let marker = "{}";
    let mut formatted = false;
    let flush = |d: &Decoded, children: &mut Vec<(&'static str, usize, usize)>, literal_start: &mut usize| {
        let end = d.text.len();
        if *literal_start < end {
            children.push(("literal", *literal_start, end));
        }
        *literal_start = end;
    };
    for part in parts {
        let Some(start) = f.children(part).iter().copied().find(|&c| facts::name(f.kind[c as usize]) == "string_start") else {
            return Ok(FParse::NotString);
        };
        let text = f.text(start);
        let Some(quote) = text.find(['\'', '"']) else { return Ok(FParse::NotString) };
        let Some(pref) = prefix(&text[..quote]) else { return Ok(FParse::NotString) };
        if pref.bytes || pref.template {
            return Ok(FParse::NotString);
        }
        formatted |= pref.format;
        for &child in f.children(part) {
            match facts::name(f.kind[child as usize]).as_str() {
                "string_content" => {
                    if !decode_body(f.text(child), f.start[child as usize] as usize, &pref, &mut d)? || d.text.len() > MAX_STRING_BYTES {
                        return Ok(FParse::Unknown);
                    }
                }
                "interpolation" => {
                    // Debug form `{x=}`: the expression source, '=' and trailing space are literal.
                    if f.children(child).iter().any(|&c| facts::name(f.kind[c as usize]) == "=") {
                        let (rs, re) = (f.start[child as usize] as usize, f.end[child as usize] as usize);
                        let field_end = f
                            .children(child)
                            .iter()
                            .copied()
                            .find(|&c| {
                                let fname = field_name(f, c);
                                fname == "type_conversion" || fname == "format_specifier"
                            })
                            .map_or(re - 1, |c| f.start[c as usize] as usize);
                        let ftext = f.text(child);
                        if let Some(debug) = ftext.get(1..field_end - rs) {
                            if d.text.len().saturating_add(debug.len()) > MAX_STRING_BYTES {
                                return Ok(FParse::Unknown);
                            }
                            for (at, ch) in debug.char_indices() {
                                d.push(ch, rs + 1 + at, rs + 1 + at + ch.len_utf8());
                            }
                        }
                    }
                    flush(&d, &mut children, &mut literal_start);
                    let s = d.text.len();
                    if s.saturating_add(marker.len()) > MAX_STRING_BYTES {
                        return Ok(FParse::Unknown);
                    }
                    for ch in marker.chars() {
                        d.push(ch, f.start[child as usize] as usize, f.end[child as usize] as usize);
                    }
                    children.push(("interpolation", s, d.text.len()));
                    literal_start = d.text.len();
                }
                _ => {}
            }
        }
        if d.text.len() > MAX_STRING_BYTES {
            return Ok(FParse::Unknown);
        }
    }
    if !formatted {
        return Ok(FParse::NotString);
    }
    flush(&d, &mut children, &mut literal_start);
    Ok(FParse::Tree(
        children
            .into_iter()
            .map(|(kind, s, e)| {
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
