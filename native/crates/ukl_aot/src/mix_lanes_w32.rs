//! `mix_lanes` at 32 lanes (8 NEON registers), written by rerail-ukl-vec's Rust (`rs.rs`) from its vector tape: do
//! not edit; `UKL_RS_WRITE=1 cargo test -p rerail-ukl-vec _kernels_rust_is` writes it again.

#![allow(unused, unused_mut, unused_parens, unused_unsafe, unreachable_code, non_snake_case, clippy::all, unsafe_op_in_unsafe_fn)]

use crate::Launch;
use core::arch::aarch64::*;
use core::sync::atomic::{AtomicU32, Ordering::Relaxed};

/// Lane `j`'s index in its register's `j`: `[0, 1, 2, 3]`, then `[4, 5, 6, 7]`, ...: a chunk's lanes.
const LANE: [[u32; 4]; 8] = [[0, 1, 2, 3], [4, 5, 6, 7], [8, 9, 10, 11], [12, 13, 14, 15], [16, 17, 18, 19], [20, 21, 22, 23], [24, 25, 26, 27], [28, 29, 30, 31]];

/// Whether a lane of mask `k` is on: a comparison of its bits, which the compiler folds where it knows `k`.
#[inline(always)]
unsafe fn any(k: &[uint32x4_t; 8]) -> bool {
    let mut x = k[0];
    for r in 1..8 {
        x = vorrq_u32(x, k[r]);
    }
    core::mem::transmute::<uint32x4_t, u128>(x) != 0
}

/// Whether every lane of mask `k` is on, the same way.
#[inline(always)]
unsafe fn all(k: &[uint32x4_t; 8]) -> bool {
    let mut x = k[0];
    for r in 1..8 {
        x = vandq_u32(x, k[r]);
    }
    core::mem::transmute::<uint32x4_t, u128>(x) == u128::MAX
}

/// Mask `k`'s lanes as bits, lane `j` bit `j`.
#[inline(always)]
unsafe fn bits(k: &[uint32x4_t; 8]) -> u32 {
    let w = vld1q_u32([1u32, 2, 4, 8].as_ptr());
    let mut b = 0u32;
    for r in 0..8 {
        b |= vaddvq_u32(vandq_u32(k[r], w)) << (4 * r);
    }
    b
}

