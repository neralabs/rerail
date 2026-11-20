//! A relation's reads as CLIF functions, written once (decision 0325): what `rerail_blocks`' `Rows` and `Relation`
//! read, for the host (the Rust backend: the blocks crate's `generated_reads.rs`), the kernels (the MSL backend:
//! `<ukl/reads/relation.h>`) and the JIT (inlined where hot).
//!
//! A read takes a descriptor's address (a `Rows`, or a `Relation`) and reaches everything else through the block's
//! self-relative references: a span's items lie `off` bytes from the span's own address, a column's chunk `chunk[k]`
//! bytes from its directory's. Every load is a word of the block, aligned to its size. The layout is the blocks
//! crate's own (`offset_of!`), so a change to it changes these reads, and their generated forms with them.
//!
//! Rows come in doubling groups of four chunks (`rows::chunk_of`): row `i`'s group is `k = 63 - clz((i >> min_log)
//! + 1)`, its place in the group `l = i - ((2^k - 1) << min_log)`, its chunk `4k + (l >> qlog)` and its place in
//! that `l & (2^qlog - 1)`, with `qlog = min_log + k - 2`.

use crate::reads::{B, fields};
use cranelift_codegen::ir::{
    InstBuilder, Value,
    condcodes::IntCC,
    types::{I32, I64},
};
use rerail_blocks::refs::Span;
use rerail_blocks::relation::{Index, Relation};
use rerail_blocks::rows::{ColDir, Rows};
use std::mem::{offset_of, size_of};

/// A span's offset from its own address, and its length (`refs::Span`).
const SPAN_OFF: i32 = offset_of!(Span<u64>, off) as i32;
const SPAN_LEN: i32 = offset_of!(Span<u64>, len) as i32;
const ROWS_MIN_LOG: i32 = offset_of!(Rows, min_log) as i32;
const ROWS_COLS: i32 = offset_of!(Rows, cols) as i32;
const ROWS_RESERVED: i32 = offset_of!(Rows, reserved) as i32;
const ROWS_NEEDED: i32 = offset_of!(Rows, needed) as i32;
const ROWS_NCOLS: i32 = offset_of!(Rows, ncols) as i32;
const COL_WIDTH: i32 = offset_of!(ColDir, width) as i32;
const COL_CHUNK: i32 = offset_of!(ColDir, chunk) as i32;
const COL_SIZE: i64 = size_of::<ColDir>() as i64;
const REL_BASE_ROWS: i32 = offset_of!(Relation, base_rows) as i32;
const REL_BASE_OFF: i32 = offset_of!(Relation, base_off) as i32;
const REL_ROWS: i64 = offset_of!(Relation, rows) as i64;
const REL_DEAD: i32 = offset_of!(Relation, dead) as i32;
const REL_INDEXES: i32 = offset_of!(Relation, indexes) as i32;
const INDEX_MASK: i32 = offset_of!(Index, mask) as i32;
const INDEX_OFFS: i32 = offset_of!(Index, offs) as i32;
const INDEX_ROWS: i32 = offset_of!(Index, rows) as i32;
const INDEX_SIZE: i64 = size_of::<Index>() as i64;

impl B<'_, '_> {
    /// The block's word of type `ty` at `p + offset` (aligned).
    pub(crate) fn word_at(&mut self, ty: cranelift_codegen::ir::Type, p: Value, offset: i32) -> Value {
        self.f.ins().load(ty, fields(), p, offset)
    }
    /// The address a span at `p + at` refers to: its own address plus its offset.
    pub(crate) fn follow(&mut self, p: Value, at: i32) -> Value {
        let off = self.word_at(I64, p, at + SPAN_OFF);
        let span = self.f.ins().iadd_imm(p, i64::from(at));
        self.f.ins().iadd(span, off)
    }
    /// `p + index * size` (`index` a u32).
    pub(crate) fn item(&mut self, p: Value, index: Value, size: i64) -> Value {
        let index = self.f.ins().uextend(I64, index);
        let at = self.f.ins().imul_imm(index, size);
        self.f.ins().iadd(p, at)
    }
    /// The read `name` of the descriptor at `desc` (in the same block) with `args`.
    pub(crate) fn call_at(&mut self, name: &str, desc: Value, args: &[Value]) -> Value {
        let tape = std::mem::replace(&mut self.tape, desc);
        let value = self.call(name, args);
        self.tape = tape;
        value
    }
    /// Row `i`'s chunk and its place in it, as u64s, for chunks whose first holds `2^min_log` rows (a u32).
    fn chunk_of(&mut self, i: Value, min_log: Value) -> (Value, Value) {
        let min_log = self.f.ins().uextend(I64, min_log);
        let i = self.f.ins().uextend(I64, i);
        let shifted = self.f.ins().ushr(i, min_log);
        let q = self.f.ins().iadd_imm(shifted, 1);
        let lz = self.f.ins().clz(q);
        let k = self.f.ins().irsub_imm(lz, 63);
        let one = self.c64(1);
        let group = self.f.ins().ishl(one, k);
        let before = self.f.ins().iadd_imm(group, -1);
        let first = self.f.ins().ishl(before, min_log);
        let l = self.f.ins().isub(i, first);
        let qlog = self.f.ins().iadd(min_log, k);
        let qlog = self.f.ins().iadd_imm(qlog, -2);
        let quarter = self.f.ins().ushr(l, qlog);
        let four_k = self.f.ins().ishl_imm(k, 2);
        let chunk = self.f.ins().iadd(four_k, quarter);
        let size = self.f.ins().ishl(one, qlog);
        let low = self.f.ins().iadd_imm(size, -1);
        let place = self.f.ins().band(l, low);
        (chunk, place)
    }
}

