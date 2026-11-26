//! `Simd8x64`'s operations on `std::simd`'s interface (`rerail-simd`): the Bitlex lexers' Rust calls these, each
//! once here, so that their code is code of `std::simd` (it becomes `use std::simd` once that is stable) on every
//! target `rerail-simd` has chunks for, and no backend is chosen at run time.
//!
//! A block is 64 bytes (`V`); a compare's result is a vector of `0xff` and `0`, as `Simd8x64`'s, so tests combine
//! before one `to_bitmask`.

use rerail_simd::prelude::*;
use rerail_simd::{Mask, Simd};

/// A block's 64 bytes.
pub type V = Simd<u8, 64>;

#[inline(always)]
fn ones(m: Mask<i8, 64>) -> V {
    // SAFETY: a mask's lanes are -1 or 0, as bytes `0xff` or 0; `i8` and `u8` arrays have one layout.
    V::from_array(unsafe { core::mem::transmute::<[i8; 64], [u8; 64]>(m.to_int().to_array()) })
}

/// 64 bytes from `b`.
#[inline(always)]
pub fn load(b: &[u8; 64]) -> V {
    V::from_array(*b)
}

/// Each byte is `x`.
#[inline(always)]
pub fn splat(x: u8) -> V {
    V::splat(x)
}

/// Byte by byte, `a == b`: `0xff` or `0`.
#[inline(always)]
pub fn eq(a: V, b: V) -> V {
    ones(a.simd_eq(b))
}

/// Byte by byte, `a & b`.
#[inline(always)]
pub fn and(a: V, b: V) -> V {
    a & b
}

/// Byte by byte, `a | b`.
#[inline(always)]
pub fn or(a: V, b: V) -> V {
    a | b
}

/// Byte by byte, `a ^ b`.
#[inline(always)]
pub fn xor(a: V, b: V) -> V {
    a ^ b
}

/// Each byte's high nibble (`b >> 4`).
#[inline(always)]
pub fn high_nibbles(a: V) -> V {
    a >> V::splat(4)
}

/// Each byte's low nibble (`b & 15`).
#[inline(always)]
pub fn low_nibbles(a: V) -> V {
    a & V::splat(15)
}

/// Byte by byte, `table[i]` for each nibble `i` of `idx`: a 16-byte table's `swizzle_dyn` of each 16 bytes (NEON's
/// `tbl`, `pshufb`).
#[inline(always)]
pub fn lookup_16(idx: V, table: &[u8; 16]) -> V {
    let t = Simd::<u8, 16>::from_array(*table);
    let i = idx.to_array();
    let mut o = [0u8; 64];
    for k in 0..4 {
        let q: [u8; 16] = i[16 * k..16 * k + 16].try_into().expect("16 bytes");
        t.swizzle_dyn(Simd::from_array(q)).copy_to_slice(&mut o[16 * k..]);
    }
    V::from_array(o)
}

/// Bit `i`: byte `i` is not 0.
#[inline(always)]
pub fn nonzero(a: V) -> u64 {
    a.simd_ne(V::splat(0)).to_bitmask()
}

/// Bit `i`: byte `i`'s high bit (of a compare's result: it is true).
#[inline(always)]
pub fn to_bitmask(a: V) -> u64 {
    a.simd_ge(V::splat(0x80)).to_bitmask()
}

/// True where `a & b` is not zero: a class test kept as a vector, so tests are combined before one `to_bitmask`.
#[inline(always)]
pub fn test(a: V, b: V) -> V {
    ones((a & b).simd_ne(V::splat(0)))
}

/// True where a byte is `x` or above, unsigned.
#[inline(always)]
pub fn ge(a: V, x: u8) -> V {
    ones(a.simd_ge(V::splat(x)))
}

/// Bytes 1 to 63 of `a`, then `next`: each byte's next one, for a pair of bytes tested at once.
#[inline(always)]
pub fn shift_in(a: V, next: u8) -> V {
    a.shift_elements_left::<1>(next)
}

/// Each byte's `k`th byte before it (`k` 1, 2 or 3), the first `k` from the end of `before` (simdjson's `prev`).
#[inline(always)]
pub fn prev(a: V, before: V, k: usize) -> V {
    match k {
        1 => a.shift_elements_right::<1>(0) | before.shift_elements_left::<63>(0),
        2 => a.shift_elements_right::<2>(0) | before.shift_elements_left::<62>(0),
        3 => a.shift_elements_right::<3>(0) | before.shift_elements_left::<61>(0),
        _ => unreachable!("prev of 1, 2 or 3 bytes"),
    }
}

/// Byte by byte, `a - b` saturated at 0: not zero where `a > b`, unsigned (simdjson's `gt_bits`).
#[inline(always)]
pub fn gt_bits(a: V, b: V) -> V {
    a.saturating_sub(b)
}

