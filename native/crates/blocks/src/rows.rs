//! Rows: a relation's columns, each in chunks that grow by quarter steps. Rows come in doubling groups: group `k`
//! holds `2^(min_log + k)` rows, so row `i` is in group `63 - clz((i >> min_log) + 1)`, and a group is four
//! chunks of a quarter of it each. A relation's room unwritten is then at most a quarter of its last group (an
//! eighth of its rows), where whole doubling chunks left up to half. Arithmetic, the same on a GPU. Every
//! column's chunk `s` covers the same rows, so one claim covers a whole row.
//!
//! Appending: a writer claims rows with one atomic add on `len`. Chunks are made by `reserve` (under the
//! relation's lock on the host; between passes for a GPU), never while a kernel writes.

use crate::block::{Block, PAGE};
use crate::refs::Span;
use crate::structure;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, AtomicU32, Ordering::*};

/// Chunks a column may have: four a doubling group, 24 groups (2^(min_log + 24) rows; 2^28 at the smallest
/// `min_log`, which is 4).
pub const DIRS: usize = 96;

/// A chunk's offset with this bit set is a sealed chunk's (decision 0622): the offset (less the bit) is that of an
/// `IntCol`, the chunk's values in the frozen columns' encoding, its words after it in one piece of the block. A raw
/// chunk's offset is a multiple of 64, so the bit is free.
pub const SEALED: i64 = 1;
/// The smallest `min_log`: a group's quarter is at least 4 rows.
pub const MIN_LOG: u32 = 4;

structure! {
    /// One column: its width in bytes (4 or 8), its key runs, and each chunk's offset from this descriptor.
    ///
    /// A 4-byte column may hold the low 32 bits of values whose high 32 bits are its rows' runs' (`runs`: the
    /// offset, in 8-byte words, from this descriptor to a run table in the same block, `KeyRuns`): an axis's node
    /// ids, each its document's key over its node's low bits, the runs one a document. 0: none, the row's 4
    /// bytes its value.
    pub struct ColDir {
        pub width: u32,
        pub runs: i32,
        pub chunk: [AtomicI64; 96],
    }
}

structure! {
    pub struct Rows {
        /// Rows claimed.
        pub len: AtomicU32,
        /// Rows the chunks made so far can hold.
        pub reserved: AtomicU32,
        pub min_log: u32,
        pub nchunks: AtomicU32,
        /// Rows a writer could not write, past `reserved` (the overflow protocol: rerun at the exact size).
        pub needed: AtomicU32,
        pub ncols: u32,
        pub cols: Span<ColDir>,
    }
}

/// The bytes a sealed chunk's piece holds: its descriptor and its words (`Rows::seal`).
fn sealed_bytes(c: &crate::col::IntCol) -> usize {
    std::mem::size_of::<crate::col::IntCol>() + c.bytes()
}

/// Row `i`'s chunk and its place in it.
#[inline]
pub fn chunk_of(i: u32, min_log: u32) -> (usize, usize) {
    let q = ((i as u64) >> min_log) + 1;
    let k = 63 - q.leading_zeros();
    let l = i as u64 - (((1u64 << k) - 1) << min_log);
    let qlog = min_log + k - 2;
    (4 * k as usize + (l >> qlog) as usize, (l & ((1u64 << qlog) - 1)) as usize)
}

impl ColDir {
    /// Where row `i`'s value lies, its rows' first chunk `2^min_log` rows: its chunk's offset from this directory,
    /// then its place at the column's width (the reads in CLIF's `col_at`, the kernels' and the JIT's).
    #[inline(always)]
    pub fn at(&self, min_log: u32, i: u32) -> *mut u8 {
        let (chunk, place) = chunk_of(i, min_log);
        debug_assert!(self.chunk[chunk].load(Relaxed) & SEALED == 0, "an address in a sealed chunk (decision 0622)");
        let start = std::ptr::from_ref(self).cast::<u8>().wrapping_offset(self.chunk[chunk].load(Relaxed) as isize);
        start.wrapping_add(place * self.width as usize).cast_mut()
    }

    /// Row `i`'s value (a 4-byte column's unsigned, over its run's high bits when it has runs; the CLIF's
    /// `col_get`).
    ///
    /// # Safety
    /// Row `i` is reserved: its chunk is made.
    #[inline(always)]
    pub unsafe fn get(&self, min_log: u32, i: u32) -> i64 {
        let (chunk, place) = chunk_of(i, min_log);
        if let Some(sealed) = self.sealed(chunk) {
            return sealed.get(place);
        }
        let p = self.at(min_log, i);
        // SAFETY: the row's value in its chunk, aligned to the column's width (the caller's); its run in the table
        // the column was given (`Rows::set_runs`).
        unsafe {
            if self.width == 4 {
                let low = i64::from(*p.cast::<u32>());
                if self.runs == 0 { low } else { i64::from(self.high(i)) << 32 | low }
            } else {
                *p.cast::<i64>()
            }
        }
    }

    /// Row `i`'s 4 bytes as written: a column with runs holds its value's low 32 bits.
    ///
    /// # Safety
    /// Row `i` is reserved, and the column is 4 bytes wide.
    #[inline(always)]
    pub unsafe fn low(&self, min_log: u32, i: u32) -> u32 {
        let (chunk, place) = chunk_of(i, min_log);
        if let Some(sealed) = self.sealed(chunk) {
            return sealed.get(place) as u32;
        }
        // SAFETY: the caller's.
        unsafe { *self.at(min_log, i).cast::<u32>() }
    }

