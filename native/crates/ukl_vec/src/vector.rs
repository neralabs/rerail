//! A kernel's vector tape (decision 0567 §2.2–2.5): its kernel tape with every call inlined, each node marked with its
//! value's uniformity, the cut at threadgroup barriers, and the barrier rule proven, or the kernel refused with its
//! place.
//!
//! - **Inlined:** a call is an `Inline` node over its parameters' stores and its callee's body, whose `Return`s store
//!   the result in the call's slot. The callee's frame follows its caller's, siblings' frames shared. Each variable
//!   of each inlined call has a number of its own (its `Local`s' fourth word): what uniformity is tracked by.
//! - **Uniformity** (`Level`), a fixed point over the kernel: a value is uniform from constants, sizes, group ids and
//!   loads at uniform addresses (the kernel's memory is race-free); the same in a SIMD group from a ballot or a
//!   broadcast; agreed from an atomic that leaves its word as it is at a uniform address; varying from the thread's
//!   ids, the other atomics, and the lanes. A variable's is the most varying of its stores' values and of the control
//!   they are under. Control is under a branch's condition, and under the exits (`return`, `break`, `continue`)
//!   some of its lanes took before, until they rejoin.
//! - **The barrier rule** (§3): a threadgroup barrier must be under control agreed across the group, a SIMD-group
//!   operation under control uniform in its SIMD group. The lanes a more varying branch or exit leaves out must end
//!   the kernel without reaching another sync point (decision 0567 §3), or the kernel is refused, naming the sync
//!   point and the branch. A branch on an agreed value that the rule rests on is checked as it runs (`CHECK`).
//!
//! A variable is a lane's own: its frame holds each variable once a lane (struct of arrays, `run`).

use std::collections::HashMap;

use crate::Error;
use crate::k::{Builder, Builtin, CHECK, EFFECT, GSYNC, Kind, Level, M, NO_SLOT, Op, SYNC, Tape, View};
use crate::lower::Program;

/// A kernel's vector tape: its root a `Func` (`[id, frame, shared, return type, parameters, (offset, type)...]`), its
/// frame a lane's bytes with every call inlined.
pub struct Kernel {
    pub tape: Tape,
    pub name: String,
}

type R<T> = Result<T, Error>;

/// The vector tape of kernel `kernel` of `p`.
pub fn vectorize(p: &Program, kernel: u32) -> R<Kernel> {
    let f = &p.funcs[kernel as usize];
    let node = f.node.ok_or_else(|| Error(format!("{} has no body", f.name)))?;
    let kv = View::new(&p.tape);
    let frame = kv.w(node, 1).next_multiple_of(16);
    let mut i = Inliner { p, kv: &kv, b: Builder::default(), top: frame, instances: 0 };
    let own = i.own(node).to_vec();
    let root = i.b.open(Kind::Func, &own);
    i.emit(kv.child(node, 0), Frame { base: 0, next: frame, slot: None, instance: 0 })?;
    i.b.close(root);
    let top = i.top;
    i.b.set_word(root, 1, top);
    let mut b = i.b;
    let mut a = Analysis::new(&b);
    a.run(&b);
    let fields = a.check(&b, p, &f.name)?;
    for (n, field) in fields.into_iter().enumerate() {
        b.set_field(n, field);
    }
    Ok(Kernel { tape: b.finish(), name: f.name.clone() })
}

/// Where an inlined body's frame is: its variables' base, its callees' base, its result's slot (offset, type,
/// variable) and its instance (its variables' numbers).
#[derive(Clone, Copy)]
struct Frame {
    base: u32,
    next: u32,
    slot: Option<(u32, M, u32)>,
    instance: u32,
}

struct Inliner<'a, 'p> {
    p: &'p Program,
    kv: &'a View<'p>,
    b: Builder,
    top: u32,
    instances: u32,
}

/// An inlined call's result's variable number in its instance.
const RESULT: u32 = (1 << 20) - 1;

/// A variable's number: its instance's, and its number in its function (`Kind::Local`).
fn var(instance: u32, n: u32) -> u32 {
    instance << 20 | n
}

