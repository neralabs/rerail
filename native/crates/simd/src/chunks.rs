//! Each lane type's operations on a chunk of `L` lanes, as the target's vector instructions (`core::arch`): the
//! operators, comparisons, masks and casts of `Simd` are loops over chunks of these, so LLVM is given vector operations
//! and never has to find them (its SLP vectorizer leaves code with gathers or 64-bit products scalar). A lane type the
//! target has no vector code for here has `L == 1`: each lane's scalar operation.
#![allow(clippy::missing_safety_doc)]

/// Binary operations, by code.
pub const ADD: u8 = 0;
pub const SUB: u8 = 1;
pub const MUL: u8 = 2;
pub const AND: u8 = 3;
pub const OR: u8 = 4;
pub const XOR: u8 = 5;
/// Shifts: the amount modulo the lane's bits; right shifts arithmetic for signed lanes.
pub const SHL: u8 = 6;
pub const SHR: u8 = 7;
/// Comparisons: all ones where true.
pub const EQ: u8 = 8;
pub const LT: u8 = 9;
pub const LE: u8 = 10;
pub const MIN: u8 = 11;
pub const MAX: u8 = 12;
/// Unary operations.
pub const NOT: u8 = 0;
pub const CLZ: u8 = 1;

/// A lane type's chunk operations.
pub trait Chunks: Copy + PartialEq + 'static {
    /// Lanes a chunk.
    const L: usize;
    /// Whether the lane is signed (a widening `as` sign-extends it).
    const SIGNED: bool;
    /// `o[..L] = a[..L] op b[..L]`.
    unsafe fn bin<const OP: u8>(a: *const Self, b: *const Self, o: *mut Self);
    /// `o[..L] = op a[..L]`.
    unsafe fn un<const OP: u8>(a: *const Self, o: *mut Self);
    /// Whether any lane of the chunk is non-zero.
    unsafe fn any(a: *const Self) -> bool;
    /// Whether every lane of the chunk is non-zero.
    unsafe fn all(a: *const Self) -> bool;
    /// The chunk's lanes that are non-zero, as the low `L` bits (lane `i` bit `i`).
    #[inline(always)]
    unsafe fn bitmask(a: *const Self) -> u64 {
        let mut b = 0u64;
        for i in 0..Self::L {
            // SAFETY: `a` holds a chunk.
            let x = unsafe { a.add(i).read() };
            b |= ((x != Self::bin1::<XOR>(x, x)) as u64) << i;
        }
        b
    }
    /// One lane's operation.
    fn bin1<const OP: u8>(a: Self, b: Self) -> Self;
    fn un1<const OP: u8>(a: Self) -> Self;
}

/// A chunk's operation `OP` as each lane's: for an operation the target has no instruction for.
///
/// # Safety
/// `a`, `b` hold a chunk, `o` room for one.
#[inline(always)]
pub unsafe fn bin_lanes<T: Chunks, const OP: u8>(a: *const T, b: *const T, o: *mut T) {
    for i in 0..T::L {
        // SAFETY: lane `i` of each chunk.
        unsafe { o.add(i).write(T::bin1::<OP>(a.add(i).read(), b.add(i).read())) }
    }
}

/// `un` as each lane's.
///
/// # Safety
/// `a` holds a chunk, `o` room for one.
#[inline(always)]
pub unsafe fn un_lanes<T: Chunks, const OP: u8>(a: *const T, o: *mut T) {
    for i in 0..T::L {
        // SAFETY: lane `i` of each chunk.
        unsafe { o.add(i).write(T::un1::<OP>(a.add(i).read())) }
    }
}

