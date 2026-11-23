//! `parent_step` at 8 lanes (2 NEON registers), written by rerail-ukl-vec's Rust (`rs.rs`) from its vector tape: do
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

/// `parent_step_inner`.
#[inline(always)]
unsafe fn b0_r0r1r2sg_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, arg2: *const u8, x3: *const u8) {
	let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x3 as *const u32).read_unaligned();
	let mut x5 = [vdupq_n_u32(0); 4];
	let mut live = k;
	let t0: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
	let t1 = [vdupq_n_u32(a0); 2];
	let t2: [uint32x4_t; 2] = [vcgeq_u32(t0[0], t1[0]), vcgeq_u32(t0[1], t1[1])];
	let t3: [uint32x4_t; 2] = [vandq_u32(live[0], t2[0]), vandq_u32(live[1], t2[1])];
	{
		live = [vbicq_u32(live[0], t3[0]), vbicq_u32(live[1], t3[1])];
	}
	if any(&live) {
		let t4 = b1_r0r1g_p(l, lid0, live, arg0, arg1);
		x5 = t4;
		let t5: u64 = (1u64).wrapping_shl((32u64) as u32);
		let t6 = [vdupq_n_u32((t5) as u32); 2];
		let t7 = [vdupq_n_u32(((t5) >> 32) as u32); 2];
		let t8: [uint32x4_t; 4] = [vandq_u32(x5[0], t6[0]), vandq_u32(x5[1], t6[1]), vandq_u32(x5[2], t7[0]), vandq_u32(x5[3], t7[1])];
		let t9: u64 = 0u64;
		let t10 = [vdupq_n_u32((t9) as u32); 2];
		let t11: [uint32x4_t; 2] = [veq64q_u32(t8[0], t8[2], t10[0], vdupq_n_u32(0)), veq64q_u32(t8[1], t8[3], t10[1], vdupq_n_u32(0))];
		let t12: [uint32x4_t; 2] = [vmvnq_u32(t11[0]), vmvnq_u32(t11[1])];
		let mut t13 = [vdupq_n_u32(0); 2];
		let t14: [uint32x4_t; 2] = [vandq_u32(live[0], t12[0]), vandq_u32(live[1], t12[1])];
		let t15: [uint32x4_t; 2] = [vbicq_u32(live[0], t12[0]), vbicq_u32(live[1], t12[1])];
		{
			let t16: [uint32x4_t; 2] = [x5[0], x5[1]];
			let t17 = [vdupq_n_u32(2147483648u32); 2];
			let t18: [uint32x4_t; 2] = [vorrq_u32(t17[0], t16[0]), vorrq_u32(t17[1], t16[1])];
			t13 = [vbslq_u32(t14[0], t18[0], t13[0]), vbslq_u32(t14[1], t18[1], t13[1])];
		}
		{
			let t19: [uint32x4_t; 2] = [x5[0], x5[1]];
			t13 = [vbslq_u32(t15[0], t19[0], t13[0]), vbslq_u32(t15[1], t19[1], t13[1])];
		}
		let t20 = (arg2 as *mut u32).add((gid0) as usize);
		if all(&live) {
			vst1q_u32(t20.add(0), t13[0]);
			vst1q_u32(t20.add(4), t13[1]);
		} else {
			let mut t21w = 0u32;
			let t21 = core::hint::black_box(&raw mut t21w);
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t20.add(0), t21).write(vgetq_lane_u32::<0>(t13[0]));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t20.add(1), t21).write(vgetq_lane_u32::<1>(t13[0]));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t20.add(2), t21).write(vgetq_lane_u32::<2>(t13[0]));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t20.add(3), t21).write(vgetq_lane_u32::<3>(t13[0]));
			core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t20.add(4), t21).write(vgetq_lane_u32::<0>(t13[1]));
			core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t20.add(5), t21).write(vgetq_lane_u32::<1>(t13[1]));
			core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t20.add(6), t21).write(vgetq_lane_u32::<2>(t13[1]));
			core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t20.add(7), t21).write(vgetq_lane_u32::<3>(t13[1]));
		}
	}
}

/// `parent_step_inner`.
#[inline(always)]
unsafe fn b0_r0r1r2sg_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, arg2: *const u8, x3: *const u8) {
	let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x3 as *const u32).read_unaligned();
	let mut x5 = [vdupq_n_u32(0); 4];
	let mut live = [vdupq_n_u32(u32::MAX); 2];
	if ((gid0) as u64 + 7) < ((a0) as u64) {
		let t0 = b1_r0r1g_f(l, lid0, arg0, arg1);
		x5 = t0;
		let t1: u64 = (1u64).wrapping_shl((32u64) as u32);
		let t2 = [vdupq_n_u32((t1) as u32); 2];
		let t3 = [vdupq_n_u32(((t1) >> 32) as u32); 2];
		let t4: [uint32x4_t; 4] = [vandq_u32(x5[0], t2[0]), vandq_u32(x5[1], t2[1]), vandq_u32(x5[2], t3[0]), vandq_u32(x5[3], t3[1])];
		let t5: u64 = 0u64;
		let t6 = [vdupq_n_u32((t5) as u32); 2];
		let t7: [uint32x4_t; 2] = [veq64q_u32(t4[0], t4[2], t6[0], vdupq_n_u32(0)), veq64q_u32(t4[1], t4[3], t6[1], vdupq_n_u32(0))];
		let t8: [uint32x4_t; 2] = [vmvnq_u32(t7[0]), vmvnq_u32(t7[1])];
		let mut t9 = [vdupq_n_u32(0); 2];
		let t10: [uint32x4_t; 2] = [vmvnq_u32(t8[0]), vmvnq_u32(t8[1])];
		{
			let t11: [uint32x4_t; 2] = [x5[0], x5[1]];
			let t12 = [vdupq_n_u32(2147483648u32); 2];
			let t13: [uint32x4_t; 2] = [vorrq_u32(t12[0], t11[0]), vorrq_u32(t12[1], t11[1])];
			t9 = [vbslq_u32(t8[0], t13[0], t9[0]), vbslq_u32(t8[1], t13[1], t9[1])];
		}
		{
			let t14: [uint32x4_t; 2] = [x5[0], x5[1]];
			t9 = [vbslq_u32(t10[0], t14[0], t9[0]), vbslq_u32(t10[1], t14[1], t9[1])];
		}
		let t15 = (arg2 as *mut u32).add((gid0) as usize);
		vst1q_u32(t15.add(0), t9[0]);
		vst1q_u32(t15.add(4), t9[1]);
	} else if (((gid0) as u64 + 7) <= u32::MAX as u64 && ((gid0) as u64) >= ((a0) as u64)) {
		return;
	} else {
		// Lanes part: the chunk's lanes on both sides of the bound.
		let t16: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
		let t17 = [vdupq_n_u32(a0); 2];
		let t18: [uint32x4_t; 2] = [vcgeq_u32(t16[0], t17[0]), vcgeq_u32(t16[1], t17[1])];
		let t19: [uint32x4_t; 2] = [vandq_u32(live[0], t18[0]), vandq_u32(live[1], t18[1])];
		{
			live = [vbicq_u32(live[0], t19[0]), vbicq_u32(live[1], t19[1])];
		}
		if any(&live) {
			let t20 = b1_r0r1g_p(l, lid0, live, arg0, arg1);
			x5 = t20;
			let t21: u64 = (1u64).wrapping_shl((32u64) as u32);
			let t22 = [vdupq_n_u32((t21) as u32); 2];
			let t23 = [vdupq_n_u32(((t21) >> 32) as u32); 2];
			let t24: [uint32x4_t; 4] = [vandq_u32(x5[0], t22[0]), vandq_u32(x5[1], t22[1]), vandq_u32(x5[2], t23[0]), vandq_u32(x5[3], t23[1])];
			let t25: u64 = 0u64;
			let t26 = [vdupq_n_u32((t25) as u32); 2];
			let t27: [uint32x4_t; 2] = [veq64q_u32(t24[0], t24[2], t26[0], vdupq_n_u32(0)), veq64q_u32(t24[1], t24[3], t26[1], vdupq_n_u32(0))];
			let t28: [uint32x4_t; 2] = [vmvnq_u32(t27[0]), vmvnq_u32(t27[1])];
			let mut t29 = [vdupq_n_u32(0); 2];
			let t30: [uint32x4_t; 2] = [vandq_u32(live[0], t28[0]), vandq_u32(live[1], t28[1])];
			let t31: [uint32x4_t; 2] = [vbicq_u32(live[0], t28[0]), vbicq_u32(live[1], t28[1])];
			{
				let t32: [uint32x4_t; 2] = [x5[0], x5[1]];
				let t33 = [vdupq_n_u32(2147483648u32); 2];
				let t34: [uint32x4_t; 2] = [vorrq_u32(t33[0], t32[0]), vorrq_u32(t33[1], t32[1])];
				t29 = [vbslq_u32(t30[0], t34[0], t29[0]), vbslq_u32(t30[1], t34[1], t29[1])];
			}
			{
				let t35: [uint32x4_t; 2] = [x5[0], x5[1]];
				t29 = [vbslq_u32(t31[0], t35[0], t29[0]), vbslq_u32(t31[1], t35[1], t29[1])];
			}
			let t36 = (arg2 as *mut u32).add((gid0) as usize);
			if all(&live) {
				vst1q_u32(t36.add(0), t29[0]);
				vst1q_u32(t36.add(4), t29[1]);
			} else {
				let mut t37w = 0u32;
				let t37 = core::hint::black_box(&raw mut t37w);
				core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t36.add(0), t37).write(vgetq_lane_u32::<0>(t29[0]));
				core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t36.add(1), t37).write(vgetq_lane_u32::<1>(t29[0]));
				core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t36.add(2), t37).write(vgetq_lane_u32::<2>(t29[0]));
				core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t36.add(3), t37).write(vgetq_lane_u32::<3>(t29[0]));
				core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t36.add(4), t37).write(vgetq_lane_u32::<0>(t29[1]));
				core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t36.add(5), t37).write(vgetq_lane_u32::<1>(t29[1]));
				core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t36.add(6), t37).write(vgetq_lane_u32::<2>(t29[1]));
				core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t36.add(7), t37).write(vgetq_lane_u32::<3>(t29[1]));
			}
		}
	}
}

