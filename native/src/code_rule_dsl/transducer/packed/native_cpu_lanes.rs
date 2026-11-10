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

use std::collections::HashMap;

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
    /// How far back a byte-local value reads: its leaves' bytes at most `lag` before its own
    /// (a `prev` adds its shift, a `pair` and a `plane_before` one). On the vectors it is
    /// computed at a lag (`Lanes::get_at`): each leaf from the bytes that far back, loaded so.
    lag: Vec<u8>,
    /// May be set on a byte of 0x00 (the zeros before a file), for a shiftable value.
    at_zero: Vec<bool>,
    /// Its value on every window of ASCII bytes, when it is the same on all of them.
    pub(super) ascii: Vec<Option<bool>>,
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
    /// A test before the gate's: the gate's other operand, when the gate's byte class is made a
    /// mask for the gate alone. A block where it has no bit skips the class's mask too, and one
    /// where it has bits makes the class and tests the gate (the class and the gate are then
    /// members). An escape's `u`, say: most blocks have no escape.
    pub(super) pre: Option<usize>,
    /// The values provably zero with the gate (the gate itself not among them).
    pub(super) zero: Vec<usize>,
    /// Every value the region computes, in order: `zero`, and values only the region reads.
    pub(super) members: Vec<usize>,
    /// The values on the vectors only the members read: made on demand inside the branch, and
    /// never in a block it skips.
    pub(super) vectors: Vec<usize>,
}

