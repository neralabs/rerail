//! Constants carried through a function's integer operations, as an optimizing compiler's
//! first pass does before it emits code.
//!
//! The code that builds tier 1 makes a value constant where it knows it: a test that a
//! gate's quick path has shown is none (`native_cpu_bytes::words_gated`), a mark that every
//! token of a kind sets. The operations after it are still written out (an `and` with the
//! zero, its count of bits, a compare of that count), and so is a branch on it. Neither
//! backend folds them: the JIT runs Cranelift without its optimizer, and the Rust that
//! `clif_rust` prints would carry them for `rustc` to remove.
//!
//! `fold` does that work on the IR itself, before either backend sees it:
//!
//! ```text
//! v1 = iconst.i64 0          ; a test's word, known none
//! v2 = band v0, v1           ; -> v2 = iconst 0
//! v3 = popcnt v2             ; -> v3 = iconst 0
//! v4 = icmp_imm uge v3, 2    ; -> v4 = iconst.i8 0
//! brif v4, block7, block8    ; -> jump block8
//! ```
//!
//! An operation with all its operands constant becomes their constant. One with a single
//! constant operand that decides it (`x & 0`, `x | 0`, `x + 0`, a `select` on a constant)
//! becomes that constant or the other operand. A branch on a constant becomes a jump. The
//! blocks no longer reached and the pure operations no longer read are then removed. Only
//! scalar integers are folded; vectors, loads, stores and calls are left as they are.

use cranelift_codegen::cursor::{Cursor, FuncCursor};
use cranelift_codegen::ir::{self, InstBuilder, InstructionData, Opcode, Value, condcodes::IntCC};
use std::collections::{HashMap, HashSet};

/// What is known of a value's bits: those known zero and those known one, within its width.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Bits {
    pub zeros: u64,
    pub ones: u64,
}

impl Bits {
    fn exact(c: u64, w: u32) -> Bits {
        Bits { zeros: !c & mask(w), ones: c & mask(w) }
    }

    /// Its value, where every bit of `w` is known.
    fn value(self, w: u32) -> Option<u64> {
        ((self.zeros | self.ones) & mask(w) == mask(w)).then_some(self.ones)
    }

    /// What is known of either: the bits both know alike.
    fn meet(self, o: Bits) -> Bits {
        Bits { zeros: self.zeros & o.zeros, ones: self.ones & o.ones }
    }
}

/// Each function's results' known bits (`returns`), by its user name's index (`u0:N`): a call
/// of it knows them of its results.
pub type Summaries = HashMap<u32, Vec<Bits>>;

/// The function with its constants folded, its branches on constants taken, and what is no
/// longer reached or read removed. Whether anything changed.
pub fn fold(func: &mut ir::Function) -> bool {
    fold_with(func, &Summaries::new())
}

/// `fold`, the results of calls of the functions of `callees` known as they say.
pub fn fold_with(func: &mut ir::Function, callees: &Summaries) -> bool {
    let mut changed = false;
    loop {
        let mut again = fold_once(func, callees);
        again |= same_params(func);
        again |= unreached(func);
        again |= merge_straight(func);
        again |= unread(func);
        if !again {
            break;
        }
        changed = true;
    }
    changed
}

/// Functions that call each other, each folded with the others' results known (`returns`),
/// until no summary changes: a callee's constant result is a constant in its callers.
pub fn fold_all(funcs: &mut [&mut ir::Function]) -> Summaries {
    let mut sums = Summaries::new();
    for _ in 0..4 {
        let mut moved = false;
        for f in funcs.iter_mut() {
            fold_with(f, &sums);
            if let Some(id) = user_index(f) {
                let r = returns(f, &sums);
                if sums.get(&id) != Some(&r) {
                    sums.insert(id, r);
                    moved = true;
                }
            }
        }
        if !moved {
            break;
        }
    }
    sums
}

/// A function's user name's index (`u0:N`).
pub fn user_index(func: &ir::Function) -> Option<u32> {
    match &func.name {
        ir::UserFuncName::User(u) => Some(u.index),
        _ => None,
    }
}

/// What every `return` of the function knows of each result.
pub fn returns(func: &ir::Function, callees: &Summaries) -> Vec<Bits> {
    let bits = known(func, callees);
    let n = func.signature.returns.len();
    let mut out: Option<Vec<Bits>> = None;
    for b in func.layout.blocks() {
        let Some(last) = func.layout.last_inst(b) else { continue };
        if func.dfg.insts[last].opcode() != Opcode::Return {
            continue;
        }
        let here: Vec<Bits> = func.dfg.inst_args(last).iter().map(|&v| bits.get(&func.dfg.resolve_aliases(v)).copied().unwrap_or_default()).collect();
        out = Some(match out {
            None => here,
            Some(o) => o.into_iter().zip(here).map(|(x, y)| x.meet(y)).collect(),
        });
    }
    out.unwrap_or_else(|| vec![Bits::default(); n])
}

/// A scalar integer type's width in bits.
fn width(t: ir::Type) -> Option<u32> {
    (t.is_int() && !t.is_vector()).then(|| t.bits())
}

fn mask(w: u32) -> u64 {
    if w >= 64 { u64::MAX } else { (1u64 << w) - 1 }
}

/// `v`, `w` bits wide, sign-extended.
fn signed(v: u64, w: u32) -> i64 {
    if w >= 64 { v as i64 } else { ((v << (64 - w)) as i64) >> (64 - w) }
}

/// What an operation folds to.
enum Folded {
    Const(u64),
    Same(Value),
    /// `x + k`, two adds of constants joined.
    AddImm(Value, i64),
}

fn binary(op: Opcode, x: u64, y: u64, w: u32) -> Option<u64> {
    let sh = (y & u64::from(w - 1)) as u32;
    Some(match op {
        Opcode::Band => x & y,
        Opcode::Bor => x | y,
        Opcode::Bxor => x ^ y,
        Opcode::BandNot => x & !y,
        Opcode::BorNot => x | !y,
        Opcode::Iadd => x.wrapping_add(y),
        Opcode::Isub => x.wrapping_sub(y),
        Opcode::Imul => x.wrapping_mul(y),
        Opcode::Ishl => x << sh,
        Opcode::Ushr => (x & mask(w)) >> sh,
        Opcode::Sshr => (signed(x, w) >> sh) as u64,
        Opcode::Umin => (x & mask(w)).min(y & mask(w)),
        Opcode::Umax => (x & mask(w)).max(y & mask(w)),
        Opcode::Smin => signed(x, w).min(signed(y, w)) as u64,
        Opcode::Smax => signed(x, w).max(signed(y, w)) as u64,
        _ => return None,
    } & mask(w))
}