/// `rows_col(rows, c)`: the address of column `c`'s directory.
pub fn rows_col(b: &mut B<'_, '_>) -> Value {
    let cols = b.follow(b.tape, ROWS_COLS);
    let c = b.arg(0);
    b.item(cols, c, COL_SIZE)
}

/// `rows_width(rows, c)`: column `c`'s width in bytes (4 or 8).
pub fn rows_width(b: &mut B<'_, '_>) -> Value {
    let c = b.arg(0);
    let dir = b.call("rows_col", &[c]);
    b.word_at(I32, dir, COL_WIDTH)
}

/// `col_at(dir, min_log, i)`: the address of row `i`'s value in the column whose directory is `dir`, its rows'
/// first chunk `2^min_log` rows (a writer stores there, at the column's width).
pub fn col_at(b: &mut B<'_, '_>) -> Value {
    let (min_log, i) = (b.arg(0), b.arg(1));
    let dir = b.tape;
    let width = b.word_at(I32, dir, COL_WIDTH);
    let (chunk, place) = b.chunk_of(i, min_log);
    let slot = b.f.ins().ishl_imm(chunk, 3);
    let slot = b.f.ins().iadd(dir, slot);
    let off = b.word_at(I64, slot, COL_CHUNK);
    let start = b.f.ins().iadd(dir, off);
    let width = b.f.ins().uextend(I64, width);
    let at = b.f.ins().imul(place, width);
    b.f.ins().iadd(start, at)
}

/// `col_get(dir, min_log, i)`: row `i`'s value in the column whose directory is `dir` (a 4-byte column's
/// unsigned): a reader of many rows of one column resolves its directory once (`rows::ColRead`).
pub fn col_get(b: &mut B<'_, '_>) -> Value {
    let (min_log, i) = (b.arg(0), b.arg(1));
    let width = b.word_at(I32, b.tape, COL_WIDTH);
    let p = b.call("col_at", &[min_log, i]);
    let four = b.f.ins().icmp_imm(IntCC::Equal, width, 4);
    b.either(
        I64,
        four,
        |b| {
            let v = b.word_at(I32, p, 0);
            b.f.ins().uextend(I64, v)
        },
        |b| b.word_at(I64, p, 0),
    )
}

/// `rows_at(rows, i, c)`: the address of row `i`'s value in column `c`.
pub fn rows_at(b: &mut B<'_, '_>) -> Value {
    let (i, c) = (b.arg(0), b.arg(1));
    let min_log = b.word_at(I32, b.tape, ROWS_MIN_LOG);
    let dir = b.call("rows_col", &[c]);
    b.call_at("col_at", dir, &[min_log, i])
}

