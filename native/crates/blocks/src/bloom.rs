//! A blocked Bloom filter in a block (decision 0477): a column's keys, read where it lies by the host, the JIT and
//! the kernels, through the reads written once in CLIF (`rerail_clif::relation`: `bloom_hash`, `bloom_mask`,
//! `bloom_may`). One hash, one format, one memory for the CPU and the GPU.
//!
//! ```text
//! h     = bloom_hash(key)                 // 32-bit multiplies (the device's native ones), 64 bits out
//! word  = (h >> 32) & (words - 1)         // one 64-bit word holds all of a key's bits
//! mask  = bit(h) | bit(h >> 6) | ... (5)  // 5 bits of the word, from 6-bit fields of h's low half
//! add:  words[word] |= mask               // the host's one writer: a load and a store; a kernel's: two 32-bit
//! test: words[word] & mask == mask        //   atomics, bit b of word w being bit b & 31 of 32-bit word 2w + b / 32
//! ```
//!
//! A filter of `2w` words folds to `w` (word `i` is words `i` and `i + w` OR-ed): a key's word is the low bits of
//! the same field.
//!
//! The descriptor and the words are two chunks of the block (the words a power of two, a free list's class), given
//! back by `release`.

use crate::block::Block;
use crate::refs::Span;
use crate::structure;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

structure! {
    /// A filter's words, `words.len()` of them, a power of two (0: a filter that holds every key).
    pub struct Bloom {
        pub words: Span<u64>,
    }
}

/// The descriptor's chunk: the smallest class.
const DESC_BYTES: usize = 64;

/// The hash a filter places a key by (`bloom_hash`, the read written once).
#[inline(always)]
pub fn hash(key: i64) -> u64 {
    // SAFETY: `bloom_hash` reads nothing through its descriptor.
    unsafe { crate::generated_reads::bloom_hash(std::ptr::null(), key as u64) }
}

/// The bits of its word a key of hash `h` sets (`bloom_mask`).
#[inline(always)]
pub fn mask(h: u64) -> u64 {
    // SAFETY: `bloom_mask` reads nothing through its descriptor.
    unsafe { crate::generated_reads::bloom_mask(std::ptr::null(), h) }
}

/// A key's word (of `words`, a power of two) and its bits there.
#[inline(always)]
pub fn place(h: u64, words: usize) -> (usize, u64) {
    ((h >> 32) as usize & (words - 1), mask(h))
}

impl Bloom {
    /// An empty filter of `words` words (a power of two) in `block`: its descriptor's offset, or `None` when the
    /// block has no room.
    pub fn try_create(block: &Block, words: usize) -> Option<usize> {
        assert!(words.is_power_of_two(), "a filter's words are a power of two");
        let at = block.try_alloc_chunk(DESC_BYTES, 64)?;
        let Some(w) = block.try_alloc_chunk(8 * words, 64) else {
            block.free_chunk(at, DESC_BYTES);
            return None;
        };
        let desc: *mut Bloom = block.ptr(at);
        // SAFETY: both chunks are this block's, just taken (zeroed), and nothing else holds them.
        unsafe { Span::set(&raw const (*desc).words, block.ptr::<u64>(w), words) };
        Some(at)
    }

    /// Gives its chunks back to `block`, its own. Nothing may read it after.
    pub fn release(&self, block: &Block) {
        let (at, words) = (block.offset_of(self), block.offset_of(self.words.as_ptr()));
        block.free_chunk(words, 8 * self.len());
        block.free_chunk(at, DESC_BYTES);
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.words.len()
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// Its words as they are now: a writer may set bits meanwhile (relaxed loads).
    #[inline(always)]
    pub fn words(&self) -> &[AtomicU64] {
        // SAFETY: the words are 8-byte aligned u64s of the block, which live while the descriptor does; read as
        // atomics, since their one writer stores while readers load.
        unsafe { std::slice::from_raw_parts(self.words.as_ptr().cast::<AtomicU64>(), self.len()) }
    }

    #[inline(always)]
    pub fn word(&self, i: usize) -> u64 {
        self.words()[i].load(Relaxed)
    }

    /// Whether a key of hash `h` may be in: false is certain (`bloom_may`, the read the kernels make).
    #[inline(always)]
    pub fn may_hold_hashed(&self, h: u64) -> bool {
        // SAFETY: a live descriptor in its block.
        unsafe { crate::generated_reads::bloom_may(std::ptr::from_ref(self).cast(), h) != 0 }
    }

    /// Adds the key whose hash is `h`, and says whether it was new (not every one of its bits set before). Its one
    /// writer only: a load and a store, no read-modify-write.
    #[inline(always)]
    pub fn add_hashed(&self, h: u64) -> bool {
        let (w, m) = place(h, self.len());
        let word = &self.words()[w];
        let was = word.load(Relaxed);
        if was & m == m {
            return false;
        }
        word.store(was | m, Relaxed);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::kind;

    /// A filter made in a block holds every key added and refuses most others, through the generated test (the
    /// kernels' own); its chunks go back to the block's free list.
    #[test]
    fn a_filter_in_a_block_holds_its_keys() {
        let block = Block::anon(kind::FACTS, 1 << 20).with_free_list();
        let at = Bloom::try_create(&block, 256).expect("room");
        // SAFETY: the descriptor just made.
        let f = unsafe { &*block.ptr::<Bloom>(at) };
        for k in (0..2_000i64).map(|i| i * 7919) {
            f.add_hashed(hash(k));
        }
        assert!((0..2_000i64).all(|i| f.may_hold_hashed(hash(i * 7919))));
        let passed = (0..20_000i64).filter(|i| f.may_hold_hashed(hash(i * 7919 + 1))).count();
        assert!(passed < 2_000, "{passed} of 20,000 absent keys passed");
        let free = block.free_bytes();
        f.release(&block);
        assert!(block.free_bytes() >= free + 8 * 256);
    }

    /// The hash's halves are each spread: keys sharing either 32-bit half never collide, and the words a set of
    /// structured keys (a document's nodes) takes are close to even.
    #[test]
    fn the_hash_spreads_structured_keys() {
        let mut seen = std::collections::HashSet::new();
        for doc in 0..64i64 {
            for node in 0..1_024i64 {
                assert!(seen.insert(hash(doc << 32 | node)), "a collision at {doc}, {node}");
            }
        }
        let mut words = [0u32; 1024];
        for &h in &seen {
            words[place(h, 1024).0] += 1;
        }
        let most = *words.iter().max().unwrap();
        assert!(most < 64 * 3, "a word took {most} of {} keys", seen.len());
    }
}
