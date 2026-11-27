//! Each operation is its lanes' scalar operation, as `std::simd` defines it: wrapping arithmetic, shifts modulo the
//! lane's bits, masks of all ones, `as` casts, and gathers and scatters through pointers.
use super::prelude::*;

#[test]
fn the_lanes_are_their_scalar_operations() {
    let a = u32x8::from_array([0, 1, 7, 0x8000_0000, u32::MAX, 12345, 31, 64]);
    let b = u32x8::from_array([5, u32::MAX, 3, 2, 1, 0, 33, 4]);
    let (x, y) = (a.to_array(), b.to_array());
    let each = |f: fn(u32, u32) -> u32| -> [u32; 8] { core::array::from_fn(|i| f(x[i], y[i])) };
    assert_eq!((a + b).to_array(), each(u32::wrapping_add));
    assert_eq!((a - b).to_array(), each(u32::wrapping_sub));
    assert_eq!((a * b).to_array(), each(u32::wrapping_mul));
    assert_eq!((a << b).to_array(), each(|p, q| p.wrapping_shl(q)));
    assert_eq!((a >> b).to_array(), each(|p, q| p.wrapping_shr(q)));
    assert_eq!((a & b).to_array(), each(|p, q| p & q));
    assert_eq!((a ^ b).to_array(), each(|p, q| p ^ q));
    assert_eq!(a.simd_lt(b).to_array(), core::array::from_fn(|i| x[i] < y[i]));
    assert_eq!(a.simd_eq(b).to_array(), core::array::from_fn(|i| x[i] == y[i]));
    assert_eq!(a.simd_max(b).to_array(), each(u32::max));
    assert_eq!(a.leading_zeros().to_array(), core::array::from_fn(|i| x[i].leading_zeros()));
    assert_eq!(a.reduce_max(), *x.iter().max().unwrap());
    assert_eq!(a.reduce_or(), x.iter().fold(0, |s, v| s | v));
    let m = a.simd_lt(b);
    assert_eq!(m.select(a, b).to_array(), core::array::from_fn(|i| if x[i] < y[i] { x[i] } else { y[i] }));
    assert_eq!(m.to_int().to_array(), core::array::from_fn(|i| if x[i] < y[i] { -1 } else { 0 }));
    assert_eq!(m.cast::<i64>().to_array(), m.to_array());
    assert!(m.any());
    assert!(!m.all());
    assert_eq!(m.to_bitmask(), (0..8).fold(0, |s, i| s | ((x[i] < y[i]) as u64) << i));
    assert_eq!((!m).to_array(), m.to_array().map(|v| !v));

    let w: u64x8 = a.cast();
    assert_eq!(w.to_array(), x.map(u64::from));
    let s = i32x4::from_array([-1, 2, i32::MIN, 7]);
    assert_eq!(s.cast::<u64>().to_array(), [u64::MAX, 2, i32::MIN as u64, 7]);
    assert_eq!(s.cast::<u32>().to_array(), [u32::MAX, 2, 0x8000_0000, 7]);
    assert_eq!((-s).to_array(), [1, -2, i32::MIN, -7]);
    assert_eq!(s.is_negative().to_array(), [true, false, true, false]);

    let data: [u64; 6] = [10, 11, 12, 13, 14, 15];
    let p = Simd::<*const u64, 4>::splat(data.as_ptr()).wrapping_add(usizex4::from_array([5, 0, 3, 3]));
    // SAFETY: every lane's pointer is in `data`.
    let g = unsafe { u64x4::gather_ptr(p) };
    assert_eq!(g.to_array(), [15, 10, 13, 13]);
    let on = Mask::<isize, 4>::from_array([true, false, true, false]);
    // SAFETY: the enabled lanes' pointers are in `data`.
    let g = unsafe { u64x4::gather_select_ptr(p, on, u64x4::splat(0)) };
    assert_eq!(g.to_array(), [15, 0, 13, 0]);
    assert_eq!(u64x4::gather_select(&data, on, usizex4::from_array([5, 0, 9, 1]), u64x4::splat(7)).to_array(), [15, 7, 7, 7]);
    let mut out = [0u64; 6];
    let q = Simd::<*mut u64, 4>::splat(out.as_mut_ptr()).wrapping_add(usizex4::from_array([1, 2, 3, 4]));
    // SAFETY: every lane's pointer is in `out`.
    unsafe { u64x4::from_array([1, 2, 3, 4]).scatter_select_ptr(q, on) };
    assert_eq!(out, [0, 1, 0, 3, 0, 0]);
}

