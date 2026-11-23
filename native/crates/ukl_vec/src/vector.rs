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
    /// Its code tape (`code`), which `run` executes.
    pub code: Tape,
    pub name: String,
    /// Whether its levels take a thread's id in its group as its SIMD group's and its lane's (`Level::Lid`): then
    /// `run` takes only a group whose lanes are so (1-D, or its first dimension a multiple of the SIMD width).
    pub lid: bool,
    /// Its code tape compiled (`jit`), on its first launch that asks for it, or why it could not be.
    pub jit: std::sync::OnceLock<Result<crate::jit::Jit, String>>,
    /// Its SSA compiled (`cl`), on its first launch that asks for it, or why it could not be.
    pub cl: std::sync::OnceLock<Result<crate::cl::Compiled, String>>,
}

type R<T> = Result<T, Error>;

/// The kept form's first bytes, and its version: a change to the layout below changes it.
const KEPT: &[u8; 8] = b"UKLVEC01";

impl Kernel {
    /// The kernel as bytes, for a store of compiled kernels (the device's): its name, its flag, then each tape's
    /// image and words, each a length and its bytes, little-endian.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = KEPT.to_vec();
        out.extend((self.name.len() as u64).to_le_bytes());
        out.extend(self.name.as_bytes());
        out.push(u8::from(self.lid));
        for t in [&self.tape, &self.code] {
            out.extend((t.image.len() as u64).to_le_bytes());
            out.extend(&t.image);
            out.extend((t.words.len() as u64).to_le_bytes());
            out.extend(t.words.iter().flat_map(|w| w.to_le_bytes()));
        }
        out
    }

    /// A kernel from `to_bytes`'s, or none if they are not one's (damaged, or another version's).
    pub fn from_bytes(b: &[u8]) -> Option<Kernel> {
        let mut at = 0usize;
        let mut take = |n: usize| -> Option<&[u8]> {
            let s = b.get(at..at.checked_add(n)?)?;
            at += n;
            Some(s)
        };
        if take(8)? != KEPT {
            return None;
        }
        let len = |s: &[u8]| usize::try_from(u64::from_le_bytes(s.try_into().ok()?)).ok();
        let n = len(take(8)?)?;
        let name = String::from_utf8(take(n)?.to_vec()).ok()?;
        let lid = take(1)?[0] != 0;
        let mut tapes = Vec::new();
        for _ in 0..2 {
            let n = len(take(8)?)?;
            let image = take(n)?.to_vec();
            let n = len(take(8)?)?;
            let words = take(n.checked_mul(4)?)?.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect();
            rerail_tape_format::View::new(&image).ok()?;
            tapes.push(Tape { image, words });
        }
        if at != b.len() {
            return None;
        }
        let code = tapes.pop()?;
        let tape = tapes.pop()?;
        Some(Kernel { tape, code, name, lid, jit: Default::default(), cl: Default::default() })
    }
}

/// The vector tape of kernel `kernel` of `p`.
pub fn vectorize(p: &Program, kernel: u32) -> R<Kernel> {
    let f = &p.funcs[kernel as usize];
    let node = f.node.ok_or_else(|| Error(format!("{} has no body", f.name)))?;
    let kv = View::new(&p.tape);
    // Each function's bodies, one per context it is called in (decision 0567 §2.2): the analysis puts each call in
    // its context's body, found as it walks, or in its function's fallback (no context) where that body is not yet
    // made. Each round makes, at each call site it reaches, the bodies the rounds before wanted there (so a body is
    // made only where a call reached may take it), and each site's set only grows, so the rounds end; then the
    // bodies the calls were put in, alone, are the kernel's, analysed once more and checked.
    let mut known: Known = HashMap::new();
    const ROUNDS: u32 = 16;
    let mut sites: HashMap<(Spec, usize), Context> = HashMap::new();
    let (none_known, no_sites) = (HashMap::new(), HashMap::new());
    for round in 1..=ROUNDS {
        let t0 = std::time::Instant::now();
        let e = emit(p, &kv, kernel, node, &known, &no_sites)?;
        let t1 = std::time::Instant::now();
        let mut a = Analysis::new(&e.b);
        a.ids = e.ids.clone();
        a.dynamic = true;
        a.run(&e.b);
        let t2 = std::time::Instant::now();
        sites = e.origins.iter().map(|(call, site)| (site.clone(), e.specs[a.assign.get(call).copied().unwrap_or(callee(&e.b, *call)) as usize].1.clone())).collect();
        if std::env::var("RERAIL_UKL_VEC_TRACE").is_ok() {
            eprintln!("{}: round {round}, {} nodes, {} bodies, {} contexts wanted; emit {:.2} s, analysis {:.2} s", f.name, e.b.len(), e.specs.len(), a.wanted.len(), (t1 - t0).as_secs_f64(), (t2 - t1).as_secs_f64());
        }
        if a.wanted.is_empty() {
            break;
        }
        // The next round's bodies: at each site, those the rounds before wanted there, and what this one did.
        let at: HashMap<usize, (Spec, usize)> = e.origins.iter().map(|(call, site)| (*call, site.clone())).collect();
        let callees: HashMap<usize, u32> = e.origins.iter().map(|(call, _)| (*call, e.b.word(*call, 0))).collect();
        // Each call's caller's body (the last body to start before it: the tape is in preorder).
        let starts: Vec<usize> = kids(&e.b, 0).into_iter().filter(|&c| kind(&e.b, c) == Kind::Body).collect();
        let caller = |call: usize| starts.partition_point(|&s| s < call).checked_sub(1).map(|k| starts[k]);
        drop(e);
        for (call, c) in std::mem::take(&mut a.wanted) {
            // A function written once for its calls has its fallback body at a call where neither it nor its caller
            // reaches a sync point: a context is what shows a group's lanes reach one together, and a callee's
            // result of one value (its context's) what keeps its caller's control so.
            let syncs = a.sync[call] || caller(call).is_none_or(|b| a.sync[b]);
            if !c.is_empty() && (syncs || !joins(p, &kv, callees[&call])) {
                known.entry(at[&call].clone()).or_default().insert(c);
            }
        }
    }
    let e = emit(p, &kv, kernel, node, &none_known, &sites)?;
    let mut b = e.b;
    let mut a = Analysis::new(&b);
    a.run(&b);
    let fields = a.check(&b, p, &f.name)?;
    let lid = a.lev.contains(&Level::Lid) || rows(&b);
    for (n, field) in fields.into_iter().enumerate() {
        b.set_field(n, field);
    }
    let tape = b.finish();
    let code = crate::code::compile(&tape)?;
    Ok(Kernel { tape, code, name: f.name.clone(), lid, jit: Default::default(), cl: Default::default() })
}

/// The nodes past which a function is written once for its calls (`joins`).
pub const JOIN_NODES: usize = 400;

/// Whether function `id` of `p` is written once for its calls: its fallback body alone (no context's, where it
/// reaches no sync point), its arguments' shapes joined in its Rust (rs.rs). A function the source declares a call (`Func::called`), or one of
/// more than `JOIN_NODES` nodes, whose bodies' code would outweigh what knowing an argument one value saves.
pub fn joins(p: &Program, kv: &View, id: u32) -> bool {
    let f = &p.funcs[id as usize];
    f.called || f.node.is_some_and(|n| kv.size(n) > JOIN_NODES)
}

/// The context a function is called in: each argument's level (an aggregate's, its most varying byte's). A body's
/// levels are relative to its entry, so the control it is called under is not its. None (a function's fallback
/// body): the calls whose context's body is not made. A function with no parameters has the one body.
type Context = Vec<Level>;

/// A function in a context: its body in the vector tape (the kernel's own, with none).
type Spec = (u32, Context);

/// The contexts each call site (its caller's body, its node in the kernel tape) has wanted its callee's bodies in.
type Known = HashMap<(Spec, usize), std::collections::BTreeSet<Context>>;