/// The largest and the least of the lanes of `a`; the largest of those mask `k` has on (0 for one off).
#[inline(always)]
unsafe fn maxv(a: &[uint32x4_t; 8]) -> u32 {
    let mut m = a[0];
    for r in 1..8 {
        m = vmaxq_u32(m, a[r]);
    }
    vmaxvq_u32(m)
}
#[inline(always)]
unsafe fn minv(a: &[uint32x4_t; 8]) -> u32 {
    let mut m = a[0];
    for r in 1..8 {
        m = vminq_u32(m, a[r]);
    }
    vminvq_u32(m)
}
#[inline(always)]
unsafe fn maxv_on(k: &[uint32x4_t; 8], a: &[uint32x4_t; 8]) -> u32 {
    let mut m = vandq_u32(k[0], a[0]);
    for r in 1..8 {
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
unsafe fn splat(x: u32) -> [uint32x4_t; 8] {
	[vdupq_n_u32(x); 8]
}
/// A 64-bit value in every lane: its halves.
#[inline(always)]
unsafe fn splat64(v: u64) -> [uint32x4_t; 16] {
	core::array::from_fn(|r| vdupq_n_u32(if r < 8 { v as u32 } else { (v >> 32) as u32 }))
}
/// Lanes `f + j`.
#[inline(always)]
unsafe fn iota(f: u32) -> [uint32x4_t; 8] {
	core::array::from_fn(|r| vaddq_u32(vdupq_n_u32(f), vld1q_u32(LANE[r].as_ptr())))
}
/// An array's first registers, and those from its 8th (a 64-bit value's halves).
#[inline(always)]
unsafe fn lo<const N: usize, const M: usize>(a: [uint32x4_t; M]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| a[r])
}
#[inline(always)]
unsafe fn hi<const N: usize, const M: usize>(a: [uint32x4_t; M]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| a[r + 8])
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
unsafe fn ix_on(k: [uint32x4_t; 8], a: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let s = vdupq_n_u32(maxv_on(&k, &a));
	core::array::from_fn(|r| vbslq_u32(k[r], a[r], s))
}
#[inline]
unsafe fn ld_u8(b: *const u8, ix: [uint32x4_t; 8], s: usize) -> [uint32x4_t; 8] {
	let u = minv(&ix);
	if u == maxv(&ix) {
		return [vdupq_n_u32((b.add(u as usize * s) as *const u8).read_unaligned() as u32); 8];
	}
	let a = words8(ix);
	let mut g = [0u32; 32];
	for j in 0..32 {
		let v = (b.add(a[j] as usize * s) as *const u8).read_unaligned();
		g[j] = v as u32;
	}
	regs8(g)
}
#[inline]
unsafe fn ld_i8(b: *const u8, ix: [uint32x4_t; 8], s: usize) -> [uint32x4_t; 8] {
	let u = minv(&ix);
	if u == maxv(&ix) {
		return [vdupq_n_u32((b.add(u as usize * s) as *const i8).read_unaligned() as i32 as u32); 8];
	}
	let a = words8(ix);
	let mut g = [0u32; 32];
	for j in 0..32 {
		let v = (b.add(a[j] as usize * s) as *const i8).read_unaligned();
		g[j] = v as i32 as u32;
	}
	regs8(g)
}
#[inline]
unsafe fn ld_u16(b: *const u8, ix: [uint32x4_t; 8], s: usize) -> [uint32x4_t; 8] {
	let u = minv(&ix);
	if u == maxv(&ix) {
		return [vdupq_n_u32((b.add(u as usize * s) as *const u16).read_unaligned() as u32); 8];
	}
	let a = words8(ix);
	let mut g = [0u32; 32];
	for j in 0..32 {
		let v = (b.add(a[j] as usize * s) as *const u16).read_unaligned();
		g[j] = v as u32;
	}
	regs8(g)
}
#[inline]
unsafe fn ld_i16(b: *const u8, ix: [uint32x4_t; 8], s: usize) -> [uint32x4_t; 8] {
	let u = minv(&ix);
	if u == maxv(&ix) {
		return [vdupq_n_u32((b.add(u as usize * s) as *const i16).read_unaligned() as i32 as u32); 8];
	}
	let a = words8(ix);
	let mut g = [0u32; 32];
	for j in 0..32 {
		let v = (b.add(a[j] as usize * s) as *const i16).read_unaligned();
		g[j] = v as i32 as u32;
	}
	regs8(g)
}
#[inline]
unsafe fn ld_u32(b: *const u8, ix: [uint32x4_t; 8], s: usize) -> [uint32x4_t; 8] {
	let u = minv(&ix);
	if u == maxv(&ix) {
		return [vdupq_n_u32((b.add(u as usize * s) as *const u32).read_unaligned() as u32); 8];
	}
	let a = words8(ix);
	let mut g = [0u32; 32];
	for j in 0..32 {
		let v = (b.add(a[j] as usize * s) as *const u32).read_unaligned();
		g[j] = v as u32;
	}
	regs8(g)
}
#[inline]
unsafe fn ld_u64(b: *const u8, ix: [uint32x4_t; 8], s: usize) -> [uint32x4_t; 16] {
	let u = minv(&ix);
	if u == maxv(&ix) {
		let v = (b.add(u as usize * s) as *const u64).read_unaligned();
		return core::array::from_fn(|r| vdupq_n_u32(if r < 8 { v as u32 } else { (v >> 32) as u32 }));
	}
	let a = words8(ix);
	let mut g = [0u32; 64];
	for j in 0..32 {
		let v = (b.add(a[j] as usize * s) as *const u64).read_unaligned();
		g[j] = v as u32;
		g[32 + j] = (v >> 32) as u32;
	}
	regs16(g)
}
#[inline]
unsafe fn ldj_u8(mut at: impl FnMut(usize) -> *const u8) -> [uint32x4_t; 8] {
	let mut g = [0u32; 32];
	for j in 0..32 {
		let v = (at(j) as *const u8).read_unaligned();
		g[j] = v as u32;
	}
	regs8(g)
}
#[inline]
unsafe fn stj_u8(k: [uint32x4_t; 8], v: [uint32x4_t; 8], mut at: impl FnMut(usize) -> *mut u8) {
	let (m, x) = (words8(k), words8(v));
	let mut z = 0u64;
	let sink = core::hint::black_box(&raw mut z) as *mut u8;
	for j in 0..32 {
		(core::hint::select_unpredictable(m[j] != 0, at(j), sink) as *mut u8).write_unaligned(x[j] as u8);
	}
}
#[inline]
unsafe fn stjf_u8(v: [uint32x4_t; 8], mut at: impl FnMut(usize) -> *mut u8) {
	let x = words8(v);
	for j in 0..32 {
		(at(j) as *mut u8).write_unaligned(x[j] as u8);
	}
}
#[inline]
unsafe fn ldj_i8(mut at: impl FnMut(usize) -> *const u8) -> [uint32x4_t; 8] {
	let mut g = [0u32; 32];
	for j in 0..32 {
		let v = (at(j) as *const i8).read_unaligned();
		g[j] = v as i32 as u32;
	}
	regs8(g)
}
#[inline]
unsafe fn ldj_u16(mut at: impl FnMut(usize) -> *const u8) -> [uint32x4_t; 8] {
	let mut g = [0u32; 32];
	for j in 0..32 {
		let v = (at(j) as *const u16).read_unaligned();
		g[j] = v as u32;
	}
	regs8(g)
}
#[inline]
unsafe fn stj_u16(k: [uint32x4_t; 8], v: [uint32x4_t; 8], mut at: impl FnMut(usize) -> *mut u8) {
	let (m, x) = (words8(k), words8(v));
	let mut z = 0u64;
	let sink = core::hint::black_box(&raw mut z) as *mut u8;
	for j in 0..32 {
		(core::hint::select_unpredictable(m[j] != 0, at(j), sink) as *mut u16).write_unaligned(x[j] as u16);
	}
}
#[inline]
unsafe fn stjf_u16(v: [uint32x4_t; 8], mut at: impl FnMut(usize) -> *mut u8) {
	let x = words8(v);
	for j in 0..32 {
		(at(j) as *mut u16).write_unaligned(x[j] as u16);
	}
}
#[inline]
unsafe fn ldj_i16(mut at: impl FnMut(usize) -> *const u8) -> [uint32x4_t; 8] {
	let mut g = [0u32; 32];
	for j in 0..32 {
		let v = (at(j) as *const i16).read_unaligned();
		g[j] = v as i32 as u32;
	}
	regs8(g)
}
#[inline]
unsafe fn ldj_u32(mut at: impl FnMut(usize) -> *const u8) -> [uint32x4_t; 8] {
	let mut g = [0u32; 32];
	for j in 0..32 {
		let v = (at(j) as *const u32).read_unaligned();
		g[j] = v as u32;
	}
	regs8(g)
}
#[inline]
unsafe fn stj_u32(k: [uint32x4_t; 8], v: [uint32x4_t; 8], mut at: impl FnMut(usize) -> *mut u8) {
	let (m, x) = (words8(k), words8(v));
	let mut z = 0u64;
	let sink = core::hint::black_box(&raw mut z) as *mut u8;
	for j in 0..32 {
		(core::hint::select_unpredictable(m[j] != 0, at(j), sink) as *mut u32).write_unaligned(x[j] as u32);
	}
}
#[inline]
unsafe fn stjf_u32(v: [uint32x4_t; 8], mut at: impl FnMut(usize) -> *mut u8) {
	let x = words8(v);
	for j in 0..32 {
		(at(j) as *mut u32).write_unaligned(x[j] as u32);
	}
}
#[inline]
unsafe fn ldj_u64(mut at: impl FnMut(usize) -> *const u8) -> [uint32x4_t; 16] {
	let mut g = [0u32; 64];
	for j in 0..32 {
		let v = (at(j) as *const u64).read_unaligned();
		g[j] = v as u32;
		g[32 + j] = (v >> 32) as u32;
	}
	regs16(g)
}
#[inline]
unsafe fn stj_u64(k: [uint32x4_t; 8], v: [uint32x4_t; 16], mut at: impl FnMut(usize) -> *mut u8) {
	let (m, x) = (words8(k), words16(v));
	let mut z = 0u64;
	let sink = core::hint::black_box(&raw mut z) as *mut u8;
	for j in 0..32 {
		(core::hint::select_unpredictable(m[j] != 0, at(j), sink) as *mut u64).write_unaligned((x[32 + j] as u64) << 32 | x[j] as u64);
	}
}
#[inline]
unsafe fn stjf_u64(v: [uint32x4_t; 16], mut at: impl FnMut(usize) -> *mut u8) {
	let x = words16(v);
	for j in 0..32 {
		(at(j) as *mut u64).write_unaligned((x[32 + j] as u64) << 32 | x[j] as u64);
	}
}
/// Each lane's address of its halves `lo` and `hi`; a lane mask `k` has off, the first on's.
#[inline(always)]
unsafe fn addrs(lo: [uint32x4_t; 8], hi: [uint32x4_t; 8], k: [uint32x4_t; 8]) -> [usize; 32] {
	let (l, h, b) = (words8(lo), words8(hi), bits(&k));
	let f = b.trailing_zeros() as usize % 32;
	core::array::from_fn(|j| {
		let j = if b >> j & 1 != 0 { j } else { f };
		((h[j] as u64) << 32 | l[j] as u64) as usize
	})
}
#[inline(always)]
unsafe fn bsl_w(k: [uint32x4_t; 8], a: [uint32x4_t; 16], b: [uint32x4_t; 16]) -> [uint32x4_t; 16] {
	core::array::from_fn(|r| vbslq_u32(k[r % 8], a[r], b[r]))
}
#[inline(always)]
unsafe fn cat_w(a: [uint32x4_t; 8], b: [uint32x4_t; 8]) -> [uint32x4_t; 16] {
	core::array::from_fn(|r| if r < 8 { a[r] } else { b[r - 8] })
}
/// Registers' lanes as words, and words as registers' lanes.
#[inline(always)]
unsafe fn words8(a: [uint32x4_t; 8]) -> [u32; 32] {
    core::mem::transmute::<[uint32x4_t; 8], [u32; 32]>(a)
}
#[inline(always)]
unsafe fn regs8(a: [u32; 32]) -> [uint32x4_t; 8] {
    core::mem::transmute::<[u32; 32], [uint32x4_t; 8]>(a)
}
#[inline(always)]
unsafe fn words16(a: [uint32x4_t; 16]) -> [u32; 64] {
    core::mem::transmute::<[uint32x4_t; 16], [u32; 64]>(a)
}
#[inline(always)]
unsafe fn regs16(a: [u32; 64]) -> [uint32x4_t; 16] {
    core::mem::transmute::<[u32; 64], [uint32x4_t; 16]>(a)
}

/// `mix_lanes_inner`.
#[inline(always)]
unsafe fn b0_r0r1sg_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg0: *const u8, arg1: *const u8, x2: *const u8) {
	let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x2 as *const u32).read_unaligned();
    let a1: u32 = (x2.wrapping_add(4u64 as usize) as *const u32).read_unaligned();
	let mut x4 = [Z; 8];
	let mut x5 = [Z; 8];
	let mut x6 = [Z; 8];
	let mut x7: u32 = 0;
	let mut x8: u32 = 0;
	let mut live = k;
	let t0 = iota(gid0);
	let t1 = splat(a0);
	let t2 = cge_r(t0, t1);
	let t3 = and_r(live, t2);
	{
		live = bic_r(live, t3);
	}
	if any(&live) {
		let mut t4: [uint32x4_t; 8];
		if all(&live) {
			let t5 = (arg0 as *const u32).add((gid0) as usize);
			t4 = [vld1q_u32(t5.add(0)), vld1q_u32(t5.add(4)), vld1q_u32(t5.add(8)), vld1q_u32(t5.add(12)), vld1q_u32(t5.add(16)), vld1q_u32(t5.add(20)), vld1q_u32(t5.add(24)), vld1q_u32(t5.add(28))];
		} else {
			let t6 = ix_on(live, t0);
			let t7 = arg0 as *const u32;
			let mut t8 = [Z; 8];
			let t9 = words8(t6);
			t8 = ldj_u32(|j| t7.add(t9[j] as usize) as *const u8);
			t4 = t8;
		}
		x4 = t4;
		let t10 = splat(1u32);
		let t11 = and_r(x4, t10);
		let t12 = tst_r(t11, t11);
		let t13 = and_r(live, t12);
		{
			let t14 = splat(3u32);
			let t15 = mul_r(x4, t14);
			let t16 = add_r(t15, t10);
			x5 = bsl_r(t13, t16, x5);
		}
		let t17 = bic_r(live, t12);
		{
			let t18: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(x4[r], vnegq_s32(vdupq_n_s32(((1u32) & 31) as i32))));
			x5 = bsl_r(t17, t18, x5);
		}
		let t19 = splat(4026531840u32);
		let t20 = clt_r(x4, t19);
		let t21 = and_r(live, t20);
		{
			let t22 = add_r(x5, x4);
			x5 = bsl_r(t21, t22, x5);
		}
		let t23 = [Z; 8];
		let t24 = ceq_r(x4, t23);
		let t25 = and_r(live, t24);
		{
			let t26 = splat(57005u32);
			x5 = bsl_r(t25, t26, x5);
		}
		let t27 = bic_r(live, t24);
		{
			let t28 = splat(7u32);
			let t29 = eor_r(x5, t28);
			x5 = bsl_r(t27, t29, x5);
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
					let t33 = words8(x4);
					let mut x4_j: u32 = t33[j];
					let t34 = words8(x5);
					let mut x5_j: u32 = t34[j];
					let t35 = words8(x6);
					let mut x6_j: u32 = t35[j];
					let mut x8_j: u32 = x8;
					loop {
						if !(((x8_j) < (((x4_j) & (15u32))))) { break; }
						x6_j = (x6_j).wrapping_add((x8_j).wrapping_mul(x5_j));
						let t36 = x8_j;
						x8_j = (x8_j).wrapping_add(1u32);
					}
					let mut t37 = words8(x6);
					t37[j] = x6_j as u32;
					x6 = regs8(t37);
				}
				break;
			}
			// Lanes part: each its own rounds.
			let t38 = splat(15u32);
			let t39 = and_r(x4, t38);
			let t40 = splat(x8);
			let t41 = clt_r(t40, t39);
			t30 = and_r(t30, t41);
			if !(any(&t30)) { break; }
			let t42 = splat(x8);
			let t43 = mul_r(t42, x5);
			let t44 = add_r(x6, t43);
			x6 = bsl_r(t30, t44, x6);
			let t45 = and_r(t30, live);
			let t46 = x8;
			x8 = (x8).wrapping_add(1u32);
		}
		loop {
			// Rounds of one condition: every lane goes on.
			if !(((x7) < (a1))) { break; }
			let t47 = splat(31u32);
			let t48 = mul_r(x6, t47);
			let t49 = splat(x7);
			let t50 = add_r(t48, t49);
			x6 = t50;
			let t51 = x7;
			x7 = (x7).wrapping_add(1u32);
		}
		let t52 = add_r(x5, x6);
		let t53 = (arg1 as *mut u32).add((gid0) as usize);
		if all(&live) {
			vst1q_u32(t53.add(0), t52[0]);
			vst1q_u32(t53.add(4), t52[1]);
			vst1q_u32(t53.add(8), t52[2]);
			vst1q_u32(t53.add(12), t52[3]);
			vst1q_u32(t53.add(16), t52[4]);
			vst1q_u32(t53.add(20), t52[5]);
			vst1q_u32(t53.add(24), t52[6]);
			vst1q_u32(t53.add(28), t52[7]);
		} else {
			stj_u32(live, t52, |j| t53.add(j) as *mut u8);
		}
	}
}