macro_rules! scalar_ops {
    ($t:ty, $bits:literal) => {
        #[inline(always)]
        fn bin1<const OP: u8>(a: $t, b: $t) -> $t {
            let t = |c: bool| if c { !0 } else { 0 };
            match OP {
                ADD => a.wrapping_add(b),
                SUB => a.wrapping_sub(b),
                MUL => a.wrapping_mul(b),
                AND => a & b,
                OR => a | b,
                XOR => a ^ b,
                SHL => a.wrapping_shl(b as u32),
                SHR => a.wrapping_shr(b as u32),
                EQ => t(a == b),
                LT => t(a < b),
                LE => t(a <= b),
                MIN => a.min(b),
                _ => a.max(b),
            }
        }
        #[inline(always)]
        fn un1<const OP: u8>(a: $t) -> $t {
            match OP {
                NOT => !a,
                _ => a.leading_zeros() as $t,
            }
        }
    };
}

/// A lane type with no chunk code: each lane on its own.
macro_rules! scalar_chunks {
    ($($t:ty, $signed:literal, $bits:literal;)*) => {$(
        impl Chunks for $t {
            const L: usize = 1;
            const SIGNED: bool = $signed;
            #[inline(always)]
            unsafe fn bin<const OP: u8>(a: *const $t, b: *const $t, o: *mut $t) {
                unsafe { o.write(Self::bin1::<OP>(a.read(), b.read())) }
            }
            #[inline(always)]
            unsafe fn un<const OP: u8>(a: *const $t, o: *mut $t) {
                unsafe { o.write(Self::un1::<OP>(a.read())) }
            }
            #[inline(always)]
            unsafe fn any(a: *const $t) -> bool {
                unsafe { a.read() != 0 }
            }
            #[inline(always)]
            unsafe fn all(a: *const $t) -> bool {
                unsafe { a.read() != 0 }
            }
            scalar_ops!($t, $bits);
        }
    )*};
}

// The generic fallback: a target with no chunk code here (neither AArch64 nor x86-64) runs each lane on its own.
#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
scalar_chunks! {
    u8, false, 8; u16, false, 16; u32, false, 32; u64, false, 64; usize, false, 64;
    i8, true, 8; i16, true, 16; i32, true, 32; i64, true, 64; isize, true, 64;
}

#[cfg(target_arch = "aarch64")]
scalar_chunks! { u16, false, 16; i16, true, 16; }

#[cfg(target_arch = "x86_64")]
scalar_chunks! { u16, false, 16; i16, true, 16; }

#[cfg(target_arch = "x86_64")]
pub mod x86;

#[cfg(target_arch = "aarch64")]
mod neon {
    use super::*;
    use core::arch::aarch64::*;

    /// A NEON lane type: its register, loads and stores, and each operation as an expression of registers `a`, `b`.
    macro_rules! neon {
        ($t:ty, $signed:literal, $bits:literal, $l:literal, $v:ty, $ld:ident, $st:ident, $u:ty, $to_u:expr, $from_u:expr,
         add $add:expr, sub $sub:expr, mul $mul:expr, and $and:expr, or $or:expr, xor $xor:expr, not $not:expr,
         shl $shl:expr, shr $shr:expr, eq $eq:expr, lt $lt:expr, le $le:expr, min $min:expr, max $max:expr,
         clz $clz:expr, any $any:expr, all $all:expr, bitmask $bm:expr) => {
            impl Chunks for $t {
                const L: usize = $l;
                const SIGNED: bool = $signed;
                #[inline(always)]
                unsafe fn bin<const OP: u8>(a: *const $t, b: *const $t, o: *mut $t) {
                    unsafe {
                        let (a, b): ($v, $v) = ($ld(a.cast()), $ld(b.cast()));
                        let to_u: fn($u) -> $v = $from_u;
                        let r: $v = match OP {
                            ADD => $add(a, b),
                            SUB => $sub(a, b),
                            MUL => $mul(a, b),
                            AND => $and(a, b),
                            OR => $or(a, b),
                            XOR => $xor(a, b),
                            SHL => $shl(a, b),
                            SHR => $shr(a, b),
                            EQ => to_u($eq(a, b)),
                            LT => to_u($lt(a, b)),
                            LE => to_u($le(a, b)),
                            MIN => $min(a, b),
                            _ => $max(a, b),
                        };
                        $st(o.cast(), r)
                    }
                }
                #[inline(always)]
                unsafe fn un<const OP: u8>(a: *const $t, o: *mut $t) {
                    unsafe {
                        let a: $v = $ld(a.cast());
                        let r: $v = match OP {
                            NOT => $not(a),
                            _ => $clz(a),
                        };
                        $st(o.cast(), r)
                    }
                }
                #[inline(always)]
                unsafe fn any(a: *const $t) -> bool {
                    unsafe {
                        let a: $v = $ld(a.cast());
                        let to: fn($v) -> $u = $to_u;
                        $any(to(a))
                    }
                }
                #[inline(always)]
                unsafe fn all(a: *const $t) -> bool {
                    unsafe {
                        let a: $v = $ld(a.cast());
                        let to: fn($v) -> $u = $to_u;
                        $all(to(a))
                    }
                }
                #[inline(always)]
                unsafe fn bitmask(a: *const $t) -> u64 {
                    unsafe {
                        let a: $v = $ld(a.cast());
                        let to: fn($v) -> $u = $to_u;
                        let bm: fn($u) -> u64 = $bm;
                        bm(to(a))
                    }
                }
                scalar_ops!($t, $bits);
            }
        };
    }

