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
/// The smallest `min_log`: a group's quarter is at least 4 rows.
pub const MIN_LOG: u32 = 4;

structure! {
    /// One column: its width in bytes (4 or 8), and each chunk's offset from this descriptor.
    pub struct ColDir {
        pub width: u32,
        pub pad: u32,
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

/// Row `i`'s chunk and its place in it.
#[inline]
pub fn chunk_of(i: u32, min_log: u32) -> (usize, usize) {
    let q = ((i as u64) >> min_log) + 1;
    let k = 63 - q.leading_zeros();
    let l = i as u64 - (((1u64 << k) - 1) << min_log);
    let qlog = min_log + k - 2;
    (4 * k as usize + (l >> qlog) as usize, (l & ((1u64 << qlog) - 1)) as usize)
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
    /// Row `i`'s value: the read written once (`col_get`, decision 0325), the kernels' and the JIT's too.
    #[inline(always)]
    pub fn get(&self, i: u32) -> i64 {
        // SAFETY: the directory is its relation's, which outlives the reader; row `i` was claimed and reserved.
        unsafe { crate::generated_reads::col_get(self.dir.cast(), self.min_log, i) as i64 }
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

    /// Where row `i`'s value in column `c` lies, and the column's width: the reads written once (`rows_at`,
    /// `rows_width`; decision 0325).
    #[inline]
    fn at(&self, i: u32, c: usize) -> (*mut u8, u32) {
        let desc = std::ptr::from_ref(self).cast::<u8>();
        // SAFETY: a live descriptor of this block; column `c` is one of its columns (the callers' row reserved).
        unsafe { (crate::generated_reads::rows_at(desc, i, c as u32) as usize as *mut u8, crate::generated_reads::rows_width(desc, c as u32)) }
    }

    /// Row `i`'s value in column `c`: the read written once (`rows_get`, decision 0325), the kernels' and the JIT's
    /// too.
    #[inline]
    pub fn get(&self, i: u32, c: usize) -> i64 {
        // SAFETY: a live descriptor; row `i` was claimed and reserved, column `c` is one of its columns.
        unsafe { crate::generated_reads::rows_get(std::ptr::from_ref(self).cast(), i, c as u32) as i64 }
    }

    /// # Safety
    /// Row `i` was claimed by the caller and reserved.
    #[inline]
    pub unsafe fn put(&self, i: u32, c: usize, v: i64) {
        let (p, w) = self.at(i, c);
        unsafe {
            if w == 4 {
                debug_assert!((0..=u32::MAX as i64).contains(&v), "{v} in a 4-byte column");
                *(p as *mut u32) = v as u32
            } else {
                *(p as *mut i64) = v
            }
        }
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
        let mut nchunks = self.nchunks.load(Relaxed);
        while capacity(nchunks, self.min_log) < n {
            let k = nchunks as usize;
            assert!(k < DIRS, "rows past {DIRS} chunks");
            let rows = chunk_rows(k, self.min_log) as usize;
            let align = |bytes: usize| if bytes >= PAGE { PAGE } else { 64 };
            let need: usize = (0..self.ncols as usize).map(|c| rows * self.col(c).width as usize).map(|b| b + align(b)).sum();
            // Room at the front, or on the free list (decision 0342), for every column's chunk.
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

    /// Makes these rows (empty, of `src`'s widths and `min_log`, in open `block`) hold `src`'s: the same chunks made,
    /// each column's chunks copied whole by `memcpy`, nothing decoded. False when `block` has no room (the rows are
    /// then empty).
    pub fn try_copy_from(&self, block: &Block, src: &Rows) -> bool {
        assert!(self.is_empty() && self.ncols == src.ncols && self.min_log == src.min_log, "a copy into rows of another shape");
        let n = src.len();
        if !self.try_reserve(block, u64::from(n)) {
            return false;
        }
        for c in 0..self.ncols as usize {
            let (from, to) = (src.col(c), self.col(c));
            assert_eq!(from.width, to.width);
            let mut k = 0;
            while chunk_first(k, self.min_log) < u64::from(n) {
                let rows = (u64::from(n) - chunk_first(k, self.min_log)).min(chunk_rows(k, self.min_log)) as usize;
                // SAFETY: chunk `k` of both columns is made (`n` is within both reserves) and holds `rows` rows of
                // `width` bytes; they lie in different blocks' memory, and this block is open with this one writer.
                unsafe {
                    let a = (from as *const ColDir as *const u8).offset(from.chunk[k].load(Relaxed) as isize);
                    let b = (to as *const ColDir as *const u8).offset(to.chunk[k].load(Relaxed) as isize) as *mut u8;
                    std::ptr::copy_nonoverlapping(a, b, rows * from.width as usize);
                }
                k += 1;
            }
        }
        self.len.store(n, Release);
        true
    }

    /// Appends rows `lo..hi` of `src` (as many columns, as wide) after these, in open `block`: each column's runs
    /// copied by `memcpy` between the chunks of both, nothing decoded. The one writer of these rows. False when `block`
    /// has no room (the rows are then as they were).
    pub fn try_append_from(&self, block: &Block, src: &Rows, lo: u32, hi: u32) -> bool {
        assert!(self.ncols == src.ncols && lo <= hi && hi <= src.len(), "an append from rows of another shape, or past them");
        let at = self.len();
        let end = at + (hi - lo);
        if !self.try_reserve(block, u64::from(end)) {
            return false;
        }
        for c in 0..self.ncols as usize {
            let (from, to) = (src.col(c), self.col(c));
            assert_eq!(from.width, to.width);
            let w = from.width as usize;
            let (mut i, mut j) = (lo, at);
            while i < hi {
                let ((sk, sp), (dk, dp)) = (chunk_of(i, src.min_log), chunk_of(j, self.min_log));
                let run = (chunk_rows(sk, src.min_log) as usize - sp).min(chunk_rows(dk, self.min_log) as usize - dp).min((hi - i) as usize);
                // SAFETY: chunk `sk` of `src` holds rows `i..i + run` (within its length) and chunk `dk` of these rows
                // `j..j + run` (reserved above); they are different relations' memory, and this one writer's.
                unsafe {
                    let a = (from as *const ColDir as *const u8).offset(from.chunk[sk].load(Relaxed) as isize).add(sp * w);
                    let b = (to as *const ColDir as *const u8).offset(to.chunk[dk].load(Relaxed) as isize).add(dp * w) as *mut u8;
                    // The store's kept block written whole from the run's (0401 F2): the same bytes in the same layout,
                    // a continuation's chain's rows laid end to end. copy: load (its save twin, to the kept file)
                    std::ptr::copy_nonoverlapping(a, b, run * w);
                }
                i += run as u32;
                j += run as u32;
            }
        }
        self.len.store(end, Release);
        true
    }

    /// Each value of column `c` replaced by `f` of it, where it lies, chunk by chunk: the one writer of an open block's
    /// rows (a copy's, mapped to another numbering, 0401 F6).
    ///
    /// # Safety
    /// Nothing else reads or writes these rows meanwhile.
    pub unsafe fn map_col(&self, c: usize, mut f: impl FnMut(i64) -> i64) {
        let w = self.col(c).width;
        let mut k = 0;
        while let Some((_, len, p)) = self.chunk_raw(c, k, u64::from(self.len())) {
            // SAFETY: chunk `k` holds `len` rows of width `w`; the caller's: no one else touches them.
            unsafe {
                if w == 4 {
                    for v in std::slice::from_raw_parts_mut(p.cast::<u32>(), len) {
                        *v = f(i64::from(*v)) as u32;
                    }
                } else {
                    for v in std::slice::from_raw_parts_mut(p.cast::<i64>(), len) {
                        *v = f(*v);
                    }
                }
            }
            k += 1;
        }
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
                let p = unsafe { (d as *const ColDir as *const u8).offset(d.chunk[k].load(Relaxed) as isize) };
                given += block.free_chunk(block.offset_of(p), chunk_rows(k, self.min_log) as usize * d.width as usize);
            }
        }
        self.nchunks.store(0, Relaxed);
        self.reserved.store(0, Release);
        self.len.store(0, Release);
        given
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

    /// Column `c`'s values as dense slices, one per chunk: a scan reads these.
    pub fn col_chunks_u32(&self, c: usize) -> impl Iterator<Item = &[u32]> + '_ {
        assert_eq!(self.col(c).width, 4);
        let n = self.len() as u64;
        (0..self.nchunks.load(Relaxed) as usize)
            .map_while(move |k| self.chunk_raw(c, k, n))
            .map(|(_, len, p)| unsafe { std::slice::from_raw_parts(p as *const u32, len) })
    }

    /// Column `c`'s first `n` values into `out`, a chunk at a time (no per-row chunk arithmetic).
    pub fn read_col(&self, c: usize, n: u32, out: &mut [i64]) {
        let w = self.col(c).width;
        let mut k = 0;
        while let Some((first, len, p)) = self.chunk_raw(c, k, n as u64) {
            let o = &mut out[first as usize..first as usize + len];
            if w == 4 {
                let s = unsafe { std::slice::from_raw_parts(p as *const u32, len) };
                for (d, &v) in o.iter_mut().zip(s) {
                    *d = v as i64;
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
        let at = if r.width == 4 {
            let s = unsafe { std::slice::from_raw_parts(p as *const u32, len) };
            a + s[a..].partition_point(|&v| (v as i64) < key)
        } else {
            let s = unsafe { std::slice::from_raw_parts(p as *const i64, len) };
            a + s[a..].partition_point(|&v| v < key)
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
        let spec = crate::relation::Spec { widths: vec![8, 8], order: Vec::new(), indexes: Vec::new(), min_log: MIN_LOG };
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
        assert!(block.used() < used + 64 << 10, "{} bytes used after, {used} before", block.used());
        for i in 0..n {
            assert_eq!((next.rows.get(i, 0), next.rows.get(i, 1)), (i64::from(i), 3 * i64::from(i)), "row {i}");
            assert_eq!(kept.rows.get(i, 1), -i64::from(i), "row {i}");
        }
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
}
