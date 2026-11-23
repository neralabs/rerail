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
            let t167 = bits(&t165);
            if t167.count_ones() <= 2 {
                let mut t168 = t167;
                while t168 != 0 {
                    let j = t168.trailing_zeros() as usize;
                    t168 &= t168 - 1;
                    let t169 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x7);
                    let mut x7_j: u64 = ((t169[8 + j] as u64) << 32 | t169[j] as u64);
                    let t170 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x8);
                    let mut x8_j: u32 = t170[j];
                    let t171 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x12);
                    let mut x12_j: u64 = ((t171[8 + j] as u64) << 32 | t171[j] as u64);
                    let mut x13_j: u32 = x13;
                    loop {
                        if !(((x13_j) < (((x8_j) & (7u32))))) { break; }
                        x12_j = ((x12_j).wrapping_mul(3u64)).wrapping_add(x7_j);
                        let t172 = x13_j;
                        x13_j = (x13_j).wrapping_add(1u32);
                    }
                    let mut t173 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x12);
                    t173[j] = x12_j as u32;
                    t173[8 + j] = (x12_j as u64 >> 32) as u32;
                    x12 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t173);
                }
                break;
            }
            // Lanes part: each its own rounds.
            let t174 = [vdupq_n_u32(7u32); 2];
            let t175: [uint32x4_t; 2] = [vandq_u32(x8[0], t174[0]), vandq_u32(x8[1], t174[1])];
            let t176 = [vdupq_n_u32(x13); 2];
            let t177: [uint32x4_t; 2] = [vcltq_u32(t176[0], t175[0]), vcltq_u32(t176[1], t175[1])];
            t165 = [vandq_u32(t165[0], t177[0]), vandq_u32(t165[1], t177[1])];
            if !(any(&t165)) { break; }
            let t179: u64 = 3u64;
            let t180 = [vdupq_n_u32((t179) as u32); 2];
            let t181: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x12[0]), vget_low_u32(t180[0]))), vreinterpretq_u32_u64(vmull_high_u32(x12[0], t180[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x12[1]), vget_low_u32(t180[1]))), vreinterpretq_u32_u64(vmull_high_u32(x12[1], t180[1])))];
            let t182: [uint32x4_t; 4] = [vmulq_u32(x12[0], t180[0]), vmulq_u32(x12[1], t180[1]), vmlaq_u32(t181[0], t180[0], x12[2]), vmlaq_u32(t181[1], t180[1], x12[3])];
            let t183: [uint32x4_t; 2] = [vaddq_u32(t182[0], x7[0]), vaddq_u32(t182[1], x7[1])];
            let t184: [uint32x4_t; 2] = [vcltq_u32(t183[0], t182[0]), vcltq_u32(t183[1], t182[1])];
            let t185: [uint32x4_t; 4] = [t183[0], t183[1], vsubq_u32(vaddq_u32(t182[2], x7[2]), t184[0]), vsubq_u32(vaddq_u32(t182[3], x7[3]), t184[1])];
            x12 = [vbslq_u32(t165[0], t185[0], x12[0]), vbslq_u32(t165[1], t185[1], x12[1]), vbslq_u32(t165[0], t185[2], x12[2]), vbslq_u32(t165[1], t185[3], x12[3])];
            let t186: [uint32x4_t; 2] = [vandq_u32(t165[0], live[0]), vandq_u32(t165[1], live[1])];
            let t187 = x13;
            x13 = (x13).wrapping_add(1u32);
        }
        let t188 = (arg3 as *const u8).add(((gid0.wrapping_add((19u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t188 as *mut u32).add(0), uint32x4x2_t(x12[0], x12[2]));
            vst2q_u32((t188 as *mut u32).add(8), uint32x4x2_t(x12[1], x12[3]));
        } else {
            let mut t189w = 0u64;
            let t189 = core::hint::black_box(&raw mut t189w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t188 as *mut u64).add(0), t189).write_unaligned(((vgetq_lane_u32::<0>(x12[2]) as u64) << 32 | vgetq_lane_u32::<0>(x12[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t188 as *mut u64).add(1), t189).write_unaligned(((vgetq_lane_u32::<1>(x12[2]) as u64) << 32 | vgetq_lane_u32::<1>(x12[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t188 as *mut u64).add(2), t189).write_unaligned(((vgetq_lane_u32::<2>(x12[2]) as u64) << 32 | vgetq_lane_u32::<2>(x12[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t188 as *mut u64).add(3), t189).write_unaligned(((vgetq_lane_u32::<3>(x12[2]) as u64) << 32 | vgetq_lane_u32::<3>(x12[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t188 as *mut u64).add(4), t189).write_unaligned(((vgetq_lane_u32::<0>(x12[3]) as u64) << 32 | vgetq_lane_u32::<0>(x12[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t188 as *mut u64).add(5), t189).write_unaligned(((vgetq_lane_u32::<1>(x12[3]) as u64) << 32 | vgetq_lane_u32::<1>(x12[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t188 as *mut u64).add(6), t189).write_unaligned(((vgetq_lane_u32::<2>(x12[3]) as u64) << 32 | vgetq_lane_u32::<2>(x12[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t188 as *mut u64).add(7), t189).write_unaligned(((vgetq_lane_u32::<3>(x12[3]) as u64) << 32 | vgetq_lane_u32::<3>(x12[1]) as u64));
        }
        let t190: u64 = a1;
        let t191 = [vdupq_n_u32((t190) as u32); 2];
        let t192 = [vdupq_n_u32(((t190) >> 32) as u32); 2];
        let t193: [uint32x4_t; 2] = [vaddq_u32(x6[0], t191[0]), vaddq_u32(x6[1], t191[1])];
        let t194: [uint32x4_t; 2] = [vcltq_u32(t193[0], x6[0]), vcltq_u32(t193[1], x6[1])];
        let t195: [uint32x4_t; 4] = [t193[0], t193[1], vsubq_u32(vaddq_u32(x6[2], t192[0]), t194[0]), vsubq_u32(vaddq_u32(x6[3], t192[1]), t194[1])];
        let t196: u64 = (a1).wrapping_shl((3u64) as u32);
        let t197 = [vdupq_n_u32((t196) as u32); 2];
        let t198 = [vdupq_n_u32(((t196) >> 32) as u32); 2];
        let t199: [uint32x4_t; 2] = [vaddq_u32(t195[0], t197[0]), vaddq_u32(t195[1], t197[1])];
        let t200: [uint32x4_t; 2] = [vcltq_u32(t199[0], t195[0]), vcltq_u32(t199[1], t195[1])];
        let t201: [uint32x4_t; 4] = [t199[0], t199[1], vsubq_u32(vaddq_u32(t195[2], t198[0]), t200[0]), vsubq_u32(vaddq_u32(t195[3], t198[1]), t200[1])];
        let t202 = (arg3 as *const u8).add(((gid0.wrapping_add((20u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t202 as *mut u32).add(0), uint32x4x2_t(t201[0], t201[2]));
            vst2q_u32((t202 as *mut u32).add(8), uint32x4x2_t(t201[1], t201[3]));
        } else {
            let mut t203w = 0u64;
            let t203 = core::hint::black_box(&raw mut t203w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t202 as *mut u64).add(0), t203).write_unaligned(((vgetq_lane_u32::<0>(t201[2]) as u64) << 32 | vgetq_lane_u32::<0>(t201[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t202 as *mut u64).add(1), t203).write_unaligned(((vgetq_lane_u32::<1>(t201[2]) as u64) << 32 | vgetq_lane_u32::<1>(t201[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t202 as *mut u64).add(2), t203).write_unaligned(((vgetq_lane_u32::<2>(t201[2]) as u64) << 32 | vgetq_lane_u32::<2>(t201[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t202 as *mut u64).add(3), t203).write_unaligned(((vgetq_lane_u32::<3>(t201[2]) as u64) << 32 | vgetq_lane_u32::<3>(t201[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t202 as *mut u64).add(4), t203).write_unaligned(((vgetq_lane_u32::<0>(t201[3]) as u64) << 32 | vgetq_lane_u32::<0>(t201[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t202 as *mut u64).add(5), t203).write_unaligned(((vgetq_lane_u32::<1>(t201[3]) as u64) << 32 | vgetq_lane_u32::<1>(t201[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t202 as *mut u64).add(6), t203).write_unaligned(((vgetq_lane_u32::<2>(t201[3]) as u64) << 32 | vgetq_lane_u32::<2>(t201[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t202 as *mut u64).add(7), t203).write_unaligned(((vgetq_lane_u32::<3>(t201[3]) as u64) << 32 | vgetq_lane_u32::<3>(t201[1]) as u64));
        }
        let t204: u64 = 4294967295u64;
        let t205 = [vdupq_n_u32((t204) as u32); 2];
        let t206: [uint32x4_t; 2] = [vandq_u32(x6[0], t205[0]), vandq_u32(x6[1], t205[1])];
        let t207: u64 = 4294967295u64;
        let t208 = [vdupq_n_u32((t207) as u32); 2];
        let t209: [uint32x4_t; 2] = [vandq_u32(x7[0], t208[0]), vandq_u32(x7[1], t208[1])];
        let t210: [uint32x4_t; 2] = [vceqq_u32(t206[0], t209[0]), vceqq_u32(t206[1], t209[1])];
        let mut t211: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t212: [uint32x4_t; 2] = [vandq_u32(live[0], t210[0]), vandq_u32(live[1], t210[1])];
        let t213: [uint32x4_t; 2] = [vbicq_u32(live[0], t210[0]), vbicq_u32(live[1], t210[1])];
        {
            let t214 = [vdupq_n_u32(1u32); 2];
            t211 = [vbslq_u32(t212[0], t214[0], t211[0]), vbslq_u32(t212[1], t214[1], t211[1])];
        }
        {
            let t215 = [vdupq_n_u32(2u32); 2];
            t211 = [vbslq_u32(t213[0], t215[0], t211[0]), vbslq_u32(t213[1], t215[1], t211[1])];
        }
        let t216: [uint32x4_t; 4] = [t211[0], t211[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        let t217 = (arg3 as *const u8).add(((gid0.wrapping_add((21u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t217 as *mut u32).add(0), uint32x4x2_t(t216[0], t216[2]));
            vst2q_u32((t217 as *mut u32).add(8), uint32x4x2_t(t216[1], t216[3]));
        } else {
            let mut t218w = 0u64;
            let t218 = core::hint::black_box(&raw mut t218w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t217 as *mut u64).add(0), t218).write_unaligned(((vgetq_lane_u32::<0>(t216[2]) as u64) << 32 | vgetq_lane_u32::<0>(t216[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t217 as *mut u64).add(1), t218).write_unaligned(((vgetq_lane_u32::<1>(t216[2]) as u64) << 32 | vgetq_lane_u32::<1>(t216[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t217 as *mut u64).add(2), t218).write_unaligned(((vgetq_lane_u32::<2>(t216[2]) as u64) << 32 | vgetq_lane_u32::<2>(t216[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t217 as *mut u64).add(3), t218).write_unaligned(((vgetq_lane_u32::<3>(t216[2]) as u64) << 32 | vgetq_lane_u32::<3>(t216[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t217 as *mut u64).add(4), t218).write_unaligned(((vgetq_lane_u32::<0>(t216[3]) as u64) << 32 | vgetq_lane_u32::<0>(t216[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t217 as *mut u64).add(5), t218).write_unaligned(((vgetq_lane_u32::<1>(t216[3]) as u64) << 32 | vgetq_lane_u32::<1>(t216[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t217 as *mut u64).add(6), t218).write_unaligned(((vgetq_lane_u32::<2>(t216[3]) as u64) << 32 | vgetq_lane_u32::<2>(t216[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t217 as *mut u64).add(7), t218).write_unaligned(((vgetq_lane_u32::<3>(t216[3]) as u64) << 32 | vgetq_lane_u32::<3>(t216[1]) as u64));
        }
        let t219: [uint32x4_t; 2] = [vtstq_u32(vorrq_u32(x6[0], x6[2]), vorrq_u32(x6[0], x6[2])), vtstq_u32(vorrq_u32(x6[1], x6[3]), vorrq_u32(x6[1], x6[3]))];
        let mut t220: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t221: [uint32x4_t; 2] = [vandq_u32(live[0], t219[0]), vandq_u32(live[1], t219[1])];
        let t222: [uint32x4_t; 2] = [vbicq_u32(live[0], t219[0]), vbicq_u32(live[1], t219[1])];
        {
            let t223 = [vdupq_n_u32(5u32); 2];
            t220 = [vbslq_u32(t221[0], t223[0], t220[0]), vbslq_u32(t221[1], t223[1], t220[1])];
        }
        {
            let t224 = [vdupq_n_u32(6u32); 2];
            t220 = [vbslq_u32(t222[0], t224[0], t220[0]), vbslq_u32(t222[1], t224[1], t220[1])];
        }
        let t225: [uint32x4_t; 4] = [t220[0], t220[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        let t226 = (arg3 as *const u8).add(((gid0.wrapping_add((22u32).wrapping_mul(x10))) as usize) * 8);
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
        let t228 = b4_ll_p(l, lid0, live, x6, x7);
        let t229 = (arg3 as *const u8).add(((gid0.wrapping_add((23u32).wrapping_mul(x10))) as usize) * 8);
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
        let t231: u32;
        if ((((a1) & (1u64))) != (0u64)) {
            t231 = 8u32;
        } else {
            t231 = 4u32;
        }
        x14 = t231;
        let t232 = b5_r0ns_p(l, lid0, live, arg0, gid0.wrapping_mul(x14), x14, 8u32);
        let t233 = (arg3 as *const u8).add(((gid0.wrapping_add((24u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t233 as *mut u32).add(0), uint32x4x2_t(t232[0], t232[2]));
            vst2q_u32((t233 as *mut u32).add(8), uint32x4x2_t(t232[1], t232[3]));
        } else {
            let mut t234w = 0u64;
            let t234 = core::hint::black_box(&raw mut t234w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t233 as *mut u64).add(0), t234).write_unaligned(((vgetq_lane_u32::<0>(t232[2]) as u64) << 32 | vgetq_lane_u32::<0>(t232[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t233 as *mut u64).add(1), t234).write_unaligned(((vgetq_lane_u32::<1>(t232[2]) as u64) << 32 | vgetq_lane_u32::<1>(t232[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t233 as *mut u64).add(2), t234).write_unaligned(((vgetq_lane_u32::<2>(t232[2]) as u64) << 32 | vgetq_lane_u32::<2>(t232[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t233 as *mut u64).add(3), t234).write_unaligned(((vgetq_lane_u32::<3>(t232[2]) as u64) << 32 | vgetq_lane_u32::<3>(t232[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t233 as *mut u64).add(4), t234).write_unaligned(((vgetq_lane_u32::<0>(t232[3]) as u64) << 32 | vgetq_lane_u32::<0>(t232[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t233 as *mut u64).add(5), t234).write_unaligned(((vgetq_lane_u32::<1>(t232[3]) as u64) << 32 | vgetq_lane_u32::<1>(t232[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t233 as *mut u64).add(6), t234).write_unaligned(((vgetq_lane_u32::<2>(t232[3]) as u64) << 32 | vgetq_lane_u32::<2>(t232[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t233 as *mut u64).add(7), t234).write_unaligned(((vgetq_lane_u32::<3>(t232[3]) as u64) << 32 | vgetq_lane_u32::<3>(t232[1]) as u64));
        }
        let t235 = if (gid0.wrapping_add(3u32) & 255u32) <= 248u32 { b6_u8_p(l, lid0, live, gid0.wrapping_add(3u32)) } else { b6_i_p(l, lid0, live, gid0.wrapping_add(3u32)) };
        let t236 = (arg3 as *const u8).add(((gid0.wrapping_add((25u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t236 as *mut u32).add(0), uint32x4x2_t(t235[0], t235[2]));
            vst2q_u32((t236 as *mut u32).add(8), uint32x4x2_t(t235[1], t235[3]));
        } else {
            let mut t237w = 0u64;
            let t237 = core::hint::black_box(&raw mut t237w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t236 as *mut u64).add(0), t237).write_unaligned(((vgetq_lane_u32::<0>(t235[2]) as u64) << 32 | vgetq_lane_u32::<0>(t235[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t236 as *mut u64).add(1), t237).write_unaligned(((vgetq_lane_u32::<1>(t235[2]) as u64) << 32 | vgetq_lane_u32::<1>(t235[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t236 as *mut u64).add(2), t237).write_unaligned(((vgetq_lane_u32::<2>(t235[2]) as u64) << 32 | vgetq_lane_u32::<2>(t235[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t236 as *mut u64).add(3), t237).write_unaligned(((vgetq_lane_u32::<3>(t235[2]) as u64) << 32 | vgetq_lane_u32::<3>(t235[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t236 as *mut u64).add(4), t237).write_unaligned(((vgetq_lane_u32::<0>(t235[3]) as u64) << 32 | vgetq_lane_u32::<0>(t235[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t236 as *mut u64).add(5), t237).write_unaligned(((vgetq_lane_u32::<1>(t235[3]) as u64) << 32 | vgetq_lane_u32::<1>(t235[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t236 as *mut u64).add(6), t237).write_unaligned(((vgetq_lane_u32::<2>(t235[3]) as u64) << 32 | vgetq_lane_u32::<2>(t235[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t236 as *mut u64).add(7), t237).write_unaligned(((vgetq_lane_u32::<3>(t235[3]) as u64) << 32 | vgetq_lane_u32::<3>(t235[1]) as u64));
        }
        let t238: u64 = 3u64;
        let t239 = [vdupq_n_u32((t238) as u32); 2];
        let t240: [uint32x4_t; 2] = [vandq_u32(x6[0], t239[0]), vandq_u32(x6[1], t239[1])];
        let t241: u64 = 0u64;
        let t242 = [vdupq_n_u32((t241) as u32); 2];
        let t243: [uint32x4_t; 2] = [vceqq_u32(t240[0], t242[0]), vceqq_u32(t240[1], t242[1])];
        let mut t244: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t245: [uint32x4_t; 2] = [vandq_u32(live[0], t243[0]), vandq_u32(live[1], t243[1])];
        let t246: [uint32x4_t; 2] = [vbicq_u32(live[0], t243[0]), vbicq_u32(live[1], t243[1])];
        if any(&t245) {
            let t247: [uint32x4_t; 4];
            let t248 = bits(&t245);
            if t248.count_ones() <= 2 {
                let t249 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x7);
                let t250 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x8);
                let mut t251 = [0u32; 16];
                let mut t252 = t248;
                while t252 != 0 {
                    let j = t252.trailing_zeros() as usize;
                    t252 &= t252 - 1;
                    let t253 = b7_ss_f(l, lid0, ((t249[8 + j] as u64) << 32 | t249[j] as u64), t250[j]);
                    let t254: u64 = t253;
                    t251[j] = t254 as u32;
                    t251[8 + j] = (t254 >> 32) as u32;
                }
                t247 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t251);
            } else {
                let t255 = b7_ll_p(l, lid0, t245, x7, x8);
                t247 = t255;
            }
            t244 = [vbslq_u32(t245[0], t247[0], t244[0]), vbslq_u32(t245[1], t247[1], t244[1]), vbslq_u32(t245[0], t247[2], t244[2]), vbslq_u32(t245[1], t247[3], t244[3])];
        }
        {
            t244 = [vbslq_u32(t246[0], x6[0], t244[0]), vbslq_u32(t246[1], x6[1], t244[1]), vbslq_u32(t246[0], x6[2], t244[2]), vbslq_u32(t246[1], x6[3], t244[3])];
        }
        let t256 = (arg3 as *const u8).add(((gid0.wrapping_add((26u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t256 as *mut u32).add(0), uint32x4x2_t(t244[0], t244[2]));
            vst2q_u32((t256 as *mut u32).add(8), uint32x4x2_t(t244[1], t244[3]));
        } else {
            let mut t257w = 0u64;
            let t257 = core::hint::black_box(&raw mut t257w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t256 as *mut u64).add(0), t257).write_unaligned(((vgetq_lane_u32::<0>(t244[2]) as u64) << 32 | vgetq_lane_u32::<0>(t244[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t256 as *mut u64).add(1), t257).write_unaligned(((vgetq_lane_u32::<1>(t244[2]) as u64) << 32 | vgetq_lane_u32::<1>(t244[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t256 as *mut u64).add(2), t257).write_unaligned(((vgetq_lane_u32::<2>(t244[2]) as u64) << 32 | vgetq_lane_u32::<2>(t244[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t256 as *mut u64).add(3), t257).write_unaligned(((vgetq_lane_u32::<3>(t244[2]) as u64) << 32 | vgetq_lane_u32::<3>(t244[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t256 as *mut u64).add(4), t257).write_unaligned(((vgetq_lane_u32::<0>(t244[3]) as u64) << 32 | vgetq_lane_u32::<0>(t244[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t256 as *mut u64).add(5), t257).write_unaligned(((vgetq_lane_u32::<1>(t244[3]) as u64) << 32 | vgetq_lane_u32::<1>(t244[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t256 as *mut u64).add(6), t257).write_unaligned(((vgetq_lane_u32::<2>(t244[3]) as u64) << 32 | vgetq_lane_u32::<2>(t244[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t256 as *mut u64).add(7), t257).write_unaligned(((vgetq_lane_u32::<3>(t244[3]) as u64) << 32 | vgetq_lane_u32::<3>(t244[1]) as u64));
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
        let t258 = (arg0 as *const u8).add(((gid0) as usize) * 8);
        let t2600 = vld2q_u32((t258 as *const u32).add(0));
        let t2601 = vld2q_u32((t258 as *const u32).add(8));
        let t260: [uint32x4_t; 4] = [t2600.0, t2601.0, t2600.1, t2601.1];
        x6 = t260;
        let t261 = (arg1 as *const u8).add(((gid0) as usize) * 8);
        let t2630 = vld2q_u32((t261 as *const u32).add(0));
        let t2631 = vld2q_u32((t261 as *const u32).add(8));
        let t263: [uint32x4_t; 4] = [t2630.0, t2631.0, t2630.1, t2631.1];
        x7 = t263;
        let t264 = (arg2 as *const u32).add((gid0) as usize);
        let t265: [uint32x4_t; 2] = [vld1q_u32(t264.add(0)), vld1q_u32(t264.add(4))];
        x8 = t265;
        let t266 = [vdupq_n_u32(63u32); 2];
        let t267: [uint32x4_t; 2] = [vandq_u32(x8[0], t266[0]), vandq_u32(x8[1], t266[1])];
        x9 = t267;
        x10 = a0;
        let t268: [uint32x4_t; 2] = [vaddq_u32(x6[0], x7[0]), vaddq_u32(x6[1], x7[1])];
        let t269: [uint32x4_t; 2] = [vcltq_u32(t268[0], x6[0]), vcltq_u32(t268[1], x6[1])];
        let t270: [uint32x4_t; 4] = [t268[0], t268[1], vsubq_u32(vaddq_u32(x6[2], x7[2]), t269[0]), vsubq_u32(vaddq_u32(x6[3], x7[3]), t269[1])];
        let t271 = (arg3 as *const u8).add(((gid0.wrapping_add((0u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t271 as *mut u32).add(0), uint32x4x2_t(t270[0], t270[2]));
        vst2q_u32((t271 as *mut u32).add(8), uint32x4x2_t(t270[1], t270[3]));
        let t272: [uint32x4_t; 2] = [vcltq_u32(x6[0], x7[0]), vcltq_u32(x6[1], x7[1])];
        let t273: [uint32x4_t; 4] = [vsubq_u32(x6[0], x7[0]), vsubq_u32(x6[1], x7[1]), vaddq_u32(vsubq_u32(x6[2], x7[2]), t272[0]), vaddq_u32(vsubq_u32(x6[3], x7[3]), t272[1])];
        let t274 = (arg3 as *const u8).add(((gid0.wrapping_add((1u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t274 as *mut u32).add(0), uint32x4x2_t(t273[0], t273[2]));
        vst2q_u32((t274 as *mut u32).add(8), uint32x4x2_t(t273[1], t273[3]));
        let t275: [uint32x4_t; 2] = [x9[0], x9[1]];
        let t276: [uint32x4_t; 2] = [vandq_u32(t275[0], vdupq_n_u32(63)), vandq_u32(t275[1], vdupq_n_u32(63))];
        let t277: [uint32x4_t; 4] = [vshlq_u32(x6[0], vreinterpretq_s32_u32(t276[0])), vshlq_u32(x6[1], vreinterpretq_s32_u32(t276[1])), vorrq_u32(vshlq_u32(x6[2], vreinterpretq_s32_u32(t276[0])), vshlq_u32(x6[0], vsubq_s32(vreinterpretq_s32_u32(t276[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(x6[3], vreinterpretq_s32_u32(t276[1])), vshlq_u32(x6[1], vsubq_s32(vreinterpretq_s32_u32(t276[1]), vdupq_n_s32(32))))];
        let t278 = (arg3 as *const u8).add(((gid0.wrapping_add((2u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t278 as *mut u32).add(0), uint32x4x2_t(t277[0], t277[2]));
        vst2q_u32((t278 as *mut u32).add(8), uint32x4x2_t(t277[1], t277[3]));
        let t279: [uint32x4_t; 2] = [x9[0], x9[1]];
        let t280: [uint32x4_t; 2] = [vandq_u32(t279[0], vdupq_n_u32(63)), vandq_u32(t279[1], vdupq_n_u32(63))];
        let t281: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(x6[0], vnegq_s32(vreinterpretq_s32_u32(t280[0]))), vshlq_u32(x6[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t280[0])))), vorrq_u32(vshlq_u32(x6[1], vnegq_s32(vreinterpretq_s32_u32(t280[1]))), vshlq_u32(x6[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t280[1])))), vshlq_u32(x6[2], vnegq_s32(vreinterpretq_s32_u32(t280[0]))), vshlq_u32(x6[3], vnegq_s32(vreinterpretq_s32_u32(t280[1])))];
        let t282 = (arg3 as *const u8).add(((gid0.wrapping_add((3u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t282 as *mut u32).add(0), uint32x4x2_t(t281[0], t281[2]));
        vst2q_u32((t282 as *mut u32).add(8), uint32x4x2_t(t281[1], t281[3]));
        let t283: [uint32x4_t; 2] = [x9[0], x9[1]];
        let t284: [uint32x4_t; 2] = [vandq_u32(t283[0], vdupq_n_u32(63)), vandq_u32(t283[1], vdupq_n_u32(63))];
        let t285: [uint32x4_t; 2] = [vcgeq_s32(vreinterpretq_s32_u32(t284[0]), vdupq_n_s32(32)), vcgeq_s32(vreinterpretq_s32_u32(t284[1]), vdupq_n_s32(32))];
        let t286: [uint32x4_t; 4] = [vbslq_u32(t285[0], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t284[0])))), vorrq_u32(vshlq_u32(x6[0], vnegq_s32(vreinterpretq_s32_u32(t284[0]))), vshlq_u32(x6[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t284[0]))))), vbslq_u32(t285[1], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t284[1])))), vorrq_u32(vshlq_u32(x6[1], vnegq_s32(vreinterpretq_s32_u32(t284[1]))), vshlq_u32(x6[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t284[1]))))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vnegq_s32(vreinterpretq_s32_u32(t284[0])))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vnegq_s32(vreinterpretq_s32_u32(t284[1]))))];
        let t287 = (arg3 as *const u8).add(((gid0.wrapping_add((4u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t287 as *mut u32).add(0), uint32x4x2_t(t286[0], t286[2]));
        vst2q_u32((t287 as *mut u32).add(8), uint32x4x2_t(t286[1], t286[3]));
        let t288: [uint32x4_t; 4] = [vshlq_n_u32::<13>(x6[0]), vshlq_n_u32::<13>(x6[1]), vorrq_u32(vshlq_n_u32::<13>(x6[2]), vshrq_n_u32::<19>(x6[0])), vorrq_u32(vshlq_n_u32::<13>(x6[3]), vshrq_n_u32::<19>(x6[1]))];
        let t289: [uint32x4_t; 2] = [vshrq_n_u32::<19>(x7[2]), vshrq_n_u32::<19>(x7[3])];
        let t290: [uint32x4_t; 4] = [vorrq_u32(t288[0], t289[0]), vorrq_u32(t288[1], t289[1]), t288[2], t288[3]];
        let t291 = (arg3 as *const u8).add(((gid0.wrapping_add((5u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t291 as *mut u32).add(0), uint32x4x2_t(t290[0], t290[2]));
        vst2q_u32((t291 as *mut u32).add(8), uint32x4x2_t(t290[1], t290[3]));
        let t292: [uint32x4_t; 4] = [vdupq_n_u32(0), vdupq_n_u32(0), vshlq_n_u32::<8>(x6[0]), vshlq_n_u32::<8>(x6[1])];
        let t293 = (arg3 as *const u8).add(((gid0.wrapping_add((6u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t293 as *mut u32).add(0), uint32x4x2_t(t292[0], t292[2]));
        vst2q_u32((t293 as *mut u32).add(8), uint32x4x2_t(t292[1], t292[3]));
        let t294: [uint32x4_t; 2] = [vshrq_n_u32::<3>(x6[2]), vshrq_n_u32::<3>(x6[3])];
        let t295: [uint32x4_t; 4] = [t294[0], t294[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        let t296 = (arg3 as *const u8).add(((gid0.wrapping_add((7u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t296 as *mut u32).add(0), uint32x4x2_t(t295[0], t295[2]));
        vst2q_u32((t296 as *mut u32).add(8), uint32x4x2_t(t295[1], t295[3]));
        let t297 = vdupq_n_s32(((((40i64) as u32)) & 63) as i32);
        let t298: [uint32x4_t; 2] = [vcgeq_s32(t297, vdupq_n_s32(32)), vcgeq_s32(t297, vdupq_n_s32(32))];
        let t299: [uint32x4_t; 4] = [vbslq_u32(t298[0], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vsubq_s32(vdupq_n_s32(32), t297))), vorrq_u32(vshlq_u32(x6[0], vnegq_s32(t297)), vshlq_u32(x6[2], vsubq_s32(vdupq_n_s32(32), t297)))), vbslq_u32(t298[1], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vsubq_s32(vdupq_n_s32(32), t297))), vorrq_u32(vshlq_u32(x6[1], vnegq_s32(t297)), vshlq_u32(x6[3], vsubq_s32(vdupq_n_s32(32), t297)))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vnegq_s32(t297))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vnegq_s32(t297)))];
        let t300 = (arg3 as *const u8).add(((gid0.wrapping_add((8u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t300 as *mut u32).add(0), uint32x4x2_t(t299[0], t299[2]));
        vst2q_u32((t300 as *mut u32).add(8), uint32x4x2_t(t299[1], t299[3]));
        let t301: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x6[0]), vget_low_u32(x7[0]))), vreinterpretq_u32_u64(vmull_high_u32(x6[0], x7[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x6[1]), vget_low_u32(x7[1]))), vreinterpretq_u32_u64(vmull_high_u32(x6[1], x7[1])))];
        let t302: [uint32x4_t; 4] = [vmulq_u32(x6[0], x7[0]), vmulq_u32(x6[1], x7[1]), vmlaq_u32(vmlaq_u32(t301[0], x6[0], x7[2]), x7[0], x6[2]), vmlaq_u32(vmlaq_u32(t301[1], x6[1], x7[3]), x7[1], x6[3])];
        let t303 = (arg3 as *const u8).add(((gid0.wrapping_add((9u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t303 as *mut u32).add(0), uint32x4x2_t(t302[0], t302[2]));
        vst2q_u32((t303 as *mut u32).add(8), uint32x4x2_t(t302[1], t302[3]));
        let t304: u64 = 0u64;
        let t305 = [vdupq_n_u32((t304) as u32); 2];
        let t306: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x7[0], t305[0]), vceqq_u32(x7[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(x7[1], t305[1]), vceqq_u32(x7[3], vdupq_n_u32(0)))];
        let t307: [uint32x4_t; 2] = [vmvnq_u32(t306[0]), vmvnq_u32(t306[1])];
        let mut t308: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t309: [uint32x4_t; 2] = [vmvnq_u32(t307[0]), vmvnq_u32(t307[1])];
        {
            let (mut t310xl, mut t310xh, mut t310yl, mut t310yh) = ([0u32; 8], [0u32; 8], [0u32; 8], [0u32; 8]);
            vst1q_u32(t310xl.as_mut_ptr().add(0), x6[0]); vst1q_u32(t310xh.as_mut_ptr().add(0), x6[2]); vst1q_u32(t310yl.as_mut_ptr().add(0), x7[0]); vst1q_u32(t310yh.as_mut_ptr().add(0), x7[2]);
            vst1q_u32(t310xl.as_mut_ptr().add(4), x6[1]); vst1q_u32(t310xh.as_mut_ptr().add(4), x6[3]); vst1q_u32(t310yl.as_mut_ptr().add(4), x7[1]); vst1q_u32(t310yh.as_mut_ptr().add(4), x7[3]);
            for j in 0..8 { let q = (((t310xh[j] as u64) << 32 | t310xl[j] as u64) as u64).checked_div(((t310yh[j] as u64) << 32 | t310yl[j] as u64) as u64).unwrap_or(0) as u64; t310xl[j] = q as u32; t310xh[j] = (q >> 32) as u32; }
            let t311: [uint32x4_t; 4] = [vld1q_u32(t310xl.as_ptr().add(0)), vld1q_u32(t310xl.as_ptr().add(4)), vld1q_u32(t310xh.as_ptr().add(0)), vld1q_u32(t310xh.as_ptr().add(4))];
            t308 = [vbslq_u32(t307[0], t311[0], t308[0]), vbslq_u32(t307[1], t311[1], t308[1]), vbslq_u32(t307[0], t311[2], t308[2]), vbslq_u32(t307[1], t311[3], t308[3])];
        }
        {
            let t312: u64 = 7u64;
            let t313 = [vdupq_n_u32((t312) as u32); 2];
            let t314: [uint32x4_t; 4] = [t313[0], t313[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            t308 = [vbslq_u32(t309[0], t314[0], t308[0]), vbslq_u32(t309[1], t314[1], t308[1]), vbslq_u32(t309[0], t314[2], t308[2]), vbslq_u32(t309[1], t314[3], t308[3])];
        }
        let t315 = (arg3 as *const u8).add(((gid0.wrapping_add((10u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t315 as *mut u32).add(0), uint32x4x2_t(t308[0], t308[2]));
        vst2q_u32((t315 as *mut u32).add(8), uint32x4x2_t(t308[1], t308[3]));
        let t316: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x6[2], x7[2]), vandq_u32(vceqq_u32(x6[2], x7[2]), vcltq_u32(x6[0], x7[0]))), vorrq_u32(vcltq_u32(x6[3], x7[3]), vandq_u32(vceqq_u32(x6[3], x7[3]), vcltq_u32(x6[1], x7[1])))];
        let t317: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t316[0]), vshrq_n_u32::<31>(t316[1])];
        let t318: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x7[2], x6[2]), vandq_u32(vceqq_u32(x7[2], x6[2]), vcltq_u32(x7[0], x6[0]))), vorrq_u32(vcltq_u32(x7[3], x6[3]), vandq_u32(vceqq_u32(x7[3], x6[3]), vcltq_u32(x7[1], x6[1])))];
        let t319: [uint32x4_t; 2] = [vmvnq_u32(t318[0]), vmvnq_u32(t318[1])];
        let t320: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t319[0]), vshrq_n_u32::<31>(t319[1])];
        let t321: [uint32x4_t; 4] = [vshlq_n_u32::<1>(t320[0]), vshlq_n_u32::<1>(t320[1]), vshrq_n_u32::<31>(t320[0]), vshrq_n_u32::<31>(t320[1])];
        let t322: [uint32x4_t; 4] = [vorrq_u32(t317[0], t321[0]), vorrq_u32(t317[1], t321[1]), t321[2], t321[3]];
        let t323: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x6[0], x7[0]), vceqq_u32(x6[2], x7[2])), vandq_u32(vceqq_u32(x6[1], x7[1]), vceqq_u32(x6[3], x7[3]))];
        let t324: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t323[0]), vshrq_n_u32::<31>(t323[1])];
        let t325: [uint32x4_t; 4] = [vshlq_n_u32::<2>(t324[0]), vshlq_n_u32::<2>(t324[1]), vshrq_n_u32::<30>(t324[0]), vshrq_n_u32::<30>(t324[1])];
        let t326: [uint32x4_t; 4] = [vorrq_u32(t322[0], t325[0]), vorrq_u32(t322[1], t325[1]), vorrq_u32(t322[2], t325[2]), vorrq_u32(t322[3], t325[3])];
        let t327: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x6[0], x7[0]), vceqq_u32(x6[2], x7[2])), vandq_u32(vceqq_u32(x6[1], x7[1]), vceqq_u32(x6[3], x7[3]))];
        let t328: [uint32x4_t; 2] = [vmvnq_u32(t327[0]), vmvnq_u32(t327[1])];
        let t329: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t328[0]), vshrq_n_u32::<31>(t328[1])];
        let t330: [uint32x4_t; 4] = [vshlq_n_u32::<3>(t329[0]), vshlq_n_u32::<3>(t329[1]), vshrq_n_u32::<29>(t329[0]), vshrq_n_u32::<29>(t329[1])];
        let t331: [uint32x4_t; 4] = [vorrq_u32(t326[0], t330[0]), vorrq_u32(t326[1], t330[1]), vorrq_u32(t326[2], t330[2]), vorrq_u32(t326[3], t330[3])];
        let t332: [uint32x4_t; 2] = [vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x6[2]), vreinterpretq_s32_u32(x7[2])), vandq_u32(vceqq_u32(x6[2], x7[2]), vcltq_u32(x6[0], x7[0]))), vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x6[3]), vreinterpretq_s32_u32(x7[3])), vandq_u32(vceqq_u32(x6[3], x7[3]), vcltq_u32(x6[1], x7[1])))];
        let t333: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t332[0]), vshrq_n_u32::<31>(t332[1])];
        let t334: [uint32x4_t; 4] = [vshlq_n_u32::<4>(t333[0]), vshlq_n_u32::<4>(t333[1]), vshrq_n_u32::<28>(t333[0]), vshrq_n_u32::<28>(t333[1])];
        let t335: [uint32x4_t; 4] = [vorrq_u32(t331[0], t334[0]), vorrq_u32(t331[1], t334[1]), vorrq_u32(t331[2], t334[2]), vorrq_u32(t331[3], t334[3])];
        let t336: [uint32x4_t; 2] = [vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x6[2]), vreinterpretq_s32_u32(x7[2])), vandq_u32(vceqq_u32(x6[2], x7[2]), vcltq_u32(x6[0], x7[0]))), vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x6[3]), vreinterpretq_s32_u32(x7[3])), vandq_u32(vceqq_u32(x6[3], x7[3]), vcltq_u32(x6[1], x7[1])))];
        let t337: [uint32x4_t; 2] = [vmvnq_u32(t336[0]), vmvnq_u32(t336[1])];
        let t338: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t337[0]), vshrq_n_u32::<31>(t337[1])];
        let t339: [uint32x4_t; 4] = [vshlq_n_u32::<5>(t338[0]), vshlq_n_u32::<5>(t338[1]), vshrq_n_u32::<27>(t338[0]), vshrq_n_u32::<27>(t338[1])];
        let t340: [uint32x4_t; 4] = [vorrq_u32(t335[0], t339[0]), vorrq_u32(t335[1], t339[1]), vorrq_u32(t335[2], t339[2]), vorrq_u32(t335[3], t339[3])];
        let t341: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x7[2], x6[2]), vandq_u32(vceqq_u32(x7[2], x6[2]), vcltq_u32(x7[0], x6[0]))), vorrq_u32(vcltq_u32(x7[3], x6[3]), vandq_u32(vceqq_u32(x7[3], x6[3]), vcltq_u32(x7[1], x6[1])))];
        let t342: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t341[0]), vshrq_n_u32::<31>(t341[1])];
        let t343: [uint32x4_t; 4] = [vshlq_n_u32::<6>(t342[0]), vshlq_n_u32::<6>(t342[1]), vshrq_n_u32::<26>(t342[0]), vshrq_n_u32::<26>(t342[1])];
        let t344: [uint32x4_t; 4] = [vorrq_u32(t340[0], t343[0]), vorrq_u32(t340[1], t343[1]), vorrq_u32(t340[2], t343[2]), vorrq_u32(t340[3], t343[3])];
        let t345 = (arg3 as *const u8).add(((gid0.wrapping_add((11u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t345 as *mut u32).add(0), uint32x4x2_t(t344[0], t344[2]));
        vst2q_u32((t345 as *mut u32).add(8), uint32x4x2_t(t344[1], t344[3]));
        let t346: [uint32x4_t; 2] = [vaddq_u32(x8[0], x6[0]), vaddq_u32(x8[1], x6[1])];
        let t347: [uint32x4_t; 2] = [vcltq_u32(t346[0], x8[0]), vcltq_u32(t346[1], x8[1])];
        let t348: [uint32x4_t; 4] = [t346[0], t346[1], vsubq_u32(vaddq_u32(vdupq_n_u32(0), x6[2]), t347[0]), vsubq_u32(vaddq_u32(vdupq_n_u32(0), x6[3]), t347[1])];
        let t349 = (arg3 as *const u8).add(((gid0.wrapping_add((12u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t349 as *mut u32).add(0), uint32x4x2_t(t348[0], t348[2]));
        vst2q_u32((t349 as *mut u32).add(8), uint32x4x2_t(t348[1], t348[3]));
        let t350: [uint32x4_t; 2] = [vreinterpretq_u32_s32(vshrq_n_s32::<31>(vreinterpretq_s32_u32(x8[0]))), vreinterpretq_u32_s32(vshrq_n_s32::<31>(vreinterpretq_s32_u32(x8[1])))];
        let t351: [uint32x4_t; 4] = [x8[0], x8[1], t350[0], t350[1]];
        let t352 = (arg3 as *const u8).add(((gid0.wrapping_add((13u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t352 as *mut u32).add(0), uint32x4x2_t(t351[0], t351[2]));
        vst2q_u32((t352 as *mut u32).add(8), uint32x4x2_t(t351[1], t351[3]));
        let t353: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<3>(x6[0]), vshlq_n_u32::<29>(x6[2])), vorrq_u32(vshrq_n_u32::<3>(x6[1]), vshlq_n_u32::<29>(x6[3])), vshrq_n_u32::<3>(x6[2]), vshrq_n_u32::<3>(x6[3])];
        let t354: [uint32x4_t; 2] = [t353[0], t353[1]];
        let t355: [uint32x4_t; 4] = [t354[0], t354[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        let t356 = (arg3 as *const u8).add(((gid0.wrapping_add((14u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t356 as *mut u32).add(0), uint32x4x2_t(t355[0], t355[2]));
        vst2q_u32((t356 as *mut u32).add(8), uint32x4x2_t(t355[1], t355[3]));
        let t357: u64 = 1u64;
        let t358 = [vdupq_n_u32((t357) as u32); 2];
        let t359: [uint32x4_t; 2] = [vandq_u32(x6[0], t358[0]), vandq_u32(x6[1], t358[1])];
        let t360: [uint32x4_t; 2] = [vtstq_u32(t359[0], t359[0]), vtstq_u32(t359[1], t359[1])];
        {
            let t361: [uint32x4_t; 4] = [veorq_u32(x6[0], x7[0]), veorq_u32(x6[1], x7[1]), veorq_u32(x6[2], x7[2]), veorq_u32(x6[3], x7[3])];
            x11 = [vbslq_u32(t360[0], t361[0], x11[0]), vbslq_u32(t360[1], t361[1], x11[1]), vbslq_u32(t360[0], t361[2], x11[2]), vbslq_u32(t360[1], t361[3], x11[3])];
        }
        let t362: [uint32x4_t; 2] = [vmvnq_u32(t360[0]), vmvnq_u32(t360[1])];
        {
            let t363: [uint32x4_t; 2] = [vaddq_u32(x6[0], x8[0]), vaddq_u32(x6[1], x8[1])];
            let t364: [uint32x4_t; 2] = [vcltq_u32(t363[0], x6[0]), vcltq_u32(t363[1], x6[1])];
            let t365: [uint32x4_t; 4] = [t363[0], t363[1], vsubq_u32(vaddq_u32(x6[2], vdupq_n_u32(0)), t364[0]), vsubq_u32(vaddq_u32(x6[3], vdupq_n_u32(0)), t364[1])];
            x11 = [vbslq_u32(t362[0], t365[0], x11[0]), vbslq_u32(t362[1], t365[1], x11[1]), vbslq_u32(t362[0], t365[2], x11[2]), vbslq_u32(t362[1], t365[3], x11[3])];
        }
        let t366 = (arg3 as *const u8).add(((gid0.wrapping_add((15u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t366 as *mut u32).add(0), uint32x4x2_t(x11[0], x11[2]));
        vst2q_u32((t366 as *mut u32).add(8), uint32x4x2_t(x11[1], x11[3]));
        let t367 = b1_l_f(l, lid0, x6);
        let t368 = b2_l_f(l, lid0, x7);
        let t369: [uint32x4_t; 4] = [vshlq_n_u32::<8>(t368[0]), vshlq_n_u32::<8>(t368[1]), vshrq_n_u32::<24>(t368[0]), vshrq_n_u32::<24>(t368[1])];
        let t370: [uint32x4_t; 2] = [vaddq_u32(t367[0], t369[0]), vaddq_u32(t367[1], t369[1])];
        let t371: [uint32x4_t; 2] = [vcltq_u32(t370[0], t367[0]), vcltq_u32(t370[1], t367[1])];
        let t372: [uint32x4_t; 4] = [t370[0], t370[1], vsubq_u32(vaddq_u32(vdupq_n_u32(0), t369[2]), t371[0]), vsubq_u32(vaddq_u32(vdupq_n_u32(0), t369[3]), t371[1])];
        let t373 = (arg3 as *const u8).add(((gid0.wrapping_add((16u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t373 as *mut u32).add(0), uint32x4x2_t(t372[0], t372[2]));
        vst2q_u32((t373 as *mut u32).add(8), uint32x4x2_t(t372[1], t372[3]));
        let t374 = [vdupq_n_u32(x10); 2];
        let mut t375x = [0u32; 8];
        let mut t375y = [0u32; 8];
        for r in 0..2 { vst1q_u32(t375x.as_mut_ptr().add(4 * r), x8[r]); vst1q_u32(t375y.as_mut_ptr().add(4 * r), t374[r]); }
        for j in 0..8 { t375x[j] = (t375x[j] as u32).checked_rem(t375y[j] as u32).unwrap_or(0) as u32; }
        let t376: [uint32x4_t; 2] = [vld1q_u32(t375x.as_ptr().add(0)), vld1q_u32(t375x.as_ptr().add(4))];
        let mut t377: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t376[0]) as usize * 8) as *const u64).read_unaligned(); t377[0] = vsetq_lane_u32::<0>(v as u32, t377[0]); t377[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t377[2]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t376[0]) as usize * 8) as *const u64).read_unaligned(); t377[0] = vsetq_lane_u32::<1>(v as u32, t377[0]); t377[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t377[2]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t376[0]) as usize * 8) as *const u64).read_unaligned(); t377[0] = vsetq_lane_u32::<2>(v as u32, t377[0]); t377[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t377[2]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t376[0]) as usize * 8) as *const u64).read_unaligned(); t377[0] = vsetq_lane_u32::<3>(v as u32, t377[0]); t377[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t377[2]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t376[1]) as usize * 8) as *const u64).read_unaligned(); t377[1] = vsetq_lane_u32::<0>(v as u32, t377[1]); t377[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t377[3]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t376[1]) as usize * 8) as *const u64).read_unaligned(); t377[1] = vsetq_lane_u32::<1>(v as u32, t377[1]); t377[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t377[3]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t376[1]) as usize * 8) as *const u64).read_unaligned(); t377[1] = vsetq_lane_u32::<2>(v as u32, t377[1]); t377[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t377[3]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t376[1]) as usize * 8) as *const u64).read_unaligned(); t377[1] = vsetq_lane_u32::<3>(v as u32, t377[1]); t377[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t377[3]); }
        let t378 = (arg3 as *const u8).add(((gid0.wrapping_add((17u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t378 as *mut u32).add(0), uint32x4x2_t(t377[0], t377[2]));
        vst2q_u32((t378 as *mut u32).add(8), uint32x4x2_t(t377[1], t377[3]));
        let t379 = b3_ll_f(l, lid0, x6, x8);
        let t380 = (arg3 as *const u8).add(((gid0.wrapping_add((18u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t380 as *mut u32).add(0), uint32x4x2_t(t379[0], t379[2]));
        vst2q_u32((t380 as *mut u32).add(8), uint32x4x2_t(t379[1], t379[3]));
        x12 = x6;
        x13 = 0u32;
        let mut t389 = live;
        loop {
            let t391 = bits(&t389);
            if t391.count_ones() <= 2 {
                let mut t392 = t391;
                while t392 != 0 {
                    let j = t392.trailing_zeros() as usize;
                    t392 &= t392 - 1;
                    let t393 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x7);
                    let mut x7_j: u64 = ((t393[8 + j] as u64) << 32 | t393[j] as u64);
                    let t394 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x8);
                    let mut x8_j: u32 = t394[j];
                    let t395 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x12);
                    let mut x12_j: u64 = ((t395[8 + j] as u64) << 32 | t395[j] as u64);
                    let mut x13_j: u32 = x13;
                    loop {
                        if !(((x13_j) < (((x8_j) & (7u32))))) { break; }
                        x12_j = ((x12_j).wrapping_mul(3u64)).wrapping_add(x7_j);
                        let t396 = x13_j;
                        x13_j = (x13_j).wrapping_add(1u32);
                    }
                    let mut t397 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x12);
                    t397[j] = x12_j as u32;
                    t397[8 + j] = (x12_j as u64 >> 32) as u32;
                    x12 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t397);
                }
                break;
            }
            // Lanes part: each its own rounds.
            let t398 = [vdupq_n_u32(7u32); 2];
            let t399: [uint32x4_t; 2] = [vandq_u32(x8[0], t398[0]), vandq_u32(x8[1], t398[1])];
            let t400 = [vdupq_n_u32(x13); 2];
            let t401: [uint32x4_t; 2] = [vcltq_u32(t400[0], t399[0]), vcltq_u32(t400[1], t399[1])];
            t389 = [vandq_u32(t389[0], t401[0]), vandq_u32(t389[1], t401[1])];
            if !(any(&t389)) { break; }
            let t403: u64 = 3u64;
            let t404 = [vdupq_n_u32((t403) as u32); 2];
            let t405: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x12[0]), vget_low_u32(t404[0]))), vreinterpretq_u32_u64(vmull_high_u32(x12[0], t404[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x12[1]), vget_low_u32(t404[1]))), vreinterpretq_u32_u64(vmull_high_u32(x12[1], t404[1])))];
            let t406: [uint32x4_t; 4] = [vmulq_u32(x12[0], t404[0]), vmulq_u32(x12[1], t404[1]), vmlaq_u32(t405[0], t404[0], x12[2]), vmlaq_u32(t405[1], t404[1], x12[3])];
            let t407: [uint32x4_t; 2] = [vaddq_u32(t406[0], x7[0]), vaddq_u32(t406[1], x7[1])];
            let t408: [uint32x4_t; 2] = [vcltq_u32(t407[0], t406[0]), vcltq_u32(t407[1], t406[1])];
            let t409: [uint32x4_t; 4] = [t407[0], t407[1], vsubq_u32(vaddq_u32(t406[2], x7[2]), t408[0]), vsubq_u32(vaddq_u32(t406[3], x7[3]), t408[1])];
            x12 = [vbslq_u32(t389[0], t409[0], x12[0]), vbslq_u32(t389[1], t409[1], x12[1]), vbslq_u32(t389[0], t409[2], x12[2]), vbslq_u32(t389[1], t409[3], x12[3])];
            let t410: [uint32x4_t; 2] = [vandq_u32(t389[0], live[0]), vandq_u32(t389[1], live[1])];
            let t411 = x13;
            x13 = (x13).wrapping_add(1u32);
        }
        let t412 = (arg3 as *const u8).add(((gid0.wrapping_add((19u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t412 as *mut u32).add(0), uint32x4x2_t(x12[0], x12[2]));
        vst2q_u32((t412 as *mut u32).add(8), uint32x4x2_t(x12[1], x12[3]));
        let t413: u64 = a1;
        let t414 = [vdupq_n_u32((t413) as u32); 2];
        let t415 = [vdupq_n_u32(((t413) >> 32) as u32); 2];
        let t416: [uint32x4_t; 2] = [vaddq_u32(x6[0], t414[0]), vaddq_u32(x6[1], t414[1])];
        let t417: [uint32x4_t; 2] = [vcltq_u32(t416[0], x6[0]), vcltq_u32(t416[1], x6[1])];
        let t418: [uint32x4_t; 4] = [t416[0], t416[1], vsubq_u32(vaddq_u32(x6[2], t415[0]), t417[0]), vsubq_u32(vaddq_u32(x6[3], t415[1]), t417[1])];
        let t419: u64 = (a1).wrapping_shl((3u64) as u32);
        let t420 = [vdupq_n_u32((t419) as u32); 2];
        let t421 = [vdupq_n_u32(((t419) >> 32) as u32); 2];
        let t422: [uint32x4_t; 2] = [vaddq_u32(t418[0], t420[0]), vaddq_u32(t418[1], t420[1])];
        let t423: [uint32x4_t; 2] = [vcltq_u32(t422[0], t418[0]), vcltq_u32(t422[1], t418[1])];
        let t424: [uint32x4_t; 4] = [t422[0], t422[1], vsubq_u32(vaddq_u32(t418[2], t421[0]), t423[0]), vsubq_u32(vaddq_u32(t418[3], t421[1]), t423[1])];
        let t425 = (arg3 as *const u8).add(((gid0.wrapping_add((20u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t425 as *mut u32).add(0), uint32x4x2_t(t424[0], t424[2]));
        vst2q_u32((t425 as *mut u32).add(8), uint32x4x2_t(t424[1], t424[3]));
        let t426: u64 = 4294967295u64;
        let t427 = [vdupq_n_u32((t426) as u32); 2];
        let t428: [uint32x4_t; 2] = [vandq_u32(x6[0], t427[0]), vandq_u32(x6[1], t427[1])];
        let t429: u64 = 4294967295u64;
        let t430 = [vdupq_n_u32((t429) as u32); 2];
        let t431: [uint32x4_t; 2] = [vandq_u32(x7[0], t430[0]), vandq_u32(x7[1], t430[1])];
        let t432: [uint32x4_t; 2] = [vceqq_u32(t428[0], t431[0]), vceqq_u32(t428[1], t431[1])];
        let mut t433: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t434: [uint32x4_t; 2] = [vmvnq_u32(t432[0]), vmvnq_u32(t432[1])];
        {
            let t435 = [vdupq_n_u32(1u32); 2];
            t433 = [vbslq_u32(t432[0], t435[0], t433[0]), vbslq_u32(t432[1], t435[1], t433[1])];
        }
        {
            let t436 = [vdupq_n_u32(2u32); 2];
            t433 = [vbslq_u32(t434[0], t436[0], t433[0]), vbslq_u32(t434[1], t436[1], t433[1])];
        }
        let t437: [uint32x4_t; 4] = [t433[0], t433[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        let t438 = (arg3 as *const u8).add(((gid0.wrapping_add((21u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t438 as *mut u32).add(0), uint32x4x2_t(t437[0], t437[2]));
        vst2q_u32((t438 as *mut u32).add(8), uint32x4x2_t(t437[1], t437[3]));
        let t439: [uint32x4_t; 2] = [vtstq_u32(vorrq_u32(x6[0], x6[2]), vorrq_u32(x6[0], x6[2])), vtstq_u32(vorrq_u32(x6[1], x6[3]), vorrq_u32(x6[1], x6[3]))];
        let mut t440: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t441: [uint32x4_t; 2] = [vmvnq_u32(t439[0]), vmvnq_u32(t439[1])];
        {
            let t442 = [vdupq_n_u32(5u32); 2];
            t440 = [vbslq_u32(t439[0], t442[0], t440[0]), vbslq_u32(t439[1], t442[1], t440[1])];
        }
        {
            let t443 = [vdupq_n_u32(6u32); 2];
            t440 = [vbslq_u32(t441[0], t443[0], t440[0]), vbslq_u32(t441[1], t443[1], t440[1])];
        }
        let t444: [uint32x4_t; 4] = [t440[0], t440[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        let t445 = (arg3 as *const u8).add(((gid0.wrapping_add((22u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t445 as *mut u32).add(0), uint32x4x2_t(t444[0], t444[2]));
        vst2q_u32((t445 as *mut u32).add(8), uint32x4x2_t(t444[1], t444[3]));
        let t446 = b4_ll_f(l, lid0, x6, x7);
        let t447 = (arg3 as *const u8).add(((gid0.wrapping_add((23u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t447 as *mut u32).add(0), uint32x4x2_t(t446[0], t446[2]));
        vst2q_u32((t447 as *mut u32).add(8), uint32x4x2_t(t446[1], t446[3]));
        let t448: u32;
        if ((((a1) & (1u64))) != (0u64)) {
            t448 = 8u32;
        } else {
            t448 = 4u32;
        }
        x14 = t448;
        let t449 = b5_r0ns_f_v0(l, lid0, arg0, gid0.wrapping_mul(x14), x14, 8u32);
        let t450 = (arg3 as *const u8).add(((gid0.wrapping_add((24u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t450 as *mut u32).add(0), uint32x4x2_t(t449[0], t449[2]));
        vst2q_u32((t450 as *mut u32).add(8), uint32x4x2_t(t449[1], t449[3]));
        let t451 = if (gid0.wrapping_add(3u32) & 255u32) <= 248u32 { b6_u8_f(l, lid0, gid0.wrapping_add(3u32)) } else { b6_i_f(l, lid0, gid0.wrapping_add(3u32)) };
        let t452 = (arg3 as *const u8).add(((gid0.wrapping_add((25u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t452 as *mut u32).add(0), uint32x4x2_t(t451[0], t451[2]));
        vst2q_u32((t452 as *mut u32).add(8), uint32x4x2_t(t451[1], t451[3]));
        let t453: u64 = 3u64;
        let t454 = [vdupq_n_u32((t453) as u32); 2];
        let t455: [uint32x4_t; 2] = [vandq_u32(x6[0], t454[0]), vandq_u32(x6[1], t454[1])];
        let t456: u64 = 0u64;
        let t457 = [vdupq_n_u32((t456) as u32); 2];
        let t458: [uint32x4_t; 2] = [vceqq_u32(t455[0], t457[0]), vceqq_u32(t455[1], t457[1])];
        let mut t459: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t460: [uint32x4_t; 2] = [vmvnq_u32(t458[0]), vmvnq_u32(t458[1])];
        if any(&t458) {
            let t461: [uint32x4_t; 4];
            let t462 = bits(&t458);
            if t462.count_ones() <= 2 {
                let t463 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x7);
                let t464 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x8);
                let mut t465 = [0u32; 16];
                let mut t466 = t462;
                while t466 != 0 {
                    let j = t466.trailing_zeros() as usize;
                    t466 &= t466 - 1;
                    let t467 = b7_ss_f(l, lid0, ((t463[8 + j] as u64) << 32 | t463[j] as u64), t464[j]);
                    let t468: u64 = t467;
                    t465[j] = t468 as u32;
                    t465[8 + j] = (t468 >> 32) as u32;
                }
                t461 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t465);
            } else {
                let t469 = b7_ll_p(l, lid0, t458, x7, x8);
                t461 = t469;
            }
            t459 = [vbslq_u32(t458[0], t461[0], t459[0]), vbslq_u32(t458[1], t461[1], t459[1]), vbslq_u32(t458[0], t461[2], t459[2]), vbslq_u32(t458[1], t461[3], t459[3])];
        }
        {
            t459 = [vbslq_u32(t460[0], x6[0], t459[0]), vbslq_u32(t460[1], x6[1], t459[1]), vbslq_u32(t460[0], x6[2], t459[2]), vbslq_u32(t460[1], x6[3], t459[3])];
        }
        let t470 = (arg3 as *const u8).add(((gid0.wrapping_add((26u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t470 as *mut u32).add(0), uint32x4x2_t(t459[0], t459[2]));
        vst2q_u32((t470 as *mut u32).add(8), uint32x4x2_t(t459[1], t459[3]));
    } else if (((gid0) as u64 + 7) <= u32::MAX as u64 && ((gid0) as u64) >= ((a0) as u64)) {
        return;
    } else {
        // Lanes part: the chunk's lanes on both sides of the bound.
        let t471: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t472 = [vdupq_n_u32(a0); 2];
        let t473: [uint32x4_t; 2] = [vcgeq_u32(t471[0], t472[0]), vcgeq_u32(t471[1], t472[1])];
        let t474: [uint32x4_t; 2] = [vandq_u32(live[0], t473[0]), vandq_u32(live[1], t473[1])];
        {
            live = [vbicq_u32(live[0], t474[0]), vbicq_u32(live[1], t474[1])];
        }
        if any(&live) {
            let t475 = (arg0 as *const u8).add(((gid0) as usize) * 8);
            let mut t476: [uint32x4_t; 4];
            if all(&live) {
                let t4770 = vld2q_u32((t475 as *const u32).add(0));
                let t4771 = vld2q_u32((t475 as *const u32).add(8));
                let t477: [uint32x4_t; 4] = [t4770.0, t4771.0, t4770.1, t4771.1];
                t476 = t477;
            } else {
                let t478 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t471[0]), vandq_u32(live[1], t471[1]))));
                let t479: [uint32x4_t; 2] = [vbslq_u32(live[0], t471[0], t478), vbslq_u32(live[1], t471[1], t478)];
                let mut t480: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t479[0]) as usize * 8) as *const u64).read_unaligned(); t480[0] = vsetq_lane_u32::<0>(v as u32, t480[0]); t480[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t480[2]); }
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t479[0]) as usize * 8) as *const u64).read_unaligned(); t480[0] = vsetq_lane_u32::<1>(v as u32, t480[0]); t480[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t480[2]); }
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t479[0]) as usize * 8) as *const u64).read_unaligned(); t480[0] = vsetq_lane_u32::<2>(v as u32, t480[0]); t480[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t480[2]); }
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t479[0]) as usize * 8) as *const u64).read_unaligned(); t480[0] = vsetq_lane_u32::<3>(v as u32, t480[0]); t480[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t480[2]); }
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t479[1]) as usize * 8) as *const u64).read_unaligned(); t480[1] = vsetq_lane_u32::<0>(v as u32, t480[1]); t480[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t480[3]); }
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t479[1]) as usize * 8) as *const u64).read_unaligned(); t480[1] = vsetq_lane_u32::<1>(v as u32, t480[1]); t480[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t480[3]); }
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t479[1]) as usize * 8) as *const u64).read_unaligned(); t480[1] = vsetq_lane_u32::<2>(v as u32, t480[1]); t480[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t480[3]); }
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t479[1]) as usize * 8) as *const u64).read_unaligned(); t480[1] = vsetq_lane_u32::<3>(v as u32, t480[1]); t480[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t480[3]); }
                t476 = t480;
            }
            x6 = t476;
            let t481 = (arg1 as *const u8).add(((gid0) as usize) * 8);
            let mut t482: [uint32x4_t; 4];
            if all(&live) {
                let t4830 = vld2q_u32((t481 as *const u32).add(0));
                let t4831 = vld2q_u32((t481 as *const u32).add(8));
                let t483: [uint32x4_t; 4] = [t4830.0, t4831.0, t4830.1, t4831.1];
                t482 = t483;
            } else {
                let t484 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t471[0]), vandq_u32(live[1], t471[1]))));
                let t485: [uint32x4_t; 2] = [vbslq_u32(live[0], t471[0], t484), vbslq_u32(live[1], t471[1], t484)];
                let mut t486: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
                { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<0>(t485[0]) as usize * 8) as *const u64).read_unaligned(); t486[0] = vsetq_lane_u32::<0>(v as u32, t486[0]); t486[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t486[2]); }
                { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<1>(t485[0]) as usize * 8) as *const u64).read_unaligned(); t486[0] = vsetq_lane_u32::<1>(v as u32, t486[0]); t486[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t486[2]); }
                { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<2>(t485[0]) as usize * 8) as *const u64).read_unaligned(); t486[0] = vsetq_lane_u32::<2>(v as u32, t486[0]); t486[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t486[2]); }
                { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<3>(t485[0]) as usize * 8) as *const u64).read_unaligned(); t486[0] = vsetq_lane_u32::<3>(v as u32, t486[0]); t486[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t486[2]); }
                { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<0>(t485[1]) as usize * 8) as *const u64).read_unaligned(); t486[1] = vsetq_lane_u32::<0>(v as u32, t486[1]); t486[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t486[3]); }
                { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<1>(t485[1]) as usize * 8) as *const u64).read_unaligned(); t486[1] = vsetq_lane_u32::<1>(v as u32, t486[1]); t486[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t486[3]); }
                { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<2>(t485[1]) as usize * 8) as *const u64).read_unaligned(); t486[1] = vsetq_lane_u32::<2>(v as u32, t486[1]); t486[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t486[3]); }
                { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<3>(t485[1]) as usize * 8) as *const u64).read_unaligned(); t486[1] = vsetq_lane_u32::<3>(v as u32, t486[1]); t486[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t486[3]); }
                t482 = t486;
            }
            x7 = t482;
            let mut t487: [uint32x4_t; 2];
            if all(&live) {
                let t488 = (arg2 as *const u32).add((gid0) as usize);
                t487 = [vld1q_u32(t488.add(0)), vld1q_u32(t488.add(4))];
            } else {
                let t489 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t471[0]), vandq_u32(live[1], t471[1]))));
                let t490: [uint32x4_t; 2] = [vbslq_u32(live[0], t471[0], t489), vbslq_u32(live[1], t471[1], t489)];
                let t491 = arg2 as *const u32;
                let mut t492: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                t492[0] = vld1q_lane_u32::<0>(t491.add(vgetq_lane_u32::<0>(t490[0]) as usize), t492[0]);
                t492[0] = vld1q_lane_u32::<1>(t491.add(vgetq_lane_u32::<1>(t490[0]) as usize), t492[0]);
                t492[0] = vld1q_lane_u32::<2>(t491.add(vgetq_lane_u32::<2>(t490[0]) as usize), t492[0]);
                t492[0] = vld1q_lane_u32::<3>(t491.add(vgetq_lane_u32::<3>(t490[0]) as usize), t492[0]);
                t492[1] = vld1q_lane_u32::<0>(t491.add(vgetq_lane_u32::<0>(t490[1]) as usize), t492[1]);
                t492[1] = vld1q_lane_u32::<1>(t491.add(vgetq_lane_u32::<1>(t490[1]) as usize), t492[1]);
                t492[1] = vld1q_lane_u32::<2>(t491.add(vgetq_lane_u32::<2>(t490[1]) as usize), t492[1]);
                t492[1] = vld1q_lane_u32::<3>(t491.add(vgetq_lane_u32::<3>(t490[1]) as usize), t492[1]);
                t487 = t492;
            }
            x8 = t487;
            let t493 = [vdupq_n_u32(63u32); 2];
            let t494: [uint32x4_t; 2] = [vandq_u32(x8[0], t493[0]), vandq_u32(x8[1], t493[1])];
            x9 = t494;
            x10 = a0;
            let t495: [uint32x4_t; 2] = [vaddq_u32(x6[0], x7[0]), vaddq_u32(x6[1], x7[1])];
            let t496: [uint32x4_t; 2] = [vcltq_u32(t495[0], x6[0]), vcltq_u32(t495[1], x6[1])];
            let t497: [uint32x4_t; 4] = [t495[0], t495[1], vsubq_u32(vaddq_u32(x6[2], x7[2]), t496[0]), vsubq_u32(vaddq_u32(x6[3], x7[3]), t496[1])];
            let t498 = (arg3 as *const u8).add(((gid0.wrapping_add((0u32).wrapping_mul(x10))) as usize) * 8);
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
            let t500: [uint32x4_t; 2] = [vcltq_u32(x6[0], x7[0]), vcltq_u32(x6[1], x7[1])];
            let t501: [uint32x4_t; 4] = [vsubq_u32(x6[0], x7[0]), vsubq_u32(x6[1], x7[1]), vaddq_u32(vsubq_u32(x6[2], x7[2]), t500[0]), vaddq_u32(vsubq_u32(x6[3], x7[3]), t500[1])];
            let t502 = (arg3 as *const u8).add(((gid0.wrapping_add((1u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t502 as *mut u32).add(0), uint32x4x2_t(t501[0], t501[2]));
                vst2q_u32((t502 as *mut u32).add(8), uint32x4x2_t(t501[1], t501[3]));
            } else {
                let mut t503w = 0u64;
                let t503 = core::hint::black_box(&raw mut t503w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t502 as *mut u64).add(0), t503).write_unaligned(((vgetq_lane_u32::<0>(t501[2]) as u64) << 32 | vgetq_lane_u32::<0>(t501[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t502 as *mut u64).add(1), t503).write_unaligned(((vgetq_lane_u32::<1>(t501[2]) as u64) << 32 | vgetq_lane_u32::<1>(t501[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t502 as *mut u64).add(2), t503).write_unaligned(((vgetq_lane_u32::<2>(t501[2]) as u64) << 32 | vgetq_lane_u32::<2>(t501[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t502 as *mut u64).add(3), t503).write_unaligned(((vgetq_lane_u32::<3>(t501[2]) as u64) << 32 | vgetq_lane_u32::<3>(t501[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t502 as *mut u64).add(4), t503).write_unaligned(((vgetq_lane_u32::<0>(t501[3]) as u64) << 32 | vgetq_lane_u32::<0>(t501[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t502 as *mut u64).add(5), t503).write_unaligned(((vgetq_lane_u32::<1>(t501[3]) as u64) << 32 | vgetq_lane_u32::<1>(t501[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t502 as *mut u64).add(6), t503).write_unaligned(((vgetq_lane_u32::<2>(t501[3]) as u64) << 32 | vgetq_lane_u32::<2>(t501[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t502 as *mut u64).add(7), t503).write_unaligned(((vgetq_lane_u32::<3>(t501[3]) as u64) << 32 | vgetq_lane_u32::<3>(t501[1]) as u64));
            }
            let t504: [uint32x4_t; 2] = [x9[0], x9[1]];
            let t505: [uint32x4_t; 2] = [vandq_u32(t504[0], vdupq_n_u32(63)), vandq_u32(t504[1], vdupq_n_u32(63))];
            let t506: [uint32x4_t; 4] = [vshlq_u32(x6[0], vreinterpretq_s32_u32(t505[0])), vshlq_u32(x6[1], vreinterpretq_s32_u32(t505[1])), vorrq_u32(vshlq_u32(x6[2], vreinterpretq_s32_u32(t505[0])), vshlq_u32(x6[0], vsubq_s32(vreinterpretq_s32_u32(t505[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(x6[3], vreinterpretq_s32_u32(t505[1])), vshlq_u32(x6[1], vsubq_s32(vreinterpretq_s32_u32(t505[1]), vdupq_n_s32(32))))];
            let t507 = (arg3 as *const u8).add(((gid0.wrapping_add((2u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t507 as *mut u32).add(0), uint32x4x2_t(t506[0], t506[2]));
                vst2q_u32((t507 as *mut u32).add(8), uint32x4x2_t(t506[1], t506[3]));
            } else {
                let mut t508w = 0u64;
                let t508 = core::hint::black_box(&raw mut t508w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t507 as *mut u64).add(0), t508).write_unaligned(((vgetq_lane_u32::<0>(t506[2]) as u64) << 32 | vgetq_lane_u32::<0>(t506[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t507 as *mut u64).add(1), t508).write_unaligned(((vgetq_lane_u32::<1>(t506[2]) as u64) << 32 | vgetq_lane_u32::<1>(t506[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t507 as *mut u64).add(2), t508).write_unaligned(((vgetq_lane_u32::<2>(t506[2]) as u64) << 32 | vgetq_lane_u32::<2>(t506[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t507 as *mut u64).add(3), t508).write_unaligned(((vgetq_lane_u32::<3>(t506[2]) as u64) << 32 | vgetq_lane_u32::<3>(t506[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t507 as *mut u64).add(4), t508).write_unaligned(((vgetq_lane_u32::<0>(t506[3]) as u64) << 32 | vgetq_lane_u32::<0>(t506[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t507 as *mut u64).add(5), t508).write_unaligned(((vgetq_lane_u32::<1>(t506[3]) as u64) << 32 | vgetq_lane_u32::<1>(t506[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t507 as *mut u64).add(6), t508).write_unaligned(((vgetq_lane_u32::<2>(t506[3]) as u64) << 32 | vgetq_lane_u32::<2>(t506[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t507 as *mut u64).add(7), t508).write_unaligned(((vgetq_lane_u32::<3>(t506[3]) as u64) << 32 | vgetq_lane_u32::<3>(t506[1]) as u64));
            }
            let t509: [uint32x4_t; 2] = [x9[0], x9[1]];
            let t510: [uint32x4_t; 2] = [vandq_u32(t509[0], vdupq_n_u32(63)), vandq_u32(t509[1], vdupq_n_u32(63))];
            let t511: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(x6[0], vnegq_s32(vreinterpretq_s32_u32(t510[0]))), vshlq_u32(x6[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t510[0])))), vorrq_u32(vshlq_u32(x6[1], vnegq_s32(vreinterpretq_s32_u32(t510[1]))), vshlq_u32(x6[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t510[1])))), vshlq_u32(x6[2], vnegq_s32(vreinterpretq_s32_u32(t510[0]))), vshlq_u32(x6[3], vnegq_s32(vreinterpretq_s32_u32(t510[1])))];
            let t512 = (arg3 as *const u8).add(((gid0.wrapping_add((3u32).wrapping_mul(x10))) as usize) * 8);
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
            let t514: [uint32x4_t; 2] = [x9[0], x9[1]];
            let t515: [uint32x4_t; 2] = [vandq_u32(t514[0], vdupq_n_u32(63)), vandq_u32(t514[1], vdupq_n_u32(63))];
            let t516: [uint32x4_t; 2] = [vcgeq_s32(vreinterpretq_s32_u32(t515[0]), vdupq_n_s32(32)), vcgeq_s32(vreinterpretq_s32_u32(t515[1]), vdupq_n_s32(32))];
            let t517: [uint32x4_t; 4] = [vbslq_u32(t516[0], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t515[0])))), vorrq_u32(vshlq_u32(x6[0], vnegq_s32(vreinterpretq_s32_u32(t515[0]))), vshlq_u32(x6[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t515[0]))))), vbslq_u32(t516[1], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t515[1])))), vorrq_u32(vshlq_u32(x6[1], vnegq_s32(vreinterpretq_s32_u32(t515[1]))), vshlq_u32(x6[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t515[1]))))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vnegq_s32(vreinterpretq_s32_u32(t515[0])))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vnegq_s32(vreinterpretq_s32_u32(t515[1]))))];
            let t518 = (arg3 as *const u8).add(((gid0.wrapping_add((4u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t518 as *mut u32).add(0), uint32x4x2_t(t517[0], t517[2]));
                vst2q_u32((t518 as *mut u32).add(8), uint32x4x2_t(t517[1], t517[3]));
            } else {
                let mut t519w = 0u64;
                let t519 = core::hint::black_box(&raw mut t519w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t518 as *mut u64).add(0), t519).write_unaligned(((vgetq_lane_u32::<0>(t517[2]) as u64) << 32 | vgetq_lane_u32::<0>(t517[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t518 as *mut u64).add(1), t519).write_unaligned(((vgetq_lane_u32::<1>(t517[2]) as u64) << 32 | vgetq_lane_u32::<1>(t517[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t518 as *mut u64).add(2), t519).write_unaligned(((vgetq_lane_u32::<2>(t517[2]) as u64) << 32 | vgetq_lane_u32::<2>(t517[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t518 as *mut u64).add(3), t519).write_unaligned(((vgetq_lane_u32::<3>(t517[2]) as u64) << 32 | vgetq_lane_u32::<3>(t517[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t518 as *mut u64).add(4), t519).write_unaligned(((vgetq_lane_u32::<0>(t517[3]) as u64) << 32 | vgetq_lane_u32::<0>(t517[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t518 as *mut u64).add(5), t519).write_unaligned(((vgetq_lane_u32::<1>(t517[3]) as u64) << 32 | vgetq_lane_u32::<1>(t517[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t518 as *mut u64).add(6), t519).write_unaligned(((vgetq_lane_u32::<2>(t517[3]) as u64) << 32 | vgetq_lane_u32::<2>(t517[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t518 as *mut u64).add(7), t519).write_unaligned(((vgetq_lane_u32::<3>(t517[3]) as u64) << 32 | vgetq_lane_u32::<3>(t517[1]) as u64));
            }
            let t520: [uint32x4_t; 4] = [vshlq_n_u32::<13>(x6[0]), vshlq_n_u32::<13>(x6[1]), vorrq_u32(vshlq_n_u32::<13>(x6[2]), vshrq_n_u32::<19>(x6[0])), vorrq_u32(vshlq_n_u32::<13>(x6[3]), vshrq_n_u32::<19>(x6[1]))];
            let t521: [uint32x4_t; 2] = [vshrq_n_u32::<19>(x7[2]), vshrq_n_u32::<19>(x7[3])];
            let t522: [uint32x4_t; 4] = [vorrq_u32(t520[0], t521[0]), vorrq_u32(t520[1], t521[1]), t520[2], t520[3]];
            let t523 = (arg3 as *const u8).add(((gid0.wrapping_add((5u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t523 as *mut u32).add(0), uint32x4x2_t(t522[0], t522[2]));
                vst2q_u32((t523 as *mut u32).add(8), uint32x4x2_t(t522[1], t522[3]));
            } else {
                let mut t524w = 0u64;
                let t524 = core::hint::black_box(&raw mut t524w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t523 as *mut u64).add(0), t524).write_unaligned(((vgetq_lane_u32::<0>(t522[2]) as u64) << 32 | vgetq_lane_u32::<0>(t522[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t523 as *mut u64).add(1), t524).write_unaligned(((vgetq_lane_u32::<1>(t522[2]) as u64) << 32 | vgetq_lane_u32::<1>(t522[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t523 as *mut u64).add(2), t524).write_unaligned(((vgetq_lane_u32::<2>(t522[2]) as u64) << 32 | vgetq_lane_u32::<2>(t522[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t523 as *mut u64).add(3), t524).write_unaligned(((vgetq_lane_u32::<3>(t522[2]) as u64) << 32 | vgetq_lane_u32::<3>(t522[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t523 as *mut u64).add(4), t524).write_unaligned(((vgetq_lane_u32::<0>(t522[3]) as u64) << 32 | vgetq_lane_u32::<0>(t522[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t523 as *mut u64).add(5), t524).write_unaligned(((vgetq_lane_u32::<1>(t522[3]) as u64) << 32 | vgetq_lane_u32::<1>(t522[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t523 as *mut u64).add(6), t524).write_unaligned(((vgetq_lane_u32::<2>(t522[3]) as u64) << 32 | vgetq_lane_u32::<2>(t522[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t523 as *mut u64).add(7), t524).write_unaligned(((vgetq_lane_u32::<3>(t522[3]) as u64) << 32 | vgetq_lane_u32::<3>(t522[1]) as u64));
            }
            let t525: [uint32x4_t; 4] = [vdupq_n_u32(0), vdupq_n_u32(0), vshlq_n_u32::<8>(x6[0]), vshlq_n_u32::<8>(x6[1])];
            let t526 = (arg3 as *const u8).add(((gid0.wrapping_add((6u32).wrapping_mul(x10))) as usize) * 8);
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
            let t528: [uint32x4_t; 2] = [vshrq_n_u32::<3>(x6[2]), vshrq_n_u32::<3>(x6[3])];
            let t529: [uint32x4_t; 4] = [t528[0], t528[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            let t530 = (arg3 as *const u8).add(((gid0.wrapping_add((7u32).wrapping_mul(x10))) as usize) * 8);
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
            let t532 = vdupq_n_s32(((((40i64) as u32)) & 63) as i32);
            let t533: [uint32x4_t; 2] = [vcgeq_s32(t532, vdupq_n_s32(32)), vcgeq_s32(t532, vdupq_n_s32(32))];
            let t534: [uint32x4_t; 4] = [vbslq_u32(t533[0], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vsubq_s32(vdupq_n_s32(32), t532))), vorrq_u32(vshlq_u32(x6[0], vnegq_s32(t532)), vshlq_u32(x6[2], vsubq_s32(vdupq_n_s32(32), t532)))), vbslq_u32(t533[1], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vsubq_s32(vdupq_n_s32(32), t532))), vorrq_u32(vshlq_u32(x6[1], vnegq_s32(t532)), vshlq_u32(x6[3], vsubq_s32(vdupq_n_s32(32), t532)))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vnegq_s32(t532))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vnegq_s32(t532)))];
            let t535 = (arg3 as *const u8).add(((gid0.wrapping_add((8u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t535 as *mut u32).add(0), uint32x4x2_t(t534[0], t534[2]));
                vst2q_u32((t535 as *mut u32).add(8), uint32x4x2_t(t534[1], t534[3]));
            } else {
                let mut t536w = 0u64;
                let t536 = core::hint::black_box(&raw mut t536w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t535 as *mut u64).add(0), t536).write_unaligned(((vgetq_lane_u32::<0>(t534[2]) as u64) << 32 | vgetq_lane_u32::<0>(t534[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t535 as *mut u64).add(1), t536).write_unaligned(((vgetq_lane_u32::<1>(t534[2]) as u64) << 32 | vgetq_lane_u32::<1>(t534[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t535 as *mut u64).add(2), t536).write_unaligned(((vgetq_lane_u32::<2>(t534[2]) as u64) << 32 | vgetq_lane_u32::<2>(t534[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t535 as *mut u64).add(3), t536).write_unaligned(((vgetq_lane_u32::<3>(t534[2]) as u64) << 32 | vgetq_lane_u32::<3>(t534[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t535 as *mut u64).add(4), t536).write_unaligned(((vgetq_lane_u32::<0>(t534[3]) as u64) << 32 | vgetq_lane_u32::<0>(t534[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t535 as *mut u64).add(5), t536).write_unaligned(((vgetq_lane_u32::<1>(t534[3]) as u64) << 32 | vgetq_lane_u32::<1>(t534[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t535 as *mut u64).add(6), t536).write_unaligned(((vgetq_lane_u32::<2>(t534[3]) as u64) << 32 | vgetq_lane_u32::<2>(t534[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t535 as *mut u64).add(7), t536).write_unaligned(((vgetq_lane_u32::<3>(t534[3]) as u64) << 32 | vgetq_lane_u32::<3>(t534[1]) as u64));
            }
            let t537: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x6[0]), vget_low_u32(x7[0]))), vreinterpretq_u32_u64(vmull_high_u32(x6[0], x7[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x6[1]), vget_low_u32(x7[1]))), vreinterpretq_u32_u64(vmull_high_u32(x6[1], x7[1])))];
            let t538: [uint32x4_t; 4] = [vmulq_u32(x6[0], x7[0]), vmulq_u32(x6[1], x7[1]), vmlaq_u32(vmlaq_u32(t537[0], x6[0], x7[2]), x7[0], x6[2]), vmlaq_u32(vmlaq_u32(t537[1], x6[1], x7[3]), x7[1], x6[3])];
            let t539 = (arg3 as *const u8).add(((gid0.wrapping_add((9u32).wrapping_mul(x10))) as usize) * 8);
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
            let t541: u64 = 0u64;
            let t542 = [vdupq_n_u32((t541) as u32); 2];
            let t543: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x7[0], t542[0]), vceqq_u32(x7[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(x7[1], t542[1]), vceqq_u32(x7[3], vdupq_n_u32(0)))];
            let t544: [uint32x4_t; 2] = [vmvnq_u32(t543[0]), vmvnq_u32(t543[1])];
            let mut t545: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            let t546: [uint32x4_t; 2] = [vandq_u32(live[0], t544[0]), vandq_u32(live[1], t544[1])];
            let t547: [uint32x4_t; 2] = [vbicq_u32(live[0], t544[0]), vbicq_u32(live[1], t544[1])];
            {
                let (mut t548xl, mut t548xh, mut t548yl, mut t548yh) = ([0u32; 8], [0u32; 8], [0u32; 8], [0u32; 8]);
                vst1q_u32(t548xl.as_mut_ptr().add(0), x6[0]); vst1q_u32(t548xh.as_mut_ptr().add(0), x6[2]); vst1q_u32(t548yl.as_mut_ptr().add(0), x7[0]); vst1q_u32(t548yh.as_mut_ptr().add(0), x7[2]);
                vst1q_u32(t548xl.as_mut_ptr().add(4), x6[1]); vst1q_u32(t548xh.as_mut_ptr().add(4), x6[3]); vst1q_u32(t548yl.as_mut_ptr().add(4), x7[1]); vst1q_u32(t548yh.as_mut_ptr().add(4), x7[3]);
                for j in 0..8 { let q = (((t548xh[j] as u64) << 32 | t548xl[j] as u64) as u64).checked_div(((t548yh[j] as u64) << 32 | t548yl[j] as u64) as u64).unwrap_or(0) as u64; t548xl[j] = q as u32; t548xh[j] = (q >> 32) as u32; }
                let t549: [uint32x4_t; 4] = [vld1q_u32(t548xl.as_ptr().add(0)), vld1q_u32(t548xl.as_ptr().add(4)), vld1q_u32(t548xh.as_ptr().add(0)), vld1q_u32(t548xh.as_ptr().add(4))];
                t545 = [vbslq_u32(t546[0], t549[0], t545[0]), vbslq_u32(t546[1], t549[1], t545[1]), vbslq_u32(t546[0], t549[2], t545[2]), vbslq_u32(t546[1], t549[3], t545[3])];
            }
            {
                let t550: u64 = 7u64;
                let t551 = [vdupq_n_u32((t550) as u32); 2];
                let t552: [uint32x4_t; 4] = [t551[0], t551[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                t545 = [vbslq_u32(t547[0], t552[0], t545[0]), vbslq_u32(t547[1], t552[1], t545[1]), vbslq_u32(t547[0], t552[2], t545[2]), vbslq_u32(t547[1], t552[3], t545[3])];
            }
            let t553 = (arg3 as *const u8).add(((gid0.wrapping_add((10u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t553 as *mut u32).add(0), uint32x4x2_t(t545[0], t545[2]));
                vst2q_u32((t553 as *mut u32).add(8), uint32x4x2_t(t545[1], t545[3]));
            } else {
                let mut t554w = 0u64;
                let t554 = core::hint::black_box(&raw mut t554w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t553 as *mut u64).add(0), t554).write_unaligned(((vgetq_lane_u32::<0>(t545[2]) as u64) << 32 | vgetq_lane_u32::<0>(t545[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t553 as *mut u64).add(1), t554).write_unaligned(((vgetq_lane_u32::<1>(t545[2]) as u64) << 32 | vgetq_lane_u32::<1>(t545[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t553 as *mut u64).add(2), t554).write_unaligned(((vgetq_lane_u32::<2>(t545[2]) as u64) << 32 | vgetq_lane_u32::<2>(t545[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t553 as *mut u64).add(3), t554).write_unaligned(((vgetq_lane_u32::<3>(t545[2]) as u64) << 32 | vgetq_lane_u32::<3>(t545[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t553 as *mut u64).add(4), t554).write_unaligned(((vgetq_lane_u32::<0>(t545[3]) as u64) << 32 | vgetq_lane_u32::<0>(t545[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t553 as *mut u64).add(5), t554).write_unaligned(((vgetq_lane_u32::<1>(t545[3]) as u64) << 32 | vgetq_lane_u32::<1>(t545[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t553 as *mut u64).add(6), t554).write_unaligned(((vgetq_lane_u32::<2>(t545[3]) as u64) << 32 | vgetq_lane_u32::<2>(t545[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t553 as *mut u64).add(7), t554).write_unaligned(((vgetq_lane_u32::<3>(t545[3]) as u64) << 32 | vgetq_lane_u32::<3>(t545[1]) as u64));
            }
            let t555: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x6[2], x7[2]), vandq_u32(vceqq_u32(x6[2], x7[2]), vcltq_u32(x6[0], x7[0]))), vorrq_u32(vcltq_u32(x6[3], x7[3]), vandq_u32(vceqq_u32(x6[3], x7[3]), vcltq_u32(x6[1], x7[1])))];
            let t556: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t555[0]), vshrq_n_u32::<31>(t555[1])];
            let t557: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x7[2], x6[2]), vandq_u32(vceqq_u32(x7[2], x6[2]), vcltq_u32(x7[0], x6[0]))), vorrq_u32(vcltq_u32(x7[3], x6[3]), vandq_u32(vceqq_u32(x7[3], x6[3]), vcltq_u32(x7[1], x6[1])))];
            let t558: [uint32x4_t; 2] = [vmvnq_u32(t557[0]), vmvnq_u32(t557[1])];
            let t559: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t558[0]), vshrq_n_u32::<31>(t558[1])];
            let t560: [uint32x4_t; 4] = [vshlq_n_u32::<1>(t559[0]), vshlq_n_u32::<1>(t559[1]), vshrq_n_u32::<31>(t559[0]), vshrq_n_u32::<31>(t559[1])];
            let t561: [uint32x4_t; 4] = [vorrq_u32(t556[0], t560[0]), vorrq_u32(t556[1], t560[1]), t560[2], t560[3]];
            let t562: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x6[0], x7[0]), vceqq_u32(x6[2], x7[2])), vandq_u32(vceqq_u32(x6[1], x7[1]), vceqq_u32(x6[3], x7[3]))];
            let t563: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t562[0]), vshrq_n_u32::<31>(t562[1])];
            let t564: [uint32x4_t; 4] = [vshlq_n_u32::<2>(t563[0]), vshlq_n_u32::<2>(t563[1]), vshrq_n_u32::<30>(t563[0]), vshrq_n_u32::<30>(t563[1])];
            let t565: [uint32x4_t; 4] = [vorrq_u32(t561[0], t564[0]), vorrq_u32(t561[1], t564[1]), vorrq_u32(t561[2], t564[2]), vorrq_u32(t561[3], t564[3])];
            let t566: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x6[0], x7[0]), vceqq_u32(x6[2], x7[2])), vandq_u32(vceqq_u32(x6[1], x7[1]), vceqq_u32(x6[3], x7[3]))];
            let t567: [uint32x4_t; 2] = [vmvnq_u32(t566[0]), vmvnq_u32(t566[1])];
            let t568: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t567[0]), vshrq_n_u32::<31>(t567[1])];
            let t569: [uint32x4_t; 4] = [vshlq_n_u32::<3>(t568[0]), vshlq_n_u32::<3>(t568[1]), vshrq_n_u32::<29>(t568[0]), vshrq_n_u32::<29>(t568[1])];
            let t570: [uint32x4_t; 4] = [vorrq_u32(t565[0], t569[0]), vorrq_u32(t565[1], t569[1]), vorrq_u32(t565[2], t569[2]), vorrq_u32(t565[3], t569[3])];
            let t571: [uint32x4_t; 2] = [vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x6[2]), vreinterpretq_s32_u32(x7[2])), vandq_u32(vceqq_u32(x6[2], x7[2]), vcltq_u32(x6[0], x7[0]))), vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x6[3]), vreinterpretq_s32_u32(x7[3])), vandq_u32(vceqq_u32(x6[3], x7[3]), vcltq_u32(x6[1], x7[1])))];
            let t572: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t571[0]), vshrq_n_u32::<31>(t571[1])];
            let t573: [uint32x4_t; 4] = [vshlq_n_u32::<4>(t572[0]), vshlq_n_u32::<4>(t572[1]), vshrq_n_u32::<28>(t572[0]), vshrq_n_u32::<28>(t572[1])];
            let t574: [uint32x4_t; 4] = [vorrq_u32(t570[0], t573[0]), vorrq_u32(t570[1], t573[1]), vorrq_u32(t570[2], t573[2]), vorrq_u32(t570[3], t573[3])];
            let t575: [uint32x4_t; 2] = [vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x6[2]), vreinterpretq_s32_u32(x7[2])), vandq_u32(vceqq_u32(x6[2], x7[2]), vcltq_u32(x6[0], x7[0]))), vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x6[3]), vreinterpretq_s32_u32(x7[3])), vandq_u32(vceqq_u32(x6[3], x7[3]), vcltq_u32(x6[1], x7[1])))];
            let t576: [uint32x4_t; 2] = [vmvnq_u32(t575[0]), vmvnq_u32(t575[1])];
            let t577: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t576[0]), vshrq_n_u32::<31>(t576[1])];
            let t578: [uint32x4_t; 4] = [vshlq_n_u32::<5>(t577[0]), vshlq_n_u32::<5>(t577[1]), vshrq_n_u32::<27>(t577[0]), vshrq_n_u32::<27>(t577[1])];
            let t579: [uint32x4_t; 4] = [vorrq_u32(t574[0], t578[0]), vorrq_u32(t574[1], t578[1]), vorrq_u32(t574[2], t578[2]), vorrq_u32(t574[3], t578[3])];
            let t580: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x7[2], x6[2]), vandq_u32(vceqq_u32(x7[2], x6[2]), vcltq_u32(x7[0], x6[0]))), vorrq_u32(vcltq_u32(x7[3], x6[3]), vandq_u32(vceqq_u32(x7[3], x6[3]), vcltq_u32(x7[1], x6[1])))];
            let t581: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t580[0]), vshrq_n_u32::<31>(t580[1])];
            let t582: [uint32x4_t; 4] = [vshlq_n_u32::<6>(t581[0]), vshlq_n_u32::<6>(t581[1]), vshrq_n_u32::<26>(t581[0]), vshrq_n_u32::<26>(t581[1])];
            let t583: [uint32x4_t; 4] = [vorrq_u32(t579[0], t582[0]), vorrq_u32(t579[1], t582[1]), vorrq_u32(t579[2], t582[2]), vorrq_u32(t579[3], t582[3])];
            let t584 = (arg3 as *const u8).add(((gid0.wrapping_add((11u32).wrapping_mul(x10))) as usize) * 8);
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
            let t586: [uint32x4_t; 2] = [vaddq_u32(x8[0], x6[0]), vaddq_u32(x8[1], x6[1])];
            let t587: [uint32x4_t; 2] = [vcltq_u32(t586[0], x8[0]), vcltq_u32(t586[1], x8[1])];
            let t588: [uint32x4_t; 4] = [t586[0], t586[1], vsubq_u32(vaddq_u32(vdupq_n_u32(0), x6[2]), t587[0]), vsubq_u32(vaddq_u32(vdupq_n_u32(0), x6[3]), t587[1])];
            let t589 = (arg3 as *const u8).add(((gid0.wrapping_add((12u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t589 as *mut u32).add(0), uint32x4x2_t(t588[0], t588[2]));
                vst2q_u32((t589 as *mut u32).add(8), uint32x4x2_t(t588[1], t588[3]));
            } else {
                let mut t590w = 0u64;
                let t590 = core::hint::black_box(&raw mut t590w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t589 as *mut u64).add(0), t590).write_unaligned(((vgetq_lane_u32::<0>(t588[2]) as u64) << 32 | vgetq_lane_u32::<0>(t588[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t589 as *mut u64).add(1), t590).write_unaligned(((vgetq_lane_u32::<1>(t588[2]) as u64) << 32 | vgetq_lane_u32::<1>(t588[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t589 as *mut u64).add(2), t590).write_unaligned(((vgetq_lane_u32::<2>(t588[2]) as u64) << 32 | vgetq_lane_u32::<2>(t588[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t589 as *mut u64).add(3), t590).write_unaligned(((vgetq_lane_u32::<3>(t588[2]) as u64) << 32 | vgetq_lane_u32::<3>(t588[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t589 as *mut u64).add(4), t590).write_unaligned(((vgetq_lane_u32::<0>(t588[3]) as u64) << 32 | vgetq_lane_u32::<0>(t588[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t589 as *mut u64).add(5), t590).write_unaligned(((vgetq_lane_u32::<1>(t588[3]) as u64) << 32 | vgetq_lane_u32::<1>(t588[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t589 as *mut u64).add(6), t590).write_unaligned(((vgetq_lane_u32::<2>(t588[3]) as u64) << 32 | vgetq_lane_u32::<2>(t588[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t589 as *mut u64).add(7), t590).write_unaligned(((vgetq_lane_u32::<3>(t588[3]) as u64) << 32 | vgetq_lane_u32::<3>(t588[1]) as u64));
            }
            let t591: [uint32x4_t; 2] = [vreinterpretq_u32_s32(vshrq_n_s32::<31>(vreinterpretq_s32_u32(x8[0]))), vreinterpretq_u32_s32(vshrq_n_s32::<31>(vreinterpretq_s32_u32(x8[1])))];
            let t592: [uint32x4_t; 4] = [x8[0], x8[1], t591[0], t591[1]];
            let t593 = (arg3 as *const u8).add(((gid0.wrapping_add((13u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t593 as *mut u32).add(0), uint32x4x2_t(t592[0], t592[2]));
                vst2q_u32((t593 as *mut u32).add(8), uint32x4x2_t(t592[1], t592[3]));
            } else {
                let mut t594w = 0u64;
                let t594 = core::hint::black_box(&raw mut t594w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t593 as *mut u64).add(0), t594).write_unaligned(((vgetq_lane_u32::<0>(t592[2]) as u64) << 32 | vgetq_lane_u32::<0>(t592[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t593 as *mut u64).add(1), t594).write_unaligned(((vgetq_lane_u32::<1>(t592[2]) as u64) << 32 | vgetq_lane_u32::<1>(t592[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t593 as *mut u64).add(2), t594).write_unaligned(((vgetq_lane_u32::<2>(t592[2]) as u64) << 32 | vgetq_lane_u32::<2>(t592[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t593 as *mut u64).add(3), t594).write_unaligned(((vgetq_lane_u32::<3>(t592[2]) as u64) << 32 | vgetq_lane_u32::<3>(t592[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t593 as *mut u64).add(4), t594).write_unaligned(((vgetq_lane_u32::<0>(t592[3]) as u64) << 32 | vgetq_lane_u32::<0>(t592[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t593 as *mut u64).add(5), t594).write_unaligned(((vgetq_lane_u32::<1>(t592[3]) as u64) << 32 | vgetq_lane_u32::<1>(t592[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t593 as *mut u64).add(6), t594).write_unaligned(((vgetq_lane_u32::<2>(t592[3]) as u64) << 32 | vgetq_lane_u32::<2>(t592[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t593 as *mut u64).add(7), t594).write_unaligned(((vgetq_lane_u32::<3>(t592[3]) as u64) << 32 | vgetq_lane_u32::<3>(t592[1]) as u64));
            }
            let t595: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<3>(x6[0]), vshlq_n_u32::<29>(x6[2])), vorrq_u32(vshrq_n_u32::<3>(x6[1]), vshlq_n_u32::<29>(x6[3])), vshrq_n_u32::<3>(x6[2]), vshrq_n_u32::<3>(x6[3])];
            let t596: [uint32x4_t; 2] = [t595[0], t595[1]];
            let t597: [uint32x4_t; 4] = [t596[0], t596[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            let t598 = (arg3 as *const u8).add(((gid0.wrapping_add((14u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t598 as *mut u32).add(0), uint32x4x2_t(t597[0], t597[2]));
                vst2q_u32((t598 as *mut u32).add(8), uint32x4x2_t(t597[1], t597[3]));
            } else {
                let mut t599w = 0u64;
                let t599 = core::hint::black_box(&raw mut t599w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t598 as *mut u64).add(0), t599).write_unaligned(((vgetq_lane_u32::<0>(t597[2]) as u64) << 32 | vgetq_lane_u32::<0>(t597[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t598 as *mut u64).add(1), t599).write_unaligned(((vgetq_lane_u32::<1>(t597[2]) as u64) << 32 | vgetq_lane_u32::<1>(t597[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t598 as *mut u64).add(2), t599).write_unaligned(((vgetq_lane_u32::<2>(t597[2]) as u64) << 32 | vgetq_lane_u32::<2>(t597[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t598 as *mut u64).add(3), t599).write_unaligned(((vgetq_lane_u32::<3>(t597[2]) as u64) << 32 | vgetq_lane_u32::<3>(t597[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t598 as *mut u64).add(4), t599).write_unaligned(((vgetq_lane_u32::<0>(t597[3]) as u64) << 32 | vgetq_lane_u32::<0>(t597[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t598 as *mut u64).add(5), t599).write_unaligned(((vgetq_lane_u32::<1>(t597[3]) as u64) << 32 | vgetq_lane_u32::<1>(t597[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t598 as *mut u64).add(6), t599).write_unaligned(((vgetq_lane_u32::<2>(t597[3]) as u64) << 32 | vgetq_lane_u32::<2>(t597[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t598 as *mut u64).add(7), t599).write_unaligned(((vgetq_lane_u32::<3>(t597[3]) as u64) << 32 | vgetq_lane_u32::<3>(t597[1]) as u64));
            }
            let t600: u64 = 1u64;
            let t601 = [vdupq_n_u32((t600) as u32); 2];
            let t602: [uint32x4_t; 2] = [vandq_u32(x6[0], t601[0]), vandq_u32(x6[1], t601[1])];
            let t603: [uint32x4_t; 2] = [vtstq_u32(t602[0], t602[0]), vtstq_u32(t602[1], t602[1])];
            let t604: [uint32x4_t; 2] = [vandq_u32(live[0], t603[0]), vandq_u32(live[1], t603[1])];
            {
                let t605: [uint32x4_t; 4] = [veorq_u32(x6[0], x7[0]), veorq_u32(x6[1], x7[1]), veorq_u32(x6[2], x7[2]), veorq_u32(x6[3], x7[3])];
                x11 = [vbslq_u32(t604[0], t605[0], x11[0]), vbslq_u32(t604[1], t605[1], x11[1]), vbslq_u32(t604[0], t605[2], x11[2]), vbslq_u32(t604[1], t605[3], x11[3])];
            }
            let t606: [uint32x4_t; 2] = [vbicq_u32(live[0], t603[0]), vbicq_u32(live[1], t603[1])];
            {
                let t607: [uint32x4_t; 2] = [vaddq_u32(x6[0], x8[0]), vaddq_u32(x6[1], x8[1])];
                let t608: [uint32x4_t; 2] = [vcltq_u32(t607[0], x6[0]), vcltq_u32(t607[1], x6[1])];
                let t609: [uint32x4_t; 4] = [t607[0], t607[1], vsubq_u32(vaddq_u32(x6[2], vdupq_n_u32(0)), t608[0]), vsubq_u32(vaddq_u32(x6[3], vdupq_n_u32(0)), t608[1])];
                x11 = [vbslq_u32(t606[0], t609[0], x11[0]), vbslq_u32(t606[1], t609[1], x11[1]), vbslq_u32(t606[0], t609[2], x11[2]), vbslq_u32(t606[1], t609[3], x11[3])];
            }
            let t610 = (arg3 as *const u8).add(((gid0.wrapping_add((15u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t610 as *mut u32).add(0), uint32x4x2_t(x11[0], x11[2]));
                vst2q_u32((t610 as *mut u32).add(8), uint32x4x2_t(x11[1], x11[3]));
            } else {
                let mut t611w = 0u64;
                let t611 = core::hint::black_box(&raw mut t611w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t610 as *mut u64).add(0), t611).write_unaligned(((vgetq_lane_u32::<0>(x11[2]) as u64) << 32 | vgetq_lane_u32::<0>(x11[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t610 as *mut u64).add(1), t611).write_unaligned(((vgetq_lane_u32::<1>(x11[2]) as u64) << 32 | vgetq_lane_u32::<1>(x11[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t610 as *mut u64).add(2), t611).write_unaligned(((vgetq_lane_u32::<2>(x11[2]) as u64) << 32 | vgetq_lane_u32::<2>(x11[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t610 as *mut u64).add(3), t611).write_unaligned(((vgetq_lane_u32::<3>(x11[2]) as u64) << 32 | vgetq_lane_u32::<3>(x11[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t610 as *mut u64).add(4), t611).write_unaligned(((vgetq_lane_u32::<0>(x11[3]) as u64) << 32 | vgetq_lane_u32::<0>(x11[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t610 as *mut u64).add(5), t611).write_unaligned(((vgetq_lane_u32::<1>(x11[3]) as u64) << 32 | vgetq_lane_u32::<1>(x11[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t610 as *mut u64).add(6), t611).write_unaligned(((vgetq_lane_u32::<2>(x11[3]) as u64) << 32 | vgetq_lane_u32::<2>(x11[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t610 as *mut u64).add(7), t611).write_unaligned(((vgetq_lane_u32::<3>(x11[3]) as u64) << 32 | vgetq_lane_u32::<3>(x11[1]) as u64));
            }
            let t612 = b1_l_p(l, lid0, live, x6);
            let t613 = b2_l_p(l, lid0, live, x7);
            let t614: [uint32x4_t; 4] = [vshlq_n_u32::<8>(t613[0]), vshlq_n_u32::<8>(t613[1]), vshrq_n_u32::<24>(t613[0]), vshrq_n_u32::<24>(t613[1])];
            let t615: [uint32x4_t; 2] = [vaddq_u32(t612[0], t614[0]), vaddq_u32(t612[1], t614[1])];
            let t616: [uint32x4_t; 2] = [vcltq_u32(t615[0], t612[0]), vcltq_u32(t615[1], t612[1])];
            let t617: [uint32x4_t; 4] = [t615[0], t615[1], vsubq_u32(vaddq_u32(vdupq_n_u32(0), t614[2]), t616[0]), vsubq_u32(vaddq_u32(vdupq_n_u32(0), t614[3]), t616[1])];
            let t618 = (arg3 as *const u8).add(((gid0.wrapping_add((16u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t618 as *mut u32).add(0), uint32x4x2_t(t617[0], t617[2]));
                vst2q_u32((t618 as *mut u32).add(8), uint32x4x2_t(t617[1], t617[3]));
            } else {
                let mut t619w = 0u64;
                let t619 = core::hint::black_box(&raw mut t619w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t618 as *mut u64).add(0), t619).write_unaligned(((vgetq_lane_u32::<0>(t617[2]) as u64) << 32 | vgetq_lane_u32::<0>(t617[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t618 as *mut u64).add(1), t619).write_unaligned(((vgetq_lane_u32::<1>(t617[2]) as u64) << 32 | vgetq_lane_u32::<1>(t617[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t618 as *mut u64).add(2), t619).write_unaligned(((vgetq_lane_u32::<2>(t617[2]) as u64) << 32 | vgetq_lane_u32::<2>(t617[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t618 as *mut u64).add(3), t619).write_unaligned(((vgetq_lane_u32::<3>(t617[2]) as u64) << 32 | vgetq_lane_u32::<3>(t617[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t618 as *mut u64).add(4), t619).write_unaligned(((vgetq_lane_u32::<0>(t617[3]) as u64) << 32 | vgetq_lane_u32::<0>(t617[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t618 as *mut u64).add(5), t619).write_unaligned(((vgetq_lane_u32::<1>(t617[3]) as u64) << 32 | vgetq_lane_u32::<1>(t617[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t618 as *mut u64).add(6), t619).write_unaligned(((vgetq_lane_u32::<2>(t617[3]) as u64) << 32 | vgetq_lane_u32::<2>(t617[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t618 as *mut u64).add(7), t619).write_unaligned(((vgetq_lane_u32::<3>(t617[3]) as u64) << 32 | vgetq_lane_u32::<3>(t617[1]) as u64));
            }
            let t620 = [vdupq_n_u32(x10); 2];
            let mut t621x = [0u32; 8];
            let mut t621y = [0u32; 8];
            for r in 0..2 { vst1q_u32(t621x.as_mut_ptr().add(4 * r), x8[r]); vst1q_u32(t621y.as_mut_ptr().add(4 * r), t620[r]); }
            for j in 0..8 { t621x[j] = (t621x[j] as u32).checked_rem(t621y[j] as u32).unwrap_or(0) as u32; }
            let t622: [uint32x4_t; 2] = [vld1q_u32(t621x.as_ptr().add(0)), vld1q_u32(t621x.as_ptr().add(4))];
            let t623 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t622[0]), vandq_u32(live[1], t622[1]))));
            let t624: [uint32x4_t; 2] = [vbslq_u32(live[0], t622[0], t623), vbslq_u32(live[1], t622[1], t623)];
            let mut t625: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t624[0]) as usize * 8) as *const u64).read_unaligned(); t625[0] = vsetq_lane_u32::<0>(v as u32, t625[0]); t625[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t625[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t624[0]) as usize * 8) as *const u64).read_unaligned(); t625[0] = vsetq_lane_u32::<1>(v as u32, t625[0]); t625[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t625[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t624[0]) as usize * 8) as *const u64).read_unaligned(); t625[0] = vsetq_lane_u32::<2>(v as u32, t625[0]); t625[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t625[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t624[0]) as usize * 8) as *const u64).read_unaligned(); t625[0] = vsetq_lane_u32::<3>(v as u32, t625[0]); t625[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t625[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t624[1]) as usize * 8) as *const u64).read_unaligned(); t625[1] = vsetq_lane_u32::<0>(v as u32, t625[1]); t625[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t625[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t624[1]) as usize * 8) as *const u64).read_unaligned(); t625[1] = vsetq_lane_u32::<1>(v as u32, t625[1]); t625[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t625[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t624[1]) as usize * 8) as *const u64).read_unaligned(); t625[1] = vsetq_lane_u32::<2>(v as u32, t625[1]); t625[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t625[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t624[1]) as usize * 8) as *const u64).read_unaligned(); t625[1] = vsetq_lane_u32::<3>(v as u32, t625[1]); t625[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t625[3]); }
            let t626 = (arg3 as *const u8).add(((gid0.wrapping_add((17u32).wrapping_mul(x10))) as usize) * 8);
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
            let t628 = b3_ll_p(l, lid0, live, x6, x8);
            let t629 = (arg3 as *const u8).add(((gid0.wrapping_add((18u32).wrapping_mul(x10))) as usize) * 8);
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
            x12 = x6;
            x13 = 0u32;
            let mut t635 = live;
            loop {
                let t637 = bits(&t635);
                if t637.count_ones() <= 2 {
                    let mut t638 = t637;
                    while t638 != 0 {
                        let j = t638.trailing_zeros() as usize;
                        t638 &= t638 - 1;
                        let t639 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x7);
                        let mut x7_j: u64 = ((t639[8 + j] as u64) << 32 | t639[j] as u64);
                        let t640 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x8);
                        let mut x8_j: u32 = t640[j];
                        let t641 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x12);
                        let mut x12_j: u64 = ((t641[8 + j] as u64) << 32 | t641[j] as u64);
                        let mut x13_j: u32 = x13;
                        loop {
                            if !(((x13_j) < (((x8_j) & (7u32))))) { break; }
                            x12_j = ((x12_j).wrapping_mul(3u64)).wrapping_add(x7_j);
                            let t642 = x13_j;
                            x13_j = (x13_j).wrapping_add(1u32);
                        }
                        let mut t643 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x12);
                        t643[j] = x12_j as u32;
                        t643[8 + j] = (x12_j as u64 >> 32) as u32;
                        x12 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t643);
                    }
                    break;
                }
                // Lanes part: each its own rounds.
                let t644 = [vdupq_n_u32(7u32); 2];
                let t645: [uint32x4_t; 2] = [vandq_u32(x8[0], t644[0]), vandq_u32(x8[1], t644[1])];
                let t646 = [vdupq_n_u32(x13); 2];
                let t647: [uint32x4_t; 2] = [vcltq_u32(t646[0], t645[0]), vcltq_u32(t646[1], t645[1])];
                t635 = [vandq_u32(t635[0], t647[0]), vandq_u32(t635[1], t647[1])];
                if !(any(&t635)) { break; }
                let t649: u64 = 3u64;
                let t650 = [vdupq_n_u32((t649) as u32); 2];
                let t651: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x12[0]), vget_low_u32(t650[0]))), vreinterpretq_u32_u64(vmull_high_u32(x12[0], t650[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x12[1]), vget_low_u32(t650[1]))), vreinterpretq_u32_u64(vmull_high_u32(x12[1], t650[1])))];
                let t652: [uint32x4_t; 4] = [vmulq_u32(x12[0], t650[0]), vmulq_u32(x12[1], t650[1]), vmlaq_u32(t651[0], t650[0], x12[2]), vmlaq_u32(t651[1], t650[1], x12[3])];
                let t653: [uint32x4_t; 2] = [vaddq_u32(t652[0], x7[0]), vaddq_u32(t652[1], x7[1])];
                let t654: [uint32x4_t; 2] = [vcltq_u32(t653[0], t652[0]), vcltq_u32(t653[1], t652[1])];
                let t655: [uint32x4_t; 4] = [t653[0], t653[1], vsubq_u32(vaddq_u32(t652[2], x7[2]), t654[0]), vsubq_u32(vaddq_u32(t652[3], x7[3]), t654[1])];
                x12 = [vbslq_u32(t635[0], t655[0], x12[0]), vbslq_u32(t635[1], t655[1], x12[1]), vbslq_u32(t635[0], t655[2], x12[2]), vbslq_u32(t635[1], t655[3], x12[3])];
                let t656: [uint32x4_t; 2] = [vandq_u32(t635[0], live[0]), vandq_u32(t635[1], live[1])];
                let t657 = x13;
                x13 = (x13).wrapping_add(1u32);
            }
            let t658 = (arg3 as *const u8).add(((gid0.wrapping_add((19u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t658 as *mut u32).add(0), uint32x4x2_t(x12[0], x12[2]));
                vst2q_u32((t658 as *mut u32).add(8), uint32x4x2_t(x12[1], x12[3]));
            } else {
                let mut t659w = 0u64;
                let t659 = core::hint::black_box(&raw mut t659w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t658 as *mut u64).add(0), t659).write_unaligned(((vgetq_lane_u32::<0>(x12[2]) as u64) << 32 | vgetq_lane_u32::<0>(x12[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t658 as *mut u64).add(1), t659).write_unaligned(((vgetq_lane_u32::<1>(x12[2]) as u64) << 32 | vgetq_lane_u32::<1>(x12[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t658 as *mut u64).add(2), t659).write_unaligned(((vgetq_lane_u32::<2>(x12[2]) as u64) << 32 | vgetq_lane_u32::<2>(x12[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t658 as *mut u64).add(3), t659).write_unaligned(((vgetq_lane_u32::<3>(x12[2]) as u64) << 32 | vgetq_lane_u32::<3>(x12[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t658 as *mut u64).add(4), t659).write_unaligned(((vgetq_lane_u32::<0>(x12[3]) as u64) << 32 | vgetq_lane_u32::<0>(x12[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t658 as *mut u64).add(5), t659).write_unaligned(((vgetq_lane_u32::<1>(x12[3]) as u64) << 32 | vgetq_lane_u32::<1>(x12[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t658 as *mut u64).add(6), t659).write_unaligned(((vgetq_lane_u32::<2>(x12[3]) as u64) << 32 | vgetq_lane_u32::<2>(x12[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t658 as *mut u64).add(7), t659).write_unaligned(((vgetq_lane_u32::<3>(x12[3]) as u64) << 32 | vgetq_lane_u32::<3>(x12[1]) as u64));
            }
            let t660: u64 = a1;
            let t661 = [vdupq_n_u32((t660) as u32); 2];
            let t662 = [vdupq_n_u32(((t660) >> 32) as u32); 2];
            let t663: [uint32x4_t; 2] = [vaddq_u32(x6[0], t661[0]), vaddq_u32(x6[1], t661[1])];
            let t664: [uint32x4_t; 2] = [vcltq_u32(t663[0], x6[0]), vcltq_u32(t663[1], x6[1])];
            let t665: [uint32x4_t; 4] = [t663[0], t663[1], vsubq_u32(vaddq_u32(x6[2], t662[0]), t664[0]), vsubq_u32(vaddq_u32(x6[3], t662[1]), t664[1])];
            let t666: u64 = (a1).wrapping_shl((3u64) as u32);
            let t667 = [vdupq_n_u32((t666) as u32); 2];
            let t668 = [vdupq_n_u32(((t666) >> 32) as u32); 2];
            let t669: [uint32x4_t; 2] = [vaddq_u32(t665[0], t667[0]), vaddq_u32(t665[1], t667[1])];
            let t670: [uint32x4_t; 2] = [vcltq_u32(t669[0], t665[0]), vcltq_u32(t669[1], t665[1])];
            let t671: [uint32x4_t; 4] = [t669[0], t669[1], vsubq_u32(vaddq_u32(t665[2], t668[0]), t670[0]), vsubq_u32(vaddq_u32(t665[3], t668[1]), t670[1])];
            let t672 = (arg3 as *const u8).add(((gid0.wrapping_add((20u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t672 as *mut u32).add(0), uint32x4x2_t(t671[0], t671[2]));
                vst2q_u32((t672 as *mut u32).add(8), uint32x4x2_t(t671[1], t671[3]));
            } else {
                let mut t673w = 0u64;
                let t673 = core::hint::black_box(&raw mut t673w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t672 as *mut u64).add(0), t673).write_unaligned(((vgetq_lane_u32::<0>(t671[2]) as u64) << 32 | vgetq_lane_u32::<0>(t671[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t672 as *mut u64).add(1), t673).write_unaligned(((vgetq_lane_u32::<1>(t671[2]) as u64) << 32 | vgetq_lane_u32::<1>(t671[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t672 as *mut u64).add(2), t673).write_unaligned(((vgetq_lane_u32::<2>(t671[2]) as u64) << 32 | vgetq_lane_u32::<2>(t671[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t672 as *mut u64).add(3), t673).write_unaligned(((vgetq_lane_u32::<3>(t671[2]) as u64) << 32 | vgetq_lane_u32::<3>(t671[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t672 as *mut u64).add(4), t673).write_unaligned(((vgetq_lane_u32::<0>(t671[3]) as u64) << 32 | vgetq_lane_u32::<0>(t671[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t672 as *mut u64).add(5), t673).write_unaligned(((vgetq_lane_u32::<1>(t671[3]) as u64) << 32 | vgetq_lane_u32::<1>(t671[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t672 as *mut u64).add(6), t673).write_unaligned(((vgetq_lane_u32::<2>(t671[3]) as u64) << 32 | vgetq_lane_u32::<2>(t671[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t672 as *mut u64).add(7), t673).write_unaligned(((vgetq_lane_u32::<3>(t671[3]) as u64) << 32 | vgetq_lane_u32::<3>(t671[1]) as u64));
            }
            let t674: u64 = 4294967295u64;
            let t675 = [vdupq_n_u32((t674) as u32); 2];
            let t676: [uint32x4_t; 2] = [vandq_u32(x6[0], t675[0]), vandq_u32(x6[1], t675[1])];
            let t677: u64 = 4294967295u64;
            let t678 = [vdupq_n_u32((t677) as u32); 2];
            let t679: [uint32x4_t; 2] = [vandq_u32(x7[0], t678[0]), vandq_u32(x7[1], t678[1])];
            let t680: [uint32x4_t; 2] = [vceqq_u32(t676[0], t679[0]), vceqq_u32(t676[1], t679[1])];
            let mut t681: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t682: [uint32x4_t; 2] = [vandq_u32(live[0], t680[0]), vandq_u32(live[1], t680[1])];
            let t683: [uint32x4_t; 2] = [vbicq_u32(live[0], t680[0]), vbicq_u32(live[1], t680[1])];
            {
                let t684 = [vdupq_n_u32(1u32); 2];
                t681 = [vbslq_u32(t682[0], t684[0], t681[0]), vbslq_u32(t682[1], t684[1], t681[1])];
            }
            {
                let t685 = [vdupq_n_u32(2u32); 2];
                t681 = [vbslq_u32(t683[0], t685[0], t681[0]), vbslq_u32(t683[1], t685[1], t681[1])];
            }
            let t686: [uint32x4_t; 4] = [t681[0], t681[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            let t687 = (arg3 as *const u8).add(((gid0.wrapping_add((21u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t687 as *mut u32).add(0), uint32x4x2_t(t686[0], t686[2]));
                vst2q_u32((t687 as *mut u32).add(8), uint32x4x2_t(t686[1], t686[3]));
            } else {
                let mut t688w = 0u64;
                let t688 = core::hint::black_box(&raw mut t688w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t687 as *mut u64).add(0), t688).write_unaligned(((vgetq_lane_u32::<0>(t686[2]) as u64) << 32 | vgetq_lane_u32::<0>(t686[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t687 as *mut u64).add(1), t688).write_unaligned(((vgetq_lane_u32::<1>(t686[2]) as u64) << 32 | vgetq_lane_u32::<1>(t686[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t687 as *mut u64).add(2), t688).write_unaligned(((vgetq_lane_u32::<2>(t686[2]) as u64) << 32 | vgetq_lane_u32::<2>(t686[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t687 as *mut u64).add(3), t688).write_unaligned(((vgetq_lane_u32::<3>(t686[2]) as u64) << 32 | vgetq_lane_u32::<3>(t686[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t687 as *mut u64).add(4), t688).write_unaligned(((vgetq_lane_u32::<0>(t686[3]) as u64) << 32 | vgetq_lane_u32::<0>(t686[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t687 as *mut u64).add(5), t688).write_unaligned(((vgetq_lane_u32::<1>(t686[3]) as u64) << 32 | vgetq_lane_u32::<1>(t686[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t687 as *mut u64).add(6), t688).write_unaligned(((vgetq_lane_u32::<2>(t686[3]) as u64) << 32 | vgetq_lane_u32::<2>(t686[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t687 as *mut u64).add(7), t688).write_unaligned(((vgetq_lane_u32::<3>(t686[3]) as u64) << 32 | vgetq_lane_u32::<3>(t686[1]) as u64));
            }
            let t689: [uint32x4_t; 2] = [vtstq_u32(vorrq_u32(x6[0], x6[2]), vorrq_u32(x6[0], x6[2])), vtstq_u32(vorrq_u32(x6[1], x6[3]), vorrq_u32(x6[1], x6[3]))];
            let mut t690: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t691: [uint32x4_t; 2] = [vandq_u32(live[0], t689[0]), vandq_u32(live[1], t689[1])];
            let t692: [uint32x4_t; 2] = [vbicq_u32(live[0], t689[0]), vbicq_u32(live[1], t689[1])];
            {
                let t693 = [vdupq_n_u32(5u32); 2];
                t690 = [vbslq_u32(t691[0], t693[0], t690[0]), vbslq_u32(t691[1], t693[1], t690[1])];
            }
            {
                let t694 = [vdupq_n_u32(6u32); 2];
                t690 = [vbslq_u32(t692[0], t694[0], t690[0]), vbslq_u32(t692[1], t694[1], t690[1])];
            }
            let t695: [uint32x4_t; 4] = [t690[0], t690[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            let t696 = (arg3 as *const u8).add(((gid0.wrapping_add((22u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t696 as *mut u32).add(0), uint32x4x2_t(t695[0], t695[2]));
                vst2q_u32((t696 as *mut u32).add(8), uint32x4x2_t(t695[1], t695[3]));
            } else {
                let mut t697w = 0u64;
                let t697 = core::hint::black_box(&raw mut t697w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t696 as *mut u64).add(0), t697).write_unaligned(((vgetq_lane_u32::<0>(t695[2]) as u64) << 32 | vgetq_lane_u32::<0>(t695[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t696 as *mut u64).add(1), t697).write_unaligned(((vgetq_lane_u32::<1>(t695[2]) as u64) << 32 | vgetq_lane_u32::<1>(t695[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t696 as *mut u64).add(2), t697).write_unaligned(((vgetq_lane_u32::<2>(t695[2]) as u64) << 32 | vgetq_lane_u32::<2>(t695[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t696 as *mut u64).add(3), t697).write_unaligned(((vgetq_lane_u32::<3>(t695[2]) as u64) << 32 | vgetq_lane_u32::<3>(t695[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t696 as *mut u64).add(4), t697).write_unaligned(((vgetq_lane_u32::<0>(t695[3]) as u64) << 32 | vgetq_lane_u32::<0>(t695[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t696 as *mut u64).add(5), t697).write_unaligned(((vgetq_lane_u32::<1>(t695[3]) as u64) << 32 | vgetq_lane_u32::<1>(t695[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t696 as *mut u64).add(6), t697).write_unaligned(((vgetq_lane_u32::<2>(t695[3]) as u64) << 32 | vgetq_lane_u32::<2>(t695[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t696 as *mut u64).add(7), t697).write_unaligned(((vgetq_lane_u32::<3>(t695[3]) as u64) << 32 | vgetq_lane_u32::<3>(t695[1]) as u64));
            }
            let t698 = b4_ll_p(l, lid0, live, x6, x7);
            let t699 = (arg3 as *const u8).add(((gid0.wrapping_add((23u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t699 as *mut u32).add(0), uint32x4x2_t(t698[0], t698[2]));
                vst2q_u32((t699 as *mut u32).add(8), uint32x4x2_t(t698[1], t698[3]));
            } else {
                let mut t700w = 0u64;
                let t700 = core::hint::black_box(&raw mut t700w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t699 as *mut u64).add(0), t700).write_unaligned(((vgetq_lane_u32::<0>(t698[2]) as u64) << 32 | vgetq_lane_u32::<0>(t698[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t699 as *mut u64).add(1), t700).write_unaligned(((vgetq_lane_u32::<1>(t698[2]) as u64) << 32 | vgetq_lane_u32::<1>(t698[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t699 as *mut u64).add(2), t700).write_unaligned(((vgetq_lane_u32::<2>(t698[2]) as u64) << 32 | vgetq_lane_u32::<2>(t698[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t699 as *mut u64).add(3), t700).write_unaligned(((vgetq_lane_u32::<3>(t698[2]) as u64) << 32 | vgetq_lane_u32::<3>(t698[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t699 as *mut u64).add(4), t700).write_unaligned(((vgetq_lane_u32::<0>(t698[3]) as u64) << 32 | vgetq_lane_u32::<0>(t698[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t699 as *mut u64).add(5), t700).write_unaligned(((vgetq_lane_u32::<1>(t698[3]) as u64) << 32 | vgetq_lane_u32::<1>(t698[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t699 as *mut u64).add(6), t700).write_unaligned(((vgetq_lane_u32::<2>(t698[3]) as u64) << 32 | vgetq_lane_u32::<2>(t698[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t699 as *mut u64).add(7), t700).write_unaligned(((vgetq_lane_u32::<3>(t698[3]) as u64) << 32 | vgetq_lane_u32::<3>(t698[1]) as u64));
            }
            let t701: u32;
            if ((((a1) & (1u64))) != (0u64)) {
                t701 = 8u32;
            } else {
                t701 = 4u32;
            }
            x14 = t701;
            let t702 = b5_r0ns_p(l, lid0, live, arg0, gid0.wrapping_mul(x14), x14, 8u32);
            let t703 = (arg3 as *const u8).add(((gid0.wrapping_add((24u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t703 as *mut u32).add(0), uint32x4x2_t(t702[0], t702[2]));
                vst2q_u32((t703 as *mut u32).add(8), uint32x4x2_t(t702[1], t702[3]));
            } else {
                let mut t704w = 0u64;
                let t704 = core::hint::black_box(&raw mut t704w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t703 as *mut u64).add(0), t704).write_unaligned(((vgetq_lane_u32::<0>(t702[2]) as u64) << 32 | vgetq_lane_u32::<0>(t702[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t703 as *mut u64).add(1), t704).write_unaligned(((vgetq_lane_u32::<1>(t702[2]) as u64) << 32 | vgetq_lane_u32::<1>(t702[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t703 as *mut u64).add(2), t704).write_unaligned(((vgetq_lane_u32::<2>(t702[2]) as u64) << 32 | vgetq_lane_u32::<2>(t702[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t703 as *mut u64).add(3), t704).write_unaligned(((vgetq_lane_u32::<3>(t702[2]) as u64) << 32 | vgetq_lane_u32::<3>(t702[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t703 as *mut u64).add(4), t704).write_unaligned(((vgetq_lane_u32::<0>(t702[3]) as u64) << 32 | vgetq_lane_u32::<0>(t702[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t703 as *mut u64).add(5), t704).write_unaligned(((vgetq_lane_u32::<1>(t702[3]) as u64) << 32 | vgetq_lane_u32::<1>(t702[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t703 as *mut u64).add(6), t704).write_unaligned(((vgetq_lane_u32::<2>(t702[3]) as u64) << 32 | vgetq_lane_u32::<2>(t702[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t703 as *mut u64).add(7), t704).write_unaligned(((vgetq_lane_u32::<3>(t702[3]) as u64) << 32 | vgetq_lane_u32::<3>(t702[1]) as u64));
            }
            let t705 = if (gid0.wrapping_add(3u32) & 255u32) <= 248u32 { b6_u8_p(l, lid0, live, gid0.wrapping_add(3u32)) } else { b6_i_p(l, lid0, live, gid0.wrapping_add(3u32)) };
            let t706 = (arg3 as *const u8).add(((gid0.wrapping_add((25u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t706 as *mut u32).add(0), uint32x4x2_t(t705[0], t705[2]));
                vst2q_u32((t706 as *mut u32).add(8), uint32x4x2_t(t705[1], t705[3]));
            } else {
                let mut t707w = 0u64;
                let t707 = core::hint::black_box(&raw mut t707w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t706 as *mut u64).add(0), t707).write_unaligned(((vgetq_lane_u32::<0>(t705[2]) as u64) << 32 | vgetq_lane_u32::<0>(t705[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t706 as *mut u64).add(1), t707).write_unaligned(((vgetq_lane_u32::<1>(t705[2]) as u64) << 32 | vgetq_lane_u32::<1>(t705[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t706 as *mut u64).add(2), t707).write_unaligned(((vgetq_lane_u32::<2>(t705[2]) as u64) << 32 | vgetq_lane_u32::<2>(t705[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t706 as *mut u64).add(3), t707).write_unaligned(((vgetq_lane_u32::<3>(t705[2]) as u64) << 32 | vgetq_lane_u32::<3>(t705[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t706 as *mut u64).add(4), t707).write_unaligned(((vgetq_lane_u32::<0>(t705[3]) as u64) << 32 | vgetq_lane_u32::<0>(t705[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t706 as *mut u64).add(5), t707).write_unaligned(((vgetq_lane_u32::<1>(t705[3]) as u64) << 32 | vgetq_lane_u32::<1>(t705[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t706 as *mut u64).add(6), t707).write_unaligned(((vgetq_lane_u32::<2>(t705[3]) as u64) << 32 | vgetq_lane_u32::<2>(t705[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t706 as *mut u64).add(7), t707).write_unaligned(((vgetq_lane_u32::<3>(t705[3]) as u64) << 32 | vgetq_lane_u32::<3>(t705[1]) as u64));
            }
            let t708: u64 = 3u64;
            let t709 = [vdupq_n_u32((t708) as u32); 2];
            let t710: [uint32x4_t; 2] = [vandq_u32(x6[0], t709[0]), vandq_u32(x6[1], t709[1])];
            let t711: u64 = 0u64;
            let t712 = [vdupq_n_u32((t711) as u32); 2];
            let t713: [uint32x4_t; 2] = [vceqq_u32(t710[0], t712[0]), vceqq_u32(t710[1], t712[1])];
            let mut t714: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            let t715: [uint32x4_t; 2] = [vandq_u32(live[0], t713[0]), vandq_u32(live[1], t713[1])];
            let t716: [uint32x4_t; 2] = [vbicq_u32(live[0], t713[0]), vbicq_u32(live[1], t713[1])];
            if any(&t715) {
                let t717: [uint32x4_t; 4];
                let t718 = bits(&t715);
                if t718.count_ones() <= 2 {
                    let t719 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x7);
                    let t720 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x8);
                    let mut t721 = [0u32; 16];
                    let mut t722 = t718;
                    while t722 != 0 {
                        let j = t722.trailing_zeros() as usize;
                        t722 &= t722 - 1;
                        let t723 = b7_ss_f(l, lid0, ((t719[8 + j] as u64) << 32 | t719[j] as u64), t720[j]);
                        let t724: u64 = t723;
                        t721[j] = t724 as u32;
                        t721[8 + j] = (t724 >> 32) as u32;
                    }
                    t717 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t721);
                } else {
                    let t725 = b7_ll_p(l, lid0, t715, x7, x8);
                    t717 = t725;
                }
                t714 = [vbslq_u32(t715[0], t717[0], t714[0]), vbslq_u32(t715[1], t717[1], t714[1]), vbslq_u32(t715[0], t717[2], t714[2]), vbslq_u32(t715[1], t717[3], t714[3])];
            }
            {
                t714 = [vbslq_u32(t716[0], x6[0], t714[0]), vbslq_u32(t716[1], x6[1], t714[1]), vbslq_u32(t716[0], x6[2], t714[2]), vbslq_u32(t716[1], x6[3], t714[3])];
            }
            let t726 = (arg3 as *const u8).add(((gid0.wrapping_add((26u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t726 as *mut u32).add(0), uint32x4x2_t(t714[0], t714[2]));
                vst2q_u32((t726 as *mut u32).add(8), uint32x4x2_t(t714[1], t714[3]));
            } else {
                let mut t727w = 0u64;
                let t727 = core::hint::black_box(&raw mut t727w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t726 as *mut u64).add(0), t727).write_unaligned(((vgetq_lane_u32::<0>(t714[2]) as u64) << 32 | vgetq_lane_u32::<0>(t714[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t726 as *mut u64).add(1), t727).write_unaligned(((vgetq_lane_u32::<1>(t714[2]) as u64) << 32 | vgetq_lane_u32::<1>(t714[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t726 as *mut u64).add(2), t727).write_unaligned(((vgetq_lane_u32::<2>(t714[2]) as u64) << 32 | vgetq_lane_u32::<2>(t714[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t726 as *mut u64).add(3), t727).write_unaligned(((vgetq_lane_u32::<3>(t714[2]) as u64) << 32 | vgetq_lane_u32::<3>(t714[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t726 as *mut u64).add(4), t727).write_unaligned(((vgetq_lane_u32::<0>(t714[3]) as u64) << 32 | vgetq_lane_u32::<0>(t714[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t726 as *mut u64).add(5), t727).write_unaligned(((vgetq_lane_u32::<1>(t714[3]) as u64) << 32 | vgetq_lane_u32::<1>(t714[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t726 as *mut u64).add(6), t727).write_unaligned(((vgetq_lane_u32::<2>(t714[3]) as u64) << 32 | vgetq_lane_u32::<2>(t714[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t726 as *mut u64).add(7), t727).write_unaligned(((vgetq_lane_u32::<3>(t714[3]) as u64) << 32 | vgetq_lane_u32::<3>(t714[1]) as u64));
            }
        }
    }
}

#[inline(always)]
unsafe fn b1_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t728: [uint32x4_t; 2] = [vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[0])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[2]))))), vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[1])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[3])))))];
    res = t728;
    return res;
}

#[inline(always)]
unsafe fn b1_l_f(l: &Launch, lid0: u32, x0: [uint32x4_t; 4]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t729: [uint32x4_t; 2] = [vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[0])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[2]))))), vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[1])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[3])))))];
    res = t729;
    return res;
}

#[inline(always)]
unsafe fn b2_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t730: [uint32x4_t; 2] = [vbslq_u32(vceqzq_u32(x0[2]), vaddq_u32(vclzq_u32(x0[0]), vdupq_n_u32(32)), vclzq_u32(x0[2])), vbslq_u32(vceqzq_u32(x0[3]), vaddq_u32(vclzq_u32(x0[1]), vdupq_n_u32(32)), vclzq_u32(x0[3]))];
    res = t730;
    return res;
}

#[inline(always)]
unsafe fn b2_l_f(l: &Launch, lid0: u32, x0: [uint32x4_t; 4]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t731: [uint32x4_t; 2] = [vbslq_u32(vceqzq_u32(x0[2]), vaddq_u32(vclzq_u32(x0[0]), vdupq_n_u32(32)), vclzq_u32(x0[2])), vbslq_u32(vceqzq_u32(x0[3]), vaddq_u32(vclzq_u32(x0[1]), vdupq_n_u32(32)), vclzq_u32(x0[3]))];
    res = t731;
    return res;
}

#[inline(always)]
unsafe fn b3_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], x1: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x2: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t732: u64 = 1u64;
    let t733 = [vdupq_n_u32((t732) as u32); 2];
    let t734: [uint32x4_t; 2] = [vandq_u32(x0[0], t733[0]), vandq_u32(x0[1], t733[1])];
    let t735: u64 = 0u64;
    let t736 = [vdupq_n_u32((t735) as u32); 2];
    let t737: [uint32x4_t; 2] = [vceqq_u32(t734[0], t736[0]), vceqq_u32(t734[1], t736[1])];
    let t738: [uint32x4_t; 2] = [vandq_u32(live[0], t737[0]), vandq_u32(live[1], t737[1])];
    {
        let t739: [uint32x4_t; 4] = [x1[0], x1[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = [vbslq_u32(t738[0], t739[0], res[0]), vbslq_u32(t738[1], t739[1], res[1]), vbslq_u32(t738[0], t739[2], res[2]), vbslq_u32(t738[1], t739[3], res[3])];
        live = [vbicq_u32(live[0], t738[0]), vbicq_u32(live[1], t738[1])];
    }
    if any(&live) {
        let t740: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x0[0]), vshlq_n_u32::<30>(x0[2])), vorrq_u32(vshrq_n_u32::<2>(x0[1]), vshlq_n_u32::<30>(x0[3])), vshrq_n_u32::<2>(x0[2]), vshrq_n_u32::<2>(x0[3])];
        let t741: [uint32x4_t; 2] = [t740[0], t740[1]];
        x2 = t741;
        let t742 = [vdupq_n_u32(1u32); 2];
        let t743: [uint32x4_t; 2] = [vsubq_u32(x2[0], t742[0]), vsubq_u32(x2[1], t742[1])];
        let t744: [uint32x4_t; 2] = [vcgeq_u32(t743[0], x1[0]), vcgeq_u32(t743[1], x1[1])];
        let t745: [uint32x4_t; 2] = [vandq_u32(live[0], t744[0]), vandq_u32(live[1], t744[1])];
        {
            let t746: u64 = 4294967295u64;
            let t747 = [vdupq_n_u32((t746) as u32); 2];
            let t748: [uint32x4_t; 4] = [t747[0], t747[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t745[0], t748[0], res[0]), vbslq_u32(t745[1], t748[1], res[1]), vbslq_u32(t745[0], t748[2], res[2]), vbslq_u32(t745[1], t748[3], res[3])];
            live = [vbicq_u32(live[0], t745[0]), vbicq_u32(live[1], t745[1])];
        }
        if any(&live) {
            let t749: u64 = 2u64;
            let t750 = [vdupq_n_u32((t749) as u32); 2];
            let t751: [uint32x4_t; 2] = [vandq_u32(x0[0], t750[0]), vandq_u32(x0[1], t750[1])];
            let t752: u64 = 0u64;
            let t753 = [vdupq_n_u32((t752) as u32); 2];
            let t754: [uint32x4_t; 2] = [vceqq_u32(t751[0], t753[0]), vceqq_u32(t751[1], t753[1])];
            let t755: [uint32x4_t; 2] = [vmvnq_u32(t754[0]), vmvnq_u32(t754[1])];
            let t756: [uint32x4_t; 2] = [vandq_u32(live[0], t755[0]), vandq_u32(live[1], t755[1])];
            {
                let t757: [uint32x4_t; 2] = [vsubq_u32(x1[0], x2[0]), vsubq_u32(x1[1], x2[1])];
                let t758: [uint32x4_t; 4] = [t757[0], t757[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t756[0], t758[0], res[0]), vbslq_u32(t756[1], t758[1], res[1]), vbslq_u32(t756[0], t758[2], res[2]), vbslq_u32(t756[1], t758[3], res[3])];
                live = [vbicq_u32(live[0], t756[0]), vbicq_u32(live[1], t756[1])];
            }
            if any(&live) {
                let t759: [uint32x4_t; 2] = [vsubq_u32(x1[0], x2[0]), vsubq_u32(x1[1], x2[1])];
                let t760: u64 = (1u64).wrapping_shl((32u64) as u32);
                let t761 = [vdupq_n_u32((t760) as u32); 2];
                let t762 = [vdupq_n_u32(((t760) >> 32) as u32); 2];
                let t763: [uint32x4_t; 4] = [vorrq_u32(t761[0], t759[0]), vorrq_u32(t761[1], t759[1]), t762[0], t762[1]];
                res = [vbslq_u32(live[0], t763[0], res[0]), vbslq_u32(live[1], t763[1], res[1]), vbslq_u32(live[0], t763[2], res[2]), vbslq_u32(live[1], t763[3], res[3])];
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
    let t764: u64 = 1u64;
    let t765 = [vdupq_n_u32((t764) as u32); 2];
    let t766: [uint32x4_t; 2] = [vandq_u32(x0[0], t765[0]), vandq_u32(x0[1], t765[1])];
    let t767: u64 = 0u64;
    let t768 = [vdupq_n_u32((t767) as u32); 2];
    let t769: [uint32x4_t; 2] = [vceqq_u32(t766[0], t768[0]), vceqq_u32(t766[1], t768[1])];
    {
        let t770: [uint32x4_t; 4] = [x1[0], x1[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = [vbslq_u32(t769[0], t770[0], res[0]), vbslq_u32(t769[1], t770[1], res[1]), vbslq_u32(t769[0], t770[2], res[2]), vbslq_u32(t769[1], t770[3], res[3])];
        live = [vbicq_u32(live[0], t769[0]), vbicq_u32(live[1], t769[1])];
    }
    if any(&live) {
        let t771: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x0[0]), vshlq_n_u32::<30>(x0[2])), vorrq_u32(vshrq_n_u32::<2>(x0[1]), vshlq_n_u32::<30>(x0[3])), vshrq_n_u32::<2>(x0[2]), vshrq_n_u32::<2>(x0[3])];
        let t772: [uint32x4_t; 2] = [t771[0], t771[1]];
        x2 = t772;
        let t773 = [vdupq_n_u32(1u32); 2];
        let t774: [uint32x4_t; 2] = [vsubq_u32(x2[0], t773[0]), vsubq_u32(x2[1], t773[1])];
        let t775: [uint32x4_t; 2] = [vcgeq_u32(t774[0], x1[0]), vcgeq_u32(t774[1], x1[1])];
        let t776: [uint32x4_t; 2] = [vandq_u32(live[0], t775[0]), vandq_u32(live[1], t775[1])];
        {
            let t777: u64 = 4294967295u64;
            let t778 = [vdupq_n_u32((t777) as u32); 2];
            let t779: [uint32x4_t; 4] = [t778[0], t778[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t776[0], t779[0], res[0]), vbslq_u32(t776[1], t779[1], res[1]), vbslq_u32(t776[0], t779[2], res[2]), vbslq_u32(t776[1], t779[3], res[3])];
            live = [vbicq_u32(live[0], t776[0]), vbicq_u32(live[1], t776[1])];
        }
        if any(&live) {
            let t780: u64 = 2u64;
            let t781 = [vdupq_n_u32((t780) as u32); 2];
            let t782: [uint32x4_t; 2] = [vandq_u32(x0[0], t781[0]), vandq_u32(x0[1], t781[1])];
            let t783: u64 = 0u64;
            let t784 = [vdupq_n_u32((t783) as u32); 2];
            let t785: [uint32x4_t; 2] = [vceqq_u32(t782[0], t784[0]), vceqq_u32(t782[1], t784[1])];
            let t786: [uint32x4_t; 2] = [vmvnq_u32(t785[0]), vmvnq_u32(t785[1])];
            let t787: [uint32x4_t; 2] = [vandq_u32(live[0], t786[0]), vandq_u32(live[1], t786[1])];
            {
                let t788: [uint32x4_t; 2] = [vsubq_u32(x1[0], x2[0]), vsubq_u32(x1[1], x2[1])];
                let t789: [uint32x4_t; 4] = [t788[0], t788[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t787[0], t789[0], res[0]), vbslq_u32(t787[1], t789[1], res[1]), vbslq_u32(t787[0], t789[2], res[2]), vbslq_u32(t787[1], t789[3], res[3])];
                live = [vbicq_u32(live[0], t787[0]), vbicq_u32(live[1], t787[1])];
            }
            if any(&live) {
                let t790: [uint32x4_t; 2] = [vsubq_u32(x1[0], x2[0]), vsubq_u32(x1[1], x2[1])];
                let t791: u64 = (1u64).wrapping_shl((32u64) as u32);
                let t792 = [vdupq_n_u32((t791) as u32); 2];
                let t793 = [vdupq_n_u32(((t791) >> 32) as u32); 2];
                let t794: [uint32x4_t; 4] = [vorrq_u32(t792[0], t790[0]), vorrq_u32(t792[1], t790[1]), t793[0], t793[1]];
                res = [vbslq_u32(live[0], t794[0], res[0]), vbslq_u32(live[1], t794[1], res[1]), vbslq_u32(live[0], t794[2], res[2]), vbslq_u32(live[1], t794[3], res[3])];
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
    let (mut t795l, mut t795h) = ([0u32; 8], [0u32; 8]);
    { let v = (((((vgetq_lane_u32::<0>(x0[2]) as u64) << 32 | vgetq_lane_u32::<0>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<0>(x1[2]) as u64) << 32 | vgetq_lane_u32::<0>(x1[0]) as u64) as u64) as u128) >> 64) as u64; t795l[0] = v as u32; t795h[0] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<1>(x0[2]) as u64) << 32 | vgetq_lane_u32::<1>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<1>(x1[2]) as u64) << 32 | vgetq_lane_u32::<1>(x1[0]) as u64) as u64) as u128) >> 64) as u64; t795l[1] = v as u32; t795h[1] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<2>(x0[2]) as u64) << 32 | vgetq_lane_u32::<2>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<2>(x1[2]) as u64) << 32 | vgetq_lane_u32::<2>(x1[0]) as u64) as u64) as u128) >> 64) as u64; t795l[2] = v as u32; t795h[2] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<3>(x0[2]) as u64) << 32 | vgetq_lane_u32::<3>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<3>(x1[2]) as u64) << 32 | vgetq_lane_u32::<3>(x1[0]) as u64) as u64) as u128) >> 64) as u64; t795l[3] = v as u32; t795h[3] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<0>(x0[3]) as u64) << 32 | vgetq_lane_u32::<0>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<0>(x1[3]) as u64) << 32 | vgetq_lane_u32::<0>(x1[1]) as u64) as u64) as u128) >> 64) as u64; t795l[4] = v as u32; t795h[4] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<1>(x0[3]) as u64) << 32 | vgetq_lane_u32::<1>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<1>(x1[3]) as u64) << 32 | vgetq_lane_u32::<1>(x1[1]) as u64) as u64) as u128) >> 64) as u64; t795l[5] = v as u32; t795h[5] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<2>(x0[3]) as u64) << 32 | vgetq_lane_u32::<2>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<2>(x1[3]) as u64) << 32 | vgetq_lane_u32::<2>(x1[1]) as u64) as u64) as u128) >> 64) as u64; t795l[6] = v as u32; t795h[6] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<3>(x0[3]) as u64) << 32 | vgetq_lane_u32::<3>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<3>(x1[3]) as u64) << 32 | vgetq_lane_u32::<3>(x1[1]) as u64) as u64) as u128) >> 64) as u64; t795l[7] = v as u32; t795h[7] = (v >> 32) as u32; }
    let t796: [uint32x4_t; 4] = [vld1q_u32(t795l.as_ptr().add(0)), vld1q_u32(t795l.as_ptr().add(4)), vld1q_u32(t795h.as_ptr().add(0)), vld1q_u32(t795h.as_ptr().add(4))];
    res = t796;
    return res;
}

#[inline(always)]
unsafe fn b4_ll_f(l: &Launch, lid0: u32, x0: [uint32x4_t; 4], x1: [uint32x4_t; 4]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let (mut t797l, mut t797h) = ([0u32; 8], [0u32; 8]);
    { let v = (((((vgetq_lane_u32::<0>(x0[2]) as u64) << 32 | vgetq_lane_u32::<0>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<0>(x1[2]) as u64) << 32 | vgetq_lane_u32::<0>(x1[0]) as u64) as u64) as u128) >> 64) as u64; t797l[0] = v as u32; t797h[0] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<1>(x0[2]) as u64) << 32 | vgetq_lane_u32::<1>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<1>(x1[2]) as u64) << 32 | vgetq_lane_u32::<1>(x1[0]) as u64) as u64) as u128) >> 64) as u64; t797l[1] = v as u32; t797h[1] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<2>(x0[2]) as u64) << 32 | vgetq_lane_u32::<2>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<2>(x1[2]) as u64) << 32 | vgetq_lane_u32::<2>(x1[0]) as u64) as u64) as u128) >> 64) as u64; t797l[2] = v as u32; t797h[2] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<3>(x0[2]) as u64) << 32 | vgetq_lane_u32::<3>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<3>(x1[2]) as u64) << 32 | vgetq_lane_u32::<3>(x1[0]) as u64) as u64) as u128) >> 64) as u64; t797l[3] = v as u32; t797h[3] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<0>(x0[3]) as u64) << 32 | vgetq_lane_u32::<0>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<0>(x1[3]) as u64) << 32 | vgetq_lane_u32::<0>(x1[1]) as u64) as u64) as u128) >> 64) as u64; t797l[4] = v as u32; t797h[4] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<1>(x0[3]) as u64) << 32 | vgetq_lane_u32::<1>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<1>(x1[3]) as u64) << 32 | vgetq_lane_u32::<1>(x1[1]) as u64) as u64) as u128) >> 64) as u64; t797l[5] = v as u32; t797h[5] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<2>(x0[3]) as u64) << 32 | vgetq_lane_u32::<2>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<2>(x1[3]) as u64) << 32 | vgetq_lane_u32::<2>(x1[1]) as u64) as u64) as u128) >> 64) as u64; t797l[6] = v as u32; t797h[6] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<3>(x0[3]) as u64) << 32 | vgetq_lane_u32::<3>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<3>(x1[3]) as u64) << 32 | vgetq_lane_u32::<3>(x1[1]) as u64) as u64) as u128) >> 64) as u64; t797l[7] = v as u32; t797h[7] = (v >> 32) as u32; }
    let t798: [uint32x4_t; 4] = [vld1q_u32(t797l.as_ptr().add(0)), vld1q_u32(t797l.as_ptr().add(4)), vld1q_u32(t797h.as_ptr().add(0)), vld1q_u32(t797h.as_ptr().add(4))];
    res = t798;
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
        let t799: [uint32x4_t; 4];
        if (x1s) == 8u32 && ((x1f) as u64) + 7 * ((x1s) as u64) <= u32::MAX as u64 && all(&live) {
            let t800 = (arg0 as *const u8).add(((x1f) as usize) * 1);
            let t8010 = vld2q_u32((t800 as *const u32).add(0));
            let t8011 = vld2q_u32((t800 as *const u32).add(8));
            let t801: [uint32x4_t; 4] = [t8010.0, t8011.0, t8010.1, t8011.1];
            t799 = t801;
        } else {
            let t802: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1s)), vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1s))];
            let t803 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t802[0]), vandq_u32(live[1], t802[1]))));
            let t804: [uint32x4_t; 2] = [vbslq_u32(live[0], t802[0], t803), vbslq_u32(live[1], t802[1], t803)];
            let mut t805: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t804[0]) as usize * 1) as *const u64).read_unaligned(); t805[0] = vsetq_lane_u32::<0>(v as u32, t805[0]); t805[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t805[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t804[0]) as usize * 1) as *const u64).read_unaligned(); t805[0] = vsetq_lane_u32::<1>(v as u32, t805[0]); t805[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t805[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t804[0]) as usize * 1) as *const u64).read_unaligned(); t805[0] = vsetq_lane_u32::<2>(v as u32, t805[0]); t805[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t805[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t804[0]) as usize * 1) as *const u64).read_unaligned(); t805[0] = vsetq_lane_u32::<3>(v as u32, t805[0]); t805[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t805[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t804[1]) as usize * 1) as *const u64).read_unaligned(); t805[1] = vsetq_lane_u32::<0>(v as u32, t805[1]); t805[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t805[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t804[1]) as usize * 1) as *const u64).read_unaligned(); t805[1] = vsetq_lane_u32::<1>(v as u32, t805[1]); t805[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t805[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t804[1]) as usize * 1) as *const u64).read_unaligned(); t805[1] = vsetq_lane_u32::<2>(v as u32, t805[1]); t805[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t805[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t804[1]) as usize * 1) as *const u64).read_unaligned(); t805[1] = vsetq_lane_u32::<3>(v as u32, t805[1]); t805[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t805[3]); }
            t799 = t805;
        }
        res = t799;
        return res;
    }
    let t806: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1s)), vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1s))];
    let t807 = [vdupq_n_u32(!(7u32)); 2];
    let t808: [uint32x4_t; 2] = [vandq_u32(t806[0], t807[0]), vandq_u32(t806[1], t807[1])];
    let t809: [uint32x4_t; 2] = [t808[0], t808[1]];
    let t810: u64 = ((arg0) as usize as u64);
    let t811 = [vdupq_n_u32((t810) as u32); 2];
    let t812 = [vdupq_n_u32(((t810) >> 32) as u32); 2];
    let t813: [uint32x4_t; 2] = [vaddq_u32(t811[0], t809[0]), vaddq_u32(t811[1], t809[1])];
    let t814: [uint32x4_t; 2] = [vcltq_u32(t813[0], t811[0]), vcltq_u32(t813[1], t811[1])];
    let t815: [uint32x4_t; 4] = [t813[0], t813[1], vsubq_u32(vaddq_u32(t812[0], vdupq_n_u32(0)), t814[0]), vsubq_u32(vaddq_u32(t812[1], vdupq_n_u32(0)), t814[1])];
    let t816: [uint32x4_t; 4] = [t815[0], t815[1], t815[2], t815[3]];
    x3 = t816;
    let t817: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1s)), vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1s))];
    let t818 = [vdupq_n_u32(7u32); 2];
    let t819: [uint32x4_t; 2] = [vandq_u32(t817[0], t818[0]), vandq_u32(t817[1], t818[1])];
    let t820 = [vdupq_n_u32(8u32); 2];
    let t821: [uint32x4_t; 2] = [vmulq_u32(t819[0], t820[0]), vmulq_u32(t819[1], t820[1])];
    x4 = t821;
    let mut t822: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x3[2]) as u64) << 32 | vgetq_lane_u32::<0>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t822[0] = vsetq_lane_u32::<0>(v as u32, t822[0]); t822[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t822[2]); }
    { let v = (((vgetq_lane_u32::<1>(x3[2]) as u64) << 32 | vgetq_lane_u32::<1>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t822[0] = vsetq_lane_u32::<1>(v as u32, t822[0]); t822[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t822[2]); }
    { let v = (((vgetq_lane_u32::<2>(x3[2]) as u64) << 32 | vgetq_lane_u32::<2>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t822[0] = vsetq_lane_u32::<2>(v as u32, t822[0]); t822[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t822[2]); }
    { let v = (((vgetq_lane_u32::<3>(x3[2]) as u64) << 32 | vgetq_lane_u32::<3>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t822[0] = vsetq_lane_u32::<3>(v as u32, t822[0]); t822[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t822[2]); }
    { let v = (((vgetq_lane_u32::<0>(x3[3]) as u64) << 32 | vgetq_lane_u32::<0>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t822[1] = vsetq_lane_u32::<0>(v as u32, t822[1]); t822[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t822[3]); }
    { let v = (((vgetq_lane_u32::<1>(x3[3]) as u64) << 32 | vgetq_lane_u32::<1>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t822[1] = vsetq_lane_u32::<1>(v as u32, t822[1]); t822[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t822[3]); }
    { let v = (((vgetq_lane_u32::<2>(x3[3]) as u64) << 32 | vgetq_lane_u32::<2>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t822[1] = vsetq_lane_u32::<2>(v as u32, t822[1]); t822[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t822[3]); }
    { let v = (((vgetq_lane_u32::<3>(x3[3]) as u64) << 32 | vgetq_lane_u32::<3>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t822[1] = vsetq_lane_u32::<3>(v as u32, t822[1]); t822[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t822[3]); }
    let t823: [uint32x4_t; 2] = [x4[0], x4[1]];
    let t824: [uint32x4_t; 2] = [vandq_u32(t823[0], vdupq_n_u32(63)), vandq_u32(t823[1], vdupq_n_u32(63))];
    let t825: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t822[0], vnegq_s32(vreinterpretq_s32_u32(t824[0]))), vshlq_u32(t822[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t824[0])))), vorrq_u32(vshlq_u32(t822[1], vnegq_s32(vreinterpretq_s32_u32(t824[1]))), vshlq_u32(t822[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t824[1])))), vshlq_u32(t822[2], vnegq_s32(vreinterpretq_s32_u32(t824[0]))), vshlq_u32(t822[3], vnegq_s32(vreinterpretq_s32_u32(t824[1])))];
    x5 = t825;
    let t826 = [vdupq_n_u32(0u32); 2];
    let t827: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t826[0])), vmvnq_u32(vceqq_u32(x4[1], t826[1]))];
    let t828 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
    let t829: [uint32x4_t; 2] = [vaddq_u32(x4[0], t828[0]), vaddq_u32(x4[1], t828[1])];
    let t830 = [vdupq_n_u32(64u32); 2];
    let t831: [uint32x4_t; 2] = [vcgtq_u32(t829[0], t830[0]), vcgtq_u32(t829[1], t830[1])];
    let t832: [uint32x4_t; 2] = [vandq_u32(t827[0], t831[0]), vandq_u32(t827[1], t831[1])];
    let mut t833: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t834: [uint32x4_t; 2] = [vandq_u32(live[0], t832[0]), vandq_u32(live[1], t832[1])];
    let t835: [uint32x4_t; 2] = [vbicq_u32(live[0], t832[0]), vbicq_u32(live[1], t832[1])];
    if any(&t834) {
        let t836: u64 = (1u64).wrapping_mul(8u64);
        let t837 = [vdupq_n_u32((t836) as u32); 2];
        let t838 = [vdupq_n_u32(((t836) >> 32) as u32); 2];
        let t839: [uint32x4_t; 2] = [vaddq_u32(x3[0], t837[0]), vaddq_u32(x3[1], t837[1])];
        let t840: [uint32x4_t; 2] = [vcltq_u32(t839[0], x3[0]), vcltq_u32(t839[1], x3[1])];
        let t841: [uint32x4_t; 4] = [t839[0], t839[1], vsubq_u32(vaddq_u32(x3[2], t838[0]), t840[0]), vsubq_u32(vaddq_u32(x3[3], t838[1]), t840[1])];
        let mut t842: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t841[2]) as u64) << 32 | vgetq_lane_u32::<0>(t841[0]) as u64) as usize as *const u64).read_unaligned(); t842[0] = vsetq_lane_u32::<0>(v as u32, t842[0]); t842[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t842[2]); }
        { let v = (((vgetq_lane_u32::<1>(t841[2]) as u64) << 32 | vgetq_lane_u32::<1>(t841[0]) as u64) as usize as *const u64).read_unaligned(); t842[0] = vsetq_lane_u32::<1>(v as u32, t842[0]); t842[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t842[2]); }
        { let v = (((vgetq_lane_u32::<2>(t841[2]) as u64) << 32 | vgetq_lane_u32::<2>(t841[0]) as u64) as usize as *const u64).read_unaligned(); t842[0] = vsetq_lane_u32::<2>(v as u32, t842[0]); t842[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t842[2]); }
        { let v = (((vgetq_lane_u32::<3>(t841[2]) as u64) << 32 | vgetq_lane_u32::<3>(t841[0]) as u64) as usize as *const u64).read_unaligned(); t842[0] = vsetq_lane_u32::<3>(v as u32, t842[0]); t842[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t842[2]); }
        { let v = (((vgetq_lane_u32::<0>(t841[3]) as u64) << 32 | vgetq_lane_u32::<0>(t841[1]) as u64) as usize as *const u64).read_unaligned(); t842[1] = vsetq_lane_u32::<0>(v as u32, t842[1]); t842[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t842[3]); }
        { let v = (((vgetq_lane_u32::<1>(t841[3]) as u64) << 32 | vgetq_lane_u32::<1>(t841[1]) as u64) as usize as *const u64).read_unaligned(); t842[1] = vsetq_lane_u32::<1>(v as u32, t842[1]); t842[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t842[3]); }
        { let v = (((vgetq_lane_u32::<2>(t841[3]) as u64) << 32 | vgetq_lane_u32::<2>(t841[1]) as u64) as usize as *const u64).read_unaligned(); t842[1] = vsetq_lane_u32::<2>(v as u32, t842[1]); t842[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t842[3]); }
        { let v = (((vgetq_lane_u32::<3>(t841[3]) as u64) << 32 | vgetq_lane_u32::<3>(t841[1]) as u64) as usize as *const u64).read_unaligned(); t842[1] = vsetq_lane_u32::<3>(v as u32, t842[1]); t842[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t842[3]); }
        let t843: [uint32x4_t; 2] = [vsubq_u32(t830[0], x4[0]), vsubq_u32(t830[1], x4[1])];
        let t844: [uint32x4_t; 2] = [t843[0], t843[1]];
        let t845: [uint32x4_t; 2] = [vandq_u32(t844[0], vdupq_n_u32(63)), vandq_u32(t844[1], vdupq_n_u32(63))];
        let t846: [uint32x4_t; 4] = [vshlq_u32(t842[0], vreinterpretq_s32_u32(t845[0])), vshlq_u32(t842[1], vreinterpretq_s32_u32(t845[1])), vorrq_u32(vshlq_u32(t842[2], vreinterpretq_s32_u32(t845[0])), vshlq_u32(t842[0], vsubq_s32(vreinterpretq_s32_u32(t845[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t842[3], vreinterpretq_s32_u32(t845[1])), vshlq_u32(t842[1], vsubq_s32(vreinterpretq_s32_u32(t845[1]), vdupq_n_s32(32))))];
        let t847: [uint32x4_t; 4] = [vorrq_u32(x5[0], t846[0]), vorrq_u32(x5[1], t846[1]), vorrq_u32(x5[2], t846[2]), vorrq_u32(x5[3], t846[3])];
        t833 = [vbslq_u32(t834[0], t847[0], t833[0]), vbslq_u32(t834[1], t847[1], t833[1]), vbslq_u32(t834[0], t847[2], t833[2]), vbslq_u32(t834[1], t847[3], t833[3])];
    }
    {
        t833 = [vbslq_u32(t835[0], x5[0], t833[0]), vbslq_u32(t835[1], x5[1], t833[1]), vbslq_u32(t835[0], x5[2], t833[2]), vbslq_u32(t835[1], x5[3], t833[3])];
    }
    res = t833;
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
        let t848: [uint32x4_t; 4];
        if (x1s) == 8u32 && ((x1f) as u64) + 7 * ((x1s) as u64) <= u32::MAX as u64 {
            let t849 = (arg0 as *const u8).add(((x1f) as usize) * 1);
            let t8500 = vld2q_u32((t849 as *const u32).add(0));
            let t8501 = vld2q_u32((t849 as *const u32).add(8));
            let t850: [uint32x4_t; 4] = [t8500.0, t8501.0, t8500.1, t8501.1];
            t848 = t850;
        } else {
            let t851: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1s)), vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1s))];
            let mut t852: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t851[0]) as usize * 1) as *const u64).read_unaligned(); t852[0] = vsetq_lane_u32::<0>(v as u32, t852[0]); t852[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t852[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t851[0]) as usize * 1) as *const u64).read_unaligned(); t852[0] = vsetq_lane_u32::<1>(v as u32, t852[0]); t852[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t852[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t851[0]) as usize * 1) as *const u64).read_unaligned(); t852[0] = vsetq_lane_u32::<2>(v as u32, t852[0]); t852[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t852[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t851[0]) as usize * 1) as *const u64).read_unaligned(); t852[0] = vsetq_lane_u32::<3>(v as u32, t852[0]); t852[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t852[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t851[1]) as usize * 1) as *const u64).read_unaligned(); t852[1] = vsetq_lane_u32::<0>(v as u32, t852[1]); t852[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t852[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t851[1]) as usize * 1) as *const u64).read_unaligned(); t852[1] = vsetq_lane_u32::<1>(v as u32, t852[1]); t852[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t852[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t851[1]) as usize * 1) as *const u64).read_unaligned(); t852[1] = vsetq_lane_u32::<2>(v as u32, t852[1]); t852[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t852[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t851[1]) as usize * 1) as *const u64).read_unaligned(); t852[1] = vsetq_lane_u32::<3>(v as u32, t852[1]); t852[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t852[3]); }
            t848 = t852;
        }
        res = t848;
        return res;
    }
    let t853: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1s)), vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1s))];
    let t854 = [vdupq_n_u32(!(7u32)); 2];
    let t855: [uint32x4_t; 2] = [vandq_u32(t853[0], t854[0]), vandq_u32(t853[1], t854[1])];
    let t856: [uint32x4_t; 2] = [t855[0], t855[1]];
    let t857: u64 = ((arg0) as usize as u64);
    let t858 = [vdupq_n_u32((t857) as u32); 2];
    let t859 = [vdupq_n_u32(((t857) >> 32) as u32); 2];
    let t860: [uint32x4_t; 2] = [vaddq_u32(t858[0], t856[0]), vaddq_u32(t858[1], t856[1])];
    let t861: [uint32x4_t; 2] = [vcltq_u32(t860[0], t858[0]), vcltq_u32(t860[1], t858[1])];
    let t862: [uint32x4_t; 4] = [t860[0], t860[1], vsubq_u32(vaddq_u32(t859[0], vdupq_n_u32(0)), t861[0]), vsubq_u32(vaddq_u32(t859[1], vdupq_n_u32(0)), t861[1])];
    let t863: [uint32x4_t; 4] = [t862[0], t862[1], t862[2], t862[3]];
    x3 = t863;
    let t864: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1s)), vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1s))];
    let t865 = [vdupq_n_u32(7u32); 2];
    let t866: [uint32x4_t; 2] = [vandq_u32(t864[0], t865[0]), vandq_u32(t864[1], t865[1])];
    let t867 = [vdupq_n_u32(8u32); 2];
    let t868: [uint32x4_t; 2] = [vmulq_u32(t866[0], t867[0]), vmulq_u32(t866[1], t867[1])];
    x4 = t868;
    let mut t869: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x3[2]) as u64) << 32 | vgetq_lane_u32::<0>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t869[0] = vsetq_lane_u32::<0>(v as u32, t869[0]); t869[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t869[2]); }
    { let v = (((vgetq_lane_u32::<1>(x3[2]) as u64) << 32 | vgetq_lane_u32::<1>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t869[0] = vsetq_lane_u32::<1>(v as u32, t869[0]); t869[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t869[2]); }
    { let v = (((vgetq_lane_u32::<2>(x3[2]) as u64) << 32 | vgetq_lane_u32::<2>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t869[0] = vsetq_lane_u32::<2>(v as u32, t869[0]); t869[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t869[2]); }
    { let v = (((vgetq_lane_u32::<3>(x3[2]) as u64) << 32 | vgetq_lane_u32::<3>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t869[0] = vsetq_lane_u32::<3>(v as u32, t869[0]); t869[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t869[2]); }
    { let v = (((vgetq_lane_u32::<0>(x3[3]) as u64) << 32 | vgetq_lane_u32::<0>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t869[1] = vsetq_lane_u32::<0>(v as u32, t869[1]); t869[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t869[3]); }
    { let v = (((vgetq_lane_u32::<1>(x3[3]) as u64) << 32 | vgetq_lane_u32::<1>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t869[1] = vsetq_lane_u32::<1>(v as u32, t869[1]); t869[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t869[3]); }
    { let v = (((vgetq_lane_u32::<2>(x3[3]) as u64) << 32 | vgetq_lane_u32::<2>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t869[1] = vsetq_lane_u32::<2>(v as u32, t869[1]); t869[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t869[3]); }
    { let v = (((vgetq_lane_u32::<3>(x3[3]) as u64) << 32 | vgetq_lane_u32::<3>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t869[1] = vsetq_lane_u32::<3>(v as u32, t869[1]); t869[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t869[3]); }
    let t870: [uint32x4_t; 2] = [x4[0], x4[1]];
    let t871: [uint32x4_t; 2] = [vandq_u32(t870[0], vdupq_n_u32(63)), vandq_u32(t870[1], vdupq_n_u32(63))];
    let t872: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t869[0], vnegq_s32(vreinterpretq_s32_u32(t871[0]))), vshlq_u32(t869[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t871[0])))), vorrq_u32(vshlq_u32(t869[1], vnegq_s32(vreinterpretq_s32_u32(t871[1]))), vshlq_u32(t869[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t871[1])))), vshlq_u32(t869[2], vnegq_s32(vreinterpretq_s32_u32(t871[0]))), vshlq_u32(t869[3], vnegq_s32(vreinterpretq_s32_u32(t871[1])))];
    x5 = t872;
    let t873 = [vdupq_n_u32(0u32); 2];
    let t874: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t873[0])), vmvnq_u32(vceqq_u32(x4[1], t873[1]))];
    let t875 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
    let t876: [uint32x4_t; 2] = [vaddq_u32(x4[0], t875[0]), vaddq_u32(x4[1], t875[1])];
    let t877 = [vdupq_n_u32(64u32); 2];
    let t878: [uint32x4_t; 2] = [vcgtq_u32(t876[0], t877[0]), vcgtq_u32(t876[1], t877[1])];
    let t879: [uint32x4_t; 2] = [vandq_u32(t874[0], t878[0]), vandq_u32(t874[1], t878[1])];
    let mut t880: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t881: [uint32x4_t; 2] = [vmvnq_u32(t879[0]), vmvnq_u32(t879[1])];
    if any(&t879) {
        let t882: u64 = (1u64).wrapping_mul(8u64);
        let t883 = [vdupq_n_u32((t882) as u32); 2];
        let t884 = [vdupq_n_u32(((t882) >> 32) as u32); 2];
        let t885: [uint32x4_t; 2] = [vaddq_u32(x3[0], t883[0]), vaddq_u32(x3[1], t883[1])];
        let t886: [uint32x4_t; 2] = [vcltq_u32(t885[0], x3[0]), vcltq_u32(t885[1], x3[1])];
        let t887: [uint32x4_t; 4] = [t885[0], t885[1], vsubq_u32(vaddq_u32(x3[2], t884[0]), t886[0]), vsubq_u32(vaddq_u32(x3[3], t884[1]), t886[1])];
        let mut t888: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t887[2]) as u64) << 32 | vgetq_lane_u32::<0>(t887[0]) as u64) as usize as *const u64).read_unaligned(); t888[0] = vsetq_lane_u32::<0>(v as u32, t888[0]); t888[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t888[2]); }
        { let v = (((vgetq_lane_u32::<1>(t887[2]) as u64) << 32 | vgetq_lane_u32::<1>(t887[0]) as u64) as usize as *const u64).read_unaligned(); t888[0] = vsetq_lane_u32::<1>(v as u32, t888[0]); t888[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t888[2]); }
        { let v = (((vgetq_lane_u32::<2>(t887[2]) as u64) << 32 | vgetq_lane_u32::<2>(t887[0]) as u64) as usize as *const u64).read_unaligned(); t888[0] = vsetq_lane_u32::<2>(v as u32, t888[0]); t888[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t888[2]); }
        { let v = (((vgetq_lane_u32::<3>(t887[2]) as u64) << 32 | vgetq_lane_u32::<3>(t887[0]) as u64) as usize as *const u64).read_unaligned(); t888[0] = vsetq_lane_u32::<3>(v as u32, t888[0]); t888[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t888[2]); }
        { let v = (((vgetq_lane_u32::<0>(t887[3]) as u64) << 32 | vgetq_lane_u32::<0>(t887[1]) as u64) as usize as *const u64).read_unaligned(); t888[1] = vsetq_lane_u32::<0>(v as u32, t888[1]); t888[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t888[3]); }
        { let v = (((vgetq_lane_u32::<1>(t887[3]) as u64) << 32 | vgetq_lane_u32::<1>(t887[1]) as u64) as usize as *const u64).read_unaligned(); t888[1] = vsetq_lane_u32::<1>(v as u32, t888[1]); t888[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t888[3]); }
        { let v = (((vgetq_lane_u32::<2>(t887[3]) as u64) << 32 | vgetq_lane_u32::<2>(t887[1]) as u64) as usize as *const u64).read_unaligned(); t888[1] = vsetq_lane_u32::<2>(v as u32, t888[1]); t888[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t888[3]); }
        { let v = (((vgetq_lane_u32::<3>(t887[3]) as u64) << 32 | vgetq_lane_u32::<3>(t887[1]) as u64) as usize as *const u64).read_unaligned(); t888[1] = vsetq_lane_u32::<3>(v as u32, t888[1]); t888[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t888[3]); }
        let t889: [uint32x4_t; 2] = [vsubq_u32(t877[0], x4[0]), vsubq_u32(t877[1], x4[1])];
        let t890: [uint32x4_t; 2] = [t889[0], t889[1]];
        let t891: [uint32x4_t; 2] = [vandq_u32(t890[0], vdupq_n_u32(63)), vandq_u32(t890[1], vdupq_n_u32(63))];
        let t892: [uint32x4_t; 4] = [vshlq_u32(t888[0], vreinterpretq_s32_u32(t891[0])), vshlq_u32(t888[1], vreinterpretq_s32_u32(t891[1])), vorrq_u32(vshlq_u32(t888[2], vreinterpretq_s32_u32(t891[0])), vshlq_u32(t888[0], vsubq_s32(vreinterpretq_s32_u32(t891[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t888[3], vreinterpretq_s32_u32(t891[1])), vshlq_u32(t888[1], vsubq_s32(vreinterpretq_s32_u32(t891[1]), vdupq_n_s32(32))))];
        let t893: [uint32x4_t; 4] = [vorrq_u32(x5[0], t892[0]), vorrq_u32(x5[1], t892[1]), vorrq_u32(x5[2], t892[2]), vorrq_u32(x5[3], t892[3])];
        t880 = [vbslq_u32(t879[0], t893[0], t880[0]), vbslq_u32(t879[1], t893[1], t880[1]), vbslq_u32(t879[0], t893[2], t880[2]), vbslq_u32(t879[1], t893[3], t880[3])];
    }
    {
        t880 = [vbslq_u32(t881[0], x5[0], t880[0]), vbslq_u32(t881[1], x5[1], t880[1]), vbslq_u32(t881[0], x5[2], t880[2]), vbslq_u32(t881[1], x5[3], t880[3])];
    }
    res = t880;
    return res;
}

#[inline(always)]
unsafe fn b6_i_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t894: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x0), vld1q_u32(LANE[1].as_ptr()))];
    let t895: [uint32x4_t; 2] = [vshlq_u32(t894[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(t894[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
    let t896 = [vdupq_n_u32(255u32); 2];
    let t897: [uint32x4_t; 2] = [vandq_u32(t894[0], t896[0]), vandq_u32(t894[1], t896[1])];
    let t898: [uint32x4_t; 2] = [vmulq_u32(t895[0], vdupq_n_u32(1000)), vmulq_u32(t895[1], vdupq_n_u32(1000))];
    let t899: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(t895[0]), vget_low_u32(vdupq_n_u32(1000)))), vreinterpretq_u32_u64(vmull_high_u32(t895[0], vdupq_n_u32(1000)))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(t895[1]), vget_low_u32(vdupq_n_u32(1000)))), vreinterpretq_u32_u64(vmull_high_u32(t895[1], vdupq_n_u32(1000))))];
    let t900: [uint32x4_t; 2] = [vaddq_u32(t898[0], t897[0]), vaddq_u32(t898[1], t897[1])];
    let t901: [uint32x4_t; 2] = [vcltq_u32(t900[0], t898[0]), vcltq_u32(t900[1], t898[1])];
    let t902: [uint32x4_t; 4] = [t900[0], t900[1], vsubq_u32(vaddq_u32(t899[0], vdupq_n_u32(0)), t901[0]), vsubq_u32(vaddq_u32(t899[1], vdupq_n_u32(0)), t901[1])];
    res = t902;
    return res;
}

#[inline(always)]
unsafe fn b6_i_f(l: &Launch, lid0: u32, x0: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t903: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x0), vld1q_u32(LANE[1].as_ptr()))];
    let t904: [uint32x4_t; 2] = [vshlq_u32(t903[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(t903[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
    let t905 = [vdupq_n_u32(255u32); 2];
    let t906: [uint32x4_t; 2] = [vandq_u32(t903[0], t905[0]), vandq_u32(t903[1], t905[1])];
    let t907: [uint32x4_t; 2] = [vmulq_u32(t904[0], vdupq_n_u32(1000)), vmulq_u32(t904[1], vdupq_n_u32(1000))];
    let t908: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(t904[0]), vget_low_u32(vdupq_n_u32(1000)))), vreinterpretq_u32_u64(vmull_high_u32(t904[0], vdupq_n_u32(1000)))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(t904[1]), vget_low_u32(vdupq_n_u32(1000)))), vreinterpretq_u32_u64(vmull_high_u32(t904[1], vdupq_n_u32(1000))))];
    let t909: [uint32x4_t; 2] = [vaddq_u32(t907[0], t906[0]), vaddq_u32(t907[1], t906[1])];
    let t910: [uint32x4_t; 2] = [vcltq_u32(t909[0], t907[0]), vcltq_u32(t909[1], t907[1])];
    let t911: [uint32x4_t; 4] = [t909[0], t909[1], vsubq_u32(vaddq_u32(t908[0], vdupq_n_u32(0)), t910[0]), vsubq_u32(vaddq_u32(t908[1], vdupq_n_u32(0)), t910[1])];
    res = t911;
    return res;
}

#[inline(always)]
unsafe fn b6_u8_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t912: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32((x0 & 255u32)), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32((x0 & 255u32)), vld1q_u32(LANE[1].as_ptr()))];
    let t913: [uint32x4_t; 2] = [t912[0], t912[1]];
    let t914: u64 = ((((((((x0 >> 8)) as u64)).wrapping_mul(1000u64)) as usize as *const u8)) as usize as u64);
    let t915 = [vdupq_n_u32((t914) as u32); 2];
    let t916 = [vdupq_n_u32(((t914) >> 32) as u32); 2];
    let t917: [uint32x4_t; 2] = [vaddq_u32(t915[0], t913[0]), vaddq_u32(t915[1], t913[1])];
    let t918: [uint32x4_t; 2] = [vcltq_u32(t917[0], t915[0]), vcltq_u32(t917[1], t915[1])];
    let t919: [uint32x4_t; 4] = [t917[0], t917[1], vsubq_u32(vaddq_u32(t916[0], vdupq_n_u32(0)), t918[0]), vsubq_u32(vaddq_u32(t916[1], vdupq_n_u32(0)), t918[1])];
    let t920: [uint32x4_t; 4] = [t919[0], t919[1], t919[2], t919[3]];
    res = t920;
    return res;
}

#[inline(always)]
unsafe fn b6_u8_f(l: &Launch, lid0: u32, x0: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t921: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32((x0 & 255u32)), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32((x0 & 255u32)), vld1q_u32(LANE[1].as_ptr()))];
    let t922: [uint32x4_t; 2] = [t921[0], t921[1]];
    let t923: u64 = ((((((((x0 >> 8)) as u64)).wrapping_mul(1000u64)) as usize as *const u8)) as usize as u64);
    let t924 = [vdupq_n_u32((t923) as u32); 2];
    let t925 = [vdupq_n_u32(((t923) >> 32) as u32); 2];
    let t926: [uint32x4_t; 2] = [vaddq_u32(t924[0], t922[0]), vaddq_u32(t924[1], t922[1])];
    let t927: [uint32x4_t; 2] = [vcltq_u32(t926[0], t924[0]), vcltq_u32(t926[1], t924[1])];
    let t928: [uint32x4_t; 4] = [t926[0], t926[1], vsubq_u32(vaddq_u32(t925[0], vdupq_n_u32(0)), t927[0]), vsubq_u32(vaddq_u32(t925[1], vdupq_n_u32(0)), t927[1])];
    let t929: [uint32x4_t; 4] = [t928[0], t928[1], t928[2], t928[3]];
    res = t929;
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
    let mut t934 = live;
    loop {
        let t936 = bits(&t934);
        if t936.count_ones() <= 2 {
            let mut t937 = t936;
            while t937 != 0 {
                let j = t937.trailing_zeros() as usize;
                t937 &= t937 - 1;
                let t938 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x1);
                let mut x1_j: u32 = t938[j];
                let t939 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x2);
                let mut x2_j: u64 = ((t939[8 + j] as u64) << 32 | t939[j] as u64);
                let mut x3_j: u32 = x3;
                loop {
                    if !(((x3_j) < (((x1_j) & (15u32))))) { break; }
                    x2_j = ((x2_j).wrapping_mul(31u64)).wrapping_add(((x3_j) as u64));
                    x3_j = (x3_j).wrapping_add(2u32);
                }
                let mut t940 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x2);
                t940[j] = x2_j as u32;
                t940[8 + j] = (x2_j as u64 >> 32) as u32;
                x2 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t940);
            }
            break;
        }
        // Lanes part: each its own rounds.
        let t941 = [vdupq_n_u32(15u32); 2];
        let t942: [uint32x4_t; 2] = [vandq_u32(x1[0], t941[0]), vandq_u32(x1[1], t941[1])];
        let t943 = [vdupq_n_u32(x3); 2];
        let t944: [uint32x4_t; 2] = [vcltq_u32(t943[0], t942[0]), vcltq_u32(t943[1], t942[1])];
        t934 = [vandq_u32(t934[0], t944[0]), vandq_u32(t934[1], t944[1])];
        if !(any(&t934)) { break; }
        let t946: u64 = 31u64;
        let t947 = [vdupq_n_u32((t946) as u32); 2];
        let t948: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x2[0]), vget_low_u32(t947[0]))), vreinterpretq_u32_u64(vmull_high_u32(x2[0], t947[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x2[1]), vget_low_u32(t947[1]))), vreinterpretq_u32_u64(vmull_high_u32(x2[1], t947[1])))];
        let t949: [uint32x4_t; 4] = [vmulq_u32(x2[0], t947[0]), vmulq_u32(x2[1], t947[1]), vmlaq_u32(t948[0], t947[0], x2[2]), vmlaq_u32(t948[1], t947[1], x2[3])];
        let t950: u64 = ((x3) as u64);
        let t951 = [vdupq_n_u32((t950) as u32); 2];
        let t952 = [vdupq_n_u32(((t950) >> 32) as u32); 2];
        let t953: [uint32x4_t; 2] = [vaddq_u32(t949[0], t951[0]), vaddq_u32(t949[1], t951[1])];
        let t954: [uint32x4_t; 2] = [vcltq_u32(t953[0], t949[0]), vcltq_u32(t953[1], t949[1])];
        let t955: [uint32x4_t; 4] = [t953[0], t953[1], vsubq_u32(vaddq_u32(t949[2], t952[0]), t954[0]), vsubq_u32(vaddq_u32(t949[3], t952[1]), t954[1])];
        x2 = [vbslq_u32(t934[0], t955[0], x2[0]), vbslq_u32(t934[1], t955[1], x2[1]), vbslq_u32(t934[0], t955[2], x2[2]), vbslq_u32(t934[1], t955[3], x2[3])];
        let t956: [uint32x4_t; 2] = [vandq_u32(t934[0], live[0]), vandq_u32(t934[1], live[1])];
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
    let mut t957: bool = !(arg5).is_null();
    if t957 {
        let mut t958: bool = ((l.group[0]) >= ((arg5.wrapping_add((((x6) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        if !t958 {
            t958 = ((l.group[1]) >= ((arg5.wrapping_add(((((x6).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        let mut t959: bool = t958;
        if !t959 {
            t959 = ((l.group[2]) >= ((arg5.wrapping_add(((((x6).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        t957 = t959;
    }
    if t957 {
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
        let t960: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
        let mut live = t960;
        let gid0 = l.group[0] * l.block[0] + lid0;
        'chunk: {
            b0_r0r1r2r3sg_p(l, lid0, live, arg0, arg1, arg2, arg3, x4);
        }
    }
}