/// The known bits of `op` of two values' known bits, where the bit logic or a constant shift
/// says them.
fn binary_bits(op: Opcode, x: Bits, y: Bits, w: u32) -> Bits {
    let m = mask(w);
    match op {
        Opcode::Band => Bits { zeros: x.zeros | y.zeros, ones: x.ones & y.ones },
        Opcode::Bor => Bits { zeros: x.zeros & y.zeros, ones: x.ones | y.ones },
        Opcode::Bxor => Bits { zeros: (x.zeros & y.zeros) | (x.ones & y.ones), ones: (x.ones & y.zeros) | (x.zeros & y.ones) },
        Opcode::BandNot => Bits { zeros: x.zeros | y.ones, ones: x.ones & y.zeros },
        Opcode::Ishl | Opcode::Ushr => match y.value(w) {
            Some(k) => {
                let k = (k & u64::from(w - 1)) as u32;
                if op == Opcode::Ishl {
                    Bits { zeros: ((x.zeros << k) | ((1u64 << k) - 1)) & m, ones: (x.ones << k) & m }
                } else {
                    Bits { zeros: ((x.zeros & m) >> k) | (!(m >> k) & m), ones: (x.ones & m) >> k }
                }
            }
            None => Bits::default(),
        },
        _ => Bits::default(),
    }
}

fn compare(cc: IntCC, x: u64, y: u64, w: u32) -> bool {
    let (ux, uy, sx, sy) = (x & mask(w), y & mask(w), signed(x, w), signed(y, w));
    match cc {
        IntCC::Equal => ux == uy,
        IntCC::NotEqual => ux != uy,
        IntCC::UnsignedLessThan => ux < uy,
        IntCC::UnsignedLessThanOrEqual => ux <= uy,
        IntCC::UnsignedGreaterThan => ux > uy,
        IntCC::UnsignedGreaterThanOrEqual => ux >= uy,
        IntCC::SignedLessThan => sx < sy,
        IntCC::SignedLessThanOrEqual => sx <= sy,
        IntCC::SignedGreaterThan => sx > sy,
        IntCC::SignedGreaterThanOrEqual => sx >= sy,
    }
}

/// An operation with one operand's bits that decide it: `x op c` (`c_right`) or `c op x`, `xb`
/// what is known of `x`. An `and` with ones wherever `x` may have one is `x`.
fn decided(op: Opcode, x: Value, xb: Bits, c: Bits, c_right: bool, w: u32) -> Option<Folded> {
    let m = mask(w);
    let cv = c.value(w);
    let (zero, ones) = (cv == Some(0), cv == Some(m));
    Some(match op {
        Opcode::Band if zero => Folded::Const(0),
        Opcode::Band if (c.ones | xb.zeros) & m == m => Folded::Same(x),
        Opcode::Bor if (c.zeros | xb.ones) & m == m => Folded::Same(x),
        Opcode::Bor if ones => Folded::Const(m),
        Opcode::Bxor | Opcode::Iadd if zero => Folded::Same(x),
        Opcode::Isub if zero && c_right => Folded::Same(x),
        Opcode::Imul if zero => Folded::Const(0),
        Opcode::Imul if cv == Some(1) => Folded::Same(x),
        Opcode::BandNot if c_right && (c.zeros | xb.zeros) & m == m => Folded::Same(x),
        Opcode::BandNot if !c_right && zero => Folded::Const(0),
        Opcode::BandNot if c_right && ones => Folded::Const(0),
        Opcode::Ishl | Opcode::Ushr | Opcode::Sshr | Opcode::Rotl | Opcode::Rotr if !c_right && zero => Folded::Const(0),
        Opcode::Ishl | Opcode::Ushr | Opcode::Sshr | Opcode::Rotl | Opcode::Rotr if c_right && cv.is_some_and(|k| k & u64::from(w - 1) == 0) => Folded::Same(x),
        _ => return None,
    })
}

/// The value a call's `k`th result is of: the callee's user name's index.
fn callee(func: &ir::Function, inst: ir::Inst) -> Option<u32> {
    match func.dfg.insts[inst].analyze_call(&func.dfg.value_lists, &func.dfg.exception_tables) {
        ir::instructions::CallInfo::Direct(fr, _) => match func.dfg.ext_funcs[fr].name {
            ir::ExternalName::User(r) => Some(func.params.user_named_funcs()[r].index),
            _ => None,
        },
        _ => None,
    }
}

/// Each block's incoming arguments: for each predecessor's branch, the values it passes.
fn incoming(func: &ir::Function) -> HashMap<ir::Block, Vec<Vec<Option<Value>>>> {
    let mut m: HashMap<ir::Block, Vec<Vec<Option<Value>>>> = HashMap::new();
    let pool = &func.dfg.value_lists;
    for b in func.layout.blocks() {
        let Some(last) = func.layout.last_inst(b) else { continue };
        for c in func.dfg.insts[last].branch_destination(&func.dfg.jump_tables, &func.dfg.exception_tables) {
            let args = c.args(pool).map(|a| a.as_value().map(|v| func.dfg.resolve_aliases(v))).collect();
            m.entry(c.block(pool)).or_default().push(args);
        }
    }
    m
}

