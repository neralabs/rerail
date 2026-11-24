//! Generic structures in blocks, after MLDB's mapped structures (`mldb/plugins/behavior/mapped_value.h`,
//! `mldb/utils/lightweight_hash.h`, `mldb/plugins/tabular/frozen_tables.h`): each one region of a block (its
//! descriptor first, its arrays after it), reached by self-relative spans (`refs::Span`), so the host, the JIT and the
//! kernels read one layout where it lies. A [`Room`] hands out the region; a structure that outgrows it takes a
//! bigger one from the same room and gives the old one back to its block's free list, as a vector's growth does.
//!
//! - [`BlockHash`]: MLDB's `Lightweight_Hash` — open addressing, a power-of-two capacity, linear probing on the
//!   engine's key hash (`mix`), one guard key (all ones) held in a bucket of its own past the table, keys and values
//!   in two arrays (no padding between them), updated in place, removal by backward shift. [`BlockSet`]: no values.
//! - [`BlockSorted`]: MLDB's `MappedSortedKeyValueArray` — a frozen map: keys sorted, values beside, found by binary
//!   search.
//! - [`BlockStrings`]: MLDB's `FrozenBlobTable` — each string's end offset, and the bytes; appended, read by index.
//! - [`BlockInterner`]: strings and a hash table of their indexes (MLDB's string table): a string's index, found by
//!   its bytes.
//! - [`Rec`]: one record (`structure!` type) in its region, with its children (spans of records) after it.
//!
//! Integer arrays frozen in the fewest bits are `col::IntCol` (MLDB's `RawMappedIntTable`); growable arrays are
//! `rerail_vector::vector::Vector`, in the region a room gives (a relation's block: the facts layer's `Home`).

use crate::block::Block;
use crate::refs::Span;
use crate::region::Mutable;
use crate::{Pod, structure};
use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;


/// What keeps a region's block leased while a structure holds the region (a facts layer's block handle).
pub use rerail_vector::{Guard, Keep, Region, Room};

/// A block's region as a room gives it (`Region`'s owner): the region, and what keeps its block.
pub struct Held(pub Mutable<u8>, pub Keep);

/// `m` (and what keeps its block) as a room's region.
pub fn region_of(mut m: Mutable<u8>, keep: Keep) -> Region {
    let (ptr, len) = (m.as_mut_ptr(), m.region().len());
    // SAFETY: the region's bytes, held by `Held` while the region lives.
    unsafe { Region::new(ptr, len, Box::new(Held(m, keep))) }
}

/// Where a room's region lies: its block and its first byte's offset there (none for a region not in a block, a
/// crate's own test's on the heap).
pub fn place_of(r: &Region) -> Option<(&Arc<Block>, usize)> {
    r.owner().downcast_ref::<Held>().map(|h| (h.0.region().block(), h.0.region().at()))
}

/// Where a vector lies (`rerail_vector::vector::Vector`): its block and its first `T`'s offset there.
pub trait Placed {
    fn place(&self) -> Option<(&Arc<Block>, usize)>;
}
impl<T: Pod> Placed for crate::vector::Vector<T> {
    fn place(&self) -> Option<(&Arc<Block>, usize)> {
        self.region().and_then(place_of)
    }
}

/// One block, which must hold everything put in it.
pub struct InBlock(pub Arc<Block>);

impl Room for InBlock {
    fn region(&self, bytes: usize, align: usize, name: &'static str) -> Region {
        region_of(Mutable::alloc_aligned(&self.0, bytes, align, name).unwrap_or_else(|| panic!("block full: {bytes} bytes for {name}")), None)
    }
}

/// A run's room for the structures no life places (`default_room`): blocks of its own, each with a free list, the
/// next leased when one is full; a block goes when its last region does. A shard's block a thread at a time: each
/// thread takes its regions from its own shard's (`shard`), as an allocator's arenas are. With one block for the run,
/// its every small array (a plan's steps, a body's variable sets, a section's lists) waited on one lock, and the run's
/// stream workers spent most of an edit's time there.
#[derive(Default)]
pub struct AnonRoom {
    open: [std::sync::Mutex<Option<Arc<Block>>>; ANON_SHARDS],
}

/// The bytes an anonymous room's block reserves (address space; pages are touched as used).
const ANON_BLOCK: usize = 64 << 20;

/// An anonymous room's shards: a block each, reserved when a thread first takes a region there.
const ANON_SHARDS: usize = 16;

/// The calling thread's shard of an anonymous room: threads take the shards in turn as each first asks.
fn shard() -> usize {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    thread_local! {
        static SHARD: usize = NEXT.fetch_add(1, Ordering::Relaxed) % ANON_SHARDS;
    }
    SHARD.with(|s| *s)
}

impl Room for AnonRoom {
    fn region(&self, bytes: usize, align: usize, name: &'static str) -> Region {
        let mut open = self.open[shard()].lock().unwrap_or_else(|e| e.into_inner());
        if let Some(b) = &*open
            && let Some(r) = Mutable::alloc_aligned(b, bytes, align, name)
        {
            return region_of(r, None);
        }
        // Room for the chunk the free list takes for `bytes` (its class, a power of two), aligned to its size up to
        // `ALIGN_MAX` past the block's first byte, and a page.
        let size = crate::free::class_of(bytes).1;
        let need = size + align.max(size.min(crate::free::ALIGN_MAX)) + 4096;
        let b = Arc::new(Block::anon(crate::block::kind::FACTS, ANON_BLOCK.max(need)).with_free_list());
        let r = Mutable::alloc_aligned(&b, bytes, align, name).unwrap_or_else(|| panic!("a new block holds no {bytes} bytes for {name}"));
        *open = Some(b);
        region_of(r, None)
    }
}

/// The run's own room (`rerail_context::local`): where a structure made with no room (`Default`) lives.
#[derive(Default)]
struct RunRoom(std::sync::OnceLock<Arc<dyn Room>>);

/// The room of the run under way (its `AnonRoom`; outside a run, its shared object's: `run_room`).
pub fn default_room() -> Arc<dyn Room> {
    run_room()
}

/// Installs the run's room as every crate's (`rerail_vector::default_room`): the engine's, at load (boot's, once a
/// process). A crate's own tests, run without it, make their vectors with no room on the heap.
pub fn install() {
    rerail_vector::install_default_room("rerail-blocks", run_room);
}

fn run_room() -> Arc<dyn Room> {
    // Outside a run (a test's thread, which calls the engine's parts directly), the room of the shared object such a
    // read takes (`rerail_context::shared`: the tests' one): a run's locals are made afresh at each read there, so a
    // room of its own each time would reserve a block for each structure, and drop it with the structure.
    let room = if rerail_context::run_own().is_some() { rerail_context::local::<RunRoom>() } else { rerail_context::shared::<RunRoom>() };
    room.0.get_or_init(|| Arc::new(AnonRoom::default())).clone()
}

/// One structure's region: its first byte, kept beside the handle so a read is one load.
struct Mem {
    region: Region,
    base: *mut u8,
}

impl Mem {
    fn new(room: &dyn Room, bytes: usize, align: usize, name: &'static str) -> Mem {
        let bytes = bytes.max(8);
        let mut region = room.region(bytes, align, name);
        let base = region.as_mut_ptr();
        // SAFETY: the region's `bytes`, this structure's alone: zeroed, as a chunk from a free list may not be.
        unsafe { std::ptr::write_bytes(base, 0, bytes) };
        Mem { region, base }
    }

