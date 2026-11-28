//! A lexer of strata (decision 0698, items 92 and 93): the lean lexer's tokens and extras, made in layers of
//! masks, so that the two can be compared for speed and accuracy. Each layer reads the ones below it:
//!
//! 1. **Classes** (`classes`): each 64-byte block's bytes classified on `std::simd`'s interface (`rerail-simd`), a
//!    mask a class, a bit a byte; the bytes a backslash escapes by simdjson's odd-run trick, one carry bit from
//!    the block before.
//! 2. **Regions** (`Text::regions`): comments and strings, block by block, every step bit operations. The lowest
//!    opener left in code is `live`'s lowest bit; its kind is which mask holds it; its prefix the run of prefix
//!    letters below it (one count of leading zeros of a two-block window); its closer the lowest bit past it of one
//!    mask (a line end; a quote no backslash escapes or a bare line end; a triple, the quote's mask and its two next
//!    bytes'), selected by the kind's bits; its span goes into the block's mask, and the openers through it are
//!    cleared. A region open at the block's end is carried. An f-string's text stops too at its interiors' `{` (the
//!    last of an odd run of `{`) and at a lone `}` (the odd-run trick over the run of `}` from the text's start),
//!    and its interiors' stops are one mask, their depth popcounts between the stops. Out come `code` and
//!    `sstart`. An f-string inside an interior is refused, to tree-sitter.
//! 3. **Depth** (`Text::depth`): the bracket depth after each byte of code, a prefix sum over the block's 64 lanes
//!    (each register of 16 four shifted adds, then the registers' carries), the depth carried in; clamped at 0 as
//!    the lean lexer's counter is by a prefix minimum where the sum goes below 0 (rare, one test). Out comes
//!    `depth0`. A block with no bracket is one test. A depth past 31 is refused (a rule).
//! 4. **Tokens** (`Walk`): the lean lexer's tokens and layout read from the masks: an item's start the next byte
//!    past the blanks, a name's end its run's, a string's stops one mask of its own kind, and whether a line end
//!    is inside brackets one bit of `depth0`, not a count.

pub mod middle;

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
    /// `{`, `}`: made only in a block that has a brace (one test of the two compares' lanes).
    lb: u64,
    rb: u64,
    /// The `{` no `{` pairs with: of each run of `{`, the first of each pair and an odd run's last (the odd-run trick
    /// over the runs of `{`, one carry bit from the block before).
    lbu: u64,
    /// The bytes the run reads a token at by a step: a digit, a quote, a byte past ASCII, and a byte an operator
    /// of two or three bytes may start at (the next byte is `=` or `>`, or the byte is one of `*/<>.` and the next
    /// the same; a space before `=` or `>` is not). A name that ends at one is read by a step too.
    slow: u64,
    /// Stratum 2: not in a comment or a string (an f-string's interiors are in its string).
    code: u64,
    /// Stratum 3: the bracket depth after this byte is 0.
    depth0: u64,
    /// Stratum 2: a string's first byte (its prefix's, or its quote's), in code or in an interior.
    sstart: u64,
    /// Stratum 2: a string's closer's first byte (in variant B, of the strings in code only).
    send: u64,
    /// A string prefix's letters (`rRbBfFtTuU`): what the superposed pass 1 reads of the prefixes (a run of them
    /// below an opener that reaches the window's bottom is B's refusal of the reading).
    pfx: u64,
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

/// simdjson's escapes: the bytes after an odd run of `bs`'s bits, and the bits of each run past its first, second
/// and so on in pairs. `prev` is whether the block's first byte is escaped, and becomes the next block's.
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

/// Stratum 1's nibble tables (simdjson's): a byte's classes are `LO[its low nibble] & HI[its high nibble]`, each class
/// a bit whose bytes are a rectangle of high and low nibbles. A mask that is a union of classes is one bit test.
mod nib {
    /// Of the first class byte: letters (two rectangles), digits, `_`, `(`, `[` `{`, `)`, `]` `}`.
    pub const ALPHA1: u8 = 1;
    pub const ALPHA2: u8 = 2;
    pub const DIGIT: u8 = 4;
    pub const UNDER: u8 = 8;
    pub const LPAREN: u8 = 16;
    pub const LSQ: u8 = 32;
    pub const RPAREN: u8 = 64;
    pub const RSQ: u8 = 128;
    pub const WORD: u8 = ALPHA1 | ALPHA2 | DIGIT | UNDER;
    pub const OPEN: u8 = LPAREN | LSQ;
    pub const CLOSE: u8 = RPAREN | RSQ;
    /// Of the second: a tab, form feed or `\r`; a backtick; `'` and `"`; a byte past ASCII; `*` `.` `/`; `<` `>`; `#`;
    /// `{` `}`.
    pub const RARE_WS: u8 = 1;
    pub const TICK: u8 = 2;
    pub const QUOTE: u8 = 4;
    pub const HIGH: u8 = 8;
    pub const DUP1: u8 = 16;
    pub const DUP2: u8 = 32;
    pub const HASH: u8 = 64;
    pub const BRACE: u8 = 128;

    const fn has(set: &[u8], x: u8) -> bool {
        let mut i = 0;
        while i < set.len() {
            if set[i] == x {
                return true;
            }
            i += 1;
        }
        false
    }

    /// The tables of classes given as (bit, high nibbles, low nibbles).
    const fn tables(classes: &[(u8, &[u8], &[u8])]) -> ([u8; 16], [u8; 16]) {
        let (mut lo, mut hi) = ([0u8; 16], [0u8; 16]);
        let mut k = 0;
        while k < classes.len() {
            let (bit, his, los) = classes[k];
            let mut n = 0;
            while n < 16 {
                if has(his, n as u8) {
                    hi[n] |= bit;
                }
                if has(los, n as u8) {
                    lo[n] |= bit;
                }
                n += 1;
            }
            k += 1;
        }
        (lo, hi)
    }

    const ALL: &[u8] = &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
    pub const T1: ([u8; 16], [u8; 16]) = tables(&[
        (ALPHA1, &[4, 6], &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]),
        (ALPHA2, &[5, 7], &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10]),
        (DIGIT, &[3], &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]),
        (UNDER, &[5], &[15]),
        (LPAREN, &[2], &[8]),
        (LSQ, &[5, 7], &[11]),
        (RPAREN, &[2], &[9]),
        (RSQ, &[5, 7], &[13]),
    ]);
    pub const T2: ([u8; 16], [u8; 16]) = tables(&[
        (RARE_WS, &[0], &[9, 12, 13]),
        (TICK, &[6], &[0]),
        (QUOTE, &[2], &[2, 7]),
        (HIGH, &[8, 9, 10, 11, 12, 13, 14, 15], ALL),
        (DUP1, &[2], &[10, 14, 15]),
        (DUP2, &[3], &[12, 14]),
        (HASH, &[2], &[3]),
        (BRACE, &[7], &[11, 13]),
    ]);
}

/// Stratum 1 of one block: two class bytes by nibble tables, then each mask one compare or bit test (`vn`: the
/// bytes one on).
#[inline(always)]
fn block(v: V, vn: V) -> B {
    let s = V::splat;
    let eq = |x: u8| v.simd_eq(s(x));
    let (lo, hi) = (v & s(0x0f), v >> s(4));
    let t = |(l, h): ([u8; 16], [u8; 16])| lo.lookup16(Simd::from_array(l)) & hi.lookup16(Simd::from_array(h));
    let (k1, k2) = (t(nib::T1), t(nib::T2));
    let is = |k: V, bits: u8| (k & s(bits)).simd_ne(s(0));
    let (space, lf) = (eq(b' ').to_bitmask(), eq(b'\n').to_bitmask());
    // Tabs, form feeds, `'\r'` and backticks are rare: one test, and each its own only in a block that has one.
    let (tab, ff, cr, bt) = if is(k2, nib::RARE_WS | nib::TICK).any() {
        (eq(b'\t').to_bitmask(), eq(0x0c).to_bitmask(), eq(b'\r').to_bitmask(), eq(b'`').to_bitmask())
    } else {
        (0, 0, 0, 0)
    };
    let (lbm, rbm) = if is(k2, nib::BRACE).any() { (eq(b'{').to_bitmask(), eq(b'}').to_bitmask()) } else { (0, 0) };
    // The next byte `=` or `>`: one subtraction and compare.
    let next_eq_gt = (vn - s(b'=')).simd_lt(s(2)) & v.simd_ne(s(b' '));
    B {
        word: is(k1, nib::WORD).to_bitmask(),
        space,
        sptab: space | tab,
        skip: space | tab | ff | lf,
        eol: lf | cr,
        lf,
        bare_lf: cr,
        sq: eq(b'\'').to_bitmask(),
        dq: eq(b'"').to_bitmask(),
        bt,
        bs: eq(b'\\').to_bitmask(),
        esc: 0,
        hash: is(k2, nib::HASH).to_bitmask(),
        ob: is(k1, nib::OPEN).to_bitmask() | lbm,
        cb: is(k1, nib::CLOSE).to_bitmask() | rbm,
        lb: lbm,
        rb: rbm,
        lbu: 0,
        slow: (is(k1, nib::DIGIT) | is(k2, nib::TICK | nib::QUOTE | nib::HIGH) | next_eq_gt | (v.simd_eq(vn) & is(k2, nib::DUP1 | nib::DUP2))).to_bitmask(),
        code: !0,
        depth0: 0,
        sstart: 0,
        send: 0,
        pfx: {
            let low = v | s(0x20);
            (low.simd_eq(s(b'f')) | low.simd_eq(s(b't')) | low.simd_eq(s(b'r')) | low.simd_eq(s(b'b')) | low.simd_eq(s(b'u'))).to_bitmask()
        },
    }
}

