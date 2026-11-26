//! `parent_group` at 32 lanes (8 NEON registers), written by rerail-ukl-vec's Rust (`rs.rs`) from its vector tape: do
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
/// Where a load reads when no lane of its mask runs (a block entered under a uniform branch none of its lanes
/// takes): zeros, as a store's lanes that are off write to a sink. Its loads' constant offsets lie inside it.
static NO_LANE: [u8; 4096] = [0; 4096];
/// Each lane's address of its halves `lo` and `hi`; a lane mask `k` has off, the first on's (none on: `NO_LANE`).
#[inline(always)]
unsafe fn addrs(lo: [uint32x4_t; 8], hi: [uint32x4_t; 8], k: [uint32x4_t; 8]) -> [usize; 32] {
	let (l, h, b) = (words8(lo), words8(hi), bits(&k));
	if b == 0 {
		return [NO_LANE.as_ptr() as usize; 32];
	}
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

/// `parent_group_inner`.
#[inline(always)]
unsafe fn b0_r0r1r2sr6gi_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg0: *const u8, arg1: *const u8, arg2: *const u8, x3: *const u8, arg6: *const u8, x6: u32, phase: u32, st: *mut u8) {
	let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x3 as *const u32).read_unaligned();
	let mut x7 = [Z; 8];
	let mut x8 = [Z; 8];
	let mut x9 = [Z; 16];
	let mut x10: u32 = 0;
	let mut live = k;
	if phase > 0 {
		live = (st as *const [uint32x4_t; 8]).read_unaligned();
		x7 = (st.add(256) as *const [uint32x4_t; 8]).read_unaligned();
		x8 = (st.add(512) as *const [uint32x4_t; 8]).read_unaligned();
		x9 = (st.add(768) as *const [uint32x4_t; 16]).read_unaligned();
		x10 = (st.add(1024) as *const u32).read_unaligned();
		if !any(&live) { return; }
	}
	if phase == 0 {
		'phase: {
			let t0 = iota(gid0);
			let t1 = iota(x6);
			let t2 = sub_r(t0, t1);
			x7 = t2;
			let t3 = splat(4294967295u32);
			x8 = t3;
			let t4 = splat(a0);
			let t5 = clt_r(t0, t4);
			let t6 = and_r(live, t5);
			if any(&t6) {
				let t7 = b1_r0r1g_p(l, lid0, t6, arg0, arg1);
				x9 = t7;
				let t8: u64 = (1u64).wrapping_shl((32u64) as u32);
				let t9 = splat((t8) as u32);
				let t10 = splat(((t8) >> 32) as u32);
				let t11 = and_r(lo(x9), t9);
				let t12 = and_r(hi(x9), t10);
				let t13 = cat_w(t11, t12);
				let t14: u64 = 0u64;
				let t15 = splat((t14) as u32);
				let t16: [uint32x4_t; 8] = core::array::from_fn(|r| veq64q_u32(t13[r], t13[(r + 8)], t15[r], Z));
				let t17 = mvn_r(t16);
				let mut t18 = [Z; 8];
				let t19 = and_r(t6, t17);
				let t20 = bic_r(t6, t17);
				{
					let t21: [uint32x4_t; 8] = lo(x9);
					let t22 = splat(2147483648u32);
					let t23 = orr_r(t22, t21);
					t18 = bsl_r(t19, t23, t18);
				}
				{
					let t24: [uint32x4_t; 8] = lo(x9);
					t18 = bsl_r(t20, t24, t18);
				}
				x8 = bsl_r(t6, t18, x8);
			}
			let t25 = (arg6 as *mut u32).add((x6) as usize);
			if all(&live) {
				vst1q_u32(t25.add(0), x8[0]);
				vst1q_u32(t25.add(4), x8[1]);
				vst1q_u32(t25.add(8), x8[2]);
				vst1q_u32(t25.add(12), x8[3]);
				vst1q_u32(t25.add(16), x8[4]);
				vst1q_u32(t25.add(20), x8[5]);
				vst1q_u32(t25.add(24), x8[6]);
				vst1q_u32(t25.add(28), x8[7]);
			} else {
				stj_u32(live, x8, |j| t25.add(j) as *mut u8);
			}
		}
	}
	if phase == 1 {
		'phase: {
			let t26 = iota(gid0);
			let t27 = splat(a0);
			let t28 = cge_r(t26, t27);
			let t29 = and_r(live, t28);
			{
				live = bic_r(live, t29);
			}
			if any(&live) {
				x10 = 0u32;
				let mut t30 = live;
				loop {
					let t31 = bits(&t30);
					if t31.count_ones() <= 2 {
						let mut t32 = t31;
						while t32 != 0 {
							let j = t32.trailing_zeros() as usize;
							t32 &= t32 - 1;
							let t33 = words8(x7);
							let mut x7_j: u32 = t33[j];
							let t34 = words8(x8);
							let mut x8_j: u32 = t34[j];
							let mut x10_j: u32 = x10;
							loop {
								let mut t35: bool = ((x10_j) < (7u32));
								if t35 {
									t35 = ((x8_j) != (4294967295u32));
								}
								let mut t36: bool = t35;
								if t36 {
									t36 = ((((x8_j) & (2147483648u32))) != (0u32));
								}
								let mut t37: bool = t36;
								if t37 {
									t37 = ((((x8_j) & (2147483647u32))) >= (x7_j));
								}
								if !(t37) { break; }
								x8_j = (arg6.wrapping_add(((((((x8_j) & (2147483647u32))).wrapping_sub(x7_j)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned();
								let t38 = x10_j;
								x10_j = (x10_j).wrapping_add(1u32);
							}
							let mut t39 = words8(x8);
							t39[j] = x8_j as u32;
							x8 = regs8(t39);
						}
						break;
					}
					// Lanes part: each its own rounds.
					let t40 = splat(4294967295u32);
					let t41 = cne_r(x8, t40);
					let t42: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(vdupq_n_u32(if ((x10) < (7u32)) { u32::MAX } else { 0 }), t41[r]));
					let t43 = splat(2147483648u32);
					let t44 = and_r(x8, t43);
					let t45 = [Z; 8];
					let t46 = cne_r(t44, t45);
					let t47 = and_r(t42, t46);
					let t48 = splat(2147483647u32);
					let t49 = and_r(x8, t48);
					let t50 = cge_r(t49, x7);
					let t51 = and_r(t47, t50);
					t30 = and_r(t30, t51);
					if !(any(&t30)) { break; }
					let t52 = and_r(x8, t48);
					let t53 = sub_r(t52, x7);
					let t54 = ix_on(t30, t53);
					let t55 = arg6 as *const u32;
					let mut t56 = [Z; 8];
					let t57 = words8(t54);
					t56 = ldj_u32(|j| t55.add(t57[j] as usize) as *const u8);
					x8 = bsl_r(t30, t56, x8);
					let t58 = and_r(t30, live);
					let t59 = x10;
					x10 = (x10).wrapping_add(1u32);
				}
				let t60 = splat(4294967295u32);
				let t61 = cne_r(x8, t60);
				let t62 = splat(2147483648u32);
				let t63 = and_r(x8, t62);
				let t64 = [Z; 8];
				let t65 = cne_r(t63, t64);
				let t66 = and_r(t61, t65);
				let t67 = and_r(live, t66);
				if any(&t67) {
					let t68 = splat(2147483647u32);
					let t69 = and_r(x8, t68);
					let t70 = b3_r0r1l_p(l, lid0, t67, arg0, arg1, t69);
					x8 = bsl_r(t67, t70, x8);
				}
				let t71 = (arg2 as *mut u32).add((gid0) as usize);
				if all(&live) {
					vst1q_u32(t71.add(0), x8[0]);
					vst1q_u32(t71.add(4), x8[1]);
					vst1q_u32(t71.add(8), x8[2]);
					vst1q_u32(t71.add(12), x8[3]);
					vst1q_u32(t71.add(16), x8[4]);
					vst1q_u32(t71.add(20), x8[5]);
					vst1q_u32(t71.add(24), x8[6]);
					vst1q_u32(t71.add(28), x8[7]);
				} else {
					stj_u32(live, x8, |j| t71.add(j) as *mut u8);
				}
			}
		}
	}
	if phase + 1 < 2 {
		(st as *mut [uint32x4_t; 8]).write_unaligned(live);
		(st.add(256) as *mut [uint32x4_t; 8]).write_unaligned(x7);
		(st.add(512) as *mut [uint32x4_t; 8]).write_unaligned(x8);
		(st.add(768) as *mut [uint32x4_t; 16]).write_unaligned(x9);
		(st.add(1024) as *mut u32).write_unaligned(x10);
	}
}

/// `parent_group_inner`.
#[inline(always)]
unsafe fn b0_r0r1r2sr6gi_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, arg2: *const u8, x3: *const u8, arg6: *const u8, x6: u32, phase: u32, st: *mut u8) {
	let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x3 as *const u32).read_unaligned();
	let mut x7 = [Z; 8];
	let mut x8 = [Z; 8];
	let mut x9 = [Z; 16];
	let mut x10: u32 = 0;
	let mut live = splat(u32::MAX);
	if phase > 0 {
		live = (st as *const [uint32x4_t; 8]).read_unaligned();
		x7 = (st.add(256) as *const [uint32x4_t; 8]).read_unaligned();
		x8 = (st.add(512) as *const [uint32x4_t; 8]).read_unaligned();
		x9 = (st.add(768) as *const [uint32x4_t; 16]).read_unaligned();
		x10 = (st.add(1024) as *const u32).read_unaligned();
		if !any(&live) { return; }
	}
	if phase == 0 {
		'phase: {
			let t0 = iota(gid0);
			let t1 = iota(x6);
			let t2 = sub_r(t0, t1);
			x7 = t2;
			let t3 = splat(4294967295u32);
			x8 = t3;
			if (((gid0) as u64 + 31) <= u32::MAX as u64 && ((gid0) as u64) >= ((a0) as u64)) {
			} else if ((gid0) as u64 + 31) < ((a0) as u64) {
				let t4 = b1_r0r1g_f(l, lid0, arg0, arg1);
				x9 = t4;
				let t5: u64 = (1u64).wrapping_shl((32u64) as u32);
				let t6 = splat((t5) as u32);
				let t7 = splat(((t5) >> 32) as u32);
				let t8 = and_r(lo(x9), t6);
				let t9 = and_r(hi(x9), t7);
				let t10 = cat_w(t8, t9);
				let t11: u64 = 0u64;
				let t12 = splat((t11) as u32);
				let t13: [uint32x4_t; 8] = core::array::from_fn(|r| veq64q_u32(t10[r], t10[(r + 8)], t12[r], Z));
				let t14 = mvn_r(t13);
				let mut t15 = [Z; 8];
				let t16 = mvn_r(t14);
				{
					let t17: [uint32x4_t; 8] = lo(x9);
					let t18 = splat(2147483648u32);
					let t19 = orr_r(t18, t17);
					t15 = bsl_r(t14, t19, t15);
				}
				{
					let t20: [uint32x4_t; 8] = lo(x9);
					t15 = bsl_r(t16, t20, t15);
				}
				x8 = t15;
			} else {
				// Lanes part: the chunk's lanes on both sides of the bound.
				let t21 = splat(a0);
				let t22 = clt_r(t0, t21);
				let t23 = and_r(live, t22);
				if any(&t23) {
					let t24 = b1_r0r1g_p(l, lid0, t23, arg0, arg1);
					x9 = t24;
					let t25: u64 = (1u64).wrapping_shl((32u64) as u32);
					let t26 = splat((t25) as u32);
					let t27 = splat(((t25) >> 32) as u32);
					let t28 = and_r(lo(x9), t26);
					let t29 = and_r(hi(x9), t27);
					let t30 = cat_w(t28, t29);
					let t31: u64 = 0u64;
					let t32 = splat((t31) as u32);
					let t33: [uint32x4_t; 8] = core::array::from_fn(|r| veq64q_u32(t30[r], t30[(r + 8)], t32[r], Z));
					let t34 = mvn_r(t33);
					let mut t35 = [Z; 8];
					let t36 = and_r(t23, t34);
					let t37 = bic_r(t23, t34);
					{
						let t38: [uint32x4_t; 8] = lo(x9);
						let t39 = splat(2147483648u32);
						let t40 = orr_r(t39, t38);
						t35 = bsl_r(t36, t40, t35);
					}
					{
						let t41: [uint32x4_t; 8] = lo(x9);
						t35 = bsl_r(t37, t41, t35);
					}
					x8 = bsl_r(t23, t35, x8);
				}
			}
			let t42 = (arg6 as *mut u32).add((x6) as usize);
			vst1q_u32(t42.add(0), x8[0]);
			vst1q_u32(t42.add(4), x8[1]);
			vst1q_u32(t42.add(8), x8[2]);
			vst1q_u32(t42.add(12), x8[3]);
			vst1q_u32(t42.add(16), x8[4]);
			vst1q_u32(t42.add(20), x8[5]);
			vst1q_u32(t42.add(24), x8[6]);
			vst1q_u32(t42.add(28), x8[7]);
		}
	}
	if phase == 1 {
		'phase: {
			let t43 = iota(gid0);
			let t44 = splat(a0);
			let t45 = cge_r(t43, t44);
			let t46 = and_r(live, t45);
			{
				live = bic_r(live, t46);
			}
			if any(&live) {
				x10 = 0u32;
				let mut t47 = live;
				loop {
					let t48 = bits(&t47);
					if t48.count_ones() <= 2 {
						let mut t49 = t48;
						while t49 != 0 {
							let j = t49.trailing_zeros() as usize;
							t49 &= t49 - 1;
							let t50 = words8(x7);
							let mut x7_j: u32 = t50[j];
							let t51 = words8(x8);
							let mut x8_j: u32 = t51[j];
							let mut x10_j: u32 = x10;
							loop {
								let mut t52: bool = ((x10_j) < (7u32));
								if t52 {
									t52 = ((x8_j) != (4294967295u32));
								}
								let mut t53: bool = t52;
								if t53 {
									t53 = ((((x8_j) & (2147483648u32))) != (0u32));
								}
								let mut t54: bool = t53;
								if t54 {
									t54 = ((((x8_j) & (2147483647u32))) >= (x7_j));
								}
								if !(t54) { break; }
								x8_j = (arg6.wrapping_add(((((((x8_j) & (2147483647u32))).wrapping_sub(x7_j)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned();
								let t55 = x10_j;
								x10_j = (x10_j).wrapping_add(1u32);
							}
							let mut t56 = words8(x8);
							t56[j] = x8_j as u32;
							x8 = regs8(t56);
						}
						break;
					}
					// Lanes part: each its own rounds.
					let t57 = splat(4294967295u32);
					let t58 = cne_r(x8, t57);
					let t59: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(vdupq_n_u32(if ((x10) < (7u32)) { u32::MAX } else { 0 }), t58[r]));
					let t60 = splat(2147483648u32);
					let t61 = and_r(x8, t60);
					let t62 = [Z; 8];
					let t63 = cne_r(t61, t62);
					let t64 = and_r(t59, t63);
					let t65 = splat(2147483647u32);
					let t66 = and_r(x8, t65);
					let t67 = cge_r(t66, x7);
					let t68 = and_r(t64, t67);
					t47 = and_r(t47, t68);
					if !(any(&t47)) { break; }
					let t69 = and_r(x8, t65);
					let t70 = sub_r(t69, x7);
					let t71 = ix_on(t47, t70);
					let t72 = arg6 as *const u32;
					let mut t73 = [Z; 8];
					let t74 = words8(t71);
					t73 = ldj_u32(|j| t72.add(t74[j] as usize) as *const u8);
					x8 = bsl_r(t47, t73, x8);
					let t75 = and_r(t47, live);
					let t76 = x10;
					x10 = (x10).wrapping_add(1u32);
				}
				let t77 = splat(4294967295u32);
				let t78 = cne_r(x8, t77);
				let t79 = splat(2147483648u32);
				let t80 = and_r(x8, t79);
				let t81 = [Z; 8];
				let t82 = cne_r(t80, t81);
				let t83 = and_r(t78, t82);
				let t84 = and_r(live, t83);
				if any(&t84) {
					let t85 = splat(2147483647u32);
					let t86 = and_r(x8, t85);
					let t87 = b3_r0r1l_p(l, lid0, t84, arg0, arg1, t86);
					x8 = bsl_r(t84, t87, x8);
				}
				let t88 = (arg2 as *mut u32).add((gid0) as usize);
				if all(&live) {
					vst1q_u32(t88.add(0), x8[0]);
					vst1q_u32(t88.add(4), x8[1]);
					vst1q_u32(t88.add(8), x8[2]);
					vst1q_u32(t88.add(12), x8[3]);
					vst1q_u32(t88.add(16), x8[4]);
					vst1q_u32(t88.add(20), x8[5]);
					vst1q_u32(t88.add(24), x8[6]);
					vst1q_u32(t88.add(28), x8[7]);
				} else {
					stj_u32(live, x8, |j| t88.add(j) as *mut u8);
				}
			}
		}
	}
	if phase + 1 < 2 {
		(st as *mut [uint32x4_t; 8]).write_unaligned(live);
		(st.add(256) as *mut [uint32x4_t; 8]).write_unaligned(x7);
		(st.add(512) as *mut [uint32x4_t; 8]).write_unaligned(x8);
		(st.add(768) as *mut [uint32x4_t; 16]).write_unaligned(x9);
		(st.add(1024) as *mut u32).write_unaligned(x10);
	}
}

/// `clif_f16_parent_step`.
#[inline(always)]
unsafe fn b1_r0r1g_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg0: *const u8, arg1: *const u8) -> [uint32x4_t; 16] {
	let gid0 = l.group[0] * l.block[0] + lid0;
	let mut x3: u32 = 0;
	let mut x4 = [Z; 16];
	let mut x5 = [Z; 8];
	let mut x6 = [Z; 8];
	let mut live = k;
	let mut res = [Z; 16];
	let t0 = iota(gid0);
	let t1 = [Z; 8];
	let t2 = ceq_r(t0, t1);
	let t3 = and_r(live, t2);
	{
		let t4 = splat64(4294967295u64);
		res = bsl_w(t3, t4, res);
		live = bic_r(live, t3);
	}
	if any(&live) {
		x3 = gid0.wrapping_sub(1u32);
		let t5 = b4_r0r1i_p(l, lid0, live, arg0, arg1, x3);
		x4 = t5;
		let t6: u64 = 1u64;
		let t7 = splat((t6) as u32);
		let t8 = and_r(lo(x4), t7);
		let t9: u64 = 0u64;
		let t10 = splat((t9) as u32);
		let t11 = ceq_r(t8, t10);
		let t12 = and_r(live, t11);
		{
			let t13 = iota(x3);
			let t14 = cat_w(lo(t13), [Z; 8]);
			res = bsl_w(t12, t14, res);
			live = bic_r(live, t12);
		}
		if any(&live) {
			let t15: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(vshrq_n_u32::<2>(x4[r]), vshlq_n_u32::<30>(x4[(r + 8)])));
			let t16: [uint32x4_t; 8] = core::array::from_fn(|r| vshrq_n_u32::<2>(x4[(r + 8)]));
			let t17 = cat_w(t15, t16);
			let t18: [uint32x4_t; 8] = lo(t17);
			x5 = t18;
			let t19 = splat(1u32);
			let t20 = sub_r(x5, t19);
			let t21 = cge_r(t20, t0);
			let t22 = and_r(live, t21);
			{
				let t23 = splat64(4294967295u64);
				res = bsl_w(t22, t23, res);
				live = bic_r(live, t22);
			}
			if any(&live) {
				let t24 = sub_r(t0, x5);
				x6 = t24;
				let t25: u64 = 2u64;
				let t26 = splat((t25) as u32);
				let t27 = and_r(lo(x4), t26);
				let t28: u64 = 0u64;
				let t29 = splat((t28) as u32);
				let t30 = ceq_r(t27, t29);
				let t31 = mvn_r(t30);
				let t32 = and_r(live, t31);
				{
					let t33 = cat_w(lo(x6), [Z; 8]);
					res = bsl_w(t32, t33, res);
					live = bic_r(live, t32);
				}
				if any(&live) {
					let t34 = ceq_r(x6, t1);
					let t35 = and_r(live, t34);
					{
						let t36 = splat64(4294967295u64);
						res = bsl_w(t35, t36, res);
						live = bic_r(live, t35);
					}
					if any(&live) {
						let t37: u64 = (1u64).wrapping_shl((32u64) as u32);
						let t38 = splat((t37) as u32);
						let t39 = splat(((t37) >> 32) as u32);
						let t40 = orr_r(t38, x6);
						let t41 = cat_w(t40, lo(t39));
						res = bsl_w(live, t41, res);
						live = bic_r(live, live);
					}
				}
			}
		}
	}
	res
}

