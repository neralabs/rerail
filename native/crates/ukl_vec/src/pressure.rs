//! A kernel's register pressure at each width, and the width it runs at (decision 0567, step 3): the SSA's values
//! live at each point, each in the registers its shape takes at a width (a lanes value in vector registers, a part
//! of four 32-bit or two 64-bit lanes each; a mask in a part of four lanes each; a uniform value, and at one lane a
//! lanes value, in an integer or float register), the most at any point the function's pressure in each file.
//!
//! The width is the one whose cost a lane is least, in cycles of a core that issues six integer operations, four
//! vector ones and three loads or stores a cycle (an Apple M1's, near enough an x86-64 core's), to its units at once,
//! so a lane's cycles are its busiest unit's: a lanes operation is a vector one of up to four lanes (fewer at a width
//! below four), at one lane an integer one; one a lane at a time (a load or store at each lane's address, a copy, a
//! shuffle) a load a lane, and at a width of two or more its lane moved out of its vector and back too (two cycles,
//! measured: `width_times`); the uniform operations and the control integer ones for the width's lanes; each
//! register past the target's at a loop's peak a spill and a reload each time round (outside loops, once); each
//! operation times how often its block runs (`TRIP` times a loop it is in); and below 32 lanes, a cross-lane
//! kernel's region boundaries (`cl`'s chunk loop, `ssa::regions`): at each, for each chunk, a dozen integer
//! operations, and each register live across it stored and loaded again.

use crate::k::M;
use crate::ssa::{Func, O, Shape, Ssa, V, reads};

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