/// What is known of each value's bits, in the layout's order: a block's parameter what every
/// branch to it passes (each already known, else nothing), a call's results its callee's
/// (`callees`).
fn known(func: &ir::Function, callees: &Summaries) -> HashMap<Value, Bits> {
    let mut bits: HashMap<Value, Bits> = HashMap::new();
    let ins = incoming(func);
    let entry = func.layout.entry_block();
    for b in func.layout.blocks() {
        if Some(b) != entry
            && let Some(edges) = ins.get(&b)
        {
            for (i, &p) in func.dfg.block_params(b).iter().enumerate() {
                let Some(w) = width(func.dfg.value_type(p)) else { continue };
                let mut acc: Option<Bits> = None;
                for e in edges {
                    let x = e.get(i).copied().flatten().and_then(|v| bits.get(&v).copied());
                    let Some(x) = x else {
                        acc = None;
                        break;
                    };
                    acc = Some(acc.map_or(x, |a| a.meet(x)));
                }
                if let Some(a) = acc.filter(|a| (a.zeros | a.ones) & mask(w) != 0) {
                    bits.insert(p, a);
                }
            }
        }
        for inst in func.layout.block_insts(b) {
            if let Some(id) = callee(func, inst)
                && let Some(rs) = callees.get(&id)
            {
                for (&r, &k) in func.dfg.inst_results(inst).iter().zip(rs) {
                    bits.insert(r, k);
                }
                continue;
            }
            let Some(&r) = func.dfg.inst_results(inst).first() else { continue };
            let Some(w) = width(func.dfg.value_type(r)) else { continue };
            let k = |v: Value| bits.get(&func.dfg.resolve_aliases(v)).copied().unwrap_or_default();
            let out = match func.dfg.insts[inst] {
                InstructionData::UnaryImm { opcode: Opcode::Iconst, imm } => Bits::exact(imm.bits() as u64, w),
                InstructionData::Unary { opcode, arg } => {
                    let x = k(arg);
                    let aw = width(func.dfg.value_type(arg)).unwrap_or(64);
                    match opcode {
                        Opcode::Bnot => Bits { zeros: x.ones, ones: x.zeros & mask(w) },
                        Opcode::Uextend => Bits { zeros: (x.zeros & mask(aw)) | (mask(w) & !mask(aw)), ones: x.ones & mask(aw) },
                        Opcode::Ireduce => Bits { zeros: x.zeros & mask(w), ones: x.ones & mask(w) },
                        // A count of bits: at most the width, 64.
                        Opcode::Ctz | Opcode::Clz | Opcode::Popcnt if x.value(aw).is_none() => Bits { zeros: mask(w) & !127, ones: 0 },
                        _ => match x.value(aw) {
                            Some(v) => match unary(opcode, v, w, aw) {
                                Some(c) => Bits::exact(c, w),
                                None => Bits::default(),
                            },
                            None => Bits::default(),
                        },
                    }
                }
                InstructionData::Binary { opcode, args: [a, c] } => {
                    let (x, y) = (k(a), k(c));
                    match (x.value(w), y.value(w)) {
                        (Some(p), Some(q)) => binary(opcode, p, q, w).map_or_else(Bits::default, |c| Bits::exact(c, w)),
                        _ => binary_bits(opcode, x, y, w),
                    }
                }
                InstructionData::BinaryImm64 { opcode, arg, imm } => {
                    let (x, y) = (k(arg), Bits::exact(imm.bits() as u64, w));
                    match imm_op(opcode) {
                        Some(Opcode::Isub) => match x.value(w) {
                            Some(p) => Bits::exact(binary(Opcode::Isub, y.ones, p, w).expect("a subtract"), w),
                            None => Bits::default(),
                        },
                        Some(op) => match x.value(w) {
                            Some(p) => binary(op, p, y.ones, w).map_or_else(Bits::default, |c| Bits::exact(c, w)),
                            None => binary_bits(op, x, y, w),
                        },
                        None => Bits::default(),
                    }
                }
                InstructionData::IntCompare { cond, args: [a, c], .. } => {
                    let aw = width(func.dfg.value_type(a)).unwrap_or(64);
                    compare_bits(cond, k(a), k(c), aw, w)
                }
                InstructionData::IntCompareImm { cond, arg, imm, .. } => {
                    let aw = width(func.dfg.value_type(arg)).unwrap_or(64);
                    compare_bits(cond, k(arg), Bits::exact(imm.bits() as u64, aw), aw, w)
                }
                InstructionData::Ternary { opcode: Opcode::Select, args: [c, x, y] } => match k(c).value(width(func.dfg.value_type(c)).unwrap_or(8)) {
                    Some(c) => k(if c != 0 { x } else { y }),
                    None => k(x).meet(k(y)),
                },
                _ => Bits::default(),
            };
            if out != Bits::default() {
                bits.insert(r, out);
            }
        }
    }
    bits
}

/// A compare's result (0 or 1, `w` bits) of what is known of its operands (`aw` bits): its
/// value where both are known, and an equality where a bit known in both differs.
fn compare_bits(cond: IntCC, x: Bits, y: Bits, aw: u32, w: u32) -> Bits {
    if let (Some(p), Some(q)) = (x.value(aw), y.value(aw)) {
        return Bits::exact(u64::from(compare(cond, p, q, aw)), w);
    }
    let differ = ((x.ones & y.zeros) | (x.zeros & y.ones)) & mask(aw) != 0;
    match cond {
        IntCC::Equal if differ => Bits::exact(0, w),
        IntCC::NotEqual if differ => Bits::exact(1, w),
        _ => Bits { zeros: mask(w) & !1, ones: 0 },
    }
}

fn unary(op: Opcode, x: u64, w: u32, aw: u32) -> Option<u64> {
    Some(match op {
        Opcode::Popcnt => u64::from((x & mask(w)).count_ones()),
        Opcode::Bnot => !x & mask(w),
        Opcode::Ineg => x.wrapping_neg() & mask(w),
        Opcode::Uextend | Opcode::Ireduce => x & mask(aw) & mask(w),
        Opcode::Sextend => signed(x, aw) as u64 & mask(w),
        Opcode::Ctz => u64::from((x | !mask(aw)).trailing_zeros().min(aw)),
        Opcode::Clz => u64::from((x & mask(aw)).leading_zeros() - (64 - aw)),
        _ => return None,
    })
}

/// The operation an immediate form is of.
fn imm_op(op: Opcode) -> Option<Opcode> {
    Some(match op {
        Opcode::BandImm => Opcode::Band,
        Opcode::BorImm => Opcode::Bor,
        Opcode::BxorImm => Opcode::Bxor,
        Opcode::IaddImm => Opcode::Iadd,
        Opcode::ImulImm => Opcode::Imul,
        Opcode::IshlImm => Opcode::Ishl,
        Opcode::UshrImm => Opcode::Ushr,
        Opcode::SshrImm => Opcode::Sshr,
        Opcode::IrsubImm => Opcode::Isub,
        _ => return None,
    })
}

