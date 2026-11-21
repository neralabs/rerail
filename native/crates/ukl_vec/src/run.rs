//! A kernel's vector tape run by an interpreter at 32 lanes (decision 0567 §6, the first back end): each node over a
//! SIMD group's lanes at once, its active lanes a mask. A uniform value with no effect is computed once for its SIMD
//! group; a varying one lane by lane; a branch on a varying value runs each side under its lanes' mask, and a loop
//! until its mask is empty. A node with a threadgroup barrier in it (`GSYNC`, the cut) runs for all of its group's
//! SIMD groups at once, each region between barriers SIMD group by SIMD group. SIMD-group operations are the lanes'
//! values permuted (a shuffle) or gathered (a ballot). The tapes are read in place.
//!
//! A lane's variables are its own, once a lane in its SIMD group's frame (struct of arrays): the variable at offset
//! `off` of `size` bytes is lane `l`'s at `off * W + l * size`.

use std::cell::RefCell;
use std::collections::HashMap;

use crate::Error;
use crate::k::{Builtin, CHECK, EFFECT, Kind, Level, M, NO_SLOT, Op, View};
use crate::lower::Program;
use crate::ops;
use crate::vector::Kernel;

/// The SIMD group's lanes.
pub const W: usize = 32;

type Vals = [u64; W];

/// A kernel argument: its bytes (a pointer's address, a structure's bytes), or an offset in the group's
/// threadgroup memory past the kernel's own (`set_thread_group_memory`).
#[derive(Clone, Debug)]
pub enum Arg {
    Bytes(Vec<u8>),
    Local(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flow {
    Normal,
    Break,
    Continue,
    Return,
}

/// A SIMD group: its lanes' control, the lanes that have ended, its frame.
struct Sg {
    index: u32,
    flow: [Flow; W],
    ended: u32,
    frame: u64,
}

impl Sg {
    fn normal(&self) -> u32 {
        let mut m = 0;
        for (l, f) in self.flow.iter().enumerate() {
            if *f == Flow::Normal {
                m |= 1 << l;
            }
        }
        m & !self.ended
    }
}

type R<T> = Result<T, Error>;

/// The lanes of a mask, in order.
fn lanes(mut m: u32) -> impl Iterator<Item = usize> {
    std::iter::from_fn(move || {
        if m == 0 {
            return None;
        }
        let l = m.trailing_zeros() as usize;
        m &= m - 1;
        Some(l)
    })
}

fn first(m: u32) -> usize {
    if m == 0 { 0 } else { m.trailing_zeros() as usize }
}

/// One dispatch's state.
struct Run<'p> {
    p: &'p Program,
    v: View<'p>,
    kernel: &'p str,
    data: u64,
    shared: u64,
    grid: [u32; 3],
    block: [u32; 3],
    group: [u32; 3],
    /// Each checked branch's decisions in the group so far, by the first SIMD group to make them, and how many of
    /// them each SIMD group has made.
    agreed: RefCell<HashMap<usize, Decisions>>,
}

/// A checked branch's decisions in the group, in order, and how many each SIMD group has made.
type Decisions = (Vec<bool>, Vec<usize>);

/// Runs `k` (a kernel of `p`) over `grid` groups of `block` threads, with `args` for its parameters. `local` is
/// the threadgroup memory a group has past the kernel's own (its `Arg::Local` arguments').
pub fn run(p: &Program, k: &Kernel, args: &[Arg], grid: [u32; 3], block: [u32; 3], local: u64) -> R<()> {
    let v = View::new(&k.tape);
    let root = 0;
    let n_params = v.w(root, 4) as usize;
    if args.len() != n_params {
        return Err(Error(format!("{} takes {n_params} arguments, given {}", k.name, args.len())));
    }
    let threads = block.iter().map(|&b| b as usize).product::<usize>();
    let nsg = threads.div_ceil(W);
    let frame = (v.w(root, 1) as usize).next_multiple_of(16) * W;
    let mut frames = vec![0u128; (nsg * frame).div_ceil(16).max(1)];
    let own = u64::from(v.w(root, 2)).next_multiple_of(16);
    let mut shared = vec![0u128; (own + local).div_ceil(16) as usize + 1];
    let pv = View::new(&p.tape);
    let data_node = pv.children(0).last().expect("the program's data");
    let data = pv.ws(data_node, 1, 0).as_ptr() as u64;
    let mut r = Run { p, v, kernel: &k.name, data, shared: shared.as_mut_ptr() as u64, grid, block, group: [0; 3], agreed: RefCell::new(HashMap::new()) };
    let body = r.v.child(root, 0);
    for gz in 0..grid[2] {
        for gy in 0..grid[1] {
            for gx in 0..grid[0] {
                r.group = [gx, gy, gz];
                r.agreed.borrow_mut().clear();
                shared.fill(0);
                frames.fill(0);
                let base = frames.as_mut_ptr() as u64;
                let mut g: Vec<Sg> = (0..nsg as u32).map(|s| Sg { index: s, flow: [Flow::Normal; W], ended: 0, frame: base + u64::from(s) * frame as u64 }).collect();
                let mut masks = vec![0u32; nsg];
                for (s, sg) in g.iter().enumerate() {
                    let n = (threads - s * W).min(W);
                    masks[s] = if n == W { u32::MAX } else { (1u32 << n) - 1 };
                    for (k, a) in args.iter().enumerate() {
                        let (off, m) = (u64::from(r.v.w(root, 5 + 2 * k)), M(r.v.w(root, 6 + 2 * k)));
                        for l in lanes(masks[s]) {
                            let at = sg.frame + off * W as u64 + l as u64 * u64::from(m.bytes());
                            match a {
                                // SAFETY: the parameter's place in the lane's frame, its type's bytes.
                                Arg::Bytes(b) => unsafe { std::ptr::copy_nonoverlapping(b.as_ptr(), at as usize as *mut u8, b.len().min(m.bytes() as usize)) },
                                Arg::Local(o) => unsafe { ops::store(M::PTR, at, r.shared + own + o) },
                            }
                        }
                    }
                }
                if r.v.gsync(body) {
                    r.gexec(&mut g, body, &masks)?;
                } else {
                    for (s, sg) in g.iter_mut().enumerate() {
                        r.vexec(sg, body, masks[s])?;
                    }
                }
            }
        }
    }
    Ok(())
}

impl Run<'_> {
    fn site(&self, w: u32) -> &str {
        self.p.sites.get(w as usize).map_or("?", String::as_str)
    }

