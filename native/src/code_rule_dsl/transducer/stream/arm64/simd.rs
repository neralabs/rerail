//! simdjson's `include/simdjson/arm64/simd.h` in Rust (`docs/parser-generators/09-compile-and-lower.md`
//! §12.4): `simd8<T>` is one NEON register, `simd8x64<T>` one 64-byte block as four of them.
//!
//! Every method is the intrinsic, or the short fixed sequence of intrinsics, that simdjson uses.
//! There are no loops, branches or per-byte reads here (guard G0.4). C++ operators become Rust's
//! operator traits where the result type allows, and methods otherwise: `==` is `eq`, `<=` `le`,
//! `>=` `ge`, `<` `lt`, `>` `gt`.
//!
//! **Safety.** NEON is in the aarch64 baseline, so every intrinsic here may run on any target
//! this module is built for (`#[cfg(target_arch = "aarch64")]`). Rust still asks for `unsafe`
//! unless the calling function names the feature with `#[target_feature]`, which cannot be
//! combined with `#[inline(always)]`. So each body is one `unsafe` block. The only memory
//! accesses are loads and stores of the arrays a method is given, at most 16 bytes from a
//! pointer into them, plus the table rows and output offsets noted where they occur.

use core::arch::aarch64::*;
use core::marker::PhantomData;
use core::ops::{Add, AddAssign, BitAnd, BitAndAssign, BitOr, BitOrAssign, BitXor, BitXorAssign, Not, Sub, SubAssign};

use crate::code_rule_dsl::transducer::stream::internal::simdprune_tables::{BITS_SET_TABLE_256_MUL2, PSHUFB_COMBINE_TABLE, THINTABLE_EPI8};

/// A lane type and the NEON register that holds sixteen of it.
pub trait Lane: Copy {
    type Register: Copy;
    /// The register's bytes, for a table lookup by any lane type (`simd8<uint8_t>(original)`).
    fn bytes(r: Self::Register) -> uint8x16_t;
}

impl Lane for bool {
    type Register = uint8x16_t;
    #[inline(always)]
    fn bytes(r: uint8x16_t) -> uint8x16_t {
        r
    }
}

impl Lane for u8 {
    type Register = uint8x16_t;
    #[inline(always)]
    fn bytes(r: uint8x16_t) -> uint8x16_t {
        r
    }
}

impl Lane for i8 {
    type Register = int8x16_t;
    #[inline(always)]
    fn bytes(r: int8x16_t) -> uint8x16_t {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vreinterpretq_u8_s8(r)
        }
    }
}

/// simdjson's `base_u8<T>`: the lane types held in a `uint8x16_t`, which share its methods.
pub trait BaseU8: Lane<Register = uint8x16_t> {}
impl BaseU8 for bool {}
impl BaseU8 for u8 {}

/// simdjson's `simd8<T>`: one 16-byte register.
#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct Simd8<T: Lane> {
    pub value: T::Register,
    _lane: PhantomData<T>,
}

impl<T: BaseU8> From<uint8x16_t> for Simd8<T> {
    #[inline(always)]
    fn from(value: uint8x16_t) -> Self {
        Simd8 { value, _lane: PhantomData }
    }
}

impl From<int8x16_t> for Simd8<i8> {
    #[inline(always)]
    fn from(value: int8x16_t) -> Self {
        Simd8 { value, _lane: PhantomData }
    }
}

impl<T: BaseU8> Default for Simd8<T> {
    #[inline(always)]
    fn default() -> Self {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vdupq_n_u8(0).into()
        }
    }
}

impl Default for Simd8<i8> {
    #[inline(always)]
    fn default() -> Self {
        Simd8::<i8>::zero()
    }
}

// ---- base_u8<T> ----

impl<T: BaseU8> BitOr for Simd8<T> {
    type Output = Self;
    #[inline(always)]
    fn bitor(self, other: Self) -> Self {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vorrq_u8(self.value, other.value).into()
        }
    }
}

impl<T: BaseU8> BitAnd for Simd8<T> {
    type Output = Self;
    #[inline(always)]
    fn bitand(self, other: Self) -> Self {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vandq_u8(self.value, other.value).into()
        }
    }
}

impl<T: BaseU8> BitXor for Simd8<T> {
    type Output = Self;
    #[inline(always)]
    fn bitxor(self, other: Self) -> Self {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            veorq_u8(self.value, other.value).into()
        }
    }
}