    #[inline]
    fn at<T>(&self, off: usize) -> *mut T {
        // SAFETY: an offset within its region, laid out by its structure.
        unsafe { self.base.add(off).cast::<T>() }
    }

    fn block(&self) -> &Arc<Block> {
        place_of(&self.region).expect("a structure's region lies in a block").0
    }

    fn offset(&self) -> usize {
        place_of(&self.region).expect("a structure's region lies in a block").1
    }
}

const fn up8(n: usize) -> usize {
    (n + 7) & !7
}

/// A value a map is keyed by: its bytes are its identity, and one value (`GUARD`, all ones) marks an empty bucket.
pub trait Key: Pod + Copy + Eq {
    const GUARD: Self;
    /// The engine's key hash (`crate::mix` over the key's 64-bit words), the same in UKL.
    fn hash(&self) -> u64;
}

macro_rules! key_words {
    ($($t:ty),*) => {$(
        impl Key for $t {
            const GUARD: Self = <$t>::MAX;
            #[inline]
            fn hash(&self) -> u64 {
                crate::mix(crate::SEED, *self as i64)
            }
        }
    )*};
}
key_words!(u32, u64, usize);

impl Key for i64 {
    const GUARD: Self = -1;
    #[inline]
    fn hash(&self) -> u64 {
        crate::mix(crate::SEED, *self)
    }
}

impl Key for i32 {
    const GUARD: Self = -1;
    #[inline]
    fn hash(&self) -> u64 {
        crate::mix(crate::SEED, i64::from(*self))
    }
}

impl<const N: usize> Key for [u32; N] {
    const GUARD: Self = [u32::MAX; N];
    #[inline]
    fn hash(&self) -> u64 {
        self.iter().fold(crate::SEED, |h, &w| crate::mix(h, i64::from(w)))
    }
}

impl<const N: usize> Key for [i64; N] {
    const GUARD: Self = [-1; N];
    #[inline]
    fn hash(&self) -> u64 {
        self.iter().fold(crate::SEED, |h, &w| crate::mix(h, w))
    }
}

impl<const N: usize> Key for [u64; N] {
    const GUARD: Self = [u64::MAX; N];
    #[inline]
    fn hash(&self) -> u64 {
        self.iter().fold(crate::SEED, |h, &w| crate::mix(h, w as i64))
    }
}

/// The engine's hash of a string's bytes: `mix` over its little-endian 8-byte words, then its length.
pub fn hash_bytes(b: &[u8]) -> u64 {
    let mut h = crate::SEED;
    let mut words = b.chunks_exact(8);
    for w in &mut words {
        h = crate::mix(h, i64::from_le_bytes(w.try_into().expect("8 bytes")));
    }
    let rest = words.remainder();
    if !rest.is_empty() {
        let mut w = [0u8; 8];
        // copy: not block data: a hash's last, partial word
        w[..rest.len()].copy_from_slice(rest);
        h = crate::mix(h, i64::from_le_bytes(w));
    }
    crate::mix(h, b.len() as i64)
}

structure! {
    /// A hash map's descriptor: `2^log` buckets (keys and values each an array, `kbytes` and `vbytes` an entry), then
    /// the guard key's bucket at index `2^log`, full when `guard` is 1. `len` counts the keys, the guard's too.
    pub struct HashDesc {
        pub log: u32,
        pub len: u32,
        pub kbytes: u32,
        pub vbytes: u32,
        pub guard: u32,
        pub pad: u32,
        pub keys: Span<u8>,
        pub vals: Span<u8>,
    }
}

/// A hash map in a block (MLDB's `Lightweight_Hash`): its descriptor and two arrays of `2^log + 1` entries each, in
/// one region.
pub struct BlockHash<K: Key, V: Pod + Copy> {
    room: Arc<dyn Room>,
    mem: Mem,
    _t: PhantomData<(K, V)>,
}

// SAFETY: its entries lie in the region it holds, written only through `&mut self`.
unsafe impl<K: Key + Send, V: Pod + Copy + Send> Send for BlockHash<K, V> {}
unsafe impl<K: Key + Sync, V: Pod + Copy + Sync> Sync for BlockHash<K, V> {}

impl<K: Key, V: Pod + Copy> BlockHash<K, V> {
    /// An empty map with room for `n` keys before it grows.
    pub fn with_capacity(room: Arc<dyn Room>, n: usize) -> Self {
        let log = (n.max(4) * 4 / 3 + 1).next_power_of_two().trailing_zeros();
        let mem = Self::make(&*room, log);
        BlockHash { room, mem, _t: PhantomData }
    }

    pub fn new(room: Arc<dyn Room>) -> Self {
        Self::with_capacity(room, 0)
    }

    /// A region of a descriptor and its arrays of `2^log + 1` entries, the keys all the guard (empty).
    fn make(room: &dyn Room, log: u32) -> Mem {
        let n = (1usize << log) + 1;
        let (kat, vat) = (up8(size_of::<HashDesc>()), up8(size_of::<HashDesc>()) + up8(n * size_of::<K>()));
        let mem = Mem::new(room, vat + n * size_of::<V>(), align_of::<K>().max(align_of::<V>()), "a hash map");
        let (d, keys, vals) = (mem.at::<HashDesc>(0), mem.at::<K>(kat), mem.at::<V>(vat));
        // SAFETY: the region was just taken, zeroed, for these.
        unsafe {
            for i in 0..n {
                keys.add(i).write(K::GUARD);
            }
            (*d).log = log;
            (*d).kbytes = size_of::<K>() as u32;
            (*d).vbytes = size_of::<V>() as u32;
            Span::set(&raw const (*d).keys, keys.cast::<u8>(), n * size_of::<K>());
            Span::set(&raw const (*d).vals, vals.cast::<u8>(), n * size_of::<V>());
        }
        mem
    }

    #[inline]
    fn desc(&self) -> &HashDesc {
        // SAFETY: its descriptor, first in its region.
        unsafe { &*self.mem.at::<HashDesc>(0) }
    }

    #[inline]
    fn d(&mut self) -> *mut HashDesc {
        self.mem.at::<HashDesc>(0)
    }

    #[inline]
    fn keys(&self) -> *mut K {
        self.desc().keys.as_ptr().cast::<K>().cast_mut()
    }

    #[inline]
    fn vals(&self) -> *mut V {
        self.desc().vals.as_ptr().cast::<V>().cast_mut()
    }

    #[inline]
    fn cap(&self) -> usize {
        1 << self.desc().log
    }

    /// Its block, and its descriptor's offset there: what a kernel's read takes.
    pub fn place(&self) -> (&Arc<Block>, usize) {
        (self.mem.block(), self.mem.offset())
    }

