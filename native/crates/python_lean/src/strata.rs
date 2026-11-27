//! A lexer of strata (decision 0698, items 92 and 93): the lean lexer's tokens and extras, made in layers of
//! masks, so that the two can be compared for speed and accuracy. Each layer reads the ones below it:
//!
//! 1. **Classes** (`classes`): each 64-byte block's bytes classified on `std::simd`'s interface (`rerail-simd`), a
//!    mask a class, a bit a byte; the bytes a backslash escapes by simdjson's odd-run trick, one carry bit from
//!    the block before.
//! 2. **Regions** (`Text::regions`): comments and strings resolved in order. Each step takes the lowest opener
//!    left in code (a `#`, or a quote no backslash escapes) and finds its terminator by one mask search: the line's
//!    end, the quote's own mask, or for a triple the quote's mask and its two next bytes'. An f-string's
//!    interiors are code read by the same steps, their bracket depth by popcounts between the steps. Out comes
//!    `code`. An f-string inside an interior is refused, to tree-sitter.
//! 3. **Depth** (`Text::depth`): the bracket depth after each byte of code, by the block's brackets' bits, with the
//!    depth carried in (clamped at 0, as the lean lexer's counter is); out comes `depth0`. A block with no bracket
//!    is one test. A depth past 31 is refused (a rule).
//! 4. **Tokens** (`Walk`): the lean lexer's tokens and layout read from the masks: an item's start the next byte
//!    past the blanks, a name's end its run's, a string's stops one mask of its own kind, and whether a line end
//!    is inside brackets one bit of `depth0`, not a count.

use crate::lex::{self, Extra, Lexed, MAX_NESTING, Quote, Refusal, T, Tok};
use rerail_simd::prelude::*;
use rerail_simd::Simd;

type V = Simd<u8, 64>;

/// A block's masks: its byte classes (stratum 1), then what the strata above make.
#[derive(Clone, Copy, Default)]
struct B {
    /// `[A-Za-z0-9_]`.
    word: u64,
    /// `' '`: an indent's run.
    space: u64,
    /// `' '`, `'\t'`.
    sptab: u64,
    /// `' '`, `'\t'`, `'\f'`, `'\n'`: the blanks between items (a `'\r'` is read by its `'\n'`).
    skip: u64,
    /// `'\n'`, `'\r'`: a comment's end.
    eol: u64,
    lf: u64,
    /// A `'\n'` no backslash escapes (nor a `'\r'` an escaped one): a one-line string's refusal.
    bare_lf: u64,
    sq: u64,
    dq: u64,
    bt: u64,
    bs: u64,
    /// The bytes an odd run of backslashes escapes.
    esc: u64,
    hash: u64,
    /// `(`, `[`, `{`.
    ob: u64,
    /// `)`, `]`, `}`.
    cb: u64,
    /// Stratum 2: not in a comment or a string (an f-string's interiors are in its string).
    code: u64,
    /// Stratum 3: the bracket depth after this byte is 0.
    depth0: u64,
    /// Stratum 2: a string's first byte (its prefix's, or its quote's), in code or in an interior.
    sstart: u64,
}

/// A quote's mask.
#[inline(always)]
fn quotes(b: &B, q: u8) -> u64 {
    match q {
        b'\'' => b.sq,
        b'"' => b.dq,
        _ => b.bt,
    }
}

/// simdjson's escapes: the bytes after an odd run of backslashes. `prev` is whether the block's first byte is
/// escaped, and becomes the next block's.
#[inline(always)]
fn escaped(bs: u64, prev: &mut u64) -> u64 {
    const EVEN: u64 = 0x5555_5555_5555_5555;
    let bs = bs & !*prev;
    let follows = (bs << 1) | *prev;
    let odd_starts = bs & !EVEN & !follows;
    let (on_even, overflow) = odd_starts.overflowing_add(bs);
    *prev = u64::from(overflow);
    (EVEN ^ (on_even << 1)) & follows
}

/// Stratum 1 of one block.
#[inline(always)]
fn block(v: V) -> B {
    let s = V::splat;
    let eq = |x: u8| v.simd_eq(s(x));
    // A letter is `b | 0x20` in `a..=z`; a digit `b - '0'` below 10 (wrapping).
    let alpha = ((v | s(0x20)) - s(b'a')).simd_lt(s(26));
    let digit = (v - s(b'0')).simd_lt(s(10));
    let word = (alpha | digit | eq(b'_')).to_bitmask();
    let (space, lf) = (eq(b' ').to_bitmask(), eq(b'\n').to_bitmask());
    // Tabs, form feeds and `'\r'` are rare: one mask, and each its own only in a block that has one.
    let rare = (eq(b'\t') | eq(0x0c) | eq(b'\r')).to_bitmask();
    let (tab, ff, cr) = if rare == 0 { (0, 0, 0) } else { (eq(b'\t').to_bitmask(), eq(0x0c).to_bitmask(), eq(b'\r').to_bitmask()) };
    B {
        word,
        space,
        sptab: space | tab,
        skip: space | tab | ff | lf,
        eol: lf | cr,
        lf,
        bare_lf: cr,
        sq: eq(b'\'').to_bitmask(),
        dq: eq(b'"').to_bitmask(),
        bt: eq(b'`').to_bitmask(),
        bs: eq(b'\\').to_bitmask(),
        esc: 0,
        hash: eq(b'#').to_bitmask(),
        ob: (eq(b'(') | eq(b'[') | eq(b'{')).to_bitmask(),
        cb: (eq(b')') | eq(b']') | eq(b'}')).to_bitmask(),
        code: !0,
        depth0: 0,
        sstart: 0,
    }
}

/// Block `bi`'s bytes that are one of `xs`: made from its bytes where an f-string asks (its braces, an interior's
/// `:` and `!`), not for every block.
#[inline(always)]
fn bytes_of<const K: usize>(src: &[u8], bi: usize, xs: [u8; K]) -> u64 {
    let at = bi * 64;
    let v = match src.get(at..at + 64) {
        Some(b) => V::from_slice(b),
        None => {
            let mut b = [0u8; 64];
            let k = src.len().saturating_sub(at).min(64);
            b[..k].copy_from_slice(&src[at..at + k]);
            V::from_array(b)
        }
    };
    let mut m = v.simd_eq(V::splat(xs[0]));
    for &x in &xs[1..] {
        m |= v.simd_eq(V::splat(x));
    }
    m.to_bitmask()
}

