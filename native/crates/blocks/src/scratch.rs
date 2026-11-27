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
//!
//! A region has a ledger (decision 0487 step 3): a kernel that binds its bytes holds it, a `reset` or the end of a use
//! with such work queued fails (the takes after it would write what the kernel reads), and a region dropped under
//! queued work keeps its memory until the work completes.

use crate::Pod;
use crate::block::Backing;
use std::cell::{Cell, RefCell};

/// Address space an anonymous scratch region reserves (nothing resident until written; on Windows nothing committed
/// until touched, `crate::windows`). Without a mapping (`crate::mem`) the reservation is allocated whole, so it is a
/// working size there.
#[cfg(any(unix, windows))]
const ANON: usize = 1 << 36;
#[cfg(not(any(unix, windows)))]
const ANON: usize = 256 << 20;
/// Bytes of a thread's scratch kept resident between uses; past them, a use's pages are given back.
const KEEP: usize = 64 << 20;

pub struct Scratch {
    base: *mut u8,
    len: usize,
    top: Cell<usize>,
    /// The most bytes a use has taken.
    high: Cell<usize>,
    /// The bytes `end` gave back to the OS, from and to: marked in use again (`reuse_pages`) when a take reaches them.
    given: Cell<(usize, usize)>,
    backing: Option<Box<dyn Backing>>,
    /// Its ledger: what a kernel's bind of its bytes holds (`Fence::uses_address`). None for a lent scratch (`lend`):
    /// its bytes are a block's, which keeps its own.
    ledger: Option<std::sync::Arc<crate::ledger::Ledger>>,
}

fn ledger_of(base: *mut u8, len: usize) -> std::sync::Arc<crate::ledger::Ledger> {
    let ext = std::sync::Arc::new(crate::block::Extent { top: std::sync::atomic::AtomicUsize::new(0), free: std::sync::atomic::AtomicUsize::new(0) });
    crate::ledger::Ledger::new(base, len.max(1), false, ext)
}

impl Scratch {
    /// An anonymous region of `reserved` bytes of address space.
    pub fn anon(reserved: usize) -> Scratch {
        let p = crate::mem::reserve(reserved);
        Scratch { base: p, len: reserved, top: Cell::new(0), high: Cell::new(0), given: Cell::new((0, 0)), backing: None, ledger: Some(ledger_of(p, reserved)) }
    }

    /// The region `backing` holds (a pooled slab): it may hold an earlier use's bytes.
    pub fn over(backing: Box<dyn Backing>) -> Scratch {
        let ledger = ledger_of(backing.base(), backing.len());
        Scratch { base: backing.base(), len: backing.len(), top: Cell::new(0), high: Cell::new(0), given: Cell::new((0, 0)), backing: Some(backing), ledger: Some(ledger) }
    }