/// The 32 bytes at `b`: the bits of the bytes that are `x`, and of those that are `y`.
#[inline(always)]
pub fn marks32(b: &[u8; 32], x: u8, y: u8) -> (u32, u32) {
    let v = Simd::<u8, 32>::from_array(*b);
    (v.simd_eq(Simd::splat(x)).to_bitmask() as u32, v.simd_eq(Simd::splat(y)).to_bitmask() as u32)
}

/// Bit `i`: byte `i` is `x`.
#[inline(always)]
pub fn eq_mask(a: V, x: u8) -> u64 {
    a.simd_eq(V::splat(x)).to_bitmask()
}

/// Each byte's classes by simdjson's nibble tables: `lo[b & 15] & hi[b >> 4]`, a class a bit.
#[inline(always)]
pub fn classify(a: V, lo: &[u8; 16], hi: &[u8; 16]) -> V {
    lookup_16(low_nibbles(a), lo) & lookup_16(high_nibbles(a), hi)
}

/// A kernel as a function: `dispatch!`'s form, with no backend to choose (the kernel's code is `std::simd`'s, its
/// target's at build time). The `on` name is not made.
#[macro_export]
macro_rules! kernel {
    ($(#[$m:meta])* $vis:vis fn $name:ident($($arg:ident: $ty:ty),* $(,)?) -> $ret:ty = $kernel:ident, on $on:ident;) => {
        $(#[$m])*
        #[inline]
        $vis fn $name($($arg: $ty),*) -> $ret {
            // SAFETY: the kernel's only requirement was its backend's, which `std::simd`'s code has not.
            unsafe { $kernel($($arg),*) }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Simd8x64;

    /// Each operation is `Simd8x64`'s, on the best backend this CPU runs, over blocks of every byte.
    #[test]
    fn each_operation_is_simd8x64s() {
        fn on<S: Simd8x64>() {
            let lo: [u8; 16] = core::array::from_fn(|i| (i as u8).wrapping_mul(29) ^ 0x5a);
            let hi: [u8; 16] = core::array::from_fn(|i| (i as u8).wrapping_mul(71) ^ 0xa5);
            for seed in 0..8u8 {
                let a: [u8; 64] = core::array::from_fn(|i| (i as u8).wrapping_mul(37).wrapping_add(seed.wrapping_mul(64)));
                let b: [u8; 64] = core::array::from_fn(|i| (i as u8).wrapping_mul(11) ^ seed.wrapping_mul(3));
                // SAFETY: `S` is a backend this CPU runs (the caller's).
                unsafe {
                    let (sa, sb) = (S::load(&a), S::load(&b));
                    let (pa, pb) = (load(&a), load(&b));
                    // Two vectors are the same where each byte value is at the same bytes of both.
                    let same = |s: S::V, p: V, what: &str| {
                        for v in 0..=255u8 {
                            assert_eq!(S::eq_mask(s, v), eq_mask(p, v), "{what}: its bytes {v:#x}");
                        }
                        assert_eq!(S::nonzero(s), nonzero(p), "{what}: its non-zero bytes");
                    };
                    // `to_bitmask` of a compare's result (`Simd8x64`'s is of those alone).
                    let mask = |s: S::V, p: V, what: &str| assert_eq!(S::to_bitmask(s), to_bitmask(p), "{what}: its bitmask");
                    same(S::eq(sa, sb), eq(pa, pb), "eq");
                    mask(S::eq(sa, sb), eq(pa, pb), "eq");
                    same(S::and(sa, sb), and(pa, pb), "and");
                    same(S::or(sa, sb), or(pa, pb), "or");
                    same(S::xor(sa, sb), xor(pa, pb), "xor");
                    same(S::high_nibbles(sa), high_nibbles(pa), "high_nibbles");
                    same(S::low_nibbles(sa), low_nibbles(pa), "low_nibbles");
                    same(S::lookup_16(S::low_nibbles(sa), &lo), lookup_16(low_nibbles(pa), &lo), "lookup_16");
                    same(S::classify(sa, &lo, &hi), classify(pa, &lo, &hi), "classify");
                    same(S::test(sa, sb), test(pa, pb), "test");
                    mask(S::test(sa, sb), test(pa, pb), "test");
                    same(S::ge(sa, 0x90), ge(pa, 0x90), "ge");
                    mask(S::ge(sa, 0x90), ge(pa, 0x90), "ge");
                    same(S::shift_in(sa, 0x22), shift_in(pa, 0x22), "shift_in");
                    for k in 1..=3 {
                        same(S::prev(sa, sb, k), prev(pa, pb, k), "prev");
                    }
                    same(S::gt_bits(sa, sb), gt_bits(pa, pb), "gt_bits");
                    let h: &[u8; 32] = a[..32].try_into().expect("32 bytes");
                    assert_eq!(S::marks32(h, a[3], a[7]), marks32(h, a[3], a[7]), "marks32");
                }
            }
        }
        #[cfg(target_arch = "aarch64")]
        on::<crate::Arm64>();
        #[cfg(not(target_arch = "aarch64"))]
        on::<crate::Fallback>();
    }
}

#[cfg(test)]
mod speed {
    use super::*;
    use crate::Simd8x64;

    /// Each operation's time over 16 MB of blocks, `std::simd`'s against `Simd8x64`'s on this CPU's best backend: where
    /// the lexers' time goes.
    #[test]
    #[ignore = "a report: each portable operation's time against Simd8x64's"]
    fn report_each_operation_against_simd8x64() {
        let bytes: Vec<u8> = (0..1usize << 24).map(|i| (i.wrapping_mul(2654435761) >> 7) as u8).collect();
        let blocks: Vec<[u8; 64]> = bytes.chunks_exact(64).map(|c| c.try_into().expect("64")).collect();
        let lo: [u8; 16] = core::array::from_fn(|i| i as u8 * 3);
        let hi: [u8; 16] = core::array::from_fn(|i| i as u8 * 5);
        let time = |f: &dyn Fn(&[u8; 64], &[u8; 64]) -> u64| {
            let mut best = f64::MAX;
            for _ in 0..5 {
                let t = std::time::Instant::now();
                let mut acc = 0u64;
                for w in blocks.windows(2) {
                    acc = acc.wrapping_add(f(&w[1], &w[0]));
                }
                std::hint::black_box(acc);
                best = best.min(t.elapsed().as_secs_f64());
            }
            best * 1e9 / blocks.len() as f64
        };
        macro_rules! both {
            ($name:literal, |$a:ident, $b:ident| $port:expr, |$sa:ident, $sb:ident| $simd8:expr) => {{
                let p = time(&|x, y| {
                    let ($a, $b) = (load(x), load(y));
                    $port
                });
                #[cfg(target_arch = "aarch64")]
                type B = crate::Arm64;
                #[cfg(not(target_arch = "aarch64"))]
                type B = crate::Fallback;
                // SAFETY: `B` is a backend this CPU runs.
                let s = time(&|x, y| unsafe {
                    let ($sa, $sb) = (B::load(x), B::load(y));
                    $simd8
                });
                eprintln!("{:>14}: portable {p:6.2} ns a block, Simd8x64 {s:6.2} ns ({:.1}x)", $name, p / s);
            }};
        }
        both!("eq_mask", |a, b| eq_mask(a, 0x22) ^ eq_mask(b, 0x5c), |a, b| B::eq_mask(a, 0x22) ^ B::eq_mask(b, 0x5c));
        both!("classify", |a, b| to_bitmask(test(classify(a, &lo, &hi), splat(3))) ^ eq_mask(b, 1), |a, b| B::to_bitmask(B::test(B::classify(a, &lo, &hi), B::splat(3))) ^ B::eq_mask(b, 1));
        both!("high_nibbles", |a, b| eq_mask(high_nibbles(a), 3) ^ eq_mask(b, 1), |a, b| B::eq_mask(B::high_nibbles(a), 3) ^ B::eq_mask(b, 1));
        both!("lookup_16", |a, b| eq_mask(lookup_16(low_nibbles(a), &lo), 3) ^ eq_mask(b, 1), |a, b| B::eq_mask(B::lookup_16(B::low_nibbles(a), &lo), 3) ^ B::eq_mask(b, 1));
        both!("test+bitmask", |a, b| to_bitmask(test(a, b)), |a, b| B::to_bitmask(B::test(a, b)));
        both!("ge", |a, b| to_bitmask(ge(a, 0xc0)) ^ eq_mask(b, 1), |a, b| B::to_bitmask(B::ge(a, 0xc0)) ^ B::eq_mask(b, 1));
        both!("prev", |a, b| eq_mask(prev(a, b, 1), 3) ^ eq_mask(prev(a, b, 3), 5), |a, b| B::eq_mask(B::prev(a, b, 1), 3) ^ B::eq_mask(B::prev(a, b, 3), 5));
        both!("shift_in", |a, b| eq_mask(shift_in(a, 7), 3) ^ eq_mask(b, 1), |a, b| B::eq_mask(B::shift_in(a, 7), 3) ^ B::eq_mask(b, 1));
        both!("gt_bits", |a, b| nonzero(gt_bits(a, b)), |a, b| B::nonzero(B::gt_bits(a, b)));
        both!("nonzero", |a, b| nonzero(a & b), |a, b| B::nonzero(B::and(a, b)));
    }
}