/// Stratum 1 over `src`: a block's masks each, and one block past the end (all clear) for a look at the next
/// block's bytes.
#[inline(never)]
fn classes(src: &[u8]) -> Vec<B> {
    let n = src.len();
    let mut out = Vec::with_capacity(n.div_ceil(64) + 1);
    let (mut carry, mut cr_carry) = (0u64, 0u64);
    let mut at = 0;
    while at < n {
        let v = match src.get(at..at + 64) {
            Some(b) => V::from_slice(b),
            None => {
                let mut b = [0u8; 64];
                b[..n - at].copy_from_slice(&src[at..]);
                V::from_array(b)
            }
        };
        let mut b = block(v);
        b.esc = escaped(b.bs, &mut carry);
        // `bare_lf` held the block's `'\r'`s: a `'\n'` after an escaped one is escaped too.
        let esc_cr = b.bare_lf & b.esc;
        b.bare_lf = b.lf & !b.esc & !((esc_cr << 1) | cr_carry);
        cr_carry = esc_cr >> 63;
        out.push(b);
        at += 64;
    }
    out.push(B::default());
    out
}

/// One block's depths, by its brackets' bits (`up` and `down`, in code): the bits where the depth after the byte is
/// 0, the depth carried out, and whether any is past the bound. Between two brackets the depth is the one after the
/// first, so each bracket sets the bits of the run before it where that run's depth is 0, the depth clamped at 0 as
/// the lean lexer's counter is. A few operations a bracket: fewer than a prefix sum over the block's 64 lanes.
#[inline(always)]
fn depth_bits(up: u64, down: u64, carry: i8) -> (u64, i8, bool) {
    let (mut zero, mut d, mut from, mut deep) = (0u64, i32::from(carry), 0u32, false);
    let mut m = up | down;
    while m != 0 {
        let k = m.trailing_zeros();
        // Bytes `from..k` lie at depth `d`.
        let run = (!0u64 << from) & !(!0u64 << k);
        zero |= if d == 0 { run } else { 0 };
        d = (d + if up >> k & 1 == 1 { 1 } else { -1 }).max(0);
        deep |= d > i32::from(MAX_DEPTH);
        from = k;
        m &= m - 1;
    }
    zero |= if d == 0 { !0u64 << from } else { 0 };
    (zero, d as i8, deep)
}

/// The bracket depth's bound: a file past it is refused (decision 0698, item 93).
const MAX_DEPTH: i8 = 31;

/// The text and its blocks' masks.
struct Text<'s> {
    src: &'s [u8],
    b: Vec<B>,
}

#[inline(always)]
fn refuse(at: usize) -> Refusal {
    Refusal::Lex(at as u32)
}

/// The blocks' masks, read: what the walk (stratum 4) reads, borrowed, so that nothing it writes is taken to change
/// them.
#[derive(Clone, Copy)]
struct Masks<'a> {
    src: &'a [u8],
    /// The text's blocks and one past its end.
    b: &'a [B],
}

impl<'a> Masks<'a> {
    #[inline(always)]
    fn at(&self, i: usize) -> u8 {
        self.src.get(i).copied().unwrap_or(0)
    }

    /// Block `bi`, at most the one past the text's end.
    #[inline(always)]
    fn blk(&self, bi: usize) -> &'a B {
        debug_assert!(bi < self.b.len());
        // SAFETY: `classes` makes a block for each 64 bytes and one more, and the reads ask for a block of a byte
        // below the text's length, or the one after such a block.
        unsafe { self.b.get_unchecked(bi) }
    }

    /// Whether byte `i`'s bit is set in a mask (`i` at most the text's length).
    #[inline(always)]
    fn bit(&self, i: usize, f: impl Fn(&B) -> u64) -> bool {
        f(self.blk(i >> 6)) >> (i & 63) & 1 == 1
    }

    /// The first byte from `at` whose bit is set in a mask.
    #[inline(always)]
    fn next(&self, at: usize, f: impl Fn(&B) -> u64) -> usize {
        let n = self.src.len();
        if at >= n {
            return n;
        }
        let mut bi = at >> 6;
        let mut m = f(self.blk(bi)) & (!0u64 << (at & 63));
        loop {
            if m != 0 {
                return (bi * 64 + m.trailing_zeros() as usize).min(n);
            }
            bi += 1;
            if bi * 64 >= n {
                return n;
            }
            m = f(self.blk(bi));
        }
    }

    /// The first byte from `at` whose bit is set in the mask `f` makes of a block's index, the block and the block
    /// after it.
    #[inline(always)]
    fn next2_in(&self, at: usize, f: impl Fn(usize, &B, &B) -> u64) -> usize {
        let n = self.src.len();
        if at >= n {
            return n;
        }
        let mut bi = at >> 6;
        let mut m = f(bi, self.blk(bi), self.blk(bi + 1)) & (!0u64 << (at & 63));
        loop {
            if m != 0 {
                return (bi * 64 + m.trailing_zeros() as usize).min(n);
            }
            bi += 1;
            if bi * 64 >= n {
                return n;
            }
            m = f(bi, self.blk(bi), self.blk(bi + 1));
        }
    }

    /// The first byte from `at` whose bit is set in the mask `f` makes of a block and its index.
    #[inline(always)]
    fn next_in(&self, at: usize, f: impl Fn(usize, &B) -> u64) -> usize {
        let n = self.src.len();
        if at >= n {
            return n;
        }
        let mut bi = at >> 6;
        let mut m = f(bi, self.blk(bi)) & (!0u64 << (at & 63));
        loop {
            if m != 0 {
                return (bi * 64 + m.trailing_zeros() as usize).min(n);
            }
            bi += 1;
            if bi * 64 >= n {
                return n;
            }
            m = f(bi, self.blk(bi));
        }
    }

    /// The first byte from `at` whose bit is clear in a mask: a run's end.
    #[inline(always)]
    fn run_end(&self, at: usize, f: impl Fn(&B) -> u64) -> usize {
        self.next(at, |b| !f(b))
    }
}

