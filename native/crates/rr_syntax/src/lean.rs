//! A lean `.rr` parser for the tape alone: the baseline the compiled grammar is measured against
//! (docs/parser-generators/30-a-lean-rust-baseline.md). It is not on the product's path.
//!
//! It reads the grammar `syntax::Parser` reads and gives the same tape (`crate::tape`), node for
//! node, written as a fast hand-written parser would be:
//!
//! - **A token is one 12-byte record**: kind, span, and the word it is when the grammar looks
//!   at that word (`code`, `min`, `decl`, …), found once by the lexer from one 8-byte load.
//!   The parser compares one-byte codes, never text. Only a few statements ask whether a
//!   token is on a line: the bytes from the line's last token to it hold no newline.
//! - **The lexer** is simdjson's shape over Bitlex's 64-byte block masks (`micro`): a block's
//!   token starts found at once and taken one at a time (`lex`). A word ends at its run of
//!   word bits; a comment, string or backtick literal by its micro-lexer, run from its start.
//! - **No statements.** Only the tape is built: the engine's tape (v9np), its bytes written by
//!   `rerail_tape_format::Writer` as the parse goes. Each node's code, offset and size
//!   and tail columns are written at their final widths in preorder as it opens, its size and
//!   tail where it closes; at the end the columns are moved into place and the header written
//!   (decision 0140). The widths are guessed from the source's length, and the file parsed
//!   again at the right ones where the guess was wrong. The open nodes and their children are
//!   a stack.
//! - **A wrap** (an operator found after its left operand: `a - b`, `x <= y`) puts its node
//!   before the operand: the nodes from there on move up one.
//! - **Comments** are placed as the parse passes them, by `tape::flatten`'s rules; the next
//!   one's start is kept, so a node costs one compare for them.
//! - **Backtracking** cuts the tape, the stack and the comments back to a mark.

use crate::micro;
use crate::tape::{ERROR, FIRST_ANONYMOUS, Field, Kind, code, kind_field};
use rerail_tape_format::{CODE_EXTRA, Layout, Narrow, Redo, Widths, Writer};

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

// What a byte starts, by byte (`START_OF`): one jump table dispatches a token.
const S_BAD: u8 = 0;
const S_PUNCT: u8 = 1;
/// Punctuation that may begin a two-byte one (`:-`, `=>`, `!=`, `<=`, `>=`, `<:`, `->`).
const S_PUNCT2: u8 = 2;
const S_WORD: u8 = 3;
const S_DIGIT: u8 = 4;
const S_QUOTE: u8 = 5;
const S_TICK: u8 = 6;
const S_SLASH: u8 = 7;
const S_HASH: u8 = 8;

static START_OF: [u8; 256] = {
    let mut t = [S_BAD; 256];
    let mut i = 0;
    while i < 256 {
        let b = i as u8;
        t[i] = if b.is_ascii_alphabetic() || b == b'_' {
            S_WORD
        } else if b.is_ascii_digit() {
            S_DIGIT
        } else {
            match b {
                b'"' => S_QUOTE,
                b'`' => S_TICK,
                b'/' => S_SLASH,
                b'#' => S_HASH,
                b':' | b'=' | b'!' | b'<' | b'>' | b'-' => S_PUNCT2,
                b'(' | b')' | b',' | b'.' | b';' | b'{' | b'}' | b'|' | b'@' | b'*' | b'[' | b']' | b'+' | b'%' | b'~' | b'^' | b'&' => S_PUNCT,
                _ => S_BAD,
            }
        };
        i += 1;
    }
    t
};

/// An ordinary token's kind by its first byte: a word, a one-byte punctuation, or a byte the
/// lexer cannot read (`BAD`).
static KIND_OF: [u8; 256] = {
    let mut t = [BAD; 256];
    let mut i = 0;
    while i < 256 {
        if START_OF[i] == S_WORD {
            t[i] = ID;
        } else if PUNCT1[i] != 0 {
            t[i] = P0 + PUNCT1[i] - 1;
        }
        i += 1;
    }
    t
};

/// A two-byte punctuation's kind by its first byte (`<:` aside: `<` takes `=` or `:`).
static PAIR_KIND: [u8; 256] = {
    let mut t = [0u8; 256];
    t[b':' as usize] = P0 + TURNSTILE;
    t[b'=' as usize] = P0 + FAT_ARROW;
    t[b'!' as usize] = P0 + NE;
    t[b'<' as usize] = P0 + LE;
    t[b'>' as usize] = P0 + GE;
    t[b'-' as usize] = P0 + ARROW;
    t
};

/// An ordinary token's kind and length, by one load: indexed by its first byte, whether it is a
/// pair's (`+ 256`), and whether that pair is `<:` (`+ 512`). A word's length is its run of
/// word bits, more than the 1 here.
static TOKEN: [(u8, u8); 1024] = {
    let mut t = [(BAD, 1u8); 1024];
    let mut c = 0;
    while c < 256 {
        t[c] = (KIND_OF[c], 1);
        if PAIR_KIND[c] != 0 {
            t[256 + c] = (PAIR_KIND[c], 2);
        }
        c += 1;
    }
    t[256 + 512 + b'<' as usize] = (P0 + SUBTYPE, 2);
    t
};

/// A run of two-byte punctuation's first bytes (`=>=`), read left to right: one whose first
/// byte is the second of the one before is none.
fn left_to_right(mut m: u64) -> u64 {
    let mut kept = 0;
    while m != 0 {
        let i = m.trailing_zeros();
        kept |= 1 << i;
        m &= !(1 << i);
        if i < 63 {
            m &= !(1 << (i + 1));
        }
    }
    kept
}

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
    kind: u8,
    word: u8,
}

struct Lexed {
    toks: Vec<Tok>,
    /// The tokens read; `toks` has `EOF`s after them, so a look ahead needs no bounds check.
    n: usize,
    comments: Vec<(u32, u32)>,
}

/// Where a fence of three or more backticks next starts, from `from`: a `pair` of backticks,
/// then the rest compared (rare: such literals are 0.02% of blocks).
fn find_fence(b: &[u8], from: usize, fence: &[u8]) -> Option<usize> {
    let mut i = from;
    loop {
        let at = micro::find_pair(b, i, b'`', b'`')?;
        if b[at..].starts_with(fence) {
            return Some(at);
        }
        i = at + 1;
    }
}

/// A word's text as one word of memory: little-endian, zeros past its end.
const fn key(s: &[u8]) -> u64 {
    let (mut v, mut i) = (0u64, 0);
    while i < s.len() {
        v |= (s[i] as u64) << (8 * i);
        i += 1;
    }
    v
}

/// The words the grammar looks at, and their codes.
const WORD_LIST: [(&[u8], u8); 30] = [
    (b"must", W_MUST),
    (b"may", W_MAY),
    (b"code", W_CODE),
    (b"in", W_IN),
    (b"tag", W_TAG),
    (b"fix", W_FIX),
    (b"safe", W_SAFE),
    (b"suppress", W_SUPPRESS),
    (b"over", W_OVER),
    (b"options", W_OPTIONS),
    (b"by", W_BY),
    (b"sep", W_SEP),
    (b"count", W_COUNT),
    (b"min", W_MIN),
    (b"max", W_MAX),
    (b"sum", W_SUM),
    (b"mean", W_MEAN),
    (b"join", W_JOIN),
    (b"product", W_PRODUCT),
    (b"_", W_WILD),
    (b"decl", W_DECL),
    (b"edit", W_EDIT),
    (b"functor", W_FUNCTOR),
    (b"type", W_TYPE),
    (b"input", W_INPUT),
    (b"output", W_OUTPUT),
    (b"lattice", W_LATTICE),
    (b"pragma", W_PRAGMA),
    (b"violation", W_VIOLATION),
    (b"policy", W_POLICY),
];

