//! `mix_lanes` at 8 lanes (2 NEON registers), written by rerail-ukl-vec's Rust (`rs.rs`) from its vector tape: do
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
/// The zero register, and a value in every lane.
const Z: uint32x4_t = unsafe { core::mem::transmute::<[u32; 4], uint32x4_t>([0; 4]) };
#[inline(always)]
unsafe fn splat(x: u32) -> [uint32x4_t; 2] {
	[vdupq_n_u32(x); 2]
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
#[inline]
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
#[inline]
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
#[inline]
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
#[inline]
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
#[inline]
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
#[inline]
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
#[inline]
unsafe fn ldj_u8(mut at: impl FnMut(usize) -> *const u8) -> [uint32x4_t; 2] {
	let mut g = [0u32; 8];
	for j in 0..8 {
		let v = (at(j) as *const u8).read_unaligned();
		g[j] = v as u32;
	}
	regs2(g)
}
#[inline]
unsafe fn stj_u8(k: [uint32x4_t; 2], v: [uint32x4_t; 2], mut at: impl FnMut(usize) -> *mut u8) {
	let (m, x) = (words2(k), words2(v));
	let mut z = 0u64;
	let sink = core::hint::black_box(&raw mut z) as *mut u8;
	for j in 0..8 {
		(core::hint::select_unpredictable(m[j] != 0, at(j), sink) as *mut u8).write_unaligned(x[j] as u8);
	}
}
#[inline]
unsafe fn stjf_u8(v: [uint32x4_t; 2], mut at: impl FnMut(usize) -> *mut u8) {
	let x = words2(v);
	for j in 0..8 {
		(at(j) as *mut u8).write_unaligned(x[j] as u8);
	}
}
#[inline]
unsafe fn ldj_i8(mut at: impl FnMut(usize) -> *const u8) -> [uint32x4_t; 2] {
	let mut g = [0u32; 8];
	for j in 0..8 {
		let v = (at(j) as *const i8).read_unaligned();
		g[j] = v as i32 as u32;
	}
	regs2(g)
}
#[inline]
unsafe fn ldj_u16(mut at: impl FnMut(usize) -> *const u8) -> [uint32x4_t; 2] {
	let mut g = [0u32; 8];
	for j in 0..8 {
		let v = (at(j) as *const u16).read_unaligned();
		g[j] = v as u32;
	}
	regs2(g)
}
#[inline]
unsafe fn stj_u16(k: [uint32x4_t; 2], v: [uint32x4_t; 2], mut at: impl FnMut(usize) -> *mut u8) {
	let (m, x) = (words2(k), words2(v));
	let mut z = 0u64;
	let sink = core::hint::black_box(&raw mut z) as *mut u8;
	for j in 0..8 {
		(core::hint::select_unpredictable(m[j] != 0, at(j), sink) as *mut u16).write_unaligned(x[j] as u16);
	}
}
#[inline]
unsafe fn stjf_u16(v: [uint32x4_t; 2], mut at: impl FnMut(usize) -> *mut u8) {
	let x = words2(v);
	for j in 0..8 {
		(at(j) as *mut u16).write_unaligned(x[j] as u16);
	}
}
#[inline]
unsafe fn ldj_i16(mut at: impl FnMut(usize) -> *const u8) -> [uint32x4_t; 2] {
	let mut g = [0u32; 8];
	for j in 0..8 {
		let v = (at(j) as *const i16).read_unaligned();
		g[j] = v as i32 as u32;
	}
	regs2(g)
}
#[inline]
unsafe fn ldj_u32(mut at: impl FnMut(usize) -> *const u8) -> [uint32x4_t; 2] {
	let mut g = [0u32; 8];
	for j in 0..8 {
		let v = (at(j) as *const u32).read_unaligned();
		g[j] = v as u32;
	}
	regs2(g)
}
#[inline]
unsafe fn stj_u32(k: [uint32x4_t; 2], v: [uint32x4_t; 2], mut at: impl FnMut(usize) -> *mut u8) {
	let (m, x) = (words2(k), words2(v));
	let mut z = 0u64;
	let sink = core::hint::black_box(&raw mut z) as *mut u8;
	for j in 0..8 {
		(core::hint::select_unpredictable(m[j] != 0, at(j), sink) as *mut u32).write_unaligned(x[j] as u32);
	}
}
#[inline]
unsafe fn stjf_u32(v: [uint32x4_t; 2], mut at: impl FnMut(usize) -> *mut u8) {
	let x = words2(v);
	for j in 0..8 {
		(at(j) as *mut u32).write_unaligned(x[j] as u32);
	}
}
#[inline]
unsafe fn ldj_u64(mut at: impl FnMut(usize) -> *const u8) -> [uint32x4_t; 4] {
	let mut g = [0u32; 16];
	for j in 0..8 {
		let v = (at(j) as *const u64).read_unaligned();
		g[j] = v as u32;
		g[8 + j] = (v >> 32) as u32;
	}
	regs4(g)
}
#[inline]
unsafe fn stj_u64(k: [uint32x4_t; 2], v: [uint32x4_t; 4], mut at: impl FnMut(usize) -> *mut u8) {
	let (m, x) = (words2(k), words4(v));
	let mut z = 0u64;
	let sink = core::hint::black_box(&raw mut z) as *mut u8;
	for j in 0..8 {
		(core::hint::select_unpredictable(m[j] != 0, at(j), sink) as *mut u64).write_unaligned((x[8 + j] as u64) << 32 | x[j] as u64);
	}
}
#[inline]
unsafe fn stjf_u64(v: [uint32x4_t; 4], mut at: impl FnMut(usize) -> *mut u8) {
	let x = words4(v);
	for j in 0..8 {
		(at(j) as *mut u64).write_unaligned((x[8 + j] as u64) << 32 | x[j] as u64);
	}
}
/// Where a load reads when no lane of its mask runs (a block entered under a uniform branch none of its lanes
/// takes): zeros, as a store's lanes that are off write to a sink. Its loads' constant offsets lie inside it.
static NO_LANE: [u8; 4096] = [0; 4096];
/// Each lane's address of its halves `lo` and `hi`; a lane mask `k` has off, the first on's (none on: `NO_LANE`).
#[inline(always)]
unsafe fn addrs(lo: [uint32x4_t; 2], hi: [uint32x4_t; 2], k: [uint32x4_t; 2]) -> [usize; 8] {
	let (l, h, b) = (words2(lo), words2(hi), bits(&k));
	if b == 0 {
		return [NO_LANE.as_ptr() as usize; 8];
	}
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

/// `mix_lanes_inner`.
#[inline(always)]
unsafe fn b0_r0r1sg_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: *const u8) {
	let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x2 as *const u32).read_unaligned();
    let a1: u32 = (x2.wrapping_add(4u64 as usize) as *const u32).read_unaligned();
	let mut x4 = [vdupq_n_u32(0); 2];
	let mut x5 = [vdupq_n_u32(0); 2];
	let mut x6 = [vdupq_n_u32(0); 2];
	let mut x7: u32 = 0;
	let mut x8: u32 = 0;
	let mut live = k;
	let t0: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
	let t1 = [vdupq_n_u32(a0); 2];
	let t2: [uint32x4_t; 2] = [vcgeq_u32(t0[0], t1[0]), vcgeq_u32(t0[1], t1[1])];
	let t3: [uint32x4_t; 2] = [vandq_u32(live[0], t2[0]), vandq_u32(live[1], t2[1])];
	{
		live = [vbicq_u32(live[0], t3[0]), vbicq_u32(live[1], t3[1])];
	}
	if any(&live) {
		let mut t4: [uint32x4_t; 2];
		if all(&live) {
			let t5 = (arg0 as *const u32).add((gid0) as usize);
			t4 = [vld1q_u32(t5.add(0)), vld1q_u32(t5.add(4))];
		} else {
			let t6 = vdupq_n_u32(maxv_on(&live, &t0));
			let t7: [uint32x4_t; 2] = [vbslq_u32(live[0], t0[0], t6), vbslq_u32(live[1], t0[1], t6)];
			let t8 = arg0 as *const u32;
			let mut t9 = [vdupq_n_u32(0); 2];
			t9[0] = vld1q_lane_u32::<0>(t8.add(vgetq_lane_u32::<0>(t7[0]) as usize), t9[0]);
			t9[0] = vld1q_lane_u32::<1>(t8.add(vgetq_lane_u32::<1>(t7[0]) as usize), t9[0]);
			t9[0] = vld1q_lane_u32::<2>(t8.add(vgetq_lane_u32::<2>(t7[0]) as usize), t9[0]);
			t9[0] = vld1q_lane_u32::<3>(t8.add(vgetq_lane_u32::<3>(t7[0]) as usize), t9[0]);
			t9[1] = vld1q_lane_u32::<0>(t8.add(vgetq_lane_u32::<0>(t7[1]) as usize), t9[1]);
			t9[1] = vld1q_lane_u32::<1>(t8.add(vgetq_lane_u32::<1>(t7[1]) as usize), t9[1]);
			t9[1] = vld1q_lane_u32::<2>(t8.add(vgetq_lane_u32::<2>(t7[1]) as usize), t9[1]);
			t9[1] = vld1q_lane_u32::<3>(t8.add(vgetq_lane_u32::<3>(t7[1]) as usize), t9[1]);
			t4 = t9;
		}
		x4 = t4;
		let t10 = [vdupq_n_u32(1u32); 2];
		let t11: [uint32x4_t; 2] = [vandq_u32(x4[0], t10[0]), vandq_u32(x4[1], t10[1])];
		let t12: [uint32x4_t; 2] = [vtstq_u32(t11[0], t11[0]), vtstq_u32(t11[1], t11[1])];
		let t13: [uint32x4_t; 2] = [vandq_u32(live[0], t12[0]), vandq_u32(live[1], t12[1])];
		{
			let t14 = [vdupq_n_u32(3u32); 2];
			let t15: [uint32x4_t; 2] = [vmulq_u32(x4[0], t14[0]), vmulq_u32(x4[1], t14[1])];
			let t16: [uint32x4_t; 2] = [vaddq_u32(t15[0], t10[0]), vaddq_u32(t15[1], t10[1])];
			x5 = [vbslq_u32(t13[0], t16[0], x5[0]), vbslq_u32(t13[1], t16[1], x5[1])];
		}
		let t17: [uint32x4_t; 2] = [vbicq_u32(live[0], t12[0]), vbicq_u32(live[1], t12[1])];
		{
			let t18: [uint32x4_t; 2] = [vshlq_u32(x4[0], vnegq_s32(vdupq_n_s32(((1u32) & 31) as i32))), vshlq_u32(x4[1], vnegq_s32(vdupq_n_s32(((1u32) & 31) as i32)))];
			x5 = [vbslq_u32(t17[0], t18[0], x5[0]), vbslq_u32(t17[1], t18[1], x5[1])];
		}
		let t19 = [vdupq_n_u32(4026531840u32); 2];
		let t20: [uint32x4_t; 2] = [vcltq_u32(x4[0], t19[0]), vcltq_u32(x4[1], t19[1])];
		let t21: [uint32x4_t; 2] = [vandq_u32(live[0], t20[0]), vandq_u32(live[1], t20[1])];
		{
			let t22: [uint32x4_t; 2] = [vaddq_u32(x5[0], x4[0]), vaddq_u32(x5[1], x4[1])];
			x5 = [vbslq_u32(t21[0], t22[0], x5[0]), vbslq_u32(t21[1], t22[1], x5[1])];
		}
		let t23 = [vdupq_n_u32(0u32); 2];
		let t24: [uint32x4_t; 2] = [vceqq_u32(x4[0], t23[0]), vceqq_u32(x4[1], t23[1])];
		let t25: [uint32x4_t; 2] = [vandq_u32(live[0], t24[0]), vandq_u32(live[1], t24[1])];
		{
			let t26 = [vdupq_n_u32(57005u32); 2];
			x5 = [vbslq_u32(t25[0], t26[0], x5[0]), vbslq_u32(t25[1], t26[1], x5[1])];
		}
		let t27: [uint32x4_t; 2] = [vbicq_u32(live[0], t24[0]), vbicq_u32(live[1], t24[1])];
		{
			let t28 = [vdupq_n_u32(7u32); 2];
			let t29: [uint32x4_t; 2] = [veorq_u32(x5[0], t28[0]), veorq_u32(x5[1], t28[1])];
			x5 = [vbslq_u32(t27[0], t29[0], x5[0]), vbslq_u32(t27[1], t29[1], x5[1])];
		}
		x6 = t23;
		x7 = 0u32;
		x8 = 0u32;
		let mut t30 = live;
		loop {
			let t31 = bits(&t30);
			if t31.count_ones() <= 2 {
				let mut t32 = t31;
				while t32 != 0 {
					let j = t32.trailing_zeros() as usize;
					t32 &= t32 - 1;
					let t33 = words2(x4);
					let mut x4_j: u32 = t33[j];
					let t34 = words2(x5);
					let mut x5_j: u32 = t34[j];
					let t35 = words2(x6);
					let mut x6_j: u32 = t35[j];
					let mut x8_j: u32 = x8;
					loop {
						if !(((x8_j) < (((x4_j) & (15u32))))) { break; }
						x6_j = (x6_j).wrapping_add((x8_j).wrapping_mul(x5_j));
						let t36 = x8_j;
						x8_j = (x8_j).wrapping_add(1u32);
					}
					let mut t37 = words2(x6);
					t37[j] = x6_j as u32;
					x6 = regs2(t37);
				}
				break;
			}
			// Lanes part: each its own rounds.
			let t38 = [vdupq_n_u32(15u32); 2];
			let t39: [uint32x4_t; 2] = [vandq_u32(x4[0], t38[0]), vandq_u32(x4[1], t38[1])];
			let t40 = [vdupq_n_u32(x8); 2];
			let t41: [uint32x4_t; 2] = [vcltq_u32(t40[0], t39[0]), vcltq_u32(t40[1], t39[1])];
			t30 = [vandq_u32(t30[0], t41[0]), vandq_u32(t30[1], t41[1])];
			if !(any(&t30)) { break; }
			let t42 = [vdupq_n_u32(x8); 2];
			let t43: [uint32x4_t; 2] = [vmulq_u32(t42[0], x5[0]), vmulq_u32(t42[1], x5[1])];
			let t44: [uint32x4_t; 2] = [vaddq_u32(x6[0], t43[0]), vaddq_u32(x6[1], t43[1])];
			x6 = [vbslq_u32(t30[0], t44[0], x6[0]), vbslq_u32(t30[1], t44[1], x6[1])];
			let t45: [uint32x4_t; 2] = [vandq_u32(t30[0], live[0]), vandq_u32(t30[1], live[1])];
			let t46 = x8;
			x8 = (x8).wrapping_add(1u32);
		}
		loop {
			// Rounds of one condition: every lane goes on.
			if !(((x7) < (a1))) { break; }
			let t47 = [vdupq_n_u32(31u32); 2];
			let t48: [uint32x4_t; 2] = [vmulq_u32(x6[0], t47[0]), vmulq_u32(x6[1], t47[1])];
			let t49 = [vdupq_n_u32(x7); 2];
			let t50: [uint32x4_t; 2] = [vaddq_u32(t48[0], t49[0]), vaddq_u32(t48[1], t49[1])];
			x6 = t50;
			let t51 = x7;
			x7 = (x7).wrapping_add(1u32);
		}
		let t52: [uint32x4_t; 2] = [vaddq_u32(x5[0], x6[0]), vaddq_u32(x5[1], x6[1])];
		let t53 = (arg1 as *mut u32).add((gid0) as usize);
		if all(&live) {
			vst1q_u32(t53.add(0), t52[0]);
			vst1q_u32(t53.add(4), t52[1]);
		} else {
			let mut t54w = 0u32;
			let t54 = core::hint::black_box(&raw mut t54w);
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t53.add(0), t54).write(vgetq_lane_u32::<0>(t52[0]));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t53.add(1), t54).write(vgetq_lane_u32::<1>(t52[0]));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t53.add(2), t54).write(vgetq_lane_u32::<2>(t52[0]));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t53.add(3), t54).write(vgetq_lane_u32::<3>(t52[0]));
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t53.add(4), t54).write(vgetq_lane_u32::<0>(t52[1]));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t53.add(5), t54).write(vgetq_lane_u32::<1>(t52[1]));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t53.add(6), t54).write(vgetq_lane_u32::<2>(t52[1]));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t53.add(7), t54).write(vgetq_lane_u32::<3>(t52[1]));
		}
	}
}

