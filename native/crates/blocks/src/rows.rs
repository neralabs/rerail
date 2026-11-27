//! Rows: a relation's columns, each in chunks that grow by quarter steps. Rows come in doubling groups: group `k`
//! holds `2^(min_log + k)` rows, so row `i` is in group `63 - clz((i >> min_log) + 1)`, and a group is four
//! chunks of a quarter of it each. A relation's room unwritten is then at most a quarter of its last group (an
//! eighth of its rows), where whole doubling chunks left up to half. Arithmetic, the same on a GPU. Every
//! column's chunk `s` covers the same rows, so one claim covers a whole row.
//!
//! Appending: a writer claims rows with one atomic add on `len`. Chunks are made by `reserve` (under the
//! relation's lock on the host; between passes for a GPU), never while a kernel writes.

use crate::block::{Block, PAGE};
use std::sync::Mutex;
use std::sync::atomic::Ordering::*;
// The format and its reads: the floor's one definition (`rerail_delta_core::rows`).
pub use rerail_delta_core::rows::{ColDir, ColRead, DIRS, I32, MIN_LOG, Rows, SEALED, SIGNED, capacity, chunk_first, chunk_of, chunk_rows, put_run};
use rerail_delta_core::rows::sealed_bytes;


/// A run table (`ColDir::runs`) of `n` runs in `block`, every run's first row 0 and high bits 0 until set
/// (`put_run`): 8 bytes a run past its count.
pub fn alloc_runs(block: &Block, n: usize) -> *mut u32 {
    let t: *mut u32 = block.alloc_t((2 + 2 * n).max(4));
    // SAFETY: just allocated, zeroed, for the count and `n` runs.
    unsafe { *t = u32::try_from(n).expect("runs in a u32") };
    t
}

/// The rows' writes that take room from a block (its chunks, their sealed pieces), a lock, or a message: blocks' own,
/// over the floor's `Rows`.
pub trait RowsW {
    /// # Safety
    /// `this` is zeroed memory in `block`, which is open.
    unsafe fn init(this: *mut Rows, block: &Block, widths: &[u32], min_log: u32);

    /// `init`, or false when the block cannot hold the columns' directories.
    ///
    /// # Safety
    /// As `init`.
    unsafe fn try_init(this: *mut Rows, block: &Block, widths: &[u32], min_log: u32) -> bool;

    /// # Safety
    /// Row `i` was claimed by the caller and reserved.
    unsafe fn put(&self, i: u32, c: usize, v: i64);

    /// Makes chunks until `n` rows fit. The caller holds the relation's lock, or no kernel is running.
    fn reserve(&self, block: &Block, n: u64);

    /// `reserve`, or false when the block cannot hold the chunks: the rows reserved are then as they were (a chunk
    /// counts once every column has its room). Each chunk's room is checked for every column before any is
    /// allocated; a writer of another relation in the block can still take it between, and then the columns'
    /// room already taken is left unused.
    fn try_reserve(&self, block: &Block, n: u64) -> bool;

    /// `try_reserve`, leaving `keep(rows)` bytes of the block free past the chunks for `rows` rows in all: room the
    /// relation's indexes take later (decision 0353 step 6), which its own rows' growth does not use up. False when
    /// a chunk would take it.
    fn try_reserve_keeping(&self, block: &Block, n: u64, keep: impl Fn(u64) -> usize) -> bool;

    /// Gives every column's chunks back to `block`'s free list (decision 0342), when it keeps one; the rows are
    /// then none. Returns the bytes given to the OS with them (`Block::free_chunk`).
    ///
    /// # Safety
    /// These rows lie in `block`, and no one reads or writes them after.
    unsafe fn release(&self, block: &Block) -> usize;

    /// Seals every chunk the rows fill (decision 0622): each column's chunk `k` below `len` whole is encoded
    /// (`col::choose`) into one piece of `block` taken from its free list, an `IntCol` with its words after it, and the
    /// raw chunk given back. A chunk the encoding would not make smaller, or one the block has no room for, stays raw.
    /// Returns the bytes the raw chunks held and the bytes the sealed ones hold.
    ///
    /// # Safety
    /// These rows lie in `block`, which keeps a free list; no one writes them after, and no reader holds an address
    /// in them (`ColDir::at`, `rows_at`) across this.
    unsafe fn seal(&self, block: &Block) -> (usize, usize);

    /// Claims `n` rows: one atomic add, and chunks made under `lock` only when the claim passes what is reserved.
    fn claim(&self, block: &Block, lock: &Mutex<()>, n: u32) -> u32;
}

