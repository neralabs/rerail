//! A growing vector in an arena ([`Grow`]): `lp_alloc`'s regions, `Rows`' claims. Its header holds where its items
//! are, how many, its room and a spare; past its room it moves to room twice as large in its arena, and the old room
//! becomes its spare (`lp_spare`: the larger one kept). A growth the spare holds takes it instead of the arena's room,
//! and an owner of several vectors passes one's spare to another (`give_spare`), as `lp_alloc`'s group shares its
//! one. Many lanes claim items at once by one atomic add on its length (`claim`); one owner pushes, pops and grows
//! (`&mut self`).
//!
//! A shared claim past the room fails as a value (`Full`): its length counts on, so the owner knows how many were
//! wanted, grows to them (`reserve`) and the launch runs the failed claims again. What was claimed is the prefix of
//! the claims that fit (`fit`), since claims take their place in the order of the atomic add.

use super::arena::{Arena, Full};
use super::{B32, Rel};
use crate::Pod;
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::sync::atomic::AtomicU32;
use core::sync::atomic::Ordering::{AcqRel, Acquire, Relaxed};

crate::structure! {
    /// A growing vector's header, any item type (a kernel's view of a `Grow<T>`).
    pub struct GrowHead {
        /// Its first item, in the same arena.
        pub at: Rel<u8, B32>,
        /// Items claimed, counting on past `cap` (claims that failed).
        pub len: AtomicU32,
        /// The end of the claims that fit: its items are `0..fit`.
        pub fit: AtomicU32,
        /// Items its room holds.
        pub cap: AtomicU32,
        /// The part its claims are made for (`Full::part`).
        pub part: u32,
        /// A room it gave up, in the same arena, and its bytes (0: none): what a growth takes before the arena's.
        pub spare: Rel<u8, B32>,
        pub spare_bytes: u32,
        pub pad: u32,
    }
}

/// The address a byte offset points to (none: 0).
#[inline(always)]
pub(crate) fn addr(r: &Rel<u8, B32>) -> usize {
    r.target_bits().map_or(0, |b| (b / 8) as usize)
}

/// Points `r` (in arena `a`) at byte `at` of the arena.
#[inline(always)]
pub(crate) fn point(r: &Rel<u8, B32>, a: &Arena<'_>, at: usize) {
    r.set(a.region(), a.region().ptr(at)).expect("an arena's room is within a byte offset's reach");
}

/// `n` items of `T` growing in an arena. Made by `make`, in the arena it grows in.
#[repr(transparent)]
pub struct Grow<T> {
    head: GrowHead,
    _t: PhantomData<fn() -> T>,
}

// SAFETY: transparent over its header, which is `Pod`.
unsafe impl<T: 'static> Pod for Grow<T> {}

impl<T: Pod + Copy> Grow<T> {
    /// A vector of room for `cap` items, in `a`.
    pub fn make<'a>(a: &'a Arena<'_>, part: u32, cap: u32) -> Result<&'a mut Grow<T>, Full> {
        let g = a.claim::<Grow<T>>(part, 1)?;
        let g = g[0].write(Grow {
            head: GrowHead { at: Rel::none(), len: AtomicU32::new(0), fit: AtomicU32::new(0), cap: AtomicU32::new(0), part, spare: Rel::none(), spare_bytes: 0, pad: 0 },
            _t: PhantomData,
        });
        if cap > 0 {
            let at = a.claim_at(part, cap as usize * size_of::<T>(), align_of::<T>())?;
            point(&g.head.at, a, at);
            g.head.cap.store(cap, Relaxed);
        }
        Ok(g)
    }

    /// Its header, as a kernel reads it.
    pub fn head(&self) -> &GrowHead {
        &self.head
    }

    #[inline(always)]
    fn base(&self) -> *mut T {
        core::ptr::with_exposed_provenance_mut(addr(&self.head.at))
    }