/// A function's pressure at each width (`WIDTHS`'), and its operations' counts, each times how often its block runs
/// (`freq`: `TRIP` times for each loop it is in).
#[derive(Clone, Debug, Default)]
pub struct FuncPressure {
    pub at: [Pressure; 6],
    /// Each block's: how often its innermost loop's body runs (the function's once, outside loops), the loop (its
    /// header; the function's, `usize::MAX`), and its pressure at each width. A loop's registers past the target's
    /// at its peak are spilled and reloaded each time round; the function's outside loops, once.
    pub blocks: Vec<(f64, usize, [Pressure; 6])>,
    /// Its operations on lanes as vectors, on lanes a lane at a time, and the others (uniform ones and control).
    pub lanes: f64,
    pub scalar: f64,
    pub other: f64,
    /// Its loads and stores of 1- or 2-byte lanes some bytes apart: at 4 lanes or more a window's load or store and
    /// its shuffle (`cl`'s `load_bytes`, `store_span`), under 4 a lane at a time.
    pub bytes: f64,
    /// A cross-lane kernel's region boundaries below 32 lanes (`ssa::regions`: its exchanges, barriers, the group's
    /// decisions its chunks exchange, the joins of different regions), and at each width the registers live across
    /// them, which each chunk keeps and reads back.
    pub exchanges: f64,
    pub kept: [f64; 6],
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

/// The times a loop's body is taken to run, for each time its loop is entered.
pub const TRIP: f64 = 16.0;

/// Each block's loop depth and its innermost loop's header (`usize::MAX` outside loops): the natural loops of the
/// back edges (a jump to a block that dominates it).
fn depths(f: &Func) -> (Vec<u32>, Vec<usize>) {
    let nb = f.blocks.len();
    let idom = crate::ssa::idoms(f);
    let dominates = |a: usize, mut b: usize| loop {
        if b == a {
            return true;
        }
        if idom[b] == usize::MAX || idom[b] == b {
            return false;
        }
        b = idom[b];
    };
    let mut preds = vec![Vec::new(); nb];
    for b in 0..nb {
        for s in f.succs(b) {
            preds[s as usize].push(b);
        }
    }
    let mut depth = vec![0u32; nb];
    let mut inner = vec![(usize::MAX, usize::MAX); nb];
    for b in 0..nb {
        if idom[b] == usize::MAX {
            continue;
        }
        for h in f.succs(b) {
            let h = h as usize;
            if !dominates(h, b) {
                continue;
            }
            let mut body = vec![false; nb];
            body[h] = true;
            let mut work = vec![b];
            while let Some(x) = work.pop() {
                if !body[x] {
                    body[x] = true;
                    work.extend(&preds[x]);
                }
            }
            let size = body.iter().filter(|&&x| x).count();
            for (x, &inside) in body.iter().enumerate() {
                depth[x] += u32::from(inside);
                if inside && size < inner[x].0 {
                    inner[x] = (size, h);
                }
            }
        }
    }
    (depth, inner.into_iter().map(|x| x.1).collect())
}

/// A lanes integer's steps, where they are a constant: lane `j`'s value is lane 0's plus `j * step`; `group`, the
/// power of two (its log) its SIMD group lane 0's value is a multiple of, where known (as `cl`'s `Steps`).
#[derive(Clone, Copy)]
struct Stride {
    step: i64,
    group: Option<u32>,
}

/// Each lanes value's `Stride`, and each uniform integer's power of two (its log) it is a multiple of: through the
/// first dimension's thread id (steps of 1, its group lane 0 a multiple of 32), sums with uniform values, products
/// and shifts by constants, casts, and a shift or mask that takes the group's lane bits away (`cl`'s `group_bin`).
fn strides(f: &Func) -> (Vec<Option<Stride>>, Vec<u32>) {
    let n = f.insts.len();
    let mut st: Vec<Option<Stride>> = vec![None; n];
    let mut al = vec![0u32; n];
    let int = |m: M| !m.float() && !m.is_agg() && m != M::B;
    // A uniform integer constant, through integer casts that keep it.
    let konst = |mut x: V| loop {
        let i = f.inst(x);
        match i.op {
            O::Const if i.shape == Shape::Uniform => return Some(crate::ops::norm(i.m, u64::from(i.a) | u64::from(i.b) << 32)),
            O::Cast if i.shape == Shape::Uniform && int(i.m) && int(M(i.b)) => x = i.a,
            _ => return None,
        }
    };
    for b in crate::ssa::rpo(f) {
        for &x in &f.blocks[b].insts {
            let i = *f.inst(x);
            if !int(i.m) {
                continue;
            }
            let lanes = i.shape == Shape::Lanes;
            match i.op {
                O::Const => al[x as usize] = konst(x).map_or(0, |k| k.trailing_zeros()),
                O::Lid if lanes && konst(i.a) == Some(0) => st[x as usize] = Some(Stride { step: 1, group: Some(5) }),
                O::Cast if int(M(i.b)) => {
                    st[x as usize] = st[i.a as usize];
                    al[x as usize] = al[i.a as usize];
                }
                O::Bin => {
                    let op = crate::k::Op::of(u32::from(i.sub));
                    let (ka, kb) = (konst(i.a), konst(i.b));
                    use crate::k::Op;
                    if lanes {
                        let r = match op {
                            Op::Add => [(i.a, i.b), (i.b, i.a)].into_iter().find_map(|(s, u)| {
                                let s = st[s as usize]?;
                                (f.inst(u).shape == Shape::Uniform).then(|| Stride { step: s.step, group: s.group.map(|g| g.min(al[u as usize])) })
                            }),
                            Op::Mul => [(i.a, kb), (i.b, ka)].into_iter().find_map(|(s, c)| {
                                let (s, c) = (st[s as usize]?, c?);
                                Some(Stride { step: s.step.wrapping_mul(c as i64), group: s.group.map(|g| (g + c.trailing_zeros()).min(64)) })
                            }),
                            Op::Shl => st[i.a as usize].zip(kb.filter(|&k| k < 64)).map(|(s, k)| Stride { step: s.step.wrapping_shl(k as u32), group: s.group.map(|g| (g + k as u32).min(64)) }),
                            Op::Shr | Op::And => st[i.a as usize].zip(kb).and_then(|(s, k)| {
                                let bits = if op == Op::Shr { k as u32 } else { (k + 1).trailing_zeros() };
                                let fits = s.step >= 0 && bits < 63 && (s.step as i128) * 31 < (1i128 << bits) && s.group? >= bits;
                                if !fits || (op == Op::And && !(k + 1).is_power_of_two()) {
                                    return None;
                                }
                                Some(if op == Op::Shr { Stride { step: 0, group: Some(s.group? - bits) } } else { Stride { step: s.step, group: Some(64) } })
                            }),
                            _ => None,
                        };
                        st[x as usize] = r;
                    } else {
                        al[x as usize] = match op {
                            Op::Add => al[i.a as usize].min(al[i.b as usize]),
                            Op::Mul => (al[i.a as usize] + al[i.b as usize]).min(64),
                            Op::Shl => kb.map_or(0, |k| (al[i.a as usize] + k as u32).min(64)),
                            _ => 0,
                        };
                    }
                }
                _ => {}
            }
        }
    }
    (st, al)
}

/// Function `f`'s pressure at each width.
pub fn of_func(ssa: &Ssa, f: &Func) -> FuncPressure {
    let n = f.insts.len();
    let nb = f.blocks.len();
    let (depth, inner) = depths(f);
    let freq: Vec<f64> = depth.iter().map(|&d| TRIP.powi(d as i32)).collect();
    let (stride, _) = strides(f);
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
            let fr = freq[b];
            let lanes = |u: u32| f.inst(u).shape == Shape::Lanes;
            // A lanes access: as `cl` lowers it, by its address's steps (one address; contiguous lanes of 4 or 8
            // bytes; lanes of 1 or 2 some bytes apart), else a lane at a time.
            let access = |eb: u32| match stride[i.a as usize] {
                Some(s) if s.step == 0 && i.op == O::Load => Some(false),
                Some(s) if s.step == i64::from(eb) && matches!(eb, 4 | 8) => Some(false),
                Some(s) if matches!(eb, 1 | 2) && s.step >= i64::from(eb) && s.step <= 16 => Some(true),
                _ => None,
            };
            let shift = i.op == O::Bin
                && lanes(x)
                && matches!(crate::k::Op::of(u32::from(i.sub)), crate::k::Op::Shl | crate::k::Op::Shr)
                && lanes(i.b);
            let kind = match i.op {
                O::Load | O::Store if lanes(i.a) && f.inst(i.a).op != O::FrameAddr => match access(i.m.bytes()) {
                    Some(true) => 'b',
                    Some(false) => 'l',
                    None => 'e',
                },
                // A shift by each lane's count: a shift and a multiply where its counts have steps, else a lane at
                // a time (Cranelift's vectors shift by one count).
                _ if shift => if stride[i.b as usize].is_some() { 'L' } else { 'e' },
                O::Atomic if i.shape == Shape::Lanes => 'e',
                O::Copy | O::Zero | O::Shuffle => 'e',
                _ => match i.shape {
                    Shape::Lanes | Shape::Mask => 'l',
                    _ => 'o',
                },
            };
            match kind {
                'e' => fp.scalar += fr,
                'b' => fp.bytes += fr,
                'l' => fp.lanes += fr,
                'L' => fp.lanes += 2.0 * fr,
                _ => fp.other += fr,
            }
        }
    }
    // A chunked kernel's region boundaries: the kernel's own (its callees' cross-lane operations keep it at 32 lanes).
    let chunked = f.cross && std::ptr::eq(f, &ssa.funcs[0]);
    let dec = if chunked { crate::ssa::group_decisions(ssa, f) } else { vec![false; nb] };
    let (_, resume) = if chunked { crate::ssa::regions(f, &dec) } else { (Vec::new(), vec![false; nb]) };
    // The registers at each width the values `live` a boundary keeps take (those the region makes again apart).
    let kept = |live: &Bits| -> [f64; 6] {
        let mut r = [0.0; 6];
        for x in live.iter().filter(|&x| f.inst(x).op != O::Entry && !crate::ssa::remade(f, x)) {
            for (k, &w) in WIDTHS.iter().enumerate() {
                let c = cost(f, x, w);
                r[k] += f64::from(c.0 + c.1 + c.2);
            }
        }
        r
    };
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
        let mut here = [Pressure::default(); 6];
        let note = |sum: &[(u32, u32, u32); 6], fp: &mut FuncPressure, here: &mut [Pressure; 6]| {
            for k in 0..6 {
                for p in [&mut fp.at[k], &mut here[k]] {
                    (p.vec, p.int, p.float) = (p.vec.max(sum[k].0), p.int.max(sum[k].1), p.float.max(sum[k].2));
                }
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
        note(&sum, &mut fp, &mut here);
        for &x in blk.insts.iter().rev() {
            if !live.has(x) {
                // Defined, not read after: live at its definition only.
                live.set(x);
                add(&mut sum, x, 1);
            }
            note(&sum, &mut fp, &mut here);
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
            note(&sum, &mut fp, &mut here);
            let n = if chunked { crate::ssa::splits(f, &dec, b, x) } else { 0 };
            if n > 0 {
                let r = kept(&live);
                fp.exchanges += freq[b] * f64::from(n);
                for k in 0..6 {
                    fp.kept[k] += freq[b] * f64::from(n) * r[k];
                }
            }
        }
        if resume[b] {
            // A join of regions: its parameters and the values live into it kept.
            let mut l = live.clone();
            for &p in &blk.params {
                l.set(p);
            }
            let r = kept(&l);
            fp.exchanges += freq[b];
            for k in 0..6 {
                fp.kept[k] += freq[b] * r[k];
            }
        }
        fp.blocks.push((freq[b], inner[b], here));
    }
    fp
}

