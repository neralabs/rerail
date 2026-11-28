//! The x86-64 chunks: a lane type's chunk is one 128-bit register (SSE2, every x86-64), its operations SSE2's, or a
//! later extension's where the build enables it (`target_feature`: SSE4.1's 32-bit products, minimums and 64-bit
//! equality, SSE4.2's 64-bit compares, AVX2's shifts by each lane's own amount). An operation with no instruction in
//! the build's set is each lane's (`bin_lanes`), as the generic fallback's.
use super::*;
use core::arch::x86_64::*;

#[inline(always)]
unsafe fn ld<T>(p: *const T) -> __m128i {
    unsafe { _mm_loadu_si128(p.cast()) }
}
#[inline(always)]
unsafe fn st<T>(p: *mut T, v: __m128i) {
    unsafe { _mm_storeu_si128(p.cast(), v) }
}
#[inline(always)]
fn ones() -> __m128i {
    unsafe { _mm_set1_epi32(-1) }
}
#[inline(always)]
fn zero() -> __m128i {
    unsafe { _mm_setzero_si128() }
}
#[inline(always)]
fn not(a: __m128i) -> __m128i {
    unsafe { _mm_xor_si128(a, ones()) }
}
/// `m`'s lanes of `x`, the rest of `y`.
#[inline(always)]
fn sel(m: __m128i, x: __m128i, y: __m128i) -> __m128i {
    unsafe { _mm_or_si128(_mm_and_si128(m, x), _mm_andnot_si128(m, y)) }
}
#[allow(dead_code)]
/// Whether every 32-bit lane of `b` is its first: a shift by one amount, which SSE2 has.
#[inline(always)]
fn uniform32(b: __m128i) -> bool {
    unsafe { _mm_movemask_epi8(_mm_cmpeq_epi32(b, _mm_shuffle_epi32::<0>(b))) == 0xffff }
}
#[allow(dead_code)]
#[inline(always)]
fn uniform64(b: __m128i) -> bool {
    unsafe { _mm_movemask_epi8(_mm_cmpeq_epi32(b, _mm_shuffle_epi32::<0b01_00_01_00>(b))) == 0xffff }
}
/// The low lane of `b` as a shift count (the register's low 64 bits).
#[allow(dead_code)]
#[inline(always)]
fn count(b: __m128i) -> __m128i {
    unsafe { _mm_move_epi64(b) }
}

// ---- 8-bit lanes. ----

mod w8 {
    use super::*;
    #[inline(always)]
    pub fn mul(a: __m128i, b: __m128i) -> __m128i {
        unsafe {
            let even = _mm_mullo_epi16(a, b);
            let odd = _mm_mullo_epi16(_mm_srli_epi16::<8>(a), _mm_srli_epi16::<8>(b));
            _mm_or_si128(_mm_and_si128(even, _mm_set1_epi16(0xff)), _mm_slli_epi16::<8>(odd))
        }
    }
    #[inline(always)]
    pub fn lt(a: __m128i, b: __m128i, signed: bool) -> __m128i {
        unsafe {
            if signed {
                _mm_cmplt_epi8(a, b)
            } else {
                let f = _mm_set1_epi8(i8::MIN);
                _mm_cmplt_epi8(_mm_xor_si128(a, f), _mm_xor_si128(b, f))
            }
        }
    }
    #[inline(always)]
    pub fn min(a: __m128i, b: __m128i, signed: bool) -> __m128i {
        if signed { sel(lt(a, b, true), a, b) } else { unsafe { _mm_min_epu8(a, b) } }
    }
    #[inline(always)]
    pub fn max(a: __m128i, b: __m128i, signed: bool) -> __m128i {
        if signed { sel(lt(a, b, true), b, a) } else { unsafe { _mm_max_epu8(a, b) } }
    }
    #[inline(always)]
    pub fn zeros(a: __m128i) -> u32 {
        unsafe { _mm_movemask_epi8(_mm_cmpeq_epi8(a, zero())) as u32 }
    }
}

// ---- 32-bit lanes. ----

