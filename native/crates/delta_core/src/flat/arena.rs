//! An arena: a region whose header holds its own top, so a twin on the CPU and a kernel on a GPU take room from it
//! the same way, with one atomic add (0603's program arena, `join_stages`' text arena). Its room comes back whole: at
//! the end of its phase call or its launch, or at a round's end (`next_round`), never a claim at a time.
//!
//! A claim past the room fails as a value ([`Full`]): the part that ran out and the bytes it asked for, and the arena
//! keeps the first failure. The top counts on past the room, so a full arena says how much was wanted (`wanted`): the
//! host grows the bucket by that much and runs the launch again (D6). A claim's room is never written past.
//!
//! Every lifecycle bucket but the stack's and the host's is one or more arenas ([`Bucket`]).

use super::{Refusal, Region};
use crate::Pod;
use core::mem::MaybeUninit;
use core::sync::atomic::AtomicU32;
use core::sync::atomic::Ordering::{AcqRel, Acquire, Relaxed};

/// The lifecycle bucket an arena's structures live in (*Flat Views and Lifecycles* §2). The stack's are fixed arrays
/// and the host's are Rust's heap, so neither is an arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Bucket {
    /// One phase call's: its region of the scratch slot, given back at the call's end.
    Phase = 2,
    /// One launch's (or one round of it): working forms shared by its phases and lanes.
    Launch = 3,
    /// An out block's: final forms, exact claims only, kept while a generation refers to the block.
    Out = 4,
    /// A kept placement's: final forms re-encoded at seal or roll-up.
    Kept = 5,
}

impl Bucket {
    fn of(v: u32) -> Option<Bucket> {
        Some(match v {
            2 => Bucket::Phase,
            3 => Bucket::Launch,
            4 => Bucket::Out,
            5 => Bucket::Kept,
            _ => return None,
        })
    }
}

crate::structure! {
    /// An arena's header, at its region's first byte; its room is the rest of the region.
    pub struct ArenaHead {
        /// "ARNA": an arena's header.
        pub magic: u32,
        /// Its bucket (`Bucket`).
        pub bucket: u32,
        /// Bytes given out past the header, counting on past `cap` (claims that failed).
        pub top: AtomicU32,
        /// Bytes past the header.
        pub cap: u32,
        /// The first failed claim's part, plus one (0: none), and the bytes it asked for.
        pub fail: AtomicU32,
        pub fail_need: AtomicU32,
        /// The round its stamped tables are stamped with: moved on by `next_round`.
        pub round: AtomicU32,
        pub pad: u32,
    }
}

pub const MAGIC: u32 = u32::from_le_bytes(*b"ARNA");
/// The header's bytes: claims start past them.
pub const HEAD: usize = size_of::<ArenaHead>();
/// The most room an arena has: its top counts on past it, and must not wrap.
pub const MAX_CAP: usize = 1 << 31;
/// Every claim's room is a multiple of this, and starts at one: so a claim of `align` at most 8 needs no padding.
pub const GRAIN: usize = 8;

/// A claim that did not fit: the part that asked, the bytes it asked for, and everything the arena was asked for so
/// far (its room and past it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Full {
    pub part: u32,
    pub need: u32,
    pub wanted: u32,
}

/// An arena over a region. Its views live as long as the borrow of it that claimed them, so `reset` and
/// `next_round`, which take `&mut self`, cannot run while one does (rule L3):
///
/// ```compile_fail,E0502
/// use rerail_delta_core::flat::{Arena, Bucket, Region};
/// let mut bytes = [0u64; 64];
/// let bytes = unsafe { core::slice::from_raw_parts_mut(bytes.as_mut_ptr().cast::<u8>(), 512) };
/// let mut a = Arena::init(Region::of(bytes), Bucket::Phase).unwrap();
/// let v = a.claim_with::<u32>(0, 4, 1).unwrap();
/// a.reset();
/// v[0] = 2; // a view used after its room was given back
/// ```
///
/// Nor can a view outlive the arena it came from:
///
/// ```compile_fail,E0597
/// use rerail_delta_core::flat::{Arena, Bucket, Region};
/// let mut bytes = [0u64; 64];
/// let bytes = unsafe { core::slice::from_raw_parts_mut(bytes.as_mut_ptr().cast::<u8>(), 512) };
/// let v = {
///     let a = Arena::init(Region::of(bytes), Bucket::Phase).unwrap();
///     a.claim_with::<u32>(0, 4, 1).unwrap()
/// };
/// v[0] = 2;
/// ```
pub struct Arena<'r> {
    region: Region<'r>,
}