impl<'p> Inliner<'_, 'p> {
    /// Node `n`'s own payload words (before its first child's).
    fn own(&self, n: usize) -> &'p [u32] {
        let v = &self.kv.v;
        let s = v.start(n) as usize;
        let e = if v.size(n) > 1 { v.start(n + 1) } else { v.end(n) } as usize;
        &self.kv.words[s..e]
    }

    fn emit(&mut self, n: usize, f: Frame) -> R<()> {
        let kv = self.kv;
        match kv.kind(n) {
            Kind::Local => {
                let (off, size, inner) = (kv.w(n, 0), kv.w(n, 1), kv.w(n, 2));
                self.b.leaf(Kind::Local, &[f.base + off, size, inner, var(f.instance, kv.w(n, 3))]);
            }
            Kind::Return => {
                let (has, site) = (kv.w(n, 0), kv.w(n, 1));
                let (slot, m, v) = f.slot.unwrap_or((NO_SLOT, M::VOID, 0));
                let r = self.b.open(Kind::Return, &[has, site, slot, m.0, v]);
                if has != 0 {
                    self.emit(kv.child(n, 0), f)?;
                }
                self.b.close(r);
            }
            Kind::Call => {
                let callee = self.p.funcs[kv.w(n, 0) as usize].node.ok_or_else(|| Error(format!("{} is called and has no body", self.p.funcs[kv.w(n, 0) as usize].name)))?;
                let m = M(kv.w(n, 1));
                let frame = kv.w(callee, 1).next_multiple_of(16);
                let base = f.next;
                let slot = base + frame;
                let next = slot + m.bytes().max(8).next_multiple_of(16);
                self.top = self.top.max(next);
                self.instances += 1;
                let instance = self.instances;
                let ret = var(instance, RESULT);
                let temp = f.base + kv.w(n, 2);
                let node = self.b.open(Kind::Inline, &[m.0, slot, temp, ret]);
                // The parameters' stores, each a statement of the call's.
                for (k, a) in kv.children(n).enumerate() {
                    let (off, pm) = (kv.w(callee, 5 + 2 * k), M(kv.w(callee, 6 + 2 * k)));
                    let e = self.b.open(Kind::Expr, &[]);
                    let s = if pm.is_agg() { self.b.open(Kind::Copy, &[pm.bytes()]) } else { self.b.open(Kind::Store, &[pm.0]) };
                    self.b.leaf(Kind::Local, &[base + off, pm.bytes(), 0, var(instance, k as u32)]);
                    // A call in an argument: its frame past this one's, whose parameters are stored as it goes.
                    self.emit(a, Frame { next, ..f })?;
                    self.b.close(s);
                    self.b.close(e);
                }
                self.emit(kv.child(callee, 0), Frame { base, next, slot: Some((slot, m, ret)), instance })?;
                self.b.close(node);
            }
            k => {
                let own = self.own(n).to_vec();
                let node = self.b.open(k, &own);
                for c in kv.children(n) {
                    self.emit(c, f)?;
                }
                self.b.close(node);
            }
        }
        Ok(())
    }
}

fn kind(b: &Builder, n: usize) -> Kind {
    Kind::of((b.key(n) & 0x3ff) as u16)
}

/// Node `n`'s value if it is a constant (folded through casts and arithmetic).
fn constant(b: &Builder, n: usize) -> Option<u64> {
    let k = kids(b, n);
    match kind(b, n) {
        Kind::Const => Some(u64::from(b.word(n, 1)) | u64::from(b.word(n, 2)) << 32),
        Kind::Cast => constant(b, k[0]),
        Kind::Bin => {
            let (x, y) = (constant(b, k[0])?, constant(b, k[1])?);
            match Op::of(b.word(n, 0)) {
                Op::Add => Some(x.wrapping_add(y)),
                Op::Sub => Some(x.wrapping_sub(y)),
                Op::Mul => Some(x.wrapping_mul(y)),
                Op::Shl => Some(x << (y & 63)),
                _ => None,
            }
        }
        _ => None,
    }
}

/// The variable address `a` is in directly: a `Local`, plus offsets.
fn direct(b: &Builder, a: usize) -> Option<u32> {
    match kind(b, a) {
        Kind::Local => Some(b.word(a, 3)),
        Kind::Bin if matches!(Op::of(b.word(a, 0)), Op::Add | Op::Sub) && M(b.word(a, 1)) == M::PTR => direct(b, kids(b, a)[0]),
        _ => None,
    }
}

fn kids(b: &Builder, n: usize) -> Vec<usize> {
    let (mut c, end) = (n + 1, n + b.size(n));
    let mut out = Vec::new();
    while c < end {
        out.push(c);
        c += b.size(c);
    }
    out
}

/// The exits some lanes took, pending until they rejoin: their control's level, and the node they rejoin at's end
/// (a loop for a `break`, an iteration for a `continue`, an inlined call for a `return`).
#[derive(Clone, Copy, Default)]
struct Exits {
    ret: Option<Level>,
    brk: Option<Level>,
    cont: Option<Level>,
}

fn join(a: Option<Level>, b: Option<Level>) -> Option<Level> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.max(y)),
        (x, None) => x,
        (None, y) => y,
    }
}

impl Exits {
    fn join(self, o: Exits) -> Exits {
        Exits { ret: join(self.ret, o.ret), brk: join(self.brk, o.brk), cont: join(self.cont, o.cont) }
    }
}

/// A place a pointer may point at: a variable, at a constant offset in it or anywhere.
type Target = (u32, Option<u32>);