/// `clif_f16_parent_step`.
#[inline(always)]
unsafe fn b1_r0r1g_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8) -> [uint32x4_t; 16] {
	let gid0 = l.group[0] * l.block[0] + lid0;
	let mut x3: u32 = 0;
	let mut x4 = [Z; 16];
	let mut x5 = [Z; 8];
	let mut x6 = [Z; 8];
	let mut live = splat(u32::MAX);
	let mut res = [Z; 16];
	if (((gid0) as u64 + 31) <= u32::MAX as u64 && (((0u32) as u64) < ((gid0) as u64) || ((0u32) as u64) > ((gid0) as u64 + 31))) {
		x3 = gid0.wrapping_sub(1u32);
		let t0 = b4_r0r1i_f(l, lid0, arg0, arg1, x3);
		x4 = t0;
		let t1: u64 = 1u64;
		let t2 = splat((t1) as u32);
		let t3 = and_r(lo(x4), t2);
		let t4: u64 = 0u64;
		let t5 = splat((t4) as u32);
		let t6 = ceq_r(t3, t5);
		{
			let t7 = iota(x3);
			let t8 = cat_w(lo(t7), [Z; 8]);
			res = bsl_w(t6, t8, res);
			live = bic_r(live, t6);
		}
		if any(&live) {
			let t9: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(vshrq_n_u32::<2>(x4[r]), vshlq_n_u32::<30>(x4[(r + 8)])));
			let t10: [uint32x4_t; 8] = core::array::from_fn(|r| vshrq_n_u32::<2>(x4[(r + 8)]));
			let t11 = cat_w(t9, t10);
			let t12: [uint32x4_t; 8] = lo(t11);
			x5 = t12;
			let t13 = splat(1u32);
			let t14 = sub_r(x5, t13);
			let t15 = iota(gid0);
			let t16 = cge_r(t14, t15);
			let t17 = and_r(live, t16);
			{
				let t18 = splat64(4294967295u64);
				res = bsl_w(t17, t18, res);
				live = bic_r(live, t17);
			}
			if any(&live) {
				let t19 = sub_r(t15, x5);
				x6 = t19;
				let t20: u64 = 2u64;
				let t21 = splat((t20) as u32);
				let t22 = and_r(lo(x4), t21);
				let t23: u64 = 0u64;
				let t24 = splat((t23) as u32);
				let t25 = ceq_r(t22, t24);
				let t26 = mvn_r(t25);
				let t27 = and_r(live, t26);
				{
					let t28 = cat_w(lo(x6), [Z; 8]);
					res = bsl_w(t27, t28, res);
					live = bic_r(live, t27);
				}
				if any(&live) {
					let t29 = [Z; 8];
					let t30 = ceq_r(x6, t29);
					let t31 = and_r(live, t30);
					{
						let t32 = splat64(4294967295u64);
						res = bsl_w(t31, t32, res);
						live = bic_r(live, t31);
					}
					if any(&live) {
						let t33: u64 = (1u64).wrapping_shl((32u64) as u32);
						let t34 = splat((t33) as u32);
						let t35 = splat(((t33) >> 32) as u32);
						let t36 = orr_r(t34, x6);
						let t37 = cat_w(t36, lo(t35));
						res = bsl_w(live, t37, res);
						live = bic_r(live, live);
					}
				}
			}
		}
	} else if false {
		let t38 = splat64(4294967295u64);
		res = t38;
		return res;
	} else {
		// Lanes part: the chunk's lanes on both sides of the bound.
		let t39 = iota(gid0);
		let t40 = [Z; 8];
		let t41 = ceq_r(t39, t40);
		let t42 = and_r(live, t41);
		{
			let t43 = splat64(4294967295u64);
			res = bsl_w(t42, t43, res);
			live = bic_r(live, t42);
		}
		if any(&live) {
			x3 = gid0.wrapping_sub(1u32);
			let t44 = b4_r0r1i_p(l, lid0, live, arg0, arg1, x3);
			x4 = t44;
			let t45: u64 = 1u64;
			let t46 = splat((t45) as u32);
			let t47 = and_r(lo(x4), t46);
			let t48: u64 = 0u64;
			let t49 = splat((t48) as u32);
			let t50 = ceq_r(t47, t49);
			let t51 = and_r(live, t50);
			{
				let t52 = iota(x3);
				let t53 = cat_w(lo(t52), [Z; 8]);
				res = bsl_w(t51, t53, res);
				live = bic_r(live, t51);
			}
			if any(&live) {
				let t54: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(vshrq_n_u32::<2>(x4[r]), vshlq_n_u32::<30>(x4[(r + 8)])));
				let t55: [uint32x4_t; 8] = core::array::from_fn(|r| vshrq_n_u32::<2>(x4[(r + 8)]));
				let t56 = cat_w(t54, t55);
				let t57: [uint32x4_t; 8] = lo(t56);
				x5 = t57;
				let t58 = splat(1u32);
				let t59 = sub_r(x5, t58);
				let t60 = cge_r(t59, t39);
				let t61 = and_r(live, t60);
				{
					let t62 = splat64(4294967295u64);
					res = bsl_w(t61, t62, res);
					live = bic_r(live, t61);
				}
				if any(&live) {
					let t63 = sub_r(t39, x5);
					x6 = t63;
					let t64: u64 = 2u64;
					let t65 = splat((t64) as u32);
					let t66 = and_r(lo(x4), t65);
					let t67: u64 = 0u64;
					let t68 = splat((t67) as u32);
					let t69 = ceq_r(t66, t68);
					let t70 = mvn_r(t69);
					let t71 = and_r(live, t70);
					{
						let t72 = cat_w(lo(x6), [Z; 8]);
						res = bsl_w(t71, t72, res);
						live = bic_r(live, t71);
					}
					if any(&live) {
						let t73 = ceq_r(x6, t40);
						let t74 = and_r(live, t73);
						{
							let t75 = splat64(4294967295u64);
							res = bsl_w(t74, t75, res);
							live = bic_r(live, t74);
						}
						if any(&live) {
							let t76: u64 = (1u64).wrapping_shl((32u64) as u32);
							let t77 = splat((t76) as u32);
							let t78 = splat(((t76) >> 32) as u32);
							let t79 = orr_r(t77, x6);
							let t80 = cat_w(t79, lo(t78));
							res = bsl_w(live, t80, res);
							live = bic_r(live, live);
						}
					}
				}
			}
		}
	}
	res
}

