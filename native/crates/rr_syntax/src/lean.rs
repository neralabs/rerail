//! A lean `.rr` parser for the tape alone: the baseline the compiled grammar is measured against
//! (docs/parser-generators/30-a-lean-rust-baseline.md). It is not on the product's path.
//!
//! It reads the grammar `syntax::Parser` reads and gives the same tape (`crate::tape`), node for
//! node, written as a fast hand-written parser would be:
//!
//! - **A token is one 16-byte record**: kind, span, line, and the word it is when the grammar
//!   looks at that word (`code`, `min`, `decl`, …), found once by the lexer. The parser compares
//!   one-byte codes, never text.
//! - **The lexer** finds the ends of comments, strings and literals 8 bytes a step (as `memchr`
//!   does without SIMD), and classes bytes by a table.
//! - **No statements.** Only the tape is built: each node is written in preorder as it opens.
//!   The open nodes and their children are a stack.
//! - **A wrap** (an operator found after its left operand: `a - b`, `x <= y`) puts its node
//!   before the operand. When the operand is one leaf, the two swap; only a larger operand
//!   moves the nodes after it.
//! - **Comments** are placed as the parse passes them, by `tape::flatten`'s rules; the next
//!   one's start is kept, so a node costs one compare for them.
//! - **Backtracking** cuts the tape, the stack and the comments back to a mark.

use crate::tape::{ERROR, FIRST_ANONYMOUS, Field, Kind, Node};

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

struct Parser<'s> {
    b: &'s [u8],
    t: Vec<Tok>,
    n: usize,
    i: usize,
    no_named: bool,
    /// The tape so far, in preorder.
    out: Vec<Node>,
    /// The open nodes: each its place in `out` and where its children start in `kids`.
    open: Vec<(u32, u32)>,
    /// The open nodes' children, by place in `out`.
    kids: Vec<u32>,
    comments: Vec<(u32, u32)>,
    /// The next comment to place, and where it starts (`u32::MAX`: none left).
    comment: usize,
    next_comment: u32,
}

/// The lexer alone (its tokens' count), to measure apart from the parse.
pub fn token_count(src: &str) -> usize {
    lex(src).n
}

/// `src`'s tape: `crate::tape(src)`, node for node.
pub fn tape(src: &str) -> Vec<Node> {
    let Lexed { toks, n, comments } = lex(src);
    let mut out = Vec::with_capacity(n + n / 2 + comments.len() + 8);
    out.push(Node { kind: Kind::SourceFile as u16, field: 0, start: 0, end: src.len() as u32, parent: None, extra: false });
    let next_comment = comments.first().map_or(u32::MAX, |c| c.0);
    let mut p = Parser { b: src.as_bytes(), t: toks, n, i: 0, no_named: false, out, open: vec![(0, 0)], kids: Vec::with_capacity(64), comments, comment: 0, next_comment };
    while p.kind() != EOF {
        let save = p.mark();
        if p.statement().is_err() {
            p.recover(save);
        }
    }
    // Comments after the last statement are the file's.
    p.comments_before(0, u32::MAX);
    p.out
}

