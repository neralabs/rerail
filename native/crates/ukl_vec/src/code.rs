//! A kernel's code tape (decision 0567 §6, the interpreter): its vector tape lowered once to a flat program over
//! registers, which `run` executes a SIMD group at a time, an instruction a whole 32-lane register.
//!
//! - **The tape:** one `Code` node, its payload the header (`HEADER`), the kernel's parameters' `(offset, type)`
//!   and the instructions, each `WORDS` words: `[opcode | fields, a, b, c, d]`. The interpreter reads the payload in
//!   place.
//! - **Opcodes are fused:** an operation and its type are one opcode (`Add32`, `LtI32`, `AddF64`, ...), so an
//!   instruction is one jump through the interpreter's table, and its handler a loop over lanes with nothing to
//!   decide. The rare ones (a narrow type's arithmetic, a division, a float's conversion) are generic (`VBinGen`, ...),
//!   their operation in the fields, still decided once an instruction.
//! - **Registers:** a SIMD group's scalars (`u64`, a value uniform in it: `Level::SimdGroup` or less), its 32-bit
//!   lanes (`[u32; 32]`: every type of 4 bytes or fewer, normalized as `ops::norm`) and its 64-bit lanes. They are
//!   allocated by expression depth: an expression's temporaries die with it, a variable lives in the frame.
//! - **Masks:** `CUR` (the lanes running), `BRK`, `CONT`, `RET` (the lanes that took those exits from the innermost
//!   loop or inlined call), `END` (the lanes that ended), and slots to save them in. A uniform branch is a jump; a
//!   varying one sets `CUR` for each side; a jump skips code whose `CUR` is empty, so no code runs with no lanes.
//! - **A `switch`** is a jump by its value through a table (`Switch` and its `Label`s, sorted), to its case; the cases
//!   follow in order, falling through. On a value that varies, a round for each value the running lanes have: the
//!   first remaining lane's, and the lanes that share it, through the same jump.
//! - **A lane's own variables** are in its SIMD group's frame, struct of arrays: lane `l` of the variable at frame
//!   offset `off`, `size` bytes, its byte `inner`, is at `off * W + l * size + inner`. A direct read or write of one
//!   (`LdL`, `StL`) is 32 consecutive values when it is a whole scalar variable.
//! - **Functions** are called, not inlined (a kernel's calls inlined would be hundreds of millions of nodes): each
//!   is a section of the code, run under the lanes that call it. A call advances the frame (by the caller's region:
//!   its frame and its result's slot) and the register and mask files (by the caller's counts), so the callee's
//!   registers are its own; the five masks go with it. The table after the code holds each function's advances.
//! - **A threadgroup barrier** (`Bar`) ends a SIMD group's turn: each SIMD group runs to it, then all go on. A poll
//!   of another group's progress (`Yield`) ends it too, and once every SIMD group of its group waits, the group's.

use crate::Error;
use crate::k::{Builder, Builtin, CHECK, EFFECT, Kind, Level, M, NO_SLOT, Op, Tape, View};
use crate::run::W;

/// An instruction's words.
pub const WORDS: usize = 5;

/// The header's words: frame bytes a lane, threadgroup bytes, parameters, scalar registers, 32-bit registers,
/// 64-bit registers, masks (each the deepest chain of calls'), the code's words, the functions.
pub const HEADER: usize = 9;

/// A function's words in the table after the code: its region's bytes in a SIMD group's frame (`W` lanes'), its
/// scalar, 32-bit, 64-bit registers and masks: what a call from it advances by.
pub const TABLE: usize = 5;

/// The masks: the lanes running, those that took a `break`, a `continue`, a `return`, and those that ended.
pub const CUR: u32 = 0;
pub const BRK: u32 = 1;
pub const CONT: u32 = 2;
pub const RET: u32 = 3;
pub const END: u32 = 4;
const FIRST_SLOT: u32 = 5;

/// The opcodes. Operands are registers unless said: `s` a scalar, `v` a vector (32- or 64-bit lanes by the type),
/// `m` a mask, `t` an instruction index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum I {
    /// The kernel's end.
    Halt,
    /// `[t]`.
    Jmp,
    /// `[m, t]`: jump if mask `m` is empty.
    Jzm,
    /// `[s, t]`: jump if `s` is 0.
    Jzs,
    /// `[s, t]`: jump if `s` is not 0.
    Jnzs,
    /// `[m, m', v]`: `m = m' & (v != 0)` (32-bit lanes).
    MAnd,
    /// `[m, m', v]`: `m = m' & (v == 0)`.
    MAndN,
    /// `[m, m']`.
    MSet,
    /// `[m, m']`: `m |= m'`.
    MOr,
    /// `[m]`: `m = 0`.
    MClr,
    /// `CUR &= !(BRK | CONT | RET | END)`.
    MLive,
    /// `[m]`: `CUR = m & !(BRK | RET | END)` (a loop's next iteration).
    MRejoin,
    /// `[m]`: `CUR = m & !(RET | END)` (past a loop).
    MExit,
    /// `[m]`: `CUR = m & !END` (past an inlined call).
    MResume,
    /// `[m, m', m'']`: `m = m' & !m''`.
    MAndNM,
    /// `[m]`: `CUR = m & !(CONT | RET | END)` (past a `switch`: its breaks' lanes go on).
    MSwEnd,
    /// `[node, site, s]`: the group decides an agreed branch alike (`s`'s value: a condition's truth, a `switch`'s
    /// value).
    Agree,
    /// `[node, site, m]`: the same, the decision `m != 0`.
    AgreeM,
    /// `[site]`: a threadgroup barrier.
    Bar,
    /// A poll of another group's progress: the SIMD group's turn ends, and once its group's have, the group yields.
    Yield,
    /// `[t, f]`: a call of the function at instruction `t` from function `f` (its table's row: what to advance by).
    Call,
    /// A function's end: back to its caller.
    Ret,
    /// `[s, n, t]`: a jump by scalar `s`'s value through the `n` `Label`s that follow (sorted by value), or to `t`.
    Switch,
    /// `[low, high, t]`: a `Switch`'s entry, never run: value `low | high << 32` jumps to `t`.
    Label,

    /// `[s, low, high]`.
    SConst,
    /// `[s, offset]`: threadgroup memory's address.
    SShared,
    /// `[s, offset]`: the program's constant memory's address.
    SGlobal,
    /// `[s, s']`.
    SMov,
    /// `[s, a, b]`, fields `op`, `type`.
    SBin,
    /// `[s, a]`, fields `op`, `type`.
    SUn,
    /// `[s, a]`, fields `from`, `to`.
    SCast,
    /// `[s, a, b]`, fields `builtin`, `type`, `as`'s `from`.
    SBuiltin,
    /// `[s, address]`, field `type`.
    SLoad,
    /// `[address, s]`, field `type`.
    SStore,
    /// `[s, address, x, y]`, fields `builtin`, `type`: one lane's atomic for the SIMD group.
    SAtomic,
    /// `[s, base, stride]`, field `type`: the first running lane's variable.
    LdLS,
    /// `[s, v]`, field `type`: the first running lane's value.
    Extract32,
    Extract64,

    /// `[v, s]`.
    Bcast32,
    Bcast64,
    /// `[v, v']`: the running lanes'.
    VMovM32,
    VMovM64,

    /// `[v, address]`, field `type`.
    VLoad,
    /// `[address, v]`, field `type`.
    VStore,
    /// `[v, base, stride]`, field `type`: each lane's variable.
    LdL,
    /// `[base, stride, v]`, field `type`.
    StL,
    /// `[v, base, stride]`: each lane's variable's address.
    LAddr,
    /// `[to, from, size]`: each running lane's bytes copied.
    VCopy,
    /// `[address, size]`.
    VZero,

    /// `[v, s]`, field 1 for 64-bit lanes: the thread's id in its group in dimension `s`.
    VLid,
    /// The same, in the grid.
    VGid,
    /// `[v]`: the lane's index in its SIMD group.
    VLane,
    /// `[s, v]`: the running lanes whose `v` is not 0.
    Ballot,
    /// `[v, value, lane]`, field the shuffle (`Builtin`).
    Shuf32,
    Shuf64,
    /// `[s, v]`, fields the reduction (`Builtin`), `type`: the running lanes' sum, minimum or maximum.
    Reduce32,
    Reduce64,
    /// `[v, address, x, y]`, fields `builtin`, `type`: each running lane's atomic, in order.
    VAtomic,

    /// `[v, a, b]`, fields `op`, `type`: each running lane's, `ops::bin`.
    VBinGen,
    /// `[v, a]`, fields `op`, `type`.
    VUnGen,
    /// `[v, a]`, fields `from`, `to`.
    VCastGen,
    /// `[v, a, b]`, fields `builtin`, `type`, `as`'s `from`.
    VBuiltinGen,

    /// `[v, v']`: 32-bit lanes' conversions.
    Zx8,
    Sx8,
    Zx16,
    Sx16,
    Truth32,
    /// 64-bit lanes to 32-bit ones.
    Truth64,
    Trunc,
    /// 32-bit lanes to 64-bit ones.
    Zext,
    Sext,

    /// `[v, a, b]`: 32-bit lanes.
    Add32,
    Sub32,
    Mul32,
    And32,
    Or32,
    Xor32,
    Shl32,
    ShrU32,
    ShrI32,
    Eq32,
    Ne32,
    LtU32,
    LtI32,
    LeU32,
    LeI32,
    MinU32,
    MinI32,
    MaxU32,
    MaxI32,
    /// 64-bit lanes (a comparison's result 32-bit lanes).
    Add64,
    Sub64,
    Mul64,
    And64,
    Or64,
    Xor64,
    Shl64,
    ShrU64,
    ShrI64,
    Eq64,
    Ne64,
    LtU64,
    LtI64,
    LeU64,
    LeI64,
    MinU64,
    MinI64,
    MaxU64,
    MaxI64,
    AddF32,
    SubF32,
    MulF32,
    DivF32,
    EqF32,
    NeF32,
    LtF32,
    LeF32,
    AddF64,
    SubF64,
    MulF64,
    DivF64,
    EqF64,
    NeF64,
    LtF64,
    LeF64,
    /// `[v, a]`.
    Neg32,
    Neg64,
    NegF32,
    NegF64,
    Not32,
    Not64,
    LNot32,
    LNot64,
    Popcount32,
}