/// One pass of folds over the function, in its layout's order. Whether any was made.
fn fold_once(func: &mut ir::Function, callees: &Summaries) -> bool {
    let bits = known(func, callees);
    let mut changed = false;
    let blocks: Vec<ir::Block> = func.layout.blocks().collect();
    for b in blocks {
        let insts: Vec<ir::Inst> = func.layout.block_insts(b).collect();
        for inst in insts {
            let dfg = &func.dfg;
            let kb = |v: Value| bits.get(&dfg.resolve_aliases(v)).copied().unwrap_or_default();
            let data = dfg.insts[inst];
            if let InstructionData::Brif { arg, blocks, .. } = data {
                let aw = width(dfg.value_type(arg)).unwrap_or(64);
                let x = kb(arg);
                // Not zero where a bit is known one; zero where all are known zero.
                let nonzero = if x.ones & mask(aw) != 0 { Some(true) } else { x.value(aw).map(|c| c != 0) };
                if let Some(nz) = nonzero {
                    let to = blocks[usize::from(!nz)];
                    func.dfg.insts[inst] = InstructionData::Jump { opcode: Opcode::Jump, destination: to };
                    changed = true;
                }
                continue;
            }
            let op = data.opcode();
            if op == Opcode::Iconst || op.is_call() || op.can_load() || op.can_store() || op.other_side_effects() || op.can_trap() {
                continue;
            }
            let Some(&r) = dfg.inst_results(inst).first() else { continue };
            let Some(w) = width(dfg.value_type(r)) else { continue };
            let folded = if let Some(c) = bits.get(&r).and_then(|x| x.value(w)) {
                Some(Folded::Const(c))
            } else {
                match data {
                    InstructionData::Binary { opcode, args: [a, c] } => {
                        let (x, y) = (kb(a), kb(c));
                        match (x.value(w), y.value(w)) {
                            (None, Some(_)) => decided(opcode, a, x, y, true, w),
                            (Some(_), None) => decided(opcode, c, y, x, false, w),
                            (None, None) if a == c && matches!(opcode, Opcode::Band | Opcode::Bor) => Some(Folded::Same(a)),
                            (None, None) if a == c && matches!(opcode, Opcode::Bxor | Opcode::Isub | Opcode::BandNot) => Some(Folded::Const(0)),
                            // `(c + x) - c`, `(x + c) - c`: `x`.
                            (None, None) if opcode == Opcode::Isub && added(func, a, c).is_some() => added(func, a, c).map(Folded::Same),
                            (None, None) => match opcode {
                                // An `and` with a side that has ones wherever the other may.
                                Opcode::Band if (y.ones | x.zeros) & mask(w) == mask(w) => Some(Folded::Same(a)),
                                Opcode::Band if (x.ones | y.zeros) & mask(w) == mask(w) => Some(Folded::Same(c)),
                                Opcode::Bor if (y.zeros | x.ones) & mask(w) == mask(w) => Some(Folded::Same(a)),
                                Opcode::Bor if (x.zeros | y.ones) & mask(w) == mask(w) => Some(Folded::Same(c)),
                                _ => None,
                            },
                            _ => None,
                        }
                    }
                    // `(x + j) + k`: `x + (j + k)`, `x` where it is none.
                    InstructionData::BinaryImm64 { opcode: Opcode::IaddImm, arg, imm } if kb(arg).value(w).is_none() && add_imm_of(func, arg).is_some() => {
                        let (x, j) = add_imm_of(func, arg).expect("an add");
                        let k = j.wrapping_add(imm.bits());
                        if signed(k as u64, w) == 0 { Some(Folded::Same(x)) } else { Some(Folded::AddImm(x, k)) }
                    }
                    InstructionData::BinaryImm64 { opcode, arg, imm } => match imm_op(opcode) {
                        Some(Opcode::Isub) | None => None,
                        Some(op) => decided(op, arg, kb(arg), Bits::exact(imm.bits() as u64, w), true, w),
                    },
                    InstructionData::Unary { opcode: Opcode::Uextend | Opcode::Ireduce, .. } => None,
                    // An unsigned compare its operands' ranges decide (`urange`).
                    InstructionData::IntCompare { cond, args: [a, c], .. } => {
                        let aw = width(dfg.value_type(a)).unwrap_or(64);
                        range_compare(cond, urange(func, &bits, a, aw, 4), urange(func, &bits, c, aw, 4)).map(|t| Folded::Const(u64::from(t)))
                    }
                    InstructionData::IntCompareImm { cond, arg, imm, .. } => {
                        let aw = width(dfg.value_type(arg)).unwrap_or(64);
                        let k = imm.bits() as u64 & mask(aw);
                        range_compare(cond, urange(func, &bits, arg, aw, 4), Some((k, k))).map(|t| Folded::Const(u64::from(t)))
                    }
                    // A complement's complement: the value.
                    InstructionData::Unary { opcode: Opcode::Bnot, arg } => match dfg.value_def(dfg.resolve_aliases(arg)) {
                        cranelift_codegen::ir::ValueDef::Result(d, _) => match dfg.insts[d] {
                            InstructionData::Unary { opcode: Opcode::Bnot, arg: x } => Some(Folded::Same(x)),
                            _ => None,
                        },
                        _ => None,
                    },
                    InstructionData::Ternary { opcode: Opcode::Select, args: [c, x, y] } => {
                        let cw = width(dfg.value_type(c)).unwrap_or(8);
                        match kb(c).value(cw) {
                            Some(k) => Some(Folded::Same(if k != 0 { x } else { y })),
                            None if dfg.resolve_aliases(x) == dfg.resolve_aliases(y) => Some(Folded::Same(x)),
                            None => None,
                        }
                    }
                    _ => None,
                }
            };
            let Some(folded) = folded else {
                // One small constant operand: the immediate form, as a compiler writes `x + 1`.
                if let InstructionData::Binary { opcode, args: [a, c] } = data {
                    let (x, y) = (kb(a).value(w), kb(c).value(w));
                    let (v, k) = match (x, y) {
                        (None, Some(k)) => (a, k),
                        (Some(k), None) if opcode != Opcode::Isub => (c, k),
                        _ => continue,
                    };
                    let k = signed(k, w);
                    // A small constant only, as an instruction's immediate is: a wide one (an
                    // address the backends place, `clif_rust`'s tables) stays a constant.
                    if i32::try_from(k).is_err() {
                        continue;
                    }
                    let rb = func.dfg.replace(inst);
                    match opcode {
                        Opcode::Iadd => rb.iadd_imm(v, k),
                        Opcode::Isub => rb.iadd_imm(v, k.wrapping_neg()),
                        Opcode::Band => rb.band_imm(v, k),
                        Opcode::Bor => rb.bor_imm(v, k),
                        Opcode::Bxor => rb.bxor_imm(v, k),
                        Opcode::Imul => rb.imul_imm(v, k),
                        _ => continue,
                    };
                    changed = true;
                }
                continue;
            };
            let ty = func.dfg.value_type(r);
            match folded {
                Folded::Const(c) => {
                    let c = c & mask(ty.bits());
                    func.dfg.replace(inst).iconst(ty, c as i64);
                }
                Folded::AddImm(x, k) => {
                    func.dfg.replace(inst).iadd_imm(x, k);
                }
                Folded::Same(v) => {
                    let v = func.dfg.resolve_aliases(v);
                    if v == r || func.dfg.value_type(v) != ty {
                        continue;
                    }
                    func.dfg.clear_results(inst);
                    func.dfg.change_to_alias(r, v);
                    func.layout.remove_inst(inst);
                }
            }
            changed = true;
        }
    }
    changed
}

