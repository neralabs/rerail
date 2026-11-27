//! References relative to their own field: one target ([`Rel`]), a run of them ([`RelSpan`]), or a run of bits
//! ([`BitSpan`]). The offset's unit decides how a target is read, because Rust gives a reference only to an aligned
//! value:
//!
//! - `Aligned`: `get() -> Option<&T>`, `as_slice() -> &[T]`;
//! - `Bytes`: by value, an unaligned load (`read`, `RelSpan::at`), as UKL's `ukl_ld_*` read;
//! - `Bits`: `BitSpan::get(bit, n)`, at most 57 bits, as UKL's `ukl_ld_bits` reads.
//!
//! A range holding a reference and its target can be moved by `memcpy` with nothing to fix up.

use super::offset::{Aligned, AtomicWord, Bytes, Off, Offset, R32, Unit, UnitKind};
use super::{Refusal, Region};
use crate::Pod;
use crate::kernel::{Decls, KernelField};
use core::fmt::{self, Write};
use core::marker::PhantomData;
use core::ptr::{with_exposed_provenance, with_exposed_provenance_mut};
use core::sync::atomic::AtomicU32;

/// One `T` in the same region, or none.
#[repr(transparent)]
pub struct Rel<T, O: Offset = R32>(O, PhantomData<fn() -> T>);

// SAFETY: transparent over its offset, which is `Pod`.
unsafe impl<T: 'static, O: Offset> Pod for Rel<T, O> {}

impl<T, O: Offset> Rel<T, O> {
    pub fn none() -> Self {
        Rel(O::none(), PhantomData)
    }

    #[inline(always)]
    pub fn is_none(&self) -> bool {
        self.0.word().get() == 0
    }

    /// Its stored units (0: none).
    #[inline(always)]
    pub fn units(&self) -> i64 {
        self.0.word().get()
    }

    #[inline(always)]
    fn field(&self) -> usize {
        core::ptr::from_ref(self).addr()
    }

    /// The target's bit address, or none.
    #[inline(always)]
    pub fn target_bits(&self) -> Option<u64> {
        let u = self.units();
        if u == 0 {
            return None;
        }
        let a = align_of::<T>();
        Some((O::base_bits(self.field(), a) as i64 + u * O::unit_bits(a) as i64) as u64)
    }

    /// Points it at the `T` at bit address `target` (a byte's address times 8 unless the unit is `Bits`), both
    /// inside `region`; refused, and left as it was, where it cannot be held.
    pub fn set_bits(&self, region: &Region<'_>, target: u64) -> Result<(), Refusal> {
        let size = size_of::<T>();
        assert!(size > 0, "a reference to a zero-sized type");
        if !region.holds(self.field(), size_of::<Self>()) || !region.holds_bits(target, size as u64 * 8) {
            return Err(Refusal::Outside);
        }
        if O::Unit::KIND != UnitKind::Bits && !target.is_multiple_of(8) {
            return Err(Refusal::Misaligned);
        }
        let u = O::units_to(self.field(), align_of::<T>(), target)?;
        self.0.word().put(u);
        Ok(())
    }

    /// Points it at `target`, both inside `region`.
    pub fn set(&self, region: &Region<'_>, target: *const T) -> Result<(), Refusal> {
        self.set_bits(region, target.addr() as u64 * 8)
    }

    /// Makes it none.
    pub fn clear(&self) {
        self.0.word().put(0);
    }

    /// The target's address (none, or a bit address that is not a byte's: none).
    #[inline(always)]
    fn target_addr(&self) -> Option<usize> {
        self.target_bits().filter(|b| b.is_multiple_of(8)).map(|b| (b / 8) as usize)
    }
}

impl<T: Pod, W: AtomicWord> Rel<T, Off<W, Aligned>> {
    /// Its target, where it lies.
    #[inline(always)]
    pub fn get(&self) -> Option<&T> {
        // SAFETY: set inside its region (`set_bits`), whose provenance was exposed and whose memory outlives this
        // borrow; aligned by its unit; `T: Pod`.
        self.target_addr().map(|a| unsafe { &*with_exposed_provenance::<T>(a) })
    }

