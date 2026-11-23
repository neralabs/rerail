//! `nearest_jump` at 8 lanes (2 NEON registers), written by rerail-ukl-vec's Rust (`rs.rs`) from its vector tape: do
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

/// `nearest_jump_inner`.
#[inline(always)]
unsafe fn b0_r0r1sg_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: *const u8) {
	let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x2.wrapping_add(4u64 as usize) as *const u32).read_unaligned();
    let a1: u32 = (x2 as *const u32).read_unaligned();
	let mut x4 = [vdupq_n_u32(0); 2];
	let mut x5 = [vdupq_n_u32(0); 2];
	let mut x6 = [vdupq_n_u32(0); 2];
	let mut x7 = [vdupq_n_u32(0); 2];
	let mut live = k;
	let t0: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
	let t1 = [vdupq_n_u32(a0); 2];
	let t2: [uint32x4_t; 2] = [vcgeq_u32(t0[0], t1[0]), vcgeq_u32(t0[1], t1[1])];
	let t3: [uint32x4_t; 2] = [vandq_u32(live[0], t2[0]), vandq_u32(live[1], t2[1])];
	{
		live = [vbicq_u32(live[0], t3[0]), vbicq_u32(live[1], t3[1])];
	}
	if any(&live) {
		let t4 = b1_ag4r0_p(l, lid0, live, arg0);
		x4 = t4;
		let t5 = [vdupq_n_u32(a1); 2];
		let mut t6x = [0u32; 8];
		let mut t6y = [0u32; 8];
		for r in 0..2 { vst1q_u32(t6x.as_mut_ptr().add(4 * r), t0[r]); vst1q_u32(t6y.as_mut_ptr().add(4 * r), t5[r]); }
		for j in 0..8 { t6x[j] = (t6x[j] as u32).checked_div(t6y[j] as u32).unwrap_or(0) as u32; }
		let t7: [uint32x4_t; 2] = [vld1q_u32(t6x.as_ptr().add(0)), vld1q_u32(t6x.as_ptr().add(4))];
		let t8: [uint32x4_t; 2] = [vmulq_u32(t7[0], t5[0]), vmulq_u32(t7[1], t5[1])];
		x5 = t8;
		let t9 = [vdupq_n_u32(0u32); 2];
		x6 = t9;
		let mut t10 = live;
		loop {
			// Lanes part: each its own rounds.
			let t11 = [vdupq_n_u32(2147483648u32); 2];
			let t12: [uint32x4_t; 2] = [vandq_u32(x4[0], t11[0]), vandq_u32(x4[1], t11[1])];
			let t13: [uint32x4_t; 2] = [vceqq_u32(t12[0], t9[0]), vceqq_u32(t12[1], t9[1])];
			let t14: [uint32x4_t; 2] = [vcltq_u32(x6[0], t5[0]), vcltq_u32(x6[1], t5[1])];
			let t15: [uint32x4_t; 2] = [vandq_u32(t13[0], t14[0]), vandq_u32(t13[1], t14[1])];
			t10 = [vandq_u32(t10[0], t15[0]), vandq_u32(t10[1], t15[1])];
			if !(any(&t10)) { break; }
			let t16: [uint32x4_t; 2] = [vaddq_u32(x5[0], x4[0]), vaddq_u32(x5[1], x4[1])];
			let t17 = b1_al4r0_p(l, lid0, t10, arg0, t16);
			x7 = t17;
			x4 = x7;
			let t18 = x6;
			x6 = [vsubq_u32(x6[0], t10[0]), vsubq_u32(x6[1], t10[1])];
			b2_ag4r0l_p(l, lid0, t10, arg0, x4);
		}
		let t19 = b3_o1l_p(l, lid0, live, arg1.wrapping_add((0u64).wrapping_mul(4u64) as usize), x6);
	}
}