/// `a - c`'s other operand where `a` is `c + x` or `x + c`.
fn added(func: &ir::Function, a: Value, c: Value) -> Option<Value> {
    let dfg = &func.dfg;
    let c = dfg.resolve_aliases(c);
    let ir::ValueDef::Result(d, _) = dfg.value_def(dfg.resolve_aliases(a)) else { return None };
    let InstructionData::Binary { opcode: Opcode::Iadd, args: [x, y] } = dfg.insts[d] else { return None };
    let (x, y) = (dfg.resolve_aliases(x), dfg.resolve_aliases(y));
    if x == c {
        Some(y)
    } else if y == c {
        Some(x)
    } else {
        None
    }
}

/// `v`'s operand and constant where `v` is `x + j` (`iadd_imm`).
fn add_imm_of(func: &ir::Function, v: Value) -> Option<(Value, i64)> {
    let dfg = &func.dfg;
    let ir::ValueDef::Result(d, _) = dfg.value_def(dfg.resolve_aliases(v)) else { return None };
    match dfg.insts[d] {
        InstructionData::BinaryImm64 { opcode: Opcode::IaddImm, arg, imm } => Some((dfg.resolve_aliases(arg), imm.bits())),
        _ => None,
    }
}

/// `v`'s unsigned range at `w` bits: its known bits' (`ones` up to all but `zeros`), and an
/// add of a constant's where it cannot wrap. `None` where nothing is known.
fn urange(func: &ir::Function, bits: &HashMap<Value, Bits>, v: Value, w: u32, depth: u32) -> Option<(u64, u64)> {
    let dfg = &func.dfg;
    let v = dfg.resolve_aliases(v);
    let m = mask(w);
    let b = bits.get(&v).copied().unwrap_or_default();
    let of_bits = (b.ones & m, m & !b.zeros);
    if depth == 0 {
        return Some(of_bits);
    }
    if let ir::ValueDef::Result(d, _) = dfg.value_def(v)
        && let InstructionData::BinaryImm64 { opcode: Opcode::IaddImm, arg, imm } = dfg.insts[d]
        && let Some((lo, hi)) = urange(func, bits, arg, w, depth - 1)
    {
        let k = imm.bits() as u64 & m;
        if hi.checked_add(k).is_some_and(|h| h <= m) {
            let (lo, hi) = (lo + k, hi + k);
            return Some((lo.max(of_bits.0), hi.min(of_bits.1)));
        }
    }
    Some(of_bits)
}

/// An unsigned compare of two ranges, where they decide it.
fn range_compare(cc: IntCC, x: Option<(u64, u64)>, y: Option<(u64, u64)>) -> Option<bool> {
    let ((xl, xh), (yl, yh)) = (x?, y?);
    match cc {
        IntCC::UnsignedLessThan if xh < yl => Some(true),
        IntCC::UnsignedLessThan if xl >= yh => Some(false),
        IntCC::UnsignedLessThanOrEqual if xh <= yl => Some(true),
        IntCC::UnsignedLessThanOrEqual if xl > yh => Some(false),
        IntCC::UnsignedGreaterThan if xl > yh => Some(true),
        IntCC::UnsignedGreaterThan if xh <= yl => Some(false),
        IntCC::UnsignedGreaterThanOrEqual if xl >= yh => Some(true),
        IntCC::UnsignedGreaterThanOrEqual if xh < yl => Some(false),
        _ => None,
    }
}

/// Block parameters every branch to which passes the same value, or the same constant: that
/// value, or the constant made at the block's start; the parameter and its arguments gone.
fn same_params(func: &mut ir::Function) -> bool {
    let entry = func.layout.entry_block();
    let ins = incoming(func);
    let mut changed = false;
    let blocks: Vec<ir::Block> = func.layout.blocks().collect();
    for b in blocks {
        if Some(b) == entry {
            continue;
        }
        let Some(edges) = ins.get(&b) else { continue };
        let params = func.dfg.block_params(b).to_vec();
        // Last first, so each index stays its parameter's.
        for (i, &p) in params.iter().enumerate().rev() {
            let args: Vec<Option<Value>> = edges.iter().map(|e| e.get(i).copied().flatten()).collect();
            let konst = |v: Value| match func.dfg.value_def(v) {
                ir::ValueDef::Result(d, _) => match func.dfg.insts[d] {
                    InstructionData::UnaryImm { opcode: Opcode::Iconst, imm } => Some(imm.bits()),
                    _ => None,
                },
                _ => None,
            };
            // Every branch passes a value (no exception's), and some other than the parameter.
            if args.iter().any(|a| a.is_none()) {
                continue;
            }
            let others: Vec<Value> = args.iter().flatten().copied().filter(|&v| v != p).collect();
            if others.is_empty() {
                continue;
            }
            let first = others[0];
            let same = others.iter().all(|&v| v == first);
            let constant = konst(first).filter(|&c| others.iter().all(|&v| konst(v) == Some(c)));
            if !same && constant.is_none() {
                continue;
            }
            let ty = func.dfg.value_type(p);
            // Each branch's argument `i` to `b` dropped.
            let srcs: Vec<ir::Block> = func.layout.blocks().collect();
            for s in srcs {
                let Some(last) = func.layout.last_inst(s) else { continue };
                let dfg = &mut func.dfg;
                for c in dfg.insts[last].branch_destination_mut(&mut dfg.jump_tables, &mut dfg.exception_tables) {
                    if c.block(&dfg.value_lists) == b {
                        c.remove(i, &mut dfg.value_lists);
                    }
                }
            }
            func.dfg.remove_block_param(p);
            let to = if same {
                first
            } else {
                let mut cur = FuncCursor::new(func).at_first_insertion_point(b);
                cur.ins().iconst(ty, constant.expect("a constant"))
            };
            func.dfg.change_to_alias(p, to);
            changed = true;
        }
    }
    changed
}

/// The blocks a block's last instruction may go to.
fn successors(func: &ir::Function, b: ir::Block) -> Vec<ir::Block> {
    let Some(last) = func.layout.last_inst(b) else { return Vec::new() };
    let pool = &func.dfg.value_lists;
    match &func.dfg.insts[last] {
        InstructionData::Jump { destination, .. } => vec![destination.block(pool)],
        InstructionData::Brif { blocks, .. } => blocks.iter().map(|c| c.block(pool)).collect(),
        InstructionData::BranchTable { table, .. } => {
            let jt = &func.stencil.dfg.jump_tables[*table];
            jt.all_branches().iter().map(|c| c.block(pool)).collect()
        }
        _ => Vec::new(),
    }
}

