//! A kernel's register pressure at each width, and the width it runs at (decision 0567, step 3): the SSA's values
//! live at each point, each in the registers its shape takes at a width (a lanes value in vector registers, a part
//! of four 32-bit or two 64-bit lanes each; a mask in a part of four lanes each; a uniform value, and at one lane a
//! lanes value, in an integer or float register), the most at any point the function's pressure in each file.
//!
//! The width is the one whose cost a lane is least, in cycles of a core that issues six integer operations, four
//! vector ones and three loads or stores a cycle (an Apple M1's, near enough an x86-64 core's): a lanes operation is
//! a vector one of up to four lanes (fewer at a width below four), at one lane an integer one; one a lane at a time
//! (a load or store at each lane's address, a copy, a shuffle) a load a lane, and at a width of two or more its lane
//! moved out of its vector and back too (two cycles, measured: `width_times`); the uniform operations and the
//! control integer ones for the width's lanes; each register past the target's a spill and a reload for the
//! width's lanes; and below 32 lanes, a cross-lane kernel's exchanges between its chunks (`cl`'s chunked code): two
//! fiber switches and their stores and loads for the chunk's lanes.

use crate::k::M;
use crate::ssa::{Func, O, Shape, Ssa, V};

/// The widths a kernel may run at.
pub const WIDTHS: [u32; 6] = [1, 2, 4, 8, 16, 32];

/// The registers of each file the code may hold values in (the target's, less those its code takes for itself:
/// temporaries, the stack and frame pointers, the SIMD group's and the frame's).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Files {
    pub vec: u32,
    pub int: u32,
    pub float: u32,
}

impl Files {
    /// The host's.
    pub fn host() -> Files {
        if cfg!(target_arch = "x86_64") {
            // 16 xmm (the floats among them), 16 general registers less rsp, rbp and two temporaries.
            Files { vec: 14, int: 12, float: 0 }
        } else {
            // 32 vector registers (the floats among them), 31 general registers less x16-x18, x29, x30 and two.
            Files { vec: 28, int: 24, float: 0 }
        }
    }
}

/// The most registers live at once in each file, at a width.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pressure {
    pub vec: u32,
    pub int: u32,
    pub float: u32,
}

/// A function's pressure at each width (`WIDTHS`'), and its operations' counts.
#[derive(Clone, Debug, Default)]
pub struct FuncPressure {
    pub at: [Pressure; 6],
    /// Its operations on lanes as vectors, on lanes a lane at a time, and the others (uniform ones and control).
    pub lanes: u32,
    pub scalar: u32,
    pub other: u32,
    /// Its exchanges with the SIMD group's other chunks below 32 lanes (cross-lane operations, the group's decisions).
    pub exchanges: u32,
    /// Its calls: the function called, and at each width the vector registers its values live across the call
    /// take (what the caller keeps, its result apart).
    pub across: Vec<(u32, [u32; 6])>,
}

/// The registers value `x` of `f` takes at width `w`: vector, integer, float.
fn cost(f: &Func, x: V, w: u32) -> (u32, u32, u32) {
    let i = f.inst(x);
    let scalar = |m: M| if m.float() { (0, 0, 1) } else { (0, 1, 0) };
    match i.shape {
        Shape::None => (0, 0, 0),
        Shape::Uniform => scalar(i.m),
        Shape::Mask if w == 1 => (0, 1, 0),
        Shape::Mask => (w.div_ceil(4), 0, 0),
        Shape::Lanes if w == 1 => scalar(i.m),
        Shape::Lanes => {
            let per = if i.m.bytes() == 8 && !i.m.is_agg() { 2 } else { 4 };
            (w.div_ceil(per), 0, 0)
        }
    }
}

/// A small bit set.
#[derive(Clone, PartialEq, Eq)]
struct Bits(Vec<u64>);

