//! What a chunk's start is made of, and which of it a chunk can read in place.
//!
//! A start (`chunk::Start`) has one component a carry. Some are **read in place**: a `prev` of a
//! value made from the bytes alone (no carried operation upstream) carries that value's top bits
//! at the block before, which the chunk computes from the bytes before it, as it reads its own.
//! The rest are **state**: what the chunk must solve for (a toggle, a fill, a region group's
//! members, a back fill, a `prev` of anything carried).

use crate::chunk::{File, Prepared, Start};
use rerail_compiler::block::{self, BlockOp};

/// One component of a start: where it is held, and whether it is read in place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Component {
    pub at: Place,
    /// The operation that carries it.
    pub op: usize,
    /// Read from the bytes before the chunk, not solved for.
    pub in_place: bool,
}

/// Where a component is held in a `Start`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Op(usize),
    Group(usize, usize),
    Back(usize),
}

impl Place {
    pub fn get(self, s: &Start) -> u64 {
        match self {
            Place::Op(i) => s.ops[i],
            Place::Group(g, k) => s.groups[g][k],
            Place::Back(i) => s.backs[i],
        }
    }

    pub fn set(self, s: &mut Start, v: u64) {
        match self {
            Place::Op(i) => s.ops[i] = v,
            Place::Group(g, k) => s.groups[g][k] = v,
            Place::Back(i) => s.backs[i] = v,
        }
    }
}

/// Each value made from the bytes alone: no carried operation and no group member upstream.
pub fn pure_values(p: &Prepared<'_>) -> Vec<bool> {
    let ops = &p.program.ops;
    let mut member = vec![false; ops.len()];
    for g in &p.groups {
        for &x in g.members.iter() {
            member[x] = true;
        }
    }
    let mut pure = vec![false; ops.len()];
    for x in 0..ops.len() {
        pure[x] = !member[x]
            && match ops[x] {
                BlockOp::Prev(..) | BlockOp::Toggle(_) | BlockOp::Escaped(_) | BlockOp::Fill(..) | BlockOp::BackFill(..) | BlockOp::Carry(_) => false,
                ref op => op.operands().iter().all(|&o| pure[usize::from(o)]),
            };
    }
    pure
}

/// Every component of a start, in `Start`'s order: the carried operations, the groups' members
/// that carry, the back fills.
pub fn components(p: &Prepared<'_>) -> Vec<Component> {
    let ops = &p.program.ops;
    let pure = pure_values(p);
    let mut out = Vec::new();
    for (i, &x) in p.carried.iter().enumerate() {
        let in_place = matches!(ops[x], BlockOp::Prev(a, _) if pure[usize::from(a)]);
        out.push(Component { at: Place::Op(i), op: x, in_place });
    }
    for (g, group) in p.groups.iter().enumerate() {
        for (k, &x) in group.members.iter().enumerate() {
            if matches!(ops[x], BlockOp::Prev(..) | BlockOp::Toggle(_) | BlockOp::Escaped(_) | BlockOp::Fill(..)) {
                out.push(Component { at: Place::Group(g, k), op: x, in_place: false });
            }
        }
    }
    for (i, &x) in p.backs.iter().enumerate() {
        out.push(Component { at: Place::Back(i), op: x, in_place: false });
    }
    out
}

/// The components read in place at block `at` of `file`: each from its value at block `at - 1`,
/// computed from that block's bytes alone (zero at the file's first block).
pub fn in_place(p: &Prepared<'_>, comps: &[Component], file: &File, at: usize, into: &mut Start) {
    if at == 0 {
        for c in comps.iter().filter(|c| c.in_place) {
            c.at.set(into, 0);
        }
        return;
    }
    let pure = pure_values(p);
    let ops = &p.program.ops;
    // The pure values over the one block before: a run of the program's pure part.
    let mut word = vec![0u64; ops.len()];
    let (b, before, len) = (&file.blocks[at - 1], file.before_of(at - 1), file.len);
    for x in 0..ops.len() {
        if !pure[x] {
            continue;
        }
        let v = |a: u16| word[usize::from(a)];
        word[x] = match ops[x] {
            BlockOp::Within(d) => block::within_at(at - 1, len, d),
            BlockOp::Prefix(n, w) => block::prefix_at(b, at - 1, len, n, w),
            BlockOp::Phase(k, r) => block::phase(64 * (at - 1), k, r),
            BlockOp::Position(j) => block::position(at - 1, j),
            BlockOp::Eq(byte) => block::eq(b, byte),
            BlockOp::Class { table, bits } => block::class(b, &p.program.tables[usize::from(table)], bits),
            BlockOp::Pair { before: t, table, bits } => block::pair(before, b, &p.program.tables[usize::from(t)], &p.program.tables[usize::from(table)], bits),
            BlockOp::Const(c) => c,
            BlockOp::Not(a) => !v(a),
            BlockOp::And(a, c) => v(a) & v(c),
            BlockOp::Or(a, c) => v(a) | v(c),
            BlockOp::Xor(a, c) => v(a) ^ v(c),
            BlockOp::AndNot(a, c) => v(a) & !v(c),
            BlockOp::Early(a, k) => v(a) >> k,
            _ => unreachable!("not pure"),
        };
    }
    for c in comps.iter().filter(|c| c.in_place) {
        let BlockOp::Prev(a, k) = ops[c.op] else { unreachable!("an in-place component is a prev") };
        c.at.set(into, if k == 0 { 0 } else { word[usize::from(a)] >> (64 - u32::from(k)) });
    }
}

/// A start's state: its components that are not read in place, in order.
pub fn state(comps: &[Component], s: &Start) -> Vec<u64> {
    comps.iter().filter(|c| !c.in_place).map(|c| c.at.get(s)).collect()
}
