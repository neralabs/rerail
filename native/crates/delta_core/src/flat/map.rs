//! Hash tables in an arena, open addressing with bounded linear probes (`ukl/hash.h`, the dedup and key maps,
//! `BlockHash`). Every word they hold is 32 bits, so a kernel claims and reads them with the 32-bit atomics every
//! device has (Metal has no 64-bit compare-and-swap):
//!
//! - [`Map`]: keys and values in two arrays of 2^k slots, a slot free while its key is the map's empty key. Many lanes
//!   claim keys at once by CAS; `claim_min` keeps each key's least value, so the contents do not depend on the order
//!   lanes ran in. One owner removes by backward shift. A key wider than a word is claimed by its row's index
//!   (`claim_by`): the slot holds the index of the first row that claimed it, hashed and compared by the row's values,
//!   which lie where they are and do not change while the map is claimed (the dedup's rows, a column's distinct values).
//! - [`Stamped`]: a slot live only while its stamp is its arena's round (`batch_runtime`'s tables), so moving the
//!   round on (`Arena::next_round`) empties it without a write. One writer (a lane's own table, as `batch_runtime`'s
//!   are a lane's evaluation's): its stamp and its key are two words, written by that writer alone.
//!
//! A key that finds no free slot within the probe bound fails as a value (`Full`): the owner makes a larger table and
//! a rehash phase moves the entries into it (`rehash_into`). Keys and values are words, held in atomics, so every read
//! and write is through `&self`.

use super::arena::{Arena, Full};
use super::grow::{addr, point};
use super::{B32, Rel};
use crate::Pod;
use crate::hash::{SEED, mix};
use core::marker::PhantomData;
use core::sync::atomic::AtomicU32;
use core::sync::atomic::Ordering::{AcqRel, Acquire, Relaxed, Release};

/// A 32-bit word a table holds in an atomic: a key (compared, hashed) or a value (stored, kept least).
pub trait Word: Copy + Eq + 'static {
    fn bits(self) -> u32;
    fn from_bits(b: u32) -> Self;
    /// Its hash: the CLIF's `rel_key_mix` of the word, its low 32 bits (`ukl_flat_hash_u32`).
    #[inline(always)]
    fn hash(self) -> u32 {
        hash_word(self.bits() as i64)
    }
}

/// A value kept least by `claim_min`: its order is its bits' as unsigned.
pub trait Least: Word + Ord {}

impl Word for u32 {
    #[inline(always)]
    fn bits(self) -> u32 {
        self
    }
    #[inline(always)]
    fn from_bits(b: u32) -> u32 {
        b
    }
}
impl Least for u32 {}

/// A value's hash in a table: `mix(SEED, x)`'s low 32 bits (`ukl_flat_hash_u32`, `ukl_flat_hash_i64`).
#[inline(always)]
pub fn hash_word(x: i64) -> u32 {
    mix(SEED, x) as u32
}

#[inline(always)]
fn load<W: Word>(a: &AtomicU32) -> W {
    W::from_bits(a.load(Acquire))
}

#[inline(always)]
fn store<W: Word>(a: &AtomicU32, v: W) {
    a.store(v.bits(), Release)
}

/// Probes a key takes at most before its table is full.
pub const PROBES: u32 = 64;

crate::structure! {
    /// A map's header, any key and value words.
    pub struct MapHead {
        pub keys: Rel<u8, B32>,
        pub vals: Rel<u8, B32>,
        /// Slots less one: a power of two less one.
        pub mask: u32,
        /// Probes a key takes at most.
        pub bound: u32,
        /// Keys claimed.
        pub len: AtomicU32,
        pub part: u32,
        /// The empty key.
        pub empty: u32,
        pub pad: u32,
    }
}

/// Keys `K` to values `V`, in an arena.
#[repr(transparent)]
pub struct Map<K, V> {
    head: MapHead,
    _kv: PhantomData<fn() -> (K, V)>,
}

// SAFETY: transparent over its header.
unsafe impl<K: 'static, V: 'static> Pod for Map<K, V> {}

/// A claim's slot, and whether this claim put its key there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Claimed {
    pub slot: u32,
    pub fresh: bool,
}

/// Slots for `n` keys, half of them free: a power of two, at least 8.
pub fn slots_for(n: usize) -> usize {
    n.saturating_mul(2).max(8).next_power_of_two()
}

