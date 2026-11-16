//! The tape's primitive reads as CLIF functions, written once (decision 0310): the reads
//! `rerail_tape_format::f16::View` and format 15's columns make, as `metal_tape.metal` ports them.
//!
//! A read takes the tape's base (the bytes its view's offsets count from), its view
//! (`view::TapeView`) and a document-local node, and returns a u32 (`NONE` for no node). Format 16's
//! value too wide for its array is all ones and lies in its side list: the node's part, its
//! 256-node block's offsets, a search for its low byte (`f16_side`, cold). A read built over another
//! calls it by name; the JIT's pass inlines the hot ones (`inline`).
//!
//! Every load here reads the tape or its view, which no code writes while it is read (`readonly`).
//! Format 16's arrays have 8 bytes of room past their last value, so a value is one word's load.

use crate::view::{F15_STRIDE_SHIFT, F15_TAIL_FROM_START, F16_BLOCK, F16_K, F16_PART, F16_SUB, at};
use cranelift_codegen::ir::{
    self, AbiParam, Function, InstBuilder, MemFlags, Signature, UserFuncName, Value,
    condcodes::IntCC,
    types::{I32, I64},
};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};

/// No node.
pub const NONE: i64 = u32::MAX as i64;

/// A read: its name, its parameters after the tape and its view, its result, whether the JIT's pass
/// inlines it (a search, or the rare case of a read, is cold: it stays a call), and its body.
pub struct Read {
    pub name: &'static str,
    pub params: &'static [ir::Type],
    pub ret: ir::Type,
    pub hot: bool,
    body: fn(&mut B<'_, '_>) -> Value,
}

/// Every read.
pub const READS: &[Read] = &[
    Read { name: "tape_f16_side", params: &[I32, I32], ret: I64, hot: false, body: f16_side },
    Read { name: "tape_size", params: &[I32], ret: I32, hot: true, body: size },
    Read { name: "tape_number", params: &[I32], ret: I32, hot: true, body: number },
    Read { name: "tape_start", params: &[I32], ret: I32, hot: true, body: start },
    Read { name: "tape_end", params: &[I32], ret: I32, hot: true, body: end },
    Read { name: "tape_parent", params: &[I32], ret: I32, hot: false, body: parent },
    Read { name: "tape_end_of", params: &[I32], ret: I32, hot: true, body: end_of },
    Read { name: "tape_next_in", params: &[I32, I32], ret: I32, hot: true, body: next_in },
    Read { name: "tape_next", params: &[I32], ret: I32, hot: true, body: next },
    Read { name: "tape_first_child", params: &[I32], ret: I32, hot: true, body: first_child },
    Read { name: "tape_child_count", params: &[I32], ret: I32, hot: true, body: child_count },
    Read { name: "tape_child", params: &[I32, I32], ret: I32, hot: true, body: child },
];

/// The read named `name`.
pub fn read(name: &str) -> Option<&'static Read> {
    READS.iter().find(|read| read.name == name)
}

/// A read's signature: the tape's base and its view (pointers), then its own parameters.
pub fn signature(read: &Read, call_conv: CallConv) -> Signature {
    let mut signature = Signature::new(call_conv);
    signature.params.extend([AbiParam::new(I64), AbiParam::new(I64)]);
    signature.params.extend(read.params.iter().map(|&ty| AbiParam::new(ty)));
    signature.returns.push(AbiParam::new(read.ret));
    signature
}

/// Where a read's calls of other reads go: the caller's reference to the read named so.
pub trait Callee {
    fn func_ref(&mut self, name: &str, signature: &Signature, func: &mut Function) -> ir::FuncRef;
}

/// The read's body as a function named `name`, its calls of other reads through `callee`.
pub fn build(read: &Read, call_conv: CallConv, name: UserFuncName, callee: &mut dyn Callee) -> Function {
    let mut func = Function::with_name_signature(name, signature(read, call_conv));
    let mut context = FunctionBuilderContext::new();
    let mut f = FunctionBuilder::new(&mut func, &mut context);
    let entry = f.create_block();
    f.append_block_params_for_function_params(entry);
    f.switch_to_block(entry);
    let params = f.block_params(entry).to_vec();
    let mut b = B { f, tape: params[0], view: params[1], args: params[2..].to_vec(), call_conv, callee };
    let value = (read.body)(&mut b);
    b.f.ins().return_(&[value]);
    b.f.seal_all_blocks();
    b.f.finalize();
    func
}