/// Each block whose only way in is its predecessor's jump, joined to that predecessor: its
/// parameters the jump's arguments. Its code runs whenever the predecessor's does, so it is
/// hot or cold as that is: a branch folded to a jump into a block marked cold (a test once
/// thought rare) leaves no cold block on the hot path.
fn merge_straight(func: &mut ir::Function) -> bool {
    let entry = func.layout.entry_block();
    let mut preds: HashMap<ir::Block, usize> = HashMap::new();
    for b in func.layout.blocks() {
        for s in successors(func, b) {
            *preds.entry(s).or_default() += 1;
        }
    }
    let mut changed = false;
    let blocks: Vec<ir::Block> = func.layout.blocks().collect();
    let mut gone: HashSet<ir::Block> = HashSet::new();
    for p in blocks {
        if gone.contains(&p) {
            continue;
        }
        loop {
            let Some(last) = func.layout.last_inst(p) else { break };
            let InstructionData::Jump { destination, .. } = func.dfg.insts[last] else { break };
            let pool = &func.dfg.value_lists;
            let b = destination.block(pool);
            if Some(b) == entry || b == p || preds.get(&b) != Some(&1) {
                break;
            }
            let Some(args) = destination.args(pool).map(|a| a.as_value()).collect::<Option<Vec<Value>>>() else { break };
            let params = func.dfg.block_params(b).to_vec();
            func.dfg.detach_block_params(b);
            for (q, a) in params.into_iter().zip(args) {
                func.dfg.change_to_alias(q, a);
            }
            func.layout.remove_inst(last);
            while let Some(i) = func.layout.first_inst(b) {
                func.layout.remove_inst(i);
                func.layout.append_inst(i, p);
            }
            func.layout.remove_block(b);
            gone.insert(b);
            changed = true;
        }
    }
    changed
}

/// The blocks no longer reached from the entry, removed. Whether any were.
fn unreached(func: &mut ir::Function) -> bool {
    let Some(entry) = func.layout.entry_block() else { return false };
    let mut seen = HashSet::from([entry]);
    let mut todo = vec![entry];
    while let Some(b) = todo.pop() {
        for s in successors(func, b) {
            if seen.insert(s) {
                todo.push(s);
            }
        }
    }
    let gone: Vec<ir::Block> = func.layout.blocks().filter(|b| !seen.contains(b)).collect();
    for &b in &gone {
        while let Some(i) = func.layout.first_inst(b) {
            func.layout.remove_inst(i);
        }
        func.layout.remove_block(b);
    }
    !gone.is_empty()
}