    fn lid(&self, sg: &Sg, lane: usize) -> ([u32; 3], u32) {
        let lin = sg.index * W as u32 + lane as u32;
        let (bx, by) = (self.block[0], self.block[1]);
        ([lin % bx, lin / bx % by, lin / (bx * by)], lin)
    }

    /// A variable's (a `Local`'s payload `p`) address in lane `l`.
    fn local(&self, sg: &Sg, p: &[u32], l: usize) -> u64 {
        sg.frame + u64::from(p[0]) * W as u64 + l as u64 * u64::from(p[1]) + u64::from(p[2])
    }

    /// Whether node `n`'s value is computed once for its SIMD group: a value, uniform in it, without effect.
    fn once(&self, n: usize) -> bool {
        matches!(
            self.v.kind(n),
            Kind::Const | Kind::Global | Kind::Shared | Kind::Load | Kind::Bin | Kind::Un | Kind::Cast | Kind::Cond | Kind::Logic | Kind::Comma | Kind::Builtin
        ) && self.v.level(n) <= Level::SimdGroup
            && self.v.field(n) & EFFECT == 0
    }

    /// A checked branch's decision `d`, by SIMD group `sg`: the same as the first SIMD group's, or the kernel fails.
    fn agree(&self, n: usize, sg: &Sg, d: bool, site: u32) -> R<()> {
        let mut a = self.agreed.borrow_mut();
        let (ds, at) = a.entry(n).or_insert_with(|| (Vec::new(), Vec::new()));
        let s = sg.index as usize;
        if at.len() <= s {
            at.resize(s + 1, 0);
        }
        let k = at[s];
        at[s] += 1;
        if k == ds.len() {
            ds.push(d);
        } else if ds[k] != d {
            return Err(Error(format!(
                "{}: {} decides differently in SIMD group {s} than in the first: an agreed value the barrier rule rests on differs across the group (decision 0567 §3)",
                self.kernel,
                self.site(site)
            )));
        }
        Ok(())
    }

    // One lane.