impl Bits {
    fn new(n: usize) -> Bits {
        Bits(vec![0; n.div_ceil(64)])
    }
    fn set(&mut self, x: V) {
        self.0[x as usize / 64] |= 1 << (x % 64);
    }
    fn clear(&mut self, x: V) {
        self.0[x as usize / 64] &= !(1 << (x % 64));
    }
    fn has(&self, x: V) -> bool {
        self.0[x as usize / 64] >> (x % 64) & 1 != 0
    }
    fn or(&mut self, o: &Bits) -> bool {
        let mut changed = false;
        for (a, &b) in self.0.iter_mut().zip(&o.0) {
            let n = *a | b;
            changed |= n != *a;
            *a = n;
        }
        changed
    }
    fn iter(&self) -> impl Iterator<Item = V> + '_ {
        self.0.iter().enumerate().flat_map(|(k, &w)| (0..64).filter(move |b| w >> b & 1 != 0).map(move |b| (k * 64 + b) as V))
    }
}

/// The values instruction `x` of `f` reads (a jump's and a call's arguments among them).
fn reads(f: &Func, x: V) -> Vec<V> {
    let i = f.inst(x);
    let mut u: Vec<V> = i.uses().collect();
    if matches!(i.op, O::Jump | O::Call) {
        u.extend(f.args_of(i).iter().copied());
    }
    u
}

