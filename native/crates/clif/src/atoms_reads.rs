//! The atoms' reads as CLIF functions, written once (decision 0478 step 2b): an id's string in a directory of runs
//! of atoms (`rerail_blocks::atoms::AtomsDirRoot`), for the host (the blocks crate's `generated_reads.rs`), the
//! kernels (`<ukl/reads/relation.h>`) and the JIT. A read takes the directory's address and reaches each run's `ends`
//! and `strings` through its spans, whose offsets are in the reader's own address space.
//!
//! An id's run is found by a binary search of the runs' first ids; its string ends at `ends[id - from]` and begins
//! where the one before it ends (`0` for the block's first). Id `0` is the empty string, in no run.

use crate::reads::B;
use cranelift_codegen::ir::{
    InstBuilder, Value,
    condcodes::IntCC,
    types::{I32, I64},
};
use rerail_blocks::atoms::{AtomsDirRoot, DirEntry};
use rerail_blocks::refs::Span;
use std::mem::{offset_of, size_of};

const DIR_N: i32 = offset_of!(AtomsDirRoot, n) as i32;
const DIR_ENTRIES: i32 = offset_of!(AtomsDirRoot, entries) as i32;
const E_FIRST: i32 = offset_of!(DirEntry, first) as i32;
const E_END: i32 = offset_of!(DirEntry, end) as i32;
const E_FROM: i32 = offset_of!(DirEntry, from) as i32;
const E_ENDS: i32 = offset_of!(DirEntry, ends) as i32;
const E_STRINGS: i32 = offset_of!(DirEntry, strings) as i32;
const E_SIZE: i64 = size_of::<DirEntry>() as i64;
const _: () = assert!(size_of::<Span<u8>>() == 16);

/// No string: an id no run holds.
const NONE: i64 = u32::MAX as i64;

impl B<'_, '_> {
    /// The entry of the run holding `id` (a u32), and whether one does: the last whose first id is at most `id`, if
    /// `id` is below its end.
    fn atoms_entry(&mut self, id: Value) -> (Value, Value) {
        let n = self.word_at(I64, self.tape, DIR_N);
        let n = self.f.ins().ireduce(I32, n);
        let entries = self.follow(self.tape, DIR_ENTRIES);
        let k = self.bisect(n, &mut |b, t| {
            let e = b.item(entries, t, E_SIZE);
            let first = b.word_at(I32, e, E_FIRST);
            b.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, first, id)
        });
        let any = self.f.ins().icmp_imm(IntCC::NotEqual, k, 0);
        // The entry before `k` (the first entry's when there is none, read but not used).
        let one = self.c32(1);
        let before = self.f.ins().isub(k, one);
        let zero = self.c32(0);
        let before = self.f.ins().select(any, before, zero);
        let e = self.item(entries, before, E_SIZE);
        let end = self.word_at(I32, e, E_END);
        let below = self.f.ins().icmp(IntCC::UnsignedLessThan, id, end);
        let held = self.f.ins().band(any, below);
        (e, held)
    }

    /// The offsets in its run's strings where atom `id` of entry `e` begins and ends (u64s).
    fn atoms_bounds(&mut self, e: Value, id: Value) -> (Value, Value) {
        let from = self.word_at(I32, e, E_FROM);
        let i = self.f.ins().isub(id, from);
        let ends = self.follow(e, E_ENDS);
        let at = self.item(ends, i, 8);
        let end = self.word_at(I64, at, 0);
        let first = self.f.ins().icmp_imm(IntCC::Equal, i, 0);
        let start = self.either(I64, first, |b| b.c64(0), |b| b.word_at(I64, at, -8));
        (start, end)
    }
}

/// `atoms_len(dir, id)`: the length of atom `id`'s string, `u32::MAX` for an id no run holds (`0` for id 0).
pub fn atoms_len(b: &mut B<'_, '_>) -> Value {
    let id = b.arg(0);
    let empty = b.f.ins().icmp_imm(IntCC::Equal, id, 0);
    b.either(
        I32,
        empty,
        |b| b.c32(0),
        |b| {
            let (e, held) = b.atoms_entry(id);
            b.either(
                I32,
                held,
                |b| {
                    let (start, end) = b.atoms_bounds(e, id);
                    let len = b.f.ins().isub(end, start);
                    b.f.ins().ireduce(I32, len)
                },
                |b| b.c32(NONE),
            )
        },
    )
}

/// `atoms_at(dir, id)`: the address of atom `id`'s string's first byte (the directory's own for an id no run holds,
/// or for id 0's empty string).
pub fn atoms_at(b: &mut B<'_, '_>) -> Value {
    let id = b.arg(0);
    let dir = b.tape;
    let empty = b.f.ins().icmp_imm(IntCC::Equal, id, 0);
    b.either(
        I64,
        empty,
        |_| dir,
        |b| {
            let (e, held) = b.atoms_entry(id);
            b.either(
                I64,
                held,
                |b| {
                    let (start, _) = b.atoms_bounds(e, id);
                    let strings = b.follow(e, E_STRINGS);
                    b.f.ins().iadd(strings, start)
                },
                |_| dir,
            )
        },
    )
}