/// A place an address reaches: a variable, its constant offset if any, and the level of the address in it.
type Place = (u32, Option<u32>, Level);

/// Bytes `off..end` of a variable, and their level.
#[derive(Clone, Copy)]
struct Cell {
    off: u32,
    end: u32,
    l: Level,
}

struct Analysis {
    lev: Vec<Level>,
    /// Each variable's levels, by the byte ranges written (a structure's fields apart; `0..u32::MAX` the whole).
    vars: HashMap<u32, Vec<Cell>>,
    /// The variables a variable's pointers may point into (a lane's own: OpenCL's private memory, which no other
    /// lane reaches). A read or write through them is one of those variables'.
    pts: HashMap<u32, Vec<Target>>,
    changed: bool,
    /// The inlined calls being walked, outermost first (the kernel's own, instance 0): each one's instance, and the
    /// level of the returns some of its lanes took so far. Those lanes never read its own variables again, so the
    /// returns raise only the writes its caller sees (`returned`).
    regions: Vec<(u32, Level)>,
    sync: Vec<bool>,
    gsync: Vec<bool>,
    effect: Vec<bool>,
}

/// Whether an atomic leaves its word as it is (a poll: `atomic_or(p, 0)`, ...), its operand node `x`.
fn identity(b: &Builder, op: Builtin, x: usize) -> bool {
    if kind(b, x) != Kind::Const {
        return false;
    }
    let (m, v) = (M(b.word(x, 0)), u64::from(b.word(x, 1)) | u64::from(b.word(x, 2)) << 32);
    match op {
        Builtin::AtomicOr | Builtin::AtomicAdd | Builtin::AtomicSub | Builtin::AtomicXor => crate::ops::norm(m, v) == 0,
        Builtin::AtomicAnd => crate::ops::norm(m, v) == crate::ops::norm(m, u64::MAX),
        _ => false,
    }
}

pub fn is_atomic(b: Builtin) -> bool {
    matches!(
        b,
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
            | Builtin::AtomicDec
    )
}

impl Analysis {
    fn new(b: &Builder) -> Self {
        let n = b.len();
        let (mut sync, mut gsync, mut effect) = (vec![false; n], vec![false; n], vec![false; n]);
        for i in (0..n).rev() {
            let (mut s, mut g, mut e) = match kind(b, i) {
                Kind::Builtin => {
                    let x = Builtin::of(b.word(i, 0));
                    (x.syncs(), x == Builtin::Barrier, x.syncs() || is_atomic(x))
                }
                Kind::Store | Kind::Update | Kind::Copy | Kind::Zero | Kind::Inline | Kind::Return | Kind::Break | Kind::Continue => (false, false, true),
                _ => (false, false, false),
            };
            for c in kids(b, i) {
                s |= sync[c];
                g |= gsync[c];
                e |= effect[c];
            }
            (sync[i], gsync[i], effect[i]) = (s, g, e);
        }
        Analysis { lev: vec![Level::Uniform; n], vars: HashMap::new(), pts: HashMap::new(), changed: false, regions: vec![(0, Level::Uniform)], sync, gsync, effect }
    }

    /// The level of bytes `at` (`off..end`) of variable `v`, or of all of it.
    fn get(&self, v: u32, at: Option<(u32, u32)>) -> Level {
        self.vars.get(&v).map_or(Level::Uniform, |cs| {
            cs.iter().filter(|c| at.is_none_or(|(o, e)| c.off < e && o < c.end)).map(|c| c.l).max().unwrap_or(Level::Uniform)
        })
    }

    /// Bytes `at` of variable `v` (all of it if `None`) written with a value of level `l`.
    fn set(&mut self, v: u32, at: Option<(u32, u32)>, l: Level) {
        if l == Level::Uniform {
            return;
        }
        let (off, end) = at.unwrap_or((0, u32::MAX));
        let cs = self.vars.entry(v).or_default();
        match cs.iter_mut().find(|c| c.off == off && c.end == end) {
            Some(c) if l > c.l => {
                c.l = l;
                self.changed = true;
            }
            Some(_) => {}
            None => {
                cs.push(Cell { off, end, l });
                self.changed = true;
            }
        }
    }

    fn var(&self, v: u32) -> Level {
        self.get(v, None)
    }

    fn raise(&mut self, v: u32, l: Level) {
        self.set(v, None, l);
    }

    /// That variable `v`'s pointers may point into `ts` too.
    fn point(&mut self, v: u32, ts: &[Target]) {
        let e = self.pts.entry(v).or_default();
        for &t in ts {
            if !e.contains(&t) {
                e.push(t);
                self.changed = true;
            }
        }
    }