/// `clif_f16_parent_step`.
#[inline(always)]
unsafe fn b1_r0r1g_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8) -> [uint32x4_t; 4] {
	let gid0 = l.group[0] * l.block[0] + lid0;
	let mut x3: u32 = 0;
	let mut x4 = [vdupq_n_u32(0); 4];
	let mut x5 = [vdupq_n_u32(0); 2];
	let mut x6 = [vdupq_n_u32(0); 2];
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 4];
	let t0: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
	let t1 = [vdupq_n_u32(0u32); 2];
	let t2: [uint32x4_t; 2] = [vceqq_u32(t0[0], t1[0]), vceqq_u32(t0[1], t1[1])];
	let t3: [uint32x4_t; 2] = [vandq_u32(live[0], t2[0]), vandq_u32(live[1], t2[1])];
	{
		let t4: u64 = 4294967295u64;
		let t5 = [vdupq_n_u32((t4) as u32); 2];
		let t6: [uint32x4_t; 4] = [t5[0], t5[1], vdupq_n_u32(0), vdupq_n_u32(0)];
		res = [vbslq_u32(t3[0], t6[0], res[0]), vbslq_u32(t3[1], t6[1], res[1]), vbslq_u32(t3[0], t6[2], res[2]), vbslq_u32(t3[1], t6[3], res[3])];
		live = [vbicq_u32(live[0], t3[0]), vbicq_u32(live[1], t3[1])];
	}
	if any(&live) {
		x3 = gid0.wrapping_sub(1u32);
		let t7 = b2_r0r1i_p(l, lid0, live, arg0, arg1, x3);
		x4 = t7;
		let t8: u64 = 1u64;
		let t9 = [vdupq_n_u32((t8) as u32); 2];
		let t10: [uint32x4_t; 2] = [vandq_u32(x4[0], t9[0]), vandq_u32(x4[1], t9[1])];
		let t11: u64 = 0u64;
		let t12 = [vdupq_n_u32((t11) as u32); 2];
		let t13: [uint32x4_t; 2] = [vceqq_u32(t10[0], t12[0]), vceqq_u32(t10[1], t12[1])];
		let t14: [uint32x4_t; 2] = [vandq_u32(live[0], t13[0]), vandq_u32(live[1], t13[1])];
		{
			let t15: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[1].as_ptr()))];
			let t16: [uint32x4_t; 4] = [t15[0], t15[1], vdupq_n_u32(0), vdupq_n_u32(0)];
			res = [vbslq_u32(t14[0], t16[0], res[0]), vbslq_u32(t14[1], t16[1], res[1]), vbslq_u32(t14[0], t16[2], res[2]), vbslq_u32(t14[1], t16[3], res[3])];
			live = [vbicq_u32(live[0], t14[0]), vbicq_u32(live[1], t14[1])];
		}
		if any(&live) {
			let t17: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x4[0]), vshlq_n_u32::<30>(x4[2])), vorrq_u32(vshrq_n_u32::<2>(x4[1]), vshlq_n_u32::<30>(x4[3])), vshrq_n_u32::<2>(x4[2]), vshrq_n_u32::<2>(x4[3])];
			let t18: [uint32x4_t; 2] = [t17[0], t17[1]];
			x5 = t18;
			let t19 = [vdupq_n_u32(1u32); 2];
			let t20: [uint32x4_t; 2] = [vsubq_u32(x5[0], t19[0]), vsubq_u32(x5[1], t19[1])];
			let t21: [uint32x4_t; 2] = [vcgeq_u32(t20[0], t0[0]), vcgeq_u32(t20[1], t0[1])];
			let t22: [uint32x4_t; 2] = [vandq_u32(live[0], t21[0]), vandq_u32(live[1], t21[1])];
			{
				let t23: u64 = 4294967295u64;
				let t24 = [vdupq_n_u32((t23) as u32); 2];
				let t25: [uint32x4_t; 4] = [t24[0], t24[1], vdupq_n_u32(0), vdupq_n_u32(0)];
				res = [vbslq_u32(t22[0], t25[0], res[0]), vbslq_u32(t22[1], t25[1], res[1]), vbslq_u32(t22[0], t25[2], res[2]), vbslq_u32(t22[1], t25[3], res[3])];
				live = [vbicq_u32(live[0], t22[0]), vbicq_u32(live[1], t22[1])];
			}
			if any(&live) {
				let t26: [uint32x4_t; 2] = [vsubq_u32(t0[0], x5[0]), vsubq_u32(t0[1], x5[1])];
				x6 = t26;
				let t27: u64 = 2u64;
				let t28 = [vdupq_n_u32((t27) as u32); 2];
				let t29: [uint32x4_t; 2] = [vandq_u32(x4[0], t28[0]), vandq_u32(x4[1], t28[1])];
				let t30: u64 = 0u64;
				let t31 = [vdupq_n_u32((t30) as u32); 2];
				let t32: [uint32x4_t; 2] = [vceqq_u32(t29[0], t31[0]), vceqq_u32(t29[1], t31[1])];
				let t33: [uint32x4_t; 2] = [vmvnq_u32(t32[0]), vmvnq_u32(t32[1])];
				let t34: [uint32x4_t; 2] = [vandq_u32(live[0], t33[0]), vandq_u32(live[1], t33[1])];
				{
					let t35: [uint32x4_t; 4] = [x6[0], x6[1], vdupq_n_u32(0), vdupq_n_u32(0)];
					res = [vbslq_u32(t34[0], t35[0], res[0]), vbslq_u32(t34[1], t35[1], res[1]), vbslq_u32(t34[0], t35[2], res[2]), vbslq_u32(t34[1], t35[3], res[3])];
					live = [vbicq_u32(live[0], t34[0]), vbicq_u32(live[1], t34[1])];
				}
				if any(&live) {
					let t36: [uint32x4_t; 2] = [vceqq_u32(x6[0], t1[0]), vceqq_u32(x6[1], t1[1])];
					let t37: [uint32x4_t; 2] = [vandq_u32(live[0], t36[0]), vandq_u32(live[1], t36[1])];
					{
						let t38: u64 = 4294967295u64;
						let t39 = [vdupq_n_u32((t38) as u32); 2];
						let t40: [uint32x4_t; 4] = [t39[0], t39[1], vdupq_n_u32(0), vdupq_n_u32(0)];
						res = [vbslq_u32(t37[0], t40[0], res[0]), vbslq_u32(t37[1], t40[1], res[1]), vbslq_u32(t37[0], t40[2], res[2]), vbslq_u32(t37[1], t40[3], res[3])];
						live = [vbicq_u32(live[0], t37[0]), vbicq_u32(live[1], t37[1])];
					}
					if any(&live) {
						let t41: u64 = (1u64).wrapping_shl((32u64) as u32);
						let t42 = [vdupq_n_u32((t41) as u32); 2];
						let t43 = [vdupq_n_u32(((t41) >> 32) as u32); 2];
						let t44: [uint32x4_t; 4] = [vorrq_u32(t42[0], x6[0]), vorrq_u32(t42[1], x6[1]), t43[0], t43[1]];
						res = [vbslq_u32(live[0], t44[0], res[0]), vbslq_u32(live[1], t44[1], res[1]), vbslq_u32(live[0], t44[2], res[2]), vbslq_u32(live[1], t44[3], res[3])];
						live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
					}
				}
			}
		}
	}
	res
}

/// `clif_f16_parent_step`.
#[inline(always)]
unsafe fn b1_r0r1g_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8) -> [uint32x4_t; 4] {
	let gid0 = l.group[0] * l.block[0] + lid0;
	let mut x3: u32 = 0;
	let mut x4 = [vdupq_n_u32(0); 4];
	let mut x5 = [vdupq_n_u32(0); 2];
	let mut x6 = [vdupq_n_u32(0); 2];
	let mut live = [vdupq_n_u32(u32::MAX); 2];
	let mut res = [vdupq_n_u32(0); 4];
	if (((gid0) as u64 + 7) <= u32::MAX as u64 && (((0u32) as u64) < ((gid0) as u64) || ((0u32) as u64) > ((gid0) as u64 + 7))) {
		x3 = gid0.wrapping_sub(1u32);
		let t0 = b2_r0r1i_f(l, lid0, arg0, arg1, x3);
		x4 = t0;
		let t1: u64 = 1u64;
		let t2 = [vdupq_n_u32((t1) as u32); 2];
		let t3: [uint32x4_t; 2] = [vandq_u32(x4[0], t2[0]), vandq_u32(x4[1], t2[1])];
		let t4: u64 = 0u64;
		let t5 = [vdupq_n_u32((t4) as u32); 2];
		let t6: [uint32x4_t; 2] = [vceqq_u32(t3[0], t5[0]), vceqq_u32(t3[1], t5[1])];
		{
			let t7: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[1].as_ptr()))];
			let t8: [uint32x4_t; 4] = [t7[0], t7[1], vdupq_n_u32(0), vdupq_n_u32(0)];
			res = [vbslq_u32(t6[0], t8[0], res[0]), vbslq_u32(t6[1], t8[1], res[1]), vbslq_u32(t6[0], t8[2], res[2]), vbslq_u32(t6[1], t8[3], res[3])];
			live = [vbicq_u32(live[0], t6[0]), vbicq_u32(live[1], t6[1])];
		}
		if any(&live) {
			let t9: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x4[0]), vshlq_n_u32::<30>(x4[2])), vorrq_u32(vshrq_n_u32::<2>(x4[1]), vshlq_n_u32::<30>(x4[3])), vshrq_n_u32::<2>(x4[2]), vshrq_n_u32::<2>(x4[3])];
			let t10: [uint32x4_t; 2] = [t9[0], t9[1]];
			x5 = t10;
			let t11 = [vdupq_n_u32(1u32); 2];
			let t12: [uint32x4_t; 2] = [vsubq_u32(x5[0], t11[0]), vsubq_u32(x5[1], t11[1])];
			let t13: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
			let t14: [uint32x4_t; 2] = [vcgeq_u32(t12[0], t13[0]), vcgeq_u32(t12[1], t13[1])];
			let t15: [uint32x4_t; 2] = [vandq_u32(live[0], t14[0]), vandq_u32(live[1], t14[1])];
			{
				let t16: u64 = 4294967295u64;
				let t17 = [vdupq_n_u32((t16) as u32); 2];
				let t18: [uint32x4_t; 4] = [t17[0], t17[1], vdupq_n_u32(0), vdupq_n_u32(0)];
				res = [vbslq_u32(t15[0], t18[0], res[0]), vbslq_u32(t15[1], t18[1], res[1]), vbslq_u32(t15[0], t18[2], res[2]), vbslq_u32(t15[1], t18[3], res[3])];
				live = [vbicq_u32(live[0], t15[0]), vbicq_u32(live[1], t15[1])];
			}
			if any(&live) {
				let t19: [uint32x4_t; 2] = [vsubq_u32(t13[0], x5[0]), vsubq_u32(t13[1], x5[1])];
				x6 = t19;
				let t20: u64 = 2u64;
				let t21 = [vdupq_n_u32((t20) as u32); 2];
				let t22: [uint32x4_t; 2] = [vandq_u32(x4[0], t21[0]), vandq_u32(x4[1], t21[1])];
				let t23: u64 = 0u64;
				let t24 = [vdupq_n_u32((t23) as u32); 2];
				let t25: [uint32x4_t; 2] = [vceqq_u32(t22[0], t24[0]), vceqq_u32(t22[1], t24[1])];
				let t26: [uint32x4_t; 2] = [vmvnq_u32(t25[0]), vmvnq_u32(t25[1])];
				let t27: [uint32x4_t; 2] = [vandq_u32(live[0], t26[0]), vandq_u32(live[1], t26[1])];
				{
					let t28: [uint32x4_t; 4] = [x6[0], x6[1], vdupq_n_u32(0), vdupq_n_u32(0)];
					res = [vbslq_u32(t27[0], t28[0], res[0]), vbslq_u32(t27[1], t28[1], res[1]), vbslq_u32(t27[0], t28[2], res[2]), vbslq_u32(t27[1], t28[3], res[3])];
					live = [vbicq_u32(live[0], t27[0]), vbicq_u32(live[1], t27[1])];
				}
				if any(&live) {
					let t29 = [vdupq_n_u32(0u32); 2];
					let t30: [uint32x4_t; 2] = [vceqq_u32(x6[0], t29[0]), vceqq_u32(x6[1], t29[1])];
					let t31: [uint32x4_t; 2] = [vandq_u32(live[0], t30[0]), vandq_u32(live[1], t30[1])];
					{
						let t32: u64 = 4294967295u64;
						let t33 = [vdupq_n_u32((t32) as u32); 2];
						let t34: [uint32x4_t; 4] = [t33[0], t33[1], vdupq_n_u32(0), vdupq_n_u32(0)];
						res = [vbslq_u32(t31[0], t34[0], res[0]), vbslq_u32(t31[1], t34[1], res[1]), vbslq_u32(t31[0], t34[2], res[2]), vbslq_u32(t31[1], t34[3], res[3])];
						live = [vbicq_u32(live[0], t31[0]), vbicq_u32(live[1], t31[1])];
					}
					if any(&live) {
						let t35: u64 = (1u64).wrapping_shl((32u64) as u32);
						let t36 = [vdupq_n_u32((t35) as u32); 2];
						let t37 = [vdupq_n_u32(((t35) >> 32) as u32); 2];
						let t38: [uint32x4_t; 4] = [vorrq_u32(t36[0], x6[0]), vorrq_u32(t36[1], x6[1]), t37[0], t37[1]];
						res = [vbslq_u32(live[0], t38[0], res[0]), vbslq_u32(live[1], t38[1], res[1]), vbslq_u32(live[0], t38[2], res[2]), vbslq_u32(live[1], t38[3], res[3])];
						live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
					}
				}
			}
		}
	} else if false {
		let t39: u64 = 4294967295u64;
		let t40 = [vdupq_n_u32((t39) as u32); 2];
		let t41: [uint32x4_t; 4] = [t40[0], t40[1], vdupq_n_u32(0), vdupq_n_u32(0)];
		res = t41;
		return res;
	} else {
		// Lanes part: the chunk's lanes on both sides of the bound.
		let t42: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
		let t43 = [vdupq_n_u32(0u32); 2];
		let t44: [uint32x4_t; 2] = [vceqq_u32(t42[0], t43[0]), vceqq_u32(t42[1], t43[1])];
		let t45: [uint32x4_t; 2] = [vandq_u32(live[0], t44[0]), vandq_u32(live[1], t44[1])];
		{
			let t46: u64 = 4294967295u64;
			let t47 = [vdupq_n_u32((t46) as u32); 2];
			let t48: [uint32x4_t; 4] = [t47[0], t47[1], vdupq_n_u32(0), vdupq_n_u32(0)];
			res = [vbslq_u32(t45[0], t48[0], res[0]), vbslq_u32(t45[1], t48[1], res[1]), vbslq_u32(t45[0], t48[2], res[2]), vbslq_u32(t45[1], t48[3], res[3])];
			live = [vbicq_u32(live[0], t45[0]), vbicq_u32(live[1], t45[1])];
		}
		if any(&live) {
			x3 = gid0.wrapping_sub(1u32);
			let t49 = b2_r0r1i_p(l, lid0, live, arg0, arg1, x3);
			x4 = t49;
			let t50: u64 = 1u64;
			let t51 = [vdupq_n_u32((t50) as u32); 2];
			let t52: [uint32x4_t; 2] = [vandq_u32(x4[0], t51[0]), vandq_u32(x4[1], t51[1])];
			let t53: u64 = 0u64;
			let t54 = [vdupq_n_u32((t53) as u32); 2];
			let t55: [uint32x4_t; 2] = [vceqq_u32(t52[0], t54[0]), vceqq_u32(t52[1], t54[1])];
			let t56: [uint32x4_t; 2] = [vandq_u32(live[0], t55[0]), vandq_u32(live[1], t55[1])];
			{
				let t57: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[1].as_ptr()))];
				let t58: [uint32x4_t; 4] = [t57[0], t57[1], vdupq_n_u32(0), vdupq_n_u32(0)];
				res = [vbslq_u32(t56[0], t58[0], res[0]), vbslq_u32(t56[1], t58[1], res[1]), vbslq_u32(t56[0], t58[2], res[2]), vbslq_u32(t56[1], t58[3], res[3])];
				live = [vbicq_u32(live[0], t56[0]), vbicq_u32(live[1], t56[1])];
			}
			if any(&live) {
				let t59: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x4[0]), vshlq_n_u32::<30>(x4[2])), vorrq_u32(vshrq_n_u32::<2>(x4[1]), vshlq_n_u32::<30>(x4[3])), vshrq_n_u32::<2>(x4[2]), vshrq_n_u32::<2>(x4[3])];
				let t60: [uint32x4_t; 2] = [t59[0], t59[1]];
				x5 = t60;
				let t61 = [vdupq_n_u32(1u32); 2];
				let t62: [uint32x4_t; 2] = [vsubq_u32(x5[0], t61[0]), vsubq_u32(x5[1], t61[1])];
				let t63: [uint32x4_t; 2] = [vcgeq_u32(t62[0], t42[0]), vcgeq_u32(t62[1], t42[1])];
				let t64: [uint32x4_t; 2] = [vandq_u32(live[0], t63[0]), vandq_u32(live[1], t63[1])];
				{
					let t65: u64 = 4294967295u64;
					let t66 = [vdupq_n_u32((t65) as u32); 2];
					let t67: [uint32x4_t; 4] = [t66[0], t66[1], vdupq_n_u32(0), vdupq_n_u32(0)];
					res = [vbslq_u32(t64[0], t67[0], res[0]), vbslq_u32(t64[1], t67[1], res[1]), vbslq_u32(t64[0], t67[2], res[2]), vbslq_u32(t64[1], t67[3], res[3])];
					live = [vbicq_u32(live[0], t64[0]), vbicq_u32(live[1], t64[1])];
				}
				if any(&live) {
					let t68: [uint32x4_t; 2] = [vsubq_u32(t42[0], x5[0]), vsubq_u32(t42[1], x5[1])];
					x6 = t68;
					let t69: u64 = 2u64;
					let t70 = [vdupq_n_u32((t69) as u32); 2];
					let t71: [uint32x4_t; 2] = [vandq_u32(x4[0], t70[0]), vandq_u32(x4[1], t70[1])];
					let t72: u64 = 0u64;
					let t73 = [vdupq_n_u32((t72) as u32); 2];
					let t74: [uint32x4_t; 2] = [vceqq_u32(t71[0], t73[0]), vceqq_u32(t71[1], t73[1])];
					let t75: [uint32x4_t; 2] = [vmvnq_u32(t74[0]), vmvnq_u32(t74[1])];
					let t76: [uint32x4_t; 2] = [vandq_u32(live[0], t75[0]), vandq_u32(live[1], t75[1])];
					{
						let t77: [uint32x4_t; 4] = [x6[0], x6[1], vdupq_n_u32(0), vdupq_n_u32(0)];
						res = [vbslq_u32(t76[0], t77[0], res[0]), vbslq_u32(t76[1], t77[1], res[1]), vbslq_u32(t76[0], t77[2], res[2]), vbslq_u32(t76[1], t77[3], res[3])];
						live = [vbicq_u32(live[0], t76[0]), vbicq_u32(live[1], t76[1])];
					}
					if any(&live) {
						let t78: [uint32x4_t; 2] = [vceqq_u32(x6[0], t43[0]), vceqq_u32(x6[1], t43[1])];
						let t79: [uint32x4_t; 2] = [vandq_u32(live[0], t78[0]), vandq_u32(live[1], t78[1])];
						{
							let t80: u64 = 4294967295u64;
							let t81 = [vdupq_n_u32((t80) as u32); 2];
							let t82: [uint32x4_t; 4] = [t81[0], t81[1], vdupq_n_u32(0), vdupq_n_u32(0)];
							res = [vbslq_u32(t79[0], t82[0], res[0]), vbslq_u32(t79[1], t82[1], res[1]), vbslq_u32(t79[0], t82[2], res[2]), vbslq_u32(t79[1], t82[3], res[3])];
							live = [vbicq_u32(live[0], t79[0]), vbicq_u32(live[1], t79[1])];
						}
						if any(&live) {
							let t83: u64 = (1u64).wrapping_shl((32u64) as u32);
							let t84 = [vdupq_n_u32((t83) as u32); 2];
							let t85 = [vdupq_n_u32(((t83) >> 32) as u32); 2];
							let t86: [uint32x4_t; 4] = [vorrq_u32(t84[0], x6[0]), vorrq_u32(t84[1], x6[1]), t85[0], t85[1]];
							res = [vbslq_u32(live[0], t86[0], res[0]), vbslq_u32(live[1], t86[1], res[1]), vbslq_u32(live[0], t86[2], res[2]), vbslq_u32(live[1], t86[3], res[3])];
							live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
						}
					}
				}
			}
		}
	}
	res
}

