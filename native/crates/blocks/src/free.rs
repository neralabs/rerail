//! A block's free list (decision 0342): chunks given back are taken again by the next chunk of their size.
//!
//! Held to what a GPU kernel can run as well as the host, and to big pieces, not millions of small ones:
//!
//! - Classes by size: powers of two from 64 B (`MIN_LOG`). A relation's column chunks are powers of two in
//!   bytes, so a chunk is never split and free chunks are never merged.
//! - A class's list is a stack whose head lies in the block's own memory (mapped with it, bindable by a kernel):
//!   a 64-bit word, 32 bits of offset in 64-byte units (0: empty) and a 32-bit tag that every push and pop
//!   advances, so one compare-and-swap is safe against a chunk popped and pushed back meanwhile.
//! - A free chunk's first 32 bits hold the next one's offset, in the same units.
//!
//! ```text
//! heads[class] = tag:32 | chunk/64:32 ──► chunk: [next/64:32 | ...] ──► chunk: [0 | ...]
//! pop:  h = head; next = *(h.chunk); CAS(head, h, (h.tag + 1, next))
//! push: h = head; *(chunk) = h.chunk; CAS(head, h, (h.tag + 1, chunk))
//! ```

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering::*};

/// The smallest class: 64 bytes.
pub const MIN_LOG: u32 = 6;
/// Classes: 64 B to 2^(MIN_LOG + CLASSES - 1) bytes.
pub const CLASSES: usize = 40;

/// The class of a chunk of `bytes` (rounded up to a power of two of at least 64), and its size.
#[inline]
pub fn class_of(bytes: usize) -> (usize, usize) {
    let size = bytes.max(1 << MIN_LOG).next_power_of_two();
    ((size.trailing_zeros() - MIN_LOG) as usize, size)
}

/// Pops a chunk of `class` from the heads at `heads` (in the block at `base`): its offset, or `None`.
///
/// # Safety
/// `heads` is a block's free-list heads, and `base` that block's start; the chunks on it lie in the block.
#[inline]
pub unsafe fn pop(base: *mut u8, heads: *const AtomicU64, class: usize) -> Option<usize> {
    // SAFETY: the caller's.
    let head = unsafe { &*heads.add(class) };
    let mut h = head.load(Acquire);
    loop {
        let at = (h as u32 as usize) << MIN_LOG;
        if at == 0 {
            return None;
        }
        // The chunk may be popped by another meanwhile: its link is then stale, and the tag makes the swap fail.
        let next = link(base, at).load(Acquire) as u64;
        match head.compare_exchange_weak(h, next | ((h >> 32).wrapping_add(1) << 32), AcqRel, Acquire) {
            Ok(_) => return Some(at),
            Err(now) => h = now,
        }
    }
}

/// Pushes the chunk at `off` (of `class`) on the heads at `heads`.
///
/// # Safety
/// As `pop`; the chunk is the caller's, unread by anyone else, and of `class`'s size.
#[inline]
pub unsafe fn push(base: *mut u8, heads: *const AtomicU64, class: usize, off: usize) {
    debug_assert!(off != 0 && off.is_multiple_of(1 << MIN_LOG), "a chunk at {off}");
    // SAFETY: the caller's.
    let head = unsafe { &*heads.add(class) };
    let mut h = head.load(Acquire);
    loop {
        link(base, off).store(h as u32, Release);
        match head.compare_exchange_weak(h, (off >> MIN_LOG) as u64 | ((h >> 32).wrapping_add(1) << 32), AcqRel, Acquire) {
            Ok(_) => return,
            Err(now) => h = now,
        }
    }
}

/// Every chunk on class `class`'s list (a freeze walks them; no one pops or pushes meanwhile).
///
/// # Safety
/// As `pop`.
pub unsafe fn each(base: *mut u8, heads: *const AtomicU64, class: usize, mut f: impl FnMut(usize)) {
    // SAFETY: the caller's.
    let mut at = (unsafe { &*heads.add(class) }.load(Acquire) as u32 as usize) << MIN_LOG;
    while at != 0 {
        let next = (link(base, at).load(Acquire) as usize) << MIN_LOG;
        f(at);
        at = next;
    }
}

fn link(base: *mut u8, off: usize) -> &'static AtomicU32 {
    // SAFETY: a chunk of the block, 64-aligned.
    unsafe { &*(base.add(off) as *const AtomicU32) }
}

#[cfg(test)]
mod tests {
    use crate::block::{Block, PAGE, kind};

    fn block() -> Block {
        Block::anon(kind::FACTS, 8 << 20).with_free_list()
    }

