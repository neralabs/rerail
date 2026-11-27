//! How a reference inside a region is stored: an offset's word and its unit, as one type parameter.
//!
//! An offset counts units from its **base**, which is derived from the address of its own field:
//!
//! - [`Bits`]: the field's first bit; a unit is one bit. A field at any bit (packed columns, succinct structures).
//! - [`Bytes`]: the field's first byte; a unit is a byte. A value at any byte (tapes, cells, strings).
//! - [`Aligned`]: the field's address rounded down to the target's alignment; a unit is that alignment (MLDB's
//!   `MappedPtr`). Every target it reaches is aligned, and a 32-bit word reaches ±8 GiB at 4-byte alignment.
//!
//! An offset of 0 is "none": it would name a target overlapping the field itself, which no real target does.

use crate::Pod;
use crate::kernel::KernelField;
use core::marker::PhantomData;
use core::sync::atomic::Ordering::Relaxed;
use core::sync::atomic::{AtomicI16, AtomicI32, AtomicI64, AtomicU16, AtomicU32, AtomicU64};

/// A word an offset or a length is stored in: an atomic, so a writer may set it through `&self` while its block is
/// open (as `structure!`'s fields are). A relaxed load is a plain load on ARM.
pub trait AtomicWord: Pod + KernelField {
    /// Its bits.
    const BITS: u32;
    const SIGNED: bool;
    /// The least and the most it holds, as an `i64` (a `u64` word's most is `i64::MAX`).
    const MIN: i64;
    const MAX: i64;
    /// Its name in a structure's UKL declaration (`i32`, `u16`): `RelSpan_i32_u32`.
    const SHORT: &'static str;
    fn zero() -> Self;
    fn get(&self) -> i64;
    /// Stores `v`, which the caller has checked lies in `MIN..=MAX`.
    fn put(&self, v: i64);
}

macro_rules! atomic_word {
    ($($a:ty, $t:ty, $short:literal);*) => {$(
        impl AtomicWord for $a {
            const BITS: u32 = <$t>::BITS;
            const SIGNED: bool = <$t>::MIN != 0;
            const MIN: i64 = <$t>::MIN as i64;
            const MAX: i64 = if (<$t>::MAX as u64) > i64::MAX as u64 { i64::MAX } else { <$t>::MAX as i64 };
            const SHORT: &'static str = $short;
            #[inline(always)]
            fn zero() -> Self {
                <$a>::new(0)
            }
            #[inline(always)]
            fn get(&self) -> i64 {
                self.load(Relaxed) as i64
            }
            #[inline(always)]
            fn put(&self, v: i64) {
                debug_assert!((Self::MIN..=Self::MAX).contains(&v));
                self.store(v as $t, Relaxed)
            }
        }
    )*};
}
atomic_word!(AtomicI16, i16, "i16"; AtomicU16, u16, "u16"; AtomicI32, i32, "i32"; AtomicU32, u32, "u32";
    AtomicI64, i64, "i64"; AtomicU64, u64, "u64");

/// The three units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitKind {
    Bits,
    Bytes,
    Aligned,
}

/// What one unit of an offset spans.
pub trait Unit: 'static {
    const KIND: UnitKind;
}

/// One bit, from the field's first bit.
pub enum Bits {}
/// One byte, from the field's first byte.
pub enum Bytes {}
/// The target's alignment, from the field's address rounded down to it.
pub enum Aligned {}

impl Unit for Bits {
    const KIND: UnitKind = UnitKind::Bits;
}
impl Unit for Bytes {
    const KIND: UnitKind = UnitKind::Bytes;
}
impl Unit for Aligned {
    const KIND: UnitKind = UnitKind::Aligned;
}

/// An offset's form: its word and its unit. The one implementor is [`Off`]; the trait is what references are
/// generic over.
///
/// # Safety
/// The implementor is `#[repr(transparent)]` over its word.
pub unsafe trait Offset: Pod {
    type Word: AtomicWord;
    type Unit: Unit;
    fn none() -> Self;
    fn word(&self) -> &Self::Word;

    /// The bit address of the base of the offset held at field address `field`, for a target aligned to `align`.
    #[inline(always)]
    fn base_bits(field: usize, align: usize) -> u64 {
        match Self::Unit::KIND {
            UnitKind::Bits | UnitKind::Bytes => field as u64 * 8,
            UnitKind::Aligned => (field & !(align - 1)) as u64 * 8,
        }
    }

    /// The bits one unit spans, for a target aligned to `align`.
    #[inline(always)]
    fn unit_bits(align: usize) -> u64 {
        match Self::Unit::KIND {
            UnitKind::Bits => 1,
            UnitKind::Bytes => 8,
            UnitKind::Aligned => align as u64 * 8,
        }
    }

    /// The units from the base to the target at bit address `target`, or why it cannot be held.
    fn units_to(field: usize, align: usize, target: u64) -> Result<i64, super::Refusal> {
        let (base, unit) = (Self::base_bits(field, align) as i64, Self::unit_bits(align) as i64);
        let d = target as i64 - base;
        if d % unit != 0 {
            return Err(super::Refusal::Misaligned);
        }
        let u = d / unit;
        if u == 0 || u < Self::Word::MIN || u > Self::Word::MAX {
            return Err(if u == 0 { super::Refusal::Outside } else { super::Refusal::Reach });
        }
        Ok(u)
    }
}

/// An offset of word `W` in unit `U`.
#[repr(transparent)]
pub struct Off<W, U>(W, PhantomData<fn() -> U>);

// SAFETY: transparent over a `Pod` word; the marker holds nothing.
unsafe impl<W: AtomicWord, U: Unit> Pod for Off<W, U> {}

// SAFETY: transparent over its word.
unsafe impl<W: AtomicWord, U: Unit> Offset for Off<W, U> {
    type Word = W;
    type Unit = U;
    #[inline(always)]
    fn none() -> Self {
        Off(W::zero(), PhantomData)
    }
    #[inline(always)]
    fn word(&self) -> &W {
        &self.0
    }
}

impl<W: AtomicWord, U: Unit> KernelField for Off<W, U> {
    fn ty(out: &mut dyn core::fmt::Write) -> core::fmt::Result {
        W::ty(out)
    }
}

/// The default: 32 bits, signed, in units of the target's alignment.
pub type R32 = Off<AtomicI32, Aligned>;
/// 16 bits, within a small area: ±128 KiB at 4-byte alignment.
pub type R16 = Off<AtomicI16, Aligned>;
/// 64 bits, for a region past what 32 reach.
pub type R64 = Off<AtomicI64, Aligned>;
/// 32 bits, forward only: twice the reach of `R32`, for a target always after its reference.
pub type F32 = Off<AtomicU32, Aligned>;
/// 32 bits at any byte.
pub type B32 = Off<AtomicI32, Bytes>;
/// 16 bits at any byte: within ±32 KiB.
pub type B16 = Off<AtomicI16, Bytes>;
/// 32 bits at any bit: within ±256 MiB.
pub type Bit32 = Off<AtomicI32, Bits>;