/// `clif_f16_slot`.
#[inline(always)]
unsafe fn b2_r0r1i_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32) -> [uint32x4_t; 4] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 4];
	let t0: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x2), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x2), vld1q_u32(LANE[1].as_ptr()))];
	let t1 = b3_r0r1sl_p(l, lid0, live, arg0, arg1, 0u32, t0);
	res = t1;
	return res;
}

/// `clif_f16_slot`.
#[inline(always)]
unsafe fn b2_r0r1i_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32) -> [uint32x4_t; 4] {
	let mut live = [vdupq_n_u32(u32::MAX); 2];
	let mut res = [vdupq_n_u32(0); 4];
	let t0: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x2), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x2), vld1q_u32(LANE[1].as_ptr()))];
	let t1 = b3_r0r1sl_p(l, lid0, [vdupq_n_u32(u32::MAX); 2], arg0, arg1, 0u32, t0);
	res = t1;
	return res;
}

/// `clif_f16_get`.
#[inline(always)]
unsafe fn b3_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
	let mut x4: u32 = 0;
	let mut x5 = [vdupq_n_u32(0); 2];
	let mut x6: u64 = 0;
	let mut x7 = [vdupq_n_u32(0); 4];
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 4];
	let t0 = b4_r0r1s_p(l, lid0, live, arg0, arg1, x2);
	x4 = t0;
	let t1: [uint32x4_t; 2] = [vshlq_u32(x3[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x3[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
	let t2 = b5_r0r1sl_p(l, lid0, live, arg0, arg1, (1u32).wrapping_add(x2), t1);
	x5 = t2;
	let t3 = b6_s_p(l, lid0, live, x4);
	x6 = t3;
	let t4 = [vdupq_n_u32(1023u32); 2];
	let t5: [uint32x4_t; 2] = [vandq_u32(x3[0], t4[0]), vandq_u32(x3[1], t4[1])];
	let t6 = [vdupq_n_u32(x4); 2];
	let t7: [uint32x4_t; 2] = [vmulq_u32(t5[0], t6[0]), vmulq_u32(t5[1], t6[1])];
	let t8 = b7_r0r1lls_p(l, lid0, live, arg0, arg1, x5, t7, 8u32);
	let t9: u64 = x6;
	let t10 = [vdupq_n_u32((t9) as u32); 2];
	let t11 = [vdupq_n_u32(((t9) >> 32) as u32); 2];
	let t12: [uint32x4_t; 4] = [vandq_u32(t8[0], t10[0]), vandq_u32(t8[1], t10[1]), vandq_u32(t8[2], t11[0]), vandq_u32(t8[3], t11[1])];
	x7 = t12;
	let t13: u64 = x6;
	let t14 = [vdupq_n_u32((t13) as u32); 2];
	let t15 = [vdupq_n_u32(((t13) >> 32) as u32); 2];
	let t16: [uint32x4_t; 2] = [veq64q_u32(x7[0], x7[2], t14[0], t15[0]), veq64q_u32(x7[1], x7[3], t14[1], t15[1])];
	let mut t17 = [vdupq_n_u32(0); 4];
	let t18: [uint32x4_t; 2] = [vandq_u32(live[0], t16[0]), vandq_u32(live[1], t16[1])];
	let t19: [uint32x4_t; 2] = [vbicq_u32(live[0], t16[0]), vbicq_u32(live[1], t16[1])];
	if any(&t18) {
		let t20: [uint32x4_t; 4];
		let t21 = bits(&t18);
		if t21.count_ones() <= 2 {
			let t22 = words2(x3);
			let mut t23 = [0u32; 16];
			let mut t24 = t21;
			while t24 != 0 {
				let j = t24.trailing_zeros() as usize;
				t24 &= t24 - 1;
				let t25 = [vdupq_n_u32(t22[j]); 2];
				let t26 = b8_r0r1sl_p(l, lid0, [vdupq_n_u32(u32::MAX); 2], arg0, arg1, (x2).wrapping_add(1u32), t25);
				t23[j] = vgetq_lane_u32::<0>(t26[0]);
				t23[8 + j] = vgetq_lane_u32::<0>(t26[2]);
			}
			t20 = regs4(t23);
		} else {
			let t27 = b8_r0r1sl_p(l, lid0, t18, arg0, arg1, (x2).wrapping_add(1u32), x3);
			t20 = t27;
		}
		t17 = [vbslq_u32(t18[0], t20[0], t17[0]), vbslq_u32(t18[1], t20[1], t17[1]), vbslq_u32(t18[0], t20[2], t17[2]), vbslq_u32(t18[1], t20[3], t17[3])];
	}
	{
		t17 = [vbslq_u32(t19[0], x7[0], t17[0]), vbslq_u32(t19[1], x7[1], t17[1]), vbslq_u32(t19[0], x7[2], t17[2]), vbslq_u32(t19[1], x7[3], t17[3])];
	}
	res = t17;
	return res;
}

/// `clif_f16_width`.
#[inline(always)]
unsafe fn b4_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32) -> u32 {
	let mut live = k;
	let mut res: u32 = 0;
	let t0 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
	res = (((t0).wrapping_shr(((4u32).wrapping_mul(x2)) as u32)) & (15u32));
	return res;
}

/// `clif_f16_column`.
#[inline(always)]
unsafe fn b5_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
	let mut x4 = [vdupq_n_u32(0); 2];
	let mut x5 = [vdupq_n_u32(0); 2];
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 40u32);
	let t1 = [vdupq_n_u32(t0); 2];
	let t2: [uint32x4_t; 2] = [vmulq_u32(x3[0], t1[0]), vmulq_u32(x3[1], t1[1])];
	let t3 = b10_r0r1s_p(l, lid0, live, arg0, arg1, 0u32);
	let t4 = [vdupq_n_u32(t3); 2];
	let t5: [uint32x4_t; 2] = [vaddq_u32(t2[0], t4[0]), vaddq_u32(t2[1], t4[1])];
	x4 = t5;
	if ((x2) == (0u32)) {
		res = x4;
		return res;
	}
	let t6 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
	let t7: [uint32x4_t; 2] = [vshlq_u32(x3[0], vdupq_n_s32(((10u32) & 31) as i32)), vshlq_u32(x3[1], vdupq_n_s32(((10u32) & 31) as i32))];
	let t8 = [vdupq_n_u32(t6); 2];
	let t9: [uint32x4_t; 2] = [vsubq_u32(t8[0], t7[0]), vsubq_u32(t8[1], t7[1])];
	let t10 = b11_ls_p(l, lid0, live, t9, 1024u32);
	x5 = t10;
	let t11 = b10_r0r1s_p(l, lid0, live, arg0, arg1, x2);
	let t12 = [vdupq_n_u32(t11); 2];
	let t13: [uint32x4_t; 2] = [vmulq_u32(x5[0], t12[0]), vmulq_u32(x5[1], t12[1])];
	let t14: [uint32x4_t; 2] = [vaddq_u32(x4[0], t13[0]), vaddq_u32(x4[1], t13[1])];
	res = t14;
	return res;
}

/// `clif_t_mask`.
#[inline(always)]
unsafe fn b6_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
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
unsafe fn b7_r0r1lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: [uint32x4_t; 2], x3: [uint32x4_t; 2], x4: u32) -> [uint32x4_t; 4] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 4];
	let t0 = b12_r0lls_p(l, lid0, live, arg0, x2, x3, x4);
	res = t0;
	return res;
}