/// `clif_f16_parent_step`.
#[inline(always)]
unsafe fn b1_r0r1l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg0: *const u8, arg1: *const u8, x2: [uint32x4_t; 8]) -> [uint32x4_t; 16] {
	let mut x3 = [Z; 8];
	let mut x4 = [Z; 16];
	let mut x5 = [Z; 8];
	let mut x6 = [Z; 8];
	let mut live = k;
	let mut res = [Z; 16];
	let t0 = [Z; 8];
	let t1 = ceq_r(x2, t0);
	let t2 = and_r(live, t1);
	{
		let t3 = splat64(4294967295u64);
		res = bsl_w(t2, t3, res);
		live = bic_r(live, t2);
	}
	if any(&live) {
		let t4 = splat(1u32);
		let t5 = sub_r(x2, t4);
		x3 = t5;
		let t6 = b4_r0r1l_p(l, lid0, live, arg0, arg1, x3);
		x4 = t6;
		let t7: u64 = 1u64;
		let t8 = splat((t7) as u32);
		let t9 = and_r(lo(x4), t8);
		let t10: u64 = 0u64;
		let t11 = splat((t10) as u32);
		let t12 = ceq_r(t9, t11);
		let t13 = and_r(live, t12);
		{
			let t14 = cat_w(lo(x3), [Z; 8]);
			res = bsl_w(t13, t14, res);
			live = bic_r(live, t13);
		}
		if any(&live) {
			let t15: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(vshrq_n_u32::<2>(x4[r]), vshlq_n_u32::<30>(x4[(r + 8)])));
			let t16: [uint32x4_t; 8] = core::array::from_fn(|r| vshrq_n_u32::<2>(x4[(r + 8)]));
			let t17 = cat_w(t15, t16);
			let t18: [uint32x4_t; 8] = lo(t17);
			x5 = t18;
			let t19 = sub_r(x5, t4);
			let t20 = cge_r(t19, x2);
			let t21 = and_r(live, t20);
			{
				let t22 = splat64(4294967295u64);
				res = bsl_w(t21, t22, res);
				live = bic_r(live, t21);
			}
			if any(&live) {
				let t23 = sub_r(x2, x5);
				x6 = t23;
				let t24: u64 = 2u64;
				let t25 = splat((t24) as u32);
				let t26 = and_r(lo(x4), t25);
				let t27: u64 = 0u64;
				let t28 = splat((t27) as u32);
				let t29 = ceq_r(t26, t28);
				let t30 = mvn_r(t29);
				let t31 = and_r(live, t30);
				{
					let t32 = cat_w(lo(x6), [Z; 8]);
					res = bsl_w(t31, t32, res);
					live = bic_r(live, t31);
				}
				if any(&live) {
					let t33 = ceq_r(x6, t0);
					let t34 = and_r(live, t33);
					{
						let t35 = splat64(4294967295u64);
						res = bsl_w(t34, t35, res);
						live = bic_r(live, t34);
					}
					if any(&live) {
						let t36: u64 = (1u64).wrapping_shl((32u64) as u32);
						let t37 = splat((t36) as u32);
						let t38 = splat(((t36) >> 32) as u32);
						let t39 = orr_r(t37, x6);
						let t40 = cat_w(t39, lo(t38));
						res = bsl_w(live, t40, res);
						live = bic_r(live, live);
					}
				}
			}
		}
	}
	res
}