impl I {
    pub const LAST: u8 = I::Popcount32 as u8;

    /// The opcode of an instruction's first word.
    ///
    /// # Safety
    ///
    /// The word is an instruction's, written by `compile` (its low byte at most `LAST`).
    #[inline(always)]
    pub unsafe fn of(w: u32) -> I {
        debug_assert!((w & 0xff) as u8 <= Self::LAST);
        // SAFETY: `I` is `repr(u8)`, its values 0 to `LAST`; the caller's word is one of them.
        unsafe { std::mem::transmute::<u8, I>(w as u8) }
    }
}

/// A register's file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    S,
    V32,
    V64,
}

/// A value: its register, file and type.
#[derive(Clone, Copy, Debug)]
struct Val {
    r: u32,
    c: Class,
    m: M,
}

/// Which lanes' file a type's values are in.
fn lanes_of(m: M) -> Class {
    if m.bytes() == 8 || m.is_agg() { Class::V64 } else { Class::V32 }
}

type R<T> = Result<T, Error>;

/// The registers of one file: those free, and how many there are.
#[derive(Default)]
struct File {
    free: Vec<u32>,
    n: u32,
}

impl File {
    fn alloc(&mut self) -> u32 {
        self.free.pop().unwrap_or_else(|| {
            self.n += 1;
            self.n - 1
        })
    }
}

struct Gen<'t> {
    v: &'t View<'t>,
    code: Vec<u32>,
    s: File,
    v32: File,
    v64: File,
    /// Mask slots in use, and the most.
    mtop: u32,
    mn: u32,
    /// Whether a node's subtree may take an exit that leaves it (`break`, `continue`, `return`).
    exits: Vec<bool>,
    /// Each function's `Body`, its row in the table, by its id.
    bodies: std::collections::HashMap<u32, (usize, u32)>,
    /// The function being compiled: its row, and its region's bytes a lane.
    row: u32,
    region: u32,
    /// The calls to patch with their callees' first instructions: the instruction's index, the callee's id.
    calls: Vec<(usize, u32)>,
}

/// The code tape of vector tape `vt` (a `Func` at its root).
pub fn compile(vt: &Tape) -> R<Tape> {
    let v = View::new(vt);
    let n = v.len();
    let mut exits = vec![false; n];
    for i in (0..n).rev() {
        exits[i] = match v.kind(i) {
            Kind::Break | Kind::Continue | Kind::Return => true,
            // A call's returns end in it.
            Kind::Call => false,
            _ => v.children(i).any(|c| exits[c]),
        };
    }
    let kids: Vec<usize> = v.children(0).collect();
    let bodies = kids[1..].iter().enumerate().map(|(k, &b)| (v.w(b, 0), (b, k as u32 + 1))).collect();
    let root = v.p(0);
    let mut g = Gen {
        v: &v,
        code: Vec::new(),
        s: File::default(),
        v32: File::default(),
        v64: File::default(),
        mtop: FIRST_SLOT,
        mn: FIRST_SLOT,
        exits,
        bodies,
        row: 0,
        region: 0,
        calls: Vec::new(),
    };
    // Each function's section: the kernel's, then each body's. A row: [start, region, scalars, 32-bit, 64-bit,
    // masks], and the rows it calls.
    let mut rows: Vec<[u32; 6]> = Vec::new();
    let mut callees: Vec<Vec<u32>> = Vec::new();
    let mut starts = std::collections::HashMap::new();
    for (k, &body) in kids.iter().enumerate() {
        let (node, region) = if k == 0 { (body, crate::vector::kernel_frame(&v)) } else { (v.child(body, 0), crate::vector::region(v.w(body, 2), M(v.w(body, 4)))) };
        (g.s, g.v32, g.v64, g.mtop, g.mn) = (File::default(), File::default(), File::default(), FIRST_SLOT, FIRST_SLOT);
        (g.row, g.region) = (k as u32, region);
        let first = g.calls.len();
        let start = g.here();
        if k > 0 {
            starts.insert(v.w(body, 0), start);
        }
        g.stmt(node)?;
        g.emit(if k == 0 { I::Halt } else { I::Ret }, 0, [0; 4]);
        rows.push([start, region, g.s.n, g.v32.n, g.v64.n, g.mn]);
        callees.push(g.calls[first..].iter().map(|&(_, id)| g.bodies[&id].1).collect());
    }
    for &(at, id) in &g.calls {
        g.code[at + 1] = starts[&id];
    }
    // The files a SIMD group needs: the deepest chain of calls' counts.
    fn most(r: usize, rows: &[[u32; 6]], callees: &[Vec<u32>], k: usize, memo: &mut Vec<Option<u32>>) -> u32 {
        if let Some(x) = memo[r] {
            return x;
        }
        let below = callees[r].iter().map(|&c| most(c as usize, rows, callees, k, memo)).max().unwrap_or(0);
        let x = rows[r][k] + below;
        memo[r] = Some(x);
        x
    }
    let total = |k: usize| most(0, &rows, &callees, k, &mut vec![None; rows.len()]);
    let params = root[4] as usize;
    let mut words = vec![total(1), root[2], params as u32, total(2), total(3), total(4), total(5), g.code.len() as u32, rows.len() as u32];
    words.extend_from_slice(&root[5..5 + 2 * params]);
    words.extend_from_slice(&g.code);
    for r in &rows {
        words.extend_from_slice(&[r[1] * W as u32, r[2], r[3], r[4], r[5]]);
    }
    let mut b = Builder::default();
    b.leaf(Kind::Code, &words);
    Ok(b.finish())
}

