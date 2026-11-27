//! An operation's own memory (decision 0687): the thread's bump frame (`op`, `Frame`, `FrameVec`) and the stack's short
//! arrays (`InlineVec`).
//!
//! An operation (a join step, a group's wave, a document's parse: what becomes one UKL op) reads its inputs in their
//! blocks and writes its outputs there; what it makes for itself lives only while it runs, and no reader outside it
//! reads it. That memory is here, and pays nothing a block's region pays: no room, no ledger, no guard, no zero fill.
//!
//! - **`op(|f| ...)`** opens a frame on this thread's arena, at a mark, and gives everything after the mark back when
//!   it ends, by unwinding too. Frames nest last-in-first-out (`Frame::sub`; an `op` inside an `op` is the same). A
//!   stolen rayon job runs to its end on the thief's stack, so its frames open and close inside whatever that thread
//!   was doing.
//! - **`Frame::vec`** gives a `FrameVec`: at the top of its frame it grows in place; below the top it moves within the
//!   frame (the space it leaves comes back when the frame ends). A vector of a frame that is not the innermost (an
//!   outer frame's, grown while an inner one is open) grows on the heap instead, so that the inner frame's end never
//!   takes it. Dropped at the top, it gives its bytes back at once, so a loop's temporaries reuse one place.
//! - **`Frame::take`** and **`Frame::zeroed`**: a slice for the frame's life.
//! - Nothing of a frame outlives it: the closure's frame is a borrow the result cannot name. A frame and its vectors
//!   stay on their thread; a worker gets a `&mut [T]` split of one.
//! - **`InlineVec<T, N>`**: up to `N` values in place (the stack), past them on the heap. Sized from the survey's
//!   bound (note t60), so the heap is the rare case. Any `Copy` value, a slice's reference too.
//!
//! The arena is a list of chunks kept per thread: the first `FIRST` bytes, each next one twice the last up to `MOST`,
//! a larger take a chunk of its own size. Its first chunk stays warm in the caches between operations; past `KEEP`
//! bytes, the chunks are given back when the outermost frame ends.

use crate::Pod;
use std::cell::{Cell, UnsafeCell};
use std::marker::PhantomData;
use std::mem::{MaybeUninit, size_of};

/// The first chunk's bytes.
const FIRST: usize = 256 << 10;
/// The biggest chunk the arena makes for its own growth (a larger take gets one of its own size).
const MOST: usize = 16 << 20;
/// The bytes of chunks a thread keeps once its outermost frame ends.
const KEEP: usize = 64 << 20;
/// A chunk's alignment, and the most a take may ask for.
const ALIGN: usize = 64;

struct Chunk {
    ptr: *mut u8,
    len: usize,
}

impl Chunk {
    fn new(len: usize) -> Chunk {
        let layout = std::alloc::Layout::from_size_align(len, ALIGN).expect("a frame chunk's layout");
        // SAFETY: a layout of at least `FIRST` bytes.
        let ptr = unsafe { std::alloc::alloc(layout) };
        if ptr.is_null() {
            std::alloc::handle_alloc_error(layout);
        }
        Chunk { ptr, len }
    }
}

impl Drop for Chunk {
    fn drop(&mut self) {
        // SAFETY: allocated with this layout (`Chunk::new`).
        unsafe { std::alloc::dealloc(self.ptr, std::alloc::Layout::from_size_align_unchecked(self.len, ALIGN)) };
    }
}

/// A thread's arena: its chunks, the one being bumped (`cur`), the top's address in it (`top`) and that chunk's end
/// (`end`), and how many frames are open (`depth`). The chunks past `cur` are free: a frame's end goes back to its
/// mark, never forward.
struct Arena {
    chunks: UnsafeCell<Vec<Chunk>>,
    cur: Cell<usize>,
    top: Cell<*mut u8>,
    end: Cell<*mut u8>,
    depth: Cell<u32>,
}

thread_local! {
    static ARENA: Arena = const {
        Arena { chunks: UnsafeCell::new(Vec::new()), cur: Cell::new(0), top: Cell::new(std::ptr::null_mut()), end: Cell::new(std::ptr::null_mut()), depth: Cell::new(0) }
    };
}