#[test]
fn the_wide_lanes_are_their_scalar_operations() {
    let x: [u64; 4] = [0x1234_5678_9abc_def0, u64::MAX, 7, 1 << 40];
    let y: [u64; 4] = [0x9E37_79B9_7F4A_7C15, 3, 65, 29];
    let (a, b) = (u64x4::from_array(x), u64x4::from_array(y));
    let each = |f: fn(u64, u64) -> u64| -> [u64; 4] { core::array::from_fn(|i| f(x[i], y[i])) };
    assert_eq!((a * b).to_array(), each(u64::wrapping_mul), "mul");
    assert_eq!((a << b).to_array(), each(|p, q| p.wrapping_shl(q as u32)), "shl");
    assert_eq!((a >> b).to_array(), each(|p, q| p.wrapping_shr(q as u32)), "shr");
    assert_eq!(a.simd_lt(b).to_array(), core::array::from_fn(|i| x[i] < y[i]), "lt");
    assert_eq!(a.simd_ne(b).to_array(), core::array::from_fn(|i| x[i] != y[i]), "ne");
    assert_eq!(a.simd_max(b).to_array(), each(u64::max), "max");
    assert_eq!(a.cast::<u32>().to_array(), x.map(|v| v as u32), "narrow");
    let m = a.simd_lt(b);
    assert_eq!(m.cast::<i32>().to_array(), m.to_array(), "mask narrow");
    assert_eq!(m.cast::<i32>().cast::<isize>().to_array(), m.to_array(), "mask widen");
    assert_eq!(m.select(a, b).to_array(), core::array::from_fn(|i| if x[i] < y[i] { x[i] } else { y[i] }), "select");
    let data = [1u8, 2, 3, 4, 5, 6, 7, 8];
    let p0 = Simd::<*const u8, 4>::splat(data.as_ptr());
    let p1 = p0.wrapping_add(usizex4::from_array([1, 2, 3, 4]));
    let ps = m.cast::<isize>().select(p1, p0);
    assert_eq!(ps.addr().to_array(), core::array::from_fn(|i| data.as_ptr() as usize + if x[i] < y[i] { i + 1 } else { 0 }), "select of pointers");
    let s = i64x4::from_array([-8, 8, i64::MIN, -1]);
    assert_eq!((s >> i64x4::splat(2)).to_array(), [-2, 2, i64::MIN >> 2, -1], "arithmetic shr");
    assert_eq!(s.simd_lt(i64x4::splat(0)).to_array(), [true, false, true, true], "signed lt");
    let o = isizex4_from([-1, 2, -3, 4]);
    let q = Simd::<*const u64, 4>::splat(core::ptr::null::<u64>().wrapping_add(100)).wrapping_offset(o);
    assert_eq!(q.addr().to_array(), [792, 816, 776, 832], "wrapping_offset in T");
}

fn isizex4_from(a: [isize; 4]) -> Simd<isize, 4> {
    Simd::from_array(a)
}

#[test]
fn a_dynamic_swizzle_is_each_lanes_index() {
    let check = |n: usize| {
        let v: [u8; 64] = core::array::from_fn(|i| 100 + i as u8);
        let ix: [u8; 64] = core::array::from_fn(|i| ((i * 37 + 5) % (n + 9)) as u8);
        let want = |i: usize| if (ix[i] as usize) < n { v[ix[i] as usize] } else { 0 };
        match n {
            16 => assert_eq!(Simd::<u8, 16>::from_slice(&v).swizzle_dyn(Simd::from_slice(&ix)).to_array(), core::array::from_fn(want)),
            32 => assert_eq!(Simd::<u8, 32>::from_slice(&v).swizzle_dyn(Simd::from_slice(&ix)).to_array(), core::array::from_fn(want)),
            64 => assert_eq!(Simd::<u8, 64>::from_slice(&v).swizzle_dyn(Simd::from_slice(&ix)).to_array(), core::array::from_fn(want)),
            _ => assert_eq!(Simd::<u8, 8>::from_slice(&v).swizzle_dyn(Simd::from_slice(&ix)).to_array(), core::array::from_fn(want)),
        }
    };
    for n in [8, 16, 32, 64] {
        check(n);
    }
}