    pub fn len(&self) -> usize {
        self.desc().len as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The bucket `k` is in, or the empty one where it would go.
    #[inline]
    fn slot(&self, k: &K) -> (usize, bool) {
        if *k == K::GUARD {
            return (self.cap(), self.desc().guard == 1);
        }
        let mask = self.cap() - 1;
        let keys = self.keys();
        let mut i = k.hash() as usize & mask;
        loop {
            // SAFETY: `i` below the capacity.
            let at = unsafe { *keys.add(i) };
            if at == *k {
                return (i, true);
            }
            if at == K::GUARD {
                return (i, false);
            }
            i = (i + 1) & mask;
        }
    }

    pub fn get(&self, k: &K) -> Option<&V> {
        let (i, found) = self.slot(k);
        // SAFETY: a full bucket's value.
        found.then(|| unsafe { &*self.vals().add(i) })
    }

    pub fn get_mut(&mut self, k: &K) -> Option<&mut V> {
        let (i, found) = self.slot(k);
        // SAFETY: a full bucket's value, written only through `&mut self`.
        found.then(|| unsafe { &mut *self.vals().add(i) })
    }

    pub fn contains_key(&self, k: &K) -> bool {
        self.slot(k).1
    }

    /// `k`'s value set to `v`; its value before.
    pub fn insert(&mut self, k: K, v: V) -> Option<V> {
        if let Some(old) = self.get_mut(&k) {
            return Some(std::mem::replace(old, v));
        }
        self.add(k, v);
        None
    }

    /// `k`'s value, put first by `make` when it has none.
    pub fn get_or_insert_with(&mut self, k: K, make: impl FnOnce() -> V) -> &mut V {
        let (i, found) = self.slot(&k);
        let i = if found { i } else { self.add(k, make()) };
        // SAFETY: a full bucket's value, written only through `&mut self`.
        unsafe { &mut *self.vals().add(i) }
    }

    /// A key not held, added: the table doubles first past three quarters full. Its bucket.
    fn add(&mut self, k: K, v: V) -> usize {
        if k != K::GUARD && 4 * (self.len() + 1) > 3 * self.cap() {
            self.grow();
        }
        let (i, found) = self.slot(&k);
        debug_assert!(!found);
        let (keys, vals) = (self.keys(), self.vals());
        let d = self.d();
        // SAFETY: an empty bucket, or the guard's.
        unsafe {
            *keys.add(i) = k;
            *vals.add(i) = v;
            (*d).len += 1;
            if k == K::GUARD {
                (*d).guard = 1;
            }
        }
        i
    }

    /// Twice the buckets, every key placed again, in a new region; the old one goes back to its block.
    fn grow(&mut self) {
        let mut next = BlockHash::<K, V> { room: self.room.clone(), mem: Self::make(&*self.room, self.desc().log + 1), _t: PhantomData };
        for (k, v) in self.iter() {
            next.add(k, v);
        }
        *self = next;
    }

    /// `k` taken out (backward shift: the keys after it in its run move up, so no bucket is a tombstone). Its value.
    pub fn remove(&mut self, k: &K) -> Option<V> {
        let (mut i, found) = self.slot(k);
        if !found {
            return None;
        }
        let (keys, vals, mask) = (self.keys(), self.vals(), self.cap() - 1);
        let d = self.d();
        // SAFETY: buckets below the capacity, and the guard's.
        unsafe {
            let v = *vals.add(i);
            (*d).len -= 1;
            if *k == K::GUARD {
                (*d).guard = 0;
                return Some(v);
            }
            let mut j = i;
            loop {
                *keys.add(i) = K::GUARD;
                loop {
                    j = (j + 1) & mask;
                    let kj = *keys.add(j);
                    if kj == K::GUARD {
                        return Some(v);
                    }
                    let home = kj.hash() as usize & mask;
                    // `kj` stays when its home lies cyclically in (i, j].
                    let stays = if i <= j { i < home && home <= j } else { i < home || home <= j };
                    if !stays {
                        break;
                    }
                }
                *keys.add(i) = *keys.add(j);
                *vals.add(i) = *vals.add(j);
                i = j;
            }
        }
    }

    /// Every key and value, in bucket order.
    pub fn iter(&self) -> impl Iterator<Item = (K, V)> + '_ {
        let (keys, vals, cap) = (self.keys(), self.vals(), self.cap());
        let guard = self.desc().guard == 1;
        // SAFETY: buckets below the capacity, and the guard's when full.
        (0..cap)
            .filter_map(move |i| unsafe {
                let k = *keys.add(i);
                (k != K::GUARD).then(|| (k, *vals.add(i)))
            })
            .chain(guard.then(|| unsafe { (K::GUARD, *vals.add(cap)) }))
    }

    pub fn keys_iter(&self) -> impl Iterator<Item = K> + '_ {
        self.iter().map(|(k, _)| k)
    }

    pub fn values(&self) -> impl Iterator<Item = V> + '_ {
        self.iter().map(|(_, v)| v)
    }

    /// Each value, writable.
    pub fn for_each_mut(&mut self, mut f: impl FnMut(K, &mut V)) {
        let (keys, vals, cap) = (self.keys(), self.vals(), self.cap());
        let guard = self.desc().guard == 1;
        // SAFETY: full buckets, written only through `&mut self`.
        unsafe {
            for i in 0..cap {
                let k = *keys.add(i);
                if k != K::GUARD {
                    f(k, &mut *vals.add(i));
                }
            }
            if guard {
                f(K::GUARD, &mut *vals.add(cap));
            }
        }
    }

    /// Empty again, its capacity kept.
    pub fn clear(&mut self) {
        let (n, keys) = (self.cap() + 1, self.keys());
        let d = self.d();
        // SAFETY: its buckets.
        unsafe {
            for i in 0..n {
                *keys.add(i) = K::GUARD;
            }
            (*d).len = 0;
            (*d).guard = 0;
        }
    }

    /// The room it takes its regions from.
    pub fn room(&self) -> &Arc<dyn Room> {
        &self.room
    }
}

impl<K: Key, V: Pod + Copy> Default for BlockHash<K, V> {
    fn default() -> Self {
        BlockHash::new(default_room())
    }
}

impl<K: Key, V: Pod + Copy> FromIterator<(K, V)> for BlockHash<K, V> {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(items: I) -> Self {
        let items = items.into_iter();
        let mut m = BlockHash::with_capacity(default_room(), items.size_hint().0);
        m.extend(items);
        m
    }
}

impl<K: Key, V: Pod + Copy> Extend<(K, V)> for BlockHash<K, V> {
    fn extend<I: IntoIterator<Item = (K, V)>>(&mut self, items: I) {
        for (k, v) in items {
            self.insert(k, v);
        }
    }
}

impl<K: Key, V: Pod + Copy + PartialEq> PartialEq for BlockHash<K, V> {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().all(|(k, v)| other.get(&k) == Some(&v))
    }
}

/// A key's place in a [`BlockHash`], held or not (`HashMap::entry`'s).
pub struct Entry<'m, K: Key, V: Pod + Copy> {
    map: &'m mut BlockHash<K, V>,
    key: K,
}

impl<'m, K: Key, V: Pod + Copy> Entry<'m, K, V> {
    pub fn or_insert(self, v: V) -> &'m mut V {
        self.map.get_or_insert_with(self.key, || v)
    }

    pub fn or_insert_with(self, make: impl FnOnce() -> V) -> &'m mut V {
        self.map.get_or_insert_with(self.key, make)
    }

    pub fn or_default(self) -> &'m mut V
    where
        V: Default,
    {
        self.map.get_or_insert_with(self.key, V::default)
    }

    pub fn and_modify(self, f: impl FnOnce(&mut V)) -> Self {
        if let Some(v) = self.map.get_mut(&self.key) {
            f(v);
        }
        self
    }

    pub fn key(&self) -> &K {
        &self.key
    }
}