impl<K: Word, V: Word> Map<K, V> {
    /// A map with room for `n` keys (twice that many slots), its keys `empty` and its values `init`, in `a`.
    pub fn make<'a>(a: &'a Arena<'_>, part: u32, n: usize, empty: K, init: V) -> Result<&'a mut Map<K, V>, Full> {
        let slots = slots_for(n);
        let m = a.claim::<Map<K, V>>(part, 1)?;
        let m = m[0].write(Map {
            head: MapHead {
                keys: Rel::none(),
                vals: Rel::none(),
                mask: (slots - 1) as u32,
                bound: PROBES.min(slots as u32),
                len: AtomicU32::new(0),
                part,
                empty: empty.bits(),
                pad: 0,
            },
            _kv: PhantomData,
        });
        let keys = a.claim::<AtomicU32>(part, slots)?;
        for k in keys.iter_mut() {
            k.write(AtomicU32::new(empty.bits()));
        }
        let ka = (keys.as_ptr().addr()) - a.region().addr();
        let vals = a.claim::<AtomicU32>(part, slots)?;
        for v in vals.iter_mut() {
            v.write(AtomicU32::new(init.bits()));
        }
        let va = (vals.as_ptr().addr()) - a.region().addr();
        point(&m.head.keys, a, ka);
        point(&m.head.vals, a, va);
        Ok(m)
    }

    pub fn head(&self) -> &MapHead {
        &self.head
    }

    pub fn empty(&self) -> K {
        K::from_bits(self.head.empty)
    }

    pub fn slots(&self) -> usize {
        self.head.mask as usize + 1
    }

    pub fn len(&self) -> usize {
        self.head.len.load(Acquire) as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline(always)]
    fn key(&self, s: u32) -> &AtomicU32 {
        // SAFETY: slot `s` (masked by every caller) of its keys, which are atomics written at `make`.
        unsafe { &*core::ptr::with_exposed_provenance::<AtomicU32>(addr(&self.head.keys)).add(s as usize) }
    }

    #[inline(always)]
    fn val(&self, s: u32) -> &AtomicU32 {
        // SAFETY: as `key`'s.
        unsafe { &*core::ptr::with_exposed_provenance::<AtomicU32>(addr(&self.head.vals)).add(s as usize) }
    }

    fn full(&self) -> Full {
        let bytes = (self.slots() as u64 * 2) * 8;
        Full { part: self.head.part, need: 1, wanted: bytes.min(u32::MAX as u64) as u32 }
    }

    /// `k`'s slot, claiming a free one for it if it has none: shared by every lane of a phase.
    #[inline]
    pub fn claim(&self, k: K) -> Result<Claimed, Full> {
        self.claim_by(k, k.hash(), |j| j == k)
    }

    /// The slot of the key `k` stands for, hashed `h`, claiming a free one for `k` if none holds a key `same` says is
    /// equal: shared by every lane of a phase. `k` is a row's index whose values `h` and `same` read (a key wider than a
    /// word); the slot keeps the first row's that claimed it. Every lane hashes and compares one key alike.
    pub fn claim_by(&self, k: K, h: u32, same: impl Fn(K) -> bool) -> Result<Claimed, Full> {
        let empty = self.empty();
        assert!(k != empty, "the map's empty key is not a key");
        for i in 0..self.head.bound {
            let s = h.wrapping_add(i) & self.head.mask;
            let at = self.key(s);
            let cur: K = load(at);
            if cur == empty {
                match at.compare_exchange(empty.bits(), k.bits(), AcqRel, Acquire) {
                    Ok(_) => {
                        self.head.len.fetch_add(1, Relaxed);
                        return Ok(Claimed { slot: s, fresh: true });
                    }
                    Err(now) if same(K::from_bits(now)) => return Ok(Claimed { slot: s, fresh: false }),
                    Err(_) => {}
                }
            } else if same(cur) {
                return Ok(Claimed { slot: s, fresh: false });
            }
        }
        Err(self.full())
    }

    /// `k`'s slot, or none.
    #[inline]
    pub fn find(&self, k: K) -> Option<u32> {
        self.find_by(k.hash(), |j| j == k)
    }