/// A word's slot in `WORDS`: its first 8 bytes and its length, hashed by a multiplier under
/// which the 30 words take 30 slots.
const fn word_slot(key: u64, len: usize) -> usize {
    ((key ^ len as u64).wrapping_mul(0x9c65_3938_2b05_37e7) >> 57) as usize
}

/// Each slot's word: its first 8 bytes, its length, its code (0: none).
static WORDS: [(u64, u8, u8); 128] = {
    let mut t = [(0u64, 0u8, 0u8); 128];
    let mut i = 0;
    while i < WORD_LIST.len() {
        let (w, code) = WORD_LIST[i];
        let k = key(if w.len() > 8 { w.split_at(8).0 } else { w });
        let slot = word_slot(k, w.len());
        assert!(t[slot].2 == 0, "two words in a slot");
        t[slot] = (k, w.len() as u8, code);
        i += 1;
    }
    t
};

/// The code (`W_…`) of a word of `len` bytes, from its first 9 (`raw`: the first 8, zeros
/// past the text), without a branch: masked to its length, looked up in `WORDS`. A word of 9
/// bytes (only `violation`) checks its last byte too.
#[inline(always)]
fn word_code(raw: u64, ninth: u8, len: usize) -> u8 {
    let w = raw & (u64::MAX >> (64 - 8 * len.clamp(1, 8)));
    let (k, l, code) = WORDS[word_slot(w, len)];
    let hit = (k == w) & (usize::from(l) == len) & ((len != 9) | (ninth == b'n'));
    code & 0u8.wrapping_sub(u8::from(hit))
}

/// `word_code` of the word `b[s..s + len]`, read from the text (a word across a block's end).
fn word_of(b: &[u8], s: usize, len: usize) -> u8 {
    let mut pad = [0u8; 9];
    let have = (b.len() - s).min(9);
    pad[..have].copy_from_slice(&b[s..s + have]);
    word_code(u64::from_le_bytes(pad[..8].try_into().expect("8 bytes")), pad[8], len)
}

/// Where the word bytes from `from` end: the run of the block's `word` bits, and on past the
/// block a byte at a time (a word across a block's end: rare).
#[inline(always)]
fn word_end(b: &[u8], w: &micro::Window, from: usize) -> usize {
    let q = from - w.base;
    let mut end = from;
    if q < 64 {
        let r = micro::run(w.word, q as u32) as usize;
        end += r;
        if q + r < 64 {
            return end;
        }
    }
    while end < b.len() && CLASS[b[end] as usize] & IDENT != 0 {
        end += 1;
    }
    end
}

/// `syntax::lex`'s tokens, without their text, in simdjson's shape: a block's token starts
/// found at once, then written without a branch a token.
///
/// A block is the 64 bytes from `base` (`micro::Window`). Its starts are every byte not
/// whitespace, but a word's bytes after its first (`!space & !(word & word << 1)`), and but a
/// two-byte punctuation's second byte (`pair`). The rare starts (`rare`: a string, a comment,
/// a backtick literal, `#`, a number, a non-ASCII character) cut the block into runs. A run's
/// tokens are written straight: the kind by the first byte (a pair's by its own table), the
/// end by the run of `word` bits, the word's code by one load and a table. A rare token is
/// read by its micro-lexer, and the starts it covers are cleared; one that ends past the block
/// starts the next block where it ends.
fn lex(src: &str) -> Lexed {
    let b = src.as_bytes();
    let n = b.len();
    // Written behind, as simdjson writes its index: room for a token a byte (each takes one at
    // least) and the `EOF`s, so a write needs no check.
    let mut toks: Vec<Tok> = Vec::with_capacity(n + 8);
    let (out, mut len) = (toks.as_mut_ptr(), 0usize);
    let mut comments = Vec::new();
    macro_rules! tok {
        ($kind:expr, $start:expr, $end:expr, $word:expr) => {{
            // SAFETY: fewer than `n + 8` tokens are written (see above).
            unsafe { out.add(len).write(Tok { start: $start as u32, end: $end as u32, kind: $kind, word: $word }) };
            len += 1;
        }};
    }
    // No token starts before `cursor`.
    let (mut base, mut cursor) = (0usize, 0usize);
    'read: while base < n {
        let w = micro::Window::at(b, base);
        let valid = if n - base >= 64 { !0 } else { (1u64 << (n - base)) - 1 };
        let mut starts = !w.space & !(w.word & (w.word << 1)) & valid & micro::bits(cursor.saturating_sub(base) as u32, 64);
        let mut pairs = w.pair & starts;
        if pairs & (pairs << 1) != 0 {
            pairs = left_to_right(pairs);
        }
        starts &= !(pairs << 1);
        loop {
            let rare = starts & w.rare;
            // The run: the starts before the next rare one.
            let mut run = starts & (rare.wrapping_sub(1) & !rare);
            while run != 0 {
                let q = run.trailing_zeros();
                run &= run - 1;
                let (pos, at) = (base + q as usize, (q & 63) as usize);
                // The token's first bytes, from the window: in bounds, no check.
                let (c, d) = (w.bytes[at], w.bytes[at + 1]);
                let raw = u64::from_le_bytes(w.bytes[at..at + 8].try_into().expect("8 bytes"));
                let ninth = w.bytes[at + 8];
                let words = micro::run(w.word, q) as usize;
                let pair = ((pairs >> q) & 1) as usize;
                // The kind and length by one load; a word's length is its run.
                let (kind, n1) = TOKEN[c as usize | pair << 8 | (pair & usize::from(d == b':')) << 9];
                let mut end = pos + words.max(usize::from(n1));
                // Only a word has a code.
                let mut code = if words != 0 { word_code(raw, ninth, words) } else { 0 };
                if at + words == 64 {
                    // A word on past the block: the last start of it.
                    end = word_end(b, &w, pos);
                    cursor = end;
                    code = word_of(b, pos, end - pos);
                }
                tok!(kind, pos, end, code);
            }
            if rare == 0 {
                break;
            }
            let pos = base + rare.trailing_zeros() as usize;
            let start = pos;
            let c = b[pos];
            let d = if pos + 1 < n { b[pos + 1] } else { 0 };
            cursor = match c {
                b'/' if d == b'/' => {
                    let end = micro::find(b, pos, b'\n').unwrap_or(n);
                    comments.push((start as u32, end as u32));
                    end
                }
                // As `str::find("*/")` from the comment's first byte: `/*/` ends at once.
                b'/' if d == b'*' => match micro::find_pair(b, pos + 1, b'*', b'/') {
                    Some(at) => {
                        comments.push((start as u32, at as u32 + 2));
                        at + 2
                    }
                    None => {
                        tok!(BAD, start, n, 0);
                        break 'read;
                    }
                },
                b'/' => {
                    tok!(P0 + SLASH, start, pos + 1, 0);
                    pos + 1
                }
                b'"' => {
                    // To the closing quote, past escapes; only a `\u` escape can be refused.
                    let (end, u) = micro::string_end(b, pos + 1);
                    let Some(end) = end else {
                        tok!(BAD, start, n, 0);
                        break 'read;
                    };
                    let ok = !u || crate::syntax::unescape(&src[pos + 1..end]).is_ok();
                    tok!(if ok { STR } else { BAD }, start, end + 1, 0);
                    end + 1
                }
                b'`' => {
                    let mut k = 1;
                    while pos + k < n && b[pos + k] == b'`' {
                        k += 1;
                    }
                    let body = pos + k;
                    // The closing fence: a backtick, a pair of them, or a longer fence.
                    let close = match k {
                        1 => micro::find(b, body, b'`'),
                        2 => micro::find_pair(b, body, b'`', b'`'),
                        _ => find_fence(b, body, &b[pos..pos + k]),
                    };
                    match close {
                        Some(at) => {
                            tok!(TICK, start, at + k, 0);
                            at + k
                        }
                        None if k == 2 => {
                            tok!(TICK, start, pos + 2, 0);
                            pos + 2
                        }
                        None => {
                            tok!(BAD, start, n, 0);
                            break 'read;
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
                    // A word right after the number (`12abc`) is a word of its own.
                    if end < n && CLASS[b[end] as usize] & IDENT != 0 {
                        let from = end;
                        while end < n && CLASS[b[end] as usize] & IDENT != 0 {
                            end += 1;
                        }
                        tok!(ID, from, end, word_of(b, from, end - from));
                    }
                    end
                }
                b'#' => {
                    let end = word_end(b, &w, pos + 1);
                    tok!(HASH, start, end, if &b[pos + 1..end] == b"include" { W_INCLUDE } else { 0 });
                    end
                }
                // A non-ASCII character: what the lexer cannot read.
                _ => {
                    let end = pos + src[pos..].chars().next().map_or(1, char::len_utf8);
                    tok!(BAD, start, end, 0);
                    end
                }
            };
            if cursor >= base + 64 {
                break;
            }
            starts &= micro::bits((cursor - base) as u32, 64);
        }
        if pairs >> 63 != 0 {
            // A pair's second byte is the next block's first.
            cursor = cursor.max(base + 65);
        }
        base = cursor.max(base + 64);
    }
    let real = len;
    for _ in 0..4 {
        tok!(EOF, n, n, 0);
    }
    // SAFETY: the first `len` tokens are written.
    unsafe { toks.set_len(len) };
    Lexed { toks, n: real, comments }
}

/// What a literal read was (`syntax::Lit`'s variant), for `!`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Lit {
    Atom,
    Cmp,
    Other,
}