impl<K: Key, V: Pod + Copy> BlockHash<K, V> {
    pub fn entry(&mut self, key: K) -> Entry<'_, K, V> {
        Entry { map: self, key }
    }

    /// Keeps the entries `keep` holds for.
    pub fn retain(&mut self, mut keep: impl FnMut(&K, &mut V) -> bool) {
        let mut out: BlockHash<K, V> = BlockHash::with_capacity(self.room.clone(), self.len());
        self.for_each_mut(|k, v| {
            if keep(&k, v) {
                out.add(k, *v);
            }
        });
        *self = out;
    }
}

impl<K: Key> BlockHash<K, ()> {
    /// Whether `k` was not held before.
    pub fn put(&mut self, k: K) -> bool {
        if self.contains_key(&k) {
            return false;
        }
        self.add(k, ());
        true
    }
}

impl<K: Key, V: Pod + Copy> Clone for BlockHash<K, V> {
    fn clone(&self) -> Self {
        let mut c = BlockHash::<K, V> { room: self.room.clone(), mem: Self::make(&*self.room, self.desc().log), _t: PhantomData };
        for (k, v) in self.iter() {
            c.add(k, v);
        }
        c
    }
}

impl<K: Key + std::fmt::Debug, V: Pod + Copy + std::fmt::Debug> std::fmt::Debug for BlockHash<K, V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

/// A set in a block: a [`BlockHash`] with no values.
pub type BlockSet<K> = BlockHash<K, ()>;

structure! {
    /// A frozen map's descriptor: `len` keys sorted ascending, each `kbytes`, and their values, each `vbytes`.
    pub struct SortedDesc {
        pub len: u32,
        pub kbytes: u32,
        pub vbytes: u32,
        pub pad: u32,
        pub keys: Span<u8>,
        pub vals: Span<u8>,
    }
}

/// A frozen map in a block (MLDB's `MappedSortedKeyValueArray`): written once, found by binary search.
pub struct BlockSorted<K: Pod + Copy + Ord, V: Pod + Copy> {
    mem: Arc<Mem>,
    _t: PhantomData<(K, V)>,
}

// SAFETY: frozen: read only.
unsafe impl<K: Pod + Copy + Ord + Send, V: Pod + Copy + Send> Send for BlockSorted<K, V> {}
unsafe impl<K: Pod + Copy + Ord + Sync, V: Pod + Copy + Sync> Sync for BlockSorted<K, V> {}

impl<K: Pod + Copy + Ord, V: Pod + Copy> BlockSorted<K, V> {
    /// The map of at most `n` entries from `entries` (any order; a key twice keeps its last value).
    pub fn build(room: &dyn Room, n: usize, entries: impl IntoIterator<Item = (K, V)>) -> Self {
        let (kat, vat) = (up8(size_of::<SortedDesc>()), up8(size_of::<SortedDesc>()) + up8(n * size_of::<K>()));
        let mem = Mem::new(room, vat + n * size_of::<V>(), align_of::<K>().max(align_of::<V>()), "a sorted map");
        let (d, keys, vals) = (mem.at::<SortedDesc>(0), mem.at::<K>(kat), mem.at::<V>(vat));
        let mut len = 0;
        // SAFETY: arrays of `n` in the region just taken.
        unsafe {
            for (k, v) in entries.into_iter().take(n) {
                keys.add(len).write(k);
                vals.add(len).write(v);
                len += 1;
            }
            let (ks, vs) = (std::slice::from_raw_parts_mut(keys, len), std::slice::from_raw_parts_mut(vals, len));
            sort_by_key(ks, vs);
            // A key twice (now adjacent, in its entries' order): its last value, once.
            let mut w = 0;
            for r in 0..len {
                if w > 0 && ks[w - 1] == ks[r] {
                    vs[w - 1] = vs[r];
                } else {
                    ks[w] = ks[r];
                    vs[w] = vs[r];
                    w += 1;
                }
            }
            len = w;
            (*d).len = len as u32;
            (*d).kbytes = size_of::<K>() as u32;
            (*d).vbytes = size_of::<V>() as u32;
            Span::set(&raw const (*d).keys, keys.cast::<u8>(), len * size_of::<K>());
            Span::set(&raw const (*d).vals, vals.cast::<u8>(), len * size_of::<V>());
        }
        BlockSorted { mem: Arc::new(mem), _t: PhantomData }
    }

    fn desc(&self) -> &SortedDesc {
        // SAFETY: its descriptor, first in its region.
        unsafe { &*self.mem.at::<SortedDesc>(0) }
    }

    pub fn keys(&self) -> &[K] {
        let d = self.desc();
        // SAFETY: `len` keys, frozen.
        unsafe { std::slice::from_raw_parts(d.keys.as_ptr().cast::<K>(), d.len as usize) }
    }

    pub fn values(&self) -> &[V] {
        let d = self.desc();
        // SAFETY: `len` values, frozen.
        unsafe { std::slice::from_raw_parts(d.vals.as_ptr().cast::<V>(), d.len as usize) }
    }

    pub fn len(&self) -> usize {
        self.desc().len as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn get(&self, k: &K) -> Option<&V> {
        self.keys().binary_search(k).ok().map(|i| &self.values()[i])
    }

    /// The index of the first key not less than `k`.
    pub fn lower_bound(&self, k: &K) -> usize {
        self.keys().partition_point(|x| x < k)
    }

    pub fn iter(&self) -> impl Iterator<Item = (K, V)> + '_ {
        self.keys().iter().copied().zip(self.values().iter().copied())
    }

    pub fn place(&self) -> (&Arc<Block>, usize) {
        (self.mem.block(), self.mem.offset())
    }
}

impl<K: Pod + Copy + Ord, V: Pod + Copy> Clone for BlockSorted<K, V> {
    fn clone(&self) -> Self {
        BlockSorted { mem: self.mem.clone(), _t: PhantomData }
    }
}

/// Keys and values sorted together by key, stable, in place: runs of 32 by insertion, then merged by rotations
/// (SymMerge). No allocation.
fn sort_by_key<K: Copy + Ord, V: Copy>(ks: &mut [K], vs: &mut [V]) {
    const RUN: usize = 32;
    let n = ks.len();
    let mut a = 0;
    while a < n {
        for i in a + 1..(a + RUN).min(n) {
            let mut j = i;
            while j > a && ks[j - 1] > ks[j] {
                ks.swap(j - 1, j);
                vs.swap(j - 1, j);
                j -= 1;
            }
        }
        a += RUN;
    }
    let mut width = RUN;
    while width < n {
        let mut lo = 0;
        while lo + width < n {
            sym_merge(ks, vs, lo, lo + width, (lo + 2 * width).min(n));
            lo += 2 * width;
        }
        width *= 2;
    }
}