/// `clif_f16_side`.
#[inline(always)]
unsafe fn b8_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
	let mut x4: u32 = 0;
	let mut x5: u32 = 0;
	let mut x6 = [vdupq_n_u32(0); 2];
	let mut x7 = [vdupq_n_u32(0); 2];
	let mut x8 = [vdupq_n_u32(0); 2];
	let mut x9: u32 = 0;
	let mut x10 = [vdupq_n_u32(0); 2];
	let mut x11 = [vdupq_n_u32(0); 2];
	let mut x12 = [vdupq_n_u32(0); 2];
	let mut x13 = [vdupq_n_u32(0); 2];
	let mut x14 = [vdupq_n_u32(0); 2];
	let mut x15 = [vdupq_n_u32(0); 2];
	let mut x16: u64 = 0;
	let mut x17 = [vdupq_n_u32(0); 4];
	let mut x18 = [vdupq_n_u32(0); 2];
	let mut x19 = [vdupq_n_u32(0); 2];
	let mut x20 = [vdupq_n_u32(0); 4];
	let mut x21 = [vdupq_n_u32(0); 4];
	let mut x22 = [vdupq_n_u32(0); 4];
	let mut x23 = [vdupq_n_u32(0); 2];
	let mut x24: u32 = 0;
	let mut x25 = [vdupq_n_u32(0); 2];
	let mut x26 = [vdupq_n_u32(0); 2];
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 4];
	let t0 = b9_r0r1s_p(l, lid0, live, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x2)));
	x4 = t0;
	let t1 = b9_r0r1s_p(l, lid0, live, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x2)));
	x5 = t1;
	let t2: [uint32x4_t; 2] = [vshlq_u32(x3[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x3[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
	x6 = t2;
	let t3 = [vdupq_n_u32(4u32); 2];
	let t4: [uint32x4_t; 2] = [vmulq_u32(x6[0], t3[0]), vmulq_u32(x6[1], t3[1])];
	let t5 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x5, t4, 4u32);
	let t6: [uint32x4_t; 2] = [t5[0], t5[1]];
	x7 = t6;
	let t7: [uint32x4_t; 2] = [vmulq_u32(x6[0], t3[0]), vmulq_u32(x6[1], t3[1])];
	let t8: [uint32x4_t; 2] = [vaddq_u32(t7[0], t3[0]), vaddq_u32(t7[1], t3[1])];
	let t9 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x5, t8, 4u32);
	let t10: [uint32x4_t; 2] = [t9[0], t9[1]];
	x8 = t10;
	let t11 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
	x9 = ((t11).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
	let t12: [uint32x4_t; 2] = [vmulq_u32(x6[0], t3[0]), vmulq_u32(x6[1], t3[1])];
	let t13 = [vdupq_n_u32(x9); 2];
	let t14: [uint32x4_t; 2] = [vsubq_u32(t13[0], t12[0]), vsubq_u32(t13[1], t12[1])];
	let t15 = b14_sl_p(l, lid0, live, 4u32, t14);
	x10 = t15;
	let t16: [uint32x4_t; 2] = [vshlq_u32(x3[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(x3[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
	let t17 = [vdupq_n_u32(3u32); 2];
	let t18: [uint32x4_t; 2] = [vandq_u32(t16[0], t17[0]), vandq_u32(t16[1], t17[1])];
	x11 = t18;
	let t19 = [vdupq_n_u32(0u32); 2];
	let t20: [uint32x4_t; 2] = [vceqq_u32(x11[0], t19[0]), vceqq_u32(x11[1], t19[1])];
	let mut t21 = [vdupq_n_u32(0); 2];
	let t22: [uint32x4_t; 2] = [vandq_u32(live[0], t20[0]), vandq_u32(live[1], t20[1])];
	let t23: [uint32x4_t; 2] = [vbicq_u32(live[0], t20[0]), vbicq_u32(live[1], t20[1])];
	{
		t21 = [vbslq_u32(t22[0], t19[0], t21[0]), vbslq_u32(t22[1], t19[1], t21[1])];
	}
	if any(&t23) {
		let t24 = [vdupq_n_u32(1u32); 2];
		let t25: [uint32x4_t; 2] = [vsubq_u32(x11[0], t24[0]), vsubq_u32(x11[1], t24[1])];
		let t26 = [vdupq_n_u32(2u32); 2];
		let t27: [uint32x4_t; 2] = [vmulq_u32(t26[0], t25[0]), vmulq_u32(t26[1], t25[1])];
		let t28: [uint32x4_t; 2] = [vaddq_u32(x7[0], t27[0]), vaddq_u32(x7[1], t27[1])];
		let t29 = b15_ll_p(l, lid0, t23, t28, x8);
		let t30 = b13_r0r1sls_p(l, lid0, t23, arg0, arg1, x4, t29, 2u32);
		let t31: [uint32x4_t; 2] = [t30[0], t30[1]];
		t21 = [vbslq_u32(t23[0], t31[0], t21[0]), vbslq_u32(t23[1], t31[1], t21[1])];
	}
	x12 = t21;
	let t32 = [vdupq_n_u32(2u32); 2];
	let t33: [uint32x4_t; 2] = [vmulq_u32(t32[0], x11[0]), vmulq_u32(t32[1], x11[1])];
	let t34: [uint32x4_t; 2] = [vaddq_u32(x7[0], t33[0]), vaddq_u32(x7[1], t33[1])];
	let t35 = b15_ll_p(l, lid0, live, t34, x8);
	let t36 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x4, t35, 2u32);
	let t37: [uint32x4_t; 2] = [t36[0], t36[1]];
	x13 = t37;
	let t38: [uint32x4_t; 2] = [vmulq_u32(t32[0], x10[0]), vmulq_u32(t32[1], x10[1])];
	let t39: [uint32x4_t; 2] = [vaddq_u32(x7[0], t38[0]), vaddq_u32(x7[1], t38[1])];
	x14 = t39;
	let t40 = [vdupq_n_u32(255u32); 2];
	let t41: [uint32x4_t; 2] = [vandq_u32(x3[0], t40[0]), vandq_u32(x3[1], t40[1])];
	x15 = t41;
	x16 = 9259542123273814144u64;
	let t42: u64 = 72340172838076673u64;
	let t43 = [vdupq_n_u32((t42) as u32); 2];
	let t44 = [vdupq_n_u32(((t42) >> 32) as u32); 2];
	let t45: [uint32x4_t; 2] = [vmulhiq_u32(x15[0], t43[0]), vmulhiq_u32(x15[1], t43[1])];
	let t46: [uint32x4_t; 4] = [vmulq_u32(x15[0], t43[0]), vmulq_u32(x15[1], t43[1]), vmlaq_u32(t45[0], x15[0], t44[0]), vmlaq_u32(t45[1], x15[1], t44[1])];
	x17 = t46;
	x18 = x12;
	x19 = x12;
	let mut t47 = live;
	loop {
		// Lanes part: each its own rounds.
		let t48: [uint32x4_t; 2] = [vcltq_u32(x19[0], x13[0]), vcltq_u32(x19[1], x13[1])];
		t47 = [vandq_u32(t47[0], t48[0]), vandq_u32(t47[1], t48[1])];
		if !(any(&t47)) { break; }
		let t49: [uint32x4_t; 2] = [vaddq_u32(x14[0], x19[0]), vaddq_u32(x14[1], x19[1])];
		let t50 = b15_ll_p(l, lid0, t47, t49, x8);
		let t51 = b13_r0r1sls_p(l, lid0, t47, arg0, arg1, x4, t50, 8u32);
		x20 = t51;
		let t52: u64 = x16;
		let t53 = [vdupq_n_u32((t52) as u32); 2];
		let t54 = [vdupq_n_u32(((t52) >> 32) as u32); 2];
		let t55: [uint32x4_t; 4] = [vorrq_u32(x20[0], t53[0]), vorrq_u32(x20[1], t53[1]), vorrq_u32(x20[2], t54[0]), vorrq_u32(x20[3], t54[1])];
		let t56: u64 = !(x16);
		let t57 = [vdupq_n_u32((t56) as u32); 2];
		let t58 = [vdupq_n_u32(((t56) >> 32) as u32); 2];
		let t59: [uint32x4_t; 4] = [vandq_u32(x17[0], t57[0]), vandq_u32(x17[1], t57[1]), vandq_u32(x17[2], t58[0]), vandq_u32(x17[3], t58[1])];
		let t60: [uint32x4_t; 2] = [vcltq_u32(t55[0], t59[0]), vcltq_u32(t55[1], t59[1])];
		let t61: [uint32x4_t; 4] = [vsubq_u32(t55[0], t59[0]), vsubq_u32(t55[1], t59[1]), vsbcq_u32(t55[2], t59[2], t60[0]), vsbcq_u32(t55[3], t59[3], t60[1])];
		x21 = t61;
		let t62: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x20[r]));
		let t63: [uint32x4_t; 4] = [vandq_u32(t62[0], x17[0]), vandq_u32(t62[1], x17[1]), vandq_u32(t62[2], x17[2]), vandq_u32(t62[3], x17[3])];
		let t64: [uint32x4_t; 4] = [veorq_u32(x20[0], x17[0]), veorq_u32(x20[1], x17[1]), veorq_u32(x20[2], x17[2]), veorq_u32(x20[3], x17[3])];
		let t65: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(t64[r]));
		let t66: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x21[r]));
		let t67: [uint32x4_t; 4] = [vandq_u32(t65[0], t66[0]), vandq_u32(t65[1], t66[1]), vandq_u32(t65[2], t66[2]), vandq_u32(t65[3], t66[3])];
		let t68: [uint32x4_t; 4] = [vorrq_u32(t63[0], t67[0]), vorrq_u32(t63[1], t67[1]), vorrq_u32(t63[2], t67[2]), vorrq_u32(t63[3], t67[3])];
		let t69: u64 = x16;
		let t70 = [vdupq_n_u32((t69) as u32); 2];
		let t71 = [vdupq_n_u32(((t69) >> 32) as u32); 2];
		let t72: [uint32x4_t; 4] = [vandq_u32(t70[0], t68[0]), vandq_u32(t70[1], t68[1]), vandq_u32(t71[0], t68[2]), vandq_u32(t71[1], t68[3])];
		x22 = t72;
		let t73: [uint32x4_t; 2] = [vsubq_u32(x13[0], x19[0]), vsubq_u32(x13[1], x19[1])];
		x23 = t73;
		let t74 = [vdupq_n_u32(8u32); 2];
		let t75: [uint32x4_t; 2] = [vcgeq_u32(x23[0], t74[0]), vcgeq_u32(x23[1], t74[1])];
		let mut t76 = [vdupq_n_u32(0); 4];
		let t77: [uint32x4_t; 2] = [vandq_u32(t47[0], t75[0]), vandq_u32(t47[1], t75[1])];
		let t78: [uint32x4_t; 2] = [vbicq_u32(t47[0], t75[0]), vbicq_u32(t47[1], t75[1])];
		{
			t76 = [vbslq_u32(t77[0], x22[0], t76[0]), vbslq_u32(t77[1], x22[1], t76[1]), vbslq_u32(t77[0], x22[2], t76[2]), vbslq_u32(t77[1], x22[3], t76[3])];
		}
		{
			let t79: [uint32x4_t; 2] = [vmulq_u32(t74[0], x23[0]), vmulq_u32(t74[1], x23[1])];
			let t80: u64 = 1u64;
			let t81 = [vdupq_n_u32((t80) as u32); 2];
			let t82: [uint32x4_t; 2] = [t79[0], t79[1]];
			let t83: [uint32x4_t; 2] = [vandq_u32(t82[0], vdupq_n_u32(63)), vandq_u32(t82[1], vdupq_n_u32(63))];
			let t84: [uint32x4_t; 4] = [vshlq_u32(t81[0], vreinterpretq_s32_u32(t83[0])), vshlq_u32(t81[1], vreinterpretq_s32_u32(t83[1])), vshlq_u32(t81[0], vsubq_s32(vreinterpretq_s32_u32(t83[0]), vdupq_n_s32(32))), vshlq_u32(t81[1], vsubq_s32(vreinterpretq_s32_u32(t83[1]), vdupq_n_s32(32)))];
			let t85: u64 = 1u64;
			let t86 = [vdupq_n_u32((t85) as u32); 2];
			let t87: [uint32x4_t; 2] = [vcltq_u32(t84[0], t86[0]), vcltq_u32(t84[1], t86[1])];
			let t88: [uint32x4_t; 4] = [vsubq_u32(t84[0], t86[0]), vsubq_u32(t84[1], t86[1]), vsbcq_u32(t84[2], vdupq_n_u32(0), t87[0]), vsbcq_u32(t84[3], vdupq_n_u32(0), t87[1])];
			let t89: [uint32x4_t; 4] = [vandq_u32(x22[0], t88[0]), vandq_u32(x22[1], t88[1]), vandq_u32(x22[2], t88[2]), vandq_u32(x22[3], t88[3])];
			t76 = [vbslq_u32(t78[0], t89[0], t76[0]), vbslq_u32(t78[1], t89[1], t76[1]), vbslq_u32(t78[0], t89[2], t76[2]), vbslq_u32(t78[1], t89[3], t76[3])];
		}
		let t90 = b16_l_p(l, lid0, t47, t76);
		let t91: [uint32x4_t; 2] = [vaddq_u32(x18[0], t90[0]), vaddq_u32(x18[1], t90[1])];
		x18 = [vbslq_u32(t47[0], t91[0], x18[0]), vbslq_u32(t47[1], t91[1], x18[1])];
		let t92: [uint32x4_t; 2] = [vandq_u32(t47[0], live[0]), vandq_u32(t47[1], live[1])];
		let t93: [uint32x4_t; 2] = [vaddq_u32(x19[0], t74[0]), vaddq_u32(x19[1], t74[1])];
		x19 = t93;
	}
	x12 = x18;
	let t94 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
	x24 = (((t94).wrapping_shr((((((4u32).wrapping_mul(x2)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32));
	let t95 = [vdupq_n_u32(1u32); 2];
	let t96: [uint32x4_t; 2] = [vsubq_u32(x10[0], t95[0]), vsubq_u32(x10[1], t95[1])];
	let t97: [uint32x4_t; 2] = [vmulq_u32(t32[0], t96[0]), vmulq_u32(t32[1], t96[1])];
	let t98: [uint32x4_t; 2] = [vaddq_u32(x7[0], t97[0]), vaddq_u32(x7[1], t97[1])];
	let t99 = b15_ll_p(l, lid0, live, t98, x8);
	let t100 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x4, t99, 2u32);
	let t101: [uint32x4_t; 2] = [t100[0], t100[1]];
	x25 = t101;
	let t102: [uint32x4_t; 2] = [vaddq_u32(x14[0], x25[0]), vaddq_u32(x14[1], x25[1])];
	let t103 = [vdupq_n_u32(x24); 2];
	let t104: [uint32x4_t; 2] = [vmulq_u32(x12[0], t103[0]), vmulq_u32(x12[1], t103[1])];
	let t105: [uint32x4_t; 2] = [vaddq_u32(t102[0], t104[0]), vaddq_u32(t102[1], t104[1])];
	let t106 = b15_ll_p(l, lid0, live, t105, x8);
	x26 = t106;
	let t107 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x4, x26, 8u32);
	let t108 = b6_s_p(l, lid0, live, x24);
	let t109: u64 = t108;
	let t110 = [vdupq_n_u32((t109) as u32); 2];
	let t111 = [vdupq_n_u32(((t109) >> 32) as u32); 2];
	let t112: [uint32x4_t; 4] = [vandq_u32(t107[0], t110[0]), vandq_u32(t107[1], t110[1]), vandq_u32(t107[2], t111[0]), vandq_u32(t107[3], t111[1])];
	res = t112;
	return res;
}

/// `clif_f16_view`.
#[inline(always)]
unsafe fn b9_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32) -> u32 {
	let mut live = k;
	let mut res: u32 = 0;
	res = (arg1.wrapping_add(((((x2).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned();
	return res;
}

/// `clif_f16_columns`.
#[inline(always)]
unsafe fn b10_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32) -> u32 {
	let mut live = k;
	let mut res: u32 = 0;
	let t0 = b9_r0r1s_p(l, lid0, live, arg0, arg1, (20u32).wrapping_add((4u32).wrapping_mul(x2)));
	res = t0;
	return res;
}

/// `ukl_min`.
#[inline(always)]
unsafe fn b11_ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x1: u32) -> [uint32x4_t; 2] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = [vdupq_n_u32(x1); 2];
	let t1: [uint32x4_t; 2] = [vcltq_u32(x0[0], t0[0]), vcltq_u32(x0[1], t0[1])];
	let mut t2 = [vdupq_n_u32(0); 2];
	let t3: [uint32x4_t; 2] = [vandq_u32(live[0], t1[0]), vandq_u32(live[1], t1[1])];
	let t4: [uint32x4_t; 2] = [vbicq_u32(live[0], t1[0]), vbicq_u32(live[1], t1[1])];
	{
		t2 = [vbslq_u32(t3[0], x0[0], t2[0]), vbslq_u32(t3[1], x0[1], t2[1])];
	}
	{
		let t5 = [vdupq_n_u32(x1); 2];
		t2 = [vbslq_u32(t4[0], t5[0], t2[0]), vbslq_u32(t4[1], t5[1], t2[1])];
	}
	res = t2;
	return res;
}

/// `clif_t_ld`.
#[inline(always)]
unsafe fn b12_r0lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1: [uint32x4_t; 2], x2: [uint32x4_t; 2], x3: u32) -> [uint32x4_t; 4] {
	let mut x4 = [vdupq_n_u32(0); 4];
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 4];
	let t0: [uint32x4_t; 2] = [vaddq_u32(x1[0], x2[0]), vaddq_u32(x1[1], x2[1])];
	let t1: [uint32x4_t; 2] = [vcltq_u32(t0[0], x1[0]), vcltq_u32(t0[1], x1[1])];
	let t2: [uint32x4_t; 4] = [t0[0], t0[1], vshrq_n_u32::<31>(t1[0]), vshrq_n_u32::<31>(t1[1])];
	let t3 = b17_r0ls_p(l, lid0, live, arg0, t2, x3);
	x4 = t3;
	let t4: [uint32x4_t; 4];
	if ((x3) >= (8u32)) {
		t4 = x4;
	} else {
		let t5 = b18_s_p(l, lid0, live, (8u32).wrapping_mul(x3));
		let t6: u64 = t5;
		let t7 = [vdupq_n_u32((t6) as u32); 2];
		let t8 = [vdupq_n_u32(((t6) >> 32) as u32); 2];
		let t9: [uint32x4_t; 4] = [vandq_u32(x4[0], t7[0]), vandq_u32(x4[1], t7[1]), vandq_u32(x4[2], t8[0]), vandq_u32(x4[3], t8[1])];
		t4 = t9;
	}
	res = t4;
	return res;
}

/// `clif_f16_ld`.
#[inline(always)]
unsafe fn b13_r0r1sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: [uint32x4_t; 2], x4: u32) -> [uint32x4_t; 4] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 4];
	let t0 = b19_r0sls_p(l, lid0, live, arg0, x2, x3, x4);
	res = t0;
	return res;
}

