//! The parser: recursive descent over tree-sitter-sql's grammar (`grammar.json`), each node
//! written to the tape (`Writer`) in preorder as it is read (decision 0226).
//!
//! **Tokens** are read as tree-sitter's lexer reads them in the parse state: white space and
//! comments skipped (the external scanner's comments, `--` and nested `/* */`, kept as extras), a
//! word a keyword where the state takes that keyword (tree-sitter's keyword lexing over its `word`
//! token, `_identifier`), an operator the longest the state takes.
//!
//! **Keywords or identifiers.** Where the grammar takes an identifier, the parser names the
//! keywords the state takes too (`Ctx`): a word that is one of them is that keyword, any other word
//! an identifier. Where the parser does not know the state's keywords (after an expression, where
//! everything that may follow it counts), a keyword word it does not read is refused.
//!
//! **The tape.** A node is opened when its kind is known (`open`), or put before its first child
//! when the kind is known only after it (`wrap`: an operator after its left operand). A frame for
//! each node open, and for each mark where a node may still be put. Each token is decided by its
//! parent's token automaton (`kinds::MOVES`), exactly as the engine's
//! `TokenGrammar::stored_children` decides it: at once where the kind decides it (`kinds::FATE`),
//! else held and decided when its parent closes (`claim`), by the walk over the children written.
//! A comment goes into the node open when the next token or node is written.

use crate::block::Blocks;
use crate::kinds::{self, K, f};
use crate::{Refusal, Tape, key};
use rerail_simd8::swar::{first_of2, load8};
use rerail_tape_format::{Redo, Widths, Writer};

mod expr;
mod stmt;

pub(crate) type R<T> = Result<T, Refusal>;

/// The deepest nesting the parser follows (the stack's bound).
const MAX_DEPTH: u32 = 160;

/// `src` parsed.
pub fn parse(src: &[u8]) -> R<Tape> {
    // Stage 1 once, for every width tried.
    let mut blocks = Blocks::of(src);
    if let Some(at) = blocks.first_nul() {
        return Err(Refusal::Lex(at as u32));
    }
    if std::str::from_utf8(src).is_err() {
        return Err(Refusal::Lex(0));
    }
    // A leading byte order mark is skipped (tree-sitter's lexer skips it): the text starts after.
    let start = if src.starts_with("\u{feff}".as_bytes()) { 3 } else { 0 };
    let mut widths = Widths::guess(src.len());
    loop {
        let mut p = P {
            src,
            b: blocks,
            at: start,
            last_end: start,
            look: None,
            w: Writer::new(0, widths, src.len() / 3 + 16),
            frames: Vec::with_capacity(64),
            held: Vec::with_capacity(64),
            extras: Vec::with_capacity(16),
            flushed: 0,
            kids: Vec::with_capacity(64),
            is_held: Vec::with_capacity(64),
            out: Vec::with_capacity(64),
            keep: Vec::with_capacity(64),
            depth: 0,
            memo: (usize::MAX, Tok { t: T::Eof, s: 0, e: 0, w: 0 }),
        };
        p.program()?;
        match p.w.finish_columns() {
            Ok(columns) => return Ok(columns),
            // A value past the guessed widths: read again at the wider ones.
            Err(Redo(nw)) => {
                if nw == widths {
                    return Err(Refusal::TooLarge);
                }
                widths = nw;
                blocks = p.b;
            }
        }
    }
}

// ---- Tokens.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum T {
    Eof,
    /// A word: an identifier or a keyword, as the parse state takes it.
    Word,
    /// `"…"`: an identifier.
    Quoted,
    /// `'…'`, with `N'…'`: a string (its continuations are read by `literal`).
    Str,
    /// `E'…'`: an escape string.
    EStr,
    /// `B'…'`, `X'…'`: a bit or hex string (`_bit_string`).
    Bits,
    /// `$tag$…$tag$`.
    Dollar,
    /// A number: an integer or a decimal one.
    Num,
    /// `$1`: a parameter.
    Param,
    /// Anything else: punctuation and operators, which the parser reads by their bytes.
    Punct,
}

#[derive(Clone, Copy, Debug)]
struct Tok {
    t: T,
    s: usize,
    e: usize,
    /// A word's keyword code (`KEYWORDS`; 0: no keyword's word), found once, as it is read.
    w: u16,
}

rerail_simd8::keyword_switch!(
    /// The keyword switch (`rerail_simd8::words`) over every keyword word, case folded.
    static KEYWORDS = kinds::WORD_TEXT, fold: true
);

#[inline(always)]
fn is_word_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c >= 0x80
}

/// The binary operators' precedences (the grammar's `precedences`, the first the highest), as
/// levels: the higher binds the tighter.
mod prec {
    pub const OR: u8 = 1;
    pub const AND: u8 = 2;
    pub const BETWEEN: u8 = 3;
    pub const PATTERN: u8 = 4;
    pub const RELATION: u8 = 6;
    pub const IN: u8 = 8;
    pub const OTHER: u8 = 9;
    pub const PLUS: u8 = 11;
    pub const TIMES: u8 = 12;
    pub const EXP: u8 = 13;
    pub const NOT: u8 = 14;
    pub const IS: u8 = 15;
    pub const AT_TIME_ZONE: u8 = 16;
    pub const COLLATE: u8 = 17;
    pub const MINUS: u8 = 19;
    pub const SUBSCRIPT: u8 = 20;
    pub const TYPECAST: u8 = 21;
}

// ---- The tape as it is read: each node written at its place, its punctuation by its kind.

/// The moves out of automaton state `s`.
#[inline]
fn moves(s: u32) -> &'static [(u32, u32)] {
    &kinds::MOVES[kinds::MOVES_AT[s as usize] as usize..kinds::MOVES_AT[s as usize + 1] as usize]
}

/// The state after `l` from `s`: most states have one or two moves, read in order; a long list
/// is searched.
#[inline]
fn step(s: u32, l: u32) -> Option<u32> {
    let m = moves(s);
    if m.len() <= 8 {
        return m.iter().find(|&&(x, _)| x == l).map(|&(_, to)| to);
    }
    m.binary_search_by_key(&l, |&(x, _)| x).ok().map(|i| m[i].1)
}