/// `rows_put(rows, i, c, v)`: `v` written as row `i`'s value in column `c`, at the column's width; 1. Where the row
/// is not below the rows the chunks hold (`reserved`), or the column not below the columns, nothing is written and
/// the rows' `needed` is counted (the overflow protocol); 0. Decision 0463: a device writer's row and column come
/// from its sizes and its kernels, which the host's sizing bounds; this bounds them whatever the sizes held. A row
/// past the chunks made would find a chunk's offset 0, its address the column's directory, and overwrite it and the
/// descriptors after it (Q265).
pub fn rows_put(b: &mut B<'_, '_>) -> Value {
    let (i, c, v) = (b.arg(0), b.arg(1), b.arg(2));
    let reserved = b.word_at(I32, b.tape, ROWS_RESERVED);
    let ncols = b.word_at(I32, b.tape, ROWS_NCOLS);
    let in_rows = b.f.ins().icmp(IntCC::UnsignedLessThan, i, reserved);
    let in_cols = b.f.ins().icmp(IntCC::UnsignedLessThan, c, ncols);
    let ok = b.f.ins().band(in_rows, in_cols);
    b.either(
        I32,
        ok,
        |b| {
            let p = b.call("rows_at", &[i, c]);
            let width = b.call("rows_width", &[c]);
            let four = b.f.ins().icmp_imm(IntCC::Equal, width, 4);
            b.either(
                I32,
                four,
                |b| {
                    let low = b.f.ins().ireduce(I32, v);
                    b.f.ins().store(written(), low, p, 0);
                    b.c32(1)
                },
                |b| {
                    b.f.ins().store(written(), v, p, 0);
                    b.c32(1)
                },
            )
        },
        |b| {
            let needed = b.f.ins().iadd_imm(b.tape, i64::from(ROWS_NEEDED));
            let one = b.c32(1);
            b.f.ins().atomic_rmw(I32, cranelift_codegen::ir::MemFlags::trusted(), cranelift_codegen::ir::AtomicRmwOp::Add, needed, one);
            b.c32(0)
        },
    )
}

/// A store of a row's value: aligned, as the loads of the block's words are, and not read-only.
fn written() -> cranelift_codegen::ir::MemFlags {
    cranelift_codegen::ir::MemFlags::new().with_notrap().with_aligned()
}

/// `rows_get(rows, i, c)`: row `i`'s value in column `c` (a 4-byte column's unsigned).
pub fn rows_get(b: &mut B<'_, '_>) -> Value {
    let (i, c) = (b.arg(0), b.arg(1));
    let min_log = b.word_at(I32, b.tape, ROWS_MIN_LOG);
    let dir = b.call("rows_col", &[c]);
    b.call_at("col_get", dir, &[min_log, i])
}

/// `rel_get(rel, id, c)`: row `id` in column `c`: the relation's own row (counted from its `base_rows`), or, for a
/// continuation (decision 0384 step 7: `base_off` set), an id below `base_rows` its base's row, the base's descriptor
/// `base_off` bytes from its own in the same block. A base may continue another (a fork of a fork, step 8): the read
/// walks the chain until the relation whose own rows hold `id` (a chain's length is bounded: `rows::MAX_SEGMENTS`).
pub fn rel_get(b: &mut B<'_, '_>) -> Value {
    let (id, c) = (b.arg(0), b.arg(1));
    let off = b.word_at(I64, b.tape, REL_BASE_OFF);
    let continues = b.f.ins().icmp_imm(IntCC::NotEqual, off, 0);
    // A relation that continues none (nearly every one): its own row, one load and one branch more than before.
    b.either(
        I64,
        continues,
        |b| {
            let tape = b.tape;
            let (head, step, done) = (b.f.create_block(), b.f.create_block(), b.f.create_block());
            b.f.append_block_param(head, I64);
            b.f.append_block_param(done, I64);
            b.f.append_block_param(done, I32);
            b.f.ins().jump(head, &[tape.into()]);
            b.f.switch_to_block(head);
            let rel = b.f.block_params(head)[0];
            let base_rows = b.word_at(I32, rel, REL_BASE_ROWS);
            let off = b.word_at(I64, rel, REL_BASE_OFF);
            let below = b.f.ins().icmp(IntCC::UnsignedLessThan, id, base_rows);
            let more = b.f.ins().icmp_imm(IntCC::NotEqual, off, 0);
            let in_base = b.f.ins().band(below, more);
            b.f.ins().brif(in_base, step, &[], done, &[rel.into(), base_rows.into()]);
            b.f.switch_to_block(step);
            let base = b.f.ins().iadd(rel, off);
            b.f.ins().jump(head, &[base.into()]);
            b.f.switch_to_block(done);
            let (rel, base_rows) = (b.f.block_params(done)[0], b.f.block_params(done)[1]);
            let i = b.f.ins().isub(id, base_rows);
            let rows = b.f.ins().iadd_imm(rel, REL_ROWS);
            b.call_at("rows_get", rows, &[i, c])
        },
        |b| {
            let base_rows = b.word_at(I32, b.tape, REL_BASE_ROWS);
            let i = b.f.ins().isub(id, base_rows);
            let rows = b.f.ins().iadd_imm(b.tape, REL_ROWS);
            b.call_at("rows_get", rows, &[i, c])
        },
    )
}