impl<'s> Text<'s> {
    #[inline(always)]
    fn m(&self) -> Masks<'_> {
        Masks { src: self.src, b: &self.b }
    }

    #[inline(always)]
    fn at(&self, i: usize) -> u8 {
        self.m().at(i)
    }

    #[inline(always)]
    fn bit(&self, i: usize, f: impl Fn(&B) -> u64) -> bool {
        self.m().bit(i, f)
    }

    #[inline(always)]
    fn next(&self, at: usize, f: impl Fn(&B) -> u64) -> usize {
        self.m().next(at, f)
    }

    /// How many bytes of `[a, b)` are set in a mask.
    #[inline(always)]
    fn count(&self, a: usize, b: usize, f: impl Fn(&B) -> u64) -> i32 {
        if a >= b {
            return 0;
        }
        let (ba, bb) = (a >> 6, (b - 1) >> 6);
        let lo = !0u64 << (a & 63);
        let hi = !0u64 >> (63 - ((b - 1) & 63));
        if ba == bb {
            return (f(&self.b[ba]) & lo & hi).count_ones() as i32;
        }
        let mut c = (f(&self.b[ba]) & lo).count_ones() + (f(&self.b[bb]) & hi).count_ones();
        for x in &self.b[ba + 1..bb] {
            c += f(x).count_ones();
        }
        c as i32
    }

    /// A string starts at `i`.
    #[inline(always)]
    fn mark(&mut self, i: usize) {
        self.b[i >> 6].sstart |= 1 << (i & 63);
    }

    /// Bytes `[a, b)` are not code.
    fn fill(&mut self, a: usize, b: usize) {
        if a >= b {
            return;
        }
        let (ba, bb) = (a >> 6, (b - 1) >> 6);
        let lo = !0u64 << (a & 63);
        let hi = !0u64 >> (63 - ((b - 1) & 63));
        if ba == bb {
            self.b[ba].code &= !(lo & hi);
            return;
        }
        self.b[ba].code &= !lo;
        self.b[bb].code &= !hi;
        for x in &mut self.b[ba + 1..bb] {
            x.code = 0;
        }
    }

    /// Stratum 2: each comment and string in turn, from the lowest opener left in code.
    #[inline(never)]
    fn regions(&mut self) -> Result<(), Refusal> {
        let n = self.src.len();
        let mut pos = 0;
        loop {
            let o = self.next(pos, |b| b.hash | ((b.sq | b.dq | b.bt) & !b.esc));
            if o >= n {
                return Ok(());
            }
            if self.src[o] == b'#' {
                let e = self.next(o, |b| b.eol);
                self.fill(o, e);
                pos = e;
                continue;
            }
            let (start, format) = self.prefix(o);
            self.mark(start);
            let end = self.string_end(o, format, 0)?;
            self.fill(start, end);
            pos = end;
        }
    }

    /// Where the string whose quote is at `o` starts (its prefix's first letter), and whether it is a format
    /// string: the run of prefix letters just below the quote (a few bytes read at the quote), where no word byte is
    /// below it.
    #[inline(always)]
    fn prefix(&self, o: usize) -> (usize, bool) {
        let pfx = |c: u8| matches!(c | 0x20, b'r' | b'b' | b'f' | b't' | b'u');
        let mut j = o;
        while j > 0 && pfx(self.src[j - 1]) {
            j -= 1;
        }
        if j == o || (j > 0 && (crate::block::is_word(self.src[j - 1]) || self.src[j - 1] >= 0x80)) {
            return (o, false);
        }
        (j, self.src[j..o].iter().any(|&c| matches!(c | 0x20, b'f' | b't')))
    }

    /// The end of the string whose quote is at `o`: its terminator found by one mask search a step. A format
    /// string's text stops at a brace too, its interiors read as code (`interior_end`).
    fn string_end(&mut self, o: usize, format: bool, nest: u32) -> Result<usize, Refusal> {
        let n = self.src.len();
        let q = self.src[o];
        let triple = self.at(o + 1) == q && self.at(o + 2) == q;
        let src = self.src;
        let braces = |bi: usize| if format { bytes_of(src, bi, *b"{}") } else { 0 };
        let mut j = o + if triple { 3 } else { 1 };
        loop {
            let c = if triple {
                // A quote no backslash escapes, and the two bytes after it its quote.
                self.m().next2_in(j, |bi, b, b1| {
                    let (qm, q1) = (quotes(b, q), quotes(b1, q));
                    (qm & !b.esc & (qm >> 1 | q1 << 63) & (qm >> 2 | q1 << 62)) | braces(bi)
                })
            } else {
                self.m().next_in(j, |bi, b| (quotes(b, q) & !b.esc) | b.bare_lf | braces(bi))
            };
            if c >= n {
                return Err(refuse(c));
            }
            match self.src[c] {
                b'\n' => return Err(refuse(c)),
                b'{' => {
                    if self.at(c + 1) == b'{' {
                        j = c + 2;
                        continue;
                    }
                    // `\N{...}`: an escape's name, not an interpolation (not read here).
                    if c >= 2 && self.src[c - 1] == b'N' && self.src[c - 2] == b'\\' && !self.bit(c - 2, |b| b.esc) {
                        return Err(refuse(c));
                    }
                    j = self.interior_end(c + 1, nest + 1)?;
                }
                b'}' => {
                    if self.at(c + 1) != b'}' {
                        return Err(refuse(c));
                    }
                    j = c + 2;
                }
                _ => return Ok(c + if triple { 3 } else { 1 }),
            }
        }
    }

    /// The end of an interpolation whose code starts at `i`: the first `}` at its own depth 0 outside its own
    /// strings, or a format specifier's end after a `:` at depth 0. The depth is the brackets' popcounts between
    /// the stops.
    fn interior_end(&mut self, mut i: usize, nest: u32) -> Result<usize, Refusal> {
        if nest > MAX_NESTING {
            return Err(refuse(i));
        }
        let n = self.src.len();
        let mut depth = 0i32;
        loop {
            let src = self.src;
            let c = self.m().next_in(i, |bi, b| b.sq | b.dq | b.bt | b.hash | b.bs | bytes_of(src, bi, *b"}:!"));
            if c >= n {
                return Err(refuse(c));
            }
            depth += self.count(i, c, |b| b.ob) - self.count(i, c, |b| b.cb);
            i = c + 1;
            match self.src[c] {
                b'#' | b'\\' => return Err(refuse(c)),
                b'}' => {
                    if depth == 0 {
                        return Ok(c + 1);
                    }
                    depth -= 1;
                }
                b':' if depth == 0 => return self.spec_end(c + 1, nest),
                b':' | b'!' => {}
                _ => {
                    // A string in the interior: a nested f-string is refused (to tree-sitter).
                    let (start, format) = self.prefix(c);
                    if format {
                        return Err(refuse(c));
                    }
                    self.mark(start);
                    i = self.string_end(c, false, nest)?;
                }
            }
        }
    }

    /// The end of a format specifier from `i` (after its `:`): its `}`, past its own interpolations.
    fn spec_end(&mut self, mut i: usize, nest: u32) -> Result<usize, Refusal> {
        let n = self.src.len();
        loop {
            let src = self.src;
            let c = self.m().next_in(i, |bi, b| b.lf | bytes_of(src, bi, *b"{}"));
            if c >= n || self.src[c] == b'\n' {
                return Err(refuse(c));
            }
            if self.src[c] == b'}' {
                return Ok(c + 1);
            }
            if self.at(c + 1) == b'{' {
                return Err(refuse(c));
            }
            i = self.interior_end(c + 1, nest + 1)?;
        }
    }

    /// Stratum 3: each block's `depth0`.
    #[inline(never)]
    fn depth(&mut self) -> Result<(), Refusal> {
        let blocks = self.src.len().div_ceil(64);
        let mut carry = 0i8;
        for (bi, b) in self.b[..blocks].iter_mut().enumerate() {
            let (up, down) = (b.ob & b.code, b.cb & b.code);
            if up | down == 0 {
                b.depth0 = if carry == 0 { !0 } else { 0 };
                continue;
            }
            let (zero, out, deep) = depth_bits(up, down, carry);
            if deep {
                return Err(refuse(bi * 64));
            }
            b.depth0 = zero;
            carry = out;
        }
        Ok(())
    }
}