#[inline]
fn accepts(s: u32) -> bool {
    kinds::ACCEPTS[s as usize / 64] >> (s % 64) & 1 == 1
}

/// The implied word from `s` to the named child `next` (`None`: to the end), where it holds a token
/// the tape can leave out: packed (`kinds::IMPLIED`), 0 for none. The state's own rows, by index.
#[inline]
fn implied(s: u32, next: Option<u32>) -> u32 {
    match next {
        Some(l) => {
            let rows = &kinds::IMPLIED[kinds::IMPLIED_AT[s as usize] as usize..kinds::IMPLIED_AT[s as usize + 1] as usize];
            if rows.len() <= 8 {
                return rows.iter().find(|&&(x, _)| x == l).map_or(0, |&(_, w)| w);
            }
            rows.binary_search_by_key(&l, |&(x, _)| x).map_or(0, |i| rows[i].1)
        }
        None => kinds::IMPLIED_END[s as usize],
    }
}

/// The tokens a node of automaton `a` leaves out, as the engine's `stored_children` decides them:
/// `kids` its children's labels in order (`kind | field << 10`, and whether the child is named),
/// `out[i]` set for each child left out. None where the walk is refused (the automaton does not
/// take the children).
fn walk(a: usize, kids: &[(u32, bool)], out: &mut [bool]) {
    // The gap `from..to` before the named child `next` (`None`: the end), from `state`.
    let close = |state: u32, from: usize, to: usize, next: Option<u32>, out: &mut [bool]| -> Option<u32> {
        let mut s = state;
        for k in &kids[from..to] {
            s = step(s, k.0)?;
        }
        let after = match next {
            Some(l) => step(s, l)?,
            None if accepts(s) => s,
            None => return None,
        };
        if to > from {
            // The word, packed: its start in `WORD`, length, and the tokens it leaves out.
            let w = implied(state, next);
            if w != 0 && (w >> 3 & 3) as usize == to - from {
                let word = &kinds::WORD[(w >> 5) as usize..(w >> 5) as usize + (to - from)];
                if word.iter().zip(&kids[from..to]).all(|(&l, k)| l == k.0) {
                    for i in from..to {
                        out[i] |= w >> (i - from) & 1 == 1;
                    }
                }
            }
        }
        Some(after)
    };
    let mut go = || -> Option<()> {
        let mut state = kinds::START[a];
        let mut from = 0;
        for (i, &(l, named)) in kids.iter().enumerate() {
            if named {
                state = close(state, from, i, Some(l), out)?;
                from = i + 1;
            }
        }
        close(state, from, kids.len(), None, out)?;
        Some(())
    };
    if go().is_none() {
        out.fill(false);
    }
}

/// Whether automaton `a` (its index) leaves token kind `t` out wherever it stands (`Some(true)`),
/// stores it (`Some(false)`), or its walk decides (`None`).
#[inline(always)]
fn fate(a: usize, t: K) -> Option<bool> {
    let fate = &kinds::FATE[a];
    let (word, bit) = (t as usize / 64, 1u64 << (t as usize % 64));
    if fate[kinds::FATE_WORDS + word] & bit != 0 { None } else { Some(fate[word] & bit != 0) }
}

/// A node being read (written, open), or a mark: a place a node may still be put over what is read
/// after it (`node`, `wrap`).
#[derive(Clone, Copy)]
struct Frame {
    /// Its node's place in the writer; a mark's, where a node put there would go.
    at: u32,
    /// Its held tokens: `held[held..]`, less those of the frames above it.
    held: u32,
    /// Its node's kind; `None` for a mark.
    kind: Option<K>,
    /// Whether it has a child (a node or a token; a comment is not one).
    any: bool,
}

/// A token whose fate waits for its parent (a mark's, a dynamic automaton's): its key, span, and
/// its place in the writer (before the node there now).
#[derive(Clone, Copy)]
struct Held {
    key: u32,
    s: u32,
    e: u32,
    at: u32,
}

/// An open node: its frame.
#[derive(Clone, Copy)]
pub(crate) struct Node(u32);

/// A mark: its frame.
#[derive(Clone, Copy)]
pub(crate) struct Mark(u32);

/// A dotted name read (`dotted`): its parts' spans, the dot before each, how many.
#[derive(Clone, Copy)]
pub(crate) struct Dotted {
    parts: [(usize, usize); 4],
    dots: [usize; 4],
    n: usize,
}

#[derive(Clone, Copy)]
struct Saved {
    at: usize,
    last_end: usize,
    look: Option<Tok>,
    depth: u32,
    nodes: usize,
    frames: usize,
    top: Frame,
    held: usize,
    flushed: usize,
    extras: usize,
}

/// An operator byte's classes (`operator_at`): 1 for `+-*/<>=` (A), 2 for `~!@#%^&|`` ` ``?` (B),
/// 4 for `*/<>=` (C).
static OP_CLASS: [u8; 256] = {
    let mut t = [0u8; 256];
    let (a, b, c) = (b"+-*/<>=", b"~!@#%^&|`?", b"*/<>=");
    let mut i = 0;
    while i < a.len() {
        t[a[i] as usize] |= 1;
        i += 1;
    }
    i = 0;
    while i < b.len() {
        t[b[i] as usize] |= 2;
        i += 1;
    }
    i = 0;
    while i < c.len() {
        t[c[i] as usize] |= 4;
        i += 1;
    }
    t
};

/// The classes of the 8 bytes at `i` (`OP_CLASS`), a byte a lane (byte `i` the lowest); none past
/// the text's end. Straight-line: 8 lookups, no loop.
#[inline(always)]
fn op_classes(src: &[u8], i: usize) -> u64 {
    let x = load8(src, i);
    let lane = |k: u32| u64::from(OP_CLASS[(x >> (8 * k)) as u8 as usize]) << (8 * k);
    lane(0) | lane(1) | lane(2) | lane(3) | lane(4) | lane(5) | lane(6) | lane(7)
}