/// A body being built: the builder, the tape's base and view, the read's own parameters.
pub struct B<'a, 'c> {
    f: FunctionBuilder<'a>,
    tape: Value,
    view: Value,
    args: Vec<Value>,
    call_conv: CallConv,
    callee: &'c mut dyn Callee,
}

fn memory() -> MemFlags {
    MemFlags::new().with_notrap().with_readonly()
}

impl B<'_, '_> {
    fn arg(&self, k: usize) -> Value {
        self.args[k]
    }
    fn c32(&mut self, value: i64) -> Value {
        self.f.ins().iconst(I32, value)
    }
    fn c64(&mut self, value: i64) -> Value {
        self.f.ins().iconst(I64, value)
    }
    /// The view's u32 at byte `offset`.
    fn view(&mut self, offset: i32) -> Value {
        self.f.ins().load(I32, memory(), self.view, offset)
    }
    /// The view's `k`-th u32 of the array at byte `offset`.
    fn view_at(&mut self, offset: i32, k: Value) -> Value {
        let k = self.f.ins().uextend(I64, k);
        let k = self.f.ins().ishl_imm(k, 2);
        let p = self.f.ins().iadd(self.view, k);
        self.f.ins().load(I32, memory(), p, offset)
    }
    /// The tape's address `base + offset` (both u32s).
    fn address(&mut self, base: Value, offset: Value) -> Value {
        let base = self.f.ins().uextend(I64, base);
        let offset = self.f.ins().uextend(I64, offset);
        let p = self.f.ins().iadd(self.tape, base);
        self.f.ins().iadd(p, offset)
    }
    fn u8(&mut self, base: Value, offset: Value) -> Value {
        let p = self.address(base, offset);
        self.f.ins().uload8(I32, memory(), p, 0)
    }
    fn u16(&mut self, base: Value, offset: Value) -> Value {
        let p = self.address(base, offset);
        self.f.ins().uload16(I32, memory(), p, 0)
    }
    fn u32(&mut self, base: Value, offset: Value) -> Value {
        let p = self.address(base, offset);
        self.f.ins().load(I32, memory(), p, 0)
    }
    /// Eight bytes, little-endian, at any alignment.
    fn word(&mut self, base: Value, offset: Value) -> Value {
        let p = self.address(base, offset);
        self.f.ins().load(I64, memory(), p, 0)
    }
    /// The low `w` bytes' mask (all ones at 8 or more).
    fn mask(&mut self, w: Value) -> Value {
        let bits = self.f.ins().imul_imm(w, 8);
        let one = self.c64(1);
        let bits = self.f.ins().uextend(I64, bits);
        let shifted = self.f.ins().ishl(one, bits);
        let low = self.f.ins().iadd_imm(shifted, -1);
        let wide = self.f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, w, 8);
        let all = self.c64(-1);
        self.f.ins().select(wide, all, low)
    }
    /// The width at nibble `k` of the view's widths.
    fn width(&mut self, k: u32) -> Value {
        let widths = self.view(at::WIDTHS);
        let shifted = self.f.ins().ushr_imm(widths, i64::from(4 * k));
        self.f.ins().band_imm(shifted, 15)
    }
    fn column(&mut self, k: i32) -> Value {
        self.view(at::COLUMNS + 4 * k)
    }
    fn format16(&mut self) -> Value {
        let format = self.view(at::FORMAT);
        self.f.ins().icmp_imm(IntCC::Equal, format, 16)
    }
    /// `then` where `cond`, else `otherwise`: a value of type `ty`.
    fn either(&mut self, ty: ir::Type, cond: Value, then: impl FnOnce(&mut Self) -> Value, otherwise: impl FnOnce(&mut Self) -> Value) -> Value {
        let (yes, no, done) = (self.f.create_block(), self.f.create_block(), self.f.create_block());
        self.f.append_block_param(done, ty);
        self.f.ins().brif(cond, yes, &[], no, &[]);
        self.f.switch_to_block(yes);
        let value = then(self);
        self.f.ins().jump(done, &[value.into()]);
        self.f.switch_to_block(no);
        let value = otherwise(self);
        self.f.ins().jump(done, &[value.into()]);
        self.f.switch_to_block(done);
        self.f.block_params(done)[0]
    }
    /// The read `name` called with `args` after the tape and view.
    fn call(&mut self, name: &str, args: &[Value]) -> Value {
        let read = read(name).expect("a read");
        let signature = signature(read, self.call_conv);
        let callee = self.callee.func_ref(name, &signature, self.f.func);
        let mut all = vec![self.tape, self.view];
        all.extend_from_slice(args);
        let call = self.f.ins().call(callee, &all);
        self.f.inst_results(call)[0]
    }
    /// Format 15's cell of row `i` of a column of width 2 or 4.
    fn cell(&mut self, column: Value, width: Value, i: Value) -> Value {
        let two = self.f.ins().icmp_imm(IntCC::Equal, width, 2);
        self.either(
            I32,
            two,
            |b| {
                let at = b.f.ins().imul_imm(i, 2);
                b.u16(column, at)
            },
            |b| {
                let at = b.f.ins().imul_imm(i, 4);
                b.u32(column, at)
            },
        )
    }
    /// Format 16's value `i` of array `a` (0 slots, 1 gaps, 2 lengths): one word's load, or (all
    /// ones) its side list's.
    fn get(&mut self, a: u32, i: Value) -> Value {
        let w = self.width(a);
        let array = self.column(1 + a as i32);
        let at = self.f.ins().imul(i, w);
        let word = self.word(array, at);
        let mask = self.mask(w);
        let value = self.f.ins().band(word, mask);
        let escaped = self.f.ins().icmp(IntCC::Equal, value, mask);
        self.either(
            I64,
            escaped,
            |b| {
                let list = b.c32(i64::from(a) + 1);
                b.call("tape_f16_side", &[list, i])
            },
            |_| value,
        )
    }
}

