//! The walk's token starts by masks (decision 0698, item 102): the bytes the serial walk starts a token at in code,
//! made from stratum 1 and 2's masks and the block's bytes with no branch, so that the walk's pass 1 counts a chunk's
//! records as the popcount of its starts.
//!
//! A token in code is a name (a run of word bytes), a number (`lex::number_at`'s rules), or an operator (the longest
//! of `operator`'s). Each is found for every byte of a block at once:
//!
//! - **Operators:** the longest operator at each byte, by masks of the byte and the two after it, and the walk's
//!   choice among overlapping ones by a machine of three states (the next byte starts a token, or one or two more
//!   bytes are the token's), composed by Kogge–Stone as B's machine is.
//! - **Numbers:** a digit's run of `[0-9]` with single underscores, a fraction after a `.`, an exponent and its sign,
//!   a suffix: runs by carry additions, a fixed number of steps. A number takes the bytes it reads: a `.`, a sign,
//!   and a word run's start after them.
//! - **Names:** each other word run's start.
//!
//! The block is read in a window of it and the block below. Each choice depends only on the run of non-blank code
//! it lies in, so a block is exact unless the run its window starts in reaches the block: then it is rare (not known),
//! as is a block with a byte the walk refuses or reads by its own path (past ASCII).

use super::{B, Masks, OP_EQ, OP_TWICE, OP1, V, bytes_from};
use rerail_simd::prelude::*;

/// The operator classes of a byte below 128: one bit each.
const C_OPEQ: u8 = 1;
const C_TWICE: u8 = 2;
const C_OP1: u8 = 4;

/// Each byte's operator classes, the low and the high half of ASCII (a table lookup each).
static OPC: [[u8; 64]; 2] = {
    let mut t = [[0u8; 64]; 2];
    let mut c = 0;
    while c < 128 {
        let mut x = 0;
        if OP_EQ[c].is_some() {
            x |= C_OPEQ;
        }
        if OP_TWICE[c].is_some() {
            x |= C_TWICE;
        }
        if OP1[c].is_some() {
            x |= C_OP1;
        }
        t[c / 64][c % 64] = x;
        c += 1;
    }
    t
};

/// A block's byte classes the starts read, from its bytes and the two after each.
#[derive(Clone, Copy, Default)]
struct Bytes {
    digit: u64,
    dot: u64,
    exp: u64,
    sign: u64,
    under: u64,
    suffix: u64,
    zero: u64,
    letter_x: u64,
    letter_o: u64,
    letter_b: u64,
    hex: u64,
    oct: u64,
    bin: u64,
    long: u64,
    high: u64,
    /// The operator at the byte is three bytes, two, one; or none.
    len3: u64,
    len2: u64,
    len1: u64,
}

#[inline(always)]
fn classes(src: &[u8], at: usize) -> Bytes {
    let (v, v1, v2) = (bytes_from(src, at), bytes_from(src, at + 1), bytes_from(src, at + 2));
    let s = V::splat;
    let eq = |v: V, x: u8| v.simd_eq(s(x)).to_bitmask();
    let opc = V::from_array(OPC[0]).swizzle_dyn(v) | V::from_array(OPC[1]).swizzle_dyn(v - s(64));
    let has = |c: u8| (opc & s(c)).simd_ne(s(0)).to_bitmask();
    let (eq1, eq2) = (eq(v1, b'='), eq(v2, b'='));
    let same1 = v.simd_eq(v1).to_bitmask();
    let dot = eq(v, b'.');
    let c1 = has(C_OPEQ) & eq1;
    let c2 = !eq1 & same1 & has(C_TWICE);
    let ellipsis = !eq1 & same1 & !has(C_TWICE) & dot & eq(v2, b'.');
    let arrow = !eq1 & !same1 & eq(v1, b'>') & (eq(v, b'-') | eq(v, b'<'));
    let len3 = (c2 & eq2) | ellipsis;
    let len2 = c1 | (c2 & !eq2) | arrow;
    let len1 = has(C_OP1) & !len2 & !len3;
    let low = v | s(0x20);
    Bytes {
        digit: (v - s(b'0')).simd_lt(s(10)).to_bitmask(),
        dot,
        exp: eq(low, b'e'),
        sign: eq(v, b'+') | eq(v, b'-'),
        under: eq(v, b'_'),
        suffix: eq(low, b'j') | eq(low, b'l'),
        zero: eq(v, b'0'),
        letter_x: eq(low, b'x'),
        letter_o: eq(low, b'o'),
        letter_b: eq(low, b'b'),
        hex: (v - s(b'0')).simd_lt(s(10)).to_bitmask() | (low - s(b'a')).simd_lt(s(6)).to_bitmask(),
        oct: (v - s(b'0')).simd_lt(s(8)).to_bitmask(),
        bin: (v - s(b'0')).simd_lt(s(2)).to_bitmask(),
        long: eq(low, b'l'),
        high: v.simd_ge(s(0x80)).to_bitmask(),
        len3,
        len2,
        len1,
    }
}

