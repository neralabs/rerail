//! The kernel tape run by an interpreter at one lane (decision 0567, step 1): each group's threads in turn through
//! a node with no sync point in it (the cut's), and all of the group's at once through one that has, its threads
//! a mask. The kernel tape is read in place.
//!
//! The barrier rule (0567 §3): a sync point is met by every thread of its group (a SIMD-group operation, of its SIMD
//! group) that has not ended. A thread that is not at it is let off only if it then ends the kernel without
//! reaching another sync point; else the kernel fails, naming both.

use crate::Error;
use crate::k::{Builtin, Kind, M, Op, View};
use crate::lower::Program;
use crate::ops;

/// The SIMD group's lanes.
pub const WIDTH: u32 = 32;

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

/// A thread: its frame and stack, its ids, where its control is.
#[derive(Clone, Debug)]
struct Th {
    fp: u64,
    sp: u64,
    ret: u64,
    flow: Flow,
    lid: [u32; 3],
    lin: u32,
    /// It ended the kernel.
    ended: bool,
    /// It was not at a sync point its group met: it may only end.
    owed: Option<u32>,
}

struct G {
    th: Vec<Th>,
    /// Group-wide calls entered: 0 in the kernel's own body.
    depth: u32,
}

type R<T> = Result<T, Error>;

/// One dispatch's state the threads share.
struct Run<'p> {
    p: &'p Program,
    v: View<'p>,
    data: u64,
    shared: u64,
    grid: [u32; 3],
    block: [u32; 3],
    group: [u32; 3],
}

/// Runs kernel `kernel` of `p` over `grid` groups of `block` threads, with `args` for its parameters. `local` is
/// the threadgroup memory a group has past the kernel's own (its `Arg::Local` arguments').
pub fn run(p: &Program, kernel: u32, args: &[Arg], grid: [u32; 3], block: [u32; 3], local: u64) -> R<()> {
    let f = &p.funcs[kernel as usize];
    let node = f.node.ok_or_else(|| Error(format!("{} has no body", f.name)))?;
    let v = View::new(&p.tape);
    if args.len() != f.params.len() {
        return Err(Error(format!("{} takes {} arguments, given {}", f.name, f.params.len(), args.len())));
    }
    let threads = block.iter().map(|&b| b as usize).product::<usize>();
    let stack = (p.stacks[kernel as usize] as usize).next_multiple_of(16).max(16);
    let mut stacks = vec![0u128; threads * stack / 16];
    let own = u64::from(v.w(node, 2)).next_multiple_of(16);
    let mut shared = vec![0u128; (own + local).div_ceil(16) as usize + 1];
    let data_node = v.children(0).last().expect("the program's data");
    let data = v.ws(data_node, 1, 0).as_ptr() as u64;
    let mut r = Run { p, v, data, shared: shared.as_mut_ptr() as u64, grid, block, group: [0; 3] };
    let body = r.v.child(node, 0);
    for gz in 0..grid[2] {
        for gy in 0..grid[1] {
            for gx in 0..grid[0] {
                r.group = [gx, gy, gz];
                shared.fill(0);
                stacks.fill(0);
                let base = stacks.as_mut_ptr() as u64;
                let mut g = G { th: Vec::with_capacity(threads), depth: 0 };
                for t in 0..threads as u32 {
                    let (bx, by) = (block[0], block[1]);
                    let fp = base + u64::from(t) * stack as u64;
                    let th = Th { fp, sp: fp + u64::from(r.v.w(node, 1)), ret: 0, flow: Flow::Normal, lid: [t % bx, t / bx % by, t / (bx * by)], lin: t, ended: false, owed: None };
                    for (k, a) in args.iter().enumerate() {
                        let (off, m) = (u64::from(r.v.w(node, 5 + 2 * k)), M(r.v.w(node, 6 + 2 * k)));
                        let at = (fp + off) as usize as *mut u8;
                        match a {
                            // SAFETY: the parameter's place in the thread's frame, its type's bytes.
                            Arg::Bytes(b) => unsafe { std::ptr::copy_nonoverlapping(b.as_ptr(), at, b.len().min(m.bytes() as usize)) },
                            Arg::Local(o) => unsafe { ops::store(M::PTR, at as u64, r.shared + own + o) },
                        }
                    }
                    g.th.push(th);
                }
                if r.v.syncs(body) {
                    let all: Vec<u32> = (0..threads as u32).collect();
                    r.gexec(&mut g, body, &all)?;
                } else {
                    for t in &mut g.th {
                        r.exec(t, body)?;
                    }
                }
            }
        }
    }
    Ok(())
}