/// `rel_dead(rel, id)`: 1 if the relation's dead bits mark row `id` (a delta's kills of its base, or of its own).
pub fn rel_dead(b: &mut B<'_, '_>) -> Value {
    let id = b.arg(0);
    let words = b.word_at(I64, b.tape, REL_DEAD + SPAN_LEN);
    let w = b.f.ins().ushr_imm(id, 6);
    let w = b.f.ins().uextend(I64, w);
    let inside = b.f.ins().icmp(IntCC::UnsignedLessThan, w, words);
    b.either(
        I32,
        inside,
        |b| {
            let dead = b.follow(b.tape, REL_DEAD);
            let at = b.f.ins().ishl_imm(w, 3);
            let p = b.f.ins().iadd(dead, at);
            let word = b.word_at(I64, p, 0);
            let bit = b.f.ins().band_imm(id, 63);
            let bit = b.f.ins().uextend(I64, bit);
            let shifted = b.f.ins().ushr(word, bit);
            let one = b.f.ins().band_imm(shifted, 1);
            b.f.ins().ireduce(I32, one)
        },
        |b| b.c32(0),
    )
}

/// The address of the relation's index `idx`.
fn index(b: &mut B<'_, '_>, idx: Value) -> Value {
    let indexes = b.follow(b.tape, REL_INDEXES);
    b.item(indexes, idx, INDEX_SIZE)
}

/// `rel_group(rel, idx, h)`: the group of rows index `idx` holds for a key of hash `h` (its bucket, before liveness
/// and the key's own test): the first entry in the index's rows `<< 32`, or'd with how many. 0 for an index of no
/// buckets.
pub fn rel_group(b: &mut B<'_, '_>) -> Value {
    let (idx, h) = (b.arg(0), b.arg(1));
    let ix = index(b, idx);
    let buckets = b.word_at(I64, ix, INDEX_OFFS + SPAN_LEN);
    let none = b.f.ins().icmp_imm(IntCC::Equal, buckets, 0);
    b.either(
        I64,
        none,
        |b| b.c64(0),
        |b| {
            let mask = b.word_at(I64, ix, INDEX_MASK);
            let bucket = b.f.ins().band(h, mask);
            let offs = b.follow(ix, INDEX_OFFS);
            let at = b.f.ins().ishl_imm(bucket, 2);
            let p = b.f.ins().iadd(offs, at);
            let start = b.word_at(I32, p, 0);
            let end = b.word_at(I32, p, 4);
            let len = b.f.ins().isub(end, start);
            let start = b.f.ins().uextend(I64, start);
            let start = b.f.ins().ishl_imm(start, 32);
            let len = b.f.ins().uextend(I64, len);
            b.f.ins().bor(start, len)
        },
    )
}

/// `rel_group_row(rel, idx, k)`: entry `k` of index `idx`'s rows (a row id; a group is a run of them).
pub fn rel_group_row(b: &mut B<'_, '_>) -> Value {
    let (idx, k) = (b.arg(0), b.arg(1));
    let ix = index(b, idx);
    let rows = b.follow(ix, INDEX_ROWS);
    let p = b.item(rows, k, 4);
    b.word_at(I32, p, 0)
}

/// `rel_key_mix(rel, h, v)`: a key's hash `h` with its next value `v` mixed in (`rerail_blocks::mix`; a key's
/// hash is its values mixed in order into `rerail_blocks::SEED`). The one hash of a key: the host's index builder,
/// the device's and every probe's (decision 0325 step 5). It reads nothing of the relation.
pub fn rel_key_mix(b: &mut B<'_, '_>) -> Value {
    let (h, v) = (b.arg(0), b.arg(1));
    let h = b.f.ins().bxor(h, v);
    let h = b.f.ins().imul_imm(h, 0x9E37_79B9_7F4A_7C15_u64 as i64);
    let s = b.f.ins().ushr_imm(h, 29);
    b.f.ins().bxor(h, s)
}

#[cfg(test)]
mod tests {
    use rerail_blocks::block::{Block, kind};
    use rerail_blocks::relation::{RelW, Relation, Spec, freeze};
    use std::sync::Mutex;

