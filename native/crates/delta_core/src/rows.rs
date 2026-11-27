//! A relation's rows' format and reads (moved from `rerail_blocks::rows`, whose writers make their chunks and seal
//! them).
//!
//! Rows: a relation's columns, each in chunks that grow by quarter steps. Rows come in doubling groups: group `k`
//! holds `2^(min_log + k)` rows, so row `i` is in group `63 - clz((i >> min_log) + 1)`, and a group is four
//! chunks of a quarter of it each. A relation's room unwritten is then at most a quarter of its last group (an
//! eighth of its rows), where whole doubling chunks left up to half. Arithmetic, the same on a GPU. Every
//! column's chunk `s` covers the same rows, so one claim covers a whole row.
//!
//! Appending: a writer claims rows with one atomic add on `len`. Chunks are made by `reserve` (under the
//! relation's lock on the host; between passes for a GPU), never while a kernel writes.

use crate::flat::RelSpan;
use crate::structure;
use core::sync::atomic::{AtomicI64, AtomicU32, Ordering::*};

/// Chunks a column may have: four a doubling group, 24 groups (2^(min_log + 24) rows; 2^28 at the smallest
/// `min_log`, which is 4).
pub const DIRS: usize = 96;

/// A chunk's offset with this bit set is a sealed chunk's (decision 0622): the offset (less the bit) is that of an
/// `IntCol`, the chunk's values in the frozen columns' encoding, its words after it in one piece of the block. A raw
/// chunk's offset is a multiple of 64, so the bit is free.
pub const SEALED: i64 = 1;
/// The smallest `min_log`: a group's quarter is at least 4 rows.
pub const MIN_LOG: u32 = 4;

/// A 4-byte column's `runs` when its values are signed: their high 32 bits are their low bits' sign (the CLIF's
/// `col_get` sign-extends them). A run table is never this far from its directory (`set_runs`).
pub const SIGNED: i32 = i32::MIN;

/// A spec's width for a column of signed 32-bit values (`Rows::init_in`): 4 bytes, `SIGNED`. A width of 4 is a
/// column of unsigned 32-bit values (ids, places, symbols); 8, of 64-bit values.
pub const I32: u32 = 4 | 1 << 16;

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
        pub cols: RelSpan<ColDir>,
    }
}