/// A node's value in side list `k` (0 codes' low parts, 1 slots, 2 gaps, 3 lengths), the node listed.
fn f16_side(b: &mut B<'_, '_>) -> Value {
    let (k, i) = (b.arg(0), b.arg(1));
    let side = b.view_at(at::SIDE, k);
    let parts = b.view_at(at::PARTS, k);
    let p = b.f.ins().ushr_imm(i, i64::from(F16_PART.trailing_zeros()));
    let p4 = b.f.ins().imul_imm(p, 4);
    let start = b.u32(parts, p4);
    let nodes = b.view(at::NODES);
    let blocks = b.f.ins().iadd_imm(nodes, i64::from(F16_BLOCK - 1));
    let blocks = b.f.ins().ushr_imm(blocks, i64::from(F16_BLOCK.trailing_zeros()));
    let first = b.f.ins().imul_imm(p, i64::from(F16_SUB));
    let left = b.f.ins().isub(blocks, first);
    let sub = b.c32(i64::from(F16_SUB));
    let nsub = b.f.ins().umin(sub, left);
    let j = b.f.ins().ushr_imm(i, i64::from(F16_BLOCK.trailing_zeros()));
    let j = b.f.ins().band_imm(j, i64::from(F16_SUB - 1));
    // The block's run of offsets: from the end of the one before (0 for the part's first) to its own end.
    let first_block = b.f.ins().icmp_imm(IntCC::Equal, j, 0);
    let before = b.f.ins().iadd_imm(j, -1);
    let zero = b.c32(0);
    let before = b.f.ins().select(first_block, zero, before);
    let before = b.f.ins().imul_imm(before, 2);
    let before = b.f.ins().iadd(start, before);
    let lo = b.u16(side, before);
    let lo = b.f.ins().select(first_block, zero, lo);
    let own = b.f.ins().imul_imm(j, 2);
    let own = b.f.ins().iadd(start, own);
    let hi = b.u16(side, own);
    let offsets = b.f.ins().imul_imm(nsub, 2);
    let offsets = b.f.ins().iadd(start, offsets);
    let low = b.f.ins().band_imm(i, i64::from(F16_BLOCK - 1));
    // The first offset at or past the node's low byte.
    let (head, body, done) = (b.f.create_block(), b.f.create_block(), b.f.create_block());
    b.f.append_block_param(head, I32);
    b.f.append_block_param(head, I32);
    b.f.append_block_param(done, I32);
    b.f.ins().jump(head, &[lo.into(), hi.into()]);
    b.f.switch_to_block(head);
    let (lo, hi) = (b.f.block_params(head)[0], b.f.block_params(head)[1]);
    let more = b.f.ins().icmp(IntCC::UnsignedLessThan, lo, hi);
    b.f.ins().brif(more, body, &[], done, &[lo.into()]);
    b.f.switch_to_block(body);
    let sum = b.f.ins().iadd(lo, hi);
    let middle = b.f.ins().ushr_imm(sum, 1);
    let at = b.f.ins().iadd(offsets, middle);
    let byte = b.u8(side, at);
    let below = b.f.ins().icmp(IntCC::UnsignedLessThan, byte, low);
    let next = b.f.ins().iadd_imm(middle, 1);
    let lo = b.f.ins().select(below, next, lo);
    let hi = b.f.ins().select(below, hi, middle);
    b.f.ins().jump(head, &[lo.into(), hi.into()]);
    b.f.switch_to_block(done);
    let found = b.f.block_params(done)[0];
    // The list's values follow the part's offsets.
    let shift = b.f.ins().imul_imm(k, 4);
    let shift = b.f.ins().iadd_imm(shift, 16);
    let widths = b.view(at::WIDTHS);
    let vw = b.f.ins().ushr(widths, shift);
    let vw = b.f.ins().band_imm(vw, 15);
    let last = b.f.ins().iadd_imm(nsub, -1);
    let last = b.f.ins().imul_imm(last, 2);
    let last = b.f.ins().iadd(start, last);
    let count = b.u16(side, last);
    let values = b.f.ins().iadd(offsets, count);
    let at = b.f.ins().imul(found, vw);
    let at = b.f.ins().iadd(values, at);
    let word = b.word(side, at);
    let mask = b.mask(vw);
    b.f.ins().band(word, mask)
}