/// The 64 bytes from `at`, zeros past the text's end.
#[cold]
fn bytes_from(src: &[u8], at: usize) -> V {
    let mut b = [0u8; 64];
    let k = src.len().saturating_sub(at).min(64);
    if k > 0 {
        b[..k].copy_from_slice(&src[at..at + k]);
    }
    V::from_array(b)
}

/// Block `bi`'s 64 bytes, zeros past the text's end.
#[inline(always)]
fn bytes(src: &[u8], bi: usize) -> V {
    let at = bi * 64;
    match src.get(at..at + 64) {
        Some(b) => V::from_slice(b),
        None => {
            let mut b = [0u8; 64];
            let k = src.len().saturating_sub(at).min(64);
            b[..k].copy_from_slice(&src[at..at + k]);
            V::from_array(b)
        }
    }
}

/// Block `bi`'s bytes that are one of `xs`: made from its bytes where an f-string's interior asks (its `:`), not
/// for every block.
#[inline(always)]
fn bytes_of<const K: usize>(src: &[u8], bi: usize, xs: [u8; K]) -> u64 {
    let v = bytes(src, bi);
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
    let mut out = Vec::with_capacity(src.len().div_ceil(64) + 1);
    classes_in(src, 0, src.len().div_ceil(64), &mut S1::default(), &mut out, usize::MAX, &mut S1::default());
    out.push(B::default());
    out
}

/// Stratum 1's carries into a block: the escape bit of a run of backslashes, of a `'\r'` an escaped one ends, and of a
/// run of `{`.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct S1 {
    pub(crate) carry: u64,
    pub(crate) cr: u64,
    pub(crate) lb: u64,
}

/// Blocks `from..to`'s classes after `out`, from the carries `c` into block `from` (left as those out of block `to - 1`),
/// and the carries into block `at` kept in `at_c`.
#[inline(always)]
fn classes_in(src: &[u8], from: usize, to: usize, c: &mut S1, out: &mut Vec<B>, at: usize, at_c: &mut S1) {
    let n = src.len();
    let (mut carry, mut cr_carry, mut lb_carry) = (c.carry, c.cr, c.lb);
    let mut at_b = from * 64;
    let end = (to * 64).min(n);
    while at_b < end {
        if at_b >> 6 == at {
            *at_c = S1 { carry, cr: cr_carry, lb: lb_carry };
        }
        let at = at_b;
        let v = match src.get(at..at + 64) {
            Some(b) => V::from_slice(b),
            None => {
                let mut b = [0u8; 64];
                b[..n - at].copy_from_slice(&src[at..]);
                V::from_array(b)
            }
        };
        // The bytes one on: the next byte of each, for the operators' second bytes.
        let vn = match src.get(at + 1..at + 65) {
            Some(b) => V::from_slice(b),
            None => bytes_from(src, at + 1),
        };
        let mut b = block(v, vn);
        b.esc = escaped(b.bs, &mut carry);
        // `bare_lf` held the block's `'\r'`s: a `'\n'` after an escaped one is escaped too.
        let esc_cr = b.bare_lf & b.esc;
        b.bare_lf = b.lf & !b.esc & !((esc_cr << 1) | cr_carry);
        cr_carry = esc_cr >> 63;
        b.lbu = b.lb & !escaped(b.lb, &mut lb_carry);
        out.push(b);
        at_b += 64;
    }
    *c = S1 { carry, cr: cr_carry, lb: lb_carry };
}

/// The bracket depth's bound: a file past it is refused (decision 0698, item 93).
const MAX_DEPTH: i8 = 31;

type L = Simd<u8, 16>;
type D = Simd<i8, 16>;

/// A block's mask as its 64 lanes, four registers of 16: -1 where the bit is set, else 0. Each byte of the mask is
/// spread over 8 lanes by one table lookup, and each lane keeps its own bit.
#[inline(always)]
fn lanes(m: u64) -> [D; 4] {
    let b = m.to_le_bytes();
    let t = L::from_array([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], 0, 0, 0, 0, 0, 0, 0, 0]);
    let bit = L::from_array([1, 2, 4, 8, 16, 32, 64, 128, 1, 2, 4, 8, 16, 32, 64, 128]);
    let r = |k: u8| {
        let ix = L::from_array([2 * k, 2 * k, 2 * k, 2 * k, 2 * k, 2 * k, 2 * k, 2 * k, 2 * k + 1, 2 * k + 1, 2 * k + 1, 2 * k + 1, 2 * k + 1, 2 * k + 1, 2 * k + 1, 2 * k + 1]);
        (t.swizzle_dyn(ix) & bit).simd_ne(L::splat(0)).to_int()
    };
    [r(0), r(1), r(2), r(3)]
}

/// A register's inclusive prefix sum: four shifted adds.
#[inline(always)]
fn prefix_sum(x: D) -> D {
    let x = x + x.shift_elements_right::<1>(0);
    let x = x + x.shift_elements_right::<2>(0);
    let x = x + x.shift_elements_right::<4>(0);
    x + x.shift_elements_right::<8>(0)
}

/// A register's inclusive prefix minimum: four shifted minimums.
#[inline(always)]
fn prefix_min(x: D) -> D {
    let x = x.simd_min(x.shift_elements_right::<1>(i8::MAX));
    let x = x.simd_min(x.shift_elements_right::<2>(i8::MAX));
    let x = x.simd_min(x.shift_elements_right::<4>(i8::MAX));
    x.simd_min(x.shift_elements_right::<8>(i8::MAX))
}

/// One block's depths (`up` and `down`, its brackets in code): the bits where the depth after the byte is 0, the
/// depth carried out, and whether any is past the bound. The depth is a prefix sum over the block's 64 lanes, each
/// register of 16 its own four shifted adds and then the registers' carries, the one carried in added first. Where
/// the sum goes below 0 (a closer with none open, which the lean lexer's counter clamps at 0), a prefix minimum
/// clamps it: rare, and one test of the sum skips it.
#[inline(always)]
fn depth_lanes(up: u64, down: u64, carry: i8) -> (u64, i8, bool) {
    let (u, d) = (lanes(up), lanes(down));
    let mut s = [D::splat(0); 4];
    let mut c = D::splat(carry);
    for k in 0..4 {
        s[k] = prefix_sum(d[k] - u[k]) + c;
        c = D::splat(s[k][15]);
    }
    let neg = (s[0].simd_min(s[1]).simd_min(s[2].simd_min(s[3]))).simd_lt(D::splat(0)).any();
    if neg {
        let mut m = D::splat(0);
        for x in &mut s {
            let pm = prefix_min(*x).simd_min(m);
            m = D::splat(pm[15]);
            *x -= pm;
        }
    }
    let deep = (s[0].simd_max(s[1]).simd_max(s[2].simd_max(s[3]))).simd_gt(D::splat(MAX_DEPTH)).any();
    let mut all = [0i8; 64];
    for (k, x) in s.iter().enumerate() {
        all[16 * k..16 * k + 16].copy_from_slice(x.as_array());
    }
    let zero = Simd::<i8, 64>::from_array(all).simd_eq(Simd::splat(0)).to_bitmask();
    (zero, s[3][15], deep)
}

/// A block's prefix masks, made from its bytes where a string's quote has a word byte just below it: a string
/// prefix's letters (`rRbBfFtTuU`), the format ones (`fFtT`), and the bytes that may not stand just below a prefix
/// (`[A-Za-z0-9_]` or past ASCII).
#[derive(Clone, Copy, Default)]
struct Pfx {
    pfx: u64,
    fpfx: u64,
    ident: u64,
}