mod w32 {
    use super::*;
    #[inline(always)]
    pub fn mul(a: __m128i, b: __m128i) -> __m128i {
        #[cfg(target_feature = "sse4.1")]
        unsafe {
            _mm_mullo_epi32(a, b)
        }
        #[cfg(not(target_feature = "sse4.1"))]
        unsafe {
            let p02 = _mm_mul_epu32(a, b);
            let p13 = _mm_mul_epu32(_mm_srli_epi64::<32>(a), _mm_srli_epi64::<32>(b));
            _mm_unpacklo_epi32(_mm_shuffle_epi32::<0b00_00_10_00>(p02), _mm_shuffle_epi32::<0b00_00_10_00>(p13))
        }
    }
    /// `a` shifted by each lane of `b` (modulo 32); none where the build has no instruction for it.
    #[inline(always)]
    pub fn shift(a: __m128i, b: __m128i, left: bool, signed: bool) -> Option<__m128i> {
        unsafe {
            let b = _mm_and_si128(b, _mm_set1_epi32(31));
            #[cfg(target_feature = "avx2")]
            {
                Some(if left {
                    _mm_sllv_epi32(a, b)
                } else if signed {
                    _mm_srav_epi32(a, b)
                } else {
                    _mm_srlv_epi32(a, b)
                })
            }
            #[cfg(not(target_feature = "avx2"))]
            {
                if !uniform32(b) {
                    return None;
                }
                let c = _mm_and_si128(b, _mm_set_epi32(0, 0, 0, -1));
                Some(if left {
                    _mm_sll_epi32(a, c)
                } else if signed {
                    _mm_sra_epi32(a, c)
                } else {
                    _mm_srl_epi32(a, c)
                })
            }
        }
    }
    #[inline(always)]
    pub fn lt(a: __m128i, b: __m128i, signed: bool) -> __m128i {
        unsafe {
            if signed {
                _mm_cmplt_epi32(a, b)
            } else {
                let f = _mm_set1_epi32(i32::MIN);
                _mm_cmplt_epi32(_mm_xor_si128(a, f), _mm_xor_si128(b, f))
            }
        }
    }
    #[inline(always)]
    pub fn min(a: __m128i, b: __m128i, signed: bool) -> __m128i {
        #[cfg(target_feature = "sse4.1")]
        unsafe {
            if signed { _mm_min_epi32(a, b) } else { _mm_min_epu32(a, b) }
        }
        #[cfg(not(target_feature = "sse4.1"))]
        sel(lt(a, b, signed), a, b)
    }
    #[inline(always)]
    pub fn max(a: __m128i, b: __m128i, signed: bool) -> __m128i {
        #[cfg(target_feature = "sse4.1")]
        unsafe {
            if signed { _mm_max_epi32(a, b) } else { _mm_max_epu32(a, b) }
        }
        #[cfg(not(target_feature = "sse4.1"))]
        sel(lt(a, b, signed), b, a)
    }
    /// The zero lanes, a bit each.
    #[inline(always)]
    pub fn zeros(a: __m128i) -> u32 {
        unsafe { _mm_movemask_ps(_mm_castsi128_ps(_mm_cmpeq_epi32(a, zero()))) as u32 }
    }
}

// ---- 64-bit lanes. ----

mod w64 {
    use super::*;
    /// A 64-bit product from 32-bit ones: the low halves' whole product, the cross products a word up.
    #[inline(always)]
    pub fn mul(a: __m128i, b: __m128i) -> __m128i {
        unsafe {
            let lo = _mm_mul_epu32(a, b);
            let cross = _mm_add_epi64(_mm_mul_epu32(_mm_srli_epi64::<32>(a), b), _mm_mul_epu32(a, _mm_srli_epi64::<32>(b)));
            _mm_add_epi64(lo, _mm_slli_epi64::<32>(cross))
        }
    }
    /// Each lane all ones where negative: its high word's sign, spread.
    #[inline(always)]
    fn sign(a: __m128i) -> __m128i {
        unsafe { _mm_shuffle_epi32::<0b11_11_01_01>(_mm_srai_epi32::<31>(a)) }
    }
    /// `a` shifted by each lane of `b` (modulo 64): an arithmetic shift is the logical one of `a ^ s`, `^ s` again
    /// (`s` the sign); none where the build has no instruction for it.
    #[inline(always)]
    pub fn shift(a: __m128i, b: __m128i, left: bool, signed: bool) -> Option<__m128i> {
        unsafe {
            let b = _mm_and_si128(b, _mm_set1_epi64x(63));
            let s = if signed && !left { sign(a) } else { zero() };
            let x = _mm_xor_si128(a, s);
            #[cfg(target_feature = "avx2")]
            let r = if left { _mm_sllv_epi64(x, b) } else { _mm_srlv_epi64(x, b) };
            #[cfg(not(target_feature = "avx2"))]
            let r = {
                if !uniform64(b) {
                    return None;
                }
                let c = count(b);
                if left { _mm_sll_epi64(x, c) } else { _mm_srl_epi64(x, c) }
            };
            Some(_mm_xor_si128(r, s))
        }
    }
    #[inline(always)]
    pub fn eq(a: __m128i, b: __m128i) -> __m128i {
        #[cfg(target_feature = "sse4.1")]
        unsafe {
            _mm_cmpeq_epi64(a, b)
        }
        #[cfg(not(target_feature = "sse4.1"))]
        unsafe {
            let c = _mm_cmpeq_epi32(a, b);
            _mm_and_si128(c, _mm_shuffle_epi32::<0b10_11_00_01>(c))
        }
    }
    /// `a < b`; none without SSE4.2's compare.
    #[inline(always)]
    #[allow(unused_variables)]
    pub fn lt(a: __m128i, b: __m128i, signed: bool) -> Option<__m128i> {
        #[cfg(target_feature = "sse4.2")]
        unsafe {
            Some(if signed {
                _mm_cmpgt_epi64(b, a)
            } else {
                let f = _mm_set1_epi64x(i64::MIN);
                _mm_cmpgt_epi64(_mm_xor_si128(b, f), _mm_xor_si128(a, f))
            })
        }
        #[cfg(not(target_feature = "sse4.2"))]
        None
    }
    /// The zero lanes, a bit each.
    #[inline(always)]
    pub fn zeros(a: __m128i) -> u32 {
        unsafe { _mm_movemask_pd(_mm_castsi128_pd(eq(a, zero()))) as u32 }
    }
}

