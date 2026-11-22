//! A kernel's SSA (decision 0567, step 3): its vector tape as functions of blocks of values, each value defined once,
//! for the compiled back ends (Cranelift's, Rust's) to make vector code of.
//!
//! - **Values** are typed (`M`'s scalar types; an aggregate's value is its address, a pointer) and shaped: uniform (one
//!   for the SIMD group: a scalar register), lanes (one a lane: a vector), or a mask (the lanes in a set). A value's
//!   shape follows from its operands' (a uniform operand of a lanes operation is splatted where it is used), so a
//!   uniform computation stays scalar wherever it is.
//! - **Variables are SSA values.** A lane's variable whose address is never taken, read and written whole, is no
//!   frame memory: each write a new value, joined at blocks' parameters (Braun et al., "Simple and efficient
//!   construction of SSA form"). The frame keeps the variables that are addressed. A write under a mask that is not
//!   the function's entry's is a `Select` of the new lanes over the old.
//! - **Masks are SSA values too**: the lanes running (`CUR`) and those that took a `break`, a `continue`, a `return`
//!   or ended. A uniform branch is a branch; a varying one sets the running lanes for each side, each side skipped
//!   when it has none (`Any`), as the code tape's lowering does (`code`), whose semantics this keeps.
//! - **A call is its callee's instructions** in its caller, its locals past its caller's region in the frame, its
//!   result and its returned lanes variables of its own, unless the callee is declared `noinline`
//!   (`lower::Func::called`): then a function, its arguments values (uniform where every call's context has them so),
//!   its result a value, under the lanes that call; the call's extra words its arguments, then the frame's advance.
//! - **Cross-lane operations** (a ballot, a shuffle, a reduction, an agreed branch's check, a barrier, a poll, a call
//!   of a function that has one) are marked: the back ends run the code between them at a width of their choosing.
//!
//! The SSA is a derived fact written once into its tape (`Ssa::tape`); the back ends read it there.

use std::collections::HashMap;

use crate::Error;
use crate::k::{Builder, Builtin, CHECK, Kind, Level, M, NO_SLOT, Op, Tape, View};
use crate::run::W;

type R<T> = Result<T, Error>;

/// A value: its index in its function.
pub type V = u32;

/// No value.
pub const NONE: V = u32::MAX;

/// A value's shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Shape {
    /// No value (an effect, a terminator).
    None,
    /// One for the SIMD group.
    Uniform,
    /// One a lane.
    Lanes,
    /// A set of lanes.
    Mask,
}

/// An operation. Operands are values unless said.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum O {
    /// `[low, high]` (words): a uniform constant.
    Const,
    /// `[k]` (a word): the function's argument `k`.
    Arg,
    /// The lanes the function runs under.
    Entry,
    /// No lanes.
    NoLanes,
    /// A value never set (a variable read before its first write): any.
    Undef,
    /// `[k]` (a word): its block's parameter `k`.
    Param,
    /// `[a, b, mask]`, sub the `Op`, `d` the operands' type: `a op b` (a comparison's a `bool`); `mask` (else `NONE`)
    /// an integer division's running lanes.
    Bin,
    /// `[a]`, sub the `Op`.
    Un,
    /// `[a, from]` (`from` an `M`'s word): `a` converted to `m`.
    Cast,
    /// `[mask, a, b]`: `a`'s lanes in `mask`, `b`'s elsewhere.
    Select,
    /// `[x, mask]`: the first lane of `mask`'s value of `x` (a lane in the chunk running it: `x` is the same in every
    /// lane of `mask`, or the code takes any one, as a varying `switch`'s round does).
    First,
    /// `[x]`: the lanes whose `x` is not 0 (`x` a `bool`'s lanes).
    MaskOf,
    /// `[m, n]`: `m & n`.
    MAnd,
    /// `[m, n]`: `m | n`.
    MOr,
    /// `[m, n]`: `m & !n`.
    MAndNot,
    /// `[m]`: whether `m` has a lane in the chunk running it (a skip of code with none).
    Any,
    /// `[m]`: whether `m` has a lane in the SIMD group (cross-lane).
    AnyGroup,
    /// `[m]`: `m`'s lanes as bits, a `u32` (a ballot; cross-lane).
    Bits,
    /// `[offset]` (a word): the program's constant memory's address.
    Global,
    /// `[offset]` (a word): the group's threadgroup memory's address.
    Shared,
    /// `[base, stride]` (words): each lane's variable's address in the frame, `fp + base + lane * stride`.
    FrameAddr,
    /// `[address, mask]`: the value of `m` at the address (one for the SIMD group at a uniform one).
    Load,
    /// `[address, x, mask]`: `x` stored at the address, the mask's lanes in order.
    Store,
    /// `[to, from, mask]`, sub-word `bytes` in `m`'s place: each lane's bytes copied.
    Copy,
    /// `[address, mask]`, `bytes` in `m`'s place: each lane's bytes cleared.
    Zero,
    /// `[dim]`: the lane's thread's id in its group (`Lid`) or the grid (`Gid`), of `m`.
    Lid,
    Gid,
    /// The lane's index in its SIMD group, a `u32`.
    Lane,
    /// `[dim]`, sub the `Builtin`: a size or the group's id, uniform.
    GroupInfo,
    /// `[a, b, from]` (`from` a word), sub the `Builtin`: a lane's own builtin (popcount, clz, mul_hi, as_*, min,
    /// max) of operands of `from`.
    LaneOp,
    /// `[x, lane, mask]`, sub the `Builtin`: a shuffle (cross-lane).
    Shuffle,
    /// `[x, mask]`, sub the `Builtin`: the running lanes' sum, minimum or maximum, uniform (cross-lane).
    Reduce,
    /// `[address, x, y, mask]`, sub the `Builtin`: an atomic, once for the SIMD group (`Uniform`'s shape) or each
    /// lane's in order.
    Atomic,
    /// `[node, site, decision, mask]` (words, values): the group decides an agreed branch alike (cross-lane: the group's);
    /// its value (of `m`, else none) is the group's decision, which the branch takes.
    Agree,
    /// `[site]` (a word): a threadgroup barrier.
    Bar,
    /// A poll of another group's progress.
    Yield,
    /// A SIMD group barrier: what its lanes wrote before it, each lane reads after it (cross-lane: a chunk of fewer
    /// lanes than the group's ends at it).
    SgBar,
    /// `[function, args at, args, mask]` (words, words, a word, a value): a call, its result of `m`.
    Call,
    /// `[block, args at, args]` (words).
    Jump,
    /// `[cond, then, else]` (a value, blocks): a branch on a uniform `bool`, to blocks with one predecessor each.
    Br,
    /// `[x, labels at, labels, default]` (a value, then words): a jump by uniform `x` through sorted `(low, high,
    /// block)` triples.
    Switch,
    /// `[x]`: the function's end, its result `x` (or `NONE`).
    Ret,
    /// The kernel's end.
    Halt,
}

impl O {
    pub const LAST: u8 = O::Halt as u8;

    pub fn of(b: u8) -> O {
        assert!(b <= Self::LAST, "an SSA operation: {b}");
        // SAFETY: `O` is `repr(u8)`, its values 0 to `LAST` in order.
        unsafe { std::mem::transmute::<u8, O>(b) }
    }

    pub fn terminates(self) -> bool {
        matches!(self, O::Jump | O::Br | O::Switch | O::Ret | O::Halt)
    }

    /// Whether it reads the SIMD group's lanes as a whole (the back ends' widths end at it).
    pub fn cross_lane(self) -> bool {
        matches!(self, O::AnyGroup | O::Bits | O::Shuffle | O::Reduce | O::Agree | O::Bar | O::Yield | O::SgBar)
    }
}

/// An instruction: its operation, sub-operation, type and shape, and its operands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Inst {
    pub op: O,
    pub sub: u8,
    pub shape: Shape,
    pub m: M,
    pub a: u32,
    pub b: u32,
    pub c: u32,
    pub d: u32,
}

impl Inst {
    fn new(op: O, m: M, a: u32, b: u32, c: u32, d: u32) -> Inst {
        Inst { op, sub: 0, shape: Shape::None, m, a, b, c, d }
    }

    /// Its operands that are values.
    pub fn uses(&self) -> impl Iterator<Item = V> {
        let (a, b, c, d) = (self.a, self.b, self.c, self.d);
        let u: [Option<V>; 4] = match self.op {
            O::Const | O::Arg | O::Entry | O::NoLanes | O::Undef | O::Param | O::Global | O::Shared | O::FrameAddr | O::Lane | O::Bar | O::Yield | O::SgBar | O::Jump | O::Halt => [None; 4],
            O::Bin => [Some(a), Some(b), Some(c), None],
            O::MAnd | O::MOr | O::MAndNot => [Some(a), Some(b), None, None],
            O::Un | O::Cast | O::MaskOf | O::Any | O::AnyGroup | O::Bits | O::Lid | O::Gid | O::GroupInfo => [Some(a), None, None, None],
            O::Select | O::Store => [Some(a), Some(b), Some(c), None],
            O::First | O::Load | O::Zero | O::Reduce => [Some(a), Some(b), None, None],
            O::Copy | O::Shuffle => [Some(a), Some(b), Some(c), None],
            O::LaneOp => [Some(a), Some(b), None, None],
            O::Atomic => [Some(a), Some(b), Some(c), Some(d)],
            O::Agree => [Some(c), Some(d), None, None],
            O::Call => [Some(d), None, None, None],
            O::Br | O::Switch => [Some(a), None, None, Some(d)],
            O::Ret => [Some(a), None, None, None],
        };
        u.into_iter().flatten().filter(|&x| x != NONE)
    }

    /// Its operands that are values, to change.
    fn uses_mut(&mut self) -> Vec<&mut u32> {
        let op = self.op;
        let Inst { a, b, c, d, .. } = self;
        let v: Vec<&mut u32> = match op {
            O::Const | O::Arg | O::Entry | O::NoLanes | O::Undef | O::Param | O::Global | O::Shared | O::FrameAddr | O::Lane | O::Bar | O::Yield | O::SgBar | O::Jump | O::Halt => vec![],
            O::Bin => vec![a, b, c],
            O::MAnd | O::MOr | O::MAndNot | O::First | O::Load | O::Zero | O::Reduce | O::LaneOp => vec![a, b],
            O::Un | O::Cast | O::MaskOf | O::Any | O::AnyGroup | O::Bits | O::Lid | O::Gid | O::GroupInfo | O::Ret => vec![a],
            O::Br | O::Switch => vec![a, d],
            O::Select | O::Store | O::Copy | O::Shuffle => vec![a, b, c],
            O::Atomic => vec![a, b, c, d],
            O::Agree => vec![c, d],
            O::Call => vec![d],
        };
        v.into_iter().filter(|x| **x != NONE).collect()
    }
}

/// A block: its parameters, then its instructions, the last its terminator.
#[derive(Clone, Debug, Default)]
pub struct Block {
    pub params: Vec<V>,
    pub insts: Vec<V>,
}

/// A function: the kernel's (the first), or a body's in its context.
#[derive(Clone, Debug)]
pub struct Func {
    /// The vector tape's body (its index; 0 the kernel).
    pub body: u32,
    pub name: String,
    /// Its region's bytes in a lane's frame (a call advances the frame by its caller's).
    pub region: u32,
    /// Its arguments' types and shapes.
    pub args: Vec<(M, Shape)>,
    /// Its result's.
    pub ret: (M, Shape),
    pub insts: Vec<Inst>,
    pub blocks: Vec<Block>,
    /// Jumps' and calls' arguments, `Switch`es' labels.
    pub extra: Vec<u32>,
    /// Whether it, or a function it calls, has a cross-lane operation.
    pub cross: bool,
}

/// A kernel's SSA: its functions, the kernel's first.
#[derive(Clone, Debug)]
pub struct Ssa {
    pub funcs: Vec<Func>,
    /// The kernel's parameters' frame offsets and types (as the code tape's: `run` stores its arguments there).
    pub params: Vec<(u32, M)>,
}

impl Func {
    pub fn inst(&self, v: V) -> &Inst {
        &self.insts[v as usize]
    }

    /// A jump's or call's arguments.
    /// If `c` (a `bool`'s lanes) is `lane == u` (through casts), `u` uniform: `u` (at most one lane of a SIMD group
    /// has it true, lanes being distinct).
    pub fn lane_is(&self, c: V) -> Option<V> {
        lane_is(&self.insts, c)
    }

    pub fn args_of(&self, i: &Inst) -> &[u32] {
        &self.extra[i.b as usize..][..i.c as usize]
    }

