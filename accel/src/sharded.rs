//! A lock sharded by reader (decision 0279 §7), for state read far more than written: the atom arena and the
//! planner's relation names. A shared read lock is still a write of one word
//! that every reader shares: a join that reads texts row by row on 16 threads spent its time on that word
//! (`lock_shared_slow`, 0278 §4). Here each thread reads under a shard of its own, given round-robin at its first
//! read, so the pool's threads write no word another reads. A writer takes every shard, in order: writes are rare
//! (a batch of interned atoms, a stored segment, a relation named for the first time), reads are per text or name.
//!
//! As `parking_lot::RwLock`, which it replaces: not reentrant for a reader while a writer waits, no poisoning.
use parking_lot::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::cell::UnsafeCell;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicUsize, Ordering};

/// One shard's lock on a line of its own: two readers of different shards share no cache line.
#[repr(align(128))]
struct Shard(RwLock<()>);

pub struct Sharded<T> {
    shards: Box<[Shard]>,
    value: UnsafeCell<T>,
}

// SAFETY: `value` is read under one shard's read lock and written under every shard's write lock, so a `&T` and a
// `&mut T` of it never coexist, and the `&T`s of several threads are as `RwLock<T>`'s.
unsafe impl<T: Send> Send for Sharded<T> {}
unsafe impl<T: Send + Sync> Sync for Sharded<T> {}

static NEXT: AtomicUsize = AtomicUsize::new(0);
std::thread_local! {
    /// This thread's shard, modulo the shards: threads take them in turn as they first read.
    static SHARD: usize = NEXT.fetch_add(1, Ordering::Relaxed);
}

impl<T> Sharded<T> {
    /// As many shards as threads can run at once (rounded up to a power of two, 8 to 64).
    pub fn new(value: T) -> Self {
        let n = std::thread::available_parallelism().map_or(8, |n| n.get()).next_power_of_two().clamp(8, 64);
        Sharded { shards: (0..n).map(|_| Shard(RwLock::new(()))).collect(), value: UnsafeCell::new(value) }
    }

    #[inline]
    pub fn read(&self) -> ReadGuard<'_, T> {
        let shard = &self.shards[SHARD.with(|s| *s) & (self.shards.len() - 1)];
        ReadGuard { _shard: shard.0.read(), value: self.value.get() }
    }

    pub fn write(&self) -> WriteGuard<'_, T> {
        WriteGuard { _shards: self.shards.iter().map(|s| s.0.write()).collect(), value: self.value.get() }
    }
}

pub struct ReadGuard<'a, T> {
    _shard: RwLockReadGuard<'a, ()>,
    value: *const T,
}

impl<T> Deref for ReadGuard<'_, T> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &T {
        // SAFETY: a shard's read lock is held while the guard lives (see `Sharded`'s `Sync`).
        unsafe { &*self.value }
    }
}

pub struct WriteGuard<'a, T> {
    _shards: Vec<RwLockWriteGuard<'a, ()>>,
    value: *mut T,
}

impl<T> Deref for WriteGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: every shard's write lock is held while the guard lives.
        unsafe { &*self.value }
    }
}

impl<T> DerefMut for WriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: as `deref`; no reader holds a shard.
        unsafe { &mut *self.value }
    }
}

/// State kept per scope (a connection's address) in one process-wide map, split by scope: each scope's entries sit in
/// one shard, under that shard's lock alone. Runs on different connections (the chunks evaluating at once, decision
/// 0278) then take different locks, where one lock over the whole map made them queue for each other
/// (`__psynch_mutexwait` in every stratum of every chunk, notes/t40). `M` is the map itself, keyed by scope or by
/// (scope, name); every key of a shard's map is of a scope that shard is chosen by.
pub struct ByScope<M> {
    shards: Box<[Locked<M>]>,
}

#[repr(align(128))]
struct Locked<M>(std::sync::Mutex<M>);

impl<M: Default> Default for ByScope<M> {
    fn default() -> Self {
        let n = std::thread::available_parallelism().map_or(8, |n| n.get()).next_power_of_two().clamp(8, 64) * 4;
        ByScope { shards: (0..n).map(|_| Locked(std::sync::Mutex::new(M::default()))).collect() }
    }
}

impl<M> ByScope<M> {
    /// The map holding `scope`'s entries (and others' that share its shard), locked. A panic while it was held does
    /// not poison it: the maps hold whole entries.
    pub fn lock(&self, scope: usize) -> std::sync::MutexGuard<'_, M> {
        // A scope is an address: its low bits are alignment. Fibonacci hashing: the product's top bits mix them all.
        let bits = self.shards.len().trailing_zeros();
        let h = (scope as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> (64 - bits);
        self.shards[h as usize].0.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Every shard in turn (tests that look across scopes).
    pub fn each(&self, mut f: impl FnMut(&mut M)) {
        for s in self.shards.iter() {
            f(&mut s.0.lock().unwrap_or_else(|p| p.into_inner()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Sharded;

    /// One scope's entries are always in the shard its lock gives, and scopes spread over the shards.
    #[test]
    fn a_scope_has_one_shard_and_scopes_spread() {
        let m: super::ByScope<std::collections::HashMap<(usize, u32), u32>> = Default::default();
        let scopes: Vec<usize> = (0..64).map(|i| 0x1_0000_0000 + i * 0x2_8000).collect();
        for &s in &scopes {
            m.lock(s).insert((s, 1), 7);
        }
        for &s in &scopes {
            assert_eq!(m.lock(s).get(&(s, 1)), Some(&7));
        }
        let (mut used, mut shards) = (0, 0);
        m.each(|shard| {
            used += usize::from(!shard.is_empty());
            shards += 1;
        });
        // The shards are the machine's threads' (four a thread): fewer than the scopes on a small machine.
        assert!(used > shards.min(scopes.len()) / 2, "64 scopes in {used} of {shards} shards");
    }
    use std::sync::Arc;

    /// Readers on many threads see only whole writes: each write keeps the two halves equal.
    #[test]
    fn readers_on_every_shard_see_only_whole_writes() {
        let lock = Arc::new(Sharded::new((0u64, 0u64)));
        let readers: Vec<_> = (0..16).map(|_| {
            let lock = Arc::clone(&lock);
            std::thread::spawn(move || {
                for _ in 0..20_000 {
                    let g = lock.read();
                    assert_eq!(g.0, g.1);
                }
            })
        }).collect();
        for i in 1..=2_000u64 {
            let mut g = lock.write();
            g.0 = i;
            std::hint::black_box(&mut *g);
            g.1 = i;
        }
        for r in readers {
            r.join().unwrap();
        }
        assert_eq!(*lock.read(), (2_000, 2_000));
    }
}
