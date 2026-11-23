//! UKL as Rust, ahead of time (decision 0567 step 3): a kernel's vector tape written as Rust whose lanes are the
//! target's SIMD registers, each operation its intrinsic (aarch64's NEON, `core::arch::aarch64`), for a comparison
//! with kernels vectorized by hand.
//!
//! The Rust is written from the vector tape, the vectorizer's own form of the kernel (its statements, each value's
//! level), not from an IR made of it: a uniform `if` or loop is Rust's, a varying one a mask of the lanes that take
//! it and a test that any do; a variable a value of every lane (a register a 4 lanes, `[uint32x4_t; P]`) or, if every
//! lane has the same, one; a function of the program a Rust function the lanes' mask is passed to, which the compiler
//! inlines. Memory is read in place: a lane's address of a uniform base and its 32-bit index is a load of the index's
//! element (`ldr w, [x, w, uxtw #2]`), consecutive lanes' a register's (`vld1q_u32`).
//!
//! A kernel is a function of a launch's group: its threads, `W` at a time (`W` lanes, `P = W / 4` registers), each
//! chunk of its first dimension. A threadgroup barrier at the top of the kernel's function parts it in phases: the
//! kernel runs every chunk's phase before any chunk's next (as the group's threads all meet at the barrier), each
//! chunk's lanes that go on and its variables saved at a phase's end and read at the next's start, and its
//! threadgroup memory is the group's array (`tg`). A barrier elsewhere (in a branch, a loop, a function the kernel's
//! calls), a SIMD-group operation, and a thread's variables whose address is taken are refused: their Rust is the
//! next step.

use crate::Error;
use crate::k::{Builtin, Kind, M, NO_SLOT, Op, View};
use std::collections::{BTreeMap, HashMap};

type R<T> = Result<T, Error>;

fn err<T>(what: impl std::fmt::Display) -> R<T> {
    Err(Error(format!("the Rust of a kernel: {what}")))
}

/// A value as the Rust being written holds it.
#[derive(Clone, Debug)]
enum V {
    /// The same in every lane: a Rust expression of `m`'s scalar type (`bool` for `M::B`).
    S(String, M),
    /// Each lane's own: a name of `[uint32x4_t; P]` (32-bit `m`), or of `[uint64x2_t; 2P]` (64-bit `m`, a pointer
    /// too: each lane's address, an integer).
    L(String, M),
    /// Lanes `first + j`, `j` the lane: a thread's id along its group's first dimension.
    Iota(String, M),
    /// A mask (a `bool` of each lane, or a C comparison's 0 or 1): `[uint32x4_t; P]`, a lane all ones where true.
    K(String),
    /// Each lane's address: a uniform base plus its 32-bit index times `scale` bytes.
    At { base: String, idx: Box<V>, scale: u32 },
    /// Lanes `first + j` (`u32`s) compared with one value `y` (unsigned): a mask, which a chunk of consecutive
    /// lanes may decide at once (all on one side of `y`).
    Cmp { op: Op, first: String, y: String },
    /// No value (a call of a function without one).
    None,
}

/// A thread's variable, kept in a Rust variable.
#[derive(Clone, Debug)]
struct Var {
    name: String,
    m: M,
    /// Each lane's (else one, the same in every lane).
    lanes: bool,
    /// A parameter whose value has a shape the caller kept (consecutive lanes, each lane's address): it.
    fixed: Option<V>,
}

/// How a function's parameter is passed: as its calls' arguments all are.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Sh {
    /// One value, the same in every lane.
    S,
    /// Lanes `x + j` (`x` a `u32`).
    Iota,
    /// The thread's id in its launch, `gid0`, which the function has as the kernel does: no parameter.
    Gid,
    /// Each lane's value.
    L,
    /// The kernel's buffer argument `k`, named as the kernel's (`arg{k}`).
    Arg(u32),
    /// One pointer into the kernel's buffer `k` (`arg{k}_{name}`), so its writes are known to be that buffer's.
    In(u32),
    /// Each lane's address: a base (the kernel's buffer `k`, if it is one), and an index of each lane (its own, of
    /// consecutive lanes from a `u32`, or `gid0`), times the scale.
    At(u32, Ix, Option<u32>),
}

/// An index of each lane, as a parameter passes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Ix {
    L,
    Iota,
    Gid,
}

/// The thread's id in its launch's first dimension, as every function names it.
const GID: &str = "gid0";

/// The kernel's buffer argument `s` is, if it is one (`arg{k}`).
fn root(s: &str) -> Option<u32> {
    s.strip_prefix("arg").and_then(|k| k.parse().ok())
}

/// The kernel's buffer argument an address `s` lies in (`arg{k}` plus an offset), if it does.
fn root_of(s: &str) -> Option<u32> {
    let k = s.strip_prefix("arg")?;
    let d = k.find(|c: char| !c.is_ascii_digit()).unwrap_or(k.len());
    (d > 0 && (d == k.len() || k[d..].starts_with(['.', '_']))).then(|| k[..d].parse().ok()).flatten()
}

impl Sh {
    fn code(self) -> String {
        match self {
            Sh::S => "s".into(),
            Sh::Iota => "i".into(),
            Sh::Gid => "g".into(),
            Sh::L => "l".into(),
            Sh::Arg(k) => format!("r{k}"),
            Sh::In(k) => format!("o{k}"),
            Sh::At(k, ix, r) => format!("a{}{k}{}", match ix { Ix::L => "l", Ix::Iota => "i", Ix::Gid => "g" }, r.map(|r| format!("r{r}")).unwrap_or_default()),
        }
    }

    fn of(x: &V) -> Sh {
        match x {
            V::S(s, M::PTR) if root(s).is_some() => Sh::Arg(root(s).unwrap()),
            V::S(s, M::PTR | M::U64) if root_of(s).is_some() => Sh::In(root_of(s).unwrap()),
            V::S(..) | V::None => Sh::S,
            V::Iota(f, _) if f == GID => Sh::Gid,
            V::Iota(..) => Sh::Iota,
            V::At { base, idx, scale } => Sh::At(
                *scale,
                match &**idx {
                    V::Iota(f, _) if f == GID => Ix::Gid,
                    V::Iota(..) => Ix::Iota,
                    _ => Ix::L,
                },
                root(base),
            ),
            V::L(..) | V::K(..) | V::Cmp { .. } => Sh::L,
        }
    }

    /// The kernel's buffer the parameter is (or is an address in), if it is one.
    fn root(self) -> Option<u32> {
        match self {
            Sh::Arg(k) | Sh::At(_, _, Some(k)) => Some(k),
            _ => None,
        }
    }

    /// The shape without its buffer (a second parameter of one buffer is passed as any other pointer).
    fn rootless(self) -> Sh {
        match self {
            Sh::Arg(_) => Sh::S,
            Sh::At(k, ix, _) => Sh::At(k, ix, None),
            sh => sh,
        }
    }
}

/// A function being written: the scope its statements run in.
#[derive(Clone)]
struct Cx {
    /// The mask of the lanes the statements run for (a `[uint32x4_t; P]` variable).
    cur: String,
    /// Whether lanes may be apart here (in a varying `if` or loop): a variable's write keeps the other lanes'.
    divergent: bool,
    /// The function's lanes that have not returned (a variable), and its result (a variable), if it has one.
    live: String,
    res: Option<(String, M, bool)>,
    /// The loop's lanes that broke out of it and that went on to its next round (variables), in a varying loop.
    brk: Option<String>,
    cont: Option<String>,
    /// Whether the function is the kernel's own (`Func`): its parameters are the launch's arguments.
    kernel: bool,
    /// The variables' function: its instance's number (`u32::MAX`: the kernel's), and its `Body`'s number.
    ctx: u32,
    body: u32,
    /// Whether every lane runs the statements here (known as they are written): no mask is read.
    full: bool,
    /// The varying loop whose lanes the statements run for, and the varying statement (a loop, a branch of an
    /// `if`) whose lanes they are, if the statements are at its level: what a variable's write may skip a mask by.
    looped: Option<usize>,
    region: Option<usize>,
    /// Whether the statements are a chunk's of the kernel's own (a return of its lanes ends the chunk).
    chunk: bool,
    /// Whether the statements are a phase's, between a function's barriers (a return of its lanes ends the phase,
    /// whose end saves the lanes that go on).
    phased: bool,
}

/// A function's instance: its number, its parameters' shapes, whether all its lanes run (`_f`, else `_p`), and the
/// kernel's buffers whose every lane's own element (at `gid0`) the caller has accessed, so is valid (`_v{k}`).
type Inst = (u32, Vec<Sh>, bool, Vec<u32>);

struct Em<'a> {
    v: View<'a>,
    w: u32,
    p: u32,
    /// Each function's `Body` node, by number.
    bodies: HashMap<u32, usize>,
    /// What one pass found, for the next: each variable's lanes, each function's instances (one of each shape of
    /// its calls' arguments, as inlining would make) and each one's result's lanes.
    lanes: HashMap<(u32, u32), bool>,
    params: std::collections::BTreeSet<Inst>,
    /// This pass's calls' shapes (the next pass's `params`).
    seen: std::collections::BTreeSet<Inst>,
    /// Each instance's number, kept from pass to pass.
    insts: BTreeMap<Inst, u32>,
    rets: HashMap<u32, bool>,
    /// The function being written's start (where its arguments' fields are read, once), and those read there.
    top: usize,
    hoisted: HashMap<String, String>,
    /// Lanes made of a value that does not change (consecutive lanes, a constant's): their name, and the depth
    /// of the block that made them.
    memo: Vec<(String, String, usize)>,
    /// The kernel's buffers whose every lane's own element is valid here (accessed in each), and the depth of the
    /// block that found it (a found one ends with its block, as `memo`'s).
    valid: Vec<(u32, usize)>,
    /// This pass's writes of memory: the buffer (if known) and whether at the thread's own element; the last pass's
    /// buffers that only their own threads write (each at its own element), so no thread writes another's.
    writes: Vec<(Option<u32>, bool)>,
    disjoint: std::collections::BTreeSet<u32>,
    /// Whether the code written since it was cleared may read where no lane of it runs (a gather whose lanes that
    /// do not run take a running one's address: with none running, an address none named); the instances that may,
    /// as the last pass found them.
    risky: bool,
    risky_insts: std::collections::BTreeSet<u32>,
    again: bool,
    out: String,
    ind: usize,
    n: u32,
    vars: BTreeMap<(u32, u32), Var>,
    /// The kernel's threadgroup memory, if it has any: the argument number it is named by (`arg{k}`, after the
    /// launch's), as a buffer.
    shared: Option<u32>,
    /// What this pass could not write, which a later pass may (a call's result it finds of each lane): an error if the
    /// passes settle with it.
    pending: Option<String>,
}

/// Kernel `name`'s vector tape `k` as Rust at `w` lanes (4, 8, 16 or 32): a module's text, its entry
/// `pub unsafe fn {name}_w{w}(argv: *const *const u8, l: &Launch)`.
pub fn kernel(k: &crate::vector::Kernel, w: u32) -> R<String> {
    if !matches!(w, 4 | 8 | 16 | 32) {
        return err(format!("{w} lanes: 4, 8, 16 or 32"));
    }
    let v = View::new(&k.tape);
    let mut bodies = HashMap::new();
    for i in 0..v.len() {
        if v.kind(i) == Kind::Body {
            bodies.insert(v.w(i, 0), i);
        }
    }
    let mut e = Em { v, w, p: w / 4, bodies, lanes: HashMap::new(), params: Default::default(), seen: Default::default(), insts: BTreeMap::new(), rets: HashMap::new(), top: 0, hoisted: HashMap::new(), memo: Vec::new(), valid: Vec::new(), writes: Vec::new(), disjoint: Default::default(), risky: false, risky_insts: Default::default(), again: true, out: String::new(), ind: 0, n: 0, vars: BTreeMap::new(), shared: None, pending: None };
    if e.v.len() > 0 && e.v.kind(0) == Kind::Func && e.v.w(0, 2) > 0 {
        e.shared = Some(e.v.w(0, 4));
    }
    // Each pass finds lanes the one before did not (a variable written a lane's value, or apart); until none. A
    // pass finds at least a function's instances a call deeper than the last's (format 16's parent read, through
    // its slot, its array's value and column, the window load: 17 passes).
    for _ in 0..64 {
        if !e.again {
            break;
        }
        e.again = false;
        e.out.clear();
        e.n = 0;
        e.seen.clear();
        e.writes.clear();
        e.pending = None;
        e.module(&k.name)?;
        // The buffers only their own threads write: none written where the writes' buffer is not known.
        let mut d: std::collections::BTreeSet<u32> = e.writes.iter().filter_map(|w| w.0).collect();
        if e.writes.iter().any(|w| w.0.is_none()) {
            d.clear();
        }
        d.retain(|k| e.writes.iter().all(|w| w.0 != Some(*k) || w.1));
        if d != e.disjoint {
            e.disjoint = d;
            e.again = true;
        }
        if e.seen != e.params {
            e.params = std::mem::take(&mut e.seen);
            e.again = true;
        }
    }
    if e.again {
        return err("the variables' lanes did not settle");
    }
    if let Some(p) = e.pending.take() {
        return err(p);
    }
    Ok(std::mem::take(&mut e.out))
}

