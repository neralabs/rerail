//! Structures in flat memory: fixed layouts read and written where they lie, through views that never allocate
//! (*Flat Views and Lifecycles*, 0686). A twin on the CPU and a kernel on a GPU run the same structure over the same
//! bytes.
//!
//! - [`offset`]: how a reference inside a region is stored. An [`Offset`] is a type parameter: its word (16, 32 or 64
//!   bits, signed or not) and its unit ([`Bits`], [`Bytes`], or the target's alignment, [`Aligned`]). [`R32`] is the
//!   default.
//! - [`arena`]: a region whose header holds its own top ([`Arena`]): room claimed by one atomic add, a failed claim a
//!   value ([`Full`]), room given back whole, at a phase's or a launch's end or a round's ([`Mark`]). Each lifecycle
//!   [`Bucket`] (phase, launch, out block, kept) is arenas.
//! - Working forms, in a phase's or a launch's arena, shaped for building: [`Grow`] (a growing vector), [`Pool`] and
//!   [`Chain`] (items and lists by index), [`Map`] and [`Stamped`] (hash tables, the latter emptied by a round),
//!   [`Stack`] (its top on the machine stack, the rest spilled). Each is a `structure!` header the kernels share, its
//!   parts reached by offsets, and a typed view; many lanes claim through `&self`, one owner grows through `&mut`.
//! - Final forms, in an out block or a kept placement, shaped for reading: [`IntTable`] (integers in the fewest bits,
//!   `IntCol`'s encodings, its encoder [`encode`]), [`Strings`], [`Sorted`], [`Csr`] and the floor's Bloom filter, each
//!   settled exactly from a working form ([`Settle`]: `need`, then `settle` into a claim of exactly that).
//! - [`rel`]: references relative to their own field ([`Rel`], [`RelSpan`], [`BitSpan`]), as the floor's `refs::Ptr`
//!   and MLDB's `MappedPtr` are: a structure is correct at any address, and a method reaches its parts from `&self`.
//!
//! Rule L1: a reference points only within its own [`Region`], and a region is one lifecycle bucket's. `set` checks
//! it, and refuses ([`Refusal`]) a target outside the region, past the offset's reach, or not at a whole unit.
//! Across regions a reference is a block id and offset (`block::Ref`), only to a frozen block (L2).
//!
//! Addresses: a view's pointer is made from an address with the region's exposed provenance
//! (`with_exposed_provenance`): a region exposes its pointer when it is made, so any byte of it may be reached from
//! any view in it, as a self-relative reference must.

pub mod arena;
pub mod encode;
pub mod finals;
pub mod grow;
pub mod map;
pub mod offset;
pub mod pool;
pub mod rel;
pub mod settle;
pub mod stack;

pub use offset::{Aligned, AtomicWord, B16, B32, Bit32, Bits, Bytes, F32, Off, Offset, R16, R32, R64, Unit, UnitKind};
pub use arena::{Arena, ArenaHead, Bucket, Full, Mark};
pub use finals::{BloomFrom, Csr, CsrFrom, IntFrom, IntTable, Sorted, SortedFrom, Strings, StringsFrom};
pub use grow::{Grow, GrowHead};
pub use map::{Claimed, Least, Map, MapHead, Stamped, StampedHead, Word};
pub use pool::{Chain, ChainHead, Link, Pool, PoolHead};
pub use settle::{Need, Room, Settle, settle_into};
pub use stack::Stack;
pub use rel::{BitSpan, Rel, RelSpan};

use core::marker::PhantomData;

/// Why a reference or a claim was refused. A phase returns it as its refusal (D6), never panics on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The reference's field, or its target, is not inside the region.
    Outside,
    /// The target is past what the offset's word reaches in its unit.
    Reach,
    /// The target is not a whole number of units from the offset's base (an `Aligned` target misaligned).
    Misaligned,
}

/// The bytes one lifecycle bucket's structures lie in: where they start and how many there are. A bound and a
/// provenance, not a reference: views into it are made by address, so many views (and their atomics' writers) may
/// share it. Its lifetime is the memory's, and a view's is at most the borrow of the region (or its arena) it came
/// from (rule L3). Not `Copy`: one owner, so an arena over it is its only handle.
#[derive(Debug)]
pub struct Region<'r> {
    at: usize,
    len: usize,
    _r: PhantomData<&'r mut [u8]>,
}

impl<'r> Region<'r> {
    /// The region over `bytes`, held for `'r`.
    pub fn of(bytes: &'r mut [u8]) -> Region<'r> {
        let p = bytes.as_mut_ptr();
        Region { at: p.expose_provenance(), len: bytes.len(), _r: PhantomData }
    }

    /// The region of `len` bytes at `p`.
    ///
    /// # Safety
    /// `p..p + len` is memory that lives for `'r`, which nothing else reaches while the region's views do except
    /// through atomics.
    pub unsafe fn from_raw(p: *mut u8, len: usize) -> Region<'r> {
        Region { at: p.expose_provenance(), len, _r: PhantomData }
    }

    /// Its first byte's address.
    pub fn addr(&self) -> usize {
        self.at
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Whether the `n` bytes at address `a` lie inside it.
    pub fn holds(&self, a: usize, n: usize) -> bool {
        a >= self.at && n <= self.len && a - self.at <= self.len - n
    }

    /// Whether the `n` bits at bit address `b` lie inside it.
    pub fn holds_bits(&self, b: u64, n: u64) -> bool {
        let (lo, hi) = (self.at as u64 * 8, (self.at + self.len) as u64 * 8);
        b >= lo && n <= hi - lo && b - lo <= hi - lo - n
    }

    /// A pointer to byte `off`, with the region's provenance.
    pub fn ptr(&self, off: usize) -> *mut u8 {
        debug_assert!(off <= self.len, "byte {off} past a region of {}", self.len);
        core::ptr::with_exposed_provenance_mut(self.at + off)
    }

    /// The `T` at byte `off`: in bounds and aligned, or refused.
    pub fn view<T: crate::Pod>(&self, off: usize) -> Result<&T, Refusal> {
        let a = self.at.checked_add(off).ok_or(Refusal::Outside)?;
        if !self.holds(a, size_of::<T>()) {
            return Err(Refusal::Outside);
        }
        if !a.is_multiple_of(align_of::<T>()) {
            return Err(Refusal::Misaligned);
        }
        // SAFETY: in bounds and aligned (checked); the region's memory lives for 'r; `T: Pod` has no invalid bit
        // pattern, and what changes in it while shared is atomic.
        Ok(unsafe { &*core::ptr::with_exposed_provenance::<T>(a) })
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_arena;
#[cfg(test)]
mod tests_final;
#[cfg(test)]
mod tests_work;
