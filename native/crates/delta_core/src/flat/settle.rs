//! Settling (*Flat Views and Lifecycles* §3): a structure built in its working form, in a short-lived bucket, is
//! written in its final form into the bucket that keeps it, in two steps. `need` sizes the final form exactly, from the
//! working form alone (MLDB's `indirectBytesRequired`, exact), so the claim in the destination is exact; `settle`
//! writes it there, its parts first and its descriptor last (MLDB's `freeze`), every reference inside its room.
//!
//! On a device the two steps are phases: a count phase computes each item's need, a scan places them, a write phase
//! writes each one (0603's size scripts). It is not a 0353 copy: the working form is the computation's scratch, which
//! no reader reads; the final form is written once, where it is kept.

use super::Region;
use super::arena::{Arena, Full, GRAIN};
use crate::Pod;
use core::mem::MaybeUninit;

/// A final form's exact bytes: its descriptor and its parts, each rounded to `GRAIN` (8 bytes, the alignment every
/// part has at most).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Need {
    pub bytes: usize,
}

impl Need {
    /// Nothing yet.
    pub const fn none() -> Need {
        Need { bytes: 0 }
    }

    /// And `n` items of `T`.
    pub const fn and<T>(self, n: usize) -> Need {
        Need { bytes: self.bytes + (n * size_of::<T>()).next_multiple_of(GRAIN) }
    }
}

/// The exact room a final form is settled into: one claim of its `need`, taken from front to back.
pub struct Room<'d> {
    region: &'d Region<'d>,
    start: usize,
    at: usize,
    end: usize,
}

impl<'d> Room<'d> {
    /// The region its references are set in.
    pub fn region(&self) -> &'d Region<'d> {
        self.region
    }

    /// Its first byte's offset in the region, and its end's.
    pub fn span(&self) -> (usize, usize) {
        (self.start, self.end)
    }

    /// `n` items of `T` (aligned to at most `GRAIN`), the next of its room.
    pub fn take<T: Pod>(&mut self, n: usize) -> &'d mut [MaybeUninit<T>] {
        const { assert!(align_of::<T>() <= GRAIN, "a settled part aligned past 8 bytes") };
        let bytes = (n * size_of::<T>()).next_multiple_of(GRAIN);
        assert!(self.at + bytes <= self.end, "a settle past its need: {} of {} bytes", self.at + bytes - self.start, self.end - self.start);
        let p = self.region.ptr(self.at).cast::<MaybeUninit<T>>();
        self.at += bytes;
        // SAFETY: the room's own next bytes, which nothing else holds (an exact claim, taken once from front to back);
        // aligned, as the claim starts at a grain and every take is a multiple of one.
        unsafe { core::slice::from_raw_parts_mut(p, n) }
    }

    /// `n` items of `T`, each `v`.
    pub fn take_with<T: Pod + Copy>(&mut self, n: usize, v: T) -> &'d mut [T] {
        let r = self.take::<T>(n);
        for x in r.iter_mut() {
            x.write(v);
        }
        // SAFETY: every item written.
        unsafe { &mut *(core::ptr::from_mut(r) as *mut [T]) }
    }

    /// Ends the settle: its need was exact.
    fn finish(self) {
        assert_eq!(self.at, self.end, "a settle that took less than its need");
    }
}

/// A working form's final form, settled exactly.
pub trait Settle {
    /// The final form's descriptor in its destination.
    type Final: Pod;

    /// Its exact bytes, from the working form alone.
    fn need(&self) -> Need;

    /// Writes the final form into `room`, claimed for exactly `need()`: its parts, then its descriptor, every
    /// reference inside `room`.
    fn settle<'d>(&self, room: &mut Room<'d>) -> &'d Self::Final;
}

impl<'r> Arena<'r> {
    /// `s`'s final form, settled into an exact claim of this arena (an out block's: L4, exact claims only).
    pub fn settle<S: Settle>(&self, part: u32, s: &S) -> Result<&S::Final, Full> {
        let need = s.need();
        let at = self.claim_at(part, need.bytes, GRAIN)?;
        let mut room = Room { region: self.region(), start: at, at, end: at + need.bytes };
        let f = s.settle(&mut room);
        room.finish();
        Ok(f)
    }
}
