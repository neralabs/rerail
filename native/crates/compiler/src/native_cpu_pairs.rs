//! Byte-pair rules made one lookup, as simdjson's UTF-8 check makes its two-byte rules
//! (`check_special_cases`: the byte before's two nibbles and this byte's high nibble looked up
//! and ANDed, each rule a bit).
//!
//! A grammar states such a rule as `prev(A, 1) & B`, with `A` and `B` byte classes: "after an
//! `0xe0`, a byte below `0xa0`". Computed as written, each one is `A` on the bytes, shifted a
//! byte (an `ext` a chunk), `B`, and an `and`. Here every rule of the program gets a bit of two
//! new tables: `P`, the byte before's classes, and `Q`, this byte's. A rule is then
//! `pair(P, Q, its bit)` (`BlockInst::Pair`), and the rules ORed in one tree are one pair of all
//! their bits. The tables' lookups are made once a chunk, for every rule: simdjson's three
//! lookups. Where every rule's `B` is a set of high nibbles, as UTF-8's are, `Q`'s low half is
//! all ones, and its lookup is one `tbl` (`native_cpu_lanes`).
//!
//! The rewrite is structural, on the CPU's program (`native_cpu_block::compile`), after the host
//! strips the prefix. The interpreter runs the program as written, so it is the rewrite's
//! oracle. A rule is either of:
//!
//! - `and(prev(L, 1), C)`: the byte before in `L`, this byte in `C`;
//! - `and_not(C, prev(L, 1))`: the byte before not in `L`, this byte in `C`. At the stream's
//!   first byte, where a `prev` reads no bit, it holds: the pair reads the byte 0x00 there, and
//!   0x00 is not in `L`.
//!
//! `L` and `C` are byte classes (an `eq` or a `class`), each one product: a set of high nibbles
//! by a set of low ones. `L` holds no 0x00, so the pair's 0x00 before the stream is the `prev`'s
//! zero bit. Eight rules at most, the tables' bits; two at least.
//!
//! A rule's pair is its value exactly, so a rule is made its pair wherever it is read. The rules
//! ORed in one tree (every inner `or` read by the tree only, every rule by its `or` only, none
//! an output) are one pair: the first rule becomes it, the others zero, so the tree's value is
//! the same. No member of a fixed point is rewritten.

use crate::block::ClassTable;
use crate::block_native::{BlockInst, BlockNative};

/// A rule: the byte before's set and this byte's, each a product (high nibbles, low nibbles).
#[derive(Clone, Copy, PartialEq, Eq)]
struct Rule {
    before: (u16, u16),
    here: (u16, u16),
}

