//! Tier 2's tape on the CPU (docs/parser-generators/37): the engine's bytes, written by the
//! generated code as it parses (decision 0140).
//!
//! Each node has a working row of 12 bytes, never part of the tape: its key (kind and field,
//! 16 bits each), its start and its end. The tape's columns are written at their place:
//! `size` and `tail` at a node's close, `off` at its push from its block's `base`, `code` at a
//! commit point. The widths (`Widths`) are constants of the compiled code.
//!
//! Here are the rare parts, each an outlined generated function, a step a node: a block's base
//! and offsets again, the commit of code ids, codes taken back from a node, and the image's
//! end (the columns moved into place, the code table, the header).

use cranelift_codegen::ir::{BlockArg, InstBuilder, MemFlags, Value, condcodes::IntCC, types};
use cranelift_frontend::FunctionBuilder;
use rerail_tape_format::{CHECKPOINT_STRIDE, CODE_EXTRA, FORMAT_VERSION, HEADER_BYTES, MAGIC, SENTINEL, TAIL_FROM_START, Widths};

use crate::native_cpu_shape::Step;

/// A working row's bytes, and where its fields are in it.
pub const ROW: i64 = 12;
pub const AT_KIND: i32 = 0;
pub const AT_FIELD: i32 = 2;
pub const AT_START: i32 = 4;
pub const AT_END: i32 = 8;

/// No code for a key yet (`id_of`).
pub const NO_CODE: i64 = 0xffff;
/// The id table's slots: a key's kind (10 bits) and field (6), then a bit for each supertype
/// class the program names (`classes`, at most `MAX_CLASSES`).
pub const fn id_slots(classes: usize) -> usize {
    1 << (16 + classes)
}
/// The most supertype classes a program's codes carry (`NODE_SUPER`'s kinds).
pub const MAX_CLASSES: usize = 4;
/// Bytes past each column's cells that a word copy may read (`finish`).
pub const SLACK: usize = 16;

/// The flags of a value that did not fit its width (`Mem::Wide`): an offset, a tail (or an end
/// before the last descendant's start, from the last descendant).
pub const WIDE_OFF: i64 = 1;
pub const WIDE_TAIL: i64 = 2;

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

/// `(rows, base, off, b, n)`: blocks `b` on, of a table of `n` nodes: each block's base (its
/// nodes' least start) and its nodes' offsets from it. Gives back the last block's base and
/// whether an offset needs 4 bytes (`WIDE_OFF`). A step a node.
pub fn blocks_from(f: &mut FunctionBuilder<'_>, p: &[Value], off_width: u8, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) {
    let (rows, base, off, b, n) = (p[0], p[1], p[2], p[3], p[4]);
    let stride = CHECKPOINT_STRIDE as i64;
    let zero = f.ins().iconst(types::I64, 0);
    let (block, go, least, lbody, put_base, offs, obody, done) =
        (f.create_block(), f.create_block(), f.create_block(), f.create_block(), f.create_block(), f.create_block(), f.create_block(), f.create_block());
    // block(k, last, wide); least(i, m); offs(i, wide); done(last, wide).
    for (x, k) in [(block, 3), (least, 2), (offs, 2), (done, 2)] {
        for _ in 0..k {
            f.append_block_param(x, types::I64);
        }
    }
    heads.extend([(block, Step::Node), (least, Step::Node), (offs, Step::Node)]);
    f.ins().jump(block, &[BlockArg::from(b), BlockArg::from(zero), BlockArg::from(zero)]);
    f.switch_to_block(block);
    let (k, last, wide) = (f.block_params(block)[0], f.block_params(block)[1], f.block_params(block)[2]);
    let from = f.ins().imul_imm(k, stride);
    let past = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, from, n);
    f.ins().brif(past, done, &[BlockArg::from(last), BlockArg::from(wide)], go, &[]);
    f.switch_to_block(go);
    let to = f.ins().iadd_imm(from, stride);
    let to = f.ins().umin(to, n);
    let first = start(f, rows, from);
    f.ins().jump(least, &[BlockArg::from(from), BlockArg::from(first)]);
    // least(i, m): the least start of the block's nodes from i on.
    f.switch_to_block(least);
    let (i, m) = (f.block_params(least)[0], f.block_params(least)[1]);
    let end = f.ins().icmp(IntCC::Equal, i, to);
    f.ins().brif(end, put_base, &[], lbody, &[]);
    f.switch_to_block(lbody);
    let s = start(f, rows, i);
    let m1 = f.ins().umin(m, s);
    let i1 = f.ins().iadd_imm(i, 1);
    f.ins().jump(least, &[BlockArg::from(i1), BlockArg::from(m1)]);
    f.switch_to_block(put_base);
    let ka = f.ins().ishl_imm(k, 2);
    let ka = f.ins().iadd(base, ka);
    f.ins().istore32(MemFlags::trusted(), m, ka, 0);
    f.ins().jump(offs, &[BlockArg::from(from), BlockArg::from(wide)]);
    // offs(i, wide): each node's offset from the base; whether one needs 4 bytes.
    f.switch_to_block(offs);
    let (i, wide) = (f.block_params(offs)[0], f.block_params(offs)[1]);
    let end = f.ins().icmp(IntCC::Equal, i, to);
    let k1 = f.ins().iadd_imm(k, 1);
    f.ins().brif(end, block, &[BlockArg::from(k1), BlockArg::from(m), BlockArg::from(wide)], obody, &[]);
    f.switch_to_block(obody);
    let s = start(f, rows, i);
    let o = f.ins().isub(s, m);
    put(f, off, i, o, off_width);
    let w = f.ins().icmp_imm(IntCC::UnsignedGreaterThan, o, 0xffff);
    let w = f.ins().uextend(types::I64, w);
    let w = f.ins().imul_imm(w, WIDE_OFF);
    let wide = f.ins().bor(wide, w);
    let i1 = f.ins().iadd_imm(i, 1);
    f.ins().jump(offs, &[BlockArg::from(i1), BlockArg::from(wide)]);
    f.switch_to_block(done);
    let (last, wide) = (f.block_params(done)[0], f.block_params(done)[1]);
    f.ins().return_(&[last, wide]);
}