/// A lane type's chunk: its width's module `$w`, `$l` lanes.
macro_rules! x86 {
    (@shift w8, $a:expr, $b:expr, $left:expr, $signed:expr) => { None::<__m128i> };
    (@shift $w:ident, $a:expr, $b:expr, $left:expr, $signed:expr) => { $w::shift($a, $b, $left, $signed) };
    (@lt w64, $a:expr, $b:expr, $signed:expr) => { w64::lt($a, $b, $signed) };
    (@lt $w:ident, $a:expr, $b:expr, $signed:expr) => { Some($w::lt($a, $b, $signed)) };
    (@min w64, $a:expr, $b:expr, $signed:expr) => { w64::lt($a, $b, $signed).map(|m| sel(m, $a, $b)) };
    (@min $w:ident, $a:expr, $b:expr, $signed:expr) => { Some($w::min($a, $b, $signed)) };
    (@max w64, $a:expr, $b:expr, $signed:expr) => { w64::lt($a, $b, $signed).map(|m| sel(m, $b, $a)) };
    (@max $w:ident, $a:expr, $b:expr, $signed:expr) => { Some($w::max($a, $b, $signed)) };
    ($t:ty, $signed:literal, $bits:literal, $l:literal, $w:ident, add $add:ident, sub $sub:ident, eq $eq:expr) => {
        impl Chunks for $t {
            const L: usize = $l;
            const SIGNED: bool = $signed;
            #[inline(always)]
            unsafe fn bin<const OP: u8>(pa: *const $t, pb: *const $t, o: *mut $t) {
                unsafe {
                    let (a, b) = (ld(pa), ld(pb));
                    let eq: fn(__m128i, __m128i) -> __m128i = $eq;
                    let r = match OP {
                        ADD => $add(a, b),
                        SUB => $sub(a, b),
                        MUL => $w::mul(a, b),
                        AND => _mm_and_si128(a, b),
                        OR => _mm_or_si128(a, b),
                        XOR => _mm_xor_si128(a, b),
                        SHL | SHR => match x86!(@shift $w, a, b, OP == SHL, $signed) {
                            Some(r) => r,
                            None => return bin_lanes::<$t, OP>(pa, pb, o),
                        },
                        EQ => eq(a, b),
                        LT | LE | MIN | MAX => {
                            let lt = |x, y| x86!(@lt $w, x, y, $signed);
                            let r = match OP {
                                LT => lt(a, b),
                                LE => lt(b, a).map(not),
                                MIN => x86!(@min $w, a, b, $signed),
                                _ => x86!(@max $w, a, b, $signed),
                            };
                            match r {
                                Some(r) => r,
                                None => return bin_lanes::<$t, OP>(pa, pb, o),
                            }
                        }
                        _ => return bin_lanes::<$t, OP>(pa, pb, o),
                    };
                    st(o, r)
                }
            }
            #[inline(always)]
            unsafe fn un<const OP: u8>(a: *const $t, o: *mut $t) {
                unsafe {
                    match OP {
                        NOT => st(o, not(ld(a))),
                        _ => un_lanes::<$t, OP>(a, o),
                    }
                }
            }
            #[inline(always)]
            unsafe fn any(a: *const $t) -> bool {
                unsafe { $w::zeros(ld(a)) != (1 << $l) - 1 }
            }
            #[inline(always)]
            unsafe fn all(a: *const $t) -> bool {
                unsafe { $w::zeros(ld(a)) == 0 }
            }
            #[inline(always)]
            unsafe fn bitmask(a: *const $t) -> u64 {
                unsafe { (!$w::zeros(ld(a)) & ((1u32 << $l) - 1)) as u64 }
            }
            scalar_ops!($t, $bits);
        }
    };
}