    /// Where address `a` points into a lane's own variables: a variable's own address, or the pointers it is made of.
    fn targets(&self, b: &Builder, a: usize) -> Vec<Target> {
        let k = kids(b, a);
        match kind(b, a) {
            Kind::Local => vec![(b.word(a, 3), Some(b.word(a, 2)))],
            Kind::Bin if M(b.word(a, 1)) == M::PTR => {
                let add = Op::of(b.word(a, 0)) == Op::Add;
                let c = constant(b, k[1]);
                let mut t: Vec<Target> = self
                    .targets(b, k[0])
                    .into_iter()
                    .map(|(v, off)| (v, off.zip(c).map(|(f, c)| if add { f.wrapping_add(c as u32) } else { f.wrapping_sub(c as u32) })))
                    .collect();
                t.extend(self.targets(b, k[1]).into_iter().map(|(v, _)| (v, None)));
                t
            }
            Kind::Cast => self.targets(b, k[0]),
            Kind::Comma | Kind::Store => self.targets(b, k[1]),
            Kind::Cond => {
                let mut t = self.targets(b, k[1]);
                t.extend(self.targets(b, k[2]));
                t
            }
            Kind::Load => self.held(b, k[0]),
            Kind::Inline => self.pts.get(&b.word(a, 3)).cloned().unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    /// The variables whose bytes address `a` reaches: its own, an inlined call's result's (an aggregate's), or
    /// those its pointers point into.
    fn holds(&self, b: &Builder, a: usize) -> Vec<Target> {
        match direct(b, a) {
            Some(v) => vec![(v, None)],
            None if kind(b, a) == Kind::Inline => vec![(b.word(a, 3), Some(0))],
            None => self.targets(b, a),
        }
    }

    /// Where the pointers held at address `a` point.
    fn held(&self, b: &Builder, a: usize) -> Vec<Target> {
        self.holds(b, a).iter().flat_map(|(v, _)| self.pts.get(v).cloned().unwrap_or_default()).collect()
    }

    /// The fixed point: every node's level from its variables' and its control's.
    fn run(&mut self, b: &Builder) {
        loop {
            self.changed = false;
            self.stmt(b, 0, Level::Uniform);
            if !self.changed {
                break;
            }
        }
    }

    /// If `a` addresses a variable of the frame directly (a `Local`, plus offsets): the variable, the offset in it
    /// if it is constant, and the offset's level (each lane's own copy read at it).
    fn local(&mut self, b: &Builder, a: usize, ctl: Level) -> Option<(u32, Option<u32>, Level)> {
        match kind(b, a) {
            Kind::Local => {
                // Its address is each lane's own; its value, the variable's.
                self.lev[a] = Level::Varying;
                Some((b.word(a, 3), Some(b.word(a, 2)), Level::Uniform))
            }
            Kind::Bin if matches!(Op::of(b.word(a, 0)), Op::Add | Op::Sub) && M(b.word(a, 1)) == M::PTR => {
                let k = kids(b, a);
                let (v, off, l) = self.local(b, k[0], ctl)?;
                let o = self.expr(b, k[1], ctl);
                self.lev[a] = Level::Varying;
                let add = Op::of(b.word(a, 0)) == Op::Add;
                let off = off.zip(constant(b, k[1])).map(|(f, c)| if add { f.wrapping_add(c as u32) } else { f.wrapping_sub(c as u32) });
                Some((v, off, l.max(o)))
            }
            _ => None,
        }
    }

    /// The lane's own variables address `a` reaches, each at its offset if constant, with the level of the address
    /// (which bytes of each lane's copy); and the level of the address as one into memory the lanes share.
    fn places(&mut self, b: &Builder, a: usize, ctl: Level) -> (Vec<Place>, Option<Level>) {
        match self.local(b, a, ctl) {
            Some(p) => (vec![p], None),
            None => {
                let al = self.expr(b, a, ctl);
                (self.targets(b, a).into_iter().map(|(v, off)| (v, off, al)).collect(), Some(al))
            }
        }
    }

    /// The level of the `size` bytes at address `a`: a variable's (and its offset's), or the address's own (memory
    /// the lanes share, race-free).
    fn read(&mut self, b: &Builder, a: usize, size: u32, ctl: Level) -> Level {
        let (ps, mem) = self.places(b, a, ctl);
        ps.into_iter().fold(mem.unwrap_or(Level::Uniform), |l, (v, off, o)| l.max(o).max(self.get(v, off.map(|f| (f, f.saturating_add(size))))))
    }

    /// The levels of the `size` bytes at address `a`, by range from its start.
    fn cells_at(&mut self, b: &Builder, a: usize, size: u32, ctl: Level) -> Vec<Cell> {
        if kind(b, a) == Kind::Inline {
            self.expr(b, a, ctl);
            let v = b.word(a, 3);
            return self.vars.get(&v).map_or(Vec::new(), |cs| cs.iter().filter(|c| c.off < size).map(|c| Cell { off: c.off, end: c.end.min(size), l: c.l }).collect());
        }
        let (ps, mem) = self.places(b, a, ctl);
        if let ([(v, Some(f), o)], None | Some(Level::Uniform)) = (ps.as_slice(), mem) {
            let (v, f, o) = (*v, *f, *o);
            let end = f.saturating_add(size);
            let mut out: Vec<Cell> =
                self.vars.get(&v).map_or(Vec::new(), |cs| cs.iter().filter(|c| c.off < end && f < c.end).map(|c| Cell { off: c.off.max(f) - f, end: c.end.min(end) - f, l: c.l }).collect());
            if o > Level::Uniform {
                out.push(Cell { off: 0, end: size, l: o });
            }
            return out;
        }
        let l = ps.into_iter().fold(mem.unwrap_or(Level::Uniform), |l, (v, off, o)| l.max(o).max(self.get(v, off.map(|f| (f, f.saturating_add(size))))));
        vec![Cell { off: 0, end: size, l }]
    }

    /// A write of `size` bytes, of levels `cells`, at address `a` under `ctl`, the value's pointers into `ts`.
    fn write(&mut self, b: &Builder, a: usize, size: u32, cells: &[Cell], ctl: Level, ts: &[Target]) {
        let most = cells.iter().map(|c| c.l).max().unwrap_or(Level::Uniform);
        let (ps, _) = self.places(b, a, ctl);
        for (v, off, o) in ps {
            let under = o.max(ctl).max(self.returned(v));
            match off {
                Some(f) => {
                    for c in cells {
                        self.set(v, Some((f + c.off, f.saturating_add(c.end))), c.l.max(under));
                    }
                    self.set(v, Some((f, f.saturating_add(size))), under);
                }
                None => self.raise(v, most.max(under)),
            }
            self.point(v, ts);
        }
    }

    fn expr(&mut self, b: &Builder, n: usize, ctl: Level) -> Level {
        let k = kids(b, n);
        let l = match kind(b, n) {
            Kind::Const | Kind::Global | Kind::Shared => Level::Uniform,
            // A variable's address as a value: which variable is the same in every lane, each lane's own copy of it
            // (its pointers tracked, `targets`).
            Kind::Local => Level::Uniform,
            Kind::Load => self.read(b, k[0], M(b.word(n, 0)).bytes(), ctl),
            Kind::Store => {
                let size = M(b.word(n, 0)).bytes();
                let x = self.expr(b, k[1], ctl);
                let ts = self.targets(b, k[1]);
                self.write(b, k[0], size, &[Cell { off: 0, end: size, l: x }], ctl, &ts);
                x
            }
            Kind::Update => {
                let size = M(b.word(n, 1)).bytes();
                let x = self.expr(b, k[1], ctl);
                let old = self.read(b, k[0], size, ctl);
                let r = x.max(old);
                self.write(b, k[0], size, &[Cell { off: 0, end: size, l: r }], ctl, &[]);
                r
            }
            Kind::Zero => {
                self.write(b, k[0], b.word(n, 1), &[], ctl, &[]);
                Level::Uniform
            }
            Kind::Copy => {
                let size = b.word(n, 0);
                let cells = self.cells_at(b, k[1], size, ctl);
                let ts = self.held(b, k[1]);
                self.write(b, k[0], size, &cells, ctl, &ts);
                cells.iter().map(|c| c.l).max().unwrap_or(Level::Uniform)
            }
            Kind::Bin | Kind::Un | Kind::Cast => k.iter().map(|&c| self.expr(b, c, ctl)).max().unwrap_or(Level::Uniform),
            Kind::Comma => {
                self.expr(b, k[0], ctl);
                self.expr(b, k[1], ctl)
            }
            Kind::Cond => {
                let c = self.expr(b, k[0], ctl);
                let x = self.expr(b, k[1], ctl.max(c));
                let y = self.expr(b, k[2], ctl.max(c));
                c.max(x).max(y)
            }
            Kind::Logic => {
                let x = self.expr(b, k[0], ctl);
                let y = self.expr(b, k[1], ctl.max(x));
                x.max(y)
            }
            Kind::Builtin => {
                let a: Vec<Level> = k.iter().map(|&c| self.expr(b, c, ctl)).collect();
                let x = Builtin::of(b.word(n, 0));
                let most = a.iter().copied().max().unwrap_or(Level::Uniform);
                match x {
                    Builtin::LocalId | Builtin::GlobalId | Builtin::SubGroupLocalId => Level::Varying,
                    Builtin::LocalSize | Builtin::GlobalSize | Builtin::GroupId | Builtin::NumGroups | Builtin::Barrier | Builtin::MemFence | Builtin::SubGroupBarrier => Level::Uniform,
                    Builtin::SubGroupBallot => Level::SimdGroup,
                    Builtin::SubGroupShuffle if a[0] <= Level::Agreed => a[0],
                    Builtin::SubGroupShuffle if a[1] <= Level::SimdGroup => Level::SimdGroup,
                    Builtin::SubGroupShuffleUp | Builtin::SubGroupShuffleDown | Builtin::SubGroupShuffleXor if a[0] <= Level::Agreed => a[0],
                    Builtin::SubGroupShuffle | Builtin::SubGroupShuffleUp | Builtin::SubGroupShuffleDown | Builtin::SubGroupShuffleXor => Level::Varying,
                    _ if is_atomic(x) => {
                        if k.len() == 2 && identity(b, x, k[1]) && a[0] <= Level::SimdGroup {
                            a[0].max(Level::Agreed)
                        } else {
                            Level::Varying
                        }
                    }
                    _ => most,
                }
            }
            Kind::Inline => {
                self.regions.push((b.word(n, 3) >> 20, Level::Uniform));
                let mut ex = Exits::default();
                for &c in &k {
                    ex = ex.join(self.stmt_in(b, c, ctl, ex));
                }
                self.regions.pop();
                let m = M(b.word(n, 0));
                // An aggregate's value is its copy's address (its bytes' levels are its result's, `cells_at`).
                if m == M::VOID || m.is_agg() { Level::Uniform } else { self.var(b.word(n, 3)) }
            }
            Kind::Block | Kind::Expr => {
                self.stmt(b, n, ctl);
                Level::Uniform
            }
            _ => Level::Uniform,
        };
        // The tape's level is the value's own: a lane's own variable's address differs in each lane.
        let own = kind(b, n) == Kind::Local || (kind(b, n) == Kind::Inline && M(b.word(n, 0)).is_agg()) || !self.targets(b, n).is_empty();
        self.lev[n] = if own { Level::Varying } else { l };
        l
    }

    /// The level the returns taken so far raise a write of variable `v` to: those of the calls inside the one `v`
    /// is a variable of.
    fn returned(&self, v: u32) -> Level {
        let at = self.regions.iter().rposition(|r| r.0 == v >> 20).map_or(0, |p| p + 1);
        self.regions[at..].iter().map(|r| r.1).max().unwrap_or(Level::Uniform)
    }

    fn stmt(&mut self, b: &Builder, n: usize, ctl: Level) -> Exits {
        self.stmt_in(b, n, ctl, Exits::default())
    }

    /// Statement `n` under `ctl`, after the exits `before` in the construct it is in: the exits it takes.
    fn stmt_in(&mut self, b: &Builder, n: usize, ctl: Level, before: Exits) -> Exits {
        let top = self.regions.len() - 1;
        let saved = self.regions[top].1;
        self.regions[top].1 = saved.max(before.ret.unwrap_or(Level::Uniform));
        let ex = self.stmt_under(b, n, ctl, before);
        self.regions[top].1 = saved;
        ex
    }

    fn stmt_under(&mut self, b: &Builder, n: usize, ctl: Level, before: Exits) -> Exits {
        // Lanes that left by a `break` or `continue` come back: what they did not write differs from what the others
        // did.
        let ctl = ctl.max(join(before.brk, before.cont).unwrap_or(Level::Uniform));
        let k = kids(b, n);
        let ex = match kind(b, n) {
            Kind::Func => {
                self.stmt(b, k[0], ctl);
                Exits::default()
            }
            Kind::Block => {
                let mut ex = Exits::default();
                for &c in &k {
                    ex = ex.join(self.stmt_in(b, c, ctl, ex));
                }
                ex
            }
            Kind::Expr => {
                self.expr(b, k[0], ctl);
                Exits::default()
            }
            Kind::If => {
                let c = self.expr(b, k[0], ctl);
                let inner = ctl.max(c);
                let mut ex = self.stmt(b, k[1], inner);
                if k.len() > 2 {
                    ex = ex.join(self.stmt(b, k[2], inner));
                }
                self.lev[n] = c;
                return ex;
            }
            Kind::While | Kind::For | Kind::Do => {
                let (cond, body) = if kind(b, n) == Kind::Do { (k[1], k[0]) } else { (k[0], k[1]) };
                // The loop's lanes leave at different iterations if its condition varies, or some break.
                let mut lc = ctl;
                let mut ex;
                loop {
                    let c = self.expr(b, cond, lc);
                    ex = self.stmt(b, body, lc.max(c));
                    if kind(b, n) == Kind::For {
                        self.stmt_in(b, k[2], lc.max(c), Exits { cont: None, ..ex });
                    }
                    let next = lc.max(c).max(ex.brk.unwrap_or(Level::Uniform));
                    if next == lc {
                        break;
                    }
                    lc = next;
                }
                self.lev[n] = lc;
                return Exits { ret: ex.ret, brk: None, cont: None };
            }
            Kind::Return => {
                if b.word(n, 0) != 0 {
                    let x = self.expr(b, k[0], ctl);
                    let slot = b.word(n, 2);
                    if slot != NO_SLOT {
                        let (v, m) = (b.word(n, 4), M(b.word(n, 3)));
                        // The result is its caller's: the lanes that returned before wrote theirs.
                        let ctl = ctl.max(self.regions.last().map_or(Level::Uniform, |r| r.1));
                        if m.is_agg() {
                            // The aggregate's bytes, from the address returned, into the call's result.
                            let size = m.bytes();
                            for c in self.cells_at(b, k[0], size, ctl) {
                                self.set(v, Some((c.off, c.end)), c.l.max(ctl));
                            }
                            self.set(v, None, ctl);
                            let ts = self.held(b, k[0]);
                            self.point(v, &ts);
                        } else {
                            self.raise(v, x.max(ctl));
                            let ts = self.targets(b, k[0]);
                            self.point(v, &ts);
                        }
                    }
                }
                Exits { ret: Some(ctl), ..Default::default() }
            }
            Kind::Break => Exits { brk: Some(ctl), ..Default::default() },
            Kind::Continue => Exits { cont: Some(ctl), ..Default::default() },
            Kind::Empty => Exits::default(),
            _ => {
                self.expr(b, n, ctl);
                Exits::default()
            }
        };
        self.lev[n] = ctl;
        ex
    }

    // The barrier rule.

    /// Each node's field, the rule checked: its level, its cut, its check.
    fn check(&mut self, b: &Builder, p: &Program, kernel: &str) -> R<Vec<u16>> {
        let n = b.len();
        // Whether the lanes past node `i` (once it is done) reach a sync point before the kernel ends.
        let mut after = vec![false; n];
        let mut stack = vec![(0usize, false)];
        while let Some((i, a)) = stack.pop() {
            after[i] = a;
            let k = kids(b, i);
            match kind(b, i) {
                Kind::While | Kind::For | Kind::Do => {
                    let whole = a || self.sync[i];
                    for c in k {
                        stack.push((c, whole));
                    }
                }
                Kind::If | Kind::Cond => {
                    // The condition: then a branch; a branch: what follows the construct.
                    let branches = k[1..].iter().any(|&c| self.sync[c]);
                    stack.push((k[0], a || branches));
                    for &c in &k[1..] {
                        stack.push((c, a));
                    }
                }
                _ => {
                    let mut later = a;
                    for &c in k.iter().rev() {
                        stack.push((c, later));
                        later |= self.sync[c];
                    }
                }
            }
        }
        let mut c = Checker { b, a: self, p, kernel, after: &after, divs: Vec::new(), checks: Vec::new(), err: None };
        c.walk(0);
        if let Some(e) = c.err {
            return Err(e);
        }
        let checks = std::mem::take(&mut c.checks);
        let mut fields = vec![0u16; n];
        for (i, f) in fields.iter_mut().enumerate() {
            *f = self.lev[i] as u16 | if self.sync[i] { SYNC } else { 0 } | if self.gsync[i] { GSYNC } else { 0 } | if self.effect[i] { EFFECT } else { 0 };
        }
        for i in checks {
            fields[i] |= CHECK;
        }
        Ok(fields)
    }
}

/// Lanes left out from a point on: by a branch or loop on a value of `level` (`node`; not `exit`), or by an exit of
/// that level (`node` the exit, `cause` the branch whose control made it vary); `allowed` if they end without
/// reaching a sync point again; pending until the walk leaves `until`, where they rejoin.
#[derive(Clone, Copy)]
struct Div {
    level: Level,
    node: usize,
    exit: bool,
    cause: Option<usize>,
    allowed: bool,
    until: usize,
}

struct Checker<'c> {
    b: &'c Builder,
    a: &'c Analysis,
    p: &'c Program,
    kernel: &'c str,
    after: &'c [bool],
    divs: Vec<Div>,
    checks: Vec<usize>,
    err: Option<Error>,
}

impl Checker<'_> {
    fn site(&self, n: usize) -> String {
        let w = match kind(self.b, n) {
            Kind::Builtin => self.b.word(n, 3),
            Kind::Return => self.b.word(n, 1),
            Kind::If | Kind::While | Kind::For | Kind::Do | Kind::Break | Kind::Continue => self.b.word(n, 0),
            Kind::Cond => return "a ?: expression".into(),
            Kind::Logic => return "a && or || expression".into(),
            k => return format!("a {k:?}"),
        };
        self.p.sites.get(w as usize).cloned().unwrap_or_else(|| "?".into())
    }

