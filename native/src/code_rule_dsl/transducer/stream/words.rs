//! The word kernel of every operation (`10` §3.1), with its carry (`06` §2).
//!
//! A kernel takes one block's operand words and the carry from the neighbouring block (the
//! previous one, or the next one for `next` and `add_back`), and gives the block's word and the
//! carry onward. Each kernel is a few word instructions: shifts, logic, one add with carry, a
//! popcount, a bit reversal. `class` is two table lookups per 16 bytes (NEON `vqtbl1q_u8`).
//!
//! For running blocks out of order, every carry also has a **summary** of a block (what the block
//! does to the carry, computed from the block alone), an associative **combine** of two summaries,
//! and an **apply** of a summary to a carry. Only summaries cross blocks; the words are recomputed
//! with the right carry (`06` §2).
//!
//! No kernel here loops; the plane scans' fixed loops are in `planes`.

use super::planes::{self, ClampMap, ModSummary, Planes};
use super::program::{ClassTable, Op, Program, Value};

/// One 64-byte block of the stream, and what the stream says about it.
#[derive(Clone, Copy, Debug)]
pub(in crate::code_rule_dsl) struct Block<'a> {
    pub(in crate::code_rule_dsl) bytes: &'a [u8; 64],
    /// Bit `j` when byte `j` starts a file.
    pub(in crate::code_rule_dsl) starts: u64,
    /// Bit `j` when byte `j` is in the stream: all ones but in the last block.
    pub(in crate::code_rule_dsl) valid: u64,
}

/// A block's word of a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::code_rule_dsl) enum Word {
    Mask(u64),
    Planes(Planes),
    Count(u64),
}

impl Word {
    pub(in crate::code_rule_dsl) fn mask(&self) -> u64 {
        match self {
            Word::Mask(m) => *m,
            _ => panic!("not a mask"),
        }
    }

    pub(in crate::code_rule_dsl) fn planes(&self) -> &Planes {
        match self {
            Word::Planes(p) => p,
            _ => panic!("not planes"),
        }
    }
}

/// A carry between blocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::code_rule_dsl) enum Carry {
    /// The operation has none.
    None,
    /// The neighbouring block's operand word, for a shift.
    Halo(u64),
    /// One bit: a carry, a borrow, a parity.
    Bit(bool),
    /// A count's value: mod `n`, or clamped.
    Num(u16),
    /// A running total.
    Total(u64),
}

/// What a block does to a carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::code_rule_dsl) enum Summary {
    None,
    /// A halo is the block's own word, whatever came before.
    Halo(u64),
    /// A 1-bit carry: the carry out for a carry in of 0 and of 1.
    Bit { zero: bool, one: bool },
    Mod(ModSummary),
    Clamp(ClampMap),
    /// A count adds the block's popcount.
    Total(u64),
}

// --- The mask kernels ---------------------------------------------------------------------

/// simdjson's `prefix_xor`, in six shift-and-XOR steps (`vmull_p64` by all ones on NEON with
/// crypto, `simd_prefix_exclusive_xor` on Metal; both equal this).
pub(in crate::code_rule_dsl) fn prefix_xor(a: u64) -> u64 {
    let a = a ^ (a << 1);
    let a = a ^ (a << 2);
    let a = a ^ (a << 4);
    let a = a ^ (a << 8);
    let a = a ^ (a << 16);
    a ^ (a << 32)
}

/// `prev(a, k)`: bit `j` is `a`'s bit `j - k`, the previous block's top `k` bits below.
pub(in crate::code_rule_dsl) fn prev(a: u64, k: u8, halo: u64) -> u64 {
    (a << k) | (halo >> (64 - u32::from(k)))
}

/// `next(a, k)`: bit `j` is `a`'s bit `j + k`, the next block's low `k` bits above.
pub(in crate::code_rule_dsl) fn next(a: u64, k: u8, halo: u64) -> u64 {
    (a >> k) | (halo << (64 - u32::from(k)))
}