/// A kernel's pressure: each function's.
pub fn of(ssa: &Ssa) -> Vec<FuncPressure> {
    ssa.funcs.iter().map(|f| of_func(ssa, f)).collect()
}

/// The cost a lane of the kernel's functions at width `WIDTHS[k]`, the target's `files` (in cycles, per lane, each
/// operation and spill times how often its block runs).
pub fn lane_cost(ps: &[FuncPressure], k: usize, files: Files) -> f64 {
    let w = WIDTHS[k];
    let past = |p: u32, have: u32| f64::from(p.saturating_sub(have));
    ps.iter()
        .map(|p| {
            // Each loop's registers past the target's at its peak: a spill and reload each time round (the
            // function's outside loops, once). Floats share the vector registers on both targets.
            let mut peak: Vec<(usize, f64, f64)> = Vec::new();
            for &(fr, lp, at) in &p.blocks {
                let at = at[k];
                let x = if w == 1 { past(at.int, files.int) + past(at.float, files.vec) } else { past(at.vec + at.float, files.vec) + past(at.int, files.int) };
                match peak.iter_mut().find(|r| r.0 == lp) {
                    Some(r) => r.2 = r.2.max(x),
                    None => peak.push((lp, fr, x)),
                }
            }
            let spilled: f64 = peak.iter().map(|r| r.1 * r.2).sum();
            let w = f64::from(w);
            let (vector, each) = if w >= 4.0 { (p.lanes + 2.0 * p.bytes, p.scalar) } else { (p.lanes, p.scalar + p.bytes) };
            // The core issues to its integer, vector and memory units at once: a lane's cycles are its busiest
            // unit's. At one lane, the lanes operations are integer ones, a lane-at-a-time access a load or store;
            // at more, a lanes operation is a vector one of up to four lanes, a lane-at-a-time access a load or
            // store and its lane's moves out of its vector and back (two cycles, measured: `width_times`). The
            // uniform operations and control are integer ones for the width's lanes; a spill a store and a reload.
            let (int, vec) = if w == 1.0 { (vector / 6.0 + p.other / 6.0, 0.0) } else { (p.other / 6.0 / w, vector / (4.0 * w.min(4.0)) + 2.0 * each) };
            // Below 32 lanes, a chunked kernel's region boundaries (`cl`'s chunk loop): for each chunk, a dozen
            // integer operations (the exchange's word, the next region's number and its check, the chunk loop, the
            // dispatch), and a store and a load of each register live across it.
            let (rint, rmem) = if w < 32.0 { (p.exchanges * 12.0 / 6.0 / w, 2.0 * p.kept[k] / 3.0 / w) } else { (0.0, 0.0) };
            let mem = (each + 2.0 * spilled / w) / 3.0 + rmem;
            (int + rint).max(vec).max(mem)
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

/// The width a kernel runs at on a target of `files`: the least cost a lane, the widest of equal ones (fewer
/// chunks, each its call and its masks).
pub fn width(ps: &[FuncPressure], files: Files) -> u32 {
    let mut best = (f64::INFINITY, 1);
    for k in (0..WIDTHS.len()).rev() {
        let c = lane_cost(ps, k, files);
        if c < best.0 {
            best = (c, WIDTHS[k]);
        }
    }
    best.1
}
