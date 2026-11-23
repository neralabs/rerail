//! `wide_lanes` at 8 lanes (2 NEON registers), written by rerail-ukl-vec's Rust (`rs.rs`) from its vector tape: do
//! not edit; `UKL_RS_WRITE=1 cargo test -p rerail-ukl-vec _kernels_rust_is` writes it again.

#![allow(unused, unused_mut, unused_parens, unused_unsafe, unreachable_code, non_snake_case, clippy::all, unsafe_op_in_unsafe_fn)]

use crate::Launch;
use core::arch::aarch64::*;
use core::sync::atomic::{AtomicU32, Ordering::Relaxed};

/// Lane `j`'s index in its register's `j`: `[0, 1, 2, 3]`, then `[4, 5, 6, 7]`, ...: a chunk's lanes.
const LANE: [[u32; 4]; 2] = [[0, 1, 2, 3], [4, 5, 6, 7]];

/// Whether a lane of mask `k` is on: a comparison of its bits, which the compiler folds where it knows `k`.
#[inline(always)]
unsafe fn any(k: &[uint32x4_t; 2]) -> bool {
    let mut x = k[0];
    for r in 1..2 {
        x = vorrq_u32(x, k[r]);
    }
    core::mem::transmute::<uint32x4_t, u128>(x) != 0
}

/// Whether every lane of mask `k` is on, the same way.
#[inline(always)]
unsafe fn all(k: &[uint32x4_t; 2]) -> bool {
    let mut x = k[0];
    for r in 1..2 {
        x = vandq_u32(x, k[r]);
    }
    core::mem::transmute::<uint32x4_t, u128>(x) == u128::MAX
}

/// Mask `k`'s lanes as bits, lane `j` bit `j`.
#[inline(always)]
unsafe fn bits(k: &[uint32x4_t; 2]) -> u32 {
    let w = vld1q_u32([1u32, 2, 4, 8].as_ptr());
    let mut b = 0u32;
    for r in 0..2 {
        b |= vaddvq_u32(vandq_u32(k[r], w)) << (4 * r);
    }
    b
}

/// The largest and the least of the lanes of `a`; the largest of those mask `k` has on (0 for one off).
#[inline(always)]
unsafe fn maxv(a: &[uint32x4_t; 2]) -> u32 {
    let mut m = a[0];
    for r in 1..2 {
        m = vmaxq_u32(m, a[r]);
    }
    vmaxvq_u32(m)
}
#[inline(always)]
unsafe fn minv(a: &[uint32x4_t; 2]) -> u32 {
    let mut m = a[0];
    for r in 1..2 {
        m = vminq_u32(m, a[r]);
    }
    vminvq_u32(m)
}
#[inline(always)]
unsafe fn maxv_on(k: &[uint32x4_t; 2], a: &[uint32x4_t; 2]) -> u32 {
    let mut m = vandq_u32(k[0], a[0]);
    for r in 1..2 {
        m = vmaxq_u32(m, vandq_u32(k[r], a[r]));
    }
    vmaxvq_u32(m)
}

/// Operations of one register NEON has as several: `!=`; a product's high half; a 64-bit value's high half of a sum
/// (less its low half's carry) and of a difference (plus its borrow); and the order and the equality of 64-bit values
/// of their halves (the high halves' order, signed or not, or where they are equal the low halves').
#[inline(always)]
unsafe fn vcneq_u32(a: uint32x4_t, b: uint32x4_t) -> uint32x4_t {
	vmvnq_u32(vceqq_u32(a, b))
}
#[inline(always)]
unsafe fn vmulhiq_u32(a: uint32x4_t, b: uint32x4_t) -> uint32x4_t {
	vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(a), vget_low_u32(b))), vreinterpretq_u32_u64(vmull_high_u32(a, b)))
}
#[inline(always)]
unsafe fn vadcq_u32(a: uint32x4_t, b: uint32x4_t, c: uint32x4_t) -> uint32x4_t {
	vsubq_u32(vaddq_u32(a, b), c)
}
#[inline(always)]
unsafe fn vsbcq_u32(a: uint32x4_t, b: uint32x4_t, c: uint32x4_t) -> uint32x4_t {
	vaddq_u32(vsubq_u32(a, b), c)
}
#[inline(always)]
unsafe fn vlt64q_u32(al: uint32x4_t, ah: uint32x4_t, bl: uint32x4_t, bh: uint32x4_t) -> uint32x4_t {
	vorrq_u32(vcltq_u32(ah, bh), vandq_u32(vceqq_u32(ah, bh), vcltq_u32(al, bl)))
}
#[inline(always)]
unsafe fn vlt64sq_u32(al: uint32x4_t, ah: uint32x4_t, bl: uint32x4_t, bh: uint32x4_t) -> uint32x4_t {
	vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(ah), vreinterpretq_s32_u32(bh)), vandq_u32(vceqq_u32(ah, bh), vcltq_u32(al, bl)))
}
#[inline(always)]
unsafe fn veq64q_u32(al: uint32x4_t, ah: uint32x4_t, bl: uint32x4_t, bh: uint32x4_t) -> uint32x4_t {
	vandq_u32(vceqq_u32(al, bl), vceqq_u32(ah, bh))
}
/// A 64-bit value in every lane: its halves.
#[inline(always)]
unsafe fn splat64(v: u64) -> [uint32x4_t; 4] {
	core::array::from_fn(|r| vdupq_n_u32(if r < 2 { v as u32 } else { (v >> 32) as u32 }))
}
/// Lanes `f + j`.
#[inline(always)]
unsafe fn iota(f: u32) -> [uint32x4_t; 2] {
	core::array::from_fn(|r| vaddq_u32(vdupq_n_u32(f), vld1q_u32(LANE[r].as_ptr())))
}
/// An array's first registers, and those from its 2th (a 64-bit value's halves).
#[inline(always)]
unsafe fn lo<const N: usize, const M: usize>(a: [uint32x4_t; M]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| a[r])
}
#[inline(always)]
unsafe fn hi<const N: usize, const M: usize>(a: [uint32x4_t; M]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| a[r + 2])
}
/// Each register's operation, over the registers.
#[inline(always)]
unsafe fn cne_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vcneq_u32(a0[r], a1[r]))
}
#[inline(always)]
unsafe fn mulhi_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vmulhiq_u32(a0[r], a1[r]))
}
#[inline(always)]
unsafe fn adc_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N], a2: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vadcq_u32(a0[r], a1[r], a2[r]))
}
#[inline(always)]
unsafe fn sbc_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N], a2: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vsbcq_u32(a0[r], a1[r], a2[r]))
}
#[inline(always)]
unsafe fn lt64_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N], a2: [uint32x4_t; N], a3: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vlt64q_u32(a0[r], a1[r], a2[r], a3[r]))
}
#[inline(always)]
unsafe fn lt64s_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N], a2: [uint32x4_t; N], a3: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vlt64sq_u32(a0[r], a1[r], a2[r], a3[r]))
}
#[inline(always)]
unsafe fn eq64_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N], a2: [uint32x4_t; N], a3: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| veq64q_u32(a0[r], a1[r], a2[r], a3[r]))
}
#[inline(always)]
unsafe fn and_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vandq_u32(a0[r], a1[r]))
}
#[inline(always)]
unsafe fn orr_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vorrq_u32(a0[r], a1[r]))
}
#[inline(always)]
unsafe fn eor_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| veorq_u32(a0[r], a1[r]))
}
#[inline(always)]
unsafe fn bic_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vbicq_u32(a0[r], a1[r]))
}
#[inline(always)]
unsafe fn mvn_r<const N: usize>(a0: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vmvnq_u32(a0[r]))
}
#[inline(always)]
unsafe fn bsl_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N], a2: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vbslq_u32(a0[r], a1[r], a2[r]))
}
#[inline(always)]
unsafe fn add_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vaddq_u32(a0[r], a1[r]))
}
#[inline(always)]
unsafe fn sub_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vsubq_u32(a0[r], a1[r]))
}
#[inline(always)]
unsafe fn mul_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vmulq_u32(a0[r], a1[r]))
}
#[inline(always)]
unsafe fn mla_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N], a2: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vmlaq_u32(a0[r], a1[r], a2[r]))
}
#[inline(always)]
unsafe fn max_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vmaxq_u32(a0[r], a1[r]))
}
#[inline(always)]
unsafe fn min_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vminq_u32(a0[r], a1[r]))
}
#[inline(always)]
unsafe fn ceq_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vceqq_u32(a0[r], a1[r]))
}
#[inline(always)]
unsafe fn clt_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vcltq_u32(a0[r], a1[r]))
}
#[inline(always)]
unsafe fn cle_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vcleq_u32(a0[r], a1[r]))
}
#[inline(always)]
unsafe fn cgt_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vcgtq_u32(a0[r], a1[r]))
}
#[inline(always)]
unsafe fn cge_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vcgeq_u32(a0[r], a1[r]))
}
#[inline(always)]
unsafe fn tst_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vtstq_u32(a0[r], a1[r]))
}
#[inline(always)]
unsafe fn uzp1_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vuzp1q_u32(a0[r], a1[r]))
}
#[inline(always)]
unsafe fn uzp2_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vuzp2q_u32(a0[r], a1[r]))
}
/// The lanes' indexes `a` where mask `k` is on, else the largest of those (a lane that does not run loads where one that does
/// does).
#[inline(always)]
unsafe fn ix_on(k: [uint32x4_t; 2], a: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
	let s = vdupq_n_u32(maxv_on(&k, &a));
	core::array::from_fn(|r| vbslq_u32(k[r], a[r], s))
}
#[inline(always)]
unsafe fn ld_u8(b: *const u8, ix: [uint32x4_t; 2], s: usize) -> [uint32x4_t; 2] {
	let u = minv(&ix);
	if u == maxv(&ix) {
		return [vdupq_n_u32((b.add(u as usize * s) as *const u8).read_unaligned() as u32); 2];
	}
	let a = words2(ix);
	let mut g = [0u32; 8];
	for j in 0..8 {
		let v = (b.add(a[j] as usize * s) as *const u8).read_unaligned();
		g[j] = v as u32;
	}
	regs2(g)
}
#[inline(always)]
unsafe fn ld_i8(b: *const u8, ix: [uint32x4_t; 2], s: usize) -> [uint32x4_t; 2] {
	let u = minv(&ix);
	if u == maxv(&ix) {
		return [vdupq_n_u32((b.add(u as usize * s) as *const i8).read_unaligned() as i32 as u32); 2];
	}
	let a = words2(ix);
	let mut g = [0u32; 8];
	for j in 0..8 {
		let v = (b.add(a[j] as usize * s) as *const i8).read_unaligned();
		g[j] = v as i32 as u32;
	}
	regs2(g)
}
#[inline(always)]
unsafe fn ld_u16(b: *const u8, ix: [uint32x4_t; 2], s: usize) -> [uint32x4_t; 2] {
	let u = minv(&ix);
	if u == maxv(&ix) {
		return [vdupq_n_u32((b.add(u as usize * s) as *const u16).read_unaligned() as u32); 2];
	}
	let a = words2(ix);
	let mut g = [0u32; 8];
	for j in 0..8 {
		let v = (b.add(a[j] as usize * s) as *const u16).read_unaligned();
		g[j] = v as u32;
	}
	regs2(g)
}
#[inline(always)]
unsafe fn ld_i16(b: *const u8, ix: [uint32x4_t; 2], s: usize) -> [uint32x4_t; 2] {
	let u = minv(&ix);
	if u == maxv(&ix) {
		return [vdupq_n_u32((b.add(u as usize * s) as *const i16).read_unaligned() as i32 as u32); 2];
	}
	let a = words2(ix);
	let mut g = [0u32; 8];
	for j in 0..8 {
		let v = (b.add(a[j] as usize * s) as *const i16).read_unaligned();
		g[j] = v as i32 as u32;
	}
	regs2(g)
}
#[inline(always)]
unsafe fn ld_u32(b: *const u8, ix: [uint32x4_t; 2], s: usize) -> [uint32x4_t; 2] {
	let u = minv(&ix);
	if u == maxv(&ix) {
		return [vdupq_n_u32((b.add(u as usize * s) as *const u32).read_unaligned() as u32); 2];
	}
	let a = words2(ix);
	let mut g = [0u32; 8];
	for j in 0..8 {
		let v = (b.add(a[j] as usize * s) as *const u32).read_unaligned();
		g[j] = v as u32;
	}
	regs2(g)
}
#[inline(always)]
unsafe fn ld_u64(b: *const u8, ix: [uint32x4_t; 2], s: usize) -> [uint32x4_t; 4] {
	let u = minv(&ix);
	if u == maxv(&ix) {
		let v = (b.add(u as usize * s) as *const u64).read_unaligned();
		return core::array::from_fn(|r| vdupq_n_u32(if r < 2 { v as u32 } else { (v >> 32) as u32 }));
	}
	let a = words2(ix);
	let mut g = [0u32; 16];
	for j in 0..8 {
		let v = (b.add(a[j] as usize * s) as *const u64).read_unaligned();
		g[j] = v as u32;
		g[8 + j] = (v >> 32) as u32;
	}
	regs4(g)
}
#[inline(always)]
unsafe fn ldj_u8(at: impl Fn(usize) -> *const u8) -> [uint32x4_t; 2] {
	let mut g = [0u32; 8];
	for j in 0..8 {
		let v = (at(j) as *const u8).read_unaligned();
		g[j] = v as u32;
	}
	regs2(g)
}
#[inline(always)]
unsafe fn stj_u8(k: [uint32x4_t; 2], v: [uint32x4_t; 2], at: impl Fn(usize) -> *mut u8) {
	let (m, x) = (words2(k), words2(v));
	let mut z = 0u64;
	let sink = core::hint::black_box(&raw mut z) as *mut u8;
	for j in 0..8 {
		(core::hint::select_unpredictable(m[j] != 0, at(j), sink) as *mut u8).write_unaligned(x[j] as u8);
	}
}
#[inline(always)]
unsafe fn stjf_u8(v: [uint32x4_t; 2], at: impl Fn(usize) -> *mut u8) {
	let x = words2(v);
	for j in 0..8 {
		(at(j) as *mut u8).write_unaligned(x[j] as u8);
	}
}
#[inline(always)]
unsafe fn ldj_i8(at: impl Fn(usize) -> *const u8) -> [uint32x4_t; 2] {
	let mut g = [0u32; 8];
	for j in 0..8 {
		let v = (at(j) as *const i8).read_unaligned();
		g[j] = v as i32 as u32;
	}
	regs2(g)
}
#[inline(always)]
unsafe fn ldj_u16(at: impl Fn(usize) -> *const u8) -> [uint32x4_t; 2] {
	let mut g = [0u32; 8];
	for j in 0..8 {
		let v = (at(j) as *const u16).read_unaligned();
		g[j] = v as u32;
	}
	regs2(g)
}
#[inline(always)]
unsafe fn stj_u16(k: [uint32x4_t; 2], v: [uint32x4_t; 2], at: impl Fn(usize) -> *mut u8) {
	let (m, x) = (words2(k), words2(v));
	let mut z = 0u64;
	let sink = core::hint::black_box(&raw mut z) as *mut u8;
	for j in 0..8 {
		(core::hint::select_unpredictable(m[j] != 0, at(j), sink) as *mut u16).write_unaligned(x[j] as u16);
	}
}
#[inline(always)]
unsafe fn stjf_u16(v: [uint32x4_t; 2], at: impl Fn(usize) -> *mut u8) {
	let x = words2(v);
	for j in 0..8 {
		(at(j) as *mut u16).write_unaligned(x[j] as u16);
	}
}
#[inline(always)]
unsafe fn ldj_i16(at: impl Fn(usize) -> *const u8) -> [uint32x4_t; 2] {
	let mut g = [0u32; 8];
	for j in 0..8 {
		let v = (at(j) as *const i16).read_unaligned();
		g[j] = v as i32 as u32;
	}
	regs2(g)
}
#[inline(always)]
unsafe fn ldj_u32(at: impl Fn(usize) -> *const u8) -> [uint32x4_t; 2] {
	let mut g = [0u32; 8];
	for j in 0..8 {
		let v = (at(j) as *const u32).read_unaligned();
		g[j] = v as u32;
	}
	regs2(g)
}
#[inline(always)]
unsafe fn stj_u32(k: [uint32x4_t; 2], v: [uint32x4_t; 2], at: impl Fn(usize) -> *mut u8) {
	let (m, x) = (words2(k), words2(v));
	let mut z = 0u64;
	let sink = core::hint::black_box(&raw mut z) as *mut u8;
	for j in 0..8 {
		(core::hint::select_unpredictable(m[j] != 0, at(j), sink) as *mut u32).write_unaligned(x[j] as u32);
	}
}
#[inline(always)]
unsafe fn stjf_u32(v: [uint32x4_t; 2], at: impl Fn(usize) -> *mut u8) {
	let x = words2(v);
	for j in 0..8 {
		(at(j) as *mut u32).write_unaligned(x[j] as u32);
	}
}
#[inline(always)]
unsafe fn ldj_u64(at: impl Fn(usize) -> *const u8) -> [uint32x4_t; 4] {
	let mut g = [0u32; 16];
	for j in 0..8 {
		let v = (at(j) as *const u64).read_unaligned();
		g[j] = v as u32;
		g[8 + j] = (v >> 32) as u32;
	}
	regs4(g)
}
#[inline(always)]
unsafe fn stj_u64(k: [uint32x4_t; 2], v: [uint32x4_t; 4], at: impl Fn(usize) -> *mut u8) {
	let (m, x) = (words2(k), words4(v));
	let mut z = 0u64;
	let sink = core::hint::black_box(&raw mut z) as *mut u8;
	for j in 0..8 {
		(core::hint::select_unpredictable(m[j] != 0, at(j), sink) as *mut u64).write_unaligned((x[8 + j] as u64) << 32 | x[j] as u64);
	}
}
#[inline(always)]
unsafe fn stjf_u64(v: [uint32x4_t; 4], at: impl Fn(usize) -> *mut u8) {
	let x = words4(v);
	for j in 0..8 {
		(at(j) as *mut u64).write_unaligned((x[8 + j] as u64) << 32 | x[j] as u64);
	}
}
/// Each lane's address of its halves `lo` and `hi`; a lane mask `k` has off, the first on's.
#[inline(always)]
unsafe fn addrs(lo: [uint32x4_t; 2], hi: [uint32x4_t; 2], k: [uint32x4_t; 2]) -> [usize; 8] {
	let (l, h, b) = (words2(lo), words2(hi), bits(&k));
	let f = b.trailing_zeros() as usize % 8;
	core::array::from_fn(|j| {
		let j = if b >> j & 1 != 0 { j } else { f };
		((h[j] as u64) << 32 | l[j] as u64) as usize
	})
}
#[inline(always)]
unsafe fn bsl_w(k: [uint32x4_t; 2], a: [uint32x4_t; 4], b: [uint32x4_t; 4]) -> [uint32x4_t; 4] {
	core::array::from_fn(|r| vbslq_u32(k[r % 2], a[r], b[r]))
}
#[inline(always)]
unsafe fn cat_w(a: [uint32x4_t; 2], b: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
	core::array::from_fn(|r| if r < 2 { a[r] } else { b[r - 2] })
}
/// Registers' lanes as words, and words as registers' lanes.
#[inline(always)]
unsafe fn words2(a: [uint32x4_t; 2]) -> [u32; 8] {
    core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(a)
}
#[inline(always)]
unsafe fn regs2(a: [u32; 8]) -> [uint32x4_t; 2] {
    core::mem::transmute::<[u32; 8], [uint32x4_t; 2]>(a)
}
#[inline(always)]
unsafe fn words4(a: [uint32x4_t; 4]) -> [u32; 16] {
    core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(a)
}
#[inline(always)]
unsafe fn regs4(a: [u32; 16]) -> [uint32x4_t; 4] {
    core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(a)
}