impl Run<'_> {
    fn name(&self, site: u32) -> &str {
        self.p.sites.get(site as usize).map_or("?", String::as_str)
    }

    fn konst(&self, n: usize) -> u64 {
        let p = self.v.p(n);
        u64::from(p[1]) | u64::from(p[2]) << 32
    }

    fn func(&self, id: u32) -> R<usize> {
        self.p.funcs[id as usize].node.ok_or_else(|| Error(format!("{} is called and has no body", self.p.funcs[id as usize].name)))
    }

    // One thread.

    /// Node `n`'s value in thread `t`.
    fn val(&self, t: &mut Th, n: usize) -> R<u64> {
        let v = &self.v;
        let p = v.p(n);
        Ok(match v.kind(n) {
            Kind::Const => self.konst(n),
            Kind::Local => t.fp + u64::from(p[0]),
            Kind::Shared => self.shared + u64::from(p[0]),
            Kind::Global => self.data + u64::from(p[0]),
            Kind::Load => {
                let a = self.val(t, v.child(n, 0))?;
                // SAFETY: the kernel's address (its buffers', frames' and threadgroup memory's are the dispatch's).
                unsafe { ops::load(M(p[0]), a) }
            }
            Kind::Store => {
                let a = self.val(t, v.child(n, 0))?;
                let x = self.val(t, v.child(n, 1))?;
                unsafe { ops::store(M(p[0]), a, x) };
                x
            }
            Kind::Zero => {
                let a = self.val(t, v.child(n, 0))?;
                unsafe { std::ptr::write_bytes(a as usize as *mut u8, 0, p[1] as usize) };
                a
            }
            Kind::Copy => {
                let d = self.val(t, v.child(n, 0))?;
                let s = self.val(t, v.child(n, 1))?;
                unsafe { std::ptr::copy(s as usize as *const u8, d as usize as *mut u8, p[0] as usize) };
                d
            }
            Kind::Bin => {
                let a = self.val(t, v.child(n, 0))?;
                let b = self.val(t, v.child(n, 1))?;
                ops::bin(Op::of(p[0]), M(p[1]), a, b).map_err(|e| Error(e.to_string()))?
            }
            Kind::Un => {
                let a = self.val(t, v.child(n, 0))?;
                ops::un(Op::of(p[0]), M(p[1]), a)
            }
            Kind::Cast => {
                let a = self.val(t, v.child(n, 0))?;
                ops::cast(M(p[0]), M(p[1]), a)
            }
            Kind::Cond => {
                let c = self.val(t, v.child(n, 0))?;
                self.val(t, v.child(n, if c != 0 { 1 } else { 2 }))?
            }
            Kind::Logic => {
                let and = p[0] != 0;
                let a = self.val(t, v.child(n, 0))? != 0;
                if a != and { u64::from(a) } else { u64::from(self.val(t, v.child(n, 1))? != 0) }
            }
            Kind::Comma => {
                self.val(t, v.child(n, 0))?;
                self.val(t, v.child(n, 1))?
            }
            Kind::Update => self.update(t, n)?,
            Kind::Call => {
                let mut args = Vec::with_capacity(4);
                for c in v.children(n) {
                    args.push(self.val(t, c)?);
                }
                self.call(t, n, &args)?
            }
            Kind::Builtin => {
                let b = Builtin::of(p[0]);
                if b.syncs() {
                    return Err(Error(format!("{} run by one thread: the cut missed it", self.name(p[3]))));
                }
                let mut a = [0u64; 3];
                for (k, c) in v.children(n).enumerate() {
                    a[k] = self.val(t, c)?;
                }
                self.builtin(t, n, b, a)?
            }
            Kind::Block | Kind::Expr => {
                self.exec(t, n)?;
                0
            }
            k => return Err(Error(format!("a {k:?} node as a value"))),
        })
    }

    /// An assignment through an operation: `[op, lhs type, op type, post]`.
    fn update(&self, t: &mut Th, n: usize) -> R<u64> {
        let v = &self.v;
        let a = self.val(t, v.child(n, 0))?;
        let x = self.val(t, v.child(n, 1))?;
        Ok(self.update_at(n, a, x))
    }

    fn update_at(&self, n: usize, a: u64, x: u64) -> u64 {
        let v = &self.v;
        let p = v.p(n);
        let (op, lm, om, post) = (Op::of(p[0]), M(p[1]), M(p[2]), p[3] != 0);
        // SAFETY: the kernel's address.
        let old = unsafe { ops::load(lm, a) };
        let r = ops::bin(op, om, ops::cast(lm, om, old), x).unwrap_or(0);
        let new = ops::cast(om, lm, r);
        unsafe { ops::store(lm, a, new) };
        if post { old } else { new }
    }

    /// A call's frame: pushed, its parameters written from `args`; the callee's node.
    fn enter(&self, t: &mut Th, n: usize, args: &[u64]) -> R<(usize, u64, u64)> {
        let v = &self.v;
        let p = v.p(n);
        let f = self.func(p[0])?;
        let (saved_fp, saved_sp) = (t.fp, t.sp);
        let fp = t.sp.next_multiple_of(16);
        let frame = u64::from(v.w(f, 1));
        unsafe { std::ptr::write_bytes(fp as usize as *mut u8, 0, frame as usize) };
        for (k, &a) in args.iter().enumerate() {
            let (off, m) = (u64::from(v.w(f, 5 + 2 * k)), M(v.w(f, 6 + 2 * k)));
            if m.is_agg() {
                unsafe { std::ptr::copy_nonoverlapping(a as usize as *const u8, (fp + off) as usize as *mut u8, m.bytes() as usize) };
            } else {
                unsafe { ops::store(m, fp + off, a) };
            }
        }
        (t.fp, t.sp) = (fp, fp + frame);
        t.ret = 0;
        Ok((f, saved_fp, saved_sp))
    }

    /// A call's frame popped: its result (an aggregate's copied to the caller's frame).
    fn leave(&self, t: &mut Th, n: usize, saved: (u64, u64)) -> u64 {
        let p = self.v.p(n);
        (t.fp, t.sp) = saved;
        let m = M(p[1]);
        if m.is_agg() {
            let to = t.fp + u64::from(p[2]);
            unsafe { std::ptr::copy(t.ret as usize as *const u8, to as usize as *mut u8, m.bytes() as usize) };
            to
        } else {
            t.ret
        }
    }

    fn call(&self, t: &mut Th, n: usize, args: &[u64]) -> R<u64> {
        let (f, fp, sp) = self.enter(t, n, args)?;
        self.exec(t, self.v.child(f, 0))?;
        Ok(self.leave(t, n, (fp, sp)))
    }

    /// A builtin that does not sync, in thread `t`.
    fn builtin(&self, t: &Th, n: usize, b: Builtin, a: [u64; 3]) -> R<u64> {
        let v = &self.v;
        let p = v.p(n);
        let m = M(p[1]);
        let d = a[0] as usize;
        let dim = |x: [u32; 3], or: u32| if d < 3 { x[d] } else { or };
        Ok(match b {
            Builtin::LocalId => u64::from(dim(t.lid, 0)),
            Builtin::LocalSize => u64::from(dim(self.block, 1)),
            Builtin::GroupId => u64::from(dim(self.group, 0)),
            Builtin::NumGroups => u64::from(dim(self.grid, 1)),
            Builtin::GlobalId => u64::from(dim(self.group, 0)) * u64::from(dim(self.block, 1)) + u64::from(dim(t.lid, 0)),
            Builtin::GlobalSize => u64::from(dim(self.grid, 1)) * u64::from(dim(self.block, 1)),
            Builtin::SubGroupLocalId => u64::from(t.lin % WIDTH),
            Builtin::MemFence => 0,
            Builtin::AtomicAdd
            | Builtin::AtomicSub
            | Builtin::AtomicAnd
            | Builtin::AtomicOr
            | Builtin::AtomicXor
            | Builtin::AtomicXchg
            | Builtin::AtomicMin
            | Builtin::AtomicMax
            | Builtin::AtomicCmpxchg
            | Builtin::AtomicInc
            | Builtin::AtomicDec => {
                // One thread runs at a time: an atomic is its read and write.
                let p = a[0];
                let old = unsafe { ops::load(m, p) };
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
                unsafe { ops::store(m, p, new) };
                old
            }
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
            Builtin::As => {
                let from = M(p[4]);
                ops::norm(m, a[0] & mask(from))
            }
            Builtin::Min | Builtin::Max => {
                let less = ops::bin(Op::Lt, m, a[0], a[1]).unwrap_or(0) != 0;
                if less == (b == Builtin::Min) { a[0] } else { a[1] }
            }
            _ => return Err(Error(format!("{} run by one thread", self.name(p[3])))),
        })
    }

    /// Statement `n` in thread `t`.
    fn exec(&self, t: &mut Th, n: usize) -> R<Flow> {
        let v = &self.v;
        let p = v.p(n);
        Ok(match v.kind(n) {
            Kind::Block => {
                for c in v.children(n) {
                    let f = self.exec(t, c)?;
                    if f != Flow::Normal {
                        return Ok(f);
                    }
                }
                Flow::Normal
            }
            Kind::Expr => {
                self.val(t, v.child(n, 0))?;
                Flow::Normal
            }
            Kind::Empty => Flow::Normal,
            Kind::If => {
                if self.val(t, v.child(n, 0))? != 0 {
                    self.exec(t, v.child(n, 1))?
                } else if v.children(n).count() > 2 {
                    self.exec(t, v.child(n, 2))?
                } else {
                    Flow::Normal
                }
            }
            Kind::While | Kind::For | Kind::Do => {
                let k = v.kind(n);
                let (cond, body) = if k == Kind::Do { (v.child(n, 1), v.child(n, 0)) } else { (v.child(n, 0), v.child(n, 1)) };
                let mut first = k == Kind::Do;
                loop {
                    if !first && self.val(t, cond)? == 0 {
                        break;
                    }
                    first = false;
                    match self.exec(t, body)? {
                        Flow::Break => break,
                        Flow::Return => return Ok(Flow::Return),
                        _ => {}
                    }
                    if k == Kind::For {
                        self.exec(t, v.child(n, 2))?;
                    }
                }
                Flow::Normal
            }
            Kind::Return => {
                if p[0] != 0 {
                    t.ret = self.val(t, v.child(n, 0))?;
                }
                Flow::Return
            }
            Kind::Break => Flow::Break,
            Kind::Continue => Flow::Continue,
            _ => {
                self.val(t, n)?;
                Flow::Normal
            }
        })
    }

    // The group.

    /// Statement `n` for the threads `act`, whatever it holds: each thread's control in its `flow`.
    fn gstmt(&self, g: &mut G, n: usize, act: &[u32]) -> R<()> {
        if self.v.syncs(n) {
            return self.gexec(g, n, act);
        }
        for &t in act {
            let th = &mut g.th[t as usize];
            let f = self.exec(th, n)?;
            th.flow = f;
            if f == Flow::Return && g.depth == 0 {
                th.ended = true;
            }
        }
        Ok(())
    }

    /// Node `n`'s values for the threads `act` (by thread; the others' 0).
    fn gchild(&self, g: &mut G, n: usize, act: &[u32]) -> R<Vec<u64>> {
        if self.v.syncs(n) {
            return self.gval(g, n, act);
        }
        let mut out = vec![0u64; g.th.len()];
        for &t in act {
            out[t as usize] = self.val(&mut g.th[t as usize], n)?;
        }
        Ok(out)
    }

    fn gexec(&self, g: &mut G, n: usize, act: &[u32]) -> R<()> {
        let v = &self.v;
        let p = v.p(n);
        match v.kind(n) {
            Kind::Block => {
                let mut live = act.to_vec();
                for c in v.children(n) {
                    if live.is_empty() {
                        break;
                    }
                    self.gstmt(g, c, &live)?;
                    live.retain(|&t| g.th[t as usize].flow == Flow::Normal);
                }
            }
            Kind::Expr => {
                self.gval(g, v.child(n, 0), act)?;
            }
            Kind::If => {
                let c = self.gchild(g, v.child(n, 0), act)?;
                let (yes, no): (Vec<u32>, Vec<u32>) = act.iter().partition(|&&t| c[t as usize] != 0);
                if !yes.is_empty() {
                    self.gstmt(g, v.child(n, 1), &yes)?;
                }
                if !no.is_empty() && v.children(n).count() > 2 {
                    self.gstmt(g, v.child(n, 2), &no)?;
                }
            }
            Kind::While | Kind::For | Kind::Do => {
                let k = v.kind(n);
                let (cond, body) = if k == Kind::Do { (v.child(n, 1), v.child(n, 0)) } else { (v.child(n, 0), v.child(n, 1)) };
                let mut lp = act.to_vec();
                let mut first = k == Kind::Do;
                loop {
                    if !first {
                        let c = self.gchild(g, cond, &lp)?;
                        lp.retain(|&t| c[t as usize] != 0);
                    }
                    first = false;
                    if lp.is_empty() {
                        break;
                    }
                    self.gstmt(g, body, &lp)?;
                    lp.retain(|&t| {
                        let th = &mut g.th[t as usize];
                        match th.flow {
                            Flow::Break => {
                                th.flow = Flow::Normal;
                                false
                            }
                            Flow::Continue => {
                                th.flow = Flow::Normal;
                                true
                            }
                            Flow::Return => false,
                            Flow::Normal => true,
                        }
                    });
                    if k == Kind::For && !lp.is_empty() {
                        self.gstmt(g, v.child(n, 2), &lp)?;
                    }
                }
            }
            Kind::Return => {
                let x = if p[0] != 0 { self.gchild(g, v.child(n, 0), act)? } else { vec![0; g.th.len()] };
                for &t in act {
                    let th = &mut g.th[t as usize];
                    th.ret = x[t as usize];
                    th.flow = Flow::Return;
                    th.ended |= g.depth == 0;
                }
            }
            _ => {
                self.gval(g, n, act)?;
            }
        }
        Ok(())
    }

    fn gval(&self, g: &mut G, n: usize, act: &[u32]) -> R<Vec<u64>> {
        let v = &self.v;
        let p = v.p(n);
        let k = v.kind(n);
        match k {
            Kind::Cond => {
                let c = self.gchild(g, v.child(n, 0), act)?;
                let (yes, no): (Vec<u32>, Vec<u32>) = act.iter().partition(|&&t| c[t as usize] != 0);
                let a = if yes.is_empty() { vec![0; g.th.len()] } else { self.gchild(g, v.child(n, 1), &yes)? };
                let mut b = if no.is_empty() { vec![0; g.th.len()] } else { self.gchild(g, v.child(n, 2), &no)? };
                for &t in &yes {
                    b[t as usize] = a[t as usize];
                }
                return Ok(b);
            }
            Kind::Logic => {
                let and = p[0] != 0;
                let mut a = self.gchild(g, v.child(n, 0), act)?;
                let rest: Vec<u32> = act.iter().copied().filter(|&t| (a[t as usize] != 0) == and).collect();
                if !rest.is_empty() {
                    let b = self.gchild(g, v.child(n, 1), &rest)?;
                    for &t in &rest {
                        a[t as usize] = u64::from(b[t as usize] != 0);
                    }
                }
                return Ok(a);
            }
            Kind::Block | Kind::Expr => {
                self.gexec(g, n, act)?;
                return Ok(vec![0; g.th.len()]);
            }
            _ => {}
        }
        let mut xs: Vec<Vec<u64>> = Vec::with_capacity(3);
        for c in v.children(n) {
            xs.push(self.gchild(g, c, act)?);
        }
        let mut out = vec![0u64; g.th.len()];
        match k {
            Kind::Call => {
                let mut saved = Vec::with_capacity(act.len());
                let mut f = 0;
                for &t in act {
                    let args: Vec<u64> = xs.iter().map(|x| x[t as usize]).collect();
                    let (callee, fp, sp) = self.enter(&mut g.th[t as usize], n, &args)?;
                    f = callee;
                    saved.push((fp, sp));
                }
                g.depth += 1;
                let r = self.gexec(g, v.child(f, 0), act);
                g.depth -= 1;
                r?;
                for (&t, s) in act.iter().zip(saved) {
                    let th = &mut g.th[t as usize];
                    th.flow = Flow::Normal;
                    out[t as usize] = self.leave(th, n, s);
                }
            }
            Kind::Builtin => {
                let b = Builtin::of(p[0]);
                if b.syncs() {
                    self.meet(g, n, b, act, &xs, &mut out)?;
                } else {
                    for &t in act {
                        let a = [0, 1, 2].map(|i| xs.get(i).map_or(0, |x| x[t as usize]));
                        out[t as usize] = self.builtin(&g.th[t as usize], n, b, a)?;
                    }
                }
            }
            _ => {
                for &t in act {
                    let x = |i: usize| xs[i][t as usize];
                    out[t as usize] = match k {
                        Kind::Load => unsafe { ops::load(M(p[0]), x(0)) },
                        Kind::Store => {
                            unsafe { ops::store(M(p[0]), x(0), x(1)) };
                            x(1)
                        }
                        Kind::Zero => {
                            unsafe { std::ptr::write_bytes(x(0) as usize as *mut u8, 0, p[1] as usize) };
                            x(0)
                        }
                        Kind::Copy => {
                            unsafe { std::ptr::copy(x(1) as usize as *const u8, x(0) as usize as *mut u8, p[0] as usize) };
                            x(0)
                        }
                        Kind::Bin => ops::bin(Op::of(p[0]), M(p[1]), x(0), x(1)).map_err(|e| Error(e.to_string()))?,
                        Kind::Un => ops::un(Op::of(p[0]), M(p[1]), x(0)),
                        Kind::Cast => ops::cast(M(p[0]), M(p[1]), x(0)),
                        Kind::Comma => x(1),
                        Kind::Update => self.update_at(n, x(0), x(1)),
                        k => return Err(Error(format!("a {k:?} node that syncs"))),
                    };
                }
            }
        }
        Ok(out)
    }

    /// A sync point met by the threads `act`: the barrier rule checked, and the operation's values.
    fn meet(&self, g: &mut G, n: usize, b: Builtin, act: &[u32], xs: &[Vec<u64>], out: &mut [u64]) -> R<()> {
        let p = self.v.p(n);
        let site = p[3];
        let mut at = vec![false; g.th.len()];
        for &t in act {
            at[t as usize] = true;
            if let Some(s) = g.th[t as usize].owed {
                return Err(Error(format!("thread {t} was not at {} (its group's threads met there without it), and then reached {}", self.name(s), self.name(site))));
            }
        }
        // Who must be here: the group's threads, or the SIMD groups' lanes, that have not ended.
        let whole = b == Builtin::Barrier;
        let sgs: std::collections::BTreeSet<u32> = act.iter().map(|&t| t / WIDTH).collect();
        for (t, th) in g.th.iter_mut().enumerate() {
            let due = whole || sgs.contains(&(t as u32 / WIDTH));
            if due && !th.ended && !at[t] {
                th.owed = Some(site);
            }
        }
        let lane_of = |t: u32| t % WIDTH;
        let n_threads = g.th.len() as u32;
        let src = |t: u32, lane: u32| -> Option<u32> {
            let s = t / WIDTH * WIDTH + lane;
            (s < n_threads && at[s as usize]).then_some(s)
        };
        for &t in act {
            let x = |i: usize| xs.get(i).map_or(0, |x| x[t as usize]);
            let own = x(0);
            out[t as usize] = match b {
                Builtin::Barrier | Builtin::SubGroupBarrier => 0,
                Builtin::SubGroupBallot => {
                    let base = t / WIDTH * WIDTH;
                    let mut m = 0u64;
                    for l in 0..WIDTH {
                        let s = base + l;
                        if s < n_threads && at[s as usize] && xs[0][s as usize] != 0 {
                            m |= 1 << l;
                        }
                    }
                    m
                }
                Builtin::SubGroupShuffle => src(t, (x(1) as u32) % WIDTH).map_or(0, |s| xs[0][s as usize]),
                Builtin::SubGroupShuffleXor => src(t, (lane_of(t) ^ x(1) as u32) % WIDTH).map_or(0, |s| xs[0][s as usize]),
                Builtin::SubGroupShuffleUp => lane_of(t).checked_sub(x(1) as u32).and_then(|l| src(t, l)).map_or(own, |s| xs[0][s as usize]),
                Builtin::SubGroupShuffleDown => {
                    let l = lane_of(t) + x(1) as u32;
                    if l < WIDTH { src(t, l).map_or(own, |s| xs[0][s as usize]) } else { own }
                }
                _ => unreachable!("a builtin that syncs"),
            };
        }
        Ok(())
    }
}

/// The bits of `m`'s width.
fn mask(m: M) -> u64 {
    if m.bits() >= 64 { u64::MAX } else { (1u64 << m.bits()) - 1 }
}