/// `a + b + carry`, and the carry out.
pub(in crate::code_rule_dsl) fn add(a: u64, b: u64, carry: bool) -> (u64, bool) {
    let (s, c1) = a.overflowing_add(b);
    let (s, c2) = s.overflowing_add(u64::from(carry));
    (s, c1 | c2)
}

/// `a - b - borrow`, and the borrow out.
pub(in crate::code_rule_dsl) fn sub(a: u64, b: u64, borrow: bool) -> (u64, bool) {
    let (s, b1) = a.overflowing_sub(b);
    let (s, b2) = s.overflowing_sub(u64::from(borrow));
    (s, b1 | b2)
}

/// `a + b + carry` with the bytes in reverse order: the carry runs to earlier bytes.
pub(in crate::code_rule_dsl) fn add_back(a: u64, b: u64, carry: bool) -> (u64, bool) {
    let (s, c) = add(a.reverse_bits(), b.reverse_bits(), carry);
    (s.reverse_bits(), c)
}

/// The inclusive prefix XOR after a parity, and the parity after the block.
pub(in crate::code_rule_dsl) fn toggle(a: u64, parity: bool) -> (u64, bool) {
    let out = prefix_xor(a) ^ 0u64.wrapping_sub(u64::from(parity));
    (out, out >> 63 != 0)
}

/// The bytes in any class of `bits`: two nibble lookups per byte, 16 bytes per instruction.
#[cfg(target_arch = "aarch64")]
pub(in crate::code_rule_dsl) fn class(table: &ClassTable, bits: u8, bytes: &[u8; 64]) -> u64 {
    use core::arch::aarch64::*;
    // SAFETY: NEON is part of the aarch64 baseline; every load reads 16 bytes inside an array
    // of at least 16.
    unsafe {
        let lo = vld1q_u8(table.lo.as_ptr());
        let hi = vld1q_u8(table.hi.as_ptr());
        let sel = vdupq_n_u8(bits);
        let low_nibble = vdupq_n_u8(0x0f);
        let chunk = |at: usize| {
            let d = vld1q_u8(bytes.as_ptr().add(at));
            let l = vqtbl1q_u8(lo, vandq_u8(d, low_nibble));
            let h = vqtbl1q_u8(hi, vshrq_n_u8::<4>(d));
            vtstq_u8(vandq_u8(l, h), sel)
        };
        // simdjson's `simd8x64<bool>::to_bitmask` for arm64: weight each byte by its bit, then
        // three rounds of pairwise adds.
        let weights: [u8; 16] = [1, 2, 4, 8, 16, 32, 64, 128, 1, 2, 4, 8, 16, 32, 64, 128];
        let w = vld1q_u8(weights.as_ptr());
        let s0 = vpaddq_u8(vandq_u8(chunk(0), w), vandq_u8(chunk(16), w));
        let s1 = vpaddq_u8(vandq_u8(chunk(32), w), vandq_u8(chunk(48), w));
        let s = vpaddq_u8(s0, s1);
        let s = vpaddq_u8(s, s);
        vgetq_lane_u64::<0>(vreinterpretq_u64_u8(s))
    }
}

/// The same on other hosts, a byte at a time: a portable stand-in so the crate builds there.
/// It is not the measured path; an SSSE3 `pshufb` kernel replaces it when x86 is measured.
#[cfg(not(target_arch = "aarch64"))]
pub(in crate::code_rule_dsl) fn class(table: &ClassTable, bits: u8, bytes: &[u8; 64]) -> u64 {
    bytes.iter().enumerate().fold(0, |m, (j, &b)| m | (u64::from(table.member(b, bits)) << j))
}

// --- One operation over one block ---------------------------------------------------------