/// `(rows, code, id_of, code_kf, code_first, from, to, codes, snode, skind, supers)`: nodes
/// `from..to` final, their codes written: each key's id, numbered by first use (a new id where
/// none has it). A key is a node's kind and field and, where the program names supertypes
/// (`classes`), a bit for each it is in (the supertypes' records, in node order, from the
/// first at `from` or after). Gives back the codes. A step a node.
pub fn commit(f: &mut FunctionBuilder<'_>, p: &[Value], classes: &[u16], heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) {
    let flags = MemFlags::trusted();
    let (rows, code, id_of, code_kf, code_first, from, to, codes) = (p[0], p[1], p[2], p[3], p[4], p[5], p[6], p[7]);
    let (snode, skind, supers) = (p[8], p[9], p[10]);
    let (step, body, new, put_id, done) = (f.create_block(), f.create_block(), f.create_block(), f.create_block(), f.create_block());
    // step(i, c, k): node i, the codes, the supertype record k (the first of node i or after).
    for _ in 0..3 {
        f.append_block_param(step, types::I64);
    }
    f.append_block_param(put_id, types::I64);
    f.append_block_param(put_id, types::I64);
    f.append_block_param(done, types::I64);
    heads.push((step, Step::Node));
    let k0 = if classes.is_empty() {
        f.ins().iconst(types::I64, 0)
    } else {
        // The first supertype record of a node from `from` on: back from the last.
        let (back, look, found) = (f.create_block(), f.create_block(), f.create_block());
        f.append_block_param(back, types::I64);
        f.append_block_param(found, types::I64);
        heads.push((back, Step::Node));
        f.ins().jump(back, &[BlockArg::from(supers)]);
        f.switch_to_block(back);
        let k = f.block_params(back)[0];
        f.ins().brif(k, look, &[], found, &[BlockArg::from(k)]);
        f.switch_to_block(look);
        let k1 = f.ins().iadd_imm(k, -1);
        let a = f.ins().ishl_imm(k1, 2);
        let a = f.ins().iadd(snode, a);
        let at = f.ins().uload32(flags, a, 0);
        let later = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, at, from);
        f.ins().brif(later, back, &[BlockArg::from(k1)], found, &[BlockArg::from(k)]);
        f.switch_to_block(found);
        f.block_params(found)[0]
    };
    f.ins().jump(step, &[BlockArg::from(from), BlockArg::from(codes), BlockArg::from(k0)]);
    f.switch_to_block(step);
    let (i, c, k) = (f.block_params(step)[0], f.block_params(step)[1], f.block_params(step)[2]);
    let end = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, i, to);
    f.ins().brif(end, done, &[BlockArg::from(c)], body, &[]);
    f.switch_to_block(body);
    let r = row(f, rows, i);
    let key = f.ins().uload32(flags, r, AT_KIND);
    let k_f = kf(f, key);
    let (k_f, k_next) = if classes.is_empty() {
        (k_f, k)
    } else {
        // Node i's supertype records: a bit each, the class's place in `classes`.
        let (sup, sbody, sdone) = (f.create_block(), f.create_block(), f.create_block());
        f.append_block_param(sup, types::I64);
        f.append_block_param(sup, types::I64);
        f.append_block_param(sdone, types::I64);
        f.append_block_param(sdone, types::I64);
        heads.push((sup, Step::Node));
        let zero = f.ins().iconst(types::I64, 0);
        f.ins().jump(sup, &[BlockArg::from(k), BlockArg::from(zero)]);
        f.switch_to_block(sup);
        let (k, m) = (f.block_params(sup)[0], f.block_params(sup)[1]);
        let more = f.ins().icmp(IntCC::UnsignedLessThan, k, supers);
        let ka = f.ins().ishl_imm(k, 2);
        let na = f.ins().iadd(snode, ka);
        let safe = f.ins().select(more, na, snode);
        let at = f.ins().uload32(flags, safe, 0);
        let here = f.ins().icmp(IntCC::Equal, at, i);
        let take = f.ins().band(more, here);
        f.ins().brif(take, sbody, &[], sdone, &[BlockArg::from(k), BlockArg::from(m)]);
        f.switch_to_block(sbody);
        let sa = f.ins().iadd(skind, ka);
        let class = f.ins().uload32(flags, sa, 0);
        let mut bit = f.ins().iconst(types::I64, 0);
        for (j, &cl) in classes.iter().enumerate() {
            let is = f.ins().icmp_imm(IntCC::Equal, class, i64::from(cl));
            let is = f.ins().uextend(types::I64, is);
            let b = f.ins().ishl_imm(is, 16 + j as i64);
            bit = f.ins().bor(bit, b);
        }
        let m1 = f.ins().bor(m, bit);
        let k1 = f.ins().iadd_imm(k, 1);
        f.ins().jump(sup, &[BlockArg::from(k1), BlockArg::from(m1)]);
        f.switch_to_block(sdone);
        let (k, m) = (f.block_params(sdone)[0], f.block_params(sdone)[1]);
        (f.ins().bor(k_f, m), k)
    };
    let k = k_f;
    let slot = f.ins().ishl_imm(k, 1);
    let slot = f.ins().iadd(id_of, slot);
    let id = f.ins().uload16(types::I64, flags, slot, 0);
    let none = f.ins().icmp_imm(IntCC::Equal, id, NO_CODE);
    f.ins().brif(none, new, &[], put_id, &[BlockArg::from(id), BlockArg::from(c)]);
    f.switch_to_block(new);
    f.set_cold_block(new);
    f.ins().istore16(flags, c, slot, 0);
    let a = f.ins().ishl_imm(c, 2);
    let a = f.ins().iadd(code_kf, a);
    f.ins().istore32(flags, k, a, 0);
    let a = f.ins().ishl_imm(c, 2);
    let a = f.ins().iadd(code_first, a);
    f.ins().istore32(flags, i, a, 0);
    let c1 = f.ins().iadd_imm(c, 1);
    f.ins().jump(put_id, &[BlockArg::from(c), BlockArg::from(c1)]);
    f.switch_to_block(put_id);
    let (id, c) = (f.block_params(put_id)[0], f.block_params(put_id)[1]);
    let a = f.ins().ishl_imm(i, 1);
    let a = f.ins().iadd(code, a);
    f.ins().istore16(flags, id, a, 0);
    let i1 = f.ins().iadd_imm(i, 1);
    f.ins().jump(step, &[BlockArg::from(i1), BlockArg::from(c), BlockArg::from(k_next)]);
    f.switch_to_block(done);
    let c = f.block_params(done)[0];
    f.ins().return_(&[c]);
}