/// The operator at `s` as the lexer reads it after an expression (`P::operator_at`).
fn operator_at(src: &[u8], s: usize) -> (bool, usize) {
    // Each 8 bytes' classes at once (`op_classes`): their lanes' A, B and C bits.
    const LANES: u64 = 0x0101_0101_0101_0101;
    // The run of operator bytes (A or B), and whether a B is in it.
    let (mut run, mut has_b) = (s, false);
    loop {
        let m = op_classes(src, run);
        let stop = (!(m | m >> 1) & LANES).trailing_zeros() as usize / 8;
        has_b |= (m >> 1) & LANES & below(stop) != 0;
        run += stop;
        if stop < 8 {
            break;
        }
    }
    let p1 = if has_b { run - s } else { 0 };
    // The run of A bytes, and the last C in it after its first byte (`op_other`'s second
    // pattern ends in one).
    let (mut a_run, mut last_c) = (s, 0);
    loop {
        let m = op_classes(src, a_run);
        let stop = (!m & LANES).trailing_zeros() as usize / 8;
        let c = (m >> 2) & LANES & below(stop);
        if c != 0 {
            last_c = a_run + (63 - c.leading_zeros() as usize) / 8 + 1;
        }
        a_run += stop;
        if stop < 8 {
            break;
        }
    }
    let p2 = if last_c >= s + 2 { last_c - s } else { 0 };
    let strings: &[&[u8]] = &[b"<=", b"!=", b">=", b"<>", b"::", b"+", b"-", b"*", b"/", b"%", b"^", b"=", b"<", b">"];
    let st = strings.iter().filter(|x| src[s..].starts_with(x)).map(|x| x.len()).max().unwrap_or(0);
    let pat = p1.max(p2);
    if pat > st { (true, pat) } else { (false, st) }
}

/// The lanes below lane `n` of 8 (all 8 where `n` is 8).
#[inline(always)]
fn below(n: usize) -> u64 {
    if n < 8 { (1u64 << (8 * n)) - 1 } else { !0 }
}

/// The kind of a key's kind bits.
#[inline(always)]
fn kind_of(k: usize) -> K {
    assert!(k <= kinds::LAST as usize, "a kind");
    // SAFETY: `K` is `repr(u16)`, its values every number from 0 to `LAST`.
    unsafe { std::mem::transmute::<u16, K>(k as u16) }
}

/// Where the grammar takes an identifier, the keywords the parse state takes too: `Sure`, the
/// parser knows them all (any other keyword word is an identifier); `Unsure`, it does not (a
/// keyword word not among them is refused).
#[derive(Clone, Copy)]
enum Ctx<'a> {
    Sure(&'a [&'a [K]]),
    Unsure(&'a [&'a [K]]),
}

const NONE: Ctx<'static> = Ctx::Sure(&[]);

pub(crate) struct P<'s> {
    src: &'s [u8],
    b: Blocks<'s>,
    /// Where the next token's white space starts (after the last token read).
    at: usize,
    /// The last token's end.
    last_end: usize,
    look: Option<Tok>,
    /// The tape, written as the text is read.
    w: Writer,
    /// The nodes being read and the marks (`Frame`), the program's first.
    frames: Vec<Frame>,
    /// The tokens waiting for their parent (`Held`), in order.
    held: Vec<Held>,
    /// Comments, in order: (kind, start, end); those before `flushed` are written.
    extras: Vec<(K, u32, u32)>,
    flushed: usize,
    /// A closing node's children and its held tokens' fates (`claim`'s scratch).
    kids: Vec<(u32, bool)>,
    is_held: Vec<bool>,
    out: Vec<bool>,
    keep: Vec<bool>,
    depth: u32,
    /// The last token read (`lex`), and where its read started: read again from there, it is the
    /// same token. `restore` clears it (the extras it read may be gone).
    memo: (usize, Tok),
}

impl<'s> P<'s> {
    #[inline(always)]
    fn byte(&self, at: usize) -> u8 {
        self.src.get(at).copied().unwrap_or(0)
    }

    fn refuse<X>(&self) -> R<X> {
        Err(Refusal::Parse(self.look.map_or(self.at, |t| t.s) as u32))
    }

    /// The parser's state, to go back to (`restore`): where a reading tree-sitter's GLR prefers
    /// (a transaction over its first statement) may not hold.
    /// What is read in a parse that may not hold changes only what is read after this: nodes
    /// written after it, frames above it, the innermost frame's state.
    fn save(&self) -> Saved {
        Saved {
            at: self.at,
            last_end: self.last_end,
            look: self.look,
            depth: self.depth,
            nodes: self.w.len(),
            frames: self.frames.len(),
            top: *self.frames.last().expect("the program's frame"),
            held: self.held.len(),
            flushed: self.flushed,
            extras: self.extras.len(),
        }
    }

    fn restore(&mut self, s: Saved) {
        (self.at, self.last_end, self.look, self.depth) = (s.at, s.last_end, s.look, s.depth);
        self.memo.0 = usize::MAX;
        self.w.truncate(s.nodes);
        self.frames.truncate(s.frames);
        *self.frames.last_mut().expect("the program's frame") = s.top;
        self.held.truncate(s.held);
        self.flushed = s.flushed;
        self.extras.truncate(s.extras);
    }

