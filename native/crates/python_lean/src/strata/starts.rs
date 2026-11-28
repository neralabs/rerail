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
    let keep = 0u64.wrapping_sub(u64::from(bi > 0));
    starts_in(t, bi, w(t.blk(bi.saturating_sub(1)).code & keep, t.blk(bi).code), false)
}

/// Block `bi`'s token starts in the bytes of `code` (the window of the block below and the block), and whether the
/// block is rare. In an interpolation (`one_by_one`) the walk reads a token at a time, so a number's rest of a word
/// run is always a name.
#[inline(always)]
fn starts_in(t: &Masks<'_>, bi: usize, code: u128, one_by_one: bool) -> (u64, bool) {
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
    let nb = code & m(|b| !b.skip & !b.eol & !b.bs) & valid;
    // Where the walk's run of tokens stops: not code, a line's end, a form feed, a backslash, the text's end.
    let stop = !code | m(|b| b.eol) | m(|b| b.skip & !b.sptab & !b.lf) | m(|b| b.bs) | !valid;
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
    let all = 0u128.wrapping_sub(u128::from(one_by_one));
    let rest_starts = rest & (reaching(stop, through) | (1u128 << 64) | all);
    let starts = starts | rest_starts;
    // Rare: a refused byte or one past ASCII in the block; the run of non-blank code the window starts in reaches
    // the block (what it starts with is below the window); a number's rest of a word run that reaches a block's
    // end, or whose walk to a stop or a start leaves the window.
    let first_run = run_ends(nb & 1, nb);
    let reaches = first_run >= (1u128 << 64);
    let rest_word = rest | ((word.wrapping_add((rest << 1) & word) ^ word) & word);
    let crosses = ((rest_word >> 63) & 3 == 3) | (rest_word >> 127 != 0);
    let rest_far = crosses | ((reaching(starts | stop, through) & rest & !(1u128 << 64) & !all) != rest & !(1u128 << 64) & !all);
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

/// The oracle of `string_starts`: for each block, whether its starts in strings are the serial walk's (blocks
/// that agree, blocks rare, the first that differs).
pub fn string_starts_are_the_walks(src: &[u8]) -> Option<(usize, usize, Option<String>)> {
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
        if tok.end > tok.start && s < src.len() && !m.bit(s, |b| b.code) {
            want[s / 64] |= 1 << (s % 64);
        }
    }
    let (mut same, mut rare, mut first) = (0, 0, None);
    let mut c = StrCarry::default();
    for (bi, &w) in want.iter().enumerate().take(n) {
        let in_code = m.blk(bi).code;
        let (got, r) = string_starts(&m, bi, &mut c);
        let got = got & !in_code;
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

/// A string's byte classes the string starts read, of the byte at `at`.
#[derive(Clone, Copy, Default)]
struct SBytes {
    r: u64,
    b: u64,
    f: u64,
    pfx: u64,
    sq: u64,
    dq: u64,
    u: u64,
    big_u: u64,
    x: u64,
    big_n: u64,
    digit: u64,
    hex: u64,
    cr: u64,
    lf: u64,
    lbrace: u64,
    rbrace: u64,
    /// The bytes an escape of two is (`\n`, `\'`, `\\` and the like).
    simple: u64,
    colon: u64,
    bang: u64,
    eq: u64,
    lower: u64,
}

#[inline(always)]
fn sclasses(src: &[u8], at: usize) -> SBytes {
    let v = bytes_from(src, at);
    let s = V::splat;
    let eq = |x: u8| v.simd_eq(s(x)).to_bitmask();
    let low = v | s(0x20);
    let leq = |x: u8| low.simd_eq(s(x)).to_bitmask();
    let mut simple = 0;
    for c in *b"\n'\"abfrntv\\" {
        simple |= eq(c);
    }
    SBytes {
        r: leq(b'r'),
        b: leq(b'b'),
        f: leq(b'f') | leq(b't'),
        pfx: leq(b'r') | leq(b'b') | leq(b'f') | leq(b't') | leq(b'u'),
        sq: eq(b'\''),
        dq: eq(b'"'),
        u: eq(b'u'),
        big_u: eq(b'U'),
        x: eq(b'x'),
        big_n: eq(b'N'),
        digit: (v - s(b'0')).simd_lt(s(10)).to_bitmask(),
        hex: (v - s(b'0')).simd_lt(s(10)).to_bitmask() | (low - s(b'a')).simd_lt(s(6)).to_bitmask(),
        cr: eq(b'\r'),
        lf: eq(b'\n'),
        lbrace: eq(b'{'),
        rbrace: eq(b'}'),
        simple,
        colon: eq(b':'),
        bang: eq(b'!'),
        eq: eq(b'='),
        lower: (v - s(b'a')).simd_lt(s(26)).to_bitmask(),
    }
}

/// What a string's starts carry from a block to the next: whether a string is open (and its kinds), the bytes of an
/// escape or a brace pair that reach into it, whether its first byte follows a string's text, the `}` runs' parity,
/// and whether the open string is one its masks do not read (rare until it closes).
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(super) struct StrCarry {
    /// The quotes after prefixes of the block before, and their kinds; the openers' ends past it, and theirs.
    q: u64,
    q_raw: u64,
    q_bytes: u64,
    q_fmt: u64,
    oe: u64,
    oe_raw: u64,
    oe_bytes: u64,
    oe_fmt: u64,
    open: bool,
    raw: bool,
    bytes: bool,
    format: bool,
    covered: u64,
    text: bool,
    rb: u64,
    rare: bool,
    /// An interpolation (or its format spec) open past the block, its depth of brackets there, and its bytes of
    /// code in the block (the next block's window reads them below it).
    reg: bool,
    spec: bool,
    idepth: i8,
    rcode: u64,
    /// A conversion's `!` is the block before's last byte: its letter is the block's first.
    cv: bool,
}

/// From each of `seeds` forward through `through` (each seed in it): the run to its end.
#[inline(always)]
fn fill(seeds: u128, through: u128) -> u128 {
    let s = seeds & through;
    s | ((through.wrapping_add(s) ^ through) & through)
}

/// Block `bi`'s token starts in its strings (a string's start, its text, escapes and brace pairs, its end) from the
/// carry `c` into it (left as the carry out), and whether the block is rare.
pub(super) fn string_starts(t: &Masks<'_>, bi: usize, c: &mut StrCarry) -> (u64, bool) {
    let src = t.src;
    let b = *t.blk(bi);
    let at = 64 * bi;
    let n: [SBytes; 6] = core::array::from_fn(|d| sclasses(src, at + d));
    let hi = |x: u64| u128::from(x);
    let bool128 = |x: bool| u128::from(x);
    // The strings that start in the block, the quote after each prefix (the prefix's letters read into the block
    // after), and the opener's end.
    let up_n = sclasses(src, at + 64);
    let s = hi(b.sstart);
    let pfx = w(n[0].pfx, up_n.pfx);
    let pre = fill(s & pfx, pfx);
    let q0 = run_ends(s, pfx);
    let kind = |letter: u128| q0 & run_ends(letter & pre, pfx);
    // With the quotes the block before's prefixes reach.
    let q = q0 | hi(c.q);
    let (raw_q, bytes_q, fmt_q) = (kind(w(n[0].r, up_n.r)) | hi(c.q_raw), kind(w(n[0].b, up_n.b)) | hi(c.q_bytes), kind(w(n[0].f, up_n.f)) | hi(c.q_fmt));
    let triple = q & ((hi(n[0].sq & n[1].sq & n[2].sq)) | hi(n[0].dq & n[1].dq & n[2].dq));
    let to_oe = |x: u128| ((x & !triple) << 1) | ((x & triple) << 3);
    // With the openers' ends the block before's openers reach.
    let oe = to_oe(q) | hi(c.oe);
    // A string's interior: from its opener's end (or the block's start, where one is open) to its closer.
    let closer = hi(b.send);
    let through = !closer;
    let interior = fill(oe | bool128(c.open), through);
    let kind_in = |k_q: u128, k_oe: u64, k: bool| fill(to_oe(k_q) | hi(k_oe) | bool128(c.open & k), through);
    let (iraw, ibytes, ifmt) = (kind_in(raw_q, c.oe_raw, c.raw), kind_in(bytes_q, c.oe_bytes, c.bytes), kind_in(fmt_q, c.oe_fmt, c.format));
    let mask = (1u128 << 64) - 1;
    let inner = interior & mask;
    // Escapes: each backslash no odd run escapes, in a string not raw; in a bytes string `\N`, `\u`, `\U` are its
    // text.
    let e0 = hi(b.bs & !b.esc) & inner & !iraw;
    let e = e0 & !(ibytes & hi(n[1].big_n | n[1].u | n[1].big_u));
    let u_ok = hi(n[1].u & n[2].hex & n[3].hex & n[4].hex & n[5].hex);
    let x_ok = hi(n[1].x & n[2].hex & n[3].hex);
    let d = hi(n[1].digit);
    let (d2, d3) = (hi(n[2].digit), hi(n[3].digit));
    let cr = hi(n[1].cr);
    let lf2 = hi(n[2].lf);
    let l2 = e & (u_ok | x_ok | d | cr | hi(n[1].simple));
    let l3 = e & (u_ok | x_ok | (d & d2) | (cr & lf2));
    let l4 = e & (u_ok | x_ok | (d & d2 & d3));
    let l5 = e & u_ok;
    let covered_e = e | (l2 << 1) | (l3 << 2) | (l4 << 3) | (l5 << 4) | (l5 << 5);
    // An f-string's braces: a pair of `{` or of `}` is one token, a `{` no `{` pairs with opens an interpolation, a
    // `}` no `}` pairs with is a refusal (rare).
    let fint = inner & ifmt & !covered_e & !hi(c.covered);
    let f64_ = fint as u64;
    let lb = hi(b.lb) & fint;
    let rb = hi(b.rb) & fint;
    let interp = (lb & hi(b.lbu) & !hi(n[1].lbrace)) as u64;
    // Interpolations, in rounds (unrolled). The brackets' depth `g` over the string's bytes, once: an
    // interpolation from its `{` at depth `G` (after it) ends at the first `}` after it with `g` at `G - 1` or the
    // first `:` at `G` (its format spec's start, which ends at the next `}`). A `!` at `G` before no `=` is a
    // conversion.
    let (up_l, down_l) = (super::lanes(b.ob & f64_), super::lanes(b.cb & f64_));
    let mut g = [super::D::splat(0); 4];
    let mut carry = super::D::splat(0);
    for r in 0..4 {
        g[r] = super::prefix_sum(down_l[r] - up_l[r]) + carry;
        carry = super::D::splat(g[r][15]);
    }
    let g_at = |bit: u64| g[(bit.trailing_zeros() as usize >> 4) & 3][bit.trailing_zeros() as usize & 15];
    let eqm = |v: i8| (0..4).fold(0u64, |m, r| m | (g[r].simd_eq(super::D::splat(v)).to_bitmask() << (16 * r)));
    let (rbr, colon, cbm) = (b.rb & f64_, n[0].colon & f64_, b.cb & f64_);
    let above = |x: u64| !(x | x.wrapping_sub(1));
    let below = |x: u64| x.wrapping_sub(1) & !x;
    let sel = |c: bool, a: u64, b: u64| (a & 0u64.wrapping_sub(u64::from(c))) | (b & !0u64.wrapping_sub(u64::from(c)));
    // A round: the open interpolation or spec (if any) to its end, then the next interpolation's start.
    #[derive(Clone, Copy, Default)]
    struct Round {
        todo: u64,
        reg: bool,
        spec: bool,
        gv: i8,
        code: u64,
        convs: u64,
        marks: u64,
        bytes: u64,
        closes: u64,
        bad: bool,
        cont_reg: bool,
        cont_spec: bool,
        cont_gv: i8,
    }
    let round = |st: Round| {
        let regm = 0u64.wrapping_sub(u64::from(st.reg));
        let (at0, atm1) = (eqm(st.gv), eqm(st.gv.wrapping_sub(1)));
        let ends = ((rbr & atm1) | (colon & at0)) & st.todo & regm;
        let end = ends.isolate_lowest_one();
        let region = st.todo & below(end) & regm;
        let conv = n[0].bang & !n[1].eq & at0 & region & f64_;
        let bad_reg = (cbm & atm1 & region) | (conv & !n[1].lower) | (region & (b.eol | b.hash | b.bs | b.sstart | b.send));
        let is_colon = end & colon != 0;
        let spec_from = sel(is_colon, above(end), sel(st.spec, st.todo, 0));
        let sends = rbr & spec_from;
        let send_ = sends.isolate_lowest_one();
        let spec_bytes = spec_from & below(send_);
        let bad_spec = spec_bytes & (b.lb | b.lf | b.send);
        let close = (end & !colon) | send_;
        let active = st.reg | st.spec;
        let todo = sel(active, above(close), st.todo);
        let a = interp & todo;
        let a = a.isolate_lowest_one();
        Round {
            todo: above(a),
            reg: a != 0,
            spec: false,
            gv: g_at(a),
            code: st.code | (region & !(conv | conv << 1)),
            convs: st.convs | conv,
            marks: st.marks | a | close | (end & colon) | conv,
            bytes: st.bytes | region | spec_bytes | end | close | a,
            closes: st.closes | close,
            bad: st.bad | ((bad_reg | bad_spec) != 0) | (((region | spec_bytes) & !(interior as u64)) != 0),
            cont_reg: st.cont_reg | (st.reg & (end == 0)),
            cont_spec: st.cont_spec | ((is_colon | st.spec) & (send_ == 0)),
            cont_gv: sel(st.reg & (end == 0), st.gv as u8 as u64, st.cont_gv as u8 as u64) as u8 as i8,
        }
    };
    let r0 = Round { todo: !0, reg: c.reg, spec: c.spec, gv: c.idepth.wrapping_neg(), ..Round::default() };
    let r = round(round(round(round(round(r0)))));
    let r = Round { code: r.code & !u64::from(c.cv), ..r };
    // More interpolations than the rounds read: rare.
    let more = r.reg;
    let ibytes_all = hi(r.bytes);
    let (rcode, rare_code) = starts_in(t, bi, w(c.rcode, r.code), true);
    let lb_pair = lb & !ibytes_all & hi(b.lbu) & hi(n[1].lbrace);
    let rb_t = rb & !ibytes_all;
    let rbu = rb_t & !hi(super::escaped(rb_t as u64, &mut c.rb));
    let rb_pair = rbu & hi(n[1].rbrace);
    let lone_rb = rbu & !hi(n[1].rbrace);
    let covered = covered_e | lb_pair | (lb_pair << 1) | rb_pair | (rb_pair << 1) | hi(c.covered);
    // A string's text: the bytes of its interior no token above takes, nor a format string's brace, nor an
    // interpolation's.
    let text = inner & !covered & !lb & !rb & !closer & !ibytes_all;
    let text_starts = text & !((text << 1) | bool128(c.text));
    let starts = s | text_starts | (e & !ibytes_all) | lb_pair | rb_pair | (closer & mask) | hi(r.marks) | hi(rcode & r.code);
    // Rare: `\U` or `\N{` (long escapes); a lone `}`; a raw string's backslash where its text has not begun, or a
    // raw f-string's (the walk's quirks there); an interpolation the masks do not read (a string, a comment, a
    // backslash or a line's end in it; a bracket it does not open closed in it; a conversion of no letter; a `{`
    // or a line's end in its spec; more than the rounds); a string the carry says is rare.
    let long = e & hi(n[1].big_u | (n[1].big_n & n[2].lbrace)) != 0;
    let raw_first = oe | (bool128(c.open & !c.text) & !hi(c.covered) & 1);
    let raw_quirk = (hi(b.bs) & inner & iraw & (raw_first | ifmt)) != 0;
    let rare_interp = r.bad | more | (rare_code & ((r.code | c.rcode) != 0));
    let rare_here = c.rare | long | (lone_rb != 0) | raw_quirk | rare_interp;
    let depth_end = g[3][15];
    // The carry out.
    let open = interior >> 64 & 1 == 1;
    // A quote past the block is the next block's to read (its opener's end too); an opener's end past it, of a
    // quote in it, is carried.
    let up = |x: u128| (x >> 64) as u64;
    let end_up = |x: u128| up(to_oe(x & mask));
    *c = StrCarry {
        q: up(q0),
        q_raw: up(raw_q),
        q_bytes: up(bytes_q),
        q_fmt: up(fmt_q),
        oe: end_up(q),
        oe_raw: end_up(raw_q),
        oe_bytes: end_up(bytes_q),
        oe_fmt: end_up(fmt_q),
        open,
        raw: iraw >> 64 & 1 == 1,
        bytes: ibytes >> 64 & 1 == 1,
        format: ifmt >> 64 & 1 == 1,
        covered: (covered >> 64) as u64,
        text: text >> 63 & 1 == 1,
        rb: c.rb,
        rare: open & (c.rare | rare_interp),
        reg: r.cont_reg,
        spec: r.cont_spec,
        idepth: depth_end.wrapping_sub(r.cont_gv),
        rcode: r.code,
        cv: r.convs >> 63 != 0,
    };
    (starts as u64, rare_here)
}

/// Block `bi`'s extras' starts (a comment's `#`, a continuation's `\`), and whether the block is rare. A
/// continuation is an extra unless the next line's first byte past its blanks starts a string's prefix and quote
/// (the walk's reading).
pub(super) fn extra_starts(t: &Masks<'_>, bi: usize) -> (u64, bool) {
    let src = t.src;
    let (b, b1) = (*t.blk(bi), *t.blk(bi + 1));
    let m = |f: fn(&B) -> u64| w(f(&b), f(&b1));
    // The window of the block and the one after: bit `j` is byte `64 * bi + j`.
    let lim = src.len().saturating_sub(64 * bi).min(128) as u32;
    let valid = ((1u128 << (lim & 127)) - 1) | 0u128.wrapping_sub(u128::from(lim >> 7));
    let quote = m(|b| b.sq | b.dq | b.bt);
    let blanks = m(|b| b.skip & !b.lf);
    let (lf, cr) = (m(|b| b.lf), m(|b| b.eol & !b.lf));
    // The bytes from which a run of prefix letters reaches a quote, and from which blanks reach one of them.
    let string_at = reaching(quote, m(|b| b.pfx));
    let line_string = reaching(string_at & valid, blanks);
    let cont = m(|b| b.bs & b.code) & valid & u128::from(u64::MAX);
    let (nl, crlf) = (cont & (lf >> 1), cont & (cr >> 1) & (lf >> 2));
    // At the text's end a continuation needs no line end.
    let at_end = cont & !(valid >> 1);
    let quirk = (nl & (line_string >> 2)) | (crlf & (line_string >> 3));
    let extras = (m(|b| b.cstart) & u128::from(u64::MAX)) | (cont & !quirk);
    // Rare: a backslash before no line end (the walk refuses it); a next line's blanks and letters that reach the
    // window's top.
    let refused = cont & !(nl | crlf | at_end) != 0;
    let after = (nl << 2) | (crlf << 3);
    let far = fill(after, blanks | m(|b| b.pfx)) >> 127 != 0;
    (extras as u64, refused | far)
}

/// The oracle of `extra_starts`: for each block, whether its extras' starts are the serial walk's.
pub fn extras_are_the_walks(src: &[u8]) -> Option<(usize, usize, Option<String>)> {
    let mut t = super::Text { src, b: super::classes(src), first: 0 };
    if !t.regions_b() || t.depth().is_err() {
        return None;
    }
    let lexed = super::lex_with(src, super::Variant::B).ok()?;
    let short = core::cell::Cell::new(false);
    let m = t.m(&short);
    let n = src.len().div_ceil(64);
    let mut want = vec![0u64; n + 1];
    for e in &lexed.extras {
        let s = e.start as usize;
        want[s / 64] |= 1 << (s % 64);
    }
    let (mut same, mut rare, mut first) = (0, 0, None);
    for (bi, &w) in want.iter().enumerate().take(n) {
        let (got, r) = extra_starts(&m, bi);
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
