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

#[inline(always)]
unsafe fn b0_r0r1r2r3sg_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, arg2: *const u8, arg3: *const u8, x32: *const u8) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x32 as *const u32).read_unaligned();
    let a1: u64 = (x32.wrapping_add(8u64 as usize) as *const u64).read_unaligned();
    let mut x56: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x64: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x72: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x80: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x88: u32 = 0;
    let mut x96: u32 = 0;
    let mut x104: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x112: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x120: u32 = 0;
    let mut live = k;
    let t1: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
    let t2 = [vdupq_n_u32(a0); 2];
    let t3: [uint32x4_t; 2] = [vcgeq_u32(t1[0], t2[0]), vcgeq_u32(t1[1], t2[1])];
    let t4: [uint32x4_t; 2] = [vandq_u32(live[0], t3[0]), vandq_u32(live[1], t3[1])];
    {
        live = [vbicq_u32(live[0], t4[0]), vbicq_u32(live[1], t4[1])];
    }
    if any(&live) {
        let t5 = (arg0 as *const u8).add(((gid0) as usize) * 8);
        let mut t6: [uint32x4_t; 4];
        if all(&live) {
            let t70 = vld2q_u32((t5 as *const u32).add(0));
            let t71 = vld2q_u32((t5 as *const u32).add(8));
            let t7: [uint32x4_t; 4] = [t70.0, t71.0, t70.1, t71.1];
            t6 = t7;
        } else {
            let t8 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t1[0]), vandq_u32(live[1], t1[1]))));
            let t9: [uint32x4_t; 2] = [vbslq_u32(live[0], t1[0], t8), vbslq_u32(live[1], t1[1], t8)];
            let mut t10: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t9[0]) as usize * 8) as *const u64).read_unaligned(); t10[0] = vsetq_lane_u32::<0>(v as u32, t10[0]); t10[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t10[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t9[0]) as usize * 8) as *const u64).read_unaligned(); t10[0] = vsetq_lane_u32::<1>(v as u32, t10[0]); t10[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t10[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t9[0]) as usize * 8) as *const u64).read_unaligned(); t10[0] = vsetq_lane_u32::<2>(v as u32, t10[0]); t10[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t10[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t9[0]) as usize * 8) as *const u64).read_unaligned(); t10[0] = vsetq_lane_u32::<3>(v as u32, t10[0]); t10[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t10[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t9[1]) as usize * 8) as *const u64).read_unaligned(); t10[1] = vsetq_lane_u32::<0>(v as u32, t10[1]); t10[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t10[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t9[1]) as usize * 8) as *const u64).read_unaligned(); t10[1] = vsetq_lane_u32::<1>(v as u32, t10[1]); t10[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t10[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t9[1]) as usize * 8) as *const u64).read_unaligned(); t10[1] = vsetq_lane_u32::<2>(v as u32, t10[1]); t10[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t10[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t9[1]) as usize * 8) as *const u64).read_unaligned(); t10[1] = vsetq_lane_u32::<3>(v as u32, t10[1]); t10[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t10[3]); }
            t6 = t10;
        }
        x56 = t6;
        let t11 = (arg1 as *const u8).add(((gid0) as usize) * 8);
        let mut t12: [uint32x4_t; 4];
        if all(&live) {
            let t130 = vld2q_u32((t11 as *const u32).add(0));
            let t131 = vld2q_u32((t11 as *const u32).add(8));
            let t13: [uint32x4_t; 4] = [t130.0, t131.0, t130.1, t131.1];
            t12 = t13;
        } else {
            let t14 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t1[0]), vandq_u32(live[1], t1[1]))));
            let t15: [uint32x4_t; 2] = [vbslq_u32(live[0], t1[0], t14), vbslq_u32(live[1], t1[1], t14)];
            let mut t16: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<0>(t15[0]) as usize * 8) as *const u64).read_unaligned(); t16[0] = vsetq_lane_u32::<0>(v as u32, t16[0]); t16[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t16[2]); }
            { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<1>(t15[0]) as usize * 8) as *const u64).read_unaligned(); t16[0] = vsetq_lane_u32::<1>(v as u32, t16[0]); t16[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t16[2]); }
            { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<2>(t15[0]) as usize * 8) as *const u64).read_unaligned(); t16[0] = vsetq_lane_u32::<2>(v as u32, t16[0]); t16[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t16[2]); }
            { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<3>(t15[0]) as usize * 8) as *const u64).read_unaligned(); t16[0] = vsetq_lane_u32::<3>(v as u32, t16[0]); t16[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t16[2]); }
            { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<0>(t15[1]) as usize * 8) as *const u64).read_unaligned(); t16[1] = vsetq_lane_u32::<0>(v as u32, t16[1]); t16[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t16[3]); }
            { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<1>(t15[1]) as usize * 8) as *const u64).read_unaligned(); t16[1] = vsetq_lane_u32::<1>(v as u32, t16[1]); t16[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t16[3]); }
            { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<2>(t15[1]) as usize * 8) as *const u64).read_unaligned(); t16[1] = vsetq_lane_u32::<2>(v as u32, t16[1]); t16[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t16[3]); }
            { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<3>(t15[1]) as usize * 8) as *const u64).read_unaligned(); t16[1] = vsetq_lane_u32::<3>(v as u32, t16[1]); t16[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t16[3]); }
            t12 = t16;
        }
        x64 = t12;
        let mut t17: [uint32x4_t; 2];
        if all(&live) {
            let t18 = (arg2 as *const u32).add((gid0) as usize);
            t17 = [vld1q_u32(t18.add(0)), vld1q_u32(t18.add(4))];
        } else {
            let t19 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t1[0]), vandq_u32(live[1], t1[1]))));
            let t20: [uint32x4_t; 2] = [vbslq_u32(live[0], t1[0], t19), vbslq_u32(live[1], t1[1], t19)];
            let t21 = arg2 as *const u32;
            let mut t22: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            t22[0] = vld1q_lane_u32::<0>(t21.add(vgetq_lane_u32::<0>(t20[0]) as usize), t22[0]);
            t22[0] = vld1q_lane_u32::<1>(t21.add(vgetq_lane_u32::<1>(t20[0]) as usize), t22[0]);
            t22[0] = vld1q_lane_u32::<2>(t21.add(vgetq_lane_u32::<2>(t20[0]) as usize), t22[0]);
            t22[0] = vld1q_lane_u32::<3>(t21.add(vgetq_lane_u32::<3>(t20[0]) as usize), t22[0]);
            t22[1] = vld1q_lane_u32::<0>(t21.add(vgetq_lane_u32::<0>(t20[1]) as usize), t22[1]);
            t22[1] = vld1q_lane_u32::<1>(t21.add(vgetq_lane_u32::<1>(t20[1]) as usize), t22[1]);
            t22[1] = vld1q_lane_u32::<2>(t21.add(vgetq_lane_u32::<2>(t20[1]) as usize), t22[1]);
            t22[1] = vld1q_lane_u32::<3>(t21.add(vgetq_lane_u32::<3>(t20[1]) as usize), t22[1]);
            t17 = t22;
        }
        x72 = t17;
        let t23 = [vdupq_n_u32(63u32); 2];
        let t24: [uint32x4_t; 2] = [vandq_u32(x72[0], t23[0]), vandq_u32(x72[1], t23[1])];
        x80 = t24;
        x88 = a0;
        let t25: u32;
        if ((((a1) & (1u64))) != (0u64)) {
            t25 = 8u32;
        } else {
            t25 = 4u32;
        }
        x96 = t25;
        let t26: [uint32x4_t; 2] = [vaddq_u32(x56[0], x64[0]), vaddq_u32(x56[1], x64[1])];
        let t27: [uint32x4_t; 2] = [vcltq_u32(t26[0], x56[0]), vcltq_u32(t26[1], x56[1])];
        let t28: [uint32x4_t; 4] = [t26[0], t26[1], vsubq_u32(vaddq_u32(x56[2], x64[2]), t27[0]), vsubq_u32(vaddq_u32(x56[3], x64[3]), t27[1])];
        let t29 = (arg3 as *const u8).add(((gid0.wrapping_add((0u32).wrapping_mul(x88))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t29 as *mut u32).add(0), uint32x4x2_t(t28[0], t28[2]));
            vst2q_u32((t29 as *mut u32).add(8), uint32x4x2_t(t28[1], t28[3]));
        } else {
            let mut t30w = 0u64;
            let t30 = core::hint::black_box(&raw mut t30w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t29 as *mut u64).add(0), t30).write_unaligned(((vgetq_lane_u32::<0>(t28[2]) as u64) << 32 | vgetq_lane_u32::<0>(t28[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t29 as *mut u64).add(1), t30).write_unaligned(((vgetq_lane_u32::<1>(t28[2]) as u64) << 32 | vgetq_lane_u32::<1>(t28[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t29 as *mut u64).add(2), t30).write_unaligned(((vgetq_lane_u32::<2>(t28[2]) as u64) << 32 | vgetq_lane_u32::<2>(t28[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t29 as *mut u64).add(3), t30).write_unaligned(((vgetq_lane_u32::<3>(t28[2]) as u64) << 32 | vgetq_lane_u32::<3>(t28[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t29 as *mut u64).add(4), t30).write_unaligned(((vgetq_lane_u32::<0>(t28[3]) as u64) << 32 | vgetq_lane_u32::<0>(t28[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t29 as *mut u64).add(5), t30).write_unaligned(((vgetq_lane_u32::<1>(t28[3]) as u64) << 32 | vgetq_lane_u32::<1>(t28[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t29 as *mut u64).add(6), t30).write_unaligned(((vgetq_lane_u32::<2>(t28[3]) as u64) << 32 | vgetq_lane_u32::<2>(t28[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t29 as *mut u64).add(7), t30).write_unaligned(((vgetq_lane_u32::<3>(t28[3]) as u64) << 32 | vgetq_lane_u32::<3>(t28[1]) as u64));
        }
        let t31: [uint32x4_t; 2] = [vcltq_u32(x56[0], x64[0]), vcltq_u32(x56[1], x64[1])];
        let t32: [uint32x4_t; 4] = [vsubq_u32(x56[0], x64[0]), vsubq_u32(x56[1], x64[1]), vaddq_u32(vsubq_u32(x56[2], x64[2]), t31[0]), vaddq_u32(vsubq_u32(x56[3], x64[3]), t31[1])];
        let t33 = (arg3 as *const u8).add(((gid0.wrapping_add((1u32).wrapping_mul(x88))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t33 as *mut u32).add(0), uint32x4x2_t(t32[0], t32[2]));
            vst2q_u32((t33 as *mut u32).add(8), uint32x4x2_t(t32[1], t32[3]));
        } else {
            let mut t34w = 0u64;
            let t34 = core::hint::black_box(&raw mut t34w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t33 as *mut u64).add(0), t34).write_unaligned(((vgetq_lane_u32::<0>(t32[2]) as u64) << 32 | vgetq_lane_u32::<0>(t32[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t33 as *mut u64).add(1), t34).write_unaligned(((vgetq_lane_u32::<1>(t32[2]) as u64) << 32 | vgetq_lane_u32::<1>(t32[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t33 as *mut u64).add(2), t34).write_unaligned(((vgetq_lane_u32::<2>(t32[2]) as u64) << 32 | vgetq_lane_u32::<2>(t32[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t33 as *mut u64).add(3), t34).write_unaligned(((vgetq_lane_u32::<3>(t32[2]) as u64) << 32 | vgetq_lane_u32::<3>(t32[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t33 as *mut u64).add(4), t34).write_unaligned(((vgetq_lane_u32::<0>(t32[3]) as u64) << 32 | vgetq_lane_u32::<0>(t32[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t33 as *mut u64).add(5), t34).write_unaligned(((vgetq_lane_u32::<1>(t32[3]) as u64) << 32 | vgetq_lane_u32::<1>(t32[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t33 as *mut u64).add(6), t34).write_unaligned(((vgetq_lane_u32::<2>(t32[3]) as u64) << 32 | vgetq_lane_u32::<2>(t32[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t33 as *mut u64).add(7), t34).write_unaligned(((vgetq_lane_u32::<3>(t32[3]) as u64) << 32 | vgetq_lane_u32::<3>(t32[1]) as u64));
        }
        let t35: [uint32x4_t; 2] = [x80[0], x80[1]];
        let t36: [uint32x4_t; 2] = [vandq_u32(t35[0], vdupq_n_u32(63)), vandq_u32(t35[1], vdupq_n_u32(63))];
        let t37: [uint32x4_t; 4] = [vshlq_u32(x56[0], vreinterpretq_s32_u32(t36[0])), vshlq_u32(x56[1], vreinterpretq_s32_u32(t36[1])), vorrq_u32(vshlq_u32(x56[2], vreinterpretq_s32_u32(t36[0])), vshlq_u32(x56[0], vsubq_s32(vreinterpretq_s32_u32(t36[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(x56[3], vreinterpretq_s32_u32(t36[1])), vshlq_u32(x56[1], vsubq_s32(vreinterpretq_s32_u32(t36[1]), vdupq_n_s32(32))))];
        let t38 = (arg3 as *const u8).add(((gid0.wrapping_add((2u32).wrapping_mul(x88))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t38 as *mut u32).add(0), uint32x4x2_t(t37[0], t37[2]));
            vst2q_u32((t38 as *mut u32).add(8), uint32x4x2_t(t37[1], t37[3]));
        } else {
            let mut t39w = 0u64;
            let t39 = core::hint::black_box(&raw mut t39w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t38 as *mut u64).add(0), t39).write_unaligned(((vgetq_lane_u32::<0>(t37[2]) as u64) << 32 | vgetq_lane_u32::<0>(t37[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t38 as *mut u64).add(1), t39).write_unaligned(((vgetq_lane_u32::<1>(t37[2]) as u64) << 32 | vgetq_lane_u32::<1>(t37[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t38 as *mut u64).add(2), t39).write_unaligned(((vgetq_lane_u32::<2>(t37[2]) as u64) << 32 | vgetq_lane_u32::<2>(t37[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t38 as *mut u64).add(3), t39).write_unaligned(((vgetq_lane_u32::<3>(t37[2]) as u64) << 32 | vgetq_lane_u32::<3>(t37[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t38 as *mut u64).add(4), t39).write_unaligned(((vgetq_lane_u32::<0>(t37[3]) as u64) << 32 | vgetq_lane_u32::<0>(t37[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t38 as *mut u64).add(5), t39).write_unaligned(((vgetq_lane_u32::<1>(t37[3]) as u64) << 32 | vgetq_lane_u32::<1>(t37[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t38 as *mut u64).add(6), t39).write_unaligned(((vgetq_lane_u32::<2>(t37[3]) as u64) << 32 | vgetq_lane_u32::<2>(t37[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t38 as *mut u64).add(7), t39).write_unaligned(((vgetq_lane_u32::<3>(t37[3]) as u64) << 32 | vgetq_lane_u32::<3>(t37[1]) as u64));
        }
        let t40: [uint32x4_t; 2] = [x80[0], x80[1]];
        let t41: [uint32x4_t; 2] = [vandq_u32(t40[0], vdupq_n_u32(63)), vandq_u32(t40[1], vdupq_n_u32(63))];
        let t42: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(x56[0], vnegq_s32(vreinterpretq_s32_u32(t41[0]))), vshlq_u32(x56[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t41[0])))), vorrq_u32(vshlq_u32(x56[1], vnegq_s32(vreinterpretq_s32_u32(t41[1]))), vshlq_u32(x56[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t41[1])))), vshlq_u32(x56[2], vnegq_s32(vreinterpretq_s32_u32(t41[0]))), vshlq_u32(x56[3], vnegq_s32(vreinterpretq_s32_u32(t41[1])))];
        let t43 = (arg3 as *const u8).add(((gid0.wrapping_add((3u32).wrapping_mul(x88))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t43 as *mut u32).add(0), uint32x4x2_t(t42[0], t42[2]));
            vst2q_u32((t43 as *mut u32).add(8), uint32x4x2_t(t42[1], t42[3]));
        } else {
            let mut t44w = 0u64;
            let t44 = core::hint::black_box(&raw mut t44w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t43 as *mut u64).add(0), t44).write_unaligned(((vgetq_lane_u32::<0>(t42[2]) as u64) << 32 | vgetq_lane_u32::<0>(t42[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t43 as *mut u64).add(1), t44).write_unaligned(((vgetq_lane_u32::<1>(t42[2]) as u64) << 32 | vgetq_lane_u32::<1>(t42[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t43 as *mut u64).add(2), t44).write_unaligned(((vgetq_lane_u32::<2>(t42[2]) as u64) << 32 | vgetq_lane_u32::<2>(t42[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t43 as *mut u64).add(3), t44).write_unaligned(((vgetq_lane_u32::<3>(t42[2]) as u64) << 32 | vgetq_lane_u32::<3>(t42[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t43 as *mut u64).add(4), t44).write_unaligned(((vgetq_lane_u32::<0>(t42[3]) as u64) << 32 | vgetq_lane_u32::<0>(t42[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t43 as *mut u64).add(5), t44).write_unaligned(((vgetq_lane_u32::<1>(t42[3]) as u64) << 32 | vgetq_lane_u32::<1>(t42[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t43 as *mut u64).add(6), t44).write_unaligned(((vgetq_lane_u32::<2>(t42[3]) as u64) << 32 | vgetq_lane_u32::<2>(t42[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t43 as *mut u64).add(7), t44).write_unaligned(((vgetq_lane_u32::<3>(t42[3]) as u64) << 32 | vgetq_lane_u32::<3>(t42[1]) as u64));
        }
        let t45: [uint32x4_t; 2] = [x80[0], x80[1]];
        let t46: [uint32x4_t; 2] = [vandq_u32(t45[0], vdupq_n_u32(63)), vandq_u32(t45[1], vdupq_n_u32(63))];
        let t47: [uint32x4_t; 2] = [vcgeq_s32(vreinterpretq_s32_u32(t46[0]), vdupq_n_s32(32)), vcgeq_s32(vreinterpretq_s32_u32(t46[1]), vdupq_n_s32(32))];
        let t48: [uint32x4_t; 4] = [vbslq_u32(t47[0], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x56[2]), vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t46[0])))), vorrq_u32(vshlq_u32(x56[0], vnegq_s32(vreinterpretq_s32_u32(t46[0]))), vshlq_u32(x56[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t46[0]))))), vbslq_u32(t47[1], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x56[3]), vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t46[1])))), vorrq_u32(vshlq_u32(x56[1], vnegq_s32(vreinterpretq_s32_u32(t46[1]))), vshlq_u32(x56[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t46[1]))))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x56[2]), vnegq_s32(vreinterpretq_s32_u32(t46[0])))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x56[3]), vnegq_s32(vreinterpretq_s32_u32(t46[1]))))];
        let t49 = (arg3 as *const u8).add(((gid0.wrapping_add((4u32).wrapping_mul(x88))) as usize) * 8);
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
        let t51: [uint32x4_t; 4] = [vshlq_n_u32::<13>(x56[0]), vshlq_n_u32::<13>(x56[1]), vorrq_u32(vshlq_n_u32::<13>(x56[2]), vshrq_n_u32::<19>(x56[0])), vorrq_u32(vshlq_n_u32::<13>(x56[3]), vshrq_n_u32::<19>(x56[1]))];
        let t52: [uint32x4_t; 2] = [vshrq_n_u32::<19>(x64[2]), vshrq_n_u32::<19>(x64[3])];
        let t53: [uint32x4_t; 4] = [vorrq_u32(t51[0], t52[0]), vorrq_u32(t51[1], t52[1]), t51[2], t51[3]];
        let t54 = (arg3 as *const u8).add(((gid0.wrapping_add((5u32).wrapping_mul(x88))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t54 as *mut u32).add(0), uint32x4x2_t(t53[0], t53[2]));
            vst2q_u32((t54 as *mut u32).add(8), uint32x4x2_t(t53[1], t53[3]));
        } else {
            let mut t55w = 0u64;
            let t55 = core::hint::black_box(&raw mut t55w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t54 as *mut u64).add(0), t55).write_unaligned(((vgetq_lane_u32::<0>(t53[2]) as u64) << 32 | vgetq_lane_u32::<0>(t53[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t54 as *mut u64).add(1), t55).write_unaligned(((vgetq_lane_u32::<1>(t53[2]) as u64) << 32 | vgetq_lane_u32::<1>(t53[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t54 as *mut u64).add(2), t55).write_unaligned(((vgetq_lane_u32::<2>(t53[2]) as u64) << 32 | vgetq_lane_u32::<2>(t53[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t54 as *mut u64).add(3), t55).write_unaligned(((vgetq_lane_u32::<3>(t53[2]) as u64) << 32 | vgetq_lane_u32::<3>(t53[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t54 as *mut u64).add(4), t55).write_unaligned(((vgetq_lane_u32::<0>(t53[3]) as u64) << 32 | vgetq_lane_u32::<0>(t53[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t54 as *mut u64).add(5), t55).write_unaligned(((vgetq_lane_u32::<1>(t53[3]) as u64) << 32 | vgetq_lane_u32::<1>(t53[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t54 as *mut u64).add(6), t55).write_unaligned(((vgetq_lane_u32::<2>(t53[3]) as u64) << 32 | vgetq_lane_u32::<2>(t53[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t54 as *mut u64).add(7), t55).write_unaligned(((vgetq_lane_u32::<3>(t53[3]) as u64) << 32 | vgetq_lane_u32::<3>(t53[1]) as u64));
        }
        let t56: [uint32x4_t; 4] = [vdupq_n_u32(0), vdupq_n_u32(0), vshlq_n_u32::<8>(x56[0]), vshlq_n_u32::<8>(x56[1])];
        let t57 = (arg3 as *const u8).add(((gid0.wrapping_add((6u32).wrapping_mul(x88))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t57 as *mut u32).add(0), uint32x4x2_t(t56[0], t56[2]));
            vst2q_u32((t57 as *mut u32).add(8), uint32x4x2_t(t56[1], t56[3]));
        } else {
            let mut t58w = 0u64;
            let t58 = core::hint::black_box(&raw mut t58w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t57 as *mut u64).add(0), t58).write_unaligned(((vgetq_lane_u32::<0>(t56[2]) as u64) << 32 | vgetq_lane_u32::<0>(t56[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t57 as *mut u64).add(1), t58).write_unaligned(((vgetq_lane_u32::<1>(t56[2]) as u64) << 32 | vgetq_lane_u32::<1>(t56[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t57 as *mut u64).add(2), t58).write_unaligned(((vgetq_lane_u32::<2>(t56[2]) as u64) << 32 | vgetq_lane_u32::<2>(t56[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t57 as *mut u64).add(3), t58).write_unaligned(((vgetq_lane_u32::<3>(t56[2]) as u64) << 32 | vgetq_lane_u32::<3>(t56[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t57 as *mut u64).add(4), t58).write_unaligned(((vgetq_lane_u32::<0>(t56[3]) as u64) << 32 | vgetq_lane_u32::<0>(t56[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t57 as *mut u64).add(5), t58).write_unaligned(((vgetq_lane_u32::<1>(t56[3]) as u64) << 32 | vgetq_lane_u32::<1>(t56[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t57 as *mut u64).add(6), t58).write_unaligned(((vgetq_lane_u32::<2>(t56[3]) as u64) << 32 | vgetq_lane_u32::<2>(t56[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t57 as *mut u64).add(7), t58).write_unaligned(((vgetq_lane_u32::<3>(t56[3]) as u64) << 32 | vgetq_lane_u32::<3>(t56[1]) as u64));
        }
        let t59: [uint32x4_t; 2] = [vshrq_n_u32::<3>(x56[2]), vshrq_n_u32::<3>(x56[3])];
        let t60: [uint32x4_t; 4] = [t59[0], t59[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        let t61 = (arg3 as *const u8).add(((gid0.wrapping_add((7u32).wrapping_mul(x88))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t61 as *mut u32).add(0), uint32x4x2_t(t60[0], t60[2]));
            vst2q_u32((t61 as *mut u32).add(8), uint32x4x2_t(t60[1], t60[3]));
        } else {
            let mut t62w = 0u64;
            let t62 = core::hint::black_box(&raw mut t62w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t61 as *mut u64).add(0), t62).write_unaligned(((vgetq_lane_u32::<0>(t60[2]) as u64) << 32 | vgetq_lane_u32::<0>(t60[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t61 as *mut u64).add(1), t62).write_unaligned(((vgetq_lane_u32::<1>(t60[2]) as u64) << 32 | vgetq_lane_u32::<1>(t60[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t61 as *mut u64).add(2), t62).write_unaligned(((vgetq_lane_u32::<2>(t60[2]) as u64) << 32 | vgetq_lane_u32::<2>(t60[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t61 as *mut u64).add(3), t62).write_unaligned(((vgetq_lane_u32::<3>(t60[2]) as u64) << 32 | vgetq_lane_u32::<3>(t60[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t61 as *mut u64).add(4), t62).write_unaligned(((vgetq_lane_u32::<0>(t60[3]) as u64) << 32 | vgetq_lane_u32::<0>(t60[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t61 as *mut u64).add(5), t62).write_unaligned(((vgetq_lane_u32::<1>(t60[3]) as u64) << 32 | vgetq_lane_u32::<1>(t60[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t61 as *mut u64).add(6), t62).write_unaligned(((vgetq_lane_u32::<2>(t60[3]) as u64) << 32 | vgetq_lane_u32::<2>(t60[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t61 as *mut u64).add(7), t62).write_unaligned(((vgetq_lane_u32::<3>(t60[3]) as u64) << 32 | vgetq_lane_u32::<3>(t60[1]) as u64));
        }
        let t63 = vdupq_n_s32(((((40i64) as u32)) & 63) as i32);
        let t64: [uint32x4_t; 2] = [vcgeq_s32(t63, vdupq_n_s32(32)), vcgeq_s32(t63, vdupq_n_s32(32))];
        let t65: [uint32x4_t; 4] = [vbslq_u32(t64[0], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x56[2]), vsubq_s32(vdupq_n_s32(32), t63))), vorrq_u32(vshlq_u32(x56[0], vnegq_s32(t63)), vshlq_u32(x56[2], vsubq_s32(vdupq_n_s32(32), t63)))), vbslq_u32(t64[1], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x56[3]), vsubq_s32(vdupq_n_s32(32), t63))), vorrq_u32(vshlq_u32(x56[1], vnegq_s32(t63)), vshlq_u32(x56[3], vsubq_s32(vdupq_n_s32(32), t63)))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x56[2]), vnegq_s32(t63))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x56[3]), vnegq_s32(t63)))];
        let t66 = (arg3 as *const u8).add(((gid0.wrapping_add((8u32).wrapping_mul(x88))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t66 as *mut u32).add(0), uint32x4x2_t(t65[0], t65[2]));
            vst2q_u32((t66 as *mut u32).add(8), uint32x4x2_t(t65[1], t65[3]));
        } else {
            let mut t67w = 0u64;
            let t67 = core::hint::black_box(&raw mut t67w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t66 as *mut u64).add(0), t67).write_unaligned(((vgetq_lane_u32::<0>(t65[2]) as u64) << 32 | vgetq_lane_u32::<0>(t65[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t66 as *mut u64).add(1), t67).write_unaligned(((vgetq_lane_u32::<1>(t65[2]) as u64) << 32 | vgetq_lane_u32::<1>(t65[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t66 as *mut u64).add(2), t67).write_unaligned(((vgetq_lane_u32::<2>(t65[2]) as u64) << 32 | vgetq_lane_u32::<2>(t65[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t66 as *mut u64).add(3), t67).write_unaligned(((vgetq_lane_u32::<3>(t65[2]) as u64) << 32 | vgetq_lane_u32::<3>(t65[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t66 as *mut u64).add(4), t67).write_unaligned(((vgetq_lane_u32::<0>(t65[3]) as u64) << 32 | vgetq_lane_u32::<0>(t65[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t66 as *mut u64).add(5), t67).write_unaligned(((vgetq_lane_u32::<1>(t65[3]) as u64) << 32 | vgetq_lane_u32::<1>(t65[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t66 as *mut u64).add(6), t67).write_unaligned(((vgetq_lane_u32::<2>(t65[3]) as u64) << 32 | vgetq_lane_u32::<2>(t65[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t66 as *mut u64).add(7), t67).write_unaligned(((vgetq_lane_u32::<3>(t65[3]) as u64) << 32 | vgetq_lane_u32::<3>(t65[1]) as u64));
        }
        let t68: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x56[0]), vget_low_u32(x64[0]))), vreinterpretq_u32_u64(vmull_high_u32(x56[0], x64[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x56[1]), vget_low_u32(x64[1]))), vreinterpretq_u32_u64(vmull_high_u32(x56[1], x64[1])))];
        let t69: [uint32x4_t; 4] = [vmulq_u32(x56[0], x64[0]), vmulq_u32(x56[1], x64[1]), vmlaq_u32(vmlaq_u32(t68[0], x56[0], x64[2]), x64[0], x56[2]), vmlaq_u32(vmlaq_u32(t68[1], x56[1], x64[3]), x64[1], x56[3])];
        let t70 = (arg3 as *const u8).add(((gid0.wrapping_add((9u32).wrapping_mul(x88))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t70 as *mut u32).add(0), uint32x4x2_t(t69[0], t69[2]));
            vst2q_u32((t70 as *mut u32).add(8), uint32x4x2_t(t69[1], t69[3]));
        } else {
            let mut t71w = 0u64;
            let t71 = core::hint::black_box(&raw mut t71w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t70 as *mut u64).add(0), t71).write_unaligned(((vgetq_lane_u32::<0>(t69[2]) as u64) << 32 | vgetq_lane_u32::<0>(t69[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t70 as *mut u64).add(1), t71).write_unaligned(((vgetq_lane_u32::<1>(t69[2]) as u64) << 32 | vgetq_lane_u32::<1>(t69[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t70 as *mut u64).add(2), t71).write_unaligned(((vgetq_lane_u32::<2>(t69[2]) as u64) << 32 | vgetq_lane_u32::<2>(t69[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t70 as *mut u64).add(3), t71).write_unaligned(((vgetq_lane_u32::<3>(t69[2]) as u64) << 32 | vgetq_lane_u32::<3>(t69[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t70 as *mut u64).add(4), t71).write_unaligned(((vgetq_lane_u32::<0>(t69[3]) as u64) << 32 | vgetq_lane_u32::<0>(t69[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t70 as *mut u64).add(5), t71).write_unaligned(((vgetq_lane_u32::<1>(t69[3]) as u64) << 32 | vgetq_lane_u32::<1>(t69[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t70 as *mut u64).add(6), t71).write_unaligned(((vgetq_lane_u32::<2>(t69[3]) as u64) << 32 | vgetq_lane_u32::<2>(t69[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t70 as *mut u64).add(7), t71).write_unaligned(((vgetq_lane_u32::<3>(t69[3]) as u64) << 32 | vgetq_lane_u32::<3>(t69[1]) as u64));
        }
        let t72: u64 = 0u64;
        let t73 = [vdupq_n_u32((t72) as u32); 2];
        let t74: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x64[0], t73[0]), vceqq_u32(x64[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(x64[1], t73[1]), vceqq_u32(x64[3], vdupq_n_u32(0)))];
        let t75: [uint32x4_t; 2] = [vmvnq_u32(t74[0]), vmvnq_u32(t74[1])];
        let mut t76: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t77: [uint32x4_t; 2] = [vandq_u32(live[0], t75[0]), vandq_u32(live[1], t75[1])];
        let t78: [uint32x4_t; 2] = [vbicq_u32(live[0], t75[0]), vbicq_u32(live[1], t75[1])];
        {
            let (mut t79xl, mut t79xh, mut t79yl, mut t79yh) = ([0u32; 8], [0u32; 8], [0u32; 8], [0u32; 8]);
            vst1q_u32(t79xl.as_mut_ptr().add(0), x56[0]); vst1q_u32(t79xh.as_mut_ptr().add(0), x56[2]); vst1q_u32(t79yl.as_mut_ptr().add(0), x64[0]); vst1q_u32(t79yh.as_mut_ptr().add(0), x64[2]);
            vst1q_u32(t79xl.as_mut_ptr().add(4), x56[1]); vst1q_u32(t79xh.as_mut_ptr().add(4), x56[3]); vst1q_u32(t79yl.as_mut_ptr().add(4), x64[1]); vst1q_u32(t79yh.as_mut_ptr().add(4), x64[3]);
            for j in 0..8 { let q = (((t79xh[j] as u64) << 32 | t79xl[j] as u64) as u64).checked_div(((t79yh[j] as u64) << 32 | t79yl[j] as u64) as u64).unwrap_or(0) as u64; t79xl[j] = q as u32; t79xh[j] = (q >> 32) as u32; }
            let t80: [uint32x4_t; 4] = [vld1q_u32(t79xl.as_ptr().add(0)), vld1q_u32(t79xl.as_ptr().add(4)), vld1q_u32(t79xh.as_ptr().add(0)), vld1q_u32(t79xh.as_ptr().add(4))];
            t76 = [vbslq_u32(t77[0], t80[0], t76[0]), vbslq_u32(t77[1], t80[1], t76[1]), vbslq_u32(t77[0], t80[2], t76[2]), vbslq_u32(t77[1], t80[3], t76[3])];
        }
        {
            let t81: u64 = 7u64;
            let t82 = [vdupq_n_u32((t81) as u32); 2];
            let t83: [uint32x4_t; 4] = [t82[0], t82[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            t76 = [vbslq_u32(t78[0], t83[0], t76[0]), vbslq_u32(t78[1], t83[1], t76[1]), vbslq_u32(t78[0], t83[2], t76[2]), vbslq_u32(t78[1], t83[3], t76[3])];
        }
        let t84 = (arg3 as *const u8).add(((gid0.wrapping_add((10u32).wrapping_mul(x88))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t84 as *mut u32).add(0), uint32x4x2_t(t76[0], t76[2]));
            vst2q_u32((t84 as *mut u32).add(8), uint32x4x2_t(t76[1], t76[3]));
        } else {
            let mut t85w = 0u64;
            let t85 = core::hint::black_box(&raw mut t85w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t84 as *mut u64).add(0), t85).write_unaligned(((vgetq_lane_u32::<0>(t76[2]) as u64) << 32 | vgetq_lane_u32::<0>(t76[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t84 as *mut u64).add(1), t85).write_unaligned(((vgetq_lane_u32::<1>(t76[2]) as u64) << 32 | vgetq_lane_u32::<1>(t76[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t84 as *mut u64).add(2), t85).write_unaligned(((vgetq_lane_u32::<2>(t76[2]) as u64) << 32 | vgetq_lane_u32::<2>(t76[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t84 as *mut u64).add(3), t85).write_unaligned(((vgetq_lane_u32::<3>(t76[2]) as u64) << 32 | vgetq_lane_u32::<3>(t76[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t84 as *mut u64).add(4), t85).write_unaligned(((vgetq_lane_u32::<0>(t76[3]) as u64) << 32 | vgetq_lane_u32::<0>(t76[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t84 as *mut u64).add(5), t85).write_unaligned(((vgetq_lane_u32::<1>(t76[3]) as u64) << 32 | vgetq_lane_u32::<1>(t76[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t84 as *mut u64).add(6), t85).write_unaligned(((vgetq_lane_u32::<2>(t76[3]) as u64) << 32 | vgetq_lane_u32::<2>(t76[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t84 as *mut u64).add(7), t85).write_unaligned(((vgetq_lane_u32::<3>(t76[3]) as u64) << 32 | vgetq_lane_u32::<3>(t76[1]) as u64));
        }
        let t86: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x56[2], x64[2]), vandq_u32(vceqq_u32(x56[2], x64[2]), vcltq_u32(x56[0], x64[0]))), vorrq_u32(vcltq_u32(x56[3], x64[3]), vandq_u32(vceqq_u32(x56[3], x64[3]), vcltq_u32(x56[1], x64[1])))];
        let t87: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t86[0]), vshrq_n_u32::<31>(t86[1])];
        let t88: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x64[2], x56[2]), vandq_u32(vceqq_u32(x64[2], x56[2]), vcltq_u32(x64[0], x56[0]))), vorrq_u32(vcltq_u32(x64[3], x56[3]), vandq_u32(vceqq_u32(x64[3], x56[3]), vcltq_u32(x64[1], x56[1])))];
        let t89: [uint32x4_t; 2] = [vmvnq_u32(t88[0]), vmvnq_u32(t88[1])];
        let t90: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t89[0]), vshrq_n_u32::<31>(t89[1])];
        let t91: [uint32x4_t; 4] = [vshlq_n_u32::<1>(t90[0]), vshlq_n_u32::<1>(t90[1]), vshrq_n_u32::<31>(t90[0]), vshrq_n_u32::<31>(t90[1])];
        let t92: [uint32x4_t; 4] = [vorrq_u32(t87[0], t91[0]), vorrq_u32(t87[1], t91[1]), t91[2], t91[3]];
        let t93: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x56[0], x64[0]), vceqq_u32(x56[2], x64[2])), vandq_u32(vceqq_u32(x56[1], x64[1]), vceqq_u32(x56[3], x64[3]))];
        let t94: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t93[0]), vshrq_n_u32::<31>(t93[1])];
        let t95: [uint32x4_t; 4] = [vshlq_n_u32::<2>(t94[0]), vshlq_n_u32::<2>(t94[1]), vshrq_n_u32::<30>(t94[0]), vshrq_n_u32::<30>(t94[1])];
        let t96: [uint32x4_t; 4] = [vorrq_u32(t92[0], t95[0]), vorrq_u32(t92[1], t95[1]), vorrq_u32(t92[2], t95[2]), vorrq_u32(t92[3], t95[3])];
        let t97: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x56[0], x64[0]), vceqq_u32(x56[2], x64[2])), vandq_u32(vceqq_u32(x56[1], x64[1]), vceqq_u32(x56[3], x64[3]))];
        let t98: [uint32x4_t; 2] = [vmvnq_u32(t97[0]), vmvnq_u32(t97[1])];
        let t99: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t98[0]), vshrq_n_u32::<31>(t98[1])];
        let t100: [uint32x4_t; 4] = [vshlq_n_u32::<3>(t99[0]), vshlq_n_u32::<3>(t99[1]), vshrq_n_u32::<29>(t99[0]), vshrq_n_u32::<29>(t99[1])];
        let t101: [uint32x4_t; 4] = [vorrq_u32(t96[0], t100[0]), vorrq_u32(t96[1], t100[1]), vorrq_u32(t96[2], t100[2]), vorrq_u32(t96[3], t100[3])];
        let t102: [uint32x4_t; 2] = [vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x56[2]), vreinterpretq_s32_u32(x64[2])), vandq_u32(vceqq_u32(x56[2], x64[2]), vcltq_u32(x56[0], x64[0]))), vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x56[3]), vreinterpretq_s32_u32(x64[3])), vandq_u32(vceqq_u32(x56[3], x64[3]), vcltq_u32(x56[1], x64[1])))];
        let t103: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t102[0]), vshrq_n_u32::<31>(t102[1])];
        let t104: [uint32x4_t; 4] = [vshlq_n_u32::<4>(t103[0]), vshlq_n_u32::<4>(t103[1]), vshrq_n_u32::<28>(t103[0]), vshrq_n_u32::<28>(t103[1])];
        let t105: [uint32x4_t; 4] = [vorrq_u32(t101[0], t104[0]), vorrq_u32(t101[1], t104[1]), vorrq_u32(t101[2], t104[2]), vorrq_u32(t101[3], t104[3])];
        let t106: [uint32x4_t; 2] = [vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x56[2]), vreinterpretq_s32_u32(x64[2])), vandq_u32(vceqq_u32(x56[2], x64[2]), vcltq_u32(x56[0], x64[0]))), vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x56[3]), vreinterpretq_s32_u32(x64[3])), vandq_u32(vceqq_u32(x56[3], x64[3]), vcltq_u32(x56[1], x64[1])))];
        let t107: [uint32x4_t; 2] = [vmvnq_u32(t106[0]), vmvnq_u32(t106[1])];
        let t108: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t107[0]), vshrq_n_u32::<31>(t107[1])];
        let t109: [uint32x4_t; 4] = [vshlq_n_u32::<5>(t108[0]), vshlq_n_u32::<5>(t108[1]), vshrq_n_u32::<27>(t108[0]), vshrq_n_u32::<27>(t108[1])];
        let t110: [uint32x4_t; 4] = [vorrq_u32(t105[0], t109[0]), vorrq_u32(t105[1], t109[1]), vorrq_u32(t105[2], t109[2]), vorrq_u32(t105[3], t109[3])];
        let t111: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x64[2], x56[2]), vandq_u32(vceqq_u32(x64[2], x56[2]), vcltq_u32(x64[0], x56[0]))), vorrq_u32(vcltq_u32(x64[3], x56[3]), vandq_u32(vceqq_u32(x64[3], x56[3]), vcltq_u32(x64[1], x56[1])))];
        let t112: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t111[0]), vshrq_n_u32::<31>(t111[1])];
        let t113: [uint32x4_t; 4] = [vshlq_n_u32::<6>(t112[0]), vshlq_n_u32::<6>(t112[1]), vshrq_n_u32::<26>(t112[0]), vshrq_n_u32::<26>(t112[1])];
        let t114: [uint32x4_t; 4] = [vorrq_u32(t110[0], t113[0]), vorrq_u32(t110[1], t113[1]), vorrq_u32(t110[2], t113[2]), vorrq_u32(t110[3], t113[3])];
        let t115 = (arg3 as *const u8).add(((gid0.wrapping_add((11u32).wrapping_mul(x88))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t115 as *mut u32).add(0), uint32x4x2_t(t114[0], t114[2]));
            vst2q_u32((t115 as *mut u32).add(8), uint32x4x2_t(t114[1], t114[3]));
        } else {
            let mut t116w = 0u64;
            let t116 = core::hint::black_box(&raw mut t116w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t115 as *mut u64).add(0), t116).write_unaligned(((vgetq_lane_u32::<0>(t114[2]) as u64) << 32 | vgetq_lane_u32::<0>(t114[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t115 as *mut u64).add(1), t116).write_unaligned(((vgetq_lane_u32::<1>(t114[2]) as u64) << 32 | vgetq_lane_u32::<1>(t114[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t115 as *mut u64).add(2), t116).write_unaligned(((vgetq_lane_u32::<2>(t114[2]) as u64) << 32 | vgetq_lane_u32::<2>(t114[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t115 as *mut u64).add(3), t116).write_unaligned(((vgetq_lane_u32::<3>(t114[2]) as u64) << 32 | vgetq_lane_u32::<3>(t114[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t115 as *mut u64).add(4), t116).write_unaligned(((vgetq_lane_u32::<0>(t114[3]) as u64) << 32 | vgetq_lane_u32::<0>(t114[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t115 as *mut u64).add(5), t116).write_unaligned(((vgetq_lane_u32::<1>(t114[3]) as u64) << 32 | vgetq_lane_u32::<1>(t114[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t115 as *mut u64).add(6), t116).write_unaligned(((vgetq_lane_u32::<2>(t114[3]) as u64) << 32 | vgetq_lane_u32::<2>(t114[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t115 as *mut u64).add(7), t116).write_unaligned(((vgetq_lane_u32::<3>(t114[3]) as u64) << 32 | vgetq_lane_u32::<3>(t114[1]) as u64));
        }
        let t117: [uint32x4_t; 2] = [vaddq_u32(x72[0], x56[0]), vaddq_u32(x72[1], x56[1])];
        let t118: [uint32x4_t; 2] = [vcltq_u32(t117[0], x72[0]), vcltq_u32(t117[1], x72[1])];
        let t119: [uint32x4_t; 4] = [t117[0], t117[1], vsubq_u32(vaddq_u32(vdupq_n_u32(0), x56[2]), t118[0]), vsubq_u32(vaddq_u32(vdupq_n_u32(0), x56[3]), t118[1])];
        let t120 = (arg3 as *const u8).add(((gid0.wrapping_add((12u32).wrapping_mul(x88))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t120 as *mut u32).add(0), uint32x4x2_t(t119[0], t119[2]));
            vst2q_u32((t120 as *mut u32).add(8), uint32x4x2_t(t119[1], t119[3]));
        } else {
            let mut t121w = 0u64;
            let t121 = core::hint::black_box(&raw mut t121w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t120 as *mut u64).add(0), t121).write_unaligned(((vgetq_lane_u32::<0>(t119[2]) as u64) << 32 | vgetq_lane_u32::<0>(t119[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t120 as *mut u64).add(1), t121).write_unaligned(((vgetq_lane_u32::<1>(t119[2]) as u64) << 32 | vgetq_lane_u32::<1>(t119[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t120 as *mut u64).add(2), t121).write_unaligned(((vgetq_lane_u32::<2>(t119[2]) as u64) << 32 | vgetq_lane_u32::<2>(t119[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t120 as *mut u64).add(3), t121).write_unaligned(((vgetq_lane_u32::<3>(t119[2]) as u64) << 32 | vgetq_lane_u32::<3>(t119[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t120 as *mut u64).add(4), t121).write_unaligned(((vgetq_lane_u32::<0>(t119[3]) as u64) << 32 | vgetq_lane_u32::<0>(t119[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t120 as *mut u64).add(5), t121).write_unaligned(((vgetq_lane_u32::<1>(t119[3]) as u64) << 32 | vgetq_lane_u32::<1>(t119[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t120 as *mut u64).add(6), t121).write_unaligned(((vgetq_lane_u32::<2>(t119[3]) as u64) << 32 | vgetq_lane_u32::<2>(t119[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t120 as *mut u64).add(7), t121).write_unaligned(((vgetq_lane_u32::<3>(t119[3]) as u64) << 32 | vgetq_lane_u32::<3>(t119[1]) as u64));
        }
        let t122: [uint32x4_t; 2] = [vreinterpretq_u32_s32(vshrq_n_s32::<31>(vreinterpretq_s32_u32(x72[0]))), vreinterpretq_u32_s32(vshrq_n_s32::<31>(vreinterpretq_s32_u32(x72[1])))];
        let t123: [uint32x4_t; 4] = [x72[0], x72[1], t122[0], t122[1]];
        let t124 = (arg3 as *const u8).add(((gid0.wrapping_add((13u32).wrapping_mul(x88))) as usize) * 8);
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
        let t126: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<3>(x56[0]), vshlq_n_u32::<29>(x56[2])), vorrq_u32(vshrq_n_u32::<3>(x56[1]), vshlq_n_u32::<29>(x56[3])), vshrq_n_u32::<3>(x56[2]), vshrq_n_u32::<3>(x56[3])];
        let t127: [uint32x4_t; 2] = [t126[0], t126[1]];
        let t128: [uint32x4_t; 4] = [t127[0], t127[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        let t129 = (arg3 as *const u8).add(((gid0.wrapping_add((14u32).wrapping_mul(x88))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t129 as *mut u32).add(0), uint32x4x2_t(t128[0], t128[2]));
            vst2q_u32((t129 as *mut u32).add(8), uint32x4x2_t(t128[1], t128[3]));
        } else {
            let mut t130w = 0u64;
            let t130 = core::hint::black_box(&raw mut t130w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t129 as *mut u64).add(0), t130).write_unaligned(((vgetq_lane_u32::<0>(t128[2]) as u64) << 32 | vgetq_lane_u32::<0>(t128[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t129 as *mut u64).add(1), t130).write_unaligned(((vgetq_lane_u32::<1>(t128[2]) as u64) << 32 | vgetq_lane_u32::<1>(t128[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t129 as *mut u64).add(2), t130).write_unaligned(((vgetq_lane_u32::<2>(t128[2]) as u64) << 32 | vgetq_lane_u32::<2>(t128[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t129 as *mut u64).add(3), t130).write_unaligned(((vgetq_lane_u32::<3>(t128[2]) as u64) << 32 | vgetq_lane_u32::<3>(t128[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t129 as *mut u64).add(4), t130).write_unaligned(((vgetq_lane_u32::<0>(t128[3]) as u64) << 32 | vgetq_lane_u32::<0>(t128[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t129 as *mut u64).add(5), t130).write_unaligned(((vgetq_lane_u32::<1>(t128[3]) as u64) << 32 | vgetq_lane_u32::<1>(t128[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t129 as *mut u64).add(6), t130).write_unaligned(((vgetq_lane_u32::<2>(t128[3]) as u64) << 32 | vgetq_lane_u32::<2>(t128[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t129 as *mut u64).add(7), t130).write_unaligned(((vgetq_lane_u32::<3>(t128[3]) as u64) << 32 | vgetq_lane_u32::<3>(t128[1]) as u64));
        }
        let t131: u64 = 1u64;
        let t132 = [vdupq_n_u32((t131) as u32); 2];
        let t133: [uint32x4_t; 2] = [vandq_u32(x56[0], t132[0]), vandq_u32(x56[1], t132[1])];
        let t134: [uint32x4_t; 2] = [vtstq_u32(t133[0], t133[0]), vtstq_u32(t133[1], t133[1])];
        let t135: [uint32x4_t; 2] = [vandq_u32(live[0], t134[0]), vandq_u32(live[1], t134[1])];
        {
            let t136: [uint32x4_t; 4] = [veorq_u32(x56[0], x64[0]), veorq_u32(x56[1], x64[1]), veorq_u32(x56[2], x64[2]), veorq_u32(x56[3], x64[3])];
            x104 = [vbslq_u32(t135[0], t136[0], x104[0]), vbslq_u32(t135[1], t136[1], x104[1]), vbslq_u32(t135[0], t136[2], x104[2]), vbslq_u32(t135[1], t136[3], x104[3])];
        }
        let t137: [uint32x4_t; 2] = [vbicq_u32(live[0], t134[0]), vbicq_u32(live[1], t134[1])];
        {
            let t138: [uint32x4_t; 2] = [vaddq_u32(x56[0], x72[0]), vaddq_u32(x56[1], x72[1])];
            let t139: [uint32x4_t; 2] = [vcltq_u32(t138[0], x56[0]), vcltq_u32(t138[1], x56[1])];
            let t140: [uint32x4_t; 4] = [t138[0], t138[1], vsubq_u32(vaddq_u32(x56[2], vdupq_n_u32(0)), t139[0]), vsubq_u32(vaddq_u32(x56[3], vdupq_n_u32(0)), t139[1])];
            x104 = [vbslq_u32(t137[0], t140[0], x104[0]), vbslq_u32(t137[1], t140[1], x104[1]), vbslq_u32(t137[0], t140[2], x104[2]), vbslq_u32(t137[1], t140[3], x104[3])];
        }
        let t141 = (arg3 as *const u8).add(((gid0.wrapping_add((15u32).wrapping_mul(x88))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t141 as *mut u32).add(0), uint32x4x2_t(x104[0], x104[2]));
            vst2q_u32((t141 as *mut u32).add(8), uint32x4x2_t(x104[1], x104[3]));
        } else {
            let mut t142w = 0u64;
            let t142 = core::hint::black_box(&raw mut t142w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t141 as *mut u64).add(0), t142).write_unaligned(((vgetq_lane_u32::<0>(x104[2]) as u64) << 32 | vgetq_lane_u32::<0>(x104[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t141 as *mut u64).add(1), t142).write_unaligned(((vgetq_lane_u32::<1>(x104[2]) as u64) << 32 | vgetq_lane_u32::<1>(x104[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t141 as *mut u64).add(2), t142).write_unaligned(((vgetq_lane_u32::<2>(x104[2]) as u64) << 32 | vgetq_lane_u32::<2>(x104[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t141 as *mut u64).add(3), t142).write_unaligned(((vgetq_lane_u32::<3>(x104[2]) as u64) << 32 | vgetq_lane_u32::<3>(x104[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t141 as *mut u64).add(4), t142).write_unaligned(((vgetq_lane_u32::<0>(x104[3]) as u64) << 32 | vgetq_lane_u32::<0>(x104[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t141 as *mut u64).add(5), t142).write_unaligned(((vgetq_lane_u32::<1>(x104[3]) as u64) << 32 | vgetq_lane_u32::<1>(x104[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t141 as *mut u64).add(6), t142).write_unaligned(((vgetq_lane_u32::<2>(x104[3]) as u64) << 32 | vgetq_lane_u32::<2>(x104[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t141 as *mut u64).add(7), t142).write_unaligned(((vgetq_lane_u32::<3>(x104[3]) as u64) << 32 | vgetq_lane_u32::<3>(x104[1]) as u64));
        }
        let t143 = b1_l_p(l, lid0, live, x56);
        let t144 = b2_l_p(l, lid0, live, x64);
        let t145: [uint32x4_t; 4] = [vshlq_n_u32::<8>(t144[0]), vshlq_n_u32::<8>(t144[1]), vshrq_n_u32::<24>(t144[0]), vshrq_n_u32::<24>(t144[1])];
        let t146: [uint32x4_t; 2] = [vaddq_u32(t143[0], t145[0]), vaddq_u32(t143[1], t145[1])];
        let t147: [uint32x4_t; 2] = [vcltq_u32(t146[0], t143[0]), vcltq_u32(t146[1], t143[1])];
        let t148: [uint32x4_t; 4] = [t146[0], t146[1], vsubq_u32(vaddq_u32(vdupq_n_u32(0), t145[2]), t147[0]), vsubq_u32(vaddq_u32(vdupq_n_u32(0), t145[3]), t147[1])];
        let t149 = (arg3 as *const u8).add(((gid0.wrapping_add((16u32).wrapping_mul(x88))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t149 as *mut u32).add(0), uint32x4x2_t(t148[0], t148[2]));
            vst2q_u32((t149 as *mut u32).add(8), uint32x4x2_t(t148[1], t148[3]));
        } else {
            let mut t150w = 0u64;
            let t150 = core::hint::black_box(&raw mut t150w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t149 as *mut u64).add(0), t150).write_unaligned(((vgetq_lane_u32::<0>(t148[2]) as u64) << 32 | vgetq_lane_u32::<0>(t148[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t149 as *mut u64).add(1), t150).write_unaligned(((vgetq_lane_u32::<1>(t148[2]) as u64) << 32 | vgetq_lane_u32::<1>(t148[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t149 as *mut u64).add(2), t150).write_unaligned(((vgetq_lane_u32::<2>(t148[2]) as u64) << 32 | vgetq_lane_u32::<2>(t148[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t149 as *mut u64).add(3), t150).write_unaligned(((vgetq_lane_u32::<3>(t148[2]) as u64) << 32 | vgetq_lane_u32::<3>(t148[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t149 as *mut u64).add(4), t150).write_unaligned(((vgetq_lane_u32::<0>(t148[3]) as u64) << 32 | vgetq_lane_u32::<0>(t148[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t149 as *mut u64).add(5), t150).write_unaligned(((vgetq_lane_u32::<1>(t148[3]) as u64) << 32 | vgetq_lane_u32::<1>(t148[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t149 as *mut u64).add(6), t150).write_unaligned(((vgetq_lane_u32::<2>(t148[3]) as u64) << 32 | vgetq_lane_u32::<2>(t148[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t149 as *mut u64).add(7), t150).write_unaligned(((vgetq_lane_u32::<3>(t148[3]) as u64) << 32 | vgetq_lane_u32::<3>(t148[1]) as u64));
        }
        let t151 = [vdupq_n_u32(x88); 2];
        let mut t152x = [0u32; 8];
        let mut t152y = [0u32; 8];
        for r in 0..2 { vst1q_u32(t152x.as_mut_ptr().add(4 * r), x72[r]); vst1q_u32(t152y.as_mut_ptr().add(4 * r), t151[r]); }
        for j in 0..8 { t152x[j] = (t152x[j] as u32).checked_rem(t152y[j] as u32).unwrap_or(0) as u32; }
        let t153: [uint32x4_t; 2] = [vld1q_u32(t152x.as_ptr().add(0)), vld1q_u32(t152x.as_ptr().add(4))];
        let t154 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t153[0]), vandq_u32(live[1], t153[1]))));
        let t155: [uint32x4_t; 2] = [vbslq_u32(live[0], t153[0], t154), vbslq_u32(live[1], t153[1], t154)];
        let mut t156: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t155[0]) as usize * 8) as *const u64).read_unaligned(); t156[0] = vsetq_lane_u32::<0>(v as u32, t156[0]); t156[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t156[2]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t155[0]) as usize * 8) as *const u64).read_unaligned(); t156[0] = vsetq_lane_u32::<1>(v as u32, t156[0]); t156[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t156[2]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t155[0]) as usize * 8) as *const u64).read_unaligned(); t156[0] = vsetq_lane_u32::<2>(v as u32, t156[0]); t156[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t156[2]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t155[0]) as usize * 8) as *const u64).read_unaligned(); t156[0] = vsetq_lane_u32::<3>(v as u32, t156[0]); t156[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t156[2]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t155[1]) as usize * 8) as *const u64).read_unaligned(); t156[1] = vsetq_lane_u32::<0>(v as u32, t156[1]); t156[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t156[3]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t155[1]) as usize * 8) as *const u64).read_unaligned(); t156[1] = vsetq_lane_u32::<1>(v as u32, t156[1]); t156[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t156[3]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t155[1]) as usize * 8) as *const u64).read_unaligned(); t156[1] = vsetq_lane_u32::<2>(v as u32, t156[1]); t156[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t156[3]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t155[1]) as usize * 8) as *const u64).read_unaligned(); t156[1] = vsetq_lane_u32::<3>(v as u32, t156[1]); t156[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t156[3]); }
        let t157 = (arg3 as *const u8).add(((gid0.wrapping_add((17u32).wrapping_mul(x88))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t157 as *mut u32).add(0), uint32x4x2_t(t156[0], t156[2]));
            vst2q_u32((t157 as *mut u32).add(8), uint32x4x2_t(t156[1], t156[3]));
        } else {
            let mut t158w = 0u64;
            let t158 = core::hint::black_box(&raw mut t158w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t157 as *mut u64).add(0), t158).write_unaligned(((vgetq_lane_u32::<0>(t156[2]) as u64) << 32 | vgetq_lane_u32::<0>(t156[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t157 as *mut u64).add(1), t158).write_unaligned(((vgetq_lane_u32::<1>(t156[2]) as u64) << 32 | vgetq_lane_u32::<1>(t156[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t157 as *mut u64).add(2), t158).write_unaligned(((vgetq_lane_u32::<2>(t156[2]) as u64) << 32 | vgetq_lane_u32::<2>(t156[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t157 as *mut u64).add(3), t158).write_unaligned(((vgetq_lane_u32::<3>(t156[2]) as u64) << 32 | vgetq_lane_u32::<3>(t156[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t157 as *mut u64).add(4), t158).write_unaligned(((vgetq_lane_u32::<0>(t156[3]) as u64) << 32 | vgetq_lane_u32::<0>(t156[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t157 as *mut u64).add(5), t158).write_unaligned(((vgetq_lane_u32::<1>(t156[3]) as u64) << 32 | vgetq_lane_u32::<1>(t156[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t157 as *mut u64).add(6), t158).write_unaligned(((vgetq_lane_u32::<2>(t156[3]) as u64) << 32 | vgetq_lane_u32::<2>(t156[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t157 as *mut u64).add(7), t158).write_unaligned(((vgetq_lane_u32::<3>(t156[3]) as u64) << 32 | vgetq_lane_u32::<3>(t156[1]) as u64));
        }
        let t159 = b3_ll_p(l, lid0, live, x56, x72);
        let t160 = (arg3 as *const u8).add(((gid0.wrapping_add((18u32).wrapping_mul(x88))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t160 as *mut u32).add(0), uint32x4x2_t(t159[0], t159[2]));
            vst2q_u32((t160 as *mut u32).add(8), uint32x4x2_t(t159[1], t159[3]));
        } else {
            let mut t161w = 0u64;
            let t161 = core::hint::black_box(&raw mut t161w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t160 as *mut u64).add(0), t161).write_unaligned(((vgetq_lane_u32::<0>(t159[2]) as u64) << 32 | vgetq_lane_u32::<0>(t159[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t160 as *mut u64).add(1), t161).write_unaligned(((vgetq_lane_u32::<1>(t159[2]) as u64) << 32 | vgetq_lane_u32::<1>(t159[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t160 as *mut u64).add(2), t161).write_unaligned(((vgetq_lane_u32::<2>(t159[2]) as u64) << 32 | vgetq_lane_u32::<2>(t159[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t160 as *mut u64).add(3), t161).write_unaligned(((vgetq_lane_u32::<3>(t159[2]) as u64) << 32 | vgetq_lane_u32::<3>(t159[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t160 as *mut u64).add(4), t161).write_unaligned(((vgetq_lane_u32::<0>(t159[3]) as u64) << 32 | vgetq_lane_u32::<0>(t159[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t160 as *mut u64).add(5), t161).write_unaligned(((vgetq_lane_u32::<1>(t159[3]) as u64) << 32 | vgetq_lane_u32::<1>(t159[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t160 as *mut u64).add(6), t161).write_unaligned(((vgetq_lane_u32::<2>(t159[3]) as u64) << 32 | vgetq_lane_u32::<2>(t159[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t160 as *mut u64).add(7), t161).write_unaligned(((vgetq_lane_u32::<3>(t159[3]) as u64) << 32 | vgetq_lane_u32::<3>(t159[1]) as u64));
        }
        x112 = x56;
        x120 = 0u32;
        let mut t166 = live;
        loop {
            // Lanes part: each its own rounds.
            let t168 = [vdupq_n_u32(7u32); 2];
            let t169: [uint32x4_t; 2] = [vandq_u32(x72[0], t168[0]), vandq_u32(x72[1], t168[1])];
            let t170 = [vdupq_n_u32(x120); 2];
            let t171: [uint32x4_t; 2] = [vcltq_u32(t170[0], t169[0]), vcltq_u32(t170[1], t169[1])];
            t166 = [vandq_u32(t166[0], t171[0]), vandq_u32(t166[1], t171[1])];
            if !(any(&t166)) { break; }
            let t173: u64 = 3u64;
            let t174 = [vdupq_n_u32((t173) as u32); 2];
            let t175: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x112[0]), vget_low_u32(t174[0]))), vreinterpretq_u32_u64(vmull_high_u32(x112[0], t174[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x112[1]), vget_low_u32(t174[1]))), vreinterpretq_u32_u64(vmull_high_u32(x112[1], t174[1])))];
            let t176: [uint32x4_t; 4] = [vmulq_u32(x112[0], t174[0]), vmulq_u32(x112[1], t174[1]), vmlaq_u32(t175[0], t174[0], x112[2]), vmlaq_u32(t175[1], t174[1], x112[3])];
            let t177: [uint32x4_t; 2] = [vaddq_u32(t176[0], x64[0]), vaddq_u32(t176[1], x64[1])];
            let t178: [uint32x4_t; 2] = [vcltq_u32(t177[0], t176[0]), vcltq_u32(t177[1], t176[1])];
            let t179: [uint32x4_t; 4] = [t177[0], t177[1], vsubq_u32(vaddq_u32(t176[2], x64[2]), t178[0]), vsubq_u32(vaddq_u32(t176[3], x64[3]), t178[1])];
            x112 = [vbslq_u32(t166[0], t179[0], x112[0]), vbslq_u32(t166[1], t179[1], x112[1]), vbslq_u32(t166[0], t179[2], x112[2]), vbslq_u32(t166[1], t179[3], x112[3])];
            let t180: [uint32x4_t; 2] = [vandq_u32(t166[0], live[0]), vandq_u32(t166[1], live[1])];
            let t181 = x120;
            x120 = (x120).wrapping_add(1u32);
        }
        let t182 = (arg3 as *const u8).add(((gid0.wrapping_add((19u32).wrapping_mul(x88))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t182 as *mut u32).add(0), uint32x4x2_t(x112[0], x112[2]));
            vst2q_u32((t182 as *mut u32).add(8), uint32x4x2_t(x112[1], x112[3]));
        } else {
            let mut t183w = 0u64;
            let t183 = core::hint::black_box(&raw mut t183w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t182 as *mut u64).add(0), t183).write_unaligned(((vgetq_lane_u32::<0>(x112[2]) as u64) << 32 | vgetq_lane_u32::<0>(x112[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t182 as *mut u64).add(1), t183).write_unaligned(((vgetq_lane_u32::<1>(x112[2]) as u64) << 32 | vgetq_lane_u32::<1>(x112[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t182 as *mut u64).add(2), t183).write_unaligned(((vgetq_lane_u32::<2>(x112[2]) as u64) << 32 | vgetq_lane_u32::<2>(x112[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t182 as *mut u64).add(3), t183).write_unaligned(((vgetq_lane_u32::<3>(x112[2]) as u64) << 32 | vgetq_lane_u32::<3>(x112[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t182 as *mut u64).add(4), t183).write_unaligned(((vgetq_lane_u32::<0>(x112[3]) as u64) << 32 | vgetq_lane_u32::<0>(x112[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t182 as *mut u64).add(5), t183).write_unaligned(((vgetq_lane_u32::<1>(x112[3]) as u64) << 32 | vgetq_lane_u32::<1>(x112[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t182 as *mut u64).add(6), t183).write_unaligned(((vgetq_lane_u32::<2>(x112[3]) as u64) << 32 | vgetq_lane_u32::<2>(x112[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t182 as *mut u64).add(7), t183).write_unaligned(((vgetq_lane_u32::<3>(x112[3]) as u64) << 32 | vgetq_lane_u32::<3>(x112[1]) as u64));
        }
        let t184: u64 = a1;
        let t185 = [vdupq_n_u32((t184) as u32); 2];
        let t186 = [vdupq_n_u32(((t184) >> 32) as u32); 2];
        let t187: [uint32x4_t; 2] = [vaddq_u32(x56[0], t185[0]), vaddq_u32(x56[1], t185[1])];
        let t188: [uint32x4_t; 2] = [vcltq_u32(t187[0], x56[0]), vcltq_u32(t187[1], x56[1])];
        let t189: [uint32x4_t; 4] = [t187[0], t187[1], vsubq_u32(vaddq_u32(x56[2], t186[0]), t188[0]), vsubq_u32(vaddq_u32(x56[3], t186[1]), t188[1])];
        let t190: u64 = (a1).wrapping_shl((3u64) as u32);
        let t191 = [vdupq_n_u32((t190) as u32); 2];
        let t192 = [vdupq_n_u32(((t190) >> 32) as u32); 2];
        let t193: [uint32x4_t; 2] = [vaddq_u32(t189[0], t191[0]), vaddq_u32(t189[1], t191[1])];
        let t194: [uint32x4_t; 2] = [vcltq_u32(t193[0], t189[0]), vcltq_u32(t193[1], t189[1])];
        let t195: [uint32x4_t; 4] = [t193[0], t193[1], vsubq_u32(vaddq_u32(t189[2], t192[0]), t194[0]), vsubq_u32(vaddq_u32(t189[3], t192[1]), t194[1])];
        let t196 = (arg3 as *const u8).add(((gid0.wrapping_add((20u32).wrapping_mul(x88))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t196 as *mut u32).add(0), uint32x4x2_t(t195[0], t195[2]));
            vst2q_u32((t196 as *mut u32).add(8), uint32x4x2_t(t195[1], t195[3]));
        } else {
            let mut t197w = 0u64;
            let t197 = core::hint::black_box(&raw mut t197w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t196 as *mut u64).add(0), t197).write_unaligned(((vgetq_lane_u32::<0>(t195[2]) as u64) << 32 | vgetq_lane_u32::<0>(t195[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t196 as *mut u64).add(1), t197).write_unaligned(((vgetq_lane_u32::<1>(t195[2]) as u64) << 32 | vgetq_lane_u32::<1>(t195[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t196 as *mut u64).add(2), t197).write_unaligned(((vgetq_lane_u32::<2>(t195[2]) as u64) << 32 | vgetq_lane_u32::<2>(t195[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t196 as *mut u64).add(3), t197).write_unaligned(((vgetq_lane_u32::<3>(t195[2]) as u64) << 32 | vgetq_lane_u32::<3>(t195[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t196 as *mut u64).add(4), t197).write_unaligned(((vgetq_lane_u32::<0>(t195[3]) as u64) << 32 | vgetq_lane_u32::<0>(t195[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t196 as *mut u64).add(5), t197).write_unaligned(((vgetq_lane_u32::<1>(t195[3]) as u64) << 32 | vgetq_lane_u32::<1>(t195[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t196 as *mut u64).add(6), t197).write_unaligned(((vgetq_lane_u32::<2>(t195[3]) as u64) << 32 | vgetq_lane_u32::<2>(t195[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t196 as *mut u64).add(7), t197).write_unaligned(((vgetq_lane_u32::<3>(t195[3]) as u64) << 32 | vgetq_lane_u32::<3>(t195[1]) as u64));
        }
        let t198: u64 = 4294967295u64;
        let t199 = [vdupq_n_u32((t198) as u32); 2];
        let t200: [uint32x4_t; 2] = [vandq_u32(x56[0], t199[0]), vandq_u32(x56[1], t199[1])];
        let t201: u64 = 4294967295u64;
        let t202 = [vdupq_n_u32((t201) as u32); 2];
        let t203: [uint32x4_t; 2] = [vandq_u32(x64[0], t202[0]), vandq_u32(x64[1], t202[1])];
        let t204: [uint32x4_t; 2] = [vceqq_u32(t200[0], t203[0]), vceqq_u32(t200[1], t203[1])];
        let mut t205: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t206: [uint32x4_t; 2] = [vandq_u32(live[0], t204[0]), vandq_u32(live[1], t204[1])];
        let t207: [uint32x4_t; 2] = [vbicq_u32(live[0], t204[0]), vbicq_u32(live[1], t204[1])];
        {
            let t208 = [vdupq_n_u32(1u32); 2];
            t205 = [vbslq_u32(t206[0], t208[0], t205[0]), vbslq_u32(t206[1], t208[1], t205[1])];
        }
        {
            let t209 = [vdupq_n_u32(2u32); 2];
            t205 = [vbslq_u32(t207[0], t209[0], t205[0]), vbslq_u32(t207[1], t209[1], t205[1])];
        }
        let t210: [uint32x4_t; 4] = [t205[0], t205[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        let t211 = (arg3 as *const u8).add(((gid0.wrapping_add((21u32).wrapping_mul(x88))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t211 as *mut u32).add(0), uint32x4x2_t(t210[0], t210[2]));
            vst2q_u32((t211 as *mut u32).add(8), uint32x4x2_t(t210[1], t210[3]));
        } else {
            let mut t212w = 0u64;
            let t212 = core::hint::black_box(&raw mut t212w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t211 as *mut u64).add(0), t212).write_unaligned(((vgetq_lane_u32::<0>(t210[2]) as u64) << 32 | vgetq_lane_u32::<0>(t210[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t211 as *mut u64).add(1), t212).write_unaligned(((vgetq_lane_u32::<1>(t210[2]) as u64) << 32 | vgetq_lane_u32::<1>(t210[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t211 as *mut u64).add(2), t212).write_unaligned(((vgetq_lane_u32::<2>(t210[2]) as u64) << 32 | vgetq_lane_u32::<2>(t210[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t211 as *mut u64).add(3), t212).write_unaligned(((vgetq_lane_u32::<3>(t210[2]) as u64) << 32 | vgetq_lane_u32::<3>(t210[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t211 as *mut u64).add(4), t212).write_unaligned(((vgetq_lane_u32::<0>(t210[3]) as u64) << 32 | vgetq_lane_u32::<0>(t210[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t211 as *mut u64).add(5), t212).write_unaligned(((vgetq_lane_u32::<1>(t210[3]) as u64) << 32 | vgetq_lane_u32::<1>(t210[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t211 as *mut u64).add(6), t212).write_unaligned(((vgetq_lane_u32::<2>(t210[3]) as u64) << 32 | vgetq_lane_u32::<2>(t210[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t211 as *mut u64).add(7), t212).write_unaligned(((vgetq_lane_u32::<3>(t210[3]) as u64) << 32 | vgetq_lane_u32::<3>(t210[1]) as u64));
        }
        let t213: [uint32x4_t; 2] = [vtstq_u32(vorrq_u32(x56[0], x56[2]), vorrq_u32(x56[0], x56[2])), vtstq_u32(vorrq_u32(x56[1], x56[3]), vorrq_u32(x56[1], x56[3]))];
        let mut t214: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t215: [uint32x4_t; 2] = [vandq_u32(live[0], t213[0]), vandq_u32(live[1], t213[1])];
        let t216: [uint32x4_t; 2] = [vbicq_u32(live[0], t213[0]), vbicq_u32(live[1], t213[1])];
        {
            let t217 = [vdupq_n_u32(5u32); 2];
            t214 = [vbslq_u32(t215[0], t217[0], t214[0]), vbslq_u32(t215[1], t217[1], t214[1])];
        }
        {
            let t218 = [vdupq_n_u32(6u32); 2];
            t214 = [vbslq_u32(t216[0], t218[0], t214[0]), vbslq_u32(t216[1], t218[1], t214[1])];
        }
        let t219: [uint32x4_t; 4] = [t214[0], t214[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        let t220 = (arg3 as *const u8).add(((gid0.wrapping_add((22u32).wrapping_mul(x88))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t220 as *mut u32).add(0), uint32x4x2_t(t219[0], t219[2]));
            vst2q_u32((t220 as *mut u32).add(8), uint32x4x2_t(t219[1], t219[3]));
        } else {
            let mut t221w = 0u64;
            let t221 = core::hint::black_box(&raw mut t221w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t220 as *mut u64).add(0), t221).write_unaligned(((vgetq_lane_u32::<0>(t219[2]) as u64) << 32 | vgetq_lane_u32::<0>(t219[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t220 as *mut u64).add(1), t221).write_unaligned(((vgetq_lane_u32::<1>(t219[2]) as u64) << 32 | vgetq_lane_u32::<1>(t219[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t220 as *mut u64).add(2), t221).write_unaligned(((vgetq_lane_u32::<2>(t219[2]) as u64) << 32 | vgetq_lane_u32::<2>(t219[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t220 as *mut u64).add(3), t221).write_unaligned(((vgetq_lane_u32::<3>(t219[2]) as u64) << 32 | vgetq_lane_u32::<3>(t219[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t220 as *mut u64).add(4), t221).write_unaligned(((vgetq_lane_u32::<0>(t219[3]) as u64) << 32 | vgetq_lane_u32::<0>(t219[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t220 as *mut u64).add(5), t221).write_unaligned(((vgetq_lane_u32::<1>(t219[3]) as u64) << 32 | vgetq_lane_u32::<1>(t219[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t220 as *mut u64).add(6), t221).write_unaligned(((vgetq_lane_u32::<2>(t219[3]) as u64) << 32 | vgetq_lane_u32::<2>(t219[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t220 as *mut u64).add(7), t221).write_unaligned(((vgetq_lane_u32::<3>(t219[3]) as u64) << 32 | vgetq_lane_u32::<3>(t219[1]) as u64));
        }
        let t222 = b4_ll_p(l, lid0, live, x56, x64);
        let t223 = (arg3 as *const u8).add(((gid0.wrapping_add((23u32).wrapping_mul(x88))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t223 as *mut u32).add(0), uint32x4x2_t(t222[0], t222[2]));
            vst2q_u32((t223 as *mut u32).add(8), uint32x4x2_t(t222[1], t222[3]));
        } else {
            let mut t224w = 0u64;
            let t224 = core::hint::black_box(&raw mut t224w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t223 as *mut u64).add(0), t224).write_unaligned(((vgetq_lane_u32::<0>(t222[2]) as u64) << 32 | vgetq_lane_u32::<0>(t222[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t223 as *mut u64).add(1), t224).write_unaligned(((vgetq_lane_u32::<1>(t222[2]) as u64) << 32 | vgetq_lane_u32::<1>(t222[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t223 as *mut u64).add(2), t224).write_unaligned(((vgetq_lane_u32::<2>(t222[2]) as u64) << 32 | vgetq_lane_u32::<2>(t222[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t223 as *mut u64).add(3), t224).write_unaligned(((vgetq_lane_u32::<3>(t222[2]) as u64) << 32 | vgetq_lane_u32::<3>(t222[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t223 as *mut u64).add(4), t224).write_unaligned(((vgetq_lane_u32::<0>(t222[3]) as u64) << 32 | vgetq_lane_u32::<0>(t222[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t223 as *mut u64).add(5), t224).write_unaligned(((vgetq_lane_u32::<1>(t222[3]) as u64) << 32 | vgetq_lane_u32::<1>(t222[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t223 as *mut u64).add(6), t224).write_unaligned(((vgetq_lane_u32::<2>(t222[3]) as u64) << 32 | vgetq_lane_u32::<2>(t222[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t223 as *mut u64).add(7), t224).write_unaligned(((vgetq_lane_u32::<3>(t222[3]) as u64) << 32 | vgetq_lane_u32::<3>(t222[1]) as u64));
        }
        let t225 = b5_r0ns_p(l, lid0, live, arg0, gid0.wrapping_mul(x96), x96, 8u32);
        let t226 = (arg3 as *const u8).add(((gid0.wrapping_add((24u32).wrapping_mul(x88))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t226 as *mut u32).add(0), uint32x4x2_t(t225[0], t225[2]));
            vst2q_u32((t226 as *mut u32).add(8), uint32x4x2_t(t225[1], t225[3]));
        } else {
            let mut t227w = 0u64;
            let t227 = core::hint::black_box(&raw mut t227w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t226 as *mut u64).add(0), t227).write_unaligned(((vgetq_lane_u32::<0>(t225[2]) as u64) << 32 | vgetq_lane_u32::<0>(t225[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t226 as *mut u64).add(1), t227).write_unaligned(((vgetq_lane_u32::<1>(t225[2]) as u64) << 32 | vgetq_lane_u32::<1>(t225[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t226 as *mut u64).add(2), t227).write_unaligned(((vgetq_lane_u32::<2>(t225[2]) as u64) << 32 | vgetq_lane_u32::<2>(t225[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t226 as *mut u64).add(3), t227).write_unaligned(((vgetq_lane_u32::<3>(t225[2]) as u64) << 32 | vgetq_lane_u32::<3>(t225[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t226 as *mut u64).add(4), t227).write_unaligned(((vgetq_lane_u32::<0>(t225[3]) as u64) << 32 | vgetq_lane_u32::<0>(t225[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t226 as *mut u64).add(5), t227).write_unaligned(((vgetq_lane_u32::<1>(t225[3]) as u64) << 32 | vgetq_lane_u32::<1>(t225[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t226 as *mut u64).add(6), t227).write_unaligned(((vgetq_lane_u32::<2>(t225[3]) as u64) << 32 | vgetq_lane_u32::<2>(t225[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t226 as *mut u64).add(7), t227).write_unaligned(((vgetq_lane_u32::<3>(t225[3]) as u64) << 32 | vgetq_lane_u32::<3>(t225[1]) as u64));
        }
        let t228 = if (gid0.wrapping_add(3u32) & 255u32) <= 248u32 { b6_u8_p(l, lid0, live, gid0.wrapping_add(3u32)) } else { b6_i_p(l, lid0, live, gid0.wrapping_add(3u32)) };
        let t229 = (arg3 as *const u8).add(((gid0.wrapping_add((25u32).wrapping_mul(x88))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t229 as *mut u32).add(0), uint32x4x2_t(t228[0], t228[2]));
            vst2q_u32((t229 as *mut u32).add(8), uint32x4x2_t(t228[1], t228[3]));
        } else {
            let mut t230w = 0u64;
            let t230 = core::hint::black_box(&raw mut t230w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t229 as *mut u64).add(0), t230).write_unaligned(((vgetq_lane_u32::<0>(t228[2]) as u64) << 32 | vgetq_lane_u32::<0>(t228[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t229 as *mut u64).add(1), t230).write_unaligned(((vgetq_lane_u32::<1>(t228[2]) as u64) << 32 | vgetq_lane_u32::<1>(t228[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t229 as *mut u64).add(2), t230).write_unaligned(((vgetq_lane_u32::<2>(t228[2]) as u64) << 32 | vgetq_lane_u32::<2>(t228[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t229 as *mut u64).add(3), t230).write_unaligned(((vgetq_lane_u32::<3>(t228[2]) as u64) << 32 | vgetq_lane_u32::<3>(t228[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t229 as *mut u64).add(4), t230).write_unaligned(((vgetq_lane_u32::<0>(t228[3]) as u64) << 32 | vgetq_lane_u32::<0>(t228[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t229 as *mut u64).add(5), t230).write_unaligned(((vgetq_lane_u32::<1>(t228[3]) as u64) << 32 | vgetq_lane_u32::<1>(t228[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t229 as *mut u64).add(6), t230).write_unaligned(((vgetq_lane_u32::<2>(t228[3]) as u64) << 32 | vgetq_lane_u32::<2>(t228[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t229 as *mut u64).add(7), t230).write_unaligned(((vgetq_lane_u32::<3>(t228[3]) as u64) << 32 | vgetq_lane_u32::<3>(t228[1]) as u64));
        }
        let t231: u64 = 3u64;
        let t232 = [vdupq_n_u32((t231) as u32); 2];
        let t233: [uint32x4_t; 2] = [vandq_u32(x56[0], t232[0]), vandq_u32(x56[1], t232[1])];
        let t234: u64 = 0u64;
        let t235 = [vdupq_n_u32((t234) as u32); 2];
        let t236: [uint32x4_t; 2] = [vceqq_u32(t233[0], t235[0]), vceqq_u32(t233[1], t235[1])];
        let mut t237: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t238: [uint32x4_t; 2] = [vandq_u32(live[0], t236[0]), vandq_u32(live[1], t236[1])];
        let t239: [uint32x4_t; 2] = [vbicq_u32(live[0], t236[0]), vbicq_u32(live[1], t236[1])];
        if any(&t238) {
            let t240: [uint32x4_t; 4];
            let t241 = bits(&t238);
            if t241.count_ones() <= 2 {
                let t242 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x64);
                let t243 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x72);
                let mut t244 = [0u32; 16];
                let mut t245 = t241;
                while t245 != 0 {
                    let j = t245.trailing_zeros() as usize;
                    t245 &= t245 - 1;
                    let t246 = b7_ss_f(l, lid0, ((t242[8 + j] as u64) << 32 | t242[j] as u64), t243[j]);
                    let t247: u64 = t246;
                    t244[j] = t247 as u32;
                    t244[8 + j] = (t247 >> 32) as u32;
                }
                t240 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t244);
            } else {
                let t248 = b7_ll_p(l, lid0, t238, x64, x72);
                t240 = t248;
            }
            t237 = [vbslq_u32(t238[0], t240[0], t237[0]), vbslq_u32(t238[1], t240[1], t237[1]), vbslq_u32(t238[0], t240[2], t237[2]), vbslq_u32(t238[1], t240[3], t237[3])];
        }
        {
            t237 = [vbslq_u32(t239[0], x56[0], t237[0]), vbslq_u32(t239[1], x56[1], t237[1]), vbslq_u32(t239[0], x56[2], t237[2]), vbslq_u32(t239[1], x56[3], t237[3])];
        }
        let t249 = (arg3 as *const u8).add(((gid0.wrapping_add((26u32).wrapping_mul(x88))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t249 as *mut u32).add(0), uint32x4x2_t(t237[0], t237[2]));
            vst2q_u32((t249 as *mut u32).add(8), uint32x4x2_t(t237[1], t237[3]));
        } else {
            let mut t250w = 0u64;
            let t250 = core::hint::black_box(&raw mut t250w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t249 as *mut u64).add(0), t250).write_unaligned(((vgetq_lane_u32::<0>(t237[2]) as u64) << 32 | vgetq_lane_u32::<0>(t237[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t249 as *mut u64).add(1), t250).write_unaligned(((vgetq_lane_u32::<1>(t237[2]) as u64) << 32 | vgetq_lane_u32::<1>(t237[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t249 as *mut u64).add(2), t250).write_unaligned(((vgetq_lane_u32::<2>(t237[2]) as u64) << 32 | vgetq_lane_u32::<2>(t237[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t249 as *mut u64).add(3), t250).write_unaligned(((vgetq_lane_u32::<3>(t237[2]) as u64) << 32 | vgetq_lane_u32::<3>(t237[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t249 as *mut u64).add(4), t250).write_unaligned(((vgetq_lane_u32::<0>(t237[3]) as u64) << 32 | vgetq_lane_u32::<0>(t237[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t249 as *mut u64).add(5), t250).write_unaligned(((vgetq_lane_u32::<1>(t237[3]) as u64) << 32 | vgetq_lane_u32::<1>(t237[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t249 as *mut u64).add(6), t250).write_unaligned(((vgetq_lane_u32::<2>(t237[3]) as u64) << 32 | vgetq_lane_u32::<2>(t237[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t249 as *mut u64).add(7), t250).write_unaligned(((vgetq_lane_u32::<3>(t237[3]) as u64) << 32 | vgetq_lane_u32::<3>(t237[1]) as u64));
        }
    }
}

#[inline(always)]
unsafe fn b0_r0r1r2r3sg_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, arg2: *const u8, arg3: *const u8, x32: *const u8) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x32 as *const u32).read_unaligned();
    let a1: u64 = (x32.wrapping_add(8u64 as usize) as *const u64).read_unaligned();
    let mut x56: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x64: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x72: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x80: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x88: u32 = 0;
    let mut x96: u32 = 0;
    let mut x104: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x112: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x120: u32 = 0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    if ((gid0) as u64 + 7) < ((a0) as u64) {
        let t251 = (arg0 as *const u8).add(((gid0) as usize) * 8);
        let t2530 = vld2q_u32((t251 as *const u32).add(0));
        let t2531 = vld2q_u32((t251 as *const u32).add(8));
        let t253: [uint32x4_t; 4] = [t2530.0, t2531.0, t2530.1, t2531.1];
        x56 = t253;
        let t254 = (arg1 as *const u8).add(((gid0) as usize) * 8);
        let t2560 = vld2q_u32((t254 as *const u32).add(0));
        let t2561 = vld2q_u32((t254 as *const u32).add(8));
        let t256: [uint32x4_t; 4] = [t2560.0, t2561.0, t2560.1, t2561.1];
        x64 = t256;
        let t257 = (arg2 as *const u32).add((gid0) as usize);
        let t258: [uint32x4_t; 2] = [vld1q_u32(t257.add(0)), vld1q_u32(t257.add(4))];
        x72 = t258;
        let t259 = [vdupq_n_u32(63u32); 2];
        let t260: [uint32x4_t; 2] = [vandq_u32(x72[0], t259[0]), vandq_u32(x72[1], t259[1])];
        x80 = t260;
        x88 = a0;
        let t261: u32;
        if ((((a1) & (1u64))) != (0u64)) {
            t261 = 8u32;
        } else {
            t261 = 4u32;
        }
        x96 = t261;
        let t262: [uint32x4_t; 2] = [vaddq_u32(x56[0], x64[0]), vaddq_u32(x56[1], x64[1])];
        let t263: [uint32x4_t; 2] = [vcltq_u32(t262[0], x56[0]), vcltq_u32(t262[1], x56[1])];
        let t264: [uint32x4_t; 4] = [t262[0], t262[1], vsubq_u32(vaddq_u32(x56[2], x64[2]), t263[0]), vsubq_u32(vaddq_u32(x56[3], x64[3]), t263[1])];
        let t265 = (arg3 as *const u8).add(((gid0.wrapping_add((0u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t265 as *mut u32).add(0), uint32x4x2_t(t264[0], t264[2]));
        vst2q_u32((t265 as *mut u32).add(8), uint32x4x2_t(t264[1], t264[3]));
        let t266: [uint32x4_t; 2] = [vcltq_u32(x56[0], x64[0]), vcltq_u32(x56[1], x64[1])];
        let t267: [uint32x4_t; 4] = [vsubq_u32(x56[0], x64[0]), vsubq_u32(x56[1], x64[1]), vaddq_u32(vsubq_u32(x56[2], x64[2]), t266[0]), vaddq_u32(vsubq_u32(x56[3], x64[3]), t266[1])];
        let t268 = (arg3 as *const u8).add(((gid0.wrapping_add((1u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t268 as *mut u32).add(0), uint32x4x2_t(t267[0], t267[2]));
        vst2q_u32((t268 as *mut u32).add(8), uint32x4x2_t(t267[1], t267[3]));
        let t269: [uint32x4_t; 2] = [x80[0], x80[1]];
        let t270: [uint32x4_t; 2] = [vandq_u32(t269[0], vdupq_n_u32(63)), vandq_u32(t269[1], vdupq_n_u32(63))];
        let t271: [uint32x4_t; 4] = [vshlq_u32(x56[0], vreinterpretq_s32_u32(t270[0])), vshlq_u32(x56[1], vreinterpretq_s32_u32(t270[1])), vorrq_u32(vshlq_u32(x56[2], vreinterpretq_s32_u32(t270[0])), vshlq_u32(x56[0], vsubq_s32(vreinterpretq_s32_u32(t270[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(x56[3], vreinterpretq_s32_u32(t270[1])), vshlq_u32(x56[1], vsubq_s32(vreinterpretq_s32_u32(t270[1]), vdupq_n_s32(32))))];
        let t272 = (arg3 as *const u8).add(((gid0.wrapping_add((2u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t272 as *mut u32).add(0), uint32x4x2_t(t271[0], t271[2]));
        vst2q_u32((t272 as *mut u32).add(8), uint32x4x2_t(t271[1], t271[3]));
        let t273: [uint32x4_t; 2] = [x80[0], x80[1]];
        let t274: [uint32x4_t; 2] = [vandq_u32(t273[0], vdupq_n_u32(63)), vandq_u32(t273[1], vdupq_n_u32(63))];
        let t275: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(x56[0], vnegq_s32(vreinterpretq_s32_u32(t274[0]))), vshlq_u32(x56[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t274[0])))), vorrq_u32(vshlq_u32(x56[1], vnegq_s32(vreinterpretq_s32_u32(t274[1]))), vshlq_u32(x56[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t274[1])))), vshlq_u32(x56[2], vnegq_s32(vreinterpretq_s32_u32(t274[0]))), vshlq_u32(x56[3], vnegq_s32(vreinterpretq_s32_u32(t274[1])))];
        let t276 = (arg3 as *const u8).add(((gid0.wrapping_add((3u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t276 as *mut u32).add(0), uint32x4x2_t(t275[0], t275[2]));
        vst2q_u32((t276 as *mut u32).add(8), uint32x4x2_t(t275[1], t275[3]));
        let t277: [uint32x4_t; 2] = [x80[0], x80[1]];
        let t278: [uint32x4_t; 2] = [vandq_u32(t277[0], vdupq_n_u32(63)), vandq_u32(t277[1], vdupq_n_u32(63))];
        let t279: [uint32x4_t; 2] = [vcgeq_s32(vreinterpretq_s32_u32(t278[0]), vdupq_n_s32(32)), vcgeq_s32(vreinterpretq_s32_u32(t278[1]), vdupq_n_s32(32))];
        let t280: [uint32x4_t; 4] = [vbslq_u32(t279[0], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x56[2]), vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t278[0])))), vorrq_u32(vshlq_u32(x56[0], vnegq_s32(vreinterpretq_s32_u32(t278[0]))), vshlq_u32(x56[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t278[0]))))), vbslq_u32(t279[1], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x56[3]), vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t278[1])))), vorrq_u32(vshlq_u32(x56[1], vnegq_s32(vreinterpretq_s32_u32(t278[1]))), vshlq_u32(x56[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t278[1]))))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x56[2]), vnegq_s32(vreinterpretq_s32_u32(t278[0])))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x56[3]), vnegq_s32(vreinterpretq_s32_u32(t278[1]))))];
        let t281 = (arg3 as *const u8).add(((gid0.wrapping_add((4u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t281 as *mut u32).add(0), uint32x4x2_t(t280[0], t280[2]));
        vst2q_u32((t281 as *mut u32).add(8), uint32x4x2_t(t280[1], t280[3]));
        let t282: [uint32x4_t; 4] = [vshlq_n_u32::<13>(x56[0]), vshlq_n_u32::<13>(x56[1]), vorrq_u32(vshlq_n_u32::<13>(x56[2]), vshrq_n_u32::<19>(x56[0])), vorrq_u32(vshlq_n_u32::<13>(x56[3]), vshrq_n_u32::<19>(x56[1]))];
        let t283: [uint32x4_t; 2] = [vshrq_n_u32::<19>(x64[2]), vshrq_n_u32::<19>(x64[3])];
        let t284: [uint32x4_t; 4] = [vorrq_u32(t282[0], t283[0]), vorrq_u32(t282[1], t283[1]), t282[2], t282[3]];
        let t285 = (arg3 as *const u8).add(((gid0.wrapping_add((5u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t285 as *mut u32).add(0), uint32x4x2_t(t284[0], t284[2]));
        vst2q_u32((t285 as *mut u32).add(8), uint32x4x2_t(t284[1], t284[3]));
        let t286: [uint32x4_t; 4] = [vdupq_n_u32(0), vdupq_n_u32(0), vshlq_n_u32::<8>(x56[0]), vshlq_n_u32::<8>(x56[1])];
        let t287 = (arg3 as *const u8).add(((gid0.wrapping_add((6u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t287 as *mut u32).add(0), uint32x4x2_t(t286[0], t286[2]));
        vst2q_u32((t287 as *mut u32).add(8), uint32x4x2_t(t286[1], t286[3]));
        let t288: [uint32x4_t; 2] = [vshrq_n_u32::<3>(x56[2]), vshrq_n_u32::<3>(x56[3])];
        let t289: [uint32x4_t; 4] = [t288[0], t288[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        let t290 = (arg3 as *const u8).add(((gid0.wrapping_add((7u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t290 as *mut u32).add(0), uint32x4x2_t(t289[0], t289[2]));
        vst2q_u32((t290 as *mut u32).add(8), uint32x4x2_t(t289[1], t289[3]));
        let t291 = vdupq_n_s32(((((40i64) as u32)) & 63) as i32);
        let t292: [uint32x4_t; 2] = [vcgeq_s32(t291, vdupq_n_s32(32)), vcgeq_s32(t291, vdupq_n_s32(32))];
        let t293: [uint32x4_t; 4] = [vbslq_u32(t292[0], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x56[2]), vsubq_s32(vdupq_n_s32(32), t291))), vorrq_u32(vshlq_u32(x56[0], vnegq_s32(t291)), vshlq_u32(x56[2], vsubq_s32(vdupq_n_s32(32), t291)))), vbslq_u32(t292[1], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x56[3]), vsubq_s32(vdupq_n_s32(32), t291))), vorrq_u32(vshlq_u32(x56[1], vnegq_s32(t291)), vshlq_u32(x56[3], vsubq_s32(vdupq_n_s32(32), t291)))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x56[2]), vnegq_s32(t291))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x56[3]), vnegq_s32(t291)))];
        let t294 = (arg3 as *const u8).add(((gid0.wrapping_add((8u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t294 as *mut u32).add(0), uint32x4x2_t(t293[0], t293[2]));
        vst2q_u32((t294 as *mut u32).add(8), uint32x4x2_t(t293[1], t293[3]));
        let t295: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x56[0]), vget_low_u32(x64[0]))), vreinterpretq_u32_u64(vmull_high_u32(x56[0], x64[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x56[1]), vget_low_u32(x64[1]))), vreinterpretq_u32_u64(vmull_high_u32(x56[1], x64[1])))];
        let t296: [uint32x4_t; 4] = [vmulq_u32(x56[0], x64[0]), vmulq_u32(x56[1], x64[1]), vmlaq_u32(vmlaq_u32(t295[0], x56[0], x64[2]), x64[0], x56[2]), vmlaq_u32(vmlaq_u32(t295[1], x56[1], x64[3]), x64[1], x56[3])];
        let t297 = (arg3 as *const u8).add(((gid0.wrapping_add((9u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t297 as *mut u32).add(0), uint32x4x2_t(t296[0], t296[2]));
        vst2q_u32((t297 as *mut u32).add(8), uint32x4x2_t(t296[1], t296[3]));
        let t298: u64 = 0u64;
        let t299 = [vdupq_n_u32((t298) as u32); 2];
        let t300: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x64[0], t299[0]), vceqq_u32(x64[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(x64[1], t299[1]), vceqq_u32(x64[3], vdupq_n_u32(0)))];
        let t301: [uint32x4_t; 2] = [vmvnq_u32(t300[0]), vmvnq_u32(t300[1])];
        let mut t302: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t303: [uint32x4_t; 2] = [vmvnq_u32(t301[0]), vmvnq_u32(t301[1])];
        {
            let (mut t304xl, mut t304xh, mut t304yl, mut t304yh) = ([0u32; 8], [0u32; 8], [0u32; 8], [0u32; 8]);
            vst1q_u32(t304xl.as_mut_ptr().add(0), x56[0]); vst1q_u32(t304xh.as_mut_ptr().add(0), x56[2]); vst1q_u32(t304yl.as_mut_ptr().add(0), x64[0]); vst1q_u32(t304yh.as_mut_ptr().add(0), x64[2]);
            vst1q_u32(t304xl.as_mut_ptr().add(4), x56[1]); vst1q_u32(t304xh.as_mut_ptr().add(4), x56[3]); vst1q_u32(t304yl.as_mut_ptr().add(4), x64[1]); vst1q_u32(t304yh.as_mut_ptr().add(4), x64[3]);
            for j in 0..8 { let q = (((t304xh[j] as u64) << 32 | t304xl[j] as u64) as u64).checked_div(((t304yh[j] as u64) << 32 | t304yl[j] as u64) as u64).unwrap_or(0) as u64; t304xl[j] = q as u32; t304xh[j] = (q >> 32) as u32; }
            let t305: [uint32x4_t; 4] = [vld1q_u32(t304xl.as_ptr().add(0)), vld1q_u32(t304xl.as_ptr().add(4)), vld1q_u32(t304xh.as_ptr().add(0)), vld1q_u32(t304xh.as_ptr().add(4))];
            t302 = [vbslq_u32(t301[0], t305[0], t302[0]), vbslq_u32(t301[1], t305[1], t302[1]), vbslq_u32(t301[0], t305[2], t302[2]), vbslq_u32(t301[1], t305[3], t302[3])];
        }
        {
            let t306: u64 = 7u64;
            let t307 = [vdupq_n_u32((t306) as u32); 2];
            let t308: [uint32x4_t; 4] = [t307[0], t307[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            t302 = [vbslq_u32(t303[0], t308[0], t302[0]), vbslq_u32(t303[1], t308[1], t302[1]), vbslq_u32(t303[0], t308[2], t302[2]), vbslq_u32(t303[1], t308[3], t302[3])];
        }
        let t309 = (arg3 as *const u8).add(((gid0.wrapping_add((10u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t309 as *mut u32).add(0), uint32x4x2_t(t302[0], t302[2]));
        vst2q_u32((t309 as *mut u32).add(8), uint32x4x2_t(t302[1], t302[3]));
        let t310: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x56[2], x64[2]), vandq_u32(vceqq_u32(x56[2], x64[2]), vcltq_u32(x56[0], x64[0]))), vorrq_u32(vcltq_u32(x56[3], x64[3]), vandq_u32(vceqq_u32(x56[3], x64[3]), vcltq_u32(x56[1], x64[1])))];
        let t311: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t310[0]), vshrq_n_u32::<31>(t310[1])];
        let t312: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x64[2], x56[2]), vandq_u32(vceqq_u32(x64[2], x56[2]), vcltq_u32(x64[0], x56[0]))), vorrq_u32(vcltq_u32(x64[3], x56[3]), vandq_u32(vceqq_u32(x64[3], x56[3]), vcltq_u32(x64[1], x56[1])))];
        let t313: [uint32x4_t; 2] = [vmvnq_u32(t312[0]), vmvnq_u32(t312[1])];
        let t314: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t313[0]), vshrq_n_u32::<31>(t313[1])];
        let t315: [uint32x4_t; 4] = [vshlq_n_u32::<1>(t314[0]), vshlq_n_u32::<1>(t314[1]), vshrq_n_u32::<31>(t314[0]), vshrq_n_u32::<31>(t314[1])];
        let t316: [uint32x4_t; 4] = [vorrq_u32(t311[0], t315[0]), vorrq_u32(t311[1], t315[1]), t315[2], t315[3]];
        let t317: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x56[0], x64[0]), vceqq_u32(x56[2], x64[2])), vandq_u32(vceqq_u32(x56[1], x64[1]), vceqq_u32(x56[3], x64[3]))];
        let t318: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t317[0]), vshrq_n_u32::<31>(t317[1])];
        let t319: [uint32x4_t; 4] = [vshlq_n_u32::<2>(t318[0]), vshlq_n_u32::<2>(t318[1]), vshrq_n_u32::<30>(t318[0]), vshrq_n_u32::<30>(t318[1])];
        let t320: [uint32x4_t; 4] = [vorrq_u32(t316[0], t319[0]), vorrq_u32(t316[1], t319[1]), vorrq_u32(t316[2], t319[2]), vorrq_u32(t316[3], t319[3])];
        let t321: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x56[0], x64[0]), vceqq_u32(x56[2], x64[2])), vandq_u32(vceqq_u32(x56[1], x64[1]), vceqq_u32(x56[3], x64[3]))];
        let t322: [uint32x4_t; 2] = [vmvnq_u32(t321[0]), vmvnq_u32(t321[1])];
        let t323: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t322[0]), vshrq_n_u32::<31>(t322[1])];
        let t324: [uint32x4_t; 4] = [vshlq_n_u32::<3>(t323[0]), vshlq_n_u32::<3>(t323[1]), vshrq_n_u32::<29>(t323[0]), vshrq_n_u32::<29>(t323[1])];
        let t325: [uint32x4_t; 4] = [vorrq_u32(t320[0], t324[0]), vorrq_u32(t320[1], t324[1]), vorrq_u32(t320[2], t324[2]), vorrq_u32(t320[3], t324[3])];
        let t326: [uint32x4_t; 2] = [vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x56[2]), vreinterpretq_s32_u32(x64[2])), vandq_u32(vceqq_u32(x56[2], x64[2]), vcltq_u32(x56[0], x64[0]))), vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x56[3]), vreinterpretq_s32_u32(x64[3])), vandq_u32(vceqq_u32(x56[3], x64[3]), vcltq_u32(x56[1], x64[1])))];
        let t327: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t326[0]), vshrq_n_u32::<31>(t326[1])];
        let t328: [uint32x4_t; 4] = [vshlq_n_u32::<4>(t327[0]), vshlq_n_u32::<4>(t327[1]), vshrq_n_u32::<28>(t327[0]), vshrq_n_u32::<28>(t327[1])];
        let t329: [uint32x4_t; 4] = [vorrq_u32(t325[0], t328[0]), vorrq_u32(t325[1], t328[1]), vorrq_u32(t325[2], t328[2]), vorrq_u32(t325[3], t328[3])];
        let t330: [uint32x4_t; 2] = [vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x56[2]), vreinterpretq_s32_u32(x64[2])), vandq_u32(vceqq_u32(x56[2], x64[2]), vcltq_u32(x56[0], x64[0]))), vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x56[3]), vreinterpretq_s32_u32(x64[3])), vandq_u32(vceqq_u32(x56[3], x64[3]), vcltq_u32(x56[1], x64[1])))];
        let t331: [uint32x4_t; 2] = [vmvnq_u32(t330[0]), vmvnq_u32(t330[1])];
        let t332: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t331[0]), vshrq_n_u32::<31>(t331[1])];
        let t333: [uint32x4_t; 4] = [vshlq_n_u32::<5>(t332[0]), vshlq_n_u32::<5>(t332[1]), vshrq_n_u32::<27>(t332[0]), vshrq_n_u32::<27>(t332[1])];
        let t334: [uint32x4_t; 4] = [vorrq_u32(t329[0], t333[0]), vorrq_u32(t329[1], t333[1]), vorrq_u32(t329[2], t333[2]), vorrq_u32(t329[3], t333[3])];
        let t335: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x64[2], x56[2]), vandq_u32(vceqq_u32(x64[2], x56[2]), vcltq_u32(x64[0], x56[0]))), vorrq_u32(vcltq_u32(x64[3], x56[3]), vandq_u32(vceqq_u32(x64[3], x56[3]), vcltq_u32(x64[1], x56[1])))];
        let t336: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t335[0]), vshrq_n_u32::<31>(t335[1])];
        let t337: [uint32x4_t; 4] = [vshlq_n_u32::<6>(t336[0]), vshlq_n_u32::<6>(t336[1]), vshrq_n_u32::<26>(t336[0]), vshrq_n_u32::<26>(t336[1])];
        let t338: [uint32x4_t; 4] = [vorrq_u32(t334[0], t337[0]), vorrq_u32(t334[1], t337[1]), vorrq_u32(t334[2], t337[2]), vorrq_u32(t334[3], t337[3])];
        let t339 = (arg3 as *const u8).add(((gid0.wrapping_add((11u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t339 as *mut u32).add(0), uint32x4x2_t(t338[0], t338[2]));
        vst2q_u32((t339 as *mut u32).add(8), uint32x4x2_t(t338[1], t338[3]));
        let t340: [uint32x4_t; 2] = [vaddq_u32(x72[0], x56[0]), vaddq_u32(x72[1], x56[1])];
        let t341: [uint32x4_t; 2] = [vcltq_u32(t340[0], x72[0]), vcltq_u32(t340[1], x72[1])];
        let t342: [uint32x4_t; 4] = [t340[0], t340[1], vsubq_u32(vaddq_u32(vdupq_n_u32(0), x56[2]), t341[0]), vsubq_u32(vaddq_u32(vdupq_n_u32(0), x56[3]), t341[1])];
        let t343 = (arg3 as *const u8).add(((gid0.wrapping_add((12u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t343 as *mut u32).add(0), uint32x4x2_t(t342[0], t342[2]));
        vst2q_u32((t343 as *mut u32).add(8), uint32x4x2_t(t342[1], t342[3]));
        let t344: [uint32x4_t; 2] = [vreinterpretq_u32_s32(vshrq_n_s32::<31>(vreinterpretq_s32_u32(x72[0]))), vreinterpretq_u32_s32(vshrq_n_s32::<31>(vreinterpretq_s32_u32(x72[1])))];
        let t345: [uint32x4_t; 4] = [x72[0], x72[1], t344[0], t344[1]];
        let t346 = (arg3 as *const u8).add(((gid0.wrapping_add((13u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t346 as *mut u32).add(0), uint32x4x2_t(t345[0], t345[2]));
        vst2q_u32((t346 as *mut u32).add(8), uint32x4x2_t(t345[1], t345[3]));
        let t347: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<3>(x56[0]), vshlq_n_u32::<29>(x56[2])), vorrq_u32(vshrq_n_u32::<3>(x56[1]), vshlq_n_u32::<29>(x56[3])), vshrq_n_u32::<3>(x56[2]), vshrq_n_u32::<3>(x56[3])];
        let t348: [uint32x4_t; 2] = [t347[0], t347[1]];
        let t349: [uint32x4_t; 4] = [t348[0], t348[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        let t350 = (arg3 as *const u8).add(((gid0.wrapping_add((14u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t350 as *mut u32).add(0), uint32x4x2_t(t349[0], t349[2]));
        vst2q_u32((t350 as *mut u32).add(8), uint32x4x2_t(t349[1], t349[3]));
        let t351: u64 = 1u64;
        let t352 = [vdupq_n_u32((t351) as u32); 2];
        let t353: [uint32x4_t; 2] = [vandq_u32(x56[0], t352[0]), vandq_u32(x56[1], t352[1])];
        let t354: [uint32x4_t; 2] = [vtstq_u32(t353[0], t353[0]), vtstq_u32(t353[1], t353[1])];
        {
            let t355: [uint32x4_t; 4] = [veorq_u32(x56[0], x64[0]), veorq_u32(x56[1], x64[1]), veorq_u32(x56[2], x64[2]), veorq_u32(x56[3], x64[3])];
            x104 = [vbslq_u32(t354[0], t355[0], x104[0]), vbslq_u32(t354[1], t355[1], x104[1]), vbslq_u32(t354[0], t355[2], x104[2]), vbslq_u32(t354[1], t355[3], x104[3])];
        }
        let t356: [uint32x4_t; 2] = [vmvnq_u32(t354[0]), vmvnq_u32(t354[1])];
        {
            let t357: [uint32x4_t; 2] = [vaddq_u32(x56[0], x72[0]), vaddq_u32(x56[1], x72[1])];
            let t358: [uint32x4_t; 2] = [vcltq_u32(t357[0], x56[0]), vcltq_u32(t357[1], x56[1])];
            let t359: [uint32x4_t; 4] = [t357[0], t357[1], vsubq_u32(vaddq_u32(x56[2], vdupq_n_u32(0)), t358[0]), vsubq_u32(vaddq_u32(x56[3], vdupq_n_u32(0)), t358[1])];
            x104 = [vbslq_u32(t356[0], t359[0], x104[0]), vbslq_u32(t356[1], t359[1], x104[1]), vbslq_u32(t356[0], t359[2], x104[2]), vbslq_u32(t356[1], t359[3], x104[3])];
        }
        let t360 = (arg3 as *const u8).add(((gid0.wrapping_add((15u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t360 as *mut u32).add(0), uint32x4x2_t(x104[0], x104[2]));
        vst2q_u32((t360 as *mut u32).add(8), uint32x4x2_t(x104[1], x104[3]));
        let t361 = b1_l_f(l, lid0, x56);
        let t362 = b2_l_f(l, lid0, x64);
        let t363: [uint32x4_t; 4] = [vshlq_n_u32::<8>(t362[0]), vshlq_n_u32::<8>(t362[1]), vshrq_n_u32::<24>(t362[0]), vshrq_n_u32::<24>(t362[1])];
        let t364: [uint32x4_t; 2] = [vaddq_u32(t361[0], t363[0]), vaddq_u32(t361[1], t363[1])];
        let t365: [uint32x4_t; 2] = [vcltq_u32(t364[0], t361[0]), vcltq_u32(t364[1], t361[1])];
        let t366: [uint32x4_t; 4] = [t364[0], t364[1], vsubq_u32(vaddq_u32(vdupq_n_u32(0), t363[2]), t365[0]), vsubq_u32(vaddq_u32(vdupq_n_u32(0), t363[3]), t365[1])];
        let t367 = (arg3 as *const u8).add(((gid0.wrapping_add((16u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t367 as *mut u32).add(0), uint32x4x2_t(t366[0], t366[2]));
        vst2q_u32((t367 as *mut u32).add(8), uint32x4x2_t(t366[1], t366[3]));
        let t368 = [vdupq_n_u32(x88); 2];
        let mut t369x = [0u32; 8];
        let mut t369y = [0u32; 8];
        for r in 0..2 { vst1q_u32(t369x.as_mut_ptr().add(4 * r), x72[r]); vst1q_u32(t369y.as_mut_ptr().add(4 * r), t368[r]); }
        for j in 0..8 { t369x[j] = (t369x[j] as u32).checked_rem(t369y[j] as u32).unwrap_or(0) as u32; }
        let t370: [uint32x4_t; 2] = [vld1q_u32(t369x.as_ptr().add(0)), vld1q_u32(t369x.as_ptr().add(4))];
        let mut t371: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t370[0]) as usize * 8) as *const u64).read_unaligned(); t371[0] = vsetq_lane_u32::<0>(v as u32, t371[0]); t371[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t371[2]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t370[0]) as usize * 8) as *const u64).read_unaligned(); t371[0] = vsetq_lane_u32::<1>(v as u32, t371[0]); t371[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t371[2]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t370[0]) as usize * 8) as *const u64).read_unaligned(); t371[0] = vsetq_lane_u32::<2>(v as u32, t371[0]); t371[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t371[2]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t370[0]) as usize * 8) as *const u64).read_unaligned(); t371[0] = vsetq_lane_u32::<3>(v as u32, t371[0]); t371[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t371[2]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t370[1]) as usize * 8) as *const u64).read_unaligned(); t371[1] = vsetq_lane_u32::<0>(v as u32, t371[1]); t371[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t371[3]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t370[1]) as usize * 8) as *const u64).read_unaligned(); t371[1] = vsetq_lane_u32::<1>(v as u32, t371[1]); t371[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t371[3]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t370[1]) as usize * 8) as *const u64).read_unaligned(); t371[1] = vsetq_lane_u32::<2>(v as u32, t371[1]); t371[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t371[3]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t370[1]) as usize * 8) as *const u64).read_unaligned(); t371[1] = vsetq_lane_u32::<3>(v as u32, t371[1]); t371[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t371[3]); }
        let t372 = (arg3 as *const u8).add(((gid0.wrapping_add((17u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t372 as *mut u32).add(0), uint32x4x2_t(t371[0], t371[2]));
        vst2q_u32((t372 as *mut u32).add(8), uint32x4x2_t(t371[1], t371[3]));
        let t373 = b3_ll_f(l, lid0, x56, x72);
        let t374 = (arg3 as *const u8).add(((gid0.wrapping_add((18u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t374 as *mut u32).add(0), uint32x4x2_t(t373[0], t373[2]));
        vst2q_u32((t374 as *mut u32).add(8), uint32x4x2_t(t373[1], t373[3]));
        x112 = x56;
        x120 = 0u32;
        let mut t383 = live;
        loop {
            // Lanes part: each its own rounds.
            let t385 = [vdupq_n_u32(7u32); 2];
            let t386: [uint32x4_t; 2] = [vandq_u32(x72[0], t385[0]), vandq_u32(x72[1], t385[1])];
            let t387 = [vdupq_n_u32(x120); 2];
            let t388: [uint32x4_t; 2] = [vcltq_u32(t387[0], t386[0]), vcltq_u32(t387[1], t386[1])];
            t383 = [vandq_u32(t383[0], t388[0]), vandq_u32(t383[1], t388[1])];
            if !(any(&t383)) { break; }
            let t390: u64 = 3u64;
            let t391 = [vdupq_n_u32((t390) as u32); 2];
            let t392: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x112[0]), vget_low_u32(t391[0]))), vreinterpretq_u32_u64(vmull_high_u32(x112[0], t391[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x112[1]), vget_low_u32(t391[1]))), vreinterpretq_u32_u64(vmull_high_u32(x112[1], t391[1])))];
            let t393: [uint32x4_t; 4] = [vmulq_u32(x112[0], t391[0]), vmulq_u32(x112[1], t391[1]), vmlaq_u32(t392[0], t391[0], x112[2]), vmlaq_u32(t392[1], t391[1], x112[3])];
            let t394: [uint32x4_t; 2] = [vaddq_u32(t393[0], x64[0]), vaddq_u32(t393[1], x64[1])];
            let t395: [uint32x4_t; 2] = [vcltq_u32(t394[0], t393[0]), vcltq_u32(t394[1], t393[1])];
            let t396: [uint32x4_t; 4] = [t394[0], t394[1], vsubq_u32(vaddq_u32(t393[2], x64[2]), t395[0]), vsubq_u32(vaddq_u32(t393[3], x64[3]), t395[1])];
            x112 = [vbslq_u32(t383[0], t396[0], x112[0]), vbslq_u32(t383[1], t396[1], x112[1]), vbslq_u32(t383[0], t396[2], x112[2]), vbslq_u32(t383[1], t396[3], x112[3])];
            let t397: [uint32x4_t; 2] = [vandq_u32(t383[0], live[0]), vandq_u32(t383[1], live[1])];
            let t398 = x120;
            x120 = (x120).wrapping_add(1u32);
        }
        let t399 = (arg3 as *const u8).add(((gid0.wrapping_add((19u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t399 as *mut u32).add(0), uint32x4x2_t(x112[0], x112[2]));
        vst2q_u32((t399 as *mut u32).add(8), uint32x4x2_t(x112[1], x112[3]));
        let t400: u64 = a1;
        let t401 = [vdupq_n_u32((t400) as u32); 2];
        let t402 = [vdupq_n_u32(((t400) >> 32) as u32); 2];
        let t403: [uint32x4_t; 2] = [vaddq_u32(x56[0], t401[0]), vaddq_u32(x56[1], t401[1])];
        let t404: [uint32x4_t; 2] = [vcltq_u32(t403[0], x56[0]), vcltq_u32(t403[1], x56[1])];
        let t405: [uint32x4_t; 4] = [t403[0], t403[1], vsubq_u32(vaddq_u32(x56[2], t402[0]), t404[0]), vsubq_u32(vaddq_u32(x56[3], t402[1]), t404[1])];
        let t406: u64 = (a1).wrapping_shl((3u64) as u32);
        let t407 = [vdupq_n_u32((t406) as u32); 2];
        let t408 = [vdupq_n_u32(((t406) >> 32) as u32); 2];
        let t409: [uint32x4_t; 2] = [vaddq_u32(t405[0], t407[0]), vaddq_u32(t405[1], t407[1])];
        let t410: [uint32x4_t; 2] = [vcltq_u32(t409[0], t405[0]), vcltq_u32(t409[1], t405[1])];
        let t411: [uint32x4_t; 4] = [t409[0], t409[1], vsubq_u32(vaddq_u32(t405[2], t408[0]), t410[0]), vsubq_u32(vaddq_u32(t405[3], t408[1]), t410[1])];
        let t412 = (arg3 as *const u8).add(((gid0.wrapping_add((20u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t412 as *mut u32).add(0), uint32x4x2_t(t411[0], t411[2]));
        vst2q_u32((t412 as *mut u32).add(8), uint32x4x2_t(t411[1], t411[3]));
        let t413: u64 = 4294967295u64;
        let t414 = [vdupq_n_u32((t413) as u32); 2];
        let t415: [uint32x4_t; 2] = [vandq_u32(x56[0], t414[0]), vandq_u32(x56[1], t414[1])];
        let t416: u64 = 4294967295u64;
        let t417 = [vdupq_n_u32((t416) as u32); 2];
        let t418: [uint32x4_t; 2] = [vandq_u32(x64[0], t417[0]), vandq_u32(x64[1], t417[1])];
        let t419: [uint32x4_t; 2] = [vceqq_u32(t415[0], t418[0]), vceqq_u32(t415[1], t418[1])];
        let mut t420: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t421: [uint32x4_t; 2] = [vmvnq_u32(t419[0]), vmvnq_u32(t419[1])];
        {
            let t422 = [vdupq_n_u32(1u32); 2];
            t420 = [vbslq_u32(t419[0], t422[0], t420[0]), vbslq_u32(t419[1], t422[1], t420[1])];
        }
        {
            let t423 = [vdupq_n_u32(2u32); 2];
            t420 = [vbslq_u32(t421[0], t423[0], t420[0]), vbslq_u32(t421[1], t423[1], t420[1])];
        }
        let t424: [uint32x4_t; 4] = [t420[0], t420[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        let t425 = (arg3 as *const u8).add(((gid0.wrapping_add((21u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t425 as *mut u32).add(0), uint32x4x2_t(t424[0], t424[2]));
        vst2q_u32((t425 as *mut u32).add(8), uint32x4x2_t(t424[1], t424[3]));
        let t426: [uint32x4_t; 2] = [vtstq_u32(vorrq_u32(x56[0], x56[2]), vorrq_u32(x56[0], x56[2])), vtstq_u32(vorrq_u32(x56[1], x56[3]), vorrq_u32(x56[1], x56[3]))];
        let mut t427: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t428: [uint32x4_t; 2] = [vmvnq_u32(t426[0]), vmvnq_u32(t426[1])];
        {
            let t429 = [vdupq_n_u32(5u32); 2];
            t427 = [vbslq_u32(t426[0], t429[0], t427[0]), vbslq_u32(t426[1], t429[1], t427[1])];
        }
        {
            let t430 = [vdupq_n_u32(6u32); 2];
            t427 = [vbslq_u32(t428[0], t430[0], t427[0]), vbslq_u32(t428[1], t430[1], t427[1])];
        }
        let t431: [uint32x4_t; 4] = [t427[0], t427[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        let t432 = (arg3 as *const u8).add(((gid0.wrapping_add((22u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t432 as *mut u32).add(0), uint32x4x2_t(t431[0], t431[2]));
        vst2q_u32((t432 as *mut u32).add(8), uint32x4x2_t(t431[1], t431[3]));
        let t433 = b4_ll_f(l, lid0, x56, x64);
        let t434 = (arg3 as *const u8).add(((gid0.wrapping_add((23u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t434 as *mut u32).add(0), uint32x4x2_t(t433[0], t433[2]));
        vst2q_u32((t434 as *mut u32).add(8), uint32x4x2_t(t433[1], t433[3]));
        let t435 = b5_r0ns_f_v0(l, lid0, arg0, gid0.wrapping_mul(x96), x96, 8u32);
        let t436 = (arg3 as *const u8).add(((gid0.wrapping_add((24u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t436 as *mut u32).add(0), uint32x4x2_t(t435[0], t435[2]));
        vst2q_u32((t436 as *mut u32).add(8), uint32x4x2_t(t435[1], t435[3]));
        let t437 = if (gid0.wrapping_add(3u32) & 255u32) <= 248u32 { b6_u8_f(l, lid0, gid0.wrapping_add(3u32)) } else { b6_i_f(l, lid0, gid0.wrapping_add(3u32)) };
        let t438 = (arg3 as *const u8).add(((gid0.wrapping_add((25u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t438 as *mut u32).add(0), uint32x4x2_t(t437[0], t437[2]));
        vst2q_u32((t438 as *mut u32).add(8), uint32x4x2_t(t437[1], t437[3]));
        let t439: u64 = 3u64;
        let t440 = [vdupq_n_u32((t439) as u32); 2];
        let t441: [uint32x4_t; 2] = [vandq_u32(x56[0], t440[0]), vandq_u32(x56[1], t440[1])];
        let t442: u64 = 0u64;
        let t443 = [vdupq_n_u32((t442) as u32); 2];
        let t444: [uint32x4_t; 2] = [vceqq_u32(t441[0], t443[0]), vceqq_u32(t441[1], t443[1])];
        let mut t445: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t446: [uint32x4_t; 2] = [vmvnq_u32(t444[0]), vmvnq_u32(t444[1])];
        if any(&t444) {
            let t447: [uint32x4_t; 4];
            let t448 = bits(&t444);
            if t448.count_ones() <= 2 {
                let t449 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x64);
                let t450 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x72);
                let mut t451 = [0u32; 16];
                let mut t452 = t448;
                while t452 != 0 {
                    let j = t452.trailing_zeros() as usize;
                    t452 &= t452 - 1;
                    let t453 = b7_ss_f(l, lid0, ((t449[8 + j] as u64) << 32 | t449[j] as u64), t450[j]);
                    let t454: u64 = t453;
                    t451[j] = t454 as u32;
                    t451[8 + j] = (t454 >> 32) as u32;
                }
                t447 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t451);
            } else {
                let t455 = b7_ll_p(l, lid0, t444, x64, x72);
                t447 = t455;
            }
            t445 = [vbslq_u32(t444[0], t447[0], t445[0]), vbslq_u32(t444[1], t447[1], t445[1]), vbslq_u32(t444[0], t447[2], t445[2]), vbslq_u32(t444[1], t447[3], t445[3])];
        }
        {
            t445 = [vbslq_u32(t446[0], x56[0], t445[0]), vbslq_u32(t446[1], x56[1], t445[1]), vbslq_u32(t446[0], x56[2], t445[2]), vbslq_u32(t446[1], x56[3], t445[3])];
        }
        let t456 = (arg3 as *const u8).add(((gid0.wrapping_add((26u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t456 as *mut u32).add(0), uint32x4x2_t(t445[0], t445[2]));
        vst2q_u32((t456 as *mut u32).add(8), uint32x4x2_t(t445[1], t445[3]));
    } else if (((gid0) as u64 + 7) <= u32::MAX as u64 && ((gid0) as u64) >= ((a0) as u64)) {
        return;
    } else {
        // Lanes part: the chunk's lanes on both sides of the bound.
        let t457: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t458 = [vdupq_n_u32(a0); 2];
        let t459: [uint32x4_t; 2] = [vcgeq_u32(t457[0], t458[0]), vcgeq_u32(t457[1], t458[1])];
        let t460: [uint32x4_t; 2] = [vandq_u32(live[0], t459[0]), vandq_u32(live[1], t459[1])];
        {
            live = [vbicq_u32(live[0], t460[0]), vbicq_u32(live[1], t460[1])];
        }
        if any(&live) {
            let t461 = (arg0 as *const u8).add(((gid0) as usize) * 8);
            let mut t462: [uint32x4_t; 4];
            if all(&live) {
                let t4630 = vld2q_u32((t461 as *const u32).add(0));
                let t4631 = vld2q_u32((t461 as *const u32).add(8));
                let t463: [uint32x4_t; 4] = [t4630.0, t4631.0, t4630.1, t4631.1];
                t462 = t463;
            } else {
                let t464 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t457[0]), vandq_u32(live[1], t457[1]))));
                let t465: [uint32x4_t; 2] = [vbslq_u32(live[0], t457[0], t464), vbslq_u32(live[1], t457[1], t464)];
                let mut t466: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t465[0]) as usize * 8) as *const u64).read_unaligned(); t466[0] = vsetq_lane_u32::<0>(v as u32, t466[0]); t466[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t466[2]); }
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t465[0]) as usize * 8) as *const u64).read_unaligned(); t466[0] = vsetq_lane_u32::<1>(v as u32, t466[0]); t466[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t466[2]); }
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t465[0]) as usize * 8) as *const u64).read_unaligned(); t466[0] = vsetq_lane_u32::<2>(v as u32, t466[0]); t466[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t466[2]); }
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t465[0]) as usize * 8) as *const u64).read_unaligned(); t466[0] = vsetq_lane_u32::<3>(v as u32, t466[0]); t466[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t466[2]); }
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t465[1]) as usize * 8) as *const u64).read_unaligned(); t466[1] = vsetq_lane_u32::<0>(v as u32, t466[1]); t466[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t466[3]); }
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t465[1]) as usize * 8) as *const u64).read_unaligned(); t466[1] = vsetq_lane_u32::<1>(v as u32, t466[1]); t466[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t466[3]); }
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t465[1]) as usize * 8) as *const u64).read_unaligned(); t466[1] = vsetq_lane_u32::<2>(v as u32, t466[1]); t466[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t466[3]); }
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t465[1]) as usize * 8) as *const u64).read_unaligned(); t466[1] = vsetq_lane_u32::<3>(v as u32, t466[1]); t466[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t466[3]); }
                t462 = t466;
            }
            x56 = t462;
            let t467 = (arg1 as *const u8).add(((gid0) as usize) * 8);
            let mut t468: [uint32x4_t; 4];
            if all(&live) {
                let t4690 = vld2q_u32((t467 as *const u32).add(0));
                let t4691 = vld2q_u32((t467 as *const u32).add(8));
                let t469: [uint32x4_t; 4] = [t4690.0, t4691.0, t4690.1, t4691.1];
                t468 = t469;
            } else {
                let t470 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t457[0]), vandq_u32(live[1], t457[1]))));
                let t471: [uint32x4_t; 2] = [vbslq_u32(live[0], t457[0], t470), vbslq_u32(live[1], t457[1], t470)];
                let mut t472: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
                { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<0>(t471[0]) as usize * 8) as *const u64).read_unaligned(); t472[0] = vsetq_lane_u32::<0>(v as u32, t472[0]); t472[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t472[2]); }
                { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<1>(t471[0]) as usize * 8) as *const u64).read_unaligned(); t472[0] = vsetq_lane_u32::<1>(v as u32, t472[0]); t472[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t472[2]); }
                { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<2>(t471[0]) as usize * 8) as *const u64).read_unaligned(); t472[0] = vsetq_lane_u32::<2>(v as u32, t472[0]); t472[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t472[2]); }
                { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<3>(t471[0]) as usize * 8) as *const u64).read_unaligned(); t472[0] = vsetq_lane_u32::<3>(v as u32, t472[0]); t472[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t472[2]); }
                { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<0>(t471[1]) as usize * 8) as *const u64).read_unaligned(); t472[1] = vsetq_lane_u32::<0>(v as u32, t472[1]); t472[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t472[3]); }
                { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<1>(t471[1]) as usize * 8) as *const u64).read_unaligned(); t472[1] = vsetq_lane_u32::<1>(v as u32, t472[1]); t472[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t472[3]); }
                { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<2>(t471[1]) as usize * 8) as *const u64).read_unaligned(); t472[1] = vsetq_lane_u32::<2>(v as u32, t472[1]); t472[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t472[3]); }
                { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<3>(t471[1]) as usize * 8) as *const u64).read_unaligned(); t472[1] = vsetq_lane_u32::<3>(v as u32, t472[1]); t472[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t472[3]); }
                t468 = t472;
            }
            x64 = t468;
            let mut t473: [uint32x4_t; 2];
            if all(&live) {
                let t474 = (arg2 as *const u32).add((gid0) as usize);
                t473 = [vld1q_u32(t474.add(0)), vld1q_u32(t474.add(4))];
            } else {
                let t475 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t457[0]), vandq_u32(live[1], t457[1]))));
                let t476: [uint32x4_t; 2] = [vbslq_u32(live[0], t457[0], t475), vbslq_u32(live[1], t457[1], t475)];
                let t477 = arg2 as *const u32;
                let mut t478: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                t478[0] = vld1q_lane_u32::<0>(t477.add(vgetq_lane_u32::<0>(t476[0]) as usize), t478[0]);
                t478[0] = vld1q_lane_u32::<1>(t477.add(vgetq_lane_u32::<1>(t476[0]) as usize), t478[0]);
                t478[0] = vld1q_lane_u32::<2>(t477.add(vgetq_lane_u32::<2>(t476[0]) as usize), t478[0]);
                t478[0] = vld1q_lane_u32::<3>(t477.add(vgetq_lane_u32::<3>(t476[0]) as usize), t478[0]);
                t478[1] = vld1q_lane_u32::<0>(t477.add(vgetq_lane_u32::<0>(t476[1]) as usize), t478[1]);
                t478[1] = vld1q_lane_u32::<1>(t477.add(vgetq_lane_u32::<1>(t476[1]) as usize), t478[1]);
                t478[1] = vld1q_lane_u32::<2>(t477.add(vgetq_lane_u32::<2>(t476[1]) as usize), t478[1]);
                t478[1] = vld1q_lane_u32::<3>(t477.add(vgetq_lane_u32::<3>(t476[1]) as usize), t478[1]);
                t473 = t478;
            }
            x72 = t473;
            let t479 = [vdupq_n_u32(63u32); 2];
            let t480: [uint32x4_t; 2] = [vandq_u32(x72[0], t479[0]), vandq_u32(x72[1], t479[1])];
            x80 = t480;
            x88 = a0;
            let t481: u32;
            if ((((a1) & (1u64))) != (0u64)) {
                t481 = 8u32;
            } else {
                t481 = 4u32;
            }
            x96 = t481;
            let t482: [uint32x4_t; 2] = [vaddq_u32(x56[0], x64[0]), vaddq_u32(x56[1], x64[1])];
            let t483: [uint32x4_t; 2] = [vcltq_u32(t482[0], x56[0]), vcltq_u32(t482[1], x56[1])];
            let t484: [uint32x4_t; 4] = [t482[0], t482[1], vsubq_u32(vaddq_u32(x56[2], x64[2]), t483[0]), vsubq_u32(vaddq_u32(x56[3], x64[3]), t483[1])];
            let t485 = (arg3 as *const u8).add(((gid0.wrapping_add((0u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t485 as *mut u32).add(0), uint32x4x2_t(t484[0], t484[2]));
                vst2q_u32((t485 as *mut u32).add(8), uint32x4x2_t(t484[1], t484[3]));
            } else {
                let mut t486w = 0u64;
                let t486 = core::hint::black_box(&raw mut t486w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t485 as *mut u64).add(0), t486).write_unaligned(((vgetq_lane_u32::<0>(t484[2]) as u64) << 32 | vgetq_lane_u32::<0>(t484[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t485 as *mut u64).add(1), t486).write_unaligned(((vgetq_lane_u32::<1>(t484[2]) as u64) << 32 | vgetq_lane_u32::<1>(t484[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t485 as *mut u64).add(2), t486).write_unaligned(((vgetq_lane_u32::<2>(t484[2]) as u64) << 32 | vgetq_lane_u32::<2>(t484[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t485 as *mut u64).add(3), t486).write_unaligned(((vgetq_lane_u32::<3>(t484[2]) as u64) << 32 | vgetq_lane_u32::<3>(t484[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t485 as *mut u64).add(4), t486).write_unaligned(((vgetq_lane_u32::<0>(t484[3]) as u64) << 32 | vgetq_lane_u32::<0>(t484[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t485 as *mut u64).add(5), t486).write_unaligned(((vgetq_lane_u32::<1>(t484[3]) as u64) << 32 | vgetq_lane_u32::<1>(t484[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t485 as *mut u64).add(6), t486).write_unaligned(((vgetq_lane_u32::<2>(t484[3]) as u64) << 32 | vgetq_lane_u32::<2>(t484[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t485 as *mut u64).add(7), t486).write_unaligned(((vgetq_lane_u32::<3>(t484[3]) as u64) << 32 | vgetq_lane_u32::<3>(t484[1]) as u64));
            }
            let t487: [uint32x4_t; 2] = [vcltq_u32(x56[0], x64[0]), vcltq_u32(x56[1], x64[1])];
            let t488: [uint32x4_t; 4] = [vsubq_u32(x56[0], x64[0]), vsubq_u32(x56[1], x64[1]), vaddq_u32(vsubq_u32(x56[2], x64[2]), t487[0]), vaddq_u32(vsubq_u32(x56[3], x64[3]), t487[1])];
            let t489 = (arg3 as *const u8).add(((gid0.wrapping_add((1u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t489 as *mut u32).add(0), uint32x4x2_t(t488[0], t488[2]));
                vst2q_u32((t489 as *mut u32).add(8), uint32x4x2_t(t488[1], t488[3]));
            } else {
                let mut t490w = 0u64;
                let t490 = core::hint::black_box(&raw mut t490w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t489 as *mut u64).add(0), t490).write_unaligned(((vgetq_lane_u32::<0>(t488[2]) as u64) << 32 | vgetq_lane_u32::<0>(t488[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t489 as *mut u64).add(1), t490).write_unaligned(((vgetq_lane_u32::<1>(t488[2]) as u64) << 32 | vgetq_lane_u32::<1>(t488[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t489 as *mut u64).add(2), t490).write_unaligned(((vgetq_lane_u32::<2>(t488[2]) as u64) << 32 | vgetq_lane_u32::<2>(t488[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t489 as *mut u64).add(3), t490).write_unaligned(((vgetq_lane_u32::<3>(t488[2]) as u64) << 32 | vgetq_lane_u32::<3>(t488[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t489 as *mut u64).add(4), t490).write_unaligned(((vgetq_lane_u32::<0>(t488[3]) as u64) << 32 | vgetq_lane_u32::<0>(t488[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t489 as *mut u64).add(5), t490).write_unaligned(((vgetq_lane_u32::<1>(t488[3]) as u64) << 32 | vgetq_lane_u32::<1>(t488[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t489 as *mut u64).add(6), t490).write_unaligned(((vgetq_lane_u32::<2>(t488[3]) as u64) << 32 | vgetq_lane_u32::<2>(t488[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t489 as *mut u64).add(7), t490).write_unaligned(((vgetq_lane_u32::<3>(t488[3]) as u64) << 32 | vgetq_lane_u32::<3>(t488[1]) as u64));
            }
            let t491: [uint32x4_t; 2] = [x80[0], x80[1]];
            let t492: [uint32x4_t; 2] = [vandq_u32(t491[0], vdupq_n_u32(63)), vandq_u32(t491[1], vdupq_n_u32(63))];
            let t493: [uint32x4_t; 4] = [vshlq_u32(x56[0], vreinterpretq_s32_u32(t492[0])), vshlq_u32(x56[1], vreinterpretq_s32_u32(t492[1])), vorrq_u32(vshlq_u32(x56[2], vreinterpretq_s32_u32(t492[0])), vshlq_u32(x56[0], vsubq_s32(vreinterpretq_s32_u32(t492[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(x56[3], vreinterpretq_s32_u32(t492[1])), vshlq_u32(x56[1], vsubq_s32(vreinterpretq_s32_u32(t492[1]), vdupq_n_s32(32))))];
            let t494 = (arg3 as *const u8).add(((gid0.wrapping_add((2u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t494 as *mut u32).add(0), uint32x4x2_t(t493[0], t493[2]));
                vst2q_u32((t494 as *mut u32).add(8), uint32x4x2_t(t493[1], t493[3]));
            } else {
                let mut t495w = 0u64;
                let t495 = core::hint::black_box(&raw mut t495w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t494 as *mut u64).add(0), t495).write_unaligned(((vgetq_lane_u32::<0>(t493[2]) as u64) << 32 | vgetq_lane_u32::<0>(t493[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t494 as *mut u64).add(1), t495).write_unaligned(((vgetq_lane_u32::<1>(t493[2]) as u64) << 32 | vgetq_lane_u32::<1>(t493[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t494 as *mut u64).add(2), t495).write_unaligned(((vgetq_lane_u32::<2>(t493[2]) as u64) << 32 | vgetq_lane_u32::<2>(t493[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t494 as *mut u64).add(3), t495).write_unaligned(((vgetq_lane_u32::<3>(t493[2]) as u64) << 32 | vgetq_lane_u32::<3>(t493[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t494 as *mut u64).add(4), t495).write_unaligned(((vgetq_lane_u32::<0>(t493[3]) as u64) << 32 | vgetq_lane_u32::<0>(t493[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t494 as *mut u64).add(5), t495).write_unaligned(((vgetq_lane_u32::<1>(t493[3]) as u64) << 32 | vgetq_lane_u32::<1>(t493[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t494 as *mut u64).add(6), t495).write_unaligned(((vgetq_lane_u32::<2>(t493[3]) as u64) << 32 | vgetq_lane_u32::<2>(t493[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t494 as *mut u64).add(7), t495).write_unaligned(((vgetq_lane_u32::<3>(t493[3]) as u64) << 32 | vgetq_lane_u32::<3>(t493[1]) as u64));
            }
            let t496: [uint32x4_t; 2] = [x80[0], x80[1]];
            let t497: [uint32x4_t; 2] = [vandq_u32(t496[0], vdupq_n_u32(63)), vandq_u32(t496[1], vdupq_n_u32(63))];
            let t498: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(x56[0], vnegq_s32(vreinterpretq_s32_u32(t497[0]))), vshlq_u32(x56[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t497[0])))), vorrq_u32(vshlq_u32(x56[1], vnegq_s32(vreinterpretq_s32_u32(t497[1]))), vshlq_u32(x56[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t497[1])))), vshlq_u32(x56[2], vnegq_s32(vreinterpretq_s32_u32(t497[0]))), vshlq_u32(x56[3], vnegq_s32(vreinterpretq_s32_u32(t497[1])))];
            let t499 = (arg3 as *const u8).add(((gid0.wrapping_add((3u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t499 as *mut u32).add(0), uint32x4x2_t(t498[0], t498[2]));
                vst2q_u32((t499 as *mut u32).add(8), uint32x4x2_t(t498[1], t498[3]));
            } else {
                let mut t500w = 0u64;
                let t500 = core::hint::black_box(&raw mut t500w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t499 as *mut u64).add(0), t500).write_unaligned(((vgetq_lane_u32::<0>(t498[2]) as u64) << 32 | vgetq_lane_u32::<0>(t498[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t499 as *mut u64).add(1), t500).write_unaligned(((vgetq_lane_u32::<1>(t498[2]) as u64) << 32 | vgetq_lane_u32::<1>(t498[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t499 as *mut u64).add(2), t500).write_unaligned(((vgetq_lane_u32::<2>(t498[2]) as u64) << 32 | vgetq_lane_u32::<2>(t498[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t499 as *mut u64).add(3), t500).write_unaligned(((vgetq_lane_u32::<3>(t498[2]) as u64) << 32 | vgetq_lane_u32::<3>(t498[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t499 as *mut u64).add(4), t500).write_unaligned(((vgetq_lane_u32::<0>(t498[3]) as u64) << 32 | vgetq_lane_u32::<0>(t498[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t499 as *mut u64).add(5), t500).write_unaligned(((vgetq_lane_u32::<1>(t498[3]) as u64) << 32 | vgetq_lane_u32::<1>(t498[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t499 as *mut u64).add(6), t500).write_unaligned(((vgetq_lane_u32::<2>(t498[3]) as u64) << 32 | vgetq_lane_u32::<2>(t498[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t499 as *mut u64).add(7), t500).write_unaligned(((vgetq_lane_u32::<3>(t498[3]) as u64) << 32 | vgetq_lane_u32::<3>(t498[1]) as u64));
            }
            let t501: [uint32x4_t; 2] = [x80[0], x80[1]];
            let t502: [uint32x4_t; 2] = [vandq_u32(t501[0], vdupq_n_u32(63)), vandq_u32(t501[1], vdupq_n_u32(63))];
            let t503: [uint32x4_t; 2] = [vcgeq_s32(vreinterpretq_s32_u32(t502[0]), vdupq_n_s32(32)), vcgeq_s32(vreinterpretq_s32_u32(t502[1]), vdupq_n_s32(32))];
            let t504: [uint32x4_t; 4] = [vbslq_u32(t503[0], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x56[2]), vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t502[0])))), vorrq_u32(vshlq_u32(x56[0], vnegq_s32(vreinterpretq_s32_u32(t502[0]))), vshlq_u32(x56[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t502[0]))))), vbslq_u32(t503[1], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x56[3]), vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t502[1])))), vorrq_u32(vshlq_u32(x56[1], vnegq_s32(vreinterpretq_s32_u32(t502[1]))), vshlq_u32(x56[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t502[1]))))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x56[2]), vnegq_s32(vreinterpretq_s32_u32(t502[0])))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x56[3]), vnegq_s32(vreinterpretq_s32_u32(t502[1]))))];
            let t505 = (arg3 as *const u8).add(((gid0.wrapping_add((4u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t505 as *mut u32).add(0), uint32x4x2_t(t504[0], t504[2]));
                vst2q_u32((t505 as *mut u32).add(8), uint32x4x2_t(t504[1], t504[3]));
            } else {
                let mut t506w = 0u64;
                let t506 = core::hint::black_box(&raw mut t506w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t505 as *mut u64).add(0), t506).write_unaligned(((vgetq_lane_u32::<0>(t504[2]) as u64) << 32 | vgetq_lane_u32::<0>(t504[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t505 as *mut u64).add(1), t506).write_unaligned(((vgetq_lane_u32::<1>(t504[2]) as u64) << 32 | vgetq_lane_u32::<1>(t504[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t505 as *mut u64).add(2), t506).write_unaligned(((vgetq_lane_u32::<2>(t504[2]) as u64) << 32 | vgetq_lane_u32::<2>(t504[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t505 as *mut u64).add(3), t506).write_unaligned(((vgetq_lane_u32::<3>(t504[2]) as u64) << 32 | vgetq_lane_u32::<3>(t504[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t505 as *mut u64).add(4), t506).write_unaligned(((vgetq_lane_u32::<0>(t504[3]) as u64) << 32 | vgetq_lane_u32::<0>(t504[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t505 as *mut u64).add(5), t506).write_unaligned(((vgetq_lane_u32::<1>(t504[3]) as u64) << 32 | vgetq_lane_u32::<1>(t504[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t505 as *mut u64).add(6), t506).write_unaligned(((vgetq_lane_u32::<2>(t504[3]) as u64) << 32 | vgetq_lane_u32::<2>(t504[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t505 as *mut u64).add(7), t506).write_unaligned(((vgetq_lane_u32::<3>(t504[3]) as u64) << 32 | vgetq_lane_u32::<3>(t504[1]) as u64));
            }
            let t507: [uint32x4_t; 4] = [vshlq_n_u32::<13>(x56[0]), vshlq_n_u32::<13>(x56[1]), vorrq_u32(vshlq_n_u32::<13>(x56[2]), vshrq_n_u32::<19>(x56[0])), vorrq_u32(vshlq_n_u32::<13>(x56[3]), vshrq_n_u32::<19>(x56[1]))];
            let t508: [uint32x4_t; 2] = [vshrq_n_u32::<19>(x64[2]), vshrq_n_u32::<19>(x64[3])];
            let t509: [uint32x4_t; 4] = [vorrq_u32(t507[0], t508[0]), vorrq_u32(t507[1], t508[1]), t507[2], t507[3]];
            let t510 = (arg3 as *const u8).add(((gid0.wrapping_add((5u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t510 as *mut u32).add(0), uint32x4x2_t(t509[0], t509[2]));
                vst2q_u32((t510 as *mut u32).add(8), uint32x4x2_t(t509[1], t509[3]));
            } else {
                let mut t511w = 0u64;
                let t511 = core::hint::black_box(&raw mut t511w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t510 as *mut u64).add(0), t511).write_unaligned(((vgetq_lane_u32::<0>(t509[2]) as u64) << 32 | vgetq_lane_u32::<0>(t509[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t510 as *mut u64).add(1), t511).write_unaligned(((vgetq_lane_u32::<1>(t509[2]) as u64) << 32 | vgetq_lane_u32::<1>(t509[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t510 as *mut u64).add(2), t511).write_unaligned(((vgetq_lane_u32::<2>(t509[2]) as u64) << 32 | vgetq_lane_u32::<2>(t509[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t510 as *mut u64).add(3), t511).write_unaligned(((vgetq_lane_u32::<3>(t509[2]) as u64) << 32 | vgetq_lane_u32::<3>(t509[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t510 as *mut u64).add(4), t511).write_unaligned(((vgetq_lane_u32::<0>(t509[3]) as u64) << 32 | vgetq_lane_u32::<0>(t509[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t510 as *mut u64).add(5), t511).write_unaligned(((vgetq_lane_u32::<1>(t509[3]) as u64) << 32 | vgetq_lane_u32::<1>(t509[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t510 as *mut u64).add(6), t511).write_unaligned(((vgetq_lane_u32::<2>(t509[3]) as u64) << 32 | vgetq_lane_u32::<2>(t509[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t510 as *mut u64).add(7), t511).write_unaligned(((vgetq_lane_u32::<3>(t509[3]) as u64) << 32 | vgetq_lane_u32::<3>(t509[1]) as u64));
            }
            let t512: [uint32x4_t; 4] = [vdupq_n_u32(0), vdupq_n_u32(0), vshlq_n_u32::<8>(x56[0]), vshlq_n_u32::<8>(x56[1])];
            let t513 = (arg3 as *const u8).add(((gid0.wrapping_add((6u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t513 as *mut u32).add(0), uint32x4x2_t(t512[0], t512[2]));
                vst2q_u32((t513 as *mut u32).add(8), uint32x4x2_t(t512[1], t512[3]));
            } else {
                let mut t514w = 0u64;
                let t514 = core::hint::black_box(&raw mut t514w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t513 as *mut u64).add(0), t514).write_unaligned(((vgetq_lane_u32::<0>(t512[2]) as u64) << 32 | vgetq_lane_u32::<0>(t512[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t513 as *mut u64).add(1), t514).write_unaligned(((vgetq_lane_u32::<1>(t512[2]) as u64) << 32 | vgetq_lane_u32::<1>(t512[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t513 as *mut u64).add(2), t514).write_unaligned(((vgetq_lane_u32::<2>(t512[2]) as u64) << 32 | vgetq_lane_u32::<2>(t512[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t513 as *mut u64).add(3), t514).write_unaligned(((vgetq_lane_u32::<3>(t512[2]) as u64) << 32 | vgetq_lane_u32::<3>(t512[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t513 as *mut u64).add(4), t514).write_unaligned(((vgetq_lane_u32::<0>(t512[3]) as u64) << 32 | vgetq_lane_u32::<0>(t512[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t513 as *mut u64).add(5), t514).write_unaligned(((vgetq_lane_u32::<1>(t512[3]) as u64) << 32 | vgetq_lane_u32::<1>(t512[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t513 as *mut u64).add(6), t514).write_unaligned(((vgetq_lane_u32::<2>(t512[3]) as u64) << 32 | vgetq_lane_u32::<2>(t512[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t513 as *mut u64).add(7), t514).write_unaligned(((vgetq_lane_u32::<3>(t512[3]) as u64) << 32 | vgetq_lane_u32::<3>(t512[1]) as u64));
            }
            let t515: [uint32x4_t; 2] = [vshrq_n_u32::<3>(x56[2]), vshrq_n_u32::<3>(x56[3])];
            let t516: [uint32x4_t; 4] = [t515[0], t515[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            let t517 = (arg3 as *const u8).add(((gid0.wrapping_add((7u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t517 as *mut u32).add(0), uint32x4x2_t(t516[0], t516[2]));
                vst2q_u32((t517 as *mut u32).add(8), uint32x4x2_t(t516[1], t516[3]));
            } else {
                let mut t518w = 0u64;
                let t518 = core::hint::black_box(&raw mut t518w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t517 as *mut u64).add(0), t518).write_unaligned(((vgetq_lane_u32::<0>(t516[2]) as u64) << 32 | vgetq_lane_u32::<0>(t516[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t517 as *mut u64).add(1), t518).write_unaligned(((vgetq_lane_u32::<1>(t516[2]) as u64) << 32 | vgetq_lane_u32::<1>(t516[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t517 as *mut u64).add(2), t518).write_unaligned(((vgetq_lane_u32::<2>(t516[2]) as u64) << 32 | vgetq_lane_u32::<2>(t516[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t517 as *mut u64).add(3), t518).write_unaligned(((vgetq_lane_u32::<3>(t516[2]) as u64) << 32 | vgetq_lane_u32::<3>(t516[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t517 as *mut u64).add(4), t518).write_unaligned(((vgetq_lane_u32::<0>(t516[3]) as u64) << 32 | vgetq_lane_u32::<0>(t516[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t517 as *mut u64).add(5), t518).write_unaligned(((vgetq_lane_u32::<1>(t516[3]) as u64) << 32 | vgetq_lane_u32::<1>(t516[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t517 as *mut u64).add(6), t518).write_unaligned(((vgetq_lane_u32::<2>(t516[3]) as u64) << 32 | vgetq_lane_u32::<2>(t516[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t517 as *mut u64).add(7), t518).write_unaligned(((vgetq_lane_u32::<3>(t516[3]) as u64) << 32 | vgetq_lane_u32::<3>(t516[1]) as u64));
            }
            let t519 = vdupq_n_s32(((((40i64) as u32)) & 63) as i32);
            let t520: [uint32x4_t; 2] = [vcgeq_s32(t519, vdupq_n_s32(32)), vcgeq_s32(t519, vdupq_n_s32(32))];
            let t521: [uint32x4_t; 4] = [vbslq_u32(t520[0], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x56[2]), vsubq_s32(vdupq_n_s32(32), t519))), vorrq_u32(vshlq_u32(x56[0], vnegq_s32(t519)), vshlq_u32(x56[2], vsubq_s32(vdupq_n_s32(32), t519)))), vbslq_u32(t520[1], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x56[3]), vsubq_s32(vdupq_n_s32(32), t519))), vorrq_u32(vshlq_u32(x56[1], vnegq_s32(t519)), vshlq_u32(x56[3], vsubq_s32(vdupq_n_s32(32), t519)))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x56[2]), vnegq_s32(t519))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x56[3]), vnegq_s32(t519)))];
            let t522 = (arg3 as *const u8).add(((gid0.wrapping_add((8u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t522 as *mut u32).add(0), uint32x4x2_t(t521[0], t521[2]));
                vst2q_u32((t522 as *mut u32).add(8), uint32x4x2_t(t521[1], t521[3]));
            } else {
                let mut t523w = 0u64;
                let t523 = core::hint::black_box(&raw mut t523w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t522 as *mut u64).add(0), t523).write_unaligned(((vgetq_lane_u32::<0>(t521[2]) as u64) << 32 | vgetq_lane_u32::<0>(t521[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t522 as *mut u64).add(1), t523).write_unaligned(((vgetq_lane_u32::<1>(t521[2]) as u64) << 32 | vgetq_lane_u32::<1>(t521[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t522 as *mut u64).add(2), t523).write_unaligned(((vgetq_lane_u32::<2>(t521[2]) as u64) << 32 | vgetq_lane_u32::<2>(t521[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t522 as *mut u64).add(3), t523).write_unaligned(((vgetq_lane_u32::<3>(t521[2]) as u64) << 32 | vgetq_lane_u32::<3>(t521[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t522 as *mut u64).add(4), t523).write_unaligned(((vgetq_lane_u32::<0>(t521[3]) as u64) << 32 | vgetq_lane_u32::<0>(t521[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t522 as *mut u64).add(5), t523).write_unaligned(((vgetq_lane_u32::<1>(t521[3]) as u64) << 32 | vgetq_lane_u32::<1>(t521[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t522 as *mut u64).add(6), t523).write_unaligned(((vgetq_lane_u32::<2>(t521[3]) as u64) << 32 | vgetq_lane_u32::<2>(t521[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t522 as *mut u64).add(7), t523).write_unaligned(((vgetq_lane_u32::<3>(t521[3]) as u64) << 32 | vgetq_lane_u32::<3>(t521[1]) as u64));
            }
            let t524: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x56[0]), vget_low_u32(x64[0]))), vreinterpretq_u32_u64(vmull_high_u32(x56[0], x64[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x56[1]), vget_low_u32(x64[1]))), vreinterpretq_u32_u64(vmull_high_u32(x56[1], x64[1])))];
            let t525: [uint32x4_t; 4] = [vmulq_u32(x56[0], x64[0]), vmulq_u32(x56[1], x64[1]), vmlaq_u32(vmlaq_u32(t524[0], x56[0], x64[2]), x64[0], x56[2]), vmlaq_u32(vmlaq_u32(t524[1], x56[1], x64[3]), x64[1], x56[3])];
            let t526 = (arg3 as *const u8).add(((gid0.wrapping_add((9u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t526 as *mut u32).add(0), uint32x4x2_t(t525[0], t525[2]));
                vst2q_u32((t526 as *mut u32).add(8), uint32x4x2_t(t525[1], t525[3]));
            } else {
                let mut t527w = 0u64;
                let t527 = core::hint::black_box(&raw mut t527w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t526 as *mut u64).add(0), t527).write_unaligned(((vgetq_lane_u32::<0>(t525[2]) as u64) << 32 | vgetq_lane_u32::<0>(t525[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t526 as *mut u64).add(1), t527).write_unaligned(((vgetq_lane_u32::<1>(t525[2]) as u64) << 32 | vgetq_lane_u32::<1>(t525[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t526 as *mut u64).add(2), t527).write_unaligned(((vgetq_lane_u32::<2>(t525[2]) as u64) << 32 | vgetq_lane_u32::<2>(t525[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t526 as *mut u64).add(3), t527).write_unaligned(((vgetq_lane_u32::<3>(t525[2]) as u64) << 32 | vgetq_lane_u32::<3>(t525[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t526 as *mut u64).add(4), t527).write_unaligned(((vgetq_lane_u32::<0>(t525[3]) as u64) << 32 | vgetq_lane_u32::<0>(t525[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t526 as *mut u64).add(5), t527).write_unaligned(((vgetq_lane_u32::<1>(t525[3]) as u64) << 32 | vgetq_lane_u32::<1>(t525[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t526 as *mut u64).add(6), t527).write_unaligned(((vgetq_lane_u32::<2>(t525[3]) as u64) << 32 | vgetq_lane_u32::<2>(t525[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t526 as *mut u64).add(7), t527).write_unaligned(((vgetq_lane_u32::<3>(t525[3]) as u64) << 32 | vgetq_lane_u32::<3>(t525[1]) as u64));
            }
            let t528: u64 = 0u64;
            let t529 = [vdupq_n_u32((t528) as u32); 2];
            let t530: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x64[0], t529[0]), vceqq_u32(x64[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(x64[1], t529[1]), vceqq_u32(x64[3], vdupq_n_u32(0)))];
            let t531: [uint32x4_t; 2] = [vmvnq_u32(t530[0]), vmvnq_u32(t530[1])];
            let mut t532: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            let t533: [uint32x4_t; 2] = [vandq_u32(live[0], t531[0]), vandq_u32(live[1], t531[1])];
            let t534: [uint32x4_t; 2] = [vbicq_u32(live[0], t531[0]), vbicq_u32(live[1], t531[1])];
            {
                let (mut t535xl, mut t535xh, mut t535yl, mut t535yh) = ([0u32; 8], [0u32; 8], [0u32; 8], [0u32; 8]);
                vst1q_u32(t535xl.as_mut_ptr().add(0), x56[0]); vst1q_u32(t535xh.as_mut_ptr().add(0), x56[2]); vst1q_u32(t535yl.as_mut_ptr().add(0), x64[0]); vst1q_u32(t535yh.as_mut_ptr().add(0), x64[2]);
                vst1q_u32(t535xl.as_mut_ptr().add(4), x56[1]); vst1q_u32(t535xh.as_mut_ptr().add(4), x56[3]); vst1q_u32(t535yl.as_mut_ptr().add(4), x64[1]); vst1q_u32(t535yh.as_mut_ptr().add(4), x64[3]);
                for j in 0..8 { let q = (((t535xh[j] as u64) << 32 | t535xl[j] as u64) as u64).checked_div(((t535yh[j] as u64) << 32 | t535yl[j] as u64) as u64).unwrap_or(0) as u64; t535xl[j] = q as u32; t535xh[j] = (q >> 32) as u32; }
                let t536: [uint32x4_t; 4] = [vld1q_u32(t535xl.as_ptr().add(0)), vld1q_u32(t535xl.as_ptr().add(4)), vld1q_u32(t535xh.as_ptr().add(0)), vld1q_u32(t535xh.as_ptr().add(4))];
                t532 = [vbslq_u32(t533[0], t536[0], t532[0]), vbslq_u32(t533[1], t536[1], t532[1]), vbslq_u32(t533[0], t536[2], t532[2]), vbslq_u32(t533[1], t536[3], t532[3])];
            }
            {
                let t537: u64 = 7u64;
                let t538 = [vdupq_n_u32((t537) as u32); 2];
                let t539: [uint32x4_t; 4] = [t538[0], t538[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                t532 = [vbslq_u32(t534[0], t539[0], t532[0]), vbslq_u32(t534[1], t539[1], t532[1]), vbslq_u32(t534[0], t539[2], t532[2]), vbslq_u32(t534[1], t539[3], t532[3])];
            }
            let t540 = (arg3 as *const u8).add(((gid0.wrapping_add((10u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t540 as *mut u32).add(0), uint32x4x2_t(t532[0], t532[2]));
                vst2q_u32((t540 as *mut u32).add(8), uint32x4x2_t(t532[1], t532[3]));
            } else {
                let mut t541w = 0u64;
                let t541 = core::hint::black_box(&raw mut t541w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t540 as *mut u64).add(0), t541).write_unaligned(((vgetq_lane_u32::<0>(t532[2]) as u64) << 32 | vgetq_lane_u32::<0>(t532[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t540 as *mut u64).add(1), t541).write_unaligned(((vgetq_lane_u32::<1>(t532[2]) as u64) << 32 | vgetq_lane_u32::<1>(t532[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t540 as *mut u64).add(2), t541).write_unaligned(((vgetq_lane_u32::<2>(t532[2]) as u64) << 32 | vgetq_lane_u32::<2>(t532[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t540 as *mut u64).add(3), t541).write_unaligned(((vgetq_lane_u32::<3>(t532[2]) as u64) << 32 | vgetq_lane_u32::<3>(t532[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t540 as *mut u64).add(4), t541).write_unaligned(((vgetq_lane_u32::<0>(t532[3]) as u64) << 32 | vgetq_lane_u32::<0>(t532[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t540 as *mut u64).add(5), t541).write_unaligned(((vgetq_lane_u32::<1>(t532[3]) as u64) << 32 | vgetq_lane_u32::<1>(t532[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t540 as *mut u64).add(6), t541).write_unaligned(((vgetq_lane_u32::<2>(t532[3]) as u64) << 32 | vgetq_lane_u32::<2>(t532[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t540 as *mut u64).add(7), t541).write_unaligned(((vgetq_lane_u32::<3>(t532[3]) as u64) << 32 | vgetq_lane_u32::<3>(t532[1]) as u64));
            }
            let t542: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x56[2], x64[2]), vandq_u32(vceqq_u32(x56[2], x64[2]), vcltq_u32(x56[0], x64[0]))), vorrq_u32(vcltq_u32(x56[3], x64[3]), vandq_u32(vceqq_u32(x56[3], x64[3]), vcltq_u32(x56[1], x64[1])))];
            let t543: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t542[0]), vshrq_n_u32::<31>(t542[1])];
            let t544: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x64[2], x56[2]), vandq_u32(vceqq_u32(x64[2], x56[2]), vcltq_u32(x64[0], x56[0]))), vorrq_u32(vcltq_u32(x64[3], x56[3]), vandq_u32(vceqq_u32(x64[3], x56[3]), vcltq_u32(x64[1], x56[1])))];
            let t545: [uint32x4_t; 2] = [vmvnq_u32(t544[0]), vmvnq_u32(t544[1])];
            let t546: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t545[0]), vshrq_n_u32::<31>(t545[1])];
            let t547: [uint32x4_t; 4] = [vshlq_n_u32::<1>(t546[0]), vshlq_n_u32::<1>(t546[1]), vshrq_n_u32::<31>(t546[0]), vshrq_n_u32::<31>(t546[1])];
            let t548: [uint32x4_t; 4] = [vorrq_u32(t543[0], t547[0]), vorrq_u32(t543[1], t547[1]), t547[2], t547[3]];
            let t549: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x56[0], x64[0]), vceqq_u32(x56[2], x64[2])), vandq_u32(vceqq_u32(x56[1], x64[1]), vceqq_u32(x56[3], x64[3]))];
            let t550: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t549[0]), vshrq_n_u32::<31>(t549[1])];
            let t551: [uint32x4_t; 4] = [vshlq_n_u32::<2>(t550[0]), vshlq_n_u32::<2>(t550[1]), vshrq_n_u32::<30>(t550[0]), vshrq_n_u32::<30>(t550[1])];
            let t552: [uint32x4_t; 4] = [vorrq_u32(t548[0], t551[0]), vorrq_u32(t548[1], t551[1]), vorrq_u32(t548[2], t551[2]), vorrq_u32(t548[3], t551[3])];
            let t553: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x56[0], x64[0]), vceqq_u32(x56[2], x64[2])), vandq_u32(vceqq_u32(x56[1], x64[1]), vceqq_u32(x56[3], x64[3]))];
            let t554: [uint32x4_t; 2] = [vmvnq_u32(t553[0]), vmvnq_u32(t553[1])];
            let t555: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t554[0]), vshrq_n_u32::<31>(t554[1])];
            let t556: [uint32x4_t; 4] = [vshlq_n_u32::<3>(t555[0]), vshlq_n_u32::<3>(t555[1]), vshrq_n_u32::<29>(t555[0]), vshrq_n_u32::<29>(t555[1])];
            let t557: [uint32x4_t; 4] = [vorrq_u32(t552[0], t556[0]), vorrq_u32(t552[1], t556[1]), vorrq_u32(t552[2], t556[2]), vorrq_u32(t552[3], t556[3])];
            let t558: [uint32x4_t; 2] = [vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x56[2]), vreinterpretq_s32_u32(x64[2])), vandq_u32(vceqq_u32(x56[2], x64[2]), vcltq_u32(x56[0], x64[0]))), vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x56[3]), vreinterpretq_s32_u32(x64[3])), vandq_u32(vceqq_u32(x56[3], x64[3]), vcltq_u32(x56[1], x64[1])))];
            let t559: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t558[0]), vshrq_n_u32::<31>(t558[1])];
            let t560: [uint32x4_t; 4] = [vshlq_n_u32::<4>(t559[0]), vshlq_n_u32::<4>(t559[1]), vshrq_n_u32::<28>(t559[0]), vshrq_n_u32::<28>(t559[1])];
            let t561: [uint32x4_t; 4] = [vorrq_u32(t557[0], t560[0]), vorrq_u32(t557[1], t560[1]), vorrq_u32(t557[2], t560[2]), vorrq_u32(t557[3], t560[3])];
            let t562: [uint32x4_t; 2] = [vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x56[2]), vreinterpretq_s32_u32(x64[2])), vandq_u32(vceqq_u32(x56[2], x64[2]), vcltq_u32(x56[0], x64[0]))), vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x56[3]), vreinterpretq_s32_u32(x64[3])), vandq_u32(vceqq_u32(x56[3], x64[3]), vcltq_u32(x56[1], x64[1])))];
            let t563: [uint32x4_t; 2] = [vmvnq_u32(t562[0]), vmvnq_u32(t562[1])];
            let t564: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t563[0]), vshrq_n_u32::<31>(t563[1])];
            let t565: [uint32x4_t; 4] = [vshlq_n_u32::<5>(t564[0]), vshlq_n_u32::<5>(t564[1]), vshrq_n_u32::<27>(t564[0]), vshrq_n_u32::<27>(t564[1])];
            let t566: [uint32x4_t; 4] = [vorrq_u32(t561[0], t565[0]), vorrq_u32(t561[1], t565[1]), vorrq_u32(t561[2], t565[2]), vorrq_u32(t561[3], t565[3])];
            let t567: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x64[2], x56[2]), vandq_u32(vceqq_u32(x64[2], x56[2]), vcltq_u32(x64[0], x56[0]))), vorrq_u32(vcltq_u32(x64[3], x56[3]), vandq_u32(vceqq_u32(x64[3], x56[3]), vcltq_u32(x64[1], x56[1])))];
            let t568: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t567[0]), vshrq_n_u32::<31>(t567[1])];
            let t569: [uint32x4_t; 4] = [vshlq_n_u32::<6>(t568[0]), vshlq_n_u32::<6>(t568[1]), vshrq_n_u32::<26>(t568[0]), vshrq_n_u32::<26>(t568[1])];
            let t570: [uint32x4_t; 4] = [vorrq_u32(t566[0], t569[0]), vorrq_u32(t566[1], t569[1]), vorrq_u32(t566[2], t569[2]), vorrq_u32(t566[3], t569[3])];
            let t571 = (arg3 as *const u8).add(((gid0.wrapping_add((11u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t571 as *mut u32).add(0), uint32x4x2_t(t570[0], t570[2]));
                vst2q_u32((t571 as *mut u32).add(8), uint32x4x2_t(t570[1], t570[3]));
            } else {
                let mut t572w = 0u64;
                let t572 = core::hint::black_box(&raw mut t572w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t571 as *mut u64).add(0), t572).write_unaligned(((vgetq_lane_u32::<0>(t570[2]) as u64) << 32 | vgetq_lane_u32::<0>(t570[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t571 as *mut u64).add(1), t572).write_unaligned(((vgetq_lane_u32::<1>(t570[2]) as u64) << 32 | vgetq_lane_u32::<1>(t570[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t571 as *mut u64).add(2), t572).write_unaligned(((vgetq_lane_u32::<2>(t570[2]) as u64) << 32 | vgetq_lane_u32::<2>(t570[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t571 as *mut u64).add(3), t572).write_unaligned(((vgetq_lane_u32::<3>(t570[2]) as u64) << 32 | vgetq_lane_u32::<3>(t570[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t571 as *mut u64).add(4), t572).write_unaligned(((vgetq_lane_u32::<0>(t570[3]) as u64) << 32 | vgetq_lane_u32::<0>(t570[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t571 as *mut u64).add(5), t572).write_unaligned(((vgetq_lane_u32::<1>(t570[3]) as u64) << 32 | vgetq_lane_u32::<1>(t570[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t571 as *mut u64).add(6), t572).write_unaligned(((vgetq_lane_u32::<2>(t570[3]) as u64) << 32 | vgetq_lane_u32::<2>(t570[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t571 as *mut u64).add(7), t572).write_unaligned(((vgetq_lane_u32::<3>(t570[3]) as u64) << 32 | vgetq_lane_u32::<3>(t570[1]) as u64));
            }
            let t573: [uint32x4_t; 2] = [vaddq_u32(x72[0], x56[0]), vaddq_u32(x72[1], x56[1])];
            let t574: [uint32x4_t; 2] = [vcltq_u32(t573[0], x72[0]), vcltq_u32(t573[1], x72[1])];
            let t575: [uint32x4_t; 4] = [t573[0], t573[1], vsubq_u32(vaddq_u32(vdupq_n_u32(0), x56[2]), t574[0]), vsubq_u32(vaddq_u32(vdupq_n_u32(0), x56[3]), t574[1])];
            let t576 = (arg3 as *const u8).add(((gid0.wrapping_add((12u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t576 as *mut u32).add(0), uint32x4x2_t(t575[0], t575[2]));
                vst2q_u32((t576 as *mut u32).add(8), uint32x4x2_t(t575[1], t575[3]));
            } else {
                let mut t577w = 0u64;
                let t577 = core::hint::black_box(&raw mut t577w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t576 as *mut u64).add(0), t577).write_unaligned(((vgetq_lane_u32::<0>(t575[2]) as u64) << 32 | vgetq_lane_u32::<0>(t575[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t576 as *mut u64).add(1), t577).write_unaligned(((vgetq_lane_u32::<1>(t575[2]) as u64) << 32 | vgetq_lane_u32::<1>(t575[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t576 as *mut u64).add(2), t577).write_unaligned(((vgetq_lane_u32::<2>(t575[2]) as u64) << 32 | vgetq_lane_u32::<2>(t575[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t576 as *mut u64).add(3), t577).write_unaligned(((vgetq_lane_u32::<3>(t575[2]) as u64) << 32 | vgetq_lane_u32::<3>(t575[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t576 as *mut u64).add(4), t577).write_unaligned(((vgetq_lane_u32::<0>(t575[3]) as u64) << 32 | vgetq_lane_u32::<0>(t575[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t576 as *mut u64).add(5), t577).write_unaligned(((vgetq_lane_u32::<1>(t575[3]) as u64) << 32 | vgetq_lane_u32::<1>(t575[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t576 as *mut u64).add(6), t577).write_unaligned(((vgetq_lane_u32::<2>(t575[3]) as u64) << 32 | vgetq_lane_u32::<2>(t575[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t576 as *mut u64).add(7), t577).write_unaligned(((vgetq_lane_u32::<3>(t575[3]) as u64) << 32 | vgetq_lane_u32::<3>(t575[1]) as u64));
            }
            let t578: [uint32x4_t; 2] = [vreinterpretq_u32_s32(vshrq_n_s32::<31>(vreinterpretq_s32_u32(x72[0]))), vreinterpretq_u32_s32(vshrq_n_s32::<31>(vreinterpretq_s32_u32(x72[1])))];
            let t579: [uint32x4_t; 4] = [x72[0], x72[1], t578[0], t578[1]];
            let t580 = (arg3 as *const u8).add(((gid0.wrapping_add((13u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t580 as *mut u32).add(0), uint32x4x2_t(t579[0], t579[2]));
                vst2q_u32((t580 as *mut u32).add(8), uint32x4x2_t(t579[1], t579[3]));
            } else {
                let mut t581w = 0u64;
                let t581 = core::hint::black_box(&raw mut t581w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t580 as *mut u64).add(0), t581).write_unaligned(((vgetq_lane_u32::<0>(t579[2]) as u64) << 32 | vgetq_lane_u32::<0>(t579[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t580 as *mut u64).add(1), t581).write_unaligned(((vgetq_lane_u32::<1>(t579[2]) as u64) << 32 | vgetq_lane_u32::<1>(t579[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t580 as *mut u64).add(2), t581).write_unaligned(((vgetq_lane_u32::<2>(t579[2]) as u64) << 32 | vgetq_lane_u32::<2>(t579[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t580 as *mut u64).add(3), t581).write_unaligned(((vgetq_lane_u32::<3>(t579[2]) as u64) << 32 | vgetq_lane_u32::<3>(t579[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t580 as *mut u64).add(4), t581).write_unaligned(((vgetq_lane_u32::<0>(t579[3]) as u64) << 32 | vgetq_lane_u32::<0>(t579[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t580 as *mut u64).add(5), t581).write_unaligned(((vgetq_lane_u32::<1>(t579[3]) as u64) << 32 | vgetq_lane_u32::<1>(t579[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t580 as *mut u64).add(6), t581).write_unaligned(((vgetq_lane_u32::<2>(t579[3]) as u64) << 32 | vgetq_lane_u32::<2>(t579[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t580 as *mut u64).add(7), t581).write_unaligned(((vgetq_lane_u32::<3>(t579[3]) as u64) << 32 | vgetq_lane_u32::<3>(t579[1]) as u64));
            }
            let t582: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<3>(x56[0]), vshlq_n_u32::<29>(x56[2])), vorrq_u32(vshrq_n_u32::<3>(x56[1]), vshlq_n_u32::<29>(x56[3])), vshrq_n_u32::<3>(x56[2]), vshrq_n_u32::<3>(x56[3])];
            let t583: [uint32x4_t; 2] = [t582[0], t582[1]];
            let t584: [uint32x4_t; 4] = [t583[0], t583[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            let t585 = (arg3 as *const u8).add(((gid0.wrapping_add((14u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t585 as *mut u32).add(0), uint32x4x2_t(t584[0], t584[2]));
                vst2q_u32((t585 as *mut u32).add(8), uint32x4x2_t(t584[1], t584[3]));
            } else {
                let mut t586w = 0u64;
                let t586 = core::hint::black_box(&raw mut t586w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t585 as *mut u64).add(0), t586).write_unaligned(((vgetq_lane_u32::<0>(t584[2]) as u64) << 32 | vgetq_lane_u32::<0>(t584[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t585 as *mut u64).add(1), t586).write_unaligned(((vgetq_lane_u32::<1>(t584[2]) as u64) << 32 | vgetq_lane_u32::<1>(t584[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t585 as *mut u64).add(2), t586).write_unaligned(((vgetq_lane_u32::<2>(t584[2]) as u64) << 32 | vgetq_lane_u32::<2>(t584[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t585 as *mut u64).add(3), t586).write_unaligned(((vgetq_lane_u32::<3>(t584[2]) as u64) << 32 | vgetq_lane_u32::<3>(t584[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t585 as *mut u64).add(4), t586).write_unaligned(((vgetq_lane_u32::<0>(t584[3]) as u64) << 32 | vgetq_lane_u32::<0>(t584[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t585 as *mut u64).add(5), t586).write_unaligned(((vgetq_lane_u32::<1>(t584[3]) as u64) << 32 | vgetq_lane_u32::<1>(t584[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t585 as *mut u64).add(6), t586).write_unaligned(((vgetq_lane_u32::<2>(t584[3]) as u64) << 32 | vgetq_lane_u32::<2>(t584[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t585 as *mut u64).add(7), t586).write_unaligned(((vgetq_lane_u32::<3>(t584[3]) as u64) << 32 | vgetq_lane_u32::<3>(t584[1]) as u64));
            }
            let t587: u64 = 1u64;
            let t588 = [vdupq_n_u32((t587) as u32); 2];
            let t589: [uint32x4_t; 2] = [vandq_u32(x56[0], t588[0]), vandq_u32(x56[1], t588[1])];
            let t590: [uint32x4_t; 2] = [vtstq_u32(t589[0], t589[0]), vtstq_u32(t589[1], t589[1])];
            let t591: [uint32x4_t; 2] = [vandq_u32(live[0], t590[0]), vandq_u32(live[1], t590[1])];
            {
                let t592: [uint32x4_t; 4] = [veorq_u32(x56[0], x64[0]), veorq_u32(x56[1], x64[1]), veorq_u32(x56[2], x64[2]), veorq_u32(x56[3], x64[3])];
                x104 = [vbslq_u32(t591[0], t592[0], x104[0]), vbslq_u32(t591[1], t592[1], x104[1]), vbslq_u32(t591[0], t592[2], x104[2]), vbslq_u32(t591[1], t592[3], x104[3])];
            }
            let t593: [uint32x4_t; 2] = [vbicq_u32(live[0], t590[0]), vbicq_u32(live[1], t590[1])];
            {
                let t594: [uint32x4_t; 2] = [vaddq_u32(x56[0], x72[0]), vaddq_u32(x56[1], x72[1])];
                let t595: [uint32x4_t; 2] = [vcltq_u32(t594[0], x56[0]), vcltq_u32(t594[1], x56[1])];
                let t596: [uint32x4_t; 4] = [t594[0], t594[1], vsubq_u32(vaddq_u32(x56[2], vdupq_n_u32(0)), t595[0]), vsubq_u32(vaddq_u32(x56[3], vdupq_n_u32(0)), t595[1])];
                x104 = [vbslq_u32(t593[0], t596[0], x104[0]), vbslq_u32(t593[1], t596[1], x104[1]), vbslq_u32(t593[0], t596[2], x104[2]), vbslq_u32(t593[1], t596[3], x104[3])];
            }
            let t597 = (arg3 as *const u8).add(((gid0.wrapping_add((15u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t597 as *mut u32).add(0), uint32x4x2_t(x104[0], x104[2]));
                vst2q_u32((t597 as *mut u32).add(8), uint32x4x2_t(x104[1], x104[3]));
            } else {
                let mut t598w = 0u64;
                let t598 = core::hint::black_box(&raw mut t598w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t597 as *mut u64).add(0), t598).write_unaligned(((vgetq_lane_u32::<0>(x104[2]) as u64) << 32 | vgetq_lane_u32::<0>(x104[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t597 as *mut u64).add(1), t598).write_unaligned(((vgetq_lane_u32::<1>(x104[2]) as u64) << 32 | vgetq_lane_u32::<1>(x104[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t597 as *mut u64).add(2), t598).write_unaligned(((vgetq_lane_u32::<2>(x104[2]) as u64) << 32 | vgetq_lane_u32::<2>(x104[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t597 as *mut u64).add(3), t598).write_unaligned(((vgetq_lane_u32::<3>(x104[2]) as u64) << 32 | vgetq_lane_u32::<3>(x104[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t597 as *mut u64).add(4), t598).write_unaligned(((vgetq_lane_u32::<0>(x104[3]) as u64) << 32 | vgetq_lane_u32::<0>(x104[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t597 as *mut u64).add(5), t598).write_unaligned(((vgetq_lane_u32::<1>(x104[3]) as u64) << 32 | vgetq_lane_u32::<1>(x104[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t597 as *mut u64).add(6), t598).write_unaligned(((vgetq_lane_u32::<2>(x104[3]) as u64) << 32 | vgetq_lane_u32::<2>(x104[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t597 as *mut u64).add(7), t598).write_unaligned(((vgetq_lane_u32::<3>(x104[3]) as u64) << 32 | vgetq_lane_u32::<3>(x104[1]) as u64));
            }
            let t599 = b1_l_p(l, lid0, live, x56);
            let t600 = b2_l_p(l, lid0, live, x64);
            let t601: [uint32x4_t; 4] = [vshlq_n_u32::<8>(t600[0]), vshlq_n_u32::<8>(t600[1]), vshrq_n_u32::<24>(t600[0]), vshrq_n_u32::<24>(t600[1])];
            let t602: [uint32x4_t; 2] = [vaddq_u32(t599[0], t601[0]), vaddq_u32(t599[1], t601[1])];
            let t603: [uint32x4_t; 2] = [vcltq_u32(t602[0], t599[0]), vcltq_u32(t602[1], t599[1])];
            let t604: [uint32x4_t; 4] = [t602[0], t602[1], vsubq_u32(vaddq_u32(vdupq_n_u32(0), t601[2]), t603[0]), vsubq_u32(vaddq_u32(vdupq_n_u32(0), t601[3]), t603[1])];
            let t605 = (arg3 as *const u8).add(((gid0.wrapping_add((16u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t605 as *mut u32).add(0), uint32x4x2_t(t604[0], t604[2]));
                vst2q_u32((t605 as *mut u32).add(8), uint32x4x2_t(t604[1], t604[3]));
            } else {
                let mut t606w = 0u64;
                let t606 = core::hint::black_box(&raw mut t606w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t605 as *mut u64).add(0), t606).write_unaligned(((vgetq_lane_u32::<0>(t604[2]) as u64) << 32 | vgetq_lane_u32::<0>(t604[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t605 as *mut u64).add(1), t606).write_unaligned(((vgetq_lane_u32::<1>(t604[2]) as u64) << 32 | vgetq_lane_u32::<1>(t604[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t605 as *mut u64).add(2), t606).write_unaligned(((vgetq_lane_u32::<2>(t604[2]) as u64) << 32 | vgetq_lane_u32::<2>(t604[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t605 as *mut u64).add(3), t606).write_unaligned(((vgetq_lane_u32::<3>(t604[2]) as u64) << 32 | vgetq_lane_u32::<3>(t604[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t605 as *mut u64).add(4), t606).write_unaligned(((vgetq_lane_u32::<0>(t604[3]) as u64) << 32 | vgetq_lane_u32::<0>(t604[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t605 as *mut u64).add(5), t606).write_unaligned(((vgetq_lane_u32::<1>(t604[3]) as u64) << 32 | vgetq_lane_u32::<1>(t604[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t605 as *mut u64).add(6), t606).write_unaligned(((vgetq_lane_u32::<2>(t604[3]) as u64) << 32 | vgetq_lane_u32::<2>(t604[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t605 as *mut u64).add(7), t606).write_unaligned(((vgetq_lane_u32::<3>(t604[3]) as u64) << 32 | vgetq_lane_u32::<3>(t604[1]) as u64));
            }
            let t607 = [vdupq_n_u32(x88); 2];
            let mut t608x = [0u32; 8];
            let mut t608y = [0u32; 8];
            for r in 0..2 { vst1q_u32(t608x.as_mut_ptr().add(4 * r), x72[r]); vst1q_u32(t608y.as_mut_ptr().add(4 * r), t607[r]); }
            for j in 0..8 { t608x[j] = (t608x[j] as u32).checked_rem(t608y[j] as u32).unwrap_or(0) as u32; }
            let t609: [uint32x4_t; 2] = [vld1q_u32(t608x.as_ptr().add(0)), vld1q_u32(t608x.as_ptr().add(4))];
            let t610 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t609[0]), vandq_u32(live[1], t609[1]))));
            let t611: [uint32x4_t; 2] = [vbslq_u32(live[0], t609[0], t610), vbslq_u32(live[1], t609[1], t610)];
            let mut t612: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t611[0]) as usize * 8) as *const u64).read_unaligned(); t612[0] = vsetq_lane_u32::<0>(v as u32, t612[0]); t612[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t612[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t611[0]) as usize * 8) as *const u64).read_unaligned(); t612[0] = vsetq_lane_u32::<1>(v as u32, t612[0]); t612[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t612[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t611[0]) as usize * 8) as *const u64).read_unaligned(); t612[0] = vsetq_lane_u32::<2>(v as u32, t612[0]); t612[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t612[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t611[0]) as usize * 8) as *const u64).read_unaligned(); t612[0] = vsetq_lane_u32::<3>(v as u32, t612[0]); t612[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t612[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t611[1]) as usize * 8) as *const u64).read_unaligned(); t612[1] = vsetq_lane_u32::<0>(v as u32, t612[1]); t612[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t612[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t611[1]) as usize * 8) as *const u64).read_unaligned(); t612[1] = vsetq_lane_u32::<1>(v as u32, t612[1]); t612[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t612[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t611[1]) as usize * 8) as *const u64).read_unaligned(); t612[1] = vsetq_lane_u32::<2>(v as u32, t612[1]); t612[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t612[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t611[1]) as usize * 8) as *const u64).read_unaligned(); t612[1] = vsetq_lane_u32::<3>(v as u32, t612[1]); t612[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t612[3]); }
            let t613 = (arg3 as *const u8).add(((gid0.wrapping_add((17u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t613 as *mut u32).add(0), uint32x4x2_t(t612[0], t612[2]));
                vst2q_u32((t613 as *mut u32).add(8), uint32x4x2_t(t612[1], t612[3]));
            } else {
                let mut t614w = 0u64;
                let t614 = core::hint::black_box(&raw mut t614w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t613 as *mut u64).add(0), t614).write_unaligned(((vgetq_lane_u32::<0>(t612[2]) as u64) << 32 | vgetq_lane_u32::<0>(t612[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t613 as *mut u64).add(1), t614).write_unaligned(((vgetq_lane_u32::<1>(t612[2]) as u64) << 32 | vgetq_lane_u32::<1>(t612[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t613 as *mut u64).add(2), t614).write_unaligned(((vgetq_lane_u32::<2>(t612[2]) as u64) << 32 | vgetq_lane_u32::<2>(t612[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t613 as *mut u64).add(3), t614).write_unaligned(((vgetq_lane_u32::<3>(t612[2]) as u64) << 32 | vgetq_lane_u32::<3>(t612[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t613 as *mut u64).add(4), t614).write_unaligned(((vgetq_lane_u32::<0>(t612[3]) as u64) << 32 | vgetq_lane_u32::<0>(t612[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t613 as *mut u64).add(5), t614).write_unaligned(((vgetq_lane_u32::<1>(t612[3]) as u64) << 32 | vgetq_lane_u32::<1>(t612[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t613 as *mut u64).add(6), t614).write_unaligned(((vgetq_lane_u32::<2>(t612[3]) as u64) << 32 | vgetq_lane_u32::<2>(t612[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t613 as *mut u64).add(7), t614).write_unaligned(((vgetq_lane_u32::<3>(t612[3]) as u64) << 32 | vgetq_lane_u32::<3>(t612[1]) as u64));
            }
            let t615 = b3_ll_p(l, lid0, live, x56, x72);
            let t616 = (arg3 as *const u8).add(((gid0.wrapping_add((18u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t616 as *mut u32).add(0), uint32x4x2_t(t615[0], t615[2]));
                vst2q_u32((t616 as *mut u32).add(8), uint32x4x2_t(t615[1], t615[3]));
            } else {
                let mut t617w = 0u64;
                let t617 = core::hint::black_box(&raw mut t617w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t616 as *mut u64).add(0), t617).write_unaligned(((vgetq_lane_u32::<0>(t615[2]) as u64) << 32 | vgetq_lane_u32::<0>(t615[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t616 as *mut u64).add(1), t617).write_unaligned(((vgetq_lane_u32::<1>(t615[2]) as u64) << 32 | vgetq_lane_u32::<1>(t615[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t616 as *mut u64).add(2), t617).write_unaligned(((vgetq_lane_u32::<2>(t615[2]) as u64) << 32 | vgetq_lane_u32::<2>(t615[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t616 as *mut u64).add(3), t617).write_unaligned(((vgetq_lane_u32::<3>(t615[2]) as u64) << 32 | vgetq_lane_u32::<3>(t615[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t616 as *mut u64).add(4), t617).write_unaligned(((vgetq_lane_u32::<0>(t615[3]) as u64) << 32 | vgetq_lane_u32::<0>(t615[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t616 as *mut u64).add(5), t617).write_unaligned(((vgetq_lane_u32::<1>(t615[3]) as u64) << 32 | vgetq_lane_u32::<1>(t615[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t616 as *mut u64).add(6), t617).write_unaligned(((vgetq_lane_u32::<2>(t615[3]) as u64) << 32 | vgetq_lane_u32::<2>(t615[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t616 as *mut u64).add(7), t617).write_unaligned(((vgetq_lane_u32::<3>(t615[3]) as u64) << 32 | vgetq_lane_u32::<3>(t615[1]) as u64));
            }
            x112 = x56;
            x120 = 0u32;
            let mut t622 = live;
            loop {
                // Lanes part: each its own rounds.
                let t624 = [vdupq_n_u32(7u32); 2];
                let t625: [uint32x4_t; 2] = [vandq_u32(x72[0], t624[0]), vandq_u32(x72[1], t624[1])];
                let t626 = [vdupq_n_u32(x120); 2];
                let t627: [uint32x4_t; 2] = [vcltq_u32(t626[0], t625[0]), vcltq_u32(t626[1], t625[1])];
                t622 = [vandq_u32(t622[0], t627[0]), vandq_u32(t622[1], t627[1])];
                if !(any(&t622)) { break; }
                let t629: u64 = 3u64;
                let t630 = [vdupq_n_u32((t629) as u32); 2];
                let t631: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x112[0]), vget_low_u32(t630[0]))), vreinterpretq_u32_u64(vmull_high_u32(x112[0], t630[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x112[1]), vget_low_u32(t630[1]))), vreinterpretq_u32_u64(vmull_high_u32(x112[1], t630[1])))];
                let t632: [uint32x4_t; 4] = [vmulq_u32(x112[0], t630[0]), vmulq_u32(x112[1], t630[1]), vmlaq_u32(t631[0], t630[0], x112[2]), vmlaq_u32(t631[1], t630[1], x112[3])];
                let t633: [uint32x4_t; 2] = [vaddq_u32(t632[0], x64[0]), vaddq_u32(t632[1], x64[1])];
                let t634: [uint32x4_t; 2] = [vcltq_u32(t633[0], t632[0]), vcltq_u32(t633[1], t632[1])];
                let t635: [uint32x4_t; 4] = [t633[0], t633[1], vsubq_u32(vaddq_u32(t632[2], x64[2]), t634[0]), vsubq_u32(vaddq_u32(t632[3], x64[3]), t634[1])];
                x112 = [vbslq_u32(t622[0], t635[0], x112[0]), vbslq_u32(t622[1], t635[1], x112[1]), vbslq_u32(t622[0], t635[2], x112[2]), vbslq_u32(t622[1], t635[3], x112[3])];
                let t636: [uint32x4_t; 2] = [vandq_u32(t622[0], live[0]), vandq_u32(t622[1], live[1])];
                let t637 = x120;
                x120 = (x120).wrapping_add(1u32);
            }
            let t638 = (arg3 as *const u8).add(((gid0.wrapping_add((19u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t638 as *mut u32).add(0), uint32x4x2_t(x112[0], x112[2]));
                vst2q_u32((t638 as *mut u32).add(8), uint32x4x2_t(x112[1], x112[3]));
            } else {
                let mut t639w = 0u64;
                let t639 = core::hint::black_box(&raw mut t639w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t638 as *mut u64).add(0), t639).write_unaligned(((vgetq_lane_u32::<0>(x112[2]) as u64) << 32 | vgetq_lane_u32::<0>(x112[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t638 as *mut u64).add(1), t639).write_unaligned(((vgetq_lane_u32::<1>(x112[2]) as u64) << 32 | vgetq_lane_u32::<1>(x112[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t638 as *mut u64).add(2), t639).write_unaligned(((vgetq_lane_u32::<2>(x112[2]) as u64) << 32 | vgetq_lane_u32::<2>(x112[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t638 as *mut u64).add(3), t639).write_unaligned(((vgetq_lane_u32::<3>(x112[2]) as u64) << 32 | vgetq_lane_u32::<3>(x112[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t638 as *mut u64).add(4), t639).write_unaligned(((vgetq_lane_u32::<0>(x112[3]) as u64) << 32 | vgetq_lane_u32::<0>(x112[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t638 as *mut u64).add(5), t639).write_unaligned(((vgetq_lane_u32::<1>(x112[3]) as u64) << 32 | vgetq_lane_u32::<1>(x112[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t638 as *mut u64).add(6), t639).write_unaligned(((vgetq_lane_u32::<2>(x112[3]) as u64) << 32 | vgetq_lane_u32::<2>(x112[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t638 as *mut u64).add(7), t639).write_unaligned(((vgetq_lane_u32::<3>(x112[3]) as u64) << 32 | vgetq_lane_u32::<3>(x112[1]) as u64));
            }
            let t640: u64 = a1;
            let t641 = [vdupq_n_u32((t640) as u32); 2];
            let t642 = [vdupq_n_u32(((t640) >> 32) as u32); 2];
            let t643: [uint32x4_t; 2] = [vaddq_u32(x56[0], t641[0]), vaddq_u32(x56[1], t641[1])];
            let t644: [uint32x4_t; 2] = [vcltq_u32(t643[0], x56[0]), vcltq_u32(t643[1], x56[1])];
            let t645: [uint32x4_t; 4] = [t643[0], t643[1], vsubq_u32(vaddq_u32(x56[2], t642[0]), t644[0]), vsubq_u32(vaddq_u32(x56[3], t642[1]), t644[1])];
            let t646: u64 = (a1).wrapping_shl((3u64) as u32);
            let t647 = [vdupq_n_u32((t646) as u32); 2];
            let t648 = [vdupq_n_u32(((t646) >> 32) as u32); 2];
            let t649: [uint32x4_t; 2] = [vaddq_u32(t645[0], t647[0]), vaddq_u32(t645[1], t647[1])];
            let t650: [uint32x4_t; 2] = [vcltq_u32(t649[0], t645[0]), vcltq_u32(t649[1], t645[1])];
            let t651: [uint32x4_t; 4] = [t649[0], t649[1], vsubq_u32(vaddq_u32(t645[2], t648[0]), t650[0]), vsubq_u32(vaddq_u32(t645[3], t648[1]), t650[1])];
            let t652 = (arg3 as *const u8).add(((gid0.wrapping_add((20u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t652 as *mut u32).add(0), uint32x4x2_t(t651[0], t651[2]));
                vst2q_u32((t652 as *mut u32).add(8), uint32x4x2_t(t651[1], t651[3]));
            } else {
                let mut t653w = 0u64;
                let t653 = core::hint::black_box(&raw mut t653w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t652 as *mut u64).add(0), t653).write_unaligned(((vgetq_lane_u32::<0>(t651[2]) as u64) << 32 | vgetq_lane_u32::<0>(t651[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t652 as *mut u64).add(1), t653).write_unaligned(((vgetq_lane_u32::<1>(t651[2]) as u64) << 32 | vgetq_lane_u32::<1>(t651[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t652 as *mut u64).add(2), t653).write_unaligned(((vgetq_lane_u32::<2>(t651[2]) as u64) << 32 | vgetq_lane_u32::<2>(t651[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t652 as *mut u64).add(3), t653).write_unaligned(((vgetq_lane_u32::<3>(t651[2]) as u64) << 32 | vgetq_lane_u32::<3>(t651[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t652 as *mut u64).add(4), t653).write_unaligned(((vgetq_lane_u32::<0>(t651[3]) as u64) << 32 | vgetq_lane_u32::<0>(t651[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t652 as *mut u64).add(5), t653).write_unaligned(((vgetq_lane_u32::<1>(t651[3]) as u64) << 32 | vgetq_lane_u32::<1>(t651[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t652 as *mut u64).add(6), t653).write_unaligned(((vgetq_lane_u32::<2>(t651[3]) as u64) << 32 | vgetq_lane_u32::<2>(t651[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t652 as *mut u64).add(7), t653).write_unaligned(((vgetq_lane_u32::<3>(t651[3]) as u64) << 32 | vgetq_lane_u32::<3>(t651[1]) as u64));
            }
            let t654: u64 = 4294967295u64;
            let t655 = [vdupq_n_u32((t654) as u32); 2];
            let t656: [uint32x4_t; 2] = [vandq_u32(x56[0], t655[0]), vandq_u32(x56[1], t655[1])];
            let t657: u64 = 4294967295u64;
            let t658 = [vdupq_n_u32((t657) as u32); 2];
            let t659: [uint32x4_t; 2] = [vandq_u32(x64[0], t658[0]), vandq_u32(x64[1], t658[1])];
            let t660: [uint32x4_t; 2] = [vceqq_u32(t656[0], t659[0]), vceqq_u32(t656[1], t659[1])];
            let mut t661: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t662: [uint32x4_t; 2] = [vandq_u32(live[0], t660[0]), vandq_u32(live[1], t660[1])];
            let t663: [uint32x4_t; 2] = [vbicq_u32(live[0], t660[0]), vbicq_u32(live[1], t660[1])];
            {
                let t664 = [vdupq_n_u32(1u32); 2];
                t661 = [vbslq_u32(t662[0], t664[0], t661[0]), vbslq_u32(t662[1], t664[1], t661[1])];
            }
            {
                let t665 = [vdupq_n_u32(2u32); 2];
                t661 = [vbslq_u32(t663[0], t665[0], t661[0]), vbslq_u32(t663[1], t665[1], t661[1])];
            }
            let t666: [uint32x4_t; 4] = [t661[0], t661[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            let t667 = (arg3 as *const u8).add(((gid0.wrapping_add((21u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t667 as *mut u32).add(0), uint32x4x2_t(t666[0], t666[2]));
                vst2q_u32((t667 as *mut u32).add(8), uint32x4x2_t(t666[1], t666[3]));
            } else {
                let mut t668w = 0u64;
                let t668 = core::hint::black_box(&raw mut t668w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t667 as *mut u64).add(0), t668).write_unaligned(((vgetq_lane_u32::<0>(t666[2]) as u64) << 32 | vgetq_lane_u32::<0>(t666[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t667 as *mut u64).add(1), t668).write_unaligned(((vgetq_lane_u32::<1>(t666[2]) as u64) << 32 | vgetq_lane_u32::<1>(t666[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t667 as *mut u64).add(2), t668).write_unaligned(((vgetq_lane_u32::<2>(t666[2]) as u64) << 32 | vgetq_lane_u32::<2>(t666[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t667 as *mut u64).add(3), t668).write_unaligned(((vgetq_lane_u32::<3>(t666[2]) as u64) << 32 | vgetq_lane_u32::<3>(t666[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t667 as *mut u64).add(4), t668).write_unaligned(((vgetq_lane_u32::<0>(t666[3]) as u64) << 32 | vgetq_lane_u32::<0>(t666[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t667 as *mut u64).add(5), t668).write_unaligned(((vgetq_lane_u32::<1>(t666[3]) as u64) << 32 | vgetq_lane_u32::<1>(t666[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t667 as *mut u64).add(6), t668).write_unaligned(((vgetq_lane_u32::<2>(t666[3]) as u64) << 32 | vgetq_lane_u32::<2>(t666[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t667 as *mut u64).add(7), t668).write_unaligned(((vgetq_lane_u32::<3>(t666[3]) as u64) << 32 | vgetq_lane_u32::<3>(t666[1]) as u64));
            }
            let t669: [uint32x4_t; 2] = [vtstq_u32(vorrq_u32(x56[0], x56[2]), vorrq_u32(x56[0], x56[2])), vtstq_u32(vorrq_u32(x56[1], x56[3]), vorrq_u32(x56[1], x56[3]))];
            let mut t670: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t671: [uint32x4_t; 2] = [vandq_u32(live[0], t669[0]), vandq_u32(live[1], t669[1])];
            let t672: [uint32x4_t; 2] = [vbicq_u32(live[0], t669[0]), vbicq_u32(live[1], t669[1])];
            {
                let t673 = [vdupq_n_u32(5u32); 2];
                t670 = [vbslq_u32(t671[0], t673[0], t670[0]), vbslq_u32(t671[1], t673[1], t670[1])];
            }
            {
                let t674 = [vdupq_n_u32(6u32); 2];
                t670 = [vbslq_u32(t672[0], t674[0], t670[0]), vbslq_u32(t672[1], t674[1], t670[1])];
            }
            let t675: [uint32x4_t; 4] = [t670[0], t670[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            let t676 = (arg3 as *const u8).add(((gid0.wrapping_add((22u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t676 as *mut u32).add(0), uint32x4x2_t(t675[0], t675[2]));
                vst2q_u32((t676 as *mut u32).add(8), uint32x4x2_t(t675[1], t675[3]));
            } else {
                let mut t677w = 0u64;
                let t677 = core::hint::black_box(&raw mut t677w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t676 as *mut u64).add(0), t677).write_unaligned(((vgetq_lane_u32::<0>(t675[2]) as u64) << 32 | vgetq_lane_u32::<0>(t675[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t676 as *mut u64).add(1), t677).write_unaligned(((vgetq_lane_u32::<1>(t675[2]) as u64) << 32 | vgetq_lane_u32::<1>(t675[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t676 as *mut u64).add(2), t677).write_unaligned(((vgetq_lane_u32::<2>(t675[2]) as u64) << 32 | vgetq_lane_u32::<2>(t675[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t676 as *mut u64).add(3), t677).write_unaligned(((vgetq_lane_u32::<3>(t675[2]) as u64) << 32 | vgetq_lane_u32::<3>(t675[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t676 as *mut u64).add(4), t677).write_unaligned(((vgetq_lane_u32::<0>(t675[3]) as u64) << 32 | vgetq_lane_u32::<0>(t675[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t676 as *mut u64).add(5), t677).write_unaligned(((vgetq_lane_u32::<1>(t675[3]) as u64) << 32 | vgetq_lane_u32::<1>(t675[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t676 as *mut u64).add(6), t677).write_unaligned(((vgetq_lane_u32::<2>(t675[3]) as u64) << 32 | vgetq_lane_u32::<2>(t675[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t676 as *mut u64).add(7), t677).write_unaligned(((vgetq_lane_u32::<3>(t675[3]) as u64) << 32 | vgetq_lane_u32::<3>(t675[1]) as u64));
            }
            let t678 = b4_ll_p(l, lid0, live, x56, x64);
            let t679 = (arg3 as *const u8).add(((gid0.wrapping_add((23u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t679 as *mut u32).add(0), uint32x4x2_t(t678[0], t678[2]));
                vst2q_u32((t679 as *mut u32).add(8), uint32x4x2_t(t678[1], t678[3]));
            } else {
                let mut t680w = 0u64;
                let t680 = core::hint::black_box(&raw mut t680w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t679 as *mut u64).add(0), t680).write_unaligned(((vgetq_lane_u32::<0>(t678[2]) as u64) << 32 | vgetq_lane_u32::<0>(t678[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t679 as *mut u64).add(1), t680).write_unaligned(((vgetq_lane_u32::<1>(t678[2]) as u64) << 32 | vgetq_lane_u32::<1>(t678[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t679 as *mut u64).add(2), t680).write_unaligned(((vgetq_lane_u32::<2>(t678[2]) as u64) << 32 | vgetq_lane_u32::<2>(t678[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t679 as *mut u64).add(3), t680).write_unaligned(((vgetq_lane_u32::<3>(t678[2]) as u64) << 32 | vgetq_lane_u32::<3>(t678[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t679 as *mut u64).add(4), t680).write_unaligned(((vgetq_lane_u32::<0>(t678[3]) as u64) << 32 | vgetq_lane_u32::<0>(t678[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t679 as *mut u64).add(5), t680).write_unaligned(((vgetq_lane_u32::<1>(t678[3]) as u64) << 32 | vgetq_lane_u32::<1>(t678[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t679 as *mut u64).add(6), t680).write_unaligned(((vgetq_lane_u32::<2>(t678[3]) as u64) << 32 | vgetq_lane_u32::<2>(t678[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t679 as *mut u64).add(7), t680).write_unaligned(((vgetq_lane_u32::<3>(t678[3]) as u64) << 32 | vgetq_lane_u32::<3>(t678[1]) as u64));
            }
            let t681 = b5_r0ns_p(l, lid0, live, arg0, gid0.wrapping_mul(x96), x96, 8u32);
            let t682 = (arg3 as *const u8).add(((gid0.wrapping_add((24u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t682 as *mut u32).add(0), uint32x4x2_t(t681[0], t681[2]));
                vst2q_u32((t682 as *mut u32).add(8), uint32x4x2_t(t681[1], t681[3]));
            } else {
                let mut t683w = 0u64;
                let t683 = core::hint::black_box(&raw mut t683w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t682 as *mut u64).add(0), t683).write_unaligned(((vgetq_lane_u32::<0>(t681[2]) as u64) << 32 | vgetq_lane_u32::<0>(t681[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t682 as *mut u64).add(1), t683).write_unaligned(((vgetq_lane_u32::<1>(t681[2]) as u64) << 32 | vgetq_lane_u32::<1>(t681[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t682 as *mut u64).add(2), t683).write_unaligned(((vgetq_lane_u32::<2>(t681[2]) as u64) << 32 | vgetq_lane_u32::<2>(t681[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t682 as *mut u64).add(3), t683).write_unaligned(((vgetq_lane_u32::<3>(t681[2]) as u64) << 32 | vgetq_lane_u32::<3>(t681[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t682 as *mut u64).add(4), t683).write_unaligned(((vgetq_lane_u32::<0>(t681[3]) as u64) << 32 | vgetq_lane_u32::<0>(t681[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t682 as *mut u64).add(5), t683).write_unaligned(((vgetq_lane_u32::<1>(t681[3]) as u64) << 32 | vgetq_lane_u32::<1>(t681[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t682 as *mut u64).add(6), t683).write_unaligned(((vgetq_lane_u32::<2>(t681[3]) as u64) << 32 | vgetq_lane_u32::<2>(t681[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t682 as *mut u64).add(7), t683).write_unaligned(((vgetq_lane_u32::<3>(t681[3]) as u64) << 32 | vgetq_lane_u32::<3>(t681[1]) as u64));
            }
            let t684 = if (gid0.wrapping_add(3u32) & 255u32) <= 248u32 { b6_u8_p(l, lid0, live, gid0.wrapping_add(3u32)) } else { b6_i_p(l, lid0, live, gid0.wrapping_add(3u32)) };
            let t685 = (arg3 as *const u8).add(((gid0.wrapping_add((25u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t685 as *mut u32).add(0), uint32x4x2_t(t684[0], t684[2]));
                vst2q_u32((t685 as *mut u32).add(8), uint32x4x2_t(t684[1], t684[3]));
            } else {
                let mut t686w = 0u64;
                let t686 = core::hint::black_box(&raw mut t686w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t685 as *mut u64).add(0), t686).write_unaligned(((vgetq_lane_u32::<0>(t684[2]) as u64) << 32 | vgetq_lane_u32::<0>(t684[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t685 as *mut u64).add(1), t686).write_unaligned(((vgetq_lane_u32::<1>(t684[2]) as u64) << 32 | vgetq_lane_u32::<1>(t684[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t685 as *mut u64).add(2), t686).write_unaligned(((vgetq_lane_u32::<2>(t684[2]) as u64) << 32 | vgetq_lane_u32::<2>(t684[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t685 as *mut u64).add(3), t686).write_unaligned(((vgetq_lane_u32::<3>(t684[2]) as u64) << 32 | vgetq_lane_u32::<3>(t684[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t685 as *mut u64).add(4), t686).write_unaligned(((vgetq_lane_u32::<0>(t684[3]) as u64) << 32 | vgetq_lane_u32::<0>(t684[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t685 as *mut u64).add(5), t686).write_unaligned(((vgetq_lane_u32::<1>(t684[3]) as u64) << 32 | vgetq_lane_u32::<1>(t684[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t685 as *mut u64).add(6), t686).write_unaligned(((vgetq_lane_u32::<2>(t684[3]) as u64) << 32 | vgetq_lane_u32::<2>(t684[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t685 as *mut u64).add(7), t686).write_unaligned(((vgetq_lane_u32::<3>(t684[3]) as u64) << 32 | vgetq_lane_u32::<3>(t684[1]) as u64));
            }
            let t687: u64 = 3u64;
            let t688 = [vdupq_n_u32((t687) as u32); 2];
            let t689: [uint32x4_t; 2] = [vandq_u32(x56[0], t688[0]), vandq_u32(x56[1], t688[1])];
            let t690: u64 = 0u64;
            let t691 = [vdupq_n_u32((t690) as u32); 2];
            let t692: [uint32x4_t; 2] = [vceqq_u32(t689[0], t691[0]), vceqq_u32(t689[1], t691[1])];
            let mut t693: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            let t694: [uint32x4_t; 2] = [vandq_u32(live[0], t692[0]), vandq_u32(live[1], t692[1])];
            let t695: [uint32x4_t; 2] = [vbicq_u32(live[0], t692[0]), vbicq_u32(live[1], t692[1])];
            if any(&t694) {
                let t696: [uint32x4_t; 4];
                let t697 = bits(&t694);
                if t697.count_ones() <= 2 {
                    let t698 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x64);
                    let t699 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x72);
                    let mut t700 = [0u32; 16];
                    let mut t701 = t697;
                    while t701 != 0 {
                        let j = t701.trailing_zeros() as usize;
                        t701 &= t701 - 1;
                        let t702 = b7_ss_f(l, lid0, ((t698[8 + j] as u64) << 32 | t698[j] as u64), t699[j]);
                        let t703: u64 = t702;
                        t700[j] = t703 as u32;
                        t700[8 + j] = (t703 >> 32) as u32;
                    }
                    t696 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t700);
                } else {
                    let t704 = b7_ll_p(l, lid0, t694, x64, x72);
                    t696 = t704;
                }
                t693 = [vbslq_u32(t694[0], t696[0], t693[0]), vbslq_u32(t694[1], t696[1], t693[1]), vbslq_u32(t694[0], t696[2], t693[2]), vbslq_u32(t694[1], t696[3], t693[3])];
            }
            {
                t693 = [vbslq_u32(t695[0], x56[0], t693[0]), vbslq_u32(t695[1], x56[1], t693[1]), vbslq_u32(t695[0], x56[2], t693[2]), vbslq_u32(t695[1], x56[3], t693[3])];
            }
            let t705 = (arg3 as *const u8).add(((gid0.wrapping_add((26u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t705 as *mut u32).add(0), uint32x4x2_t(t693[0], t693[2]));
                vst2q_u32((t705 as *mut u32).add(8), uint32x4x2_t(t693[1], t693[3]));
            } else {
                let mut t706w = 0u64;
                let t706 = core::hint::black_box(&raw mut t706w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t705 as *mut u64).add(0), t706).write_unaligned(((vgetq_lane_u32::<0>(t693[2]) as u64) << 32 | vgetq_lane_u32::<0>(t693[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t705 as *mut u64).add(1), t706).write_unaligned(((vgetq_lane_u32::<1>(t693[2]) as u64) << 32 | vgetq_lane_u32::<1>(t693[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t705 as *mut u64).add(2), t706).write_unaligned(((vgetq_lane_u32::<2>(t693[2]) as u64) << 32 | vgetq_lane_u32::<2>(t693[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t705 as *mut u64).add(3), t706).write_unaligned(((vgetq_lane_u32::<3>(t693[2]) as u64) << 32 | vgetq_lane_u32::<3>(t693[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t705 as *mut u64).add(4), t706).write_unaligned(((vgetq_lane_u32::<0>(t693[3]) as u64) << 32 | vgetq_lane_u32::<0>(t693[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t705 as *mut u64).add(5), t706).write_unaligned(((vgetq_lane_u32::<1>(t693[3]) as u64) << 32 | vgetq_lane_u32::<1>(t693[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t705 as *mut u64).add(6), t706).write_unaligned(((vgetq_lane_u32::<2>(t693[3]) as u64) << 32 | vgetq_lane_u32::<2>(t693[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t705 as *mut u64).add(7), t706).write_unaligned(((vgetq_lane_u32::<3>(t693[3]) as u64) << 32 | vgetq_lane_u32::<3>(t693[1]) as u64));
            }
        }
    }
}

#[inline(always)]
unsafe fn b1_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t707: [uint32x4_t; 2] = [vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[0])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[2]))))), vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[1])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[3])))))];
    res = t707;
    return res;
}

#[inline(always)]
unsafe fn b1_l_f(l: &Launch, lid0: u32, x0: [uint32x4_t; 4]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t708: [uint32x4_t; 2] = [vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[0])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[2]))))), vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[1])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[3])))))];
    res = t708;
    return res;
}

#[inline(always)]
unsafe fn b2_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t709: [uint32x4_t; 2] = [vbslq_u32(vceqzq_u32(x0[2]), vaddq_u32(vclzq_u32(x0[0]), vdupq_n_u32(32)), vclzq_u32(x0[2])), vbslq_u32(vceqzq_u32(x0[3]), vaddq_u32(vclzq_u32(x0[1]), vdupq_n_u32(32)), vclzq_u32(x0[3]))];
    res = t709;
    return res;
}

#[inline(always)]
unsafe fn b2_l_f(l: &Launch, lid0: u32, x0: [uint32x4_t; 4]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t710: [uint32x4_t; 2] = [vbslq_u32(vceqzq_u32(x0[2]), vaddq_u32(vclzq_u32(x0[0]), vdupq_n_u32(32)), vclzq_u32(x0[2])), vbslq_u32(vceqzq_u32(x0[3]), vaddq_u32(vclzq_u32(x0[1]), vdupq_n_u32(32)), vclzq_u32(x0[3]))];
    res = t710;
    return res;
}

#[inline(always)]
unsafe fn b3_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], x8: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x16: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t711: u64 = 1u64;
    let t712 = [vdupq_n_u32((t711) as u32); 2];
    let t713: [uint32x4_t; 2] = [vandq_u32(x0[0], t712[0]), vandq_u32(x0[1], t712[1])];
    let t714: u64 = 0u64;
    let t715 = [vdupq_n_u32((t714) as u32); 2];
    let t716: [uint32x4_t; 2] = [vceqq_u32(t713[0], t715[0]), vceqq_u32(t713[1], t715[1])];
    let t717: [uint32x4_t; 2] = [vandq_u32(live[0], t716[0]), vandq_u32(live[1], t716[1])];
    {
        let t718: [uint32x4_t; 4] = [x8[0], x8[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = [vbslq_u32(t717[0], t718[0], res[0]), vbslq_u32(t717[1], t718[1], res[1]), vbslq_u32(t717[0], t718[2], res[2]), vbslq_u32(t717[1], t718[3], res[3])];
        live = [vbicq_u32(live[0], t717[0]), vbicq_u32(live[1], t717[1])];
    }
    if any(&live) {
        let t719: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x0[0]), vshlq_n_u32::<30>(x0[2])), vorrq_u32(vshrq_n_u32::<2>(x0[1]), vshlq_n_u32::<30>(x0[3])), vshrq_n_u32::<2>(x0[2]), vshrq_n_u32::<2>(x0[3])];
        let t720: [uint32x4_t; 2] = [t719[0], t719[1]];
        x16 = t720;
        let t721 = [vdupq_n_u32(1u32); 2];
        let t722: [uint32x4_t; 2] = [vsubq_u32(x16[0], t721[0]), vsubq_u32(x16[1], t721[1])];
        let t723: [uint32x4_t; 2] = [vcgeq_u32(t722[0], x8[0]), vcgeq_u32(t722[1], x8[1])];
        let t724: [uint32x4_t; 2] = [vandq_u32(live[0], t723[0]), vandq_u32(live[1], t723[1])];
        {
            let t725: u64 = 4294967295u64;
            let t726 = [vdupq_n_u32((t725) as u32); 2];
            let t727: [uint32x4_t; 4] = [t726[0], t726[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t724[0], t727[0], res[0]), vbslq_u32(t724[1], t727[1], res[1]), vbslq_u32(t724[0], t727[2], res[2]), vbslq_u32(t724[1], t727[3], res[3])];
            live = [vbicq_u32(live[0], t724[0]), vbicq_u32(live[1], t724[1])];
        }
        if any(&live) {
            let t728: u64 = 2u64;
            let t729 = [vdupq_n_u32((t728) as u32); 2];
            let t730: [uint32x4_t; 2] = [vandq_u32(x0[0], t729[0]), vandq_u32(x0[1], t729[1])];
            let t731: u64 = 0u64;
            let t732 = [vdupq_n_u32((t731) as u32); 2];
            let t733: [uint32x4_t; 2] = [vceqq_u32(t730[0], t732[0]), vceqq_u32(t730[1], t732[1])];
            let t734: [uint32x4_t; 2] = [vmvnq_u32(t733[0]), vmvnq_u32(t733[1])];
            let t735: [uint32x4_t; 2] = [vandq_u32(live[0], t734[0]), vandq_u32(live[1], t734[1])];
            {
                let t736: [uint32x4_t; 2] = [vsubq_u32(x8[0], x16[0]), vsubq_u32(x8[1], x16[1])];
                let t737: [uint32x4_t; 4] = [t736[0], t736[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t735[0], t737[0], res[0]), vbslq_u32(t735[1], t737[1], res[1]), vbslq_u32(t735[0], t737[2], res[2]), vbslq_u32(t735[1], t737[3], res[3])];
                live = [vbicq_u32(live[0], t735[0]), vbicq_u32(live[1], t735[1])];
            }
            if any(&live) {
                let t738: [uint32x4_t; 2] = [vsubq_u32(x8[0], x16[0]), vsubq_u32(x8[1], x16[1])];
                let t739: u64 = (1u64).wrapping_shl((32u64) as u32);
                let t740 = [vdupq_n_u32((t739) as u32); 2];
                let t741 = [vdupq_n_u32(((t739) >> 32) as u32); 2];
                let t742: [uint32x4_t; 4] = [vorrq_u32(t740[0], t738[0]), vorrq_u32(t740[1], t738[1]), t741[0], t741[1]];
                res = [vbslq_u32(live[0], t742[0], res[0]), vbslq_u32(live[1], t742[1], res[1]), vbslq_u32(live[0], t742[2], res[2]), vbslq_u32(live[1], t742[3], res[3])];
                live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
            }
        }
    }
    res
}

#[inline(always)]
unsafe fn b3_ll_f(l: &Launch, lid0: u32, x0: [uint32x4_t; 4], x8: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x16: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t743: u64 = 1u64;
    let t744 = [vdupq_n_u32((t743) as u32); 2];
    let t745: [uint32x4_t; 2] = [vandq_u32(x0[0], t744[0]), vandq_u32(x0[1], t744[1])];
    let t746: u64 = 0u64;
    let t747 = [vdupq_n_u32((t746) as u32); 2];
    let t748: [uint32x4_t; 2] = [vceqq_u32(t745[0], t747[0]), vceqq_u32(t745[1], t747[1])];
    {
        let t749: [uint32x4_t; 4] = [x8[0], x8[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = [vbslq_u32(t748[0], t749[0], res[0]), vbslq_u32(t748[1], t749[1], res[1]), vbslq_u32(t748[0], t749[2], res[2]), vbslq_u32(t748[1], t749[3], res[3])];
        live = [vbicq_u32(live[0], t748[0]), vbicq_u32(live[1], t748[1])];
    }
    if any(&live) {
        let t750: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x0[0]), vshlq_n_u32::<30>(x0[2])), vorrq_u32(vshrq_n_u32::<2>(x0[1]), vshlq_n_u32::<30>(x0[3])), vshrq_n_u32::<2>(x0[2]), vshrq_n_u32::<2>(x0[3])];
        let t751: [uint32x4_t; 2] = [t750[0], t750[1]];
        x16 = t751;
        let t752 = [vdupq_n_u32(1u32); 2];
        let t753: [uint32x4_t; 2] = [vsubq_u32(x16[0], t752[0]), vsubq_u32(x16[1], t752[1])];
        let t754: [uint32x4_t; 2] = [vcgeq_u32(t753[0], x8[0]), vcgeq_u32(t753[1], x8[1])];
        let t755: [uint32x4_t; 2] = [vandq_u32(live[0], t754[0]), vandq_u32(live[1], t754[1])];
        {
            let t756: u64 = 4294967295u64;
            let t757 = [vdupq_n_u32((t756) as u32); 2];
            let t758: [uint32x4_t; 4] = [t757[0], t757[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t755[0], t758[0], res[0]), vbslq_u32(t755[1], t758[1], res[1]), vbslq_u32(t755[0], t758[2], res[2]), vbslq_u32(t755[1], t758[3], res[3])];
            live = [vbicq_u32(live[0], t755[0]), vbicq_u32(live[1], t755[1])];
        }
        if any(&live) {
            let t759: u64 = 2u64;
            let t760 = [vdupq_n_u32((t759) as u32); 2];
            let t761: [uint32x4_t; 2] = [vandq_u32(x0[0], t760[0]), vandq_u32(x0[1], t760[1])];
            let t762: u64 = 0u64;
            let t763 = [vdupq_n_u32((t762) as u32); 2];
            let t764: [uint32x4_t; 2] = [vceqq_u32(t761[0], t763[0]), vceqq_u32(t761[1], t763[1])];
            let t765: [uint32x4_t; 2] = [vmvnq_u32(t764[0]), vmvnq_u32(t764[1])];
            let t766: [uint32x4_t; 2] = [vandq_u32(live[0], t765[0]), vandq_u32(live[1], t765[1])];
            {
                let t767: [uint32x4_t; 2] = [vsubq_u32(x8[0], x16[0]), vsubq_u32(x8[1], x16[1])];
                let t768: [uint32x4_t; 4] = [t767[0], t767[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t766[0], t768[0], res[0]), vbslq_u32(t766[1], t768[1], res[1]), vbslq_u32(t766[0], t768[2], res[2]), vbslq_u32(t766[1], t768[3], res[3])];
                live = [vbicq_u32(live[0], t766[0]), vbicq_u32(live[1], t766[1])];
            }
            if any(&live) {
                let t769: [uint32x4_t; 2] = [vsubq_u32(x8[0], x16[0]), vsubq_u32(x8[1], x16[1])];
                let t770: u64 = (1u64).wrapping_shl((32u64) as u32);
                let t771 = [vdupq_n_u32((t770) as u32); 2];
                let t772 = [vdupq_n_u32(((t770) >> 32) as u32); 2];
                let t773: [uint32x4_t; 4] = [vorrq_u32(t771[0], t769[0]), vorrq_u32(t771[1], t769[1]), t772[0], t772[1]];
                res = [vbslq_u32(live[0], t773[0], res[0]), vbslq_u32(live[1], t773[1], res[1]), vbslq_u32(live[0], t773[2], res[2]), vbslq_u32(live[1], t773[3], res[3])];
                live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
            }
        }
    }
    res
}

#[inline(always)]
unsafe fn b4_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], x8: [uint32x4_t; 4]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let (mut t774l, mut t774h) = ([0u32; 8], [0u32; 8]);
    { let v = (((((vgetq_lane_u32::<0>(x0[2]) as u64) << 32 | vgetq_lane_u32::<0>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<0>(x8[2]) as u64) << 32 | vgetq_lane_u32::<0>(x8[0]) as u64) as u64) as u128) >> 64) as u64; t774l[0] = v as u32; t774h[0] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<1>(x0[2]) as u64) << 32 | vgetq_lane_u32::<1>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<1>(x8[2]) as u64) << 32 | vgetq_lane_u32::<1>(x8[0]) as u64) as u64) as u128) >> 64) as u64; t774l[1] = v as u32; t774h[1] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<2>(x0[2]) as u64) << 32 | vgetq_lane_u32::<2>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<2>(x8[2]) as u64) << 32 | vgetq_lane_u32::<2>(x8[0]) as u64) as u64) as u128) >> 64) as u64; t774l[2] = v as u32; t774h[2] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<3>(x0[2]) as u64) << 32 | vgetq_lane_u32::<3>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<3>(x8[2]) as u64) << 32 | vgetq_lane_u32::<3>(x8[0]) as u64) as u64) as u128) >> 64) as u64; t774l[3] = v as u32; t774h[3] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<0>(x0[3]) as u64) << 32 | vgetq_lane_u32::<0>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<0>(x8[3]) as u64) << 32 | vgetq_lane_u32::<0>(x8[1]) as u64) as u64) as u128) >> 64) as u64; t774l[4] = v as u32; t774h[4] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<1>(x0[3]) as u64) << 32 | vgetq_lane_u32::<1>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<1>(x8[3]) as u64) << 32 | vgetq_lane_u32::<1>(x8[1]) as u64) as u64) as u128) >> 64) as u64; t774l[5] = v as u32; t774h[5] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<2>(x0[3]) as u64) << 32 | vgetq_lane_u32::<2>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<2>(x8[3]) as u64) << 32 | vgetq_lane_u32::<2>(x8[1]) as u64) as u64) as u128) >> 64) as u64; t774l[6] = v as u32; t774h[6] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<3>(x0[3]) as u64) << 32 | vgetq_lane_u32::<3>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<3>(x8[3]) as u64) << 32 | vgetq_lane_u32::<3>(x8[1]) as u64) as u64) as u128) >> 64) as u64; t774l[7] = v as u32; t774h[7] = (v >> 32) as u32; }
    let t775: [uint32x4_t; 4] = [vld1q_u32(t774l.as_ptr().add(0)), vld1q_u32(t774l.as_ptr().add(4)), vld1q_u32(t774h.as_ptr().add(0)), vld1q_u32(t774h.as_ptr().add(4))];
    res = t775;
    return res;
}

#[inline(always)]
unsafe fn b4_ll_f(l: &Launch, lid0: u32, x0: [uint32x4_t; 4], x8: [uint32x4_t; 4]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let (mut t776l, mut t776h) = ([0u32; 8], [0u32; 8]);
    { let v = (((((vgetq_lane_u32::<0>(x0[2]) as u64) << 32 | vgetq_lane_u32::<0>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<0>(x8[2]) as u64) << 32 | vgetq_lane_u32::<0>(x8[0]) as u64) as u64) as u128) >> 64) as u64; t776l[0] = v as u32; t776h[0] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<1>(x0[2]) as u64) << 32 | vgetq_lane_u32::<1>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<1>(x8[2]) as u64) << 32 | vgetq_lane_u32::<1>(x8[0]) as u64) as u64) as u128) >> 64) as u64; t776l[1] = v as u32; t776h[1] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<2>(x0[2]) as u64) << 32 | vgetq_lane_u32::<2>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<2>(x8[2]) as u64) << 32 | vgetq_lane_u32::<2>(x8[0]) as u64) as u64) as u128) >> 64) as u64; t776l[2] = v as u32; t776h[2] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<3>(x0[2]) as u64) << 32 | vgetq_lane_u32::<3>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<3>(x8[2]) as u64) << 32 | vgetq_lane_u32::<3>(x8[0]) as u64) as u64) as u128) >> 64) as u64; t776l[3] = v as u32; t776h[3] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<0>(x0[3]) as u64) << 32 | vgetq_lane_u32::<0>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<0>(x8[3]) as u64) << 32 | vgetq_lane_u32::<0>(x8[1]) as u64) as u64) as u128) >> 64) as u64; t776l[4] = v as u32; t776h[4] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<1>(x0[3]) as u64) << 32 | vgetq_lane_u32::<1>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<1>(x8[3]) as u64) << 32 | vgetq_lane_u32::<1>(x8[1]) as u64) as u64) as u128) >> 64) as u64; t776l[5] = v as u32; t776h[5] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<2>(x0[3]) as u64) << 32 | vgetq_lane_u32::<2>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<2>(x8[3]) as u64) << 32 | vgetq_lane_u32::<2>(x8[1]) as u64) as u64) as u128) >> 64) as u64; t776l[6] = v as u32; t776h[6] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<3>(x0[3]) as u64) << 32 | vgetq_lane_u32::<3>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<3>(x8[3]) as u64) << 32 | vgetq_lane_u32::<3>(x8[1]) as u64) as u64) as u128) >> 64) as u64; t776l[7] = v as u32; t776h[7] = (v >> 32) as u32; }
    let t777: [uint32x4_t; 4] = [vld1q_u32(t776l.as_ptr().add(0)), vld1q_u32(t776l.as_ptr().add(4)), vld1q_u32(t776h.as_ptr().add(0)), vld1q_u32(t776h.as_ptr().add(4))];
    res = t777;
    return res;
}

#[inline(always)]
unsafe fn b5_r0ns_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8f: u32, x8s: u32, x16: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if ((x16) == (8u32)) {
        let t778: [uint32x4_t; 4];
        if (x8s) == 8u32 && ((x8f) as u64) + 7 * ((x8s) as u64) <= u32::MAX as u64 && all(&live) {
            let t779 = (arg0 as *const u8).add(((x8f) as usize) * 1);
            let t7800 = vld2q_u32((t779 as *const u32).add(0));
            let t7801 = vld2q_u32((t779 as *const u32).add(8));
            let t780: [uint32x4_t; 4] = [t7800.0, t7801.0, t7800.1, t7801.1];
            t778 = t780;
        } else {
            let t781: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8s)), vmlaq_u32(vdupq_n_u32(x8f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8s))];
            let t782 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t781[0]), vandq_u32(live[1], t781[1]))));
            let t783: [uint32x4_t; 2] = [vbslq_u32(live[0], t781[0], t782), vbslq_u32(live[1], t781[1], t782)];
            let mut t784: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t783[0]) as usize * 1) as *const u64).read_unaligned(); t784[0] = vsetq_lane_u32::<0>(v as u32, t784[0]); t784[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t784[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t783[0]) as usize * 1) as *const u64).read_unaligned(); t784[0] = vsetq_lane_u32::<1>(v as u32, t784[0]); t784[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t784[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t783[0]) as usize * 1) as *const u64).read_unaligned(); t784[0] = vsetq_lane_u32::<2>(v as u32, t784[0]); t784[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t784[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t783[0]) as usize * 1) as *const u64).read_unaligned(); t784[0] = vsetq_lane_u32::<3>(v as u32, t784[0]); t784[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t784[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t783[1]) as usize * 1) as *const u64).read_unaligned(); t784[1] = vsetq_lane_u32::<0>(v as u32, t784[1]); t784[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t784[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t783[1]) as usize * 1) as *const u64).read_unaligned(); t784[1] = vsetq_lane_u32::<1>(v as u32, t784[1]); t784[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t784[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t783[1]) as usize * 1) as *const u64).read_unaligned(); t784[1] = vsetq_lane_u32::<2>(v as u32, t784[1]); t784[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t784[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t783[1]) as usize * 1) as *const u64).read_unaligned(); t784[1] = vsetq_lane_u32::<3>(v as u32, t784[1]); t784[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t784[3]); }
            t778 = t784;
        }
        res = t778;
        return res;
    }
    let t785: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8s)), vmlaq_u32(vdupq_n_u32(x8f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8s))];
    let t786 = [vdupq_n_u32(!(7u32)); 2];
    let t787: [uint32x4_t; 2] = [vandq_u32(t785[0], t786[0]), vandq_u32(t785[1], t786[1])];
    let t788: [uint32x4_t; 2] = [t787[0], t787[1]];
    let t789: u64 = ((arg0) as usize as u64);
    let t790 = [vdupq_n_u32((t789) as u32); 2];
    let t791 = [vdupq_n_u32(((t789) >> 32) as u32); 2];
    let t792: [uint32x4_t; 2] = [vaddq_u32(t790[0], t788[0]), vaddq_u32(t790[1], t788[1])];
    let t793: [uint32x4_t; 2] = [vcltq_u32(t792[0], t790[0]), vcltq_u32(t792[1], t790[1])];
    let t794: [uint32x4_t; 4] = [t792[0], t792[1], vsubq_u32(vaddq_u32(t791[0], vdupq_n_u32(0)), t793[0]), vsubq_u32(vaddq_u32(t791[1], vdupq_n_u32(0)), t793[1])];
    let t795: [uint32x4_t; 4] = [t794[0], t794[1], t794[2], t794[3]];
    x24 = t795;
    let t796: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8s)), vmlaq_u32(vdupq_n_u32(x8f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8s))];
    let t797 = [vdupq_n_u32(7u32); 2];
    let t798: [uint32x4_t; 2] = [vandq_u32(t796[0], t797[0]), vandq_u32(t796[1], t797[1])];
    let t799 = [vdupq_n_u32(8u32); 2];
    let t800: [uint32x4_t; 2] = [vmulq_u32(t798[0], t799[0]), vmulq_u32(t798[1], t799[1])];
    x32 = t800;
    let mut t801: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x24[2]) as u64) << 32 | vgetq_lane_u32::<0>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t801[0] = vsetq_lane_u32::<0>(v as u32, t801[0]); t801[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t801[2]); }
    { let v = (((vgetq_lane_u32::<1>(x24[2]) as u64) << 32 | vgetq_lane_u32::<1>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t801[0] = vsetq_lane_u32::<1>(v as u32, t801[0]); t801[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t801[2]); }
    { let v = (((vgetq_lane_u32::<2>(x24[2]) as u64) << 32 | vgetq_lane_u32::<2>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t801[0] = vsetq_lane_u32::<2>(v as u32, t801[0]); t801[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t801[2]); }
    { let v = (((vgetq_lane_u32::<3>(x24[2]) as u64) << 32 | vgetq_lane_u32::<3>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t801[0] = vsetq_lane_u32::<3>(v as u32, t801[0]); t801[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t801[2]); }
    { let v = (((vgetq_lane_u32::<0>(x24[3]) as u64) << 32 | vgetq_lane_u32::<0>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t801[1] = vsetq_lane_u32::<0>(v as u32, t801[1]); t801[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t801[3]); }
    { let v = (((vgetq_lane_u32::<1>(x24[3]) as u64) << 32 | vgetq_lane_u32::<1>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t801[1] = vsetq_lane_u32::<1>(v as u32, t801[1]); t801[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t801[3]); }
    { let v = (((vgetq_lane_u32::<2>(x24[3]) as u64) << 32 | vgetq_lane_u32::<2>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t801[1] = vsetq_lane_u32::<2>(v as u32, t801[1]); t801[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t801[3]); }
    { let v = (((vgetq_lane_u32::<3>(x24[3]) as u64) << 32 | vgetq_lane_u32::<3>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t801[1] = vsetq_lane_u32::<3>(v as u32, t801[1]); t801[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t801[3]); }
    let t802: [uint32x4_t; 2] = [x32[0], x32[1]];
    let t803: [uint32x4_t; 2] = [vandq_u32(t802[0], vdupq_n_u32(63)), vandq_u32(t802[1], vdupq_n_u32(63))];
    let t804: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t801[0], vnegq_s32(vreinterpretq_s32_u32(t803[0]))), vshlq_u32(t801[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t803[0])))), vorrq_u32(vshlq_u32(t801[1], vnegq_s32(vreinterpretq_s32_u32(t803[1]))), vshlq_u32(t801[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t803[1])))), vshlq_u32(t801[2], vnegq_s32(vreinterpretq_s32_u32(t803[0]))), vshlq_u32(t801[3], vnegq_s32(vreinterpretq_s32_u32(t803[1])))];
    x40 = t804;
    let t805 = [vdupq_n_u32(0u32); 2];
    let t806: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t805[0])), vmvnq_u32(vceqq_u32(x32[1], t805[1]))];
    let t807 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t808: [uint32x4_t; 2] = [vaddq_u32(x32[0], t807[0]), vaddq_u32(x32[1], t807[1])];
    let t809 = [vdupq_n_u32(64u32); 2];
    let t810: [uint32x4_t; 2] = [vcgtq_u32(t808[0], t809[0]), vcgtq_u32(t808[1], t809[1])];
    let t811: [uint32x4_t; 2] = [vandq_u32(t806[0], t810[0]), vandq_u32(t806[1], t810[1])];
    let mut t812: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t813: [uint32x4_t; 2] = [vandq_u32(live[0], t811[0]), vandq_u32(live[1], t811[1])];
    let t814: [uint32x4_t; 2] = [vbicq_u32(live[0], t811[0]), vbicq_u32(live[1], t811[1])];
    if any(&t813) {
        let t815: u64 = (1u64).wrapping_mul(8u64);
        let t816 = [vdupq_n_u32((t815) as u32); 2];
        let t817 = [vdupq_n_u32(((t815) >> 32) as u32); 2];
        let t818: [uint32x4_t; 2] = [vaddq_u32(x24[0], t816[0]), vaddq_u32(x24[1], t816[1])];
        let t819: [uint32x4_t; 2] = [vcltq_u32(t818[0], x24[0]), vcltq_u32(t818[1], x24[1])];
        let t820: [uint32x4_t; 4] = [t818[0], t818[1], vsubq_u32(vaddq_u32(x24[2], t817[0]), t819[0]), vsubq_u32(vaddq_u32(x24[3], t817[1]), t819[1])];
        let mut t821: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t820[2]) as u64) << 32 | vgetq_lane_u32::<0>(t820[0]) as u64) as usize as *const u64).read_unaligned(); t821[0] = vsetq_lane_u32::<0>(v as u32, t821[0]); t821[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t821[2]); }
        { let v = (((vgetq_lane_u32::<1>(t820[2]) as u64) << 32 | vgetq_lane_u32::<1>(t820[0]) as u64) as usize as *const u64).read_unaligned(); t821[0] = vsetq_lane_u32::<1>(v as u32, t821[0]); t821[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t821[2]); }
        { let v = (((vgetq_lane_u32::<2>(t820[2]) as u64) << 32 | vgetq_lane_u32::<2>(t820[0]) as u64) as usize as *const u64).read_unaligned(); t821[0] = vsetq_lane_u32::<2>(v as u32, t821[0]); t821[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t821[2]); }
        { let v = (((vgetq_lane_u32::<3>(t820[2]) as u64) << 32 | vgetq_lane_u32::<3>(t820[0]) as u64) as usize as *const u64).read_unaligned(); t821[0] = vsetq_lane_u32::<3>(v as u32, t821[0]); t821[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t821[2]); }
        { let v = (((vgetq_lane_u32::<0>(t820[3]) as u64) << 32 | vgetq_lane_u32::<0>(t820[1]) as u64) as usize as *const u64).read_unaligned(); t821[1] = vsetq_lane_u32::<0>(v as u32, t821[1]); t821[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t821[3]); }
        { let v = (((vgetq_lane_u32::<1>(t820[3]) as u64) << 32 | vgetq_lane_u32::<1>(t820[1]) as u64) as usize as *const u64).read_unaligned(); t821[1] = vsetq_lane_u32::<1>(v as u32, t821[1]); t821[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t821[3]); }
        { let v = (((vgetq_lane_u32::<2>(t820[3]) as u64) << 32 | vgetq_lane_u32::<2>(t820[1]) as u64) as usize as *const u64).read_unaligned(); t821[1] = vsetq_lane_u32::<2>(v as u32, t821[1]); t821[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t821[3]); }
        { let v = (((vgetq_lane_u32::<3>(t820[3]) as u64) << 32 | vgetq_lane_u32::<3>(t820[1]) as u64) as usize as *const u64).read_unaligned(); t821[1] = vsetq_lane_u32::<3>(v as u32, t821[1]); t821[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t821[3]); }
        let t822: [uint32x4_t; 2] = [vsubq_u32(t809[0], x32[0]), vsubq_u32(t809[1], x32[1])];
        let t823: [uint32x4_t; 2] = [t822[0], t822[1]];
        let t824: [uint32x4_t; 2] = [vandq_u32(t823[0], vdupq_n_u32(63)), vandq_u32(t823[1], vdupq_n_u32(63))];
        let t825: [uint32x4_t; 4] = [vshlq_u32(t821[0], vreinterpretq_s32_u32(t824[0])), vshlq_u32(t821[1], vreinterpretq_s32_u32(t824[1])), vorrq_u32(vshlq_u32(t821[2], vreinterpretq_s32_u32(t824[0])), vshlq_u32(t821[0], vsubq_s32(vreinterpretq_s32_u32(t824[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t821[3], vreinterpretq_s32_u32(t824[1])), vshlq_u32(t821[1], vsubq_s32(vreinterpretq_s32_u32(t824[1]), vdupq_n_s32(32))))];
        let t826: [uint32x4_t; 4] = [vorrq_u32(x40[0], t825[0]), vorrq_u32(x40[1], t825[1]), vorrq_u32(x40[2], t825[2]), vorrq_u32(x40[3], t825[3])];
        t812 = [vbslq_u32(t813[0], t826[0], t812[0]), vbslq_u32(t813[1], t826[1], t812[1]), vbslq_u32(t813[0], t826[2], t812[2]), vbslq_u32(t813[1], t826[3], t812[3])];
    }
    {
        t812 = [vbslq_u32(t814[0], x40[0], t812[0]), vbslq_u32(t814[1], x40[1], t812[1]), vbslq_u32(t814[0], x40[2], t812[2]), vbslq_u32(t814[1], x40[3], t812[3])];
    }
    res = t812;
    return res;
}

#[inline(always)]
unsafe fn b5_r0ns_f_v0(l: &Launch, lid0: u32, arg0: *const u8, x8f: u32, x8s: u32, x16: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if ((x16) == (8u32)) {
        let t827: [uint32x4_t; 4];
        if (x8s) == 8u32 && ((x8f) as u64) + 7 * ((x8s) as u64) <= u32::MAX as u64 {
            let t828 = (arg0 as *const u8).add(((x8f) as usize) * 1);
            let t8290 = vld2q_u32((t828 as *const u32).add(0));
            let t8291 = vld2q_u32((t828 as *const u32).add(8));
            let t829: [uint32x4_t; 4] = [t8290.0, t8291.0, t8290.1, t8291.1];
            t827 = t829;
        } else {
            let t830: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8s)), vmlaq_u32(vdupq_n_u32(x8f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8s))];
            let mut t831: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t830[0]) as usize * 1) as *const u64).read_unaligned(); t831[0] = vsetq_lane_u32::<0>(v as u32, t831[0]); t831[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t831[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t830[0]) as usize * 1) as *const u64).read_unaligned(); t831[0] = vsetq_lane_u32::<1>(v as u32, t831[0]); t831[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t831[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t830[0]) as usize * 1) as *const u64).read_unaligned(); t831[0] = vsetq_lane_u32::<2>(v as u32, t831[0]); t831[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t831[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t830[0]) as usize * 1) as *const u64).read_unaligned(); t831[0] = vsetq_lane_u32::<3>(v as u32, t831[0]); t831[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t831[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t830[1]) as usize * 1) as *const u64).read_unaligned(); t831[1] = vsetq_lane_u32::<0>(v as u32, t831[1]); t831[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t831[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t830[1]) as usize * 1) as *const u64).read_unaligned(); t831[1] = vsetq_lane_u32::<1>(v as u32, t831[1]); t831[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t831[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t830[1]) as usize * 1) as *const u64).read_unaligned(); t831[1] = vsetq_lane_u32::<2>(v as u32, t831[1]); t831[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t831[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t830[1]) as usize * 1) as *const u64).read_unaligned(); t831[1] = vsetq_lane_u32::<3>(v as u32, t831[1]); t831[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t831[3]); }
            t827 = t831;
        }
        res = t827;
        return res;
    }
    let t832: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8s)), vmlaq_u32(vdupq_n_u32(x8f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8s))];
    let t833 = [vdupq_n_u32(!(7u32)); 2];
    let t834: [uint32x4_t; 2] = [vandq_u32(t832[0], t833[0]), vandq_u32(t832[1], t833[1])];
    let t835: [uint32x4_t; 2] = [t834[0], t834[1]];
    let t836: u64 = ((arg0) as usize as u64);
    let t837 = [vdupq_n_u32((t836) as u32); 2];
    let t838 = [vdupq_n_u32(((t836) >> 32) as u32); 2];
    let t839: [uint32x4_t; 2] = [vaddq_u32(t837[0], t835[0]), vaddq_u32(t837[1], t835[1])];
    let t840: [uint32x4_t; 2] = [vcltq_u32(t839[0], t837[0]), vcltq_u32(t839[1], t837[1])];
    let t841: [uint32x4_t; 4] = [t839[0], t839[1], vsubq_u32(vaddq_u32(t838[0], vdupq_n_u32(0)), t840[0]), vsubq_u32(vaddq_u32(t838[1], vdupq_n_u32(0)), t840[1])];
    let t842: [uint32x4_t; 4] = [t841[0], t841[1], t841[2], t841[3]];
    x24 = t842;
    let t843: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8s)), vmlaq_u32(vdupq_n_u32(x8f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8s))];
    let t844 = [vdupq_n_u32(7u32); 2];
    let t845: [uint32x4_t; 2] = [vandq_u32(t843[0], t844[0]), vandq_u32(t843[1], t844[1])];
    let t846 = [vdupq_n_u32(8u32); 2];
    let t847: [uint32x4_t; 2] = [vmulq_u32(t845[0], t846[0]), vmulq_u32(t845[1], t846[1])];
    x32 = t847;
    let mut t848: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x24[2]) as u64) << 32 | vgetq_lane_u32::<0>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t848[0] = vsetq_lane_u32::<0>(v as u32, t848[0]); t848[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t848[2]); }
    { let v = (((vgetq_lane_u32::<1>(x24[2]) as u64) << 32 | vgetq_lane_u32::<1>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t848[0] = vsetq_lane_u32::<1>(v as u32, t848[0]); t848[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t848[2]); }
    { let v = (((vgetq_lane_u32::<2>(x24[2]) as u64) << 32 | vgetq_lane_u32::<2>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t848[0] = vsetq_lane_u32::<2>(v as u32, t848[0]); t848[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t848[2]); }
    { let v = (((vgetq_lane_u32::<3>(x24[2]) as u64) << 32 | vgetq_lane_u32::<3>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t848[0] = vsetq_lane_u32::<3>(v as u32, t848[0]); t848[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t848[2]); }
    { let v = (((vgetq_lane_u32::<0>(x24[3]) as u64) << 32 | vgetq_lane_u32::<0>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t848[1] = vsetq_lane_u32::<0>(v as u32, t848[1]); t848[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t848[3]); }
    { let v = (((vgetq_lane_u32::<1>(x24[3]) as u64) << 32 | vgetq_lane_u32::<1>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t848[1] = vsetq_lane_u32::<1>(v as u32, t848[1]); t848[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t848[3]); }
    { let v = (((vgetq_lane_u32::<2>(x24[3]) as u64) << 32 | vgetq_lane_u32::<2>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t848[1] = vsetq_lane_u32::<2>(v as u32, t848[1]); t848[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t848[3]); }
    { let v = (((vgetq_lane_u32::<3>(x24[3]) as u64) << 32 | vgetq_lane_u32::<3>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t848[1] = vsetq_lane_u32::<3>(v as u32, t848[1]); t848[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t848[3]); }
    let t849: [uint32x4_t; 2] = [x32[0], x32[1]];
    let t850: [uint32x4_t; 2] = [vandq_u32(t849[0], vdupq_n_u32(63)), vandq_u32(t849[1], vdupq_n_u32(63))];
    let t851: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t848[0], vnegq_s32(vreinterpretq_s32_u32(t850[0]))), vshlq_u32(t848[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t850[0])))), vorrq_u32(vshlq_u32(t848[1], vnegq_s32(vreinterpretq_s32_u32(t850[1]))), vshlq_u32(t848[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t850[1])))), vshlq_u32(t848[2], vnegq_s32(vreinterpretq_s32_u32(t850[0]))), vshlq_u32(t848[3], vnegq_s32(vreinterpretq_s32_u32(t850[1])))];
    x40 = t851;
    let t852 = [vdupq_n_u32(0u32); 2];
    let t853: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t852[0])), vmvnq_u32(vceqq_u32(x32[1], t852[1]))];
    let t854 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t855: [uint32x4_t; 2] = [vaddq_u32(x32[0], t854[0]), vaddq_u32(x32[1], t854[1])];
    let t856 = [vdupq_n_u32(64u32); 2];
    let t857: [uint32x4_t; 2] = [vcgtq_u32(t855[0], t856[0]), vcgtq_u32(t855[1], t856[1])];
    let t858: [uint32x4_t; 2] = [vandq_u32(t853[0], t857[0]), vandq_u32(t853[1], t857[1])];
    let mut t859: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t860: [uint32x4_t; 2] = [vmvnq_u32(t858[0]), vmvnq_u32(t858[1])];
    if any(&t858) {
        let t861: u64 = (1u64).wrapping_mul(8u64);
        let t862 = [vdupq_n_u32((t861) as u32); 2];
        let t863 = [vdupq_n_u32(((t861) >> 32) as u32); 2];
        let t864: [uint32x4_t; 2] = [vaddq_u32(x24[0], t862[0]), vaddq_u32(x24[1], t862[1])];
        let t865: [uint32x4_t; 2] = [vcltq_u32(t864[0], x24[0]), vcltq_u32(t864[1], x24[1])];
        let t866: [uint32x4_t; 4] = [t864[0], t864[1], vsubq_u32(vaddq_u32(x24[2], t863[0]), t865[0]), vsubq_u32(vaddq_u32(x24[3], t863[1]), t865[1])];
        let mut t867: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t866[2]) as u64) << 32 | vgetq_lane_u32::<0>(t866[0]) as u64) as usize as *const u64).read_unaligned(); t867[0] = vsetq_lane_u32::<0>(v as u32, t867[0]); t867[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t867[2]); }
        { let v = (((vgetq_lane_u32::<1>(t866[2]) as u64) << 32 | vgetq_lane_u32::<1>(t866[0]) as u64) as usize as *const u64).read_unaligned(); t867[0] = vsetq_lane_u32::<1>(v as u32, t867[0]); t867[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t867[2]); }
        { let v = (((vgetq_lane_u32::<2>(t866[2]) as u64) << 32 | vgetq_lane_u32::<2>(t866[0]) as u64) as usize as *const u64).read_unaligned(); t867[0] = vsetq_lane_u32::<2>(v as u32, t867[0]); t867[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t867[2]); }
        { let v = (((vgetq_lane_u32::<3>(t866[2]) as u64) << 32 | vgetq_lane_u32::<3>(t866[0]) as u64) as usize as *const u64).read_unaligned(); t867[0] = vsetq_lane_u32::<3>(v as u32, t867[0]); t867[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t867[2]); }
        { let v = (((vgetq_lane_u32::<0>(t866[3]) as u64) << 32 | vgetq_lane_u32::<0>(t866[1]) as u64) as usize as *const u64).read_unaligned(); t867[1] = vsetq_lane_u32::<0>(v as u32, t867[1]); t867[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t867[3]); }
        { let v = (((vgetq_lane_u32::<1>(t866[3]) as u64) << 32 | vgetq_lane_u32::<1>(t866[1]) as u64) as usize as *const u64).read_unaligned(); t867[1] = vsetq_lane_u32::<1>(v as u32, t867[1]); t867[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t867[3]); }
        { let v = (((vgetq_lane_u32::<2>(t866[3]) as u64) << 32 | vgetq_lane_u32::<2>(t866[1]) as u64) as usize as *const u64).read_unaligned(); t867[1] = vsetq_lane_u32::<2>(v as u32, t867[1]); t867[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t867[3]); }
        { let v = (((vgetq_lane_u32::<3>(t866[3]) as u64) << 32 | vgetq_lane_u32::<3>(t866[1]) as u64) as usize as *const u64).read_unaligned(); t867[1] = vsetq_lane_u32::<3>(v as u32, t867[1]); t867[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t867[3]); }
        let t868: [uint32x4_t; 2] = [vsubq_u32(t856[0], x32[0]), vsubq_u32(t856[1], x32[1])];
        let t869: [uint32x4_t; 2] = [t868[0], t868[1]];
        let t870: [uint32x4_t; 2] = [vandq_u32(t869[0], vdupq_n_u32(63)), vandq_u32(t869[1], vdupq_n_u32(63))];
        let t871: [uint32x4_t; 4] = [vshlq_u32(t867[0], vreinterpretq_s32_u32(t870[0])), vshlq_u32(t867[1], vreinterpretq_s32_u32(t870[1])), vorrq_u32(vshlq_u32(t867[2], vreinterpretq_s32_u32(t870[0])), vshlq_u32(t867[0], vsubq_s32(vreinterpretq_s32_u32(t870[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t867[3], vreinterpretq_s32_u32(t870[1])), vshlq_u32(t867[1], vsubq_s32(vreinterpretq_s32_u32(t870[1]), vdupq_n_s32(32))))];
        let t872: [uint32x4_t; 4] = [vorrq_u32(x40[0], t871[0]), vorrq_u32(x40[1], t871[1]), vorrq_u32(x40[2], t871[2]), vorrq_u32(x40[3], t871[3])];
        t859 = [vbslq_u32(t858[0], t872[0], t859[0]), vbslq_u32(t858[1], t872[1], t859[1]), vbslq_u32(t858[0], t872[2], t859[2]), vbslq_u32(t858[1], t872[3], t859[3])];
    }
    {
        t859 = [vbslq_u32(t860[0], x40[0], t859[0]), vbslq_u32(t860[1], x40[1], t859[1]), vbslq_u32(t860[0], x40[2], t859[2]), vbslq_u32(t860[1], x40[3], t859[3])];
    }
    res = t859;
    return res;
}

#[inline(always)]
unsafe fn b6_i_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t873: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x0), vld1q_u32(LANE[1].as_ptr()))];
    let t874: [uint32x4_t; 2] = [vshlq_u32(t873[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(t873[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
    let t875 = [vdupq_n_u32(255u32); 2];
    let t876: [uint32x4_t; 2] = [vandq_u32(t873[0], t875[0]), vandq_u32(t873[1], t875[1])];
    let t877: [uint32x4_t; 2] = [vmulq_u32(t874[0], vdupq_n_u32(1000)), vmulq_u32(t874[1], vdupq_n_u32(1000))];
    let t878: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(t874[0]), vget_low_u32(vdupq_n_u32(1000)))), vreinterpretq_u32_u64(vmull_high_u32(t874[0], vdupq_n_u32(1000)))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(t874[1]), vget_low_u32(vdupq_n_u32(1000)))), vreinterpretq_u32_u64(vmull_high_u32(t874[1], vdupq_n_u32(1000))))];
    let t879: [uint32x4_t; 2] = [vaddq_u32(t877[0], t876[0]), vaddq_u32(t877[1], t876[1])];
    let t880: [uint32x4_t; 2] = [vcltq_u32(t879[0], t877[0]), vcltq_u32(t879[1], t877[1])];
    let t881: [uint32x4_t; 4] = [t879[0], t879[1], vsubq_u32(vaddq_u32(t878[0], vdupq_n_u32(0)), t880[0]), vsubq_u32(vaddq_u32(t878[1], vdupq_n_u32(0)), t880[1])];
    res = t881;
    return res;
}

#[inline(always)]
unsafe fn b6_i_f(l: &Launch, lid0: u32, x0: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t882: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x0), vld1q_u32(LANE[1].as_ptr()))];
    let t883: [uint32x4_t; 2] = [vshlq_u32(t882[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(t882[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
    let t884 = [vdupq_n_u32(255u32); 2];
    let t885: [uint32x4_t; 2] = [vandq_u32(t882[0], t884[0]), vandq_u32(t882[1], t884[1])];
    let t886: [uint32x4_t; 2] = [vmulq_u32(t883[0], vdupq_n_u32(1000)), vmulq_u32(t883[1], vdupq_n_u32(1000))];
    let t887: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(t883[0]), vget_low_u32(vdupq_n_u32(1000)))), vreinterpretq_u32_u64(vmull_high_u32(t883[0], vdupq_n_u32(1000)))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(t883[1]), vget_low_u32(vdupq_n_u32(1000)))), vreinterpretq_u32_u64(vmull_high_u32(t883[1], vdupq_n_u32(1000))))];
    let t888: [uint32x4_t; 2] = [vaddq_u32(t886[0], t885[0]), vaddq_u32(t886[1], t885[1])];
    let t889: [uint32x4_t; 2] = [vcltq_u32(t888[0], t886[0]), vcltq_u32(t888[1], t886[1])];
    let t890: [uint32x4_t; 4] = [t888[0], t888[1], vsubq_u32(vaddq_u32(t887[0], vdupq_n_u32(0)), t889[0]), vsubq_u32(vaddq_u32(t887[1], vdupq_n_u32(0)), t889[1])];
    res = t890;
    return res;
}

#[inline(always)]
unsafe fn b6_u8_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t891: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32((x0 & 255u32)), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32((x0 & 255u32)), vld1q_u32(LANE[1].as_ptr()))];
    let t892: [uint32x4_t; 2] = [t891[0], t891[1]];
    let t893: u64 = ((((((((x0 >> 8)) as u64)).wrapping_mul(1000u64)) as usize as *const u8)) as usize as u64);
    let t894 = [vdupq_n_u32((t893) as u32); 2];
    let t895 = [vdupq_n_u32(((t893) >> 32) as u32); 2];
    let t896: [uint32x4_t; 2] = [vaddq_u32(t894[0], t892[0]), vaddq_u32(t894[1], t892[1])];
    let t897: [uint32x4_t; 2] = [vcltq_u32(t896[0], t894[0]), vcltq_u32(t896[1], t894[1])];
    let t898: [uint32x4_t; 4] = [t896[0], t896[1], vsubq_u32(vaddq_u32(t895[0], vdupq_n_u32(0)), t897[0]), vsubq_u32(vaddq_u32(t895[1], vdupq_n_u32(0)), t897[1])];
    let t899: [uint32x4_t; 4] = [t898[0], t898[1], t898[2], t898[3]];
    res = t899;
    return res;
}

#[inline(always)]
unsafe fn b6_u8_f(l: &Launch, lid0: u32, x0: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t900: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32((x0 & 255u32)), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32((x0 & 255u32)), vld1q_u32(LANE[1].as_ptr()))];
    let t901: [uint32x4_t; 2] = [t900[0], t900[1]];
    let t902: u64 = ((((((((x0 >> 8)) as u64)).wrapping_mul(1000u64)) as usize as *const u8)) as usize as u64);
    let t903 = [vdupq_n_u32((t902) as u32); 2];
    let t904 = [vdupq_n_u32(((t902) >> 32) as u32); 2];
    let t905: [uint32x4_t; 2] = [vaddq_u32(t903[0], t901[0]), vaddq_u32(t903[1], t901[1])];
    let t906: [uint32x4_t; 2] = [vcltq_u32(t905[0], t903[0]), vcltq_u32(t905[1], t903[1])];
    let t907: [uint32x4_t; 4] = [t905[0], t905[1], vsubq_u32(vaddq_u32(t904[0], vdupq_n_u32(0)), t906[0]), vsubq_u32(vaddq_u32(t904[1], vdupq_n_u32(0)), t906[1])];
    let t908: [uint32x4_t; 4] = [t907[0], t907[1], t907[2], t907[3]];
    res = t908;
    return res;
}

#[inline(always)]
unsafe fn b7_ss_f(l: &Launch, lid0: u32, x0: u64, x8: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x16: u64 = 0;
    let mut x24: u32 = 0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    x16 = x0;
    x24 = 0u32;
    loop {
        // Rounds of one condition: every lane goes on.
        if !(((x24) < (((x8) & (15u32))))) { break; }
        x16 = ((x16).wrapping_mul(31u64)).wrapping_add(((x24) as u64));
        x24 = (x24).wrapping_add(2u32);
    }
    res = x16;
    return res;
}

#[inline(always)]
unsafe fn b7_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], x8: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x16: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x24: u32 = 0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    x16 = x0;
    x24 = 0u32;
    let mut t913 = live;
    loop {
        // Lanes part: each its own rounds.
        let t915 = [vdupq_n_u32(15u32); 2];
        let t916: [uint32x4_t; 2] = [vandq_u32(x8[0], t915[0]), vandq_u32(x8[1], t915[1])];
        let t917 = [vdupq_n_u32(x24); 2];
        let t918: [uint32x4_t; 2] = [vcltq_u32(t917[0], t916[0]), vcltq_u32(t917[1], t916[1])];
        t913 = [vandq_u32(t913[0], t918[0]), vandq_u32(t913[1], t918[1])];
        if !(any(&t913)) { break; }
        let t920: u64 = 31u64;
        let t921 = [vdupq_n_u32((t920) as u32); 2];
        let t922: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x16[0]), vget_low_u32(t921[0]))), vreinterpretq_u32_u64(vmull_high_u32(x16[0], t921[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x16[1]), vget_low_u32(t921[1]))), vreinterpretq_u32_u64(vmull_high_u32(x16[1], t921[1])))];
        let t923: [uint32x4_t; 4] = [vmulq_u32(x16[0], t921[0]), vmulq_u32(x16[1], t921[1]), vmlaq_u32(t922[0], t921[0], x16[2]), vmlaq_u32(t922[1], t921[1], x16[3])];
        let t924: u64 = ((x24) as u64);
        let t925 = [vdupq_n_u32((t924) as u32); 2];
        let t926 = [vdupq_n_u32(((t924) >> 32) as u32); 2];
        let t927: [uint32x4_t; 2] = [vaddq_u32(t923[0], t925[0]), vaddq_u32(t923[1], t925[1])];
        let t928: [uint32x4_t; 2] = [vcltq_u32(t927[0], t923[0]), vcltq_u32(t927[1], t923[1])];
        let t929: [uint32x4_t; 4] = [t927[0], t927[1], vsubq_u32(vaddq_u32(t923[2], t926[0]), t928[0]), vsubq_u32(vaddq_u32(t923[3], t926[1]), t928[1])];
        x16 = [vbslq_u32(t913[0], t929[0], x16[0]), vbslq_u32(t913[1], t929[1], x16[1]), vbslq_u32(t913[0], t929[2], x16[2]), vbslq_u32(t913[1], t929[3], x16[3])];
        let t930: [uint32x4_t; 2] = [vandq_u32(t913[0], live[0]), vandq_u32(t913[1], live[1])];
        x24 = (x24).wrapping_add(2u32);
    }
    res = x16;
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
    let x32: *const u8 = *argv.add(4);
    let arg5: *const u8 = (*argv.add(5) as *const *const u8).read_unaligned();
    let x56: u32 = (*argv.add(6) as *const u32).read_unaligned();
    let mut t931: bool = !(arg5).is_null();
    if t931 {
        let mut t932: bool = ((l.group[0]) >= ((arg5.wrapping_add((((x56) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        if !t932 {
            t932 = ((l.group[1]) >= ((arg5.wrapping_add(((((x56).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        let mut t933: bool = t932;
        if !t933 {
            t933 = ((l.group[2]) >= ((arg5.wrapping_add(((((x56).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        t931 = t933;
    }
    if t931 {
        return;
    }
    let mut lid0 = 0u32;
    while lid0 + 8 <= n {
        let mut live = [vdupq_n_u32(u32::MAX); 2];
        let gid0 = l.group[0] * l.block[0] + lid0;
        'chunk: {
            b0_r0r1r2r3sg_f(l, lid0, arg0, arg1, arg2, arg3, x32);
        }
        lid0 += 8;
    }
    if lid0 < n {
        // Lanes part: the group's last threads, those under `n`.
        let t934: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
        let mut live = t934;
        let gid0 = l.group[0] * l.block[0] + lid0;
        'chunk: {
            b0_r0r1r2r3sg_p(l, lid0, live, arg0, arg1, arg2, arg3, x32);
        }
    }
}
