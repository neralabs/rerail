//! Byte-pair tests made one lookup, as simdjson's UTF-8 check makes its two-byte rules
//! (`check_special_cases`: the byte before's two nibbles and this byte's high nibble, looked up
//! and ANDed, each rule a bit).
//!
//! A grammar states such a rule as `prev(A, 1) & B`, with `A` and `B` byte classes: "after an
//! `0xe0`, a byte below `0xa0`". Each one is computed apart: `A` on the bytes, shifted one byte
//! (an `ext` a chunk), `B`, and an `and`. Several of them ORed together, as JSON's UTF-8 check's
//! four (`0xe0`, `0xed`, `0xf0`, `0xf4` and the byte after each), are one `pair`
//! (`BlockInst::Pair`): a table of the byte before's classes, one bit a rule, ANDed with the
//! table of this byte's classes, then one test of the rules' bits.
//!
//! The rewrite is structural, on the CPU's program (`native_cpu_block::compile`), after the host
//! strips the prefix. The interpreter runs the program as written, so it is the rewrite's
//! oracle. A rule is a term `and(prev(L, 1), C)` where:
//!
//! - `L` is one byte, or a class that is one product (a set of high nibbles by a set of low
//!   ones), without 0x00: before a file a `prev` reads no bit, and a pair reads the byte 0x00;
//! - `C` is one bit of a table: the pair's table, so its lookup is the one `C` had.
//!
//! The terms ORed in one tree (every inner `or` read by the tree only, every term by its `or`
//! only, none an output or a fixed point's) are then one pair: the first term becomes it, the
//! others zero, so the tree's value is the same. Two terms at least: one alone costs a lookup
//! more than it saves.

use crate::block::ClassTable;
use crate::block_native::{BlockInst, BlockNative};

/// `native` with its byte-pair rules made pairs, or `None` where it has no two to merge.
pub fn fused(native: &BlockNative) -> Option<BlockNative> {
    let n = native.insts.len();
    let mut readers = vec![0usize; n];
    for inst in &native.insts {
        for o in inst.operands() {
            readers[o] += 1;
        }
    }
    let mut pinned = vec![false; n];
    for &o in &native.outputs {
        pinned[usize::from(o)] = true;
    }
    for g in &native.groups {
        for &m in &g.members {
            pinned[m] = true;
        }
        for &(c, x) in &g.carries {
            pinned[c] = true;
            pinned[x] = true;
        }
    }
    let one_reader = |x: usize| readers[x] == 1 && !pinned[x];
    let mut out = native.clone();
    let mut changed = false;
    for root in 0..n {
        if !matches!(native.insts[root], BlockInst::Or(..)) {
            continue;
        }
        // A root: not itself an inner `or` of a larger tree.
        let inner = |x: usize| matches!(native.insts[x], BlockInst::Or(..)) && one_reader(x);
        if (0..n).any(|y| matches!(native.insts[y], BlockInst::Or(a, b) if (usize::from(a) == root || usize::from(b) == root) && inner(root))) {
            continue;
        }
        let mut leaves = Vec::new();
        let mut stack = vec![root];
        while let Some(x) = stack.pop() {
            match native.insts[x] {
                BlockInst::Or(a, b) if x == root || inner(x) => stack.extend([usize::from(a), usize::from(b)]),
                _ => leaves.push(x),
            }
        }
        leaves.sort_unstable();
        // The terms, by the table of their `C`: (term, `L`'s high and low nibbles, `C`'s bit).
        let mut by_table: Vec<(u16, Vec<(usize, u16, u16, u8)>)> = Vec::new();
        for &x in leaves.iter().filter(|&&x| one_reader(x)) {
            let Some((table, nibbles, bit)) = term(native, x) else { continue };
            let at = match by_table.iter().position(|(t, _)| *t == table) {
                Some(at) => at,
                None => {
                    by_table.push((table, Vec::new()));
                    by_table.len() - 1
                }
            };
            // One `L` a bit: a second term on a bit already taken stays as it is.
            if by_table[at].1.iter().all(|t| t.3 != bit) {
                by_table[at].1.push((x, nibbles.0, nibbles.1, bit));
            }
        }
        for (table, terms) in by_table.into_iter().filter(|(_, t)| t.len() >= 2) {
            let mut before = ClassTable::default();
            let mut bits = 0u8;
            for &(_, hi, lo, bit) in &terms {
                for h in (0..16).filter(|h| hi >> h & 1 == 1) {
                    before.hi[h] |= bit;
                }
                for l in (0..16).filter(|l| lo >> l & 1 == 1) {
                    before.lo[l] |= bit;
                }
                bits |= bit;
            }
            let before = match out.tables.iter().position(|t| *t == before) {
                Some(t) => t,
                None => {
                    out.tables.push(before);
                    out.tables.len() - 1
                }
            };
            out.insts[terms[0].0] = BlockInst::Pair { before: before as u16, table, bits };
            for &(x, ..) in &terms[1..] {
                out.insts[x] = BlockInst::Const(0);
            }
            changed = true;
        }
    }
    changed.then_some(out)
}

/// Term `x` as a rule: `C`'s table, `L`'s high and low nibbles (a product), and `C`'s bit.
fn term(native: &BlockNative, x: usize) -> Option<(u16, (u16, u16), u8)> {
    let BlockInst::And(a, b) = native.insts[x] else { return None };
    let (a, b) = (usize::from(a), usize::from(b));
    let (p, c) = match (native.insts[a], native.insts[b]) {
        (BlockInst::Prev(l, 1), _) => (through(native, usize::from(l)), through(native, b)),
        (_, BlockInst::Prev(l, 1)) => (through(native, usize::from(l)), through(native, a)),
        _ => return None,
    };
    let BlockInst::Class { table, bits } = native.insts[c] else { return None };
    if bits.count_ones() != 1 {
        return None;
    }
    let set: Vec<u8> = (0..=255u8).filter(|&b| holds(native, p, b) == Some(true)).collect();
    if set.is_empty() || (0..=255u8).any(|b| holds(native, p, b).is_none()) || set.contains(&0) {
        return None;
    }
    // A product: every high nibble of the set with every low nibble of it.
    let (hi, lo) = set.iter().fold((0u16, 0u16), |(h, l), &b| (h | 1 << (b >> 4), l | 1 << (b & 15)));
    let product = (0..16).filter(|h| hi >> h & 1 == 1).all(|h| (0..16).filter(|l| lo >> l & 1 == 1).all(|l| set.contains(&(h << 4 | l))));
    product.then_some((table, (hi, lo), bits))
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

/// Whether byte `b` is in byte class `x` (an `eq` or a `class`), `None` for any other value.
fn holds(native: &BlockNative, x: usize, b: u8) -> Option<bool> {
    match native.insts[x] {
        BlockInst::EqByte(c) => Some(b == c),
        BlockInst::Class { table, bits } => Some(native.tables[usize::from(table)].member(b, bits)),
        _ => None,
    }
}