    /// A `Switch`'s labels: `(value, block)`.
    pub fn labels(&self, i: &Inst) -> impl Iterator<Item = (u64, u32)> + '_ {
        self.extra[i.b as usize..][..3 * i.c as usize].chunks(3).map(|t| (u64::from(t[0]) | u64::from(t[1]) << 32, t[2]))
    }

    /// A `Switch`'s default block (its extra word after its labels).
    pub fn default_of(&self, i: &Inst) -> u32 {
        self.extra[(i.b + 3 * i.c) as usize]
    }

    /// Block `b`'s successors.
    pub fn succs(&self, b: usize) -> Vec<u32> {
        let t = self.inst(*self.blocks[b].insts.last().expect("a block's terminator"));
        match t.op {
            O::Jump => vec![t.a],
            O::Br => vec![t.b, t.c],
            O::Switch => {
                let mut s: Vec<u32> = self.labels(t).map(|(_, b)| b).collect();
                s.push(self.default_of(t));
                s
            }
            _ => vec![],
        }
    }
}

/// A mask variable.
const CUR: u64 = 0;
const BRK: u64 = 1;
const CONT: u64 = 2;
const RET: u64 = 3;
const END: u64 = 4;
/// The function's result.
const RESULT: u64 = 5;
/// A lane's variable of `s` bytes at frame offset `o` (`local(o, s)`); a temporary (`TEMP + n`).
const LOCAL: u64 = 1 << 32;

/// A kept variable's id: its offset and size (variables of disjoint scopes share their places, each its own).
fn local(o: u32, s: u32) -> u64 {
    LOCAL + (u64::from(s) << 24) + u64::from(o)
}
const TEMP: u64 = 2 << 32;
/// `Func::lane_is` over the values `insts`.
fn lane_is(insts: &[Inst], c: V) -> Option<V> {
    let strip = |mut x: V| {
        while insts[x as usize].op == O::Cast {
            x = insts[x as usize].a;
        }
        x
    };
    let i = insts[strip(c) as usize];
    if i.op != O::Bin || Op::of(u32::from(i.sub)) != Op::Eq {
        return None;
    }
    let lane = |x: V| insts[strip(x) as usize].op == O::Lane;
    let uniform = |x: V| insts[x as usize].shape == Shape::Uniform;
    if lane(i.a) && uniform(i.b) {
        Some(i.b)
    } else if lane(i.b) && uniform(i.a) {
        Some(i.a)
    } else {
        None
    }
}

/// The most values a function's SSA may have: past it the kernel is refused as it is built, never compiled or run.
/// The compiled code's size and its compile's time grow with it (Cranelift took 17 to 24 s on functions of 110,000 to
/// 330,000 values); the library's kernels' largest function is under 8,000.
pub const MAX_VALUES: usize = 20_000;

/// The most values a function's builder may make (dead ones and those simplified away among them, which `finish`
/// drops): a function past it would be past `MAX_VALUES`, its build stopped there.
const MAX_MADE: usize = 8 * MAX_VALUES;

/// A body whose calls are its instructions (`build`'s `bodies`).
const INLINE: usize = usize::MAX;

/// A phi being made: its block, variable, and operands in its block's predecessors' order.
struct Phi {
    block: u32,
    var: u64,
    ops: Vec<V>,
}

/// The SSA of the vector tape `vt`.
pub fn build(vt: &Tape, names: &dyn Fn(u32) -> String, called: &dyn Fn(u32) -> bool) -> R<Ssa> {
    let v = View::new(vt);
    let n = v.len();
    let mut exits = vec![false; n];
    for i in (0..n).rev() {
        exits[i] = match v.kind(i) {
            Kind::Break | Kind::Continue | Kind::Return => true,
            Kind::Call => false,
            _ => v.children(i).any(|c| exits[c]),
        };
    }
    let all: Vec<usize> = v.children(0).collect();
    // The functions: the kernel's, then each body of a `noinline` function. Each body by its id: its node, and its
    // index among the functions (`INLINE`: its calls are its instructions).
    let kids: Vec<usize> = std::iter::once(all[0]).chain(all[1..].iter().copied().filter(|&b| called(v.w(b, 1)))).collect();
    let mut bodies: HashMap<u32, (usize, usize)> = all[1..].iter().map(|&b| (v.w(b, 0), (b, INLINE))).collect();
    for (k, &b) in kids.iter().enumerate().skip(1) {
        bodies.insert(v.w(b, 0), (b, k));
    }
    // The calls each function makes (through the bodies it has inlined too), and each called body's arguments'
    // uniformity (every call's, by its context's levels).
    let mut calls: Vec<Vec<usize>> = vec![Vec::new(); kids.len()];
    let mut uniform_arg: HashMap<u32, Vec<bool>> = HashMap::new();
    fn walk(v: &View, root: usize, bodies: &HashMap<u32, (usize, usize)>, out: &mut Vec<usize>, uniform_arg: &mut HashMap<u32, Vec<bool>>) {
        for i in root..root + v.size(root) {
            if v.kind(i) == Kind::Call {
                let id = v.w(i, 3);
                let (body, k) = bodies[&id];
                if k == INLINE {
                    walk(v, body, bodies, out, uniform_arg);
                    continue;
                }
                out.push(k);
                let bp = v.p(body);
                let u: Vec<bool> = v.children(i).enumerate().map(|(a, c)| v.level(c) <= Level::SimdGroup && !M(bp[8 + 2 * a]).is_agg()).collect();
                let e = uniform_arg.entry(id).or_insert_with(|| u.clone());
                for (x, y) in e.iter_mut().zip(&u) {
                    *x &= *y;
                }
            }
        }
    }
    for (k, &f) in kids.iter().enumerate() {
        walk(&v, f, &bodies, &mut calls[k], &mut uniform_arg);
    }
    // The functions callees first (a call that recurses is refused before: `vector`).
    let mut order = Vec::new();
    let mut seen = vec![false; kids.len()];
    fn visit(k: usize, calls: &[Vec<usize>], seen: &mut [bool], order: &mut Vec<usize>) {
        if seen[k] {
            return;
        }
        seen[k] = true;
        for &c in &calls[k] {
            visit(c, calls, seen, order);
        }
        order.push(k);
    }
    visit(0, &calls, &mut seen, &mut order);
    let mut funcs: Vec<Option<Func>> = vec![None; kids.len()];
    for &k in &order {
        let f = kids[k];
        let (root, region, args, ret, body_id) = if k == 0 {
            let p = v.p(0);
            let params = p[4] as usize;
            let args: Vec<(u32, M)> = (0..params).map(|a| (p[5 + 2 * a], M(p[6 + 2 * a]))).collect();
            (v.child(0, 0), crate::vector::kernel_frame(&v), args, M::VOID, 0u32)
        } else {
            let p = v.p(f);
            let params = p[6] as usize;
            let args: Vec<(u32, M)> = (0..params).map(|a| (p[7 + 2 * a], M(p[8 + 2 * a]))).collect();
            (v.child(f, 0), crate::vector::region(p[2], M(p[4])), args, M(p[4]), p[0])
        };
        let uni = if k == 0 { vec![true; args.len()] } else { uniform_arg.get(&body_id).cloned().unwrap_or_else(|| vec![false; args.len()]) };
        let name = if k == 0 { names(u32::MAX) } else { names(v.w(f, 1)) };
        let mut b = Fb::new(&v, &exits, &bodies, &funcs, root, k == 0);
        b.region = region;
        b.promote(root, &args);
        let func = b.function(k as u32, name, region, &args, &uni, ret, root)?;
        if func.insts.len() > MAX_VALUES {
            return Err(Error(format!("{}: {} values, over {MAX_VALUES}: too large to compile", func.name, func.insts.len())));
        }
        funcs[k] = Some(func);
    }
    let funcs: Vec<Func> = funcs.into_iter().map(|f| f.expect("each function built")).collect();
    let p = v.p(0);
    let params = (0..p[4] as usize).map(|a| (p[5 + 2 * a], M(p[6 + 2 * a]))).collect();
    let mut s = Ssa { funcs, params };
    let cross: Vec<bool> = {
        // An atomic once for the SIMD group is the group's too: a chunk of fewer lanes would make it once each.
        let mut c: Vec<bool> = s.funcs.iter().map(|f| f.insts.iter().any(|i| i.op.cross_lane() || (i.op == O::Atomic && i.shape == Shape::Uniform))).collect();
        for &k in &order {
            let f = &s.funcs[k];
            if f.insts.iter().any(|i| i.op == O::Call && c[i.a as usize]) {
                c[k] = true;
            }
        }
        c
    };
    for (f, c) in s.funcs.iter_mut().zip(cross) {
        f.cross = c;
    }
    for f in &s.funcs {
        verify(f).map_err(|e| Error(format!("{}: the SSA: {e}", f.name)))?;
    }
    Ok(s)
}

/// A function's builder.
struct Fb<'a, 't> {
    v: &'a View<'t>,
    exits: &'a [bool],
    bodies: &'a HashMap<u32, (usize, usize)>,
    built: &'a [Option<Func>],
    kernel: bool,
    insts: Vec<Inst>,
    /// Each value's block (a parameter's too).
    at: Vec<u32>,
    blocks: Vec<Block>,
    preds: Vec<Vec<u32>>,
    sealed: Vec<bool>,
    /// Whether a block has its terminator.
    ended: Vec<bool>,
    cur: u32,
    defs: HashMap<(u32, u64), V>,
    incomplete: HashMap<u32, Vec<(u64, V)>>,
    phis: HashMap<V, Phi>,
    /// A value replaced by another (a trivial phi's).
    repl: HashMap<V, V>,
    /// Each variable's type.
    var_m: HashMap<u64, M>,
    /// The frame's variables kept as values: offset to size.
    promoted: std::collections::HashSet<(u32, u32)>,
    entry: V,
    temps: u64,
    extra: Vec<u32>,
    /// Values made once: constants, `Entry`, `NoLanes`, `Undef` by type, in the entry block.
    consts: HashMap<(O, u32, u32, u32), V>,
    /// The phis each value is an operand of.
    phi_users: HashMap<V, Vec<V>>,
    /// Whether a value is uniform whatever its block's parameters turn out to be (none of its operands one): what a
    /// branch may be on as it is made.
    stable: Vec<bool>,
    /// The region in a lane's frame of the body being built (the function's, or an inlined call's): a call from it
    /// advances the frame past it.
    region: u32,
    /// Where the body being built is in the function's frame: 0, or past the regions of the calls it is inlined in.
    base: u32,
    /// The body's returned lanes (a mask variable) and its result: `RET` and `RESULT`, or an inlined call's own.
    rv: u64,
    res: u64,
    /// The ends of a uniform branch's sides whose lanes have all left (a `return`, a `break`, a `continue`), not
    /// yet ended: each goes on past the statements its exit skips (`stmts`), never through the code under the lanes
    /// that remain, so that code's lanes stay what they were (the entry's, where they were).
    dead: Vec<u32>,
    /// Whether the statement being built is followed by those its exits skip (`stmts`): its sides' ends may go
    /// past them (`dead`).
    skips: bool,
}

impl<'a, 't> Fb<'a, 't> {
    fn new(v: &'a View<'t>, exits: &'a [bool], bodies: &'a HashMap<u32, (usize, usize)>, built: &'a [Option<Func>], _root: usize, kernel: bool) -> Self {
        Fb {
            v,
            exits,
            bodies,
            built,
            kernel,
            insts: Vec::new(),
            at: Vec::new(),
            blocks: Vec::new(),
            preds: Vec::new(),
            sealed: Vec::new(),
            ended: Vec::new(),
            cur: 0,
            defs: HashMap::new(),
            incomplete: HashMap::new(),
            phis: HashMap::new(),
            repl: HashMap::new(),
            var_m: HashMap::new(),
            promoted: std::collections::HashSet::new(),
            entry: NONE,
            temps: 0,
            extra: Vec::new(),
            consts: HashMap::new(),
            phi_users: HashMap::new(),
            stable: Vec::new(),
            region: 0,
            base: 0,
            rv: RET,
            res: RESULT,
            dead: Vec::new(),
            skips: false,
        }
    }

    /// The frame's variables kept as values: those every access of reads or writes whole, at one type's size, by
    /// its `Local` directly, never as an address. A function's parameters among them.
    fn promote(&mut self, root: usize, args: &[(u32, M)]) {
        let v = self.v;
        // Each variable (its offset and size: variables of disjoint scopes share places) and whether it may be kept,
        // and whether as a float. A variable is one size, so another size at its offset is another variable.
        let mut seen: HashMap<(u32, u32), (bool, Option<bool>)> = HashMap::new();
        self.scan(root, 0, self.region, &mut seen);
        for &(off, m) in args {
            let e = seen.entry((off, m.bytes())).or_insert((true, Some(m.float())));
            if m.is_agg() {
                e.0 = false;
            }
        }
        self.promoted = seen.into_iter().filter(|(_, e)| e.0).map(|(k, _)| k).collect();
    }

