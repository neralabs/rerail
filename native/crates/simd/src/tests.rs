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
