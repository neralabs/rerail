//! XML literals (`xml_expression`) and XML patterns (`xml_pattern`): tree-sitter-scala's
//! `_xml_node`s, read on the bytes, the lexer's token buffer cut at the literal's start and again
//! at its end (`P::relex_at`), and at each embedded `{ … }`, which the parser reads as code.
//!
//! - **Runs.** Text (`[^<{}]+`), attribute strings and the ends of comments, CDATA sections and
//!   processing instructions are found 64 bytes a step (`find3`: three byte compares and a
//!   bitmask, over `rerail_simd8`'s backends), as the lexer's masks find a string's end; a name is
//!   the word mask's run (`Blocks::word_end`) and its `-`, `.`, `:`.
//! - **Text and blanks.** The external scanner skips blanks before a tag's `<`: blanks before a
//!   child element are no text. Elsewhere the lexer's `xml_text` takes them (it may start with a
//!   blank), so text holds the blanks around it, and blanks alone before `</`, `{` or `<!--` are
//!   text.
//! - **Tokens.** `<`, `>`, `/>`, `</`, `=`, `{`, `}`, `,` are written as tokens: the parent's
//!   automaton decides which the tape stores, as for any other token.

use super::P;
use crate::kinds::{K, f};
use crate::lex::{R, Refusal, T, decode};
use rerail_simd8::Simd8x64;

/// The first byte from `at` that is `a`, `b` or `c` (or the text's length), 64 bytes a step.
#[inline(always)]
unsafe fn find3_k<S: Simd8x64>(src: &[u8], at: usize, a: u8, b: u8, c: u8) -> usize {
    let mut i = at;
    loop {
        let block: [u8; 64] = match src.get(i..i + 64) {
            Some(s) => s.try_into().expect("64 bytes"),
            None => {
                if i >= src.len() {
                    return src.len();
                }
                // The last block, padded with `a`: a stop at the text's end.
                let mut p = [a; 64];
                p[..src.len() - i].copy_from_slice(&src[i..]);
                p
            }
        };
        let m = unsafe {
            let v = S::load(&block);
            S::to_bitmask(S::or(S::or(S::eq(v, S::splat(a)), S::eq(v, S::splat(b))), S::eq(v, S::splat(c))))
        };
        if m != 0 {
            return (i + m.trailing_zeros() as usize).min(src.len());
        }
        i += 64;
    }
}

rerail_simd8::dispatch! {
    fn find3_best(src: &[u8], at: usize, a: u8, b: u8, c: u8) -> usize = find3_k, on find3_on;
}

#[inline]
fn find3(src: &[u8], at: usize, a: u8, b: u8, c: u8) -> usize {
    find3_best(src, at, a, b, c)
}

/// The end of `close` (`-->`, `]]>`, `?>`) from `at`: the first stop of its first byte where the
/// rest follows.
fn find_close(src: &[u8], mut at: usize, close: &[u8]) -> Option<usize> {
    loop {
        let i = find3(src, at, close[0], close[0], close[0]);
        if i >= src.len() {
            return None;
        }
        if src[i..].starts_with(close) {
            return Some(i + close.len());
        }
        at = i + 1;
    }
}

/// Whether a name may start at `at` (`[_\p{L}]`: past ASCII, the letters read for certain).
fn name_start(src: &[u8], at: usize) -> bool {
    match src.get(at) {
        Some(&c) if c.is_ascii_alphabetic() || c == b'_' => true,
        Some(&c) if c >= 0x80 => matches!(decode(src, at), Some((ch, _)) if ch.is_alphabetic() && letter(ch)),
        _ => false,
    }
}

/// The letters past ASCII a name is read with (Latin, Greek, Cyrillic, kana, CJK, Hangul).
fn letter(c: char) -> bool {
    let u = c as u32;
    matches!(u, 0xc0..=0x24f | 0x370..=0x3ff | 0x400..=0x4ff | 0x3040..=0x30ff | 0x4e00..=0x9fff | 0xac00..=0xd7a3) && u != 0xd7 && u != 0xf7
}