    /// Its target, to write.
    ///
    /// # Safety
    /// Nothing else reads or writes the target while the result lives.
    #[allow(clippy::mut_from_ref)]
    #[inline(always)]
    pub unsafe fn get_mut(&self) -> Option<&mut T> {
        self.target_addr().map(|a| unsafe { &mut *with_exposed_provenance_mut::<T>(a) })
    }
}

impl<T: Pod + Copy, W: AtomicWord> Rel<T, Off<W, Bytes>> {
    /// Its target's value, read where it lies at any byte.
    #[inline(always)]
    pub fn read(&self) -> Option<T> {
        // SAFETY: as `get`, by an unaligned read.
        self.target_addr().map(|a| unsafe { with_exposed_provenance::<T>(a).read_unaligned() })
    }

    /// Writes its target's value.
    ///
    /// # Safety
    /// It is not none, and nothing else reads or writes the target meanwhile.
    #[inline(always)]
    pub unsafe fn write(&self, v: T) {
        let a = self.target_addr().expect("a write through a none reference");
        unsafe { with_exposed_provenance_mut::<T>(a).write_unaligned(v) }
    }
}

impl<T, O: Offset + KernelField> KernelField for Rel<T, O> {
    fn ty(out: &mut dyn Write) -> fmt::Result {
        O::ty(out)
    }
}

/// `len` `T`s in the same region, from its offset's target. `L` is its length's word.
#[repr(C)]
pub struct RelSpan<T, O: Offset = R32, L: AtomicWord = AtomicU32> {
    at: Rel<T, O>,
    len: L,
}

// SAFETY: two `Pod` fields; `NO_PADDING` refuses a pair the compiler pads, at every use.
unsafe impl<T: 'static, O: Offset, L: AtomicWord> Pod for RelSpan<T, O, L> {}

impl<T, O: Offset, L: AtomicWord> RelSpan<T, O, L> {
    /// A pair the compiler pads (a 16-bit offset beside a 32-bit length) is not a layout: refused at compile time.
    const NO_PADDING: () = assert!(
        size_of::<Self>() == size_of::<O>() + size_of::<L>(),
        "a RelSpan whose offset and length the compiler pads: pair words of one width"
    );

    pub fn empty() -> Self {
        let () = Self::NO_PADDING;
        RelSpan { at: Rel::none(), len: L::zero() }
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.len.get() as usize
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Its first `T`'s bit address, or none.
    pub fn target_bits(&self) -> Option<u64> {
        self.at.target_bits()
    }

    /// Points it at the `n` `T`s at bit address `target`, all inside `region`.
    pub fn set_bits(&self, region: &Region<'_>, target: u64, n: usize) -> Result<(), Refusal> {
        let () = Self::NO_PADDING;
        if n as i64 > L::MAX {
            return Err(Refusal::Reach);
        }
        if n == 0 {
            self.at.clear();
            self.len.put(0);
            return Ok(());
        }
        let bytes = (size_of::<T>() as u64).checked_mul(n as u64).ok_or(Refusal::Outside)?;
        if !region.holds_bits(target, bytes * 8) {
            return Err(Refusal::Outside);
        }
        self.at.set_bits(region, target)?;
        self.len.put(n as i64);
        Ok(())
    }

    /// Points it at the `n` `T`s from `target`, all inside `region`.
    pub fn set(&self, region: &Region<'_>, target: *const T, n: usize) -> Result<(), Refusal> {
        self.set_bits(region, target.addr() as u64 * 8, n)
    }

    #[inline(always)]
    fn first(&self) -> Option<usize> {
        self.at.target_addr()
    }
}

impl<T: Pod, W: AtomicWord, L: AtomicWord> RelSpan<T, Off<W, Aligned>, L> {
    /// Its `T`s, where they lie.
    #[inline(always)]
    pub fn as_slice(&self) -> &[T] {
        match self.first() {
            // SAFETY: set inside its region, aligned by its unit, `len` of them (`set_bits`).
            Some(a) if !self.is_empty() => unsafe { core::slice::from_raw_parts(with_exposed_provenance::<T>(a), self.len()) },
            _ => &[],
        }
    }