    /// The slot of a key hashed `h` that `same` says is the one looked for, or none (`claim_by`'s).
    pub fn find_by(&self, h: u32, same: impl Fn(K) -> bool) -> Option<u32> {
        let empty = self.empty();
        for i in 0..self.head.bound {
            let s = h.wrapping_add(i) & self.head.mask;
            let cur: K = load(self.key(s));
            if cur == empty {
                return None;
            }
            if same(cur) {
                return Some(s);
            }
        }
        None
    }

    /// `k`'s value, or none.
    pub fn get(&self, k: K) -> Option<V> {
        self.find(k).map(|s| load(self.val(s)))
    }

    /// Sets `k`'s value (the last writer's stays).
    pub fn set(&self, k: K, v: V) -> Result<Claimed, Full> {
        let c = self.claim(k)?;
        store(self.val(c.slot), v);
        Ok(c)
    }

    /// The value at slot `s`.
    pub fn val_at(&self, s: u32) -> V {
        load(self.val(s & self.head.mask))
    }

    /// The key and value at slot `s`, if it holds one.
    pub fn at(&self, s: u32) -> Option<(K, V)> {
        let k: K = load(self.key(s & self.head.mask));
        (k != self.empty()).then(|| (k, load(self.val(s & self.head.mask))))
    }

    /// Its entries, in slot order.
    pub fn iter(&self) -> impl Iterator<Item = (K, V)> + '_ {
        (0..self.slots() as u32).filter_map(|s| self.at(s))
    }

    /// Moves slots `from..to`'s entries into `into` (a rehash phase's lane's share; the lanes' shares cover every
    /// slot). Values set as they lie. Keys claimed by their own hash (`claim`, not `claim_by`).
    pub fn rehash_into(&self, into: &Map<K, V>, from: u32, to: u32) -> Result<(), Full> {
        for s in from..to.min(self.slots() as u32) {
            if let Some((k, v)) = self.at(s) {
                into.set(k, v)?;
            }
        }
        Ok(())
    }

    /// Takes `k` out, its followers shifted back so every probe still finds them (one owner; keys claimed by their own
    /// hash).
    pub fn remove(&mut self, k: K) -> Option<V> {
        let mut hole = self.find(k)?;
        let v = load(self.val(hole));
        let empty = self.empty();
        let mask = self.head.mask;
        let mut j = hole;
        loop {
            j = (j + 1) & mask;
            let kj: K = load(self.key(j));
            if kj == empty {
                break;
            }
            let home = kj.hash() & mask;
            // `kj` may fill the hole when the hole lies on its probe path: between its home and its slot.
            if (j.wrapping_sub(home) & mask) >= (j.wrapping_sub(hole) & mask) {
                store(self.key(hole), kj);
                store(self.val(hole), load::<V>(self.val(j)));
                hole = j;
            }
        }
        store(self.key(hole), empty);
        self.head.len.fetch_sub(1, Relaxed);
        Some(v)
    }
}

impl<K: Word, V: Least> Map<K, V> {
    /// Claims `k` and keeps its least value, `v` or one before it: the same contents in any order of lanes. The map's
    /// `init` must be past every value (its maximum).
    pub fn claim_min(&self, k: K, v: V) -> Result<Claimed, Full> {
        let c = self.claim(k)?;
        self.val(c.slot).fetch_min(v.bits(), AcqRel);
        Ok(c)
    }

    /// `claim_by` keeping the key's least value, `v` or one before it (`claim_min`'s): the dedup's least row of each
    /// key, the same contents in any order of lanes, whichever row's index the slot holds.
    pub fn claim_min_by(&self, k: K, h: u32, same: impl Fn(K) -> bool, v: V) -> Result<Claimed, Full> {
        let c = self.claim_by(k, h, same)?;
        self.val(c.slot).fetch_min(v.bits(), AcqRel);
        Ok(c)
    }
}

crate::structure! {
    /// A stamped table's header.
    pub struct StampedHead {
        /// Each slot's stamp: the round that wrote it (0: none).
        pub stamps: Rel<u8, B32>,
        pub keys: Rel<u8, B32>,
        pub vals: Rel<u8, B32>,
        pub mask: u32,
        pub bound: u32,
        pub part: u32,
        pub pad: [u32; 2],
    }
}

