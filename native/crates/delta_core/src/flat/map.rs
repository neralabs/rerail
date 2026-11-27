//! Hash tables in an arena, open addressing with bounded linear probes (`ukl/hash.h`, the dedup and key maps,
//! `BlockHash`):
//!
//! - [`Map`]: keys and values in two arrays of 2^k slots, a slot free while its key is the map's empty key. Many lanes
//!   claim keys at once by CAS; `claim_min` keeps each key's least value, so the contents do not depend on the order
//!   lanes ran in. One owner removes by backward shift.
//! - [`Stamped`]: a slot live only while its stamp is its arena's round (`batch_runtime`'s tables), so moving the
//!   round on (`Arena::next_round`) empties it without a write.
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
use core::sync::atomic::Ordering::{AcqRel, Acquire, Relaxed};
use core::sync::atomic::{AtomicU32, AtomicU64};

/// A word a table holds in an atomic: a key (compared, hashed) or a value (stored, kept least).
pub trait Word: Copy + Eq + 'static {
    type Atomic: Pod;
    fn new(v: Self) -> Self::Atomic;
    fn load(a: &Self::Atomic) -> Self;
    fn store(a: &Self::Atomic, v: Self);
    fn cas(a: &Self::Atomic, old: Self, new: Self) -> Result<Self, Self>;
    fn hash(self) -> u64;
    /// As two words (a header holds a map's empty key in them).
    fn to_u64(self) -> u64;
    fn from_u64(v: u64) -> Self;
}

/// A value kept least by `claim_min`.
pub trait Least: Word + Ord {
    fn fetch_min(a: &Self::Atomic, v: Self) -> Self;
}

macro_rules! word {
    ($t:ty, $a:ty) => {
        impl Word for $t {
            type Atomic = $a;
            #[inline(always)]
            fn new(v: $t) -> $a {
                <$a>::new(v)
            }
            #[inline(always)]
            fn load(a: &$a) -> $t {
                a.load(Acquire)
            }
            #[inline(always)]
            fn store(a: &$a, v: $t) {
                a.store(v, core::sync::atomic::Ordering::Release)
            }
            #[inline(always)]
            fn cas(a: &$a, old: $t, new: $t) -> Result<$t, $t> {
                a.compare_exchange(old, new, AcqRel, Acquire)
            }
            #[inline(always)]
            fn hash(self) -> u64 {
                mix(SEED, self as i64)
            }
            fn to_u64(self) -> u64 {
                self as u64
            }
            fn from_u64(v: u64) -> $t {
                v as $t
            }
        }
        impl Least for $t {
            #[inline(always)]
            fn fetch_min(a: &$a, v: $t) -> $t {
                a.fetch_min(v, AcqRel)
            }
        }
    };
}
word!(u32, AtomicU32);
word!(u64, AtomicU64);