/// `lo..mid` and `mid..hi`, each sorted, merged stably in place (Kim and Kutzner's SymMerge, as Go's `sort.Stable`).
fn sym_merge<K: Copy + Ord, V: Copy>(ks: &mut [K], vs: &mut [V], lo: usize, mid: usize, hi: usize) {
    if lo >= mid || mid >= hi {
        return;
    }
    if mid - lo == 1 {
        // The one key `lo` goes after the keys of `mid..hi` less than it.
        let i = mid + ks[mid..hi].partition_point(|x| *x < ks[lo]);
        ks[lo..i].rotate_left(1);
        vs[lo..i].rotate_left(1);
        return;
    }
    if hi - mid == 1 {
        // The one key `mid` goes before the keys of `lo..mid` greater than it.
        let i = lo + ks[lo..mid].partition_point(|x| *x <= ks[mid]);
        ks[i..hi].rotate_right(1);
        vs[i..hi].rotate_right(1);
        return;
    }
    let m = lo + (hi - lo) / 2;
    let n = m + mid;
    let (mut start, mut r) = if mid > m { (n - hi, m) } else { (lo, mid) };
    let p = n - 1;
    while start < r {
        let c = start + (r - start) / 2;
        if ks[p - c] >= ks[c] {
            start = c + 1;
        } else {
            r = c;
        }
    }
    let end = n - start;
    if start < mid && mid < end {
        ks[start..end].rotate_left(mid - start);
        vs[start..end].rotate_left(mid - start);
    }
    if lo < start && start < m {
        sym_merge(ks, vs, lo, start, m);
    }
    if m < end && end < hi {
        sym_merge(ks, vs, m, end, hi);
    }
}

structure! {
    /// A string table's descriptor: `len` strings, string `i`'s bytes `ends[i - 1]..ends[i]` of `bytes`.
    pub struct StringsDesc {
        pub len: u32,
        pub pad: u32,
        pub ends: Span<u64>,
        pub bytes: Span<u8>,
    }
}

/// Strings in a block (MLDB's `FrozenBlobTable`): appended, each read by its index.
pub struct BlockStrings {
    room: Arc<dyn Room>,
    mem: Arc<Mem>,
    /// Room in its arrays: strings, bytes.
    cap: (usize, usize),
}

// SAFETY: its bytes lie in the region it holds, written only through `&mut self`.
unsafe impl Send for BlockStrings {}
unsafe impl Sync for BlockStrings {}

impl BlockStrings {
    pub fn with_capacity(room: Arc<dyn Room>, strings: usize, bytes: usize) -> Self {
        let cap = (strings.max(16), bytes.max(256));
        let mem = Arc::new(Self::make(&*room, cap));
        BlockStrings { room, mem, cap }
    }

    pub fn new(room: Arc<dyn Room>) -> Self {
        Self::with_capacity(room, 0, 0)
    }

    fn make(room: &dyn Room, (strings, bytes): (usize, usize)) -> Mem {
        let (eat, bat) = (up8(size_of::<StringsDesc>()), up8(size_of::<StringsDesc>()) + 8 * strings);
        let mem = Mem::new(room, bat + bytes, 8, "a string table");
        let d = mem.at::<StringsDesc>(0);
        // SAFETY: the region was just taken for these.
        unsafe {
            Span::set(&raw const (*d).ends, mem.at::<u64>(eat), 0);
            Span::set(&raw const (*d).bytes, mem.at::<u8>(bat), 0);
        }
        mem
    }

    fn desc(&self) -> &StringsDesc {
        // SAFETY: its descriptor, first in its region.
        unsafe { &*self.mem.at::<StringsDesc>(0) }
    }

    pub fn len(&self) -> usize {
        self.desc().len as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn ends(&self) -> &[u64] {
        self.desc().ends.as_slice()
    }

    /// The bytes of every string, in order.
    pub fn bytes(&self) -> &[u8] {
        self.desc().bytes.as_slice()
    }

    pub fn get_bytes(&self, i: usize) -> &[u8] {
        let ends = self.ends();
        let (a, b) = (if i == 0 { 0 } else { ends[i - 1] as usize }, ends[i] as usize);
        &self.bytes()[a..b]
    }

    pub fn get(&self, i: usize) -> &str {
        // SAFETY: every string was pushed as a `str` (`push`), or as bytes its reader reads as bytes.
        unsafe { std::str::from_utf8_unchecked(self.get_bytes(i)) }
    }

    /// Appends `s`; its index.
    pub fn push(&mut self, s: &str) -> usize {
        self.push_bytes(s.as_bytes())
    }

    pub fn push_bytes(&mut self, s: &[u8]) -> usize {
        let (n, used) = (self.len(), self.bytes().len());
        if n + 1 > self.cap.0 || used + s.len() > self.cap.1 {
            let cap = ((n + 1).max(2 * self.cap.0), (used + s.len()).max(2 * self.cap.1));
            let mem = Self::make(&*self.room, cap);
            let d = mem.at::<StringsDesc>(0);
            // SAFETY: the new arrays hold the old ones' and more.
            unsafe {
                // copy: growth: a table that outgrew its region, moved once, as a vector's growth moves it
                std::ptr::copy_nonoverlapping(self.ends().as_ptr(), (*d).ends.as_ptr().cast_mut(), n);
                // copy: growth: as above
                std::ptr::copy_nonoverlapping(self.bytes().as_ptr(), (*d).bytes.as_ptr().cast_mut(), used);
                (*d).len = n as u32;
                (*d).ends.len.store(n as u64, Relaxed);
                (*d).bytes.len.store(used as u64, Relaxed);
            }
            (self.mem, self.cap) = (Arc::new(mem), cap);
        }
        let d = self.mem.at::<StringsDesc>(0);
        // SAFETY: within the arrays' room (above).
        unsafe {
            // copy: derived: a string kept, written once where it is kept
            std::ptr::copy_nonoverlapping(s.as_ptr(), (*d).bytes.as_ptr().cast_mut().add(used), s.len());
            *(*d).ends.as_ptr().cast_mut().add(n) = (used + s.len()) as u64;
            (*d).len = n as u32 + 1;
            (*d).ends.len.store(n as u64 + 1, Relaxed);
            (*d).bytes.len.store((used + s.len()) as u64, Relaxed);
        }
        n
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> + '_ {
        (0..self.len()).map(|i| self.get(i))
    }

    pub fn place(&self) -> (&Arc<Block>, usize) {
        (self.mem.block(), self.mem.offset())
    }

    /// Its strings so far, read where they lie while more are appended: the region they are in stays while the view
    /// holds it (an append writes past them; a table that grows moves to a new region and leaves this one).
    pub fn view(&self) -> StringsView {
        let d = self.desc();
        StringsView { _mem: self.mem.clone(), ends: d.ends.as_ptr(), bytes: d.bytes.as_ptr(), len: self.len() }
    }

    /// Empty again, its room kept.
    pub fn clear(&mut self) {
        let d = self.mem.at::<StringsDesc>(0);
        // SAFETY: its descriptor.
        unsafe {
            (*d).len = 0;
            (*d).ends.len.store(0, Relaxed);
            (*d).bytes.len.store(0, Relaxed);
        }
    }
}

/// A table's first `len` strings (`BlockStrings::view`), read where they lie.
#[derive(Clone)]
pub struct StringsView {
    _mem: Arc<Mem>,
    ends: *const u64,
    bytes: *const u8,
    len: usize,
}

// SAFETY: it reads strings written before it was made, never written again.
unsafe impl Send for StringsView {}
unsafe impl Sync for StringsView {}

impl StringsView {
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn get_bytes(&self, i: usize) -> &[u8] {
        assert!(i < self.len, "string {i} of {}", self.len);
        // SAFETY: strings below `len`, written before the view.
        unsafe {
            let (a, b) = (if i == 0 { 0 } else { *self.ends.add(i - 1) as usize }, *self.ends.add(i) as usize);
            std::slice::from_raw_parts(self.bytes.add(a), b - a)
        }
    }

