//! A block's free lists (decision 0342, amended by 0494 step 6): chunks given back are taken again by the next chunk
//! of their size. The lists are the block's ledger's (`ledger::State::lists`), not in the chunks: a free chunk holds
//! nothing, so its pages go to the OS once no live chunk shares them.
//!
//! - Classes by size: powers of two from 64 B (`MIN_LOG`).
//! - Each chunk lies aligned to its size, up to `ALIGN_MAX` (from the front, the bytes skipped to align it go onto the
//!   lists as pieces). A chunk freed merges with its buddy (the chunk of its size beside it, `at ^ size`) while the
//!   buddy is free, and a class with none free splits the smallest larger one: freed small chunks make whole pages and
//!   larger chunks again.
//! - The lowest free chunk of a class is taken first.

/// The smallest class: 64 bytes.
pub const MIN_LOG: u32 = 6;
/// Classes: 64 B to 2^(MIN_LOG + CLASSES - 1) bytes.
pub const CLASSES: usize = 40;

/// The most a chunk from a block's front is aligned to: one of a larger class lies at a multiple of this (a block made
/// for a chunk has room for it and this much more, `facts::rows`' margin), and merges only where it is aligned to its
/// size.
pub const ALIGN_MAX: usize = 1 << 20;

/// The piece of `left` free bytes at `at` that goes onto a list as one chunk: the largest power of two that fits and
/// is aligned to its size, or, from `ALIGN_MAX` up, the largest that fits at an offset aligned to `ALIGN_MAX`.
#[inline]
pub fn piece(at: usize, left: usize) -> usize {
    let p = 1usize << (usize::BITS - 1 - left.leading_zeros());
    if p >= ALIGN_MAX && at.is_multiple_of(ALIGN_MAX) { p } else { p.min(1 << at.trailing_zeros()) }
}