    /// `promote`'s variables of the body at `root`, at `base` in the frame, its region `region`, and of each call it
    /// inlines (past its region), their parameters too.
    fn scan(&self, root: usize, base: u32, region: u32, seen: &mut HashMap<(u32, u32), (bool, Option<bool>)>) {
        let v = self.v;
        for i in root..root + v.size(root) {
            if v.kind(i) == Kind::Call {
                let (body, k) = self.bodies[&v.w(i, 3)];
                if k == INLINE {
                    let bp = v.p(body);
                    let at = base + region;
                    self.scan(v.child(body, 0), at, crate::vector::region(bp[2], M(bp[4])), seen);
                    for a in 0..bp[6] as usize {
                        let (off, m) = (bp[7 + 2 * a], M(bp[8 + 2 * a]));
                        let e = seen.entry((at + off, m.bytes())).or_insert((true, Some(m.float())));
                        if m.is_agg() {
                            e.0 = false;
                        }
                    }
                }
                continue;
            }
            if v.kind(i) != Kind::Local {
                continue;
            }
            let p = v.p(i);
            let (off, size, inner) = (p[0] + base, p[1], p[2]);
            let parent_ok = self.whole_access(i);
            let e = seen.entry((off, size)).or_insert((true, None));
            if inner != 0 || parent_ok.is_none() {
                e.0 = false;
            }
            if let Some(m) = parent_ok {
                if m.bytes() != size || m.is_agg() {
                    e.0 = false;
                }
                match e.1 {
                    None => e.1 = Some(m.float()),
                    Some(f) if f != m.float() => e.0 = false,
                    _ => {}
                }
            }
        }
    }

    /// If `Local` node `i` is the address of its parent's whole access (a `Load`, `Store` or `Update` of it, as its
    /// first child): the access's type.
    fn whole_access(&self, i: usize) -> Option<M> {
        let v = self.v;
        // The parent: the nearest node before `i` whose subtree holds it, `i` its first child.
        if i == 0 {
            return None;
        }
        let par = i - 1;
        if par + v.size(par) <= i {
            return None;
        }
        match v.kind(par) {
            Kind::Load => Some(M(v.w(par, 0))),
            Kind::Store => Some(M(v.w(par, 0))),
            Kind::Update => Some(M(v.w(par, 1))),
            _ => None,
        }
    }

    // The blocks and values.

    fn block(&mut self) -> u32 {
        self.blocks.push(Block::default());
        self.preds.push(Vec::new());
        self.sealed.push(false);
        self.ended.push(false);
        (self.blocks.len() - 1) as u32
    }

    fn push(&mut self, i: Inst) -> V {
        let x = self.insts.len() as V;
        let st = self.stable_of(&i);
        self.insts.push(i);
        self.stable.push(st);
        self.at.push(self.cur);
        self.blocks[self.cur as usize].insts.push(x);
        x
    }

    /// A value made once, in the entry block (before its terminator, which a later one is put before).
    fn once(&mut self, op: O, m: M, a: u32, b: u32) -> V {
        let key = (op, m.0, a, b);
        if let Some(&x) = self.consts.get(&key) {
            return x;
        }
        let x = self.insts.len() as V;
        let mut i = Inst::new(op, m, a, b, 0, 0);
        i.shape = match op {
            O::Entry | O::NoLanes => Shape::Mask,
            _ => Shape::Uniform,
        };
        self.insts.push(i);
        self.stable.push(true);
        self.at.push(0);
        let b0 = &mut self.blocks[0].insts;
        // The entry block's values made once go first.
        let at = b0.iter().position(|&y| !matches!(self.insts[y as usize].op, O::Const | O::Entry | O::NoLanes | O::Undef | O::Arg | O::Global | O::Shared)).unwrap_or(b0.len());
        b0.insert(at, x);
        self.consts.insert(key, x);
        x
    }

    fn konst(&mut self, m: M, x: u64) -> V {
        self.once(O::Const, m, x as u32, (x >> 32) as u32)
    }

    fn undef(&mut self, m: M) -> V {
        self.once(O::Undef, m, 0, 0)
    }

    fn none(&mut self) -> V {
        self.once(O::NoLanes, M::VOID, 0, 0)
    }

    fn ins(&mut self, op: O, m: M, a: u32, b: u32, c: u32, d: u32) -> V {
        self.push(Inst::new(op, m, a, b, c, d))
    }

    fn ins_sub(&mut self, op: O, sub: u32, m: M, a: u32, b: u32, c: u32, d: u32) -> V {
        let mut i = Inst::new(op, m, a, b, c, d);
        i.sub = sub as u8;
        self.push(i)
    }

    /// Value `x`'s shape set (as it is made).
    fn shaped(&mut self, x: V, s: Shape) -> V {
        self.insts[x as usize].shape = s;
        let i = self.insts[x as usize];
        self.stable[x as usize] = self.stable_of(&i);
        x
    }

    /// Whether instruction `i` is uniform whatever any block parameter turns out to be.
    fn stable_of(&self, i: &Inst) -> bool {
        match i.op {
            O::Const | O::Global | O::Shared | O::GroupInfo | O::Any | O::AnyGroup | O::Bits | O::Reduce | O::First | O::Agree => true,
            O::Arg | O::Atomic => i.shape == Shape::Uniform,
            O::Bin => self.is_uniform(i.a) && self.is_uniform(i.b),
            O::Un | O::Cast | O::LaneOp => i.uses().all(|u| self.is_uniform(u)),
            O::Load => self.is_uniform(i.a),
            _ => false,
        }
    }

    /// Whether `x` is uniform for good (`stable`).
    /// Whether mask `m` has at most one lane of its SIMD group: made of `lane == u`'s (`lane_is`).
    fn single(&self, m: V) -> bool {
        let i = self.insts[self.resolve(m) as usize];
        match i.op {
            O::NoLanes => true,
            O::MaskOf => lane_is(&self.insts, self.resolve(i.a)).is_some(),
            O::MAnd => self.single(i.a) || self.single(i.b),
            O::MAndNot => self.single(i.a),
            _ => false,
        }
    }

    fn is_uniform(&self, x: V) -> bool {
        let x = self.resolve(x);
        self.stable[x as usize] && self.insts[x as usize].shape == Shape::Uniform
    }

    fn resolve(&self, mut x: V) -> V {
        while let Some(&y) = self.repl.get(&x) {
            x = y;
        }
        x
    }

    fn shape(&self, x: V) -> Shape {
        self.insts[self.resolve(x) as usize].shape
    }

    fn m_of(&self, x: V) -> M {
        self.insts[self.resolve(x) as usize].m
    }

    // The terminators.

    fn jump(&mut self, to: u32) {
        if self.ended[self.cur as usize] {
            return;
        }
        let from = self.cur;
        self.ins(O::Jump, M::VOID, to, 0, 0, 0);
        self.ended[from as usize] = true;
        self.preds[to as usize].push(from);
    }

    /// A branch on uniform `c` to two new blocks (each with this one its one predecessor), sealed.
    fn br(&mut self, c: V) -> (u32, u32) {
        let (t, e) = (self.block(), self.block());
        let from = self.cur;
        let mask = self.cur();
        self.ins(O::Br, M::VOID, c, t, e, mask);
        self.ended[from as usize] = true;
        self.preds[t as usize].push(from);
        self.preds[e as usize].push(from);
        self.seal(t);
        self.seal(e);
        (t, e)
    }

    /// A branch on uniform `c` to blocks `t` and `e`, which other branches may reach too (sealed by the caller): a
    /// branch's own blocks (a branch's targets have no other predecessor), each a jump on.
    fn br_to(&mut self, c: V, t: u32, e: u32) {
        let (a, b) = self.br(c);
        self.switch_to(a);
        self.jump(t);
        self.switch_to(b);
        self.jump(e);
    }

    /// A uniform condition `n` (its level the SIMD group's or below) as branches to `t` where it is true and `e`
    /// where it is false: an `&&` or `||` a branch on each side, no truth value made of them to branch on again.
    fn cond_to(&mut self, n: usize, t: u32, e: u32) -> R<()> {
        let v = self.v;
        let l = self.logic_of(n);
        if v.kind(l) == Kind::Logic && !self.exits[n] {
            let n = l;
            let and = v.w(n, 0) != 0;
            let kids: Vec<usize> = v.children(n).collect();
            let mid = self.block();
            if and { self.cond_to(kids[0], mid, e)? } else { self.cond_to(kids[0], t, mid)? }
            self.seal(mid);
            self.switch_to(mid);
            return self.cond_to(kids[1], t, e);
        }
        let c = self.cond(n)?;
        let c = self.scalar(c);
        self.br_to(c, t, e);
        Ok(())
    }

    /// The `&&` or `||` that `n` is, through casts (of its 0 or 1, which keep its truth), else `n`.
    fn logic_of(&self, n: usize) -> usize {
        let v = self.v;
        let mut x = n;
        while v.kind(x) == Kind::Cast {
            let Some(k) = v.children(x).next() else { return n };
            x = k;
        }
        if v.kind(x) == Kind::Logic { x } else { n }
    }

    fn switch_to(&mut self, b: u32) {
        self.cur = b;
    }

    // Variables (Braun et al.).

    fn write(&mut self, var: u64, x: V) {
        self.defs.insert((self.cur, var), x);
    }

    fn read(&mut self, var: u64) -> V {
        let b = self.cur;
        self.read_in(var, b)
    }

    fn read_in(&mut self, var: u64, b: u32) -> V {
        // Up a chain of blocks with one predecessor each, without recursion (a kernel's chains are long).
        let mut chain = Vec::new();
        let mut x = b;
        let val = loop {
            if let Some(&d) = self.defs.get(&(x, var)) {
                break self.resolve(d);
            }
            if !self.sealed[x as usize] {
                let p = self.param(x, var);
                self.incomplete.entry(x).or_default().push((var, p));
                self.defs.insert((x, var), p);
                break p;
            }
            match self.preds[x as usize].len() {
                1 => {
                    chain.push(x);
                    x = self.preds[x as usize][0];
                }
                0 => {
                    // The entry block: a variable never written is undefined (a mask: none; the lanes running: the
                    // entry's).
                    let d = if var == CUR {
                        self.entry
                    } else if self.is_mask_var(var) {
                        self.none()
                    } else {
                        let m = self.var_type(var);
                        self.undef(m)
                    };
                    self.defs.insert((x, var), d);
                    break d;
                }
                _ => {
                    let p = self.param(x, var);
                    self.defs.insert((x, var), p);
                    let d = self.phi_operands(p);
                    self.defs.insert((x, var), d);
                    break d;
                }
            }
        };
        for c in chain {
            self.defs.insert((c, var), val);
        }
        val
    }

    fn is_mask_var(&self, var: u64) -> bool {
        var <= END || (var != RESULT && self.var_m.get(&var) == Some(&M::VOID))
    }

    fn var_type(&self, var: u64) -> M {
        if var <= END { M::VOID } else { *self.var_m.get(&var).unwrap_or(&M::U64) }
    }

    fn param(&mut self, b: u32, var: u64) -> V {
        let x = self.insts.len() as V;
        let m = self.var_type(var);
        let k = self.blocks[b as usize].params.len() as u32;
        let mut i = Inst::new(O::Param, m, k, 0, 0, 0);
        i.shape = if self.is_mask_var(var) { Shape::Mask } else { Shape::Uniform };
        self.insts.push(i);
        self.stable.push(false);
        self.at.push(b);
        self.blocks[b as usize].params.push(x);
        self.phis.insert(x, Phi { block: b, var, ops: Vec::new() });
        x
    }

    fn phi_operands(&mut self, p: V) -> V {
        let (b, var) = {
            let ph = &self.phis[&p];
            (ph.block, ph.var)
        };
        let preds = self.preds[b as usize].clone();
        let mut ops = Vec::with_capacity(preds.len());
        for q in preds {
            ops.push(self.read_in(var, q));
        }
        for &o in &ops {
            self.phi_users.entry(o).or_default().push(p);
        }
        self.phis.get_mut(&p).expect("a phi").ops = ops;
        self.try_trivial(p)
    }

    /// Phi `p` replaced by its one operand, if it has one other than itself.
    fn try_trivial(&mut self, p: V) -> V {
        let ops = self.phis[&p].ops.clone();
        let mut same = NONE;
        for o in ops {
            let o = self.resolve(o);
            if o == same || o == p {
                continue;
            }
            if same != NONE {
                return p;
            }
            same = o;
        }
        if same == NONE {
            let m = self.insts[p as usize].m;
            let var = self.phis[&p].var;
            same = if self.is_mask_var(var) { self.none() } else { self.undef(m) };
        }
        self.repl.insert(p, same);
        // The phis that used it may be trivial now.
        let users = self.phi_users.remove(&p).unwrap_or_default();
        for q in users {
            if q == p || self.repl.contains_key(&q) {
                continue;
            }
            if let Some(ph) = self.phis.get_mut(&q) {
                for o in &mut ph.ops {
                    if *o == p {
                        *o = same;
                    }
                }
            }
            self.phi_users.entry(same).or_default().push(q);
            self.try_trivial(q);
        }
        same
    }

    fn seal(&mut self, b: u32) {
        if self.sealed[b as usize] {
            return;
        }
        self.sealed[b as usize] = true;
        for (_, p) in self.incomplete.remove(&b).unwrap_or_default() {
            self.phi_operands(p);
        }
    }

    // Masks.