/// A byte's class for the token reader's dispatch.
const C_OP: u8 = 0;
const C_NAME: u8 = 1;
const C_DIGIT: u8 = 2;
const C_QUOTE: u8 = 3;
const C_DOT: u8 = 4;
const C_HIGH: u8 = 5;
static CLASS: [u8; 256] = {
    let mut t = [C_OP; 256];
    let mut c = 0;
    while c < 256 {
        let b = c as u8;
        t[c] = match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => C_NAME,
            b'0'..=b'9' => C_DIGIT,
            b'\'' | b'"' | b'`' => C_QUOTE,
            b'.' => C_DOT,
            0x80.. => C_HIGH,
            _ => C_OP,
        };
        c += 1;
    }
    t
};

/// The bytes that start a token at once after spaces and tabs: not a line's end, a comment, a continuation, a form
/// feed, or a byte past ASCII (an odd blank or a name's character, read by the lean lexer's path).
static FAST: [bool; 256] = {
    let mut t = [false; 256];
    let mut c = 0;
    while c < 128 {
        t[c] = !matches!(c as u8, b'\n' | b'\r' | b'#' | b'\\' | 0x0c | b' ' | b'\t');
        c += 1;
    }
    t
};

/// A table of a token kind a byte below 128, of a `const fn` of the byte.
macro_rules! table {
    ($f:ident) => {{
        let mut t = [None; 128];
        let mut c = 0;
        while c < 128 {
            t[c] = $f(c as u8);
            c += 1;
        }
        t
    }};
}

/// A byte's operator alone, the operator it makes with a `=` after it, and with itself after it.
const fn op1(c: u8) -> Option<T> {
    Some(match c {
        b'(' => T::LParen,
        b')' => T::RParen,
        b'[' => T::LBracket,
        b']' => T::RBracket,
        b'{' => T::LBrace,
        b'}' => T::RBrace,
        b',' => T::Comma,
        b':' => T::Colon,
        b';' => T::Semi,
        b'.' => T::Dot,
        b'=' => T::Eq,
        b'@' => T::At,
        b'+' => T::Plus,
        b'-' => T::Minus,
        b'*' => T::Star,
        b'/' => T::Slash,
        b'%' => T::Percent,
        b'&' => T::Amp,
        b'|' => T::Pipe,
        b'^' => T::Caret,
        b'~' => T::Tilde,
        b'<' => T::Lt,
        b'>' => T::Gt,
        _ => return None,
    })
}
static OP1: [Option<T>; 128] = table!(op1);
const fn op_eq(c: u8) -> Option<T> {
    Some(match c {
        b'<' => T::Le,
        b'>' => T::Ge,
        b'=' => T::EqEq,
        b'!' => T::Ne,
        b':' => T::Walrus,
        b'+' => T::PlusEq,
        b'-' => T::MinusEq,
        b'*' => T::StarEq,
        b'/' => T::SlashEq,
        b'@' => T::AtEq,
        b'%' => T::PercentEq,
        b'&' => T::AmpEq,
        b'^' => T::CaretEq,
        b'|' => T::PipeEq,
        _ => return None,
    })
}
static OP_EQ: [Option<T>; 128] = table!(op_eq);
const fn op_twice(c: u8) -> Option<T> {
    Some(match c {
        b'*' => T::Pow,
        b'/' => T::DSlash,
        b'>' => T::RShift,
        b'<' => T::LShift,
        _ => return None,
    })
}
static OP_TWICE: [Option<T>; 128] = table!(op_twice);
const fn op_twice_eq(c: u8) -> Option<T> {
    Some(match c {
        b'*' => T::PowEq,
        b'/' => T::DSlashEq,
        b'>' => T::RShiftEq,
        b'<' => T::LShiftEq,
        _ => return None,
    })
}
static OP_TWICE_EQ: [Option<T>; 128] = table!(op_twice_eq);

/// The operator at bytes `a`, `b`, `c` and its length, by the tables (the lean lexer's `operator_at`, the longest).
#[inline(always)]
fn operator(a: u8, b: u8, c: u8) -> Option<(T, usize)> {
    if a >= 0x80 {
        return None;
    }
    let ai = usize::from(a);
    if b == b'=' {
        if let Some(t) = OP_EQ[ai] {
            return Some((t, 2));
        }
    } else if b == a {
        if let Some(t) = OP_TWICE[ai] {
            return Some(if c == b'=' { (OP_TWICE_EQ[ai].expect("an operator and its `=`"), 3) } else { (t, 2) });
        }
        if a == b'.' && c == b'.' {
            return Some((T::Ellipsis, 3));
        }
    } else if b == b'>' && (a == b'-' || a == b'<') {
        return Some((if a == b'-' { T::Arrow } else { T::LtGt }, 2));
    }
    OP1[ai].map(|t| (t, 1))
}

/// Stratum 4: the walk that writes the tokens.
struct Walk<'s> {
    t: Masks<'s>,
    toks: Vec<Tok>,
    extras: Vec<Extra>,
    /// Where the last token or extra ended.
    p: usize,
    /// Interpolations nested.
    depth: u32,
    indents: Vec<u32>,
    /// As the lean lexer's (the scanner's flag).
    inside_interpolated_string: bool,
}