    /// Node `n`'s value in lane `l` (a value without effect).
    fn sval(&self, sg: &Sg, n: usize, l: usize) -> R<u64> {
        let v = &self.v;
        let p = v.p(n);
        Ok(match v.kind(n) {
            Kind::Const => u64::from(p[1]) | u64::from(p[2]) << 32,
            Kind::Local => self.local(sg, p, l),
            Kind::Shared => self.shared + u64::from(p[0]),
            Kind::Global => self.data + u64::from(p[0]),
            Kind::Load => {
                let a = self.sval(sg, v.child(n, 0), l)?;
                // SAFETY: the kernel's address (its buffers', frames' and threadgroup memory's are the dispatch's).
                unsafe { ops::load(M(p[0]), a) }
            }
            Kind::Bin => {
                let a = self.sval(sg, v.child(n, 0), l)?;
                let b = self.sval(sg, v.child(n, 1), l)?;
                ops::bin(Op::of(p[0]), M(p[1]), a, b).map_err(|e| Error(e.to_string()))?
            }
            Kind::Un => ops::un(Op::of(p[0]), M(p[1]), self.sval(sg, v.child(n, 0), l)?),
            Kind::Cast => ops::cast(M(p[0]), M(p[1]), self.sval(sg, v.child(n, 0), l)?),
            Kind::Cond => {
                let c = self.sval(sg, v.child(n, 0), l)?;
                self.sval(sg, v.child(n, if c != 0 { 1 } else { 2 }), l)?
            }
            Kind::Logic => {
                let and = p[0] != 0;
                let a = self.sval(sg, v.child(n, 0), l)? != 0;
                if a != and { u64::from(a) } else { u64::from(self.sval(sg, v.child(n, 1), l)? != 0) }
            }
            Kind::Comma => self.sval(sg, v.child(n, 1), l)?,
            Kind::Builtin => {
                let mut a = [0u64; 3];
                for (k, c) in v.children(n).enumerate() {
                    a[k] = self.sval(sg, c, l)?;
                }
                self.builtin(sg, l, n, a)?
            }
            k => return Err(Error(format!("a {k:?} node computed once"))),
        })
    }

    /// A builtin with no effect beyond its lane, in lane `l`.
    fn builtin(&self, sg: &Sg, l: usize, n: usize, a: [u64; 3]) -> R<u64> {
        let p = self.v.p(n);
        let (b, m) = (Builtin::of(p[0]), M(p[1]));
        let (lid, lin) = self.lid(sg, l);
        let d = a[0] as usize;
        let dim = |x: [u32; 3], or: u32| if d < 3 { x[d] } else { or };
        Ok(match b {
            Builtin::LocalId => u64::from(dim(lid, 0)),
            Builtin::LocalSize => u64::from(dim(self.block, 1)),
            Builtin::GroupId => u64::from(dim(self.group, 0)),
            Builtin::NumGroups => u64::from(dim(self.grid, 1)),
            Builtin::GlobalId => u64::from(dim(self.group, 0)) * u64::from(dim(self.block, 1)) + u64::from(dim(lid, 0)),
            Builtin::GlobalSize => u64::from(dim(self.grid, 1)) * u64::from(dim(self.block, 1)),
            Builtin::SubGroupLocalId => u64::from(lin % W as u32),
            Builtin::MemFence | Builtin::Barrier | Builtin::SubGroupBarrier => 0,
            Builtin::Popcount => u64::from((ops::norm(m, a[0]) & mask(m)).count_ones()),
            Builtin::Clz => u64::from((ops::norm(m, a[0]) & mask(m)).leading_zeros() - (64 - m.bits())),
            Builtin::MulHi => {
                if m.bits() == 64 {
                    if m.signed() { ((i128::from(a[0] as i64) * i128::from(a[1] as i64)) >> 64) as u64 } else { ((u128::from(a[0]) * u128::from(a[1])) >> 64) as u64 }
                } else if m.signed() {
                    ops::norm(m, ((a[0] as i64).wrapping_mul(a[1] as i64) >> m.bits()) as u64)
                } else {
                    ops::norm(m, (ops::norm(m, a[0]).wrapping_mul(ops::norm(m, a[1]))) >> m.bits())
                }
            }
            Builtin::As => ops::norm(m, a[0] & mask(M(p[4]))),
            Builtin::Min | Builtin::Max => {
                let less = ops::bin(Op::Lt, m, a[0], a[1]).unwrap_or(0) != 0;
                if less == (b == Builtin::Min) { a[0] } else { a[1] }
            }
            _ => return Err(Error(format!("{} as a lane's own", self.site(p[3])))),
        })
    }