    fn cur(&mut self) -> V {
        self.read(CUR)
    }

    fn set_cur(&mut self, m: V) {
        self.write(CUR, m);
    }

    fn mask_op(&mut self, op: O, a: V, b: V) -> V {
        let (a, b) = (self.resolve(a), self.resolve(b));
        let none = self.none();
        match op {
            O::MAnd if a == none || b == none => return none,
            O::MAnd if a == b => return a,
            O::MOr if a == none => return b,
            O::MOr if b == none || a == b => return a,
            O::MAndNot if b == none || a == none => return a,
            O::MAndNot if a == b => return none,
            _ => {}
        }
        let x = self.ins(op, M::VOID, a, b, 0, 0);
        self.shaped(x, Shape::Mask);
        x
    }

    /// `var |= CUR; CUR = none` (a `break`, `continue`, `return`).
    fn exit(&mut self, var: u64) {
        let (c, x) = (self.cur(), self.read(var));
        let y = self.mask_op(O::MOr, x, c);
        self.write(var, y);
        let none = self.none();
        self.set_cur(none);
    }

    /// `CUR = m & !(the exits in `vars`)`.
    fn rejoin(&mut self, m: V, vars: &[u64]) {
        let mut x = m;
        for &var in vars {
            let e = self.read(var);
            x = self.mask_op(O::MAndNot, x, e);
        }
        self.set_cur(x);
    }

    /// Whether `CUR` is the function's entry's lanes.
    fn full(&mut self) -> bool {
        let c = self.cur();
        c == self.entry
    }

    /// `any(CUR)` as a uniform `bool`, or none when it is the entry's (never empty).
    fn any_cur(&mut self) -> Option<V> {
        let c = self.cur();
        if c == self.entry {
            return None;
        }
        if c == self.none() {
            return Some(self.konst(M::B, 0));
        }
        let x = self.ins(O::Any, M::B, c, 0, 0, 0);
        self.shaped(x, Shape::Uniform);
        Some(x)
    }

    /// `body` run where `CUR` has lanes; then on.
    fn guarded(&mut self, body: impl FnOnce(&mut Self) -> R<()>) -> R<()> {
        match self.any_cur() {
            None => body(self),
            Some(c) => {
                let (t, e) = self.br(c);
                let join = self.block();
                self.switch_to(t);
                body(self)?;
                self.jump(join);
                self.switch_to(e);
                self.jump(join);
                self.seal(join);
                self.switch_to(join);
                Ok(())
            }
        }
    }

    /// Variable `var` set to `x` in the lanes running (all of its lanes where they are the entry's, or it had no
    /// value).
    fn assign(&mut self, var: u64, x: V) {
        let old = self.read(var);
        let full = self.full();
        let undef = self.insts[old as usize].op == O::Undef;
        let y = if full || undef {
            x
        } else {
            let c = self.cur();
            let m = self.var_type(var);
            let s = self.ins(O::Select, m, c, x, old, 0);
            self.shaped(s, Shape::Lanes);
            s
        };
        self.write(var, y);
    }

    fn temp(&mut self, m: M) -> u64 {
        self.temps += 1;
        let var = TEMP + self.temps;
        self.var_m.insert(var, m);
        var
    }

    // The function.

    fn function(mut self, k: u32, name: String, region: u32, args: &[(u32, M)], uni: &[bool], ret: M, root: usize) -> R<Func> {
        self.region = region;
        let b0 = self.block();
        self.seal(b0);
        self.switch_to(b0);
        self.entry = self.once(O::Entry, M::VOID, 0, 0);
        let entry = self.entry;
        self.write(CUR, entry);
        let mut arg_shapes = Vec::new();
        for (a, &(off, m)) in args.iter().enumerate() {
            let m = if m.is_agg() { M::PTR } else { m };
            let shape = if self.kernel || uni.get(a).copied().unwrap_or(false) { Shape::Uniform } else { Shape::Lanes };
            arg_shapes.push((m, shape));
            let x = self.ins(O::Arg, m, a as u32, 0, 0, 0);
            self.shaped(x, shape);
            let am = args[a].1;
            if self.promoted.contains(&(off, am.bytes())) {
                let var = local(off, am.bytes());
                self.var_m.insert(var, am);
                self.write(var, x);
            } else if self.kernel {
                // The kernel's arguments are in its frame already (`run`'s).
            } else if am.is_agg() {
                let to = self.frame_addr(off * W as u32, am.bytes());
                let c = self.cur();
                let mut i = Inst::new(O::Copy, M(am.bytes()), to, x, c, 0);
                i.shape = Shape::None;
                self.push(i);
            } else {
                let to = self.frame_addr(off * W as u32, am.bytes());
                let c = self.cur();
                self.ins(O::Store, am, to, x, c, 0);
            }
        }
        if ret != M::VOID && !ret.is_agg() {
            self.var_m.insert(RESULT, ret);
        }
        self.base = 0;
        self.stmt(root)?;
        if self.kernel {
            self.ins(O::Halt, M::VOID, 0, 0, 0, 0);
        } else {
            let r = if ret == M::VOID || ret.is_agg() { NONE } else { self.read(RESULT) };
            self.ins(O::Ret, M::VOID, r, 0, 0, 0);
        }
        let cur = self.cur;
        self.ended[cur as usize] = true;
        self.finish(k, name, region, arg_shapes, ret)
    }

    // Statements: entered with lanes running (`CUR` not empty), as the code tape's.

    fn stmt(&mut self, n: usize) -> R<()> {
        let v = self.v;
        // Only a block's and a uniform branch's sides' ends may go past the statements that follow (`dead`): a
        // loop's, a switch's, a call's, a masked branch's join them.
        let skips = self.skips;
        if !matches!(v.kind(n), Kind::Block | Kind::If) {
            self.skips = false;
        }
        let r = self.stmt_of(n);
        self.skips = skips;
        r
    }

    fn stmt_of(&mut self, n: usize) -> R<()> {
        let v = self.v;
        if self.insts.len() > MAX_MADE {
            return Err(Error(format!("over {MAX_MADE} values made in one function: too large to compile")));
        }
        match v.kind(n) {
            Kind::Block => {
                let kids: Vec<usize> = v.children(n).collect();
                self.stmts(&kids)?;
            }
            Kind::Expr => {
                self.expr(v.child(n, 0))?;
            }
            Kind::Empty => {}
            Kind::If => self.branch(n)?,
            Kind::While | Kind::For | Kind::Do => self.loop_(n)?,
            Kind::Switch => self.switch(n)?,
            Kind::Return => {
                let p = v.p(n);
                let (has, slot, m) = (p[0] != 0, p[2], M(p[3]));
                let x = if has { self.expr(v.child(n, 0))? } else { None };
                if slot == NO_SLOT {
                    self.exit(END);
                } else {
                    if let Some(x) = x {
                        if m.is_agg() {
                            let to = self.frame_addr((self.base + slot) * W as u32, m.bytes());
                            let c = self.cur();
                            self.ins(O::Copy, M(m.bytes()), to, x, c, 0);
                        } else {
                            let x = self.fit(x, m);
                            let res = self.res;
                            self.assign(res, x);
                        }
                    }
                    let rv = self.rv;
                    self.exit(rv);
                }
            }
            Kind::Break => self.exit(BRK),
            Kind::Continue => self.exit(CONT),
            _ => {
                self.expr(n)?;
            }
        }
        Ok(())
    }

    /// Statements in order, those past one that may exit run only where lanes remain.
    fn stmts(&mut self, kids: &[usize]) -> R<()> {
        let outer = self.skips;
        for (k, &c) in kids.iter().enumerate() {
            let more = k + 1 < kids.len();
            // The last statement's exits skip what the list's do.
            self.skips = if more { self.exits[c] } else { outer };
            let mark = self.dead.len();
            self.stmt(c)?;
            if self.exits[c] && more {
                self.skips = outer;
                let dead: Vec<u32> = self.dead.drain(mark..).collect();
                let rest = &kids[k + 1..];
                return self.skipped(dead, |g| g.stmts(rest));
            }
        }
        self.skips = outer;
        Ok(())
    }

    /// `body` (the statements past one that may exit), where lanes remain (`guarded`), and the ends `dead` whose
    /// lanes have all left on past it.
    fn skipped(&mut self, dead: Vec<u32>, body: impl FnOnce(&mut Self) -> R<()>) -> R<()> {
        if dead.is_empty() {
            return self.guarded(body);
        }
        let join = self.block();
        self.guarded(body)?;
        self.jump(join);
        for d in dead {
            self.switch_to(d);
            self.jump(join);
        }
        self.seal(join);
        self.switch_to(join);
        Ok(())
    }

    /// A condition's value, a `bool` (uniform or lanes).
    fn cond(&mut self, n: usize) -> R<V> {
        let c = self.value(n)?;
        let m = self.m_of(c);
        Ok(if m == M::B { c } else { self.cast(c, m, M::B) })
    }

    /// A checked branch's decision `c` (uniform), agreed by the group: the value its branch takes (the group's, the
    /// same as `c` wherever the code runs the SIMD group's lanes at once).
    fn agree(&mut self, n: usize, site: u32, c: V) -> V {
        let m = self.insts[c as usize].m;
        let mask = self.cur();
        let x = self.ins(O::Agree, m, n as u32, site, c, mask);
        self.shaped(x, Shape::Uniform)
    }

    fn mask_of(&mut self, c: V) -> V {
        let x = self.ins(O::MaskOf, M::B, c, 0, 0, 0);
        self.shaped(x, Shape::Mask);
        x
    }

    fn branch(&mut self, n: usize) -> R<()> {
        let v = self.v;
        let site = v.w(n, 0);
        let check = v.field(n) & CHECK != 0;
        let kids: Vec<usize> = v.children(n).collect();
        let direct = !check && v.kind(self.logic_of(kids[0])) == Kind::Logic && v.level(kids[0]) <= Level::SimdGroup;
        let c = if direct { NONE } else { self.cond(kids[0])? };
        let c = if !direct && v.level(kids[0]) <= Level::SimdGroup { self.scalar(c) } else { c };
        if direct || self.is_uniform(c) {
            let skips = self.skips;
            let (t, e) = if direct {
                let (t, e) = (self.block(), self.block());
                self.cond_to(kids[0], t, e)?;
                self.seal(t);
                self.seal(e);
                (t, e)
            } else {
                let c = if check { self.agree(n, site, c) } else { c };
                self.br(c)
            };
            let join = self.block();
            // A side whose lanes have all left goes past the statements that follow, where they do (`dead`); one
            // side at least joins.
            self.switch_to(t);
            self.stmt(kids[1])?;
            let t_dead = skips && self.cur() == self.none();
            let t_end = self.cur;
            if !t_dead {
                self.jump(join);
            }
            self.switch_to(e);
            if kids.len() > 2 {
                self.stmt(kids[2])?;
            }
            if skips && !t_dead && self.cur() == self.none() {
                self.dead.push(self.cur);
            } else {
                self.jump(join);
            }
            if t_dead {
                self.dead.push(t_end);
            }
            self.seal(join);
            self.switch_to(join);
            return Ok(());
        }
        let s0 = self.cur();
        let mc = self.mask_of(c);
        let t = self.mask_op(O::MAnd, s0, mc);
        let e = self.mask_op(O::MAndNot, s0, mc);
        if check {
            let d = self.ins(O::AnyGroup, M::B, t, 0, 0, 0);
            self.shaped(d, Shape::Uniform);
            self.ins(O::Agree, M::VOID, n as u32, site, d, 0);
        }
        self.set_cur(t);
        self.skips = false;
        self.guarded(|g| g.stmt(kids[1]))?;
        if kids.len() > 2 {
            self.set_cur(e);
            self.guarded(|g| g.stmt(kids[2]))?;
        }
        self.set_cur(s0);
        if self.exits[n] {
            let c = self.cur();
            { let rv = self.rv; self.rejoin(c, &[BRK, CONT, rv, END]); }
        }
        Ok(())
    }

    fn switch(&mut self, n: usize) -> R<()> {
        let v = self.v;
        let (site, m) = (v.w(n, 0), M(v.w(n, 1)));
        let check = v.field(n) & CHECK != 0;
        let kids: Vec<usize> = v.children(n).collect();
        let c = self.value(kids[0])?;
        let c = self.fit(c, m);
        let s0 = self.cur();
        let sb = self.read(BRK);
        let none = self.none();
        self.write(BRK, none);
        let cases = &kids[1..];
        if self.is_uniform(c) || check || v.level(kids[0]) <= Level::SimdGroup {
            let c = self.scalar(c);
            let c = if check { self.agree(n, site, c) } else { c };
            self.cases(c, m, cases)?;
        } else {
            // A round for each value the running lanes have.
            let rest = self.temp(M::VOID);
            self.write(rest, s0);
            let top = self.block();
            self.jump(top);
            self.switch_to(top);
            let r = self.read_mask(rest);
            let any = self.ins(O::Any, M::B, r, 0, 0, 0);
            self.shaped(any, Shape::Uniform);
            let (go, done) = self.br(any);
            self.switch_to(go);
            let r = self.read_mask(rest);
            let x = self.ins(O::First, m, c, r, 0, 0);
            self.shaped(x, Shape::Uniform);
            let eq = self.ins_sub(O::Bin, Op::Eq as u32, M::B, c, x, NONE, m.0);
            self.shaped(eq, Shape::Lanes);
            let me = self.mask_of(eq);
            let now = self.mask_op(O::MAnd, r, me);
            let left = self.mask_op(O::MAndNot, r, now);
            self.write(rest, left);
            self.set_cur(now);
            self.cases(x, m, cases)?;
            self.jump(top);
            self.seal(top);
            self.switch_to(done);
        }
        { let rv = self.rv; self.rejoin(s0, &[CONT, rv, END]); }
        self.write(BRK, sb);
        Ok(())
    }