/// `~`: XOR with 0xff, which for `simd8<bool>` is simdjson's `*this ^ true`.
impl<T: BaseU8> Not for Simd8<T> {
    type Output = Self;
    #[inline(always)]
    fn not(self) -> Self {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            veorq_u8(self.value, vdupq_n_u8(0xff)).into()
        }
    }
}

impl<T: BaseU8> BitOrAssign for Simd8<T> {
    #[inline(always)]
    fn bitor_assign(&mut self, other: Self) {
        *self = *self | other;
    }
}

impl<T: BaseU8> BitAndAssign for Simd8<T> {
    #[inline(always)]
    fn bitand_assign(&mut self, other: Self) {
        *self = *self & other;
    }
}

impl<T: BaseU8> BitXorAssign for Simd8<T> {
    #[inline(always)]
    fn bitxor_assign(&mut self, other: Self) {
        *self = *self ^ other;
    }
}

impl<T: BaseU8> Simd8<T> {
    #[inline(always)]
    pub fn bit_andnot(self, other: Self) -> Self {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vbicq_u8(self.value, other.value).into()
        }
    }

    /// simdjson's `operator==`.
    #[inline(always)]
    pub fn eq(self, other: Self) -> Simd8<bool> {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vceqq_u8(self.value, other.value).into()
        }
    }

    /// Byte `i` is byte `i - N`, reaching into the end of `prev_chunk`.
    #[inline(always)]
    pub fn prev<const N: i32>(self, prev_chunk: Self) -> Self {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            const { assert!(N >= 1 && N <= 16) };
            let (before, this) = (prev_chunk.value, self.value);
            // `vextq_u8(prev_chunk, this, 16 - N)`. Rust cannot pass `16 - N` as the intrinsic's constant,
            // so the match names each one; `N` is a constant, so it is resolved at compile time.
            match N {
                1 => vextq_u8::<15>(before, this),
                2 => vextq_u8::<14>(before, this),
                3 => vextq_u8::<13>(before, this),
                4 => vextq_u8::<12>(before, this),
                5 => vextq_u8::<11>(before, this),
                6 => vextq_u8::<10>(before, this),
                7 => vextq_u8::<9>(before, this),
                8 => vextq_u8::<8>(before, this),
                9 => vextq_u8::<7>(before, this),
                10 => vextq_u8::<6>(before, this),
                11 => vextq_u8::<5>(before, this),
                12 => vextq_u8::<4>(before, this),
                13 => vextq_u8::<3>(before, this),
                14 => vextq_u8::<2>(before, this),
                15 => vextq_u8::<1>(before, this),
                _ => vextq_u8::<0>(before, this),
            }
            .into()
        }
    }
}

// ---- simd8<bool> ----

/// The bit of each byte's position in its half, for `to_bitmask`.
const BIT_MASK: [u8; 16] = [0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80, 0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80];

impl Simd8<bool> {
    #[inline(always)]
    pub fn splat(value: bool) -> Self {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vmovq_n_u8(0u8.wrapping_sub(value as u8)).into()
        }
    }

    #[inline(always)]
    pub fn to_bitmask(self) -> u32 {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            let minput = self & Simd8::<u8>::load(&BIT_MASK).value.into();
            let tmp = vpaddq_u8(minput.value, minput.value);
            let tmp = vpaddq_u8(tmp, tmp);
            let tmp = vpaddq_u8(tmp, tmp);
            vgetq_lane_u16::<0>(vreinterpretq_u16_u8(tmp)) as u32
        }
    }

    /// Four bits per byte: `vshrn_n_u16(…, 4)`.
    #[inline(always)]
    pub fn to_bitmask64(self) -> u64 {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vget_lane_u64::<0>(vreinterpret_u64_u8(vshrn_n_u16::<4>(vreinterpretq_u16_u8(self.value))))
        }
    }

    #[inline(always)]
    pub fn any(self) -> bool {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vmaxvq_u32(vreinterpretq_u32_u8(self.value)) != 0
        }
    }
}

// ---- simd8<uint8_t> ----

/// A register that is a 16-entry table (`apply_lookup_16_to`): `simd8<uint8_t>` and
/// `simd8<int8_t>`.
pub trait Lookup16: Sized {
    fn apply_lookup_16_to<T: Lane>(self, original: Simd8<T>) -> Self;
}

impl Lookup16 for Simd8<u8> {
    #[inline(always)]
    fn apply_lookup_16_to<T: Lane>(self, original: Simd8<T>) -> Self {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe { vqtbl1q_u8(self.value, T::bytes(original.value)).into() }
    }
}