    #[inline(always)]
    unsafe fn sh32(b: uint32x4_t) -> int32x4_t {
        unsafe { vreinterpretq_s32_u32(vandq_u32(b, vdupq_n_u32(31))) }
    }
    #[inline(always)]
    unsafe fn sh64(b: uint64x2_t) -> int64x2_t {
        unsafe { vreinterpretq_s64_u64(vandq_u64(b, vdupq_n_u64(63))) }
    }
    #[inline(always)]
    unsafe fn sh8(b: uint8x16_t) -> int8x16_t {
        unsafe { vreinterpretq_s8_u8(vandq_u8(b, vdupq_n_u8(7))) }
    }
    /// A 64-bit product from 32-bit ones: the low halves' whole product, the cross products a word up.
    #[inline(always)]
    unsafe fn mul64(a: uint64x2_t, b: uint64x2_t) -> uint64x2_t {
        unsafe {
            let (al, bl) = (vmovn_u64(a), vmovn_u64(b));
            let (ah, bh) = (vshrn_n_u64::<32>(a), vshrn_n_u64::<32>(b));
            let cross = vmla_u32(vmul_u32(al, bh), ah, bl);
            vaddq_u64(vmull_u32(al, bl), vshll_n_u32::<32>(cross))
        }
    }
    #[inline(always)]
    unsafe fn clz64(a: uint64x2_t) -> uint64x2_t {
        unsafe { vcombine_u64(vdup_n_u64(vgetq_lane_u64::<0>(a).leading_zeros() as u64), vdup_n_u64(vgetq_lane_u64::<1>(a).leading_zeros() as u64)) }
    }
    /// Each non-zero lane's bit: the lanes tested, each its place's bit, summed across.
    #[inline(always)]
    fn bits8(a: uint8x16_t) -> u64 {
        const PLACE: [u8; 16] = [1, 2, 4, 8, 16, 32, 64, 128, 1, 2, 4, 8, 16, 32, 64, 128];
        unsafe {
            let m = vandq_u8(vtstq_u8(a, a), vld1q_u8(PLACE.as_ptr()));
            vaddv_u8(vget_low_u8(m)) as u64 | (vaddv_u8(vget_high_u8(m)) as u64) << 8
        }
    }
    #[inline(always)]
    fn bits32(a: uint32x4_t) -> u64 {
        const PLACE: [u32; 4] = [1, 2, 4, 8];
        unsafe { vaddvq_u32(vandq_u32(vtstq_u32(a, a), vld1q_u32(PLACE.as_ptr()))) as u64 }
    }
    #[inline(always)]
    fn bits64(a: uint64x2_t) -> u64 {
        const PLACE: [u64; 2] = [1, 2];
        unsafe { vaddvq_u64(vandq_u64(vtstq_u64(a, a), vld1q_u64(PLACE.as_ptr()))) }
    }
    #[inline(always)]
    unsafe fn any64(a: uint64x2_t) -> bool {
        unsafe { vmaxvq_u32(vreinterpretq_u32_u64(a)) != 0 }
    }
    #[inline(always)]
    unsafe fn all64(a: uint64x2_t) -> bool {
        unsafe { vminvq_u32(vreinterpretq_u32_u64(vceqzq_u64(a))) == 0 }
    }

