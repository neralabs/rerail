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
// The filter's format, its hash and its reads: the floor's one definition (`rerail_delta_core::bloom`).
pub use rerail_delta_core::bloom::{Bloom, hash, mask, place};

/// The descriptor's chunk: the smallest class.
const DESC_BYTES: usize = 64;

/// A filter's writes that take room from its block: blocks' own, over the floor's `Bloom`.
pub trait BloomW {
    /// An empty filter of `words` words (a power of two) in `block`: its descriptor's offset, or `None` when the
    /// block has no room.
    fn try_create(block: &Block, words: usize) -> Option<usize>;

    /// The filter whose descriptor lies at `at` of `block` (`try_create`'s offset).
    fn in_block(block: &Block, at: usize) -> Option<&Bloom>;

    /// Gives its chunks back to `block`, its own. Nothing may read it after.
    fn release(&self, block: &Block);
}

impl BloomW for Bloom {
    fn try_create(block: &Block, words: usize) -> Option<usize> {
        assert!(words.is_power_of_two(), "a filter's words are a power of two");
        let at = block.try_alloc_chunk(DESC_BYTES, 64)?;
        let Some(w) = block.try_alloc_chunk(8 * words, 64) else {
            block.free_chunk(at, DESC_BYTES);
            return None;
        };
        let desc: *mut Bloom = block.ptr(at);
        // SAFETY: both chunks are this block's, just taken (zeroed), and nothing else holds them.
        unsafe { block.span(&raw const (*desc).words, block.ptr::<u64>(w), words) };
        Some(at)
    }
    fn in_block(block: &Block, at: usize) -> Option<&Bloom> {
        if at == 0 || at + size_of::<Bloom>() > block.reserved() {
            return None;
        }
        // SAFETY: within the block's reservation, where `try_create` put a descriptor.
        Some(unsafe { &*block.ptr::<Bloom>(at) })
    }
    fn release(&self, block: &Block) {
        let (at, words) = (block.offset_of(self), block.offset_of(self.words.as_ptr()));
        block.free_chunk(words, 8 * self.len());
        block.free_chunk(at, DESC_BYTES);
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