/// The vector tape's bodies: the kernel's, then each function's it reaches, in each context `known` names and its
/// fallback, its calls put in their fallbacks; or, with `sites`, in the context each site names alone.
fn emit<'a, 'p>(p: &'p Program, kv: &'a View<'p>, kernel: u32, node: usize, known: &'a Known, sites: &'a HashMap<(Spec, usize), Context>) -> R<Emitter<'a, 'p>> {
    let mut e = Emitter { p, kv, b: Builder::default(), sites, known, specs: Vec::new(), ids: HashMap::new(), origins: Vec::new(), calls: HashMap::new() };
    let own = e.own(node).to_vec();
    let root = e.b.open(Kind::Func, &own);
    e.emit(kv.child(node, 0), &(kernel, Vec::new()), 0)?;
    // Each function in each context, once: its body under its own frame.
    let mut at = 0;
    while at < e.specs.len() {
        let spec = e.specs[at].clone();
        let s = at as u32;
        at += 1;
        let id = spec.0;
        let callee = p.funcs[id as usize].node.ok_or_else(|| Error(format!("{} is called and has no body", p.funcs[id as usize].name)))?;
        let (frame, m) = (kv.w(callee, 1).next_multiple_of(16), M(kv.w(callee, 3)));
        let params = kv.w(callee, 4) as usize;
        let mut words = vec![s, id, frame, frame, m.0, var(s + 1, RESULT), params as u32];
        words.extend((0..2 * params).map(|k| kv.w(callee, 5 + k)));
        let body = e.b.open(Kind::Body, &words);
        e.emit(kv.child(callee, 0), &spec, s + 1)?;
        e.b.close(body);
    }
    e.b.close(root);
    // A call that recurses is refused (a lane's frame is the deepest chain of calls' regions, `code`).
    e.depth(kernel, &mut HashMap::new(), &mut Vec::new())?;
    Ok(e)
}

/// The kernel's own region in a lane's frame (a vector tape's): its frame (its first body's `Func` words).
pub fn kernel_frame(v: &View) -> u32 {
    v.w(0, 1).next_multiple_of(16)
}

/// A function's region in a lane's frame: its frame, then its result's slot.
pub fn region(frame: u32, m: M) -> u32 {
    frame.next_multiple_of(16) + m.bytes().max(8).next_multiple_of(16)
}

/// The kernel tape's bodies copied into the vector tape: the kernel's, then each function's it reaches, once a
/// context it is called in (a `Body`); calls stay calls, each to its callee's body in its context.
struct Emitter<'a, 'p> {
    p: &'p Program,
    kv: &'a View<'p>,
    b: Builder,
    /// Each call site's context (by its caller's body and its node in the kernel tape), for the last emission; the
    /// contexts each function's bodies are made in, for a round's.
    sites: &'a HashMap<(Spec, usize), Context>,
    known: &'a Known,
    /// The bodies, in the order first called, and their numbers.
    specs: Vec<Spec>,
    ids: HashMap<Spec, u32>,
    /// Each call in the vector tape: its node, and its site.
    origins: Vec<(usize, (Spec, usize))>,
    /// Each function's callees.
    calls: HashMap<u32, Vec<u32>>,
}

/// A function's result's variable number in its instance.
const RESULT: u32 = VAR_IN_FUNCTION;

/// A variable number's bits of its number in its function (`var`'s `n`).
pub(crate) const VAR_IN_FUNCTION: u32 = (1 << 20) - 1;

/// A variable's number: its function's instance (the kernel's 0, a function's its id plus 1), and its number in its
/// function (`Kind::Local`).
fn var(instance: u32, n: u32) -> u32 {
    instance << 20 | n
}

/// The body a `Call` calls (its function in its context).
fn callee(b: &Builder, call: usize) -> u32 {
    b.word(call, 3)
}

/// The instance of the body a `Call` calls.
fn callee_instance(b: &Builder, call: usize) -> u32 {
    callee(b, call) + 1
}

impl<'p> Emitter<'_, 'p> {
    /// Node `n`'s own payload words (before its first child's).
    fn own(&self, n: usize) -> &'p [u32] {
        let v = &self.kv.v;
        let s = v.start(n) as usize;
        let e = if v.size(n) > 1 { v.start(n + 1) } else { v.end(n) } as usize;
        &self.kv.words[s..e]
    }

    /// The number of `spec`'s body, made if new.
    fn spec(&mut self, spec: Spec) -> u32 {
        if let Some(&s) = self.ids.get(&spec) {
            return s;
        }
        let s = self.specs.len() as u32;
        self.ids.insert(spec.clone(), s);
        self.specs.push(spec);
        s
    }

    /// Node `n` of the body of `spec` (its variables of `instance`).
    fn emit(&mut self, n: usize, spec: &Spec, instance: u32) -> R<()> {
        let func = spec.0;
        let kv = self.kv;
        match kv.kind(n) {
            Kind::Local => {
                let (off, size, inner) = (kv.w(n, 0), kv.w(n, 1), kv.w(n, 2));
                self.b.leaf(Kind::Local, &[off, size, inner, var(instance, kv.w(n, 3))]);
            }
            Kind::Return => {
                let (has, site) = (kv.w(n, 0), kv.w(n, 1));
                // The kernel's own return ends the thread; a function's stores its result at its slot.
                let (slot, m, v) = if instance == 0 {
                    (NO_SLOT, M::VOID, 0)
                } else {
                    let node = self.p.funcs[func as usize].node.expect("a called function's body");
                    (kv.w(node, 1).next_multiple_of(16), M(kv.w(node, 3)), var(instance, RESULT))
                };
                let r = self.b.open(Kind::Return, &[has, site, slot, m.0, v]);
                if has != 0 {
                    self.emit(kv.child(n, 0), spec, instance)?;
                }
                self.b.close(r);
            }
            Kind::Call => {
                let id = kv.w(n, 0);
                let cs = self.calls.entry(func).or_default();
                if !cs.contains(&id) {
                    cs.push(id);
                }
                let site = (spec.clone(), n);
                let s = self.spec((id, self.sites.get(&site).cloned().unwrap_or_default()));
                for c in self.known.get(&site).into_iter().flatten() {
                    self.spec((id, c.clone()));
                }
                let node = self.b.open(Kind::Call, &[id, kv.w(n, 1), kv.w(n, 2), s]);
                self.origins.push((node, site));
                for a in kv.children(n) {
                    self.emit(a, spec, instance)?;
                }
                self.b.close(node);
            }
            k => {
                let own = self.own(n).to_vec();
                let node = self.b.open(k, &own);
                for c in kv.children(n) {
                    self.emit(c, spec, instance)?;
                }
                self.b.close(node);
            }
        }
        Ok(())
    }

    /// The bytes a lane's frame needs from function `f`'s down: its region, then its deepest callee's. A function
    /// that calls itself is refused (the GPUs' kernels cannot).
    fn depth(&self, f: u32, memo: &mut HashMap<u32, u32>, path: &mut Vec<u32>) -> R<u32> {
        if let Some(&d) = memo.get(&f) {
            return Ok(d);
        }
        if path.contains(&f) {
            return Err(Error(format!("{} calls itself: a kernel's calls cannot recurse", self.p.funcs[f as usize].name)));
        }
        path.push(f);
        let node = self.p.funcs[f as usize].node.expect("a body");
        let own = if path.len() == 1 { self.kv.w(node, 1).next_multiple_of(16) } else { region(self.kv.w(node, 1), M(self.kv.w(node, 3))) };
        let mut below = 0;
        for &g in self.calls.get(&f).map_or(&[][..], Vec::as_slice) {
            below = below.max(self.depth(g, memo, path)?);
        }
        path.pop();
        memo.insert(f, own + below);
        Ok(own + below)
    }
}

fn kind(b: &Builder, n: usize) -> Kind {
    Kind::of((b.key(n) & 0x3ff) as u16)
}

/// Node `n`'s value if it is a constant (folded through casts and arithmetic).
/// Whether a kernel takes a thread's id in its group's second or third dimension (`Level::SimdGroup`'s).
fn rows(b: &Builder) -> bool {
    (0..b.len()).any(|i| {
        kind(b, i) == Kind::Builtin
            && matches!(Builtin::of(b.word(i, 0)), Builtin::LocalId | Builtin::GlobalId)
            && b.size(i) > 1 && matches!(constant(b, i + 1), Some(1 | 2))
    })
}

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

fn kids(b: &Builder, n: usize) -> Kids {
    let (mut c, end) = (n + 1, n + b.size(n));
    let mut out = Kids { len: 0, inline: [0; KIDS_INLINE], more: Vec::new() };
    while c < end {
        if out.len < KIDS_INLINE {
            out.inline[out.len] = c;
        } else {
            if out.more.is_empty() {
                out.more.extend_from_slice(&out.inline);
            }
            out.more.push(c);
        }
        out.len += 1;
        c += b.size(c);
    }
    out
}

/// A node's children held in place up to `KIDS_INLINE` of them (the analysis asks for them at every node it walks, so
/// a list made on the heap each time was much of its work), past that in a vector.
const KIDS_INLINE: usize = 8;

struct Kids {
    len: usize,
    inline: [usize; KIDS_INLINE],
    more: Vec<usize>,
}

impl std::ops::Deref for Kids {
    type Target = [usize];

    fn deref(&self) -> &[usize] {
        if self.len <= KIDS_INLINE { &self.inline[..self.len] } else { &self.more }
    }
}

impl IntoIterator for Kids {
    type Item = usize;
    type IntoIter = KidsIter;

    fn into_iter(self) -> KidsIter {
        KidsIter { kids: self, at: 0 }
    }
}

struct KidsIter {
    kids: Kids,
    at: usize,
}

