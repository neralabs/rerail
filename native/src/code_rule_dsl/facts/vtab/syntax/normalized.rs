//! A string literal's normalized text, with a map from that text back to the literal's source (decision 0058 §3).
//!
//! `str(n, v)` is a language's decoded string: the text a program holds at run time, escapes resolved. SQL in
//! `"SELECT \"name\" FROM t"` parses only once the escapes are gone. A guest made from a string is parsed from this
//! text, and its nodes need places in the file: the map says, for each stretch of the decoded text, where it came
//! from. A stretch copied from the source keeps its columns byte for byte; an escape's output stands at the escape.
//!
//! The scanner only cuts the body into copied runs and escapes. Each escape is decoded by the language's own decoder
//! (`core_rr::string_contents`, which `str` uses), and the joined text must equal that decoder's value for the whole
//! literal: when it does not (a rule the scanner does not know, universal newlines), there is no map, and the guest
//! stands at the literal as before.

use super::core_rr::{string_contents, Family};

/// One stretch of a decoded text: from decoded byte `d`, at source byte `s` (relative to the literal's start); an
/// escape's stretch stands whole at its source, a copied one byte for byte. The last entry ends the text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Piece {
    pub d: u32,
    pub s: i64,
    pub escape: bool,
}

/// A literal's decoded text and its map (`pieces`, sorted by `d`, ending with the text's end).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Normalized {
    pub text: String,
    pub pieces: Vec<Piece>,
}

impl Normalized {
    /// Whether the text is copied whole: one stretch, no escapes (a `Placement::Copied`).
    pub(crate) fn copied(&self) -> Option<i64> {
        match self.pieces.as_slice() {
            [p, end] if !p.escape && end.s - p.s == i64::from(end.d - p.d) => Some(p.s - i64::from(p.d)),
            _ => None,
        }
    }
}

/// The string literal `text` of `kind` in a `label` file, normalized, with its map; `None` when it is not a constant
/// string (bytes, an f-string, a template with substitutions) or the language's decoder disagrees with the map.
pub(crate) fn normalized(label: &str, kind: &str, text: &str) -> Option<Normalized> {
    if !matches!(kind, "string" | "string_literal" | "raw_string_literal" | "interpreted_string_literal") {
        return None;
    }
    let family = Family::of(Some(label));
    let (prefix, value) = string_contents(family, text)?;
    if family == Family::Python && prefix.contains(['b', 'f', 't']) {
        return None;
    }
    if family == Family::Rust && prefix.contains('b') {
        return None;
    }
    let (a, b) = body(family, &prefix, text)?;
    let raw = prefix.contains('r');
    let mut out = Normalized { text: String::with_capacity(b - a), pieces: Vec::new() };
    let mut run = a;
    let mut i = a;
    let bytes = text.as_bytes();
    let push = |out: &mut Normalized, s: usize, escape: bool| {
        let d = out.text.len() as u32;
        let s = s as i64;
        match out.pieces.last_mut() {
            Some(last) if last.d == d => *last = Piece { d, s, escape },
            // A copied stretch right after a copied one with the same offset is the same stretch.
            Some(last) if !escape && !last.escape && last.s - i64::from(last.d) == s - i64::from(d) => {}
            _ => out.pieces.push(Piece { d, s, escape }),
        }
    };
    while i < b {
        if raw || bytes[i] != b'\\' {
            i += 1;
            continue;
        }
        let len = escape_len(family, &text[i..b])?;
        if run < i {
            push(&mut out, run, false);
            out.text.push_str(&text[run..i]);
        }
        push(&mut out, i, true);
        out.text.push_str(&decode_escape(family, &text[i..i + len])?);
        i += len;
        run = i;
    }
    if run < b {
        push(&mut out, run, false);
        out.text.push_str(&text[run..b]);
    }
    out.pieces.push(Piece { d: out.text.len() as u32, s: b as i64, escape: false });
    (out.text == value).then_some(out)
}

