//! The JIT's reads of a task's document (decision 0310): calls of the tape's reads written once in CLIF
//! (`rerail_clif::reads`), which every function's pass inlines (`native_cpu_codegen::define_function`), on the
//! tape where it is held: no node records. A node in and out is the batch's (its document's first node,
//! `node_base`, added), as every other table of the batch numbers it; a span is in the batch's numbering of
//! source bytes (`source_base` added); a code is the document's table's `DeviceCode`.

use super::native_cpu::NativeTaskContext;
use crate::code_rule_dsl::tape::resident::DeviceCode;
use cranelift_codegen::ir::{InstBuilder, MemFlags, Value, condcodes::IntCC, types};
use cranelift_frontend::FunctionBuilder;
use rerail_clif::inline::ReadIds;
use std::mem;

const NONE: i64 = u32::MAX as i64;

/// A field of the task's context no code changes while the task runs: its document's tape, view, codes and place
/// in the batch.
fn fixed(function: &mut FunctionBuilder<'_>, task: Value, offset: usize, ty: types::Type) -> Value {
    function.ins().load(ty, MemFlags::trusted().with_readonly().with_can_move(), task, offset as i32)
}

fn node_base(function: &mut FunctionBuilder<'_>, task: Value) -> Value {
    fixed(function, task, mem::offset_of!(NativeTaskContext, node_base), types::I32)
}

/// The read `name` of the batch's node `node` (and `more`), as its document numbers it.
fn call(function: &mut FunctionBuilder<'_>, reads: &ReadIds, task: Value, name: &str, node: Value, more: &[Value]) -> Value {
    let pointer = function.func.dfg.value_type(task);
    let tape = fixed(function, task, mem::offset_of!(NativeTaskContext, tape), pointer);
    let view = fixed(function, task, mem::offset_of!(NativeTaskContext, view), pointer);
    let base = node_base(function, task);
    let local = function.ins().isub(node, base);
    let callee = reads.func_ref(name, function.func);
    let mut args = vec![tape, view, local];
    args.extend_from_slice(more);
    let call = function.ins().call(callee, &args);
    function.inst_results(call)[0]
}

/// A node its document numbers, in the batch's numbering (`NONE` stays).
fn global(function: &mut FunctionBuilder<'_>, task: Value, local: Value) -> Value {
    let base = node_base(function, task);
    let none = function.ins().icmp_imm(IntCC::Equal, local, NONE);
    let at = function.ins().iadd(local, base);
    function.ins().select(none, local, at)
}

/// The node's span, in the batch's numbering of source bytes.
pub(super) fn span(function: &mut FunctionBuilder<'_>, reads: &ReadIds, task: Value, node: Value) -> (Value, Value) {
    let start = call(function, reads, task, "tape_start", node, &[]);
    let end = call(function, reads, task, "tape_end", node, &[]);
    let base = fixed(function, task, mem::offset_of!(NativeTaskContext, source_base), types::I32);
    (function.ins().iadd(start, base), function.ins().iadd(end, base))
}

/// The address of the node's code in its document's table.
pub(super) fn code(function: &mut FunctionBuilder<'_>, reads: &ReadIds, task: Value, node: Value) -> Value {
    let pointer = function.func.dfg.value_type(task);
    let number = call(function, reads, task, "tape_number", node, &[]);
    let codes = fixed(function, task, mem::offset_of!(NativeTaskContext, codes), pointer);
    let offset = function.ins().imul_imm(number, mem::size_of::<DeviceCode>() as i64);
    let offset = function.ins().uextend(pointer, offset);
    function.ins().iadd(codes, offset)
}

fn code_field(function: &mut FunctionBuilder<'_>, code: Value, offset: usize, bytes: usize) -> Value {
    let flags = MemFlags::trusted().with_readonly();
    match bytes {
        1 => function.ins().uload8(types::I32, flags, code, offset as i32),
        _ => function.ins().uload16(types::I32, flags, code, offset as i32),
    }
}

/// The node's kind (`transducer_kind`'s: ERROR is `ERROR_SYMBOL`).
pub(super) fn kind(function: &mut FunctionBuilder<'_>, reads: &ReadIds, task: Value, node: Value) -> Value {
    let code = code(function, reads, task, node);
    code_field(function, code, mem::offset_of!(DeviceCode, kind), 2)
}