    neon!(u32, false, 32, 4, uint32x4_t, vld1q_u32, vst1q_u32, uint32x4_t, |x| x, |x| x,
        add vaddq_u32, sub vsubq_u32, mul vmulq_u32, and vandq_u32, or vorrq_u32, xor veorq_u32, not vmvnq_u32,
        shl |a, b| vshlq_u32(a, sh32(b)), shr |a, b| vshlq_u32(a, vnegq_s32(sh32(b))),
        eq vceqq_u32, lt vcltq_u32, le vcleq_u32, min vminq_u32, max vmaxq_u32,
        clz vclzq_u32, any |a| vmaxvq_u32(a) != 0, all |a| vminvq_u32(a) != 0, bitmask bits32);
    neon!(i32, true, 32, 4, int32x4_t, vld1q_s32, vst1q_s32, uint32x4_t, |x| vreinterpretq_u32_s32(x), |x| vreinterpretq_s32_u32(x),
        add vaddq_s32, sub vsubq_s32, mul vmulq_s32, and vandq_s32, or vorrq_s32, xor veorq_s32, not vmvnq_s32,
        shl |a, b| vshlq_s32(a, sh32(vreinterpretq_u32_s32(b))), shr |a, b| vshlq_s32(a, vnegq_s32(sh32(vreinterpretq_u32_s32(b)))),
        eq vceqq_s32, lt vcltq_s32, le vcleq_s32, min vminq_s32, max vmaxq_s32,
        clz vclzq_s32, any |a| vmaxvq_u32(a) != 0, all |a| vminvq_u32(a) != 0, bitmask bits32);
    neon!(u8, false, 8, 16, uint8x16_t, vld1q_u8, vst1q_u8, uint8x16_t, |x| x, |x| x,
        add vaddq_u8, sub vsubq_u8, mul vmulq_u8, and vandq_u8, or vorrq_u8, xor veorq_u8, not vmvnq_u8,
        shl |a, b| vshlq_u8(a, sh8(b)), shr |a, b| vshlq_u8(a, vnegq_s8(sh8(b))),
        eq vceqq_u8, lt vcltq_u8, le vcleq_u8, min vminq_u8, max vmaxq_u8,
        clz vclzq_u8, any |a| vmaxvq_u8(a) != 0, all |a| vminvq_u8(a) != 0, bitmask bits8);
    #[inline(always)]
    unsafe fn sh8s(b: int8x16_t) -> int8x16_t {
        unsafe { vandq_s8(b, vdupq_n_s8(7)) }
    }
    neon!(i8, true, 8, 16, int8x16_t, vld1q_s8, vst1q_s8, uint8x16_t, |x| vreinterpretq_u8_s8(x), |x| vreinterpretq_s8_u8(x),
        add vaddq_s8, sub vsubq_s8, mul vmulq_s8, and vandq_s8, or vorrq_s8, xor veorq_s8, not vmvnq_s8,
        shl |a, b| vshlq_s8(a, sh8s(b)), shr |a, b| vshlq_s8(a, vnegq_s8(sh8s(b))),
        eq vceqq_s8, lt vcltq_s8, le vcleq_s8, min vminq_s8, max vmaxq_s8,
        clz vclzq_s8, any |a| vmaxvq_u8(a) != 0, all |a| vminvq_u8(a) != 0, bitmask bits8);
    macro_rules! neon64 {
        ($t:ty, $signed:literal) => {
            neon!($t, $signed, 64, 2, uint64x2_t, vld1q_u64, vst1q_u64, uint64x2_t, |x| x, |x| x,
                add vaddq_u64, sub vsubq_u64, mul mul64, and vandq_u64, or vorrq_u64, xor veorq_u64,
                not |a| veorq_u64(a, vdupq_n_u64(!0)),
                shl |a, b| vshlq_u64(a, sh64(b)),
                shr |a, b| if $signed {
                    vreinterpretq_u64_s64(vshlq_s64(vreinterpretq_s64_u64(a), vnegq_s64(sh64(b))))
                } else {
                    vshlq_u64(a, vnegq_s64(sh64(b)))
                },
                eq vceqq_u64,
                lt |a, b| if $signed { vcltq_s64(vreinterpretq_s64_u64(a), vreinterpretq_s64_u64(b)) } else { vcltq_u64(a, b) },
                le |a, b| if $signed { vcleq_s64(vreinterpretq_s64_u64(a), vreinterpretq_s64_u64(b)) } else { vcleq_u64(a, b) },
                min |a, b| {
                    let lt = if $signed { vcltq_s64(vreinterpretq_s64_u64(a), vreinterpretq_s64_u64(b)) } else { vcltq_u64(a, b) };
                    vbslq_u64(lt, a, b)
                },
                max |a, b| {
                    let lt = if $signed { vcltq_s64(vreinterpretq_s64_u64(a), vreinterpretq_s64_u64(b)) } else { vcltq_u64(a, b) };
                    vbslq_u64(lt, b, a)
                },
                clz clz64, any any64, all all64, bitmask bits64);
        };
    }
    neon64!(u64, false);
    neon64!(i64, true);
    neon64!(usize, false);
    neon64!(isize, true);
}