    /// Row `i`'s high 32 bits: its run's, the last whose first row is not past `i` (a search of the run table,
    /// a few entries, one a document; the CLIF's `col_high`).
    ///
    /// # Safety
    /// The column has runs, the first from row 0.
    #[inline(always)]
    pub unsafe fn high(&self, i: u32) -> u32 {
        // SAFETY: the caller's; the table is in this descriptor's block, `runs` words from it: its count, then each
        // run's first row and high bits.
        unsafe {
            let t = std::ptr::from_ref(self).cast::<u64>().offset(self.runs as isize).cast::<u32>();
            let (mut lo, mut hi) = (0u32, *t);
            while lo < hi {
                let m = lo + (hi - lo) / 2;
                if *t.add(2 + 2 * m as usize) <= i { lo = m + 1 } else { hi = m }
            }
            *t.add(2 * lo as usize + 1)
        }
    }

    /// The key runs' table: its count, and run `m`'s first row and high bits (`high`'s table), where the column has
    /// runs.
    ///
    /// # Safety
    /// The column's runs were set (`Rows::set_runs`).
    #[inline]
    unsafe fn runs_table(&self) -> (*const u32, u32) {
        // SAFETY: the caller's, as `high`'s.
        unsafe {
            let t = std::ptr::from_ref(self).cast::<u64>().offset(self.runs as isize).cast::<u32>();
            (t, *t)
        }
    }

    /// Chunk `k`'s values in the frozen encoding, where it is sealed (decision 0622).
    #[inline(always)]
    pub fn sealed(&self, k: usize) -> Option<&crate::col::IntCol> {
        let off = self.chunk[k].load(Relaxed);
        // SAFETY: a sealed chunk's offset (less its tag) is its `IntCol`'s from this directory, in the same block.
        (off & SEALED != 0).then(|| unsafe { &*std::ptr::from_ref(self).cast::<u8>().offset((off & !SEALED) as isize).cast::<crate::col::IntCol>() })
    }

    /// Whether the column holds low bits under key runs.
    #[inline(always)]
    pub fn has_runs(&self) -> bool {
        self.runs != 0
    }
}

/// A run table (`ColDir::runs`) of `n` runs in `block`, every run's first row 0 and high bits 0 until set
/// (`put_run`): 8 bytes a run past its count.
pub fn alloc_runs(block: &Block, n: usize) -> *mut u32 {
    let t: *mut u32 = block.alloc_t((2 + 2 * n).max(4));
    // SAFETY: just allocated, zeroed, for the count and `n` runs.
    unsafe { *t = u32::try_from(n).expect("runs in a u32") };
    t
}

/// Run `k` of the table at `t`: rows from `first` (each run's past the one's before, the first's 0) hold values
/// whose high 32 bits are `high`.
///
/// # Safety
/// `t` is a run table of more than `k` runs (`alloc_runs`), not yet read.
pub unsafe fn put_run(t: *mut u32, k: usize, first: u32, high: u32) {
    // SAFETY: the caller's.
    unsafe {
        debug_assert!(k < *t as usize);
        *t.add(2 + 2 * k) = first;
        *t.add(3 + 2 * k) = high;
    }
}

/// Rows chunk `s` holds.
#[inline]
pub fn chunk_rows(s: usize, min_log: u32) -> u64 {
    1u64 << (min_log as usize + s / 4 - 2)
}

/// The first row of chunk `s`.
#[inline]
pub fn chunk_first(s: usize, min_log: u32) -> u64 {
    let k = s / 4;
    (((1u64 << k) - 1) << min_log) + (s % 4) as u64 * chunk_rows(s, min_log)
}

/// Rows the first `nchunks` chunks hold.
#[inline]
pub fn capacity(nchunks: u32, min_log: u32) -> u64 {
    chunk_first(nchunks as usize, min_log)
}

/// One column, resolved once (its directory, width and `min_log`): a reader of many of its rows pays only the
/// chunk arithmetic and one load a row.
#[derive(Clone, Copy)]
pub struct ColRead {
    dir: *const ColDir,
    width: u32,
    min_log: u32,
}

impl ColRead {
    /// Row `i`'s value (`ColDir::get`; the CLIF's `col_get`, decision 0325, the kernels' and the JIT's).
    #[inline(always)]
    pub fn get(&self, i: u32) -> i64 {
        // SAFETY: the directory is its relation's, which outlives the reader; row `i` was claimed and reserved.
        unsafe { (*self.dir).get(self.min_log, i) }
    }
}

impl Rows {
    /// Column `c`, resolved for many reads.
    #[inline]
    pub fn reader(&self, c: usize) -> ColRead {
        let d = self.col(c);
        ColRead { dir: d, width: d.width, min_log: self.min_log }
    }

    /// # Safety
    /// `this` is zeroed memory in `block`, which is open.
    pub unsafe fn init(this: *mut Rows, block: &Block, widths: &[u32], min_log: u32) {
        assert!(unsafe { Rows::try_init(this, block, widths, min_log) }, "block full: {} columns' directories", widths.len());
    }

    /// `init`, or false when the block cannot hold the columns' directories.
    ///
    /// # Safety
    /// As `init`.
    pub unsafe fn try_init(this: *mut Rows, block: &Block, widths: &[u32], min_log: u32) -> bool {
        let Some(cols) = block.try_alloc_t::<ColDir>(widths.len()) else { return false };
        unsafe { Rows::init_in(this, cols, widths, min_log) };
        true
    }