    /// Items it holds: the claims that fit.
    pub fn len(&self) -> usize {
        self.head.fit.load(Acquire) as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn cap(&self) -> usize {
        self.head.cap.load(Acquire) as usize
    }

    /// Items asked for, the failed claims' too: what its room must hold for them to fit.
    pub fn wanted(&self) -> usize {
        self.head.len.load(Acquire) as usize
    }

    /// `n` items' room, claimed by one atomic add: shared by every lane of a phase. Past its room, `Full`, its length
    /// counting on (`wanted`); the owner grows it to that and the claim runs again.
    #[allow(clippy::mut_from_ref)]
    pub fn claim(&self, n: u32) -> Result<&mut [MaybeUninit<T>], Full> {
        let h = &self.head;
        let old = h.len.fetch_add(n, AcqRel);
        let end = old.checked_add(n).filter(|&e| e as usize <= self.cap());
        let Some(end) = end else {
            let size = size_of::<T>() as u64;
            let sat = |v: u64| v.min(u32::MAX as u64) as u32;
            return Err(Full { part: h.part, need: sat(n as u64 * size), wanted: sat((old as u64 + n as u64) * size) });
        };
        h.fit.fetch_max(end, AcqRel);
        // SAFETY: items `old..end` of its room, claimed by this add alone; the room moves only under `&mut self`.
        Ok(unsafe { core::slice::from_raw_parts_mut(self.base().add(old as usize).cast::<MaybeUninit<T>>(), n as usize) })
    }

    /// Its length set to what fit: the failed claims forgotten, to be made again.
    fn settle_len(&mut self) {
        let fit = self.head.fit.load(Relaxed);
        self.head.len.store(fit, Relaxed);
    }

    /// Its spare's bytes: a room it gave up (0: none).
    pub fn spare(&self) -> usize {
        self.head.spare_bytes as usize
    }

    /// Room for `more` items past its length: past its room, it moves to its spare where that holds them, else to
    /// room at least twice as large in `a`, the arena it was made in; the room it leaves becomes its spare when larger.
    pub fn reserve(&mut self, a: &Arena<'_>, more: usize) -> Result<(), Full> {
        self.settle_len();
        let (len, cap) = (self.len(), self.cap());
        let want = len.checked_add(more).filter(|&w| w <= u32::MAX as usize).ok_or(Full { part: self.head.part, need: u32::MAX, wanted: u32::MAX })?;
        if want <= cap {
            return Ok(());
        }
        let size = size_of::<T>().max(1);
        let (spare, spare_at) = (self.spare(), addr(&self.head.spare));
        let (at, new, took) = if spare >= want * size && spare_at.is_multiple_of(align_of::<T>()) {
            (spare_at - a.region().addr(), (spare / size).min(u32::MAX as usize), true)
        } else {
            let new = want.max(2 * cap).max(8).min(u32::MAX as usize);
            (a.claim_at(self.head.part, new * size_of::<T>(), align_of::<T>())?, new, false)
        };
        let to = a.region().ptr(at).cast::<T>();
        if len > 0 {
            // SAFETY: the old room's first `len` items, written, into room no claim overlaps (new, or the spare it
            // gave up, which nothing else holds).
            unsafe { core::ptr::copy_nonoverlapping(self.base(), to, len) };
        }
        let (old_at, old_bytes) = (addr(&self.head.at), cap * size_of::<T>());
        if took {
            self.head.spare = Rel::none();
            self.head.spare_bytes = 0;
        }
        if old_bytes > self.spare() {
            point(&self.head.spare, a, old_at - a.region().addr());
            self.head.spare_bytes = old_bytes as u32;
        }
        point(&self.head.at, a, at);
        self.head.cap.store(new as u32, Relaxed);
        Ok(())
    }

    /// Gives its spare to `to`, another vector of its arena, where it is larger than `to`'s (`lp_spare`): an owner's
    /// vectors share the rooms they give up, as `lp_alloc`'s group does.
    pub fn give_spare<U: Pod + Copy>(&mut self, a: &Arena<'_>, to: &mut Grow<U>) {
        let (bytes, at) = (self.spare(), addr(&self.head.spare));
        if bytes > to.spare() {
            point(&to.head.spare, a, at - a.region().addr());
            to.head.spare_bytes = bytes as u32;
            self.head.spare = Rel::none();
            self.head.spare_bytes = 0;
        }
    }

    /// Appends `v`, growing in `a` past its room.
    pub fn push(&mut self, a: &Arena<'_>, v: T) -> Result<(), Full> {
        self.reserve(a, 1)?;
        let n = self.len();
        // SAFETY: inside its room (reserved), one owner.
        unsafe { self.base().add(n).write(v) };
        self.head.fit.store(n as u32 + 1, Relaxed);
        self.head.len.store(n as u32 + 1, Relaxed);
        Ok(())
    }

    /// Its last item, taken off.
    pub fn pop(&mut self) -> Option<T> {
        self.settle_len();
        let n = self.len().checked_sub(1)?;
        self.head.fit.store(n as u32, Relaxed);
        self.head.len.store(n as u32, Relaxed);
        // SAFETY: item `n`, written (inside what fit), one owner.
        Some(unsafe { self.base().add(n).read() })
    }

    /// Empties it; its room stays.
    pub fn clear(&mut self) {
        self.head.fit.store(0, Relaxed);
        self.head.len.store(0, Relaxed);
    }

    /// Its items. `&mut self`, so no lane's claim is still being written.
    pub fn as_slice(&mut self) -> &[T] {
        self.settle_len();
        match self.len() {
            0 => &[],
            // SAFETY: its first `len` items, claimed and written by their claims' ends (the phase that claimed them
            // is over: no claim's borrow lives).
            n => unsafe { core::slice::from_raw_parts(self.base(), n) },
        }
    }

    pub fn as_mut_slice(&mut self) -> &mut [T] {
        self.settle_len();
        match self.len() {
            0 => &mut [],
            // SAFETY: as `as_slice`, with this borrow its only one.
            n => unsafe { core::slice::from_raw_parts_mut(self.base(), n) },
        }
    }
}