    /// The JIT's relation reads (each called through an exported function, its hot reads inlined into it: the pass
    /// checked as `jit::Reads` builds) read a relation as the blocks crate's host reads do: every row's value in a
    /// 4-byte and an 8-byte column over many chunks, through its rows and through a column resolved once, its dead
    /// bit and its key's index group, for a frozen base and a frozen delta of it.
    #[test]
    fn the_jits_relation_reads_are_the_hosts() {
        let reads = crate::jit::Reads::new();
        type Two = extern "C" fn(*const u8, u32, u32) -> u64;
        type One = extern "C" fn(*const u8, u32) -> u32;
        type Group = extern "C" fn(*const u8, u32, u64) -> u64;
        // SAFETY: each read's signature: its descriptor's address, then its parameters.
        let (rows_get, col_get, rel_get): (Two, Two, Two) =
            unsafe {
                (
                    std::mem::transmute::<*const u8, Two>(reads.function("rows_get")),
                    std::mem::transmute::<*const u8, Two>(reads.function("col_get")),
                    std::mem::transmute::<*const u8, Two>(reads.function("rel_get")),
                )
            };
        let rel_dead: One = unsafe { std::mem::transmute(reads.function("rel_dead")) };
        let rel_group: Group = unsafe { std::mem::transmute(reads.function("rel_group")) };

        let spec = Spec { widths: vec![8, 4], order: vec![0], indexes: vec![vec![1]], min_log: 4 };
        let lock = Mutex::new(());
        let base_block = Block::anon(kind::FACTS, 16 << 20);
        let base = Relation::create(&base_block, &spec, None);
        let rows: Vec<i64> = (0..5_000i64).flat_map(|i| [(i * 7919) << 33 | i, i % 37]).collect();
        RelW { block: &base_block, rel: base, lock: &lock }.extend(&rows);
        freeze(&base_block, base, &spec, None);
        let top_block = Block::anon(kind::FACTS, 16 << 20);
        let top = Relation::create(&top_block, &spec, Some(base));
        let w = RelW { block: &top_block, rel: top, lock: &lock };
        for id in (0..base.rows.len()).step_by(13) {
            w.kill(id);
        }
        w.extend(&(0..700i64).flat_map(|i| [-i - 1, 40 + i % 5]).collect::<Vec<_>>());
        freeze(&top_block, top, &spec, Some(base));

        for rel in [base, top] {
            let desc = std::ptr::from_ref(rel).cast::<u8>();
            let rows = std::ptr::from_ref(&rel.rows).cast::<u8>();
            let cols = [rel.rows.col(0), rel.rows.col(1)];
            for i in 0..rel.rows.len() {
                let id = rel.base_rows + i;
                for c in 0..2u32 {
                    let want = rel.rows.get(i, c as usize);
                    assert_eq!(rows_get(rows, i, c) as i64, want, "rows_get row {i} column {c}");
                    let dir = std::ptr::from_ref(cols[c as usize]).cast::<u8>();
                    assert_eq!(col_get(dir, rel.rows.min_log, i) as i64, want, "col_get row {i} column {c}");
                    assert_eq!(rel_get(desc, id, c) as i64, want, "rel_get row {id} column {c}");
                }
                assert_eq!(rel_dead(desc, i) != 0, rel.dead_bit(i), "rel_dead row {i}");
                let key = rel.own(id, 1);
                let h = rerail_blocks::key_hash(&[key]);
                let group = rel.group(0, h);
                let g = rel_group(desc, 0, h);
                let index_rows = rel.index(0).rows.as_slice();
                let start = if group.is_empty() { 0 } else { (group.as_ptr() as usize - index_rows.as_ptr() as usize) / 4 };
                assert_eq!(g, ((start as u64) << 32) | group.len() as u64, "rel_group of row {id}'s key");
            }
        }
    }

