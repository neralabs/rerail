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
use std::collections::{BTreeMap, HashMap, HashSet};

type R<T> = Result<T, Error>;

fn err<T>(what: impl std::fmt::Display) -> R<T> {
    Err(Error(format!("the Rust of a kernel: {what}")))
}

/// A statement of a function's phase (`Em::phases`): its own, phase `j` of the call of a function with phases, or
/// an unrolled loop's counter (its node) set to a round's value.
#[derive(Clone, Copy, Debug)]
enum Part {
    S(usize),
    Sub(usize, u32),
    Set(usize, u32),
}

/// A value as the Rust being written holds it.
#[derive(Clone, Debug)]
enum V {
    /// The same in every lane: a Rust expression of `m`'s scalar type (`bool` for `M::B`).
    S(String, M),
    /// Each lane's own: a name of `[uint32x4_t; P]` (32-bit `m`), or of `[uint32x4_t; 2P]` (64-bit `m`, a pointer
    /// too: each lane's address, an integer), its lanes' low halves the first `P` registers and their high halves
    /// the next: lane `j` in register `j / 4`'s element `j % 4` of each, as a 32-bit lane is, so a mask's register
    /// selects both halves and a value cut to 32 bits is its low ones.
    L(String, M),
    /// Each lane's 64-bit `m` whose high half is 0 (a 32-bit value widened, a value masked to 32 bits or fewer): a
    /// name of its low halves' `[uint32x4_t; P]`, the operations on it of 32 bits where their high halves stay 0.
    Z(String),
    /// Lanes `first + j`, `j` the lane: a thread's id along its group's first dimension.
    Iota(String, M),
    /// A mask (a `bool` of each lane, or a C comparison's 0 or 1): `[uint32x4_t; P]`, a lane all ones where true.
    K(String),
    /// Lanes `first + j * stride` (`u32`s, wrapping), `stride` one value known as the program runs: consecutive
    /// lanes times a value (a row of a table of a width read from memory).
    Lin(String, String),
    /// Each lane's address: a uniform base (a pointer, or none: a 64-bit offset, an integer) plus its 32-bit index
    /// (consecutive lanes, `Lin`'s, or each lane's) times `scale` bytes.
    At { base: String, idx: Box<V>, scale: u32 },
    /// Lanes `first + j` (`u32`s) compared with one value `y` (unsigned): a mask, which a chunk of consecutive
    /// lanes may decide at once (all on one side of `y`).
    Cmp { op: Op, first: String, y: String },
    /// A thread's variable's address, each lane's its own variable (`C`'s `&x`): the Rust place of its registers
    /// (each lane's, `[uint32x4_t; P]` or `2P`), and the variable's type. A function it is passed to takes the
    /// registers by `&mut` (`Sh::Ref`); a load, store or update through it is one of the variable, in place.
    Ref(String, M),
    /// A thread's array's address (`array`): the Rust place of its registers, its elements and their bytes. A
    /// function it is passed to takes them by `&mut` (`Sh::Arr`); its accesses through it are its elements'.
    Arr(String, u32, u32),
    /// No value (a call of a function without one).
    None,
}

/// Each lane's 64-bit value as its halves, an expression of each register: the low halves', and the high halves'
/// unless they are 0.
#[derive(Clone, Debug)]
struct H {
    lo: Vec<String>,
    hi: Option<Vec<String>>,
}

/// A thread's variable, kept in a Rust variable.
#[derive(Clone, Debug)]
struct Var {
    name: String,
    m: M,
    /// Each lane's (else one, the same in every lane).
    lanes: bool,
    /// Lanes `first + j` (`j` the lane) where it is not each lane's: a `u32` of `first`, every store of it one of
    /// consecutive lanes where all of them run (else each lane's, from the next pass).
    iota: bool,
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
    /// Lanes `x + j` (`x` a `u32`) of a chunk that crosses no multiple of `2^K`: lanes that share their bits from
    /// `K` up, so their shift by `K` or more is one value, and their low bits up to `K` or more consecutive lanes.
    /// A caller passes it where it tested its chunk (`Em::wants`).
    Run(u32),
    /// Lanes `first + j * stride` (`V::Lin`): two `u32`s, `{name}f` and `{name}s`.
    Lin,
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
    /// A caller's thread variable's address (`V::Ref`), its type's word: its registers, by `&mut`.
    Ref(u32),
    /// A caller's thread array (`V::Arr`): its elements and their bytes, its registers by `&mut`.
    Arr(u32, u32),
}

/// An index of each lane, as a parameter passes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Ix {
    L,
    Iota,
    Lin,
    Gid,
}

/// The thread's id in its launch's first dimension, as every function names it.
const GID: &str = "gid0";

/// The kernel's buffer argument `s` is, if it is one (`arg{k}`).
fn root(s: &str) -> Option<u32> {
    s.strip_prefix("arg").and_then(|k| k.parse().ok())
}

/// Address `a` (one lane's or each lane's) plus `inner` bytes.
fn offset_addr(a: V, inner: u32) -> R<V> {
    Ok(match a {
        a if inner == 0 => a,
        V::S(s, _) => V::S(format!("{s}.wrapping_add({inner})"), M::PTR),
        V::At { base, idx, scale } if !base.is_empty() => V::At { base: format!("{base}.wrapping_add({inner})"), idx, scale },
        a => return err(format!("byte {inner} of a structure at {a:?}")),
    })
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
            Sh::Run(k) => format!("u{k}"),
            Sh::Lin => "n".into(),
            Sh::Gid => "g".into(),
            Sh::L => "l".into(),
            Sh::Arg(k) => format!("r{k}"),
            Sh::In(k) => format!("o{k}"),
            Sh::Ref(m) => format!("p{m}"),
            Sh::Arr(e, b) => format!("q{e}x{b}"),
            Sh::At(k, ix, r) => format!("a{}{k}{}", match ix { Ix::L => "l", Ix::Iota => "i", Ix::Lin => "n", Ix::Gid => "g" }, r.map(|r| format!("r{r}")).unwrap_or_default()),
        }
    }

    fn of(x: &V) -> Sh {
        match x {
            V::S(s, M::PTR) if root(s).is_some() => Sh::Arg(root(s).unwrap()),
            V::S(s, M::PTR | M::U64) if root_of(s).is_some() => Sh::In(root_of(s).unwrap()),
            V::S(..) | V::None => Sh::S,
            V::Iota(f, _) if f == GID => Sh::Gid,
            V::Iota(..) => Sh::Iota,
            V::Lin(..) => Sh::Lin,
            V::At { base, idx, scale } => Sh::At(
                *scale,
                match &**idx {
                    V::Iota(f, _) if f == GID => Ix::Gid,
                    V::Iota(..) => Ix::Iota,
                    V::Lin(..) => Ix::Lin,
                    _ => Ix::L,
                },
                root(base),
            ),
            V::L(..) | V::Z(..) | V::K(..) | V::Cmp { .. } => Sh::L,
            V::Ref(_, m) => Sh::Ref(m.0),
            V::Arr(_, e, b) => Sh::Arr(*e, *b),
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
    /// The label of the loop whose `break` all its lanes take together (Rust's `break`), where `brk` is not.
    exit: Option<String>,
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
    /// Whether the lanes are apart only by returns, at the function's own level (no loop or branch of lanes
    /// around): a lane off has returned, so it reads none of the function's variables again.
    left: bool,
}

/// A function's instance: its number, its parameters' shapes, whether all its lanes run (`_f`, else `_p`), and the
/// kernel's buffers whose every lane's own element (at `gid0`) the caller has accessed, so is valid (`_v{k}`).
type Inst = (u32, Vec<Sh>, bool, Vec<u32>);

struct Em<'a> {
    /// The program's constant memory (`Program::data`): what a `Global` addresses.
    data: &'a [u8],
    /// Each of the program's functions: whether it is declared `noinline` (`UKL_SHARED`, `UKL_LARGE`, `UKL_COLD`:
    /// `lower::Func::called`). An instance of one longer than `CALLED_LINES` is the compiler's to inline or call
    /// (`#[inline]`): inlined everywhere, `join_run`'s `clif_verdict_entry` and the like made its Rust 9.5 MB of
    /// code. Every other instance is inlined: the label passes' reads (54 lines at most at 8 lanes), left to LLVM,
    /// stayed calls, and `label_parents` and `label_leaves` each took 8% more instructions (decision 0567).
    called: Vec<bool>,
    /// Each function's name in the source.
    names: Vec<String>,
    v: View<'a>,
    w: u32,
    p: u32,
    /// Each function's `Body` node, by number.
    bodies: HashMap<u32, usize>,
    /// What one pass found, for the next: each variable's lanes, each function's instances (one of each shape of
    /// its calls' arguments, as inlining would make) and each one's result's lanes.
    lanes: HashMap<(u32, u32), bool>,
    /// The variables found consecutive lanes (`Var::iota`), for the next pass.
    iotas: std::collections::HashSet<(u32, u32)>,
    /// Pointer variables that only ever hold one thread variable's address (`p = &x`): `x`, through them (`V::Ref`).
    refs: HashMap<(u32, u32), V>,
    /// A thread's structures held as their fields (`fields`), by instance and variable: each field's byte and type.
    structs: HashMap<(u32, u32), Vec<(u32, M)>>,
    /// A thread's structures held in memory of the chunk's (`mem`), by instance and variable: the Rust name of their
    /// bytes, a row a lane (`[[u8; size]; W]`), and the size.
    mems: HashMap<(u32, u32), (String, u32)>,
    /// The function being written's parameters of consecutive lanes: of a chunk known to cross no `2^K`
    /// (`Sh::Run`), each one's `K`; of any (`Sh::Iota`), each one's function and parameter number.
    runs: HashMap<String, u32>,
    iota_params: HashMap<String, (u32, usize)>,
    /// The `?:`s and short circuits of one condition (by instance and node) found an arm or a right side of lanes,
    /// for the next pass: the others' value is one, though the tape's level says lanes (an instance of uniform
    /// arguments).
    lane_conds: std::collections::HashSet<(u32, usize)>,
    /// The names of the lanes' variables' copies of one lane, while a loop's scalar rounds are written
    /// (`finish_scalar`), and whether a store there was of lanes (the rounds then not written).
    shadows: std::collections::HashSet<String>,
    shadow_fail: bool,
    /// The `K` each function's parameter of consecutive lanes would take `Sh::Run` of (its smallest shift or mask
    /// that wants it, or its callee's), kept from pass to pass: a call that has such lanes tests its chunk.
    wants: HashMap<(u32, usize), u32>,
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
    /// The kernel's buffers the last pass wrote (`None`: a write it did not know the buffer of): a copy of a
    /// structure from another is read where it lies (`aliases`, the variables so read).
    written: Option<std::collections::BTreeSet<u32>>,
    aliases: std::collections::HashSet<(u32, u32)>,
    /// A thread's array (a variable reached at a byte offset of one value for all lanes, its every access of 4 bytes,
    /// or of 8): its Rust name, its elements, and their bytes, each element the chunk's lanes' registers (`elem`).
    arrays: HashMap<(u32, u32), (String, u32, u32)>,
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
    /// The phase of a called function with phases the call being written runs, and its state's offset in the
    /// caller's (`Part::Sub`).
    sub: Option<(u32, usize)>,
    /// What this pass could not write, which a later pass may (a call's result it finds of each lane): an error if the
    /// passes settle with it.
    pending: Option<String>,
    /// The group is one chunk, one SIMD group (`kernel_group`): its threads run in step, so a barrier is met where
    /// it is written, and a SIMD group's operation is one across the chunk's lanes.
    one: bool,
    /// Each function (by its number in the program) called as compiled code of its own (`Read`), not written here.
    compiled: HashMap<u32, Read>,
    /// The structures (by context and variable) copied into whole where the copy cannot read them where they lie (its
    /// source's bytes a write of the kernel reaches): fields, though only read after (`fields`).
    unaliased: HashSet<(u32, u32)>,
}

/// A function of the program that a kernel's Rust calls as compiled code of its own, through the launch's table of
/// them (`Launch::reads`, by `index`), not written as its vector code: a read written once in CLIF (decision 0310),
/// whose UKL the kernel's source includes (`<ukl/reads/*.h>`) and which the CPU has compiled (`rerail_clif::jit`).
/// Its UKL takes a base `t` and the offset `a0` of its first pointer from it, and computes every address as an
/// offset from `t`; its compiled code takes that pointer (`t + a0`), its other arguments as the UKL's, and computes
/// addresses.
#[derive(Clone, Debug)]
pub struct Read {
    /// Its name in the program (the UKL's, `clif_rows_at`).
    pub name: String,
    /// Its place in the launch's table.
    pub index: u32,
    /// Its result is an address from `t` (the UKL's: the compiled code's less `t`).
    pub result: bool,
    /// Each of its UKL arguments past `t` and `a0`: an offset from `t` (passed to the compiled code plus `t`).
    pub offsets: Vec<bool>,
}

/// Kernel `name`'s vector tape `k` (of program `prog`, whose constant memory it reads) as Rust at `w` lanes (4, 8, 16
/// or 32): a module's text, its entry
/// `pub unsafe fn {name}_w{w}(argv: *const *const u8, l: &Launch)`.
pub fn kernel(prog: &crate::lower::Program, k: &crate::vector::Kernel, w: u32) -> R<String> {
    emit(prog, k, w, false, &[])
}

/// The Rust of kernel `k` for groups of one SIMD group (`UKL_SIMD_WIDTH`, 32 threads), each group one chunk of 32
/// lanes: its barriers, anywhere (in a loop, a branch every thread takes, a called function), are met in step, and
/// its SIMD groups' operations are the chunk's. A launch of other groups is refused where it runs. The functions
/// `reads` names are calls of their compiled code (`Read`).
pub fn kernel_group(prog: &crate::lower::Program, k: &crate::vector::Kernel, reads: &[Read]) -> R<String> {
    emit(prog, k, 32, true, reads)
}

