//! The join program's reads of a batch's views and atoms, in Rust (decisions 0534, 0536, 0704): a view's row is a
//! read of its document's tape, numbering, lines and source where they lie, never a relation made of it (decision
//! 0353), through the views' one Rust reader, the host executor's (`rerail_reads::view_reads::Doc`). Here a
//! document is found from its directory's row (`doc`), and a row of a view from it once (`At`); and the atoms
//! directory's reads (`atoms_len`, `atoms_str`, `atoms_find`), the CLIF's written directly. A test holds each to its
//! CLIF over real batches' directories (`tests_lang`: `the_join_programs_view_reads_are_the_jits`).
//!
//! # Safety
//!
//! Every read takes a document of a directory in its batch's block (`doc`'s caller's), and rows and nodes within
//! what it holds, as its CLIF does.
#![allow(unsafe_op_in_unsafe_fn, clippy::missing_safety_doc)]

use rerail_reads::view_reads::{Doc, Place, Tape};
use rerail_reads::views::{ViewDir, ViewDoc, ViewHome, ViewRow, code_atoms};

/// The directory's block: its address less its offset in it.
#[inline]
pub unsafe fn block(d: *const ViewDir) -> *const u8 {
    d.cast::<u8>().wrapping_sub((*d).at as usize)
}

/// Home `h`'s bytes at `at`, by its host address: this executor is the host's, whatever device the directory is also
/// made for (`ViewHome::gpu` is that device's).
#[inline]
pub unsafe fn home(d: *const ViewDir, h: u32, at: u64) -> *const u8 {
    let homes = block(d).add((*d).homes as usize).cast::<ViewHome>();
    ((*homes.add(h as usize)).host as *const u8).wrapping_add(at as usize)
}

/// Directory `d`'s row `m`.
#[inline]
pub unsafe fn row<'a>(d: *const ViewDir, m: u32) -> &'a ViewRow {
    &*block(d).add((*d).rows as usize).cast::<ViewRow>().add(m as usize)
}

/// Directory `d`'s document `m` as the views' reads take it: its entry, its tape at its home's host address (a 16z
/// tape with the cache its batch's reads share, the directory's home of it), its line index and its source where they
/// lie, and its language's codes in the directory's codes' home (decision 0697).
#[inline]
pub unsafe fn doc<'a>(d: *const ViewDir, m: u32) -> Doc<'a> {
    let row = row(d, m);
    let e = &*block(d).add(row.entry as usize).cast::<ViewDoc>();
    let tape = core::slice::from_raw_parts(home(d, row.homes[0], row.tape), row.bytes as usize);
    let lines = home(d, row.homes[1], row.lines);
    // Its line index (decision 0612), its length from its head, and its word's room after it.
    let len = rerail_reads::lines::len_of(core::slice::from_raw_parts(lines, rerail_reads::lines::HEAD)).unwrap_or(0);
    let lines = core::slice::from_raw_parts(lines, len + 8);
    let source = core::slice::from_raw_parts(home(d, row.homes[2], row.source), e.source_len as usize);
    let cache = match (*d).cache {
        0 => core::ptr::null(),
        k => home(d, k - 1, 0),
    };
    let codes = home(d, (*d).codes, 0);
    let tag = e.tag as u8;
    Doc::new(e, Tape::new(tape, &row.view, cache), lines, source, (code_atoms(codes, tag, 0), code_atoms(codes, tag, 1)))
}

/// A view's row found once (`at`): its document, its view (`views::VIEWS`) and its row in the document. Its document
/// keeps the tape node its columns last selected, so a row's columns select it once.
#[derive(Clone)]
pub struct At<'a> {
    d: Doc<'a>,
    k: u32,
    r: u32,
}

/// View `k`'s (`views::VIEWS`) row `r` of document `d`.
#[inline]
pub fn at(d: Doc<'_>, k: u32, r: u32) -> At<'_> {
    At { d, k, r }
}

impl At<'_> {
    /// Its column `col`.
    #[inline]
    pub fn col(&self, c: u32) -> u64 {
        col(&self.d, self.k, self.r, c)
    }
}

/// Column `col` of view `k`'s (`views::VIEWS`) row `r` of document `d`: through `d`, which keeps the tape node it last
/// selected, so a row's columns select it once.
#[inline]
pub fn col(d: &Doc<'_>, k: u32, r: u32, col: u32) -> u64 {
    (match k {
        0 => d.node(r, col),
        1 => d.child(r, col),
        2 => d.field(r, col),
        3 => d.core(r, col),
        4 => d.line(r, col),
        5 => d.loc(Place::Line, r, col),
        6 => d.loc(Place::EndLine, r, col),
        7 => d.loc(Place::Col, r, col),
        8 => d.loc(Place::Docof, r, col),
        9 => d.tape_row(r, col),
        10 => d.depth_row(r, col),
        11 => d.order_row(r, col),
        _ => d.op(r, col),
    }) as u64
}