impl Simd8<u8> {
    #[inline(always)]
    pub fn splat(value: u8) -> Self {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vmovq_n_u8(value).into()
        }
    }

    #[inline(always)]
    pub fn zero() -> Self {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vdupq_n_u8(0).into()
        }
    }

    #[inline(always)]
    pub fn load(values: &[u8; 16]) -> Self {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vld1q_u8(values.as_ptr()).into()
        }
    }

    /// simdjson's 16-value constructor.
    #[inline(always)]
    pub fn new(values: [u8; 16]) -> Self {
        Self::load(&values)
    }

    #[inline(always)]
    pub fn repeat_16(values: [u8; 16]) -> Self {
        Self::new(values)
    }

    #[inline(always)]
    pub fn store(self, dst: &mut [u8; 16]) {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vst1q_u8(dst.as_mut_ptr(), self.value)
        }
    }

    #[inline(always)]
    pub fn saturating_add(self, other: Self) -> Self {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vqaddq_u8(self.value, other.value).into()
        }
    }

    #[inline(always)]
    pub fn saturating_sub(self, other: Self) -> Self {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vqsubq_u8(self.value, other.value).into()
        }
    }

    /// The largest byte (`vmaxvq_u8`).
    #[inline(always)]
    pub fn max_val(self) -> u8 {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vmaxvq_u8(self.value)
        }
    }

    #[inline(always)]
    pub fn min_val(self) -> u8 {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vminvq_u8(self.value)
        }
    }

    /// simdjson's `max_val(other)`: the larger byte of each pair.
    #[inline(always)]
    pub fn max_val_with(self, other: Self) -> Self {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vmaxq_u8(self.value, other.value).into()
        }
    }

    #[inline(always)]
    pub fn min_val_with(self, other: Self) -> Self {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vminq_u8(self.value, other.value).into()
        }
    }

    #[inline(always)]
    pub fn le(self, other: Self) -> Simd8<bool> {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vcleq_u8(self.value, other.value).into()
        }
    }

    #[inline(always)]
    pub fn ge(self, other: Self) -> Simd8<bool> {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vcgeq_u8(self.value, other.value).into()
        }
    }

    #[inline(always)]
    pub fn lt(self, other: Self) -> Simd8<bool> {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vcltq_u8(self.value, other.value).into()
        }
    }

    #[inline(always)]
    pub fn gt(self, other: Self) -> Simd8<bool> {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vcgtq_u8(self.value, other.value).into()
        }
    }

    #[inline(always)]
    pub fn gt_bits(self, other: Self) -> Self {
        self.gt(other).value.into()
    }

    #[inline(always)]
    pub fn lt_bits(self, other: Self) -> Self {
        self.lt(other).value.into()
    }

    #[inline(always)]
    pub fn any_bits_set(self, bits: Self) -> Simd8<bool> {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vtstq_u8(self.value, bits.value).into()
        }
    }

    #[inline(always)]
    pub fn any_bits_set_anywhere(self) -> bool {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vmaxvq_u32(vreinterpretq_u32_u8(self.value)) != 0
        }
    }

    /// simdjson's `any_bits_set_anywhere(bits)`.
    #[inline(always)]
    pub fn any_bits_set_anywhere_in(self, bits: Self) -> bool {
        (self & bits).any_bits_set_anywhere()
    }

    #[inline(always)]
    pub fn shr<const N: i32>(self) -> Self {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vshrq_n_u8::<N>(self.value).into()
        }
    }

    #[inline(always)]
    pub fn shl<const N: i32>(self) -> Self {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vshlq_n_u8::<N>(self.value).into()
        }
    }

    /// Each byte as an index into `lookup_table` (`vqtbl1q_u8`): 0 for indexes of 16 or more.
    #[inline(always)]
    pub fn lookup_16<L: Lane>(self, lookup_table: Simd8<L>) -> Simd8<L>
    where
        Simd8<L>: Lookup16,
    {
        lookup_table.apply_lookup_16_to(self)
    }

    /// simdjson's `lookup_16(replace0, …, replace15)`.
    #[inline(always)]
    pub fn lookup_16_values(self, replace: [u8; 16]) -> Simd8<u8> {
        self.lookup_16(Simd8::<u8>::repeat_16(replace))
    }

    #[inline(always)]
    pub fn to_bitmask64(self) -> u64 {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vget_lane_u64::<0>(vreinterpret_u64_u8(vshrn_n_u16::<4>(vreinterpretq_u16_u8(self.value))))
        }
    }

    /// The bytes whose bit in `mask` is clear, in order, at the start of `output`.
    #[inline(always)]
    pub fn compress(self, mask: u16, output: &mut [u8; 16]) {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            let mask1 = mask as u8;
            let mask2 = (mask >> 8) as u8;
            let shufmask64 = vcombine_u64(vcreate_u64(THINTABLE_EPI8[mask1 as usize]), vcreate_u64(THINTABLE_EPI8[mask2 as usize]));
            let inc = Simd8::<u8>::new([0, 0, 0, 0, 0, 0, 0, 0, 8, 8, 8, 8, 8, 8, 8, 8]).value;
            let shufmask = vaddq_u8(vreinterpretq_u8_u64(shufmask64), inc);
            let pruned = vqtbl1q_u8(self.value, shufmask);
            let pop1 = BITS_SET_TABLE_256_MUL2[mask1 as usize] as usize;
            // Rows start at `pop1 * 8 <= 128`; 16 bytes from there are within the 272.
            let compactmask = vld1q_u8(PSHUFB_COMBINE_TABLE.as_ptr().add(pop1 * 8));
            Simd8::<u8>::from(vqtbl1q_u8(pruned, compactmask)).store(output);
        }
    }

    /// The kept bytes of each half: 8 bytes written at `output1` and 8 at `output2`.
    ///
    /// # Safety
    /// Both pointers must have 8 writable bytes. As in simdjson they may overlap: the second
    /// write lands after the first.
    #[inline(always)]
    pub unsafe fn compress_halves(self, mask: u16, output1: *mut u8, output2: *mut u8) {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            let mask1 = mask as u8;
            let mask2 = (mask >> 8) as u8;
            let compactmask1 = vcreate_u8(THINTABLE_EPI8[mask1 as usize]);
            let compactmask2 = vadd_u8(vcreate_u8(THINTABLE_EPI8[mask2 as usize]), vdup_n_u8(8));
            // The caller gives 8 writable bytes at each pointer.
            vst1_u8(output1, vqtbl1_u8(self.value, compactmask1));
            vst1_u8(output2, vqtbl1_u8(self.value, compactmask2));
        }
    }
}