/// `clif_f16_parent`.
#[inline(always)]
unsafe fn b3_r0r1l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg0: *const u8, arg1: *const u8, x2: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut x3 = [Z; 16];
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b1_r0r1l_p(l, lid0, live, arg0, arg1, x2);
	x3 = t0;
	let mut t1 = live;
	loop {
		// Lanes part: each its own rounds.
		let t2: u64 = (1u64).wrapping_shl((32u64) as u32);
		let t3 = splat((t2) as u32);
		let t4 = splat(((t2) >> 32) as u32);
		let t5 = and_r(lo(x3), t3);
		let t6 = and_r(hi(x3), t4);
		let t7 = cat_w(t5, t6);
		let t8: u64 = 0u64;
		let t9 = splat((t8) as u32);
		let t10: [uint32x4_t; 8] = core::array::from_fn(|r| veq64q_u32(t7[r], t7[(r + 8)], t9[r], Z));
		let t11 = mvn_r(t10);
		t1 = and_r(t1, t11);
		if !(any(&t1)) { break; }
		let t12: [uint32x4_t; 8] = lo(x3);
		let t13 = b1_r0r1l_p(l, lid0, t1, arg0, arg1, t12);
		x3 = bsl_w(t1, t13, x3);
	}
	let t14: [uint32x4_t; 8] = lo(x3);
	res = t14;
	return res;
}

/// `clif_f16_slot`.
#[inline(always)]
unsafe fn b4_r0r1i_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg0: *const u8, arg1: *const u8, x2: u32) -> [uint32x4_t; 16] {
	let mut live = k;
	let mut res = [Z; 16];
	let t0 = iota(x2);
	let t1 = b5_r0r1sl_p(l, lid0, live, arg0, arg1, 0u32, t0);
	res = t1;
	return res;
}

/// `clif_f16_slot`.
#[inline(always)]
unsafe fn b4_r0r1i_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32) -> [uint32x4_t; 16] {
	let mut live = splat(u32::MAX);
	let mut res = [Z; 16];
	let t0 = iota(x2);
	let t1 = b5_r0r1sl_p(l, lid0, splat(u32::MAX), arg0, arg1, 0u32, t0);
	res = t1;
	return res;
}

/// `clif_f16_slot`.
#[inline(always)]
unsafe fn b4_r0r1l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg0: *const u8, arg1: *const u8, x2: [uint32x4_t; 8]) -> [uint32x4_t; 16] {
	let mut live = k;
	let mut res = [Z; 16];
	let t0 = b5_r0r1sl_p(l, lid0, live, arg0, arg1, 0u32, x2);
	res = t0;
	return res;
}

/// `clif_f16_get`.
#[inline(always)]
unsafe fn b5_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg0: *const u8, arg1: *const u8, x2: u32, x3: [uint32x4_t; 8]) -> [uint32x4_t; 16] {
	let mut x4: u32 = 0;
	let mut x5 = [Z; 8];
	let mut x6: u64 = 0;
	let mut x7 = [Z; 16];
	let mut live = k;
	let mut res = [Z; 16];
	let t0 = b6_r0r1s_p(l, lid0, live, arg0, arg1, x2);
	x4 = t0;
	let t1: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(x3[r], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))));
	let t2 = b7_r0r1sl_p(l, lid0, live, arg0, arg1, (1u32).wrapping_add(x2), t1);
	x5 = t2;
	let t3 = b8_s_p(l, lid0, live, x4);
	x6 = t3;
	let t4 = splat(1023u32);
	let t5 = and_r(x3, t4);
	let t6 = splat(x4);
	let t7 = mul_r(t5, t6);
	let t8 = b9_r0r1lls_p(l, lid0, live, arg0, arg1, x5, t7, 8u32);
	let t9: u64 = x6;
	let t10 = splat((t9) as u32);
	let t11 = splat(((t9) >> 32) as u32);
	let t12 = and_r(lo(t8), t10);
	let t13 = and_r(hi(t8), t11);
	let t14 = cat_w(t12, t13);
	x7 = t14;
	let t15: u64 = x6;
	let t16 = splat((t15) as u32);
	let t17 = splat(((t15) >> 32) as u32);
	let t18 = eq64_r(lo(x7), hi(x7), t16, t17);
	let mut t19 = [Z; 16];
	let t20 = and_r(live, t18);
	let t21 = bic_r(live, t18);
	if any(&t20) {
		let t22: [uint32x4_t; 16];
		let t23 = bits(&t20);
		if t23.count_ones() <= 2 {
			let t24 = words8(x3);
			let mut t25 = [0u32; 64];
			let mut t26 = t23;
			while t26 != 0 {
				let j = t26.trailing_zeros() as usize;
				t26 &= t26 - 1;
				let t27 = splat(t24[j]);
				let t28 = b10_r0r1sl_p(l, lid0, splat(u32::MAX), arg0, arg1, (x2).wrapping_add(1u32), t27);
				t25[j] = vgetq_lane_u32::<0>(t28[0]);
				t25[32 + j] = vgetq_lane_u32::<0>(t28[8]);
			}
			t22 = regs16(t25);
		} else {
			let t29 = b10_r0r1sl_p(l, lid0, t20, arg0, arg1, (x2).wrapping_add(1u32), x3);
			t22 = t29;
		}
		t19 = bsl_w(t20, t22, t19);
	}
	{
		t19 = bsl_w(t21, x7, t19);
	}
	res = t19;
	return res;
}