impl Em<'_> {
    fn line(&mut self, s: impl AsRef<str>) {
        // A block's end: the lanes it made are out of scope.
        if s.as_ref().starts_with('}') {
            let d = self.ind;
            self.memo.retain(|m| m.2 <= d);
            self.valid.retain(|m| m.1 <= d);
        }
        for _ in 0..self.ind {
            self.out.push_str("    ");
        }
        self.out.push_str(s.as_ref());
        self.out.push('\n');
    }

    fn tmp(&mut self) -> String {
        self.n += 1;
        format!("t{}", self.n)
    }

    /// `f(a[p], ...)` for each register `p`, as a new name.
    fn map(&mut self, f: impl Fn(u32) -> String) -> String {
        let t = self.tmp();
        let parts: Vec<String> = (0..self.p).map(&f).collect();
        self.line(format!("let {t}: [uint32x4_t; {}] = [{}];", self.p, parts.join(", ")));
        t
    }

    fn any(&self, k: &str) -> String {
        format!("any(&{k})")
    }

    fn all(&self, k: &str) -> String {
        format!("all(&{k})")
    }

    fn ones(&self) -> String {
        format!("[vdupq_n_u32(u32::MAX); {}]", self.p)
    }

    fn on(&self, k: &str, j: u32) -> String {
        format!("vgetq_lane_u32::<{}>({k}[{}]) != 0", j % 4, j / 4)
    }

    fn zeros(&self) -> String {
        format!("[vdupq_n_u32(0); {}]", self.p)
    }

    /// Zeros of each lane's `m`.
    fn zeros_of(&self, m: M) -> String {
        if wide(m) { format!("[vdupq_n_u64(0); {}]", 2 * self.p) } else { self.zeros() }
    }

    /// Each lane's 64-bit value: `f` of each of the `2P` registers of 2 lanes.
    fn map64(&mut self, f: impl Fn(u32) -> String) -> String {
        let t = self.tmp();
        let parts: Vec<String> = (0..2 * self.p).map(&f).collect();
        self.line(format!("let {t}: [uint64x2_t; {}] = [{}];", 2 * self.p, parts.join(", ")));
        t
    }

    /// A mask of 64-bit lanes (each register's two) as the lanes' mask.
    fn narrow_mask(&mut self, k: &str) -> String {
        let k = k.to_string();
        self.map(|r| format!("vcombine_u32(vmovn_u64({k}[{}]), vmovn_u64({k}[{}]))", 2 * r, 2 * r + 1))
    }

    /// `a` where mask `k`'s lanes are on, else `b`: the registers of each lane's `m`, as an array.
    fn blend(&self, m: M, k: &str, a: &str, b: &str) -> String {
        let parts: Vec<String> = if wide(m) {
            (0..2 * self.p).map(|r| format!("vbslq_u64({}, {a}[{r}], {b}[{r}])", wmask(k, r))).collect()
        } else {
            (0..self.p).map(|r| format!("vbslq_u32({k}[{r}], {a}[{r}], {b}[{r}])")).collect()
        };
        format!("[{}]", parts.join(", "))
    }

    fn module(&mut self, name: &str) -> R<()> {
        let (w, p) = (self.w, self.p);
        self.line(format!("//! `{name}` at {w} lanes ({p} NEON registers), written by rerail-ukl-vec's Rust (`rs.rs`) from its vector tape: do"));
        self.line("//! not edit; `UKL_RS_WRITE=1 cargo test -p rerail-ukl-vec _kernels_rust_is` writes it again.");
        self.line("");
        self.line("#![allow(unused, unused_mut, unused_parens, unused_unsafe, unreachable_code, non_snake_case, clippy::all, unsafe_op_in_unsafe_fn)]");
        self.line("");
        self.line("use crate::Launch;");
        self.line("use core::arch::aarch64::*;");
        self.line("use core::sync::atomic::{AtomicU32, Ordering::Relaxed};");
        self.line("");
        self.line("/// Lane `j`'s index in its register's `j`: `[0, 1, 2, 3]`, then `[4, 5, 6, 7]`, ...: a chunk's lanes.");
        let iota: Vec<String> = (0..p).map(|r| format!("[{}, {}, {}, {}]", 4 * r, 4 * r + 1, 4 * r + 2, 4 * r + 3)).collect();
        self.line(format!("const LANE: [[u32; 4]; {p}] = [{}];", iota.join(", ")));
        self.line("");
        self.line("/// Whether a lane of mask `k` is on: a comparison of its bits, which the compiler folds where it knows `k`.");
        self.line("#[inline(always)]");
        self.line(format!("unsafe fn any(k: &[uint32x4_t; {p}]) -> bool {{"));
        self.line(format!("    let mut x = k[0];\n    for r in 1..{p} {{\n        x = vorrq_u32(x, k[r]);\n    }}\n    core::mem::transmute::<uint32x4_t, u128>(x) != 0"));
        self.line("}");
        self.line("");
        self.line("/// Whether every lane of mask `k` is on, the same way.");
        self.line("#[inline(always)]");
        self.line(format!("unsafe fn all(k: &[uint32x4_t; {p}]) -> bool {{"));
        self.line(format!("    let mut x = k[0];\n    for r in 1..{p} {{\n        x = vandq_u32(x, k[r]);\n    }}\n    core::mem::transmute::<uint32x4_t, u128>(x) == u128::MAX"));
        self.line("}");
        self.line("");
        // The functions' instances, in their numbers' and shapes' order, then the kernel's.
        let insts: Vec<Inst> = self.params.iter().cloned().collect();
        for (id, sh, full, valid) in insts {
            self.body(id, &sh, full, &valid)?;
            self.line("");
        }
        self.line(format!("/// The group `l.group`'s threads, {w} at a time: `argv[j]` the address of argument `j`'s bytes (a buffer's address,"));
        self.line("/// a value's bytes).");
        self.line("///");
        self.line("/// # Safety");
        self.line("///");
        self.line("/// `argv` holds the kernel's arguments, its buffers as large as it reads and writes; `l` is a 1-D group's launch.");
        self.line(format!("pub unsafe fn {name}_w{w}(argv: *const *const u8, l: &Launch) {{"));
        self.ind += 1;
        self.line("let n = l.block[0] * l.block[1] * l.block[2];");
        self.top = self.out.len();
        self.hoisted.clear();
        let cx = Cx { cur: "live".into(), divergent: false, live: "live".into(), res: None, brk: None, cont: None, kernel: true, ctx: u32::MAX, body: u32::MAX, full: false, looped: None, region: None, chunk: false, phased: false };
        self.stmt(0, &cx)?;
        self.ind -= 1;
        self.line("}");
        Ok(())
    }

    /// The kernel's statement `n`, which its lanes run: for each chunk of `W` of the group's threads, all of whose
    /// lanes run, then the group's last threads (under `n`), if any, as some lanes of a chunk.
    fn chunks(&mut self, n: usize, cx: &Cx) -> R<()> {
        let w = self.w;
        self.line("let mut lid0 = 0u32;");
        self.line(format!("while lid0 + {w} <= n {{"));
        self.ind += 1;
        self.line(format!("let mut live = {};", self.ones()));
        self.line(format!("let {GID} = l.group[0] * l.block[0] + lid0;"));
        self.line("'chunk: {");
        self.ind += 1;
        self.stmt(n, &Cx { full: true, chunk: true, ..cx.clone() })?;
        self.ind -= 1;
        self.line("}");
        self.line(format!("lid0 += {w};"));
        self.ind -= 1;
        self.line("}");
        self.line("if lid0 < n {");
        self.ind += 1;
        self.line("// Lanes part: the group's last threads, those under `n`.");
        let t = self.map(|r| format!("vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[{r}].as_ptr())), vdupq_n_u32(n))"));
        self.line(format!("let mut live = {t};"));
        self.line(format!("let {GID} = l.group[0] * l.block[0] + lid0;"));
        self.line("'chunk: {");
        self.ind += 1;
        self.stmt(n, &Cx { full: false, chunk: true, ..cx.clone() })?;
        self.ind -= 1;
        self.line("}");
        self.ind -= 1;
        self.line("}");
        Ok(())
    }

    /// The kernel's statement `n` for each chunk; if it calls a function with barriers, once a phase of it: every
    /// chunk's phase before any chunk's next, as every thread of a group meets the others at a barrier. A chunk's
    /// lanes that go on and its variables are saved at a phase's end (in `st`, a slot of each), and read again at
    /// the next's start: what the GPU keeps in its threads' registers across a barrier.
    fn phased_chunks(&mut self, n: usize, cx: &Cx) -> R<()> {
        let called = (n..n + self.v.size(n)).filter(|&i| self.v.kind(i) == Kind::Call).find_map(|i| self.phases(self.v.w(i, 3)).map(|p| (self.v.w(i, 3), p.len())));
        let Some((id, np)) = called else { return self.chunks(n, cx) };
        let bytes = self.state_bytes(id);
        let w = self.w;
        self.line("// Phases: each chunk's, then the next phase's (its barriers).");
        self.line(format!("let mut state: Vec<u8> = Vec::with_capacity(n.div_ceil({w}) as usize * {bytes});"));
        self.line("let st: *mut u8 = state.as_mut_ptr();");
        // Each phase's own loop of the chunks, its number a constant: the function inlined there is that phase's.
        for j in 0..np {
            self.line("{");
            self.ind += 1;
            self.line(format!("let phase = {j}u32;"));
            self.chunks(n, cx)?;
            self.ind -= 1;
            self.line("}");
        }
        Ok(())
    }

    /// Whether statement `s` is a barrier of the group's threads (a call of a function that is only one).
    fn is_barrier(&self, s: usize) -> bool {
        if self.v.kind(s) != Kind::Expr {
            return false;
        }
        let e = self.v.child(s, 0);
        let only = |a: usize, b: usize| {
            (a..b).any(|i| self.v.kind(i) == Kind::Builtin && Builtin::of(self.v.w(i, 0)) == Builtin::Barrier)
                && (a..b).all(|i| match self.v.kind(i) {
                    Kind::Block | Kind::Expr | Kind::Empty => true,
                    Kind::Return => self.v.w(i, 0) == 0,
                    Kind::Builtin => Builtin::of(self.v.w(i, 0)) == Builtin::Barrier,
                    Kind::Const => true,
                    _ => false,
                })
        };
        match self.v.kind(e) {
            Kind::Builtin => only(e, e + self.v.size(e)),
            Kind::Call => self.bodies.get(&self.v.w(e, 3)).is_some_and(|&b| self.v.w(b, 6) == 0 && only(b + 1, b + self.v.size(b))),
            _ => false,
        }
    }

    /// Function `id`'s statements in phases, if barriers part them: its top-level statements between them.
    fn phases(&self, id: u32) -> Option<Vec<Vec<usize>>> {
        let &b = self.bodies.get(&id)?;
        let block = self.v.child(b, 0);
        if self.v.kind(block) != Kind::Block {
            return None;
        }
        let mut out = vec![Vec::new()];
        for c in self.v.children(block) {
            if self.is_barrier(c) {
                out.push(Vec::new());
            } else {
                out.last_mut().unwrap().push(c);
            }
        }
        // A function that is only a barrier (`ukl_tg_barrier`) is one, not phases: a call of it elsewhere is refused.
        (out.len() > 1 && out.iter().any(|p| !p.is_empty())).then_some(out)
    }

    /// A slot of a phase's saved value: the bytes of the widest lanes, `[uint64x2_t; 2P]`.
    fn slot(&self) -> usize {
        32 * self.p as usize
    }

    /// The variables function `id` keeps (its locals but its parameters), by offset.
    fn kept(&self, id: u32) -> Vec<u32> {
        let b = self.bodies[&id];
        let np = self.v.w(b, 6) as usize;
        let params: Vec<u32> = (0..np).map(|k| self.v.w(b, 7 + 2 * k)).collect();
        let mut offs: Vec<u32> = (b + 1..b + self.v.size(b)).filter(|&i| self.v.kind(i) == Kind::Local).map(|i| self.v.w(i, 0)).filter(|o| !params.contains(o)).collect();
        offs.sort();
        offs.dedup();
        offs
    }

    /// A chunk's saved state of function `id`'s phases: its lanes that go on, then each variable kept.
    fn state_bytes(&self, id: u32) -> usize {
        (1 + self.kept(id).len()) * self.slot()
    }

    /// Whether statement `n` reads the lanes' ids or calls a function (of the lanes' mask).
    fn of_lanes(&self, n: usize) -> bool {
        (n..n + self.v.size(n)).any(|i| match self.v.kind(i) {
            Kind::Call | Kind::Inline => true,
            Kind::Builtin => matches!(Builtin::of(self.v.w(i, 0)), Builtin::LocalId | Builtin::GlobalId | Builtin::SubGroupLocalId),
            _ => false,
        })
    }

    /// Function number `id`'s `Body`: a Rust function of the caller's mask and its parameters.
    fn body(&mut self, id: u32, shapes: &[Sh], full: bool, valid: &[u32]) -> R<()> {
        let n = self.bodies[&id];
        let ctx = self.inst(id, shapes, full, valid);
        let v = &self.v;
        let ret = M(v.w(n, 4));
        let np = v.w(n, 6) as usize;
        let params: Vec<(u32, M)> = (0..np).map(|k| (v.w(n, 7 + 2 * k), M(v.w(n, 8 + 2 * k)))).collect();
        let mut ps = vec!["l: &Launch".to_string(), "lid0: u32".to_string()];
        if !full {
            ps.push(format!("k: [uint32x4_t; {}]", self.p));
        }
        for (k, &(off, m)) in params.iter().enumerate() {
            let name = format!("x{off}");
            let var = if m.is_agg() {
                ps.push(format!("{name}: *const u8"));
                Var { name, m: M::PTR, lanes: false, fixed: None }
            } else {
                match shapes.get(k).copied().unwrap_or(Sh::S) {
                    Sh::Arg(r) => {
                        let name = format!("arg{r}");
                        ps.push(format!("{name}: *const u8"));
                        Var { name, m, lanes: false, fixed: None }
                    }
                    Sh::Gid => Var { name, m, lanes: false, fixed: Some(V::Iota(GID.into(), m)) },
                    Sh::In(r) => {
                        let name = format!("arg{r}_{name}");
                        ps.push(format!("{name}: {}", self.ty(m, false)?));
                        Var { name, m, lanes: false, fixed: None }
                    }
                    Sh::S => {
                        ps.push(format!("{name}: {}", self.ty(m, false)?));
                        Var { name, m, lanes: false, fixed: None }
                    }
                    Sh::L => {
                        ps.push(format!("{name}: {}", self.ty(m, true)?));
                        self.lanes.insert((ctx, off), true);
                        Var { name, m, lanes: true, fixed: None }
                    }
                    Sh::Iota => {
                        ps.push(format!("{name}: u32"));
                        Var { name: name.clone(), m, lanes: false, fixed: Some(V::Iota(name, m)) }
                    }
                    Sh::At(scale, ix, r) => {
                        let (b, i) = (r.map(|r| format!("arg{r}")).unwrap_or(format!("{name}b")), format!("{name}i"));
                        ps.push(format!("{b}: *const u8"));
                        let idx = match ix {
                            Ix::Gid => V::Iota(GID.into(), M::U32),
                            Ix::Iota => {
                                ps.push(format!("{i}: u32"));
                                V::Iota(i, M::U32)
                            }
                            Ix::L => {
                                ps.push(format!("{i}: {}", self.ty(M::U32, true)?));
                                V::L(i, M::U32)
                            }
                        };
                        Var { name, m, lanes: false, fixed: Some(V::At { base: b, idx: Box::new(idx), scale }) }
                    }
                }
            };
            self.vars.insert((ctx, off), var);
        }
        let phases = self.phases(id);
        if phases.is_some() {
            if ret != M::VOID {
                return err("a function with barriers that returns a value");
            }
            ps.push("phase: u32".into());
            ps.push("st: *mut u8".into());
        }
        let rl = self.rets.get(&ctx).copied().unwrap_or(false);
        let rt = if ret == M::VOID { String::new() } else { format!(" -> {}", self.ty(ret, rl)?) };
        self.line("#[inline(always)]");
        self.line(format!("unsafe fn {}({}){rt} {{", fname(id, shapes, full, valid), ps.join(", ")));
        self.ind += 1;
        self.line(format!("let {GID} = l.group[0] * l.block[0] + lid0;"));
        self.top = self.out.len();
        self.hoisted.clear();
        self.valid = valid.iter().map(|&k| (k, 1)).collect();
        // Its variables, each declared once at its start (a scope's are its function's in C's sense here).
        let locals = self.locals(n, ctx, &params)?;
        for var in &locals {
            let init = if var.lanes { self.zeros_of(var.m) } else if var.m == M::B { "false".into() } else if var.m == M::PTR { "core::ptr::null()".into() } else { "0".into() };
            self.line(format!("let mut {}: {} = {init};", var.name, self.ty(var.m, var.lanes)?));
        }
        if full {
            self.line(format!("let mut live = {};", self.ones()));
        } else {
            self.line("let mut live = k;");
        }
        let res = if ret == M::VOID {
            None
        } else {
            let init = if rl { self.zeros_of(ret) } else if ret == M::B { "false".into() } else { "0".into() };
            self.line(format!("let mut res: {} = {init};", self.ty(ret, rl)?));
            Some(("res".to_string(), ret, rl))
        };
        let cx = Cx { cur: "live".into(), divergent: false, live: "live".into(), res: res.clone(), brk: None, cont: None, kernel: false, ctx, body: id, full, looped: None, region: None, chunk: false, phased: false };
        self.risky = false;
        if let Some(phases) = phases {
            // Its phases, between its barriers: the one the kernel runs now, its state the last phase's.
            let slot = self.slot();
            let p = self.p;
            self.line("if phase > 0 {");
            self.ind += 1;
            self.line(format!("live = (st as *const [uint32x4_t; {p}]).read_unaligned();"));
            for (j, var) in locals.iter().enumerate() {
                let t = self.ty(var.m, var.lanes)?;
                self.line(format!("{} = (st.add({}) as *const {t}).read_unaligned();", var.name, (j + 1) * slot));
            }
            self.line(format!("if !{} {{ return; }}", self.any("live")));
            self.ind -= 1;
            self.line("}");
            for (j, stmts) in phases.iter().enumerate() {
                self.line(format!("if phase == {j} {{"));
                self.ind += 1;
                self.line("'phase: {");
                self.ind += 1;
                // A later phase's lanes are those that went on: a mask, whatever the instance's.
                self.seq(stmts, &Cx { phased: true, full: full && j == 0, ..cx.clone() }, None)?;
                self.ind -= 1;
                self.line("}");
                self.ind -= 1;
                self.line("}");
            }
            self.line(format!("if phase + 1 < {} {{", phases.len()));
            self.ind += 1;
            self.line(format!("(st as *mut [uint32x4_t; {p}]).write_unaligned(live);"));
            for (j, var) in locals.iter().enumerate() {
                let t = self.ty(var.m, var.lanes)?;
                self.line(format!("(st.add({}) as *mut {t}).write_unaligned({});", (j + 1) * slot, var.name));
            }
            self.ind -= 1;
            self.line("}");
        } else {
            self.stmt(self.v.child(n, 0), &cx)?;
        }
        if self.risky != self.risky_insts.contains(&ctx) {
            if self.risky { self.risky_insts.insert(ctx) } else { self.risky_insts.remove(&ctx) };
            self.again = true;
        }
        if res.is_some() && !self.out.ends_with("return res;\n") {
            self.line("res");
        }
        self.ind -= 1;
        self.line("}");
        Ok(())
    }

    /// Function `id`'s instance of `shapes`' number.
    fn inst(&mut self, id: u32, shapes: &[Sh], full: bool, valid: &[u32]) -> u32 {
        let k = self.insts.len() as u32;
        *self.insts.entry((id, shapes.to_vec(), full, valid.to_vec())).or_insert(k)
    }

    /// Whether every lane's own element of the kernel's buffer `k` is valid here.
    fn is_valid(&self, k: u32) -> bool {
        self.valid.iter().any(|v| v.0 == k)
    }

    /// Whether address `a`, of `m`, is every lane's own element of the kernel's buffer: that buffer.
    fn own(&self, a: &V, m: M) -> Option<u32> {
        match a {
            V::At { base, idx, scale } if *scale == m.bytes() && matches!(&**idx, V::Iota(f, _) if f == GID) => root(base),
            _ => None,
        }
    }

    /// An access of `a` (of `m`) where all lanes run: every lane's own element of its buffer is valid after it.
    fn accessed(&mut self, a: &V, m: M, cx: &Cx) {
        if cx.full
            && let Some(k) = self.own(a, m)
            && !self.is_valid(k)
        {
            let d = self.ind;
            self.valid.push((k, d));
        }
    }

    /// Whether function `b`'s first statement, which every call runs, reads or writes through its parameter `k`.
    fn first_access(&self, b: usize, k: usize) -> bool {
        let off = self.v.w(b, 7 + 2 * k);
        let mut s = self.v.child(b, 0);
        while self.v.kind(s) == Kind::Block {
            let Some(c) = self.v.children(s).next() else { return false };
            s = c;
        }
        if !matches!(self.v.kind(s), Kind::Expr | Kind::Return) || (s..s + self.v.size(s)).any(|i| matches!(self.v.kind(i), Kind::Logic | Kind::Cond | Kind::Comma)) {
            return false;
        }
        (s..s + self.v.size(s)).any(|i| {
            let through = |a: usize| self.loaded(a) == Some(off);
            match self.v.kind(i) {
                Kind::Builtin => matches!(Builtin::of(self.v.w(i, 0)), Builtin::AtomicLoad | Builtin::AtomicStore) && through(self.v.child(i, 0)),
                Kind::Load | Kind::Store => !M(self.v.w(i, 0)).is_agg() && through(self.v.child(i, 0)),
                _ => false,
            }
        })
    }

    /// Function `n`'s (context `ctx`'s) variables but its parameters, as accessed: each a Rust variable.
    fn locals(&mut self, n: usize, ctx: u32, params: &[(u32, M)]) -> R<Vec<Var>> {
        let mut out: BTreeMap<u32, Var> = BTreeMap::new();
        let end = n + self.v.size(n);
        for i in n + 1..end {
            if self.v.kind(i) != Kind::Local {
                continue;
            }
            let off = self.v.w(i, 0);
            if params.iter().any(|&(o, _)| o == off) || out.contains_key(&off) {
                continue;
            }
            // Its type: the access's (a load's, a store's, an update's).
            let m = self.access_type(i).unwrap_or(M::U32);
            let lanes = self.lanes.get(&(ctx, off)).copied().unwrap_or(false);
            out.insert(off, Var { name: format!("x{off}"), m, lanes, fixed: None });
        }
        for (off, var) in &out {
            self.vars.insert((ctx, *off), var.clone());
        }
        Ok(out.into_values().collect())
    }

    /// The type node `local`'s parent reads or writes there.
    fn access_type(&self, local: usize) -> Option<M> {
        // The parent: the nearest node before it whose subtree holds it.
        let mut q = local;
        while q > 0 {
            q -= 1;
            if q + self.v.size(q) > local {
                return match self.v.kind(q) {
                    Kind::Load | Kind::Store => Some(M(self.v.w(q, 0))),
                    Kind::Update => Some(M(self.v.w(q, 1))),
                    _ => None,
                };
            }
        }
        None
    }

    fn ty(&self, m: M, lanes: bool) -> R<String> {
        Ok(match (lanes, m) {
            (_, m) if m.is_agg() => "*const u8".into(),
            (false, M::B) => "bool".into(),
            (false, M::PTR) => "*const u8".into(),
            (false, m) => scalar(m)?.into(),
            (true, m) if m == M::B || m.bytes() == 4 => format!("[uint32x4_t; {}]", self.p),
            (true, m) if wide(m) => format!("[uint64x2_t; {}]", 2 * self.p),
            (true, m) => return err(format!("lanes of {m:?}")),
        })
    }

    // Statements.

    fn stmt(&mut self, n: usize, cx: &Cx) -> R<()> {
        let v = &self.v;
        let kids: Vec<usize> = v.children(n).collect();
        match v.kind(n) {
            Kind::Func => {
                // The kernel's: its parameters the launch's arguments, then its body.
                let np = v.w(n, 4) as usize;
                let ps: Vec<(u32, M)> = (0..np).map(|k| (v.w(n, 5 + 2 * k), M(v.w(n, 6 + 2 * k)))).collect();
                for (k, (off, m)) in ps.into_iter().enumerate() {
                    // A buffer: `arg{k}`, as every function that is passed it names it.
                    let name = if m == M::PTR { format!("arg{k}") } else { format!("x{off}") };
                    let val = if m.is_agg() { format!("*argv.add({k})") } else if m == M::PTR { format!("(*argv.add({k}) as *const *const u8).read_unaligned()") } else { format!("(*argv.add({k}) as *const {}).read_unaligned()", scalar(m)?) };
                    self.line(format!("let {name}: {} = {val};", self.ty(if m.is_agg() { M::PTR } else { m }, false)?));
                    self.vars.insert((u32::MAX, off), Var { name, m: if m.is_agg() { M::PTR } else { m }, lanes: false, fixed: None });
                }
                if let Some(k) = self.shared {
                    // The group's threadgroup memory: its threads', as a buffer of the launch's.
                    let words = self.v.w(n, 2).div_ceil(8);
                    self.line(format!("let mut tg = [0u64; {words}];"));
                    self.line(format!("let arg{k}: *const u8 = tg.as_mut_ptr() as *const u8;"));
                }
                self.top = self.out.len();
                // Its statements: one the lanes run, for each chunk of them; one of none (the launch's checks), once.
                let stmts: Vec<usize> = kids.into_iter().filter(|&c| self.v.kind(c) != Kind::Body).collect();
                let block = stmts.len() == 1 && self.v.kind(stmts[0]) == Kind::Block;
                let inner: Vec<usize> = if block { self.v.children(stmts[0]).collect() } else { stmts.clone() };
                if block && !(stmts[0]..stmts[0] + self.v.size(stmts[0])).any(|i| self.v.kind(i) == Kind::Local && !self.vars.contains_key(&(u32::MAX, self.v.w(i, 0)))) {
                    for c in inner {
                        if self.of_lanes(c) { self.phased_chunks(c, cx)? } else { self.stmt(c, cx)? }
                    }
                } else {
                    for c in stmts {
                        self.phased_chunks(c, cx)?;
                    }
                }
            }
            Kind::Block => self.seq(&kids, cx, None)?,
            Kind::Expr => {
                self.expr(kids[0], cx)?;
            }
            Kind::Empty => {}
            Kind::If => {
                if self.identity_guard(n) {
                    // Its lanes that do not take it add the reduction's identity: all of them run it.
                    return self.stmt(kids[1], cx);
                }
                let c = self.expr(kids[0], cx)?;
                self.if_with(n, c, cx)?;
            }
            Kind::While | Kind::For => {
                let (cond, body, step) = (kids[0], kids[1], kids.get(2).copied());
                // Where all lanes run, a condition with no effects and a body none leaves but by it: rounds unmasked
                // while every lane's condition holds, then (the chunk's last rounds) each lane its own. Its test is
                // the loop's own where the condition is one value, or consecutive lanes against one; it is worth
                // another where the hint says the lanes go on together (`UKL_LIKELY`), and not otherwise (lanes
                // that part early make it a second branch of each chunk's that the processor guesses wrong).
                // A condition of one value, and nothing in the loop that ends some lanes' rounds: all its lanes go on
                // or none, so its rounds are Rust's, their lanes the statement's.
                let leaves = (n..n + self.v.size(n)).any(|i| matches!(self.v.kind(i), Kind::Break | Kind::Continue | Kind::Return));
                if v.level(cond) == crate::k::Level::Uniform && !leaves && !self.effects(cond) {
                    self.line("loop {");
                    self.ind += 1;
                    self.line("// Rounds of one condition: every lane goes on.");
                    match self.expr(cond, cx)? {
                        V::S(c, _) => self.line(format!("if !({}) {{ break; }}", truth(&c, M::B))),
                        c => return err(format!("a uniform loop's condition of lanes: {c:?}")),
                    }
                    self.stmt(body, cx)?;
                    if let Some(s) = step {
                        self.stmt(s, cx)?;
                    }
                    self.ind -= 1;
                    self.line("}");
                    return Ok(());
                }
                let (at, top) = (self.out.len(), self.top);
                if cx.full && !self.effects(cond) && !self.exits(body) && step.is_none_or(|s| !self.exits(s)) {
                    self.line("loop {");
                    self.ind += 1;
                    self.line("// Full rounds: every lane goes on.");
                    let round = Cx { looped: Some(n), ..cx.clone() };
                    let c = self.expr(cond, &round)?;
                    let together = self.v.w(n, 1) == crate::k::IF_LIKELY;
                    let test = match &c {
                        V::S(c, _) => Some(format!("!({})", truth(c, M::B))),
                        V::Cmp { .. } => Some(self.mask(&c)?),
                        c if together => Some(self.mask(c)?),
                        _ => None,
                    }
                    .map(|k| if k.starts_with("!(") { k } else { format!("!{}", self.all(&k)) });
                    let Some(test) = test else {
                        // No full rounds: what was written of them goes (after what the function's top gained
                        // since), and with it what it found.
                        self.out.truncate(at + (self.top - top));
                        self.ind -= 1;
                        let d = self.ind;
                        self.memo.retain(|m| m.2 <= d);
                        self.valid.retain(|m| m.1 <= d);
                        return self.stmt_loop(n, cx);
                    };
                    self.line(format!("if {test} {{ break; }}"));
                    self.stmt(body, &round)?;
                    if let Some(s) = step {
                        self.stmt(s, &round)?;
                    }
                    self.ind -= 1;
                    self.line("}");
                }
                self.stmt_loop(n, cx)?;
            }
            Kind::Return => {
                let has = v.w(n, 0) != 0;
                let x = if has { Some(self.expr(kids[0], cx)?) } else { None };
                if cx.kernel {
                    // The thread ends.
                    if cx.divergent {
                        let cur = cx.cur.clone();
                        let live = cx.live.clone();
                        self.line(format!("{live} = [{}];", (0..self.p).map(|r| format!("vbicq_u32({live}[{r}], {cur}[{r}])")).collect::<Vec<_>>().join(", ")));
                    } else if cx.chunk {
                        self.line("break 'chunk;");
                    } else {
                        self.line("return;");
                    }
                    return Ok(());
                }
                // A value no call reads (an atomic's whose old value none does) is none.
                let x = x.filter(|x| !matches!(x, V::None));
                if let (Some(x), Some((res, m, lanes))) = (x, cx.res.clone()) {
                    if lanes {
                        let l = if m == M::B { self.mask(&x)? } else { self.lanes(&x, m)? };
                        let cur = cx.cur.clone();
                        if cx.divergent {
                            let b = self.blend(m, &cur, &l, &res);
                            self.line(format!("{res} = {b};"));
                        } else {
                            // Every lane still running returns here: the result is theirs.
                            self.line(format!("{res} = {l};"));
                        }
                    } else {
                        match x {
                            V::S(s, xm) => self.line(format!("{res} = {};", conv(&s, xm, m))),
                            _ => {
                                self.again = true;
                                self.rets.insert(cx.ctx, true);
                            }
                        }
                    }
                }
                if cx.divergent {
                    let cur = cx.cur.clone();
                    let live = cx.live.clone();
                    self.line(format!("{live} = [{}];", (0..self.p).map(|r| format!("vbicq_u32({live}[{r}], {cur}[{r}])")).collect::<Vec<_>>().join(", ")));
                } else if let Some((res, ..)) = &cx.res {
                    self.line(format!("return {res};"));
                } else if cx.phased {
                    // Its lanes end: none goes on to the next phase.
                    let live = cx.live.clone();
                    self.line(format!("{live} = {};", self.zeros()));
                    self.line("break 'phase;");
                } else {
                    self.line("return;");
                }
            }
            Kind::Break | Kind::Continue => {
                let to = if v.kind(n) == Kind::Break { cx.brk.clone() } else { cx.cont.clone() };
                let Some(to) = to else { return err("a break or continue out of no loop") };
                let cur = cx.cur.clone();
                self.line(format!("{to} = [{}];", (0..self.p).map(|r| format!("vorrq_u32({to}[{r}], {cur}[{r}])")).collect::<Vec<_>>().join(", ")));
            }
            k => return err(format!("a {k:?} statement")),
        }
        Ok(())
    }

    /// Loop `n`'s rounds, each lane its own: the lanes still in it, those whose condition holds; none, it ends.
    fn stmt_loop(&mut self, n: usize, cx: &Cx) -> R<()> {
        let kids: Vec<usize> = self.v.children(n).collect();
        let (cond, body, step) = (kids[0], kids[1], kids.get(2).copied());
        let lm = self.tmp();
        self.line(format!("let mut {lm} = {};", cx.cur));
        let has = |k: Kind| (n..n + self.v.size(n)).any(|i| self.v.kind(i) == k);
        let (hb, hc) = (has(Kind::Break), has(Kind::Continue));
        let brk = self.tmp();
        if hb {
            self.line(format!("let mut {brk} = {};", self.zeros()));
        }
        self.line("loop {");
        self.ind += 1;
        self.line("// Lanes part: each its own rounds.");
        let head = Cx { cur: lm.clone(), divergent: true, brk: hb.then(|| brk.clone()), cont: None, full: false, looped: Some(n), region: Some(n), ..cx.clone() };
        let c = self.expr(cond, &head)?;
        let k = self.mask(&c)?;
        if hb {
            self.line(format!("{lm} = [{}];", (0..self.p).map(|r| format!("vandq_u32(vbicq_u32({lm}[{r}], {brk}[{r}]), {k}[{r}])")).collect::<Vec<_>>().join(", ")));
        } else {
            self.line(format!("{lm} = [{}];", (0..self.p).map(|r| format!("vandq_u32({lm}[{r}], {k}[{r}])")).collect::<Vec<_>>().join(", ")));
        }
        self.line(format!("if !({}) {{ break; }}", self.any(&lm)));
        let cont = self.tmp();
        if hc {
            self.line(format!("let mut {cont} = {};", self.zeros()));
        }
        let inner = Cx { cur: lm.clone(), divergent: true, brk: hb.then(|| brk.clone()), cont: hc.then(|| cont.clone()), full: false, looped: Some(n), region: Some(n), ..cx.clone() };
        self.stmt(body, &inner)?;
        if let Some(s) = step {
            let t = self.refresh(&Cx { cont: None, ..inner.clone() });
            self.stmt(s, &Cx { cur: t, cont: None, ..inner.clone() })?;
        }
        // The lanes that returned leave it too.
        if cx.live != lm && self.returns(body) {
            let live = cx.live.clone();
            self.line(format!("{lm} = [{}];", (0..self.p).map(|r| format!("vandq_u32({lm}[{r}], {live}[{r}])")).collect::<Vec<_>>().join(", ")));
        }
        self.ind -= 1;
        self.line("}");
        Ok(())
    }

    /// Statements `kids` in turn, the first's condition `pre` if it is an `if` whose condition is already evaluated.
    /// Where lanes leave (a return, a break), the rest runs for those that remain, if any. Where all lanes run,
    /// an `if` of consecutive lanes against one value is decided for the chunk: none take it, all do, or (the
    /// slow path) some, each with the rest of the statements after it.
    fn seq(&mut self, kids: &[usize], cx: &Cx, mut pre: Option<V>) -> R<()> {
        let mut cx = cx.clone();
        let mut open = 0;
        for (j, &c) in kids.iter().enumerate() {
            let rest = &kids[j + 1..];
            if self.v.kind(c) == Kind::If && !self.identity_guard(c) {
                let cond = match pre.take() {
                    Some(x) => x,
                    None => {
                        let k0 = self.v.child(c, 0);
                        self.expr(k0, &cx)?
                    }
                };
                if let (V::Cmp { op, first, y }, true) = (&cond, cx.full) {
                    let (all, none) = self.decided(*op, first, y);
                    let kids_c: Vec<usize> = self.v.children(c).collect();
                    let exits = self.exits(c);
                    self.line(format!("if {none} {{"));
                    self.ind += 1;
                    if let Some(&e) = kids_c.get(2) {
                        self.stmt(e, &cx)?;
                    }
                    if exits && !kids_c.get(2).is_some_and(|&e| self.ends(e)) {
                        self.seq(rest, &cx, None)?;
                    }
                    self.ind -= 1;
                    self.line(format!("}} else if {all} {{"));
                    self.ind += 1;
                    self.stmt(kids_c[1], &cx)?;
                    if exits && !self.ends(kids_c[1]) {
                        self.seq(rest, &cx, None)?;
                    }
                    self.ind -= 1;
                    self.line("} else {");
                    self.ind += 1;
                    self.line("// Lanes part: the chunk's lanes on both sides of the bound.");
                    let part = Cx { full: false, ..cx.clone() };
                    if exits {
                        let mut all_kids = vec![c];
                        all_kids.extend_from_slice(rest);
                        self.seq(&all_kids, &part, Some(cond))?;
                    } else {
                        self.if_with(c, cond, &part)?;
                    }
                    self.ind -= 1;
                    self.line("}");
                    if exits {
                        break;
                    }
                    continue;
                }
                self.if_with(c, cond, &cx)?;
            } else {
                pre = None;
                self.stmt(c, &cx)?;
            }
            if !rest.is_empty() && self.exits(c) {
                // Lanes left: the rest runs for those that remain, if any.
                let t = self.refresh(&cx);
                self.line(format!("if {} {{", self.any(&t)));
                self.ind += 1;
                open += 1;
                // Lanes that left are not these: a write of the rest (a result too) is these lanes' alone.
                cx.divergent = cx.divergent || cx.cur != t || cx.res.is_some();
                cx.cur = t;
                cx.full = false;
            }
        }
        for _ in 0..open {
            self.ind -= 1;
            self.line("}");
        }
        Ok(())
    }

    /// Whether statement `n`, run by all lanes, ends their run of the statements after it (it ends in a return,
    /// a break or a continue on every path).
    fn ends(&self, n: usize) -> bool {
        match self.v.kind(n) {
            Kind::Return | Kind::Break | Kind::Continue => true,
            Kind::Block => self.v.children(n).last().is_some_and(|c| self.ends(c)),
            Kind::If => {
                let k: Vec<usize> = self.v.children(n).collect();
                k.len() == 3 && self.ends(k[1]) && self.ends(k[2])
            }
            _ => false,
        }
    }

    /// Whether all, and whether none, of the chunk's lanes `first + j` are `op y`, as Rust conditions (a lane past
    /// `u32::MAX` wraps, so a chunk that does is neither).
    fn decided(&self, op: Op, first: &str, y: &str) -> (String, String) {
        let w1 = self.w - 1;
        let (f, e, y) = (format!("(({first}) as u64)"), format!("(({first}) as u64 + {w1})"), format!("(({y}) as u64)"));
        let nw = format!("{e} <= u32::MAX as u64");
        let between = format!("({nw} && ({y} < {f} || {y} > {e}))");
        match op {
            Op::Ge => (format!("({nw} && {f} >= {y})"), format!("{e} < {y}")),
            Op::Gt => (format!("({nw} && {f} > {y})"), format!("{e} <= {y}")),
            Op::Lt => (format!("{e} < {y}"), format!("({nw} && {f} >= {y})")),
            Op::Le => (format!("{e} <= {y}"), format!("({nw} && {f} > {y})")),
            Op::Eq => ("false".into(), between),
            _ => (between, "false".into()),
        }
    }

    /// `if` statement `n` whose condition is `c`: Rust's, of one value; else its lanes' mask, each branch run if
    /// any lane takes it.
    fn if_with(&mut self, n: usize, c: V, cx: &Cx) -> R<()> {
        let kids: Vec<usize> = self.v.children(n).collect();
        if let V::S(c, _) = &c {
            self.line(format!("if {} {{", truth(c, M::B)));
            self.ind += 1;
            self.stmt(kids[1], cx)?;
            self.ind -= 1;
            if kids.len() > 2 {
                self.line("} else {");
                self.ind += 1;
                self.stmt(kids[2], cx)?;
                self.ind -= 1;
            }
            self.line("}");
            return Ok(());
        }
        let k = self.mask(&c)?;
        let hint = self.v.w(n, 1);
        if cx.full && hint != 0 && !self.exits(n) {
            // A chunk whose lanes all go the way the hint says runs that side unmasked; another, both masked.
            let (test, side) = if hint == crate::k::IF_LIKELY { (self.all(&k), Some(kids[1])) } else { (format!("!{}", self.any(&k)), kids.get(2).copied()) };
            self.line(format!("if {test} {{"));
            self.ind += 1;
            if let Some(side) = side {
                self.stmt(side, cx)?;
            }
            self.ind -= 1;
            self.line("} else {");
            self.ind += 1;
            self.line("// Lanes part: the chunk's lanes go both ways.");
            self.sides(n, &k, &Cx { full: false, ..cx.clone() })?;
            self.ind -= 1;
            self.line("}");
            return Ok(());
        }
        self.sides(n, &k, cx)
    }

    /// `if` statement `n`'s sides, the lanes of `cx` whose mask `k` holds taking the first, the others the second.
    /// Where it rejoins, each side runs masked, whether or not a lane takes it, unless it may read where no lane
    /// runs; where it may end lanes, each side runs where some lane takes it.
    fn sides(&mut self, n: usize, k: &str, cx: &Cx) -> R<()> {
        let kids: Vec<usize> = self.v.children(n).collect();
        let cur = cx.cur.clone();
        let rejoins = !self.exits(n);
        let t = if cx.full { k.to_string() } else { self.map(|r| format!("vandq_u32({cur}[{r}], {k}[{r}])")) };
        let inner = Cx { cur: t.clone(), divergent: true, full: false, looped: None, region: Some(kids[1]), ..cx.clone() };
        self.side(kids[1], &t, &inner, rejoins)?;
        if kids.len() > 2 {
            let e = if cx.full { self.map(|r| format!("vmvnq_u32({k}[{r}])")) } else { self.map(|r| format!("vbicq_u32({cur}[{r}], {k}[{r}])")) };
            let inner = Cx { cur: e.clone(), divergent: true, full: false, looped: None, region: Some(kids[2]), ..cx.clone() };
            self.side(kids[2], &e, &inner, rejoins)?;
        }
        Ok(())
    }

    /// One side `s` of an `if`, its lanes `t`: run masked, or (where it may end lanes, or read where none runs)
    /// where any lane takes it.
    fn side(&mut self, s: usize, t: &str, cx: &Cx, rejoins: bool) -> R<()> {
        let at = self.out.len();
        self.line("{");
        let open = self.out[at..].to_string();
        let before = std::mem::replace(&mut self.risky, false);
        self.ind += 1;
        self.stmt(s, cx)?;
        self.ind -= 1;
        self.line("}");
        if !rejoins || self.risky {
            self.out.replace_range(at..at + open.len(), &open.replacen('{', &format!("if {} {{", self.any(t)), 1));
        }
        self.risky |= before;
        Ok(())
    }

    /// Node `n` with its casts stripped.
    fn bare(&self, mut n: usize) -> usize {
        while self.v.kind(n) == Kind::Cast {
            n = self.v.child(n, 0);
        }
        n
    }

    /// The variable whose whole value node `n` loads, if it is one.
    fn loaded(&self, n: usize) -> Option<u32> {
        let n = self.bare(n);
        (self.v.kind(n) == Kind::Load && self.v.kind(self.v.child(n, 0)) == Kind::Local && self.v.w(self.v.child(n, 0), 2) == 0).then(|| self.v.w(self.v.child(n, 0), 0))
    }

    /// Whether `if` statement `n` is `if (x > 0) r(x)` (or `x != 0`), `x` unsigned and `r` the unread reduction of
    /// `x` at one address (an atomic maximum, sum or or, or a function of one): a lane that does not take it adds
    /// the reduction's identity, so all may run it, and the reduction is skipped where its result is the identity.
    fn identity_guard(&self, n: usize) -> bool {
        let v = &self.v;
        let kids: Vec<usize> = v.children(n).collect();
        if kids.len() != 2 || v.level(n) == crate::k::Level::Uniform {
            return false;
        }
        let c = self.bare(kids[0]);
        if v.kind(c) != Kind::Bin || !matches!(Op::of(v.w(c, 0)), Op::Gt | Op::Ne) || M(v.w(c, 1)) != M::U32 {
            return false;
        }
        let (a, z) = (v.child(c, 0), self.bare(v.child(c, 1)));
        let Some(x) = self.loaded(a) else { return false };
        if v.kind(z) != Kind::Const || v.w(z, 1) != 0 || v.w(z, 2) != 0 {
            return false;
        }
        let reduces = |b: usize, val: u32, via_param: bool| -> bool {
            v.kind(b) == Kind::Builtin
                && matches!(Builtin::of(v.w(b, 0)), Builtin::AtomicMax | Builtin::AtomicAdd | Builtin::AtomicOr)
                && M(v.w(b, 1)) == M::U32
                && self.loaded(v.child(b, 1)) == Some(val)
                && (via_param || self.unread(b))
        };
        // The branch: one expression statement.
        let mut t = kids[1];
        while v.kind(t) == Kind::Block && v.children(t).count() == 1 {
            t = v.child(t, 0);
        }
        if v.kind(t) != Kind::Expr {
            return false;
        }
        let e = v.child(t, 0);
        match v.kind(e) {
            Kind::Builtin => reduces(e, x, false),
            Kind::Call => {
                let Some(&b) = self.bodies.get(&v.w(e, 3)) else { return false };
                let args: Vec<usize> = v.children(e).collect();
                let Some(k) = args.iter().position(|&a| self.loaded(a) == Some(x)) else { return false };
                let pk = v.w(b, 7 + 2 * k);
                // Its body: one statement, the reduction of the parameter (its value, if returned, read by none).
                let mut s = v.child(b, 0);
                while v.kind(s) == Kind::Block && v.children(s).count() == 1 {
                    s = v.child(s, 0);
                }
                matches!(v.kind(s), Kind::Expr | Kind::Return) && v.children(s).count() == 1 && reduces(v.child(s, 0), pk, true) && self.unread(e)
            }
            _ => false,
        }
    }

    /// Node range of `cx`'s function, and its variable `off`'s accesses there (not its inner functions').
    fn accesses(&self, cx: &Cx, off: u32) -> Vec<usize> {
        let f = if cx.kernel { 0 } else { self.bodies[&cx.body] };
        let mut out = Vec::new();
        let mut i = f + 1;
        while i < f + self.v.size(f) {
            if self.v.kind(i) == Kind::Body {
                i += self.v.size(i);
                continue;
            }
            if self.v.kind(i) == Kind::Local && self.v.w(i, 0) == off {
                out.push(i);
            }
            i += 1;
        }
        out
    }

    /// Node `n`'s parent.
    fn parent(&self, n: usize) -> Option<usize> {
        (0..n).rev().find(|&q| q + self.v.size(q) > n)
    }

    /// Whether variable `off`, written where `cx`'s lanes run (lanes apart), is read by none of the lanes that do
    /// not: then its write needs no mask. Either the statements are a varying loop's (at its level), the loop holds
    /// no `continue` nor lies in a loop, and no access of the variable follows it (a lane that leaves never reads
    /// it again); or every access lies in the statement whose lanes run, the first a write of it there on every
    /// path (each entry writes it before it reads it).
    fn dead_off(&self, cx: &Cx, off: u32) -> bool {
        let acc = self.accesses(cx, off);
        let within = |s: usize, i: usize| s <= i && i < s + self.v.size(s);
        if let Some(l) = cx.looped {
            let f = if cx.kernel { 0 } else { self.bodies[&cx.body] };
            let nested = (f..l).any(|q| matches!(self.v.kind(q), Kind::While | Kind::For) && within(q, l));
            let cont = (l..l + self.v.size(l)).any(|i| self.v.kind(i) == Kind::Continue);
            if !nested && !cont && acc.iter().all(|&i| i < l + self.v.size(l)) {
                return true;
            }
        }
        if let Some(s) = cx.region
            && acc.iter().all(|&i| within(s, i))
            && let Some(&first) = acc.first()
            && let Some(st) = self.parent(first)
            && self.v.kind(st) == Kind::Store
            && self.v.child(st, 0) == first
            && !acc.iter().any(|&i| within(self.v.child(st, 1), i))
        {
            // Unconditional in the statement: only blocks and an expression statement between them.
            let mut q = self.parent(st);
            while let Some(x) = q {
                if x == s || (matches!(self.v.kind(s), Kind::While | Kind::For) && x == self.v.child(s, 1)) {
                    return true;
                }
                if !matches!(self.v.kind(x), Kind::Block | Kind::Expr) {
                    return false;
                }
                q = self.parent(x);
            }
        }
        false
    }

    /// Whether expression `n` writes memory or meets other threads (a store, an atomic, a barrier, a call).
    fn effects(&self, n: usize) -> bool {
        (n..n + self.v.size(n)).any(|i| match self.v.kind(i) {
            Kind::Store | Kind::Update | Kind::Call | Kind::Inline => true,
            Kind::Builtin => !matches!(Builtin::of(self.v.w(i, 0)), Builtin::LocalId | Builtin::LocalSize | Builtin::GlobalId | Builtin::GlobalSize | Builtin::GroupId | Builtin::NumGroups | Builtin::SubGroupLocalId),
            _ => false,
        })
    }

    /// Whether statement `n` may end some lanes' run of the statements after it (a return, break or continue in it).
    fn exits(&self, n: usize) -> bool {
        (n..n + self.v.size(n)).any(|i| matches!(self.v.kind(i), Kind::Return | Kind::Break | Kind::Continue) && self.v.level(i) != crate::k::Level::Uniform)
    }

    /// Whether statement `n` holds a return of some lanes.
    fn returns(&self, n: usize) -> bool {
        (n..n + self.v.size(n)).any(|i| self.v.kind(i) == Kind::Return && self.v.level(i) != crate::k::Level::Uniform)
    }

    /// The lanes of `cx` that remain: not returned, broken out or gone on.
    fn refresh(&mut self, cx: &Cx) -> String {
        if cx.cur == cx.live && cx.brk.is_none() && cx.cont.is_none() {
            return cx.live.clone();
        }
        let cur = cx.cur.clone();
        let live = cx.live.clone();
        let brk = cx.brk.clone();
        let cont = cx.cont.clone();
        self.map(|r| {
            let mut e = format!("vandq_u32({cur}[{r}], {live}[{r}])");
            if let Some(b) = &brk {
                e = format!("vbicq_u32({e}, {b}[{r}])");
            }
            if let Some(c) = &cont {
                e = format!("vbicq_u32({e}, {c}[{r}])");
            }
            e
        })
    }

    // Expressions.

    fn expr(&mut self, n: usize, cx: &Cx) -> R<V> {
        self.expr_at(n, cx).map_err(|e| if e.0.contains(" (node ") { e } else { Error(format!("{} (node {n})", e.0)) })
    }

    fn expr_at(&mut self, n: usize, cx: &Cx) -> R<V> {
        let v = &self.v;
        let p = v.p(n);
        let kids: Vec<usize> = v.children(n).collect();
        Ok(match v.kind(n) {
            Kind::Const => {
                // A pointer's constant: an integer (an offset, or null), as `u64`.
                let m = if M(p[0]) == M::PTR { M::U64 } else { M(p[0]) };
                let x = u64::from(p[1]) | u64::from(p[2]) << 32;
                V::S(literal(m, x)?, m)
            }
            Kind::Local => {
                let off = p[0];
                match self.vars.get(&(cx.ctx, off)) {
                    // An aggregate's (a structure's bytes where they are): its address.
                    Some(var) if var.m == M::PTR && !var.lanes => V::S(var.name.clone(), M::PTR),
                    _ => return err(format!("the address of a thread's variable (at {off})")),
                }
            }
            Kind::Load => {
                let m = M(p[0]);
                if m.is_agg() {
                    return self.expr(kids[0], cx);
                }
                if let Some(var) = self.var_of(kids[0], cx) {
                    if let Some(f) = var.fixed {
                        return Ok(f);
                    }
                    return Ok(if var.lanes { if var.m == M::B { V::K(var.name) } else { V::L(var.name, var.m) } } else { V::S(var.name, var.m) });
                }
                let a = self.expr(kids[0], cx)?;
                // A field of a struct passed by value (its bytes the caller's, which nothing here writes): read once,
                // at the function's start.
                if let V::S(at, _) = &a
                    && self.of_value(kids[0], cx)
                {
                    {
                        let e = format!("({at} as *const {}).read_unaligned()", scalar(m)?);
                        if let Some(h) = self.hoisted.get(&e) {
                            return Ok(V::S(h.clone(), m));
                        }
                        let h = format!("a{}", self.hoisted.len());
                        let text = format!("    let {h}: {} = {e};\n", scalar(m)?);
                        self.out.insert_str(self.top, &text);
                        self.top += text.len();
                        self.hoisted.insert(e, h.clone());
                        return Ok(V::S(h, m));
                    }
                }
                self.load(m, &a, cx)?
            }
            Kind::Store => {
                let m = M(p[0]);
                let x = self.expr(kids[1], cx)?;
                if let Some(var) = self.var_of(kids[0], cx) {
                    self.assign(&var, x.clone(), cx)?;
                    return Ok(x);
                }
                let a = self.expr(kids[0], cx)?;
                self.store(m, &a, &x, cx)?;
                x
            }
            Kind::Update => {
                let (op, lm, om, post) = (Op::of(p[0]), M(p[1]), M(p[2]), p[3] != 0);
                let Some(var) = self.var_of(kids[0], cx) else { return err("an update of memory") };
                let old = if var.lanes { V::L(var.name.clone(), var.m) } else { V::S(var.name.clone(), var.m) };
                let x = self.expr(kids[1], cx)?;
                let op = if op == Op::None { Op::Add } else { op };
                let _ = lm;
                // A count of each lane where lanes are apart, which a lane that does not run reads again: its
                // mask added (all ones a lane that runs, -1), not a select.
                let off = var.name[1..].parse::<u32>().unwrap_or(u32::MAX);
                if var.lanes && !wide(var.m) && cx.divergent && matches!(op, Op::Add | Op::Sub) && !self.dead_off(cx, off)
                    && let V::S(c, _) = &x
                    && let Some(c) = konst(c)
                {
                    let saved = if post { Some(self.hold(&old)?) } else { None };
                    let (name, cur) = (var.name.clone(), cx.cur.clone());
                    let f = |r: u32| match (op, c) {
                        (Op::Add, 1) => format!("vsubq_u32({name}[{r}], {cur}[{r}])"),
                        (Op::Sub, 1) => format!("vaddq_u32({name}[{r}], {cur}[{r}])"),
                        (Op::Add, c) => format!("vaddq_u32({name}[{r}], vandq_u32({cur}[{r}], vdupq_n_u32({c})))"),
                        (_, c) => format!("vsubq_u32({name}[{r}], vandq_u32({cur}[{r}], vdupq_n_u32({c})))"),
                    };
                    let parts: Vec<String> = (0..self.p).map(f).collect();
                    self.line(format!("{name} = [{}];", parts.join(", ")));
                    return Ok(saved.unwrap_or(V::L(name, var.m)));
                }
                let new = self.bin(op, om, &old, &x)?;
                let saved = if post { Some(self.hold(&old)?) } else { None };
                self.assign(&var, new.clone(), cx)?;
                saved.unwrap_or(new)
            }
            Kind::Bin => {
                let (op, m) = (Op::of(p[0]), M(p[1]));
                let a = self.expr(kids[0], cx)?;
                let b = self.expr(kids[1], cx)?;
                // A pointer and an offset: an address.
                if op == Op::Add
                    && m == M::PTR
                    && let Some(at) = self.address(&a, &b, cx)?
                {
                    return Ok(at);
                }
                self.bin(op, m, &a, &b)?
            }
            Kind::Un => {
                let (op, m) = (Op::of(p[0]), M(p[1]));
                let a = self.expr(kids[0], cx)?;
                match (op, a) {
                    (Op::LNot, V::S(s, am)) => V::S(format!("!({})", truth(&s, am)), M::B),
                    (Op::LNot, V::K(k)) => V::K(self.map(|r| format!("vmvnq_u32({k}[{r}])"))),
                    (Op::LNot, V::Cmp { op, first, y }) => V::Cmp { op: negated(op), first, y },
                    (Op::Not, V::S(s, _)) => V::S(format!("!({s})"), m),
                    (Op::Neg, V::S(s, _)) => V::S(format!("({s}).wrapping_neg()"), m),
                    (op @ (Op::Not | Op::Neg), a) if wide(m) => {
                        let l = self.lanes(&a, m)?;
                        V::L(
                            self.map64(|r| match op {
                                Op::Not => format!("vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64({l}[{r}])))"),
                                _ => format!("vreinterpretq_u64_s64(vnegq_s64(vreinterpretq_s64_u64({l}[{r}])))"),
                            }),
                            m,
                        )
                    }
                    (op, a) => {
                        let l = self.lanes(&a, m)?;
                        match op {
                            Op::Not => V::L(self.map(|r| format!("vmvnq_u32({l}[{r}])")), m),
                            Op::Neg => V::L(self.map(|r| format!("vreinterpretq_u32_s32(vnegq_s32(vreinterpretq_s32_u32({l}[{r}])))")), m),
                            Op::LNot => V::K(self.map(|r| format!("vceqzq_u32({l}[{r}])"))),
                            _ => return err(format!("a unary {op:?}")),
                        }
                    }
                }
            }
            Kind::Cast => {
                let (from, to) = (M(p[0]), M(p[1]));
                let a = self.expr(kids[0], cx)?;
                self.cast(a, from, to)?
            }
            Kind::Logic => {
                let and = p[0] != 0;
                let a = self.expr(kids[0], cx)?;
                if let V::S(x, xm) = &a {
                    // One value of every lane: C's short circuit as Rust's, the right side run only when it decides.
                    let t = self.tmp();
                    self.line(format!("let mut {t}: bool = {};", truth(x, *xm)));
                    self.line(format!("if {}{t} {{", if and { "" } else { "!" }));
                    self.ind += 1;
                    let b = self.expr(kids[1], cx)?;
                    let V::S(y, ym) = b else { return err("a short circuit of one value and the lanes'") };
                    self.line(format!("{t} = {};", truth(&y, ym)));
                    self.ind -= 1;
                    self.line("}");
                    return Ok(V::S(t, M::B));
                }
                // Each lane's: a right side with no effect evaluated whole and combined; one with, under its lanes
                // (the next step).
                if self.effects(kids[1]) {
                    return err("a short circuit of lanes whose right side has an effect");
                }
                let b = self.expr(kids[1], cx)?;
                let (ka, kb) = (self.mask(&a)?, self.mask(&b)?);
                let f = if and { "vandq_u32" } else { "vorrq_u32" };
                V::K(self.map(|r| format!("{f}({ka}[{r}], {kb}[{r}])")))
            }
            Kind::Call => {
                let id = p[3];
                let Some(&b) = self.bodies.get(&id) else { return err(format!("a call of function {id}, which has no body")) };
                let np = self.v.w(b, 6) as usize;
                let ret = M(self.v.w(b, 4));
                // Where all lanes run, the instance of all lanes (no mask); else the mask's.
                let mut args = vec!["l".to_string(), "lid0".to_string()];
                if !cx.full {
                    args.push(cx.cur.clone());
                }
                // A function with barriers: the kernel's phase of it, and the chunk's state.
                let phased = self.phases(id).is_some();
                if phased && !cx.kernel {
                    return err(format!("a barrier in function {id}, which the kernel's function calls (barriers part only the kernel's)"));
                }
                let mut vals = Vec::new();
                for &c in kids.iter().take(np) {
                    vals.push(self.expr(c, cx)?);
                }
                // The instance of its arguments' shapes (an aggregate's, its address, one).
                let mut shapes: Vec<Sh> = vals.iter().enumerate().map(|(k, a)| if M(self.v.w(b, 8 + 2 * k)).is_agg() { Sh::S } else { Sh::of(a) }).collect();
                // One parameter of each buffer by its name; another of it as any pointer.
                for k in 1..shapes.len() {
                    if shapes[k].root().is_some() && shapes[..k].iter().any(|s| s.root() == shapes[k].root()) {
                        shapes[k] = shapes[k].rootless();
                    }
                }
                let valid: Vec<u32> = {
                    let mut v: Vec<u32> = shapes.iter().filter_map(|s| s.root()).filter(|&k| self.is_valid(k)).collect();
                    v.sort();
                    v
                };
                self.seen.insert((id, shapes.clone(), cx.full, valid.clone()));
                let ctx = self.inst(id, &shapes, cx.full, &valid);
                if self.risky_insts.contains(&ctx) {
                    self.risky = true;
                }
                // Its first statement's access, through a parameter of every lane's own element, where all lanes run.
                let first: Vec<u32> = (0..shapes.len()).filter(|&k| matches!(shapes[k], Sh::At(_, Ix::Gid, Some(_))) && self.first_access(b, k)).filter_map(|k| shapes[k].root()).collect();
                for (k, a) in vals.into_iter().enumerate() {
                    let pm = M(self.v.w(b, 8 + 2 * k));
                    match (shapes[k], a) {
                        (_, V::S(s, _)) if pm.is_agg() => args.push(s),
                        (_, _) if pm.is_agg() => return err("an aggregate argument of each lane"),
                        (Sh::S, V::S(s, am)) => args.push(conv(&s, am, pm)),
                        (Sh::Arg(_), V::S(s, _)) => args.push(s),
                        (Sh::In(_), V::S(s, am)) => args.push(conv(&s, am, pm)),
                        (Sh::Gid, _) => {}
                        (Sh::Iota, V::Iota(f, _)) => args.push(f),
                        (Sh::L, a) => {
                            let l = if pm == M::B { self.mask(&a)? } else { self.lanes(&a, pm)? };
                            args.push(l);
                        }
                        (Sh::At(_, ix, _), V::At { base, idx, .. }) => {
                            args.push(base);
                            match (ix, *idx) {
                                (Ix::Gid, _) => {}
                                (_, V::Iota(f, _)) => args.push(f),
                                (_, i) => args.push(self.lanes(&i, M::U32)?),
                            }
                        }
                        (sh, a) => return err(format!("{a:?} passed as {sh:?}")),
                    }
                }
                if phased {
                    args.push("phase".into());
                    args.push(format!("st.add((lid0 / {}) as usize * {})", self.w, self.state_bytes(id)));
                }
                let call = format!("{}({})", fname(id, &shapes, cx.full, &valid), args.join(", "));
                let r = if ret == M::VOID {
                    self.line(format!("{call};"));
                    V::None
                } else {
                    let t = self.tmp();
                    self.line(format!("let {t} = {call};"));
                    if self.rets.get(&ctx).copied().unwrap_or(false) { if ret == M::B { V::K(t) } else { V::L(t, ret) } } else { V::S(t, ret) }
                };
                if cx.full {
                    for k in first {
                        if !self.is_valid(k) {
                            let d = self.ind;
                            self.valid.push((k, d));
                        }
                    }
                }
                r
            }
            Kind::Builtin => return self.builtin(n, cx),
            Kind::Cond => return self.cond(n, cx),
            Kind::Comma => {
                self.expr(kids[0], cx)?;
                return self.expr(kids[1], cx);
            }
            Kind::Shared => {
                // The group's threadgroup memory, the kernel's (`tg`), as a buffer.
                let Some(k) = self.shared else { return err("threadgroup memory of a kernel with none") };
                V::S(if p[0] == 0 { format!("arg{k}") } else { format!("arg{k}.wrapping_add({})", p[0]) }, M::PTR)
            }
            k => return err(format!("a {k:?} value")),
        })
    }

    /// `c ? a : b` (node `n`). Of one condition: Rust's `if`, its value each lane's where the arms' may be. Of each
    /// lane's: each arm under its lanes (an arm with a call, or one that may read where no lane runs, only where
    /// some lane takes it), the lanes' value a select of the two.
    fn cond(&mut self, n: usize, cx: &Cx) -> R<V> {
        let m = M(self.v.w(n, 0));
        let kids: Vec<usize> = self.v.children(n).collect();
        let lanes = self.v.level(n) != crate::k::Level::Uniform;
        let c = self.expr(kids[0], cx)?;
        let t = self.tmp();
        if let V::S(c, cm) = &c {
            let c = truth(c, *cm);
            self.line(format!("let {t}: {};", self.ty(m, lanes)?));
            self.line(format!("if {c} {{"));
            for (j, &arm) in kids[1..].iter().enumerate() {
                if j == 1 {
                    self.line("} else {");
                }
                self.ind += 1;
                let x = self.expr(arm, cx)?;
                let e = if !lanes {
                    match x {
                        V::S(s, xm) => conv(&s, xm, m),
                        x => return err(format!("a uniform ?:'s arm of lanes: {x:?}")),
                    }
                } else if m == M::B {
                    self.mask(&x)?
                } else {
                    self.lanes(&x, m)?
                };
                self.line(format!("{t} = {e};"));
                self.ind -= 1;
            }
            self.line("}");
            return Ok(if !lanes { V::S(t, m) } else if m == M::B { V::K(t) } else { V::L(t, m) });
        }
        let k = self.mask(&c)?;
        let cur = cx.cur.clone();
        self.line(format!("let mut {t}: {} = {};", self.ty(m, true)?, self.zeros_of(m)));
        let ka = if cx.full { k.clone() } else { self.map(|r| format!("vandq_u32({cur}[{r}], {k}[{r}])")) };
        let kb = if cx.full { self.map(|r| format!("vmvnq_u32({k}[{r}])")) } else { self.map(|r| format!("vbicq_u32({cur}[{r}], {k}[{r}])")) };
        for (arm, ka) in [(kids[1], ka), (kids[2], kb)] {
            let calls = (arm..arm + self.v.size(arm)).any(|i| matches!(self.v.kind(i), Kind::Call | Kind::Inline));
            let at = self.out.len();
            self.line("{");
            let open = self.out[at..].to_string();
            let top = self.top;
            let before = std::mem::replace(&mut self.risky, false);
            self.ind += 1;
            let inner = Cx { cur: ka.clone(), divergent: true, full: false, looped: None, region: None, ..cx.clone() };
            let x = self.expr(arm, &inner)?;
            let l = if m == M::B { self.mask(&x)? } else { self.lanes(&x, m)? };
            let b = self.blend(m, &ka, &l, &t);
            self.line(format!("{t} = {b};"));
            self.ind -= 1;
            self.line("}");
            if calls || self.risky {
                let at = at + (self.top - top);
                self.out.replace_range(at..at + open.len(), &open.replacen('{', &format!("if {} {{", self.any(&ka)), 1));
            }
            self.risky |= before;
        }
        Ok(if m == M::B { V::K(t) } else { V::L(t, m) })
    }

    /// The thread's variable node `a` is the address of, whole.
    fn var_of(&mut self, a: usize, cx: &Cx) -> Option<Var> {
        if self.v.kind(a) != Kind::Local || self.v.w(a, 2) != 0 {
            return None;
        }
        let var = self.vars.get(&(cx.ctx, self.v.w(a, 0)))?.clone();
        (var.m != M::PTR || var.lanes || !self.agg_param(cx, self.v.w(a, 0))).then_some(var)
    }

    /// Whether address `a` is an aggregate parameter's plus constants, and the function writes none of its bytes.
    fn of_value(&self, a: usize, cx: &Cx) -> bool {
        let mut agg = None;
        for i in a..a + self.v.size(a) {
            match self.v.kind(i) {
                Kind::Const | Kind::Cast | Kind::Bin => {}
                Kind::Local if self.agg_param(cx, self.v.w(i, 0)) => agg = Some(self.v.w(i, 0)),
                _ => return false,
            }
        }
        let Some(off) = agg else { return false };
        let f = if cx.kernel { 0 } else { self.bodies[&cx.body] };
        // Its writes: a store or an update whose address holds it, in the function (not its inner functions').
        !(f..f + self.v.size(f)).any(|i| {
            matches!(self.v.kind(i), Kind::Store | Kind::Update) && {
                let t = self.v.child(i, 0);
                (t..t + self.v.size(t)).any(|j| self.v.kind(j) == Kind::Local && self.v.w(j, 0) == off)
            } && (cx.kernel || !self.bodies.values().any(|&b| b != f && b > f && b < i && i < b + self.v.size(b)))
        })
    }

    /// Whether variable `off` of `cx`'s function is an aggregate parameter (its value its address).
    fn agg_param(&self, cx: &Cx, off: u32) -> bool {
        let n = if cx.kernel { 0 } else { self.bodies[&cx.body] };
        let (at, k0) = if cx.kernel { (4, 5) } else { (6, 7) };
        let np = self.v.w(n, at) as usize;
        (0..np).any(|k| self.v.w(n, k0 + 2 * k) == off && M(self.v.w(n, k0 + 1 + 2 * k)).is_agg())
    }

    fn assign(&mut self, var: &Var, x: V, cx: &Cx) -> R<()> {
        if var.fixed.is_some() {
            return err(format!("a write of parameter {}, passed as its calls' shape", var.name));
        }
        let key = (cx.ctx, self.vars.iter().find(|(k, v)| k.0 == cx.ctx && v.name == var.name).map(|(k, _)| k.1).unwrap_or(0));
        if !var.lanes {
            match x {
                V::S(s, xm) if !cx.divergent => {
                    self.line(format!("{} = {};", var.name, conv(&s, xm, var.m)));
                    return Ok(());
                }
                _ => {
                    // A lane's value, or a value set where lanes are apart: each lane's, from the next pass.
                    self.lanes.insert(key, true);
                    self.again = true;
                    return Ok(());
                }
            }
        }
        let l = if var.m == M::B { self.mask(&x)? } else { self.lanes(&x, var.m)? };
        let off = var.name[1..].parse::<u32>().unwrap_or(u32::MAX);
        if cx.divergent && !self.dead_off(cx, off) {
            let b = self.blend(var.m, &cx.cur, &l, &var.name);
            self.line(format!("{} = {b};", var.name));
        } else {
            self.line(format!("{} = {l};", var.name));
        }
        Ok(())
    }

    /// `x` kept as it is now (a post-increment's old value).
    fn hold(&mut self, x: &V) -> R<V> {
        Ok(match x {
            V::S(s, m) => {
                let t = self.tmp();
                self.line(format!("let {t} = {s};"));
                V::S(t, *m)
            }
            V::L(l, m) => {
                let t = self.tmp();
                self.line(format!("let {t} = {l};"));
                V::L(t, *m)
            }
            x => x.clone(),
        })
    }

    /// Pointer `a` plus offset `b` (each lane's, or one): an address, if it is a base and lanes' 32-bit indexes;
    /// or each lane's own, a base and each lane's 64-bit offset, where a lane that does not run (of `cx`) has the
    /// offset 0 (the base's first bytes, not an address none named).
    fn address(&mut self, a: &V, b: &V, cx: &Cx) -> R<Option<V>> {
        let (base, off) = match (a, b) {
            (V::S(s, M::PTR), o) => (s.clone(), o.clone()),
            (o, V::S(s, M::PTR)) => (s.clone(), o.clone()),
            _ => return Ok(None),
        };
        Ok(match off {
            V::S(o, _) => Some(V::S(format!("{base}.wrapping_add({o} as usize)"), M::PTR)),
            V::At { base: b2, idx, scale } if b2.is_empty() => Some(V::At { base, idx, scale }),
            V::L(l, lm) if wide(lm) => {
                let o = if cx.full {
                    l
                } else {
                    let cur = cx.cur.clone();
                    self.map64(|r| format!("vandq_u64({l}[{r}], {})", wmask(&cur, r)))
                };
                Some(V::L(self.map64(|r| format!("vaddq_u64(vdupq_n_u64(({base}) as u64), {o}[{r}])")), M::U64))
            }
            _ => None,
        })
    }

    fn bin(&mut self, op: Op, m: M, a: &V, b: &V) -> R<V> {
        // A lane's 32-bit index widened and scaled: an offset of each lane (an address once a base is added).
        // A pointer's operation on integers (offsets): theirs, at `u64`; a comparison with a pointer, of pointers.
        let real = |x: &V| matches!(x, V::S(_, M::PTR));
        let m = if m == M::PTR && !real(a) && !real(b) { M::U64 } else { m };
        if m == M::PTR || m == M::U64 || m == M::I64 {
            match (op, a, b) {
                // A lane's index times a scale (a constant of 32 bits): still an index; times another value, the
                // lanes' product below.
                (Op::Mul, V::At { base, idx, scale: 1 }, V::S(k, _)) | (Op::Mul, V::S(k, _), V::At { base, idx, scale: 1 }) if base.is_empty() && konst(k).is_some() => {
                    return Ok(V::At { base: String::new(), idx: idx.clone(), scale: konst(k).unwrap_or(1) });
                }
                (_, V::S(x, xm), V::S(y, ym)) => {
                    let (x, y) = (conv(x, *xm, m), conv(y, *ym, m));
                    return Ok(V::S(scalar_bin(op, m, &x, &y)?, if op.compares() { M::B } else { m }));
                }
                _ => return self.bin64(op, m, a, b),
            }
        }
        match (a, b) {
            (V::S(x, xm), V::S(y, ym)) => {
                let (x, y) = (conv(x, *xm, m), conv(y, *ym, m));
                return Ok(V::S(scalar_bin(op, m, &x, &y)?, if op.compares() { M::B } else { m }));
            }
            // Consecutive lanes compared with one value: a mask a chunk may decide at once.
            (V::Iota(f, _), V::S(y, ym)) if op.compares() && m.bytes() == 4 && !m.signed() => {
                return Ok(V::Cmp { op, first: f.clone(), y: conv(y, *ym, M::U32) });
            }
            (V::S(y, ym), V::Iota(f, _)) if op.compares() && m.bytes() == 4 && !m.signed() => {
                return Ok(V::Cmp { op: swapped(op), first: f.clone(), y: conv(y, *ym, M::U32) });
            }
            // A thread id plus a uniform value: still consecutive.
            (V::Iota(f, fm), V::S(y, ym)) | (V::S(y, ym), V::Iota(f, fm)) if op == Op::Add => {
                return Ok(V::Iota(format!("{f}.wrapping_add({})", conv(y, *ym, *fm)), m));
            }
            _ => {}
        }
        if m.bytes() != 4 || m.float() {
            return err(format!("{op:?} of lanes of {m:?}"));
        }
        let s = m.signed();
        if matches!(op, Op::Div | Op::Rem) {
            return self.div(op, m, a, b);
        }
        // A shift by a uniform count.
        if matches!(op, Op::Shl | Op::Shr) {
            let x = self.lanes(a, m)?;
            let c = match b {
                V::S(c, _) => format!("vdupq_n_s32((({c}) & 31) as i32)"),
                _ => {
                    // Each lane's count (C's, masked to 31): a shift left by it, or right by its negation (an
                    // arithmetic one for a signed type).
                    let l = self.lanes(b, m)?;
                    let k = |r: u32| format!("vreinterpretq_s32_u32(vandq_u32({l}[{r}], vdupq_n_u32(31)))");
                    return Ok(V::L(
                        match (op, s) {
                            (Op::Shl, _) => self.map(|r| format!("vshlq_u32({x}[{r}], {})", k(r))),
                            (_, false) => self.map(|r| format!("vshlq_u32({x}[{r}], vnegq_s32({}))", k(r))),
                            _ => self.map(|r| format!("vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32({x}[{r}]), vnegq_s32({})))", k(r))),
                        },
                        m,
                    ));
                }
            };
            return Ok(V::L(
                match (op, s) {
                    (Op::Shl, _) => self.map(|r| format!("vshlq_u32({x}[{r}], {c})")),
                    (_, false) => self.map(|r| format!("vshlq_u32({x}[{r}], vnegq_s32({c}))")),
                    (_, true) => self.map(|r| format!("vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32({x}[{r}]), vnegq_s32({c})))")),
                },
                m,
            ));
        }
        let (x, y) = (self.lanes(a, m)?, self.lanes(b, m)?);
        let si = |f: &str, r: u32| format!("{f}(vreinterpretq_s32_u32({x}[{r}]), vreinterpretq_s32_u32({y}[{r}]))");
        Ok(match op {
            Op::Add => V::L(self.map(|r| format!("vaddq_u32({x}[{r}], {y}[{r}])")), m),
            Op::Sub => V::L(self.map(|r| format!("vsubq_u32({x}[{r}], {y}[{r}])")), m),
            Op::Mul => V::L(self.map(|r| format!("vmulq_u32({x}[{r}], {y}[{r}])")), m),
            Op::And => V::L(self.map(|r| format!("vandq_u32({x}[{r}], {y}[{r}])")), m),
            Op::Or => V::L(self.map(|r| format!("vorrq_u32({x}[{r}], {y}[{r}])")), m),
            Op::Xor => V::L(self.map(|r| format!("veorq_u32({x}[{r}], {y}[{r}])")), m),
            Op::Eq => V::K(self.map(|r| format!("vceqq_u32({x}[{r}], {y}[{r}])"))),
            Op::Ne => V::K(self.map(|r| format!("vmvnq_u32(vceqq_u32({x}[{r}], {y}[{r}]))"))),
            Op::Lt if !s => V::K(self.map(|r| format!("vcltq_u32({x}[{r}], {y}[{r}])"))),
            Op::Le if !s => V::K(self.map(|r| format!("vcleq_u32({x}[{r}], {y}[{r}])"))),
            Op::Gt if !s => V::K(self.map(|r| format!("vcgtq_u32({x}[{r}], {y}[{r}])"))),
            Op::Ge if !s => V::K(self.map(|r| format!("vcgeq_u32({x}[{r}], {y}[{r}])"))),
            Op::Lt => V::K(self.map(|r| si("vcltq_s32", r))),
            Op::Le => V::K(self.map(|r| si("vcleq_s32", r))),
            Op::Gt => V::K(self.map(|r| si("vcgtq_s32", r))),
            Op::Ge => V::K(self.map(|r| si("vcgeq_s32", r))),
            op => return err(format!("{op:?} of lanes")),
        })
    }

    /// `a op b` of each lane's 64-bit `m`.
    fn bin64(&mut self, op: Op, m: M, a: &V, b: &V) -> R<V> {
        let s = m.signed();
        let x = self.lanes(a, m)?;
        if matches!(op, Op::Shl | Op::Shr) {
            // The count: one, or each lane's (C's, below 64).
            let c = match b {
                V::S(c, cm) => {
                    let c = format!("vdupq_n_s64((({}) & 63) as i64)", conv(c, *cm, M::U32));
                    (0..2 * self.p).map(|_| c.clone()).collect::<Vec<_>>()
                }
                _ => {
                    let y = self.lanes(b, M::U64)?;
                    (0..2 * self.p).map(|r| format!("vreinterpretq_s64_u64(vandq_u64({y}[{r}], vdupq_n_u64(63)))")).collect()
                }
            };
            return Ok(V::L(
                match (op, s) {
                    (Op::Shl, _) => self.map64(|r| format!("vshlq_u64({x}[{r}], {})", c[r as usize])),
                    (_, false) => self.map64(|r| format!("vshlq_u64({x}[{r}], vnegq_s64({}))", c[r as usize])),
                    (_, true) => self.map64(|r| format!("vreinterpretq_u64_s64(vshlq_s64(vreinterpretq_s64_u64({x}[{r}]), vnegq_s64({})))", c[r as usize])),
                },
                m,
            ));
        }
        let y = self.lanes(b, m)?;
        if matches!(op, Op::Mul | Op::Div | Op::Rem) {
            // NEON has no 64-bit multiply or divide: each lane's own (a divisor of 0 gives 0).
            let st = if s { "i64" } else { "u64" };
            let f = match op {
                Op::Mul => "wrapping_mul",
                Op::Div => "checked_div",
                _ => "checked_rem",
            };
            let t = self.tmp();
            self.line(format!("let mut {t}: [uint64x2_t; {}] = {};", 2 * self.p, self.zeros_of(m)));
            for j in 0..self.w {
                let (r, q) = (j / 2, j % 2);
                let e = format!("(vgetq_lane_u64::<{q}>({x}[{r}]) as {st}).{f}(vgetq_lane_u64::<{q}>({y}[{r}]) as {st})");
                let e = if op == Op::Mul { e } else { format!("{e}.unwrap_or(0)") };
                self.line(format!("{t}[{r}] = vsetq_lane_u64::<{q}>({e} as u64, {t}[{r}]);"));
            }
            return Ok(V::L(t, m));
        }
        let si = |f: &str, r: u32| format!("{f}(vreinterpretq_s64_u64({x}[{r}]), vreinterpretq_s64_u64({y}[{r}]))");
        let k = match op {
            Op::Add => return Ok(V::L(self.map64(|r| format!("vaddq_u64({x}[{r}], {y}[{r}])")), m)),
            Op::Sub => return Ok(V::L(self.map64(|r| format!("vsubq_u64({x}[{r}], {y}[{r}])")), m)),
            Op::And => return Ok(V::L(self.map64(|r| format!("vandq_u64({x}[{r}], {y}[{r}])")), m)),
            Op::Or => return Ok(V::L(self.map64(|r| format!("vorrq_u64({x}[{r}], {y}[{r}])")), m)),
            Op::Xor => return Ok(V::L(self.map64(|r| format!("veorq_u64({x}[{r}], {y}[{r}])")), m)),
            Op::Eq | Op::Ne => self.map64(|r| format!("vceqq_u64({x}[{r}], {y}[{r}])")),
            Op::Lt if !s => self.map64(|r| format!("vcltq_u64({x}[{r}], {y}[{r}])")),
            Op::Le if !s => self.map64(|r| format!("vcleq_u64({x}[{r}], {y}[{r}])")),
            Op::Gt if !s => self.map64(|r| format!("vcgtq_u64({x}[{r}], {y}[{r}])")),
            Op::Ge if !s => self.map64(|r| format!("vcgeq_u64({x}[{r}], {y}[{r}])")),
            Op::Lt => self.map64(|r| si("vcltq_s64", r)),
            Op::Le => self.map64(|r| si("vcleq_s64", r)),
            Op::Gt => self.map64(|r| si("vcgtq_s64", r)),
            Op::Ge => self.map64(|r| si("vcgeq_s64", r)),
            op => return err(format!("{op:?} of 64-bit lanes")),
        };
        let k = self.narrow_mask(&k);
        Ok(V::K(if op == Op::Ne { self.map(|r| format!("vmvnq_u32({k}[{r}])")) } else { k }))
    }

    /// A division of lanes: each lane's own (NEON has none of integers); a lane whose divisor is 0 gets 0, as one
    /// that does not run may hold one.
    fn div(&mut self, op: Op, m: M, a: &V, b: &V) -> R<V> {
        let (x, y) = (self.lanes(a, m)?, self.lanes(b, m)?);
        let t = self.tmp();
        let (w, p) = (self.w, self.p);
        let st = if m.signed() { "i32" } else { "u32" };
        let f = if op == Op::Div { "checked_div" } else { "checked_rem" };
        self.line(format!("let mut {t}x = [0u32; {w}];"));
        self.line(format!("let mut {t}y = [0u32; {w}];"));
        self.line(format!("for r in 0..{p} {{ vst1q_u32({t}x.as_mut_ptr().add(4 * r), {x}[r]); vst1q_u32({t}y.as_mut_ptr().add(4 * r), {y}[r]); }}"));
        self.line(format!("for j in 0..{w} {{ {t}x[j] = ({t}x[j] as {st}).{f}({t}y[j] as {st}).unwrap_or(0) as u32; }}"));
        Ok(V::L(self.map(|r| format!("vld1q_u32({t}x.as_ptr().add({}))", 4 * r)), m))
    }

    fn cast(&mut self, a: V, from: M, to: M) -> R<V> {
        if from.is_agg() || to.is_agg() {
            return Ok(a);
        }
        Ok(match a {
            // An integer as a pointer: an offset, an integer still (`u64`).
            V::S(s, sm) if to == M::PTR && sm != M::PTR => {
                let s = match konst(&s) { Some(c) => literal(M::U64, u64::from(c))?, None => conv(&s, sm, M::U64) };
                V::S(s, M::U64)
            }
            // A constant's: the constant (a dimension's position, a scale stay literals).
            V::S(s, sm) if sm != M::B && !sm.float() && !to.float() && to != M::B && konst(&s).is_some_and(|c| !sm.signed() || (c as i32) >= 0) => {
                V::S(literal(to, u64::from(konst(&s).unwrap()))?, to)
            }
            V::S(s, sm) => {
                if to == M::B {
                    V::S(truth(&s, sm), M::B)
                } else {
                    V::S(conv(&s, sm, to), to)
                }
            }
            // A comparison's 0 or 1 and its truth: the mask, either way.
            V::K(k) if to == M::B || from == M::B || to.bytes() == 4 => V::K(k),
            c @ V::Cmp { .. } if to == M::B || from == M::B || to.bytes() == 4 => c,
            V::K(k) if wide(to) => V::L(self.lanes(&V::K(k), to)?, to),
            V::K(_) => return err(format!("a mask as {to:?}")),
            // 64-bit lanes: as other 64 bits, the same; as 32 or fewer, each lane's low half; as a truth, whether
            // any bit is set.
            V::L(l, _) if wide(from) && wide(to) => V::L(l, to),
            V::L(l, _) if wide(from) && to == M::B => {
                let k = self.map64(|r| format!("vtstq_u64({l}[{r}], {l}[{r}])"));
                V::K(self.narrow_mask(&k))
            }
            V::L(l, _) if wide(from) && to.bytes() <= 4 => V::L(self.map(|r| format!("vcombine_u32(vmovn_u64({l}[{}]), vmovn_u64({l}[{}]))", 2 * r, 2 * r + 1)), to),
            V::At { ref base, .. } if base.is_empty() && wide(from) && wide(to) => a,
            V::At { ref base, .. } if base.is_empty() && to.bytes() == 4 => {
                let l = self.lanes(&a, M::U64)?;
                V::L(self.map(|r| format!("vcombine_u32(vmovn_u64({l}[{}]), vmovn_u64({l}[{}]))", 2 * r, 2 * r + 1)), to)
            }
            V::L(..) if wide(to) && from.bytes() == 4 && from.signed() => V::L(self.lanes(&a, to)?, to),
            V::L(l, _) if to == M::B => V::K(self.map(|r| format!("vtstq_u32({l}[{r}], {l}[{r}])"))),
            V::L(l, _) if to.bytes() == 4 && from.bytes() == 4 => V::L(l, to),
            V::Iota(f, _) if to.bytes() == 4 => V::Iota(f, to),
            // A lane's 32-bit index widened: an offset, scaled 1 so far.
            V::L(..) | V::Iota(..) if to.bytes() == 8 && from.bytes() == 4 && !from.signed() => V::At { base: String::new(), idx: Box::new(a), scale: 1 },
            a => return err(format!("{a:?} from {from:?} to {to:?}")),
        })
    }

    /// `x` as each lane's `m` (a uniform value in every lane, a thread id's lanes made).
    fn lanes(&mut self, x: &V, m: M) -> R<String> {
        // Lanes of a value that does not change, made once in their block.
        let key = match x {
            V::Iota(f, _) => Some(format!("iota {f}")),
            V::S(s, _) if konst(s).is_some() || self.hoisted.values().any(|h| h == s) => Some(format!("dup {s}")),
            _ => None,
        }
        .map(|k| if wide(m) { format!("wide {k}") } else { k });
        if let Some(k) = &key
            && let Some(e) = self.memo.iter().find(|e| &e.0 == k)
        {
            return Ok(e.1.clone());
        }
        let l = if wide(m) { self.lanes64(x, m)? } else { self.lanes_of(x, m)? };
        if let Some(k) = key {
            let d = self.ind;
            self.memo.push((k, l.clone(), d));
        }
        Ok(l)
    }

    /// `x` as each lane's 64-bit `m`.
    fn lanes64(&mut self, x: &V, m: M) -> R<String> {
        let widen = |l: &str, signed: bool, r: u32| {
            let h = if r % 2 == 0 { "low" } else { "high" };
            if signed {
                format!("vreinterpretq_u64_s64(vmovl_s32(vget_{h}_s32(vreinterpretq_s32_u32({l}[{}]))))", r / 2)
            } else {
                format!("vmovl_u32(vget_{h}_u32({l}[{}]))", r / 2)
            }
        };
        Ok(match x {
            V::L(l, lm) if wide(*lm) => l.clone(),
            V::L(l, lm) => {
                let (l, s) = (l.clone(), lm.signed());
                self.map64(|r| widen(&l, s, r))
            }
            V::S(s, sm) => {
                let e = if *sm == M::B { format!("if {s} {{ 1u64 }} else {{ 0 }}") } else { conv(s, *sm, M::U64) };
                let t = self.tmp();
                self.line(format!("let {t} = [vdupq_n_u64({e}); {}];", 2 * self.p));
                t
            }
            V::Iota(..) | V::K(_) | V::Cmp { .. } => {
                let l = self.lanes(x, M::U32)?;
                self.map64(|r| widen(&l, false, r))
            }
            // A lane's 32-bit index widened and scaled.
            V::At { base, idx, scale } if base.is_empty() => {
                let l = self.lanes(idx, M::U32)?;
                let w = self.map64(|r| widen(&l, false, r));
                match *scale {
                    1 => w,
                    k if k.is_power_of_two() => self.map64(|r| format!("vshlq_n_u64::<{}>({w}[{r}])", k.trailing_zeros())),
                    k => self.map64(|r| format!("vcombine_u64(vcreate_u64(vgetq_lane_u64::<0>({w}[{r}]).wrapping_mul({k})), vcreate_u64(vgetq_lane_u64::<1>({w}[{r}]).wrapping_mul({k})))")),
                }
            }
            x => return err(format!("{x:?} as lanes of {m:?}")),
        })
    }

    fn lanes_of(&mut self, x: &V, m: M) -> R<String> {
        Ok(match x {
            // 64-bit lanes as 32: each lane's low half.
            V::L(l, lm) if wide(*lm) => {
                let l = l.clone();
                self.map(|r| format!("vcombine_u32(vmovn_u64({l}[{}]), vmovn_u64({l}[{}]))", 2 * r, 2 * r + 1))
            }
            V::At { base, .. } if base.is_empty() => {
                let l = self.lanes64(x, M::U64)?;
                self.map(|r| format!("vcombine_u32(vmovn_u64({l}[{}]), vmovn_u64({l}[{}]))", 2 * r, 2 * r + 1))
            }
            V::L(l, _) => l.clone(),
            V::S(s, sm) => {
                let e = if *sm == M::B { format!("if {s} {{ 1 }} else {{ 0 }}") } else { conv(s, *sm, M::U32) };
                let t = self.tmp();
                self.line(format!("let {t} = [vdupq_n_u32({e}); {}];", self.p));
                t
            }
            V::Iota(f, _) => {
                let f = f.clone();
                self.map(|r| format!("vaddq_u32(vdupq_n_u32({f}), vld1q_u32(LANE[{r}].as_ptr()))"))
            }
            V::Cmp { .. } => {
                let k = self.mask(x)?;
                self.map(|r| format!("vshrq_n_u32::<31>({k}[{r}])"))
            }
            // A comparison's 0 or 1.
            V::K(k) => {
                let k = k.clone();
                self.map(|r| format!("vshrq_n_u32::<31>({k}[{r}])"))
            }
            x => return err(format!("{x:?} as lanes of {m:?}")),
        })
    }

    /// `x`'s truth in each lane, as a mask.
    fn mask(&mut self, x: &V) -> R<String> {
        Ok(match x {
            V::K(k) => k.clone(),
            V::Cmp { op, first, y } => {
                let l = self.lanes(&V::Iota(first.clone(), M::U32), M::U32)?;
                let yl = self.lanes(&V::S(y.clone(), M::U32), M::U32)?;
                let f = match op {
                    Op::Eq => "vceqq_u32",
                    Op::Lt => "vcltq_u32",
                    Op::Le => "vcleq_u32",
                    Op::Gt => "vcgtq_u32",
                    Op::Ge => "vcgeq_u32",
                    _ => "",
                };
                if f.is_empty() {
                    self.map(|r| format!("vmvnq_u32(vceqq_u32({l}[{r}], {yl}[{r}]))"))
                } else {
                    self.map(|r| format!("{f}({l}[{r}], {yl}[{r}])"))
                }
            }
            V::S(s, sm) => {
                let t = self.tmp();
                self.line(format!("let {t} = [vdupq_n_u32(if {} {{ u32::MAX }} else {{ 0 }}); {}];", truth(s, *sm), self.p));
                t
            }
            V::L(l, lm) if wide(*lm) => {
                let l = l.clone();
                let k = self.map64(|r| format!("vtstq_u64({l}[{r}], {l}[{r}])"));
                self.narrow_mask(&k)
            }
            x => {
                let l = self.lanes(x, M::U32)?;
                self.map(|r| format!("vtstq_u32({l}[{r}], {l}[{r}])"))
            }
        })
    }

    /// A load of `m` at `a` in the lanes of `cx` that run.
    fn load(&mut self, m: M, a: &V, cx: &Cx) -> R<V> {
        let r = self.load_at(m, a, cx);
        self.accessed(a, m, cx);
        r
    }

    fn load_at(&mut self, m: M, a: &V, cx: &Cx) -> R<V> {
        match a {
            V::S(s, _) => Ok(V::S(format!("({s} as *const {}).read_unaligned()", scalar(m)?), m)),
            V::At { base, idx, scale } if m.bytes() == 4 && *scale == 4 => {
                let cur = cx.cur.clone();
                match &**idx {
                    // Consecutive: a register's 4 lanes at once where all run, else a gather of their indexes.
                    V::Iota(first, _) if cx.full => {
                        let q = self.tmp();
                        self.line(format!("let {q} = ({base} as *const u32).add(({first}) as usize);"));
                        Ok(V::L(self.map(|r| format!("vld1q_u32({q}.add({}))", 4 * r)), m))
                    }
                    V::Iota(first, _) => {
                        let first = first.clone();
                        let t = self.tmp();
                        self.line(format!("let mut {t}: [uint32x4_t; {}];", self.p));
                        self.line(format!("if {} {{", self.all(&cur)));
                        self.ind += 1;
                        let q = self.tmp();
                        self.line(format!("let {q} = ({base} as *const u32).add(({first}) as usize);"));
                        let parts: Vec<String> = (0..self.p).map(|r| format!("vld1q_u32({q}.add({}))", 4 * r)).collect();
                        self.line(format!("{t} = [{}];", parts.join(", ")));
                        self.ind -= 1;
                        self.line("} else {");
                        self.ind += 1;
                        let l = self.lanes(&V::Iota(first, M::U32), M::U32)?;
                        let g = self.gather(base, &l, &cur);
                        self.line(format!("{t} = {g};"));
                        self.ind -= 1;
                        self.line("}");
                        Ok(V::L(t, m))
                    }
                    i => {
                        let l = self.lanes(i, M::U32)?;
                        let g = if cx.full { self.gather_all(base, &l) } else { self.gather(base, &l, &cur) };
                        Ok(V::L(g, m))
                    }
                }
            }
            // Each lane's address: a load of each (a lane that does not run reads where its address was made:
            // the base's first bytes, `address`).
            V::L(at, am) if wide(*am) => {
                if !cx.full {
                    self.risky = true;
                }
                let (at, t) = (at.clone(), self.tmp());
                if wide(m) {
                    self.line(format!("let mut {t}: [uint64x2_t; {}] = {};", 2 * self.p, self.zeros_of(m)));
                    for j in 0..self.w {
                        let (r, q) = (j / 2, j % 2);
                        self.line(format!("{t}[{r}] = vsetq_lane_u64::<{q}>((vgetq_lane_u64::<{q}>({at}[{r}]) as usize as *const u64).read_unaligned(), {t}[{r}]);"));
                    }
                } else {
                    let st = scalar(m)?;
                    self.line(format!("let mut {t}: [uint32x4_t; {}] = {};", self.p, self.zeros()));
                    for j in 0..self.w {
                        let (r, q, ar, aq) = (j / 4, j % 4, j / 2, j % 2);
                        self.line(format!("{t}[{r}] = vsetq_lane_u32::<{q}>((vgetq_lane_u64::<{aq}>({at}[{ar}]) as usize as *const {st}).read_unaligned() as u32, {t}[{r}]);"));
                    }
                }
                Ok(V::L(t, m))
            }
            a => err(format!("a load of {m:?} at {a:?}")),
        }
    }

    /// Each lane's `u32` at `base` plus its index `l` times 4, every lane running.
    fn gather_all(&mut self, base: &str, l: &str) -> String {
        let b = self.tmp();
        self.line(format!("let {b} = {base} as *const u32;"));
        let t = self.tmp();
        self.line(format!("let mut {t}: [uint32x4_t; {}] = {};", self.p, self.zeros()));
        for j in 0..self.w {
            let (r, q) = (j / 4, j % 4);
            self.line(format!("{t}[{r}] = vld1q_lane_u32::<{q}>({b}.add(vgetq_lane_u32::<{q}>({l}[{r}]) as usize), {t}[{r}]);"));
        }
        t
    }

    /// Each lane's `u32` at `base` plus its index `l` times 4, without a branch: a lane that does not run (in
    /// `cur`) loads a running one's element (the largest index of those), as some lane of `cur` runs wherever a
    /// statement does (each is under a test that any does).
    fn gather(&mut self, base: &str, l: &str, cur: &str) -> String {
        // A lane's own element, where every lane's is valid: its address for a lane that does not run.
        if let Some(k) = root(base)
            && self.is_valid(k)
        {
            let own = self.lanes(&V::Iota(GID.into(), M::U32), M::U32).unwrap_or_default();
            let ix = self.map(|r| format!("vbslq_u32({cur}[{r}], {l}[{r}], {own}[{r}])"));
            return self.gather_all(base, &ix);
        }
        self.risky = true;
        let mut e = format!("vandq_u32({cur}[0], {l}[0])");
        for r in 1..self.p {
            e = format!("vmaxq_u32({e}, vandq_u32({cur}[{r}], {l}[{r}]))");
        }
        let s = self.tmp();
        self.line(format!("let {s} = vdupq_n_u32(vmaxvq_u32({e}));"));
        let ix = self.map(|r| format!("vbslq_u32({cur}[{r}], {l}[{r}], {s})"));
        let b = self.tmp();
        self.line(format!("let {b} = {base} as *const u32;"));
        let t = self.tmp();
        self.line(format!("let mut {t}: [uint32x4_t; {}] = {};", self.p, self.zeros()));
        for j in 0..self.w {
            let (r, q) = (j / 4, j % 4);
            self.line(format!("{t}[{r}] = vld1q_lane_u32::<{q}>({b}.add(vgetq_lane_u32::<{q}>({ix}[{r}]) as usize), {t}[{r}]);"));
        }
        t
    }

    /// A store of `x` (of `m`) at `a` in the lanes of `cx` that run.
    fn store(&mut self, m: M, a: &V, x: &V, cx: &Cx) -> R<()> {
        let k = match a {
            V::S(s, _) => root_of(s),
            V::At { base, .. } => root(base),
            _ => None,
        };
        let own = self.own(a, m).is_some();
        self.writes.push((k, own));
        // Every lane's own element, each valid, of a buffer whose every write is a thread's own element: a lane
        // that does not run writes back what is there (no other thread writes it), the registers stored whole.
        if !cx.full
            && let Some(k) = self.own(a, m)
            && self.is_valid(k)
            && self.disjoint.contains(&k)
            && let V::At { base, idx, .. } = a
            && let V::Iota(first, _) = &**idx
        {
            let l = self.lanes(x, m)?;
            let cur = cx.cur.clone();
            let q = self.tmp();
            self.line(format!("let {q} = ({base} as *mut u32).add(({first}) as usize);"));
            for r in 0..self.p {
                self.line(format!("vst1q_u32({q}.add({o}), vbslq_u32({cur}[{r}], {l}[{r}], vld1q_u32({q}.add({o}))));", o = 4 * r));
            }
            return Ok(());
        }
        let r = self.store_at(m, a, x, cx);
        self.accessed(a, m, cx);
        r
    }

    fn store_at(&mut self, m: M, a: &V, x: &V, cx: &Cx) -> R<()> {
        match a {
            V::S(s, _) => {
                let x = match x {
                    V::S(e, xm) => conv(e, *xm, m),
                    _ => {
                        // An address this pass takes for one value, a call's result a later pass may find each lane's.
                        self.pending.get_or_insert_with(|| "a store of lanes' values at one address".into());
                        return Ok(());
                    }
                };
                self.line(format!("({s} as *mut {}).write_unaligned({x});", scalar(m)?));
                Ok(())
            }
            V::At { base, idx, scale } if m.bytes() == 4 && *scale == 4 => {
                let l = self.lanes(x, m)?;
                let cur = cx.cur.clone();
                match &**idx {
                    V::Iota(first, _) if cx.full => {
                        let q = self.tmp();
                        self.line(format!("let {q} = ({base} as *mut u32).add(({first}) as usize);"));
                        for r in 0..self.p {
                            self.line(format!("vst1q_u32({q}.add({}), {l}[{r}]);", 4 * r));
                        }
                    }
                    i if cx.full => {
                        let ix = self.lanes(i, M::U32)?;
                        for j in 0..self.w {
                            let (r, q) = (j / 4, j % 4);
                            self.line(format!("({base} as *mut u32).add(vgetq_lane_u32::<{q}>({ix}[{r}]) as usize).write(vgetq_lane_u32::<{q}>({l}[{r}]));"));
                        }
                    }
                    V::Iota(first, _) => {
                        let q = self.tmp();
                        self.line(format!("let {q} = ({base} as *mut u32).add(({first}) as usize);"));
                        self.line(format!("if {} {{", self.all(&cur)));
                        self.ind += 1;
                        for r in 0..self.p {
                            self.line(format!("vst1q_u32({q}.add({}), {l}[{r}]);", 4 * r));
                        }
                        self.ind -= 1;
                        self.line("} else {");
                        self.ind += 1;
                        // Each lane's element, or (a lane that does not run) a word of the stack's: no branch.
                        // (its address opaque, so the compiler keeps the select and does not branch instead).
                        let sink = self.tmp();
                        self.line(format!("let mut {sink}w = 0u32;"));
                        self.line(format!("let {sink} = core::hint::black_box(&raw mut {sink}w);"));
                        for j in 0..self.w {
                            let on = self.on(&cur, j);
                            self.line(format!("core::hint::select_unpredictable({on}, {q}.add({j}), {sink}).write(vgetq_lane_u32::<{}>({l}[{}]));", j % 4, j / 4));
                        }
                        self.ind -= 1;
                        self.line("}");
                    }
                    i => {
                        let ix = self.lanes(i, M::U32)?;
                        // (its address opaque, so the compiler keeps the select and does not branch instead).
                        let sink = self.tmp();
                        self.line(format!("let mut {sink}w = 0u32;"));
                        self.line(format!("let {sink} = core::hint::black_box(&raw mut {sink}w);"));
                        for j in 0..self.w {
                            let on = self.on(&cur, j);
                            let (r, q) = (j / 4, j % 4);
                            self.line(format!("core::hint::select_unpredictable({on}, ({base} as *mut u32).add(vgetq_lane_u32::<{q}>({ix}[{r}]) as usize), {sink}).write(vgetq_lane_u32::<{q}>({l}[{r}]));"));
                        }
                    }
                }
                Ok(())
            }
            a => err(format!("a store of {m:?} at {a:?}")),
        }
    }

    fn builtin(&mut self, n: usize, cx: &Cx) -> R<V> {
        let b = Builtin::of(self.v.w(n, 0));
        let m = M(self.v.w(n, 1));
        let kids: Vec<usize> = self.v.children(n).collect();
        let mut args = Vec::new();
        for &c in &kids {
            args.push(self.expr(c, cx)?);
        }
        let dim = |a: &V| -> R<usize> {
            match a {
                V::S(s, _) => konst(s).map(|d| d as usize).ok_or_else(|| Error(format!("the Rust of a kernel: a position of a dimension not constant: {s}"))),
                _ => err("a position of each lane's dimension"),
            }
        };
        Ok(match b {
            Builtin::LocalId | Builtin::GlobalId => {
                let d = dim(&args[0])?;
                match (b, d) {
                    (Builtin::GlobalId, 0) => V::Iota(GID.into(), M::U32),
                    (_, 0) => V::Iota("lid0".into(), M::U32),
                    (Builtin::GlobalId, d) => V::S(format!("(l.group[{d}] * l.block[{d}])"), M::U32),
                    _ => V::S("0u32".into(), M::U32),
                }
            }
            Builtin::LocalSize => V::S(format!("l.block[{}]", dim(&args[0])?), M::U32),
            Builtin::GroupId => V::S(format!("l.group[{}]", dim(&args[0])?), M::U32),
            Builtin::NumGroups => V::S(format!("l.groups[{}]", dim(&args[0])?), M::U32),
            Builtin::GlobalSize => {
                let d = dim(&args[0])?;
                V::S(format!("(l.groups[{d}] * l.block[{d}])"), M::U32)
            }
            Builtin::MemFence | Builtin::Yield => V::None,
            Builtin::AtomicLoad => self.load(m, &args[0], cx)?,
            Builtin::AtomicStore => {
                self.store(m, &args[0], &args[1], cx)?;
                V::None
            }
            Builtin::AtomicAdd | Builtin::AtomicMax | Builtin::AtomicMin | Builtin::AtomicOr | Builtin::AtomicAnd if m.bytes() == 4 && !m.signed() => {
                let unread = self.unread(n);
                let k = match &args[0] {
                    V::S(s, _) => root_of(s),
                    V::At { base, .. } => root(base),
                    _ => None,
                };
                self.writes.push((k, false));
                let f = match b {
                    Builtin::AtomicAdd => "fetch_add",
                    Builtin::AtomicMax => "fetch_max",
                    Builtin::AtomicMin => "fetch_min",
                    Builtin::AtomicOr => "fetch_or",
                    _ => "fetch_and",
                };
                match (&args[0], &args[1]) {
                    // One address and one value: each running lane's own atomic, in lane order. An add is one atomic
                    // of the value times the running lanes, each lane's old value the first plus the value times the
                    // running lanes below it; another operation, one atomic a running lane.
                    (V::S(p, _), V::S(x, xm)) if b == Builtin::AtomicAdd => {
                        let x = conv(x, *xm, M::U32);
                        let (w, pp) = (self.w, self.p);
                        let (rank, count) = if cx.full {
                            let rk = self.map(|r| format!("vld1q_u32(LANE[{r}].as_ptr())"));
                            (rk, format!("{w}u32"))
                        } else {
                            let cur = cx.cur.clone();
                            let (km, rm, c) = (self.tmp(), self.tmp(), self.tmp());
                            self.line(format!("let mut {km} = [0u32; {w}];"));
                            self.line(format!("for r in 0..{pp} {{ vst1q_u32({km}.as_mut_ptr().add(4 * r), {cur}[r]); }}"));
                            self.line(format!("let mut {rm} = [0u32; {w}];"));
                            self.line(format!("let mut {c} = 0u32;"));
                            self.line(format!("for j in 0..{w} {{ {rm}[j] = {c}; {c} += {km}[j] & 1; }}"));
                            let rk = self.map(|r| format!("vld1q_u32({rm}.as_ptr().add({}))", 4 * r));
                            (rk, c)
                        };
                        let t = self.tmp();
                        self.line(format!("let {t} = AtomicU32::from_ptr({p} as *mut u32).fetch_add(({x}).wrapping_mul({count}), Relaxed);"));
                        if unread {
                            return Ok(V::S(t, m));
                        }
                        V::L(self.map(|r| format!("vmlaq_u32(vdupq_n_u32({t}), {rank}[{r}], vdupq_n_u32({x}))")), m)
                    }
                    // Another operation: each running lane's atomic in turn, its old value its own.
                    (V::S(p, _), V::S(x, xm)) => {
                        let x = conv(x, *xm, M::U32);
                        let (w, pp) = (self.w, self.p);
                        let (km, ov) = (self.tmp(), self.tmp());
                        let on = if cx.full { "vdupq_n_u32(u32::MAX)".to_string() } else { format!("{}[r]", cx.cur) };
                        self.line(format!("let mut {km} = [0u32; {w}];"));
                        self.line(format!("for r in 0..{pp} {{ vst1q_u32({km}.as_mut_ptr().add(4 * r), {on}); }}"));
                        self.line(format!("let mut {ov} = [0u32; {w}];"));
                        self.line(format!("for j in 0..{w} {{ if {km}[j] != 0 {{ {ov}[j] = AtomicU32::from_ptr({p} as *mut u32).{f}({x}, Relaxed); }} }}"));
                        V::L(self.map(|r| format!("vld1q_u32({ov}.as_ptr().add({}))", 4 * r)), m)
                    }
                    // One address, each lane's value, its old value read by none: the lanes' combination, one atomic.
                    (V::S(p, _), x) if unread => {
                        let l = self.lanes(x, m)?;
                        let cur = cx.cur.clone();
                        let (id, op, red) = match b {
                            Builtin::AtomicAdd => ("0", "vaddq_u32", "vaddvq_u32"),
                            Builtin::AtomicMax => ("0", "vmaxq_u32", "vmaxvq_u32"),
                            Builtin::AtomicMin => ("u32::MAX", "vminq_u32", "vminvq_u32"),
                            Builtin::AtomicOr => ("0", "vorrq_u32", "vmaxvq_u32"),
                            _ => return err("an and of lanes at one address"),
                        };
                        if b == Builtin::AtomicOr {
                            return err("an or of lanes at one address");
                        }
                        // Lanes that do not run add the identity (none where all run); the atomic is skipped where the
                        // lanes' combination is the identity (it would change nothing).
                        let k = if cx.full { l } else { self.map(|r| format!("vbslq_u32({cur}[{r}], {l}[{r}], vdupq_n_u32({id}))")) };
                        let mut e = format!("{k}[0]");
                        for r in 1..self.p {
                            e = format!("{op}({e}, {k}[{r}])");
                        }
                        let t = self.tmp();
                        self.line(format!("let {t} = {red}({e});"));
                        self.line(format!("if {t} != {id} {{ AtomicU32::from_ptr({p} as *mut u32).{f}({t}, Relaxed); }}"));
                        V::None
                    }
                    // Each lane's address: each running lane's atomic in turn, its old value its own.
                    (V::At { base, idx, scale }, x) => {
                        let (base, scale, idx) = (base.clone(), *scale, (**idx).clone());
                        let (il, xl) = (self.lanes(&idx, M::U32)?, self.lanes(x, M::U32)?);
                        let (w, pp) = (self.w, self.p);
                        let (km, ia, xa, ov) = (self.tmp(), self.tmp(), self.tmp(), self.tmp());
                        let on = if cx.full { "vdupq_n_u32(u32::MAX)".to_string() } else { format!("{}[r]", cx.cur) };
                        self.line(format!("let (mut {km}, mut {ia}, mut {xa}, mut {ov}) = ([0u32; {w}], [0u32; {w}], [0u32; {w}], [0u32; {w}]);"));
                        self.line(format!("for r in 0..{pp} {{ vst1q_u32({km}.as_mut_ptr().add(4 * r), {on}); vst1q_u32({ia}.as_mut_ptr().add(4 * r), {il}[r]); vst1q_u32({xa}.as_mut_ptr().add(4 * r), {xl}[r]); }}"));
                        self.line(format!("for j in 0..{w} {{ if {km}[j] != 0 {{ {ov}[j] = AtomicU32::from_ptr(({base} as *mut u8).add({ia}[j] as usize * {scale}) as *mut u32).{f}({xa}[j], Relaxed); }} }}"));
                        V::L(self.map(|r| format!("vld1q_u32({ov}.as_ptr().add({}))", 4 * r)), m)
                    }
                    _ => return err(format!("{b:?} of each lane's address")),
                }
            }
            // The ones of each lane's `m`: each byte's (`vcnt`), summed pairwise up to the lane's width.
            Builtin::Popcount => match &args[0] {
                V::S(s, _) => V::S(format!("((({s}) as {}).count_ones() as {})", scalar(m).unwrap_or("u32"), scalar(m).unwrap_or("u32")), m),
                a if wide(m) => {
                    let l = self.lanes(a, m)?;
                    V::L(self.map64(|r| format!("vpaddlq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u64({l}[{r}])))))")), m)
                }
                a => {
                    let l = self.lanes(a, m)?;
                    V::L(self.map(|r| format!("vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32({l}[{r}]))))")), m)
                }
            },
            b => return err(format!("the intrinsic {b:?}")),
        })
    }

    /// Whether node `n`'s value is read by none (its parent an expression statement).
    fn unread(&self, n: usize) -> bool {
        let mut q = n;
        while q > 0 {
            q -= 1;
            if q + self.v.size(q) > n {
                return self.v.kind(q) == Kind::Expr || (self.v.kind(q) == Kind::Return && self.v.w(q, 2) != NO_SLOT && self.body_unread(q));
            }
        }
        false
    }

    /// Whether the function whose `Return` is node `ret` has its result read by none of its calls.
    fn body_unread(&self, ret: usize) -> bool {
        let Some((&id, _)) = self.bodies.iter().filter(|&(_, &b)| b < ret && ret < b + self.v.size(b)).max_by_key(|&(_, &b)| b) else { return false };
        (0..self.v.len()).filter(|&i| self.v.kind(i) == Kind::Call && self.v.w(i, 3) == id).all(|i| self.unread(i))
    }
}