    /// `init` with the columns' directories at `cols` (zeroed, `widths.len()` of them, in the same block).
    ///
    /// # Safety
    /// As `init`, and `cols` as said.
    pub unsafe fn init_in(this: *mut Rows, cols: *mut ColDir, widths: &[u32], min_log: u32) {
        assert!(min_log >= MIN_LOG, "min_log {min_log} under {MIN_LOG}");
        unsafe {
            for (c, &w) in widths.iter().enumerate() {
                assert!(w == 4 || w == 8);
                (*cols.add(c)).width = w;
                (*cols.add(c)).runs = 0;
            }
            (*this).min_log = min_log;
            (*this).ncols = widths.len() as u32;
            Span::set(&raw mut (*this).cols, cols, widths.len());
        }
    }

    #[inline]
    pub fn len(&self) -> u32 {
        self.len.load(Acquire)
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The rows the chunks made hold.
    #[inline]
    pub fn reserved(&self) -> u32 {
        self.reserved_now()
    }

    #[inline]
    fn reserved_now(&self) -> u32 {
        self.reserved.load(Acquire)
    }

    #[inline]
    pub fn col(&self, c: usize) -> &ColDir {
        &self.cols.as_slice()[c]
    }

    /// Where row `i`'s value in column `c` lies, and the column's width (the CLIF's `rows_at`, `rows_width`).
    #[inline]
    fn at(&self, i: u32, c: usize) -> (*mut u8, u32) {
        let d = self.col(c);
        (d.at(self.min_log, i), d.width)
    }

    /// Row `i`'s value in column `c` (the CLIF's `rows_get`, decision 0325, the kernels' and the JIT's).
    #[inline]
    pub fn get(&self, i: u32, c: usize) -> i64 {
        // SAFETY: row `i` was claimed and reserved.
        unsafe { self.col(c).get(self.min_log, i) }
    }

    /// # Safety
    /// Row `i` was claimed by the caller and reserved.
    #[inline]
    pub unsafe fn put(&self, i: u32, c: usize, v: i64) {
        let (p, w) = self.at(i, c);
        unsafe {
            if w == 4 {
                // A column with runs holds a value's low bits, its high bits its row's run's.
                let d = self.col(c);
                let fits = if d.has_runs() { (v as u64 >> 32) as u32 == d.high(i) } else { v >> 32 == 0 };
                assert!(fits, "{v} in 4-byte column {c}: not its row {i}'s high bits");
                *(p as *mut u32) = v as u32
            } else {
                *(p as *mut i64) = v
            }
        }
    }

    /// Column `c`, 4 bytes wide, under the key runs of the table at `runs` (`alloc_runs`, in the block this relation
    /// lies in): each row then holds its value's low 32 bits, its high bits its run's, for every reader (the CLIF's
    /// `col_get`, the kernels' and the JIT's). Set before any row is written, by its one writer; the columns of a
    /// relation may share one table.
    ///
    /// # Safety
    /// `runs` is in the same block as the relation and outlives it there; its runs are set, the first from row 0,
    /// before any row is written.
    pub unsafe fn set_runs(&self, c: usize, runs: *const u32) {
        let d = self.col(c);
        assert_eq!(d.width, 4, "runs of high bits over a 4-byte column");
        assert!(self.is_empty(), "runs given before the column's rows");
        let off = runs as isize - std::ptr::from_ref(d) as isize;
        assert!(off % 8 == 0 && off != 0, "a run table at a word from the column's directory");
        let words = i32::try_from(off / 8).expect("a run table within 16 GB of the column's directory");
        // SAFETY: the directory's one writer, before any reader (asserted empty).
        unsafe { (*std::ptr::from_ref(d).cast_mut()).runs = words };
    }

    /// Makes chunks until `n` rows fit. The caller holds the relation's lock, or no kernel is running.
    pub fn reserve(&self, block: &Block, n: u64) {
        assert!(self.try_reserve(block, n), "block full: rows to {n} past {} of {}", block.used(), block.reserved());
    }

    /// `reserve`, or false when the block cannot hold the chunks: the rows reserved are then as they were (a chunk
    /// counts once every column has its room). Each chunk's room is checked for every column before any is
    /// allocated; a writer of another relation in the block can still take it between, and then the columns'
    /// room already taken is left unused.
    pub fn try_reserve(&self, block: &Block, n: u64) -> bool {
        self.try_reserve_keeping(block, n, |_| 0)
    }

    /// `try_reserve`, leaving `keep(rows)` bytes of the block free past the chunks for `rows` rows in all: room the
    /// relation's indexes take later (decision 0353 step 6), which its own rows' growth does not use up. False when
    /// a chunk would take it.
    pub fn try_reserve_keeping(&self, block: &Block, n: u64, keep: impl Fn(u64) -> usize) -> bool {
        let mut nchunks = self.nchunks.load(Relaxed);
        while capacity(nchunks, self.min_log) < n {
            let k = nchunks as usize;
            assert!(k < DIRS, "rows past {DIRS} chunks");
            let rows = chunk_rows(k, self.min_log) as usize;
            let align = |bytes: usize| if bytes >= PAGE { PAGE } else { 64 };
            let need: usize = (0..self.ncols as usize).map(|c| rows * self.col(c).width as usize).map(|b| b + align(b)).sum();
            // Room at the front, or on the free list (decision 0342), for every column's chunk, and the room kept
            // past it.
            let need = need + keep(capacity(nchunks + 1, self.min_log));
            if block.reserved().saturating_sub(block.used()) < need && block.free_bytes() < need {
                return false;
            }
            let mut taken: [usize; 8] = [0; 8];
            for c in 0..self.ncols as usize {
                let d = self.col(c);
                let bytes = rows * d.width as usize;
                // Not zeroed: a value is read only below `len`, written before its row is claimed (0325 step 5).
                let Some(off) = block.try_alloc_chunk_any(bytes, align(bytes)) else {
                    // The chunks this round took go back (to the free list, when the block keeps one).
                    for (c2, &at) in taken.iter().enumerate().take(c.min(8)) {
                        block.free_chunk(at, rows * self.col(c2).width as usize);
                    }
                    return false;
                };
                if c < 8 {
                    taken[c] = off;
                }
                d.chunk[k].store(block.ptr::<u8>(off) as isize as i64 - d as *const ColDir as isize as i64, Relaxed);
            }
            nchunks += 1;
            self.nchunks.store(nchunks, Relaxed);
            self.reserved.store(capacity(nchunks, self.min_log).min(u32::MAX as u64) as u32, Release);
        }
        true
    }

    /// Gives every column's chunks back to `block`'s free list (decision 0342), when it keeps one; the rows are
    /// then none. Returns the bytes given to the OS with them (`Block::free_chunk`).
    ///
    /// # Safety
    /// These rows lie in `block`, and no one reads or writes them after.
    pub unsafe fn release(&self, block: &Block) -> usize {
        if !block.has_free_list() {
            return 0;
        }
        let mut given = 0;
        let n = self.nchunks.load(Relaxed) as usize;
        for c in 0..self.ncols as usize {
            let d = self.col(c);
            for k in 0..n {
                let off = d.chunk[k].load(Relaxed);
                let p = unsafe { (d as *const ColDir as *const u8).offset((off & !SEALED) as isize) };
                let bytes = match d.sealed(k) {
                    Some(sealed) => sealed_bytes(sealed),
                    None => chunk_rows(k, self.min_log) as usize * d.width as usize,
                };
                given += block.free_chunk(block.offset_of(p), bytes);
            }
        }
        self.nchunks.store(0, Relaxed);
        self.reserved.store(0, Release);
        self.len.store(0, Release);
        given
    }

    /// Whether any chunk of column `c` is sealed (decision 0622).
    pub fn is_sealed(&self, c: usize) -> bool {
        let d = self.col(c);
        (0..self.nchunks.load(Relaxed) as usize).any(|k| d.sealed(k).is_some())
    }

    /// Seals every chunk the rows fill (decision 0622): each column's chunk `k` below `len` whole is encoded
    /// (`col::choose`) into one piece of `block` taken from its free list, an `IntCol` with its words after it, and the
    /// raw chunk given back. A chunk the encoding would not make smaller, or one the block has no room for, stays raw.
    /// Returns the bytes the raw chunks held and the bytes the sealed ones hold.
    ///
    /// # Safety
    /// These rows lie in `block`, which keeps a free list; no one writes them after, and no reader holds an address
    /// in them (`ColDir::at`, `rows_at`) across this.
    pub unsafe fn seal(&self, block: &Block) -> (usize, usize) {
        if !block.has_free_list() {
            return (0, 0);
        }
        let n = self.len() as u64;
        let (mut raw, mut sealed) = (0usize, 0usize);
        let mut vals: Vec<i64> = Vec::new();
        for c in 0..self.ncols as usize {
            let d = self.col(c);
            for k in 0..self.nchunks.load(Relaxed) as usize {
                let (first, rows) = (chunk_first(k, self.min_log), chunk_rows(k, self.min_log));
                if first + rows > n || d.sealed(k).is_some() {
                    continue;
                }
                vals.clear();
                // SAFETY: rows below `len` are written.
                vals.extend((first..first + rows).map(|i| unsafe { d.get(self.min_log, i as u32) }));
                // Read at random, by row id: an encoding whose read is a few words (`col::choose_for_reads`), and only
                // where it saves a quarter of the chunk, a sealed read costing more than a raw one.
                let choice = crate::col::choose_for_reads(&vals);
                let bytes = std::mem::size_of::<crate::col::IntCol>() + choice.bytes();
                let held = rows as usize * d.width as usize;
                if bytes * 4 > held * 3 {
                    continue;
                }
                let Some(at) = block.try_alloc_chunk_any(bytes, 64) else { continue };
                // SAFETY: a fresh piece of `bytes` in this block: the descriptor, then its words.
                unsafe {
                    std::ptr::write_bytes(block.ptr::<u8>(at), 0, bytes);
                    let this = block.ptr::<crate::col::IntCol>(at);
                    let words = block.ptr::<u8>(at).add(std::mem::size_of::<crate::col::IntCol>()).cast::<u64>();
                    let (data, runs) = crate::col::IntCol::lay_out_at(this, words, &choice);
                    crate::col::write(&choice, &vals, data, runs);
                }
                let old = d.chunk[k].load(Relaxed);
                let p = unsafe { (d as *const ColDir as *const u8).offset(old as isize) };
                let rel = block.ptr::<u8>(at) as isize as i64 - d as *const ColDir as isize as i64;
                d.chunk[k].store(rel | SEALED, Release);
                block.free_chunk(block.offset_of(p), held);
                raw += held;
                sealed += bytes;
            }
        }
        (raw, sealed)
    }

    /// Claims `n` rows: one atomic add, and chunks made under `lock` only when the claim passes what is reserved.
    pub fn claim(&self, block: &Block, lock: &Mutex<()>, n: u32) -> u32 {
        let _ = block;
        let start = self.len.fetch_add(n, AcqRel);
        let end = start as u64 + n as u64;
        if end > self.reserved_now() as u64 {
            let _g = lock.lock().unwrap();
            if end > self.reserved_now() as u64 {
                self.reserve(block, end);
            }
        }
        start
    }

    /// Claims `n` rows within what is reserved: one atomic add, no chunk made.
    ///
    /// # Safety
    /// The claim ends within `reserved` (the caller reserved its rows first, `try_reserve`).
    #[inline]
    pub unsafe fn claim_reserved(&self, n: u32) -> u32 {
        let start = self.len.fetch_add(n, AcqRel);
        debug_assert!(start as u64 + n as u64 <= self.reserved_now() as u64, "a claim past the rows reserved");
        start
    }

    /// Bytes of values written (not counting chunk room unwritten).
    pub fn value_bytes(&self) -> usize {
        (0..self.ncols as usize).map(|c| self.col(c).width as usize).sum::<usize>() * self.len() as usize
    }

    /// Chunk `k` of column `c`: its first row, its rows in use (of `n`), and where it starts.
    #[inline]
    fn chunk_raw(&self, c: usize, k: usize, n: u64) -> Option<(u64, usize, *mut u8)> {
        let first = chunk_first(k, self.min_log);
        if first >= n || k >= self.nchunks.load(Relaxed) as usize {
            return None;
        }
        let len = (n - first).min(chunk_rows(k, self.min_log)) as usize;
        let d = self.col(c);
        let p = unsafe { (d as *const ColDir as *const u8).offset(d.chunk[k].load(Relaxed) as isize) as *mut u8 };
        Some((first, len, p))
    }

    /// Column `c`'s values as dense slices, one per chunk: a scan reads these. A sealed column has none
    /// ([`Rows::is_sealed`]); its readers read its values.
    pub fn col_chunks_u32(&self, c: usize) -> impl Iterator<Item = &[u32]> + '_ {
        assert_eq!(self.col(c).width, 4);
        assert!(!self.is_sealed(c), "a scan of column {c}'s raw chunks, sealed (decision 0622)");
        assert!(!self.col(c).has_runs(), "a scan of column {c}'s values reads only their low bits");
        let n = self.len() as u64;
        (0..self.nchunks.load(Relaxed) as usize)
            .map_while(move |k| self.chunk_raw(c, k, n))
            .map(|(_, len, p)| unsafe { std::slice::from_raw_parts(p as *const u32, len) })
    }