/// `mix_lanes_inner`.
#[inline(always)]
unsafe fn b0_r0r1sg_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: *const u8) {
	let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x2 as *const u32).read_unaligned();
    let a1: u32 = (x2.wrapping_add(4u64 as usize) as *const u32).read_unaligned();
	let mut x4 = [vdupq_n_u32(0); 2];
	let mut x5 = [vdupq_n_u32(0); 2];
	let mut x6 = [vdupq_n_u32(0); 2];
	let mut x7: u32 = 0;
	let mut x8: u32 = 0;
	let mut live = [vdupq_n_u32(u32::MAX); 2];
	if ((gid0) as u64 + 7) < ((a0) as u64) {
		let t0 = (arg0 as *const u32).add((gid0) as usize);
		let t1: [uint32x4_t; 2] = [vld1q_u32(t0.add(0)), vld1q_u32(t0.add(4))];
		x4 = t1;
		let t2 = [vdupq_n_u32(1u32); 2];
		let t3: [uint32x4_t; 2] = [vandq_u32(x4[0], t2[0]), vandq_u32(x4[1], t2[1])];
		let t4: [uint32x4_t; 2] = [vtstq_u32(t3[0], t3[0]), vtstq_u32(t3[1], t3[1])];
		{
			let t5 = [vdupq_n_u32(3u32); 2];
			let t6: [uint32x4_t; 2] = [vmulq_u32(x4[0], t5[0]), vmulq_u32(x4[1], t5[1])];
			let t7: [uint32x4_t; 2] = [vaddq_u32(t6[0], t2[0]), vaddq_u32(t6[1], t2[1])];
			x5 = [vbslq_u32(t4[0], t7[0], x5[0]), vbslq_u32(t4[1], t7[1], x5[1])];
		}
		let t8: [uint32x4_t; 2] = [vmvnq_u32(t4[0]), vmvnq_u32(t4[1])];
		{
			let t9: [uint32x4_t; 2] = [vshlq_u32(x4[0], vnegq_s32(vdupq_n_s32(((1u32) & 31) as i32))), vshlq_u32(x4[1], vnegq_s32(vdupq_n_s32(((1u32) & 31) as i32)))];
			x5 = [vbslq_u32(t8[0], t9[0], x5[0]), vbslq_u32(t8[1], t9[1], x5[1])];
		}
		let t10 = [vdupq_n_u32(4026531840u32); 2];
		let t11: [uint32x4_t; 2] = [vcltq_u32(x4[0], t10[0]), vcltq_u32(x4[1], t10[1])];
		if all(&t11) {
			let t12: [uint32x4_t; 2] = [vaddq_u32(x5[0], x4[0]), vaddq_u32(x5[1], x4[1])];
			x5 = t12;
		} else {
			// Lanes part: the chunk's lanes go both ways.
			let t13: [uint32x4_t; 2] = [vandq_u32(live[0], t11[0]), vandq_u32(live[1], t11[1])];
			{
				let t14: [uint32x4_t; 2] = [vaddq_u32(x5[0], x4[0]), vaddq_u32(x5[1], x4[1])];
				x5 = [vbslq_u32(t13[0], t14[0], x5[0]), vbslq_u32(t13[1], t14[1], x5[1])];
			}
		}
		let t15 = [vdupq_n_u32(0u32); 2];
		let t16: [uint32x4_t; 2] = [vceqq_u32(x4[0], t15[0]), vceqq_u32(x4[1], t15[1])];
		if !any(&t16) {
			let t17 = [vdupq_n_u32(7u32); 2];
			let t18: [uint32x4_t; 2] = [veorq_u32(x5[0], t17[0]), veorq_u32(x5[1], t17[1])];
			x5 = t18;
		} else {
			// Lanes part: the chunk's lanes go both ways.
			let t19: [uint32x4_t; 2] = [vandq_u32(live[0], t16[0]), vandq_u32(live[1], t16[1])];
			{
				let t20 = [vdupq_n_u32(57005u32); 2];
				x5 = [vbslq_u32(t19[0], t20[0], x5[0]), vbslq_u32(t19[1], t20[1], x5[1])];
			}
			let t21: [uint32x4_t; 2] = [vbicq_u32(live[0], t16[0]), vbicq_u32(live[1], t16[1])];
			{
				let t22 = [vdupq_n_u32(7u32); 2];
				let t23: [uint32x4_t; 2] = [veorq_u32(x5[0], t22[0]), veorq_u32(x5[1], t22[1])];
				x5 = [vbslq_u32(t21[0], t23[0], x5[0]), vbslq_u32(t21[1], t23[1], x5[1])];
			}
		}
		x6 = t15;
		x7 = 0u32;
		x8 = 0u32;
		loop {
			// Full rounds: every lane goes on.
			let t24 = [vdupq_n_u32(15u32); 2];
			let t25: [uint32x4_t; 2] = [vandq_u32(x4[0], t24[0]), vandq_u32(x4[1], t24[1])];
			let t26 = [vdupq_n_u32(x8); 2];
			let t27: [uint32x4_t; 2] = [vcltq_u32(t26[0], t25[0]), vcltq_u32(t26[1], t25[1])];
			if !all(&t27) { break; }
			let t28 = [vdupq_n_u32(x8); 2];
			let t29: [uint32x4_t; 2] = [vmulq_u32(t28[0], x5[0]), vmulq_u32(t28[1], x5[1])];
			let t30: [uint32x4_t; 2] = [vaddq_u32(x6[0], t29[0]), vaddq_u32(x6[1], t29[1])];
			x6 = t30;
			let t31 = x8;
			x8 = (x8).wrapping_add(1u32);
		}
		let mut t32 = live;
		loop {
			let t33 = bits(&t32);
			if t33.count_ones() <= 2 {
				let mut t34 = t33;
				while t34 != 0 {
					let j = t34.trailing_zeros() as usize;
					t34 &= t34 - 1;
					let t35 = words2(x4);
					let mut x4_j: u32 = t35[j];
					let t36 = words2(x5);
					let mut x5_j: u32 = t36[j];
					let t37 = words2(x6);
					let mut x6_j: u32 = t37[j];
					let mut x8_j: u32 = x8;
					loop {
						if !(((x8_j) < (((x4_j) & (15u32))))) { break; }
						x6_j = (x6_j).wrapping_add((x8_j).wrapping_mul(x5_j));
						let t38 = x8_j;
						x8_j = (x8_j).wrapping_add(1u32);
					}
					let mut t39 = words2(x6);
					t39[j] = x6_j as u32;
					x6 = regs2(t39);
				}
				break;
			}
			// Lanes part: each its own rounds.
			let t40 = [vdupq_n_u32(15u32); 2];
			let t41: [uint32x4_t; 2] = [vandq_u32(x4[0], t40[0]), vandq_u32(x4[1], t40[1])];
			let t42 = [vdupq_n_u32(x8); 2];
			let t43: [uint32x4_t; 2] = [vcltq_u32(t42[0], t41[0]), vcltq_u32(t42[1], t41[1])];
			t32 = [vandq_u32(t32[0], t43[0]), vandq_u32(t32[1], t43[1])];
			if !(any(&t32)) { break; }
			let t44 = [vdupq_n_u32(x8); 2];
			let t45: [uint32x4_t; 2] = [vmulq_u32(t44[0], x5[0]), vmulq_u32(t44[1], x5[1])];
			let t46: [uint32x4_t; 2] = [vaddq_u32(x6[0], t45[0]), vaddq_u32(x6[1], t45[1])];
			x6 = [vbslq_u32(t32[0], t46[0], x6[0]), vbslq_u32(t32[1], t46[1], x6[1])];
			let t47: [uint32x4_t; 2] = [vandq_u32(t32[0], live[0]), vandq_u32(t32[1], live[1])];
			let t48 = x8;
			x8 = (x8).wrapping_add(1u32);
		}
		loop {
			// Rounds of one condition: every lane goes on.
			if !(((x7) < (a1))) { break; }
			let t49 = [vdupq_n_u32(31u32); 2];
			let t50: [uint32x4_t; 2] = [vmulq_u32(x6[0], t49[0]), vmulq_u32(x6[1], t49[1])];
			let t51 = [vdupq_n_u32(x7); 2];
			let t52: [uint32x4_t; 2] = [vaddq_u32(t50[0], t51[0]), vaddq_u32(t50[1], t51[1])];
			x6 = t52;
			let t53 = x7;
			x7 = (x7).wrapping_add(1u32);
		}
		let t54: [uint32x4_t; 2] = [vaddq_u32(x5[0], x6[0]), vaddq_u32(x5[1], x6[1])];
		let t55 = (arg1 as *mut u32).add((gid0) as usize);
		vst1q_u32(t55.add(0), t54[0]);
		vst1q_u32(t55.add(4), t54[1]);
	} else if (((gid0) as u64 + 7) <= u32::MAX as u64 && ((gid0) as u64) >= ((a0) as u64)) {
		return;
	} else {
		// Lanes part: the chunk's lanes on both sides of the bound.
		let t56: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
		let t57 = [vdupq_n_u32(a0); 2];
		let t58: [uint32x4_t; 2] = [vcgeq_u32(t56[0], t57[0]), vcgeq_u32(t56[1], t57[1])];
		let t59: [uint32x4_t; 2] = [vandq_u32(live[0], t58[0]), vandq_u32(live[1], t58[1])];
		{
			live = [vbicq_u32(live[0], t59[0]), vbicq_u32(live[1], t59[1])];
		}
		if any(&live) {
			let mut t60: [uint32x4_t; 2];
			if all(&live) {
				let t61 = (arg0 as *const u32).add((gid0) as usize);
				t60 = [vld1q_u32(t61.add(0)), vld1q_u32(t61.add(4))];
			} else {
				let t62 = vdupq_n_u32(maxv_on(&live, &t56));
				let t63: [uint32x4_t; 2] = [vbslq_u32(live[0], t56[0], t62), vbslq_u32(live[1], t56[1], t62)];
				let t64 = arg0 as *const u32;
				let mut t65 = [vdupq_n_u32(0); 2];
				t65[0] = vld1q_lane_u32::<0>(t64.add(vgetq_lane_u32::<0>(t63[0]) as usize), t65[0]);
				t65[0] = vld1q_lane_u32::<1>(t64.add(vgetq_lane_u32::<1>(t63[0]) as usize), t65[0]);
				t65[0] = vld1q_lane_u32::<2>(t64.add(vgetq_lane_u32::<2>(t63[0]) as usize), t65[0]);
				t65[0] = vld1q_lane_u32::<3>(t64.add(vgetq_lane_u32::<3>(t63[0]) as usize), t65[0]);
				t65[1] = vld1q_lane_u32::<0>(t64.add(vgetq_lane_u32::<0>(t63[1]) as usize), t65[1]);
				t65[1] = vld1q_lane_u32::<1>(t64.add(vgetq_lane_u32::<1>(t63[1]) as usize), t65[1]);
				t65[1] = vld1q_lane_u32::<2>(t64.add(vgetq_lane_u32::<2>(t63[1]) as usize), t65[1]);
				t65[1] = vld1q_lane_u32::<3>(t64.add(vgetq_lane_u32::<3>(t63[1]) as usize), t65[1]);
				t60 = t65;
			}
			x4 = t60;
			let t66 = [vdupq_n_u32(1u32); 2];
			let t67: [uint32x4_t; 2] = [vandq_u32(x4[0], t66[0]), vandq_u32(x4[1], t66[1])];
			let t68: [uint32x4_t; 2] = [vtstq_u32(t67[0], t67[0]), vtstq_u32(t67[1], t67[1])];
			let t69: [uint32x4_t; 2] = [vandq_u32(live[0], t68[0]), vandq_u32(live[1], t68[1])];
			{
				let t70 = [vdupq_n_u32(3u32); 2];
				let t71: [uint32x4_t; 2] = [vmulq_u32(x4[0], t70[0]), vmulq_u32(x4[1], t70[1])];
				let t72: [uint32x4_t; 2] = [vaddq_u32(t71[0], t66[0]), vaddq_u32(t71[1], t66[1])];
				x5 = [vbslq_u32(t69[0], t72[0], x5[0]), vbslq_u32(t69[1], t72[1], x5[1])];
			}
			let t73: [uint32x4_t; 2] = [vbicq_u32(live[0], t68[0]), vbicq_u32(live[1], t68[1])];
			{
				let t74: [uint32x4_t; 2] = [vshlq_u32(x4[0], vnegq_s32(vdupq_n_s32(((1u32) & 31) as i32))), vshlq_u32(x4[1], vnegq_s32(vdupq_n_s32(((1u32) & 31) as i32)))];
				x5 = [vbslq_u32(t73[0], t74[0], x5[0]), vbslq_u32(t73[1], t74[1], x5[1])];
			}
			let t75 = [vdupq_n_u32(4026531840u32); 2];
			let t76: [uint32x4_t; 2] = [vcltq_u32(x4[0], t75[0]), vcltq_u32(x4[1], t75[1])];
			let t77: [uint32x4_t; 2] = [vandq_u32(live[0], t76[0]), vandq_u32(live[1], t76[1])];
			{
				let t78: [uint32x4_t; 2] = [vaddq_u32(x5[0], x4[0]), vaddq_u32(x5[1], x4[1])];
				x5 = [vbslq_u32(t77[0], t78[0], x5[0]), vbslq_u32(t77[1], t78[1], x5[1])];
			}
			let t79 = [vdupq_n_u32(0u32); 2];
			let t80: [uint32x4_t; 2] = [vceqq_u32(x4[0], t79[0]), vceqq_u32(x4[1], t79[1])];
			let t81: [uint32x4_t; 2] = [vandq_u32(live[0], t80[0]), vandq_u32(live[1], t80[1])];
			{
				let t82 = [vdupq_n_u32(57005u32); 2];
				x5 = [vbslq_u32(t81[0], t82[0], x5[0]), vbslq_u32(t81[1], t82[1], x5[1])];
			}
			let t83: [uint32x4_t; 2] = [vbicq_u32(live[0], t80[0]), vbicq_u32(live[1], t80[1])];
			{
				let t84 = [vdupq_n_u32(7u32); 2];
				let t85: [uint32x4_t; 2] = [veorq_u32(x5[0], t84[0]), veorq_u32(x5[1], t84[1])];
				x5 = [vbslq_u32(t83[0], t85[0], x5[0]), vbslq_u32(t83[1], t85[1], x5[1])];
			}
			x6 = t79;
			x7 = 0u32;
			x8 = 0u32;
			let mut t86 = live;
			loop {
				let t87 = bits(&t86);
				if t87.count_ones() <= 2 {
					let mut t88 = t87;
					while t88 != 0 {
						let j = t88.trailing_zeros() as usize;
						t88 &= t88 - 1;
						let t89 = words2(x4);
						let mut x4_j: u32 = t89[j];
						let t90 = words2(x5);
						let mut x5_j: u32 = t90[j];
						let t91 = words2(x6);
						let mut x6_j: u32 = t91[j];
						let mut x8_j: u32 = x8;
						loop {
							if !(((x8_j) < (((x4_j) & (15u32))))) { break; }
							x6_j = (x6_j).wrapping_add((x8_j).wrapping_mul(x5_j));
							let t92 = x8_j;
							x8_j = (x8_j).wrapping_add(1u32);
						}
						let mut t93 = words2(x6);
						t93[j] = x6_j as u32;
						x6 = regs2(t93);
					}
					break;
				}
				// Lanes part: each its own rounds.
				let t94 = [vdupq_n_u32(15u32); 2];
				let t95: [uint32x4_t; 2] = [vandq_u32(x4[0], t94[0]), vandq_u32(x4[1], t94[1])];
				let t96 = [vdupq_n_u32(x8); 2];
				let t97: [uint32x4_t; 2] = [vcltq_u32(t96[0], t95[0]), vcltq_u32(t96[1], t95[1])];
				t86 = [vandq_u32(t86[0], t97[0]), vandq_u32(t86[1], t97[1])];
				if !(any(&t86)) { break; }
				let t98 = [vdupq_n_u32(x8); 2];
				let t99: [uint32x4_t; 2] = [vmulq_u32(t98[0], x5[0]), vmulq_u32(t98[1], x5[1])];
				let t100: [uint32x4_t; 2] = [vaddq_u32(x6[0], t99[0]), vaddq_u32(x6[1], t99[1])];
				x6 = [vbslq_u32(t86[0], t100[0], x6[0]), vbslq_u32(t86[1], t100[1], x6[1])];
				let t101: [uint32x4_t; 2] = [vandq_u32(t86[0], live[0]), vandq_u32(t86[1], live[1])];
				let t102 = x8;
				x8 = (x8).wrapping_add(1u32);
			}
			loop {
				// Rounds of one condition: every lane goes on.
				if !(((x7) < (a1))) { break; }
				let t103 = [vdupq_n_u32(31u32); 2];
				let t104: [uint32x4_t; 2] = [vmulq_u32(x6[0], t103[0]), vmulq_u32(x6[1], t103[1])];
				let t105 = [vdupq_n_u32(x7); 2];
				let t106: [uint32x4_t; 2] = [vaddq_u32(t104[0], t105[0]), vaddq_u32(t104[1], t105[1])];
				x6 = t106;
				let t107 = x7;
				x7 = (x7).wrapping_add(1u32);
			}
			let t108: [uint32x4_t; 2] = [vaddq_u32(x5[0], x6[0]), vaddq_u32(x5[1], x6[1])];
			let t109 = (arg1 as *mut u32).add((gid0) as usize);
			if all(&live) {
				vst1q_u32(t109.add(0), t108[0]);
				vst1q_u32(t109.add(4), t108[1]);
			} else {
				let mut t110w = 0u32;
				let t110 = core::hint::black_box(&raw mut t110w);
				core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t109.add(0), t110).write(vgetq_lane_u32::<0>(t108[0]));
				core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t109.add(1), t110).write(vgetq_lane_u32::<1>(t108[0]));
				core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t109.add(2), t110).write(vgetq_lane_u32::<2>(t108[0]));
				core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t109.add(3), t110).write(vgetq_lane_u32::<3>(t108[0]));
				core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t109.add(4), t110).write(vgetq_lane_u32::<0>(t108[1]));
				core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t109.add(5), t110).write(vgetq_lane_u32::<1>(t108[1]));
				core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t109.add(6), t110).write(vgetq_lane_u32::<2>(t108[1]));
				core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t109.add(7), t110).write(vgetq_lane_u32::<3>(t108[1]));
			}
		}
	}
}