    pub fn get(&self, i: usize) -> &str {
        // SAFETY: as `BlockStrings::get`.
        unsafe { std::str::from_utf8_unchecked(self.get_bytes(i)) }
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> + '_ {
        (0..self.len).map(|i| self.get(i))
    }
}

/// `T`s appended, never changed (a table's column beside its strings): read where they lie through a view while more
/// are appended, as `StringsView` reads strings.
pub struct BlockLog<T: Pod + Copy> {
    room: Arc<dyn Room>,
    mem: Arc<Mem>,
    len: usize,
    cap: usize,
    _t: PhantomData<T>,
}

// SAFETY: its `T`s lie in the region it holds, appended only through `&mut self`.
unsafe impl<T: Pod + Copy + Send> Send for BlockLog<T> {}
unsafe impl<T: Pod + Copy + Sync> Sync for BlockLog<T> {}

impl<T: Pod + Copy> BlockLog<T> {
    pub fn new(room: Arc<dyn Room>) -> Self {
        let cap = 64;
        BlockLog { mem: Arc::new(Mem::new(&*room, cap * size_of::<T>(), align_of::<T>(), "a log")), room, len: 0, cap, _t: PhantomData }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn push(&mut self, v: T) {
        if self.len == self.cap {
            let cap = 2 * self.cap;
            let mem = Mem::new(&*self.room, cap * size_of::<T>(), align_of::<T>(), "a log");
            // SAFETY: the new region holds the `len` written and more.
            // copy: growth: a log that outgrew its region, moved once, as a vector's growth moves it
            unsafe { std::ptr::copy_nonoverlapping(self.mem.at::<T>(0), mem.at::<T>(0), self.len) };
            (self.mem, self.cap) = (Arc::new(mem), cap);
        }
        // SAFETY: below its room, past every view's `T`s.
        unsafe { self.mem.at::<T>(0).add(self.len).write(v) };
        self.len += 1;
    }

    /// Its `T`s so far, read where they lie while more are appended.
    pub fn view(&self) -> LogView<T> {
        LogView { _mem: self.mem.clone(), at: self.mem.at::<T>(0), len: self.len }
    }
}

impl<T: Pod + Copy> Extend<T> for BlockLog<T> {
    fn extend<I: IntoIterator<Item = T>>(&mut self, items: I) {
        for v in items {
            self.push(v);
        }
    }
}

impl<T: Pod + Copy> std::ops::Deref for BlockLog<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        // SAFETY: `len` `T`s written.
        unsafe { std::slice::from_raw_parts(self.mem.at::<T>(0), self.len) }
    }
}

impl<T: Pod + Copy> Default for BlockLog<T> {
    fn default() -> Self {
        BlockLog::new(default_room())
    }
}

/// A log's first `len` `T`s (`BlockLog::view`), read where they lie.
pub struct LogView<T: Pod> {
    _mem: Arc<Mem>,
    at: *const T,
    len: usize,
}

impl<T: Pod> Clone for LogView<T> {
    fn clone(&self) -> Self {
        LogView { _mem: self._mem.clone(), at: self.at, len: self.len }
    }
}

// SAFETY: it reads `T`s written before it was made, never written again.
unsafe impl<T: Pod + Send> Send for LogView<T> {}
unsafe impl<T: Pod + Sync> Sync for LogView<T> {}

impl<T: Pod> std::ops::Deref for LogView<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        // SAFETY: as `BlockLog::view`.
        unsafe { std::slice::from_raw_parts(self.at, self.len) }
    }
}

impl Default for BlockStrings {
    fn default() -> Self {
        BlockStrings::new(default_room())
    }
}

impl Clone for BlockStrings {
    fn clone(&self) -> Self {
        let mut c = BlockStrings::with_capacity(self.room.clone(), self.len(), self.bytes().len());
        for i in 0..self.len() {
            c.push_bytes(self.get_bytes(i));
        }
        c
    }
}

impl std::fmt::Debug for BlockStrings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

/// Strings and their indexes by their bytes (MLDB's string table): each string once.
pub struct BlockInterner {
    strings: BlockStrings,
    /// Each string's index, under its bytes' hash (`hash_bytes`); a string whose hash another took goes under the
    /// next free one (`probe`).
    table: BlockHash<u64, u32>,
}

impl BlockInterner {
    pub fn new(room: Arc<dyn Room>) -> Self {
        BlockInterner { strings: BlockStrings::new(room.clone()), table: BlockHash::new(room) }
    }

    pub fn with_capacity(room: Arc<dyn Room>, strings: usize, bytes: usize) -> Self {
        BlockInterner { strings: BlockStrings::with_capacity(room.clone(), strings, bytes), table: BlockHash::with_capacity(room, strings) }
    }

    /// Where `s` is or would go: its hash, stepped past the hashes other strings took.
    fn probe(&self, s: &[u8]) -> (u64, Option<usize>) {
        let mut h = hash_bytes(s);
        loop {
            if h == u64::MAX {
                h = 0;
            }
            match self.table.get(&h) {
                None => return (h, None),
                Some(&i) if self.strings.get_bytes(i as usize) == s => return (h, Some(i as usize)),
                Some(_) => h = h.wrapping_add(1),
            }
        }
    }

    /// `s`'s index, if it is held.
    pub fn find(&self, s: &str) -> Option<usize> {
        self.probe(s.as_bytes()).1
    }

    /// `s`'s index, added when new.
    pub fn intern(&mut self, s: &str) -> usize {
        self.intern_bytes(s.as_bytes())
    }

    pub fn intern_bytes(&mut self, s: &[u8]) -> usize {
        match self.probe(s) {
            (_, Some(i)) => i,
            (h, None) => {
                let i = self.strings.push_bytes(s);
                self.table.insert(h, i as u32);
                i
            }
        }
    }

    pub fn get(&self, i: usize) -> &str {
        self.strings.get(i)
    }

    pub fn len(&self) -> usize {
        self.strings.len()
    }

    pub fn is_empty(&self) -> bool {
        self.strings.is_empty()
    }

    pub fn strings(&self) -> &BlockStrings {
        &self.strings
    }

    /// Its strings so far (`BlockStrings::view`).
    pub fn view(&self) -> StringsView {
        self.strings.view()
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> + '_ {
        self.strings.iter()
    }
}

impl Default for BlockInterner {
    fn default() -> Self {
        BlockInterner::new(default_room())
    }
}

impl Clone for BlockInterner {
    fn clone(&self) -> Self {
        BlockInterner { strings: self.strings.clone(), table: self.table.clone() }
    }
}

impl std::fmt::Debug for BlockInterner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.strings.fmt(f)
    }
}