/// `nearest_jump_inner`.
#[inline(always)]
unsafe fn b0_r0r1sg_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: *const u8) {
	let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x2.wrapping_add(4u64 as usize) as *const u32).read_unaligned();
    let a1: u32 = (x2 as *const u32).read_unaligned();
	let mut x4 = [vdupq_n_u32(0); 2];
	let mut x5 = [vdupq_n_u32(0); 2];
	let mut x6 = [vdupq_n_u32(0); 2];
	let mut x7 = [vdupq_n_u32(0); 2];
	let mut live = [vdupq_n_u32(u32::MAX); 2];
	if ((gid0) as u64 + 7) < ((a0) as u64) {
		let t0 = b1_ag4r0_f(l, lid0, arg0);
		x4 = t0;
		let t1: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
		let t2 = [vdupq_n_u32(a1); 2];
		let mut t3x = [0u32; 8];
		let mut t3y = [0u32; 8];
		for r in 0..2 { vst1q_u32(t3x.as_mut_ptr().add(4 * r), t1[r]); vst1q_u32(t3y.as_mut_ptr().add(4 * r), t2[r]); }
		for j in 0..8 { t3x[j] = (t3x[j] as u32).checked_div(t3y[j] as u32).unwrap_or(0) as u32; }
		let t4: [uint32x4_t; 2] = [vld1q_u32(t3x.as_ptr().add(0)), vld1q_u32(t3x.as_ptr().add(4))];
		let t5: [uint32x4_t; 2] = [vmulq_u32(t4[0], t2[0]), vmulq_u32(t4[1], t2[1])];
		x5 = t5;
		let t6 = [vdupq_n_u32(0u32); 2];
		x6 = t6;
		let mut t7 = live;
		loop {
			// Lanes part: each its own rounds.
			let t8 = [vdupq_n_u32(2147483648u32); 2];
			let t9: [uint32x4_t; 2] = [vandq_u32(x4[0], t8[0]), vandq_u32(x4[1], t8[1])];
			let t10: [uint32x4_t; 2] = [vceqq_u32(t9[0], t6[0]), vceqq_u32(t9[1], t6[1])];
			let t11: [uint32x4_t; 2] = [vcltq_u32(x6[0], t2[0]), vcltq_u32(x6[1], t2[1])];
			let t12: [uint32x4_t; 2] = [vandq_u32(t10[0], t11[0]), vandq_u32(t10[1], t11[1])];
			t7 = [vandq_u32(t7[0], t12[0]), vandq_u32(t7[1], t12[1])];
			if !(any(&t7)) { break; }
			let t13: [uint32x4_t; 2] = [vaddq_u32(x5[0], x4[0]), vaddq_u32(x5[1], x4[1])];
			let t14 = b1_al4r0_p_v0(l, lid0, t7, arg0, t13);
			x7 = t14;
			x4 = x7;
			let t15 = x6;
			x6 = [vsubq_u32(x6[0], t7[0]), vsubq_u32(x6[1], t7[1])];
			b2_ag4r0l_p_v0(l, lid0, t7, arg0, x4);
		}
		let t16 = b3_o1l_f(l, lid0, arg1.wrapping_add((0u64).wrapping_mul(4u64) as usize), x6);
	} else if (((gid0) as u64 + 7) <= u32::MAX as u64 && ((gid0) as u64) >= ((a0) as u64)) {
		return;
	} else {
		// Lanes part: the chunk's lanes on both sides of the bound.
		let t17: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
		let t18 = [vdupq_n_u32(a0); 2];
		let t19: [uint32x4_t; 2] = [vcgeq_u32(t17[0], t18[0]), vcgeq_u32(t17[1], t18[1])];
		let t20: [uint32x4_t; 2] = [vandq_u32(live[0], t19[0]), vandq_u32(live[1], t19[1])];
		{
			live = [vbicq_u32(live[0], t20[0]), vbicq_u32(live[1], t20[1])];
		}
		if any(&live) {
			let t21 = b1_ag4r0_p(l, lid0, live, arg0);
			x4 = t21;
			let t22 = [vdupq_n_u32(a1); 2];
			let mut t23x = [0u32; 8];
			let mut t23y = [0u32; 8];
			for r in 0..2 { vst1q_u32(t23x.as_mut_ptr().add(4 * r), t17[r]); vst1q_u32(t23y.as_mut_ptr().add(4 * r), t22[r]); }
			for j in 0..8 { t23x[j] = (t23x[j] as u32).checked_div(t23y[j] as u32).unwrap_or(0) as u32; }
			let t24: [uint32x4_t; 2] = [vld1q_u32(t23x.as_ptr().add(0)), vld1q_u32(t23x.as_ptr().add(4))];
			let t25: [uint32x4_t; 2] = [vmulq_u32(t24[0], t22[0]), vmulq_u32(t24[1], t22[1])];
			x5 = t25;
			let t26 = [vdupq_n_u32(0u32); 2];
			x6 = t26;
			let mut t27 = live;
			loop {
				// Lanes part: each its own rounds.
				let t28 = [vdupq_n_u32(2147483648u32); 2];
				let t29: [uint32x4_t; 2] = [vandq_u32(x4[0], t28[0]), vandq_u32(x4[1], t28[1])];
				let t30: [uint32x4_t; 2] = [vceqq_u32(t29[0], t26[0]), vceqq_u32(t29[1], t26[1])];
				let t31: [uint32x4_t; 2] = [vcltq_u32(x6[0], t22[0]), vcltq_u32(x6[1], t22[1])];
				let t32: [uint32x4_t; 2] = [vandq_u32(t30[0], t31[0]), vandq_u32(t30[1], t31[1])];
				t27 = [vandq_u32(t27[0], t32[0]), vandq_u32(t27[1], t32[1])];
				if !(any(&t27)) { break; }
				let t33: [uint32x4_t; 2] = [vaddq_u32(x5[0], x4[0]), vaddq_u32(x5[1], x4[1])];
				let t34 = b1_al4r0_p(l, lid0, t27, arg0, t33);
				x7 = t34;
				x4 = x7;
				let t35 = x6;
				x6 = [vsubq_u32(x6[0], t27[0]), vsubq_u32(x6[1], t27[1])];
				b2_ag4r0l_p(l, lid0, t27, arg0, x4);
			}
			let t36 = b3_o1l_p(l, lid0, live, arg1.wrapping_add((0u64).wrapping_mul(4u64) as usize), x6);
		}
	}
}