impl Add for Simd8<u8> {
    type Output = Self;
    #[inline(always)]
    fn add(self, other: Self) -> Self {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vaddq_u8(self.value, other.value).into()
        }
    }
}

impl Sub for Simd8<u8> {
    type Output = Self;
    #[inline(always)]
    fn sub(self, other: Self) -> Self {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vsubq_u8(self.value, other.value).into()
        }
    }
}

impl AddAssign for Simd8<u8> {
    #[inline(always)]
    fn add_assign(&mut self, other: Self) {
        *self = *self + other;
    }
}

impl SubAssign for Simd8<u8> {
    #[inline(always)]
    fn sub_assign(&mut self, other: Self) {
        *self = *self - other;
    }
}

// ---- simd8<int8_t> ----

impl Lookup16 for Simd8<i8> {
    #[inline(always)]
    fn apply_lookup_16_to<T: Lane>(self, original: Simd8<T>) -> Self {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vqtbl1q_s8(self.value, T::bytes(original.value)).into()
        }
    }
}

impl Simd8<i8> {
    #[inline(always)]
    pub fn splat(value: i8) -> Self {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vmovq_n_s8(value).into()
        }
    }

    #[inline(always)]
    pub fn zero() -> Self {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vdupq_n_s8(0).into()
        }
    }

    #[inline(always)]
    pub fn load(values: &[i8; 16]) -> Self {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vld1q_s8(values.as_ptr()).into()
        }
    }

    #[inline(always)]
    pub fn new(values: [i8; 16]) -> Self {
        Self::load(&values)
    }

    #[inline(always)]
    pub fn repeat_16(values: [i8; 16]) -> Self {
        Self::new(values)
    }

    #[inline(always)]
    pub fn store(self, dst: &mut [i8; 16]) {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vst1q_s8(dst.as_mut_ptr(), self.value)
        }
    }

    /// simdjson's `explicit simd8(const uint8x16_t)`.
    #[inline(always)]
    pub fn from_u8(other: Simd8<u8>) -> Self {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vreinterpretq_s8_u8(other.value).into()
        }
    }

    /// simdjson's `explicit operator simd8<uint8_t>()`.
    #[inline(always)]
    pub fn to_u8(self) -> Simd8<u8> {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vreinterpretq_u8_s8(self.value).into()
        }
    }

    #[inline(always)]
    pub fn max_val(self, other: Self) -> Self {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vmaxq_s8(self.value, other.value).into()
        }
    }

    #[inline(always)]
    pub fn min_val(self, other: Self) -> Self {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vminq_s8(self.value, other.value).into()
        }
    }

    #[inline(always)]
    pub fn gt(self, other: Self) -> Simd8<bool> {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vcgtq_s8(self.value, other.value).into()
        }
    }

    #[inline(always)]
    pub fn lt(self, other: Self) -> Simd8<bool> {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vcltq_s8(self.value, other.value).into()
        }
    }

    #[inline(always)]
    pub fn eq(self, other: Self) -> Simd8<bool> {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vceqq_s8(self.value, other.value).into()
        }
    }

    #[inline(always)]
    pub fn prev<const N: i32>(self, prev_chunk: Self) -> Self {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            const { assert!(N >= 1 && N <= 16) };
            let (before, this) = (prev_chunk.value, self.value);
            // `vextq_s8(prev_chunk, this, 16 - N)`. Rust cannot pass `16 - N` as the intrinsic's constant,
            // so the match names each one; `N` is a constant, so it is resolved at compile time.
            match N {
                1 => vextq_s8::<15>(before, this),
                2 => vextq_s8::<14>(before, this),
                3 => vextq_s8::<13>(before, this),
                4 => vextq_s8::<12>(before, this),
                5 => vextq_s8::<11>(before, this),
                6 => vextq_s8::<10>(before, this),
                7 => vextq_s8::<9>(before, this),
                8 => vextq_s8::<8>(before, this),
                9 => vextq_s8::<7>(before, this),
                10 => vextq_s8::<6>(before, this),
                11 => vextq_s8::<5>(before, this),
                12 => vextq_s8::<4>(before, this),
                13 => vextq_s8::<3>(before, this),
                14 => vextq_s8::<2>(before, this),
                15 => vextq_s8::<1>(before, this),
                _ => vextq_s8::<0>(before, this),
            }
            .into()
        }
    }

    #[inline(always)]
    pub fn lookup_16<L: Lane>(self, lookup_table: Simd8<L>) -> Simd8<L>
    where
        Simd8<L>: Lookup16,
    {
        lookup_table.apply_lookup_16_to(self)
    }

    #[inline(always)]
    pub fn lookup_16_values(self, replace: [i8; 16]) -> Simd8<i8> {
        self.lookup_16(Simd8::<i8>::repeat_16(replace))
    }
}