    /// An atomic at `a[0]`, one lane's.
    fn atomic(&self, n: usize, a: [u64; 3]) -> u64 {
        let p = self.v.p(n);
        let (b, m) = (Builtin::of(p[0]), M(p[1]));
        let ptr = a[0];
        // SAFETY: the kernel's address; the interpreter runs one lane at a time, so an atomic is its read and write.
        let old = unsafe { ops::load(m, ptr) };
        let new = match b {
            Builtin::AtomicAdd => old.wrapping_add(a[1]),
            Builtin::AtomicSub => old.wrapping_sub(a[1]),
            Builtin::AtomicAnd => old & a[1],
            Builtin::AtomicOr => old | a[1],
            Builtin::AtomicXor => old ^ a[1],
            Builtin::AtomicXchg => a[1],
            Builtin::AtomicInc => old.wrapping_add(1),
            Builtin::AtomicDec => old.wrapping_sub(1),
            Builtin::AtomicMin | Builtin::AtomicMax => {
                let less = ops::bin(Op::Lt, m, a[1], old).unwrap_or(0) != 0;
                if less == (b == Builtin::AtomicMin) { a[1] } else { old }
            }
            _ => {
                if ops::norm(m, old) == ops::norm(m, a[1]) {
                    a[2]
                } else {
                    old
                }
            }
        };
        unsafe { ops::store(m, ptr, new) };
        old
    }

    // A SIMD group.

    /// Node `n`'s values in the lanes `mask` of `sg` (the others' unspecified).
    fn vval(&self, sg: &mut Sg, n: usize, mask: u32) -> R<Vals> {
        if self.once(n) {
            return Ok([self.sval(sg, n, first(mask))?; W]);
        }
        let v = &self.v;
        let p = v.p(n);
        let mut out = [0u64; W];
        match v.kind(n) {
            Kind::Local => {
                for l in lanes(mask) {
                    out[l] = self.local(sg, p, l);
                }
            }
            Kind::Cond => {
                let c = self.vval(sg, v.child(n, 0), mask)?;
                let yes = lanes(mask).filter(|&l| c[l] != 0).fold(0u32, |m, l| m | 1 << l);
                let a = if yes != 0 { self.vval(sg, v.child(n, 1), yes)? } else { out };
                let b = if mask & !yes != 0 { self.vval(sg, v.child(n, 2), mask & !yes)? } else { out };
                for l in lanes(mask) {
                    out[l] = if yes >> l & 1 != 0 { a[l] } else { b[l] };
                }
            }
            Kind::Logic => {
                let and = p[0] != 0;
                let a = self.vval(sg, v.child(n, 0), mask)?;
                let rest = lanes(mask).filter(|&l| (a[l] != 0) == and).fold(0u32, |m, l| m | 1 << l);
                let b = if rest != 0 { self.vval(sg, v.child(n, 1), rest)? } else { out };
                for l in lanes(mask) {
                    out[l] = u64::from(if rest >> l & 1 != 0 { b[l] != 0 } else { a[l] != 0 });
                }
            }
            Kind::Inline => return self.vinline(sg, n, mask),
            Kind::Block | Kind::Expr => {
                self.vexec(sg, n, mask)?;
            }
            Kind::Builtin => {
                let mut xs = [[0u64; W]; 3];
                for (j, c) in v.children(n).enumerate() {
                    xs[j] = self.vval(sg, c, mask)?;
                }
                self.vbuiltin(sg, n, mask, &xs, &mut out)?;
            }
            _ => {
                let mut xs = [[0u64; W]; 3];
                for (j, c) in v.children(n).enumerate() {
                    xs[j] = self.vval(sg, c, mask)?;
                }
                self.vop(sg, n, mask, [&xs[0], &xs[1], &xs[2]], &mut out)?;
            }
        }
        Ok(out)
    }

    /// Node `n`'s operation over the lanes `mask`, its operands' values `x` (a node whose operands are all evaluated
    /// first): the node read once, its lanes in a loop.
    fn vop(&self, sg: &Sg, n: usize, mask: u32, x: [&Vals; 3], out: &mut Vals) -> R<()> {
        let p = self.v.p(n);
        match self.v.kind(n) {
            Kind::Const => out.fill(u64::from(p[1]) | u64::from(p[2]) << 32),
            Kind::Shared => out.fill(self.shared + u64::from(p[0])),
            Kind::Global => out.fill(self.data + u64::from(p[0])),
            Kind::Local => {
                let base = sg.frame + u64::from(p[0]) * W as u64 + u64::from(p[2]);
                let size = u64::from(p[1]);
                for l in lanes(mask) {
                    out[l] = base + l as u64 * size;
                }
            }
            Kind::Load => {
                let m = M(p[0]);
                for l in lanes(mask) {
                    // SAFETY: the kernel's addresses.
                    out[l] = unsafe { ops::load(m, x[0][l]) };
                }
            }
            Kind::Store => {
                let m = M(p[0]);
                for l in lanes(mask) {
                    unsafe { ops::store(m, x[0][l], x[1][l]) };
                    out[l] = x[1][l];
                }
            }
            Kind::Bin => {
                let (op, m) = (Op::of(p[0]), M(p[1]));
                for l in lanes(mask) {
                    out[l] = ops::bin(op, m, x[0][l], x[1][l]).map_err(|e| Error(e.to_string()))?;
                }
            }
            Kind::Un => {
                let (op, m) = (Op::of(p[0]), M(p[1]));
                for l in lanes(mask) {
                    out[l] = ops::un(op, m, x[0][l]);
                }
            }
            Kind::Cast => {
                let (from, to) = (M(p[0]), M(p[1]));
                for l in lanes(mask) {
                    out[l] = ops::cast(from, to, x[0][l]);
                }
            }
            Kind::Comma => *out = *x[1],
            _ => {
                for l in lanes(mask) {
                    out[l] = self.lane_op(sg, n, l, [x[0][l], x[1][l], x[2][l]])?;
                }
            }
        }
        Ok(())
    }