/// `ukl_atomic_load`.
#[inline(always)]
unsafe fn b1_al4r0_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x0i: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = vdupq_n_u32(maxv_on(&live, &x0i));
	let t1: [uint32x4_t; 2] = [vbslq_u32(live[0], x0i[0], t0), vbslq_u32(live[1], x0i[1], t0)];
	let t2 = arg0 as *const u32;
	let mut t3 = [vdupq_n_u32(0); 2];
	t3[0] = vld1q_lane_u32::<0>(t2.add(vgetq_lane_u32::<0>(t1[0]) as usize), t3[0]);
	t3[0] = vld1q_lane_u32::<1>(t2.add(vgetq_lane_u32::<1>(t1[0]) as usize), t3[0]);
	t3[0] = vld1q_lane_u32::<2>(t2.add(vgetq_lane_u32::<2>(t1[0]) as usize), t3[0]);
	t3[0] = vld1q_lane_u32::<3>(t2.add(vgetq_lane_u32::<3>(t1[0]) as usize), t3[0]);
	t3[1] = vld1q_lane_u32::<0>(t2.add(vgetq_lane_u32::<0>(t1[1]) as usize), t3[1]);
	t3[1] = vld1q_lane_u32::<1>(t2.add(vgetq_lane_u32::<1>(t1[1]) as usize), t3[1]);
	t3[1] = vld1q_lane_u32::<2>(t2.add(vgetq_lane_u32::<2>(t1[1]) as usize), t3[1]);
	t3[1] = vld1q_lane_u32::<3>(t2.add(vgetq_lane_u32::<3>(t1[1]) as usize), t3[1]);
	res = t3;
	return res;
}