/// Keys `K` to values `V`, live for its arena's round: `Arena::next_round` empties it without a write. One writer.
#[repr(transparent)]
pub struct Stamped<K, V> {
    head: StampedHead,
    _kv: PhantomData<fn() -> (K, V)>,
}

// SAFETY: transparent over its header.
unsafe impl<K: 'static, V: 'static> Pod for Stamped<K, V> {}

impl<K: Word, V: Word> Stamped<K, V> {
    /// A table with room for `n` keys per round, in `a`, kept across rounds: made below the mark a round keeps.
    pub fn make<'a>(a: &'a Arena<'_>, part: u32, n: usize) -> Result<&'a Stamped<K, V>, Full> {
        let slots = slots_for(n);
        let t = a.claim::<Stamped<K, V>>(part, 1)?;
        let t = &*t[0].write(Stamped {
            head: StampedHead { stamps: Rel::none(), keys: Rel::none(), vals: Rel::none(), mask: (slots - 1) as u32, bound: PROBES.min(slots as u32), part, pad: [0; 2] },
            _kv: PhantomData,
        });
        // Stamp 0 is no round's: rounds start at 1. Keys and values are read only under a live stamp.
        let stamps = a.claim_with::<u32>(part, slots, 0)?;
        let sa = stamps.as_ptr().addr() - a.region().addr();
        let keys = a.claim::<AtomicU32>(part, slots)?;
        let ka = keys.as_ptr().addr() - a.region().addr();
        let vals = a.claim::<AtomicU32>(part, slots)?;
        let va = vals.as_ptr().addr() - a.region().addr();
        point(&t.head.stamps, a, sa);
        point(&t.head.keys, a, ka);
        point(&t.head.vals, a, va);
        Ok(t)
    }

    pub fn head(&self) -> &StampedHead {
        &self.head
    }

    #[inline(always)]
    fn word(&self, r: &Rel<u8, B32>, s: u32) -> &AtomicU32 {
        // SAFETY: slot `s` (masked by every caller) of one of its arrays, words made at `make`.
        unsafe { &*core::ptr::with_exposed_provenance::<AtomicU32>(addr(r)).add(s as usize) }
    }

    /// `k`'s slot this round, claiming one (stale, or never used) if it has none. A fresh claim's value is the
    /// claimer's to write (`set_at`); a slot found was claimed this round. Its one writer's.
    pub fn claim(&self, a: &Arena<'_>, k: K) -> Result<Claimed, Full> {
        let round = a.round();
        let h = k.hash();
        for i in 0..self.head.bound {
            let s = h.wrapping_add(i) & self.head.mask;
            let stamp = self.word(&self.head.stamps, s);
            if stamp.load(Acquire) != round {
                // Its key first, then its stamp: a reader that sees the stamp sees the key.
                store(self.word(&self.head.keys, s), k);
                stamp.store(round, Release);
                return Ok(Claimed { slot: s, fresh: true });
            }
            if load::<K>(self.word(&self.head.keys, s)) == k {
                return Ok(Claimed { slot: s, fresh: false });
            }
        }
        let bytes = (self.head.mask as u64 + 1) * 2 * 12;
        Err(Full { part: self.head.part, need: 1, wanted: bytes.min(u32::MAX as u64) as u32 })
    }

    /// `k`'s slot this round, or none.
    pub fn find(&self, a: &Arena<'_>, k: K) -> Option<u32> {
        let round = a.round();
        let h = k.hash();
        for i in 0..self.head.bound {
            let s = h.wrapping_add(i) & self.head.mask;
            if self.word(&self.head.stamps, s).load(Acquire) != round {
                return None;
            }
            if load::<K>(self.word(&self.head.keys, s)) == k {
                return Some(s);
            }
        }
        None
    }

    pub fn set_at(&self, slot: u32, v: V) {
        store(self.word(&self.head.vals, slot & self.head.mask), v)
    }

    /// `k`'s value this round, or none.
    pub fn get(&self, a: &Arena<'_>, k: K) -> Option<V> {
        self.find(a, k).map(|s| load(self.word(&self.head.vals, s)))
    }

    /// Keys live this round.
    pub fn live(&self, a: &Arena<'_>) -> usize {
        let round = a.round();
        (0..=self.head.mask).filter(|&s| self.word(&self.head.stamps, s).load(Acquire) == round).count()
    }
}