    /// Node `n`'s operation in lane `l`, its operands' values `x` (a node whose operands are all evaluated first).
    fn lane_op(&self, sg: &Sg, n: usize, l: usize, x: [u64; 3]) -> R<u64> {
        let p = self.v.p(n);
        Ok(match self.v.kind(n) {
            Kind::Const => u64::from(p[1]) | u64::from(p[2]) << 32,
            Kind::Shared => self.shared + u64::from(p[0]),
            Kind::Global => self.data + u64::from(p[0]),
            Kind::Local => self.local(sg, p, l),
            // SAFETY: the kernel's addresses.
            Kind::Load => unsafe { ops::load(M(p[0]), x[0]) },
            Kind::Store => {
                unsafe { ops::store(M(p[0]), x[0], x[1]) };
                x[1]
            }
            Kind::Zero => {
                unsafe { std::ptr::write_bytes(x[0] as usize as *mut u8, 0, p[1] as usize) };
                x[0]
            }
            Kind::Copy => {
                unsafe { std::ptr::copy(x[1] as usize as *const u8, x[0] as usize as *mut u8, p[0] as usize) };
                x[0]
            }
            Kind::Bin => ops::bin(Op::of(p[0]), M(p[1]), x[0], x[1]).map_err(|e| Error(e.to_string()))?,
            Kind::Un => ops::un(Op::of(p[0]), M(p[1]), x[0]),
            Kind::Cast => ops::cast(M(p[0]), M(p[1]), x[0]),
            Kind::Comma => x[1],
            Kind::Update => {
                let (op, lm, om, post) = (Op::of(p[0]), M(p[1]), M(p[2]), p[3] != 0);
                let old = unsafe { ops::load(lm, x[0]) };
                let r = ops::bin(op, om, ops::cast(lm, om, old), x[1]).unwrap_or(0);
                let new = ops::cast(om, lm, r);
                unsafe { ops::store(lm, x[0], new) };
                if post { old } else { new }
            }
            Kind::Builtin => self.builtin(sg, l, n, x)?,
            k => return Err(Error(format!("a {k:?} node as a lane's operation"))),
        })
    }

    /// A builtin over the lanes `mask`: a SIMD-group operation the lanes' values permuted or gathered, an atomic one
    /// lane after another (once, for the SIMD group, when its value is uniform in it).
    fn vbuiltin(&self, sg: &Sg, n: usize, mask: u32, xs: &[Vals; 3], out: &mut Vals) -> R<()> {
        let p = self.v.p(n);
        let b = Builtin::of(p[0]);
        let from = |src: Option<usize>, or: u64| match src {
            Some(s) if mask >> s & 1 != 0 => xs[0][s],
            _ => or,
        };
        match b {
            Builtin::SubGroupBallot => {
                let m = lanes(mask).filter(|&l| xs[0][l] != 0).fold(0u64, |m, l| m | 1 << l);
                for l in lanes(mask) {
                    out[l] = m;
                }
            }
            Builtin::SubGroupShuffle => {
                for l in lanes(mask) {
                    out[l] = from(Some(xs[1][l] as usize % W), 0);
                }
            }
            Builtin::SubGroupShuffleXor => {
                for l in lanes(mask) {
                    out[l] = from(Some((l ^ xs[1][l] as usize) % W), 0);
                }
            }
            Builtin::SubGroupShuffleUp => {
                for l in lanes(mask) {
                    out[l] = from(l.checked_sub(xs[1][l] as usize), xs[0][l]);
                }
            }
            Builtin::SubGroupShuffleDown => {
                for l in lanes(mask) {
                    let s = l + xs[1][l] as usize;
                    out[l] = from((s < W).then_some(s), xs[0][l]);
                }
            }
            Builtin::SubGroupBarrier | Builtin::Barrier | Builtin::MemFence => {}
            _ if crate::vector::is_atomic(b) => {
                if self.v.level(n) <= Level::SimdGroup {
                    let l = first(mask);
                    let r = self.atomic(n, [xs[0][l], xs[1][l], xs[2][l]]);
                    for l in lanes(mask) {
                        out[l] = r;
                    }
                } else {
                    for l in lanes(mask) {
                        out[l] = self.atomic(n, [xs[0][l], xs[1][l], xs[2][l]]);
                    }
                }
            }
            _ => {
                for l in lanes(mask) {
                    out[l] = self.builtin(sg, l, n, [xs[0][l], xs[1][l], xs[2][l]])?;
                }
            }
        }
        Ok(())
    }

