//! Tier 2's tape on the CPU: format 16, written by the writer's ops as tier 2 parses (decision 0434 §6, R17;
//! decision 0631).
//!
//! Each node has a working row of 12 bytes, never part of the tape: its key (kind and field,
//! 16 bits each), its start and its end, and a size cell, written at its close. At each commit point
//! the nodes before it are passed to the writer's ops (`pass`), each outlined, a step a node.

use cranelift_codegen::ir::{BlockArg, InstBuilder, MemFlags, Value, condcodes::IntCC, types};
use cranelift_frontend::FunctionBuilder;
use rerail_tape_format::Widths;

use crate::native_cpu_shape::Step;

/// A working row's bytes, and where its fields are in it.
pub const ROW: i64 = 12;
pub const AT_KIND: i32 = 0;
pub const AT_FIELD: i32 = 2;
pub const AT_START: i32 = 4;
pub const AT_END: i32 = 8;

pub use crate::stream_analysis::{MAX_CLASSES, id_slots};
/// Bytes past each column's cells that a word may read.
pub const SLACK: usize = 16;


/// Cell `i`'s address in a column of `width`-byte cells.
pub fn cell(f: &mut FunctionBuilder<'_>, col: Value, i: Value, width: u8) -> Value {
    let off = f.ins().ishl_imm(i, if width == 2 { 1 } else { 2 });
    f.ins().iadd(col, off)
}

/// Cell `i` of a column, stored at its width.
pub fn put(f: &mut FunctionBuilder<'_>, col: Value, i: Value, x: Value, width: u8) {
    let a = cell(f, col, i, width);
    if width == 2 {
        f.ins().istore16(MemFlags::trusted(), x, a, 0);
    } else {
        f.ins().istore32(MemFlags::trusted(), x, a, 0);
    }
}

/// Cell `i` of a column, loaded.
pub fn get(f: &mut FunctionBuilder<'_>, col: Value, i: Value, width: u8) -> Value {
    let a = cell(f, col, i, width);
    if width == 2 { f.ins().uload16(types::I64, MemFlags::trusted(), a, 0) } else { f.ins().uload32(MemFlags::trusted(), a, 0) }
}

/// Row `i`'s address.
pub fn row(f: &mut FunctionBuilder<'_>, rows: Value, i: Value) -> Value {
    let off = f.ins().imul_imm(i, ROW);
    let a = f.ins().iadd(rows, off);
    f.ins().bitcast(types::I64, MemFlags::new(), a)
}

/// A row's start.
pub fn start(f: &mut FunctionBuilder<'_>, rows: Value, i: Value) -> Value {
    let r = row(f, rows, i);
    f.ins().uload32(MemFlags::trusted(), r, AT_START)
}

/// A 32-bit key (kind | field << 16) as the code table's `kf` (kind | field << 10).
pub fn kf(f: &mut FunctionBuilder<'_>, key: Value) -> Value {
    let kind = f.ins().band_imm(key, 0x3ff);
    let field = f.ins().ushr_imm(key, 16);
    let field = f.ins().band_imm(field, 0x3f);
    let field = f.ins().ishl_imm(field, i64::from(rerail_tape_format::KIND_BITS));
    f.ins().bor(kind, field)
}

/// A row's kind for an ERROR leaf, a statement recovered (decision 0650): `stream_ir::ERROR_ROW`.
pub use crate::stream_ir::ERROR_ROW;

/// Node `i`'s key as the id table's slot (`id_slots`): its `kf`, and its slot (its field's top
/// bits, `SLOT_SHIFT`) at bit 16. Its classes are its number's (`stream_analysis::numbers_by_key`, `class_bits`).
fn slot_key(f: &mut FunctionBuilder<'_>, rows: Value, i: Value) -> Value {
    let r = row(f, rows, i);
    let key = f.ins().uload32(MemFlags::trusted(), r, AT_KIND);
    let k_f = kf(f, key);
    let slot = f.ins().ushr_imm(key, 16 + crate::stream_analysis::SLOT_SHIFT);
    let slot = f.ins().ishl_imm(slot, 16);
    f.ins().bor(k_f, slot)
}

/// What the passes are compiled for: the widths (its sizes') and the supertype classes the keys carry, ascending.
#[derive(Clone, Copy)]
pub struct Image<'a> {
    pub widths: Widths,
    /// The supertype classes the keys carry, ascending, and a node's by its slot and kind (`Structure::super_classes`).
    pub classes: &'a [u16],
    pub table: &'a [(u16, u16, Vec<u16>)],
}