    /// Decision 0463: the device's row writer (`rows_put`) writes a row the chunks hold at its column's width, and
    /// a row past them, or a column past the columns, not at all: it counts the rows' `needed`, and the relation's
    /// directory and descriptor are as they were (a row past the chunks made would have found a chunk offset of 0,
    /// its address the column's directory: Q265).
    #[test]
    fn the_devices_row_writer_stays_in_its_rows() {
        let reads = crate::jit::Reads::new();
        type Put = extern "C" fn(*const u8, u32, u32, u64) -> u32;
        type Two = extern "C" fn(*const u8, u32, u32) -> u64;
        // SAFETY: each read's signature: its descriptor's address, then its parameters.
        let (put, get): (Put, Two) = unsafe { (std::mem::transmute::<*const u8, Put>(reads.function("rows_put")), std::mem::transmute::<*const u8, Two>(reads.function("rows_get"))) };
        let spec = Spec { widths: vec![8, 4], order: vec![0], indexes: vec![], min_log: 4 };
        let lock = Mutex::new(());
        let block = Block::anon(kind::FACTS, 1 << 20);
        let rel = Relation::create(&block, &spec, None);
        RelW { block: &block, rel, lock: &lock }.extend(&(0..40i64).flat_map(|i| [i, i]).collect::<Vec<_>>());
        let rows = std::ptr::from_ref(&rel.rows).cast::<u8>();
        let reserved = rel.rows.reserved.load(std::sync::atomic::Ordering::Relaxed);
        assert!(reserved >= 40);
        let descriptor = |r: &Relation| unsafe { std::slice::from_raw_parts(std::ptr::from_ref(r).cast::<u8>(), std::mem::size_of::<Relation>()) }.to_vec();
        let dirs = |r: &Relation| r.rows.cols.as_slice().iter().map(|d| (d.width, d.chunk.iter().map(|c| c.load(std::sync::atomic::Ordering::Relaxed)).collect::<Vec<_>>())).collect::<Vec<_>>();
        // In its rows: written at the column's width (the 4-byte column's next row untouched).
        assert_eq!(put(rows, 7, 0, 0x0101_463c_52c3_00cd), 1);
        assert_eq!(put(rows, 7, 1, 0xFFFF_FFFF_0000_0009), 1);
        assert_eq!(get(rows, 7, 0), 0x0101_463c_52c3_00cd);
        assert_eq!((get(rows, 7, 1), get(rows, 8, 1)), (9, 8));
        // What an unchecked writer's address was (`rows_at`): a row past the chunks made, inside its column's
        // directory, whose chunk offsets it would overwrite.
        // SAFETY: `rows_at`'s signature; it reads the descriptor and the directory only.
        let at: Two = unsafe { std::mem::transmute(reads.function("rows_at")) };
        let col = std::ptr::from_ref(&rel.rows.cols.as_slice()[0]) as u64;
        let wild = at(rows, reserved + 16, 0);
        assert!((col..col + std::mem::size_of::<rerail_blocks::rows::ColDir>() as u64).contains(&wild), "row {} at {wild:#x}, the directory at {col:#x}", reserved + 16);
        // Past its chunks, or its columns: nothing written, each counted.
        let (before, dir) = (descriptor(rel), dirs(rel));
        for (i, c) in [(reserved, 0), (reserved, 1), (reserved + 5000, 0), (u32::MAX, 1), (3, 2), (3, u32::MAX)] {
            assert_eq!(put(rows, i, c, 0x0101_463c_52c3_00cd), 0, "row {i} column {c}");
        }
        assert_eq!(rel.rows.needed.load(std::sync::atomic::Ordering::Relaxed), 6);
        let mut now = descriptor(rel);
        let at = std::mem::offset_of!(Relation, rows) + std::mem::offset_of!(rerail_blocks::rows::Rows, needed);
        now[at..at + 4].copy_from_slice(&before[at..at + 4]);
        assert_eq!(now, before, "the descriptor, but for its count");
        assert_eq!(dirs(rel), dir, "the columns' directories");
    }

    /// The host's chunk arithmetic for ranges (`rows::chunk_of`) is the reads': every row's address through
    /// `chunk_of` is the one `rows_at` gives.
    #[test]
    fn the_hosts_chunk_arithmetic_is_the_reads() {
        let spec = Spec { widths: vec![8], order: vec![0], indexes: vec![], min_log: 4 };
        let lock = Mutex::new(());
        let block = Block::anon(kind::FACTS, 16 << 20);
        let rel = Relation::create(&block, &spec, None);
        RelW { block: &block, rel, lock: &lock }.extend(&(0..20_000i64).collect::<Vec<_>>());
        let dir = rel.rows.col(0);
        for i in 0..rel.rows.len() {
            let (k, l) = rerail_blocks::rows::chunk_of(i, rel.rows.min_log);
            let by_host = std::ptr::from_ref(dir) as i64 + dir.chunk[k].load(std::sync::atomic::Ordering::Relaxed) + 8 * l as i64;
            // SAFETY: a live descriptor and a reserved row.
            let by_read = unsafe { rerail_blocks::generated_reads::rows_at(std::ptr::from_ref(&rel.rows).cast(), i, 0) } as i64;
            assert_eq!(by_read, by_host, "row {i}");
        }
    }
}