    /// A mask temporary's value.
    fn read_mask(&mut self, var: u64) -> V {
        self.read(var)
    }

    /// A `Switch` by uniform `x` (of `m`) to `cases`, then the cases in order, each from its own lanes or the case
    /// before's, unless none are left.
    fn cases(&mut self, x: V, m: M, cases: &[usize]) -> R<()> {
        let v = self.v;
        let end = self.block();
        let starts: Vec<u32> = cases.iter().map(|_| self.block()).collect();
        // The `Switch`'s targets: a block of its own each (one predecessor), jumping to its case's start.
        let from = self.cur;
        let mut labels = Vec::new();
        let mut default = None;
        let mut trampolines = Vec::new();
        for (k, &c) in cases.iter().enumerate() {
            let t = self.block();
            trampolines.push((t, starts[k]));
            if v.w(c, 0) != 0 {
                default = Some(t);
            } else {
                labels.push((crate::ops::norm(m, u64::from(v.w(c, 1)) | u64::from(v.w(c, 2)) << 32), t));
            }
        }
        let dflt = default.unwrap_or_else(|| {
            let t = self.block();
            trampolines.push((t, end));
            t
        });
        labels.sort_unstable();
        let at = self.extra.len() as u32;
        for &(val, t) in &labels {
            self.extra.extend_from_slice(&[val as u32, (val >> 32) as u32, t]);
        }
        self.extra.push(dflt);
        let mask = self.cur();
        self.ins(O::Switch, M::VOID, x, at, labels.len() as u32, mask);
        self.ended[from as usize] = true;
        for &(t, _) in &trampolines {
            self.preds[t as usize].push(from);
            self.seal(t);
        }
        for &(t, to) in &trampolines {
            self.switch_to(t);
            self.jump(to);
        }
        for (k, &c) in cases.iter().enumerate() {
            self.seal(starts[k]);
            self.switch_to(starts[k]);
            match self.any_cur() {
                None => self.stmt(v.child(c, 0))?,
                Some(a) => {
                    let (go, skip) = self.br(a);
                    self.switch_to(skip);
                    self.jump(end);
                    self.switch_to(go);
                    self.stmt(v.child(c, 0))?;
                }
            }
            let next = if k + 1 < cases.len() { starts[k + 1] } else { end };
            self.jump(next);
        }
        self.seal(end);
        self.switch_to(end);
        Ok(())
    }

    fn loop_(&mut self, n: usize) -> R<()> {
        let v = self.v;
        let k = v.kind(n);
        let site = v.w(n, 0);
        let check = v.field(n) & CHECK != 0;
        let kids: Vec<usize> = v.children(n).collect();
        let (cond, body) = if k == Kind::Do { (kids[1], kids[0]) } else { (kids[0], kids[1]) };
        let sl = self.cur();
        let (sb, sc) = (self.read(BRK), self.read(CONT));
        let none = self.none();
        self.write(BRK, none);
        let top = self.block();
        let first = self.block();
        let exit = self.block();
        self.jump(if k == Kind::Do { first } else { top });
        // The test: lanes left, then the condition.
        self.switch_to(top);
        match self.any_cur() {
            None => {}
            Some(a) => {
                let (go, out) = self.br(a);
                self.switch_to(out);
                self.jump(exit);
                self.switch_to(go);
            }
        }
        let c = self.cond(cond)?;
        let c = if v.level(cond) <= Level::SimdGroup { self.scalar(c) } else { c };
        if self.is_uniform(c) {
            let c = if check { self.agree(n, site, c) } else { c };
            let (go, out) = self.br(c);
            self.switch_to(out);
            self.jump(exit);
            self.switch_to(go);
            self.jump(first);
        } else {
            let cur = self.cur();
            let mc = self.mask_of(c);
            let now = self.mask_op(O::MAnd, cur, mc);
            self.set_cur(now);
            if check {
                let d = self.ins(O::AnyGroup, M::B, now, 0, 0, 0);
                self.shaped(d, Shape::Uniform);
                self.ins(O::Agree, M::VOID, n as u32, site, d, 0);
            }
            let a = self.ins(O::Any, M::B, now, 0, 0, 0);
            self.shaped(a, Shape::Uniform);
            let (go, out) = self.br(a);
            self.switch_to(out);
            self.jump(exit);
            self.switch_to(go);
            self.jump(first);
        }
        self.seal(first);
        self.switch_to(first);
        let si = self.cur();
        self.write(CONT, none);
        self.stmt(body)?;
        { let rv = self.rv; self.rejoin(si, &[BRK, rv, END]); }
        if k == Kind::For && v.kind(kids[2]) != Kind::Empty {
            match self.any_cur() {
                None => self.stmt(kids[2])?,
                Some(a) => {
                    let (go, out) = self.br(a);
                    self.switch_to(out);
                    self.jump(exit);
                    self.switch_to(go);
                    self.stmt(kids[2])?;
                }
            }
        }
        self.jump(top);
        self.seal(top);
        self.seal(exit);
        self.switch_to(exit);
        { let rv = self.rv; self.rejoin(sl, &[rv, END]); }
        self.write(BRK, sb);
        self.write(CONT, sc);
        Ok(())
    }

    // Expressions.

    fn value(&mut self, n: usize) -> R<V> {
        self.expr(n)?.ok_or_else(|| Error(format!("a {:?} node has no value", self.v.kind(n))))
    }

    /// Whether node `n`'s value is computed once for its SIMD group (as the code tape's `once`).
    fn once_node(&self, n: usize) -> bool {
        self.v.level(n) <= Level::SimdGroup && self.v.field(n) & crate::k::EFFECT == 0
    }

    /// `x` as a uniform value (the first running lane's, if it is lanes).
    fn scalar(&mut self, x: V) -> V {
        if self.is_uniform(x) {
            return x;
        }
        let m = self.m_of(x);
        let c = self.cur();
        let y = self.ins(O::First, m, x, c, 0, 0);
        self.shaped(y, Shape::Uniform);
        y
    }

    /// `x` as a value of `m` (an integer's width and sign).
    fn fit(&mut self, x: V, m: M) -> V {
        let from = self.m_of(x);
        if from == m || m.is_agg() || from.is_agg() {
            return x;
        }
        self.cast(x, from, m)
    }

    fn cast(&mut self, x: V, from: M, to: M) -> V {
        if from == to {
            return x;
        }
        let s = self.shape(x);
        let y = self.ins(O::Cast, to, x, from.0, 0, 0);
        self.shaped(y, s);
        y
    }

    fn frame_addr(&mut self, base: u32, stride: u32) -> V {
        let x = self.ins(O::FrameAddr, M::PTR, base, stride, 0, 0);
        self.shaped(x, Shape::Lanes);
        x
    }

    /// If node `a` addresses a lane's own variable directly (a `Local`, plus constant offsets): its offset, the
    /// variable's size, the offset into it.
    fn direct(&self, a: usize) -> Option<(u32, u32, u32)> {
        let v = self.v;
        match v.kind(a) {
            Kind::Local => {
                let p = v.p(a);
                Some((p[0] + self.base, p[1], p[2]))
            }
            Kind::Bin if matches!(Op::of(v.w(a, 0)), Op::Add | Op::Sub) && M(v.w(a, 1)) == M::PTR => {
                let (off, size, inner) = self.direct(v.child(a, 0))?;
                let c = constant(v, v.child(a, 1))? as u32;
                Some((off, size, if Op::of(v.w(a, 0)) == Op::Add { inner.wrapping_add(c) } else { inner.wrapping_sub(c) }))
            }
            _ => None,
        }
    }

    /// The variable kept as values that node `a` addresses whole, if it does.
    fn kept(&self, a: usize) -> Option<u64> {
        if self.v.kind(a) != Kind::Local {
            return None;
        }
        let p = self.v.p(a);
        let o = p[0] + self.base;
        (p[2] == 0 && self.promoted.contains(&(o, p[1]))).then_some(local(o, p[1]))
    }

    /// The address node `a` gives (a frame variable's directly).
    fn address(&mut self, a: usize) -> R<V> {
        if let Some((off, size, inner)) = self.direct(a) {
            return Ok(self.frame_addr(off * W as u32 + inner, size));
        }
        self.value(a)
    }

    fn load(&mut self, m: M, addr: V, once: bool) -> V {
        let c = self.cur();
        let x = self.ins(O::Load, m, addr, c, 0, 0);
        let s = self.shape(addr);
        self.insts[x as usize].shape = if s == Shape::Uniform { Shape::Uniform } else { Shape::Lanes };
        if once { self.scalar(x) } else { x }
    }

    fn store(&mut self, m: M, addr: V, x: V) {
        let c = self.cur();
        self.ins(O::Store, m, addr, x, c, 0);
    }

    /// A kept variable's value as `m` (its writes' type may differ in sign or width of the same bytes).
    fn read_var(&mut self, var: u64, m: M) -> V {
        let x = self.read(var);
        let vm = self.var_type(var);
        if vm == m { x } else { self.cast(x, vm, m) }
    }

    fn write_var(&mut self, var: u64, m: M, x: V) {
        self.var_m.entry(var).or_insert(m);
        let vm = self.var_type(var);
        let x = self.fit(x, m);
        let x = if vm == m { x } else { self.cast(x, m, vm) };
        self.assign(var, x);
    }

    fn expr(&mut self, n: usize) -> R<Option<V>> {
        let v = self.v;
        let p = v.p(n);
        let once = self.once_node(n);
        let kids: Vec<usize> = v.children(n).collect();
        Ok(Some(match v.kind(n) {
            Kind::Const => {
                let m = M(p[0]);
                self.konst(if m.is_agg() { M::PTR } else { m }, u64::from(p[1]) | u64::from(p[2]) << 32)
            }
            Kind::Global => self.once(O::Global, M::PTR, p[0], 0),
            Kind::Shared => self.once(O::Shared, M::PTR, p[0], 0),
            Kind::Local => self.frame_addr((p[0] + self.base) * W as u32 + p[2], p[1]),
            Kind::Load => {
                let m = M(p[0]);
                if m.is_agg() {
                    return self.expr(kids[0]);
                }
                if let Some(var) = self.kept(kids[0]) {
                    let x = self.read_var(var, m);
                    if once { self.scalar(x) } else { x }
                } else {
                    let a = self.address(kids[0])?;
                    self.load(m, a, once)
                }
            }
            Kind::Store => {
                let m = M(p[0]);
                if let Some(var) = self.kept(kids[0]) {
                    let x = self.value(kids[1])?;
                    let x = self.fit(x, m);
                    self.write_var(var, m, x);
                    x
                } else {
                    let a = self.address(kids[0])?;
                    let x = self.value(kids[1])?;
                    let x = self.fit(x, m);
                    self.store(m, a, x);
                    x
                }
            }
            Kind::Update => {
                let (op, lm, om, post) = (Op::of(p[0]), M(p[1]), M(p[2]), p[3] != 0);
                let kept = self.kept(kids[0]);
                let a = match kept {
                    Some(_) => NONE,
                    None => self.address(kids[0])?,
                };
                let y = self.value(kids[1])?;
                let old = match kept {
                    Some(var) => self.read_var(var, lm),
                    None => self.load(lm, a, false),
                };
                let o = self.cast(old, lm, om);
                let y = self.fit(y, om);
                let r = self.bin(op, om, o, y);
                let new = self.cast(r, om, lm);
                match kept {
                    Some(var) => self.write_var(var, lm, new),
                    None => self.store(lm, a, new),
                }
                if post { old } else { new }
            }
            Kind::Zero => {
                let a = self.value(kids[0])?;
                let c = self.cur();
                self.ins(O::Zero, M(p[1]), a, c, 0, 0);
                a
            }
            Kind::Copy => {
                let d = self.value(kids[0])?;
                let s = self.value(kids[1])?;
                let c = self.cur();
                self.ins(O::Copy, M(p[0]), d, s, c, 0);
                d
            }
            Kind::Bin => {
                let (op, m) = (Op::of(p[0]), M(p[1]));
                let a = self.value(kids[0])?;
                let b = self.value(kids[1])?;
                let (a, b) = (self.fit(a, m), self.fit(b, m));
                let (a, b) = if once { (self.scalar(a), self.scalar(b)) } else { (a, b) };
                self.bin(op, m, a, b)
            }
            Kind::Un => {
                let (op, m) = (Op::of(p[0]), M(p[1]));
                let a = self.value(kids[0])?;
                let a = self.fit(a, m);
                let a = if once { self.scalar(a) } else { a };
                let s = self.shape(a);
                let x = self.ins_sub(O::Un, op as u32, if op == Op::LNot { M::B } else { m }, a, 0, 0, 0);
                // `!x`'s result is a `bool` of `x`'s type's truth: the operand's type is `m`.
                self.insts[x as usize].d = m.0;
                self.shaped(x, s)
            }
            Kind::Cast => {
                let (from, to) = (M(p[0]), M(p[1]));
                let a = self.value(kids[0])?;
                let a = self.fit(a, from);
                let a = if once { self.scalar(a) } else { a };
                if from.is_agg() || to.is_agg() {
                    a
                } else {
                    self.cast(a, from, to)
                }
            }
            Kind::Comma => {
                self.expr(kids[0])?;
                return self.expr(kids[1]);
            }
            Kind::Cond | Kind::Logic => return self.choose(n, once).map(Some),
            Kind::Builtin => return self.builtin(n, once),
            Kind::Call => return self.call(n),
            Kind::Block | Kind::Expr => {
                self.stmt(n)?;
                return Ok(None);
            }
            k => return Err(Error(format!("a {k:?} node as a value"))),
        }))
    }