impl Arena {
    fn chunks(&self) -> &mut Vec<Chunk> {
        // SAFETY: the arena is its thread's alone, and no reference to its list outlives a call of its own.
        unsafe { &mut *self.chunks.get() }
    }

    /// `bytes` at `align`, past the top: in the current chunk, else the next (made, or made bigger, as needed).
    #[inline]
    fn bump(&self, bytes: usize, align: usize) -> *mut u8 {
        debug_assert!(align.is_power_of_two() && align <= ALIGN);
        let top = self.top.get();
        let pad = (top as usize).wrapping_neg() & (align - 1);
        if !top.is_null() && pad + bytes <= self.end.get() as usize - top as usize {
            let start = top.wrapping_add(pad);
            self.top.set(start.wrapping_add(bytes));
            return start;
        }
        self.bump_slow(bytes, align)
    }

    #[cold]
    #[inline(never)]
    fn bump_slow(&self, bytes: usize, align: usize) -> *mut u8 {
        let chunks = self.chunks();
        let mut cur = self.cur.get();
        let mut top = chunks.get(cur).filter(|_| !self.top.get().is_null()).map_or(0, |c| self.top.get() as usize - c.ptr as usize);
        loop {
            if let Some(c) = chunks.get(cur) {
                let start = top.next_multiple_of(align);
                if start + bytes <= c.len {
                    self.cur.set(cur);
                    // SAFETY: inside the chunk.
                    unsafe {
                        self.top.set(c.ptr.add(start + bytes));
                        self.end.set(c.ptr.add(c.len));
                        return c.ptr.add(start);
                    }
                }
                if top == 0 {
                    // A free chunk too small for this take: made again, big enough.
                    chunks[cur] = Chunk::new(Self::grown(c.len, bytes));
                    continue;
                }
                cur += 1;
                top = 0;
                continue;
            }
            let last = chunks.last().map_or(FIRST / 2, |c| c.len);
            chunks.push(Chunk::new(Self::grown(last, bytes)));
        }
    }

    /// The size of the chunk after one of `last` bytes, for a take of `bytes`.
    fn grown(last: usize, bytes: usize) -> usize {
        (last * 2).clamp(FIRST, MOST).max(bytes.next_multiple_of(ALIGN))
    }

    /// Whether `end` is the top's address.
    #[inline]
    fn at_top(&self, end: *const u8) -> bool {
        !end.is_null() && self.top.get() as *const u8 == end
    }

    /// Grows the take that ends at the top by `extra` bytes, where its chunk has them.
    #[inline]
    fn extend_top(&self, extra: usize) -> bool {
        let top = self.top.get();
        if (self.end.get() as usize - top as usize) >= extra {
            self.top.set(top.wrapping_add(extra));
            true
        } else {
            false
        }
    }

    /// Gives back the take that ends at the top and starts at `start`.
    #[inline]
    fn give_back_top(&self, start: *const u8) {
        self.top.set(start as *mut u8);
    }

    /// Keeps at most `KEEP` bytes of chunks (with no frame open), and puts the top at the first one's start.
    fn trim(&self) {
        let chunks = self.chunks();
        let mut kept = 0;
        let n = chunks.iter().take_while(|c| {
            kept += c.len;
            kept <= KEEP
        });
        let n = n.count().max(1);
        chunks.truncate(n);
        self.cur.set(0);
        if let Some(c) = chunks.first() {
            self.top.set(c.ptr);
            // SAFETY: the chunk's end.
            self.end.set(unsafe { c.ptr.add(c.len) });
        }
    }
}

/// An open frame of this thread's arena (`op`): what its vectors and takes come from.
pub struct Frame {
    arena: *const Arena,
    depth: u32,
    /// A frame stays on its thread.
    _thread: PhantomData<*const ()>,
}

