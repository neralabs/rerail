//! A blocked Bloom filter in a block (decision 0488): a column's keys, read where it lies by the host, the JIT and
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

/// The hash a filter places a key by (the CLIF's `bloom_hash`, the kernels' and the JIT's), in 32-bit multiplies (the
/// device's native ones): the key's halves folded (`lo ^ hi * φ`), murmur3's finalizer, and a second word of it for
/// the high half.
#[inline(always)]
pub fn hash(key: i64) -> u64 {
    let v = key as u64;
    let mut h = (v as u32) ^ ((v >> 32) as u32).wrapping_mul(0x9e37_79b1);
    h ^= h >> 16;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^= h >> 16;
    let mut w = h.wrapping_mul(0x27d4_eb2f);
    w ^= w >> 15;
    (u64::from(w) << 32) | u64::from(h)
}

/// The bits of its word a key of hash `h` sets (the CLIF's `bloom_mask`): five, from 6-bit fields of `h`.
#[inline(always)]
pub fn mask(h: u64) -> u64 {
    (0..5).fold(0, |m, i| m | 1u64 << ((h >> (6 * i)) & 63))
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

    /// The filter whose descriptor lies at `at` of a block's bytes (a pinned block's, read in place): `None` when it
    /// does not fit there or is misaligned. Its words' span is relative to it, so it reads the same wherever the
    /// block is mapped.
    pub fn at(bytes: &[u8], at: usize) -> Option<&Bloom> {
        let end = at.checked_add(size_of::<Bloom>())?;
        if end > bytes.len() || at == 0 || !(bytes.as_ptr() as usize + at).is_multiple_of(align_of::<Bloom>()) {
            return None;
        }
        // SAFETY: in bounds and aligned; a descriptor is a span, read in place.
        Some(unsafe { &*bytes.as_ptr().add(at).cast::<Bloom>() })
    }

    /// The filter whose descriptor lies at `at` of `block` (`try_create`'s offset).
    pub fn in_block(block: &Block, at: usize) -> Option<&Bloom> {
        if at == 0 || at + size_of::<Bloom>() > block.reserved() {
            return None;
        }
        // SAFETY: within the block's reservation, where `try_create` put a descriptor.
        Some(unsafe { &*block.ptr::<Bloom>(at) })
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

    /// Whether a key of hash `h` may be in: false is certain (the CLIF's `bloom_may`, the read the kernels make). Its
    /// word `(h >> 32) & (words - 1)` has every bit of `mask(h)`; a filter of no words holds every key.
    #[inline(always)]
    pub fn may_hold_hashed(&self, h: u64) -> bool {
        if self.is_empty() {
            return true;
        }
        let (at, m) = place(h, self.len());
        self.word(at) & m == m
    }

    /// Which of `keys` (at most 64) may be in: bit `i` for key `i`. The block's hashes in one loop (no loads, so the
    /// compiler may keep several in flight, or vectorize them), then one test each (`bloom_may`): a screen's selection
    /// pass over a block of a scan's keys, as the device's is over a grid of them.
    pub fn may_hold_each(&self, keys: &[i64]) -> u64 {
        assert!(keys.len() <= 64, "a block of 64 keys at most");
        let mut h = [0u64; 64];
        for (o, &k) in h.iter_mut().zip(keys) {
            *o = hash(k);
        }
        h[..keys.len()].iter().enumerate().fold(0u64, |m, (i, &h)| m | u64::from(self.may_hold_hashed(h)) << i)
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
        let keys: Vec<i64> = (0..64i64).map(|i| i * 7919 + i % 2).collect();
        let each = f.may_hold_each(&keys);
        assert!((0..64).all(|i| (each >> i & 1 != 0) == f.may_hold_hashed(hash(keys[i]))), "a block's test is each key's");
        assert!((0..64).step_by(2).all(|i| each >> i & 1 != 0));
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
