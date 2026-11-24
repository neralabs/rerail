//! The prover's formulas where both its solvers read them (decision 0670, step 3): a window's gates, its fanout and
//! its plain clauses, and a solve's own, as chunks of one block a compile's provers share (`FormulaBlock`). The CPU's
//! solver reads them there, and a batch's kernel binds the block once and reads each formula at its offsets: nothing
//! is uploaded (decision 0353).
//!
//! `Grown` is an array that grows as a formula is built: on the heap (a solver's own working memory, a test's), or
//! in a block, a chunk of its free list, taken again twice as large as it fills (the old chunk given back), as a
//! `Vec` grows. A formula's arrays do not move once it is built: a solve adds its own after them and takes them off
//! again (`truncate`), within the chunk.

use rerail_blocks::block::{kind, Block};
use std::sync::Arc;

/// The block a compile's provers build their formulas in: address space reserved, nothing resident until written;
/// chunks given back are taken again (its free list).
#[derive(Clone)]
pub struct FormulaBlock(Arc<Block>);

/// Its reservation: address space only (`Block::anon`). A kernel reads it by 32-bit word offsets: 16 GiB at most.
const RESERVED: usize = 16 << 30;

impl FormulaBlock {
    pub fn new() -> FormulaBlock {
        FormulaBlock(Arc::new(Block::anon(kind::SCRATCH, RESERVED).with_free_list()))
    }

    pub fn block(&self) -> &Block {
        &self.0
    }

    /// Whether `other` is this block.
    pub fn same(&self, other: &FormulaBlock) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Default for FormulaBlock {
    fn default() -> FormulaBlock {
        FormulaBlock::new()
    }
}

impl std::fmt::Debug for FormulaBlock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FormulaBlock({} bytes used)", self.0.used())
    }
}

/// A chunk of a block: its offset and the values it holds room for.
struct Chunk {
    block: FormulaBlock,
    at: usize,
    cap: usize,
}

/// An array that grows: on the heap, or in a `FormulaBlock`.
pub struct Grown<T: Copy> {
    heap: Vec<T>,
    chunk: Option<Chunk>,
    len: usize,
    /// Its first value, wherever it is (set as it grows): a read is a slice, with no test of where.
    ptr: *mut T,
}

impl<T: Copy> Default for Grown<T> {
    fn default() -> Self {
        let mut heap = Vec::new();
        let ptr = heap.as_mut_ptr();
        Grown { heap, chunk: None, len: 0, ptr }
    }
}

impl<T: Copy> Grown<T> {
    /// Empty, in `block`: its first push takes a chunk.
    pub fn in_block(block: &FormulaBlock) -> Self {
        let mut g = Grown::default();
        g.chunk = Some(Chunk { block: block.clone(), at: 0, cap: 0 });
        g
    }

    /// Empty, where `other` is: in its block, or on the heap.
    pub fn like<U: Copy>(other: &Grown<U>) -> Self {
        match other.placed() {
            Some((b, _)) => Grown::in_block(b),
            None => Grown::default(),
        }
    }

    /// `n` values long, those past its length `v`.
    pub fn resize(&mut self, n: usize, v: T) {
        if n <= self.len {
            self.truncate(n);
            return;
        }
        self.reserve(n - self.len);
        while self.len < n {
            self.push(v);
        }
    }