/// Whether a term read is a call that is not a functor's (`syntax::Term::Call { functor: false }`).
type Call = bool;

type R<T> = Result<T, ()>;

/// A place to go back to: the token, the tape, the stack and the comments placed.
#[derive(Clone, Copy)]
struct Mark {
    i: usize,
    out: usize,
    kids: usize,
    open: usize,
    comment: usize,
}

struct Parser<'s, L: Layout> {
    b: &'s [u8],
    t: &'s [Tok],
    n: usize,
    i: usize,
    no_named: bool,
    /// The tape so far, in preorder.
    out: Writer<L>,
    /// The open nodes: each its place in `out` and where its children start in `kids`.
    open: Vec<(u32, u32)>,
    /// The open nodes' children, by place in `out`.
    kids: Vec<u32>,
    comments: &'s [(u32, u32)],
    /// The next comment to place, and where it starts (`u32::MAX`: none left).
    comment: usize,
    next_comment: u32,
}

/// The lexer alone (its tokens' count), to measure apart from the parse.
pub fn token_count(src: &str) -> usize {
    lex(src).n
}

/// `src`'s tape: the engine's (`code_rule_dsl::tape`'s image of `crate::tape(src)`), its bytes.
pub fn tape(src: &str) -> Vec<u8> {
    let lexed = lex(src);
    let mut widths = Widths::guess(src.len());
    loop {
        // Nearly every file is written at the narrow widths, fixed in the parser's code.
        let at = if widths == Narrow::WIDTHS { tape_at(src, &lexed, Narrow) } else { tape_at(src, &lexed, widths) };
        match at {
            Ok(bytes) => return bytes,
            Err(Redo(w)) => widths = w,
        }
    }
}

/// `tape` at `layout`'s widths, the lexer's tokens given.
fn tape_at<L: Layout>(src: &str, lexed: &Lexed, layout: L) -> Result<Vec<u8>, Redo> {
    let Lexed { toks, n, comments } = lexed;
    let mut out = Writer::new(rerail_tape_format::RR_TAG, layout, n + n / 2 + comments.len() + 8);
    out.push(code(Kind::SourceFile as u16, 0, false), 0);
    let next_comment = comments.first().map_or(u32::MAX, |c| c.0);
    let mut p = Parser { b: src.as_bytes(), t: toks, n: *n, i: 0, no_named: false, out, open: vec![(0, 0)], kids: Vec::with_capacity(64), comments, comment: 0, next_comment };
    while p.kind() != EOF {
        let save = p.mark();
        if p.statement().is_err() {
            p.recover(save);
        }
        // The statement's nodes are final: their codes are written now.
        p.out.commit();
    }
    // Comments after the last statement are the file's.
    p.comments_before(0, u32::MAX);
    p.out.close(0, src.len() as u32);
    p.out.finish(src.as_bytes())
}