/// Runs `f` with a frame of this thread's arena, opened at the arena's top; everything taken from it is given back
/// when `f` ends (or unwinds). Nothing of the frame can be part of `f`'s result.
pub fn op<R>(f: impl FnOnce(&Frame) -> R) -> R {
    ARENA.with(|a| {
        let arena = a as *const Arena;
        let depth = a.depth.get() + 1;
        a.depth.set(depth);
        /// The frame's end: the arena back at its mark.
        struct Close {
            arena: *const Arena,
            cur: usize,
            top: *mut u8,
            end: *mut u8,
        }
        impl Drop for Close {
            fn drop(&mut self) {
                // SAFETY: this thread's arena, which outlives every frame of it.
                let a = unsafe { &*self.arena };
                a.cur.set(self.cur);
                a.top.set(self.top);
                a.end.set(self.end);
                let depth = a.depth.get() - 1;
                a.depth.set(depth);
                if depth == 0 {
                    a.trim();
                }
            }
        }
        let _close = Close { arena, cur: a.cur.get(), top: a.top.get(), end: a.end.get() };
        f(&Frame { arena, depth, _thread: PhantomData })
    })
}

impl Frame {
    fn arena(&self) -> &Arena {
        // SAFETY: this thread's arena (a frame never leaves its thread), which outlives every frame of it.
        unsafe { &*self.arena }
    }

    /// Whether no frame is open inside it.
    fn innermost(&self) -> bool {
        self.arena().depth.get() == self.depth
    }

    /// A frame inside this one (`op`).
    pub fn sub<R>(&self, f: impl FnOnce(&Frame) -> R) -> R {
        op(f)
    }

    /// An empty vector of this frame.
    pub fn vec<T: Pod>(&self) -> FrameVec<'_, T> {
        FrameVec { ptr: std::ptr::NonNull::dangling().as_ptr(), len: 0, cap: if size_of::<T>() == 0 { usize::MAX } else { 0 }, heap: false, depth: self.depth, arena: self.arena, _frame: PhantomData }
    }

    /// An empty vector of this frame with room for `n`.
    pub fn vec_with<T: Pod>(&self, n: usize) -> FrameVec<'_, T> {
        let mut v = self.vec();
        v.reserve(n);
        v
    }

    /// A vector of this frame holding `items`.
    pub fn collect<T: Pod, I: IntoIterator<Item = T>>(&self, items: I) -> FrameVec<'_, T> {
        let mut v = self.vec();
        v.extend(items);
        v
    }

    /// A vector of this frame holding a copy of `items`.
    pub fn vec_from<T: Pod + Copy>(&self, items: &[T]) -> FrameVec<'_, T> {
        let mut v = self.vec_with(items.len());
        v.extend_from_slice(items);
        v
    }

    /// `n` values of `T` for the frame's life, their contents whatever an earlier take left (write before reading, as
    /// `blocks::scratch::Scratch::take`). Taken from the innermost frame only: an outer frame's take, made while an
    /// inner one is open, would lie in the inner one's bytes.
    #[allow(clippy::mut_from_ref)]
    pub fn take<T: Pod>(&self, n: usize) -> &mut [T] {
        assert!(self.innermost(), "a take from a frame with an inner frame open");
        if size_of::<T>() == 0 || n == 0 {
            // SAFETY: an empty slice, or one of zero-sized values.
            return unsafe { std::slice::from_raw_parts_mut(std::ptr::NonNull::dangling().as_ptr(), n) };
        }
        let p = self.arena().bump(n.checked_mul(size_of::<T>()).expect("a take's bytes"), align_of::<T>().max(8));
        // SAFETY: the frame's own bytes, aligned for `T`, which no other take overlaps until the frame ends; every bit
        // pattern is a value of a `Pod` type.
        unsafe { std::slice::from_raw_parts_mut(p.cast::<T>(), n) }
    }

    /// `n` zero values of `T` for the frame's life.
    #[allow(clippy::mut_from_ref)]
    pub fn zeroed<T: Pod>(&self, n: usize) -> &mut [T] {
        let s = self.take::<T>(n);
        // SAFETY: the take's own bytes; zero is a value of every `Pod` type.
        unsafe { std::ptr::write_bytes(s.as_mut_ptr(), 0, n) };
        s
    }
}