impl Add for Simd8<i8> {
    type Output = Self;
    #[inline(always)]
    fn add(self, other: Self) -> Self {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vaddq_s8(self.value, other.value).into()
        }
    }
}

impl Sub for Simd8<i8> {
    type Output = Self;
    #[inline(always)]
    fn sub(self, other: Self) -> Self {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            vsubq_s8(self.value, other.value).into()
        }
    }
}

impl AddAssign for Simd8<i8> {
    #[inline(always)]
    fn add_assign(&mut self, other: Self) {
        *self = *self + other;
    }
}

impl SubAssign for Simd8<i8> {
    #[inline(always)]
    fn sub_assign(&mut self, other: Self) {
        *self = *self - other;
    }
}

// ---- simd8x64<T> ----

/// simdjson's `simd8x64<T>`: one 64-byte block as `NUM_CHUNKS` registers. Not `Clone` or `Copy`:
/// simdjson deletes its copy constructor, so a block is read once and passed by reference.
pub struct Simd8x64<T: Lane> {
    pub chunks: [Simd8<T>; 4],
}

// simdjson: static_assert(NUM_CHUNKS == 4, "ARM kernel should use four registers per 64-byte block.")
const _: () = assert!(Simd8x64::<u8>::NUM_CHUNKS == 4, "ARM kernel should use four registers per 64-byte block.");
const _: () = assert!(Simd8x64::<bool>::NUM_CHUNKS == 4, "ARM kernel should use four registers per 64-byte block.");

impl<T: Lane> Simd8x64<T> {
    pub const NUM_CHUNKS: usize = 64 / size_of::<Simd8<T>>();

    #[inline(always)]
    pub fn new(chunk0: Simd8<T>, chunk1: Simd8<T>, chunk2: Simd8<T>, chunk3: Simd8<T>) -> Self {
        Simd8x64 { chunks: [chunk0, chunk1, chunk2, chunk3] }
    }