/// The class of a chunk of `bytes` (rounded up to a power of two of at least 64), and its size.
#[inline]
pub fn class_of(bytes: usize) -> (usize, usize) {
    let size = bytes.max(1 << MIN_LOG).next_power_of_two();
    ((size.trailing_zeros() - MIN_LOG) as usize, size)
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
        let (top, free) = (b.used(), b.free_bytes());
        b.free_chunk(a, 4096);
        assert_eq!(b.free_bytes(), free + 4096);
        let other = b.try_alloc_chunk(8192, 64).unwrap();
        assert_ne!(other, a, "another size does not take it");
        let again = b.try_alloc_chunk(4096, 64).unwrap();
        assert_eq!(again, a, "the next of its size takes it");
        assert_eq!(b.free_bytes(), free);
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

    /// Every free chunk taken (as 64-byte chunks): the pieces skipped to align the block's first chunks, so that a test
    /// sees only the chunks it frees.
    fn drain(b: &Block) {
        while b.free_bytes() > 0 {
            b.try_alloc_chunk(64, 64).unwrap();
        }
    }

    /// Two free buddies merge into one chunk of the class above, which the next of that size takes; a class with none
    /// free splits the smallest larger chunk, its halves taken by the next of their size, before anything bumps.
    #[test]
    fn free_buddies_merge_and_a_larger_chunk_splits() {
        let b = block();
        let whole = b.try_alloc_chunk(8192, 64).unwrap();
        assert_eq!(whole % 8192, 0, "aligned to its size");
        drain(&b);
        b.free_chunk(whole, 8192);
        let top = b.used();
        assert_eq!(b.try_alloc_chunk(4096, 64), Some(whole), "split: its lower half");
        assert_eq!(b.try_alloc_chunk(4096, 64), Some(whole + 4096), "and its upper");
        b.free_chunk(whole, 4096);
        b.free_chunk(whole + 4096, 4096);
        assert_eq!(b.try_alloc_chunk(8192, 64), Some(whole), "merged: the 8 KB chunk again");
        b.free_chunk(whole, 8192);
        assert_eq!(b.try_alloc_chunk(2048, 64), Some(whole), "split twice: its lowest quarter");
        assert_eq!(b.try_alloc_chunk(2048, 64), Some(whole + 2048));
        assert_eq!(b.try_alloc_chunk(4096, 64), Some(whole + 4096));
        assert_eq!(b.used(), top, "nothing from the front");
    }

    /// A chunk's pages go to the OS only once no live chunk shares them and its free chunk, merged, is of
    /// `ledger::GIVE_MIN` or more: freed beside a live chunk the page stays; when the last chunk in it is freed, the
    /// rest of the block live, its free chunk is a page and stays too, for the next chunk of its class (taken with no
    /// fault). A free that merges past `GIVE_MIN` gives its pages (`ledger`'s `a_small_free_keeps_its_pages`).
    #[cfg(target_vendor = "apple")]
    #[test]
    fn a_page_stays_while_a_live_chunk_shares_it_or_its_free_chunk_is_small() {
        let b = block();
        let page = b.try_alloc_chunk(PAGE, 64).unwrap();
        drain(&b);
        b.free_chunk(page, PAGE);
        let (a, c) = (b.try_alloc_chunk(PAGE / 2, 64).unwrap(), b.try_alloc_chunk(PAGE / 2, 64).unwrap());
        assert_eq!((a, c), (page, page + PAGE / 2), "two halves of one page");
        unsafe { std::ptr::write_bytes(b.ptr::<u8>(a), 0xab, PAGE) };
        b.free_chunk(a, PAGE / 2);
        assert!(crate::mem::tests::modified(b.ptr::<u8>(a)), "a live chunk shares the page");
        b.free_chunk(c, PAGE / 2);
        assert!(crate::mem::tests::modified(b.ptr::<u8>(a)), "its free chunk a page, under GIVE_MIN: kept");
        assert_eq!(b.try_alloc_chunk(PAGE, 64), Some(page), "the merged page taken again");
    }

    /// A block keeping pages below a size (`keeping_pages_below`, a thread's large room): a chunk freed under it keeps
    /// its pages for the next; one at or past it gives them back, as every block's does.
    #[cfg(target_vendor = "apple")]
    #[test]
    fn a_chunk_under_the_resident_size_keeps_its_pages() {
        let b = block().keeping_pages_below(4 * PAGE);
        let small = b.try_alloc_chunk(PAGE, 64).unwrap();
        let big = b.try_alloc_chunk(8 * PAGE, 64).unwrap();
        drain(&b);
        unsafe { std::ptr::write_bytes(b.ptr::<u8>(small), 0xab, PAGE) };
        unsafe { std::ptr::write_bytes(b.ptr::<u8>(big), 0xab, 8 * PAGE) };
        assert_eq!(b.free_chunk(small, PAGE), 0, "nothing given to the OS");
        assert!(crate::mem::tests::modified(b.ptr::<u8>(small)), "its page kept");
        assert!(b.free_chunk(big, 8 * PAGE) > 0, "a chunk past the size gives its pages back");
        assert!(!crate::mem::tests::modified(b.ptr::<u8>(big)));
        assert_eq!(b.try_alloc_chunk(PAGE, 64), Some(small), "the kept page taken again");
    }

    /// A block made for a chunk (`facts::rows`' `2 * need` and its 1 MB margin) holds it: a large chunk lies aligned
    /// to `ALIGN_MAX`, not to its size, so it has no buddy, and its pieces go back by their own sizes.
    #[test]
    fn a_large_chunk_fits_a_block_made_for_it() {
        let need = 5 << 20;
        let b = Block::anon(kind::FACTS, 2 * need + super::ALIGN_MAX).with_free_list();
        let (at, kept) = b.try_alloc_chunk_fit(need, 64).expect("room for an 8 MB class at 1 MB");
        assert_eq!(at % super::ALIGN_MAX, 0);
        assert!(kept >= need);
        b.free_chunk_fit(at, kept);
        assert_eq!(b.try_alloc_chunk(4 << 20, 64), Some(at), "its first piece, 4 MB at 1 MB");
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
        // Every chunk is held or free, and the lists hold what was given back and not taken again (and the bytes
        // skipped to align the first chunk).
        let n = held.lock().unwrap().len();
        assert!(b.used() >= n * 256 + b.free_bytes());
        assert!(b.free_bytes() <= 8 * 256 + 256, "a thread takes back what it gave: at most one chunk each is left free");
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

    /// Q337: a freeze gives back only the free chunks' own pages, never one a live chunk shares. A free chunk under a
    /// page shares its page with its neighbours; marked reusable, the OS could drop the page under pressure, and a
    /// reader read zeros.
    #[cfg(target_vendor = "apple")]
    #[test]
    fn a_freeze_gives_back_no_page_a_live_chunk_shares() {
        let b = block();
        let before = b.try_alloc_chunk(64, 64).unwrap();
        let mid = b.try_alloc_chunk(4096, 64).unwrap();
        let after = b.try_alloc_chunk(64, 64).unwrap();
        assert!(before / PAGE == mid / PAGE && after / PAGE == mid / PAGE, "the chunk shares its page with both");
        unsafe {
            *b.ptr::<u64>(before) = 7;
            *b.ptr::<u64>(after) = 9;
        }
        b.free_chunk(mid, 4096);
        let f = b.freeze();
        assert!(crate::mem::tests::modified(f.ptr::<u8>(before)), "the page of the chunk before still holds its bytes");
        assert!(crate::mem::tests::modified(f.ptr::<u8>(after)), "the page of the chunk after still holds its bytes");
        assert_eq!(unsafe { (*f.ptr::<u64>(before), *f.ptr::<u64>(after)) }, (7, 9));
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
        let zeroed = crate::block::zeroed_bytes();
        assert_eq!(b.try_alloc_chunk_any(4096, 64), Some(a));
        assert_eq!(unsafe { *b.ptr::<u8>(a + 100) }, 0xab, "left as it was");
        assert_eq!(crate::block::zeroed_bytes(), zeroed, "nothing zeroed");
        b.free_chunk(a, 4096);
        assert_eq!(b.try_alloc_chunk(4096, 64), Some(a));
        assert_eq!(unsafe { *b.ptr::<u8>(a + 100) }, 0, "zeroed when asked");
        assert!(crate::block::zeroed_bytes() >= zeroed + 4096, "and counted");
    }
}