/// The fields of a generic instruction: up to three bytes.
fn fields(a: u32, b: u32, c: u32) -> u32 {
    debug_assert!(a < 256 && b < 256 && c < 256);
    a | b << 8 | c << 16
}

/// Node `n`'s value if it is a constant (folded through casts and arithmetic).
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

/// The fused opcode of `a op b` on vectors of `m`, and whether its operands are swapped (`>` as `<`).
fn fused_bin(op: Op, m: M) -> Option<(I, bool)> {
    use I::*;
    let swap = matches!(op, Op::Gt | Op::Ge);
    let op = match op {
        Op::Gt => Op::Lt,
        Op::Ge => Op::Le,
        o => o,
    };
    let i = match (m, op) {
        (M::B, Op::And) => And32,
        (M::B, Op::Or) => Or32,
        (M::B, Op::Xor) => Xor32,
        (M::B, Op::Eq) => Eq32,
        (M::B, Op::Ne) => Ne32,
        (M::I32 | M::U32, o) => match (o, m.signed()) {
            (Op::Add, _) => Add32,
            (Op::Sub, _) => Sub32,
            (Op::Mul, _) => Mul32,
            (Op::And, _) => And32,
            (Op::Or, _) => Or32,
            (Op::Xor, _) => Xor32,
            (Op::Shl, _) => Shl32,
            (Op::Shr, false) => ShrU32,
            (Op::Shr, true) => ShrI32,
            (Op::Eq, _) => Eq32,
            (Op::Ne, _) => Ne32,
            (Op::Lt, false) => LtU32,
            (Op::Lt, true) => LtI32,
            (Op::Le, false) => LeU32,
            (Op::Le, true) => LeI32,
            _ => return None,
        },
        (M::I64 | M::U64 | M::PTR, o) => match (o, m.signed()) {
            (Op::Add, _) => Add64,
            (Op::Sub, _) => Sub64,
            (Op::Mul, _) => Mul64,
            (Op::And, _) => And64,
            (Op::Or, _) => Or64,
            (Op::Xor, _) => Xor64,
            (Op::Shl, _) => Shl64,
            (Op::Shr, false) => ShrU64,
            (Op::Shr, true) => ShrI64,
            (Op::Eq, _) => Eq64,
            (Op::Ne, _) => Ne64,
            (Op::Lt, false) => LtU64,
            (Op::Lt, true) => LtI64,
            (Op::Le, false) => LeU64,
            (Op::Le, true) => LeI64,
            _ => return None,
        },
        (M::F32, o) => match o {
            Op::Add => AddF32,
            Op::Sub => SubF32,
            Op::Mul => MulF32,
            Op::Div => DivF32,
            Op::Eq => EqF32,
            Op::Ne => NeF32,
            Op::Lt => LtF32,
            Op::Le => LeF32,
            _ => return None,
        },
        (M::F64, o) => match o {
            Op::Add => AddF64,
            Op::Sub => SubF64,
            Op::Mul => MulF64,
            Op::Div => DivF64,
            Op::Eq => EqF64,
            Op::Ne => NeF64,
            Op::Lt => LtF64,
            Op::Le => LeF64,
            _ => return None,
        },
        _ => return None,
    };
    Some((i, swap))
}