/// The node's subtree's size (itself and its descendants).
fn size(b: &mut B<'_, '_>) -> Value {
    let i = b.arg(0);
    let f16 = b.format16();
    b.either(
        I32,
        f16,
        |b| {
            let v = b.get(0, i);
            let leaf = b.f.ins().band_imm(v, 1);
            let leaf = b.f.ins().icmp_imm(IntCC::NotEqual, leaf, 0);
            let inner = b.f.ins().ushr_imm(v, 1);
            let inner = b.f.ins().ireduce(I32, inner);
            let one = b.c32(1);
            b.f.ins().select(leaf, one, inner)
        },
        |b| {
            let column = b.column(1);
            let width = b.width(0);
            let cell = b.cell(column, width, i);
            let one = b.c32(1);
            b.f.ins().umax(cell, one)
        },
    )
}

/// The node's code's number in its table (format 16: its language's).
fn number(b: &mut B<'_, '_>) -> Value {
    let i = b.arg(0);
    let f16 = b.format16();
    b.either(
        I32,
        f16,
        |b| {
            let codes = b.column(0);
            let byte = b.u8(codes, i);
            let cut = b.view(at::CUT);
            let below = b.f.ins().icmp(IntCC::UnsignedLessThan, byte, cut);
            b.either(
                I32,
                below,
                |_| byte,
                |b| {
                    let per = b.view(at::PER);
                    let over = b.f.ins().isub(byte, cut);
                    let over = b.f.ins().imul(over, per);
                    let list = b.c32(0);
                    let low = b.call("tape_f16_side", &[list, i]);
                    let low = b.f.ins().ireduce(I32, low);
                    let number = b.f.ins().iadd(cut, over);
                    b.f.ins().iadd(number, low)
                },
            )
        },
        |b| {
            let codes = b.column(0);
            let at = b.f.ins().imul_imm(i, 2);
            b.u16(codes, at)
        },
    )
}