/// `wide_lanes_inner`.
#[inline(always)]
unsafe fn b0_r0r1r2r3sg_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, arg2: *const u8, arg3: *const u8, x4: *const u8) {
	let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x4 as *const u32).read_unaligned();
    let a1: u64 = (x4.wrapping_add(8u64 as usize) as *const u64).read_unaligned();
	let mut x6 = [vdupq_n_u32(0); 4];
	let mut x7 = [vdupq_n_u32(0); 4];
	let mut x8 = [vdupq_n_u32(0); 2];
	let mut x9 = [vdupq_n_u32(0); 2];
	let mut x10: u32 = 0;
	let mut x11 = [vdupq_n_u32(0); 4];
	let mut x12 = [vdupq_n_u32(0); 4];
	let mut x13: u32 = 0;
	let mut x14: u32 = 0;
	let mut x15: u32 = 0;
	let mut live = k;
	let t0: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
	let t1 = [vdupq_n_u32(a0); 2];
	let t2: [uint32x4_t; 2] = [vcgeq_u32(t0[0], t1[0]), vcgeq_u32(t0[1], t1[1])];
	let t3: [uint32x4_t; 2] = [vandq_u32(live[0], t2[0]), vandq_u32(live[1], t2[1])];
	{
		live = [vbicq_u32(live[0], t3[0]), vbicq_u32(live[1], t3[1])];
	}
	if any(&live) {
		let t4 = (arg0 as *const u8).add(((gid0) as usize) * 8);
		let mut t5: [uint32x4_t; 4];
		if all(&live) {
			let t6 = vld2q_u32((t4 as *const u32).add(0));
			let t7 = vld2q_u32((t4 as *const u32).add(8));
			let t8: [uint32x4_t; 4] = [t6.0, t7.0, t6.1, t7.1];
			t5 = t8;
		} else {
			let t9 = vdupq_n_u32(maxv_on(&live, &t0));
			let t10: [uint32x4_t; 2] = [vbslq_u32(live[0], t0[0], t9), vbslq_u32(live[1], t0[1], t9)];
			let mut t11 = [vdupq_n_u32(0); 4];
			let t12 = minv(&t10);
			if t12 == maxv(&t10) {
				let t13 = ((arg0 as *const u8).add(t12 as usize * 8) as *const u64).read_unaligned();
				t11 = [vdupq_n_u32(t13 as u32), vdupq_n_u32(t13 as u32), vdupq_n_u32((t13 >> 32) as u32), vdupq_n_u32((t13 >> 32) as u32)];
			} else {
				{ let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t10[0]) as usize * 8) as *const u64).read_unaligned(); t11[0] = vsetq_lane_u32::<0>(v as u32, t11[0]); t11[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t11[2]); }
				{ let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t10[0]) as usize * 8) as *const u64).read_unaligned(); t11[0] = vsetq_lane_u32::<1>(v as u32, t11[0]); t11[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t11[2]); }
				{ let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t10[0]) as usize * 8) as *const u64).read_unaligned(); t11[0] = vsetq_lane_u32::<2>(v as u32, t11[0]); t11[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t11[2]); }
				{ let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t10[0]) as usize * 8) as *const u64).read_unaligned(); t11[0] = vsetq_lane_u32::<3>(v as u32, t11[0]); t11[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t11[2]); }
				{ let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t10[1]) as usize * 8) as *const u64).read_unaligned(); t11[1] = vsetq_lane_u32::<0>(v as u32, t11[1]); t11[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t11[3]); }
				{ let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t10[1]) as usize * 8) as *const u64).read_unaligned(); t11[1] = vsetq_lane_u32::<1>(v as u32, t11[1]); t11[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t11[3]); }
				{ let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t10[1]) as usize * 8) as *const u64).read_unaligned(); t11[1] = vsetq_lane_u32::<2>(v as u32, t11[1]); t11[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t11[3]); }
				{ let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t10[1]) as usize * 8) as *const u64).read_unaligned(); t11[1] = vsetq_lane_u32::<3>(v as u32, t11[1]); t11[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t11[3]); }
			}
			t5 = t11;
		}
		x6 = t5;
		let t14 = (arg1 as *const u8).add(((gid0) as usize) * 8);
		let mut t15: [uint32x4_t; 4];
		if all(&live) {
			let t16 = vld2q_u32((t14 as *const u32).add(0));
			let t17 = vld2q_u32((t14 as *const u32).add(8));
			let t18: [uint32x4_t; 4] = [t16.0, t17.0, t16.1, t17.1];
			t15 = t18;
		} else {
			let t19 = vdupq_n_u32(maxv_on(&live, &t0));
			let t20: [uint32x4_t; 2] = [vbslq_u32(live[0], t0[0], t19), vbslq_u32(live[1], t0[1], t19)];
			let mut t21 = [vdupq_n_u32(0); 4];
			let t22 = minv(&t20);
			if t22 == maxv(&t20) {
				let t23 = ((arg1 as *const u8).add(t22 as usize * 8) as *const u64).read_unaligned();
				t21 = [vdupq_n_u32(t23 as u32), vdupq_n_u32(t23 as u32), vdupq_n_u32((t23 >> 32) as u32), vdupq_n_u32((t23 >> 32) as u32)];
			} else {
				{ let v = ((arg1 as *const u8).add(vgetq_lane_u32::<0>(t20[0]) as usize * 8) as *const u64).read_unaligned(); t21[0] = vsetq_lane_u32::<0>(v as u32, t21[0]); t21[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t21[2]); }
				{ let v = ((arg1 as *const u8).add(vgetq_lane_u32::<1>(t20[0]) as usize * 8) as *const u64).read_unaligned(); t21[0] = vsetq_lane_u32::<1>(v as u32, t21[0]); t21[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t21[2]); }
				{ let v = ((arg1 as *const u8).add(vgetq_lane_u32::<2>(t20[0]) as usize * 8) as *const u64).read_unaligned(); t21[0] = vsetq_lane_u32::<2>(v as u32, t21[0]); t21[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t21[2]); }
				{ let v = ((arg1 as *const u8).add(vgetq_lane_u32::<3>(t20[0]) as usize * 8) as *const u64).read_unaligned(); t21[0] = vsetq_lane_u32::<3>(v as u32, t21[0]); t21[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t21[2]); }
				{ let v = ((arg1 as *const u8).add(vgetq_lane_u32::<0>(t20[1]) as usize * 8) as *const u64).read_unaligned(); t21[1] = vsetq_lane_u32::<0>(v as u32, t21[1]); t21[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t21[3]); }
				{ let v = ((arg1 as *const u8).add(vgetq_lane_u32::<1>(t20[1]) as usize * 8) as *const u64).read_unaligned(); t21[1] = vsetq_lane_u32::<1>(v as u32, t21[1]); t21[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t21[3]); }
				{ let v = ((arg1 as *const u8).add(vgetq_lane_u32::<2>(t20[1]) as usize * 8) as *const u64).read_unaligned(); t21[1] = vsetq_lane_u32::<2>(v as u32, t21[1]); t21[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t21[3]); }
				{ let v = ((arg1 as *const u8).add(vgetq_lane_u32::<3>(t20[1]) as usize * 8) as *const u64).read_unaligned(); t21[1] = vsetq_lane_u32::<3>(v as u32, t21[1]); t21[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t21[3]); }
			}
			t15 = t21;
		}
		x7 = t15;
		let mut t24: [uint32x4_t; 2];
		if all(&live) {
			let t25 = (arg2 as *const u32).add((gid0) as usize);
			t24 = [vld1q_u32(t25.add(0)), vld1q_u32(t25.add(4))];
		} else {
			let t26 = vdupq_n_u32(maxv_on(&live, &t0));
			let t27: [uint32x4_t; 2] = [vbslq_u32(live[0], t0[0], t26), vbslq_u32(live[1], t0[1], t26)];
			let t28 = arg2 as *const u32;
			let mut t29 = [vdupq_n_u32(0); 2];
			t29[0] = vld1q_lane_u32::<0>(t28.add(vgetq_lane_u32::<0>(t27[0]) as usize), t29[0]);
			t29[0] = vld1q_lane_u32::<1>(t28.add(vgetq_lane_u32::<1>(t27[0]) as usize), t29[0]);
			t29[0] = vld1q_lane_u32::<2>(t28.add(vgetq_lane_u32::<2>(t27[0]) as usize), t29[0]);
			t29[0] = vld1q_lane_u32::<3>(t28.add(vgetq_lane_u32::<3>(t27[0]) as usize), t29[0]);
			t29[1] = vld1q_lane_u32::<0>(t28.add(vgetq_lane_u32::<0>(t27[1]) as usize), t29[1]);
			t29[1] = vld1q_lane_u32::<1>(t28.add(vgetq_lane_u32::<1>(t27[1]) as usize), t29[1]);
			t29[1] = vld1q_lane_u32::<2>(t28.add(vgetq_lane_u32::<2>(t27[1]) as usize), t29[1]);
			t29[1] = vld1q_lane_u32::<3>(t28.add(vgetq_lane_u32::<3>(t27[1]) as usize), t29[1]);
			t24 = t29;
		}
		x8 = t24;
		let t30 = [vdupq_n_u32(63u32); 2];
		let t31: [uint32x4_t; 2] = [vandq_u32(x8[0], t30[0]), vandq_u32(x8[1], t30[1])];
		x9 = t31;
		x10 = a0;
		let t32: [uint32x4_t; 2] = [vaddq_u32(x6[0], x7[0]), vaddq_u32(x6[1], x7[1])];
		let t33: [uint32x4_t; 2] = [vcltq_u32(t32[0], x6[0]), vcltq_u32(t32[1], x6[1])];
		let t34: [uint32x4_t; 4] = [t32[0], t32[1], vadcq_u32(x6[2], x7[2], t33[0]), vadcq_u32(x6[3], x7[3], t33[1])];
		let t35 = (arg3 as *const u8).add(((gid0.wrapping_add((0u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t35 as *mut u32).add(0), uint32x4x2_t(t34[0], t34[2]));
			vst2q_u32((t35 as *mut u32).add(8), uint32x4x2_t(t34[1], t34[3]));
		} else {
			let mut t36w = 0u64;
			let t36 = core::hint::black_box(&raw mut t36w) as *mut u64;
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t35 as *mut u64).add(0), t36).write_unaligned(((vgetq_lane_u32::<0>(t34[2]) as u64) << 32 | vgetq_lane_u32::<0>(t34[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t35 as *mut u64).add(1), t36).write_unaligned(((vgetq_lane_u32::<1>(t34[2]) as u64) << 32 | vgetq_lane_u32::<1>(t34[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t35 as *mut u64).add(2), t36).write_unaligned(((vgetq_lane_u32::<2>(t34[2]) as u64) << 32 | vgetq_lane_u32::<2>(t34[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t35 as *mut u64).add(3), t36).write_unaligned(((vgetq_lane_u32::<3>(t34[2]) as u64) << 32 | vgetq_lane_u32::<3>(t34[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t35 as *mut u64).add(4), t36).write_unaligned(((vgetq_lane_u32::<0>(t34[3]) as u64) << 32 | vgetq_lane_u32::<0>(t34[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t35 as *mut u64).add(5), t36).write_unaligned(((vgetq_lane_u32::<1>(t34[3]) as u64) << 32 | vgetq_lane_u32::<1>(t34[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t35 as *mut u64).add(6), t36).write_unaligned(((vgetq_lane_u32::<2>(t34[3]) as u64) << 32 | vgetq_lane_u32::<2>(t34[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t35 as *mut u64).add(7), t36).write_unaligned(((vgetq_lane_u32::<3>(t34[3]) as u64) << 32 | vgetq_lane_u32::<3>(t34[1]) as u64));
		}
		let t37: [uint32x4_t; 2] = [vcltq_u32(x6[0], x7[0]), vcltq_u32(x6[1], x7[1])];
		let t38: [uint32x4_t; 4] = [vsubq_u32(x6[0], x7[0]), vsubq_u32(x6[1], x7[1]), vsbcq_u32(x6[2], x7[2], t37[0]), vsbcq_u32(x6[3], x7[3], t37[1])];
		let t39 = (arg3 as *const u8).add(((gid0.wrapping_add((1u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t39 as *mut u32).add(0), uint32x4x2_t(t38[0], t38[2]));
			vst2q_u32((t39 as *mut u32).add(8), uint32x4x2_t(t38[1], t38[3]));
		} else {
			let mut t40w = 0u64;
			let t40 = core::hint::black_box(&raw mut t40w) as *mut u64;
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t39 as *mut u64).add(0), t40).write_unaligned(((vgetq_lane_u32::<0>(t38[2]) as u64) << 32 | vgetq_lane_u32::<0>(t38[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t39 as *mut u64).add(1), t40).write_unaligned(((vgetq_lane_u32::<1>(t38[2]) as u64) << 32 | vgetq_lane_u32::<1>(t38[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t39 as *mut u64).add(2), t40).write_unaligned(((vgetq_lane_u32::<2>(t38[2]) as u64) << 32 | vgetq_lane_u32::<2>(t38[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t39 as *mut u64).add(3), t40).write_unaligned(((vgetq_lane_u32::<3>(t38[2]) as u64) << 32 | vgetq_lane_u32::<3>(t38[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t39 as *mut u64).add(4), t40).write_unaligned(((vgetq_lane_u32::<0>(t38[3]) as u64) << 32 | vgetq_lane_u32::<0>(t38[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t39 as *mut u64).add(5), t40).write_unaligned(((vgetq_lane_u32::<1>(t38[3]) as u64) << 32 | vgetq_lane_u32::<1>(t38[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t39 as *mut u64).add(6), t40).write_unaligned(((vgetq_lane_u32::<2>(t38[3]) as u64) << 32 | vgetq_lane_u32::<2>(t38[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t39 as *mut u64).add(7), t40).write_unaligned(((vgetq_lane_u32::<3>(t38[3]) as u64) << 32 | vgetq_lane_u32::<3>(t38[1]) as u64));
		}
		let t41: [uint32x4_t; 2] = [x9[0], x9[1]];
		let t42: [uint32x4_t; 2] = [vandq_u32(t41[0], vdupq_n_u32(63)), vandq_u32(t41[1], vdupq_n_u32(63))];
		let t43: [uint32x4_t; 4] = [vshlq_u32(x6[0], vreinterpretq_s32_u32(t42[0])), vshlq_u32(x6[1], vreinterpretq_s32_u32(t42[1])), vorrq_u32(vshlq_u32(x6[2], vreinterpretq_s32_u32(t42[0])), vshlq_u32(x6[0], vsubq_s32(vreinterpretq_s32_u32(t42[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(x6[3], vreinterpretq_s32_u32(t42[1])), vshlq_u32(x6[1], vsubq_s32(vreinterpretq_s32_u32(t42[1]), vdupq_n_s32(32))))];
		let t44 = (arg3 as *const u8).add(((gid0.wrapping_add((2u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t44 as *mut u32).add(0), uint32x4x2_t(t43[0], t43[2]));
			vst2q_u32((t44 as *mut u32).add(8), uint32x4x2_t(t43[1], t43[3]));
		} else {
			let mut t45w = 0u64;
			let t45 = core::hint::black_box(&raw mut t45w) as *mut u64;
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t44 as *mut u64).add(0), t45).write_unaligned(((vgetq_lane_u32::<0>(t43[2]) as u64) << 32 | vgetq_lane_u32::<0>(t43[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t44 as *mut u64).add(1), t45).write_unaligned(((vgetq_lane_u32::<1>(t43[2]) as u64) << 32 | vgetq_lane_u32::<1>(t43[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t44 as *mut u64).add(2), t45).write_unaligned(((vgetq_lane_u32::<2>(t43[2]) as u64) << 32 | vgetq_lane_u32::<2>(t43[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t44 as *mut u64).add(3), t45).write_unaligned(((vgetq_lane_u32::<3>(t43[2]) as u64) << 32 | vgetq_lane_u32::<3>(t43[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t44 as *mut u64).add(4), t45).write_unaligned(((vgetq_lane_u32::<0>(t43[3]) as u64) << 32 | vgetq_lane_u32::<0>(t43[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t44 as *mut u64).add(5), t45).write_unaligned(((vgetq_lane_u32::<1>(t43[3]) as u64) << 32 | vgetq_lane_u32::<1>(t43[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t44 as *mut u64).add(6), t45).write_unaligned(((vgetq_lane_u32::<2>(t43[3]) as u64) << 32 | vgetq_lane_u32::<2>(t43[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t44 as *mut u64).add(7), t45).write_unaligned(((vgetq_lane_u32::<3>(t43[3]) as u64) << 32 | vgetq_lane_u32::<3>(t43[1]) as u64));
		}
		let t46: [uint32x4_t; 2] = [x9[0], x9[1]];
		let t47: [uint32x4_t; 2] = [vandq_u32(t46[0], vdupq_n_u32(63)), vandq_u32(t46[1], vdupq_n_u32(63))];
		let t48: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(x6[0], vnegq_s32(vreinterpretq_s32_u32(t47[0]))), vshlq_u32(x6[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t47[0])))), vorrq_u32(vshlq_u32(x6[1], vnegq_s32(vreinterpretq_s32_u32(t47[1]))), vshlq_u32(x6[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t47[1])))), vshlq_u32(x6[2], vnegq_s32(vreinterpretq_s32_u32(t47[0]))), vshlq_u32(x6[3], vnegq_s32(vreinterpretq_s32_u32(t47[1])))];
		let t49 = (arg3 as *const u8).add(((gid0.wrapping_add((3u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t49 as *mut u32).add(0), uint32x4x2_t(t48[0], t48[2]));
			vst2q_u32((t49 as *mut u32).add(8), uint32x4x2_t(t48[1], t48[3]));
		} else {
			let mut t50w = 0u64;
			let t50 = core::hint::black_box(&raw mut t50w) as *mut u64;
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t49 as *mut u64).add(0), t50).write_unaligned(((vgetq_lane_u32::<0>(t48[2]) as u64) << 32 | vgetq_lane_u32::<0>(t48[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t49 as *mut u64).add(1), t50).write_unaligned(((vgetq_lane_u32::<1>(t48[2]) as u64) << 32 | vgetq_lane_u32::<1>(t48[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t49 as *mut u64).add(2), t50).write_unaligned(((vgetq_lane_u32::<2>(t48[2]) as u64) << 32 | vgetq_lane_u32::<2>(t48[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t49 as *mut u64).add(3), t50).write_unaligned(((vgetq_lane_u32::<3>(t48[2]) as u64) << 32 | vgetq_lane_u32::<3>(t48[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t49 as *mut u64).add(4), t50).write_unaligned(((vgetq_lane_u32::<0>(t48[3]) as u64) << 32 | vgetq_lane_u32::<0>(t48[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t49 as *mut u64).add(5), t50).write_unaligned(((vgetq_lane_u32::<1>(t48[3]) as u64) << 32 | vgetq_lane_u32::<1>(t48[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t49 as *mut u64).add(6), t50).write_unaligned(((vgetq_lane_u32::<2>(t48[3]) as u64) << 32 | vgetq_lane_u32::<2>(t48[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t49 as *mut u64).add(7), t50).write_unaligned(((vgetq_lane_u32::<3>(t48[3]) as u64) << 32 | vgetq_lane_u32::<3>(t48[1]) as u64));
		}
		let t51: [uint32x4_t; 2] = [x9[0], x9[1]];
		let t52: [uint32x4_t; 2] = [vandq_u32(t51[0], vdupq_n_u32(63)), vandq_u32(t51[1], vdupq_n_u32(63))];
		let t53: [uint32x4_t; 2] = [vcgeq_s32(vreinterpretq_s32_u32(t52[0]), vdupq_n_s32(32)), vcgeq_s32(vreinterpretq_s32_u32(t52[1]), vdupq_n_s32(32))];
		let t54: [uint32x4_t; 4] = [vbslq_u32(t53[0], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t52[0])))), vorrq_u32(vshlq_u32(x6[0], vnegq_s32(vreinterpretq_s32_u32(t52[0]))), vshlq_u32(x6[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t52[0]))))), vbslq_u32(t53[1], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t52[1])))), vorrq_u32(vshlq_u32(x6[1], vnegq_s32(vreinterpretq_s32_u32(t52[1]))), vshlq_u32(x6[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t52[1]))))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vnegq_s32(vreinterpretq_s32_u32(t52[0])))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vnegq_s32(vreinterpretq_s32_u32(t52[1]))))];
		let t55 = (arg3 as *const u8).add(((gid0.wrapping_add((4u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t55 as *mut u32).add(0), uint32x4x2_t(t54[0], t54[2]));
			vst2q_u32((t55 as *mut u32).add(8), uint32x4x2_t(t54[1], t54[3]));
		} else {
			let mut t56w = 0u64;
			let t56 = core::hint::black_box(&raw mut t56w) as *mut u64;
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t55 as *mut u64).add(0), t56).write_unaligned(((vgetq_lane_u32::<0>(t54[2]) as u64) << 32 | vgetq_lane_u32::<0>(t54[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t55 as *mut u64).add(1), t56).write_unaligned(((vgetq_lane_u32::<1>(t54[2]) as u64) << 32 | vgetq_lane_u32::<1>(t54[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t55 as *mut u64).add(2), t56).write_unaligned(((vgetq_lane_u32::<2>(t54[2]) as u64) << 32 | vgetq_lane_u32::<2>(t54[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t55 as *mut u64).add(3), t56).write_unaligned(((vgetq_lane_u32::<3>(t54[2]) as u64) << 32 | vgetq_lane_u32::<3>(t54[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t55 as *mut u64).add(4), t56).write_unaligned(((vgetq_lane_u32::<0>(t54[3]) as u64) << 32 | vgetq_lane_u32::<0>(t54[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t55 as *mut u64).add(5), t56).write_unaligned(((vgetq_lane_u32::<1>(t54[3]) as u64) << 32 | vgetq_lane_u32::<1>(t54[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t55 as *mut u64).add(6), t56).write_unaligned(((vgetq_lane_u32::<2>(t54[3]) as u64) << 32 | vgetq_lane_u32::<2>(t54[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t55 as *mut u64).add(7), t56).write_unaligned(((vgetq_lane_u32::<3>(t54[3]) as u64) << 32 | vgetq_lane_u32::<3>(t54[1]) as u64));
		}
		let t57: [uint32x4_t; 4] = [vshlq_n_u32::<13>(x6[0]), vshlq_n_u32::<13>(x6[1]), vorrq_u32(vshlq_n_u32::<13>(x6[2]), vshrq_n_u32::<19>(x6[0])), vorrq_u32(vshlq_n_u32::<13>(x6[3]), vshrq_n_u32::<19>(x6[1]))];
		let t58: [uint32x4_t; 2] = [vshrq_n_u32::<19>(x7[2]), vshrq_n_u32::<19>(x7[3])];
		let t59: [uint32x4_t; 4] = [vorrq_u32(t57[0], t58[0]), vorrq_u32(t57[1], t58[1]), t57[2], t57[3]];
		let t60 = (arg3 as *const u8).add(((gid0.wrapping_add((5u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t60 as *mut u32).add(0), uint32x4x2_t(t59[0], t59[2]));
			vst2q_u32((t60 as *mut u32).add(8), uint32x4x2_t(t59[1], t59[3]));
		} else {
			let mut t61w = 0u64;
			let t61 = core::hint::black_box(&raw mut t61w) as *mut u64;
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t60 as *mut u64).add(0), t61).write_unaligned(((vgetq_lane_u32::<0>(t59[2]) as u64) << 32 | vgetq_lane_u32::<0>(t59[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t60 as *mut u64).add(1), t61).write_unaligned(((vgetq_lane_u32::<1>(t59[2]) as u64) << 32 | vgetq_lane_u32::<1>(t59[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t60 as *mut u64).add(2), t61).write_unaligned(((vgetq_lane_u32::<2>(t59[2]) as u64) << 32 | vgetq_lane_u32::<2>(t59[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t60 as *mut u64).add(3), t61).write_unaligned(((vgetq_lane_u32::<3>(t59[2]) as u64) << 32 | vgetq_lane_u32::<3>(t59[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t60 as *mut u64).add(4), t61).write_unaligned(((vgetq_lane_u32::<0>(t59[3]) as u64) << 32 | vgetq_lane_u32::<0>(t59[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t60 as *mut u64).add(5), t61).write_unaligned(((vgetq_lane_u32::<1>(t59[3]) as u64) << 32 | vgetq_lane_u32::<1>(t59[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t60 as *mut u64).add(6), t61).write_unaligned(((vgetq_lane_u32::<2>(t59[3]) as u64) << 32 | vgetq_lane_u32::<2>(t59[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t60 as *mut u64).add(7), t61).write_unaligned(((vgetq_lane_u32::<3>(t59[3]) as u64) << 32 | vgetq_lane_u32::<3>(t59[1]) as u64));
		}
		let t62 = [vdupq_n_u32(0), vdupq_n_u32(0), vshlq_n_u32::<8>(x6[0]), vshlq_n_u32::<8>(x6[1])];
		let t63 = (arg3 as *const u8).add(((gid0.wrapping_add((6u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t63 as *mut u32).add(0), uint32x4x2_t(t62[0], t62[2]));
			vst2q_u32((t63 as *mut u32).add(8), uint32x4x2_t(t62[1], t62[3]));
		} else {
			let mut t64w = 0u64;
			let t64 = core::hint::black_box(&raw mut t64w) as *mut u64;
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t63 as *mut u64).add(0), t64).write_unaligned(((vgetq_lane_u32::<0>(t62[2]) as u64) << 32 | vgetq_lane_u32::<0>(t62[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t63 as *mut u64).add(1), t64).write_unaligned(((vgetq_lane_u32::<1>(t62[2]) as u64) << 32 | vgetq_lane_u32::<1>(t62[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t63 as *mut u64).add(2), t64).write_unaligned(((vgetq_lane_u32::<2>(t62[2]) as u64) << 32 | vgetq_lane_u32::<2>(t62[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t63 as *mut u64).add(3), t64).write_unaligned(((vgetq_lane_u32::<3>(t62[2]) as u64) << 32 | vgetq_lane_u32::<3>(t62[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t63 as *mut u64).add(4), t64).write_unaligned(((vgetq_lane_u32::<0>(t62[3]) as u64) << 32 | vgetq_lane_u32::<0>(t62[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t63 as *mut u64).add(5), t64).write_unaligned(((vgetq_lane_u32::<1>(t62[3]) as u64) << 32 | vgetq_lane_u32::<1>(t62[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t63 as *mut u64).add(6), t64).write_unaligned(((vgetq_lane_u32::<2>(t62[3]) as u64) << 32 | vgetq_lane_u32::<2>(t62[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t63 as *mut u64).add(7), t64).write_unaligned(((vgetq_lane_u32::<3>(t62[3]) as u64) << 32 | vgetq_lane_u32::<3>(t62[1]) as u64));
		}
		let t65: [uint32x4_t; 2] = [vshrq_n_u32::<3>(x6[2]), vshrq_n_u32::<3>(x6[3])];
		let t66: [uint32x4_t; 4] = [t65[0], t65[1], vdupq_n_u32(0), vdupq_n_u32(0)];
		let t67 = (arg3 as *const u8).add(((gid0.wrapping_add((7u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t67 as *mut u32).add(0), uint32x4x2_t(t66[0], t66[2]));
			vst2q_u32((t67 as *mut u32).add(8), uint32x4x2_t(t66[1], t66[3]));
		} else {
			let mut t68w = 0u64;
			let t68 = core::hint::black_box(&raw mut t68w) as *mut u64;
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t67 as *mut u64).add(0), t68).write_unaligned(((vgetq_lane_u32::<0>(t66[2]) as u64) << 32 | vgetq_lane_u32::<0>(t66[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t67 as *mut u64).add(1), t68).write_unaligned(((vgetq_lane_u32::<1>(t66[2]) as u64) << 32 | vgetq_lane_u32::<1>(t66[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t67 as *mut u64).add(2), t68).write_unaligned(((vgetq_lane_u32::<2>(t66[2]) as u64) << 32 | vgetq_lane_u32::<2>(t66[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t67 as *mut u64).add(3), t68).write_unaligned(((vgetq_lane_u32::<3>(t66[2]) as u64) << 32 | vgetq_lane_u32::<3>(t66[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t67 as *mut u64).add(4), t68).write_unaligned(((vgetq_lane_u32::<0>(t66[3]) as u64) << 32 | vgetq_lane_u32::<0>(t66[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t67 as *mut u64).add(5), t68).write_unaligned(((vgetq_lane_u32::<1>(t66[3]) as u64) << 32 | vgetq_lane_u32::<1>(t66[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t67 as *mut u64).add(6), t68).write_unaligned(((vgetq_lane_u32::<2>(t66[3]) as u64) << 32 | vgetq_lane_u32::<2>(t66[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t67 as *mut u64).add(7), t68).write_unaligned(((vgetq_lane_u32::<3>(t66[3]) as u64) << 32 | vgetq_lane_u32::<3>(t66[1]) as u64));
		}
		let t69 = vdupq_n_s32(((((40i64) as u32)) & 63) as i32);
		let t70: [uint32x4_t; 2] = [vcgeq_s32(t69, vdupq_n_s32(32)), vcgeq_s32(t69, vdupq_n_s32(32))];
		let t71: [uint32x4_t; 4] = [vbslq_u32(t70[0], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vsubq_s32(vdupq_n_s32(32), t69))), vorrq_u32(vshlq_u32(x6[0], vnegq_s32(t69)), vshlq_u32(x6[2], vsubq_s32(vdupq_n_s32(32), t69)))), vbslq_u32(t70[1], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vsubq_s32(vdupq_n_s32(32), t69))), vorrq_u32(vshlq_u32(x6[1], vnegq_s32(t69)), vshlq_u32(x6[3], vsubq_s32(vdupq_n_s32(32), t69)))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vnegq_s32(t69))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vnegq_s32(t69)))];
		let t72 = (arg3 as *const u8).add(((gid0.wrapping_add((8u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t72 as *mut u32).add(0), uint32x4x2_t(t71[0], t71[2]));
			vst2q_u32((t72 as *mut u32).add(8), uint32x4x2_t(t71[1], t71[3]));
		} else {
			let mut t6w = 0u64;
			let t6 = core::hint::black_box(&raw mut t6w) as *mut u64;
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t72 as *mut u64).add(0), t6).write_unaligned(((vgetq_lane_u32::<0>(t71[2]) as u64) << 32 | vgetq_lane_u32::<0>(t71[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t72 as *mut u64).add(1), t6).write_unaligned(((vgetq_lane_u32::<1>(t71[2]) as u64) << 32 | vgetq_lane_u32::<1>(t71[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t72 as *mut u64).add(2), t6).write_unaligned(((vgetq_lane_u32::<2>(t71[2]) as u64) << 32 | vgetq_lane_u32::<2>(t71[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t72 as *mut u64).add(3), t6).write_unaligned(((vgetq_lane_u32::<3>(t71[2]) as u64) << 32 | vgetq_lane_u32::<3>(t71[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t72 as *mut u64).add(4), t6).write_unaligned(((vgetq_lane_u32::<0>(t71[3]) as u64) << 32 | vgetq_lane_u32::<0>(t71[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t72 as *mut u64).add(5), t6).write_unaligned(((vgetq_lane_u32::<1>(t71[3]) as u64) << 32 | vgetq_lane_u32::<1>(t71[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t72 as *mut u64).add(6), t6).write_unaligned(((vgetq_lane_u32::<2>(t71[3]) as u64) << 32 | vgetq_lane_u32::<2>(t71[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t72 as *mut u64).add(7), t6).write_unaligned(((vgetq_lane_u32::<3>(t71[3]) as u64) << 32 | vgetq_lane_u32::<3>(t71[1]) as u64));
		}
		let t7: [uint32x4_t; 2] = [vmulhiq_u32(x6[0], x7[0]), vmulhiq_u32(x6[1], x7[1])];
		let t73: [uint32x4_t; 4] = [vmulq_u32(x6[0], x7[0]), vmulq_u32(x6[1], x7[1]), vmlaq_u32(vmlaq_u32(t7[0], x6[0], x7[2]), x7[0], x6[2]), vmlaq_u32(vmlaq_u32(t7[1], x6[1], x7[3]), x7[1], x6[3])];
		let t74 = (arg3 as *const u8).add(((gid0.wrapping_add((9u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t74 as *mut u32).add(0), uint32x4x2_t(t73[0], t73[2]));
			vst2q_u32((t74 as *mut u32).add(8), uint32x4x2_t(t73[1], t73[3]));
		} else {
			let mut t75w = 0u64;
			let t75 = core::hint::black_box(&raw mut t75w) as *mut u64;
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t74 as *mut u64).add(0), t75).write_unaligned(((vgetq_lane_u32::<0>(t73[2]) as u64) << 32 | vgetq_lane_u32::<0>(t73[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t74 as *mut u64).add(1), t75).write_unaligned(((vgetq_lane_u32::<1>(t73[2]) as u64) << 32 | vgetq_lane_u32::<1>(t73[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t74 as *mut u64).add(2), t75).write_unaligned(((vgetq_lane_u32::<2>(t73[2]) as u64) << 32 | vgetq_lane_u32::<2>(t73[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t74 as *mut u64).add(3), t75).write_unaligned(((vgetq_lane_u32::<3>(t73[2]) as u64) << 32 | vgetq_lane_u32::<3>(t73[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t74 as *mut u64).add(4), t75).write_unaligned(((vgetq_lane_u32::<0>(t73[3]) as u64) << 32 | vgetq_lane_u32::<0>(t73[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t74 as *mut u64).add(5), t75).write_unaligned(((vgetq_lane_u32::<1>(t73[3]) as u64) << 32 | vgetq_lane_u32::<1>(t73[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t74 as *mut u64).add(6), t75).write_unaligned(((vgetq_lane_u32::<2>(t73[3]) as u64) << 32 | vgetq_lane_u32::<2>(t73[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t74 as *mut u64).add(7), t75).write_unaligned(((vgetq_lane_u32::<3>(t73[3]) as u64) << 32 | vgetq_lane_u32::<3>(t73[1]) as u64));
		}
		let t76: u64 = 0u64;
		let t77 = [vdupq_n_u32((t76) as u32); 2];
		let t78: [uint32x4_t; 2] = [veq64q_u32(x7[0], x7[2], t77[0], vdupq_n_u32(0)), veq64q_u32(x7[1], x7[3], t77[1], vdupq_n_u32(0))];
		let t79: [uint32x4_t; 2] = [vmvnq_u32(t78[0]), vmvnq_u32(t78[1])];
		let mut t80 = [vdupq_n_u32(0); 4];
		let t81: [uint32x4_t; 2] = [vandq_u32(live[0], t79[0]), vandq_u32(live[1], t79[1])];
		let t82: [uint32x4_t; 2] = [vbicq_u32(live[0], t79[0]), vbicq_u32(live[1], t79[1])];
		{
			let (mut t83xl, mut t83xh, mut t83yl, mut t83yh) = ([0u32; 8], [0u32; 8], [0u32; 8], [0u32; 8]);
			vst1q_u32(t83xl.as_mut_ptr().add(0), x6[0]); vst1q_u32(t83xh.as_mut_ptr().add(0), x6[2]); vst1q_u32(t83yl.as_mut_ptr().add(0), x7[0]); vst1q_u32(t83yh.as_mut_ptr().add(0), x7[2]);
			vst1q_u32(t83xl.as_mut_ptr().add(4), x6[1]); vst1q_u32(t83xh.as_mut_ptr().add(4), x6[3]); vst1q_u32(t83yl.as_mut_ptr().add(4), x7[1]); vst1q_u32(t83yh.as_mut_ptr().add(4), x7[3]);
			for j in 0..8 { let q = (((t83xh[j] as u64) << 32 | t83xl[j] as u64) as u64).checked_div(((t83yh[j] as u64) << 32 | t83yl[j] as u64) as u64).unwrap_or(0) as u64; t83xl[j] = q as u32; t83xh[j] = (q >> 32) as u32; }
			let t84: [uint32x4_t; 4] = [vld1q_u32(t83xl.as_ptr().add(0)), vld1q_u32(t83xl.as_ptr().add(4)), vld1q_u32(t83xh.as_ptr().add(0)), vld1q_u32(t83xh.as_ptr().add(4))];
			t80 = [vbslq_u32(t81[0], t84[0], t80[0]), vbslq_u32(t81[1], t84[1], t80[1]), vbslq_u32(t81[0], t84[2], t80[2]), vbslq_u32(t81[1], t84[3], t80[3])];
		}
		{
			let t85: u64 = 7u64;
			let t86 = [vdupq_n_u32((t85) as u32); 2];
			let t87: [uint32x4_t; 4] = [t86[0], t86[1], vdupq_n_u32(0), vdupq_n_u32(0)];
			t80 = [vbslq_u32(t82[0], t87[0], t80[0]), vbslq_u32(t82[1], t87[1], t80[1]), vbslq_u32(t82[0], t87[2], t80[2]), vbslq_u32(t82[1], t87[3], t80[3])];
		}
		let t88 = (arg3 as *const u8).add(((gid0.wrapping_add((10u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t88 as *mut u32).add(0), uint32x4x2_t(t80[0], t80[2]));
			vst2q_u32((t88 as *mut u32).add(8), uint32x4x2_t(t80[1], t80[3]));
		} else {
			let mut t89w = 0u64;
			let t89 = core::hint::black_box(&raw mut t89w) as *mut u64;
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t88 as *mut u64).add(0), t89).write_unaligned(((vgetq_lane_u32::<0>(t80[2]) as u64) << 32 | vgetq_lane_u32::<0>(t80[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t88 as *mut u64).add(1), t89).write_unaligned(((vgetq_lane_u32::<1>(t80[2]) as u64) << 32 | vgetq_lane_u32::<1>(t80[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t88 as *mut u64).add(2), t89).write_unaligned(((vgetq_lane_u32::<2>(t80[2]) as u64) << 32 | vgetq_lane_u32::<2>(t80[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t88 as *mut u64).add(3), t89).write_unaligned(((vgetq_lane_u32::<3>(t80[2]) as u64) << 32 | vgetq_lane_u32::<3>(t80[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t88 as *mut u64).add(4), t89).write_unaligned(((vgetq_lane_u32::<0>(t80[3]) as u64) << 32 | vgetq_lane_u32::<0>(t80[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t88 as *mut u64).add(5), t89).write_unaligned(((vgetq_lane_u32::<1>(t80[3]) as u64) << 32 | vgetq_lane_u32::<1>(t80[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t88 as *mut u64).add(6), t89).write_unaligned(((vgetq_lane_u32::<2>(t80[3]) as u64) << 32 | vgetq_lane_u32::<2>(t80[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t88 as *mut u64).add(7), t89).write_unaligned(((vgetq_lane_u32::<3>(t80[3]) as u64) << 32 | vgetq_lane_u32::<3>(t80[1]) as u64));
		}
		let t90: [uint32x4_t; 2] = [vlt64q_u32(x6[0], x6[2], x7[0], x7[2]), vlt64q_u32(x6[1], x6[3], x7[1], x7[3])];
		let t91: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t90[0]), vshrq_n_u32::<31>(t90[1])];
		let t92: [uint32x4_t; 2] = [vlt64q_u32(x7[0], x7[2], x6[0], x6[2]), vlt64q_u32(x7[1], x7[3], x6[1], x6[3])];
		let t93: [uint32x4_t; 2] = [vmvnq_u32(t92[0]), vmvnq_u32(t92[1])];
		let t94: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t93[0]), vshrq_n_u32::<31>(t93[1])];
		let t95: [uint32x4_t; 4] = [vshlq_n_u32::<1>(t94[0]), vshlq_n_u32::<1>(t94[1]), vshrq_n_u32::<31>(t94[0]), vshrq_n_u32::<31>(t94[1])];
		let t96: [uint32x4_t; 4] = [vorrq_u32(t91[0], t95[0]), vorrq_u32(t91[1], t95[1]), t95[2], t95[3]];
		let t97: [uint32x4_t; 2] = [veq64q_u32(x6[0], x6[2], x7[0], x7[2]), veq64q_u32(x6[1], x6[3], x7[1], x7[3])];
		let t98: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t97[0]), vshrq_n_u32::<31>(t97[1])];
		let t99: [uint32x4_t; 4] = [vshlq_n_u32::<2>(t98[0]), vshlq_n_u32::<2>(t98[1]), vshrq_n_u32::<30>(t98[0]), vshrq_n_u32::<30>(t98[1])];
		let t100: [uint32x4_t; 4] = [vorrq_u32(t96[0], t99[0]), vorrq_u32(t96[1], t99[1]), vorrq_u32(t96[2], t99[2]), vorrq_u32(t96[3], t99[3])];
		let t101: [uint32x4_t; 2] = [veq64q_u32(x6[0], x6[2], x7[0], x7[2]), veq64q_u32(x6[1], x6[3], x7[1], x7[3])];
		let t102: [uint32x4_t; 2] = [vmvnq_u32(t101[0]), vmvnq_u32(t101[1])];
		let t103: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t102[0]), vshrq_n_u32::<31>(t102[1])];
		let t104: [uint32x4_t; 4] = [vshlq_n_u32::<3>(t103[0]), vshlq_n_u32::<3>(t103[1]), vshrq_n_u32::<29>(t103[0]), vshrq_n_u32::<29>(t103[1])];
		let t105: [uint32x4_t; 4] = [vorrq_u32(t100[0], t104[0]), vorrq_u32(t100[1], t104[1]), vorrq_u32(t100[2], t104[2]), vorrq_u32(t100[3], t104[3])];
		let t106: [uint32x4_t; 2] = [vlt64sq_u32(x6[0], x6[2], x7[0], x7[2]), vlt64sq_u32(x6[1], x6[3], x7[1], x7[3])];
		let t107: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t106[0]), vshrq_n_u32::<31>(t106[1])];
		let t108: [uint32x4_t; 4] = [vshlq_n_u32::<4>(t107[0]), vshlq_n_u32::<4>(t107[1]), vshrq_n_u32::<28>(t107[0]), vshrq_n_u32::<28>(t107[1])];
		let t109: [uint32x4_t; 4] = [vorrq_u32(t105[0], t108[0]), vorrq_u32(t105[1], t108[1]), vorrq_u32(t105[2], t108[2]), vorrq_u32(t105[3], t108[3])];
		let t110: [uint32x4_t; 2] = [vlt64sq_u32(x6[0], x6[2], x7[0], x7[2]), vlt64sq_u32(x6[1], x6[3], x7[1], x7[3])];
		let t111: [uint32x4_t; 2] = [vmvnq_u32(t110[0]), vmvnq_u32(t110[1])];
		let t112: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t111[0]), vshrq_n_u32::<31>(t111[1])];
		let t113: [uint32x4_t; 4] = [vshlq_n_u32::<5>(t112[0]), vshlq_n_u32::<5>(t112[1]), vshrq_n_u32::<27>(t112[0]), vshrq_n_u32::<27>(t112[1])];
		let t114: [uint32x4_t; 4] = [vorrq_u32(t109[0], t113[0]), vorrq_u32(t109[1], t113[1]), vorrq_u32(t109[2], t113[2]), vorrq_u32(t109[3], t113[3])];
		let t115: [uint32x4_t; 2] = [vlt64q_u32(x7[0], x7[2], x6[0], x6[2]), vlt64q_u32(x7[1], x7[3], x6[1], x6[3])];
		let t116: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t115[0]), vshrq_n_u32::<31>(t115[1])];
		let t117: [uint32x4_t; 4] = [vshlq_n_u32::<6>(t116[0]), vshlq_n_u32::<6>(t116[1]), vshrq_n_u32::<26>(t116[0]), vshrq_n_u32::<26>(t116[1])];
		let t118: [uint32x4_t; 4] = [vorrq_u32(t114[0], t117[0]), vorrq_u32(t114[1], t117[1]), vorrq_u32(t114[2], t117[2]), vorrq_u32(t114[3], t117[3])];
		let t119 = (arg3 as *const u8).add(((gid0.wrapping_add((11u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t119 as *mut u32).add(0), uint32x4x2_t(t118[0], t118[2]));
			vst2q_u32((t119 as *mut u32).add(8), uint32x4x2_t(t118[1], t118[3]));
		} else {
			let mut t120w = 0u64;
			let t120 = core::hint::black_box(&raw mut t120w) as *mut u64;
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t119 as *mut u64).add(0), t120).write_unaligned(((vgetq_lane_u32::<0>(t118[2]) as u64) << 32 | vgetq_lane_u32::<0>(t118[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t119 as *mut u64).add(1), t120).write_unaligned(((vgetq_lane_u32::<1>(t118[2]) as u64) << 32 | vgetq_lane_u32::<1>(t118[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t119 as *mut u64).add(2), t120).write_unaligned(((vgetq_lane_u32::<2>(t118[2]) as u64) << 32 | vgetq_lane_u32::<2>(t118[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t119 as *mut u64).add(3), t120).write_unaligned(((vgetq_lane_u32::<3>(t118[2]) as u64) << 32 | vgetq_lane_u32::<3>(t118[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t119 as *mut u64).add(4), t120).write_unaligned(((vgetq_lane_u32::<0>(t118[3]) as u64) << 32 | vgetq_lane_u32::<0>(t118[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t119 as *mut u64).add(5), t120).write_unaligned(((vgetq_lane_u32::<1>(t118[3]) as u64) << 32 | vgetq_lane_u32::<1>(t118[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t119 as *mut u64).add(6), t120).write_unaligned(((vgetq_lane_u32::<2>(t118[3]) as u64) << 32 | vgetq_lane_u32::<2>(t118[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t119 as *mut u64).add(7), t120).write_unaligned(((vgetq_lane_u32::<3>(t118[3]) as u64) << 32 | vgetq_lane_u32::<3>(t118[1]) as u64));
		}
		let t121: [uint32x4_t; 2] = [vaddq_u32(x8[0], x6[0]), vaddq_u32(x8[1], x6[1])];
		let t122: [uint32x4_t; 2] = [vcltq_u32(t121[0], x8[0]), vcltq_u32(t121[1], x8[1])];
		let t123: [uint32x4_t; 4] = [t121[0], t121[1], vadcq_u32(vdupq_n_u32(0), x6[2], t122[0]), vadcq_u32(vdupq_n_u32(0), x6[3], t122[1])];
		let t124 = (arg3 as *const u8).add(((gid0.wrapping_add((12u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t124 as *mut u32).add(0), uint32x4x2_t(t123[0], t123[2]));
			vst2q_u32((t124 as *mut u32).add(8), uint32x4x2_t(t123[1], t123[3]));
		} else {
			let mut t125w = 0u64;
			let t125 = core::hint::black_box(&raw mut t125w) as *mut u64;
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t124 as *mut u64).add(0), t125).write_unaligned(((vgetq_lane_u32::<0>(t123[2]) as u64) << 32 | vgetq_lane_u32::<0>(t123[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t124 as *mut u64).add(1), t125).write_unaligned(((vgetq_lane_u32::<1>(t123[2]) as u64) << 32 | vgetq_lane_u32::<1>(t123[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t124 as *mut u64).add(2), t125).write_unaligned(((vgetq_lane_u32::<2>(t123[2]) as u64) << 32 | vgetq_lane_u32::<2>(t123[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t124 as *mut u64).add(3), t125).write_unaligned(((vgetq_lane_u32::<3>(t123[2]) as u64) << 32 | vgetq_lane_u32::<3>(t123[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t124 as *mut u64).add(4), t125).write_unaligned(((vgetq_lane_u32::<0>(t123[3]) as u64) << 32 | vgetq_lane_u32::<0>(t123[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t124 as *mut u64).add(5), t125).write_unaligned(((vgetq_lane_u32::<1>(t123[3]) as u64) << 32 | vgetq_lane_u32::<1>(t123[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t124 as *mut u64).add(6), t125).write_unaligned(((vgetq_lane_u32::<2>(t123[3]) as u64) << 32 | vgetq_lane_u32::<2>(t123[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t124 as *mut u64).add(7), t125).write_unaligned(((vgetq_lane_u32::<3>(t123[3]) as u64) << 32 | vgetq_lane_u32::<3>(t123[1]) as u64));
		}
		let t126: [uint32x4_t; 2] = [vreinterpretq_u32_s32(vshrq_n_s32::<31>(vreinterpretq_s32_u32(x8[0]))), vreinterpretq_u32_s32(vshrq_n_s32::<31>(vreinterpretq_s32_u32(x8[1])))];
		let t127: [uint32x4_t; 4] = [x8[0], x8[1], t126[0], t126[1]];
		let t128 = (arg3 as *const u8).add(((gid0.wrapping_add((13u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t128 as *mut u32).add(0), uint32x4x2_t(t127[0], t127[2]));
			vst2q_u32((t128 as *mut u32).add(8), uint32x4x2_t(t127[1], t127[3]));
		} else {
			let mut t129w = 0u64;
			let t129 = core::hint::black_box(&raw mut t129w) as *mut u64;
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t128 as *mut u64).add(0), t129).write_unaligned(((vgetq_lane_u32::<0>(t127[2]) as u64) << 32 | vgetq_lane_u32::<0>(t127[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t128 as *mut u64).add(1), t129).write_unaligned(((vgetq_lane_u32::<1>(t127[2]) as u64) << 32 | vgetq_lane_u32::<1>(t127[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t128 as *mut u64).add(2), t129).write_unaligned(((vgetq_lane_u32::<2>(t127[2]) as u64) << 32 | vgetq_lane_u32::<2>(t127[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t128 as *mut u64).add(3), t129).write_unaligned(((vgetq_lane_u32::<3>(t127[2]) as u64) << 32 | vgetq_lane_u32::<3>(t127[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t128 as *mut u64).add(4), t129).write_unaligned(((vgetq_lane_u32::<0>(t127[3]) as u64) << 32 | vgetq_lane_u32::<0>(t127[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t128 as *mut u64).add(5), t129).write_unaligned(((vgetq_lane_u32::<1>(t127[3]) as u64) << 32 | vgetq_lane_u32::<1>(t127[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t128 as *mut u64).add(6), t129).write_unaligned(((vgetq_lane_u32::<2>(t127[3]) as u64) << 32 | vgetq_lane_u32::<2>(t127[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t128 as *mut u64).add(7), t129).write_unaligned(((vgetq_lane_u32::<3>(t127[3]) as u64) << 32 | vgetq_lane_u32::<3>(t127[1]) as u64));
		}
		let t130: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<3>(x6[0]), vshlq_n_u32::<29>(x6[2])), vorrq_u32(vshrq_n_u32::<3>(x6[1]), vshlq_n_u32::<29>(x6[3])), vshrq_n_u32::<3>(x6[2]), vshrq_n_u32::<3>(x6[3])];
		let t131: [uint32x4_t; 2] = [t130[0], t130[1]];
		let t132: [uint32x4_t; 4] = [t131[0], t131[1], vdupq_n_u32(0), vdupq_n_u32(0)];
		let t133 = (arg3 as *const u8).add(((gid0.wrapping_add((14u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t133 as *mut u32).add(0), uint32x4x2_t(t132[0], t132[2]));
			vst2q_u32((t133 as *mut u32).add(8), uint32x4x2_t(t132[1], t132[3]));
		} else {
			let mut t134w = 0u64;
			let t134 = core::hint::black_box(&raw mut t134w) as *mut u64;
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t133 as *mut u64).add(0), t134).write_unaligned(((vgetq_lane_u32::<0>(t132[2]) as u64) << 32 | vgetq_lane_u32::<0>(t132[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t133 as *mut u64).add(1), t134).write_unaligned(((vgetq_lane_u32::<1>(t132[2]) as u64) << 32 | vgetq_lane_u32::<1>(t132[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t133 as *mut u64).add(2), t134).write_unaligned(((vgetq_lane_u32::<2>(t132[2]) as u64) << 32 | vgetq_lane_u32::<2>(t132[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t133 as *mut u64).add(3), t134).write_unaligned(((vgetq_lane_u32::<3>(t132[2]) as u64) << 32 | vgetq_lane_u32::<3>(t132[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t133 as *mut u64).add(4), t134).write_unaligned(((vgetq_lane_u32::<0>(t132[3]) as u64) << 32 | vgetq_lane_u32::<0>(t132[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t133 as *mut u64).add(5), t134).write_unaligned(((vgetq_lane_u32::<1>(t132[3]) as u64) << 32 | vgetq_lane_u32::<1>(t132[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t133 as *mut u64).add(6), t134).write_unaligned(((vgetq_lane_u32::<2>(t132[3]) as u64) << 32 | vgetq_lane_u32::<2>(t132[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t133 as *mut u64).add(7), t134).write_unaligned(((vgetq_lane_u32::<3>(t132[3]) as u64) << 32 | vgetq_lane_u32::<3>(t132[1]) as u64));
		}
		let t135: u64 = 1u64;
		let t136 = [vdupq_n_u32((t135) as u32); 2];
		let t137: [uint32x4_t; 2] = [vandq_u32(x6[0], t136[0]), vandq_u32(x6[1], t136[1])];
		let t138: [uint32x4_t; 2] = [vtstq_u32(t137[0], t137[0]), vtstq_u32(t137[1], t137[1])];
		let t139: [uint32x4_t; 2] = [vandq_u32(live[0], t138[0]), vandq_u32(live[1], t138[1])];
		{
			let t140: [uint32x4_t; 4] = [veorq_u32(x6[0], x7[0]), veorq_u32(x6[1], x7[1]), veorq_u32(x6[2], x7[2]), veorq_u32(x6[3], x7[3])];
			x11 = [vbslq_u32(t139[0], t140[0], x11[0]), vbslq_u32(t139[1], t140[1], x11[1]), vbslq_u32(t139[0], t140[2], x11[2]), vbslq_u32(t139[1], t140[3], x11[3])];
		}
		let t141: [uint32x4_t; 2] = [vbicq_u32(live[0], t138[0]), vbicq_u32(live[1], t138[1])];
		{
			let t142: [uint32x4_t; 2] = [vaddq_u32(x6[0], x8[0]), vaddq_u32(x6[1], x8[1])];
			let t143: [uint32x4_t; 2] = [vcltq_u32(t142[0], x6[0]), vcltq_u32(t142[1], x6[1])];
			let t144: [uint32x4_t; 4] = [t142[0], t142[1], vadcq_u32(x6[2], vdupq_n_u32(0), t143[0]), vadcq_u32(x6[3], vdupq_n_u32(0), t143[1])];
			x11 = [vbslq_u32(t141[0], t144[0], x11[0]), vbslq_u32(t141[1], t144[1], x11[1]), vbslq_u32(t141[0], t144[2], x11[2]), vbslq_u32(t141[1], t144[3], x11[3])];
		}
		let t145 = (arg3 as *const u8).add(((gid0.wrapping_add((15u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t145 as *mut u32).add(0), uint32x4x2_t(x11[0], x11[2]));
			vst2q_u32((t145 as *mut u32).add(8), uint32x4x2_t(x11[1], x11[3]));
		} else {
			let mut t146w = 0u64;
			let t146 = core::hint::black_box(&raw mut t146w) as *mut u64;
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t145 as *mut u64).add(0), t146).write_unaligned(((vgetq_lane_u32::<0>(x11[2]) as u64) << 32 | vgetq_lane_u32::<0>(x11[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t145 as *mut u64).add(1), t146).write_unaligned(((vgetq_lane_u32::<1>(x11[2]) as u64) << 32 | vgetq_lane_u32::<1>(x11[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t145 as *mut u64).add(2), t146).write_unaligned(((vgetq_lane_u32::<2>(x11[2]) as u64) << 32 | vgetq_lane_u32::<2>(x11[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t145 as *mut u64).add(3), t146).write_unaligned(((vgetq_lane_u32::<3>(x11[2]) as u64) << 32 | vgetq_lane_u32::<3>(x11[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t145 as *mut u64).add(4), t146).write_unaligned(((vgetq_lane_u32::<0>(x11[3]) as u64) << 32 | vgetq_lane_u32::<0>(x11[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t145 as *mut u64).add(5), t146).write_unaligned(((vgetq_lane_u32::<1>(x11[3]) as u64) << 32 | vgetq_lane_u32::<1>(x11[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t145 as *mut u64).add(6), t146).write_unaligned(((vgetq_lane_u32::<2>(x11[3]) as u64) << 32 | vgetq_lane_u32::<2>(x11[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t145 as *mut u64).add(7), t146).write_unaligned(((vgetq_lane_u32::<3>(x11[3]) as u64) << 32 | vgetq_lane_u32::<3>(x11[1]) as u64));
		}
		let t147 = b1_l_p(l, lid0, live, x6);
		let t148 = b2_l_p(l, lid0, live, x7);
		let t149: [uint32x4_t; 4] = [vshlq_n_u32::<8>(t148[0]), vshlq_n_u32::<8>(t148[1]), vshrq_n_u32::<24>(t148[0]), vshrq_n_u32::<24>(t148[1])];
		let t150: [uint32x4_t; 2] = [vaddq_u32(t147[0], t149[0]), vaddq_u32(t147[1], t149[1])];
		let t16: [uint32x4_t; 2] = [vcltq_u32(t150[0], t147[0]), vcltq_u32(t150[1], t147[1])];
		let t17: [uint32x4_t; 4] = [t150[0], t150[1], vadcq_u32(vdupq_n_u32(0), t149[2], t16[0]), vadcq_u32(vdupq_n_u32(0), t149[3], t16[1])];
		let t151 = (arg3 as *const u8).add(((gid0.wrapping_add((16u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t151 as *mut u32).add(0), uint32x4x2_t(t17[0], t17[2]));
			vst2q_u32((t151 as *mut u32).add(8), uint32x4x2_t(t17[1], t17[3]));
		} else {
			let mut t152w = 0u64;
			let t152 = core::hint::black_box(&raw mut t152w) as *mut u64;
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t151 as *mut u64).add(0), t152).write_unaligned(((vgetq_lane_u32::<0>(t17[2]) as u64) << 32 | vgetq_lane_u32::<0>(t17[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t151 as *mut u64).add(1), t152).write_unaligned(((vgetq_lane_u32::<1>(t17[2]) as u64) << 32 | vgetq_lane_u32::<1>(t17[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t151 as *mut u64).add(2), t152).write_unaligned(((vgetq_lane_u32::<2>(t17[2]) as u64) << 32 | vgetq_lane_u32::<2>(t17[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t151 as *mut u64).add(3), t152).write_unaligned(((vgetq_lane_u32::<3>(t17[2]) as u64) << 32 | vgetq_lane_u32::<3>(t17[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t151 as *mut u64).add(4), t152).write_unaligned(((vgetq_lane_u32::<0>(t17[3]) as u64) << 32 | vgetq_lane_u32::<0>(t17[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t151 as *mut u64).add(5), t152).write_unaligned(((vgetq_lane_u32::<1>(t17[3]) as u64) << 32 | vgetq_lane_u32::<1>(t17[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t151 as *mut u64).add(6), t152).write_unaligned(((vgetq_lane_u32::<2>(t17[3]) as u64) << 32 | vgetq_lane_u32::<2>(t17[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t151 as *mut u64).add(7), t152).write_unaligned(((vgetq_lane_u32::<3>(t17[3]) as u64) << 32 | vgetq_lane_u32::<3>(t17[1]) as u64));
		}
		let t153 = [vdupq_n_u32(x10); 2];
		let mut t154x = [0u32; 8];
		let mut t154y = [0u32; 8];
		for r in 0..2 { vst1q_u32(t154x.as_mut_ptr().add(4 * r), x8[r]); vst1q_u32(t154y.as_mut_ptr().add(4 * r), t153[r]); }
		for j in 0..8 { t154x[j] = (t154x[j] as u32).checked_rem(t154y[j] as u32).unwrap_or(0) as u32; }
		let t155: [uint32x4_t; 2] = [vld1q_u32(t154x.as_ptr().add(0)), vld1q_u32(t154x.as_ptr().add(4))];
		let t156 = vdupq_n_u32(maxv_on(&live, &t155));
		let t157: [uint32x4_t; 2] = [vbslq_u32(live[0], t155[0], t156), vbslq_u32(live[1], t155[1], t156)];
		let mut t158 = [vdupq_n_u32(0); 4];
		let t159 = minv(&t157);
		if t159 == maxv(&t157) {
			let t160 = ((arg0 as *const u8).add(t159 as usize * 8) as *const u64).read_unaligned();
			t158 = [vdupq_n_u32(t160 as u32), vdupq_n_u32(t160 as u32), vdupq_n_u32((t160 >> 32) as u32), vdupq_n_u32((t160 >> 32) as u32)];
		} else {
			{ let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t157[0]) as usize * 8) as *const u64).read_unaligned(); t158[0] = vsetq_lane_u32::<0>(v as u32, t158[0]); t158[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t158[2]); }
			{ let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t157[0]) as usize * 8) as *const u64).read_unaligned(); t158[0] = vsetq_lane_u32::<1>(v as u32, t158[0]); t158[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t158[2]); }
			{ let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t157[0]) as usize * 8) as *const u64).read_unaligned(); t158[0] = vsetq_lane_u32::<2>(v as u32, t158[0]); t158[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t158[2]); }
			{ let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t157[0]) as usize * 8) as *const u64).read_unaligned(); t158[0] = vsetq_lane_u32::<3>(v as u32, t158[0]); t158[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t158[2]); }
			{ let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t157[1]) as usize * 8) as *const u64).read_unaligned(); t158[1] = vsetq_lane_u32::<0>(v as u32, t158[1]); t158[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t158[3]); }
			{ let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t157[1]) as usize * 8) as *const u64).read_unaligned(); t158[1] = vsetq_lane_u32::<1>(v as u32, t158[1]); t158[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t158[3]); }
			{ let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t157[1]) as usize * 8) as *const u64).read_unaligned(); t158[1] = vsetq_lane_u32::<2>(v as u32, t158[1]); t158[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t158[3]); }
			{ let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t157[1]) as usize * 8) as *const u64).read_unaligned(); t158[1] = vsetq_lane_u32::<3>(v as u32, t158[1]); t158[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t158[3]); }
		}
		let t161 = (arg3 as *const u8).add(((gid0.wrapping_add((17u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t161 as *mut u32).add(0), uint32x4x2_t(t158[0], t158[2]));
			vst2q_u32((t161 as *mut u32).add(8), uint32x4x2_t(t158[1], t158[3]));
		} else {
			let mut t162w = 0u64;
			let t162 = core::hint::black_box(&raw mut t162w) as *mut u64;
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t161 as *mut u64).add(0), t162).write_unaligned(((vgetq_lane_u32::<0>(t158[2]) as u64) << 32 | vgetq_lane_u32::<0>(t158[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t161 as *mut u64).add(1), t162).write_unaligned(((vgetq_lane_u32::<1>(t158[2]) as u64) << 32 | vgetq_lane_u32::<1>(t158[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t161 as *mut u64).add(2), t162).write_unaligned(((vgetq_lane_u32::<2>(t158[2]) as u64) << 32 | vgetq_lane_u32::<2>(t158[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t161 as *mut u64).add(3), t162).write_unaligned(((vgetq_lane_u32::<3>(t158[2]) as u64) << 32 | vgetq_lane_u32::<3>(t158[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t161 as *mut u64).add(4), t162).write_unaligned(((vgetq_lane_u32::<0>(t158[3]) as u64) << 32 | vgetq_lane_u32::<0>(t158[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t161 as *mut u64).add(5), t162).write_unaligned(((vgetq_lane_u32::<1>(t158[3]) as u64) << 32 | vgetq_lane_u32::<1>(t158[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t161 as *mut u64).add(6), t162).write_unaligned(((vgetq_lane_u32::<2>(t158[3]) as u64) << 32 | vgetq_lane_u32::<2>(t158[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t161 as *mut u64).add(7), t162).write_unaligned(((vgetq_lane_u32::<3>(t158[3]) as u64) << 32 | vgetq_lane_u32::<3>(t158[1]) as u64));
		}
		let t163 = b3_ll_p(l, lid0, live, x6, x8);
		let t164 = (arg3 as *const u8).add(((gid0.wrapping_add((18u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t164 as *mut u32).add(0), uint32x4x2_t(t163[0], t163[2]));
			vst2q_u32((t164 as *mut u32).add(8), uint32x4x2_t(t163[1], t163[3]));
		} else {
			let mut t165w = 0u64;
			let t165 = core::hint::black_box(&raw mut t165w) as *mut u64;
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t164 as *mut u64).add(0), t165).write_unaligned(((vgetq_lane_u32::<0>(t163[2]) as u64) << 32 | vgetq_lane_u32::<0>(t163[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t164 as *mut u64).add(1), t165).write_unaligned(((vgetq_lane_u32::<1>(t163[2]) as u64) << 32 | vgetq_lane_u32::<1>(t163[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t164 as *mut u64).add(2), t165).write_unaligned(((vgetq_lane_u32::<2>(t163[2]) as u64) << 32 | vgetq_lane_u32::<2>(t163[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t164 as *mut u64).add(3), t165).write_unaligned(((vgetq_lane_u32::<3>(t163[2]) as u64) << 32 | vgetq_lane_u32::<3>(t163[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t164 as *mut u64).add(4), t165).write_unaligned(((vgetq_lane_u32::<0>(t163[3]) as u64) << 32 | vgetq_lane_u32::<0>(t163[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t164 as *mut u64).add(5), t165).write_unaligned(((vgetq_lane_u32::<1>(t163[3]) as u64) << 32 | vgetq_lane_u32::<1>(t163[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t164 as *mut u64).add(6), t165).write_unaligned(((vgetq_lane_u32::<2>(t163[3]) as u64) << 32 | vgetq_lane_u32::<2>(t163[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t164 as *mut u64).add(7), t165).write_unaligned(((vgetq_lane_u32::<3>(t163[3]) as u64) << 32 | vgetq_lane_u32::<3>(t163[1]) as u64));
		}
		x12 = x6;
		x13 = 0u32;
		let mut t166 = live;
		loop {
			let t167 = bits(&t166);
			if t167.count_ones() <= 2 {
				let mut t168 = t167;
				while t168 != 0 {
					let j = t168.trailing_zeros() as usize;
					t168 &= t168 - 1;
					let t169 = words4(x7);
					let mut x7_j: u64 = ((t169[8 + j] as u64) << 32 | t169[j] as u64);
					let t170 = words2(x8);
					let mut x8_j: u32 = t170[j];
					let t171 = words4(x12);
					let mut x12_j: u64 = ((t171[8 + j] as u64) << 32 | t171[j] as u64);
					let mut x13_j: u32 = x13;
					loop {
						if !(((x13_j) < (((x8_j) & (7u32))))) { break; }
						x12_j = ((x12_j).wrapping_mul(3u64)).wrapping_add(x7_j);
						let t172 = x13_j;
						x13_j = (x13_j).wrapping_add(1u32);
					}
					let mut t173 = words4(x12);
					t173[j] = x12_j as u32;
					t173[8 + j] = (x12_j as u64 >> 32) as u32;
					x12 = regs4(t173);
				}
				break;
			}
			// Lanes part: each its own rounds.
			let t174 = [vdupq_n_u32(7u32); 2];
			let t175: [uint32x4_t; 2] = [vandq_u32(x8[0], t174[0]), vandq_u32(x8[1], t174[1])];
			let t176 = [vdupq_n_u32(x13); 2];
			let t177: [uint32x4_t; 2] = [vcltq_u32(t176[0], t175[0]), vcltq_u32(t176[1], t175[1])];
			t166 = [vandq_u32(t166[0], t177[0]), vandq_u32(t166[1], t177[1])];
			if !(any(&t166)) { break; }
			let t178: u64 = 3u64;
			let t179 = [vdupq_n_u32((t178) as u32); 2];
			let t180: [uint32x4_t; 2] = [vmulhiq_u32(x12[0], t179[0]), vmulhiq_u32(x12[1], t179[1])];
			let t181: [uint32x4_t; 4] = [vmulq_u32(x12[0], t179[0]), vmulq_u32(x12[1], t179[1]), vmlaq_u32(t180[0], t179[0], x12[2]), vmlaq_u32(t180[1], t179[1], x12[3])];
			let t182: [uint32x4_t; 2] = [vaddq_u32(t181[0], x7[0]), vaddq_u32(t181[1], x7[1])];
			let t183: [uint32x4_t; 2] = [vcltq_u32(t182[0], t181[0]), vcltq_u32(t182[1], t181[1])];
			let t184: [uint32x4_t; 4] = [t182[0], t182[1], vadcq_u32(t181[2], x7[2], t183[0]), vadcq_u32(t181[3], x7[3], t183[1])];
			x12 = [vbslq_u32(t166[0], t184[0], x12[0]), vbslq_u32(t166[1], t184[1], x12[1]), vbslq_u32(t166[0], t184[2], x12[2]), vbslq_u32(t166[1], t184[3], x12[3])];
			let t185: [uint32x4_t; 2] = [vandq_u32(t166[0], live[0]), vandq_u32(t166[1], live[1])];
			let t186 = x13;
			x13 = (x13).wrapping_add(1u32);
		}
		let t187 = (arg3 as *const u8).add(((gid0.wrapping_add((19u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t187 as *mut u32).add(0), uint32x4x2_t(x12[0], x12[2]));
			vst2q_u32((t187 as *mut u32).add(8), uint32x4x2_t(x12[1], x12[3]));
		} else {
			let mut t188w = 0u64;
			let t188 = core::hint::black_box(&raw mut t188w) as *mut u64;
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t187 as *mut u64).add(0), t188).write_unaligned(((vgetq_lane_u32::<0>(x12[2]) as u64) << 32 | vgetq_lane_u32::<0>(x12[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t187 as *mut u64).add(1), t188).write_unaligned(((vgetq_lane_u32::<1>(x12[2]) as u64) << 32 | vgetq_lane_u32::<1>(x12[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t187 as *mut u64).add(2), t188).write_unaligned(((vgetq_lane_u32::<2>(x12[2]) as u64) << 32 | vgetq_lane_u32::<2>(x12[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t187 as *mut u64).add(3), t188).write_unaligned(((vgetq_lane_u32::<3>(x12[2]) as u64) << 32 | vgetq_lane_u32::<3>(x12[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t187 as *mut u64).add(4), t188).write_unaligned(((vgetq_lane_u32::<0>(x12[3]) as u64) << 32 | vgetq_lane_u32::<0>(x12[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t187 as *mut u64).add(5), t188).write_unaligned(((vgetq_lane_u32::<1>(x12[3]) as u64) << 32 | vgetq_lane_u32::<1>(x12[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t187 as *mut u64).add(6), t188).write_unaligned(((vgetq_lane_u32::<2>(x12[3]) as u64) << 32 | vgetq_lane_u32::<2>(x12[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t187 as *mut u64).add(7), t188).write_unaligned(((vgetq_lane_u32::<3>(x12[3]) as u64) << 32 | vgetq_lane_u32::<3>(x12[1]) as u64));
		}
		let t189: u64 = a1;
		let t190 = [vdupq_n_u32((t189) as u32); 2];
		let t191 = [vdupq_n_u32(((t189) >> 32) as u32); 2];
		let t192: [uint32x4_t; 2] = [vaddq_u32(x6[0], t190[0]), vaddq_u32(x6[1], t190[1])];
		let t193: [uint32x4_t; 2] = [vcltq_u32(t192[0], x6[0]), vcltq_u32(t192[1], x6[1])];
		let t194: [uint32x4_t; 4] = [t192[0], t192[1], vadcq_u32(x6[2], t191[0], t193[0]), vadcq_u32(x6[3], t191[1], t193[1])];
		let t195: u64 = (a1).wrapping_shl((3u64) as u32);
		let t196 = [vdupq_n_u32((t195) as u32); 2];
		let t197 = [vdupq_n_u32(((t195) >> 32) as u32); 2];
		let t198: [uint32x4_t; 2] = [vaddq_u32(t194[0], t196[0]), vaddq_u32(t194[1], t196[1])];
		let t199: [uint32x4_t; 2] = [vcltq_u32(t198[0], t194[0]), vcltq_u32(t198[1], t194[1])];
		let t200: [uint32x4_t; 4] = [t198[0], t198[1], vadcq_u32(t194[2], t197[0], t199[0]), vadcq_u32(t194[3], t197[1], t199[1])];
		let t201 = (arg3 as *const u8).add(((gid0.wrapping_add((20u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t201 as *mut u32).add(0), uint32x4x2_t(t200[0], t200[2]));
			vst2q_u32((t201 as *mut u32).add(8), uint32x4x2_t(t200[1], t200[3]));
		} else {
			let mut t202w = 0u64;
			let t202 = core::hint::black_box(&raw mut t202w) as *mut u64;
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t201 as *mut u64).add(0), t202).write_unaligned(((vgetq_lane_u32::<0>(t200[2]) as u64) << 32 | vgetq_lane_u32::<0>(t200[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t201 as *mut u64).add(1), t202).write_unaligned(((vgetq_lane_u32::<1>(t200[2]) as u64) << 32 | vgetq_lane_u32::<1>(t200[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t201 as *mut u64).add(2), t202).write_unaligned(((vgetq_lane_u32::<2>(t200[2]) as u64) << 32 | vgetq_lane_u32::<2>(t200[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t201 as *mut u64).add(3), t202).write_unaligned(((vgetq_lane_u32::<3>(t200[2]) as u64) << 32 | vgetq_lane_u32::<3>(t200[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t201 as *mut u64).add(4), t202).write_unaligned(((vgetq_lane_u32::<0>(t200[3]) as u64) << 32 | vgetq_lane_u32::<0>(t200[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t201 as *mut u64).add(5), t202).write_unaligned(((vgetq_lane_u32::<1>(t200[3]) as u64) << 32 | vgetq_lane_u32::<1>(t200[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t201 as *mut u64).add(6), t202).write_unaligned(((vgetq_lane_u32::<2>(t200[3]) as u64) << 32 | vgetq_lane_u32::<2>(t200[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t201 as *mut u64).add(7), t202).write_unaligned(((vgetq_lane_u32::<3>(t200[3]) as u64) << 32 | vgetq_lane_u32::<3>(t200[1]) as u64));
		}
		let t203: u64 = 4294967295u64;
		let t204 = [vdupq_n_u32((t203) as u32); 2];
		let t205: [uint32x4_t; 2] = [vandq_u32(x6[0], t204[0]), vandq_u32(x6[1], t204[1])];
		let t206: u64 = 4294967295u64;
		let t207 = [vdupq_n_u32((t206) as u32); 2];
		let t208: [uint32x4_t; 2] = [vandq_u32(x7[0], t207[0]), vandq_u32(x7[1], t207[1])];
		let t209: [uint32x4_t; 2] = [vceqq_u32(t205[0], t208[0]), vceqq_u32(t205[1], t208[1])];
		let mut t210 = [vdupq_n_u32(0); 2];
		let t211: [uint32x4_t; 2] = [vandq_u32(live[0], t209[0]), vandq_u32(live[1], t209[1])];
		let t212: [uint32x4_t; 2] = [vbicq_u32(live[0], t209[0]), vbicq_u32(live[1], t209[1])];
		{
			let t213 = [vdupq_n_u32(1u32); 2];
			t210 = [vbslq_u32(t211[0], t213[0], t210[0]), vbslq_u32(t211[1], t213[1], t210[1])];
		}
		{
			let t214 = [vdupq_n_u32(2u32); 2];
			t210 = [vbslq_u32(t212[0], t214[0], t210[0]), vbslq_u32(t212[1], t214[1], t210[1])];
		}
		let t215: [uint32x4_t; 4] = [t210[0], t210[1], vdupq_n_u32(0), vdupq_n_u32(0)];
		let t216 = (arg3 as *const u8).add(((gid0.wrapping_add((21u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t216 as *mut u32).add(0), uint32x4x2_t(t215[0], t215[2]));
			vst2q_u32((t216 as *mut u32).add(8), uint32x4x2_t(t215[1], t215[3]));
		} else {
			let mut t217w = 0u64;
			let t217 = core::hint::black_box(&raw mut t217w) as *mut u64;
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t216 as *mut u64).add(0), t217).write_unaligned(((vgetq_lane_u32::<0>(t215[2]) as u64) << 32 | vgetq_lane_u32::<0>(t215[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t216 as *mut u64).add(1), t217).write_unaligned(((vgetq_lane_u32::<1>(t215[2]) as u64) << 32 | vgetq_lane_u32::<1>(t215[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t216 as *mut u64).add(2), t217).write_unaligned(((vgetq_lane_u32::<2>(t215[2]) as u64) << 32 | vgetq_lane_u32::<2>(t215[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t216 as *mut u64).add(3), t217).write_unaligned(((vgetq_lane_u32::<3>(t215[2]) as u64) << 32 | vgetq_lane_u32::<3>(t215[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t216 as *mut u64).add(4), t217).write_unaligned(((vgetq_lane_u32::<0>(t215[3]) as u64) << 32 | vgetq_lane_u32::<0>(t215[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t216 as *mut u64).add(5), t217).write_unaligned(((vgetq_lane_u32::<1>(t215[3]) as u64) << 32 | vgetq_lane_u32::<1>(t215[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t216 as *mut u64).add(6), t217).write_unaligned(((vgetq_lane_u32::<2>(t215[3]) as u64) << 32 | vgetq_lane_u32::<2>(t215[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t216 as *mut u64).add(7), t217).write_unaligned(((vgetq_lane_u32::<3>(t215[3]) as u64) << 32 | vgetq_lane_u32::<3>(t215[1]) as u64));
		}
		let t218: [uint32x4_t; 2] = [vtstq_u32(vorrq_u32(x6[0], x6[2]), vorrq_u32(x6[0], x6[2])), vtstq_u32(vorrq_u32(x6[1], x6[3]), vorrq_u32(x6[1], x6[3]))];
		let mut t219 = [vdupq_n_u32(0); 2];
		let t220: [uint32x4_t; 2] = [vandq_u32(live[0], t218[0]), vandq_u32(live[1], t218[1])];
		let t221: [uint32x4_t; 2] = [vbicq_u32(live[0], t218[0]), vbicq_u32(live[1], t218[1])];
		{
			let t222 = [vdupq_n_u32(5u32); 2];
			t219 = [vbslq_u32(t220[0], t222[0], t219[0]), vbslq_u32(t220[1], t222[1], t219[1])];
		}
		{
			let t223 = [vdupq_n_u32(6u32); 2];
			t219 = [vbslq_u32(t221[0], t223[0], t219[0]), vbslq_u32(t221[1], t223[1], t219[1])];
		}
		let t224: [uint32x4_t; 4] = [t219[0], t219[1], vdupq_n_u32(0), vdupq_n_u32(0)];
		let t225 = (arg3 as *const u8).add(((gid0.wrapping_add((22u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t225 as *mut u32).add(0), uint32x4x2_t(t224[0], t224[2]));
			vst2q_u32((t225 as *mut u32).add(8), uint32x4x2_t(t224[1], t224[3]));
		} else {
			let mut t226w = 0u64;
			let t226 = core::hint::black_box(&raw mut t226w) as *mut u64;
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t225 as *mut u64).add(0), t226).write_unaligned(((vgetq_lane_u32::<0>(t224[2]) as u64) << 32 | vgetq_lane_u32::<0>(t224[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t225 as *mut u64).add(1), t226).write_unaligned(((vgetq_lane_u32::<1>(t224[2]) as u64) << 32 | vgetq_lane_u32::<1>(t224[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t225 as *mut u64).add(2), t226).write_unaligned(((vgetq_lane_u32::<2>(t224[2]) as u64) << 32 | vgetq_lane_u32::<2>(t224[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t225 as *mut u64).add(3), t226).write_unaligned(((vgetq_lane_u32::<3>(t224[2]) as u64) << 32 | vgetq_lane_u32::<3>(t224[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t225 as *mut u64).add(4), t226).write_unaligned(((vgetq_lane_u32::<0>(t224[3]) as u64) << 32 | vgetq_lane_u32::<0>(t224[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t225 as *mut u64).add(5), t226).write_unaligned(((vgetq_lane_u32::<1>(t224[3]) as u64) << 32 | vgetq_lane_u32::<1>(t224[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t225 as *mut u64).add(6), t226).write_unaligned(((vgetq_lane_u32::<2>(t224[3]) as u64) << 32 | vgetq_lane_u32::<2>(t224[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t225 as *mut u64).add(7), t226).write_unaligned(((vgetq_lane_u32::<3>(t224[3]) as u64) << 32 | vgetq_lane_u32::<3>(t224[1]) as u64));
		}
		let t227 = b4_ll_p(l, lid0, live, x6, x7);
		let t228 = (arg3 as *const u8).add(((gid0.wrapping_add((23u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t228 as *mut u32).add(0), uint32x4x2_t(t227[0], t227[2]));
			vst2q_u32((t228 as *mut u32).add(8), uint32x4x2_t(t227[1], t227[3]));
		} else {
			let mut t229w = 0u64;
			let t229 = core::hint::black_box(&raw mut t229w) as *mut u64;
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t228 as *mut u64).add(0), t229).write_unaligned(((vgetq_lane_u32::<0>(t227[2]) as u64) << 32 | vgetq_lane_u32::<0>(t227[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t228 as *mut u64).add(1), t229).write_unaligned(((vgetq_lane_u32::<1>(t227[2]) as u64) << 32 | vgetq_lane_u32::<1>(t227[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t228 as *mut u64).add(2), t229).write_unaligned(((vgetq_lane_u32::<2>(t227[2]) as u64) << 32 | vgetq_lane_u32::<2>(t227[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t228 as *mut u64).add(3), t229).write_unaligned(((vgetq_lane_u32::<3>(t227[2]) as u64) << 32 | vgetq_lane_u32::<3>(t227[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t228 as *mut u64).add(4), t229).write_unaligned(((vgetq_lane_u32::<0>(t227[3]) as u64) << 32 | vgetq_lane_u32::<0>(t227[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t228 as *mut u64).add(5), t229).write_unaligned(((vgetq_lane_u32::<1>(t227[3]) as u64) << 32 | vgetq_lane_u32::<1>(t227[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t228 as *mut u64).add(6), t229).write_unaligned(((vgetq_lane_u32::<2>(t227[3]) as u64) << 32 | vgetq_lane_u32::<2>(t227[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t228 as *mut u64).add(7), t229).write_unaligned(((vgetq_lane_u32::<3>(t227[3]) as u64) << 32 | vgetq_lane_u32::<3>(t227[1]) as u64));
		}
		let t230: u32;
		if ((((a1) & (1u64))) != (0u64)) {
			t230 = 8u32;
		} else {
			t230 = 4u32;
		}
		x14 = t230;
		let t231 = b5_r0ns_p(l, lid0, live, arg0, gid0.wrapping_mul(x14), x14, 8u32);
		let t232 = (arg3 as *const u8).add(((gid0.wrapping_add((24u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t232 as *mut u32).add(0), uint32x4x2_t(t231[0], t231[2]));
			vst2q_u32((t232 as *mut u32).add(8), uint32x4x2_t(t231[1], t231[3]));
		} else {
			let mut t233w = 0u64;
			let t233 = core::hint::black_box(&raw mut t233w) as *mut u64;
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t232 as *mut u64).add(0), t233).write_unaligned(((vgetq_lane_u32::<0>(t231[2]) as u64) << 32 | vgetq_lane_u32::<0>(t231[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t232 as *mut u64).add(1), t233).write_unaligned(((vgetq_lane_u32::<1>(t231[2]) as u64) << 32 | vgetq_lane_u32::<1>(t231[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t232 as *mut u64).add(2), t233).write_unaligned(((vgetq_lane_u32::<2>(t231[2]) as u64) << 32 | vgetq_lane_u32::<2>(t231[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t232 as *mut u64).add(3), t233).write_unaligned(((vgetq_lane_u32::<3>(t231[2]) as u64) << 32 | vgetq_lane_u32::<3>(t231[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t232 as *mut u64).add(4), t233).write_unaligned(((vgetq_lane_u32::<0>(t231[3]) as u64) << 32 | vgetq_lane_u32::<0>(t231[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t232 as *mut u64).add(5), t233).write_unaligned(((vgetq_lane_u32::<1>(t231[3]) as u64) << 32 | vgetq_lane_u32::<1>(t231[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t232 as *mut u64).add(6), t233).write_unaligned(((vgetq_lane_u32::<2>(t231[3]) as u64) << 32 | vgetq_lane_u32::<2>(t231[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t232 as *mut u64).add(7), t233).write_unaligned(((vgetq_lane_u32::<3>(t231[3]) as u64) << 32 | vgetq_lane_u32::<3>(t231[1]) as u64));
		}
		let t234 = if (gid0.wrapping_add(3u32) & 255u32) <= 248u32 { b6_u8_p(l, lid0, live, gid0.wrapping_add(3u32)) } else { b6_i_p(l, lid0, live, gid0.wrapping_add(3u32)) };
		let t235 = (arg3 as *const u8).add(((gid0.wrapping_add((25u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t235 as *mut u32).add(0), uint32x4x2_t(t234[0], t234[2]));
			vst2q_u32((t235 as *mut u32).add(8), uint32x4x2_t(t234[1], t234[3]));
		} else {
			let mut t236w = 0u64;
			let t236 = core::hint::black_box(&raw mut t236w) as *mut u64;
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t235 as *mut u64).add(0), t236).write_unaligned(((vgetq_lane_u32::<0>(t234[2]) as u64) << 32 | vgetq_lane_u32::<0>(t234[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t235 as *mut u64).add(1), t236).write_unaligned(((vgetq_lane_u32::<1>(t234[2]) as u64) << 32 | vgetq_lane_u32::<1>(t234[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t235 as *mut u64).add(2), t236).write_unaligned(((vgetq_lane_u32::<2>(t234[2]) as u64) << 32 | vgetq_lane_u32::<2>(t234[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t235 as *mut u64).add(3), t236).write_unaligned(((vgetq_lane_u32::<3>(t234[2]) as u64) << 32 | vgetq_lane_u32::<3>(t234[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t235 as *mut u64).add(4), t236).write_unaligned(((vgetq_lane_u32::<0>(t234[3]) as u64) << 32 | vgetq_lane_u32::<0>(t234[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t235 as *mut u64).add(5), t236).write_unaligned(((vgetq_lane_u32::<1>(t234[3]) as u64) << 32 | vgetq_lane_u32::<1>(t234[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t235 as *mut u64).add(6), t236).write_unaligned(((vgetq_lane_u32::<2>(t234[3]) as u64) << 32 | vgetq_lane_u32::<2>(t234[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t235 as *mut u64).add(7), t236).write_unaligned(((vgetq_lane_u32::<3>(t234[3]) as u64) << 32 | vgetq_lane_u32::<3>(t234[1]) as u64));
		}
		let t237: u64 = 3u64;
		let t238 = [vdupq_n_u32((t237) as u32); 2];
		let t239: [uint32x4_t; 2] = [vandq_u32(x6[0], t238[0]), vandq_u32(x6[1], t238[1])];
		let t240: u64 = 0u64;
		let t241 = [vdupq_n_u32((t240) as u32); 2];
		let t242: [uint32x4_t; 2] = [vceqq_u32(t239[0], t241[0]), vceqq_u32(t239[1], t241[1])];
		let mut t243 = [vdupq_n_u32(0); 4];
		let t244: [uint32x4_t; 2] = [vandq_u32(live[0], t242[0]), vandq_u32(live[1], t242[1])];
		let t245: [uint32x4_t; 2] = [vbicq_u32(live[0], t242[0]), vbicq_u32(live[1], t242[1])];
		if any(&t244) {
			let t246: [uint32x4_t; 4];
			let t247 = bits(&t244);
			if t247.count_ones() <= 2 {
				let t248 = words4(x7);
				let t249 = words2(x8);
				let mut t250 = [0u32; 16];
				let mut t251 = t247;
				while t251 != 0 {
					let j = t251.trailing_zeros() as usize;
					t251 &= t251 - 1;
					let t252 = b7_ss_f(l, lid0, ((t248[8 + j] as u64) << 32 | t248[j] as u64), t249[j]);
					let t253: u64 = t252;
					t250[j] = t253 as u32;
					t250[8 + j] = (t253 >> 32) as u32;
				}
				t246 = regs4(t250);
			} else {
				let t254 = b7_ll_p(l, lid0, t244, x7, x8);
				t246 = t254;
			}
			t243 = [vbslq_u32(t244[0], t246[0], t243[0]), vbslq_u32(t244[1], t246[1], t243[1]), vbslq_u32(t244[0], t246[2], t243[2]), vbslq_u32(t244[1], t246[3], t243[3])];
		}
		{
			t243 = [vbslq_u32(t245[0], x6[0], t243[0]), vbslq_u32(t245[1], x6[1], t243[1]), vbslq_u32(t245[0], x6[2], t243[2]), vbslq_u32(t245[1], x6[3], t243[3])];
		}
		let t255 = (arg3 as *const u8).add(((gid0.wrapping_add((26u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t255 as *mut u32).add(0), uint32x4x2_t(t243[0], t243[2]));
			vst2q_u32((t255 as *mut u32).add(8), uint32x4x2_t(t243[1], t243[3]));
		} else {
			let mut t256w = 0u64;
			let t256 = core::hint::black_box(&raw mut t256w) as *mut u64;
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t255 as *mut u64).add(0), t256).write_unaligned(((vgetq_lane_u32::<0>(t243[2]) as u64) << 32 | vgetq_lane_u32::<0>(t243[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t255 as *mut u64).add(1), t256).write_unaligned(((vgetq_lane_u32::<1>(t243[2]) as u64) << 32 | vgetq_lane_u32::<1>(t243[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t255 as *mut u64).add(2), t256).write_unaligned(((vgetq_lane_u32::<2>(t243[2]) as u64) << 32 | vgetq_lane_u32::<2>(t243[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t255 as *mut u64).add(3), t256).write_unaligned(((vgetq_lane_u32::<3>(t243[2]) as u64) << 32 | vgetq_lane_u32::<3>(t243[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t255 as *mut u64).add(4), t256).write_unaligned(((vgetq_lane_u32::<0>(t243[3]) as u64) << 32 | vgetq_lane_u32::<0>(t243[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t255 as *mut u64).add(5), t256).write_unaligned(((vgetq_lane_u32::<1>(t243[3]) as u64) << 32 | vgetq_lane_u32::<1>(t243[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t255 as *mut u64).add(6), t256).write_unaligned(((vgetq_lane_u32::<2>(t243[3]) as u64) << 32 | vgetq_lane_u32::<2>(t243[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t255 as *mut u64).add(7), t256).write_unaligned(((vgetq_lane_u32::<3>(t243[3]) as u64) << 32 | vgetq_lane_u32::<3>(t243[1]) as u64));
		}
		x15 = (((((a1) & (7u64))) as u32)).wrapping_add(1u32);
		let t257 = b5_r0ns_p(l, lid0, live, arg0, gid0.wrapping_mul(x15), x15, x15);
		let t258 = b8_s_p(l, lid0, live, (8u32).wrapping_mul(x15));
		let t259: u64 = t258;
		let t260 = [vdupq_n_u32((t259) as u32); 2];
		let t261 = [vdupq_n_u32(((t259) >> 32) as u32); 2];
		let t262: [uint32x4_t; 4] = [vandq_u32(t257[0], t260[0]), vandq_u32(t257[1], t260[1]), vandq_u32(t257[2], t261[0]), vandq_u32(t257[3], t261[1])];
		let t263 = (arg3 as *const u8).add(((gid0.wrapping_add((27u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t263 as *mut u32).add(0), uint32x4x2_t(t262[0], t262[2]));
			vst2q_u32((t263 as *mut u32).add(8), uint32x4x2_t(t262[1], t262[3]));
		} else {
			let mut t264w = 0u64;
			let t264 = core::hint::black_box(&raw mut t264w) as *mut u64;
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t263 as *mut u64).add(0), t264).write_unaligned(((vgetq_lane_u32::<0>(t262[2]) as u64) << 32 | vgetq_lane_u32::<0>(t262[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t263 as *mut u64).add(1), t264).write_unaligned(((vgetq_lane_u32::<1>(t262[2]) as u64) << 32 | vgetq_lane_u32::<1>(t262[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t263 as *mut u64).add(2), t264).write_unaligned(((vgetq_lane_u32::<2>(t262[2]) as u64) << 32 | vgetq_lane_u32::<2>(t262[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t263 as *mut u64).add(3), t264).write_unaligned(((vgetq_lane_u32::<3>(t262[2]) as u64) << 32 | vgetq_lane_u32::<3>(t262[0]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t263 as *mut u64).add(4), t264).write_unaligned(((vgetq_lane_u32::<0>(t262[3]) as u64) << 32 | vgetq_lane_u32::<0>(t262[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t263 as *mut u64).add(5), t264).write_unaligned(((vgetq_lane_u32::<1>(t262[3]) as u64) << 32 | vgetq_lane_u32::<1>(t262[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t263 as *mut u64).add(6), t264).write_unaligned(((vgetq_lane_u32::<2>(t262[3]) as u64) << 32 | vgetq_lane_u32::<2>(t262[1]) as u64));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t263 as *mut u64).add(7), t264).write_unaligned(((vgetq_lane_u32::<3>(t262[3]) as u64) << 32 | vgetq_lane_u32::<3>(t262[1]) as u64));
		}
	}
}

/// `ukl_popcount64`.
#[inline(always)]
unsafe fn b1_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4]) -> [uint32x4_t; 2] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0: [uint32x4_t; 2] = [vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[0])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[2]))))), vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[1])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[3])))))];
	res = t0;
	return res;
}

/// `ukl_clz64`.
#[inline(always)]
unsafe fn b2_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4]) -> [uint32x4_t; 2] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0: [uint32x4_t; 2] = [vbslq_u32(vceqzq_u32(x0[2]), vaddq_u32(vclzq_u32(x0[0]), vdupq_n_u32(32)), vclzq_u32(x0[2])), vbslq_u32(vceqzq_u32(x0[3]), vaddq_u32(vclzq_u32(x0[1]), vdupq_n_u32(32)), vclzq_u32(x0[3]))];
	res = t0;
	return res;
}

/// `wide_step`.
#[inline(always)]
unsafe fn b3_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], x1: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
	let mut x2 = [vdupq_n_u32(0); 2];
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 4];
	let t0: u64 = 1u64;
	let t1 = [vdupq_n_u32((t0) as u32); 2];
	let t2: [uint32x4_t; 2] = [vandq_u32(x0[0], t1[0]), vandq_u32(x0[1], t1[1])];
	let t3: u64 = 0u64;
	let t4 = [vdupq_n_u32((t3) as u32); 2];
	let t5: [uint32x4_t; 2] = [vceqq_u32(t2[0], t4[0]), vceqq_u32(t2[1], t4[1])];
	let t6: [uint32x4_t; 2] = [vandq_u32(live[0], t5[0]), vandq_u32(live[1], t5[1])];
	{
		let t7: [uint32x4_t; 4] = [x1[0], x1[1], vdupq_n_u32(0), vdupq_n_u32(0)];
		res = [vbslq_u32(t6[0], t7[0], res[0]), vbslq_u32(t6[1], t7[1], res[1]), vbslq_u32(t6[0], t7[2], res[2]), vbslq_u32(t6[1], t7[3], res[3])];
		live = [vbicq_u32(live[0], t6[0]), vbicq_u32(live[1], t6[1])];
	}
	if any(&live) {
		let t8: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x0[0]), vshlq_n_u32::<30>(x0[2])), vorrq_u32(vshrq_n_u32::<2>(x0[1]), vshlq_n_u32::<30>(x0[3])), vshrq_n_u32::<2>(x0[2]), vshrq_n_u32::<2>(x0[3])];
		let t9: [uint32x4_t; 2] = [t8[0], t8[1]];
		x2 = t9;
		let t10 = [vdupq_n_u32(1u32); 2];
		let t11: [uint32x4_t; 2] = [vsubq_u32(x2[0], t10[0]), vsubq_u32(x2[1], t10[1])];
		let t12: [uint32x4_t; 2] = [vcgeq_u32(t11[0], x1[0]), vcgeq_u32(t11[1], x1[1])];
		let t13: [uint32x4_t; 2] = [vandq_u32(live[0], t12[0]), vandq_u32(live[1], t12[1])];
		{
			let t14: u64 = 4294967295u64;
			let t15 = [vdupq_n_u32((t14) as u32); 2];
			let t16: [uint32x4_t; 4] = [t15[0], t15[1], vdupq_n_u32(0), vdupq_n_u32(0)];
			res = [vbslq_u32(t13[0], t16[0], res[0]), vbslq_u32(t13[1], t16[1], res[1]), vbslq_u32(t13[0], t16[2], res[2]), vbslq_u32(t13[1], t16[3], res[3])];
			live = [vbicq_u32(live[0], t13[0]), vbicq_u32(live[1], t13[1])];
		}
		if any(&live) {
			let t17: u64 = 2u64;
			let t18 = [vdupq_n_u32((t17) as u32); 2];
			let t19: [uint32x4_t; 2] = [vandq_u32(x0[0], t18[0]), vandq_u32(x0[1], t18[1])];
			let t20: u64 = 0u64;
			let t21 = [vdupq_n_u32((t20) as u32); 2];
			let t22: [uint32x4_t; 2] = [vceqq_u32(t19[0], t21[0]), vceqq_u32(t19[1], t21[1])];
			let t23: [uint32x4_t; 2] = [vmvnq_u32(t22[0]), vmvnq_u32(t22[1])];
			let t24: [uint32x4_t; 2] = [vandq_u32(live[0], t23[0]), vandq_u32(live[1], t23[1])];
			{
				let t25: [uint32x4_t; 2] = [vsubq_u32(x1[0], x2[0]), vsubq_u32(x1[1], x2[1])];
				let t26: [uint32x4_t; 4] = [t25[0], t25[1], vdupq_n_u32(0), vdupq_n_u32(0)];
				res = [vbslq_u32(t24[0], t26[0], res[0]), vbslq_u32(t24[1], t26[1], res[1]), vbslq_u32(t24[0], t26[2], res[2]), vbslq_u32(t24[1], t26[3], res[3])];
				live = [vbicq_u32(live[0], t24[0]), vbicq_u32(live[1], t24[1])];
			}
			if any(&live) {
				let t27: [uint32x4_t; 2] = [vsubq_u32(x1[0], x2[0]), vsubq_u32(x1[1], x2[1])];
				let t28: u64 = (1u64).wrapping_shl((32u64) as u32);
				let t29 = [vdupq_n_u32((t28) as u32); 2];
				let t30 = [vdupq_n_u32(((t28) >> 32) as u32); 2];
				let t31: [uint32x4_t; 4] = [vorrq_u32(t29[0], t27[0]), vorrq_u32(t29[1], t27[1]), t30[0], t30[1]];
				res = [vbslq_u32(live[0], t31[0], res[0]), vbslq_u32(live[1], t31[1], res[1]), vbslq_u32(live[0], t31[2], res[2]), vbslq_u32(live[1], t31[3], res[3])];
				live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
			}
		}
	}
	res
}

/// `ukl_umulhi64`.
#[inline(always)]
unsafe fn b4_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], x1: [uint32x4_t; 4]) -> [uint32x4_t; 4] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 4];
	let (mut t0l, mut t0h) = ([0u32; 8], [0u32; 8]);
	{ let v = (((((vgetq_lane_u32::<0>(x0[2]) as u64) << 32 | vgetq_lane_u32::<0>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<0>(x1[2]) as u64) << 32 | vgetq_lane_u32::<0>(x1[0]) as u64) as u64) as u128) >> 64) as u64; t0l[0] = v as u32; t0h[0] = (v >> 32) as u32; }
	{ let v = (((((vgetq_lane_u32::<1>(x0[2]) as u64) << 32 | vgetq_lane_u32::<1>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<1>(x1[2]) as u64) << 32 | vgetq_lane_u32::<1>(x1[0]) as u64) as u64) as u128) >> 64) as u64; t0l[1] = v as u32; t0h[1] = (v >> 32) as u32; }
	{ let v = (((((vgetq_lane_u32::<2>(x0[2]) as u64) << 32 | vgetq_lane_u32::<2>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<2>(x1[2]) as u64) << 32 | vgetq_lane_u32::<2>(x1[0]) as u64) as u64) as u128) >> 64) as u64; t0l[2] = v as u32; t0h[2] = (v >> 32) as u32; }
	{ let v = (((((vgetq_lane_u32::<3>(x0[2]) as u64) << 32 | vgetq_lane_u32::<3>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<3>(x1[2]) as u64) << 32 | vgetq_lane_u32::<3>(x1[0]) as u64) as u64) as u128) >> 64) as u64; t0l[3] = v as u32; t0h[3] = (v >> 32) as u32; }
	{ let v = (((((vgetq_lane_u32::<0>(x0[3]) as u64) << 32 | vgetq_lane_u32::<0>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<0>(x1[3]) as u64) << 32 | vgetq_lane_u32::<0>(x1[1]) as u64) as u64) as u128) >> 64) as u64; t0l[4] = v as u32; t0h[4] = (v >> 32) as u32; }
	{ let v = (((((vgetq_lane_u32::<1>(x0[3]) as u64) << 32 | vgetq_lane_u32::<1>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<1>(x1[3]) as u64) << 32 | vgetq_lane_u32::<1>(x1[1]) as u64) as u64) as u128) >> 64) as u64; t0l[5] = v as u32; t0h[5] = (v >> 32) as u32; }
	{ let v = (((((vgetq_lane_u32::<2>(x0[3]) as u64) << 32 | vgetq_lane_u32::<2>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<2>(x1[3]) as u64) << 32 | vgetq_lane_u32::<2>(x1[1]) as u64) as u64) as u128) >> 64) as u64; t0l[6] = v as u32; t0h[6] = (v >> 32) as u32; }
	{ let v = (((((vgetq_lane_u32::<3>(x0[3]) as u64) << 32 | vgetq_lane_u32::<3>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<3>(x1[3]) as u64) << 32 | vgetq_lane_u32::<3>(x1[1]) as u64) as u64) as u128) >> 64) as u64; t0l[7] = v as u32; t0h[7] = (v >> 32) as u32; }
	let t1: [uint32x4_t; 4] = [vld1q_u32(t0l.as_ptr().add(0)), vld1q_u32(t0l.as_ptr().add(4)), vld1q_u32(t0h.as_ptr().add(0)), vld1q_u32(t0h.as_ptr().add(4))];
	res = t1;
	return res;
}

/// `ukl_ld_window`.
#[inline(always)]
unsafe fn b5_r0ns_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1f: u32, x1s: u32, x2: u32) -> [uint32x4_t; 4] {
	let mut x3 = [vdupq_n_u32(0); 4];
	let mut x4 = [vdupq_n_u32(0); 2];
	let mut x5 = [vdupq_n_u32(0); 4];
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 4];
	if ((x2) == (8u32)) {
		let t0: [uint32x4_t; 4];
		if (x1s) == 8u32 && ((x1f) as u64) + 7 * ((x1s) as u64) <= u32::MAX as u64 && all(&live) {
			let t1 = (arg0 as *const u8).add(((x1f) as usize) * 1);
			let t2 = vld2q_u32((t1 as *const u32).add(0));
			let t3 = vld2q_u32((t1 as *const u32).add(8));
			let t4: [uint32x4_t; 4] = [t2.0, t3.0, t2.1, t3.1];
			t0 = t4;
		} else {
			let mut t5 = [vdupq_n_u32(0); 4];
			if ((x1f) as u64) + 7 * ((x1s) as u64) <= u32::MAX as u64 && all(&live) {
				let t6 = (arg0 as *const u8).add(((x1f) as usize) * 1);
				let t7 = ((x1s) as usize) * 1;
				{ let v = (t6.add(0 * t7) as *const u64).read_unaligned(); t5[0] = vsetq_lane_u32::<0>(v as u32, t5[0]); t5[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t5[2]); }
				{ let v = (t6.add(1 * t7) as *const u64).read_unaligned(); t5[0] = vsetq_lane_u32::<1>(v as u32, t5[0]); t5[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t5[2]); }
				{ let v = (t6.add(2 * t7) as *const u64).read_unaligned(); t5[0] = vsetq_lane_u32::<2>(v as u32, t5[0]); t5[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t5[2]); }
				{ let v = (t6.add(3 * t7) as *const u64).read_unaligned(); t5[0] = vsetq_lane_u32::<3>(v as u32, t5[0]); t5[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t5[2]); }
				{ let v = (t6.add(4 * t7) as *const u64).read_unaligned(); t5[1] = vsetq_lane_u32::<0>(v as u32, t5[1]); t5[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t5[3]); }
				{ let v = (t6.add(5 * t7) as *const u64).read_unaligned(); t5[1] = vsetq_lane_u32::<1>(v as u32, t5[1]); t5[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t5[3]); }
				{ let v = (t6.add(6 * t7) as *const u64).read_unaligned(); t5[1] = vsetq_lane_u32::<2>(v as u32, t5[1]); t5[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t5[3]); }
				{ let v = (t6.add(7 * t7) as *const u64).read_unaligned(); t5[1] = vsetq_lane_u32::<3>(v as u32, t5[1]); t5[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t5[3]); }
			} else {
				let t8: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1s)), vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1s))];
				let t9 = vdupq_n_u32(maxv_on(&live, &t8));
				let t10: [uint32x4_t; 2] = [vbslq_u32(live[0], t8[0], t9), vbslq_u32(live[1], t8[1], t9)];
				let mut t11 = [vdupq_n_u32(0); 4];
				let t12 = minv(&t10);
				if t12 == maxv(&t10) {
					let t13 = ((arg0 as *const u8).add(t12 as usize * 1) as *const u64).read_unaligned();
					t11 = [vdupq_n_u32(t13 as u32), vdupq_n_u32(t13 as u32), vdupq_n_u32((t13 >> 32) as u32), vdupq_n_u32((t13 >> 32) as u32)];
				} else {
					{ let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t10[0]) as usize * 1) as *const u64).read_unaligned(); t11[0] = vsetq_lane_u32::<0>(v as u32, t11[0]); t11[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t11[2]); }
					{ let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t10[0]) as usize * 1) as *const u64).read_unaligned(); t11[0] = vsetq_lane_u32::<1>(v as u32, t11[0]); t11[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t11[2]); }
					{ let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t10[0]) as usize * 1) as *const u64).read_unaligned(); t11[0] = vsetq_lane_u32::<2>(v as u32, t11[0]); t11[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t11[2]); }
					{ let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t10[0]) as usize * 1) as *const u64).read_unaligned(); t11[0] = vsetq_lane_u32::<3>(v as u32, t11[0]); t11[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t11[2]); }
					{ let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t10[1]) as usize * 1) as *const u64).read_unaligned(); t11[1] = vsetq_lane_u32::<0>(v as u32, t11[1]); t11[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t11[3]); }
					{ let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t10[1]) as usize * 1) as *const u64).read_unaligned(); t11[1] = vsetq_lane_u32::<1>(v as u32, t11[1]); t11[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t11[3]); }
					{ let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t10[1]) as usize * 1) as *const u64).read_unaligned(); t11[1] = vsetq_lane_u32::<2>(v as u32, t11[1]); t11[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t11[3]); }
					{ let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t10[1]) as usize * 1) as *const u64).read_unaligned(); t11[1] = vsetq_lane_u32::<3>(v as u32, t11[1]); t11[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t11[3]); }
				}
				t5 = t11;
			}
			t0 = t5;
		}
		res = t0;
		return res;
	}
	if ((x2) == (4u32)) {
		let t14: [uint32x4_t; 2];
		if (x1s) == 4u32 && ((x1f) as u64) + 7 * ((x1s) as u64) <= u32::MAX as u64 && all(&live) {
			let t15 = (arg0 as *const u8).add(((x1f) as usize) * 1);
			let t16: [uint32x4_t; 2] = core::array::from_fn(|r| vld1q_u32((t15 as *const u32).add(4 * r)));
			t14 = t16;
		} else {
			let mut t17 = [vdupq_n_u32(0); 2];
			if ((x1f) as u64) + 7 * ((x1s) as u64) <= u32::MAX as u64 && all(&live) {
				let t18 = (arg0 as *const u8).add(((x1f) as usize) * 1);
				let t19 = ((x1s) as usize) * 1;
				t17[0] = vld1q_lane_u32::<0>(t18.add(0 * t19) as *const u32, t17[0]);
				t17[0] = vld1q_lane_u32::<1>(t18.add(1 * t19) as *const u32, t17[0]);
				t17[0] = vld1q_lane_u32::<2>(t18.add(2 * t19) as *const u32, t17[0]);
				t17[0] = vld1q_lane_u32::<3>(t18.add(3 * t19) as *const u32, t17[0]);
				t17[1] = vld1q_lane_u32::<0>(t18.add(4 * t19) as *const u32, t17[1]);
				t17[1] = vld1q_lane_u32::<1>(t18.add(5 * t19) as *const u32, t17[1]);
				t17[1] = vld1q_lane_u32::<2>(t18.add(6 * t19) as *const u32, t17[1]);
				t17[1] = vld1q_lane_u32::<3>(t18.add(7 * t19) as *const u32, t17[1]);
			} else {
				let t20: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1s)), vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1s))];
				let t21 = vdupq_n_u32(maxv_on(&live, &t20));
				let t22: [uint32x4_t; 2] = [vbslq_u32(live[0], t20[0], t21), vbslq_u32(live[1], t20[1], t21)];
				let mut t23 = [vdupq_n_u32(0); 2];
				let t24 = minv(&t22);
				if t24 == maxv(&t22) {
					t23 = [vdupq_n_u32((((arg0 as *const u8).add(t24 as usize * 1) as *const u32).read_unaligned() as u32)); 2];
				} else {
					t23[0] = vsetq_lane_u32::<0>((((arg0 as *const u8).add(vgetq_lane_u32::<0>(t22[0]) as usize * 1) as *const u32).read_unaligned() as u32), t23[0]);
					t23[0] = vsetq_lane_u32::<1>((((arg0 as *const u8).add(vgetq_lane_u32::<1>(t22[0]) as usize * 1) as *const u32).read_unaligned() as u32), t23[0]);
					t23[0] = vsetq_lane_u32::<2>((((arg0 as *const u8).add(vgetq_lane_u32::<2>(t22[0]) as usize * 1) as *const u32).read_unaligned() as u32), t23[0]);
					t23[0] = vsetq_lane_u32::<3>((((arg0 as *const u8).add(vgetq_lane_u32::<3>(t22[0]) as usize * 1) as *const u32).read_unaligned() as u32), t23[0]);
					t23[1] = vsetq_lane_u32::<0>((((arg0 as *const u8).add(vgetq_lane_u32::<0>(t22[1]) as usize * 1) as *const u32).read_unaligned() as u32), t23[1]);
					t23[1] = vsetq_lane_u32::<1>((((arg0 as *const u8).add(vgetq_lane_u32::<1>(t22[1]) as usize * 1) as *const u32).read_unaligned() as u32), t23[1]);
					t23[1] = vsetq_lane_u32::<2>((((arg0 as *const u8).add(vgetq_lane_u32::<2>(t22[1]) as usize * 1) as *const u32).read_unaligned() as u32), t23[1]);
					t23[1] = vsetq_lane_u32::<3>((((arg0 as *const u8).add(vgetq_lane_u32::<3>(t22[1]) as usize * 1) as *const u32).read_unaligned() as u32), t23[1]);
				}
				t17 = t23;
			}
			t14 = t17;
		}
		let t25: [uint32x4_t; 4] = [t14[0], t14[1], vdupq_n_u32(0), vdupq_n_u32(0)];
		res = t25;
		return res;
	}
	if ((x2) == (2u32)) {
		let t26: [uint32x4_t; 2];
		if (x1s) == 2u32 && ((x1f) as u64) + 7 * ((x1s) as u64) <= u32::MAX as u64 && all(&live) {
			let t27 = (arg0 as *const u8).add(((x1f) as usize) * 1);
			let t28: [uint32x4_t; 2] = core::array::from_fn(|r| vmovl_u16(vld1_u16((t27 as *const u16).add(4 * r))));
			t26 = t28;
		} else {
			let mut t29 = [vdupq_n_u32(0); 2];
			if ((x1f) as u64) + 7 * ((x1s) as u64) <= u32::MAX as u64 && all(&live) {
				let t30 = (arg0 as *const u8).add(((x1f) as usize) * 1);
				let t31 = ((x1s) as usize) * 1;
				t29[0] = vsetq_lane_u32::<0>((t30.add(0 * t31) as *const u16).read_unaligned() as u32, t29[0]);
				t29[0] = vsetq_lane_u32::<1>((t30.add(1 * t31) as *const u16).read_unaligned() as u32, t29[0]);
				t29[0] = vsetq_lane_u32::<2>((t30.add(2 * t31) as *const u16).read_unaligned() as u32, t29[0]);
				t29[0] = vsetq_lane_u32::<3>((t30.add(3 * t31) as *const u16).read_unaligned() as u32, t29[0]);
				t29[1] = vsetq_lane_u32::<0>((t30.add(4 * t31) as *const u16).read_unaligned() as u32, t29[1]);
				t29[1] = vsetq_lane_u32::<1>((t30.add(5 * t31) as *const u16).read_unaligned() as u32, t29[1]);
				t29[1] = vsetq_lane_u32::<2>((t30.add(6 * t31) as *const u16).read_unaligned() as u32, t29[1]);
				t29[1] = vsetq_lane_u32::<3>((t30.add(7 * t31) as *const u16).read_unaligned() as u32, t29[1]);
			} else {
				let t32: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1s)), vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1s))];
				let t33 = vdupq_n_u32(maxv_on(&live, &t32));
				let t34: [uint32x4_t; 2] = [vbslq_u32(live[0], t32[0], t33), vbslq_u32(live[1], t32[1], t33)];
				let mut t35 = [vdupq_n_u32(0); 2];
				let t36 = minv(&t34);
				if t36 == maxv(&t34) {
					t35 = [vdupq_n_u32((((arg0 as *const u8).add(t36 as usize * 1) as *const u16).read_unaligned() as u32)); 2];
				} else {
					t35[0] = vsetq_lane_u32::<0>((((arg0 as *const u8).add(vgetq_lane_u32::<0>(t34[0]) as usize * 1) as *const u16).read_unaligned() as u32), t35[0]);
					t35[0] = vsetq_lane_u32::<1>((((arg0 as *const u8).add(vgetq_lane_u32::<1>(t34[0]) as usize * 1) as *const u16).read_unaligned() as u32), t35[0]);
					t35[0] = vsetq_lane_u32::<2>((((arg0 as *const u8).add(vgetq_lane_u32::<2>(t34[0]) as usize * 1) as *const u16).read_unaligned() as u32), t35[0]);
					t35[0] = vsetq_lane_u32::<3>((((arg0 as *const u8).add(vgetq_lane_u32::<3>(t34[0]) as usize * 1) as *const u16).read_unaligned() as u32), t35[0]);
					t35[1] = vsetq_lane_u32::<0>((((arg0 as *const u8).add(vgetq_lane_u32::<0>(t34[1]) as usize * 1) as *const u16).read_unaligned() as u32), t35[1]);
					t35[1] = vsetq_lane_u32::<1>((((arg0 as *const u8).add(vgetq_lane_u32::<1>(t34[1]) as usize * 1) as *const u16).read_unaligned() as u32), t35[1]);
					t35[1] = vsetq_lane_u32::<2>((((arg0 as *const u8).add(vgetq_lane_u32::<2>(t34[1]) as usize * 1) as *const u16).read_unaligned() as u32), t35[1]);
					t35[1] = vsetq_lane_u32::<3>((((arg0 as *const u8).add(vgetq_lane_u32::<3>(t34[1]) as usize * 1) as *const u16).read_unaligned() as u32), t35[1]);
				}
				t29 = t35;
			}
			t26 = t29;
		}
		let t37: [uint32x4_t; 4] = [t26[0], t26[1], vdupq_n_u32(0), vdupq_n_u32(0)];
		res = t37;
		return res;
	}
	if ((x2) == (1u32)) {
		let t38: [uint32x4_t; 2];
		if (x1s) == 1u32 && ((x1f) as u64) + 7 * ((x1s) as u64) <= u32::MAX as u64 && all(&live) {
			let t39 = (arg0 as *const u8).add(((x1f) as usize) * 1);
			let t40 = vmovl_u8(vld1_u8(t39.add(0)));
			let t41: [uint32x4_t; 2] = [vmovl_u16(vget_low_u16(t40)), vmovl_u16(vget_high_u16(t40))];
			t38 = t41;
		} else {
			let mut t42 = [vdupq_n_u32(0); 2];
			if ((x1f) as u64) + 7 * ((x1s) as u64) <= u32::MAX as u64 && all(&live) {
				let t43 = (arg0 as *const u8).add(((x1f) as usize) * 1);
				let t44 = ((x1s) as usize) * 1;
				t42[0] = vsetq_lane_u32::<0>((t43.add(0 * t44) as *const u8).read_unaligned() as u32, t42[0]);
				t42[0] = vsetq_lane_u32::<1>((t43.add(1 * t44) as *const u8).read_unaligned() as u32, t42[0]);
				t42[0] = vsetq_lane_u32::<2>((t43.add(2 * t44) as *const u8).read_unaligned() as u32, t42[0]);
				t42[0] = vsetq_lane_u32::<3>((t43.add(3 * t44) as *const u8).read_unaligned() as u32, t42[0]);
				t42[1] = vsetq_lane_u32::<0>((t43.add(4 * t44) as *const u8).read_unaligned() as u32, t42[1]);
				t42[1] = vsetq_lane_u32::<1>((t43.add(5 * t44) as *const u8).read_unaligned() as u32, t42[1]);
				t42[1] = vsetq_lane_u32::<2>((t43.add(6 * t44) as *const u8).read_unaligned() as u32, t42[1]);
				t42[1] = vsetq_lane_u32::<3>((t43.add(7 * t44) as *const u8).read_unaligned() as u32, t42[1]);
			} else {
				let t45: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1s)), vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1s))];
				let t46 = vdupq_n_u32(maxv_on(&live, &t45));
				let t47: [uint32x4_t; 2] = [vbslq_u32(live[0], t45[0], t46), vbslq_u32(live[1], t45[1], t46)];
				let mut t48 = [vdupq_n_u32(0); 2];
				let t49 = minv(&t47);
				if t49 == maxv(&t47) {
					t48 = [vdupq_n_u32((((arg0 as *const u8).add(t49 as usize * 1) as *const u8).read_unaligned() as u32)); 2];
				} else {
					t48[0] = vsetq_lane_u32::<0>((((arg0 as *const u8).add(vgetq_lane_u32::<0>(t47[0]) as usize * 1) as *const u8).read_unaligned() as u32), t48[0]);
					t48[0] = vsetq_lane_u32::<1>((((arg0 as *const u8).add(vgetq_lane_u32::<1>(t47[0]) as usize * 1) as *const u8).read_unaligned() as u32), t48[0]);
					t48[0] = vsetq_lane_u32::<2>((((arg0 as *const u8).add(vgetq_lane_u32::<2>(t47[0]) as usize * 1) as *const u8).read_unaligned() as u32), t48[0]);
					t48[0] = vsetq_lane_u32::<3>((((arg0 as *const u8).add(vgetq_lane_u32::<3>(t47[0]) as usize * 1) as *const u8).read_unaligned() as u32), t48[0]);
					t48[1] = vsetq_lane_u32::<0>((((arg0 as *const u8).add(vgetq_lane_u32::<0>(t47[1]) as usize * 1) as *const u8).read_unaligned() as u32), t48[1]);
					t48[1] = vsetq_lane_u32::<1>((((arg0 as *const u8).add(vgetq_lane_u32::<1>(t47[1]) as usize * 1) as *const u8).read_unaligned() as u32), t48[1]);
					t48[1] = vsetq_lane_u32::<2>((((arg0 as *const u8).add(vgetq_lane_u32::<2>(t47[1]) as usize * 1) as *const u8).read_unaligned() as u32), t48[1]);
					t48[1] = vsetq_lane_u32::<3>((((arg0 as *const u8).add(vgetq_lane_u32::<3>(t47[1]) as usize * 1) as *const u8).read_unaligned() as u32), t48[1]);
				}
				t42 = t48;
			}
			t38 = t42;
		}
		let t50: [uint32x4_t; 4] = [t38[0], t38[1], vdupq_n_u32(0), vdupq_n_u32(0)];
		res = t50;
		return res;
	}
	let t51: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1s)), vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1s))];
	let t52 = [vdupq_n_u32(!(7u32)); 2];
	let t53: [uint32x4_t; 2] = [vandq_u32(t51[0], t52[0]), vandq_u32(t51[1], t52[1])];
	let t54: [uint32x4_t; 2] = [t53[0], t53[1]];
	let t55: u64 = ((arg0) as usize as u64);
	let t56 = [vdupq_n_u32((t55) as u32); 2];
	let t57 = [vdupq_n_u32(((t55) >> 32) as u32); 2];
	let t58: [uint32x4_t; 2] = [vaddq_u32(t56[0], t54[0]), vaddq_u32(t56[1], t54[1])];
	let t59: [uint32x4_t; 2] = [vcltq_u32(t58[0], t56[0]), vcltq_u32(t58[1], t56[1])];
	let t60: [uint32x4_t; 4] = [t58[0], t58[1], vadcq_u32(t57[0], vdupq_n_u32(0), t59[0]), vadcq_u32(t57[1], vdupq_n_u32(0), t59[1])];
	let t61: [uint32x4_t; 4] = [t60[0], t60[1], t60[2], t60[3]];
	x3 = t61;
	let t62 = words2([x3[0], x3[1]]);
	let t63 = words2([x3[2], x3[3]]);
	let t64 = bits(&live);
	let t65 = t64.trailing_zeros() as usize % 8;
	let t66: [usize; 8] = [{ let j = if t64 >> 0 & 1 != 0 { 0 } else { t65 }; ((t63[j] as u64) << 32 | t62[j] as u64) as usize }, { let j = if t64 >> 1 & 1 != 0 { 1 } else { t65 }; ((t63[j] as u64) << 32 | t62[j] as u64) as usize }, { let j = if t64 >> 2 & 1 != 0 { 2 } else { t65 }; ((t63[j] as u64) << 32 | t62[j] as u64) as usize }, { let j = if t64 >> 3 & 1 != 0 { 3 } else { t65 }; ((t63[j] as u64) << 32 | t62[j] as u64) as usize }, { let j = if t64 >> 4 & 1 != 0 { 4 } else { t65 }; ((t63[j] as u64) << 32 | t62[j] as u64) as usize }, { let j = if t64 >> 5 & 1 != 0 { 5 } else { t65 }; ((t63[j] as u64) << 32 | t62[j] as u64) as usize }, { let j = if t64 >> 6 & 1 != 0 { 6 } else { t65 }; ((t63[j] as u64) << 32 | t62[j] as u64) as usize }, { let j = if t64 >> 7 & 1 != 0 { 7 } else { t65 }; ((t63[j] as u64) << 32 | t62[j] as u64) as usize }];
	let t67: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1s)), vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1s))];
	let t68 = [vdupq_n_u32(7u32); 2];
	let t69: [uint32x4_t; 2] = [vandq_u32(t67[0], t68[0]), vandq_u32(t67[1], t68[1])];
	let t70 = [vdupq_n_u32(8u32); 2];
	let t71: [uint32x4_t; 2] = [vmulq_u32(t69[0], t70[0]), vmulq_u32(t69[1], t70[1])];
	x4 = t71;
	let mut t72 = [vdupq_n_u32(0); 4];
	{ let v = (t66[0] as *const u64).read_unaligned(); t72[0] = vsetq_lane_u32::<0>(v as u32, t72[0]); t72[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t72[2]); }
	{ let v = (t66[1] as *const u64).read_unaligned(); t72[0] = vsetq_lane_u32::<1>(v as u32, t72[0]); t72[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t72[2]); }
	{ let v = (t66[2] as *const u64).read_unaligned(); t72[0] = vsetq_lane_u32::<2>(v as u32, t72[0]); t72[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t72[2]); }
	{ let v = (t66[3] as *const u64).read_unaligned(); t72[0] = vsetq_lane_u32::<3>(v as u32, t72[0]); t72[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t72[2]); }
	{ let v = (t66[4] as *const u64).read_unaligned(); t72[1] = vsetq_lane_u32::<0>(v as u32, t72[1]); t72[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t72[3]); }
	{ let v = (t66[5] as *const u64).read_unaligned(); t72[1] = vsetq_lane_u32::<1>(v as u32, t72[1]); t72[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t72[3]); }
	{ let v = (t66[6] as *const u64).read_unaligned(); t72[1] = vsetq_lane_u32::<2>(v as u32, t72[1]); t72[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t72[3]); }
	{ let v = (t66[7] as *const u64).read_unaligned(); t72[1] = vsetq_lane_u32::<3>(v as u32, t72[1]); t72[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t72[3]); }
	let t73: [uint32x4_t; 2] = [x4[0], x4[1]];
	let t74: [uint32x4_t; 2] = [vandq_u32(t73[0], vdupq_n_u32(63)), vandq_u32(t73[1], vdupq_n_u32(63))];
	let t75: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t72[0], vnegq_s32(vreinterpretq_s32_u32(t74[0]))), vshlq_u32(t72[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t74[0])))), vorrq_u32(vshlq_u32(t72[1], vnegq_s32(vreinterpretq_s32_u32(t74[1]))), vshlq_u32(t72[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t74[1])))), vshlq_u32(t72[2], vnegq_s32(vreinterpretq_s32_u32(t74[0]))), vshlq_u32(t72[3], vnegq_s32(vreinterpretq_s32_u32(t74[1])))];
	x5 = t75;
	let t76 = [vdupq_n_u32(0u32); 2];
	let t77: [uint32x4_t; 2] = [vcneq_u32(x4[0], t76[0]), vcneq_u32(x4[1], t76[1])];
	let t78 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
	let t79: [uint32x4_t; 2] = [vaddq_u32(x4[0], t78[0]), vaddq_u32(x4[1], t78[1])];
	let t80 = [vdupq_n_u32(64u32); 2];
	let t81: [uint32x4_t; 2] = [vcgtq_u32(t79[0], t80[0]), vcgtq_u32(t79[1], t80[1])];
	let t82: [uint32x4_t; 2] = [vandq_u32(t77[0], t81[0]), vandq_u32(t77[1], t81[1])];
	let mut t83 = [vdupq_n_u32(0); 4];
	let t84: [uint32x4_t; 2] = [vandq_u32(live[0], t82[0]), vandq_u32(live[1], t82[1])];
	let t85: [uint32x4_t; 2] = [vbicq_u32(live[0], t82[0]), vbicq_u32(live[1], t82[1])];
	{
		let t86: u64 = (1u64).wrapping_mul(8u64);
		let t87 = [vdupq_n_u32((t86) as u32); 2];
		let t88 = [vdupq_n_u32(((t86) >> 32) as u32); 2];
		let t89: [uint32x4_t; 2] = [vaddq_u32(x3[0], t87[0]), vaddq_u32(x3[1], t87[1])];
		let t90: [uint32x4_t; 2] = [vcltq_u32(t89[0], x3[0]), vcltq_u32(t89[1], x3[1])];
		let t91: [uint32x4_t; 4] = [t89[0], t89[1], vadcq_u32(x3[2], t88[0], t90[0]), vadcq_u32(x3[3], t88[1], t90[1])];
		let t92: [usize; 8] = core::array::from_fn(|j| t66[j].wrapping_add(((1u64).wrapping_mul(8u64)) as u64 as usize));
		let mut t93 = [vdupq_n_u32(0); 4];
		{ let v = (t92[0] as *const u64).read_unaligned(); t93[0] = vsetq_lane_u32::<0>(v as u32, t93[0]); t93[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t93[2]); }
		{ let v = (t92[1] as *const u64).read_unaligned(); t93[0] = vsetq_lane_u32::<1>(v as u32, t93[0]); t93[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t93[2]); }
		{ let v = (t92[2] as *const u64).read_unaligned(); t93[0] = vsetq_lane_u32::<2>(v as u32, t93[0]); t93[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t93[2]); }
		{ let v = (t92[3] as *const u64).read_unaligned(); t93[0] = vsetq_lane_u32::<3>(v as u32, t93[0]); t93[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t93[2]); }
		{ let v = (t92[4] as *const u64).read_unaligned(); t93[1] = vsetq_lane_u32::<0>(v as u32, t93[1]); t93[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t93[3]); }
		{ let v = (t92[5] as *const u64).read_unaligned(); t93[1] = vsetq_lane_u32::<1>(v as u32, t93[1]); t93[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t93[3]); }
		{ let v = (t92[6] as *const u64).read_unaligned(); t93[1] = vsetq_lane_u32::<2>(v as u32, t93[1]); t93[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t93[3]); }
		{ let v = (t92[7] as *const u64).read_unaligned(); t93[1] = vsetq_lane_u32::<3>(v as u32, t93[1]); t93[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t93[3]); }
		let t94: [uint32x4_t; 2] = [vsubq_u32(t80[0], x4[0]), vsubq_u32(t80[1], x4[1])];
		let t95: [uint32x4_t; 2] = [t94[0], t94[1]];
		let t96: [uint32x4_t; 2] = [vandq_u32(t95[0], vdupq_n_u32(63)), vandq_u32(t95[1], vdupq_n_u32(63))];
		let t97: [uint32x4_t; 4] = [vshlq_u32(t93[0], vreinterpretq_s32_u32(t96[0])), vshlq_u32(t93[1], vreinterpretq_s32_u32(t96[1])), vorrq_u32(vshlq_u32(t93[2], vreinterpretq_s32_u32(t96[0])), vshlq_u32(t93[0], vsubq_s32(vreinterpretq_s32_u32(t96[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t93[3], vreinterpretq_s32_u32(t96[1])), vshlq_u32(t93[1], vsubq_s32(vreinterpretq_s32_u32(t96[1]), vdupq_n_s32(32))))];
		let t98: [uint32x4_t; 4] = [vorrq_u32(x5[0], t97[0]), vorrq_u32(x5[1], t97[1]), vorrq_u32(x5[2], t97[2]), vorrq_u32(x5[3], t97[3])];
		t83 = [vbslq_u32(t84[0], t98[0], t83[0]), vbslq_u32(t84[1], t98[1], t83[1]), vbslq_u32(t84[0], t98[2], t83[2]), vbslq_u32(t84[1], t98[3], t83[3])];
	}
	{
		t83 = [vbslq_u32(t85[0], x5[0], t83[0]), vbslq_u32(t85[1], x5[1], t83[1]), vbslq_u32(t85[0], x5[2], t83[2]), vbslq_u32(t85[1], x5[3], t83[3])];
	}
	res = t83;
	return res;
}

/// `wide_part`.
#[inline(always)]
unsafe fn b6_i_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> [uint32x4_t; 4] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 4];
	let t0: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x0), vld1q_u32(LANE[1].as_ptr()))];
	let t1: [uint32x4_t; 2] = [vshlq_u32(t0[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(t0[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
	let t2 = [vdupq_n_u32(255u32); 2];
	let t3: [uint32x4_t; 2] = [vandq_u32(t0[0], t2[0]), vandq_u32(t0[1], t2[1])];
	let t4: [uint32x4_t; 2] = [vmulq_u32(t1[0], vdupq_n_u32(1000)), vmulq_u32(t1[1], vdupq_n_u32(1000))];
	let t5: [uint32x4_t; 2] = [vmulhiq_u32(t1[0], vdupq_n_u32(1000)), vmulhiq_u32(t1[1], vdupq_n_u32(1000))];
	let t6: [uint32x4_t; 2] = [vaddq_u32(t4[0], t3[0]), vaddq_u32(t4[1], t3[1])];
	let t7: [uint32x4_t; 2] = [vcltq_u32(t6[0], t4[0]), vcltq_u32(t6[1], t4[1])];
	let t8: [uint32x4_t; 4] = [t6[0], t6[1], vadcq_u32(t5[0], vdupq_n_u32(0), t7[0]), vadcq_u32(t5[1], vdupq_n_u32(0), t7[1])];
	res = t8;
	return res;
}

/// `wide_part`.
#[inline(always)]
unsafe fn b6_u8_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> [uint32x4_t; 4] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 4];
	let t0: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32((x0 & 255u32)), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32((x0 & 255u32)), vld1q_u32(LANE[1].as_ptr()))];
	let t1: [uint32x4_t; 2] = [t0[0], t0[1]];
	let t2: u64 = ((((((((x0 >> 8)) as u64)).wrapping_mul(1000u64)) as usize as *const u8)) as usize as u64);
	let t3 = [vdupq_n_u32((t2) as u32); 2];
	let t4 = [vdupq_n_u32(((t2) >> 32) as u32); 2];
	let t5: [uint32x4_t; 2] = [vaddq_u32(t3[0], t1[0]), vaddq_u32(t3[1], t1[1])];
	let t6: [uint32x4_t; 2] = [vcltq_u32(t5[0], t3[0]), vcltq_u32(t5[1], t3[1])];
	let t7: [uint32x4_t; 4] = [t5[0], t5[1], vadcq_u32(t4[0], vdupq_n_u32(0), t6[0]), vadcq_u32(t4[1], vdupq_n_u32(0), t6[1])];
	let t8: [uint32x4_t; 4] = [t7[0], t7[1], t7[2], t7[3]];
	res = t8;
	return res;
}

/// `wide_rare`.
#[inline(always)]
unsafe fn b7_ss_f(l: &Launch, lid0: u32, x0: u64, x1: u32) -> u64 {
	let mut x2: u64 = 0;
	let mut x3: u32 = 0;
	let mut live = [vdupq_n_u32(u32::MAX); 2];
	let mut res: u64 = 0;
	x2 = x0;
	x3 = 0u32;
	loop {
		// Rounds of one condition: every lane goes on.
		if !(((x3) < (((x1) & (15u32))))) { break; }
		x2 = ((x2).wrapping_mul(31u64)).wrapping_add(((x3) as u64));
		x3 = (x3).wrapping_add(2u32);
	}
	res = x2;
	return res;
}

/// `wide_rare`.
#[inline(always)]
unsafe fn b7_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], x1: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
	let mut x2 = [vdupq_n_u32(0); 4];
	let mut x3: u32 = 0;
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 4];
	x2 = x0;
	x3 = 0u32;
	let mut t0 = live;
	loop {
		let t1 = bits(&t0);
		if t1.count_ones() <= 2 {
			let mut t2 = t1;
			while t2 != 0 {
				let j = t2.trailing_zeros() as usize;
				t2 &= t2 - 1;
				let t3 = words2(x1);
				let mut x1_j: u32 = t3[j];
				let t4 = words4(x2);
				let mut x2_j: u64 = ((t4[8 + j] as u64) << 32 | t4[j] as u64);
				let mut x3_j: u32 = x3;
				loop {
					if !(((x3_j) < (((x1_j) & (15u32))))) { break; }
					x2_j = ((x2_j).wrapping_mul(31u64)).wrapping_add(((x3_j) as u64));
					x3_j = (x3_j).wrapping_add(2u32);
				}
				let mut t5 = words4(x2);
				t5[j] = x2_j as u32;
				t5[8 + j] = (x2_j as u64 >> 32) as u32;
				x2 = regs4(t5);
			}
			break;
		}
		// Lanes part: each its own rounds.
		let t6 = [vdupq_n_u32(15u32); 2];
		let t7: [uint32x4_t; 2] = [vandq_u32(x1[0], t6[0]), vandq_u32(x1[1], t6[1])];
		let t8 = [vdupq_n_u32(x3); 2];
		let t9: [uint32x4_t; 2] = [vcltq_u32(t8[0], t7[0]), vcltq_u32(t8[1], t7[1])];
		t0 = [vandq_u32(t0[0], t9[0]), vandq_u32(t0[1], t9[1])];
		if !(any(&t0)) { break; }
		let t10: u64 = 31u64;
		let t11 = [vdupq_n_u32((t10) as u32); 2];
		let t12: [uint32x4_t; 2] = [vmulhiq_u32(x2[0], t11[0]), vmulhiq_u32(x2[1], t11[1])];
		let t13: [uint32x4_t; 4] = [vmulq_u32(x2[0], t11[0]), vmulq_u32(x2[1], t11[1]), vmlaq_u32(t12[0], t11[0], x2[2]), vmlaq_u32(t12[1], t11[1], x2[3])];
		let t14: u64 = ((x3) as u64);
		let t15 = [vdupq_n_u32((t14) as u32); 2];
		let t16 = [vdupq_n_u32(((t14) >> 32) as u32); 2];
		let t17: [uint32x4_t; 2] = [vaddq_u32(t13[0], t15[0]), vaddq_u32(t13[1], t15[1])];
		let t18: [uint32x4_t; 2] = [vcltq_u32(t17[0], t13[0]), vcltq_u32(t17[1], t13[1])];
		let t19: [uint32x4_t; 4] = [t17[0], t17[1], vadcq_u32(t13[2], t16[0], t18[0]), vadcq_u32(t13[3], t16[1], t18[1])];
		x2 = [vbslq_u32(t0[0], t19[0], x2[0]), vbslq_u32(t0[1], t19[1], x2[1]), vbslq_u32(t0[0], t19[2], x2[2]), vbslq_u32(t0[1], t19[3], x2[3])];
		let t20: [uint32x4_t; 2] = [vandq_u32(t0[0], live[0]), vandq_u32(t0[1], live[1])];
		x3 = (x3).wrapping_add(2u32);
	}
	res = x2;
	return res;
}

/// `ukl_mask64`.
#[inline(always)]
unsafe fn b8_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
	let mut live = k;
	let mut res: u64 = 0;
	let t0: u64;
	if ((x0) >= (64u32)) {
		t0 = !(0u64);
	} else {
		t0 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
	}
	res = t0;
	return res;
}

/// The group `l.group`'s threads, 8 at a time: `argv[j]` the address of argument `j`'s bytes (a buffer's address,
/// a value's bytes).
///
/// # Safety
///
/// `argv` holds the kernel's arguments, its buffers as large as it reads and writes; `l` is a 1-D group's launch.
pub unsafe fn wide_lanes_w8(argv: *const *const u8, l: &Launch) {
	let n = l.block[0] * l.block[1] * l.block[2];
	let arg0: *const u8 = (*argv.add(0) as *const *const u8).read_unaligned();
	let arg1: *const u8 = (*argv.add(1) as *const *const u8).read_unaligned();
	let arg2: *const u8 = (*argv.add(2) as *const *const u8).read_unaligned();
	let arg3: *const u8 = (*argv.add(3) as *const *const u8).read_unaligned();
	let x4: *const u8 = *argv.add(4);
	let arg5: *const u8 = (*argv.add(5) as *const *const u8).read_unaligned();
	let x6: u32 = (*argv.add(6) as *const u32).read_unaligned();
	let mut t451: bool = !(arg5).is_null();
	if t451 {
		let mut t452: bool = ((l.group[0]) >= ((arg5.wrapping_add((((x6) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
		if !t452 {
			t452 = ((l.group[1]) >= ((arg5.wrapping_add(((((x6).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
		}
		let mut t453: bool = t452;
		if !t453 {
			t453 = ((l.group[2]) >= ((arg5.wrapping_add(((((x6).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
		}
		t451 = t453;
	}
	if t451 {
		return;
	}
	let mut lid0 = 0u32;
	while lid0 + 8 <= n {
		let mut live = [vdupq_n_u32(u32::MAX); 2];
		let gid0 = l.group[0] * l.block[0] + lid0;
		'chunk: {
			b0_r0r1r2r3sg_p(l, lid0, [vdupq_n_u32(u32::MAX); 2], arg0, arg1, arg2, arg3, x4);
		}
		lid0 += 8;
	}
	if lid0 < n {
		// Lanes part: the group's last threads, those under `n`.
		let t454: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
		let mut live = t454;
		let gid0 = l.group[0] * l.block[0] + lid0;
		'chunk: {
			b0_r0r1r2r3sg_p(l, lid0, live, arg0, arg1, arg2, arg3, x4);
		}
	}
}