    /// Column `c`'s first `n` values into `out`, a chunk at a time (no per-row chunk arithmetic).
    pub fn read_col(&self, c: usize, n: u32, out: &mut [i64]) {
        let d = self.col(c);
        let w = d.width;
        let mut k = 0;
        while let Some((first, len, p)) = self.chunk_raw(c, k, n as u64) {
            let o = &mut out[first as usize..first as usize + len];
            if let Some(sealed) = d.sealed(k) {
                let mut j = 0;
                sealed.for_each(|v| {
                    if j < len {
                        o[j] = v;
                    }
                    j += 1;
                });
            } else if w == 4 {
                let s = unsafe { std::slice::from_raw_parts(p as *const u32, len) };
                if d.has_runs() {
                    for (j, (o, &v)) in o.iter_mut().zip(s).enumerate() {
                        // SAFETY: the column's runs are set (`set_runs`).
                        *o = i64::from(unsafe { d.high(first as u32 + j as u32) }) << 32 | i64::from(v);
                    }
                } else {
                    for (o, &v) in o.iter_mut().zip(s) {
                        *o = v as i64;
                    }
                }
            } else {
                o.copy_from_slice(unsafe { std::slice::from_raw_parts(p as *const i64, len) });
            }
            k += 1;
        }
    }