/// A pair of words as one key (a row's two columns), held in one 64-bit atomic: compared and hashed whole, not
/// ordered, so not a `Least` value.
impl Word for [u32; 2] {
    type Atomic = AtomicU64;
    #[inline(always)]
    fn new(v: [u32; 2]) -> AtomicU64 {
        AtomicU64::new(v.to_u64())
    }
    #[inline(always)]
    fn load(a: &AtomicU64) -> [u32; 2] {
        Self::from_u64(a.load(Acquire))
    }
    #[inline(always)]
    fn store(a: &AtomicU64, v: [u32; 2]) {
        a.store(v.to_u64(), core::sync::atomic::Ordering::Release)
    }
    #[inline(always)]
    fn cas(a: &AtomicU64, old: [u32; 2], new: [u32; 2]) -> Result<[u32; 2], [u32; 2]> {
        a.compare_exchange(old.to_u64(), new.to_u64(), AcqRel, Acquire).map(Self::from_u64).map_err(Self::from_u64)
    }
    #[inline(always)]
    fn hash(self) -> u64 {
        mix(mix(SEED, self[0] as i64), self[1] as i64)
    }
    fn to_u64(self) -> u64 {
        self[0] as u64 | (self[1] as u64) << 32
    }
    fn from_u64(v: u64) -> [u32; 2] {
        [v as u32, (v >> 32) as u32]
    }
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
        /// The empty key, as two words.
        pub empty_lo: u32,
        pub empty_hi: u32,
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
        let e = empty.to_u64();
        let m = m[0].write(Map {
            head: MapHead {
                keys: Rel::none(),
                vals: Rel::none(),
                mask: (slots - 1) as u32,
                bound: PROBES.min(slots as u32),
                len: AtomicU32::new(0),
                part,
                empty_lo: e as u32,
                empty_hi: (e >> 32) as u32,
            },
            _kv: PhantomData,
        });
        let keys = a.claim::<K::Atomic>(part, slots)?;
        for k in keys.iter_mut() {
            k.write(K::new(empty));
        }
        let ka = (keys.as_ptr().addr()) - a.region().addr();
        let vals = a.claim::<V::Atomic>(part, slots)?;
        for v in vals.iter_mut() {
            v.write(V::new(init));
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
        K::from_u64(self.head.empty_lo as u64 | (self.head.empty_hi as u64) << 32)
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
    fn key(&self, s: u32) -> &K::Atomic {
        // SAFETY: slot `s` (masked by every caller) of its keys, which are atomics written at `make`.
        unsafe { &*core::ptr::with_exposed_provenance::<K::Atomic>(addr(&self.head.keys)).add(s as usize) }
    }

    #[inline(always)]
    fn val(&self, s: u32) -> &V::Atomic {
        // SAFETY: as `key`'s.
        unsafe { &*core::ptr::with_exposed_provenance::<V::Atomic>(addr(&self.head.vals)).add(s as usize) }
    }

    fn full(&self) -> Full {
        let bytes = (self.slots() as u64 * 2) * (size_of::<K::Atomic>() + size_of::<V::Atomic>()) as u64;
        Full { part: self.head.part, need: 1, wanted: bytes.min(u32::MAX as u64) as u32 }
    }

    /// `k`'s slot, claiming a free one for it if it has none: shared by every lane of a phase.
    pub fn claim(&self, k: K) -> Result<Claimed, Full> {
        let empty = self.empty();
        assert!(k != empty, "the map's empty key is not a key");
        let h = k.hash() as u32;
        for i in 0..self.head.bound {
            let s = h.wrapping_add(i) & self.head.mask;
            let at = self.key(s);
            let cur = K::load(at);
            if cur == k {
                return Ok(Claimed { slot: s, fresh: false });
            }
            if cur == empty {
                match K::cas(at, empty, k) {
                    Ok(_) => {
                        self.head.len.fetch_add(1, Relaxed);
                        return Ok(Claimed { slot: s, fresh: true });
                    }
                    Err(now) if now == k => return Ok(Claimed { slot: s, fresh: false }),
                    Err(_) => {}
                }
            }
        }
        Err(self.full())
    }

    /// `k`'s slot, or none.
    pub fn find(&self, k: K) -> Option<u32> {
        let empty = self.empty();
        let h = k.hash() as u32;
        for i in 0..self.head.bound {
            let s = h.wrapping_add(i) & self.head.mask;
            let cur = K::load(self.key(s));
            if cur == k && k != empty {
                return Some(s);
            }
            if cur == empty {
                return None;
            }
        }
        None
    }

    /// `k`'s value, or none.
    pub fn get(&self, k: K) -> Option<V> {
        self.find(k).map(|s| V::load(self.val(s)))
    }

    /// Sets `k`'s value (the last writer's stays).
    pub fn set(&self, k: K, v: V) -> Result<Claimed, Full> {
        let c = self.claim(k)?;
        V::store(self.val(c.slot), v);
        Ok(c)
    }

    /// The key and value at slot `s`, if it holds one.
    pub fn at(&self, s: u32) -> Option<(K, V)> {
        let k = K::load(self.key(s & self.head.mask));
        (k != self.empty()).then(|| (k, V::load(self.val(s & self.head.mask))))
    }

    /// Its entries, in slot order.
    pub fn iter(&self) -> impl Iterator<Item = (K, V)> + '_ {
        (0..self.slots() as u32).filter_map(|s| self.at(s))
    }

    /// Moves slots `from..to`'s entries into `into` (a rehash phase's lane's share; the lanes' shares cover every
    /// slot). Values set as they lie.
    pub fn rehash_into(&self, into: &Map<K, V>, from: u32, to: u32) -> Result<(), Full> {
        for s in from..to.min(self.slots() as u32) {
            if let Some((k, v)) = self.at(s) {
                into.set(k, v)?;
            }
        }
        Ok(())
    }

    /// Takes `k` out, its followers shifted back so every probe still finds them (one owner).
    pub fn remove(&mut self, k: K) -> Option<V> {
        let mut hole = self.find(k)?;
        let v = V::load(self.val(hole));
        let empty = self.empty();
        let mask = self.head.mask;
        let mut j = hole;
        loop {
            j = (j + 1) & mask;
            let kj = K::load(self.key(j));
            if kj == empty {
                break;
            }
            let home = kj.hash() as u32 & mask;
            // `kj` may fill the hole when the hole lies on its probe path: between its home and its slot.
            if (j.wrapping_sub(home) & mask) >= (j.wrapping_sub(hole) & mask) {
                K::store(self.key(hole), kj);
                V::store(self.val(hole), V::load(self.val(j)));
                hole = j;
            }
        }
        K::store(self.key(hole), empty);
        self.head.len.fetch_sub(1, Relaxed);
        Some(v)
    }
}

impl<K: Word, V: Least> Map<K, V> {
    /// Claims `k` and keeps its least value, `v` or one before it: the same contents in any order of lanes. The map's
    /// `init` must be past every value (its maximum).
    pub fn claim_min(&self, k: K, v: V) -> Result<Claimed, Full> {
        let c = self.claim(k)?;
        V::fetch_min(self.val(c.slot), v);
        Ok(c)
    }
}

