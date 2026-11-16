//! Rows: a relation's columns, each in chunks that double. Chunk `k` holds `2^(min_log + k)` rows, so row `i` is
//! in chunk `63 - clz((i >> min_log) + 1)`: arithmetic, the same on a GPU. Every column's chunk `k` covers the
//! same rows, so one claim covers a whole row.
//!
//! Appending: a writer claims rows with one atomic add on `len`. Chunks are made by `reserve` (under the
//! relation's lock on the host; between passes for a GPU), never while a kernel writes.

use crate::block::{Block, PAGE};
use crate::refs::Span;
use crate::structure;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, AtomicU32, Ordering::*};

pub const DIRS: usize = 32;

structure! {
    /// One column: its width in bytes (4 or 8), and each chunk's offset from this descriptor.
    pub struct ColDir {
        pub width: u32,
        pub pad: u32,
        pub chunk: [AtomicI64; 32],
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

#[inline]
pub fn chunk_of(i: u32, min_log: u32) -> (usize, usize) {
    let q = ((i as u64) >> min_log) + 1;
    let k = 63 - q.leading_zeros();
    let first = ((1u64 << k) - 1) << min_log;
    (k as usize, (i as u64 - first) as usize)
}

#[inline]
pub fn capacity(nchunks: u32, min_log: u32) -> u64 {
    ((1u64 << nchunks) - 1) << min_log
}

impl Rows {
    /// # Safety
    /// `this` is zeroed memory in `block`, which is open.
    pub unsafe fn init(this: *mut Rows, block: &Block, widths: &[u32], min_log: u32) {
        let cols: *mut ColDir = block.alloc_t(widths.len());
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

    pub fn len(&self) -> u32 {
        self.len.load(Acquire)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn reserved_now(&self) -> u32 {
        self.reserved.load(Acquire)
    }

    pub fn col(&self, c: usize) -> &ColDir {
        &self.cols.as_slice()[c]
    }

    #[inline]
    fn at(&self, i: u32, c: usize) -> (*mut u8, u32) {
        let (k, l) = chunk_of(i, self.min_log);
        let d = self.col(c);
        let p = unsafe { (d as *const ColDir as *const u8).offset(d.chunk[k].load(Relaxed) as isize) as *mut u8 };
        (unsafe { p.add(l * d.width as usize) }, d.width)
    }

    #[inline]
    pub fn get(&self, i: u32, c: usize) -> i64 {
        let (p, w) = self.at(i, c);
        unsafe { if w == 4 { *(p as *const u32) as i64 } else { *(p as *const i64) } }
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
        let mut nchunks = self.nchunks.load(Relaxed);
        while capacity(nchunks, self.min_log) < n {
            let k = nchunks as usize;
            assert!(k < DIRS, "rows past 32 chunks");
            let rows = 1usize << (self.min_log as usize + k);
            for c in 0..self.ncols as usize {
                let d = self.col(c);
                let bytes = rows * d.width as usize;
                let off = block.alloc(bytes, if bytes >= PAGE { PAGE } else { 64 });
                d.chunk[k].store(block.ptr::<u8>(off) as isize as i64 - d as *const ColDir as isize as i64, Relaxed);
            }
            nchunks += 1;
        }
        self.nchunks.store(nchunks, Relaxed);
        self.reserved.store(capacity(nchunks, self.min_log).min(u32::MAX as u64) as u32, Release);
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

    /// Bytes of values written (not counting chunk room unwritten).
    pub fn value_bytes(&self) -> usize {
        (0..self.ncols as usize).map(|c| self.col(c).width as usize).sum::<usize>() * self.len() as usize
    }

    /// Column `c`'s values as dense slices, one per chunk: a scan reads these.
    pub fn col_chunks_u32(&self, c: usize) -> impl Iterator<Item = &[u32]> + '_ {
        let d = self.col(c);
        assert_eq!(d.width, 4);
        let n = self.len() as u64;
        (0..self.nchunks.load(Relaxed) as usize).filter_map(move |k| {
            let first = ((1u64 << k) - 1) << self.min_log;
            if first >= n {
                return None;
            }
            let cap = 1u64 << (self.min_log as usize + k);
            let len = (n - first).min(cap) as usize;
            let p = unsafe { (d as *const ColDir as *const u8).offset(d.chunk[k].load(Relaxed) as isize) as *const u32 };
            Some(unsafe { std::slice::from_raw_parts(p, len) })
        })
    }
}