    /// Writes column `c`'s first `vals.len()` rows from `vals`, a chunk at a time. The rows are reserved, and no
    /// one else writes them.
    pub fn write_col(&self, c: usize, vals: &[i64]) {
        let w = self.col(c).width;
        assert!(!self.col(c).has_runs(), "values written into column {c} by their low bits, unchecked");
        let mut k = 0;
        while let Some((first, len, p)) = self.chunk_raw(c, k, vals.len() as u64) {
            let v = &vals[first as usize..first as usize + len];
            if w == 4 {
                let s = unsafe { std::slice::from_raw_parts_mut(p as *mut u32, len) };
                for (d, &x) in s.iter_mut().zip(v) {
                    *d = x as u32;
                }
            } else {
                unsafe { std::slice::from_raw_parts_mut(p as *mut i64, len) }.copy_from_slice(v);
            }
            k += 1;
        }
    }

    /// The first row in `lo..hi` whose column `c` is not less than `key`, the column sorted over that range: a
    /// search of the chunks' first values, then of one chunk's slice.
    pub fn lower_bound(&self, c: usize, lo: u32, hi: u32, key: i64) -> u32 {
        if lo >= hi {
            return lo;
        }
        if self.col(c).has_runs() && (0..1 << 63).contains(&key) {
            return self.lower_bound_in_runs(c, lo, hi, key);
        }
        let ml = self.min_log;
        let r = self.reader(c);
        let (mut ka, _) = chunk_of(lo, ml);
        let (kb, _) = chunk_of(hi - 1, ml);
        // The last chunk in ka..=kb whose first row (clamped to lo) is less than key.
        let first = |k: usize| (chunk_first(k, ml) as u32).max(lo);
        let mut b = kb;
        while ka < b {
            let m = (ka + b).div_ceil(2);
            if r.get(first(m)) < key { ka = m } else { b = m - 1 }
        }
        let k = ka;
        if r.get(first(k)) >= key {
            return first(k);
        }
        let (start, len, p) = self.chunk_raw(c, k, hi as u64).expect("a chunk in range");
        let a = (lo as u64).saturating_sub(start) as usize;
        let at = if let Some(sealed) = self.col(c).sealed(k) {
            sealed.lower_bound(a, len, key)
        } else if self.col(c).has_runs() {
            // A value's high bits are its run's: the search reads each row's value.
            let (mut l, mut h) = (a, len);
            while l < h {
                let m = l + (h - l) / 2;
                if r.get(start as u32 + m as u32) < key { l = m + 1 } else { h = m }
            }
            l
        } else if r.width == 4 {
            let s = unsafe { std::slice::from_raw_parts(p as *const u32, len) };
            a + s[a..].partition_point(|&v| (v as i64) < key)
        } else {
            let s = unsafe { std::slice::from_raw_parts(p as *const i64, len) };
            a + s[a..].partition_point(|&v| v < key)
        };
        start as u32 + at as u32
    }