/// `ukl_min`.
#[inline(always)]
unsafe fn b14_sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x1: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = [vdupq_n_u32(x0); 2];
	let t1: [uint32x4_t; 2] = [vcltq_u32(t0[0], x1[0]), vcltq_u32(t0[1], x1[1])];
	let mut t2 = [vdupq_n_u32(0); 2];
	let t3: [uint32x4_t; 2] = [vandq_u32(live[0], t1[0]), vandq_u32(live[1], t1[1])];
	let t4: [uint32x4_t; 2] = [vbicq_u32(live[0], t1[0]), vbicq_u32(live[1], t1[1])];
	{
		let t5 = [vdupq_n_u32(x0); 2];
		t2 = [vbslq_u32(t3[0], t5[0], t2[0]), vbslq_u32(t3[1], t5[1], t2[1])];
	}
	{
		t2 = [vbslq_u32(t4[0], x1[0], t2[0]), vbslq_u32(t4[1], x1[1], t2[1])];
	}
	res = t2;
	return res;
}

/// `ukl_min`.
#[inline(always)]
unsafe fn b15_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x1: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0: [uint32x4_t; 2] = [vcltq_u32(x0[0], x1[0]), vcltq_u32(x0[1], x1[1])];
	let mut t1 = [vdupq_n_u32(0); 2];
	let t2: [uint32x4_t; 2] = [vandq_u32(live[0], t0[0]), vandq_u32(live[1], t0[1])];
	let t3: [uint32x4_t; 2] = [vbicq_u32(live[0], t0[0]), vbicq_u32(live[1], t0[1])];
	{
		t1 = [vbslq_u32(t2[0], x0[0], t1[0]), vbslq_u32(t2[1], x0[1], t1[1])];
	}
	{
		t1 = [vbslq_u32(t3[0], x1[0], t1[0]), vbslq_u32(t3[1], x1[1], t1[1])];
	}
	res = t1;
	return res;
}

/// `ukl_popcount64`.
#[inline(always)]
unsafe fn b16_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4]) -> [uint32x4_t; 2] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0: [uint32x4_t; 2] = [vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[0])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[2]))))), vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[1])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[3])))))];
	res = t0;
	return res;
}