/// A growable array in a frame (`Frame::vec`): read and written as a slice; gone with its frame.
pub struct FrameVec<'f, T: Pod> {
    ptr: *mut T,
    len: usize,
    cap: usize,
    /// Its values are on the heap: it grew while its frame was not the innermost.
    heap: bool,
    depth: u32,
    arena: *const Arena,
    _frame: PhantomData<(&'f Frame, T)>,
}

impl<'f, T: Pod> FrameVec<'f, T> {
    fn arena(&self) -> &Arena {
        // SAFETY: as `Frame::arena`: a vector never leaves its frame's thread.
        unsafe { &*self.arena }
    }

    fn end(&self) -> *const u8 {
        self.ptr.wrapping_add(self.cap) as *const u8
    }

    /// Its region is the arena's top, in its own frame, the innermost.
    #[inline]
    fn on_top(&self) -> bool {
        let a = self.arena();
        self.cap > 0 && a.at_top(self.end()) && !self.heap && a.depth.get() == self.depth
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    #[inline]
    pub fn capacity(&self) -> usize {
        self.cap
    }
    #[inline]
    pub fn as_slice(&self) -> &[T] {
        // SAFETY: its first `len` values are written.
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        // SAFETY: its first `len` values are written, and its own.
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) }
    }

    /// Room for `extra` more.
    #[inline]
    pub fn reserve(&mut self, extra: usize) {
        let need = self.len.checked_add(extra).expect("a frame vector's length");
        if need > self.cap {
            self.grow(need);
        }
    }

    /// Room for `need` in all: its first take inline; a later growth (`grow_more`) out of line.
    #[inline]
    fn grow(&mut self, need: usize) {
        if self.cap == 0 {
            let a = self.arena();
            if a.depth.get() == self.depth {
                let new_cap = need.max(4);
                self.ptr = a.bump(new_cap.checked_mul(size_of::<T>()).expect("a frame vector's bytes"), align_of::<T>().max(8)).cast();
                self.cap = new_cap;
                return;
            }
        }
        self.grow_more(need);
    }

    /// Room for `need`: in place at the top where the chunk has it, else moved within the frame (or to the heap, for a
    /// frame that is not the innermost).
    #[cold]
    #[inline(never)]
    fn grow_more(&mut self, need: usize) {
        let new_cap = need.max(self.cap * 2).max(4);
        let size = size_of::<T>();
        if self.on_top() && self.arena().extend_top((new_cap - self.cap) * size) {
            self.cap = new_cap;
            return;
        }
        let bytes = new_cap.checked_mul(size).expect("a frame vector's bytes");
        let innermost = self.arena().depth.get() == self.depth;
        let align = align_of::<T>().max(8);
        let p = if innermost {
            self.arena().bump(bytes, align)
        } else {
            let layout = std::alloc::Layout::from_size_align(bytes, align).expect("a frame vector's layout");
            // SAFETY: a layout of at least one value's bytes.
            let p = unsafe { std::alloc::alloc(layout) };
            if p.is_null() {
                std::alloc::handle_alloc_error(layout);
            }
            p
        };
        // SAFETY: `len` written values, moved to a region of at least `new_cap`, which does not overlap the old.
        unsafe { std::ptr::copy_nonoverlapping(self.ptr, p.cast::<T>(), self.len) };
        self.free_heap();
        self.ptr = p.cast();
        self.cap = new_cap;
        self.heap = !innermost;
    }

    fn free_heap(&mut self) {
        if self.heap && self.cap > 0 {
            // SAFETY: allocated with this layout (`grow`).
            unsafe { std::alloc::dealloc(self.ptr.cast(), std::alloc::Layout::from_size_align_unchecked(self.cap * size_of::<T>(), align_of::<T>().max(8))) };
        }
    }

    #[inline]
    pub fn push(&mut self, v: T) {
        if self.len == self.cap {
            self.grow(self.len + 1);
        }
        // SAFETY: below its room.
        unsafe { self.ptr.add(self.len).write(v) };
        self.len += 1;
    }

    #[inline]
    pub fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        self.len -= 1;
        // SAFETY: a written value, no longer counted.
        Some(unsafe { self.ptr.add(self.len).read() })
    }

    pub fn extend_from_slice(&mut self, src: &[T])
    where
        T: Copy,
    {
        self.reserve(src.len());
        // SAFETY: room for them, in its own region, which `src` (a slice the caller holds) does not overlap.
        unsafe { std::ptr::copy_nonoverlapping(src.as_ptr(), self.ptr.add(self.len), src.len()) };
        self.len += src.len();
    }

    pub fn resize(&mut self, n: usize, v: T)
    where
        T: Copy,
    {
        if n > self.len {
            self.reserve(n - self.len);
            for i in self.len..n {
                // SAFETY: below its room.
                unsafe { self.ptr.add(i).write(v) };
            }
        }
        self.len = n;
    }

    pub fn truncate(&mut self, n: usize) {
        self.len = self.len.min(n);
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }
}