/// `clif_f16_width`.
#[inline(always)]
unsafe fn b6_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg0: *const u8, arg1: *const u8, x2: u32) -> u32 {
	let mut live = k;
	let mut res: u32 = 0;
	let t0 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
	res = (((t0).wrapping_shr(((4u32).wrapping_mul(x2)) as u32)) & (15u32));
	return res;
}

/// `clif_f16_column`.
#[inline(always)]
unsafe fn b7_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg0: *const u8, arg1: *const u8, x2: u32, x3: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut x4 = [Z; 8];
	let mut x5 = [Z; 8];
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 40u32);
	let t1 = splat(t0);
	let t2 = mul_r(x3, t1);
	let t3 = b12_r0r1s_p(l, lid0, live, arg0, arg1, 0u32);
	let t4 = splat(t3);
	let t5 = add_r(t2, t4);
	x4 = t5;
	if ((x2) == (0u32)) {
		res = x4;
		return res;
	}
	let t6 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
	let t7: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(x3[r], vdupq_n_s32(((10u32) & 31) as i32)));
	let t8 = splat(t6);
	let t9 = sub_r(t8, t7);
	let t10 = b13_ls_p(l, lid0, live, t9, 1024u32);
	x5 = t10;
	let t11 = b12_r0r1s_p(l, lid0, live, arg0, arg1, x2);
	let t12 = splat(t11);
	let t13 = mul_r(x5, t12);
	let t14 = add_r(x4, t13);
	res = t14;
	return res;
}

/// `clif_t_mask`.
#[inline(always)]
unsafe fn b8_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: u32) -> u64 {
	let mut live = k;
	let mut res: u64 = 0;
	let t0: u64;
	if ((x0) >= (8u32)) {
		t0 = !(0u64);
	} else {
		t0 = ((1u64).wrapping_shl(((((x0).wrapping_mul(8u32)) as u64)) as u32)).wrapping_sub(1u64);
	}
	res = t0;
	return res;
}

/// `clif_f16_ld`.
#[inline(always)]
unsafe fn b9_r0r1lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg0: *const u8, arg1: *const u8, x2: [uint32x4_t; 8], x3: [uint32x4_t; 8], x4: u32) -> [uint32x4_t; 16] {
	let mut live = k;
	let mut res = [Z; 16];
	let t0 = b14_r0lls_p(l, lid0, live, arg0, x2, x3, x4);
	res = t0;
	return res;
}

/// `clif_f16_side`.
#[inline(always)]
unsafe fn b10_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg0: *const u8, arg1: *const u8, x2: u32, x3: [uint32x4_t; 8]) -> [uint32x4_t; 16] {
	let mut x4: u32 = 0;
	let mut x5: u32 = 0;
	let mut x6 = [Z; 8];
	let mut x7 = [Z; 8];
	let mut x8 = [Z; 8];
	let mut x9: u32 = 0;
	let mut x10 = [Z; 8];
	let mut x11 = [Z; 8];
	let mut x12 = [Z; 8];
	let mut x13 = [Z; 8];
	let mut x14 = [Z; 8];
	let mut x15 = [Z; 8];
	let mut x16: u64 = 0;
	let mut x17 = [Z; 16];
	let mut x18 = [Z; 8];
	let mut x19 = [Z; 8];
	let mut x20 = [Z; 16];
	let mut x21 = [Z; 16];
	let mut x22 = [Z; 16];
	let mut x23 = [Z; 8];
	let mut x24: u32 = 0;
	let mut x25 = [Z; 8];
	let mut x26 = [Z; 8];
	let mut live = k;
	let mut res = [Z; 16];
	let t0 = b11_r0r1s_p(l, lid0, live, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x2)));
	x4 = t0;
	let t1 = b11_r0r1s_p(l, lid0, live, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x2)));
	x5 = t1;
	let t2: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(x3[r], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))));
	x6 = t2;
	let t3 = splat(4u32);
	let t4 = mul_r(x6, t3);
	let t5 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x5, t4, 4u32);
	let t6: [uint32x4_t; 8] = lo(t5);
	x7 = t6;
	let t7 = mul_r(x6, t3);
	let t8 = add_r(t7, t3);
	let t9 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x5, t8, 4u32);
	let t10: [uint32x4_t; 8] = lo(t9);
	x8 = t10;
	let t11 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
	x9 = ((t11).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
	let t12 = mul_r(x6, t3);
	let t13 = splat(x9);
	let t14 = sub_r(t13, t12);
	let t15 = b16_sl_p(l, lid0, live, 4u32, t14);
	x10 = t15;
	let t16: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(x3[r], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))));
	let t17 = splat(3u32);
	let t18 = and_r(t16, t17);
	x11 = t18;
	let t19 = [Z; 8];
	let t20 = ceq_r(x11, t19);
	let mut t21 = [Z; 8];
	let t22 = and_r(live, t20);
	let t23 = bic_r(live, t20);
	{
		t21 = bsl_r(t22, t19, t21);
	}
	if any(&t23) {
		let t24 = splat(1u32);
		let t25 = sub_r(x11, t24);
		let t26 = splat(2u32);
		let t27 = mul_r(t26, t25);
		let t28 = add_r(x7, t27);
		let t29 = b17_ll_p(l, lid0, t23, t28, x8);
		let t30 = b15_r0r1sls_p(l, lid0, t23, arg0, arg1, x4, t29, 2u32);
		let t31: [uint32x4_t; 8] = lo(t30);
		t21 = bsl_r(t23, t31, t21);
	}
	x12 = t21;
	let t32 = splat(2u32);
	let t33 = mul_r(t32, x11);
	let t34 = add_r(x7, t33);
	let t35 = b17_ll_p(l, lid0, live, t34, x8);
	let t36 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x4, t35, 2u32);
	let t37: [uint32x4_t; 8] = lo(t36);
	x13 = t37;
	let t38 = mul_r(t32, x10);
	let t39 = add_r(x7, t38);
	x14 = t39;
	let t40 = splat(255u32);
	let t41 = and_r(x3, t40);
	x15 = t41;
	x16 = 9259542123273814144u64;
	let t42: u64 = 72340172838076673u64;
	let t43 = splat((t42) as u32);
	let t44 = splat(((t42) >> 32) as u32);
	let t45 = mulhi_r(x15, t43);
	let t46 = mul_r(x15, t43);
	let t47 = mla_r(t45, x15, t44);
	let t48 = cat_w(t46, t47);
	x17 = t48;
	x18 = x12;
	x19 = x12;
	let mut t49 = live;
	loop {
		// Lanes part: each its own rounds.
		let t50 = clt_r(x19, x13);
		t49 = and_r(t49, t50);
		if !(any(&t49)) { break; }
		let t51 = add_r(x14, x19);
		let t52 = b17_ll_p(l, lid0, t49, t51, x8);
		let t53 = b15_r0r1sls_p(l, lid0, t49, arg0, arg1, x4, t52, 8u32);
		x20 = t53;
		let t54: u64 = x16;
		let t55 = splat((t54) as u32);
		let t56 = splat(((t54) >> 32) as u32);
		let t57 = orr_r(lo(x20), t55);
		let t58 = orr_r(hi(x20), t56);
		let t59 = cat_w(t57, t58);
		let t60: u64 = !(x16);
		let t61 = splat((t60) as u32);
		let t62 = splat(((t60) >> 32) as u32);
		let t63 = and_r(lo(x17), t61);
		let t64 = and_r(hi(x17), t62);
		let t65 = cat_w(t63, t64);
		let t66: [uint32x4_t; 8] = core::array::from_fn(|r| vcltq_u32(t59[r], t65[r]));
		let t67: [uint32x4_t; 8] = core::array::from_fn(|r| vsubq_u32(t59[r], t65[r]));
		let t68 = sbc_r(hi(t59), hi(t65), t66);
		let t69 = cat_w(t67, t68);
		x21 = t69;
		let t70: [uint32x4_t; 16] = core::array::from_fn(|r| vmvnq_u32(x20[r]));
		let t71: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t70[r], x17[r]));
		let t72: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t70[(r + 8)], x17[(r + 8)]));
		let t73 = cat_w(t71, t72);
		let t74: [uint32x4_t; 8] = core::array::from_fn(|r| veorq_u32(x20[r], x17[r]));
		let t75: [uint32x4_t; 8] = core::array::from_fn(|r| veorq_u32(x20[(r + 8)], x17[(r + 8)]));
		let t76 = cat_w(t74, t75);
		let t77 = mvn_r(t76);
		let t78 = mvn_r(x21);
		let t79: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t77[r], t78[r]));
		let t80: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t77[(r + 8)], t78[(r + 8)]));
		let t81 = cat_w(t79, t80);
		let t82: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(t73[r], t81[r]));
		let t83: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(t73[(r + 8)], t81[(r + 8)]));
		let t84 = cat_w(t82, t83);
		let t85: u64 = x16;
		let t86 = splat((t85) as u32);
		let t87 = splat(((t85) >> 32) as u32);
		let t88 = and_r(t86, lo(t84));
		let t89 = and_r(t87, hi(t84));
		let t90 = cat_w(t88, t89);
		x22 = t90;
		let t91 = sub_r(x13, x19);
		x23 = t91;
		let t92 = splat(8u32);
		let t93 = cge_r(x23, t92);
		let mut t94 = [Z; 16];
		let t95 = and_r(t49, t93);
		let t96 = bic_r(t49, t93);
		{
			t94 = bsl_w(t95, x22, t94);
		}
		{
			let t97 = mul_r(t92, x23);
			let t98: u64 = 1u64;
			let t99 = splat((t98) as u32);
			let t100: [uint32x4_t; 8] = lo(t97);
			let t101: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t100[r], vdupq_n_u32(63)));
			let t102: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t99[r], vreinterpretq_s32_u32(t101[r])));
			let t103: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t99[r], vsubq_s32(vreinterpretq_s32_u32(t101[r]), vdupq_n_s32(32))));
			let t104 = cat_w(t102, t103);
			let t105: u64 = 1u64;
			let t106 = splat((t105) as u32);
			let t107 = clt_r(lo(t104), t106);
			let t108 = sub_r(lo(t104), t106);
			let t109: [uint32x4_t; 8] = core::array::from_fn(|r| vsbcq_u32(t104[(r + 8)], Z, t107[r]));
			let t110 = cat_w(t108, t109);
			let t111: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(x22[r], t110[r]));
			let t112: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(x22[(r + 8)], t110[(r + 8)]));
			let t113 = cat_w(t111, t112);
			t94 = bsl_w(t96, t113, t94);
		}
		let t114 = b18_l_p(l, lid0, t49, t94);
		let t115 = add_r(x18, t114);
		x18 = bsl_r(t49, t115, x18);
		let t116 = and_r(t49, live);
		let t117 = add_r(x19, t92);
		x19 = t117;
	}
	x12 = x18;
	let t118 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
	x24 = (((t118).wrapping_shr((((((4u32).wrapping_mul(x2)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32));
	let t119 = splat(1u32);
	let t120 = sub_r(x10, t119);
	let t121 = mul_r(t32, t120);
	let t122 = add_r(x7, t121);
	let t123 = b17_ll_p(l, lid0, live, t122, x8);
	let t124 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x4, t123, 2u32);
	let t125: [uint32x4_t; 8] = lo(t124);
	x25 = t125;
	let t126 = add_r(x14, x25);
	let t127 = splat(x24);
	let t128 = mul_r(x12, t127);
	let t129 = add_r(t126, t128);
	let t130 = b17_ll_p(l, lid0, live, t129, x8);
	x26 = t130;
	let t131 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x4, x26, 8u32);
	let t132 = b8_s_p(l, lid0, live, x24);
	let t133: u64 = t132;
	let t134 = splat((t133) as u32);
	let t135 = splat(((t133) >> 32) as u32);
	let t136 = and_r(lo(t131), t134);
	let t137 = and_r(hi(t131), t135);
	let t138 = cat_w(t136, t137);
	res = t138;
	return res;
}

/// `clif_f16_view`.
#[inline(always)]
unsafe fn b11_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg0: *const u8, arg1: *const u8, x2: u32) -> u32 {
	let mut live = k;
	let mut res: u32 = 0;
	res = (arg1.wrapping_add(((((x2).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned();
	return res;
}

/// `clif_f16_columns`.
#[inline(always)]
unsafe fn b12_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg0: *const u8, arg1: *const u8, x2: u32) -> u32 {
	let mut live = k;
	let mut res: u32 = 0;
	let t0 = b11_r0r1s_p(l, lid0, live, arg0, arg1, (20u32).wrapping_add((4u32).wrapping_mul(x2)));
	res = t0;
	return res;
}

/// `ukl_min`.
#[inline(always)]
unsafe fn b13_ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 8], x1: u32) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = splat(x1);
	let t1 = clt_r(x0, t0);
	let mut t2 = [Z; 8];
	let t3 = and_r(live, t1);
	let t4 = bic_r(live, t1);
	{
		t2 = bsl_r(t3, x0, t2);
	}
	{
		let t5 = splat(x1);
		t2 = bsl_r(t4, t5, t2);
	}
	res = t2;
	return res;
}

/// `clif_t_ld`.
#[inline(always)]
unsafe fn b14_r0lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg0: *const u8, x1: [uint32x4_t; 8], x2: [uint32x4_t; 8], x3: u32) -> [uint32x4_t; 16] {
	let mut x4 = [Z; 16];
	let mut live = k;
	let mut res = [Z; 16];
	let t0 = add_r(x1, x2);
	let t1 = clt_r(t0, x1);
	let t2: [uint32x4_t; 8] = core::array::from_fn(|r| vshrq_n_u32::<31>(t1[r]));
	let t3 = cat_w(lo(t0), t2);
	let t4 = b19_r0ls_p(l, lid0, live, arg0, t3, x3);
	x4 = t4;
	let t5: [uint32x4_t; 16];
	if ((x3) >= (8u32)) {
		t5 = x4;
	} else {
		let t6 = b20_s_p(l, lid0, live, (8u32).wrapping_mul(x3));
		let t7: u64 = t6;
		let t8 = splat((t7) as u32);
		let t9 = splat(((t7) >> 32) as u32);
		let t10 = and_r(lo(x4), t8);
		let t11 = and_r(hi(x4), t9);
		let t12 = cat_w(t10, t11);
		t5 = t12;
	}
	res = t5;
	return res;
}

/// `clif_f16_ld`.
#[inline(always)]
unsafe fn b15_r0r1sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg0: *const u8, arg1: *const u8, x2: u32, x3: [uint32x4_t; 8], x4: u32) -> [uint32x4_t; 16] {
	let mut live = k;
	let mut res = [Z; 16];
	let t0 = b21_r0sls_p(l, lid0, live, arg0, x2, x3, x4);
	res = t0;
	return res;
}