/// `n` `T`s in one region, zeroed, fixed in length (a `Box<[T]>`'s place): read and written as a slice; an array of
/// atomics is written through `&self`.
pub struct BlockArray<T: Pod> {
    mem: Mem,
    len: usize,
    _t: PhantomData<T>,
}

// SAFETY: its `T`s lie in the region it holds, written through `&mut self` (or atomically).
unsafe impl<T: Pod + Send> Send for BlockArray<T> {}
unsafe impl<T: Pod + Sync> Sync for BlockArray<T> {}

impl<T: Pod> BlockArray<T> {
    pub fn zeroed(room: &dyn Room, n: usize) -> Self {
        BlockArray { mem: Mem::new(room, n * size_of::<T>(), align_of::<T>(), "an array"), len: n, _t: PhantomData }
    }

    pub fn place(&self) -> (&Arc<Block>, usize) {
        (self.mem.block(), self.mem.offset())
    }
}

impl<T: Pod + Copy> BlockArray<T> {
    /// The `T`s of `items` (exactly `n`).
    pub fn from_iter(room: &dyn Room, n: usize, items: impl IntoIterator<Item = T>) -> Self {
        let mut a = BlockArray::zeroed(room, n);
        let mut i = 0;
        for v in items.into_iter().take(n) {
            a[i] = v;
            i += 1;
        }
        assert_eq!(i, n, "an array of {n} given {i}");
        a
    }
}

impl<T: Pod> std::ops::Deref for BlockArray<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        // SAFETY: `len` `T`s, zeroed or written.
        unsafe { std::slice::from_raw_parts(self.mem.at::<T>(0), self.len) }
    }
}

impl<T: Pod> std::ops::DerefMut for BlockArray<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        // SAFETY: as `deref`, through `&mut self`.
        unsafe { std::slice::from_raw_parts_mut(self.mem.at::<T>(0), self.len) }
    }
}

/// One record of type `T` in its region, with `bytes_after` bytes for its children after it (MLDB's `MappedValue`).
pub struct Rec<T: Pod> {
    mem: Arc<Mem>,
    _t: PhantomData<T>,
}

// SAFETY: its bytes lie in the region it holds, written once by `fill`.
unsafe impl<T: Pod + Send> Send for Rec<T> {}
unsafe impl<T: Pod + Sync> Sync for Rec<T> {}

impl<T: Pod> Rec<T> {
    /// A zeroed record, filled by `fill` (the record, and the `bytes_after` zeroed bytes after it for its children).
    pub fn new(room: &dyn Room, bytes_after: usize, fill: impl FnOnce(*mut T, *mut u8)) -> Self {
        let at = up8(size_of::<T>());
        let mem = Mem::new(room, at + bytes_after, align_of::<T>(), "a record");
        fill(mem.at::<T>(0), mem.at::<u8>(at));
        Rec { mem: Arc::new(mem), _t: PhantomData }
    }

    pub fn get(&self) -> &T {
        // SAFETY: its record, first in its region.
        unsafe { &*self.mem.at::<T>(0) }
    }

    pub fn place(&self) -> (&Arc<Block>, usize) {
        (self.mem.block(), self.mem.offset())
    }
}

impl<T: Pod> Clone for Rec<T> {
    fn clone(&self) -> Self {
        Rec { mem: self.mem.clone(), _t: PhantomData }
    }
}

impl<T: Pod> std::ops::Deref for Rec<T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An id table in a block (`IdTable`, a set of row ids found by their rows' hashes) finds each id through growth,
    /// removals that move ids probed past the removed one back, and an id written again in its slot; ids whose hashes
    /// collide are told apart by their rows. (The engine's other id set in a block, `facts::rows::IdSet`, was merged
    /// into it, with this test.)
    #[test]
    fn an_id_table_finds_each_id_through_growth_and_removals() {
        // Rows are their ids' values; a hash of 8 values only, so many collide.
        let row = |id: u32| u64::from(id) * 7 + 1;
        let hash = |&id: &u32| row(id) % 8;
        let mut set: IdTable<u32> = IdTable::new();
        for id in 0..3000u32 {
            assert_eq!(set.find(hash(&id), |&i| row(i) == row(id)), None);
            set.insert_unique(hash(&id), id, hash);
        }
        assert_eq!(set.len(), 3000);
        for id in (0..3000u32).step_by(3) {
            assert_eq!(set.remove(hash(&id), |&i| row(i) == row(id), hash), Some(id));
        }
        assert_eq!(set.len(), 2000);
        for id in 0..3000u32 {
            let want = (id % 3 != 0).then_some(id);
            assert_eq!(set.find(hash(&id), |&i| row(i) == row(id)).copied(), want, "id {id}");
        }
        assert!(set.find_mut(hash(&1), |&i| i == 1).map(|v| *v = 1).is_some());
        assert!(set.find_mut(hash(&0), |&i| i == 0).is_none(), "a removed id is not held");
    }
    use crate::block::kind;

    fn room() -> Arc<dyn Room> {
        Arc::new(InBlock(Arc::new(Block::anon(kind::FACTS, 1 << 24).with_free_list())))
    }

    /// A region past half a new block's size, just over a power of two (the strings' table: 33,554,500 bytes), takes
    /// a block of its own that holds the free list's chunk for it, aligned to its size.
    #[test]
    fn an_anonymous_room_holds_a_region_past_a_power_of_two() {
        let room = AnonRoom::default();
        for bytes in [(32 << 20) + 68, (64 << 20) + 1, 100] {
            let r = room.region(bytes, 8, "a test's region");
            assert_eq!(r.len(), bytes);
        }
    }

    #[test]
    fn a_block_hash_holds_what_a_hash_map_holds() {
        let mut m = BlockHash::<u64, u32>::new(room());
        let mut want = std::collections::HashMap::new();
        for i in 0..5000u64 {
            let k = (i * 7919) % 3001;
            if i % 5 == 4 {
                assert_eq!(m.remove(&k), want.remove(&k));
            } else {
                assert_eq!(m.insert(k, i as u32), want.insert(k, i as u32));
            }
        }
        m.insert(u64::MAX, 9);
        want.insert(u64::MAX, 9);
        assert_eq!(m.len(), want.len());
        for (k, v) in &want {
            assert_eq!(m.get(k), Some(v));
        }
        let mut got: Vec<_> = m.iter().collect();
        got.sort();
        let mut all: Vec<_> = want.into_iter().collect();
        all.sort();
        assert_eq!(got, all);
    }

    #[test]
    fn a_sorted_map_finds_its_keys_and_keeps_the_last_value() {
        let entries: Vec<(u32, u32)> = (0..1000u32).map(|i| ((i * 37) % 500, i)).collect();
        let m = BlockSorted::build(&*room(), entries.len(), entries.iter().copied());
        assert_eq!(m.len(), 500);
        assert!(m.keys().windows(2).all(|w| w[0] < w[1]));
        for (k, _) in &entries {
            let last = entries.iter().rev().find(|(k2, _)| k2 == k).unwrap().1;
            assert_eq!(m.get(k), Some(&last));
        }
    }

    #[test]
    fn an_interner_holds_each_string_once() {
        let mut t = BlockInterner::new(room());
        let words: Vec<String> = (0..3000).map(|i| format!("w{}", i % 1200)).collect();
        let ids: Vec<usize> = words.iter().map(|w| t.intern(w)).collect();
        assert_eq!(t.len(), 1200);
        for (w, &i) in words.iter().zip(&ids) {
            assert_eq!(t.get(i), w);
            assert_eq!(t.find(w), Some(i));
        }
        assert_eq!(t.intern(""), 1200);
        assert_eq!(t.get(1200), "");
    }
}
impl<K: Key, V: Pod + Copy> std::panic::UnwindSafe for BlockHash<K, V> {}
impl<K: Key, V: Pod + Copy> std::panic::RefUnwindSafe for BlockHash<K, V> {}
impl<K: Pod + Copy + Ord, V: Pod + Copy> std::panic::UnwindSafe for BlockSorted<K, V> {}
impl<K: Pod + Copy + Ord, V: Pod + Copy> std::panic::RefUnwindSafe for BlockSorted<K, V> {}
impl std::panic::UnwindSafe for BlockStrings {}
impl std::panic::RefUnwindSafe for BlockStrings {}
impl std::panic::UnwindSafe for BlockInterner {}
impl std::panic::RefUnwindSafe for BlockInterner {}
impl std::panic::UnwindSafe for StringsView {}
impl std::panic::RefUnwindSafe for StringsView {}
impl<T: Pod + Copy> std::panic::UnwindSafe for BlockLog<T> {}
impl<T: Pod + Copy> std::panic::RefUnwindSafe for BlockLog<T> {}
impl<T: Pod> std::panic::UnwindSafe for LogView<T> {}
impl<T: Pod> std::panic::RefUnwindSafe for LogView<T> {}
impl<T: Pod> std::panic::UnwindSafe for BlockArray<T> {}
impl<T: Pod> std::panic::RefUnwindSafe for BlockArray<T> {}
impl<T: Pod> std::panic::UnwindSafe for Rec<T> {}
impl<T: Pod> std::panic::RefUnwindSafe for Rec<T> {}