/// A window of two blocks' masks, the one below `bi` in the low half.
#[inline(always)]
fn w(lo: u64, hi: u64) -> u128 {
    (u128::from(hi) << 64) | u128::from(lo)
}

/// The state after each byte from each start state of a machine of three states, each byte's function given as
/// the state after it from each state (two planes): Kogge–Stone's seven rounds over 128 bytes, by masks.
#[inline(always)]
fn ks3(mut f: [[u128; 2]; 3]) -> [[u128; 2]; 3] {
    macro_rules! round {
        ($d:expr) => {{
            let low = !(!0u128 << $d);
            let mut g = [[0u128; 2]; 3];
            for (s, gs) in g.iter_mut().enumerate() {
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
    round!(64);
    f
}

/// The ends of the runs of `x` from each of `starts` (a start not in `x` is its own end): one addition.
#[inline(always)]
fn run_ends(starts: u128, x: u128) -> u128 {
    (x.wrapping_add(starts & x) & !x) | (starts & !x)
}

/// Positions from which a forward walk through `through` bytes ends at one of `hit` (and `hit` itself): the runs of
/// `through` just below each hit, by one addition in the window's reverse.
#[inline(always)]
fn reaching(hit: u128, through: u128) -> u128 {
    let (h, t) = (hit.reverse_bits(), through.reverse_bits());
    let q = (h << 1) & t;
    (h | ((t.wrapping_add(q) ^ t) & t)).reverse_bits()
}

/// Block `bi`'s token starts in code (the serial walk's, its layout tokens and a string's aside), and whether the
/// block is rare (its starts not known by masks).
pub(super) fn code_starts(t: &Masks<'_>, bi: usize) -> (u64, bool) {
    let src = t.src;
    // The block below (none below the text's start: masked).
    let (pb, b) = (*t.blk(bi.saturating_sub(1)), *t.blk(bi));
    let keep = 0u64.wrapping_sub(u64::from(bi > 0));
    let at = |d: usize| (classes(src, (64 * bi.saturating_sub(1)) + d), classes(src, 64 * bi + d));
    let (c0, c1, c2, c3) = (at(0), at(1), at(2), at(3));
    let m = |f: fn(&B) -> u64| w(f(&pb) & keep, f(&b));
    let k = |f: fn(&Bytes) -> u64| w(f(&c0.0) & keep, f(&c0.1));
    // A class of the byte one, two, three after each (read past the window's top, not shifted in).
    let n1 = |f: fn(&Bytes) -> u64| w(f(&c1.0) & keep, f(&c1.1));
    let n2 = |f: fn(&Bytes) -> u64| w(f(&c2.0) & keep, f(&c2.1));
    let n3 = |f: fn(&Bytes) -> u64| w(f(&c3.0) & keep, f(&c3.1));
    // The text's bytes (none past its end).
    // The window's bit `j` is byte `64 * (bi - 1) + j`.
    let lim = (src.len() + 64).saturating_sub(64 * bi).min(128) as u32;
    let valid = ((1u128 << (lim & 127)) - 1) | 0u128.wrapping_sub(u128::from(lim >> 7));
    // The bytes of code a token is read in: not a blank, a line's end or a backslash (a continuation's).
    let nb = m(|b| b.code & !b.skip & !b.eol & !b.bs) & valid;
    // Where the walk's run of tokens stops: not code, a line's end, a form feed, a backslash, the text's end.
    let stop = !m(|b| b.code) | m(|b| b.eol) | m(|b| b.skip & !b.sptab & !b.lf) | m(|b| b.bs) | !valid;
    let word = m(|b| b.word) & nb;
    let high = k(|c| c.high) & nb;
    let op = nb & !word & !high;
    let (len3, len2, len1) = (k(|c| c.len3) & op, k(|c| c.len2) & op, k(|c| c.len1) & op);
    // A byte of code with no operator: the walk refuses it.
    let refused = op & !(len1 | len2 | len3);
    // Operators: from "a token starts here", the state after the byte is its length less one; from "one more
    // byte", a token starts after it; from "two more", one more after it.
    let f = ks3([[len2, len3], [0, 0], [op, 0]]);
    let after0 = !(f[0][0] | f[0][1]);
    let at0 = (after0 << 1) | 1;
    let op_starts = op & at0;
    // Numbers. A digit's run: digits with single underscores between and after them.
    let digit = k(|c| c.digit) & nb;
    let under = k(|c| c.under) & nb;
    let x = digit | (under & (digit << 1));
    let run_start = word & !(word << 1);
    let dot = k(|c| c.dot) & nb;
    // A digit's run after any `.` that starts a token (a number's fraction, or a number that starts at the `.`) is
    // the number's.
    let after_dot = (dot & op_starts) << 1;
    let digit_starts = run_start & digit & !after_dot;
    let (exp, suffix, zero, long) = (k(|c| c.exp), k(|c| c.suffix), k(|c| c.zero), k(|c| c.long));
    let (sign1, digit1, digit2, dot1) = (n1(|c| c.sign), n1(|c| c.digit), n2(|c| c.digit), n1(|c| c.dot));
    let (lx1, lo1, lb1, under2) = (n1(|c| c.letter_x), n1(|c| c.letter_o), n1(|c| c.letter_b), n2(|c| c.under));
    // Each base's digits, at the byte and one, two, three after it.
    let hex = [k(|c| c.hex), n1(|c| c.hex), n2(|c| c.hex), n3(|c| c.hex)];
    let oct = [k(|c| c.oct), n1(|c| c.oct), n2(|c| c.oct), n3(|c| c.oct)];
    let bin = [k(|c| c.bin), n1(|c| c.bin), n2(|c| c.bin), n3(|c| c.bin)];
    // The numbers from the digit runs' starts `ds`: where each ends, the bytes of operators and the word runs'
    // starts they take, and the `.`s that start numbers.
    let numbers_from = |ds: u128| {
        // A number of base 16, 8 or 2 (`0` and its letter) has no fraction or exponent.
        let zero_letter = ds & zero;
        let (kx, ko, kb) = (zero_letter & lx1, zero_letter & lo1, zero_letter & lb1);
        let dec = ds & !(kx | ko | kb);
        // Its int part's end, a fraction after it, and the fraction's end.
        let k1 = run_ends(dec, x);
        let frac = k1 & dot;
        // A `.` before a digit that is not a number's fraction starts a number.
        let dot_starts = dot & op_starts & digit1 & !frac;
        let k2 = (k1 & !frac) | run_ends(frac << 1, x) | run_ends(dot_starts << 1, x);
        // An exponent: `e` and a digit, or `e`, a sign and a digit. Its sign and its digits' run are the number's.
        let e = k2 & exp;
        let e_digit = e & digit1;
        let e_sign = e & !e_digit & sign1 & digit2;
        let k3 = (k2 & !(e_digit | e_sign)) | run_ends(e_digit << 1, x) | run_ends(e_sign << 2, x);
        // A suffix where the number ends.
        let sfx = k3 & suffix;
        let dec_end = (k3 & !sfx) | (sfx << 1);
        // A number of base 16, 8 or 2: its digits from after the letter (and one underscore there), with single
        // underscores before digits, then an `l`; with no digit there, the number is its `0`.
        let based_end = |kind: u128, ok: [u128; 4]| {
            let u2 = kind & under2;
            let has = kind & ((ok[2] & !u2) | (ok[3] & u2));
            let first = ((has & !u2) << 2) | ((has & u2) << 3);
            let end = run_ends(first, ok[0] | (under & ok[1]));
            let l = end & long;
            (end & !l) | (l << 1) | ((kind & !has) << 1)
        };
        let ends = dec_end | based_end(kx, hex) | based_end(ko, oct) | based_end(kb, bin);
        // The word runs' starts a number takes: its exponent's `e` and its suffix just after a fraction's `.` (a
        // run's start there), and its signed exponent's digits.
        let taken = ((e_digit | e_sign | sfx) & run_start & (frac << 1 | dot_starts << 1)) | (e_sign << 2);
        (ends, frac, dot_starts, e_sign, taken)
    };
    // A signed exponent's digits are a word run's start but the number's: the numbers again without them (where
    // that takes others' again, rare).
    let first = numbers_from(digit_starts);
    let (ends, frac, dot_starts, e_sign, taken) = numbers_from(digit_starts & !(first.3 << 2));
    let exp_again = (e_sign << 2) != (first.3 << 2);
    // The operators' bytes a number takes: a fraction's `.` and an exponent's sign.
    let op_taken = frac | (e_sign << 1);
    let names = run_start & !digit & !taken;
    let numbers = (digit_starts & !taken) | dot_starts;
    let starts = names | numbers | (op_starts & !op_taken & !dot_starts);
    // A number that ends inside a word run: the rest is a name where the walk's run stops before any other token
    // starts (the walk's next run starts there), else the walk passes over it.
    let rest = ends & word & !run_start;
    let through = !(starts | stop) & valid;
    // Where the rest starts a block, the walk's next block reads its first byte as a run's start.
    let rest_starts = rest & (reaching(stop, through) | (1u128 << 64));
    let starts = starts | rest_starts;
    // Rare: a refused byte or one past ASCII in the block; the run of non-blank code the window starts in reaches
    // the block (what it starts with is below the window); a number's rest of a word run that reaches a block's
    // end, or whose walk to a stop or a start leaves the window.
    let first_run = run_ends(nb & 1, nb);
    let reaches = first_run >= (1u128 << 64);
    let rest_word = rest | ((word.wrapping_add((rest << 1) & word) ^ word) & word);
    let crosses = ((rest_word >> 63) & 3 == 3) | (rest_word >> 127 != 0);
    let rest_far = crosses | ((reaching(starts | stop, through) & rest & !(1u128 << 64)) != rest & !(1u128 << 64));
    // A `.` that starts a token at a block's last byte, before a digit: the walk reads it as an operator (its test
    // of the byte after is the block's).
    let edge = (1u128 << 63) | (1u128 << 127);
    let dot_edge = dot & op_starts & !frac & edge & digit1 != 0;
    // A fraction's `.` before another `.`: the operators' machine reads the dots from the fraction's.
    // A rest that starts with a digit is a number again, with a rest of its own.
    let frac_dots = (frac & dot1 != 0) | exp_again | (rest & digit != 0);
    let rare = (((refused | high) >> 64) != 0) | ((nb & 1 != 0) & reaches) | rest_far | dot_edge | frac_dots;
    ((starts >> 64) as u64, rare)
}

/// The token starts by masks against the serial walk's (variant B's), block by block on `src`: the blocks whose starts
/// in code are the walk's, the rare ones, and the first block that differs (none where B does not read the text).
pub fn starts_are_the_walks(src: &[u8]) -> Option<(usize, usize, Option<String>)> {
    let mut t = super::Text { src, b: super::classes(src), first: 0 };
    if !t.regions_b() || t.depth().is_err() {
        return None;
    }
    let lexed = super::lex_with(src, super::Variant::B).ok()?;
    let short = core::cell::Cell::new(false);
    let m = t.m(&short);
    let n = src.len().div_ceil(64);
    let mut want = vec![0u64; n + 1];
    for tok in &lexed.toks {
        let s = tok.start as usize;
        if tok.end > tok.start && s < src.len() && m.bit(s, |b| b.code) {
            want[s / 64] |= 1 << (s % 64);
        }
    }
    let (mut same, mut rare, mut first) = (0, 0, None);
    for (bi, &w) in want.iter().enumerate().take(n) {
        let (got, r) = code_starts(&m, bi);
        if r {
            rare += 1;
        } else if got == w {
            same += 1;
        } else if first.is_none() {
            let at = bi * 64;
            let text = String::from_utf8_lossy(&src[at.saturating_sub(16)..(at + 80).min(src.len())]).into_owned();
            first = Some(format!("block {bi}: masks {got:064b}, walk {w:064b} (bit 0 the block's first byte, the text from 16 before it: {text:?})"));
        }
    }
    Some((same, rare, first))
}
