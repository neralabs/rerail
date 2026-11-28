//! The atoms' reads as CLIF functions, written once (decision 0478 step 2b): an id's string in a directory of runs
//! of atoms (`rerail_blocks::atoms::AtomsDirRoot`), for the kernels (`<ukl/reads/relation.h>`) and the JIT (the host's, `atoms::str_in`, written directly: decision 0536). A read takes the directory's address and reaches each run's `ends`
//! and `strings` through its spans, whose offsets are in the reader's own address space.
//!
//! An id's run is found by a binary search of the runs' first ids; its string ends at `ends[id - from]` and begins
//! where the one before it ends (`0` for the block's first). Id `0` is the empty string, in no run.
//!
//! A string is found by its hash (the store's, `content_store::atoms::hash_str`) in each run's table from hash to id
//! (`atoms_find`): the read gives the atoms whose slot has the hash's high half and whose string is as long, and its
//! caller compares their bytes, which lie where it reads them (the kernels' arena: another buffer than the
//! directory's).

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
const E_TABLE: i32 = offset_of!(DirEntry, table) as i32;
const SPAN_LEN: i32 = 8;
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

/// `atoms_find(dir, h, len, k)`: the `k`th atom (from 0, the runs in order) whose slot in its run's table, probed
/// from hash `h` (a u64) to the first empty slot, has `h`'s high half, whose id is one of the run's own
/// (`first..end`), and whose string is `len` bytes; `u32::MAX` past the last. The caller compares each one's bytes
/// with its string's and asks for the next on a difference: a string is one atom, so it stops at the first equal.
pub fn atoms_find(b: &mut B<'_, '_>) -> Value {
    let (h, len, want) = (b.arg(0), b.arg(1), b.arg(2));
    let n = b.word_at(I64, b.tape, DIR_N);
    let n = b.f.ins().ireduce(I32, n);
    let entries = b.follow(b.tape, DIR_ENTRIES);
    let tag = b.f.ins().band_imm(h, 0xFFFF_FFFF_0000_0000_u64 as i64);
    let (run, run_body, probe, slot, taken, check, next_slot, next_run, done) = (
        b.f.create_block(),
        b.f.create_block(),
        b.f.create_block(),
        b.f.create_block(),
        b.f.create_block(),
        b.f.create_block(),
        b.f.create_block(),
        b.f.create_block(),
        b.f.create_block(),
    );
    b.f.append_block_param(done, I32);
    // The run `r`, and how many atoms matched before it.
    b.f.append_block_param(run, I32);
    b.f.append_block_param(run, I32);
    let zero = b.c32(0);
    b.f.ins().jump(run, &[zero.into(), zero.into()]);
    b.f.switch_to_block(run);
    let (r, seen) = (b.f.block_params(run)[0], b.f.block_params(run)[1]);
    let more = b.f.ins().icmp(IntCC::UnsignedLessThan, r, n);
    let none = b.c32(NONE);
    b.f.ins().brif(more, run_body, &[], done, &[none.into()]);
    b.f.switch_to_block(run_body);
    let e = b.item(entries, r, E_SIZE);
    let table = b.follow(e, E_TABLE);
    let slots = b.word_at(I64, e, E_TABLE + SPAN_LEN);
    let mask = b.f.ins().iadd_imm(slots, -1);
    let start = b.f.ins().band(h, mask);
    let zero64 = b.c64(0);
    // The probe `p` (of at most `slots`), at slot `i`, `seen` so far.
    b.f.append_block_param(probe, I64);
    b.f.append_block_param(probe, I64);
    b.f.append_block_param(probe, I32);
    b.f.ins().jump(probe, &[zero64.into(), start.into(), seen.into()]);
    b.f.switch_to_block(probe);
    let (p, i, seen) = (b.f.block_params(probe)[0], b.f.block_params(probe)[1], b.f.block_params(probe)[2]);
    let r1 = b.f.ins().iadd_imm(r, 1);
    b.f.append_block_param(next_run, I32);
    b.f.append_block_param(next_run, I32);
    let within = b.f.ins().icmp(IntCC::UnsignedLessThan, p, slots);
    b.f.ins().brif(within, slot, &[], next_run, &[r1.into(), seen.into()]);
    b.f.switch_to_block(slot);
    let at = b.f.ins().ishl_imm(i, 3);
    let at = b.f.ins().iadd(table, at);
    let v = b.word_at(I64, at, 0);
    let empty = b.f.ins().icmp_imm(IntCC::Equal, v, 0);
    b.f.ins().brif(empty, next_run, &[r1.into(), seen.into()], taken, &[]);
    b.f.switch_to_block(taken);
    let high = b.f.ins().band_imm(v, 0xFFFF_FFFF_0000_0000_u64 as i64);
    let same = b.f.ins().icmp(IntCC::Equal, high, tag);
    let id = b.f.ins().ireduce(I32, v);
    let id = b.f.ins().iadd_imm(id, -1);
    let first = b.word_at(I32, e, E_FIRST);
    let end = b.word_at(I32, e, E_END);
    let lo = b.f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, id, first);
    let hi = b.f.ins().icmp(IntCC::UnsignedLessThan, id, end);
    let own = b.f.ins().band(lo, hi);
    let own = b.f.ins().band(own, same);
    b.f.append_block_param(next_slot, I32);
    b.f.ins().brif(own, check, &[], next_slot, &[seen.into()]);
    b.f.switch_to_block(check);
    let (s0, s1) = b.atoms_bounds(e, id);
    let l = b.f.ins().isub(s1, s0);
    let l = b.f.ins().ireduce(I32, l);
    let long = b.f.ins().icmp(IntCC::Equal, l, len);
    let this = b.f.ins().icmp(IntCC::Equal, seen, want);
    let found = b.f.ins().band(long, this);
    let seen1 = b.f.ins().iadd_imm(seen, 1);
    let seen = b.f.ins().select(long, seen1, seen);
    let go_on = b.f.create_block();
    b.f.ins().brif(found, done, &[id.into()], go_on, &[]);
    b.f.switch_to_block(go_on);
    b.f.ins().jump(next_slot, &[seen.into()]);
    b.f.switch_to_block(next_slot);
    let seen = b.f.block_params(next_slot)[0];
    let p1 = b.f.ins().iadd_imm(p, 1);
    let i1 = b.f.ins().iadd_imm(i, 1);
    let i1 = b.f.ins().band(i1, mask);
    b.f.ins().jump(probe, &[p1.into(), i1.into(), seen.into()]);
    b.f.switch_to_block(next_run);
    let (r, seen) = (b.f.block_params(next_run)[0], b.f.block_params(next_run)[1]);
    b.f.ins().jump(run, &[r.into(), seen.into()]);
    b.f.switch_to_block(done);
    b.f.block_params(done)[0]
}