/// The node's start (a document-local byte).
fn start(b: &mut B<'_, '_>) -> Value {
    let i = b.arg(0);
    let f16 = b.format16();
    b.either(
        I32,
        f16,
        |b| {
            // Its checkpoint, then the gaps of the nodes after it, to the node's own.
            let c = b.f.ins().ushr_imm(i, i64::from(F16_K.trailing_zeros()));
            let cw = b.width(3);
            let checkpoints = b.column(4);
            let at = b.f.ins().imul(c, cw);
            let word = b.word(checkpoints, at);
            let mask = b.mask(cw);
            let from = b.f.ins().band(word, mask);
            let first = b.f.ins().imul_imm(c, i64::from(F16_K));
            let first = b.f.ins().iadd_imm(first, 1);
            let (head, body, done) = (b.f.create_block(), b.f.create_block(), b.f.create_block());
            b.f.append_block_param(head, I32);
            b.f.append_block_param(head, I64);
            b.f.append_block_param(done, I64);
            b.f.ins().jump(head, &[first.into(), from.into()]);
            b.f.switch_to_block(head);
            let (j, sum) = (b.f.block_params(head)[0], b.f.block_params(head)[1]);
            let more = b.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, j, i);
            b.f.ins().brif(more, body, &[], done, &[sum.into()]);
            b.f.switch_to_block(body);
            let gap = b.get(1, j);
            let sum = b.f.ins().iadd(sum, gap);
            let j = b.f.ins().iadd_imm(j, 1);
            b.f.ins().jump(head, &[j.into(), sum.into()]);
            b.f.switch_to_block(done);
            let sum = b.f.block_params(done)[0];
            b.f.ins().ireduce(I32, sum)
        },
        |b| {
            let base = b.column(2);
            let c = b.f.ins().ushr_imm(i, i64::from(F15_STRIDE_SHIFT));
            let c = b.f.ins().imul_imm(c, 4);
            let checkpoint = b.u32(base, c);
            let off = b.column(3);
            let width = b.width(1);
            let cell = b.cell(off, width, i);
            b.f.ins().iadd(checkpoint, cell)
        },
    )
}

/// The node's end (a document-local byte).
fn end(b: &mut B<'_, '_>) -> Value {
    let i = b.arg(0);
    let f16 = b.format16();
    b.either(
        I32,
        f16,
        |b| {
            let start = b.call("tape_start", &[i]);
            let length = b.get(2, i);
            let length = b.f.ins().ireduce(I32, length);
            b.f.ins().iadd(start, length)
        },
        |b| {
            // A tail from the node's own start, or from its last descendant's.
            let widths = b.view(at::WIDTHS);
            let own = b.f.ins().band_imm(widths, i64::from(F15_TAIL_FROM_START));
            let own = b.f.ins().icmp_imm(IntCC::NotEqual, own, 0);
            let size = b.call("tape_size", &[i]);
            let last = b.f.ins().iadd(i, size);
            let last = b.f.ins().iadd_imm(last, -1);
            let from = b.f.ins().select(own, i, last);
            let from = b.call("tape_start", &[from]);
            let tail = b.column(4);
            let width = b.width(2);
            let cell = b.cell(tail, width, i);
            b.f.ins().iadd(from, cell)
        },
    )
}

/// The node's parent, or `NONE` at the root. Format 16: the node before is the parent (an inner
/// node), or a leaf naming the previous sibling or, every K-th sibling, the parent. Format 15: the
/// nearest node before whose subtree holds the node, back over at most 64 nodes, else a descent
/// from the root over the siblings' subtrees.
fn parent(b: &mut B<'_, '_>) -> Value {
    let i = b.arg(0);
    let root = b.f.ins().icmp_imm(IntCC::Equal, i, 0);
    b.either(
        I32,
        root,
        |b| b.c32(NONE),
        |b| {
            let f16 = b.format16();
            b.either(I32, f16, |b| parent16(b, i), |b| parent15(b, i))
        },
    )
}

