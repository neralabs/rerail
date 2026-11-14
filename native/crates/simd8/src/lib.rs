//! simdjson's `simd8x64` operations in Rust: 64 bytes in registers, compared, looked up by nibble
//! and packed into a 64-bit mask, one bit a byte. simdjson writes its stage 1 once over these
//! operations and compiles it for each implementation; so does a kernel here (`dispatch!`).
//!
//! The backends are simdjson's implementations:
//!
//! - `Arm64`: NEON, four 16-byte registers, packed into a mask by pairwise adds (every aarch64 CPU);
//! - `Haswell`: AVX2, two 32-byte registers (x86_64 with AVX2, BMI1/2, LZCNT, PCLMULQDQ);
//! - `Westmere`: SSE, four 16-byte registers (x86_64 with SSE4.2, PCLMULQDQ);
//! - `Fallback`: the 64 bytes as an array, byte by byte (any CPU; simdjson's fallback is scalar too).
//!
//! The best one the CPU runs is chosen at run time on x86_64 (`best`), as simdjson chooses; on
//! aarch64 NEON is always there. A kernel is a generic function over `Simd8x64`, marked
//! `#[inline(always)]`: `dispatch!` compiles it inside a function that enables the backend's CPU
//! features, so every operation inlines into one loop.
//!
//! The operations' results are vectors of `0xff` (true) and `0` (false) bytes, as simdjson's
//! `simd8<bool>`; `to_bitmask` packs one into the mask.

/// A backend: the 64-byte vector type and its operations.
///
/// # Safety
///
/// Each operation uses the backend's CPU instructions: call them only where the CPU runs the
/// backend (`Backend::runs`), as `dispatch!` does.
pub unsafe trait Simd8x64 {
    /// 64 bytes.
    type V: Copy;
    /// Which backend this is.
    const BACKEND: Backend;

    /// 64 bytes from `b`.
    unsafe fn load(b: &[u8; 64]) -> Self::V;
    /// Each byte is `x`.
    unsafe fn splat(x: u8) -> Self::V;
    /// Byte by byte, `a == b`: `0xff` or `0`.
    unsafe fn eq(a: Self::V, b: Self::V) -> Self::V;
    /// Byte by byte, `a & b`.
    unsafe fn and(a: Self::V, b: Self::V) -> Self::V;
    /// Byte by byte, `a | b`.
    unsafe fn or(a: Self::V, b: Self::V) -> Self::V;
    /// Each byte's high nibble (`b >> 4`).
    unsafe fn high_nibbles(a: Self::V) -> Self::V;
    /// Each byte's low nibble (`b & 15`).
    unsafe fn low_nibbles(a: Self::V) -> Self::V;
    /// Byte by byte, `table[i]` for each nibble `i` of `idx` (`vqtbl1q_u8`, `pshufb`).
    unsafe fn lookup_16(idx: Self::V, table: &[u8; 16]) -> Self::V;
    /// Bit `i`: byte `i` is not 0.
    unsafe fn nonzero(a: Self::V) -> u64;
    /// Bit `i`: byte `i`'s high bit (of a compare's result: it is true).
    unsafe fn to_bitmask(a: Self::V) -> u64;

    /// Bit `i`: byte `i` is `x`.
    #[inline(always)]
    unsafe fn eq_mask(a: Self::V, x: u8) -> u64 {
        unsafe { Self::to_bitmask(Self::eq(a, Self::splat(x))) }
    }

    /// Each byte's classes by simdjson's nibble tables: `lo[b & 15] & hi[b >> 4]`. A class is a bit
    /// set in both tables: the low nibbles and the high nibbles it holds.
    #[inline(always)]
    unsafe fn classify(a: Self::V, lo: &[u8; 16], hi: &[u8; 16]) -> Self::V {
        unsafe { Self::and(Self::lookup_16(Self::low_nibbles(a), lo), Self::lookup_16(Self::high_nibbles(a), hi)) }
    }
}