/// Whether each lane's `m` takes 64 bits (two lanes a register): a 64-bit integer, or an address.
fn wide(m: M) -> bool {
    m.bytes() == 8 && !m.is_agg() && !m.float()
}

/// Register `r` (of `2P`) of mask `k` (of `P`), as a mask of 64-bit lanes.
fn wmask(k: &str, r: u32) -> String {
    let h = if r % 2 == 0 { "low" } else { "high" };
    format!("vreinterpretq_u64_s64(vmovl_s32(vget_{h}_s32(vreinterpretq_s32_u32({k}[{}]))))", r / 2)
}

/// Function `id`'s instance of `shapes`' name.
fn fname(id: u32, shapes: &[Sh], full: bool, valid: &[u32]) -> String {
    let mut s = format!("b{id}_");
    for sh in shapes {
        s.push_str(&sh.code());
    }
    s.push_str(if full { "_f" } else { "_p" });
    for k in valid {
        s.push_str(&format!("_v{k}"));
    }
    s
}

/// `!(a op b)` as `a op' b`.
fn negated(op: Op) -> Op {
    match op {
        Op::Lt => Op::Ge,
        Op::Ge => Op::Lt,
        Op::Le => Op::Gt,
        Op::Gt => Op::Le,
        Op::Eq => Op::Ne,
        _ => Op::Eq,
    }
}