impl<L: Layout> Parser<'_, L> {
    // Tokens.

    fn kind_at(&self, k: usize) -> u8 {
        self.t.get(self.i + k).map_or(EOF, |t| t.kind)
    }

    fn kind(&self) -> u8 {
        self.kind_at(0)
    }

    fn word_at(&self, k: usize) -> u8 {
        self.t.get(self.i + k).map_or(0, |t| if t.kind == ID { t.word } else { 0 })
    }

    fn is_p(&self, p: u8) -> bool {
        self.kind() == P0 + p
    }

    fn is_p_at(&self, k: usize, p: u8) -> bool {
        self.kind_at(k) == P0 + p
    }

    fn is_id(&self, w: u8) -> bool {
        self.word_at(0) == w
    }

    fn is_id_at(&self, k: usize) -> bool {
        self.kind_at(k) == ID
    }

    fn start(&self) -> u32 {
        self.t.get(self.i).map_or(self.b.len() as u32, |t| t.start)
    }

    /// Whether the next token is on the line of byte `*seen`, the bytes up to `*seen` known
    /// to be on it: no newline from there to the token's start. `*seen` moves to the start, so
    /// a line's bytes are read once. As `syntax::Lines` compares lines, with no index of them.
    fn on_line(&self, seen: &mut u32) -> bool {
        let start = self.start();
        let same = !self.b[(*seen as usize).min(start as usize)..start as usize].contains(&b'\n');
        *seen = start;
        same
    }

    /// Where the token before the next one ends: the bytes to it are on its line (`on_line`).
    fn last_end(&self) -> Option<u32> {
        self.i.checked_sub(1).and_then(|j| self.t.get(j)).map(|t| t.end)
    }

    // The tape.

    #[cold]
    #[inline(never)]
    fn comments_before(&mut self, parent: u32, before: u32) {
        while self.next_comment < before {
            let (start, end) = self.comments[self.comment];
            let _ = parent;
            let id = self.out.push(code(Kind::Comment as u16, 0, true), start);
            self.out.close(id, end);
            self.comment += 1;
            self.next_comment = self.comments.get(self.comment).map_or(u32::MAX, |c| c.0);
        }
    }

    #[cold]
    #[inline(never)]
    fn skip_comments(&mut self, before: u32) {
        while self.next_comment < before {
            self.comment += 1;
            self.next_comment = self.comments.get(self.comment).map_or(u32::MAX, |c| c.0);
        }
    }

    fn top(&self) -> (u32, u32) {
        *self.open.last().expect("source_file stays open")
    }

    /// A child of the open node; it stays open when `open`. Inline, as every node is made by
    /// it; a comment near it (rare) goes the long way.
    #[inline(always)]
    fn push(&mut self, kind: u16, start: u32, end: u32, open: bool) {
        if self.next_comment < end.max(start) {
            return self.push_near_comment(kind, start, end, open);
        }
        let id = self.out.push(code(kind, 0, false), start) as u32;
        self.kids.push(id);
        if open {
            self.open.push((id, self.kids.len() as u32));
        } else {
            self.out.close(id as usize, end);
        }
    }

    /// `push`, with a comment before the node's end: placed before it, or inside its text.
    #[cold]
    #[inline(never)]
    fn push_near_comment(&mut self, kind: u16, start: u32, end: u32, open: bool) {
        let parent = self.top().0;
        if self.next_comment < start {
            self.comments_before(parent, start);
        }
        let id = self.out.push(code(kind, 0, false), start) as u32;
        self.kids.push(id);
        if open {
            self.open.push((id, self.kids.len() as u32));
        } else {
            self.out.close(id as usize, end);
            if self.next_comment < end {
                // A leaf's text is its own: a comment inside it is not a node.
                self.skip_comments(end);
            }
        }
    }

    #[inline(always)]
    fn open(&mut self, kind: Kind) {
        let at = self.start();
        self.push(kind as u16, at, at, true);
    }

    #[inline(always)]
    fn close(&mut self) {
        let end = match self.i.checked_sub(1) {
            Some(j) => self.t.get(j).map_or(0, |t| t.end),
            None => 0,
        };
        let (id, base) = self.open.pop().expect("an open node");
        let end = end.max(self.out.start(id as usize));
        // A node opened for children holds the comments inside it, though it has none (`()`,
        // its tokens implied).
        if self.next_comment < end {
            self.comments_before(id, end);
        }
        // Its subtree: the nodes from it on.
        self.out.close(id as usize, end);
        self.kids.truncate(base as usize);
    }

    fn here(&self) -> usize {
        self.kids.len() - self.top().1 as usize
    }

    fn last(&self) -> Option<u32> {
        if self.kids.len() > self.top().1 as usize { self.kids.last().copied() } else { None }
    }

    #[inline(always)]
    fn field(&mut self, field: Field) {
        if let Some(last) = self.last() {
            let (kind, _) = kind_field(self.out.key(last as usize));
            self.out.set_key(last as usize, code(kind, field as u16, false));
        }
    }

    /// A node of `kind` around the open node's children from `from` on, open, before the first.
    fn wrap(&mut self, from: usize, kind: Kind) {
        let (x, base) = self.top();
        let at = base as usize + from;
        let Some(&k) = self.kids.get(at) else {
            // Nothing to wrap: an empty node at the end.
            let _ = x;
            let id = self.out.push(code(kind as u16, 0, false), 0) as u32;
            self.kids.push(id);
            self.open.push((id, self.kids.len() as u32));
            return;
        };
        // The operand's field is the new node's; the new node goes before it.
        let _ = x;
        let (first_kind, field) = kind_field(self.out.key(k as usize));
        let start = self.out.start(k as usize);
        self.out.set_key(k as usize, code(first_kind, 0, self.out.key(k as usize) >> 16 & u32::from(CODE_EXTRA) != 0));
        self.out.insert(k as usize, code(kind as u16, field, false), start);
        for c in &mut self.kids[at..] {
            *c += 1;
        }
        self.kids.insert(at, k);
        self.open.push((k, at as u32 + 1));
    }

    fn mark(&self) -> Mark {
        Mark { i: self.i, out: self.out.len(), kids: self.kids.len(), open: self.open.len(), comment: self.comment }
    }

    fn reset(&mut self, m: Mark) {
        self.i = m.i;
        self.open.truncate(m.open);
        self.kids.truncate(m.kids);
        self.out.truncate(m.out);
        self.comment = m.comment;
        self.next_comment = self.comments.get(self.comment).map_or(u32::MAX, |c| c.0);
    }

    #[inline(always)]
    fn leaf_over(&mut self, kind: u16, from: usize, to: usize) {
        if from < to && to <= self.n {
            self.push(kind, self.t[from].start, self.t[to - 1].end, false);
        }
    }

    fn bump_as(&mut self, kind: Kind) {
        self.leaf_over(kind as u16, self.i, self.i + 1);
        self.i += 1;
    }

    #[inline(always)]
    fn anon(&mut self) {
        let (k, w) = self.t.get(self.i).map_or((EOF, 0), |t| (t.kind, t.word));
        let kind = match k {
            ID if (W_MUST..=W_JOIN).contains(&w) => Kind::Must as u16 + u16::from(w - W_MUST),
            HASH if w == W_INCLUDE => Kind::HashInclude as u16,
            k if k >= P0 => FIRST_ANONYMOUS + u16::from(k - P0),
            _ => Kind::Identifier as u16,
        };
        self.leaf_over(kind, self.i, self.i + 1);
        self.i += 1;
    }

    /// A token the grammar implies: the tape leaves it out (`syntax::Parser::implied`).
    fn implied(&mut self) {
        self.i += 1;
    }

    fn anon_over(&mut self, p: u8, at: usize) {
        self.leaf_over(FIRST_ANONYMOUS + u16::from(p), at, at + 1);
    }

    fn word(&mut self, field: Field) -> R<()> {
        let kind = match self.kind() {
            ID => Kind::Identifier,
            NUM => Kind::Number,
            FLOAT => Kind::Float,
            STR => Kind::String,
            TICK => Kind::CodeLiteral,
            BAD => return Err(()),
            _ => {
                self.anon();
                return Ok(());
            }
        };
        self.bump_as(kind);
        self.field(field);
        Ok(())
    }

    /// The punctuation `p`, implied.
    fn p(&mut self, p: u8) -> R<()> {
        if self.is_p(p) {
            self.implied();
            Ok(())
        } else {
            Err(())
        }
    }

    /// The punctuation `p`, stored.
    fn p_stored(&mut self, p: u8) -> R<()> {
        if self.is_p(p) {
            self.anon();
            Ok(())
        } else {
            Err(())
        }
    }

    fn id(&mut self) -> R<()> {
        if self.kind() == ID {
            self.bump_as(Kind::Identifier);
            Ok(())
        } else {
            Err(())
        }
    }

    fn string(&mut self) -> R<()> {
        if self.kind() == STR {
            self.bump_as(Kind::String);
            Ok(())
        } else {
            Err(())
        }
    }

    fn list(&mut self, close: u8, field: Field, mut item: impl FnMut(&mut Self) -> R<()>) -> R<()> {
        if self.is_p(close) {
            return Ok(());
        }
        item(self)?;
        self.field(field);
        while self.is_p(COMMA) {
            self.implied();
            item(self)?;
            self.field(field);
        }
        Ok(())
    }

    fn braces(&mut self) -> R<()> {
        if !self.is_p(LBRACE) {
            return Err(());
        }
        self.i += 1;
        let mut depth = 1;
        while depth > 0 {
            match self.kind() {
                k if k == P0 + LBRACE => depth += 1,
                k if k == P0 + RBRACE => depth -= 1,
                EOF | BAD => return Err(()),
                _ => {}
            }
            self.i += 1;
        }
        Ok(())
    }

    fn json(&mut self, field: Field) -> R<()> {
        let from = self.i;
        self.braces()?;
        self.leaf_over(Kind::JsonObject as u16, from, self.i);
        self.field(field);
        Ok(())
    }

    fn brace_column(&mut self, field: Field) -> R<()> {
        let from = self.i;
        self.braces()?;
        if self.i - from == 3 && self.t[from + 1].kind == ID {
            self.leaf_over(Kind::Identifier as u16, from + 1, from + 2);
            self.field(field);
        } else {
            self.leaf_over(Kind::RawText as u16, from, self.i);
            self.field(field);
        }
        Ok(())
    }

    fn ty(&mut self) -> R<()> {
        self.open(Kind::Type);
        self.id()?;
        self.field(Field::Name);
        if self.is_p(LT) {
            if self.is_p_at(1, GT) {
                self.anon();
                self.anon();
            } else {
                self.implied();
                self.id()?;
                self.field(Field::Argument);
                self.p(GT)?;
            }
        }
        self.close();
        Ok(())
    }

    fn column(&mut self) -> R<()> {
        self.open(Kind::Column);
        self.id()?;
        self.field(Field::Name);
        self.p(COLON)?;
        self.ty()?;
        self.field(Field::Type);
        self.close();
        Ok(())
    }

    fn columns(&mut self) -> R<()> {
        self.open(Kind::ColumnList);
        self.p(LPAREN)?;
        self.list(RPAREN, Field::Column, Self::column)?;
        self.p(RPAREN)?;
        self.close();
        self.field(Field::Columns);
        Ok(())
    }

    fn arguments(&mut self) -> R<()> {
        self.open(Kind::ArgumentList);
        self.p(LPAREN)?;
        self.list(RPAREN, Field::Argument, |p| p.term().map(|_| ()))?;
        self.p(RPAREN)?;
        self.close();
        self.field(Field::Arguments);
        Ok(())
    }

    fn rest_of_line(&mut self, from: Option<u32>, field: Field) -> R<()> {
        let Some(mut seen) = from else { return Ok(()) };
        while self.kind() != EOF && self.on_line(&mut seen) {
            self.word(field)?;
        }
        Ok(())
    }

    fn raw_line(&mut self, at: Option<u32>, field: Field) -> R<()> {
        let from = self.i;
        let mut seen = at.unwrap_or(0);
        while self.kind() != EOF && at.is_some() && self.on_line(&mut seen) {
            if self.kind() == BAD {
                return Err(());
            }
            self.i += 1;
        }
        self.leaf_over(Kind::RawText as u16, from, self.i);
        self.field(field);
        Ok(())
    }

    /// `syntax::Parser::recover`: one ERROR leaf from the statement's first token to where
    /// parsing resumes.
    fn recover(&mut self, save: Mark) {
        let first = save.i;
        self.reset(save);
        let b = self.b;
        let line_start = |at: usize| b[..at].iter().rev().take_while(|&&c| c != b'\n').all(|c| c.is_ascii_whitespace());
        let n = self.n;
        let mut k = first + 1;
        while k < n {
            match self.t[k].kind {
                HASH => break,
                x if x == P0 + DOT && self.t[k + 1].kind == ID && line_start(self.t[k].start as usize) => break,
                x if x == P0 + DOT => {
                    k += 1;
                    break;
                }
                _ => k += 1,
            }
        }
        let k = k.min(n);
        self.leaf_over(ERROR, first, k);
        self.i = k;
    }

    fn statement(&mut self) -> R<()> {
        let first = self.i;
        if self.kind() == HASH {
            let include = self.t[self.i].word == W_INCLUDE;
            self.open(Kind::Include);
            self.implied();
            if !include {
                return Err(());
            }
            self.string()?;
            self.field(Field::Path);
            self.close();
            Ok(())
        } else if self.is_p(DOT) && self.is_id_at(1) {
            let d = self.t[self.i + 1].word;
            let policy = d == W_POLICY && self.is_p_at(2, LBRACE);
            let kind = match d {
                W_DECL => Kind::Declaration,
                W_EDIT => Kind::EditDeclaration,
                W_FUNCTOR => Kind::FunctorDeclaration,
                W_TYPE => Kind::TypeDeclaration,
                W_INPUT | W_OUTPUT => Kind::IoDirective,
                W_LATTICE => Kind::LatticeDeclaration,
                W_PRAGMA => Kind::Pragma,
                W_VIOLATION => Kind::ViolationDeclaration,
                W_POLICY if policy => Kind::PolicyDocument,
                _ => Kind::Setting,
            };
            // The keyword's kind: the directives' in their words' order.
            let keyword = Kind::DotDecl as u16 + u16::from(d.wrapping_sub(W_DECL));
            self.open(kind);
            // A `.` is stored under a kind whose tail may hold any punctuation (a setting's, a
            // declaration's, a functor's); a directive's word is implied, but `.input`/`.output`.
            if kind == Kind::Setting {
                self.anon();
                self.bump_as(Kind::Identifier);
                self.field(Field::Name);
            } else if kind == Kind::IoDirective {
                self.leaf_over(keyword, self.i, self.i + 2);
                self.i += 2;
            } else if matches!(kind, Kind::Declaration | Kind::FunctorDeclaration) {
                self.anon();
                self.implied();
            } else {
                self.i += 2;
            }
            self.directive(first, d, kind)?;
            self.close();
            Ok(())
        } else {
            self.clause()
        }
    }

    fn io_relation(&mut self) -> R<()> {
        self.open(Kind::IoRelation);
        self.id()?;
        self.field(Field::Name);
        if self.is_p(LPAREN) {
            let open = self.i;
            self.i += 1;
            let from = self.i;
            let mut depth = 1;
            while depth > 0 {
                match self.kind() {
                    k if k == P0 + LPAREN => depth += 1,
                    k if k == P0 + RPAREN => depth -= 1,
                    EOF | BAD => return Err(()),
                    _ => {}
                }
                self.i += 1;
            }
            let close = self.i - 1;
            if from == close {
                self.anon_over(LPAREN, open);
                self.field(Field::Parameters);
                self.anon_over(RPAREN, close);
            } else {
                self.leaf_over(Kind::RawText as u16, from, close);
                self.field(Field::Parameters);
            }
        }
        self.close();
        Ok(())
    }

    fn lattice_operation(&mut self) -> R<()> {
        self.open(Kind::LatticeOperation);
        self.id()?;
        self.field(Field::Name);
        self.p(ARROW)?;
        if self.is_p(AT) && self.is_id_at(1) && !self.is_p_at(2, LPAREN) {
            self.open(Kind::FunctorCall);
            self.implied();
            self.id()?;
            self.field(Field::Function);
            self.close();
            self.field(Field::Value);
            self.close();
            return Ok(());
        }
        self.term()?;
        self.field(Field::Value);
        self.close();
        Ok(())
    }

    /// A directive's statement, after its `.word`; `first` its first token.
    fn directive(&mut self, first: usize, d: u8, kind: Kind) -> R<()> {
        match d {
            W_DECL => {
                self.id()?;
                self.field(Field::Name);
                self.columns()?;
                let at = self.last_end();
                self.rest_of_line(at, Field::Modifier)?;
            }
            W_EDIT => {
                self.id()?;
                self.field(Field::Name);
                self.columns()?;
            }
            W_FUNCTOR => {
                self.id()?;
                self.field(Field::Name);
                self.columns()?;
                self.p_stored(COLON)?;
                self.ty()?;
                self.field(Field::ReturnType);
                let at = self.last_end();
                self.rest_of_line(at, Field::Modifier)?;
            }
            W_TYPE => {
                self.id()?;
                self.field(Field::Name);
                let at = self.last_end();
                self.raw_line(at, Field::Definition)?;
            }
            W_INPUT | W_OUTPUT => self.list(DOT, Field::Relation, Self::io_relation)?,
            W_LATTICE => {
                self.id()?;
                self.field(Field::Name);
                self.p(LT)?;
                if !self.is_p(GT) {
                    self.id()?;
                    self.field(Field::Base);
                }
                self.p(GT)?;
                self.p(LBRACE)?;
                self.list(RBRACE, Field::Operation, Self::lattice_operation)?;
                self.p(RBRACE)?;
            }
            W_PRAGMA => {
                while self.kind() == STR {
                    self.string()?;
                    self.field(Field::Argument);
                }
            }
            W_VIOLATION => self.violation()?,
            W_POLICY if kind == Kind::PolicyDocument => {
                let from = self.i;
                self.json(Field::Document)?;
                // The document must be JSON.
                let text = &self.b[self.t[from].start as usize..self.t[self.i - 1].end as usize];
                serde_json::from_slice::<serde_json::Value>(text).map_err(|_| ())?;
            }
            _ => {
                let mut seen = self.t[first].start;
                while self.kind() != EOF && (self.on_line(&mut seen) || (self.i > 0 && self.t[self.i - 1].kind == P0 + COMMA)) {
                    if self.is_p(LBRACE) {
                        self.json(Field::Argument)?;
                    } else {
                        self.word(Field::Argument)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn violation(&mut self) -> R<()> {
        self.id()?;
        self.field(Field::Name);
        self.columns()?;
        loop {
            if self.is_id(W_CODE) && self.kind_at(1) == STR {
                self.implied();
                self.string()?;
                self.field(Field::Code);
            } else if (self.is_id(W_CODE) || self.is_id(W_TAG)) && self.is_p_at(1, LBRACE) {
                let is_code = self.is_id(W_CODE);
                self.open(if is_code { Kind::CodeColumn } else { Kind::TagColumn });
                self.implied();
                self.brace_column(Field::Column)?;
                if is_code && self.is_id(W_IN) && self.kind_at(1) == STR {
                    self.implied();
                    self.string()?;
                    self.field(Field::Codes);
                }
                self.close();
                self.field(if is_code { Field::Code } else { Field::Tag });
            } else {
                break;
            }
        }
        self.string()?;
        self.field(Field::Message);
        loop {
            if self.is_id(W_FIX) && self.is_id_at(1) {
                self.open(Kind::FixLine);
                self.anon();
                if self.is_id(W_SAFE) && self.is_id_at(1) {
                    self.anon();
                }
                self.id()?;
                self.field(Field::Relation);
                self.string()?;
                self.field(Field::Description);
                self.close();
                self.field(Field::Fix);
            } else if self.is_id(W_SUPPRESS) && self.is_id_at(1) && self.kind_at(2) == STR {
                self.open(Kind::SuppressLine);
                self.implied();
                self.id()?;
                self.field(Field::Tool);
                self.string()?;
                self.field(Field::Rule);
                while self.is_p(COMMA) && self.kind_at(1) == STR {
                    self.implied();
                    self.string()?;
                    self.field(Field::Rule);
                }
                if self.is_id(W_OVER) && self.is_p_at(1, LBRACE) {
                    self.implied();
                    self.brace_column(Field::Over)?;
                }
                self.close();
                self.field(Field::Suppress);
            } else if self.is_id(W_OPTIONS) && self.is_p_at(1, LBRACE) {
                self.implied();
                self.json(Field::Options)?;
            } else {
                break;
            }
        }
        Ok(())
    }

    fn head(&mut self) -> R<()> {
        self.open(Kind::Head);
        if (self.is_id(W_MUST) || self.is_id(W_MAY)) && self.is_id_at(1) {
            self.anon();
            self.field(Field::Modal);
        }
        self.id()?;
        self.field(Field::Relation);
        self.arguments()?;
        self.close();
        Ok(())
    }

    fn clause(&mut self) -> R<()> {
        self.open(Kind::Clause);
        self.head()?;
        self.field(Field::Head);
        while self.is_p(COMMA) {
            self.implied();
            self.head()?;
            self.field(Field::Head);
        }
        if self.is_p(TURNSTILE) {
            self.implied();
            self.disjunction()?;
            self.field(Field::Body);
        }
        if self.is_p(FAT_ARROW) {
            self.implied();
            self.list(DOT, Field::Edit, |p| p.term().map(|_| ()))?;
        }
        self.p(DOT)?;
        self.close();
        Ok(())
    }

    fn disjunction(&mut self) -> R<()> {
        let from = self.here();
        self.conjunction()?;
        if self.is_p(SEMI) {
            self.wrap(from, Kind::Disjunction);
            self.field(Field::Alternative);
            while self.is_p(SEMI) {
                self.implied();
                self.conjunction()?;
                self.field(Field::Alternative);
            }
            self.close();
        }
        Ok(())
    }

    fn conjunction(&mut self) -> R<()> {
        self.open(Kind::Conjunction);
        self.literal()?;
        self.field(Field::Literal);
        while self.is_p(COMMA) {
            self.implied();
            self.literal()?;
            self.field(Field::Literal);
        }
        self.close();
        Ok(())
    }

    fn literal(&mut self) -> R<Lit> {
        if self.is_p(BANG) {
            self.open(Kind::Negation);
            self.implied();
            let inner = self.literal()?;
            self.field(Field::Operand);
            self.close();
            return if inner == Lit::Cmp { Err(()) } else { Ok(inner) };
        }
        if self.is_p(LPAREN) {
            let save = self.mark();
            self.open(Kind::Group);
            self.implied();
            let group = self.disjunction().and_then(|()| {
                self.field(Field::Body);
                self.p(RPAREN)
            });
            let k = self.kind();
            let term_follows = k >= P0 && matches!(k - P0, EQ | NE | LT | LE | GT | GE | PLUS | MINUS | STAR | SLASH | PERCENT);
            if group.is_ok() && !term_follows {
                self.close();
                return Ok(Lit::Other);
            }
            self.reset(save);
        }
        let from = self.here();
        let call = self.term()?;
        if self.is_p(TILDE) {
            self.wrap(from, Kind::PatternMatch);
            self.field(Field::Subject);
            self.implied();
            if self.kind() != TICK {
                return Err(());
            }
            self.bump_as(Kind::CodeLiteral);
            self.field(Field::Pattern);
            self.close();
            return Ok(Lit::Other);
        }
        let k = self.kind();
        if !(k >= P0 && matches!(k - P0, EQ | NE | LT | LE | GT | GE)) {
            if !call {
                return Err(());
            }
            self.retag_atom();
            return Ok(Lit::Atom);
        }
        self.wrap(from, Kind::Comparison);
        self.field(Field::Left);
        self.anon();
        self.field(Field::Operator);
        self.term()?;
        self.field(Field::Right);
        self.close();
        Ok(Lit::Cmp)
    }

    /// The call just read is an atom: its function is its relation.
    fn retag_atom(&mut self) {
        let Some(call) = self.last() else { return };
        let (_, field) = kind_field(self.out.key(call as usize));
        self.out.set_key(call as usize, code(Kind::Atom as u16, field, false));
        // Its function is its first child, after any comments: the first node after it that is
        // not one.
        for n in call as usize + 1..self.out.len() {
            let k = self.out.key(n);
            if k >> 16 & u32::from(CODE_EXTRA) == 0 {
                if kind_field(k).1 == Field::Function as u16 {
                    self.out.set_key(n, code(kind_field(k).0, Field::Relation as u16, false));
                }
                break;
            }
        }
    }

    fn term(&mut self) -> R<Call> {
        let from = self.here();
        let mut call = self.product()?;
        while self.is_p(PLUS) || self.is_p(MINUS) {
            self.wrap(from, Kind::BinaryExpression);
            self.field(Field::Left);
            self.anon();
            self.field(Field::Operator);
            self.product()?;
            self.field(Field::Right);
            self.close();
            call = false;
        }
        Ok(call)
    }

    fn product(&mut self) -> R<Call> {
        let from = self.here();
        let mut call = self.primary()?;
        while self.is_p(STAR) || self.is_p(SLASH) || self.is_p(PERCENT) {
            self.wrap(from, Kind::BinaryExpression);
            self.field(Field::Left);
            self.anon();
            self.field(Field::Operator);
            self.primary()?;
            self.field(Field::Right);
            self.close();
            call = false;
        }
        Ok(call)
    }

    fn aggregate(&mut self) -> R<Call> {
        self.open(Kind::Aggregate);
        // `count` with a body alone is implied.
        if self.is_id(W_COUNT) && self.is_p_at(1, COLON) {
            self.implied();
        } else {
            self.anon();
            self.field(Field::Operator);
        }
        if !self.is_p(COLON) {
            self.agg_part()?;
            self.field(Field::Value);
        }
        if self.is_id(W_BY) {
            self.anon();
            if self.is_p(LPAREN) {
                self.anon();
                self.list(RPAREN, Field::Key, |p| p.term().map(|_| ()))?;
                self.p_stored(RPAREN)?;
            } else {
                self.agg_part()?;
                self.field(Field::Key);
            }
        }
        if self.is_id(W_SEP) {
            self.anon();
            self.string()?;
            self.field(Field::Separator);
        }
        self.p(COLON)?;
        self.p(LBRACE)?;
        self.disjunction()?;
        self.field(Field::Body);
        self.p(RBRACE)?;
        self.close();
        Ok(false)
    }

    fn agg_part(&mut self) -> R<Call> {
        let named = std::mem::replace(&mut self.no_named, true);
        let t = self.term();
        self.no_named = named;
        t
    }

    fn call_or_aggregate(&mut self) -> R<Call> {
        let save = self.mark();
        self.open(Kind::Call);
        self.bump_as(Kind::Identifier);
        self.field(Field::Function);
        self.arguments()?;
        self.close();
        if self.is_p(COLON) {
            self.reset(save);
            return self.aggregate();
        }
        Ok(true)
    }

    fn primary(&mut self) -> R<Call> {
        let kind = match self.kind() {
            NUM => Kind::Number,
            FLOAT => Kind::Float,
            STR => Kind::String,
            TICK => Kind::CodeLiteral,
            k if k == P0 + MINUS && matches!(self.kind_at(1), NUM | FLOAT) => {
                let from = self.i;
                let kind = if self.kind_at(1) == NUM { Kind::Number } else { Kind::Float };
                self.i += 2;
                self.leaf_over(kind as u16, from, from + 2);
                return Ok(false);
            }
            k if k == P0 + MINUS => {
                self.open(Kind::UnaryExpression);
                self.implied();
                self.primary()?;
                self.field(Field::Operand);
                self.close();
                return Ok(false);
            }
            k if k == P0 + LPAREN => {
                // `(f(x))` is still a call: as a literal, an atom.
                self.open(Kind::ParenthesizedExpression);
                self.implied();
                let call = self.term()?;
                self.field(Field::Expression);
                self.p(RPAREN)?;
                self.close();
                return Ok(call);
            }
            k if k == P0 + LBRACE => {
                self.json(Field::None)?;
                return Ok(false);
            }
            k if k == P0 + AT => {
                self.open(Kind::FunctorCall);
                self.implied();
                self.id()?;
                self.field(Field::Function);
                self.arguments()?;
                self.close();
                return Ok(false);
            }
            ID => {
                let w = self.word_at(0);
                let aggregate_word = is_aggregate_word(w);
                if w == W_WILD {
                    Kind::Wildcard
                } else if w == W_COUNT && self.is_p_at(1, COLON) {
                    return self.aggregate();
                } else if aggregate_word && self.is_p_at(1, LPAREN) {
                    return self.call_or_aggregate();
                } else if aggregate_word
                    && !(self.kind_at(1) >= P0 && matches!(self.kind_at(1) - P0, COMMA | RPAREN | EQ | NE | LT | GT | LE | GE | PLUS | MINUS | STAR | SLASH | PERCENT | DOT | SEMI | RBRACE))
                {
                    let save = self.mark();
                    self.i += 1;
                    let aggregate = self.agg_part().is_ok() && (self.is_p(COLON) || self.is_id(W_BY) || self.is_id(W_SEP));
                    self.reset(save);
                    if aggregate {
                        return self.aggregate();
                    }
                    Kind::Variable
                } else if self.is_p_at(1, LPAREN) {
                    self.open(Kind::Call);
                    self.bump_as(Kind::Identifier);
                    self.field(Field::Function);
                    self.arguments()?;
                    self.close();
                    return Ok(true);
                } else if self.is_p_at(1, COLON) && !self.no_named {
                    self.open(Kind::NamedArgument);
                    self.bump_as(Kind::Identifier);
                    self.field(Field::Name);
                    self.implied();
                    self.term()?;
                    self.field(Field::Value);
                    self.close();
                    return Ok(false);
                } else {
                    Kind::Variable
                }
            }
            _ => return Err(()),
        };
        self.bump_as(kind);
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    /// The `.rr` files of the repository.
    fn corpus() -> Vec<(String, String)> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let (mut files, mut work) = (Vec::new(), vec![root]);
        while let Some(dir) = work.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for e in entries.flatten() {
                let p = e.path();
                let skip = p.file_name().is_some_and(|n| [".git", "target", "node_modules"].contains(&n.to_string_lossy().as_ref()));
                if p.is_dir() && !skip {
                    work.push(p);
                } else if p.extension().is_some_and(|x| x == "rr") {
                    files.extend(std::fs::read_to_string(&p).ok().map(|t| (p.display().to_string(), t)));
                }
            }
        }
        files
    }

    /// The lexer's tokens and comments are the engine's lexer's (`syntax::lex`), kind for kind
    /// and span for span, on every `.rr` file.
    /// Tapes whose columns need 4 bytes, each parsed again at them (`Widths::guess` writes 2
    /// below `SIZE_GUESS_BYTES`): a tail of a 70,000-byte string, an offset across a 70,000-byte
    /// comment, more than 65,535 nodes (20,000 facts). Each is the reference parser's tape, byte for byte.
    #[test]
    fn wide_tapes_are_parsed_again_at_their_widths() {
        let long = "x".repeat(70_000);
        let many = "r(1).\n".repeat(20_000);
        let cases = [
            (format!("r(\"{long}\").\n"), [2, 2, 4]),
            (format!("r(1).\n/* {long} */\nr(2).\nr(3).\n"), [2, 4, 4]),
            (many, [4, 2, 2]),
        ];
        for (src, want) in &cases {
            assert!(src.len() < rerail_tape_format::SIZE_GUESS_BYTES);
            let t = super::tape(src);
            assert_eq!(t, crate::tape::image(&crate::tape(src), src), "{} bytes", src.len());
            assert_eq!([t[9], t[10], t[11]], *want, "{} bytes", src.len());
        }
    }

    #[test]
    fn lean_tokens_are_the_engines() {
        use crate::syntax::Tok;
        let files = corpus();
        for (path, text) in &files {
            let lexed = super::lex(text);
            let engine = crate::syntax::lex(text);
            let want: Vec<(u8, u32, u32)> = engine
                .toks
                .iter()
                .map(|(t, s, e)| {
                    let k = match t {
                        Tok::Id(_) => super::ID,
                        Tok::Str(_) => super::STR,
                        Tok::Num(_) => super::NUM,
                        Tok::Float(_) => super::FLOAT,
                        Tok::Tick(_) => super::TICK,
                        Tok::Hash(_) => super::HASH,
                        Tok::Bad => super::BAD,
                        Tok::P(p) => super::P0 + crate::syntax::PUNCT.iter().position(|q| q == p).expect("punctuation") as u8,
                        Tok::Eof => super::EOF,
                    };
                    (k, *s as u32, *e as u32)
                })
                .collect();
            let got: Vec<(u8, u32, u32)> = lexed.toks[..lexed.n].iter().map(|t| (t.kind, t.start, t.end)).collect();
            if got != want {
                let k = got.iter().zip(&want).position(|(a, b)| a != b).unwrap_or(got.len().min(want.len()));
                let at = want.get(k).or(got.get(k)).map_or(0, |t| t.1 as usize);
                panic!("{path}: token {k}: lean {:?}, engine {:?}, near {:?}", got.get(k), want.get(k), text.get(at.saturating_sub(20)..(at + 20).min(text.len())));
            }
            let comments: Vec<(u32, u32)> = engine.comments.iter().map(|&(s, e)| (s as u32, e as u32)).collect();
            assert_eq!(lexed.comments, comments, "{path}");
            for (t, (tok, _, _)) in lexed.toks.iter().zip(&engine.toks) {
                if let Tok::Id(word) = tok {
                    assert_eq!(t.word, super::word_of(word.as_bytes(), 0, word.len()), "{path}: {word}");
                    let listed = super::WORD_LIST.iter().find(|w| w.0 == word.as_bytes()).map_or(0, |w| w.1);
                    assert_eq!(t.word, listed, "{path}: {word}");
                }
            }
        }
    }

    /// Every `.rr` file of the repository, and every prefix of each at a few cut points (to run
    /// the recovery paths), gives `crate::tape`'s tape.
    /// The width guess (`rerail_tape_format::Widths::guess`) against the widths each file's tape
    /// needs: the files parsed twice, and the most nodes a byte (what `SIZE_GUESS_BYTES` is
    /// set from). By hand: it prints the numbers.
    #[test]
    #[ignore = "a calibration: run by hand"]
    fn widths_guess_calibration() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let (mut files, mut work) = (Vec::new(), vec![root]);
        while let Some(dir) = work.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for e in entries.flatten() {
                let p = e.path();
                let skip = p.file_name().is_some_and(|n| [".git", "target", "node_modules"].contains(&n.to_string_lossy().as_ref()));
                if p.is_dir() && !skip {
                    work.push(p);
                } else if p.extension().is_some_and(|x| x == "rr") {
                    files.extend(std::fs::read_to_string(&p).ok());
                }
            }
        }
        let (mut again, mut most, mut big, mut largest) = (0, 0f64, 0, 0);
        for src in &files {
            let t = super::tape(src);
            let nodes = u64::from_le_bytes(t[56..64].try_into().unwrap());
            let need = rerail_tape_format::Widths { size: t[9], off: t[10], tail: t[11] & 0x7f, from_start: t[11] & 0x80 != 0 };
            let guess = rerail_tape_format::Widths::guess(src.len());
            if need != guess {
                again += 1;
                eprintln!("  {} bytes, {nodes} nodes: needs {need:?}, guessed {guess:?}", src.len());
            }
            most = most.max(nodes as f64 / src.len().max(1) as f64);
            big += usize::from(src.len() >= 1 << 16);
            largest = largest.max(src.len());
        }
        eprintln!("{} files: {again} parsed again; most nodes a byte {most:.3}; {big} of 64 KiB or more, the largest {largest} bytes", files.len());
    }

    #[test]
    fn lean_tape_is_the_engines() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let (mut files, mut work) = (Vec::new(), vec![root]);
        while let Some(dir) = work.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for e in entries.flatten() {
                let p = e.path();
                let skip = p.file_name().is_some_and(|n| [".git", "target", "node_modules"].contains(&n.to_string_lossy().as_ref()));
                if p.is_dir() && !skip {
                    work.push(p);
                } else if p.extension().is_some_and(|x| x == "rr") {
                    files.extend(std::fs::read_to_string(&p).ok().map(|t| (p.display().to_string(), t)));
                }
            }
        }
        assert!(files.len() > 1000, "{} files", files.len());
        let mut checked = 0;
        for (path, text) in &files {
            let mut cuts = vec![text.len()];
            for k in 1..8 {
                let mut at = text.len() * k / 8;
                while !text.is_char_boundary(at) {
                    at -= 1;
                }
                cuts.push(at);
            }
            for at in cuts {
                let src = &text[..at];
                let (lean, engine) = (super::tape(src), crate::tape::image(&crate::tape(src), src));
                if lean != engine {
                    let k = lean.iter().zip(&engine).position(|(a, b)| a != b).unwrap_or(lean.len().min(engine.len()));
                    panic!("{path} cut at {at}: tapes of {} / {} bytes differ from byte {k}: {}", lean.len(), engine.len(), rerail_tape_format::place(&engine, k));
                }
                checked += 1;
            }
        }
        eprintln!("{} files, {checked} texts", files.len());
    }
}