/// The pure operations whose results nothing reads, removed. Whether any were.
fn unread(func: &mut ir::Function) -> bool {
    let mut any = false;
    loop {
        let mut used: HashSet<Value> = HashSet::new();
        let pool = &func.dfg.value_lists;
        for b in func.layout.blocks() {
            for inst in func.layout.block_insts(b) {
                for &v in func.dfg.inst_args(inst) {
                    used.insert(func.dfg.resolve_aliases(v));
                }
                let mut targets: Vec<ir::BlockCall> = Vec::new();
                match &func.dfg.insts[inst] {
                    InstructionData::Jump { destination, .. } => targets.push(*destination),
                    InstructionData::Brif { blocks, .. } => targets.extend(blocks),
                    InstructionData::BranchTable { table, .. } => targets.extend(func.stencil.dfg.jump_tables[*table].all_branches()),
                    _ => {}
                }
                for c in targets {
                    for a in c.args(pool) {
                        if let Some(v) = a.as_value() {
                            used.insert(func.dfg.resolve_aliases(v));
                        }
                    }
                }
            }
        }
        let dead: Vec<ir::Inst> = func
            .layout
            .blocks()
            .flat_map(|b| func.layout.block_insts(b))
            .filter(|&inst| {
                let op = func.dfg.insts[inst].opcode();
                let pure = !(op.is_branch() || op.is_terminator() || op.is_call() || op.can_store() || op.can_load() || op.can_trap() || op.other_side_effects());
                let results = func.dfg.inst_results(inst);
                pure && !results.is_empty() && results.iter().all(|r| !used.contains(r))
            })
            .collect();
        if dead.is_empty() {
            return any;
        }
        for inst in dead {
            func.layout.remove_inst(inst);
        }
        any = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cranelift_codegen::ir::{AbiParam, Function, Signature, UserFuncName, types};
    use cranelift_codegen::isa::CallConv;
    use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};

    fn function(params: usize, build: impl FnOnce(&mut FunctionBuilder<'_>, &[Value])) -> Function {
        let mut sig = Signature::new(CallConv::SystemV);
        sig.params.extend((0..params).map(|_| AbiParam::new(types::I64)));
        sig.returns.push(AbiParam::new(types::I64));
        let mut func = Function::with_name_signature(UserFuncName::user(0, 0), sig);
        let mut fctx = FunctionBuilderContext::new();
        let mut f = FunctionBuilder::new(&mut func, &mut fctx);
        let entry = f.create_block();
        f.append_block_params_for_function_params(entry);
        f.switch_to_block(entry);
        let p = f.block_params(entry).to_vec();
        build(&mut f, &p);
        f.seal_all_blocks();
        f.finalize();
        func
    }

    fn ops(func: &Function) -> Vec<Opcode> {
        func.layout.blocks().flat_map(|b| func.layout.block_insts(b)).map(|i| func.dfg.insts[i].opcode()).collect()
    }

    fn verified(func: &Function) {
        let flags = cranelift_codegen::settings::Flags::new(cranelift_codegen::settings::builder());
        cranelift_codegen::verify_function(func, &flags).unwrap_or_else(|e| panic!("{e}\n{}", func.display()));
    }

    /// A word known none, counted and compared, then branched on: the count, the compare and
    /// the branch fold, the branch's other side is gone, the block it goes to is joined to the
    /// entry, and the function returns its input.
    #[test]
    fn a_count_of_none_folds_through_its_branch() {
        let mut func = function(1, |f, p| {
            let (hit, miss) = (f.create_block(), f.create_block());
            let zero = f.ins().iconst(types::I64, 0);
            let w = f.ins().band(p[0], zero);
            let n = f.ins().popcnt(w);
            let many = f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, n, 2);
            f.ins().brif(many, hit, &[], miss, &[]);
            f.switch_to_block(hit);
            let seven = f.ins().iconst(types::I64, 7);
            f.ins().return_(&[seven]);
            f.switch_to_block(miss);
            let r = f.ins().bor(p[0], w);
            f.ins().return_(&[r]);
        });
        assert!(fold(&mut func));
        verified(&func);
        assert_eq!(ops(&func), vec![Opcode::Return], "{}", func.display());
        assert_eq!(func.layout.blocks().count(), 1, "the jump's block joined to the entry");
    }

    /// Constants of each width fold as that width: an 8-bit compare's result, a 32-bit add
    /// that wraps, a signed shift and an extend of a negative byte.
    #[test]
    fn constants_fold_at_their_width() {
        let mut func = function(0, |f, _| {
            let a = f.ins().iconst(types::I32, 0xffff_fff0u32 as i64);
            let b = f.ins().iadd_imm(a, 0x20);
            let c = f.ins().uextend(types::I64, b);
            let d = f.ins().iconst(types::I8, 0x80);
            let e = f.ins().sextend(types::I64, d);
            let s = f.ins().sshr_imm(e, 4);
            let lt = f.ins().icmp_imm(IntCC::SignedLessThan, e, 0);
            let lt = f.ins().uextend(types::I64, lt);
            let x = f.ins().bxor(c, s);
            let y = f.ins().ishl_imm(lt, 40);
            let r = f.ins().bor(x, y);
            f.ins().return_(&[r]);
        });
        fold(&mut func);
        verified(&func);
        let expect = (0x10u64 ^ (-8i64 as u64)) | 1 << 40;
        let insts: Vec<ir::Inst> = func.layout.blocks().flat_map(|b| func.layout.block_insts(b)).collect();
        assert_eq!(insts.len(), 2, "{}", func.display());
        match func.dfg.insts[insts[0]] {
            InstructionData::UnaryImm { opcode: Opcode::Iconst, imm } => assert_eq!(imm.bits() as u64, expect),
            ref other => panic!("{other:?}"),
        }
    }

    /// A sum less one of its operands is the other (`(s + n) - s`), and two adds of constants
    /// are one (`(n + 1) - 1` is `n`), as a compiler writes a token's length from its end.
    #[test]
    fn a_sum_less_its_operand_is_the_other() {
        let mut func = function(2, |f, p| {
            let e = f.ins().iadd(p[0], p[1]);
            let n = f.ins().isub(e, p[0]);
            let len = f.ins().iadd_imm(n, 1);
            let back = f.ins().iadd_imm(len, -1);
            let three = f.ins().iadd_imm(len, 2);
            let r = f.ins().bxor(back, three);
            f.ins().return_(&[r]);
        });
        fold(&mut func);
        verified(&func);
        // `p1 ^ (p1 + 3)`: the add of `p0` and its subtract gone, the adds joined.
        assert_eq!(ops(&func), vec![Opcode::IaddImm, Opcode::Bxor, Opcode::Return], "{}", func.display());
    }

    /// An unsigned compare its operands' ranges decide: a count of bits (64 at most) over four
    /// plus one is at least one and below 64; its branch taken. One they do not, kept.
    #[test]
    fn a_range_decides_an_unsigned_compare() {
        let mut func = function(2, |f, p| {
            let z = f.ins().ctz(p[0]);
            let lanes = f.ins().ushr_imm(z, 2);
            let len = f.ins().iadd_imm(lanes, 1);
            let some = f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, len, 1);
            let small = f.ins().icmp_imm(IntCC::UnsignedLessThan, len, 64);
            let open = f.ins().icmp_imm(IntCC::UnsignedLessThan, len, 16);
            let a = f.ins().band(some, small);
            let b = f.ins().band(a, open);
            let b = f.ins().uextend(types::I64, b);
            let r = f.ins().iadd(b, p[1]);
            f.ins().return_(&[r]);
        });
        fold(&mut func);
        verified(&func);
        let cmps = func.layout.blocks().flat_map(|b| func.layout.block_insts(b)).filter(|&i| matches!(func.dfg.insts[i].opcode(), Opcode::Icmp | Opcode::IcmpImm)).count();
        assert_eq!(cmps, 1, "only `len < 16` is open: {}", func.display());
    }

    /// A complement's complement is the value: both `bnot`s go, where the inner one has no
    /// other reader; one complement stays one.
    #[test]
    fn a_complements_complement_is_its_value() {
        let mut func = function(2, |f, p| {
            let a = f.ins().bnot(p[0]);
            let b = f.ins().bnot(a);
            let c = f.ins().bnot(p[1]);
            let r = f.ins().band(b, c);
            f.ins().return_(&[r]);
        });
        fold(&mut func);
        verified(&func);
        let ops: Vec<Opcode> = func.layout.blocks().flat_map(|b| func.layout.block_insts(b)).map(|i| func.dfg.insts[i].opcode()).collect();
        assert_eq!(ops, vec![Opcode::Bnot, Opcode::Band, Opcode::Return], "{}", func.display());
    }

    /// One constant that decides an operation: `x & 0`, `x | 0`, `x + 0`, a `select` on a
    /// constant; the others (`x - c`, `x & c`) are kept, and nothing that is read is removed.
    #[test]
    fn a_deciding_constant_folds_and_others_stay() {
        let mut func = function(2, |f, p| {
            let zero = f.ins().iconst(types::I64, 0);
            let one = f.ins().iconst(types::I8, 1);
            let a = f.ins().bor(p[0], zero);
            let b = f.ins().iadd(a, zero);
            let c = f.ins().select(one, b, p[1]);
            let d = f.ins().band_imm(c, 0xff);
            let e = f.ins().iadd_imm(d, -3);
            let g = f.ins().band(p[1], zero);
            let r = f.ins().bor(e, g);
            f.ins().return_(&[r]);
        });
        fold(&mut func);
        verified(&func);
        assert_eq!(ops(&func), vec![Opcode::BandImm, Opcode::IaddImm, Opcode::Return], "{}", func.display());
    }

    /// Loads, stores and calls stay even where their results are not read; a loop's block
    /// that is reached stays.
    #[test]
    fn effects_and_reached_blocks_stay() {
        let mut func = function(1, |f, p| {
            let (top, out) = (f.create_block(), f.create_block());
            f.append_block_param(top, types::I64);
            let _unused = f.ins().load(types::I64, ir::MemFlags::trusted(), p[0], 0);
            f.ins().store(ir::MemFlags::trusted(), p[0], p[0], 8);
            f.ins().jump(top, &[p[0].into()]);
            f.switch_to_block(top);
            let i = f.block_params(top)[0];
            let n = f.ins().iadd_imm(i, -1);
            let more = f.ins().icmp_imm(IntCC::NotEqual, n, 0);
            f.ins().brif(more, top, &[n.into()], out, &[]);
            f.switch_to_block(out);
            f.ins().return_(&[n]);
        });
        assert!(!fold(&mut func), "{}", func.display());
        verified(&func);
        assert_eq!(ops(&func).iter().filter(|o| matches!(o, Opcode::Load | Opcode::Store)).count(), 2);
    }

    /// A join whose every branch passes the same constant, or the same value, is that; a loop's
    /// parameter that only passes itself on, after a constant, is that constant.
    #[test]
    fn a_parameter_every_branch_gives_alike_is_that_value() {
        let mut func = function(2, |f, p| {
            let (left, right, join, top, out) = (f.create_block(), f.create_block(), f.create_block(), f.create_block(), f.create_block());
            f.append_block_param(join, types::I64);
            f.append_block_param(join, types::I64);
            f.append_block_param(top, types::I64);
            f.ins().brif(p[0], left, &[], right, &[]);
            f.switch_to_block(left);
            let five = f.ins().iconst(types::I64, 5);
            f.ins().jump(join, &[five.into(), p[1].into()]);
            f.switch_to_block(right);
            let five = f.ins().iconst(types::I64, 5);
            f.ins().jump(join, &[five.into(), p[1].into()]);
            f.switch_to_block(join);
            let (k, v) = (f.block_params(join)[0], f.block_params(join)[1]);
            let zero = f.ins().iconst(types::I64, 0);
            f.ins().jump(top, &[zero.into()]);
            f.switch_to_block(top);
            let z = f.block_params(top)[0];
            let more = f.ins().icmp_imm(IntCC::NotEqual, v, 3);
            f.ins().brif(more, top, &[z.into()], out, &[]);
            f.switch_to_block(out);
            let a = f.ins().iadd(v, k);
            let r = f.ins().bor(a, z);
            f.ins().return_(&[r]);
        });
        assert!(fold(&mut func));
        verified(&func);
        let params: usize = func.layout.blocks().skip(1).map(|b| func.dfg.block_params(b).len()).sum();
        assert_eq!(params, 0, "{}", func.display());
        let ops = ops(&func);
        assert!(ops.contains(&Opcode::IaddImm) || ops.contains(&Opcode::Iadd), "{}", func.display());
        assert!(!ops.contains(&Opcode::Bor), "`| 0` gone: {}", func.display());
    }

    /// Bits known without the value: a 32-bit value widened has its high half none, so a
    /// shift that reads only that half is a constant and an `and` that keeps the low half is
    /// the value; a constant field ORed in above it is read back as that constant.
    #[test]
    fn known_bits_decide_an_and_and_a_shift() {
        let mut func = function(1, |f, p| {
            let low = f.ins().ireduce(types::I32, p[0]);
            let x = f.ins().uextend(types::I64, low);
            let seven = f.ins().iconst(types::I64, 7);
            let field = f.ins().ishl_imm(seven, 32);
            let w = f.ins().bor(x, field);
            let hi = f.ins().ushr_imm(w, 32);
            let lo = f.ins().band_imm(w, 0xffff_ffff);
            let back = f.ins().band_imm(x, 0xffff_ffff);
            let r = f.ins().iadd(hi, lo);
            let r = f.ins().iadd(r, back);
            f.ins().return_(&[r]);
        });
        fold(&mut func);
        verified(&func);
        let ops = ops(&func);
        assert!(!ops.contains(&Opcode::UshrImm), "the high half is 7: {}", func.display());
        assert_eq!(ops.iter().filter(|o| **o == Opcode::BandImm).count(), 1, "only the low half of `x | 7 << 32` kept: {}", func.display());
    }

    /// A callee's results known in every return (`returns`) are known of its call's results
    /// (`fold_all`): its constant field (a tag of none, packed as zero) is a constant in the
    /// caller, and the caller's `and` that keeps the bits it may have is gone.
    #[test]
    fn a_callees_known_results_are_its_callers() {
        let mut callee = function(1, |f, p| {
            let (a, b, join) = (f.create_block(), f.create_block(), f.create_block());
            f.append_block_param(join, types::I64);
            f.append_block_param(join, types::I64);
            f.ins().brif(p[0], a, &[], b, &[]);
            f.switch_to_block(a);
            let none = f.ins().iconst(types::I64, 0);
            f.ins().jump(join, &[p[0].into(), none.into()]);
            f.switch_to_block(b);
            let nine = f.ins().iconst(types::I64, 9);
            let none = f.ins().iconst(types::I64, 0);
            f.ins().jump(join, &[nine.into(), none.into()]);
            f.switch_to_block(join);
            let (l, tag) = (f.block_params(join)[0], f.block_params(join)[1]);
            let low = f.ins().ireduce(types::I32, l);
            let x = f.ins().uextend(types::I64, low);
            let t = f.ins().ishl_imm(tag, 32);
            let r = f.ins().bor(x, t);
            f.ins().return_(&[r]);
        });
        callee.name = ir::UserFuncName::user(0, 1);
        let mut caller = function(1, |f, p| {
            let mut sig = ir::Signature::new(CallConv::SystemV);
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
            let name = f.func.declare_imported_user_function(ir::UserExternalName::new(0, 1));
            let signature = f.import_signature(sig);
            let fr = f.import_function(ir::ExtFuncData { name: ir::ExternalName::user(name), signature, colocated: true });
            let call = f.ins().call(fr, &[p[0]]);
            let r = f.inst_results(call)[0];
            let tag = f.ins().ushr_imm(r, 32);
            let tag = f.ins().iadd_imm(tag, -1);
            let l = f.ins().band_imm(r, 0xffff_ffff);
            let s = f.ins().iadd(l, tag);
            f.ins().return_(&[s]);
        });
        let sums = fold_all(&mut [&mut callee, &mut caller]);
        verified(&callee);
        verified(&caller);
        assert_eq!((sums[&1][0].ones >> 32, sums[&1][0].zeros >> 32), (0, 0xffff_ffff), "the tag field is none: {:?}", sums[&1]);
        assert_eq!(ops(&caller), vec![Opcode::Call, Opcode::IaddImm, Opcode::Return], "{}", caller.display());
    }

    /// A test known true from its bits alone (a word ORed with a constant bit is not zero):
    /// its branch is a jump, and the block it always goes to, marked cold, is joined to the
    /// hot block before it.
    #[test]
    fn a_known_bit_decides_a_test_and_its_cold_block_joins() {
        let mut func = function(1, |f, p| {
            let (merge, plain, out) = (f.create_block(), f.create_block(), f.create_block());
            f.append_block_param(out, types::I64);
            let hits = f.ins().bor_imm(p[0], 0x1000_0000);
            f.ins().brif(hits, merge, &[], plain, &[]);
            f.switch_to_block(merge);
            f.set_cold_block(merge);
            let m = f.ins().band_imm(hits, 0xff);
            f.ins().jump(out, &[m.into()]);
            f.switch_to_block(plain);
            f.ins().jump(out, &[p[0].into()]);
            f.switch_to_block(out);
            let r = f.block_params(out)[0];
            f.ins().return_(&[r]);
        });
        assert!(fold(&mut func));
        verified(&func);
        assert_eq!(func.layout.blocks().count(), 1, "{}", func.display());
        assert!(!func.layout.is_cold(func.layout.entry_block().unwrap()));
        assert_eq!(ops(&func), vec![Opcode::BorImm, Opcode::BandImm, Opcode::Return], "{}", func.display());
    }
}