impl<T: Pod> Drop for FrameVec<'_, T> {
    fn drop(&mut self) {
        if self.on_top() {
            self.arena().give_back_top(self.ptr as *const u8);
        }
        self.free_heap();
    }
}

impl<T: Pod> std::ops::Deref for FrameVec<'_, T> {
    type Target = [T];
    #[inline]
    fn deref(&self) -> &[T] {
        self.as_slice()
    }
}

impl<T: Pod> std::ops::DerefMut for FrameVec<'_, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut [T] {
        self.as_mut_slice()
    }
}

impl<T: Pod> Extend<T> for FrameVec<'_, T> {
    #[inline]
    fn extend<I: IntoIterator<Item = T>>(&mut self, items: I) {
        let items = items.into_iter();
        self.reserve(items.size_hint().0);
        let mut len = self.len;
        for v in items {
            if len == self.cap {
                self.len = len;
                self.grow(len + 1);
            }
            // SAFETY: below its room.
            unsafe { self.ptr.add(len).write(v) };
            len += 1;
        }
        self.len = len;
    }
}

impl<'a, T: Pod + Copy> Extend<&'a T> for FrameVec<'_, T> {
    fn extend<I: IntoIterator<Item = &'a T>>(&mut self, items: I) {
        self.extend(items.into_iter().copied())
    }
}

impl<'a, T: Pod> IntoIterator for &'a FrameVec<'_, T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.as_slice().iter()
    }
}

impl<T: Pod + std::fmt::Debug> std::fmt::Debug for FrameVec<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.as_slice().fmt(f)
    }
}

/// Up to `N` values in place, on the stack where it lies; past them, on the heap.
pub struct InlineVec<T: Copy, const N: usize> {
    len: usize,
    inline: [MaybeUninit<T>; N],
    spill: Option<Vec<T>>,
}

impl<T: Copy, const N: usize> InlineVec<T, N> {
    pub const fn new() -> Self {
        InlineVec { len: 0, inline: [const { MaybeUninit::uninit() }; N], spill: None }
    }

    #[inline]
    pub fn push(&mut self, v: T) {
        if let Some(s) = self.spill.as_mut() {
            s.push(v);
        } else if self.len < N {
            self.inline[self.len].write(v);
            self.len += 1;
        } else {
            self.spill_with(v);
        }
    }

    #[cold]
    fn spill_with(&mut self, v: T) {
        let mut s = Vec::with_capacity(N * 2 + 1);
        s.extend_from_slice(self.inline_slice());
        s.push(v);
        self.spill = Some(s);
    }

    fn inline_slice(&self) -> &[T] {
        // SAFETY: its first `len` values are written.
        unsafe { std::slice::from_raw_parts(self.inline.as_ptr().cast::<T>(), self.len) }
    }

    /// Whether it went past `N` (onto the heap).
    pub fn spilled(&self) -> bool {
        self.spill.is_some()
    }