/// The group `l.group`'s threads, 8 at a time: `argv[j]` the address of argument `j`'s bytes (a buffer's address,
/// a value's bytes).
///
/// # Safety
///
/// `argv` holds the kernel's arguments, its buffers as large as it reads and writes; `l` is a 1-D group's launch.
pub unsafe fn mix_lanes_w8(argv: *const *const u8, l: &Launch) {
	let n = l.block[0] * l.block[1] * l.block[2];
	let arg0: *const u8 = (*argv.add(0) as *const *const u8).read_unaligned();
	let arg1: *const u8 = (*argv.add(1) as *const *const u8).read_unaligned();
	let x2: *const u8 = *argv.add(2);
	let arg3: *const u8 = (*argv.add(3) as *const *const u8).read_unaligned();
	let x4: u32 = (*argv.add(4) as *const u32).read_unaligned();
	let mut t185: bool = !(arg3).is_null();
	if t185 {
		let mut t186: bool = ((l.group[0]) >= ((arg3.wrapping_add((((x4) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
		if !t186 {
			t186 = ((l.group[1]) >= ((arg3.wrapping_add(((((x4).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
		}
		let mut t187: bool = t186;
		if !t187 {
			t187 = ((l.group[2]) >= ((arg3.wrapping_add(((((x4).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
		}
		t185 = t187;
	}
	if t185 {
		return;
	}
	let mut lid0 = 0u32;
	while lid0 + 8 <= n {
		let mut live = [vdupq_n_u32(u32::MAX); 2];
		let gid0 = l.group[0] * l.block[0] + lid0;
		'chunk: {
			b0_r0r1sg_f(l, lid0, arg0, arg1, x2);
		}
		lid0 += 8;
	}
	if lid0 < n {
		// Lanes part: the group's last threads, those under `n`.
		let t188: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
		let mut live = t188;
		let gid0 = l.group[0] * l.block[0] + lid0;
		'chunk: {
			b0_r0r1sg_p(l, lid0, live, arg0, arg1, x2);
		}
	}
}