/// An atoms directory's layout (`rerail_blocks::atoms::AtomsDirRoot`, `DirEntry`; a test holds it to it): its
/// runs' count and entries, and each entry's first id, end, the id its strings start at, their ends and bytes, and
/// its table from hash to id (decision 0666).
pub(crate) mod atoms_at {
    pub const DIR_N: usize = 0;
    pub const DIR_ENTRIES: usize = 8;
    pub const E_FIRST: usize = 0;
    pub const E_END: usize = 4;
    pub const E_FROM: usize = 8;
    pub const E_ENDS: usize = 16;
    pub const E_STRINGS: usize = 32;
    pub const E_TABLE: usize = 48;
    pub const E_SIZE: usize = 64;
    /// A span's length, after its offset.
    pub const SPAN_LEN: usize = 8;
}

/// The address a span at `p + o` names (its own place plus its offset).
#[inline(always)]
unsafe fn follow(p: *const u8, o: usize) -> *const u8 {
    p.wrapping_add(o).wrapping_add(p.add(o).cast::<i64>().read_unaligned() as usize)
}

#[inline(always)]
unsafe fn r32(p: *const u8, o: usize) -> u32 {
    p.add(o).cast::<u32>().read_unaligned()
}

#[inline(always)]
unsafe fn r64(p: *const u8, o: usize) -> u64 {
    p.add(o).cast::<u64>().read_unaligned()
}

/// Directory `dir`'s runs: their count and their entries.
#[inline(always)]
unsafe fn atoms_runs(dir: *const u8) -> (u32, *const u8) {
    use atoms_at::*;
    (r64(dir, DIR_N) as u32, follow(dir, DIR_ENTRIES))
}

/// The entry of directory `dir`'s run holding atom `id`, if one does: the last whose first id is at most `id`, if
/// `id` is below its end.
#[inline]
unsafe fn atoms_entry(dir: *const u8, id: u32) -> Option<*const u8> {
    use atoms_at::*;
    let (n, entries) = atoms_runs(dir);
    let entry = |t: u32| entries.wrapping_add(t as usize * E_SIZE);
    let (mut lo, mut hi) = (0u32, n);
    while lo < hi {
        let m = lo + (hi - lo) / 2;
        if r32(entry(m), E_FIRST) <= id { lo = m + 1 } else { hi = m }
    }
    let e = entry(lo.max(1) - 1);
    (lo != 0 && id < r32(e, E_END)).then_some(e)
}

/// The offsets in its run's strings where atom `id` of entry `e` begins and ends.
#[inline]
unsafe fn atoms_bounds(e: *const u8, id: u32) -> (u64, u64) {
    use atoms_at::*;
    let i = id.wrapping_sub(r32(e, E_FROM)) as usize;
    let ends = follow(e, E_ENDS).cast::<u64>();
    (if i == 0 { 0 } else { ends.add(i - 1).read_unaligned() }, ends.add(i).read_unaligned())
}

/// The length of atom `id`'s string in directory `dir`, `u32::MAX` for an id no run holds (0 for id 0): the CLIF's
/// `atoms_len` (`rerail_blocks::atoms::str_in`'s).
#[inline]
pub unsafe fn atoms_len(dir: *const u8, id: u32) -> u32 {
    if id == 0 {
        return 0;
    }
    match atoms_entry(dir, id) {
        Some(e) => {
            let (start, end) = atoms_bounds(e, id);
            end.wrapping_sub(start) as u32
        }
        None => u32::MAX,
    }
}

/// The address of atom `id`'s string's first byte in directory `dir` (the directory's own for an id no run holds, or
/// for id 0's empty string): the CLIF's `atoms_at`.
#[inline]
pub unsafe fn atoms_str(dir: *const u8, id: u32) -> *const u8 {
    if id == 0 {
        return dir;
    }
    match atoms_entry(dir, id) {
        Some(e) => follow(e, atoms_at::E_STRINGS).wrapping_add(atoms_bounds(e, id).0 as usize),
        None => dir,
    }
}

/// The `want`th atom (from 0, the runs in order) whose slot in its run's table, probed from hash `h` to the first
/// empty slot, has `h`'s high half, whose id is one of the run's own (`first..end`), and whose string is `len` bytes;
/// `u32::MAX` past the last: the CLIF's `atoms_find` (decision 0666). The caller compares each one's bytes with its
/// string's and asks for the next on a difference: a string is one atom, so it stops at the first equal.
pub unsafe fn atoms_find(dir: *const u8, h: u64, len: u32, want: u32) -> u32 {
    use atoms_at::*;
    let (n, entries) = atoms_runs(dir);
    let tag = h & 0xFFFF_FFFF_0000_0000;
    let mut seen = 0u32;
    for r in 0..n {
        let e = entries.wrapping_add(r as usize * E_SIZE);
        let table = follow(e, E_TABLE).cast::<u64>();
        let slots = r64(e, E_TABLE + SPAN_LEN);
        let mask = slots.wrapping_sub(1);
        let (first, end) = (r32(e, E_FIRST), r32(e, E_END));
        let mut i = h & mask;
        for _ in 0..slots {
            let v = table.add(i as usize).read_unaligned();
            if v == 0 {
                break;
            }
            let id = (v as u32).wrapping_sub(1);
            if v & 0xFFFF_FFFF_0000_0000 == tag && id >= first && id < end {
                let (s0, s1) = atoms_bounds(e, id);
                if s1.wrapping_sub(s0) as u32 == len {
                    if seen == want {
                        return id;
                    }
                    seen += 1;
                }
            }
            i = (i + 1) & mask;
        }
    }
    u32::MAX
}