    /// An inlined call over the lanes `mask`: its parameters' stores and body, then its result, the lanes that
    /// returned back.
    fn vinline(&self, sg: &mut Sg, n: usize, mask: u32) -> R<Vals> {
        let v = &self.v;
        let mut live = mask;
        for c in v.children(n) {
            if live == 0 {
                break;
            }
            self.vexec(sg, c, live)?;
            live &= sg.normal();
        }
        for l in lanes(mask) {
            if sg.flow[l] == Flow::Return {
                sg.flow[l] = Flow::Normal;
            }
        }
        Ok(self.result(sg, n, mask & !sg.ended))
    }

    /// An inlined call's result in the lanes `mask`: its slot's value, or an aggregate's copy's address.
    fn result(&self, sg: &Sg, n: usize, mask: u32) -> Vals {
        let p = self.v.p(n);
        let m = M(p[0]);
        let mut out = [0u64; W];
        if m == M::VOID {
            return out;
        }
        for l in lanes(mask) {
            let slot = self.local(sg, &[p[1], m.bytes(), 0], l);
            out[l] = if m.is_agg() {
                let to = self.local(sg, &[p[2], m.bytes(), 0], l);
                // SAFETY: two places of the lane's frame, the aggregate's bytes.
                unsafe { std::ptr::copy(slot as usize as *const u8, to as usize as *mut u8, m.bytes() as usize) };
                to
            } else {
                unsafe { ops::load(m, slot) }
            };
        }
        out
    }

    /// A branch's decision on the lanes `mask` of `sg` from its condition's values `c`: the lanes it takes, the
    /// decision checked (`CHECK`) against the group's.
    fn decide(&self, sg: &Sg, n: usize, mask: u32, c: &Vals) -> R<u32> {
        let yes = lanes(mask).filter(|&l| c[l] != 0).fold(0u32, |m, l| m | 1 << l);
        if self.v.field(n) & CHECK != 0 && mask != 0 {
            self.agree(n, sg, yes != 0, self.v.w(n, 0))?;
        }
        Ok(yes)
    }

    /// Statement `n` over the lanes `mask` of `sg`: each lane's control in its `flow`.
    fn vexec(&self, sg: &mut Sg, n: usize, mask: u32) -> R<()> {
        let v = &self.v;
        let p = v.p(n);
        match v.kind(n) {
            Kind::Block => {
                let mut live = mask;
                for c in v.children(n) {
                    if live == 0 {
                        break;
                    }
                    self.vexec(sg, c, live)?;
                    live &= sg.normal();
                }
            }
            Kind::Expr => {
                self.vval(sg, v.child(n, 0), mask)?;
            }
            Kind::Empty => {}
            Kind::If => {
                let c = self.vval(sg, v.child(n, 0), mask)?;
                let yes = self.decide(sg, n, mask, &c)?;
                if yes != 0 {
                    self.vexec(sg, v.child(n, 1), yes)?;
                }
                if mask & !yes != 0 && v.children(n).count() > 2 {
                    self.vexec(sg, v.child(n, 2), mask & !yes)?;
                }
            }
            Kind::While | Kind::For | Kind::Do => {
                let k = v.kind(n);
                let (cond, body) = if k == Kind::Do { (v.child(n, 1), v.child(n, 0)) } else { (v.child(n, 0), v.child(n, 1)) };
                let mut lp = mask;
                let mut first = k == Kind::Do;
                loop {
                    if !first {
                        let c = self.vval(sg, cond, lp)?;
                        lp = self.decide(sg, n, lp, &c)?;
                    }
                    first = false;
                    if lp == 0 {
                        break;
                    }
                    self.vexec(sg, body, lp)?;
                    lp = Self::rejoin(sg, lp);
                    if k == Kind::For && lp != 0 {
                        self.vexec(sg, v.child(n, 2), lp)?;
                    }
                }
            }
            Kind::Return => {
                let x = if p[0] != 0 { self.vval(sg, v.child(n, 0), mask)? } else { [0; W] };
                self.ret(sg, p, mask, &x);
            }
            Kind::Break => {
                for l in lanes(mask) {
                    sg.flow[l] = Flow::Break;
                }
            }
            Kind::Continue => {
                for l in lanes(mask) {
                    sg.flow[l] = Flow::Continue;
                }
            }
            _ => {
                self.vval(sg, n, mask)?;
            }
        }
        Ok(())
    }