/// A 64-bit mask's `all` and `any` over 8 lanes: whether every (any) lane is set, the lane that is not set anywhere
/// among them (a sealed chunk's low bit among raw ones, `rerail_ukl_rust::lanes::col_get_in`).
#[test]
fn a_mask_of_eight_64_bit_lanes_is_all_only_where_every_lane_is() {
    let words = [0x10ec0u64, 0x10ec0, 0x10ec0, 0x10ec0, 0x10ec0, 0x10ec0, 0xffff_ffff_ffff_d240, 0xffff_ffff_ffff_c941];
    for at in 0..8 {
        let mut w = [0x10ec0u64; 8];
        w[at] = 0xffff_ffff_ffff_c941;
        let raw = (Simd::<u64, 8>::from_array(w) & Simd::splat(1)).simd_eq(Simd::splat(0));
        assert!(!raw.all(), "lane {at} is not raw: {:#x}", raw.to_bitmask());
        assert!(raw.any(), "lane {at}: the others are raw");
        assert_eq!(raw.to_bitmask(), 0xff & !(1 << at), "lane {at}");
    }
    let raw = (Simd::<u64, 8>::from_array(words) & Simd::splat(1)).simd_eq(Simd::splat(0));
    assert_eq!((raw.all(), raw.to_bitmask()), (false, 0x7f));
}

/// Every lane type's operations, each lane its scalar operation's, at a chunk's width and past it: the target's
/// chunk code (NEON, SSE2 to AVX2), and the per-lane fallback where it has none.
#[test]
fn every_lane_types_chunks_are_their_scalar_operations() {
    macro_rules! check {
        ($($t:ty)*) => {$({
            const N: usize = 16;
            let mut x = [0 as $t; N];
            let mut y = [0 as $t; N];
            let mut h = 0x9E37_79B9_7F4A_7C15u64;
            for i in 0..N {
                h = h.wrapping_mul(0x5851_F42D_4C95_7F2D).wrapping_add(1442695040888963407);
                x[i] = (h >> 7) as $t;
                y[i] = if i % 3 == 0 { x[i] } else { (h >> 29) as $t };
            }
            x[1] = <$t>::MAX;
            x[2] = <$t>::MIN;
            y[5] = 0;
            let (a, b) = (Simd::<$t, N>::from_array(x), Simd::<$t, N>::from_array(y));
            let each = |f: &dyn Fn($t, $t) -> $t| -> [$t; N] { core::array::from_fn(|i| f(x[i], y[i])) };
            assert_eq!((a + b).to_array(), each(&|p, q| p.wrapping_add(q)), "{}", stringify!($t));
            assert_eq!((a - b).to_array(), each(&|p, q| p.wrapping_sub(q)), "{}", stringify!($t));
            assert_eq!((a * b).to_array(), each(&|p, q| p.wrapping_mul(q)), "{}", stringify!($t));
            assert_eq!((a & b).to_array(), each(&|p, q| p & q));
            assert_eq!((a | b).to_array(), each(&|p, q| p | q));
            assert_eq!((a ^ b).to_array(), each(&|p, q| p ^ q));
            assert_eq!((!a).to_array(), x.map(|p| !p));
            // Shifts by each lane's amount, and by one amount (SSE2's shift by a register).
            assert_eq!((a << b).to_array(), each(&|p, q| p.wrapping_shl(q as u32)), "{} <<", stringify!($t));
            assert_eq!((a >> b).to_array(), each(&|p, q| p.wrapping_shr(q as u32)), "{} >>", stringify!($t));
            for k in [0 as $t, 1, 5, (<$t>::BITS - 1) as $t] {
                let s = Simd::<$t, N>::splat(k);
                assert_eq!((a << s).to_array(), x.map(|p| p.wrapping_shl(k as u32)), "{} << {k}", stringify!($t));
                assert_eq!((a >> s).to_array(), x.map(|p| p.wrapping_shr(k as u32)), "{} >> {k}", stringify!($t));
            }
            assert_eq!(a.simd_eq(b).to_array(), core::array::from_fn(|i| x[i] == y[i]), "{} ==", stringify!($t));
            assert_eq!(a.simd_lt(b).to_array(), core::array::from_fn(|i| x[i] < y[i]), "{} <", stringify!($t));
            assert_eq!(a.simd_le(b).to_array(), core::array::from_fn(|i| x[i] <= y[i]), "{} <=", stringify!($t));
            assert_eq!(a.simd_ge(b).to_array(), core::array::from_fn(|i| x[i] >= y[i]), "{} >=", stringify!($t));
            assert_eq!(a.simd_min(b).to_array(), each(&|p, q| p.min(q)), "{} min", stringify!($t));
            assert_eq!(a.simd_max(b).to_array(), each(&|p, q| p.max(q)), "{} max", stringify!($t));
            assert_eq!(a.leading_zeros().to_array().map(|v| v as $t), x.map(|p| p.leading_zeros() as $t), "{} clz", stringify!($t));
            let m = a.simd_lt(b);
            assert_eq!(m.to_bitmask(), (0..N).fold(0, |s, i| s | ((x[i] < y[i]) as u64) << i), "{} bitmask", stringify!($t));
            assert_eq!(m.any(), x.iter().zip(&y).any(|(p, q)| p < q));
            assert!(!(a.simd_ne(a)).any() && a.simd_eq(a).all());
            assert_eq!(m.select(a, b).to_array(), each(&|p, q| if p < q { p } else { q }));
        })*};
    }
    check!(u8 i8 u16 i16 u32 i32 u64 i64 usize isize);
}