/// `a op b` as `b op' a`.
fn swapped(op: Op) -> Op {
    match op {
        Op::Lt => Op::Gt,
        Op::Gt => Op::Lt,
        Op::Le => Op::Ge,
        Op::Ge => Op::Le,
        op => op,
    }
}

/// The value of a constant as `literal` writes it (`4u32`, `(4usize as *const u8)`), if `s` is one.
fn konst(s: &str) -> Option<u32> {
    let s = s.strip_prefix('(').and_then(|s| s.strip_suffix(" as *const u8)")).unwrap_or(s);
    let digits = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    ["", "u8", "i8", "u16", "i16", "u32", "i32", "u64", "i64", "usize"].contains(&&s[digits..]).then(|| s[..digits].parse().ok()).flatten()
}

/// `m`'s Rust scalar type.
fn scalar(m: M) -> R<&'static str> {
    Ok(match m {
        M::B => "bool",
        M::I8 => "i8",
        M::U8 => "u8",
        M::I16 => "i16",
        M::U16 => "u16",
        M::I32 => "i32",
        M::U32 => "u32",
        M::I64 => "i64",
        M::U64 => "u64",
        M::F32 => "f32",
        M::F64 => "f64",
        M::PTR => "*const u8",
        m => return err(format!("a scalar of {m:?}")),
    })
}