impl Gen<'_> {
    fn emit(&mut self, i: I, f: u32, w: [u32; 4]) -> usize {
        let at = self.code.len();
        self.code.push(i as u32 | f << 8);
        self.code.extend_from_slice(&w);
        at
    }

    /// The instruction index the next instruction has.
    fn here(&self) -> u32 {
        (self.code.len() / WORDS) as u32
    }

    /// Jump instruction `at`'s target, word `k`, set to here.
    fn patch(&mut self, at: usize, k: usize) {
        let h = self.here();
        self.code[at + k] = h;
    }

    fn jzm(&mut self, m: u32) -> usize {
        self.emit(I::Jzm, 0, [m, 0, 0, 0])
    }

    fn slot(&mut self) -> u32 {
        self.mtop += 1;
        self.mn = self.mn.max(self.mtop);
        self.mtop - 1
    }

    fn alloc(&mut self, c: Class, m: M) -> Val {
        let r = match c {
            Class::S => self.s.alloc(),
            Class::V32 => self.v32.alloc(),
            Class::V64 => self.v64.alloc(),
        };
        Val { r, c, m }
    }

    fn free(&mut self, x: Val) {
        match x.c {
            Class::S => self.s.free.push(x.r),
            Class::V32 => self.v32.free.push(x.r),
            Class::V64 => self.v64.free.push(x.r),
        }
    }

    fn free_opt(&mut self, x: Option<Val>) {
        if let Some(x) = x {
            self.free(x);
        }
    }

    /// `x` as a scalar.
    fn scalar(&mut self, x: Val) -> Val {
        if x.c == Class::S {
            return x;
        }
        let d = self.alloc(Class::S, x.m);
        let i = if x.c == Class::V32 { I::Extract32 } else { I::Extract64 };
        self.emit(i, x.m.0 & 0xff, [d.r, x.r, 0, 0]);
        self.free(x);
        d
    }

    /// `x` as a vector (of its type's lanes).
    fn vector(&mut self, x: Val) -> Val {
        if x.c != Class::S {
            return x;
        }
        let c = lanes_of(x.m);
        let d = self.alloc(c, x.m);
        self.emit(if c == Class::V32 { I::Bcast32 } else { I::Bcast64 }, 0, [d.r, x.r, 0, 0]);
        self.free(x);
        d
    }

    /// `x` as a value of `m`'s lanes (a scalar left one), converting between 32- and 64-bit lanes by `x`'s sign.
    fn fit(&mut self, x: Val, m: M) -> Val {
        if x.c == Class::S || lanes_of(m) == x.c {
            return Val { m, ..x };
        }
        self.cast(x, x.m, m)
    }

    fn value(&mut self, n: usize) -> R<Val> {
        self.expr(n)?.ok_or_else(|| Error(format!("a {:?} node has no value", self.v.kind(n))))
    }

    /// Whether node `n`'s value is computed once for its SIMD group: uniform in it, without effect, a value.
    fn once(&self, n: usize) -> bool {
        self.v.level(n) <= Level::SimdGroup && self.v.field(n) & EFFECT == 0
    }

    /// If node `a` addresses a lane's own variable directly (a `Local`, plus constant offsets): its base and
    /// stride (`LdL`).
    fn direct(&self, a: usize) -> Option<(u32, u32)> {
        let v = self.v;
        match v.kind(a) {
            Kind::Local => {
                let p = v.p(a);
                Some((p[0] * W as u32 + p[2], p[1]))
            }
            Kind::Bin if matches!(Op::of(v.w(a, 0)), Op::Add | Op::Sub) && M(v.w(a, 1)) == M::PTR => {
                let (base, stride) = self.direct(v.child(a, 0))?;
                let c = constant(v, v.child(a, 1))? as u32;
                Some((if Op::of(v.w(a, 0)) == Op::Add { base.wrapping_add(c) } else { base.wrapping_sub(c) }, stride))
            }
            _ => None,
        }
    }

    // Statements: entered with `CUR` not empty.

    fn stmt(&mut self, n: usize) -> R<()> {
        let v = self.v;
        match v.kind(n) {
            Kind::Block => {
                let kids: Vec<usize> = v.children(n).collect();
                let mut skips = Vec::new();
                for (k, &c) in kids.iter().enumerate() {
                    self.stmt(c)?;
                    if self.exits[c] && k + 1 < kids.len() {
                        skips.push(self.jzm(CUR));
                    }
                }
                for s in skips {
                    self.patch(s, 2);
                }
            }
            Kind::Expr => {
                let x = self.expr(v.child(n, 0))?;
                self.free_opt(x);
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
                    self.free_opt(x);
                    self.emit(I::MOr, 0, [END, CUR, 0, 0]);
                } else {
                    if let Some(x) = x {
                        self.store_slot(slot, m, x);
                    }
                    self.emit(I::MOr, 0, [RET, CUR, 0, 0]);
                }
                self.emit(I::MClr, 0, [CUR, 0, 0, 0]);
            }
            Kind::Break => {
                self.emit(I::MOr, 0, [BRK, CUR, 0, 0]);
                self.emit(I::MClr, 0, [CUR, 0, 0, 0]);
            }
            Kind::Continue => {
                self.emit(I::MOr, 0, [CONT, CUR, 0, 0]);
                self.emit(I::MClr, 0, [CUR, 0, 0, 0]);
            }
            _ => {
                let x = self.expr(n)?;
                self.free_opt(x);
            }
        }
        Ok(())
    }

    /// `x` (of `m`) stored in an inlined call's result slot, each running lane's.
    fn store_slot(&mut self, slot: u32, m: M, x: Val) {
        let base = slot * W as u32;
        if m.is_agg() {
            let to = self.alloc(Class::V64, M::PTR);
            self.emit(I::LAddr, 0, [to.r, base, m.bytes(), 0]);
            let from = self.vector(x);
            self.emit(I::VCopy, 0, [to.r, from.r, m.bytes(), 0]);
            self.free(from);
            self.free(to);
        } else {
            let x = self.fit(x, m);
            let x = self.vector(x);
            self.emit(I::StL, m.0, [base, m.bytes(), x.r, 0]);
            self.free(x);
        }
    }

    /// A condition's value as 32-bit lanes or a scalar (it is a `bool`).
    fn cond(&mut self, n: usize) -> R<Val> {
        let c = self.value(n)?;
        Ok(if c.c == Class::V64 { self.cast(c, c.m, M::B) } else { c })
    }

    fn branch(&mut self, n: usize) -> R<()> {
        let v = self.v;
        let site = v.w(n, 0);
        let check = v.field(n) & CHECK != 0;
        let kids: Vec<usize> = v.children(n).collect();
        let c = self.cond(kids[0])?;
        if c.c == Class::S {
            if check {
                self.emit(I::Agree, 0, [n as u32, site, c.r, 0]);
            }
            let j = self.emit(I::Jzs, 0, [c.r, 0, 0, 0]);
            self.free(c);
            self.stmt(kids[1])?;
            if kids.len() > 2 {
                let over = self.emit(I::Jmp, 0, [0; 4]);
                self.patch(j, 2);
                self.stmt(kids[2])?;
                self.patch(over, 1);
            } else {
                self.patch(j, 2);
            }
            return Ok(());
        }
        let (t, e, s0) = (self.slot(), self.slot(), self.slot());
        self.emit(I::MAnd, 0, [t, CUR, c.r, 0]);
        self.emit(I::MAndN, 0, [e, CUR, c.r, 0]);
        self.free(c);
        self.emit(I::MSet, 0, [s0, CUR, 0, 0]);
        if check {
            self.emit(I::AgreeM, 0, [n as u32, site, t, 0]);
        }
        self.emit(I::MSet, 0, [CUR, t, 0, 0]);
        let j = self.jzm(CUR);
        self.stmt(kids[1])?;
        self.patch(j, 2);
        if kids.len() > 2 {
            self.emit(I::MSet, 0, [CUR, e, 0, 0]);
            let j = self.jzm(CUR);
            self.stmt(kids[2])?;
            self.patch(j, 2);
        }
        self.emit(I::MSet, 0, [CUR, s0, 0, 0]);
        if self.exits[n] {
            self.emit(I::MLive, 0, [0; 4]);
        }
        self.mtop -= 3;
        Ok(())
    }

    /// A `switch`. On a value the SIMD group shares, a `Switch` to its case, and the cases in order, each falling
    /// through to the next as C's do. On one that varies, a round for each value its running lanes have: the first
    /// remaining lane's value, its lanes (those that have it) through the same `Switch`, so a round costs one
    /// comparison of the lanes, not one for each label. Past it, the lanes that came in, less those that took a
    /// `continue` or a `return` (its breaks' lanes go on).
    fn switch(&mut self, n: usize) -> R<()> {
        let v = self.v;
        let (site, m) = (v.w(n, 0), M(v.w(n, 1)));
        let check = v.field(n) & CHECK != 0;
        let kids: Vec<usize> = v.children(n).collect();
        let c = self.value(kids[0])?;
        let c = self.fit(c, m);
        let (s0, sb) = (self.slot(), self.slot());
        self.emit(I::MSet, 0, [s0, CUR, 0, 0]);
        self.emit(I::MSet, 0, [sb, BRK, 0, 0]);
        self.emit(I::MClr, 0, [BRK, 0, 0, 0]);
        let cases = &kids[1..];
        if c.c == Class::S || check {
            let c = self.scalar(c);
            if check {
                self.emit(I::Agree, 0, [n as u32, site, c.r, 0]);
            }
            self.cases(c.r, m, cases)?;
            self.free(c);
        } else {
            let c = self.vector(c);
            let rest = self.slot();
            self.emit(I::MSet, 0, [rest, CUR, 0, 0]);
            let top = self.here();
            let done = self.jzm(rest);
            self.emit(I::MSet, 0, [CUR, rest, 0, 0]);
            let x = self.alloc(Class::S, m);
            let wide = c.c == Class::V64;
            self.emit(if wide { I::Extract64 } else { I::Extract32 }, m.0, [x.r, c.r, 0, 0]);
            let (b, t) = (self.alloc(c.c, m), self.alloc(Class::V32, M::B));
            self.emit(if wide { I::Bcast64 } else { I::Bcast32 }, 0, [b.r, x.r, 0, 0]);
            self.emit(if wide { I::Eq64 } else { I::Eq32 }, 0, [t.r, c.r, b.r, 0]);
            self.emit(I::MAnd, 0, [CUR, rest, t.r, 0]);
            self.emit(I::MAndNM, 0, [rest, rest, CUR, 0]);
            self.free(t);
            self.free(b);
            self.cases(x.r, m, cases)?;
            self.free(x);
            self.emit(I::Jmp, 0, [top, 0, 0, 0]);
            self.patch(done, 2);
            self.free(c);
            self.mtop -= 1;
        }
        self.emit(I::MSwEnd, 0, [s0, 0, 0, 0]);
        self.emit(I::MSet, 0, [BRK, sb, 0, 0]);
        self.mtop -= 2;
        Ok(())
    }

    /// A `Switch` by scalar `x` (of `m`) to `cases`, then the cases in order: each runs from its own (or the case
    /// before's) lanes, unless none are left.
    fn cases(&mut self, x: u32, m: M, cases: &[usize]) -> R<()> {
        let v = self.v;
        let labelled: Vec<usize> = cases.iter().copied().filter(|&k| v.w(k, 0) == 0).collect();
        let at = self.emit(I::Switch, 0, [x, labelled.len() as u32, 0, 0]);
        let table = self.code.len();
        for _ in &labelled {
            self.emit(I::Label, 0, [0; 4]);
        }
        let (mut entries, mut default, mut outs) = (Vec::new(), None, Vec::new());
        for &k in cases {
            let start = self.here();
            if v.w(k, 0) != 0 {
                default = Some(start);
            } else {
                entries.push((crate::ops::norm(m, u64::from(v.w(k, 1)) | u64::from(v.w(k, 2)) << 32), start));
            }
            outs.push(self.jzm(CUR));
            self.stmt(v.child(k, 0))?;
        }
        let end = self.here();
        for o in outs {
            self.code[o + 2] = end;
        }
        self.code[at + 3] = default.unwrap_or(end);
        entries.sort_unstable();
        for (e, (x, t)) in entries.into_iter().enumerate() {
            self.code[table + e * WORDS + 1..][..3].copy_from_slice(&[x as u32, (x >> 32) as u32, t]);
        }
        Ok(())
    }

    fn loop_(&mut self, n: usize) -> R<()> {
        let v = self.v;
        let k = v.kind(n);
        let site = v.w(n, 0);
        let check = v.field(n) & CHECK != 0;
        let kids: Vec<usize> = v.children(n).collect();
        let (cond, body) = if k == Kind::Do { (kids[1], kids[0]) } else { (kids[0], kids[1]) };
        let (sl, sb, sc, si) = (self.slot(), self.slot(), self.slot(), self.slot());
        self.emit(I::MSet, 0, [sl, CUR, 0, 0]);
        self.emit(I::MSet, 0, [sb, BRK, 0, 0]);
        self.emit(I::MSet, 0, [sc, CONT, 0, 0]);
        self.emit(I::MClr, 0, [BRK, 0, 0, 0]);
        let first = (k == Kind::Do).then(|| self.emit(I::Jmp, 0, [0; 4]));
        let top = self.here();
        let mut outs = vec![self.jzm(CUR)];
        let c = self.cond(cond)?;
        if c.c == Class::S {
            if check {
                self.emit(I::Agree, 0, [n as u32, site, c.r, 0]);
            }
            outs.push(self.emit(I::Jzs, 0, [c.r, 0, 0, 0]));
        } else {
            self.emit(I::MAnd, 0, [CUR, CUR, c.r, 0]);
            if check {
                self.emit(I::AgreeM, 0, [n as u32, site, CUR, 0]);
            }
            outs.push(self.jzm(CUR));
        }
        self.free(c);
        if let Some(f) = first {
            self.patch(f, 1);
        }
        self.emit(I::MSet, 0, [si, CUR, 0, 0]);
        self.emit(I::MClr, 0, [CONT, 0, 0, 0]);
        self.stmt(body)?;
        self.emit(I::MRejoin, 0, [si, 0, 0, 0]);
        if k == Kind::For && v.kind(kids[2]) != Kind::Empty {
            outs.push(self.jzm(CUR));
            self.stmt(kids[2])?;
        }
        self.emit(I::Jmp, 0, [top, 0, 0, 0]);
        for o in outs {
            self.patch(o, 2);
        }
        self.emit(I::MExit, 0, [sl, 0, 0, 0]);
        self.emit(I::MSet, 0, [BRK, sb, 0, 0]);
        self.emit(I::MSet, 0, [CONT, sc, 0, 0]);
        self.mtop -= 4;
        Ok(())
    }

    // Expressions.

    fn konst(&mut self, m: M, x: u64) -> Val {
        let d = self.alloc(Class::S, m);
        self.emit(I::SConst, 0, [d.r, x as u32, (x >> 32) as u32, 0]);
        d
    }

    /// `x` of `from` converted to `to`, in its file (a scalar's by `SCast`).
    fn cast(&mut self, x: Val, from: M, to: M) -> Val {
        if x.c == Class::S {
            if from == to {
                return Val { m: to, ..x };
            }
            let d = self.alloc(Class::S, to);
            self.emit(I::SCast, fields(from.0, to.0, 0), [d.r, x.r, 0, 0]);
            self.free(x);
            return d;
        }
        let (fc, tc) = (lanes_of(from), lanes_of(to));
        let int = !from.float() && !to.float() && !from.is_agg() && !to.is_agg();
        let one = |g: &mut Self, i: I, x: Val, c: Class| -> Val {
            let d = g.alloc(c, to);
            g.emit(i, 0, [d.r, x.r, 0, 0]);
            g.free(x);
            d
        };
        if !int {
            if from == to {
                return Val { m: to, ..x };
            }
            let d = self.alloc(tc, to);
            self.emit(I::VCastGen, fields(from.0, to.0, 0), [d.r, x.r, 0, 0]);
            self.free(x);
            return d;
        }
        // The value at 32 bits if it is to be in 32-bit lanes.
        let x = match (fc, tc) {
            (Class::V64, Class::V32) if to == M::B => return one(self, I::Truth64, x, Class::V32),
            (Class::V64, Class::V32) => one(self, I::Trunc, x, Class::V32),
            (Class::V32, Class::V64) => return one(self, if from.signed() { I::Sext } else { I::Zext }, x, Class::V64),
            (Class::V64, _) => return Val { m: to, ..x },
            _ => x,
        };
        match to {
            M::B if from != M::B => one(self, I::Truth32, x, Class::V32),
            M::I8 => one(self, I::Sx8, x, Class::V32),
            M::U8 => one(self, I::Zx8, x, Class::V32),
            M::I16 => one(self, I::Sx16, x, Class::V32),
            M::U16 => one(self, I::Zx16, x, Class::V32),
            _ => Val { m: to, ..x },
        }
    }

    fn expr(&mut self, n: usize) -> R<Option<Val>> {
        let v = self.v;
        let p = v.p(n);
        let once = self.once(n);
        let kids: Vec<usize> = v.children(n).collect();
        Ok(Some(match v.kind(n) {
            Kind::Const => self.konst(M(p[0]), u64::from(p[1]) | u64::from(p[2]) << 32),
            Kind::Global | Kind::Shared => {
                let d = self.alloc(Class::S, M::PTR);
                self.emit(if v.kind(n) == Kind::Global { I::SGlobal } else { I::SShared }, 0, [d.r, p[0], 0, 0]);
                d
            }
            Kind::Local => {
                let d = self.alloc(Class::V64, M::PTR);
                self.emit(I::LAddr, 0, [d.r, p[0] * W as u32 + p[2], p[1], 0]);
                d
            }
            Kind::Load => {
                let m = M(p[0]);
                if m.is_agg() {
                    return self.expr(kids[0]);
                }
                if let Some((base, stride)) = self.direct(kids[0]) {
                    let d = self.alloc(if once { Class::S } else { lanes_of(m) }, m);
                    self.emit(if once { I::LdLS } else { I::LdL }, m.0, [d.r, base, stride, 0]);
                    d
                } else {
                    let a = self.value(kids[0])?;
                    if once {
                        let a = self.scalar(a);
                        let d = self.alloc(Class::S, m);
                        self.emit(I::SLoad, m.0, [d.r, a.r, 0, 0]);
                        self.free(a);
                        d
                    } else {
                        let a = self.vector(a);
                        let d = self.alloc(lanes_of(m), m);
                        self.emit(I::VLoad, m.0, [d.r, a.r, 0, 0]);
                        self.free(a);
                        d
                    }
                }
            }
            Kind::Store => {
                let m = M(p[0]);
                if let Some((base, stride)) = self.direct(kids[0]) {
                    let x = self.value(kids[1])?;
                    let x = self.fit(x, m);
                    let xv = self.vector(x);
                    self.emit(I::StL, m.0, [base, stride, xv.r, 0]);
                    xv
                } else {
                    let a = self.value(kids[0])?;
                    let x = self.value(kids[1])?;
                    let x = self.fit(x, m);
                    if a.c == Class::S && x.c == Class::S {
                        self.emit(I::SStore, m.0, [a.r, x.r, 0, 0]);
                        self.free(a);
                        x
                    } else {
                        let a = self.vector(a);
                        let x = self.vector(x);
                        self.emit(I::VStore, m.0, [a.r, x.r, 0, 0]);
                        self.free(a);
                        x
                    }
                }
            }
            Kind::Update => {
                let (op, lm, om, post) = (Op::of(p[0]), M(p[1]), M(p[2]), p[3] != 0);
                let direct = self.direct(kids[0]);
                let a = if direct.is_none() {
                    let a = self.value(kids[0])?;
                    Some(self.vector(a))
                } else {
                    None
                };
                let y = self.value(kids[1])?;
                let old = self.alloc(lanes_of(lm), lm);
                match (direct, a) {
                    (Some((base, stride)), _) => self.emit(I::LdL, lm.0, [old.r, base, stride, 0]),
                    (None, Some(a)) => self.emit(I::VLoad, lm.0, [old.r, a.r, 0, 0]),
                    _ => unreachable!("an update's address"),
                };
                // `old` stays for a postfix's value; the operation on a copy of it.
                let o = self.copy(old);
                let o = self.cast(o, lm, om);
                let y = self.fit(y, om);
                let r = self.bin(op, om, o, y);
                let new = self.cast(r, om, lm);
                let new = self.vector(new);
                match (direct, a) {
                    (Some((base, stride)), _) => self.emit(I::StL, lm.0, [base, stride, new.r, 0]),
                    (None, Some(a)) => {
                        let at = self.emit(I::VStore, lm.0, [a.r, new.r, 0, 0]);
                        self.free(a);
                        at
                    }
                    _ => unreachable!("an update's address"),
                };
                if post {
                    self.free(new);
                    old
                } else {
                    self.free(old);
                    new
                }
            }
            Kind::Zero => {
                let a = self.value(kids[0])?;
                let a = self.vector(a);
                self.emit(I::VZero, 0, [a.r, p[1], 0, 0]);
                a
            }
            Kind::Copy => {
                let d = self.value(kids[0])?;
                let d = self.vector(d);
                let s = self.value(kids[1])?;
                let s = self.vector(s);
                self.emit(I::VCopy, 0, [d.r, s.r, p[0], 0]);
                self.free(s);
                d
            }
            Kind::Bin => {
                let (op, m) = (Op::of(p[0]), M(p[1]));
                let a = self.value(kids[0])?;
                let b = self.value(kids[1])?;
                let (a, b) = (self.fit(a, m), self.fit(b, m));
                if once {
                    let (a, b) = (self.scalar(a), self.scalar(b));
                    let rm = if op.compares() { M::B } else { m };
                    let d = self.alloc(Class::S, rm);
                    self.emit(I::SBin, fields(op as u32, m.0, 0), [d.r, a.r, b.r, 0]);
                    self.free(a);
                    self.free(b);
                    d
                } else {
                    self.bin(op, m, a, b)
                }
            }
            Kind::Un => {
                let (op, m) = (Op::of(p[0]), M(p[1]));
                let a = self.value(kids[0])?;
                let a = self.fit(a, m);
                if once {
                    let a = self.scalar(a);
                    let d = self.alloc(Class::S, m);
                    self.emit(I::SUn, fields(op as u32, m.0, 0), [d.r, a.r, 0, 0]);
                    self.free(a);
                    d
                } else {
                    let a = self.vector(a);
                    let f = match (op, a.c, m) {
                        (Op::Neg, _, M::F32) => Some(I::NegF32),
                        (Op::Neg, _, M::F64) => Some(I::NegF64),
                        (Op::Neg, Class::V32, M::I32 | M::U32) => Some(I::Neg32),
                        (Op::Neg, Class::V64, M::I64 | M::U64 | M::PTR) => Some(I::Neg64),
                        (Op::Not, Class::V32, M::I32 | M::U32) => Some(I::Not32),
                        (Op::Not, Class::V64, M::I64 | M::U64 | M::PTR) => Some(I::Not64),
                        (Op::LNot, Class::V32, m) if !m.float() => Some(I::LNot32),
                        (Op::LNot, Class::V64, m) if !m.float() => Some(I::LNot64),
                        _ => None,
                    };
                    let d = self.alloc(a.c, m);
                    match f {
                        Some(i) => self.emit(i, 0, [d.r, a.r, 0, 0]),
                        None => self.emit(I::VUnGen, fields(op as u32, m.0, 0), [d.r, a.r, 0, 0]),
                    };
                    self.free(a);
                    d
                }
            }
            Kind::Cast => {
                let (from, to) = (M(p[0]), M(p[1]));
                let a = self.value(kids[0])?;
                let a = self.fit(a, from);
                if once {
                    let a = self.scalar(a);
                    self.cast(a, from, to)
                } else {
                    self.cast(a, from, to)
                }
            }
            Kind::Comma => {
                let a = self.expr(kids[0])?;
                self.free_opt(a);
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

    /// A copy of vector `x` in a new register.
    fn copy(&mut self, x: Val) -> Val {
        let d = self.alloc(x.c, x.m);
        let i = if x.c == Class::V32 { I::VMovM32 } else { I::VMovM64 };
        self.emit(i, 0, [d.r, x.r, 0, 0]);
        d
    }

    /// `a op b` on vectors of `m` (`a`, `b` freed).
    fn bin(&mut self, op: Op, m: M, a: Val, b: Val) -> Val {
        let (a, b) = (self.vector(a), self.vector(b));
        let rc = if op.compares() { Class::V32 } else { lanes_of(m) };
        let rm = if op.compares() { M::B } else { m };
        let d = self.alloc(rc, rm);
        match fused_bin(op, m) {
            Some((i, swap)) => {
                let (x, y) = if swap { (b, a) } else { (a, b) };
                self.emit(i, 0, [d.r, x.r, y.r, 0]);
            }
            None => {
                self.emit(I::VBinGen, fields(op as u32, m.0, 0), [d.r, a.r, b.r, 0]);
            }
        }
        self.free(a);
        self.free(b);
        d
    }

    /// `?:`, `&&` or `||`: its value's lanes from its operands' under their masks, or by a jump on a uniform
    /// condition.
    fn choose(&mut self, n: usize, once: bool) -> R<Val> {
        let v = self.v;
        let logic = v.kind(n) == Kind::Logic;
        let and = logic && v.w(n, 0) != 0;
        let kids: Vec<usize> = v.children(n).collect();
        let m = if logic { M::B } else { M(v.w(n, 0)) };
        let m = if m.is_agg() { M::PTR } else { m };
        let c = self.cond(kids[0])?;
        // A uniform value's condition is uniform too: a jump.
        let c = if once { self.scalar(c) } else { c };
        let class = if once { Class::S } else { lanes_of(m) };
        let d = self.alloc(class, m);
        // The value of operand `k` into `d`, under the lanes running.
        let put = |g: &mut Self, k: usize| -> R<()> {
            let x = g.value(kids[k])?;
            let x = if logic { g.cast(x, x.m, M::B) } else { g.fit(x, m) };
            let x = if class == Class::S { g.scalar(x) } else { g.vector(x) };
            let i = match class {
                Class::S => I::SMov,
                Class::V32 => I::VMovM32,
                Class::V64 => I::VMovM64,
            };
            g.emit(i, 0, [d.r, x.r, 0, 0]);
            g.free(x);
            Ok(())
        };
        if c.c == Class::S {
            // `&&` and `||`: the first operand's truth, then the second's where it does not decide.
            if logic {
                let t = self.cast(c, c.m, M::B);
                if class == Class::S {
                    self.emit(I::SMov, 0, [d.r, t.r, 0, 0]);
                } else {
                    let tv = self.alloc(Class::V32, M::B);
                    self.emit(I::Bcast32, 0, [tv.r, t.r, 0, 0]);
                    self.emit(I::VMovM32, 0, [d.r, tv.r, 0, 0]);
                    self.free(tv);
                }
                let j = self.emit(if and { I::Jzs } else { I::Jnzs }, 0, [t.r, 0, 0, 0]);
                self.free(t);
                put(self, 1)?;
                self.patch(j, 2);
            } else {
                let j = self.emit(I::Jzs, 0, [c.r, 0, 0, 0]);
                self.free(c);
                put(self, 1)?;
                let over = self.emit(I::Jmp, 0, [0; 4]);
                self.patch(j, 2);
                put(self, 2)?;
                self.patch(over, 1);
            }
            return Ok(d);
        }
        let (t, e, s0) = (self.slot(), self.slot(), self.slot());
        self.emit(I::MAnd, 0, [t, CUR, c.r, 0]);
        self.emit(I::MAndN, 0, [e, CUR, c.r, 0]);
        self.emit(I::MSet, 0, [s0, CUR, 0, 0]);
        if logic {
            // `d` is the first operand's truth; the lanes it does not decide take the second's.
            self.emit(I::Truth32, 0, [d.r, c.r, 0, 0]);
            self.free(c);
            self.emit(I::MSet, 0, [CUR, if and { t } else { e }, 0, 0]);
            let j = self.jzm(CUR);
            put(self, 1)?;
            self.patch(j, 2);
        } else {
            self.free(c);
            self.emit(I::MSet, 0, [CUR, t, 0, 0]);
            let j = self.jzm(CUR);
            put(self, 1)?;
            self.patch(j, 2);
            self.emit(I::MSet, 0, [CUR, e, 0, 0]);
            let j = self.jzm(CUR);
            put(self, 2)?;
            self.patch(j, 2);
        }
        self.emit(I::MSet, 0, [CUR, s0, 0, 0]);
        if self.exits[n] {
            self.emit(I::MLive, 0, [0; 4]);
        }
        self.mtop -= 3;
        Ok(d)
    }

    fn builtin(&mut self, n: usize, once: bool) -> R<Option<Val>> {
        let v = self.v;
        let p = v.p(n);
        let (b, m) = (Builtin::of(p[0]), M(p[1]));
        let kids: Vec<usize> = v.children(n).collect();
        let mut args = Vec::with_capacity(kids.len());
        for &c in &kids {
            args.push(self.value(c)?);
        }
        let free_all = |g: &mut Self, args: Vec<Val>| {
            for a in args {
                g.free(a);
            }
        };
        let arg_m = |k: usize| if k < kids.len() { M(p[4 + k]) } else { M::VOID };
        Ok(Some(match b {
            Builtin::LocalId | Builtin::GlobalId => {
                let dim = self.scalar(args[0]);
                let c = lanes_of(m);
                let d = self.alloc(c, m);
                let i = if b == Builtin::LocalId { I::VLid } else { I::VGid };
                self.emit(i, u32::from(c == Class::V64), [d.r, dim.r, 0, 0]);
                self.free(dim);
                d
            }
            Builtin::SubGroupLocalId => {
                let d = self.alloc(Class::V32, M::U32);
                self.emit(I::VLane, 0, [d.r, 0, 0, 0]);
                free_all(self, args);
                self.fit(d, m)
            }
            Builtin::Barrier => {
                free_all(self, args);
                self.emit(I::Bar, 0, [p[3], 0, 0, 0]);
                return Ok(None);
            }
            Builtin::MemFence | Builtin::SubGroupBarrier => {
                free_all(self, args);
                return Ok(None);
            }
            Builtin::Yield => {
                self.emit(I::Yield, 0, [0; 4]);
                return Ok(None);
            }
            Builtin::SubGroupBallot => {
                let x = args[0];
                let x = if x.c == Class::V64 { self.cast(x, x.m, M::B) } else { x };
                let x = self.vector(x);
                let d = self.alloc(Class::S, m);
                self.emit(I::Ballot, 0, [d.r, x.r, 0, 0]);
                self.free(x);
                d
            }
            Builtin::SubGroupReduceAdd | Builtin::SubGroupReduceMin | Builtin::SubGroupReduceMax => {
                let x = self.fit(args[0], m);
                let x = self.vector(x);
                let d = self.alloc(Class::S, m);
                let i = if x.c == Class::V32 { I::Reduce32 } else { I::Reduce64 };
                self.emit(i, fields(b as u32, m.0, 0), [d.r, x.r, 0, 0]);
                self.free(x);
                d
            }
            Builtin::SubGroupShuffle | Builtin::SubGroupShuffleUp | Builtin::SubGroupShuffleDown | Builtin::SubGroupShuffleXor => {
                let x = self.fit(args[0], m);
                let x = self.vector(x);
                let lane = self.fit(args[1], M::U32);
                let lane = self.vector(lane);
                let d = self.alloc(x.c, m);
                let i = if x.c == Class::V32 { I::Shuf32 } else { I::Shuf64 };
                self.emit(i, b as u32, [d.r, x.r, lane.r, 0]);
                self.free(x);
                self.free(lane);
                d
            }
            _ if crate::vector::is_atomic(b) => {
                // A uniform atomic is the SIMD group's one (as `run`'s was before: its value is every lane's).
                let once = self.v.level(n) <= Level::SimdGroup;
                let mut ops = [0u32; 3];
                let mut held = Vec::new();
                for (k, a) in args.into_iter().enumerate() {
                    let a = if k == 0 { self.fit(a, M::PTR) } else { self.fit(a, m) };
                    let a = if once { self.scalar(a) } else { self.vector(a) };
                    ops[k] = a.r;
                    held.push(a);
                }
                let d = self.alloc(if once { Class::S } else { lanes_of(m) }, m);
                self.emit(if once { I::SAtomic } else { I::VAtomic }, fields(b as u32, m.0, 0), [d.r, ops[0], ops[1], ops[2]]);
                free_all(self, held);
                d
            }
            Builtin::LocalSize | Builtin::GlobalSize | Builtin::GroupId | Builtin::NumGroups => {
                let dim = self.scalar(args[0]);
                let d = self.alloc(Class::S, m);
                self.emit(I::SBuiltin, fields(b as u32, m.0, 0), [d.r, dim.r, 0, 0]);
                self.free(dim);
                d
            }
            _ => {
                // The lanes' own: popcount, clz, mul_hi, as_*, min, max, fma.
                let from = arg_m(0);
                let mut held: Vec<Val> = Vec::new();
                for (k, a) in args.into_iter().enumerate() {
                    let a = self.fit(a, arg_m(k));
                    held.push(if once { self.scalar(a) } else { self.vector(a) });
                }
                let r = |k: usize| held.get(k).map_or(0, |a| a.r);
                if once {
                    let d = self.alloc(Class::S, m);
                    self.emit(I::SBuiltin, fields(b as u32, m.0, from.0 & 0xff), [d.r, r(0), r(1), r(2)]);
                    free_all(self, held);
                    d
                } else {
                    let fused = match (b, m) {
                        (Builtin::Min, M::U32) => Some(I::MinU32),
                        (Builtin::Min, M::I32) => Some(I::MinI32),
                        (Builtin::Max, M::U32) => Some(I::MaxU32),
                        (Builtin::Max, M::I32) => Some(I::MaxI32),
                        (Builtin::Min, M::U64) => Some(I::MinU64),
                        (Builtin::Min, M::I64) => Some(I::MinI64),
                        (Builtin::Max, M::U64) => Some(I::MaxU64),
                        (Builtin::Max, M::I64) => Some(I::MaxI64),
                        (Builtin::Popcount, M::U32 | M::I32) => Some(I::Popcount32),
                        _ => None,
                    };
                    let d = self.alloc(lanes_of(m), m);
                    match fused {
                        Some(i) => self.emit(i, 0, [d.r, r(0), r(1), 0]),
                        None => self.emit(I::VBuiltinGen, fields(b as u32, m.0, from.0 & 0xff), [d.r, r(0), r(1), r(2)]),
                    };
                    free_all(self, held);
                    d
                }
            }
        }))
    }

    /// A call: its arguments' values, then each stored in its callee's parameter (past this function's region, where
    /// the callee's frame is), then the callee run under the lanes that call it; its returned lanes back; its result
    /// read from its slot.
    fn call(&mut self, n: usize) -> R<Option<Val>> {
        let v = self.v;
        let p = v.p(n);
        let (m, temp, id) = (M(p[1]), p[2], p[3]);
        let (body, _) = self.bodies[&id];
        let bp = v.p(body);
        let slot = bp[3];
        let kids: Vec<usize> = v.children(n).collect();
        // Every argument first: one with a call in it runs that call in the same place.
        let mut vals = Vec::with_capacity(kids.len());
        for (k, &a) in kids.iter().enumerate() {
            let pm = M(bp[8 + 2 * k]);
            let x = self.value(a)?;
            let x = if pm.is_agg() { x } else { self.fit(x, pm) };
            vals.push(self.vector(x));
        }
        let adv = self.region;
        for (k, x) in vals.into_iter().enumerate() {
            let (off, pm) = (bp[7 + 2 * k], M(bp[8 + 2 * k]));
            let base = (adv + off) * W as u32;
            if pm.is_agg() {
                let to = self.alloc(Class::V64, M::PTR);
                self.emit(I::LAddr, 0, [to.r, base, pm.bytes(), 0]);
                self.emit(I::VCopy, 0, [to.r, x.r, pm.bytes(), 0]);
                self.free(to);
            } else {
                self.emit(I::StL, pm.0, [base, pm.bytes(), x.r, 0]);
            }
            self.free(x);
        }
        let (sa, sr) = (self.slot(), self.slot());
        self.emit(I::MSet, 0, [sa, CUR, 0, 0]);
        self.emit(I::MSet, 0, [sr, RET, 0, 0]);
        self.emit(I::MClr, 0, [RET, 0, 0, 0]);
        let at = self.emit(I::Call, 0, [0, self.row, 0, 0]);
        self.calls.push((at, id));
        self.emit(I::MResume, 0, [sa, 0, 0, 0]);
        self.emit(I::MSet, 0, [RET, sr, 0, 0]);
        self.mtop -= 2;
        let base = (adv + slot) * W as u32;
        Ok(if m == M::VOID {
            None
        } else if m.is_agg() {
            let from = self.alloc(Class::V64, M::PTR);
            self.emit(I::LAddr, 0, [from.r, base, m.bytes(), 0]);
            let to = self.alloc(Class::V64, M::PTR);
            self.emit(I::LAddr, 0, [to.r, temp * W as u32, m.bytes(), 0]);
            self.emit(I::VCopy, 0, [to.r, from.r, m.bytes(), 0]);
            self.free(from);
            Some(to)
        } else {
            let d = self.alloc(lanes_of(m), m);
            self.emit(I::LdL, m.0, [d.r, base, m.bytes(), 0]);
            Some(d)
        })
    }
}