    /// The lanes of a loop's iteration that go on: a `break`'s leave, a `continue`'s come back, a `return`'s leave.
    fn rejoin(sg: &mut Sg, lp: u32) -> u32 {
        let mut next = 0;
        for l in lanes(lp) {
            match sg.flow[l] {
                Flow::Break => sg.flow[l] = Flow::Normal,
                Flow::Continue => {
                    sg.flow[l] = Flow::Normal;
                    next |= 1 << l;
                }
                Flow::Return => {}
                Flow::Normal => next |= 1 << l,
            }
        }
        next & !sg.ended
    }

    /// A `return` of the lanes `mask`, their values `x`: stored in its call's slot, or the lanes end.
    fn ret(&self, sg: &mut Sg, p: &[u32], mask: u32, x: &Vals) {
        let (slot, m) = (p[2], M(p[3]));
        for l in lanes(mask) {
            sg.flow[l] = Flow::Return;
            if slot == NO_SLOT {
                sg.ended |= 1 << l;
            } else if p[0] != 0 {
                let at = self.local(sg, &[slot, m.bytes(), 0], l);
                if m.is_agg() {
                    // SAFETY: the value is the aggregate's address; the slot, the lane's.
                    unsafe { std::ptr::copy(x[l] as usize as *const u8, at as usize as *mut u8, m.bytes() as usize) };
                } else {
                    unsafe { ops::store(m, at, x[l]) };
                }
            }
        }
    }

    // The group.

    fn gstmt(&self, g: &mut [Sg], n: usize, masks: &[u32]) -> R<()> {
        if self.v.gsync(n) {
            return self.gexec(g, n, masks);
        }
        for (s, sg) in g.iter_mut().enumerate() {
            if masks[s] != 0 {
                self.vexec(sg, n, masks[s])?;
            }
        }
        Ok(())
    }

    fn gchild(&self, g: &mut [Sg], n: usize, masks: &[u32]) -> R<Vec<Vals>> {
        if self.v.gsync(n) {
            return self.gval(g, n, masks);
        }
        let mut out = vec![[0u64; W]; g.len()];
        for (s, sg) in g.iter_mut().enumerate() {
            if masks[s] != 0 {
                out[s] = self.vval(sg, n, masks[s])?;
            }
        }
        Ok(out)
    }

    /// A branch's decision over the group: each SIMD group's lanes it takes.
    fn gdecide(&self, g: &[Sg], n: usize, masks: &[u32], c: &[Vals]) -> R<Vec<u32>> {
        let mut yes = vec![0u32; g.len()];
        for (s, sg) in g.iter().enumerate() {
            yes[s] = self.decide(sg, n, masks[s], &c[s])?;
        }
        Ok(yes)
    }

    fn normals(g: &[Sg], masks: &[u32]) -> Vec<u32> {
        g.iter().zip(masks).map(|(sg, &m)| m & sg.normal()).collect()
    }