/// The cast's chunk code: 4 bytes to 8 (signed: extended), 8 to 4 (truncated).
#[test]
fn casts_widen_and_narrow_as_as_does() {
    let s = i32x8::from_array([-1, 2, i32::MIN, 7, i32::MAX, -9, 0, 1 << 20]);
    assert_eq!(s.cast::<i64>().to_array(), s.to_array().map(i64::from));
    let u = u32x8::from_array([u32::MAX, 2, 0x8000_0000, 7, 9, 0, 1, 3]);
    assert_eq!(u.cast::<u64>().to_array(), u.to_array().map(u64::from));
    let w = u64x8::from_array([u64::MAX, 1 << 33 | 5, 7, 0x8000_0000_0000_0001, 9, 0, 1, 3]);
    assert_eq!(w.cast::<u32>().to_array(), w.to_array().map(|v| v as u32));
}

/// A 64-byte table looked up by 64 indices, those past it 0: NEON's `tbl`s, x86's `pshufb`s, or each lane's.
#[test]
fn a_dynamic_swizzle_of_64_bytes_is_each_lanes_index() {
    let t: [u8; 64] = core::array::from_fn(|i| (i as u8).wrapping_mul(37).wrapping_add(11));
    let ix: [u8; 64] = core::array::from_fn(|i| (i as u8).wrapping_mul(53) ^ if i % 5 == 0 { 0x80 } else { 0 });
    let r = Simd::<u8, 64>::from_array(t).swizzle_dyn(Simd::from_array(ix));
    assert_eq!(r.to_array(), core::array::from_fn(|i| if (ix[i] as usize) < 64 { t[ix[i] as usize] } else { 0 }));
}