impl RowsW for Rows {
    unsafe fn init(this: *mut Rows, block: &Block, widths: &[u32], min_log: u32) {
        assert!(unsafe { Rows::try_init(this, block, widths, min_log) }, "block full: {} columns' directories", widths.len());
    }

    unsafe fn try_init(this: *mut Rows, block: &Block, widths: &[u32], min_log: u32) -> bool {
        let Some(cols) = block.try_alloc_t::<ColDir>(widths.len()) else { return false };
        unsafe { Rows::init_in(this, cols, widths, min_log) };
        true
    }

    #[inline]
    unsafe fn put(&self, i: u32, c: usize, v: i64) {
        let (p, w) = self.at(i, c);
        unsafe {
            if w == 4 {
                // A column with runs holds a value's low bits, its high bits its row's run's.
                let d = self.col(c);
                let fits = if d.is_signed() {
                    i32::try_from(v).is_ok()
                } else if d.has_runs() {
                    (v as u64 >> 32) as u32 == d.high(i)
                } else {
                    v >> 32 == 0
                };
                assert!(fits, "{v} in 4-byte column {c}: {}", if d.is_signed() { "past 32 signed bits".to_string() } else { format!("not its row {i}'s high bits") });
                *(p as *mut u32) = v as u32
            } else {
                *(p as *mut i64) = v
            }
        }
    }

    fn reserve(&self, block: &Block, n: u64) {
        assert!(self.try_reserve(block, n), "block full: rows to {n} past {} of {}", block.used(), block.reserved());
    }

    fn try_reserve(&self, block: &Block, n: u64) -> bool {
        self.try_reserve_keeping(block, n, |_| 0)
    }

    fn try_reserve_keeping(&self, block: &Block, n: u64, keep: impl Fn(u64) -> usize) -> bool {
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

    unsafe fn release(&self, block: &Block) -> usize {
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

    unsafe fn seal(&self, block: &Block) -> (usize, usize) {
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
                    let (data, runs) = <crate::col::IntCol as crate::col::IntColW>::lay_out_at(this, words, &choice);
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

    fn claim(&self, block: &Block, lock: &Mutex<()>, n: u32) -> u32 {
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

    /// A column of signed 32-bit values (`I32`) reads as its values through every read (a row's, a column resolved
    /// once, a chunk at a time, the searches), and refuses a value past 32 signed bits; a 4-byte column beside it
    /// stays unsigned.
    #[test]
    fn a_signed_columns_values_read_as_written() {
        let block = Block::anon(kind::FACTS, 64 << 20);
        let lock = Mutex::new(());
        let rows: &Rows = unsafe { &*block.alloc_t::<Rows>(1) };
        unsafe { Rows::init(rows as *const Rows as *mut Rows, &block, &[I32, 4], MIN_LOG) };
        assert!(rows.col(0).is_signed() && !rows.col(0).has_runs() && !rows.col(1).is_signed());
        let vals: Vec<i64> = (0..5_000i64).map(|i| i64::from(i32::MIN) + i * 858_993).collect();
        let n = vals.len() as u32;
        let s = rows.claim(&block, &lock, n);
        for (i, &v) in vals.iter().enumerate() {
            unsafe { rows.put(s + i as u32, 0, v) };
            unsafe { rows.put(s + i as u32, 1, u32::MAX.into()) };
        }
        let r = rows.reader(0);
        for i in 0..n {
            assert_eq!((rows.get(i, 0), r.get(i), rows.get(i, 1)), (vals[i as usize], vals[i as usize], i64::from(u32::MAX)), "row {i}");
        }
        let mut back = vec![0i64; vals.len()];
        rows.read_col(0, n, &mut back);
        assert_eq!(back, vals);
        for key in [i64::from(i32::MIN) - 1, i64::from(i32::MIN), -1, 0, vals[1234], vals[1234] + 1, i64::from(i32::MAX)] {
            assert_eq!(rows.lower_bound(0, 0, n, key), vals.partition_point(|&v| v < key) as u32, "lower {key}");
        }
        let past = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe { rows.put(s, 0, i64::from(i32::MAX) + 1) }));
        assert!(past.is_err(), "a value past 32 signed bits is refused");
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
        // Documents of uneven rows (one of a single row), keys ascending as the values are (i64): one whose high bits
        // have their top bit set (a negative value) first, then keys past a u32 once shifted.
        let firsts = [0u32, 1, 2_900, 2_901, 7_000, 19_999];
        let keys = [u32::MAX, 3, 9, 10, 4_000_000, 4_000_001];
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
        // Row 1's run is key 3's: a value of another document's key is refused.
        let refused = std::panic::catch_unwind(|| unsafe { rows.put(1, 0, 9 << 32 | 7) });
        assert!(refused.is_err(), "a value under another run's high bits");
    }
}