    fn bin(&mut self, op: Op, m: M, a: V, b: V) -> V {
        let s = self.shape(a).max(self.shape(b));
        let rm = if op.compares() { M::B } else { m };
        // An integer division's lanes: those that run divide (a lane that does not may hold a 0).
        let mask = if matches!(op, Op::Div | Op::Rem) && !m.float() { self.cur() } else { NONE };
        let x = self.ins_sub(O::Bin, op as u32, rm, a, b, mask, m.0);
        self.shaped(x, s);
        x
    }

    /// `?:`, `&&`, `||`: by a branch on a uniform condition, or each side under its lanes.
    fn choose(&mut self, n: usize, once: bool) -> R<V> {
        let v = self.v;
        let logic = v.kind(n) == Kind::Logic;
        let and = logic && v.w(n, 0) != 0;
        let kids: Vec<usize> = v.children(n).collect();
        let m = if logic { M::B } else { M(v.w(n, 0)) };
        let m = if m.is_agg() { M::PTR } else { m };
        let c = self.cond(kids[0])?;
        let c = if once || v.level(kids[0]) <= Level::SimdGroup { self.scalar(c) } else { c };
        let d = self.temp(m);
        let put = |g: &mut Self, k: usize| -> R<()> {
            let x = g.value(kids[k])?;
            let x = if logic {
                let xm = g.m_of(x);
                g.cast(x, xm, M::B)
            } else {
                g.fit(x, m)
            };
            let x = if once { g.scalar(x) } else { x };
            g.assign(d, x);
            Ok(())
        };
        if self.is_uniform(c) {
            if logic {
                self.write(d, c);
                // `&&`: the second where the first is true; `||`: where it is false.
                let (t, e) = self.br(c);
                let join = self.block();
                let (go, skip) = if and { (t, e) } else { (e, t) };
                self.switch_to(go);
                put(self, 1)?;
                self.jump(join);
                self.switch_to(skip);
                self.jump(join);
                self.seal(join);
                self.switch_to(join);
            } else {
                let (t, e) = self.br(c);
                let join = self.block();
                self.switch_to(t);
                put(self, 1)?;
                self.jump(join);
                self.switch_to(e);
                put(self, 2)?;
                self.jump(join);
                self.seal(join);
                self.switch_to(join);
            }
            return Ok(self.read(d));
        }
        let s0 = self.cur();
        let mc = self.mask_of(c);
        let t = self.mask_op(O::MAnd, s0, mc);
        let e = self.mask_op(O::MAndNot, s0, mc);
        if logic {
            self.write(d, c);
            self.set_cur(if and { t } else { e });
            self.guarded(|g| put(g, 1))?;
        } else {
            self.set_cur(t);
            self.guarded(|g| put(g, 1))?;
            self.set_cur(e);
            self.guarded(|g| put(g, 2))?;
        }
        self.set_cur(s0);
        if self.exits[n] {
            let c = self.cur();
            { let rv = self.rv; self.rejoin(c, &[BRK, CONT, rv, END]); }
        }
        Ok(self.read(d))
    }

    fn builtin(&mut self, n: usize, once: bool) -> R<Option<V>> {
        let v = self.v;
        let p = v.p(n);
        let (b, m) = (Builtin::of(p[0]), M(p[1]));
        let kids: Vec<usize> = v.children(n).collect();
        let mut args = Vec::with_capacity(kids.len());
        for &c in &kids {
            args.push(self.value(c)?);
        }
        let arg_m = |k: usize| if k < kids.len() { M(p[4 + k]) } else { M::VOID };
        let set = |g: &mut Self, x: V, s: Shape| g.shaped(x, s);
        Ok(Some(match b {
            Builtin::LocalId | Builtin::GlobalId => {
                let dim = self.scalar(args[0]);
                let x = self.ins(if b == Builtin::LocalId { O::Lid } else { O::Gid }, m, dim, 0, 0, 0);
                // The second or third dimension's: the SIMD group's (`vector`'s `Level::SimdGroup`).
                set(self, x, if v.level(n) <= Level::SimdGroup { Shape::Uniform } else { Shape::Lanes })
            }
            Builtin::SubGroupLocalId => {
                let x = self.ins(O::Lane, M::U32, 0, 0, 0, 0);
                let x = set(self, x, Shape::Lanes);
                self.fit(x, m)
            }
            Builtin::Barrier => {
                self.ins(O::Bar, M::VOID, p[3], 0, 0, 0);
                return Ok(None);
            }
            Builtin::MemFence => return Ok(None),
            Builtin::SubGroupBarrier => {
                self.ins(O::SgBar, M::VOID, 0, 0, 0, 0);
                return Ok(None);
            }
            Builtin::Yield => {
                self.ins(O::Yield, M::VOID, 0, 0, 0, 0);
                return Ok(None);
            }
            Builtin::SubGroupBallot => {
                let xm = self.m_of(args[0]);
                let x = self.cast(args[0], xm, M::B);
                let x = if v.level(kids[0]) <= Level::SimdGroup { self.scalar(x) } else { x };
                let mk = if self.is_uniform(x) {
                    // A uniform truth: every running lane's or none.
                    let c = self.cur();
                    let none = self.none();
                    self.br_value(x, c, none)
                } else {
                    let mo = self.mask_of(x);
                    let c = self.cur();
                    self.mask_op(O::MAnd, c, mo)
                };
                let r = self.ins(O::Bits, M::U32, mk, 0, 0, 0);
                let r = set(self, r, Shape::Uniform);
                self.fit(r, m)
            }
            Builtin::SubGroupReduceAdd | Builtin::SubGroupReduceMin | Builtin::SubGroupReduceMax => {
                let x = self.fit(args[0], m);
                let c = self.cur();
                let r = self.ins_sub(O::Reduce, b as u32, m, x, c, 0, 0);
                set(self, r, Shape::Uniform)
            }
            Builtin::SubGroupShuffle | Builtin::SubGroupShuffleUp | Builtin::SubGroupShuffleDown | Builtin::SubGroupShuffleXor => {
                let x = self.fit(args[0], m);
                let lane = self.fit(args[1], M::U32);
                let c = self.cur();
                let r = self.ins_sub(O::Shuffle, b as u32, m, x, lane, c, 0);
                set(self, r, Shape::Lanes)
            }
            _ if crate::vector::is_atomic(b) => {
                // Once for the SIMD group: where its control is the group's, or where at most one lane runs it
                // (`single`) and its operands are uniform.
                let cur = self.cur();
                let one = self.v.level(n) <= Level::SimdGroup || (self.single(cur) && args.iter().all(|&a| self.is_uniform(a)));
                let mut ops = [NONE; 3];
                for (k, &a) in args.iter().enumerate() {
                    let a = if k == 0 { self.fit(a, M::PTR) } else { self.fit(a, m) };
                    ops[k] = if one { self.scalar(a) } else { a };
                }
                let c = self.cur();
                let r = self.ins_sub(O::Atomic, b as u32, m, ops[0], ops[1], ops[2], c);
                set(self, r, if one { Shape::Uniform } else { Shape::Lanes })
            }
            Builtin::LocalSize | Builtin::GlobalSize | Builtin::GroupId | Builtin::NumGroups => {
                let dim = self.scalar(args[0]);
                let r = self.ins_sub(O::GroupInfo, b as u32, m, dim, 0, 0, 0);
                set(self, r, Shape::Uniform)
            }
            _ => {
                let from = arg_m(0);
                let mut held = [NONE; 2];
                for (k, &a) in args.iter().enumerate().take(2) {
                    let a = self.fit(a, arg_m(k));
                    held[k] = if once { self.scalar(a) } else { a };
                }
                let s = held.iter().filter(|&&x| x != NONE).map(|&x| self.shape(x)).max().unwrap_or(Shape::Uniform);
                let r = self.ins_sub(O::LaneOp, b as u32, m, held[0], held[1], from.0, 0);
                set(self, r, s)
            }
        }))
    }

    /// A mask by a uniform `bool`: `t` where it is true, else `f`.
    fn br_value(&mut self, c: V, t: V, f: V) -> V {
        let var = self.temp(M::VOID);
        let (bt, bf) = self.br(c);
        let join = self.block();
        self.switch_to(bt);
        self.write(var, t);
        self.jump(join);
        self.switch_to(bf);
        self.write(var, f);
        self.jump(join);
        self.seal(join);
        self.switch_to(join);
        self.read(var)
    }

    fn call(&mut self, n: usize) -> R<Option<V>> {
        let v = self.v;
        let p = v.p(n);
        let (m, temp, id) = (M(p[1]), p[2], p[3]);
        let (body, k) = self.bodies[&id];
        if k == INLINE {
            return self.inline(n);
        }
        let bp = v.p(body);
        let slot = bp[3];
        let callee = self.built[k].as_ref().expect("a callee built before its callers");
        let kids: Vec<usize> = v.children(n).collect();
        let mut vals = Vec::with_capacity(kids.len());
        for (a, &c) in kids.iter().enumerate() {
            let pm = M(bp[8 + 2 * a]);
            let x = self.value(c)?;
            let x = if pm.is_agg() { x } else { self.fit(x, pm) };
            let x = if callee.args[a].1 == Shape::Uniform { self.scalar(x) } else { x };
            vals.push(x);
        }
        let (ret_m, ret_s) = callee.ret;
        // The callee's frame is past the body the call is in.
        let advance = self.base + self.region;
        let at = self.extra.len() as u32;
        self.extra.extend_from_slice(&vals);
        self.extra.push(advance);
        let c = self.cur();
        let r = self.ins(O::Call, if ret_m.is_agg() { M::VOID } else { ret_m }, k as u32, at, vals.len() as u32, c);
        self.shaped(r, if m == M::VOID || m.is_agg() { Shape::None } else { ret_s });
        Ok(if m == M::VOID {
            None
        } else if m.is_agg() {
            let from = self.frame_addr((advance + slot) * W as u32, m.bytes());
            let to = self.frame_addr((self.base + temp) * W as u32, m.bytes());
            let c = self.cur();
            self.ins(O::Copy, M(m.bytes()), to, from, c, 0);
            Some(to)
        } else {
            Some(r)
        })
    }

    /// A call of a function whose calls are its instructions (not `noinline`): its body built here, under the
    /// lanes that call, its frame past the body the call is in, its result and its returned lanes variables of its
    /// own. Every lane that called goes on after it (a `return` leaves the callee only).
    fn inline(&mut self, n: usize) -> R<Option<V>> {
        let v = self.v;
        let p = v.p(n);
        let (m, temp, id) = (M(p[1]), p[2], p[3]);
        let (body, _) = self.bodies[&id];
        let bp = v.p(body);
        let (slot, ret) = (bp[3], M(bp[4]));
        let kids: Vec<usize> = v.children(n).collect();
        let mut vals = Vec::with_capacity(kids.len());
        for (a, &c) in kids.iter().enumerate() {
            let pm = M(bp[8 + 2 * a]);
            let x = self.value(c)?;
            vals.push(if pm.is_agg() { x } else { self.fit(x, pm) });
        }
        let at = self.base + self.region;
        let saved = (self.base, self.region, self.rv, self.res);
        self.base = at;
        self.region = crate::vector::region(bp[2], ret);
        // No lane has returned from this call yet (a call in a loop is made again each time).
        self.rv = self.temp(M::VOID);
        let none = self.none();
        self.write(self.rv, none);
        self.res = if ret == M::VOID || ret.is_agg() { RESULT } else { self.temp(ret) };
        for (a, &x) in vals.iter().enumerate() {
            let (off, pm) = (bp[7 + 2 * a], M(bp[8 + 2 * a]));
            if self.promoted.contains(&(at + off, pm.bytes())) {
                let var = local(at + off, pm.bytes());
                self.write_var(var, pm, x);
            } else {
                let to = self.frame_addr((at + off) * W as u32, pm.bytes());
                let c = self.cur();
                if pm.is_agg() {
                    let mut i = Inst::new(O::Copy, M(pm.bytes()), to, x, c, 0);
                    i.shape = Shape::None;
                    self.push(i);
                } else {
                    self.ins(O::Store, pm, to, x, c, 0);
                }
            }
        }
        let lanes = self.cur();
        self.stmt(v.child(body, 0))?;
        self.set_cur(lanes);
        let res = self.res;
        let r = if m == M::VOID {
            None
        } else if m.is_agg() {
            let from = self.frame_addr((at + slot) * W as u32, m.bytes());
            let to = self.frame_addr((saved.0 + temp) * W as u32, m.bytes());
            let c = self.cur();
            self.ins(O::Copy, M(m.bytes()), to, from, c, 0);
            Some(to)
        } else {
            Some(self.read(res))
        };
        (self.base, self.region, self.rv, self.res) = saved;
        Ok(r)
    }

