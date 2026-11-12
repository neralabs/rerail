//! The lean parser's first lexer, a byte at a time (with 8-byte searches), kept only so the
//! bench (`rr_against_rust`) can compare it with the block lexer (`lean::lex`) in one run.
//! It gives the same tokens, each with its line.
#![allow(dead_code)]


// Token kinds; punctuation is `P0` plus its place in `syntax::PUNCT` (and in the anonymous kinds).
const ID: u8 = 0;
const STR: u8 = 1;
const NUM: u8 = 2;
const FLOAT: u8 = 3;
const TICK: u8 = 4;
const HASH: u8 = 5;
const BAD: u8 = 6;
const EOF: u8 = 7;
const P0: u8 = 16;

// Punctuation, by its place in `syntax::PUNCT`.
const TURNSTILE: u8 = 0;
const FAT_ARROW: u8 = 1;
const NE: u8 = 2;
const LE: u8 = 3;
const GE: u8 = 4;
const SUBTYPE: u8 = 5;
const ARROW: u8 = 6;
const LPAREN: u8 = 7;
const RPAREN: u8 = 8;
const COMMA: u8 = 9;
const DOT: u8 = 10;
const SEMI: u8 = 11;
const COLON: u8 = 12;
const EQ: u8 = 13;
const LT: u8 = 14;
const GT: u8 = 15;
const BANG: u8 = 16;
const LBRACE: u8 = 17;
const RBRACE: u8 = 18;
const AT: u8 = 20;
const STAR: u8 = 21;
const MINUS: u8 = 24;
const PLUS: u8 = 25;
const SLASH: u8 = 26;
const PERCENT: u8 = 27;
const TILDE: u8 = 28;

// The words the grammar looks at. 1 to 18 are the reserved words, in the anonymous kinds' order
// (`Kind::Must` on); 21 to 30 the directives, in their keywords' order (`Kind::DotDecl` on).
const W_MUST: u8 = 1;
const W_MAY: u8 = 2;
const W_CODE: u8 = 3;
const W_IN: u8 = 4;
const W_TAG: u8 = 5;
const W_FIX: u8 = 6;
const W_SAFE: u8 = 7;
const W_SUPPRESS: u8 = 8;
const W_OVER: u8 = 9;
const W_OPTIONS: u8 = 10;
const W_BY: u8 = 11;
const W_SEP: u8 = 12;
const W_COUNT: u8 = 13;
const W_MIN: u8 = 14;
const W_MAX: u8 = 15;
const W_SUM: u8 = 16;
const W_MEAN: u8 = 17;
const W_JOIN: u8 = 18;
const W_PRODUCT: u8 = 19;
const W_WILD: u8 = 20;
const W_DECL: u8 = 21;
const W_EDIT: u8 = 22;
const W_FUNCTOR: u8 = 23;
const W_TYPE: u8 = 24;
const W_INPUT: u8 = 25;
const W_OUTPUT: u8 = 26;
const W_LATTICE: u8 = 27;
const W_PRAGMA: u8 = 28;
const W_VIOLATION: u8 = 29;
const W_POLICY: u8 = 30;
/// `#include`, on a `HASH` token.
const W_INCLUDE: u8 = 31;

fn word_of(w: &[u8]) -> u8 {
    if w.len() > 9 {
        return 0;
    }
    match w {
        b"must" => W_MUST,
        b"may" => W_MAY,
        b"code" => W_CODE,
        b"in" => W_IN,
        b"tag" => W_TAG,
        b"fix" => W_FIX,
        b"safe" => W_SAFE,
        b"suppress" => W_SUPPRESS,
        b"over" => W_OVER,
        b"options" => W_OPTIONS,
        b"by" => W_BY,
        b"sep" => W_SEP,
        b"count" => W_COUNT,
        b"min" => W_MIN,
        b"max" => W_MAX,
        b"sum" => W_SUM,
        b"mean" => W_MEAN,
        b"join" => W_JOIN,
        b"product" => W_PRODUCT,
        b"_" => W_WILD,
        b"decl" => W_DECL,
        b"edit" => W_EDIT,
        b"functor" => W_FUNCTOR,
        b"type" => W_TYPE,
        b"input" => W_INPUT,
        b"output" => W_OUTPUT,
        b"lattice" => W_LATTICE,
        b"pragma" => W_PRAGMA,
        b"violation" => W_VIOLATION,
        b"policy" => W_POLICY,
        _ => 0,
    }
}

fn is_aggregate_word(w: u8) -> bool {
    matches!(w, W_MIN | W_MAX | W_SUM | W_PRODUCT | W_MEAN | W_JOIN)
}

