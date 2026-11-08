//! Tier 1's byte-local values on the block's byte vectors, before any mask is made (the CPU JIT,
//! `native_cpu_block`), as simdjson's stage 1 keeps its classes in vector registers.
//!
//! A value is **byte-local** when each of its bytes is a function of the block's bytes at most 16
//! back: a byte test, a class, a plane, the logic of byte-local values, and a `prev` by up to 16
//! of one without a `prev` of its own. Such a value is computed per 16-byte chunk:
//!
//! - **Shared lookups.** A table's two nibble lookups are made once per chunk, for every class
//!   taken from it.
//! - **A `prev` on bytes.** `prev(v, k)` is `v`'s chunk and the chunk before it, shifted by `k`
//!   bytes (one `ext` on aarch64). The chunk before the block's first is `v` over the 16 bytes
//!   before the block, which are the previous block's, or zeros before a file. No carried word.
//! - **One mask at the edge.** A value becomes a 64-bit mask only where a value outside the
//!   vectors, an output or a carried field reads it.
//!
//! The plan (`Plan::of`) also says which values are known on a window of ASCII bytes, for the
//! full-block body's one branch: when the block and the 16 bytes before it are all below 0x80,
//! a body with those values folded runs (its UTF-8 checks, typically, are all zero).

use super::block::ClassTable;
use super::native_cpu_block::{Sym, bitmask};
use super::native_ir::{BlockInst, BlockNative};
use cranelift_codegen::ir::{ConstantData, InstBuilder, MemFlags, Value, condcodes::IntCC, types};
use cranelift_frontend::FunctionBuilder;

/// How far back a byte-local value may look: the bytes before a file (`FRONT`).
const MAX_LAG: u8 = 16;

/// Which instructions are computed on the byte vectors, and what is known of them.
#[derive(Debug)]
pub(super) struct Plan {
    /// Computed on the byte vectors.
    pub(super) lanes: Vec<bool>,
    /// Made a 64-bit mask too: read outside the vectors, an output, or a carried field's value.
    pub(super) mask: Vec<bool>,
    /// Can be shifted by a `prev`: byte-local with no `prev`, `pair` or `plane_before` in it.
    shiftable: Vec<bool>,
    /// May be set on a byte of 0x00 (the zeros before a file), for a shiftable value.
    at_zero: Vec<bool>,
    /// Its value on every window of ASCII bytes, when it is the same on all of them.
    ascii: Vec<Option<bool>>,
    /// Some value reads the 16 bytes before the block.
    pub(super) before: bool,
    /// The full-block body has an ASCII twin: some value is known on ASCII windows.
    pub(super) ascii_branch: bool,
    /// Each value's word in a full block, when it is the same in every one: made of constants
    /// and `within` only (all ones there), `prev`s of such values included.
    pub(super) full: Vec<Option<u64>>,
    /// The gated regions of the full body (`Region`), in the order they were chosen.
    pub(super) regions: Vec<Region>,
    /// Each value's region, if it is in one.
    pub(super) region_of: Vec<Option<usize>>,
}

/// A gated region: values that are zero in a block where the gate is zero and their own carried
/// bits came in zero, and values only they read. The full body computes them behind one branch,
/// so a block with nothing for them (a block without numbers, for a number check) skips them,
/// their class masks and their `prev`s included.
#[derive(Debug)]
pub(super) struct Region {
    pub(super) gate: usize,
    /// The values provably zero with the gate (the gate itself not among them).
    pub(super) zero: Vec<usize>,
    /// Every value the region computes, in order: `zero`, and values only the region reads.
    pub(super) members: Vec<usize>,
}

/// What one operation costs, roughly, in instructions: for choosing the regions worth a branch.
fn weight(inst: &BlockInst, lane: bool, mask: bool) -> usize {
    match inst {
        _ if lane => if mask { 12 } else { 0 },
        BlockInst::Const(_) | BlockInst::Within(_) => 0,
        BlockInst::Prev(..) => 6,
        BlockInst::Fill(..) => 9,
        BlockInst::Toggle(_) => 14,
        BlockInst::Escaped(_) => 14,
        _ => 1,
    }
}