/// The bytes a sealed chunk's piece holds: its descriptor and its words (`Rows::seal`).
pub fn sealed_bytes(c: &crate::col::IntCol) -> usize {
    core::mem::size_of::<crate::col::IntCol>() + c.bytes()
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
        let start = core::ptr::from_ref(self).cast::<u8>().wrapping_offset(self.chunk[chunk].load(Relaxed) as isize);
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
                let raw = *p.cast::<u32>();
                let low = i64::from(raw);
                if self.runs == 0 {
                    low
                } else if self.runs == SIGNED {
                    i64::from(raw as i32)
                } else {
                    i64::from(self.high(i)) << 32 | low
                }
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
            let t = core::ptr::from_ref(self).cast::<u64>().offset(self.runs as isize).cast::<u32>();
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
    pub unsafe fn runs_table(&self) -> (*const u32, u32) {
        // SAFETY: the caller's, as `high`'s.
        unsafe {
            let t = core::ptr::from_ref(self).cast::<u64>().offset(self.runs as isize).cast::<u32>();
            (t, *t)
        }
    }

    /// Chunk `k`'s values in the frozen encoding, where it is sealed (decision 0622).
    #[inline(always)]
    pub fn sealed(&self, k: usize) -> Option<&crate::col::IntCol> {
        let off = self.chunk[k].load(Relaxed);
        // SAFETY: a sealed chunk's offset (less its tag) is its `IntCol`'s from this directory, in the same block.
        (off & SEALED != 0).then(|| unsafe { &*core::ptr::from_ref(self).cast::<u8>().offset((off & !SEALED) as isize).cast::<crate::col::IntCol>() })
    }

    /// Whether the column holds low bits under key runs.
    #[inline(always)]
    pub fn has_runs(&self) -> bool {
        self.runs != 0 && self.runs != SIGNED
    }

    /// Whether the column holds signed 32-bit values (`I32`).
    #[inline(always)]
    pub fn is_signed(&self) -> bool {
        self.runs == SIGNED
    }
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

    /// Rows `lo..hi`' values in turn, as `get` reads each: a chunk's place found once, then its values where they
    /// lie, one after another (a column with runs reads each row's high bits, as `get` does).
    #[inline(always)]
    pub fn each(&self, lo: u32, hi: u32, mut f: impl FnMut(i64)) {
        // SAFETY: as `get`'s: rows `lo..hi` were claimed and reserved, and a chunk's rows lie in it one after another
        // at the column's width (`ColDir::at`).
        let d = unsafe { &*self.dir };
        if d.has_runs() {
            for i in lo..hi {
                f(self.get(i));
            }
            return;
        }
        let signed = d.runs == SIGNED;
        let mut i = lo;
        while i < hi {
            let (chunk, place) = chunk_of(i, self.min_log);
            let n = ((chunk_rows(chunk, self.min_log) - place as u64).min(u64::from(hi - i))) as usize;
            // A sealed chunk's values by its encoding, as `get` reads them (decision 0622).
            if let Some(sealed) = d.sealed(chunk) {
                (place..place + n).for_each(|k| f(sealed.get(k)));
                i += n as u32;
                continue;
            }
            let p = d.at(self.min_log, i);
            // SAFETY: as above: `n` rows of the chunk from row `i`, at the column's width.
            unsafe {
                if self.width == 4 && signed {
                    core::slice::from_raw_parts(p.cast::<i32>(), n).iter().for_each(|&v| f(i64::from(v)));
                } else if self.width == 4 {
                    core::slice::from_raw_parts(p.cast::<u32>(), n).iter().for_each(|&v| f(i64::from(v)));
                } else {
                    core::slice::from_raw_parts(p.cast::<i64>(), n).iter().for_each(|&v| f(v));
                }
            }
            i += n as u32;
        }
    }
}

impl Rows {
    /// Column `c`, resolved for many reads.
    #[inline]
    pub fn reader(&self, c: usize) -> ColRead {
        let d = self.col(c);
        ColRead { dir: d, width: d.width, min_log: self.min_log }
    }