/// `(id_of, code_kf, code_first, codes, node)`: the codes first used at `node` or after, taken
/// back (an edit there: they are numbered again at the next commit). Gives back the codes. A
/// step a code.
pub fn uncode(f: &mut FunctionBuilder<'_>, p: &[Value], heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) {
    let flags = MemFlags::trusted();
    let (id_of, code_kf, code_first, codes, node) = (p[0], p[1], p[2], p[3], p[4]);
    let (step, look, back, done) = (f.create_block(), f.create_block(), f.create_block(), f.create_block());
    f.append_block_param(step, types::I64);
    f.append_block_param(done, types::I64);
    heads.push((step, Step::Node));
    f.ins().jump(step, &[BlockArg::from(codes)]);
    f.switch_to_block(step);
    let c = f.block_params(step)[0];
    f.ins().brif(c, look, &[], done, &[BlockArg::from(c)]);
    f.switch_to_block(look);
    let c1 = f.ins().iadd_imm(c, -1);
    let a = f.ins().ishl_imm(c1, 2);
    let a = f.ins().iadd(code_first, a);
    let first = f.ins().uload32(flags, a, 0);
    let later = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, first, node);
    f.ins().brif(later, back, &[], done, &[BlockArg::from(c)]);
    f.switch_to_block(back);
    let a = f.ins().ishl_imm(c1, 2);
    let a = f.ins().iadd(code_kf, a);
    let k = f.ins().uload32(flags, a, 0);
    let slot = f.ins().ishl_imm(k, 1);
    let slot = f.ins().iadd(id_of, slot);
    let none = f.ins().iconst(types::I64, NO_CODE);
    f.ins().istore16(flags, none, slot, 0);
    f.ins().jump(step, &[BlockArg::from(c1)]);
    f.switch_to_block(done);
    let c = f.block_params(done)[0];
    f.ins().return_(&[c]);
}