/// Constant `x` of `m` as a Rust literal.
fn literal(m: M, x: u64) -> R<String> {
    Ok(match m {
        M::B => (x != 0).to_string(),
        M::F32 => format!("f32::from_bits({:#x})", x as u32),
        M::F64 => format!("f64::from_bits({x:#x})"),
        M::PTR => format!("({x}usize as *const u8)"),
        m if m.signed() => format!("{}{}", crate::ops::norm(m, x) as i64, scalar(m)?),
        m => format!("{}{}", crate::ops::norm(m, x), scalar(m)?),
    })
}

/// Scalar `s` of `from` as `to`.
fn conv(s: &str, from: M, to: M) -> String {
    if from == to || (from == M::PTR && to == M::PTR) {
        return s.to_string();
    }
    match (from, to) {
        (M::B, t) => format!("(({s}) as {})", scalar(t).unwrap_or("u32")),
        (f, M::PTR) => format!("(({s}) as {} as usize as *const u8)", scalar(f).unwrap_or("u64")),
        (M::PTR, t) => format!("(({s}) as usize as {})", scalar(t).unwrap_or("u64")),
        (_, t) => format!("(({s}) as {})", scalar(t).unwrap_or("u32")),
    }
}

/// Scalar `s` of `m` as a `bool`.
fn truth(s: &str, m: M) -> String {
    if m == M::B { s.to_string() } else if m == M::PTR { format!("!({s}).is_null()") } else { format!("(({s}) != 0)") }
}