/// An op of the format 16 writer in tier 2's module: its id and signature.
pub type OpRef = (cranelift_module::FuncId, cranelift_codegen::ir::Signature);

/// The writer's ops tier 2 calls (decision 0434 §6, R17), defined in its module
/// (`rerail_clif::writer::define_ops`): the node words' and the passes' calls.
pub struct Ops {
    pub open: OpRef,
    pub close: OpRef,
    /// `w_leaf`: a node and its close in one, where its key is known at its word (`stream_facts::direct_keys`).
    pub leaf: OpRef,
    pub commit: OpRef,
    pub retag: OpRef,
    pub fail: OpRef,
}

impl Ops {
    pub fn of(ids: &std::collections::HashMap<&'static str, OpRef>) -> Self {
        let op = |n: &str| ids[n].clone();
        Ops { open: op("w_open"), close: op("w_close"), leaf: op("w_leaf"), commit: op("w_commit"), retag: op("w_retag"), fail: op("w_fail") }
    }

    /// The ops' names, for the shape check of the code that calls them.
    pub fn names(&self) -> Vec<cranelift_codegen::ir::UserExternalName> {
        [&self.open, &self.close, &self.leaf, &self.commit, &self.retag, &self.fail].iter().map(|o| cranelift_codegen::ir::UserExternalName::new(0, o.0.as_u32())).collect()
    }
}

/// An op called with the state's address `desc` and `args`: its result (0, or the failure it kept).
pub fn call_op(f: &mut FunctionBuilder<'_>, op: &OpRef, desc: Value, args: &[Value]) -> Value {
    let mut all = vec![desc];
    all.extend_from_slice(args);
    crate::native_cpu_stream::call_own(f, op.0, &op.1, &all)[0]
}

/// The failure `code` at node `i`, kept by the ops (`w_fail`: the first one stays).
pub fn fail_op(f: &mut FunctionBuilder<'_>, ops: &Ops, desc: Value, code: u32, i: Value) {
    let c = f.ins().iconst(types::I32, i64::from(code));
    let i = f.ins().ireduce(types::I32, i);
    call_op(f, &ops.fail, desc, &[c, i]);
}

/// Node `i`'s number in the language's table (`numbers`, by its slot key): `u32::MAX` where the
/// table has none.
pub fn number_of(f: &mut FunctionBuilder<'_>, numbers: Value, key: Value) -> Value {
    let a = f.ins().ishl_imm(key, 2);
    let a = f.ins().iadd(numbers, a);
    f.ins().load(types::I32, MemFlags::trusted(), a, 0)
}

/// The writer's state: its open nodes' count and the open part's place (`rerail_clif::writer`).
const OPEN_LEN: i32 = std::mem::offset_of!(rerail_clif::writer::State, open_len) as i32;
const OPEN_AT: i32 = (std::mem::offset_of!(rerail_clif::writer::State, open) + std::mem::offset_of!(rerail_clif::writer::Part, at)) as i32;
const BAD: i32 = std::mem::offset_of!(rerail_clif::writer::State, bad) as i32;

