//! `wide_lanes` at 32 lanes (8 NEON registers), written by rerail-ukl-vec's Rust (`rs.rs`) from its vector tape: do
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
#[inline(always)]
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
#[inline(always)]
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
#[inline(always)]
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
#[inline(always)]
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
#[inline(always)]
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
#[inline(always)]
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
#[inline(always)]
unsafe fn ldj_u8(at: impl Fn(usize) -> *const u8) -> [uint32x4_t; 8] {
	let mut g = [0u32; 32];
	for j in 0..32 {
		let v = (at(j) as *const u8).read_unaligned();
		g[j] = v as u32;
	}
	regs8(g)
}
#[inline(always)]
unsafe fn stj_u8(k: [uint32x4_t; 8], v: [uint32x4_t; 8], at: impl Fn(usize) -> *mut u8) {
	let (m, x) = (words8(k), words8(v));
	let mut z = 0u64;
	let sink = core::hint::black_box(&raw mut z) as *mut u8;
	for j in 0..32 {
		(core::hint::select_unpredictable(m[j] != 0, at(j), sink) as *mut u8).write_unaligned(x[j] as u8);
	}
}
#[inline(always)]
unsafe fn stjf_u8(v: [uint32x4_t; 8], at: impl Fn(usize) -> *mut u8) {
	let x = words8(v);
	for j in 0..32 {
		(at(j) as *mut u8).write_unaligned(x[j] as u8);
	}
}
#[inline(always)]
unsafe fn ldj_i8(at: impl Fn(usize) -> *const u8) -> [uint32x4_t; 8] {
	let mut g = [0u32; 32];
	for j in 0..32 {
		let v = (at(j) as *const i8).read_unaligned();
		g[j] = v as i32 as u32;
	}
	regs8(g)
}
#[inline(always)]
unsafe fn ldj_u16(at: impl Fn(usize) -> *const u8) -> [uint32x4_t; 8] {
	let mut g = [0u32; 32];
	for j in 0..32 {
		let v = (at(j) as *const u16).read_unaligned();
		g[j] = v as u32;
	}
	regs8(g)
}
#[inline(always)]
unsafe fn stj_u16(k: [uint32x4_t; 8], v: [uint32x4_t; 8], at: impl Fn(usize) -> *mut u8) {
	let (m, x) = (words8(k), words8(v));
	let mut z = 0u64;
	let sink = core::hint::black_box(&raw mut z) as *mut u8;
	for j in 0..32 {
		(core::hint::select_unpredictable(m[j] != 0, at(j), sink) as *mut u16).write_unaligned(x[j] as u16);
	}
}
#[inline(always)]
unsafe fn stjf_u16(v: [uint32x4_t; 8], at: impl Fn(usize) -> *mut u8) {
	let x = words8(v);
	for j in 0..32 {
		(at(j) as *mut u16).write_unaligned(x[j] as u16);
	}
}
#[inline(always)]
unsafe fn ldj_i16(at: impl Fn(usize) -> *const u8) -> [uint32x4_t; 8] {
	let mut g = [0u32; 32];
	for j in 0..32 {
		let v = (at(j) as *const i16).read_unaligned();
		g[j] = v as i32 as u32;
	}
	regs8(g)
}
#[inline(always)]
unsafe fn ldj_u32(at: impl Fn(usize) -> *const u8) -> [uint32x4_t; 8] {
	let mut g = [0u32; 32];
	for j in 0..32 {
		let v = (at(j) as *const u32).read_unaligned();
		g[j] = v as u32;
	}
	regs8(g)
}
#[inline(always)]
unsafe fn stj_u32(k: [uint32x4_t; 8], v: [uint32x4_t; 8], at: impl Fn(usize) -> *mut u8) {
	let (m, x) = (words8(k), words8(v));
	let mut z = 0u64;
	let sink = core::hint::black_box(&raw mut z) as *mut u8;
	for j in 0..32 {
		(core::hint::select_unpredictable(m[j] != 0, at(j), sink) as *mut u32).write_unaligned(x[j] as u32);
	}
}
#[inline(always)]
unsafe fn stjf_u32(v: [uint32x4_t; 8], at: impl Fn(usize) -> *mut u8) {
	let x = words8(v);
	for j in 0..32 {
		(at(j) as *mut u32).write_unaligned(x[j] as u32);
	}
}
#[inline(always)]
unsafe fn ldj_u64(at: impl Fn(usize) -> *const u8) -> [uint32x4_t; 16] {
	let mut g = [0u32; 64];
	for j in 0..32 {
		let v = (at(j) as *const u64).read_unaligned();
		g[j] = v as u32;
		g[32 + j] = (v >> 32) as u32;
	}
	regs16(g)
}
#[inline(always)]
unsafe fn stj_u64(k: [uint32x4_t; 8], v: [uint32x4_t; 16], at: impl Fn(usize) -> *mut u8) {
	let (m, x) = (words8(k), words16(v));
	let mut z = 0u64;
	let sink = core::hint::black_box(&raw mut z) as *mut u8;
	for j in 0..32 {
		(core::hint::select_unpredictable(m[j] != 0, at(j), sink) as *mut u64).write_unaligned((x[32 + j] as u64) << 32 | x[j] as u64);
	}
}
#[inline(always)]
unsafe fn stjf_u64(v: [uint32x4_t; 16], at: impl Fn(usize) -> *mut u8) {
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

/// `wide_lanes_inner`.
#[inline(always)]
unsafe fn b0_r0r1r2r3sg_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg0: *const u8, arg1: *const u8, arg2: *const u8, arg3: *const u8, x4: *const u8) {
	let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x4 as *const u32).read_unaligned();
    let a1: u64 = (x4.wrapping_add(8u64 as usize) as *const u64).read_unaligned();
	let mut x6 = [Z; 16];
	let mut x7 = [Z; 16];
	let mut x8 = [Z; 8];
	let mut x9 = [Z; 8];
	let mut x10: u32 = 0;
	let mut x11 = [Z; 16];
	let mut x12 = [Z; 16];
	let mut x13: u32 = 0;
	let mut x14: u32 = 0;
	let mut x15: u32 = 0;
	let mut live = k;
	let t0 = iota(gid0);
	let t1 = splat(a0);
	let t2 = cge_r(t0, t1);
	let t3 = and_r(live, t2);
	{
		live = bic_r(live, t3);
	}
	if any(&live) {
		let t4 = (arg0 as *const u8).add(((gid0) as usize) * 8);
		let mut t5: [uint32x4_t; 16];
		if all(&live) {
			let t6 = vld2q_u32((t4 as *const u32).add(0));
			let t7 = vld2q_u32((t4 as *const u32).add(8));
			let t8 = vld2q_u32((t4 as *const u32).add(16));
			let t9 = vld2q_u32((t4 as *const u32).add(24));
			let t10 = vld2q_u32((t4 as *const u32).add(32));
			let t11 = vld2q_u32((t4 as *const u32).add(40));
			let t12 = vld2q_u32((t4 as *const u32).add(48));
			let t13 = vld2q_u32((t4 as *const u32).add(56));
			let t14: [uint32x4_t; 16] = [t6.0, t7.0, t8.0, t9.0, t10.0, t11.0, t12.0, t13.0, t6.1, t7.1, t8.1, t9.1, t10.1, t11.1, t12.1, t13.1];
			t5 = t14;
		} else {
			let t15 = ix_on(live, t0);
			let t16 = ld_u64(arg0 as *const u8, t15, 8);
			t5 = t16;
		}
		x6 = t5;
		let t17 = (arg1 as *const u8).add(((gid0) as usize) * 8);
		let mut t18: [uint32x4_t; 16];
		if all(&live) {
			let t19 = vld2q_u32((t17 as *const u32).add(0));
			let t20 = vld2q_u32((t17 as *const u32).add(8));
			let t21 = vld2q_u32((t17 as *const u32).add(16));
			let t22 = vld2q_u32((t17 as *const u32).add(24));
			let t23 = vld2q_u32((t17 as *const u32).add(32));
			let t24 = vld2q_u32((t17 as *const u32).add(40));
			let t25 = vld2q_u32((t17 as *const u32).add(48));
			let t26 = vld2q_u32((t17 as *const u32).add(56));
			let t27: [uint32x4_t; 16] = [t19.0, t20.0, t21.0, t22.0, t23.0, t24.0, t25.0, t26.0, t19.1, t20.1, t21.1, t22.1, t23.1, t24.1, t25.1, t26.1];
			t18 = t27;
		} else {
			let t28 = ix_on(live, t0);
			let t29 = ld_u64(arg1 as *const u8, t28, 8);
			t18 = t29;
		}
		x7 = t18;
		let mut t30: [uint32x4_t; 8];
		if all(&live) {
			let t31 = (arg2 as *const u32).add((gid0) as usize);
			t30 = [vld1q_u32(t31.add(0)), vld1q_u32(t31.add(4)), vld1q_u32(t31.add(8)), vld1q_u32(t31.add(12)), vld1q_u32(t31.add(16)), vld1q_u32(t31.add(20)), vld1q_u32(t31.add(24)), vld1q_u32(t31.add(28))];
		} else {
			let t32 = ix_on(live, t0);
			let t33 = arg2 as *const u32;
			let mut t34 = [Z; 8];
			let t35 = words8(t32);
			t34 = ldj_u32(|j| t33.add(t35[j] as usize) as *const u8);
			t30 = t34;
		}
		x8 = t30;
		let t36 = splat(63u32);
		let t37 = and_r(x8, t36);
		x9 = t37;
		x10 = a0;
		let t38: [uint32x4_t; 8] = core::array::from_fn(|r| vaddq_u32(x6[r], x7[r]));
		let t39 = clt_r(t38, lo(x6));
		let t40 = adc_r(hi(x6), hi(x7), t39);
		let t41 = cat_w(lo(t38), t40);
		let t42 = (arg3 as *const u8).add(((gid0.wrapping_add((0u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t42 as *mut u32).add(0), uint32x4x2_t(t41[0], t41[8]));
			vst2q_u32((t42 as *mut u32).add(8), uint32x4x2_t(t41[1], t41[9]));
			vst2q_u32((t42 as *mut u32).add(16), uint32x4x2_t(t41[2], t41[10]));
			vst2q_u32((t42 as *mut u32).add(24), uint32x4x2_t(t41[3], t41[11]));
			vst2q_u32((t42 as *mut u32).add(32), uint32x4x2_t(t41[4], t41[12]));
			vst2q_u32((t42 as *mut u32).add(40), uint32x4x2_t(t41[5], t41[13]));
			vst2q_u32((t42 as *mut u32).add(48), uint32x4x2_t(t41[6], t41[14]));
			vst2q_u32((t42 as *mut u32).add(56), uint32x4x2_t(t41[7], t41[15]));
		} else {
			stj_u64(live, t41, |j| (t42 as *mut u64).add(j) as *mut u8);
		}
		let t43: [uint32x4_t; 8] = core::array::from_fn(|r| vcltq_u32(x6[r], x7[r]));
		let t44: [uint32x4_t; 8] = core::array::from_fn(|r| vsubq_u32(x6[r], x7[r]));
		let t45 = sbc_r(hi(x6), hi(x7), t43);
		let t46 = cat_w(t44, t45);
		let t47 = (arg3 as *const u8).add(((gid0.wrapping_add((1u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t47 as *mut u32).add(0), uint32x4x2_t(t46[0], t46[8]));
			vst2q_u32((t47 as *mut u32).add(8), uint32x4x2_t(t46[1], t46[9]));
			vst2q_u32((t47 as *mut u32).add(16), uint32x4x2_t(t46[2], t46[10]));
			vst2q_u32((t47 as *mut u32).add(24), uint32x4x2_t(t46[3], t46[11]));
			vst2q_u32((t47 as *mut u32).add(32), uint32x4x2_t(t46[4], t46[12]));
			vst2q_u32((t47 as *mut u32).add(40), uint32x4x2_t(t46[5], t46[13]));
			vst2q_u32((t47 as *mut u32).add(48), uint32x4x2_t(t46[6], t46[14]));
			vst2q_u32((t47 as *mut u32).add(56), uint32x4x2_t(t46[7], t46[15]));
		} else {
			stj_u64(live, t46, |j| (t47 as *mut u64).add(j) as *mut u8);
		}
		let t48: [uint32x4_t; 8] = lo(x9);
		let t49: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t48[r], vdupq_n_u32(63)));
		let t50: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(x6[r], vreinterpretq_s32_u32(t49[r])));
		let t51: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(vshlq_u32(x6[(r + 8)], vreinterpretq_s32_u32(t49[r])), vshlq_u32(x6[r], vsubq_s32(vreinterpretq_s32_u32(t49[r]), vdupq_n_s32(32)))));
		let t52 = cat_w(t50, t51);
		let t53 = (arg3 as *const u8).add(((gid0.wrapping_add((2u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t53 as *mut u32).add(0), uint32x4x2_t(t52[0], t52[8]));
			vst2q_u32((t53 as *mut u32).add(8), uint32x4x2_t(t52[1], t52[9]));
			vst2q_u32((t53 as *mut u32).add(16), uint32x4x2_t(t52[2], t52[10]));
			vst2q_u32((t53 as *mut u32).add(24), uint32x4x2_t(t52[3], t52[11]));
			vst2q_u32((t53 as *mut u32).add(32), uint32x4x2_t(t52[4], t52[12]));
			vst2q_u32((t53 as *mut u32).add(40), uint32x4x2_t(t52[5], t52[13]));
			vst2q_u32((t53 as *mut u32).add(48), uint32x4x2_t(t52[6], t52[14]));
			vst2q_u32((t53 as *mut u32).add(56), uint32x4x2_t(t52[7], t52[15]));
		} else {
			stj_u64(live, t52, |j| (t53 as *mut u64).add(j) as *mut u8);
		}
		let t54: [uint32x4_t; 8] = lo(x9);
		let t55: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t54[r], vdupq_n_u32(63)));
		let t56: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(vshlq_u32(x6[r], vnegq_s32(vreinterpretq_s32_u32(t55[r]))), vshlq_u32(x6[(r + 8)], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t55[r])))));
		let t57: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(x6[(r + 8)], vnegq_s32(vreinterpretq_s32_u32(t55[r]))));
		let t58 = cat_w(t56, t57);
		let t59 = (arg3 as *const u8).add(((gid0.wrapping_add((3u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t59 as *mut u32).add(0), uint32x4x2_t(t58[0], t58[8]));
			vst2q_u32((t59 as *mut u32).add(8), uint32x4x2_t(t58[1], t58[9]));
			vst2q_u32((t59 as *mut u32).add(16), uint32x4x2_t(t58[2], t58[10]));
			vst2q_u32((t59 as *mut u32).add(24), uint32x4x2_t(t58[3], t58[11]));
			vst2q_u32((t59 as *mut u32).add(32), uint32x4x2_t(t58[4], t58[12]));
			vst2q_u32((t59 as *mut u32).add(40), uint32x4x2_t(t58[5], t58[13]));
			vst2q_u32((t59 as *mut u32).add(48), uint32x4x2_t(t58[6], t58[14]));
			vst2q_u32((t59 as *mut u32).add(56), uint32x4x2_t(t58[7], t58[15]));
		} else {
			stj_u64(live, t58, |j| (t59 as *mut u64).add(j) as *mut u8);
		}
		let t60: [uint32x4_t; 8] = lo(x9);
		let t61: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t60[r], vdupq_n_u32(63)));
		let t62: [uint32x4_t; 8] = core::array::from_fn(|r| vcgeq_s32(vreinterpretq_s32_u32(t61[r]), vdupq_n_s32(32)));
		let t63: [uint32x4_t; 8] = core::array::from_fn(|r| vbslq_u32(t62[r], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[(r + 8)]), vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t61[r])))), vorrq_u32(vshlq_u32(x6[r], vnegq_s32(vreinterpretq_s32_u32(t61[r]))), vshlq_u32(x6[(r + 8)], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t61[r]))))));
		let t64: [uint32x4_t; 8] = core::array::from_fn(|r| vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[(r + 8)]), vnegq_s32(vreinterpretq_s32_u32(t61[r])))));
		let t65 = cat_w(t63, t64);
		let t66 = (arg3 as *const u8).add(((gid0.wrapping_add((4u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t66 as *mut u32).add(0), uint32x4x2_t(t65[0], t65[8]));
			vst2q_u32((t66 as *mut u32).add(8), uint32x4x2_t(t65[1], t65[9]));
			vst2q_u32((t66 as *mut u32).add(16), uint32x4x2_t(t65[2], t65[10]));
			vst2q_u32((t66 as *mut u32).add(24), uint32x4x2_t(t65[3], t65[11]));
			vst2q_u32((t66 as *mut u32).add(32), uint32x4x2_t(t65[4], t65[12]));
			vst2q_u32((t66 as *mut u32).add(40), uint32x4x2_t(t65[5], t65[13]));
			vst2q_u32((t66 as *mut u32).add(48), uint32x4x2_t(t65[6], t65[14]));
			vst2q_u32((t66 as *mut u32).add(56), uint32x4x2_t(t65[7], t65[15]));
		} else {
			stj_u64(live, t65, |j| (t66 as *mut u64).add(j) as *mut u8);
		}
		let t67: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_n_u32::<13>(x6[r]));
		let t68: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(vshlq_n_u32::<13>(x6[(r + 8)]), vshrq_n_u32::<19>(x6[r])));
		let t69 = cat_w(t67, t68);
		let t70: [uint32x4_t; 8] = core::array::from_fn(|r| vshrq_n_u32::<19>(x7[(r + 8)]));
		let t71 = orr_r(lo(t69), t70);
		let t72 = cat_w(t71, hi(t69));
		let t73 = (arg3 as *const u8).add(((gid0.wrapping_add((5u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t73 as *mut u32).add(0), uint32x4x2_t(t72[0], t72[8]));
			vst2q_u32((t73 as *mut u32).add(8), uint32x4x2_t(t72[1], t72[9]));
			vst2q_u32((t73 as *mut u32).add(16), uint32x4x2_t(t72[2], t72[10]));
			vst2q_u32((t73 as *mut u32).add(24), uint32x4x2_t(t72[3], t72[11]));
			vst2q_u32((t73 as *mut u32).add(32), uint32x4x2_t(t72[4], t72[12]));
			vst2q_u32((t73 as *mut u32).add(40), uint32x4x2_t(t72[5], t72[13]));
			vst2q_u32((t73 as *mut u32).add(48), uint32x4x2_t(t72[6], t72[14]));
			vst2q_u32((t73 as *mut u32).add(56), uint32x4x2_t(t72[7], t72[15]));
		} else {
			stj_u64(live, t72, |j| (t73 as *mut u64).add(j) as *mut u8);
		}
		let t74: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_n_u32::<8>(x6[r]));
		let t75 = cat_w([Z; 8], t74);
		let t76 = (arg3 as *const u8).add(((gid0.wrapping_add((6u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t76 as *mut u32).add(0), uint32x4x2_t(t75[0], t75[8]));
			vst2q_u32((t76 as *mut u32).add(8), uint32x4x2_t(t75[1], t75[9]));
			vst2q_u32((t76 as *mut u32).add(16), uint32x4x2_t(t75[2], t75[10]));
			vst2q_u32((t76 as *mut u32).add(24), uint32x4x2_t(t75[3], t75[11]));
			vst2q_u32((t76 as *mut u32).add(32), uint32x4x2_t(t75[4], t75[12]));
			vst2q_u32((t76 as *mut u32).add(40), uint32x4x2_t(t75[5], t75[13]));
			vst2q_u32((t76 as *mut u32).add(48), uint32x4x2_t(t75[6], t75[14]));
			vst2q_u32((t76 as *mut u32).add(56), uint32x4x2_t(t75[7], t75[15]));
		} else {
			stj_u64(live, t75, |j| (t76 as *mut u64).add(j) as *mut u8);
		}
		let t77: [uint32x4_t; 8] = core::array::from_fn(|r| vshrq_n_u32::<3>(x6[(r + 8)]));
		let t78 = cat_w(lo(t77), [Z; 8]);
		let t79 = (arg3 as *const u8).add(((gid0.wrapping_add((7u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t79 as *mut u32).add(0), uint32x4x2_t(t78[0], t78[8]));
			vst2q_u32((t79 as *mut u32).add(8), uint32x4x2_t(t78[1], t78[9]));
			vst2q_u32((t79 as *mut u32).add(16), uint32x4x2_t(t78[2], t78[10]));
			vst2q_u32((t79 as *mut u32).add(24), uint32x4x2_t(t78[3], t78[11]));
			vst2q_u32((t79 as *mut u32).add(32), uint32x4x2_t(t78[4], t78[12]));
			vst2q_u32((t79 as *mut u32).add(40), uint32x4x2_t(t78[5], t78[13]));
			vst2q_u32((t79 as *mut u32).add(48), uint32x4x2_t(t78[6], t78[14]));
			vst2q_u32((t79 as *mut u32).add(56), uint32x4x2_t(t78[7], t78[15]));
		} else {
			stj_u64(live, t78, |j| (t79 as *mut u64).add(j) as *mut u8);
		}
		let t80 = vdupq_n_s32(((((40i64) as u32)) & 63) as i32);
		let t81: [uint32x4_t; 8] = [vcgeq_s32(t80, vdupq_n_s32(32)), vcgeq_s32(t80, vdupq_n_s32(32)), vcgeq_s32(t80, vdupq_n_s32(32)), vcgeq_s32(t80, vdupq_n_s32(32)), vcgeq_s32(t80, vdupq_n_s32(32)), vcgeq_s32(t80, vdupq_n_s32(32)), vcgeq_s32(t80, vdupq_n_s32(32)), vcgeq_s32(t80, vdupq_n_s32(32))];
		let t82: [uint32x4_t; 8] = core::array::from_fn(|r| vbslq_u32(t81[r], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[(r + 8)]), vsubq_s32(vdupq_n_s32(32), t80))), vorrq_u32(vshlq_u32(x6[r], vnegq_s32(t80)), vshlq_u32(x6[(r + 8)], vsubq_s32(vdupq_n_s32(32), t80)))));
		let t83: [uint32x4_t; 8] = core::array::from_fn(|r| vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[(r + 8)]), vnegq_s32(t80))));
		let t84 = cat_w(t82, t83);
		let t6 = (arg3 as *const u8).add(((gid0.wrapping_add((8u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t6 as *mut u32).add(0), uint32x4x2_t(t84[0], t84[8]));
			vst2q_u32((t6 as *mut u32).add(8), uint32x4x2_t(t84[1], t84[9]));
			vst2q_u32((t6 as *mut u32).add(16), uint32x4x2_t(t84[2], t84[10]));
			vst2q_u32((t6 as *mut u32).add(24), uint32x4x2_t(t84[3], t84[11]));
			vst2q_u32((t6 as *mut u32).add(32), uint32x4x2_t(t84[4], t84[12]));
			vst2q_u32((t6 as *mut u32).add(40), uint32x4x2_t(t84[5], t84[13]));
			vst2q_u32((t6 as *mut u32).add(48), uint32x4x2_t(t84[6], t84[14]));
			vst2q_u32((t6 as *mut u32).add(56), uint32x4x2_t(t84[7], t84[15]));
		} else {
			stj_u64(live, t84, |j| (t6 as *mut u64).add(j) as *mut u8);
		}
		let t7: [uint32x4_t; 8] = core::array::from_fn(|r| vmulhiq_u32(x6[r], x7[r]));
		let t9: [uint32x4_t; 8] = core::array::from_fn(|r| vmulq_u32(x6[r], x7[r]));
		let t10: [uint32x4_t; 8] = core::array::from_fn(|r| vmlaq_u32(vmlaq_u32(t7[r], x6[r], x7[(r + 8)]), x7[r], x6[(r + 8)]));
		let t8 = cat_w(t9, t10);
		let t11 = (arg3 as *const u8).add(((gid0.wrapping_add((9u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t11 as *mut u32).add(0), uint32x4x2_t(t8[0], t8[8]));
			vst2q_u32((t11 as *mut u32).add(8), uint32x4x2_t(t8[1], t8[9]));
			vst2q_u32((t11 as *mut u32).add(16), uint32x4x2_t(t8[2], t8[10]));
			vst2q_u32((t11 as *mut u32).add(24), uint32x4x2_t(t8[3], t8[11]));
			vst2q_u32((t11 as *mut u32).add(32), uint32x4x2_t(t8[4], t8[12]));
			vst2q_u32((t11 as *mut u32).add(40), uint32x4x2_t(t8[5], t8[13]));
			vst2q_u32((t11 as *mut u32).add(48), uint32x4x2_t(t8[6], t8[14]));
			vst2q_u32((t11 as *mut u32).add(56), uint32x4x2_t(t8[7], t8[15]));
		} else {
			stj_u64(live, t8, |j| (t11 as *mut u64).add(j) as *mut u8);
		}
		let t12: u64 = 0u64;
		let t13 = splat((t12) as u32);
		let t85: [uint32x4_t; 8] = core::array::from_fn(|r| veq64q_u32(x7[r], x7[(r + 8)], t13[r], Z));
		let t86 = mvn_r(t85);
		let mut t87 = [Z; 16];
		let t88 = and_r(live, t86);
		let t89 = bic_r(live, t86);
		{
			let (mut t90xl, mut t90xh, mut t90yl, mut t90yh) = ([0u32; 32], [0u32; 32], [0u32; 32], [0u32; 32]);
			vst1q_u32(t90xl.as_mut_ptr().add(0), x6[0]); vst1q_u32(t90xh.as_mut_ptr().add(0), x6[8]); vst1q_u32(t90yl.as_mut_ptr().add(0), x7[0]); vst1q_u32(t90yh.as_mut_ptr().add(0), x7[8]);
			vst1q_u32(t90xl.as_mut_ptr().add(4), x6[1]); vst1q_u32(t90xh.as_mut_ptr().add(4), x6[9]); vst1q_u32(t90yl.as_mut_ptr().add(4), x7[1]); vst1q_u32(t90yh.as_mut_ptr().add(4), x7[9]);
			vst1q_u32(t90xl.as_mut_ptr().add(8), x6[2]); vst1q_u32(t90xh.as_mut_ptr().add(8), x6[10]); vst1q_u32(t90yl.as_mut_ptr().add(8), x7[2]); vst1q_u32(t90yh.as_mut_ptr().add(8), x7[10]);
			vst1q_u32(t90xl.as_mut_ptr().add(12), x6[3]); vst1q_u32(t90xh.as_mut_ptr().add(12), x6[11]); vst1q_u32(t90yl.as_mut_ptr().add(12), x7[3]); vst1q_u32(t90yh.as_mut_ptr().add(12), x7[11]);
			vst1q_u32(t90xl.as_mut_ptr().add(16), x6[4]); vst1q_u32(t90xh.as_mut_ptr().add(16), x6[12]); vst1q_u32(t90yl.as_mut_ptr().add(16), x7[4]); vst1q_u32(t90yh.as_mut_ptr().add(16), x7[12]);
			vst1q_u32(t90xl.as_mut_ptr().add(20), x6[5]); vst1q_u32(t90xh.as_mut_ptr().add(20), x6[13]); vst1q_u32(t90yl.as_mut_ptr().add(20), x7[5]); vst1q_u32(t90yh.as_mut_ptr().add(20), x7[13]);
			vst1q_u32(t90xl.as_mut_ptr().add(24), x6[6]); vst1q_u32(t90xh.as_mut_ptr().add(24), x6[14]); vst1q_u32(t90yl.as_mut_ptr().add(24), x7[6]); vst1q_u32(t90yh.as_mut_ptr().add(24), x7[14]);
			vst1q_u32(t90xl.as_mut_ptr().add(28), x6[7]); vst1q_u32(t90xh.as_mut_ptr().add(28), x6[15]); vst1q_u32(t90yl.as_mut_ptr().add(28), x7[7]); vst1q_u32(t90yh.as_mut_ptr().add(28), x7[15]);
			for j in 0..32 { let q = (((t90xh[j] as u64) << 32 | t90xl[j] as u64) as u64).checked_div(((t90yh[j] as u64) << 32 | t90yl[j] as u64) as u64).unwrap_or(0) as u64; t90xl[j] = q as u32; t90xh[j] = (q >> 32) as u32; }
			let t91: [uint32x4_t; 8] = [vld1q_u32(t90xl.as_ptr().add(0)), vld1q_u32(t90xl.as_ptr().add(4)), vld1q_u32(t90xl.as_ptr().add(8)), vld1q_u32(t90xl.as_ptr().add(12)), vld1q_u32(t90xl.as_ptr().add(16)), vld1q_u32(t90xl.as_ptr().add(20)), vld1q_u32(t90xl.as_ptr().add(24)), vld1q_u32(t90xl.as_ptr().add(28))];
			let t92: [uint32x4_t; 8] = [vld1q_u32(t90xh.as_ptr().add(0)), vld1q_u32(t90xh.as_ptr().add(4)), vld1q_u32(t90xh.as_ptr().add(8)), vld1q_u32(t90xh.as_ptr().add(12)), vld1q_u32(t90xh.as_ptr().add(16)), vld1q_u32(t90xh.as_ptr().add(20)), vld1q_u32(t90xh.as_ptr().add(24)), vld1q_u32(t90xh.as_ptr().add(28))];
			let t93 = cat_w(t91, t92);
			t87 = bsl_w(t88, t93, t87);
		}
		{
			let t94 = splat64(7u64);
			t87 = bsl_w(t89, t94, t87);
		}
		let t95 = (arg3 as *const u8).add(((gid0.wrapping_add((10u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t95 as *mut u32).add(0), uint32x4x2_t(t87[0], t87[8]));
			vst2q_u32((t95 as *mut u32).add(8), uint32x4x2_t(t87[1], t87[9]));
			vst2q_u32((t95 as *mut u32).add(16), uint32x4x2_t(t87[2], t87[10]));
			vst2q_u32((t95 as *mut u32).add(24), uint32x4x2_t(t87[3], t87[11]));
			vst2q_u32((t95 as *mut u32).add(32), uint32x4x2_t(t87[4], t87[12]));
			vst2q_u32((t95 as *mut u32).add(40), uint32x4x2_t(t87[5], t87[13]));
			vst2q_u32((t95 as *mut u32).add(48), uint32x4x2_t(t87[6], t87[14]));
			vst2q_u32((t95 as *mut u32).add(56), uint32x4x2_t(t87[7], t87[15]));
		} else {
			stj_u64(live, t87, |j| (t95 as *mut u64).add(j) as *mut u8);
		}
		let t96: [uint32x4_t; 8] = core::array::from_fn(|r| vlt64q_u32(x6[r], x6[(r + 8)], x7[r], x7[(r + 8)]));
		let t97: [uint32x4_t; 8] = core::array::from_fn(|r| vshrq_n_u32::<31>(t96[r]));
		let t98: [uint32x4_t; 8] = core::array::from_fn(|r| vlt64q_u32(x7[r], x7[(r + 8)], x6[r], x6[(r + 8)]));
		let t99 = mvn_r(t98);
		let t100: [uint32x4_t; 8] = core::array::from_fn(|r| vshrq_n_u32::<31>(t99[r]));
		let t101: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_n_u32::<1>(t100[r]));
		let t102: [uint32x4_t; 8] = core::array::from_fn(|r| vshrq_n_u32::<31>(t100[r]));
		let t103 = cat_w(t101, t102);
		let t104 = orr_r(t97, lo(t103));
		let t105 = cat_w(t104, hi(t103));
		let t106: [uint32x4_t; 8] = core::array::from_fn(|r| veq64q_u32(x6[r], x6[(r + 8)], x7[r], x7[(r + 8)]));
		let t107: [uint32x4_t; 8] = core::array::from_fn(|r| vshrq_n_u32::<31>(t106[r]));
		let t108: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_n_u32::<2>(t107[r]));
		let t109: [uint32x4_t; 8] = core::array::from_fn(|r| vshrq_n_u32::<30>(t107[r]));
		let t110 = cat_w(t108, t109);
		let t111: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(t105[r], t110[r]));
		let t112: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(t105[(r + 8)], t110[(r + 8)]));
		let t113 = cat_w(t111, t112);
		let t114: [uint32x4_t; 8] = core::array::from_fn(|r| veq64q_u32(x6[r], x6[(r + 8)], x7[r], x7[(r + 8)]));
		let t115 = mvn_r(t114);
		let t116: [uint32x4_t; 8] = core::array::from_fn(|r| vshrq_n_u32::<31>(t115[r]));
		let t117: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_n_u32::<3>(t116[r]));
		let t118: [uint32x4_t; 8] = core::array::from_fn(|r| vshrq_n_u32::<29>(t116[r]));
		let t119 = cat_w(t117, t118);
		let t120: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(t113[r], t119[r]));
		let t121: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(t113[(r + 8)], t119[(r + 8)]));
		let t122 = cat_w(t120, t121);
		let t123: [uint32x4_t; 8] = core::array::from_fn(|r| vlt64sq_u32(x6[r], x6[(r + 8)], x7[r], x7[(r + 8)]));
		let t124: [uint32x4_t; 8] = core::array::from_fn(|r| vshrq_n_u32::<31>(t123[r]));
		let t125: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_n_u32::<4>(t124[r]));
		let t19: [uint32x4_t; 8] = core::array::from_fn(|r| vshrq_n_u32::<28>(t124[r]));
		let t126 = cat_w(t125, t19);
		let t21: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(t122[r], t126[r]));
		let t22: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(t122[(r + 8)], t126[(r + 8)]));
		let t20 = cat_w(t21, t22);
		let t23: [uint32x4_t; 8] = core::array::from_fn(|r| vlt64sq_u32(x6[r], x6[(r + 8)], x7[r], x7[(r + 8)]));
		let t24 = mvn_r(t23);
		let t25: [uint32x4_t; 8] = core::array::from_fn(|r| vshrq_n_u32::<31>(t24[r]));
		let t127: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_n_u32::<5>(t25[r]));
		let t128: [uint32x4_t; 8] = core::array::from_fn(|r| vshrq_n_u32::<27>(t25[r]));
		let t26 = cat_w(t127, t128);
		let t129: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(t20[r], t26[r]));
		let t130: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(t20[(r + 8)], t26[(r + 8)]));
		let t131 = cat_w(t129, t130);
		let t132: [uint32x4_t; 8] = core::array::from_fn(|r| vlt64q_u32(x7[r], x7[(r + 8)], x6[r], x6[(r + 8)]));
		let t133: [uint32x4_t; 8] = core::array::from_fn(|r| vshrq_n_u32::<31>(t132[r]));
		let t134: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_n_u32::<6>(t133[r]));
		let t135: [uint32x4_t; 8] = core::array::from_fn(|r| vshrq_n_u32::<26>(t133[r]));
		let t136 = cat_w(t134, t135);
		let t137: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(t131[r], t136[r]));
		let t138: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(t131[(r + 8)], t136[(r + 8)]));
		let t139 = cat_w(t137, t138);
		let t140 = (arg3 as *const u8).add(((gid0.wrapping_add((11u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t140 as *mut u32).add(0), uint32x4x2_t(t139[0], t139[8]));
			vst2q_u32((t140 as *mut u32).add(8), uint32x4x2_t(t139[1], t139[9]));
			vst2q_u32((t140 as *mut u32).add(16), uint32x4x2_t(t139[2], t139[10]));
			vst2q_u32((t140 as *mut u32).add(24), uint32x4x2_t(t139[3], t139[11]));
			vst2q_u32((t140 as *mut u32).add(32), uint32x4x2_t(t139[4], t139[12]));
			vst2q_u32((t140 as *mut u32).add(40), uint32x4x2_t(t139[5], t139[13]));
			vst2q_u32((t140 as *mut u32).add(48), uint32x4x2_t(t139[6], t139[14]));
			vst2q_u32((t140 as *mut u32).add(56), uint32x4x2_t(t139[7], t139[15]));
		} else {
			stj_u64(live, t139, |j| (t140 as *mut u64).add(j) as *mut u8);
		}
		let t141 = add_r(x8, lo(x6));
		let t142 = clt_r(t141, x8);
		let t143: [uint32x4_t; 8] = core::array::from_fn(|r| vadcq_u32(Z, x6[(r + 8)], t142[r]));
		let t144 = cat_w(lo(t141), t143);
		let t145 = (arg3 as *const u8).add(((gid0.wrapping_add((12u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t145 as *mut u32).add(0), uint32x4x2_t(t144[0], t144[8]));
			vst2q_u32((t145 as *mut u32).add(8), uint32x4x2_t(t144[1], t144[9]));
			vst2q_u32((t145 as *mut u32).add(16), uint32x4x2_t(t144[2], t144[10]));
			vst2q_u32((t145 as *mut u32).add(24), uint32x4x2_t(t144[3], t144[11]));
			vst2q_u32((t145 as *mut u32).add(32), uint32x4x2_t(t144[4], t144[12]));
			vst2q_u32((t145 as *mut u32).add(40), uint32x4x2_t(t144[5], t144[13]));
			vst2q_u32((t145 as *mut u32).add(48), uint32x4x2_t(t144[6], t144[14]));
			vst2q_u32((t145 as *mut u32).add(56), uint32x4x2_t(t144[7], t144[15]));
		} else {
			stj_u64(live, t144, |j| (t145 as *mut u64).add(j) as *mut u8);
		}
		let t146: [uint32x4_t; 8] = core::array::from_fn(|r| vreinterpretq_u32_s32(vshrq_n_s32::<31>(vreinterpretq_s32_u32(x8[r]))));
		let t147 = cat_w(lo(x8), lo(t146));
		let t148 = (arg3 as *const u8).add(((gid0.wrapping_add((13u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t148 as *mut u32).add(0), uint32x4x2_t(t147[0], t147[8]));
			vst2q_u32((t148 as *mut u32).add(8), uint32x4x2_t(t147[1], t147[9]));
			vst2q_u32((t148 as *mut u32).add(16), uint32x4x2_t(t147[2], t147[10]));
			vst2q_u32((t148 as *mut u32).add(24), uint32x4x2_t(t147[3], t147[11]));
			vst2q_u32((t148 as *mut u32).add(32), uint32x4x2_t(t147[4], t147[12]));
			vst2q_u32((t148 as *mut u32).add(40), uint32x4x2_t(t147[5], t147[13]));
			vst2q_u32((t148 as *mut u32).add(48), uint32x4x2_t(t147[6], t147[14]));
			vst2q_u32((t148 as *mut u32).add(56), uint32x4x2_t(t147[7], t147[15]));
		} else {
			stj_u64(live, t147, |j| (t148 as *mut u64).add(j) as *mut u8);
		}
		let t149: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(vshrq_n_u32::<3>(x6[r]), vshlq_n_u32::<29>(x6[(r + 8)])));
		let t150: [uint32x4_t; 8] = core::array::from_fn(|r| vshrq_n_u32::<3>(x6[(r + 8)]));
		let t151 = cat_w(t149, t150);
		let t152: [uint32x4_t; 8] = lo(t151);
		let t153 = cat_w(lo(t152), [Z; 8]);
		let t154 = (arg3 as *const u8).add(((gid0.wrapping_add((14u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t154 as *mut u32).add(0), uint32x4x2_t(t153[0], t153[8]));
			vst2q_u32((t154 as *mut u32).add(8), uint32x4x2_t(t153[1], t153[9]));
			vst2q_u32((t154 as *mut u32).add(16), uint32x4x2_t(t153[2], t153[10]));
			vst2q_u32((t154 as *mut u32).add(24), uint32x4x2_t(t153[3], t153[11]));
			vst2q_u32((t154 as *mut u32).add(32), uint32x4x2_t(t153[4], t153[12]));
			vst2q_u32((t154 as *mut u32).add(40), uint32x4x2_t(t153[5], t153[13]));
			vst2q_u32((t154 as *mut u32).add(48), uint32x4x2_t(t153[6], t153[14]));
			vst2q_u32((t154 as *mut u32).add(56), uint32x4x2_t(t153[7], t153[15]));
		} else {
			stj_u64(live, t153, |j| (t154 as *mut u64).add(j) as *mut u8);
		}
		let t155: u64 = 1u64;
		let t156 = splat((t155) as u32);
		let t157 = and_r(lo(x6), t156);
		let t158 = tst_r(t157, t157);
		let t159 = and_r(live, t158);
		{
			let t160: [uint32x4_t; 8] = core::array::from_fn(|r| veorq_u32(x6[r], x7[r]));
			let t161: [uint32x4_t; 8] = core::array::from_fn(|r| veorq_u32(x6[(r + 8)], x7[(r + 8)]));
			let t162 = cat_w(t160, t161);
			x11 = bsl_w(t159, t162, x11);
		}
		let t163 = bic_r(live, t158);
		{
			let t164 = add_r(lo(x6), x8);
			let t165 = clt_r(t164, lo(x6));
			let t166: [uint32x4_t; 8] = core::array::from_fn(|r| vadcq_u32(x6[(r + 8)], Z, t165[r]));
			let t167 = cat_w(lo(t164), t166);
			x11 = bsl_w(t163, t167, x11);
		}
		let t168 = (arg3 as *const u8).add(((gid0.wrapping_add((15u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t168 as *mut u32).add(0), uint32x4x2_t(x11[0], x11[8]));
			vst2q_u32((t168 as *mut u32).add(8), uint32x4x2_t(x11[1], x11[9]));
			vst2q_u32((t168 as *mut u32).add(16), uint32x4x2_t(x11[2], x11[10]));
			vst2q_u32((t168 as *mut u32).add(24), uint32x4x2_t(x11[3], x11[11]));
			vst2q_u32((t168 as *mut u32).add(32), uint32x4x2_t(x11[4], x11[12]));
			vst2q_u32((t168 as *mut u32).add(40), uint32x4x2_t(x11[5], x11[13]));
			vst2q_u32((t168 as *mut u32).add(48), uint32x4x2_t(x11[6], x11[14]));
			vst2q_u32((t168 as *mut u32).add(56), uint32x4x2_t(x11[7], x11[15]));
		} else {
			stj_u64(live, x11, |j| (t168 as *mut u64).add(j) as *mut u8);
		}
		let t169 = b1_l_p(l, lid0, live, x6);
		let t170 = b2_l_p(l, lid0, live, x7);
		let t171: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_n_u32::<8>(t170[r]));
		let t172: [uint32x4_t; 8] = core::array::from_fn(|r| vshrq_n_u32::<24>(t170[r]));
		let t173 = cat_w(t171, t172);
		let t174 = add_r(t169, lo(t173));
		let t175 = clt_r(t174, t169);
		let t176: [uint32x4_t; 8] = core::array::from_fn(|r| vadcq_u32(Z, t173[(r + 8)], t175[r]));
		let t177 = cat_w(lo(t174), t176);
		let t178 = (arg3 as *const u8).add(((gid0.wrapping_add((16u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t178 as *mut u32).add(0), uint32x4x2_t(t177[0], t177[8]));
			vst2q_u32((t178 as *mut u32).add(8), uint32x4x2_t(t177[1], t177[9]));
			vst2q_u32((t178 as *mut u32).add(16), uint32x4x2_t(t177[2], t177[10]));
			vst2q_u32((t178 as *mut u32).add(24), uint32x4x2_t(t177[3], t177[11]));
			vst2q_u32((t178 as *mut u32).add(32), uint32x4x2_t(t177[4], t177[12]));
			vst2q_u32((t178 as *mut u32).add(40), uint32x4x2_t(t177[5], t177[13]));
			vst2q_u32((t178 as *mut u32).add(48), uint32x4x2_t(t177[6], t177[14]));
			vst2q_u32((t178 as *mut u32).add(56), uint32x4x2_t(t177[7], t177[15]));
		} else {
			stj_u64(live, t177, |j| (t178 as *mut u64).add(j) as *mut u8);
		}
		let t179 = splat(x10);
		let mut t180x = [0u32; 32];
		let mut t180y = [0u32; 32];
		for r in 0..8 { vst1q_u32(t180x.as_mut_ptr().add(4 * r), x8[r]); vst1q_u32(t180y.as_mut_ptr().add(4 * r), t179[r]); }
		for j in 0..32 { t180x[j] = (t180x[j] as u32).checked_rem(t180y[j] as u32).unwrap_or(0) as u32; }
		let t181: [uint32x4_t; 8] = [vld1q_u32(t180x.as_ptr().add(0)), vld1q_u32(t180x.as_ptr().add(4)), vld1q_u32(t180x.as_ptr().add(8)), vld1q_u32(t180x.as_ptr().add(12)), vld1q_u32(t180x.as_ptr().add(16)), vld1q_u32(t180x.as_ptr().add(20)), vld1q_u32(t180x.as_ptr().add(24)), vld1q_u32(t180x.as_ptr().add(28))];
		let t182 = ix_on(live, t181);
		let t183 = ld_u64(arg0 as *const u8, t182, 8);
		let t184 = (arg3 as *const u8).add(((gid0.wrapping_add((17u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t184 as *mut u32).add(0), uint32x4x2_t(t183[0], t183[8]));
			vst2q_u32((t184 as *mut u32).add(8), uint32x4x2_t(t183[1], t183[9]));
			vst2q_u32((t184 as *mut u32).add(16), uint32x4x2_t(t183[2], t183[10]));
			vst2q_u32((t184 as *mut u32).add(24), uint32x4x2_t(t183[3], t183[11]));
			vst2q_u32((t184 as *mut u32).add(32), uint32x4x2_t(t183[4], t183[12]));
			vst2q_u32((t184 as *mut u32).add(40), uint32x4x2_t(t183[5], t183[13]));
			vst2q_u32((t184 as *mut u32).add(48), uint32x4x2_t(t183[6], t183[14]));
			vst2q_u32((t184 as *mut u32).add(56), uint32x4x2_t(t183[7], t183[15]));
		} else {
			stj_u64(live, t183, |j| (t184 as *mut u64).add(j) as *mut u8);
		}
		let t185 = b3_ll_p(l, lid0, live, x6, x8);
		let t186 = (arg3 as *const u8).add(((gid0.wrapping_add((18u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t186 as *mut u32).add(0), uint32x4x2_t(t185[0], t185[8]));
			vst2q_u32((t186 as *mut u32).add(8), uint32x4x2_t(t185[1], t185[9]));
			vst2q_u32((t186 as *mut u32).add(16), uint32x4x2_t(t185[2], t185[10]));
			vst2q_u32((t186 as *mut u32).add(24), uint32x4x2_t(t185[3], t185[11]));
			vst2q_u32((t186 as *mut u32).add(32), uint32x4x2_t(t185[4], t185[12]));
			vst2q_u32((t186 as *mut u32).add(40), uint32x4x2_t(t185[5], t185[13]));
			vst2q_u32((t186 as *mut u32).add(48), uint32x4x2_t(t185[6], t185[14]));
			vst2q_u32((t186 as *mut u32).add(56), uint32x4x2_t(t185[7], t185[15]));
		} else {
			stj_u64(live, t185, |j| (t186 as *mut u64).add(j) as *mut u8);
		}
		x12 = x6;
		x13 = 0u32;
		let mut t187 = live;
		loop {
			let t188 = bits(&t187);
			if t188.count_ones() <= 2 {
				let mut t189 = t188;
				while t189 != 0 {
					let j = t189.trailing_zeros() as usize;
					t189 &= t189 - 1;
					let t190 = words16(x7);
					let mut x7_j: u64 = ((t190[32 + j] as u64) << 32 | t190[j] as u64);
					let t191 = words8(x8);
					let mut x8_j: u32 = t191[j];
					let t192 = words16(x12);
					let mut x12_j: u64 = ((t192[32 + j] as u64) << 32 | t192[j] as u64);
					let mut x13_j: u32 = x13;
					loop {
						if !(((x13_j) < (((x8_j) & (7u32))))) { break; }
						x12_j = ((x12_j).wrapping_mul(3u64)).wrapping_add(x7_j);
						let t193 = x13_j;
						x13_j = (x13_j).wrapping_add(1u32);
					}
					let mut t194 = words16(x12);
					t194[j] = x12_j as u32;
					t194[32 + j] = (x12_j as u64 >> 32) as u32;
					x12 = regs16(t194);
				}
				break;
			}
			// Lanes part: each its own rounds.
			let t195 = splat(7u32);
			let t196 = and_r(x8, t195);
			let t197 = splat(x13);
			let t198 = clt_r(t197, t196);
			t187 = and_r(t187, t198);
			if !(any(&t187)) { break; }
			let t199: u64 = 3u64;
			let t200 = splat((t199) as u32);
			let t201 = mulhi_r(lo(x12), t200);
			let t202 = mul_r(lo(x12), t200);
			let t203 = mla_r(t201, t200, hi(x12));
			let t204 = cat_w(t202, t203);
			let t205: [uint32x4_t; 8] = core::array::from_fn(|r| vaddq_u32(t204[r], x7[r]));
			let t206 = clt_r(t205, lo(t204));
			let t207 = adc_r(hi(t204), hi(x7), t206);
			let t208 = cat_w(lo(t205), t207);
			x12 = bsl_w(t187, t208, x12);
			let t209 = and_r(t187, live);
			let t210 = x13;
			x13 = (x13).wrapping_add(1u32);
		}
		let t211 = (arg3 as *const u8).add(((gid0.wrapping_add((19u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t211 as *mut u32).add(0), uint32x4x2_t(x12[0], x12[8]));
			vst2q_u32((t211 as *mut u32).add(8), uint32x4x2_t(x12[1], x12[9]));
			vst2q_u32((t211 as *mut u32).add(16), uint32x4x2_t(x12[2], x12[10]));
			vst2q_u32((t211 as *mut u32).add(24), uint32x4x2_t(x12[3], x12[11]));
			vst2q_u32((t211 as *mut u32).add(32), uint32x4x2_t(x12[4], x12[12]));
			vst2q_u32((t211 as *mut u32).add(40), uint32x4x2_t(x12[5], x12[13]));
			vst2q_u32((t211 as *mut u32).add(48), uint32x4x2_t(x12[6], x12[14]));
			vst2q_u32((t211 as *mut u32).add(56), uint32x4x2_t(x12[7], x12[15]));
		} else {
			stj_u64(live, x12, |j| (t211 as *mut u64).add(j) as *mut u8);
		}
		let t212: u64 = a1;
		let t213 = splat((t212) as u32);
		let t214 = splat(((t212) >> 32) as u32);
		let t215 = add_r(lo(x6), t213);
		let t216 = clt_r(t215, lo(x6));
		let t217 = adc_r(hi(x6), t214, t216);
		let t218 = cat_w(lo(t215), t217);
		let t219: u64 = (a1).wrapping_shl((3u64) as u32);
		let t220 = splat((t219) as u32);
		let t221 = splat(((t219) >> 32) as u32);
		let t222 = add_r(lo(t218), t220);
		let t223 = clt_r(t222, lo(t218));
		let t224 = adc_r(hi(t218), t221, t223);
		let t225 = cat_w(lo(t222), t224);
		let t226 = (arg3 as *const u8).add(((gid0.wrapping_add((20u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t226 as *mut u32).add(0), uint32x4x2_t(t225[0], t225[8]));
			vst2q_u32((t226 as *mut u32).add(8), uint32x4x2_t(t225[1], t225[9]));
			vst2q_u32((t226 as *mut u32).add(16), uint32x4x2_t(t225[2], t225[10]));
			vst2q_u32((t226 as *mut u32).add(24), uint32x4x2_t(t225[3], t225[11]));
			vst2q_u32((t226 as *mut u32).add(32), uint32x4x2_t(t225[4], t225[12]));
			vst2q_u32((t226 as *mut u32).add(40), uint32x4x2_t(t225[5], t225[13]));
			vst2q_u32((t226 as *mut u32).add(48), uint32x4x2_t(t225[6], t225[14]));
			vst2q_u32((t226 as *mut u32).add(56), uint32x4x2_t(t225[7], t225[15]));
		} else {
			stj_u64(live, t225, |j| (t226 as *mut u64).add(j) as *mut u8);
		}
		let t227: u64 = 4294967295u64;
		let t228 = splat((t227) as u32);
		let t229 = and_r(lo(x6), t228);
		let t230: u64 = 4294967295u64;
		let t231 = splat((t230) as u32);
		let t232 = and_r(lo(x7), t231);
		let t233 = ceq_r(t229, t232);
		let mut t234 = [Z; 8];
		let t235 = and_r(live, t233);
		let t236 = bic_r(live, t233);
		{
			let t237 = splat(1u32);
			t234 = bsl_r(t235, t237, t234);
		}
		{
			let t238 = splat(2u32);
			t234 = bsl_r(t236, t238, t234);
		}
		let t239 = cat_w(lo(t234), [Z; 8]);
		let t240 = (arg3 as *const u8).add(((gid0.wrapping_add((21u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t240 as *mut u32).add(0), uint32x4x2_t(t239[0], t239[8]));
			vst2q_u32((t240 as *mut u32).add(8), uint32x4x2_t(t239[1], t239[9]));
			vst2q_u32((t240 as *mut u32).add(16), uint32x4x2_t(t239[2], t239[10]));
			vst2q_u32((t240 as *mut u32).add(24), uint32x4x2_t(t239[3], t239[11]));
			vst2q_u32((t240 as *mut u32).add(32), uint32x4x2_t(t239[4], t239[12]));
			vst2q_u32((t240 as *mut u32).add(40), uint32x4x2_t(t239[5], t239[13]));
			vst2q_u32((t240 as *mut u32).add(48), uint32x4x2_t(t239[6], t239[14]));
			vst2q_u32((t240 as *mut u32).add(56), uint32x4x2_t(t239[7], t239[15]));
		} else {
			stj_u64(live, t239, |j| (t240 as *mut u64).add(j) as *mut u8);
		}
		let t241: [uint32x4_t; 8] = core::array::from_fn(|r| vtstq_u32(vorrq_u32(x6[r], x6[(r + 8)]), vorrq_u32(x6[r], x6[(r + 8)])));
		let mut t242 = [Z; 8];
		let t243 = and_r(live, t241);
		let t244 = bic_r(live, t241);
		{
			let t245 = splat(5u32);
			t242 = bsl_r(t243, t245, t242);
		}
		{
			let t246 = splat(6u32);
			t242 = bsl_r(t244, t246, t242);
		}
		let t247 = cat_w(lo(t242), [Z; 8]);
		let t248 = (arg3 as *const u8).add(((gid0.wrapping_add((22u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t248 as *mut u32).add(0), uint32x4x2_t(t247[0], t247[8]));
			vst2q_u32((t248 as *mut u32).add(8), uint32x4x2_t(t247[1], t247[9]));
			vst2q_u32((t248 as *mut u32).add(16), uint32x4x2_t(t247[2], t247[10]));
			vst2q_u32((t248 as *mut u32).add(24), uint32x4x2_t(t247[3], t247[11]));
			vst2q_u32((t248 as *mut u32).add(32), uint32x4x2_t(t247[4], t247[12]));
			vst2q_u32((t248 as *mut u32).add(40), uint32x4x2_t(t247[5], t247[13]));
			vst2q_u32((t248 as *mut u32).add(48), uint32x4x2_t(t247[6], t247[14]));
			vst2q_u32((t248 as *mut u32).add(56), uint32x4x2_t(t247[7], t247[15]));
		} else {
			stj_u64(live, t247, |j| (t248 as *mut u64).add(j) as *mut u8);
		}
		let t249 = b4_ll_p(l, lid0, live, x6, x7);
		let t250 = (arg3 as *const u8).add(((gid0.wrapping_add((23u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t250 as *mut u32).add(0), uint32x4x2_t(t249[0], t249[8]));
			vst2q_u32((t250 as *mut u32).add(8), uint32x4x2_t(t249[1], t249[9]));
			vst2q_u32((t250 as *mut u32).add(16), uint32x4x2_t(t249[2], t249[10]));
			vst2q_u32((t250 as *mut u32).add(24), uint32x4x2_t(t249[3], t249[11]));
			vst2q_u32((t250 as *mut u32).add(32), uint32x4x2_t(t249[4], t249[12]));
			vst2q_u32((t250 as *mut u32).add(40), uint32x4x2_t(t249[5], t249[13]));
			vst2q_u32((t250 as *mut u32).add(48), uint32x4x2_t(t249[6], t249[14]));
			vst2q_u32((t250 as *mut u32).add(56), uint32x4x2_t(t249[7], t249[15]));
		} else {
			stj_u64(live, t249, |j| (t250 as *mut u64).add(j) as *mut u8);
		}
		let t251: u32;
		if ((((a1) & (1u64))) != (0u64)) {
			t251 = 8u32;
		} else {
			t251 = 4u32;
		}
		x14 = t251;
		let t252 = b5_r0ns_p(l, lid0, live, arg0, gid0.wrapping_mul(x14), x14, 8u32);
		let t253 = (arg3 as *const u8).add(((gid0.wrapping_add((24u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t253 as *mut u32).add(0), uint32x4x2_t(t252[0], t252[8]));
			vst2q_u32((t253 as *mut u32).add(8), uint32x4x2_t(t252[1], t252[9]));
			vst2q_u32((t253 as *mut u32).add(16), uint32x4x2_t(t252[2], t252[10]));
			vst2q_u32((t253 as *mut u32).add(24), uint32x4x2_t(t252[3], t252[11]));
			vst2q_u32((t253 as *mut u32).add(32), uint32x4x2_t(t252[4], t252[12]));
			vst2q_u32((t253 as *mut u32).add(40), uint32x4x2_t(t252[5], t252[13]));
			vst2q_u32((t253 as *mut u32).add(48), uint32x4x2_t(t252[6], t252[14]));
			vst2q_u32((t253 as *mut u32).add(56), uint32x4x2_t(t252[7], t252[15]));
		} else {
			stj_u64(live, t252, |j| (t253 as *mut u64).add(j) as *mut u8);
		}
		let t254 = if (gid0.wrapping_add(3u32) & 255u32) <= 224u32 { b6_u8_p(l, lid0, live, gid0.wrapping_add(3u32)) } else { b6_i_p(l, lid0, live, gid0.wrapping_add(3u32)) };
		let t255 = (arg3 as *const u8).add(((gid0.wrapping_add((25u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t255 as *mut u32).add(0), uint32x4x2_t(t254[0], t254[8]));
			vst2q_u32((t255 as *mut u32).add(8), uint32x4x2_t(t254[1], t254[9]));
			vst2q_u32((t255 as *mut u32).add(16), uint32x4x2_t(t254[2], t254[10]));
			vst2q_u32((t255 as *mut u32).add(24), uint32x4x2_t(t254[3], t254[11]));
			vst2q_u32((t255 as *mut u32).add(32), uint32x4x2_t(t254[4], t254[12]));
			vst2q_u32((t255 as *mut u32).add(40), uint32x4x2_t(t254[5], t254[13]));
			vst2q_u32((t255 as *mut u32).add(48), uint32x4x2_t(t254[6], t254[14]));
			vst2q_u32((t255 as *mut u32).add(56), uint32x4x2_t(t254[7], t254[15]));
		} else {
			stj_u64(live, t254, |j| (t255 as *mut u64).add(j) as *mut u8);
		}
		let t256: u64 = 3u64;
		let t257 = splat((t256) as u32);
		let t258 = and_r(lo(x6), t257);
		let t259: u64 = 0u64;
		let t260 = splat((t259) as u32);
		let t261 = ceq_r(t258, t260);
		let mut t262 = [Z; 16];
		let t263 = and_r(live, t261);
		let t264 = bic_r(live, t261);
		if any(&t263) {
			let t265: [uint32x4_t; 16];
			let t266 = bits(&t263);
			if t266.count_ones() <= 2 {
				let t267 = words16(x7);
				let t268 = words8(x8);
				let mut t269 = [0u32; 64];
				let mut t270 = t266;
				while t270 != 0 {
					let j = t270.trailing_zeros() as usize;
					t270 &= t270 - 1;
					let t271 = b7_ss_f(l, lid0, ((t267[32 + j] as u64) << 32 | t267[j] as u64), t268[j]);
					let t272: u64 = t271;
					t269[j] = t272 as u32;
					t269[32 + j] = (t272 >> 32) as u32;
				}
				t265 = regs16(t269);
			} else {
				let t273 = b7_ll_p(l, lid0, t263, x7, x8);
				t265 = t273;
			}
			t262 = bsl_w(t263, t265, t262);
		}
		{
			t262 = bsl_w(t264, x6, t262);
		}
		let t274 = (arg3 as *const u8).add(((gid0.wrapping_add((26u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t274 as *mut u32).add(0), uint32x4x2_t(t262[0], t262[8]));
			vst2q_u32((t274 as *mut u32).add(8), uint32x4x2_t(t262[1], t262[9]));
			vst2q_u32((t274 as *mut u32).add(16), uint32x4x2_t(t262[2], t262[10]));
			vst2q_u32((t274 as *mut u32).add(24), uint32x4x2_t(t262[3], t262[11]));
			vst2q_u32((t274 as *mut u32).add(32), uint32x4x2_t(t262[4], t262[12]));
			vst2q_u32((t274 as *mut u32).add(40), uint32x4x2_t(t262[5], t262[13]));
			vst2q_u32((t274 as *mut u32).add(48), uint32x4x2_t(t262[6], t262[14]));
			vst2q_u32((t274 as *mut u32).add(56), uint32x4x2_t(t262[7], t262[15]));
		} else {
			stj_u64(live, t262, |j| (t274 as *mut u64).add(j) as *mut u8);
		}
		x15 = (((((a1) & (7u64))) as u32)).wrapping_add(1u32);
		let t275 = b5_r0ns_p(l, lid0, live, arg0, gid0.wrapping_mul(x15), x15, x15);
		let t276 = b8_s_p(l, lid0, live, (8u32).wrapping_mul(x15));
		let t277: u64 = t276;
		let t278 = splat((t277) as u32);
		let t279 = splat(((t277) >> 32) as u32);
		let t280 = and_r(lo(t275), t278);
		let t281 = and_r(hi(t275), t279);
		let t282 = cat_w(t280, t281);
		let t283 = (arg3 as *const u8).add(((gid0.wrapping_add((27u32).wrapping_mul(x10))) as usize) * 8);
		if all(&live) {
			vst2q_u32((t283 as *mut u32).add(0), uint32x4x2_t(t282[0], t282[8]));
			vst2q_u32((t283 as *mut u32).add(8), uint32x4x2_t(t282[1], t282[9]));
			vst2q_u32((t283 as *mut u32).add(16), uint32x4x2_t(t282[2], t282[10]));
			vst2q_u32((t283 as *mut u32).add(24), uint32x4x2_t(t282[3], t282[11]));
			vst2q_u32((t283 as *mut u32).add(32), uint32x4x2_t(t282[4], t282[12]));
			vst2q_u32((t283 as *mut u32).add(40), uint32x4x2_t(t282[5], t282[13]));
			vst2q_u32((t283 as *mut u32).add(48), uint32x4x2_t(t282[6], t282[14]));
			vst2q_u32((t283 as *mut u32).add(56), uint32x4x2_t(t282[7], t282[15]));
		} else {
			stj_u64(live, t282, |j| (t283 as *mut u64).add(j) as *mut u8);
		}
	}
}

/// `ukl_popcount64`.
#[inline(always)]
unsafe fn b1_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16]) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0: [uint32x4_t; 8] = core::array::from_fn(|r| vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[r])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[(r + 8)]))))));
	res = t0;
	return res;
}

/// `ukl_clz64`.
#[inline(always)]
unsafe fn b2_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16]) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0: [uint32x4_t; 8] = core::array::from_fn(|r| vbslq_u32(vceqzq_u32(x0[(r + 8)]), vaddq_u32(vclzq_u32(x0[r]), vdupq_n_u32(32)), vclzq_u32(x0[(r + 8)])));
	res = t0;
	return res;
}

/// `wide_step`.
#[inline(always)]
unsafe fn b3_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], x1: [uint32x4_t; 8]) -> [uint32x4_t; 16] {
	let mut x2 = [Z; 8];
	let mut live = k;
	let mut res = [Z; 16];
	let t0: u64 = 1u64;
	let t1 = splat((t0) as u32);
	let t2 = and_r(lo(x0), t1);
	let t3: u64 = 0u64;
	let t4 = splat((t3) as u32);
	let t5 = ceq_r(t2, t4);
	let t6 = and_r(live, t5);
	{
		let t7 = cat_w(lo(x1), [Z; 8]);
		res = bsl_w(t6, t7, res);
		live = bic_r(live, t6);
	}
	if any(&live) {
		let t8: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(vshrq_n_u32::<2>(x0[r]), vshlq_n_u32::<30>(x0[(r + 8)])));
		let t9: [uint32x4_t; 8] = core::array::from_fn(|r| vshrq_n_u32::<2>(x0[(r + 8)]));
		let t10 = cat_w(t8, t9);
		let t11: [uint32x4_t; 8] = lo(t10);
		x2 = t11;
		let t12 = splat(1u32);
		let t13 = sub_r(x2, t12);
		let t14 = cge_r(t13, x1);
		let t15 = and_r(live, t14);
		{
			let t16 = splat64(4294967295u64);
			res = bsl_w(t15, t16, res);
			live = bic_r(live, t15);
		}
		if any(&live) {
			let t17: u64 = 2u64;
			let t18 = splat((t17) as u32);
			let t19 = and_r(lo(x0), t18);
			let t20: u64 = 0u64;
			let t21 = splat((t20) as u32);
			let t22 = ceq_r(t19, t21);
			let t23 = mvn_r(t22);
			let t24 = and_r(live, t23);
			{
				let t25 = sub_r(x1, x2);
				let t26 = cat_w(lo(t25), [Z; 8]);
				res = bsl_w(t24, t26, res);
				live = bic_r(live, t24);
			}
			if any(&live) {
				let t27 = sub_r(x1, x2);
				let t28: u64 = (1u64).wrapping_shl((32u64) as u32);
				let t29 = splat((t28) as u32);
				let t30 = splat(((t28) >> 32) as u32);
				let t31 = orr_r(t29, t27);
				let t32 = cat_w(t31, lo(t30));
				res = bsl_w(live, t32, res);
				live = bic_r(live, live);
			}
		}
	}
	res
}

/// `ukl_umulhi64`.
#[inline(always)]
unsafe fn b4_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], x1: [uint32x4_t; 16]) -> [uint32x4_t; 16] {
	let mut live = k;
	let mut res = [Z; 16];
	let (mut t0l, mut t0h) = ([0u32; 32], [0u32; 32]);
	{ let v = (((((vgetq_lane_u32::<0>(x0[8]) as u64) << 32 | vgetq_lane_u32::<0>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<0>(x1[8]) as u64) << 32 | vgetq_lane_u32::<0>(x1[0]) as u64) as u64) as u128) >> 64) as u64; t0l[0] = v as u32; t0h[0] = (v >> 32) as u32; }
	{ let v = (((((vgetq_lane_u32::<1>(x0[8]) as u64) << 32 | vgetq_lane_u32::<1>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<1>(x1[8]) as u64) << 32 | vgetq_lane_u32::<1>(x1[0]) as u64) as u64) as u128) >> 64) as u64; t0l[1] = v as u32; t0h[1] = (v >> 32) as u32; }
	{ let v = (((((vgetq_lane_u32::<2>(x0[8]) as u64) << 32 | vgetq_lane_u32::<2>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<2>(x1[8]) as u64) << 32 | vgetq_lane_u32::<2>(x1[0]) as u64) as u64) as u128) >> 64) as u64; t0l[2] = v as u32; t0h[2] = (v >> 32) as u32; }
	{ let v = (((((vgetq_lane_u32::<3>(x0[8]) as u64) << 32 | vgetq_lane_u32::<3>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<3>(x1[8]) as u64) << 32 | vgetq_lane_u32::<3>(x1[0]) as u64) as u64) as u128) >> 64) as u64; t0l[3] = v as u32; t0h[3] = (v >> 32) as u32; }
	{ let v = (((((vgetq_lane_u32::<0>(x0[9]) as u64) << 32 | vgetq_lane_u32::<0>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<0>(x1[9]) as u64) << 32 | vgetq_lane_u32::<0>(x1[1]) as u64) as u64) as u128) >> 64) as u64; t0l[4] = v as u32; t0h[4] = (v >> 32) as u32; }
	{ let v = (((((vgetq_lane_u32::<1>(x0[9]) as u64) << 32 | vgetq_lane_u32::<1>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<1>(x1[9]) as u64) << 32 | vgetq_lane_u32::<1>(x1[1]) as u64) as u64) as u128) >> 64) as u64; t0l[5] = v as u32; t0h[5] = (v >> 32) as u32; }
	{ let v = (((((vgetq_lane_u32::<2>(x0[9]) as u64) << 32 | vgetq_lane_u32::<2>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<2>(x1[9]) as u64) << 32 | vgetq_lane_u32::<2>(x1[1]) as u64) as u64) as u128) >> 64) as u64; t0l[6] = v as u32; t0h[6] = (v >> 32) as u32; }
	{ let v = (((((vgetq_lane_u32::<3>(x0[9]) as u64) << 32 | vgetq_lane_u32::<3>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<3>(x1[9]) as u64) << 32 | vgetq_lane_u32::<3>(x1[1]) as u64) as u64) as u128) >> 64) as u64; t0l[7] = v as u32; t0h[7] = (v >> 32) as u32; }
	{ let v = (((((vgetq_lane_u32::<0>(x0[10]) as u64) << 32 | vgetq_lane_u32::<0>(x0[2]) as u64) as u64) as u128 * (((vgetq_lane_u32::<0>(x1[10]) as u64) << 32 | vgetq_lane_u32::<0>(x1[2]) as u64) as u64) as u128) >> 64) as u64; t0l[8] = v as u32; t0h[8] = (v >> 32) as u32; }
	{ let v = (((((vgetq_lane_u32::<1>(x0[10]) as u64) << 32 | vgetq_lane_u32::<1>(x0[2]) as u64) as u64) as u128 * (((vgetq_lane_u32::<1>(x1[10]) as u64) << 32 | vgetq_lane_u32::<1>(x1[2]) as u64) as u64) as u128) >> 64) as u64; t0l[9] = v as u32; t0h[9] = (v >> 32) as u32; }
	{ let v = (((((vgetq_lane_u32::<2>(x0[10]) as u64) << 32 | vgetq_lane_u32::<2>(x0[2]) as u64) as u64) as u128 * (((vgetq_lane_u32::<2>(x1[10]) as u64) << 32 | vgetq_lane_u32::<2>(x1[2]) as u64) as u64) as u128) >> 64) as u64; t0l[10] = v as u32; t0h[10] = (v >> 32) as u32; }
	{ let v = (((((vgetq_lane_u32::<3>(x0[10]) as u64) << 32 | vgetq_lane_u32::<3>(x0[2]) as u64) as u64) as u128 * (((vgetq_lane_u32::<3>(x1[10]) as u64) << 32 | vgetq_lane_u32::<3>(x1[2]) as u64) as u64) as u128) >> 64) as u64; t0l[11] = v as u32; t0h[11] = (v >> 32) as u32; }
	{ let v = (((((vgetq_lane_u32::<0>(x0[11]) as u64) << 32 | vgetq_lane_u32::<0>(x0[3]) as u64) as u64) as u128 * (((vgetq_lane_u32::<0>(x1[11]) as u64) << 32 | vgetq_lane_u32::<0>(x1[3]) as u64) as u64) as u128) >> 64) as u64; t0l[12] = v as u32; t0h[12] = (v >> 32) as u32; }
	{ let v = (((((vgetq_lane_u32::<1>(x0[11]) as u64) << 32 | vgetq_lane_u32::<1>(x0[3]) as u64) as u64) as u128 * (((vgetq_lane_u32::<1>(x1[11]) as u64) << 32 | vgetq_lane_u32::<1>(x1[3]) as u64) as u64) as u128) >> 64) as u64; t0l[13] = v as u32; t0h[13] = (v >> 32) as u32; }
	{ let v = (((((vgetq_lane_u32::<2>(x0[11]) as u64) << 32 | vgetq_lane_u32::<2>(x0[3]) as u64) as u64) as u128 * (((vgetq_lane_u32::<2>(x1[11]) as u64) << 32 | vgetq_lane_u32::<2>(x1[3]) as u64) as u64) as u128) >> 64) as u64; t0l[14] = v as u32; t0h[14] = (v >> 32) as u32; }
	{ let v = (((((vgetq_lane_u32::<3>(x0[11]) as u64) << 32 | vgetq_lane_u32::<3>(x0[3]) as u64) as u64) as u128 * (((vgetq_lane_u32::<3>(x1[11]) as u64) << 32 | vgetq_lane_u32::<3>(x1[3]) as u64) as u64) as u128) >> 64) as u64; t0l[15] = v as u32; t0h[15] = (v >> 32) as u32; }
	{ let v = (((((vgetq_lane_u32::<0>(x0[12]) as u64) << 32 | vgetq_lane_u32::<0>(x0[4]) as u64) as u64) as u128 * (((vgetq_lane_u32::<0>(x1[12]) as u64) << 32 | vgetq_lane_u32::<0>(x1[4]) as u64) as u64) as u128) >> 64) as u64; t0l[16] = v as u32; t0h[16] = (v >> 32) as u32; }
	{ let v = (((((vgetq_lane_u32::<1>(x0[12]) as u64) << 32 | vgetq_lane_u32::<1>(x0[4]) as u64) as u64) as u128 * (((vgetq_lane_u32::<1>(x1[12]) as u64) << 32 | vgetq_lane_u32::<1>(x1[4]) as u64) as u64) as u128) >> 64) as u64; t0l[17] = v as u32; t0h[17] = (v >> 32) as u32; }
	{ let v = (((((vgetq_lane_u32::<2>(x0[12]) as u64) << 32 | vgetq_lane_u32::<2>(x0[4]) as u64) as u64) as u128 * (((vgetq_lane_u32::<2>(x1[12]) as u64) << 32 | vgetq_lane_u32::<2>(x1[4]) as u64) as u64) as u128) >> 64) as u64; t0l[18] = v as u32; t0h[18] = (v >> 32) as u32; }
	{ let v = (((((vgetq_lane_u32::<3>(x0[12]) as u64) << 32 | vgetq_lane_u32::<3>(x0[4]) as u64) as u64) as u128 * (((vgetq_lane_u32::<3>(x1[12]) as u64) << 32 | vgetq_lane_u32::<3>(x1[4]) as u64) as u64) as u128) >> 64) as u64; t0l[19] = v as u32; t0h[19] = (v >> 32) as u32; }
	{ let v = (((((vgetq_lane_u32::<0>(x0[13]) as u64) << 32 | vgetq_lane_u32::<0>(x0[5]) as u64) as u64) as u128 * (((vgetq_lane_u32::<0>(x1[13]) as u64) << 32 | vgetq_lane_u32::<0>(x1[5]) as u64) as u64) as u128) >> 64) as u64; t0l[20] = v as u32; t0h[20] = (v >> 32) as u32; }
	{ let v = (((((vgetq_lane_u32::<1>(x0[13]) as u64) << 32 | vgetq_lane_u32::<1>(x0[5]) as u64) as u64) as u128 * (((vgetq_lane_u32::<1>(x1[13]) as u64) << 32 | vgetq_lane_u32::<1>(x1[5]) as u64) as u64) as u128) >> 64) as u64; t0l[21] = v as u32; t0h[21] = (v >> 32) as u32; }
	{ let v = (((((vgetq_lane_u32::<2>(x0[13]) as u64) << 32 | vgetq_lane_u32::<2>(x0[5]) as u64) as u64) as u128 * (((vgetq_lane_u32::<2>(x1[13]) as u64) << 32 | vgetq_lane_u32::<2>(x1[5]) as u64) as u64) as u128) >> 64) as u64; t0l[22] = v as u32; t0h[22] = (v >> 32) as u32; }
	{ let v = (((((vgetq_lane_u32::<3>(x0[13]) as u64) << 32 | vgetq_lane_u32::<3>(x0[5]) as u64) as u64) as u128 * (((vgetq_lane_u32::<3>(x1[13]) as u64) << 32 | vgetq_lane_u32::<3>(x1[5]) as u64) as u64) as u128) >> 64) as u64; t0l[23] = v as u32; t0h[23] = (v >> 32) as u32; }
	{ let v = (((((vgetq_lane_u32::<0>(x0[14]) as u64) << 32 | vgetq_lane_u32::<0>(x0[6]) as u64) as u64) as u128 * (((vgetq_lane_u32::<0>(x1[14]) as u64) << 32 | vgetq_lane_u32::<0>(x1[6]) as u64) as u64) as u128) >> 64) as u64; t0l[24] = v as u32; t0h[24] = (v >> 32) as u32; }
	{ let v = (((((vgetq_lane_u32::<1>(x0[14]) as u64) << 32 | vgetq_lane_u32::<1>(x0[6]) as u64) as u64) as u128 * (((vgetq_lane_u32::<1>(x1[14]) as u64) << 32 | vgetq_lane_u32::<1>(x1[6]) as u64) as u64) as u128) >> 64) as u64; t0l[25] = v as u32; t0h[25] = (v >> 32) as u32; }
	{ let v = (((((vgetq_lane_u32::<2>(x0[14]) as u64) << 32 | vgetq_lane_u32::<2>(x0[6]) as u64) as u64) as u128 * (((vgetq_lane_u32::<2>(x1[14]) as u64) << 32 | vgetq_lane_u32::<2>(x1[6]) as u64) as u64) as u128) >> 64) as u64; t0l[26] = v as u32; t0h[26] = (v >> 32) as u32; }
	{ let v = (((((vgetq_lane_u32::<3>(x0[14]) as u64) << 32 | vgetq_lane_u32::<3>(x0[6]) as u64) as u64) as u128 * (((vgetq_lane_u32::<3>(x1[14]) as u64) << 32 | vgetq_lane_u32::<3>(x1[6]) as u64) as u64) as u128) >> 64) as u64; t0l[27] = v as u32; t0h[27] = (v >> 32) as u32; }
	{ let v = (((((vgetq_lane_u32::<0>(x0[15]) as u64) << 32 | vgetq_lane_u32::<0>(x0[7]) as u64) as u64) as u128 * (((vgetq_lane_u32::<0>(x1[15]) as u64) << 32 | vgetq_lane_u32::<0>(x1[7]) as u64) as u64) as u128) >> 64) as u64; t0l[28] = v as u32; t0h[28] = (v >> 32) as u32; }
	{ let v = (((((vgetq_lane_u32::<1>(x0[15]) as u64) << 32 | vgetq_lane_u32::<1>(x0[7]) as u64) as u64) as u128 * (((vgetq_lane_u32::<1>(x1[15]) as u64) << 32 | vgetq_lane_u32::<1>(x1[7]) as u64) as u64) as u128) >> 64) as u64; t0l[29] = v as u32; t0h[29] = (v >> 32) as u32; }
	{ let v = (((((vgetq_lane_u32::<2>(x0[15]) as u64) << 32 | vgetq_lane_u32::<2>(x0[7]) as u64) as u64) as u128 * (((vgetq_lane_u32::<2>(x1[15]) as u64) << 32 | vgetq_lane_u32::<2>(x1[7]) as u64) as u64) as u128) >> 64) as u64; t0l[30] = v as u32; t0h[30] = (v >> 32) as u32; }
	{ let v = (((((vgetq_lane_u32::<3>(x0[15]) as u64) << 32 | vgetq_lane_u32::<3>(x0[7]) as u64) as u64) as u128 * (((vgetq_lane_u32::<3>(x1[15]) as u64) << 32 | vgetq_lane_u32::<3>(x1[7]) as u64) as u64) as u128) >> 64) as u64; t0l[31] = v as u32; t0h[31] = (v >> 32) as u32; }
	let t1: [uint32x4_t; 8] = [vld1q_u32(t0l.as_ptr().add(0)), vld1q_u32(t0l.as_ptr().add(4)), vld1q_u32(t0l.as_ptr().add(8)), vld1q_u32(t0l.as_ptr().add(12)), vld1q_u32(t0l.as_ptr().add(16)), vld1q_u32(t0l.as_ptr().add(20)), vld1q_u32(t0l.as_ptr().add(24)), vld1q_u32(t0l.as_ptr().add(28))];
	let t2: [uint32x4_t; 8] = [vld1q_u32(t0h.as_ptr().add(0)), vld1q_u32(t0h.as_ptr().add(4)), vld1q_u32(t0h.as_ptr().add(8)), vld1q_u32(t0h.as_ptr().add(12)), vld1q_u32(t0h.as_ptr().add(16)), vld1q_u32(t0h.as_ptr().add(20)), vld1q_u32(t0h.as_ptr().add(24)), vld1q_u32(t0h.as_ptr().add(28))];
	let t3 = cat_w(t1, t2);
	res = t3;
	return res;
}

/// `ukl_ld_window`.
#[inline(always)]
unsafe fn b5_r0ns_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg0: *const u8, x1f: u32, x1s: u32, x2: u32) -> [uint32x4_t; 16] {
	let mut x3 = [Z; 16];
	let mut x4 = [Z; 8];
	let mut x5 = [Z; 16];
	let mut live = k;
	let mut res = [Z; 16];
	if ((x2) == (8u32)) {
		let t0: [uint32x4_t; 16];
		if (x1s) == 8u32 && ((x1f) as u64) + 31 * ((x1s) as u64) <= u32::MAX as u64 && all(&live) {
			let t1 = (arg0 as *const u8).add(((x1f) as usize) * 1);
			let t2 = vld2q_u32((t1 as *const u32).add(0));
			let t3 = vld2q_u32((t1 as *const u32).add(8));
			let t4 = vld2q_u32((t1 as *const u32).add(16));
			let t5 = vld2q_u32((t1 as *const u32).add(24));
			let t6 = vld2q_u32((t1 as *const u32).add(32));
			let t7 = vld2q_u32((t1 as *const u32).add(40));
			let t8 = vld2q_u32((t1 as *const u32).add(48));
			let t9 = vld2q_u32((t1 as *const u32).add(56));
			let t10: [uint32x4_t; 16] = [t2.0, t3.0, t4.0, t5.0, t6.0, t7.0, t8.0, t9.0, t2.1, t3.1, t4.1, t5.1, t6.1, t7.1, t8.1, t9.1];
			t0 = t10;
		} else {
			let mut t11 = [Z; 16];
			if ((x1f) as u64) + 31 * ((x1s) as u64) <= u32::MAX as u64 && all(&live) {
				let t12 = (arg0 as *const u8).add(((x1f) as usize) * 1);
				let t13 = ((x1s) as usize) * 1;
				t11 = ldj_u64(|j| t12.add(j * t13) as *const u8);
			} else {
				let t14: [uint32x4_t; 8] = core::array::from_fn(|r| vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[r].as_ptr()), vdupq_n_u32(x1s)));
				let t15 = ix_on(live, t14);
				let t16 = ld_u64(arg0 as *const u8, t15, 1);
				t11 = t16;
			}
			t0 = t11;
		}
		res = t0;
		return res;
	}
	if ((x2) == (4u32)) {
		let t17: [uint32x4_t; 8];
		if (x1s) == 4u32 && ((x1f) as u64) + 31 * ((x1s) as u64) <= u32::MAX as u64 && all(&live) {
			let t18 = (arg0 as *const u8).add(((x1f) as usize) * 1);
			let t19: [uint32x4_t; 8] = core::array::from_fn(|r| vld1q_u32((t18 as *const u32).add(4 * r)));
			t17 = t19;
		} else {
			let mut t20 = [Z; 8];
			if ((x1f) as u64) + 31 * ((x1s) as u64) <= u32::MAX as u64 && all(&live) {
				let t21 = (arg0 as *const u8).add(((x1f) as usize) * 1);
				let t22 = ((x1s) as usize) * 1;
				t20 = ldj_u32(|j| t21.add(j * t22) as *const u8);
			} else {
				let t23: [uint32x4_t; 8] = core::array::from_fn(|r| vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[r].as_ptr()), vdupq_n_u32(x1s)));
				let t24 = ix_on(live, t23);
				let t25 = ld_u32(arg0 as *const u8, t24, 1);
				t20 = t25;
			}
			t17 = t20;
		}
		let t26 = cat_w(lo(t17), [Z; 8]);
		res = t26;
		return res;
	}
	if ((x2) == (2u32)) {
		let t27: [uint32x4_t; 8];
		if (x1s) == 2u32 && ((x1f) as u64) + 31 * ((x1s) as u64) <= u32::MAX as u64 && all(&live) {
			let t28 = (arg0 as *const u8).add(((x1f) as usize) * 1);
			let t29: [uint32x4_t; 8] = core::array::from_fn(|r| vmovl_u16(vld1_u16((t28 as *const u16).add(4 * r))));
			t27 = t29;
		} else {
			let mut t30 = [Z; 8];
			if ((x1f) as u64) + 31 * ((x1s) as u64) <= u32::MAX as u64 && all(&live) {
				let t31 = (arg0 as *const u8).add(((x1f) as usize) * 1);
				let t32 = ((x1s) as usize) * 1;
				t30 = ldj_u16(|j| t31.add(j * t32) as *const u8);
			} else {
				let t33: [uint32x4_t; 8] = core::array::from_fn(|r| vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[r].as_ptr()), vdupq_n_u32(x1s)));
				let t34 = ix_on(live, t33);
				let t35 = ld_u16(arg0 as *const u8, t34, 1);
				t30 = t35;
			}
			t27 = t30;
		}
		let t36 = cat_w(lo(t27), [Z; 8]);
		res = t36;
		return res;
	}
	if ((x2) == (1u32)) {
		let t37: [uint32x4_t; 8];
		if (x1s) == 1u32 && ((x1f) as u64) + 31 * ((x1s) as u64) <= u32::MAX as u64 && all(&live) {
			let t38 = (arg0 as *const u8).add(((x1f) as usize) * 1);
			let t39 = vmovl_u8(vld1_u8(t38.add(0)));
			let t40 = vmovl_u8(vld1_u8(t38.add(8)));
			let t41 = vmovl_u8(vld1_u8(t38.add(16)));
			let t42 = vmovl_u8(vld1_u8(t38.add(24)));
			let t43: [uint32x4_t; 8] = [vmovl_u16(vget_low_u16(t39)), vmovl_u16(vget_high_u16(t39)), vmovl_u16(vget_low_u16(t40)), vmovl_u16(vget_high_u16(t40)), vmovl_u16(vget_low_u16(t41)), vmovl_u16(vget_high_u16(t41)), vmovl_u16(vget_low_u16(t42)), vmovl_u16(vget_high_u16(t42))];
			t37 = t43;
		} else {
			let mut t44 = [Z; 8];
			if ((x1f) as u64) + 31 * ((x1s) as u64) <= u32::MAX as u64 && all(&live) {
				let t45 = (arg0 as *const u8).add(((x1f) as usize) * 1);
				let t46 = ((x1s) as usize) * 1;
				t44 = ldj_u8(|j| t45.add(j * t46) as *const u8);
			} else {
				let t47: [uint32x4_t; 8] = core::array::from_fn(|r| vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[r].as_ptr()), vdupq_n_u32(x1s)));
				let t48 = ix_on(live, t47);
				let t49 = ld_u8(arg0 as *const u8, t48, 1);
				t44 = t49;
			}
			t37 = t44;
		}
		let t50 = cat_w(lo(t37), [Z; 8]);
		res = t50;
		return res;
	}
	let t51: [uint32x4_t; 8] = core::array::from_fn(|r| vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[r].as_ptr()), vdupq_n_u32(x1s)));
	let t52 = splat(!(7u32));
	let t53 = and_r(t51, t52);
	let t54: [uint32x4_t; 8] = lo(t53);
	let t55: u64 = ((arg0) as usize as u64);
	let t56 = splat((t55) as u32);
	let t57 = splat(((t55) >> 32) as u32);
	let t58 = add_r(t56, t54);
	let t59 = clt_r(t58, t56);
	let t60: [uint32x4_t; 8] = core::array::from_fn(|r| vadcq_u32(t57[r], Z, t59[r]));
	let t61 = cat_w(lo(t58), t60);
	let t62 = cat_w(lo(t61), hi(t61));
	x3 = t62;
	let t63: [uint32x4_t; 8] = lo(x3);
	let t64: [uint32x4_t; 8] = hi(x3);
	let t65 = addrs(t63, t64, live);
	let t66: [uint32x4_t; 8] = core::array::from_fn(|r| vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[r].as_ptr()), vdupq_n_u32(x1s)));
	let t67 = splat(7u32);
	let t68 = and_r(t66, t67);
	let t69 = splat(8u32);
	let t70 = mul_r(t68, t69);
	x4 = t70;
	let mut t71 = [Z; 16];
	t71 = ldj_u64(|j| (t65[j] as *const u8) as *const u8);
	let t72: [uint32x4_t; 8] = lo(x4);
	let t73: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t72[r], vdupq_n_u32(63)));
	let t74: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(vshlq_u32(t71[r], vnegq_s32(vreinterpretq_s32_u32(t73[r]))), vshlq_u32(t71[(r + 8)], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t73[r])))));
	let t75: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t71[(r + 8)], vnegq_s32(vreinterpretq_s32_u32(t73[r]))));
	let t76 = cat_w(t74, t75);
	x5 = t76;
	let t77 = [Z; 8];
	let t78 = cne_r(x4, t77);
	let t79 = splat((x2).wrapping_mul(8u32));
	let t80 = add_r(x4, t79);
	let t81 = splat(64u32);
	let t82 = cgt_r(t80, t81);
	let t83 = and_r(t78, t82);
	let mut t84 = [Z; 16];
	let t85 = and_r(live, t83);
	let t86 = bic_r(live, t83);
	{
		let t87: u64 = (1u64).wrapping_mul(8u64);
		let t88 = splat((t87) as u32);
		let t89 = splat(((t87) >> 32) as u32);
		let t90 = add_r(lo(x3), t88);
		let t91 = clt_r(t90, lo(x3));
		let t92 = adc_r(hi(x3), t89, t91);
		let t93 = cat_w(lo(t90), t92);
		let t94: [usize; 32] = core::array::from_fn(|j| t65[j].wrapping_add(((1u64).wrapping_mul(8u64)) as u64 as usize));
		let mut t95 = [Z; 16];
		t95 = ldj_u64(|j| (t94[j] as *const u8) as *const u8);
		let t96 = sub_r(t81, x4);
		let t97: [uint32x4_t; 8] = lo(t96);
		let t98: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t97[r], vdupq_n_u32(63)));
		let t99: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t95[r], vreinterpretq_s32_u32(t98[r])));
		let t100: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(vshlq_u32(t95[(r + 8)], vreinterpretq_s32_u32(t98[r])), vshlq_u32(t95[r], vsubq_s32(vreinterpretq_s32_u32(t98[r]), vdupq_n_s32(32)))));
		let t101 = cat_w(t99, t100);
		let t102: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(x5[r], t101[r]));
		let t103: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(x5[(r + 8)], t101[(r + 8)]));
		let t104 = cat_w(t102, t103);
		t84 = bsl_w(t85, t104, t84);
	}
	{
		t84 = bsl_w(t86, x5, t84);
	}
	res = t84;
	return res;
}