    /// A chunk given back is taken by the next chunk of its size, zeroed; one of another size is not.
    #[test]
    fn a_freed_chunk_is_taken_by_the_next_of_its_size_zeroed() {
        let b = block();
        let a = b.try_alloc_chunk(4096, 64).unwrap();
        unsafe { std::ptr::write_bytes(b.ptr::<u8>(a), 0xab, 4096) };
        let top = b.used();
        b.free_chunk(a, 4096);
        assert_eq!(b.free_bytes(), 4096);
        let other = b.try_alloc_chunk(8192, 64).unwrap();
        assert_ne!(other, a, "another size does not take it");
        let again = b.try_alloc_chunk(4096, 64).unwrap();
        assert_eq!(again, a, "the next of its size takes it");
        assert_eq!(b.free_bytes(), 0);
        let bytes = unsafe { std::slice::from_raw_parts(b.ptr::<u8>(again), 4096) };
        assert!(bytes.iter().all(|&x| x == 0), "a reused chunk reads zero, as a fresh one");
        assert_eq!(b.used(), top + 8192, "only the other size bumped");
    }

    /// Sizes under 64 bytes are 64-byte chunks: freed and taken as such.
    #[test]
    fn a_small_chunk_is_a_64_byte_one() {
        let b = block();
        let a = b.try_alloc_chunk(16, 64).unwrap();
        let c = b.try_alloc_chunk(16, 64).unwrap();
        assert!(c >= a + 64, "a 16-byte chunk takes 64");
        b.free_chunk(a, 16);
        assert_eq!(b.try_alloc_chunk(40, 64), Some(a));
    }

    /// Threads taking and giving back chunks at once never hand one chunk out twice.
    #[test]
    fn chunks_taken_and_given_back_at_once_are_never_handed_out_twice() {
        let b = block();
        let held = std::sync::Mutex::new(std::collections::HashSet::new());
        // Each thread is the test's (decision 0442: a caller's thread holds its object).
        let caller = rerail_context::caller();
        std::thread::scope(|s| {
            for _ in 0..8 {
                s.spawn(|| {
                    let _caller = caller.enter();
                    for i in 0..2000 {
                        let a = b.try_alloc_chunk(256, 64).unwrap();
                        assert!(held.lock().unwrap().insert(a), "chunk {a} handed out twice");
                        if i % 3 != 0 {
                            held.lock().unwrap().remove(&a);
                            b.free_chunk(a, 256);
                        }
                    }
                });
            }
        });
        // Every chunk is held or free, and the lists hold what was given back and not taken again.
        let n = held.lock().unwrap().len();
        assert!(b.used() >= n * 256 + b.free_bytes());
        assert!(b.free_bytes() <= 8 * 256, "a thread takes back what it gave: at most one chunk each is left free");
    }

    /// A bump block keeps no list: a chunk given back is not taken again.
    #[test]
    fn a_bump_block_reuses_nothing() {
        let b = Block::anon(kind::FACTS, 1 << 20);
        let a = b.try_alloc_chunk(4096, 64).unwrap();
        b.free_chunk(a, 4096);
        assert_eq!(b.free_bytes(), 0);
        assert_ne!(b.try_alloc_chunk(4096, 64), Some(a));
    }

    /// Freezing drops the lists: no chunk is free after, and the block reads as it was written.
    #[test]
    fn a_freeze_drops_the_free_list() {
        let b = block();
        let a = b.try_alloc_chunk(4 * PAGE, PAGE).unwrap();
        let keep = b.try_alloc_chunk(64, 64).unwrap();
        unsafe { *b.ptr::<u64>(keep) = 7 };
        assert!(b.has_free_list());
        b.free_chunk(a, 4 * PAGE);
        let f = b.freeze();
        assert_eq!(f.free_bytes(), 0);
        assert!(!f.has_free_list());
        assert_eq!(unsafe { *f.ptr::<u64>(keep) }, 7);
    }

    /// A chunk written before it is read (a relation's rows) is taken again as it was left: no bytes are zeroed
    /// for it. One asked zeroed is, and the bytes zeroed are counted (the run's count: other tests zero too).
    #[test]
    fn a_chunk_written_before_read_is_not_zeroed() {
        let _run = rerail_context::test_run();
        let b = block();
        let a = b.try_alloc_chunk(4096, 64).unwrap();
        unsafe { std::ptr::write_bytes(b.ptr::<u8>(a), 0xab, 4096) };
        b.free_chunk(a, 4096);
        let zeroed = crate::block::zeroed_here();
        assert_eq!(b.try_alloc_chunk_any(4096, 64), Some(a));
        assert_eq!(unsafe { *b.ptr::<u8>(a + 100) }, 0xab, "left as it was");
        assert_eq!(crate::block::zeroed_here(), zeroed, "nothing zeroed");
        b.free_chunk(a, 4096);
        assert_eq!(b.try_alloc_chunk(4096, 64), Some(a));
        assert_eq!(unsafe { *b.ptr::<u8>(a + 100) }, 0, "zeroed when asked");
        assert!(crate::block::zeroed_here() >= zeroed + 4096, "and counted");
    }
}