    fn deeper(&mut self) -> R<()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH { self.refuse() } else { Ok(()) }
    }

    // ---- The tape: frames, nodes, tokens.

    #[inline(always)]
    fn top(&mut self) -> &mut Frame {
        self.frames.last_mut().expect("the program's frame")
    }

    /// The comments before `before`, written into the node open (a comment before a node's first
    /// child is its parent's).
    #[inline(always)]
    fn flush(&mut self, before: usize) {
        while let Some(&(k, s, e)) = self.extras.get(self.flushed) {
            if s as usize >= before {
                break;
            }
            self.flushed += 1;
            let at = self.w.push(key(k, 0, true), s);
            self.w.close(at, e);
        }
    }

    /// The program, the root: open before anything is read. Its span is from its first child or
    /// comment (the text's end where it has none) to the text's end; the comments before its first
    /// statement and after its last are its own.
    pub(super) fn open_program(&mut self) -> Node {
        let at = self.w.push(key(K::Program, 0, false), 0);
        self.frames.push(Frame { at: at as u32, held: 0, kind: Some(K::Program), any: false });
        Node(0)
    }

    pub(super) fn close_program(&mut self, n: Node) {
        self.end_marks(n.0);
        self.flush(usize::MAX);
        let end = self.src.len();
        let first = self.held.first().map(|h| h.s as usize);
        let first = match (self.w.len() > 1, first) {
            (true, Some(h)) => h.min(self.w.start(1) as usize),
            (true, None) => self.w.start(1) as usize,
            (false, Some(h)) => h,
            (false, None) => end,
        };
        let f = self.frames.pop().expect("the program's frame");
        self.claim(0, K::Program, f.held as usize);
        self.w.set_start(0, first as u32);
        self.w.close(0, end as u32);
    }

    /// A mark at the next token: a node may be put over what is read from here on (`node`,
    /// `wrap`). It needs no end: the node below it closing ends it.
    pub(super) fn mark(&mut self) -> R<Mark> {
        let t = self.peek()?;
        self.flush(t.s);
        self.frames.push(Frame { at: self.w.len() as u32, held: self.held.len() as u32, kind: None, any: false });
        Ok(Mark(self.frames.len() as u32 - 1))
    }

    /// A node of `kind` given `field`, open at the next token: what is read until its `close` is
    /// its.
    pub(super) fn open(&mut self, kind: K, field: u8) -> R<Node> {
        let t = self.peek()?;
        self.flush(t.s);
        self.top().any = true;
        let at = self.w.push(key(kind, field, false), t.s as u32);
        self.frames.push(Frame { at: at as u32, held: self.held.len() as u32, kind: Some(kind), any: false });
        Ok(Node(self.frames.len() as u32 - 1))
    }

    /// The marks above frame `i` ended: their children are frame `i`'s.
    #[inline(always)]
    fn end_marks(&mut self, i: u32) {
        let mut any = false;
        while self.frames.len() > i as usize + 1 {
            let f = self.frames.pop().expect("a frame");
            // A node left open would have no size, and a walk over its parent's children would
            // not end: a parser bug stops here.
            assert!(f.kind.is_none(), "a node left open");
            any |= f.any;
        }
        self.frames[i as usize].any |= any;
    }

    /// A node of `kind` given `field` put at mark `m`, over what was read since (an operator's left
    /// operand), left open: what is read until its `close` is its too. `m` stays, now holding it.
    pub(super) fn wrap(&mut self, m: Mark, kind: K, field: u8) -> Node {
        self.end_marks(m.0);
        let f = self.frames[m.0 as usize];
        let (at, held) = (f.at as usize, f.held as usize);
        // Its start: its first child's, a held token's where one comes first.
        let start = match self.held.get(held) {
            Some(h) if h.at as usize == at => h.s,
            _ if at < self.w.len() => self.w.start(at),
            _ => 0,
        };
        self.w.insert(at, key(kind, field, false), start);
        for h in &mut self.held[held..] {
            h.at += 1;
        }
        self.frames[m.0 as usize] = Frame { any: true, ..f };
        self.frames.push(Frame { at: at as u32, held: held as u32, kind: Some(kind), any: f.any });
        Node(self.frames.len() as u32 - 1)
    }

    /// A node of `kind` over what was read since mark `m`, closed: its place. `m` stays (a node of
    /// another kind may be put over this one: `a + b + c`).
    pub(super) fn node(&mut self, kind: K, m: Mark) -> u32 {
        let n = self.wrap(m, kind, 0);
        self.close(n)
    }

    /// A node of `kind` over what was read since `m`, spanning `s..e` (a node whose last bytes are
    /// hidden tokens: a signed number's digits).
    pub(super) fn node_span(&mut self, kind: K, m: Mark, s: usize, e: usize) -> u32 {
        let n = self.wrap(m, kind, 0);
        self.close_span(n, Some((s, e)))
    }

    /// A node of `kind` over what was read since `m`, ending at `e` (its last bytes hidden tokens
    /// read after its last child: an interval's string continuations).
    pub(super) fn node_end(&mut self, kind: K, m: Mark, e: usize) -> u32 {
        let n = self.wrap(m, kind, 0);
        let s = self.w.start(self.frames[n.0 as usize].at as usize) as usize;
        self.close_span(n, Some((s, e)))
    }

    /// Closes node `n` at `e` (a node whose last bytes are no child's: an interval's string).
    pub(super) fn close_end(&mut self, n: Node, e: usize) -> u32 {
        let s = self.w.start(self.frames[n.0 as usize].at as usize) as usize;
        self.close_span(n, Some((s, e)))
    }

    /// Whether a child (a node or a token) was read since mark `m`.
    pub(super) fn read_since(&self, m: Mark) -> bool {
        self.frames[m.0 as usize..].iter().any(|f| f.any)
    }

    /// The first node read since mark `m` (its place): an operator's left operand.
    #[inline]
    pub(super) fn mark_child(&self, m: Mark) -> u32 {
        self.frames[m.0 as usize].at
    }

    /// Closed node `id`'s end.
    pub(super) fn end_at(&self, id: u32) -> usize {
        self.w.end(id as usize) as usize
    }

    /// A child of `kind` at `s..e`: a named leaf (`leaf`) or a token (`token`), by the kind.
    pub(super) fn child(&mut self, kind: K, field: u8, s: usize, e: usize) {
        if kinds::KINDS[kind as usize].1 {
            self.leaf(kind, field, s, e);
        } else {
            self.token(kind, field, s, e);
        }
    }

    /// Closes node `n`: its held tokens decided and the stored ones written, its end the last
    /// token's (an empty node's span `0..0`). Its place.
    #[inline]
    pub(super) fn close(&mut self, n: Node) -> u32 {
        self.close_span(n, None)
    }

    pub(super) fn close_span(&mut self, n: Node, span: Option<(usize, usize)>) -> u32 {
        self.end_marks(n.0);
        let f = self.frames.pop().expect("its frame");
        let at = f.at as usize;
        self.claim(at, f.kind.expect("a node"), f.held as usize);
        match span {
            Some((s, e)) => {
                self.w.set_start(at, s as u32);
                self.w.close(at, e as u32);
            }
            None if !f.any => {
                self.w.set_start(at, 0);
                self.w.close(at, 0);
            }
            None => self.w.close(at, self.last_end as u32),
        }
        at as u32
    }

    /// The held tokens from `held[from]` on, children of the node at `at` of `kind`: each left out
    /// or written at its place, by the kind (`FATE`) or by its automaton's walk over the node's
    /// children.
    fn claim(&mut self, at: usize, kind: K, from: usize) {
        let n = self.held.len() - from;
        if n == 0 {
            return;
        }
        let a = kinds::AUTOMATON[kind as usize] as usize;
        self.keep.clear();
        if a == 0 {
            self.keep.resize(n, true);
        } else if kinds::DYNAMIC[a - 1] {
            // The children in order: the nodes written (a node's subtree skipped by its size, a
            // comment passed), each held token before the node at its place.
            self.kids.clear();
            self.is_held.clear();
            let (mut h, mut i) = (from, at + 1);
            loop {
                while h < self.held.len() && self.held[h].at as usize <= i {
                    self.kids.push((self.held[h].key & 0xffff, false));
                    self.is_held.push(true);
                    h += 1;
                }
                if i >= self.w.len() {
                    break;
                }
                let k = self.w.key(i);
                if k >> 16 & 1 == 0 {
                    self.kids.push((k & 0xffff, kinds::KINDS[(k & 0x3ff) as usize].1));
                    self.is_held.push(false);
                }
                i += self.w.size(i) as usize;
            }
            self.out.clear();
            self.out.resize(self.kids.len(), false);
            walk(a - 1, &self.kids, &mut self.out);
            for (j, &held) in self.is_held.iter().enumerate() {
                if held {
                    self.keep.push(!self.out[j]);
                }
            }
        } else {
            // A token the kind decides (a mark's, held before its node was known); a token the
            // automaton never takes is stored, as the walk's refusal stores it.
            for h in &self.held[from..] {
                let t = (h.key & 0x3ff) as usize;
                self.keep.push(fate(a - 1, kind_of(t)) != Some(true));
            }
        }
        let mut off = 0;
        for (h, &keep) in self.held[from..].iter().zip(&self.keep) {
            if keep {
                self.w.insert_leaf(h.at as usize + off, h.key, h.s, h.e);
                off += 1;
            }
        }
        self.held.truncate(from);
    }

    /// A named leaf (a keyword, an identifier, a literal) at `s..e`, given `field`: its place.
    #[inline]
    pub(super) fn leaf(&mut self, kind: K, field: u8, s: usize, e: usize) -> u32 {
        self.flush(s);
        self.top().any = true;
        let at = self.w.push(key(kind, field, false), s as u32);
        self.w.close(at, e as u32);
        at as u32
    }

    /// A token (punctuation, an anonymous kind) at `s..e`: written, left out, or held, by its
    /// parent's kind (`FATE`). Held in a mark (its node is not known yet) and in a node whose walk
    /// decides some token (its children all read first).
    #[inline]
    pub(super) fn token(&mut self, kind: K, field: u8, s: usize, e: usize) {
        self.flush(s);
        let f = self.top();
        f.any = true;
        let k = key(kind, field, false);
        if let Some(p) = f.kind {
            let a = kinds::AUTOMATON[p as usize] as usize;
            let stored = if a == 0 {
                Some(true)
            } else if kinds::DYNAMIC[a - 1] {
                None
            } else {
                fate(a - 1, kind).map(|dropped| !dropped)
            };
            match stored {
                Some(true) => {
                    let at = self.w.push(k, s as u32);
                    self.w.close(at, e as u32);
                    return;
                }
                Some(false) => return,
                None => {}
            }
        }
        self.held.push(Held { key: k, s: s as u32, e: e as u32, at: self.w.len() as u32 });
    }

    /// Node `id` given `field`, where it has none: a field given inside a hidden rule wins over one
    /// given outside it, as tree-sitter's cursor looks for a node's field from its nearest hidden
    /// parent out.
    #[inline]
    pub(super) fn set_field(&mut self, id: u32, field: u8) {
        let k = self.w.key(id as usize);
        if k >> 10 & 0x3f == 0 {
            self.w.set_key(id as usize, k | u32::from(field) << 10);
        }
    }

    /// Node `id`'s kind.
    #[inline]
    pub(super) fn kind_at(&self, id: u32) -> K {
        kind_of((self.w.key(id as usize) & 0x3ff) as usize)
    }

    /// Node `id` given `kind` (a kind found after it closed whose tokens' fates are the same:
    /// `(x)` a parenthesized expression read as a one-item list).
    pub(super) fn retype(&mut self, id: u32, kind: K) {
        let k = self.w.key(id as usize);
        self.w.set_key(id as usize, k & !0x3ff | kind as u32);
    }

    /// The children of mark `m` given `field` (a hidden rule's field falls on its nodes), where
    /// they have none.
    pub(super) fn field_since(&mut self, m: Mark, field: u8) {
        let mut i = self.frames[m.0 as usize].at as usize;
        while i < self.w.len() {
            if self.w.key(i) >> 16 & 1 == 0 {
                self.set_field(i as u32, field);
            }
            i += (self.w.size(i) as usize).max(1);
        }
    }

    // ---- The lexer.

    /// Skips white space and comments (kept as extras) from `self.at`: the next token's start.
    fn skip(&mut self) -> R<usize> {
        let mut at = self.at;
        loop {
            at = self.b.blank_end(at);
            match (self.byte(at), self.byte(at + 1)) {
                (b'-', b'-') => {
                    let e = self.b.line_end(at);
                    self.extra(K::Comment, at, e);
                    at = e;
                }
                (b'/', b'*') => {
                    let Some(e) = self.b.comment_end(at + 2) else { return Err(Refusal::Parse(at as u32)) };
                    self.extra(K::Comment, at, e);
                    at = e;
                }
                // A psql meta-command: an extra, or (a command that sends the query buffer) the token
                // `program` takes in a statement's `;` place.
                (b'\\', _) => match self.meta_command(at)? {
                    Some(e) => {
                        self.extra(K::MetaCommand, at, e);
                        at = e;
                    }
                    None => return Ok(at),
                },
                _ => return Ok(at),
            }
        }
    }

    /// The psql meta-command at `at` (its `\\`), as the scanner reads it: its end, where it is an
    /// extra; `None` for a command that sends the query buffer (`\\g`, `\\gset`, …), which the
    /// grammar's `_meta_command_send` token reads (`send_end`).
    fn meta_command(&self, at: usize) -> R<Option<usize>> {
        let e = self.meta_name_end(at);
        let name = &self.src[at + 1..e];
        if name.is_empty() {
            return Err(Refusal::Parse(at as u32));
        }
        if name.len() <= 16 && [&b"g"[..], b"gx", b"gset", b"gexec", b"gdesc", b"crosstabview", b"watch"].contains(&name) {
            return Ok(None);
        }
        let line = self.b.line_end(e);
        if name == b"set" || name == b"copy" {
            if name == b"copy" {
                // `\copy (query)`: refused. The blanks before it are on the command's line.
                let x = self.b.blank_end(e);
                if self.byte(x) == b'(' && first_of2(self.src, e, x, b'\n', b'\r') == x {
                    return Err(Refusal::Parse(at as u32));
                }
            }
            return Ok(Some(first_of2(self.src, e, line, b'\r', b'\r')));
        }
        let x = first_of2(self.src, e, line, b'\r', b'\\');
        Ok(Some(if self.byte(x) == b'\\' && self.byte(x + 1) == b'\\' { x + 2 } else { x }))
    }

    /// The end of a meta-command's name after its `\\` at `at`: its first blank or `\\`.
    #[inline]
    fn meta_name_end(&self, at: usize) -> usize {
        first_of2(self.src, at + 1, self.b.blank_stop(at + 1), b'\\', b'\\')
    }

    /// The end of the send command at `at` (`_meta_command_send`: `\\g`, its arguments after a
    /// blank, and a closing `\\\\`). Its name is one of `meta_command`'s send names, all letters.
    fn send_end(&self, at: usize) -> usize {
        let mut e = self.meta_name_end(at);
        if matches!(self.byte(e), b' ' | b'\t' | 0x0b | 0x0c) {
            e = first_of2(self.src, e, self.b.line_end(e), b'\r', b'\\');
        }
        if self.byte(e) == b'\\' && self.byte(e + 1) == b'\\' {
            e += 2;
        }
        e
    }

    fn extra(&mut self, kind: K, s: usize, e: usize) {
        // A token read twice (a look ahead, then again) adds its comments once.
        if self.extras.last().is_none_or(|x| (x.1 as usize) < s) {
            self.extras.push((kind, s as u32, e as u32));
        }
    }

    /// The next token.
    #[inline]
    fn peek(&mut self) -> R<Tok> {
        match self.look {
            Some(t) => Ok(t),
            None => self.lex(),
        }
    }

    /// The next token, read from the text (`peek` when none is read ahead).
    #[inline(never)]
    fn lex(&mut self) -> R<Tok> {
        // A token read ahead and given back (a two-token look) is not read again.
        if self.memo.0 == self.at {
            self.look = Some(self.memo.1);
            return Ok(self.memo.1);
        }
        let from = self.at;
        let s = self.skip()?;
        let c = self.byte(s);
        let tok = |t, e| Tok { t, s, e, w: 0 };
        let t = if s >= self.src.len() {
            tok(T::Eof, s)
        } else if is_word_start(c) {
            let e = self.b.word_end(s);
            if e - s == 1 && self.byte(e) == b'\'' {
                match c | 0x20 {
                    b'n' => tok(T::Str, self.quoted_end(e)?),
                    b'e' => tok(T::EStr, self.escape_end(e)?),
                    b'b' | b'x' => tok(T::Bits, self.quoted_end(e)?),
                    _ => return self.refuse(),
                }
            } else if self.byte(e) == b'&' && (c | 0x20) == b'u' && e - s == 1 {
                return self.refuse();
            } else {
                self.check_word(s, e)?;
                Tok { t: T::Word, s, e, w: KEYWORDS.find(self.src, s, e) as u16 }
            }
        } else if c == b'"' {
            let mut e = s + 1;
            loop {
                let q = self.b.dquote_stop(e);
                if q >= self.src.len() {
                    return Err(Refusal::Parse(s as u32));
                }
                if self.byte(q + 1) == b'"' {
                    e = q + 2;
                } else {
                    break tok(T::Quoted, q + 1);
                }
            }
        } else if c == b'\'' {
            tok(T::Str, self.quoted_end(s)?)
        } else if c.is_ascii_digit() || c == b'.' && self.byte(s + 1).is_ascii_digit() {
            let e = self.number_end(s)?;
            tok(T::Num, e)
        } else if c == b'$' {
            let d = self.b.word_end(s + 1);
            if d > s + 1 && self.src[s + 1..d].iter().all(u8::is_ascii_digit) {
                tok(T::Param, d)
            } else {
                tok(T::Dollar, s)
            }
        } else {
            tok(T::Punct, s)
        };
        self.look = Some(t);
        self.memo = (from, t);
        Ok(t)
    }

    /// A word's bytes past ASCII: no white space (tree-sitter's `\s` takes Unicode's, which a word
    /// here would not split at).
    fn check_word(&self, s: usize, e: usize) -> R<()> {
        if e - s > 0 && self.src[s..e].iter().any(|&b| b >= 0x80) {
            let w = std::str::from_utf8(&self.src[s..e]).map_err(|_| Refusal::Lex(s as u32))?;
            if w.chars().any(|c| c.is_whitespace() || c == '\u{feff}' || c == '\u{200b}') {
                return Err(Refusal::Lex(s as u32));
            }
        }
        Ok(())
    }

    /// The end of the string whose quote is at `q`: `'…'`, a `''` inside it a quote.
    fn quoted_end(&self, q: usize) -> R<usize> {
        let mut e = q + 1;
        loop {
            let x = self.b.quote_stop(e);
            if x >= self.src.len() {
                return Err(Refusal::Parse(q as u32));
            }
            if self.src[x] == b'\\' {
                e = x + 1;
                continue;
            }
            if self.byte(x + 1) == b'\'' {
                e = x + 2;
            } else {
                return Ok(x + 1);
            }
        }
    }

    /// The end of the escape string whose quote is at `q`: `\` escapes the next byte.
    fn escape_end(&self, q: usize) -> R<usize> {
        let mut e = q + 1;
        loop {
            let x = self.b.quote_stop(e);
            if x >= self.src.len() {
                return Err(Refusal::Parse(q as u32));
            }
            if self.src[x] == b'\\' {
                e = x + 2;
                continue;
            }
            if self.byte(x + 1) == b'\'' {
                e = x + 2;
            } else {
                return Ok(x + 1);
            }
        }
    }

    /// A number's end: `_integer`'s and `_decimal_number`'s patterns (decimal digits, with `_`
    /// between them, a fraction, an exponent). A number a word byte follows, or a hexadecimal,
    /// octal or binary one, is refused.
    fn number_end(&self, s: usize) -> R<usize> {
        let digits = |mut at: usize| {
            let from = at;
            while self.byte(at).is_ascii_digit() || self.byte(at) == b'_' && self.byte(at + 1).is_ascii_digit() && at > from {
                at += 1;
            }
            at
        };
        if self.byte(s) == b'0' && matches!(self.byte(s + 1) | 0x20, b'x' | b'o' | b'b') {
            return Err(Refusal::Parse(s as u32));
        }
        let mut e = digits(s);
        if self.byte(e) == b'.' {
            e = digits(e + 1);
        }
        if matches!(self.byte(e), b'e' | b'E') {
            let mut x = e + 1;
            if matches!(self.byte(x), b'+' | b'-') {
                x += 1;
            }
            if self.byte(x).is_ascii_digit() {
                e = digits(x);
            }
        }
        if is_word_start(self.byte(e)) || self.byte(e).is_ascii_digit() || self.byte(e) == b'$' || self.byte(e) == b'.' {
            return Err(Refusal::Parse(s as u32));
        }
        Ok(e)
    }

    /// Takes the token `t` (from `peek`).
    #[inline]
    fn take(&mut self, t: Tok) {
        self.at = t.e;
        self.last_end = t.e;
        self.look = None;
    }

    /// The keywords the next token, a word, is (`kinds::WORDS`); empty for any other.
    #[inline]
    fn words(&mut self) -> R<&'static [(&'static str, K)]> {
        let t = self.peek()?;
        let (a, b) = kinds::WORD_AT[usize::from(t.w)];
        Ok(&kinds::WORDS[usize::from(a)..usize::from(b)])
    }

    /// Whether the next token is keyword `k`.
    fn is(&mut self, k: K) -> R<bool> {
        Ok(self.words()?.iter().any(|&(_, x)| x == k))
    }

    /// Keyword `k`, read as a leaf, where it is the next token.
    fn kw(&mut self, k: K) -> R<Option<u32>> {
        if !self.is(k)? {
            return Ok(None);
        }
        let t = self.peek()?;
        self.take(t);
        Ok(Some(self.leaf(k, 0, t.s, t.e)))
    }

    fn expect(&mut self, k: K) -> R<u32> {
        match self.kw(k)? {
            Some(id) => Ok(id),
            None => self.refuse(),
        }
    }

    /// Keyword `k` given `field`.
    fn kw_field(&mut self, k: K, field: u8) -> R<Option<u32>> {
        let id = self.kw(k)?;
        if let Some(id) = id {
            self.set_field(id, field);
        }
        Ok(id)
    }

    /// Whether the next token is punctuation `p` (its bytes), and no longer operator.
    fn at_punct(&mut self, p: &[u8]) -> R<bool> {
        let t = self.peek()?;
        Ok(t.t == T::Punct && self.src[t.s..].starts_with(p) && self.op_len(t.s, p.len()))
    }

    /// Whether the token at `s` is the `n` bytes there, as the lexer takes it: no longer operator
    /// (`op_other`, `<=`, `::`) continues it.
    fn op_len(&self, s: usize, n: usize) -> bool {
        let c = self.src[s];
        match c {
            b'(' | b')' | b',' | b';' | b'[' | b']' | b'.' => n == 1,
            b':' => n == 2 || self.byte(s + 1) != b':' && self.byte(s + 1) != b'=',
            _ => self.operator_at(s).1 == n,
        }
    }

    /// Punctuation `p`, an anonymous token, where it is next (`token`).
    fn punct(&mut self, p: &[u8], kind: K) -> R<Option<()>> {
        if !self.at_punct(p)? {
            return Ok(None);
        }
        let t = self.peek()?;
        let e = t.s + p.len();
        self.take(Tok { e, ..t });
        self.token(kind, 0, t.s, e);
        Ok(Some(()))
    }

    fn expect_punct(&mut self, p: &[u8], kind: K) -> R<()> {
        match self.punct(p, kind)? {
            Some(()) => Ok(()),
            None => self.refuse(),
        }
    }

    /// The operator at `s` as the lexer reads it after an expression: the longest of the string
    /// operators and `op_other`'s two patterns, a string winning a tie. (whether it is
    /// `op_other`, its length).
    fn operator_at(&self, s: usize) -> (bool, usize) {
        operator_at(self.src, s)
    }

    // ---- Identifiers.

    /// An identifier, where the next token is one in `ctx` (see `Ctx`), read: its span; `None`
    /// where it is not.
    fn ident_span(&mut self, ctx: Ctx) -> R<Option<(usize, usize)>> {
        let t = self.peek()?;
        match t.t {
            T::Quoted => {}
            T::Word => {
                let ws = self.words()?;
                if !ws.is_empty() {
                    let (sets, sure) = match ctx {
                        Ctx::Sure(s) => (s, true),
                        Ctx::Unsure(s) => (s, false),
                    };
                    if ws.iter().any(|&(_, k)| sets.iter().any(|set| set.contains(&k))) {
                        return Ok(None);
                    }
                    if !sure {
                        return self.refuse();
                    }
                }
            }
            _ => return Ok(None),
        }
        self.take(t);
        Ok(Some((t.s, t.e)))
    }

    /// An identifier in `ctx`, given `field`, written: its place; `None` where none is next.
    fn ident(&mut self, ctx: Ctx, field: u8) -> R<Option<u32>> {
        Ok(self.ident_span(ctx)?.map(|(s, e)| self.leaf(K::Identifier, field, s, e)))
    }

    /// `_column`: an identifier, or a quoted name as the `literal` the grammar aliases its
    /// `_literal_string` to (`"ip"`, which both read: tree-sitter takes the `literal`).
    fn expect_column(&mut self, ctx: Ctx, field: u8) -> R<u32> {
        let t = self.peek()?;
        let quoted = self.src.get(t.s as usize) == Some(&b'"');
        let Some((s, e)) = self.ident_span(ctx)? else { return self.refuse() };
        Ok(self.leaf(if quoted { K::Literal } else { K::Identifier }, field, s, e))
    }

    fn expect_ident(&mut self, ctx: Ctx, field: u8) -> R<u32> {
        match self.ident(ctx, field)? {
            Some(id) => Ok(id),
            None => self.refuse(),
        }
    }

    /// The parts of a dotted name (`a`, `a.b`, …, at most `max`), the first an identifier in `ctx`,
    /// read: each part's span and the dot before it (`dots[0]` unused); `None` where the next token
    /// is no identifier. A dot followed by no identifier (`a.*`, `a.(`) is left unread.
    fn dotted(&mut self, ctx: Ctx, max: usize) -> R<Option<Dotted>> {
        let Some(first) = self.ident_span(ctx)? else { return Ok(None) };
        let mut d = Dotted { parts: [first; 4], dots: [0; 4], n: 1 };
        while self.at_punct(b".")? {
            let t = self.peek()?;
            let save = (self.at, self.last_end, self.look);
            self.take(Tok { e: t.s + 1, ..t });
            let Some(part) = self.ident_span(NONE)? else {
                (self.at, self.last_end, self.look) = save;
                break;
            };
            if d.n == max {
                return self.refuse();
            }
            d.parts[d.n] = part;
            d.dots[d.n] = t.s;
            d.n += 1;
        }
        Ok(Some(d))
    }

    /// `object_reference`: `name`, `schema.name` or `database.schema.name`, the first part an
    /// identifier in `ctx`. `None` where the next token is no identifier.
    fn object_reference(&mut self, ctx: Ctx, field: u8) -> R<Option<u32>> {
        let Some(d) = self.dotted(ctx, 3)? else { return Ok(None) };
        Ok(Some(self.write_reference(&d, d.n, field)))
    }

    /// The `object_reference` of `d`'s first `n` parts given `field`, written: identifiers (`name`,
    /// `schema`, `database` from the last back) and the dots between them.
    fn write_reference(&mut self, d: &Dotted, n: usize, field: u8) -> u32 {
        let (s, e) = (d.parts[0].0, d.parts[n - 1].1);
        self.flush(s);
        self.top().any = true;
        let at = self.w.push(key(K::ObjectReference, field, false), s as u32);
        self.frames.push(Frame { at: at as u32, held: self.held.len() as u32, kind: Some(K::ObjectReference), any: true });
        let fields: &[u8] = match n {
            1 => &[f::NAME],
            2 => &[f::SCHEMA, f::NAME],
            _ => &[f::DATABASE, f::SCHEMA, f::NAME],
        };
        for i in 0..n {
            if i > 0 {
                self.token(K::TDot, 0, d.dots[i], d.dots[i] + 1);
            }
            self.leaf(K::Identifier, fields[i], d.parts[i].0, d.parts[i].1);
        }
        let node = Node(self.frames.len() as u32 - 1);
        self.close_span(node, Some((s, e)))
    }
}