/// `wide_part`.
#[inline(always)]
unsafe fn b6_i_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: u32) -> [uint32x4_t; 16] {
	let mut live = k;
	let mut res = [Z; 16];
	let t0 = iota(x0);
	let t1: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t0[r], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))));
	let t2 = splat(255u32);
	let t3 = and_r(t0, t2);
	let t4: [uint32x4_t; 8] = core::array::from_fn(|r| vmulq_u32(t1[r], vdupq_n_u32(1000)));
	let t5: [uint32x4_t; 8] = core::array::from_fn(|r| vmulhiq_u32(t1[r], vdupq_n_u32(1000)));
	let t6 = add_r(t4, t3);
	let t7 = clt_r(t6, t4);
	let t8: [uint32x4_t; 8] = core::array::from_fn(|r| vadcq_u32(t5[r], Z, t7[r]));
	let t9 = cat_w(lo(t6), t8);
	res = t9;
	return res;
}

/// `wide_part`.
#[inline(always)]
unsafe fn b6_u8_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: u32) -> [uint32x4_t; 16] {
	let mut live = k;
	let mut res = [Z; 16];
	let t0 = iota((x0 & 255u32));
	let t1: [uint32x4_t; 8] = lo(t0);
	let t2: u64 = ((((((((x0 >> 8)) as u64)).wrapping_mul(1000u64)) as usize as *const u8)) as usize as u64);
	let t3 = splat((t2) as u32);
	let t4 = splat(((t2) >> 32) as u32);
	let t5 = add_r(t3, t1);
	let t6 = clt_r(t5, t3);
	let t7: [uint32x4_t; 8] = core::array::from_fn(|r| vadcq_u32(t4[r], Z, t6[r]));
	let t8 = cat_w(lo(t5), t7);
	let t9 = cat_w(lo(t8), hi(t8));
	res = t9;
	return res;
}