    /// Statement `n`, which has a threadgroup barrier in it, over the group's lanes `masks`.
    fn gexec(&self, g: &mut [Sg], n: usize, masks: &[u32]) -> R<()> {
        let v = &self.v;
        let p = v.p(n);
        match v.kind(n) {
            Kind::Block => {
                let mut live = masks.to_vec();
                for c in v.children(n) {
                    if live.iter().all(|&m| m == 0) {
                        break;
                    }
                    self.gstmt(g, c, &live)?;
                    live = Self::normals(g, &live);
                }
            }
            Kind::Expr => {
                self.gval(g, v.child(n, 0), masks)?;
            }
            Kind::If => {
                let c = self.gchild(g, v.child(n, 0), masks)?;
                let yes = self.gdecide(g, n, masks, &c)?;
                if yes.iter().any(|&m| m != 0) {
                    self.gstmt(g, v.child(n, 1), &yes)?;
                }
                let no: Vec<u32> = masks.iter().zip(&yes).map(|(m, y)| m & !y).collect();
                if no.iter().any(|&m| m != 0) && v.children(n).count() > 2 {
                    self.gstmt(g, v.child(n, 2), &no)?;
                }
            }
            Kind::While | Kind::For | Kind::Do => {
                let k = v.kind(n);
                let (cond, body) = if k == Kind::Do { (v.child(n, 1), v.child(n, 0)) } else { (v.child(n, 0), v.child(n, 1)) };
                let mut lp = masks.to_vec();
                let mut first = k == Kind::Do;
                loop {
                    if !first {
                        let c = self.gchild(g, cond, &lp)?;
                        lp = self.gdecide(g, n, &lp, &c)?;
                    }
                    first = false;
                    if lp.iter().all(|&m| m == 0) {
                        break;
                    }
                    self.gstmt(g, body, &lp)?;
                    for (s, sg) in g.iter_mut().enumerate() {
                        lp[s] = Self::rejoin(sg, lp[s]);
                    }
                    if k == Kind::For && lp.iter().any(|&m| m != 0) {
                        self.gstmt(g, v.child(n, 2), &lp)?;
                    }
                }
            }
            Kind::Return => {
                let x = if p[0] != 0 { self.gchild(g, v.child(n, 0), masks)? } else { vec![[0; W]; g.len()] };
                for (s, sg) in g.iter_mut().enumerate() {
                    self.ret(sg, p, masks[s], &x[s]);
                }
            }
            _ => {
                self.gval(g, n, masks)?;
            }
        }
        Ok(())
    }

    /// Expression `n`, which has a threadgroup barrier in it, over the group's lanes `masks`.
    fn gval(&self, g: &mut [Sg], n: usize, masks: &[u32]) -> R<Vec<Vals>> {
        let v = &self.v;
        let p = v.p(n);
        let mut out = vec![[0u64; W]; g.len()];
        match v.kind(n) {
            Kind::Inline => {
                let mut live = masks.to_vec();
                for c in v.children(n) {
                    if live.iter().all(|&m| m == 0) {
                        break;
                    }
                    self.gstmt(g, c, &live)?;
                    live = Self::normals(g, &live);
                }
                for (s, sg) in g.iter_mut().enumerate() {
                    for l in lanes(masks[s]) {
                        if sg.flow[l] == Flow::Return {
                            sg.flow[l] = Flow::Normal;
                        }
                    }
                    out[s] = self.result(sg, n, masks[s] & !sg.ended);
                }
            }
            Kind::Block | Kind::Expr => self.gexec(g, n, masks)?,
            Kind::Cond | Kind::Logic => {
                let cond = v.kind(n) == Kind::Cond;
                let c = self.gchild(g, v.child(n, 0), masks)?;
                // The lanes that take the second operand: a `?:`'s true ones, an `&&`'s true, an `||`'s false.
                let take = cond || p[0] != 0;
                let first: Vec<u32> = c.iter().zip(masks).map(|(c, &m)| lanes(m).filter(|&l| (c[l] != 0) == take).fold(0u32, |x, l| x | 1 << l)).collect();
                let a = self.gchild(g, v.child(n, 1), &first)?;
                let rest: Vec<u32> = masks.iter().zip(&first).map(|(m, f)| m & !f).collect();
                let b = if cond { self.gchild(g, v.child(n, 2), &rest)? } else { c.clone() };
                for s in 0..g.len() {
                    for l in lanes(masks[s]) {
                        let took = first[s] >> l & 1 != 0;
                        out[s][l] = if cond {
                            if took { a[s][l] } else { b[s][l] }
                        } else {
                            u64::from(if took { a[s][l] != 0 } else { c[s][l] != 0 })
                        };
                    }
                }
            }
            Kind::Builtin if Builtin::of(p[0]) == Builtin::Barrier => {
                // The barrier: every SIMD group's region before it has run (the rule proven by the vector pass).
                for c in v.children(n) {
                    self.gchild(g, c, masks)?;
                }
            }
            _ => {
                let mut xs = Vec::with_capacity(3);
                for c in v.children(n) {
                    xs.push(self.gchild(g, c, masks)?);
                }
                let none = [0u64; W];
                for (s, sg) in g.iter().enumerate() {
                    if masks[s] != 0 {
                        let x = [0, 1, 2].map(|j| xs.get(j).map_or(&none, |x: &Vec<Vals>| &x[s]));
                        self.vop(sg, n, masks[s], x, &mut out[s])?;
                    }
                }
            }
        }
        Ok(out)
    }
}

/// The bits of `m`'s width.
fn mask(m: M) -> u64 {
    if m.bits() >= 64 { u64::MAX } else { (1u64 << m.bits()) - 1 }
}