    /// Its `T`s, to write.
    ///
    /// # Safety
    /// Nothing else reads or writes them while the result lives.
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn as_mut_slice(&self) -> &mut [T] {
        match self.first() {
            Some(a) if !self.is_empty() => unsafe { core::slice::from_raw_parts_mut(with_exposed_provenance_mut::<T>(a), self.len()) },
            _ => &mut [],
        }
    }
}

impl<T: Pod + Copy, W: AtomicWord, L: AtomicWord> RelSpan<T, Off<W, Bytes>, L> {
    /// Its `i`th `T`'s value, read where it lies at any byte.
    #[inline(always)]
    pub fn at(&self, i: usize) -> T {
        assert!(i < self.len(), "item {i} of a span of {}", self.len());
        let a = self.first().expect("a span of items is set");
        // SAFETY: item `i` lies inside the span (checked), and the span inside its region.
        unsafe { with_exposed_provenance::<T>(a + i * size_of::<T>()).read_unaligned() }
    }

    /// Writes its `i`th `T`.
    ///
    /// # Safety
    /// Nothing else reads or writes item `i` meanwhile.
    pub unsafe fn put(&self, i: usize, v: T) {
        assert!(i < self.len(), "item {i} of a span of {}", self.len());
        let a = self.first().expect("a span of items is set");
        unsafe { with_exposed_provenance_mut::<T>(a + i * size_of::<T>()).write_unaligned(v) }
    }
}

/// `bits` bits in the same region, from its offset's target: a packed column, a selector table, a succinct
/// structure. Read `n` bits at a time, at most 57, from any bit (UKL's `ukl_ld_bits`).
#[repr(C)]
pub struct BitSpan<O: Offset = super::Bit32, L: AtomicWord = AtomicU32> {
    at: Rel<u8, O>,
    bits: L,
}

// SAFETY: as `RelSpan`'s.
unsafe impl<O: Offset, L: AtomicWord> Pod for BitSpan<O, L> {}

impl<O: Offset, L: AtomicWord> BitSpan<O, L> {
    const NO_PADDING: () = assert!(
        size_of::<Self>() == size_of::<O>() + size_of::<L>(),
        "a BitSpan whose offset and length the compiler pads: pair words of one width"
    );

    pub fn empty() -> Self {
        let () = Self::NO_PADDING;
        BitSpan { at: Rel::none(), bits: L::zero() }
    }

    #[inline(always)]
    pub fn bits(&self) -> u64 {
        self.bits.get() as u64
    }

    /// Points it at the `n` bits from bit address `target`, all inside `region`.
    pub fn set_bits(&self, region: &Region<'_>, target: u64, n: u64) -> Result<(), Refusal> {
        let () = Self::NO_PADDING;
        if n as i64 > L::MAX || n > i64::MAX as u64 {
            return Err(Refusal::Reach);
        }
        if n == 0 {
            self.at.clear();
            self.bits.put(0);
            return Ok(());
        }
        if !region.holds_bits(target, n) || !region.holds(core::ptr::from_ref(self).addr(), size_of::<Self>()) {
            return Err(Refusal::Outside);
        }
        // A bit span's target is its first bit: the `u8` it lies in need not be wholly inside, so the reference's
        // own check is made on the bit alone.
        if O::Unit::KIND != UnitKind::Bits && !target.is_multiple_of(8) {
            return Err(Refusal::Misaligned);
        }
        let u = O::units_to(core::ptr::from_ref(&self.at).addr(), 1, target)?;
        self.at.0.word().put(u);
        self.bits.put(n as i64);
        Ok(())
    }

    /// Its first bit's address, or none.
    pub fn target_bits(&self) -> Option<u64> {
        self.at.target_bits()
    }

    #[inline(always)]
    fn first_bit(&self) -> u64 {
        self.at.target_bits().expect("a bit span is set")
    }

    /// The `n` bits (at most 57) from bit `b` of the span, as the low bits of the result.
    #[inline(always)]
    pub fn get(&self, b: u64, n: u32) -> u64 {
        assert!(n <= 57 && b.checked_add(n as u64).is_some_and(|e| e <= self.bits()), "bits {b}..+{n} of a span of {}", self.bits());
        if n == 0 {
            return 0;
        }
        let at = self.first_bit() + b;
        let (byte, r) = ((at / 8) as usize, (at % 8) as u32);
        let k = (r + n).div_ceil(8) as usize;
        let mut w = 0u64;
        for i in 0..k {
            // SAFETY: the bytes the bits lie in, inside the span (checked) and so inside its region.
            w |= (unsafe { *with_exposed_provenance::<u8>(byte + i) } as u64) << (8 * i);
        }
        (w >> r) & ((1u64 << n) - 1)
    }