/// `ukl_atomic_load`.
#[inline(always)]
unsafe fn b1_al4r0_p_v0(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x0i: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
	let gid0 = l.group[0] * l.block[0] + lid0;
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
	let t1: [uint32x4_t; 2] = [vbslq_u32(live[0], x0i[0], t0[0]), vbslq_u32(live[1], x0i[1], t0[1])];
	let t2 = arg0 as *const u32;
	let mut t3 = [vdupq_n_u32(0); 2];
	t3[0] = vld1q_lane_u32::<0>(t2.add(vgetq_lane_u32::<0>(t1[0]) as usize), t3[0]);
	t3[0] = vld1q_lane_u32::<1>(t2.add(vgetq_lane_u32::<1>(t1[0]) as usize), t3[0]);
	t3[0] = vld1q_lane_u32::<2>(t2.add(vgetq_lane_u32::<2>(t1[0]) as usize), t3[0]);
	t3[0] = vld1q_lane_u32::<3>(t2.add(vgetq_lane_u32::<3>(t1[0]) as usize), t3[0]);
	t3[1] = vld1q_lane_u32::<0>(t2.add(vgetq_lane_u32::<0>(t1[1]) as usize), t3[1]);
	t3[1] = vld1q_lane_u32::<1>(t2.add(vgetq_lane_u32::<1>(t1[1]) as usize), t3[1]);
	t3[1] = vld1q_lane_u32::<2>(t2.add(vgetq_lane_u32::<2>(t1[1]) as usize), t3[1]);
	t3[1] = vld1q_lane_u32::<3>(t2.add(vgetq_lane_u32::<3>(t1[1]) as usize), t3[1]);
	res = t3;
	return res;
}

/// `ukl_atomic_load`.
#[inline(always)]
unsafe fn b1_ag4r0_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8) -> [uint32x4_t; 2] {
	let gid0 = l.group[0] * l.block[0] + lid0;
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let mut t0: [uint32x4_t; 2];
	if all(&live) {
		let t1 = (arg0 as *const u32).add((gid0) as usize);
		t0 = [vld1q_u32(t1.add(0)), vld1q_u32(t1.add(4))];
	} else {
		let t2: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
		let t3 = vdupq_n_u32(maxv_on(&live, &t2));
		let t4: [uint32x4_t; 2] = [vbslq_u32(live[0], t2[0], t3), vbslq_u32(live[1], t2[1], t3)];
		let t5 = arg0 as *const u32;
		let mut t6 = [vdupq_n_u32(0); 2];
		t6[0] = vld1q_lane_u32::<0>(t5.add(vgetq_lane_u32::<0>(t4[0]) as usize), t6[0]);
		t6[0] = vld1q_lane_u32::<1>(t5.add(vgetq_lane_u32::<1>(t4[0]) as usize), t6[0]);
		t6[0] = vld1q_lane_u32::<2>(t5.add(vgetq_lane_u32::<2>(t4[0]) as usize), t6[0]);
		t6[0] = vld1q_lane_u32::<3>(t5.add(vgetq_lane_u32::<3>(t4[0]) as usize), t6[0]);
		t6[1] = vld1q_lane_u32::<0>(t5.add(vgetq_lane_u32::<0>(t4[1]) as usize), t6[1]);
		t6[1] = vld1q_lane_u32::<1>(t5.add(vgetq_lane_u32::<1>(t4[1]) as usize), t6[1]);
		t6[1] = vld1q_lane_u32::<2>(t5.add(vgetq_lane_u32::<2>(t4[1]) as usize), t6[1]);
		t6[1] = vld1q_lane_u32::<3>(t5.add(vgetq_lane_u32::<3>(t4[1]) as usize), t6[1]);
		t0 = t6;
	}
	res = t0;
	return res;
}

/// `ukl_atomic_load`.
#[inline(always)]
unsafe fn b1_ag4r0_f(l: &Launch, lid0: u32, arg0: *const u8) -> [uint32x4_t; 2] {
	let gid0 = l.group[0] * l.block[0] + lid0;
	let mut live = [vdupq_n_u32(u32::MAX); 2];
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = (arg0 as *const u32).add((gid0) as usize);
	let t1: [uint32x4_t; 2] = [vld1q_u32(t0.add(0)), vld1q_u32(t0.add(4))];
	res = t1;
	return res;
}

