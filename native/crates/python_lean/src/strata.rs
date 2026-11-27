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
//! 3. **Depth** (`Text::depth`): the bracket depth after each byte of code, a prefix sum and a prefix minimum of
//!    the block's 64 steps (the minimum clamps it at 0, as the lean lexer's counter is), with the depth carried
//!    in; out comes `depth0`. A block with no bracket is one test. A depth past 31 is refused (a rule).
//! 4. **Tokens** (`Walk`): the lean lexer's tokens and layout read from the masks: an item's start the next byte
//!    past the blanks, a name's end its run's, a string's stops one mask of its own kind, and whether a line end
//!    is inside brackets one bit of `depth0`, not a count.

use crate::lex::{self, Extra, Lexed, MAX_NESTING, Quote, Refusal, T, Tok};
use rerail_simd::prelude::*;
use rerail_simd::{Mask, Simd};

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
    /// `'\n'`, `'\r'`, `'\f'`: what makes a gap a line's end.
    brk: u64,
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
    lb: u64,
    rb: u64,
    /// `(`, `[`, `{`.
    ob: u64,
    /// `)`, `]`, `}`.
    cb: u64,
    colon: u64,
    bang: u64,
    /// A string prefix's letters (`rRbBfFtTuU`), and the format ones (`fFtT`).
    pfx: u64,
    fmt: u64,
    /// Stratum 2: not in a comment or a string (an f-string's interiors are in its string).
    code: u64,
    /// Stratum 3: the bracket depth after this byte is 0.
    depth0: u64,
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
    let low = v | s(0x20);
    let alpha = (low - s(b'a')).simd_lt(s(26));
    let digit = (v - s(b'0')).simd_lt(s(10));
    let word = (alpha | digit | eq(b'_')).to_bitmask();
    let (space, tab, ff, lf, cr) = (eq(b' ').to_bitmask(), eq(b'\t').to_bitmask(), eq(0x0c).to_bitmask(), eq(b'\n').to_bitmask(), eq(b'\r').to_bitmask());
    let (lb, rb) = (eq(b'{').to_bitmask(), eq(b'}').to_bitmask());
    let leq = |x: u8| low.simd_eq(s(x));
    let fmt = leq(b'f') | leq(b't');
    B {
        word,
        space,
        sptab: space | tab,
        skip: space | tab | ff | lf,
        eol: lf | cr,
        brk: lf | cr | ff,
        lf,
        bare_lf: cr,
        sq: eq(b'\'').to_bitmask(),
        dq: eq(b'"').to_bitmask(),
        bt: eq(b'`').to_bitmask(),
        bs: eq(b'\\').to_bitmask(),
        esc: 0,
        hash: eq(b'#').to_bitmask(),
        lb,
        rb,
        ob: (eq(b'(') | eq(b'[')).to_bitmask() | lb,
        cb: (eq(b')') | eq(b']')).to_bitmask() | rb,
        colon: eq(b':').to_bitmask(),
        bang: eq(b'!').to_bitmask(),
        pfx: (fmt | leq(b'r') | leq(b'b') | leq(b'u')).to_bitmask(),
        fmt: fmt.to_bitmask(),
        code: !0,
        depth0: 0,
    }
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

/// Byte `i / 8` of `m` in lane `i`, tested at its bit `i % 8`: a block's mask as a mask of lanes.
#[inline(always)]
fn lanes(m: u64) -> Mask<i8, 64> {
    const IDX: [u8; 64] = {
        let mut t = [0u8; 64];
        let mut i = 0;
        while i < 64 {
            t[i] = (i / 8) as u8;
            i += 1;
        }
        t
    };
    const BIT: [u8; 64] = {
        let mut t = [0u8; 64];
        let mut i = 0;
        while i < 64 {
            t[i] = 1 << (i % 8);
            i += 1;
        }
        t
    };
    let mut t = [0u8; 16];
    t[..8].copy_from_slice(&m.to_le_bytes());
    let table = Simd::<u8, 16>::from_array(t);
    let mut o = [0u8; 64];
    for k in 0..4 {
        table.swizzle_dyn(Simd::from_slice(&IDX[16 * k..])).copy_to_slice(&mut o[16 * k..16 * k + 16]);
    }
    (V::from_array(o) & V::from_array(BIT)).simd_ne(V::splat(0))
}

