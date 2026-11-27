//! Items of one size taken and given back by index ([`Pool`]: the writer's areas and rooms), and lists linked through
//! one by index ([`Chain`]: `batch_runtime`'s links). An index is 1-based: 0 is none, as a list's end.
//!
//! A pool's room is fixed when it is made: an item is taken from its free stack, else from its untouched items by one
//! atomic add. Past its room, a take fails as a value (`Full`), its count running on so the owner knows how many were
//! wanted. Takes are shared by every lane of a phase; gives and reads take `&mut self`, so they never meet a take.

use super::arena::{Arena, Full};
use super::grow::{addr, point};
use super::{B32, Rel};
use crate::Pod;
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::sync::atomic::AtomicU32;
use core::sync::atomic::Ordering::{AcqRel, Acquire, Relaxed, Release};

crate::structure! {
    /// A pool's header, any item type.
    pub struct PoolHead {
        /// Its items, `cap` of them.
        pub items: Rel<u8, B32>,
        /// Its free stack: `nfree` indexes given back.
        pub free: Rel<u8, B32>,
        pub cap: u32,
        /// Items never taken start past this, counting on past `cap` (takes that failed).
        pub next: AtomicU32,
        pub nfree: AtomicU32,
        pub part: u32,
    }
}

/// Items of `T` taken by index.
#[repr(transparent)]
pub struct Pool<T> {
    head: PoolHead,
    _t: PhantomData<fn() -> T>,
}

// SAFETY: transparent over its header.
unsafe impl<T: 'static> Pod for Pool<T> {}

impl<T: Pod> Pool<T> {
    /// A pool of `cap` items, in `a`.
    pub fn make<'a>(a: &'a Arena<'_>, part: u32, cap: u32) -> Result<&'a mut Pool<T>, Full> {
        let p = a.claim::<Pool<T>>(part, 1)?;
        let p = p[0].write(Pool {
            head: PoolHead { items: Rel::none(), free: Rel::none(), cap, next: AtomicU32::new(0), nfree: AtomicU32::new(0), part },
            _t: PhantomData,
        });
        p.place(a, cap)?;
        Ok(p)
    }

    fn place(&self, a: &Arena<'_>, cap: u32) -> Result<(), Full> {
        if cap > 0 {
            let items = a.claim_at(self.head.part, cap as usize * size_of::<T>(), align_of::<T>())?;
            let free = a.claim_at(self.head.part, cap as usize * 4, 4)?;
            point(&self.head.items, a, items);
            point(&self.head.free, a, free);
        }
        Ok(())
    }

    pub fn head(&self) -> &PoolHead {
        &self.head
    }

    pub fn cap(&self) -> usize {
        self.head.cap as usize
    }

    /// Items taken and not given back.
    pub fn live(&self) -> usize {
        (self.head.next.load(Acquire).min(self.head.cap) - self.head.nfree.load(Acquire)) as usize
    }

    /// Takes asked for, the failed ones' too, less those given back: what its room must hold.
    pub fn wanted(&self) -> usize {
        (self.head.next.load(Acquire) - self.head.nfree.load(Acquire)) as usize
    }

    #[inline(always)]
    fn item(&self, i: u32) -> *mut T {
        debug_assert!(i >= 1 && i <= self.head.cap);
        core::ptr::with_exposed_provenance_mut::<T>(addr(&self.head.items)).wrapping_add(i as usize - 1)
    }

    #[inline(always)]
    fn free_slot(&self, k: u32) -> &AtomicU32 {
        // SAFETY: slot `k` of its free stack, inside it (`k < cap`).
        unsafe { &*core::ptr::with_exposed_provenance::<AtomicU32>(addr(&self.head.free)).add(k as usize) }
    }

    /// An item, its index and its room: one given back if any, else one never taken. Shared by every lane of a phase.
    #[allow(clippy::mut_from_ref)]
    pub fn take(&self) -> Result<(u32, &mut MaybeUninit<T>), Full> {
        let h = &self.head;
        let mut n = h.nfree.load(Acquire);
        while n > 0 {
            match h.nfree.compare_exchange_weak(n, n - 1, AcqRel, Acquire) {
                // No give runs while a take does (`give` takes `&mut self`): slot `n - 1` holds what was given.
                Ok(_) => return Ok(self.room(self.free_slot(n - 1).load(Relaxed))),
                Err(m) => n = m,
            }
        }
        let i = h.next.fetch_add(1, AcqRel);
        if i >= h.cap {
            let size = size_of::<T>() as u64;
            return Err(Full { part: h.part, need: size as u32, wanted: ((i as u64 + 1) * size).min(u32::MAX as u64) as u32 });
        }
        Ok(self.room(i + 1))
    }

    #[allow(clippy::mut_from_ref)]
    fn room(&self, i: u32) -> (u32, &mut MaybeUninit<T>) {
        // SAFETY: item `i`, which this take alone holds until it is given back (`&mut self`).
        (i, unsafe { &mut *self.item(i).cast::<MaybeUninit<T>>() })
    }

    /// Gives item `i` back, to be taken again.
    pub fn give(&mut self, i: u32) {
        assert!(i >= 1 && i <= self.head.next.load(Relaxed).min(self.head.cap), "item {i} of a pool was never taken");
        let n = self.head.nfree.load(Relaxed);
        debug_assert!(n < self.head.cap);
        self.free_slot(n).store(i, Relaxed);
        self.head.nfree.store(n + 1, Relaxed);
    }

    /// Forgets the takes that failed, to be made again once it has room for them.
    pub fn settle(&mut self) {
        let next = self.head.next.load(Relaxed).min(self.head.cap);
        self.head.next.store(next, Relaxed);
    }