/// A place in an arena's room to empty it back to: what lies below it stays (a launch's tables kept across its
/// rounds), what lies past it is given back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mark(u32);

impl<'r> Arena<'r> {
    /// Makes an arena of `bucket` over `region`: its header written, its room empty. The region's start must be
    /// aligned to `GRAIN`; past `MAX_CAP` bytes of room, the rest is not used.
    pub fn init(region: Region<'r>, bucket: Bucket) -> Result<Arena<'r>, Refusal> {
        if region.len() < HEAD {
            return Err(Refusal::Outside);
        }
        if !region.addr().is_multiple_of(GRAIN) {
            return Err(Refusal::Misaligned);
        }
        let cap = ((region.len() - HEAD).min(MAX_CAP) / GRAIN * GRAIN) as u32;
        let head = ArenaHead {
            magic: MAGIC,
            bucket: bucket as u32,
            top: AtomicU32::new(0),
            cap,
            fail: AtomicU32::new(0),
            fail_need: AtomicU32::new(0),
            round: AtomicU32::new(1),
            pad: 0,
        };
        // SAFETY: the region's first bytes, in bounds and aligned (checked), its own for 'r.
        unsafe { region.ptr(0).cast::<ArenaHead>().write(head) };
        Ok(Arena { region })
    }

    /// The arena another made over `region` (a host, or the launch before): its header as it lies.
    ///
    /// # Safety
    /// No other handle of this arena resets it, or moves its round on, while this one's views live.
    pub unsafe fn open(region: Region<'r>) -> Result<Arena<'r>, Refusal> {
        let ok = {
            let head = region.view::<ArenaHead>(0)?;
            head.magic == MAGIC && Bucket::of(head.bucket).is_some() && HEAD + head.cap as usize <= region.len()
        };
        if !ok {
            return Err(Refusal::Outside);
        }
        Ok(Arena { region })
    }

    #[inline(always)]
    fn head(&self) -> &ArenaHead {
        // SAFETY: written at `init` (or found at `open`), in bounds and aligned.
        unsafe { &*self.region.ptr(0).cast::<ArenaHead>() }
    }

    /// Its region: references set in it are checked against it.
    pub fn region(&self) -> &Region<'r> {
        &self.region
    }

    pub fn bucket(&self) -> Bucket {
        Bucket::of(self.head().bucket).expect("an arena's bucket")
    }

    /// Bytes of room.
    pub fn cap(&self) -> usize {
        self.head().cap as usize
    }

    /// Bytes given out, at most the room.
    pub fn used(&self) -> usize {
        (self.head().top.load(Acquire) as usize).min(self.cap())
    }

    /// Bytes asked for, the failed claims' too: what the room must be for the same work to fit.
    pub fn wanted(&self) -> usize {
        self.head().top.load(Acquire) as usize
    }

    /// The round stamped tables are stamped with.
    pub fn round(&self) -> u32 {
        self.head().round.load(Acquire)
    }

    /// The first claim that failed, or none.
    pub fn failure(&self) -> Option<Full> {
        let h = self.head();
        let part = h.fail.load(Acquire);
        (part != 0).then(|| Full { part: part - 1, need: h.fail_need.load(Relaxed), wanted: h.top.load(Relaxed) })
    }

    /// `bytes` of room at an address aligned to `align` (a power of two): its offset in the region. One atomic add
    /// of the room rounded to `GRAIN`, and, for an alignment past it, what aligning can skip.
    pub fn claim_at(&self, part: u32, bytes: usize, align: usize) -> Result<usize, Full> {
        debug_assert!(align.is_power_of_two());
        let pad = align.saturating_sub(GRAIN);
        let room = bytes.checked_next_multiple_of(GRAIN).and_then(|b| b.checked_add(pad)).filter(|&r| r <= MAX_CAP);
        let h = self.head();
        let Some(room) = room else {
            return Err(self.fail(part, bytes, h.top.load(Relaxed)));
        };
        let old = h.top.fetch_add(room as u32, AcqRel) as usize;
        if old + room > h.cap as usize {
            return Err(self.fail(part, room, (old + room) as u32));
        }
        let start = HEAD + old;
        let at = (self.region.addr() + start).next_multiple_of(align) - self.region.addr();
        debug_assert!(at + bytes <= HEAD + old + room);
        Ok(at)
    }

    #[cold]
    fn fail(&self, part: u32, need: usize, wanted: u32) -> Full {
        let h = self.head();
        let need = need.min(u32::MAX as usize) as u32;
        if h.fail.compare_exchange(0, part + 1, AcqRel, Relaxed).is_ok() {
            h.fail_need.store(need, Relaxed);
        }
        Full { part, need, wanted }
    }

    /// `n` `T`s of room, their contents unwritten.
    #[allow(clippy::mut_from_ref)]
    pub fn claim<T: Pod>(&self, part: u32, n: usize) -> Result<&mut [MaybeUninit<T>], Full> {
        let bytes = n.saturating_mul(size_of::<T>());
        let at = self.claim_at(part, bytes, align_of::<T>())?;
        // SAFETY: room no other claim overlaps until `reset`, which takes `&mut self`; in the region, aligned.
        Ok(unsafe { core::slice::from_raw_parts_mut(self.region.ptr(at).cast::<MaybeUninit<T>>(), n) })
    }

    /// `n` `T`s, each `v`.
    #[allow(clippy::mut_from_ref)]
    pub fn claim_with<T: Pod + Copy>(&self, part: u32, n: usize, v: T) -> Result<&mut [T], Full> {
        let room = self.claim::<T>(part, n)?;
        for x in room.iter_mut() {
            x.write(v);
        }
        // SAFETY: every item written.
        Ok(unsafe { &mut *(core::ptr::from_mut(room) as *mut [T]) })
    }

    /// A copy of `src` (a working form's values, as settling writes them).
    #[allow(clippy::mut_from_ref)]
    pub fn claim_copy<T: Pod + Copy>(&self, part: u32, src: &[T]) -> Result<&mut [T], Full> {
        let room = self.claim::<T>(part, src.len())?;
        for (x, v) in room.iter_mut().zip(src) {
            x.write(*v);
        }
        // SAFETY: every item written.
        Ok(unsafe { &mut *(core::ptr::from_mut(room) as *mut [T]) })
    }

    /// One `T` (a structure's descriptor), written as `v`, shared: its fields that change are atomics.
    pub fn put<T: Pod>(&self, part: u32, v: T) -> Result<&T, Full> {
        let room = self.claim::<T>(part, 1)?;
        Ok(&*room[0].write(v))
    }

    /// Where its room is used to now.
    pub fn mark(&self) -> Mark {
        Mark(self.used() as u32)
    }

    /// Gives back every claim past `m`, and forgets the failures.
    pub fn reset_to(&mut self, m: Mark) {
        let h = self.head();
        h.top.store(m.0.min(h.cap), Relaxed);
        h.fail.store(0, Relaxed);
        h.fail_need.store(0, Relaxed);
    }

    /// Empties it: every claim given back.
    pub fn reset(&mut self) {
        self.reset_to(Mark(0));
    }

    /// Ends a round: every claim past `keep` given back, and the round moved on, so a stamped table kept below
    /// `keep` reads as empty without being cleared (`batch_runtime`'s generations). Its views are gone with the
    /// borrow; a table kept across rounds is found again by its offset (`region().view`).
    pub fn next_round(&mut self, keep: Mark) -> u32 {
        self.reset_to(keep);
        self.head().round.fetch_add(1, AcqRel) + 1
    }
}
