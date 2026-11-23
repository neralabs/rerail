//! `parent_link` at 32 lanes (8 NEON registers), written by rerail-ukl-vec's Rust (`rs.rs`) from its vector tape: do
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

/// `parent_link_inner`.
#[inline(always)]
unsafe fn b0_r0r1sg_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg0: *const u8, arg1: *const u8, x2: *const u8) {
	let gid0 = l.group[0] * l.block[0] + lid0;
	let mut x4 = [Z; 8];
	let mut x5: u32 = 0;
	let mut x6 = [Z; 8];
	let mut live = k;
	let t0 = iota(gid0);
	let t1 = splat((x2 as *const u32).read_unaligned());
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
		x5 = 0u32;
		let mut t10 = live;
		loop {
			let t11 = bits(&t10);
			if t11.count_ones() <= 2 {
				let mut t12 = t11;
				while t12 != 0 {
					let j = t12.trailing_zeros() as usize;
					t12 &= t12 - 1;
					let t13 = words8(x4);
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
					let mut t17 = words8(x4);
					t17[j] = x4_j as u32;
					x4 = regs8(t17);
				}
				break;
			}
			// Lanes part: each its own rounds.
			let t18 = splat(4294967295u32);
			let t19 = cne_r(x4, t18);
			let t20: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(vdupq_n_u32(if ((x5) < (7u32)) { u32::MAX } else { 0 }), t19[r]));
			let t21 = splat(2147483648u32);
			let t22 = and_r(x4, t21);
			let t23 = [Z; 8];
			let t24 = cne_r(t22, t23);
			let t25 = and_r(t20, t24);
			t10 = and_r(t10, t25);
			if !(any(&t10)) { break; }
			let t26 = splat(2147483647u32);
			let t27 = and_r(x4, t26);
			let t28 = ix_on(t10, t27);
			let t29 = arg0 as *const u32;
			let mut t30 = [Z; 8];
			let t31 = words8(t28);
			t30 = ldj_u32(|j| t29.add(t31[j] as usize) as *const u8);
			x4 = bsl_r(t10, t30, x4);
			let t32 = and_r(t10, live);
			let t33 = x5;
			x5 = (x5).wrapping_add(1u32);
		}
		let t34 = splat(4294967295u32);
		let t35 = cne_r(x4, t34);
		let t36 = splat(2147483648u32);
		let t37 = and_r(x4, t36);
		let t38 = [Z; 8];
		let t39 = cne_r(t37, t38);
		let t40 = and_r(t35, t39);
		x6 = t40;
		let mut t41 = [Z; 8];
		let t42 = and_r(live, x6);
		let t43 = bic_r(live, x6);
		{
			t41 = bsl_r(t42, t34, t41);
		}
		{
			t41 = bsl_r(t43, x4, t41);
		}
		let t44 = (arg1 as *mut u32).add((gid0) as usize);
		if all(&live) {
			vst1q_u32(t44.add(0), t41[0]);
			vst1q_u32(t44.add(4), t41[1]);
			vst1q_u32(t44.add(8), t41[2]);
			vst1q_u32(t44.add(12), t41[3]);
			vst1q_u32(t44.add(16), t41[4]);
			vst1q_u32(t44.add(20), t41[5]);
			vst1q_u32(t44.add(24), t41[6]);
			vst1q_u32(t44.add(28), t41[7]);
		} else {
			stj_u32(live, t41, |j| t44.add(j) as *mut u8);
		}
		let t45 = and_r(live, x6);
		{
			let t46 = iota(gid0.wrapping_add(1u32));
			let t47 = bits(&t45);
			if t47 != 0 {
			    let j = 31 - t47.leading_zeros() as usize;
			    (arg1.wrapping_add(((((x2 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(words8(t46)[j]);
			}
		}
	}
}

/// `parent_link_inner`.
#[inline(always)]
unsafe fn b0_r0r1sg_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: *const u8) {
	let gid0 = l.group[0] * l.block[0] + lid0;
	let mut x4 = [Z; 8];
	let mut x5: u32 = 0;
	let mut x6 = [Z; 8];
	let mut live = splat(u32::MAX);
	if ((gid0) as u64 + 31) < (((x2 as *const u32).read_unaligned()) as u64) {
		let t0 = (arg0 as *const u32).add((gid0) as usize);
		let t1: [uint32x4_t; 8] = [vld1q_u32(t0.add(0)), vld1q_u32(t0.add(4)), vld1q_u32(t0.add(8)), vld1q_u32(t0.add(12)), vld1q_u32(t0.add(16)), vld1q_u32(t0.add(20)), vld1q_u32(t0.add(24)), vld1q_u32(t0.add(28))];
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
					let t5 = words8(x4);
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
					let mut t9 = words8(x4);
					t9[j] = x4_j as u32;
					x4 = regs8(t9);
				}
				break;
			}
			// Lanes part: each its own rounds.
			let t10 = splat(4294967295u32);
			let t11 = cne_r(x4, t10);
			let t12: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(vdupq_n_u32(if ((x5) < (7u32)) { u32::MAX } else { 0 }), t11[r]));
			let t13 = splat(2147483648u32);
			let t14 = and_r(x4, t13);
			let t15 = [Z; 8];
			let t16 = cne_r(t14, t15);
			let t17 = and_r(t12, t16);
			t2 = and_r(t2, t17);
			if !(any(&t2)) { break; }
			let t18 = splat(2147483647u32);
			let t19 = and_r(x4, t18);
			let t20 = iota(gid0);
			let t21 = bsl_r(t2, t19, t20);
			let t22 = arg0 as *const u32;
			let mut t23 = [Z; 8];
			let t24 = words8(t21);
			t23 = ldj_u32(|j| t22.add(t24[j] as usize) as *const u8);
			x4 = bsl_r(t2, t23, x4);
			let t25 = and_r(t2, live);
			let t26 = x5;
			x5 = (x5).wrapping_add(1u32);
		}
		let t27 = splat(4294967295u32);
		let t28 = cne_r(x4, t27);
		let t29 = splat(2147483648u32);
		let t30 = and_r(x4, t29);
		let t31 = [Z; 8];
		let t32 = cne_r(t30, t31);
		let t33 = and_r(t28, t32);
		x6 = t33;
		let mut t34 = [Z; 8];
		let t35 = mvn_r(x6);
		{
			t34 = bsl_r(x6, t27, t34);
		}
		{
			t34 = bsl_r(t35, x4, t34);
		}
		let t36 = (arg1 as *mut u32).add((gid0) as usize);
		vst1q_u32(t36.add(0), t34[0]);
		vst1q_u32(t36.add(4), t34[1]);
		vst1q_u32(t36.add(8), t34[2]);
		vst1q_u32(t36.add(12), t34[3]);
		vst1q_u32(t36.add(16), t34[4]);
		vst1q_u32(t36.add(20), t34[5]);
		vst1q_u32(t36.add(24), t34[6]);
		vst1q_u32(t36.add(28), t34[7]);
		{
			let t37 = iota(gid0.wrapping_add(1u32));
			let t38 = bits(&x6);
			if t38 != 0 {
			    let j = 31 - t38.leading_zeros() as usize;
			    (arg1.wrapping_add(((((x2 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(words8(t37)[j]);
			}
		}
	} else if (((gid0) as u64 + 31) <= u32::MAX as u64 && ((gid0) as u64) >= (((x2 as *const u32).read_unaligned()) as u64)) {
		return;
	} else {
		// Lanes part: the chunk's lanes on both sides of the bound.
		let t39 = iota(gid0);
		let t40 = splat((x2 as *const u32).read_unaligned());
		let t41 = cge_r(t39, t40);
		let t42 = and_r(live, t41);
		{
			live = bic_r(live, t42);
		}
		if any(&live) {
			let mut t43: [uint32x4_t; 8];
			if all(&live) {
				let t44 = (arg0 as *const u32).add((gid0) as usize);
				t43 = [vld1q_u32(t44.add(0)), vld1q_u32(t44.add(4)), vld1q_u32(t44.add(8)), vld1q_u32(t44.add(12)), vld1q_u32(t44.add(16)), vld1q_u32(t44.add(20)), vld1q_u32(t44.add(24)), vld1q_u32(t44.add(28))];
			} else {
				let t45 = ix_on(live, t39);
				let t46 = arg0 as *const u32;
				let mut t47 = [Z; 8];
				let t48 = words8(t45);
				t47 = ldj_u32(|j| t46.add(t48[j] as usize) as *const u8);
				t43 = t47;
			}
			x4 = t43;
			x5 = 0u32;
			let mut t49 = live;
			loop {
				let t50 = bits(&t49);
				if t50.count_ones() <= 2 {
					let mut t51 = t50;
					while t51 != 0 {
						let j = t51.trailing_zeros() as usize;
						t51 &= t51 - 1;
						let t52 = words8(x4);
						let mut x4_j: u32 = t52[j];
						let mut x5_j: u32 = x5;
						loop {
							let mut t53: bool = ((x5_j) < (7u32));
							if t53 {
								t53 = ((x4_j) != (4294967295u32));
							}
							let mut t54: bool = t53;
							if t54 {
								t54 = ((((x4_j) & (2147483648u32))) != (0u32));
							}
							if !(t54) { break; }
							x4_j = (arg0.wrapping_add((((((x4_j) & (2147483647u32))) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned();
							let t55 = x5_j;
							x5_j = (x5_j).wrapping_add(1u32);
						}
						let mut t56 = words8(x4);
						t56[j] = x4_j as u32;
						x4 = regs8(t56);
					}
					break;
				}
				// Lanes part: each its own rounds.
				let t57 = splat(4294967295u32);
				let t58 = cne_r(x4, t57);
				let t59: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(vdupq_n_u32(if ((x5) < (7u32)) { u32::MAX } else { 0 }), t58[r]));
				let t60 = splat(2147483648u32);
				let t61 = and_r(x4, t60);
				let t62 = [Z; 8];
				let t63 = cne_r(t61, t62);
				let t64 = and_r(t59, t63);
				t49 = and_r(t49, t64);
				if !(any(&t49)) { break; }
				let t65 = splat(2147483647u32);
				let t66 = and_r(x4, t65);
				let t67 = ix_on(t49, t66);
				let t68 = arg0 as *const u32;
				let mut t69 = [Z; 8];
				let t70 = words8(t67);
				t69 = ldj_u32(|j| t68.add(t70[j] as usize) as *const u8);
				x4 = bsl_r(t49, t69, x4);
				let t71 = and_r(t49, live);
				let t72 = x5;
				x5 = (x5).wrapping_add(1u32);
			}
			let t73 = splat(4294967295u32);
			let t74 = cne_r(x4, t73);
			let t75 = splat(2147483648u32);
			let t76 = and_r(x4, t75);
			let t77 = [Z; 8];
			let t78 = cne_r(t76, t77);
			let t79 = and_r(t74, t78);
			x6 = t79;
			let mut t80 = [Z; 8];
			let t81 = and_r(live, x6);
			let t82 = bic_r(live, x6);
			{
				t80 = bsl_r(t81, t73, t80);
			}
			{
				t80 = bsl_r(t82, x4, t80);
			}
			let t83 = (arg1 as *mut u32).add((gid0) as usize);
			if all(&live) {
				vst1q_u32(t83.add(0), t80[0]);
				vst1q_u32(t83.add(4), t80[1]);
				vst1q_u32(t83.add(8), t80[2]);
				vst1q_u32(t83.add(12), t80[3]);
				vst1q_u32(t83.add(16), t80[4]);
				vst1q_u32(t83.add(20), t80[5]);
				vst1q_u32(t83.add(24), t80[6]);
				vst1q_u32(t83.add(28), t80[7]);
			} else {
				stj_u32(live, t80, |j| t83.add(j) as *mut u8);
			}
			let t84 = and_r(live, x6);
			{
				let t85 = iota(gid0.wrapping_add(1u32));
				let t86 = bits(&t84);
				if t86 != 0 {
				    let j = 31 - t86.leading_zeros() as usize;
				    (arg1.wrapping_add(((((x2 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(words8(t85)[j]);
				}
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
pub unsafe fn parent_link_w32(argv: *const *const u8, l: &Launch) {
	let n = l.block[0] * l.block[1] * l.block[2];
	let arg0: *const u8 = (*argv.add(0) as *const *const u8).read_unaligned();
	let arg1: *const u8 = (*argv.add(1) as *const *const u8).read_unaligned();
	let x2: *const u8 = *argv.add(2);
	let arg3: *const u8 = (*argv.add(3) as *const *const u8).read_unaligned();
	let x4: u32 = (*argv.add(4) as *const u32).read_unaligned();
	let mut t181: bool = !(arg3).is_null();
	if t181 {
		let mut t182: bool = ((l.group[0]) >= ((arg3.wrapping_add((((x4) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
		if !t182 {
			t182 = ((l.group[1]) >= ((arg3.wrapping_add(((((x4).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
		}
		let mut t183: bool = t182;
		if !t183 {
			t183 = ((l.group[2]) >= ((arg3.wrapping_add(((((x4).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
		}
		t181 = t183;
	}
	if t181 {
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
		let t184: [uint32x4_t; 8] = core::array::from_fn(|r| vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[r].as_ptr())), vdupq_n_u32(n)));
		let mut live = t184;
		let gid0 = l.group[0] * l.block[0] + lid0;
		'chunk: {
			b0_r0r1sg_p(l, lid0, live, arg0, arg1, x2);
		}
	}
}