impl Parser<'_> {
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

    fn line(&self) -> u32 {
        self.t.get(self.i).map_or(self.t[self.n].line, |t| t.line)
    }

    /// The line of the token before the next one's last byte.
    fn last_line(&self) -> u32 {
        let Some(j) = self.i.checked_sub(1) else { return 0 };
        let Some(t) = self.t.get(j) else { return 0 };
        if matches!(t.kind, STR | TICK | BAD) {
            t.line + newlines(&self.b[t.start as usize..(t.end as usize).saturating_sub(1).max(t.start as usize)])
        } else {
            t.line
        }
    }

    // The tape.

    fn comments_before(&mut self, parent: u32, before: u32) {
        while self.next_comment < before {
            let (start, end) = self.comments[self.comment];
            self.out.push(Node { kind: Kind::Comment as u16, field: 0, start, end, parent: Some(parent), extra: true });
            self.comment += 1;
            self.next_comment = self.comments.get(self.comment).map_or(u32::MAX, |c| c.0);
        }
    }

    fn skip_comments(&mut self, before: u32) {
        while self.next_comment < before {
            self.comment += 1;
            self.next_comment = self.comments.get(self.comment).map_or(u32::MAX, |c| c.0);
        }
    }

    fn top(&self) -> (u32, u32) {
        *self.open.last().expect("source_file stays open")
    }

    /// A child of the open node; it stays open when `open`.
    fn push(&mut self, kind: u16, start: u32, end: u32, open: bool) {
        let parent = self.top().0;
        if self.next_comment < start {
            self.comments_before(parent, start);
        }
        let id = self.out.len() as u32;
        self.out.push(Node { kind, field: 0, start, end, parent: Some(parent), extra: false });
        self.kids.push(id);
        if open {
            self.open.push((id, self.kids.len() as u32));
        } else if self.next_comment < end {
            // A leaf's text is its own: a comment inside it is not a node.
            self.skip_comments(end);
        }
    }

    fn open(&mut self, kind: Kind) {
        let at = self.start();
        self.push(kind as u16, at, at, true);
    }

    fn close(&mut self) {
        let end = match self.i.checked_sub(1) {
            Some(j) => self.t.get(j).map_or(0, |t| t.end),
            None => 0,
        };
        let (id, base) = self.open.pop().expect("an open node");
        let node = &mut self.out[id as usize];
        node.end = end.max(node.start);
        let end = node.end;
        if self.next_comment < end {
            if self.kids.len() > base as usize {
                self.comments_before(id, end);
            } else {
                self.skip_comments(end);
            }
        }
        self.kids.truncate(base as usize);
    }

    fn here(&self) -> usize {
        self.kids.len() - self.top().1 as usize
    }

    fn last(&self) -> Option<u32> {
        if self.kids.len() > self.top().1 as usize { self.kids.last().copied() } else { None }
    }

    fn field(&mut self, field: Field) {
        if let Some(last) = self.last() {
            self.out[last as usize].field = field as u16;
        }
    }

    /// A node of `kind` around the open node's children from `from` on, open, before the first.
    fn wrap(&mut self, from: usize, kind: Kind) {
        let (x, base) = self.top();
        let at = base as usize + from;
        let Some(&k) = self.kids.get(at) else {
            // Nothing to wrap: an empty node at the end.
            let id = self.out.len() as u32;
            self.out.push(Node { kind: kind as u16, field: 0, start: 0, end: 0, parent: Some(x), extra: false });
            self.kids.push(id);
            self.open.push((id, self.kids.len() as u32));
            return;
        };
        let mut first = self.out[k as usize];
        let wrapper = Node { kind: kind as u16, field: first.field, start: first.start, end: first.start, parent: Some(x), extra: false };
        first.field = 0;
        first.parent = Some(k);
        if k as usize + 1 == self.out.len() {
            // One leaf: the two swap.
            self.out[k as usize] = wrapper;
            self.out.push(first);
        } else {
            // A larger operand: the nodes from it on shift by one, and the open node's children
            // there are the new node's.
            self.out[k as usize] = first;
            for node in &mut self.out[k as usize + 1..] {
                if let Some(p) = node.parent.as_mut() {
                    if *p >= k {
                        *p += 1;
                    } else if *p == x {
                        *p = k;
                    }
                }
            }
            self.out[k as usize].parent = Some(k);
            self.out.insert(k as usize, wrapper);
        }
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

    fn leaf_over(&mut self, kind: u16, from: usize, to: usize) {
        if from < to && to <= self.n {
            self.push(kind, self.t[from].start, self.t[to - 1].end, false);
        }
    }

    fn bump_as(&mut self, kind: Kind) {
        self.leaf_over(kind as u16, self.i, self.i + 1);
        self.i += 1;
    }

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

    fn p(&mut self, p: u8) -> R<()> {
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
            self.anon();
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
            self.anon_over(LBRACE, from);
            self.leaf_over(Kind::Identifier as u16, from + 1, from + 2);
            self.field(field);
            self.anon_over(RBRACE, from + 2);
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
            self.anon();
            if !self.is_p(GT) {
                self.id()?;
                self.field(Field::Argument);
            }
            self.p(GT)?;
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

    fn rest_of_line(&mut self, line: u32, field: Field) -> R<()> {
        while self.kind() != EOF && self.line() == line {
            self.word(field)?;
        }
        Ok(())
    }

    fn raw_line(&mut self, line: u32, field: Field) -> R<()> {
        let from = self.i;
        while self.kind() != EOF && self.line() == line {
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
        let line = self.line();
        if self.kind() == HASH {
            let include = self.t[self.i].word == W_INCLUDE;
            self.open(Kind::Include);
            self.anon();
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
            if kind == Kind::Setting {
                self.anon();
                self.bump_as(Kind::Identifier);
                self.field(Field::Name);
            } else {
                self.leaf_over(keyword, self.i, self.i + 2);
                self.i += 2;
            }
            self.directive(line, d, kind)?;
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
            self.anon();
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
            self.leaf_over(Kind::RawText as u16, from, close);
            self.field(Field::Parameters);
            self.anon_over(RPAREN, close);
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
            self.anon();
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

    fn directive(&mut self, line: u32, d: u8, kind: Kind) -> R<()> {
        match d {
            W_DECL => {
                self.id()?;
                self.field(Field::Name);
                self.columns()?;
                let at = self.last_line();
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
                self.p(COLON)?;
                self.ty()?;
                self.field(Field::ReturnType);
                let at = self.last_line();
                self.rest_of_line(at, Field::Modifier)?;
            }
            W_TYPE => {
                self.id()?;
                self.field(Field::Name);
                let at = self.last_line();
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
                let mut line = line;
                while self.kind() != EOF && (self.line() == line || (self.i > 0 && self.t[self.i - 1].kind == P0 + COMMA)) {
                    line = self.line();
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
                self.anon();
                self.string()?;
                self.field(Field::Code);
            } else if (self.is_id(W_CODE) || self.is_id(W_TAG)) && self.is_p_at(1, LBRACE) {
                let is_code = self.is_id(W_CODE);
                self.open(if is_code { Kind::CodeColumn } else { Kind::TagColumn });
                self.anon();
                self.brace_column(Field::Column)?;
                if is_code && self.is_id(W_IN) && self.kind_at(1) == STR {
                    self.anon();
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
                self.anon();
                self.id()?;
                self.field(Field::Tool);
                self.string()?;
                self.field(Field::Rule);
                while self.is_p(COMMA) && self.kind_at(1) == STR {
                    self.anon();
                    self.string()?;
                    self.field(Field::Rule);
                }
                if self.is_id(W_OVER) && self.is_p_at(1, LBRACE) {
                    self.anon();
                    self.brace_column(Field::Over)?;
                }
                self.close();
                self.field(Field::Suppress);
            } else if self.is_id(W_OPTIONS) && self.is_p_at(1, LBRACE) {
                self.anon();
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
            self.anon();
            self.head()?;
            self.field(Field::Head);
        }
        if self.is_p(TURNSTILE) {
            self.anon();
            self.disjunction()?;
            self.field(Field::Body);
        }
        if self.is_p(FAT_ARROW) {
            self.anon();
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
                self.anon();
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
            self.anon();
            self.literal()?;
            self.field(Field::Literal);
        }
        self.close();
        Ok(())
    }

    fn literal(&mut self) -> R<Lit> {
        if self.is_p(BANG) {
            self.open(Kind::Negation);
            self.anon();
            let inner = self.literal()?;
            self.field(Field::Operand);
            self.close();
            return if inner == Lit::Cmp { Err(()) } else { Ok(inner) };
        }
        if self.is_p(LPAREN) {
            let save = self.mark();
            self.open(Kind::Group);
            self.anon();
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
            self.anon();
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
        self.out[call as usize].kind = Kind::Atom as u16;
        // Its function is its first child, after any comments.
        for node in &mut self.out[call as usize + 1..] {
            if node.parent == Some(call) && !node.extra {
                if node.field == Field::Function as u16 {
                    node.field = Field::Relation as u16;
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
        self.anon();
        self.field(Field::Operator);
        if !self.is_p(COLON) {
            self.agg_part()?;
            self.field(Field::Value);
        }
        if self.is_id(W_BY) {
            self.anon();
            if self.is_p(LPAREN) {
                self.anon();
                self.list(RPAREN, Field::Key, |p| p.term().map(|_| ()))?;
                self.p(RPAREN)?;
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
                self.anon();
                self.field(Field::Operator);
                self.primary()?;
                self.field(Field::Operand);
                self.close();
                return Ok(false);
            }
            k if k == P0 + LPAREN => {
                // `(f(x))` is still a call: as a literal, an atom.
                self.open(Kind::ParenthesizedExpression);
                self.anon();
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
                self.anon();
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
                    self.anon();
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
    /// Every `.rr` file of the repository, and every prefix of each at a few cut points (to run
    /// the recovery paths), gives `crate::tape`'s tape.
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
                assert_eq!(super::tape(src), crate::tape(src), "{path} cut at {at}");
                checked += 1;
            }
        }
        eprintln!("{} files, {checked} texts", files.len());
    }
}