/// `n` bytes from `src` to `dst`, 8 at a time (each column has `SLACK` bytes past its cells),
/// then 16 zero bytes from `dst + n`: the column's padding (the next column, written after,
/// takes its place from its own start). Gives back `dst + n`.
fn copy(f: &mut FunctionBuilder<'_>, dst: Value, src: Value, n: Value, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> Value {
    let flags = MemFlags::trusted();
    let (step, body, done) = (f.create_block(), f.create_block(), f.create_block());
    f.append_block_param(step, types::I64);
    heads.push((step, Step::Node));
    let zero = f.ins().iconst(types::I64, 0);
    f.ins().jump(step, &[BlockArg::from(zero)]);
    f.switch_to_block(step);
    let i = f.block_params(step)[0];
    let end = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, i, n);
    f.ins().brif(end, done, &[], body, &[]);
    f.switch_to_block(body);
    let s = f.ins().iadd(src, i);
    let x = f.ins().load(types::I64, flags, s, 0);
    let d = f.ins().iadd(dst, i);
    f.ins().store(flags, x, d, 0);
    let i8 = f.ins().iadd_imm(i, 8);
    f.ins().jump(step, &[BlockArg::from(i8)]);
    f.switch_to_block(done);
    let at = f.ins().iadd(dst, n);
    f.ins().store(flags, zero, at, 0);
    f.ins().store(flags, zero, at, 8);
    at
}

/// `at` rounded up to `COLUMN_ALIGN`, from the image's start.
fn padded(f: &mut FunctionBuilder<'_>, image: Value, at: Value) -> Value {
    let rel = f.ins().isub(at, image);
    let rel = f.ins().iadd_imm(rel, rerail_tape_format::COLUMN_ALIGN as i64 - 1);
    let rel = f.ins().band_imm(rel, -(rerail_tape_format::COLUMN_ALIGN as i64));
    f.ins().iadd(image, rel)
}

/// What the image's end writes besides the columns: the language's tag, the widths, and which
/// kinds are extras (a code's flag).
#[derive(Clone, Copy)]
pub struct Image<'a> {
    pub tag: u8,
    pub widths: Widths,
    pub extras: &'a [u16],
    /// The supertype classes the codes carry (`commit`), ascending.
    pub classes: &'a [u16],
}

