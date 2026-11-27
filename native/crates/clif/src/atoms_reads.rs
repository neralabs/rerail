//! The atoms' reads as CLIF functions, written once (decision 0478 step 2b): an id's string in a directory of runs
//! of atoms (`rerail_blocks::atoms::AtomsDirRoot`), for the kernels (`<ukl/reads/relation.h>`) and the JIT (the host's,
//! `atoms::str_in`, written directly: decision 0536). A read takes a launch's arguments (`AtomsArgs`): the blocks the
//! launch passes, each its descriptor's address, and the table of which argument is which descriptor's stable id. An
//! entry names its run's block by that id, and byte offsets into it, so the directory holds no address and reads the
//! same on every device: the read finds the entry's id in the table and reads the block at its argument.
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
use rerail_blocks::atoms::{AtomsArgs, AtomsDirRoot, DirEntry};
use std::mem::{offset_of, size_of};

const DIR_N: i32 = offset_of!(AtomsDirRoot, n) as i32;
const DIR_ENTRIES: i32 = offset_of!(AtomsDirRoot, entries) as i32;
const A_N: i32 = offset_of!(AtomsArgs, n) as i32;
const A_DIR: i32 = offset_of!(AtomsArgs, dir) as i32;
const A_ROOT: i32 = offset_of!(AtomsArgs, root) as i32;
const A_IDS: i32 = offset_of!(AtomsArgs, ids) as i32;
const A_ARGS: i32 = offset_of!(AtomsArgs, args) as i32;
const E_DESC: i32 = offset_of!(DirEntry, desc) as i32;
const E_FIRST: i32 = offset_of!(DirEntry, first) as i32;
const E_END: i32 = offset_of!(DirEntry, end) as i32;
const E_FROM: i32 = offset_of!(DirEntry, from) as i32;
const E_ENDS: i32 = offset_of!(DirEntry, ends) as i32;
const E_STRINGS: i32 = offset_of!(DirEntry, strings) as i32;
const E_TABLE: i32 = offset_of!(DirEntry, table) as i32;
const E_SLOTS: i32 = offset_of!(DirEntry, slots) as i32;
const E_SIZE: i64 = size_of::<DirEntry>() as i64;

/// No string: an id no run holds.
const NONE: i64 = u32::MAX as i64;

impl B<'_, '_> {
    /// Argument `k`'s address (a block the launch passes): its descriptor's, from the launch's arguments.
    fn atoms_arg(&mut self, k: Value) -> Value {
        let slot = self.item(self.tape, k, 8);
        let at = self.word_at(I64, slot, A_ARGS);
        self.f.ins().iadd(self.tape, at)
    }

    /// The directory's root: in the block of argument `dir`, at `root`.
    fn atoms_dir(&mut self) -> Value {
        let k = self.word_at(I32, self.tape, A_DIR);
        let block = self.atoms_arg(k);
        let root = self.word_at(I32, self.tape, A_ROOT);
        let root = self.f.ins().uextend(I64, root);
        self.f.ins().iadd(block, root)
    }

    /// The address of entry `e`'s block (the argument whose stable id is the entry's, `ids` ascending, by a binary
    /// search of the table), and whether the launch passes it.
    fn atoms_block(&mut self, e: Value) -> (Value, Value) {
        let desc = self.word_at(I32, e, E_DESC);
        let nargs = self.word_at(I32, self.tape, A_N);
        let k = self.bisect(nargs, &mut |b, i| {
            let at = b.item(b.tape, i, 4);
            let d = b.word_at(I32, at, A_IDS);
            b.f.ins().icmp(IntCC::UnsignedLessThan, d, desc)
        });
        let within = self.f.ins().icmp(IntCC::UnsignedLessThan, k, nargs);
        let zero = self.c32(0);
        let k = self.f.ins().select(within, k, zero);
        let at = self.item(self.tape, k, 4);
        let d = self.word_at(I32, at, A_IDS);
        let same = self.f.ins().icmp(IntCC::Equal, d, desc);
        let passed = self.f.ins().band(within, same);
        (self.atoms_arg(k), passed)
    }