// Byte classes.
const WS: u8 = 1;
const IDENT: u8 = 2;
const DIGIT: u8 = 4;
const START: u8 = 8;

static CLASS: [u8; 256] = {
    let mut c = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        let b = i as u8;
        if matches!(b, b' ' | b'\t' | b'\n' | b'\x0c' | b'\r') {
            c[i] = WS;
        }
        if b.is_ascii_alphanumeric() || b == b'_' {
            c[i] |= IDENT;
        }
        if b.is_ascii_digit() {
            c[i] |= DIGIT;
        }
        if b.is_ascii_alphabetic() || b == b'_' {
            c[i] |= START;
        }
        i += 1;
    }
    c
};

/// One-byte punctuation, by byte: its place in `syntax::PUNCT` plus one (0: none).
static PUNCT1: [u8; 256] = {
    let mut t = [0u8; 256];
    let s = b"(),.;:=<>!{}|@*[]-+/%~^&";
    let mut i = 0;
    while i < s.len() {
        t[s[i] as usize] = 7 + i as u8 + 1;
        i += 1;
    }
    t
};

#[derive(Clone, Copy)]
struct Tok {
    start: u32,
    end: u32,
    line: u32,
    kind: u8,
    word: u8,
}

struct Lexed {
    toks: Vec<Tok>,
    /// The tokens read; `toks` has `EOF`s after them, so a look ahead needs no bounds check.
    n: usize,
    comments: Vec<(u32, u32)>,
}

// Searches 8 bytes a step, as `memchr`'s portable path does: each byte equal to the one
// sought gets its top bit set, exactly (no carry from one byte to the next).
const LO7: u64 = 0x7f7f_7f7f_7f7f_7f7f;
const HI: u64 = 0x8080_8080_8080_8080;
const ONES: u64 = 0x0101_0101_0101_0101;

fn equal(word: u64, x: u8) -> u64 {
    let v = word ^ (ONES * u64::from(x));
    !(((v & LO7) + LO7) | v) & HI
}

fn word_at(b: &[u8], i: usize) -> u64 {
    u64::from_le_bytes(b[i..i + 8].try_into().expect("8 bytes"))
}

/// The first `x` in `b`.
fn find(b: &[u8], x: u8) -> Option<usize> {
    let mut i = 0;
    while i + 8 <= b.len() {
        let m = equal(word_at(b, i), x);
        if m != 0 {
            return Some(i + (m.trailing_zeros() / 8) as usize);
        }
        i += 8;
    }
    b[i..].iter().position(|&c| c == x).map(|e| i + e)
}

/// The first `x` or `y` in `b`.
fn find2(b: &[u8], x: u8, y: u8) -> Option<usize> {
    let mut i = 0;
    while i + 8 <= b.len() {
        let w = word_at(b, i);
        let m = equal(w, x) | equal(w, y);
        if m != 0 {
            return Some(i + (m.trailing_zeros() / 8) as usize);
        }
        i += 8;
    }
    b[i..].iter().position(|&c| c == x || c == y).map(|e| i + e)
}

/// The first place `needle` (not empty) starts in `b`: its first byte found, then the rest compared.
fn find_str(b: &[u8], needle: &[u8]) -> Option<usize> {
    let mut i = 0;
    while let Some(e) = find(&b[i..], needle[0]) {
        let at = i + e;
        if b.len() - at < needle.len() {
            return None;
        }
        if &b[at..at + needle.len()] == needle {
            return Some(at);
        }
        i = at + 1;
    }
    None
}

fn newlines(b: &[u8]) -> u32 {
    let (mut n, mut i) = (0, 0);
    while i + 8 <= b.len() {
        n += equal(word_at(b, i), b'\n').count_ones();
        i += 8;
    }
    n + b[i..].iter().filter(|&&c| c == b'\n').count() as u32
}