/// What may be true of a value over windows of ASCII bytes: whether it can be 0, whether 1.
#[derive(Clone, Copy)]
struct Can {
    zero: bool,
    one: bool,
}

fn class_byte(t: &ClassTable, b: u8) -> u8 {
    t.lo[usize::from(b & 15)] & t.hi[usize::from(b >> 4)]
}

impl Plan {
    pub(super) fn of(native: &BlockNative) -> Plan {
        let n = native.insts.len();
        let (mut lanes, mut shiftable, mut at_zero) = (vec![false; n], vec![false; n], vec![false; n]);
        let mut can = vec![Can { zero: true, one: true }; n];
        for (at, inst) in native.insts.iter().enumerate() {
            let o = |x: u16| usize::from(x);
            let (l, s, z, c) = match *inst {
                BlockInst::Const(0) => (true, true, false, Can { zero: true, one: false }),
                BlockInst::Const(u64::MAX) => (true, true, true, Can { zero: false, one: true }),
                BlockInst::EqByte(b) => (true, true, b == 0, Can { zero: true, one: b < 0x80 }),
                BlockInst::Class { table, bits } => {
                    let t = &native.tables[usize::from(table)];
                    let one = (0..0x80u8).any(|b| class_byte(t, b) & bits != 0);
                    let zero = (0..0x80u8).any(|b| class_byte(t, b) & bits == 0);
                    (true, true, class_byte(t, 0) & bits != 0, Can { zero, one })
                }
                BlockInst::Plane(k) => (true, true, false, Can { zero: true, one: k < 7 }),
                BlockInst::PlaneBefore(k) => (true, false, false, Can { zero: true, one: k < 7 }),
                BlockInst::Pair { before, table, bits } => {
                    let (p, t) = (&native.tables[usize::from(before)], &native.tables[usize::from(table)]);
                    let ps = (0..0x80u8).fold(0, |a, b| a | class_byte(p, b));
                    let ts = (0..0x80u8).fold(0, |a, b| a | class_byte(t, b));
                    (true, false, false, Can { zero: true, one: ps & ts & bits != 0 })
                }
                BlockInst::Not(a) => (lanes[o(a)], shiftable[o(a)], !at_zero[o(a)], Can { zero: can[o(a)].one, one: can[o(a)].zero }),
                BlockInst::And(a, b) | BlockInst::Or(a, b) | BlockInst::Xor(a, b) | BlockInst::AndNot(a, b) => {
                    let (x, y) = (can[o(a)], can[o(b)]);
                    let (zx, zy) = (at_zero[o(a)], at_zero[o(b)]);
                    let (z, c) = match *inst {
                        BlockInst::And(..) => (zx && zy, Can { zero: x.zero || y.zero, one: x.one && y.one }),
                        BlockInst::Or(..) => (zx || zy, Can { zero: x.zero && y.zero, one: x.one || y.one }),
                        BlockInst::Xor(..) => (zx != zy, Can { zero: (x.zero && y.zero) || (x.one && y.one), one: (x.one && y.zero) || (x.zero && y.one) }),
                        _ => (zx && !zy, Can { zero: x.zero || y.one, one: x.one && y.zero }),
                    };
                    (lanes[o(a)] && lanes[o(b)], shiftable[o(a)] && shiftable[o(b)], z, c)
                }
                BlockInst::Prev(a, k) => (lanes[o(a)] && shiftable[o(a)] && k <= MAX_LAG, false, false, can[o(a)]),
                _ => (false, false, false, Can { zero: true, one: true }),
            };
            (lanes[at], shiftable[at], at_zero[at], can[at]) = (l, s, z, c);
        }
        // Each value's readers.
        let mut users: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (at, inst) in native.insts.iter().enumerate() {
            for x in inst.operands() {
                users[x].push(at);
            }
        }
        let mut output = vec![false; n];
        for &v in &native.outputs {
            output[usize::from(v)] = true;
        }
        // On the vectors only where it saves a mask or a carried field: a `prev` or logic none of
        // whose readers is on the vectors, and whose operands are masks anyway, is cheaper as a
        // mask's operation.
        let needs_mask = |lanes: &[bool], x: usize| lanes[x] && (output[x] || users[x].iter().any(|&u| !lanes[u]));
        loop {
            let mut changed = false;
            for (at, inst) in native.insts.iter().enumerate() {
                if !lanes[at] || users[at].iter().any(|&u| lanes[u]) {
                    continue;
                }
                let demote = match *inst {
                    // A `prev` read only as a mask: on the vectors it shifts bytes and needs no
                    // carried field, so a gated region can skip it and its operand. Only where the
                    // operand is made a mask anyway, for a reader other than `prev`s, is the
                    // mask shifted instead.
                    BlockInst::Prev(a, _) => {
                        let a = usize::from(a);
                        output[a] || users[a].iter().any(|&u| u != at && !lanes[u] && !matches!(native.insts[u], BlockInst::Prev(..)))
                    }
                    BlockInst::Not(_) | BlockInst::And(..) | BlockInst::Or(..) | BlockInst::Xor(..) | BlockInst::AndNot(..) => {
                        inst.operands().into_iter().all(|x| needs_mask(&lanes, x) || matches!(native.insts[x], BlockInst::Const(_)))
                    }
                    _ => false,
                };
                if demote {
                    lanes[at] = false;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        let mask: Vec<bool> = (0..n).map(|x| needs_mask(&lanes, x)).collect();
        let ascii: Vec<Option<bool>> = can.iter().map(|c| if !c.one { Some(false) } else if !c.zero { Some(true) } else { None }).collect();
        let before = native.insts.iter().enumerate().any(|(at, i)| lanes[at] && matches!(i, BlockInst::Prev(..)));
        // Worth a branch: a byte test or class known on ASCII bytes (not a constant).
        let ascii_branch = native.insts.iter().enumerate().any(|(at, i)| lanes[at] && ascii[at].is_some() && matches!(i, BlockInst::EqByte(_) | BlockInst::Class { .. } | BlockInst::Pair { .. } | BlockInst::Plane(_) | BlockInst::PlaneBefore(_)));
        let full = full_words(native, &lanes);
        // The values with a carried field: a carry of their own, or a `prev` of them on masks.
        let field: Vec<bool> = (0..n)
            .map(|x| matches!(native.insts[x], BlockInst::Toggle(_) | BlockInst::Fill(..) | BlockInst::Escaped(_)) || users[x].iter().any(|&u| matches!(native.insts[u], BlockInst::Prev(..)) && !lanes[u]))
            .collect();
        let (regions, region_of) = regions(native, &lanes, &mask, &users, &output, &field, &full);
        Plan { lanes, mask, shiftable, at_zero, ascii, before, ascii_branch, full, regions, region_of }
    }

    /// Whether `prev` instruction `at` reads a carried word (it is not on the vectors).
    pub(super) fn carries(&self, at: usize) -> bool {
        !self.lanes[at]
    }
}

/// Each value's word in a full block when it is the same in every full block. A full block's
/// `within` is all ones; the block before it is full too, or the file's first, where `within d`
/// lacks its low `d` bits and a `prev` shifts in zeros. So the bits known of each value are
/// followed in both kinds of block, and a `prev` in a full block knows the bits it shifts in when
/// the top bits are known, and the same, in both.
fn full_words(native: &BlockNative, lanes: &[bool]) -> Vec<Option<u64>> {
    /// The bits known of a value, and their values.
    #[derive(Clone, Copy)]
    struct Known {
        known: u64,
        value: u64,
    }
    const NONE: Known = Known { known: 0, value: 0 };
    let all = |c: u64| Known { known: u64::MAX, value: c };
    let not = |x: Known| Known { known: x.known, value: !x.value & x.known };
    let and = |x: Known, y: Known| {
        let ones = x.known & x.value & y.known & y.value;
        let zeros = (x.known & !x.value) | (y.known & !y.value);
        Known { known: ones | zeros, value: ones }
    };
    let or = |x: Known, y: Known| not(and(not(x), not(y)));
    let xor = |x: Known, y: Known| Known { known: x.known & y.known, value: (x.value ^ y.value) & x.known & y.known };
    let n = native.insts.len();
    // (in a full block, in the first block)
    let mut w: Vec<(Known, Known)> = vec![(NONE, NONE); n];
    for (at, inst) in native.insts.iter().enumerate() {
        let get = |x: u16| w[usize::from(x)];
        let both = |f: &dyn Fn(Known, Known) -> Known, a: u16, b: u16| {
            let ((xa, ya), (xb, yb)) = (get(a), get(b));
            (f(xa, xb), f(ya, yb))
        };
        w[at] = match *inst {
            BlockInst::Const(c) => (all(c), all(c)),
            _ if lanes[at] => (NONE, NONE),
            BlockInst::Within(d) => (all(u64::MAX), all(if d == 0 { u64::MAX } else { !((1u64 << d) - 1) })),
            BlockInst::Not(a) => {
                let (x, y) = get(a);
                (not(x), not(y))
            }
            BlockInst::And(a, b) => both(&and, a, b),
            BlockInst::Or(a, b) => both(&or, a, b),
            BlockInst::Xor(a, b) => both(&xor, a, b),
            BlockInst::AndNot(a, b) => both(&|x, y| and(x, not(y)), a, b),
            BlockInst::Prev(a, k) => {
                let (x, y) = get(a);
                let low = (1u64 << k) - 1;
                let top = |c: u64| c >> (64 - u32::from(k));
                // Shifted in: the last block's top bits, known where both kinds agree on them.
                let same = top(x.known) & top(y.known) & !(top(x.value) ^ top(y.value));
                let full = Known { known: (x.known << k) | same, value: (x.value << k) | (top(x.value) & same) };
                let first = Known { known: (y.known << k) | low, value: y.value << k };
                (full, first)
            }
            _ => (NONE, NONE),
        };
    }
    w.into_iter().map(|(full, _)| (full.known == u64::MAX).then_some(full.value)).collect()
}

/// The gated regions (`Region`). A gate is an `and` of a mask with a byte class, as a token's
/// start and the bytes a check takes: sparse. A value is zero with it when, in a block where the
/// gate is zero and the carried bits of these values came in zero, it must be: an `and` with
/// one, an `and not` of one, an `or` or `xor` of two, a `prev` of one, a `fill` from one, a
/// `toggle` or `escaped` of one. The regions worth a branch are chosen greedily, the largest
/// first, each without the values of those before it.
fn regions(native: &BlockNative, lanes: &[bool], mask: &[bool], users: &[Vec<usize>], output: &[bool], field: &[bool], full: &[Option<u64>]) -> (Vec<Region>, Vec<Option<usize>>) {
    const WORTH: usize = 24;
    const MOST: usize = 4;
    let n = native.insts.len();
    let mut region_of: Vec<Option<usize>> = vec![None; n];
    let mut gates = vec![false; n];
    let mut out: Vec<Region> = Vec::new();
    let zeros = |g: usize, region_of: &[Option<usize>]| -> Vec<bool> {
        let mut z = vec![false; n];
        z[g] = true;
        for (at, inst) in native.insts.iter().enumerate().skip(g + 1) {
            if lanes[at] || region_of[at].is_some() || full[at].is_some() {
                continue;
            }
            let is = |x: u16| z[usize::from(x)] || matches!(native.insts[usize::from(x)], BlockInst::Const(0));
            z[at] = match *inst {
                BlockInst::And(a, b) => is(a) || is(b),
                BlockInst::AndNot(a, _) | BlockInst::Prev(a, _) | BlockInst::Fill(a, _) | BlockInst::Toggle(a) | BlockInst::Escaped(a) => is(a),
                BlockInst::Or(a, b) | BlockInst::Xor(a, b) => is(a) && is(b),
                _ => false,
            };
        }
        z[g] = false;
        z
    };
    for _ in 0..MOST {
        let mut worthy: Vec<(usize, usize, Vec<bool>, Vec<bool>)> = Vec::new();
        for (g, inst) in native.insts.iter().enumerate() {
            let candidate = !lanes[g]
                && region_of[g].is_none()
                && !gates[g]
                && match *inst {
                    // Inside a mask that is not a byte class and not the same in every full block:
                    // a quote's bits, say, are in nearly every block.
                    BlockInst::And(a, b) => lanes[usize::from(a)] != lanes[usize::from(b)] && [a, b].iter().all(|&x| lanes[usize::from(x)] || full[usize::from(x)].is_none()),
                    BlockInst::AndNot(a, b) => !lanes[usize::from(a)] && lanes[usize::from(b)] && full[usize::from(a)].is_none(),
                    _ => false,
                };
            if !candidate {
                continue;
            }
            let z = zeros(g, &region_of);
            // The values only the region reads, with no carried field of their own: made inside it.
            let mut only = vec![false; n];
            for x in (0..n).rev() {
                if z[x] || x == g || gates[x] || region_of[x].is_some() || output[x] || field[x] || users[x].is_empty() || (lanes[x] && !mask[x]) {
                    continue;
                }
                let inside = |u: usize| z[u] || only[u];
                only[x] = if lanes[x] { users[x].iter().filter(|&&u| !lanes[u]).all(|&u| inside(u)) } else { users[x].iter().all(|&u| inside(u)) };
            }
            let w: usize = (0..n).filter(|&x| z[x] || only[x]).map(|x| weight(&native.insts[x], lanes[x], mask[x])).sum();
            if w >= WORTH {
                worthy.push((g, w, z, only));
            }
        }
        let mut best = worthy.iter().max_by_key(|c| (c.1, std::cmp::Reverse(c.0)));
        // A gate inside the region that skips nearly as much is sparser: the number check's, say,
        // a token start with a number's first byte, inside a token start without some other
        // byte.
        while let Some(c) = best {
            let inner = worthy.iter().filter(|d| d.0 != c.0 && (c.2[d.0] || c.3[d.0]) && d.1 * 4 >= c.1 * 3).max_by_key(|d| (d.1, std::cmp::Reverse(d.0)));
            match inner {
                Some(d) => best = Some(d),
                None => break,
            }
        }
        let best = best.cloned();
        let Some((g, _, z, only)) = best else { break };
        let r = out.len();
        let members: Vec<usize> = (0..n).filter(|&x| z[x] || only[x]).collect();
        for &x in &members {
            region_of[x] = Some(r);
        }
        gates[g] = true;
        out.push(Region { gate: g, zero: (0..n).filter(|&x| z[x]).collect(), members });
    }
    (out, region_of)
}

/// A value's 16 bytes: known (all zero or all ones), or a vector, complemented or not (a
/// compare gives the complement free; `and` with one is a `bic`).
#[derive(Clone, Copy)]
pub(super) enum Lane {
    Known(bool),
    V(Value, bool),
}

impl Lane {
    fn not(self) -> Lane {
        match self {
            Lane::Known(b) => Lane::Known(!b),
            Lane::V(v, neg) => Lane::V(v, !neg),
        }
    }

    fn and(f: &mut FunctionBuilder<'_>, a: Lane, b: Lane) -> Lane {
        match (a, b) {
            (Lane::Known(false), _) | (_, Lane::Known(false)) => Lane::Known(false),
            (Lane::Known(true), s) | (s, Lane::Known(true)) => s,
            (Lane::V(x, false), Lane::V(y, false)) => Lane::V(f.ins().band(x, y), false),
            (Lane::V(x, false), Lane::V(y, true)) | (Lane::V(y, true), Lane::V(x, false)) => Lane::V(f.ins().band_not(x, y), false),
            (Lane::V(x, true), Lane::V(y, true)) => Lane::V(f.ins().bor(x, y), true),
        }
    }

    fn xor(f: &mut FunctionBuilder<'_>, a: Lane, b: Lane) -> Lane {
        match (a, b) {
            (Lane::Known(x), Lane::Known(y)) => Lane::Known(x != y),
            (Lane::Known(false), s) | (s, Lane::Known(false)) => s,
            (Lane::Known(true), s) | (s, Lane::Known(true)) => s.not(),
            (Lane::V(x, nx), Lane::V(y, ny)) => Lane::V(f.ins().bxor(x, y), nx != ny),
        }
    }
}

/// The byte vectors of one block's body, and each byte-local value's chunks, made on demand.
/// Chunk 0 is the 16 bytes before the block; chunks 1 to 4 are the block's.
#[derive(Clone)]
pub(super) struct Lanes<'a> {
    native: &'a BlockNative,
    plan: &'a Plan,
    /// The ASCII body: values known on ASCII windows are folded.
    ascii: bool,
    /// The block's first: the bytes before it are zeros in place of nothing (`at_zero`).
    first: Option<Value>,
    bytes: [Option<Value>; 5],
    befores: [Option<Value>; 5],
    base: Value,
    nibble: Value,
    zero16: Value,
    nibbles: [Option<(Value, Value)>; 5],
    nibbles_before: [Option<(Value, Value)>; 5],
    lookups: Vec<[Option<Value>; 5]>,
    /// Each table's two halves, and each splatted byte, made once in the body.
    tables: Vec<Option<(Value, Value)>>,
    splats: [Option<Value>; 256],
    lookups_before: Vec<[Option<Value>; 5]>,
    memo: Vec<[Option<Lane>; 5]>,
}

impl<'a> Lanes<'a> {
    /// `bytes`: the block's chunks (0 the 16 bytes before it), when already loaded. `general`:
    /// the body may run on a file's first block, whose bytes before are no file's.
    pub(super) fn new(f: &mut FunctionBuilder<'_>, native: &'a BlockNative, plan: &'a Plan, base: Value, i: Value, general: bool, ascii: bool, bytes: Option<[Option<Value>; 5]>, nibble: Value) -> Self {
        let c = f.ins().iconst(types::I8, 0);
        let zero16 = f.ins().splat(types::I8X16, c);
        let first = general.then(|| {
            // All ones past the first block, zero in it: the bytes before a file read as no bits.
            let past = f.ins().icmp_imm(IntCC::NotEqual, i, 0);
            let past = f.ins().ineg(past);
            f.ins().splat(types::I8X16, past)
        });
        let t = native.tables.len();
        Lanes {
            native,
            plan,
            ascii,
            first,
            bytes: bytes.unwrap_or([None; 5]),
            befores: [None; 5],
            base,
            nibble,
            zero16,
            nibbles: [None; 5],
            nibbles_before: [None; 5],
            lookups: vec![[None; 5]; t],
            tables: vec![None; t],
            splats: [None; 256],
            lookups_before: vec![[None; 5]; t],
            memo: vec![[None; 5]; native.insts.len()],
        }
    }

    /// The block's chunk `c` of bytes (0: the 16 before it), loaded once.
    pub(super) fn bytes(&mut self, f: &mut FunctionBuilder<'_>, c: usize) -> Value {
        if let Some(v) = self.bytes[c] {
            return v;
        }
        let v = f.ins().load(types::I8X16, MemFlags::new().with_notrap(), self.base, 16 * c as i32 - 16);
        self.bytes[c] = Some(v);
        v
    }

    /// Chunk `c`'s bytes one back (each byte's byte before), for `pair` and `plane_before`.
    fn before(&mut self, f: &mut FunctionBuilder<'_>, c: usize) -> Value {
        if let Some(v) = self.befores[c] {
            return v;
        }
        let v = f.ins().load(types::I8X16, MemFlags::new().with_notrap(), self.base, 16 * c as i32 - 17);
        self.befores[c] = Some(v);
        v
    }

    /// Table `t`'s class bytes at chunk `c` (or at the bytes one back), shared by its classes.
    fn lookup(&mut self, f: &mut FunctionBuilder<'_>, t: usize, c: usize, back: bool) -> Value {
        let cached = if back { self.lookups_before[t][c] } else { self.lookups[t][c] };
        if let Some(v) = cached {
            return v;
        }
        let nib = if back { self.nibbles_before[c] } else { self.nibbles[c] };
        let (lo, hi) = match nib {
            Some(x) => x,
            None => {
                let b = if back { self.before(f, c) } else { self.bytes(f, c) };
                let x = (f.ins().band(b, self.nibble), f.ins().ushr_imm(b, 4));
                if back { self.nibbles_before[c] = Some(x) } else { self.nibbles[c] = Some(x) }
                x
            }
        };
        let (tlo, thi) = match self.tables[t] {
            Some(x) => x,
            None => {
                let table = &self.native.tables[t];
                let tlo = f.func.dfg.constants.insert(ConstantData::from(&table.lo[..]));
                let thi = f.func.dfg.constants.insert(ConstantData::from(&table.hi[..]));
                let x = (f.ins().vconst(types::I8X16, tlo), f.ins().vconst(types::I8X16, thi));
                self.tables[t] = Some(x);
                x
            }
        };
        let l = f.ins().swizzle(tlo, lo);
        let h = f.ins().swizzle(thi, hi);
        let v = f.ins().band(l, h);
        if back { self.lookups_before[t][c] = Some(v) } else { self.lookups[t][c] = Some(v) }
        v
    }

    fn splat(&mut self, f: &mut FunctionBuilder<'_>, byte: u8) -> Value {
        if byte == 0 {
            return self.zero16;
        }
        if let Some(v) = self.splats[usize::from(byte)] {
            return v;
        }
        let c = f.ins().iconst(types::I8, i64::from(byte));
        let v = f.ins().splat(types::I8X16, c);
        self.splats[usize::from(byte)] = Some(v);
        v
    }

    /// The bytes with none of `bits` set in `v`: the complement of the test, free.
    fn none_of(&mut self, f: &mut FunctionBuilder<'_>, v: Value, bits: u8) -> Lane {
        let sel = self.splat(f, bits);
        let picked = f.ins().band(v, sel);
        Lane::V(f.ins().icmp(IntCC::Equal, picked, self.zero16), true)
    }

    /// A lane as a plain or complemented vector of the polarity `neg`.
    fn polar(&mut self, f: &mut FunctionBuilder<'_>, x: Lane, neg: bool) -> Value {
        match x {
            Lane::Known(b) => {
                if b != neg {
                    self.splat(f, 0xff)
                } else {
                    self.zero16
                }
            }
            Lane::V(v, n) if n == neg => v,
            Lane::V(v, _) => f.ins().bnot(v),
        }
    }

    /// Instruction `at`'s chunk `c`.
    pub(super) fn get(&mut self, f: &mut FunctionBuilder<'_>, at: usize, c: usize) -> Lane {
        if let Some(l) = self.memo[at][c] {
            return l;
        }
        let lane = match self.plan.ascii[at] {
            Some(b) if self.ascii => Lane::Known(b),
            _ => self.make(f, at, c),
        };
        self.memo[at][c] = Some(lane);
        lane
    }

    fn make(&mut self, f: &mut FunctionBuilder<'_>, at: usize, c: usize) -> Lane {
        let o = |x: u16| usize::from(x);
        match self.native.insts[at] {
            BlockInst::Const(x) => Lane::Known(x != 0),
            BlockInst::EqByte(byte) => {
                let b = self.bytes(f, c);
                let s = self.splat(f, byte);
                Lane::V(f.ins().icmp(IntCC::Equal, b, s), false)
            }
            BlockInst::Class { table, bits } => {
                let v = self.lookup(f, usize::from(table), c, false);
                self.none_of(f, v, bits)
            }
            BlockInst::Plane(k) => {
                let b = self.bytes(f, c);
                self.none_of(f, b, 1 << k)
            }
            BlockInst::PlaneBefore(k) => {
                let b = self.before(f, c);
                self.none_of(f, b, 1 << k)
            }
            BlockInst::Pair { before, table, bits } => {
                let p = self.lookup(f, usize::from(before), c, true);
                let t = self.lookup(f, usize::from(table), c, false);
                let both = f.ins().band(p, t);
                self.none_of(f, both, bits)
            }
            BlockInst::Not(a) => self.get(f, o(a), c).not(),
            BlockInst::And(a, b) => {
                let (x, y) = (self.get(f, o(a), c), self.get(f, o(b), c));
                Lane::and(f, x, y)
            }
            BlockInst::AndNot(a, b) => {
                let (x, y) = (self.get(f, o(a), c), self.get(f, o(b), c));
                Lane::and(f, x, y.not())
            }
            BlockInst::Or(a, b) => {
                let (x, y) = (self.get(f, o(a), c), self.get(f, o(b), c));
                Lane::and(f, x.not(), y.not()).not()
            }
            BlockInst::Xor(a, b) => {
                let (x, y) = (self.get(f, o(a), c), self.get(f, o(b), c));
                Lane::xor(f, x, y)
            }
            BlockInst::Prev(a, k) => {
                assert!(c >= 1, "a prev is not shifted again");
                let mut back = self.get(f, o(a), c - 1);
                let here = self.get(f, o(a), c);
                // Before a file the bytes are zeros: where `a` may hold on 0x00, cut them.
                if c == 1
                    && self.plan.at_zero[o(a)]
                    && let Some(keep) = self.first
                {
                    back = Lane::and(f, back, Lane::V(keep, false));
                }
                if let (Lane::Known(x), Lane::Known(y)) = (back, here)
                    && x == y
                {
                    return Lane::Known(x);
                }
                let neg = match (back, here) {
                    (_, Lane::V(_, n)) | (Lane::V(_, n), _) => n,
                    _ => false,
                };
                let (x, y) = (self.polar(f, back, neg), self.polar(f, here, neg));
                // Bytes `16 - k ..` of the chunk before, then the chunk's first `16 - k`: `ext`.
                let lanes: Vec<u8> = (0..16u8).map(|l| 16 - k + l).collect();
                let mask = f.func.dfg.immediates.push(ConstantData::from(&lanes[..]));
                Lane::V(f.ins().shuffle(x, y, mask), neg)
            }
            _ => unreachable!("not byte-local"),
        }
    }

    /// The block's four chunks of a value as a 64-bit mask, complemented as they are.
    pub(super) fn mask(&mut self, f: &mut FunctionBuilder<'_>, parts: &[Lane], weights: Value) -> Sym {
        if let Some(Lane::Known(b)) = parts.first().copied()
            && parts.iter().all(|p| matches!(p, Lane::Known(x) if *x == b))
        {
            return Sym::Known(if b { u64::MAX } else { 0 });
        }
        let neg = parts.iter().find_map(|p| if let Lane::V(_, n) = p { Some(*n) } else { None }).unwrap_or(false);
        let vs: Vec<Value> = parts.iter().map(|&p| self.polar(f, p, neg)).collect();
        Sym::Value(bitmask(f, &vs, weights), neg)
    }

}

/// Whether the block's bytes, and the 16 before it when a value reads them, are all ASCII: the
/// full body's one branch. Gives the test and the chunks it loaded, for the bodies after it.
pub(super) fn all_ascii(f: &mut FunctionBuilder<'_>, native: &BlockNative, plan: &Plan, base: Value) -> (Value, [Option<Value>; 5]) {
    let back = plan.before || native.insts.iter().any(|i| matches!(i, BlockInst::Pair { .. } | BlockInst::PlaneBefore(_)));
    let mut bytes = [None; 5];
    let mut any: Option<Value> = None;
    for c in (if back { 0 } else { 1 })..5 {
        let b = f.ins().load(types::I8X16, MemFlags::new().with_notrap(), base, 16 * c as i32 - 16);
        bytes[c] = Some(b);
        any = Some(match any {
            None => b,
            Some(x) => f.ins().bor(x, b),
        });
    }
    let any = any.expect("four chunks");
    let words = f.ins().bitcast(types::I64X2, MemFlags::new().with_endianness(cranelift_codegen::ir::Endianness::Little), any);
    let (a, b) = (f.ins().extractlane(words, 0), f.ins().extractlane(words, 1));
    let both = f.ins().bor(a, b);
    let high = f.ins().band_imm(both, 0x8080_8080_8080_8080_u64 as i64);
    (f.ins().icmp_imm(IntCC::Equal, high, 0), bytes)
}
