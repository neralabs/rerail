//! Decompressed read cache: an LRU of source and tape bytes backed by an anonymous mmap arena.

use super::{CachedBytes, HashKey, StoreError, StoreResult};
use std::collections::HashMap;
use std::ops::Deref;
#[cfg(unix)]
use std::ptr::NonNull;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum CacheKey {
    Source(HashKey),
    Tape(HashKey),
}

pub(super) enum CachedBacking {
    Arena {
        arena: Arc<MmapArena>,
        offset: usize,
        length: usize,
    },
    Heap(Box<[u8]>),
}

pub(super) struct MmapArena {
    #[cfg(unix)]
    pointer: NonNull<u8>,
    #[cfg(not(unix))]
    bytes: std::cell::UnsafeCell<Box<[u8]>>,
    length: usize,
}

// The cache mutex serializes arena appends. Published tape slices never
// overlap a later append, and collection writes only to a new arena.
unsafe impl Send for MmapArena {}
unsafe impl Sync for MmapArena {}

impl MmapArena {
    fn new(length: usize) -> StoreResult<Self> {
        #[cfg(unix)]
        {
            let pointer = unsafe {
                libc::mmap(
                    std::ptr::null_mut(),
                    length,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_PRIVATE | libc::MAP_ANON,
                    -1,
                    0,
                )
            };
            if pointer == libc::MAP_FAILED {
                return Err(StoreError::Io(std::io::Error::last_os_error()));
            }
            Ok(Self {
                pointer: NonNull::new(pointer.cast::<u8>())
                    .expect("mmap returned a non-null pointer"),
                length,
            })
        }
        #[cfg(not(unix))]
        {
            Ok(Self {
                bytes: std::cell::UnsafeCell::new(vec![0; length].into_boxed_slice()),
                length,
            })
        }
    }

    fn copy_in(&self, offset: usize, bytes: &[u8]) {
        assert!(offset.saturating_add(bytes.len()) <= self.length);
        #[cfg(unix)]
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                self.pointer.as_ptr().add(offset),
                bytes.len(),
            );
        }
        #[cfg(not(unix))]
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                (*self.bytes.get()).as_mut_ptr().add(offset),
                bytes.len(),
            );
        }
    }

    fn slice(&self, offset: usize, length: usize) -> &[u8] {
        assert!(offset.saturating_add(length) <= self.length);
        #[cfg(unix)]
        unsafe {
            std::slice::from_raw_parts(self.pointer.as_ptr().add(offset), length)
        }
        #[cfg(not(unix))]
        {
            unsafe { &(*self.bytes.get())[offset..offset + length] }
        }
    }
}

#[cfg(unix)]
impl Drop for MmapArena {
    fn drop(&mut self) {
        unsafe {
            libc::madvise(
                self.pointer.as_ptr().cast(),
                self.length,
                libc::MADV_DONTNEED,
            );
            libc::munmap(self.pointer.as_ptr().cast(), self.length);
        }
    }
}

impl CachedBytes {
    fn heap(bytes: Vec<u8>) -> Self {
        Self {
            backing: CachedBacking::Heap(bytes.into_boxed_slice()),
        }
    }

    fn arena(arena: Arc<MmapArena>, offset: usize, length: usize) -> Self {
        Self {
            backing: CachedBacking::Arena {
                arena,
                offset,
                length,
            },
        }
    }
}

impl Deref for CachedBytes {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        match &self.backing {
            CachedBacking::Arena {
                arena,
                offset,
                length,
            } => arena.slice(*offset, *length),
            CachedBacking::Heap(bytes) => bytes,
        }
    }
}

pub(super) struct DecompressedCacheEntry {
    value: Arc<CachedBytes>,
    expires_at: Instant,
    previous: Option<CacheKey>,
    next: Option<CacheKey>,
}

pub(super) struct DecompressedReadCache {
    byte_limit: usize,
    arena_bytes: usize,
    ttl: Duration,
    used_bytes: usize,
    cursor: usize,
    active: Option<Arc<MmapArena>>,
    retired: Option<std::sync::Weak<MmapArena>>,
    head: Option<CacheKey>,
    tail: Option<CacheKey>,
    entries: HashMap<CacheKey, DecompressedCacheEntry>,
    /// Entry-map probes made while unlinking and relinking LRU nodes.
    #[cfg(test)]
    lru_link_probes: usize,
}

impl DecompressedReadCache {
    pub(super) fn new(byte_limit: usize, ttl: Duration) -> Self {
        Self {
            byte_limit,
            arena_bytes: byte_limit.saturating_mul(2),
            ttl,
            used_bytes: 0,
            cursor: 0,
            active: None,
            retired: None,
            head: None,
            tail: None,
            entries: HashMap::new(),
            #[cfg(test)]
            lru_link_probes: 0,
        }
    }