/// `mix_lanes_inner`.
#[inline(always)]
unsafe fn b0_r0r1sg_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: *const u8) {
	let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x2 as *const u32).read_unaligned();
    let a1: u32 = (x2.wrapping_add(4u64 as usize) as *const u32).read_unaligned();
	let mut x4 = [Z; 8];
	let mut x5 = [Z; 8];
	let mut x6 = [Z; 8];
	let mut x7: u32 = 0;
	let mut x8: u32 = 0;
	let mut live = splat(u32::MAX);
	if ((gid0) as u64 + 31) < ((a0) as u64) {
		let t0 = (arg0 as *const u32).add((gid0) as usize);
		let t1: [uint32x4_t; 8] = [vld1q_u32(t0.add(0)), vld1q_u32(t0.add(4)), vld1q_u32(t0.add(8)), vld1q_u32(t0.add(12)), vld1q_u32(t0.add(16)), vld1q_u32(t0.add(20)), vld1q_u32(t0.add(24)), vld1q_u32(t0.add(28))];
		x4 = t1;
		let t2 = splat(1u32);
		let t3 = and_r(x4, t2);
		let t4 = tst_r(t3, t3);
		{
			let t5 = splat(3u32);
			let t6 = mul_r(x4, t5);
			let t7 = add_r(t6, t2);
			x5 = bsl_r(t4, t7, x5);
		}
		let t8 = mvn_r(t4);
		{
			let t9: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(x4[r], vnegq_s32(vdupq_n_s32(((1u32) & 31) as i32))));
			x5 = bsl_r(t8, t9, x5);
		}
		let t10 = splat(4026531840u32);
		let t11 = clt_r(x4, t10);
		if all(&t11) {
			let t12 = add_r(x5, x4);
			x5 = t12;
		} else {
			// Lanes part: the chunk's lanes go both ways.
			let t13 = and_r(live, t11);
			{
				let t14 = add_r(x5, x4);
				x5 = bsl_r(t13, t14, x5);
			}
		}
		let t15 = [Z; 8];
		let t16 = ceq_r(x4, t15);
		if !any(&t16) {
			let t17 = splat(7u32);
			let t18 = eor_r(x5, t17);
			x5 = t18;
		} else {
			// Lanes part: the chunk's lanes go both ways.
			let t19 = and_r(live, t16);
			{
				let t20 = splat(57005u32);
				x5 = bsl_r(t19, t20, x5);
			}
			let t21 = bic_r(live, t16);
			{
				let t22 = splat(7u32);
				let t23 = eor_r(x5, t22);
				x5 = bsl_r(t21, t23, x5);
			}
		}
		x6 = t15;
		x7 = 0u32;
		x8 = 0u32;
		loop {
			// Full rounds: every lane goes on.
			let t24 = splat(15u32);
			let t25 = and_r(x4, t24);
			let t26 = splat(x8);
			let t27 = clt_r(t26, t25);
			if !all(&t27) { break; }
			let t28 = splat(x8);
			let t29 = mul_r(t28, x5);
			let t30 = add_r(x6, t29);
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
					let t35 = words8(x4);
					let mut x4_j: u32 = t35[j];
					let t36 = words8(x5);
					let mut x5_j: u32 = t36[j];
					let t37 = words8(x6);
					let mut x6_j: u32 = t37[j];
					let mut x8_j: u32 = x8;
					loop {
						if !(((x8_j) < (((x4_j) & (15u32))))) { break; }
						x6_j = (x6_j).wrapping_add((x8_j).wrapping_mul(x5_j));
						let t38 = x8_j;
						x8_j = (x8_j).wrapping_add(1u32);
					}
					let mut t39 = words8(x6);
					t39[j] = x6_j as u32;
					x6 = regs8(t39);
				}
				break;
			}
			// Lanes part: each its own rounds.
			let t40 = splat(15u32);
			let t41 = and_r(x4, t40);
			let t42 = splat(x8);
			let t43 = clt_r(t42, t41);
			t32 = and_r(t32, t43);
			if !(any(&t32)) { break; }
			let t44 = splat(x8);
			let t45 = mul_r(t44, x5);
			let t46 = add_r(x6, t45);
			x6 = bsl_r(t32, t46, x6);
			let t47 = and_r(t32, live);
			let t48 = x8;
			x8 = (x8).wrapping_add(1u32);
		}
		loop {
			// Rounds of one condition: every lane goes on.
			if !(((x7) < (a1))) { break; }
			let t49 = splat(31u32);
			let t50 = mul_r(x6, t49);
			let t51 = splat(x7);
			let t52 = add_r(t50, t51);
			x6 = t52;
			let t53 = x7;
			x7 = (x7).wrapping_add(1u32);
		}
		let t54 = add_r(x5, x6);
		let t55 = (arg1 as *mut u32).add((gid0) as usize);
		vst1q_u32(t55.add(0), t54[0]);
		vst1q_u32(t55.add(4), t54[1]);
		vst1q_u32(t55.add(8), t54[2]);
		vst1q_u32(t55.add(12), t54[3]);
		vst1q_u32(t55.add(16), t54[4]);
		vst1q_u32(t55.add(20), t54[5]);
		vst1q_u32(t55.add(24), t54[6]);
		vst1q_u32(t55.add(28), t54[7]);
	} else if (((gid0) as u64 + 31) <= u32::MAX as u64 && ((gid0) as u64) >= ((a0) as u64)) {
		return;
	} else {
		// Lanes part: the chunk's lanes on both sides of the bound.
		let t56 = iota(gid0);
		let t57 = splat(a0);
		let t58 = cge_r(t56, t57);
		let t59 = and_r(live, t58);
		{
			live = bic_r(live, t59);
		}
		if any(&live) {
			let mut t60: [uint32x4_t; 8];
			if all(&live) {
				let t61 = (arg0 as *const u32).add((gid0) as usize);
				t60 = [vld1q_u32(t61.add(0)), vld1q_u32(t61.add(4)), vld1q_u32(t61.add(8)), vld1q_u32(t61.add(12)), vld1q_u32(t61.add(16)), vld1q_u32(t61.add(20)), vld1q_u32(t61.add(24)), vld1q_u32(t61.add(28))];
			} else {
				let t62 = ix_on(live, t56);
				let t63 = arg0 as *const u32;
				let mut t64 = [Z; 8];
				let t65 = words8(t62);
				t64 = ldj_u32(|j| t63.add(t65[j] as usize) as *const u8);
				t60 = t64;
			}
			x4 = t60;
			let t66 = splat(1u32);
			let t67 = and_r(x4, t66);
			let t68 = tst_r(t67, t67);
			let t69 = and_r(live, t68);
			{
				let t70 = splat(3u32);
				let t71 = mul_r(x4, t70);
				let t72 = add_r(t71, t66);
				x5 = bsl_r(t69, t72, x5);
			}
			let t73 = bic_r(live, t68);
			{
				let t74: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(x4[r], vnegq_s32(vdupq_n_s32(((1u32) & 31) as i32))));
				x5 = bsl_r(t73, t74, x5);
			}
			let t75 = splat(4026531840u32);
			let t76 = clt_r(x4, t75);
			let t77 = and_r(live, t76);
			{
				let t78 = add_r(x5, x4);
				x5 = bsl_r(t77, t78, x5);
			}
			let t79 = [Z; 8];
			let t80 = ceq_r(x4, t79);
			let t81 = and_r(live, t80);
			{
				let t82 = splat(57005u32);
				x5 = bsl_r(t81, t82, x5);
			}
			let t83 = bic_r(live, t80);
			{
				let t84 = splat(7u32);
				let t85 = eor_r(x5, t84);
				x5 = bsl_r(t83, t85, x5);
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
						let t89 = words8(x4);
						let mut x4_j: u32 = t89[j];
						let t90 = words8(x5);
						let mut x5_j: u32 = t90[j];
						let t91 = words8(x6);
						let mut x6_j: u32 = t91[j];
						let mut x8_j: u32 = x8;
						loop {
							if !(((x8_j) < (((x4_j) & (15u32))))) { break; }
							x6_j = (x6_j).wrapping_add((x8_j).wrapping_mul(x5_j));
							let t92 = x8_j;
							x8_j = (x8_j).wrapping_add(1u32);
						}
						let mut t93 = words8(x6);
						t93[j] = x6_j as u32;
						x6 = regs8(t93);
					}
					break;
				}
				// Lanes part: each its own rounds.
				let t94 = splat(15u32);
				let t95 = and_r(x4, t94);
				let t96 = splat(x8);
				let t97 = clt_r(t96, t95);
				t86 = and_r(t86, t97);
				if !(any(&t86)) { break; }
				let t98 = splat(x8);
				let t99 = mul_r(t98, x5);
				let t100 = add_r(x6, t99);
				x6 = bsl_r(t86, t100, x6);
				let t101 = and_r(t86, live);
				let t102 = x8;
				x8 = (x8).wrapping_add(1u32);
			}
			loop {
				// Rounds of one condition: every lane goes on.
				if !(((x7) < (a1))) { break; }
				let t103 = splat(31u32);
				let t104 = mul_r(x6, t103);
				let t105 = splat(x7);
				let t106 = add_r(t104, t105);
				x6 = t106;
				let t107 = x7;
				x7 = (x7).wrapping_add(1u32);
			}
			let t108 = add_r(x5, x6);
			let t109 = (arg1 as *mut u32).add((gid0) as usize);
			if all(&live) {
				vst1q_u32(t109.add(0), t108[0]);
				vst1q_u32(t109.add(4), t108[1]);
				vst1q_u32(t109.add(8), t108[2]);
				vst1q_u32(t109.add(12), t108[3]);
				vst1q_u32(t109.add(16), t108[4]);
				vst1q_u32(t109.add(20), t108[5]);
				vst1q_u32(t109.add(24), t108[6]);
				vst1q_u32(t109.add(28), t108[7]);
			} else {
				stj_u32(live, t108, |j| t109.add(j) as *mut u8);
			}
		}
	}
}