impl Iterator for KidsIter {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        let c = self.kids.get(self.at).copied();
        self.at += 1;
        c
    }
}

impl<'a> IntoIterator for &'a Kids {
    type Item = &'a usize;
    type IntoIter = std::slice::Iter<'a, usize>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
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
        (Some(x), Some(y)) => Some(x.join(y)),
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

/// A variable's cells where the walk is: its byte ranges and their levels (a range not in it is uniform).
type Cells = std::collections::BTreeSet<(u32, u32, Level)>;

/// A body's own variables' cells where the walk is (and those of others it reaches). A join is the union. Its
/// variables in order, each one's cells shared until written: a copy of the state (at each branch, each loop's
/// iteration, each exit) copies its list, not its cells, and a write copies the one variable's; a join takes what
/// the other holds that it does not, skipping the cells both share.
#[derive(Clone, Default, PartialEq, Debug)]
struct State(Vec<(u32, std::rc::Rc<Cells>)>);

impl State {
    fn new() -> State {
        State(Vec::new())
    }

    fn get(&self, v: &u32) -> Option<&Cells> {
        self.0.binary_search_by_key(v, |e| e.0).ok().map(|i| &*self.0[i].1)
    }

    /// Variable `v`'s cells, to write (none before: empty).
    fn at(&mut self, v: u32) -> &mut Cells {
        let i = match self.0.binary_search_by_key(&v, |e| e.0) {
            Ok(i) => i,
            Err(i) => {
                self.0.insert(i, (v, Default::default()));
                i
            }
        };
        std::rc::Rc::make_mut(&mut self.0[i].1)
    }

    /// Variable `v`'s cells made `cs` (shared still if they already are).
    fn insert(&mut self, v: u32, cs: Cells) {
        if self.get(&v) != Some(&cs) {
            *self.at(v) = cs;
        }
    }

    /// Cell `c` added to variable `v`'s (shared still if it is there).
    fn add(&mut self, v: u32, c: (u32, u32, Level)) {
        if !self.get(&v).is_some_and(|cs| cs.contains(&c)) {
            self.at(v).insert(c);
        }
    }

    fn retain(&mut self, mut f: impl FnMut(u32) -> bool) {
        self.0.retain(|e| f(e.0));
    }

    fn iter(&self) -> impl Iterator<Item = (u32, &Cells)> {
        self.0.iter().map(|(v, cs)| (*v, &**cs))
    }
}

/// A debugging trace: `RERAIL_UKL_VEC_WHY_VAR`, the variable whose writes are printed.
fn trace_var() -> Option<u32> {
    static V: std::sync::OnceLock<Option<u32>> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var("RERAIL_UKL_VEC_WHY_VAR").ok().and_then(|t| u32::from_str_radix(t.trim_start_matches("0x"), 16).ok()))
}

fn join_into(to: &mut State, from: &State) {
    for (v, cs) in &from.0 {
        match to.0.binary_search_by_key(v, |e| e.0) {
            Ok(i) => {
                let t = &mut to.0[i].1;
                if std::rc::Rc::ptr_eq(t, cs) {
                    continue;
                }
                if **t == **cs {
                    // The same cells: shared from here on, so the next join of these finds them so at once.
                    *t = cs.clone();
                } else if !cs.is_subset(t) {
                    std::rc::Rc::make_mut(t).extend(cs.iter().copied());
                }
            }
            Err(i) => to.0.insert(i, (*v, cs.clone())),
        }
    }
}

/// The states the exits of a loop or `switch` take to where they go: its `break`s' (and a loop's `continue`s').
#[derive(Default)]
struct Leaving {
    looping: bool,
    brk: Vec<State>,
    cont: Vec<State>,
}

struct Analysis {
    lev: Vec<Level>,
    /// The body walked: its instance, its own variables' state there (flow-sensitive: a store to a variable replaces
    /// what it held, and where control rejoins, what either way wrote is raised by the level that split it), the
    /// variables written (a log, for what a branch wrote), each variable's bytes, and the exits' states.
    cur: u32,
    local: State,
    log: Vec<u32>,
    sizes: HashMap<u32, u32>,
    leaving: Vec<Leaving>,
    /// Each body's state on entry, of the variables of other bodies it reaches (through its parameters'
    /// pointers): the join of its callers' states there. And its state on leaving (its returns' and its end's): what
    /// a call of it leaves in them. The states at the body walked's returns.
    incoming: HashMap<u32, State>,
    effects: HashMap<u32, State>,
    /// The variables of other bodies each body writes (its callees' writes too).
    writes: HashMap<u32, std::collections::BTreeSet<u32>>,
    returns: Vec<State>,
    /// Each variable's levels from writes that are not its body's own walk's (flow-insensitive): a callee's
    /// parameters, a result, a write through a pointer from another body. By the byte ranges written (a structure's
    /// fields apart; `0..u32::MAX` the whole).
    vars: HashMap<u32, Vec<Cell>>,
    /// The variables a variable's pointers may point into (a lane's own: OpenCL's private memory, which no other
    /// lane reaches). A read or write through them is one of those variables'.
    pts: HashMap<u32, Vec<Target>>,
    changed: bool,
    /// The worklist (`run`): the instances to walk again (the kernel's 0, a body's its number plus 1), because what
    /// their walk reads changed; the instances that read each variable of another's (its levels or its pointers),
    /// and the callers of each body (which read its effects and writes).
    dirty: std::collections::BTreeSet<u32>,
    readers: HashMap<u32, Vec<u32>>,
    /// The variables of other instances the walk under way has read, to add to `readers` when it ends.
    reads: std::cell::RefCell<Vec<u32>>,
    callers: HashMap<u32, std::collections::BTreeSet<u32>>,
    /// The inlined calls being walked, outermost first (the kernel's own, instance 0): each one's instance, and the
    /// level of the returns some of its lanes took so far. Those lanes never read its own variables again, so the
    /// returns raise only the writes its caller sees (`returned`).
    regions: Vec<(u32, Level)>,
    sync: Vec<bool>,
    gsync: Vec<bool>,
    effect: Vec<bool>,
    /// Each function's `Body`, by its id.
    bodies: HashMap<u32, usize>,
    /// The control each function is called under: the most varying of its calls'.
    called: HashMap<u32, Level>,
    /// The returns pending where each function is called: they raise its writes outside it (the lanes that took
    /// them do not call it). The most varying of its calls'.
    pending: HashMap<u32, Level>,
    /// Each call's context (`Context`), as last walked.
    contexts: HashMap<usize, Context>,
    /// A round's (`dynamic`): the bodies made, by function and context; the body each call is put in; the contexts
    /// that have none yet.
    dynamic: bool,
    ids: HashMap<Spec, u32>,
    assign: HashMap<usize, u32>,
    wanted: HashMap<usize, Context>,
}

