//! A blocked Bloom filter's format, its hash and its reads (moved from `rerail_blocks::bloom`, whose writers place
//! a filter in a block and give it back).
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
//! back by `rerail_blocks::bloom::BloomW::release`.

use crate::flat::RelSpan;
use crate::structure;
use core::sync::atomic::{AtomicU64, Ordering::Relaxed};

structure! {
    /// A filter's words, `words.len()` of them, a power of two (0: a filter that holds every key).
    pub struct Bloom {
        pub words: RelSpan<u64>,
    }
}

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

/// A key of several columns as a filter places it (decision 0706 E2): one column's value is hashed as `hash` hashes it
/// (so a one-column relation filter places keys as the kept column filters do); several are folded by the hashed
/// index's key hash first (`crate::hash::key_hash`), then hashed so.
#[inline]
pub fn key(vals: &[i64]) -> u64 {
    match vals {
        [v] => hash(*v),
        _ => hash(crate::hash::key_hash(vals) as i64),
    }
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
        unsafe { core::slice::from_raw_parts(self.words.as_ptr().cast::<AtomicU64>(), self.len()) }
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