/// `src` lexed by the strata: the lean lexer's `lex`.
pub(crate) fn lex(src: &[u8]) -> Result<Lexed, Refusal> {
    let mut t = Text { src, b: classes(src) };
    t.regions()?;
    t.depth()?;
    let mut w = Walk { t: t.m(), toks: Vec::with_capacity(src.len() / 3 + 16), extras: Vec::new(), p: 0, depth: 0, indents: vec![0], inside_interpolated_string: false };
    w.module()?;
    Ok(Lexed { toks: w.toks, extras: w.extras })
}

impl<'s> Walk<'s> {
    #[inline(always)]
    fn at(&self, i: usize) -> u8 {
        self.t.at(i)
    }

    #[inline(always)]
    fn push(&mut self, kind: T, start: usize, end: usize) {
        self.toks.push(Tok { kind, soft: 0, start: start as u32, end: end as u32 });
        self.p = end;
    }

    /// Whether the bracket depth after byte `p - 1` is above 0 (stratum 3's bit, not a count).
    #[inline(always)]
    fn in_brackets(&self, p: usize) -> bool {
        p > 0 && !self.t.bit(p - 1, |b| b.depth0)
    }

    /// The scanner's look from `p` (the lean lexer's `scan`).
    fn scan(&self, p: usize) -> Result<Option<(u32, i64, usize)>, Refusal> {
        let src = self.t.src;
        let (mut i, mut eol, mut indent, mut first_comment) = (p, false, 0u32, -1i64);
        loop {
            match self.at(i) {
                b'\n' => {
                    eol = true;
                    indent = 0;
                    i += 1;
                }
                b' ' => {
                    let end = self.t.run_end(i, |b| b.space);
                    indent += (end - i) as u32;
                    i = end;
                }
                b'\r' => {
                    if self.at(i + 1) != b'\n' {
                        return Err(refuse(i));
                    }
                    indent = 0;
                    i += 1;
                }
                0x0c => {
                    indent = 0;
                    i += 1;
                }
                b'\t' => {
                    indent += 8;
                    i += 1;
                }
                b'#' => {
                    if !eol {
                        return Ok(None);
                    }
                    if first_comment < 0 {
                        first_comment = i64::from(indent);
                    }
                    i = self.t.next(i, |b| b.eol);
                    if i < src.len() {
                        i += 1;
                    }
                    indent = 0;
                }
                b'\\' => {
                    i += 1;
                    if self.at(i) == b'\r' {
                        i += 1;
                    }
                    if self.at(i) == b'\n' || i >= src.len() {
                        i += 1;
                    } else {
                        return Ok(None);
                    }
                }
                _ if i >= src.len() => return Ok(Some((0, first_comment, i))),
                _ => break,
            }
        }
        Ok(if eol { Some((indent, first_comment, i)) } else { None })
    }

    /// Blanks and line ends from `i`: the next item's start.
    #[inline(always)]
    fn blanks(&self, mut i: usize) -> Result<usize, Refusal> {
        loop {
            i = self.t.run_end(i, |b| b.skip);
            match self.at(i) {
                b'\r' => {
                    if self.at(i + 1) != b'\n' {
                        return Err(refuse(i));
                    }
                    i += 2;
                }
                c if c >= 0x80 && lex::odd_blank_at(self.t.src, i) > 0 => i += 3,
                _ => return Ok(i),
            }
        }
    }

    /// The whole file (the lean lexer's `module`, its bracket count read from `depth0`).
    #[inline(never)]
    fn module(&mut self) -> Result<(), Refusal> {
        let n = self.t.src.len();
        let (mut open, mut header) = (false, false);
        let src = self.t.src;
        loop {
            // The first byte past the spaces and tabs (one mask search). In the usual case it starts a token, and
            // the scanner gives nothing: the tokens are read at once, a run of them where it is code.
            let k = self.t.run_end(self.p, |b| b.sptab);
            if k < n && FAST[usize::from(src[k])] {
                let (kind, at) = if self.t.bit(k, |b| b.code) { self.run(k)? } else { (self.token(k)?, k) };
                open = true;
                header = kind == T::Colon && !self.in_brackets(at + 1);
                continue;
            }
            let line_end = k >= n || matches!(self.t.src[k], b'\n' | b'\r' | b'#' | b'\\' | 0x0c);
            if line_end {
                if self.in_brackets(self.p) {
                    if self.dedent_in_brackets(self.p)? {
                        return Err(refuse(self.p));
                    }
                } else {
                    // The layout tokens take no bytes, so the scanner's look from `p` is the same for each: it is
                    // taken once.
                    if let Some((indent, first_comment, _)) = self.scan(self.p)? {
                        loop {
                            let current = *self.indents.last().expect("an indent");
                            let p = self.p;
                            if header && indent > current {
                                self.indents.push(indent);
                                self.push(T::Indent, p, p);
                                open = false;
                                header = false;
                                break;
                            } else if open {
                                self.push(T::Newline, p, p);
                                open = false;
                                header = false;
                            } else if indent < current && first_comment < i64::from(current) {
                                self.indents.pop();
                                self.push(T::Dedent, p, p);
                            } else {
                                break;
                            }
                        }
                    }
                }
            }
            let i = self.blanks(k)?;
            if i >= n {
                self.push(T::Eof, n, n);
                return Ok(());
            }
            match self.t.src[i] {
                b'#' => {
                    let end = self.t.next(i, |b| b.eol);
                    self.extras.push(Extra { start: i as u32, end: end as u32, comment: true });
                    self.p = end;
                    continue;
                }
                b'\\' => {
                    let mut j = i + 1;
                    if self.at(j) == b'\r' {
                        j += 1;
                    }
                    if self.at(j) == b'\n' {
                        j += 1;
                    } else if j < n {
                        return Err(refuse(i));
                    }
                    let k = self.t.run_end(j, |b| b.sptab | (b.skip & !b.lf));
                    if lex::string_prefix_at(self.t.src, k).is_none() {
                        self.extras.push(Extra { start: i as u32, end: j as u32, comment: false });
                    }
                    self.p = j;
                    continue;
                }
                _ => {}
            }
            let (kind, at) = if self.t.bit(i, |b| b.code) && FAST[usize::from(src[i])] { self.run(i)? } else { (self.token(i)?, i) };
            open = true;
            header = kind == T::Colon && !self.in_brackets(at + 1);
        }
    }