/// `(image, size, base, off, tail, code_kf, id_of, n, codes, len)`: the image's end. The code
/// column is in place (from `HEADER_BYTES`); the other columns are moved after it, each padded,
/// then the code table (each code's `kf`, flags, no alias, no classes), and the header. The id table is cleared for the next file.
/// Gives back the image's length.
pub fn finish(f: &mut FunctionBuilder<'_>, p: &[Value], x: Image<'_>, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) {
    let flags = MemFlags::trusted();
    let (image, size, base, off, tail, code_kf, id_of, n, codes, len) = (p[0], p[1], p[2], p[3], p[4], p[5], p[6], p[7], p[8], p[9]);
    let w = x.widths;
    // The code column's end, padded.
    let code_end = f.ins().ishl_imm(n, 1);
    let code_end = f.ins().iadd_imm(code_end, HEADER_BYTES as i64);
    let code_end = f.ins().iadd(image, code_end);
    let zero = f.ins().iconst(types::I64, 0);
    f.ins().store(flags, zero, code_end, 0);
    f.ins().store(flags, zero, code_end, 8);
    let mut at = padded(f, image, code_end);
    for (col, width) in [(size, w.size), (base, 0), (off, w.off), (tail, w.tail)] {
        let bytes = if width == 0 {
            // A base each 16 nodes, 4 bytes each.
            let b = f.ins().iadd_imm(n, CHECKPOINT_STRIDE as i64 - 1);
            let b = f.ins().ushr_imm(b, CHECKPOINT_STRIDE.trailing_zeros() as i64);
            f.ins().ishl_imm(b, 2)
        } else {
            f.ins().imul_imm(n, i64::from(width))
        };
        let end = copy(f, at, col, bytes, heads);
        at = padded(f, image, end);
    }
    // The code table, and the id table cleared: a step a code. A code's entry is its `kf`, its
    // flags, no alias, its classes' count and the classes (`commit`'s bits, ascending).
    let (step, body, done) = (f.create_block(), f.create_block(), f.create_block());
    f.append_block_param(step, types::I64);
    f.append_block_param(step, types::I64);
    f.append_block_param(done, types::I64);
    heads.push((step, Step::Node));
    f.ins().jump(step, &[BlockArg::from(zero), BlockArg::from(at)]);
    f.switch_to_block(step);
    let (c, e) = (f.block_params(step)[0], f.block_params(step)[1]);
    let end = f.ins().icmp(IntCC::Equal, c, codes);
    f.ins().brif(end, done, &[BlockArg::from(e)], body, &[]);
    f.switch_to_block(body);
    let a = f.ins().ishl_imm(c, 2);
    let a = f.ins().iadd(code_kf, a);
    let slot_key = f.ins().uload32(flags, a, 0);
    let k = f.ins().band_imm(slot_key, 0xffff);
    let kind = f.ins().band_imm(k, 0x3ff);
    let mut extra = f.ins().iconst(types::I8, 0);
    for &e in x.extras {
        let is = f.ins().icmp_imm(IntCC::Equal, kind, i64::from(e));
        extra = f.ins().bor(extra, is);
    }
    let extra = f.ins().uextend(types::I64, extra);
    let fl = f.ins().imul_imm(extra, i64::from(CODE_EXTRA));
    f.ins().istore16(flags, k, e, 0);
    f.ins().istore8(flags, fl, e, 2);
    let s = f.ins().iconst(types::I64, i64::from(SENTINEL));
    f.ins().istore16(flags, s, e, 3);
    // The classes: each stored at the next place, which moves on where the code has it.
    let mut place = f.ins().iadd_imm(e, 6);
    let mut count = zero;
    for (j, &cl) in x.classes.iter().enumerate() {
        let has = f.ins().ushr_imm(slot_key, 16 + j as i64);
        let has = f.ins().band_imm(has, 1);
        let v = f.ins().iconst(types::I64, i64::from(cl));
        f.ins().istore16(flags, v, place, 0);
        let step2 = f.ins().ishl_imm(has, 1);
        place = f.ins().iadd(place, step2);
        count = f.ins().iadd(count, has);
    }
    f.ins().istore8(flags, count, e, 5);
    let slot = f.ins().ishl_imm(slot_key, 1);
    let slot = f.ins().iadd(id_of, slot);
    let none = f.ins().iconst(types::I64, NO_CODE);
    f.ins().istore16(flags, none, slot, 0);
    let c1 = f.ins().iadd_imm(c, 1);
    f.ins().jump(step, &[BlockArg::from(c1), BlockArg::from(place)]);
    f.switch_to_block(done);
    let table_end = f.block_params(done)[0];
    let table = f.ins().isub(table_end, at);
    let total = f.ins().isub(table_end, image);
    // The header: magic, tag, widths, version, the source's length; the lengths: nodes, codes,
    // the code table's bytes, no aliases.
    let magic = f.ins().iconst(types::I64, u64::from_le_bytes(*MAGIC) as i64);
    f.ins().store(flags, magic, image, 0);
    let head = u32::from(x.tag) | u32::from(w.size) << 8 | u32::from(w.off) << 16 | u32::from(w.tail | if w.from_start { TAIL_FROM_START } else { 0 }) << 24;
    let head = f.ins().iconst(types::I64, i64::from(head));
    f.ins().istore32(flags, head, image, 8);
    let version = f.ins().iconst(types::I64, i64::from(FORMAT_VERSION));
    f.ins().istore32(flags, version, image, 12);
    f.ins().store(flags, len, image, 16);
    for (k, v) in [n, codes, table, zero, zero].into_iter().enumerate() {
        f.ins().store(flags, v, image, (rerail_tape_format::header::NODES + 8 * k) as i32);
    }
    f.ins().return_(&[total]);
}