/// The carry an operation starts the stream with (or, for a backward one, ends it with).
pub(in crate::code_rule_dsl) fn initial(op: &Op) -> Carry {
    match op {
        Op::Prev(..) | Op::Next(..) => Carry::Halo(0),
        Op::Add(..) | Op::Sub(..) | Op::AddBack(..) | Op::Toggle(_) => Carry::Bit(false),
        Op::ScanMod { .. } | Op::ScanClamp { .. } => Carry::Num(0),
        Op::Count(_) => Carry::Total(0),
        _ => Carry::None,
    }
}

/// Run instruction `at` of `program` over one block: its operands' words are `words[v]`.
/// `Rounds`, `Feed` and `End` are the interpreter's (step 2), not a block kernel's.
pub(in crate::code_rule_dsl) fn step(
    program: &Program,
    at: usize,
    block: &Block<'_>,
    words: &dyn Fn(Value) -> Word,
    carry: Carry,
) -> (Word, Carry) {
    let m = |v: &Value| words(*v).mask();
    let bit = || match carry {
        Carry::Bit(b) => b,
        _ => unreachable!("verified"),
    };
    let halo = || match carry {
        Carry::Halo(h) => h,
        _ => unreachable!("verified"),
    };
    let num = || match carry {
        Carry::Num(n) => n,
        _ => unreachable!("verified"),
    };
    let mask = |w: u64| (Word::Mask(w), Carry::None);
    match &program.insts()[at] {
        Op::Starts => mask(block.starts),
        Op::Const(c) => mask(*c),
        Op::Class { table, bits } => mask(class(&program.tables()[usize::from(*table)], *bits, block.bytes)),
        Op::Not(a) => mask(!m(a)),
        Op::And(a, b) => mask(m(a) & m(b)),
        Op::Or(a, b) => mask(m(a) | m(b)),
        Op::Xor(a, b) => mask(m(a) ^ m(b)),
        Op::AndNot(a, b) => mask(m(a) & !m(b)),
        Op::Select(c, a, b) => mask((m(c) & m(a)) | (!m(c) & m(b))),
        Op::Prev(a, k) => (Word::Mask(prev(m(a), *k, halo())), Carry::Halo(m(a))),
        // Past the stream's end a value is zero, so the next block's halo is masked.
        Op::Next(a, k) => {
            let a = m(a) & block.valid;
            (Word::Mask(next(a, *k, halo())), Carry::Halo(a))
        }
        Op::Add(a, b) => {
            let (s, c) = add(m(a), m(b), bit());
            (Word::Mask(s), Carry::Bit(c))
        }
        Op::Sub(a, b) => {
            let (s, c) = sub(m(a), m(b), bit());
            (Word::Mask(s), Carry::Bit(c))
        }
        Op::AddBack(a, b) => {
            let (s, c) = add_back(m(a) & block.valid, m(b) & block.valid, bit());
            (Word::Mask(s), Carry::Bit(c))
        }
        Op::Toggle(a) => {
            let (s, c) = toggle(m(a), bit());
            (Word::Mask(s), Carry::Bit(c))
        }
        Op::ScanMod { n, up, down, reset } => {
            let (p, after) = planes::scan_mod(*n, m(up), m(down), m(reset), num() as u8);
            (Word::Planes(p), Carry::Num(u16::from(after)))
        }
        Op::ScanClamp { max, up, down, reset } => {
            let (p, after) = planes::scan_clamp(*max, m(up), m(down), m(reset), num());
            (Word::Planes(p), Carry::Num(after))
        }
        Op::Plane(a, p) => mask(words(*a).planes().planes[usize::from(*p)]),
        Op::PlaneEq(a, c) => mask(words(*a).planes().eq(u16::from(*c))),
        Op::Count(a) => {
            let Carry::Total(t) = carry else { unreachable!("verified") };
            let t = t + u64::from((m(a) & block.valid).count_ones());
            (Word::Count(t), Carry::Total(t))
        }
        Op::Rounds { .. } | Op::Feed(_) | Op::End(_) => {
            unreachable!("rounds are run by the interpreter")
        }
    }
}