    #[cfg(test)]
    pub(super) fn contains(&self, key: CacheKey) -> bool {
        self.entries.contains_key(&key)
    }

    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Counts one entry-map probe of LRU relinking; a no-op outside tests.
    fn count_link_probe(&mut self) {
        #[cfg(test)]
        {
            self.lru_link_probes += 1;
        }
    }

    fn detach(&mut self, key: CacheKey) {
        self.count_link_probe();
        let Some(entry) = self.entries.get(&key) else {
            return;
        };
        let (previous, next) = (entry.previous, entry.next);
        if let Some(previous) = previous {
            self.count_link_probe();
            self.entries
                .get_mut(&previous)
                .expect("LRU previous exists")
                .next = next;
        } else {
            self.head = next;
        }
        if let Some(next) = next {
            self.count_link_probe();
            self.entries
                .get_mut(&next)
                .expect("LRU next exists")
                .previous = previous;
        } else {
            self.tail = previous;
        }
        self.count_link_probe();
        let entry = self.entries.get_mut(&key).expect("LRU entry exists");
        entry.previous = None;
        entry.next = None;
    }
    fn append(&mut self, key: CacheKey) {
        let previous = self.tail;
        if let Some(previous) = previous {
            self.count_link_probe();
            self.entries
                .get_mut(&previous)
                .expect("LRU tail exists")
                .next = Some(key);
        } else {
            self.head = Some(key);
        }
        self.count_link_probe();
        let entry = self.entries.get_mut(&key).expect("LRU entry exists");
        entry.previous = previous;
        entry.next = None;
        self.tail = Some(key);
    }
    fn touch(&mut self, key: CacheKey) {
        self.detach(key);
        self.append(key);
    }
    pub(super) fn expire(&mut self, now: Instant) {
        while let Some(key) = self.head {
            if self
                .entries
                .get(&key)
                .is_some_and(|entry| entry.expires_at > now)
            {
                break;
            }
            self.remove(key);
        }
        if self.entries.is_empty() {
            self.release_empty_arena();
        }
    }
    fn release_empty_arena(&mut self) {
        if self
            .active
            .as_ref()
            .is_some_and(|arena| Arc::strong_count(arena) == 1)
        {
            self.active = None;
            self.cursor = 0;
        } else {
            self.cursor = self.arena_bytes;
        }
        if self
            .retired
            .as_ref()
            .is_some_and(|arena| arena.strong_count() == 0)
        {
            self.retired = None;
        }
    }
    fn remove(&mut self, key: CacheKey) {
        self.detach(key);
        if let Some(entry) = self.entries.remove(&key) {
            self.used_bytes = self.used_bytes.saturating_sub(entry.value.len());
        }
    }
    pub(super) fn get(&mut self, key: CacheKey) -> Option<Arc<CachedBytes>> {
        let now = Instant::now();
        self.expire(now);
        let value = {
            let entry = self.entries.get_mut(&key)?;
            entry.expires_at = now + self.ttl;
            Arc::clone(&entry.value)
        };
        self.touch(key);
        Some(value)
    }
    fn compact(&mut self) -> StoreResult<bool> {
        if self
            .retired
            .as_ref()
            .and_then(std::sync::Weak::upgrade)
            .is_some()
        {
            return Ok(false);
        }
        let replacement = Arc::new(MmapArena::new(self.arena_bytes)?);
        let mut cursor = 0;
        let mut key = self.head;
        while let Some(current) = key {
            let entry = self.entries.get_mut(&current).expect("LRU entry exists");
            cursor = (cursor + 7) & !7;
            replacement.copy_in(cursor, &entry.value);
            entry.value = Arc::new(CachedBytes::arena(
                Arc::clone(&replacement),
                cursor,
                entry.value.len(),
            ));
            cursor += entry.value.len();
            key = entry.next;
        }
        if let Some(previous) = self.active.replace(replacement) {
            self.retired = Some(Arc::downgrade(&previous));
        }
        self.cursor = cursor;
        Ok(true)
    }
    pub(super) fn insert_bytes(
        &mut self,
        key: CacheKey,
        bytes: Vec<u8>,
    ) -> StoreResult<Arc<CachedBytes>> {
        self.expire(Instant::now());
        self.remove(key);
        if bytes.len() > self.byte_limit {
            return Ok(Arc::new(CachedBytes::heap(bytes)));
        }
        while self.used_bytes.saturating_add(bytes.len()) > self.byte_limit {
            let Some(oldest) = self.head else { break };
            self.remove(oldest);
        }
        if self.active.is_none() {
            self.active = Some(Arc::new(MmapArena::new(self.arena_bytes)?));
            self.cursor = 0;
        }
        let offset = (self.cursor + 7) & !7;
        if offset.saturating_add(bytes.len()) > self.arena_bytes && !self.compact()? {
            return Ok(Arc::new(CachedBytes::heap(bytes)));
        }
        let offset = (self.cursor + 7) & !7;
        if offset.saturating_add(bytes.len()) > self.arena_bytes {
            return Ok(Arc::new(CachedBytes::heap(bytes)));
        }
        let active = Arc::clone(self.active.as_ref().expect("active arena"));
        active.copy_in(offset, &bytes);
        let value = Arc::new(CachedBytes::arena(active, offset, bytes.len()));
        self.cursor = offset + bytes.len();
        self.used_bytes += bytes.len();
        self.entries.insert(
            key,
            DecompressedCacheEntry {
                value: Arc::clone(&value),
                expires_at: Instant::now() + self.ttl,
                previous: None,
                next: None,
            },
        );
        self.append(key);
        Ok(value)
    }
    pub(super) fn clear(&mut self) {
        self.used_bytes = 0;
        self.head = None;
        self.tail = None;
        self.entries.clear();
        self.release_empty_arena();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mmap_decompressed_cache_holds_sources_and_tapes() {
        let mut cache = DecompressedReadCache::new(12, Duration::from_secs(1));
        let first_key = CacheKey::Source(HashKey::sha256(b"first"));
        let second_key = CacheKey::Tape(HashKey::sha256(b"second"));
        let third_key = CacheKey::Source(HashKey::sha256(b"third"));
        let fourth_key = CacheKey::Tape(HashKey::sha256(b"fourth"));
        let first = cache.insert_bytes(first_key, b"first".to_vec()).unwrap();
        assert!(Arc::ptr_eq(&cache.get(first_key).unwrap(), &first));
        let second = cache.insert_bytes(second_key, b"second".to_vec()).unwrap();
        assert_eq!(&**second, b"second");
        let third = cache.insert_bytes(third_key, b"third".to_vec()).unwrap();
        assert_eq!(&**third, b"third");
        let fourth = cache.insert_bytes(fourth_key, b"fourth".to_vec()).unwrap();
        assert_eq!(&**fourth, b"fourth");
        let copied_third = cache.get(third_key).unwrap();
        assert_eq!(&**copied_third, b"third");
        assert!(!Arc::ptr_eq(&copied_third, &third));
        assert!(cache.get(first_key).is_none());
        assert!(cache.active.is_some());
        assert!(cache.retired.is_some());
        cache.expire(Instant::now() + Duration::from_secs(2));
        assert_eq!(cache.used_bytes, 0);
        assert!(cache.entries.is_empty());
    }

    #[test]
    fn decompressed_cache_refreshes_lru_in_constant_work() {
        const ENTRIES: u64 = 10_000;
        let key = |ordinal: u64| CacheKey::Source(HashKey::sha256(&ordinal.to_le_bytes()));
        let mut cache = DecompressedReadCache::new(100_000, Duration::from_secs(30));
        for ordinal in 0..ENTRIES {
            cache
                .insert_bytes(key(ordinal), vec![ordinal as u8])
                .unwrap();
        }
        // Refresh every entry from the least to the most recently used end: each refresh unlinks
        // an interior node and relinks it at the tail.
        cache.lru_link_probes = 0;
        for ordinal in 0..ENTRIES {
            assert!(cache.get(key(ordinal)).is_some());
        }
        assert!(
            cache.lru_link_probes <= 6 * ENTRIES as usize,
            "{ENTRIES} LRU refreshes made {} entry probes",
            cache.lru_link_probes
        );
        assert_eq!(cache.head, Some(key(0)));
        assert_eq!(cache.tail, Some(key(ENTRIES - 1)));
    }

    #[test]
    fn decompressed_cache_has_a_hard_byte_limit() {
        let mut cache = DecompressedReadCache::new(5, Duration::from_secs(30));
        let source_key = CacheKey::Source(HashKey::sha256(b"abc"));
        let tape_key = CacheKey::Tape(HashKey::sha256(b"def"));
        let oversized_key = CacheKey::Source(HashKey::sha256(b"too large"));
        cache.insert_bytes(source_key, b"abc".to_vec()).unwrap();
        cache.insert_bytes(tape_key, b"def".to_vec()).unwrap();
        assert_eq!(cache.used_bytes, 3);
        assert!(cache.get(source_key).is_none());
        assert_eq!(&**cache.get(tape_key).unwrap(), b"def");
        cache
            .insert_bytes(oversized_key, b"too large".to_vec())
            .unwrap();
        assert!(cache.get(oversized_key).is_none());
        assert!(cache.used_bytes <= cache.byte_limit);
    }
}