/// `syntax::lex`'s tokens, without their text.
fn lex(src: &str) -> Lexed {
    let b = src.as_bytes();
    let n = b.len();
    let mut toks = Vec::with_capacity(n / 4 + 8);
    let mut comments = Vec::new();
    let (mut pos, mut line) = (0usize, 1u32);
    macro_rules! tok {
        ($kind:expr, $start:expr, $end:expr, $word:expr) => {
            toks.push(Tok { start: $start as u32, end: $end as u32, line, kind: $kind, word: $word })
        };
    }
    'read: loop {
        while pos < n && CLASS[b[pos] as usize] & WS != 0 {
            line += u32::from(b[pos] == b'\n');
            pos += 1;
        }
        if pos >= n {
            break;
        }
        let start = pos;
        let c = b[pos];
        let d = if pos + 1 < n { b[pos + 1] } else { 0 };
        match c {
            b'/' if d == b'/' => {
                pos = find(&b[pos..], b'\n').map_or(n, |e| pos + e);
                comments.push((start as u32, pos as u32));
            }
            // As `str::find("*/")` from the comment's first byte: `/*/` ends at once.
            b'/' if d == b'*' => match find_str(&b[pos + 1..], b"*/") {
                Some(e) => {
                    let end = pos + 1 + e + 2;
                    line += newlines(&b[pos..end]);
                    pos = end;
                    comments.push((start as u32, pos as u32));
                }
                None => {
                    tok!(BAD, start, n, 0);
                    break;
                }
            },
            b'"' => {
                // To the closing quote, past escapes; only a `\u` escape can be refused.
                let (mut end, mut u) = (pos + 1, false);
                loop {
                    match find2(&b[end..], b'"', b'\\') {
                        None => {
                            end = n;
                            break;
                        }
                        Some(e) => {
                            end += e;
                            if b[end] == b'"' {
                                break;
                            }
                            u |= end + 1 < n && b[end + 1] == b'u';
                            end += 2;
                            if end >= n {
                                break;
                            }
                        }
                    }
                }
                if end >= n {
                    tok!(BAD, start, n, 0);
                    break 'read;
                }
                let ok = !u || crate::syntax::unescape(&src[pos + 1..end]).is_ok();
                tok!(if ok { STR } else { BAD }, start, end + 1, 0);
                line += newlines(&b[start..end]);
                pos = end + 1;
            }
            b'`' => {
                let mut k = 1;
                while pos + k < n && b[pos + k] == b'`' {
                    k += 1;
                }
                let body = pos + k;
                match find_str(&b[body..], &b[pos..pos + k]) {
                    Some(close) => {
                        let end = body + close + k;
                        tok!(TICK, start, end, 0);
                        line += newlines(&b[start..end]);
                        pos = end;
                    }
                    None if k == 2 => {
                        tok!(TICK, start, pos + 2, 0);
                        pos += 2;
                    }
                    None => {
                        tok!(BAD, start, n, 0);
                        break;
                    }
                }
            }
            b'0'..=b'9' => {
                let mut end = pos + 1;
                while end < n && CLASS[b[end] as usize] & DIGIT != 0 {
                    end += 1;
                }
                // `1.5`: a `.` between digits; `0.` ends a clause.
                if end + 1 < n && b[end] == b'.' && b[end + 1].is_ascii_digit() {
                    end += 2;
                    while end < n && CLASS[b[end] as usize] & DIGIT != 0 {
                        end += 1;
                    }
                    tok!(FLOAT, start, end, 0);
                } else {
                    let ok = end - pos < 19 || src[pos..end].parse::<i64>().is_ok();
                    tok!(if ok { NUM } else { BAD }, start, end, 0);
                }
                pos = end;
            }
            b'#' => {
                let mut end = pos + 1;
                while end < n && CLASS[b[end] as usize] & IDENT != 0 {
                    end += 1;
                }
                tok!(HASH, start, end, if &b[pos + 1..end] == b"include" { W_INCLUDE } else { 0 });
                pos = end;
            }
            _ if CLASS[c as usize] & START != 0 => {
                let mut end = pos + 1;
                while end < n && CLASS[b[end] as usize] & IDENT != 0 {
                    end += 1;
                }
                tok!(ID, start, end, word_of(&b[pos..end]));
                pos = end;
            }
            _ => {
                let two = match (c, d) {
                    (b':', b'-') => TURNSTILE,
                    (b'=', b'>') => FAT_ARROW,
                    (b'!', b'=') => NE,
                    (b'<', b'=') => LE,
                    (b'>', b'=') => GE,
                    (b'<', b':') => SUBTYPE,
                    (b'-', b'>') => ARROW,
                    _ => u8::MAX,
                };
                if two != u8::MAX {
                    tok!(P0 + two, start, pos + 2, 0);
                    pos += 2;
                } else if PUNCT1[c as usize] != 0 {
                    tok!(P0 + PUNCT1[c as usize] - 1, start, pos + 1, 0);
                    pos += 1;
                } else {
                    let end = pos + src[pos..].chars().next().map_or(1, char::len_utf8);
                    tok!(BAD, start, end, 0);
                    pos = end;
                }
            }
        }
    }
    let real = toks.len();
    // The file's end: its line counts every newline, also after a token that ended the read.
    let eof_line = if pos >= n { line } else { newlines(b) + 1 };
    for _ in 0..4 {
        toks.push(Tok { start: n as u32, end: n as u32, line: eof_line, kind: EOF, word: 0 });
    }
    Lexed { toks, n: real, comments }
}


/// The tokens' count, for the bench.
pub fn token_count(src: &str) -> usize {
    lex(src).n
}