/// `ukl_atomic_store`.
#[inline(always)]
unsafe fn b2_ag4r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1: [uint32x4_t; 2]) {
	let gid0 = l.group[0] * l.block[0] + lid0;
	let mut live = k;
	let t0 = (arg0 as *mut u32).add((gid0) as usize);
	if all(&live) {
		vst1q_u32(t0.add(0), x1[0]);
		vst1q_u32(t0.add(4), x1[1]);
	} else {
		let mut t1w = 0u32;
		let t1 = core::hint::black_box(&raw mut t1w);
		core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t0.add(0), t1).write(vgetq_lane_u32::<0>(x1[0]));
		core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t0.add(1), t1).write(vgetq_lane_u32::<1>(x1[0]));
		core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t0.add(2), t1).write(vgetq_lane_u32::<2>(x1[0]));
		core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t0.add(3), t1).write(vgetq_lane_u32::<3>(x1[0]));
		core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t0.add(4), t1).write(vgetq_lane_u32::<0>(x1[1]));
		core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t0.add(5), t1).write(vgetq_lane_u32::<1>(x1[1]));
		core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t0.add(6), t1).write(vgetq_lane_u32::<2>(x1[1]));
		core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t0.add(7), t1).write(vgetq_lane_u32::<3>(x1[1]));
	}
}

/// `ukl_atomic_store`.
#[inline(always)]
unsafe fn b2_ag4r0l_p_v0(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1: [uint32x4_t; 2]) {
	let gid0 = l.group[0] * l.block[0] + lid0;
	let mut live = k;
	let t0 = (arg0 as *mut u32).add((gid0) as usize);
	vst1q_u32(t0.add(0), vbslq_u32(live[0], x1[0], vld1q_u32(t0.add(0))));
	vst1q_u32(t0.add(4), vbslq_u32(live[1], x1[1], vld1q_u32(t0.add(4))));
}

/// `ukl_atomic_max`.
#[inline(always)]
unsafe fn b3_o1l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg1_x0: *const u8, x1: [uint32x4_t; 2]) -> u32 {
	let mut live = k;
	let mut res: u32 = 0;
	let t0: [uint32x4_t; 2] = [vbslq_u32(live[0], x1[0], vdupq_n_u32(0)), vbslq_u32(live[1], x1[1], vdupq_n_u32(0))];
	let t1 = vmaxvq_u32(vmaxq_u32(t0[0], t0[1]));
	if t1 != 0 { AtomicU32::from_ptr(arg1_x0 as *mut u32).fetch_max(t1, Relaxed); }
	return res;
}

/// `ukl_atomic_max`.
#[inline(always)]
unsafe fn b3_o1l_f(l: &Launch, lid0: u32, arg1_x0: *const u8, x1: [uint32x4_t; 2]) -> u32 {
	let mut live = [vdupq_n_u32(u32::MAX); 2];
	let mut res: u32 = 0;
	let t0 = vmaxvq_u32(vmaxq_u32(x1[0], x1[1]));
	if t0 != 0 { AtomicU32::from_ptr(arg1_x0 as *mut u32).fetch_max(t0, Relaxed); }
	return res;
}

/// The group `l.group`'s threads, 8 at a time: `argv[j]` the address of argument `j`'s bytes (a buffer's address,
/// a value's bytes).
///
/// # Safety
///
/// `argv` holds the kernel's arguments, its buffers as large as it reads and writes; `l` is a 1-D group's launch.
pub unsafe fn nearest_jump_w8(argv: *const *const u8, l: &Launch) {
	let n = l.block[0] * l.block[1] * l.block[2];
	let arg0: *const u8 = (*argv.add(0) as *const *const u8).read_unaligned();
	let arg1: *const u8 = (*argv.add(1) as *const *const u8).read_unaligned();
	let x2: *const u8 = *argv.add(2);
	let arg3: *const u8 = (*argv.add(3) as *const *const u8).read_unaligned();
	let x4: u32 = (*argv.add(4) as *const u32).read_unaligned();
	let mut t107: bool = !(arg3).is_null();
	if t107 {
		let mut t108: bool = ((l.group[0]) >= ((arg3.wrapping_add((((x4) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
		if !t108 {
			t108 = ((l.group[1]) >= ((arg3.wrapping_add(((((x4).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
		}
		let mut t109: bool = t108;
		if !t109 {
			t109 = ((l.group[2]) >= ((arg3.wrapping_add(((((x4).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
		}
		t107 = t109;
	}
	if t107 {
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
		let t110: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
		let mut live = t110;
		let gid0 = l.group[0] * l.block[0] + lid0;
		'chunk: {
			b0_r0r1sg_p(l, lid0, live, arg0, arg1, x2);
		}
	}
}