/// The byte range of a literal's body in `text`: after its prefix and opening quote, before its closing one.
fn body(family: Family, prefix: &str, text: &str) -> Option<(usize, usize)> {
    let mut a = prefix.len();
    let mut hashes = 0;
    if family == Family::Rust && prefix.contains('r') {
        hashes = text[a..].bytes().take_while(|&c| c == b'#').count();
        a += hashes;
    }
    let rest = &text[a..];
    let quote = ["\"\"\"", "'''", "\"", "'", "`"].into_iter().find(|q| rest.starts_with(q))?;
    if family != Family::Python && quote.len() == 3 {
        return None;
    }
    let end = text.len().checked_sub(quote.len() + hashes)?;
    let a = a + quote.len();
    (a <= end && text[end..].starts_with(quote)).then_some((a, end))
}

/// The length in bytes of the escape `s` starts with (`s` starts with a backslash).
fn escape_len(family: Family, s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let c = *b.get(1)?;
    let digits = |from: usize, max: usize, ok: fn(u8) -> bool| from + b[from..].iter().take(max).take_while(|&&c| ok(c)).count();
    let braced = |from: usize| b[from..].iter().position(|&c| c == b'}').map(|p| from + p + 1);
    let newline = |from: usize| if b.get(from) == Some(&b'\r') && b.get(from + 1) == Some(&b'\n') { from + 2 } else { from + 1 };
    Some(match (family, c) {
        (_, b'\r' | b'\n') if family != Family::Rust => newline(1),
        // Rust skips the newline and the next line's leading whitespace.
        (Family::Rust, b'\r' | b'\n') => {
            let n = newline(1);
            n + b[n..].iter().take_while(|c| c.is_ascii_whitespace()).count()
        }
        (Family::Python, b'0'..=b'7') => digits(1, 3, |c| (b'0'..=b'7').contains(&c)),
        (Family::Python, b'N') if b.get(2) == Some(&b'{') => braced(3)?,
        (Family::Python, b'u') => 6,
        (Family::Python, b'U') => 10,
        (_, b'x') => 4,
        (Family::Rust | Family::Ecma, b'u') if b.get(2) == Some(&b'{') => braced(3)?,
        (Family::Ecma, b'u') => 6,
        // Any other escape is the backslash and one character.
        _ => 1 + s[1..].chars().next()?.len_utf8(),
    }
    .min(s.len()))
}

/// An escape's decoded text, by the language's decoder over a literal holding only it.
fn decode_escape(family: Family, escape: &str) -> Option<String> {
    string_contents(family, &format!("\"{escape}\"")).map(|(_, v)| v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(n: &Normalized) -> Vec<(u32, i64, bool)> {
        n.pieces.iter().map(|p| (p.d, p.s, p.escape)).collect()
    }

    #[test]
    fn a_plain_literal_is_copied() {
        let n = normalized("python", "string", "'SELECT 1'").unwrap();
        assert_eq!(n.text, "SELECT 1");
        assert_eq!(n.copied(), Some(1));
        let n = normalized("rust", "raw_string_literal", "r#\"a \"b\"\"#").unwrap();
        assert_eq!(n.text, "a \"b\"");
        assert_eq!(n.copied(), Some(3));
    }

    #[test]
    fn escapes_stand_at_their_source() {
        // "SELECT \"name\" FROM t": the quotes are escapes, `name` and the rest are copied.
        let src = r#""SELECT \"name\" FROM t""#;
        let n = normalized("python", "string", src).unwrap();
        assert_eq!(n.text, "SELECT \"name\" FROM t");
        assert_eq!(map(&n), [(0, 1, false), (7, 8, true), (8, 10, false), (12, 14, true), (13, 16, false), (20, 23, false)]);
        let n = normalized("rust", "string_literal", r#""a\x42\u{43}d""#).unwrap();
        assert_eq!(n.text, "aBCd");
        assert_eq!(map(&n), [(0, 1, false), (1, 2, true), (2, 6, true), (3, 12, false), (4, 13, false)]);
        let n = normalized("javascript", "string", r"'a\nb\u0041'").unwrap();
        assert_eq!(n.text, "a\nbA");
        assert_eq!(map(&n), [(0, 1, false), (1, 2, true), (2, 4, false), (3, 5, true), (4, 11, false)]);
    }

    #[test]
    fn what_is_not_a_constant_string_has_no_text() {
        assert_eq!(normalized("python", "string", "b'x'"), None);
        assert_eq!(normalized("python", "string", "f'{x}'"), None);
        assert_eq!(normalized("python", "integer", "1"), None);
    }
}
