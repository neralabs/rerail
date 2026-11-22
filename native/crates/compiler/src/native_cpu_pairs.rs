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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
    let is_rule: Vec<Option<Rule>> = (0..n).map(|x| if grouped[x] { None } else { rule(native, x) }).collect();
    // The rules ORed in one tree (every inner `or` read by the tree only, every rule by its `or`
    // only): one group, its boxes merged. Every other rule: a group of its own.
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
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut in_tree = vec![false; n];
    for root in (0..n).filter(|&x| matches!(native.insts[x], BlockInst::Or(..)) && !inner_of[x] && !grouped[x]) {
        let mut rules = Vec::new();
        let mut stack = vec![root];
        while let Some(x) = stack.pop() {
            match native.insts[x] {
                BlockInst::Or(a, b) if x == root || inner(x) => stack.extend([usize::from(a), usize::from(b)]),
                _ if is_rule[x].is_some() && one_reader(x) => rules.push(x),
                _ => {}
            }
        }
        rules.sort_unstable();
        if rules.len() >= 2 {
            for &x in &rules {
                in_tree[x] = true;
            }
            groups.push(rules);
        }
    }
    groups.extend((0..n).filter(|&x| is_rule[x].is_some() && !in_tree[x]).map(|x| vec![x]));
    // Each group's boxes, and their bits: one a box, eight at most, a box shared where two
    // groups have it. A group that no longer fits is left as it is.
    let mut boxes: Vec<Box> = Vec::new();
    let mut fused: Vec<(Vec<usize>, u8)> = Vec::new();
    for g in groups {
        let mine = merged(g.iter().map(|&x| is_rule[x].expect("a rule").as_box()).collect());
        let mut taken = boxes.clone();
        let mut bits = 0u8;
        for b in mine {
            let k = match taken.iter().position(|&t| t == b) {
                Some(k) => k,
                None => {
                    taken.push(b);
                    taken.len() - 1
                }
            };
            bits |= 1u8.checked_shl(k as u32).unwrap_or(0);
        }
        if taken.len() <= 8 {
            boxes = taken;
            fused.push((g, bits));
        }
    }
    if fused.iter().map(|(g, _)| g.len()).sum::<usize>() < 2 {
        return None;
    }
    let (mut before, mut here) = (ClassTable::default(), ClassTable::default());
    for (k, b) in boxes.iter().enumerate() {
        for (t, hi, lo) in [(&mut before, b[0], b[1]), (&mut here, b[2], b[3])] {
            for h in (0..16).filter(|h| hi >> h & 1 == 1) {
                t.hi[h] |= 1 << k;
            }
            for l in (0..16).filter(|l| lo >> l & 1 == 1) {
                t.lo[l] |= 1 << k;
            }
        }
    }
    // Every box's set of this byte all its low nibbles: the low half all ones, looked up never.
    if boxes.iter().all(|b| b[3] == 0xffff) {
        here.lo = [0xff; 16];
    }
    let mut out = native.clone();
    let p = out.tables.len() as u16;
    out.tables.extend([before, here]);
    // A group's first rule its pair, the others zero: its tree's value, or its own.
    for (g, bits) in fused {
        out.insts[g[0]] = BlockInst::Pair { before: p, table: p + 1, bits };
        for &x in &g[1..] {
            out.insts[x] = BlockInst::Const(0);
        }
    }
    Some(out)
}

/// A box of byte pairs: the byte before's high nibbles and low ones, this byte's high nibbles
/// and low ones. A pair table's bit holds one.
pub type Box = [u16; 4];

impl Rule {
    fn as_box(self) -> Box {
        [self.before.0, self.before.1, self.here.0, self.here.1]
    }
}

/// The boxes of one tree's rules, fewer: two that differ in one of the four sets are one, their
/// union in it; and a box is split where a part of it would be one with another box, and the
/// rest with a third, as simdjson's tables share `TOO_LARGE` between `f4` and `f5..ff`. The
/// union of the boxes is the same, and the tree reads only their union.
pub fn merged(mut v: Vec<Box>) -> Vec<Box> {
    let differ = |a: &Box, b: &Box| (0..4).filter(|&d| a[d] != b[d]).collect::<Vec<usize>>();
    loop {
        let mut changed = false;
        'merge: for i in 0..v.len() {
            for j in i + 1..v.len() {
                let ds = differ(&v[i], &v[j]);
                if ds.len() <= 1 {
                    if let Some(&d) = ds.first() {
                        v[i][d] |= v[j][d];
                    }
                    v.remove(j);
                    changed = true;
                    break 'merge;
                }
            }
        }
        if changed {
            continue;
        }
        // Split box `i` along `f` to `j`'s set there (a strict subset of `i`'s): the part is one
        // with `j` along `d`, and the rest must be one with a third box.
        'split: for i in 0..v.len() {
            for j in (0..v.len()).filter(|&j| j != i) {
                let ds = differ(&v[i], &v[j]);
                if ds.len() != 2 {
                    continue;
                }
                for (f, d) in [(ds[0], ds[1]), (ds[1], ds[0])] {
                    if v[j][f] & !v[i][f] != 0 {
                        continue;
                    }
                    let mut rest = v[i];
                    rest[f] &= !v[j][f];
                    if (0..v.len()).any(|k| k != i && k != j && differ(&rest, &v[k]).len() == 1) {
                        let mut part = v[i];
                        part[f] = v[j][f];
                        part[d] |= v[j][d];
                        v[j] = part;
                        v[i] = rest;
                        changed = true;
                        break 'split;
                    }
                }
            }
        }
        if !changed {
            return v;
        }
    }
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
pub fn product(set: &[bool; 256]) -> Option<(u16, u16)> {
    let (hi, lo) = (0..256).filter(|&b| set[b]).fold((0u16, 0u16), |(h, l), b| (h | 1 << (b >> 4), l | 1 << (b & 15)));
    let all = (0..256).all(|b| set[b] == (hi >> (b >> 4) & 1 == 1 && lo >> (b & 15) & 1 == 1));
    (hi != 0 && all).then_some((hi, lo))
}
