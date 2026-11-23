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
        let mut t162 = live;
        loop {
            // Lanes part: each its own rounds.
            let t164 = [vdupq_n_u32(7u32); 2];
            let t165: [uint32x4_t; 2] = [vandq_u32(x72[0], t164[0]), vandq_u32(x72[1], t164[1])];
            let t166 = [vdupq_n_u32(x120); 2];
            let t167: [uint32x4_t; 2] = [vcltq_u32(t166[0], t165[0]), vcltq_u32(t166[1], t165[1])];
            t162 = [vandq_u32(t162[0], t167[0]), vandq_u32(t162[1], t167[1])];
            if !(any(&t162)) { break; }
            let t169: u64 = 3u64;
            let t170 = [vdupq_n_u32((t169) as u32); 2];
            let t171: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x112[0]), vget_low_u32(t170[0]))), vreinterpretq_u32_u64(vmull_high_u32(x112[0], t170[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x112[1]), vget_low_u32(t170[1]))), vreinterpretq_u32_u64(vmull_high_u32(x112[1], t170[1])))];
            let t172: [uint32x4_t; 4] = [vmulq_u32(x112[0], t170[0]), vmulq_u32(x112[1], t170[1]), vmlaq_u32(t171[0], t170[0], x112[2]), vmlaq_u32(t171[1], t170[1], x112[3])];
            let t173: [uint32x4_t; 2] = [vaddq_u32(t172[0], x64[0]), vaddq_u32(t172[1], x64[1])];
            let t174: [uint32x4_t; 2] = [vcltq_u32(t173[0], t172[0]), vcltq_u32(t173[1], t172[1])];
            let t175: [uint32x4_t; 4] = [t173[0], t173[1], vsubq_u32(vaddq_u32(t172[2], x64[2]), t174[0]), vsubq_u32(vaddq_u32(t172[3], x64[3]), t174[1])];
            x112 = [vbslq_u32(t162[0], t175[0], x112[0]), vbslq_u32(t162[1], t175[1], x112[1]), vbslq_u32(t162[0], t175[2], x112[2]), vbslq_u32(t162[1], t175[3], x112[3])];
            let t176: [uint32x4_t; 2] = [vandq_u32(t162[0], live[0]), vandq_u32(t162[1], live[1])];
            let t177 = x120;
            x120 = (x120).wrapping_add(1u32);
        }
        let t178 = (arg3 as *const u8).add(((gid0.wrapping_add((19u32).wrapping_mul(x88))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t178 as *mut u32).add(0), uint32x4x2_t(x112[0], x112[2]));
            vst2q_u32((t178 as *mut u32).add(8), uint32x4x2_t(x112[1], x112[3]));
        } else {
            let mut t179w = 0u64;
            let t179 = core::hint::black_box(&raw mut t179w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t178 as *mut u64).add(0), t179).write_unaligned(((vgetq_lane_u32::<0>(x112[2]) as u64) << 32 | vgetq_lane_u32::<0>(x112[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t178 as *mut u64).add(1), t179).write_unaligned(((vgetq_lane_u32::<1>(x112[2]) as u64) << 32 | vgetq_lane_u32::<1>(x112[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t178 as *mut u64).add(2), t179).write_unaligned(((vgetq_lane_u32::<2>(x112[2]) as u64) << 32 | vgetq_lane_u32::<2>(x112[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t178 as *mut u64).add(3), t179).write_unaligned(((vgetq_lane_u32::<3>(x112[2]) as u64) << 32 | vgetq_lane_u32::<3>(x112[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t178 as *mut u64).add(4), t179).write_unaligned(((vgetq_lane_u32::<0>(x112[3]) as u64) << 32 | vgetq_lane_u32::<0>(x112[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t178 as *mut u64).add(5), t179).write_unaligned(((vgetq_lane_u32::<1>(x112[3]) as u64) << 32 | vgetq_lane_u32::<1>(x112[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t178 as *mut u64).add(6), t179).write_unaligned(((vgetq_lane_u32::<2>(x112[3]) as u64) << 32 | vgetq_lane_u32::<2>(x112[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t178 as *mut u64).add(7), t179).write_unaligned(((vgetq_lane_u32::<3>(x112[3]) as u64) << 32 | vgetq_lane_u32::<3>(x112[1]) as u64));
        }
        let t180: u64 = a1;
        let t181 = [vdupq_n_u32((t180) as u32); 2];
        let t182 = [vdupq_n_u32(((t180) >> 32) as u32); 2];
        let t183: [uint32x4_t; 2] = [vaddq_u32(x56[0], t181[0]), vaddq_u32(x56[1], t181[1])];
        let t184: [uint32x4_t; 2] = [vcltq_u32(t183[0], x56[0]), vcltq_u32(t183[1], x56[1])];
        let t185: [uint32x4_t; 4] = [t183[0], t183[1], vsubq_u32(vaddq_u32(x56[2], t182[0]), t184[0]), vsubq_u32(vaddq_u32(x56[3], t182[1]), t184[1])];
        let t186: u64 = (a1).wrapping_shl((3u64) as u32);
        let t187 = [vdupq_n_u32((t186) as u32); 2];
        let t188 = [vdupq_n_u32(((t186) >> 32) as u32); 2];
        let t189: [uint32x4_t; 2] = [vaddq_u32(t185[0], t187[0]), vaddq_u32(t185[1], t187[1])];
        let t190: [uint32x4_t; 2] = [vcltq_u32(t189[0], t185[0]), vcltq_u32(t189[1], t185[1])];
        let t191: [uint32x4_t; 4] = [t189[0], t189[1], vsubq_u32(vaddq_u32(t185[2], t188[0]), t190[0]), vsubq_u32(vaddq_u32(t185[3], t188[1]), t190[1])];
        let t192 = (arg3 as *const u8).add(((gid0.wrapping_add((20u32).wrapping_mul(x88))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t192 as *mut u32).add(0), uint32x4x2_t(t191[0], t191[2]));
            vst2q_u32((t192 as *mut u32).add(8), uint32x4x2_t(t191[1], t191[3]));
        } else {
            let mut t193w = 0u64;
            let t193 = core::hint::black_box(&raw mut t193w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t192 as *mut u64).add(0), t193).write_unaligned(((vgetq_lane_u32::<0>(t191[2]) as u64) << 32 | vgetq_lane_u32::<0>(t191[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t192 as *mut u64).add(1), t193).write_unaligned(((vgetq_lane_u32::<1>(t191[2]) as u64) << 32 | vgetq_lane_u32::<1>(t191[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t192 as *mut u64).add(2), t193).write_unaligned(((vgetq_lane_u32::<2>(t191[2]) as u64) << 32 | vgetq_lane_u32::<2>(t191[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t192 as *mut u64).add(3), t193).write_unaligned(((vgetq_lane_u32::<3>(t191[2]) as u64) << 32 | vgetq_lane_u32::<3>(t191[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t192 as *mut u64).add(4), t193).write_unaligned(((vgetq_lane_u32::<0>(t191[3]) as u64) << 32 | vgetq_lane_u32::<0>(t191[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t192 as *mut u64).add(5), t193).write_unaligned(((vgetq_lane_u32::<1>(t191[3]) as u64) << 32 | vgetq_lane_u32::<1>(t191[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t192 as *mut u64).add(6), t193).write_unaligned(((vgetq_lane_u32::<2>(t191[3]) as u64) << 32 | vgetq_lane_u32::<2>(t191[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t192 as *mut u64).add(7), t193).write_unaligned(((vgetq_lane_u32::<3>(t191[3]) as u64) << 32 | vgetq_lane_u32::<3>(t191[1]) as u64));
        }
        let t194: u64 = 4294967295u64;
        let t195 = [vdupq_n_u32((t194) as u32); 2];
        let t196: [uint32x4_t; 2] = [vandq_u32(x56[0], t195[0]), vandq_u32(x56[1], t195[1])];
        let t197: u64 = 4294967295u64;
        let t198 = [vdupq_n_u32((t197) as u32); 2];
        let t199: [uint32x4_t; 2] = [vandq_u32(x64[0], t198[0]), vandq_u32(x64[1], t198[1])];
        let t200: [uint32x4_t; 2] = [vceqq_u32(t196[0], t199[0]), vceqq_u32(t196[1], t199[1])];
        let mut t201: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t202: [uint32x4_t; 2] = [vandq_u32(live[0], t200[0]), vandq_u32(live[1], t200[1])];
        let t203: [uint32x4_t; 2] = [vbicq_u32(live[0], t200[0]), vbicq_u32(live[1], t200[1])];
        {
            let t204 = [vdupq_n_u32(1u32); 2];
            t201 = [vbslq_u32(t202[0], t204[0], t201[0]), vbslq_u32(t202[1], t204[1], t201[1])];
        }
        {
            let t205 = [vdupq_n_u32(2u32); 2];
            t201 = [vbslq_u32(t203[0], t205[0], t201[0]), vbslq_u32(t203[1], t205[1], t201[1])];
        }
        let t206: [uint32x4_t; 4] = [t201[0], t201[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        let t207 = (arg3 as *const u8).add(((gid0.wrapping_add((21u32).wrapping_mul(x88))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t207 as *mut u32).add(0), uint32x4x2_t(t206[0], t206[2]));
            vst2q_u32((t207 as *mut u32).add(8), uint32x4x2_t(t206[1], t206[3]));
        } else {
            let mut t208w = 0u64;
            let t208 = core::hint::black_box(&raw mut t208w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t207 as *mut u64).add(0), t208).write_unaligned(((vgetq_lane_u32::<0>(t206[2]) as u64) << 32 | vgetq_lane_u32::<0>(t206[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t207 as *mut u64).add(1), t208).write_unaligned(((vgetq_lane_u32::<1>(t206[2]) as u64) << 32 | vgetq_lane_u32::<1>(t206[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t207 as *mut u64).add(2), t208).write_unaligned(((vgetq_lane_u32::<2>(t206[2]) as u64) << 32 | vgetq_lane_u32::<2>(t206[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t207 as *mut u64).add(3), t208).write_unaligned(((vgetq_lane_u32::<3>(t206[2]) as u64) << 32 | vgetq_lane_u32::<3>(t206[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t207 as *mut u64).add(4), t208).write_unaligned(((vgetq_lane_u32::<0>(t206[3]) as u64) << 32 | vgetq_lane_u32::<0>(t206[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t207 as *mut u64).add(5), t208).write_unaligned(((vgetq_lane_u32::<1>(t206[3]) as u64) << 32 | vgetq_lane_u32::<1>(t206[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t207 as *mut u64).add(6), t208).write_unaligned(((vgetq_lane_u32::<2>(t206[3]) as u64) << 32 | vgetq_lane_u32::<2>(t206[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t207 as *mut u64).add(7), t208).write_unaligned(((vgetq_lane_u32::<3>(t206[3]) as u64) << 32 | vgetq_lane_u32::<3>(t206[1]) as u64));
        }
        let t209: [uint32x4_t; 2] = [vtstq_u32(vorrq_u32(x56[0], x56[2]), vorrq_u32(x56[0], x56[2])), vtstq_u32(vorrq_u32(x56[1], x56[3]), vorrq_u32(x56[1], x56[3]))];
        let mut t210: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t211: [uint32x4_t; 2] = [vandq_u32(live[0], t209[0]), vandq_u32(live[1], t209[1])];
        let t212: [uint32x4_t; 2] = [vbicq_u32(live[0], t209[0]), vbicq_u32(live[1], t209[1])];
        {
            let t213 = [vdupq_n_u32(5u32); 2];
            t210 = [vbslq_u32(t211[0], t213[0], t210[0]), vbslq_u32(t211[1], t213[1], t210[1])];
        }
        {
            let t214 = [vdupq_n_u32(6u32); 2];
            t210 = [vbslq_u32(t212[0], t214[0], t210[0]), vbslq_u32(t212[1], t214[1], t210[1])];
        }
        let t215: [uint32x4_t; 4] = [t210[0], t210[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        let t216 = (arg3 as *const u8).add(((gid0.wrapping_add((22u32).wrapping_mul(x88))) as usize) * 8);
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
        let t218 = b4_ll_p(l, lid0, live, x56, x64);
        let t219 = (arg3 as *const u8).add(((gid0.wrapping_add((23u32).wrapping_mul(x88))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t219 as *mut u32).add(0), uint32x4x2_t(t218[0], t218[2]));
            vst2q_u32((t219 as *mut u32).add(8), uint32x4x2_t(t218[1], t218[3]));
        } else {
            let mut t220w = 0u64;
            let t220 = core::hint::black_box(&raw mut t220w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t219 as *mut u64).add(0), t220).write_unaligned(((vgetq_lane_u32::<0>(t218[2]) as u64) << 32 | vgetq_lane_u32::<0>(t218[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t219 as *mut u64).add(1), t220).write_unaligned(((vgetq_lane_u32::<1>(t218[2]) as u64) << 32 | vgetq_lane_u32::<1>(t218[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t219 as *mut u64).add(2), t220).write_unaligned(((vgetq_lane_u32::<2>(t218[2]) as u64) << 32 | vgetq_lane_u32::<2>(t218[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t219 as *mut u64).add(3), t220).write_unaligned(((vgetq_lane_u32::<3>(t218[2]) as u64) << 32 | vgetq_lane_u32::<3>(t218[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t219 as *mut u64).add(4), t220).write_unaligned(((vgetq_lane_u32::<0>(t218[3]) as u64) << 32 | vgetq_lane_u32::<0>(t218[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t219 as *mut u64).add(5), t220).write_unaligned(((vgetq_lane_u32::<1>(t218[3]) as u64) << 32 | vgetq_lane_u32::<1>(t218[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t219 as *mut u64).add(6), t220).write_unaligned(((vgetq_lane_u32::<2>(t218[3]) as u64) << 32 | vgetq_lane_u32::<2>(t218[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t219 as *mut u64).add(7), t220).write_unaligned(((vgetq_lane_u32::<3>(t218[3]) as u64) << 32 | vgetq_lane_u32::<3>(t218[1]) as u64));
        }
        let t221 = b5_r0ns_p(l, lid0, live, arg0, gid0.wrapping_mul(x96), x96, 8u32);
        let t222 = (arg3 as *const u8).add(((gid0.wrapping_add((24u32).wrapping_mul(x88))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t222 as *mut u32).add(0), uint32x4x2_t(t221[0], t221[2]));
            vst2q_u32((t222 as *mut u32).add(8), uint32x4x2_t(t221[1], t221[3]));
        } else {
            let mut t223w = 0u64;
            let t223 = core::hint::black_box(&raw mut t223w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t222 as *mut u64).add(0), t223).write_unaligned(((vgetq_lane_u32::<0>(t221[2]) as u64) << 32 | vgetq_lane_u32::<0>(t221[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t222 as *mut u64).add(1), t223).write_unaligned(((vgetq_lane_u32::<1>(t221[2]) as u64) << 32 | vgetq_lane_u32::<1>(t221[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t222 as *mut u64).add(2), t223).write_unaligned(((vgetq_lane_u32::<2>(t221[2]) as u64) << 32 | vgetq_lane_u32::<2>(t221[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t222 as *mut u64).add(3), t223).write_unaligned(((vgetq_lane_u32::<3>(t221[2]) as u64) << 32 | vgetq_lane_u32::<3>(t221[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t222 as *mut u64).add(4), t223).write_unaligned(((vgetq_lane_u32::<0>(t221[3]) as u64) << 32 | vgetq_lane_u32::<0>(t221[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t222 as *mut u64).add(5), t223).write_unaligned(((vgetq_lane_u32::<1>(t221[3]) as u64) << 32 | vgetq_lane_u32::<1>(t221[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t222 as *mut u64).add(6), t223).write_unaligned(((vgetq_lane_u32::<2>(t221[3]) as u64) << 32 | vgetq_lane_u32::<2>(t221[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t222 as *mut u64).add(7), t223).write_unaligned(((vgetq_lane_u32::<3>(t221[3]) as u64) << 32 | vgetq_lane_u32::<3>(t221[1]) as u64));
        }
        let t224 = if (gid0.wrapping_add(3u32) & 255u32) <= 248u32 { b6_u8_p(l, lid0, live, gid0.wrapping_add(3u32)) } else { b6_i_p(l, lid0, live, gid0.wrapping_add(3u32)) };
        let t225 = (arg3 as *const u8).add(((gid0.wrapping_add((25u32).wrapping_mul(x88))) as usize) * 8);
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
        let t227 = (arg0 as *const u8).add(((gid0) as usize) * 8);
        let t2290 = vld2q_u32((t227 as *const u32).add(0));
        let t2291 = vld2q_u32((t227 as *const u32).add(8));
        let t229: [uint32x4_t; 4] = [t2290.0, t2291.0, t2290.1, t2291.1];
        x56 = t229;
        let t230 = (arg1 as *const u8).add(((gid0) as usize) * 8);
        let t2320 = vld2q_u32((t230 as *const u32).add(0));
        let t2321 = vld2q_u32((t230 as *const u32).add(8));
        let t232: [uint32x4_t; 4] = [t2320.0, t2321.0, t2320.1, t2321.1];
        x64 = t232;
        let t233 = (arg2 as *const u32).add((gid0) as usize);
        let t234: [uint32x4_t; 2] = [vld1q_u32(t233.add(0)), vld1q_u32(t233.add(4))];
        x72 = t234;
        let t235 = [vdupq_n_u32(63u32); 2];
        let t236: [uint32x4_t; 2] = [vandq_u32(x72[0], t235[0]), vandq_u32(x72[1], t235[1])];
        x80 = t236;
        x88 = a0;
        let t237: u32;
        if ((((a1) & (1u64))) != (0u64)) {
            t237 = 8u32;
        } else {
            t237 = 4u32;
        }
        x96 = t237;
        let t238: [uint32x4_t; 2] = [vaddq_u32(x56[0], x64[0]), vaddq_u32(x56[1], x64[1])];
        let t239: [uint32x4_t; 2] = [vcltq_u32(t238[0], x56[0]), vcltq_u32(t238[1], x56[1])];
        let t240: [uint32x4_t; 4] = [t238[0], t238[1], vsubq_u32(vaddq_u32(x56[2], x64[2]), t239[0]), vsubq_u32(vaddq_u32(x56[3], x64[3]), t239[1])];
        let t241 = (arg3 as *const u8).add(((gid0.wrapping_add((0u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t241 as *mut u32).add(0), uint32x4x2_t(t240[0], t240[2]));
        vst2q_u32((t241 as *mut u32).add(8), uint32x4x2_t(t240[1], t240[3]));
        let t242: [uint32x4_t; 2] = [vcltq_u32(x56[0], x64[0]), vcltq_u32(x56[1], x64[1])];
        let t243: [uint32x4_t; 4] = [vsubq_u32(x56[0], x64[0]), vsubq_u32(x56[1], x64[1]), vaddq_u32(vsubq_u32(x56[2], x64[2]), t242[0]), vaddq_u32(vsubq_u32(x56[3], x64[3]), t242[1])];
        let t244 = (arg3 as *const u8).add(((gid0.wrapping_add((1u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t244 as *mut u32).add(0), uint32x4x2_t(t243[0], t243[2]));
        vst2q_u32((t244 as *mut u32).add(8), uint32x4x2_t(t243[1], t243[3]));
        let t245: [uint32x4_t; 2] = [x80[0], x80[1]];
        let t246: [uint32x4_t; 2] = [vandq_u32(t245[0], vdupq_n_u32(63)), vandq_u32(t245[1], vdupq_n_u32(63))];
        let t247: [uint32x4_t; 4] = [vshlq_u32(x56[0], vreinterpretq_s32_u32(t246[0])), vshlq_u32(x56[1], vreinterpretq_s32_u32(t246[1])), vorrq_u32(vshlq_u32(x56[2], vreinterpretq_s32_u32(t246[0])), vshlq_u32(x56[0], vsubq_s32(vreinterpretq_s32_u32(t246[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(x56[3], vreinterpretq_s32_u32(t246[1])), vshlq_u32(x56[1], vsubq_s32(vreinterpretq_s32_u32(t246[1]), vdupq_n_s32(32))))];
        let t248 = (arg3 as *const u8).add(((gid0.wrapping_add((2u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t248 as *mut u32).add(0), uint32x4x2_t(t247[0], t247[2]));
        vst2q_u32((t248 as *mut u32).add(8), uint32x4x2_t(t247[1], t247[3]));
        let t249: [uint32x4_t; 2] = [x80[0], x80[1]];
        let t250: [uint32x4_t; 2] = [vandq_u32(t249[0], vdupq_n_u32(63)), vandq_u32(t249[1], vdupq_n_u32(63))];
        let t251: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(x56[0], vnegq_s32(vreinterpretq_s32_u32(t250[0]))), vshlq_u32(x56[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t250[0])))), vorrq_u32(vshlq_u32(x56[1], vnegq_s32(vreinterpretq_s32_u32(t250[1]))), vshlq_u32(x56[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t250[1])))), vshlq_u32(x56[2], vnegq_s32(vreinterpretq_s32_u32(t250[0]))), vshlq_u32(x56[3], vnegq_s32(vreinterpretq_s32_u32(t250[1])))];
        let t252 = (arg3 as *const u8).add(((gid0.wrapping_add((3u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t252 as *mut u32).add(0), uint32x4x2_t(t251[0], t251[2]));
        vst2q_u32((t252 as *mut u32).add(8), uint32x4x2_t(t251[1], t251[3]));
        let t253: [uint32x4_t; 2] = [x80[0], x80[1]];
        let t254: [uint32x4_t; 2] = [vandq_u32(t253[0], vdupq_n_u32(63)), vandq_u32(t253[1], vdupq_n_u32(63))];
        let t255: [uint32x4_t; 2] = [vcgeq_s32(vreinterpretq_s32_u32(t254[0]), vdupq_n_s32(32)), vcgeq_s32(vreinterpretq_s32_u32(t254[1]), vdupq_n_s32(32))];
        let t256: [uint32x4_t; 4] = [vbslq_u32(t255[0], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x56[2]), vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t254[0])))), vorrq_u32(vshlq_u32(x56[0], vnegq_s32(vreinterpretq_s32_u32(t254[0]))), vshlq_u32(x56[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t254[0]))))), vbslq_u32(t255[1], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x56[3]), vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t254[1])))), vorrq_u32(vshlq_u32(x56[1], vnegq_s32(vreinterpretq_s32_u32(t254[1]))), vshlq_u32(x56[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t254[1]))))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x56[2]), vnegq_s32(vreinterpretq_s32_u32(t254[0])))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x56[3]), vnegq_s32(vreinterpretq_s32_u32(t254[1]))))];
        let t257 = (arg3 as *const u8).add(((gid0.wrapping_add((4u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t257 as *mut u32).add(0), uint32x4x2_t(t256[0], t256[2]));
        vst2q_u32((t257 as *mut u32).add(8), uint32x4x2_t(t256[1], t256[3]));
        let t258: [uint32x4_t; 4] = [vshlq_n_u32::<13>(x56[0]), vshlq_n_u32::<13>(x56[1]), vorrq_u32(vshlq_n_u32::<13>(x56[2]), vshrq_n_u32::<19>(x56[0])), vorrq_u32(vshlq_n_u32::<13>(x56[3]), vshrq_n_u32::<19>(x56[1]))];
        let t259: [uint32x4_t; 2] = [vshrq_n_u32::<19>(x64[2]), vshrq_n_u32::<19>(x64[3])];
        let t260: [uint32x4_t; 4] = [vorrq_u32(t258[0], t259[0]), vorrq_u32(t258[1], t259[1]), t258[2], t258[3]];
        let t261 = (arg3 as *const u8).add(((gid0.wrapping_add((5u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t261 as *mut u32).add(0), uint32x4x2_t(t260[0], t260[2]));
        vst2q_u32((t261 as *mut u32).add(8), uint32x4x2_t(t260[1], t260[3]));
        let t262: [uint32x4_t; 4] = [vdupq_n_u32(0), vdupq_n_u32(0), vshlq_n_u32::<8>(x56[0]), vshlq_n_u32::<8>(x56[1])];
        let t263 = (arg3 as *const u8).add(((gid0.wrapping_add((6u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t263 as *mut u32).add(0), uint32x4x2_t(t262[0], t262[2]));
        vst2q_u32((t263 as *mut u32).add(8), uint32x4x2_t(t262[1], t262[3]));
        let t264: [uint32x4_t; 2] = [vshrq_n_u32::<3>(x56[2]), vshrq_n_u32::<3>(x56[3])];
        let t265: [uint32x4_t; 4] = [t264[0], t264[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        let t266 = (arg3 as *const u8).add(((gid0.wrapping_add((7u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t266 as *mut u32).add(0), uint32x4x2_t(t265[0], t265[2]));
        vst2q_u32((t266 as *mut u32).add(8), uint32x4x2_t(t265[1], t265[3]));
        let t267 = vdupq_n_s32(((((40i64) as u32)) & 63) as i32);
        let t268: [uint32x4_t; 2] = [vcgeq_s32(t267, vdupq_n_s32(32)), vcgeq_s32(t267, vdupq_n_s32(32))];
        let t269: [uint32x4_t; 4] = [vbslq_u32(t268[0], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x56[2]), vsubq_s32(vdupq_n_s32(32), t267))), vorrq_u32(vshlq_u32(x56[0], vnegq_s32(t267)), vshlq_u32(x56[2], vsubq_s32(vdupq_n_s32(32), t267)))), vbslq_u32(t268[1], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x56[3]), vsubq_s32(vdupq_n_s32(32), t267))), vorrq_u32(vshlq_u32(x56[1], vnegq_s32(t267)), vshlq_u32(x56[3], vsubq_s32(vdupq_n_s32(32), t267)))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x56[2]), vnegq_s32(t267))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x56[3]), vnegq_s32(t267)))];
        let t270 = (arg3 as *const u8).add(((gid0.wrapping_add((8u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t270 as *mut u32).add(0), uint32x4x2_t(t269[0], t269[2]));
        vst2q_u32((t270 as *mut u32).add(8), uint32x4x2_t(t269[1], t269[3]));
        let t271: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x56[0]), vget_low_u32(x64[0]))), vreinterpretq_u32_u64(vmull_high_u32(x56[0], x64[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x56[1]), vget_low_u32(x64[1]))), vreinterpretq_u32_u64(vmull_high_u32(x56[1], x64[1])))];
        let t272: [uint32x4_t; 4] = [vmulq_u32(x56[0], x64[0]), vmulq_u32(x56[1], x64[1]), vmlaq_u32(vmlaq_u32(t271[0], x56[0], x64[2]), x64[0], x56[2]), vmlaq_u32(vmlaq_u32(t271[1], x56[1], x64[3]), x64[1], x56[3])];
        let t273 = (arg3 as *const u8).add(((gid0.wrapping_add((9u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t273 as *mut u32).add(0), uint32x4x2_t(t272[0], t272[2]));
        vst2q_u32((t273 as *mut u32).add(8), uint32x4x2_t(t272[1], t272[3]));
        let t274: u64 = 0u64;
        let t275 = [vdupq_n_u32((t274) as u32); 2];
        let t276: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x64[0], t275[0]), vceqq_u32(x64[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(x64[1], t275[1]), vceqq_u32(x64[3], vdupq_n_u32(0)))];
        let t277: [uint32x4_t; 2] = [vmvnq_u32(t276[0]), vmvnq_u32(t276[1])];
        let mut t278: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t279: [uint32x4_t; 2] = [vmvnq_u32(t277[0]), vmvnq_u32(t277[1])];
        {
            let (mut t280xl, mut t280xh, mut t280yl, mut t280yh) = ([0u32; 8], [0u32; 8], [0u32; 8], [0u32; 8]);
            vst1q_u32(t280xl.as_mut_ptr().add(0), x56[0]); vst1q_u32(t280xh.as_mut_ptr().add(0), x56[2]); vst1q_u32(t280yl.as_mut_ptr().add(0), x64[0]); vst1q_u32(t280yh.as_mut_ptr().add(0), x64[2]);
            vst1q_u32(t280xl.as_mut_ptr().add(4), x56[1]); vst1q_u32(t280xh.as_mut_ptr().add(4), x56[3]); vst1q_u32(t280yl.as_mut_ptr().add(4), x64[1]); vst1q_u32(t280yh.as_mut_ptr().add(4), x64[3]);
            for j in 0..8 { let q = (((t280xh[j] as u64) << 32 | t280xl[j] as u64) as u64).checked_div(((t280yh[j] as u64) << 32 | t280yl[j] as u64) as u64).unwrap_or(0) as u64; t280xl[j] = q as u32; t280xh[j] = (q >> 32) as u32; }
            let t281: [uint32x4_t; 4] = [vld1q_u32(t280xl.as_ptr().add(0)), vld1q_u32(t280xl.as_ptr().add(4)), vld1q_u32(t280xh.as_ptr().add(0)), vld1q_u32(t280xh.as_ptr().add(4))];
            t278 = [vbslq_u32(t277[0], t281[0], t278[0]), vbslq_u32(t277[1], t281[1], t278[1]), vbslq_u32(t277[0], t281[2], t278[2]), vbslq_u32(t277[1], t281[3], t278[3])];
        }
        {
            let t282: u64 = 7u64;
            let t283 = [vdupq_n_u32((t282) as u32); 2];
            let t284: [uint32x4_t; 4] = [t283[0], t283[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            t278 = [vbslq_u32(t279[0], t284[0], t278[0]), vbslq_u32(t279[1], t284[1], t278[1]), vbslq_u32(t279[0], t284[2], t278[2]), vbslq_u32(t279[1], t284[3], t278[3])];
        }
        let t285 = (arg3 as *const u8).add(((gid0.wrapping_add((10u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t285 as *mut u32).add(0), uint32x4x2_t(t278[0], t278[2]));
        vst2q_u32((t285 as *mut u32).add(8), uint32x4x2_t(t278[1], t278[3]));
        let t286: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x56[2], x64[2]), vandq_u32(vceqq_u32(x56[2], x64[2]), vcltq_u32(x56[0], x64[0]))), vorrq_u32(vcltq_u32(x56[3], x64[3]), vandq_u32(vceqq_u32(x56[3], x64[3]), vcltq_u32(x56[1], x64[1])))];
        let t287: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t286[0]), vshrq_n_u32::<31>(t286[1])];
        let t288: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x64[2], x56[2]), vandq_u32(vceqq_u32(x64[2], x56[2]), vcltq_u32(x64[0], x56[0]))), vorrq_u32(vcltq_u32(x64[3], x56[3]), vandq_u32(vceqq_u32(x64[3], x56[3]), vcltq_u32(x64[1], x56[1])))];
        let t289: [uint32x4_t; 2] = [vmvnq_u32(t288[0]), vmvnq_u32(t288[1])];
        let t290: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t289[0]), vshrq_n_u32::<31>(t289[1])];
        let t291: [uint32x4_t; 4] = [vshlq_n_u32::<1>(t290[0]), vshlq_n_u32::<1>(t290[1]), vshrq_n_u32::<31>(t290[0]), vshrq_n_u32::<31>(t290[1])];
        let t292: [uint32x4_t; 4] = [vorrq_u32(t287[0], t291[0]), vorrq_u32(t287[1], t291[1]), t291[2], t291[3]];
        let t293: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x56[0], x64[0]), vceqq_u32(x56[2], x64[2])), vandq_u32(vceqq_u32(x56[1], x64[1]), vceqq_u32(x56[3], x64[3]))];
        let t294: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t293[0]), vshrq_n_u32::<31>(t293[1])];
        let t295: [uint32x4_t; 4] = [vshlq_n_u32::<2>(t294[0]), vshlq_n_u32::<2>(t294[1]), vshrq_n_u32::<30>(t294[0]), vshrq_n_u32::<30>(t294[1])];
        let t296: [uint32x4_t; 4] = [vorrq_u32(t292[0], t295[0]), vorrq_u32(t292[1], t295[1]), vorrq_u32(t292[2], t295[2]), vorrq_u32(t292[3], t295[3])];
        let t297: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x56[0], x64[0]), vceqq_u32(x56[2], x64[2])), vandq_u32(vceqq_u32(x56[1], x64[1]), vceqq_u32(x56[3], x64[3]))];
        let t298: [uint32x4_t; 2] = [vmvnq_u32(t297[0]), vmvnq_u32(t297[1])];
        let t299: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t298[0]), vshrq_n_u32::<31>(t298[1])];
        let t300: [uint32x4_t; 4] = [vshlq_n_u32::<3>(t299[0]), vshlq_n_u32::<3>(t299[1]), vshrq_n_u32::<29>(t299[0]), vshrq_n_u32::<29>(t299[1])];
        let t301: [uint32x4_t; 4] = [vorrq_u32(t296[0], t300[0]), vorrq_u32(t296[1], t300[1]), vorrq_u32(t296[2], t300[2]), vorrq_u32(t296[3], t300[3])];
        let t302: [uint32x4_t; 2] = [vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x56[2]), vreinterpretq_s32_u32(x64[2])), vandq_u32(vceqq_u32(x56[2], x64[2]), vcltq_u32(x56[0], x64[0]))), vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x56[3]), vreinterpretq_s32_u32(x64[3])), vandq_u32(vceqq_u32(x56[3], x64[3]), vcltq_u32(x56[1], x64[1])))];
        let t303: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t302[0]), vshrq_n_u32::<31>(t302[1])];
        let t304: [uint32x4_t; 4] = [vshlq_n_u32::<4>(t303[0]), vshlq_n_u32::<4>(t303[1]), vshrq_n_u32::<28>(t303[0]), vshrq_n_u32::<28>(t303[1])];
        let t305: [uint32x4_t; 4] = [vorrq_u32(t301[0], t304[0]), vorrq_u32(t301[1], t304[1]), vorrq_u32(t301[2], t304[2]), vorrq_u32(t301[3], t304[3])];
        let t306: [uint32x4_t; 2] = [vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x56[2]), vreinterpretq_s32_u32(x64[2])), vandq_u32(vceqq_u32(x56[2], x64[2]), vcltq_u32(x56[0], x64[0]))), vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x56[3]), vreinterpretq_s32_u32(x64[3])), vandq_u32(vceqq_u32(x56[3], x64[3]), vcltq_u32(x56[1], x64[1])))];
        let t307: [uint32x4_t; 2] = [vmvnq_u32(t306[0]), vmvnq_u32(t306[1])];
        let t308: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t307[0]), vshrq_n_u32::<31>(t307[1])];
        let t309: [uint32x4_t; 4] = [vshlq_n_u32::<5>(t308[0]), vshlq_n_u32::<5>(t308[1]), vshrq_n_u32::<27>(t308[0]), vshrq_n_u32::<27>(t308[1])];
        let t310: [uint32x4_t; 4] = [vorrq_u32(t305[0], t309[0]), vorrq_u32(t305[1], t309[1]), vorrq_u32(t305[2], t309[2]), vorrq_u32(t305[3], t309[3])];
        let t311: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x64[2], x56[2]), vandq_u32(vceqq_u32(x64[2], x56[2]), vcltq_u32(x64[0], x56[0]))), vorrq_u32(vcltq_u32(x64[3], x56[3]), vandq_u32(vceqq_u32(x64[3], x56[3]), vcltq_u32(x64[1], x56[1])))];
        let t312: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t311[0]), vshrq_n_u32::<31>(t311[1])];
        let t313: [uint32x4_t; 4] = [vshlq_n_u32::<6>(t312[0]), vshlq_n_u32::<6>(t312[1]), vshrq_n_u32::<26>(t312[0]), vshrq_n_u32::<26>(t312[1])];
        let t314: [uint32x4_t; 4] = [vorrq_u32(t310[0], t313[0]), vorrq_u32(t310[1], t313[1]), vorrq_u32(t310[2], t313[2]), vorrq_u32(t310[3], t313[3])];
        let t315 = (arg3 as *const u8).add(((gid0.wrapping_add((11u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t315 as *mut u32).add(0), uint32x4x2_t(t314[0], t314[2]));
        vst2q_u32((t315 as *mut u32).add(8), uint32x4x2_t(t314[1], t314[3]));
        let t316: [uint32x4_t; 2] = [vaddq_u32(x72[0], x56[0]), vaddq_u32(x72[1], x56[1])];
        let t317: [uint32x4_t; 2] = [vcltq_u32(t316[0], x72[0]), vcltq_u32(t316[1], x72[1])];
        let t318: [uint32x4_t; 4] = [t316[0], t316[1], vsubq_u32(vaddq_u32(vdupq_n_u32(0), x56[2]), t317[0]), vsubq_u32(vaddq_u32(vdupq_n_u32(0), x56[3]), t317[1])];
        let t319 = (arg3 as *const u8).add(((gid0.wrapping_add((12u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t319 as *mut u32).add(0), uint32x4x2_t(t318[0], t318[2]));
        vst2q_u32((t319 as *mut u32).add(8), uint32x4x2_t(t318[1], t318[3]));
        let t320: [uint32x4_t; 2] = [vreinterpretq_u32_s32(vshrq_n_s32::<31>(vreinterpretq_s32_u32(x72[0]))), vreinterpretq_u32_s32(vshrq_n_s32::<31>(vreinterpretq_s32_u32(x72[1])))];
        let t321: [uint32x4_t; 4] = [x72[0], x72[1], t320[0], t320[1]];
        let t322 = (arg3 as *const u8).add(((gid0.wrapping_add((13u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t322 as *mut u32).add(0), uint32x4x2_t(t321[0], t321[2]));
        vst2q_u32((t322 as *mut u32).add(8), uint32x4x2_t(t321[1], t321[3]));
        let t323: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<3>(x56[0]), vshlq_n_u32::<29>(x56[2])), vorrq_u32(vshrq_n_u32::<3>(x56[1]), vshlq_n_u32::<29>(x56[3])), vshrq_n_u32::<3>(x56[2]), vshrq_n_u32::<3>(x56[3])];
        let t324: [uint32x4_t; 2] = [t323[0], t323[1]];
        let t325: [uint32x4_t; 4] = [t324[0], t324[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        let t326 = (arg3 as *const u8).add(((gid0.wrapping_add((14u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t326 as *mut u32).add(0), uint32x4x2_t(t325[0], t325[2]));
        vst2q_u32((t326 as *mut u32).add(8), uint32x4x2_t(t325[1], t325[3]));
        let t327: u64 = 1u64;
        let t328 = [vdupq_n_u32((t327) as u32); 2];
        let t329: [uint32x4_t; 2] = [vandq_u32(x56[0], t328[0]), vandq_u32(x56[1], t328[1])];
        let t330: [uint32x4_t; 2] = [vtstq_u32(t329[0], t329[0]), vtstq_u32(t329[1], t329[1])];
        {
            let t331: [uint32x4_t; 4] = [veorq_u32(x56[0], x64[0]), veorq_u32(x56[1], x64[1]), veorq_u32(x56[2], x64[2]), veorq_u32(x56[3], x64[3])];
            x104 = [vbslq_u32(t330[0], t331[0], x104[0]), vbslq_u32(t330[1], t331[1], x104[1]), vbslq_u32(t330[0], t331[2], x104[2]), vbslq_u32(t330[1], t331[3], x104[3])];
        }
        let t332: [uint32x4_t; 2] = [vmvnq_u32(t330[0]), vmvnq_u32(t330[1])];
        {
            let t333: [uint32x4_t; 2] = [vaddq_u32(x56[0], x72[0]), vaddq_u32(x56[1], x72[1])];
            let t334: [uint32x4_t; 2] = [vcltq_u32(t333[0], x56[0]), vcltq_u32(t333[1], x56[1])];
            let t335: [uint32x4_t; 4] = [t333[0], t333[1], vsubq_u32(vaddq_u32(x56[2], vdupq_n_u32(0)), t334[0]), vsubq_u32(vaddq_u32(x56[3], vdupq_n_u32(0)), t334[1])];
            x104 = [vbslq_u32(t332[0], t335[0], x104[0]), vbslq_u32(t332[1], t335[1], x104[1]), vbslq_u32(t332[0], t335[2], x104[2]), vbslq_u32(t332[1], t335[3], x104[3])];
        }
        let t336 = (arg3 as *const u8).add(((gid0.wrapping_add((15u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t336 as *mut u32).add(0), uint32x4x2_t(x104[0], x104[2]));
        vst2q_u32((t336 as *mut u32).add(8), uint32x4x2_t(x104[1], x104[3]));
        let t337 = b1_l_f(l, lid0, x56);
        let t338 = b2_l_f(l, lid0, x64);
        let t339: [uint32x4_t; 4] = [vshlq_n_u32::<8>(t338[0]), vshlq_n_u32::<8>(t338[1]), vshrq_n_u32::<24>(t338[0]), vshrq_n_u32::<24>(t338[1])];
        let t340: [uint32x4_t; 2] = [vaddq_u32(t337[0], t339[0]), vaddq_u32(t337[1], t339[1])];
        let t341: [uint32x4_t; 2] = [vcltq_u32(t340[0], t337[0]), vcltq_u32(t340[1], t337[1])];
        let t342: [uint32x4_t; 4] = [t340[0], t340[1], vsubq_u32(vaddq_u32(vdupq_n_u32(0), t339[2]), t341[0]), vsubq_u32(vaddq_u32(vdupq_n_u32(0), t339[3]), t341[1])];
        let t343 = (arg3 as *const u8).add(((gid0.wrapping_add((16u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t343 as *mut u32).add(0), uint32x4x2_t(t342[0], t342[2]));
        vst2q_u32((t343 as *mut u32).add(8), uint32x4x2_t(t342[1], t342[3]));
        let t344 = [vdupq_n_u32(x88); 2];
        let mut t345x = [0u32; 8];
        let mut t345y = [0u32; 8];
        for r in 0..2 { vst1q_u32(t345x.as_mut_ptr().add(4 * r), x72[r]); vst1q_u32(t345y.as_mut_ptr().add(4 * r), t344[r]); }
        for j in 0..8 { t345x[j] = (t345x[j] as u32).checked_rem(t345y[j] as u32).unwrap_or(0) as u32; }
        let t346: [uint32x4_t; 2] = [vld1q_u32(t345x.as_ptr().add(0)), vld1q_u32(t345x.as_ptr().add(4))];
        let mut t347: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t346[0]) as usize * 8) as *const u64).read_unaligned(); t347[0] = vsetq_lane_u32::<0>(v as u32, t347[0]); t347[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t347[2]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t346[0]) as usize * 8) as *const u64).read_unaligned(); t347[0] = vsetq_lane_u32::<1>(v as u32, t347[0]); t347[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t347[2]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t346[0]) as usize * 8) as *const u64).read_unaligned(); t347[0] = vsetq_lane_u32::<2>(v as u32, t347[0]); t347[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t347[2]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t346[0]) as usize * 8) as *const u64).read_unaligned(); t347[0] = vsetq_lane_u32::<3>(v as u32, t347[0]); t347[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t347[2]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t346[1]) as usize * 8) as *const u64).read_unaligned(); t347[1] = vsetq_lane_u32::<0>(v as u32, t347[1]); t347[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t347[3]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t346[1]) as usize * 8) as *const u64).read_unaligned(); t347[1] = vsetq_lane_u32::<1>(v as u32, t347[1]); t347[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t347[3]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t346[1]) as usize * 8) as *const u64).read_unaligned(); t347[1] = vsetq_lane_u32::<2>(v as u32, t347[1]); t347[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t347[3]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t346[1]) as usize * 8) as *const u64).read_unaligned(); t347[1] = vsetq_lane_u32::<3>(v as u32, t347[1]); t347[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t347[3]); }
        let t348 = (arg3 as *const u8).add(((gid0.wrapping_add((17u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t348 as *mut u32).add(0), uint32x4x2_t(t347[0], t347[2]));
        vst2q_u32((t348 as *mut u32).add(8), uint32x4x2_t(t347[1], t347[3]));
        let t349 = b3_ll_f(l, lid0, x56, x72);
        let t350 = (arg3 as *const u8).add(((gid0.wrapping_add((18u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t350 as *mut u32).add(0), uint32x4x2_t(t349[0], t349[2]));
        vst2q_u32((t350 as *mut u32).add(8), uint32x4x2_t(t349[1], t349[3]));
        x112 = x56;
        x120 = 0u32;
        let mut t355 = live;
        loop {
            // Lanes part: each its own rounds.
            let t357 = [vdupq_n_u32(7u32); 2];
            let t358: [uint32x4_t; 2] = [vandq_u32(x72[0], t357[0]), vandq_u32(x72[1], t357[1])];
            let t359 = [vdupq_n_u32(x120); 2];
            let t360: [uint32x4_t; 2] = [vcltq_u32(t359[0], t358[0]), vcltq_u32(t359[1], t358[1])];
            t355 = [vandq_u32(t355[0], t360[0]), vandq_u32(t355[1], t360[1])];
            if !(any(&t355)) { break; }
            let t362: u64 = 3u64;
            let t363 = [vdupq_n_u32((t362) as u32); 2];
            let t364: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x112[0]), vget_low_u32(t363[0]))), vreinterpretq_u32_u64(vmull_high_u32(x112[0], t363[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x112[1]), vget_low_u32(t363[1]))), vreinterpretq_u32_u64(vmull_high_u32(x112[1], t363[1])))];
            let t365: [uint32x4_t; 4] = [vmulq_u32(x112[0], t363[0]), vmulq_u32(x112[1], t363[1]), vmlaq_u32(t364[0], t363[0], x112[2]), vmlaq_u32(t364[1], t363[1], x112[3])];
            let t366: [uint32x4_t; 2] = [vaddq_u32(t365[0], x64[0]), vaddq_u32(t365[1], x64[1])];
            let t367: [uint32x4_t; 2] = [vcltq_u32(t366[0], t365[0]), vcltq_u32(t366[1], t365[1])];
            let t368: [uint32x4_t; 4] = [t366[0], t366[1], vsubq_u32(vaddq_u32(t365[2], x64[2]), t367[0]), vsubq_u32(vaddq_u32(t365[3], x64[3]), t367[1])];
            x112 = [vbslq_u32(t355[0], t368[0], x112[0]), vbslq_u32(t355[1], t368[1], x112[1]), vbslq_u32(t355[0], t368[2], x112[2]), vbslq_u32(t355[1], t368[3], x112[3])];
            let t369: [uint32x4_t; 2] = [vandq_u32(t355[0], live[0]), vandq_u32(t355[1], live[1])];
            let t370 = x120;
            x120 = (x120).wrapping_add(1u32);
        }
        let t371 = (arg3 as *const u8).add(((gid0.wrapping_add((19u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t371 as *mut u32).add(0), uint32x4x2_t(x112[0], x112[2]));
        vst2q_u32((t371 as *mut u32).add(8), uint32x4x2_t(x112[1], x112[3]));
        let t372: u64 = a1;
        let t373 = [vdupq_n_u32((t372) as u32); 2];
        let t374 = [vdupq_n_u32(((t372) >> 32) as u32); 2];
        let t375: [uint32x4_t; 2] = [vaddq_u32(x56[0], t373[0]), vaddq_u32(x56[1], t373[1])];
        let t376: [uint32x4_t; 2] = [vcltq_u32(t375[0], x56[0]), vcltq_u32(t375[1], x56[1])];
        let t377: [uint32x4_t; 4] = [t375[0], t375[1], vsubq_u32(vaddq_u32(x56[2], t374[0]), t376[0]), vsubq_u32(vaddq_u32(x56[3], t374[1]), t376[1])];
        let t378: u64 = (a1).wrapping_shl((3u64) as u32);
        let t379 = [vdupq_n_u32((t378) as u32); 2];
        let t380 = [vdupq_n_u32(((t378) >> 32) as u32); 2];
        let t381: [uint32x4_t; 2] = [vaddq_u32(t377[0], t379[0]), vaddq_u32(t377[1], t379[1])];
        let t382: [uint32x4_t; 2] = [vcltq_u32(t381[0], t377[0]), vcltq_u32(t381[1], t377[1])];
        let t383: [uint32x4_t; 4] = [t381[0], t381[1], vsubq_u32(vaddq_u32(t377[2], t380[0]), t382[0]), vsubq_u32(vaddq_u32(t377[3], t380[1]), t382[1])];
        let t384 = (arg3 as *const u8).add(((gid0.wrapping_add((20u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t384 as *mut u32).add(0), uint32x4x2_t(t383[0], t383[2]));
        vst2q_u32((t384 as *mut u32).add(8), uint32x4x2_t(t383[1], t383[3]));
        let t385: u64 = 4294967295u64;
        let t386 = [vdupq_n_u32((t385) as u32); 2];
        let t387: [uint32x4_t; 2] = [vandq_u32(x56[0], t386[0]), vandq_u32(x56[1], t386[1])];
        let t388: u64 = 4294967295u64;
        let t389 = [vdupq_n_u32((t388) as u32); 2];
        let t390: [uint32x4_t; 2] = [vandq_u32(x64[0], t389[0]), vandq_u32(x64[1], t389[1])];
        let t391: [uint32x4_t; 2] = [vceqq_u32(t387[0], t390[0]), vceqq_u32(t387[1], t390[1])];
        let mut t392: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t393: [uint32x4_t; 2] = [vmvnq_u32(t391[0]), vmvnq_u32(t391[1])];
        {
            let t394 = [vdupq_n_u32(1u32); 2];
            t392 = [vbslq_u32(t391[0], t394[0], t392[0]), vbslq_u32(t391[1], t394[1], t392[1])];
        }
        {
            let t395 = [vdupq_n_u32(2u32); 2];
            t392 = [vbslq_u32(t393[0], t395[0], t392[0]), vbslq_u32(t393[1], t395[1], t392[1])];
        }
        let t396: [uint32x4_t; 4] = [t392[0], t392[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        let t397 = (arg3 as *const u8).add(((gid0.wrapping_add((21u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t397 as *mut u32).add(0), uint32x4x2_t(t396[0], t396[2]));
        vst2q_u32((t397 as *mut u32).add(8), uint32x4x2_t(t396[1], t396[3]));
        let t398: [uint32x4_t; 2] = [vtstq_u32(vorrq_u32(x56[0], x56[2]), vorrq_u32(x56[0], x56[2])), vtstq_u32(vorrq_u32(x56[1], x56[3]), vorrq_u32(x56[1], x56[3]))];
        let mut t399: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t400: [uint32x4_t; 2] = [vmvnq_u32(t398[0]), vmvnq_u32(t398[1])];
        {
            let t401 = [vdupq_n_u32(5u32); 2];
            t399 = [vbslq_u32(t398[0], t401[0], t399[0]), vbslq_u32(t398[1], t401[1], t399[1])];
        }
        {
            let t402 = [vdupq_n_u32(6u32); 2];
            t399 = [vbslq_u32(t400[0], t402[0], t399[0]), vbslq_u32(t400[1], t402[1], t399[1])];
        }
        let t403: [uint32x4_t; 4] = [t399[0], t399[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        let t404 = (arg3 as *const u8).add(((gid0.wrapping_add((22u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t404 as *mut u32).add(0), uint32x4x2_t(t403[0], t403[2]));
        vst2q_u32((t404 as *mut u32).add(8), uint32x4x2_t(t403[1], t403[3]));
        let t405 = b4_ll_f(l, lid0, x56, x64);
        let t406 = (arg3 as *const u8).add(((gid0.wrapping_add((23u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t406 as *mut u32).add(0), uint32x4x2_t(t405[0], t405[2]));
        vst2q_u32((t406 as *mut u32).add(8), uint32x4x2_t(t405[1], t405[3]));
        let t407 = b5_r0ns_f_v0(l, lid0, arg0, gid0.wrapping_mul(x96), x96, 8u32);
        let t408 = (arg3 as *const u8).add(((gid0.wrapping_add((24u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t408 as *mut u32).add(0), uint32x4x2_t(t407[0], t407[2]));
        vst2q_u32((t408 as *mut u32).add(8), uint32x4x2_t(t407[1], t407[3]));
        let t409 = if (gid0.wrapping_add(3u32) & 255u32) <= 248u32 { b6_u8_f(l, lid0, gid0.wrapping_add(3u32)) } else { b6_i_f(l, lid0, gid0.wrapping_add(3u32)) };
        let t410 = (arg3 as *const u8).add(((gid0.wrapping_add((25u32).wrapping_mul(x88))) as usize) * 8);
        vst2q_u32((t410 as *mut u32).add(0), uint32x4x2_t(t409[0], t409[2]));
        vst2q_u32((t410 as *mut u32).add(8), uint32x4x2_t(t409[1], t409[3]));
    } else if (((gid0) as u64 + 7) <= u32::MAX as u64 && ((gid0) as u64) >= ((a0) as u64)) {
        return;
    } else {
        // Lanes part: the chunk's lanes on both sides of the bound.
        let t411: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t412 = [vdupq_n_u32(a0); 2];
        let t413: [uint32x4_t; 2] = [vcgeq_u32(t411[0], t412[0]), vcgeq_u32(t411[1], t412[1])];
        let t414: [uint32x4_t; 2] = [vandq_u32(live[0], t413[0]), vandq_u32(live[1], t413[1])];
        {
            live = [vbicq_u32(live[0], t414[0]), vbicq_u32(live[1], t414[1])];
        }
        if any(&live) {
            let t415 = (arg0 as *const u8).add(((gid0) as usize) * 8);
            let mut t416: [uint32x4_t; 4];
            if all(&live) {
                let t4170 = vld2q_u32((t415 as *const u32).add(0));
                let t4171 = vld2q_u32((t415 as *const u32).add(8));
                let t417: [uint32x4_t; 4] = [t4170.0, t4171.0, t4170.1, t4171.1];
                t416 = t417;
            } else {
                let t418 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t411[0]), vandq_u32(live[1], t411[1]))));
                let t419: [uint32x4_t; 2] = [vbslq_u32(live[0], t411[0], t418), vbslq_u32(live[1], t411[1], t418)];
                let mut t420: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t419[0]) as usize * 8) as *const u64).read_unaligned(); t420[0] = vsetq_lane_u32::<0>(v as u32, t420[0]); t420[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t420[2]); }
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t419[0]) as usize * 8) as *const u64).read_unaligned(); t420[0] = vsetq_lane_u32::<1>(v as u32, t420[0]); t420[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t420[2]); }
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t419[0]) as usize * 8) as *const u64).read_unaligned(); t420[0] = vsetq_lane_u32::<2>(v as u32, t420[0]); t420[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t420[2]); }
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t419[0]) as usize * 8) as *const u64).read_unaligned(); t420[0] = vsetq_lane_u32::<3>(v as u32, t420[0]); t420[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t420[2]); }
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t419[1]) as usize * 8) as *const u64).read_unaligned(); t420[1] = vsetq_lane_u32::<0>(v as u32, t420[1]); t420[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t420[3]); }
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t419[1]) as usize * 8) as *const u64).read_unaligned(); t420[1] = vsetq_lane_u32::<1>(v as u32, t420[1]); t420[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t420[3]); }
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t419[1]) as usize * 8) as *const u64).read_unaligned(); t420[1] = vsetq_lane_u32::<2>(v as u32, t420[1]); t420[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t420[3]); }
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t419[1]) as usize * 8) as *const u64).read_unaligned(); t420[1] = vsetq_lane_u32::<3>(v as u32, t420[1]); t420[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t420[3]); }
                t416 = t420;
            }
            x56 = t416;
            let t421 = (arg1 as *const u8).add(((gid0) as usize) * 8);
            let mut t422: [uint32x4_t; 4];
            if all(&live) {
                let t4230 = vld2q_u32((t421 as *const u32).add(0));
                let t4231 = vld2q_u32((t421 as *const u32).add(8));
                let t423: [uint32x4_t; 4] = [t4230.0, t4231.0, t4230.1, t4231.1];
                t422 = t423;
            } else {
                let t424 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t411[0]), vandq_u32(live[1], t411[1]))));
                let t425: [uint32x4_t; 2] = [vbslq_u32(live[0], t411[0], t424), vbslq_u32(live[1], t411[1], t424)];
                let mut t426: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
                { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<0>(t425[0]) as usize * 8) as *const u64).read_unaligned(); t426[0] = vsetq_lane_u32::<0>(v as u32, t426[0]); t426[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t426[2]); }
                { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<1>(t425[0]) as usize * 8) as *const u64).read_unaligned(); t426[0] = vsetq_lane_u32::<1>(v as u32, t426[0]); t426[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t426[2]); }
                { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<2>(t425[0]) as usize * 8) as *const u64).read_unaligned(); t426[0] = vsetq_lane_u32::<2>(v as u32, t426[0]); t426[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t426[2]); }
                { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<3>(t425[0]) as usize * 8) as *const u64).read_unaligned(); t426[0] = vsetq_lane_u32::<3>(v as u32, t426[0]); t426[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t426[2]); }
                { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<0>(t425[1]) as usize * 8) as *const u64).read_unaligned(); t426[1] = vsetq_lane_u32::<0>(v as u32, t426[1]); t426[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t426[3]); }
                { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<1>(t425[1]) as usize * 8) as *const u64).read_unaligned(); t426[1] = vsetq_lane_u32::<1>(v as u32, t426[1]); t426[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t426[3]); }
                { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<2>(t425[1]) as usize * 8) as *const u64).read_unaligned(); t426[1] = vsetq_lane_u32::<2>(v as u32, t426[1]); t426[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t426[3]); }
                { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<3>(t425[1]) as usize * 8) as *const u64).read_unaligned(); t426[1] = vsetq_lane_u32::<3>(v as u32, t426[1]); t426[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t426[3]); }
                t422 = t426;
            }
            x64 = t422;
            let mut t427: [uint32x4_t; 2];
            if all(&live) {
                let t428 = (arg2 as *const u32).add((gid0) as usize);
                t427 = [vld1q_u32(t428.add(0)), vld1q_u32(t428.add(4))];
            } else {
                let t429 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t411[0]), vandq_u32(live[1], t411[1]))));
                let t430: [uint32x4_t; 2] = [vbslq_u32(live[0], t411[0], t429), vbslq_u32(live[1], t411[1], t429)];
                let t431 = arg2 as *const u32;
                let mut t432: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                t432[0] = vld1q_lane_u32::<0>(t431.add(vgetq_lane_u32::<0>(t430[0]) as usize), t432[0]);
                t432[0] = vld1q_lane_u32::<1>(t431.add(vgetq_lane_u32::<1>(t430[0]) as usize), t432[0]);
                t432[0] = vld1q_lane_u32::<2>(t431.add(vgetq_lane_u32::<2>(t430[0]) as usize), t432[0]);
                t432[0] = vld1q_lane_u32::<3>(t431.add(vgetq_lane_u32::<3>(t430[0]) as usize), t432[0]);
                t432[1] = vld1q_lane_u32::<0>(t431.add(vgetq_lane_u32::<0>(t430[1]) as usize), t432[1]);
                t432[1] = vld1q_lane_u32::<1>(t431.add(vgetq_lane_u32::<1>(t430[1]) as usize), t432[1]);
                t432[1] = vld1q_lane_u32::<2>(t431.add(vgetq_lane_u32::<2>(t430[1]) as usize), t432[1]);
                t432[1] = vld1q_lane_u32::<3>(t431.add(vgetq_lane_u32::<3>(t430[1]) as usize), t432[1]);
                t427 = t432;
            }
            x72 = t427;
            let t433 = [vdupq_n_u32(63u32); 2];
            let t434: [uint32x4_t; 2] = [vandq_u32(x72[0], t433[0]), vandq_u32(x72[1], t433[1])];
            x80 = t434;
            x88 = a0;
            let t435: u32;
            if ((((a1) & (1u64))) != (0u64)) {
                t435 = 8u32;
            } else {
                t435 = 4u32;
            }
            x96 = t435;
            let t436: [uint32x4_t; 2] = [vaddq_u32(x56[0], x64[0]), vaddq_u32(x56[1], x64[1])];
            let t437: [uint32x4_t; 2] = [vcltq_u32(t436[0], x56[0]), vcltq_u32(t436[1], x56[1])];
            let t438: [uint32x4_t; 4] = [t436[0], t436[1], vsubq_u32(vaddq_u32(x56[2], x64[2]), t437[0]), vsubq_u32(vaddq_u32(x56[3], x64[3]), t437[1])];
            let t439 = (arg3 as *const u8).add(((gid0.wrapping_add((0u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t439 as *mut u32).add(0), uint32x4x2_t(t438[0], t438[2]));
                vst2q_u32((t439 as *mut u32).add(8), uint32x4x2_t(t438[1], t438[3]));
            } else {
                let mut t440w = 0u64;
                let t440 = core::hint::black_box(&raw mut t440w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t439 as *mut u64).add(0), t440).write_unaligned(((vgetq_lane_u32::<0>(t438[2]) as u64) << 32 | vgetq_lane_u32::<0>(t438[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t439 as *mut u64).add(1), t440).write_unaligned(((vgetq_lane_u32::<1>(t438[2]) as u64) << 32 | vgetq_lane_u32::<1>(t438[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t439 as *mut u64).add(2), t440).write_unaligned(((vgetq_lane_u32::<2>(t438[2]) as u64) << 32 | vgetq_lane_u32::<2>(t438[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t439 as *mut u64).add(3), t440).write_unaligned(((vgetq_lane_u32::<3>(t438[2]) as u64) << 32 | vgetq_lane_u32::<3>(t438[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t439 as *mut u64).add(4), t440).write_unaligned(((vgetq_lane_u32::<0>(t438[3]) as u64) << 32 | vgetq_lane_u32::<0>(t438[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t439 as *mut u64).add(5), t440).write_unaligned(((vgetq_lane_u32::<1>(t438[3]) as u64) << 32 | vgetq_lane_u32::<1>(t438[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t439 as *mut u64).add(6), t440).write_unaligned(((vgetq_lane_u32::<2>(t438[3]) as u64) << 32 | vgetq_lane_u32::<2>(t438[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t439 as *mut u64).add(7), t440).write_unaligned(((vgetq_lane_u32::<3>(t438[3]) as u64) << 32 | vgetq_lane_u32::<3>(t438[1]) as u64));
            }
            let t441: [uint32x4_t; 2] = [vcltq_u32(x56[0], x64[0]), vcltq_u32(x56[1], x64[1])];
            let t442: [uint32x4_t; 4] = [vsubq_u32(x56[0], x64[0]), vsubq_u32(x56[1], x64[1]), vaddq_u32(vsubq_u32(x56[2], x64[2]), t441[0]), vaddq_u32(vsubq_u32(x56[3], x64[3]), t441[1])];
            let t443 = (arg3 as *const u8).add(((gid0.wrapping_add((1u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t443 as *mut u32).add(0), uint32x4x2_t(t442[0], t442[2]));
                vst2q_u32((t443 as *mut u32).add(8), uint32x4x2_t(t442[1], t442[3]));
            } else {
                let mut t444w = 0u64;
                let t444 = core::hint::black_box(&raw mut t444w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t443 as *mut u64).add(0), t444).write_unaligned(((vgetq_lane_u32::<0>(t442[2]) as u64) << 32 | vgetq_lane_u32::<0>(t442[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t443 as *mut u64).add(1), t444).write_unaligned(((vgetq_lane_u32::<1>(t442[2]) as u64) << 32 | vgetq_lane_u32::<1>(t442[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t443 as *mut u64).add(2), t444).write_unaligned(((vgetq_lane_u32::<2>(t442[2]) as u64) << 32 | vgetq_lane_u32::<2>(t442[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t443 as *mut u64).add(3), t444).write_unaligned(((vgetq_lane_u32::<3>(t442[2]) as u64) << 32 | vgetq_lane_u32::<3>(t442[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t443 as *mut u64).add(4), t444).write_unaligned(((vgetq_lane_u32::<0>(t442[3]) as u64) << 32 | vgetq_lane_u32::<0>(t442[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t443 as *mut u64).add(5), t444).write_unaligned(((vgetq_lane_u32::<1>(t442[3]) as u64) << 32 | vgetq_lane_u32::<1>(t442[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t443 as *mut u64).add(6), t444).write_unaligned(((vgetq_lane_u32::<2>(t442[3]) as u64) << 32 | vgetq_lane_u32::<2>(t442[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t443 as *mut u64).add(7), t444).write_unaligned(((vgetq_lane_u32::<3>(t442[3]) as u64) << 32 | vgetq_lane_u32::<3>(t442[1]) as u64));
            }
            let t445: [uint32x4_t; 2] = [x80[0], x80[1]];
            let t446: [uint32x4_t; 2] = [vandq_u32(t445[0], vdupq_n_u32(63)), vandq_u32(t445[1], vdupq_n_u32(63))];
            let t447: [uint32x4_t; 4] = [vshlq_u32(x56[0], vreinterpretq_s32_u32(t446[0])), vshlq_u32(x56[1], vreinterpretq_s32_u32(t446[1])), vorrq_u32(vshlq_u32(x56[2], vreinterpretq_s32_u32(t446[0])), vshlq_u32(x56[0], vsubq_s32(vreinterpretq_s32_u32(t446[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(x56[3], vreinterpretq_s32_u32(t446[1])), vshlq_u32(x56[1], vsubq_s32(vreinterpretq_s32_u32(t446[1]), vdupq_n_s32(32))))];
            let t448 = (arg3 as *const u8).add(((gid0.wrapping_add((2u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t448 as *mut u32).add(0), uint32x4x2_t(t447[0], t447[2]));
                vst2q_u32((t448 as *mut u32).add(8), uint32x4x2_t(t447[1], t447[3]));
            } else {
                let mut t449w = 0u64;
                let t449 = core::hint::black_box(&raw mut t449w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t448 as *mut u64).add(0), t449).write_unaligned(((vgetq_lane_u32::<0>(t447[2]) as u64) << 32 | vgetq_lane_u32::<0>(t447[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t448 as *mut u64).add(1), t449).write_unaligned(((vgetq_lane_u32::<1>(t447[2]) as u64) << 32 | vgetq_lane_u32::<1>(t447[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t448 as *mut u64).add(2), t449).write_unaligned(((vgetq_lane_u32::<2>(t447[2]) as u64) << 32 | vgetq_lane_u32::<2>(t447[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t448 as *mut u64).add(3), t449).write_unaligned(((vgetq_lane_u32::<3>(t447[2]) as u64) << 32 | vgetq_lane_u32::<3>(t447[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t448 as *mut u64).add(4), t449).write_unaligned(((vgetq_lane_u32::<0>(t447[3]) as u64) << 32 | vgetq_lane_u32::<0>(t447[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t448 as *mut u64).add(5), t449).write_unaligned(((vgetq_lane_u32::<1>(t447[3]) as u64) << 32 | vgetq_lane_u32::<1>(t447[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t448 as *mut u64).add(6), t449).write_unaligned(((vgetq_lane_u32::<2>(t447[3]) as u64) << 32 | vgetq_lane_u32::<2>(t447[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t448 as *mut u64).add(7), t449).write_unaligned(((vgetq_lane_u32::<3>(t447[3]) as u64) << 32 | vgetq_lane_u32::<3>(t447[1]) as u64));
            }
            let t450: [uint32x4_t; 2] = [x80[0], x80[1]];
            let t451: [uint32x4_t; 2] = [vandq_u32(t450[0], vdupq_n_u32(63)), vandq_u32(t450[1], vdupq_n_u32(63))];
            let t452: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(x56[0], vnegq_s32(vreinterpretq_s32_u32(t451[0]))), vshlq_u32(x56[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t451[0])))), vorrq_u32(vshlq_u32(x56[1], vnegq_s32(vreinterpretq_s32_u32(t451[1]))), vshlq_u32(x56[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t451[1])))), vshlq_u32(x56[2], vnegq_s32(vreinterpretq_s32_u32(t451[0]))), vshlq_u32(x56[3], vnegq_s32(vreinterpretq_s32_u32(t451[1])))];
            let t453 = (arg3 as *const u8).add(((gid0.wrapping_add((3u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t453 as *mut u32).add(0), uint32x4x2_t(t452[0], t452[2]));
                vst2q_u32((t453 as *mut u32).add(8), uint32x4x2_t(t452[1], t452[3]));
            } else {
                let mut t454w = 0u64;
                let t454 = core::hint::black_box(&raw mut t454w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t453 as *mut u64).add(0), t454).write_unaligned(((vgetq_lane_u32::<0>(t452[2]) as u64) << 32 | vgetq_lane_u32::<0>(t452[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t453 as *mut u64).add(1), t454).write_unaligned(((vgetq_lane_u32::<1>(t452[2]) as u64) << 32 | vgetq_lane_u32::<1>(t452[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t453 as *mut u64).add(2), t454).write_unaligned(((vgetq_lane_u32::<2>(t452[2]) as u64) << 32 | vgetq_lane_u32::<2>(t452[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t453 as *mut u64).add(3), t454).write_unaligned(((vgetq_lane_u32::<3>(t452[2]) as u64) << 32 | vgetq_lane_u32::<3>(t452[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t453 as *mut u64).add(4), t454).write_unaligned(((vgetq_lane_u32::<0>(t452[3]) as u64) << 32 | vgetq_lane_u32::<0>(t452[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t453 as *mut u64).add(5), t454).write_unaligned(((vgetq_lane_u32::<1>(t452[3]) as u64) << 32 | vgetq_lane_u32::<1>(t452[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t453 as *mut u64).add(6), t454).write_unaligned(((vgetq_lane_u32::<2>(t452[3]) as u64) << 32 | vgetq_lane_u32::<2>(t452[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t453 as *mut u64).add(7), t454).write_unaligned(((vgetq_lane_u32::<3>(t452[3]) as u64) << 32 | vgetq_lane_u32::<3>(t452[1]) as u64));
            }
            let t455: [uint32x4_t; 2] = [x80[0], x80[1]];
            let t456: [uint32x4_t; 2] = [vandq_u32(t455[0], vdupq_n_u32(63)), vandq_u32(t455[1], vdupq_n_u32(63))];
            let t457: [uint32x4_t; 2] = [vcgeq_s32(vreinterpretq_s32_u32(t456[0]), vdupq_n_s32(32)), vcgeq_s32(vreinterpretq_s32_u32(t456[1]), vdupq_n_s32(32))];
            let t458: [uint32x4_t; 4] = [vbslq_u32(t457[0], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x56[2]), vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t456[0])))), vorrq_u32(vshlq_u32(x56[0], vnegq_s32(vreinterpretq_s32_u32(t456[0]))), vshlq_u32(x56[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t456[0]))))), vbslq_u32(t457[1], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x56[3]), vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t456[1])))), vorrq_u32(vshlq_u32(x56[1], vnegq_s32(vreinterpretq_s32_u32(t456[1]))), vshlq_u32(x56[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t456[1]))))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x56[2]), vnegq_s32(vreinterpretq_s32_u32(t456[0])))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x56[3]), vnegq_s32(vreinterpretq_s32_u32(t456[1]))))];
            let t459 = (arg3 as *const u8).add(((gid0.wrapping_add((4u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t459 as *mut u32).add(0), uint32x4x2_t(t458[0], t458[2]));
                vst2q_u32((t459 as *mut u32).add(8), uint32x4x2_t(t458[1], t458[3]));
            } else {
                let mut t460w = 0u64;
                let t460 = core::hint::black_box(&raw mut t460w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t459 as *mut u64).add(0), t460).write_unaligned(((vgetq_lane_u32::<0>(t458[2]) as u64) << 32 | vgetq_lane_u32::<0>(t458[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t459 as *mut u64).add(1), t460).write_unaligned(((vgetq_lane_u32::<1>(t458[2]) as u64) << 32 | vgetq_lane_u32::<1>(t458[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t459 as *mut u64).add(2), t460).write_unaligned(((vgetq_lane_u32::<2>(t458[2]) as u64) << 32 | vgetq_lane_u32::<2>(t458[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t459 as *mut u64).add(3), t460).write_unaligned(((vgetq_lane_u32::<3>(t458[2]) as u64) << 32 | vgetq_lane_u32::<3>(t458[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t459 as *mut u64).add(4), t460).write_unaligned(((vgetq_lane_u32::<0>(t458[3]) as u64) << 32 | vgetq_lane_u32::<0>(t458[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t459 as *mut u64).add(5), t460).write_unaligned(((vgetq_lane_u32::<1>(t458[3]) as u64) << 32 | vgetq_lane_u32::<1>(t458[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t459 as *mut u64).add(6), t460).write_unaligned(((vgetq_lane_u32::<2>(t458[3]) as u64) << 32 | vgetq_lane_u32::<2>(t458[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t459 as *mut u64).add(7), t460).write_unaligned(((vgetq_lane_u32::<3>(t458[3]) as u64) << 32 | vgetq_lane_u32::<3>(t458[1]) as u64));
            }
            let t461: [uint32x4_t; 4] = [vshlq_n_u32::<13>(x56[0]), vshlq_n_u32::<13>(x56[1]), vorrq_u32(vshlq_n_u32::<13>(x56[2]), vshrq_n_u32::<19>(x56[0])), vorrq_u32(vshlq_n_u32::<13>(x56[3]), vshrq_n_u32::<19>(x56[1]))];
            let t462: [uint32x4_t; 2] = [vshrq_n_u32::<19>(x64[2]), vshrq_n_u32::<19>(x64[3])];
            let t463: [uint32x4_t; 4] = [vorrq_u32(t461[0], t462[0]), vorrq_u32(t461[1], t462[1]), t461[2], t461[3]];
            let t464 = (arg3 as *const u8).add(((gid0.wrapping_add((5u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t464 as *mut u32).add(0), uint32x4x2_t(t463[0], t463[2]));
                vst2q_u32((t464 as *mut u32).add(8), uint32x4x2_t(t463[1], t463[3]));
            } else {
                let mut t465w = 0u64;
                let t465 = core::hint::black_box(&raw mut t465w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t464 as *mut u64).add(0), t465).write_unaligned(((vgetq_lane_u32::<0>(t463[2]) as u64) << 32 | vgetq_lane_u32::<0>(t463[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t464 as *mut u64).add(1), t465).write_unaligned(((vgetq_lane_u32::<1>(t463[2]) as u64) << 32 | vgetq_lane_u32::<1>(t463[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t464 as *mut u64).add(2), t465).write_unaligned(((vgetq_lane_u32::<2>(t463[2]) as u64) << 32 | vgetq_lane_u32::<2>(t463[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t464 as *mut u64).add(3), t465).write_unaligned(((vgetq_lane_u32::<3>(t463[2]) as u64) << 32 | vgetq_lane_u32::<3>(t463[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t464 as *mut u64).add(4), t465).write_unaligned(((vgetq_lane_u32::<0>(t463[3]) as u64) << 32 | vgetq_lane_u32::<0>(t463[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t464 as *mut u64).add(5), t465).write_unaligned(((vgetq_lane_u32::<1>(t463[3]) as u64) << 32 | vgetq_lane_u32::<1>(t463[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t464 as *mut u64).add(6), t465).write_unaligned(((vgetq_lane_u32::<2>(t463[3]) as u64) << 32 | vgetq_lane_u32::<2>(t463[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t464 as *mut u64).add(7), t465).write_unaligned(((vgetq_lane_u32::<3>(t463[3]) as u64) << 32 | vgetq_lane_u32::<3>(t463[1]) as u64));
            }
            let t466: [uint32x4_t; 4] = [vdupq_n_u32(0), vdupq_n_u32(0), vshlq_n_u32::<8>(x56[0]), vshlq_n_u32::<8>(x56[1])];
            let t467 = (arg3 as *const u8).add(((gid0.wrapping_add((6u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t467 as *mut u32).add(0), uint32x4x2_t(t466[0], t466[2]));
                vst2q_u32((t467 as *mut u32).add(8), uint32x4x2_t(t466[1], t466[3]));
            } else {
                let mut t468w = 0u64;
                let t468 = core::hint::black_box(&raw mut t468w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t467 as *mut u64).add(0), t468).write_unaligned(((vgetq_lane_u32::<0>(t466[2]) as u64) << 32 | vgetq_lane_u32::<0>(t466[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t467 as *mut u64).add(1), t468).write_unaligned(((vgetq_lane_u32::<1>(t466[2]) as u64) << 32 | vgetq_lane_u32::<1>(t466[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t467 as *mut u64).add(2), t468).write_unaligned(((vgetq_lane_u32::<2>(t466[2]) as u64) << 32 | vgetq_lane_u32::<2>(t466[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t467 as *mut u64).add(3), t468).write_unaligned(((vgetq_lane_u32::<3>(t466[2]) as u64) << 32 | vgetq_lane_u32::<3>(t466[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t467 as *mut u64).add(4), t468).write_unaligned(((vgetq_lane_u32::<0>(t466[3]) as u64) << 32 | vgetq_lane_u32::<0>(t466[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t467 as *mut u64).add(5), t468).write_unaligned(((vgetq_lane_u32::<1>(t466[3]) as u64) << 32 | vgetq_lane_u32::<1>(t466[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t467 as *mut u64).add(6), t468).write_unaligned(((vgetq_lane_u32::<2>(t466[3]) as u64) << 32 | vgetq_lane_u32::<2>(t466[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t467 as *mut u64).add(7), t468).write_unaligned(((vgetq_lane_u32::<3>(t466[3]) as u64) << 32 | vgetq_lane_u32::<3>(t466[1]) as u64));
            }
            let t469: [uint32x4_t; 2] = [vshrq_n_u32::<3>(x56[2]), vshrq_n_u32::<3>(x56[3])];
            let t470: [uint32x4_t; 4] = [t469[0], t469[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            let t471 = (arg3 as *const u8).add(((gid0.wrapping_add((7u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t471 as *mut u32).add(0), uint32x4x2_t(t470[0], t470[2]));
                vst2q_u32((t471 as *mut u32).add(8), uint32x4x2_t(t470[1], t470[3]));
            } else {
                let mut t472w = 0u64;
                let t472 = core::hint::black_box(&raw mut t472w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t471 as *mut u64).add(0), t472).write_unaligned(((vgetq_lane_u32::<0>(t470[2]) as u64) << 32 | vgetq_lane_u32::<0>(t470[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t471 as *mut u64).add(1), t472).write_unaligned(((vgetq_lane_u32::<1>(t470[2]) as u64) << 32 | vgetq_lane_u32::<1>(t470[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t471 as *mut u64).add(2), t472).write_unaligned(((vgetq_lane_u32::<2>(t470[2]) as u64) << 32 | vgetq_lane_u32::<2>(t470[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t471 as *mut u64).add(3), t472).write_unaligned(((vgetq_lane_u32::<3>(t470[2]) as u64) << 32 | vgetq_lane_u32::<3>(t470[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t471 as *mut u64).add(4), t472).write_unaligned(((vgetq_lane_u32::<0>(t470[3]) as u64) << 32 | vgetq_lane_u32::<0>(t470[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t471 as *mut u64).add(5), t472).write_unaligned(((vgetq_lane_u32::<1>(t470[3]) as u64) << 32 | vgetq_lane_u32::<1>(t470[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t471 as *mut u64).add(6), t472).write_unaligned(((vgetq_lane_u32::<2>(t470[3]) as u64) << 32 | vgetq_lane_u32::<2>(t470[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t471 as *mut u64).add(7), t472).write_unaligned(((vgetq_lane_u32::<3>(t470[3]) as u64) << 32 | vgetq_lane_u32::<3>(t470[1]) as u64));
            }
            let t473 = vdupq_n_s32(((((40i64) as u32)) & 63) as i32);
            let t474: [uint32x4_t; 2] = [vcgeq_s32(t473, vdupq_n_s32(32)), vcgeq_s32(t473, vdupq_n_s32(32))];
            let t475: [uint32x4_t; 4] = [vbslq_u32(t474[0], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x56[2]), vsubq_s32(vdupq_n_s32(32), t473))), vorrq_u32(vshlq_u32(x56[0], vnegq_s32(t473)), vshlq_u32(x56[2], vsubq_s32(vdupq_n_s32(32), t473)))), vbslq_u32(t474[1], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x56[3]), vsubq_s32(vdupq_n_s32(32), t473))), vorrq_u32(vshlq_u32(x56[1], vnegq_s32(t473)), vshlq_u32(x56[3], vsubq_s32(vdupq_n_s32(32), t473)))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x56[2]), vnegq_s32(t473))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x56[3]), vnegq_s32(t473)))];
            let t476 = (arg3 as *const u8).add(((gid0.wrapping_add((8u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t476 as *mut u32).add(0), uint32x4x2_t(t475[0], t475[2]));
                vst2q_u32((t476 as *mut u32).add(8), uint32x4x2_t(t475[1], t475[3]));
            } else {
                let mut t477w = 0u64;
                let t477 = core::hint::black_box(&raw mut t477w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t476 as *mut u64).add(0), t477).write_unaligned(((vgetq_lane_u32::<0>(t475[2]) as u64) << 32 | vgetq_lane_u32::<0>(t475[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t476 as *mut u64).add(1), t477).write_unaligned(((vgetq_lane_u32::<1>(t475[2]) as u64) << 32 | vgetq_lane_u32::<1>(t475[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t476 as *mut u64).add(2), t477).write_unaligned(((vgetq_lane_u32::<2>(t475[2]) as u64) << 32 | vgetq_lane_u32::<2>(t475[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t476 as *mut u64).add(3), t477).write_unaligned(((vgetq_lane_u32::<3>(t475[2]) as u64) << 32 | vgetq_lane_u32::<3>(t475[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t476 as *mut u64).add(4), t477).write_unaligned(((vgetq_lane_u32::<0>(t475[3]) as u64) << 32 | vgetq_lane_u32::<0>(t475[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t476 as *mut u64).add(5), t477).write_unaligned(((vgetq_lane_u32::<1>(t475[3]) as u64) << 32 | vgetq_lane_u32::<1>(t475[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t476 as *mut u64).add(6), t477).write_unaligned(((vgetq_lane_u32::<2>(t475[3]) as u64) << 32 | vgetq_lane_u32::<2>(t475[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t476 as *mut u64).add(7), t477).write_unaligned(((vgetq_lane_u32::<3>(t475[3]) as u64) << 32 | vgetq_lane_u32::<3>(t475[1]) as u64));
            }
            let t478: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x56[0]), vget_low_u32(x64[0]))), vreinterpretq_u32_u64(vmull_high_u32(x56[0], x64[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x56[1]), vget_low_u32(x64[1]))), vreinterpretq_u32_u64(vmull_high_u32(x56[1], x64[1])))];
            let t479: [uint32x4_t; 4] = [vmulq_u32(x56[0], x64[0]), vmulq_u32(x56[1], x64[1]), vmlaq_u32(vmlaq_u32(t478[0], x56[0], x64[2]), x64[0], x56[2]), vmlaq_u32(vmlaq_u32(t478[1], x56[1], x64[3]), x64[1], x56[3])];
            let t480 = (arg3 as *const u8).add(((gid0.wrapping_add((9u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t480 as *mut u32).add(0), uint32x4x2_t(t479[0], t479[2]));
                vst2q_u32((t480 as *mut u32).add(8), uint32x4x2_t(t479[1], t479[3]));
            } else {
                let mut t481w = 0u64;
                let t481 = core::hint::black_box(&raw mut t481w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t480 as *mut u64).add(0), t481).write_unaligned(((vgetq_lane_u32::<0>(t479[2]) as u64) << 32 | vgetq_lane_u32::<0>(t479[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t480 as *mut u64).add(1), t481).write_unaligned(((vgetq_lane_u32::<1>(t479[2]) as u64) << 32 | vgetq_lane_u32::<1>(t479[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t480 as *mut u64).add(2), t481).write_unaligned(((vgetq_lane_u32::<2>(t479[2]) as u64) << 32 | vgetq_lane_u32::<2>(t479[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t480 as *mut u64).add(3), t481).write_unaligned(((vgetq_lane_u32::<3>(t479[2]) as u64) << 32 | vgetq_lane_u32::<3>(t479[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t480 as *mut u64).add(4), t481).write_unaligned(((vgetq_lane_u32::<0>(t479[3]) as u64) << 32 | vgetq_lane_u32::<0>(t479[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t480 as *mut u64).add(5), t481).write_unaligned(((vgetq_lane_u32::<1>(t479[3]) as u64) << 32 | vgetq_lane_u32::<1>(t479[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t480 as *mut u64).add(6), t481).write_unaligned(((vgetq_lane_u32::<2>(t479[3]) as u64) << 32 | vgetq_lane_u32::<2>(t479[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t480 as *mut u64).add(7), t481).write_unaligned(((vgetq_lane_u32::<3>(t479[3]) as u64) << 32 | vgetq_lane_u32::<3>(t479[1]) as u64));
            }
            let t482: u64 = 0u64;
            let t483 = [vdupq_n_u32((t482) as u32); 2];
            let t484: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x64[0], t483[0]), vceqq_u32(x64[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(x64[1], t483[1]), vceqq_u32(x64[3], vdupq_n_u32(0)))];
            let t485: [uint32x4_t; 2] = [vmvnq_u32(t484[0]), vmvnq_u32(t484[1])];
            let mut t486: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            let t487: [uint32x4_t; 2] = [vandq_u32(live[0], t485[0]), vandq_u32(live[1], t485[1])];
            let t488: [uint32x4_t; 2] = [vbicq_u32(live[0], t485[0]), vbicq_u32(live[1], t485[1])];
            {
                let (mut t489xl, mut t489xh, mut t489yl, mut t489yh) = ([0u32; 8], [0u32; 8], [0u32; 8], [0u32; 8]);
                vst1q_u32(t489xl.as_mut_ptr().add(0), x56[0]); vst1q_u32(t489xh.as_mut_ptr().add(0), x56[2]); vst1q_u32(t489yl.as_mut_ptr().add(0), x64[0]); vst1q_u32(t489yh.as_mut_ptr().add(0), x64[2]);
                vst1q_u32(t489xl.as_mut_ptr().add(4), x56[1]); vst1q_u32(t489xh.as_mut_ptr().add(4), x56[3]); vst1q_u32(t489yl.as_mut_ptr().add(4), x64[1]); vst1q_u32(t489yh.as_mut_ptr().add(4), x64[3]);
                for j in 0..8 { let q = (((t489xh[j] as u64) << 32 | t489xl[j] as u64) as u64).checked_div(((t489yh[j] as u64) << 32 | t489yl[j] as u64) as u64).unwrap_or(0) as u64; t489xl[j] = q as u32; t489xh[j] = (q >> 32) as u32; }
                let t490: [uint32x4_t; 4] = [vld1q_u32(t489xl.as_ptr().add(0)), vld1q_u32(t489xl.as_ptr().add(4)), vld1q_u32(t489xh.as_ptr().add(0)), vld1q_u32(t489xh.as_ptr().add(4))];
                t486 = [vbslq_u32(t487[0], t490[0], t486[0]), vbslq_u32(t487[1], t490[1], t486[1]), vbslq_u32(t487[0], t490[2], t486[2]), vbslq_u32(t487[1], t490[3], t486[3])];
            }
            {
                let t491: u64 = 7u64;
                let t492 = [vdupq_n_u32((t491) as u32); 2];
                let t493: [uint32x4_t; 4] = [t492[0], t492[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                t486 = [vbslq_u32(t488[0], t493[0], t486[0]), vbslq_u32(t488[1], t493[1], t486[1]), vbslq_u32(t488[0], t493[2], t486[2]), vbslq_u32(t488[1], t493[3], t486[3])];
            }
            let t494 = (arg3 as *const u8).add(((gid0.wrapping_add((10u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t494 as *mut u32).add(0), uint32x4x2_t(t486[0], t486[2]));
                vst2q_u32((t494 as *mut u32).add(8), uint32x4x2_t(t486[1], t486[3]));
            } else {
                let mut t495w = 0u64;
                let t495 = core::hint::black_box(&raw mut t495w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t494 as *mut u64).add(0), t495).write_unaligned(((vgetq_lane_u32::<0>(t486[2]) as u64) << 32 | vgetq_lane_u32::<0>(t486[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t494 as *mut u64).add(1), t495).write_unaligned(((vgetq_lane_u32::<1>(t486[2]) as u64) << 32 | vgetq_lane_u32::<1>(t486[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t494 as *mut u64).add(2), t495).write_unaligned(((vgetq_lane_u32::<2>(t486[2]) as u64) << 32 | vgetq_lane_u32::<2>(t486[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t494 as *mut u64).add(3), t495).write_unaligned(((vgetq_lane_u32::<3>(t486[2]) as u64) << 32 | vgetq_lane_u32::<3>(t486[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t494 as *mut u64).add(4), t495).write_unaligned(((vgetq_lane_u32::<0>(t486[3]) as u64) << 32 | vgetq_lane_u32::<0>(t486[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t494 as *mut u64).add(5), t495).write_unaligned(((vgetq_lane_u32::<1>(t486[3]) as u64) << 32 | vgetq_lane_u32::<1>(t486[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t494 as *mut u64).add(6), t495).write_unaligned(((vgetq_lane_u32::<2>(t486[3]) as u64) << 32 | vgetq_lane_u32::<2>(t486[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t494 as *mut u64).add(7), t495).write_unaligned(((vgetq_lane_u32::<3>(t486[3]) as u64) << 32 | vgetq_lane_u32::<3>(t486[1]) as u64));
            }
            let t496: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x56[2], x64[2]), vandq_u32(vceqq_u32(x56[2], x64[2]), vcltq_u32(x56[0], x64[0]))), vorrq_u32(vcltq_u32(x56[3], x64[3]), vandq_u32(vceqq_u32(x56[3], x64[3]), vcltq_u32(x56[1], x64[1])))];
            let t497: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t496[0]), vshrq_n_u32::<31>(t496[1])];
            let t498: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x64[2], x56[2]), vandq_u32(vceqq_u32(x64[2], x56[2]), vcltq_u32(x64[0], x56[0]))), vorrq_u32(vcltq_u32(x64[3], x56[3]), vandq_u32(vceqq_u32(x64[3], x56[3]), vcltq_u32(x64[1], x56[1])))];
            let t499: [uint32x4_t; 2] = [vmvnq_u32(t498[0]), vmvnq_u32(t498[1])];
            let t500: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t499[0]), vshrq_n_u32::<31>(t499[1])];
            let t501: [uint32x4_t; 4] = [vshlq_n_u32::<1>(t500[0]), vshlq_n_u32::<1>(t500[1]), vshrq_n_u32::<31>(t500[0]), vshrq_n_u32::<31>(t500[1])];
            let t502: [uint32x4_t; 4] = [vorrq_u32(t497[0], t501[0]), vorrq_u32(t497[1], t501[1]), t501[2], t501[3]];
            let t503: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x56[0], x64[0]), vceqq_u32(x56[2], x64[2])), vandq_u32(vceqq_u32(x56[1], x64[1]), vceqq_u32(x56[3], x64[3]))];
            let t504: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t503[0]), vshrq_n_u32::<31>(t503[1])];
            let t505: [uint32x4_t; 4] = [vshlq_n_u32::<2>(t504[0]), vshlq_n_u32::<2>(t504[1]), vshrq_n_u32::<30>(t504[0]), vshrq_n_u32::<30>(t504[1])];
            let t506: [uint32x4_t; 4] = [vorrq_u32(t502[0], t505[0]), vorrq_u32(t502[1], t505[1]), vorrq_u32(t502[2], t505[2]), vorrq_u32(t502[3], t505[3])];
            let t507: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x56[0], x64[0]), vceqq_u32(x56[2], x64[2])), vandq_u32(vceqq_u32(x56[1], x64[1]), vceqq_u32(x56[3], x64[3]))];
            let t508: [uint32x4_t; 2] = [vmvnq_u32(t507[0]), vmvnq_u32(t507[1])];
            let t509: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t508[0]), vshrq_n_u32::<31>(t508[1])];
            let t510: [uint32x4_t; 4] = [vshlq_n_u32::<3>(t509[0]), vshlq_n_u32::<3>(t509[1]), vshrq_n_u32::<29>(t509[0]), vshrq_n_u32::<29>(t509[1])];
            let t511: [uint32x4_t; 4] = [vorrq_u32(t506[0], t510[0]), vorrq_u32(t506[1], t510[1]), vorrq_u32(t506[2], t510[2]), vorrq_u32(t506[3], t510[3])];
            let t512: [uint32x4_t; 2] = [vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x56[2]), vreinterpretq_s32_u32(x64[2])), vandq_u32(vceqq_u32(x56[2], x64[2]), vcltq_u32(x56[0], x64[0]))), vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x56[3]), vreinterpretq_s32_u32(x64[3])), vandq_u32(vceqq_u32(x56[3], x64[3]), vcltq_u32(x56[1], x64[1])))];
            let t513: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t512[0]), vshrq_n_u32::<31>(t512[1])];
            let t514: [uint32x4_t; 4] = [vshlq_n_u32::<4>(t513[0]), vshlq_n_u32::<4>(t513[1]), vshrq_n_u32::<28>(t513[0]), vshrq_n_u32::<28>(t513[1])];
            let t515: [uint32x4_t; 4] = [vorrq_u32(t511[0], t514[0]), vorrq_u32(t511[1], t514[1]), vorrq_u32(t511[2], t514[2]), vorrq_u32(t511[3], t514[3])];
            let t516: [uint32x4_t; 2] = [vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x56[2]), vreinterpretq_s32_u32(x64[2])), vandq_u32(vceqq_u32(x56[2], x64[2]), vcltq_u32(x56[0], x64[0]))), vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x56[3]), vreinterpretq_s32_u32(x64[3])), vandq_u32(vceqq_u32(x56[3], x64[3]), vcltq_u32(x56[1], x64[1])))];
            let t517: [uint32x4_t; 2] = [vmvnq_u32(t516[0]), vmvnq_u32(t516[1])];
            let t518: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t517[0]), vshrq_n_u32::<31>(t517[1])];
            let t519: [uint32x4_t; 4] = [vshlq_n_u32::<5>(t518[0]), vshlq_n_u32::<5>(t518[1]), vshrq_n_u32::<27>(t518[0]), vshrq_n_u32::<27>(t518[1])];
            let t520: [uint32x4_t; 4] = [vorrq_u32(t515[0], t519[0]), vorrq_u32(t515[1], t519[1]), vorrq_u32(t515[2], t519[2]), vorrq_u32(t515[3], t519[3])];
            let t521: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x64[2], x56[2]), vandq_u32(vceqq_u32(x64[2], x56[2]), vcltq_u32(x64[0], x56[0]))), vorrq_u32(vcltq_u32(x64[3], x56[3]), vandq_u32(vceqq_u32(x64[3], x56[3]), vcltq_u32(x64[1], x56[1])))];
            let t522: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t521[0]), vshrq_n_u32::<31>(t521[1])];
            let t523: [uint32x4_t; 4] = [vshlq_n_u32::<6>(t522[0]), vshlq_n_u32::<6>(t522[1]), vshrq_n_u32::<26>(t522[0]), vshrq_n_u32::<26>(t522[1])];
            let t524: [uint32x4_t; 4] = [vorrq_u32(t520[0], t523[0]), vorrq_u32(t520[1], t523[1]), vorrq_u32(t520[2], t523[2]), vorrq_u32(t520[3], t523[3])];
            let t525 = (arg3 as *const u8).add(((gid0.wrapping_add((11u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t525 as *mut u32).add(0), uint32x4x2_t(t524[0], t524[2]));
                vst2q_u32((t525 as *mut u32).add(8), uint32x4x2_t(t524[1], t524[3]));
            } else {
                let mut t526w = 0u64;
                let t526 = core::hint::black_box(&raw mut t526w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t525 as *mut u64).add(0), t526).write_unaligned(((vgetq_lane_u32::<0>(t524[2]) as u64) << 32 | vgetq_lane_u32::<0>(t524[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t525 as *mut u64).add(1), t526).write_unaligned(((vgetq_lane_u32::<1>(t524[2]) as u64) << 32 | vgetq_lane_u32::<1>(t524[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t525 as *mut u64).add(2), t526).write_unaligned(((vgetq_lane_u32::<2>(t524[2]) as u64) << 32 | vgetq_lane_u32::<2>(t524[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t525 as *mut u64).add(3), t526).write_unaligned(((vgetq_lane_u32::<3>(t524[2]) as u64) << 32 | vgetq_lane_u32::<3>(t524[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t525 as *mut u64).add(4), t526).write_unaligned(((vgetq_lane_u32::<0>(t524[3]) as u64) << 32 | vgetq_lane_u32::<0>(t524[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t525 as *mut u64).add(5), t526).write_unaligned(((vgetq_lane_u32::<1>(t524[3]) as u64) << 32 | vgetq_lane_u32::<1>(t524[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t525 as *mut u64).add(6), t526).write_unaligned(((vgetq_lane_u32::<2>(t524[3]) as u64) << 32 | vgetq_lane_u32::<2>(t524[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t525 as *mut u64).add(7), t526).write_unaligned(((vgetq_lane_u32::<3>(t524[3]) as u64) << 32 | vgetq_lane_u32::<3>(t524[1]) as u64));
            }
            let t527: [uint32x4_t; 2] = [vaddq_u32(x72[0], x56[0]), vaddq_u32(x72[1], x56[1])];
            let t528: [uint32x4_t; 2] = [vcltq_u32(t527[0], x72[0]), vcltq_u32(t527[1], x72[1])];
            let t529: [uint32x4_t; 4] = [t527[0], t527[1], vsubq_u32(vaddq_u32(vdupq_n_u32(0), x56[2]), t528[0]), vsubq_u32(vaddq_u32(vdupq_n_u32(0), x56[3]), t528[1])];
            let t530 = (arg3 as *const u8).add(((gid0.wrapping_add((12u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t530 as *mut u32).add(0), uint32x4x2_t(t529[0], t529[2]));
                vst2q_u32((t530 as *mut u32).add(8), uint32x4x2_t(t529[1], t529[3]));
            } else {
                let mut t531w = 0u64;
                let t531 = core::hint::black_box(&raw mut t531w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t530 as *mut u64).add(0), t531).write_unaligned(((vgetq_lane_u32::<0>(t529[2]) as u64) << 32 | vgetq_lane_u32::<0>(t529[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t530 as *mut u64).add(1), t531).write_unaligned(((vgetq_lane_u32::<1>(t529[2]) as u64) << 32 | vgetq_lane_u32::<1>(t529[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t530 as *mut u64).add(2), t531).write_unaligned(((vgetq_lane_u32::<2>(t529[2]) as u64) << 32 | vgetq_lane_u32::<2>(t529[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t530 as *mut u64).add(3), t531).write_unaligned(((vgetq_lane_u32::<3>(t529[2]) as u64) << 32 | vgetq_lane_u32::<3>(t529[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t530 as *mut u64).add(4), t531).write_unaligned(((vgetq_lane_u32::<0>(t529[3]) as u64) << 32 | vgetq_lane_u32::<0>(t529[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t530 as *mut u64).add(5), t531).write_unaligned(((vgetq_lane_u32::<1>(t529[3]) as u64) << 32 | vgetq_lane_u32::<1>(t529[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t530 as *mut u64).add(6), t531).write_unaligned(((vgetq_lane_u32::<2>(t529[3]) as u64) << 32 | vgetq_lane_u32::<2>(t529[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t530 as *mut u64).add(7), t531).write_unaligned(((vgetq_lane_u32::<3>(t529[3]) as u64) << 32 | vgetq_lane_u32::<3>(t529[1]) as u64));
            }
            let t532: [uint32x4_t; 2] = [vreinterpretq_u32_s32(vshrq_n_s32::<31>(vreinterpretq_s32_u32(x72[0]))), vreinterpretq_u32_s32(vshrq_n_s32::<31>(vreinterpretq_s32_u32(x72[1])))];
            let t533: [uint32x4_t; 4] = [x72[0], x72[1], t532[0], t532[1]];
            let t534 = (arg3 as *const u8).add(((gid0.wrapping_add((13u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t534 as *mut u32).add(0), uint32x4x2_t(t533[0], t533[2]));
                vst2q_u32((t534 as *mut u32).add(8), uint32x4x2_t(t533[1], t533[3]));
            } else {
                let mut t535w = 0u64;
                let t535 = core::hint::black_box(&raw mut t535w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t534 as *mut u64).add(0), t535).write_unaligned(((vgetq_lane_u32::<0>(t533[2]) as u64) << 32 | vgetq_lane_u32::<0>(t533[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t534 as *mut u64).add(1), t535).write_unaligned(((vgetq_lane_u32::<1>(t533[2]) as u64) << 32 | vgetq_lane_u32::<1>(t533[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t534 as *mut u64).add(2), t535).write_unaligned(((vgetq_lane_u32::<2>(t533[2]) as u64) << 32 | vgetq_lane_u32::<2>(t533[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t534 as *mut u64).add(3), t535).write_unaligned(((vgetq_lane_u32::<3>(t533[2]) as u64) << 32 | vgetq_lane_u32::<3>(t533[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t534 as *mut u64).add(4), t535).write_unaligned(((vgetq_lane_u32::<0>(t533[3]) as u64) << 32 | vgetq_lane_u32::<0>(t533[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t534 as *mut u64).add(5), t535).write_unaligned(((vgetq_lane_u32::<1>(t533[3]) as u64) << 32 | vgetq_lane_u32::<1>(t533[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t534 as *mut u64).add(6), t535).write_unaligned(((vgetq_lane_u32::<2>(t533[3]) as u64) << 32 | vgetq_lane_u32::<2>(t533[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t534 as *mut u64).add(7), t535).write_unaligned(((vgetq_lane_u32::<3>(t533[3]) as u64) << 32 | vgetq_lane_u32::<3>(t533[1]) as u64));
            }
            let t536: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<3>(x56[0]), vshlq_n_u32::<29>(x56[2])), vorrq_u32(vshrq_n_u32::<3>(x56[1]), vshlq_n_u32::<29>(x56[3])), vshrq_n_u32::<3>(x56[2]), vshrq_n_u32::<3>(x56[3])];
            let t537: [uint32x4_t; 2] = [t536[0], t536[1]];
            let t538: [uint32x4_t; 4] = [t537[0], t537[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            let t539 = (arg3 as *const u8).add(((gid0.wrapping_add((14u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t539 as *mut u32).add(0), uint32x4x2_t(t538[0], t538[2]));
                vst2q_u32((t539 as *mut u32).add(8), uint32x4x2_t(t538[1], t538[3]));
            } else {
                let mut t540w = 0u64;
                let t540 = core::hint::black_box(&raw mut t540w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t539 as *mut u64).add(0), t540).write_unaligned(((vgetq_lane_u32::<0>(t538[2]) as u64) << 32 | vgetq_lane_u32::<0>(t538[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t539 as *mut u64).add(1), t540).write_unaligned(((vgetq_lane_u32::<1>(t538[2]) as u64) << 32 | vgetq_lane_u32::<1>(t538[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t539 as *mut u64).add(2), t540).write_unaligned(((vgetq_lane_u32::<2>(t538[2]) as u64) << 32 | vgetq_lane_u32::<2>(t538[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t539 as *mut u64).add(3), t540).write_unaligned(((vgetq_lane_u32::<3>(t538[2]) as u64) << 32 | vgetq_lane_u32::<3>(t538[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t539 as *mut u64).add(4), t540).write_unaligned(((vgetq_lane_u32::<0>(t538[3]) as u64) << 32 | vgetq_lane_u32::<0>(t538[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t539 as *mut u64).add(5), t540).write_unaligned(((vgetq_lane_u32::<1>(t538[3]) as u64) << 32 | vgetq_lane_u32::<1>(t538[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t539 as *mut u64).add(6), t540).write_unaligned(((vgetq_lane_u32::<2>(t538[3]) as u64) << 32 | vgetq_lane_u32::<2>(t538[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t539 as *mut u64).add(7), t540).write_unaligned(((vgetq_lane_u32::<3>(t538[3]) as u64) << 32 | vgetq_lane_u32::<3>(t538[1]) as u64));
            }
            let t541: u64 = 1u64;
            let t542 = [vdupq_n_u32((t541) as u32); 2];
            let t543: [uint32x4_t; 2] = [vandq_u32(x56[0], t542[0]), vandq_u32(x56[1], t542[1])];
            let t544: [uint32x4_t; 2] = [vtstq_u32(t543[0], t543[0]), vtstq_u32(t543[1], t543[1])];
            let t545: [uint32x4_t; 2] = [vandq_u32(live[0], t544[0]), vandq_u32(live[1], t544[1])];
            {
                let t546: [uint32x4_t; 4] = [veorq_u32(x56[0], x64[0]), veorq_u32(x56[1], x64[1]), veorq_u32(x56[2], x64[2]), veorq_u32(x56[3], x64[3])];
                x104 = [vbslq_u32(t545[0], t546[0], x104[0]), vbslq_u32(t545[1], t546[1], x104[1]), vbslq_u32(t545[0], t546[2], x104[2]), vbslq_u32(t545[1], t546[3], x104[3])];
            }
            let t547: [uint32x4_t; 2] = [vbicq_u32(live[0], t544[0]), vbicq_u32(live[1], t544[1])];
            {
                let t548: [uint32x4_t; 2] = [vaddq_u32(x56[0], x72[0]), vaddq_u32(x56[1], x72[1])];
                let t549: [uint32x4_t; 2] = [vcltq_u32(t548[0], x56[0]), vcltq_u32(t548[1], x56[1])];
                let t550: [uint32x4_t; 4] = [t548[0], t548[1], vsubq_u32(vaddq_u32(x56[2], vdupq_n_u32(0)), t549[0]), vsubq_u32(vaddq_u32(x56[3], vdupq_n_u32(0)), t549[1])];
                x104 = [vbslq_u32(t547[0], t550[0], x104[0]), vbslq_u32(t547[1], t550[1], x104[1]), vbslq_u32(t547[0], t550[2], x104[2]), vbslq_u32(t547[1], t550[3], x104[3])];
            }
            let t551 = (arg3 as *const u8).add(((gid0.wrapping_add((15u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t551 as *mut u32).add(0), uint32x4x2_t(x104[0], x104[2]));
                vst2q_u32((t551 as *mut u32).add(8), uint32x4x2_t(x104[1], x104[3]));
            } else {
                let mut t552w = 0u64;
                let t552 = core::hint::black_box(&raw mut t552w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t551 as *mut u64).add(0), t552).write_unaligned(((vgetq_lane_u32::<0>(x104[2]) as u64) << 32 | vgetq_lane_u32::<0>(x104[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t551 as *mut u64).add(1), t552).write_unaligned(((vgetq_lane_u32::<1>(x104[2]) as u64) << 32 | vgetq_lane_u32::<1>(x104[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t551 as *mut u64).add(2), t552).write_unaligned(((vgetq_lane_u32::<2>(x104[2]) as u64) << 32 | vgetq_lane_u32::<2>(x104[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t551 as *mut u64).add(3), t552).write_unaligned(((vgetq_lane_u32::<3>(x104[2]) as u64) << 32 | vgetq_lane_u32::<3>(x104[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t551 as *mut u64).add(4), t552).write_unaligned(((vgetq_lane_u32::<0>(x104[3]) as u64) << 32 | vgetq_lane_u32::<0>(x104[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t551 as *mut u64).add(5), t552).write_unaligned(((vgetq_lane_u32::<1>(x104[3]) as u64) << 32 | vgetq_lane_u32::<1>(x104[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t551 as *mut u64).add(6), t552).write_unaligned(((vgetq_lane_u32::<2>(x104[3]) as u64) << 32 | vgetq_lane_u32::<2>(x104[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t551 as *mut u64).add(7), t552).write_unaligned(((vgetq_lane_u32::<3>(x104[3]) as u64) << 32 | vgetq_lane_u32::<3>(x104[1]) as u64));
            }
            let t553 = b1_l_p(l, lid0, live, x56);
            let t554 = b2_l_p(l, lid0, live, x64);
            let t555: [uint32x4_t; 4] = [vshlq_n_u32::<8>(t554[0]), vshlq_n_u32::<8>(t554[1]), vshrq_n_u32::<24>(t554[0]), vshrq_n_u32::<24>(t554[1])];
            let t556: [uint32x4_t; 2] = [vaddq_u32(t553[0], t555[0]), vaddq_u32(t553[1], t555[1])];
            let t557: [uint32x4_t; 2] = [vcltq_u32(t556[0], t553[0]), vcltq_u32(t556[1], t553[1])];
            let t558: [uint32x4_t; 4] = [t556[0], t556[1], vsubq_u32(vaddq_u32(vdupq_n_u32(0), t555[2]), t557[0]), vsubq_u32(vaddq_u32(vdupq_n_u32(0), t555[3]), t557[1])];
            let t559 = (arg3 as *const u8).add(((gid0.wrapping_add((16u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t559 as *mut u32).add(0), uint32x4x2_t(t558[0], t558[2]));
                vst2q_u32((t559 as *mut u32).add(8), uint32x4x2_t(t558[1], t558[3]));
            } else {
                let mut t560w = 0u64;
                let t560 = core::hint::black_box(&raw mut t560w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t559 as *mut u64).add(0), t560).write_unaligned(((vgetq_lane_u32::<0>(t558[2]) as u64) << 32 | vgetq_lane_u32::<0>(t558[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t559 as *mut u64).add(1), t560).write_unaligned(((vgetq_lane_u32::<1>(t558[2]) as u64) << 32 | vgetq_lane_u32::<1>(t558[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t559 as *mut u64).add(2), t560).write_unaligned(((vgetq_lane_u32::<2>(t558[2]) as u64) << 32 | vgetq_lane_u32::<2>(t558[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t559 as *mut u64).add(3), t560).write_unaligned(((vgetq_lane_u32::<3>(t558[2]) as u64) << 32 | vgetq_lane_u32::<3>(t558[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t559 as *mut u64).add(4), t560).write_unaligned(((vgetq_lane_u32::<0>(t558[3]) as u64) << 32 | vgetq_lane_u32::<0>(t558[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t559 as *mut u64).add(5), t560).write_unaligned(((vgetq_lane_u32::<1>(t558[3]) as u64) << 32 | vgetq_lane_u32::<1>(t558[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t559 as *mut u64).add(6), t560).write_unaligned(((vgetq_lane_u32::<2>(t558[3]) as u64) << 32 | vgetq_lane_u32::<2>(t558[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t559 as *mut u64).add(7), t560).write_unaligned(((vgetq_lane_u32::<3>(t558[3]) as u64) << 32 | vgetq_lane_u32::<3>(t558[1]) as u64));
            }
            let t561 = [vdupq_n_u32(x88); 2];
            let mut t562x = [0u32; 8];
            let mut t562y = [0u32; 8];
            for r in 0..2 { vst1q_u32(t562x.as_mut_ptr().add(4 * r), x72[r]); vst1q_u32(t562y.as_mut_ptr().add(4 * r), t561[r]); }
            for j in 0..8 { t562x[j] = (t562x[j] as u32).checked_rem(t562y[j] as u32).unwrap_or(0) as u32; }
            let t563: [uint32x4_t; 2] = [vld1q_u32(t562x.as_ptr().add(0)), vld1q_u32(t562x.as_ptr().add(4))];
            let t564 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t563[0]), vandq_u32(live[1], t563[1]))));
            let t565: [uint32x4_t; 2] = [vbslq_u32(live[0], t563[0], t564), vbslq_u32(live[1], t563[1], t564)];
            let mut t566: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t565[0]) as usize * 8) as *const u64).read_unaligned(); t566[0] = vsetq_lane_u32::<0>(v as u32, t566[0]); t566[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t566[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t565[0]) as usize * 8) as *const u64).read_unaligned(); t566[0] = vsetq_lane_u32::<1>(v as u32, t566[0]); t566[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t566[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t565[0]) as usize * 8) as *const u64).read_unaligned(); t566[0] = vsetq_lane_u32::<2>(v as u32, t566[0]); t566[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t566[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t565[0]) as usize * 8) as *const u64).read_unaligned(); t566[0] = vsetq_lane_u32::<3>(v as u32, t566[0]); t566[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t566[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t565[1]) as usize * 8) as *const u64).read_unaligned(); t566[1] = vsetq_lane_u32::<0>(v as u32, t566[1]); t566[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t566[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t565[1]) as usize * 8) as *const u64).read_unaligned(); t566[1] = vsetq_lane_u32::<1>(v as u32, t566[1]); t566[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t566[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t565[1]) as usize * 8) as *const u64).read_unaligned(); t566[1] = vsetq_lane_u32::<2>(v as u32, t566[1]); t566[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t566[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t565[1]) as usize * 8) as *const u64).read_unaligned(); t566[1] = vsetq_lane_u32::<3>(v as u32, t566[1]); t566[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t566[3]); }
            let t567 = (arg3 as *const u8).add(((gid0.wrapping_add((17u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t567 as *mut u32).add(0), uint32x4x2_t(t566[0], t566[2]));
                vst2q_u32((t567 as *mut u32).add(8), uint32x4x2_t(t566[1], t566[3]));
            } else {
                let mut t568w = 0u64;
                let t568 = core::hint::black_box(&raw mut t568w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t567 as *mut u64).add(0), t568).write_unaligned(((vgetq_lane_u32::<0>(t566[2]) as u64) << 32 | vgetq_lane_u32::<0>(t566[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t567 as *mut u64).add(1), t568).write_unaligned(((vgetq_lane_u32::<1>(t566[2]) as u64) << 32 | vgetq_lane_u32::<1>(t566[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t567 as *mut u64).add(2), t568).write_unaligned(((vgetq_lane_u32::<2>(t566[2]) as u64) << 32 | vgetq_lane_u32::<2>(t566[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t567 as *mut u64).add(3), t568).write_unaligned(((vgetq_lane_u32::<3>(t566[2]) as u64) << 32 | vgetq_lane_u32::<3>(t566[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t567 as *mut u64).add(4), t568).write_unaligned(((vgetq_lane_u32::<0>(t566[3]) as u64) << 32 | vgetq_lane_u32::<0>(t566[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t567 as *mut u64).add(5), t568).write_unaligned(((vgetq_lane_u32::<1>(t566[3]) as u64) << 32 | vgetq_lane_u32::<1>(t566[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t567 as *mut u64).add(6), t568).write_unaligned(((vgetq_lane_u32::<2>(t566[3]) as u64) << 32 | vgetq_lane_u32::<2>(t566[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t567 as *mut u64).add(7), t568).write_unaligned(((vgetq_lane_u32::<3>(t566[3]) as u64) << 32 | vgetq_lane_u32::<3>(t566[1]) as u64));
            }
            let t569 = b3_ll_p(l, lid0, live, x56, x72);
            let t570 = (arg3 as *const u8).add(((gid0.wrapping_add((18u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t570 as *mut u32).add(0), uint32x4x2_t(t569[0], t569[2]));
                vst2q_u32((t570 as *mut u32).add(8), uint32x4x2_t(t569[1], t569[3]));
            } else {
                let mut t571w = 0u64;
                let t571 = core::hint::black_box(&raw mut t571w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t570 as *mut u64).add(0), t571).write_unaligned(((vgetq_lane_u32::<0>(t569[2]) as u64) << 32 | vgetq_lane_u32::<0>(t569[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t570 as *mut u64).add(1), t571).write_unaligned(((vgetq_lane_u32::<1>(t569[2]) as u64) << 32 | vgetq_lane_u32::<1>(t569[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t570 as *mut u64).add(2), t571).write_unaligned(((vgetq_lane_u32::<2>(t569[2]) as u64) << 32 | vgetq_lane_u32::<2>(t569[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t570 as *mut u64).add(3), t571).write_unaligned(((vgetq_lane_u32::<3>(t569[2]) as u64) << 32 | vgetq_lane_u32::<3>(t569[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t570 as *mut u64).add(4), t571).write_unaligned(((vgetq_lane_u32::<0>(t569[3]) as u64) << 32 | vgetq_lane_u32::<0>(t569[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t570 as *mut u64).add(5), t571).write_unaligned(((vgetq_lane_u32::<1>(t569[3]) as u64) << 32 | vgetq_lane_u32::<1>(t569[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t570 as *mut u64).add(6), t571).write_unaligned(((vgetq_lane_u32::<2>(t569[3]) as u64) << 32 | vgetq_lane_u32::<2>(t569[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t570 as *mut u64).add(7), t571).write_unaligned(((vgetq_lane_u32::<3>(t569[3]) as u64) << 32 | vgetq_lane_u32::<3>(t569[1]) as u64));
            }
            x112 = x56;
            x120 = 0u32;
            let mut t572 = live;
            loop {
                // Lanes part: each its own rounds.
                let t574 = [vdupq_n_u32(7u32); 2];
                let t575: [uint32x4_t; 2] = [vandq_u32(x72[0], t574[0]), vandq_u32(x72[1], t574[1])];
                let t576 = [vdupq_n_u32(x120); 2];
                let t577: [uint32x4_t; 2] = [vcltq_u32(t576[0], t575[0]), vcltq_u32(t576[1], t575[1])];
                t572 = [vandq_u32(t572[0], t577[0]), vandq_u32(t572[1], t577[1])];
                if !(any(&t572)) { break; }
                let t579: u64 = 3u64;
                let t580 = [vdupq_n_u32((t579) as u32); 2];
                let t581: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x112[0]), vget_low_u32(t580[0]))), vreinterpretq_u32_u64(vmull_high_u32(x112[0], t580[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x112[1]), vget_low_u32(t580[1]))), vreinterpretq_u32_u64(vmull_high_u32(x112[1], t580[1])))];
                let t582: [uint32x4_t; 4] = [vmulq_u32(x112[0], t580[0]), vmulq_u32(x112[1], t580[1]), vmlaq_u32(t581[0], t580[0], x112[2]), vmlaq_u32(t581[1], t580[1], x112[3])];
                let t583: [uint32x4_t; 2] = [vaddq_u32(t582[0], x64[0]), vaddq_u32(t582[1], x64[1])];
                let t584: [uint32x4_t; 2] = [vcltq_u32(t583[0], t582[0]), vcltq_u32(t583[1], t582[1])];
                let t585: [uint32x4_t; 4] = [t583[0], t583[1], vsubq_u32(vaddq_u32(t582[2], x64[2]), t584[0]), vsubq_u32(vaddq_u32(t582[3], x64[3]), t584[1])];
                x112 = [vbslq_u32(t572[0], t585[0], x112[0]), vbslq_u32(t572[1], t585[1], x112[1]), vbslq_u32(t572[0], t585[2], x112[2]), vbslq_u32(t572[1], t585[3], x112[3])];
                let t586: [uint32x4_t; 2] = [vandq_u32(t572[0], live[0]), vandq_u32(t572[1], live[1])];
                let t587 = x120;
                x120 = (x120).wrapping_add(1u32);
            }
            let t588 = (arg3 as *const u8).add(((gid0.wrapping_add((19u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t588 as *mut u32).add(0), uint32x4x2_t(x112[0], x112[2]));
                vst2q_u32((t588 as *mut u32).add(8), uint32x4x2_t(x112[1], x112[3]));
            } else {
                let mut t589w = 0u64;
                let t589 = core::hint::black_box(&raw mut t589w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t588 as *mut u64).add(0), t589).write_unaligned(((vgetq_lane_u32::<0>(x112[2]) as u64) << 32 | vgetq_lane_u32::<0>(x112[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t588 as *mut u64).add(1), t589).write_unaligned(((vgetq_lane_u32::<1>(x112[2]) as u64) << 32 | vgetq_lane_u32::<1>(x112[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t588 as *mut u64).add(2), t589).write_unaligned(((vgetq_lane_u32::<2>(x112[2]) as u64) << 32 | vgetq_lane_u32::<2>(x112[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t588 as *mut u64).add(3), t589).write_unaligned(((vgetq_lane_u32::<3>(x112[2]) as u64) << 32 | vgetq_lane_u32::<3>(x112[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t588 as *mut u64).add(4), t589).write_unaligned(((vgetq_lane_u32::<0>(x112[3]) as u64) << 32 | vgetq_lane_u32::<0>(x112[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t588 as *mut u64).add(5), t589).write_unaligned(((vgetq_lane_u32::<1>(x112[3]) as u64) << 32 | vgetq_lane_u32::<1>(x112[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t588 as *mut u64).add(6), t589).write_unaligned(((vgetq_lane_u32::<2>(x112[3]) as u64) << 32 | vgetq_lane_u32::<2>(x112[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t588 as *mut u64).add(7), t589).write_unaligned(((vgetq_lane_u32::<3>(x112[3]) as u64) << 32 | vgetq_lane_u32::<3>(x112[1]) as u64));
            }
            let t590: u64 = a1;
            let t591 = [vdupq_n_u32((t590) as u32); 2];
            let t592 = [vdupq_n_u32(((t590) >> 32) as u32); 2];
            let t593: [uint32x4_t; 2] = [vaddq_u32(x56[0], t591[0]), vaddq_u32(x56[1], t591[1])];
            let t594: [uint32x4_t; 2] = [vcltq_u32(t593[0], x56[0]), vcltq_u32(t593[1], x56[1])];
            let t595: [uint32x4_t; 4] = [t593[0], t593[1], vsubq_u32(vaddq_u32(x56[2], t592[0]), t594[0]), vsubq_u32(vaddq_u32(x56[3], t592[1]), t594[1])];
            let t596: u64 = (a1).wrapping_shl((3u64) as u32);
            let t597 = [vdupq_n_u32((t596) as u32); 2];
            let t598 = [vdupq_n_u32(((t596) >> 32) as u32); 2];
            let t599: [uint32x4_t; 2] = [vaddq_u32(t595[0], t597[0]), vaddq_u32(t595[1], t597[1])];
            let t600: [uint32x4_t; 2] = [vcltq_u32(t599[0], t595[0]), vcltq_u32(t599[1], t595[1])];
            let t601: [uint32x4_t; 4] = [t599[0], t599[1], vsubq_u32(vaddq_u32(t595[2], t598[0]), t600[0]), vsubq_u32(vaddq_u32(t595[3], t598[1]), t600[1])];
            let t602 = (arg3 as *const u8).add(((gid0.wrapping_add((20u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t602 as *mut u32).add(0), uint32x4x2_t(t601[0], t601[2]));
                vst2q_u32((t602 as *mut u32).add(8), uint32x4x2_t(t601[1], t601[3]));
            } else {
                let mut t603w = 0u64;
                let t603 = core::hint::black_box(&raw mut t603w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t602 as *mut u64).add(0), t603).write_unaligned(((vgetq_lane_u32::<0>(t601[2]) as u64) << 32 | vgetq_lane_u32::<0>(t601[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t602 as *mut u64).add(1), t603).write_unaligned(((vgetq_lane_u32::<1>(t601[2]) as u64) << 32 | vgetq_lane_u32::<1>(t601[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t602 as *mut u64).add(2), t603).write_unaligned(((vgetq_lane_u32::<2>(t601[2]) as u64) << 32 | vgetq_lane_u32::<2>(t601[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t602 as *mut u64).add(3), t603).write_unaligned(((vgetq_lane_u32::<3>(t601[2]) as u64) << 32 | vgetq_lane_u32::<3>(t601[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t602 as *mut u64).add(4), t603).write_unaligned(((vgetq_lane_u32::<0>(t601[3]) as u64) << 32 | vgetq_lane_u32::<0>(t601[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t602 as *mut u64).add(5), t603).write_unaligned(((vgetq_lane_u32::<1>(t601[3]) as u64) << 32 | vgetq_lane_u32::<1>(t601[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t602 as *mut u64).add(6), t603).write_unaligned(((vgetq_lane_u32::<2>(t601[3]) as u64) << 32 | vgetq_lane_u32::<2>(t601[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t602 as *mut u64).add(7), t603).write_unaligned(((vgetq_lane_u32::<3>(t601[3]) as u64) << 32 | vgetq_lane_u32::<3>(t601[1]) as u64));
            }
            let t604: u64 = 4294967295u64;
            let t605 = [vdupq_n_u32((t604) as u32); 2];
            let t606: [uint32x4_t; 2] = [vandq_u32(x56[0], t605[0]), vandq_u32(x56[1], t605[1])];
            let t607: u64 = 4294967295u64;
            let t608 = [vdupq_n_u32((t607) as u32); 2];
            let t609: [uint32x4_t; 2] = [vandq_u32(x64[0], t608[0]), vandq_u32(x64[1], t608[1])];
            let t610: [uint32x4_t; 2] = [vceqq_u32(t606[0], t609[0]), vceqq_u32(t606[1], t609[1])];
            let mut t611: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t612: [uint32x4_t; 2] = [vandq_u32(live[0], t610[0]), vandq_u32(live[1], t610[1])];
            let t613: [uint32x4_t; 2] = [vbicq_u32(live[0], t610[0]), vbicq_u32(live[1], t610[1])];
            {
                let t614 = [vdupq_n_u32(1u32); 2];
                t611 = [vbslq_u32(t612[0], t614[0], t611[0]), vbslq_u32(t612[1], t614[1], t611[1])];
            }
            {
                let t615 = [vdupq_n_u32(2u32); 2];
                t611 = [vbslq_u32(t613[0], t615[0], t611[0]), vbslq_u32(t613[1], t615[1], t611[1])];
            }
            let t616: [uint32x4_t; 4] = [t611[0], t611[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            let t617 = (arg3 as *const u8).add(((gid0.wrapping_add((21u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t617 as *mut u32).add(0), uint32x4x2_t(t616[0], t616[2]));
                vst2q_u32((t617 as *mut u32).add(8), uint32x4x2_t(t616[1], t616[3]));
            } else {
                let mut t618w = 0u64;
                let t618 = core::hint::black_box(&raw mut t618w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t617 as *mut u64).add(0), t618).write_unaligned(((vgetq_lane_u32::<0>(t616[2]) as u64) << 32 | vgetq_lane_u32::<0>(t616[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t617 as *mut u64).add(1), t618).write_unaligned(((vgetq_lane_u32::<1>(t616[2]) as u64) << 32 | vgetq_lane_u32::<1>(t616[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t617 as *mut u64).add(2), t618).write_unaligned(((vgetq_lane_u32::<2>(t616[2]) as u64) << 32 | vgetq_lane_u32::<2>(t616[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t617 as *mut u64).add(3), t618).write_unaligned(((vgetq_lane_u32::<3>(t616[2]) as u64) << 32 | vgetq_lane_u32::<3>(t616[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t617 as *mut u64).add(4), t618).write_unaligned(((vgetq_lane_u32::<0>(t616[3]) as u64) << 32 | vgetq_lane_u32::<0>(t616[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t617 as *mut u64).add(5), t618).write_unaligned(((vgetq_lane_u32::<1>(t616[3]) as u64) << 32 | vgetq_lane_u32::<1>(t616[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t617 as *mut u64).add(6), t618).write_unaligned(((vgetq_lane_u32::<2>(t616[3]) as u64) << 32 | vgetq_lane_u32::<2>(t616[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t617 as *mut u64).add(7), t618).write_unaligned(((vgetq_lane_u32::<3>(t616[3]) as u64) << 32 | vgetq_lane_u32::<3>(t616[1]) as u64));
            }
            let t619: [uint32x4_t; 2] = [vtstq_u32(vorrq_u32(x56[0], x56[2]), vorrq_u32(x56[0], x56[2])), vtstq_u32(vorrq_u32(x56[1], x56[3]), vorrq_u32(x56[1], x56[3]))];
            let mut t620: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t621: [uint32x4_t; 2] = [vandq_u32(live[0], t619[0]), vandq_u32(live[1], t619[1])];
            let t622: [uint32x4_t; 2] = [vbicq_u32(live[0], t619[0]), vbicq_u32(live[1], t619[1])];
            {
                let t623 = [vdupq_n_u32(5u32); 2];
                t620 = [vbslq_u32(t621[0], t623[0], t620[0]), vbslq_u32(t621[1], t623[1], t620[1])];
            }
            {
                let t624 = [vdupq_n_u32(6u32); 2];
                t620 = [vbslq_u32(t622[0], t624[0], t620[0]), vbslq_u32(t622[1], t624[1], t620[1])];
            }
            let t625: [uint32x4_t; 4] = [t620[0], t620[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            let t626 = (arg3 as *const u8).add(((gid0.wrapping_add((22u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t626 as *mut u32).add(0), uint32x4x2_t(t625[0], t625[2]));
                vst2q_u32((t626 as *mut u32).add(8), uint32x4x2_t(t625[1], t625[3]));
            } else {
                let mut t627w = 0u64;
                let t627 = core::hint::black_box(&raw mut t627w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t626 as *mut u64).add(0), t627).write_unaligned(((vgetq_lane_u32::<0>(t625[2]) as u64) << 32 | vgetq_lane_u32::<0>(t625[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t626 as *mut u64).add(1), t627).write_unaligned(((vgetq_lane_u32::<1>(t625[2]) as u64) << 32 | vgetq_lane_u32::<1>(t625[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t626 as *mut u64).add(2), t627).write_unaligned(((vgetq_lane_u32::<2>(t625[2]) as u64) << 32 | vgetq_lane_u32::<2>(t625[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t626 as *mut u64).add(3), t627).write_unaligned(((vgetq_lane_u32::<3>(t625[2]) as u64) << 32 | vgetq_lane_u32::<3>(t625[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t626 as *mut u64).add(4), t627).write_unaligned(((vgetq_lane_u32::<0>(t625[3]) as u64) << 32 | vgetq_lane_u32::<0>(t625[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t626 as *mut u64).add(5), t627).write_unaligned(((vgetq_lane_u32::<1>(t625[3]) as u64) << 32 | vgetq_lane_u32::<1>(t625[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t626 as *mut u64).add(6), t627).write_unaligned(((vgetq_lane_u32::<2>(t625[3]) as u64) << 32 | vgetq_lane_u32::<2>(t625[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t626 as *mut u64).add(7), t627).write_unaligned(((vgetq_lane_u32::<3>(t625[3]) as u64) << 32 | vgetq_lane_u32::<3>(t625[1]) as u64));
            }
            let t628 = b4_ll_p(l, lid0, live, x56, x64);
            let t629 = (arg3 as *const u8).add(((gid0.wrapping_add((23u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t629 as *mut u32).add(0), uint32x4x2_t(t628[0], t628[2]));
                vst2q_u32((t629 as *mut u32).add(8), uint32x4x2_t(t628[1], t628[3]));
            } else {
                let mut t630w = 0u64;
                let t630 = core::hint::black_box(&raw mut t630w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t629 as *mut u64).add(0), t630).write_unaligned(((vgetq_lane_u32::<0>(t628[2]) as u64) << 32 | vgetq_lane_u32::<0>(t628[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t629 as *mut u64).add(1), t630).write_unaligned(((vgetq_lane_u32::<1>(t628[2]) as u64) << 32 | vgetq_lane_u32::<1>(t628[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t629 as *mut u64).add(2), t630).write_unaligned(((vgetq_lane_u32::<2>(t628[2]) as u64) << 32 | vgetq_lane_u32::<2>(t628[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t629 as *mut u64).add(3), t630).write_unaligned(((vgetq_lane_u32::<3>(t628[2]) as u64) << 32 | vgetq_lane_u32::<3>(t628[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t629 as *mut u64).add(4), t630).write_unaligned(((vgetq_lane_u32::<0>(t628[3]) as u64) << 32 | vgetq_lane_u32::<0>(t628[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t629 as *mut u64).add(5), t630).write_unaligned(((vgetq_lane_u32::<1>(t628[3]) as u64) << 32 | vgetq_lane_u32::<1>(t628[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t629 as *mut u64).add(6), t630).write_unaligned(((vgetq_lane_u32::<2>(t628[3]) as u64) << 32 | vgetq_lane_u32::<2>(t628[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t629 as *mut u64).add(7), t630).write_unaligned(((vgetq_lane_u32::<3>(t628[3]) as u64) << 32 | vgetq_lane_u32::<3>(t628[1]) as u64));
            }
            let t631 = b5_r0ns_p(l, lid0, live, arg0, gid0.wrapping_mul(x96), x96, 8u32);
            let t632 = (arg3 as *const u8).add(((gid0.wrapping_add((24u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t632 as *mut u32).add(0), uint32x4x2_t(t631[0], t631[2]));
                vst2q_u32((t632 as *mut u32).add(8), uint32x4x2_t(t631[1], t631[3]));
            } else {
                let mut t633w = 0u64;
                let t633 = core::hint::black_box(&raw mut t633w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t632 as *mut u64).add(0), t633).write_unaligned(((vgetq_lane_u32::<0>(t631[2]) as u64) << 32 | vgetq_lane_u32::<0>(t631[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t632 as *mut u64).add(1), t633).write_unaligned(((vgetq_lane_u32::<1>(t631[2]) as u64) << 32 | vgetq_lane_u32::<1>(t631[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t632 as *mut u64).add(2), t633).write_unaligned(((vgetq_lane_u32::<2>(t631[2]) as u64) << 32 | vgetq_lane_u32::<2>(t631[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t632 as *mut u64).add(3), t633).write_unaligned(((vgetq_lane_u32::<3>(t631[2]) as u64) << 32 | vgetq_lane_u32::<3>(t631[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t632 as *mut u64).add(4), t633).write_unaligned(((vgetq_lane_u32::<0>(t631[3]) as u64) << 32 | vgetq_lane_u32::<0>(t631[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t632 as *mut u64).add(5), t633).write_unaligned(((vgetq_lane_u32::<1>(t631[3]) as u64) << 32 | vgetq_lane_u32::<1>(t631[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t632 as *mut u64).add(6), t633).write_unaligned(((vgetq_lane_u32::<2>(t631[3]) as u64) << 32 | vgetq_lane_u32::<2>(t631[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t632 as *mut u64).add(7), t633).write_unaligned(((vgetq_lane_u32::<3>(t631[3]) as u64) << 32 | vgetq_lane_u32::<3>(t631[1]) as u64));
            }
            let t634 = if (gid0.wrapping_add(3u32) & 255u32) <= 248u32 { b6_u8_p(l, lid0, live, gid0.wrapping_add(3u32)) } else { b6_i_p(l, lid0, live, gid0.wrapping_add(3u32)) };
            let t635 = (arg3 as *const u8).add(((gid0.wrapping_add((25u32).wrapping_mul(x88))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t635 as *mut u32).add(0), uint32x4x2_t(t634[0], t634[2]));
                vst2q_u32((t635 as *mut u32).add(8), uint32x4x2_t(t634[1], t634[3]));
            } else {
                let mut t636w = 0u64;
                let t636 = core::hint::black_box(&raw mut t636w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t635 as *mut u64).add(0), t636).write_unaligned(((vgetq_lane_u32::<0>(t634[2]) as u64) << 32 | vgetq_lane_u32::<0>(t634[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t635 as *mut u64).add(1), t636).write_unaligned(((vgetq_lane_u32::<1>(t634[2]) as u64) << 32 | vgetq_lane_u32::<1>(t634[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t635 as *mut u64).add(2), t636).write_unaligned(((vgetq_lane_u32::<2>(t634[2]) as u64) << 32 | vgetq_lane_u32::<2>(t634[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t635 as *mut u64).add(3), t636).write_unaligned(((vgetq_lane_u32::<3>(t634[2]) as u64) << 32 | vgetq_lane_u32::<3>(t634[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t635 as *mut u64).add(4), t636).write_unaligned(((vgetq_lane_u32::<0>(t634[3]) as u64) << 32 | vgetq_lane_u32::<0>(t634[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t635 as *mut u64).add(5), t636).write_unaligned(((vgetq_lane_u32::<1>(t634[3]) as u64) << 32 | vgetq_lane_u32::<1>(t634[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t635 as *mut u64).add(6), t636).write_unaligned(((vgetq_lane_u32::<2>(t634[3]) as u64) << 32 | vgetq_lane_u32::<2>(t634[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t635 as *mut u64).add(7), t636).write_unaligned(((vgetq_lane_u32::<3>(t634[3]) as u64) << 32 | vgetq_lane_u32::<3>(t634[1]) as u64));
            }
        }
    }
}

#[inline(always)]
unsafe fn b1_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t637: [uint32x4_t; 2] = [vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[0])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[2]))))), vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[1])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[3])))))];
    res = t637;
    return res;
}

#[inline(always)]
unsafe fn b1_l_f(l: &Launch, lid0: u32, x0: [uint32x4_t; 4]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t638: [uint32x4_t; 2] = [vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[0])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[2]))))), vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[1])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[3])))))];
    res = t638;
    return res;
}

#[inline(always)]
unsafe fn b2_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t639: [uint32x4_t; 2] = [vbslq_u32(vceqzq_u32(x0[2]), vaddq_u32(vclzq_u32(x0[0]), vdupq_n_u32(32)), vclzq_u32(x0[2])), vbslq_u32(vceqzq_u32(x0[3]), vaddq_u32(vclzq_u32(x0[1]), vdupq_n_u32(32)), vclzq_u32(x0[3]))];
    res = t639;
    return res;
}

#[inline(always)]
unsafe fn b2_l_f(l: &Launch, lid0: u32, x0: [uint32x4_t; 4]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t640: [uint32x4_t; 2] = [vbslq_u32(vceqzq_u32(x0[2]), vaddq_u32(vclzq_u32(x0[0]), vdupq_n_u32(32)), vclzq_u32(x0[2])), vbslq_u32(vceqzq_u32(x0[3]), vaddq_u32(vclzq_u32(x0[1]), vdupq_n_u32(32)), vclzq_u32(x0[3]))];
    res = t640;
    return res;
}

#[inline(always)]
unsafe fn b3_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], x8: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x16: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t641: u64 = 1u64;
    let t642 = [vdupq_n_u32((t641) as u32); 2];
    let t643: [uint32x4_t; 2] = [vandq_u32(x0[0], t642[0]), vandq_u32(x0[1], t642[1])];
    let t644: u64 = 0u64;
    let t645 = [vdupq_n_u32((t644) as u32); 2];
    let t646: [uint32x4_t; 2] = [vceqq_u32(t643[0], t645[0]), vceqq_u32(t643[1], t645[1])];
    let t647: [uint32x4_t; 2] = [vandq_u32(live[0], t646[0]), vandq_u32(live[1], t646[1])];
    {
        let t648: [uint32x4_t; 4] = [x8[0], x8[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = [vbslq_u32(t647[0], t648[0], res[0]), vbslq_u32(t647[1], t648[1], res[1]), vbslq_u32(t647[0], t648[2], res[2]), vbslq_u32(t647[1], t648[3], res[3])];
        live = [vbicq_u32(live[0], t647[0]), vbicq_u32(live[1], t647[1])];
    }
    if any(&live) {
        let t649: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x0[0]), vshlq_n_u32::<30>(x0[2])), vorrq_u32(vshrq_n_u32::<2>(x0[1]), vshlq_n_u32::<30>(x0[3])), vshrq_n_u32::<2>(x0[2]), vshrq_n_u32::<2>(x0[3])];
        let t650: [uint32x4_t; 2] = [t649[0], t649[1]];
        x16 = t650;
        let t651 = [vdupq_n_u32(1u32); 2];
        let t652: [uint32x4_t; 2] = [vsubq_u32(x16[0], t651[0]), vsubq_u32(x16[1], t651[1])];
        let t653: [uint32x4_t; 2] = [vcgeq_u32(t652[0], x8[0]), vcgeq_u32(t652[1], x8[1])];
        let t654: [uint32x4_t; 2] = [vandq_u32(live[0], t653[0]), vandq_u32(live[1], t653[1])];
        {
            let t655: u64 = 4294967295u64;
            let t656 = [vdupq_n_u32((t655) as u32); 2];
            let t657: [uint32x4_t; 4] = [t656[0], t656[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t654[0], t657[0], res[0]), vbslq_u32(t654[1], t657[1], res[1]), vbslq_u32(t654[0], t657[2], res[2]), vbslq_u32(t654[1], t657[3], res[3])];
            live = [vbicq_u32(live[0], t654[0]), vbicq_u32(live[1], t654[1])];
        }
        if any(&live) {
            let t658: u64 = 2u64;
            let t659 = [vdupq_n_u32((t658) as u32); 2];
            let t660: [uint32x4_t; 2] = [vandq_u32(x0[0], t659[0]), vandq_u32(x0[1], t659[1])];
            let t661: u64 = 0u64;
            let t662 = [vdupq_n_u32((t661) as u32); 2];
            let t663: [uint32x4_t; 2] = [vceqq_u32(t660[0], t662[0]), vceqq_u32(t660[1], t662[1])];
            let t664: [uint32x4_t; 2] = [vmvnq_u32(t663[0]), vmvnq_u32(t663[1])];
            let t665: [uint32x4_t; 2] = [vandq_u32(live[0], t664[0]), vandq_u32(live[1], t664[1])];
            {
                let t666: [uint32x4_t; 2] = [vsubq_u32(x8[0], x16[0]), vsubq_u32(x8[1], x16[1])];
                let t667: [uint32x4_t; 4] = [t666[0], t666[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t665[0], t667[0], res[0]), vbslq_u32(t665[1], t667[1], res[1]), vbslq_u32(t665[0], t667[2], res[2]), vbslq_u32(t665[1], t667[3], res[3])];
                live = [vbicq_u32(live[0], t665[0]), vbicq_u32(live[1], t665[1])];
            }
            if any(&live) {
                let t668: [uint32x4_t; 2] = [vsubq_u32(x8[0], x16[0]), vsubq_u32(x8[1], x16[1])];
                let t669: u64 = (1u64).wrapping_shl((32u64) as u32);
                let t670 = [vdupq_n_u32((t669) as u32); 2];
                let t671 = [vdupq_n_u32(((t669) >> 32) as u32); 2];
                let t672: [uint32x4_t; 4] = [vorrq_u32(t670[0], t668[0]), vorrq_u32(t670[1], t668[1]), t671[0], t671[1]];
                res = [vbslq_u32(live[0], t672[0], res[0]), vbslq_u32(live[1], t672[1], res[1]), vbslq_u32(live[0], t672[2], res[2]), vbslq_u32(live[1], t672[3], res[3])];
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
    let t673: u64 = 1u64;
    let t674 = [vdupq_n_u32((t673) as u32); 2];
    let t675: [uint32x4_t; 2] = [vandq_u32(x0[0], t674[0]), vandq_u32(x0[1], t674[1])];
    let t676: u64 = 0u64;
    let t677 = [vdupq_n_u32((t676) as u32); 2];
    let t678: [uint32x4_t; 2] = [vceqq_u32(t675[0], t677[0]), vceqq_u32(t675[1], t677[1])];
    {
        let t679: [uint32x4_t; 4] = [x8[0], x8[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = [vbslq_u32(t678[0], t679[0], res[0]), vbslq_u32(t678[1], t679[1], res[1]), vbslq_u32(t678[0], t679[2], res[2]), vbslq_u32(t678[1], t679[3], res[3])];
        live = [vbicq_u32(live[0], t678[0]), vbicq_u32(live[1], t678[1])];
    }
    if any(&live) {
        let t680: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x0[0]), vshlq_n_u32::<30>(x0[2])), vorrq_u32(vshrq_n_u32::<2>(x0[1]), vshlq_n_u32::<30>(x0[3])), vshrq_n_u32::<2>(x0[2]), vshrq_n_u32::<2>(x0[3])];
        let t681: [uint32x4_t; 2] = [t680[0], t680[1]];
        x16 = t681;
        let t682 = [vdupq_n_u32(1u32); 2];
        let t683: [uint32x4_t; 2] = [vsubq_u32(x16[0], t682[0]), vsubq_u32(x16[1], t682[1])];
        let t684: [uint32x4_t; 2] = [vcgeq_u32(t683[0], x8[0]), vcgeq_u32(t683[1], x8[1])];
        let t685: [uint32x4_t; 2] = [vandq_u32(live[0], t684[0]), vandq_u32(live[1], t684[1])];
        {
            let t686: u64 = 4294967295u64;
            let t687 = [vdupq_n_u32((t686) as u32); 2];
            let t688: [uint32x4_t; 4] = [t687[0], t687[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t685[0], t688[0], res[0]), vbslq_u32(t685[1], t688[1], res[1]), vbslq_u32(t685[0], t688[2], res[2]), vbslq_u32(t685[1], t688[3], res[3])];
            live = [vbicq_u32(live[0], t685[0]), vbicq_u32(live[1], t685[1])];
        }
        if any(&live) {
            let t689: u64 = 2u64;
            let t690 = [vdupq_n_u32((t689) as u32); 2];
            let t691: [uint32x4_t; 2] = [vandq_u32(x0[0], t690[0]), vandq_u32(x0[1], t690[1])];
            let t692: u64 = 0u64;
            let t693 = [vdupq_n_u32((t692) as u32); 2];
            let t694: [uint32x4_t; 2] = [vceqq_u32(t691[0], t693[0]), vceqq_u32(t691[1], t693[1])];
            let t695: [uint32x4_t; 2] = [vmvnq_u32(t694[0]), vmvnq_u32(t694[1])];
            let t696: [uint32x4_t; 2] = [vandq_u32(live[0], t695[0]), vandq_u32(live[1], t695[1])];
            {
                let t697: [uint32x4_t; 2] = [vsubq_u32(x8[0], x16[0]), vsubq_u32(x8[1], x16[1])];
                let t698: [uint32x4_t; 4] = [t697[0], t697[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t696[0], t698[0], res[0]), vbslq_u32(t696[1], t698[1], res[1]), vbslq_u32(t696[0], t698[2], res[2]), vbslq_u32(t696[1], t698[3], res[3])];
                live = [vbicq_u32(live[0], t696[0]), vbicq_u32(live[1], t696[1])];
            }
            if any(&live) {
                let t699: [uint32x4_t; 2] = [vsubq_u32(x8[0], x16[0]), vsubq_u32(x8[1], x16[1])];
                let t700: u64 = (1u64).wrapping_shl((32u64) as u32);
                let t701 = [vdupq_n_u32((t700) as u32); 2];
                let t702 = [vdupq_n_u32(((t700) >> 32) as u32); 2];
                let t703: [uint32x4_t; 4] = [vorrq_u32(t701[0], t699[0]), vorrq_u32(t701[1], t699[1]), t702[0], t702[1]];
                res = [vbslq_u32(live[0], t703[0], res[0]), vbslq_u32(live[1], t703[1], res[1]), vbslq_u32(live[0], t703[2], res[2]), vbslq_u32(live[1], t703[3], res[3])];
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
    let (mut t704l, mut t704h) = ([0u32; 8], [0u32; 8]);
    { let v = (((((vgetq_lane_u32::<0>(x0[2]) as u64) << 32 | vgetq_lane_u32::<0>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<0>(x8[2]) as u64) << 32 | vgetq_lane_u32::<0>(x8[0]) as u64) as u64) as u128) >> 64) as u64; t704l[0] = v as u32; t704h[0] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<1>(x0[2]) as u64) << 32 | vgetq_lane_u32::<1>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<1>(x8[2]) as u64) << 32 | vgetq_lane_u32::<1>(x8[0]) as u64) as u64) as u128) >> 64) as u64; t704l[1] = v as u32; t704h[1] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<2>(x0[2]) as u64) << 32 | vgetq_lane_u32::<2>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<2>(x8[2]) as u64) << 32 | vgetq_lane_u32::<2>(x8[0]) as u64) as u64) as u128) >> 64) as u64; t704l[2] = v as u32; t704h[2] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<3>(x0[2]) as u64) << 32 | vgetq_lane_u32::<3>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<3>(x8[2]) as u64) << 32 | vgetq_lane_u32::<3>(x8[0]) as u64) as u64) as u128) >> 64) as u64; t704l[3] = v as u32; t704h[3] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<0>(x0[3]) as u64) << 32 | vgetq_lane_u32::<0>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<0>(x8[3]) as u64) << 32 | vgetq_lane_u32::<0>(x8[1]) as u64) as u64) as u128) >> 64) as u64; t704l[4] = v as u32; t704h[4] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<1>(x0[3]) as u64) << 32 | vgetq_lane_u32::<1>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<1>(x8[3]) as u64) << 32 | vgetq_lane_u32::<1>(x8[1]) as u64) as u64) as u128) >> 64) as u64; t704l[5] = v as u32; t704h[5] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<2>(x0[3]) as u64) << 32 | vgetq_lane_u32::<2>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<2>(x8[3]) as u64) << 32 | vgetq_lane_u32::<2>(x8[1]) as u64) as u64) as u128) >> 64) as u64; t704l[6] = v as u32; t704h[6] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<3>(x0[3]) as u64) << 32 | vgetq_lane_u32::<3>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<3>(x8[3]) as u64) << 32 | vgetq_lane_u32::<3>(x8[1]) as u64) as u64) as u128) >> 64) as u64; t704l[7] = v as u32; t704h[7] = (v >> 32) as u32; }
    let t705: [uint32x4_t; 4] = [vld1q_u32(t704l.as_ptr().add(0)), vld1q_u32(t704l.as_ptr().add(4)), vld1q_u32(t704h.as_ptr().add(0)), vld1q_u32(t704h.as_ptr().add(4))];
    res = t705;
    return res;
}

#[inline(always)]
unsafe fn b4_ll_f(l: &Launch, lid0: u32, x0: [uint32x4_t; 4], x8: [uint32x4_t; 4]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let (mut t706l, mut t706h) = ([0u32; 8], [0u32; 8]);
    { let v = (((((vgetq_lane_u32::<0>(x0[2]) as u64) << 32 | vgetq_lane_u32::<0>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<0>(x8[2]) as u64) << 32 | vgetq_lane_u32::<0>(x8[0]) as u64) as u64) as u128) >> 64) as u64; t706l[0] = v as u32; t706h[0] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<1>(x0[2]) as u64) << 32 | vgetq_lane_u32::<1>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<1>(x8[2]) as u64) << 32 | vgetq_lane_u32::<1>(x8[0]) as u64) as u64) as u128) >> 64) as u64; t706l[1] = v as u32; t706h[1] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<2>(x0[2]) as u64) << 32 | vgetq_lane_u32::<2>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<2>(x8[2]) as u64) << 32 | vgetq_lane_u32::<2>(x8[0]) as u64) as u64) as u128) >> 64) as u64; t706l[2] = v as u32; t706h[2] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<3>(x0[2]) as u64) << 32 | vgetq_lane_u32::<3>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<3>(x8[2]) as u64) << 32 | vgetq_lane_u32::<3>(x8[0]) as u64) as u64) as u128) >> 64) as u64; t706l[3] = v as u32; t706h[3] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<0>(x0[3]) as u64) << 32 | vgetq_lane_u32::<0>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<0>(x8[3]) as u64) << 32 | vgetq_lane_u32::<0>(x8[1]) as u64) as u64) as u128) >> 64) as u64; t706l[4] = v as u32; t706h[4] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<1>(x0[3]) as u64) << 32 | vgetq_lane_u32::<1>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<1>(x8[3]) as u64) << 32 | vgetq_lane_u32::<1>(x8[1]) as u64) as u64) as u128) >> 64) as u64; t706l[5] = v as u32; t706h[5] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<2>(x0[3]) as u64) << 32 | vgetq_lane_u32::<2>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<2>(x8[3]) as u64) << 32 | vgetq_lane_u32::<2>(x8[1]) as u64) as u64) as u128) >> 64) as u64; t706l[6] = v as u32; t706h[6] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<3>(x0[3]) as u64) << 32 | vgetq_lane_u32::<3>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<3>(x8[3]) as u64) << 32 | vgetq_lane_u32::<3>(x8[1]) as u64) as u64) as u128) >> 64) as u64; t706l[7] = v as u32; t706h[7] = (v >> 32) as u32; }
    let t707: [uint32x4_t; 4] = [vld1q_u32(t706l.as_ptr().add(0)), vld1q_u32(t706l.as_ptr().add(4)), vld1q_u32(t706h.as_ptr().add(0)), vld1q_u32(t706h.as_ptr().add(4))];
    res = t707;
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
        let t708: [uint32x4_t; 4];
        if (x8s) == 8u32 && ((x8f) as u64) + 7 * ((x8s) as u64) <= u32::MAX as u64 && all(&live) {
            let t709 = (arg0 as *const u8).add(((x8f) as usize) * 1);
            let t7100 = vld2q_u32((t709 as *const u32).add(0));
            let t7101 = vld2q_u32((t709 as *const u32).add(8));
            let t710: [uint32x4_t; 4] = [t7100.0, t7101.0, t7100.1, t7101.1];
            t708 = t710;
        } else {
            let t711: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8s)), vmlaq_u32(vdupq_n_u32(x8f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8s))];
            let t712 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t711[0]), vandq_u32(live[1], t711[1]))));
            let t713: [uint32x4_t; 2] = [vbslq_u32(live[0], t711[0], t712), vbslq_u32(live[1], t711[1], t712)];
            let mut t714: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t713[0]) as usize * 1) as *const u64).read_unaligned(); t714[0] = vsetq_lane_u32::<0>(v as u32, t714[0]); t714[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t714[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t713[0]) as usize * 1) as *const u64).read_unaligned(); t714[0] = vsetq_lane_u32::<1>(v as u32, t714[0]); t714[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t714[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t713[0]) as usize * 1) as *const u64).read_unaligned(); t714[0] = vsetq_lane_u32::<2>(v as u32, t714[0]); t714[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t714[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t713[0]) as usize * 1) as *const u64).read_unaligned(); t714[0] = vsetq_lane_u32::<3>(v as u32, t714[0]); t714[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t714[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t713[1]) as usize * 1) as *const u64).read_unaligned(); t714[1] = vsetq_lane_u32::<0>(v as u32, t714[1]); t714[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t714[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t713[1]) as usize * 1) as *const u64).read_unaligned(); t714[1] = vsetq_lane_u32::<1>(v as u32, t714[1]); t714[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t714[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t713[1]) as usize * 1) as *const u64).read_unaligned(); t714[1] = vsetq_lane_u32::<2>(v as u32, t714[1]); t714[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t714[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t713[1]) as usize * 1) as *const u64).read_unaligned(); t714[1] = vsetq_lane_u32::<3>(v as u32, t714[1]); t714[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t714[3]); }
            t708 = t714;
        }
        res = t708;
        return res;
    }
    let t715: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8s)), vmlaq_u32(vdupq_n_u32(x8f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8s))];
    let t716 = [vdupq_n_u32(!(7u32)); 2];
    let t717: [uint32x4_t; 2] = [vandq_u32(t715[0], t716[0]), vandq_u32(t715[1], t716[1])];
    let t718: [uint32x4_t; 2] = [t717[0], t717[1]];
    let t719: u64 = ((arg0) as usize as u64);
    let t720 = [vdupq_n_u32((t719) as u32); 2];
    let t721 = [vdupq_n_u32(((t719) >> 32) as u32); 2];
    let t722: [uint32x4_t; 2] = [vaddq_u32(t720[0], t718[0]), vaddq_u32(t720[1], t718[1])];
    let t723: [uint32x4_t; 2] = [vcltq_u32(t722[0], t720[0]), vcltq_u32(t722[1], t720[1])];
    let t724: [uint32x4_t; 4] = [t722[0], t722[1], vsubq_u32(vaddq_u32(t721[0], vdupq_n_u32(0)), t723[0]), vsubq_u32(vaddq_u32(t721[1], vdupq_n_u32(0)), t723[1])];
    let t725: [uint32x4_t; 4] = [t724[0], t724[1], t724[2], t724[3]];
    x24 = t725;
    let t726: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8s)), vmlaq_u32(vdupq_n_u32(x8f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8s))];
    let t727 = [vdupq_n_u32(7u32); 2];
    let t728: [uint32x4_t; 2] = [vandq_u32(t726[0], t727[0]), vandq_u32(t726[1], t727[1])];
    let t729 = [vdupq_n_u32(8u32); 2];
    let t730: [uint32x4_t; 2] = [vmulq_u32(t728[0], t729[0]), vmulq_u32(t728[1], t729[1])];
    x32 = t730;
    let mut t731: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x24[2]) as u64) << 32 | vgetq_lane_u32::<0>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t731[0] = vsetq_lane_u32::<0>(v as u32, t731[0]); t731[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t731[2]); }
    { let v = (((vgetq_lane_u32::<1>(x24[2]) as u64) << 32 | vgetq_lane_u32::<1>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t731[0] = vsetq_lane_u32::<1>(v as u32, t731[0]); t731[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t731[2]); }
    { let v = (((vgetq_lane_u32::<2>(x24[2]) as u64) << 32 | vgetq_lane_u32::<2>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t731[0] = vsetq_lane_u32::<2>(v as u32, t731[0]); t731[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t731[2]); }
    { let v = (((vgetq_lane_u32::<3>(x24[2]) as u64) << 32 | vgetq_lane_u32::<3>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t731[0] = vsetq_lane_u32::<3>(v as u32, t731[0]); t731[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t731[2]); }
    { let v = (((vgetq_lane_u32::<0>(x24[3]) as u64) << 32 | vgetq_lane_u32::<0>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t731[1] = vsetq_lane_u32::<0>(v as u32, t731[1]); t731[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t731[3]); }
    { let v = (((vgetq_lane_u32::<1>(x24[3]) as u64) << 32 | vgetq_lane_u32::<1>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t731[1] = vsetq_lane_u32::<1>(v as u32, t731[1]); t731[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t731[3]); }
    { let v = (((vgetq_lane_u32::<2>(x24[3]) as u64) << 32 | vgetq_lane_u32::<2>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t731[1] = vsetq_lane_u32::<2>(v as u32, t731[1]); t731[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t731[3]); }
    { let v = (((vgetq_lane_u32::<3>(x24[3]) as u64) << 32 | vgetq_lane_u32::<3>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t731[1] = vsetq_lane_u32::<3>(v as u32, t731[1]); t731[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t731[3]); }
    let t732: [uint32x4_t; 2] = [x32[0], x32[1]];
    let t733: [uint32x4_t; 2] = [vandq_u32(t732[0], vdupq_n_u32(63)), vandq_u32(t732[1], vdupq_n_u32(63))];
    let t734: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t731[0], vnegq_s32(vreinterpretq_s32_u32(t733[0]))), vshlq_u32(t731[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t733[0])))), vorrq_u32(vshlq_u32(t731[1], vnegq_s32(vreinterpretq_s32_u32(t733[1]))), vshlq_u32(t731[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t733[1])))), vshlq_u32(t731[2], vnegq_s32(vreinterpretq_s32_u32(t733[0]))), vshlq_u32(t731[3], vnegq_s32(vreinterpretq_s32_u32(t733[1])))];
    x40 = t734;
    let t735 = [vdupq_n_u32(0u32); 2];
    let t736: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t735[0])), vmvnq_u32(vceqq_u32(x32[1], t735[1]))];
    let t737 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t738: [uint32x4_t; 2] = [vaddq_u32(x32[0], t737[0]), vaddq_u32(x32[1], t737[1])];
    let t739 = [vdupq_n_u32(64u32); 2];
    let t740: [uint32x4_t; 2] = [vcgtq_u32(t738[0], t739[0]), vcgtq_u32(t738[1], t739[1])];
    let t741: [uint32x4_t; 2] = [vandq_u32(t736[0], t740[0]), vandq_u32(t736[1], t740[1])];
    let mut t742: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t743: [uint32x4_t; 2] = [vandq_u32(live[0], t741[0]), vandq_u32(live[1], t741[1])];
    let t744: [uint32x4_t; 2] = [vbicq_u32(live[0], t741[0]), vbicq_u32(live[1], t741[1])];
    if any(&t743) {
        let t745: u64 = (1u64).wrapping_mul(8u64);
        let t746 = [vdupq_n_u32((t745) as u32); 2];
        let t747 = [vdupq_n_u32(((t745) >> 32) as u32); 2];
        let t748: [uint32x4_t; 2] = [vaddq_u32(x24[0], t746[0]), vaddq_u32(x24[1], t746[1])];
        let t749: [uint32x4_t; 2] = [vcltq_u32(t748[0], x24[0]), vcltq_u32(t748[1], x24[1])];
        let t750: [uint32x4_t; 4] = [t748[0], t748[1], vsubq_u32(vaddq_u32(x24[2], t747[0]), t749[0]), vsubq_u32(vaddq_u32(x24[3], t747[1]), t749[1])];
        let mut t751: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t750[2]) as u64) << 32 | vgetq_lane_u32::<0>(t750[0]) as u64) as usize as *const u64).read_unaligned(); t751[0] = vsetq_lane_u32::<0>(v as u32, t751[0]); t751[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t751[2]); }
        { let v = (((vgetq_lane_u32::<1>(t750[2]) as u64) << 32 | vgetq_lane_u32::<1>(t750[0]) as u64) as usize as *const u64).read_unaligned(); t751[0] = vsetq_lane_u32::<1>(v as u32, t751[0]); t751[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t751[2]); }
        { let v = (((vgetq_lane_u32::<2>(t750[2]) as u64) << 32 | vgetq_lane_u32::<2>(t750[0]) as u64) as usize as *const u64).read_unaligned(); t751[0] = vsetq_lane_u32::<2>(v as u32, t751[0]); t751[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t751[2]); }
        { let v = (((vgetq_lane_u32::<3>(t750[2]) as u64) << 32 | vgetq_lane_u32::<3>(t750[0]) as u64) as usize as *const u64).read_unaligned(); t751[0] = vsetq_lane_u32::<3>(v as u32, t751[0]); t751[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t751[2]); }
        { let v = (((vgetq_lane_u32::<0>(t750[3]) as u64) << 32 | vgetq_lane_u32::<0>(t750[1]) as u64) as usize as *const u64).read_unaligned(); t751[1] = vsetq_lane_u32::<0>(v as u32, t751[1]); t751[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t751[3]); }
        { let v = (((vgetq_lane_u32::<1>(t750[3]) as u64) << 32 | vgetq_lane_u32::<1>(t750[1]) as u64) as usize as *const u64).read_unaligned(); t751[1] = vsetq_lane_u32::<1>(v as u32, t751[1]); t751[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t751[3]); }
        { let v = (((vgetq_lane_u32::<2>(t750[3]) as u64) << 32 | vgetq_lane_u32::<2>(t750[1]) as u64) as usize as *const u64).read_unaligned(); t751[1] = vsetq_lane_u32::<2>(v as u32, t751[1]); t751[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t751[3]); }
        { let v = (((vgetq_lane_u32::<3>(t750[3]) as u64) << 32 | vgetq_lane_u32::<3>(t750[1]) as u64) as usize as *const u64).read_unaligned(); t751[1] = vsetq_lane_u32::<3>(v as u32, t751[1]); t751[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t751[3]); }
        let t752: [uint32x4_t; 2] = [vsubq_u32(t739[0], x32[0]), vsubq_u32(t739[1], x32[1])];
        let t753: [uint32x4_t; 2] = [t752[0], t752[1]];
        let t754: [uint32x4_t; 2] = [vandq_u32(t753[0], vdupq_n_u32(63)), vandq_u32(t753[1], vdupq_n_u32(63))];
        let t755: [uint32x4_t; 4] = [vshlq_u32(t751[0], vreinterpretq_s32_u32(t754[0])), vshlq_u32(t751[1], vreinterpretq_s32_u32(t754[1])), vorrq_u32(vshlq_u32(t751[2], vreinterpretq_s32_u32(t754[0])), vshlq_u32(t751[0], vsubq_s32(vreinterpretq_s32_u32(t754[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t751[3], vreinterpretq_s32_u32(t754[1])), vshlq_u32(t751[1], vsubq_s32(vreinterpretq_s32_u32(t754[1]), vdupq_n_s32(32))))];
        let t756: [uint32x4_t; 4] = [vorrq_u32(x40[0], t755[0]), vorrq_u32(x40[1], t755[1]), vorrq_u32(x40[2], t755[2]), vorrq_u32(x40[3], t755[3])];
        t742 = [vbslq_u32(t743[0], t756[0], t742[0]), vbslq_u32(t743[1], t756[1], t742[1]), vbslq_u32(t743[0], t756[2], t742[2]), vbslq_u32(t743[1], t756[3], t742[3])];
    }
    {
        t742 = [vbslq_u32(t744[0], x40[0], t742[0]), vbslq_u32(t744[1], x40[1], t742[1]), vbslq_u32(t744[0], x40[2], t742[2]), vbslq_u32(t744[1], x40[3], t742[3])];
    }
    res = t742;
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
        let t757: [uint32x4_t; 4];
        if (x8s) == 8u32 && ((x8f) as u64) + 7 * ((x8s) as u64) <= u32::MAX as u64 {
            let t758 = (arg0 as *const u8).add(((x8f) as usize) * 1);
            let t7590 = vld2q_u32((t758 as *const u32).add(0));
            let t7591 = vld2q_u32((t758 as *const u32).add(8));
            let t759: [uint32x4_t; 4] = [t7590.0, t7591.0, t7590.1, t7591.1];
            t757 = t759;
        } else {
            let t760: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8s)), vmlaq_u32(vdupq_n_u32(x8f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8s))];
            let mut t761: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t760[0]) as usize * 1) as *const u64).read_unaligned(); t761[0] = vsetq_lane_u32::<0>(v as u32, t761[0]); t761[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t761[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t760[0]) as usize * 1) as *const u64).read_unaligned(); t761[0] = vsetq_lane_u32::<1>(v as u32, t761[0]); t761[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t761[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t760[0]) as usize * 1) as *const u64).read_unaligned(); t761[0] = vsetq_lane_u32::<2>(v as u32, t761[0]); t761[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t761[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t760[0]) as usize * 1) as *const u64).read_unaligned(); t761[0] = vsetq_lane_u32::<3>(v as u32, t761[0]); t761[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t761[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t760[1]) as usize * 1) as *const u64).read_unaligned(); t761[1] = vsetq_lane_u32::<0>(v as u32, t761[1]); t761[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t761[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t760[1]) as usize * 1) as *const u64).read_unaligned(); t761[1] = vsetq_lane_u32::<1>(v as u32, t761[1]); t761[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t761[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t760[1]) as usize * 1) as *const u64).read_unaligned(); t761[1] = vsetq_lane_u32::<2>(v as u32, t761[1]); t761[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t761[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t760[1]) as usize * 1) as *const u64).read_unaligned(); t761[1] = vsetq_lane_u32::<3>(v as u32, t761[1]); t761[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t761[3]); }
            t757 = t761;
        }
        res = t757;
        return res;
    }
    let t762: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8s)), vmlaq_u32(vdupq_n_u32(x8f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8s))];
    let t763 = [vdupq_n_u32(!(7u32)); 2];
    let t764: [uint32x4_t; 2] = [vandq_u32(t762[0], t763[0]), vandq_u32(t762[1], t763[1])];
    let t765: [uint32x4_t; 2] = [t764[0], t764[1]];
    let t766: u64 = ((arg0) as usize as u64);
    let t767 = [vdupq_n_u32((t766) as u32); 2];
    let t768 = [vdupq_n_u32(((t766) >> 32) as u32); 2];
    let t769: [uint32x4_t; 2] = [vaddq_u32(t767[0], t765[0]), vaddq_u32(t767[1], t765[1])];
    let t770: [uint32x4_t; 2] = [vcltq_u32(t769[0], t767[0]), vcltq_u32(t769[1], t767[1])];
    let t771: [uint32x4_t; 4] = [t769[0], t769[1], vsubq_u32(vaddq_u32(t768[0], vdupq_n_u32(0)), t770[0]), vsubq_u32(vaddq_u32(t768[1], vdupq_n_u32(0)), t770[1])];
    let t772: [uint32x4_t; 4] = [t771[0], t771[1], t771[2], t771[3]];
    x24 = t772;
    let t773: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8s)), vmlaq_u32(vdupq_n_u32(x8f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8s))];
    let t774 = [vdupq_n_u32(7u32); 2];
    let t775: [uint32x4_t; 2] = [vandq_u32(t773[0], t774[0]), vandq_u32(t773[1], t774[1])];
    let t776 = [vdupq_n_u32(8u32); 2];
    let t777: [uint32x4_t; 2] = [vmulq_u32(t775[0], t776[0]), vmulq_u32(t775[1], t776[1])];
    x32 = t777;
    let mut t778: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x24[2]) as u64) << 32 | vgetq_lane_u32::<0>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t778[0] = vsetq_lane_u32::<0>(v as u32, t778[0]); t778[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t778[2]); }
    { let v = (((vgetq_lane_u32::<1>(x24[2]) as u64) << 32 | vgetq_lane_u32::<1>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t778[0] = vsetq_lane_u32::<1>(v as u32, t778[0]); t778[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t778[2]); }
    { let v = (((vgetq_lane_u32::<2>(x24[2]) as u64) << 32 | vgetq_lane_u32::<2>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t778[0] = vsetq_lane_u32::<2>(v as u32, t778[0]); t778[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t778[2]); }
    { let v = (((vgetq_lane_u32::<3>(x24[2]) as u64) << 32 | vgetq_lane_u32::<3>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t778[0] = vsetq_lane_u32::<3>(v as u32, t778[0]); t778[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t778[2]); }
    { let v = (((vgetq_lane_u32::<0>(x24[3]) as u64) << 32 | vgetq_lane_u32::<0>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t778[1] = vsetq_lane_u32::<0>(v as u32, t778[1]); t778[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t778[3]); }
    { let v = (((vgetq_lane_u32::<1>(x24[3]) as u64) << 32 | vgetq_lane_u32::<1>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t778[1] = vsetq_lane_u32::<1>(v as u32, t778[1]); t778[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t778[3]); }
    { let v = (((vgetq_lane_u32::<2>(x24[3]) as u64) << 32 | vgetq_lane_u32::<2>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t778[1] = vsetq_lane_u32::<2>(v as u32, t778[1]); t778[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t778[3]); }
    { let v = (((vgetq_lane_u32::<3>(x24[3]) as u64) << 32 | vgetq_lane_u32::<3>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t778[1] = vsetq_lane_u32::<3>(v as u32, t778[1]); t778[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t778[3]); }
    let t779: [uint32x4_t; 2] = [x32[0], x32[1]];
    let t780: [uint32x4_t; 2] = [vandq_u32(t779[0], vdupq_n_u32(63)), vandq_u32(t779[1], vdupq_n_u32(63))];
    let t781: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t778[0], vnegq_s32(vreinterpretq_s32_u32(t780[0]))), vshlq_u32(t778[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t780[0])))), vorrq_u32(vshlq_u32(t778[1], vnegq_s32(vreinterpretq_s32_u32(t780[1]))), vshlq_u32(t778[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t780[1])))), vshlq_u32(t778[2], vnegq_s32(vreinterpretq_s32_u32(t780[0]))), vshlq_u32(t778[3], vnegq_s32(vreinterpretq_s32_u32(t780[1])))];
    x40 = t781;
    let t782 = [vdupq_n_u32(0u32); 2];
    let t783: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t782[0])), vmvnq_u32(vceqq_u32(x32[1], t782[1]))];
    let t784 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t785: [uint32x4_t; 2] = [vaddq_u32(x32[0], t784[0]), vaddq_u32(x32[1], t784[1])];
    let t786 = [vdupq_n_u32(64u32); 2];
    let t787: [uint32x4_t; 2] = [vcgtq_u32(t785[0], t786[0]), vcgtq_u32(t785[1], t786[1])];
    let t788: [uint32x4_t; 2] = [vandq_u32(t783[0], t787[0]), vandq_u32(t783[1], t787[1])];
    let mut t789: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t790: [uint32x4_t; 2] = [vmvnq_u32(t788[0]), vmvnq_u32(t788[1])];
    if any(&t788) {
        let t791: u64 = (1u64).wrapping_mul(8u64);
        let t792 = [vdupq_n_u32((t791) as u32); 2];
        let t793 = [vdupq_n_u32(((t791) >> 32) as u32); 2];
        let t794: [uint32x4_t; 2] = [vaddq_u32(x24[0], t792[0]), vaddq_u32(x24[1], t792[1])];
        let t795: [uint32x4_t; 2] = [vcltq_u32(t794[0], x24[0]), vcltq_u32(t794[1], x24[1])];
        let t796: [uint32x4_t; 4] = [t794[0], t794[1], vsubq_u32(vaddq_u32(x24[2], t793[0]), t795[0]), vsubq_u32(vaddq_u32(x24[3], t793[1]), t795[1])];
        let mut t797: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t796[2]) as u64) << 32 | vgetq_lane_u32::<0>(t796[0]) as u64) as usize as *const u64).read_unaligned(); t797[0] = vsetq_lane_u32::<0>(v as u32, t797[0]); t797[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t797[2]); }
        { let v = (((vgetq_lane_u32::<1>(t796[2]) as u64) << 32 | vgetq_lane_u32::<1>(t796[0]) as u64) as usize as *const u64).read_unaligned(); t797[0] = vsetq_lane_u32::<1>(v as u32, t797[0]); t797[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t797[2]); }
        { let v = (((vgetq_lane_u32::<2>(t796[2]) as u64) << 32 | vgetq_lane_u32::<2>(t796[0]) as u64) as usize as *const u64).read_unaligned(); t797[0] = vsetq_lane_u32::<2>(v as u32, t797[0]); t797[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t797[2]); }
        { let v = (((vgetq_lane_u32::<3>(t796[2]) as u64) << 32 | vgetq_lane_u32::<3>(t796[0]) as u64) as usize as *const u64).read_unaligned(); t797[0] = vsetq_lane_u32::<3>(v as u32, t797[0]); t797[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t797[2]); }
        { let v = (((vgetq_lane_u32::<0>(t796[3]) as u64) << 32 | vgetq_lane_u32::<0>(t796[1]) as u64) as usize as *const u64).read_unaligned(); t797[1] = vsetq_lane_u32::<0>(v as u32, t797[1]); t797[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t797[3]); }
        { let v = (((vgetq_lane_u32::<1>(t796[3]) as u64) << 32 | vgetq_lane_u32::<1>(t796[1]) as u64) as usize as *const u64).read_unaligned(); t797[1] = vsetq_lane_u32::<1>(v as u32, t797[1]); t797[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t797[3]); }
        { let v = (((vgetq_lane_u32::<2>(t796[3]) as u64) << 32 | vgetq_lane_u32::<2>(t796[1]) as u64) as usize as *const u64).read_unaligned(); t797[1] = vsetq_lane_u32::<2>(v as u32, t797[1]); t797[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t797[3]); }
        { let v = (((vgetq_lane_u32::<3>(t796[3]) as u64) << 32 | vgetq_lane_u32::<3>(t796[1]) as u64) as usize as *const u64).read_unaligned(); t797[1] = vsetq_lane_u32::<3>(v as u32, t797[1]); t797[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t797[3]); }
        let t798: [uint32x4_t; 2] = [vsubq_u32(t786[0], x32[0]), vsubq_u32(t786[1], x32[1])];
        let t799: [uint32x4_t; 2] = [t798[0], t798[1]];
        let t800: [uint32x4_t; 2] = [vandq_u32(t799[0], vdupq_n_u32(63)), vandq_u32(t799[1], vdupq_n_u32(63))];
        let t801: [uint32x4_t; 4] = [vshlq_u32(t797[0], vreinterpretq_s32_u32(t800[0])), vshlq_u32(t797[1], vreinterpretq_s32_u32(t800[1])), vorrq_u32(vshlq_u32(t797[2], vreinterpretq_s32_u32(t800[0])), vshlq_u32(t797[0], vsubq_s32(vreinterpretq_s32_u32(t800[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t797[3], vreinterpretq_s32_u32(t800[1])), vshlq_u32(t797[1], vsubq_s32(vreinterpretq_s32_u32(t800[1]), vdupq_n_s32(32))))];
        let t802: [uint32x4_t; 4] = [vorrq_u32(x40[0], t801[0]), vorrq_u32(x40[1], t801[1]), vorrq_u32(x40[2], t801[2]), vorrq_u32(x40[3], t801[3])];
        t789 = [vbslq_u32(t788[0], t802[0], t789[0]), vbslq_u32(t788[1], t802[1], t789[1]), vbslq_u32(t788[0], t802[2], t789[2]), vbslq_u32(t788[1], t802[3], t789[3])];
    }
    {
        t789 = [vbslq_u32(t790[0], x40[0], t789[0]), vbslq_u32(t790[1], x40[1], t789[1]), vbslq_u32(t790[0], x40[2], t789[2]), vbslq_u32(t790[1], x40[3], t789[3])];
    }
    res = t789;
    return res;
}

#[inline(always)]
unsafe fn b6_i_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t803: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x0), vld1q_u32(LANE[1].as_ptr()))];
    let t804: [uint32x4_t; 2] = [vshlq_u32(t803[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(t803[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
    let t805 = [vdupq_n_u32(255u32); 2];
    let t806: [uint32x4_t; 2] = [vandq_u32(t803[0], t805[0]), vandq_u32(t803[1], t805[1])];
    let t807: [uint32x4_t; 2] = [vmulq_u32(t804[0], vdupq_n_u32(1000)), vmulq_u32(t804[1], vdupq_n_u32(1000))];
    let t808: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(t804[0]), vget_low_u32(vdupq_n_u32(1000)))), vreinterpretq_u32_u64(vmull_high_u32(t804[0], vdupq_n_u32(1000)))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(t804[1]), vget_low_u32(vdupq_n_u32(1000)))), vreinterpretq_u32_u64(vmull_high_u32(t804[1], vdupq_n_u32(1000))))];
    let t809: [uint32x4_t; 2] = [vaddq_u32(t807[0], t806[0]), vaddq_u32(t807[1], t806[1])];
    let t810: [uint32x4_t; 2] = [vcltq_u32(t809[0], t807[0]), vcltq_u32(t809[1], t807[1])];
    let t811: [uint32x4_t; 4] = [t809[0], t809[1], vsubq_u32(vaddq_u32(t808[0], vdupq_n_u32(0)), t810[0]), vsubq_u32(vaddq_u32(t808[1], vdupq_n_u32(0)), t810[1])];
    res = t811;
    return res;
}

#[inline(always)]
unsafe fn b6_i_f(l: &Launch, lid0: u32, x0: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t812: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x0), vld1q_u32(LANE[1].as_ptr()))];
    let t813: [uint32x4_t; 2] = [vshlq_u32(t812[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(t812[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
    let t814 = [vdupq_n_u32(255u32); 2];
    let t815: [uint32x4_t; 2] = [vandq_u32(t812[0], t814[0]), vandq_u32(t812[1], t814[1])];
    let t816: [uint32x4_t; 2] = [vmulq_u32(t813[0], vdupq_n_u32(1000)), vmulq_u32(t813[1], vdupq_n_u32(1000))];
    let t817: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(t813[0]), vget_low_u32(vdupq_n_u32(1000)))), vreinterpretq_u32_u64(vmull_high_u32(t813[0], vdupq_n_u32(1000)))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(t813[1]), vget_low_u32(vdupq_n_u32(1000)))), vreinterpretq_u32_u64(vmull_high_u32(t813[1], vdupq_n_u32(1000))))];
    let t818: [uint32x4_t; 2] = [vaddq_u32(t816[0], t815[0]), vaddq_u32(t816[1], t815[1])];
    let t819: [uint32x4_t; 2] = [vcltq_u32(t818[0], t816[0]), vcltq_u32(t818[1], t816[1])];
    let t820: [uint32x4_t; 4] = [t818[0], t818[1], vsubq_u32(vaddq_u32(t817[0], vdupq_n_u32(0)), t819[0]), vsubq_u32(vaddq_u32(t817[1], vdupq_n_u32(0)), t819[1])];
    res = t820;
    return res;
}

#[inline(always)]
unsafe fn b6_u8_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t821: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32((x0 & 255u32)), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32((x0 & 255u32)), vld1q_u32(LANE[1].as_ptr()))];
    let t822: [uint32x4_t; 2] = [t821[0], t821[1]];
    let t823: u64 = ((((((((x0 >> 8)) as u64)).wrapping_mul(1000u64)) as usize as *const u8)) as usize as u64);
    let t824 = [vdupq_n_u32((t823) as u32); 2];
    let t825 = [vdupq_n_u32(((t823) >> 32) as u32); 2];
    let t826: [uint32x4_t; 2] = [vaddq_u32(t824[0], t822[0]), vaddq_u32(t824[1], t822[1])];
    let t827: [uint32x4_t; 2] = [vcltq_u32(t826[0], t824[0]), vcltq_u32(t826[1], t824[1])];
    let t828: [uint32x4_t; 4] = [t826[0], t826[1], vsubq_u32(vaddq_u32(t825[0], vdupq_n_u32(0)), t827[0]), vsubq_u32(vaddq_u32(t825[1], vdupq_n_u32(0)), t827[1])];
    let t829: [uint32x4_t; 4] = [t828[0], t828[1], t828[2], t828[3]];
    res = t829;
    return res;
}

#[inline(always)]
unsafe fn b6_u8_f(l: &Launch, lid0: u32, x0: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t830: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32((x0 & 255u32)), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32((x0 & 255u32)), vld1q_u32(LANE[1].as_ptr()))];
    let t831: [uint32x4_t; 2] = [t830[0], t830[1]];
    let t832: u64 = ((((((((x0 >> 8)) as u64)).wrapping_mul(1000u64)) as usize as *const u8)) as usize as u64);
    let t833 = [vdupq_n_u32((t832) as u32); 2];
    let t834 = [vdupq_n_u32(((t832) >> 32) as u32); 2];
    let t835: [uint32x4_t; 2] = [vaddq_u32(t833[0], t831[0]), vaddq_u32(t833[1], t831[1])];
    let t836: [uint32x4_t; 2] = [vcltq_u32(t835[0], t833[0]), vcltq_u32(t835[1], t833[1])];
    let t837: [uint32x4_t; 4] = [t835[0], t835[1], vsubq_u32(vaddq_u32(t834[0], vdupq_n_u32(0)), t836[0]), vsubq_u32(vaddq_u32(t834[1], vdupq_n_u32(0)), t836[1])];
    let t838: [uint32x4_t; 4] = [t837[0], t837[1], t837[2], t837[3]];
    res = t838;
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
    let mut t839: bool = !(arg5).is_null();
    if t839 {
        let mut t840: bool = ((l.group[0]) >= ((arg5.wrapping_add((((x56) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        if !t840 {
            t840 = ((l.group[1]) >= ((arg5.wrapping_add(((((x56).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        let mut t841: bool = t840;
        if !t841 {
            t841 = ((l.group[2]) >= ((arg5.wrapping_add(((((x56).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        t839 = t841;
    }
    if t839 {
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
        let t842: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
        let mut live = t842;
        let gid0 = l.group[0] * l.block[0] + lid0;
        'chunk: {
            b0_r0r1r2r3sg_p(l, lid0, live, arg0, arg1, arg2, arg3, x32);
        }
    }
}