    fn walk(&mut self, n: usize) {
        if self.err.is_some() {
            return;
        }
        let b = self.b;
        let k = kids(b, n);
        match kind(b, n) {
            Kind::If | Kind::Cond | Kind::Logic => {
                self.walk(k[0]);
                let l = self.a.lev[k[0]];
                for (j, &c) in k[1..].iter().enumerate() {
                    if l > Level::Uniform {
                        // The lanes left out are in the other branch, then past the construct (`&&`'s skipped ones
                        // past it at once).
                        let other = kind(b, n) != Kind::Logic && k[1..].iter().enumerate().any(|(x, &o)| x != j && self.a.sync[o]);
                        self.divs.push(Div { level: l, node: n, exit: false, cause: None, allowed: !other && !self.after[n], until: n });
                    }
                    self.walk(c);
                    self.divs.retain(|d| d.exit || d.node != n);
                }
            }
            Kind::While | Kind::For | Kind::Do => {
                // Lanes that leave at different iterations by a condition that varies (its breaks and returns are
                // their own).
                let l = self.a.lev[if kind(b, n) == Kind::Do { k[1] } else { k[0] }];
                if l > Level::Uniform {
                    self.divs.push(Div { level: l, node: n, exit: false, cause: None, allowed: !self.after[n], until: n });
                }
                for &c in &k {
                    self.walk(c);
                }
                // Its own, and its breaks' and continues', end here.
                self.divs.retain(|d| d.until != n);
            }
            Kind::Inline => {
                for &c in &k {
                    self.walk(c);
                }
                // Its returns rejoin here.
                self.divs.retain(|d| d.until != n);
            }
            Kind::Return | Kind::Break | Kind::Continue => {
                for &c in &k {
                    self.walk(c);
                }
                // Where its lanes rejoin: its call's end, its loop's. The kernel's own return ends them.
                let until = match kind(b, n) {
                    Kind::Return => self.region(n),
                    _ => Some(self.enclosing_loop(n)),
                };
                let Some(until) = until else { return };
                // Its lanes are some of those there only by the control that varies within that place: its branches
                // and loops, and the exits taken before it.
                let end = until + b.size(until);
                let mut l = Level::Uniform;
                let mut cause = None;
                for d in self.divs.iter().filter(|d| d.node > until && d.node < end) {
                    if d.level > l {
                        l = d.level;
                        cause = Some(d.cause.unwrap_or(d.node));
                    }
                }
                if l > Level::Uniform {
                    // A `continue`'s lanes come back for the next iteration: they never end before it.
                    let allowed = kind(b, n) != Kind::Continue && !self.after[until];
                    self.divs.push(Div { level: l, node: n, exit: true, cause, allowed, until });
                }
            }
            Kind::Builtin => {
                for &c in &k {
                    self.walk(c);
                }
                let x = Builtin::of(b.word(n, 0));
                if x.syncs() {
                    self.meet(n, x);
                }
            }
            _ => {
                for &c in &k {
                    self.walk(c);
                }
            }
        }
    }