#[cfg(test)]
mod tests {
    /// `operator_at` a byte at a time, as it was before it read 8 bytes a step.
    fn operator_by_bytes(src: &[u8], s: usize) -> (bool, usize) {
        const A: &[u8] = b"+-*/<>=";
        const B: &[u8] = b"~!@#%^&|`?";
        const C: &[u8] = b"*/<>=";
        let (mut run, mut has_b) = (s, false);
        while let Some(&c) = src.get(run) {
            if B.contains(&c) {
                has_b = true;
            } else if !A.contains(&c) {
                break;
            }
            run += 1;
        }
        let p1 = if has_b { run - s } else { 0 };
        let mut a_run = s;
        while src.get(a_run).is_some_and(|c| A.contains(c)) {
            a_run += 1;
        }
        let p2 = (s + 2..=a_run).rev().find(|&k| C.contains(&src[k - 1])).map_or(0, |k| k - s);
        let strings: &[&[u8]] = &[b"<=", b"!=", b">=", b"<>", b"::", b"+", b"-", b"*", b"/", b"%", b"^", b"=", b"<", b">"];
        let st = strings.iter().filter(|x| src[s..].starts_with(x)).map(|x| x.len()).max().unwrap_or(0);
        let pat = p1.max(p2);
        if pat > st { (true, pat) } else { (false, st) }
    }

    #[test]
    fn operators_8_bytes_a_step_as_a_byte_at_a_time() {
        // Every text of up to 4 bytes from operator bytes and a stop, and long runs across the
        // 8-byte steps, at each start.
        let alphabet = b"+-*/<>=~!@#%^&|`?:a ";
        let mut texts: Vec<Vec<u8>> = vec![Vec::new()];
        for _ in 0..4 {
            let more: Vec<Vec<u8>> = texts.iter().flat_map(|t| alphabet.iter().map(move |&c| [t.as_slice(), &[c]].concat())).collect();
            texts.extend(more);
        }
        for n in [7, 8, 9, 15, 16, 17, 30] {
            for (a, b) in [(b'-', b'-'), (b'<', b'='), (b'-', b'!'), (b'=', b'*'), (b'|', b'|')] {
                let mut t: Vec<u8> = (0..n).map(|i| if i % 3 == 2 { b } else { a }).collect();
                texts.push(t.clone());
                t.push(b'x');
                texts.push(t);
            }
        }
        for t in &texts {
            for s in 0..t.len() {
                assert_eq!(super::operator_at(t, s), operator_by_bytes(t, s), "{:?} at {s}", String::from_utf8_lossy(t));
            }
        }
    }
}