/// A mask of 64 one-byte lanes: its bitmask is each lane's bit (NEON folds the four registers at once).
#[test]
fn a_mask_of_64_bytes_has_each_lanes_bit() {
    for seed in 0..64u64 {
        let x: [u8; 64] = core::array::from_fn(|i| ((i as u64).wrapping_mul(0x9e37_79b9).wrapping_add(seed * 7) >> 3) as u8);
        let m = Simd::<u8, 64>::from_array(x).simd_lt(Simd::splat(0x80));
        assert_eq!(m.to_bitmask(), (0..64).fold(0, |b, i| b | u64::from(x[i] < 0x80) << i), "seed {seed}");
        let s = Simd::<i8, 64>::from_array(x.map(|v| v as i8)).simd_lt(Simd::splat(0));
        assert_eq!(s.to_bitmask(), (0..64).fold(0, |b, i| b | u64::from(x[i] >= 0x80) << i), "seed {seed}, signed");
    }
}

/// Shifts of one-byte lanes by each amount, left and right, at 16, 32 and 64 lanes: each lane `std::simd`'s.
#[test]
fn byte_shifts_are_each_lanes() {
    let x: [u8; 64] = core::array::from_fn(|i| i as u8 * 3 + 1);
    macro_rules! at {
        ($n:literal, $($o:literal)*) => {$({
            let v = Simd::<u8, $n>::from_slice(&x);
            let l = v.shift_elements_left::<$o>(0xee).to_array();
            let r = v.shift_elements_right::<$o>(0xee).to_array();
            for i in 0..$n {
                assert_eq!(l[i], if i + $o < $n { x[i + $o] } else { 0xee }, "{} lanes left {}: lane {i}", $n, $o);
                assert_eq!(r[i], if i < $o { 0xee } else { x[i - $o] }, "{} lanes right {}: lane {i}", $n, $o);
            }
        })*};
    }
    at!(16, 0 1 2 3 7 15 16);
    at!(32, 0 1 3 15 16 17 31 32);
    at!(64, 0 1 2 3 15 16 17 33 47 48 61 62 63 64);
}

/// Unsigned saturating adds and subtracts and absolute differences, every pair of bytes and a spread of `u32`s: each
/// lane the scalar's.
#[test]
fn unsigned_saturating_ops_are_each_lanes() {
    use crate::num::SimdUint;
    for a in 0..=255u8 {
        let x = Simd::<u8, 64>::splat(a);
        let y: [u8; 64] = core::array::from_fn(|i| (i * 4 + a as usize % 4) as u8);
        let ys = Simd::from_array(y);
        let (s, d, ad) = (x.saturating_add(ys).to_array(), x.saturating_sub(ys).to_array(), x.abs_diff(ys).to_array());
        for i in 0..64 {
            assert_eq!((s[i], d[i], ad[i]), (a.saturating_add(y[i]), a.saturating_sub(y[i]), a.abs_diff(y[i])), "{a} and {}", y[i]);
        }
    }
    let v: [u32; 8] = [0, 1, 7, u32::MAX, u32::MAX - 1, 1 << 31, 12345, 3];
    let w: [u32; 8] = [5, 1, u32::MAX, 2, u32::MAX, 1 << 31, 54321, 0];
    let (a, b) = (Simd::from_array(v), Simd::from_array(w));
    let (s, d) = (a.saturating_add(b).to_array(), a.saturating_sub(b).to_array());
    for i in 0..8 {
        assert_eq!((s[i], d[i]), (v[i].saturating_add(w[i]), v[i].saturating_sub(w[i])), "lane {i}");
    }
}

#[test]
fn a_nibble_table_lookup_is_each_lanes_entry() {
    let t: [u8; 16] = core::array::from_fn(|i| 200 + i as u8);
    // Every byte value as an index: the table's entry below 16, 0 from 16 on (128 and past included).
    for at in (0..256).step_by(64) {
        let ix: [u8; 64] = core::array::from_fn(|i| (at + i) as u8);
        let want: [u8; 64] = core::array::from_fn(|i| if ix[i] < 16 { t[ix[i] as usize] } else { 0 });
        assert_eq!(Simd::<u8, 64>::from_array(ix).lookup16(Simd::from_array(t)).to_array(), want);
        let ix16: [u8; 16] = core::array::from_fn(|i| ix[i]);
        assert_eq!(Simd::<u8, 16>::from_array(ix16).lookup16(Simd::from_array(t)).to_array(), core::array::from_fn::<u8, 16, _>(|i| want[i]));
    }
}