/// simdjson's implementations, as `best` names the one the CPU runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Backend {
    Arm64 = 1,
    Haswell = 2,
    Westmere = 3,
    Fallback = 4,
}

impl Backend {
    /// Every backend, best first.
    pub const ALL: [Backend; 4] = [Backend::Arm64, Backend::Haswell, Backend::Westmere, Backend::Fallback];

    /// Whether this CPU runs the backend.
    pub fn runs(self) -> bool {
        match self {
            Backend::Arm64 => cfg!(target_arch = "aarch64"),
            #[cfg(target_arch = "x86_64")]
            Backend::Haswell => {
                std::arch::is_x86_feature_detected!("avx2")
                    && std::arch::is_x86_feature_detected!("bmi1")
                    && std::arch::is_x86_feature_detected!("bmi2")
                    && std::arch::is_x86_feature_detected!("lzcnt")
                    && std::arch::is_x86_feature_detected!("pclmulqdq")
            }
            #[cfg(target_arch = "x86_64")]
            Backend::Westmere => std::arch::is_x86_feature_detected!("sse4.2") && std::arch::is_x86_feature_detected!("pclmulqdq"),
            #[cfg(not(target_arch = "x86_64"))]
            Backend::Haswell | Backend::Westmere => false,
            Backend::Fallback => true,
        }
    }
}

/// The best backend this CPU runs (found once).
pub fn best() -> Backend {
    use std::sync::atomic::{AtomicU8, Ordering};
    static BEST: AtomicU8 = AtomicU8::new(0);
    match BEST.load(Ordering::Relaxed) {
        1 => Backend::Arm64,
        2 => Backend::Haswell,
        3 => Backend::Westmere,
        4 => Backend::Fallback,
        _ => {
            let b = Backend::ALL.into_iter().find(|b| b.runs()).unwrap_or(Backend::Fallback);
            BEST.store(b as u8, Ordering::Relaxed);
            b
        }
    }
}

