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
unsafe fn b0_r0r1r2r3sg_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, arg2: *const u8, arg3: *const u8, x4: *const u8) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x4 as *const u32).read_unaligned();
    let a1: u64 = (x4.wrapping_add(8u64 as usize) as *const u64).read_unaligned();
    let mut x6: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x7: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x8: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x9: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x10: u32 = 0;
    let mut x11: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x12: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x13: u32 = 0;
    let mut x14: u32 = 0;
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
        x6 = t6;
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
        x7 = t12;
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
        x8 = t17;
        let t23 = [vdupq_n_u32(63u32); 2];
        let t24: [uint32x4_t; 2] = [vandq_u32(x8[0], t23[0]), vandq_u32(x8[1], t23[1])];
        x9 = t24;
        x10 = a0;
        let t25: [uint32x4_t; 2] = [vaddq_u32(x6[0], x7[0]), vaddq_u32(x6[1], x7[1])];
        let t26: [uint32x4_t; 2] = [vcltq_u32(t25[0], x6[0]), vcltq_u32(t25[1], x6[1])];
        let t27: [uint32x4_t; 4] = [t25[0], t25[1], vsubq_u32(vaddq_u32(x6[2], x7[2]), t26[0]), vsubq_u32(vaddq_u32(x6[3], x7[3]), t26[1])];
        let t28 = (arg3 as *const u8).add(((gid0.wrapping_add((0u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t28 as *mut u32).add(0), uint32x4x2_t(t27[0], t27[2]));
            vst2q_u32((t28 as *mut u32).add(8), uint32x4x2_t(t27[1], t27[3]));
        } else {
            let mut t29w = 0u64;
            let t29 = core::hint::black_box(&raw mut t29w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t28 as *mut u64).add(0), t29).write_unaligned(((vgetq_lane_u32::<0>(t27[2]) as u64) << 32 | vgetq_lane_u32::<0>(t27[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t28 as *mut u64).add(1), t29).write_unaligned(((vgetq_lane_u32::<1>(t27[2]) as u64) << 32 | vgetq_lane_u32::<1>(t27[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t28 as *mut u64).add(2), t29).write_unaligned(((vgetq_lane_u32::<2>(t27[2]) as u64) << 32 | vgetq_lane_u32::<2>(t27[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t28 as *mut u64).add(3), t29).write_unaligned(((vgetq_lane_u32::<3>(t27[2]) as u64) << 32 | vgetq_lane_u32::<3>(t27[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t28 as *mut u64).add(4), t29).write_unaligned(((vgetq_lane_u32::<0>(t27[3]) as u64) << 32 | vgetq_lane_u32::<0>(t27[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t28 as *mut u64).add(5), t29).write_unaligned(((vgetq_lane_u32::<1>(t27[3]) as u64) << 32 | vgetq_lane_u32::<1>(t27[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t28 as *mut u64).add(6), t29).write_unaligned(((vgetq_lane_u32::<2>(t27[3]) as u64) << 32 | vgetq_lane_u32::<2>(t27[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t28 as *mut u64).add(7), t29).write_unaligned(((vgetq_lane_u32::<3>(t27[3]) as u64) << 32 | vgetq_lane_u32::<3>(t27[1]) as u64));
        }
        let t30: [uint32x4_t; 2] = [vcltq_u32(x6[0], x7[0]), vcltq_u32(x6[1], x7[1])];
        let t31: [uint32x4_t; 4] = [vsubq_u32(x6[0], x7[0]), vsubq_u32(x6[1], x7[1]), vaddq_u32(vsubq_u32(x6[2], x7[2]), t30[0]), vaddq_u32(vsubq_u32(x6[3], x7[3]), t30[1])];
        let t32 = (arg3 as *const u8).add(((gid0.wrapping_add((1u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t32 as *mut u32).add(0), uint32x4x2_t(t31[0], t31[2]));
            vst2q_u32((t32 as *mut u32).add(8), uint32x4x2_t(t31[1], t31[3]));
        } else {
            let mut t33w = 0u64;
            let t33 = core::hint::black_box(&raw mut t33w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t32 as *mut u64).add(0), t33).write_unaligned(((vgetq_lane_u32::<0>(t31[2]) as u64) << 32 | vgetq_lane_u32::<0>(t31[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t32 as *mut u64).add(1), t33).write_unaligned(((vgetq_lane_u32::<1>(t31[2]) as u64) << 32 | vgetq_lane_u32::<1>(t31[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t32 as *mut u64).add(2), t33).write_unaligned(((vgetq_lane_u32::<2>(t31[2]) as u64) << 32 | vgetq_lane_u32::<2>(t31[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t32 as *mut u64).add(3), t33).write_unaligned(((vgetq_lane_u32::<3>(t31[2]) as u64) << 32 | vgetq_lane_u32::<3>(t31[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t32 as *mut u64).add(4), t33).write_unaligned(((vgetq_lane_u32::<0>(t31[3]) as u64) << 32 | vgetq_lane_u32::<0>(t31[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t32 as *mut u64).add(5), t33).write_unaligned(((vgetq_lane_u32::<1>(t31[3]) as u64) << 32 | vgetq_lane_u32::<1>(t31[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t32 as *mut u64).add(6), t33).write_unaligned(((vgetq_lane_u32::<2>(t31[3]) as u64) << 32 | vgetq_lane_u32::<2>(t31[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t32 as *mut u64).add(7), t33).write_unaligned(((vgetq_lane_u32::<3>(t31[3]) as u64) << 32 | vgetq_lane_u32::<3>(t31[1]) as u64));
        }
        let t34: [uint32x4_t; 2] = [x9[0], x9[1]];
        let t35: [uint32x4_t; 2] = [vandq_u32(t34[0], vdupq_n_u32(63)), vandq_u32(t34[1], vdupq_n_u32(63))];
        let t36: [uint32x4_t; 4] = [vshlq_u32(x6[0], vreinterpretq_s32_u32(t35[0])), vshlq_u32(x6[1], vreinterpretq_s32_u32(t35[1])), vorrq_u32(vshlq_u32(x6[2], vreinterpretq_s32_u32(t35[0])), vshlq_u32(x6[0], vsubq_s32(vreinterpretq_s32_u32(t35[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(x6[3], vreinterpretq_s32_u32(t35[1])), vshlq_u32(x6[1], vsubq_s32(vreinterpretq_s32_u32(t35[1]), vdupq_n_s32(32))))];
        let t37 = (arg3 as *const u8).add(((gid0.wrapping_add((2u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t37 as *mut u32).add(0), uint32x4x2_t(t36[0], t36[2]));
            vst2q_u32((t37 as *mut u32).add(8), uint32x4x2_t(t36[1], t36[3]));
        } else {
            let mut t38w = 0u64;
            let t38 = core::hint::black_box(&raw mut t38w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t37 as *mut u64).add(0), t38).write_unaligned(((vgetq_lane_u32::<0>(t36[2]) as u64) << 32 | vgetq_lane_u32::<0>(t36[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t37 as *mut u64).add(1), t38).write_unaligned(((vgetq_lane_u32::<1>(t36[2]) as u64) << 32 | vgetq_lane_u32::<1>(t36[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t37 as *mut u64).add(2), t38).write_unaligned(((vgetq_lane_u32::<2>(t36[2]) as u64) << 32 | vgetq_lane_u32::<2>(t36[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t37 as *mut u64).add(3), t38).write_unaligned(((vgetq_lane_u32::<3>(t36[2]) as u64) << 32 | vgetq_lane_u32::<3>(t36[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t37 as *mut u64).add(4), t38).write_unaligned(((vgetq_lane_u32::<0>(t36[3]) as u64) << 32 | vgetq_lane_u32::<0>(t36[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t37 as *mut u64).add(5), t38).write_unaligned(((vgetq_lane_u32::<1>(t36[3]) as u64) << 32 | vgetq_lane_u32::<1>(t36[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t37 as *mut u64).add(6), t38).write_unaligned(((vgetq_lane_u32::<2>(t36[3]) as u64) << 32 | vgetq_lane_u32::<2>(t36[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t37 as *mut u64).add(7), t38).write_unaligned(((vgetq_lane_u32::<3>(t36[3]) as u64) << 32 | vgetq_lane_u32::<3>(t36[1]) as u64));
        }
        let t39: [uint32x4_t; 2] = [x9[0], x9[1]];
        let t40: [uint32x4_t; 2] = [vandq_u32(t39[0], vdupq_n_u32(63)), vandq_u32(t39[1], vdupq_n_u32(63))];
        let t41: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(x6[0], vnegq_s32(vreinterpretq_s32_u32(t40[0]))), vshlq_u32(x6[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t40[0])))), vorrq_u32(vshlq_u32(x6[1], vnegq_s32(vreinterpretq_s32_u32(t40[1]))), vshlq_u32(x6[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t40[1])))), vshlq_u32(x6[2], vnegq_s32(vreinterpretq_s32_u32(t40[0]))), vshlq_u32(x6[3], vnegq_s32(vreinterpretq_s32_u32(t40[1])))];
        let t42 = (arg3 as *const u8).add(((gid0.wrapping_add((3u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t42 as *mut u32).add(0), uint32x4x2_t(t41[0], t41[2]));
            vst2q_u32((t42 as *mut u32).add(8), uint32x4x2_t(t41[1], t41[3]));
        } else {
            let mut t43w = 0u64;
            let t43 = core::hint::black_box(&raw mut t43w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t42 as *mut u64).add(0), t43).write_unaligned(((vgetq_lane_u32::<0>(t41[2]) as u64) << 32 | vgetq_lane_u32::<0>(t41[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t42 as *mut u64).add(1), t43).write_unaligned(((vgetq_lane_u32::<1>(t41[2]) as u64) << 32 | vgetq_lane_u32::<1>(t41[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t42 as *mut u64).add(2), t43).write_unaligned(((vgetq_lane_u32::<2>(t41[2]) as u64) << 32 | vgetq_lane_u32::<2>(t41[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t42 as *mut u64).add(3), t43).write_unaligned(((vgetq_lane_u32::<3>(t41[2]) as u64) << 32 | vgetq_lane_u32::<3>(t41[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t42 as *mut u64).add(4), t43).write_unaligned(((vgetq_lane_u32::<0>(t41[3]) as u64) << 32 | vgetq_lane_u32::<0>(t41[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t42 as *mut u64).add(5), t43).write_unaligned(((vgetq_lane_u32::<1>(t41[3]) as u64) << 32 | vgetq_lane_u32::<1>(t41[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t42 as *mut u64).add(6), t43).write_unaligned(((vgetq_lane_u32::<2>(t41[3]) as u64) << 32 | vgetq_lane_u32::<2>(t41[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t42 as *mut u64).add(7), t43).write_unaligned(((vgetq_lane_u32::<3>(t41[3]) as u64) << 32 | vgetq_lane_u32::<3>(t41[1]) as u64));
        }
        let t44: [uint32x4_t; 2] = [x9[0], x9[1]];
        let t45: [uint32x4_t; 2] = [vandq_u32(t44[0], vdupq_n_u32(63)), vandq_u32(t44[1], vdupq_n_u32(63))];
        let t46: [uint32x4_t; 2] = [vcgeq_s32(vreinterpretq_s32_u32(t45[0]), vdupq_n_s32(32)), vcgeq_s32(vreinterpretq_s32_u32(t45[1]), vdupq_n_s32(32))];
        let t47: [uint32x4_t; 4] = [vbslq_u32(t46[0], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t45[0])))), vorrq_u32(vshlq_u32(x6[0], vnegq_s32(vreinterpretq_s32_u32(t45[0]))), vshlq_u32(x6[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t45[0]))))), vbslq_u32(t46[1], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t45[1])))), vorrq_u32(vshlq_u32(x6[1], vnegq_s32(vreinterpretq_s32_u32(t45[1]))), vshlq_u32(x6[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t45[1]))))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vnegq_s32(vreinterpretq_s32_u32(t45[0])))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vnegq_s32(vreinterpretq_s32_u32(t45[1]))))];
        let t48 = (arg3 as *const u8).add(((gid0.wrapping_add((4u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t48 as *mut u32).add(0), uint32x4x2_t(t47[0], t47[2]));
            vst2q_u32((t48 as *mut u32).add(8), uint32x4x2_t(t47[1], t47[3]));
        } else {
            let mut t49w = 0u64;
            let t49 = core::hint::black_box(&raw mut t49w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t48 as *mut u64).add(0), t49).write_unaligned(((vgetq_lane_u32::<0>(t47[2]) as u64) << 32 | vgetq_lane_u32::<0>(t47[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t48 as *mut u64).add(1), t49).write_unaligned(((vgetq_lane_u32::<1>(t47[2]) as u64) << 32 | vgetq_lane_u32::<1>(t47[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t48 as *mut u64).add(2), t49).write_unaligned(((vgetq_lane_u32::<2>(t47[2]) as u64) << 32 | vgetq_lane_u32::<2>(t47[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t48 as *mut u64).add(3), t49).write_unaligned(((vgetq_lane_u32::<3>(t47[2]) as u64) << 32 | vgetq_lane_u32::<3>(t47[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t48 as *mut u64).add(4), t49).write_unaligned(((vgetq_lane_u32::<0>(t47[3]) as u64) << 32 | vgetq_lane_u32::<0>(t47[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t48 as *mut u64).add(5), t49).write_unaligned(((vgetq_lane_u32::<1>(t47[3]) as u64) << 32 | vgetq_lane_u32::<1>(t47[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t48 as *mut u64).add(6), t49).write_unaligned(((vgetq_lane_u32::<2>(t47[3]) as u64) << 32 | vgetq_lane_u32::<2>(t47[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t48 as *mut u64).add(7), t49).write_unaligned(((vgetq_lane_u32::<3>(t47[3]) as u64) << 32 | vgetq_lane_u32::<3>(t47[1]) as u64));
        }
        let t50: [uint32x4_t; 4] = [vshlq_n_u32::<13>(x6[0]), vshlq_n_u32::<13>(x6[1]), vorrq_u32(vshlq_n_u32::<13>(x6[2]), vshrq_n_u32::<19>(x6[0])), vorrq_u32(vshlq_n_u32::<13>(x6[3]), vshrq_n_u32::<19>(x6[1]))];
        let t51: [uint32x4_t; 2] = [vshrq_n_u32::<19>(x7[2]), vshrq_n_u32::<19>(x7[3])];
        let t52: [uint32x4_t; 4] = [vorrq_u32(t50[0], t51[0]), vorrq_u32(t50[1], t51[1]), t50[2], t50[3]];
        let t53 = (arg3 as *const u8).add(((gid0.wrapping_add((5u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t53 as *mut u32).add(0), uint32x4x2_t(t52[0], t52[2]));
            vst2q_u32((t53 as *mut u32).add(8), uint32x4x2_t(t52[1], t52[3]));
        } else {
            let mut t54w = 0u64;
            let t54 = core::hint::black_box(&raw mut t54w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t53 as *mut u64).add(0), t54).write_unaligned(((vgetq_lane_u32::<0>(t52[2]) as u64) << 32 | vgetq_lane_u32::<0>(t52[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t53 as *mut u64).add(1), t54).write_unaligned(((vgetq_lane_u32::<1>(t52[2]) as u64) << 32 | vgetq_lane_u32::<1>(t52[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t53 as *mut u64).add(2), t54).write_unaligned(((vgetq_lane_u32::<2>(t52[2]) as u64) << 32 | vgetq_lane_u32::<2>(t52[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t53 as *mut u64).add(3), t54).write_unaligned(((vgetq_lane_u32::<3>(t52[2]) as u64) << 32 | vgetq_lane_u32::<3>(t52[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t53 as *mut u64).add(4), t54).write_unaligned(((vgetq_lane_u32::<0>(t52[3]) as u64) << 32 | vgetq_lane_u32::<0>(t52[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t53 as *mut u64).add(5), t54).write_unaligned(((vgetq_lane_u32::<1>(t52[3]) as u64) << 32 | vgetq_lane_u32::<1>(t52[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t53 as *mut u64).add(6), t54).write_unaligned(((vgetq_lane_u32::<2>(t52[3]) as u64) << 32 | vgetq_lane_u32::<2>(t52[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t53 as *mut u64).add(7), t54).write_unaligned(((vgetq_lane_u32::<3>(t52[3]) as u64) << 32 | vgetq_lane_u32::<3>(t52[1]) as u64));
        }
        let t55: [uint32x4_t; 4] = [vdupq_n_u32(0), vdupq_n_u32(0), vshlq_n_u32::<8>(x6[0]), vshlq_n_u32::<8>(x6[1])];
        let t56 = (arg3 as *const u8).add(((gid0.wrapping_add((6u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t56 as *mut u32).add(0), uint32x4x2_t(t55[0], t55[2]));
            vst2q_u32((t56 as *mut u32).add(8), uint32x4x2_t(t55[1], t55[3]));
        } else {
            let mut t57w = 0u64;
            let t57 = core::hint::black_box(&raw mut t57w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t56 as *mut u64).add(0), t57).write_unaligned(((vgetq_lane_u32::<0>(t55[2]) as u64) << 32 | vgetq_lane_u32::<0>(t55[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t56 as *mut u64).add(1), t57).write_unaligned(((vgetq_lane_u32::<1>(t55[2]) as u64) << 32 | vgetq_lane_u32::<1>(t55[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t56 as *mut u64).add(2), t57).write_unaligned(((vgetq_lane_u32::<2>(t55[2]) as u64) << 32 | vgetq_lane_u32::<2>(t55[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t56 as *mut u64).add(3), t57).write_unaligned(((vgetq_lane_u32::<3>(t55[2]) as u64) << 32 | vgetq_lane_u32::<3>(t55[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t56 as *mut u64).add(4), t57).write_unaligned(((vgetq_lane_u32::<0>(t55[3]) as u64) << 32 | vgetq_lane_u32::<0>(t55[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t56 as *mut u64).add(5), t57).write_unaligned(((vgetq_lane_u32::<1>(t55[3]) as u64) << 32 | vgetq_lane_u32::<1>(t55[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t56 as *mut u64).add(6), t57).write_unaligned(((vgetq_lane_u32::<2>(t55[3]) as u64) << 32 | vgetq_lane_u32::<2>(t55[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t56 as *mut u64).add(7), t57).write_unaligned(((vgetq_lane_u32::<3>(t55[3]) as u64) << 32 | vgetq_lane_u32::<3>(t55[1]) as u64));
        }
        let t58: [uint32x4_t; 2] = [vshrq_n_u32::<3>(x6[2]), vshrq_n_u32::<3>(x6[3])];
        let t59: [uint32x4_t; 4] = [t58[0], t58[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        let t60 = (arg3 as *const u8).add(((gid0.wrapping_add((7u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t60 as *mut u32).add(0), uint32x4x2_t(t59[0], t59[2]));
            vst2q_u32((t60 as *mut u32).add(8), uint32x4x2_t(t59[1], t59[3]));
        } else {
            let mut t61w = 0u64;
            let t61 = core::hint::black_box(&raw mut t61w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t60 as *mut u64).add(0), t61).write_unaligned(((vgetq_lane_u32::<0>(t59[2]) as u64) << 32 | vgetq_lane_u32::<0>(t59[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t60 as *mut u64).add(1), t61).write_unaligned(((vgetq_lane_u32::<1>(t59[2]) as u64) << 32 | vgetq_lane_u32::<1>(t59[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t60 as *mut u64).add(2), t61).write_unaligned(((vgetq_lane_u32::<2>(t59[2]) as u64) << 32 | vgetq_lane_u32::<2>(t59[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t60 as *mut u64).add(3), t61).write_unaligned(((vgetq_lane_u32::<3>(t59[2]) as u64) << 32 | vgetq_lane_u32::<3>(t59[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t60 as *mut u64).add(4), t61).write_unaligned(((vgetq_lane_u32::<0>(t59[3]) as u64) << 32 | vgetq_lane_u32::<0>(t59[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t60 as *mut u64).add(5), t61).write_unaligned(((vgetq_lane_u32::<1>(t59[3]) as u64) << 32 | vgetq_lane_u32::<1>(t59[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t60 as *mut u64).add(6), t61).write_unaligned(((vgetq_lane_u32::<2>(t59[3]) as u64) << 32 | vgetq_lane_u32::<2>(t59[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t60 as *mut u64).add(7), t61).write_unaligned(((vgetq_lane_u32::<3>(t59[3]) as u64) << 32 | vgetq_lane_u32::<3>(t59[1]) as u64));
        }
        let t62 = vdupq_n_s32(((((40i64) as u32)) & 63) as i32);
        let t63: [uint32x4_t; 2] = [vcgeq_s32(t62, vdupq_n_s32(32)), vcgeq_s32(t62, vdupq_n_s32(32))];
        let t64: [uint32x4_t; 4] = [vbslq_u32(t63[0], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vsubq_s32(vdupq_n_s32(32), t62))), vorrq_u32(vshlq_u32(x6[0], vnegq_s32(t62)), vshlq_u32(x6[2], vsubq_s32(vdupq_n_s32(32), t62)))), vbslq_u32(t63[1], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vsubq_s32(vdupq_n_s32(32), t62))), vorrq_u32(vshlq_u32(x6[1], vnegq_s32(t62)), vshlq_u32(x6[3], vsubq_s32(vdupq_n_s32(32), t62)))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vnegq_s32(t62))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vnegq_s32(t62)))];
        let t65 = (arg3 as *const u8).add(((gid0.wrapping_add((8u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t65 as *mut u32).add(0), uint32x4x2_t(t64[0], t64[2]));
            vst2q_u32((t65 as *mut u32).add(8), uint32x4x2_t(t64[1], t64[3]));
        } else {
            let mut t66w = 0u64;
            let t66 = core::hint::black_box(&raw mut t66w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t65 as *mut u64).add(0), t66).write_unaligned(((vgetq_lane_u32::<0>(t64[2]) as u64) << 32 | vgetq_lane_u32::<0>(t64[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t65 as *mut u64).add(1), t66).write_unaligned(((vgetq_lane_u32::<1>(t64[2]) as u64) << 32 | vgetq_lane_u32::<1>(t64[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t65 as *mut u64).add(2), t66).write_unaligned(((vgetq_lane_u32::<2>(t64[2]) as u64) << 32 | vgetq_lane_u32::<2>(t64[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t65 as *mut u64).add(3), t66).write_unaligned(((vgetq_lane_u32::<3>(t64[2]) as u64) << 32 | vgetq_lane_u32::<3>(t64[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t65 as *mut u64).add(4), t66).write_unaligned(((vgetq_lane_u32::<0>(t64[3]) as u64) << 32 | vgetq_lane_u32::<0>(t64[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t65 as *mut u64).add(5), t66).write_unaligned(((vgetq_lane_u32::<1>(t64[3]) as u64) << 32 | vgetq_lane_u32::<1>(t64[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t65 as *mut u64).add(6), t66).write_unaligned(((vgetq_lane_u32::<2>(t64[3]) as u64) << 32 | vgetq_lane_u32::<2>(t64[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t65 as *mut u64).add(7), t66).write_unaligned(((vgetq_lane_u32::<3>(t64[3]) as u64) << 32 | vgetq_lane_u32::<3>(t64[1]) as u64));
        }
        let t67: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x6[0]), vget_low_u32(x7[0]))), vreinterpretq_u32_u64(vmull_high_u32(x6[0], x7[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x6[1]), vget_low_u32(x7[1]))), vreinterpretq_u32_u64(vmull_high_u32(x6[1], x7[1])))];
        let t68: [uint32x4_t; 4] = [vmulq_u32(x6[0], x7[0]), vmulq_u32(x6[1], x7[1]), vmlaq_u32(vmlaq_u32(t67[0], x6[0], x7[2]), x7[0], x6[2]), vmlaq_u32(vmlaq_u32(t67[1], x6[1], x7[3]), x7[1], x6[3])];
        let t69 = (arg3 as *const u8).add(((gid0.wrapping_add((9u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t69 as *mut u32).add(0), uint32x4x2_t(t68[0], t68[2]));
            vst2q_u32((t69 as *mut u32).add(8), uint32x4x2_t(t68[1], t68[3]));
        } else {
            let mut t70w = 0u64;
            let t70 = core::hint::black_box(&raw mut t70w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t69 as *mut u64).add(0), t70).write_unaligned(((vgetq_lane_u32::<0>(t68[2]) as u64) << 32 | vgetq_lane_u32::<0>(t68[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t69 as *mut u64).add(1), t70).write_unaligned(((vgetq_lane_u32::<1>(t68[2]) as u64) << 32 | vgetq_lane_u32::<1>(t68[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t69 as *mut u64).add(2), t70).write_unaligned(((vgetq_lane_u32::<2>(t68[2]) as u64) << 32 | vgetq_lane_u32::<2>(t68[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t69 as *mut u64).add(3), t70).write_unaligned(((vgetq_lane_u32::<3>(t68[2]) as u64) << 32 | vgetq_lane_u32::<3>(t68[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t69 as *mut u64).add(4), t70).write_unaligned(((vgetq_lane_u32::<0>(t68[3]) as u64) << 32 | vgetq_lane_u32::<0>(t68[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t69 as *mut u64).add(5), t70).write_unaligned(((vgetq_lane_u32::<1>(t68[3]) as u64) << 32 | vgetq_lane_u32::<1>(t68[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t69 as *mut u64).add(6), t70).write_unaligned(((vgetq_lane_u32::<2>(t68[3]) as u64) << 32 | vgetq_lane_u32::<2>(t68[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t69 as *mut u64).add(7), t70).write_unaligned(((vgetq_lane_u32::<3>(t68[3]) as u64) << 32 | vgetq_lane_u32::<3>(t68[1]) as u64));
        }
        let t71: u64 = 0u64;
        let t72 = [vdupq_n_u32((t71) as u32); 2];
        let t73: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x7[0], t72[0]), vceqq_u32(x7[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(x7[1], t72[1]), vceqq_u32(x7[3], vdupq_n_u32(0)))];
        let t74: [uint32x4_t; 2] = [vmvnq_u32(t73[0]), vmvnq_u32(t73[1])];
        let mut t75: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t76: [uint32x4_t; 2] = [vandq_u32(live[0], t74[0]), vandq_u32(live[1], t74[1])];
        let t77: [uint32x4_t; 2] = [vbicq_u32(live[0], t74[0]), vbicq_u32(live[1], t74[1])];
        {
            let (mut t78xl, mut t78xh, mut t78yl, mut t78yh) = ([0u32; 8], [0u32; 8], [0u32; 8], [0u32; 8]);
            vst1q_u32(t78xl.as_mut_ptr().add(0), x6[0]); vst1q_u32(t78xh.as_mut_ptr().add(0), x6[2]); vst1q_u32(t78yl.as_mut_ptr().add(0), x7[0]); vst1q_u32(t78yh.as_mut_ptr().add(0), x7[2]);
            vst1q_u32(t78xl.as_mut_ptr().add(4), x6[1]); vst1q_u32(t78xh.as_mut_ptr().add(4), x6[3]); vst1q_u32(t78yl.as_mut_ptr().add(4), x7[1]); vst1q_u32(t78yh.as_mut_ptr().add(4), x7[3]);
            for j in 0..8 { let q = (((t78xh[j] as u64) << 32 | t78xl[j] as u64) as u64).checked_div(((t78yh[j] as u64) << 32 | t78yl[j] as u64) as u64).unwrap_or(0) as u64; t78xl[j] = q as u32; t78xh[j] = (q >> 32) as u32; }
            let t79: [uint32x4_t; 4] = [vld1q_u32(t78xl.as_ptr().add(0)), vld1q_u32(t78xl.as_ptr().add(4)), vld1q_u32(t78xh.as_ptr().add(0)), vld1q_u32(t78xh.as_ptr().add(4))];
            t75 = [vbslq_u32(t76[0], t79[0], t75[0]), vbslq_u32(t76[1], t79[1], t75[1]), vbslq_u32(t76[0], t79[2], t75[2]), vbslq_u32(t76[1], t79[3], t75[3])];
        }
        {
            let t80: u64 = 7u64;
            let t81 = [vdupq_n_u32((t80) as u32); 2];
            let t82: [uint32x4_t; 4] = [t81[0], t81[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            t75 = [vbslq_u32(t77[0], t82[0], t75[0]), vbslq_u32(t77[1], t82[1], t75[1]), vbslq_u32(t77[0], t82[2], t75[2]), vbslq_u32(t77[1], t82[3], t75[3])];
        }
        let t83 = (arg3 as *const u8).add(((gid0.wrapping_add((10u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t83 as *mut u32).add(0), uint32x4x2_t(t75[0], t75[2]));
            vst2q_u32((t83 as *mut u32).add(8), uint32x4x2_t(t75[1], t75[3]));
        } else {
            let mut t84w = 0u64;
            let t84 = core::hint::black_box(&raw mut t84w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t83 as *mut u64).add(0), t84).write_unaligned(((vgetq_lane_u32::<0>(t75[2]) as u64) << 32 | vgetq_lane_u32::<0>(t75[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t83 as *mut u64).add(1), t84).write_unaligned(((vgetq_lane_u32::<1>(t75[2]) as u64) << 32 | vgetq_lane_u32::<1>(t75[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t83 as *mut u64).add(2), t84).write_unaligned(((vgetq_lane_u32::<2>(t75[2]) as u64) << 32 | vgetq_lane_u32::<2>(t75[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t83 as *mut u64).add(3), t84).write_unaligned(((vgetq_lane_u32::<3>(t75[2]) as u64) << 32 | vgetq_lane_u32::<3>(t75[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t83 as *mut u64).add(4), t84).write_unaligned(((vgetq_lane_u32::<0>(t75[3]) as u64) << 32 | vgetq_lane_u32::<0>(t75[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t83 as *mut u64).add(5), t84).write_unaligned(((vgetq_lane_u32::<1>(t75[3]) as u64) << 32 | vgetq_lane_u32::<1>(t75[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t83 as *mut u64).add(6), t84).write_unaligned(((vgetq_lane_u32::<2>(t75[3]) as u64) << 32 | vgetq_lane_u32::<2>(t75[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t83 as *mut u64).add(7), t84).write_unaligned(((vgetq_lane_u32::<3>(t75[3]) as u64) << 32 | vgetq_lane_u32::<3>(t75[1]) as u64));
        }
        let t85: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x6[2], x7[2]), vandq_u32(vceqq_u32(x6[2], x7[2]), vcltq_u32(x6[0], x7[0]))), vorrq_u32(vcltq_u32(x6[3], x7[3]), vandq_u32(vceqq_u32(x6[3], x7[3]), vcltq_u32(x6[1], x7[1])))];
        let t86: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t85[0]), vshrq_n_u32::<31>(t85[1])];
        let t87: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x7[2], x6[2]), vandq_u32(vceqq_u32(x7[2], x6[2]), vcltq_u32(x7[0], x6[0]))), vorrq_u32(vcltq_u32(x7[3], x6[3]), vandq_u32(vceqq_u32(x7[3], x6[3]), vcltq_u32(x7[1], x6[1])))];
        let t88: [uint32x4_t; 2] = [vmvnq_u32(t87[0]), vmvnq_u32(t87[1])];
        let t89: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t88[0]), vshrq_n_u32::<31>(t88[1])];
        let t90: [uint32x4_t; 4] = [vshlq_n_u32::<1>(t89[0]), vshlq_n_u32::<1>(t89[1]), vshrq_n_u32::<31>(t89[0]), vshrq_n_u32::<31>(t89[1])];
        let t91: [uint32x4_t; 4] = [vorrq_u32(t86[0], t90[0]), vorrq_u32(t86[1], t90[1]), t90[2], t90[3]];
        let t92: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x6[0], x7[0]), vceqq_u32(x6[2], x7[2])), vandq_u32(vceqq_u32(x6[1], x7[1]), vceqq_u32(x6[3], x7[3]))];
        let t93: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t92[0]), vshrq_n_u32::<31>(t92[1])];
        let t94: [uint32x4_t; 4] = [vshlq_n_u32::<2>(t93[0]), vshlq_n_u32::<2>(t93[1]), vshrq_n_u32::<30>(t93[0]), vshrq_n_u32::<30>(t93[1])];
        let t95: [uint32x4_t; 4] = [vorrq_u32(t91[0], t94[0]), vorrq_u32(t91[1], t94[1]), vorrq_u32(t91[2], t94[2]), vorrq_u32(t91[3], t94[3])];
        let t96: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x6[0], x7[0]), vceqq_u32(x6[2], x7[2])), vandq_u32(vceqq_u32(x6[1], x7[1]), vceqq_u32(x6[3], x7[3]))];
        let t97: [uint32x4_t; 2] = [vmvnq_u32(t96[0]), vmvnq_u32(t96[1])];
        let t98: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t97[0]), vshrq_n_u32::<31>(t97[1])];
        let t99: [uint32x4_t; 4] = [vshlq_n_u32::<3>(t98[0]), vshlq_n_u32::<3>(t98[1]), vshrq_n_u32::<29>(t98[0]), vshrq_n_u32::<29>(t98[1])];
        let t100: [uint32x4_t; 4] = [vorrq_u32(t95[0], t99[0]), vorrq_u32(t95[1], t99[1]), vorrq_u32(t95[2], t99[2]), vorrq_u32(t95[3], t99[3])];
        let t101: [uint32x4_t; 2] = [vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x6[2]), vreinterpretq_s32_u32(x7[2])), vandq_u32(vceqq_u32(x6[2], x7[2]), vcltq_u32(x6[0], x7[0]))), vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x6[3]), vreinterpretq_s32_u32(x7[3])), vandq_u32(vceqq_u32(x6[3], x7[3]), vcltq_u32(x6[1], x7[1])))];
        let t102: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t101[0]), vshrq_n_u32::<31>(t101[1])];
        let t103: [uint32x4_t; 4] = [vshlq_n_u32::<4>(t102[0]), vshlq_n_u32::<4>(t102[1]), vshrq_n_u32::<28>(t102[0]), vshrq_n_u32::<28>(t102[1])];
        let t104: [uint32x4_t; 4] = [vorrq_u32(t100[0], t103[0]), vorrq_u32(t100[1], t103[1]), vorrq_u32(t100[2], t103[2]), vorrq_u32(t100[3], t103[3])];
        let t105: [uint32x4_t; 2] = [vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x6[2]), vreinterpretq_s32_u32(x7[2])), vandq_u32(vceqq_u32(x6[2], x7[2]), vcltq_u32(x6[0], x7[0]))), vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x6[3]), vreinterpretq_s32_u32(x7[3])), vandq_u32(vceqq_u32(x6[3], x7[3]), vcltq_u32(x6[1], x7[1])))];
        let t106: [uint32x4_t; 2] = [vmvnq_u32(t105[0]), vmvnq_u32(t105[1])];
        let t107: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t106[0]), vshrq_n_u32::<31>(t106[1])];
        let t108: [uint32x4_t; 4] = [vshlq_n_u32::<5>(t107[0]), vshlq_n_u32::<5>(t107[1]), vshrq_n_u32::<27>(t107[0]), vshrq_n_u32::<27>(t107[1])];
        let t109: [uint32x4_t; 4] = [vorrq_u32(t104[0], t108[0]), vorrq_u32(t104[1], t108[1]), vorrq_u32(t104[2], t108[2]), vorrq_u32(t104[3], t108[3])];
        let t110: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x7[2], x6[2]), vandq_u32(vceqq_u32(x7[2], x6[2]), vcltq_u32(x7[0], x6[0]))), vorrq_u32(vcltq_u32(x7[3], x6[3]), vandq_u32(vceqq_u32(x7[3], x6[3]), vcltq_u32(x7[1], x6[1])))];
        let t111: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t110[0]), vshrq_n_u32::<31>(t110[1])];
        let t112: [uint32x4_t; 4] = [vshlq_n_u32::<6>(t111[0]), vshlq_n_u32::<6>(t111[1]), vshrq_n_u32::<26>(t111[0]), vshrq_n_u32::<26>(t111[1])];
        let t113: [uint32x4_t; 4] = [vorrq_u32(t109[0], t112[0]), vorrq_u32(t109[1], t112[1]), vorrq_u32(t109[2], t112[2]), vorrq_u32(t109[3], t112[3])];
        let t114 = (arg3 as *const u8).add(((gid0.wrapping_add((11u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t114 as *mut u32).add(0), uint32x4x2_t(t113[0], t113[2]));
            vst2q_u32((t114 as *mut u32).add(8), uint32x4x2_t(t113[1], t113[3]));
        } else {
            let mut t115w = 0u64;
            let t115 = core::hint::black_box(&raw mut t115w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t114 as *mut u64).add(0), t115).write_unaligned(((vgetq_lane_u32::<0>(t113[2]) as u64) << 32 | vgetq_lane_u32::<0>(t113[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t114 as *mut u64).add(1), t115).write_unaligned(((vgetq_lane_u32::<1>(t113[2]) as u64) << 32 | vgetq_lane_u32::<1>(t113[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t114 as *mut u64).add(2), t115).write_unaligned(((vgetq_lane_u32::<2>(t113[2]) as u64) << 32 | vgetq_lane_u32::<2>(t113[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t114 as *mut u64).add(3), t115).write_unaligned(((vgetq_lane_u32::<3>(t113[2]) as u64) << 32 | vgetq_lane_u32::<3>(t113[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t114 as *mut u64).add(4), t115).write_unaligned(((vgetq_lane_u32::<0>(t113[3]) as u64) << 32 | vgetq_lane_u32::<0>(t113[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t114 as *mut u64).add(5), t115).write_unaligned(((vgetq_lane_u32::<1>(t113[3]) as u64) << 32 | vgetq_lane_u32::<1>(t113[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t114 as *mut u64).add(6), t115).write_unaligned(((vgetq_lane_u32::<2>(t113[3]) as u64) << 32 | vgetq_lane_u32::<2>(t113[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t114 as *mut u64).add(7), t115).write_unaligned(((vgetq_lane_u32::<3>(t113[3]) as u64) << 32 | vgetq_lane_u32::<3>(t113[1]) as u64));
        }
        let t116: [uint32x4_t; 2] = [vaddq_u32(x8[0], x6[0]), vaddq_u32(x8[1], x6[1])];
        let t117: [uint32x4_t; 2] = [vcltq_u32(t116[0], x8[0]), vcltq_u32(t116[1], x8[1])];
        let t118: [uint32x4_t; 4] = [t116[0], t116[1], vsubq_u32(vaddq_u32(vdupq_n_u32(0), x6[2]), t117[0]), vsubq_u32(vaddq_u32(vdupq_n_u32(0), x6[3]), t117[1])];
        let t119 = (arg3 as *const u8).add(((gid0.wrapping_add((12u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t119 as *mut u32).add(0), uint32x4x2_t(t118[0], t118[2]));
            vst2q_u32((t119 as *mut u32).add(8), uint32x4x2_t(t118[1], t118[3]));
        } else {
            let mut t120w = 0u64;
            let t120 = core::hint::black_box(&raw mut t120w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t119 as *mut u64).add(0), t120).write_unaligned(((vgetq_lane_u32::<0>(t118[2]) as u64) << 32 | vgetq_lane_u32::<0>(t118[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t119 as *mut u64).add(1), t120).write_unaligned(((vgetq_lane_u32::<1>(t118[2]) as u64) << 32 | vgetq_lane_u32::<1>(t118[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t119 as *mut u64).add(2), t120).write_unaligned(((vgetq_lane_u32::<2>(t118[2]) as u64) << 32 | vgetq_lane_u32::<2>(t118[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t119 as *mut u64).add(3), t120).write_unaligned(((vgetq_lane_u32::<3>(t118[2]) as u64) << 32 | vgetq_lane_u32::<3>(t118[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t119 as *mut u64).add(4), t120).write_unaligned(((vgetq_lane_u32::<0>(t118[3]) as u64) << 32 | vgetq_lane_u32::<0>(t118[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t119 as *mut u64).add(5), t120).write_unaligned(((vgetq_lane_u32::<1>(t118[3]) as u64) << 32 | vgetq_lane_u32::<1>(t118[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t119 as *mut u64).add(6), t120).write_unaligned(((vgetq_lane_u32::<2>(t118[3]) as u64) << 32 | vgetq_lane_u32::<2>(t118[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t119 as *mut u64).add(7), t120).write_unaligned(((vgetq_lane_u32::<3>(t118[3]) as u64) << 32 | vgetq_lane_u32::<3>(t118[1]) as u64));
        }
        let t121: [uint32x4_t; 2] = [vreinterpretq_u32_s32(vshrq_n_s32::<31>(vreinterpretq_s32_u32(x8[0]))), vreinterpretq_u32_s32(vshrq_n_s32::<31>(vreinterpretq_s32_u32(x8[1])))];
        let t122: [uint32x4_t; 4] = [x8[0], x8[1], t121[0], t121[1]];
        let t123 = (arg3 as *const u8).add(((gid0.wrapping_add((13u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t123 as *mut u32).add(0), uint32x4x2_t(t122[0], t122[2]));
            vst2q_u32((t123 as *mut u32).add(8), uint32x4x2_t(t122[1], t122[3]));
        } else {
            let mut t124w = 0u64;
            let t124 = core::hint::black_box(&raw mut t124w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t123 as *mut u64).add(0), t124).write_unaligned(((vgetq_lane_u32::<0>(t122[2]) as u64) << 32 | vgetq_lane_u32::<0>(t122[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t123 as *mut u64).add(1), t124).write_unaligned(((vgetq_lane_u32::<1>(t122[2]) as u64) << 32 | vgetq_lane_u32::<1>(t122[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t123 as *mut u64).add(2), t124).write_unaligned(((vgetq_lane_u32::<2>(t122[2]) as u64) << 32 | vgetq_lane_u32::<2>(t122[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t123 as *mut u64).add(3), t124).write_unaligned(((vgetq_lane_u32::<3>(t122[2]) as u64) << 32 | vgetq_lane_u32::<3>(t122[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t123 as *mut u64).add(4), t124).write_unaligned(((vgetq_lane_u32::<0>(t122[3]) as u64) << 32 | vgetq_lane_u32::<0>(t122[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t123 as *mut u64).add(5), t124).write_unaligned(((vgetq_lane_u32::<1>(t122[3]) as u64) << 32 | vgetq_lane_u32::<1>(t122[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t123 as *mut u64).add(6), t124).write_unaligned(((vgetq_lane_u32::<2>(t122[3]) as u64) << 32 | vgetq_lane_u32::<2>(t122[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t123 as *mut u64).add(7), t124).write_unaligned(((vgetq_lane_u32::<3>(t122[3]) as u64) << 32 | vgetq_lane_u32::<3>(t122[1]) as u64));
        }
        let t125: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<3>(x6[0]), vshlq_n_u32::<29>(x6[2])), vorrq_u32(vshrq_n_u32::<3>(x6[1]), vshlq_n_u32::<29>(x6[3])), vshrq_n_u32::<3>(x6[2]), vshrq_n_u32::<3>(x6[3])];
        let t126: [uint32x4_t; 2] = [t125[0], t125[1]];
        let t127: [uint32x4_t; 4] = [t126[0], t126[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        let t128 = (arg3 as *const u8).add(((gid0.wrapping_add((14u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t128 as *mut u32).add(0), uint32x4x2_t(t127[0], t127[2]));
            vst2q_u32((t128 as *mut u32).add(8), uint32x4x2_t(t127[1], t127[3]));
        } else {
            let mut t129w = 0u64;
            let t129 = core::hint::black_box(&raw mut t129w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t128 as *mut u64).add(0), t129).write_unaligned(((vgetq_lane_u32::<0>(t127[2]) as u64) << 32 | vgetq_lane_u32::<0>(t127[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t128 as *mut u64).add(1), t129).write_unaligned(((vgetq_lane_u32::<1>(t127[2]) as u64) << 32 | vgetq_lane_u32::<1>(t127[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t128 as *mut u64).add(2), t129).write_unaligned(((vgetq_lane_u32::<2>(t127[2]) as u64) << 32 | vgetq_lane_u32::<2>(t127[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t128 as *mut u64).add(3), t129).write_unaligned(((vgetq_lane_u32::<3>(t127[2]) as u64) << 32 | vgetq_lane_u32::<3>(t127[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t128 as *mut u64).add(4), t129).write_unaligned(((vgetq_lane_u32::<0>(t127[3]) as u64) << 32 | vgetq_lane_u32::<0>(t127[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t128 as *mut u64).add(5), t129).write_unaligned(((vgetq_lane_u32::<1>(t127[3]) as u64) << 32 | vgetq_lane_u32::<1>(t127[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t128 as *mut u64).add(6), t129).write_unaligned(((vgetq_lane_u32::<2>(t127[3]) as u64) << 32 | vgetq_lane_u32::<2>(t127[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t128 as *mut u64).add(7), t129).write_unaligned(((vgetq_lane_u32::<3>(t127[3]) as u64) << 32 | vgetq_lane_u32::<3>(t127[1]) as u64));
        }
        let t130: u64 = 1u64;
        let t131 = [vdupq_n_u32((t130) as u32); 2];
        let t132: [uint32x4_t; 2] = [vandq_u32(x6[0], t131[0]), vandq_u32(x6[1], t131[1])];
        let t133: [uint32x4_t; 2] = [vtstq_u32(t132[0], t132[0]), vtstq_u32(t132[1], t132[1])];
        let t134: [uint32x4_t; 2] = [vandq_u32(live[0], t133[0]), vandq_u32(live[1], t133[1])];
        {
            let t135: [uint32x4_t; 4] = [veorq_u32(x6[0], x7[0]), veorq_u32(x6[1], x7[1]), veorq_u32(x6[2], x7[2]), veorq_u32(x6[3], x7[3])];
            x11 = [vbslq_u32(t134[0], t135[0], x11[0]), vbslq_u32(t134[1], t135[1], x11[1]), vbslq_u32(t134[0], t135[2], x11[2]), vbslq_u32(t134[1], t135[3], x11[3])];
        }
        let t136: [uint32x4_t; 2] = [vbicq_u32(live[0], t133[0]), vbicq_u32(live[1], t133[1])];
        {
            let t137: [uint32x4_t; 2] = [vaddq_u32(x6[0], x8[0]), vaddq_u32(x6[1], x8[1])];
            let t138: [uint32x4_t; 2] = [vcltq_u32(t137[0], x6[0]), vcltq_u32(t137[1], x6[1])];
            let t139: [uint32x4_t; 4] = [t137[0], t137[1], vsubq_u32(vaddq_u32(x6[2], vdupq_n_u32(0)), t138[0]), vsubq_u32(vaddq_u32(x6[3], vdupq_n_u32(0)), t138[1])];
            x11 = [vbslq_u32(t136[0], t139[0], x11[0]), vbslq_u32(t136[1], t139[1], x11[1]), vbslq_u32(t136[0], t139[2], x11[2]), vbslq_u32(t136[1], t139[3], x11[3])];
        }
        let t140 = (arg3 as *const u8).add(((gid0.wrapping_add((15u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t140 as *mut u32).add(0), uint32x4x2_t(x11[0], x11[2]));
            vst2q_u32((t140 as *mut u32).add(8), uint32x4x2_t(x11[1], x11[3]));
        } else {
            let mut t141w = 0u64;
            let t141 = core::hint::black_box(&raw mut t141w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t140 as *mut u64).add(0), t141).write_unaligned(((vgetq_lane_u32::<0>(x11[2]) as u64) << 32 | vgetq_lane_u32::<0>(x11[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t140 as *mut u64).add(1), t141).write_unaligned(((vgetq_lane_u32::<1>(x11[2]) as u64) << 32 | vgetq_lane_u32::<1>(x11[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t140 as *mut u64).add(2), t141).write_unaligned(((vgetq_lane_u32::<2>(x11[2]) as u64) << 32 | vgetq_lane_u32::<2>(x11[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t140 as *mut u64).add(3), t141).write_unaligned(((vgetq_lane_u32::<3>(x11[2]) as u64) << 32 | vgetq_lane_u32::<3>(x11[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t140 as *mut u64).add(4), t141).write_unaligned(((vgetq_lane_u32::<0>(x11[3]) as u64) << 32 | vgetq_lane_u32::<0>(x11[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t140 as *mut u64).add(5), t141).write_unaligned(((vgetq_lane_u32::<1>(x11[3]) as u64) << 32 | vgetq_lane_u32::<1>(x11[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t140 as *mut u64).add(6), t141).write_unaligned(((vgetq_lane_u32::<2>(x11[3]) as u64) << 32 | vgetq_lane_u32::<2>(x11[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t140 as *mut u64).add(7), t141).write_unaligned(((vgetq_lane_u32::<3>(x11[3]) as u64) << 32 | vgetq_lane_u32::<3>(x11[1]) as u64));
        }
        let t142 = b1_l_p(l, lid0, live, x6);
        let t143 = b2_l_p(l, lid0, live, x7);
        let t144: [uint32x4_t; 4] = [vshlq_n_u32::<8>(t143[0]), vshlq_n_u32::<8>(t143[1]), vshrq_n_u32::<24>(t143[0]), vshrq_n_u32::<24>(t143[1])];
        let t145: [uint32x4_t; 2] = [vaddq_u32(t142[0], t144[0]), vaddq_u32(t142[1], t144[1])];
        let t146: [uint32x4_t; 2] = [vcltq_u32(t145[0], t142[0]), vcltq_u32(t145[1], t142[1])];
        let t147: [uint32x4_t; 4] = [t145[0], t145[1], vsubq_u32(vaddq_u32(vdupq_n_u32(0), t144[2]), t146[0]), vsubq_u32(vaddq_u32(vdupq_n_u32(0), t144[3]), t146[1])];
        let t148 = (arg3 as *const u8).add(((gid0.wrapping_add((16u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t148 as *mut u32).add(0), uint32x4x2_t(t147[0], t147[2]));
            vst2q_u32((t148 as *mut u32).add(8), uint32x4x2_t(t147[1], t147[3]));
        } else {
            let mut t149w = 0u64;
            let t149 = core::hint::black_box(&raw mut t149w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t148 as *mut u64).add(0), t149).write_unaligned(((vgetq_lane_u32::<0>(t147[2]) as u64) << 32 | vgetq_lane_u32::<0>(t147[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t148 as *mut u64).add(1), t149).write_unaligned(((vgetq_lane_u32::<1>(t147[2]) as u64) << 32 | vgetq_lane_u32::<1>(t147[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t148 as *mut u64).add(2), t149).write_unaligned(((vgetq_lane_u32::<2>(t147[2]) as u64) << 32 | vgetq_lane_u32::<2>(t147[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t148 as *mut u64).add(3), t149).write_unaligned(((vgetq_lane_u32::<3>(t147[2]) as u64) << 32 | vgetq_lane_u32::<3>(t147[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t148 as *mut u64).add(4), t149).write_unaligned(((vgetq_lane_u32::<0>(t147[3]) as u64) << 32 | vgetq_lane_u32::<0>(t147[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t148 as *mut u64).add(5), t149).write_unaligned(((vgetq_lane_u32::<1>(t147[3]) as u64) << 32 | vgetq_lane_u32::<1>(t147[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t148 as *mut u64).add(6), t149).write_unaligned(((vgetq_lane_u32::<2>(t147[3]) as u64) << 32 | vgetq_lane_u32::<2>(t147[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t148 as *mut u64).add(7), t149).write_unaligned(((vgetq_lane_u32::<3>(t147[3]) as u64) << 32 | vgetq_lane_u32::<3>(t147[1]) as u64));
        }
        let t150 = [vdupq_n_u32(x10); 2];
        let mut t151x = [0u32; 8];
        let mut t151y = [0u32; 8];
        for r in 0..2 { vst1q_u32(t151x.as_mut_ptr().add(4 * r), x8[r]); vst1q_u32(t151y.as_mut_ptr().add(4 * r), t150[r]); }
        for j in 0..8 { t151x[j] = (t151x[j] as u32).checked_rem(t151y[j] as u32).unwrap_or(0) as u32; }
        let t152: [uint32x4_t; 2] = [vld1q_u32(t151x.as_ptr().add(0)), vld1q_u32(t151x.as_ptr().add(4))];
        let t153 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t152[0]), vandq_u32(live[1], t152[1]))));
        let t154: [uint32x4_t; 2] = [vbslq_u32(live[0], t152[0], t153), vbslq_u32(live[1], t152[1], t153)];
        let mut t155: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t154[0]) as usize * 8) as *const u64).read_unaligned(); t155[0] = vsetq_lane_u32::<0>(v as u32, t155[0]); t155[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t155[2]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t154[0]) as usize * 8) as *const u64).read_unaligned(); t155[0] = vsetq_lane_u32::<1>(v as u32, t155[0]); t155[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t155[2]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t154[0]) as usize * 8) as *const u64).read_unaligned(); t155[0] = vsetq_lane_u32::<2>(v as u32, t155[0]); t155[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t155[2]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t154[0]) as usize * 8) as *const u64).read_unaligned(); t155[0] = vsetq_lane_u32::<3>(v as u32, t155[0]); t155[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t155[2]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t154[1]) as usize * 8) as *const u64).read_unaligned(); t155[1] = vsetq_lane_u32::<0>(v as u32, t155[1]); t155[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t155[3]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t154[1]) as usize * 8) as *const u64).read_unaligned(); t155[1] = vsetq_lane_u32::<1>(v as u32, t155[1]); t155[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t155[3]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t154[1]) as usize * 8) as *const u64).read_unaligned(); t155[1] = vsetq_lane_u32::<2>(v as u32, t155[1]); t155[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t155[3]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t154[1]) as usize * 8) as *const u64).read_unaligned(); t155[1] = vsetq_lane_u32::<3>(v as u32, t155[1]); t155[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t155[3]); }
        let t156 = (arg3 as *const u8).add(((gid0.wrapping_add((17u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t156 as *mut u32).add(0), uint32x4x2_t(t155[0], t155[2]));
            vst2q_u32((t156 as *mut u32).add(8), uint32x4x2_t(t155[1], t155[3]));
        } else {
            let mut t157w = 0u64;
            let t157 = core::hint::black_box(&raw mut t157w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t156 as *mut u64).add(0), t157).write_unaligned(((vgetq_lane_u32::<0>(t155[2]) as u64) << 32 | vgetq_lane_u32::<0>(t155[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t156 as *mut u64).add(1), t157).write_unaligned(((vgetq_lane_u32::<1>(t155[2]) as u64) << 32 | vgetq_lane_u32::<1>(t155[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t156 as *mut u64).add(2), t157).write_unaligned(((vgetq_lane_u32::<2>(t155[2]) as u64) << 32 | vgetq_lane_u32::<2>(t155[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t156 as *mut u64).add(3), t157).write_unaligned(((vgetq_lane_u32::<3>(t155[2]) as u64) << 32 | vgetq_lane_u32::<3>(t155[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t156 as *mut u64).add(4), t157).write_unaligned(((vgetq_lane_u32::<0>(t155[3]) as u64) << 32 | vgetq_lane_u32::<0>(t155[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t156 as *mut u64).add(5), t157).write_unaligned(((vgetq_lane_u32::<1>(t155[3]) as u64) << 32 | vgetq_lane_u32::<1>(t155[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t156 as *mut u64).add(6), t157).write_unaligned(((vgetq_lane_u32::<2>(t155[3]) as u64) << 32 | vgetq_lane_u32::<2>(t155[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t156 as *mut u64).add(7), t157).write_unaligned(((vgetq_lane_u32::<3>(t155[3]) as u64) << 32 | vgetq_lane_u32::<3>(t155[1]) as u64));
        }
        let t158 = b3_ll_p(l, lid0, live, x6, x8);
        let t159 = (arg3 as *const u8).add(((gid0.wrapping_add((18u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t159 as *mut u32).add(0), uint32x4x2_t(t158[0], t158[2]));
            vst2q_u32((t159 as *mut u32).add(8), uint32x4x2_t(t158[1], t158[3]));
        } else {
            let mut t160w = 0u64;
            let t160 = core::hint::black_box(&raw mut t160w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t159 as *mut u64).add(0), t160).write_unaligned(((vgetq_lane_u32::<0>(t158[2]) as u64) << 32 | vgetq_lane_u32::<0>(t158[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t159 as *mut u64).add(1), t160).write_unaligned(((vgetq_lane_u32::<1>(t158[2]) as u64) << 32 | vgetq_lane_u32::<1>(t158[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t159 as *mut u64).add(2), t160).write_unaligned(((vgetq_lane_u32::<2>(t158[2]) as u64) << 32 | vgetq_lane_u32::<2>(t158[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t159 as *mut u64).add(3), t160).write_unaligned(((vgetq_lane_u32::<3>(t158[2]) as u64) << 32 | vgetq_lane_u32::<3>(t158[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t159 as *mut u64).add(4), t160).write_unaligned(((vgetq_lane_u32::<0>(t158[3]) as u64) << 32 | vgetq_lane_u32::<0>(t158[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t159 as *mut u64).add(5), t160).write_unaligned(((vgetq_lane_u32::<1>(t158[3]) as u64) << 32 | vgetq_lane_u32::<1>(t158[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t159 as *mut u64).add(6), t160).write_unaligned(((vgetq_lane_u32::<2>(t158[3]) as u64) << 32 | vgetq_lane_u32::<2>(t158[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t159 as *mut u64).add(7), t160).write_unaligned(((vgetq_lane_u32::<3>(t158[3]) as u64) << 32 | vgetq_lane_u32::<3>(t158[1]) as u64));
        }
        x12 = x6;
        x13 = 0u32;
        let mut t165 = live;
        loop {
            // Lanes part: each its own rounds.
            let t167 = [vdupq_n_u32(7u32); 2];
            let t168: [uint32x4_t; 2] = [vandq_u32(x8[0], t167[0]), vandq_u32(x8[1], t167[1])];
            let t169 = [vdupq_n_u32(x13); 2];
            let t170: [uint32x4_t; 2] = [vcltq_u32(t169[0], t168[0]), vcltq_u32(t169[1], t168[1])];
            t165 = [vandq_u32(t165[0], t170[0]), vandq_u32(t165[1], t170[1])];
            if !(any(&t165)) { break; }
            let t172: u64 = 3u64;
            let t173 = [vdupq_n_u32((t172) as u32); 2];
            let t174: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x12[0]), vget_low_u32(t173[0]))), vreinterpretq_u32_u64(vmull_high_u32(x12[0], t173[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x12[1]), vget_low_u32(t173[1]))), vreinterpretq_u32_u64(vmull_high_u32(x12[1], t173[1])))];
            let t175: [uint32x4_t; 4] = [vmulq_u32(x12[0], t173[0]), vmulq_u32(x12[1], t173[1]), vmlaq_u32(t174[0], t173[0], x12[2]), vmlaq_u32(t174[1], t173[1], x12[3])];
            let t176: [uint32x4_t; 2] = [vaddq_u32(t175[0], x7[0]), vaddq_u32(t175[1], x7[1])];
            let t177: [uint32x4_t; 2] = [vcltq_u32(t176[0], t175[0]), vcltq_u32(t176[1], t175[1])];
            let t178: [uint32x4_t; 4] = [t176[0], t176[1], vsubq_u32(vaddq_u32(t175[2], x7[2]), t177[0]), vsubq_u32(vaddq_u32(t175[3], x7[3]), t177[1])];
            x12 = [vbslq_u32(t165[0], t178[0], x12[0]), vbslq_u32(t165[1], t178[1], x12[1]), vbslq_u32(t165[0], t178[2], x12[2]), vbslq_u32(t165[1], t178[3], x12[3])];
            let t179: [uint32x4_t; 2] = [vandq_u32(t165[0], live[0]), vandq_u32(t165[1], live[1])];
            let t180 = x13;
            x13 = (x13).wrapping_add(1u32);
        }
        let t181 = (arg3 as *const u8).add(((gid0.wrapping_add((19u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t181 as *mut u32).add(0), uint32x4x2_t(x12[0], x12[2]));
            vst2q_u32((t181 as *mut u32).add(8), uint32x4x2_t(x12[1], x12[3]));
        } else {
            let mut t182w = 0u64;
            let t182 = core::hint::black_box(&raw mut t182w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t181 as *mut u64).add(0), t182).write_unaligned(((vgetq_lane_u32::<0>(x12[2]) as u64) << 32 | vgetq_lane_u32::<0>(x12[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t181 as *mut u64).add(1), t182).write_unaligned(((vgetq_lane_u32::<1>(x12[2]) as u64) << 32 | vgetq_lane_u32::<1>(x12[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t181 as *mut u64).add(2), t182).write_unaligned(((vgetq_lane_u32::<2>(x12[2]) as u64) << 32 | vgetq_lane_u32::<2>(x12[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t181 as *mut u64).add(3), t182).write_unaligned(((vgetq_lane_u32::<3>(x12[2]) as u64) << 32 | vgetq_lane_u32::<3>(x12[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t181 as *mut u64).add(4), t182).write_unaligned(((vgetq_lane_u32::<0>(x12[3]) as u64) << 32 | vgetq_lane_u32::<0>(x12[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t181 as *mut u64).add(5), t182).write_unaligned(((vgetq_lane_u32::<1>(x12[3]) as u64) << 32 | vgetq_lane_u32::<1>(x12[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t181 as *mut u64).add(6), t182).write_unaligned(((vgetq_lane_u32::<2>(x12[3]) as u64) << 32 | vgetq_lane_u32::<2>(x12[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t181 as *mut u64).add(7), t182).write_unaligned(((vgetq_lane_u32::<3>(x12[3]) as u64) << 32 | vgetq_lane_u32::<3>(x12[1]) as u64));
        }
        let t183: u64 = a1;
        let t184 = [vdupq_n_u32((t183) as u32); 2];
        let t185 = [vdupq_n_u32(((t183) >> 32) as u32); 2];
        let t186: [uint32x4_t; 2] = [vaddq_u32(x6[0], t184[0]), vaddq_u32(x6[1], t184[1])];
        let t187: [uint32x4_t; 2] = [vcltq_u32(t186[0], x6[0]), vcltq_u32(t186[1], x6[1])];
        let t188: [uint32x4_t; 4] = [t186[0], t186[1], vsubq_u32(vaddq_u32(x6[2], t185[0]), t187[0]), vsubq_u32(vaddq_u32(x6[3], t185[1]), t187[1])];
        let t189: u64 = (a1).wrapping_shl((3u64) as u32);
        let t190 = [vdupq_n_u32((t189) as u32); 2];
        let t191 = [vdupq_n_u32(((t189) >> 32) as u32); 2];
        let t192: [uint32x4_t; 2] = [vaddq_u32(t188[0], t190[0]), vaddq_u32(t188[1], t190[1])];
        let t193: [uint32x4_t; 2] = [vcltq_u32(t192[0], t188[0]), vcltq_u32(t192[1], t188[1])];
        let t194: [uint32x4_t; 4] = [t192[0], t192[1], vsubq_u32(vaddq_u32(t188[2], t191[0]), t193[0]), vsubq_u32(vaddq_u32(t188[3], t191[1]), t193[1])];
        let t195 = (arg3 as *const u8).add(((gid0.wrapping_add((20u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t195 as *mut u32).add(0), uint32x4x2_t(t194[0], t194[2]));
            vst2q_u32((t195 as *mut u32).add(8), uint32x4x2_t(t194[1], t194[3]));
        } else {
            let mut t196w = 0u64;
            let t196 = core::hint::black_box(&raw mut t196w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t195 as *mut u64).add(0), t196).write_unaligned(((vgetq_lane_u32::<0>(t194[2]) as u64) << 32 | vgetq_lane_u32::<0>(t194[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t195 as *mut u64).add(1), t196).write_unaligned(((vgetq_lane_u32::<1>(t194[2]) as u64) << 32 | vgetq_lane_u32::<1>(t194[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t195 as *mut u64).add(2), t196).write_unaligned(((vgetq_lane_u32::<2>(t194[2]) as u64) << 32 | vgetq_lane_u32::<2>(t194[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t195 as *mut u64).add(3), t196).write_unaligned(((vgetq_lane_u32::<3>(t194[2]) as u64) << 32 | vgetq_lane_u32::<3>(t194[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t195 as *mut u64).add(4), t196).write_unaligned(((vgetq_lane_u32::<0>(t194[3]) as u64) << 32 | vgetq_lane_u32::<0>(t194[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t195 as *mut u64).add(5), t196).write_unaligned(((vgetq_lane_u32::<1>(t194[3]) as u64) << 32 | vgetq_lane_u32::<1>(t194[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t195 as *mut u64).add(6), t196).write_unaligned(((vgetq_lane_u32::<2>(t194[3]) as u64) << 32 | vgetq_lane_u32::<2>(t194[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t195 as *mut u64).add(7), t196).write_unaligned(((vgetq_lane_u32::<3>(t194[3]) as u64) << 32 | vgetq_lane_u32::<3>(t194[1]) as u64));
        }
        let t197: u64 = 4294967295u64;
        let t198 = [vdupq_n_u32((t197) as u32); 2];
        let t199: [uint32x4_t; 2] = [vandq_u32(x6[0], t198[0]), vandq_u32(x6[1], t198[1])];
        let t200: u64 = 4294967295u64;
        let t201 = [vdupq_n_u32((t200) as u32); 2];
        let t202: [uint32x4_t; 2] = [vandq_u32(x7[0], t201[0]), vandq_u32(x7[1], t201[1])];
        let t203: [uint32x4_t; 2] = [vceqq_u32(t199[0], t202[0]), vceqq_u32(t199[1], t202[1])];
        let mut t204: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t205: [uint32x4_t; 2] = [vandq_u32(live[0], t203[0]), vandq_u32(live[1], t203[1])];
        let t206: [uint32x4_t; 2] = [vbicq_u32(live[0], t203[0]), vbicq_u32(live[1], t203[1])];
        {
            let t207 = [vdupq_n_u32(1u32); 2];
            t204 = [vbslq_u32(t205[0], t207[0], t204[0]), vbslq_u32(t205[1], t207[1], t204[1])];
        }
        {
            let t208 = [vdupq_n_u32(2u32); 2];
            t204 = [vbslq_u32(t206[0], t208[0], t204[0]), vbslq_u32(t206[1], t208[1], t204[1])];
        }
        let t209: [uint32x4_t; 4] = [t204[0], t204[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        let t210 = (arg3 as *const u8).add(((gid0.wrapping_add((21u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t210 as *mut u32).add(0), uint32x4x2_t(t209[0], t209[2]));
            vst2q_u32((t210 as *mut u32).add(8), uint32x4x2_t(t209[1], t209[3]));
        } else {
            let mut t211w = 0u64;
            let t211 = core::hint::black_box(&raw mut t211w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t210 as *mut u64).add(0), t211).write_unaligned(((vgetq_lane_u32::<0>(t209[2]) as u64) << 32 | vgetq_lane_u32::<0>(t209[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t210 as *mut u64).add(1), t211).write_unaligned(((vgetq_lane_u32::<1>(t209[2]) as u64) << 32 | vgetq_lane_u32::<1>(t209[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t210 as *mut u64).add(2), t211).write_unaligned(((vgetq_lane_u32::<2>(t209[2]) as u64) << 32 | vgetq_lane_u32::<2>(t209[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t210 as *mut u64).add(3), t211).write_unaligned(((vgetq_lane_u32::<3>(t209[2]) as u64) << 32 | vgetq_lane_u32::<3>(t209[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t210 as *mut u64).add(4), t211).write_unaligned(((vgetq_lane_u32::<0>(t209[3]) as u64) << 32 | vgetq_lane_u32::<0>(t209[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t210 as *mut u64).add(5), t211).write_unaligned(((vgetq_lane_u32::<1>(t209[3]) as u64) << 32 | vgetq_lane_u32::<1>(t209[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t210 as *mut u64).add(6), t211).write_unaligned(((vgetq_lane_u32::<2>(t209[3]) as u64) << 32 | vgetq_lane_u32::<2>(t209[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t210 as *mut u64).add(7), t211).write_unaligned(((vgetq_lane_u32::<3>(t209[3]) as u64) << 32 | vgetq_lane_u32::<3>(t209[1]) as u64));
        }
        let t212: [uint32x4_t; 2] = [vtstq_u32(vorrq_u32(x6[0], x6[2]), vorrq_u32(x6[0], x6[2])), vtstq_u32(vorrq_u32(x6[1], x6[3]), vorrq_u32(x6[1], x6[3]))];
        let mut t213: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t214: [uint32x4_t; 2] = [vandq_u32(live[0], t212[0]), vandq_u32(live[1], t212[1])];
        let t215: [uint32x4_t; 2] = [vbicq_u32(live[0], t212[0]), vbicq_u32(live[1], t212[1])];
        {
            let t216 = [vdupq_n_u32(5u32); 2];
            t213 = [vbslq_u32(t214[0], t216[0], t213[0]), vbslq_u32(t214[1], t216[1], t213[1])];
        }
        {
            let t217 = [vdupq_n_u32(6u32); 2];
            t213 = [vbslq_u32(t215[0], t217[0], t213[0]), vbslq_u32(t215[1], t217[1], t213[1])];
        }
        let t218: [uint32x4_t; 4] = [t213[0], t213[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        let t219 = (arg3 as *const u8).add(((gid0.wrapping_add((22u32).wrapping_mul(x10))) as usize) * 8);
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
        let t221 = b4_ll_p(l, lid0, live, x6, x7);
        let t222 = (arg3 as *const u8).add(((gid0.wrapping_add((23u32).wrapping_mul(x10))) as usize) * 8);
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
        let t224: u32;
        if ((((a1) & (1u64))) != (0u64)) {
            t224 = 8u32;
        } else {
            t224 = 4u32;
        }
        x14 = t224;
        let t225 = b5_r0ns_p(l, lid0, live, arg0, gid0.wrapping_mul(x14), x14, 8u32);
        let t226 = (arg3 as *const u8).add(((gid0.wrapping_add((24u32).wrapping_mul(x10))) as usize) * 8);
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
        let t229 = (arg3 as *const u8).add(((gid0.wrapping_add((25u32).wrapping_mul(x10))) as usize) * 8);
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
        let t233: [uint32x4_t; 2] = [vandq_u32(x6[0], t232[0]), vandq_u32(x6[1], t232[1])];
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
                let t242 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x7);
                let t243 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x8);
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
                let t248 = b7_ll_p(l, lid0, t238, x7, x8);
                t240 = t248;
            }
            t237 = [vbslq_u32(t238[0], t240[0], t237[0]), vbslq_u32(t238[1], t240[1], t237[1]), vbslq_u32(t238[0], t240[2], t237[2]), vbslq_u32(t238[1], t240[3], t237[3])];
        }
        {
            t237 = [vbslq_u32(t239[0], x6[0], t237[0]), vbslq_u32(t239[1], x6[1], t237[1]), vbslq_u32(t239[0], x6[2], t237[2]), vbslq_u32(t239[1], x6[3], t237[3])];
        }
        let t249 = (arg3 as *const u8).add(((gid0.wrapping_add((26u32).wrapping_mul(x10))) as usize) * 8);
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
unsafe fn b0_r0r1r2r3sg_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, arg2: *const u8, arg3: *const u8, x4: *const u8) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x4 as *const u32).read_unaligned();
    let a1: u64 = (x4.wrapping_add(8u64 as usize) as *const u64).read_unaligned();
    let mut x6: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x7: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x8: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x9: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x10: u32 = 0;
    let mut x11: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x12: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x13: u32 = 0;
    let mut x14: u32 = 0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    if ((gid0) as u64 + 7) < ((a0) as u64) {
        let t251 = (arg0 as *const u8).add(((gid0) as usize) * 8);
        let t2530 = vld2q_u32((t251 as *const u32).add(0));
        let t2531 = vld2q_u32((t251 as *const u32).add(8));
        let t253: [uint32x4_t; 4] = [t2530.0, t2531.0, t2530.1, t2531.1];
        x6 = t253;
        let t254 = (arg1 as *const u8).add(((gid0) as usize) * 8);
        let t2560 = vld2q_u32((t254 as *const u32).add(0));
        let t2561 = vld2q_u32((t254 as *const u32).add(8));
        let t256: [uint32x4_t; 4] = [t2560.0, t2561.0, t2560.1, t2561.1];
        x7 = t256;
        let t257 = (arg2 as *const u32).add((gid0) as usize);
        let t258: [uint32x4_t; 2] = [vld1q_u32(t257.add(0)), vld1q_u32(t257.add(4))];
        x8 = t258;
        let t259 = [vdupq_n_u32(63u32); 2];
        let t260: [uint32x4_t; 2] = [vandq_u32(x8[0], t259[0]), vandq_u32(x8[1], t259[1])];
        x9 = t260;
        x10 = a0;
        let t261: [uint32x4_t; 2] = [vaddq_u32(x6[0], x7[0]), vaddq_u32(x6[1], x7[1])];
        let t262: [uint32x4_t; 2] = [vcltq_u32(t261[0], x6[0]), vcltq_u32(t261[1], x6[1])];
        let t263: [uint32x4_t; 4] = [t261[0], t261[1], vsubq_u32(vaddq_u32(x6[2], x7[2]), t262[0]), vsubq_u32(vaddq_u32(x6[3], x7[3]), t262[1])];
        let t264 = (arg3 as *const u8).add(((gid0.wrapping_add((0u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t264 as *mut u32).add(0), uint32x4x2_t(t263[0], t263[2]));
        vst2q_u32((t264 as *mut u32).add(8), uint32x4x2_t(t263[1], t263[3]));
        let t265: [uint32x4_t; 2] = [vcltq_u32(x6[0], x7[0]), vcltq_u32(x6[1], x7[1])];
        let t266: [uint32x4_t; 4] = [vsubq_u32(x6[0], x7[0]), vsubq_u32(x6[1], x7[1]), vaddq_u32(vsubq_u32(x6[2], x7[2]), t265[0]), vaddq_u32(vsubq_u32(x6[3], x7[3]), t265[1])];
        let t267 = (arg3 as *const u8).add(((gid0.wrapping_add((1u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t267 as *mut u32).add(0), uint32x4x2_t(t266[0], t266[2]));
        vst2q_u32((t267 as *mut u32).add(8), uint32x4x2_t(t266[1], t266[3]));
        let t268: [uint32x4_t; 2] = [x9[0], x9[1]];
        let t269: [uint32x4_t; 2] = [vandq_u32(t268[0], vdupq_n_u32(63)), vandq_u32(t268[1], vdupq_n_u32(63))];
        let t270: [uint32x4_t; 4] = [vshlq_u32(x6[0], vreinterpretq_s32_u32(t269[0])), vshlq_u32(x6[1], vreinterpretq_s32_u32(t269[1])), vorrq_u32(vshlq_u32(x6[2], vreinterpretq_s32_u32(t269[0])), vshlq_u32(x6[0], vsubq_s32(vreinterpretq_s32_u32(t269[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(x6[3], vreinterpretq_s32_u32(t269[1])), vshlq_u32(x6[1], vsubq_s32(vreinterpretq_s32_u32(t269[1]), vdupq_n_s32(32))))];
        let t271 = (arg3 as *const u8).add(((gid0.wrapping_add((2u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t271 as *mut u32).add(0), uint32x4x2_t(t270[0], t270[2]));
        vst2q_u32((t271 as *mut u32).add(8), uint32x4x2_t(t270[1], t270[3]));
        let t272: [uint32x4_t; 2] = [x9[0], x9[1]];
        let t273: [uint32x4_t; 2] = [vandq_u32(t272[0], vdupq_n_u32(63)), vandq_u32(t272[1], vdupq_n_u32(63))];
        let t274: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(x6[0], vnegq_s32(vreinterpretq_s32_u32(t273[0]))), vshlq_u32(x6[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t273[0])))), vorrq_u32(vshlq_u32(x6[1], vnegq_s32(vreinterpretq_s32_u32(t273[1]))), vshlq_u32(x6[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t273[1])))), vshlq_u32(x6[2], vnegq_s32(vreinterpretq_s32_u32(t273[0]))), vshlq_u32(x6[3], vnegq_s32(vreinterpretq_s32_u32(t273[1])))];
        let t275 = (arg3 as *const u8).add(((gid0.wrapping_add((3u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t275 as *mut u32).add(0), uint32x4x2_t(t274[0], t274[2]));
        vst2q_u32((t275 as *mut u32).add(8), uint32x4x2_t(t274[1], t274[3]));
        let t276: [uint32x4_t; 2] = [x9[0], x9[1]];
        let t277: [uint32x4_t; 2] = [vandq_u32(t276[0], vdupq_n_u32(63)), vandq_u32(t276[1], vdupq_n_u32(63))];
        let t278: [uint32x4_t; 2] = [vcgeq_s32(vreinterpretq_s32_u32(t277[0]), vdupq_n_s32(32)), vcgeq_s32(vreinterpretq_s32_u32(t277[1]), vdupq_n_s32(32))];
        let t279: [uint32x4_t; 4] = [vbslq_u32(t278[0], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t277[0])))), vorrq_u32(vshlq_u32(x6[0], vnegq_s32(vreinterpretq_s32_u32(t277[0]))), vshlq_u32(x6[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t277[0]))))), vbslq_u32(t278[1], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t277[1])))), vorrq_u32(vshlq_u32(x6[1], vnegq_s32(vreinterpretq_s32_u32(t277[1]))), vshlq_u32(x6[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t277[1]))))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vnegq_s32(vreinterpretq_s32_u32(t277[0])))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vnegq_s32(vreinterpretq_s32_u32(t277[1]))))];
        let t280 = (arg3 as *const u8).add(((gid0.wrapping_add((4u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t280 as *mut u32).add(0), uint32x4x2_t(t279[0], t279[2]));
        vst2q_u32((t280 as *mut u32).add(8), uint32x4x2_t(t279[1], t279[3]));
        let t281: [uint32x4_t; 4] = [vshlq_n_u32::<13>(x6[0]), vshlq_n_u32::<13>(x6[1]), vorrq_u32(vshlq_n_u32::<13>(x6[2]), vshrq_n_u32::<19>(x6[0])), vorrq_u32(vshlq_n_u32::<13>(x6[3]), vshrq_n_u32::<19>(x6[1]))];
        let t282: [uint32x4_t; 2] = [vshrq_n_u32::<19>(x7[2]), vshrq_n_u32::<19>(x7[3])];
        let t283: [uint32x4_t; 4] = [vorrq_u32(t281[0], t282[0]), vorrq_u32(t281[1], t282[1]), t281[2], t281[3]];
        let t284 = (arg3 as *const u8).add(((gid0.wrapping_add((5u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t284 as *mut u32).add(0), uint32x4x2_t(t283[0], t283[2]));
        vst2q_u32((t284 as *mut u32).add(8), uint32x4x2_t(t283[1], t283[3]));
        let t285: [uint32x4_t; 4] = [vdupq_n_u32(0), vdupq_n_u32(0), vshlq_n_u32::<8>(x6[0]), vshlq_n_u32::<8>(x6[1])];
        let t286 = (arg3 as *const u8).add(((gid0.wrapping_add((6u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t286 as *mut u32).add(0), uint32x4x2_t(t285[0], t285[2]));
        vst2q_u32((t286 as *mut u32).add(8), uint32x4x2_t(t285[1], t285[3]));
        let t287: [uint32x4_t; 2] = [vshrq_n_u32::<3>(x6[2]), vshrq_n_u32::<3>(x6[3])];
        let t288: [uint32x4_t; 4] = [t287[0], t287[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        let t289 = (arg3 as *const u8).add(((gid0.wrapping_add((7u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t289 as *mut u32).add(0), uint32x4x2_t(t288[0], t288[2]));
        vst2q_u32((t289 as *mut u32).add(8), uint32x4x2_t(t288[1], t288[3]));
        let t290 = vdupq_n_s32(((((40i64) as u32)) & 63) as i32);
        let t291: [uint32x4_t; 2] = [vcgeq_s32(t290, vdupq_n_s32(32)), vcgeq_s32(t290, vdupq_n_s32(32))];
        let t292: [uint32x4_t; 4] = [vbslq_u32(t291[0], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vsubq_s32(vdupq_n_s32(32), t290))), vorrq_u32(vshlq_u32(x6[0], vnegq_s32(t290)), vshlq_u32(x6[2], vsubq_s32(vdupq_n_s32(32), t290)))), vbslq_u32(t291[1], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vsubq_s32(vdupq_n_s32(32), t290))), vorrq_u32(vshlq_u32(x6[1], vnegq_s32(t290)), vshlq_u32(x6[3], vsubq_s32(vdupq_n_s32(32), t290)))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vnegq_s32(t290))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vnegq_s32(t290)))];
        let t293 = (arg3 as *const u8).add(((gid0.wrapping_add((8u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t293 as *mut u32).add(0), uint32x4x2_t(t292[0], t292[2]));
        vst2q_u32((t293 as *mut u32).add(8), uint32x4x2_t(t292[1], t292[3]));
        let t294: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x6[0]), vget_low_u32(x7[0]))), vreinterpretq_u32_u64(vmull_high_u32(x6[0], x7[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x6[1]), vget_low_u32(x7[1]))), vreinterpretq_u32_u64(vmull_high_u32(x6[1], x7[1])))];
        let t295: [uint32x4_t; 4] = [vmulq_u32(x6[0], x7[0]), vmulq_u32(x6[1], x7[1]), vmlaq_u32(vmlaq_u32(t294[0], x6[0], x7[2]), x7[0], x6[2]), vmlaq_u32(vmlaq_u32(t294[1], x6[1], x7[3]), x7[1], x6[3])];
        let t296 = (arg3 as *const u8).add(((gid0.wrapping_add((9u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t296 as *mut u32).add(0), uint32x4x2_t(t295[0], t295[2]));
        vst2q_u32((t296 as *mut u32).add(8), uint32x4x2_t(t295[1], t295[3]));
        let t297: u64 = 0u64;
        let t298 = [vdupq_n_u32((t297) as u32); 2];
        let t299: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x7[0], t298[0]), vceqq_u32(x7[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(x7[1], t298[1]), vceqq_u32(x7[3], vdupq_n_u32(0)))];
        let t300: [uint32x4_t; 2] = [vmvnq_u32(t299[0]), vmvnq_u32(t299[1])];
        let mut t301: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t302: [uint32x4_t; 2] = [vmvnq_u32(t300[0]), vmvnq_u32(t300[1])];
        {
            let (mut t303xl, mut t303xh, mut t303yl, mut t303yh) = ([0u32; 8], [0u32; 8], [0u32; 8], [0u32; 8]);
            vst1q_u32(t303xl.as_mut_ptr().add(0), x6[0]); vst1q_u32(t303xh.as_mut_ptr().add(0), x6[2]); vst1q_u32(t303yl.as_mut_ptr().add(0), x7[0]); vst1q_u32(t303yh.as_mut_ptr().add(0), x7[2]);
            vst1q_u32(t303xl.as_mut_ptr().add(4), x6[1]); vst1q_u32(t303xh.as_mut_ptr().add(4), x6[3]); vst1q_u32(t303yl.as_mut_ptr().add(4), x7[1]); vst1q_u32(t303yh.as_mut_ptr().add(4), x7[3]);
            for j in 0..8 { let q = (((t303xh[j] as u64) << 32 | t303xl[j] as u64) as u64).checked_div(((t303yh[j] as u64) << 32 | t303yl[j] as u64) as u64).unwrap_or(0) as u64; t303xl[j] = q as u32; t303xh[j] = (q >> 32) as u32; }
            let t304: [uint32x4_t; 4] = [vld1q_u32(t303xl.as_ptr().add(0)), vld1q_u32(t303xl.as_ptr().add(4)), vld1q_u32(t303xh.as_ptr().add(0)), vld1q_u32(t303xh.as_ptr().add(4))];
            t301 = [vbslq_u32(t300[0], t304[0], t301[0]), vbslq_u32(t300[1], t304[1], t301[1]), vbslq_u32(t300[0], t304[2], t301[2]), vbslq_u32(t300[1], t304[3], t301[3])];
        }
        {
            let t305: u64 = 7u64;
            let t306 = [vdupq_n_u32((t305) as u32); 2];
            let t307: [uint32x4_t; 4] = [t306[0], t306[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            t301 = [vbslq_u32(t302[0], t307[0], t301[0]), vbslq_u32(t302[1], t307[1], t301[1]), vbslq_u32(t302[0], t307[2], t301[2]), vbslq_u32(t302[1], t307[3], t301[3])];
        }
        let t308 = (arg3 as *const u8).add(((gid0.wrapping_add((10u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t308 as *mut u32).add(0), uint32x4x2_t(t301[0], t301[2]));
        vst2q_u32((t308 as *mut u32).add(8), uint32x4x2_t(t301[1], t301[3]));
        let t309: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x6[2], x7[2]), vandq_u32(vceqq_u32(x6[2], x7[2]), vcltq_u32(x6[0], x7[0]))), vorrq_u32(vcltq_u32(x6[3], x7[3]), vandq_u32(vceqq_u32(x6[3], x7[3]), vcltq_u32(x6[1], x7[1])))];
        let t310: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t309[0]), vshrq_n_u32::<31>(t309[1])];
        let t311: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x7[2], x6[2]), vandq_u32(vceqq_u32(x7[2], x6[2]), vcltq_u32(x7[0], x6[0]))), vorrq_u32(vcltq_u32(x7[3], x6[3]), vandq_u32(vceqq_u32(x7[3], x6[3]), vcltq_u32(x7[1], x6[1])))];
        let t312: [uint32x4_t; 2] = [vmvnq_u32(t311[0]), vmvnq_u32(t311[1])];
        let t313: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t312[0]), vshrq_n_u32::<31>(t312[1])];
        let t314: [uint32x4_t; 4] = [vshlq_n_u32::<1>(t313[0]), vshlq_n_u32::<1>(t313[1]), vshrq_n_u32::<31>(t313[0]), vshrq_n_u32::<31>(t313[1])];
        let t315: [uint32x4_t; 4] = [vorrq_u32(t310[0], t314[0]), vorrq_u32(t310[1], t314[1]), t314[2], t314[3]];
        let t316: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x6[0], x7[0]), vceqq_u32(x6[2], x7[2])), vandq_u32(vceqq_u32(x6[1], x7[1]), vceqq_u32(x6[3], x7[3]))];
        let t317: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t316[0]), vshrq_n_u32::<31>(t316[1])];
        let t318: [uint32x4_t; 4] = [vshlq_n_u32::<2>(t317[0]), vshlq_n_u32::<2>(t317[1]), vshrq_n_u32::<30>(t317[0]), vshrq_n_u32::<30>(t317[1])];
        let t319: [uint32x4_t; 4] = [vorrq_u32(t315[0], t318[0]), vorrq_u32(t315[1], t318[1]), vorrq_u32(t315[2], t318[2]), vorrq_u32(t315[3], t318[3])];
        let t320: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x6[0], x7[0]), vceqq_u32(x6[2], x7[2])), vandq_u32(vceqq_u32(x6[1], x7[1]), vceqq_u32(x6[3], x7[3]))];
        let t321: [uint32x4_t; 2] = [vmvnq_u32(t320[0]), vmvnq_u32(t320[1])];
        let t322: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t321[0]), vshrq_n_u32::<31>(t321[1])];
        let t323: [uint32x4_t; 4] = [vshlq_n_u32::<3>(t322[0]), vshlq_n_u32::<3>(t322[1]), vshrq_n_u32::<29>(t322[0]), vshrq_n_u32::<29>(t322[1])];
        let t324: [uint32x4_t; 4] = [vorrq_u32(t319[0], t323[0]), vorrq_u32(t319[1], t323[1]), vorrq_u32(t319[2], t323[2]), vorrq_u32(t319[3], t323[3])];
        let t325: [uint32x4_t; 2] = [vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x6[2]), vreinterpretq_s32_u32(x7[2])), vandq_u32(vceqq_u32(x6[2], x7[2]), vcltq_u32(x6[0], x7[0]))), vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x6[3]), vreinterpretq_s32_u32(x7[3])), vandq_u32(vceqq_u32(x6[3], x7[3]), vcltq_u32(x6[1], x7[1])))];
        let t326: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t325[0]), vshrq_n_u32::<31>(t325[1])];
        let t327: [uint32x4_t; 4] = [vshlq_n_u32::<4>(t326[0]), vshlq_n_u32::<4>(t326[1]), vshrq_n_u32::<28>(t326[0]), vshrq_n_u32::<28>(t326[1])];
        let t328: [uint32x4_t; 4] = [vorrq_u32(t324[0], t327[0]), vorrq_u32(t324[1], t327[1]), vorrq_u32(t324[2], t327[2]), vorrq_u32(t324[3], t327[3])];
        let t329: [uint32x4_t; 2] = [vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x6[2]), vreinterpretq_s32_u32(x7[2])), vandq_u32(vceqq_u32(x6[2], x7[2]), vcltq_u32(x6[0], x7[0]))), vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x6[3]), vreinterpretq_s32_u32(x7[3])), vandq_u32(vceqq_u32(x6[3], x7[3]), vcltq_u32(x6[1], x7[1])))];
        let t330: [uint32x4_t; 2] = [vmvnq_u32(t329[0]), vmvnq_u32(t329[1])];
        let t331: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t330[0]), vshrq_n_u32::<31>(t330[1])];
        let t332: [uint32x4_t; 4] = [vshlq_n_u32::<5>(t331[0]), vshlq_n_u32::<5>(t331[1]), vshrq_n_u32::<27>(t331[0]), vshrq_n_u32::<27>(t331[1])];
        let t333: [uint32x4_t; 4] = [vorrq_u32(t328[0], t332[0]), vorrq_u32(t328[1], t332[1]), vorrq_u32(t328[2], t332[2]), vorrq_u32(t328[3], t332[3])];
        let t334: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x7[2], x6[2]), vandq_u32(vceqq_u32(x7[2], x6[2]), vcltq_u32(x7[0], x6[0]))), vorrq_u32(vcltq_u32(x7[3], x6[3]), vandq_u32(vceqq_u32(x7[3], x6[3]), vcltq_u32(x7[1], x6[1])))];
        let t335: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t334[0]), vshrq_n_u32::<31>(t334[1])];
        let t336: [uint32x4_t; 4] = [vshlq_n_u32::<6>(t335[0]), vshlq_n_u32::<6>(t335[1]), vshrq_n_u32::<26>(t335[0]), vshrq_n_u32::<26>(t335[1])];
        let t337: [uint32x4_t; 4] = [vorrq_u32(t333[0], t336[0]), vorrq_u32(t333[1], t336[1]), vorrq_u32(t333[2], t336[2]), vorrq_u32(t333[3], t336[3])];
        let t338 = (arg3 as *const u8).add(((gid0.wrapping_add((11u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t338 as *mut u32).add(0), uint32x4x2_t(t337[0], t337[2]));
        vst2q_u32((t338 as *mut u32).add(8), uint32x4x2_t(t337[1], t337[3]));
        let t339: [uint32x4_t; 2] = [vaddq_u32(x8[0], x6[0]), vaddq_u32(x8[1], x6[1])];
        let t340: [uint32x4_t; 2] = [vcltq_u32(t339[0], x8[0]), vcltq_u32(t339[1], x8[1])];
        let t341: [uint32x4_t; 4] = [t339[0], t339[1], vsubq_u32(vaddq_u32(vdupq_n_u32(0), x6[2]), t340[0]), vsubq_u32(vaddq_u32(vdupq_n_u32(0), x6[3]), t340[1])];
        let t342 = (arg3 as *const u8).add(((gid0.wrapping_add((12u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t342 as *mut u32).add(0), uint32x4x2_t(t341[0], t341[2]));
        vst2q_u32((t342 as *mut u32).add(8), uint32x4x2_t(t341[1], t341[3]));
        let t343: [uint32x4_t; 2] = [vreinterpretq_u32_s32(vshrq_n_s32::<31>(vreinterpretq_s32_u32(x8[0]))), vreinterpretq_u32_s32(vshrq_n_s32::<31>(vreinterpretq_s32_u32(x8[1])))];
        let t344: [uint32x4_t; 4] = [x8[0], x8[1], t343[0], t343[1]];
        let t345 = (arg3 as *const u8).add(((gid0.wrapping_add((13u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t345 as *mut u32).add(0), uint32x4x2_t(t344[0], t344[2]));
        vst2q_u32((t345 as *mut u32).add(8), uint32x4x2_t(t344[1], t344[3]));
        let t346: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<3>(x6[0]), vshlq_n_u32::<29>(x6[2])), vorrq_u32(vshrq_n_u32::<3>(x6[1]), vshlq_n_u32::<29>(x6[3])), vshrq_n_u32::<3>(x6[2]), vshrq_n_u32::<3>(x6[3])];
        let t347: [uint32x4_t; 2] = [t346[0], t346[1]];
        let t348: [uint32x4_t; 4] = [t347[0], t347[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        let t349 = (arg3 as *const u8).add(((gid0.wrapping_add((14u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t349 as *mut u32).add(0), uint32x4x2_t(t348[0], t348[2]));
        vst2q_u32((t349 as *mut u32).add(8), uint32x4x2_t(t348[1], t348[3]));
        let t350: u64 = 1u64;
        let t351 = [vdupq_n_u32((t350) as u32); 2];
        let t352: [uint32x4_t; 2] = [vandq_u32(x6[0], t351[0]), vandq_u32(x6[1], t351[1])];
        let t353: [uint32x4_t; 2] = [vtstq_u32(t352[0], t352[0]), vtstq_u32(t352[1], t352[1])];
        {
            let t354: [uint32x4_t; 4] = [veorq_u32(x6[0], x7[0]), veorq_u32(x6[1], x7[1]), veorq_u32(x6[2], x7[2]), veorq_u32(x6[3], x7[3])];
            x11 = [vbslq_u32(t353[0], t354[0], x11[0]), vbslq_u32(t353[1], t354[1], x11[1]), vbslq_u32(t353[0], t354[2], x11[2]), vbslq_u32(t353[1], t354[3], x11[3])];
        }
        let t355: [uint32x4_t; 2] = [vmvnq_u32(t353[0]), vmvnq_u32(t353[1])];
        {
            let t356: [uint32x4_t; 2] = [vaddq_u32(x6[0], x8[0]), vaddq_u32(x6[1], x8[1])];
            let t357: [uint32x4_t; 2] = [vcltq_u32(t356[0], x6[0]), vcltq_u32(t356[1], x6[1])];
            let t358: [uint32x4_t; 4] = [t356[0], t356[1], vsubq_u32(vaddq_u32(x6[2], vdupq_n_u32(0)), t357[0]), vsubq_u32(vaddq_u32(x6[3], vdupq_n_u32(0)), t357[1])];
            x11 = [vbslq_u32(t355[0], t358[0], x11[0]), vbslq_u32(t355[1], t358[1], x11[1]), vbslq_u32(t355[0], t358[2], x11[2]), vbslq_u32(t355[1], t358[3], x11[3])];
        }
        let t359 = (arg3 as *const u8).add(((gid0.wrapping_add((15u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t359 as *mut u32).add(0), uint32x4x2_t(x11[0], x11[2]));
        vst2q_u32((t359 as *mut u32).add(8), uint32x4x2_t(x11[1], x11[3]));
        let t360 = b1_l_f(l, lid0, x6);
        let t361 = b2_l_f(l, lid0, x7);
        let t362: [uint32x4_t; 4] = [vshlq_n_u32::<8>(t361[0]), vshlq_n_u32::<8>(t361[1]), vshrq_n_u32::<24>(t361[0]), vshrq_n_u32::<24>(t361[1])];
        let t363: [uint32x4_t; 2] = [vaddq_u32(t360[0], t362[0]), vaddq_u32(t360[1], t362[1])];
        let t364: [uint32x4_t; 2] = [vcltq_u32(t363[0], t360[0]), vcltq_u32(t363[1], t360[1])];
        let t365: [uint32x4_t; 4] = [t363[0], t363[1], vsubq_u32(vaddq_u32(vdupq_n_u32(0), t362[2]), t364[0]), vsubq_u32(vaddq_u32(vdupq_n_u32(0), t362[3]), t364[1])];
        let t366 = (arg3 as *const u8).add(((gid0.wrapping_add((16u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t366 as *mut u32).add(0), uint32x4x2_t(t365[0], t365[2]));
        vst2q_u32((t366 as *mut u32).add(8), uint32x4x2_t(t365[1], t365[3]));
        let t367 = [vdupq_n_u32(x10); 2];
        let mut t368x = [0u32; 8];
        let mut t368y = [0u32; 8];
        for r in 0..2 { vst1q_u32(t368x.as_mut_ptr().add(4 * r), x8[r]); vst1q_u32(t368y.as_mut_ptr().add(4 * r), t367[r]); }
        for j in 0..8 { t368x[j] = (t368x[j] as u32).checked_rem(t368y[j] as u32).unwrap_or(0) as u32; }
        let t369: [uint32x4_t; 2] = [vld1q_u32(t368x.as_ptr().add(0)), vld1q_u32(t368x.as_ptr().add(4))];
        let mut t370: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t369[0]) as usize * 8) as *const u64).read_unaligned(); t370[0] = vsetq_lane_u32::<0>(v as u32, t370[0]); t370[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t370[2]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t369[0]) as usize * 8) as *const u64).read_unaligned(); t370[0] = vsetq_lane_u32::<1>(v as u32, t370[0]); t370[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t370[2]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t369[0]) as usize * 8) as *const u64).read_unaligned(); t370[0] = vsetq_lane_u32::<2>(v as u32, t370[0]); t370[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t370[2]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t369[0]) as usize * 8) as *const u64).read_unaligned(); t370[0] = vsetq_lane_u32::<3>(v as u32, t370[0]); t370[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t370[2]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t369[1]) as usize * 8) as *const u64).read_unaligned(); t370[1] = vsetq_lane_u32::<0>(v as u32, t370[1]); t370[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t370[3]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t369[1]) as usize * 8) as *const u64).read_unaligned(); t370[1] = vsetq_lane_u32::<1>(v as u32, t370[1]); t370[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t370[3]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t369[1]) as usize * 8) as *const u64).read_unaligned(); t370[1] = vsetq_lane_u32::<2>(v as u32, t370[1]); t370[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t370[3]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t369[1]) as usize * 8) as *const u64).read_unaligned(); t370[1] = vsetq_lane_u32::<3>(v as u32, t370[1]); t370[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t370[3]); }
        let t371 = (arg3 as *const u8).add(((gid0.wrapping_add((17u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t371 as *mut u32).add(0), uint32x4x2_t(t370[0], t370[2]));
        vst2q_u32((t371 as *mut u32).add(8), uint32x4x2_t(t370[1], t370[3]));
        let t372 = b3_ll_f(l, lid0, x6, x8);
        let t373 = (arg3 as *const u8).add(((gid0.wrapping_add((18u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t373 as *mut u32).add(0), uint32x4x2_t(t372[0], t372[2]));
        vst2q_u32((t373 as *mut u32).add(8), uint32x4x2_t(t372[1], t372[3]));
        x12 = x6;
        x13 = 0u32;
        let mut t382 = live;
        loop {
            // Lanes part: each its own rounds.
            let t384 = [vdupq_n_u32(7u32); 2];
            let t385: [uint32x4_t; 2] = [vandq_u32(x8[0], t384[0]), vandq_u32(x8[1], t384[1])];
            let t386 = [vdupq_n_u32(x13); 2];
            let t387: [uint32x4_t; 2] = [vcltq_u32(t386[0], t385[0]), vcltq_u32(t386[1], t385[1])];
            t382 = [vandq_u32(t382[0], t387[0]), vandq_u32(t382[1], t387[1])];
            if !(any(&t382)) { break; }
            let t389: u64 = 3u64;
            let t390 = [vdupq_n_u32((t389) as u32); 2];
            let t391: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x12[0]), vget_low_u32(t390[0]))), vreinterpretq_u32_u64(vmull_high_u32(x12[0], t390[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x12[1]), vget_low_u32(t390[1]))), vreinterpretq_u32_u64(vmull_high_u32(x12[1], t390[1])))];
            let t392: [uint32x4_t; 4] = [vmulq_u32(x12[0], t390[0]), vmulq_u32(x12[1], t390[1]), vmlaq_u32(t391[0], t390[0], x12[2]), vmlaq_u32(t391[1], t390[1], x12[3])];
            let t393: [uint32x4_t; 2] = [vaddq_u32(t392[0], x7[0]), vaddq_u32(t392[1], x7[1])];
            let t394: [uint32x4_t; 2] = [vcltq_u32(t393[0], t392[0]), vcltq_u32(t393[1], t392[1])];
            let t395: [uint32x4_t; 4] = [t393[0], t393[1], vsubq_u32(vaddq_u32(t392[2], x7[2]), t394[0]), vsubq_u32(vaddq_u32(t392[3], x7[3]), t394[1])];
            x12 = [vbslq_u32(t382[0], t395[0], x12[0]), vbslq_u32(t382[1], t395[1], x12[1]), vbslq_u32(t382[0], t395[2], x12[2]), vbslq_u32(t382[1], t395[3], x12[3])];
            let t396: [uint32x4_t; 2] = [vandq_u32(t382[0], live[0]), vandq_u32(t382[1], live[1])];
            let t397 = x13;
            x13 = (x13).wrapping_add(1u32);
        }
        let t398 = (arg3 as *const u8).add(((gid0.wrapping_add((19u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t398 as *mut u32).add(0), uint32x4x2_t(x12[0], x12[2]));
        vst2q_u32((t398 as *mut u32).add(8), uint32x4x2_t(x12[1], x12[3]));
        let t399: u64 = a1;
        let t400 = [vdupq_n_u32((t399) as u32); 2];
        let t401 = [vdupq_n_u32(((t399) >> 32) as u32); 2];
        let t402: [uint32x4_t; 2] = [vaddq_u32(x6[0], t400[0]), vaddq_u32(x6[1], t400[1])];
        let t403: [uint32x4_t; 2] = [vcltq_u32(t402[0], x6[0]), vcltq_u32(t402[1], x6[1])];
        let t404: [uint32x4_t; 4] = [t402[0], t402[1], vsubq_u32(vaddq_u32(x6[2], t401[0]), t403[0]), vsubq_u32(vaddq_u32(x6[3], t401[1]), t403[1])];
        let t405: u64 = (a1).wrapping_shl((3u64) as u32);
        let t406 = [vdupq_n_u32((t405) as u32); 2];
        let t407 = [vdupq_n_u32(((t405) >> 32) as u32); 2];
        let t408: [uint32x4_t; 2] = [vaddq_u32(t404[0], t406[0]), vaddq_u32(t404[1], t406[1])];
        let t409: [uint32x4_t; 2] = [vcltq_u32(t408[0], t404[0]), vcltq_u32(t408[1], t404[1])];
        let t410: [uint32x4_t; 4] = [t408[0], t408[1], vsubq_u32(vaddq_u32(t404[2], t407[0]), t409[0]), vsubq_u32(vaddq_u32(t404[3], t407[1]), t409[1])];
        let t411 = (arg3 as *const u8).add(((gid0.wrapping_add((20u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t411 as *mut u32).add(0), uint32x4x2_t(t410[0], t410[2]));
        vst2q_u32((t411 as *mut u32).add(8), uint32x4x2_t(t410[1], t410[3]));
        let t412: u64 = 4294967295u64;
        let t413 = [vdupq_n_u32((t412) as u32); 2];
        let t414: [uint32x4_t; 2] = [vandq_u32(x6[0], t413[0]), vandq_u32(x6[1], t413[1])];
        let t415: u64 = 4294967295u64;
        let t416 = [vdupq_n_u32((t415) as u32); 2];
        let t417: [uint32x4_t; 2] = [vandq_u32(x7[0], t416[0]), vandq_u32(x7[1], t416[1])];
        let t418: [uint32x4_t; 2] = [vceqq_u32(t414[0], t417[0]), vceqq_u32(t414[1], t417[1])];
        let mut t419: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t420: [uint32x4_t; 2] = [vmvnq_u32(t418[0]), vmvnq_u32(t418[1])];
        {
            let t421 = [vdupq_n_u32(1u32); 2];
            t419 = [vbslq_u32(t418[0], t421[0], t419[0]), vbslq_u32(t418[1], t421[1], t419[1])];
        }
        {
            let t422 = [vdupq_n_u32(2u32); 2];
            t419 = [vbslq_u32(t420[0], t422[0], t419[0]), vbslq_u32(t420[1], t422[1], t419[1])];
        }
        let t423: [uint32x4_t; 4] = [t419[0], t419[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        let t424 = (arg3 as *const u8).add(((gid0.wrapping_add((21u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t424 as *mut u32).add(0), uint32x4x2_t(t423[0], t423[2]));
        vst2q_u32((t424 as *mut u32).add(8), uint32x4x2_t(t423[1], t423[3]));
        let t425: [uint32x4_t; 2] = [vtstq_u32(vorrq_u32(x6[0], x6[2]), vorrq_u32(x6[0], x6[2])), vtstq_u32(vorrq_u32(x6[1], x6[3]), vorrq_u32(x6[1], x6[3]))];
        let mut t426: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t427: [uint32x4_t; 2] = [vmvnq_u32(t425[0]), vmvnq_u32(t425[1])];
        {
            let t428 = [vdupq_n_u32(5u32); 2];
            t426 = [vbslq_u32(t425[0], t428[0], t426[0]), vbslq_u32(t425[1], t428[1], t426[1])];
        }
        {
            let t429 = [vdupq_n_u32(6u32); 2];
            t426 = [vbslq_u32(t427[0], t429[0], t426[0]), vbslq_u32(t427[1], t429[1], t426[1])];
        }
        let t430: [uint32x4_t; 4] = [t426[0], t426[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        let t431 = (arg3 as *const u8).add(((gid0.wrapping_add((22u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t431 as *mut u32).add(0), uint32x4x2_t(t430[0], t430[2]));
        vst2q_u32((t431 as *mut u32).add(8), uint32x4x2_t(t430[1], t430[3]));
        let t432 = b4_ll_f(l, lid0, x6, x7);
        let t433 = (arg3 as *const u8).add(((gid0.wrapping_add((23u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t433 as *mut u32).add(0), uint32x4x2_t(t432[0], t432[2]));
        vst2q_u32((t433 as *mut u32).add(8), uint32x4x2_t(t432[1], t432[3]));
        let t434: u32;
        if ((((a1) & (1u64))) != (0u64)) {
            t434 = 8u32;
        } else {
            t434 = 4u32;
        }
        x14 = t434;
        let t435 = b5_r0ns_f_v0(l, lid0, arg0, gid0.wrapping_mul(x14), x14, 8u32);
        let t436 = (arg3 as *const u8).add(((gid0.wrapping_add((24u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t436 as *mut u32).add(0), uint32x4x2_t(t435[0], t435[2]));
        vst2q_u32((t436 as *mut u32).add(8), uint32x4x2_t(t435[1], t435[3]));
        let t437 = if (gid0.wrapping_add(3u32) & 255u32) <= 248u32 { b6_u8_f(l, lid0, gid0.wrapping_add(3u32)) } else { b6_i_f(l, lid0, gid0.wrapping_add(3u32)) };
        let t438 = (arg3 as *const u8).add(((gid0.wrapping_add((25u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t438 as *mut u32).add(0), uint32x4x2_t(t437[0], t437[2]));
        vst2q_u32((t438 as *mut u32).add(8), uint32x4x2_t(t437[1], t437[3]));
        let t439: u64 = 3u64;
        let t440 = [vdupq_n_u32((t439) as u32); 2];
        let t441: [uint32x4_t; 2] = [vandq_u32(x6[0], t440[0]), vandq_u32(x6[1], t440[1])];
        let t442: u64 = 0u64;
        let t443 = [vdupq_n_u32((t442) as u32); 2];
        let t444: [uint32x4_t; 2] = [vceqq_u32(t441[0], t443[0]), vceqq_u32(t441[1], t443[1])];
        let mut t445: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t446: [uint32x4_t; 2] = [vmvnq_u32(t444[0]), vmvnq_u32(t444[1])];
        if any(&t444) {
            let t447: [uint32x4_t; 4];
            let t448 = bits(&t444);
            if t448.count_ones() <= 2 {
                let t449 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x7);
                let t450 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x8);
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
                let t455 = b7_ll_p(l, lid0, t444, x7, x8);
                t447 = t455;
            }
            t445 = [vbslq_u32(t444[0], t447[0], t445[0]), vbslq_u32(t444[1], t447[1], t445[1]), vbslq_u32(t444[0], t447[2], t445[2]), vbslq_u32(t444[1], t447[3], t445[3])];
        }
        {
            t445 = [vbslq_u32(t446[0], x6[0], t445[0]), vbslq_u32(t446[1], x6[1], t445[1]), vbslq_u32(t446[0], x6[2], t445[2]), vbslq_u32(t446[1], x6[3], t445[3])];
        }
        let t456 = (arg3 as *const u8).add(((gid0.wrapping_add((26u32).wrapping_mul(x10))) as usize) * 8);
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
            x6 = t462;
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
            x7 = t468;
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
            x8 = t473;
            let t479 = [vdupq_n_u32(63u32); 2];
            let t480: [uint32x4_t; 2] = [vandq_u32(x8[0], t479[0]), vandq_u32(x8[1], t479[1])];
            x9 = t480;
            x10 = a0;
            let t481: [uint32x4_t; 2] = [vaddq_u32(x6[0], x7[0]), vaddq_u32(x6[1], x7[1])];
            let t482: [uint32x4_t; 2] = [vcltq_u32(t481[0], x6[0]), vcltq_u32(t481[1], x6[1])];
            let t483: [uint32x4_t; 4] = [t481[0], t481[1], vsubq_u32(vaddq_u32(x6[2], x7[2]), t482[0]), vsubq_u32(vaddq_u32(x6[3], x7[3]), t482[1])];
            let t484 = (arg3 as *const u8).add(((gid0.wrapping_add((0u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t484 as *mut u32).add(0), uint32x4x2_t(t483[0], t483[2]));
                vst2q_u32((t484 as *mut u32).add(8), uint32x4x2_t(t483[1], t483[3]));
            } else {
                let mut t485w = 0u64;
                let t485 = core::hint::black_box(&raw mut t485w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t484 as *mut u64).add(0), t485).write_unaligned(((vgetq_lane_u32::<0>(t483[2]) as u64) << 32 | vgetq_lane_u32::<0>(t483[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t484 as *mut u64).add(1), t485).write_unaligned(((vgetq_lane_u32::<1>(t483[2]) as u64) << 32 | vgetq_lane_u32::<1>(t483[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t484 as *mut u64).add(2), t485).write_unaligned(((vgetq_lane_u32::<2>(t483[2]) as u64) << 32 | vgetq_lane_u32::<2>(t483[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t484 as *mut u64).add(3), t485).write_unaligned(((vgetq_lane_u32::<3>(t483[2]) as u64) << 32 | vgetq_lane_u32::<3>(t483[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t484 as *mut u64).add(4), t485).write_unaligned(((vgetq_lane_u32::<0>(t483[3]) as u64) << 32 | vgetq_lane_u32::<0>(t483[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t484 as *mut u64).add(5), t485).write_unaligned(((vgetq_lane_u32::<1>(t483[3]) as u64) << 32 | vgetq_lane_u32::<1>(t483[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t484 as *mut u64).add(6), t485).write_unaligned(((vgetq_lane_u32::<2>(t483[3]) as u64) << 32 | vgetq_lane_u32::<2>(t483[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t484 as *mut u64).add(7), t485).write_unaligned(((vgetq_lane_u32::<3>(t483[3]) as u64) << 32 | vgetq_lane_u32::<3>(t483[1]) as u64));
            }
            let t486: [uint32x4_t; 2] = [vcltq_u32(x6[0], x7[0]), vcltq_u32(x6[1], x7[1])];
            let t487: [uint32x4_t; 4] = [vsubq_u32(x6[0], x7[0]), vsubq_u32(x6[1], x7[1]), vaddq_u32(vsubq_u32(x6[2], x7[2]), t486[0]), vaddq_u32(vsubq_u32(x6[3], x7[3]), t486[1])];
            let t488 = (arg3 as *const u8).add(((gid0.wrapping_add((1u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t488 as *mut u32).add(0), uint32x4x2_t(t487[0], t487[2]));
                vst2q_u32((t488 as *mut u32).add(8), uint32x4x2_t(t487[1], t487[3]));
            } else {
                let mut t489w = 0u64;
                let t489 = core::hint::black_box(&raw mut t489w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t488 as *mut u64).add(0), t489).write_unaligned(((vgetq_lane_u32::<0>(t487[2]) as u64) << 32 | vgetq_lane_u32::<0>(t487[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t488 as *mut u64).add(1), t489).write_unaligned(((vgetq_lane_u32::<1>(t487[2]) as u64) << 32 | vgetq_lane_u32::<1>(t487[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t488 as *mut u64).add(2), t489).write_unaligned(((vgetq_lane_u32::<2>(t487[2]) as u64) << 32 | vgetq_lane_u32::<2>(t487[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t488 as *mut u64).add(3), t489).write_unaligned(((vgetq_lane_u32::<3>(t487[2]) as u64) << 32 | vgetq_lane_u32::<3>(t487[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t488 as *mut u64).add(4), t489).write_unaligned(((vgetq_lane_u32::<0>(t487[3]) as u64) << 32 | vgetq_lane_u32::<0>(t487[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t488 as *mut u64).add(5), t489).write_unaligned(((vgetq_lane_u32::<1>(t487[3]) as u64) << 32 | vgetq_lane_u32::<1>(t487[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t488 as *mut u64).add(6), t489).write_unaligned(((vgetq_lane_u32::<2>(t487[3]) as u64) << 32 | vgetq_lane_u32::<2>(t487[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t488 as *mut u64).add(7), t489).write_unaligned(((vgetq_lane_u32::<3>(t487[3]) as u64) << 32 | vgetq_lane_u32::<3>(t487[1]) as u64));
            }
            let t490: [uint32x4_t; 2] = [x9[0], x9[1]];
            let t491: [uint32x4_t; 2] = [vandq_u32(t490[0], vdupq_n_u32(63)), vandq_u32(t490[1], vdupq_n_u32(63))];
            let t492: [uint32x4_t; 4] = [vshlq_u32(x6[0], vreinterpretq_s32_u32(t491[0])), vshlq_u32(x6[1], vreinterpretq_s32_u32(t491[1])), vorrq_u32(vshlq_u32(x6[2], vreinterpretq_s32_u32(t491[0])), vshlq_u32(x6[0], vsubq_s32(vreinterpretq_s32_u32(t491[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(x6[3], vreinterpretq_s32_u32(t491[1])), vshlq_u32(x6[1], vsubq_s32(vreinterpretq_s32_u32(t491[1]), vdupq_n_s32(32))))];
            let t493 = (arg3 as *const u8).add(((gid0.wrapping_add((2u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t493 as *mut u32).add(0), uint32x4x2_t(t492[0], t492[2]));
                vst2q_u32((t493 as *mut u32).add(8), uint32x4x2_t(t492[1], t492[3]));
            } else {
                let mut t494w = 0u64;
                let t494 = core::hint::black_box(&raw mut t494w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t493 as *mut u64).add(0), t494).write_unaligned(((vgetq_lane_u32::<0>(t492[2]) as u64) << 32 | vgetq_lane_u32::<0>(t492[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t493 as *mut u64).add(1), t494).write_unaligned(((vgetq_lane_u32::<1>(t492[2]) as u64) << 32 | vgetq_lane_u32::<1>(t492[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t493 as *mut u64).add(2), t494).write_unaligned(((vgetq_lane_u32::<2>(t492[2]) as u64) << 32 | vgetq_lane_u32::<2>(t492[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t493 as *mut u64).add(3), t494).write_unaligned(((vgetq_lane_u32::<3>(t492[2]) as u64) << 32 | vgetq_lane_u32::<3>(t492[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t493 as *mut u64).add(4), t494).write_unaligned(((vgetq_lane_u32::<0>(t492[3]) as u64) << 32 | vgetq_lane_u32::<0>(t492[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t493 as *mut u64).add(5), t494).write_unaligned(((vgetq_lane_u32::<1>(t492[3]) as u64) << 32 | vgetq_lane_u32::<1>(t492[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t493 as *mut u64).add(6), t494).write_unaligned(((vgetq_lane_u32::<2>(t492[3]) as u64) << 32 | vgetq_lane_u32::<2>(t492[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t493 as *mut u64).add(7), t494).write_unaligned(((vgetq_lane_u32::<3>(t492[3]) as u64) << 32 | vgetq_lane_u32::<3>(t492[1]) as u64));
            }
            let t495: [uint32x4_t; 2] = [x9[0], x9[1]];
            let t496: [uint32x4_t; 2] = [vandq_u32(t495[0], vdupq_n_u32(63)), vandq_u32(t495[1], vdupq_n_u32(63))];
            let t497: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(x6[0], vnegq_s32(vreinterpretq_s32_u32(t496[0]))), vshlq_u32(x6[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t496[0])))), vorrq_u32(vshlq_u32(x6[1], vnegq_s32(vreinterpretq_s32_u32(t496[1]))), vshlq_u32(x6[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t496[1])))), vshlq_u32(x6[2], vnegq_s32(vreinterpretq_s32_u32(t496[0]))), vshlq_u32(x6[3], vnegq_s32(vreinterpretq_s32_u32(t496[1])))];
            let t498 = (arg3 as *const u8).add(((gid0.wrapping_add((3u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t498 as *mut u32).add(0), uint32x4x2_t(t497[0], t497[2]));
                vst2q_u32((t498 as *mut u32).add(8), uint32x4x2_t(t497[1], t497[3]));
            } else {
                let mut t499w = 0u64;
                let t499 = core::hint::black_box(&raw mut t499w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t498 as *mut u64).add(0), t499).write_unaligned(((vgetq_lane_u32::<0>(t497[2]) as u64) << 32 | vgetq_lane_u32::<0>(t497[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t498 as *mut u64).add(1), t499).write_unaligned(((vgetq_lane_u32::<1>(t497[2]) as u64) << 32 | vgetq_lane_u32::<1>(t497[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t498 as *mut u64).add(2), t499).write_unaligned(((vgetq_lane_u32::<2>(t497[2]) as u64) << 32 | vgetq_lane_u32::<2>(t497[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t498 as *mut u64).add(3), t499).write_unaligned(((vgetq_lane_u32::<3>(t497[2]) as u64) << 32 | vgetq_lane_u32::<3>(t497[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t498 as *mut u64).add(4), t499).write_unaligned(((vgetq_lane_u32::<0>(t497[3]) as u64) << 32 | vgetq_lane_u32::<0>(t497[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t498 as *mut u64).add(5), t499).write_unaligned(((vgetq_lane_u32::<1>(t497[3]) as u64) << 32 | vgetq_lane_u32::<1>(t497[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t498 as *mut u64).add(6), t499).write_unaligned(((vgetq_lane_u32::<2>(t497[3]) as u64) << 32 | vgetq_lane_u32::<2>(t497[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t498 as *mut u64).add(7), t499).write_unaligned(((vgetq_lane_u32::<3>(t497[3]) as u64) << 32 | vgetq_lane_u32::<3>(t497[1]) as u64));
            }
            let t500: [uint32x4_t; 2] = [x9[0], x9[1]];
            let t501: [uint32x4_t; 2] = [vandq_u32(t500[0], vdupq_n_u32(63)), vandq_u32(t500[1], vdupq_n_u32(63))];
            let t502: [uint32x4_t; 2] = [vcgeq_s32(vreinterpretq_s32_u32(t501[0]), vdupq_n_s32(32)), vcgeq_s32(vreinterpretq_s32_u32(t501[1]), vdupq_n_s32(32))];
            let t503: [uint32x4_t; 4] = [vbslq_u32(t502[0], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t501[0])))), vorrq_u32(vshlq_u32(x6[0], vnegq_s32(vreinterpretq_s32_u32(t501[0]))), vshlq_u32(x6[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t501[0]))))), vbslq_u32(t502[1], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t501[1])))), vorrq_u32(vshlq_u32(x6[1], vnegq_s32(vreinterpretq_s32_u32(t501[1]))), vshlq_u32(x6[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t501[1]))))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vnegq_s32(vreinterpretq_s32_u32(t501[0])))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vnegq_s32(vreinterpretq_s32_u32(t501[1]))))];
            let t504 = (arg3 as *const u8).add(((gid0.wrapping_add((4u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t504 as *mut u32).add(0), uint32x4x2_t(t503[0], t503[2]));
                vst2q_u32((t504 as *mut u32).add(8), uint32x4x2_t(t503[1], t503[3]));
            } else {
                let mut t505w = 0u64;
                let t505 = core::hint::black_box(&raw mut t505w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t504 as *mut u64).add(0), t505).write_unaligned(((vgetq_lane_u32::<0>(t503[2]) as u64) << 32 | vgetq_lane_u32::<0>(t503[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t504 as *mut u64).add(1), t505).write_unaligned(((vgetq_lane_u32::<1>(t503[2]) as u64) << 32 | vgetq_lane_u32::<1>(t503[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t504 as *mut u64).add(2), t505).write_unaligned(((vgetq_lane_u32::<2>(t503[2]) as u64) << 32 | vgetq_lane_u32::<2>(t503[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t504 as *mut u64).add(3), t505).write_unaligned(((vgetq_lane_u32::<3>(t503[2]) as u64) << 32 | vgetq_lane_u32::<3>(t503[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t504 as *mut u64).add(4), t505).write_unaligned(((vgetq_lane_u32::<0>(t503[3]) as u64) << 32 | vgetq_lane_u32::<0>(t503[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t504 as *mut u64).add(5), t505).write_unaligned(((vgetq_lane_u32::<1>(t503[3]) as u64) << 32 | vgetq_lane_u32::<1>(t503[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t504 as *mut u64).add(6), t505).write_unaligned(((vgetq_lane_u32::<2>(t503[3]) as u64) << 32 | vgetq_lane_u32::<2>(t503[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t504 as *mut u64).add(7), t505).write_unaligned(((vgetq_lane_u32::<3>(t503[3]) as u64) << 32 | vgetq_lane_u32::<3>(t503[1]) as u64));
            }
            let t506: [uint32x4_t; 4] = [vshlq_n_u32::<13>(x6[0]), vshlq_n_u32::<13>(x6[1]), vorrq_u32(vshlq_n_u32::<13>(x6[2]), vshrq_n_u32::<19>(x6[0])), vorrq_u32(vshlq_n_u32::<13>(x6[3]), vshrq_n_u32::<19>(x6[1]))];
            let t507: [uint32x4_t; 2] = [vshrq_n_u32::<19>(x7[2]), vshrq_n_u32::<19>(x7[3])];
            let t508: [uint32x4_t; 4] = [vorrq_u32(t506[0], t507[0]), vorrq_u32(t506[1], t507[1]), t506[2], t506[3]];
            let t509 = (arg3 as *const u8).add(((gid0.wrapping_add((5u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t509 as *mut u32).add(0), uint32x4x2_t(t508[0], t508[2]));
                vst2q_u32((t509 as *mut u32).add(8), uint32x4x2_t(t508[1], t508[3]));
            } else {
                let mut t510w = 0u64;
                let t510 = core::hint::black_box(&raw mut t510w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t509 as *mut u64).add(0), t510).write_unaligned(((vgetq_lane_u32::<0>(t508[2]) as u64) << 32 | vgetq_lane_u32::<0>(t508[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t509 as *mut u64).add(1), t510).write_unaligned(((vgetq_lane_u32::<1>(t508[2]) as u64) << 32 | vgetq_lane_u32::<1>(t508[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t509 as *mut u64).add(2), t510).write_unaligned(((vgetq_lane_u32::<2>(t508[2]) as u64) << 32 | vgetq_lane_u32::<2>(t508[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t509 as *mut u64).add(3), t510).write_unaligned(((vgetq_lane_u32::<3>(t508[2]) as u64) << 32 | vgetq_lane_u32::<3>(t508[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t509 as *mut u64).add(4), t510).write_unaligned(((vgetq_lane_u32::<0>(t508[3]) as u64) << 32 | vgetq_lane_u32::<0>(t508[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t509 as *mut u64).add(5), t510).write_unaligned(((vgetq_lane_u32::<1>(t508[3]) as u64) << 32 | vgetq_lane_u32::<1>(t508[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t509 as *mut u64).add(6), t510).write_unaligned(((vgetq_lane_u32::<2>(t508[3]) as u64) << 32 | vgetq_lane_u32::<2>(t508[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t509 as *mut u64).add(7), t510).write_unaligned(((vgetq_lane_u32::<3>(t508[3]) as u64) << 32 | vgetq_lane_u32::<3>(t508[1]) as u64));
            }
            let t511: [uint32x4_t; 4] = [vdupq_n_u32(0), vdupq_n_u32(0), vshlq_n_u32::<8>(x6[0]), vshlq_n_u32::<8>(x6[1])];
            let t512 = (arg3 as *const u8).add(((gid0.wrapping_add((6u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t512 as *mut u32).add(0), uint32x4x2_t(t511[0], t511[2]));
                vst2q_u32((t512 as *mut u32).add(8), uint32x4x2_t(t511[1], t511[3]));
            } else {
                let mut t513w = 0u64;
                let t513 = core::hint::black_box(&raw mut t513w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t512 as *mut u64).add(0), t513).write_unaligned(((vgetq_lane_u32::<0>(t511[2]) as u64) << 32 | vgetq_lane_u32::<0>(t511[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t512 as *mut u64).add(1), t513).write_unaligned(((vgetq_lane_u32::<1>(t511[2]) as u64) << 32 | vgetq_lane_u32::<1>(t511[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t512 as *mut u64).add(2), t513).write_unaligned(((vgetq_lane_u32::<2>(t511[2]) as u64) << 32 | vgetq_lane_u32::<2>(t511[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t512 as *mut u64).add(3), t513).write_unaligned(((vgetq_lane_u32::<3>(t511[2]) as u64) << 32 | vgetq_lane_u32::<3>(t511[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t512 as *mut u64).add(4), t513).write_unaligned(((vgetq_lane_u32::<0>(t511[3]) as u64) << 32 | vgetq_lane_u32::<0>(t511[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t512 as *mut u64).add(5), t513).write_unaligned(((vgetq_lane_u32::<1>(t511[3]) as u64) << 32 | vgetq_lane_u32::<1>(t511[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t512 as *mut u64).add(6), t513).write_unaligned(((vgetq_lane_u32::<2>(t511[3]) as u64) << 32 | vgetq_lane_u32::<2>(t511[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t512 as *mut u64).add(7), t513).write_unaligned(((vgetq_lane_u32::<3>(t511[3]) as u64) << 32 | vgetq_lane_u32::<3>(t511[1]) as u64));
            }
            let t514: [uint32x4_t; 2] = [vshrq_n_u32::<3>(x6[2]), vshrq_n_u32::<3>(x6[3])];
            let t515: [uint32x4_t; 4] = [t514[0], t514[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            let t516 = (arg3 as *const u8).add(((gid0.wrapping_add((7u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t516 as *mut u32).add(0), uint32x4x2_t(t515[0], t515[2]));
                vst2q_u32((t516 as *mut u32).add(8), uint32x4x2_t(t515[1], t515[3]));
            } else {
                let mut t517w = 0u64;
                let t517 = core::hint::black_box(&raw mut t517w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t516 as *mut u64).add(0), t517).write_unaligned(((vgetq_lane_u32::<0>(t515[2]) as u64) << 32 | vgetq_lane_u32::<0>(t515[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t516 as *mut u64).add(1), t517).write_unaligned(((vgetq_lane_u32::<1>(t515[2]) as u64) << 32 | vgetq_lane_u32::<1>(t515[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t516 as *mut u64).add(2), t517).write_unaligned(((vgetq_lane_u32::<2>(t515[2]) as u64) << 32 | vgetq_lane_u32::<2>(t515[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t516 as *mut u64).add(3), t517).write_unaligned(((vgetq_lane_u32::<3>(t515[2]) as u64) << 32 | vgetq_lane_u32::<3>(t515[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t516 as *mut u64).add(4), t517).write_unaligned(((vgetq_lane_u32::<0>(t515[3]) as u64) << 32 | vgetq_lane_u32::<0>(t515[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t516 as *mut u64).add(5), t517).write_unaligned(((vgetq_lane_u32::<1>(t515[3]) as u64) << 32 | vgetq_lane_u32::<1>(t515[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t516 as *mut u64).add(6), t517).write_unaligned(((vgetq_lane_u32::<2>(t515[3]) as u64) << 32 | vgetq_lane_u32::<2>(t515[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t516 as *mut u64).add(7), t517).write_unaligned(((vgetq_lane_u32::<3>(t515[3]) as u64) << 32 | vgetq_lane_u32::<3>(t515[1]) as u64));
            }
            let t518 = vdupq_n_s32(((((40i64) as u32)) & 63) as i32);
            let t519: [uint32x4_t; 2] = [vcgeq_s32(t518, vdupq_n_s32(32)), vcgeq_s32(t518, vdupq_n_s32(32))];
            let t520: [uint32x4_t; 4] = [vbslq_u32(t519[0], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vsubq_s32(vdupq_n_s32(32), t518))), vorrq_u32(vshlq_u32(x6[0], vnegq_s32(t518)), vshlq_u32(x6[2], vsubq_s32(vdupq_n_s32(32), t518)))), vbslq_u32(t519[1], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vsubq_s32(vdupq_n_s32(32), t518))), vorrq_u32(vshlq_u32(x6[1], vnegq_s32(t518)), vshlq_u32(x6[3], vsubq_s32(vdupq_n_s32(32), t518)))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vnegq_s32(t518))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vnegq_s32(t518)))];
            let t521 = (arg3 as *const u8).add(((gid0.wrapping_add((8u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t521 as *mut u32).add(0), uint32x4x2_t(t520[0], t520[2]));
                vst2q_u32((t521 as *mut u32).add(8), uint32x4x2_t(t520[1], t520[3]));
            } else {
                let mut t522w = 0u64;
                let t522 = core::hint::black_box(&raw mut t522w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t521 as *mut u64).add(0), t522).write_unaligned(((vgetq_lane_u32::<0>(t520[2]) as u64) << 32 | vgetq_lane_u32::<0>(t520[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t521 as *mut u64).add(1), t522).write_unaligned(((vgetq_lane_u32::<1>(t520[2]) as u64) << 32 | vgetq_lane_u32::<1>(t520[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t521 as *mut u64).add(2), t522).write_unaligned(((vgetq_lane_u32::<2>(t520[2]) as u64) << 32 | vgetq_lane_u32::<2>(t520[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t521 as *mut u64).add(3), t522).write_unaligned(((vgetq_lane_u32::<3>(t520[2]) as u64) << 32 | vgetq_lane_u32::<3>(t520[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t521 as *mut u64).add(4), t522).write_unaligned(((vgetq_lane_u32::<0>(t520[3]) as u64) << 32 | vgetq_lane_u32::<0>(t520[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t521 as *mut u64).add(5), t522).write_unaligned(((vgetq_lane_u32::<1>(t520[3]) as u64) << 32 | vgetq_lane_u32::<1>(t520[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t521 as *mut u64).add(6), t522).write_unaligned(((vgetq_lane_u32::<2>(t520[3]) as u64) << 32 | vgetq_lane_u32::<2>(t520[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t521 as *mut u64).add(7), t522).write_unaligned(((vgetq_lane_u32::<3>(t520[3]) as u64) << 32 | vgetq_lane_u32::<3>(t520[1]) as u64));
            }
            let t523: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x6[0]), vget_low_u32(x7[0]))), vreinterpretq_u32_u64(vmull_high_u32(x6[0], x7[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x6[1]), vget_low_u32(x7[1]))), vreinterpretq_u32_u64(vmull_high_u32(x6[1], x7[1])))];
            let t524: [uint32x4_t; 4] = [vmulq_u32(x6[0], x7[0]), vmulq_u32(x6[1], x7[1]), vmlaq_u32(vmlaq_u32(t523[0], x6[0], x7[2]), x7[0], x6[2]), vmlaq_u32(vmlaq_u32(t523[1], x6[1], x7[3]), x7[1], x6[3])];
            let t525 = (arg3 as *const u8).add(((gid0.wrapping_add((9u32).wrapping_mul(x10))) as usize) * 8);
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
            let t527: u64 = 0u64;
            let t528 = [vdupq_n_u32((t527) as u32); 2];
            let t529: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x7[0], t528[0]), vceqq_u32(x7[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(x7[1], t528[1]), vceqq_u32(x7[3], vdupq_n_u32(0)))];
            let t530: [uint32x4_t; 2] = [vmvnq_u32(t529[0]), vmvnq_u32(t529[1])];
            let mut t531: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            let t532: [uint32x4_t; 2] = [vandq_u32(live[0], t530[0]), vandq_u32(live[1], t530[1])];
            let t533: [uint32x4_t; 2] = [vbicq_u32(live[0], t530[0]), vbicq_u32(live[1], t530[1])];
            {
                let (mut t534xl, mut t534xh, mut t534yl, mut t534yh) = ([0u32; 8], [0u32; 8], [0u32; 8], [0u32; 8]);
                vst1q_u32(t534xl.as_mut_ptr().add(0), x6[0]); vst1q_u32(t534xh.as_mut_ptr().add(0), x6[2]); vst1q_u32(t534yl.as_mut_ptr().add(0), x7[0]); vst1q_u32(t534yh.as_mut_ptr().add(0), x7[2]);
                vst1q_u32(t534xl.as_mut_ptr().add(4), x6[1]); vst1q_u32(t534xh.as_mut_ptr().add(4), x6[3]); vst1q_u32(t534yl.as_mut_ptr().add(4), x7[1]); vst1q_u32(t534yh.as_mut_ptr().add(4), x7[3]);
                for j in 0..8 { let q = (((t534xh[j] as u64) << 32 | t534xl[j] as u64) as u64).checked_div(((t534yh[j] as u64) << 32 | t534yl[j] as u64) as u64).unwrap_or(0) as u64; t534xl[j] = q as u32; t534xh[j] = (q >> 32) as u32; }
                let t535: [uint32x4_t; 4] = [vld1q_u32(t534xl.as_ptr().add(0)), vld1q_u32(t534xl.as_ptr().add(4)), vld1q_u32(t534xh.as_ptr().add(0)), vld1q_u32(t534xh.as_ptr().add(4))];
                t531 = [vbslq_u32(t532[0], t535[0], t531[0]), vbslq_u32(t532[1], t535[1], t531[1]), vbslq_u32(t532[0], t535[2], t531[2]), vbslq_u32(t532[1], t535[3], t531[3])];
            }
            {
                let t536: u64 = 7u64;
                let t537 = [vdupq_n_u32((t536) as u32); 2];
                let t538: [uint32x4_t; 4] = [t537[0], t537[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                t531 = [vbslq_u32(t533[0], t538[0], t531[0]), vbslq_u32(t533[1], t538[1], t531[1]), vbslq_u32(t533[0], t538[2], t531[2]), vbslq_u32(t533[1], t538[3], t531[3])];
            }
            let t539 = (arg3 as *const u8).add(((gid0.wrapping_add((10u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t539 as *mut u32).add(0), uint32x4x2_t(t531[0], t531[2]));
                vst2q_u32((t539 as *mut u32).add(8), uint32x4x2_t(t531[1], t531[3]));
            } else {
                let mut t540w = 0u64;
                let t540 = core::hint::black_box(&raw mut t540w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t539 as *mut u64).add(0), t540).write_unaligned(((vgetq_lane_u32::<0>(t531[2]) as u64) << 32 | vgetq_lane_u32::<0>(t531[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t539 as *mut u64).add(1), t540).write_unaligned(((vgetq_lane_u32::<1>(t531[2]) as u64) << 32 | vgetq_lane_u32::<1>(t531[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t539 as *mut u64).add(2), t540).write_unaligned(((vgetq_lane_u32::<2>(t531[2]) as u64) << 32 | vgetq_lane_u32::<2>(t531[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t539 as *mut u64).add(3), t540).write_unaligned(((vgetq_lane_u32::<3>(t531[2]) as u64) << 32 | vgetq_lane_u32::<3>(t531[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t539 as *mut u64).add(4), t540).write_unaligned(((vgetq_lane_u32::<0>(t531[3]) as u64) << 32 | vgetq_lane_u32::<0>(t531[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t539 as *mut u64).add(5), t540).write_unaligned(((vgetq_lane_u32::<1>(t531[3]) as u64) << 32 | vgetq_lane_u32::<1>(t531[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t539 as *mut u64).add(6), t540).write_unaligned(((vgetq_lane_u32::<2>(t531[3]) as u64) << 32 | vgetq_lane_u32::<2>(t531[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t539 as *mut u64).add(7), t540).write_unaligned(((vgetq_lane_u32::<3>(t531[3]) as u64) << 32 | vgetq_lane_u32::<3>(t531[1]) as u64));
            }
            let t541: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x6[2], x7[2]), vandq_u32(vceqq_u32(x6[2], x7[2]), vcltq_u32(x6[0], x7[0]))), vorrq_u32(vcltq_u32(x6[3], x7[3]), vandq_u32(vceqq_u32(x6[3], x7[3]), vcltq_u32(x6[1], x7[1])))];
            let t542: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t541[0]), vshrq_n_u32::<31>(t541[1])];
            let t543: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x7[2], x6[2]), vandq_u32(vceqq_u32(x7[2], x6[2]), vcltq_u32(x7[0], x6[0]))), vorrq_u32(vcltq_u32(x7[3], x6[3]), vandq_u32(vceqq_u32(x7[3], x6[3]), vcltq_u32(x7[1], x6[1])))];
            let t544: [uint32x4_t; 2] = [vmvnq_u32(t543[0]), vmvnq_u32(t543[1])];
            let t545: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t544[0]), vshrq_n_u32::<31>(t544[1])];
            let t546: [uint32x4_t; 4] = [vshlq_n_u32::<1>(t545[0]), vshlq_n_u32::<1>(t545[1]), vshrq_n_u32::<31>(t545[0]), vshrq_n_u32::<31>(t545[1])];
            let t547: [uint32x4_t; 4] = [vorrq_u32(t542[0], t546[0]), vorrq_u32(t542[1], t546[1]), t546[2], t546[3]];
            let t548: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x6[0], x7[0]), vceqq_u32(x6[2], x7[2])), vandq_u32(vceqq_u32(x6[1], x7[1]), vceqq_u32(x6[3], x7[3]))];
            let t549: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t548[0]), vshrq_n_u32::<31>(t548[1])];
            let t550: [uint32x4_t; 4] = [vshlq_n_u32::<2>(t549[0]), vshlq_n_u32::<2>(t549[1]), vshrq_n_u32::<30>(t549[0]), vshrq_n_u32::<30>(t549[1])];
            let t551: [uint32x4_t; 4] = [vorrq_u32(t547[0], t550[0]), vorrq_u32(t547[1], t550[1]), vorrq_u32(t547[2], t550[2]), vorrq_u32(t547[3], t550[3])];
            let t552: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x6[0], x7[0]), vceqq_u32(x6[2], x7[2])), vandq_u32(vceqq_u32(x6[1], x7[1]), vceqq_u32(x6[3], x7[3]))];
            let t553: [uint32x4_t; 2] = [vmvnq_u32(t552[0]), vmvnq_u32(t552[1])];
            let t554: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t553[0]), vshrq_n_u32::<31>(t553[1])];
            let t555: [uint32x4_t; 4] = [vshlq_n_u32::<3>(t554[0]), vshlq_n_u32::<3>(t554[1]), vshrq_n_u32::<29>(t554[0]), vshrq_n_u32::<29>(t554[1])];
            let t556: [uint32x4_t; 4] = [vorrq_u32(t551[0], t555[0]), vorrq_u32(t551[1], t555[1]), vorrq_u32(t551[2], t555[2]), vorrq_u32(t551[3], t555[3])];
            let t557: [uint32x4_t; 2] = [vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x6[2]), vreinterpretq_s32_u32(x7[2])), vandq_u32(vceqq_u32(x6[2], x7[2]), vcltq_u32(x6[0], x7[0]))), vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x6[3]), vreinterpretq_s32_u32(x7[3])), vandq_u32(vceqq_u32(x6[3], x7[3]), vcltq_u32(x6[1], x7[1])))];
            let t558: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t557[0]), vshrq_n_u32::<31>(t557[1])];
            let t559: [uint32x4_t; 4] = [vshlq_n_u32::<4>(t558[0]), vshlq_n_u32::<4>(t558[1]), vshrq_n_u32::<28>(t558[0]), vshrq_n_u32::<28>(t558[1])];
            let t560: [uint32x4_t; 4] = [vorrq_u32(t556[0], t559[0]), vorrq_u32(t556[1], t559[1]), vorrq_u32(t556[2], t559[2]), vorrq_u32(t556[3], t559[3])];
            let t561: [uint32x4_t; 2] = [vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x6[2]), vreinterpretq_s32_u32(x7[2])), vandq_u32(vceqq_u32(x6[2], x7[2]), vcltq_u32(x6[0], x7[0]))), vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x6[3]), vreinterpretq_s32_u32(x7[3])), vandq_u32(vceqq_u32(x6[3], x7[3]), vcltq_u32(x6[1], x7[1])))];
            let t562: [uint32x4_t; 2] = [vmvnq_u32(t561[0]), vmvnq_u32(t561[1])];
            let t563: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t562[0]), vshrq_n_u32::<31>(t562[1])];
            let t564: [uint32x4_t; 4] = [vshlq_n_u32::<5>(t563[0]), vshlq_n_u32::<5>(t563[1]), vshrq_n_u32::<27>(t563[0]), vshrq_n_u32::<27>(t563[1])];
            let t565: [uint32x4_t; 4] = [vorrq_u32(t560[0], t564[0]), vorrq_u32(t560[1], t564[1]), vorrq_u32(t560[2], t564[2]), vorrq_u32(t560[3], t564[3])];
            let t566: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x7[2], x6[2]), vandq_u32(vceqq_u32(x7[2], x6[2]), vcltq_u32(x7[0], x6[0]))), vorrq_u32(vcltq_u32(x7[3], x6[3]), vandq_u32(vceqq_u32(x7[3], x6[3]), vcltq_u32(x7[1], x6[1])))];
            let t567: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t566[0]), vshrq_n_u32::<31>(t566[1])];
            let t568: [uint32x4_t; 4] = [vshlq_n_u32::<6>(t567[0]), vshlq_n_u32::<6>(t567[1]), vshrq_n_u32::<26>(t567[0]), vshrq_n_u32::<26>(t567[1])];
            let t569: [uint32x4_t; 4] = [vorrq_u32(t565[0], t568[0]), vorrq_u32(t565[1], t568[1]), vorrq_u32(t565[2], t568[2]), vorrq_u32(t565[3], t568[3])];
            let t570 = (arg3 as *const u8).add(((gid0.wrapping_add((11u32).wrapping_mul(x10))) as usize) * 8);
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
            let t572: [uint32x4_t; 2] = [vaddq_u32(x8[0], x6[0]), vaddq_u32(x8[1], x6[1])];
            let t573: [uint32x4_t; 2] = [vcltq_u32(t572[0], x8[0]), vcltq_u32(t572[1], x8[1])];
            let t574: [uint32x4_t; 4] = [t572[0], t572[1], vsubq_u32(vaddq_u32(vdupq_n_u32(0), x6[2]), t573[0]), vsubq_u32(vaddq_u32(vdupq_n_u32(0), x6[3]), t573[1])];
            let t575 = (arg3 as *const u8).add(((gid0.wrapping_add((12u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t575 as *mut u32).add(0), uint32x4x2_t(t574[0], t574[2]));
                vst2q_u32((t575 as *mut u32).add(8), uint32x4x2_t(t574[1], t574[3]));
            } else {
                let mut t576w = 0u64;
                let t576 = core::hint::black_box(&raw mut t576w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t575 as *mut u64).add(0), t576).write_unaligned(((vgetq_lane_u32::<0>(t574[2]) as u64) << 32 | vgetq_lane_u32::<0>(t574[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t575 as *mut u64).add(1), t576).write_unaligned(((vgetq_lane_u32::<1>(t574[2]) as u64) << 32 | vgetq_lane_u32::<1>(t574[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t575 as *mut u64).add(2), t576).write_unaligned(((vgetq_lane_u32::<2>(t574[2]) as u64) << 32 | vgetq_lane_u32::<2>(t574[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t575 as *mut u64).add(3), t576).write_unaligned(((vgetq_lane_u32::<3>(t574[2]) as u64) << 32 | vgetq_lane_u32::<3>(t574[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t575 as *mut u64).add(4), t576).write_unaligned(((vgetq_lane_u32::<0>(t574[3]) as u64) << 32 | vgetq_lane_u32::<0>(t574[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t575 as *mut u64).add(5), t576).write_unaligned(((vgetq_lane_u32::<1>(t574[3]) as u64) << 32 | vgetq_lane_u32::<1>(t574[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t575 as *mut u64).add(6), t576).write_unaligned(((vgetq_lane_u32::<2>(t574[3]) as u64) << 32 | vgetq_lane_u32::<2>(t574[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t575 as *mut u64).add(7), t576).write_unaligned(((vgetq_lane_u32::<3>(t574[3]) as u64) << 32 | vgetq_lane_u32::<3>(t574[1]) as u64));
            }
            let t577: [uint32x4_t; 2] = [vreinterpretq_u32_s32(vshrq_n_s32::<31>(vreinterpretq_s32_u32(x8[0]))), vreinterpretq_u32_s32(vshrq_n_s32::<31>(vreinterpretq_s32_u32(x8[1])))];
            let t578: [uint32x4_t; 4] = [x8[0], x8[1], t577[0], t577[1]];
            let t579 = (arg3 as *const u8).add(((gid0.wrapping_add((13u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t579 as *mut u32).add(0), uint32x4x2_t(t578[0], t578[2]));
                vst2q_u32((t579 as *mut u32).add(8), uint32x4x2_t(t578[1], t578[3]));
            } else {
                let mut t580w = 0u64;
                let t580 = core::hint::black_box(&raw mut t580w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t579 as *mut u64).add(0), t580).write_unaligned(((vgetq_lane_u32::<0>(t578[2]) as u64) << 32 | vgetq_lane_u32::<0>(t578[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t579 as *mut u64).add(1), t580).write_unaligned(((vgetq_lane_u32::<1>(t578[2]) as u64) << 32 | vgetq_lane_u32::<1>(t578[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t579 as *mut u64).add(2), t580).write_unaligned(((vgetq_lane_u32::<2>(t578[2]) as u64) << 32 | vgetq_lane_u32::<2>(t578[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t579 as *mut u64).add(3), t580).write_unaligned(((vgetq_lane_u32::<3>(t578[2]) as u64) << 32 | vgetq_lane_u32::<3>(t578[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t579 as *mut u64).add(4), t580).write_unaligned(((vgetq_lane_u32::<0>(t578[3]) as u64) << 32 | vgetq_lane_u32::<0>(t578[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t579 as *mut u64).add(5), t580).write_unaligned(((vgetq_lane_u32::<1>(t578[3]) as u64) << 32 | vgetq_lane_u32::<1>(t578[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t579 as *mut u64).add(6), t580).write_unaligned(((vgetq_lane_u32::<2>(t578[3]) as u64) << 32 | vgetq_lane_u32::<2>(t578[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t579 as *mut u64).add(7), t580).write_unaligned(((vgetq_lane_u32::<3>(t578[3]) as u64) << 32 | vgetq_lane_u32::<3>(t578[1]) as u64));
            }
            let t581: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<3>(x6[0]), vshlq_n_u32::<29>(x6[2])), vorrq_u32(vshrq_n_u32::<3>(x6[1]), vshlq_n_u32::<29>(x6[3])), vshrq_n_u32::<3>(x6[2]), vshrq_n_u32::<3>(x6[3])];
            let t582: [uint32x4_t; 2] = [t581[0], t581[1]];
            let t583: [uint32x4_t; 4] = [t582[0], t582[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            let t584 = (arg3 as *const u8).add(((gid0.wrapping_add((14u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t584 as *mut u32).add(0), uint32x4x2_t(t583[0], t583[2]));
                vst2q_u32((t584 as *mut u32).add(8), uint32x4x2_t(t583[1], t583[3]));
            } else {
                let mut t585w = 0u64;
                let t585 = core::hint::black_box(&raw mut t585w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t584 as *mut u64).add(0), t585).write_unaligned(((vgetq_lane_u32::<0>(t583[2]) as u64) << 32 | vgetq_lane_u32::<0>(t583[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t584 as *mut u64).add(1), t585).write_unaligned(((vgetq_lane_u32::<1>(t583[2]) as u64) << 32 | vgetq_lane_u32::<1>(t583[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t584 as *mut u64).add(2), t585).write_unaligned(((vgetq_lane_u32::<2>(t583[2]) as u64) << 32 | vgetq_lane_u32::<2>(t583[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t584 as *mut u64).add(3), t585).write_unaligned(((vgetq_lane_u32::<3>(t583[2]) as u64) << 32 | vgetq_lane_u32::<3>(t583[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t584 as *mut u64).add(4), t585).write_unaligned(((vgetq_lane_u32::<0>(t583[3]) as u64) << 32 | vgetq_lane_u32::<0>(t583[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t584 as *mut u64).add(5), t585).write_unaligned(((vgetq_lane_u32::<1>(t583[3]) as u64) << 32 | vgetq_lane_u32::<1>(t583[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t584 as *mut u64).add(6), t585).write_unaligned(((vgetq_lane_u32::<2>(t583[3]) as u64) << 32 | vgetq_lane_u32::<2>(t583[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t584 as *mut u64).add(7), t585).write_unaligned(((vgetq_lane_u32::<3>(t583[3]) as u64) << 32 | vgetq_lane_u32::<3>(t583[1]) as u64));
            }
            let t586: u64 = 1u64;
            let t587 = [vdupq_n_u32((t586) as u32); 2];
            let t588: [uint32x4_t; 2] = [vandq_u32(x6[0], t587[0]), vandq_u32(x6[1], t587[1])];
            let t589: [uint32x4_t; 2] = [vtstq_u32(t588[0], t588[0]), vtstq_u32(t588[1], t588[1])];
            let t590: [uint32x4_t; 2] = [vandq_u32(live[0], t589[0]), vandq_u32(live[1], t589[1])];
            {
                let t591: [uint32x4_t; 4] = [veorq_u32(x6[0], x7[0]), veorq_u32(x6[1], x7[1]), veorq_u32(x6[2], x7[2]), veorq_u32(x6[3], x7[3])];
                x11 = [vbslq_u32(t590[0], t591[0], x11[0]), vbslq_u32(t590[1], t591[1], x11[1]), vbslq_u32(t590[0], t591[2], x11[2]), vbslq_u32(t590[1], t591[3], x11[3])];
            }
            let t592: [uint32x4_t; 2] = [vbicq_u32(live[0], t589[0]), vbicq_u32(live[1], t589[1])];
            {
                let t593: [uint32x4_t; 2] = [vaddq_u32(x6[0], x8[0]), vaddq_u32(x6[1], x8[1])];
                let t594: [uint32x4_t; 2] = [vcltq_u32(t593[0], x6[0]), vcltq_u32(t593[1], x6[1])];
                let t595: [uint32x4_t; 4] = [t593[0], t593[1], vsubq_u32(vaddq_u32(x6[2], vdupq_n_u32(0)), t594[0]), vsubq_u32(vaddq_u32(x6[3], vdupq_n_u32(0)), t594[1])];
                x11 = [vbslq_u32(t592[0], t595[0], x11[0]), vbslq_u32(t592[1], t595[1], x11[1]), vbslq_u32(t592[0], t595[2], x11[2]), vbslq_u32(t592[1], t595[3], x11[3])];
            }
            let t596 = (arg3 as *const u8).add(((gid0.wrapping_add((15u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t596 as *mut u32).add(0), uint32x4x2_t(x11[0], x11[2]));
                vst2q_u32((t596 as *mut u32).add(8), uint32x4x2_t(x11[1], x11[3]));
            } else {
                let mut t597w = 0u64;
                let t597 = core::hint::black_box(&raw mut t597w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t596 as *mut u64).add(0), t597).write_unaligned(((vgetq_lane_u32::<0>(x11[2]) as u64) << 32 | vgetq_lane_u32::<0>(x11[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t596 as *mut u64).add(1), t597).write_unaligned(((vgetq_lane_u32::<1>(x11[2]) as u64) << 32 | vgetq_lane_u32::<1>(x11[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t596 as *mut u64).add(2), t597).write_unaligned(((vgetq_lane_u32::<2>(x11[2]) as u64) << 32 | vgetq_lane_u32::<2>(x11[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t596 as *mut u64).add(3), t597).write_unaligned(((vgetq_lane_u32::<3>(x11[2]) as u64) << 32 | vgetq_lane_u32::<3>(x11[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t596 as *mut u64).add(4), t597).write_unaligned(((vgetq_lane_u32::<0>(x11[3]) as u64) << 32 | vgetq_lane_u32::<0>(x11[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t596 as *mut u64).add(5), t597).write_unaligned(((vgetq_lane_u32::<1>(x11[3]) as u64) << 32 | vgetq_lane_u32::<1>(x11[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t596 as *mut u64).add(6), t597).write_unaligned(((vgetq_lane_u32::<2>(x11[3]) as u64) << 32 | vgetq_lane_u32::<2>(x11[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t596 as *mut u64).add(7), t597).write_unaligned(((vgetq_lane_u32::<3>(x11[3]) as u64) << 32 | vgetq_lane_u32::<3>(x11[1]) as u64));
            }
            let t598 = b1_l_p(l, lid0, live, x6);
            let t599 = b2_l_p(l, lid0, live, x7);
            let t600: [uint32x4_t; 4] = [vshlq_n_u32::<8>(t599[0]), vshlq_n_u32::<8>(t599[1]), vshrq_n_u32::<24>(t599[0]), vshrq_n_u32::<24>(t599[1])];
            let t601: [uint32x4_t; 2] = [vaddq_u32(t598[0], t600[0]), vaddq_u32(t598[1], t600[1])];
            let t602: [uint32x4_t; 2] = [vcltq_u32(t601[0], t598[0]), vcltq_u32(t601[1], t598[1])];
            let t603: [uint32x4_t; 4] = [t601[0], t601[1], vsubq_u32(vaddq_u32(vdupq_n_u32(0), t600[2]), t602[0]), vsubq_u32(vaddq_u32(vdupq_n_u32(0), t600[3]), t602[1])];
            let t604 = (arg3 as *const u8).add(((gid0.wrapping_add((16u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t604 as *mut u32).add(0), uint32x4x2_t(t603[0], t603[2]));
                vst2q_u32((t604 as *mut u32).add(8), uint32x4x2_t(t603[1], t603[3]));
            } else {
                let mut t605w = 0u64;
                let t605 = core::hint::black_box(&raw mut t605w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t604 as *mut u64).add(0), t605).write_unaligned(((vgetq_lane_u32::<0>(t603[2]) as u64) << 32 | vgetq_lane_u32::<0>(t603[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t604 as *mut u64).add(1), t605).write_unaligned(((vgetq_lane_u32::<1>(t603[2]) as u64) << 32 | vgetq_lane_u32::<1>(t603[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t604 as *mut u64).add(2), t605).write_unaligned(((vgetq_lane_u32::<2>(t603[2]) as u64) << 32 | vgetq_lane_u32::<2>(t603[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t604 as *mut u64).add(3), t605).write_unaligned(((vgetq_lane_u32::<3>(t603[2]) as u64) << 32 | vgetq_lane_u32::<3>(t603[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t604 as *mut u64).add(4), t605).write_unaligned(((vgetq_lane_u32::<0>(t603[3]) as u64) << 32 | vgetq_lane_u32::<0>(t603[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t604 as *mut u64).add(5), t605).write_unaligned(((vgetq_lane_u32::<1>(t603[3]) as u64) << 32 | vgetq_lane_u32::<1>(t603[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t604 as *mut u64).add(6), t605).write_unaligned(((vgetq_lane_u32::<2>(t603[3]) as u64) << 32 | vgetq_lane_u32::<2>(t603[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t604 as *mut u64).add(7), t605).write_unaligned(((vgetq_lane_u32::<3>(t603[3]) as u64) << 32 | vgetq_lane_u32::<3>(t603[1]) as u64));
            }
            let t606 = [vdupq_n_u32(x10); 2];
            let mut t607x = [0u32; 8];
            let mut t607y = [0u32; 8];
            for r in 0..2 { vst1q_u32(t607x.as_mut_ptr().add(4 * r), x8[r]); vst1q_u32(t607y.as_mut_ptr().add(4 * r), t606[r]); }
            for j in 0..8 { t607x[j] = (t607x[j] as u32).checked_rem(t607y[j] as u32).unwrap_or(0) as u32; }
            let t608: [uint32x4_t; 2] = [vld1q_u32(t607x.as_ptr().add(0)), vld1q_u32(t607x.as_ptr().add(4))];
            let t609 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t608[0]), vandq_u32(live[1], t608[1]))));
            let t610: [uint32x4_t; 2] = [vbslq_u32(live[0], t608[0], t609), vbslq_u32(live[1], t608[1], t609)];
            let mut t611: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t610[0]) as usize * 8) as *const u64).read_unaligned(); t611[0] = vsetq_lane_u32::<0>(v as u32, t611[0]); t611[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t611[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t610[0]) as usize * 8) as *const u64).read_unaligned(); t611[0] = vsetq_lane_u32::<1>(v as u32, t611[0]); t611[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t611[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t610[0]) as usize * 8) as *const u64).read_unaligned(); t611[0] = vsetq_lane_u32::<2>(v as u32, t611[0]); t611[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t611[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t610[0]) as usize * 8) as *const u64).read_unaligned(); t611[0] = vsetq_lane_u32::<3>(v as u32, t611[0]); t611[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t611[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t610[1]) as usize * 8) as *const u64).read_unaligned(); t611[1] = vsetq_lane_u32::<0>(v as u32, t611[1]); t611[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t611[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t610[1]) as usize * 8) as *const u64).read_unaligned(); t611[1] = vsetq_lane_u32::<1>(v as u32, t611[1]); t611[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t611[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t610[1]) as usize * 8) as *const u64).read_unaligned(); t611[1] = vsetq_lane_u32::<2>(v as u32, t611[1]); t611[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t611[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t610[1]) as usize * 8) as *const u64).read_unaligned(); t611[1] = vsetq_lane_u32::<3>(v as u32, t611[1]); t611[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t611[3]); }
            let t612 = (arg3 as *const u8).add(((gid0.wrapping_add((17u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t612 as *mut u32).add(0), uint32x4x2_t(t611[0], t611[2]));
                vst2q_u32((t612 as *mut u32).add(8), uint32x4x2_t(t611[1], t611[3]));
            } else {
                let mut t613w = 0u64;
                let t613 = core::hint::black_box(&raw mut t613w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t612 as *mut u64).add(0), t613).write_unaligned(((vgetq_lane_u32::<0>(t611[2]) as u64) << 32 | vgetq_lane_u32::<0>(t611[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t612 as *mut u64).add(1), t613).write_unaligned(((vgetq_lane_u32::<1>(t611[2]) as u64) << 32 | vgetq_lane_u32::<1>(t611[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t612 as *mut u64).add(2), t613).write_unaligned(((vgetq_lane_u32::<2>(t611[2]) as u64) << 32 | vgetq_lane_u32::<2>(t611[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t612 as *mut u64).add(3), t613).write_unaligned(((vgetq_lane_u32::<3>(t611[2]) as u64) << 32 | vgetq_lane_u32::<3>(t611[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t612 as *mut u64).add(4), t613).write_unaligned(((vgetq_lane_u32::<0>(t611[3]) as u64) << 32 | vgetq_lane_u32::<0>(t611[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t612 as *mut u64).add(5), t613).write_unaligned(((vgetq_lane_u32::<1>(t611[3]) as u64) << 32 | vgetq_lane_u32::<1>(t611[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t612 as *mut u64).add(6), t613).write_unaligned(((vgetq_lane_u32::<2>(t611[3]) as u64) << 32 | vgetq_lane_u32::<2>(t611[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t612 as *mut u64).add(7), t613).write_unaligned(((vgetq_lane_u32::<3>(t611[3]) as u64) << 32 | vgetq_lane_u32::<3>(t611[1]) as u64));
            }
            let t614 = b3_ll_p(l, lid0, live, x6, x8);
            let t615 = (arg3 as *const u8).add(((gid0.wrapping_add((18u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t615 as *mut u32).add(0), uint32x4x2_t(t614[0], t614[2]));
                vst2q_u32((t615 as *mut u32).add(8), uint32x4x2_t(t614[1], t614[3]));
            } else {
                let mut t616w = 0u64;
                let t616 = core::hint::black_box(&raw mut t616w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t615 as *mut u64).add(0), t616).write_unaligned(((vgetq_lane_u32::<0>(t614[2]) as u64) << 32 | vgetq_lane_u32::<0>(t614[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t615 as *mut u64).add(1), t616).write_unaligned(((vgetq_lane_u32::<1>(t614[2]) as u64) << 32 | vgetq_lane_u32::<1>(t614[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t615 as *mut u64).add(2), t616).write_unaligned(((vgetq_lane_u32::<2>(t614[2]) as u64) << 32 | vgetq_lane_u32::<2>(t614[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t615 as *mut u64).add(3), t616).write_unaligned(((vgetq_lane_u32::<3>(t614[2]) as u64) << 32 | vgetq_lane_u32::<3>(t614[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t615 as *mut u64).add(4), t616).write_unaligned(((vgetq_lane_u32::<0>(t614[3]) as u64) << 32 | vgetq_lane_u32::<0>(t614[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t615 as *mut u64).add(5), t616).write_unaligned(((vgetq_lane_u32::<1>(t614[3]) as u64) << 32 | vgetq_lane_u32::<1>(t614[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t615 as *mut u64).add(6), t616).write_unaligned(((vgetq_lane_u32::<2>(t614[3]) as u64) << 32 | vgetq_lane_u32::<2>(t614[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t615 as *mut u64).add(7), t616).write_unaligned(((vgetq_lane_u32::<3>(t614[3]) as u64) << 32 | vgetq_lane_u32::<3>(t614[1]) as u64));
            }
            x12 = x6;
            x13 = 0u32;
            let mut t621 = live;
            loop {
                // Lanes part: each its own rounds.
                let t623 = [vdupq_n_u32(7u32); 2];
                let t624: [uint32x4_t; 2] = [vandq_u32(x8[0], t623[0]), vandq_u32(x8[1], t623[1])];
                let t625 = [vdupq_n_u32(x13); 2];
                let t626: [uint32x4_t; 2] = [vcltq_u32(t625[0], t624[0]), vcltq_u32(t625[1], t624[1])];
                t621 = [vandq_u32(t621[0], t626[0]), vandq_u32(t621[1], t626[1])];
                if !(any(&t621)) { break; }
                let t628: u64 = 3u64;
                let t629 = [vdupq_n_u32((t628) as u32); 2];
                let t630: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x12[0]), vget_low_u32(t629[0]))), vreinterpretq_u32_u64(vmull_high_u32(x12[0], t629[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x12[1]), vget_low_u32(t629[1]))), vreinterpretq_u32_u64(vmull_high_u32(x12[1], t629[1])))];
                let t631: [uint32x4_t; 4] = [vmulq_u32(x12[0], t629[0]), vmulq_u32(x12[1], t629[1]), vmlaq_u32(t630[0], t629[0], x12[2]), vmlaq_u32(t630[1], t629[1], x12[3])];
                let t632: [uint32x4_t; 2] = [vaddq_u32(t631[0], x7[0]), vaddq_u32(t631[1], x7[1])];
                let t633: [uint32x4_t; 2] = [vcltq_u32(t632[0], t631[0]), vcltq_u32(t632[1], t631[1])];
                let t634: [uint32x4_t; 4] = [t632[0], t632[1], vsubq_u32(vaddq_u32(t631[2], x7[2]), t633[0]), vsubq_u32(vaddq_u32(t631[3], x7[3]), t633[1])];
                x12 = [vbslq_u32(t621[0], t634[0], x12[0]), vbslq_u32(t621[1], t634[1], x12[1]), vbslq_u32(t621[0], t634[2], x12[2]), vbslq_u32(t621[1], t634[3], x12[3])];
                let t635: [uint32x4_t; 2] = [vandq_u32(t621[0], live[0]), vandq_u32(t621[1], live[1])];
                let t636 = x13;
                x13 = (x13).wrapping_add(1u32);
            }
            let t637 = (arg3 as *const u8).add(((gid0.wrapping_add((19u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t637 as *mut u32).add(0), uint32x4x2_t(x12[0], x12[2]));
                vst2q_u32((t637 as *mut u32).add(8), uint32x4x2_t(x12[1], x12[3]));
            } else {
                let mut t638w = 0u64;
                let t638 = core::hint::black_box(&raw mut t638w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t637 as *mut u64).add(0), t638).write_unaligned(((vgetq_lane_u32::<0>(x12[2]) as u64) << 32 | vgetq_lane_u32::<0>(x12[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t637 as *mut u64).add(1), t638).write_unaligned(((vgetq_lane_u32::<1>(x12[2]) as u64) << 32 | vgetq_lane_u32::<1>(x12[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t637 as *mut u64).add(2), t638).write_unaligned(((vgetq_lane_u32::<2>(x12[2]) as u64) << 32 | vgetq_lane_u32::<2>(x12[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t637 as *mut u64).add(3), t638).write_unaligned(((vgetq_lane_u32::<3>(x12[2]) as u64) << 32 | vgetq_lane_u32::<3>(x12[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t637 as *mut u64).add(4), t638).write_unaligned(((vgetq_lane_u32::<0>(x12[3]) as u64) << 32 | vgetq_lane_u32::<0>(x12[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t637 as *mut u64).add(5), t638).write_unaligned(((vgetq_lane_u32::<1>(x12[3]) as u64) << 32 | vgetq_lane_u32::<1>(x12[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t637 as *mut u64).add(6), t638).write_unaligned(((vgetq_lane_u32::<2>(x12[3]) as u64) << 32 | vgetq_lane_u32::<2>(x12[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t637 as *mut u64).add(7), t638).write_unaligned(((vgetq_lane_u32::<3>(x12[3]) as u64) << 32 | vgetq_lane_u32::<3>(x12[1]) as u64));
            }
            let t639: u64 = a1;
            let t640 = [vdupq_n_u32((t639) as u32); 2];
            let t641 = [vdupq_n_u32(((t639) >> 32) as u32); 2];
            let t642: [uint32x4_t; 2] = [vaddq_u32(x6[0], t640[0]), vaddq_u32(x6[1], t640[1])];
            let t643: [uint32x4_t; 2] = [vcltq_u32(t642[0], x6[0]), vcltq_u32(t642[1], x6[1])];
            let t644: [uint32x4_t; 4] = [t642[0], t642[1], vsubq_u32(vaddq_u32(x6[2], t641[0]), t643[0]), vsubq_u32(vaddq_u32(x6[3], t641[1]), t643[1])];
            let t645: u64 = (a1).wrapping_shl((3u64) as u32);
            let t646 = [vdupq_n_u32((t645) as u32); 2];
            let t647 = [vdupq_n_u32(((t645) >> 32) as u32); 2];
            let t648: [uint32x4_t; 2] = [vaddq_u32(t644[0], t646[0]), vaddq_u32(t644[1], t646[1])];
            let t649: [uint32x4_t; 2] = [vcltq_u32(t648[0], t644[0]), vcltq_u32(t648[1], t644[1])];
            let t650: [uint32x4_t; 4] = [t648[0], t648[1], vsubq_u32(vaddq_u32(t644[2], t647[0]), t649[0]), vsubq_u32(vaddq_u32(t644[3], t647[1]), t649[1])];
            let t651 = (arg3 as *const u8).add(((gid0.wrapping_add((20u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t651 as *mut u32).add(0), uint32x4x2_t(t650[0], t650[2]));
                vst2q_u32((t651 as *mut u32).add(8), uint32x4x2_t(t650[1], t650[3]));
            } else {
                let mut t652w = 0u64;
                let t652 = core::hint::black_box(&raw mut t652w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t651 as *mut u64).add(0), t652).write_unaligned(((vgetq_lane_u32::<0>(t650[2]) as u64) << 32 | vgetq_lane_u32::<0>(t650[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t651 as *mut u64).add(1), t652).write_unaligned(((vgetq_lane_u32::<1>(t650[2]) as u64) << 32 | vgetq_lane_u32::<1>(t650[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t651 as *mut u64).add(2), t652).write_unaligned(((vgetq_lane_u32::<2>(t650[2]) as u64) << 32 | vgetq_lane_u32::<2>(t650[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t651 as *mut u64).add(3), t652).write_unaligned(((vgetq_lane_u32::<3>(t650[2]) as u64) << 32 | vgetq_lane_u32::<3>(t650[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t651 as *mut u64).add(4), t652).write_unaligned(((vgetq_lane_u32::<0>(t650[3]) as u64) << 32 | vgetq_lane_u32::<0>(t650[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t651 as *mut u64).add(5), t652).write_unaligned(((vgetq_lane_u32::<1>(t650[3]) as u64) << 32 | vgetq_lane_u32::<1>(t650[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t651 as *mut u64).add(6), t652).write_unaligned(((vgetq_lane_u32::<2>(t650[3]) as u64) << 32 | vgetq_lane_u32::<2>(t650[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t651 as *mut u64).add(7), t652).write_unaligned(((vgetq_lane_u32::<3>(t650[3]) as u64) << 32 | vgetq_lane_u32::<3>(t650[1]) as u64));
            }
            let t653: u64 = 4294967295u64;
            let t654 = [vdupq_n_u32((t653) as u32); 2];
            let t655: [uint32x4_t; 2] = [vandq_u32(x6[0], t654[0]), vandq_u32(x6[1], t654[1])];
            let t656: u64 = 4294967295u64;
            let t657 = [vdupq_n_u32((t656) as u32); 2];
            let t658: [uint32x4_t; 2] = [vandq_u32(x7[0], t657[0]), vandq_u32(x7[1], t657[1])];
            let t659: [uint32x4_t; 2] = [vceqq_u32(t655[0], t658[0]), vceqq_u32(t655[1], t658[1])];
            let mut t660: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t661: [uint32x4_t; 2] = [vandq_u32(live[0], t659[0]), vandq_u32(live[1], t659[1])];
            let t662: [uint32x4_t; 2] = [vbicq_u32(live[0], t659[0]), vbicq_u32(live[1], t659[1])];
            {
                let t663 = [vdupq_n_u32(1u32); 2];
                t660 = [vbslq_u32(t661[0], t663[0], t660[0]), vbslq_u32(t661[1], t663[1], t660[1])];
            }
            {
                let t664 = [vdupq_n_u32(2u32); 2];
                t660 = [vbslq_u32(t662[0], t664[0], t660[0]), vbslq_u32(t662[1], t664[1], t660[1])];
            }
            let t665: [uint32x4_t; 4] = [t660[0], t660[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            let t666 = (arg3 as *const u8).add(((gid0.wrapping_add((21u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t666 as *mut u32).add(0), uint32x4x2_t(t665[0], t665[2]));
                vst2q_u32((t666 as *mut u32).add(8), uint32x4x2_t(t665[1], t665[3]));
            } else {
                let mut t667w = 0u64;
                let t667 = core::hint::black_box(&raw mut t667w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t666 as *mut u64).add(0), t667).write_unaligned(((vgetq_lane_u32::<0>(t665[2]) as u64) << 32 | vgetq_lane_u32::<0>(t665[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t666 as *mut u64).add(1), t667).write_unaligned(((vgetq_lane_u32::<1>(t665[2]) as u64) << 32 | vgetq_lane_u32::<1>(t665[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t666 as *mut u64).add(2), t667).write_unaligned(((vgetq_lane_u32::<2>(t665[2]) as u64) << 32 | vgetq_lane_u32::<2>(t665[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t666 as *mut u64).add(3), t667).write_unaligned(((vgetq_lane_u32::<3>(t665[2]) as u64) << 32 | vgetq_lane_u32::<3>(t665[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t666 as *mut u64).add(4), t667).write_unaligned(((vgetq_lane_u32::<0>(t665[3]) as u64) << 32 | vgetq_lane_u32::<0>(t665[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t666 as *mut u64).add(5), t667).write_unaligned(((vgetq_lane_u32::<1>(t665[3]) as u64) << 32 | vgetq_lane_u32::<1>(t665[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t666 as *mut u64).add(6), t667).write_unaligned(((vgetq_lane_u32::<2>(t665[3]) as u64) << 32 | vgetq_lane_u32::<2>(t665[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t666 as *mut u64).add(7), t667).write_unaligned(((vgetq_lane_u32::<3>(t665[3]) as u64) << 32 | vgetq_lane_u32::<3>(t665[1]) as u64));
            }
            let t668: [uint32x4_t; 2] = [vtstq_u32(vorrq_u32(x6[0], x6[2]), vorrq_u32(x6[0], x6[2])), vtstq_u32(vorrq_u32(x6[1], x6[3]), vorrq_u32(x6[1], x6[3]))];
            let mut t669: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t670: [uint32x4_t; 2] = [vandq_u32(live[0], t668[0]), vandq_u32(live[1], t668[1])];
            let t671: [uint32x4_t; 2] = [vbicq_u32(live[0], t668[0]), vbicq_u32(live[1], t668[1])];
            {
                let t672 = [vdupq_n_u32(5u32); 2];
                t669 = [vbslq_u32(t670[0], t672[0], t669[0]), vbslq_u32(t670[1], t672[1], t669[1])];
            }
            {
                let t673 = [vdupq_n_u32(6u32); 2];
                t669 = [vbslq_u32(t671[0], t673[0], t669[0]), vbslq_u32(t671[1], t673[1], t669[1])];
            }
            let t674: [uint32x4_t; 4] = [t669[0], t669[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            let t675 = (arg3 as *const u8).add(((gid0.wrapping_add((22u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t675 as *mut u32).add(0), uint32x4x2_t(t674[0], t674[2]));
                vst2q_u32((t675 as *mut u32).add(8), uint32x4x2_t(t674[1], t674[3]));
            } else {
                let mut t676w = 0u64;
                let t676 = core::hint::black_box(&raw mut t676w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t675 as *mut u64).add(0), t676).write_unaligned(((vgetq_lane_u32::<0>(t674[2]) as u64) << 32 | vgetq_lane_u32::<0>(t674[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t675 as *mut u64).add(1), t676).write_unaligned(((vgetq_lane_u32::<1>(t674[2]) as u64) << 32 | vgetq_lane_u32::<1>(t674[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t675 as *mut u64).add(2), t676).write_unaligned(((vgetq_lane_u32::<2>(t674[2]) as u64) << 32 | vgetq_lane_u32::<2>(t674[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t675 as *mut u64).add(3), t676).write_unaligned(((vgetq_lane_u32::<3>(t674[2]) as u64) << 32 | vgetq_lane_u32::<3>(t674[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t675 as *mut u64).add(4), t676).write_unaligned(((vgetq_lane_u32::<0>(t674[3]) as u64) << 32 | vgetq_lane_u32::<0>(t674[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t675 as *mut u64).add(5), t676).write_unaligned(((vgetq_lane_u32::<1>(t674[3]) as u64) << 32 | vgetq_lane_u32::<1>(t674[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t675 as *mut u64).add(6), t676).write_unaligned(((vgetq_lane_u32::<2>(t674[3]) as u64) << 32 | vgetq_lane_u32::<2>(t674[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t675 as *mut u64).add(7), t676).write_unaligned(((vgetq_lane_u32::<3>(t674[3]) as u64) << 32 | vgetq_lane_u32::<3>(t674[1]) as u64));
            }
            let t677 = b4_ll_p(l, lid0, live, x6, x7);
            let t678 = (arg3 as *const u8).add(((gid0.wrapping_add((23u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t678 as *mut u32).add(0), uint32x4x2_t(t677[0], t677[2]));
                vst2q_u32((t678 as *mut u32).add(8), uint32x4x2_t(t677[1], t677[3]));
            } else {
                let mut t679w = 0u64;
                let t679 = core::hint::black_box(&raw mut t679w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t678 as *mut u64).add(0), t679).write_unaligned(((vgetq_lane_u32::<0>(t677[2]) as u64) << 32 | vgetq_lane_u32::<0>(t677[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t678 as *mut u64).add(1), t679).write_unaligned(((vgetq_lane_u32::<1>(t677[2]) as u64) << 32 | vgetq_lane_u32::<1>(t677[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t678 as *mut u64).add(2), t679).write_unaligned(((vgetq_lane_u32::<2>(t677[2]) as u64) << 32 | vgetq_lane_u32::<2>(t677[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t678 as *mut u64).add(3), t679).write_unaligned(((vgetq_lane_u32::<3>(t677[2]) as u64) << 32 | vgetq_lane_u32::<3>(t677[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t678 as *mut u64).add(4), t679).write_unaligned(((vgetq_lane_u32::<0>(t677[3]) as u64) << 32 | vgetq_lane_u32::<0>(t677[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t678 as *mut u64).add(5), t679).write_unaligned(((vgetq_lane_u32::<1>(t677[3]) as u64) << 32 | vgetq_lane_u32::<1>(t677[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t678 as *mut u64).add(6), t679).write_unaligned(((vgetq_lane_u32::<2>(t677[3]) as u64) << 32 | vgetq_lane_u32::<2>(t677[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t678 as *mut u64).add(7), t679).write_unaligned(((vgetq_lane_u32::<3>(t677[3]) as u64) << 32 | vgetq_lane_u32::<3>(t677[1]) as u64));
            }
            let t680: u32;
            if ((((a1) & (1u64))) != (0u64)) {
                t680 = 8u32;
            } else {
                t680 = 4u32;
            }
            x14 = t680;
            let t681 = b5_r0ns_p(l, lid0, live, arg0, gid0.wrapping_mul(x14), x14, 8u32);
            let t682 = (arg3 as *const u8).add(((gid0.wrapping_add((24u32).wrapping_mul(x10))) as usize) * 8);
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
            let t685 = (arg3 as *const u8).add(((gid0.wrapping_add((25u32).wrapping_mul(x10))) as usize) * 8);
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
            let t689: [uint32x4_t; 2] = [vandq_u32(x6[0], t688[0]), vandq_u32(x6[1], t688[1])];
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
                    let t698 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x7);
                    let t699 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x8);
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
                    let t704 = b7_ll_p(l, lid0, t694, x7, x8);
                    t696 = t704;
                }
                t693 = [vbslq_u32(t694[0], t696[0], t693[0]), vbslq_u32(t694[1], t696[1], t693[1]), vbslq_u32(t694[0], t696[2], t693[2]), vbslq_u32(t694[1], t696[3], t693[3])];
            }
            {
                t693 = [vbslq_u32(t695[0], x6[0], t693[0]), vbslq_u32(t695[1], x6[1], t693[1]), vbslq_u32(t695[0], x6[2], t693[2]), vbslq_u32(t695[1], x6[3], t693[3])];
            }
            let t705 = (arg3 as *const u8).add(((gid0.wrapping_add((26u32).wrapping_mul(x10))) as usize) * 8);
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
unsafe fn b3_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], x1: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x2: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
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
        let t718: [uint32x4_t; 4] = [x1[0], x1[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = [vbslq_u32(t717[0], t718[0], res[0]), vbslq_u32(t717[1], t718[1], res[1]), vbslq_u32(t717[0], t718[2], res[2]), vbslq_u32(t717[1], t718[3], res[3])];
        live = [vbicq_u32(live[0], t717[0]), vbicq_u32(live[1], t717[1])];
    }
    if any(&live) {
        let t719: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x0[0]), vshlq_n_u32::<30>(x0[2])), vorrq_u32(vshrq_n_u32::<2>(x0[1]), vshlq_n_u32::<30>(x0[3])), vshrq_n_u32::<2>(x0[2]), vshrq_n_u32::<2>(x0[3])];
        let t720: [uint32x4_t; 2] = [t719[0], t719[1]];
        x2 = t720;
        let t721 = [vdupq_n_u32(1u32); 2];
        let t722: [uint32x4_t; 2] = [vsubq_u32(x2[0], t721[0]), vsubq_u32(x2[1], t721[1])];
        let t723: [uint32x4_t; 2] = [vcgeq_u32(t722[0], x1[0]), vcgeq_u32(t722[1], x1[1])];
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
                let t736: [uint32x4_t; 2] = [vsubq_u32(x1[0], x2[0]), vsubq_u32(x1[1], x2[1])];
                let t737: [uint32x4_t; 4] = [t736[0], t736[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t735[0], t737[0], res[0]), vbslq_u32(t735[1], t737[1], res[1]), vbslq_u32(t735[0], t737[2], res[2]), vbslq_u32(t735[1], t737[3], res[3])];
                live = [vbicq_u32(live[0], t735[0]), vbicq_u32(live[1], t735[1])];
            }
            if any(&live) {
                let t738: [uint32x4_t; 2] = [vsubq_u32(x1[0], x2[0]), vsubq_u32(x1[1], x2[1])];
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
unsafe fn b3_ll_f(l: &Launch, lid0: u32, x0: [uint32x4_t; 4], x1: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x2: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t743: u64 = 1u64;
    let t744 = [vdupq_n_u32((t743) as u32); 2];
    let t745: [uint32x4_t; 2] = [vandq_u32(x0[0], t744[0]), vandq_u32(x0[1], t744[1])];
    let t746: u64 = 0u64;
    let t747 = [vdupq_n_u32((t746) as u32); 2];
    let t748: [uint32x4_t; 2] = [vceqq_u32(t745[0], t747[0]), vceqq_u32(t745[1], t747[1])];
    {
        let t749: [uint32x4_t; 4] = [x1[0], x1[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = [vbslq_u32(t748[0], t749[0], res[0]), vbslq_u32(t748[1], t749[1], res[1]), vbslq_u32(t748[0], t749[2], res[2]), vbslq_u32(t748[1], t749[3], res[3])];
        live = [vbicq_u32(live[0], t748[0]), vbicq_u32(live[1], t748[1])];
    }
    if any(&live) {
        let t750: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x0[0]), vshlq_n_u32::<30>(x0[2])), vorrq_u32(vshrq_n_u32::<2>(x0[1]), vshlq_n_u32::<30>(x0[3])), vshrq_n_u32::<2>(x0[2]), vshrq_n_u32::<2>(x0[3])];
        let t751: [uint32x4_t; 2] = [t750[0], t750[1]];
        x2 = t751;
        let t752 = [vdupq_n_u32(1u32); 2];
        let t753: [uint32x4_t; 2] = [vsubq_u32(x2[0], t752[0]), vsubq_u32(x2[1], t752[1])];
        let t754: [uint32x4_t; 2] = [vcgeq_u32(t753[0], x1[0]), vcgeq_u32(t753[1], x1[1])];
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
                let t767: [uint32x4_t; 2] = [vsubq_u32(x1[0], x2[0]), vsubq_u32(x1[1], x2[1])];
                let t768: [uint32x4_t; 4] = [t767[0], t767[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t766[0], t768[0], res[0]), vbslq_u32(t766[1], t768[1], res[1]), vbslq_u32(t766[0], t768[2], res[2]), vbslq_u32(t766[1], t768[3], res[3])];
                live = [vbicq_u32(live[0], t766[0]), vbicq_u32(live[1], t766[1])];
            }
            if any(&live) {
                let t769: [uint32x4_t; 2] = [vsubq_u32(x1[0], x2[0]), vsubq_u32(x1[1], x2[1])];
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
unsafe fn b4_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], x1: [uint32x4_t; 4]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let (mut t774l, mut t774h) = ([0u32; 8], [0u32; 8]);
    { let v = (((((vgetq_lane_u32::<0>(x0[2]) as u64) << 32 | vgetq_lane_u32::<0>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<0>(x1[2]) as u64) << 32 | vgetq_lane_u32::<0>(x1[0]) as u64) as u64) as u128) >> 64) as u64; t774l[0] = v as u32; t774h[0] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<1>(x0[2]) as u64) << 32 | vgetq_lane_u32::<1>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<1>(x1[2]) as u64) << 32 | vgetq_lane_u32::<1>(x1[0]) as u64) as u64) as u128) >> 64) as u64; t774l[1] = v as u32; t774h[1] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<2>(x0[2]) as u64) << 32 | vgetq_lane_u32::<2>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<2>(x1[2]) as u64) << 32 | vgetq_lane_u32::<2>(x1[0]) as u64) as u64) as u128) >> 64) as u64; t774l[2] = v as u32; t774h[2] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<3>(x0[2]) as u64) << 32 | vgetq_lane_u32::<3>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<3>(x1[2]) as u64) << 32 | vgetq_lane_u32::<3>(x1[0]) as u64) as u64) as u128) >> 64) as u64; t774l[3] = v as u32; t774h[3] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<0>(x0[3]) as u64) << 32 | vgetq_lane_u32::<0>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<0>(x1[3]) as u64) << 32 | vgetq_lane_u32::<0>(x1[1]) as u64) as u64) as u128) >> 64) as u64; t774l[4] = v as u32; t774h[4] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<1>(x0[3]) as u64) << 32 | vgetq_lane_u32::<1>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<1>(x1[3]) as u64) << 32 | vgetq_lane_u32::<1>(x1[1]) as u64) as u64) as u128) >> 64) as u64; t774l[5] = v as u32; t774h[5] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<2>(x0[3]) as u64) << 32 | vgetq_lane_u32::<2>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<2>(x1[3]) as u64) << 32 | vgetq_lane_u32::<2>(x1[1]) as u64) as u64) as u128) >> 64) as u64; t774l[6] = v as u32; t774h[6] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<3>(x0[3]) as u64) << 32 | vgetq_lane_u32::<3>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<3>(x1[3]) as u64) << 32 | vgetq_lane_u32::<3>(x1[1]) as u64) as u64) as u128) >> 64) as u64; t774l[7] = v as u32; t774h[7] = (v >> 32) as u32; }
    let t775: [uint32x4_t; 4] = [vld1q_u32(t774l.as_ptr().add(0)), vld1q_u32(t774l.as_ptr().add(4)), vld1q_u32(t774h.as_ptr().add(0)), vld1q_u32(t774h.as_ptr().add(4))];
    res = t775;
    return res;
}

#[inline(always)]
unsafe fn b4_ll_f(l: &Launch, lid0: u32, x0: [uint32x4_t; 4], x1: [uint32x4_t; 4]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let (mut t776l, mut t776h) = ([0u32; 8], [0u32; 8]);
    { let v = (((((vgetq_lane_u32::<0>(x0[2]) as u64) << 32 | vgetq_lane_u32::<0>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<0>(x1[2]) as u64) << 32 | vgetq_lane_u32::<0>(x1[0]) as u64) as u64) as u128) >> 64) as u64; t776l[0] = v as u32; t776h[0] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<1>(x0[2]) as u64) << 32 | vgetq_lane_u32::<1>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<1>(x1[2]) as u64) << 32 | vgetq_lane_u32::<1>(x1[0]) as u64) as u64) as u128) >> 64) as u64; t776l[1] = v as u32; t776h[1] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<2>(x0[2]) as u64) << 32 | vgetq_lane_u32::<2>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<2>(x1[2]) as u64) << 32 | vgetq_lane_u32::<2>(x1[0]) as u64) as u64) as u128) >> 64) as u64; t776l[2] = v as u32; t776h[2] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<3>(x0[2]) as u64) << 32 | vgetq_lane_u32::<3>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<3>(x1[2]) as u64) << 32 | vgetq_lane_u32::<3>(x1[0]) as u64) as u64) as u128) >> 64) as u64; t776l[3] = v as u32; t776h[3] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<0>(x0[3]) as u64) << 32 | vgetq_lane_u32::<0>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<0>(x1[3]) as u64) << 32 | vgetq_lane_u32::<0>(x1[1]) as u64) as u64) as u128) >> 64) as u64; t776l[4] = v as u32; t776h[4] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<1>(x0[3]) as u64) << 32 | vgetq_lane_u32::<1>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<1>(x1[3]) as u64) << 32 | vgetq_lane_u32::<1>(x1[1]) as u64) as u64) as u128) >> 64) as u64; t776l[5] = v as u32; t776h[5] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<2>(x0[3]) as u64) << 32 | vgetq_lane_u32::<2>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<2>(x1[3]) as u64) << 32 | vgetq_lane_u32::<2>(x1[1]) as u64) as u64) as u128) >> 64) as u64; t776l[6] = v as u32; t776h[6] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<3>(x0[3]) as u64) << 32 | vgetq_lane_u32::<3>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<3>(x1[3]) as u64) << 32 | vgetq_lane_u32::<3>(x1[1]) as u64) as u64) as u128) >> 64) as u64; t776l[7] = v as u32; t776h[7] = (v >> 32) as u32; }
    let t777: [uint32x4_t; 4] = [vld1q_u32(t776l.as_ptr().add(0)), vld1q_u32(t776l.as_ptr().add(4)), vld1q_u32(t776h.as_ptr().add(0)), vld1q_u32(t776h.as_ptr().add(4))];
    res = t777;
    return res;
}

#[inline(always)]
unsafe fn b5_r0ns_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1f: u32, x1s: u32, x2: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x3: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x4: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x5: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if ((x2) == (8u32)) {
        let t778: [uint32x4_t; 4];
        if (x1s) == 8u32 && ((x1f) as u64) + 7 * ((x1s) as u64) <= u32::MAX as u64 && all(&live) {
            let t779 = (arg0 as *const u8).add(((x1f) as usize) * 1);
            let t7800 = vld2q_u32((t779 as *const u32).add(0));
            let t7801 = vld2q_u32((t779 as *const u32).add(8));
            let t780: [uint32x4_t; 4] = [t7800.0, t7801.0, t7800.1, t7801.1];
            t778 = t780;
        } else {
            let t781: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1s)), vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1s))];
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
    let t785: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1s)), vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1s))];
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
    x3 = t795;
    let t796: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1s)), vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1s))];
    let t797 = [vdupq_n_u32(7u32); 2];
    let t798: [uint32x4_t; 2] = [vandq_u32(t796[0], t797[0]), vandq_u32(t796[1], t797[1])];
    let t799 = [vdupq_n_u32(8u32); 2];
    let t800: [uint32x4_t; 2] = [vmulq_u32(t798[0], t799[0]), vmulq_u32(t798[1], t799[1])];
    x4 = t800;
    let mut t801: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x3[2]) as u64) << 32 | vgetq_lane_u32::<0>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t801[0] = vsetq_lane_u32::<0>(v as u32, t801[0]); t801[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t801[2]); }
    { let v = (((vgetq_lane_u32::<1>(x3[2]) as u64) << 32 | vgetq_lane_u32::<1>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t801[0] = vsetq_lane_u32::<1>(v as u32, t801[0]); t801[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t801[2]); }
    { let v = (((vgetq_lane_u32::<2>(x3[2]) as u64) << 32 | vgetq_lane_u32::<2>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t801[0] = vsetq_lane_u32::<2>(v as u32, t801[0]); t801[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t801[2]); }
    { let v = (((vgetq_lane_u32::<3>(x3[2]) as u64) << 32 | vgetq_lane_u32::<3>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t801[0] = vsetq_lane_u32::<3>(v as u32, t801[0]); t801[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t801[2]); }
    { let v = (((vgetq_lane_u32::<0>(x3[3]) as u64) << 32 | vgetq_lane_u32::<0>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t801[1] = vsetq_lane_u32::<0>(v as u32, t801[1]); t801[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t801[3]); }
    { let v = (((vgetq_lane_u32::<1>(x3[3]) as u64) << 32 | vgetq_lane_u32::<1>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t801[1] = vsetq_lane_u32::<1>(v as u32, t801[1]); t801[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t801[3]); }
    { let v = (((vgetq_lane_u32::<2>(x3[3]) as u64) << 32 | vgetq_lane_u32::<2>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t801[1] = vsetq_lane_u32::<2>(v as u32, t801[1]); t801[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t801[3]); }
    { let v = (((vgetq_lane_u32::<3>(x3[3]) as u64) << 32 | vgetq_lane_u32::<3>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t801[1] = vsetq_lane_u32::<3>(v as u32, t801[1]); t801[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t801[3]); }
    let t802: [uint32x4_t; 2] = [x4[0], x4[1]];
    let t803: [uint32x4_t; 2] = [vandq_u32(t802[0], vdupq_n_u32(63)), vandq_u32(t802[1], vdupq_n_u32(63))];
    let t804: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t801[0], vnegq_s32(vreinterpretq_s32_u32(t803[0]))), vshlq_u32(t801[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t803[0])))), vorrq_u32(vshlq_u32(t801[1], vnegq_s32(vreinterpretq_s32_u32(t803[1]))), vshlq_u32(t801[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t803[1])))), vshlq_u32(t801[2], vnegq_s32(vreinterpretq_s32_u32(t803[0]))), vshlq_u32(t801[3], vnegq_s32(vreinterpretq_s32_u32(t803[1])))];
    x5 = t804;
    let t805 = [vdupq_n_u32(0u32); 2];
    let t806: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t805[0])), vmvnq_u32(vceqq_u32(x4[1], t805[1]))];
    let t807 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
    let t808: [uint32x4_t; 2] = [vaddq_u32(x4[0], t807[0]), vaddq_u32(x4[1], t807[1])];
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
        let t818: [uint32x4_t; 2] = [vaddq_u32(x3[0], t816[0]), vaddq_u32(x3[1], t816[1])];
        let t819: [uint32x4_t; 2] = [vcltq_u32(t818[0], x3[0]), vcltq_u32(t818[1], x3[1])];
        let t820: [uint32x4_t; 4] = [t818[0], t818[1], vsubq_u32(vaddq_u32(x3[2], t817[0]), t819[0]), vsubq_u32(vaddq_u32(x3[3], t817[1]), t819[1])];
        let mut t821: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t820[2]) as u64) << 32 | vgetq_lane_u32::<0>(t820[0]) as u64) as usize as *const u64).read_unaligned(); t821[0] = vsetq_lane_u32::<0>(v as u32, t821[0]); t821[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t821[2]); }
        { let v = (((vgetq_lane_u32::<1>(t820[2]) as u64) << 32 | vgetq_lane_u32::<1>(t820[0]) as u64) as usize as *const u64).read_unaligned(); t821[0] = vsetq_lane_u32::<1>(v as u32, t821[0]); t821[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t821[2]); }
        { let v = (((vgetq_lane_u32::<2>(t820[2]) as u64) << 32 | vgetq_lane_u32::<2>(t820[0]) as u64) as usize as *const u64).read_unaligned(); t821[0] = vsetq_lane_u32::<2>(v as u32, t821[0]); t821[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t821[2]); }
        { let v = (((vgetq_lane_u32::<3>(t820[2]) as u64) << 32 | vgetq_lane_u32::<3>(t820[0]) as u64) as usize as *const u64).read_unaligned(); t821[0] = vsetq_lane_u32::<3>(v as u32, t821[0]); t821[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t821[2]); }
        { let v = (((vgetq_lane_u32::<0>(t820[3]) as u64) << 32 | vgetq_lane_u32::<0>(t820[1]) as u64) as usize as *const u64).read_unaligned(); t821[1] = vsetq_lane_u32::<0>(v as u32, t821[1]); t821[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t821[3]); }
        { let v = (((vgetq_lane_u32::<1>(t820[3]) as u64) << 32 | vgetq_lane_u32::<1>(t820[1]) as u64) as usize as *const u64).read_unaligned(); t821[1] = vsetq_lane_u32::<1>(v as u32, t821[1]); t821[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t821[3]); }
        { let v = (((vgetq_lane_u32::<2>(t820[3]) as u64) << 32 | vgetq_lane_u32::<2>(t820[1]) as u64) as usize as *const u64).read_unaligned(); t821[1] = vsetq_lane_u32::<2>(v as u32, t821[1]); t821[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t821[3]); }
        { let v = (((vgetq_lane_u32::<3>(t820[3]) as u64) << 32 | vgetq_lane_u32::<3>(t820[1]) as u64) as usize as *const u64).read_unaligned(); t821[1] = vsetq_lane_u32::<3>(v as u32, t821[1]); t821[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t821[3]); }
        let t822: [uint32x4_t; 2] = [vsubq_u32(t809[0], x4[0]), vsubq_u32(t809[1], x4[1])];
        let t823: [uint32x4_t; 2] = [t822[0], t822[1]];
        let t824: [uint32x4_t; 2] = [vandq_u32(t823[0], vdupq_n_u32(63)), vandq_u32(t823[1], vdupq_n_u32(63))];
        let t825: [uint32x4_t; 4] = [vshlq_u32(t821[0], vreinterpretq_s32_u32(t824[0])), vshlq_u32(t821[1], vreinterpretq_s32_u32(t824[1])), vorrq_u32(vshlq_u32(t821[2], vreinterpretq_s32_u32(t824[0])), vshlq_u32(t821[0], vsubq_s32(vreinterpretq_s32_u32(t824[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t821[3], vreinterpretq_s32_u32(t824[1])), vshlq_u32(t821[1], vsubq_s32(vreinterpretq_s32_u32(t824[1]), vdupq_n_s32(32))))];
        let t826: [uint32x4_t; 4] = [vorrq_u32(x5[0], t825[0]), vorrq_u32(x5[1], t825[1]), vorrq_u32(x5[2], t825[2]), vorrq_u32(x5[3], t825[3])];
        t812 = [vbslq_u32(t813[0], t826[0], t812[0]), vbslq_u32(t813[1], t826[1], t812[1]), vbslq_u32(t813[0], t826[2], t812[2]), vbslq_u32(t813[1], t826[3], t812[3])];
    }
    {
        t812 = [vbslq_u32(t814[0], x5[0], t812[0]), vbslq_u32(t814[1], x5[1], t812[1]), vbslq_u32(t814[0], x5[2], t812[2]), vbslq_u32(t814[1], x5[3], t812[3])];
    }
    res = t812;
    return res;
}

#[inline(always)]
unsafe fn b5_r0ns_f_v0(l: &Launch, lid0: u32, arg0: *const u8, x1f: u32, x1s: u32, x2: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x3: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x4: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x5: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if ((x2) == (8u32)) {
        let t827: [uint32x4_t; 4];
        if (x1s) == 8u32 && ((x1f) as u64) + 7 * ((x1s) as u64) <= u32::MAX as u64 {
            let t828 = (arg0 as *const u8).add(((x1f) as usize) * 1);
            let t8290 = vld2q_u32((t828 as *const u32).add(0));
            let t8291 = vld2q_u32((t828 as *const u32).add(8));
            let t829: [uint32x4_t; 4] = [t8290.0, t8291.0, t8290.1, t8291.1];
            t827 = t829;
        } else {
            let t830: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1s)), vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1s))];
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
    let t832: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1s)), vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1s))];
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
    x3 = t842;
    let t843: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1s)), vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1s))];
    let t844 = [vdupq_n_u32(7u32); 2];
    let t845: [uint32x4_t; 2] = [vandq_u32(t843[0], t844[0]), vandq_u32(t843[1], t844[1])];
    let t846 = [vdupq_n_u32(8u32); 2];
    let t847: [uint32x4_t; 2] = [vmulq_u32(t845[0], t846[0]), vmulq_u32(t845[1], t846[1])];
    x4 = t847;
    let mut t848: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x3[2]) as u64) << 32 | vgetq_lane_u32::<0>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t848[0] = vsetq_lane_u32::<0>(v as u32, t848[0]); t848[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t848[2]); }
    { let v = (((vgetq_lane_u32::<1>(x3[2]) as u64) << 32 | vgetq_lane_u32::<1>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t848[0] = vsetq_lane_u32::<1>(v as u32, t848[0]); t848[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t848[2]); }
    { let v = (((vgetq_lane_u32::<2>(x3[2]) as u64) << 32 | vgetq_lane_u32::<2>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t848[0] = vsetq_lane_u32::<2>(v as u32, t848[0]); t848[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t848[2]); }
    { let v = (((vgetq_lane_u32::<3>(x3[2]) as u64) << 32 | vgetq_lane_u32::<3>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t848[0] = vsetq_lane_u32::<3>(v as u32, t848[0]); t848[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t848[2]); }
    { let v = (((vgetq_lane_u32::<0>(x3[3]) as u64) << 32 | vgetq_lane_u32::<0>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t848[1] = vsetq_lane_u32::<0>(v as u32, t848[1]); t848[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t848[3]); }
    { let v = (((vgetq_lane_u32::<1>(x3[3]) as u64) << 32 | vgetq_lane_u32::<1>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t848[1] = vsetq_lane_u32::<1>(v as u32, t848[1]); t848[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t848[3]); }
    { let v = (((vgetq_lane_u32::<2>(x3[3]) as u64) << 32 | vgetq_lane_u32::<2>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t848[1] = vsetq_lane_u32::<2>(v as u32, t848[1]); t848[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t848[3]); }
    { let v = (((vgetq_lane_u32::<3>(x3[3]) as u64) << 32 | vgetq_lane_u32::<3>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t848[1] = vsetq_lane_u32::<3>(v as u32, t848[1]); t848[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t848[3]); }
    let t849: [uint32x4_t; 2] = [x4[0], x4[1]];
    let t850: [uint32x4_t; 2] = [vandq_u32(t849[0], vdupq_n_u32(63)), vandq_u32(t849[1], vdupq_n_u32(63))];
    let t851: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t848[0], vnegq_s32(vreinterpretq_s32_u32(t850[0]))), vshlq_u32(t848[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t850[0])))), vorrq_u32(vshlq_u32(t848[1], vnegq_s32(vreinterpretq_s32_u32(t850[1]))), vshlq_u32(t848[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t850[1])))), vshlq_u32(t848[2], vnegq_s32(vreinterpretq_s32_u32(t850[0]))), vshlq_u32(t848[3], vnegq_s32(vreinterpretq_s32_u32(t850[1])))];
    x5 = t851;
    let t852 = [vdupq_n_u32(0u32); 2];
    let t853: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t852[0])), vmvnq_u32(vceqq_u32(x4[1], t852[1]))];
    let t854 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
    let t855: [uint32x4_t; 2] = [vaddq_u32(x4[0], t854[0]), vaddq_u32(x4[1], t854[1])];
    let t856 = [vdupq_n_u32(64u32); 2];
    let t857: [uint32x4_t; 2] = [vcgtq_u32(t855[0], t856[0]), vcgtq_u32(t855[1], t856[1])];
    let t858: [uint32x4_t; 2] = [vandq_u32(t853[0], t857[0]), vandq_u32(t853[1], t857[1])];
    let mut t859: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t860: [uint32x4_t; 2] = [vmvnq_u32(t858[0]), vmvnq_u32(t858[1])];
    if any(&t858) {
        let t861: u64 = (1u64).wrapping_mul(8u64);
        let t862 = [vdupq_n_u32((t861) as u32); 2];
        let t863 = [vdupq_n_u32(((t861) >> 32) as u32); 2];
        let t864: [uint32x4_t; 2] = [vaddq_u32(x3[0], t862[0]), vaddq_u32(x3[1], t862[1])];
        let t865: [uint32x4_t; 2] = [vcltq_u32(t864[0], x3[0]), vcltq_u32(t864[1], x3[1])];
        let t866: [uint32x4_t; 4] = [t864[0], t864[1], vsubq_u32(vaddq_u32(x3[2], t863[0]), t865[0]), vsubq_u32(vaddq_u32(x3[3], t863[1]), t865[1])];
        let mut t867: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t866[2]) as u64) << 32 | vgetq_lane_u32::<0>(t866[0]) as u64) as usize as *const u64).read_unaligned(); t867[0] = vsetq_lane_u32::<0>(v as u32, t867[0]); t867[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t867[2]); }
        { let v = (((vgetq_lane_u32::<1>(t866[2]) as u64) << 32 | vgetq_lane_u32::<1>(t866[0]) as u64) as usize as *const u64).read_unaligned(); t867[0] = vsetq_lane_u32::<1>(v as u32, t867[0]); t867[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t867[2]); }
        { let v = (((vgetq_lane_u32::<2>(t866[2]) as u64) << 32 | vgetq_lane_u32::<2>(t866[0]) as u64) as usize as *const u64).read_unaligned(); t867[0] = vsetq_lane_u32::<2>(v as u32, t867[0]); t867[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t867[2]); }
        { let v = (((vgetq_lane_u32::<3>(t866[2]) as u64) << 32 | vgetq_lane_u32::<3>(t866[0]) as u64) as usize as *const u64).read_unaligned(); t867[0] = vsetq_lane_u32::<3>(v as u32, t867[0]); t867[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t867[2]); }
        { let v = (((vgetq_lane_u32::<0>(t866[3]) as u64) << 32 | vgetq_lane_u32::<0>(t866[1]) as u64) as usize as *const u64).read_unaligned(); t867[1] = vsetq_lane_u32::<0>(v as u32, t867[1]); t867[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t867[3]); }
        { let v = (((vgetq_lane_u32::<1>(t866[3]) as u64) << 32 | vgetq_lane_u32::<1>(t866[1]) as u64) as usize as *const u64).read_unaligned(); t867[1] = vsetq_lane_u32::<1>(v as u32, t867[1]); t867[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t867[3]); }
        { let v = (((vgetq_lane_u32::<2>(t866[3]) as u64) << 32 | vgetq_lane_u32::<2>(t866[1]) as u64) as usize as *const u64).read_unaligned(); t867[1] = vsetq_lane_u32::<2>(v as u32, t867[1]); t867[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t867[3]); }
        { let v = (((vgetq_lane_u32::<3>(t866[3]) as u64) << 32 | vgetq_lane_u32::<3>(t866[1]) as u64) as usize as *const u64).read_unaligned(); t867[1] = vsetq_lane_u32::<3>(v as u32, t867[1]); t867[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t867[3]); }
        let t868: [uint32x4_t; 2] = [vsubq_u32(t856[0], x4[0]), vsubq_u32(t856[1], x4[1])];
        let t869: [uint32x4_t; 2] = [t868[0], t868[1]];
        let t870: [uint32x4_t; 2] = [vandq_u32(t869[0], vdupq_n_u32(63)), vandq_u32(t869[1], vdupq_n_u32(63))];
        let t871: [uint32x4_t; 4] = [vshlq_u32(t867[0], vreinterpretq_s32_u32(t870[0])), vshlq_u32(t867[1], vreinterpretq_s32_u32(t870[1])), vorrq_u32(vshlq_u32(t867[2], vreinterpretq_s32_u32(t870[0])), vshlq_u32(t867[0], vsubq_s32(vreinterpretq_s32_u32(t870[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t867[3], vreinterpretq_s32_u32(t870[1])), vshlq_u32(t867[1], vsubq_s32(vreinterpretq_s32_u32(t870[1]), vdupq_n_s32(32))))];
        let t872: [uint32x4_t; 4] = [vorrq_u32(x5[0], t871[0]), vorrq_u32(x5[1], t871[1]), vorrq_u32(x5[2], t871[2]), vorrq_u32(x5[3], t871[3])];
        t859 = [vbslq_u32(t858[0], t872[0], t859[0]), vbslq_u32(t858[1], t872[1], t859[1]), vbslq_u32(t858[0], t872[2], t859[2]), vbslq_u32(t858[1], t872[3], t859[3])];
    }
    {
        t859 = [vbslq_u32(t860[0], x5[0], t859[0]), vbslq_u32(t860[1], x5[1], t859[1]), vbslq_u32(t860[0], x5[2], t859[2]), vbslq_u32(t860[1], x5[3], t859[3])];
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
unsafe fn b7_ss_f(l: &Launch, lid0: u32, x0: u64, x1: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x2: u64 = 0;
    let mut x3: u32 = 0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
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

#[inline(always)]
unsafe fn b7_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], x1: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x2: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x3: u32 = 0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    x2 = x0;
    x3 = 0u32;
    let mut t913 = live;
    loop {
        // Lanes part: each its own rounds.
        let t915 = [vdupq_n_u32(15u32); 2];
        let t916: [uint32x4_t; 2] = [vandq_u32(x1[0], t915[0]), vandq_u32(x1[1], t915[1])];
        let t917 = [vdupq_n_u32(x3); 2];
        let t918: [uint32x4_t; 2] = [vcltq_u32(t917[0], t916[0]), vcltq_u32(t917[1], t916[1])];
        t913 = [vandq_u32(t913[0], t918[0]), vandq_u32(t913[1], t918[1])];
        if !(any(&t913)) { break; }
        let t920: u64 = 31u64;
        let t921 = [vdupq_n_u32((t920) as u32); 2];
        let t922: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x2[0]), vget_low_u32(t921[0]))), vreinterpretq_u32_u64(vmull_high_u32(x2[0], t921[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x2[1]), vget_low_u32(t921[1]))), vreinterpretq_u32_u64(vmull_high_u32(x2[1], t921[1])))];
        let t923: [uint32x4_t; 4] = [vmulq_u32(x2[0], t921[0]), vmulq_u32(x2[1], t921[1]), vmlaq_u32(t922[0], t921[0], x2[2]), vmlaq_u32(t922[1], t921[1], x2[3])];
        let t924: u64 = ((x3) as u64);
        let t925 = [vdupq_n_u32((t924) as u32); 2];
        let t926 = [vdupq_n_u32(((t924) >> 32) as u32); 2];
        let t927: [uint32x4_t; 2] = [vaddq_u32(t923[0], t925[0]), vaddq_u32(t923[1], t925[1])];
        let t928: [uint32x4_t; 2] = [vcltq_u32(t927[0], t923[0]), vcltq_u32(t927[1], t923[1])];
        let t929: [uint32x4_t; 4] = [t927[0], t927[1], vsubq_u32(vaddq_u32(t923[2], t926[0]), t928[0]), vsubq_u32(vaddq_u32(t923[3], t926[1]), t928[1])];
        x2 = [vbslq_u32(t913[0], t929[0], x2[0]), vbslq_u32(t913[1], t929[1], x2[1]), vbslq_u32(t913[0], t929[2], x2[2]), vbslq_u32(t913[1], t929[3], x2[3])];
        let t930: [uint32x4_t; 2] = [vandq_u32(t913[0], live[0]), vandq_u32(t913[1], live[1])];
        x3 = (x3).wrapping_add(2u32);
    }
    res = x2;
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
    let x4: *const u8 = *argv.add(4);
    let arg5: *const u8 = (*argv.add(5) as *const *const u8).read_unaligned();
    let x6: u32 = (*argv.add(6) as *const u32).read_unaligned();
    let mut t931: bool = !(arg5).is_null();
    if t931 {
        let mut t932: bool = ((l.group[0]) >= ((arg5.wrapping_add((((x6) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        if !t932 {
            t932 = ((l.group[1]) >= ((arg5.wrapping_add(((((x6).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        let mut t933: bool = t932;
        if !t933 {
            t933 = ((l.group[2]) >= ((arg5.wrapping_add(((((x6).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
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
            b0_r0r1r2r3sg_f(l, lid0, arg0, arg1, arg2, arg3, x4);
        }
        lid0 += 8;
    }
    if lid0 < n {
        // Lanes part: the group's last threads, those under `n`.
        let t934: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
        let mut live = t934;
        let gid0 = l.group[0] * l.block[0] + lid0;
        'chunk: {
            b0_r0r1r2r3sg_p(l, lid0, live, arg0, arg1, arg2, arg3, x4);
        }
    }
}