fn scalar_bin(op: Op, m: M, x: &str, y: &str) -> R<String> {
    let t = scalar(m)?;
    if m == M::PTR {
        return Ok(match op {
            Op::Add => format!("({x}).wrapping_add(({y}) as usize)"),
            Op::Eq => format!("(({x}) == ({y}))"),
            Op::Ne => format!("(({x}) != ({y}))"),
            // An offset, typed a pointer: its integer's.
            Op::Sub => format!("(({x}) as usize).wrapping_sub(({y}) as usize) as *const u8"),
            Op::Mul => format!("((({x}) as usize).wrapping_mul(({y}) as usize) as *const u8)"),
            Op::Shl => format!("((({x}) as usize).wrapping_shl(({y}) as u32) as *const u8)"),
            _ => return err(format!("{op:?} of pointers")),
        });
    }
    Ok(match op {
        Op::Add if m.float() => format!("(({x}) + ({y}))"),
        Op::Add => format!("({x}).wrapping_add({y})"),
        Op::Sub if m.float() => format!("(({x}) - ({y}))"),
        Op::Sub => format!("({x}).wrapping_sub({y})"),
        Op::Mul if m.float() => format!("(({x}) * ({y}))"),
        Op::Mul => format!("({x}).wrapping_mul({y})"),
        Op::Div if m.float() => format!("(({x}) / ({y}))"),
        Op::Div => format!("({x}).checked_div({y}).unwrap_or(0)"),
        Op::Rem => format!("({x}).checked_rem({y}).unwrap_or(0)"),
        Op::Shl => format!("({x}).wrapping_shl(({y}) as u32)"),
        Op::Shr => format!("({x}).wrapping_shr(({y}) as u32)"),
        Op::And => format!("(({x}) & ({y}))"),
        Op::Or => format!("(({x}) | ({y}))"),
        Op::Xor => format!("(({x}) ^ ({y}))"),
        Op::Eq => format!("(({x}) == ({y}))"),
        Op::Ne => format!("(({x}) != ({y}))"),
        Op::Lt => format!("(({x}) < ({y}))"),
        Op::Le => format!("(({x}) <= ({y}))"),
        Op::Gt => format!("(({x}) > ({y}))"),
        Op::Ge => format!("(({x}) >= ({y}))"),
        op => return err(format!("a binary {op:?} of {t}")),
    })
}