fn is_blank(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

impl P<'_> {
    /// Whether an XML literal starts at the next token: a lone `<` touching a name's start.
    pub(super) fn xml_ahead(&mut self) -> bool {
        let t = self.tok();
        t.kind == T::Op && t.end == t.start + 1 && self.src[t.start as usize] == b'<' && name_start(self.src, t.end as usize)
    }

    /// The end of the blanks from `at` (line ends among them), by the blank mask's runs.
    fn xml_blank_end(&self, mut at: usize) -> usize {
        loop {
            at = self.lx.blocks.blank_end(at);
            match self.src.get(at) {
                Some(b'\n' | b'\r') => at += 1,
                _ => return at,
            }
        }
    }

    /// The end of an XML name from `at` (`[-.:_\p{L}\p{Nd}]*` after its start).
    fn xml_name_end(&self, at: usize) -> R<usize> {
        let mut e = at;
        loop {
            e = self.lx.blocks.word_end(e);
            match self.src.get(e) {
                Some(b'-' | b'.' | b':') => e += 1,
                Some(&c) if c >= 0x80 => match decode(self.src, e) {
                    Some((ch, n)) if ch.is_alphanumeric() && letter(ch) => e += n,
                    Some((ch, _)) if ch.is_alphanumeric() => return Err(Refusal::Lex(e as u32)),
                    _ => return Ok(e),
                },
                _ => return Ok(e),
            }
        }
    }

    /// A token at `s..e` written, the place past it.
    fn xml_tok(&mut self, kind: K, s: usize, e: usize) -> usize {
        self.t.token(kind, f::NONE, s as u32, e as u32);
        e
    }

    fn xml_refuse<X>(&mut self, at: usize) -> R<X> {
        self.far = self.far.max(at as u32);
        Err(Refusal::Parse(at as u32))
    }

    /// The token buffer and the scanner moved to `at`: the next token is lexed from there.
    fn xml_resume(&mut self, at: usize) {
        self.relex_at(at);
        self.sp = at as u32;
        self.t.last_end = self.t.last_end.max(at as u32);
    }

    /// `<a …>…</a>` (`xml_expression`, its nodes): the next token the `<`.
    pub(super) fn xml_expression(&mut self) -> R<u32> {
        let s = self.tok().start as usize;
        self.t.flush(s as u32);
        self.relex_at(s);
        let n = self.t.open(K::XmlExpression, f::NONE, s as u32);
        let (_, e) = self.xml_element(s, false)?;
        // Another node after it on the line (`_xml_node+`): refused.
        let b = self.lx.blocks.blank_end(e);
        if self.src.get(b) == Some(&b'<') && (name_start(self.src, b + 1) || matches!(self.src.get(b + 1), Some(b'!' | b'?'))) {
            return self.xml_refuse(b);
        }
        self.xml_resume(e);
        Ok(self.close(n))
    }

    /// `<a …>…</a>` in a pattern (`xml_pattern`): the next token the `<`.
    pub(super) fn xml_pattern(&mut self) -> R<u32> {
        let s = self.tok().start as usize;
        self.t.flush(s as u32);
        self.relex_at(s);
        let (n, e) = self.xml_element(s, true)?;
        self.xml_resume(e);
        Ok(n)
    }

    /// An element from its `<` at `s` (`xml_element`, or `xml_pattern` in a pattern): its place
    /// and end.
    fn xml_element(&mut self, s: usize, pattern: bool) -> R<(u32, usize)> {
        self.deeper()?;
        let kind = if pattern { K::XmlPattern } else { K::XmlElement };
        let n = self.t.open(kind, f::NONE, s as u32);
        let src = self.src;
        let mut p = self.xml_tok(K::TLt, s, s + 1);
        let e = self.xml_name_end(p)?;
        self.t.leaf(K::XmlName, f::NAME, p as u32, e as u32);
        p = e;
        // The attributes, then `/>` or `>`.
        loop {
            p = self.xml_blank_end(p);
            match src.get(p) {
                Some(b'/') if src.get(p + 1) == Some(&b'>') => {
                    let e = self.xml_tok(K::TX2fX3e, p, p + 2);
                    self.depth -= 1;
                    return Ok((self.close(n), e));
                }
                Some(b'>') => {
                    p = self.xml_tok(K::TGt, p, p + 1);
                    break;
                }
                _ if name_start(src, p) => p = self.xml_attribute(p)?,
                _ => return self.xml_refuse(p),
            }
        }
        // The content, then `</a>`.
        loop {
            match src.get(p) {
                None => return self.xml_refuse(p),
                Some(b'<') => match src.get(p + 1) {
                    Some(b'/') => {
                        let mut q = self.xml_tok(K::TX3cX2f, p, p + 2);
                        q = self.xml_blank_end(q);
                        if !name_start(src, q) {
                            return self.xml_refuse(q);
                        }
                        let e = self.xml_name_end(q)?;
                        self.t.leaf(K::XmlName, f::NONE, q as u32, e as u32);
                        q = self.xml_blank_end(e);
                        if src.get(q) != Some(&b'>') {
                            return self.xml_refuse(q);
                        }
                        let e = self.xml_tok(K::TGt, q, q + 1);
                        self.depth -= 1;
                        return Ok((self.close(n), e));
                    }
                    Some(b'!') if src[p..].starts_with(b"<!--") => {
                        let Some(e) = find_close(src, p + 4, b"-->") else { return self.xml_refuse(p) };
                        // `--` inside a comment is no comment of tree-sitter's.
                        if src[p + 4..e - 3].windows(2).any(|w| w == b"--") || src[p + 4..e - 3].ends_with(b"-") {
                            return self.xml_refuse(p);
                        }
                        self.t.leaf(K::XmlComment, f::NONE, p as u32, e as u32);
                        p = e;
                    }
                    Some(b'!') if src[p..].starts_with(b"<![CDATA[") => {
                        let Some(e) = find_close(src, p + 9, b"]]>") else { return self.xml_refuse(p) };
                        self.t.leaf(K::XmlCdata, f::NONE, p as u32, e as u32);
                        p = e;
                    }
                    Some(b'?') => {
                        let Some(e) = find_close(src, p + 2, b"?>") else { return self.xml_refuse(p) };
                        self.t.leaf(K::XmlProcessingInstruction, f::NONE, p as u32, e as u32);
                        p = e;
                    }
                    _ if name_start(src, p + 1) => p = self.xml_element(p, pattern)?.1,
                    _ => return self.xml_refuse(p),
                },
                Some(b'{') if src.get(p + 1) == Some(&b'{') => {
                    self.t.leaf(K::XmlText, f::NONE, p as u32, p as u32 + 2);
                    p += 2;
                }
                Some(b'}') if src.get(p + 1) == Some(&b'}') => {
                    self.t.leaf(K::XmlText, f::NONE, p as u32, p as u32 + 2);
                    p += 2;
                }
                Some(b'{') => p = if pattern { self.xml_embedded_patterns(p)? } else { self.xml_block(p, f::NONE)? },
                Some(b'}') => return self.xml_refuse(p),
                Some(_) => {
                    let e = find3(src, p, b'<', b'{', b'}');
                    let b = self.xml_blank_end(p);
                    if b == e && src.get(e) == Some(&b'<') && name_start(src, e + 1) {
                        // Blanks before a child element: the scanner's, no text.
                        p = e;
                        continue;
                    }
                    // A comment where the text starts is the lexer's or the scanner's: refused.
                    if src[b..e].starts_with(b"//") || src[b..e].starts_with(b"/*") {
                        return self.xml_refuse(b);
                    }
                    self.t.leaf(K::XmlText, f::NONE, p as u32, e as u32);
                    p = e;
                }
            }
        }
    }

    /// `key="v"`, `key='v'`, `key={e}` (`xml_attribute`: `key`, `value`) from `s`: its end.
    fn xml_attribute(&mut self, s: usize) -> R<usize> {
        let src = self.src;
        let n = self.t.open(K::XmlAttribute, f::NONE, s as u32);
        let e = self.xml_name_end(s)?;
        self.t.leaf(K::XmlName, f::KEY, s as u32, e as u32);
        let mut p = self.xml_blank_end(e);
        if src.get(p) != Some(&b'=') {
            return self.xml_refuse(p);
        }
        p = self.xml_tok(K::TEq, p, p + 1);
        p = self.xml_blank_end(p);
        let e = match src.get(p) {
            Some(&q @ (b'"' | b'\'')) => {
                let e = find3(src, p + 1, q, b'<', q);
                if src.get(e) != Some(&q) {
                    return self.xml_refuse(e);
                }
                self.t.leaf(K::XmlString, f::VALUE, p as u32, e as u32 + 1);
                e + 1
            }
            Some(b'{') if src.get(p + 1) != Some(&b'{') => self.xml_block(p, f::VALUE)?,
            _ => return self.xml_refuse(p),
        };
        self.close(n);
        Ok(e)
    }

    /// `{ … }` at `s` (`block`), read as code, given `field`: its end.
    fn xml_block(&mut self, s: usize, field: u8) -> R<usize> {
        self.relex_at(s);
        self.sp = s as u32;
        let b = self.with_fol(0, |p| p.block())?;
        if field != f::NONE {
            self.t.refield(b, field);
        }
        Ok(self.t.last_end as usize)
    }

    /// `{ p, q }` at `s` in an XML pattern: its patterns (`_xml_embedded_pattern`), `x*` a
    /// `repeat_pattern`: its end.
    fn xml_embedded_patterns(&mut self, s: usize) -> R<usize> {
        self.relex_at(s);
        self.sp = s as u32;
        self.tk(K::TLBrace)?;
        loop {
            if Self::is_name(self.tok()) && self.is_op_at(1, crate::lex::op::AT) {
                // `xs @ _*`: the capture's pattern a whole one (`_*` its repeat pattern).
                let n = self.open(K::CapturePattern);
                self.identifier(f::NAME)?;
                self.tk(K::TAt)?;
                self.pattern_f(f::PATTERN)?;
                self.close(n);
            } else {
                self.pattern_f(f::NONE)?;
            }
            if self.peek() == T::Comma {
                self.tk(K::TComma)?;
            } else {
                break;
            }
        }
        if self.peek() != T::RBrace {
            return self.refuse();
        }
        let t = self.tk(K::TRBrace)?;
        Ok(t.end as usize)
    }
}