    /// `lower_bound` of a column under key runs (its values' high bits its runs', a document's key under a node id):
    /// the runs searched for `key`'s high bits once, then the low bits of that run's rows alone. A search by `get`
    /// searched the runs again for every row it read: `text(n, s)` by `n` at about 230 ns a probe of 17 k rows.
    fn lower_bound_in_runs(&self, c: usize, lo: u32, hi: u32, key: i64) -> u32 {
        let d = self.col(c);
        let (kh, kl) = ((key >> 32) as u32, key as u32);
        // SAFETY: the column has runs; run `m`'s first row is at `2 + 2m`, its high bits at `3 + 2m`.
        let (t, count) = unsafe { d.runs_table() };
        let first = |m: u32| unsafe { *t.add(2 + 2 * m as usize) };
        let high = |m: u32| unsafe { *t.add(3 + 2 * m as usize) };
        // The column sorted over `lo..hi`: its runs' high bits nondecreasing there. The first run of `key`'s high bits
        // or above, and the first above them.
        let (mut a, mut b) = (0u32, count);
        while a < b {
            let m = a + (b - a) / 2;
            if high(m) < kh { a = m + 1 } else { b = m }
        }
        let start = if a < count { first(a).max(lo) } else { hi };
        if a == count || high(a) > kh || start >= hi {
            return start.min(hi);
        }
        let mut e = a + 1;
        while e < count && high(e) == kh {
            e += 1;
        }
        let end = if e < count { first(e).min(hi) } else { hi };
        // Rows `start..end` share `key`'s high bits: their low bits ordered, read as they lie.
        self.lower_bound_low(c, start, end, kl, key)
    }

    /// The first row in `lo..hi` (rows of one high bits' run) whose low bits are not below `kl` (`key`'s): the chunks'
    /// first rows, then the chunk's 4-byte slice, or its sealed encoding's search by the whole key.
    fn lower_bound_low(&self, c: usize, lo: u32, hi: u32, kl: u32, key: i64) -> u32 {
        if lo >= hi {
            return lo;
        }
        let (ml, d) = (self.min_log, self.col(c));
        let low = |i: u32| unsafe { d.low(ml, i) };
        let (mut ka, _) = chunk_of(lo, ml);
        let (kb, _) = chunk_of(hi - 1, ml);
        let first = |k: usize| (chunk_first(k, ml) as u32).max(lo);
        let mut b = kb;
        while ka < b {
            let m = (ka + b).div_ceil(2);
            if low(first(m)) < kl { ka = m } else { b = m - 1 }
        }
        let k = ka;
        if low(first(k)) >= kl {
            return first(k);
        }
        let (start, len, p) = self.chunk_raw(c, k, hi as u64).expect("a chunk in range");
        let a = (lo as u64).saturating_sub(start) as usize;
        let at = if let Some(sealed) = d.sealed(k) {
            sealed.lower_bound(a, len, key)
        } else {
            // SAFETY: the chunk's `len` rows of 4 bytes, from `p`.
            let s = unsafe { std::slice::from_raw_parts(p as *const u32, len) };
            a + s[a..].partition_point(|&v| v < kl)
        };
        start as u32 + at as u32
    }

