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

use cranelift_codegen::ir::{self, InstBuilder, InstructionData, Opcode, Value, condcodes::IntCC};
use std::collections::{HashMap, HashSet};

/// The function with its constants folded, its branches on constants taken, and what is no
/// longer reached or read removed. Whether anything changed.
pub fn fold(func: &mut ir::Function) -> bool {
    let mut changed = false;
    loop {
        let mut again = fold_once(func);
        again |= unreached(func);
        again |= unread(func);
        if !again {
            return changed;
        }
        changed = true;
    }
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

/// An operation with one constant operand that decides it: `x op c` (`c_right`) or `c op x`.
fn decided(op: Opcode, x: Value, c: u64, c_right: bool, w: u32) -> Option<Folded> {
    let (zero, ones) = (c & mask(w) == 0, c & mask(w) == mask(w));
    Some(match op {
        Opcode::Band if zero => Folded::Const(0),
        Opcode::Band if ones => Folded::Same(x),
        Opcode::Bor if zero => Folded::Same(x),
        Opcode::Bor if ones => Folded::Const(mask(w)),
        Opcode::Bxor | Opcode::Iadd if zero => Folded::Same(x),
        Opcode::Isub if zero && c_right => Folded::Same(x),
        Opcode::Imul if zero => Folded::Const(0),
        Opcode::BandNot if c_right && zero => Folded::Same(x),
        Opcode::BandNot if !c_right && zero => Folded::Const(0),
        Opcode::BandNot if c_right && ones => Folded::Const(0),
        Opcode::Ishl | Opcode::Ushr | Opcode::Sshr | Opcode::Rotl | Opcode::Rotr if !c_right && zero => Folded::Const(0),
        Opcode::Ishl | Opcode::Ushr | Opcode::Sshr | Opcode::Rotl | Opcode::Rotr if c_right && c & u64::from(w - 1) == 0 => Folded::Same(x),
        _ => return None,
    })
}

/// One pass of folds over the function, in its layout's order. Whether any was made.
fn fold_once(func: &mut ir::Function) -> bool {
    let mut consts: HashMap<Value, u64> = HashMap::new();
    let mut changed = false;
    let blocks: Vec<ir::Block> = func.layout.blocks().collect();
    for b in blocks {
        let insts: Vec<ir::Inst> = func.layout.block_insts(b).collect();
        for inst in insts {
            let dfg = &func.dfg;
            let k = |v: Value| consts.get(&dfg.resolve_aliases(v)).copied();
            let result = dfg.inst_results(inst).first().copied();
            let w = result.and_then(|r| width(dfg.value_type(r)));
            let folded = match (dfg.insts[inst], w) {
                (InstructionData::UnaryImm { opcode: Opcode::Iconst, imm }, Some(w)) => {
                    consts.insert(result.expect("a result"), imm.bits() as u64 & mask(w));
                    continue;
                }
                (InstructionData::Unary { opcode, arg }, Some(w)) => {
                    let aw = width(dfg.value_type(arg));
                    match (k(arg), aw) {
                        (Some(x), Some(aw)) => match opcode {
                            Opcode::Popcnt => Some(Folded::Const(u64::from((x & mask(w)).count_ones()))),
                            Opcode::Bnot => Some(Folded::Const(!x & mask(w))),
                            Opcode::Ineg => Some(Folded::Const(x.wrapping_neg() & mask(w))),
                            Opcode::Uextend | Opcode::Ireduce => Some(Folded::Const(x & mask(aw) & mask(w))),
                            Opcode::Sextend => Some(Folded::Const(signed(x, aw) as u64 & mask(w))),
                            Opcode::Ctz => Some(Folded::Const(u64::from((x | !mask(aw)).trailing_zeros().min(aw)))),
                            Opcode::Clz => Some(Folded::Const(u64::from((x & mask(aw)).leading_zeros() - (64 - aw)))),
                            _ => None,
                        },
                        _ => None,
                    }
                }
                (InstructionData::Binary { opcode, args: [a, b] }, Some(w)) => match (k(a), k(b)) {
                    (Some(x), Some(y)) => binary(opcode, x, y, w).map(Folded::Const),
                    (None, Some(y)) => decided(opcode, a, y, true, w),
                    (Some(x), None) => decided(opcode, b, x, false, w),
                    (None, None) if a == b && matches!(opcode, Opcode::Band | Opcode::Bor) => Some(Folded::Same(a)),
                    (None, None) if a == b && matches!(opcode, Opcode::Bxor | Opcode::Isub | Opcode::BandNot) => Some(Folded::Const(0)),
                    _ => None,
                },
                (InstructionData::BinaryImm64 { opcode, arg, imm }, Some(w)) => {
                    let y = imm.bits() as u64;
                    let op = match opcode {
                        Opcode::BandImm => Opcode::Band,
                        Opcode::BorImm => Opcode::Bor,
                        Opcode::BxorImm => Opcode::Bxor,
                        Opcode::IaddImm => Opcode::Iadd,
                        Opcode::ImulImm => Opcode::Imul,
                        Opcode::IshlImm => Opcode::Ishl,
                        Opcode::UshrImm => Opcode::Ushr,
                        Opcode::SshrImm => Opcode::Sshr,
                        Opcode::IrsubImm => Opcode::Isub,
                        _ => Opcode::Nop,
                    };
                    match (k(arg), op) {
                        (_, Opcode::Nop) => None,
                        (Some(x), Opcode::Isub) => binary(op, y, x, w).map(Folded::Const),
                        (Some(x), op) => binary(op, x, y, w).map(Folded::Const),
                        (None, Opcode::Isub) => None,
                        (None, op) => decided(op, arg, y, true, w),
                    }
                }
                (InstructionData::IntCompare { cond, args: [a, b], .. }, Some(_)) => {
                    let aw = width(dfg.value_type(a));
                    match (k(a), k(b), aw) {
                        (Some(x), Some(y), Some(aw)) => Some(Folded::Const(u64::from(compare(cond, x, y, aw)))),
                        _ => None,
                    }
                }
                (InstructionData::IntCompareImm { cond, arg, imm, .. }, Some(_)) => {
                    let aw = width(dfg.value_type(arg));
                    match (k(arg), aw) {
                        (Some(x), Some(aw)) => Some(Folded::Const(u64::from(compare(cond, x, imm.bits() as u64, aw)))),
                        _ => None,
                    }
                }
                (InstructionData::Ternary { opcode: Opcode::Select, args: [c, x, y] }, _) => match k(c) {
                    Some(c) => Some(Folded::Same(if c != 0 { x } else { y })),
                    None if dfg.resolve_aliases(x) == dfg.resolve_aliases(y) => Some(Folded::Same(x)),
                    None => None,
                },
                (InstructionData::Brif { arg, blocks, .. }, _) => {
                    if let Some(c) = k(arg) {
                        let to = blocks[usize::from(c == 0)];
                        func.dfg.insts[inst] = InstructionData::Jump { opcode: Opcode::Jump, destination: to };
                        changed = true;
                    }
                    continue;
                }
                _ => None,
            };
            let Some(folded) = folded else { continue };
            let r = result.expect("a folded operation's result");
            let ty = func.dfg.value_type(r);
            match folded {
                Folded::Const(c) => {
                    let c = c & mask(ty.bits());
                    func.dfg.replace(inst).iconst(ty, c as i64);
                    consts.insert(r, c);
                }
                Folded::Same(v) => {
                    let v = func.dfg.resolve_aliases(v);
                    if v == r {
                        continue;
                    }
                    func.dfg.clear_results(inst);
                    func.dfg.change_to_alias(r, v);
                    func.layout.remove_inst(inst);
                    if let Some(&c) = consts.get(&v) {
                        consts.insert(r, c);
                    }
                }
            }
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
    /// the branch fold, the branch's other side is gone, and the function returns its input.
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
        assert_eq!(ops(&func), vec![Opcode::Jump, Opcode::Return], "{}", func.display());
        assert_eq!(func.layout.blocks().count(), 2);
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
}