    #[inline]
    pub fn as_slice(&self) -> &[T] {
        match &self.spill {
            Some(s) => s,
            None => self.inline_slice(),
        }
    }

    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        match &mut self.spill {
            Some(s) => s,
            // SAFETY: its first `len` values are written, and its own.
            None => unsafe { std::slice::from_raw_parts_mut(self.inline.as_mut_ptr().cast::<T>(), self.len) },
        }
    }

    pub fn pop(&mut self) -> Option<T> {
        match &mut self.spill {
            Some(s) => s.pop(),
            None if self.len > 0 => {
                self.len -= 1;
                // SAFETY: a written value, no longer counted.
                Some(unsafe { self.inline[self.len].assume_init_read() })
            }
            None => None,
        }
    }

    pub fn clear(&mut self) {
        self.len = 0;
        self.spill = None;
    }

    /// `n` values: cut, or filled up with `v`.
    pub fn resize(&mut self, n: usize, v: T) {
        if let Some(s) = self.spill.as_mut() {
            s.resize(n, v);
        } else if n <= self.len {
            self.len = n;
        } else {
            (self.len..n).for_each(|_| self.push(v));
        }
    }

    pub fn len(&self) -> usize {
        self.as_slice().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<T: Copy, const N: usize> Default for InlineVec<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Copy, const N: usize> Clone for InlineVec<T, N> {
    fn clone(&self) -> Self {
        let mut v = Self::new();
        v.extend(self.as_slice().iter().copied());
        v
    }
}

impl<T: Copy, const N: usize> std::ops::Deref for InlineVec<T, N> {
    type Target = [T];
    #[inline]
    fn deref(&self) -> &[T] {
        self.as_slice()
    }
}

impl<T: Copy, const N: usize> std::ops::DerefMut for InlineVec<T, N> {
    #[inline]
    fn deref_mut(&mut self) -> &mut [T] {
        self.as_mut_slice()
    }
}

impl<T: Copy, const N: usize> Extend<T> for InlineVec<T, N> {
    fn extend<I: IntoIterator<Item = T>>(&mut self, items: I) {
        for v in items {
            self.push(v);
        }
    }
}

impl<T: Copy, const N: usize> FromIterator<T> for InlineVec<T, N> {
    fn from_iter<I: IntoIterator<Item = T>>(items: I) -> Self {
        let mut v = Self::new();
        v.extend(items);
        v
    }
}

impl<T: Copy, const N: usize> IntoIterator for InlineVec<T, N> {
    type Item = T;
    type IntoIter = InlineIntoIter<T, N>;
    fn into_iter(self) -> Self::IntoIter {
        let back = self.len();
        InlineIntoIter { v: self, front: 0, back }
    }
}

impl<'a, T: Copy, const N: usize> IntoIterator for &'a InlineVec<T, N> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.as_slice().iter()
    }
}

/// An `InlineVec`'s values, by value.
pub struct InlineIntoIter<T: Copy, const N: usize> {
    v: InlineVec<T, N>,
    front: usize,
    back: usize,
}

impl<T: Copy, const N: usize> Iterator for InlineIntoIter<T, N> {
    type Item = T;
    #[inline]
    fn next(&mut self) -> Option<T> {
        (self.front < self.back).then(|| {
            self.front += 1;
            self.v.as_slice()[self.front - 1]
        })
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.back - self.front, Some(self.back - self.front))
    }
}

impl<T: Copy, const N: usize> DoubleEndedIterator for InlineIntoIter<T, N> {
    #[inline]
    fn next_back(&mut self) -> Option<T> {
        (self.front < self.back).then(|| {
            self.back -= 1;
            self.v.as_slice()[self.back]
        })
    }
}

impl<T: Copy, const N: usize> ExactSizeIterator for InlineIntoIter<T, N> {}

impl<T: Copy + PartialEq, const N: usize> PartialEq for InlineVec<T, N> {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}
impl<T: Copy + Eq, const N: usize> Eq for InlineVec<T, N> {}

impl<T: Copy + std::hash::Hash, const N: usize> std::hash::Hash for InlineVec<T, N> {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        self.as_slice().hash(h)
    }
}

impl<T: Copy + std::fmt::Debug, const N: usize> std::fmt::Debug for InlineVec<T, N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.as_slice().fmt(f)
    }
}

/// A frame vector cannot outlive its frame:
///
/// ```compile_fail
/// let v = rerail_vector::frame::op(|f| f.vec::<u32>());
/// ```
///
/// nor be sent to another thread:
///
/// ```compile_fail
/// rerail_vector::frame::op(|f| {
///     let v = f.vec::<u32>();
///     std::thread::scope(|s| { s.spawn(move || drop(v)); });
/// });
/// ```
#[doc(hidden)]
pub struct CompileFailTests;