    /// The block it is in, and its byte offset there (`None` on the heap).
    pub fn placed(&self) -> Option<(&FormulaBlock, usize)> {
        self.chunk.as_ref().map(|c| (&c.block, c.at))
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Room for `more` values past its length.
    fn reserve(&mut self, more: usize) {
        let Some(c) = &mut self.chunk else {
            self.heap.reserve(more);
            self.ptr = self.heap.as_mut_ptr();
            return;
        };
        let need = self.len + more;
        if need <= c.cap {
            return;
        }
        let cap = need.max(c.cap * 2).max(64 / size_of::<T>().max(1));
        let bytes = cap * size_of::<T>();
        let block = c.block.block();
        let at = block.try_alloc_chunk_any(bytes, 16).unwrap_or_else(|| panic!("the prover's formula block is full: {bytes} bytes more"));
        // The class it was given: a power of two of at least 64 bytes, all of it the array's.
        let cap = bytes.next_power_of_two().max(64) / size_of::<T>();
        if c.cap > 0 {
            // SAFETY: both chunks are the block's, live, and apart; the old holds `len` values.
            unsafe { std::ptr::copy_nonoverlapping(block.ptr::<T>(c.at), block.ptr::<T>(at), self.len) };
            block.free_chunk(c.at, c.cap * size_of::<T>());
        }
        (c.at, c.cap) = (at, cap);
        self.ptr = block.ptr::<T>(at);
    }

    pub fn push(&mut self, v: T) {
        if self.chunk.is_none() {
            self.heap.push(v);
            self.ptr = self.heap.as_mut_ptr();
        } else {
            self.reserve(1);
            // SAFETY: the chunk has room for `len + 1` values.
            unsafe { self.ptr.add(self.len).write(v) };
        }
        self.len += 1;
    }

    pub fn extend_from_slice(&mut self, vs: &[T]) {
        if self.chunk.is_none() {
            self.heap.extend_from_slice(vs);
            self.ptr = self.heap.as_mut_ptr();
        } else {
            self.reserve(vs.len());
            // SAFETY: the chunk has room for `len + vs.len()` values, apart from `vs`' (the block is written here only).
            unsafe { std::ptr::copy_nonoverlapping(vs.as_ptr(), self.ptr.add(self.len), vs.len()) };
        }
        self.len += vs.len();
    }

    pub fn truncate(&mut self, n: usize) {
        if n < self.len {
            self.heap.truncate(n);
            self.len = n;
        }
    }

    pub fn clear(&mut self) {
        self.truncate(0);
    }
}

impl<T: Copy> std::ops::Deref for Grown<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        // SAFETY: `ptr` is its first value (dangling and aligned while it has none), and `len` values follow, written
        // by this array.
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
}

impl<T: Copy> std::ops::DerefMut for Grown<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        // SAFETY: as `deref`; `&mut self` is its one writer.
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) }
    }
}

impl<T: Copy> Drop for Grown<T> {
    fn drop(&mut self) {
        if let Some(c) = &self.chunk
            && c.cap > 0
        {
            c.block.block().free_chunk(c.at, c.cap * size_of::<T>());
        }
    }
}

/// A clone in the same place: the same block, its own chunk.
impl<T: Copy> Clone for Grown<T> {
    fn clone(&self) -> Self {
        let mut g = match &self.chunk {
            Some(c) => Grown::in_block(&c.block),
            None => Grown::default(),
        };
        g.extend_from_slice(self);
        g
    }
}

impl<T: Copy + std::fmt::Debug> std::fmt::Debug for Grown<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl<T: Copy + PartialEq> PartialEq for Grown<T> {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

// SAFETY: a chunk is written through `&mut self` alone; the block's allocation is thread-safe.
unsafe impl<T: Copy + Send> Send for Grown<T> {}
unsafe impl<T: Copy + Sync> Sync for Grown<T> {}

#[cfg(test)]
mod tests {
    use super::*;

    /// An array in a block holds what was pushed through its growth, a truncation and a clone, as one on the heap.
    #[test]
    fn a_grown_array_in_a_block_is_the_heaps() {
        let block = FormulaBlock::new();
        let (mut a, mut h) = (Grown::<u32>::in_block(&block), Grown::<u32>::default());
        for i in 0..10_000u32 {
            a.push(i * 7);
            h.push(i * 7);
            if i % 1000 == 999 {
                a.extend_from_slice(&[1, 2, 3]);
                h.extend_from_slice(&[1, 2, 3]);
            }
        }
        assert_eq!(&*a, &*h);
        a.truncate(5000);
        h.truncate(5000);
        a[3] = 9;
        h[3] = 9;
        assert_eq!(a, h);
        let b = a.clone();
        drop(a);
        assert_eq!(&*b, &*h);
        assert!(b.placed().is_some_and(|(x, _)| x.same(&block)));
    }
}