/// The node's field.
pub(super) fn field(function: &mut FunctionBuilder<'_>, reads: &ReadIds, task: Value, node: Value) -> Value {
    let code = code(function, reads, task, node);
    code_field(function, code, mem::offset_of!(DeviceCode, field), 2)
}

/// The node's flags: named, extra, missing, error at bits 0 to 3.
pub(super) fn flags(function: &mut FunctionBuilder<'_>, reads: &ReadIds, task: Value, node: Value) -> Value {
    let code = code(function, reads, task, node);
    code_field(function, code, mem::offset_of!(DeviceCode, flags), 1)
}

/// Whether the node is named (an i8 truth).
pub(super) fn named(function: &mut FunctionBuilder<'_>, reads: &ReadIds, task: Value, node: Value) -> Value {
    let flags = flags(function, reads, task, node);
    let bit = function.ins().band_imm(flags, 1);
    function.ins().icmp_imm(IntCC::NotEqual, bit, 0)
}

/// The node's supertype memberships: how many, and the address of the first (u16s, ascending).
pub(super) fn classes(function: &mut FunctionBuilder<'_>, reads: &ReadIds, task: Value, node: Value) -> (Value, Value) {
    let code = code(function, reads, task, node);
    let count = code_field(function, code, mem::offset_of!(DeviceCode, class_count), 1);
    (count, function.ins().iadd_imm(code, mem::offset_of!(DeviceCode, classes) as i64))
}

/// The node's parent, or `NONE`.
pub(super) fn parent(function: &mut FunctionBuilder<'_>, reads: &ReadIds, task: Value, node: Value) -> Value {
    let local = call(function, reads, task, "tape_parent", node, &[]);
    global(function, task, local)
}

/// One past the node's subtree: its last descendant's next node.
pub(super) fn subtree_end(function: &mut FunctionBuilder<'_>, reads: &ReadIds, task: Value, node: Value) -> Value {
    let size = call(function, reads, task, "tape_size", node, &[]);
    function.ins().iadd(node, size)
}

/// The node's first child, or `NONE`.
pub(super) fn first_child(function: &mut FunctionBuilder<'_>, reads: &ReadIds, task: Value, node: Value) -> Value {
    let local = call(function, reads, task, "tape_first_child", node, &[]);
    global(function, task, local)
}

/// The sibling after `node` in its parent's subtree, which ends at `end`, or `NONE`: a hop over its subtree.
pub(super) fn next_in(function: &mut FunctionBuilder<'_>, reads: &ReadIds, task: Value, node: Value, end: Value) -> Value {
    let after = subtree_end(function, reads, task, node);
    let inside = function.ins().icmp(IntCC::UnsignedLessThan, after, end);
    let none = function.ins().iconst(types::I32, NONE);
    function.ins().select(inside, after, none)
}

/// The node's next sibling (`named`: its next named one), or `NONE`.
pub(super) fn next(function: &mut FunctionBuilder<'_>, reads: &ReadIds, task: Value, node: Value, named: bool) -> Value {
    let parent = parent(function, reads, task, node);
    let (inside, hop, test, done) = (function.create_block(), function.create_block(), function.create_block(), function.create_block());
    function.append_block_param(hop, types::I32);
    function.append_block_param(test, types::I32);
    function.append_block_param(done, types::I32);
    let root = function.ins().icmp_imm(IntCC::Equal, parent, NONE);
    let none = function.ins().iconst(types::I32, NONE);
    function.ins().brif(root, done, &[none.into()], inside, &[]);
    // A parent's subtree's end, read once: every sibling lies before it.
    function.switch_to_block(inside);
    let end = subtree_end(function, reads, task, parent);
    function.ins().jump(hop, &[node.into()]);
    function.switch_to_block(hop);
    let at = function.block_params(hop)[0];
    let sibling = next_in(function, reads, task, at, end);
    let missing = function.ins().icmp_imm(IntCC::Equal, sibling, NONE);
    function.ins().brif(missing, done, &[sibling.into()], test, &[sibling.into()]);
    function.switch_to_block(test);
    let sibling = function.block_params(test)[0];
    if named {
        let is_named = self::named(function, reads, task, sibling);
        function.ins().brif(is_named, done, &[sibling.into()], hop, &[sibling.into()]);
    } else {
        function.ins().jump(done, &[sibling.into()]);
    }
    function.switch_to_block(done);
    function.block_params(done)[0]
}