    /// Item `i`, to read: `&mut self`, so no take's room is still being written.
    pub fn get(&mut self, i: u32) -> &T {
        self.read().get(i)
    }

    /// Its items, to read by many lanes: no take or give while it lives.
    pub fn read(&mut self) -> PoolRead<'_, T> {
        self.settle();
        PoolRead { pool: self }
    }

    /// Item `i`, to write (one owner).
    pub fn get_mut(&mut self, i: u32) -> &mut T {
        assert!(i >= 1 && i <= self.head.next.load(Relaxed).min(self.head.cap), "item {i} of a pool was never taken");
        // SAFETY: a taken item, with this borrow its only one.
        unsafe { &mut *self.item(i) }
    }
}

/// A pool's items, read where they lie while nothing takes or gives.
pub struct PoolRead<'p, T> {
    pool: &'p Pool<T>,
}

impl<'p, T: Pod> PoolRead<'p, T> {
    /// Item `i`: taken at some time (its bytes are what its taker, or an earlier one, wrote).
    pub fn get(&self, i: u32) -> &'p T {
        assert!(i >= 1 && i <= self.pool.head.next.load(Relaxed).min(self.pool.head.cap), "item {i} of a pool was never taken");
        // SAFETY: inside its items; nothing writes them while this read lives (it holds the pool's `&mut`); a region's
        // bytes are initialized memory and `T: Pod`.
        unsafe { &*self.pool.item(i) }
    }
}

crate::structure! {
    /// A chain pool's header: its items' pool and each item's next, by index.
    pub struct ChainHead {
        pub pool: PoolHead,
        /// Item `i`'s next at word `i - 1`: 0 ends a list.
        pub next: Rel<u8, B32>,
    }
}

/// Lists of `T` linked by 1-based index, through one pool. Tails are shared: many lists may cons onto one.
#[repr(C)]
pub struct Chain<T> {
    pool: Pool<T>,
    next: Rel<u8, B32>,
}

// SAFETY: two `Pod` fields, no padding (`ChainHead`'s layout).
unsafe impl<T: 'static> Pod for Chain<T> {}

/// A list: its first link, or none.
pub type Link = u32;

impl<T: Pod> Chain<T> {
    /// A chain pool of room for `cap` links, in `a`.
    pub fn make<'a>(a: &'a Arena<'_>, part: u32, cap: u32) -> Result<&'a mut Chain<T>, Full> {
        let c = a.claim::<Chain<T>>(part, 1)?;
        let c = c[0].write(Chain {
            pool: Pool { head: PoolHead { items: Rel::none(), free: Rel::none(), cap, next: AtomicU32::new(0), nfree: AtomicU32::new(0), part }, _t: PhantomData },
            next: Rel::none(),
        });
        c.pool.place(a, cap)?;
        if cap > 0 {
            let at = a.claim_at(part, cap as usize * 4, 4)?;
            point(&c.next, a, at);
        }
        Ok(c)
    }

    pub fn pool(&self) -> &Pool<T> {
        &self.pool
    }

    #[inline(always)]
    fn next_of(&self, i: Link) -> &AtomicU32 {
        debug_assert!(i >= 1 && i <= self.pool.head.cap);
        // SAFETY: word `i - 1` of its next words, inside them.
        unsafe { &*core::ptr::with_exposed_provenance::<AtomicU32>(addr(&self.next)).add(i as usize - 1) }
    }

    /// The list `v` then `tail`: shared by every lane of a phase.
    pub fn cons(&self, tail: Link, v: T) -> Result<Link, Full> {
        let (i, room) = self.pool.take()?;
        room.write(v);
        self.next_of(i).store(tail, Release);
        Ok(i)
    }

    /// `v` pushed onto the list `head` names, which many lanes push onto at once.
    pub fn push(&self, head: &AtomicU32, v: T) -> Result<Link, Full> {
        let (i, room) = self.pool.take()?;
        room.write(v);
        let mut t = head.load(Acquire);
        loop {
            self.next_of(i).store(t, Relaxed);
            match head.compare_exchange_weak(t, i, AcqRel, Acquire) {
                Ok(_) => return Ok(i),
                Err(now) => t = now,
            }
        }
    }

    /// Its lists, to walk by many lanes: no cons or push while it lives.
    pub fn read(&mut self) -> ChainRead<'_, T> {
        self.pool.settle();
        ChainRead { chain: self }
    }
}

/// A chain pool's lists, read where they lie.
pub struct ChainRead<'c, T> {
    chain: &'c Chain<T>,
}

impl<'c, T: Pod> ChainRead<'c, T> {
    /// The list from `l`, first to last.
    pub fn walk(&self, l: Link) -> Walk<'c, T> {
        Walk { chain: self.chain, at: l, left: self.chain.pool.head.cap }
    }
}

/// A list's items, first to last.
pub struct Walk<'c, T> {
    chain: &'c Chain<T>,
    at: Link,
    /// Links it may still take: no list is longer than its pool, so a cycle (an item given back while a list held
    /// it) ends the walk with a panic, never a hang.
    left: u32,
}

impl<'c, T: Pod> Iterator for Walk<'c, T> {
    type Item = &'c T;

    fn next(&mut self) -> Option<&'c T> {
        if self.at == 0 {
            return None;
        }
        let i = self.at;
        self.left = self.left.checked_sub(1).expect("a list longer than its pool: a cycle");
        assert!(i <= self.chain.pool.head.next.load(Relaxed).min(self.chain.pool.head.cap), "a link past a chain's taken items");
        self.at = self.chain.next_of(i).load(Acquire);
        // SAFETY: a taken item, written by its cons before its link was published; nothing writes while the read lives.
        Some(unsafe { &*self.chain.pool.item(i) })
    }
}