/// `wide_rare`.
#[inline(always)]
unsafe fn b7_ss_f(l: &Launch, lid0: u32, x0: u64, x1: u32) -> u64 {
	let mut x2: u64 = 0;
	let mut x3: u32 = 0;
	let mut live = splat(u32::MAX);
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
unsafe fn b7_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], x1: [uint32x4_t; 8]) -> [uint32x4_t; 16] {
	let mut x2 = [Z; 16];
	let mut x3: u32 = 0;
	let mut live = k;
	let mut res = [Z; 16];
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
				let t3 = words8(x1);
				let mut x1_j: u32 = t3[j];
				let t4 = words16(x2);
				let mut x2_j: u64 = ((t4[32 + j] as u64) << 32 | t4[j] as u64);
				let mut x3_j: u32 = x3;
				loop {
					if !(((x3_j) < (((x1_j) & (15u32))))) { break; }
					x2_j = ((x2_j).wrapping_mul(31u64)).wrapping_add(((x3_j) as u64));
					x3_j = (x3_j).wrapping_add(2u32);
				}
				let mut t5 = words16(x2);
				t5[j] = x2_j as u32;
				t5[32 + j] = (x2_j as u64 >> 32) as u32;
				x2 = regs16(t5);
			}
			break;
		}
		// Lanes part: each its own rounds.
		let t6 = splat(15u32);
		let t7 = and_r(x1, t6);
		let t8 = splat(x3);
		let t9 = clt_r(t8, t7);
		t0 = and_r(t0, t9);
		if !(any(&t0)) { break; }
		let t10: u64 = 31u64;
		let t11 = splat((t10) as u32);
		let t12 = mulhi_r(lo(x2), t11);
		let t13 = mul_r(lo(x2), t11);
		let t14 = mla_r(t12, t11, hi(x2));
		let t15 = cat_w(t13, t14);
		let t16: u64 = ((x3) as u64);
		let t17 = splat((t16) as u32);
		let t18 = splat(((t16) >> 32) as u32);
		let t19 = add_r(lo(t15), t17);
		let t20 = clt_r(t19, lo(t15));
		let t21 = adc_r(hi(t15), t18, t20);
		let t22 = cat_w(lo(t19), t21);
		x2 = bsl_w(t0, t22, x2);
		let t23 = and_r(t0, live);
		x3 = (x3).wrapping_add(2u32);
	}
	res = x2;
	return res;
}