/// `ukl_min`.
#[inline(always)]
unsafe fn b16_sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: u32, x1: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = splat(x0);
	let t1 = clt_r(t0, x1);
	let mut t2 = [Z; 8];
	let t3 = and_r(live, t1);
	let t4 = bic_r(live, t1);
	{
		let t5 = splat(x0);
		t2 = bsl_r(t3, t5, t2);
	}
	{
		t2 = bsl_r(t4, x1, t2);
	}
	res = t2;
	return res;
}

/// `ukl_min`.
#[inline(always)]
unsafe fn b17_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 8], x1: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = clt_r(x0, x1);
	let mut t1 = [Z; 8];
	let t2 = and_r(live, t0);
	let t3 = bic_r(live, t0);
	{
		t1 = bsl_r(t2, x0, t1);
	}
	{
		t1 = bsl_r(t3, x1, t1);
	}
	res = t1;
	return res;
}

/// `ukl_popcount64`.
#[inline(always)]
unsafe fn b18_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16]) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0: [uint32x4_t; 8] = core::array::from_fn(|r| vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[r])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[(r + 8)]))))));
	res = t0;
	return res;
}

/// `ukl_ld_window`.
#[inline(always)]
unsafe fn b19_r0ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg0: *const u8, x1: [uint32x4_t; 16], x2: u32) -> [uint32x4_t; 16] {
	let mut x3 = [Z; 16];
	let mut x4 = [Z; 8];
	let mut x5 = [Z; 16];
	let mut live = k;
	let mut res = [Z; 16];
	if ((x2) == (8u32)) {
		let t0 = cat_w(live, live);
		let t1: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(x1[r], t0[r]));
		let t2: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(x1[(r + 8)], t0[(r + 8)]));
		let t3 = cat_w(t1, t2);
		let t4: u64 = ((arg0) as usize as u64);
		let t5 = splat((t4) as u32);
		let t6 = splat(((t4) >> 32) as u32);
		let t7 = add_r(t5, lo(t3));
		let t8 = clt_r(t7, t5);
		let t9 = adc_r(t6, hi(t3), t8);
		let t10 = cat_w(lo(t7), t9);
		let t11: [uint32x4_t; 8] = lo(t10);
		let t12: [uint32x4_t; 8] = hi(t10);
		let t13 = addrs(t11, t12, live);
		let mut t14 = [Z; 16];
		t14 = ldj_u64(|j| (t13[j] as *const u8) as *const u8);
		res = t14;
		return res;
	}
	if ((x2) == (4u32)) {
		let t15 = cat_w(live, live);
		let t16: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(x1[r], t15[r]));
		let t17: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(x1[(r + 8)], t15[(r + 8)]));
		let t18 = cat_w(t16, t17);
		let t19: u64 = ((arg0) as usize as u64);
		let t20 = splat((t19) as u32);
		let t21 = splat(((t19) >> 32) as u32);
		let t22 = add_r(t20, lo(t18));
		let t23 = clt_r(t22, t20);
		let t24 = adc_r(t21, hi(t18), t23);
		let t25 = cat_w(lo(t22), t24);
		let t26: [uint32x4_t; 8] = lo(t25);
		let t27: [uint32x4_t; 8] = hi(t25);
		let t28 = addrs(t26, t27, live);
		let mut t29 = [Z; 8];
		t29 = ldj_u32(|j| (t28[j] as *const u8) as *const u8);
		let t30 = cat_w(lo(t29), [Z; 8]);
		res = t30;
		return res;
	}
	if ((x2) == (2u32)) {
		let t31 = cat_w(live, live);
		let t32: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(x1[r], t31[r]));
		let t33: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(x1[(r + 8)], t31[(r + 8)]));
		let t34 = cat_w(t32, t33);
		let t35: u64 = ((arg0) as usize as u64);
		let t36 = splat((t35) as u32);
		let t37 = splat(((t35) >> 32) as u32);
		let t38 = add_r(t36, lo(t34));
		let t39 = clt_r(t38, t36);
		let t40 = adc_r(t37, hi(t34), t39);
		let t41 = cat_w(lo(t38), t40);
		let t42: [uint32x4_t; 8] = lo(t41);
		let t43: [uint32x4_t; 8] = hi(t41);
		let t44 = addrs(t42, t43, live);
		let mut t45 = [Z; 8];
		t45 = ldj_u16(|j| (t44[j] as *const u8) as *const u8);
		let t46 = cat_w(lo(t45), [Z; 8]);
		res = t46;
		return res;
	}
	if ((x2) == (1u32)) {
		let t47 = cat_w(live, live);
		let t48: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(x1[r], t47[r]));
		let t49: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(x1[(r + 8)], t47[(r + 8)]));
		let t50 = cat_w(t48, t49);
		let t51: u64 = ((arg0) as usize as u64);
		let t52 = splat((t51) as u32);
		let t53 = splat(((t51) >> 32) as u32);
		let t54 = add_r(t52, lo(t50));
		let t55 = clt_r(t54, t52);
		let t56 = adc_r(t53, hi(t50), t55);
		let t57 = cat_w(lo(t54), t56);
		let t58: [uint32x4_t; 8] = lo(t57);
		let t59: [uint32x4_t; 8] = hi(t57);
		let t60 = addrs(t58, t59, live);
		let mut t61 = [Z; 8];
		t61 = ldj_u8(|j| (t60[j] as *const u8) as *const u8);
		let t62 = cat_w(lo(t61), [Z; 8]);
		res = t62;
		return res;
	}
	let t63 = cat_w(live, live);
	let t64: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(x1[r], t63[r]));
	let t65: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(x1[(r + 8)], t63[(r + 8)]));
	let t66 = cat_w(t64, t65);
	let t67: u64 = ((arg0) as usize as u64);
	let t68 = splat((t67) as u32);
	let t69 = splat(((t67) >> 32) as u32);
	let t70 = add_r(t68, lo(t66));
	let t71 = clt_r(t70, t68);
	let t72 = adc_r(t69, hi(t66), t71);
	let t73 = cat_w(lo(t70), t72);
	let t74: u64 = ((arg0) as usize as u64);
	let t75 = splat((t74) as u32);
	let t76 = splat(((t74) >> 32) as u32);
	let t77 = add_r(t75, lo(x1));
	let t78 = clt_r(t77, t75);
	let t79 = adc_r(t76, hi(x1), t78);
	let t80 = cat_w(lo(t77), t79);
	let t81: u64 = 7u64;
	let t82 = splat((t81) as u32);
	let t83 = and_r(lo(t80), t82);
	let t84 = clt_r(lo(t73), t83);
	let t85 = sub_r(lo(t73), t83);
	let t86: [uint32x4_t; 8] = core::array::from_fn(|r| vsbcq_u32(t73[(r + 8)], Z, t84[r]));
	let t87 = cat_w(t85, t86);
	x3 = t87;
	let t88: [uint32x4_t; 8] = lo(x3);
	let t89: [uint32x4_t; 8] = hi(x3);
	let t90 = addrs(t88, t89, live);
	let t91: u64 = ((arg0) as usize as u64);
	let t92 = splat((t91) as u32);
	let t93 = splat(((t91) >> 32) as u32);
	let t94 = add_r(t92, lo(x1));
	let t95 = clt_r(t94, t92);
	let t96 = adc_r(t93, hi(x1), t95);
	let t97 = cat_w(lo(t94), t96);
	let t98: u64 = 7u64;
	let t99 = splat((t98) as u32);
	let t100 = and_r(lo(t97), t99);
	let t101 = splat(8u32);
	let t102 = mul_r(t100, t101);
	x4 = t102;
	let mut t103 = [Z; 16];
	t103 = ldj_u64(|j| (t90[j] as *const u8) as *const u8);
	let t104: [uint32x4_t; 8] = lo(x4);
	let t105: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t104[r], vdupq_n_u32(63)));
	let t106: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(vshlq_u32(t103[r], vnegq_s32(vreinterpretq_s32_u32(t105[r]))), vshlq_u32(t103[(r + 8)], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t105[r])))));
	let t107: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t103[(r + 8)], vnegq_s32(vreinterpretq_s32_u32(t105[r]))));
	let t108 = cat_w(t106, t107);
	x5 = t108;
	let t109 = [Z; 8];
	let t110 = cne_r(x4, t109);
	let t111 = splat((x2).wrapping_mul(8u32));
	let t112 = add_r(x4, t111);
	let t113 = splat(64u32);
	let t114 = cgt_r(t112, t113);
	let t115 = and_r(t110, t114);
	let mut t116 = [Z; 16];
	let t117 = and_r(live, t115);
	let t118 = bic_r(live, t115);
	{
		let t119: u64 = (1u64).wrapping_mul(8u64);
		let t120 = splat((t119) as u32);
		let t121 = splat(((t119) >> 32) as u32);
		let t122 = add_r(lo(x3), t120);
		let t123 = clt_r(t122, lo(x3));
		let t124 = adc_r(hi(x3), t121, t123);
		let t125 = cat_w(lo(t122), t124);
		let t126: [usize; 32] = core::array::from_fn(|j| t90[j].wrapping_add(((1u64).wrapping_mul(8u64)) as u64 as usize));
		let mut t127 = [Z; 16];
		t127 = ldj_u64(|j| (t126[j] as *const u8) as *const u8);
		let t128 = sub_r(t113, x4);
		let t129: [uint32x4_t; 8] = lo(t128);
		let t130: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t129[r], vdupq_n_u32(63)));
		let t131: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t127[r], vreinterpretq_s32_u32(t130[r])));
		let t132: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(vshlq_u32(t127[(r + 8)], vreinterpretq_s32_u32(t130[r])), vshlq_u32(t127[r], vsubq_s32(vreinterpretq_s32_u32(t130[r]), vdupq_n_s32(32)))));
		let t133 = cat_w(t131, t132);
		let t134: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(x5[r], t133[r]));
		let t135: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(x5[(r + 8)], t133[(r + 8)]));
		let t136 = cat_w(t134, t135);
		t116 = bsl_w(t117, t136, t116);
	}
	{
		t116 = bsl_w(t118, x5, t116);
	}
	res = t116;
	return res;
}