    /// Chunk `IDX`. simdjson returns zero past `NUM_CHUNKS`, which matters only on one-register
    /// targets; here `IDX` is checked at compile time.
    #[inline(always)]
    pub fn get<const IDX: usize>(&self) -> Simd8<T> {
        const { assert!(IDX < 4) };
        self.chunks[IDX]
    }
}

impl<T: BaseU8> Simd8x64<T> {
    #[inline(always)]
    pub fn reduce_or(&self) -> Simd8<T> {
        (self.chunks[0] | self.chunks[1]) | (self.chunks[2] | self.chunks[3])
    }

    /// Bit `i` is bit `i % 8` of byte `i`: an AND with the bit positions, then four `vpaddq_u8`.
    #[inline(always)]
    pub fn to_bitmask(&self) -> u64 {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            let bit_mask = Simd8::<u8>::load(&BIT_MASK).value;
            let sum0 = vpaddq_u8(vandq_u8(self.chunks[0].value, bit_mask), vandq_u8(self.chunks[1].value, bit_mask));
            let sum1 = vpaddq_u8(vandq_u8(self.chunks[2].value, bit_mask), vandq_u8(self.chunks[3].value, bit_mask));
            let sum0 = vpaddq_u8(sum0, sum1);
            let sum0 = vpaddq_u8(sum0, sum0);
            vgetq_lane_u64::<0>(vreinterpretq_u64_u8(sum0))
        }
    }
}

impl Simd8x64<u8> {
    #[inline(always)]
    pub fn load(ptr: &[u8; 64]) -> Self {
        // SAFETY: `[u8; 64]` and `[[u8; 16]; 4]` have the same size and alignment.
        let c = unsafe { &*(ptr as *const [u8; 64] as *const [[u8; 16]; 4]) };
        Simd8x64::new(Simd8::<u8>::load(&c[0]), Simd8::<u8>::load(&c[1]), Simd8::<u8>::load(&c[2]), Simd8::<u8>::load(&c[3]))
    }

    #[inline(always)]
    pub fn store(&self, ptr: &mut [u8; 64]) {
        // SAFETY: as in `load`.
        let c = unsafe { &mut *(ptr as *mut [u8; 64] as *mut [[u8; 16]; 4]) };
        self.chunks[0].store(&mut c[0]);
        self.chunks[1].store(&mut c[1]);
        self.chunks[2].store(&mut c[2]);
        self.chunks[3].store(&mut c[3]);
    }

    /// The bytes whose bit in `mask` is clear, in order; returns their count. Each half of each
    /// chunk is written by `compress_halves` at the running offset, as in simdjson.
    #[inline(always)]
    pub fn compress(&self, mask: u64, output: &mut [u8; 64]) -> u64 {
        // SAFETY: NEON is in the aarch64 baseline (module note).
        unsafe {
            let popcounts = vget_lane_u64::<0>(vreinterpret_u64_u8(vcnt_u8(vcreate_u8(!mask))));
            let offsets = popcounts.wrapping_mul(0x0101_0101_0101_0101);
            let out = output.as_mut_ptr();
            // Each offset is at most 56, so every 8-byte write ends within the 64 bytes.
            self.chunks[0].compress_halves(mask as u16, out, out.add((popcounts & 0xff) as usize));
            self.chunks[1].compress_halves((mask >> 16) as u16, out.add((offsets >> 8 & 0xff) as usize), out.add((offsets >> 16 & 0xff) as usize));
            self.chunks[2].compress_halves((mask >> 32) as u16, out.add((offsets >> 24 & 0xff) as usize), out.add((offsets >> 32 & 0xff) as usize));
            self.chunks[3].compress_halves((mask >> 48) as u16, out.add((offsets >> 40 & 0xff) as usize), out.add((offsets >> 48 & 0xff) as usize));
            offsets >> 56
        }
    }

    #[inline(always)]
    pub fn eq(&self, m: u8) -> u64 {
        let mask = Simd8::<u8>::splat(m);
        Simd8x64::<bool>::new(self.chunks[0].eq(mask), self.chunks[1].eq(mask), self.chunks[2].eq(mask), self.chunks[3].eq(mask)).to_bitmask()
    }

    #[inline(always)]
    pub fn lteq(&self, m: u8) -> u64 {
        let mask = Simd8::<u8>::splat(m);
        Simd8x64::<bool>::new(self.chunks[0].le(mask), self.chunks[1].le(mask), self.chunks[2].le(mask), self.chunks[3].le(mask)).to_bitmask()
    }
}