/// One block's depths: the bits where the depth after the byte is 0, the depth carried out, and whether any is
/// past the bound. `up` and `down` are the block's opening and closing brackets in code.
#[inline(always)]
fn depth_block(up: u64, down: u64, carry: i8) -> (u64, i8, bool) {
    type D = Simd<i8, 64>;
    // +1 at an opener, -1 at a closer (a set lane is -1).
    let mut s: D = lanes(down).to_int() - lanes(up).to_int();
    s += s.shift_elements_right::<1>(0);
    s += s.shift_elements_right::<2>(0);
    s += s.shift_elements_right::<4>(0);
    s += s.shift_elements_right::<8>(0);
    s += s.shift_elements_right::<16>(0);
    s += s.shift_elements_right::<32>(0);
    s += D::splat(carry);
    // The depth clamped at 0 as it goes: the sum less its least value so far, where that is below 0.
    let mut m = s;
    m = m.simd_min(m.shift_elements_right::<1>(i8::MAX));
    m = m.simd_min(m.shift_elements_right::<2>(i8::MAX));
    m = m.simd_min(m.shift_elements_right::<4>(i8::MAX));
    m = m.simd_min(m.shift_elements_right::<8>(i8::MAX));
    m = m.simd_min(m.shift_elements_right::<16>(i8::MAX));
    m = m.simd_min(m.shift_elements_right::<32>(i8::MAX));
    let d = s - m.simd_min(D::splat(0));
    (d.simd_eq(D::splat(0)).to_bitmask(), d[63], d.simd_gt(D::splat(MAX_DEPTH)).any())
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

    /// The first byte from `at` whose bit is set in the mask `f` makes of a block and the block after it (the
    /// text's length where none is).
    #[inline(always)]
    fn next2(&self, at: usize, f: impl Fn(&B, &B) -> u64) -> usize {
        let n = self.src.len();
        if at >= n {
            return n;
        }
        let mut bi = at >> 6;
        let mut m = f(self.blk(bi), self.blk(bi + 1)) & (!0u64 << (at & 63));
        loop {
            if m != 0 {
                return (bi * 64 + m.trailing_zeros() as usize).min(n);
            }
            bi += 1;
            if bi * 64 >= n {
                return n;
            }
            m = f(self.blk(bi), self.blk(bi + 1));
        }
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

    /// The first byte from `at` whose bit is clear in a mask: a run's end.
    #[inline(always)]
    fn run_end(&self, at: usize, f: impl Fn(&B) -> u64) -> usize {
        self.next(at, |b| !f(b))
    }

    /// Whether any byte of `[a, b)` is set in a mask (`a < b`, both at most the text's length).
    #[inline(always)]
    fn any(&self, a: usize, b: usize, f: impl Fn(&B) -> u64) -> bool {
        let (ba, bb) = (a >> 6, (b - 1) >> 6);
        let lo = !0u64 << (a & 63);
        let hi = !0u64 >> (63 - ((b - 1) & 63));
        if ba == bb {
            return f(self.blk(ba)) & lo & hi != 0;
        }
        (ba + 1..bb).any(|x| f(self.blk(x)) != 0) || f(self.blk(ba)) & lo != 0 || f(self.blk(bb)) & hi != 0
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
    fn next2(&self, at: usize, f: impl Fn(&B, &B) -> u64) -> usize {
        self.m().next2(at, f)
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
            let end = self.string_end(o, format, 0)?;
            self.fill(start, end);
            pos = end;
        }
    }

    /// Where the string whose quote is at `o` starts (its prefix's first letter), and whether it is a format
    /// string: the run of prefix letters just below the quote, where no word byte is below it.
    #[inline(always)]
    fn prefix(&self, o: usize) -> (usize, bool) {
        let (bi, q) = (o >> 6, o & 63);
        let b = &self.b[bi];
        let below = (1u64 << q) - 1;
        let not_prefix = !b.pfx & below;
        if not_prefix == 0 {
            // The run may go on into the block before (or the quote is the block's first byte).
            return self.prefix_slow(o);
        }
        let top = 63 - not_prefix.leading_zeros() as usize;
        let run = below & !((2u64 << top) - 1);
        if run == 0 || b.word >> top & 1 == 1 || self.src[bi * 64 + top] >= 0x80 {
            return (o, false);
        }
        (bi * 64 + top + 1, b.fmt & run != 0)
    }

    fn prefix_slow(&self, o: usize) -> (usize, bool) {
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
    fn string_end(&self, o: usize, format: bool, nest: u32) -> Result<usize, Refusal> {
        let n = self.src.len();
        let q = self.src[o];
        let triple = self.at(o + 1) == q && self.at(o + 2) == q;
        let braces = |b: &B| if format { b.lb | b.rb } else { 0 };
        let mut j = o + if triple { 3 } else { 1 };
        loop {
            let c = if triple {
                // A quote no backslash escapes, and the two bytes after it its quote.
                self.next2(j, |b, b1| {
                    let (qm, q1) = (quotes(b, q), quotes(b1, q));
                    (qm & !b.esc & (qm >> 1 | q1 << 63) & (qm >> 2 | q1 << 62)) | braces(b)
                })
            } else {
                self.next(j, |b| (quotes(b, q) & !b.esc) | b.bare_lf | braces(b))
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
    fn interior_end(&self, mut i: usize, nest: u32) -> Result<usize, Refusal> {
        if nest > MAX_NESTING {
            return Err(refuse(i));
        }
        let n = self.src.len();
        let mut depth = 0i32;
        loop {
            let c = self.next(i, |b| b.sq | b.dq | b.bt | b.rb | b.colon | b.bang | b.hash | b.bs);
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
                    let (_, format) = self.prefix(c);
                    if format {
                        return Err(refuse(c));
                    }
                    i = self.string_end(c, false, nest)?;
                }
            }
        }
    }

    /// The end of a format specifier from `i` (after its `:`): its `}`, past its own interpolations.
    fn spec_end(&self, mut i: usize, nest: u32) -> Result<usize, Refusal> {
        let n = self.src.len();
        loop {
            let c = self.next(i, |b| b.lb | b.rb | b.lf);
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
            let (zero, out, deep) = depth_block(up, down, carry);
            if deep {
                return Err(refuse(bi * 64));
            }
            b.depth0 = zero;
            carry = out;
        }
        Ok(())
    }
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
            // The usual case: the gap to the next item holds no line's end (one test of a mask), and the item is a
            // token; the scanner gives nothing, so the token is read at once.
            let i = self.t.run_end(self.p, |b| b.skip);
            if i < n && (i == self.p || !self.t.any(self.p, i, |b| b.brk)) && !matches!(src[i], b'#' | b'\\' | b'\r') && src[i] < 0x80 {
                let kind = self.token(i)?;
                open = true;
                header = kind == T::Colon && !self.in_brackets(i + 1);
                continue;
            }
            let k = self.t.run_end(self.p, |b| b.sptab);
            let line_end = k >= n || matches!(self.t.src[k], b'\n' | b'\r' | b'#' | b'\\' | 0x0c);
            if line_end {
                if self.in_brackets(self.p) {
                    if self.dedent_in_brackets(self.p)? {
                        return Err(refuse(self.p));
                    }
                } else {
                    while let Some((indent, first_comment, _)) = self.scan(self.p)? {
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
            let kind = self.token(i)?;
            open = true;
            header = kind == T::Colon && !self.in_brackets(i + 1);
        }
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

    /// The token at `i`, and those of a string it starts.
    #[inline(always)]
    fn token(&mut self, i: usize) -> Result<T, Refusal> {
        let src = self.t.src;
        let c = src[i];
        if self.t.bit(i, |b| b.word) && !c.is_ascii_digit() || c >= 0x80 {
            if self.t.bit(i, |b| b.pfx)
                && let Some((prefix, q)) = lex::string_prefix_at(src, i)
            {
                self.string(i, prefix, q)?;
                return Ok(T::StrEnd);
            }
            let end = self.name_end(i)?;
            let (kind, soft) = lex::keyword(src, i, end);
            self.toks.push(Tok { kind, soft, start: i as u32, end: end as u32 });
            self.p = end;
            return Ok(kind);
        }
        if c.is_ascii_digit() || (c == b'.' && self.at(i + 1).is_ascii_digit()) {
            let (kind, end) = lex::number_at(src, i);
            self.push(kind, i, end);
            return Ok(kind);
        }
        if c == b'\'' || c == b'"' || c == b'`' {
            let (prefix, q) = lex::string_prefix_at(src, i).expect("a quote");
            self.string(i, prefix, q)?;
            return Ok(T::StrEnd);
        }
        let (kind, len) = lex::operator_at(src, i).ok_or_else(|| refuse(i))?;
        self.push(kind, i, i + len);
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
            let s = self.t.next(j, |b| quotes(b, quote) | b.bs | b.lf | if format { b.lb | b.rb } else { 0 });
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
            let s = self.t.next(i, |b| b.lb | b.rb | b.lf);
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
    fn a_nested_f_string_is_refused() {
        assert!(matches!(check(b"f\"{f'{x}'}\"\n"), Check::StrataRefuses(_)));
        let deep = format!("x = {}{}\n", "(".repeat(40), ")".repeat(40));
        assert!(matches!(check(deep.as_bytes()), Check::StrataRefuses(_)));
    }
}