/// `ukl_ld_window`.
#[inline(always)]
unsafe fn b19_r0al1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg0: *const u8, x1b: *const u8, x1i: [uint32x4_t; 8], x2: u32) -> [uint32x4_t; 16] {
	let mut x3 = [Z; 16];
	let mut x4 = [Z; 8];
	let mut x5 = [Z; 16];
	let mut live = k;
	let mut res = [Z; 16];
	if ((x2) == (8u32)) {
		let t0 = ix_on(live, x1i);
		let t1 = ld_u64((arg0).wrapping_add((x1b) as usize) as *const u8, t0, 1);
		res = t1;
		return res;
	}
	if ((x2) == (4u32)) {
		let t2 = ix_on(live, x1i);
		let t3 = ld_u32((arg0).wrapping_add((x1b) as usize) as *const u8, t2, 1);
		let t4 = cat_w(lo(t3), [Z; 8]);
		res = t4;
		return res;
	}
	if ((x2) == (2u32)) {
		let t5 = ix_on(live, x1i);
		let t6 = ld_u16((arg0).wrapping_add((x1b) as usize) as *const u8, t5, 1);
		let t7 = cat_w(lo(t6), [Z; 8]);
		res = t7;
		return res;
	}
	if ((x2) == (1u32)) {
		let t8 = ix_on(live, x1i);
		let t9 = ld_u8((arg0).wrapping_add((x1b) as usize) as *const u8, t8, 1);
		let t10 = cat_w(lo(t9), [Z; 8]);
		res = t10;
		return res;
	}
	let t11: [uint32x4_t; 8] = lo(x1i);
	let t12: u64 = (((x1b).wrapping_add((((arg0) as usize as u64)) as usize)) as usize as u64);
	let t13 = splat((t12) as u32);
	let t14 = splat(((t12) >> 32) as u32);
	let t15 = add_r(t13, t11);
	let t16 = clt_r(t15, t13);
	let t17: [uint32x4_t; 8] = core::array::from_fn(|r| vadcq_u32(t14[r], Z, t16[r]));
	let t18 = cat_w(lo(t15), t17);
	let t19: u64 = 7u64;
	let t20 = splat((t19) as u32);
	let t21 = and_r(lo(t18), t20);
	let t22: [uint32x4_t; 8] = lo(x1i);
	let t23: u64 = (((arg0).wrapping_add((x1b) as usize)) as usize as u64);
	let t24 = splat((t23) as u32);
	let t25 = splat(((t23) >> 32) as u32);
	let t26 = add_r(t24, t22);
	let t27 = clt_r(t26, t24);
	let t28: [uint32x4_t; 8] = core::array::from_fn(|r| vadcq_u32(t25[r], Z, t27[r]));
	let t29 = cat_w(lo(t26), t28);
	let t30 = clt_r(lo(t29), t21);
	let t31 = sub_r(lo(t29), t21);
	let t32: [uint32x4_t; 8] = core::array::from_fn(|r| vsbcq_u32(t29[(r + 8)], Z, t30[r]));
	let t33 = cat_w(t31, t32);
	x3 = t33;
	let t34: [uint32x4_t; 8] = lo(x3);
	let t35: [uint32x4_t; 8] = hi(x3);
	let t36 = addrs(t34, t35, live);
	let t37: [uint32x4_t; 8] = lo(x1i);
	let t38: u64 = (((x1b).wrapping_add((((arg0) as usize as u64)) as usize)) as usize as u64);
	let t39 = splat((t38) as u32);
	let t40 = splat(((t38) >> 32) as u32);
	let t41 = add_r(t39, t37);
	let t42 = clt_r(t41, t39);
	let t43: [uint32x4_t; 8] = core::array::from_fn(|r| vadcq_u32(t40[r], Z, t42[r]));
	let t44 = cat_w(lo(t41), t43);
	let t45: u64 = 7u64;
	let t46 = splat((t45) as u32);
	let t47 = and_r(lo(t44), t46);
	let t48 = splat(8u32);
	let t49 = mul_r(t47, t48);
	x4 = t49;
	let mut t50 = [Z; 16];
	t50 = ldj_u64(|j| (t36[j] as *const u8) as *const u8);
	let t51: [uint32x4_t; 8] = lo(x4);
	let t52: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t51[r], vdupq_n_u32(63)));
	let t53: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(vshlq_u32(t50[r], vnegq_s32(vreinterpretq_s32_u32(t52[r]))), vshlq_u32(t50[(r + 8)], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t52[r])))));
	let t54: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t50[(r + 8)], vnegq_s32(vreinterpretq_s32_u32(t52[r]))));
	let t55 = cat_w(t53, t54);
	x5 = t55;
	let t56 = [Z; 8];
	let t57 = cne_r(x4, t56);
	let t58 = splat((x2).wrapping_mul(8u32));
	let t59 = add_r(x4, t58);
	let t60 = splat(64u32);
	let t61 = cgt_r(t59, t60);
	let t62 = and_r(t57, t61);
	let mut t63 = [Z; 16];
	let t64 = and_r(live, t62);
	let t65 = bic_r(live, t62);
	{
		let t66: u64 = (1u64).wrapping_mul(8u64);
		let t67 = splat((t66) as u32);
		let t68 = splat(((t66) >> 32) as u32);
		let t69 = add_r(lo(x3), t67);
		let t70 = clt_r(t69, lo(x3));
		let t71 = adc_r(hi(x3), t68, t70);
		let t72 = cat_w(lo(t69), t71);
		let t73: [usize; 32] = core::array::from_fn(|j| t36[j].wrapping_add(((1u64).wrapping_mul(8u64)) as u64 as usize));
		let mut t74 = [Z; 16];
		t74 = ldj_u64(|j| (t73[j] as *const u8) as *const u8);
		let t75 = sub_r(t60, x4);
		let t76: [uint32x4_t; 8] = lo(t75);
		let t77: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t76[r], vdupq_n_u32(63)));
		let t78: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t74[r], vreinterpretq_s32_u32(t77[r])));
		let t79: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(vshlq_u32(t74[(r + 8)], vreinterpretq_s32_u32(t77[r])), vshlq_u32(t74[r], vsubq_s32(vreinterpretq_s32_u32(t77[r]), vdupq_n_s32(32)))));
		let t80 = cat_w(t78, t79);
		let t81: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(x5[r], t80[r]));
		let t82: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(x5[(r + 8)], t80[(r + 8)]));
		let t83 = cat_w(t81, t82);
		t63 = bsl_w(t64, t83, t63);
	}
	{
		t63 = bsl_w(t65, x5, t63);
	}
	res = t63;
	return res;
}