/// `ukl_ld_window`.
#[inline(always)]
unsafe fn b17_r0ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1: [uint32x4_t; 4], x2: u32) -> [uint32x4_t; 4] {
	let mut x3 = [vdupq_n_u32(0); 4];
	let mut x4 = [vdupq_n_u32(0); 2];
	let mut x5 = [vdupq_n_u32(0); 4];
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 4];
	if ((x2) == (8u32)) {
		let t0: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
		let t1: [uint32x4_t; 4] = [vandq_u32(x1[0], t0[0]), vandq_u32(x1[1], t0[1]), vandq_u32(x1[2], t0[2]), vandq_u32(x1[3], t0[3])];
		let t2: u64 = ((arg0) as usize as u64);
		let t3 = [vdupq_n_u32((t2) as u32); 2];
		let t4 = [vdupq_n_u32(((t2) >> 32) as u32); 2];
		let t5: [uint32x4_t; 2] = [vaddq_u32(t3[0], t1[0]), vaddq_u32(t3[1], t1[1])];
		let t6: [uint32x4_t; 2] = [vcltq_u32(t5[0], t3[0]), vcltq_u32(t5[1], t3[1])];
		let t7: [uint32x4_t; 4] = [t5[0], t5[1], vadcq_u32(t4[0], t1[2], t6[0]), vadcq_u32(t4[1], t1[3], t6[1])];
		let t8 = words2([t7[0], t7[1]]);
		let t9 = words2([t7[2], t7[3]]);
		let t10 = bits(&live);
		let t11 = t10.trailing_zeros() as usize % 8;
		let t12: [usize; 8] = [{ let j = if t10 >> 0 & 1 != 0 { 0 } else { t11 }; ((t9[j] as u64) << 32 | t8[j] as u64) as usize }, { let j = if t10 >> 1 & 1 != 0 { 1 } else { t11 }; ((t9[j] as u64) << 32 | t8[j] as u64) as usize }, { let j = if t10 >> 2 & 1 != 0 { 2 } else { t11 }; ((t9[j] as u64) << 32 | t8[j] as u64) as usize }, { let j = if t10 >> 3 & 1 != 0 { 3 } else { t11 }; ((t9[j] as u64) << 32 | t8[j] as u64) as usize }, { let j = if t10 >> 4 & 1 != 0 { 4 } else { t11 }; ((t9[j] as u64) << 32 | t8[j] as u64) as usize }, { let j = if t10 >> 5 & 1 != 0 { 5 } else { t11 }; ((t9[j] as u64) << 32 | t8[j] as u64) as usize }, { let j = if t10 >> 6 & 1 != 0 { 6 } else { t11 }; ((t9[j] as u64) << 32 | t8[j] as u64) as usize }, { let j = if t10 >> 7 & 1 != 0 { 7 } else { t11 }; ((t9[j] as u64) << 32 | t8[j] as u64) as usize }];
		let mut t13 = [vdupq_n_u32(0); 4];
		{ let v = (t12[0] as *const u64).read_unaligned(); t13[0] = vsetq_lane_u32::<0>(v as u32, t13[0]); t13[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t13[2]); }
		{ let v = (t12[1] as *const u64).read_unaligned(); t13[0] = vsetq_lane_u32::<1>(v as u32, t13[0]); t13[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t13[2]); }
		{ let v = (t12[2] as *const u64).read_unaligned(); t13[0] = vsetq_lane_u32::<2>(v as u32, t13[0]); t13[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t13[2]); }
		{ let v = (t12[3] as *const u64).read_unaligned(); t13[0] = vsetq_lane_u32::<3>(v as u32, t13[0]); t13[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t13[2]); }
		{ let v = (t12[4] as *const u64).read_unaligned(); t13[1] = vsetq_lane_u32::<0>(v as u32, t13[1]); t13[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t13[3]); }
		{ let v = (t12[5] as *const u64).read_unaligned(); t13[1] = vsetq_lane_u32::<1>(v as u32, t13[1]); t13[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t13[3]); }
		{ let v = (t12[6] as *const u64).read_unaligned(); t13[1] = vsetq_lane_u32::<2>(v as u32, t13[1]); t13[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t13[3]); }
		{ let v = (t12[7] as *const u64).read_unaligned(); t13[1] = vsetq_lane_u32::<3>(v as u32, t13[1]); t13[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t13[3]); }
		res = t13;
		return res;
	}
	if ((x2) == (4u32)) {
		let t14: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
		let t15: [uint32x4_t; 4] = [vandq_u32(x1[0], t14[0]), vandq_u32(x1[1], t14[1]), vandq_u32(x1[2], t14[2]), vandq_u32(x1[3], t14[3])];
		let t16: u64 = ((arg0) as usize as u64);
		let t17 = [vdupq_n_u32((t16) as u32); 2];
		let t18 = [vdupq_n_u32(((t16) >> 32) as u32); 2];
		let t19: [uint32x4_t; 2] = [vaddq_u32(t17[0], t15[0]), vaddq_u32(t17[1], t15[1])];
		let t20: [uint32x4_t; 2] = [vcltq_u32(t19[0], t17[0]), vcltq_u32(t19[1], t17[1])];
		let t21: [uint32x4_t; 4] = [t19[0], t19[1], vadcq_u32(t18[0], t15[2], t20[0]), vadcq_u32(t18[1], t15[3], t20[1])];
		let t22 = words2([t21[0], t21[1]]);
		let t23 = words2([t21[2], t21[3]]);
		let t24 = bits(&live);
		let t25 = t24.trailing_zeros() as usize % 8;
		let t26: [usize; 8] = [{ let j = if t24 >> 0 & 1 != 0 { 0 } else { t25 }; ((t23[j] as u64) << 32 | t22[j] as u64) as usize }, { let j = if t24 >> 1 & 1 != 0 { 1 } else { t25 }; ((t23[j] as u64) << 32 | t22[j] as u64) as usize }, { let j = if t24 >> 2 & 1 != 0 { 2 } else { t25 }; ((t23[j] as u64) << 32 | t22[j] as u64) as usize }, { let j = if t24 >> 3 & 1 != 0 { 3 } else { t25 }; ((t23[j] as u64) << 32 | t22[j] as u64) as usize }, { let j = if t24 >> 4 & 1 != 0 { 4 } else { t25 }; ((t23[j] as u64) << 32 | t22[j] as u64) as usize }, { let j = if t24 >> 5 & 1 != 0 { 5 } else { t25 }; ((t23[j] as u64) << 32 | t22[j] as u64) as usize }, { let j = if t24 >> 6 & 1 != 0 { 6 } else { t25 }; ((t23[j] as u64) << 32 | t22[j] as u64) as usize }, { let j = if t24 >> 7 & 1 != 0 { 7 } else { t25 }; ((t23[j] as u64) << 32 | t22[j] as u64) as usize }];
		let mut t27 = [vdupq_n_u32(0); 2];
		t27[0] = vsetq_lane_u32::<0>((t26[0] as *const u32).read_unaligned() as u32, t27[0]);
		t27[0] = vsetq_lane_u32::<1>((t26[1] as *const u32).read_unaligned() as u32, t27[0]);
		t27[0] = vsetq_lane_u32::<2>((t26[2] as *const u32).read_unaligned() as u32, t27[0]);
		t27[0] = vsetq_lane_u32::<3>((t26[3] as *const u32).read_unaligned() as u32, t27[0]);
		t27[1] = vsetq_lane_u32::<0>((t26[4] as *const u32).read_unaligned() as u32, t27[1]);
		t27[1] = vsetq_lane_u32::<1>((t26[5] as *const u32).read_unaligned() as u32, t27[1]);
		t27[1] = vsetq_lane_u32::<2>((t26[6] as *const u32).read_unaligned() as u32, t27[1]);
		t27[1] = vsetq_lane_u32::<3>((t26[7] as *const u32).read_unaligned() as u32, t27[1]);
		let t28: [uint32x4_t; 4] = [t27[0], t27[1], vdupq_n_u32(0), vdupq_n_u32(0)];
		res = t28;
		return res;
	}
	if ((x2) == (2u32)) {
		let t29: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
		let t30: [uint32x4_t; 4] = [vandq_u32(x1[0], t29[0]), vandq_u32(x1[1], t29[1]), vandq_u32(x1[2], t29[2]), vandq_u32(x1[3], t29[3])];
		let t31: u64 = ((arg0) as usize as u64);
		let t32 = [vdupq_n_u32((t31) as u32); 2];
		let t33 = [vdupq_n_u32(((t31) >> 32) as u32); 2];
		let t34: [uint32x4_t; 2] = [vaddq_u32(t32[0], t30[0]), vaddq_u32(t32[1], t30[1])];
		let t35: [uint32x4_t; 2] = [vcltq_u32(t34[0], t32[0]), vcltq_u32(t34[1], t32[1])];
		let t36: [uint32x4_t; 4] = [t34[0], t34[1], vadcq_u32(t33[0], t30[2], t35[0]), vadcq_u32(t33[1], t30[3], t35[1])];
		let t37 = words2([t36[0], t36[1]]);
		let t38 = words2([t36[2], t36[3]]);
		let t39 = bits(&live);
		let t40 = t39.trailing_zeros() as usize % 8;
		let t41: [usize; 8] = [{ let j = if t39 >> 0 & 1 != 0 { 0 } else { t40 }; ((t38[j] as u64) << 32 | t37[j] as u64) as usize }, { let j = if t39 >> 1 & 1 != 0 { 1 } else { t40 }; ((t38[j] as u64) << 32 | t37[j] as u64) as usize }, { let j = if t39 >> 2 & 1 != 0 { 2 } else { t40 }; ((t38[j] as u64) << 32 | t37[j] as u64) as usize }, { let j = if t39 >> 3 & 1 != 0 { 3 } else { t40 }; ((t38[j] as u64) << 32 | t37[j] as u64) as usize }, { let j = if t39 >> 4 & 1 != 0 { 4 } else { t40 }; ((t38[j] as u64) << 32 | t37[j] as u64) as usize }, { let j = if t39 >> 5 & 1 != 0 { 5 } else { t40 }; ((t38[j] as u64) << 32 | t37[j] as u64) as usize }, { let j = if t39 >> 6 & 1 != 0 { 6 } else { t40 }; ((t38[j] as u64) << 32 | t37[j] as u64) as usize }, { let j = if t39 >> 7 & 1 != 0 { 7 } else { t40 }; ((t38[j] as u64) << 32 | t37[j] as u64) as usize }];
		let mut t42 = [vdupq_n_u32(0); 2];
		t42[0] = vsetq_lane_u32::<0>((t41[0] as *const u16).read_unaligned() as u32, t42[0]);
		t42[0] = vsetq_lane_u32::<1>((t41[1] as *const u16).read_unaligned() as u32, t42[0]);
		t42[0] = vsetq_lane_u32::<2>((t41[2] as *const u16).read_unaligned() as u32, t42[0]);
		t42[0] = vsetq_lane_u32::<3>((t41[3] as *const u16).read_unaligned() as u32, t42[0]);
		t42[1] = vsetq_lane_u32::<0>((t41[4] as *const u16).read_unaligned() as u32, t42[1]);
		t42[1] = vsetq_lane_u32::<1>((t41[5] as *const u16).read_unaligned() as u32, t42[1]);
		t42[1] = vsetq_lane_u32::<2>((t41[6] as *const u16).read_unaligned() as u32, t42[1]);
		t42[1] = vsetq_lane_u32::<3>((t41[7] as *const u16).read_unaligned() as u32, t42[1]);
		let t43: [uint32x4_t; 4] = [t42[0], t42[1], vdupq_n_u32(0), vdupq_n_u32(0)];
		res = t43;
		return res;
	}
	if ((x2) == (1u32)) {
		let t44: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
		let t45: [uint32x4_t; 4] = [vandq_u32(x1[0], t44[0]), vandq_u32(x1[1], t44[1]), vandq_u32(x1[2], t44[2]), vandq_u32(x1[3], t44[3])];
		let t46: u64 = ((arg0) as usize as u64);
		let t47 = [vdupq_n_u32((t46) as u32); 2];
		let t48 = [vdupq_n_u32(((t46) >> 32) as u32); 2];
		let t49: [uint32x4_t; 2] = [vaddq_u32(t47[0], t45[0]), vaddq_u32(t47[1], t45[1])];
		let t50: [uint32x4_t; 2] = [vcltq_u32(t49[0], t47[0]), vcltq_u32(t49[1], t47[1])];
		let t51: [uint32x4_t; 4] = [t49[0], t49[1], vadcq_u32(t48[0], t45[2], t50[0]), vadcq_u32(t48[1], t45[3], t50[1])];
		let t52 = words2([t51[0], t51[1]]);
		let t53 = words2([t51[2], t51[3]]);
		let t54 = bits(&live);
		let t55 = t54.trailing_zeros() as usize % 8;
		let t56: [usize; 8] = [{ let j = if t54 >> 0 & 1 != 0 { 0 } else { t55 }; ((t53[j] as u64) << 32 | t52[j] as u64) as usize }, { let j = if t54 >> 1 & 1 != 0 { 1 } else { t55 }; ((t53[j] as u64) << 32 | t52[j] as u64) as usize }, { let j = if t54 >> 2 & 1 != 0 { 2 } else { t55 }; ((t53[j] as u64) << 32 | t52[j] as u64) as usize }, { let j = if t54 >> 3 & 1 != 0 { 3 } else { t55 }; ((t53[j] as u64) << 32 | t52[j] as u64) as usize }, { let j = if t54 >> 4 & 1 != 0 { 4 } else { t55 }; ((t53[j] as u64) << 32 | t52[j] as u64) as usize }, { let j = if t54 >> 5 & 1 != 0 { 5 } else { t55 }; ((t53[j] as u64) << 32 | t52[j] as u64) as usize }, { let j = if t54 >> 6 & 1 != 0 { 6 } else { t55 }; ((t53[j] as u64) << 32 | t52[j] as u64) as usize }, { let j = if t54 >> 7 & 1 != 0 { 7 } else { t55 }; ((t53[j] as u64) << 32 | t52[j] as u64) as usize }];
		let mut t57 = [vdupq_n_u32(0); 2];
		t57[0] = vsetq_lane_u32::<0>((t56[0] as *const u8).read_unaligned() as u32, t57[0]);
		t57[0] = vsetq_lane_u32::<1>((t56[1] as *const u8).read_unaligned() as u32, t57[0]);
		t57[0] = vsetq_lane_u32::<2>((t56[2] as *const u8).read_unaligned() as u32, t57[0]);
		t57[0] = vsetq_lane_u32::<3>((t56[3] as *const u8).read_unaligned() as u32, t57[0]);
		t57[1] = vsetq_lane_u32::<0>((t56[4] as *const u8).read_unaligned() as u32, t57[1]);
		t57[1] = vsetq_lane_u32::<1>((t56[5] as *const u8).read_unaligned() as u32, t57[1]);
		t57[1] = vsetq_lane_u32::<2>((t56[6] as *const u8).read_unaligned() as u32, t57[1]);
		t57[1] = vsetq_lane_u32::<3>((t56[7] as *const u8).read_unaligned() as u32, t57[1]);
		let t58: [uint32x4_t; 4] = [t57[0], t57[1], vdupq_n_u32(0), vdupq_n_u32(0)];
		res = t58;
		return res;
	}
	let t59: u64 = !(7u64);
	let t60 = [vdupq_n_u32((t59) as u32); 2];
	let t61 = [vdupq_n_u32(((t59) >> 32) as u32); 2];
	let t62: [uint32x4_t; 4] = [vandq_u32(x1[0], t60[0]), vandq_u32(x1[1], t60[1]), vandq_u32(x1[2], t61[0]), vandq_u32(x1[3], t61[1])];
	let t63: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
	let t64: [uint32x4_t; 4] = [vandq_u32(t62[0], t63[0]), vandq_u32(t62[1], t63[1]), vandq_u32(t62[2], t63[2]), vandq_u32(t62[3], t63[3])];
	let t65: u64 = ((arg0) as usize as u64);
	let t66 = [vdupq_n_u32((t65) as u32); 2];
	let t67 = [vdupq_n_u32(((t65) >> 32) as u32); 2];
	let t68: [uint32x4_t; 2] = [vaddq_u32(t66[0], t64[0]), vaddq_u32(t66[1], t64[1])];
	let t69: [uint32x4_t; 2] = [vcltq_u32(t68[0], t66[0]), vcltq_u32(t68[1], t66[1])];
	let t70: [uint32x4_t; 4] = [t68[0], t68[1], vadcq_u32(t67[0], t64[2], t69[0]), vadcq_u32(t67[1], t64[3], t69[1])];
	x3 = t70;
	let t71 = words2([x3[0], x3[1]]);
	let t72 = words2([x3[2], x3[3]]);
	let t73 = bits(&live);
	let t74 = t73.trailing_zeros() as usize % 8;
	let t75: [usize; 8] = [{ let j = if t73 >> 0 & 1 != 0 { 0 } else { t74 }; ((t72[j] as u64) << 32 | t71[j] as u64) as usize }, { let j = if t73 >> 1 & 1 != 0 { 1 } else { t74 }; ((t72[j] as u64) << 32 | t71[j] as u64) as usize }, { let j = if t73 >> 2 & 1 != 0 { 2 } else { t74 }; ((t72[j] as u64) << 32 | t71[j] as u64) as usize }, { let j = if t73 >> 3 & 1 != 0 { 3 } else { t74 }; ((t72[j] as u64) << 32 | t71[j] as u64) as usize }, { let j = if t73 >> 4 & 1 != 0 { 4 } else { t74 }; ((t72[j] as u64) << 32 | t71[j] as u64) as usize }, { let j = if t73 >> 5 & 1 != 0 { 5 } else { t74 }; ((t72[j] as u64) << 32 | t71[j] as u64) as usize }, { let j = if t73 >> 6 & 1 != 0 { 6 } else { t74 }; ((t72[j] as u64) << 32 | t71[j] as u64) as usize }, { let j = if t73 >> 7 & 1 != 0 { 7 } else { t74 }; ((t72[j] as u64) << 32 | t71[j] as u64) as usize }];
	let t76: u64 = 7u64;
	let t77 = [vdupq_n_u32((t76) as u32); 2];
	let t78: [uint32x4_t; 2] = [vandq_u32(x1[0], t77[0]), vandq_u32(x1[1], t77[1])];
	let t79 = [vdupq_n_u32(8u32); 2];
	let t80: [uint32x4_t; 2] = [vmulq_u32(t78[0], t79[0]), vmulq_u32(t78[1], t79[1])];
	x4 = t80;
	let mut t81 = [vdupq_n_u32(0); 4];
	{ let v = (t75[0] as *const u64).read_unaligned(); t81[0] = vsetq_lane_u32::<0>(v as u32, t81[0]); t81[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t81[2]); }
	{ let v = (t75[1] as *const u64).read_unaligned(); t81[0] = vsetq_lane_u32::<1>(v as u32, t81[0]); t81[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t81[2]); }
	{ let v = (t75[2] as *const u64).read_unaligned(); t81[0] = vsetq_lane_u32::<2>(v as u32, t81[0]); t81[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t81[2]); }
	{ let v = (t75[3] as *const u64).read_unaligned(); t81[0] = vsetq_lane_u32::<3>(v as u32, t81[0]); t81[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t81[2]); }
	{ let v = (t75[4] as *const u64).read_unaligned(); t81[1] = vsetq_lane_u32::<0>(v as u32, t81[1]); t81[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t81[3]); }
	{ let v = (t75[5] as *const u64).read_unaligned(); t81[1] = vsetq_lane_u32::<1>(v as u32, t81[1]); t81[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t81[3]); }
	{ let v = (t75[6] as *const u64).read_unaligned(); t81[1] = vsetq_lane_u32::<2>(v as u32, t81[1]); t81[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t81[3]); }
	{ let v = (t75[7] as *const u64).read_unaligned(); t81[1] = vsetq_lane_u32::<3>(v as u32, t81[1]); t81[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t81[3]); }
	let t82: [uint32x4_t; 2] = [x4[0], x4[1]];
	let t83: [uint32x4_t; 2] = [vandq_u32(t82[0], vdupq_n_u32(63)), vandq_u32(t82[1], vdupq_n_u32(63))];
	let t84: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t81[0], vnegq_s32(vreinterpretq_s32_u32(t83[0]))), vshlq_u32(t81[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t83[0])))), vorrq_u32(vshlq_u32(t81[1], vnegq_s32(vreinterpretq_s32_u32(t83[1]))), vshlq_u32(t81[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t83[1])))), vshlq_u32(t81[2], vnegq_s32(vreinterpretq_s32_u32(t83[0]))), vshlq_u32(t81[3], vnegq_s32(vreinterpretq_s32_u32(t83[1])))];
	x5 = t84;
	let t85 = [vdupq_n_u32(0u32); 2];
	let t86: [uint32x4_t; 2] = [vcneq_u32(x4[0], t85[0]), vcneq_u32(x4[1], t85[1])];
	let t87 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
	let t88: [uint32x4_t; 2] = [vaddq_u32(x4[0], t87[0]), vaddq_u32(x4[1], t87[1])];
	let t89 = [vdupq_n_u32(64u32); 2];
	let t90: [uint32x4_t; 2] = [vcgtq_u32(t88[0], t89[0]), vcgtq_u32(t88[1], t89[1])];
	let t91: [uint32x4_t; 2] = [vandq_u32(t86[0], t90[0]), vandq_u32(t86[1], t90[1])];
	let mut t92 = [vdupq_n_u32(0); 4];
	let t93: [uint32x4_t; 2] = [vandq_u32(live[0], t91[0]), vandq_u32(live[1], t91[1])];
	let t94: [uint32x4_t; 2] = [vbicq_u32(live[0], t91[0]), vbicq_u32(live[1], t91[1])];
	{
		let t95: u64 = (1u64).wrapping_mul(8u64);
		let t96 = [vdupq_n_u32((t95) as u32); 2];
		let t97 = [vdupq_n_u32(((t95) >> 32) as u32); 2];
		let t98: [uint32x4_t; 2] = [vaddq_u32(x3[0], t96[0]), vaddq_u32(x3[1], t96[1])];
		let t99: [uint32x4_t; 2] = [vcltq_u32(t98[0], x3[0]), vcltq_u32(t98[1], x3[1])];
		let t100: [uint32x4_t; 4] = [t98[0], t98[1], vadcq_u32(x3[2], t97[0], t99[0]), vadcq_u32(x3[3], t97[1], t99[1])];
		let t101: [usize; 8] = core::array::from_fn(|j| t75[j].wrapping_add(((1u64).wrapping_mul(8u64)) as u64 as usize));
		let mut t102 = [vdupq_n_u32(0); 4];
		{ let v = (t101[0] as *const u64).read_unaligned(); t102[0] = vsetq_lane_u32::<0>(v as u32, t102[0]); t102[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t102[2]); }
		{ let v = (t101[1] as *const u64).read_unaligned(); t102[0] = vsetq_lane_u32::<1>(v as u32, t102[0]); t102[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t102[2]); }
		{ let v = (t101[2] as *const u64).read_unaligned(); t102[0] = vsetq_lane_u32::<2>(v as u32, t102[0]); t102[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t102[2]); }
		{ let v = (t101[3] as *const u64).read_unaligned(); t102[0] = vsetq_lane_u32::<3>(v as u32, t102[0]); t102[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t102[2]); }
		{ let v = (t101[4] as *const u64).read_unaligned(); t102[1] = vsetq_lane_u32::<0>(v as u32, t102[1]); t102[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t102[3]); }
		{ let v = (t101[5] as *const u64).read_unaligned(); t102[1] = vsetq_lane_u32::<1>(v as u32, t102[1]); t102[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t102[3]); }
		{ let v = (t101[6] as *const u64).read_unaligned(); t102[1] = vsetq_lane_u32::<2>(v as u32, t102[1]); t102[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t102[3]); }
		{ let v = (t101[7] as *const u64).read_unaligned(); t102[1] = vsetq_lane_u32::<3>(v as u32, t102[1]); t102[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t102[3]); }
		let t103: [uint32x4_t; 2] = [vsubq_u32(t89[0], x4[0]), vsubq_u32(t89[1], x4[1])];
		let t104: [uint32x4_t; 2] = [t103[0], t103[1]];
		let t105: [uint32x4_t; 2] = [vandq_u32(t104[0], vdupq_n_u32(63)), vandq_u32(t104[1], vdupq_n_u32(63))];
		let t106: [uint32x4_t; 4] = [vshlq_u32(t102[0], vreinterpretq_s32_u32(t105[0])), vshlq_u32(t102[1], vreinterpretq_s32_u32(t105[1])), vorrq_u32(vshlq_u32(t102[2], vreinterpretq_s32_u32(t105[0])), vshlq_u32(t102[0], vsubq_s32(vreinterpretq_s32_u32(t105[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t102[3], vreinterpretq_s32_u32(t105[1])), vshlq_u32(t102[1], vsubq_s32(vreinterpretq_s32_u32(t105[1]), vdupq_n_s32(32))))];
		let t107: [uint32x4_t; 4] = [vorrq_u32(x5[0], t106[0]), vorrq_u32(x5[1], t106[1]), vorrq_u32(x5[2], t106[2]), vorrq_u32(x5[3], t106[3])];
		t92 = [vbslq_u32(t93[0], t107[0], t92[0]), vbslq_u32(t93[1], t107[1], t92[1]), vbslq_u32(t93[0], t107[2], t92[2]), vbslq_u32(t93[1], t107[3], t92[3])];
	}
	{
		t92 = [vbslq_u32(t94[0], x5[0], t92[0]), vbslq_u32(t94[1], x5[1], t92[1]), vbslq_u32(t94[0], x5[2], t92[2]), vbslq_u32(t94[1], x5[3], t92[3])];
	}
	res = t92;
	return res;
}

/// `ukl_ld_window`.
#[inline(always)]
unsafe fn b17_r0al1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1b: *const u8, x1i: [uint32x4_t; 2], x2: u32) -> [uint32x4_t; 4] {
	let mut x3 = [vdupq_n_u32(0); 4];
	let mut x4 = [vdupq_n_u32(0); 2];
	let mut x5 = [vdupq_n_u32(0); 4];
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 4];
	if ((x2) == (8u32)) {
		let t0 = vdupq_n_u32(maxv_on(&live, &x1i));
		let t1: [uint32x4_t; 2] = [vbslq_u32(live[0], x1i[0], t0), vbslq_u32(live[1], x1i[1], t0)];
		let mut t2 = [vdupq_n_u32(0); 4];
		let t3 = minv(&t1);
		if t3 == maxv(&t1) {
			let t4 = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(t3 as usize * 1) as *const u64).read_unaligned();
			t2 = [vdupq_n_u32(t4 as u32), vdupq_n_u32(t4 as u32), vdupq_n_u32((t4 >> 32) as u32), vdupq_n_u32((t4 >> 32) as u32)];
		} else {
			{ let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1[0]) as usize * 1) as *const u64).read_unaligned(); t2[0] = vsetq_lane_u32::<0>(v as u32, t2[0]); t2[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t2[2]); }
			{ let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1[0]) as usize * 1) as *const u64).read_unaligned(); t2[0] = vsetq_lane_u32::<1>(v as u32, t2[0]); t2[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t2[2]); }
			{ let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1[0]) as usize * 1) as *const u64).read_unaligned(); t2[0] = vsetq_lane_u32::<2>(v as u32, t2[0]); t2[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t2[2]); }
			{ let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1[0]) as usize * 1) as *const u64).read_unaligned(); t2[0] = vsetq_lane_u32::<3>(v as u32, t2[0]); t2[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t2[2]); }
			{ let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1[1]) as usize * 1) as *const u64).read_unaligned(); t2[1] = vsetq_lane_u32::<0>(v as u32, t2[1]); t2[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t2[3]); }
			{ let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1[1]) as usize * 1) as *const u64).read_unaligned(); t2[1] = vsetq_lane_u32::<1>(v as u32, t2[1]); t2[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t2[3]); }
			{ let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1[1]) as usize * 1) as *const u64).read_unaligned(); t2[1] = vsetq_lane_u32::<2>(v as u32, t2[1]); t2[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t2[3]); }
			{ let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1[1]) as usize * 1) as *const u64).read_unaligned(); t2[1] = vsetq_lane_u32::<3>(v as u32, t2[1]); t2[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t2[3]); }
		}
		res = t2;
		return res;
	}
	if ((x2) == (4u32)) {
		let t5 = vdupq_n_u32(maxv_on(&live, &x1i));
		let t6: [uint32x4_t; 2] = [vbslq_u32(live[0], x1i[0], t5), vbslq_u32(live[1], x1i[1], t5)];
		let mut t7 = [vdupq_n_u32(0); 2];
		let t8 = minv(&t6);
		if t8 == maxv(&t6) {
			t7 = [vdupq_n_u32(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(t8 as usize * 1) as *const u32).read_unaligned() as u32)); 2];
		} else {
			t7[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t6[0]) as usize * 1) as *const u32).read_unaligned() as u32), t7[0]);
			t7[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t6[0]) as usize * 1) as *const u32).read_unaligned() as u32), t7[0]);
			t7[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t6[0]) as usize * 1) as *const u32).read_unaligned() as u32), t7[0]);
			t7[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t6[0]) as usize * 1) as *const u32).read_unaligned() as u32), t7[0]);
			t7[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t6[1]) as usize * 1) as *const u32).read_unaligned() as u32), t7[1]);
			t7[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t6[1]) as usize * 1) as *const u32).read_unaligned() as u32), t7[1]);
			t7[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t6[1]) as usize * 1) as *const u32).read_unaligned() as u32), t7[1]);
			t7[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t6[1]) as usize * 1) as *const u32).read_unaligned() as u32), t7[1]);
		}
		let t9: [uint32x4_t; 4] = [t7[0], t7[1], vdupq_n_u32(0), vdupq_n_u32(0)];
		res = t9;
		return res;
	}
	if ((x2) == (2u32)) {
		let t10 = vdupq_n_u32(maxv_on(&live, &x1i));
		let t11: [uint32x4_t; 2] = [vbslq_u32(live[0], x1i[0], t10), vbslq_u32(live[1], x1i[1], t10)];
		let mut t12 = [vdupq_n_u32(0); 2];
		let t13 = minv(&t11);
		if t13 == maxv(&t11) {
			t12 = [vdupq_n_u32(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(t13 as usize * 1) as *const u16).read_unaligned() as u32)); 2];
		} else {
			t12[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t11[0]) as usize * 1) as *const u16).read_unaligned() as u32), t12[0]);
			t12[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t11[0]) as usize * 1) as *const u16).read_unaligned() as u32), t12[0]);
			t12[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t11[0]) as usize * 1) as *const u16).read_unaligned() as u32), t12[0]);
			t12[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t11[0]) as usize * 1) as *const u16).read_unaligned() as u32), t12[0]);
			t12[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t11[1]) as usize * 1) as *const u16).read_unaligned() as u32), t12[1]);
			t12[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t11[1]) as usize * 1) as *const u16).read_unaligned() as u32), t12[1]);
			t12[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t11[1]) as usize * 1) as *const u16).read_unaligned() as u32), t12[1]);
			t12[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t11[1]) as usize * 1) as *const u16).read_unaligned() as u32), t12[1]);
		}
		let t14: [uint32x4_t; 4] = [t12[0], t12[1], vdupq_n_u32(0), vdupq_n_u32(0)];
		res = t14;
		return res;
	}
	if ((x2) == (1u32)) {
		let t15 = vdupq_n_u32(maxv_on(&live, &x1i));
		let t16: [uint32x4_t; 2] = [vbslq_u32(live[0], x1i[0], t15), vbslq_u32(live[1], x1i[1], t15)];
		let mut t17 = [vdupq_n_u32(0); 2];
		let t18 = minv(&t16);
		if t18 == maxv(&t16) {
			t17 = [vdupq_n_u32(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(t18 as usize * 1) as *const u8).read_unaligned() as u32)); 2];
		} else {
			t17[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t16[0]) as usize * 1) as *const u8).read_unaligned() as u32), t17[0]);
			t17[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t16[0]) as usize * 1) as *const u8).read_unaligned() as u32), t17[0]);
			t17[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t16[0]) as usize * 1) as *const u8).read_unaligned() as u32), t17[0]);
			t17[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t16[0]) as usize * 1) as *const u8).read_unaligned() as u32), t17[0]);
			t17[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t16[1]) as usize * 1) as *const u8).read_unaligned() as u32), t17[1]);
			t17[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t16[1]) as usize * 1) as *const u8).read_unaligned() as u32), t17[1]);
			t17[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t16[1]) as usize * 1) as *const u8).read_unaligned() as u32), t17[1]);
			t17[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t16[1]) as usize * 1) as *const u8).read_unaligned() as u32), t17[1]);
		}
		let t19: [uint32x4_t; 4] = [t17[0], t17[1], vdupq_n_u32(0), vdupq_n_u32(0)];
		res = t19;
		return res;
	}
	let t20: [uint32x4_t; 2] = [x1i[0], x1i[1]];
	let t21: u64 = ((x1b) as usize as u64);
	let t22 = [vdupq_n_u32((t21) as u32); 2];
	let t23 = [vdupq_n_u32(((t21) >> 32) as u32); 2];
	let t24: [uint32x4_t; 2] = [vaddq_u32(t22[0], t20[0]), vaddq_u32(t22[1], t20[1])];
	let t25: [uint32x4_t; 2] = [vcltq_u32(t24[0], t22[0]), vcltq_u32(t24[1], t22[1])];
	let t26: [uint32x4_t; 4] = [t24[0], t24[1], vadcq_u32(t23[0], vdupq_n_u32(0), t25[0]), vadcq_u32(t23[1], vdupq_n_u32(0), t25[1])];
	let t27: u64 = !(7u64);
	let t28 = [vdupq_n_u32((t27) as u32); 2];
	let t29 = [vdupq_n_u32(((t27) >> 32) as u32); 2];
	let t30: [uint32x4_t; 4] = [vandq_u32(t26[0], t28[0]), vandq_u32(t26[1], t28[1]), vandq_u32(t26[2], t29[0]), vandq_u32(t26[3], t29[1])];
	let t31: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
	let t32: [uint32x4_t; 4] = [vandq_u32(t30[0], t31[0]), vandq_u32(t30[1], t31[1]), vandq_u32(t30[2], t31[2]), vandq_u32(t30[3], t31[3])];
	let t33: u64 = ((arg0) as usize as u64);
	let t34 = [vdupq_n_u32((t33) as u32); 2];
	let t35 = [vdupq_n_u32(((t33) >> 32) as u32); 2];
	let t36: [uint32x4_t; 2] = [vaddq_u32(t34[0], t32[0]), vaddq_u32(t34[1], t32[1])];
	let t37: [uint32x4_t; 2] = [vcltq_u32(t36[0], t34[0]), vcltq_u32(t36[1], t34[1])];
	let t38: [uint32x4_t; 4] = [t36[0], t36[1], vadcq_u32(t35[0], t32[2], t37[0]), vadcq_u32(t35[1], t32[3], t37[1])];
	x3 = t38;
	let t39 = words2([x3[0], x3[1]]);
	let t40 = words2([x3[2], x3[3]]);
	let t41 = bits(&live);
	let t42 = t41.trailing_zeros() as usize % 8;
	let t43: [usize; 8] = [{ let j = if t41 >> 0 & 1 != 0 { 0 } else { t42 }; ((t40[j] as u64) << 32 | t39[j] as u64) as usize }, { let j = if t41 >> 1 & 1 != 0 { 1 } else { t42 }; ((t40[j] as u64) << 32 | t39[j] as u64) as usize }, { let j = if t41 >> 2 & 1 != 0 { 2 } else { t42 }; ((t40[j] as u64) << 32 | t39[j] as u64) as usize }, { let j = if t41 >> 3 & 1 != 0 { 3 } else { t42 }; ((t40[j] as u64) << 32 | t39[j] as u64) as usize }, { let j = if t41 >> 4 & 1 != 0 { 4 } else { t42 }; ((t40[j] as u64) << 32 | t39[j] as u64) as usize }, { let j = if t41 >> 5 & 1 != 0 { 5 } else { t42 }; ((t40[j] as u64) << 32 | t39[j] as u64) as usize }, { let j = if t41 >> 6 & 1 != 0 { 6 } else { t42 }; ((t40[j] as u64) << 32 | t39[j] as u64) as usize }, { let j = if t41 >> 7 & 1 != 0 { 7 } else { t42 }; ((t40[j] as u64) << 32 | t39[j] as u64) as usize }];
	let t44: [uint32x4_t; 2] = [x1i[0], x1i[1]];
	let t45: u64 = ((x1b) as usize as u64);
	let t46 = [vdupq_n_u32((t45) as u32); 2];
	let t47 = [vdupq_n_u32(((t45) >> 32) as u32); 2];
	let t48: [uint32x4_t; 2] = [vaddq_u32(t46[0], t44[0]), vaddq_u32(t46[1], t44[1])];
	let t49: [uint32x4_t; 2] = [vcltq_u32(t48[0], t46[0]), vcltq_u32(t48[1], t46[1])];
	let t50: [uint32x4_t; 4] = [t48[0], t48[1], vadcq_u32(t47[0], vdupq_n_u32(0), t49[0]), vadcq_u32(t47[1], vdupq_n_u32(0), t49[1])];
	let t51: u64 = 7u64;
	let t52 = [vdupq_n_u32((t51) as u32); 2];
	let t53: [uint32x4_t; 2] = [vandq_u32(t50[0], t52[0]), vandq_u32(t50[1], t52[1])];
	let t54 = [vdupq_n_u32(8u32); 2];
	let t55: [uint32x4_t; 2] = [vmulq_u32(t53[0], t54[0]), vmulq_u32(t53[1], t54[1])];
	x4 = t55;
	let mut t56 = [vdupq_n_u32(0); 4];
	{ let v = (t43[0] as *const u64).read_unaligned(); t56[0] = vsetq_lane_u32::<0>(v as u32, t56[0]); t56[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t56[2]); }
	{ let v = (t43[1] as *const u64).read_unaligned(); t56[0] = vsetq_lane_u32::<1>(v as u32, t56[0]); t56[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t56[2]); }
	{ let v = (t43[2] as *const u64).read_unaligned(); t56[0] = vsetq_lane_u32::<2>(v as u32, t56[0]); t56[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t56[2]); }
	{ let v = (t43[3] as *const u64).read_unaligned(); t56[0] = vsetq_lane_u32::<3>(v as u32, t56[0]); t56[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t56[2]); }
	{ let v = (t43[4] as *const u64).read_unaligned(); t56[1] = vsetq_lane_u32::<0>(v as u32, t56[1]); t56[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t56[3]); }
	{ let v = (t43[5] as *const u64).read_unaligned(); t56[1] = vsetq_lane_u32::<1>(v as u32, t56[1]); t56[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t56[3]); }
	{ let v = (t43[6] as *const u64).read_unaligned(); t56[1] = vsetq_lane_u32::<2>(v as u32, t56[1]); t56[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t56[3]); }
	{ let v = (t43[7] as *const u64).read_unaligned(); t56[1] = vsetq_lane_u32::<3>(v as u32, t56[1]); t56[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t56[3]); }
	let t57: [uint32x4_t; 2] = [x4[0], x4[1]];
	let t58: [uint32x4_t; 2] = [vandq_u32(t57[0], vdupq_n_u32(63)), vandq_u32(t57[1], vdupq_n_u32(63))];
	let t59: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t56[0], vnegq_s32(vreinterpretq_s32_u32(t58[0]))), vshlq_u32(t56[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t58[0])))), vorrq_u32(vshlq_u32(t56[1], vnegq_s32(vreinterpretq_s32_u32(t58[1]))), vshlq_u32(t56[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t58[1])))), vshlq_u32(t56[2], vnegq_s32(vreinterpretq_s32_u32(t58[0]))), vshlq_u32(t56[3], vnegq_s32(vreinterpretq_s32_u32(t58[1])))];
	x5 = t59;
	let t60 = [vdupq_n_u32(0u32); 2];
	let t61: [uint32x4_t; 2] = [vcneq_u32(x4[0], t60[0]), vcneq_u32(x4[1], t60[1])];
	let t62 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
	let t63: [uint32x4_t; 2] = [vaddq_u32(x4[0], t62[0]), vaddq_u32(x4[1], t62[1])];
	let t64 = [vdupq_n_u32(64u32); 2];
	let t65: [uint32x4_t; 2] = [vcgtq_u32(t63[0], t64[0]), vcgtq_u32(t63[1], t64[1])];
	let t66: [uint32x4_t; 2] = [vandq_u32(t61[0], t65[0]), vandq_u32(t61[1], t65[1])];
	let mut t67 = [vdupq_n_u32(0); 4];
	let t68: [uint32x4_t; 2] = [vandq_u32(live[0], t66[0]), vandq_u32(live[1], t66[1])];
	let t69: [uint32x4_t; 2] = [vbicq_u32(live[0], t66[0]), vbicq_u32(live[1], t66[1])];
	{
		let t70: u64 = (1u64).wrapping_mul(8u64);
		let t71 = [vdupq_n_u32((t70) as u32); 2];
		let t72 = [vdupq_n_u32(((t70) >> 32) as u32); 2];
		let t73: [uint32x4_t; 2] = [vaddq_u32(x3[0], t71[0]), vaddq_u32(x3[1], t71[1])];
		let t74: [uint32x4_t; 2] = [vcltq_u32(t73[0], x3[0]), vcltq_u32(t73[1], x3[1])];
		let t75: [uint32x4_t; 4] = [t73[0], t73[1], vadcq_u32(x3[2], t72[0], t74[0]), vadcq_u32(x3[3], t72[1], t74[1])];
		let t76: [usize; 8] = core::array::from_fn(|j| t43[j].wrapping_add(((1u64).wrapping_mul(8u64)) as u64 as usize));
		let mut t77 = [vdupq_n_u32(0); 4];
		{ let v = (t76[0] as *const u64).read_unaligned(); t77[0] = vsetq_lane_u32::<0>(v as u32, t77[0]); t77[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t77[2]); }
		{ let v = (t76[1] as *const u64).read_unaligned(); t77[0] = vsetq_lane_u32::<1>(v as u32, t77[0]); t77[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t77[2]); }
		{ let v = (t76[2] as *const u64).read_unaligned(); t77[0] = vsetq_lane_u32::<2>(v as u32, t77[0]); t77[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t77[2]); }
		{ let v = (t76[3] as *const u64).read_unaligned(); t77[0] = vsetq_lane_u32::<3>(v as u32, t77[0]); t77[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t77[2]); }
		{ let v = (t76[4] as *const u64).read_unaligned(); t77[1] = vsetq_lane_u32::<0>(v as u32, t77[1]); t77[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t77[3]); }
		{ let v = (t76[5] as *const u64).read_unaligned(); t77[1] = vsetq_lane_u32::<1>(v as u32, t77[1]); t77[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t77[3]); }
		{ let v = (t76[6] as *const u64).read_unaligned(); t77[1] = vsetq_lane_u32::<2>(v as u32, t77[1]); t77[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t77[3]); }
		{ let v = (t76[7] as *const u64).read_unaligned(); t77[1] = vsetq_lane_u32::<3>(v as u32, t77[1]); t77[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t77[3]); }
		let t78: [uint32x4_t; 2] = [vsubq_u32(t64[0], x4[0]), vsubq_u32(t64[1], x4[1])];
		let t79: [uint32x4_t; 2] = [t78[0], t78[1]];
		let t80: [uint32x4_t; 2] = [vandq_u32(t79[0], vdupq_n_u32(63)), vandq_u32(t79[1], vdupq_n_u32(63))];
		let t81: [uint32x4_t; 4] = [vshlq_u32(t77[0], vreinterpretq_s32_u32(t80[0])), vshlq_u32(t77[1], vreinterpretq_s32_u32(t80[1])), vorrq_u32(vshlq_u32(t77[2], vreinterpretq_s32_u32(t80[0])), vshlq_u32(t77[0], vsubq_s32(vreinterpretq_s32_u32(t80[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t77[3], vreinterpretq_s32_u32(t80[1])), vshlq_u32(t77[1], vsubq_s32(vreinterpretq_s32_u32(t80[1]), vdupq_n_s32(32))))];
		let t82: [uint32x4_t; 4] = [vorrq_u32(x5[0], t81[0]), vorrq_u32(x5[1], t81[1]), vorrq_u32(x5[2], t81[2]), vorrq_u32(x5[3], t81[3])];
		t67 = [vbslq_u32(t68[0], t82[0], t67[0]), vbslq_u32(t68[1], t82[1], t67[1]), vbslq_u32(t68[0], t82[2], t67[2]), vbslq_u32(t68[1], t82[3], t67[3])];
	}
	{
		t67 = [vbslq_u32(t69[0], x5[0], t67[0]), vbslq_u32(t69[1], x5[1], t67[1]), vbslq_u32(t69[0], x5[2], t67[2]), vbslq_u32(t69[1], x5[3], t67[3])];
	}
	res = t67;
	return res;
}

/// `ukl_mask64`.
#[inline(always)]
unsafe fn b18_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
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
unsafe fn b19_r0sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1: u32, x2: [uint32x4_t; 2], x3: u32) -> [uint32x4_t; 4] {
	let mut x4 = [vdupq_n_u32(0); 4];
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 4];
	let t0 = b17_r0al1s_p(l, lid0, live, arg0, ((((x1) as u64)) as usize as *const u8), x2, x3);
	x4 = t0;
	let t1: [uint32x4_t; 4];
	if ((x3) >= (8u32)) {
		t1 = x4;
	} else {
		let t2 = b18_s_p(l, lid0, live, (8u32).wrapping_mul(x3));
		let t3: u64 = t2;
		let t4 = [vdupq_n_u32((t3) as u32); 2];
		let t5 = [vdupq_n_u32(((t3) >> 32) as u32); 2];
		let t6: [uint32x4_t; 4] = [vandq_u32(x4[0], t4[0]), vandq_u32(x4[1], t4[1]), vandq_u32(x4[2], t5[0]), vandq_u32(x4[3], t5[1])];
		t1 = t6;
	}
	res = t1;
	return res;
}

