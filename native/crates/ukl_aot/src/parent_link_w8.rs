//! `parent_link` at 8 lanes (2 NEON registers), written by rerail-ukl-vec's Rust (`rs.rs`) from its vector tape: do
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

/// `parent_link_inner`.
#[inline(always)]
unsafe fn b0_r0r1sg_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: *const u8) {
	let gid0 = l.group[0] * l.block[0] + lid0;
	let mut x4 = [vdupq_n_u32(0); 2];
	let mut x5: u32 = 0;
	let mut x6 = [vdupq_n_u32(0); 2];
	let mut live = k;
	let t0: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
	let t1 = [vdupq_n_u32((x2 as *const u32).read_unaligned()); 2];
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
		x5 = 0u32;
		let mut t10 = live;
		loop {
			let t11 = bits(&t10);
			if t11.count_ones() <= 2 {
				let mut t12 = t11;
				while t12 != 0 {
					let j = t12.trailing_zeros() as usize;
					t12 &= t12 - 1;
					let t13 = words2(x4);
					let mut x4_j: u32 = t13[j];
					let mut x5_j: u32 = x5;
					loop {
						let mut t14: bool = ((x5_j) < (7u32));
						if t14 {
							t14 = ((x4_j) != (4294967295u32));
						}
						let mut t15: bool = t14;
						if t15 {
							t15 = ((((x4_j) & (2147483648u32))) != (0u32));
						}
						if !(t15) { break; }
						x4_j = (arg0.wrapping_add((((((x4_j) & (2147483647u32))) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned();
						let t16 = x5_j;
						x5_j = (x5_j).wrapping_add(1u32);
					}
					let mut t17 = words2(x4);
					t17[j] = x4_j as u32;
					x4 = regs2(t17);
				}
				break;
			}
			// Lanes part: each its own rounds.
			let t18 = [vdupq_n_u32(4294967295u32); 2];
			let t19: [uint32x4_t; 2] = [vcneq_u32(x4[0], t18[0]), vcneq_u32(x4[1], t18[1])];
			let t20: [uint32x4_t; 2] = [vandq_u32(vdupq_n_u32(if ((x5) < (7u32)) { u32::MAX } else { 0 }), t19[0]), vandq_u32(vdupq_n_u32(if ((x5) < (7u32)) { u32::MAX } else { 0 }), t19[1])];
			let t21 = [vdupq_n_u32(2147483648u32); 2];
			let t22: [uint32x4_t; 2] = [vandq_u32(x4[0], t21[0]), vandq_u32(x4[1], t21[1])];
			let t23 = [vdupq_n_u32(0u32); 2];
			let t24: [uint32x4_t; 2] = [vcneq_u32(t22[0], t23[0]), vcneq_u32(t22[1], t23[1])];
			let t25: [uint32x4_t; 2] = [vandq_u32(t20[0], t24[0]), vandq_u32(t20[1], t24[1])];
			t10 = [vandq_u32(t10[0], t25[0]), vandq_u32(t10[1], t25[1])];
			if !(any(&t10)) { break; }
			let t26 = [vdupq_n_u32(2147483647u32); 2];
			let t27: [uint32x4_t; 2] = [vandq_u32(x4[0], t26[0]), vandq_u32(x4[1], t26[1])];
			let t28 = vdupq_n_u32(maxv_on(&t10, &t27));
			let t29: [uint32x4_t; 2] = [vbslq_u32(t10[0], t27[0], t28), vbslq_u32(t10[1], t27[1], t28)];
			let t30 = arg0 as *const u32;
			let mut t31 = [vdupq_n_u32(0); 2];
			t31[0] = vld1q_lane_u32::<0>(t30.add(vgetq_lane_u32::<0>(t29[0]) as usize), t31[0]);
			t31[0] = vld1q_lane_u32::<1>(t30.add(vgetq_lane_u32::<1>(t29[0]) as usize), t31[0]);
			t31[0] = vld1q_lane_u32::<2>(t30.add(vgetq_lane_u32::<2>(t29[0]) as usize), t31[0]);
			t31[0] = vld1q_lane_u32::<3>(t30.add(vgetq_lane_u32::<3>(t29[0]) as usize), t31[0]);
			t31[1] = vld1q_lane_u32::<0>(t30.add(vgetq_lane_u32::<0>(t29[1]) as usize), t31[1]);
			t31[1] = vld1q_lane_u32::<1>(t30.add(vgetq_lane_u32::<1>(t29[1]) as usize), t31[1]);
			t31[1] = vld1q_lane_u32::<2>(t30.add(vgetq_lane_u32::<2>(t29[1]) as usize), t31[1]);
			t31[1] = vld1q_lane_u32::<3>(t30.add(vgetq_lane_u32::<3>(t29[1]) as usize), t31[1]);
			x4 = [vbslq_u32(t10[0], t31[0], x4[0]), vbslq_u32(t10[1], t31[1], x4[1])];
			let t32: [uint32x4_t; 2] = [vandq_u32(t10[0], live[0]), vandq_u32(t10[1], live[1])];
			let t33 = x5;
			x5 = (x5).wrapping_add(1u32);
		}
		let t34 = [vdupq_n_u32(4294967295u32); 2];
		let t35: [uint32x4_t; 2] = [vcneq_u32(x4[0], t34[0]), vcneq_u32(x4[1], t34[1])];
		let t36 = [vdupq_n_u32(2147483648u32); 2];
		let t37: [uint32x4_t; 2] = [vandq_u32(x4[0], t36[0]), vandq_u32(x4[1], t36[1])];
		let t38 = [vdupq_n_u32(0u32); 2];
		let t39: [uint32x4_t; 2] = [vcneq_u32(t37[0], t38[0]), vcneq_u32(t37[1], t38[1])];
		let t40: [uint32x4_t; 2] = [vandq_u32(t35[0], t39[0]), vandq_u32(t35[1], t39[1])];
		x6 = t40;
		let mut t41 = [vdupq_n_u32(0); 2];
		let t42: [uint32x4_t; 2] = [vandq_u32(live[0], x6[0]), vandq_u32(live[1], x6[1])];
		let t43: [uint32x4_t; 2] = [vbicq_u32(live[0], x6[0]), vbicq_u32(live[1], x6[1])];
		{
			t41 = [vbslq_u32(t42[0], t34[0], t41[0]), vbslq_u32(t42[1], t34[1], t41[1])];
		}
		{
			t41 = [vbslq_u32(t43[0], x4[0], t41[0]), vbslq_u32(t43[1], x4[1], t41[1])];
		}
		let t44 = (arg1 as *mut u32).add((gid0) as usize);
		if all(&live) {
			vst1q_u32(t44.add(0), t41[0]);
			vst1q_u32(t44.add(4), t41[1]);
		} else {
			let mut t45w = 0u32;
			let t45 = core::hint::black_box(&raw mut t45w);
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t44.add(0), t45).write(vgetq_lane_u32::<0>(t41[0]));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t44.add(1), t45).write(vgetq_lane_u32::<1>(t41[0]));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t44.add(2), t45).write(vgetq_lane_u32::<2>(t41[0]));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t44.add(3), t45).write(vgetq_lane_u32::<3>(t41[0]));
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t44.add(4), t45).write(vgetq_lane_u32::<0>(t41[1]));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t44.add(5), t45).write(vgetq_lane_u32::<1>(t41[1]));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t44.add(6), t45).write(vgetq_lane_u32::<2>(t41[1]));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t44.add(7), t45).write(vgetq_lane_u32::<3>(t41[1]));
		}
		let t46: [uint32x4_t; 2] = [vandq_u32(live[0], x6[0]), vandq_u32(live[1], x6[1])];
		{
			let t47: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0.wrapping_add(1u32)), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0.wrapping_add(1u32)), vld1q_u32(LANE[1].as_ptr()))];
			let t48 = bits(&t46);
			if t48 != 0 {
			    let j = 31 - t48.leading_zeros() as usize;
			    (arg1.wrapping_add(((((x2 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(words2(t47)[j]);
			}
		}
	}
}

/// `parent_link_inner`.
#[inline(always)]
unsafe fn b0_r0r1sg_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: *const u8) {
	let gid0 = l.group[0] * l.block[0] + lid0;
	let mut x4 = [vdupq_n_u32(0); 2];
	let mut x5: u32 = 0;
	let mut x6 = [vdupq_n_u32(0); 2];
	let mut live = [vdupq_n_u32(u32::MAX); 2];
	if ((gid0) as u64 + 7) < (((x2 as *const u32).read_unaligned()) as u64) {
		let t0 = (arg0 as *const u32).add((gid0) as usize);
		let t1: [uint32x4_t; 2] = [vld1q_u32(t0.add(0)), vld1q_u32(t0.add(4))];
		x4 = t1;
		x5 = 0u32;
		let mut t2 = live;
		loop {
			let t3 = bits(&t2);
			if t3.count_ones() <= 2 {
				let mut t4 = t3;
				while t4 != 0 {
					let j = t4.trailing_zeros() as usize;
					t4 &= t4 - 1;
					let t5 = words2(x4);
					let mut x4_j: u32 = t5[j];
					let mut x5_j: u32 = x5;
					loop {
						let mut t6: bool = ((x5_j) < (7u32));
						if t6 {
							t6 = ((x4_j) != (4294967295u32));
						}
						let mut t7: bool = t6;
						if t7 {
							t7 = ((((x4_j) & (2147483648u32))) != (0u32));
						}
						if !(t7) { break; }
						x4_j = (arg0.wrapping_add((((((x4_j) & (2147483647u32))) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned();
						let t8 = x5_j;
						x5_j = (x5_j).wrapping_add(1u32);
					}
					let mut t9 = words2(x4);
					t9[j] = x4_j as u32;
					x4 = regs2(t9);
				}
				break;
			}
			// Lanes part: each its own rounds.
			let t10 = [vdupq_n_u32(4294967295u32); 2];
			let t11: [uint32x4_t; 2] = [vcneq_u32(x4[0], t10[0]), vcneq_u32(x4[1], t10[1])];
			let t12: [uint32x4_t; 2] = [vandq_u32(vdupq_n_u32(if ((x5) < (7u32)) { u32::MAX } else { 0 }), t11[0]), vandq_u32(vdupq_n_u32(if ((x5) < (7u32)) { u32::MAX } else { 0 }), t11[1])];
			let t13 = [vdupq_n_u32(2147483648u32); 2];
			let t14: [uint32x4_t; 2] = [vandq_u32(x4[0], t13[0]), vandq_u32(x4[1], t13[1])];
			let t15 = [vdupq_n_u32(0u32); 2];
			let t16: [uint32x4_t; 2] = [vcneq_u32(t14[0], t15[0]), vcneq_u32(t14[1], t15[1])];
			let t17: [uint32x4_t; 2] = [vandq_u32(t12[0], t16[0]), vandq_u32(t12[1], t16[1])];
			t2 = [vandq_u32(t2[0], t17[0]), vandq_u32(t2[1], t17[1])];
			if !(any(&t2)) { break; }
			let t18 = [vdupq_n_u32(2147483647u32); 2];
			let t19: [uint32x4_t; 2] = [vandq_u32(x4[0], t18[0]), vandq_u32(x4[1], t18[1])];
			let t20: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
			let t21: [uint32x4_t; 2] = [vbslq_u32(t2[0], t19[0], t20[0]), vbslq_u32(t2[1], t19[1], t20[1])];
			let t22 = arg0 as *const u32;
			let mut t23 = [vdupq_n_u32(0); 2];
			t23[0] = vld1q_lane_u32::<0>(t22.add(vgetq_lane_u32::<0>(t21[0]) as usize), t23[0]);
			t23[0] = vld1q_lane_u32::<1>(t22.add(vgetq_lane_u32::<1>(t21[0]) as usize), t23[0]);
			t23[0] = vld1q_lane_u32::<2>(t22.add(vgetq_lane_u32::<2>(t21[0]) as usize), t23[0]);
			t23[0] = vld1q_lane_u32::<3>(t22.add(vgetq_lane_u32::<3>(t21[0]) as usize), t23[0]);
			t23[1] = vld1q_lane_u32::<0>(t22.add(vgetq_lane_u32::<0>(t21[1]) as usize), t23[1]);
			t23[1] = vld1q_lane_u32::<1>(t22.add(vgetq_lane_u32::<1>(t21[1]) as usize), t23[1]);
			t23[1] = vld1q_lane_u32::<2>(t22.add(vgetq_lane_u32::<2>(t21[1]) as usize), t23[1]);
			t23[1] = vld1q_lane_u32::<3>(t22.add(vgetq_lane_u32::<3>(t21[1]) as usize), t23[1]);
			x4 = [vbslq_u32(t2[0], t23[0], x4[0]), vbslq_u32(t2[1], t23[1], x4[1])];
			let t24: [uint32x4_t; 2] = [vandq_u32(t2[0], live[0]), vandq_u32(t2[1], live[1])];
			let t25 = x5;
			x5 = (x5).wrapping_add(1u32);
		}
		let t26 = [vdupq_n_u32(4294967295u32); 2];
		let t27: [uint32x4_t; 2] = [vcneq_u32(x4[0], t26[0]), vcneq_u32(x4[1], t26[1])];
		let t28 = [vdupq_n_u32(2147483648u32); 2];
		let t29: [uint32x4_t; 2] = [vandq_u32(x4[0], t28[0]), vandq_u32(x4[1], t28[1])];
		let t30 = [vdupq_n_u32(0u32); 2];
		let t31: [uint32x4_t; 2] = [vcneq_u32(t29[0], t30[0]), vcneq_u32(t29[1], t30[1])];
		let t32: [uint32x4_t; 2] = [vandq_u32(t27[0], t31[0]), vandq_u32(t27[1], t31[1])];
		x6 = t32;
		let mut t33 = [vdupq_n_u32(0); 2];
		let t34: [uint32x4_t; 2] = [vmvnq_u32(x6[0]), vmvnq_u32(x6[1])];
		{
			t33 = [vbslq_u32(x6[0], t26[0], t33[0]), vbslq_u32(x6[1], t26[1], t33[1])];
		}
		{
			t33 = [vbslq_u32(t34[0], x4[0], t33[0]), vbslq_u32(t34[1], x4[1], t33[1])];
		}
		let t35 = (arg1 as *mut u32).add((gid0) as usize);
		vst1q_u32(t35.add(0), t33[0]);
		vst1q_u32(t35.add(4), t33[1]);
		{
			let t36: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0.wrapping_add(1u32)), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0.wrapping_add(1u32)), vld1q_u32(LANE[1].as_ptr()))];
			let t37 = bits(&x6);
			if t37 != 0 {
			    let j = 31 - t37.leading_zeros() as usize;
			    (arg1.wrapping_add(((((x2 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(words2(t36)[j]);
			}
		}
	} else if (((gid0) as u64 + 7) <= u32::MAX as u64 && ((gid0) as u64) >= (((x2 as *const u32).read_unaligned()) as u64)) {
		return;
	} else {
		// Lanes part: the chunk's lanes on both sides of the bound.
		let t38: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
		let t39 = [vdupq_n_u32((x2 as *const u32).read_unaligned()); 2];
		let t40: [uint32x4_t; 2] = [vcgeq_u32(t38[0], t39[0]), vcgeq_u32(t38[1], t39[1])];
		let t41: [uint32x4_t; 2] = [vandq_u32(live[0], t40[0]), vandq_u32(live[1], t40[1])];
		{
			live = [vbicq_u32(live[0], t41[0]), vbicq_u32(live[1], t41[1])];
		}
		if any(&live) {
			let mut t42: [uint32x4_t; 2];
			if all(&live) {
				let t43 = (arg0 as *const u32).add((gid0) as usize);
				t42 = [vld1q_u32(t43.add(0)), vld1q_u32(t43.add(4))];
			} else {
				let t44 = vdupq_n_u32(maxv_on(&live, &t38));
				let t45: [uint32x4_t; 2] = [vbslq_u32(live[0], t38[0], t44), vbslq_u32(live[1], t38[1], t44)];
				let t46 = arg0 as *const u32;
				let mut t47 = [vdupq_n_u32(0); 2];
				t47[0] = vld1q_lane_u32::<0>(t46.add(vgetq_lane_u32::<0>(t45[0]) as usize), t47[0]);
				t47[0] = vld1q_lane_u32::<1>(t46.add(vgetq_lane_u32::<1>(t45[0]) as usize), t47[0]);
				t47[0] = vld1q_lane_u32::<2>(t46.add(vgetq_lane_u32::<2>(t45[0]) as usize), t47[0]);
				t47[0] = vld1q_lane_u32::<3>(t46.add(vgetq_lane_u32::<3>(t45[0]) as usize), t47[0]);
				t47[1] = vld1q_lane_u32::<0>(t46.add(vgetq_lane_u32::<0>(t45[1]) as usize), t47[1]);
				t47[1] = vld1q_lane_u32::<1>(t46.add(vgetq_lane_u32::<1>(t45[1]) as usize), t47[1]);
				t47[1] = vld1q_lane_u32::<2>(t46.add(vgetq_lane_u32::<2>(t45[1]) as usize), t47[1]);
				t47[1] = vld1q_lane_u32::<3>(t46.add(vgetq_lane_u32::<3>(t45[1]) as usize), t47[1]);
				t42 = t47;
			}
			x4 = t42;
			x5 = 0u32;
			let mut t48 = live;
			loop {
				let t49 = bits(&t48);
				if t49.count_ones() <= 2 {
					let mut t50 = t49;
					while t50 != 0 {
						let j = t50.trailing_zeros() as usize;
						t50 &= t50 - 1;
						let t51 = words2(x4);
						let mut x4_j: u32 = t51[j];
						let mut x5_j: u32 = x5;
						loop {
							let mut t52: bool = ((x5_j) < (7u32));
							if t52 {
								t52 = ((x4_j) != (4294967295u32));
							}
							let mut t53: bool = t52;
							if t53 {
								t53 = ((((x4_j) & (2147483648u32))) != (0u32));
							}
							if !(t53) { break; }
							x4_j = (arg0.wrapping_add((((((x4_j) & (2147483647u32))) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned();
							let t54 = x5_j;
							x5_j = (x5_j).wrapping_add(1u32);
						}
						let mut t55 = words2(x4);
						t55[j] = x4_j as u32;
						x4 = regs2(t55);
					}
					break;
				}
				// Lanes part: each its own rounds.
				let t56 = [vdupq_n_u32(4294967295u32); 2];
				let t57: [uint32x4_t; 2] = [vcneq_u32(x4[0], t56[0]), vcneq_u32(x4[1], t56[1])];
				let t58: [uint32x4_t; 2] = [vandq_u32(vdupq_n_u32(if ((x5) < (7u32)) { u32::MAX } else { 0 }), t57[0]), vandq_u32(vdupq_n_u32(if ((x5) < (7u32)) { u32::MAX } else { 0 }), t57[1])];
				let t59 = [vdupq_n_u32(2147483648u32); 2];
				let t60: [uint32x4_t; 2] = [vandq_u32(x4[0], t59[0]), vandq_u32(x4[1], t59[1])];
				let t61 = [vdupq_n_u32(0u32); 2];
				let t62: [uint32x4_t; 2] = [vcneq_u32(t60[0], t61[0]), vcneq_u32(t60[1], t61[1])];
				let t63: [uint32x4_t; 2] = [vandq_u32(t58[0], t62[0]), vandq_u32(t58[1], t62[1])];
				t48 = [vandq_u32(t48[0], t63[0]), vandq_u32(t48[1], t63[1])];
				if !(any(&t48)) { break; }
				let t64 = [vdupq_n_u32(2147483647u32); 2];
				let t65: [uint32x4_t; 2] = [vandq_u32(x4[0], t64[0]), vandq_u32(x4[1], t64[1])];
				let t66 = vdupq_n_u32(maxv_on(&t48, &t65));
				let t67: [uint32x4_t; 2] = [vbslq_u32(t48[0], t65[0], t66), vbslq_u32(t48[1], t65[1], t66)];
				let t68 = arg0 as *const u32;
				let mut t69 = [vdupq_n_u32(0); 2];
				t69[0] = vld1q_lane_u32::<0>(t68.add(vgetq_lane_u32::<0>(t67[0]) as usize), t69[0]);
				t69[0] = vld1q_lane_u32::<1>(t68.add(vgetq_lane_u32::<1>(t67[0]) as usize), t69[0]);
				t69[0] = vld1q_lane_u32::<2>(t68.add(vgetq_lane_u32::<2>(t67[0]) as usize), t69[0]);
				t69[0] = vld1q_lane_u32::<3>(t68.add(vgetq_lane_u32::<3>(t67[0]) as usize), t69[0]);
				t69[1] = vld1q_lane_u32::<0>(t68.add(vgetq_lane_u32::<0>(t67[1]) as usize), t69[1]);
				t69[1] = vld1q_lane_u32::<1>(t68.add(vgetq_lane_u32::<1>(t67[1]) as usize), t69[1]);
				t69[1] = vld1q_lane_u32::<2>(t68.add(vgetq_lane_u32::<2>(t67[1]) as usize), t69[1]);
				t69[1] = vld1q_lane_u32::<3>(t68.add(vgetq_lane_u32::<3>(t67[1]) as usize), t69[1]);
				x4 = [vbslq_u32(t48[0], t69[0], x4[0]), vbslq_u32(t48[1], t69[1], x4[1])];
				let t70: [uint32x4_t; 2] = [vandq_u32(t48[0], live[0]), vandq_u32(t48[1], live[1])];
				let t71 = x5;
				x5 = (x5).wrapping_add(1u32);
			}
			let t72 = [vdupq_n_u32(4294967295u32); 2];
			let t73: [uint32x4_t; 2] = [vcneq_u32(x4[0], t72[0]), vcneq_u32(x4[1], t72[1])];
			let t74 = [vdupq_n_u32(2147483648u32); 2];
			let t75: [uint32x4_t; 2] = [vandq_u32(x4[0], t74[0]), vandq_u32(x4[1], t74[1])];
			let t76 = [vdupq_n_u32(0u32); 2];
			let t77: [uint32x4_t; 2] = [vcneq_u32(t75[0], t76[0]), vcneq_u32(t75[1], t76[1])];
			let t78: [uint32x4_t; 2] = [vandq_u32(t73[0], t77[0]), vandq_u32(t73[1], t77[1])];
			x6 = t78;
			let mut t79 = [vdupq_n_u32(0); 2];
			let t80: [uint32x4_t; 2] = [vandq_u32(live[0], x6[0]), vandq_u32(live[1], x6[1])];
			let t81: [uint32x4_t; 2] = [vbicq_u32(live[0], x6[0]), vbicq_u32(live[1], x6[1])];
			{
				t79 = [vbslq_u32(t80[0], t72[0], t79[0]), vbslq_u32(t80[1], t72[1], t79[1])];
			}
			{
				t79 = [vbslq_u32(t81[0], x4[0], t79[0]), vbslq_u32(t81[1], x4[1], t79[1])];
			}
			let t82 = (arg1 as *mut u32).add((gid0) as usize);
			if all(&live) {
				vst1q_u32(t82.add(0), t79[0]);
				vst1q_u32(t82.add(4), t79[1]);
			} else {
				let mut t83w = 0u32;
				let t83 = core::hint::black_box(&raw mut t83w);
				core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t82.add(0), t83).write(vgetq_lane_u32::<0>(t79[0]));
				core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t82.add(1), t83).write(vgetq_lane_u32::<1>(t79[0]));
				core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t82.add(2), t83).write(vgetq_lane_u32::<2>(t79[0]));
				core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t82.add(3), t83).write(vgetq_lane_u32::<3>(t79[0]));
				core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t82.add(4), t83).write(vgetq_lane_u32::<0>(t79[1]));
				core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t82.add(5), t83).write(vgetq_lane_u32::<1>(t79[1]));
				core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t82.add(6), t83).write(vgetq_lane_u32::<2>(t79[1]));
				core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t82.add(7), t83).write(vgetq_lane_u32::<3>(t79[1]));
			}
			let t84: [uint32x4_t; 2] = [vandq_u32(live[0], x6[0]), vandq_u32(live[1], x6[1])];
			{
				let t85: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0.wrapping_add(1u32)), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0.wrapping_add(1u32)), vld1q_u32(LANE[1].as_ptr()))];
				let t86 = bits(&t84);
				if t86 != 0 {
				    let j = 31 - t86.leading_zeros() as usize;
				    (arg1.wrapping_add(((((x2 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(words2(t85)[j]);
				}
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
pub unsafe fn parent_link_w8(argv: *const *const u8, l: &Launch) {
	let n = l.block[0] * l.block[1] * l.block[2];
	let arg0: *const u8 = (*argv.add(0) as *const *const u8).read_unaligned();
	let arg1: *const u8 = (*argv.add(1) as *const *const u8).read_unaligned();
	let x2: *const u8 = *argv.add(2);
	let arg3: *const u8 = (*argv.add(3) as *const *const u8).read_unaligned();
	let x4: u32 = (*argv.add(4) as *const u32).read_unaligned();
	let mut t182: bool = !(arg3).is_null();
	if t182 {
		let mut t183: bool = ((l.group[0]) >= ((arg3.wrapping_add((((x4) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
		if !t183 {
			t183 = ((l.group[1]) >= ((arg3.wrapping_add(((((x4).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
		}
		let mut t184: bool = t183;
		if !t184 {
			t184 = ((l.group[2]) >= ((arg3.wrapping_add(((((x4).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
		}
		t182 = t184;
	}
	if t182 {
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
		let t185: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
		let mut live = t185;
		let gid0 = l.group[0] * l.block[0] + lid0;
		'chunk: {
			b0_r0r1sg_p(l, lid0, live, arg0, arg1, x2);
		}
	}
}