    /// A run of tokens from `k` (a token's start, in code), rr's run: the block's token starts one mask, each taken
    /// by its lowest bit, a name's end its run of word bits. The run ends at the first byte of the block that is not
    /// code (a string's or a comment's start: stratum 2's), a line's end, a form feed or a `\`, so no layout comes
    /// between its tokens and no name of it is a string's prefix. The last token's kind and start.
    #[inline(always)]
    fn run(&mut self, k: usize) -> Result<(T, usize), Refusal> {
        let src = self.t.src;
        let n = src.len();
        let base = k & !63;
        let b = self.t.blk(k >> 6);
        let q0 = (k & 63) as u32;
        let valid = if n - base >= 64 { !0u64 } else { (1u64 << (n - base)) - 1 };
        let ff = b.skip & !b.sptab & !b.lf;
        let stop = (!b.code | b.eol | ff | b.bs | !valid) & (!0u64 << q0);
        let before_stop = if stop == 0 { !0u64 } else { stop.isolate_lowest_one() - 1 };
        let mut starts = ((!b.skip & !(b.word & (b.word << 1)) & (!0u64 << q0)) | (1u64 << q0)) & before_stop;
        let mut last = (T::Eof, k);
        while starts != 0 {
            let q = starts.trailing_zeros();
            let pos = base + q as usize;
            let c = src[pos];
            let end = match CLASS[usize::from(c)] {
                C_NAME => {
                    let words = (!(b.word >> q)).trailing_zeros() as usize;
                    let mut end = pos + words;
                    // On past the block, or into a character past ASCII: the name's whole read.
                    if q as usize + words == 64 || self.at(end) >= 0x80 {
                        end = self.name_end(pos)?;
                    }
                    let (kind, soft) = lex::keyword(src, pos, end);
                    self.toks.push(Tok { kind, soft, start: pos as u32, end: end as u32 });
                    last = (kind, pos);
                    end
                }
                C_DIGIT => {
                    let (kind, end) = lex::number_at(src, pos);
                    self.toks.push(Tok { kind, soft: 0, start: pos as u32, end: end as u32 });
                    last = (kind, pos);
                    end
                }
                // A byte past ASCII (an odd blank, a name's character) or a quote: the walk's own path.
                C_HIGH | C_QUOTE => break,
                cls => {
                    let (kind, end) = if cls == C_DOT && self.at(pos + 1).is_ascii_digit() {
                        lex::number_at(src, pos)
                    } else {
                        let (kind, len) = operator(c, self.at(pos + 1), self.at(pos + 2)).ok_or_else(|| refuse(pos))?;
                        (kind, pos + len)
                    };
                    self.toks.push(Tok { kind, soft: 0, start: pos as u32, end: end as u32 });
                    last = (kind, pos);
                    end
                }
            };
            self.p = end;
            let e = end - base;
            if e >= 64 {
                break;
            }
            starts &= !0u64 << e;
        }
        Ok(last)
    }

    /// The lean lexer's `dedent_in_brackets`.
    fn dedent_in_brackets(&self, p: usize) -> Result<bool, Refusal> {
        if self.inside_interpolated_string || self.closer_may_follow() {
            return Ok(false);
        }
        let current = i64::from(*self.indents.last().expect("an indent"));
        Ok(match self.scan(p)? {
            Some((indent, first_comment, next)) => i64::from(indent) < current && first_comment < current && !matches!(self.at(next), b'\'' | b'"' | b'`'),
            None => false,
        })
    }

    fn closer_may_follow(&self) -> bool {
        let n = self.toks.len();
        let kind = |k: usize| self.toks[k].kind;
        n > 0
            && match kind(n - 1) {
                T::Name | T::Int | T::Float | T::True | T::False | T::None | T::Ellipsis | T::StrEnd | T::RParen | T::RBracket | T::RBrace | T::LParen | T::LBrace | T::Comma => true,
                T::LBracket => n < 2 || !matches!(kind(n - 2), T::Name | T::StrEnd | T::RParen | T::RBracket | T::RBrace),
                _ => false,
            }
    }

    /// The token at `i`, and those of a string it starts: dispatched on the byte's class (one table load).
    #[inline(always)]
    fn token(&mut self, i: usize) -> Result<T, Refusal> {
        let src = self.t.src;
        let c = src[i];
        match CLASS[usize::from(c)] {
            C_NAME | C_HIGH => {
                // A string's prefix only where stratum 2 found a string's start: a name is read at once.
                if self.t.bit(i, |b| b.sstart)
                    && let Some((prefix, q)) = lex::string_prefix_at(src, i)
                {
                    self.string(i, prefix, q)?;
                    return Ok(T::StrEnd);
                }
                let end = self.name_end(i)?;
                let (kind, soft) = lex::keyword(src, i, end);
                self.toks.push(Tok { kind, soft, start: i as u32, end: end as u32 });
                self.p = end;
                Ok(kind)
            }
            C_QUOTE => {
                let (prefix, q) = lex::string_prefix_at(src, i).expect("a quote");
                self.string(i, prefix, q)?;
                Ok(T::StrEnd)
            }
            C_DIGIT => self.number(i),
            C_DOT if self.at(i + 1).is_ascii_digit() => self.number(i),
            _ => {
                let (kind, len) = operator(c, self.at(i + 1), self.at(i + 2)).ok_or_else(|| refuse(i))?;
                self.push(kind, i, i + len);
                Ok(kind)
            }
        }
    }

    fn number(&mut self, i: usize) -> Result<T, Refusal> {
        let (kind, end) = lex::number_at(self.t.src, i);
        self.push(kind, i, end);
        Ok(kind)
    }

    /// A name's end: its run of word bytes; a character past ASCII by its XID class.
    fn name_end(&self, i: usize) -> Result<usize, Refusal> {
        let src = self.t.src;
        let mut j = i;
        let mut first = true;
        loop {
            j = self.t.run_end(j, |b| b.word);
            if self.at(j) < 0x80 || lex::odd_blank_at(src, j) > 0 {
                break;
            }
            let rest = std::str::from_utf8(&src[j..(j + 4).min(src.len())]).or_else(|e| std::str::from_utf8(&src[j..j + e.valid_up_to()])).map_err(|_| refuse(j))?;
            let Some(ch) = rest.chars().next() else { return Err(refuse(j)) };
            let ok = if first && j == i { unicode_ident::is_xid_start(ch) } else { unicode_ident::is_xid_continue(ch) };
            if !ok {
                if j == i {
                    return Err(refuse(j));
                }
                break;
            }
            first = false;
            j += ch.len_utf8();
        }
        Ok(j)
    }