/// The group `l.group`'s threads, 8 at a time: `argv[j]` the address of argument `j`'s bytes (a buffer's address,
/// a value's bytes).
///
/// # Safety
///
/// `argv` holds the kernel's arguments, its buffers as large as it reads and writes; `l` is a 1-D group's launch.
pub unsafe fn parent_step_w8(argv: *const *const u8, l: &Launch) {
	let n = l.block[0] * l.block[1] * l.block[2];
	let arg0: *const u8 = (*argv.add(0) as *const *const u8).read_unaligned();
	let arg1: *const u8 = (*argv.add(1) as *const *const u8).read_unaligned();
	let arg2: *const u8 = (*argv.add(2) as *const *const u8).read_unaligned();
	let x3: *const u8 = *argv.add(3);
	let arg4: *const u8 = (*argv.add(4) as *const *const u8).read_unaligned();
	let x5: u32 = (*argv.add(5) as *const u32).read_unaligned();
	let mut t587: bool = !(arg4).is_null();
	if t587 {
		let mut t588: bool = ((l.group[0]) >= ((arg4.wrapping_add((((x5) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
		if !t588 {
			t588 = ((l.group[1]) >= ((arg4.wrapping_add(((((x5).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
		}
		let mut t589: bool = t588;
		if !t589 {
			t589 = ((l.group[2]) >= ((arg4.wrapping_add(((((x5).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
		}
		t587 = t589;
	}
	if t587 {
		return;
	}
	let mut lid0 = 0u32;
	while lid0 + 8 <= n {
		let mut live = [vdupq_n_u32(u32::MAX); 2];
		let gid0 = l.group[0] * l.block[0] + lid0;
		'chunk: {
			b0_r0r1r2sg_f(l, lid0, arg0, arg1, arg2, x3);
		}
		lid0 += 8;
	}
	if lid0 < n {
		// Lanes part: the group's last threads, those under `n`.
		let t590: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
		let mut live = t590;
		let gid0 = l.group[0] * l.block[0] + lid0;
		'chunk: {
			b0_r0r1r2sg_p(l, lid0, live, arg0, arg1, arg2, x3);
		}
	}
}