/// `ukl_mask64`.
#[inline(always)]
unsafe fn b20_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: u32) -> u64 {
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

/// `clif_t_ld`.
#[inline(always)]
unsafe fn b21_r0sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg0: *const u8, x1: u32, x2: [uint32x4_t; 8], x3: u32) -> [uint32x4_t; 16] {
	let mut x4 = [Z; 16];
	let mut live = k;
	let mut res = [Z; 16];
	let t0 = b19_r0al1s_p(l, lid0, live, arg0, ((((x1) as u64)) as usize as *const u8), x2, x3);
	x4 = t0;
	let t1: [uint32x4_t; 16];
	if ((x3) >= (8u32)) {
		t1 = x4;
	} else {
		let t2 = b20_s_p(l, lid0, live, (8u32).wrapping_mul(x3));
		let t3: u64 = t2;
		let t4 = splat((t3) as u32);
		let t5 = splat(((t3) >> 32) as u32);
		let t6 = and_r(lo(x4), t4);
		let t7 = and_r(hi(x4), t5);
		let t8 = cat_w(t6, t7);
		t1 = t8;
	}
	res = t1;
	return res;
}

/// The group `l.group`'s threads, 32 at a time: `argv[j]` the address of argument `j`'s bytes (a buffer's address,
/// a value's bytes).
///
/// # Safety
///
/// `argv` holds the kernel's arguments, its buffers as large as it reads and writes; `l` is a 1-D group's launch.
pub unsafe fn parent_group_w32(argv: *const *const u8, l: &Launch) {
	let n = l.block[0] * l.block[1] * l.block[2];
	let arg0: *const u8 = (*argv.add(0) as *const *const u8).read_unaligned();
	let arg1: *const u8 = (*argv.add(1) as *const *const u8).read_unaligned();
	let arg2: *const u8 = (*argv.add(2) as *const *const u8).read_unaligned();
	let x3: *const u8 = *argv.add(3);
	let arg4: *const u8 = (*argv.add(4) as *const *const u8).read_unaligned();
	let x5: u32 = (*argv.add(5) as *const u32).read_unaligned();
	let mut tg = [0u64; 128];
	let arg6: *const u8 = tg.as_mut_ptr() as *const u8;
	let mut t843: bool = !(arg4).is_null();
	if t843 {
		let mut t844: bool = ((l.group[0]) >= ((arg4.wrapping_add((((x5) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
		if !t844 {
			t844 = ((l.group[1]) >= ((arg4.wrapping_add(((((x5).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
		}
		let mut t845: bool = t844;
		if !t845 {
			t845 = ((l.group[2]) >= ((arg4.wrapping_add(((((x5).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
		}
		t843 = t845;
	}
	if t843 {
		return;
	}
	// Phases: each chunk's, then the next phase's (its barriers).
	let mut state: Vec<u8> = Vec::with_capacity(n.div_ceil(32) as usize * 1280);
	let st: *mut u8 = state.as_mut_ptr();
	{
		let phase = 0u32;
		let mut lid0 = 0u32;
		while lid0 + 32 <= n {
			let mut live = splat(u32::MAX);
			let gid0 = l.group[0] * l.block[0] + lid0;
			'chunk: {
				b0_r0r1r2sr6gi_f(l, lid0, arg0, arg1, arg2, x3, arg6, lid0, phase, st.add((lid0 / 32) as usize * 1280));
			}
			lid0 += 32;
		}
		if lid0 < n {
			// Lanes part: the group's last threads, those under `n`.
			let t846: [uint32x4_t; 8] = core::array::from_fn(|r| vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[r].as_ptr())), vdupq_n_u32(n)));
			let mut live = t846;
			let gid0 = l.group[0] * l.block[0] + lid0;
			'chunk: {
				b0_r0r1r2sr6gi_p(l, lid0, live, arg0, arg1, arg2, x3, arg6, lid0, phase, st.add((lid0 / 32) as usize * 1280));
			}
		}
	}
	{
		let phase = 1u32;
		let mut lid0 = 0u32;
		while lid0 + 32 <= n {
			let mut live = splat(u32::MAX);
			let gid0 = l.group[0] * l.block[0] + lid0;
			'chunk: {
				b0_r0r1r2sr6gi_f(l, lid0, arg0, arg1, arg2, x3, arg6, lid0, phase, st.add((lid0 / 32) as usize * 1280));
			}
			lid0 += 32;
		}
		if lid0 < n {
			// Lanes part: the group's last threads, those under `n`.
			let t847: [uint32x4_t; 8] = core::array::from_fn(|r| vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[r].as_ptr())), vdupq_n_u32(n)));
			let mut live = t847;
			let gid0 = l.group[0] * l.block[0] + lid0;
			'chunk: {
				b0_r0r1r2sr6gi_p(l, lid0, live, arg0, arg1, arg2, x3, arg6, lid0, phase, st.add((lid0 / 32) as usize * 1280));
			}
		}
	}
}