/// A hash table of plain values in a block, hashed by what the caller computes (`hashbrown::HashTable`'s place): a
/// value's slot found by its hash and the caller's equality, as a set of row ids is, whose rows lie elsewhere. Each
/// slot is a tag of its value's hash (0: empty) and the value, in two arrays in the run's room. It grows (every value
/// placed again by the caller's hash) past three quarters full.
pub struct IdTable<T: Pod + Copy> {
    tags: crate::vector::Vector<u32>,
    vals: crate::vector::Vector<T>,
    len: usize,
}

impl<T: Pod + Copy> Default for IdTable<T> {
    fn default() -> Self {
        IdTable { tags: crate::vector::Vector::new(), vals: crate::vector::Vector::new(), len: 0 }
    }
}

impl<T: Pod + Copy> IdTable<T> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Room for `n` values before it grows.
    pub fn with_capacity(n: usize) -> Self {
        let mut t = Self::default();
        if n > 0 {
            t.make((n * 4 / 3 + 1).next_power_of_two().max(8));
        }
        t
    }

    fn make(&mut self, cap: usize) {
        self.tags = crate::vector::Vector::with_capacity(cap);
        self.tags.resize(cap, 0);
        self.vals = crate::vector::Vector::with_capacity(cap);
        // SAFETY: plain values, each read only after its tag says it was written.
        unsafe { self.vals.set_len(cap) };
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Its slots.
    pub fn capacity(&self) -> usize {
        self.tags.len()
    }

    #[inline]
    fn tag(h: u64) -> u32 {
        // Never 0, the empty slot's.
        (h >> 32) as u32 | 1
    }

    /// The slot of a value equal to `eq`'s, by its hash `h`.
    #[inline]
    fn slot(&self, h: u64, mut eq: impl FnMut(&T) -> bool) -> Option<usize> {
        if self.len == 0 {
            return None;
        }
        let (mask, tag) = (self.tags.len() - 1, Self::tag(h));
        let mut i = h as usize & mask;
        loop {
            let t = self.tags[i];
            if t == 0 {
                return None;
            }
            if t == tag && eq(&self.vals[i]) {
                return Some(i);
            }
            i = (i + 1) & mask;
        }
    }

    pub fn find(&self, h: u64, eq: impl FnMut(&T) -> bool) -> Option<&T> {
        self.slot(h, eq).map(|i| &self.vals[i])
    }

    pub fn find_mut(&mut self, h: u64, eq: impl FnMut(&T) -> bool) -> Option<&mut T> {
        self.slot(h, eq).map(|i| &mut self.vals[i])
    }

    /// `v`, which it does not hold, put in by its hash `h`; `hasher` hashes the values it holds when it grows.
    pub fn insert_unique(&mut self, h: u64, v: T, hasher: impl Fn(&T) -> u64) -> &mut T {
        if 4 * (self.len + 1) > 3 * self.tags.len() {
            self.grow(&hasher);
        }
        let i = self.place(h, v);
        self.len += 1;
        &mut self.vals[i]
    }

    fn place(&mut self, h: u64, v: T) -> usize {
        let mask = self.tags.len() - 1;
        let mut i = h as usize & mask;
        while self.tags[i] != 0 {
            i = (i + 1) & mask;
        }
        self.tags[i] = Self::tag(h);
        self.vals[i] = v;
        i
    }

    /// Twice the slots (8 at least), every value placed again by `hasher`.
    #[cold]
    fn grow(&mut self, hasher: &impl Fn(&T) -> u64) {
        let (tags, vals) = (std::mem::take(&mut self.tags), std::mem::take(&mut self.vals));
        self.make((2 * tags.len()).max(8));
        for (i, &t) in tags.iter().enumerate() {
            if t != 0 {
                let v = vals[i];
                self.place(hasher(&v), v);
            }
        }
    }

    /// A value equal to `eq`'s taken out (backward shift, so no slot is a tombstone); `hasher` places the values
    /// after it again.
    pub fn remove(&mut self, h: u64, eq: impl FnMut(&T) -> bool, hasher: impl Fn(&T) -> u64) -> Option<T> {
        let mut i = self.slot(h, eq)?;
        let v = self.vals[i];
        let mask = self.tags.len() - 1;
        self.tags[i] = 0;
        self.len -= 1;
        // The values after it in its run placed again.
        i = (i + 1) & mask;
        while self.tags[i] != 0 {
            let w = self.vals[i];
            self.tags[i] = 0;
            self.place(hasher(&w), w);
            i = (i + 1) & mask;
        }
        Some(v)
    }

    pub fn clear(&mut self) {
        self.tags.iter_mut().for_each(|t| *t = 0);
        self.len = 0;
    }

    /// Each value held, in slot order.
    pub fn iter(&self) -> impl Iterator<Item = &T> + '_ {
        self.tags.iter().zip(self.vals.iter()).filter(|(t, _)| **t != 0).map(|(_, v)| v)
    }

    /// Each value held, to change in place (its hash, which placed it, kept).
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> + '_ {
        self.tags.iter().zip(self.vals.iter_mut()).filter(|(t, _)| **t != 0).map(|(_, v)| v)
    }
}

impl<T: Pod + Copy> Clone for IdTable<T> {
    fn clone(&self) -> Self {
        IdTable { tags: self.tags.clone(), vals: self.vals.clone(), len: self.len }
    }
}