    /// `init` with the columns' directories at `cols` (zeroed, `widths.len()` of them, in `region`, its block, as it
    /// is).
    ///
    /// # Safety
    /// As `init`, and `cols` as said.
    pub unsafe fn init_in(this: *mut Rows, region: &crate::flat::Region<'_>, cols: *mut ColDir, widths: &[u32], min_log: u32) {
        assert!(min_log >= MIN_LOG, "min_log {min_log} under {MIN_LOG}");
        unsafe {
            for (c, &w) in widths.iter().enumerate() {
                assert!(w == 4 || w == 8 || w == I32, "a width of 4, 8 or `I32`");
                (*cols.add(c)).width = w & 0xff;
                (*cols.add(c)).runs = if w == I32 { SIGNED } else { 0 };
            }
            (*this).min_log = min_log;
            (*this).ncols = widths.len() as u32;
            (*this).cols.set(region, cols, widths.len()).expect("a relation's column directories in its block");
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
    pub fn reserved_now(&self) -> u32 {
        self.reserved.load(Acquire)
    }

    #[inline]
    pub fn col(&self, c: usize) -> &ColDir {
        &self.cols.as_slice()[c]
    }

    /// Where row `i`'s value in column `c` lies, and the column's width (the CLIF's `rows_at`, `rows_width`).
    #[inline]
    pub fn at(&self, i: u32, c: usize) -> (*mut u8, u32) {
        let d = self.col(c);
        (d.at(self.min_log, i), d.width)
    }

    /// Row `i`'s value in column `c` (the CLIF's `rows_get`, decision 0325, the kernels' and the JIT's).
    #[inline]
    pub fn get(&self, i: u32, c: usize) -> i64 {
        // SAFETY: row `i` was claimed and reserved.
        unsafe { self.col(c).get(self.min_log, i) }
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
        assert!(!d.is_signed(), "runs over a column of signed values");
        assert!(self.is_empty(), "runs given before the column's rows");
        let off = runs as isize - core::ptr::from_ref(d) as isize;
        assert!(off % 8 == 0 && off != 0, "a run table at a word from the column's directory");
        let words = i32::try_from(off / 8).ok().filter(|&w| w != SIGNED).expect("a run table within 16 GB of the column's directory");
        // SAFETY: the directory's one writer, before any reader (asserted empty).
        unsafe { (*core::ptr::from_ref(d).cast_mut()).runs = words };
    }

    /// Whether any chunk of column `c` is sealed (decision 0622).
    pub fn is_sealed(&self, c: usize) -> bool {
        let d = self.col(c);
        (0..self.nchunks.load(Relaxed) as usize).any(|k| d.sealed(k).is_some())
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
        assert!(!self.col(c).has_runs() && !self.col(c).is_signed(), "a scan of column {c}'s values as unsigned 32-bit ones");
        let n = self.len() as u64;
        (0..self.nchunks.load(Relaxed) as usize)
            .map_while(move |k| self.chunk_raw(c, k, n))
            .map(|(_, len, p)| unsafe { core::slice::from_raw_parts(p as *const u32, len) })
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
                let s = unsafe { core::slice::from_raw_parts(p as *const u32, len) };
                if d.has_runs() {
                    for (j, (o, &v)) in o.iter_mut().zip(s).enumerate() {
                        // SAFETY: the column's runs are set (`set_runs`).
                        *o = i64::from(unsafe { d.high(first as u32 + j as u32) }) << 32 | i64::from(v);
                    }
                } else if d.is_signed() {
                    for (o, &v) in o.iter_mut().zip(s) {
                        *o = i64::from(v as i32);
                    }
                } else {
                    for (o, &v) in o.iter_mut().zip(s) {
                        *o = v as i64;
                    }
                }
            } else {
                o.copy_from_slice(unsafe { core::slice::from_raw_parts(p as *const i64, len) });
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
                let s = unsafe { core::slice::from_raw_parts_mut(p as *mut u32, len) };
                for (d, &x) in s.iter_mut().zip(v) {
                    *d = x as u32;
                }
            } else {
                unsafe { core::slice::from_raw_parts_mut(p as *mut i64, len) }.copy_from_slice(v);
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
        if self.col(c).has_runs() {
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
        } else if r.width == 4 && self.col(c).is_signed() {
            let s = unsafe { core::slice::from_raw_parts(p as *const u32, len) };
            a + s[a..].partition_point(|&v| i64::from(v as i32) < key)
        } else if r.width == 4 {
            let s = unsafe { core::slice::from_raw_parts(p as *const u32, len) };
            a + s[a..].partition_point(|&v| (v as i64) < key)
        } else {
            let s = unsafe { core::slice::from_raw_parts(p as *const i64, len) };
            a + s[a..].partition_point(|&v| v < key)
        };
        start as u32 + at as u32
    }

    /// `lower_bound` of a column under key runs (its values' high bits its runs', a document's key under a node id):
    /// the runs searched for `key`'s high bits once, then the low bits of that run's rows alone. A search by `get`
    /// searched the runs again for every row it read: `text(n, s)` by `n` at about 230 ns a probe of 17 k rows.
    fn lower_bound_in_runs(&self, c: usize, lo: u32, hi: u32, key: i64) -> u32 {
        let d = self.col(c);
        // The column's order is the values' (i64): runs by their high bits signed, then low bits unsigned.
        let (kh, kl) = ((key >> 32) as i32, key as u32);
        // SAFETY: the column has runs; run `m`'s first row is at `2 + 2m`, its high bits at `3 + 2m`.
        let (t, count) = unsafe { d.runs_table() };
        let first = |m: u32| unsafe { *t.add(2 + 2 * m as usize) };
        let high = |m: u32| unsafe { *t.add(3 + 2 * m as usize) } as i32;
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
            let s = unsafe { core::slice::from_raw_parts(p as *const u32, len) };
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