#[inline(always)]
fn prefix_masks(src: &[u8], bi: usize, word: u64) -> Pfx {
    let v = bytes(src, bi);
    let low = v | V::splat(0x20);
    let leq = |x: u8| low.simd_eq(V::splat(x));
    let fmt = leq(b'f') | leq(b't');
    Pfx { pfx: (fmt | leq(b'r') | leq(b'b') | leq(b'u')).to_bitmask(), fpfx: fmt.to_bitmask(), ident: word | v.simd_ge(V::splat(0x80)).to_bitmask() }
}

/// The text and its blocks' masks.
struct Text<'s> {
    src: &'s [u8],
    /// Blocks `first..`'s masks.
    b: Vec<B>,
    first: usize,
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
    /// The text's blocks and one past its end, from block `first`: a window of them where a chunk is lexed from the
    /// middle, which says where a read went past it (`short`).
    b: &'a [B],
    first: usize,
    short: &'a core::cell::Cell<bool>,
}

impl<'a> Masks<'a> {
    #[inline(always)]
    fn at(&self, i: usize) -> u8 {
        self.src.get(i).copied().unwrap_or(0)
    }

    /// Block `bi`, at most the one past the text's end.
    #[inline(always)]
    fn blk(&self, bi: usize) -> &'a B {
        static NONE: B = B { word: 0, space: 0, sptab: 0, skip: 0, eol: 0, lf: 0, bare_lf: 0, sq: 0, dq: 0, bt: 0, bs: 0, esc: 0, hash: 0, ob: 0, cb: 0, lb: 0, rb: 0, lbu: 0, slow: 0, code: 0, depth0: 0, sstart: 0, send: 0, pfx: 0 };
        // The whole text's blocks hold every block a read asks for (`classes` makes one for each 64 bytes and one
        // more); a window may not, and a read past it is made again over a longer one.
        match self.b.get(bi.wrapping_sub(self.first)) {
            Some(b) => b,
            None => {
                self.short.set(true);
                &NONE
            }
        }
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
            if bi * 64 >= n || self.short.get() {
                return n;
            }
            m = f(self.blk(bi));
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
            if bi * 64 >= n || self.short.get() {
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

/// What stratum 2's step is inside: the stack's frames, code at its bottom.
#[derive(Clone, Copy, PartialEq, Eq)]
enum F {
    Code,
    Comment,
    /// A string's text: its quote, whether a triple, whether a format string.
    Text { q: u8, triple: bool, format: bool },
    /// An f-string's interior, its bracket depth so far.
    Interior { depth: i32 },
    /// A format specifier, after its interior's `:`.
    Spec,
}

/// A bit window of two blocks: the one before `bi` below, `bi`'s above.
#[inline(always)]
fn win(lo: u64, hi: u64) -> u128 {
    (u128::from(hi) << 64) | u128::from(lo)
}

/// Bits `[a, 64)`.
#[inline(always)]
fn from(a: u32) -> u64 {
    if a >= 64 { 0 } else { !0u64 << a }
}

impl<'s> Text<'s> {
    #[inline(always)]
    fn m<'a>(&'a self, short: &'a core::cell::Cell<bool>) -> Masks<'a> {
        Masks { src: self.src, b: &self.b, first: self.first, short }
    }

    /// A string starts at `i`.
    #[inline(always)]
    fn mark(&mut self, i: usize) {
        self.b[(i >> 6) - self.first].sstart |= 1 << (i & 63);
    }

    /// Where the string whose quote is bit `c` of block `bi` starts (its prefix's first letter), and whether it is a
    /// format string. The prefix is the run of `pfx` just below the quote, where the byte below that run is not
    /// `ident`: in a window of the block and the one before it, the highest bit below the quote that is not `pfx`
    /// (one count of leading zeros), and the run above it.
    #[inline(always)]
    fn prefix(&self, bi: usize, c: u32) -> (usize, bool) {
        let pw = if bi > 0 { self.b[bi - 1 - self.first].word } else { 0 };
        if (win(pw, self.b[bi - self.first].word) << 1) >> (64 + c) & 1 == 0 {
            return (bi * 64 + c as usize, false);
        }
        let b = prefix_masks(self.src, bi, self.b[bi - self.first].word);
        let p = if bi > 0 { prefix_masks(self.src, bi - 1, pw) } else { Pfx::default() };
        let below = (1u128 << (64 + c)) - 1;
        let not_pfx = !win(p.pfx, b.pfx) & below;
        if not_pfx == 0 {
            return self.prefix_far(bi, c);
        }
        let h = 127 - not_pfx.leading_zeros();
        let run = below & !((2u128 << h) - 1);
        if run == 0 || win(p.ident, b.ident) >> h & 1 == 1 {
            return (bi * 64 + c as usize, false);
        }
        (bi * 64 + h as usize + 1 - 64, win(p.fpfx, b.fpfx) & run != 0)
    }

    /// A prefix of more letters than a block holds: the blocks below read until one is not all letters (rare).
    #[cold]
    fn prefix_far(&self, bi: usize, c: u32) -> (usize, bool) {
        let m = |k: usize| prefix_masks(self.src, k, self.b[k - self.first].word);
        let mut k = bi - 1;
        while k > 0 && m(k).pfx == !0 {
            k -= 1;
        }
        let not_pfx = !m(k).pfx;
        let h = if not_pfx == 0 { None } else { Some(k * 64 + 63 - not_pfx.leading_zeros() as usize) };
        let start = h.map_or(0, |h| h + 1);
        if h.is_some_and(|h| m(h >> 6).ident >> (h & 63) & 1 == 1) {
            return (bi * 64 + c as usize, false);
        }
        let format = (start..bi * 64 + c as usize).any(|i| m(i >> 6).fpfx >> (i & 63) & 1 == 1);
        (start, format)
    }

    /// Stratum 2: comments and strings, block by block. In each block a step takes the lowest event of what is open
    /// (an opener in code; a comment's line end; a string's closer, an interior's `{`, a lone `}`; an interior's
    /// stops) as one mask's lowest bit, and the masks of the bits past it; what is open at the block's end is
    /// carried to the next, with an interior's depth (its brackets' popcounts) and a `}` run's escape bit. Out
    /// come `code` and `sstart`.
    #[inline(never)]
    fn regions(&mut self) -> Result<(), Refusal> {
        let n = self.src.len();
        let blocks = n.div_ceil(64);
        let mut stack: Vec<F> = Vec::with_capacity(8);
        let mut top = F::Code;
        // The first bit of the block not yet read; where the outermost region opened; the bits of the block a
        // triple's closer from the block before takes; a `}` run's escape carried in a string's text.
        let (mut p, mut open, mut taken, mut rbc) = (0u32, 0usize, 0u64, 0u64);
        let mut nest = 0u32;
        for bi in 0..blocks {
            let (b, b1) = (self.b[bi - self.first], self.b[bi + 1 - self.first]);
            let base = bi * 64;
            let mut nc = taken;
            taken = 0;
            // An interior's `:`, made where an interior reaches this block.
            let mut colon: Option<u64> = None;
            loop {
                if p >= 64 {
                    p -= 64;
                    break;
                }
                if top == F::Code {
                    // The plan's step, for comments and plain strings: the lowest opener left (`live`'s lowest
                    // bit), its closer the lowest bit of one mask past it, its span into `nc`, and the openers
                    // through it cleared. A format string goes on in the frames' step below.
                    let mut live = (b.hash | ((b.sq | b.dq | b.bt) & !b.esc)) & from(p);
                    let mut next = None;
                    while live != 0 {
                        let c = live.trailing_zeros();
                        if b.hash >> c & 1 == 1 {
                            let hits = b.eol & from(c);
                            if hits == 0 {
                                top = F::Comment;
                                open = base + c as usize;
                                stack.push(F::Code);
                                break;
                            }
                            let e = hits.trailing_zeros();
                            nc |= from(c) & !from(e);
                            live &= from(e);
                            continue;
                        }
                        let (start, format) = self.prefix(bi, c);
                        self.mark(start);
                        if start < base {
                            self.b[bi - 1 - self.first].code &= !(!0u64 << (start - (base - 64)));
                        }
                        let rel = start.max(base) - base;
                        let (q, triple) = self.quote(&b, &b1, c);
                        let k = if triple { 3 } else { 1 };
                        if format {
                            open = base + rel;
                            stack.push(F::Code);
                            top = F::Text { q, triple, format };
                            rbc = 0;
                            next = Some(c + k);
                            break;
                        }
                        let (qm, q1) = (quotes(&b, q), quotes(&b1, q));
                        let t = 0u64.wrapping_sub(u64::from(triple));
                        let one = (qm & !b.esc) | b.bare_lf;
                        let three = qm & !b.esc & (qm >> 1 | q1 << 63) & (qm >> 2 | q1 << 62);
                        let hits = ((three & t) | (one & !t)) & from(c + k);
                        if hits == 0 {
                            open = base + rel;
                            stack.push(F::Code);
                            top = F::Text { q, triple, format };
                            next = Some(c + k);
                            break;
                        }
                        let e = hits.trailing_zeros();
                        if b.bare_lf >> e & 1 == 1 && !triple {
                            return Err(refuse(base + e as usize));
                        }
                        let end = e + k;
                        self.b[bi - self.first].send |= 1 << e;
                        nc |= from(rel as u32) & !from(end);
                        if end > 64 {
                            taken = !from(end - 64);
                            next = Some(end);
                            break;
                        }
                        live &= from(end);
                    }
                    match next {
                        Some(x) => {
                            p = x;
                            continue;
                        }
                        None if top == F::Code => {
                            p = 0;
                            break;
                        }
                        None => {
                            // A comment open past the block.
                            p = 0;
                            break;
                        }
                    }
                }
                let past = from(p);
                let ev = match top {
                    F::Code => b.hash | ((b.sq | b.dq | b.bt) & !b.esc),
                    F::Comment => b.eol,
                    F::Text { q, triple, format } => {
                        let qm = quotes(&b, q);
                        let q1 = quotes(&b1, q);
                        // A one-line string's or a triple's closer, as a select of the two words by `triple`.
                        let t = 0u64.wrapping_sub(u64::from(triple));
                        let one = (qm & !b.esc) | b.bare_lf;
                        let three = qm & !b.esc & (qm >> 1 | q1 << 63) & (qm >> 2 | q1 << 62);
                        let close = (three & t) | (one & !t);
                        if format { close | self.braces(&b, &b1, past, rbc).0 } else { close }
                    }
                    F::Interior { .. } => b.sq | b.dq | b.bt | b.hash | b.bs | b.rb | *colon.get_or_insert_with(|| bytes_of(self.src, bi, *b":")),
                    F::Spec => b.lf | b.lb | b.rb,
                } & past;
                if ev == 0 {
                    // Nothing more in the block: what is open is carried.
                    match &mut top {
                        F::Interior { depth } => *depth += (b.ob & past).count_ones() as i32 - (b.cb & past).count_ones() as i32,
                        F::Text { format: true, .. } => rbc = self.braces(&b, &b1, past, rbc).1,
                        _ => {}
                    }
                    p = 0;
                    break;
                }
                let c = ev.trailing_zeros();
                let at = base + c as usize;
                let is = |m: u64| m >> c & 1 == 1;
                match top {
                    F::Code => {
                        if is(b.hash) {
                            stack.push(top);
                            top = F::Comment;
                            open = at;
                            p = c + 1;
                            continue;
                        }
                        let (start, format) = self.prefix(bi, c);
                        self.mark(start);
                        if start < base {
                            self.b[bi - 1 - self.first].code &= !(!0u64 << (start - (base - 64)));
                        }
                        open = start.max(base);
                        let (q, triple) = self.quote(&b, &b1, c);
                        stack.push(top);
                        top = F::Text { q, triple, format };
                        rbc = 0;
                        p = c + if triple { 3 } else { 1 };
                    }
                    F::Comment => {
                        top = stack.pop().expect("code");
                        nc |= from((open - base.min(open)) as u32) & !from(c);
                        p = c + 1;
                    }
                    F::Text { q, triple, .. } => {
                        if !is(quotes(&b, q)) {
                            // Not its closer: a one-line string's bare line end, or a format string's lone `}` or
                            // interior's `{`.
                            if is(b.bare_lf | b.rb) {
                                return Err(refuse(at));
                            }
                            // `\N{...}`: an escape's name, not an interpolation (not read here).
                            let w = win(if bi > 0 { self.b[bi - 1 - self.first].bs & !self.b[bi - 1 - self.first].esc } else { 0 }, b.bs & !b.esc) << 2;
                            if w >> (64 + c) & 1 == 1 {
                                let nm = win(if bi > 0 { bytes_of(self.src, bi - 1, *b"N") } else { 0 }, bytes_of(self.src, bi, *b"N")) << 1;
                                if nm >> (64 + c) & 1 == 1 {
                                    return Err(refuse(at));
                                }
                            }
                            nest += 1;
                            if nest > MAX_NESTING {
                                return Err(refuse(at + 1));
                            }
                            stack.push(top);
                            top = F::Interior { depth: 0 };
                            p = c + 1;
                            continue;
                        }
                        let k = if triple { 3 } else { 1 };
                        self.b[bi - self.first].send |= 1 << c;
                        top = stack.pop().expect("a frame");
                        if top == F::Code {
                            let end = c + k;
                            nc |= from((open - base.min(open)) as u32) & !from(end);
                            if end > 64 {
                                taken = !from(end - 64);
                            }
                        }
                        p = c + k;
                    }
                    F::Interior { depth } => {
                        let span = past & !from(c);
                        let depth = depth + (b.ob & span).count_ones() as i32 - (b.cb & span).count_ones() as i32;
                        top = F::Interior { depth };
                        p = c + 1;
                        if is(b.hash | b.bs) {
                            return Err(refuse(at));
                        }
                        if is(b.rb) {
                            if depth == 0 {
                                nest -= 1;
                                top = stack.pop().expect("a string");
                                rbc = 0;
                            } else {
                                top = F::Interior { depth: depth - 1 };
                            }
                        } else if is(b.sq | b.dq | b.bt) {
                            // A string in the interior: a nested f-string is refused (to tree-sitter).
                            let (start, format) = self.prefix(bi, c);
                            if format {
                                return Err(refuse(at));
                            }
                            self.mark(start);
                            let (q, triple) = self.quote(&b, &b1, c);
                            stack.push(top);
                            top = F::Text { q, triple, format: false };
                            p = c + if triple { 3 } else { 1 };
                        } else if depth == 0 {
                            // A `:` at the interior's depth 0: its format specifier.
                            top = F::Spec;
                        }
                    }
                    F::Spec => {
                        p = c + 1;
                        if is(b.lf) {
                            return Err(refuse(at));
                        }
                        if is(b.rb) {
                            nest -= 1;
                            top = stack.pop().expect("a string");
                            rbc = 0;
                        } else {
                            if (b.lb >> 1 | b1.lb << 63) >> c & 1 == 1 {
                                return Err(refuse(at));
                            }
                            nest += 1;
                            if nest > MAX_NESTING {
                                return Err(refuse(at + 1));
                            }
                            stack.push(top);
                            top = F::Interior { depth: 0 };
                        }
                    }
                }
            }
            if top != F::Code {
                // The outermost region runs past the block.
                nc |= from((open - base.min(open)) as u32);
                open = base + 64;
            }
            self.b[bi - self.first].code = !nc;
        }
        match top {
            F::Code | F::Comment => Ok(()),
            _ => Err(refuse(n)),
        }
    }

    /// The quote at bit `c` of a block (by which mask holds it), and whether it opens a triple (the quote's mask at
    /// the two next bytes).
    #[inline(always)]
    fn quote(&self, b: &B, b1: &B, c: u32) -> (u8, bool) {
        let q = if b.sq >> c & 1 == 1 {
            b'\''
        } else if b.dq >> c & 1 == 1 {
            b'"'
        } else {
            b'`'
        };
        let (qm, q1) = (quotes(b, q), quotes(b1, q));
        (q, ((qm >> 1 | q1 << 63) & (qm >> 2 | q1 << 62)) >> c & 1 == 1)
    }

    /// An f-string's text's brace events from bit `past`'s first: an interior's `{` (the last of an odd run of `{`,
    /// `lbu` past which no `{` follows) and a lone `}` (the odd-run trick over the run of `}` from the text's own
    /// start, so that a run whose first `}` closes an interior pairs from its second; a `}` no `}` pairs with), and
    /// the `}` escape bit carried out.
    #[inline(always)]
    fn braces(&self, b: &B, b1: &B, past: u64, rbc: u64) -> (u64, u64) {
        let open = b.lbu & !(b.lb >> 1 | b1.lb << 63);
        let rbt = b.rb & past;
        let mut carry = rbc;
        let e = escaped(rbt, &mut carry);
        let lone = rbt & !e & !(b.rb >> 1 | b1.rb << 63);
        (open | lone, carry)
    }

    /// Stratum 3: each block's `depth0`.
    #[inline(never)]
    fn depth(&mut self) -> Result<(), Refusal> {
        self.depth_in(0, self.src.len().div_ceil(64), &mut 0)
    }

    /// Stratum 3 over blocks `from..to`, from the depth `carry` into block `from` (left as the depth out of block
    /// `to - 1`).
    #[inline(always)]
    fn depth_in(&mut self, from: usize, to: usize, carry_io: &mut i8) -> Result<(), Refusal> {
        let mut carry = *carry_io;
        let first = self.first;
        for (k, b) in self.b[from - first..to - first].iter_mut().enumerate() {
            let bi = from + k;
            let (up, down) = (b.ob & b.code, b.cb & b.code);
            if up | down == 0 {
                b.depth0 = if carry == 0 { !0 } else { 0 };
                continue;
            }
            let (zero, out, deep) = depth_lanes(up, down, carry);
            if deep {
                return Err(refuse(bi * 64));
            }
            b.depth0 = zero;
            carry = out;
        }
        *carry_io = carry;
        Ok(())
    }
}

/// A machine of three states over a block's 64 bytes, C (0), X (1) and Y (2): at a bit of `ex` C and X swap, at
/// one of `ey` C and Y, at one of `r` X and Y become C. Each byte's function is two bit planes a start state (the
/// state's code after the byte, low bit and high bit), and the functions of the bytes are composed by Kogge–Stone:
/// six rounds, in each every byte's function after the function of the bytes `d` below it, by masks. Out comes
/// each start state's planes of the state after each byte.
#[inline(always)]
fn machine(ex: u64, ey: u64, r: u64) -> [[u64; 2]; 3] {
    let mut f = [[ex, ey], [!(ex | r), 0], [0, !(ey | r)]];
    macro_rules! round {
        ($d:expr) => {{
            let low = !(!0u64 << $d);
            let mut g = [[0u64; 2]; 3];
            for (s, gs) in g.iter_mut().enumerate() {
                // The state after the bytes `d` below, from start state `s` (`s` itself below the block's start).
                let wl = (f[s][0] << $d) | if s == 1 { low } else { 0 };
                let wh = (f[s][1] << $d) | if s == 2 { low } else { 0 };
                let wc = !(wl | wh);
                for k in 0..2 {
                    gs[k] = (wc & f[0][k]) | (wl & f[1][k]) | (wh & f[2][k]);
                }
            }
            f = g;
        }};
    }
    round!(1);
    round!(2);
    round!(4);
    round!(8);
    round!(16);
    round!(32);
    f
}

/// The planes of the state after each byte from start state `s`, and the state after the block's last byte.
#[inline(always)]
fn run_from(f: &[[u64; 2]; 3], s: u8) -> (u64, u64, u8) {
    let [lo, hi] = f[usize::from(s)];
    (lo, hi, ((lo >> 63) | ((hi >> 63) << 1)) as u8)
}

/// Bit `i` the parity of `x`'s bits `0..=i`: six shifted XORs.
#[inline(always)]
fn prefix_xor(x: u64) -> u64 {
    let x = x ^ (x << 1);
    let x = x ^ (x << 2);
    let x = x ^ (x << 4);
    let x = x ^ (x << 8);
    let x = x ^ (x << 16);
    x ^ (x << 32)
}

/// The one-line strings' machine's shortcut: each kind's strings by a prefix XOR of its quotes (simdjson's), where
/// no quote of one kind lies in a string of the other and no string is open at a bare line end; else none.
#[inline(always)]
fn by_parity(ex: u64, ey: u64, r: u64, s: u8) -> Option<(u64, u64, u8)> {
    let lo = prefix_xor(ex) ^ if s == 1 { !0 } else { 0 };
    let hi = prefix_xor(ey) ^ if s == 2 { !0 } else { 0 };
    let (lb, hb) = (lo << 1 | u64::from(s == 1), hi << 1 | u64::from(s == 2));
    if (ex & hb) | (ey & lb) | (r & (lo | hi)) != 0 {
        return None;
    }
    Some((lo, hi, ((lo >> 63) | ((hi >> 63) << 1)) as u8))
}

/// The planes of a state that holds from `s` until the first bit of `r` (where it becomes C), with no other events.
#[inline(always)]
fn hold(s: u8, r: u64) -> (u64, u64, u8) {
    let until = if r == 0 { !0 } else { !(!0u64 << r.trailing_zeros()) };
    let (lo, hi) = (if s == 1 { until } else { 0 }, if s == 2 { until } else { 0 });
    (lo, hi, ((lo >> 63) | ((hi >> 63) << 1)) as u8)
}

/// Variant B's stratum 2's carries into a block: the triples' and the one-line strings' machines' states, a comment's
/// carry, the last bit of the block before's comment line-end mask, and the bytes a triple's closer from the block
/// before takes (bits 0 and 1).
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct RB {
    pub(crate) t0: u8,
    pub(crate) s0: u8,
    pub(crate) ccarry: u64,
    pub(crate) cext_prev: u64,
    pub(crate) tr_taken: u64,
}

impl<'s> Text<'s> {
    /// Stratum 2 with no step inside a block (variant B): each block's comments and strings from a fixed sequence of
    /// bit operations. Three layers, each a machine of `machine`'s, carried from block to block:
    ///
    /// 1. triples: runs of exactly three quotes no backslash escapes, a machine of C and the two triples;
    /// 2. one-line strings: each quote outside the triples swaps C and its string, a bare line end ends one;
    /// 3. comments: from each `#` in C to its line's end, by one addition's carries (generate at the `#`,
    ///    propagate up to the line end).
    ///
    /// Each layer takes what the one below makes as if the one above were not there; where that would be wrong a
    /// mask says so (a triple's opener in a one-line string or a comment; a comment whose line end a backslash
    /// escapes inside a quote it holds; a run of four quotes; a backtick in code; a string open at a line end or
    /// at the text's end), and the text is read by variant A instead: false. A format string is read as a plain
    /// string, its interiors left to the walk, which says where that is wrong (an interior's string of the
    /// enclosing one-line string's quote).
    #[inline(never)]
    fn regions_b(&mut self) -> bool {
        let mut c = RB::default();
        self.regions_b_in(0, self.src.len().div_ceil(64), &mut c) && c.t0 == 0 && c.s0 == 0
    }

    /// Variant B's stratum 2 over blocks `from..to`, from the carries `c` into block `from` (left as those out of block
    /// `to - 1`); the block before `from` is in the window where `from` is not the text's first. False where B's
    /// reading is wrong.
    #[inline(always)]
    fn regions_b_in(&mut self, from: usize, to: usize, c: &mut RB) -> bool {
        let RB { mut t0, mut s0, mut ccarry, mut cext_prev, mut tr_taken } = *c;
        let mut prev = if from > 0 { self.b[from - 1 - self.first] } else { B::default() };
        for bi in from..to {
            let (b, b1) = (self.b[bi - self.first], self.b[bi + 1 - self.first]);
            let (qs, qd) = (b.sq & !b.esc, b.dq & !b.esc);
            let runs = |q: u64, nq: u64, pq: u64| {
                let (q1, q2, q3) = (q >> 1 | nq << 63, q >> 2 | nq << 62, q >> 3 | nq << 61);
                (q & q1 & q2 & !(q << 1 | pq >> 63) & !q3, q & q1 & q2 & q3)
            };
            let (r3s, r4s) = runs(qs, b1.sq & !b1.esc, prev.sq & !prev.esc);
            let (r3d, r4d) = runs(qd, b1.dq & !b1.esc, prev.dq & !prev.esc);
            if r4s | r4d != 0 {
                return false;
            }
            // 1. Triples.
            let r3 = r3s | r3d;
            let (tl, th, t1) = if r3 == 0 { hold(t0, 0) } else { run_from(&machine(r3s, r3d, 0), t0) };
            let ta = tl | th;
            let tb = ta << 1 | u64::from(t0 != 0);
            let t_close = r3 & tb & !ta;
            let t_open = r3 & !tb;
            let tr = tb | ta | t_close << 1 | t_close << 2 | tr_taken;
            tr_taken = t_close >> 62 | t_close >> 63;
            // 2. One-line strings, outside the triples.
            let (ex, ey) = (qs & !tr, qd & !tr);
            let (sl, sh, s1) = if ex | ey == 0 {
                hold(s0, b.bare_lf)
            } else {
                by_parity(ex, ey, b.bare_lf, s0).unwrap_or_else(|| run_from(&machine(ex, ey, b.bare_lf), s0))
            };
            let sa = sl | sh;
            let sb = sa << 1 | u64::from(s0 != 0);
            // 3. Comments: a `#` in C outside the triples generates, every byte but a line end propagates.
            let g = b.hash & !tr & !sb;
            let a = !b.eol | g;
            let (x, o1) = a.overflowing_add(g);
            let (x, o2) = x.overflowing_add(ccarry);
            ccarry = u64::from(o1 | o2);
            let carries = x ^ a ^ g;
            let comment = g | (a & carries);
            // A comment reaches a line end where the byte below it is the comment's (or a `'\r'` just past it).
            let cext = comment | (b.eol & !b.lf & (comment << 1 | cext_prev));
            let lend = cext << 1 | cext_prev;
            cext_prev = cext >> 63;
            // The layers' guesses, checked.
            let wrong = (t_open & (sb | comment))
                | (b.bare_lf & sb & !tr & !lend)
                | (b.lf & b.esc & sb & lend)
                | (b.bt & !b.esc & !tr & !sb & !comment);
            if wrong != 0 {
                return false;
            }
            let s_open = (qs | qd) & !tr & !sb & !comment;
            let s_region = (sb | sa) & !comment & !tr;
            let Some((pre, starts)) = self.prefixes_b(bi, s_open | t_open) else { return false };
            if bi > 0 {
                self.b[bi - 1 - self.first].code &= !(pre as u64);
                self.b[bi - 1 - self.first].sstart |= starts as u64;
            }
            self.b[bi - self.first].code = !(comment | s_region | tr | (pre >> 64) as u64);
            self.b[bi - self.first].sstart = (starts >> 64) as u64;
            self.b[bi - self.first].send = ((qs | qd) & !tr & sb & !sa & !comment) | t_close;
            (t0, s0, prev) = (t1, s1, b);
        }
        *c = RB { t0, s0, ccarry, cext_prev, tr_taken };
        true
    }

    /// The prefixes of the strings that open at `openers` (block `bi`): in a window of the block and the one below
    /// it, the run of prefix letters just below each, by one addition in the window's reverse (carried up a run
    /// from the opener's byte below it, stopped by the first byte not a letter), less the runs that stand on a word
    /// byte (one more addition, from each such run's first byte): the runs' bits, and each string's first byte.
    /// None where a run reaches the window's bottom (rare).
    #[inline(always)]
    fn prefixes_b(&self, bi: usize, openers: u64) -> Option<(u128, u128)> {
        let o = win(0, openers);
        let pw = if bi > 0 { self.b[bi - 1 - self.first].word } else { 0 };
        if (win(pw, self.b[bi - self.first].word) << 1) & o == 0 {
            return Some((0, o));
        }
        let c = prefix_masks(self.src, bi, self.b[bi - self.first].word);
        let p = if bi > 0 { prefix_masks(self.src, bi - 1, pw) } else { Pfx::default() };
        let x = win(p.pfx, c.pfx).reverse_bits();
        let g = (o.reverse_bits() << 1) & x;
        let (sum, over) = x.overflowing_add(g);
        if over {
            return None;
        }
        let run = (x & !sum).reverse_bits();
        let stop = (sum & !x).reverse_bits();
        let bad = ((stop & win(p.ident, c.ident)) << 1) & run;
        let run = run & run.wrapping_add(bad);
        Some((run, (run & !(run << 1)) | (o & !(run << 1))))
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

/// A byte's operator alone and its length 1 (0: none), for every byte.
static OP1L: [(T, u8); 256] = {
    let mut t = [(T::Eof, 0u8); 256];
    let mut c = 0;
    while c < 128 {
        if let Some(k) = op1(c as u8) {
            t[c] = (k, 1);
        }
        c += 1;
    }
    t
};

/// A keyword's slot: its first 8 bytes, its length, its kind and soft keyword.
#[derive(Clone, Copy)]
struct Kw {
    lo: u64,
    len: u8,
    kind: T,
    soft: u8,
}

const KW_BITS: u32 = 8;

/// The keywords' perfect hash: each one's 8 bytes times `KW_MULT`, the top `KW_BITS` bits its slot.
const KW_MULT: u64 = {
    let mut m: u64 = 0x9E37_79B9_7F4A_7C15;
    loop {
        let mut used = [false; 1 << KW_BITS];
        let mut ok = true;
        let mut i = 0;
        while i < lex::WORD_LIST.len() {
            let h = (kw_lo(lex::WORD_LIST[i].0).wrapping_mul(m) >> (64 - KW_BITS)) as usize;
            if used[h] {
                ok = false;
                break;
            }
            used[h] = true;
            i += 1;
        }
        if ok {
            break m;
        }
        m = m.wrapping_add(0x6A09_E667_F3BC_C909) | 1;
    }
};

const fn kw_lo(w: &[u8]) -> u64 {
    let mut x = 0u64;
    let mut i = 0;
    while i < w.len() && i < 8 {
        x |= (w[i] as u64) << (8 * i);
        i += 1;
    }
    x
}

static KWS: [Kw; 1 << KW_BITS] = {
    let mut t = [Kw { lo: 0, len: 0, kind: T::Name, soft: 0 }; 1 << KW_BITS];
    let mut i = 0;
    while i < lex::WORD_LIST.len() {
        let (w, kind, soft) = lex::WORD_LIST[i];
        assert!(w.len() <= 8, "a keyword past 8 bytes");
        t[(kw_lo(w).wrapping_mul(KW_MULT) >> (64 - KW_BITS)) as usize] = Kw { lo: kw_lo(w), len: w.len() as u8, kind, soft };
        i += 1;
    }
    t
};

/// The name `src[i..i + len]`'s kind and soft keyword (`lex::keyword`'s) without a branch: its first 8 bytes, one
/// probe of the perfect hash, and the slot's bytes and length compared (a selection, not a branch).
#[inline(always)]
fn keyword_at(src: &[u8], i: usize, len: usize) -> (T, u8) {
    let mask = if len >= 8 { !0u64 } else { (1u64 << (8 * len)) - 1 };
    let lo = rerail_simd8::swar::load8(src, i) & mask;
    let k = KWS[(lo.wrapping_mul(KW_MULT) >> (64 - KW_BITS)) as usize];
    // The slot's word or a name, by a mask of the comparison: no branch.
    let hit = 0u8.wrapping_sub(u8::from(k.lo == lo) & u8::from(usize::from(k.len) == len));
    let kind = (k.kind as u8 & hit) | (T::Name as u8 & !hit);
    // SAFETY: `kind` is the discriminant of `k.kind` or of `T::Name` (`T` is `repr(u8)`).
    (unsafe { core::mem::transmute::<u8, T>(kind) }, k.soft & hit)
}

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

/// The indents open, as the walk reads them: the top against a line's indent. The text's walk keeps them (a `Vec`); a
/// chunk's summary keeps its own above an entry it does not know (`middle::Sym`).
pub(crate) trait Stack {
    /// Whether the top is below `w`: a header's next line indented past it opens a level.
    fn below(&mut self, w: u32) -> bool;
    /// Whether the top is above `m`, for a line's dedent: the layout loop pops it (`pop`) where it is.
    fn pops_above(&mut self, m: i64) -> bool;
    /// Whether the top is above `m`, for a line in brackets (a dedent there is refused).
    fn above(&mut self, m: i64) -> bool;
    fn push(&mut self, w: u32);
    fn pop(&mut self);
}

impl Stack for Vec<u32> {
    #[inline(always)]
    fn below(&mut self, w: u32) -> bool {
        *self.last().expect("an indent") < w
    }

    #[inline(always)]
    fn pops_above(&mut self, m: i64) -> bool {
        self.above(m)
    }

    #[inline(always)]
    fn above(&mut self, m: i64) -> bool {
        i64::from(*self.last().expect("an indent")) > m
    }

    #[inline(always)]
    fn push(&mut self, w: u32) {
        Vec::push(self, w);
    }

    #[inline(always)]
    fn pop(&mut self) {
        Vec::pop(self);
    }
}

/// Stratum 4: the walk that writes the tokens.
struct Walk<'s, S: Stack = Vec<u32>> {
    t: Masks<'s>,
    toks: Vec<Tok>,
    extras: Vec<Extra>,
    /// Where the last token or extra ended.
    p: usize,
    /// Interpolations nested.
    depth: u32,
    indents: S,
    /// As the lean lexer's (the scanner's flag).
    inside_interpolated_string: bool,
    /// The format string being read (its quote, whether a triple), while its interiors are.
    outer: Option<(u8, bool)>,
    /// Whether stratum 2 was variant B's, which reads format strings as plain ones: where an interior holds a string
    /// of the enclosing one-line string's quote, that reading is wrong, and the walk says so (`b_wrong`).
    b: bool,
    b_wrong: bool,
    /// The walk stops at the first item that starts from `stop` on (a chunk's end, lexed from the middle).
    stop: usize,
    /// Whether the end of the text's token is read.
    done: bool,
}

/// Which stratum 2 a lexer of strata reads with: the plan's step, block by block (A), or no step inside a block
/// (B).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    A,
    B,
}

/// `src` lexed by the strata (the lean lexer's `lex`), stratum 2 by `v`. Where variant B's layers or the walk say B's reading is wrong
/// (rare), the text is read by A.
pub(crate) fn lex_with(src: &[u8], v: Variant) -> Result<Lexed, Refusal> {
    let mut t = Text { src, b: classes(src), first: 0 };
    let b = v == Variant::B && t.regions_b();
    if !b {
        if v == Variant::B {
            t = Text { src, b: classes(src), first: 0 };
        }
        t.regions()?;
    }
    t.depth()?;
    let short = core::cell::Cell::new(false);
    let mut w = Walk { t: t.m(&short), toks: Vec::with_capacity(src.len() / 3 + 16), extras: Vec::new(), p: 0, depth: 0, indents: vec![0], inside_interpolated_string: false, outer: None, b, b_wrong: false, stop: usize::MAX, done: false };
    match w.module(false, false).map(|_| ()) {
        Err(_) if w.b_wrong => lex_with(src, Variant::A),
        Err(r) => Err(r),
        Ok(()) => Ok(Lexed { toks: w.toks, extras: w.extras }),
    }
}

/// Whether variant B reads `src` itself, not by A (for a measurement of how often it does not).
pub fn b_reads(src: &[u8]) -> bool {
    let mut t = Text { src, b: classes(src), first: 0 };
    if !t.regions_b() || t.depth().is_err() {
        return false;
    }
    let short = core::cell::Cell::new(false);
    let mut w = Walk { t: t.m(&short), toks: Vec::new(), extras: Vec::new(), p: 0, depth: 0, indents: vec![0], inside_interpolated_string: false, outer: None, b: true, b_wrong: false, stop: usize::MAX, done: false };
    w.module(false, false).is_ok() || !w.b_wrong
}

impl<'s, S: Stack> Walk<'s, S> {
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
            // A chunk's walk that read past its window reads nothing more (it is made again over a longer one).
            if self.t.short.get() {
                return Ok(None);
            }
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
    fn module(&mut self, mut open: bool, mut header: bool) -> Result<(bool, bool), Refusal> {
        let n = self.t.src.len();
        let src = self.t.src;
        loop {
            // A chunk's walk stops at its end, or where it read past its window (made again over a longer one).
            if self.p >= self.stop || self.t.short.get() {
                return Ok((open, header));
            }
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
                            let p = self.p;
                            if header && self.indents.below(indent) {
                                self.indents.push(indent);
                                self.push(T::Indent, p, p);
                                open = false;
                                header = false;
                                break;
                            } else if open {
                                self.push(T::Newline, p, p);
                                open = false;
                                header = false;
                            } else if self.indents.pops_above(i64::from(indent).max(first_comment)) {
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
                self.done = true;
                return Ok((open, header));
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
        let mut last = (T::Eof, k);
        // Block by block while no stop is met: a run goes on into the next block (from the last token's end, which
        // may lie in it).
        let mut at = k;
        'blocks: loop {
            let base = at & !63;
            if base >= n {
                break;
            }
            let b = self.t.blk(base >> 6);
            let q0 = (at & 63) as u32;
            let valid = if n - base >= 64 { !0u64 } else { (1u64 << (n - base)) - 1 };
            let ff = b.skip & !b.sptab & !b.lf;
            let stop = (!b.code | b.eol | ff | b.bs | !valid) & (!0u64 << q0);
            let before_stop = if stop == 0 { !0u64 } else { stop.isolate_lowest_one() - 1 };
            let mut starts = ((!b.skip & !(b.word & (b.word << 1)) & (!0u64 << q0)) | (u64::from(at == k) << q0)) & before_stop;
            // The next block: the rest of a name that reaches this one's end.
            let nb = self.t.blk((base >> 6) + 1);
            // Each token's start is its bit of `starts`, and no token waits on the one before: a name ends at its run
            // of word bits, an operator after its one byte (one of more bytes is at a `slow` bit). Which of the two it
            // is, and a name's keyword, are selections, not branches.
            while starts != 0 {
                let q = starts.trailing_zeros();
                starts &= starts - 1;
                let pos = base + q as usize;
                let c = src[pos];
                // A name's run of word bits (none for an operator's byte), on into the next block's where it reaches
                // this one's end.
                let w0 = (!(b.word >> q)).trailing_zeros() as usize;
                let cross = q as usize + w0 == 64;
                let w1 = (!nb.word).trailing_zeros() as usize * usize::from(cross);
                let words = w0 + w1;
                // The byte after the name: its `slow` bit, in this block or the next.
                let r = b.slow >> q;
                let after = core::hint::select_unpredictable(cross, nb.slow >> w1.min(63), r >> w0.min(63));
                // The rare tokens, one branch: a number (a digit, or a `.` before one), a quote, a byte past ASCII, an
                // operator of more than one byte, a name to a `slow` byte, and one past the next block too.
                let rare = r | (u64::from(c == b'.') & (r >> 1)) | after | u64::from(w1 == 64);
                if rare & 1 != 0 {
                    let end = match CLASS[usize::from(c)] {
                        C_NAME => {
                            let end = self.name_end(pos)?;
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
                        C_DOT | C_OP => {
                            let (kind, end) = if c == b'.' && self.at(pos + 1).is_ascii_digit() {
                                lex::number_at(src, pos)
                            } else {
                                let (kind, len) = operator(c, self.at(pos + 1), self.at(pos + 2)).ok_or_else(|| refuse(pos))?;
                                (kind, pos + len)
                            };
                            self.toks.push(Tok { kind, soft: 0, start: pos as u32, end: end as u32 });
                            last = (kind, pos);
                            end
                        }
                        // A byte past ASCII (an odd blank, a name's character) or a quote: the walk's own path.
                        _ => break 'blocks,
                    };
                    self.p = end;
                    let e = end - base;
                    if e >= 64 {
                        break;
                    }
                    starts &= !0u64 << e;
                    continue;
                }
                let (wk, ws) = keyword_at(src, pos, words);
                let (ok, ol) = OP1L[usize::from(c)];
                let name = (b.word >> q) & 1 != 0;
                // An operator's byte with no operator: one test of both, so that it is not a branch on which of the
                // two the token is.
                if u8::from(!name) & u8::from(ol == 0) != 0 {
                    return Err(refuse(pos));
                }
                let (kind, soft, end) = core::hint::select_unpredictable(name, (wk, ws, pos + words), (ok, 0, pos + 1));
                self.toks.push(Tok { kind, soft, start: pos as u32, end: end as u32 });
                last = (kind, pos);
                self.p = end;
            }
            if stop != 0 {
                break;
            }
            at = self.p.max(base + 64);
        }
        Ok(last)
    }

    /// The lean lexer's `dedent_in_brackets`.
    fn dedent_in_brackets(&mut self, p: usize) -> Result<bool, Refusal> {
        if self.inside_interpolated_string || self.closer_may_follow() {
            return Ok(false);
        }
        Ok(match self.scan(p)? {
            Some((indent, first_comment, next)) => !matches!(self.at(next), b'\'' | b'"' | b'`') && self.indents.above(i64::from(indent).max(first_comment)),
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
                // A string's prefix only where stratum 2 found a string's start (or, in an interior, where the name's
                // run of prefix letters reaches a quote): a name is read at once.
                if (self.t.bit(i, |b| b.sstart) || (self.depth > 0 && self.prefix_to_quote(i)))
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

    /// Whether the name at `i` (in an interior) is a string's prefix, by masks: its run of word bytes ends at a quote,
    /// and each byte of the run is a prefix letter.
    fn prefix_to_quote(&self, i: usize) -> bool {
        let e = self.t.run_end(i, |b| b.word);
        if !self.t.bit(e, |b| b.sq | b.dq | b.bt) || e - i > 64 {
            return false;
        }
        let src = self.t.src;
        let (bi, be) = (i >> 6, e >> 6);
        let x = win(prefix_masks(src, bi, self.t.blk(bi).word).pfx, if be > bi { prefix_masks(src, be, self.t.blk(be).word).pfx } else { 0 });
        let run = !(!0u128 << (e - i)) << (i & 63);
        x & run == run
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
        if self.depth > 0 {
            // A string in an interior: a format string there is refused (to tree-sitter), and one of the enclosing
            // one-line string's quote is where B's plain reading of that string is wrong.
            if q.format {
                return Err(refuse(i));
            }
            if self.b && self.outer.is_some_and(|(oq, triple)| !triple && oq == q.quote) {
                self.b_wrong = true;
                return Err(refuse(i));
            }
        } else if q.format {
            self.outer = Some((q.quote, q.triple));
        }
        self.push(T::StrStart, i, i + open);
        self.inside_interpolated_string = q.format;
        let mut j = i + open;
        let mut text = j;
        let mut content = false;
        let (quote, format) = (q.quote, q.format);
        // The string's closer is stratum 2's (`send`), so its text stops only at its backslashes and a format
        // string's braces: not at each quote and line end of it. Variant B finds no closer of a string in an
        // interior, which is read by its quotes and line ends.
        let known = !(self.b && self.depth > 0);
        loop {
            let s = self.t.next(j, |b| if known { b.send } else { quotes(b, quote) | b.lf } | b.bs | if format { b.lb | b.rb } else { 0 });
            if s >= src.len() {
                return Err(refuse(s));
            }
            content |= s > j;
            let c = src[s];
            if c == q.quote {
                if known || !q.triple || (self.at(s + 1) == q.quote && self.at(s + 2) == q.quote) {
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
            // A chunk's walk that read past its window reads nothing more (it is made again over a longer one).
            if self.t.short.get() {
                return Err(refuse(i));
            }
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
            let s = self.t.next_in(i, |_, b| b.lf | b.lb | b.rb);
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

/// The lexer of strata (variant A) against the lean lexer on `src`.
pub fn check(src: &[u8]) -> Check {
    check_with(src, Variant::A)
}

/// The lexer of strata, stratum 2 by `v`, against the lean lexer on `src`.
pub fn check_with(src: &[u8], v: Variant) -> Check {
    let at = |r: Refusal| match r {
        Refusal::Lex(at) | Refusal::Parse(at) => at,
        Refusal::TooLarge => u32::MAX,
    };
    match (lex::lex(src), lex_with(src, v)) {
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

/// The strata up to `upto` alone (1: classes, 2: regions, 3: depth, 4: the whole lexer), stratum 2 by `v`, for a
/// measurement of each: a sum that keeps the work.
pub fn upto(src: &[u8], upto: u8, v: Variant) -> usize {
    let mut t = Text { src, b: classes(src), first: 0 };
    if upto >= 2 && v == Variant::B && t.regions_b() {
    } else if upto >= 2 && t.regions().is_err() {
        return 0;
    }
    if upto >= 3 && t.depth().is_err() {
        return 0;
    }
    if upto >= 4 {
        return token_count_with(src, v).unwrap_or(0);
    }
    t.b.iter().map(|b| (b.code ^ b.depth0 ^ b.word) as usize).fold(0, usize::wrapping_add)
}

/// How many tokens and extras the strata lex `src` to (the lexer alone, for a measurement).
pub fn token_count(src: &[u8]) -> Result<usize, Refusal> {
    token_count_with(src, Variant::A)
}

/// How many tokens and extras the strata lex `src` to, stratum 2 by `v`.
pub fn token_count_with(src: &[u8], v: Variant) -> Result<usize, Refusal> {
    let l = lex_with(src, v)?;
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
            let mut t = Text { src: &src, b: classes(&src), first: 0 };
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
                assert_eq!(t.m(&core::cell::Cell::new(false)).bit(i, |b| b.depth0), w == 0, "byte {i}");
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
        for v in [Variant::A, Variant::B] {
            let bad: Vec<_> = texts.iter().map(|t| (t, check_with(t.as_bytes(), v))).filter(|(_, c)| *c != Check::Same).collect();
            assert!(bad.is_empty(), "{v:?}: {bad:#?}");
        }
        // A text across many blocks: each region and bracket crossing a block's edge somewhere.
        let mut long = String::new();
        for k in 0..40 {
            long.push_str(&format!("x{k} = f(a, '{}', \"\"\"q\n{}\"\"\", # c ( {k}\n  [{}])  # e\n", "s".repeat(k), "t".repeat(k * 3), "1, ".repeat(k)));
        }
        assert_eq!(check(long.as_bytes()), Check::Same);
        assert_eq!(check_with(long.as_bytes(), Variant::B), Check::Same);
        assert!(b_reads(long.as_bytes()));
    }

    /// Variant B's layers on texts of quotes, comments and line ends, against the lean lexer: the same tokens, or
    /// both refusing, whether B reads the text itself or A does.
    #[test]
    fn variant_b_is_the_lean_lexer() {
        let mut x = 0x2545_f491_4f6c_dd1du64;
        let (mut own, mut all) = (0, 0);
        for len in [10usize, 63, 64, 65, 130, 400] {
            for _ in 0..300 {
                let src: Vec<u8> = (0..len)
                    .map(|_| {
                        x ^= x << 13;
                        x ^= x >> 7;
                        x ^= x << 17;
                        b"''''\\\"\"\"\"#\n\n  ab(rf{}x)"[(x >> 8) as usize % 23]
                    })
                    .collect();
                let (a, b) = (check_with(&src, Variant::A), check_with(&src, Variant::B));
                let ok = |c: &Check| matches!(c, Check::Same | Check::BothRefuse | Check::StrataRefuses(_));
                assert!(ok(&b) && (a == b || matches!((&a, &b), (Check::StrataRefuses(_), Check::StrataRefuses(_)))), "{:?}: A {a:?}, B {b:?}", String::from_utf8_lossy(&src));
                if a == Check::Same {
                    all += 1;
                    own += usize::from(b_reads(&src));
                }
            }
        }
        // Of the texts the lean lexer reads, B reads most itself.
        assert!(own * 2 > all, "B read {own} of {all} itself");
    }

    /// The three-state machine against the same machine stepped byte by byte.
    #[test]
    fn the_machine_is_the_stepped_machine() {
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        let mut r = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for _ in 0..2000 {
            let (a, b2, c, d, e) = (r(), r(), r(), r(), r());
            let (ex, ey, rs) = (a & b2 & !c, a & !b2 & c, !a & b2 & d & e);
            let f = machine(ex, ey, rs);
            for s0 in 0..3u8 {
                let (lo, hi, last) = run_from(&f, s0);
                let mut s = s0;
                for i in 0..64 {
                    let bit = |m: u64| m >> i & 1 == 1;
                    s = match s {
                        0 if bit(ex) => 1,
                        0 if bit(ey) => 2,
                        1 if bit(ex) || bit(rs) => 0,
                        2 if bit(ey) || bit(rs) => 0,
                        _ => s,
                    };
                    assert_eq!((lo >> i & 1) as u8 | ((hi >> i & 1) as u8) << 1, s, "from {s0}, byte {i}");
                }
                assert_eq!(last, s);
            }
        }
    }

    #[test]
    fn operators_are_the_lean_lexers() {
        let bytes = b"*/<>=!:+-@%&^|.()[]{},;~ax";
        for &a in bytes {
            for &b in bytes {
                for &c in bytes {
                    let s = [a, b, c];
                    assert_eq!(operator(a, b, c).map(|(t, n)| (t as u8, n)), lex::operator_at(&s, 0).map(|(t, n)| (t as u8, n)), "{:?}", String::from_utf8_lossy(&s));
                    let (t, n) = OP1L[usize::from(a)];
                    assert_eq!((n > 0).then_some((t as u8, 1)), operator(a, 0, 0).map(|(t, n)| (t as u8, n)), "{:?}", String::from_utf8_lossy(&s));
                }
            }
        }
    }

    #[test]
    fn keywords_by_the_perfect_hash_are_the_switchs() {
        let mut names: Vec<Vec<u8>> = lex::WORD_LIST.iter().map(|w| w.0.to_vec()).collect();
        for w in lex::WORD_LIST {
            names.push([w.0, b"x"].concat());
            names.push(w.0[..w.0.len() - 1].to_vec());
            names.push([b"_", w.0].concat());
        }
        names.extend([b"x".to_vec(), b"continue_".to_vec(), b"continuex".to_vec(), b"__init__".to_vec(), b"self".to_vec()]);
        for w in names.iter().filter(|w| !w.is_empty()) {
            // The name in a text, and at its end.
            for src in [[w.as_slice(), b" = 1"].concat(), w.clone()] {
                let (k, s) = keyword_at(&src, 0, w.len());
                assert_eq!((k as u8, s), { let (k, s) = lex::keyword(&src, 0, w.len()); (k as u8, s) }, "{:?}", String::from_utf8_lossy(w));
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