    fn enclosing_loop(&self, n: usize) -> usize {
        self.ancestors(n).into_iter().find(|&a| matches!(kind(self.b, a), Kind::While | Kind::For | Kind::Do)).unwrap_or(0)
    }

    fn region(&self, n: usize) -> Option<usize> {
        self.ancestors(n).into_iter().find(|&a| kind(self.b, a) == Kind::Inline)
    }

    /// Node `n`'s ancestors, innermost first.
    fn ancestors(&self, n: usize) -> Vec<usize> {
        let mut out = Vec::new();
        let mut at = 0;
        while at != n {
            out.push(at);
            at = kids(self.b, at).into_iter().find(|&c| c <= n && n < c + self.b.size(c)).expect("a path to the node");
        }
        out.reverse();
        out
    }

    /// Sync point `n` met under the divergences pending.
    fn meet(&mut self, n: usize, x: Builtin) {
        let need = if x == Builtin::Barrier { Level::Agreed } else { Level::SimdGroup };
        for d in self.divs.clone() {
            if d.level <= need {
                if d.level == Level::Agreed && x == Builtin::Barrier {
                    // The rule rests on the group's agreeing: checked where control turns on it.
                    self.checks.push(d.cause.unwrap_or(d.node));
                }
                continue;
            }
            if d.allowed {
                continue;
            }
            let whose = if x == Builtin::Barrier { "its threadgroup's threads" } else { "its SIMD group's lanes" };
            let why = match kind(self.b, d.node) {
                Kind::Return | Kind::Break | Kind::Continue => match d.cause {
                    Some(c) => format!("{} leaves some of them out (under {}, on a value that varies), and they reach a sync point again", self.site(d.node), self.site(c)),
                    None => format!("{} leaves some of them out, and they reach a sync point again", self.site(d.node)),
                },
                _ => format!("{} turns on a value that varies across them, and the ones it leaves out reach a sync point again", self.site(d.node)),
            };
            self.err = Some(Error(format!("{}: {} is not reached by all of {whose} together (decision 0567 §3): {why}", self.kernel, self.site(n))));
            return;
        }
    }
}