fn parent16(b: &mut B<'_, '_>, i: Value) -> Value {
    let (head, body, sibling, done) = (b.f.create_block(), b.f.create_block(), b.f.create_block(), b.f.create_block());
    b.f.append_block_param(head, I32);
    b.f.append_block_param(done, I32);
    b.f.ins().jump(head, &[i.into()]);
    b.f.switch_to_block(head);
    let j = b.f.block_params(head)[0];
    let none = b.c32(NONE);
    b.f.ins().brif(j, body, &[], done, &[none.into()]);
    b.f.switch_to_block(body);
    let before = b.f.ins().iadd_imm(j, -1);
    let v = b.get(0, before);
    let leaf = b.f.ins().band_imm(v, 1);
    b.f.ins().brif(leaf, sibling, &[], done, &[before.into()]);
    b.f.switch_to_block(sibling);
    let step = b.f.ins().ushr_imm(v, 2);
    let step = b.f.ins().ireduce(I32, step);
    let back = b.f.ins().isub(j, step);
    let to_parent = b.f.ins().band_imm(v, 2);
    let next = b.f.create_block();
    b.f.ins().brif(to_parent, done, &[back.into()], next, &[]);
    b.f.switch_to_block(next);
    b.f.ins().jump(head, &[back.into()]);
    b.f.switch_to_block(done);
    b.f.block_params(done)[0]
}

fn parent15(b: &mut B<'_, '_>, i: Value) -> Value {
    let done = b.f.create_block();
    b.f.append_block_param(done, I32);
    // Back over at most 64 nodes.
    let (head, body, descend) = (b.f.create_block(), b.f.create_block(), b.f.create_block());
    b.f.append_block_param(head, I32);
    b.f.append_block_param(head, I32);
    let zero = b.c32(0);
    b.f.ins().jump(head, &[i.into(), zero.into()]);
    b.f.switch_to_block(head);
    let (j, k) = (b.f.block_params(head)[0], b.f.block_params(head)[1]);
    let some = b.f.ins().icmp_imm(IntCC::NotEqual, j, 0);
    let near = b.f.ins().icmp_imm(IntCC::UnsignedLessThan, k, 64);
    let go = b.f.ins().band(some, near);
    b.f.ins().brif(go, body, &[], descend, &[]);
    b.f.switch_to_block(body);
    let before = b.f.ins().iadd_imm(j, -1);
    let size = b.call("tape_size", &[before]);
    let end = b.f.ins().iadd(before, size);
    let holds = b.f.ins().icmp(IntCC::UnsignedGreaterThan, end, i);
    let next = b.f.create_block();
    b.f.ins().brif(holds, done, &[before.into()], next, &[]);
    b.f.switch_to_block(next);
    let k = b.f.ins().iadd_imm(k, 1);
    b.f.ins().jump(head, &[before.into(), k.into()]);
    // A descent from the root: at each level, the child whose subtree holds the node.
    b.f.switch_to_block(descend);
    let (level, scan, hop, found) = (b.f.create_block(), b.f.create_block(), b.f.create_block(), b.f.create_block());
    b.f.append_block_param(level, I32);
    b.f.append_block_param(scan, I32);
    b.f.append_block_param(scan, I32);
    let zero = b.c32(0);
    b.f.ins().jump(level, &[zero.into()]);
    b.f.switch_to_block(level);
    let at = b.f.block_params(level)[0];
    let c = b.f.ins().iadd_imm(at, 1);
    b.f.ins().jump(scan, &[at.into(), c.into()]);
    b.f.switch_to_block(scan);
    let (at, c) = (b.f.block_params(scan)[0], b.f.block_params(scan)[1]);
    let size = b.call("tape_size", &[c]);
    let after = b.f.ins().iadd(c, size);
    let past = b.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, after, i);
    b.f.ins().brif(past, hop, &[], found, &[]);
    b.f.switch_to_block(hop);
    b.f.ins().jump(scan, &[at.into(), after.into()]);
    b.f.switch_to_block(found);
    let is = b.f.ins().icmp(IntCC::Equal, c, i);
    b.f.ins().brif(is, done, &[at.into()], level, &[c.into()]);
    b.f.switch_to_block(done);
    b.f.block_params(done)[0]
}