    /// A string from `i` (the lean lexer's `string`): its text's stops one mask of its own quote, `\`, `\n`, and in
    /// a format string `{` and `}`.
    fn string(&mut self, i: usize, prefix: usize, q: Quote) -> Result<(), Refusal> {
        let src = self.t.src;
        let open = prefix + if q.triple { 3 } else { 1 };
        self.push(T::StrStart, i, i + open);
        self.inside_interpolated_string = q.format;
        let mut j = i + open;
        let mut text = j;
        let mut content = false;
        let (quote, format) = (q.quote, q.format);
        loop {
            let s = self.t.next_in(j, |bi, b| quotes(b, quote) | b.bs | b.lf | if format { bytes_of(src, bi, *b"{}") } else { 0 });
            if s >= src.len() {
                return Err(refuse(s));
            }
            content |= s > j;
            let c = src[s];
            if c == q.quote {
                if !q.triple || (self.at(s + 1) == q.quote && self.at(s + 2) == q.quote) {
                    let close = if q.triple { 3 } else { 1 };
                    if content {
                        self.text(text, s);
                        self.push(T::StrEnd, s, s + close);
                    } else {
                        self.push(T::StrEnd, text, s + close);
                    }
                    self.inside_interpolated_string = false;
                    return Ok(());
                }
                j = s + 1;
                content = true;
            } else if c == b'\n' {
                if !q.triple {
                    return Err(refuse(s));
                }
                j = s + 1;
                content = true;
            } else if c == b'\\' {
                let next = self.at(s + 1);
                if q.raw {
                    j = s + 1;
                    if next == q.quote || next == b'\\' || next == b'\n' {
                        j += 1;
                    } else if next == b'\r' {
                        j += if self.at(s + 2) == b'\n' { 2 } else { 1 };
                    }
                    continue;
                }
                if q.bytes && matches!(next, b'N' | b'u' | b'U') {
                    j = s + 2;
                    content = true;
                    continue;
                }
                self.text(text, s);
                self.backslash(s);
                j = self.p;
                text = j;
                content = false;
            } else {
                if content {
                    self.text(text, s);
                } else {
                    let mut k = text;
                    while k < s {
                        self.backslash(k);
                        k = self.p;
                    }
                }
                if c == b'{' && self.at(s + 1) != b'{' {
                    j = self.interpolation(s)?;
                } else {
                    if self.at(s + 1) != c {
                        return Err(refuse(s));
                    }
                    self.push(T::EscInterp, s, s + 2);
                    j = s + 2;
                }
                text = j;
                content = false;
            }
        }
    }

    fn backslash(&mut self, s: usize) {
        match lex::escape_at(self.t.src, s) {
            Some(end) => self.push(T::Escape, s, end),
            None => self.push(T::Backslash, s, s + 1),
        }
    }

    #[inline(always)]
    fn text(&mut self, from: usize, to: usize) {
        if to > from {
            self.push(T::Text, from, to);
        }
    }

    /// An interpolation from its `{` at `s` (the lean lexer's `interpolation`).
    fn interpolation(&mut self, s: usize) -> Result<usize, Refusal> {
        self.depth += 1;
        if self.depth > MAX_NESTING {
            return Err(refuse(s));
        }
        self.push(T::IOpen, s, s + 1);
        let src = self.t.src;
        let mut brackets = 0u32;
        let mut i = s + 1;
        loop {
            if self.dedent_in_brackets(i)? {
                return Err(refuse(i));
            }
            loop {
                i = self.t.run_end(i, |b| b.skip);
                if self.at(i) == b'\r' && self.at(i + 1) == b'\n' {
                    i += 2;
                } else {
                    break;
                }
            }
            if i >= src.len() {
                return Err(refuse(i));
            }
            let c = src[i];
            if brackets == 0 {
                if c == b'}' {
                    self.push(T::IClose, i, i + 1);
                    self.depth -= 1;
                    return Ok(i + 1);
                }
                if c == b'!' && self.at(i + 1) != b'=' {
                    if !self.at(i + 1).is_ascii_lowercase() {
                        return Err(refuse(i));
                    }
                    self.push(T::Conversion, i, i + 2);
                    i += 2;
                    continue;
                }
                if c == b':' {
                    self.push(T::Spec, i, i + 1);
                    let end = self.spec(i + 1)?;
                    self.push(T::IClose, end, end + 1);
                    self.depth -= 1;
                    return Ok(end + 1);
                }
            }
            if c == b'#' || c == b'\\' {
                return Err(refuse(i));
            }
            let kind = self.token(i)?;
            match kind {
                T::LParen | T::LBracket | T::LBrace => brackets += 1,
                T::RParen | T::RBracket | T::RBrace => {
                    if brackets == 0 {
                        return Err(refuse(i));
                    }
                    brackets -= 1;
                }
                _ => {}
            }
            i = self.p;
        }
    }

    /// A format specifier's text from `i` (the lean lexer's `spec`): its stops one mask, `{`, `}`, `\n`.
    fn spec(&mut self, mut i: usize) -> Result<usize, Refusal> {
        let src = self.t.src;
        loop {
            let s = self.t.next_in(i, |bi, b| b.lf | bytes_of(src, bi, *b"{}"));
            if s >= src.len() || src[s] == b'\n' {
                return Err(refuse(s));
            }
            if src[s] == b'}' {
                return Ok(s);
            }
            if self.at(s + 1) == b'{' {
                return Err(refuse(s));
            }
            i = self.interpolation(s)?;
        }
    }
}

/// How the lexer of strata's reading of a text compares with the lean lexer's.
#[derive(Debug, PartialEq, Eq)]
pub enum Check {
    /// The same tokens and extras.
    Same,
    /// Both refuse the text.
    BothRefuse,
    /// The lean lexer refuses it, the strata read it.
    LeanRefuses(u32),
    /// The strata refuse it (at this byte), the lean lexer reads it.
    StrataRefuses(u32),
    /// Both read it, to different tokens: the first that differs, as the lean lexer's text and the strata's.
    Differ(String, String),
}