/// What one operation costs, roughly, in instructions: for choosing the regions worth a branch.
fn weight(inst: &BlockInst, lane: bool, mask: bool) -> usize {
    match inst {
        // On the vectors: one instruction per 16-byte chunk, four a block (a compare, a lookup's
        // test, an `ext` for a `prev`, a logic op); a `not` and a constant are free (a compare
        // gives the complement, a known lane folds); a `pair`, two lookups and an `and`. Made a
        // mask too: four `and`s, three pairwise adds and a move.
        _ if lane => {
            let own = match inst {
                BlockInst::Const(_) | BlockInst::Not(_) => 0,
                BlockInst::Pair { .. } => 8,
                _ => 4,
            };
            own + if mask { 9 } else { 0 }
        }
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

/// Which values are byte-local, on the vectors, and how far back each reads (`Plan::lag`), and
/// what may be true of each on ASCII bytes. `deep`: a `prev` of any byte-local value (within
/// `MAX_LAG`) and `within` are on the vectors too; else only a `prev` of one with no `prev` in
/// it, simdjson's `prev<N>` of the input's classes.
fn lane_plan(native: &BlockNative, deep: bool) -> (Vec<bool>, Vec<u8>, Vec<bool>, Vec<Can>) {
    let n = native.insts.len();
    let (mut lanes, mut lag, mut at_zero) = (vec![false; n], vec![0u8; n], vec![false; n]);
    let mut can = vec![Can { zero: true, one: true }; n];
    for (at, inst) in native.insts.iter().enumerate() {
        let o = |x: u16| usize::from(x);
        let (l, s, z, c) = match *inst {
            BlockInst::Const(0) => (true, 0, false, Can { zero: true, one: false }),
            BlockInst::Const(u64::MAX) => (true, 0, true, Can { zero: false, one: true }),
            BlockInst::EqByte(b) => (true, 0, b == 0, Can { zero: true, one: b < 0x80 }),
            BlockInst::Class { table, bits } => {
                let t = &native.tables[usize::from(table)];
                let one = (0..0x80u8).any(|b| class_byte(t, b) & bits != 0);
                let zero = (0..0x80u8).any(|b| class_byte(t, b) & bits == 0);
                (true, 0, class_byte(t, 0) & bits != 0, Can { zero, one })
            }
            BlockInst::Plane(k) => (true, 0, false, Can { zero: true, one: k < 7 }),
            BlockInst::PlaneBefore(k) => (true, 1, false, Can { zero: true, one: k < 7 }),
            BlockInst::Pair { before, table, bits } => {
                let (p, t) = (&native.tables[usize::from(before)], &native.tables[usize::from(table)]);
                let ps = (0..0x80u8).fold(0, |a, b| a | class_byte(p, b));
                let ts = (0..0x80u8).fold(0, |a, b| a | class_byte(t, b));
                (true, 1, false, Can { zero: true, one: ps & ts & bits != 0 })
            }
            BlockInst::Not(a) => (lanes[o(a)], lag[o(a)], !at_zero[o(a)], Can { zero: can[o(a)].one, one: can[o(a)].zero }),
            BlockInst::And(a, b) | BlockInst::Or(a, b) | BlockInst::Xor(a, b) | BlockInst::AndNot(a, b) => {
                let (x, y) = (can[o(a)], can[o(b)]);
                let (zx, zy) = (at_zero[o(a)], at_zero[o(b)]);
                let (z, c) = match *inst {
                    BlockInst::And(..) => (zx && zy, Can { zero: x.zero || y.zero, one: x.one && y.one }),
                    BlockInst::Or(..) => (zx || zy, Can { zero: x.zero && y.zero, one: x.one || y.one }),
                    BlockInst::Xor(..) => (zx != zy, Can { zero: (x.zero && y.zero) || (x.one && y.one), one: (x.one && y.zero) || (x.zero && y.one) }),
                    _ => (zx && !zy, Can { zero: x.zero || y.one, one: x.one && y.zero }),
                };
                (lanes[o(a)] && lanes[o(b)], lag[o(a)].max(lag[o(b)]), z, c)
            }
            // A `prev`: its operand's bytes `k` further back (an `ext` a chunk). Before a file it
            // reads no bits.
            BlockInst::Prev(a, k) => {
                let l = u16::from(lag[o(a)]) + u16::from(k);
                (lanes[o(a)] && (deep || lag[o(a)] == 0) && l <= u16::from(MAX_LAG), l.min(255) as u8, false, can[o(a)])
            }
            // The bytes inside the stream: two compares of the positions, all ones in a full
            // block (past a delay of 48 not in the 16 bytes before one).
            BlockInst::Within(d) => (deep && d <= 48, 0, false, Can { zero: true, one: true }),
            _ => (false, 0, false, Can { zero: true, one: true }),
        };
        (lanes[at], lag[at], at_zero[at], can[at]) = (l, s, z, c);
    }
    (lanes, lag, at_zero, can)
}

/// A byte-local value is on the vectors only where that saves a mask or a carried field: a
/// `prev` or logic none of whose readers is on the vectors, and whose operands are masks anyway,
/// is cheaper as a mask's operation; a `within` no value on the vectors reads, too.
fn demote(native: &BlockNative, lanes: &mut [bool], users: &[Vec<usize>], output: &[bool]) {
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
                // operand is made a mask anyway, for a reader other than `prev`s, is the mask
                // shifted instead.
                BlockInst::Prev(a, _) => {
                    let a = usize::from(a);
                    output[a] || users[a].iter().any(|&u| u != at && !lanes[u] && !matches!(native.insts[u], BlockInst::Prev(..)))
                }
                BlockInst::Not(_) | BlockInst::And(..) | BlockInst::Or(..) | BlockInst::Xor(..) | BlockInst::AndNot(..) => {
                    inst.operands().into_iter().all(|x| needs_mask(lanes, x) || matches!(native.insts[x], BlockInst::Const(_)))
                }
                BlockInst::Within(_) => true,
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
}

impl Plan {
    /// The plan: which values are on the vectors, and the gated regions. On the vectors a
    /// `prev` reads the bytes before the block, so it carries nothing between blocks, and a
    /// region can skip it; as a mask it carries the block's last bits, and a region cannot. But
    /// over a `prev`, logic on the vectors costs four times what it does as masks. So the vectors
    /// go deep (`lane_plan`) only where a gate skips them: the regions are found with them deep,
    /// the values those regions skip keep them, every other value takes the shallow plan, and the
    /// regions are found again.
    pub(super) fn of(native: &BlockNative) -> Plan {
        let n = native.insts.len();
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
        let (mut deep, lag, at_zero, can) = lane_plan(native, true);
        demote(native, &mut deep, &users, &output);
        let (mut lanes, _, _, _) = lane_plan(native, false);
        demote(native, &mut lanes, &users, &output);
        let gated = {
            let (_, regions) = Self::regions_of(native, &deep, &users, &output);
            let mut gated = vec![false; n];
            for r in &regions {
                for &x in r.members.iter().chain(&r.vectors) {
                    gated[x] = true;
                }
            }
            gated
        };
        // Deep where gated, and so are the operands of what is: a value on the vectors reads
        // them there (made on demand, a mask once for a reader outside).
        let mut need = vec![false; n];
        for x in (0..n).rev() {
            if deep[x] && (gated[x] || need[x]) {
                need[x] = true;
                for o in native.insts[x].operands() {
                    need[o] = need[o] || deep[o];
                }
            }
        }
        for x in 0..n {
            lanes[x] = lanes[x] || need[x];
        }
        demote(native, &mut lanes, &users, &output);
        let (mask, regions) = Self::regions_of(native, &lanes, &users, &output);
        let mut region_of = vec![None; n];
        for (r, reg) in regions.iter().enumerate() {
            for &m in &reg.members {
                region_of[m] = Some(r);
            }
        }
        let ascii: Vec<Option<bool>> = can.iter().map(|c| if !c.one { Some(false) } else if !c.zero { Some(true) } else { None }).collect();
        let before = (0..n).any(|at| lanes[at] && lag[at] > 0);
        // Worth a branch: a byte test or class known on ASCII bytes (not a constant).
        let ascii_branch = native.insts.iter().enumerate().any(|(at, i)| lanes[at] && ascii[at].is_some() && matches!(i, BlockInst::EqByte(_) | BlockInst::Class { .. } | BlockInst::Pair { .. } | BlockInst::Plane(_) | BlockInst::PlaneBefore(_)));
        let full = full_words(native, &lanes);
        Plan { lanes, mask, lag, at_zero, ascii, before, ascii_branch, full, regions, region_of }
    }

    /// The values made masks and the gated regions, for a choice of values on the vectors.
    fn regions_of(native: &BlockNative, lanes: &[bool], users: &[Vec<usize>], output: &[bool]) -> (Vec<bool>, Vec<Region>) {
        let n = native.insts.len();
        let mask: Vec<bool> = (0..n).map(|x| lanes[x] && (output[x] || users[x].iter().any(|&u| !lanes[u]))).collect();
        let full = full_words(native, lanes);
        // The values with a carried field: a carry of their own, or a `prev` of them on masks.
        let field: Vec<bool> = (0..n)
            .map(|x| matches!(native.insts[x], BlockInst::Toggle(_) | BlockInst::Fill(..) | BlockInst::Escaped(_)) || users[x].iter().any(|&u| matches!(native.insts[u], BlockInst::Prev(..)) && !lanes[u]))
            .collect();
        let (regions, _) = regions(native, lanes, &mask, users, output, &field, &full);
        (mask, regions)
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
            BlockInst::Within(d) => (all(u64::MAX), all(if d == 0 { u64::MAX } else { !((1u64 << d) - 1) })),
            _ if lanes[at] => (NONE, NONE),
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
    const MOST: usize = 8;
    // The values a region gives the body after it, each held from the region's end to its
    // reader: simdjson's skipped work gives one error bit. More, and the region costs registers
    // the work it skips does not.
    const MOST_OUT: usize = 4;
    let n = native.insts.len();
    // A fixed point's members are made in its loop, never behind a gate.
    let mut grouped = vec![false; n];
    for g in &native.groups {
        for &m in &g.members {
            grouped[m] = true;
        }
    }
    // A gate's conditions: how many tests its bits need at once, and whether one is carried
    // state (an escape, a fill, a value of a pass before). A byte class, or a run of them, is in
    // most blocks of text; a class with an escape before it, or inside a string, is rare.
    // Without measured densities a gate needs two conditions, one of them carried: simdjson's
    // escapes, a `u` after an escaping backslash.
    let mut conditions = vec![0usize; n];
    let mut carried = vec![false; n];
    for (x, inst) in native.insts.iter().enumerate() {
        let c = |o: u16| conditions[usize::from(o)];
        conditions[x] = match *inst {
            BlockInst::Const(_) | BlockInst::Within(_) => 0,
            BlockInst::And(a, b) => c(a) + c(b),
            BlockInst::AndNot(a, _) => c(a) + 1,
            BlockInst::Prev(a, _) => c(a),
            _ => 1,
        };
        carried[x] = match *inst {
            BlockInst::And(a, b) => carried[usize::from(a)] || carried[usize::from(b)],
            BlockInst::AndNot(a, _) | BlockInst::Prev(a, _) => carried[usize::from(a)],
            _ => !lanes[x] && inst.operands().is_empty() && !matches!(inst, BlockInst::Const(_) | BlockInst::Within(_)) || matches!(inst, BlockInst::Escaped(_) | BlockInst::Toggle(_) | BlockInst::Fill(..) | BlockInst::Carry(_)) || grouped[x],
        };
    }
    let mut region_of: Vec<Option<usize>> = vec![None; n];
    let mut gates = vec![false; n];
    let mut out: Vec<Region> = Vec::new();
    let zeros = |g: usize, region_of: &[Option<usize>]| -> Vec<bool> {
        let mut z = vec![false; n];
        z[g] = true;
        for (at, inst) in native.insts.iter().enumerate().skip(g + 1) {
            if lanes[at] || region_of[at].is_some() || full[at].is_some() || grouped[at] {
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
        let mut worthy: Vec<(usize, usize, Vec<bool>, Vec<bool>, Vec<bool>)> = Vec::new();
        for (g, inst) in native.insts.iter().enumerate() {
            let candidate = !lanes[g]
                && !grouped[g]
                && region_of[g].is_none()
                && !gates[g]
                // Any mask of two conditions or more, not the same in every full block: what it
                // skips decides. A gate set in most blocks costs its test and a branch predicted
                // taken, against at least `WORTH` of work skipped where it is not.
                && full[g].is_none()
                && conditions[g] >= 2
                && carried[g]
                && !users[g].is_empty()
                && !matches!(inst, BlockInst::Const(_) | BlockInst::Not(_));
            if !candidate {
                continue;
            }
            let z = zeros(g, &region_of);
            // The values only the region reads, with no carried field of their own: made inside it.
            let mut only = vec![false; n];
            for x in (0..n).rev() {
                if z[x] || x == g || gates[x] || region_of[x].is_some() || output[x] || field[x] || users[x].is_empty() || (lanes[x] && !mask[x]) || grouped[x] {
                    continue;
                }
                let inside = |u: usize| z[u] || only[u];
                only[x] = if lanes[x] { users[x].iter().filter(|&&u| !lanes[u]).all(|&u| inside(u)) } else { users[x].iter().all(|&u| inside(u)) };
            }
            // Convex: a value outside that reads the region and is read by it would have to be
            // made both before and after its branch. The region's values after one go. A value on
            // the vectors reading another reads its bytes, not its mask: no order between them.
            let mut member: Vec<bool> = (0..n).map(|x| z[x] || only[x]).collect();
            let word = |o: usize, x: usize| !(lanes[o] && lanes[x]);
            loop {
                let mut reach = vec![false; n];
                let mut cut = false;
                for x in 0..n {
                    let from = native.insts[x].operands().iter().any(|&o| word(o, x) && member[o] || reach[o]);
                    if !member[x] {
                        reach[x] = from;
                    } else if native.insts[x].operands().iter().any(|&o| reach[o]) {
                        member[x] = false;
                        reach[x] = true;
                        cut = true;
                    }
                }
                if !cut {
                    break;
                }
            }
            let (z, only): (Vec<bool>, Vec<bool>) = ((0..n).map(|x| z[x] && member[x]).collect(), (0..n).map(|x| only[x] && member[x]).collect());
            let vectors = exclusive_vectors(lanes, users, output, &member);
            // Read after it: by a value neither a member nor one of its vectors, or an output.
            let out = (0..n).filter(|&x| member[x] && (output[x] || users[x].iter().any(|&u| !member[u] && !vectors[u]))).count();
            if out > MOST_OUT {
                continue;
            }
            let w: usize = (0..n).filter(|&x| member[x] || vectors[x]).map(|x| weight(&native.insts[x], lanes[x], mask[x] && member[x])).sum();
            if w >= WORTH {
                worthy.push((g, w, z, only, vectors));
            }
        }
        let mut best = worthy.iter().max_by_key(|c| (c.1, std::cmp::Reverse(c.0)));
        // A gate inside the region that skips nearly as much is sparser: the number check's, say,
        // a token start with a number's first byte, inside a token start without some other
        // byte.
        while let Some(c) = best {
            // Strictly less, so the walk ends: two gates can each be in the other's region.
            let inner = worthy.iter().filter(|d| d.0 != c.0 && (c.2[d.0] || c.3[d.0]) && d.1 < c.1 && d.1 * 4 >= c.1 * 3).max_by_key(|d| (d.1, std::cmp::Reverse(d.0)));
            match inner {
                Some(d) => best = Some(d),
                None => break,
            }
        }
        let best = best.cloned();
        let Some((g, _, z, only, vectors)) = best else { break };
        let r = out.len();
        // The gate's class, made a mask only for the gate, and carried by no field.
        let pre = match native.insts[g] {
            BlockInst::And(a, b) | BlockInst::AndNot(a, b) => {
                let (a, b) = (usize::from(a), usize::from(b));
                let (word, class) = if lanes[a] { (b, a) } else { (a, b) };
                let alone = lanes[class] && !lanes[word] && !output[class] && !field[class] && region_of[class].is_none() && users[class].iter().all(|&u| lanes[u] || u == g);
                alone.then_some((word, class))
            }
            _ => None,
        };
        let mut members: Vec<usize> = (0..n).filter(|&x| z[x] || only[x]).collect();
        if let Some((_, class)) = pre {
            members.extend([class, g]);
            members.sort_unstable();
        }
        let pre = pre.map(|(word, _)| word);
        for &x in &members {
            region_of[x] = Some(r);
        }
        gates[g] = true;
        let vectors: Vec<usize> = (0..n).filter(|&x| vectors[x] && !members.contains(&x)).collect();
        out.push(Region { gate: g, pre, zero: (0..n).filter(|&x| z[x]).collect(), members, vectors });
    }
    (out, region_of)
}

/// The pure values (`BlockNative::pure`) remade in each pass that reads them, not stored by the
/// first and loaded: simdjson's "recompute, do not keep", where it pays.
/// - Constants and `within`: free in a full block, never worth a slot.
/// - A value inside a gated region of a later pass, when every pure value is remade: made there,
///   behind the gate, it costs nothing in a block the gate skips. A region is worth its branch
///   only with the work it skips inside it, so the regions are found with all of it remade.
///
/// Anything else stays stored once and loaded at its reader: remade, a byte check would be held
/// across the later pass's body, a register cost a load does not have. A remade value's
/// operands are remade too.
pub(super) fn remade(native: &BlockNative) -> Vec<bool> {
    let pure = native.pure();
    let n = native.insts.len();
    let mut remade: Vec<bool> = (0..n).map(|x| pure[x] && matches!(native.insts[x], BlockInst::Const(_) | BlockInst::Within(_))).collect();
    let trial = native.passes_with(&pure);
    for (k, pass) in trial.passes.iter().enumerate().skip(1) {
        let super::native_ir::Pass::Forward { body, .. } = pass else { continue };
        let plan = Plan::of(body);
        for r in &plan.regions {
            for &m in r.members.iter().chain(&r.vectors) {
                let o = trial.origins[k][m] as usize;
                if pure[o] {
                    remade[o] = true;
                }
            }
        }
    }
    // A remade value's operands are remade with it: one loaded would be a mask, and the value
    // would leave the vectors, and its region. Made once in the pass, other readers share it.
    for x in (0..n).rev() {
        if remade[x] {
            for o in native.insts[x].operands() {
                remade[o] = remade[o] || pure[o];
            }
        }
    }
    remade
}

/// The values on the vectors that only `member`s read, through other such values: a block that
/// skips the members makes none of them. A value a mask outside reads, or an output, is not.
fn exclusive_vectors(lanes: &[bool], users: &[Vec<usize>], output: &[bool], member: &[bool]) -> Vec<bool> {
    let n = lanes.len();
    let mut inside = vec![false; n];
    for x in (0..n).rev() {
        if !lanes[x] || member[x] || output[x] || users[x].is_empty() {
            continue;
        }
        inside[x] = users[x].iter().all(|&u| member[u] || inside[u]);
    }
    inside
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
/// Chunks 1 to 4 are the block's. A value is made at a lag (`get_at`): its chunk `c` at lag `L`
/// is its bytes `L` before chunk `c`'s, each leaf from the bytes loaded that far back. So
/// `prev(a, k)` at lag `L` is `a` at lag `L + k`, simdjson's `prev<N>` on the input, for any
/// depth of `prev`s within `MAX_LAG`.
#[derive(Clone)]
pub(super) struct Lanes<'a> {
    native: &'a BlockNative,
    plan: &'a Plan,
    /// The ASCII body: values known on ASCII windows are folded.
    ascii: bool,
    /// The block's index and the stream's length outside a full block's body (`None` there):
    /// for `within`, and for a `prev`'s cut before the stream.
    position: Option<(Value, Value)>,
    /// Each (chunk, lag)'s bytes, loaded once.
    bytes: HashMap<(usize, u8), Value>,
    base: Value,
    nibble: Value,
    zero16: Value,
    nibbles: HashMap<(usize, u8), (Value, Value)>,
    /// Each table's lookup at (chunk, lag), shared by its classes.
    lookups: HashMap<(usize, usize, u8), Value>,
    /// Each table's two halves, and each splatted byte, made once in the body.
    tables: Vec<Option<(Value, Value)>>,
    splats: [Option<Value>; 256],
    index: Option<Value>,
    memo: HashMap<(usize, usize, u8), Lane>,
}

impl<'a> Lanes<'a> {
    /// `bytes`: the block's chunks at lag 0 (index 0: the 16 bytes before it), when already
    /// loaded. `general`: the body may run on a file's first or last block.
    pub(super) fn new(f: &mut FunctionBuilder<'_>, native: &'a BlockNative, plan: &'a Plan, base: Value, i: Value, len: Value, general: bool, ascii: bool, bytes: Option<[Option<Value>; 5]>, nibble: Value) -> Self {
        let c = f.ins().iconst(types::I8, 0);
        let zero16 = f.ins().splat(types::I8X16, c);
        let mut loaded = HashMap::new();
        for (k, b) in bytes.unwrap_or([None; 5]).into_iter().enumerate() {
            if let Some(b) = b {
                loaded.insert(if k == 0 { (1, 16) } else { (k, 0) }, b);
            }
        }
        Lanes {
            native,
            plan,
            ascii,
            position: general.then_some((i, len)),
            bytes: loaded,
            base,
            nibble,
            zero16,
            nibbles: HashMap::new(),
            lookups: HashMap::new(),
            tables: vec![None; native.tables.len()],
            splats: [None; 256],
            index: None,
            memo: HashMap::new(),
        }
    }

    /// The bytes `lag` before chunk `c`'s, loaded once (an unaligned load: the window holds
    /// `MAX_LAG` bytes before a block, `FRONT` zeros before a file).
    fn bytes(&mut self, f: &mut FunctionBuilder<'_>, c: usize, lag: u8) -> Value {
        if let Some(&v) = self.bytes.get(&(c, lag)) {
            return v;
        }
        let v = f.ins().load(types::I8X16, MemFlags::new().with_notrap(), self.base, 16 * c as i32 - 16 - i32::from(lag));
        self.bytes.insert((c, lag), v);
        v
    }

    /// Table `t`'s class bytes at chunk `c`, lag `lag`, shared by its classes.
    fn lookup(&mut self, f: &mut FunctionBuilder<'_>, t: usize, c: usize, lag: u8) -> Value {
        if let Some(&v) = self.lookups.get(&(t, c, lag)) {
            return v;
        }
        let (lo, hi) = match self.nibbles.get(&(c, lag)) {
            Some(&x) => x,
            None => {
                let b = self.bytes(f, c, lag);
                let x = (f.ins().band(b, self.nibble), f.ins().ushr_imm(b, 4));
                self.nibbles.insert((c, lag), x);
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
        self.lookups.insert((t, c, lag), v);
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
        self.get_at(f, at, c, 0)
    }

    /// Instruction `at`'s chunk `c`, `lag` bytes back.
    fn get_at(&mut self, f: &mut FunctionBuilder<'_>, at: usize, c: usize, lag: u8) -> Lane {
        if let Some(&l) = self.memo.get(&(at, c, lag)) {
            return l;
        }
        let lane = match self.plan.ascii[at] {
            Some(b) if self.ascii => Lane::Known(b),
            _ => self.make(f, at, c, lag),
        };
        self.memo.insert((at, c, lag), lane);
        lane
    }

    fn make(&mut self, f: &mut FunctionBuilder<'_>, at: usize, c: usize, lag: u8) -> Lane {
        let o = |x: u16| usize::from(x);
        match self.native.insts[at] {
            BlockInst::Const(x) => Lane::Known(x != 0),
            BlockInst::Within(d) => self.within(f, d, c, lag),
            BlockInst::EqByte(byte) => {
                let b = self.bytes(f, c, lag);
                let s = self.splat(f, byte);
                Lane::V(f.ins().icmp(IntCC::Equal, b, s), false)
            }
            BlockInst::Class { table, bits } => {
                let v = self.lookup(f, usize::from(table), c, lag);
                self.none_of(f, v, bits)
            }
            BlockInst::Plane(k) => {
                let b = self.bytes(f, c, lag);
                self.none_of(f, b, 1 << k)
            }
            BlockInst::PlaneBefore(k) => {
                let b = self.bytes(f, c, lag + 1);
                self.none_of(f, b, 1 << k)
            }
            BlockInst::Pair { before, table, bits } => {
                let p = self.lookup(f, usize::from(before), c, lag + 1);
                let t = self.lookup(f, usize::from(table), c, lag);
                let both = f.ins().band(p, t);
                self.none_of(f, both, bits)
            }
            BlockInst::Not(a) => self.get_at(f, o(a), c, lag).not(),
            BlockInst::And(a, b) => {
                let (x, y) = (self.get_at(f, o(a), c, lag), self.get_at(f, o(b), c, lag));
                Lane::and(f, x, y)
            }
            BlockInst::AndNot(a, b) => {
                let (x, y) = (self.get_at(f, o(a), c, lag), self.get_at(f, o(b), c, lag));
                Lane::and(f, x, y.not())
            }
            BlockInst::Or(a, b) => {
                let (x, y) = (self.get_at(f, o(a), c, lag), self.get_at(f, o(b), c, lag));
                Lane::and(f, x.not(), y.not()).not()
            }
            BlockInst::Xor(a, b) => {
                let (x, y) = (self.get_at(f, o(a), c, lag), self.get_at(f, o(b), c, lag));
                Lane::xor(f, x, y)
            }
            BlockInst::Prev(a, k) => {
                if c >= 2 {
                    // simdjson's `prev<N>`: the chunk before's last `k` bytes, then this chunk's
                    // first `16 - k`, one `ext`. Its positions are all at least `k`.
                    let (back, here) = (self.get_at(f, o(a), c - 1, lag), self.get_at(f, o(a), c, lag));
                    return self.ext(f, back, here, k);
                }
                // The first chunk: the chunk before it at this lag would need bytes before the
                // window, so `a` is made `k` bytes further back instead.
                let v = self.get_at(f, o(a), c, lag + k);
                // Before the stream a `prev` reads no bits: where `a` may hold on the zeros
                // before a file, cut this value's positions less than `k` (a first block).
                if self.plan.at_zero[o(a)]
                    && let Some(cut) = self.from(f, c, lag, k)
                {
                    return Lane::and(f, v, cut);
                }
                v
            }
            _ => unreachable!("not byte-local"),
        }
    }

    /// Bytes `16 - k ..` of `back`, then `here`'s first `16 - k`: one `ext`.
    fn ext(&mut self, f: &mut FunctionBuilder<'_>, back: Lane, here: Lane, k: u8) -> Lane {
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
        let lanes: Vec<u8> = (0..16u8).map(|l| 16 - k + l).collect();
        let mask = f.func.dfg.immediates.push(ConstantData::from(&lanes[..]));
        Lane::V(f.ins().shuffle(x, y, mask), neg)
    }

    /// The lanes' indexes, 0 to 15.
    fn index(&mut self, f: &mut FunctionBuilder<'_>) -> Value {
        if let Some(v) = self.index {
            return v;
        }
        let index: Vec<u8> = (0..16).collect();
        let index = f.func.dfg.constants.insert(ConstantData::from(&index[..]));
        let v = f.ins().vconst(types::I8X16, index);
        self.index = Some(v);
        v
    }

    /// Chunk `c`'s first stream position, and `x` less it clamped to `0..=16` as a splat: a lane
    /// index bound.
    fn bound(f: &mut FunctionBuilder<'_>, i: Value, c: usize, lag: u8, x: Value) -> Value {
        let start = f.ins().ishl_imm(i, 6);
        let b = f.ins().iadd_imm(start, 16 * c as i64 - 16 - i64::from(lag));
        let x = f.ins().isub(x, b);
        let (zero, sixteen) = (f.ins().iconst(types::I64, 0), f.ins().iconst(types::I64, 16));
        let x = f.ins().smax(x, zero);
        let x = f.ins().smin(x, sixteen);
        let x = f.ins().ireduce(types::I8, x);
        f.ins().splat(types::I8X16, x)
    }

    /// The lanes of chunk `c`, `lag` back, at stream positions `k` or more: `None` in a full
    /// block's body, where all are (its positions are at least `64 - MAX_LAG`).
    fn from(&mut self, f: &mut FunctionBuilder<'_>, c: usize, lag: u8, k: u8) -> Option<Lane> {
        let (i, _) = self.position?;
        let at = f.ins().iconst(types::I64, i64::from(k));
        let lo = Self::bound(f, i, c, lag, at);
        let index = self.index(f);
        Some(Lane::V(f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, index, lo), false))
    }

    /// `within(d)`'s chunk `c` at lag `lag`: the lanes at stream positions `p` (less the lag)
    /// with `d <= p < len + d`. In a full block's body all are: from the second block on, `p` is
    /// at least 48 for any lag up to 16, and `d <= 48` (`lane_plan`). Elsewhere `lo <= j < hi`,
    /// the range's ends less the chunk's first position, clamped: two compares with the indexes.
    fn within(&mut self, f: &mut FunctionBuilder<'_>, d: u8, c: usize, lag: u8) -> Lane {
        let Some((i, len)) = self.position else {
            return Lane::Known(true);
        };
        let d64 = f.ins().iconst(types::I64, i64::from(d));
        let lo = Self::bound(f, i, c, lag, d64);
        let end = f.ins().iadd_imm(len, i64::from(d));
        let hi = Self::bound(f, i, c, lag, end);
        let index = self.index(f);
        let from = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, index, lo);
        let to = f.ins().icmp(IntCC::UnsignedLessThan, index, hi);
        Lane::V(f.ins().band(from, to), false)
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