crate::structure! {
    /// A stamped table's header.
    pub struct StampedHead {
        /// Each slot's stamp and key: the round in the high word, the key in the low.
        pub slots: Rel<u8, B32>,
        pub vals: Rel<u8, B32>,
        pub mask: u32,
        pub bound: u32,
        pub part: u32,
        pub pad: u32,
    }
}

/// `u32` keys to values `V`, live for its arena's round: `Arena::next_round` empties it without a write.
#[repr(transparent)]
pub struct Stamped<V> {
    head: StampedHead,
    _v: PhantomData<fn() -> V>,
}

// SAFETY: transparent over its header.
unsafe impl<V: 'static> Pod for Stamped<V> {}

impl<V: Word> Stamped<V> {
    /// A table with room for `n` keys per round, in `a`, kept across rounds: made below the mark a round keeps.
    pub fn make<'a>(a: &'a Arena<'_>, part: u32, n: usize) -> Result<&'a Stamped<V>, Full> {
        let slots = slots_for(n);
        let t = a.claim::<Stamped<V>>(part, 1)?;
        let t = &*t[0].write(Stamped {
            head: StampedHead { slots: Rel::none(), vals: Rel::none(), mask: (slots - 1) as u32, bound: PROBES.min(slots as u32), part, pad: 0 },
            _v: PhantomData,
        });
        // Stamp 0 is no round's: rounds start at 1.
        let words = a.claim_with::<u64>(part, slots, 0)?;
        let sa = words.as_ptr().addr() - a.region().addr();
        let vals = a.claim::<V::Atomic>(part, slots)?;
        let va = vals.as_ptr().addr() - a.region().addr();
        point(&t.head.slots, a, sa);
        point(&t.head.vals, a, va);
        Ok(t)
    }

    pub fn head(&self) -> &StampedHead {
        &self.head
    }

    #[inline(always)]
    fn slot(&self, s: u32) -> &AtomicU64 {
        // SAFETY: slot `s` (masked) of its stamp words.
        unsafe { &*core::ptr::with_exposed_provenance::<AtomicU64>(addr(&self.head.slots)).add(s as usize) }
    }

    #[inline(always)]
    fn val(&self, s: u32) -> &V::Atomic {
        // SAFETY: as `slot`'s.
        unsafe { &*core::ptr::with_exposed_provenance::<V::Atomic>(addr(&self.head.vals)).add(s as usize) }
    }

    /// `k`'s slot this round, claiming one (stale, or never used) if it has none. A fresh claim's value is the
    /// claimer's to write (`set_at`); a slot found was claimed this round.
    pub fn claim(&self, a: &Arena<'_>, k: u32) -> Result<Claimed, Full> {
        let round = a.round() as u64;
        let mine = round << 32 | k as u64;
        let h = (k as u64).hash() as u32;
        for i in 0..self.head.bound {
            let s = h.wrapping_add(i) & self.head.mask;
            let at = self.slot(s);
            let mut cur = at.load(Acquire);
            loop {
                if cur == mine {
                    return Ok(Claimed { slot: s, fresh: false });
                }
                if cur >> 32 == round {
                    break;
                }
                match at.compare_exchange(cur, mine, AcqRel, Acquire) {
                    Ok(_) => return Ok(Claimed { slot: s, fresh: true }),
                    // Another claim took it this round (for `k`, or another key: the loop's first tests say), or it
                    // is still stale.
                    Err(now) => cur = now,
                }
            }
        }
        let bytes = (self.head.mask as u64 + 1) * 2 * (8 + size_of::<V::Atomic>()) as u64;
        Err(Full { part: self.head.part, need: 1, wanted: bytes.min(u32::MAX as u64) as u32 })
    }

    /// `k`'s slot this round, or none.
    pub fn find(&self, a: &Arena<'_>, k: u32) -> Option<u32> {
        let round = a.round() as u64;
        let mine = round << 32 | k as u64;
        let h = (k as u64).hash() as u32;
        for i in 0..self.head.bound {
            let s = h.wrapping_add(i) & self.head.mask;
            let cur = self.slot(s).load(Acquire);
            if cur == mine {
                return Some(s);
            }
            if cur >> 32 != round {
                return None;
            }
        }
        None
    }

    pub fn set_at(&self, slot: u32, v: V) {
        V::store(self.val(slot & self.head.mask), v)
    }

    /// `k`'s value this round, or none.
    pub fn get(&self, a: &Arena<'_>, k: u32) -> Option<V> {
        self.find(a, k).map(|s| V::load(self.val(s)))
    }

    /// Keys live this round.
    pub fn live(&self, a: &Arena<'_>) -> usize {
        let round = a.round() as u64;
        (0..=self.head.mask).filter(|&s| self.slot(s).load(Acquire) >> 32 == round).count()
    }
}