/// The group `l.group`'s threads, 32 at a time: `argv[j]` the address of argument `j`'s bytes (a buffer's address,
/// a value's bytes).
///
/// # Safety
///
/// `argv` holds the kernel's arguments, its buffers as large as it reads and writes; `l` is a 1-D group's launch.
pub unsafe fn mix_lanes_w32(argv: *const *const u8, l: &Launch) {
	let n = l.block[0] * l.block[1] * l.block[2];
	let arg0: *const u8 = (*argv.add(0) as *const *const u8).read_unaligned();
	let arg1: *const u8 = (*argv.add(1) as *const *const u8).read_unaligned();
	let x2: *const u8 = *argv.add(2);
	let arg3: *const u8 = (*argv.add(3) as *const *const u8).read_unaligned();
	let x4: u32 = (*argv.add(4) as *const u32).read_unaligned();
	let mut t183: bool = !(arg3).is_null();
	if t183 {
		let mut t184: bool = ((l.group[0]) >= ((arg3.wrapping_add((((x4) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
		if !t184 {
			t184 = ((l.group[1]) >= ((arg3.wrapping_add(((((x4).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
		}
		let mut t185: bool = t184;
		if !t185 {
			t185 = ((l.group[2]) >= ((arg3.wrapping_add(((((x4).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
		}
		t183 = t185;
	}
	if t183 {
		return;
	}
	let mut lid0 = 0u32;
	while lid0 + 32 <= n {
		let mut live = splat(u32::MAX);
		let gid0 = l.group[0] * l.block[0] + lid0;
		'chunk: {
			b0_r0r1sg_f(l, lid0, arg0, arg1, x2);
		}
		lid0 += 32;
	}
	if lid0 < n {
		// Lanes part: the group's last threads, those under `n`.
		let t186: [uint32x4_t; 8] = core::array::from_fn(|r| vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[r].as_ptr())), vdupq_n_u32(n)));
		let mut live = t186;
		let gid0 = l.group[0] * l.block[0] + lid0;
		'chunk: {
			b0_r0r1sg_p(l, lid0, live, arg0, arg1, x2);
		}
	}
}
