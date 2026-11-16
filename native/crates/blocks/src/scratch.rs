//! Scratch: the temporaries of a freeze or a pack, taken from one region kept between uses instead of the heap.
//!
//! A freeze of 1M rows needs about 45 MB of temporaries (the columns read once, the sort's keys and ids, the
//! order, the index's buckets). As heap allocations that is a dozen large `malloc`s and `free`s per freeze, each
//! a fresh mapping faulted in page by page. A scratch region is mapped once and reused: a take is a bump of an
//! offset, `reset` gives everything back at once, and its pages stay resident for the next use.
//!
//! - **`Scratch::anon`**: address space only; a page costs once it is first written.
//! - **`Scratch::over`**: a region the caller owns, such as a slab of the engine's GPU buffer pool
//!   (`rerail_accel::gpu::shared::lease`), so freeze's temporaries count in the one GPU budget and can be read by
//!   a kernel.
//! - **Per thread** (`with`): each thread keeps one, made on first use (`Scratch::anon`), or the one
//!   `install`ed. Past `KEEP` bytes, what a use touched is given back to the OS when it ends.
//!
//! A take's contents are whatever the last use left: a caller writes before it reads, or asks for `zeroed`.

use crate::Pod;
use crate::block::Backing;
use std::cell::{Cell, RefCell};

/// Address space an anonymous scratch region reserves (nothing resident until written).
const ANON: usize = 1 << 36;
/// Bytes of a thread's scratch kept resident between uses; past them, a use's pages are given back.
const KEEP: usize = 64 << 20;

pub struct Scratch {
    base: *mut u8,
    len: usize,
    top: Cell<usize>,
    /// The most bytes a use has taken.
    high: Cell<usize>,
    backing: Option<Box<dyn Backing>>,
}

impl Scratch {
    /// An anonymous region of `reserved` bytes of address space.
    pub fn anon(reserved: usize) -> Scratch {
        let p = unsafe {
            libc::mmap(std::ptr::null_mut(), reserved, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_ANON | libc::MAP_PRIVATE, -1, 0)
        };
        assert!(p != libc::MAP_FAILED, "mmap of {reserved} bytes of scratch failed");
        Scratch { base: p.cast(), len: reserved, top: Cell::new(0), high: Cell::new(0), backing: None }
    }

    /// The region `backing` holds (a pooled slab): it may hold an earlier use's bytes.
    pub fn over(backing: Box<dyn Backing>) -> Scratch {
        Scratch { base: backing.base(), len: backing.len(), top: Cell::new(0), high: Cell::new(0), backing: Some(backing) }
    }

    /// `n` values of `T`, their contents whatever the last use left.
    #[allow(clippy::mut_from_ref)]
    pub fn take<T: Pod + Copy>(&self, n: usize) -> &mut [T] {
        let align = align_of::<T>().max(64);
        let start = self.top.get().div_ceil(align) * align;
        let end = start + n * size_of::<T>();
        assert!(end <= self.len, "scratch full: {end} of {} bytes", self.len);
        self.top.set(end);
        self.high.set(self.high.get().max(end));
        // SAFETY: in bounds, aligned, and no other take overlaps it until `reset`, which takes `&mut self`; every
        // bit pattern is a value of a `Pod` type.
        unsafe { std::slice::from_raw_parts_mut(self.base.add(start).cast::<T>(), n) }
    }

    /// `n` zero values of `T`.
    #[allow(clippy::mut_from_ref)]
    pub fn zeroed<T: Pod + Copy>(&self, n: usize) -> &mut [T] {
        let s = self.take::<T>(n);
        // SAFETY: the take's own bytes; zero is a value of every `Pod` type here.
        unsafe { std::ptr::write_bytes(s.as_mut_ptr(), 0, n) };
        s
    }

    /// Gives every take back.
    pub fn reset(&mut self) {
        self.top.set(0);
    }

    /// The most bytes a use has taken.
    pub fn high_water(&self) -> usize {
        self.high.get()
    }

    /// Ends a use: everything given back, and past `keep` bytes, an anonymous region's pages returned to the OS.
    fn end(&mut self, keep: usize) {
        let used = self.top.get();
        self.top.set(0);
        if self.backing.is_none() && used > keep {
            let from = keep.div_ceil(crate::block::PAGE) * crate::block::PAGE;
            // SAFETY: the region's own pages, none taken now.
            unsafe { libc::madvise(self.base.add(from).cast(), used - from, libc::MADV_FREE) };
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if self.backing.is_none() {
            // SAFETY: the mapping this region made.
            unsafe { libc::munmap(self.base.cast(), self.len) };
        }
    }
}

thread_local! {
    static THREAD: RefCell<Option<Scratch>> = const { RefCell::new(None) };
}

/// Runs `f` with this thread's scratch, emptied: made on first use, unless one was `install`ed.
pub fn with<R>(f: impl FnOnce(&Scratch) -> R) -> R {
    THREAD.with(|t| {
        let mut t = t.borrow_mut();
        let s = t.get_or_insert_with(|| Scratch::anon(ANON));
        s.reset();
        let out = f(s);
        s.end(KEEP);
        out
    })
}

/// Makes `s` this thread's scratch (a pooled slab, say); returns the one it replaces.
pub fn install(s: Scratch) -> Option<Scratch> {
    THREAD.with(|t| t.borrow_mut().replace(s))
}