x86!(u8, false, 8, 16, w8, add _mm_add_epi8, sub _mm_sub_epi8, eq |a, b| _mm_cmpeq_epi8(a, b));
x86!(i8, true, 8, 16, w8, add _mm_add_epi8, sub _mm_sub_epi8, eq |a, b| _mm_cmpeq_epi8(a, b));
x86!(u32, false, 32, 4, w32, add _mm_add_epi32, sub _mm_sub_epi32, eq |a, b| _mm_cmpeq_epi32(a, b));
x86!(i32, true, 32, 4, w32, add _mm_add_epi32, sub _mm_sub_epi32, eq |a, b| _mm_cmpeq_epi32(a, b));
x86!(u64, false, 64, 2, w64, add _mm_add_epi64, sub _mm_sub_epi64, eq w64::eq);
x86!(i64, true, 64, 2, w64, add _mm_add_epi64, sub _mm_sub_epi64, eq w64::eq);
x86!(usize, false, 64, 2, w64, add _mm_add_epi64, sub _mm_sub_epi64, eq w64::eq);
x86!(isize, true, 64, 2, w64, add _mm_add_epi64, sub _mm_sub_epi64, eq w64::eq);

/// `chunks::cast`'s x86-64 code: 4 lanes a step, 4 bytes wide to 8 (unpacked with their sign or zero) or 8 to 4 (each
/// lane's low word).
///
/// # Safety
/// As `chunks::cast`, `N` a multiple of 4.
#[inline(always)]
pub unsafe fn cast<T: Chunks, U: Chunks, const N: usize>(a: *const T, o: *mut U) -> bool {
    let (st_, su) = (size_of::<T>(), size_of::<U>());
    unsafe {
        if st_ == 4 && su == 8 {
            for i in (0..N).step_by(4) {
                let v = ld(a.add(i));
                let s = if T::SIGNED { _mm_srai_epi32::<31>(v) } else { zero() };
                let o = o.add(i).cast::<u64>();
                st(o, _mm_unpacklo_epi32(v, s));
                st(o.add(2), _mm_unpackhi_epi32(v, s));
            }
            return true;
        }
        if st_ == 8 && su == 4 {
            for i in (0..N).step_by(4) {
                let p = a.add(i).cast::<u64>();
                let (x, y) = (_mm_castsi128_ps(ld(p)), _mm_castsi128_ps(ld(p.add(2))));
                st(o.add(i), _mm_castps_si128(_mm_shuffle_ps::<0b10_00_10_00>(x, y)));
            }
            return true;
        }
    }
    false
}

/// `swizzle_dyn` of `N` bytes (16 to 64, a multiple of 16) with SSSE3: each 16 indices looked up in each 16 bytes of
/// the table (`pshufb`), an index outside those 16 saturated past 0x7f so that lookup gives 0.
///
/// # Safety
/// `t`, `ix` and `o` hold `N` bytes.
#[cfg(target_feature = "ssse3")]
#[inline(always)]
pub unsafe fn swizzle_dyn<const N: usize>(t: *const u8, ix: *const u8, o: *mut u8) {
    unsafe {
        for j in 0..N / 16 {
            let i = ld(ix.add(16 * j));
            let mut r = zero();
            for k in 0..N / 16 {
                // An index below 16 * k wraps to 0xd0 or more, one past these 16 is 16 or more: either, with 0x70
                // added (saturated), has its top bit set, which `pshufb` reads as 0.
                let at = _mm_adds_epu8(_mm_sub_epi8(i, _mm_set1_epi8((16 * k) as i8)), _mm_set1_epi8(0x70));
                r = _mm_or_si128(r, _mm_shuffle_epi8(ld(t.add(16 * k)), at));
            }
            st(o.add(16 * j), r);
        }
    }
}