/// `(desc, rows, size_c, numbers, from, to, keep)`: nodes `from..to` passed
/// to the format 16 writer's ops at `desc` (decision 0434 §6, R17), in preorder: before each
/// node, and at `to`, the ops' open nodes whose subtrees end there closed at their ends (the
/// bottom one, the root, kept where `keep`); each node opened with its number (`numbers` by its
/// slot key, as `commit` keys it) and start, then committed. Nothing where the ops already
/// failed; a key with no number is the ops' failure (`fail::NUMBER`). A step a node, a step a
/// close.
pub fn pass(f: &mut FunctionBuilder<'_>, p: &[Value], size_w: u8, ops: &Ops, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) {
    use rerail_clif::writer::{OPEN, fail};
    let flags = MemFlags::trusted();
    let (desc, rows, size_c, numbers, from, to, keep) = (p[0], p[1], p[2], p[3], p[4], p[5], p[6]);
    let (go, step, pop, look, shut, after, opening, missing) = (f.create_block(), f.create_block(), f.create_block(), f.create_block(), f.create_block(), f.create_block(), f.create_block(), f.create_block());
    let (opened, committed, done, out) = (f.create_block(), f.create_block(), f.create_block(), f.create_block());
    let bad = f.ins().uload32(flags, desc, BAD);
    f.ins().brif(bad, out, &[], go, &[]);
    f.switch_to_block(go);
    // step(i): node i.
    f.append_block_param(step, types::I64);
    heads.push((step, Step::Node));
    f.ins().jump(step, &[BlockArg::from(from)]);
    f.switch_to_block(step);
    let i = f.block_params(step)[0];
    heads.push((pop, Step::Pop));
    f.ins().jump(pop, &[]);
    // The innermost open node of the ops, closed where its subtree ends at i or before.
    f.switch_to_block(pop);
    let len = f.ins().uload32(flags, desc, OPEN_LEN);
    let more = f.ins().icmp(IntCC::UnsignedGreaterThan, len, keep);
    f.ins().brif(more, look, &[], after, &[]);
    f.switch_to_block(look);
    let rel = f.ins().load(types::I64, flags, desc, OPEN_AT);
    let opens = f.ins().iadd(desc, rel);
    let top = f.ins().iadd_imm(len, -1);
    let top = f.ins().imul_imm(top, i64::from(OPEN));
    let t = f.ins().iadd(opens, top);
    let j = f.ins().uload32(flags, t, 0);
    let size = get(f, size_c, j, size_w);
    let ends = f.ins().iadd(j, size);
    let ended = f.ins().icmp(IntCC::UnsignedLessThanOrEqual, ends, i);
    f.ins().brif(ended, shut, &[], after, &[]);
    f.switch_to_block(shut);
    let r = row(f, rows, j);
    let end = f.ins().uload32(flags, r, AT_END);
    let failed = call_op(f, &ops.close, desc, &[end]);
    f.ins().brif(failed, out, &[], pop, &[]);
    f.switch_to_block(after);
    let at_to = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, i, to);
    f.ins().brif(at_to, done, &[], opening, &[]);
    f.switch_to_block(opening);
    let key = slot_key(f, rows, i);
    let number = number_of(f, numbers, key);
    let none = f.ins().icmp_imm(IntCC::Equal, number, i64::from(u32::MAX));
    f.ins().brif(none, missing, &[], opened, &[]);
    f.switch_to_block(missing);
    f.set_cold_block(missing);
    fail_op(f, ops, desc, fail::NUMBER, i);
    f.ins().jump(out, &[]);
    f.switch_to_block(opened);
    let r = row(f, rows, i);
    let start = f.ins().uload32(flags, r, AT_START);
    let failed = call_op(f, &ops.open, desc, &[number, start]);
    f.ins().brif(failed, out, &[], committed, &[]);
    // Final once opened: no word reaches a node passed. The ops' calls are `f16::write_with`'s, a commit after each
    // open, so the tape's bytes are the writer's of these nodes (16z lays its blocks as they become final).
    f.switch_to_block(committed);
    let failed = call_op(f, &ops.commit, desc, &[]);
    let i1 = f.ins().iadd_imm(i, 1);
    f.ins().brif(failed, out, &[], step, &[BlockArg::from(i1)]);
    f.switch_to_block(done);
    f.ins().jump(out, &[]);
    f.switch_to_block(out);
    f.ins().return_(&[]);
}

/// `(desc, numbers, rows, i)`: node `i`, already passed to the ops and open
/// there (the root), retagged as its row now is (`w_retag`), its number by its key as `pass`
/// takes it. A key with no number is the ops' failure (`fail::NUMBER`).
pub fn retag(f: &mut FunctionBuilder<'_>, p: &[Value], ops: &Ops, _heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) {
    use rerail_clif::writer::fail;
    let (desc, numbers, rows, i) = (p[0], p[1], p[2], p[3]);
    let (missing, known) = (f.create_block(), f.create_block());
    let key = slot_key(f, rows, i);
    let number = number_of(f, numbers, key);
    let none = f.ins().icmp_imm(IntCC::Equal, number, i64::from(u32::MAX));
    f.ins().brif(none, missing, &[], known, &[]);
    f.switch_to_block(missing);
    f.set_cold_block(missing);
    fail_op(f, ops, desc, fail::NUMBER, i);
    f.ins().return_(&[]);
    f.switch_to_block(known);
    let node = f.ins().ireduce(types::I32, i);
    call_op(f, &ops.retag, desc, &[node, number]);
    f.ins().return_(&[]);
}