/// What one block does to instruction `at`'s carry, from the block alone.
pub(in crate::code_rule_dsl) fn summary(
    program: &Program,
    at: usize,
    block: &Block<'_>,
    words: &dyn Fn(Value) -> Word,
) -> Summary {
    let m = |v: &Value| words(*v).mask();
    match &program.insts()[at] {
        Op::Prev(a, _) => Summary::Halo(m(a)),
        Op::Next(a, _) => Summary::Halo(m(a) & block.valid),
        Op::Add(a, b) => {
            let (s, zero) = m(a).overflowing_add(m(b));
            Summary::Bit { zero, one: zero | (s == u64::MAX) }
        }
        Op::Sub(a, b) => Summary::Bit { zero: m(a) < m(b), one: m(a) <= m(b) },
        Op::AddBack(a, b) => {
            let (ra, rb) = ((m(a) & block.valid).reverse_bits(), (m(b) & block.valid).reverse_bits());
            let (s, zero) = ra.overflowing_add(rb);
            Summary::Bit { zero, one: zero | (s == u64::MAX) }
        }
        Op::Toggle(a) => {
            let odd = m(a).count_ones() & 1 != 0;
            Summary::Bit { zero: odd, one: !odd }
        }
        Op::ScanMod { n, up, down, reset } => {
            Summary::Mod(planes::summary_mod(*n, m(up), m(down), m(reset)))
        }
        Op::ScanClamp { max, up, down, reset } => {
            Summary::Clamp(planes::summary_clamp(*max, m(up), m(down), m(reset)))
        }
        Op::Count(a) => Summary::Total(u64::from((m(a) & block.valid).count_ones())),
        _ => Summary::None,
    }
}

/// `first` then `then`, in the carry's direction. Associative.
pub(in crate::code_rule_dsl) fn combine(op: &Op, first: Summary, then: Summary) -> Summary {
    match (first, then) {
        (Summary::None, Summary::None) => Summary::None,
        (Summary::Halo(_), Summary::Halo(h)) => Summary::Halo(h),
        (Summary::Bit { zero: z1, one: o1 }, Summary::Bit { zero: z2, one: o2 }) => {
            let g = |c: bool| if c { o2 } else { z2 };
            Summary::Bit { zero: g(z1), one: g(o1) }
        }
        (Summary::Mod(a), Summary::Mod(b)) => {
            let Op::ScanMod { n, .. } = op else { unreachable!() };
            Summary::Mod(planes::combine_mod(*n, a, b))
        }
        (Summary::Clamp(a), Summary::Clamp(b)) => {
            let Op::ScanClamp { max, .. } = op else { unreachable!() };
            Summary::Clamp(planes::combine_clamp(*max, a, b))
        }
        (Summary::Total(a), Summary::Total(b)) => Summary::Total(a + b),
        _ => unreachable!("one operation's summaries"),
    }
}

/// The carry after a block, from its summary and the carry before it.
pub(in crate::code_rule_dsl) fn apply(op: &Op, s: Summary, carry: Carry) -> Carry {
    match (s, carry) {
        (Summary::None, c) => c,
        (Summary::Halo(h), _) => Carry::Halo(h),
        (Summary::Bit { zero, one }, Carry::Bit(c)) => Carry::Bit(if c { one } else { zero }),
        (Summary::Mod(s), Carry::Num(c)) => {
            let Op::ScanMod { n, .. } = op else { unreachable!() };
            Carry::Num(u16::from(planes::apply_mod(*n, s, c as u8)))
        }
        (Summary::Clamp(s), Carry::Num(c)) => Carry::Num(planes::apply_clamp(s, c)),
        (Summary::Total(t), Carry::Total(c)) => Carry::Total(c + t),
        _ => unreachable!("one operation's summary and carry"),
    }
}