/// The lexer of strata against the lean lexer on `src`.
pub fn check(src: &[u8]) -> Check {
    let at = |r: Refusal| match r {
        Refusal::Lex(at) | Refusal::Parse(at) => at,
        Refusal::TooLarge => u32::MAX,
    };
    match (lex::lex(src), lex(src)) {
        (Err(_), Err(_)) => Check::BothRefuse,
        (Err(r), Ok(_)) => Check::LeanRefuses(at(r)),
        (Ok(_), Err(r)) => Check::StrataRefuses(at(r)),
        (Ok(a), Ok(b)) => {
            let tok = |t: &Tok| (t.kind, t.soft, t.start, t.end);
            let ex = |e: &Extra| (e.start, e.end, e.comment);
            if a.toks.iter().map(tok).eq(b.toks.iter().map(tok)) && a.extras.iter().map(ex).eq(b.extras.iter().map(ex)) {
                return Check::Same;
            }
            let (ta, tb) = (a.text(src), b.text(src));
            let k = ta.lines().zip(tb.lines()).position(|(x, y)| x != y).unwrap_or(ta.lines().count().min(tb.lines().count()));
            let line = |t: &str| t.lines().skip(k.saturating_sub(2)).take(5).collect::<Vec<_>>().join("\n");
            Check::Differ(line(&ta), line(&tb))
        }
    }
}

/// The strata up to `upto` alone (1: classes, 2: regions, 3: depth, 4: the whole lexer), for a measurement of
/// each: a sum that keeps the work.
pub fn upto(src: &[u8], upto: u8) -> usize {
    let mut t = Text { src, b: classes(src) };
    if upto >= 2 && t.regions().is_err() {
        return 0;
    }
    if upto >= 3 && t.depth().is_err() {
        return 0;
    }
    if upto >= 4 {
        return token_count(src).unwrap_or(0);
    }
    t.b.iter().map(|b| (b.code ^ b.depth0 ^ b.word) as usize).fold(0, usize::wrapping_add)
}

/// How many tokens and extras the strata lex `src` to (the lexer alone, for a measurement).
pub fn token_count(src: &[u8]) -> Result<usize, Refusal> {
    let l = lex(src)?;
    Ok(l.toks.len() + l.extras.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each byte's escape, one at a time: a backslash no backslash escapes escapes the byte after it.
    fn escapes_scalar(src: &[u8]) -> Vec<bool> {
        let mut out = vec![false; src.len()];
        let mut next = false;
        for (i, &c) in src.iter().enumerate() {
            out[i] = next;
            next = c == b'\\' && !out[i];
        }
        out
    }

    #[test]
    fn escapes_are_the_odd_runs() {
        let mut x = 0x2545_f491_4f6c_dd1du64;
        for len in [1usize, 63, 64, 65, 130, 500] {
            for _ in 0..50 {
                let src: Vec<u8> = (0..len)
                    .map(|_| {
                        x ^= x << 13;
                        x ^= x >> 7;
                        x ^= x << 17;
                        b"\\\\\\a'"[(x >> 8) as usize % 5]
                    })
                    .collect();
                let b = classes(&src);
                let want = escapes_scalar(&src);
                for (i, &w) in want.iter().enumerate() {
                    assert_eq!(b[i >> 6].esc >> (i & 63) & 1 == 1, w, "byte {i} of {:?}", String::from_utf8_lossy(&src));
                }
            }
        }
    }

    #[test]
    fn depth_is_the_clamped_count() {
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        for _ in 0..200 {
            let src: Vec<u8> = (0..300)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    b"(([)]}{a  "[(x >> 8) as usize % 10]
                })
                .collect();
            let mut t = Text { src: &src, b: classes(&src) };
            let mut d = 0i32;
            let mut want = Vec::new();
            for &c in &src {
                d = match c {
                    b'(' | b'[' | b'{' => d + 1,
                    b')' | b']' | b'}' => (d - 1).max(0),
                    _ => d,
                };
                want.push(d);
            }
            if want.iter().any(|&d| d > i32::from(MAX_DEPTH)) {
                assert!(t.depth().is_err());
                continue;
            }
            t.depth().unwrap();
            for (i, &w) in want.iter().enumerate() {
                assert_eq!(t.bit(i, |b| b.depth0), w == 0, "byte {i}");
            }
        }
    }

    #[test]
    fn regions_and_layout_as_the_lean_lexer() {
        let texts = [
            "if a:\n    b\nc\n",
            "x = (1,\n 2)\n",
            "if a:\n    b\n    # c\nd\n",
            "if a:\n    b\n# c\nd\n",
            "f'a{b!r:>{w}}c'\n",
            "'\\n\\d'\n",
            "1..real\n",
            "0x_ffL 1_0.0_1e1_0j .5\n",
            "x = 'a # b' # c ( \ny = [1, # ]\n 2]\n",
            "s = f\"{d['k']:>{w}} {x}}}{{y\"\n",
            "t = '''a\n'' b ( '''\nu = \"\"\"a\\\"\"\" b\"\"\"\n",
            "r'\\'' + rb'\\\\' + b'\\N' + f'{a!=b}{c:=d}'\n",
            "def f():\n    return [\n        1,\n    ]\n\nclass C:\n    pass\n",
            "x = 1 \\\n  + 2\n",
            "a = {'k': (1, [2, {3}])}\nif a:\n  b\r\nelse:\n  c\r\n",
            "\u{feff}x = 1\n",
            "s = f'''\n{\n  x\n}\n'''\n",
            "é = 1\nxf'a'\n",
        ];
        let bad: Vec<_> = texts.iter().map(|t| (t, check(t.as_bytes()))).filter(|(_, c)| *c != Check::Same).collect();
        assert!(bad.is_empty(), "{bad:#?}");
        // A text across many blocks: each region and bracket crossing a block's edge somewhere.
        let mut long = String::new();
        for k in 0..40 {
            long.push_str(&format!("x{k} = f(a, '{}', \"\"\"q\n{}\"\"\", # c ( {k}\n  [{}])  # e\n", "s".repeat(k), "t".repeat(k * 3), "1, ".repeat(k)));
        }
        assert_eq!(check(long.as_bytes()), Check::Same);
    }

    #[test]
    fn operators_are_the_lean_lexers() {
        let bytes = b"*/<>=!:+-@%&^|.()[]{},;~ax";
        for &a in bytes {
            for &b in bytes {
                for &c in bytes {
                    let s = [a, b, c];
                    assert_eq!(operator(a, b, c).map(|(t, n)| (t as u8, n)), lex::operator_at(&s, 0).map(|(t, n)| (t as u8, n)), "{:?}", String::from_utf8_lossy(&s));
                }
            }
        }
    }

    #[test]
    fn a_nested_f_string_is_refused() {
        assert!(matches!(check(b"f\"{f'{x}'}\"\n"), Check::StrataRefuses(_)));
        let deep = format!("x = {}{}\n", "(".repeat(40), ")".repeat(40));
        assert!(matches!(check(deep.as_bytes()), Check::StrataRefuses(_)));
    }
}