/// Function `f`'s pressure at each width.
pub fn of_func(ssa: &Ssa, f: &Func) -> FuncPressure {
    let n = f.insts.len();
    let nb = f.blocks.len();
    // Each block's values read before it defines them (`gen`) and those it defines (its parameters among them).
    let mut gen_ = vec![Bits::new(n); nb];
    let mut def = vec![Bits::new(n); nb];
    let mut fp = FuncPressure::default();
    for (b, blk) in f.blocks.iter().enumerate() {
        for &p in &blk.params {
            def[b].set(p);
        }
        for &x in &blk.insts {
            for u in reads(f, x) {
                if !def[b].has(u) {
                    gen_[b].set(u);
                }
            }
            def[b].set(x);
            let i = f.inst(x);
            let lanes = |u: u32| f.inst(u).shape == Shape::Lanes;
            let each = match i.op {
                O::Load => lanes(i.a) && f.inst(i.a).op != O::FrameAddr,
                O::Store => lanes(i.a) && f.inst(i.a).op != O::FrameAddr,
                O::Atomic => i.shape == Shape::Lanes,
                O::Copy | O::Zero | O::Shuffle => true,
                _ => false,
            };
            if each {
                fp.scalar += 1;
            } else {
                match i.shape {
                    Shape::Lanes | Shape::Mask => fp.lanes += 1,
                    _ => fp.other += 1,
                }
            }
        }
    }
    if f.cross {
        let dec = crate::ssa::group_decisions(ssa, f);
        let ops = f.blocks.iter().flat_map(|b| &b.insts).filter(|&&x| {
            let i = f.inst(x);
            i.op.cross_lane() || (i.op == O::Atomic && i.shape == Shape::Uniform)
        });
        fp.exchanges = (ops.count() + dec.iter().filter(|&&d| d).count()) as u32;
    }
    let succs: Vec<Vec<u32>> = (0..nb).map(|b| f.succs(b)).collect();
    let mut live_in = gen_.clone();
    let mut live_out = vec![Bits::new(n); nb];
    loop {
        let mut changed = false;
        for b in (0..nb).rev() {
            for &s in &succs[b] {
                let s = &live_in[s as usize];
                // SAFETY of the split borrow: `live_out` and `live_in` are apart.
                changed |= live_out[b].or(s);
            }
            let mut li = live_out[b].clone();
            for (a, &d) in li.0.iter_mut().zip(&def[b].0) {
                *a &= !d;
            }
            li.or(&gen_[b]);
            if li != live_in[b] {
                live_in[b] = li;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    // Each block backwards from what it leaves live: at each instruction, what it defines and what is live after it,
    // then what it reads in place of what it defines.
    for (b, blk) in f.blocks.iter().enumerate() {
        let mut live = live_out[b].clone();
        let mut sum = [(0u32, 0u32, 0u32); 6];
        for x in live.iter() {
            for (k, &w) in WIDTHS.iter().enumerate() {
                let c = cost(f, x, w);
                sum[k] = (sum[k].0 + c.0, sum[k].1 + c.1, sum[k].2 + c.2);
            }
        }
        let note = |sum: &[(u32, u32, u32); 6], fp: &mut FuncPressure| {
            for k in 0..6 {
                let p = &mut fp.at[k];
                (p.vec, p.int, p.float) = (p.vec.max(sum[k].0), p.int.max(sum[k].1), p.float.max(sum[k].2));
            }
        };
        let add = |sum: &mut [(u32, u32, u32); 6], x: V, sign: i32| {
            for (k, &w) in WIDTHS.iter().enumerate() {
                let c = cost(f, x, w);
                let s = &mut sum[k];
                if sign > 0 {
                    *s = (s.0 + c.0, s.1 + c.1, s.2 + c.2);
                } else {
                    *s = (s.0 - c.0, s.1 - c.1, s.2 - c.2);
                }
            }
        };
        note(&sum, &mut fp);
        for &x in blk.insts.iter().rev() {
            if !live.has(x) {
                // Defined, not read after: live at its definition only.
                live.set(x);
                add(&mut sum, x, 1);
            }
            note(&sum, &mut fp);
            live.clear(x);
            add(&mut sum, x, -1);
            if f.inst(x).op == O::Call {
                fp.across.push((f.inst(x).a, std::array::from_fn(|k| sum[k].0 + sum[k].2)));
            }
            for u in reads(f, x) {
                if !live.has(u) {
                    live.set(u);
                    add(&mut sum, u, 1);
                }
            }
            note(&sum, &mut fp);
        }
    }
    fp
}

/// A kernel's pressure: each function's.
pub fn of(ssa: &Ssa) -> Vec<FuncPressure> {
    ssa.funcs.iter().map(|f| of_func(ssa, f)).collect()
}

/// The cost a lane of the kernel's functions at width `WIDTHS[k]`, the target's `files` (in operations, per lane).
pub fn lane_cost(ps: &[FuncPressure], k: usize, files: Files) -> f64 {
    let w = WIDTHS[k];
    let past = |p: u32, have: u32| f64::from(p.saturating_sub(have));
    ps.iter()
        .map(|p| {
            let at = p.at[k];
            // Floats share the vector registers on both targets.
            let spilled = if w == 1 { past(at.int, files.int) + past(at.float, files.vec) } else { past(at.vec + at.float, files.vec) + past(at.int, files.int) };
            let w = f64::from(w);
            let lanes = if w == 1.0 { f64::from(p.lanes) / 6.0 } else { f64::from(p.lanes) / (4.0 * w.min(4.0)) };
            let each = f64::from(p.scalar) * if w == 1.0 { 1.0 / 3.0 } else { 1.0 / 3.0 + 2.0 };
            // An exchange below 32 lanes: a chunk's call of the runtime, its switch out to its group's turns and
            // back, and its stores and loads, for the chunk's lanes: about 100 cycles (measured: row_text, whose
            // branches are mostly the group's, takes 6 times as long at 8 lanes as at 32, 13 times at 4).
            let exchanges = if w < 32.0 { f64::from(p.exchanges) * 100.0 / w } else { 0.0 };
            lanes + each + exchanges + (f64::from(p.other) / 6.0 + 2.0 * spilled / 3.0) / w
        })
        .sum()
}

/// Whether each function keeps its callers' registers (`CallConv::PreserveAll`) at width `WIDTHS[k]`:
/// where its callers keep more vector registers live across a call of it than it takes itself, so saving in it the
/// registers it writes costs less than its callers saving theirs. The kernel's function (the entry's) never.
pub fn keeps_callers(ps: &[FuncPressure], k: usize) -> Vec<bool> {
    let mut most = vec![0u32; ps.len()];
    for p in ps {
        for &(callee, regs) in &p.across {
            let m = &mut most[callee as usize];
            *m = (*m).max(regs[k]);
        }
    }
    ps.iter().enumerate().map(|(f, p)| f != 0 && most[f] > p.at[k].vec + p.at[k].float).collect()
}

/// The width a kernel runs at on a target of `files`: the least cost a lane.
pub fn width(ps: &[FuncPressure], files: Files) -> u32 {
    let mut best = (f64::INFINITY, 1);
    for k in 0..WIDTHS.len() {
        let c = lane_cost(ps, k, files);
        if c < best.0 {
            best = (c, WIDTHS[k]);
        }
    }
    best.1
}