    /// Entry `i` of `w`-bit entries packed from the span's first bit (UKL's `ukl_ld_packed`).
    #[inline(always)]
    pub fn packed(&self, i: u64, w: u32) -> u64 {
        self.get(i * w as u64, w)
    }

    /// Writes the low `n` bits of `v` (at most 57) at bit `b` of the span, the bits around them kept.
    ///
    /// # Safety
    /// Nothing else reads or writes the bytes those bits lie in meanwhile.
    pub unsafe fn put(&self, b: u64, n: u32, v: u64) {
        assert!(n <= 57 && b.checked_add(n as u64).is_some_and(|e| e <= self.bits()), "bits {b}..+{n} of a span of {}", self.bits());
        if n == 0 {
            return;
        }
        let at = self.first_bit() + b;
        let (byte, r) = ((at / 8) as usize, (at % 8) as u32);
        let k = (r + n).div_ceil(8) as usize;
        let m = ((1u64 << n) - 1) << r;
        let v = (v << r) & m;
        for i in 0..k {
            let p = with_exposed_provenance_mut::<u8>(byte + i);
            let (bm, bv) = ((m >> (8 * i)) as u8, (v >> (8 * i)) as u8);
            // SAFETY: as `get`'s, with the one writer the caller promises.
            unsafe { *p = (*p & !bm) | bv };
        }
    }
}

/// A span's UKL declaration's name, by its words: `RelSpan_i32_u32`.
fn span_name(kind: &'static str, o: &'static str, l: &'static str) -> Option<&'static str> {
    macro_rules! names {
        ($($k:literal, $o:literal, $l:literal);*) => {
            match (kind, o, l) {
                $(($k, $o, $l) => Some(concat!($k, "_", $o, "_", $l)),)*
                _ => None,
            }
        };
    }
    names!("RelSpan", "i16", "u16"; "RelSpan", "u16", "u16"; "RelSpan", "i32", "u32"; "RelSpan", "u32", "u32";
        "RelSpan", "i64", "u64"; "RelSpan", "u64", "u64"; "RelSpan", "i32", "i32"; "RelSpan", "i64", "u32";
        "BitSpan", "i32", "u32"; "BitSpan", "u32", "u32"; "BitSpan", "i64", "u64"; "BitSpan", "i16", "u16")
}

fn declare_span<O: Offset + KernelField, L: AtomicWord>(kind: &'static str, len: &'static str, out: &mut dyn Decls) -> fmt::Result {
    let name = span_name(kind, O::Word::SHORT, L::SHORT).ok_or(fmt::Error)?;
    let size = size_of::<O>() + size_of::<L>();
    out.declare(name, &|s: &mut dyn Write| {
        write!(s, "typedef struct {name} {{\n    ")?;
        O::decl("at", s)?;
        s.write_str(";\n    ")?;
        L::decl(len, s)?;
        write!(s, ";\n}} {name};\n")?;
        writeln!(s, "UKL_STATIC_ASSERT(sizeof({name}) == {size}, \"{name}: size\");")?;
        writeln!(s, "UKL_STATIC_ASSERT(UKL_OFFSETOF({name}, {len}) == {}, \"{name}.{len}\");", size_of::<O>())
    })
}

fn span_ty<O: Offset, L: AtomicWord>(kind: &'static str, out: &mut dyn Write) -> fmt::Result {
    out.write_str(span_name(kind, O::Word::SHORT, L::SHORT).ok_or(fmt::Error)?)
}

impl<T, O: Offset + KernelField, L: AtomicWord> KernelField for RelSpan<T, O, L> {
    fn ty(out: &mut dyn Write) -> fmt::Result {
        span_ty::<O, L>("RelSpan", out)
    }
    fn deps(out: &mut dyn Decls) -> fmt::Result {
        declare_span::<O, L>("RelSpan", "len", out)
    }
}

impl<O: Offset + KernelField, L: AtomicWord> KernelField for BitSpan<O, L> {
    fn ty(out: &mut dyn Write) -> fmt::Result {
        span_ty::<O, L>("BitSpan", out)
    }
    fn deps(out: &mut dyn Decls) -> fmt::Result {
        declare_span::<O, L>("BitSpan", "bits", out)
    }
}