/// `native` with its byte-pair rules made pairs, or `None` where it has fewer than two.
pub fn fused(native: &BlockNative) -> Option<BlockNative> {
    let n = native.insts.len();
    let mut readers = vec![0usize; n];
    for inst in &native.insts {
        for o in inst.operands() {
            readers[o] += 1;
        }
    }
    let mut output = vec![false; n];
    for &o in &native.outputs {
        output[usize::from(o)] = true;
    }
    let mut grouped = vec![false; n];
    for g in &native.groups {
        for &m in &g.members {
            grouped[m] = true;
        }
        for &(c, x) in &g.carries {
            grouped[c] = true;
            grouped[x] = true;
        }
    }
    // The rules and their bits: one bit a distinct rule, eight at most.
    let mut kinds: Vec<Rule> = Vec::new();
    let mut bit_of: Vec<Option<u8>> = vec![None; n];
    for x in (0..n).filter(|&x| !grouped[x]) {
        let Some(r) = rule(native, x) else { continue };
        let k = match kinds.iter().position(|&q| q == r) {
            Some(k) => k,
            None if kinds.len() < 8 => {
                kinds.push(r);
                kinds.len() - 1
            }
            None => continue,
        };
        bit_of[x] = Some(1 << k);
    }
    if bit_of.iter().flatten().count() < 2 {
        return None;
    }
    let (mut before, mut here) = (ClassTable::default(), ClassTable::default());
    for (k, r) in kinds.iter().enumerate() {
        for (t, (hi, lo)) in [(&mut before, r.before), (&mut here, r.here)] {
            for h in (0..16).filter(|h| hi >> h & 1 == 1) {
                t.hi[h] |= 1 << k;
            }
            for l in (0..16).filter(|l| lo >> l & 1 == 1) {
                t.lo[l] |= 1 << k;
            }
        }
    }
    // Every rule's set of this byte all its low nibbles: the low half all ones, looked up never.
    if kinds.iter().all(|r| r.here.1 == 0xffff) {
        here.lo = [0xff; 16];
    }
    let mut out = native.clone();
    let p = out.tables.len() as u16;
    out.tables.extend([before, here]);
    let pair = |bits: u8| BlockInst::Pair { before: p, table: p + 1, bits };
    // The rules ORed in one tree: one pair of their bits.
    let one_reader = |x: usize| readers[x] == 1 && !output[x] && !grouped[x];
    let inner = |x: usize| matches!(native.insts[x], BlockInst::Or(..)) && one_reader(x);
    let mut inner_of = vec![false; n];
    for inst in &native.insts {
        if let BlockInst::Or(a, b) = *inst {
            for o in [usize::from(a), usize::from(b)] {
                inner_of[o] = inner_of[o] || inner(o);
            }
        }
    }
    let mut done = vec![false; n];
    for root in (0..n).filter(|&x| matches!(native.insts[x], BlockInst::Or(..)) && !inner_of[x] && !grouped[x]) {
        let mut rules = Vec::new();
        let mut stack = vec![root];
        while let Some(x) = stack.pop() {
            match native.insts[x] {
                BlockInst::Or(a, b) if x == root || inner(x) => stack.extend([usize::from(a), usize::from(b)]),
                _ if bit_of[x].is_some() && one_reader(x) => rules.push(x),
                _ => {}
            }
        }
        rules.sort_unstable();
        if rules.len() < 2 {
            continue;
        }
        let bits = rules.iter().fold(0u8, |b, &x| b | bit_of[x].expect("a rule"));
        out.insts[rules[0]] = pair(bits);
        for &x in &rules[1..] {
            out.insts[x] = BlockInst::Const(0);
        }
        for &x in &rules {
            done[x] = true;
        }
    }
    // Every other rule: its own pair, its lookups shared with the rest.
    for x in (0..n).filter(|&x| !done[x]) {
        if let Some(bit) = bit_of[x] {
            out.insts[x] = pair(bit);
        }
    }
    Some(out)
}

/// Value `x` as a rule: `and(prev(L, 1), C)`, or `and_not(C, prev(L, 1))` (the byte before not
/// in `L`).
fn rule(native: &BlockNative, x: usize) -> Option<Rule> {
    let prev_of = |y: usize| match native.insts[y] {
        BlockInst::Prev(l, 1) => Some(through(native, usize::from(l))),
        _ => None,
    };
    let (l, c, outside) = match native.insts[x] {
        BlockInst::And(a, b) => {
            let (a, b) = (usize::from(a), usize::from(b));
            match (prev_of(a), prev_of(b)) {
                (Some(l), _) => (l, through(native, b), false),
                (_, Some(l)) => (l, through(native, a), false),
                _ => return None,
            }
        }
        BlockInst::AndNot(a, b) => (prev_of(usize::from(b))?, through(native, usize::from(a)), true),
        _ => return None,
    };
    let l = set(native, l)?;
    if l[0] {
        return None;
    }
    let before = if outside { std::array::from_fn(|b| !l[b]) } else { l };
    Some(Rule { before: product(&before)?, here: product(&set(native, c)?)? })
}

/// `x` past an `and_not` of zero: a class cut by the prefix the host strips (0389), whose
/// prefix is then the constant zero.
fn through(native: &BlockNative, mut x: usize) -> usize {
    while let BlockInst::AndNot(a, z) = native.insts[x]
        && native.insts[usize::from(z)] == BlockInst::Const(0)
    {
        x = usize::from(a);
    }
    x
}

/// Byte class `x`'s bytes (an `eq` or a `class`), `None` for any other value.
fn set(native: &BlockNative, x: usize) -> Option<[bool; 256]> {
    match native.insts[x] {
        BlockInst::EqByte(c) => Some(std::array::from_fn(|b| b == usize::from(c))),
        BlockInst::Class { table, bits } => Some(std::array::from_fn(|b| native.tables[usize::from(table)].member(b as u8, bits))),
        _ => None,
    }
}

/// A set as one product, its high nibbles and its low ones, if it is one and not empty.
fn product(set: &[bool; 256]) -> Option<(u16, u16)> {
    let (hi, lo) = (0..256).filter(|&b| set[b]).fold((0u16, 0u16), |(h, l), b| (h | 1 << (b >> 4), l | 1 << (b & 15)));
    let all = (0..256).all(|b| set[b] == (hi >> (b >> 4) & 1 == 1 && lo >> (b & 15) & 1 == 1));
    (hi != 0 && all).then_some((hi, lo))
}