/// A kernel, compiled for each backend and run on the best the CPU runs (simdjson's run-time
/// choice of implementation).
///
/// ```ignore
/// #[inline(always)]
/// unsafe fn count<S: Simd8x64>(src: &[u8], x: u8) -> u32 { … }
/// rerail_simd8::dispatch! {
///     /// How many bytes of `src` are `x`.
///     pub fn count_x(src: &[u8], x: u8) -> u32 = count, on count_x_on;
/// }
/// ```
///
/// gives `count_x(src, x)` on `best()`, and `count_x_on(backend, src, x)` on a given one (for
/// tests; `None` where the CPU does not run it). The kernel must be `#[inline(always)]`: it is
/// inlined into a function with the backend's features enabled, and its operations with it.
#[macro_export]
macro_rules! dispatch {
    ($(#[$m:meta])* $vis:vis fn $name:ident($($arg:ident: $ty:ty),* $(,)?) -> $ret:ty = $kernel:ident, on $on:ident;) => {
        $(#[$m])*
        #[inline]
        $vis fn $name($($arg: $ty),*) -> $ret {
            #[cfg(target_arch = "aarch64")]
            {
                // SAFETY: NEON is part of every aarch64 target.
                unsafe { $kernel::<$crate::Arm64>($($arg),*) }
            }
            #[cfg(not(target_arch = "aarch64"))]
            {
                $on($crate::best(), $($arg),*).expect("the best backend runs")
            }
        }

        /// As the kernel's dispatched function, on `backend`: `None` where the CPU does not run it.
        #[allow(dead_code)]
        $vis fn $on(backend: $crate::Backend, $($arg: $ty),*) -> Option<$ret> {
            // (`best` runs: its features are not detected again.)
            if backend != $crate::best() && !backend.runs() {
                return None;
            }
            match backend {
                #[cfg(target_arch = "aarch64")]
                // SAFETY: the CPU runs NEON (checked above).
                $crate::Backend::Arm64 => Some(unsafe { $kernel::<$crate::Arm64>($($arg),*) }),
                #[cfg(target_arch = "x86_64")]
                $crate::Backend::Haswell => {
                    #[target_feature(enable = "avx2,bmi1,bmi2,lzcnt,popcnt,pclmulqdq")]
                    unsafe fn haswell($($arg: $ty),*) -> $ret {
                        unsafe { $kernel::<$crate::Haswell>($($arg),*) }
                    }
                    // SAFETY: the CPU runs AVX2 and the rest (checked above).
                    Some(unsafe { haswell($($arg),*) })
                }
                #[cfg(target_arch = "x86_64")]
                $crate::Backend::Westmere => {
                    #[target_feature(enable = "sse4.2,popcnt,pclmulqdq")]
                    unsafe fn westmere($($arg: $ty),*) -> $ret {
                        unsafe { $kernel::<$crate::Westmere>($($arg),*) }
                    }
                    // SAFETY: the CPU runs SSE4.2 and PCLMULQDQ (checked above).
                    Some(unsafe { westmere($($arg),*) })
                }
                // SAFETY: the fallback is plain Rust.
                $crate::Backend::Fallback => Some(unsafe { $kernel::<$crate::Fallback>($($arg),*) }),
                #[allow(unreachable_patterns)]
                _ => None,
            }
        }
    };
}

#[cfg(target_arch = "aarch64")]
pub use arm64::Arm64;
#[cfg(target_arch = "x86_64")]
pub use x86::{Haswell, Westmere};

/// simdjson's arm64 implementation: NEON.
#[cfg(target_arch = "aarch64")]
mod arm64 {
    use std::arch::aarch64::*;

    pub struct Arm64;

    unsafe impl super::Simd8x64 for Arm64 {
        type V = [uint8x16_t; 4];
        const BACKEND: super::Backend = super::Backend::Arm64;

        #[inline(always)]
        unsafe fn load(b: &[u8; 64]) -> Self::V {
            let p = b.as_ptr();
            unsafe { [vld1q_u8(p), vld1q_u8(p.add(16)), vld1q_u8(p.add(32)), vld1q_u8(p.add(48))] }
        }
        #[inline(always)]
        unsafe fn splat(x: u8) -> Self::V {
            unsafe { [vdupq_n_u8(x); 4] }
        }
        #[inline(always)]
        unsafe fn eq(a: Self::V, b: Self::V) -> Self::V {
            unsafe { std::array::from_fn(|i| vceqq_u8(a[i], b[i])) }
        }
        #[inline(always)]
        unsafe fn and(a: Self::V, b: Self::V) -> Self::V {
            unsafe { std::array::from_fn(|i| vandq_u8(a[i], b[i])) }
        }
        #[inline(always)]
        unsafe fn or(a: Self::V, b: Self::V) -> Self::V {
            unsafe { std::array::from_fn(|i| vorrq_u8(a[i], b[i])) }
        }
        #[inline(always)]
        unsafe fn high_nibbles(a: Self::V) -> Self::V {
            unsafe { std::array::from_fn(|i| vshrq_n_u8::<4>(a[i])) }
        }
        #[inline(always)]
        unsafe fn low_nibbles(a: Self::V) -> Self::V {
            unsafe { std::array::from_fn(|i| vandq_u8(a[i], vdupq_n_u8(15))) }
        }
        #[inline(always)]
        unsafe fn lookup_16(idx: Self::V, table: &[u8; 16]) -> Self::V {
            unsafe {
                let t = vld1q_u8(table.as_ptr());
                std::array::from_fn(|i| vqtbl1q_u8(t, idx[i]))
            }
        }
        #[inline(always)]
        unsafe fn nonzero(a: Self::V) -> u64 {
            unsafe { Self::to_bitmask(std::array::from_fn(|i| vtstq_u8(a[i], a[i]))) }
        }
        /// simdjson's `to_bitmask`: each byte ANDed with its bit, then pairwise adds.
        #[inline(always)]
        unsafe fn to_bitmask(a: Self::V) -> u64 {
            const BITS: [u8; 16] = [1, 2, 4, 8, 16, 32, 64, 128, 1, 2, 4, 8, 16, 32, 64, 128];
            unsafe {
                let bits = vld1q_u8(BITS.as_ptr());
                let ab = vpaddq_u8(vandq_u8(a[0], bits), vandq_u8(a[1], bits));
                let cd = vpaddq_u8(vandq_u8(a[2], bits), vandq_u8(a[3], bits));
                let s = vpaddq_u8(ab, cd);
                let s = vpaddq_u8(s, s);
                vgetq_lane_u64::<0>(vreinterpretq_u64_u8(s))
            }
        }
    }
}

/// simdjson's haswell (AVX2) and westmere (SSE) implementations.
#[cfg(target_arch = "x86_64")]
mod x86 {
    use std::arch::x86_64::*;

    pub struct Haswell;

    unsafe impl super::Simd8x64 for Haswell {
        type V = [__m256i; 2];
        const BACKEND: super::Backend = super::Backend::Haswell;

        #[inline(always)]
        unsafe fn load(b: &[u8; 64]) -> Self::V {
            let p = b.as_ptr().cast::<__m256i>();
            unsafe { [_mm256_loadu_si256(p), _mm256_loadu_si256(p.add(1))] }
        }
        #[inline(always)]
        unsafe fn splat(x: u8) -> Self::V {
            unsafe { [_mm256_set1_epi8(x as i8); 2] }
        }
        #[inline(always)]
        unsafe fn eq(a: Self::V, b: Self::V) -> Self::V {
            unsafe { [_mm256_cmpeq_epi8(a[0], b[0]), _mm256_cmpeq_epi8(a[1], b[1])] }
        }
        #[inline(always)]
        unsafe fn and(a: Self::V, b: Self::V) -> Self::V {
            unsafe { [_mm256_and_si256(a[0], b[0]), _mm256_and_si256(a[1], b[1])] }
        }
        #[inline(always)]
        unsafe fn or(a: Self::V, b: Self::V) -> Self::V {
            unsafe { [_mm256_or_si256(a[0], b[0]), _mm256_or_si256(a[1], b[1])] }
        }
        /// As simdjson: a 16-bit shift, then the low nibble (x86 has no byte shift).
        #[inline(always)]
        unsafe fn high_nibbles(a: Self::V) -> Self::V {
            unsafe {
                let m = _mm256_set1_epi8(15);
                [_mm256_and_si256(_mm256_srli_epi16::<4>(a[0]), m), _mm256_and_si256(_mm256_srli_epi16::<4>(a[1]), m)]
            }
        }
        #[inline(always)]
        unsafe fn low_nibbles(a: Self::V) -> Self::V {
            unsafe {
                let m = _mm256_set1_epi8(15);
                [_mm256_and_si256(a[0], m), _mm256_and_si256(a[1], m)]
            }
        }
        /// `pshufb` looks up within each 16-byte lane: the table is in both.
        #[inline(always)]
        unsafe fn lookup_16(idx: Self::V, table: &[u8; 16]) -> Self::V {
            unsafe {
                let t = _mm256_broadcastsi128_si256(_mm_loadu_si128(table.as_ptr().cast()));
                [_mm256_shuffle_epi8(t, idx[0]), _mm256_shuffle_epi8(t, idx[1])]
            }
        }
        #[inline(always)]
        unsafe fn nonzero(a: Self::V) -> u64 {
            unsafe { !Self::to_bitmask(Self::eq(a, Self::splat(0))) }
        }
        #[inline(always)]
        unsafe fn to_bitmask(a: Self::V) -> u64 {
            unsafe { u64::from(_mm256_movemask_epi8(a[0]) as u32) | u64::from(_mm256_movemask_epi8(a[1]) as u32) << 32 }
        }
    }

    pub struct Westmere;

    unsafe impl super::Simd8x64 for Westmere {
        type V = [__m128i; 4];
        const BACKEND: super::Backend = super::Backend::Westmere;

        #[inline(always)]
        unsafe fn load(b: &[u8; 64]) -> Self::V {
            let p = b.as_ptr().cast::<__m128i>();
            unsafe { [_mm_loadu_si128(p), _mm_loadu_si128(p.add(1)), _mm_loadu_si128(p.add(2)), _mm_loadu_si128(p.add(3))] }
        }
        #[inline(always)]
        unsafe fn splat(x: u8) -> Self::V {
            unsafe { [_mm_set1_epi8(x as i8); 4] }
        }
        #[inline(always)]
        unsafe fn eq(a: Self::V, b: Self::V) -> Self::V {
            unsafe { std::array::from_fn(|i| _mm_cmpeq_epi8(a[i], b[i])) }
        }
        #[inline(always)]
        unsafe fn and(a: Self::V, b: Self::V) -> Self::V {
            unsafe { std::array::from_fn(|i| _mm_and_si128(a[i], b[i])) }
        }
        #[inline(always)]
        unsafe fn or(a: Self::V, b: Self::V) -> Self::V {
            unsafe { std::array::from_fn(|i| _mm_or_si128(a[i], b[i])) }
        }
        #[inline(always)]
        unsafe fn high_nibbles(a: Self::V) -> Self::V {
            unsafe { std::array::from_fn(|i| _mm_and_si128(_mm_srli_epi16::<4>(a[i]), _mm_set1_epi8(15))) }
        }
        #[inline(always)]
        unsafe fn low_nibbles(a: Self::V) -> Self::V {
            unsafe { std::array::from_fn(|i| _mm_and_si128(a[i], _mm_set1_epi8(15))) }
        }
        #[inline(always)]
        unsafe fn lookup_16(idx: Self::V, table: &[u8; 16]) -> Self::V {
            unsafe {
                let t = _mm_loadu_si128(table.as_ptr().cast());
                std::array::from_fn(|i| _mm_shuffle_epi8(t, idx[i]))
            }
        }
        #[inline(always)]
        unsafe fn nonzero(a: Self::V) -> u64 {
            unsafe { !Self::to_bitmask(Self::eq(a, Self::splat(0))) }
        }
        #[inline(always)]
        unsafe fn to_bitmask(a: Self::V) -> u64 {
            unsafe {
                (0..4).fold(0, |m, i| m | u64::from(_mm_movemask_epi8(a[i]) as u16) << (16 * i))
            }
        }
    }
}

/// simdjson's fallback: the bytes one by one.
pub struct Fallback;

unsafe impl Simd8x64 for Fallback {
    type V = [u8; 64];
    const BACKEND: Backend = Backend::Fallback;

    #[inline(always)]
    unsafe fn load(b: &[u8; 64]) -> Self::V {
        *b
    }
    #[inline(always)]
    unsafe fn splat(x: u8) -> Self::V {
        [x; 64]
    }
    #[inline(always)]
    unsafe fn eq(a: Self::V, b: Self::V) -> Self::V {
        std::array::from_fn(|i| if a[i] == b[i] { 0xff } else { 0 })
    }
    #[inline(always)]
    unsafe fn and(a: Self::V, b: Self::V) -> Self::V {
        std::array::from_fn(|i| a[i] & b[i])
    }
    #[inline(always)]
    unsafe fn or(a: Self::V, b: Self::V) -> Self::V {
        std::array::from_fn(|i| a[i] | b[i])
    }
    #[inline(always)]
    unsafe fn high_nibbles(a: Self::V) -> Self::V {
        a.map(|b| b >> 4)
    }
    #[inline(always)]
    unsafe fn low_nibbles(a: Self::V) -> Self::V {
        a.map(|b| b & 15)
    }
    #[inline(always)]
    unsafe fn lookup_16(idx: Self::V, table: &[u8; 16]) -> Self::V {
        idx.map(|i| table[usize::from(i & 15)])
    }
    #[inline(always)]
    unsafe fn nonzero(a: Self::V) -> u64 {
        a.iter().enumerate().fold(0, |m, (i, &b)| m | u64::from(b != 0) << i)
    }
    #[inline(always)]
    unsafe fn to_bitmask(a: Self::V) -> u64 {
        a.iter().enumerate().fold(0, |m, (i, &b)| m | u64::from(b >> 7) << i)
    }
    /// 8 bytes a word (SWAR): each equal byte's low bit, the 8 gathered into one byte by a
    /// multiply.
    #[inline(always)]
    unsafe fn eq_mask(a: Self::V, x: u8) -> u64 {
        const LO7: u64 = 0x7f7f_7f7f_7f7f_7f7f;
        const ONES: u64 = 0x0101_0101_0101_0101;
        let mut m = 0;
        for k in 0..8 {
            let w = u64::from_le_bytes(a[8 * k..8 * k + 8].try_into().expect("8 bytes"));
            let v = w ^ (ONES * u64::from(x));
            let hit = (!(((v & LO7) + LO7) | v) >> 7) & ONES;
            m |= (hit.wrapping_mul(0x0102_0408_1020_4080) >> 56) << (8 * k);
        }
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Per backend: the masks of `eq`, `classify` and `nonzero` over a block.
    #[inline(always)]
    unsafe fn masks<S: Simd8x64>(b: &[u8; 64]) -> [u64; 4] {
        const LO: [u8; 16] = [1, 2, 4, 8, 16, 32, 64, 128, 3, 5, 9, 17, 33, 65, 129, 255];
        const HI: [u8; 16] = [255, 1, 2, 4, 8, 16, 32, 64, 128, 6, 10, 18, 34, 66, 130, 0];
        unsafe {
            let v = S::load(b);
            let c = S::classify(v, &LO, &HI);
            [S::eq_mask(v, b'x'), S::nonzero(c), S::nonzero(S::and(c, S::splat(0x11))), S::to_bitmask(S::or(S::eq(v, S::splat(b'{')), S::eq(v, S::splat(0x80))))]
        }
    }

    dispatch! {
        fn block_masks(b: &[u8; 64]) -> [u64; 4] = masks, on block_masks_on;
    }

    #[test]
    fn every_backend_the_cpu_runs_gives_the_fallbacks_masks() {
        let mut ran = Vec::new();
        let mut b = [0u8; 64];
        let mut x = 0x2545_f491_4f6c_dd1du64;
        for _ in 0..2000 {
            for c in &mut b {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                *c = [b'x', b'{', 0x80, (x >> 8) as u8][(x % 4) as usize];
            }
            // The fallback's SWAR compare, as a byte by byte one.
            for x in [b'x', b'{', 0x80, (x >> 8) as u8, 0, 0xff] {
                let bytes = (0..64).fold(0u64, |m, i| m | u64::from(b[i] == x) << i);
                // SAFETY: the fallback is plain Rust.
                assert_eq!(unsafe { Fallback::eq_mask(Fallback::load(&b), x) }, bytes);
            }
            let want = block_masks_on(Backend::Fallback, &b).unwrap();
            assert_eq!(block_masks(&b), want);
            for be in Backend::ALL {
                if let Some(got) = block_masks_on(be, &b) {
                    assert_eq!(got, want, "{be:?}");
                    if !ran.contains(&be) {
                        ran.push(be);
                    }
                }
            }
        }
        eprintln!("backends run: {ran:?}, best {:?}", best());
        assert!(ran.contains(&best()));
    }
}