    /// The first row in `lo..hi` whose column `c` is greater than `key`, every row before `lo` being at most
    /// `key`: galloping forward from `lo`, for a key whose rows are few.
    pub fn upper_from(&self, c: usize, lo: u32, hi: u32, key: i64) -> u32 {
        let r = self.reader(c);
        // Rows lo..lo + step are at most key; find a step past which a row is greater.
        let mut step = 1u32;
        let mut a = lo;
        while a < hi && r.get(a) <= key {
            a = lo + step;
            step *= 2;
        }
        // The answer is in (lo + step / 4)..min(a + 1, hi): a binary search there.
        let (mut l, mut h) = (lo + (step / 4).min(hi - lo), a.min(hi));
        while l < h {
            let m = l + (h - l) / 2;
            if r.get(m) <= key { l = m + 1 } else { h = m }
        }
        l
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{Block, kind};

    /// A relation whose last holder went gives its chunks' pages back to the OS (Q76) as they go to the block's free
    /// list (0342), and the relation beside it reads every row it wrote: interleaved chunks of the two never share a
    /// page given back. A relation made after takes the freed chunks again, their pages in use, and reads its rows.
    #[test]
    fn a_dead_relations_pages_go_back_and_its_neighbours_rows_stay() {
        let block = Block::anon(kind::FACTS, 64 << 20).with_free_list();
        let spec = crate::relation::Spec { widths: vec![8, 8], sort: None, indexes: Vec::new(), min_log: MIN_LOG };
        let dead = crate::relation::Relation::create(&block, &spec, None);
        let kept = crate::relation::Relation::create(&block, &spec, None);
        let n = 200_000u32;
        // Grown in turns, so their chunks interleave in the block.
        for step in 1..=8u64 {
            dead.rows.reserve(&block, step * n as u64 / 8);
            kept.rows.reserve(&block, step * n as u64 / 8);
        }
        let fill = |r: &crate::relation::Relation, sign: i64| unsafe {
            r.rows.claim_reserved(n);
            for i in 0..n {
                r.rows.put(i, 0, i64::from(i));
                r.rows.put(i, 1, sign * i64::from(i));
            }
        };
        fill(dead, -1);
        fill(kept, -1);
        let used = block.used();
        let given = unsafe { dead.release(&block) };
        assert!(given > (n as usize * 16) / 2, "{given} bytes given back of {} written", n as usize * 16);
        for i in 0..n {
            assert_eq!((kept.rows.get(i, 0), kept.rows.get(i, 1)), (i64::from(i), -i64::from(i)), "row {i}");
        }
        // The next relation of the same size takes the freed chunks: the block grows by its descriptor alone.
        let next = crate::relation::Relation::create(&block, &spec, None);
        for step in 1..=8u64 {
            next.rows.reserve(&block, step * n as u64 / 8);
        }
        fill(next, 3);
        // A 64 KiB margin for the descriptor. Read as `(used + 64) << 10` it was a thousand times the block, and held
        // of any growth.
        assert!(block.used() < used + (64 << 10), "{} bytes used after, {used} before", block.used());
        for i in 0..n {
            assert_eq!((next.rows.get(i, 0), next.rows.get(i, 1)), (i64::from(i), 3 * i64::from(i)), "row {i}");
            assert_eq!(kept.rows.get(i, 1), -i64::from(i), "row {i}");
        }
    }

    /// Decision 0622: a relation's full chunks sealed in the frozen encodings read as they were through every host
    /// read (`get`, the reader, `read_col`, `lower_bound`, `upper_from`), the chunk past `len` stays raw and is still
    /// written, and the raw chunks' bytes go back to the block's free list.
    #[test]
    fn sealed_chunks_read_as_their_rows_and_give_their_bytes_back() {
        let block = Block::anon(kind::FACTS, 64 << 20).with_free_list();
        let lock = Mutex::new(());
        let rows: &Rows = unsafe { &*block.alloc_t::<Rows>(1) };
        unsafe { Rows::init(rows as *const Rows as *mut Rows, &block, &[8, 8, 4, 8], MIN_LOG) };
        let n = 50_000u32;
        // Sorted with repeats (an order's lead), constant, small, and scattered.
        let col = |c: usize, i: i64| match c {
            0 => i / 3 * 5,
            1 => 7,
            2 => (i * 2_654_435_761) % 1000,
            _ => (i * 2_654_435_761) ^ (i << 40),
        };
        let s = rows.claim(&block, &lock, n);
        for i in 0..n {
            for c in 0..4 {
                unsafe { rows.put(s + i, c, col(c, i64::from(i))) };
            }
        }
        let free_before = block.free_bytes();
        let (raw, sealed) = unsafe { rows.seal(&block) };
        assert!(raw > 0 && sealed < raw, "{sealed} sealed bytes for {raw} raw");
        assert!(block.free_bytes() > free_before, "the raw chunks went back to the free list");
        assert!(rows.is_sealed(0) && rows.is_sealed(1), "the sorted and the constant columns sealed");
        for c in 0..4 {
            let r = rows.reader(c);
            let mut back = vec![0i64; n as usize];
            rows.read_col(c, n, &mut back);
            for i in 0..n {
                let want = col(c, i64::from(i));
                assert_eq!(rows.get(i, c), want, "row {i} of column {c}");
                assert_eq!(r.get(i), want);
                assert_eq!(back[i as usize], want);
            }
        }
        let vals: Vec<i64> = (0..n).map(|i| col(0, i64::from(i))).collect();
        for key in (-3..vals[vals.len() - 1] + 4).step_by(997) {
            for (lo, hi) in [(0, n), (17, n - 5), (4_000, 4_100)] {
                let want_lo = lo + vals[lo as usize..hi as usize].partition_point(|&v| v < key) as u32;
                assert_eq!(rows.lower_bound(0, lo, hi, key), want_lo, "lower {key} in {lo}..{hi}");
                let want_hi = lo.max(want_lo) + vals[want_lo as usize..hi as usize].partition_point(|&v| v <= key) as u32;
                assert_eq!(rows.upper_from(0, want_lo, hi, key), want_hi, "upper {key} in {lo}..{hi}");
            }
        }
        // The last chunk is part full: still raw, still written.
        let more = rows.claim(&block, &lock, 10);
        for i in more..more + 10 {
            unsafe { rows.put(i, 1, 9) };
            assert_eq!(rows.get(i, 1), 9);
        }
        assert_eq!(rows.get(n - 1, 1), 7);
        let given = unsafe { rows.release(&block) };
        let _ = given;
    }

    #[test]
    fn chunks_are_quarter_steps_and_cover_every_row_once() {
        for ml in [MIN_LOG, 6, 10] {
            let mut want = (0usize, 0usize);
            for i in 0..(1u32 << (ml + 6)) {
                let (k, l) = chunk_of(i, ml);
                assert_eq!(chunk_first(k, ml) + l as u64, i as u64, "row {i} at min_log {ml}");
                assert!((l as u64) < chunk_rows(k, ml));
                assert!(k == want.0 && l == want.1 || k == want.0 + 1 && l == 0, "rows in order, chunk by chunk");
                want = (k, l + 1);
            }
            // A doubling group is four chunks of a quarter each.
            assert_eq!(chunk_rows(4, ml) * 4, 1 << (ml + 1));
            assert_eq!(capacity(8, ml), 3 << ml);
        }
    }

    #[test]
    fn searches_agree_with_a_scan() {
        let block = Block::anon(kind::FACTS, 64 << 20);
        let lock = Mutex::new(());
        let rows: &Rows = unsafe { &*block.alloc_t::<Rows>(1) };
        unsafe { Rows::init(rows as *const Rows as *mut Rows, &block, &[4, 8], MIN_LOG) };
        // Sorted, with runs of 1 to 9 equal values.
        let vals: Vec<i64> = (0..20_000).map(|i: i64| i / (1 + i % 9) * 3).scan(0, |m, v| { *m = (*m).max(v); Some(*m) }).collect();
        let n = vals.len() as u32;
        let s = rows.claim(&block, &lock, n);
        for (i, &v) in vals.iter().enumerate() {
            unsafe { rows.put(s + i as u32, 0, v) };
            unsafe { rows.put(s + i as u32, 1, v) };
        }
        for c in 0..2 {
            for key in (-3..vals[vals.len() - 1] + 4).step_by(7) {
                for (lo, hi) in [(0, n), (17, n - 5), (4_000, 4_100)] {
                    let want_lo = lo + vals[lo as usize..hi as usize].partition_point(|&v| v < key) as u32;
                    assert_eq!(rows.lower_bound(c, lo, hi, key), want_lo, "lower {key} in {lo}..{hi}");
                    let want_hi = lo.max(want_lo) + vals[want_lo as usize..hi as usize].partition_point(|&v| v <= key) as u32;
                    assert_eq!(rows.upper_from(c, want_lo, hi, key), want_hi, "upper {key} in {lo}..{hi}");
                }
            }
        }
        let mut back = vec![0i64; vals.len()];
        rows.read_col(1, n, &mut back);
        assert_eq!(back, vals);
    }

    /// A column under key runs (an axis's node ids: each document's key over its nodes' low bits) reads as its
    /// values through every read: a row's, a column resolved once, a chunk at a time and the searches; a value whose
    /// high bits are not its row's run's is refused.
    #[test]
    fn a_columns_key_runs_read_as_its_values() {
        let block = Block::anon(kind::FACTS, 64 << 20).with_free_list();
        let lock = Mutex::new(());
        let rows: &Rows = unsafe { &*block.alloc_t::<Rows>(1) };
        unsafe { Rows::init(rows as *const Rows as *mut Rows, &block, &[4, 4], MIN_LOG) };
        // Documents of uneven rows (one of a single row), keys ascending, past a u32 once shifted.
        let firsts = [0u32, 1, 2_900, 2_901, 7_000, 19_999];
        let keys = [3u32, 9, 10, 4_000_000, 4_000_001, u32::MAX];
        let t = alloc_runs(&block, firsts.len());
        for (k, (&f, &key)) in firsts.iter().zip(&keys).enumerate() {
            unsafe { put_run(t, k, f, key) };
        }
        unsafe { rows.set_runs(0, t) };
        let n = 20_000u32;
        let run = |i: u32| firsts.iter().rposition(|&f| f <= i).unwrap();
        // Within a document, low bits ascending (a node's, nodes in order) with repeats.
        let vals: Vec<i64> = (0..n).map(|i| i64::from(keys[run(i)]) << 32 | i64::from((i - firsts[run(i)]) / 3 * 5)).collect();
        let s = rows.claim(&block, &lock, n);
        for (i, &v) in vals.iter().enumerate() {
            unsafe { rows.put(s + i as u32, 0, v) };
            unsafe { rows.put(s + i as u32, 1, v & 0xffff_ffff) };
        }
        let r = rows.reader(0);
        for i in 0..n {
            assert_eq!((rows.get(i, 0), r.get(i), rows.get(i, 1)), (vals[i as usize], vals[i as usize], vals[i as usize] & 0xffff_ffff), "row {i}");
            assert_eq!(unsafe { rows.col(0).low(rows.min_log, i) }, vals[i as usize] as u32, "row {i}'s low bits");
        }
        let mut back = vec![0i64; vals.len()];
        rows.read_col(0, n, &mut back);
        assert_eq!(back, vals);
        // The searches, the runs searched once and then a run's low bits (`lower_bound_in_runs`), raw and then sealed
        // (a sealed chunk holds whole values, searched by the whole key). Keys between runs, inside a run, past its
        // last low bits, and below and past every row.
        let keys = [vals[0] - 1, vals[0], vals[1], vals[2_899] + 1, vals[2_900] - 1, vals[2_901], vals[12_345], vals[12_345] + 1, vals[19_998] + 2, vals[19_999], 5 << 32, i64::MAX];
        let search = |what: &str| {
            for key in keys {
                for (lo, hi) in [(0, n), (17, n - 5), (4_000, 4_100), (2_899, 2_902), (2_901, 2_901)] {
                    let want_lo = lo + vals[lo as usize..hi as usize].partition_point(|&v| v < key) as u32;
                    assert_eq!(rows.lower_bound(0, lo, hi, key), want_lo, "{what}: lower {key:#x} in {lo}..{hi}");
                    let want_hi = lo.max(want_lo) + vals[want_lo as usize..hi as usize].partition_point(|&v| v <= key) as u32;
                    assert_eq!(rows.upper_from(0, want_lo, hi, key), want_hi, "{what}: upper {key:#x} in {lo}..{hi}");
                }
            }
        };
        search("raw");
        let (_, sealed) = unsafe { rows.seal(&block) };
        assert!(sealed > 0 && (0..rows.nchunks.load(Relaxed) as usize).any(|k| rows.col(0).sealed(k).is_some()), "some chunk of the runs column sealed");
        search("sealed");
        // Row 1's run is key 9's: a value of another document's key is refused.
        let refused = std::panic::catch_unwind(|| unsafe { rows.put(1, 0, 3 << 32 | 7) });
        assert!(refused.is_err(), "a value under another run's high bits");
    }
}