/// One past the node's subtree.
fn end_of(b: &mut B<'_, '_>) -> Value {
    let i = b.arg(0);
    let size = b.call("tape_size", &[i]);
    b.f.ins().iadd(i, size)
}

/// The node after the node's subtree while it is before `end` (its parent's subtree's end), else `NONE`.
fn next_in(b: &mut B<'_, '_>) -> Value {
    let (i, end) = (b.arg(0), b.arg(1));
    let after = b.call("tape_end_of", &[i]);
    let inside = b.f.ins().icmp(IntCC::UnsignedLessThan, after, end);
    let none = b.c32(NONE);
    b.f.ins().select(inside, after, none)
}

/// The node's next sibling, or `NONE`.
fn next(b: &mut B<'_, '_>) -> Value {
    let i = b.arg(0);
    let parent = b.call("tape_parent", &[i]);
    let root = b.f.ins().icmp_imm(IntCC::Equal, parent, NONE);
    b.either(
        I32,
        root,
        |b| b.c32(NONE),
        |b| {
            let end = b.call("tape_end_of", &[parent]);
            b.call("tape_next_in", &[i, end])
        },
    )
}

/// The node's first child, or `NONE`.
fn first_child(b: &mut B<'_, '_>) -> Value {
    let i = b.arg(0);
    let size = b.call("tape_size", &[i]);
    let inner = b.f.ins().icmp_imm(IntCC::UnsignedGreaterThan, size, 1);
    let child = b.f.ins().iadd_imm(i, 1);
    let none = b.c32(NONE);
    b.f.ins().select(inner, child, none)
}

/// The node's children, by hops over their subtrees.
fn child_count(b: &mut B<'_, '_>) -> Value {
    let i = b.arg(0);
    let end = b.call("tape_end_of", &[i]);
    let first = b.f.ins().iadd_imm(i, 1);
    let (head, body, done) = (b.f.create_block(), b.f.create_block(), b.f.create_block());
    b.f.append_block_param(head, I32);
    b.f.append_block_param(head, I32);
    b.f.append_block_param(done, I32);
    let zero = b.c32(0);
    b.f.ins().jump(head, &[first.into(), zero.into()]);
    b.f.switch_to_block(head);
    let (c, n) = (b.f.block_params(head)[0], b.f.block_params(head)[1]);
    let more = b.f.ins().icmp(IntCC::UnsignedLessThan, c, end);
    b.f.ins().brif(more, body, &[], done, &[n.into()]);
    b.f.switch_to_block(body);
    let size = b.call("tape_size", &[c]);
    let c = b.f.ins().iadd(c, size);
    let n = b.f.ins().iadd_imm(n, 1);
    b.f.ins().jump(head, &[c.into(), n.into()]);
    b.f.switch_to_block(done);
    b.f.block_params(done)[0]
}

/// The node's `k`-th child (from 0), by hops over its elder siblings' subtrees, or `NONE`.
fn child(b: &mut B<'_, '_>) -> Value {
    let (i, k) = (b.arg(0), b.arg(1));
    let end = b.call("tape_end_of", &[i]);
    let first = b.f.ins().iadd_imm(i, 1);
    let (head, body, hop, done) = (b.f.create_block(), b.f.create_block(), b.f.create_block(), b.f.create_block());
    b.f.append_block_param(head, I32);
    b.f.append_block_param(head, I32);
    b.f.append_block_param(done, I32);
    b.f.ins().jump(head, &[first.into(), k.into()]);
    b.f.switch_to_block(head);
    let (c, k) = (b.f.block_params(head)[0], b.f.block_params(head)[1]);
    let more = b.f.ins().icmp(IntCC::UnsignedLessThan, c, end);
    let none = b.c32(NONE);
    b.f.ins().brif(more, body, &[], done, &[none.into()]);
    b.f.switch_to_block(body);
    b.f.ins().brif(k, hop, &[], done, &[c.into()]);
    b.f.switch_to_block(hop);
    let size = b.call("tape_size", &[c]);
    let c = b.f.ins().iadd(c, size);
    let k = b.f.ins().iadd_imm(k, -1);
    b.f.ins().jump(head, &[c.into(), k.into()]);
    b.f.switch_to_block(done);
    b.f.block_params(done)[0]
}