#[cfg(test)]
mod tests {
    use super::*;

    fn top() -> (usize, usize, u32) {
        ARENA.with(|a| (a.cur.get(), a.top.get() as usize, a.depth.get()))
    }

    #[test]
    fn a_vector_at_the_top_grows_in_place() {
        op(|f| {
            let mut v = f.vec_with::<u32>(4);
            let at = v.as_ptr();
            for i in 0..1000 {
                v.push(i);
            }
            assert_eq!(v.as_ptr(), at, "grown in place at the top");
            assert_eq!(v.iter().copied().sum::<u32>(), 999 * 1000 / 2);
        });
    }

    #[test]
    fn a_vector_below_the_top_moves_within_the_frame() {
        op(|f| {
            let mut a = f.vec_with::<u64>(2);
            let mut b = f.vec_with::<u64>(2);
            a.extend([1, 2]);
            b.extend([7]);
            let was = a.as_ptr();
            a.extend(3..=100);
            assert_ne!(a.as_ptr(), was, "moved past the vector above it");
            assert!(!a.heap, "within the frame");
            assert_eq!(a.iter().copied().sum::<u64>(), 5050);
            assert_eq!(&b[..], &[7]);
        });
    }

    #[test]
    fn a_frame_gives_its_bytes_back_when_it_ends_and_when_it_unwinds() {
        // The thread's first chunk made, so that the top is where every outermost frame leaves it.
        op(|f| drop(f.vec_with::<u8>(1)));
        let before = top();
        op(|f| {
            let mut v = f.vec::<u8>();
            v.resize(10_000, 1);
            let inner = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                f.sub(|g| {
                    let _ = g.zeroed::<u64>(100_000);
                    panic!("unwound");
                })
            }));
            assert!(inner.is_err());
            assert_eq!(top().2, 1, "the inner frame closed by its unwinding");
            assert_eq!(v.len(), 10_000);
        });
        assert_eq!(top(), before);
    }

    #[test]
    fn an_outer_frames_vector_grown_inside_an_inner_frame_goes_to_the_heap() {
        op(|f| {
            let mut v = f.vec_with::<u32>(1);
            v.push(1);
            f.sub(|g| {
                let t = g.zeroed::<u32>(16);
                v.extend(2..=50);
                assert!(v.heap, "an outer frame's vector does not grow into the inner frame");
                t[0] = 9;
            });
            v.push(51);
            assert_eq!(v.iter().copied().sum::<u32>(), 51 * 52 / 2);
        });
    }

    #[test]
    fn a_vector_dropped_at_the_top_gives_its_bytes_back() {
        op(|f| {
            let first = {
                let mut v = f.vec::<u64>();
                v.extend(0..64);
                v.as_ptr() as usize
            };
            let mut again = f.vec::<u64>();
            again.extend(0..64);
            assert_eq!(again.as_ptr() as usize, first, "the same place, given back");
        });
    }

    #[test]
    fn a_take_larger_than_a_chunk_has_a_chunk_of_its_own() {
        op(|f| {
            let big = f.zeroed::<u8>(MOST * 2);
            assert!(big.iter().all(|&b| b == 0));
            big[MOST * 2 - 1] = 1;
            let small = f.take::<u32>(8);
            small.fill(3);
            assert_eq!(big[MOST * 2 - 1], 1);
        });
        ARENA.with(|a| assert!(a.chunks().iter().map(|c| c.len).sum::<usize>() <= KEEP.max(MOST * 2 + ALIGN), "kept within the bound"));
    }

    #[test]
    #[should_panic(expected = "inner frame open")]
    fn an_outer_frames_take_inside_an_inner_frame_fails() {
        op(|f| {
            f.sub(|_| {
                let _ = f.take::<u32>(4);
            })
        });
    }

    #[test]
    fn a_frames_split_is_written_by_scoped_workers() {
        op(|f| {
            let mut v = f.vec::<u64>();
            v.resize(1 << 16, 0);
            let (a, b) = v.split_at_mut(1 << 15);
            rayon::join(|| a.iter_mut().enumerate().for_each(|(i, x)| *x = i as u64), || b.iter_mut().for_each(|x| *x = 1));
            assert_eq!(v[..1 << 15].iter().sum::<u64>(), ((1u64 << 15) - 1) * (1 << 15) / 2);
            assert_eq!(v[1 << 15..].iter().sum::<u64>(), 1 << 15);
        });
    }

    #[test]
    fn a_stolen_jobs_frame_closes_inside_the_thiefs() {
        op(|_| {
            let sums: Vec<u64> = (0..64u64)
                .map(|i| {
                    rayon::join(
                        || op(|g| g.collect(0..i).iter().sum::<u64>()),
                        || op(|g| g.collect(0..i * 2).iter().sum::<u64>()),
                    )
                })
                .map(|(a, b)| a + b)
                .collect();
            assert_eq!(sums[10], 45 + 190);
        });
    }

    #[test]
    fn an_inline_vector_spills_past_its_bound() {
        let mut v = InlineVec::<u32, 4>::new();
        v.extend(0..4);
        assert!(!v.spilled());
        v.push(4);
        assert!(v.spilled());
        assert_eq!(&v[..], &[0, 1, 2, 3, 4]);
        assert_eq!(v.pop(), Some(4));
        let w: InlineVec<u32, 4> = [0, 1, 2].into_iter().collect();
        assert_eq!(w.len(), 3);
        assert_eq!(v.clone()[..4], [0, 1, 2, 3]);
        assert_eq!(v.clone().into_iter().rev().collect::<Vec<_>>(), [3, 2, 1, 0]);
        assert_eq!(w.into_iter().collect::<Vec<_>>(), [0, 1, 2]);
    }

    /// Not a check: what a 32-byte array made, filled and dropped costs, from a frame, the heap (`Vec`) and a
    /// `Vector` in its room (here the heap's; in the engine the run's block room, which costs more).
    #[test]
    #[ignore]
    fn what_a_small_array_costs() {
        const N: u64 = 10_000_000;
        let t = std::time::Instant::now();
        let mut s = 0u64;
        op(|f| {
            for i in 0..N {
                let mut v = f.vec_with::<u64>(4);
                v.extend([i, 1, 2, 3]);
                s += std::hint::black_box(&v)[0];
            }
        });
        let frame = t.elapsed();
        let t = std::time::Instant::now();
        op(|f| {
            for i in 0..N {
                let v = f.vec_with::<u64>(4);
                s += std::hint::black_box(&v).capacity() as u64 + i;
            }
        });
        let made = t.elapsed();
        let t = std::time::Instant::now();
        op(|f| {
            for i in 0..N / 1000 {
                f.sub(|g| {
                    for j in 0..1000 {
                        let v = g.take::<u64>(4);
                        v[0] = i + j;
                        s += std::hint::black_box(&v)[0];
                    }
                })
            }
        });
        let taken = t.elapsed();
        let t = std::time::Instant::now();
        for i in 0..N {
            let mut v = Vec::<u64>::with_capacity(4);
            v.extend([i, 1, 2, 3]);
            s += std::hint::black_box(&v)[0];
        }
        let heap = t.elapsed();
        let t = std::time::Instant::now();
        for i in 0..N {
            let mut v = crate::vector::Vector::<u64>::with_capacity(4);
            v.extend([i, 1, 2, 3]);
            s += std::hint::black_box(&v)[0];
        }
        let vector = t.elapsed();
        let t = std::time::Instant::now();
        for i in 0..N {
            let mut v = InlineVec::<u64, 4>::new();
            v.extend([i, 1, 2, 3]);
            s += std::hint::black_box(&v)[0];
        }
        let inline = t.elapsed();
        let per = |d: std::time::Duration| d.as_nanos() as f64 / N as f64;
        println!(
            "ns per array: inline {:.1}, frame {:.1} (made and dropped only {:.1}; a take {:.1}), Vec {:.1}, Vector (heap room) {:.1}  [{s}]",
            per(inline),
            per(frame),
            per(made),
            per(taken),
            per(heap),
            per(vector)
        );
    }
}