/// `o[..N] = a[..N] as U` for lanes 4 bytes wide to 8 (`T::SIGNED`: sign-extended), 8 to 4 (truncated), or of one
/// width (the bits); `false` where there is no chunk code for it (each lane converted on its own).
///
/// # Safety
/// `a` holds `N` lanes of `T`, `o` room for `N` of `U`.
#[inline(always)]
pub unsafe fn cast<T: Chunks, U: Chunks, const N: usize>(a: *const T, o: *mut U) -> bool {
    let (st, su) = (size_of::<T>(), size_of::<U>());
    if st == su {
        unsafe { core::ptr::copy_nonoverlapping(a.cast::<u8>(), o.cast::<u8>(), N * st) };
        return true;
    }
    #[cfg(target_arch = "x86_64")]
    if N.is_multiple_of(4) {
        return unsafe { x86::cast::<T, U, N>(a, o) };
    }
    #[cfg(target_arch = "aarch64")]
    if N.is_multiple_of(4) {
        use core::arch::aarch64::*;
        unsafe {
            if st == 4 && su == 8 {
                for i in (0..N).step_by(4) {
                    let o = o.add(i).cast::<u64>();
                    if T::SIGNED {
                        let v = vld1q_s32(a.add(i).cast());
                        vst1q_s64(o.cast(), vmovl_s32(vget_low_s32(v)));
                        vst1q_s64(o.add(2).cast(), vmovl_high_s32(v));
                    } else {
                        let v = vld1q_u32(a.add(i).cast());
                        vst1q_u64(o, vmovl_u32(vget_low_u32(v)));
                        vst1q_u64(o.add(2), vmovl_high_u32(v));
                    }
                }
                return true;
            }
            if st == 8 && su == 4 {
                for i in (0..N).step_by(4) {
                    let p = a.add(i).cast::<u64>();
                    vst1q_u32(o.add(i).cast(), vmovn_high_u64(vmovn_u64(vld1q_u64(p)), vld1q_u64(p.add(2))));
                }
                return true;
            }
        }
    }
    false
}