fn emit(prog: &crate::lower::Program, k: &crate::vector::Kernel, w: u32, one: bool, reads: &[Read]) -> R<String> {
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
    let mut e = Em { data: prog.data(), called: prog.funcs.iter().map(|f| f.called).collect(), names: prog.funcs.iter().map(|f| f.name.clone()).collect(), v, w, p: w / 4, bodies, lanes: HashMap::new(), iotas: Default::default(), refs: HashMap::new(), structs: HashMap::new(), mems: HashMap::new(), runs: HashMap::new(), lane_conds: Default::default(), shadows: Default::default(), shadow_fail: false, iota_params: HashMap::new(), wants: HashMap::new(), params: Default::default(), seen: Default::default(), insts: BTreeMap::new(), rets: HashMap::new(), top: 0, hoisted: HashMap::new(), memo: Vec::new(), valid: Vec::new(), writes: Vec::new(), disjoint: Default::default(), written: Some(Default::default()), aliases: Default::default(), arrays: HashMap::new(), risky: false, risky_insts: Default::default(), again: true, out: String::new(), ind: 0, n: 0, vars: BTreeMap::new(), shared: None, sub: None, pending: None, one, compiled: HashMap::new(), unaliased: HashSet::new() };
    for r in reads {
        if let Some(f) = prog.funcs.iter().position(|f| f.name == r.name) {
            e.compiled.insert(f as u32, r.clone());
        }
    }
    if !e.v.is_empty() && e.v.kind(0) == Kind::Func && e.v.w(0, 2) > 0 {
        e.shared = Some(e.v.w(0, 4));
    }
    // Each pass finds lanes the one before did not (a variable written a lane's value, or apart); until none. A
    // pass finds at least a function's instances a call deeper than the last's (format 16's parent read, through
    // its slot, its array's value and column, the window load: 17 passes).
    let mut refused = None;
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
        // Where the last pass stopped, if a refusal ended it: nothing of it goes on.
        e.ind = 0;
        e.memo.clear();
        e.valid.clear();
        e.sub = None;
        // A pass that learned something (`again`) and then refused a statement: the refusal may be of what it learned
        // too late in it to apply (a structure found to need its fields after a statement read it whole). The next
        // pass applies it; a refusal that stays is the last pass's.
        match e.module(&k.name) {
            Err(x) if e.again => {
                refused = Some(x);
                continue;
            }
            r => r?,
        }
        refused = None;
        // The buffers only their own threads write: none written where the writes' buffer is not known.
        let mut d: std::collections::BTreeSet<u32> = e.writes.iter().filter_map(|w| w.0).collect();
        if e.writes.iter().any(|w| w.0.is_none()) {
            d.clear();
        }
        let written = (!e.writes.iter().any(|w| w.0.is_none())).then(|| e.writes.iter().filter_map(|w| w.0).collect());
        if written != e.written {
            e.written = written;
            e.again = true;
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
    if let Some(x) = refused {
        return Err(x);
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
        if self.rolled() {
            let r = roll(s.as_ref(), self.p as usize);
            return self.line_at(&r);
        }
        self.line_at(s.as_ref())
    }

    fn line_at(&mut self, s: impl AsRef<str>) {
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
        // Of more than two registers, the same expression of each: written once, over the registers (`from_fn`,
        // which the compiler unrolls), so a chunk of 32 lanes is not eight copies of each operation's text.
        if self.p > 2
            && let Some(e) = per_register(&parts, 0)
        {
            self.line(format!("let {t}: [uint32x4_t; {}] = core::array::from_fn(|r| {e});", self.p));
            return t;
        }
        self.line(format!("let {t}: [uint32x4_t; {}] = [{}];", self.p, parts.join(", ")));
        t
    }

    /// Each lane's `m` (1, 2, 4 or 8 bytes) loaded from the address `at` of lane `j` (an expression of `j`, a pointer)
    /// into `t` (`[uint32x4_t; P]`, or `2P` its halves, declared): a loop over the lanes (`rolled`).
    fn load_lanes(&mut self, t: &str, m: M, at: &str) -> R<()> {
        let (w, p) = (self.w, self.p);
        let n = if wide(m) { 2 * p } else { p };
        let g = self.tmp();
        self.line(format!("let mut {g} = [0u32; {}];", 4 * n));
        let (ty, ext) = match (m.bytes(), m.signed()) {
            (8, _) => ("u64", ""),
            (1, true) => ("i8", " as i32"),
            (2, true) => ("i16", " as i32"),
            (1, false) => ("u8", ""),
            (2, false) => ("u16", ""),
            _ => ("u32", ""),
        };
        let put = if wide(m) { format!("{g}[j] = v as u32; {g}[{w} + j] = (v >> 32) as u32;") } else { format!("{g}[j] = v{ext} as u32;") };
        self.line(format!("for j in 0..{w} {{ let v = ({at} as *const {ty}).read_unaligned(); {put} }}"));
        self.line(format!("{t} = core::mem::transmute::<[u32; {}], [uint32x4_t; {n}]>({g});", 4 * n));
        Ok(())
    }

    /// Whether a lane's or a register's operations are written as loops over them (more than two registers, 8
    /// lanes), not each one's text.
    fn rolled(&self) -> bool {
        self.p > 2
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
        format!("[vdupq_n_u32(0); {}]", if wide(m) { 2 * self.p } else { self.p })
    }

    /// `a` where mask `k`'s lanes are on, else `b`: the registers of each lane's `m`, as an array (a 64-bit
    /// value's halves each by the mask's register of their lanes).
    fn blend(&self, m: M, k: &str, a: &str, b: &str) -> String {
        let n = if wide(m) { 2 * self.p } else { self.p };
        let parts: Vec<String> = (0..n).map(|r| format!("vbslq_u32({k}[{}], {a}[{r}], {b}[{r}])", r % self.p)).collect();
        format!("[{}]", parts.join(", "))
    }

    /// Each lane's 64-bit value `x` (of `m`) as its halves: each register's expression of the low halves, and of
    /// the high ones unless they are 0.
    fn halves(&mut self, x: &V, m: M) -> R<H> {
        let p = self.p;
        let regs = |l: &str, from: u32| (0..p).map(|r| format!("{l}[{}]", from + r)).collect::<Vec<_>>();
        Ok(match x {
            V::L(l, lm) if wide(*lm) => H { lo: regs(l, 0), hi: Some(regs(l, p)) },
            V::Z(l) => H { lo: regs(l, 0), hi: None },
            V::L(l, lm) if lm.signed() => {
                let l = l.clone();
                let s = self.map(|r| format!("vreinterpretq_u32_s32(vshrq_n_s32::<31>(vreinterpretq_s32_u32({l}[{r}])))"));
                H { lo: regs(&l, 0), hi: Some(regs(&s, 0)) }
            }
            V::S(s, sm) => {
                let v = if *sm == M::B { format!("if {s} {{ 1u64 }} else {{ 0 }}") } else { conv(s, *sm, M::U64) };
                let c = konst64(&v);
                let t = self.tmp();
                self.line(format!("let {t}: u64 = {v};"));
                let lo = self.lanes(&V::S(format!("({t}) as u32"), M::U32), M::U32)?;
                if c.is_some_and(|c| c >> 32 == 0) {
                    H { lo: regs(&lo, 0), hi: None }
                } else {
                    let hi = self.lanes(&V::S(format!("(({t}) >> 32) as u32"), M::U32), M::U32)?;
                    H { lo: regs(&lo, 0), hi: Some(regs(&hi, 0)) }
                }
            }
            // A lane's 32-bit index widened and scaled: the high half the bits the scale shifts out.
            V::At { base, idx, scale } if base.is_empty() => {
                let l = self.lanes(idx, M::U32)?;
                match *scale {
                    1 => H { lo: regs(&l, 0), hi: None },
                    k if k.is_power_of_two() => {
                        let s = k.trailing_zeros();
                        H { lo: (0..p).map(|r| format!("vshlq_n_u32::<{s}>({l}[{r}])")).collect(), hi: Some((0..p).map(|r| format!("vshrq_n_u32::<{}>({l}[{r}])", 32 - s)).collect()) }
                    }
                    k => {
                        let (lo, hi) = (self.map(|r| format!("vmulq_u32({l}[{r}], vdupq_n_u32({k}))")), self.mulhi32(&regs(&l, 0), &vec![format!("vdupq_n_u32({k})"); p as usize]));
                        H { lo: regs(&lo, 0), hi: Some(regs(&hi, 0)) }
                    }
                }
            }
            // Of a base: the offset's, plus the base.
            V::At { base, idx, scale } => {
                let o = self.halves(&V::At { base: String::new(), idx: idx.clone(), scale: *scale }, M::U64)?;
                let o = self.of_halves(o, M::U64);
                let s = self.bin64(Op::Add, M::U64, &V::S(format!("(({base}) as usize as u64)"), M::U64), &o)?;
                return self.halves(&s, m);
            }
            x => {
                let l = self.lanes(x, M::U32)?;
                H { lo: regs(&l, 0), hi: None }
            }
        })
        .map(|h| if m.signed() || wide(m) { h } else { H { hi: None, ..h } })
    }

    /// Halves `h` as a value of each lane's 64-bit `m`: `Z` where the high ones are 0.
    fn of_halves(&mut self, h: H, m: M) -> V {
        let t = self.tmp();
        match h.hi {
            None => {
                self.line(format!("let {t}: [uint32x4_t; {}] = [{}];", self.p, h.lo.join(", ")));
                V::Z(t)
            }
            Some(hi) => {
                self.line(format!("let {t}: [uint32x4_t; {}] = [{}, {}];", 2 * self.p, h.lo.join(", "), hi.join(", ")));
                V::L(t, m)
            }
        }
    }

    /// The high 32 bits of each lane's product of 32-bit `a` and `b` (each register's expression).
    fn mulhi32(&mut self, a: &[String], b: &[String]) -> String {
        self.map(|r| {
            let (a, b) = (&a[r as usize], &b[r as usize]);
            format!("vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32({a}), vget_low_u32({b}))), vreinterpretq_u32_u64(vmull_high_u32({a}, {b})))")
        })
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
        if (0..self.v.len()).any(|i| self.v.kind(i) == Kind::Global) {
            self.line("");
            self.line("/// The program's constant memory, which its `__constant` variables are.");
            let bytes: Vec<String> = self.data.iter().map(|b| b.to_string()).collect();
            self.line(format!("static DATA: [u8; {}] = [{}];", self.data.len(), bytes.join(", ")));
        }
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
        self.line("/// Mask `k`'s lanes as bits, lane `j` bit `j`.");
        self.line("#[inline(always)]");
        self.line(format!("unsafe fn bits(k: &[uint32x4_t; {p}]) -> u32 {{"));
        self.line(format!("    let w = vld1q_u32([1u32, 2, 4, 8].as_ptr());\n    let mut b = 0u32;\n    for r in 0..{p} {{\n        b |= vaddvq_u32(vandq_u32(k[r], w)) << (4 * r);\n    }}\n    b"));
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
        if self.one {
            self.line(format!("assert!(n == {w}, \"{name}: a group of one SIMD group ({w} threads)\");"));
        }
        self.top = self.out.len();
        self.hoisted.clear();
        self.runs.clear();
        self.iota_params.clear();
        let cx = Cx { cur: "live".into(), divergent: false, live: "live".into(), res: None, brk: None, exit: None, cont: None, kernel: true, ctx: u32::MAX, body: u32::MAX, full: false, looped: None, region: None, chunk: false, phased: false, left: false };
        self.stmt(0, &cx)?;
        self.ind -= 1;
        self.line("}");
        Ok(())
    }

    /// The kernel's statement `n`, which its lanes run: for each chunk of `W` of the group's threads, all of whose
    /// lanes run, then the group's last threads (under `n`), if any, as some lanes of a chunk.
    fn chunks(&mut self, n: usize, cx: &Cx) -> R<()> {
        let w = self.w;
        // A group of one chunk, all of whose lanes run.
        if self.one {
            self.line("{");
            self.ind += 1;
            self.line("let lid0 = 0u32;");
            self.line(format!("let mut live = {};", self.ones()));
            self.line(format!("let {GID} = l.group[0] * l.block[0];"));
            self.line("'chunk: {");
            self.ind += 1;
            self.stmt(n, &Cx { full: true, chunk: true, ..cx.clone() })?;
            self.ind -= 1;
            self.line("}");
            self.ind -= 1;
            self.line("}");
            return Ok(());
        }
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

    /// Function `id`'s statements in phases, if barriers part them: its top-level statements between them
    /// (`phase_parts`).
    fn phases(&self, id: u32) -> Option<Vec<Vec<Part>>> {
        // A group of one chunk: its barriers met where they are, no phases.
        if self.one {
            return None;
        }
        let &b = self.bodies.get(&id)?;
        let block = self.v.child(b, 0);
        if self.v.kind(block) != Kind::Block {
            return None;
        }
        let mut out = vec![Vec::new()];
        for c in self.v.children(block) {
            self.phase_parts(c, &mut out);
        }
        // A function that is only a barrier (`ukl_tg_barrier`) is one, not phases: a call of it elsewhere is refused.
        (out.len() > 1 && out.iter().any(|p| !p.is_empty())).then_some(out)
    }

    /// Top-level statement `c`'s parts, onto the phases `out`: a barrier ends a phase. A call of a function with
    /// phases (its arguments without effects, evaluated again in each) spans that function's: its first phase in
    /// the statement's phase, each next one a phase of its own. A loop of a constant count whose body's statements
    /// a barrier parts (`unrolled`) is its rounds one after another: each its counter set, then its body's parts.
    fn phase_parts(&self, c: usize, out: &mut Vec<Vec<Part>>) {
        if self.is_barrier(c) {
            out.push(Vec::new());
        } else if let Some((r, values, body)) = self.unrolled(c) {
            for x in values {
                out.last_mut().unwrap().push(Part::Set(r, x));
                for d in self.v.children(body) {
                    self.phase_parts(d, out);
                }
            }
        } else if let Some((_, np)) = self.phased_call(c) {
            out.last_mut().unwrap().push(Part::Sub(c, 0));
            for j in 1..np {
                out.push(vec![Part::Sub(c, j as u32)]);
            }
        } else {
            out.last_mut().unwrap().push(Part::S(c));
        }
    }

    /// Statement `c` as a loop of a constant count of rounds whose body a barrier parts: `{ r = a; for (; r < b;
    /// r++) body }`, its counter `r` written by nothing else and the body left by nothing but its end (at most 16
    /// rounds). Its counter's node, the counter's value in each round, and the body's block.
    fn unrolled(&self, c: usize) -> Option<(usize, Vec<u32>, usize)> {
        let v = &self.v;
        if v.kind(c) != Kind::Block || v.children(c).count() != 2 {
            return None;
        }
        let (init, f) = (v.child(c, 0), v.child(c, 1));
        let init = if v.kind(init) == Kind::Block && v.children(init).count() == 1 { v.child(init, 0) } else { init };
        if v.kind(init) != Kind::Expr || v.kind(f) != Kind::For || v.children(f).count() != 3 {
            return None;
        }
        let konst = |i: usize| (v.kind(i) == Kind::Const && v.w(i, 2) == 0).then(|| v.w(i, 1));
        let local = |i: usize| (v.kind(i) == Kind::Local && v.w(i, 2) == 0).then(|| v.w(i, 3));
        // `r = a`.
        let st = v.child(init, 0);
        if v.kind(st) != Kind::Store {
            return None;
        }
        let r = v.child(st, 0);
        let (var, a) = (local(r)?, konst(v.child(st, 1))?);
        // `r < b`.
        let (cond, body, step) = (v.child(f, 0), v.child(f, 1), v.child(f, 2));
        let cmp = if v.kind(cond) == Kind::Cast { v.child(cond, 0) } else { cond };
        if v.kind(cmp) != Kind::Bin || v.w(cmp, 0) != Op::Lt as u32 || M(v.w(cmp, 1)).signed() {
            return None;
        }
        let ld = v.child(cmp, 0);
        if v.kind(ld) != Kind::Load || local(v.child(ld, 0)) != Some(var) {
            return None;
        }
        let b = konst(v.child(cmp, 1))?;
        // `r++`.
        let up = if v.kind(step) == Kind::Expr { v.child(step, 0) } else { step };
        if v.kind(up) != Kind::Update || v.w(up, 0) != Op::Add as u32 || local(v.child(up, 0)) != Some(var) || konst(v.child(up, 1)) != Some(1) {
            return None;
        }
        // The body: a block that holds a barrier at its top, reads the counter only, and is left only at its end.
        if v.kind(body) != Kind::Block || !v.children(body).any(|d| self.is_barrier(d)) || b <= a || b - a > 16 {
            return None;
        }
        let end = body + v.size(body);
        // A `break` or `continue` of an inner loop (or a `break` of a switch) stays in the round.
        let inner = |i: usize, brk: bool| (body + 1..i).any(|q| (matches!(v.kind(q), Kind::While | Kind::For | Kind::Do) || (brk && v.kind(q) == Kind::Switch)) && i < q + v.size(q));
        if (body..end).any(|i| match v.kind(i) {
            Kind::Return => true,
            Kind::Break => !inner(i, true),
            Kind::Continue => !inner(i, false),
            _ => false,
        }) {
            return None;
        }
        if (body..end).any(|i| matches!(v.kind(i), Kind::Store | Kind::Update | Kind::Copy) && {
            let t = v.child(i, 0);
            (t..t + v.size(t)).any(|j| local(j) == Some(var))
        }) {
            return None;
        }
        Some((r, (a..b).collect(), body))
    }

    /// A slot of a phase's saved value: the bytes of the widest lanes, `[uint32x4_t; 2P]`.
    fn slot(&self) -> usize {
        32 * self.p as usize
    }

    /// Variable node `local`'s number in its function (its parameters first, then its declarations in order):
    /// what names a variable, not its offset in the frame, which variables of scopes apart share (a loop's counter
    /// and a later declaration's), each its own Rust variable, of one value or each lane's apart.
    fn vid(&self, local: usize) -> u32 {
        self.v.w(local, 3) & crate::vector::VAR_IN_FUNCTION
    }

    /// The variables function `id` keeps (its locals but its parameters), by number (`vid`).
    fn kept(&self, id: u32) -> Vec<u32> {
        let b = self.bodies[&id];
        let np = self.v.w(b, 6) as usize;
        let mut offs: Vec<u32> = (b + 1..b + self.v.size(b)).filter(|&i| self.v.kind(i) == Kind::Local).map(|i| self.vid(i)).filter(|&o| o as usize >= np).collect();
        offs.sort();
        offs.dedup();
        offs
    }

    /// A chunk's saved state of function `id`'s phases: its lanes that go on, then each variable kept.
    fn state_bytes(&self, id: u32) -> usize {
        let own = (1 + self.kept(id).len()) * self.slot();
        // A called function's with phases after its own (`Part::Sub`).
        let sub = self.phases(id).into_iter().flatten().flatten().filter_map(|p| match p {
            Part::Sub(s, _) => self.phased_call(s).map(|(g, _)| self.state_bytes(g)),
            Part::S(_) | Part::Set(..) => None,
        });
        own + sub.max().unwrap_or(0)
    }

    /// Statement `s` as a call of a function with phases (its number and its phases' count), its arguments
    /// without effects.
    fn phased_call(&self, s: usize) -> Option<(u32, usize)> {
        if self.v.kind(s) != Kind::Expr {
            return None;
        }
        let e = self.v.child(s, 0);
        if self.v.kind(e) != Kind::Call || self.v.children(e).any(|a| self.effects(a)) {
            return None;
        }
        let g = self.v.w(e, 3);
        self.phases(g).map(|p| (g, p.len()))
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
        let params: Vec<(u32, M)> = (0..np).map(|k| (k as u32, M(v.w(n, 8 + 2 * k)))).collect();
        let mut ps = vec!["l: &Launch".to_string(), "lid0: u32".to_string()];
        if !full {
            ps.push(format!("k: [uint32x4_t; {}]", self.p));
        }
        self.runs.clear();
        self.iota_params.clear();
        for (k, &(off, m)) in params.iter().enumerate() {
            let name = format!("x{off}");
            // A parameter the function writes: a mutable binding.
            let mt = if self.writes_var(n, off) { "mut " } else { "" };
            let var = if m.is_agg() && !matches!(shapes.get(k), Some(Sh::At(..))) {
                ps.push(format!("{name}: *const u8"));
                Var { name, m: M::PTR, lanes: false, iota: false, fixed: None }
            } else {
                match shapes.get(k).copied().unwrap_or(Sh::S) {
                    Sh::Arg(r) => {
                        let name = format!("arg{r}");
                        ps.push(format!("{name}: *const u8"));
                        Var { name, m, lanes: false, iota: false, fixed: None }
                    }
                    Sh::Gid => Var { name, m, lanes: false, iota: false, fixed: Some(V::Iota(GID.into(), m)) },
                    Sh::Ref(r) => {
                        ps.push(format!("{name}: &mut {}", self.ty(M(r), true)?));
                        Var { name: name.clone(), m: M::PTR, lanes: false, iota: false, fixed: Some(V::Ref(format!("(*{name})"), M(r))) }
                    }
                    Sh::Arr(e, b) => {
                        let regs = if b == 8 { 2 * self.p } else { self.p };
                        ps.push(format!("{name}: &mut [[uint32x4_t; {regs}]; {e}]"));
                        Var { name: name.clone(), m: M::PTR, lanes: false, iota: false, fixed: Some(V::Arr(format!("(*{name})"), e, b)) }
                    }
                    Sh::In(r) => {
                        let name = format!("arg{r}_{name}");
                        ps.push(format!("{name}: {}", self.ty(m, false)?));
                        Var { name, m, lanes: false, iota: false, fixed: None }
                    }
                    Sh::S => {
                        ps.push(format!("{mt}{name}: {}", self.ty(m, false)?));
                        Var { name, m, lanes: false, iota: false, fixed: None }
                    }
                    Sh::L => {
                        ps.push(format!("{mt}{name}: {}", self.ty(m, true)?));
                        self.lanes.insert((ctx, off), true);
                        Var { name, m, lanes: true, iota: false, fixed: None }
                    }
                    Sh::Iota | Sh::Run(_) => {
                        ps.push(format!("{name}: u32"));
                        if let Sh::Run(b) = shapes[k] {
                            self.runs.insert(name.clone(), b);
                        } else {
                            self.iota_params.insert(name.clone(), (id, k));
                        }
                        Var { name: name.clone(), m, lanes: false, iota: false, fixed: Some(V::Iota(name, m)) }
                    }
                    Sh::Lin => {
                        ps.push(format!("{name}f: u32"));
                        ps.push(format!("{name}s: u32"));
                        Var { name: name.clone(), m, lanes: false, iota: false, fixed: Some(V::Lin(format!("{name}f"), format!("{name}s"))) }
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
                            Ix::Lin => {
                                ps.push(format!("{i}f: u32"));
                                ps.push(format!("{i}s: u32"));
                                V::Lin(format!("{i}f"), format!("{i}s"))
                            }
                            Ix::L => {
                                ps.push(format!("{i}: {}", self.ty(M::U32, true)?));
                                V::L(i, M::U32)
                            }
                        };
                        Var { name, m, lanes: false, iota: false, fixed: Some(V::At { base: b, idx: Box::new(idx), scale }) }
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
        let rt = if ret == M::VOID { String::new() } else { format!(" -> {}", self.result_ty(ret, rl)?) };
        let called = self.called.get(self.v.w(n, 1) as usize).copied().unwrap_or(false);
        if let Some(name) = self.names.get(self.v.w(n, 1) as usize) {
            self.line(format!("/// `{name}`."));
        }
        let attr = self.out.len();
        self.line("#[inline(always)]");
        let body = self.out.len();
        self.line(format!("unsafe fn {}({}){rt} {{", fname(id, shapes, full, valid), ps.join(", ")));
        self.ind += 1;
        self.line(format!("let {GID} = l.group[0] * l.block[0] + lid0;"));
        self.top = self.out.len();
        self.hoisted.clear();
        self.valid = valid.iter().map(|&k| (k, 1)).collect();
        // Its variables, each declared once at its start (a scope's are its function's in C's sense here).
        let locals = self.locals(n, ctx, &params)?;
        // A parameter of a shape its calls give (consecutive lanes, each lane's address) that the function writes, or
        // one of one value it writes each lane's value to (as a pass found): a variable of each lane's, from it.
        for &(off, _) in &params {
            let Some(var) = self.vars.get(&(ctx, off)).cloned() else { continue };
            let f = match var.fixed.clone() {
                Some(V::Ref(..) | V::Arr(..)) => continue,
                Some(f) => f,
                None if !var.lanes && !var.m.is_agg() && self.lanes.get(&(ctx, off)).copied().unwrap_or(false) => V::S(var.name.clone(), var.m),
                None => continue,
            };
            if !self.writes_var(n, off) {
                continue;
            }
            let name = format!("{}_w", var.name);
            let l = if var.m == M::B { self.mask(&f)? } else { self.lanes(&f, var.m)? };
            self.line(format!("let mut {name}: {} = {l};", self.ty(var.m, true)?));
            self.vars.insert((ctx, off), Var { name, m: var.m, lanes: true, iota: false, fixed: None });
        }
        if phases.is_some() && self.arrays.keys().any(|k| k.0 == ctx) {
            return err("a thread's array in a function with barriers");
        }
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
            let init = if ret.is_agg() {
                if rl { format!("[[0u8; {}]; {}]", ret.bytes(), self.w) } else { format!("[0u8; {}]", ret.bytes()) }
            } else if rl {
                self.zeros_of(ret)
            } else if ret == M::B {
                "false".into()
            } else if ret == M::PTR {
                "core::ptr::null()".into()
            } else {
                "0".into()
            };
            self.line(format!("let mut res: {} = {init};", self.result_ty(ret, rl)?));
            Some(("res".to_string(), ret, rl))
        };
        let cx = Cx { cur: "live".into(), divergent: false, live: "live".into(), res: res.clone(), brk: None, exit: None, cont: None, kernel: false, ctx, body: id, full, looped: None, region: None, chunk: false, phased: false, left: false };
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
                let pcx = Cx { phased: true, full: full && j == 0, ..cx.clone() };
                // Its own statements in runs; a called function's phase `k`, its state after this one's.
                let mut run: Vec<usize> = Vec::new();
                for part in stmts.iter().copied().chain([Part::Sub(usize::MAX, 0)]) {
                    match part {
                        Part::S(s) => run.push(s),
                        Part::Set(r, x) => {
                            if !run.is_empty() {
                                self.seq(&std::mem::take(&mut run), &pcx, None)?;
                            }
                            let Some(var) = self.vars.get(&(ctx, self.vid(r))).cloned() else { return err("an unrolled loop's counter undeclared") };
                            let x = literal(var.m, u64::from(x))?;
                            if var.lanes {
                                self.line(format!("{} = [vdupq_n_u32({x}); {}];", var.name, self.p));
                            } else {
                                self.line(format!("{} = {x};", var.name));
                            }
                        }
                        Part::Sub(s, k) => {
                            if !run.is_empty() {
                                self.seq(&std::mem::take(&mut run), &pcx, None)?;
                            }
                            if s != usize::MAX {
                                self.sub = Some((k, (1 + self.kept(id).len()) * slot));
                                self.seq(&[s], &pcx, None)?;
                                self.sub = None;
                            }
                        }
                    }
                }
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
        if called && self.out[body..].matches('\n').count() > CALLED_LINES {
            self.out.replace_range(attr..body, &format!("{}#[inline]\n", "    ".repeat(self.ind)));
        }
        Ok(())
    }

    /// A call of read `r` (its body `b`), compiled code of its own, through the launch's table: one call where its
    /// arguments are each one value (as a function's instance of one value's arguments runs once), else one for each
    /// running lane, its results gathered into the lanes.
    fn call_compiled(&mut self, r: &Read, b: usize, vals: Vec<V>, cx: &Cx) -> R<V> {
        let ret = M(self.v.w(b, 4));
        let ms: Vec<M> = (0..vals.len()).map(|k| M(self.v.w(b, 8 + 2 * k))).collect();
        if vals.len() < 2 || r.offsets.len() != vals.len() - 2 || ms.iter().chain([&ret]).any(|m| m.is_agg() || m.float()) {
            return err(format!("{}: not a read's arguments and result", r.name));
        }
        let ty = |m: M| if m.bytes() == 8 { "u64" } else { "u32" };
        let (w, p) = (self.w, self.p);
        // Each argument as an expression of lane `j`, or its one value.
        let mut each = Vec::new();
        for (k, v) in vals.iter().enumerate() {
            let m = ms[k];
            let t = self.tmp();
            match v {
                V::S(s, am) => {
                    let x = if *am == M::PTR { format!("(({s}) as usize as u64)") } else { conv(s, *am, if m == M::PTR { M::U64 } else { m }) };
                    self.line(format!("let {t}: {} = {x} as {};", ty(m), ty(m)));
                    each.push(t);
                }
                v => {
                    let l = self.lanes(v, m)?;
                    let n = if wide(m) || m == M::PTR { 2 * p } else { p };
                    self.line(format!("let {t} = core::mem::transmute::<[uint32x4_t; {n}], [u32; {}]>({l});", 4 * n));
                    each.push(if n == 2 * p { format!("(({t}[{w} + j] as u64) << 32 | {t}[j] as u64)") } else { format!("{t}[j]") });
                }
            }
        }
        let base = each[0].clone();
        let mut args = vec![format!("{base}.wrapping_add({})", each[1])];
        for (k, a) in each[2..].iter().enumerate() {
            args.push(if r.offsets[k] { format!("{base}.wrapping_add({a})") } else { a.clone() });
        }
        let f = self.tmp();
        let sig = std::iter::once("u64").chain(ms[2..].iter().map(|&m| ty(m))).collect::<Vec<_>>().join(", ");
        self.line(format!("let {f}: unsafe extern \"C\" fn({sig}) -> {} = core::mem::transmute(*l.reads.add({}));", ty(ret), r.index));
        let call = format!("{f}({})", args.join(", "));
        let call = if r.result { format!("{call}.wrapping_sub({base})") } else { call };
        let t = self.tmp();
        if vals.iter().all(|v| matches!(v, V::S(..))) {
            self.line(format!("let {t}: {} = {call};", ty(ret)));
            return Ok(V::S(t, ret));
        }
        // Each running lane's call, its result in its lane.
        let n = if wide(ret) { 2 * p } else { p };
        let g = self.tmp();
        self.line(format!("let mut {g} = [0u32; {}];", 4 * n));
        let on = if cx.full {
            String::new()
        } else {
            let k = self.tmp();
            let cur = cx.cur.clone();
            self.line(format!("let {k} = bits(&{cur});"));
            format!("if {k} >> j & 1 != 0 ")
        };
        let put = if n == 2 * p { format!("{g}[j] = v as u32; {g}[{w} + j] = (v >> 32) as u32;") } else { format!("{g}[j] = v;") };
        self.line(format!("for j in 0..{w} {{ {on}{{ let v = {call}; {put} }} }}"));
        self.line(format!("let {t} = core::mem::transmute::<[u32; {}], [uint32x4_t; {n}]>({g});", 4 * n));
        Ok(V::L(t, ret))
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
        let off = k as u32;
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
        let mut arrays = std::collections::BTreeSet::new();
        let end = n + self.v.size(n);
        for i in n + 1..end {
            if self.v.kind(i) != Kind::Local {
                continue;
            }
            let off = self.vid(i);
            if params.iter().any(|&(o, _)| o == off) || out.contains_key(&off) || arrays.contains(&off) {
                continue;
            }
            // A thread's structure: each field its own variable, as a scalar is.
            if let Some(fields) = self.fields(n, off, !self.unaliased.contains(&(ctx, off))) {
                for &(byte, m) in &fields {
                    let key = field_key(off, byte);
                    let lanes = self.lanes.get(&(ctx, key)).copied().unwrap_or(false);
                    let iota = !lanes && m.bytes() == 4 && self.iotas.contains(&(ctx, key));
                    out.insert(key, Var { name: format!("x{off}_{byte}"), m, lanes, iota, fixed: self.refs.get(&(ctx, key)).cloned() });
                }
                self.structs.insert((ctx, off), fields);
                arrays.insert(off);
                continue;
            }
            // A thread's array: each element the chunk's lanes' registers (of 8 bytes, its halves').
            if let Some((elems, bytes)) = self.array(n, off) {
                let name = format!("x{off}");
                let (regs, zero) = if bytes == 8 { (2 * self.p, self.zeros_of(M::U64)) } else { (self.p, self.zeros_of(M::U32)) };
                self.line(format!("let mut {name}: [[uint32x4_t; {regs}]; {elems}] = [{zero}; {elems}];"));
                self.arrays.insert((ctx, off), (name, elems, bytes));
                arrays.insert(off);
                continue;
            }
            // A thread's structure of other uses (copied into more than once, its address passed): its bytes, a row
            // a lane, read and written as memory is.
            if self.v.w(i, 1) > 8 && !self.aliasable(n, off) {
                let (name, size) = (format!("x{off}"), self.v.w(i, 1));
                self.line(format!("let mut {name} = [[0u8; {size}]; {}];", self.w));
                self.mems.insert((ctx, off), (name, size));
                arrays.insert(off);
                continue;
            }
            // Its type: an access's (a load's, a store's, an update's) of its size, else its first access's.
            let size = self.v.w(i, 1);
            let m = (i..end)
                .filter(|&j| self.v.kind(j) == Kind::Local && self.vid(j) == off && self.v.w(j, 2) == 0)
                .filter_map(|j| self.access_type(j))
                .find(|m| m.bytes() == size && !m.is_agg())
                .or_else(|| self.access_type(i))
                .unwrap_or(M::U32);
            let lanes = self.lanes.get(&(ctx, off)).copied().unwrap_or(false);
            let iota = !lanes && m.bytes() == 4 && self.iotas.contains(&(ctx, off));
            out.insert(off, Var { name: format!("x{off}"), m, lanes, iota, fixed: self.refs.get(&(ctx, off)).cloned() });
        }
        for (off, var) in &out {
            self.vars.insert((ctx, *off), var.clone());
        }
        Ok(out.into_values().collect())
    }

    /// If variable `off` of function `f` is a thread's structure of fields (more than one): each field's byte and
    /// type, in order. Each access of it is a scalar's load, store or update at it plus a constant (a field), or a
    /// copy of it whole (from bytes or into them), or a read of it whole (an aggregate argument): its address is
    /// not otherwise passed or kept. Its fields do not overlap. Where it is read whole (copied from, passed, returned,
    /// loaded) every byte is a field's: the bytes no access names are fields too (`gaps`), which a copy into it
    /// fills and a read of it whole writes.
    fn fields(&self, f: usize, off: u32, alias: bool) -> Option<Vec<(u32, M)>> {
        let mut size = 0;
        let mut whole = false;
        let mut fields: Vec<(u32, M)> = Vec::new();
        for i in f + 1..f + self.v.size(f) {
            if self.v.kind(i) != Kind::Local || self.vid(i) != off {
                continue;
            }
            size = self.v.w(i, 1);
            // Its address: it, plus constants (a field of a field).
            let (mut at, mut byte) = (i, self.v.w(i, 2));
            let mut r = self.parent(i)?;
            while let Some((l, b)) = self.local_field(r)
                && l == i
            {
                (at, byte) = (r, b);
                r = self.parent(r)?;
            }
            let m = match self.v.kind(r) {
                Kind::Load | Kind::Store => M(self.v.w(r, 0)),
                Kind::Update => M(self.v.w(r, 1)),
                // Whole: copied into, or from (read whole).
                Kind::Copy if at == i && byte == 0 => {
                    whole |= self.v.child(r, 1) == i;
                    continue;
                }
                // Whole: an aggregate argument (passed as its bytes' address), or the function's result.
                Kind::Call | Kind::Return if at == i && byte == 0 && (self.v.kind(r) == Kind::Return || self.agg_arg(r, i)) => {
                    whole = true;
                    continue;
                }
                _ => return None,
            };
            if self.v.child(r, 0) != at {
                return None;
            }
            if m.is_agg() {
                // Read whole (an aggregate argument).
                if at == i && byte == 0 && self.v.kind(r) == Kind::Load {
                    whole = true;
                    continue;
                }
                return None;
            }
            if !fields.contains(&(byte, m)) {
                fields.push((byte, m));
            }
        }
        if fields.is_empty() || (fields.len() == 1 && fields[0].1.bytes() == size) {
            return None;
        }
        fields.sort_by_key(|f| f.0);
        let fits = fields.windows(2).all(|w| w[0].0 + w[0].1.bytes() <= w[1].0) && fields.last().is_some_and(|l| l.0 + l.1.bytes() <= size);
        if !fits {
            return None;
        }
        if whole {
            // Copied into from memory and only read: read where it is (`aliasable`), not each byte loaded; unless
            // its copy found it cannot be (`unaliased`).
            if alias && self.aliasable(f, off) {
                return None;
            }
            let named = fields.clone();
            let mut at = 0;
            for (b, m) in named.iter().copied().chain([(size, M::VOID)]) {
                fields.extend(gaps(at, b));
                at = b + m.bytes();
            }
            fields.sort_by_key(|f| f.0);
        }
        Some(fields)
    }

    /// Whether node `a` is call `call`'s argument for an aggregate parameter.
    fn agg_arg(&self, call: usize, a: usize) -> bool {
        let Some(&b) = self.bodies.get(&self.v.w(call, 3)) else { return false };
        let Some(k) = self.v.children(call).position(|c| c == a) else { return false };
        M(self.v.w(b, 8 + 2 * k)).is_agg()
    }

    /// Whether variable `off` of function `f` is copied into whole, from memory or a call's result (not another
    /// variable), and only read after (`Kind::Copy`'s alias).
    fn aliasable(&self, f: usize, off: u32) -> bool {
        (f + 1..f + self.v.size(f)).any(|i| {
            self.v.kind(i) == Kind::Copy && {
                let (d, s) = (self.v.child(i, 0), self.v.child(i, 1));
                self.v.kind(d) == Kind::Local && self.vid(d) == off && self.v.w(d, 2) == 0 && self.v.kind(s) != Kind::Local && self.only_read(d, i)
            }
        })
    }

    /// Whether function `n` writes its variable `off` (a store or an update of it).
    fn writes_var(&self, n: usize, off: u32) -> bool {
        (n + 1..n + self.v.size(n)).any(|i| {
            matches!(self.v.kind(i), Kind::Store | Kind::Update) && {
                let a = self.v.child(i, 0);
                self.v.kind(a) == Kind::Local && self.v.w(a, 2) == 0 && self.vid(a) == off
            }
        })
    }

    /// If node `a` is a thread's structure whole (`fields`): its fields.
    fn struct_of(&self, a: usize, cx: &Cx) -> Option<Vec<(u32, M)>> {
        (self.v.kind(a) == Kind::Local && self.v.w(a, 2) == 0).then(|| self.structs.get(&(cx.ctx, self.vid(a))).cloned()).flatten()
    }

    /// Field `byte` of the thread's structure at node `a`: its value.
    fn field_value(&mut self, a: usize, byte: u32, cx: &Cx) -> R<V> {
        let Some(var) = self.vars.get(&(cx.ctx, field_key(self.vid(a), byte))).cloned() else { return err("a structure's field undeclared") };
        Ok(if var.lanes {
            if var.m == M::B { V::K(var.name) } else { V::L(var.name, var.m) }
        } else if var.iota {
            V::Iota(var.name, var.m)
        } else {
            V::S(var.name, var.m)
        })
    }

    /// A thread's structure at node `a`, read whole (an aggregate argument, passed as its bytes' address): its
    /// fields written to bytes of the thread's own, one value's or each lane's (then `V::At`, a lane's at its row).
    fn struct_bytes(&mut self, a: usize, cx: &Cx) -> R<V> {
        let fields = self.struct_of(a, cx).expect("a structure");
        let size = self.v.w(a, 1);
        let vals: Vec<(u32, M, V)> = fields.iter().map(|&(b, m)| Ok((b, m, self.field_value(a, b, cx)?))).collect::<R<_>>()?;
        let t = self.tmp();
        if vals.iter().all(|v| matches!(v.2, V::S(..))) {
            self.line(format!("let mut {t} = [0u8; {size}];"));
            for (b, m, v) in vals {
                let V::S(x, xm) = v else { unreachable!() };
                self.line(format!("({t}.as_mut_ptr().add({b}) as *mut {}).write_unaligned({});", self.ty(m, false)?, conv(&x, xm, m)));
            }
            return Ok(V::S(format!("{t}.as_ptr()"), M::PTR));
        }
        let w = self.w;
        self.line(format!("let mut {t} = [[0u8; {size}]; {w}];"));
        let mut each = Vec::new();
        for (b, m, v) in &vals {
            let V::S(x, xm) = self.lane_of(v)? else { return err("a field's value at a lane") };
            each.push(format!("({t}[j].as_mut_ptr().add({b}) as *mut {}).write_unaligned({});", self.ty(*m, false)?, conv(&x, xm, *m)));
        }
        self.line(format!("for j in 0..{w} {{ {} }}", each.join(" ")));
        Ok(V::At { base: format!("({t}.as_ptr() as *const u8)"), idx: Box::new(V::Iota("0u32".into(), M::U32)), scale: size })
    }

    /// Address `a` as a variable plus constants (`Local` plus `Const`s): the variable's node and the byte.
    fn local_field(&self, a: usize) -> Option<(usize, u32)> {
        if self.v.kind(a) != Kind::Bin || Op::of(self.v.w(a, 0)) != Op::Add {
            return None;
        }
        let (l, c) = (self.v.child(a, 0), self.v.child(a, 1));
        if self.v.kind(c) != Kind::Const || self.v.w(c, 2) != 0 {
            return None;
        }
        let (l, at) = match self.v.kind(l) {
            Kind::Local => (l, self.v.w(l, 2)),
            _ => self.local_field(l)?,
        };
        Some((l, at + self.v.w(c, 1)))
    }

    /// If variable `off` of function `f` is a thread's array: its elements and their bytes (4 or 8). It is more than
    /// 8 bytes, and each access of it is an integer's of that size, a load, store or update at it or at it plus an
    /// offset (`elem`), which nothing else reads (its address is not passed or kept).
    fn array(&self, f: usize, off: u32) -> Option<(u32, u32)> {
        let mut size = 0;
        let mut bytes = 0;
        for i in f + 1..f + self.v.size(f) {
            if self.v.kind(i) != Kind::Local || self.vid(i) != off {
                continue;
            }
            size = self.v.w(i, 1);
            let q = self.parent(i)?;
            let at = if self.v.kind(q) == Kind::Bin && Op::of(self.v.w(q, 0)) == Op::Add && self.v.child(q, 0) == i { q } else { i };
            let r = if at == q { self.parent(q)? } else { q };
            let m = match self.v.kind(r) {
                Kind::Load | Kind::Store => M(self.v.w(r, 0)),
                Kind::Update => M(self.v.w(r, 1)),
                // Passed whole to a function, as a pointer it reads its elements through (`V::Arr`).
                Kind::Call if at == i && self.v.w(i, 2) == 0 && !self.agg_arg(r, i) => continue,
                _ => return None,
            };
            if self.v.child(r, 0) != at || !matches!(m.bytes(), 4 | 8) || m.float() || m.is_agg() || m == M::PTR || (bytes != 0 && bytes != m.bytes()) {
                return None;
            }
            bytes = m.bytes();
        }
        (size > 8 && bytes != 0 && size % bytes == 0).then(|| (size / bytes, bytes))
    }

    /// If address `a` is an element of a thread's array (`array`): the element, as a Rust place.
    fn elem(&mut self, a: usize, cx: &Cx) -> R<Option<String>> {
        // The array (its variable, or a parameter it was passed as), and the index.
        let (base, ix) = match self.v.kind(a) {
            Kind::Local | Kind::Load => (a, None),
            Kind::Bin if Op::of(self.v.w(a, 0)) == Op::Add && matches!(self.v.kind(self.v.child(a, 0)), Kind::Local | Kind::Load) => (self.v.child(a, 0), Some(self.v.child(a, 1))),
            _ => return Ok(None),
        };
        let found = match self.v.kind(base) {
            Kind::Local => self.arrays.get(&(cx.ctx, self.vid(base))).cloned(),
            _ => {
                let l = self.v.child(base, 0);
                match (self.v.kind(l), M(self.v.w(base, 0))) {
                    (Kind::Local, M::PTR) if self.v.w(l, 2) == 0 => match self.vars.get(&(cx.ctx, self.vid(l))) {
                        Some(Var { fixed: Some(V::Arr(place, e, b)), .. }) => Some((place.clone(), *e, *b)),
                        _ => None,
                    },
                    _ => None,
                }
            }
        };
        let Some((name, elems, bytes)) = found else { return Ok(None) };
        let Some(ix) = ix else { return Ok(Some(format!("{name}[0]"))) };
        // Its byte offset: one value for all lanes.
        let s = match self.expr(ix, cx)? {
            V::S(s, _) => s,
            // One value in the lanes that run, kept as each lane's (a variable set where lanes are apart): an index
            // times the element's bytes, the index the largest of the running lanes' (they all hold it).
            _ if self.v.level(ix) == crate::k::Level::Uniform
                && self.v.kind(ix) == Kind::Bin
                && Op::of(self.v.w(ix, 0)) == Op::Mul
                && self.v.kind(self.v.child(ix, 0)) == Kind::Cast
                && M(self.v.w(self.v.child(ix, 0), 0)).bytes() == 4
                && self.v.kind(self.v.child(ix, 1)) == Kind::Const
                && self.v.w(self.v.child(ix, 1), 1) == bytes
                && self.v.w(self.v.child(ix, 1), 2) == 0 =>
            {
                let x = self.expr(self.v.child(self.v.child(ix, 0), 0), cx)?;
                let l = self.lanes(&x, M::U32)?;
                let mut on: Vec<String> = (0..self.p).map(|r| format!("vandq_u32({l}[{r}], {}[{r}])", cx.cur)).collect();
                while on.len() > 1 {
                    on = on.chunks(2).map(|c| if c.len() == 2 { format!("vmaxq_u32({}, {})", c[0], c[1]) } else { c[0].clone() }).collect();
                }
                let t = self.tmp();
                self.line(format!("let {t} = vmaxvq_u32({}) as usize;", on[0]));
                return Ok(Some(format!("{name}[{t}]")));
            }
            _ => return err("an element of a thread's array at an offset of each lane"),
        };
        if let Some(k) = konst64(&s) {
            let b = u64::from(bytes);
            if k % b != 0 || k / b >= u64::from(elems) {
                return err(format!("an element of a thread's array at byte {k}"));
            }
            return Ok(Some(format!("{name}[{}]", k / b)));
        }
        let t = self.tmp();
        self.line(format!("let {t} = ({s}) as usize / {bytes};"));
        Ok(Some(format!("{name}[{t}]")))
    }

    /// Element `e` of a thread's array set to `x`: in the lanes that run.
    fn set_elem(&mut self, e: &str, m: M, x: &V, cx: &Cx) -> R<()> {
        let l = self.lanes(x, m)?;
        if cx.divergent {
            let b = self.blend(m, &cx.cur, &l, e);
            self.line(format!("{e} = {b};"));
        } else {
            self.line(format!("{e} = {l};"));
        }
        Ok(())
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

    /// A function's result's Rust type: a value's (`ty`), or an aggregate's bytes, one value's or each lane's.
    fn result_ty(&self, m: M, lanes: bool) -> R<String> {
        if m.is_agg() {
            return Ok(if lanes { format!("[[u8; {}]; {}]", m.bytes(), self.w) } else { format!("[u8; {}]", m.bytes()) });
        }
        self.ty(m, lanes)
    }

    fn ty(&self, m: M, lanes: bool) -> R<String> {
        Ok(match (lanes, m) {
            (_, m) if m.is_agg() => "*const u8".into(),
            (false, M::B) => "bool".into(),
            (false, M::PTR) => "*const u8".into(),
            (false, m) => scalar(m)?.into(),
            // A narrower integer's lanes: 32-bit ones, holding it extended (`narrowed`).
            (true, m) if m == M::B || matches!(m.bytes(), 1 | 2 | 4) => format!("[uint32x4_t; {}]", self.p),
            (true, m) if wide(m) => format!("[uint32x4_t; {}]", 2 * self.p),
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
                let ps: Vec<(u32, M)> = (0..np).map(|k| (k as u32, M(v.w(n, 6 + 2 * k)))).collect();
                for (k, (off, m)) in ps.into_iter().enumerate() {
                    // A buffer: `arg{k}`, as every function that is passed it names it.
                    let name = if m == M::PTR { format!("arg{k}") } else { format!("x{off}") };
                    let val = if m.is_agg() { format!("*argv.add({k})") } else if m == M::PTR { format!("(*argv.add({k}) as *const *const u8).read_unaligned()") } else { format!("(*argv.add({k}) as *const {}).read_unaligned()", scalar(m)?) };
                    self.line(format!("let {name}: {} = {val};", self.ty(if m.is_agg() { M::PTR } else { m }, false)?));
                    self.vars.insert((u32::MAX, off), Var { name, m: if m.is_agg() { M::PTR } else { m }, lanes: false, iota: false, fixed: None });
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
                if block && !(stmts[0]..stmts[0] + self.v.size(stmts[0])).any(|i| self.v.kind(i) == Kind::Local && !self.vars.contains_key(&(u32::MAX, self.vid(i)))) {
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
                // A variable the loop sets: its lanes' addresses made before it are not its next rounds' (`addrs`).
                for i in n..n + v.size(n) {
                    if matches!(v.kind(i), Kind::Store | Kind::Update) && v.kind(v.child(i, 0)) == Kind::Local {
                        let key = format!("addr x{}", self.vid(v.child(i, 0)));
                        self.memo.retain(|m| m.0 != key);
                    }
                }
                // Where all lanes run, a condition with no effects and a body none leaves but by it: rounds unmasked
                // while every lane's condition holds, then (the chunk's last rounds) each lane its own. Its test is
                // the loop's own where the condition is one value, or consecutive lanes against one; it is worth
                // another where the hint says the lanes go on together (`UKL_LIKELY`), and not otherwise (lanes
                // that part early make it a second branch of each chunk's that the processor guesses wrong).
                // A condition of one value (the tape's, or this instance's: of uniform arguments), and nothing in the
                // loop that ends some lanes' rounds: all its lanes go on or none, so its rounds are Rust's, their
                // lanes the statement's. A condition of lanes here: what was written of it goes, as below.
                let uniform = v.level(cond) == crate::k::Level::Uniform;
                // A `continue` of this loop (not of a loop inside it) ends some lanes' round, not their rounds: every
                // lane comes to the step. In rounds of one condition each round's lanes that continue skip the rest
                // of its body (`cont`), and the step is the statement's lanes'.
                let end = n + v.size(n);
                let inner = |i: usize| (n + 1..i).any(|q| matches!(v.kind(q), Kind::While | Kind::For | Kind::Do) && i < q + v.size(q));
                // A `break` of a switch in the loop stays in its round.
                let switched = |i: usize| v.kind(i) == Kind::Break && (n + 1..i).any(|q| v.kind(q) == Kind::Switch && i < q + v.size(q) && !(q + 1..i).any(|r| matches!(v.kind(r), Kind::While | Kind::For | Kind::Do) && i < r + v.size(r)));
                let skips = uniform
                    && (n..end).any(|i| v.kind(i) == Kind::Continue)
                    && (n..end).all(|i| match v.kind(i) {
                        Kind::Break | Kind::Return => false,
                        Kind::Continue => !inner(i) && v.level(i) != crate::k::Level::Uniform,
                        _ => true,
                    });
                // A `break` of this loop that the lanes in a round take together (each statement between the loop
                // and it an `if` of a condition of one value, a block or an expression), and no other exit of it:
                // all its lanes leave at once, so it is Rust's `break` of the loop (its label). (A statement's level
                // is its control's, the loop's lanes apart or not; its conditions' are the values'.)
                let together = |i: usize| {
                    (n + 1..i).filter(|&q| i < q + v.size(q)).all(|q| match v.kind(q) {
                        Kind::If => v.level(v.child(q, 0)) == crate::k::Level::Uniform,
                        Kind::Block | Kind::Expr => true,
                        _ => q == body,
                    })
                };
                let breaks = uniform
                    && !skips
                    && (n..end).any(|i| v.kind(i) == Kind::Break && !inner(i) && !switched(i))
                    && (n..end).all(|i| match v.kind(i) {
                        Kind::Break => inner(i) || switched(i) || together(i),
                        Kind::Continue => inner(i),
                        Kind::Return => false,
                        _ => true,
                    });
                let leaves = !skips && !breaks && (n..end).any(|i| matches!(v.kind(i), Kind::Break | Kind::Continue | Kind::Return) && !switched(i));
                if !leaves && !self.effects(cond) {
                    let (at, top) = (self.out.len(), self.top);
                    let label = if breaks { Some(format!("'{}", self.tmp())) } else { None };
                    self.line(match &label { Some(l) => format!("{l}: loop {{"), None => "loop {".into() });
                    self.ind += 1;
                    self.line("// Rounds of one condition: every lane goes on.");
                    let c = self.expr(cond, cx)?;
                    // One value in the lanes that run, held as each lane's (a variable set where lanes were apart):
                    // theirs, any one's.
                    let c = match c {
                        c @ V::S(..) => c,
                        c if uniform => {
                            let k = self.mask(&c)?;
                            let on = if cx.full { k } else { let cur = cx.cur.clone(); self.map(|r| format!("vandq_u32({k}[{r}], {cur}[{r}])")) };
                            V::S(self.any(&on), M::B)
                        }
                        c => c,
                    };
                    match c {
                        V::S(c, _) => {
                            self.line(format!("if !({}) {{ break; }}", truth(&c, M::B)));
                            if skips {
                                let cont = self.tmp();
                                self.line(format!("let mut {cont} = {};", self.zeros()));
                                self.stmt(body, &Cx { cont: Some(cont), ..cx.clone() })?;
                            } else if let Some(l) = &label {
                                // Its breaks are its label's: an outer loop's lanes that break (`brk`) are not its.
                                self.stmt(body, &Cx { exit: Some(l.clone()), brk: None, ..cx.clone() })?;
                            } else {
                                self.stmt(body, cx)?;
                            }
                            if let Some(s) = step {
                                self.stmt(s, cx)?;
                            }
                            self.ind -= 1;
                            self.line("}");
                            return Ok(());
                        }
                        c if uniform => return err(format!("a uniform loop's condition of lanes: {c:?}")),
                        _ => {
                            self.out.truncate(at + (self.top - top));
                            self.ind -= 1;
                            let d = self.ind;
                            self.memo.retain(|m| m.2 <= d);
                            self.valid.retain(|m| m.1 <= d);
                        }
                    }
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
                    if m.is_agg() {
                        // An aggregate's bytes, from its address: one value's, or each running lane's from its own.
                        let size = m.bytes();
                        match (&x, lanes) {
                            (V::S(a, _), false) => self.line(format!("{res} = ({a} as *const [u8; {size}]).read_unaligned();")),
                            (_, false) => {
                                self.again = true;
                                self.rets.insert(cx.ctx, true);
                            }
                            (x, true) => {
                                let w = self.w;
                                let V::S(a, am) = self.lane_of(x)? else { return err("an aggregate result's address at a lane") };
                                let a = if am == M::PTR { a } else { format!("({a} as usize as *const u8)") };
                                let guard = if cx.divergent {
                                    let k = self.mask(&V::K(cx.cur.clone()))?;
                                    let km = self.tmp();
                                    self.line(format!("let {km} = core::mem::transmute::<[uint32x4_t; {}], [u32; {w}]>({k});", self.p));
                                    format!("if {km}[j] != 0 ")
                                } else {
                                    String::new()
                                };
                                self.line(format!("for j in 0..{w} {{ {guard}{{ {res}[j] = ({a} as *const [u8; {size}]).read_unaligned(); }} }}"));
                            }
                        }
                    } else if lanes {
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
                // A loop's label (a `break` all its lanes take together): Rust's.
                if v.kind(n) == Kind::Break
                    && let Some(l) = &cx.exit
                    && (cx.brk.is_none() || v.level(n) == crate::k::Level::Uniform)
                {
                    self.line(format!("break {l};"));
                    return Ok(());
                }
                let to = if v.kind(n) == Kind::Break { cx.brk.clone() } else { cx.cont.clone() };
                let Some(to) = to else { return err("a break or continue out of no loop") };
                let cur = cx.cur.clone();
                self.line(format!("{to} = [{}];", (0..self.p).map(|r| format!("vorrq_u32({to}[{r}], {cur}[{r}])")).collect::<Vec<_>>().join(", ")));
            }
            Kind::Switch => self.switch(n, cx)?,
            Kind::Do => self.stmt_do(n, cx)?,
            k => return err(format!("a {k:?} statement")),
        }
        Ok(())
    }

    /// Switch `n`: each case entered at the case of its value (else the default, else none) and run by the lanes
    /// entered at it or before it (falling through), until they break. Its breaks that all its lanes take (of one
    /// level) are Rust's `break` of its block; others, its mask of the lanes that broke.
    fn switch(&mut self, n: usize, cx: &Cx) -> R<()> {
        let v = &self.v;
        let kids: Vec<usize> = v.children(n).collect();
        let (value, cases) = (kids[0], &kids[1..]);
        let m = M(v.w(n, 1));
        if m.bytes() > 4 {
            return err("a switch of a 64-bit value");
        }
        let end = n + v.size(n);
        // Its own breaks: none of a loop or a switch inside it.
        let own: Vec<usize> = (n + 1..end).filter(|&i| v.kind(i) == Kind::Break && !(n + 1..i).any(|q| matches!(v.kind(q), Kind::While | Kind::For | Kind::Do | Kind::Switch) && i < q + v.size(q))).collect();
        let apart = own.iter().any(|&i| v.level(i) != crate::k::Level::Uniform);
        let labels: Vec<Option<u32>> = cases.iter().map(|&c| (v.w(c, 0) == 0).then(|| v.w(c, 1))).collect();
        let default = labels.iter().position(Option::is_none).unwrap_or(cases.len());
        let x = self.expr(value, cx)?;
        let label = format!("'{}", self.tmp());
        let brk = apart.then(|| self.tmp());
        if let Some(b) = &brk {
            self.line(format!("let mut {b} = {};", self.zeros()));
        }
        let e = self.tmp();
        match &x {
            V::S(s, xm) => {
                let arms: Vec<String> = labels.iter().enumerate().filter_map(|(j, l)| l.map(|l| format!("{l}u32 => {j}u32"))).collect();
                self.line(format!("let {e}: u32 = match {} {{ {}{}_ => {default}u32 }};", conv(s, *xm, M::U32), arms.join(", "), if arms.is_empty() { "" } else { ", " }));
            }
            x => {
                let l = self.lanes(x, M::U32)?;
                // The first case of each lane's value: the cases' tests from the last to the first.
                let mut at = self.map(|_| format!("vdupq_n_u32({default})"));
                for (j, l2) in labels.iter().enumerate().rev() {
                    let Some(lv) = l2 else { continue };
                    at = self.map(|r| format!("vbslq_u32(vceqq_u32({l}[{r}], vdupq_n_u32({lv}u32)), vdupq_n_u32({j}), {at}[{r}])"));
                }
                self.line(format!("let {e} = {at};"));
            }
        }
        self.line(format!("{label}: {{"));
        self.ind += 1;
        let uniform = matches!(x, V::S(..));
        for (j, &c) in cases.iter().enumerate() {
            let body = self.v.child(c, 0);
            if uniform {
                self.line(format!("if {e} <= {j}u32 {{"));
                self.ind += 1;
                let cur = match &brk {
                    Some(b) => {
                        let cur = cx.cur.clone();
                        self.map(|r| format!("vbicq_u32({cur}[{r}], {b}[{r}])"))
                    }
                    None => cx.cur.clone(),
                };
                let divergent = cx.divergent || brk.is_some();
                let inner = Cx { cur, divergent, full: cx.full && brk.is_none(), brk: brk.clone(), exit: Some(label.clone()), ..cx.clone() };
                self.stmt(body, &inner)?;
            } else {
                let cur = cx.cur.clone();
                let on = self.map(|r| {
                    let k = format!("vandq_u32({cur}[{r}], vcleq_u32({e}[{r}], vdupq_n_u32({j})))");
                    match &brk {
                        Some(b) => format!("vbicq_u32({k}, {b}[{r}])"),
                        None => k,
                    }
                });
                self.line(format!("if {} {{", self.any(&on)));
                self.ind += 1;
                let inner = Cx { cur: on, divergent: true, full: false, left: false, brk: brk.clone(), exit: Some(label.clone()), region: Some(n), ..cx.clone() };
                self.stmt(body, &inner)?;
            }
            self.ind -= 1;
            self.line("}");
        }
        self.ind -= 1;
        self.line("}");
        Ok(())
    }

    /// Loop `n`'s rounds, each lane its own: the lanes still in it, those whose condition holds; none, it ends.
    /// Whether lanes loop `n`'s rounds may be each lane's in turn (`finish_scalar`): it leaves only by its condition,
    /// calls nothing, writes only variables, and reads no lane's id, SIMD group, barrier or atomic (a lane's rounds
    /// run alone are one interleaving of the threads').
    fn scalar_loop(&self, n: usize) -> bool {
        (n..n + self.v.size(n)).all(|i| match self.v.kind(i) {
            Kind::Break | Kind::Continue | Kind::Return | Kind::Call | Kind::Inline => false,
            Kind::Store | Kind::Update => {
                let a = self.v.child(i, 0);
                self.v.kind(a) == Kind::Local && self.v.w(a, 2) == 0
            }
            Kind::Builtin => {
                let b = Builtin::of(self.v.w(i, 0));
                matches!(b, Builtin::LocalSize | Builtin::GlobalSize | Builtin::GroupId | Builtin::NumGroups | Builtin::MemFence) || b as u32 >= Builtin::Popcount as u32
            }
            _ => true,
        })
    }

    /// At a lanes loop `n`'s round, `lm` its lanes that go on: where at most `FEW` do, each one's rounds to the
    /// loop's end in turn, as Rust's rounds of one value (a round of all the chunk's lanes costs as much as of all of
    /// them: its gathers a load a lane either way), then the loop left. Each variable the loop reads that is each
    /// lane's (consecutive lanes, an address of each lane's) is the lane's copy for those rounds, and each it writes
    /// one, written back to the lane after; a variable of one value it writes (dead past the loop, or it would be each
    /// lane's) is a copy, each lane's rounds from its value now. Not written where a round's value is of lanes.
    /// Whether node `n`'s condition (a short circuit, `?:`) is of lanes where its first operand is one value, as a
    /// pass learned: never in a lane's scalar rounds (`finish_scalar`), whose values are that lane's.
    fn lane_cond(&self, ctx: u32, n: usize) -> bool {
        self.shadows.is_empty() && self.lane_conds.contains(&(ctx, n))
    }

    /// Node `n`'s condition found of lanes: learned for the next pass; in a lane's scalar rounds, those rounds not
    /// written.
    fn learn_lane_cond(&mut self, ctx: u32, n: usize) {
        if self.shadows.is_empty() {
            self.lane_conds.insert((ctx, n));
            self.again = true;
        } else {
            self.shadow_fail = true;
        }
    }

    fn finish_scalar(&mut self, n: usize, lm: &str, cx: &Cx) -> R<()> {
        if !self.scalar_loop(n) {
            return Ok(());
        }
        let kids: Vec<usize> = self.v.children(n).collect();
        let (cond, body, step) = (kids[0], kids[1], kids.get(2).copied());
        let (at, top, ind) = (self.out.len(), self.top, self.ind);
        let (w, p) = (self.w, self.p);
        let (bits, left) = (self.tmp(), self.tmp());
        self.line(format!("let {bits} = bits(&{lm});"));
        self.line(format!("if {bits}.count_ones() <= {} {{", Self::FEW));
        self.ind += 1;
        self.line(format!("let mut {left} = {bits};"));
        self.line(format!("while {left} != 0 {{"));
        self.ind += 1;
        self.line(format!("let j = {left}.trailing_zeros() as usize;"));
        self.line(format!("{left} &= {left} - 1;"));
        let end = n + self.v.size(n);
        let mut ids: Vec<u32> = (n..end).filter(|&i| self.v.kind(i) == Kind::Local).map(|i| self.vid(i)).collect();
        ids.sort();
        ids.dedup();
        let written: Vec<u32> = (n..end).filter(|&i| matches!(self.v.kind(i), Kind::Store | Kind::Update) && self.v.kind(self.v.child(i, 0)) == Kind::Local).map(|i| self.vid(self.v.child(i, 0))).collect();
        let mut saved = Vec::new();
        let mut back = Vec::new();
        let mut ok = true;
        for id in ids {
            let Some(var) = self.vars.get(&(cx.ctx, id)).cloned() else { continue };
            let writes = written.contains(&id);
            let now = match &var.fixed {
                Some(V::S(..)) => None,
                Some(f) => Some(f.clone()),
                None if var.lanes => Some(if var.m == M::B { V::K(var.name.clone()) } else { V::L(var.name.clone(), var.m) }),
                None if var.iota => Some(V::Iota(var.name.clone(), var.m)),
                None if writes => Some(V::S(var.name.clone(), var.m)),
                None => None,
            };
            let Some(now) = now else { continue };
            let V::S(e, em) = self.lane_of(&now)? else {
                ok = false;
                break;
            };
            let copy = if var.fixed.is_some() && !writes {
                Var { fixed: Some(V::S(conv(&e, em, var.m), var.m)), ..var.clone() }
            } else {
                let name = format!("{}_j", var.name);
                self.line(format!("let mut {name}: {} = {};", scalar(var.m)?, conv(&e, em, var.m)));
                self.shadows.insert(name.clone());
                if var.lanes && writes {
                    back.push((var.clone(), name.clone()));
                }
                Var { name, m: var.m, lanes: false, iota: false, fixed: None }
            };
            saved.push((id, var));
            self.vars.insert((cx.ctx, id), copy);
        }
        if ok {
            let one = Cx { cur: self.ones(), divergent: false, left: false, brk: None, exit: None, cont: None, full: true, looped: None, region: None, ..cx.clone() };
            self.shadow_fail = false;
            self.line("loop {");
            self.ind += 1;
            match self.expr(cond, &one)? {
                V::S(c, _) => {
                    self.line(format!("if !({}) {{ break; }}", truth(&c, M::B)));
                    self.stmt(body, &one)?;
                    if let Some(s) = step {
                        self.stmt(s, &one)?;
                    }
                }
                _ => ok = false,
            }
            self.ind -= 1;
            self.line("}");
            // The lane's values back in their lanes.
            for (var, copy) in &back {
                let regs = if wide(var.m) { 2 * p } else { p };
                let (lo, hi) = if var.m == M::B {
                    (format!("if {copy} {{ u32::MAX }} else {{ 0 }}"), None)
                } else if wide(var.m) {
                    (format!("{copy} as u32"), Some(format!("({copy} as u64 >> 32) as u32")))
                } else if var.m.float() {
                    (format!("{copy}.to_bits()"), None)
                } else {
                    (format!("{copy} as u32"), None)
                };
                let a = self.tmp();
                self.line(format!("let mut {a} = core::mem::transmute::<[uint32x4_t; {regs}], [u32; {}]>({});", 4 * regs, var.name));
                self.line(format!("{a}[j] = {lo};"));
                if let Some(hi) = hi {
                    self.line(format!("{a}[{w} + j] = {hi};"));
                }
                self.line(format!("{} = core::mem::transmute::<[u32; {}], [uint32x4_t; {regs}]>({a});", var.name, 4 * regs));
            }
            self.ind -= 1;
            self.line("}");
            self.line("break;");
            self.ind -= 1;
            self.line("}");
        }
        for (id, var) in saved {
            self.vars.insert((cx.ctx, id), var);
        }
        self.shadows.clear();
        if !ok || std::mem::take(&mut self.shadow_fail) {
            // Rounds of lanes: what was written of them goes (after what the function's top gained since).
            self.out.truncate(at + (self.top - top));
            self.ind = ind;
            self.memo.retain(|m| m.2 <= ind);
            self.valid.retain(|m| m.1 <= ind);
        }
        Ok(())
    }

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
        self.finish_scalar(n, &lm, cx)?;
        self.line("// Lanes part: each its own rounds.");
        let head = Cx { cur: lm.clone(), divergent: true, left: false, brk: hb.then(|| brk.clone()), exit: None, cont: None, full: false, looped: Some(n), region: Some(n), ..cx.clone() };
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
        let inner = Cx { cur: lm.clone(), divergent: true, left: false, brk: hb.then(|| brk.clone()), exit: None, cont: hc.then(|| cont.clone()), full: false, looped: Some(n), region: Some(n), ..cx.clone() };
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

    /// Loop `do body while (cond)`, node `n`: its body, then its condition, in the lanes still in it. A condition of
    /// one value and nothing that leaves it but its end: Rust's loop, its lanes the statement's.
    fn stmt_do(&mut self, n: usize, cx: &Cx) -> R<()> {
        let v = &self.v;
        let kids: Vec<usize> = v.children(n).collect();
        let (body, cond) = (kids[0], kids[1]);
        let end = n + v.size(n);
        let leaves = (n + 1..end).any(|i| matches!(v.kind(i), Kind::Break | Kind::Continue | Kind::Return));
        // A variable the loop sets: its lanes' addresses made before it are not its next rounds' (`addrs`).
        for i in n..end {
            if matches!(v.kind(i), Kind::Store | Kind::Update) && v.kind(v.child(i, 0)) == Kind::Local {
                let key = format!("addr x{}", self.vid(v.child(i, 0)));
                self.memo.retain(|m| m.0 != key);
            }
        }
        if v.level(cond) == crate::k::Level::Uniform && !leaves && !self.effects(cond) {
            self.line("loop {");
            self.ind += 1;
            self.stmt(body, cx)?;
            if let V::S(c, cm) = self.expr(cond, cx)? {
                self.line(format!("if !({}) {{ break; }}", truth(&c, cm)));
                self.ind -= 1;
                self.line("}");
                return Ok(());
            }
            return err("a do loop's condition of one value, of lanes here");
        }
        let has = |k: Kind| (n..end).any(|i| self.v.kind(i) == k);
        let (hb, hc) = (has(Kind::Break), has(Kind::Continue));
        let lm = self.tmp();
        self.line(format!("let mut {lm} = {};", cx.cur));
        let brk = self.tmp();
        if hb {
            self.line(format!("let mut {brk} = {};", self.zeros()));
        }
        self.line("loop {");
        self.ind += 1;
        let cont = self.tmp();
        if hc {
            self.line(format!("let mut {cont} = {};", self.zeros()));
        }
        let inner = Cx { cur: lm.clone(), divergent: true, left: false, brk: hb.then(|| brk.clone()), exit: None, cont: hc.then(|| cont.clone()), full: false, looped: Some(n), region: Some(n), ..cx.clone() };
        self.stmt(body, &inner)?;
        // The lanes that broke or returned leave; those that continued come to the condition.
        if hb {
            self.line(format!("{lm} = [{}];", (0..self.p).map(|r| format!("vbicq_u32({lm}[{r}], {brk}[{r}])")).collect::<Vec<_>>().join(", ")));
        }
        if cx.live != lm && self.returns(body) {
            let live = cx.live.clone();
            self.line(format!("{lm} = [{}];", (0..self.p).map(|r| format!("vandq_u32({lm}[{r}], {live}[{r}])")).collect::<Vec<_>>().join(", ")));
        }
        let head = Cx { cur: lm.clone(), divergent: true, left: false, brk: None, exit: None, cont: None, full: false, looped: Some(n), region: Some(n), ..cx.clone() };
        self.line(format!("if {} {{", self.any(&lm)));
        self.ind += 1;
        let c = self.expr(cond, &head)?;
        let k = self.mask(&c)?;
        self.line(format!("{lm} = [{}];", (0..self.p).map(|r| format!("vandq_u32({lm}[{r}], {k}[{r}])")).collect::<Vec<_>>().join(", ")));
        self.ind -= 1;
        self.line("}");
        self.line(format!("if !({}) {{ break; }}", self.any(&lm)));
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
                let returns = !(c..c + self.v.size(c)).any(|i| matches!(self.v.kind(i), Kind::Break | Kind::Continue));
                cx.left = (!cx.divergent || cx.left) && returns && cx.looped.is_none() && cx.region.is_none();
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
        let inner = Cx { cur: t.clone(), divergent: true, left: false, full: false, looped: None, region: Some(kids[1]), ..cx.clone() };
        self.side(kids[1], &t, &inner, rejoins)?;
        if kids.len() > 2 {
            let e = if cx.full { self.map(|r| format!("vmvnq_u32({k}[{r}])")) } else { self.map(|r| format!("vbicq_u32({cur}[{r}], {k}[{r}])")) };
            let inner = Cx { cur: e.clone(), divergent: true, left: false, full: false, looped: None, region: Some(kids[2]), ..cx.clone() };
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
        if self.risky || (!rejoins && !self.merged_exit(s)) {
            self.out.replace_range(at..at + open.len(), &open.replacen('{', &format!("if {} {{", self.any(t)), 1));
        }
        self.risky |= before;
        Ok(())
    }

    /// Whether side `s`, which ends some lanes, is a value merged by its mask and those lanes let off: a `return`
    /// (or a value set, then one) of expressions with no loop, call or effect. Run masked, it costs less than a test of
    /// whether any lane takes it: the test is a reduction across the lanes and a branch, each of a chain of returns.
    fn merged_exit(&self, s: usize) -> bool {
        let n = s + self.v.size(s);
        self.ends(s)
            && self.v.size(s) <= 24
            && (s..n).all(|i| !matches!(self.v.kind(i), Kind::While | Kind::Do | Kind::For | Kind::Call | Kind::Inline | Kind::Update | Kind::Break | Kind::Continue | Kind::If))
            && !self.effects(s)
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
        (self.v.kind(n) == Kind::Load && self.v.kind(self.v.child(n, 0)) == Kind::Local && self.v.w(self.v.child(n, 0), 2) == 0).then(|| self.vid(self.v.child(n, 0)))
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
                let pk = k as u32;
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
            if self.v.kind(i) == Kind::Local && self.vid(i) == off {
                out.push(i);
            }
            i += 1;
        }
        out
    }

    /// Parameter `p` (a function's number and the parameter's) of consecutive lanes would take a chunk that crosses
    /// no `2^k`: its callers test theirs from the next pass (the smallest `k` asked, which serves every larger).
    fn want(&mut self, p: (u32, usize), k: u32) {
        let w = self.wants.entry(p).or_insert(u32::MAX);
        if k < *w {
            *w = k;
            self.again = true;
        }
    }

    /// Node `n`'s parent.
    fn parent(&self, n: usize) -> Option<usize> {
        (0..n).rev().find(|&q| q + self.v.size(q) > n)
    }

    /// Whether every read of variable `off` of `cx`'s function (a load of it, an update) is of one value in the lanes
    /// that run it (the vector tape's `Uniform`: where control rejoins, a variable written apart is raised to the
    /// level that split it, so a read there is not). Then one value holds it, whatever lanes run its writes: a lane
    /// that did not run one never reads what it wrote as its own.
    fn read_uniform(&self, cx: &Cx, off: u32) -> bool {
        let f = if cx.kernel { 0 } else { self.bodies[&cx.body] };
        let inner = |i: usize| self.bodies.values().any(|&b| b != f && b > f && b < i && i < b + self.v.size(b));
        let mut any = false;
        for i in f..f + self.v.size(f) {
            if self.v.kind(i) != Kind::Local || self.vid(i) != off || inner(i) {
                continue;
            }
            let Some(q) = self.parent(i) else { return false };
            match self.v.kind(q) {
                Kind::Load | Kind::Update if self.v.child(q, 0) == i && self.v.w(i, 2) == 0 => {
                    if self.v.level(q) != crate::k::Level::Uniform {
                        return false;
                    }
                    any = true;
                }
                Kind::Store if self.v.child(q, 0) == i && self.v.w(i, 2) == 0 => {}
                _ => return false,
            }
        }
        any
    }

    /// Whether variable `off`, written where `cx`'s lanes run (lanes apart), is read by none of the lanes that do
    /// not: then its write needs no mask. Either the lanes off have returned (`Cx::left`); or the statements are a
    /// varying loop's (at its level), the loop holds
    /// no `continue` nor lies in a loop, and no access of the variable follows it (a lane that leaves never reads
    /// it again); or every access lies in the statement whose lanes run, the first a write of it there on every
    /// path (each entry writes it before it reads it).
    fn dead_off(&self, cx: &Cx, off: u32) -> bool {
        if cx.left {
            return true;
        }
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
            Kind::Local if self.struct_of(n, cx).is_some() => return self.struct_bytes(n, cx),
            // A structure in memory, not an alias: each lane's row, plus the byte.
            Kind::Local if self.mems.contains_key(&(cx.ctx, self.vid(n))) && !self.aliases.contains(&(cx.ctx, self.vid(n))) => {
                let (name, size) = self.mems[&(cx.ctx, self.vid(n))].clone();
                let a = V::At { base: format!("({name}.as_mut_ptr() as *const u8)"), idx: Box::new(V::Iota("0u32".into(), M::U32)), scale: size };
                return offset_addr(a, p[2]);
            }
            Kind::Local if self.v.w(n, 2) == 0 && self.arrays.contains_key(&(cx.ctx, self.vid(n))) => {
                let (name, e, b) = self.arrays[&(cx.ctx, self.vid(n))].clone();
                V::Arr(name, e, b)
            }
            Kind::Local => {
                let off = self.vid(n);
                // A structure read where it lies (`Kind::Copy`), or a parameter of one at each lane's address: its
                // address, plus the byte asked.
                if (self.aliases.contains(&(cx.ctx, off)) || self.agg_param(cx, off))
                    && let Some(Var { fixed: Some(a), .. }) = self.vars.get(&(cx.ctx, off)).cloned()
                {
                    return offset_addr(a, p[2]);
                }
                match self.vars.get(&(cx.ctx, off)).cloned() {
                    // An aggregate's (a structure's bytes where they are): its address.
                    Some(var) if var.m == M::PTR && !var.lanes && self.v.w(n, 1) > 8 => V::S(var.name.clone(), M::PTR),
                    // A variable's own (not one of a parameter's shape): each lane's, passed by `&mut` (`V::Ref`).
                    Some(var) if var.fixed.is_none() && p[2] == 0 => {
                        if !var.lanes {
                            self.lanes.insert((cx.ctx, off), true);
                            self.again = true;
                        }
                        V::Ref(var.name.clone(), var.m)
                    }
                    _ => return err(format!("the address of a thread's variable (at {off})")),
                }
            }
            Kind::Load => {
                let m = M(p[0]);
                if let Some(e) = self.elem(kids[0], cx)? {
                    return Ok(V::L(e, m));
                }
                if m.is_agg() {
                    if self.struct_of(kids[0], cx).is_some() {
                        return self.struct_bytes(kids[0], cx);
                    }
                    return self.expr(kids[0], cx);
                }
                // A scalar of constant memory (a `__constant` variable's), which nothing writes: its value.
                if self.v.kind(kids[0]) == Kind::Global
                    && !m.float()
                    && m != M::PTR
                    && let Some(b) = self.data.get(self.v.w(kids[0], 0) as usize..).and_then(|d| d.get(..m.bytes() as usize))
                {
                    let x = b.iter().rev().fold(0u64, |x, &y| x << 8 | u64::from(y));
                    return Ok(V::S(literal(m, x)?, m));
                }
                if let Some(var) = self.var_of(kids[0], cx) {
                    if let Some(f) = var.fixed {
                        return Ok(f);
                    }
                    return Ok(if var.lanes {
                        if var.m == M::B { V::K(var.name) } else { V::L(var.name, var.m) }
                    } else if var.iota {
                        V::Iota(var.name, var.m)
                    } else {
                        V::S(var.name, var.m)
                    });
                }
                let a = self.expr(kids[0], cx)?;
                // Through a variable's address: the variable.
                if let V::Ref(place, rm) = &a {
                    if rm.bytes() != m.bytes() {
                        return err(format!("a load of {m:?} from a variable of {rm:?}"));
                    }
                    return Ok(if m == M::B { V::K(place.clone()) } else { V::L(place.clone(), m) });
                }
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
                if let Some(e) = self.elem(kids[0], cx)? {
                    self.set_elem(&e, m, &x, cx)?;
                    return Ok(x);
                }
                if let Some(var) = self.var_of(kids[0], cx) {
                    self.assign(&var, x.clone(), cx)?;
                    return Ok(x);
                }
                let a = self.expr(kids[0], cx)?;
                // Through a variable's address: the variable, as its own store in the lanes that run.
                if let V::Ref(place, rm) = a {
                    let var = Var { name: place, m: rm, lanes: true, iota: false, fixed: None };
                    self.assign(&var, x.clone(), cx)?;
                    return Ok(x);
                }
                self.store(m, &a, &x, cx)?;
                x
            }
            Kind::Update => {
                let (op, lm, om, post) = (Op::of(p[0]), M(p[1]), M(p[2]), p[3] != 0);
                if let Some(e) = self.elem(kids[0], cx)? {
                    let old = V::L(e.clone(), lm);
                    let x = self.expr(kids[1], cx)?;
                    let op = if op == Op::None { Op::Add } else { op };
                    let saved = if post { Some(self.hold(&old)?) } else { None };
                    let new = self.bin(op, om, &old, &x)?;
                    self.set_elem(&e, lm, &new, cx)?;
                    return Ok(saved.unwrap_or(new));
                }
                let var = match self.var_of(kids[0], cx) {
                    Some(var) => var,
                    // Through a variable's address: the variable.
                    None => match self.expr(kids[0], cx)? {
                        V::Ref(place, rm) => Var { name: place, m: rm, lanes: true, iota: false, fixed: None },
                        // Memory's: its value loaded, the operation, the value stored (in the lanes that run).
                        a => {
                            let old = self.load(lm, &a, cx)?;
                            let x = self.expr(kids[1], cx)?;
                            let op = if op == Op::None { Op::Add } else { op };
                            let saved = if post { Some(self.hold(&old)?) } else { None };
                            let new = self.bin(op, om, &old, &x)?;
                            self.store(lm, &a, &new, cx)?;
                            return Ok(saved.unwrap_or(new));
                        }
                    },
                };
                let old = if var.lanes { V::L(var.name.clone(), var.m) } else if var.iota { V::Iota(var.name.clone(), var.m) } else { V::S(var.name.clone(), var.m) };
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
                let r = self.bin(op, m, &a, &b)?;
                // Each lane's pointer plus one offset, where the pointer's lanes' addresses are made (`addrs`): its
                // lanes' addresses, theirs plus it.
                if op == Op::Add
                    && m == M::PTR
                    && let (V::L(an, _), V::S(off, _), V::L(rn, _)) = (&a, &b, &r)
                    && let Some(base) = self.memo.iter().find(|e| e.0 == format!("addr {an}")).map(|e| e.1.clone())
                {
                    let t = self.tmp();
                    self.line(format!("let {t}: [usize; {w}] = core::array::from_fn(|j| {base}[j].wrapping_add(({off}) as u64 as usize));", w = self.w));
                    let d = self.ind;
                    self.memo.push((format!("addr {rn}"), t, d));
                }
                r
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
                    // Of 64-bit lanes: each half's complement; a negation, 0 minus the value (its borrow between them).
                    (Op::Not, a) if wide(m) => {
                        let l = self.lanes(&a, m)?;
                        let t = self.tmp();
                        self.line(format!("let {t}: [uint32x4_t; {n}] = core::array::from_fn(|r| vmvnq_u32({l}[r]));", n = 2 * self.p));
                        V::L(t, m)
                    }
                    (Op::Neg, a) if wide(m) => self.bin64(Op::Sub, m, &V::S("0u64".into(), M::U64), &a)?,
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
                    // One value of every lane: C's short circuit as Rust's, the right side run only when it decides;
                    // of a right side of lanes (as the last pass found it), a mask, the left side's in every lane
                    // until the right side's decides.
                    let t = self.tmp();
                    let lanes = self.lane_cond(cx.ctx, n);
                    let x = truth(x, *xm);
                    // A right side of lanes that reads no memory (but variables) and calls nothing: evaluated whole,
                    // the left side's mask and its (no branch to skip a few operations).
                    let cheap = !(kids[1]..kids[1] + self.v.size(kids[1])).any(|i| match self.v.kind(i) {
                        Kind::Load => self.v.kind(self.v.child(i, 0)) != Kind::Local,
                        Kind::Call | Kind::Inline | Kind::Builtin | Kind::Store | Kind::Update => true,
                        _ => false,
                    });
                    if lanes && cheap {
                        let b = self.expr(kids[1], cx)?;
                        let kb = self.mask(&b)?;
                        let f = if and { "vandq_u32" } else { "vorrq_u32" };
                        return Ok(V::K(self.map(|r| format!("{f}(vdupq_n_u32(if {x} {{ u32::MAX }} else {{ 0 }}), {kb}[{r}])"))));
                    }
                    if lanes {
                        self.line(format!("let mut {t}: [uint32x4_t; {}] = [vdupq_n_u32(if {x} {{ u32::MAX }} else {{ 0 }}); {}];", self.p, self.p));
                        self.line(format!("if {}({x}) {{", if and { "" } else { "!" }));
                    } else {
                        self.line(format!("let mut {t}: bool = {x};"));
                        self.line(format!("if {}{t} {{", if and { "" } else { "!" }));
                    }
                    self.ind += 1;
                    let b = self.expr(kids[1], cx)?;
                    match b {
                        b if lanes => {
                            let k = self.mask(&b)?;
                            self.line(format!("{t} = {k};"));
                        }
                        V::S(y, ym) => self.line(format!("{t} = {};", truth(&y, ym))),
                        _ => {
                            self.learn_lane_cond(cx.ctx, n);
                        }
                    }
                    self.ind -= 1;
                    self.line("}");
                    return Ok(if lanes { V::K(t) } else { V::S(t, M::B) });
                }
                // Each lane's: a right side with no effect evaluated whole and combined; one with, under its lanes
                // (the next step).
                // One with: evaluated in the lanes that run whose left side does not decide (none, none evaluate it).
                if self.effects(kids[1]) {
                    let ka = self.mask(&a)?;
                    let cur = cx.cur.clone();
                    let on = match (and, cx.full) {
                        (true, true) => ka.clone(),
                        (true, false) => self.map(|r| format!("vandq_u32({cur}[{r}], {ka}[{r}])")),
                        (false, true) => self.map(|r| format!("vmvnq_u32({ka}[{r}])")),
                        (false, false) => self.map(|r| format!("vbicq_u32({cur}[{r}], {ka}[{r}])")),
                    };
                    let t = self.tmp();
                    self.line(format!("let mut {t}: [uint32x4_t; {}] = {ka};", self.p));
                    self.line(format!("if {} {{", self.any(&on)));
                    self.ind += 1;
                    let inner = Cx { cur: on.clone(), divergent: true, left: false, full: false, looped: None, region: None, ..cx.clone() };
                    let b = self.expr(kids[1], &inner)?;
                    let kb = self.mask(&b)?;
                    // The lanes evaluated take the right side's truth; the others keep the left's.
                    self.line(format!("{t} = [{}];", (0..self.p).map(|r| format!("vbslq_u32({on}[{r}], {kb}[{r}], {t}[{r}])")).collect::<Vec<_>>().join(", ")));
                    self.ind -= 1;
                    self.line("}");
                    return Ok(V::K(t));
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
                let mut vals = Vec::new();
                for &c in kids.iter().take(np) {
                    vals.push(self.expr(c, cx)?);
                }
                return self.call(id, vals, cx);
            }
            Kind::Builtin => return self.builtin(n, cx),
            Kind::Cond => return self.cond(n, cx),
            Kind::Comma => {
                self.expr(kids[0], cx)?;
                return self.expr(kids[1], cx);
            }
            Kind::Global => V::S(format!("DATA.as_ptr().wrapping_add({})", p[0]), M::PTR),
            Kind::Shared => {
                // The group's threadgroup memory, the kernel's (`tg`), as a buffer.
                let Some(k) = self.shared else { return err("threadgroup memory of a kernel with none") };
                V::S(if p[0] == 0 { format!("arg{k}") } else { format!("arg{k}.wrapping_add({})", p[0]) }, M::PTR)
            }
            Kind::Copy => {
                // A structure copied whole into a variable that is only read after, from bytes no write of the
                // kernel reaches (a buffer's, or a call's result): the variable is those bytes, read where they lie
                // (its address each lane's where the source's is), nothing copied (decision 0353).
                let (dst, src) = (kids[0], kids[1]);
                // A thread's array: its elements, each from the other array's or from bytes (or to bytes).
                let arr = |e: &Self, a: usize| (e.v.kind(a) == Kind::Local && e.v.w(a, 2) == 0).then(|| e.arrays.get(&(cx.ctx, e.vid(a))).cloned()).flatten();
                let (ad, asrc) = (arr(self, dst), arr(self, src));
                if ad.is_some() || asrc.is_some() {
                    let (_, elems, bytes) = ad.clone().or(asrc.clone()).expect("an array");
                    if ad.as_ref().is_some_and(|a| (a.1, a.2) != (elems, bytes)) || asrc.as_ref().is_some_and(|a| (a.1, a.2) != (elems, bytes)) || elems * bytes != p[0] {
                        return err("a copy between arrays of other shapes");
                    }
                    let m = if bytes == 8 { M::U64 } else { M::U32 };
                    let (da, sa) = (if ad.is_some() { None } else { Some(self.expr(dst, cx)?) }, if asrc.is_some() { None } else { Some(self.expr(src, cx)?) });
                    for e in 0..elems {
                        let x = match (&asrc, &sa) {
                            (Some((name, ..)), _) => V::L(format!("{name}[{e}]"), m),
                            (None, Some(a)) => {
                                let at = offset_addr(a.clone(), e * bytes)?;
                                self.load(m, &at, cx)?
                            }
                            _ => unreachable!(),
                        };
                        match (&ad, &da) {
                            (Some((name, ..)), _) => self.set_elem(&format!("{name}[{e}]"), m, &x, cx)?,
                            (None, Some(a)) => {
                                let at = offset_addr(a.clone(), e * bytes)?;
                                self.store(m, &at, &x, cx)?;
                            }
                            _ => unreachable!(),
                        }
                    }
                    return Ok(V::None);
                }
                // A thread's structure's fields: copied each.
                let sd = self.struct_of(dst, cx);
                let ss = self.struct_of(src, cx);
                if sd.is_some() || ss.is_some() {
                    let fields = sd.clone().or(ss.clone()).expect("a structure");
                    let (da, sa) = (if sd.is_some() { None } else { Some(self.expr(dst, cx)?) }, if ss.is_some() { None } else { Some(self.expr(src, cx)?) });
                    for (byte, m) in fields {
                        let x = match (&ss, &sa) {
                            (Some(f), _) => match f.iter().find(|f| f.0 == byte && f.1 == m) {
                                Some(_) => self.field_value(src, byte, cx)?,
                                None => return err("a copy between structures of other fields"),
                            },
                            (None, Some(a)) => {
                                let at = offset_addr(a.clone(), byte)?;
                                self.load(m, &at, cx)?
                            }
                            _ => unreachable!(),
                        };
                        match &da {
                            None => {
                                let var = self.vars.get(&(cx.ctx, field_key(self.vid(dst), byte))).cloned().expect("a field");
                                self.assign(&var, x, cx)?;
                            }
                            Some(a) => {
                                let at = offset_addr(a.clone(), byte)?;
                                self.store(m, &at, &x, cx)?;
                            }
                        }
                    }
                    return Ok(V::None);
                }
                if self.v.kind(dst) == Kind::Local && self.v.w(dst, 2) == 0 && self.only_read(dst, n) {
                    let a = self.expr(src, cx)?;
                    let k = match &a {
                        V::S(s, _) => root_of(s),
                        V::At { base, .. } => root_of(base),
                        _ => None,
                    };
                    // A call's result: its bytes the call's own, which nothing writes after.
                    if k.is_some_and(|k| self.unwritten(k)) || self.v.kind(src) == Kind::Call {
                        let id = self.vid(dst);
                        self.aliases.insert((cx.ctx, id));
                        self.vars.insert((cx.ctx, id), Var { name: format!("x{id}"), m: M::PTR, lanes: false, iota: false, fixed: Some(a.clone()) });
                        return Ok(a);
                    }
                }
                // A structure copied whole that `fields` left to be read where it lies, which it cannot be here: its
                // fields, from the next pass.
                if self.v.kind(dst) == Kind::Local && self.v.w(dst, 2) == 0 && self.unaliased.insert((cx.ctx, self.vid(dst))) {
                    self.again = true;
                    return Ok(V::None);
                }
                // Bytes to bytes: each running lane's (one copy where both addresses are one value's).
                let (d, s) = (self.expr(dst, cx)?, self.expr(src, cx)?);
                let k = match &d {
                    V::S(s, _) => root_of(s),
                    V::At { base, .. } => root_of(base),
                    _ => None,
                };
                if self.v.kind(dst) != Kind::Local {
                    self.writes.push((k, false));
                }
                let bytes = p[0];
                let ptr = |x: V| match x {
                    V::S(a, M::PTR) => Ok(V::S(a, M::PTR)),
                    V::S(a, _) => Ok(V::S(format!("({a} as usize as *const u8)"), M::PTR)),
                    x => Ok(x),
                };
                let (d, s) = (ptr(d)?, ptr(s)?);
                if let (V::S(da, _), V::S(sa, _)) = (&d, &s) {
                    let guard = if cx.full { "true".to_string() } else { self.any(&cx.cur.clone()) };
                    self.line(format!("if {guard} {{ core::ptr::copy({sa}, {da} as *mut u8, {bytes}); }}"));
                    return Ok(d);
                }
                let w = self.w;
                let (V::S(dj, djm), V::S(sj, sjm)) = (self.lane_of(&d)?, self.lane_of(&s)?) else { return err("a copy's addresses at a lane") };
                let addr = |a: String, m: M| if m == M::PTR { a } else { format!("({a} as usize as *const u8)") };
                let (dj, sj) = (addr(dj, djm), addr(sj, sjm));
                let guard = if cx.full {
                    String::new()
                } else {
                    let k = self.mask(&V::K(cx.cur.clone()))?;
                    let km = self.tmp();
                    self.line(format!("let {km} = core::mem::transmute::<[uint32x4_t; {}], [u32; {w}]>({k});", self.p));
                    format!("if {km}[j] != 0 ")
                };
                self.line(format!("for j in 0..{w} {{ {guard}{{ core::ptr::copy({sj}, {dj} as *mut u8, {bytes}); }} }}"));
                return Ok(d);
            }
            k => return err(format!("a {k:?} value")),
        })
    }

    /// Whether variable node `dst`'s variable (its every node of the same number and instance) is, but by copy `copy`
    /// into it, only read: each other node of it is the address (plus offsets) of a load, or a copy's source.
    fn only_read(&self, dst: usize, copy: usize) -> bool {
        let var = self.v.w(dst, 3);
        (0..self.v.len()).filter(|&i| i != dst && self.v.kind(i) == Kind::Local && self.v.w(i, 3) == var).all(|i| {
            let mut top = i;
            let mut q = self.parent(top);
            while let Some(p) = q
                && matches!(self.v.kind(p), Kind::Bin | Kind::Cast)
            {
                top = p;
                q = self.parent(p);
            }
            match q {
                Some(p) if self.v.kind(p) == Kind::Load => self.v.child(p, 0) == top,
                Some(p) if self.v.kind(p) == Kind::Copy => p != copy && self.v.child(p, 1) == top,
                // An argument: an aggregate is passed as its address (a write through it is the source buffer's, which
                // the pass finds).
                Some(p) if self.v.kind(p) == Kind::Call => true,
                _ => false,
            }
        })
    }

    /// Whether the last pass wrote no byte of the kernel's buffer `k` (and no write of a buffer it did not know).
    fn unwritten(&self, k: u32) -> bool {
        self.written.as_ref().is_some_and(|w| !w.contains(&k))
    }

    /// The lanes at which the vector side of a rare call costs more than its one-value instance once a lane.
    const FEW: u32 = 2;

    /// An arm of a `?:` of each lane's condition (`cx` its lanes), node `arm`. A call of a function that reads no
    /// lane's id or SIMD group (`lane_free`): where few of the chunk's lanes take it (`FEW`, as the program finds
    /// them), its instance of one value, once a lane at its arguments (a rare case: a side list's search the
    /// chunk's other lanes do not take), else the lanes' instance.
    fn arm(&mut self, arm: usize, cx: &Cx) -> R<V> {
        let id = self.v.w(arm, 3);
        let eligible = self.v.kind(arm) == Kind::Call
            && self.bodies.get(&id).is_some_and(|&b| {
                let r = M(self.v.w(b, 4));
                r != M::VOID && !r.is_agg() && !r.float() && (r.bytes() == 4 || wide(r) || r == M::B)
            })
            && self.lane_free(id, &mut Vec::new());
        if !eligible {
            return self.expr(arm, cx);
        }
        let b = self.bodies[&id];
        let (np, m) = (self.v.w(b, 6) as usize, M(self.v.w(b, 4)));
        let kids: Vec<usize> = self.v.children(arm).collect();
        let mut vals = Vec::new();
        for &c in kids.iter().take(np) {
            vals.push(self.expr(c, cx)?);
        }
        let (w, p) = (self.w, self.p);
        let n = if wide(m) { 2 * p } else { p };
        let (t, bits) = (self.tmp(), self.tmp());
        self.line(format!("let {t}: [uint32x4_t; {n}];"));
        self.line(format!("let {bits} = bits(&{});", cx.cur));
        self.line(format!("if {bits}.count_ones() <= {} {{", Self::FEW));
        self.ind += 1;
        let mut one = Vec::new();
        for x in &vals {
            one.push(self.lane_of(x)?);
        }
        let (ra, left) = (self.tmp(), self.tmp());
        self.line(format!("let mut {ra} = [0u32; {}];", n * 4));
        self.line(format!("let mut {left} = {bits};"));
        self.line(format!("while {left} != 0 {{"));
        self.ind += 1;
        self.line(format!("let j = {left}.trailing_zeros() as usize;"));
        self.line(format!("{left} &= {left} - 1;"));
        let r = self.call(id, one, &Cx { full: true, ..cx.clone() })?;
        // Its value: one, or (an instance whose result is each lane's) its first lane's, every lane's the same.
        let (lo, hi) = match &r {
            V::S(s, sm) if m == M::B => (format!("if {} {{ u32::MAX }} else {{ 0 }}", truth(s, *sm)), None),
            V::S(s, sm) if wide(m) => {
                let v = self.tmp();
                self.line(format!("let {v}: u64 = {};", conv(s, *sm, M::U64)));
                (format!("{v} as u32"), Some(format!("({v} >> 32) as u32")))
            }
            V::S(s, sm) => (conv(s, *sm, M::U32), None),
            V::K(k) => (format!("vgetq_lane_u32::<0>({k}[0])"), None),
            V::L(l, _) if wide(m) => (format!("vgetq_lane_u32::<0>({l}[0])"), Some(format!("vgetq_lane_u32::<0>({l}[{p}])"))),
            V::L(l, _) => (format!("vgetq_lane_u32::<0>({l}[0])"), None),
            x => return err(format!("a rare call's value {x:?}")),
        };
        self.line(format!("{ra}[j] = {lo};"));
        if let Some(hi) = hi {
            self.line(format!("{ra}[{w} + j] = {hi};"));
        }
        self.ind -= 1;
        self.line("}");
        self.line(format!("{t} = core::mem::transmute::<[u32; {}], [uint32x4_t; {n}]>({ra});", n * 4));
        self.ind -= 1;
        self.line("} else {");
        self.ind += 1;
        let r = self.call(id, vals, cx)?;
        let l = if m == M::B { self.mask(&r)? } else { self.lanes(&r, m)? };
        self.line(format!("{t} = {l};"));
        self.ind -= 1;
        self.line("}");
        Ok(if m == M::B { V::K(t) } else { V::L(t, m) })
    }

    /// Whether function `id` (and every function it calls) reads no lane's id, no SIMD group's lanes and meets no
    /// barrier: a lane's call of its instance of one value is that lane's answer.
    fn lane_free(&self, id: u32, seen: &mut Vec<u32>) -> bool {
        if seen.contains(&id) {
            return true;
        }
        seen.push(id);
        let Some(&b) = self.bodies.get(&id) else { return false };
        if self.phases(id).is_some() {
            return false;
        }
        (b..b + self.v.size(b)).all(|i| match self.v.kind(i) {
            Kind::Builtin => !matches!(
                Builtin::of(self.v.w(i, 0)),
                Builtin::LocalId
                    | Builtin::GlobalId
                    | Builtin::SubGroupLocalId
                    | Builtin::Barrier
                    | Builtin::Yield
                    | Builtin::SubGroupBarrier
                    | Builtin::SubGroupBallot
                    | Builtin::SubGroupShuffle
                    | Builtin::SubGroupShuffleUp
                    | Builtin::SubGroupShuffleDown
                    | Builtin::SubGroupShuffleXor
                    | Builtin::SubGroupReduceAdd
                    | Builtin::SubGroupReduceMin
                    | Builtin::SubGroupReduceMax
            ),
            Kind::Call => self.lane_free(self.v.w(i, 3), seen),
            _ => true,
        })
    }

    /// Value `x` (of each lane) at lane `j` (a `usize` the code names `j`): one value, of lanes read from an array
    /// bound here (before the loop over the lanes).
    fn lane_of(&mut self, x: &V) -> R<V> {
        let w = self.w;
        Ok(match x {
            V::S(..) | V::None => x.clone(),
            V::Ref(..) | V::Arr(..) => return err("a thread variable's address at one lane"),
            V::Iota(f, _) => V::S(format!("({f}).wrapping_add(j as u32)"), M::U32),
            V::Lin(f, s) => V::S(format!("({f}).wrapping_add(({s}).wrapping_mul(j as u32))"), M::U32),
            V::At { base, idx, scale } => {
                let i = self.lane_of(idx)?;
                let V::S(i, im) = i else { return err("a lane's index") };
                let off = format!("(({}) as usize).wrapping_mul({scale})", conv(&i, im, M::U32));
                if base.is_empty() { V::S(format!("({off} as u64)"), M::U64) } else { V::S(format!("({base} as *const u8).wrapping_add({off})"), M::PTR) }
            }
            V::K(..) | V::Cmp { .. } => {
                let k = self.mask(x)?;
                let a = self.tmp();
                self.line(format!("let {a} = core::mem::transmute::<[uint32x4_t; {}], [u32; {w}]>({k});", self.p));
                V::S(format!("({a}[j] != 0)"), M::B)
            }
            V::L(l, m) if wide(*m) => {
                let a = self.tmp();
                self.line(format!("let {a} = core::mem::transmute::<[uint32x4_t; {}], [u32; {}]>({l});", 2 * self.p, 2 * w));
                let x = format!("(({a}[{w} + j] as u64) << 32 | {a}[j] as u64)");
                // A pointer's halves: its address.
                V::S(if *m == M::PTR { format!("({x} as usize as *const u8)") } else if *m == M::U64 { x } else { format!("({x} as {})", scalar(*m)?) }, *m)
            }
            V::Z(l) => {
                let a = self.tmp();
                self.line(format!("let {a} = core::mem::transmute::<[uint32x4_t; {}], [u32; {w}]>({l});", self.p));
                V::S(format!("({a}[j] as u64)"), M::U64)
            }
            V::L(l, m) => {
                let a = self.tmp();
                self.line(format!("let {a} = core::mem::transmute::<[uint32x4_t; {}], [u32; {w}]>({l});", self.p));
                if m.float() { V::S(format!("f32::from_bits({a}[j])"), M::F32) } else if m.signed() { V::S(format!("({a}[j] as i32)"), M::I32) } else { V::S(format!("{a}[j]"), M::U32) }
            }
        })
    }

    /// A call of function `id` with its arguments' values `vals`: the instance of their shapes, its result.
    fn call(&mut self, id: u32, vals: Vec<V>, cx: &Cx) -> R<V> {
        let Some(&b) = self.bodies.get(&id) else { return err(format!("a call of function {id}, which has no body")) };
        if let Some(r) = self.compiled.get(&self.v.w(b, 1)).cloned() {
            return self.call_compiled(&r, b, vals, cx);
        }
        let ret = M(self.v.w(b, 4));
        // Where all lanes run, the instance of all lanes (no mask); else the mask's.
        let mut args = vec!["l".to_string(), "lid0".to_string()];
        if !cx.full {
            args.push(cx.cur.clone());
        }
        // A function with barriers: the kernel's phase of it, and the chunk's state.
        let phased = self.phases(id).is_some();
        // From a function with phases, its statement's phase of the callee's (`Part::Sub`); else the kernel's.
        let sub = if phased { self.sub.take() } else { None };
        if phased && !cx.kernel && sub.is_none() {
            return err(format!("a barrier in function {id}, called where its caller's phases do not part it (in a branch or a loop)"));
        }
        // The instance of its arguments' shapes (an aggregate's, its address: one, or each lane's, `Sh::At`).
        let mut shapes: Vec<Sh> = vals.iter().enumerate().map(|(k, a)| if M(self.v.w(b, 8 + 2 * k)).is_agg() && !matches!(a, V::At { .. }) { Sh::S } else { Sh::of(a) }).collect();
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
        // A parameter of consecutive lanes that would take a chunk crossing no `2^K` (`Em::wants`): lanes
        // known so are passed so (`Sh::Run`); a parameter's that are not, its own callers are asked to test;
        // others this call tests, and calls the instance that knows where they pass (one test of a chunk).
        let mut test = None;
        for k in 0..shapes.len() {
            let (Sh::Iota, Some(&want), V::Iota(f, _)) = (shapes[k], self.wants.get(&(id, k)), &vals[k]) else { continue };
            if let Some(&b) = self.runs.get(f) {
                if b <= want {
                    shapes[k] = Sh::Run(b);
                }
            } else if let Some(&p) = self.iota_params.get(f) {
                self.want(p, want);
            } else if test.is_none() {
                test = Some((k, f.clone(), want));
            }
        }
        let run = test.as_ref().map(|(k, f, want)| {
            let mut s = shapes.clone();
            s[*k] = Sh::Run(*want);
            let ctx = self.inst(id, &s, cx.full, &valid);
            self.seen.insert((id, s.clone(), cx.full, valid.clone()));
            (s, ctx, f.clone(), *want)
        });
        self.seen.insert((id, shapes.clone(), cx.full, valid.clone()));
        let ctx = self.inst(id, &shapes, cx.full, &valid);
        // Both instances' results of one shape: each lane's, if either's is.
        if let Some((_, rc, ..)) = &run
            && self.rets.get(rc).copied().unwrap_or(false) != self.rets.get(&ctx).copied().unwrap_or(false)
        {
            self.rets.insert(*rc, true);
            self.rets.insert(ctx, true);
            self.again = true;
        }
        if let Some((_, rc, ..)) = &run
            && self.risky_insts.contains(rc)
        {
            self.risky = true;
        }
        if self.risky_insts.contains(&ctx) {
            self.risky = true;
        }
        // Its first statement's access, through a parameter of every lane's own element, where all lanes run.
        let first: Vec<u32> = (0..shapes.len()).filter(|&k| matches!(shapes[k], Sh::At(_, Ix::Gid, Some(_))) && self.first_access(b, k)).filter_map(|k| shapes[k].root()).collect();
        for (k, a) in vals.into_iter().enumerate() {
            let pm = M(self.v.w(b, 8 + 2 * k));
            match (shapes[k], a) {
                (_, V::S(s, _)) if pm.is_agg() => args.push(s),
                (sh, _) if pm.is_agg() && !matches!(sh, Sh::At(..)) => return err("an aggregate argument of each lane, not at an address of each"),
                (Sh::S, V::S(s, am)) => args.push(conv(&s, am, pm)),
                (Sh::Arg(_), V::S(s, _)) => args.push(s),
                (Sh::In(_), V::S(s, am)) => args.push(conv(&s, am, pm)),
                (Sh::Gid, _) => {}
                (Sh::Ref(_), V::Ref(place, _)) | (Sh::Arr(..), V::Arr(place, ..)) => args.push(format!("&mut {place}")),
                (Sh::Iota | Sh::Run(_), V::Iota(f, _)) => args.push(f),
                (Sh::L, a) => {
                    let l = if pm == M::B { self.mask(&a)? } else { self.lanes(&a, pm)? };
                    args.push(l);
                }
                (Sh::Lin, V::Lin(f, s)) => {
                    args.push(f);
                    args.push(s);
                }
                (Sh::At(_, ix, _), V::At { base, idx, .. }) => {
                    // An offset of no base: one of a null pointer, the same bits.
                    args.push(if base.is_empty() { "core::ptr::null::<u8>()".into() } else { base });
                    match (ix, *idx) {
                        (Ix::Gid, _) => {}
                        (_, V::Iota(f, _)) => args.push(f),
                        (_, V::Lin(f, s)) => {
                            args.push(f);
                            args.push(s);
                        }
                        (_, i) => args.push(self.lanes(&i, M::U32)?),
                    }
                }
                (sh, a) => return err(format!("{a:?} passed as {sh:?}")),
            }
        }
        // A variable passed by `&mut`: the other arguments first, as values (one may read that variable).
        if args.iter().any(|a| a.starts_with("&mut ")) {
            for a in args.iter_mut().skip(2) {
                if !a.starts_with("&mut ") && a.chars().any(|c| !c.is_alphanumeric() && c != '_') {
                    let t = self.tmp();
                    self.line(format!("let {t} = {a};"));
                    *a = t;
                }
            }
        }
        if phased {
            if let Some((k, at)) = sub {
                args.push(format!("{k}u32"));
                args.push(format!("st.add({at})"));
            } else {
                args.push("phase".into());
                args.push(format!("st.add((lid0 / {}) as usize * {})", self.w, self.state_bytes(id)));
            }
        }
        let call = match &run {
            // The chunk's first lane's bits below `K` leave room for its others: it crosses no `2^K`.
            Some((s, _, f, k)) => {
                let m = if *k >= 32 { u32::MAX } else { (1u32 << k) - 1 };
                let a = args.join(", ");
                format!("if ({f} & {m}u32) <= {}u32 {{ {}({a}) }} else {{ {}({a}) }}", m - (self.w - 1), fname(id, s, cx.full, &valid), fname(id, &shapes, cx.full, &valid))
            }
            None => format!("{}({})", fname(id, &shapes, cx.full, &valid), args.join(", ")),
        };
        let r = if ret == M::VOID {
            self.line(format!("{call};"));
            V::None
        } else {
            let t = self.tmp();
            self.line(format!("let {t} = {call};"));
            let rl = self.rets.get(&ctx).copied().unwrap_or(false);
            if ret.is_agg() {
                // Its bytes, here: their address, one value's or each lane's at its row.
                if rl { V::At { base: format!("({t}.as_ptr() as *const u8)"), idx: Box::new(V::Iota("0u32".into(), M::U32)), scale: ret.bytes() } } else { V::S(format!("{t}.as_ptr()"), M::PTR) }
            } else if rl {
                if ret == M::B { V::K(t) } else { V::L(t, ret) }
            } else {
                V::S(t, ret)
            }
        };
        if cx.full {
            for k in first {
                if !self.is_valid(k) {
                    let d = self.ind;
                    self.valid.push((k, d));
                }
            }
        }
        Ok(r)
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
            // Of lanes by the tape, but one value where both arms are (as the last pass found them).
            let varying = lanes;
            let lanes = lanes && self.lane_cond(cx.ctx, n);
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
                        _ if varying => {
                            self.learn_lane_cond(cx.ctx, n);
                            "0".into()
                        }
                        // One value in the lanes that run, held as each lane's: the first running lane's.
                        x => self.one_of(&x, m, cx)?,
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
            let inner = Cx { cur: ka.clone(), divergent: true, left: false, full: false, looped: None, region: None, ..cx.clone() };
            let x = self.arm(arm, &inner)?;
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
        // A structure's field.
        let (l, byte) = if self.v.kind(a) == Kind::Local { (a, self.v.w(a, 2)) } else { self.local_field(a)? };
        if self.structs.contains_key(&(cx.ctx, self.vid(l))) {
            return self.vars.get(&(cx.ctx, field_key(self.vid(l), byte))).cloned();
        }
        if self.v.kind(a) != Kind::Local || self.v.w(a, 2) != 0 {
            return None;
        }
        if self.aliases.contains(&(cx.ctx, self.vid(a))) || self.mems.contains_key(&(cx.ctx, self.vid(a))) {
            return None;
        }
        let var = self.vars.get(&(cx.ctx, self.vid(a)))?.clone();
        (!self.agg_param(cx, self.vid(a))).then_some(var)
    }

    /// Whether address `a` is an aggregate parameter's plus constants, and the function writes none of its bytes.
    fn of_value(&self, a: usize, cx: &Cx) -> bool {
        let mut agg = None;
        for i in a..a + self.v.size(a) {
            match self.v.kind(i) {
                Kind::Const | Kind::Cast | Kind::Bin => {}
                Kind::Local if self.agg_param(cx, self.vid(i)) => agg = Some(self.vid(i)),
                _ => return false,
            }
        }
        let Some(off) = agg else { return false };
        let f = if cx.kernel { 0 } else { self.bodies[&cx.body] };
        // Its writes: a store or an update whose address holds it, in the function (not its inner functions').
        !(f..f + self.v.size(f)).any(|i| {
            matches!(self.v.kind(i), Kind::Store | Kind::Update) && {
                let t = self.v.child(i, 0);
                (t..t + self.v.size(t)).any(|j| self.v.kind(j) == Kind::Local && self.vid(j) == off)
            } && (cx.kernel || !self.bodies.values().any(|&b| b != f && b > f && b < i && i < b + self.v.size(b)))
        })
    }

    /// Whether variable `off` (a number, `vid`) of `cx`'s function is an aggregate parameter (its value its address):
    /// parameter `off`, a function's parameters its first variables.
    fn agg_param(&self, cx: &Cx, off: u32) -> bool {
        let n = if cx.kernel { 0 } else { self.bodies[&cx.body] };
        let (at, k0) = if cx.kernel { (4, 5) } else { (6, 7) };
        let np = self.v.w(n, at) as usize;
        (off as usize) < np && M(self.v.w(n, k0 + 1 + 2 * off as usize)).is_agg()
    }

    fn assign(&mut self, var: &Var, x: V, cx: &Cx) -> R<()> {
        // A thread variable's address: the pointer an alias of that variable, if it is only ever that one's.
        if let V::Ref(place, _) = &x {
            let key = (cx.ctx, var.name[1..].parse::<u32>().unwrap_or(u32::MAX));
            return match (self.refs.get(&key), &var.fixed) {
                (Some(V::Ref(p, _)), _) if p == place => Ok(()),
                (Some(_), _) | (None, Some(_)) => err(format!("{} holds another variable's address", var.name)),
                (None, None) => {
                    self.refs.insert(key, x.clone());
                    self.again = true;
                    Ok(())
                }
            };
        }
        if var.fixed.is_some() {
            return err(format!("a write of parameter {}, passed as its calls' shape", var.name));
        }
        let key = (cx.ctx, self.vars.iter().find(|(k, v)| k.0 == cx.ctx && v.name == var.name).map(|(k, _)| k.1).unwrap_or(0));
        let off = var.name[1..].parse::<u32>().unwrap_or(u32::MAX);
        if !var.lanes {
            match x {
                V::S(s, xm) if (!cx.divergent || self.dead_off(cx, off) || self.read_uniform(cx, off)) && !var.iota => {
                    self.line(format!("{} = {};", var.name, conv(&s, xm, var.m)));
                    return Ok(());
                }
                // Consecutive lanes where all of them run, or where those that do not never read it again: their
                // first, the variable's (`Var::iota`); found so, a variable of one value is one of them from the next
                // pass.
                V::Iota(f, _) if (!cx.divergent || self.dead_off(cx, off)) && var.m.bytes() == 4 && !self.shadows.contains(&var.name) => {
                    if var.iota {
                        self.line(format!("{} = {f};", var.name));
                    } else {
                        self.iotas.insert(key);
                        self.again = true;
                    }
                    return Ok(());
                }
                _ => {
                    // A lane's copy in a loop's scalar rounds (`finish_scalar`): those rounds are not one lane's.
                    if self.shadows.contains(&var.name) {
                        self.shadow_fail = true;
                        return Ok(());
                    }
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
        // Its lanes' addresses, a pointer's: made here, once for the reads of it in this block (`addrs`).
        let key = format!("addr {}", var.name);
        self.memo.retain(|m| m.0 != key);
        if var.m == M::PTR {
            self.addrs(&V::L(var.name.clone(), var.m), cx)?;
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
            V::At { base: b2, idx, scale } => Some(V::At { base: format!("({base}).wrapping_add(({b2}) as usize)"), idx, scale }),
            o @ (V::L(..) | V::Z(..)) if matches!(&o, V::Z(..)) || matches!(&o, V::L(_, lm) if wide(*lm)) => {
                let o = if cx.full {
                    o
                } else {
                    // The mask as a 64-bit value: all ones in both halves of a running lane.
                    let (cur, t, p) = (cx.cur.clone(), self.tmp(), self.p);
                    self.line(format!("let {t}: [uint32x4_t; {}] = [{}];", 2 * p, (0..2 * p).map(|r| format!("{cur}[{}]", r % p)).collect::<Vec<_>>().join(", ")));
                    self.bin64(Op::And, M::U64, &o, &V::L(t, M::U64))?
                };
                Some(self.bin64(Op::Add, M::U64, &V::S(format!("(({base}) as usize as u64)"), M::U64), &o)?)
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
                // An offset of each lane plus one value: the value its base (an address's, once a pointer is added).
                (Op::Add, V::S(y, ym), V::At { base, idx, scale }) | (Op::Add, V::At { base, idx, scale }, V::S(y, ym)) if m == M::U64 && *ym != M::PTR => {
                    let y = conv(y, *ym, M::U64);
                    let base = if base.is_empty() { format!("(({y}) as usize as *const u8)") } else { format!("({base}).wrapping_add(({y}) as usize)") };
                    return Ok(V::At { base, idx: idx.clone(), scale: *scale });
                }
                _ => return self.bin64(op, m, a, b),
            }
        }
        match (a, b) {
            // Two constants of 32 bits: the constant (so a mask or a count written `1024 - 1` is one, `konst`'s).
            (V::S(x, _), V::S(y, _)) if m == M::U32 && fold32(op, x, y).is_some() => {
                return Ok(V::S(format!("{}u32", fold32(op, x, y).unwrap_or(0)), m));
            }
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
            (V::Iota(f, fm), V::S(y, ym)) if op == Op::Sub => {
                return Ok(V::Iota(format!("{f}.wrapping_sub({})", conv(y, *ym, *fm)), m));
            }
            // Consecutive lanes times one value, and such lanes plus, minus or times one value or shifted up by a
            // constant: lanes `first + j * stride` still (each wrapping as the lanes' would).
            (V::Iota(f, _), V::S(y, ym)) | (V::S(y, ym), V::Iota(f, _)) if op == Op::Mul && m == M::U32 => {
                let y = conv(y, *ym, m);
                return Ok(V::Lin(format!("{f}.wrapping_mul({y})"), y));
            }
            (V::Lin(f, s), V::S(y, ym)) | (V::S(y, ym), V::Lin(f, s)) if matches!(op, Op::Add | Op::Mul) && m == M::U32 => {
                let y = conv(y, *ym, m);
                return Ok(if op == Op::Add { V::Lin(format!("{f}.wrapping_add({y})"), s.clone()) } else { V::Lin(format!("{f}.wrapping_mul({y})"), format!("{s}.wrapping_mul({y})")) });
            }
            (V::Lin(f, s), V::S(y, ym)) if op == Op::Sub && m == M::U32 => {
                return Ok(V::Lin(format!("{f}.wrapping_sub({})", conv(y, *ym, m)), s.clone()));
            }
            (V::Lin(f, s), V::S(y, _)) if op == Op::Shl && m == M::U32 && konst(y).is_some_and(|c| c < 32) => {
                return Ok(V::Lin(format!("{f}.wrapping_shl({y})"), format!("{s}.wrapping_shl({y})")));
            }
            // Consecutive lanes shifted down by `c`, or masked to their bits below `c`: where their chunk crosses no
            // `2^c` (`Sh::Run`), one value, or consecutive lanes; of a parameter that does not know, its callers are
            // asked to test their chunk (`Em::wants`).
            (V::Iota(f, _), V::S(y, _)) | (V::S(y, _), V::Iota(f, _)) if (op == Op::Shr && matches!(a, V::Iota(..)) || op == Op::And) && m == M::U32 => {
                if let Some(c) = konst(y)
                    && let Some(k) = if op == Op::Shr { Some(c) } else { (c != u32::MAX && (c + 1).is_power_of_two()).then(|| (c + 1).trailing_zeros()) }
                    && k < 32
                    && (1u32 << k) >= self.w
                {
                    if let Some(&b) = self.runs.get(f)
                        && b <= k
                    {
                        return Ok(if op == Op::Shr { V::S(format!("({f} >> {c})"), m) } else { V::Iota(format!("({f} & {c}u32)"), m) });
                    }
                    if let Some(&p) = self.iota_params.get(f) {
                        self.want(p, k);
                    }
                }
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

    /// `x` as halves whose expressions are each a register of a name (an operand read more than once is computed
    /// once).
    fn named(&mut self, x: &V, m: M) -> R<H> {
        let h = self.halves(x, m)?;
        let simple = |v: &Vec<String>| v.iter().all(|e| e.ends_with(']') && !e.contains('('));
        if simple(&h.lo) && h.hi.as_ref().is_none_or(simple) {
            return Ok(h);
        }
        let v = self.of_halves(h, m);
        self.halves(&v, m)
    }

    /// `a op b` of each lane's 64-bit `m`, on the lanes' halves: an operation of 32 bits on each, a carry or a borrow
    /// between them, and none on a high half that is 0.
    fn bin64(&mut self, op: Op, m: M, a: &V, b: &V) -> R<V> {
        let (s, p) = (m.signed(), self.p as usize);
        if matches!(op, Op::Shl | Op::Shr) {
            let x = self.named(a, m)?;
            return self.shift64(op, s, m, x, b);
        }
        let (x, y) = (self.named(a, m)?, self.named(b, m)?);
        let zero = || "vdupq_n_u32(0)".to_string();
        let hi = |h: &H, r: usize| h.hi.as_ref().map_or_else(zero, |v| v[r].clone());
        let each = |f: &dyn Fn(usize) -> String| (0..p).map(f).collect::<Vec<_>>();
        let (lo, hi) = match op {
            Op::And | Op::Or | Op::Xor => {
                let f = match op {
                    Op::And => "vandq_u32",
                    Op::Or => "vorrq_u32",
                    _ => "veorq_u32",
                };
                let lo = each(&|r| format!("{f}({}, {})", x.lo[r], y.lo[r]));
                let hi = match (&x.hi, &y.hi) {
                    (None, None) => None,
                    (None, Some(_)) | (Some(_), None) if op == Op::And => None,
                    (Some(h), None) | (None, Some(h)) => Some(h.clone()),
                    (Some(a), Some(b)) => Some(each(&|r| format!("{f}({}, {})", a[r], b[r]))),
                };
                (lo, hi)
            }
            // A carry out of the low halves: their sum below an addend (all ones, -1, where it is).
            Op::Add => {
                let l = self.map(|r| format!("vaddq_u32({}, {})", x.lo[r as usize], y.lo[r as usize]));
                let c = self.map(|r| format!("vcltq_u32({l}[{r}], {})", x.lo[r as usize]));
                let hi = match (&x.hi, &y.hi) {
                    (None, None) => each(&|r| format!("vshrq_n_u32::<31>({c}[{r}])")),
                    _ => each(&|r| format!("vsubq_u32(vaddq_u32({}, {}), {c}[{r}])", hi(&x, r), hi(&y, r))),
                };
                (each(&|r| format!("{l}[{r}]")), Some(hi))
            }
            // A borrow: the low half subtracted past the other (all ones, -1, where it is).
            Op::Sub => {
                let c = self.map(|r| format!("vcltq_u32({}, {})", x.lo[r as usize], y.lo[r as usize]));
                (each(&|r| format!("vsubq_u32({}, {})", x.lo[r], y.lo[r])), Some(each(&|r| format!("vaddq_u32(vsubq_u32({}, {}), {c}[{r}])", hi(&x, r), hi(&y, r)))))
            }
            // The low 64 bits of the product: the low halves' whole product, and each high half times the other's low.
            Op::Mul => {
                let h = self.mulhi32(&x.lo, &y.lo);
                let mut hi = each(&|r| format!("{h}[{r}]"));
                for (a, b) in [(&x.lo, &y.hi), (&y.lo, &x.hi)] {
                    if let Some(b) = b {
                        hi = each(&|r| format!("vmlaq_u32({}, {}, {})", hi[r], a[r], b[r]));
                    }
                }
                (each(&|r| format!("vmulq_u32({}, {})", x.lo[r], y.lo[r])), Some(hi))
            }
            // NEON divides no integers: each lane's own (a divisor of 0 gives 0).
            Op::Div | Op::Rem => {
                let (st, f, w) = (if s { "i64" } else { "u64" }, if op == Op::Div { "checked_div" } else { "checked_rem" }, self.w);
                let t = self.tmp();
                self.line(format!("let (mut {t}xl, mut {t}xh, mut {t}yl, mut {t}yh) = ([0u32; {w}], [0u32; {w}], [0u32; {w}], [0u32; {w}]);"));
                for r in 0..p {
                    let o = 4 * r;
                    self.line(format!(
                        "vst1q_u32({t}xl.as_mut_ptr().add({o}), {}); vst1q_u32({t}xh.as_mut_ptr().add({o}), {}); vst1q_u32({t}yl.as_mut_ptr().add({o}), {}); vst1q_u32({t}yh.as_mut_ptr().add({o}), {});",
                        x.lo[r],
                        hi(&x, r),
                        y.lo[r],
                        hi(&y, r)
                    ));
                }
                self.line(format!("for j in 0..{w} {{ let q = ((({t}xh[j] as u64) << 32 | {t}xl[j] as u64) as {st}).{f}((({t}yh[j] as u64) << 32 | {t}yl[j] as u64) as {st}).unwrap_or(0) as u64; {t}xl[j] = q as u32; {t}xh[j] = (q >> 32) as u32; }}"));
                (each(&|r| format!("vld1q_u32({t}xl.as_ptr().add({}))", 4 * r)), Some(each(&|r| format!("vld1q_u32({t}xh.as_ptr().add({}))", 4 * r))))
            }
            Op::Eq | Op::Ne => {
                let k = self.map(|r| {
                    let r = r as usize;
                    let lo = format!("vceqq_u32({}, {})", x.lo[r], y.lo[r]);
                    if x.hi.is_none() && y.hi.is_none() { lo } else { format!("vandq_u32({lo}, vceqq_u32({}, {}))", hi(&x, r), hi(&y, r)) }
                });
                return Ok(V::K(if op == Op::Ne { self.map(|r| format!("vmvnq_u32({k}[{r}])")) } else { k }));
            }
            // An order: the high halves' (signed, of a signed type), or where they are equal the low halves'.
            Op::Lt | Op::Le | Op::Gt | Op::Ge => {
                let (a, b, not) = match op {
                    Op::Lt => (&x, &y, false),
                    Op::Gt => (&y, &x, false),
                    Op::Ge => (&x, &y, true),
                    _ => (&y, &x, true),
                };
                let k = self.map(|r| {
                    let r = r as usize;
                    let lo = format!("vcltq_u32({}, {})", a.lo[r], b.lo[r]);
                    if a.hi.is_none() && b.hi.is_none() {
                        return lo;
                    }
                    let (ha, hb) = (hi(a, r), hi(b, r));
                    let lt = if s { format!("vcltq_s32(vreinterpretq_s32_u32({ha}), vreinterpretq_s32_u32({hb}))") } else { format!("vcltq_u32({ha}, {hb})") };
                    format!("vorrq_u32({lt}, vandq_u32(vceqq_u32({ha}, {hb}), {lo}))")
                });
                return Ok(V::K(if not { self.map(|r| format!("vmvnq_u32({k}[{r}])")) } else { k }));
            }
            op => return err(format!("{op:?} of 64-bit lanes")),
        };
        Ok(self.of_halves(H { lo, hi }, m))
    }

    /// `x << c` or `x >> c` of each lane's 64-bit `m` (arithmetic, `s`, of a signed one) on its halves, `c` (C's,
    /// below 64) a constant, one value, or each lane's: a constant's shifts by their counts, else NEON's shift by
    /// each lane's signed count, whose count past a half's width gives 0 (or the sign).
    fn shift64(&mut self, op: Op, s: bool, m: M, x: H, b: &V) -> R<V> {
        let p = self.p as usize;
        let each = |f: &dyn Fn(usize) -> String| (0..p).map(f).collect::<Vec<_>>();
        let zero = || "vdupq_n_u32(0)".to_string();
        let hi = |r: usize| x.hi.as_ref().map_or_else(zero, |v| v[r].clone());
        let sr = |e: &str, k: u32| format!("vreinterpretq_u32_s32(vshrq_n_s32::<{k}>(vreinterpretq_s32_u32({e})))");
        let (lo, h) = if let V::S(c, cm) = b
            && let Some(k) = konst64(&conv(c, *cm, M::U64))
        {
            let k = (k & 63) as u32;
            match (op, s) {
                _ if k == 0 => (x.lo.clone(), x.hi.clone()),
                (Op::Shl, _) if k < 32 => (
                    each(&|r| format!("vshlq_n_u32::<{k}>({})", x.lo[r])),
                    Some(each(&|r| if x.hi.is_some() { format!("vorrq_u32(vshlq_n_u32::<{k}>({}), vshrq_n_u32::<{}>({}))", hi(r), 32 - k, x.lo[r]) } else { format!("vshrq_n_u32::<{}>({})", 32 - k, x.lo[r]) })),
                ),
                (Op::Shl, _) => (each(&|_| zero()), Some(each(&|r| if k == 32 { x.lo[r].clone() } else { format!("vshlq_n_u32::<{}>({})", k - 32, x.lo[r]) }))),
                (_, false) if k < 32 => (
                    each(&|r| if x.hi.is_some() { format!("vorrq_u32(vshrq_n_u32::<{k}>({}), vshlq_n_u32::<{}>({}))", x.lo[r], 32 - k, hi(r)) } else { format!("vshrq_n_u32::<{k}>({})", x.lo[r]) }),
                    x.hi.as_ref().map(|v| v.iter().map(|e| format!("vshrq_n_u32::<{k}>({e})")).collect()),
                ),
                (_, false) => (each(&|r| if k == 32 { hi(r) } else { format!("vshrq_n_u32::<{}>({})", k - 32, hi(r)) }), None),
                (_, true) if k < 32 => (each(&|r| format!("vorrq_u32(vshrq_n_u32::<{k}>({}), vshlq_n_u32::<{}>({}))", x.lo[r], 32 - k, hi(r))), Some(each(&|r| sr(&hi(r), k)))),
                (_, true) => (each(&|r| if k == 32 { hi(r) } else { sr(&hi(r), k - 32) }), Some(each(&|r| sr(&hi(r), 31)))),
            }
        } else {
            // Each lane's count as a signed 32-bit lane.
            let c = match b {
                V::S(c, cm) => {
                    let t = self.tmp();
                    self.line(format!("let {t} = vdupq_n_s32((({}) & 63) as i32);", conv(c, *cm, M::U32)));
                    (0..p).map(|_| t.clone()).collect::<Vec<_>>()
                }
                _ => {
                    let y = self.lanes(b, M::U32)?;
                    let t = self.map(|r| format!("vandq_u32({y}[{r}], vdupq_n_u32(63))"));
                    (0..p).map(|r| format!("vreinterpretq_s32_u32({t}[{r}])")).collect()
                }
            };
            let less = |r: usize, d: i32| format!("vsubq_s32({}, vdupq_n_s32({d}))", c[r]);
            let neg = |r: usize| format!("vnegq_s32({})", c[r]);
            let from = |r: usize, d: i32| format!("vsubq_s32(vdupq_n_s32({d}), {})", c[r]);
            match (op, s) {
                (Op::Shl, _) => (
                    each(&|r| format!("vshlq_u32({}, {})", x.lo[r], c[r])),
                    Some(each(&|r| {
                        let up = format!("vshlq_u32({}, {})", x.lo[r], less(r, 32));
                        if x.hi.is_some() { format!("vorrq_u32(vshlq_u32({}, {}), {up})", hi(r), c[r]) } else { up }
                    })),
                ),
                (_, false) => (
                    each(&|r| {
                        let down = format!("vshlq_u32({}, {})", x.lo[r], neg(r));
                        if x.hi.is_some() { format!("vorrq_u32({down}, vshlq_u32({}, {}))", hi(r), from(r, 32)) } else { down }
                    }),
                    x.hi.as_ref().map(|v| (0..p).map(|r| format!("vshlq_u32({}, {})", v[r], neg(r))).collect()),
                ),
                (_, true) => {
                    let big = self.map(|r| format!("vcgeq_s32({}, vdupq_n_s32(32))", c[r as usize]));
                    let si = |e: &str, k: &str| format!("vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32({e}), {k}))");
                    (
                        each(&|r| format!("vbslq_u32({big}[{r}], {}, vorrq_u32(vshlq_u32({}, {}), vshlq_u32({}, {})))", si(&hi(r), &from(r, 32)), x.lo[r], neg(r), hi(r), from(r, 32))),
                        Some(each(&|r| si(&hi(r), &neg(r)))),
                    )
                }
            }
        };
        Ok(self.of_halves(H { lo, hi: h }, m))
    }

    /// A division of lanes: each lane's own (NEON has none of integers); a lane whose divisor is 0 gets 0, as one
    /// that does not run may hold one.
    fn div(&mut self, op: Op, m: M, a: &V, b: &V) -> R<V> {
        // Unsigned by a power of two the program names: a shift, or a mask of the bits below it.
        if !m.signed()
            && let V::S(c, _) = b
            && let Some(k) = konst(c).filter(|k| k.is_power_of_two())
        {
            let x = self.lanes(a, m)?;
            return Ok(V::L(
                if op == Op::Div {
                    self.map(|r| format!("vshlq_u32({x}[{r}], vdupq_n_s32(-{}))", k.trailing_zeros()))
                } else {
                    self.map(|r| format!("vandq_u32({x}[{r}], vdupq_n_u32({}u32))", k - 1))
                },
                m,
            ));
        }
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

    /// 32-bit lanes `l` as `to`, of 4 bytes or fewer: a narrower type's bits, zero- or sign-extended in its lane.
    fn narrowed(&mut self, l: String, to: M) -> V {
        let b = to.bytes();
        if b >= 4 {
            return V::L(l, to);
        }
        let s = 32 - 8 * b;
        V::L(
            if to.signed() {
                self.map(|r| format!("vreinterpretq_u32_s32(vshrq_n_s32::<{s}>(vreinterpretq_s32_u32(vshlq_n_u32::<{s}>({l}[{r}]))))"))
            } else {
                self.map(|r| format!("vandq_u32({l}[{r}], vdupq_n_u32({}))", (1u64 << (8 * b)) - 1))
            },
            to,
        )
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
            V::K(k) if wide(to) => V::Z(self.lanes(&V::K(k), M::U32)?),
            // As a narrower integer: its 0 or 1, in 32-bit lanes (as a narrower one's are held).
            V::K(k) if matches!(to.bytes(), 1 | 2) && !to.float() => V::L(self.lanes(&V::K(k), M::U32)?, to),
            c @ V::Cmp { .. } if matches!(to.bytes(), 1 | 2) && !to.float() => V::L(self.lanes(&c, M::U32)?, to),
            V::K(_) => return err(format!("a mask as {to:?}")),
            // 64-bit lanes: as other 64 bits, the same; as 32 or fewer, each lane's low half; as a truth, whether
            // any bit of either half is set.
            V::L(l, _) if wide(from) && wide(to) => V::L(l, to),
            V::Z(l) if wide(to) => V::Z(l),
            V::L(l, _) if wide(from) && to == M::B => {
                let p = self.p;
                V::K(self.map(|r| format!("vtstq_u32(vorrq_u32({l}[{r}], {l}[{}]), vorrq_u32({l}[{r}], {l}[{}]))", p + r, p + r)))
            }
            V::Z(l) if to == M::B => V::K(self.map(|r| format!("vtstq_u32({l}[{r}], {l}[{r}])"))),
            V::L(l, _) if wide(from) && to.bytes() <= 4 => {
                let lo = self.map(|r| format!("{l}[{r}]"));
                self.narrowed(lo, to)
            }
            V::Z(l) if to.bytes() <= 4 => self.narrowed(l, to),
            V::At { .. } if wide(from) && wide(to) => a,
            V::At { .. } if to.bytes() <= 4 => {
                let h = self.halves(&a, M::U64)?;
                let lo = self.map(|r| h.lo[r as usize].clone());
                self.narrowed(lo, to)
            }
            V::L(..) if wide(to) && from.bytes() == 4 && from.signed() => {
                let h = self.halves(&a, from)?;
                self.of_halves(h, to)
            }
            // A narrower signed one's: held sign-extended in 32 bits, widened as an `i32` is.
            V::L(l, _) if wide(to) && matches!(from.bytes(), 1 | 2) && from.signed() => {
                let h = self.halves(&V::L(l, M::I32), M::I32)?;
                self.of_halves(h, to)
            }
            V::L(l, _) if to == M::B => V::K(self.map(|r| format!("vtstq_u32({l}[{r}], {l}[{r}])"))),
            V::L(l, _) if to.bytes() <= 4 && from.bytes() == 4 => self.narrowed(l, to),
            V::L(l, _) if to.bytes() == 4 && from.bytes() < 4 => V::L(l, to),
            V::Iota(f, _) if to.bytes() == 4 => V::Iota(f, to),
            V::Lin(f, s) if to.bytes() == 4 && to != M::B && !to.float() => V::Lin(f, s),
            // A lane's 32-bit index widened: an offset, scaled 1 so far.
            V::L(..) | V::Iota(..) | V::Lin(..) if to.bytes() == 8 && from.bytes() == 4 && !from.signed() => V::At { base: String::new(), idx: Box::new(a), scale: 1 },
            // A narrower unsigned one's: its lanes hold it zero-extended, a 32-bit lane's the same.
            V::L(l, _) if to.bytes() == 8 && from.bytes() < 4 && !from.signed() && from != M::B => V::At { base: String::new(), idx: Box::new(V::L(l, M::U32)), scale: 1 },
            // Otherwise, lanes `first + j * stride` as each lane's.
            V::Lin(..) => {
                let l = self.lanes(&a, M::U32)?;
                return self.cast(V::L(l, from), from, to);
            }
            a => return err(format!("{a:?} from {from:?} to {to:?}")),
        })
    }

    /// Value `x` of `m`, one value in the lanes of `cx` that run (the tape's level) but held as each lane's: the
    /// first running lane's, as a scalar expression.
    fn one_of(&mut self, x: &V, m: M, cx: &Cx) -> R<String> {
        if let V::S(s, xm) = x {
            return Ok(conv(s, *xm, m));
        }
        let first = if cx.full {
            "0usize".to_string()
        } else {
            let t = self.tmp();
            self.line(format!("let {t} = (bits(&{}).trailing_zeros() as usize).min({});", cx.cur, self.w - 1));
            t
        };
        let first = first.as_str();
        let V::S(e, em) = self.lane_of(x)? else { return err("a value at its first lane") };
        let t = self.tmp();
        self.line(format!("let {t} = {{ let j = {first}; {} }};", conv(&e, em, m)));
        Ok(t)
    }

    /// SIMD group operation `b` (of `m`) on `args`, the group the chunk's 32 lanes (`one`), in the lanes that run:
    /// a ballot their bits where the argument holds; a reduction theirs folded (one value); a shuffle each lane's
    /// read of its source lane's value, 0 where that lane does not run (its own, past the group's ends, for `up`
    /// and `down`), as the vector tape's interpreter has it.
    fn sg_op(&mut self, b: Builtin, m: M, args: &[V], cx: &Cx) -> R<V> {
        let w = self.w;
        let p = self.p;
        // The running lanes' bits.
        let on = if cx.full {
            format!("{}u32", if w == 32 { u32::MAX } else { (1u32 << w) - 1 })
        } else {
            let t = self.tmp();
            self.line(format!("let {t} = bits(&{});", cx.cur));
            t
        };
        if b == Builtin::SubGroupBallot {
            let k = self.mask(&args[0])?;
            let t = self.tmp();
            self.line(format!("let {t} = bits(&{k}) & {on};"));
            return Ok(V::S(t, M::U32));
        }
        let wide = wide(m);
        let (regs, n) = if wide { (2 * p, 2 * w) } else { (p, w) };
        // A value of one lane's: the same in every lane.
        if let V::S(x, xm) = &args[0]
            && matches!(b, Builtin::SubGroupReduceMin | Builtin::SubGroupReduceMax)
        {
            return Ok(V::S(conv(x, *xm, m), m));
        }
        let l = self.lanes(&args[0], m)?;
        let xa = self.tmp();
        self.line(format!("let {xa} = core::mem::transmute::<[uint32x4_t; {regs}], [u32; {n}]>({l});"));
        let lane = |j: &str| if wide { format!("(({xa}[{w} + {j}] as u64) << 32 | {xa}[{j}] as u64)") } else { format!("{xa}[{j}]") };
        let ty = if wide { "u64" } else { "u32" };
        match b {
            Builtin::SubGroupReduceAdd | Builtin::SubGroupReduceMin | Builtin::SubGroupReduceMax => {
                let st = if m.float() {
                    return err(format!("{b:?} of {m:?}"));
                } else if m.signed() {
                    if wide { "i64" } else { "i32" }
                } else {
                    ty
                };
                let (init, f) = match b {
                    Builtin::SubGroupReduceAdd => ("0".to_string(), "r.wrapping_add(x)"),
                    Builtin::SubGroupReduceMin => (format!("{st}::MAX"), "r.min(x)"),
                    _ => (format!("{st}::MIN"), "r.max(x)"),
                };
                let t = self.tmp();
                self.line(format!("let mut {t}: {st} = {init};"));
                self.line(format!("for j in 0..{w} {{ if ({on} >> j) & 1 != 0 {{ let (r, x) = ({t}, {} as {st}); {t} = {f}; }} }}", lane("j")));
                Ok(V::S(format!("({t} as {})", scalar(m)?), m))
            }
            _ => {
                let i = self.lanes(&args[1], M::U32)?;
                let ia = self.tmp();
                self.line(format!("let {ia} = core::mem::transmute::<[uint32x4_t; {p}], [u32; {w}]>({i});"));
                let (src, keep) = match b {
                    Builtin::SubGroupShuffle => (format!("Some({ia}[j] as usize & 31)"), false),
                    Builtin::SubGroupShuffleXor => (format!("Some((j ^ {ia}[j] as usize) & 31)"), false),
                    Builtin::SubGroupShuffleUp => (format!("(j as usize).checked_sub({ia}[j] as usize)"), true),
                    _ => (format!("Some(j + {ia}[j] as usize).filter(|&s| s < 32)"), true),
                };
                let out = self.tmp();
                self.line(format!("let mut {out} = [0u32; {n}];"));
                let other = if keep { lane("j") } else { "0".into() };
                let put = if wide { format!("{out}[j] = v as u32; {out}[{w} + j] = (v >> 32) as u32;") } else { format!("{out}[j] = v;") };
                self.line(format!(
                    "for j in 0..{w} {{ let v: {ty} = match {src} {{ Some(s) if ({on} >> s) & 1 != 0 => {}, Some(_) if !{keep} => 0, _ => {other} }}; {put} }}",
                    lane("s")
                ));
                let r = self.tmp();
                self.line(format!("let {r} = core::mem::transmute::<[u32; {n}], [uint32x4_t; {regs}]>({out});"));
                Ok(V::L(r, m))
            }
        }
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

    /// `x` as each lane's 64-bit `m`: its halves' `[uint32x4_t; 2P]`.
    fn lanes64(&mut self, x: &V, m: M) -> R<String> {
        if let V::L(l, lm) = x
            && wide(*lm)
        {
            return Ok(l.clone());
        }
        let h = self.halves(x, m)?;
        let hi = h.hi.unwrap_or_else(|| vec!["vdupq_n_u32(0)".into(); self.p as usize]);
        let t = self.tmp();
        self.line(format!("let {t}: [uint32x4_t; {}] = [{}, {}];", 2 * self.p, h.lo.join(", "), hi.join(", ")));
        Ok(t)
    }

    fn lanes_of(&mut self, x: &V, m: M) -> R<String> {
        Ok(match x {
            // 64-bit lanes as 32: each lane's low half.
            V::L(l, lm) if wide(*lm) => {
                let l = l.clone();
                self.map(|r| format!("{l}[{r}]"))
            }
            V::Z(l) => l.clone(),
            V::At { .. } => {
                let h = self.halves(x, M::U64)?;
                self.map(|r| h.lo[r as usize].clone())
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
            V::Lin(f, s) => {
                let (f, s) = (f.clone(), s.clone());
                self.map(|r| format!("vmlaq_u32(vdupq_n_u32({f}), vld1q_u32(LANE[{r}].as_ptr()), vdupq_n_u32({s}))"))
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
            // A 64-bit value's: whether either half has a bit set.
            V::L(l, lm) if wide(*lm) => {
                let (l, p) = (l.clone(), self.p);
                self.map(|r| format!("vtstq_u32(vorrq_u32({l}[{r}], {l}[{}]), vorrq_u32({l}[{r}], {l}[{}]))", p + r, p + r))
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
                    // Lanes a stride apart: each lane's at its step from the first's.
                    i @ V::Lin(f, s) => {
                        let (base, i) = (base.clone(), i.clone());
                        let g = self.strided(m, &base, f, s, 4, cx, |me| {
                            let l = me.lanes(&i, M::U32)?;
                            Ok(if cx.full { me.gather_all(&base, &l) } else { me.gather(&base, &l, &cur) })
                        })?;
                        Ok(V::L(g, m))
                    }
                    i => {
                        let l = self.lanes(i, M::U32)?;
                        let g = if cx.full { self.gather_all(base, &l) } else { self.gather(base, &l, &cur) };
                        Ok(V::L(g, m))
                    }
                }
            }
            V::At { base, idx, scale } if matches!(m.bytes(), 1 | 2 | 4 | 8) => self.load_elems(m, &base.clone(), &idx.clone(), *scale, cx),
            // Each lane's address: a load of each. A lane that does not run reads a running one's address (some lane
            // of `cur` runs wherever a statement does), as a gather's: its own may be anything.
            a @ (V::L(..) | V::Z(..)) if matches!(a, V::Z(..)) || matches!(a, V::L(_, am) if wide(*am)) => {
                let (p, w) = (self.p, self.w);
                let addr = self.addrs(a, cx)?;
                let t = self.tmp();
                self.line(format!("let mut {t}: [uint32x4_t; {}] = {};", if wide(m) { 2 * p } else { p }, self.zeros_of(m)));
                if self.rolled() {
                    self.load_lanes(&t, m, &format!("({addr}[j] as *const u8)"))?;
                    return Ok(V::L(t, m));
                }
                for j in 0..w {
                    let (r, q) = (j / 4, j % 4);
                    if wide(m) {
                        self.line(format!("{{ let v = ({addr}[{j}] as *const u64).read_unaligned(); {t}[{r}] = vsetq_lane_u32::<{q}>(v as u32, {t}[{r}]); {t}[{}] = vsetq_lane_u32::<{q}>((v >> 32) as u32, {t}[{}]); }}", p + r, p + r));
                    } else {
                        self.line(format!("{t}[{r}] = vsetq_lane_u32::<{q}>(({addr}[{j}] as *const {}).read_unaligned() as u32, {t}[{r}]);", scalar(m)?));
                    }
                }
                Ok(V::L(t, m))
            }
            a => err(format!("a load of {m:?} at {a:?}")),
        }
    }

    /// Each lane's address of `a` (64-bit lanes) as a `[usize; W]`. A lane that does not run has the first running
    /// one's (some lane of `cx` runs wherever a statement does), as a gather's: its own may be anything. Made once
    /// for a value in its block (`memo`): a variable's where it is set (`assign`), which every read of it in that
    /// block reads, under the lanes there or fewer.
    fn addrs(&mut self, a: &V, cx: &Cx) -> R<String> {
        let key = match a {
            V::L(n, _) => Some(format!("addr {n}")),
            _ => None,
        };
        if let Some(k) = &key
            && let Some(e) = self.memo.iter().find(|e| &e.0 == k)
        {
            return Ok(e.1.clone());
        }
        let h = self.named(a, M::U64)?;
        let (p, w) = (self.p, self.w);
        let lanes = |hs: &[String]| hs.join(", ");
        let (lo, hi) = (self.tmp(), self.tmp());
        self.line(format!("let {lo} = core::mem::transmute::<[uint32x4_t; {p}], [u32; {w}]>([{}]);", lanes(&h.lo)));
        match &h.hi {
            Some(hs) => self.line(format!("let {hi} = core::mem::transmute::<[uint32x4_t; {p}], [u32; {w}]>([{}]);", lanes(hs))),
            None => self.line(format!("let {hi} = [0u32; {w}];")),
        }
        let addr = self.tmp();
        // Of more than 8 lanes: a loop over them (`from_fn`), not each lane's text.
        if self.rolled() {
            let j = if cx.full {
                "j".to_string()
            } else {
                let (b, f) = (self.tmp(), self.tmp());
                self.line(format!("let {b} = bits(&{});", cx.cur));
                self.line(format!("let {f} = {b}.trailing_zeros() as usize % {w};"));
                format!("(if {b} >> j & 1 != 0 {{ j }} else {{ {f} }})")
            };
            self.line(format!("let {addr}: [usize; {w}] = core::array::from_fn(|j| {{ let j = {j}; (({hi}[j] as u64) << 32 | {lo}[j] as u64) as usize }});"));
        } else if cx.full {
            let at = (0..w).map(|j| format!("(({hi}[{j}] as u64) << 32 | {lo}[{j}] as u64) as usize")).collect::<Vec<_>>().join(", ");
            self.line(format!("let {addr}: [usize; {w}] = [{at}];"));
        } else {
            let (b, f) = (self.tmp(), self.tmp());
            self.line(format!("let {b} = bits(&{});", cx.cur));
            self.line(format!("let {f} = {b}.trailing_zeros() as usize % {w};"));
            // A lane that does not run: the first running one's address (its halves' select, before the pair).
            let at = (0..w).map(|j| format!("{{ let j = if {b} >> {j} & 1 != 0 {{ {j} }} else {{ {f} }}; (({hi}[j] as u64) << 32 | {lo}[j] as u64) as usize }}")).collect::<Vec<_>>().join(", ");
            self.line(format!("let {addr}: [usize; {w}] = [{at}];"));
        }
        if let Some(k) = key {
            let d = self.ind;
            self.memo.push((k, addr.clone(), d));
        }
        Ok(addr)
    }

    /// Each lane's `m` at `base` plus its index `first + j * stride` times `scale` (lane `j`'s), in a register's lanes
    /// one load each at a step of the stride's bytes, where every lane of `cx` runs and no lane's index wraps (as the
    /// program finds them); else `other`'s (each lane's index).
    fn strided(&mut self, m: M, base: &str, first: &str, stride: &str, scale: u32, cx: &Cx, other: impl FnOnce(&mut Self) -> R<String>) -> R<String> {
        let (e, p, w) = (m.bytes(), self.p, self.w);
        let n = if e == 8 { 2 * p } else { p };
        let t = self.tmp();
        let all = if cx.full { String::new() } else { format!(" && {}", self.all(&cx.cur)) };
        self.line(format!("let mut {t}: [uint32x4_t; {n}] = [vdupq_n_u32(0); {n}];"));
        self.line(format!("if (({first}) as u64) + {} * (({stride}) as u64) <= u32::MAX as u64{all} {{", w - 1));
        self.ind += 1;
        let (q, st) = (self.tmp(), self.tmp());
        self.line(format!("let {q} = ({base} as *const u8).add((({first}) as usize) * {scale});"));
        self.line(format!("let {st} = (({stride}) as usize) * {scale};"));
        let ut = ["u8", "u16", "", "u32", "", "", "", "u64"][e as usize - 1];
        let rolled = self.rolled();
        if rolled {
            self.load_lanes(&t, m, &format!("{q}.add(j * {st})"))?;
        }
        for j in (0..w).filter(|_| !rolled) {
            let (r, l) = (j / 4, j % 4);
            let a = format!("{q}.add({j} * {st})");
            match (e, m.signed()) {
                (4, _) => self.line(format!("{t}[{r}] = vld1q_lane_u32::<{l}>({a} as *const u32, {t}[{r}]);")),
                (8, _) => self.line(format!("{{ let v = ({a} as *const u64).read_unaligned(); {t}[{r}] = vsetq_lane_u32::<{l}>(v as u32, {t}[{r}]); {t}[{}] = vsetq_lane_u32::<{l}>((v >> 32) as u32, {t}[{}]); }}", p + r, p + r)),
                (1, true) => self.line(format!("{t}[{r}] = vsetq_lane_u32::<{l}>(({a} as *const i8).read() as i32 as u32, {t}[{r}]);")),
                (2, true) => self.line(format!("{t}[{r}] = vsetq_lane_u32::<{l}>(({a} as *const i16).read_unaligned() as i32 as u32, {t}[{r}]);")),
                _ => self.line(format!("{t}[{r}] = vsetq_lane_u32::<{l}>(({a} as *const {ut}).read_unaligned() as u32, {t}[{r}]);")),
            }
        }
        self.ind -= 1;
        self.line("} else {");
        self.ind += 1;
        let g = other(self)?;
        self.line(format!("{t} = {g};"));
        self.ind -= 1;
        self.line("}");
        Ok(t)
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
            V::At { base, .. } => root_of(base),
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
                    _ if m.bytes() == 4 => {
                        // Lanes' values at one address: C's threads race there, and any one's value stands. The
                        // last running lane's, as each running lane's in lane order would leave it: one store, where
                        // a lane runs (its mask's bits tested once).
                        let l = self.lanes(x, m)?;
                        let (w, p) = (self.w, self.p);
                        if cx.full {
                            self.line(format!("({s} as *mut u32).write_unaligned(vgetq_lane_u32::<{}>({l}[{}]));", (w - 1) % 4, (w - 1) / 4));
                        } else {
                            let b = self.tmp();
                            self.line(format!("let {b} = bits(&{});", cx.cur));
                            self.line(format!("if {b} != 0 {{"));
                            self.line(format!("    let j = 31 - {b}.leading_zeros() as usize;"));
                            self.line(format!("    ({s} as *mut u32).write_unaligned(core::mem::transmute::<[uint32x4_t; {p}], [u32; {w}]>({l})[j]);"));
                            self.line("}");
                        }
                        return Ok(());
                    }
                    // Another size's: the last running lane's, as above.
                    x if matches!(m.bytes(), 1 | 2 | 8) && !m.is_agg() => {
                        let w = self.w;
                        let V::S(xj, xm) = self.lane_of(x)? else { return err("a stored value at a lane") };
                        let (t, xv) = if m == M::B { ("u8".to_string(), format!("({}) as u8", truth(&xj, xm))) } else { (scalar(m)?.to_string(), conv(&xj, xm, m)) };
                        if cx.full {
                            self.line(format!("{{ let j = {}; ({s} as *mut {t}).write_unaligned({xv}); }}", w - 1));
                        } else {
                            let b = self.tmp();
                            self.line(format!("let {b} = bits(&{});", cx.cur));
                            self.line(format!("if {b} != 0 {{ let j = 31 - {b}.leading_zeros() as usize; ({s} as *mut {t}).write_unaligned({xv}); }}"));
                        }
                        return Ok(());
                    }
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
                    // Lanes a stride apart: each lane's word stored from its lane at its step from the first's (a lane
                    // that does not run, to a word of the stack's), where no lane's index wraps; else each lane's.
                    i @ V::Lin(f, s) => {
                        let i = i.clone();
                        self.line(format!("if (({f}) as u64) + {} * (({s}) as u64) <= u32::MAX as u64 {{", self.w - 1));
                        self.ind += 1;
                        let (q, st) = (self.tmp(), self.tmp());
                        self.line(format!("let {q} = ({base} as *mut u32).add(({f}) as usize);"));
                        self.line(format!("let {st} = ({s}) as usize;"));
                        let sink = (!cx.full).then(|| self.tmp());
                        if let Some(sink) = &sink {
                            self.line(format!("let mut {sink}w = 0u32;"));
                            self.line(format!("let {sink} = core::hint::black_box(&raw mut {sink}w);"));
                        }
                        for j in 0..self.w {
                            let at = format!("{q}.add({j} * {st})");
                            let at = match &sink {
                                Some(sink) => format!("core::hint::select_unpredictable({}, {at}, {sink})", self.on(&cur, j)),
                                None => at,
                            };
                            self.line(format!("vst1q_lane_u32::<{}>({at}, {l}[{}]);", j % 4, j / 4));
                        }
                        self.ind -= 1;
                        self.line("} else {");
                        self.ind += 1;
                        self.scatter4(base, &i, &l, &cur, cx.full)?;
                        self.ind -= 1;
                        self.line("}");
                    }
                    i if cx.full => self.scatter4(base, i, &l, &cur, true)?,
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
                    i => self.scatter4(base, i, &l, &cur, false)?,
                }
                Ok(())
            }
            V::At { base, idx, scale } if matches!(m.bytes(), 1 | 2 | 4 | 8) => self.store_elems(m, &base.clone(), &idx.clone(), *scale, x, cx),
            // Each lane's address (a pointer held as each lane's): each running lane's value at its own, in turn.
            a @ (V::L(..) | V::Z(..)) if matches!(m.bytes(), 1 | 2 | 4 | 8) && !m.is_agg() => {
                let w = self.w;
                let V::S(aj, am) = self.lane_of(a)? else { return err("a store's address at a lane") };
                let V::S(xj, xm) = self.lane_of(x)? else { return err("a store's value at a lane") };
                let aj = if am == M::PTR { aj } else { format!("({aj} as usize as *const u8)") };
                let guard = if cx.full {
                    String::new()
                } else {
                    let k = self.mask(&V::K(cx.cur.clone()))?;
                    let km = self.tmp();
                    self.line(format!("let {km} = core::mem::transmute::<[uint32x4_t; {}], [u32; {w}]>({k});", self.p));
                    format!("if {km}[j] != 0 ")
                };
                let t = if m == M::B { "u8".to_string() } else { scalar(m)?.to_string() };
                let xv = if m == M::B { format!("({}) as u8", truth(&xj, xm)) } else { conv(&xj, xm, m) };
                self.line(format!("for j in 0..{w} {{ {guard}{{ ({aj} as *mut {t}).write_unaligned({xv}); }} }}"));
                Ok(())
            }
            a => err(format!("a store of {m:?} at {a:?}")),
        }
    }

    /// Each lane's `u32` of `l` at `base` plus its index of `i` times 4: in the lanes of `cur` that run, unless `full`
    /// (all do).
    fn scatter4(&mut self, base: &str, i: &V, l: &str, cur: &str, full: bool) -> R<()> {
        let ix = self.lanes(i, M::U32)?;
        if full {
            for j in 0..self.w {
                let (r, q) = (j / 4, j % 4);
                self.line(format!("({base} as *mut u32).add(vgetq_lane_u32::<{q}>({ix}[{r}]) as usize).write(vgetq_lane_u32::<{q}>({l}[{r}]));"));
            }
            return Ok(());
        }
        // (its address opaque, so the compiler keeps the select and does not branch instead).
        let sink = self.tmp();
        self.line(format!("let mut {sink}w = 0u32;"));
        self.line(format!("let {sink} = core::hint::black_box(&raw mut {sink}w);"));
        for j in 0..self.w {
            let on = self.on(cur, j);
            let (r, q) = (j / 4, j % 4);
            self.line(format!("core::hint::select_unpredictable({on}, ({base} as *mut u32).add(vgetq_lane_u32::<{q}>({ix}[{r}]) as usize), {sink}).write(vgetq_lane_u32::<{q}>({l}[{r}]));"));
        }
        Ok(())
    }

    /// Each lane's element of `m` (1, 2, 4 or 8 bytes) at `base` plus its index `idx` times `scale` bytes, the
    /// lanes' own element (`scale` its size, `idx` consecutive) where they are.
    fn elem_at(&mut self, m: M, base: &str, idx: &V, scale: u32, cx: &Cx) -> R<(String, Option<String>, Option<String>)> {
        // A register's lanes at once: their first element's address. Else each lane's index, a lane that does
        // not run one that does (its element valid wherever a running lane's is).
        if scale == m.bytes()
            && let V::Iota(first, _) = idx
        {
            let q = self.tmp();
            self.line(format!("let {q} = ({base} as *const u8).add((({first}) as usize) * {scale});"));
            return Ok((q, None, None));
        }
        let l = self.lanes(idx, M::U32)?;
        if cx.full {
            return Ok((base.to_string(), Some(l), None));
        }
        let cur = cx.cur.clone();
        let mut e = format!("vandq_u32({cur}[0], {l}[0])");
        for r in 1..self.p {
            e = format!("vmaxq_u32({e}, vandq_u32({cur}[{r}], {l}[{r}]))");
        }
        let s = self.tmp();
        self.line(format!("let {s} = vdupq_n_u32(vmaxvq_u32({e}));"));
        Ok((base.to_string(), Some(self.map(|r| format!("vbslq_u32({cur}[{r}], {l}[{r}], {s})"))), Some(cur)))
    }

    /// A load of each lane's `m` (1, 2 or 8 bytes, or 4 at a scale not its own) at `base` plus its index times
    /// `scale`: consecutive elements a register's lanes at once where all run (8-byte ones parted in their halves,
    /// `vld2q_u32`), else each lane's element.
    fn load_elems(&mut self, m: M, base: &str, idx: &V, scale: u32, cx: &Cx) -> R<V> {
        let (e, p) = (m.bytes(), self.p);
        // Lanes `first + j * stride`: where the stride makes the elements consecutive and no lane's index wraps (as
        // the program finds them), a register's lanes at once; else each lane's.
        if let V::Lin(f, s) = idx
            && scale > 0
            && e % scale == 0
        {
            let n = if e == 8 { 2 * p } else { p };
            let t = self.tmp();
            let all = if cx.full { String::new() } else { format!(" && {}", self.all(&cx.cur)) };
            self.line(format!("let {t}: [uint32x4_t; {n}];"));
            self.line(format!("if ({s}) == {}u32 && (({f}) as u64) + {} * (({s}) as u64) <= u32::MAX as u64{all} {{", e / scale, self.w - 1));
            self.ind += 1;
            let q = self.tmp();
            self.line(format!("let {q} = ({base} as *const u8).add((({f}) as usize) * {scale});"));
            let l = self.consecutive(m, &q)?;
            self.line(format!("{t} = {l};"));
            self.ind -= 1;
            self.line("} else {");
            self.ind += 1;
            let idx = idx.clone();
            let g = self.strided(m, base, f, s, scale, cx, |me| {
                let l = me.lanes(&idx, M::U32)?;
                let V::L(g, _) = me.load_elems(m, base, &V::L(l, M::U32), scale, cx)? else { unreachable!() };
                Ok(g)
            })?;
            self.line(format!("{t} = {g};"));
            self.ind -= 1;
            self.line("}");
            return Ok(V::L(t, m));
        }
        // Lanes a stride apart (at a scale no stride makes consecutive): each lane's at its step from the first's.
        if let V::Lin(f, s) = idx
            && scale > 0
        {
            let idx = idx.clone();
            let g = self.strided(m, base, f, s, scale, cx, |me| {
                let l = me.lanes(&idx, M::U32)?;
                let V::L(g, _) = me.load_elems(m, base, &V::L(l, M::U32), scale, cx)? else { unreachable!() };
                Ok(g)
            })?;
            return Ok(V::L(g, m));
        }
        let (at, ix, _) = self.elem_at(m, base, idx, scale, cx)?;
        let n = if e == 8 { 2 * p } else { p };
        let t = self.tmp();
        let Some(ix) = ix else {
            // Consecutive elements.
            if cx.full {
                let l = self.consecutive(m, &at)?;
                return Ok(V::L(l, m));
            }
            self.line(format!("let mut {t}: [uint32x4_t; {n}];"));
            self.line(format!("if {} {{", self.all(&cx.cur)));
            self.ind += 1;
            let l = self.consecutive(m, &at)?;
            self.line(format!("{t} = {l};"));
            self.ind -= 1;
            self.line("} else {");
            self.ind += 1;
            let first = match idx {
                V::Iota(f, _) => f.clone(),
                _ => unreachable!(),
            };
            let lanes = self.lanes(&V::Iota(first, M::U32), M::U32)?;
            let g = self.load_elems(m, base, &V::L(lanes, M::U32), scale, cx)?;
            let V::L(g, _) = g else { unreachable!() };
            self.line(format!("{t} = {g};"));
            self.ind -= 1;
            self.line("}");
            return Ok(V::L(t, m));
        };
        if !cx.full {
            self.risky = true;
        }
        let ut = ["u8", "u16", "", "u32", "", "", "", "u64"][e as usize - 1];
        let ext = |v: &str| match (e, m.signed()) {
            (1, true) => format!("({v} as i8 as i32 as u32)"),
            (2, true) => format!("({v} as i16 as i32 as u32)"),
            _ => format!("({v} as u32)"),
        };
        self.line(format!("let mut {t}: [uint32x4_t; {n}] = [vdupq_n_u32(0); {n}];"));
        // Every lane's index one (a lane that does not run holds a running one's): one element for all, as a
        // uniform load's (a document's view where a group's words are one file's).
        let (mut lo, mut hi) = (format!("{ix}[0]"), format!("{ix}[0]"));
        for r in 1..p {
            lo = format!("vminq_u32({lo}, {ix}[{r}])");
            hi = format!("vmaxq_u32({hi}, {ix}[{r}])");
        }
        let u = self.tmp();
        self.line(format!("let {u} = vminvq_u32({lo});"));
        self.line(format!("if {u} == vmaxvq_u32({hi}) {{"));
        self.ind += 1;
        let one = format!("(({at} as *const u8).add({u} as usize * {scale}) as *const {ut}).read_unaligned()");
        if e == 8 {
            let v = self.tmp();
            self.line(format!("let {v} = {one};"));
            self.line(format!("{t} = [{}];", (0..n).map(|r| if r < p { format!("vdupq_n_u32({v} as u32)") } else { format!("vdupq_n_u32(({v} >> 32) as u32)") }).collect::<Vec<_>>().join(", ")));
        } else {
            self.line(format!("{t} = [vdupq_n_u32({}); {n}];", ext(&one)));
        }
        self.ind -= 1;
        self.line("} else {");
        self.ind += 1;
        let rolled = self.rolled();
        if rolled {
            let ia = self.tmp();
            self.line(format!("let {ia} = core::mem::transmute::<[uint32x4_t; {p}], [u32; {}]>({ix});", self.w));
            self.load_lanes(&t, m, &format!("({at} as *const u8).add({ia}[j] as usize * {scale})"))?;
        }
        for j in (0..self.w).filter(|_| !rolled) {
            let (r, q) = (j / 4, j % 4);
            let a = format!("(({at} as *const u8).add(vgetq_lane_u32::<{q}>({ix}[{r}]) as usize * {scale}) as *const {ut}).read_unaligned()");
            if e == 8 {
                self.line(format!("{{ let v = {a}; {t}[{r}] = vsetq_lane_u32::<{q}>(v as u32, {t}[{r}]); {t}[{}] = vsetq_lane_u32::<{q}>((v >> 32) as u32, {t}[{}]); }}", p + r, p + r));
            } else {
                self.line(format!("{t}[{r}] = vsetq_lane_u32::<{q}>({}, {t}[{r}]);", ext(&a)));
            }
        }
        self.ind -= 1;
        self.line("}");
        Ok(V::L(t, m))
    }

    /// The lanes' consecutive elements of `m` from address `q`: its registers (8-byte elements' low halves, then
    /// their high halves; narrower ones zero- or sign-extended to their lanes).
    fn consecutive(&mut self, m: M, q: &str) -> R<String> {
        let (e, p, s) = (m.bytes(), self.p, m.signed());
        let t = self.tmp();
        match e {
            8 => {
                for r in 0..p {
                    self.line(format!("let {t}{r} = vld2q_u32(({q} as *const u32).add({}));", 8 * r));
                }
                let lo: Vec<String> = (0..p).map(|r| format!("{t}{r}.0")).collect();
                let hi: Vec<String> = (0..p).map(|r| format!("{t}{r}.1")).collect();
                self.line(format!("let {t}: [uint32x4_t; {}] = [{}, {}];", 2 * p, lo.join(", "), hi.join(", ")));
            }
            4 => self.line(format!("let {t}: [uint32x4_t; {p}] = core::array::from_fn(|r| vld1q_u32(({q} as *const u32).add(4 * r)));")),
            2 if s => self.line(format!("let {t}: [uint32x4_t; {p}] = core::array::from_fn(|r| vreinterpretq_u32_s32(vmovl_s16(vld1_s16(({q} as *const i16).add(4 * r)))));")),
            2 => self.line(format!("let {t}: [uint32x4_t; {p}] = core::array::from_fn(|r| vmovl_u16(vld1_u16(({q} as *const u16).add(4 * r))));")),
            1 => {
                // Eight bytes a pair of registers; a last register alone, its four.
                let mut regs = Vec::new();
                for r in (0..p).step_by(2) {
                    let b = self.tmp();
                    if r + 1 < p {
                        self.line(format!("let {b} = {};", if s { format!("vreinterpretq_u16_s16(vmovl_s8(vld1_s8(({q} as *const i8).add({}))))", 4 * r) } else { format!("vmovl_u8(vld1_u8({q}.add({})))", 4 * r) }));
                        for h in ["low", "high"] {
                            regs.push(if s { format!("vreinterpretq_u32_s32(vmovl_s16(vget_{h}_s16(vreinterpretq_s16_u16({b}))))") } else { format!("vmovl_u16(vget_{h}_u16({b}))") });
                        }
                    } else {
                        let v = format!("vcreate_u8(({q}.add({}) as *const u32).read_unaligned() as u64)", 4 * r);
                        regs.push(if s { format!("vreinterpretq_u32_s32(vmovl_s16(vget_low_s16(vmovl_s8(vreinterpret_s8_u8({v})))))") } else { format!("vmovl_u16(vget_low_u16(vmovl_u8({v})))") });
                    }
                }
                self.line(format!("let {t}: [uint32x4_t; {p}] = [{}];", regs.join(", ")));
            }
            e => return err(format!("consecutive elements of {e} bytes")),
        }
        Ok(t)
    }

    /// A store of each lane's `x` of `m` (1, 2, 4 or 8 bytes) at `base` plus its index times `scale`, in the lanes
    /// of `cx` that run: consecutive elements a register's lanes at once where all run, else each lane's own write
    /// (a lane that does not run writes a word of the stack's, no branch).
    fn store_elems(&mut self, m: M, base: &str, idx: &V, scale: u32, x: &V, cx: &Cx) -> R<()> {
        let (e, p) = (m.bytes(), self.p);
        let l = self.lanes(x, m)?;
        let ut = ["u8", "u16", "", "u32", "", "", "", "u64"][e as usize - 1];
        let val = |j: u32| {
            let (r, q) = (j / 4, j % 4);
            if e == 8 { format!("((vgetq_lane_u32::<{q}>({l}[{}]) as u64) << 32 | vgetq_lane_u32::<{q}>({l}[{r}]) as u64)", p + r) } else { format!("(vgetq_lane_u32::<{q}>({l}[{r}]) as {ut})") }
        };
        let consecutive = scale == e && matches!(idx, V::Iota(..));
        let whole = |this: &mut Self, q: &str| {
            for r in 0..p {
                match e {
                    8 => this.line(format!("vst2q_u32(({q} as *mut u32).add({}), uint32x4x2_t({l}[{r}], {l}[{}]));", 8 * r, p + r)),
                    4 => this.line(format!("vst1q_u32(({q} as *mut u32).add({}), {l}[{r}]);", 4 * r)),
                    2 => this.line(format!("vst1_u16(({q} as *mut u16).add({}), vmovn_u32({l}[{r}]));", 4 * r)),
                    _ => {
                        for j in 4 * r..4 * r + 4 {
                            this.line(format!("({q} as *mut u8).add({j}).write({});", val(j)));
                        }
                    }
                }
            }
        };
        if consecutive {
            let (q, _, _) = self.elem_at(m, base, idx, scale, &Cx { full: true, ..cx.clone() })?;
            if cx.full {
                whole(self, &q);
                return Ok(());
            }
            self.line(format!("if {} {{", self.all(&cx.cur)));
            self.ind += 1;
            whole(self, &q);
            self.ind -= 1;
            self.line("} else {");
            self.ind += 1;
            let sink = self.tmp();
            self.line(format!("let mut {sink}w = 0u64;"));
            self.line(format!("let {sink} = core::hint::black_box(&raw mut {sink}w) as *mut {ut};"));
            for j in 0..self.w {
                let on = self.on(&cx.cur, j);
                self.line(format!("core::hint::select_unpredictable({on}, ({q} as *mut {ut}).add({j}), {sink}).write_unaligned({});", val(j)));
            }
            self.ind -= 1;
            self.line("}");
            return Ok(());
        }
        let ix = self.lanes(idx, M::U32)?;
        let sink = self.tmp();
        self.line(format!("let mut {sink}w = 0u64;"));
        self.line(format!("let {sink} = core::hint::black_box(&raw mut {sink}w) as *mut {ut};"));
        for j in 0..self.w {
            let (r, q) = (j / 4, j % 4);
            let at = format!("(({base} as *mut u8).add(vgetq_lane_u32::<{q}>({ix}[{r}]) as usize * {scale}) as *mut {ut})");
            if cx.full {
                self.line(format!("{at}.write_unaligned({});", val(j)));
            } else {
                let on = self.on(&cx.cur, j);
                self.line(format!("core::hint::select_unpredictable({on}, {at}, {sink}).write_unaligned({});", val(j)));
            }
        }
        Ok(())
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
            Builtin::Yield => V::None,
            // A group of one chunk: its threads in step.
            Builtin::Barrier | Builtin::SubGroupBarrier if self.one => V::None,
            // The thread's lane in its SIMD group of 32: its chunk's lanes from `lid0`'s (a chunk lies in one group).
            Builtin::SubGroupLocalId => V::Iota(if self.one { "lid0".into() } else { "(lid0 % 32)".into() }, M::U32),
            // The SIMD group's operations, its lanes the chunk's (a group of one chunk): across the registers.
            Builtin::SubGroupBallot | Builtin::SubGroupReduceAdd | Builtin::SubGroupReduceMin | Builtin::SubGroupReduceMax | Builtin::SubGroupShuffle | Builtin::SubGroupShuffleUp | Builtin::SubGroupShuffleDown | Builtin::SubGroupShuffleXor => {
                if !self.one {
                    return err(format!("{b:?} where a SIMD group is not the chunk"));
                }
                self.sg_op(b, m, &args, cx)?
            }
            // The device's fence (`ukl_fence_device`): the kernel's groups run on the machine's threads.
            Builtin::MemFence => {
                self.line("core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);");
                V::None
            }
            Builtin::AtomicLoad => self.load(m, &args[0], cx)?,
            Builtin::AtomicStore => {
                self.store(m, &args[0], &args[1], cx)?;
                V::None
            }
            Builtin::AtomicAdd | Builtin::AtomicMax | Builtin::AtomicMin | Builtin::AtomicOr | Builtin::AtomicAnd
                if m.bytes() == 4
                    && !m.signed()
                    && match (&args[0], &args[1]) {
                        (V::S(..), V::S(..)) | (V::At { .. }, _) => true,
                        (V::S(..), _) => matches!(b, Builtin::AtomicAdd | Builtin::AtomicMax | Builtin::AtomicMin) && self.unread(n),
                        _ => false,
                    } =>
            {
                let unread = self.unread(n);
                let k = match &args[0] {
                    V::S(s, _) => root_of(s),
                    V::At { base, .. } => root_of(base),
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
            // Another atomic of a 32-bit word (or one of the above signed): each running lane's in turn, at its
            // address, its old value its own.
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
                if m.bytes() == 4 && !m.float() =>
            {
                let k = match &args[0] {
                    V::S(s, _) => root_of(s),
                    V::At { base, .. } => root_of(base),
                    _ => None,
                };
                self.writes.push((k, false));
                let (at, t) = if m.signed() { ("core::sync::atomic::AtomicI32", "i32") } else { ("AtomicU32", "u32") };
                let w = self.w;
                // Each lane's address and operands, read at lane `j`.
                let addr = match self.lane_of(&args[0])? {
                    V::S(a, M::U64) => format!("({a} as usize as *mut {t})"),
                    V::S(a, _) => format!("({a} as *mut {t})"),
                    _ => return err("an atomic's address at a lane"),
                };
                let mut ops = Vec::new();
                for a in &args[1..] {
                    let V::S(x, xm) = self.lane_of(a)? else { return err("an atomic's operand at a lane") };
                    ops.push(format!("({} as {t})", conv(&x, xm, m)));
                }
                let op = |i: usize| ops.get(i).cloned().unwrap_or_default();
                let call = match b {
                    Builtin::AtomicAdd => format!("fetch_add({}, Relaxed)", op(0)),
                    Builtin::AtomicSub => format!("fetch_sub({}, Relaxed)", op(0)),
                    Builtin::AtomicAnd => format!("fetch_and({}, Relaxed)", op(0)),
                    Builtin::AtomicOr => format!("fetch_or({}, Relaxed)", op(0)),
                    Builtin::AtomicXor => format!("fetch_xor({}, Relaxed)", op(0)),
                    Builtin::AtomicXchg => format!("swap({}, Relaxed)", op(0)),
                    Builtin::AtomicMin => format!("fetch_min({}, Relaxed)", op(0)),
                    Builtin::AtomicMax => format!("fetch_max({}, Relaxed)", op(0)),
                    Builtin::AtomicInc => "fetch_add(1, Relaxed)".into(),
                    Builtin::AtomicDec => "fetch_sub(1, Relaxed)".into(),
                    _ => format!("compare_exchange({}, {}, Relaxed, Relaxed).unwrap_or_else(|x| x)", op(0), op(1)),
                };
                let on = if cx.full { None } else { Some(self.mask(&V::K(cx.cur.clone()))?) };
                let ov = self.tmp();
                self.line(format!("let mut {ov} = [0u32; {w}];"));
                let guard = match &on {
                    Some(k) => {
                        let km = self.tmp();
                        self.line(format!("let {km} = core::mem::transmute::<[uint32x4_t; {}], [u32; {w}]>({k});", self.p));
                        format!("if {km}[j] != 0 ")
                    }
                    None => String::new(),
                };
                self.line(format!("for j in 0..{w} {{ {guard}{{ {ov}[j] = {at}::from_ptr({addr}).{call} as u32; }} }}"));
                if self.unread(n) {
                    V::None
                } else {
                    V::L(self.map(|r| format!("vld1q_u32({ov}.as_ptr().add({}))", 4 * r)), m)
                }
            }
            // The ones of each lane's `m`: each byte's (`vcnt`), summed pairwise up to the lane's width.
            Builtin::Popcount => match &args[0] {
                V::S(s, _) => V::S(format!("((({s}) as {}).count_ones() as {})", scalar(m).unwrap_or("u32"), scalar(m).unwrap_or("u32")), m),
                // Of 64 bits: its halves' ones summed, a count of 32 bits.
                a if wide(m) => {
                    let h = self.named(a, m)?;
                    let cnt = |e: &str| format!("vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32({e}))))");
                    let p = self.p as usize;
                    let lo = (0..p).map(|r| match &h.hi { Some(hi) => format!("vaddq_u32({}, {})", cnt(&h.lo[r]), cnt(&hi[r])), None => cnt(&h.lo[r]) }).collect();
                    self.of_halves(H { lo, hi: None }, m)
                }
                a => {
                    let l = self.lanes(a, m)?;
                    V::L(self.map(|r| format!("vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32({l}[{r}]))))")), m)
                }
            },
            // The zeros above a value's highest one: of 64 bits, its high half's, or past a high half of none,
            // 32 and its low half's.
            Builtin::Clz => match &args[0] {
                V::S(s, _) => V::S(format!("((({s}) as {}).leading_zeros() as {})", scalar(m)?, scalar(m)?), m),
                a if wide(m) => {
                    let h = self.named(a, m)?;
                    let lo = (0..self.p as usize)
                        .map(|r| match &h.hi {
                            Some(hi) => format!("vbslq_u32(vceqzq_u32({}), vaddq_u32(vclzq_u32({}), vdupq_n_u32(32)), vclzq_u32({}))", hi[r], h.lo[r], hi[r]),
                            None => format!("vaddq_u32(vclzq_u32({}), vdupq_n_u32(32))", h.lo[r]),
                        })
                        .collect();
                    self.of_halves(H { lo, hi: None }, m)
                }
                a if m.bytes() == 4 => {
                    let l = self.lanes(a, m)?;
                    V::L(self.map(|r| format!("vclzq_u32({l}[{r}])")), m)
                }
                a => return err(format!("the zeros above {a:?} of {m:?}")),
            },
            // The high half of a product: of 64 bits, each lane's own (NEON has no 64-bit multiply); of 32, the halves
            // of each pair's product.
            Builtin::MulHi => match (&args[0], &args[1]) {
                (V::S(a, am), V::S(b, bm)) => {
                    let (wt, st) = (if m.signed() { "i128" } else { "u128" }, scalar(m)?);
                    let bits = 8 * m.bytes();
                    V::S(format!("((({} as {st}) as {wt} * ({} as {st}) as {wt}) >> {bits}) as {st}", conv(a, *am, m), conv(b, *bm, m)), m)
                }
                (a, b) if wide(m) => {
                    let (x, y, w, p) = (self.named(a, m)?, self.named(b, m)?, self.w, self.p as usize);
                    let (wt, st) = if m.signed() { ("i128", "i64") } else { ("u128", "u64") };
                    let t = self.tmp();
                    self.line(format!("let (mut {t}l, mut {t}h) = ([0u32; {w}], [0u32; {w}]);"));
                    let hv = |h: &H, r: usize| h.hi.as_ref().map_or_else(|| "vdupq_n_u32(0)".to_string(), |v| v[r].clone());
                    for j in 0..w {
                        let (r, q) = ((j / 4) as usize, j % 4);
                        let v = |h: &H| format!("((vgetq_lane_u32::<{q}>({}) as u64) << 32 | vgetq_lane_u32::<{q}>({}) as u64)", hv(h, r), h.lo[r]);
                        self.line(format!("{{ let v = ((({} as {st}) as {wt} * ({} as {st}) as {wt}) >> 64) as u64; {t}l[{j}] = v as u32; {t}h[{j}] = (v >> 32) as u32; }}", v(&x), v(&y)));
                    }
                    let lo = (0..p).map(|r| format!("vld1q_u32({t}l.as_ptr().add({}))", 4 * r)).collect();
                    let hi = (0..p).map(|r| format!("vld1q_u32({t}h.as_ptr().add({}))", 4 * r)).collect();
                    self.of_halves(H { lo, hi: Some(hi) }, m)
                }
                (a, b) if m.bytes() == 4 && !m.signed() => {
                    let (x, y) = (self.lanes(a, m)?, self.lanes(b, m)?);
                    let (xs, ys): (Vec<String>, Vec<String>) = (0..self.p).map(|r| (format!("{x}[{r}]"), format!("{y}[{r}]"))).unzip();
                    V::L(self.mulhi32(&xs, &ys), m)
                }
                (a, _) => return err(format!("the high half of {a:?}'s product of {m:?}")),
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

/// The lines past which an instance of a function the source declares a call (`Em::called`) is the Rust compiler's
/// to inline or not: a steer for code size, not a rule.
const CALLED_LINES: usize = 200;

/// Bytes `from..to` of a structure that no access names, as fields: each the widest of 4, 2 or 1 bytes that its
/// byte's alignment and the end allow.
fn gaps(from: u32, to: u32) -> Vec<(u32, M)> {
    let mut out = Vec::new();
    let mut b = from;
    while b < to {
        let n = [4u32, 2, 1].into_iter().find(|&n| b.is_multiple_of(n) && b + n <= to).unwrap_or(1);
        out.push((b, [M::U8, M::U16, M::VOID, M::U32][n as usize - 1]));
        b += n;
    }
    out
}

/// Line `s` with each list of `p` registers' expressions (`[e0, e1, ...]`) that are one expression of the
/// register's index (`per_register`) written once, over the registers (`from_fn`), where the list's type is known:
/// a typed `let`'s value, an assignment's, a transmute's argument.
fn roll(s: &str, p: usize) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut at = 0;
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'[' {
            let before = s[..i].trim_end();
            let typed = before.ends_with("] =") || before.ends_with(">(") || (before.ends_with(" =") && !before.contains("let "));
            // The list's end and its elements at its own depth.
            let mut depth = 0i32;
            let mut parts = Vec::new();
            let mut from = i + 1;
            let mut end = None;
            for k in i..b.len() {
                match b[k] {
                    b'[' | b'(' | b'{' => depth += 1,
                    b']' | b')' | b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            parts.push(s[from..k].trim().to_string());
                            end = Some(k);
                            break;
                        }
                    }
                    b',' if depth == 1 => {
                        parts.push(s[from..k].trim().to_string());
                        from = k + 1;
                    }
                    b';' if depth == 1 => break,
                    _ => {}
                }
            }
            // Of `p` registers, or of `2p` (a 64-bit value's halves): one expression, or one of each half.
            let rolled = match parts.len() {
                n if n == p => per_register(&parts, 0),
                n if n == 2 * p => per_register(&parts, 0).or_else(|| {
                    let (a, b) = (per_register(&parts[..p], 0)?, per_register(&parts[p..], p)?);
                    Some(format!("if r < {p} {{ {a} }} else {{ {b} }}"))
                }),
                _ => None,
            };
            if let Some(e) = end
                && typed
                && let Some(t) = rolled
            {
                out.push_str(&s[at..i]);
                out.push_str(&format!("core::array::from_fn(|r| {t})"));
                at = e + 1;
                i = e + 1;
                continue;
            }
        }
        i += 1;
    }
    out.push_str(&s[at..]);
    out
}

/// The registers' expressions `parts` (`map`) as one of a variable `r`, if they are one text but for numbers that
/// are each its register's index (an element's, `x[r]`). Not where that number is a constant's (`::<r>`).
fn per_register(parts: &[String], shift: usize) -> Option<String> {
    // Each text's runs of digits and of other characters.
    fn runs(x: &str) -> Vec<&str> {
        let mut out = Vec::new();
        let mut at = 0;
        let b = x.as_bytes();
        while at < b.len() {
            let d = b[at].is_ascii_digit();
            let end = (at..b.len()).find(|&k| b[k].is_ascii_digit() != d).unwrap_or(b.len());
            out.push(&x[at..end]);
            at = end;
        }
        out
    }
    let all: Vec<Vec<&str>> = parts.iter().map(|x| runs(x)).collect();
    let first = &all[0];
    if all.iter().any(|r| r.len() != first.len()) {
        return None;
    }
    let mut out = String::new();
    let mut varies = false;
    for (t, run) in first.iter().enumerate() {
        if all.iter().all(|r| r[t] == *run) {
            out.push_str(run);
            continue;
        }
        let digits = run.as_bytes()[0].is_ascii_digit();
        // A number that is each register's index (`r`, part `k` register `k + shift`), or its index in its half
        // (`r - shift`); not part of a name (`t12`) nor a constant parameter's.
        let named = t > 0 && first[t - 1].ends_with(|c: char| c.is_ascii_alphabetic() || c == '_');
        if !digits || named || out.ends_with("::<") {
            return None;
        }
        if all.iter().enumerate().all(|(k, r)| r[t] == (k + shift).to_string()) {
            out.push('r');
        } else if shift > 0 && all.iter().enumerate().all(|(k, r)| r[t] == k.to_string()) {
            out.push_str(&format!("(r - {shift})"));
        } else {
            return None;
        }
        varies = true;
    }
    varies.then_some(out)
}

/// A structure's field's key among its function's variables (`Em::structs`): above every variable's number.
fn field_key(var: u32, byte: u32) -> u32 {
    1 << 31 | byte << 20 | var
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

/// `x op y` of two constants of `u32` (`konst`'s), if both are and `op` is an integer's operation that cannot fail.
fn fold32(op: Op, x: &str, y: &str) -> Option<u32> {
    let (x, y) = (konst(x)?, konst(y)?);
    Some(match op {
        Op::Add => x.wrapping_add(y),
        Op::Sub => x.wrapping_sub(y),
        Op::Mul => x.wrapping_mul(y),
        Op::And => x & y,
        Op::Or => x | y,
        Op::Xor => x ^ y,
        Op::Shl => x.wrapping_shl(y),
        Op::Shr => x.wrapping_shr(y),
        _ => return None,
    })
}

/// A constant of 64 bits, an unsigned literal or one cast to an unsigned type no narrower than its value.
fn konst64(s: &str) -> Option<u64> {
    let mut s = s.trim();
    while let Some(i) = s.strip_prefix('(').and_then(|i| i.strip_suffix("as u64)")) {
        s = i.trim_end();
        while let Some(i) = s.strip_prefix('(').and_then(|i| i.strip_suffix(')')) {
            s = i;
        }
    }
    let digits = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    ["", "u8", "u16", "u32", "u64"].contains(&&s[digits..]).then(|| s[..digits].parse().ok()).flatten()
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