    /// The entry of the run holding `id` (a u32), the address of its block (the argument whose stable id is the
    /// entry's), and whether one does: the last entry whose first id is at most `id`, if `id` is below its end and the
    /// launch passes its block.
    fn atoms_entry(&mut self, id: Value) -> (Value, Value, Value) {
        let dir = self.atoms_dir();
        let n = self.word_at(I32, dir, DIR_N);
        let entries = self.follow_rel::<DirEntry>(dir, DIR_ENTRIES);
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
        let (block, passed) = self.atoms_block(e);
        let held = self.f.ins().band(held, passed);
        (e, block, held)
    }

    /// The address of entry `e`'s array at `field` (a byte offset into its block, at `block`).
    fn atoms_in_block(&mut self, e: Value, block: Value, field: i32) -> Value {
        let off = self.word_at(I32, e, field);
        let off = self.f.ins().uextend(I64, off);
        self.f.ins().iadd(block, off)
    }

    /// The offsets in its run's strings where atom `id` of entry `e` begins and ends (u64s).
    fn atoms_bounds(&mut self, e: Value, block: Value, id: Value) -> (Value, Value) {
        let from = self.word_at(I32, e, E_FROM);
        let i = self.f.ins().isub(id, from);
        let ends = self.atoms_in_block(e, block, E_ENDS);
        let at = self.item(ends, i, 8);
        let end = self.word_at(I64, at, 0);
        let first = self.f.ins().icmp_imm(IntCC::Equal, i, 0);
        let start = self.either(I64, first, |b| b.c64(0), |b| b.word_at(I64, at, -8));
        (start, end)
    }
}

/// `atoms_len(args, id)`: the length of atom `id`'s string, `u32::MAX` for an id no run holds (`0` for id 0).
pub fn atoms_len(b: &mut B<'_, '_>) -> Value {
    let id = b.arg(0);
    let empty = b.f.ins().icmp_imm(IntCC::Equal, id, 0);
    b.either(
        I32,
        empty,
        |b| b.c32(0),
        |b| {
            let (e, block, held) = b.atoms_entry(id);
            b.either(
                I32,
                held,
                |b| {
                    let (start, end) = b.atoms_bounds(e, block, id);
                    let len = b.f.ins().isub(end, start);
                    b.f.ins().ireduce(I32, len)
                },
                |b| b.c32(NONE),
            )
        },
    )
}

/// `atoms_at(args, id)`: the address of atom `id`'s string's first byte (the arguments' own for an id no run
/// holds, or for id 0's empty string).
pub fn atoms_at(b: &mut B<'_, '_>) -> Value {
    let id = b.arg(0);
    let args = b.tape;
    let empty = b.f.ins().icmp_imm(IntCC::Equal, id, 0);
    b.either(
        I64,
        empty,
        |_| args,
        |b| {
            let (e, block, held) = b.atoms_entry(id);
            b.either(
                I64,
                held,
                |b| {
                    let (start, _) = b.atoms_bounds(e, block, id);
                    let strings = b.atoms_in_block(e, block, E_STRINGS);
                    b.f.ins().iadd(strings, start)
                },
                |_| args,
            )
        },
    )
}

/// `atoms_find(args, h, len, k)`: the `k`th atom (from 0, the runs in order) whose slot in its run's table, probed
/// from hash `h` (a u64) to the first empty slot, has `h`'s high half, whose id is one of the run's own
/// (`first..end`), and whose string is `len` bytes; `u32::MAX` past the last. A run whose block the launch does not
/// pass holds none. The caller compares each one's bytes
/// with its string's and asks for the next on a difference: a string is one atom, so it stops at the first equal.
pub fn atoms_find(b: &mut B<'_, '_>) -> Value {
    let (h, len, want) = (b.arg(0), b.arg(1), b.arg(2));
    let dir = b.atoms_dir();
    let n = b.word_at(I32, dir, DIR_N);
    let entries = b.follow_rel::<DirEntry>(dir, DIR_ENTRIES);
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
    let (block, passed) = b.atoms_block(e);
    let table = b.atoms_in_block(e, block, E_TABLE);
    let slots = b.word_at(I32, e, E_SLOTS);
    let slots = b.f.ins().uextend(I64, slots);
    let zero64 = b.c64(0);
    let slots = b.f.ins().select(passed, slots, zero64);
    let mask = b.f.ins().iadd_imm(slots, -1);
    let start = b.f.ins().band(h, mask);
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
    let (s0, s1) = b.atoms_bounds(e, block, id);
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