    /// `f` with a scratch over `bytes` (an operation's scratch slot's, where its block lies), for the call alone: its
    /// takes are of those bytes (from their first 64-byte boundary), a bump of an offset. Nothing is allocated, mapped,
    /// kept or given back: what a phase's call takes is its slot's bytes, as on a device.
    pub fn lend<R>(bytes: &mut [u8], f: impl FnOnce(&Scratch) -> R) -> R {
        let skip = bytes.as_ptr().align_offset(64).min(bytes.len());
        let bytes = &mut bytes[skip..];
        let s = Scratch { base: bytes.as_mut_ptr(), len: bytes.len(), top: Cell::new(0), high: Cell::new(0), given: Cell::new((0, 0)), backing: None, ledger: None };
        f(&s)
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
        let (from, to) = self.given.get();
        if to > from && end > from {
            // SAFETY: the region's own pages, given back by `end`.
            unsafe { crate::mem::reuse_pages(self.base.add(from), to - from) };
            self.given.set((0, 0));
        }
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

    /// Gives every take back. Fails while GPU work that binds its bytes is queued: the next takes would write them.
    pub fn reset(&mut self) {
        self.quiet("a reset");
        self.top.set(0);
    }

    fn quiet(&self, what: &str) {
        if self.top.get() > 0 && self.ledger.as_ref().is_some_and(|l| !l.quiet()) {
            panic!("scratch: {what} of {} bytes taken, with GPU work that reads them queued (wait for its fence first)", self.top.get());
        }
    }

    /// Its ledger (decision 0487): what a fence's use of its bytes holds; none for a lent scratch.
    pub fn ledger(&self) -> Option<&std::sync::Arc<crate::ledger::Ledger>> {
        self.ledger.as_ref()
    }

    /// The most bytes a use has taken.
    pub fn high_water(&self) -> usize {
        self.high.get()
    }

    /// Ends a use: everything given back, and past `keep` bytes, an anonymous region's pages returned to the OS.
    fn end(&mut self, keep: usize) {
        self.quiet("the end of a use");
        let used = self.top.get();
        self.top.set(0);
        if self.backing.is_none() && used > keep {
            let from = keep.div_ceil(crate::block::PAGE) * crate::block::PAGE;
            // SAFETY: the region's own pages, none taken now.
            unsafe { crate::mem::free_pages(self.base.add(from), used - from) };
            let (f, t) = self.given.get();
            self.given.set(if t > f { (f.min(from), t.max(used)) } else { (from, used) });
        }
    }
}

impl Drop for Scratch {
    /// Its memory goes once no queued work binds it (R5): the mapping it made released, or its backing dropped.
    fn drop(&mut self) {
        // A lent scratch's bytes are their owner's: nothing to let go.
        let Some(ledger) = &self.ledger else { return };
        let (base, len) = (self.base as usize, self.len);
        let backing = self.backing.take();
        ledger.let_go(Box::new(move || match backing {
            Some(b) => drop(b),
            // SAFETY: the mapping this region made, which no work reaches now.
            None => unsafe { crate::mem::release(base as *mut u8, len) },
        }));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::Fence;

    /// A lent scratch takes from the bytes it is lent, from their first 64-byte boundary, and lets nothing of them go:
    /// the owner's bytes hold what the use wrote after it.
    #[test]
    fn a_lent_scratch_takes_from_the_bytes_it_is_lent() {
        let mut bytes = [0u8; 4096];
        let (lo, hi) = (bytes.as_ptr() as usize, bytes.as_ptr() as usize + bytes.len());
        let at = Scratch::lend(&mut bytes, |s| {
            let a = s.take::<u32>(16);
            let b = s.take::<u64>(8);
            a.fill(7);
            b.fill(9);
            for p in [a.as_ptr() as usize, b.as_ptr() as usize] {
                assert!(p % 64 == 0 && p >= lo && p + 64 <= hi, "a take of the lent bytes, aligned");
            }
            a.as_ptr() as usize - lo
        });
        assert_eq!(&bytes[at..at + 4], &7u32.to_ne_bytes(), "the use's writes stay in the owner's bytes");
        let full = std::panic::catch_unwind(|| Scratch::lend(&mut [0u8; 256], |s| s.take::<u64>(64).len()));
        assert!(full.is_err(), "a take past the lent bytes fails");
    }

    /// Decision 0487 step 3 at a scratch region: a kernel's bind of its bytes holds it; a reset (or a use's end) with
    /// that work queued fails, and passes once it completes; a region dropped under queued work keeps its memory until
    /// then.
    #[test]
    fn a_scratch_reset_under_a_queued_gpu_use_fails() {
        let fails = |f: &mut dyn FnMut()| std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).is_err();
        let mut s = Scratch::anon(1 << 24);
        let at = s.take::<u64>(512).as_ptr() as usize;
        let f = Fence::new();
        assert!(f.uses_address(at), "its bytes are found by address");
        assert!(fails(&mut || s.reset()), "a reset under the queued work");
        f.complete();
        s.reset();
        // Dropped under queued work: still found, so held, until the work completes.
        let s = Scratch::anon(1 << 24);
        let at = s.take::<u64>(8).as_ptr() as usize;
        let f = Fence::new();
        assert!(f.uses_address(at));
        drop(s);
        assert!(crate::ledger::of_address(at).is_some(), "its memory is held by the queued work");
        f.complete();
        assert!(crate::ledger::of_address(at).is_none(), "and goes once it completes");
    }
}