    // The function, finished: values renumbered in block order, the phis' operands its jumps' arguments, shapes to
    // their fixed point, and the simplifications (a select under the entry's lanes, a first lane of a uniform).

    fn finish(mut self, k: u32, name: String, region: u32, args: Vec<(M, Shape)>, ret: M) -> R<Func> {
        // Every phi resolved.
        let live_phis: Vec<V> = self.phis.keys().copied().filter(|p| !self.repl.contains_key(p)).collect();
        for p in &live_phis {
            let ops: Vec<V> = self.phis[p].ops.iter().map(|&o| self.resolve(o)).collect();
            self.phis.get_mut(p).expect("a phi").ops = ops;
        }
        // Each instruction's operands resolved.
        let resolve_all = |s: &mut Self| {
            for i in 0..s.insts.len() {
                let mut inst = s.insts[i];
                for u in inst.uses_mut() {
                    *u = s.resolve(*u);
                }
                s.insts[i] = inst;
            }
            for e in 0..s.extra.len() {
                // Call arguments are values; labels are words (resolved only where a call's).
                let _ = e;
            }
        };
        resolve_all(&mut self);
        let calls: Vec<V> = (0..self.insts.len() as V).filter(|&x| self.insts[x as usize].op == O::Call).collect();
        for c in calls {
            let i = self.insts[c as usize];
            for e in i.b..i.b + i.c {
                let x = self.extra[e as usize];
                self.extra[e as usize] = self.resolve(x);
            }
        }
        // Simplified: a mask with none or of one mask twice; a select under the entry's lanes is its new value, under
        // none its old, of a value never set its new; then the phis made trivial by that. Shapes are found after, from none (a parameter is lanes only if an
        // operand is), and a first lane of a value then uniform is it.
        loop {
            let mut changed = false;
            for x in 0..self.insts.len() as V {
                if self.repl.contains_key(&x) {
                    continue;
                }
                let i = self.insts[x as usize];
                let no = |u: V| self.insts[u as usize].op == O::NoLanes;
                let to = match i.op {
                    // Masks with none, or of one mask twice (a loop's exits, found none once its phis are).
                    O::MAnd | O::MAndNot if no(i.a) => Some(i.a),
                    O::MAnd if no(i.b) => Some(i.b),
                    O::MOr | O::MAndNot if no(i.b) => Some(i.a),
                    O::MOr if no(i.a) => Some(i.b),
                    O::MAnd | O::MOr if i.a == i.b => Some(i.a),
                    O::Select if i.a == self.entry => Some(i.b),
                    O::Select if self.insts[i.a as usize].op == O::NoLanes => Some(i.c),
                    O::Select if i.b == i.c => Some(i.b),
                    O::Select if self.insts[i.c as usize].op == O::Undef => Some(i.b),
                    _ => None,
                };
                if let Some(y) = to {
                    let y = self.resolve(y);
                    if y != x {
                        self.repl.insert(x, y);
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
            resolve_all(&mut self);
            let ps: Vec<V> = self.phis.keys().copied().filter(|p| !self.repl.contains_key(p)).collect();
            for p in ps {
                if self.repl.contains_key(&p) {
                    continue;
                }
                let ops: Vec<V> = self.phis[&p].ops.iter().map(|&o| self.resolve(o)).collect();
                self.phis.get_mut(&p).expect("a phi").ops = ops;
                self.try_trivial(p);
            }
            resolve_all(&mut self);
        }
        let ps: Vec<V> = self.phis.keys().copied().filter(|p| !self.repl.contains_key(p)).collect();
        for p in &ps {
            let ops: Vec<V> = self.phis[p].ops.iter().map(|&o| self.resolve(o)).collect();
            self.phis.get_mut(p).expect("a phi").ops = ops;
        }
        for &p in &ps {
            let mask = self.is_mask_var(self.phis[&p].var);
            self.insts[p as usize].shape = if mask { Shape::Mask } else { Shape::Uniform };
        }
        loop {
            let mut more = false;
            for x in 0..self.insts.len() {
                if self.repl.contains_key(&(x as V)) {
                    continue;
                }
                let i = self.insts[x];
                let lanes = |u: V| self.insts[u as usize].shape == Shape::Lanes;
                let pick = |b: bool| if b { Shape::Lanes } else { Shape::Uniform };
                let s = match i.op {
                    O::Param if i.shape == Shape::Mask => Shape::Mask,
                    O::Param => pick(self.phis[&(x as V)].ops.iter().any(|&u| lanes(u))),
                    O::Bin | O::Cast | O::Un | O::LaneOp => pick(i.uses().any(lanes)),
                    O::Load => pick(lanes(i.a)),
                    _ => i.shape,
                };
                if s != i.shape {
                    self.insts[x].shape = s;
                    more = true;
                }
            }
            if !more {
                break;
            }
        }
        for x in 0..self.insts.len() as V {
            let i = self.insts[x as usize];
            if i.op == O::First && !self.repl.contains_key(&x) && self.insts[self.resolve(i.a) as usize].shape == Shape::Uniform {
                let y = self.resolve(i.a);
                self.repl.insert(x, y);
            }
        }
        resolve_all(&mut self);
        for &p in &ps {
            let ops: Vec<V> = self.phis[&p].ops.iter().map(|&o| self.resolve(o)).collect();
            self.phis.get_mut(&p).expect("a phi").ops = ops;
        }
        // Renumber: reachable blocks in order, their parameters and instructions; dead values (no effect, no use)
        // dropped.
        let nb = self.blocks.len();
        let mut reach = vec![false; nb];
        let mut stack = vec![0u32];
        while let Some(b) = stack.pop() {
            if reach[b as usize] {
                continue;
            }
            reach[b as usize] = true;
            let t = *self.blocks[b as usize].insts.last().ok_or_else(|| Error(format!("block {b} has no terminator")))?;
            let i = self.insts[t as usize];
            let succ: Vec<u32> = match i.op {
                O::Jump => vec![i.a],
                O::Br => vec![i.b, i.c],
                O::Switch => {
                    let mut s: Vec<u32> = self.extra[i.b as usize..][..3 * i.c as usize].chunks(3).map(|t| t[2]).collect();
                    s.push(self.extra[(i.b + 3 * i.c) as usize]);
                    s
                }
                _ => vec![],
            };
            stack.extend(succ);
        }
        // Uses, to drop the dead: an instruction with no effect and no use.
        let pure = |op: O| matches!(op, O::Const | O::Arg | O::Entry | O::NoLanes | O::Undef | O::Param | O::Bin | O::Un | O::Cast | O::Select | O::First | O::MaskOf | O::MAnd | O::MOr | O::MAndNot | O::Any | O::Global | O::Shared | O::FrameAddr | O::Lid | O::Gid | O::Lane | O::GroupInfo | O::LaneOp | O::Load);
        let mut used = vec![false; self.insts.len()];
        let mut work: Vec<V> = Vec::new();
        for b in 0..nb {
            if !reach[b] {
                continue;
            }
            for &x in &self.blocks[b].insts {
                if self.repl.contains_key(&x) {
                    continue;
                }
                if !pure(self.insts[x as usize].op) {
                    work.push(x);
                }
            }
        }
        while let Some(x) = work.pop() {
            if used[x as usize] {
                continue;
            }
            used[x as usize] = true;
            let i = self.insts[x as usize];
            for u in i.uses() {
                work.push(self.resolve(u));
            }
            if i.op == O::Call {
                for e in i.b..i.b + i.c {
                    work.push(self.resolve(self.extra[e as usize]));
                }
            }
            if i.op == O::Param {
                if let Some(ph) = self.phis.get(&x) {
                    for &o in &ph.ops {
                        work.push(self.resolve(o));
                    }
                }
            }
            // A jump uses its target's live parameters' operands: added below as they are found live.
        }
        // Parameters live only through a jump to their block are used (above, by `Param`'s operands, once a use of
        // the parameter is found).
        let mut num = vec![NONE; self.insts.len()];
        let mut insts = Vec::new();
        let mut blocks = Vec::new();
        let mut bnum = vec![u32::MAX; nb];
        for b in 0..nb {
            if reach[b] {
                bnum[b] = blocks.len() as u32;
                blocks.push(Block::default());
            }
        }
        // Each block's live parameters, in order: kept as its params.
        let mut kept_params: Vec<Vec<V>> = vec![Vec::new(); nb];
        for b in 0..nb {
            if !reach[b] {
                continue;
            }
            let nbk = bnum[b] as usize;
            for &p in &self.blocks[b].params {
                if self.repl.contains_key(&p) || !used[p as usize] {
                    continue;
                }
                num[p as usize] = insts.len() as V;
                let mut i = self.insts[p as usize];
                i.a = blocks[nbk].params.len() as u32;
                insts.push(i);
                blocks[nbk].params.push(num[p as usize]);
                kept_params[b].push(p);
            }
        }
        // The extra words, rebuilt: jumps' arguments (their targets' kept parameters' operands), calls', labels.
        let mut extra = Vec::new();
        for b in 0..nb {
            if !reach[b] {
                continue;
            }
            let nbk = bnum[b] as usize;
            for &x in &self.blocks[b].insts {
                if self.repl.contains_key(&x) {
                    continue;
                }
                let i = self.insts[x as usize];
                if !(used[x as usize] || i.op.terminates()) {
                    continue;
                }
                num[x as usize] = insts.len() as V;
                insts.push(i);
                blocks[nbk].insts.push(num[x as usize]);
            }
        }
        // Operands renumbered (every operand is defined in a block reached, before its use in the order or as a
        // parameter: checked by `verify`).
        let map = |s: &Self, num: &[V], x: V| -> V {
            if x == NONE {
                return NONE;
            }
            num[s.resolve(x) as usize]
        };
        for b in 0..nb {
            if !reach[b] {
                continue;
            }
            let nbk = bnum[b] as usize;
            let list = blocks[nbk].insts.clone();
            for &y in &list {
                let mut i = insts[y as usize];
                for u in i.uses_mut() {
                    *u = map(&self, &num, *u);
                }
                match i.op {
                    O::Jump => {
                        let to = i.a as usize;
                        let pred_index = self.preds[to].iter().position(|&q| q == b as u32).expect("a jump's block among its target's predecessors");
                        let at = extra.len() as u32;
                        for &p in &kept_params[to] {
                            let o = self.phis[&p].ops[pred_index];
                            extra.push(map(&self, &num, o));
                        }
                        i.b = at;
                        i.c = kept_params[to].len() as u32;
                        i.a = bnum[to];
                    }
                    O::Br => {
                        i.b = bnum[i.b as usize];
                        i.c = bnum[i.c as usize];
                    }
                    O::Switch => {
                        let at = extra.len() as u32;
                        for t in self.extra[i.b as usize..][..3 * i.c as usize].chunks(3) {
                            extra.extend_from_slice(&[t[0], t[1], bnum[t[2] as usize]]);
                        }
                        extra.push(bnum[self.extra[(i.b + 3 * i.c) as usize] as usize]);
                        i.b = at;
                    }
                    O::Call => {
                        let at = extra.len() as u32;
                        for e in i.b..i.b + i.c {
                            extra.push(map(&self, &num, self.extra[e as usize]));
                        }
                        extra.push(self.extra[(i.b + i.c) as usize]);
                        i.b = at;
                    }
                    _ => {}
                }
                insts[y as usize] = i;
            }
        }
        let ret_shape = if ret == M::VOID || ret.is_agg() {
            Shape::None
        } else {
            blocks.iter().filter_map(|b| b.insts.last()).map(|&t| insts[t as usize]).filter(|i| i.op == O::Ret && i.a != NONE).map(|i| insts[i.a as usize].shape).max().unwrap_or(Shape::Uniform)
        };
        Ok(Func { body: k, name, region, args, ret: (if ret.is_agg() { M::VOID } else { ret }, ret_shape), insts, blocks, extra, cross: false })
    }
}

/// Node `n`'s value if it is a constant (as the code tape's).
fn constant(v: &View, n: usize) -> Option<u64> {
    match v.kind(n) {
        Kind::Const => Some(u64::from(v.w(n, 1)) | u64::from(v.w(n, 2)) << 32),
        Kind::Cast => constant(v, n + 1),
        Kind::Bin => {
            let (x, y) = (constant(v, v.child(n, 0))?, constant(v, v.child(n, 1))?);
            match Op::of(v.w(n, 0)) {
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

/// A function's SSA checked: each block ends in its one terminator; each operand is defined before its use, in a
/// block that dominates the use's (a parameter at its block's start); each jump passes its target's parameters; a
/// branch's and a switch's targets have no parameters and one predecessor.
pub fn verify(f: &Func) -> Result<(), String> {
    let nb = f.blocks.len();
    let mut block_of = vec![u32::MAX; f.insts.len()];
    let mut pos = vec![0usize; f.insts.len()];
    for (b, blk) in f.blocks.iter().enumerate() {
        for (k, &p) in blk.params.iter().enumerate() {
            block_of[p as usize] = b as u32;
            pos[p as usize] = 0;
            if f.inst(p).op != O::Param || f.inst(p).a != k as u32 {
                return Err(format!("block {b}'s parameter {k} is not its"));
            }
        }
        if blk.insts.is_empty() {
            return Err(format!("block {b} is empty"));
        }
        for (k, &x) in blk.insts.iter().enumerate() {
            block_of[x as usize] = b as u32;
            pos[x as usize] = k + 1;
            let last = k + 1 == blk.insts.len();
            if f.inst(x).op.terminates() != last {
                return Err(format!("block {b}: {:?} at {k} of {}", f.inst(x).op, blk.insts.len()));
            }
        }
    }
    // Predecessors, and dominators (the iterative algorithm, blocks in order: the entry first).
    let mut preds = vec![Vec::new(); nb];
    for b in 0..nb {
        for s in f.succs(b) {
            if s as usize >= nb {
                return Err(format!("block {b} jumps to block {s}, past the {nb}"));
            }
            preds[s as usize].push(b);
        }
    }
    for b in 0..nb {
        let t = f.inst(*f.blocks[b].insts.last().expect("a terminator"));
        match t.op {
            O::Jump => {
                if t.c as usize != f.blocks[t.a as usize].params.len() {
                    return Err(format!("block {b} passes {} arguments to block {}'s {} parameters", t.c, t.a, f.blocks[t.a as usize].params.len()));
                }
            }
            O::Br | O::Switch => {
                for s in f.succs(b) {
                    if !f.blocks[s as usize].params.is_empty() || preds[s as usize].len() != 1 {
                        return Err(format!("block {b} branches to block {s}, which has parameters or other predecessors"));
                    }
                }
            }
            _ => {}
        }
    }
    let idom = idoms(f);
    let dominates = |a: usize, mut b: usize| {
        loop {
            if a == b {
                return true;
            }
            if b == 0 || idom[b] == usize::MAX {
                return false;
            }
            b = idom[b];
        }
    };
    let check = |user: V, x: V, ub: usize| -> Result<(), String> {
        if x as usize >= f.insts.len() {
            return Err(format!("value {user} uses {x}, past the {}", f.insts.len()));
        }
        let db = block_of[x as usize] as usize;
        if db == u32::MAX as usize {
            return Err(format!("value {user} uses {x}, in no block"));
        }
        if db == ub {
            if pos[x as usize] >= pos[user as usize] {
                return Err(format!("value {user} uses {x}, defined after it in block {ub}"));
            }
        } else if !dominates(db, ub) {
            return Err(format!("value {user} (block {ub}) uses {x} (block {db}), which does not dominate it"));
        }
        Ok(())
    };
    for b in 0..nb {
        for &x in &f.blocks[b].insts {
            let i = f.inst(x);
            for u in i.uses() {
                check(x, u, b)?;
            }
            if matches!(i.op, O::Jump | O::Call) {
                for &u in f.args_of(i) {
                    check(x, u, b)?;
                }
            }
        }
    }
    Ok(())
}

/// The blocks reached, in reverse postorder.
/// Whether each block of function `f` of `ssa` has an operation that reads the SIMD group's lanes as a whole: a
/// cross-lane one, a uniform atomic (made once for the group), a call of a function that has one.
pub fn cross_blocks(ssa: &Ssa, f: &Func) -> Vec<bool> {
    f.blocks
        .iter()
        .map(|blk| {
            blk.insts.iter().any(|&x| {
                let i = f.inst(x);
                i.op.cross_lane() || (i.op == O::Atomic && i.shape == Shape::Uniform) || (i.op == O::Call && ssa.funcs[i.a as usize].cross)
            })
        })
        .collect()
}

/// Whether each block's branch (`Br`, `Switch`) must be the SIMD group's decision where its lanes run as chunks of
/// their own (decision 0567, step 3): whether what it branches to reaches an operation of the group's lanes as a
/// whole (`cross_blocks`) before the blocks join again (its immediate post-dominator), so every chunk meets the
/// group's operations in one order. A block with no such branch is false.
pub fn group_decisions(ssa: &Ssa, f: &Func) -> Vec<bool> {
    let nb = f.blocks.len();
    let cross = cross_blocks(ssa, f);
    let succs: Vec<Vec<u32>> = (0..nb).map(|b| f.succs(b)).collect();
    let ipdom = post_dominators(&succs);
    (0..nb)
        .map(|b| {
            let t = f.inst(*f.blocks[b].insts.last().expect("a terminator"));
            if !matches!(t.op, O::Br | O::Switch) {
                return false;
            }
            let stop = ipdom[b];
            let mut seen = vec![false; nb];
            let mut stack: Vec<usize> = succs[b].iter().map(|&s| s as usize).collect();
            while let Some(x) = stack.pop() {
                if Some(x) == stop || seen[x] {
                    continue;
                }
                seen[x] = true;
                if cross[x] {
                    return true;
                }
                stack.extend(succs[x].iter().map(|&s| s as usize));
            }
            false
        })
        .collect()
}

/// Each block's immediate post-dominator (none: the function's exit, or a block that reaches no exit), by the
/// dominators of the reversed graph from a virtual exit every block without successors goes to.
fn post_dominators(succs: &[Vec<u32>]) -> Vec<Option<usize>> {
    let nb = succs.len();
    let exit = nb;
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); nb + 1];
    for (b, s) in succs.iter().enumerate() {
        for &t in s {
            preds[t as usize].push(b);
        }
        if s.is_empty() {
            preds[exit].push(b);
        }
    }
    // The reversed graph's reverse postorder from the exit (its successors a block's predecessors).
    let mut seen = vec![false; nb + 1];
    let mut post = Vec::new();
    let mut stack = vec![(exit, 0usize)];
    seen[exit] = true;
    while let Some(&mut (b, ref mut k)) = stack.last_mut() {
        if *k < preds[b].len() {
            let n = preds[b][*k];
            *k += 1;
            if !seen[n] {
                seen[n] = true;
                stack.push((n, 0));
            }
        } else {
            post.push(b);
            stack.pop();
        }
    }
    post.reverse();
    let mut order = vec![usize::MAX; nb + 1];
    for (k, &b) in post.iter().enumerate() {
        order[b] = k;
    }
    let mut idom = vec![usize::MAX; nb + 1];
    idom[exit] = exit;
    loop {
        let mut changed = false;
        for &b in post.iter().skip(1) {
            // The reversed graph's predecessors of b: its successors (the exit for one with none).
            let rp: Vec<usize> = if succs[b].is_empty() { vec![exit] } else { succs[b].iter().map(|&s| s as usize).collect() };
            let mut new = usize::MAX;
            for p in rp {
                if idom[p] == usize::MAX {
                    continue;
                }
                new = if new == usize::MAX { p } else { intersect(&idom, &order, p, new) };
            }
            if new != usize::MAX && idom[b] != new {
                idom[b] = new;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    (0..nb).map(|b| if idom[b] == usize::MAX || idom[b] == exit { None } else { Some(idom[b]) }).collect()
}

/// Each block's immediate dominator (the entry's itself; `usize::MAX` a block no path reaches): the iterative
/// algorithm, over the blocks in reverse postorder.
pub fn idoms(f: &Func) -> Vec<usize> {
    let nb = f.blocks.len();
    let mut preds = vec![Vec::new(); nb];
    for b in 0..nb {
        for s in f.succs(b) {
            if (s as usize) < nb {
                preds[s as usize].push(b);
            }
        }
    }
    let rpo = rpo(f);
    let mut idom = vec![usize::MAX; nb];
    let mut order = vec![usize::MAX; nb];
    for (k, &b) in rpo.iter().enumerate() {
        order[b] = k;
    }
    idom[0] = 0;
    let mut changed = true;
    while changed {
        changed = false;
        for &b in &rpo[1..] {
            let mut new = usize::MAX;
            for &p in &preds[b] {
                if idom[p] == usize::MAX {
                    continue;
                }
                new = if new == usize::MAX { p } else { intersect(&idom, &order, p, new) };
            }
            if new != idom[b] {
                idom[b] = new;
                changed = true;
            }
        }
    }
    idom
}

pub fn rpo(f: &Func) -> Vec<usize> {
    let nb = f.blocks.len();
    let mut seen = vec![false; nb];
    let mut post = Vec::new();
    let mut stack = vec![(0usize, 0usize)];
    seen[0] = true;
    while let Some(&mut (b, ref mut k)) = stack.last_mut() {
        let s = f.succs(b);
        if *k < s.len() {
            let n = s[*k] as usize;
            *k += 1;
            if !seen[n] {
                seen[n] = true;
                stack.push((n, 0));
            }
        } else {
            post.push(b);
            stack.pop();
        }
    }
    post.reverse();
    post
}

fn intersect(idom: &[usize], order: &[usize], mut a: usize, mut b: usize) -> usize {
    while a != b {
        while order[a] > order[b] {
            a = idom[a];
        }
        while order[b] > order[a] {
            b = idom[b];
        }
    }
    a
}

impl Ssa {
    /// The SSA as a tape (`Kind::Code`'s one node): `[functions, each one's start]`, then each function: `[body,
    /// region, arguments, values, blocks, extra words, cross, result type, result shape]`, its arguments' `(type,
    /// shape)`, its blocks' `(parameters, instructions)`, its values (`[op | sub << 8 | shape << 16, m, a, b, c,
    /// d]`, the blocks' parameters then instructions, in order), its extra words.
    pub fn tape(&self) -> Tape {
        let mut w = vec![self.funcs.len() as u32];
        let starts = w.len();
        w.extend(std::iter::repeat_n(0, self.funcs.len()));
        for (k, f) in self.funcs.iter().enumerate() {
            w[starts + k] = w.len() as u32;
            w.extend_from_slice(&[f.body, f.region, f.args.len() as u32, f.insts.len() as u32, f.blocks.len() as u32, f.extra.len() as u32, u32::from(f.cross), f.ret.0.0, f.ret.1 as u32]);
            for &(m, s) in &f.args {
                w.extend_from_slice(&[m.0, s as u32]);
            }
            for b in &f.blocks {
                w.extend_from_slice(&[b.params.len() as u32, b.insts.len() as u32]);
            }
            for b in &f.blocks {
                for &x in b.params.iter().chain(&b.insts) {
                    let i = f.inst(x);
                    w.extend_from_slice(&[i.op as u32 | u32::from(i.sub) << 8 | (i.shape as u32) << 16, i.m.0, i.a, i.b, i.c, i.d]);
                }
            }
            w.extend_from_slice(&f.extra);
        }
        let mut b = Builder::default();
        b.leaf(Kind::Code, &w);
        b.finish()
    }

    /// A readable listing, for a test's failure and a reader.
    pub fn listing(&self) -> String {
        use std::fmt::Write;
        let mut s = String::new();
        for (k, f) in self.funcs.iter().enumerate() {
            let _ = writeln!(s, "fn {k} {} (region {}, args {:?}, ret {:?}{}):", f.name, f.region, f.args, f.ret, if f.cross { ", cross-lane" } else { "" });
            for (b, blk) in f.blocks.iter().enumerate() {
                let ps: Vec<String> = blk.params.iter().map(|&p| format!("v{p}:{:?}", f.inst(p).shape)).collect();
                let _ = writeln!(s, "  b{b}({}):", ps.join(", "));
                for &x in &blk.insts {
                    let i = f.inst(x);
                    let extra = match i.op {
                        O::Jump | O::Call => format!(" {:?}", f.args_of(i)),
                        O::Switch => format!(" {:?}", f.labels(i).collect::<Vec<_>>()),
                        _ => String::new(),
                    };
                    let _ = writeln!(s, "    v{x} = {:?}/{} {:?} m{} [{} {} {} {}]{extra}", i.op, i.sub, i.shape, i.m.0, i.a as i32, i.b as i32, i.c as i32, i.d as i32);
                }
            }
        }
        s
    }
}