/// The level of operation `n` (a `Bin`, `Un` or `Cast`) on operands of levels `ls`: their join; but of a thread's id
/// (`Lid`), its remainder or low bits by the SIMD width (or a power of two below it) are its lane's, its quotient or
/// high bits by it (or a multiple) its SIMD group's, a conversion to an integer as wide or wider itself, and anything
/// else varies.
fn op_level(b: &Builder, n: usize, ls: &[Level]) -> Level {
    if !ls.contains(&Level::Lid) {
        return Level::all(ls.iter().copied());
    }
    let w = crate::run::W as u64;
    let k = kids(b, n);
    match kind(b, n) {
        Kind::Cast => {
            let (from, to) = (M(b.word(n, 0)), M(b.word(n, 1)));
            let int = |m: M| !m.float() && m != M::B && m != M::PTR && !m.is_agg();
            if int(from) && int(to) && to.bytes() >= 4 { Level::Lid } else { Level::Varying }
        }
        Kind::Bin if ls[0] == Level::Lid && ls[1] == Level::Uniform => match (Op::of(b.word(n, 0)), constant(b, k[1])) {
            (Op::Rem, Some(c)) if c.is_power_of_two() && c <= w => Level::Lane,
            (Op::And, Some(c)) if (c + 1).is_power_of_two() && c < w => Level::Lane,
            (Op::Div, Some(c)) if c % w == 0 && c > 0 => Level::SimdGroup,
            (Op::Shr, Some(c)) if c >= u64::from(w.trailing_zeros()) => Level::SimdGroup,
            _ => Level::Varying,
        },
        _ => Level::Varying,
    }
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
            | Builtin::AtomicLoad
            | Builtin::AtomicStore
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
                    (x.syncs(), x == Builtin::Barrier, x.syncs() || is_atomic(x) || x == Builtin::Yield)
                }
                Kind::Store | Kind::Update | Kind::Copy | Kind::Zero | Kind::Call | Kind::Return | Kind::Break | Kind::Continue => (false, false, true),
                _ => (false, false, false),
            };
            for c in kids(b, i) {
                s |= sync[c];
                g |= gsync[c];
                e |= effect[c];
            }
            (sync[i], gsync[i], effect[i]) = (s, g, e);
        }
        // A call syncs where its callee does: to a fixed point, bodies calling bodies.
        let bodies: HashMap<u32, usize> = kids(b, 0).into_iter().filter(|&c| kind(b, c) == Kind::Body).map(|c| (b.word(c, 0), c)).collect();
        loop {
            let mut changed = false;
            for i in (0..n).rev() {
                let (mut s, mut g) = (sync[i], gsync[i]);
                if kind(b, i) == Kind::Call {
                    let body = bodies[&callee(b, i)];
                    s |= sync[body];
                    g |= gsync[body];
                }
                for c in kids(b, i) {
                    s |= sync[c];
                    g |= gsync[c];
                }
                if (s, g) != (sync[i], gsync[i]) {
                    (sync[i], gsync[i]) = (s, g);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        Analysis {
            cur: 0,
            local: State::new(),
            log: Vec::new(),
            sizes: HashMap::new(),
            leaving: Vec::new(),
            incoming: HashMap::new(),
            effects: HashMap::new(),
            writes: HashMap::new(),
            dynamic: false,
            ids: HashMap::new(),
            assign: HashMap::new(),
            wanted: HashMap::new(),
            returns: Vec::new(),
            lev: vec![Level::Uniform; n],
            vars: HashMap::new(), pts: HashMap::new(), changed: false, dirty: Default::default(), readers: HashMap::new(), reads: Default::default(), callers: HashMap::new(), regions: vec![(0, Level::Uniform)], sync, gsync, effect, bodies, called: HashMap::new(), pending: HashMap::new(), contexts: HashMap::new() }
    }

    /// The level of bytes `at` (`off..end`) of variable `v`, or of all of it: its body's state's, where the walk is,
    /// and the writes from elsewhere.
    fn get(&self, v: u32, at: Option<(u32, u32)>) -> Level {
        let hit = |o: u32, e: u32| at.is_none_or(|(x, y)| o < y && x < e);
        self.note(v);
        let g = self.vars.get(&v).map_or(Level::Uniform, |cs| Level::all(cs.iter().filter(|c| hit(c.off, c.end)).map(|c| c.l)));
        let l = self.local.get(&v).map_or(Level::Uniform, |cs| Level::all(cs.iter().filter(|c| hit(c.0, c.1)).map(|c| c.2)));
        if trace_var() == Some(v) {
            eprintln!("get {v:#x} {at:?}: {g:?} {l:?} {:?}", self.local.get(&v));
        }
        g.join(l)
    }

    /// A store of `cells` at offset `f` (`size` bytes) of the body's own variable `v`, where the walk is: what those
    /// bytes held is gone.
    fn store_local(&mut self, v: u32, f: u32, size: u32, cells: &[Cell]) {
        let e = f.saturating_add(size);
        let mut cs = self.local.get(&v).cloned().unwrap_or_default();
        let hit: Vec<(u32, u32, Level)> = cs.iter().filter(|c| c.0 < e && f < c.1).copied().collect();
        for c in hit {
            cs.remove(&c);
            if c.0 < f {
                cs.insert((c.0, f, c.2));
            }
            if e < c.1 {
                cs.insert((e, c.1, c.2));
            }
        }
        for c in cells.iter().filter(|c| c.l > Level::Uniform) {
            cs.insert((f + c.off, f.saturating_add(c.end), c.l));
        }
        self.local.insert(v, cs);
        if trace_var() == Some(v) {
            eprintln!("store {v:#x} at {f}+{size}: {:?}", cells.iter().map(|c| (c.off, c.end, c.l)).collect::<Vec<_>>());
        }
        self.log.push(v);
    }

    /// Bytes `at` of the body's own variable `v` raised to `l`, where the walk is (a write that may not be there).
    fn raise_local(&mut self, v: u32, at: Option<(u32, u32)>, l: Level) {
        let (o, e) = at.unwrap_or((0, self.sizes.get(&v).copied().unwrap_or(u32::MAX)));
        if l > Level::Uniform {
            self.local.add(v, (o, e, l));
        }
        if trace_var() == Some(v) {
            eprintln!("raise {v:#x} {o}..{e} to {l:?}");
        }
        self.log.push(v);
    }

    /// Where control rejoins: the variables written since `from` in the log raised to `l`, the level that split it.
    #[track_caller]
    fn rejoin(&mut self, from: usize, l: Level) {
        if l == Level::Uniform {
            return;
        }
        let written: Vec<u32> = self.log[from..].to_vec();
        for v in written {
            if trace_var() == Some(v) {
                eprintln!("rejoin {v:#x} at {l:?} from {} of {} by {}", from, self.log.len(), std::panic::Location::caller().line());
            }
            let size = self.sizes.get(&v).copied().unwrap_or(u32::MAX);
            self.local.add(v, (0, size, l));
        }
    }

    /// Bytes `at` of variable `v` (all of it if `None`) written with a value of level `l`.
    fn set(&mut self, v: u32, at: Option<(u32, u32)>, l: Level) {
        if l == Level::Uniform {
            return;
        }
        let (off, end) = at.unwrap_or((0, u32::MAX));
        let cs = self.vars.entry(v).or_default();
        match cs.iter_mut().find(|c| c.off == off && c.end == end) {
            Some(c) if c.l.join(l) != c.l => {
                c.l = c.l.join(l);
                self.changed = true;
                self.mark(v);
            }
            Some(_) => {}
            None => {
                cs.push(Cell { off, end, l });
                self.changed = true;
                self.mark(v);
            }
        }
    }

    /// That instance `self.cur` reads variable `v` of another's: a change to it walks `self.cur` again.
    fn note(&self, v: u32) {
        if v >> 20 != self.cur {
            self.reads.borrow_mut().push(v);
        }
    }

    /// Variable `v`'s levels or pointers changed: its instance and those that read it are walked again.
    fn mark(&mut self, v: u32) {
        self.dirty.insert(v >> 20);
        if let Some(rs) = self.readers.get(&v) {
            self.dirty.extend(rs.iter().copied());
        }
    }

    fn var(&self, v: u32) -> Level {
        self.get(v, None)
    }

    fn raise(&mut self, v: u32, l: Level) {
        self.set(v, None, l);
    }

    /// That variable `v`'s pointers may point into `ts` too. One target a variable pointed into: a second offset in
    /// it widens it to anywhere in it, so a pointer stepped in a loop (`p += 4`) reaches the fixed point.
    fn point(&mut self, v: u32, ts: &[Target]) {
        let e = self.pts.entry(v).or_default();
        for &(var, off) in ts {
            match e.iter_mut().find(|t| t.0 == var) {
                Some(t) if t.1 == off || t.1.is_none() => {}
                Some(t) => {
                    t.1 = None;
                    self.changed = true;
                    self.dirty.insert(u32::MAX);
                }
                None => {
                    e.push((var, off));
                    self.changed = true;
                    self.dirty.insert(u32::MAX);
                }
            }
        }
        if self.dirty.remove(&u32::MAX) {
            self.mark(v);
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
            Kind::Call => {
                let v = var(self.instance_of(b, a), RESULT);
                self.note(v);
                self.pts.get(&v).cloned().unwrap_or_default()
            }
            _ => Vec::new(),
        }
    }

    /// The variables whose bytes address `a` reaches: its own, an inlined call's result's (an aggregate's), or
    /// those its pointers point into.
    fn holds(&self, b: &Builder, a: usize) -> Vec<Target> {
        match direct(b, a) {
            Some(v) => vec![(v, None)],
            None if kind(b, a) == Kind::Call => vec![(var(self.instance_of(b, a), RESULT), Some(0))],
            None => self.targets(b, a),
        }
    }

    /// Where the pointers held at address `a` point.
    fn held(&self, b: &Builder, a: usize) -> Vec<Target> {
        self.holds(b, a)
            .iter()
            .flat_map(|(v, _)| {
                self.note(*v);
                self.pts.get(v).cloned().unwrap_or_default()
            })
            .collect()
    }

    /// The body call `n` is put in: where the walk put it (a round's), else its own.
    fn body_of(&self, b: &Builder, n: usize) -> u32 {
        self.assign.get(&n).copied().unwrap_or_else(|| callee(b, n))
    }

    fn instance_of(&self, b: &Builder, n: usize) -> u32 {
        self.body_of(b, n) + 1
    }

    /// Walks instance `inst`: the kernel's (0), or a function's body (its number plus 1) under the control it is
    /// called under, its own region the returns'.
    fn walk(&mut self, b: &Builder, inst: u32) {
        if inst == 0 {
            (self.cur, self.local) = (0, State::new());
            self.stmt(b, kids(b, 0)[0], Level::Uniform);
            return;
        }
        let id = inst - 1;
        let body = self.bodies[&id];
        let under = self.called.get(&id).copied().unwrap_or(Level::Uniform);
        // Outside it, the returns pending at its calls (a region no variable is of).
        let pending = self.pending.get(&id).copied().unwrap_or(Level::Uniform);
        // Its levels are relative to its entry: the lanes that call it. What the caller's control and its
        // pending returns make of its result and its writes is the caller's (they are the caller's
        // variables there, rejoined where its control does); the control it is called under stays its
        // node's level (the barrier rule's, `check`).
        let _ = pending;
        let outer = std::mem::replace(&mut self.regions, vec![(u32::MAX, Level::Uniform), (id + 1, Level::Uniform)]);
        (self.cur, self.local) = (id + 1, self.incoming.get(&id).cloned().unwrap_or_default());
        self.returns.clear();
        let from = self.log.len();
        self.stmt(b, kids(b, body)[0], Level::Uniform);
        let foreign: Vec<u32> = self.log[from..].iter().copied().filter(|v| v >> 20 != id + 1).collect();
        let w = self.writes.entry(id).or_default();
        let mut grew = false;
        for v in foreign {
            grew |= w.insert(v);
        }
        let mut exit = std::mem::take(&mut self.local);
        for s in std::mem::take(&mut self.returns) {
            join_into(&mut exit, &s);
        }
        exit.retain(|v| v >> 20 != id + 1);
        let e = self.effects.entry(id).or_default();
        for (v, cs) in exit.iter() {
            let to = e.at(v);
            for c in cs {
                grew |= to.insert(*c);
            }
        }
        // Its callers read what it writes and leaves.
        if grew {
            self.changed = true;
            if let Some(cs) = self.callers.get(&id) {
                self.dirty.extend(cs.iter().copied());
            }
        }
        self.regions = outer;
        self.lev[body] = under;
    }

    /// The analysis to its fixed point: a worklist of the instances whose walk reads what changed (each walked once
    /// to start, the kernel first, then the bodies, callers before callees), then whole passes until one changes
    /// nothing. The first pass finds nothing to change unless a walk read what its dependencies (`note`, `mark`) miss:
    /// then the passes go on, as they would alone.
    fn run(&mut self, b: &Builder) {
        let mut ids: Vec<u32> = self.bodies.keys().copied().collect();
        ids.sort_unstable();
        self.dirty = std::iter::once(0).chain(ids.iter().map(|i| i + 1)).collect();
        let mut walks = 0;
        while let Some(i) = self.dirty.pop_first() {
            walks += 1;
            self.walk(b, i);
            let mut rs = std::mem::take(self.reads.get_mut());
            rs.sort_unstable();
            rs.dedup();
            for v in rs {
                let r = self.readers.entry(v).or_default();
                if !r.contains(&i) {
                    r.push(i);
                }
            }
        }
        let mut passes = 0;
        loop {
            passes += 1;
            self.changed = false;
            self.stmt(b, 0, Level::Uniform);
            if !self.changed {
                break;
            }
        }
        self.dirty.clear();
        if std::env::var("RERAIL_UKL_VEC_TRACE").is_ok() {
            eprintln!("  {walks} walks of {} instances, then {passes} passes", ids.len() + 1);
        }
    }

    /// If `a` addresses a variable of the frame directly (a `Local`, plus offsets): the variable, the offset in it
    /// if it is constant, and the offset's level (each lane's own copy read at it).
    fn local(&mut self, b: &Builder, a: usize, ctl: Level) -> Option<(u32, Option<u32>, Level)> {
        match kind(b, a) {
            Kind::Local => {
                // Its address is each lane's own; its value, the variable's.
                self.lev[a] = Level::Varying;
                self.sizes.insert(b.word(a, 3), b.word(a, 1));
                Some((b.word(a, 3), Some(b.word(a, 2)), Level::Uniform))
            }
            Kind::Bin if matches!(Op::of(b.word(a, 0)), Op::Add | Op::Sub) && M(b.word(a, 1)) == M::PTR => {
                let k = kids(b, a);
                let (v, off, l) = self.local(b, k[0], ctl)?;
                let o = self.expr(b, k[1], ctl);
                self.lev[a] = Level::Varying;
                let add = Op::of(b.word(a, 0)) == Op::Add;
                let off = off.zip(constant(b, k[1])).map(|(f, c)| if add { f.wrapping_add(c as u32) } else { f.wrapping_sub(c as u32) });
                Some((v, off, l.join(o)))
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
        ps.into_iter().fold(mem.unwrap_or(Level::Uniform), |l, (v, off, o)| l.join(o).join(self.get(v, off.map(|f| (f, f.saturating_add(size))))))
    }

    /// The levels of the `size` bytes at address `a`, by range from its start.
    fn cells_at(&mut self, b: &Builder, a: usize, size: u32, ctl: Level) -> Vec<Cell> {
        if kind(b, a) == Kind::Call {
            self.expr(b, a, ctl);
            let v = var(self.instance_of(b, a), RESULT);
            self.note(v);
            return self.vars.get(&v).map_or(Vec::new(), |cs| cs.iter().filter(|c| c.off < size).map(|c| Cell { off: c.off, end: c.end.min(size), l: c.l }).collect());
        }
        let (ps, mem) = self.places(b, a, ctl);
        if let ([(v, Some(f), o)], None | Some(Level::Uniform)) = (ps.as_slice(), mem) {
            let (v, f, o) = (*v, *f, *o);
            let end = f.saturating_add(size);
            self.note(v);
            let mut out: Vec<Cell> =
                self.vars.get(&v).map_or(Vec::new(), |cs| cs.iter().filter(|c| c.off < end && f < c.end).map(|c| Cell { off: c.off.max(f) - f, end: c.end.min(end) - f, l: c.l }).collect());
            if let Some(cs) = self.local.get(&v) {
                out.extend(cs.iter().filter(|c| c.0 < end && f < c.1).map(|c| Cell { off: c.0.max(f) - f, end: c.1.min(end) - f, l: c.2 }));
            }
            if o > Level::Uniform {
                out.push(Cell { off: 0, end: size, l: o });
            }
            return out;
        }
        let l = ps.into_iter().fold(mem.unwrap_or(Level::Uniform), |l, (v, off, o)| l.join(o).join(self.get(v, off.map(|f| (f, f.saturating_add(size))))));
        vec![Cell { off: 0, end: size, l }]
    }

    /// A write of `size` bytes, of levels `cells`, at address `a` under `ctl`, the value's pointers into `ts`, where
    /// the walk is. At one known place: a store there (its control is the rejoining's, `rejoin`). Anywhere else: a
    /// raise by the control. Another body's variable: by the returns taken so far too (the lanes that took them do
    /// not write it).
    fn write(&mut self, b: &Builder, a: usize, size: u32, cells: &[Cell], ctl: Level, ts: &[Target]) {
        let most = Level::all(cells.iter().map(|c| c.l));
        let (ps, _) = self.places(b, a, ctl);
        let one = ps.len() == 1;
        let ret = self.regions.last().map_or(Level::Uniform, |r| r.1);
        for (v, off, o) in ps {
            let r = if v >> 20 == self.cur { Level::Uniform } else { ret };
            if trace_var() == Some(v) {
                eprintln!("write {v:#x} at {a} in {} under {ctl:?}: address {o:?}, returns {r:?}, one place {one}, off {off:?}, cells {:?}", self.cur, cells.iter().map(|c| c.l).collect::<Vec<_>>());
            }
            match off {
                Some(f) if one && o == Level::Uniform && r == Level::Uniform => self.store_local(v, f, size, cells),
                Some(f) => {
                    let under = o.join(ctl).join(r);
                    for c in cells {
                        self.raise_local(v, Some((f + c.off, f.saturating_add(c.end))), c.l.join(under));
                    }
                    self.raise_local(v, Some((f, f.saturating_add(size))), under);
                }
                None => self.raise_local(v, None, most.join(o).join(ctl).join(r)),
            }
            self.point(v, ts);
        }
    }

    /// The variables of other bodies a call `n`'s arguments reach (its callee's `body`): those a pointer argument
    /// points into, those an aggregate's pointers do (the aggregate itself is copied), and what pointers held there
    /// point into.
    fn reached(&self, b: &Builder, n: usize, body: usize) -> Vec<u32> {
        let mut out: Vec<u32> = Vec::new();
        for (j, a) in kids(b, n).into_iter().enumerate() {
            let ts = if M(b.word(body, 8 + 2 * j)).is_agg() { self.held(b, a) } else { self.targets(b, a) };
            for (v, _) in ts {
                if !out.contains(&v) {
                    out.push(v);
                }
            }
        }
        let mut i = 0;
        while i < out.len() {
            self.note(out[i]);
            for &(v, _) in self.pts.get(&out[i]).map_or(&[][..], |t| t.as_slice()) {
                if !out.contains(&v) {
                    out.push(v);
                }
            }
            i += 1;
        }
        out
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
                let r = x.join(old);
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
                Level::all(cells.iter().map(|c| c.l))
            }
            Kind::Bin | Kind::Un | Kind::Cast => {
                let ls: Vec<Level> = k.iter().map(|&c| self.expr(b, c, ctl)).collect();
                op_level(b, n, &ls)
            }
            Kind::Comma => {
                self.expr(b, k[0], ctl);
                self.expr(b, k[1], ctl)
            }
            Kind::Cond => {
                let c = self.expr(b, k[0], ctl);
                let (s0, from) = (self.local.clone(), self.log.len());
                let x = self.expr(b, k[1], ctl.join(c));
                let then = std::mem::replace(&mut self.local, s0);
                let y = self.expr(b, k[2], ctl.join(c));
                join_into(&mut self.local, &then);
                self.rejoin(from, c);
                c.join(x).join(y)
            }
            Kind::Logic => {
                let x = self.expr(b, k[0], ctl);
                let (s0, from) = (self.local.clone(), self.log.len());
                let y = self.expr(b, k[1], ctl.join(x));
                join_into(&mut self.local, &s0);
                self.rejoin(from, x);
                x.join(y)
            }
            Kind::Builtin => {
                let a: Vec<Level> = k.iter().map(|&c| self.expr(b, c, ctl)).collect();
                let x = Builtin::of(b.word(n, 0));
                let most = Level::all(a.iter().copied());
                match x {
                    // A 1-D group's thread id: its SIMD group's and lane's apart (`run` refuses another group's
                    // shape for a kernel that takes it so); the lane's index, the lane's.
                    Builtin::LocalId if constant(b, k[0]) == Some(0) => Level::Lid,
                    // A thread's id in the second or third dimension: its SIMD group's (`run` takes such a kernel only
                    // in a group whose SIMD groups each lie in one row of the first dimension).
                    Builtin::LocalId | Builtin::GlobalId if matches!(constant(b, k[0]), Some(1 | 2)) => Level::SimdGroup,
                    Builtin::SubGroupLocalId => Level::Lane,
                    Builtin::LocalId | Builtin::GlobalId => Level::Varying,
                    Builtin::LocalSize | Builtin::GlobalSize | Builtin::GroupId | Builtin::NumGroups | Builtin::Barrier | Builtin::MemFence | Builtin::SubGroupBarrier | Builtin::Yield => Level::Uniform,
                    // A SIMD group's ballot or reduction: its own; of what is alike at a lane in every SIMD group, the
                    // whole group's.
                    Builtin::SubGroupBallot | Builtin::SubGroupReduceAdd | Builtin::SubGroupReduceMin | Builtin::SubGroupReduceMax => match a[0] {
                        l if l <= Level::Agreed => l,
                        Level::Lane => Level::Agreed,
                        _ => Level::SimdGroup,
                    },
                    // A lane's value read from lane `a[1]`: a uniform value; a SIMD group's own; one alike at a lane
                    // in every SIMD group, read from one lane (the whole group's) or from a lane alike so (alike so);
                    // any value, from one lane (the SIMD group's).
                    Builtin::SubGroupShuffle => match (a[0], a[1]) {
                        (x, _) if x <= Level::SimdGroup => x,
                        (Level::Lane, i) if i <= Level::Agreed => Level::Agreed,
                        (Level::Lane, Level::Lane) => Level::Lane,
                        (_, i) if i <= Level::SimdGroup => Level::SimdGroup,
                        _ => Level::Varying,
                    },
                    // A shift or a butterfly by a uniform distance: a function of the lane, if the value was.
                    Builtin::SubGroupShuffleUp | Builtin::SubGroupShuffleDown | Builtin::SubGroupShuffleXor => match (a[0], a[1]) {
                        (x, _) if x <= Level::SimdGroup => x,
                        (Level::Lane, i) if i <= Level::Agreed => Level::Lane,
                        _ => Level::Varying,
                    },
                    _ if is_atomic(x) => {
                        if (x == Builtin::AtomicLoad || k.len() == 2 && identity(b, x, k[1])) && a[0] <= Level::SimdGroup {
                            a[0].join(Level::Agreed)
                        } else {
                            Level::Varying
                        }
                    }
                    _ => most,
                }
            }
            Kind::Call => {
                // Its arguments' levels, its context: then the body it is put in (a round's: its context's, if made),
                // and its arguments stored in that body's parameters. A parameter is the callee's: only the lanes that
                // call it write it, and read it (its body's levels are relative to its entry: the caller's control is
                // the caller's to rejoin).
                let own = self.bodies[&callee(b, n)];
                let mut args = Vec::new();
                let mut context = Vec::new();
                for (j, &a) in k.iter().enumerate() {
                    let pm = M(b.word(own, 8 + 2 * j));
                    if pm.is_agg() {
                        let cells = self.cells_at(b, a, pm.bytes(), ctl);
                        context.push(Level::all(cells.iter().map(|c| c.l)));
                        args.push((cells, Level::Uniform, self.held(b, a)));
                    } else {
                        let x = self.expr(b, a, ctl);
                        context.push(x);
                        args.push((Vec::new(), x, self.targets(b, a)));
                    }
                }
                let f = b.word(n, 0);
                let id = if self.dynamic {
                    // The call's context at its last visit (its levels' fixed point) is the one it wants: those it
                    // passes on the way are not.
                    match self.ids.get(&(f, context.clone())) {
                        Some(&s) => {
                            self.wanted.remove(&n);
                            s
                        }
                        None => {
                            self.wanted.insert(n, context.clone());
                            callee(b, n)
                        }
                    }
                } else {
                    callee(b, n)
                };
                self.assign.insert(n, id);
                let (inst, body) = (id + 1, self.bodies[&id]);
                for (j, (cells, x, ts)) in args.into_iter().enumerate() {
                    let (v, pm) = (var(inst, j as u32), M(b.word(body, 8 + 2 * j)));
                    if pm.is_agg() {
                        for c in cells {
                            self.set(v, Some((c.off, c.end)), c.l);
                        }
                    } else {
                        self.raise(v, x);
                    }
                    self.point(v, &ts);
                }
                self.contexts.insert(n, context);
                // The callee's entry state: the caller's, in what it reaches; what it leaves there, after.
                let reached = self.reached(b, n, body);
                let ret = self.regions.last().map_or(Level::Uniform, |r| r.1);
                self.callers.entry(id).or_default().insert(self.cur);
                {
                    let into = self.incoming.entry(id).or_default();
                    let mut grew = false;
                    for v in &reached {
                        if let Some(cs) = self.local.get(v)
                            && !into.get(v).is_some_and(|e| cs.is_subset(e))
                        {
                            let e = into.at(*v);
                            for c in cs {
                                grew |= e.insert(*c);
                            }
                        }
                    }
                    if grew {
                        self.changed = true;
                        self.dirty.insert(id + 1);
                    }
                }
                // What it writes there, as it leaves it; what it does not write, as it was.
                let written = self.writes.get(&id).cloned().unwrap_or_default();
                for v in reached.into_iter().filter(|v| written.contains(v)) {
                    let mut out = self.effects.get(&id).and_then(|e| e.get(&v)).cloned().unwrap_or_default();
                    if v >> 20 != self.cur && ret > Level::Uniform {
                        out.insert((0, self.sizes.get(&v).copied().unwrap_or(u32::MAX), ret));
                    }
                    if trace_var() == Some(v) {
                        eprintln!("call {n} of {} (body {id}) reaches it: {out:?}", b.word(n, 0));
                    }
                    self.local.insert(v, out);
                    self.log.push(v);
                }
                let c = self.called.entry(id).or_insert(Level::Uniform);
                if c.join(ctl) != *c {
                    *c = c.join(ctl);
                    self.changed = true;
                    self.dirty.insert(id + 1);
                }
                // The returns pending here: those of every region walked (a variable outside them all's).
                let r = Level::all(self.regions.iter().map(|r| r.1));
                let pending = self.pending.entry(id).or_insert(Level::Uniform);
                if pending.join(r) != *pending {
                    *pending = pending.join(r);
                    self.changed = true;
                    self.dirty.insert(id + 1);
                }
                let m = M(b.word(n, 1));
                // An aggregate's value is its copy's address (its bytes' levels are its result's, `cells_at`).
                if m == M::VOID || m.is_agg() { Level::Uniform } else { self.var(var(inst, RESULT)) }
            }
            Kind::Block | Kind::Expr => {
                self.stmt(b, n, ctl);
                Level::Uniform
            }
            _ => Level::Uniform,
        };
        // The tape's level is the value's own: a lane's own variable's address differs in each lane.
        let own = kind(b, n) == Kind::Local || (kind(b, n) == Kind::Call && M(b.word(n, 1)).is_agg()) || !self.targets(b, n).is_empty();
        self.lev[n] = if own { Level::Varying } else { l };
        l
    }

    fn stmt(&mut self, b: &Builder, n: usize, ctl: Level) -> Exits {
        self.stmt_in(b, n, ctl, Exits::default())
    }

    /// Statement `n` under `ctl`, after the exits `before` in the construct it is in: the exits it takes.
    fn stmt_in(&mut self, b: &Builder, n: usize, ctl: Level, before: Exits) -> Exits {
        let top = self.regions.len() - 1;
        let saved = self.regions[top].1;
        self.regions[top].1 = saved.join(before.ret.unwrap_or(Level::Uniform));
        let ex = self.stmt_under(b, n, ctl, before);
        self.regions[top].1 = saved;
        ex
    }

    fn stmt_under(&mut self, b: &Builder, n: usize, ctl: Level, before: Exits) -> Exits {
        // Lanes that left by a `break` or `continue` come back: what they did not write differs from what the others
        // did.
        let ctl = ctl.join(join(before.brk, before.cont).unwrap_or(Level::Uniform));
        let k = kids(b, n);
        let ex = match kind(b, n) {
            Kind::Func => {
                self.walk(b, 0);
                let mut ids: Vec<u32> = self.bodies.keys().copied().collect();
                ids.sort_unstable();
                for id in ids {
                    self.walk(b, id + 1);
                }
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
                let inner = ctl.join(c);
                let (s0, from) = (self.local.clone(), self.log.len());
                let mut ex = self.stmt(b, k[1], inner);
                let then = std::mem::replace(&mut self.local, s0);
                if k.len() > 2 {
                    ex = ex.join(self.stmt(b, k[2], inner));
                }
                join_into(&mut self.local, &then);
                self.rejoin(from, c);
                self.lev[n] = c;
                return ex;
            }
            Kind::While | Kind::For | Kind::Do => {
                let (cond, body) = if kind(b, n) == Kind::Do { (k[1], k[0]) } else { (k[0], k[1]) };
                let do_ = kind(b, n) == Kind::Do;
                // The loop's lanes leave at different iterations if its condition varies, or some break. Its state at
                // the top: what comes in, joined with what comes round (the iteration's end, its continues), to a
                // fixed point; past it, the lanes that left by the condition's and by the breaks'.
                let mut lc = ctl;
                let mut ex;
                let from = self.log.len();
                let mut top = self.local.clone();
                let out = loop {
                    self.local = top.clone();
                    let iteration = self.log.len();
                    self.leaving.push(Leaving { looping: true, ..Default::default() });
                    let mut c = Level::Uniform;
                    if !do_ {
                        c = self.expr(b, cond, lc);
                    }
                    let left = self.local.clone();
                    ex = self.stmt(b, body, lc.join(c));
                    let l = self.leaving.pop().expect("the loop's exits");
                    for s in &l.cont {
                        join_into(&mut self.local, s);
                    }
                    // The lanes that continued come back: what the iteration wrote after they left differs from
                    // theirs by the level that sent them (a strong store keeps its value's level, the same in the
                    // lanes that went on as in those that did not).
                    if let Some(cl) = ex.cont {
                        self.rejoin(iteration, cl);
                    }
                    let mut out = left;
                    if do_ {
                        c = self.expr(b, cond, lc);
                        out = self.local.clone();
                    }
                    if kind(b, n) == Kind::For {
                        self.stmt_in(b, k[2], lc.join(c), Exits { cont: None, ..ex });
                    }
                    for s in &l.brk {
                        join_into(&mut out, s);
                    }
                    let mut next_top = top.clone();
                    join_into(&mut next_top, &self.local);
                    let next = lc.join(c).join(ex.brk.unwrap_or(Level::Uniform));
                    if next == lc && next_top == top {
                        break out;
                    }
                    (lc, top) = (next, next_top);
                };
                self.local = out;
                if let Some(t) = trace_var()
                    && self.log[from..].contains(&t)
                {
                    eprintln!("loop {} ({}) ctl {ctl:?} lc {lc:?} brk {:?}", n, b.word(n, 0), ex.brk);
                }
                self.rejoin(from, lc);
                self.lev[n] = lc;
                return Exits { ret: ex.ret, brk: None, cont: None };
            }
            Kind::Switch => {
                // Each case under the value's level, after the exits the cases before it took (their breaks' lanes
                // skip the rest); its breaks end here, its continues and returns go on.
                let c = self.expr(b, k[0], ctl);
                let inner = ctl.join(c);
                let mut ex = Exits::default();
                // A case's lanes: those whose value it is (the state coming in), and those falling through.
                let (s0, from) = (self.local.clone(), self.log.len());
                self.leaving.push(Leaving::default());
                for &case in &k[1..] {
                    join_into(&mut self.local, &s0);
                    ex = ex.join(self.stmt_in(b, case, inner, ex));
                }
                let l = self.leaving.pop().expect("the switch's exits");
                join_into(&mut self.local, &s0);
                for s in &l.brk {
                    join_into(&mut self.local, s);
                }
                // The lanes that broke out of a case come back, as a loop's that continued do.
                self.rejoin(from, c.join(ex.brk.unwrap_or(Level::Uniform)));
                self.lev[n] = c;
                return Exits { brk: None, ..ex };
            }
            Kind::Case => self.stmt(b, k[0], ctl),
            Kind::Return => {
                if b.word(n, 0) != 0 {
                    let x = self.expr(b, k[0], ctl);
                    let slot = b.word(n, 2);
                    if slot != NO_SLOT {
                        let (v, m) = (b.word(n, 4), M(b.word(n, 3)));
                        // The result is its caller's: the lanes that returned before wrote theirs.
                        let ctl = ctl.join(self.regions.last().map_or(Level::Uniform, |r| r.1));
                        if m.is_agg() {
                            // The aggregate's bytes, from the address returned, into the call's result.
                            let size = m.bytes();
                            for c in self.cells_at(b, k[0], size, ctl) {
                                self.set(v, Some((c.off, c.end)), c.l.join(ctl));
                            }
                            self.set(v, None, ctl);
                            let ts = self.held(b, k[0]);
                            self.point(v, &ts);
                        } else {
                            self.raise(v, x.join(ctl));
                            let ts = self.targets(b, k[0]);
                            self.point(v, &ts);
                        }
                    }
                }
                let st = self.local.clone();
                self.returns.push(st);
                Exits { ret: Some(ctl), ..Default::default() }
            }
            Kind::Break => {
                let st = self.local.clone();
                if let Some(l) = self.leaving.last_mut() {
                    l.brk.push(st);
                }
                Exits { brk: Some(ctl), ..Default::default() }
            }
            Kind::Continue => {
                let st = self.local.clone();
                if let Some(l) = self.leaving.iter_mut().rev().find(|l| l.looping) {
                    l.cont.push(st);
                }
                Exits { cont: Some(ctl), ..Default::default() }
            }
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
        // A function's body: what follows any of its calls follows it. To a fixed point, bodies calling bodies.
        let mut follows: HashMap<u32, bool> = HashMap::new();
        loop {
        let mut next: HashMap<u32, bool> = HashMap::new();
        let mut stack: Vec<(usize, bool)> = kids(b, 0).into_iter().map(|c| (c, kind(b, c) == Kind::Body && follows.get(&b.word(c, 0)).copied().unwrap_or(false))).collect();
        while let Some((i, a)) = stack.pop() {
            after[i] = a;
            let k = kids(b, i);
            match kind(b, i) {
                Kind::Call => {
                    *next.entry(callee(b, i)).or_default() |= a;
                    // Its arguments, then its callee's body, then what follows it.
                    let mut later = a || self.sync[self.bodies[&callee(b, i)]];
                    for &c in k.iter().rev() {
                        stack.push((c, later));
                        later |= self.sync[c];
                    }
                }
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
        if next == follows {
            break;
        }
        follows = next;
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
            Kind::If | Kind::While | Kind::For | Kind::Do | Kind::Break | Kind::Continue | Kind::Switch => self.b.word(n, 0),
            Kind::Cond => return "a ?: expression".into(),
            Kind::Call => return format!("the call of {}", self.p.funcs[self.b.word(n, 0) as usize].name),
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
            Kind::Switch => {
                // As an `if`'s branches: the lanes a case leaves out are in the others, then past the switch.
                self.walk(k[0]);
                let l = self.a.lev[k[0]];
                for (j, &c) in k[1..].iter().enumerate() {
                    if l > Level::Uniform {
                        let other = k[1..].iter().enumerate().any(|(x, &o)| x != j && self.a.sync[o]);
                        self.divs.push(Div { level: l, node: n, exit: false, cause: None, allowed: !other && !self.after[n], until: n });
                    }
                    self.walk(c);
                    self.divs.retain(|d| d.exit || d.node != n);
                }
                // Its breaks rejoin here.
                self.divs.retain(|d| d.until != n);
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
            Kind::Func => {
                // The kernel's body, then each function's: its own divergences from its start (its calls'
                // are met at each call).
                for &c in &k {
                    self.divs.clear();
                    self.walk(c);
                }
            }
            Kind::Body => {
                for &c in &k {
                    self.walk(c);
                }
                // Its returns rejoin here.
                self.divs.retain(|d| d.until != n);
            }
            Kind::Call => {
                for &c in &k {
                    self.walk(c);
                }
                // Its callee's sync points, met here by the lanes that call it.
                if self.a.sync[n] {
                    self.meet(n, self.a.gsync[n]);
                }
            }
            Kind::Return | Kind::Break | Kind::Continue => {
                for &c in &k {
                    self.walk(c);
                }
                // Where its lanes rejoin: its call's end, its loop's. The kernel's own return ends them.
                let until = match kind(b, n) {
                    Kind::Return => self.region(n),
                    Kind::Break => Some(self.enclosing_breakable(n)),
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
                    self.meet(n, x == Builtin::Barrier);
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

    /// A development trace (`RERAIL_UKL_VEC_WHY`): node `n`'s subtree, each node's level, each variable's cells.
    fn why(&self, n: usize, depth: usize) {
        let b = self.b;
        let words: Vec<u32> = (0..4).map(|k| if k < 4 { b.word(n, k) } else { 0 }).collect();
        let cells = if kind(b, n) == Kind::Local { format!(" var {:#x} {:?}", b.word(n, 3), self.a.vars.get(&b.word(n, 3)).map(|cs| cs.iter().map(|c| (c.off, c.end, c.l)).collect::<Vec<_>>())) } else { String::new() };
        eprintln!("{:w$}{n} {:?} {:?} {:?}{cells}", "", kind(b, n), self.a.lev[n], words, w = 2 * depth);
        if depth < 12 {
            for c in kids(b, n) {
                self.why(c, depth + 1);
            }
        }
    }

    /// Where a `break` goes: the innermost loop or `switch` around it.
    fn enclosing_breakable(&self, n: usize) -> usize {
        self.ancestors(n).into_iter().find(|&a| matches!(kind(self.b, a), Kind::While | Kind::For | Kind::Do | Kind::Switch)).unwrap_or(0)
    }

    fn region(&self, n: usize) -> Option<usize> {
        self.ancestors(n).into_iter().find(|&a| kind(self.b, a) == Kind::Body)
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

    /// Sync point `n` met under the divergences pending: a threadgroup barrier (`barrier`: there, or in a call's
    /// callee), else a SIMD-group operation.
    fn meet(&mut self, n: usize, barrier: bool) {
        for d in self.divs.clone() {
            // A SIMD-group operation needs the SIMD group's lanes together: a decision the same in it. A threadgroup
            // barrier needs the group's: one the same in each SIMD group (`Agreed`, by the kernel's protocol, or
            // `SimdGroup`, each its own: a group of one SIMD group's `ukl_tg_sum`), on which the rule rests, checked
            // where control turns on it (`run`'s `agree`: never a barrier passed apart).
            if d.level <= Level::SimdGroup {
                if barrier && d.level >= Level::Agreed {
                    self.checks.push(d.cause.unwrap_or(d.node));
                }
                continue;
            }
            if d.allowed {
                continue;
            }
            if std::env::var("RERAIL_UKL_VEC_WHY").is_ok() {
                let at = d.cause.unwrap_or(d.node);
                eprintln!("refused {} by {at}; around it:", d.node);
                for x in self.ancestors(d.node) {
                    let k = kind(self.b, x);
                    if matches!(k, Kind::Body | Kind::For | Kind::While | Kind::Do | Kind::If | Kind::Call) {
                        let f = if k == Kind::Body { self.p.funcs[self.b.word(x, 1) as usize].name.clone() } else { String::new() };
                        eprintln!("  {x} {k:?} {:?} {} {f}", self.a.lev[x], self.site(x));
                    }
                }
                self.why(kids(self.b, at).first().copied().unwrap_or(at), 0);
            }
            if let Ok(t) = std::env::var("RERAIL_UKL_VEC_WHY_TARGET") {
                let t = u32::from_str_radix(t.trim_start_matches("0x"), 16).unwrap_or(0);
                for (v, ts) in &self.a.pts {
                    if ts.iter().any(|x| x.0 == t) {
                        eprintln!("target of {v:#x}: {:?}; its cells {:?}", ts, self.a.vars.get(v).map(|cs| cs.iter().map(|c| (c.off, c.end, c.l)).collect::<Vec<_>>()));
                    }
                }
                for (k, &body) in kids(self.b, 0).iter().enumerate().skip(1) {
                    let _ = k;
                    eprintln!("body {} = {} called {:?} pending {:?}", self.b.word(body, 0), self.p.funcs[self.b.word(body, 1) as usize].name, self.a.called.get(&self.b.word(body, 0)), self.a.pending.get(&self.b.word(body, 0)));
                }
            }
            if let Ok(ns) = std::env::var("RERAIL_UKL_VEC_WHY_NODES") {
                for n in ns.split(',').filter_map(|x| x.parse::<usize>().ok()) {
                    eprintln!("node {n}: {}", self.site(n));
                    self.why(kids(self.b, n).first().copied().unwrap_or(n), 0);
                }
            }
            if let Some(v) = trace_var() {
                for i in 0..self.b.len() {
                    if matches!(kind(self.b, i), Kind::Store | Kind::Copy | Kind::Update) {
                        let t = kids(self.b, i)[0];
                        if kind(self.b, t) == Kind::Local && self.b.word(t, 3) == v {
                            eprintln!("written by {i}:");
                            self.why(i, 0);
                        }
                    }
                }
                eprintln!("pts of {v:#x}: {:?}", self.a.pts.get(&v));
            }
            if let Ok(f) = std::env::var("RERAIL_UKL_VEC_WHY_CALLS") {
                for i in 0..self.b.len() {
                    if kind(self.b, i) == Kind::Call && self.p.funcs[self.b.word(i, 0) as usize].name == f {
                        eprintln!("call {i} at {} body {} level {:?} context {:?} result {:?}", self.site(i), callee(self.b, i), self.a.lev[i], self.a.contexts.get(&i), self.a.var(var(callee_instance(self.b, i), RESULT)));
                    }
                }
            }
            if let Ok(i) = std::env::var("RERAIL_UKL_VEC_WHY_INSTANCE") {
                let i = u32::from_str_radix(i.trim_start_matches("0x"), 16).unwrap_or(0);
                let mut vs: Vec<_> = self.a.vars.iter().filter(|(v, _)| *v >> 20 == i).collect();
                vs.sort_by_key(|(v, _)| **v);
                for (v, cs) in vs {
                    eprintln!("var {v:#x}: {:?} pts {:?}", cs.iter().map(|c| (c.off, c.end, c.l)).collect::<Vec<_>>(), self.a.pts.get(v));
                }
                let mut ps: Vec<_> = self.a.pts.iter().filter(|(v, _)| *v >> 20 == i).collect();
                ps.sort_by_key(|(v, _)| **v);
                for (v, ts) in ps {
                    let named: Vec<_> = ts.iter().map(|(t, o)| (format!("{t:#x}"), *o, self.a.vars.get(t).map(|cs| cs.iter().map(|c| (c.off, c.end, c.l)).collect::<Vec<_>>()))).collect();
                    eprintln!("pts {v:#x}: {named:?}");
                }
                eprintln!("called {:?} pending {:?}", self.a.called.get(&(i - 1)), self.a.pending.get(&(i - 1)));
            }
            let whose = if barrier { "its threadgroup's threads" } else { "its SIMD group's lanes" };
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

/// The nodes kernel `kernel`'s vector tape would have, every call inlined (for measuring what inlining costs).
pub fn inlined_size(p: &Program, kernel: u32) -> u128 {
    let kv = View::new(&p.tape);
    let mut memo: HashMap<usize, u128> = HashMap::new();
    fn size(p: &Program, kv: &View, n: usize, memo: &mut HashMap<usize, u128>) -> u128 {
        let mut s = 1;
        if kv.kind(n) == Kind::Call
            && let Some(callee) = p.funcs[kv.w(n, 0) as usize].node
        {
            if let Some(&x) = memo.get(&callee) {
                s += x;
            } else {
                let x = size(p, kv, callee, memo);
                memo.insert(callee, x);
                s += x;
            }
        }
        for c in kv.children(n) {
            s += size(p, kv, c, memo);
        }
        s
    }
    p.funcs[kernel as usize].node.map_or(0, |n| size(p, &kv, n, &mut memo))
}