/// `ukl_mask64`.
#[inline(always)]
unsafe fn b8_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: u32) -> u64 {
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

/// The group `l.group`'s threads, 32 at a time: `argv[j]` the address of argument `j`'s bytes (a buffer's address,
/// a value's bytes).
///
/// # Safety
///
/// `argv` holds the kernel's arguments, its buffers as large as it reads and writes; `l` is a 1-D group's launch.
pub unsafe fn wide_lanes_w32(argv: *const *const u8, l: &Launch) {
	let n = l.block[0] * l.block[1] * l.block[2];
	let arg0: *const u8 = (*argv.add(0) as *const *const u8).read_unaligned();
	let arg1: *const u8 = (*argv.add(1) as *const *const u8).read_unaligned();
	let arg2: *const u8 = (*argv.add(2) as *const *const u8).read_unaligned();
	let arg3: *const u8 = (*argv.add(3) as *const *const u8).read_unaligned();
	let x4: *const u8 = *argv.add(4);
	let arg5: *const u8 = (*argv.add(5) as *const *const u8).read_unaligned();
	let x6: u32 = (*argv.add(6) as *const u32).read_unaligned();
	let mut t478: bool = !(arg5).is_null();
	if t478 {
		let mut t479: bool = ((l.group[0]) >= ((arg5.wrapping_add((((x6) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
		if !t479 {
			t479 = ((l.group[1]) >= ((arg5.wrapping_add(((((x6).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
		}
		let mut t480: bool = t479;
		if !t480 {
			t480 = ((l.group[2]) >= ((arg5.wrapping_add(((((x6).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
		}
		t478 = t480;
	}
	if t478 {
		return;
	}
	let mut lid0 = 0u32;
	while lid0 + 32 <= n {
		let mut live = splat(u32::MAX);
		let gid0 = l.group[0] * l.block[0] + lid0;
		'chunk: {
			b0_r0r1r2r3sg_p(l, lid0, splat(u32::MAX), arg0, arg1, arg2, arg3, x4);
		}
		lid0 += 32;
	}
	if lid0 < n {
		// Lanes part: the group's last threads, those under `n`.
		let t481: [uint32x4_t; 8] = core::array::from_fn(|r| vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[r].as_ptr())), vdupq_n_u32(n)));
		let mut live = t481;
		let gid0 = l.group[0] * l.block[0] + lid0;
		'chunk: {
			b0_r0r1r2r3sg_p(l, lid0, live, arg0, arg1, arg2, arg3, x4);
		}
	}
}
