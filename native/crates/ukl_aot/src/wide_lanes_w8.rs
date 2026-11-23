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
    let mut x15: u32 = 0;
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
        x15 = (((((a1) & (7u64))) as u32)).wrapping_add(1u32);
        let t258 = b5_r0ns_p(l, lid0, live, arg0, gid0.wrapping_mul(x15), x15, x15);
        let t259 = b8_s_p(l, lid0, live, (8u32).wrapping_mul(x15));
        let t260: u64 = t259;
        let t261 = [vdupq_n_u32((t260) as u32); 2];
        let t262 = [vdupq_n_u32(((t260) >> 32) as u32); 2];
        let t263: [uint32x4_t; 4] = [vandq_u32(t258[0], t261[0]), vandq_u32(t258[1], t261[1]), vandq_u32(t258[2], t262[0]), vandq_u32(t258[3], t262[1])];
        let t264 = (arg3 as *const u8).add(((gid0.wrapping_add((27u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t264 as *mut u32).add(0), uint32x4x2_t(t263[0], t263[2]));
            vst2q_u32((t264 as *mut u32).add(8), uint32x4x2_t(t263[1], t263[3]));
        } else {
            let mut t265w = 0u64;
            let t265 = core::hint::black_box(&raw mut t265w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t264 as *mut u64).add(0), t265).write_unaligned(((vgetq_lane_u32::<0>(t263[2]) as u64) << 32 | vgetq_lane_u32::<0>(t263[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t264 as *mut u64).add(1), t265).write_unaligned(((vgetq_lane_u32::<1>(t263[2]) as u64) << 32 | vgetq_lane_u32::<1>(t263[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t264 as *mut u64).add(2), t265).write_unaligned(((vgetq_lane_u32::<2>(t263[2]) as u64) << 32 | vgetq_lane_u32::<2>(t263[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t264 as *mut u64).add(3), t265).write_unaligned(((vgetq_lane_u32::<3>(t263[2]) as u64) << 32 | vgetq_lane_u32::<3>(t263[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t264 as *mut u64).add(4), t265).write_unaligned(((vgetq_lane_u32::<0>(t263[3]) as u64) << 32 | vgetq_lane_u32::<0>(t263[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t264 as *mut u64).add(5), t265).write_unaligned(((vgetq_lane_u32::<1>(t263[3]) as u64) << 32 | vgetq_lane_u32::<1>(t263[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t264 as *mut u64).add(6), t265).write_unaligned(((vgetq_lane_u32::<2>(t263[3]) as u64) << 32 | vgetq_lane_u32::<2>(t263[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t264 as *mut u64).add(7), t265).write_unaligned(((vgetq_lane_u32::<3>(t263[3]) as u64) << 32 | vgetq_lane_u32::<3>(t263[1]) as u64));
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
    let mut x15: u32 = 0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    if ((gid0) as u64 + 7) < ((a0) as u64) {
        let t266 = (arg0 as *const u8).add(((gid0) as usize) * 8);
        let t2680 = vld2q_u32((t266 as *const u32).add(0));
        let t2681 = vld2q_u32((t266 as *const u32).add(8));
        let t268: [uint32x4_t; 4] = [t2680.0, t2681.0, t2680.1, t2681.1];
        x6 = t268;
        let t269 = (arg1 as *const u8).add(((gid0) as usize) * 8);
        let t2710 = vld2q_u32((t269 as *const u32).add(0));
        let t2711 = vld2q_u32((t269 as *const u32).add(8));
        let t271: [uint32x4_t; 4] = [t2710.0, t2711.0, t2710.1, t2711.1];
        x7 = t271;
        let t272 = (arg2 as *const u32).add((gid0) as usize);
        let t273: [uint32x4_t; 2] = [vld1q_u32(t272.add(0)), vld1q_u32(t272.add(4))];
        x8 = t273;
        let t274 = [vdupq_n_u32(63u32); 2];
        let t275: [uint32x4_t; 2] = [vandq_u32(x8[0], t274[0]), vandq_u32(x8[1], t274[1])];
        x9 = t275;
        x10 = a0;
        let t276: [uint32x4_t; 2] = [vaddq_u32(x6[0], x7[0]), vaddq_u32(x6[1], x7[1])];
        let t277: [uint32x4_t; 2] = [vcltq_u32(t276[0], x6[0]), vcltq_u32(t276[1], x6[1])];
        let t278: [uint32x4_t; 4] = [t276[0], t276[1], vsubq_u32(vaddq_u32(x6[2], x7[2]), t277[0]), vsubq_u32(vaddq_u32(x6[3], x7[3]), t277[1])];
        let t279 = (arg3 as *const u8).add(((gid0.wrapping_add((0u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t279 as *mut u32).add(0), uint32x4x2_t(t278[0], t278[2]));
        vst2q_u32((t279 as *mut u32).add(8), uint32x4x2_t(t278[1], t278[3]));
        let t280: [uint32x4_t; 2] = [vcltq_u32(x6[0], x7[0]), vcltq_u32(x6[1], x7[1])];
        let t281: [uint32x4_t; 4] = [vsubq_u32(x6[0], x7[0]), vsubq_u32(x6[1], x7[1]), vaddq_u32(vsubq_u32(x6[2], x7[2]), t280[0]), vaddq_u32(vsubq_u32(x6[3], x7[3]), t280[1])];
        let t282 = (arg3 as *const u8).add(((gid0.wrapping_add((1u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t282 as *mut u32).add(0), uint32x4x2_t(t281[0], t281[2]));
        vst2q_u32((t282 as *mut u32).add(8), uint32x4x2_t(t281[1], t281[3]));
        let t283: [uint32x4_t; 2] = [x9[0], x9[1]];
        let t284: [uint32x4_t; 2] = [vandq_u32(t283[0], vdupq_n_u32(63)), vandq_u32(t283[1], vdupq_n_u32(63))];
        let t285: [uint32x4_t; 4] = [vshlq_u32(x6[0], vreinterpretq_s32_u32(t284[0])), vshlq_u32(x6[1], vreinterpretq_s32_u32(t284[1])), vorrq_u32(vshlq_u32(x6[2], vreinterpretq_s32_u32(t284[0])), vshlq_u32(x6[0], vsubq_s32(vreinterpretq_s32_u32(t284[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(x6[3], vreinterpretq_s32_u32(t284[1])), vshlq_u32(x6[1], vsubq_s32(vreinterpretq_s32_u32(t284[1]), vdupq_n_s32(32))))];
        let t286 = (arg3 as *const u8).add(((gid0.wrapping_add((2u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t286 as *mut u32).add(0), uint32x4x2_t(t285[0], t285[2]));
        vst2q_u32((t286 as *mut u32).add(8), uint32x4x2_t(t285[1], t285[3]));
        let t287: [uint32x4_t; 2] = [x9[0], x9[1]];
        let t288: [uint32x4_t; 2] = [vandq_u32(t287[0], vdupq_n_u32(63)), vandq_u32(t287[1], vdupq_n_u32(63))];
        let t289: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(x6[0], vnegq_s32(vreinterpretq_s32_u32(t288[0]))), vshlq_u32(x6[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t288[0])))), vorrq_u32(vshlq_u32(x6[1], vnegq_s32(vreinterpretq_s32_u32(t288[1]))), vshlq_u32(x6[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t288[1])))), vshlq_u32(x6[2], vnegq_s32(vreinterpretq_s32_u32(t288[0]))), vshlq_u32(x6[3], vnegq_s32(vreinterpretq_s32_u32(t288[1])))];
        let t290 = (arg3 as *const u8).add(((gid0.wrapping_add((3u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t290 as *mut u32).add(0), uint32x4x2_t(t289[0], t289[2]));
        vst2q_u32((t290 as *mut u32).add(8), uint32x4x2_t(t289[1], t289[3]));
        let t291: [uint32x4_t; 2] = [x9[0], x9[1]];
        let t292: [uint32x4_t; 2] = [vandq_u32(t291[0], vdupq_n_u32(63)), vandq_u32(t291[1], vdupq_n_u32(63))];
        let t293: [uint32x4_t; 2] = [vcgeq_s32(vreinterpretq_s32_u32(t292[0]), vdupq_n_s32(32)), vcgeq_s32(vreinterpretq_s32_u32(t292[1]), vdupq_n_s32(32))];
        let t294: [uint32x4_t; 4] = [vbslq_u32(t293[0], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t292[0])))), vorrq_u32(vshlq_u32(x6[0], vnegq_s32(vreinterpretq_s32_u32(t292[0]))), vshlq_u32(x6[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t292[0]))))), vbslq_u32(t293[1], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t292[1])))), vorrq_u32(vshlq_u32(x6[1], vnegq_s32(vreinterpretq_s32_u32(t292[1]))), vshlq_u32(x6[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t292[1]))))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vnegq_s32(vreinterpretq_s32_u32(t292[0])))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vnegq_s32(vreinterpretq_s32_u32(t292[1]))))];
        let t295 = (arg3 as *const u8).add(((gid0.wrapping_add((4u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t295 as *mut u32).add(0), uint32x4x2_t(t294[0], t294[2]));
        vst2q_u32((t295 as *mut u32).add(8), uint32x4x2_t(t294[1], t294[3]));
        let t296: [uint32x4_t; 4] = [vshlq_n_u32::<13>(x6[0]), vshlq_n_u32::<13>(x6[1]), vorrq_u32(vshlq_n_u32::<13>(x6[2]), vshrq_n_u32::<19>(x6[0])), vorrq_u32(vshlq_n_u32::<13>(x6[3]), vshrq_n_u32::<19>(x6[1]))];
        let t297: [uint32x4_t; 2] = [vshrq_n_u32::<19>(x7[2]), vshrq_n_u32::<19>(x7[3])];
        let t298: [uint32x4_t; 4] = [vorrq_u32(t296[0], t297[0]), vorrq_u32(t296[1], t297[1]), t296[2], t296[3]];
        let t299 = (arg3 as *const u8).add(((gid0.wrapping_add((5u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t299 as *mut u32).add(0), uint32x4x2_t(t298[0], t298[2]));
        vst2q_u32((t299 as *mut u32).add(8), uint32x4x2_t(t298[1], t298[3]));
        let t300: [uint32x4_t; 4] = [vdupq_n_u32(0), vdupq_n_u32(0), vshlq_n_u32::<8>(x6[0]), vshlq_n_u32::<8>(x6[1])];
        let t301 = (arg3 as *const u8).add(((gid0.wrapping_add((6u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t301 as *mut u32).add(0), uint32x4x2_t(t300[0], t300[2]));
        vst2q_u32((t301 as *mut u32).add(8), uint32x4x2_t(t300[1], t300[3]));
        let t302: [uint32x4_t; 2] = [vshrq_n_u32::<3>(x6[2]), vshrq_n_u32::<3>(x6[3])];
        let t303: [uint32x4_t; 4] = [t302[0], t302[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        let t304 = (arg3 as *const u8).add(((gid0.wrapping_add((7u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t304 as *mut u32).add(0), uint32x4x2_t(t303[0], t303[2]));
        vst2q_u32((t304 as *mut u32).add(8), uint32x4x2_t(t303[1], t303[3]));
        let t305 = vdupq_n_s32(((((40i64) as u32)) & 63) as i32);
        let t306: [uint32x4_t; 2] = [vcgeq_s32(t305, vdupq_n_s32(32)), vcgeq_s32(t305, vdupq_n_s32(32))];
        let t307: [uint32x4_t; 4] = [vbslq_u32(t306[0], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vsubq_s32(vdupq_n_s32(32), t305))), vorrq_u32(vshlq_u32(x6[0], vnegq_s32(t305)), vshlq_u32(x6[2], vsubq_s32(vdupq_n_s32(32), t305)))), vbslq_u32(t306[1], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vsubq_s32(vdupq_n_s32(32), t305))), vorrq_u32(vshlq_u32(x6[1], vnegq_s32(t305)), vshlq_u32(x6[3], vsubq_s32(vdupq_n_s32(32), t305)))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vnegq_s32(t305))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vnegq_s32(t305)))];
        let t308 = (arg3 as *const u8).add(((gid0.wrapping_add((8u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t308 as *mut u32).add(0), uint32x4x2_t(t307[0], t307[2]));
        vst2q_u32((t308 as *mut u32).add(8), uint32x4x2_t(t307[1], t307[3]));
        let t309: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x6[0]), vget_low_u32(x7[0]))), vreinterpretq_u32_u64(vmull_high_u32(x6[0], x7[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x6[1]), vget_low_u32(x7[1]))), vreinterpretq_u32_u64(vmull_high_u32(x6[1], x7[1])))];
        let t310: [uint32x4_t; 4] = [vmulq_u32(x6[0], x7[0]), vmulq_u32(x6[1], x7[1]), vmlaq_u32(vmlaq_u32(t309[0], x6[0], x7[2]), x7[0], x6[2]), vmlaq_u32(vmlaq_u32(t309[1], x6[1], x7[3]), x7[1], x6[3])];
        let t311 = (arg3 as *const u8).add(((gid0.wrapping_add((9u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t311 as *mut u32).add(0), uint32x4x2_t(t310[0], t310[2]));
        vst2q_u32((t311 as *mut u32).add(8), uint32x4x2_t(t310[1], t310[3]));
        let t312: u64 = 0u64;
        let t313 = [vdupq_n_u32((t312) as u32); 2];
        let t314: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x7[0], t313[0]), vceqq_u32(x7[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(x7[1], t313[1]), vceqq_u32(x7[3], vdupq_n_u32(0)))];
        let t315: [uint32x4_t; 2] = [vmvnq_u32(t314[0]), vmvnq_u32(t314[1])];
        let mut t316: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t317: [uint32x4_t; 2] = [vmvnq_u32(t315[0]), vmvnq_u32(t315[1])];
        {
            let (mut t318xl, mut t318xh, mut t318yl, mut t318yh) = ([0u32; 8], [0u32; 8], [0u32; 8], [0u32; 8]);
            vst1q_u32(t318xl.as_mut_ptr().add(0), x6[0]); vst1q_u32(t318xh.as_mut_ptr().add(0), x6[2]); vst1q_u32(t318yl.as_mut_ptr().add(0), x7[0]); vst1q_u32(t318yh.as_mut_ptr().add(0), x7[2]);
            vst1q_u32(t318xl.as_mut_ptr().add(4), x6[1]); vst1q_u32(t318xh.as_mut_ptr().add(4), x6[3]); vst1q_u32(t318yl.as_mut_ptr().add(4), x7[1]); vst1q_u32(t318yh.as_mut_ptr().add(4), x7[3]);
            for j in 0..8 { let q = (((t318xh[j] as u64) << 32 | t318xl[j] as u64) as u64).checked_div(((t318yh[j] as u64) << 32 | t318yl[j] as u64) as u64).unwrap_or(0) as u64; t318xl[j] = q as u32; t318xh[j] = (q >> 32) as u32; }
            let t319: [uint32x4_t; 4] = [vld1q_u32(t318xl.as_ptr().add(0)), vld1q_u32(t318xl.as_ptr().add(4)), vld1q_u32(t318xh.as_ptr().add(0)), vld1q_u32(t318xh.as_ptr().add(4))];
            t316 = [vbslq_u32(t315[0], t319[0], t316[0]), vbslq_u32(t315[1], t319[1], t316[1]), vbslq_u32(t315[0], t319[2], t316[2]), vbslq_u32(t315[1], t319[3], t316[3])];
        }
        {
            let t320: u64 = 7u64;
            let t321 = [vdupq_n_u32((t320) as u32); 2];
            let t322: [uint32x4_t; 4] = [t321[0], t321[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            t316 = [vbslq_u32(t317[0], t322[0], t316[0]), vbslq_u32(t317[1], t322[1], t316[1]), vbslq_u32(t317[0], t322[2], t316[2]), vbslq_u32(t317[1], t322[3], t316[3])];
        }
        let t323 = (arg3 as *const u8).add(((gid0.wrapping_add((10u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t323 as *mut u32).add(0), uint32x4x2_t(t316[0], t316[2]));
        vst2q_u32((t323 as *mut u32).add(8), uint32x4x2_t(t316[1], t316[3]));
        let t324: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x6[2], x7[2]), vandq_u32(vceqq_u32(x6[2], x7[2]), vcltq_u32(x6[0], x7[0]))), vorrq_u32(vcltq_u32(x6[3], x7[3]), vandq_u32(vceqq_u32(x6[3], x7[3]), vcltq_u32(x6[1], x7[1])))];
        let t325: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t324[0]), vshrq_n_u32::<31>(t324[1])];
        let t326: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x7[2], x6[2]), vandq_u32(vceqq_u32(x7[2], x6[2]), vcltq_u32(x7[0], x6[0]))), vorrq_u32(vcltq_u32(x7[3], x6[3]), vandq_u32(vceqq_u32(x7[3], x6[3]), vcltq_u32(x7[1], x6[1])))];
        let t327: [uint32x4_t; 2] = [vmvnq_u32(t326[0]), vmvnq_u32(t326[1])];
        let t328: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t327[0]), vshrq_n_u32::<31>(t327[1])];
        let t329: [uint32x4_t; 4] = [vshlq_n_u32::<1>(t328[0]), vshlq_n_u32::<1>(t328[1]), vshrq_n_u32::<31>(t328[0]), vshrq_n_u32::<31>(t328[1])];
        let t330: [uint32x4_t; 4] = [vorrq_u32(t325[0], t329[0]), vorrq_u32(t325[1], t329[1]), t329[2], t329[3]];
        let t331: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x6[0], x7[0]), vceqq_u32(x6[2], x7[2])), vandq_u32(vceqq_u32(x6[1], x7[1]), vceqq_u32(x6[3], x7[3]))];
        let t332: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t331[0]), vshrq_n_u32::<31>(t331[1])];
        let t333: [uint32x4_t; 4] = [vshlq_n_u32::<2>(t332[0]), vshlq_n_u32::<2>(t332[1]), vshrq_n_u32::<30>(t332[0]), vshrq_n_u32::<30>(t332[1])];
        let t334: [uint32x4_t; 4] = [vorrq_u32(t330[0], t333[0]), vorrq_u32(t330[1], t333[1]), vorrq_u32(t330[2], t333[2]), vorrq_u32(t330[3], t333[3])];
        let t335: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x6[0], x7[0]), vceqq_u32(x6[2], x7[2])), vandq_u32(vceqq_u32(x6[1], x7[1]), vceqq_u32(x6[3], x7[3]))];
        let t336: [uint32x4_t; 2] = [vmvnq_u32(t335[0]), vmvnq_u32(t335[1])];
        let t337: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t336[0]), vshrq_n_u32::<31>(t336[1])];
        let t338: [uint32x4_t; 4] = [vshlq_n_u32::<3>(t337[0]), vshlq_n_u32::<3>(t337[1]), vshrq_n_u32::<29>(t337[0]), vshrq_n_u32::<29>(t337[1])];
        let t339: [uint32x4_t; 4] = [vorrq_u32(t334[0], t338[0]), vorrq_u32(t334[1], t338[1]), vorrq_u32(t334[2], t338[2]), vorrq_u32(t334[3], t338[3])];
        let t340: [uint32x4_t; 2] = [vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x6[2]), vreinterpretq_s32_u32(x7[2])), vandq_u32(vceqq_u32(x6[2], x7[2]), vcltq_u32(x6[0], x7[0]))), vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x6[3]), vreinterpretq_s32_u32(x7[3])), vandq_u32(vceqq_u32(x6[3], x7[3]), vcltq_u32(x6[1], x7[1])))];
        let t341: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t340[0]), vshrq_n_u32::<31>(t340[1])];
        let t342: [uint32x4_t; 4] = [vshlq_n_u32::<4>(t341[0]), vshlq_n_u32::<4>(t341[1]), vshrq_n_u32::<28>(t341[0]), vshrq_n_u32::<28>(t341[1])];
        let t343: [uint32x4_t; 4] = [vorrq_u32(t339[0], t342[0]), vorrq_u32(t339[1], t342[1]), vorrq_u32(t339[2], t342[2]), vorrq_u32(t339[3], t342[3])];
        let t344: [uint32x4_t; 2] = [vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x6[2]), vreinterpretq_s32_u32(x7[2])), vandq_u32(vceqq_u32(x6[2], x7[2]), vcltq_u32(x6[0], x7[0]))), vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x6[3]), vreinterpretq_s32_u32(x7[3])), vandq_u32(vceqq_u32(x6[3], x7[3]), vcltq_u32(x6[1], x7[1])))];
        let t345: [uint32x4_t; 2] = [vmvnq_u32(t344[0]), vmvnq_u32(t344[1])];
        let t346: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t345[0]), vshrq_n_u32::<31>(t345[1])];
        let t347: [uint32x4_t; 4] = [vshlq_n_u32::<5>(t346[0]), vshlq_n_u32::<5>(t346[1]), vshrq_n_u32::<27>(t346[0]), vshrq_n_u32::<27>(t346[1])];
        let t348: [uint32x4_t; 4] = [vorrq_u32(t343[0], t347[0]), vorrq_u32(t343[1], t347[1]), vorrq_u32(t343[2], t347[2]), vorrq_u32(t343[3], t347[3])];
        let t349: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x7[2], x6[2]), vandq_u32(vceqq_u32(x7[2], x6[2]), vcltq_u32(x7[0], x6[0]))), vorrq_u32(vcltq_u32(x7[3], x6[3]), vandq_u32(vceqq_u32(x7[3], x6[3]), vcltq_u32(x7[1], x6[1])))];
        let t350: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t349[0]), vshrq_n_u32::<31>(t349[1])];
        let t351: [uint32x4_t; 4] = [vshlq_n_u32::<6>(t350[0]), vshlq_n_u32::<6>(t350[1]), vshrq_n_u32::<26>(t350[0]), vshrq_n_u32::<26>(t350[1])];
        let t352: [uint32x4_t; 4] = [vorrq_u32(t348[0], t351[0]), vorrq_u32(t348[1], t351[1]), vorrq_u32(t348[2], t351[2]), vorrq_u32(t348[3], t351[3])];
        let t353 = (arg3 as *const u8).add(((gid0.wrapping_add((11u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t353 as *mut u32).add(0), uint32x4x2_t(t352[0], t352[2]));
        vst2q_u32((t353 as *mut u32).add(8), uint32x4x2_t(t352[1], t352[3]));
        let t354: [uint32x4_t; 2] = [vaddq_u32(x8[0], x6[0]), vaddq_u32(x8[1], x6[1])];
        let t355: [uint32x4_t; 2] = [vcltq_u32(t354[0], x8[0]), vcltq_u32(t354[1], x8[1])];
        let t356: [uint32x4_t; 4] = [t354[0], t354[1], vsubq_u32(vaddq_u32(vdupq_n_u32(0), x6[2]), t355[0]), vsubq_u32(vaddq_u32(vdupq_n_u32(0), x6[3]), t355[1])];
        let t357 = (arg3 as *const u8).add(((gid0.wrapping_add((12u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t357 as *mut u32).add(0), uint32x4x2_t(t356[0], t356[2]));
        vst2q_u32((t357 as *mut u32).add(8), uint32x4x2_t(t356[1], t356[3]));
        let t358: [uint32x4_t; 2] = [vreinterpretq_u32_s32(vshrq_n_s32::<31>(vreinterpretq_s32_u32(x8[0]))), vreinterpretq_u32_s32(vshrq_n_s32::<31>(vreinterpretq_s32_u32(x8[1])))];
        let t359: [uint32x4_t; 4] = [x8[0], x8[1], t358[0], t358[1]];
        let t360 = (arg3 as *const u8).add(((gid0.wrapping_add((13u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t360 as *mut u32).add(0), uint32x4x2_t(t359[0], t359[2]));
        vst2q_u32((t360 as *mut u32).add(8), uint32x4x2_t(t359[1], t359[3]));
        let t361: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<3>(x6[0]), vshlq_n_u32::<29>(x6[2])), vorrq_u32(vshrq_n_u32::<3>(x6[1]), vshlq_n_u32::<29>(x6[3])), vshrq_n_u32::<3>(x6[2]), vshrq_n_u32::<3>(x6[3])];
        let t362: [uint32x4_t; 2] = [t361[0], t361[1]];
        let t363: [uint32x4_t; 4] = [t362[0], t362[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        let t364 = (arg3 as *const u8).add(((gid0.wrapping_add((14u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t364 as *mut u32).add(0), uint32x4x2_t(t363[0], t363[2]));
        vst2q_u32((t364 as *mut u32).add(8), uint32x4x2_t(t363[1], t363[3]));
        let t365: u64 = 1u64;
        let t366 = [vdupq_n_u32((t365) as u32); 2];
        let t367: [uint32x4_t; 2] = [vandq_u32(x6[0], t366[0]), vandq_u32(x6[1], t366[1])];
        let t368: [uint32x4_t; 2] = [vtstq_u32(t367[0], t367[0]), vtstq_u32(t367[1], t367[1])];
        {
            let t369: [uint32x4_t; 4] = [veorq_u32(x6[0], x7[0]), veorq_u32(x6[1], x7[1]), veorq_u32(x6[2], x7[2]), veorq_u32(x6[3], x7[3])];
            x11 = [vbslq_u32(t368[0], t369[0], x11[0]), vbslq_u32(t368[1], t369[1], x11[1]), vbslq_u32(t368[0], t369[2], x11[2]), vbslq_u32(t368[1], t369[3], x11[3])];
        }
        let t370: [uint32x4_t; 2] = [vmvnq_u32(t368[0]), vmvnq_u32(t368[1])];
        {
            let t371: [uint32x4_t; 2] = [vaddq_u32(x6[0], x8[0]), vaddq_u32(x6[1], x8[1])];
            let t372: [uint32x4_t; 2] = [vcltq_u32(t371[0], x6[0]), vcltq_u32(t371[1], x6[1])];
            let t373: [uint32x4_t; 4] = [t371[0], t371[1], vsubq_u32(vaddq_u32(x6[2], vdupq_n_u32(0)), t372[0]), vsubq_u32(vaddq_u32(x6[3], vdupq_n_u32(0)), t372[1])];
            x11 = [vbslq_u32(t370[0], t373[0], x11[0]), vbslq_u32(t370[1], t373[1], x11[1]), vbslq_u32(t370[0], t373[2], x11[2]), vbslq_u32(t370[1], t373[3], x11[3])];
        }
        let t374 = (arg3 as *const u8).add(((gid0.wrapping_add((15u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t374 as *mut u32).add(0), uint32x4x2_t(x11[0], x11[2]));
        vst2q_u32((t374 as *mut u32).add(8), uint32x4x2_t(x11[1], x11[3]));
        let t375 = b1_l_f(l, lid0, x6);
        let t376 = b2_l_f(l, lid0, x7);
        let t377: [uint32x4_t; 4] = [vshlq_n_u32::<8>(t376[0]), vshlq_n_u32::<8>(t376[1]), vshrq_n_u32::<24>(t376[0]), vshrq_n_u32::<24>(t376[1])];
        let t378: [uint32x4_t; 2] = [vaddq_u32(t375[0], t377[0]), vaddq_u32(t375[1], t377[1])];
        let t379: [uint32x4_t; 2] = [vcltq_u32(t378[0], t375[0]), vcltq_u32(t378[1], t375[1])];
        let t380: [uint32x4_t; 4] = [t378[0], t378[1], vsubq_u32(vaddq_u32(vdupq_n_u32(0), t377[2]), t379[0]), vsubq_u32(vaddq_u32(vdupq_n_u32(0), t377[3]), t379[1])];
        let t381 = (arg3 as *const u8).add(((gid0.wrapping_add((16u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t381 as *mut u32).add(0), uint32x4x2_t(t380[0], t380[2]));
        vst2q_u32((t381 as *mut u32).add(8), uint32x4x2_t(t380[1], t380[3]));
        let t382 = [vdupq_n_u32(x10); 2];
        let mut t383x = [0u32; 8];
        let mut t383y = [0u32; 8];
        for r in 0..2 { vst1q_u32(t383x.as_mut_ptr().add(4 * r), x8[r]); vst1q_u32(t383y.as_mut_ptr().add(4 * r), t382[r]); }
        for j in 0..8 { t383x[j] = (t383x[j] as u32).checked_rem(t383y[j] as u32).unwrap_or(0) as u32; }
        let t384: [uint32x4_t; 2] = [vld1q_u32(t383x.as_ptr().add(0)), vld1q_u32(t383x.as_ptr().add(4))];
        let mut t385: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t384[0]) as usize * 8) as *const u64).read_unaligned(); t385[0] = vsetq_lane_u32::<0>(v as u32, t385[0]); t385[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t385[2]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t384[0]) as usize * 8) as *const u64).read_unaligned(); t385[0] = vsetq_lane_u32::<1>(v as u32, t385[0]); t385[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t385[2]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t384[0]) as usize * 8) as *const u64).read_unaligned(); t385[0] = vsetq_lane_u32::<2>(v as u32, t385[0]); t385[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t385[2]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t384[0]) as usize * 8) as *const u64).read_unaligned(); t385[0] = vsetq_lane_u32::<3>(v as u32, t385[0]); t385[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t385[2]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t384[1]) as usize * 8) as *const u64).read_unaligned(); t385[1] = vsetq_lane_u32::<0>(v as u32, t385[1]); t385[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t385[3]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t384[1]) as usize * 8) as *const u64).read_unaligned(); t385[1] = vsetq_lane_u32::<1>(v as u32, t385[1]); t385[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t385[3]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t384[1]) as usize * 8) as *const u64).read_unaligned(); t385[1] = vsetq_lane_u32::<2>(v as u32, t385[1]); t385[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t385[3]); }
        { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t384[1]) as usize * 8) as *const u64).read_unaligned(); t385[1] = vsetq_lane_u32::<3>(v as u32, t385[1]); t385[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t385[3]); }
        let t386 = (arg3 as *const u8).add(((gid0.wrapping_add((17u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t386 as *mut u32).add(0), uint32x4x2_t(t385[0], t385[2]));
        vst2q_u32((t386 as *mut u32).add(8), uint32x4x2_t(t385[1], t385[3]));
        let t387 = b3_ll_f(l, lid0, x6, x8);
        let t388 = (arg3 as *const u8).add(((gid0.wrapping_add((18u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t388 as *mut u32).add(0), uint32x4x2_t(t387[0], t387[2]));
        vst2q_u32((t388 as *mut u32).add(8), uint32x4x2_t(t387[1], t387[3]));
        x12 = x6;
        x13 = 0u32;
        let mut t397 = live;
        loop {
            let t399 = bits(&t397);
            if t399.count_ones() <= 2 {
                let mut t400 = t399;
                while t400 != 0 {
                    let j = t400.trailing_zeros() as usize;
                    t400 &= t400 - 1;
                    let t401 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x7);
                    let mut x7_j: u64 = ((t401[8 + j] as u64) << 32 | t401[j] as u64);
                    let t402 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x8);
                    let mut x8_j: u32 = t402[j];
                    let t403 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x12);
                    let mut x12_j: u64 = ((t403[8 + j] as u64) << 32 | t403[j] as u64);
                    let mut x13_j: u32 = x13;
                    loop {
                        if !(((x13_j) < (((x8_j) & (7u32))))) { break; }
                        x12_j = ((x12_j).wrapping_mul(3u64)).wrapping_add(x7_j);
                        let t404 = x13_j;
                        x13_j = (x13_j).wrapping_add(1u32);
                    }
                    let mut t405 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x12);
                    t405[j] = x12_j as u32;
                    t405[8 + j] = (x12_j as u64 >> 32) as u32;
                    x12 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t405);
                }
                break;
            }
            // Lanes part: each its own rounds.
            let t406 = [vdupq_n_u32(7u32); 2];
            let t407: [uint32x4_t; 2] = [vandq_u32(x8[0], t406[0]), vandq_u32(x8[1], t406[1])];
            let t408 = [vdupq_n_u32(x13); 2];
            let t409: [uint32x4_t; 2] = [vcltq_u32(t408[0], t407[0]), vcltq_u32(t408[1], t407[1])];
            t397 = [vandq_u32(t397[0], t409[0]), vandq_u32(t397[1], t409[1])];
            if !(any(&t397)) { break; }
            let t411: u64 = 3u64;
            let t412 = [vdupq_n_u32((t411) as u32); 2];
            let t413: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x12[0]), vget_low_u32(t412[0]))), vreinterpretq_u32_u64(vmull_high_u32(x12[0], t412[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x12[1]), vget_low_u32(t412[1]))), vreinterpretq_u32_u64(vmull_high_u32(x12[1], t412[1])))];
            let t414: [uint32x4_t; 4] = [vmulq_u32(x12[0], t412[0]), vmulq_u32(x12[1], t412[1]), vmlaq_u32(t413[0], t412[0], x12[2]), vmlaq_u32(t413[1], t412[1], x12[3])];
            let t415: [uint32x4_t; 2] = [vaddq_u32(t414[0], x7[0]), vaddq_u32(t414[1], x7[1])];
            let t416: [uint32x4_t; 2] = [vcltq_u32(t415[0], t414[0]), vcltq_u32(t415[1], t414[1])];
            let t417: [uint32x4_t; 4] = [t415[0], t415[1], vsubq_u32(vaddq_u32(t414[2], x7[2]), t416[0]), vsubq_u32(vaddq_u32(t414[3], x7[3]), t416[1])];
            x12 = [vbslq_u32(t397[0], t417[0], x12[0]), vbslq_u32(t397[1], t417[1], x12[1]), vbslq_u32(t397[0], t417[2], x12[2]), vbslq_u32(t397[1], t417[3], x12[3])];
            let t418: [uint32x4_t; 2] = [vandq_u32(t397[0], live[0]), vandq_u32(t397[1], live[1])];
            let t419 = x13;
            x13 = (x13).wrapping_add(1u32);
        }
        let t420 = (arg3 as *const u8).add(((gid0.wrapping_add((19u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t420 as *mut u32).add(0), uint32x4x2_t(x12[0], x12[2]));
        vst2q_u32((t420 as *mut u32).add(8), uint32x4x2_t(x12[1], x12[3]));
        let t421: u64 = a1;
        let t422 = [vdupq_n_u32((t421) as u32); 2];
        let t423 = [vdupq_n_u32(((t421) >> 32) as u32); 2];
        let t424: [uint32x4_t; 2] = [vaddq_u32(x6[0], t422[0]), vaddq_u32(x6[1], t422[1])];
        let t425: [uint32x4_t; 2] = [vcltq_u32(t424[0], x6[0]), vcltq_u32(t424[1], x6[1])];
        let t426: [uint32x4_t; 4] = [t424[0], t424[1], vsubq_u32(vaddq_u32(x6[2], t423[0]), t425[0]), vsubq_u32(vaddq_u32(x6[3], t423[1]), t425[1])];
        let t427: u64 = (a1).wrapping_shl((3u64) as u32);
        let t428 = [vdupq_n_u32((t427) as u32); 2];
        let t429 = [vdupq_n_u32(((t427) >> 32) as u32); 2];
        let t430: [uint32x4_t; 2] = [vaddq_u32(t426[0], t428[0]), vaddq_u32(t426[1], t428[1])];
        let t431: [uint32x4_t; 2] = [vcltq_u32(t430[0], t426[0]), vcltq_u32(t430[1], t426[1])];
        let t432: [uint32x4_t; 4] = [t430[0], t430[1], vsubq_u32(vaddq_u32(t426[2], t429[0]), t431[0]), vsubq_u32(vaddq_u32(t426[3], t429[1]), t431[1])];
        let t433 = (arg3 as *const u8).add(((gid0.wrapping_add((20u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t433 as *mut u32).add(0), uint32x4x2_t(t432[0], t432[2]));
        vst2q_u32((t433 as *mut u32).add(8), uint32x4x2_t(t432[1], t432[3]));
        let t434: u64 = 4294967295u64;
        let t435 = [vdupq_n_u32((t434) as u32); 2];
        let t436: [uint32x4_t; 2] = [vandq_u32(x6[0], t435[0]), vandq_u32(x6[1], t435[1])];
        let t437: u64 = 4294967295u64;
        let t438 = [vdupq_n_u32((t437) as u32); 2];
        let t439: [uint32x4_t; 2] = [vandq_u32(x7[0], t438[0]), vandq_u32(x7[1], t438[1])];
        let t440: [uint32x4_t; 2] = [vceqq_u32(t436[0], t439[0]), vceqq_u32(t436[1], t439[1])];
        let mut t441: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t442: [uint32x4_t; 2] = [vmvnq_u32(t440[0]), vmvnq_u32(t440[1])];
        {
            let t443 = [vdupq_n_u32(1u32); 2];
            t441 = [vbslq_u32(t440[0], t443[0], t441[0]), vbslq_u32(t440[1], t443[1], t441[1])];
        }
        {
            let t444 = [vdupq_n_u32(2u32); 2];
            t441 = [vbslq_u32(t442[0], t444[0], t441[0]), vbslq_u32(t442[1], t444[1], t441[1])];
        }
        let t445: [uint32x4_t; 4] = [t441[0], t441[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        let t446 = (arg3 as *const u8).add(((gid0.wrapping_add((21u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t446 as *mut u32).add(0), uint32x4x2_t(t445[0], t445[2]));
        vst2q_u32((t446 as *mut u32).add(8), uint32x4x2_t(t445[1], t445[3]));
        let t447: [uint32x4_t; 2] = [vtstq_u32(vorrq_u32(x6[0], x6[2]), vorrq_u32(x6[0], x6[2])), vtstq_u32(vorrq_u32(x6[1], x6[3]), vorrq_u32(x6[1], x6[3]))];
        let mut t448: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t449: [uint32x4_t; 2] = [vmvnq_u32(t447[0]), vmvnq_u32(t447[1])];
        {
            let t450 = [vdupq_n_u32(5u32); 2];
            t448 = [vbslq_u32(t447[0], t450[0], t448[0]), vbslq_u32(t447[1], t450[1], t448[1])];
        }
        {
            let t451 = [vdupq_n_u32(6u32); 2];
            t448 = [vbslq_u32(t449[0], t451[0], t448[0]), vbslq_u32(t449[1], t451[1], t448[1])];
        }
        let t452: [uint32x4_t; 4] = [t448[0], t448[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        let t453 = (arg3 as *const u8).add(((gid0.wrapping_add((22u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t453 as *mut u32).add(0), uint32x4x2_t(t452[0], t452[2]));
        vst2q_u32((t453 as *mut u32).add(8), uint32x4x2_t(t452[1], t452[3]));
        let t454 = b4_ll_f(l, lid0, x6, x7);
        let t455 = (arg3 as *const u8).add(((gid0.wrapping_add((23u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t455 as *mut u32).add(0), uint32x4x2_t(t454[0], t454[2]));
        vst2q_u32((t455 as *mut u32).add(8), uint32x4x2_t(t454[1], t454[3]));
        let t456: u32;
        if ((((a1) & (1u64))) != (0u64)) {
            t456 = 8u32;
        } else {
            t456 = 4u32;
        }
        x14 = t456;
        let t457 = b5_r0ns_f_v0(l, lid0, arg0, gid0.wrapping_mul(x14), x14, 8u32);
        let t458 = (arg3 as *const u8).add(((gid0.wrapping_add((24u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t458 as *mut u32).add(0), uint32x4x2_t(t457[0], t457[2]));
        vst2q_u32((t458 as *mut u32).add(8), uint32x4x2_t(t457[1], t457[3]));
        let t459 = if (gid0.wrapping_add(3u32) & 255u32) <= 248u32 { b6_u8_f(l, lid0, gid0.wrapping_add(3u32)) } else { b6_i_f(l, lid0, gid0.wrapping_add(3u32)) };
        let t460 = (arg3 as *const u8).add(((gid0.wrapping_add((25u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t460 as *mut u32).add(0), uint32x4x2_t(t459[0], t459[2]));
        vst2q_u32((t460 as *mut u32).add(8), uint32x4x2_t(t459[1], t459[3]));
        let t461: u64 = 3u64;
        let t462 = [vdupq_n_u32((t461) as u32); 2];
        let t463: [uint32x4_t; 2] = [vandq_u32(x6[0], t462[0]), vandq_u32(x6[1], t462[1])];
        let t464: u64 = 0u64;
        let t465 = [vdupq_n_u32((t464) as u32); 2];
        let t466: [uint32x4_t; 2] = [vceqq_u32(t463[0], t465[0]), vceqq_u32(t463[1], t465[1])];
        let mut t467: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t468: [uint32x4_t; 2] = [vmvnq_u32(t466[0]), vmvnq_u32(t466[1])];
        if any(&t466) {
            let t469: [uint32x4_t; 4];
            let t470 = bits(&t466);
            if t470.count_ones() <= 2 {
                let t471 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x7);
                let t472 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x8);
                let mut t473 = [0u32; 16];
                let mut t474 = t470;
                while t474 != 0 {
                    let j = t474.trailing_zeros() as usize;
                    t474 &= t474 - 1;
                    let t475 = b7_ss_f(l, lid0, ((t471[8 + j] as u64) << 32 | t471[j] as u64), t472[j]);
                    let t476: u64 = t475;
                    t473[j] = t476 as u32;
                    t473[8 + j] = (t476 >> 32) as u32;
                }
                t469 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t473);
            } else {
                let t477 = b7_ll_p(l, lid0, t466, x7, x8);
                t469 = t477;
            }
            t467 = [vbslq_u32(t466[0], t469[0], t467[0]), vbslq_u32(t466[1], t469[1], t467[1]), vbslq_u32(t466[0], t469[2], t467[2]), vbslq_u32(t466[1], t469[3], t467[3])];
        }
        {
            t467 = [vbslq_u32(t468[0], x6[0], t467[0]), vbslq_u32(t468[1], x6[1], t467[1]), vbslq_u32(t468[0], x6[2], t467[2]), vbslq_u32(t468[1], x6[3], t467[3])];
        }
        let t478 = (arg3 as *const u8).add(((gid0.wrapping_add((26u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t478 as *mut u32).add(0), uint32x4x2_t(t467[0], t467[2]));
        vst2q_u32((t478 as *mut u32).add(8), uint32x4x2_t(t467[1], t467[3]));
        x15 = (((((a1) & (7u64))) as u32)).wrapping_add(1u32);
        let t479 = b5_r0ns_f_v0(l, lid0, arg0, gid0.wrapping_mul(x15), x15, x15);
        let t480 = b8_s_f(l, lid0, (8u32).wrapping_mul(x15));
        let t481: u64 = t480;
        let t482 = [vdupq_n_u32((t481) as u32); 2];
        let t483 = [vdupq_n_u32(((t481) >> 32) as u32); 2];
        let t484: [uint32x4_t; 4] = [vandq_u32(t479[0], t482[0]), vandq_u32(t479[1], t482[1]), vandq_u32(t479[2], t483[0]), vandq_u32(t479[3], t483[1])];
        let t485 = (arg3 as *const u8).add(((gid0.wrapping_add((27u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t485 as *mut u32).add(0), uint32x4x2_t(t484[0], t484[2]));
        vst2q_u32((t485 as *mut u32).add(8), uint32x4x2_t(t484[1], t484[3]));
    } else if (((gid0) as u64 + 7) <= u32::MAX as u64 && ((gid0) as u64) >= ((a0) as u64)) {
        return;
    } else {
        // Lanes part: the chunk's lanes on both sides of the bound.
        let t486: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t487 = [vdupq_n_u32(a0); 2];
        let t488: [uint32x4_t; 2] = [vcgeq_u32(t486[0], t487[0]), vcgeq_u32(t486[1], t487[1])];
        let t489: [uint32x4_t; 2] = [vandq_u32(live[0], t488[0]), vandq_u32(live[1], t488[1])];
        {
            live = [vbicq_u32(live[0], t489[0]), vbicq_u32(live[1], t489[1])];
        }
        if any(&live) {
            let t490 = (arg0 as *const u8).add(((gid0) as usize) * 8);
            let mut t491: [uint32x4_t; 4];
            if all(&live) {
                let t4920 = vld2q_u32((t490 as *const u32).add(0));
                let t4921 = vld2q_u32((t490 as *const u32).add(8));
                let t492: [uint32x4_t; 4] = [t4920.0, t4921.0, t4920.1, t4921.1];
                t491 = t492;
            } else {
                let t493 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t486[0]), vandq_u32(live[1], t486[1]))));
                let t494: [uint32x4_t; 2] = [vbslq_u32(live[0], t486[0], t493), vbslq_u32(live[1], t486[1], t493)];
                let mut t495: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t494[0]) as usize * 8) as *const u64).read_unaligned(); t495[0] = vsetq_lane_u32::<0>(v as u32, t495[0]); t495[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t495[2]); }
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t494[0]) as usize * 8) as *const u64).read_unaligned(); t495[0] = vsetq_lane_u32::<1>(v as u32, t495[0]); t495[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t495[2]); }
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t494[0]) as usize * 8) as *const u64).read_unaligned(); t495[0] = vsetq_lane_u32::<2>(v as u32, t495[0]); t495[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t495[2]); }
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t494[0]) as usize * 8) as *const u64).read_unaligned(); t495[0] = vsetq_lane_u32::<3>(v as u32, t495[0]); t495[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t495[2]); }
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t494[1]) as usize * 8) as *const u64).read_unaligned(); t495[1] = vsetq_lane_u32::<0>(v as u32, t495[1]); t495[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t495[3]); }
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t494[1]) as usize * 8) as *const u64).read_unaligned(); t495[1] = vsetq_lane_u32::<1>(v as u32, t495[1]); t495[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t495[3]); }
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t494[1]) as usize * 8) as *const u64).read_unaligned(); t495[1] = vsetq_lane_u32::<2>(v as u32, t495[1]); t495[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t495[3]); }
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t494[1]) as usize * 8) as *const u64).read_unaligned(); t495[1] = vsetq_lane_u32::<3>(v as u32, t495[1]); t495[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t495[3]); }
                t491 = t495;
            }
            x6 = t491;
            let t496 = (arg1 as *const u8).add(((gid0) as usize) * 8);
            let mut t497: [uint32x4_t; 4];
            if all(&live) {
                let t4980 = vld2q_u32((t496 as *const u32).add(0));
                let t4981 = vld2q_u32((t496 as *const u32).add(8));
                let t498: [uint32x4_t; 4] = [t4980.0, t4981.0, t4980.1, t4981.1];
                t497 = t498;
            } else {
                let t499 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t486[0]), vandq_u32(live[1], t486[1]))));
                let t500: [uint32x4_t; 2] = [vbslq_u32(live[0], t486[0], t499), vbslq_u32(live[1], t486[1], t499)];
                let mut t501: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
                { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<0>(t500[0]) as usize * 8) as *const u64).read_unaligned(); t501[0] = vsetq_lane_u32::<0>(v as u32, t501[0]); t501[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t501[2]); }
                { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<1>(t500[0]) as usize * 8) as *const u64).read_unaligned(); t501[0] = vsetq_lane_u32::<1>(v as u32, t501[0]); t501[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t501[2]); }
                { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<2>(t500[0]) as usize * 8) as *const u64).read_unaligned(); t501[0] = vsetq_lane_u32::<2>(v as u32, t501[0]); t501[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t501[2]); }
                { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<3>(t500[0]) as usize * 8) as *const u64).read_unaligned(); t501[0] = vsetq_lane_u32::<3>(v as u32, t501[0]); t501[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t501[2]); }
                { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<0>(t500[1]) as usize * 8) as *const u64).read_unaligned(); t501[1] = vsetq_lane_u32::<0>(v as u32, t501[1]); t501[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t501[3]); }
                { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<1>(t500[1]) as usize * 8) as *const u64).read_unaligned(); t501[1] = vsetq_lane_u32::<1>(v as u32, t501[1]); t501[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t501[3]); }
                { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<2>(t500[1]) as usize * 8) as *const u64).read_unaligned(); t501[1] = vsetq_lane_u32::<2>(v as u32, t501[1]); t501[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t501[3]); }
                { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<3>(t500[1]) as usize * 8) as *const u64).read_unaligned(); t501[1] = vsetq_lane_u32::<3>(v as u32, t501[1]); t501[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t501[3]); }
                t497 = t501;
            }
            x7 = t497;
            let mut t502: [uint32x4_t; 2];
            if all(&live) {
                let t503 = (arg2 as *const u32).add((gid0) as usize);
                t502 = [vld1q_u32(t503.add(0)), vld1q_u32(t503.add(4))];
            } else {
                let t504 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t486[0]), vandq_u32(live[1], t486[1]))));
                let t505: [uint32x4_t; 2] = [vbslq_u32(live[0], t486[0], t504), vbslq_u32(live[1], t486[1], t504)];
                let t506 = arg2 as *const u32;
                let mut t507: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                t507[0] = vld1q_lane_u32::<0>(t506.add(vgetq_lane_u32::<0>(t505[0]) as usize), t507[0]);
                t507[0] = vld1q_lane_u32::<1>(t506.add(vgetq_lane_u32::<1>(t505[0]) as usize), t507[0]);
                t507[0] = vld1q_lane_u32::<2>(t506.add(vgetq_lane_u32::<2>(t505[0]) as usize), t507[0]);
                t507[0] = vld1q_lane_u32::<3>(t506.add(vgetq_lane_u32::<3>(t505[0]) as usize), t507[0]);
                t507[1] = vld1q_lane_u32::<0>(t506.add(vgetq_lane_u32::<0>(t505[1]) as usize), t507[1]);
                t507[1] = vld1q_lane_u32::<1>(t506.add(vgetq_lane_u32::<1>(t505[1]) as usize), t507[1]);
                t507[1] = vld1q_lane_u32::<2>(t506.add(vgetq_lane_u32::<2>(t505[1]) as usize), t507[1]);
                t507[1] = vld1q_lane_u32::<3>(t506.add(vgetq_lane_u32::<3>(t505[1]) as usize), t507[1]);
                t502 = t507;
            }
            x8 = t502;
            let t508 = [vdupq_n_u32(63u32); 2];
            let t509: [uint32x4_t; 2] = [vandq_u32(x8[0], t508[0]), vandq_u32(x8[1], t508[1])];
            x9 = t509;
            x10 = a0;
            let t510: [uint32x4_t; 2] = [vaddq_u32(x6[0], x7[0]), vaddq_u32(x6[1], x7[1])];
            let t511: [uint32x4_t; 2] = [vcltq_u32(t510[0], x6[0]), vcltq_u32(t510[1], x6[1])];
            let t512: [uint32x4_t; 4] = [t510[0], t510[1], vsubq_u32(vaddq_u32(x6[2], x7[2]), t511[0]), vsubq_u32(vaddq_u32(x6[3], x7[3]), t511[1])];
            let t513 = (arg3 as *const u8).add(((gid0.wrapping_add((0u32).wrapping_mul(x10))) as usize) * 8);
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
            let t515: [uint32x4_t; 2] = [vcltq_u32(x6[0], x7[0]), vcltq_u32(x6[1], x7[1])];
            let t516: [uint32x4_t; 4] = [vsubq_u32(x6[0], x7[0]), vsubq_u32(x6[1], x7[1]), vaddq_u32(vsubq_u32(x6[2], x7[2]), t515[0]), vaddq_u32(vsubq_u32(x6[3], x7[3]), t515[1])];
            let t517 = (arg3 as *const u8).add(((gid0.wrapping_add((1u32).wrapping_mul(x10))) as usize) * 8);
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
            let t519: [uint32x4_t; 2] = [x9[0], x9[1]];
            let t520: [uint32x4_t; 2] = [vandq_u32(t519[0], vdupq_n_u32(63)), vandq_u32(t519[1], vdupq_n_u32(63))];
            let t521: [uint32x4_t; 4] = [vshlq_u32(x6[0], vreinterpretq_s32_u32(t520[0])), vshlq_u32(x6[1], vreinterpretq_s32_u32(t520[1])), vorrq_u32(vshlq_u32(x6[2], vreinterpretq_s32_u32(t520[0])), vshlq_u32(x6[0], vsubq_s32(vreinterpretq_s32_u32(t520[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(x6[3], vreinterpretq_s32_u32(t520[1])), vshlq_u32(x6[1], vsubq_s32(vreinterpretq_s32_u32(t520[1]), vdupq_n_s32(32))))];
            let t522 = (arg3 as *const u8).add(((gid0.wrapping_add((2u32).wrapping_mul(x10))) as usize) * 8);
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
            let t524: [uint32x4_t; 2] = [x9[0], x9[1]];
            let t525: [uint32x4_t; 2] = [vandq_u32(t524[0], vdupq_n_u32(63)), vandq_u32(t524[1], vdupq_n_u32(63))];
            let t526: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(x6[0], vnegq_s32(vreinterpretq_s32_u32(t525[0]))), vshlq_u32(x6[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t525[0])))), vorrq_u32(vshlq_u32(x6[1], vnegq_s32(vreinterpretq_s32_u32(t525[1]))), vshlq_u32(x6[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t525[1])))), vshlq_u32(x6[2], vnegq_s32(vreinterpretq_s32_u32(t525[0]))), vshlq_u32(x6[3], vnegq_s32(vreinterpretq_s32_u32(t525[1])))];
            let t527 = (arg3 as *const u8).add(((gid0.wrapping_add((3u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t527 as *mut u32).add(0), uint32x4x2_t(t526[0], t526[2]));
                vst2q_u32((t527 as *mut u32).add(8), uint32x4x2_t(t526[1], t526[3]));
            } else {
                let mut t528w = 0u64;
                let t528 = core::hint::black_box(&raw mut t528w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t527 as *mut u64).add(0), t528).write_unaligned(((vgetq_lane_u32::<0>(t526[2]) as u64) << 32 | vgetq_lane_u32::<0>(t526[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t527 as *mut u64).add(1), t528).write_unaligned(((vgetq_lane_u32::<1>(t526[2]) as u64) << 32 | vgetq_lane_u32::<1>(t526[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t527 as *mut u64).add(2), t528).write_unaligned(((vgetq_lane_u32::<2>(t526[2]) as u64) << 32 | vgetq_lane_u32::<2>(t526[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t527 as *mut u64).add(3), t528).write_unaligned(((vgetq_lane_u32::<3>(t526[2]) as u64) << 32 | vgetq_lane_u32::<3>(t526[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t527 as *mut u64).add(4), t528).write_unaligned(((vgetq_lane_u32::<0>(t526[3]) as u64) << 32 | vgetq_lane_u32::<0>(t526[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t527 as *mut u64).add(5), t528).write_unaligned(((vgetq_lane_u32::<1>(t526[3]) as u64) << 32 | vgetq_lane_u32::<1>(t526[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t527 as *mut u64).add(6), t528).write_unaligned(((vgetq_lane_u32::<2>(t526[3]) as u64) << 32 | vgetq_lane_u32::<2>(t526[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t527 as *mut u64).add(7), t528).write_unaligned(((vgetq_lane_u32::<3>(t526[3]) as u64) << 32 | vgetq_lane_u32::<3>(t526[1]) as u64));
            }
            let t529: [uint32x4_t; 2] = [x9[0], x9[1]];
            let t530: [uint32x4_t; 2] = [vandq_u32(t529[0], vdupq_n_u32(63)), vandq_u32(t529[1], vdupq_n_u32(63))];
            let t531: [uint32x4_t; 2] = [vcgeq_s32(vreinterpretq_s32_u32(t530[0]), vdupq_n_s32(32)), vcgeq_s32(vreinterpretq_s32_u32(t530[1]), vdupq_n_s32(32))];
            let t532: [uint32x4_t; 4] = [vbslq_u32(t531[0], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t530[0])))), vorrq_u32(vshlq_u32(x6[0], vnegq_s32(vreinterpretq_s32_u32(t530[0]))), vshlq_u32(x6[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t530[0]))))), vbslq_u32(t531[1], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t530[1])))), vorrq_u32(vshlq_u32(x6[1], vnegq_s32(vreinterpretq_s32_u32(t530[1]))), vshlq_u32(x6[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t530[1]))))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vnegq_s32(vreinterpretq_s32_u32(t530[0])))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vnegq_s32(vreinterpretq_s32_u32(t530[1]))))];
            let t533 = (arg3 as *const u8).add(((gid0.wrapping_add((4u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t533 as *mut u32).add(0), uint32x4x2_t(t532[0], t532[2]));
                vst2q_u32((t533 as *mut u32).add(8), uint32x4x2_t(t532[1], t532[3]));
            } else {
                let mut t534w = 0u64;
                let t534 = core::hint::black_box(&raw mut t534w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t533 as *mut u64).add(0), t534).write_unaligned(((vgetq_lane_u32::<0>(t532[2]) as u64) << 32 | vgetq_lane_u32::<0>(t532[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t533 as *mut u64).add(1), t534).write_unaligned(((vgetq_lane_u32::<1>(t532[2]) as u64) << 32 | vgetq_lane_u32::<1>(t532[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t533 as *mut u64).add(2), t534).write_unaligned(((vgetq_lane_u32::<2>(t532[2]) as u64) << 32 | vgetq_lane_u32::<2>(t532[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t533 as *mut u64).add(3), t534).write_unaligned(((vgetq_lane_u32::<3>(t532[2]) as u64) << 32 | vgetq_lane_u32::<3>(t532[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t533 as *mut u64).add(4), t534).write_unaligned(((vgetq_lane_u32::<0>(t532[3]) as u64) << 32 | vgetq_lane_u32::<0>(t532[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t533 as *mut u64).add(5), t534).write_unaligned(((vgetq_lane_u32::<1>(t532[3]) as u64) << 32 | vgetq_lane_u32::<1>(t532[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t533 as *mut u64).add(6), t534).write_unaligned(((vgetq_lane_u32::<2>(t532[3]) as u64) << 32 | vgetq_lane_u32::<2>(t532[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t533 as *mut u64).add(7), t534).write_unaligned(((vgetq_lane_u32::<3>(t532[3]) as u64) << 32 | vgetq_lane_u32::<3>(t532[1]) as u64));
            }
            let t535: [uint32x4_t; 4] = [vshlq_n_u32::<13>(x6[0]), vshlq_n_u32::<13>(x6[1]), vorrq_u32(vshlq_n_u32::<13>(x6[2]), vshrq_n_u32::<19>(x6[0])), vorrq_u32(vshlq_n_u32::<13>(x6[3]), vshrq_n_u32::<19>(x6[1]))];
            let t536: [uint32x4_t; 2] = [vshrq_n_u32::<19>(x7[2]), vshrq_n_u32::<19>(x7[3])];
            let t537: [uint32x4_t; 4] = [vorrq_u32(t535[0], t536[0]), vorrq_u32(t535[1], t536[1]), t535[2], t535[3]];
            let t538 = (arg3 as *const u8).add(((gid0.wrapping_add((5u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t538 as *mut u32).add(0), uint32x4x2_t(t537[0], t537[2]));
                vst2q_u32((t538 as *mut u32).add(8), uint32x4x2_t(t537[1], t537[3]));
            } else {
                let mut t539w = 0u64;
                let t539 = core::hint::black_box(&raw mut t539w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t538 as *mut u64).add(0), t539).write_unaligned(((vgetq_lane_u32::<0>(t537[2]) as u64) << 32 | vgetq_lane_u32::<0>(t537[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t538 as *mut u64).add(1), t539).write_unaligned(((vgetq_lane_u32::<1>(t537[2]) as u64) << 32 | vgetq_lane_u32::<1>(t537[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t538 as *mut u64).add(2), t539).write_unaligned(((vgetq_lane_u32::<2>(t537[2]) as u64) << 32 | vgetq_lane_u32::<2>(t537[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t538 as *mut u64).add(3), t539).write_unaligned(((vgetq_lane_u32::<3>(t537[2]) as u64) << 32 | vgetq_lane_u32::<3>(t537[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t538 as *mut u64).add(4), t539).write_unaligned(((vgetq_lane_u32::<0>(t537[3]) as u64) << 32 | vgetq_lane_u32::<0>(t537[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t538 as *mut u64).add(5), t539).write_unaligned(((vgetq_lane_u32::<1>(t537[3]) as u64) << 32 | vgetq_lane_u32::<1>(t537[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t538 as *mut u64).add(6), t539).write_unaligned(((vgetq_lane_u32::<2>(t537[3]) as u64) << 32 | vgetq_lane_u32::<2>(t537[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t538 as *mut u64).add(7), t539).write_unaligned(((vgetq_lane_u32::<3>(t537[3]) as u64) << 32 | vgetq_lane_u32::<3>(t537[1]) as u64));
            }
            let t540: [uint32x4_t; 4] = [vdupq_n_u32(0), vdupq_n_u32(0), vshlq_n_u32::<8>(x6[0]), vshlq_n_u32::<8>(x6[1])];
            let t541 = (arg3 as *const u8).add(((gid0.wrapping_add((6u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t541 as *mut u32).add(0), uint32x4x2_t(t540[0], t540[2]));
                vst2q_u32((t541 as *mut u32).add(8), uint32x4x2_t(t540[1], t540[3]));
            } else {
                let mut t542w = 0u64;
                let t542 = core::hint::black_box(&raw mut t542w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t541 as *mut u64).add(0), t542).write_unaligned(((vgetq_lane_u32::<0>(t540[2]) as u64) << 32 | vgetq_lane_u32::<0>(t540[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t541 as *mut u64).add(1), t542).write_unaligned(((vgetq_lane_u32::<1>(t540[2]) as u64) << 32 | vgetq_lane_u32::<1>(t540[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t541 as *mut u64).add(2), t542).write_unaligned(((vgetq_lane_u32::<2>(t540[2]) as u64) << 32 | vgetq_lane_u32::<2>(t540[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t541 as *mut u64).add(3), t542).write_unaligned(((vgetq_lane_u32::<3>(t540[2]) as u64) << 32 | vgetq_lane_u32::<3>(t540[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t541 as *mut u64).add(4), t542).write_unaligned(((vgetq_lane_u32::<0>(t540[3]) as u64) << 32 | vgetq_lane_u32::<0>(t540[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t541 as *mut u64).add(5), t542).write_unaligned(((vgetq_lane_u32::<1>(t540[3]) as u64) << 32 | vgetq_lane_u32::<1>(t540[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t541 as *mut u64).add(6), t542).write_unaligned(((vgetq_lane_u32::<2>(t540[3]) as u64) << 32 | vgetq_lane_u32::<2>(t540[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t541 as *mut u64).add(7), t542).write_unaligned(((vgetq_lane_u32::<3>(t540[3]) as u64) << 32 | vgetq_lane_u32::<3>(t540[1]) as u64));
            }
            let t543: [uint32x4_t; 2] = [vshrq_n_u32::<3>(x6[2]), vshrq_n_u32::<3>(x6[3])];
            let t544: [uint32x4_t; 4] = [t543[0], t543[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            let t545 = (arg3 as *const u8).add(((gid0.wrapping_add((7u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t545 as *mut u32).add(0), uint32x4x2_t(t544[0], t544[2]));
                vst2q_u32((t545 as *mut u32).add(8), uint32x4x2_t(t544[1], t544[3]));
            } else {
                let mut t546w = 0u64;
                let t546 = core::hint::black_box(&raw mut t546w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t545 as *mut u64).add(0), t546).write_unaligned(((vgetq_lane_u32::<0>(t544[2]) as u64) << 32 | vgetq_lane_u32::<0>(t544[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t545 as *mut u64).add(1), t546).write_unaligned(((vgetq_lane_u32::<1>(t544[2]) as u64) << 32 | vgetq_lane_u32::<1>(t544[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t545 as *mut u64).add(2), t546).write_unaligned(((vgetq_lane_u32::<2>(t544[2]) as u64) << 32 | vgetq_lane_u32::<2>(t544[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t545 as *mut u64).add(3), t546).write_unaligned(((vgetq_lane_u32::<3>(t544[2]) as u64) << 32 | vgetq_lane_u32::<3>(t544[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t545 as *mut u64).add(4), t546).write_unaligned(((vgetq_lane_u32::<0>(t544[3]) as u64) << 32 | vgetq_lane_u32::<0>(t544[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t545 as *mut u64).add(5), t546).write_unaligned(((vgetq_lane_u32::<1>(t544[3]) as u64) << 32 | vgetq_lane_u32::<1>(t544[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t545 as *mut u64).add(6), t546).write_unaligned(((vgetq_lane_u32::<2>(t544[3]) as u64) << 32 | vgetq_lane_u32::<2>(t544[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t545 as *mut u64).add(7), t546).write_unaligned(((vgetq_lane_u32::<3>(t544[3]) as u64) << 32 | vgetq_lane_u32::<3>(t544[1]) as u64));
            }
            let t547 = vdupq_n_s32(((((40i64) as u32)) & 63) as i32);
            let t548: [uint32x4_t; 2] = [vcgeq_s32(t547, vdupq_n_s32(32)), vcgeq_s32(t547, vdupq_n_s32(32))];
            let t549: [uint32x4_t; 4] = [vbslq_u32(t548[0], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vsubq_s32(vdupq_n_s32(32), t547))), vorrq_u32(vshlq_u32(x6[0], vnegq_s32(t547)), vshlq_u32(x6[2], vsubq_s32(vdupq_n_s32(32), t547)))), vbslq_u32(t548[1], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vsubq_s32(vdupq_n_s32(32), t547))), vorrq_u32(vshlq_u32(x6[1], vnegq_s32(t547)), vshlq_u32(x6[3], vsubq_s32(vdupq_n_s32(32), t547)))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vnegq_s32(t547))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vnegq_s32(t547)))];
            let t550 = (arg3 as *const u8).add(((gid0.wrapping_add((8u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t550 as *mut u32).add(0), uint32x4x2_t(t549[0], t549[2]));
                vst2q_u32((t550 as *mut u32).add(8), uint32x4x2_t(t549[1], t549[3]));
            } else {
                let mut t551w = 0u64;
                let t551 = core::hint::black_box(&raw mut t551w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t550 as *mut u64).add(0), t551).write_unaligned(((vgetq_lane_u32::<0>(t549[2]) as u64) << 32 | vgetq_lane_u32::<0>(t549[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t550 as *mut u64).add(1), t551).write_unaligned(((vgetq_lane_u32::<1>(t549[2]) as u64) << 32 | vgetq_lane_u32::<1>(t549[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t550 as *mut u64).add(2), t551).write_unaligned(((vgetq_lane_u32::<2>(t549[2]) as u64) << 32 | vgetq_lane_u32::<2>(t549[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t550 as *mut u64).add(3), t551).write_unaligned(((vgetq_lane_u32::<3>(t549[2]) as u64) << 32 | vgetq_lane_u32::<3>(t549[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t550 as *mut u64).add(4), t551).write_unaligned(((vgetq_lane_u32::<0>(t549[3]) as u64) << 32 | vgetq_lane_u32::<0>(t549[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t550 as *mut u64).add(5), t551).write_unaligned(((vgetq_lane_u32::<1>(t549[3]) as u64) << 32 | vgetq_lane_u32::<1>(t549[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t550 as *mut u64).add(6), t551).write_unaligned(((vgetq_lane_u32::<2>(t549[3]) as u64) << 32 | vgetq_lane_u32::<2>(t549[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t550 as *mut u64).add(7), t551).write_unaligned(((vgetq_lane_u32::<3>(t549[3]) as u64) << 32 | vgetq_lane_u32::<3>(t549[1]) as u64));
            }
            let t552: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x6[0]), vget_low_u32(x7[0]))), vreinterpretq_u32_u64(vmull_high_u32(x6[0], x7[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x6[1]), vget_low_u32(x7[1]))), vreinterpretq_u32_u64(vmull_high_u32(x6[1], x7[1])))];
            let t553: [uint32x4_t; 4] = [vmulq_u32(x6[0], x7[0]), vmulq_u32(x6[1], x7[1]), vmlaq_u32(vmlaq_u32(t552[0], x6[0], x7[2]), x7[0], x6[2]), vmlaq_u32(vmlaq_u32(t552[1], x6[1], x7[3]), x7[1], x6[3])];
            let t554 = (arg3 as *const u8).add(((gid0.wrapping_add((9u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t554 as *mut u32).add(0), uint32x4x2_t(t553[0], t553[2]));
                vst2q_u32((t554 as *mut u32).add(8), uint32x4x2_t(t553[1], t553[3]));
            } else {
                let mut t555w = 0u64;
                let t555 = core::hint::black_box(&raw mut t555w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t554 as *mut u64).add(0), t555).write_unaligned(((vgetq_lane_u32::<0>(t553[2]) as u64) << 32 | vgetq_lane_u32::<0>(t553[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t554 as *mut u64).add(1), t555).write_unaligned(((vgetq_lane_u32::<1>(t553[2]) as u64) << 32 | vgetq_lane_u32::<1>(t553[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t554 as *mut u64).add(2), t555).write_unaligned(((vgetq_lane_u32::<2>(t553[2]) as u64) << 32 | vgetq_lane_u32::<2>(t553[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t554 as *mut u64).add(3), t555).write_unaligned(((vgetq_lane_u32::<3>(t553[2]) as u64) << 32 | vgetq_lane_u32::<3>(t553[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t554 as *mut u64).add(4), t555).write_unaligned(((vgetq_lane_u32::<0>(t553[3]) as u64) << 32 | vgetq_lane_u32::<0>(t553[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t554 as *mut u64).add(5), t555).write_unaligned(((vgetq_lane_u32::<1>(t553[3]) as u64) << 32 | vgetq_lane_u32::<1>(t553[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t554 as *mut u64).add(6), t555).write_unaligned(((vgetq_lane_u32::<2>(t553[3]) as u64) << 32 | vgetq_lane_u32::<2>(t553[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t554 as *mut u64).add(7), t555).write_unaligned(((vgetq_lane_u32::<3>(t553[3]) as u64) << 32 | vgetq_lane_u32::<3>(t553[1]) as u64));
            }
            let t556: u64 = 0u64;
            let t557 = [vdupq_n_u32((t556) as u32); 2];
            let t558: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x7[0], t557[0]), vceqq_u32(x7[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(x7[1], t557[1]), vceqq_u32(x7[3], vdupq_n_u32(0)))];
            let t559: [uint32x4_t; 2] = [vmvnq_u32(t558[0]), vmvnq_u32(t558[1])];
            let mut t560: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            let t561: [uint32x4_t; 2] = [vandq_u32(live[0], t559[0]), vandq_u32(live[1], t559[1])];
            let t562: [uint32x4_t; 2] = [vbicq_u32(live[0], t559[0]), vbicq_u32(live[1], t559[1])];
            {
                let (mut t563xl, mut t563xh, mut t563yl, mut t563yh) = ([0u32; 8], [0u32; 8], [0u32; 8], [0u32; 8]);
                vst1q_u32(t563xl.as_mut_ptr().add(0), x6[0]); vst1q_u32(t563xh.as_mut_ptr().add(0), x6[2]); vst1q_u32(t563yl.as_mut_ptr().add(0), x7[0]); vst1q_u32(t563yh.as_mut_ptr().add(0), x7[2]);
                vst1q_u32(t563xl.as_mut_ptr().add(4), x6[1]); vst1q_u32(t563xh.as_mut_ptr().add(4), x6[3]); vst1q_u32(t563yl.as_mut_ptr().add(4), x7[1]); vst1q_u32(t563yh.as_mut_ptr().add(4), x7[3]);
                for j in 0..8 { let q = (((t563xh[j] as u64) << 32 | t563xl[j] as u64) as u64).checked_div(((t563yh[j] as u64) << 32 | t563yl[j] as u64) as u64).unwrap_or(0) as u64; t563xl[j] = q as u32; t563xh[j] = (q >> 32) as u32; }
                let t564: [uint32x4_t; 4] = [vld1q_u32(t563xl.as_ptr().add(0)), vld1q_u32(t563xl.as_ptr().add(4)), vld1q_u32(t563xh.as_ptr().add(0)), vld1q_u32(t563xh.as_ptr().add(4))];
                t560 = [vbslq_u32(t561[0], t564[0], t560[0]), vbslq_u32(t561[1], t564[1], t560[1]), vbslq_u32(t561[0], t564[2], t560[2]), vbslq_u32(t561[1], t564[3], t560[3])];
            }
            {
                let t565: u64 = 7u64;
                let t566 = [vdupq_n_u32((t565) as u32); 2];
                let t567: [uint32x4_t; 4] = [t566[0], t566[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                t560 = [vbslq_u32(t562[0], t567[0], t560[0]), vbslq_u32(t562[1], t567[1], t560[1]), vbslq_u32(t562[0], t567[2], t560[2]), vbslq_u32(t562[1], t567[3], t560[3])];
            }
            let t568 = (arg3 as *const u8).add(((gid0.wrapping_add((10u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t568 as *mut u32).add(0), uint32x4x2_t(t560[0], t560[2]));
                vst2q_u32((t568 as *mut u32).add(8), uint32x4x2_t(t560[1], t560[3]));
            } else {
                let mut t569w = 0u64;
                let t569 = core::hint::black_box(&raw mut t569w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t568 as *mut u64).add(0), t569).write_unaligned(((vgetq_lane_u32::<0>(t560[2]) as u64) << 32 | vgetq_lane_u32::<0>(t560[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t568 as *mut u64).add(1), t569).write_unaligned(((vgetq_lane_u32::<1>(t560[2]) as u64) << 32 | vgetq_lane_u32::<1>(t560[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t568 as *mut u64).add(2), t569).write_unaligned(((vgetq_lane_u32::<2>(t560[2]) as u64) << 32 | vgetq_lane_u32::<2>(t560[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t568 as *mut u64).add(3), t569).write_unaligned(((vgetq_lane_u32::<3>(t560[2]) as u64) << 32 | vgetq_lane_u32::<3>(t560[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t568 as *mut u64).add(4), t569).write_unaligned(((vgetq_lane_u32::<0>(t560[3]) as u64) << 32 | vgetq_lane_u32::<0>(t560[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t568 as *mut u64).add(5), t569).write_unaligned(((vgetq_lane_u32::<1>(t560[3]) as u64) << 32 | vgetq_lane_u32::<1>(t560[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t568 as *mut u64).add(6), t569).write_unaligned(((vgetq_lane_u32::<2>(t560[3]) as u64) << 32 | vgetq_lane_u32::<2>(t560[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t568 as *mut u64).add(7), t569).write_unaligned(((vgetq_lane_u32::<3>(t560[3]) as u64) << 32 | vgetq_lane_u32::<3>(t560[1]) as u64));
            }
            let t570: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x6[2], x7[2]), vandq_u32(vceqq_u32(x6[2], x7[2]), vcltq_u32(x6[0], x7[0]))), vorrq_u32(vcltq_u32(x6[3], x7[3]), vandq_u32(vceqq_u32(x6[3], x7[3]), vcltq_u32(x6[1], x7[1])))];
            let t571: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t570[0]), vshrq_n_u32::<31>(t570[1])];
            let t572: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x7[2], x6[2]), vandq_u32(vceqq_u32(x7[2], x6[2]), vcltq_u32(x7[0], x6[0]))), vorrq_u32(vcltq_u32(x7[3], x6[3]), vandq_u32(vceqq_u32(x7[3], x6[3]), vcltq_u32(x7[1], x6[1])))];
            let t573: [uint32x4_t; 2] = [vmvnq_u32(t572[0]), vmvnq_u32(t572[1])];
            let t574: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t573[0]), vshrq_n_u32::<31>(t573[1])];
            let t575: [uint32x4_t; 4] = [vshlq_n_u32::<1>(t574[0]), vshlq_n_u32::<1>(t574[1]), vshrq_n_u32::<31>(t574[0]), vshrq_n_u32::<31>(t574[1])];
            let t576: [uint32x4_t; 4] = [vorrq_u32(t571[0], t575[0]), vorrq_u32(t571[1], t575[1]), t575[2], t575[3]];
            let t577: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x6[0], x7[0]), vceqq_u32(x6[2], x7[2])), vandq_u32(vceqq_u32(x6[1], x7[1]), vceqq_u32(x6[3], x7[3]))];
            let t578: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t577[0]), vshrq_n_u32::<31>(t577[1])];
            let t579: [uint32x4_t; 4] = [vshlq_n_u32::<2>(t578[0]), vshlq_n_u32::<2>(t578[1]), vshrq_n_u32::<30>(t578[0]), vshrq_n_u32::<30>(t578[1])];
            let t580: [uint32x4_t; 4] = [vorrq_u32(t576[0], t579[0]), vorrq_u32(t576[1], t579[1]), vorrq_u32(t576[2], t579[2]), vorrq_u32(t576[3], t579[3])];
            let t581: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x6[0], x7[0]), vceqq_u32(x6[2], x7[2])), vandq_u32(vceqq_u32(x6[1], x7[1]), vceqq_u32(x6[3], x7[3]))];
            let t582: [uint32x4_t; 2] = [vmvnq_u32(t581[0]), vmvnq_u32(t581[1])];
            let t583: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t582[0]), vshrq_n_u32::<31>(t582[1])];
            let t584: [uint32x4_t; 4] = [vshlq_n_u32::<3>(t583[0]), vshlq_n_u32::<3>(t583[1]), vshrq_n_u32::<29>(t583[0]), vshrq_n_u32::<29>(t583[1])];
            let t585: [uint32x4_t; 4] = [vorrq_u32(t580[0], t584[0]), vorrq_u32(t580[1], t584[1]), vorrq_u32(t580[2], t584[2]), vorrq_u32(t580[3], t584[3])];
            let t586: [uint32x4_t; 2] = [vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x6[2]), vreinterpretq_s32_u32(x7[2])), vandq_u32(vceqq_u32(x6[2], x7[2]), vcltq_u32(x6[0], x7[0]))), vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x6[3]), vreinterpretq_s32_u32(x7[3])), vandq_u32(vceqq_u32(x6[3], x7[3]), vcltq_u32(x6[1], x7[1])))];
            let t587: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t586[0]), vshrq_n_u32::<31>(t586[1])];
            let t588: [uint32x4_t; 4] = [vshlq_n_u32::<4>(t587[0]), vshlq_n_u32::<4>(t587[1]), vshrq_n_u32::<28>(t587[0]), vshrq_n_u32::<28>(t587[1])];
            let t589: [uint32x4_t; 4] = [vorrq_u32(t585[0], t588[0]), vorrq_u32(t585[1], t588[1]), vorrq_u32(t585[2], t588[2]), vorrq_u32(t585[3], t588[3])];
            let t590: [uint32x4_t; 2] = [vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x6[2]), vreinterpretq_s32_u32(x7[2])), vandq_u32(vceqq_u32(x6[2], x7[2]), vcltq_u32(x6[0], x7[0]))), vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x6[3]), vreinterpretq_s32_u32(x7[3])), vandq_u32(vceqq_u32(x6[3], x7[3]), vcltq_u32(x6[1], x7[1])))];
            let t591: [uint32x4_t; 2] = [vmvnq_u32(t590[0]), vmvnq_u32(t590[1])];
            let t592: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t591[0]), vshrq_n_u32::<31>(t591[1])];
            let t593: [uint32x4_t; 4] = [vshlq_n_u32::<5>(t592[0]), vshlq_n_u32::<5>(t592[1]), vshrq_n_u32::<27>(t592[0]), vshrq_n_u32::<27>(t592[1])];
            let t594: [uint32x4_t; 4] = [vorrq_u32(t589[0], t593[0]), vorrq_u32(t589[1], t593[1]), vorrq_u32(t589[2], t593[2]), vorrq_u32(t589[3], t593[3])];
            let t595: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x7[2], x6[2]), vandq_u32(vceqq_u32(x7[2], x6[2]), vcltq_u32(x7[0], x6[0]))), vorrq_u32(vcltq_u32(x7[3], x6[3]), vandq_u32(vceqq_u32(x7[3], x6[3]), vcltq_u32(x7[1], x6[1])))];
            let t596: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t595[0]), vshrq_n_u32::<31>(t595[1])];
            let t597: [uint32x4_t; 4] = [vshlq_n_u32::<6>(t596[0]), vshlq_n_u32::<6>(t596[1]), vshrq_n_u32::<26>(t596[0]), vshrq_n_u32::<26>(t596[1])];
            let t598: [uint32x4_t; 4] = [vorrq_u32(t594[0], t597[0]), vorrq_u32(t594[1], t597[1]), vorrq_u32(t594[2], t597[2]), vorrq_u32(t594[3], t597[3])];
            let t599 = (arg3 as *const u8).add(((gid0.wrapping_add((11u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t599 as *mut u32).add(0), uint32x4x2_t(t598[0], t598[2]));
                vst2q_u32((t599 as *mut u32).add(8), uint32x4x2_t(t598[1], t598[3]));
            } else {
                let mut t600w = 0u64;
                let t600 = core::hint::black_box(&raw mut t600w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t599 as *mut u64).add(0), t600).write_unaligned(((vgetq_lane_u32::<0>(t598[2]) as u64) << 32 | vgetq_lane_u32::<0>(t598[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t599 as *mut u64).add(1), t600).write_unaligned(((vgetq_lane_u32::<1>(t598[2]) as u64) << 32 | vgetq_lane_u32::<1>(t598[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t599 as *mut u64).add(2), t600).write_unaligned(((vgetq_lane_u32::<2>(t598[2]) as u64) << 32 | vgetq_lane_u32::<2>(t598[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t599 as *mut u64).add(3), t600).write_unaligned(((vgetq_lane_u32::<3>(t598[2]) as u64) << 32 | vgetq_lane_u32::<3>(t598[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t599 as *mut u64).add(4), t600).write_unaligned(((vgetq_lane_u32::<0>(t598[3]) as u64) << 32 | vgetq_lane_u32::<0>(t598[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t599 as *mut u64).add(5), t600).write_unaligned(((vgetq_lane_u32::<1>(t598[3]) as u64) << 32 | vgetq_lane_u32::<1>(t598[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t599 as *mut u64).add(6), t600).write_unaligned(((vgetq_lane_u32::<2>(t598[3]) as u64) << 32 | vgetq_lane_u32::<2>(t598[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t599 as *mut u64).add(7), t600).write_unaligned(((vgetq_lane_u32::<3>(t598[3]) as u64) << 32 | vgetq_lane_u32::<3>(t598[1]) as u64));
            }
            let t601: [uint32x4_t; 2] = [vaddq_u32(x8[0], x6[0]), vaddq_u32(x8[1], x6[1])];
            let t602: [uint32x4_t; 2] = [vcltq_u32(t601[0], x8[0]), vcltq_u32(t601[1], x8[1])];
            let t603: [uint32x4_t; 4] = [t601[0], t601[1], vsubq_u32(vaddq_u32(vdupq_n_u32(0), x6[2]), t602[0]), vsubq_u32(vaddq_u32(vdupq_n_u32(0), x6[3]), t602[1])];
            let t604 = (arg3 as *const u8).add(((gid0.wrapping_add((12u32).wrapping_mul(x10))) as usize) * 8);
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
            let t606: [uint32x4_t; 2] = [vreinterpretq_u32_s32(vshrq_n_s32::<31>(vreinterpretq_s32_u32(x8[0]))), vreinterpretq_u32_s32(vshrq_n_s32::<31>(vreinterpretq_s32_u32(x8[1])))];
            let t607: [uint32x4_t; 4] = [x8[0], x8[1], t606[0], t606[1]];
            let t608 = (arg3 as *const u8).add(((gid0.wrapping_add((13u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t608 as *mut u32).add(0), uint32x4x2_t(t607[0], t607[2]));
                vst2q_u32((t608 as *mut u32).add(8), uint32x4x2_t(t607[1], t607[3]));
            } else {
                let mut t609w = 0u64;
                let t609 = core::hint::black_box(&raw mut t609w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t608 as *mut u64).add(0), t609).write_unaligned(((vgetq_lane_u32::<0>(t607[2]) as u64) << 32 | vgetq_lane_u32::<0>(t607[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t608 as *mut u64).add(1), t609).write_unaligned(((vgetq_lane_u32::<1>(t607[2]) as u64) << 32 | vgetq_lane_u32::<1>(t607[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t608 as *mut u64).add(2), t609).write_unaligned(((vgetq_lane_u32::<2>(t607[2]) as u64) << 32 | vgetq_lane_u32::<2>(t607[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t608 as *mut u64).add(3), t609).write_unaligned(((vgetq_lane_u32::<3>(t607[2]) as u64) << 32 | vgetq_lane_u32::<3>(t607[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t608 as *mut u64).add(4), t609).write_unaligned(((vgetq_lane_u32::<0>(t607[3]) as u64) << 32 | vgetq_lane_u32::<0>(t607[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t608 as *mut u64).add(5), t609).write_unaligned(((vgetq_lane_u32::<1>(t607[3]) as u64) << 32 | vgetq_lane_u32::<1>(t607[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t608 as *mut u64).add(6), t609).write_unaligned(((vgetq_lane_u32::<2>(t607[3]) as u64) << 32 | vgetq_lane_u32::<2>(t607[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t608 as *mut u64).add(7), t609).write_unaligned(((vgetq_lane_u32::<3>(t607[3]) as u64) << 32 | vgetq_lane_u32::<3>(t607[1]) as u64));
            }
            let t610: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<3>(x6[0]), vshlq_n_u32::<29>(x6[2])), vorrq_u32(vshrq_n_u32::<3>(x6[1]), vshlq_n_u32::<29>(x6[3])), vshrq_n_u32::<3>(x6[2]), vshrq_n_u32::<3>(x6[3])];
            let t611: [uint32x4_t; 2] = [t610[0], t610[1]];
            let t612: [uint32x4_t; 4] = [t611[0], t611[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            let t613 = (arg3 as *const u8).add(((gid0.wrapping_add((14u32).wrapping_mul(x10))) as usize) * 8);
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
            let t615: u64 = 1u64;
            let t616 = [vdupq_n_u32((t615) as u32); 2];
            let t617: [uint32x4_t; 2] = [vandq_u32(x6[0], t616[0]), vandq_u32(x6[1], t616[1])];
            let t618: [uint32x4_t; 2] = [vtstq_u32(t617[0], t617[0]), vtstq_u32(t617[1], t617[1])];
            let t619: [uint32x4_t; 2] = [vandq_u32(live[0], t618[0]), vandq_u32(live[1], t618[1])];
            {
                let t620: [uint32x4_t; 4] = [veorq_u32(x6[0], x7[0]), veorq_u32(x6[1], x7[1]), veorq_u32(x6[2], x7[2]), veorq_u32(x6[3], x7[3])];
                x11 = [vbslq_u32(t619[0], t620[0], x11[0]), vbslq_u32(t619[1], t620[1], x11[1]), vbslq_u32(t619[0], t620[2], x11[2]), vbslq_u32(t619[1], t620[3], x11[3])];
            }
            let t621: [uint32x4_t; 2] = [vbicq_u32(live[0], t618[0]), vbicq_u32(live[1], t618[1])];
            {
                let t622: [uint32x4_t; 2] = [vaddq_u32(x6[0], x8[0]), vaddq_u32(x6[1], x8[1])];
                let t623: [uint32x4_t; 2] = [vcltq_u32(t622[0], x6[0]), vcltq_u32(t622[1], x6[1])];
                let t624: [uint32x4_t; 4] = [t622[0], t622[1], vsubq_u32(vaddq_u32(x6[2], vdupq_n_u32(0)), t623[0]), vsubq_u32(vaddq_u32(x6[3], vdupq_n_u32(0)), t623[1])];
                x11 = [vbslq_u32(t621[0], t624[0], x11[0]), vbslq_u32(t621[1], t624[1], x11[1]), vbslq_u32(t621[0], t624[2], x11[2]), vbslq_u32(t621[1], t624[3], x11[3])];
            }
            let t625 = (arg3 as *const u8).add(((gid0.wrapping_add((15u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t625 as *mut u32).add(0), uint32x4x2_t(x11[0], x11[2]));
                vst2q_u32((t625 as *mut u32).add(8), uint32x4x2_t(x11[1], x11[3]));
            } else {
                let mut t626w = 0u64;
                let t626 = core::hint::black_box(&raw mut t626w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t625 as *mut u64).add(0), t626).write_unaligned(((vgetq_lane_u32::<0>(x11[2]) as u64) << 32 | vgetq_lane_u32::<0>(x11[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t625 as *mut u64).add(1), t626).write_unaligned(((vgetq_lane_u32::<1>(x11[2]) as u64) << 32 | vgetq_lane_u32::<1>(x11[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t625 as *mut u64).add(2), t626).write_unaligned(((vgetq_lane_u32::<2>(x11[2]) as u64) << 32 | vgetq_lane_u32::<2>(x11[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t625 as *mut u64).add(3), t626).write_unaligned(((vgetq_lane_u32::<3>(x11[2]) as u64) << 32 | vgetq_lane_u32::<3>(x11[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t625 as *mut u64).add(4), t626).write_unaligned(((vgetq_lane_u32::<0>(x11[3]) as u64) << 32 | vgetq_lane_u32::<0>(x11[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t625 as *mut u64).add(5), t626).write_unaligned(((vgetq_lane_u32::<1>(x11[3]) as u64) << 32 | vgetq_lane_u32::<1>(x11[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t625 as *mut u64).add(6), t626).write_unaligned(((vgetq_lane_u32::<2>(x11[3]) as u64) << 32 | vgetq_lane_u32::<2>(x11[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t625 as *mut u64).add(7), t626).write_unaligned(((vgetq_lane_u32::<3>(x11[3]) as u64) << 32 | vgetq_lane_u32::<3>(x11[1]) as u64));
            }
            let t627 = b1_l_p(l, lid0, live, x6);
            let t628 = b2_l_p(l, lid0, live, x7);
            let t629: [uint32x4_t; 4] = [vshlq_n_u32::<8>(t628[0]), vshlq_n_u32::<8>(t628[1]), vshrq_n_u32::<24>(t628[0]), vshrq_n_u32::<24>(t628[1])];
            let t630: [uint32x4_t; 2] = [vaddq_u32(t627[0], t629[0]), vaddq_u32(t627[1], t629[1])];
            let t631: [uint32x4_t; 2] = [vcltq_u32(t630[0], t627[0]), vcltq_u32(t630[1], t627[1])];
            let t632: [uint32x4_t; 4] = [t630[0], t630[1], vsubq_u32(vaddq_u32(vdupq_n_u32(0), t629[2]), t631[0]), vsubq_u32(vaddq_u32(vdupq_n_u32(0), t629[3]), t631[1])];
            let t633 = (arg3 as *const u8).add(((gid0.wrapping_add((16u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t633 as *mut u32).add(0), uint32x4x2_t(t632[0], t632[2]));
                vst2q_u32((t633 as *mut u32).add(8), uint32x4x2_t(t632[1], t632[3]));
            } else {
                let mut t634w = 0u64;
                let t634 = core::hint::black_box(&raw mut t634w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t633 as *mut u64).add(0), t634).write_unaligned(((vgetq_lane_u32::<0>(t632[2]) as u64) << 32 | vgetq_lane_u32::<0>(t632[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t633 as *mut u64).add(1), t634).write_unaligned(((vgetq_lane_u32::<1>(t632[2]) as u64) << 32 | vgetq_lane_u32::<1>(t632[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t633 as *mut u64).add(2), t634).write_unaligned(((vgetq_lane_u32::<2>(t632[2]) as u64) << 32 | vgetq_lane_u32::<2>(t632[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t633 as *mut u64).add(3), t634).write_unaligned(((vgetq_lane_u32::<3>(t632[2]) as u64) << 32 | vgetq_lane_u32::<3>(t632[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t633 as *mut u64).add(4), t634).write_unaligned(((vgetq_lane_u32::<0>(t632[3]) as u64) << 32 | vgetq_lane_u32::<0>(t632[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t633 as *mut u64).add(5), t634).write_unaligned(((vgetq_lane_u32::<1>(t632[3]) as u64) << 32 | vgetq_lane_u32::<1>(t632[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t633 as *mut u64).add(6), t634).write_unaligned(((vgetq_lane_u32::<2>(t632[3]) as u64) << 32 | vgetq_lane_u32::<2>(t632[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t633 as *mut u64).add(7), t634).write_unaligned(((vgetq_lane_u32::<3>(t632[3]) as u64) << 32 | vgetq_lane_u32::<3>(t632[1]) as u64));
            }
            let t635 = [vdupq_n_u32(x10); 2];
            let mut t636x = [0u32; 8];
            let mut t636y = [0u32; 8];
            for r in 0..2 { vst1q_u32(t636x.as_mut_ptr().add(4 * r), x8[r]); vst1q_u32(t636y.as_mut_ptr().add(4 * r), t635[r]); }
            for j in 0..8 { t636x[j] = (t636x[j] as u32).checked_rem(t636y[j] as u32).unwrap_or(0) as u32; }
            let t637: [uint32x4_t; 2] = [vld1q_u32(t636x.as_ptr().add(0)), vld1q_u32(t636x.as_ptr().add(4))];
            let t638 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t637[0]), vandq_u32(live[1], t637[1]))));
            let t639: [uint32x4_t; 2] = [vbslq_u32(live[0], t637[0], t638), vbslq_u32(live[1], t637[1], t638)];
            let mut t640: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t639[0]) as usize * 8) as *const u64).read_unaligned(); t640[0] = vsetq_lane_u32::<0>(v as u32, t640[0]); t640[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t640[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t639[0]) as usize * 8) as *const u64).read_unaligned(); t640[0] = vsetq_lane_u32::<1>(v as u32, t640[0]); t640[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t640[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t639[0]) as usize * 8) as *const u64).read_unaligned(); t640[0] = vsetq_lane_u32::<2>(v as u32, t640[0]); t640[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t640[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t639[0]) as usize * 8) as *const u64).read_unaligned(); t640[0] = vsetq_lane_u32::<3>(v as u32, t640[0]); t640[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t640[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t639[1]) as usize * 8) as *const u64).read_unaligned(); t640[1] = vsetq_lane_u32::<0>(v as u32, t640[1]); t640[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t640[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t639[1]) as usize * 8) as *const u64).read_unaligned(); t640[1] = vsetq_lane_u32::<1>(v as u32, t640[1]); t640[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t640[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t639[1]) as usize * 8) as *const u64).read_unaligned(); t640[1] = vsetq_lane_u32::<2>(v as u32, t640[1]); t640[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t640[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t639[1]) as usize * 8) as *const u64).read_unaligned(); t640[1] = vsetq_lane_u32::<3>(v as u32, t640[1]); t640[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t640[3]); }
            let t641 = (arg3 as *const u8).add(((gid0.wrapping_add((17u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t641 as *mut u32).add(0), uint32x4x2_t(t640[0], t640[2]));
                vst2q_u32((t641 as *mut u32).add(8), uint32x4x2_t(t640[1], t640[3]));
            } else {
                let mut t642w = 0u64;
                let t642 = core::hint::black_box(&raw mut t642w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t641 as *mut u64).add(0), t642).write_unaligned(((vgetq_lane_u32::<0>(t640[2]) as u64) << 32 | vgetq_lane_u32::<0>(t640[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t641 as *mut u64).add(1), t642).write_unaligned(((vgetq_lane_u32::<1>(t640[2]) as u64) << 32 | vgetq_lane_u32::<1>(t640[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t641 as *mut u64).add(2), t642).write_unaligned(((vgetq_lane_u32::<2>(t640[2]) as u64) << 32 | vgetq_lane_u32::<2>(t640[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t641 as *mut u64).add(3), t642).write_unaligned(((vgetq_lane_u32::<3>(t640[2]) as u64) << 32 | vgetq_lane_u32::<3>(t640[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t641 as *mut u64).add(4), t642).write_unaligned(((vgetq_lane_u32::<0>(t640[3]) as u64) << 32 | vgetq_lane_u32::<0>(t640[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t641 as *mut u64).add(5), t642).write_unaligned(((vgetq_lane_u32::<1>(t640[3]) as u64) << 32 | vgetq_lane_u32::<1>(t640[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t641 as *mut u64).add(6), t642).write_unaligned(((vgetq_lane_u32::<2>(t640[3]) as u64) << 32 | vgetq_lane_u32::<2>(t640[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t641 as *mut u64).add(7), t642).write_unaligned(((vgetq_lane_u32::<3>(t640[3]) as u64) << 32 | vgetq_lane_u32::<3>(t640[1]) as u64));
            }
            let t643 = b3_ll_p(l, lid0, live, x6, x8);
            let t644 = (arg3 as *const u8).add(((gid0.wrapping_add((18u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t644 as *mut u32).add(0), uint32x4x2_t(t643[0], t643[2]));
                vst2q_u32((t644 as *mut u32).add(8), uint32x4x2_t(t643[1], t643[3]));
            } else {
                let mut t645w = 0u64;
                let t645 = core::hint::black_box(&raw mut t645w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t644 as *mut u64).add(0), t645).write_unaligned(((vgetq_lane_u32::<0>(t643[2]) as u64) << 32 | vgetq_lane_u32::<0>(t643[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t644 as *mut u64).add(1), t645).write_unaligned(((vgetq_lane_u32::<1>(t643[2]) as u64) << 32 | vgetq_lane_u32::<1>(t643[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t644 as *mut u64).add(2), t645).write_unaligned(((vgetq_lane_u32::<2>(t643[2]) as u64) << 32 | vgetq_lane_u32::<2>(t643[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t644 as *mut u64).add(3), t645).write_unaligned(((vgetq_lane_u32::<3>(t643[2]) as u64) << 32 | vgetq_lane_u32::<3>(t643[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t644 as *mut u64).add(4), t645).write_unaligned(((vgetq_lane_u32::<0>(t643[3]) as u64) << 32 | vgetq_lane_u32::<0>(t643[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t644 as *mut u64).add(5), t645).write_unaligned(((vgetq_lane_u32::<1>(t643[3]) as u64) << 32 | vgetq_lane_u32::<1>(t643[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t644 as *mut u64).add(6), t645).write_unaligned(((vgetq_lane_u32::<2>(t643[3]) as u64) << 32 | vgetq_lane_u32::<2>(t643[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t644 as *mut u64).add(7), t645).write_unaligned(((vgetq_lane_u32::<3>(t643[3]) as u64) << 32 | vgetq_lane_u32::<3>(t643[1]) as u64));
            }
            x12 = x6;
            x13 = 0u32;
            let mut t650 = live;
            loop {
                let t652 = bits(&t650);
                if t652.count_ones() <= 2 {
                    let mut t653 = t652;
                    while t653 != 0 {
                        let j = t653.trailing_zeros() as usize;
                        t653 &= t653 - 1;
                        let t654 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x7);
                        let mut x7_j: u64 = ((t654[8 + j] as u64) << 32 | t654[j] as u64);
                        let t655 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x8);
                        let mut x8_j: u32 = t655[j];
                        let t656 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x12);
                        let mut x12_j: u64 = ((t656[8 + j] as u64) << 32 | t656[j] as u64);
                        let mut x13_j: u32 = x13;
                        loop {
                            if !(((x13_j) < (((x8_j) & (7u32))))) { break; }
                            x12_j = ((x12_j).wrapping_mul(3u64)).wrapping_add(x7_j);
                            let t657 = x13_j;
                            x13_j = (x13_j).wrapping_add(1u32);
                        }
                        let mut t658 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x12);
                        t658[j] = x12_j as u32;
                        t658[8 + j] = (x12_j as u64 >> 32) as u32;
                        x12 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t658);
                    }
                    break;
                }
                // Lanes part: each its own rounds.
                let t659 = [vdupq_n_u32(7u32); 2];
                let t660: [uint32x4_t; 2] = [vandq_u32(x8[0], t659[0]), vandq_u32(x8[1], t659[1])];
                let t661 = [vdupq_n_u32(x13); 2];
                let t662: [uint32x4_t; 2] = [vcltq_u32(t661[0], t660[0]), vcltq_u32(t661[1], t660[1])];
                t650 = [vandq_u32(t650[0], t662[0]), vandq_u32(t650[1], t662[1])];
                if !(any(&t650)) { break; }
                let t664: u64 = 3u64;
                let t665 = [vdupq_n_u32((t664) as u32); 2];
                let t666: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x12[0]), vget_low_u32(t665[0]))), vreinterpretq_u32_u64(vmull_high_u32(x12[0], t665[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x12[1]), vget_low_u32(t665[1]))), vreinterpretq_u32_u64(vmull_high_u32(x12[1], t665[1])))];
                let t667: [uint32x4_t; 4] = [vmulq_u32(x12[0], t665[0]), vmulq_u32(x12[1], t665[1]), vmlaq_u32(t666[0], t665[0], x12[2]), vmlaq_u32(t666[1], t665[1], x12[3])];
                let t668: [uint32x4_t; 2] = [vaddq_u32(t667[0], x7[0]), vaddq_u32(t667[1], x7[1])];
                let t669: [uint32x4_t; 2] = [vcltq_u32(t668[0], t667[0]), vcltq_u32(t668[1], t667[1])];
                let t670: [uint32x4_t; 4] = [t668[0], t668[1], vsubq_u32(vaddq_u32(t667[2], x7[2]), t669[0]), vsubq_u32(vaddq_u32(t667[3], x7[3]), t669[1])];
                x12 = [vbslq_u32(t650[0], t670[0], x12[0]), vbslq_u32(t650[1], t670[1], x12[1]), vbslq_u32(t650[0], t670[2], x12[2]), vbslq_u32(t650[1], t670[3], x12[3])];
                let t671: [uint32x4_t; 2] = [vandq_u32(t650[0], live[0]), vandq_u32(t650[1], live[1])];
                let t672 = x13;
                x13 = (x13).wrapping_add(1u32);
            }
            let t673 = (arg3 as *const u8).add(((gid0.wrapping_add((19u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t673 as *mut u32).add(0), uint32x4x2_t(x12[0], x12[2]));
                vst2q_u32((t673 as *mut u32).add(8), uint32x4x2_t(x12[1], x12[3]));
            } else {
                let mut t674w = 0u64;
                let t674 = core::hint::black_box(&raw mut t674w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t673 as *mut u64).add(0), t674).write_unaligned(((vgetq_lane_u32::<0>(x12[2]) as u64) << 32 | vgetq_lane_u32::<0>(x12[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t673 as *mut u64).add(1), t674).write_unaligned(((vgetq_lane_u32::<1>(x12[2]) as u64) << 32 | vgetq_lane_u32::<1>(x12[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t673 as *mut u64).add(2), t674).write_unaligned(((vgetq_lane_u32::<2>(x12[2]) as u64) << 32 | vgetq_lane_u32::<2>(x12[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t673 as *mut u64).add(3), t674).write_unaligned(((vgetq_lane_u32::<3>(x12[2]) as u64) << 32 | vgetq_lane_u32::<3>(x12[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t673 as *mut u64).add(4), t674).write_unaligned(((vgetq_lane_u32::<0>(x12[3]) as u64) << 32 | vgetq_lane_u32::<0>(x12[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t673 as *mut u64).add(5), t674).write_unaligned(((vgetq_lane_u32::<1>(x12[3]) as u64) << 32 | vgetq_lane_u32::<1>(x12[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t673 as *mut u64).add(6), t674).write_unaligned(((vgetq_lane_u32::<2>(x12[3]) as u64) << 32 | vgetq_lane_u32::<2>(x12[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t673 as *mut u64).add(7), t674).write_unaligned(((vgetq_lane_u32::<3>(x12[3]) as u64) << 32 | vgetq_lane_u32::<3>(x12[1]) as u64));
            }
            let t675: u64 = a1;
            let t676 = [vdupq_n_u32((t675) as u32); 2];
            let t677 = [vdupq_n_u32(((t675) >> 32) as u32); 2];
            let t678: [uint32x4_t; 2] = [vaddq_u32(x6[0], t676[0]), vaddq_u32(x6[1], t676[1])];
            let t679: [uint32x4_t; 2] = [vcltq_u32(t678[0], x6[0]), vcltq_u32(t678[1], x6[1])];
            let t680: [uint32x4_t; 4] = [t678[0], t678[1], vsubq_u32(vaddq_u32(x6[2], t677[0]), t679[0]), vsubq_u32(vaddq_u32(x6[3], t677[1]), t679[1])];
            let t681: u64 = (a1).wrapping_shl((3u64) as u32);
            let t682 = [vdupq_n_u32((t681) as u32); 2];
            let t683 = [vdupq_n_u32(((t681) >> 32) as u32); 2];
            let t684: [uint32x4_t; 2] = [vaddq_u32(t680[0], t682[0]), vaddq_u32(t680[1], t682[1])];
            let t685: [uint32x4_t; 2] = [vcltq_u32(t684[0], t680[0]), vcltq_u32(t684[1], t680[1])];
            let t686: [uint32x4_t; 4] = [t684[0], t684[1], vsubq_u32(vaddq_u32(t680[2], t683[0]), t685[0]), vsubq_u32(vaddq_u32(t680[3], t683[1]), t685[1])];
            let t687 = (arg3 as *const u8).add(((gid0.wrapping_add((20u32).wrapping_mul(x10))) as usize) * 8);
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
            let t689: u64 = 4294967295u64;
            let t690 = [vdupq_n_u32((t689) as u32); 2];
            let t691: [uint32x4_t; 2] = [vandq_u32(x6[0], t690[0]), vandq_u32(x6[1], t690[1])];
            let t692: u64 = 4294967295u64;
            let t693 = [vdupq_n_u32((t692) as u32); 2];
            let t694: [uint32x4_t; 2] = [vandq_u32(x7[0], t693[0]), vandq_u32(x7[1], t693[1])];
            let t695: [uint32x4_t; 2] = [vceqq_u32(t691[0], t694[0]), vceqq_u32(t691[1], t694[1])];
            let mut t696: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t697: [uint32x4_t; 2] = [vandq_u32(live[0], t695[0]), vandq_u32(live[1], t695[1])];
            let t698: [uint32x4_t; 2] = [vbicq_u32(live[0], t695[0]), vbicq_u32(live[1], t695[1])];
            {
                let t699 = [vdupq_n_u32(1u32); 2];
                t696 = [vbslq_u32(t697[0], t699[0], t696[0]), vbslq_u32(t697[1], t699[1], t696[1])];
            }
            {
                let t700 = [vdupq_n_u32(2u32); 2];
                t696 = [vbslq_u32(t698[0], t700[0], t696[0]), vbslq_u32(t698[1], t700[1], t696[1])];
            }
            let t701: [uint32x4_t; 4] = [t696[0], t696[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            let t702 = (arg3 as *const u8).add(((gid0.wrapping_add((21u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t702 as *mut u32).add(0), uint32x4x2_t(t701[0], t701[2]));
                vst2q_u32((t702 as *mut u32).add(8), uint32x4x2_t(t701[1], t701[3]));
            } else {
                let mut t703w = 0u64;
                let t703 = core::hint::black_box(&raw mut t703w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t702 as *mut u64).add(0), t703).write_unaligned(((vgetq_lane_u32::<0>(t701[2]) as u64) << 32 | vgetq_lane_u32::<0>(t701[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t702 as *mut u64).add(1), t703).write_unaligned(((vgetq_lane_u32::<1>(t701[2]) as u64) << 32 | vgetq_lane_u32::<1>(t701[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t702 as *mut u64).add(2), t703).write_unaligned(((vgetq_lane_u32::<2>(t701[2]) as u64) << 32 | vgetq_lane_u32::<2>(t701[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t702 as *mut u64).add(3), t703).write_unaligned(((vgetq_lane_u32::<3>(t701[2]) as u64) << 32 | vgetq_lane_u32::<3>(t701[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t702 as *mut u64).add(4), t703).write_unaligned(((vgetq_lane_u32::<0>(t701[3]) as u64) << 32 | vgetq_lane_u32::<0>(t701[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t702 as *mut u64).add(5), t703).write_unaligned(((vgetq_lane_u32::<1>(t701[3]) as u64) << 32 | vgetq_lane_u32::<1>(t701[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t702 as *mut u64).add(6), t703).write_unaligned(((vgetq_lane_u32::<2>(t701[3]) as u64) << 32 | vgetq_lane_u32::<2>(t701[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t702 as *mut u64).add(7), t703).write_unaligned(((vgetq_lane_u32::<3>(t701[3]) as u64) << 32 | vgetq_lane_u32::<3>(t701[1]) as u64));
            }
            let t704: [uint32x4_t; 2] = [vtstq_u32(vorrq_u32(x6[0], x6[2]), vorrq_u32(x6[0], x6[2])), vtstq_u32(vorrq_u32(x6[1], x6[3]), vorrq_u32(x6[1], x6[3]))];
            let mut t705: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t706: [uint32x4_t; 2] = [vandq_u32(live[0], t704[0]), vandq_u32(live[1], t704[1])];
            let t707: [uint32x4_t; 2] = [vbicq_u32(live[0], t704[0]), vbicq_u32(live[1], t704[1])];
            {
                let t708 = [vdupq_n_u32(5u32); 2];
                t705 = [vbslq_u32(t706[0], t708[0], t705[0]), vbslq_u32(t706[1], t708[1], t705[1])];
            }
            {
                let t709 = [vdupq_n_u32(6u32); 2];
                t705 = [vbslq_u32(t707[0], t709[0], t705[0]), vbslq_u32(t707[1], t709[1], t705[1])];
            }
            let t710: [uint32x4_t; 4] = [t705[0], t705[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            let t711 = (arg3 as *const u8).add(((gid0.wrapping_add((22u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t711 as *mut u32).add(0), uint32x4x2_t(t710[0], t710[2]));
                vst2q_u32((t711 as *mut u32).add(8), uint32x4x2_t(t710[1], t710[3]));
            } else {
                let mut t712w = 0u64;
                let t712 = core::hint::black_box(&raw mut t712w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t711 as *mut u64).add(0), t712).write_unaligned(((vgetq_lane_u32::<0>(t710[2]) as u64) << 32 | vgetq_lane_u32::<0>(t710[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t711 as *mut u64).add(1), t712).write_unaligned(((vgetq_lane_u32::<1>(t710[2]) as u64) << 32 | vgetq_lane_u32::<1>(t710[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t711 as *mut u64).add(2), t712).write_unaligned(((vgetq_lane_u32::<2>(t710[2]) as u64) << 32 | vgetq_lane_u32::<2>(t710[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t711 as *mut u64).add(3), t712).write_unaligned(((vgetq_lane_u32::<3>(t710[2]) as u64) << 32 | vgetq_lane_u32::<3>(t710[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t711 as *mut u64).add(4), t712).write_unaligned(((vgetq_lane_u32::<0>(t710[3]) as u64) << 32 | vgetq_lane_u32::<0>(t710[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t711 as *mut u64).add(5), t712).write_unaligned(((vgetq_lane_u32::<1>(t710[3]) as u64) << 32 | vgetq_lane_u32::<1>(t710[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t711 as *mut u64).add(6), t712).write_unaligned(((vgetq_lane_u32::<2>(t710[3]) as u64) << 32 | vgetq_lane_u32::<2>(t710[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t711 as *mut u64).add(7), t712).write_unaligned(((vgetq_lane_u32::<3>(t710[3]) as u64) << 32 | vgetq_lane_u32::<3>(t710[1]) as u64));
            }
            let t713 = b4_ll_p(l, lid0, live, x6, x7);
            let t714 = (arg3 as *const u8).add(((gid0.wrapping_add((23u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t714 as *mut u32).add(0), uint32x4x2_t(t713[0], t713[2]));
                vst2q_u32((t714 as *mut u32).add(8), uint32x4x2_t(t713[1], t713[3]));
            } else {
                let mut t715w = 0u64;
                let t715 = core::hint::black_box(&raw mut t715w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t714 as *mut u64).add(0), t715).write_unaligned(((vgetq_lane_u32::<0>(t713[2]) as u64) << 32 | vgetq_lane_u32::<0>(t713[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t714 as *mut u64).add(1), t715).write_unaligned(((vgetq_lane_u32::<1>(t713[2]) as u64) << 32 | vgetq_lane_u32::<1>(t713[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t714 as *mut u64).add(2), t715).write_unaligned(((vgetq_lane_u32::<2>(t713[2]) as u64) << 32 | vgetq_lane_u32::<2>(t713[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t714 as *mut u64).add(3), t715).write_unaligned(((vgetq_lane_u32::<3>(t713[2]) as u64) << 32 | vgetq_lane_u32::<3>(t713[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t714 as *mut u64).add(4), t715).write_unaligned(((vgetq_lane_u32::<0>(t713[3]) as u64) << 32 | vgetq_lane_u32::<0>(t713[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t714 as *mut u64).add(5), t715).write_unaligned(((vgetq_lane_u32::<1>(t713[3]) as u64) << 32 | vgetq_lane_u32::<1>(t713[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t714 as *mut u64).add(6), t715).write_unaligned(((vgetq_lane_u32::<2>(t713[3]) as u64) << 32 | vgetq_lane_u32::<2>(t713[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t714 as *mut u64).add(7), t715).write_unaligned(((vgetq_lane_u32::<3>(t713[3]) as u64) << 32 | vgetq_lane_u32::<3>(t713[1]) as u64));
            }
            let t716: u32;
            if ((((a1) & (1u64))) != (0u64)) {
                t716 = 8u32;
            } else {
                t716 = 4u32;
            }
            x14 = t716;
            let t717 = b5_r0ns_p(l, lid0, live, arg0, gid0.wrapping_mul(x14), x14, 8u32);
            let t718 = (arg3 as *const u8).add(((gid0.wrapping_add((24u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t718 as *mut u32).add(0), uint32x4x2_t(t717[0], t717[2]));
                vst2q_u32((t718 as *mut u32).add(8), uint32x4x2_t(t717[1], t717[3]));
            } else {
                let mut t719w = 0u64;
                let t719 = core::hint::black_box(&raw mut t719w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t718 as *mut u64).add(0), t719).write_unaligned(((vgetq_lane_u32::<0>(t717[2]) as u64) << 32 | vgetq_lane_u32::<0>(t717[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t718 as *mut u64).add(1), t719).write_unaligned(((vgetq_lane_u32::<1>(t717[2]) as u64) << 32 | vgetq_lane_u32::<1>(t717[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t718 as *mut u64).add(2), t719).write_unaligned(((vgetq_lane_u32::<2>(t717[2]) as u64) << 32 | vgetq_lane_u32::<2>(t717[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t718 as *mut u64).add(3), t719).write_unaligned(((vgetq_lane_u32::<3>(t717[2]) as u64) << 32 | vgetq_lane_u32::<3>(t717[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t718 as *mut u64).add(4), t719).write_unaligned(((vgetq_lane_u32::<0>(t717[3]) as u64) << 32 | vgetq_lane_u32::<0>(t717[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t718 as *mut u64).add(5), t719).write_unaligned(((vgetq_lane_u32::<1>(t717[3]) as u64) << 32 | vgetq_lane_u32::<1>(t717[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t718 as *mut u64).add(6), t719).write_unaligned(((vgetq_lane_u32::<2>(t717[3]) as u64) << 32 | vgetq_lane_u32::<2>(t717[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t718 as *mut u64).add(7), t719).write_unaligned(((vgetq_lane_u32::<3>(t717[3]) as u64) << 32 | vgetq_lane_u32::<3>(t717[1]) as u64));
            }
            let t720 = if (gid0.wrapping_add(3u32) & 255u32) <= 248u32 { b6_u8_p(l, lid0, live, gid0.wrapping_add(3u32)) } else { b6_i_p(l, lid0, live, gid0.wrapping_add(3u32)) };
            let t721 = (arg3 as *const u8).add(((gid0.wrapping_add((25u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t721 as *mut u32).add(0), uint32x4x2_t(t720[0], t720[2]));
                vst2q_u32((t721 as *mut u32).add(8), uint32x4x2_t(t720[1], t720[3]));
            } else {
                let mut t722w = 0u64;
                let t722 = core::hint::black_box(&raw mut t722w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t721 as *mut u64).add(0), t722).write_unaligned(((vgetq_lane_u32::<0>(t720[2]) as u64) << 32 | vgetq_lane_u32::<0>(t720[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t721 as *mut u64).add(1), t722).write_unaligned(((vgetq_lane_u32::<1>(t720[2]) as u64) << 32 | vgetq_lane_u32::<1>(t720[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t721 as *mut u64).add(2), t722).write_unaligned(((vgetq_lane_u32::<2>(t720[2]) as u64) << 32 | vgetq_lane_u32::<2>(t720[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t721 as *mut u64).add(3), t722).write_unaligned(((vgetq_lane_u32::<3>(t720[2]) as u64) << 32 | vgetq_lane_u32::<3>(t720[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t721 as *mut u64).add(4), t722).write_unaligned(((vgetq_lane_u32::<0>(t720[3]) as u64) << 32 | vgetq_lane_u32::<0>(t720[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t721 as *mut u64).add(5), t722).write_unaligned(((vgetq_lane_u32::<1>(t720[3]) as u64) << 32 | vgetq_lane_u32::<1>(t720[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t721 as *mut u64).add(6), t722).write_unaligned(((vgetq_lane_u32::<2>(t720[3]) as u64) << 32 | vgetq_lane_u32::<2>(t720[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t721 as *mut u64).add(7), t722).write_unaligned(((vgetq_lane_u32::<3>(t720[3]) as u64) << 32 | vgetq_lane_u32::<3>(t720[1]) as u64));
            }
            let t723: u64 = 3u64;
            let t724 = [vdupq_n_u32((t723) as u32); 2];
            let t725: [uint32x4_t; 2] = [vandq_u32(x6[0], t724[0]), vandq_u32(x6[1], t724[1])];
            let t726: u64 = 0u64;
            let t727 = [vdupq_n_u32((t726) as u32); 2];
            let t728: [uint32x4_t; 2] = [vceqq_u32(t725[0], t727[0]), vceqq_u32(t725[1], t727[1])];
            let mut t729: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            let t730: [uint32x4_t; 2] = [vandq_u32(live[0], t728[0]), vandq_u32(live[1], t728[1])];
            let t731: [uint32x4_t; 2] = [vbicq_u32(live[0], t728[0]), vbicq_u32(live[1], t728[1])];
            if any(&t730) {
                let t732: [uint32x4_t; 4];
                let t733 = bits(&t730);
                if t733.count_ones() <= 2 {
                    let t734 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x7);
                    let t735 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x8);
                    let mut t736 = [0u32; 16];
                    let mut t737 = t733;
                    while t737 != 0 {
                        let j = t737.trailing_zeros() as usize;
                        t737 &= t737 - 1;
                        let t738 = b7_ss_f(l, lid0, ((t734[8 + j] as u64) << 32 | t734[j] as u64), t735[j]);
                        let t739: u64 = t738;
                        t736[j] = t739 as u32;
                        t736[8 + j] = (t739 >> 32) as u32;
                    }
                    t732 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t736);
                } else {
                    let t740 = b7_ll_p(l, lid0, t730, x7, x8);
                    t732 = t740;
                }
                t729 = [vbslq_u32(t730[0], t732[0], t729[0]), vbslq_u32(t730[1], t732[1], t729[1]), vbslq_u32(t730[0], t732[2], t729[2]), vbslq_u32(t730[1], t732[3], t729[3])];
            }
            {
                t729 = [vbslq_u32(t731[0], x6[0], t729[0]), vbslq_u32(t731[1], x6[1], t729[1]), vbslq_u32(t731[0], x6[2], t729[2]), vbslq_u32(t731[1], x6[3], t729[3])];
            }
            let t741 = (arg3 as *const u8).add(((gid0.wrapping_add((26u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t741 as *mut u32).add(0), uint32x4x2_t(t729[0], t729[2]));
                vst2q_u32((t741 as *mut u32).add(8), uint32x4x2_t(t729[1], t729[3]));
            } else {
                let mut t742w = 0u64;
                let t742 = core::hint::black_box(&raw mut t742w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t741 as *mut u64).add(0), t742).write_unaligned(((vgetq_lane_u32::<0>(t729[2]) as u64) << 32 | vgetq_lane_u32::<0>(t729[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t741 as *mut u64).add(1), t742).write_unaligned(((vgetq_lane_u32::<1>(t729[2]) as u64) << 32 | vgetq_lane_u32::<1>(t729[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t741 as *mut u64).add(2), t742).write_unaligned(((vgetq_lane_u32::<2>(t729[2]) as u64) << 32 | vgetq_lane_u32::<2>(t729[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t741 as *mut u64).add(3), t742).write_unaligned(((vgetq_lane_u32::<3>(t729[2]) as u64) << 32 | vgetq_lane_u32::<3>(t729[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t741 as *mut u64).add(4), t742).write_unaligned(((vgetq_lane_u32::<0>(t729[3]) as u64) << 32 | vgetq_lane_u32::<0>(t729[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t741 as *mut u64).add(5), t742).write_unaligned(((vgetq_lane_u32::<1>(t729[3]) as u64) << 32 | vgetq_lane_u32::<1>(t729[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t741 as *mut u64).add(6), t742).write_unaligned(((vgetq_lane_u32::<2>(t729[3]) as u64) << 32 | vgetq_lane_u32::<2>(t729[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t741 as *mut u64).add(7), t742).write_unaligned(((vgetq_lane_u32::<3>(t729[3]) as u64) << 32 | vgetq_lane_u32::<3>(t729[1]) as u64));
            }
            x15 = (((((a1) & (7u64))) as u32)).wrapping_add(1u32);
            let t743 = b5_r0ns_p(l, lid0, live, arg0, gid0.wrapping_mul(x15), x15, x15);
            let t744 = b8_s_p(l, lid0, live, (8u32).wrapping_mul(x15));
            let t745: u64 = t744;
            let t746 = [vdupq_n_u32((t745) as u32); 2];
            let t747 = [vdupq_n_u32(((t745) >> 32) as u32); 2];
            let t748: [uint32x4_t; 4] = [vandq_u32(t743[0], t746[0]), vandq_u32(t743[1], t746[1]), vandq_u32(t743[2], t747[0]), vandq_u32(t743[3], t747[1])];
            let t749 = (arg3 as *const u8).add(((gid0.wrapping_add((27u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t749 as *mut u32).add(0), uint32x4x2_t(t748[0], t748[2]));
                vst2q_u32((t749 as *mut u32).add(8), uint32x4x2_t(t748[1], t748[3]));
            } else {
                let mut t750w = 0u64;
                let t750 = core::hint::black_box(&raw mut t750w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t749 as *mut u64).add(0), t750).write_unaligned(((vgetq_lane_u32::<0>(t748[2]) as u64) << 32 | vgetq_lane_u32::<0>(t748[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t749 as *mut u64).add(1), t750).write_unaligned(((vgetq_lane_u32::<1>(t748[2]) as u64) << 32 | vgetq_lane_u32::<1>(t748[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t749 as *mut u64).add(2), t750).write_unaligned(((vgetq_lane_u32::<2>(t748[2]) as u64) << 32 | vgetq_lane_u32::<2>(t748[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t749 as *mut u64).add(3), t750).write_unaligned(((vgetq_lane_u32::<3>(t748[2]) as u64) << 32 | vgetq_lane_u32::<3>(t748[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t749 as *mut u64).add(4), t750).write_unaligned(((vgetq_lane_u32::<0>(t748[3]) as u64) << 32 | vgetq_lane_u32::<0>(t748[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t749 as *mut u64).add(5), t750).write_unaligned(((vgetq_lane_u32::<1>(t748[3]) as u64) << 32 | vgetq_lane_u32::<1>(t748[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t749 as *mut u64).add(6), t750).write_unaligned(((vgetq_lane_u32::<2>(t748[3]) as u64) << 32 | vgetq_lane_u32::<2>(t748[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t749 as *mut u64).add(7), t750).write_unaligned(((vgetq_lane_u32::<3>(t748[3]) as u64) << 32 | vgetq_lane_u32::<3>(t748[1]) as u64));
            }
        }
    }
}

#[inline(always)]
unsafe fn b1_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t751: [uint32x4_t; 2] = [vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[0])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[2]))))), vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[1])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[3])))))];
    res = t751;
    return res;
}

#[inline(always)]
unsafe fn b1_l_f(l: &Launch, lid0: u32, x0: [uint32x4_t; 4]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t752: [uint32x4_t; 2] = [vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[0])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[2]))))), vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[1])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[3])))))];
    res = t752;
    return res;
}

#[inline(always)]
unsafe fn b2_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t753: [uint32x4_t; 2] = [vbslq_u32(vceqzq_u32(x0[2]), vaddq_u32(vclzq_u32(x0[0]), vdupq_n_u32(32)), vclzq_u32(x0[2])), vbslq_u32(vceqzq_u32(x0[3]), vaddq_u32(vclzq_u32(x0[1]), vdupq_n_u32(32)), vclzq_u32(x0[3]))];
    res = t753;
    return res;
}

#[inline(always)]
unsafe fn b2_l_f(l: &Launch, lid0: u32, x0: [uint32x4_t; 4]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t754: [uint32x4_t; 2] = [vbslq_u32(vceqzq_u32(x0[2]), vaddq_u32(vclzq_u32(x0[0]), vdupq_n_u32(32)), vclzq_u32(x0[2])), vbslq_u32(vceqzq_u32(x0[3]), vaddq_u32(vclzq_u32(x0[1]), vdupq_n_u32(32)), vclzq_u32(x0[3]))];
    res = t754;
    return res;
}

#[inline(always)]
unsafe fn b3_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], x1: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x2: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t755: u64 = 1u64;
    let t756 = [vdupq_n_u32((t755) as u32); 2];
    let t757: [uint32x4_t; 2] = [vandq_u32(x0[0], t756[0]), vandq_u32(x0[1], t756[1])];
    let t758: u64 = 0u64;
    let t759 = [vdupq_n_u32((t758) as u32); 2];
    let t760: [uint32x4_t; 2] = [vceqq_u32(t757[0], t759[0]), vceqq_u32(t757[1], t759[1])];
    let t761: [uint32x4_t; 2] = [vandq_u32(live[0], t760[0]), vandq_u32(live[1], t760[1])];
    {
        let t762: [uint32x4_t; 4] = [x1[0], x1[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = [vbslq_u32(t761[0], t762[0], res[0]), vbslq_u32(t761[1], t762[1], res[1]), vbslq_u32(t761[0], t762[2], res[2]), vbslq_u32(t761[1], t762[3], res[3])];
        live = [vbicq_u32(live[0], t761[0]), vbicq_u32(live[1], t761[1])];
    }
    if any(&live) {
        let t763: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x0[0]), vshlq_n_u32::<30>(x0[2])), vorrq_u32(vshrq_n_u32::<2>(x0[1]), vshlq_n_u32::<30>(x0[3])), vshrq_n_u32::<2>(x0[2]), vshrq_n_u32::<2>(x0[3])];
        let t764: [uint32x4_t; 2] = [t763[0], t763[1]];
        x2 = t764;
        let t765 = [vdupq_n_u32(1u32); 2];
        let t766: [uint32x4_t; 2] = [vsubq_u32(x2[0], t765[0]), vsubq_u32(x2[1], t765[1])];
        let t767: [uint32x4_t; 2] = [vcgeq_u32(t766[0], x1[0]), vcgeq_u32(t766[1], x1[1])];
        let t768: [uint32x4_t; 2] = [vandq_u32(live[0], t767[0]), vandq_u32(live[1], t767[1])];
        {
            let t769: u64 = 4294967295u64;
            let t770 = [vdupq_n_u32((t769) as u32); 2];
            let t771: [uint32x4_t; 4] = [t770[0], t770[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t768[0], t771[0], res[0]), vbslq_u32(t768[1], t771[1], res[1]), vbslq_u32(t768[0], t771[2], res[2]), vbslq_u32(t768[1], t771[3], res[3])];
            live = [vbicq_u32(live[0], t768[0]), vbicq_u32(live[1], t768[1])];
        }
        if any(&live) {
            let t772: u64 = 2u64;
            let t773 = [vdupq_n_u32((t772) as u32); 2];
            let t774: [uint32x4_t; 2] = [vandq_u32(x0[0], t773[0]), vandq_u32(x0[1], t773[1])];
            let t775: u64 = 0u64;
            let t776 = [vdupq_n_u32((t775) as u32); 2];
            let t777: [uint32x4_t; 2] = [vceqq_u32(t774[0], t776[0]), vceqq_u32(t774[1], t776[1])];
            let t778: [uint32x4_t; 2] = [vmvnq_u32(t777[0]), vmvnq_u32(t777[1])];
            let t779: [uint32x4_t; 2] = [vandq_u32(live[0], t778[0]), vandq_u32(live[1], t778[1])];
            {
                let t780: [uint32x4_t; 2] = [vsubq_u32(x1[0], x2[0]), vsubq_u32(x1[1], x2[1])];
                let t781: [uint32x4_t; 4] = [t780[0], t780[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t779[0], t781[0], res[0]), vbslq_u32(t779[1], t781[1], res[1]), vbslq_u32(t779[0], t781[2], res[2]), vbslq_u32(t779[1], t781[3], res[3])];
                live = [vbicq_u32(live[0], t779[0]), vbicq_u32(live[1], t779[1])];
            }
            if any(&live) {
                let t782: [uint32x4_t; 2] = [vsubq_u32(x1[0], x2[0]), vsubq_u32(x1[1], x2[1])];
                let t783: u64 = (1u64).wrapping_shl((32u64) as u32);
                let t784 = [vdupq_n_u32((t783) as u32); 2];
                let t785 = [vdupq_n_u32(((t783) >> 32) as u32); 2];
                let t786: [uint32x4_t; 4] = [vorrq_u32(t784[0], t782[0]), vorrq_u32(t784[1], t782[1]), t785[0], t785[1]];
                res = [vbslq_u32(live[0], t786[0], res[0]), vbslq_u32(live[1], t786[1], res[1]), vbslq_u32(live[0], t786[2], res[2]), vbslq_u32(live[1], t786[3], res[3])];
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
    let t787: u64 = 1u64;
    let t788 = [vdupq_n_u32((t787) as u32); 2];
    let t789: [uint32x4_t; 2] = [vandq_u32(x0[0], t788[0]), vandq_u32(x0[1], t788[1])];
    let t790: u64 = 0u64;
    let t791 = [vdupq_n_u32((t790) as u32); 2];
    let t792: [uint32x4_t; 2] = [vceqq_u32(t789[0], t791[0]), vceqq_u32(t789[1], t791[1])];
    {
        let t793: [uint32x4_t; 4] = [x1[0], x1[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = [vbslq_u32(t792[0], t793[0], res[0]), vbslq_u32(t792[1], t793[1], res[1]), vbslq_u32(t792[0], t793[2], res[2]), vbslq_u32(t792[1], t793[3], res[3])];
        live = [vbicq_u32(live[0], t792[0]), vbicq_u32(live[1], t792[1])];
    }
    if any(&live) {
        let t794: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x0[0]), vshlq_n_u32::<30>(x0[2])), vorrq_u32(vshrq_n_u32::<2>(x0[1]), vshlq_n_u32::<30>(x0[3])), vshrq_n_u32::<2>(x0[2]), vshrq_n_u32::<2>(x0[3])];
        let t795: [uint32x4_t; 2] = [t794[0], t794[1]];
        x2 = t795;
        let t796 = [vdupq_n_u32(1u32); 2];
        let t797: [uint32x4_t; 2] = [vsubq_u32(x2[0], t796[0]), vsubq_u32(x2[1], t796[1])];
        let t798: [uint32x4_t; 2] = [vcgeq_u32(t797[0], x1[0]), vcgeq_u32(t797[1], x1[1])];
        let t799: [uint32x4_t; 2] = [vandq_u32(live[0], t798[0]), vandq_u32(live[1], t798[1])];
        {
            let t800: u64 = 4294967295u64;
            let t801 = [vdupq_n_u32((t800) as u32); 2];
            let t802: [uint32x4_t; 4] = [t801[0], t801[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t799[0], t802[0], res[0]), vbslq_u32(t799[1], t802[1], res[1]), vbslq_u32(t799[0], t802[2], res[2]), vbslq_u32(t799[1], t802[3], res[3])];
            live = [vbicq_u32(live[0], t799[0]), vbicq_u32(live[1], t799[1])];
        }
        if any(&live) {
            let t803: u64 = 2u64;
            let t804 = [vdupq_n_u32((t803) as u32); 2];
            let t805: [uint32x4_t; 2] = [vandq_u32(x0[0], t804[0]), vandq_u32(x0[1], t804[1])];
            let t806: u64 = 0u64;
            let t807 = [vdupq_n_u32((t806) as u32); 2];
            let t808: [uint32x4_t; 2] = [vceqq_u32(t805[0], t807[0]), vceqq_u32(t805[1], t807[1])];
            let t809: [uint32x4_t; 2] = [vmvnq_u32(t808[0]), vmvnq_u32(t808[1])];
            let t810: [uint32x4_t; 2] = [vandq_u32(live[0], t809[0]), vandq_u32(live[1], t809[1])];
            {
                let t811: [uint32x4_t; 2] = [vsubq_u32(x1[0], x2[0]), vsubq_u32(x1[1], x2[1])];
                let t812: [uint32x4_t; 4] = [t811[0], t811[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t810[0], t812[0], res[0]), vbslq_u32(t810[1], t812[1], res[1]), vbslq_u32(t810[0], t812[2], res[2]), vbslq_u32(t810[1], t812[3], res[3])];
                live = [vbicq_u32(live[0], t810[0]), vbicq_u32(live[1], t810[1])];
            }
            if any(&live) {
                let t813: [uint32x4_t; 2] = [vsubq_u32(x1[0], x2[0]), vsubq_u32(x1[1], x2[1])];
                let t814: u64 = (1u64).wrapping_shl((32u64) as u32);
                let t815 = [vdupq_n_u32((t814) as u32); 2];
                let t816 = [vdupq_n_u32(((t814) >> 32) as u32); 2];
                let t817: [uint32x4_t; 4] = [vorrq_u32(t815[0], t813[0]), vorrq_u32(t815[1], t813[1]), t816[0], t816[1]];
                res = [vbslq_u32(live[0], t817[0], res[0]), vbslq_u32(live[1], t817[1], res[1]), vbslq_u32(live[0], t817[2], res[2]), vbslq_u32(live[1], t817[3], res[3])];
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
    let (mut t818l, mut t818h) = ([0u32; 8], [0u32; 8]);
    { let v = (((((vgetq_lane_u32::<0>(x0[2]) as u64) << 32 | vgetq_lane_u32::<0>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<0>(x1[2]) as u64) << 32 | vgetq_lane_u32::<0>(x1[0]) as u64) as u64) as u128) >> 64) as u64; t818l[0] = v as u32; t818h[0] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<1>(x0[2]) as u64) << 32 | vgetq_lane_u32::<1>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<1>(x1[2]) as u64) << 32 | vgetq_lane_u32::<1>(x1[0]) as u64) as u64) as u128) >> 64) as u64; t818l[1] = v as u32; t818h[1] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<2>(x0[2]) as u64) << 32 | vgetq_lane_u32::<2>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<2>(x1[2]) as u64) << 32 | vgetq_lane_u32::<2>(x1[0]) as u64) as u64) as u128) >> 64) as u64; t818l[2] = v as u32; t818h[2] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<3>(x0[2]) as u64) << 32 | vgetq_lane_u32::<3>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<3>(x1[2]) as u64) << 32 | vgetq_lane_u32::<3>(x1[0]) as u64) as u64) as u128) >> 64) as u64; t818l[3] = v as u32; t818h[3] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<0>(x0[3]) as u64) << 32 | vgetq_lane_u32::<0>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<0>(x1[3]) as u64) << 32 | vgetq_lane_u32::<0>(x1[1]) as u64) as u64) as u128) >> 64) as u64; t818l[4] = v as u32; t818h[4] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<1>(x0[3]) as u64) << 32 | vgetq_lane_u32::<1>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<1>(x1[3]) as u64) << 32 | vgetq_lane_u32::<1>(x1[1]) as u64) as u64) as u128) >> 64) as u64; t818l[5] = v as u32; t818h[5] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<2>(x0[3]) as u64) << 32 | vgetq_lane_u32::<2>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<2>(x1[3]) as u64) << 32 | vgetq_lane_u32::<2>(x1[1]) as u64) as u64) as u128) >> 64) as u64; t818l[6] = v as u32; t818h[6] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<3>(x0[3]) as u64) << 32 | vgetq_lane_u32::<3>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<3>(x1[3]) as u64) << 32 | vgetq_lane_u32::<3>(x1[1]) as u64) as u64) as u128) >> 64) as u64; t818l[7] = v as u32; t818h[7] = (v >> 32) as u32; }
    let t819: [uint32x4_t; 4] = [vld1q_u32(t818l.as_ptr().add(0)), vld1q_u32(t818l.as_ptr().add(4)), vld1q_u32(t818h.as_ptr().add(0)), vld1q_u32(t818h.as_ptr().add(4))];
    res = t819;
    return res;
}

#[inline(always)]
unsafe fn b4_ll_f(l: &Launch, lid0: u32, x0: [uint32x4_t; 4], x1: [uint32x4_t; 4]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let (mut t820l, mut t820h) = ([0u32; 8], [0u32; 8]);
    { let v = (((((vgetq_lane_u32::<0>(x0[2]) as u64) << 32 | vgetq_lane_u32::<0>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<0>(x1[2]) as u64) << 32 | vgetq_lane_u32::<0>(x1[0]) as u64) as u64) as u128) >> 64) as u64; t820l[0] = v as u32; t820h[0] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<1>(x0[2]) as u64) << 32 | vgetq_lane_u32::<1>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<1>(x1[2]) as u64) << 32 | vgetq_lane_u32::<1>(x1[0]) as u64) as u64) as u128) >> 64) as u64; t820l[1] = v as u32; t820h[1] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<2>(x0[2]) as u64) << 32 | vgetq_lane_u32::<2>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<2>(x1[2]) as u64) << 32 | vgetq_lane_u32::<2>(x1[0]) as u64) as u64) as u128) >> 64) as u64; t820l[2] = v as u32; t820h[2] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<3>(x0[2]) as u64) << 32 | vgetq_lane_u32::<3>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<3>(x1[2]) as u64) << 32 | vgetq_lane_u32::<3>(x1[0]) as u64) as u64) as u128) >> 64) as u64; t820l[3] = v as u32; t820h[3] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<0>(x0[3]) as u64) << 32 | vgetq_lane_u32::<0>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<0>(x1[3]) as u64) << 32 | vgetq_lane_u32::<0>(x1[1]) as u64) as u64) as u128) >> 64) as u64; t820l[4] = v as u32; t820h[4] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<1>(x0[3]) as u64) << 32 | vgetq_lane_u32::<1>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<1>(x1[3]) as u64) << 32 | vgetq_lane_u32::<1>(x1[1]) as u64) as u64) as u128) >> 64) as u64; t820l[5] = v as u32; t820h[5] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<2>(x0[3]) as u64) << 32 | vgetq_lane_u32::<2>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<2>(x1[3]) as u64) << 32 | vgetq_lane_u32::<2>(x1[1]) as u64) as u64) as u128) >> 64) as u64; t820l[6] = v as u32; t820h[6] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<3>(x0[3]) as u64) << 32 | vgetq_lane_u32::<3>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<3>(x1[3]) as u64) << 32 | vgetq_lane_u32::<3>(x1[1]) as u64) as u64) as u128) >> 64) as u64; t820l[7] = v as u32; t820h[7] = (v >> 32) as u32; }
    let t821: [uint32x4_t; 4] = [vld1q_u32(t820l.as_ptr().add(0)), vld1q_u32(t820l.as_ptr().add(4)), vld1q_u32(t820h.as_ptr().add(0)), vld1q_u32(t820h.as_ptr().add(4))];
    res = t821;
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
        let t822: [uint32x4_t; 4];
        if (x1s) == 8u32 && ((x1f) as u64) + 7 * ((x1s) as u64) <= u32::MAX as u64 && all(&live) {
            let t823 = (arg0 as *const u8).add(((x1f) as usize) * 1);
            let t8240 = vld2q_u32((t823 as *const u32).add(0));
            let t8241 = vld2q_u32((t823 as *const u32).add(8));
            let t824: [uint32x4_t; 4] = [t8240.0, t8241.0, t8240.1, t8241.1];
            t822 = t824;
        } else {
            let t825: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1s)), vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1s))];
            let t826 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t825[0]), vandq_u32(live[1], t825[1]))));
            let t827: [uint32x4_t; 2] = [vbslq_u32(live[0], t825[0], t826), vbslq_u32(live[1], t825[1], t826)];
            let mut t828: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t827[0]) as usize * 1) as *const u64).read_unaligned(); t828[0] = vsetq_lane_u32::<0>(v as u32, t828[0]); t828[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t828[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t827[0]) as usize * 1) as *const u64).read_unaligned(); t828[0] = vsetq_lane_u32::<1>(v as u32, t828[0]); t828[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t828[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t827[0]) as usize * 1) as *const u64).read_unaligned(); t828[0] = vsetq_lane_u32::<2>(v as u32, t828[0]); t828[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t828[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t827[0]) as usize * 1) as *const u64).read_unaligned(); t828[0] = vsetq_lane_u32::<3>(v as u32, t828[0]); t828[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t828[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t827[1]) as usize * 1) as *const u64).read_unaligned(); t828[1] = vsetq_lane_u32::<0>(v as u32, t828[1]); t828[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t828[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t827[1]) as usize * 1) as *const u64).read_unaligned(); t828[1] = vsetq_lane_u32::<1>(v as u32, t828[1]); t828[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t828[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t827[1]) as usize * 1) as *const u64).read_unaligned(); t828[1] = vsetq_lane_u32::<2>(v as u32, t828[1]); t828[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t828[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t827[1]) as usize * 1) as *const u64).read_unaligned(); t828[1] = vsetq_lane_u32::<3>(v as u32, t828[1]); t828[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t828[3]); }
            t822 = t828;
        }
        res = t822;
        return res;
    }
    if ((x2) == (4u32)) {
        let t829: [uint32x4_t; 2];
        if (x1s) == 4u32 && ((x1f) as u64) + 7 * ((x1s) as u64) <= u32::MAX as u64 && all(&live) {
            let t830 = (arg0 as *const u8).add(((x1f) as usize) * 1);
            let t831: [uint32x4_t; 2] = core::array::from_fn(|r| vld1q_u32((t830 as *const u32).add(4 * r)));
            t829 = t831;
        } else {
            let t832: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1s)), vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1s))];
            let t833 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t832[0]), vandq_u32(live[1], t832[1]))));
            let t834: [uint32x4_t; 2] = [vbslq_u32(live[0], t832[0], t833), vbslq_u32(live[1], t832[1], t833)];
            let mut t835: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            t835[0] = vsetq_lane_u32::<0>((((arg0 as *const u8).add(vgetq_lane_u32::<0>(t834[0]) as usize * 1) as *const u32).read_unaligned() as u32), t835[0]);
            t835[0] = vsetq_lane_u32::<1>((((arg0 as *const u8).add(vgetq_lane_u32::<1>(t834[0]) as usize * 1) as *const u32).read_unaligned() as u32), t835[0]);
            t835[0] = vsetq_lane_u32::<2>((((arg0 as *const u8).add(vgetq_lane_u32::<2>(t834[0]) as usize * 1) as *const u32).read_unaligned() as u32), t835[0]);
            t835[0] = vsetq_lane_u32::<3>((((arg0 as *const u8).add(vgetq_lane_u32::<3>(t834[0]) as usize * 1) as *const u32).read_unaligned() as u32), t835[0]);
            t835[1] = vsetq_lane_u32::<0>((((arg0 as *const u8).add(vgetq_lane_u32::<0>(t834[1]) as usize * 1) as *const u32).read_unaligned() as u32), t835[1]);
            t835[1] = vsetq_lane_u32::<1>((((arg0 as *const u8).add(vgetq_lane_u32::<1>(t834[1]) as usize * 1) as *const u32).read_unaligned() as u32), t835[1]);
            t835[1] = vsetq_lane_u32::<2>((((arg0 as *const u8).add(vgetq_lane_u32::<2>(t834[1]) as usize * 1) as *const u32).read_unaligned() as u32), t835[1]);
            t835[1] = vsetq_lane_u32::<3>((((arg0 as *const u8).add(vgetq_lane_u32::<3>(t834[1]) as usize * 1) as *const u32).read_unaligned() as u32), t835[1]);
            t829 = t835;
        }
        let t836: [uint32x4_t; 4] = [t829[0], t829[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t836;
        return res;
    }
    if ((x2) == (2u32)) {
        let t837: [uint32x4_t; 2];
        if (x1s) == 2u32 && ((x1f) as u64) + 7 * ((x1s) as u64) <= u32::MAX as u64 && all(&live) {
            let t838 = (arg0 as *const u8).add(((x1f) as usize) * 1);
            let t839: [uint32x4_t; 2] = core::array::from_fn(|r| vmovl_u16(vld1_u16((t838 as *const u16).add(4 * r))));
            t837 = t839;
        } else {
            let t840: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1s)), vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1s))];
            let t841 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t840[0]), vandq_u32(live[1], t840[1]))));
            let t842: [uint32x4_t; 2] = [vbslq_u32(live[0], t840[0], t841), vbslq_u32(live[1], t840[1], t841)];
            let mut t843: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            t843[0] = vsetq_lane_u32::<0>((((arg0 as *const u8).add(vgetq_lane_u32::<0>(t842[0]) as usize * 1) as *const u16).read_unaligned() as u32), t843[0]);
            t843[0] = vsetq_lane_u32::<1>((((arg0 as *const u8).add(vgetq_lane_u32::<1>(t842[0]) as usize * 1) as *const u16).read_unaligned() as u32), t843[0]);
            t843[0] = vsetq_lane_u32::<2>((((arg0 as *const u8).add(vgetq_lane_u32::<2>(t842[0]) as usize * 1) as *const u16).read_unaligned() as u32), t843[0]);
            t843[0] = vsetq_lane_u32::<3>((((arg0 as *const u8).add(vgetq_lane_u32::<3>(t842[0]) as usize * 1) as *const u16).read_unaligned() as u32), t843[0]);
            t843[1] = vsetq_lane_u32::<0>((((arg0 as *const u8).add(vgetq_lane_u32::<0>(t842[1]) as usize * 1) as *const u16).read_unaligned() as u32), t843[1]);
            t843[1] = vsetq_lane_u32::<1>((((arg0 as *const u8).add(vgetq_lane_u32::<1>(t842[1]) as usize * 1) as *const u16).read_unaligned() as u32), t843[1]);
            t843[1] = vsetq_lane_u32::<2>((((arg0 as *const u8).add(vgetq_lane_u32::<2>(t842[1]) as usize * 1) as *const u16).read_unaligned() as u32), t843[1]);
            t843[1] = vsetq_lane_u32::<3>((((arg0 as *const u8).add(vgetq_lane_u32::<3>(t842[1]) as usize * 1) as *const u16).read_unaligned() as u32), t843[1]);
            t837 = t843;
        }
        let t844: [uint32x4_t; 4] = [t837[0], t837[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t844;
        return res;
    }
    if ((x2) == (1u32)) {
        let t845: [uint32x4_t; 2];
        if (x1s) == 1u32 && ((x1f) as u64) + 7 * ((x1s) as u64) <= u32::MAX as u64 && all(&live) {
            let t846 = (arg0 as *const u8).add(((x1f) as usize) * 1);
            let t848 = vmovl_u8(vld1_u8(t846.add(0)));
            let t847: [uint32x4_t; 2] = [vmovl_u16(vget_low_u16(t848)), vmovl_u16(vget_high_u16(t848))];
            t845 = t847;
        } else {
            let t849: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1s)), vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1s))];
            let t850 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t849[0]), vandq_u32(live[1], t849[1]))));
            let t851: [uint32x4_t; 2] = [vbslq_u32(live[0], t849[0], t850), vbslq_u32(live[1], t849[1], t850)];
            let mut t852: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            t852[0] = vsetq_lane_u32::<0>((((arg0 as *const u8).add(vgetq_lane_u32::<0>(t851[0]) as usize * 1) as *const u8).read_unaligned() as u32), t852[0]);
            t852[0] = vsetq_lane_u32::<1>((((arg0 as *const u8).add(vgetq_lane_u32::<1>(t851[0]) as usize * 1) as *const u8).read_unaligned() as u32), t852[0]);
            t852[0] = vsetq_lane_u32::<2>((((arg0 as *const u8).add(vgetq_lane_u32::<2>(t851[0]) as usize * 1) as *const u8).read_unaligned() as u32), t852[0]);
            t852[0] = vsetq_lane_u32::<3>((((arg0 as *const u8).add(vgetq_lane_u32::<3>(t851[0]) as usize * 1) as *const u8).read_unaligned() as u32), t852[0]);
            t852[1] = vsetq_lane_u32::<0>((((arg0 as *const u8).add(vgetq_lane_u32::<0>(t851[1]) as usize * 1) as *const u8).read_unaligned() as u32), t852[1]);
            t852[1] = vsetq_lane_u32::<1>((((arg0 as *const u8).add(vgetq_lane_u32::<1>(t851[1]) as usize * 1) as *const u8).read_unaligned() as u32), t852[1]);
            t852[1] = vsetq_lane_u32::<2>((((arg0 as *const u8).add(vgetq_lane_u32::<2>(t851[1]) as usize * 1) as *const u8).read_unaligned() as u32), t852[1]);
            t852[1] = vsetq_lane_u32::<3>((((arg0 as *const u8).add(vgetq_lane_u32::<3>(t851[1]) as usize * 1) as *const u8).read_unaligned() as u32), t852[1]);
            t845 = t852;
        }
        let t853: [uint32x4_t; 4] = [t845[0], t845[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t853;
        return res;
    }
    let t854: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1s)), vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1s))];
    let t855 = [vdupq_n_u32(!(7u32)); 2];
    let t856: [uint32x4_t; 2] = [vandq_u32(t854[0], t855[0]), vandq_u32(t854[1], t855[1])];
    let t857: [uint32x4_t; 2] = [t856[0], t856[1]];
    let t858: u64 = ((arg0) as usize as u64);
    let t859 = [vdupq_n_u32((t858) as u32); 2];
    let t860 = [vdupq_n_u32(((t858) >> 32) as u32); 2];
    let t861: [uint32x4_t; 2] = [vaddq_u32(t859[0], t857[0]), vaddq_u32(t859[1], t857[1])];
    let t862: [uint32x4_t; 2] = [vcltq_u32(t861[0], t859[0]), vcltq_u32(t861[1], t859[1])];
    let t863: [uint32x4_t; 4] = [t861[0], t861[1], vsubq_u32(vaddq_u32(t860[0], vdupq_n_u32(0)), t862[0]), vsubq_u32(vaddq_u32(t860[1], vdupq_n_u32(0)), t862[1])];
    let t864: [uint32x4_t; 4] = [t863[0], t863[1], t863[2], t863[3]];
    x3 = t864;
    let t865: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1s)), vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1s))];
    let t866 = [vdupq_n_u32(7u32); 2];
    let t867: [uint32x4_t; 2] = [vandq_u32(t865[0], t866[0]), vandq_u32(t865[1], t866[1])];
    let t868 = [vdupq_n_u32(8u32); 2];
    let t869: [uint32x4_t; 2] = [vmulq_u32(t867[0], t868[0]), vmulq_u32(t867[1], t868[1])];
    x4 = t869;
    let mut t870: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x3[2]) as u64) << 32 | vgetq_lane_u32::<0>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t870[0] = vsetq_lane_u32::<0>(v as u32, t870[0]); t870[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t870[2]); }
    { let v = (((vgetq_lane_u32::<1>(x3[2]) as u64) << 32 | vgetq_lane_u32::<1>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t870[0] = vsetq_lane_u32::<1>(v as u32, t870[0]); t870[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t870[2]); }
    { let v = (((vgetq_lane_u32::<2>(x3[2]) as u64) << 32 | vgetq_lane_u32::<2>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t870[0] = vsetq_lane_u32::<2>(v as u32, t870[0]); t870[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t870[2]); }
    { let v = (((vgetq_lane_u32::<3>(x3[2]) as u64) << 32 | vgetq_lane_u32::<3>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t870[0] = vsetq_lane_u32::<3>(v as u32, t870[0]); t870[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t870[2]); }
    { let v = (((vgetq_lane_u32::<0>(x3[3]) as u64) << 32 | vgetq_lane_u32::<0>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t870[1] = vsetq_lane_u32::<0>(v as u32, t870[1]); t870[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t870[3]); }
    { let v = (((vgetq_lane_u32::<1>(x3[3]) as u64) << 32 | vgetq_lane_u32::<1>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t870[1] = vsetq_lane_u32::<1>(v as u32, t870[1]); t870[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t870[3]); }
    { let v = (((vgetq_lane_u32::<2>(x3[3]) as u64) << 32 | vgetq_lane_u32::<2>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t870[1] = vsetq_lane_u32::<2>(v as u32, t870[1]); t870[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t870[3]); }
    { let v = (((vgetq_lane_u32::<3>(x3[3]) as u64) << 32 | vgetq_lane_u32::<3>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t870[1] = vsetq_lane_u32::<3>(v as u32, t870[1]); t870[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t870[3]); }
    let t871: [uint32x4_t; 2] = [x4[0], x4[1]];
    let t872: [uint32x4_t; 2] = [vandq_u32(t871[0], vdupq_n_u32(63)), vandq_u32(t871[1], vdupq_n_u32(63))];
    let t873: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t870[0], vnegq_s32(vreinterpretq_s32_u32(t872[0]))), vshlq_u32(t870[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t872[0])))), vorrq_u32(vshlq_u32(t870[1], vnegq_s32(vreinterpretq_s32_u32(t872[1]))), vshlq_u32(t870[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t872[1])))), vshlq_u32(t870[2], vnegq_s32(vreinterpretq_s32_u32(t872[0]))), vshlq_u32(t870[3], vnegq_s32(vreinterpretq_s32_u32(t872[1])))];
    x5 = t873;
    let t874 = [vdupq_n_u32(0u32); 2];
    let t875: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t874[0])), vmvnq_u32(vceqq_u32(x4[1], t874[1]))];
    let t876 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
    let t877: [uint32x4_t; 2] = [vaddq_u32(x4[0], t876[0]), vaddq_u32(x4[1], t876[1])];
    let t878 = [vdupq_n_u32(64u32); 2];
    let t879: [uint32x4_t; 2] = [vcgtq_u32(t877[0], t878[0]), vcgtq_u32(t877[1], t878[1])];
    let t880: [uint32x4_t; 2] = [vandq_u32(t875[0], t879[0]), vandq_u32(t875[1], t879[1])];
    let mut t881: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t882: [uint32x4_t; 2] = [vandq_u32(live[0], t880[0]), vandq_u32(live[1], t880[1])];
    let t883: [uint32x4_t; 2] = [vbicq_u32(live[0], t880[0]), vbicq_u32(live[1], t880[1])];
    if any(&t882) {
        let t884: u64 = (1u64).wrapping_mul(8u64);
        let t885 = [vdupq_n_u32((t884) as u32); 2];
        let t886 = [vdupq_n_u32(((t884) >> 32) as u32); 2];
        let t887: [uint32x4_t; 2] = [vaddq_u32(x3[0], t885[0]), vaddq_u32(x3[1], t885[1])];
        let t888: [uint32x4_t; 2] = [vcltq_u32(t887[0], x3[0]), vcltq_u32(t887[1], x3[1])];
        let t889: [uint32x4_t; 4] = [t887[0], t887[1], vsubq_u32(vaddq_u32(x3[2], t886[0]), t888[0]), vsubq_u32(vaddq_u32(x3[3], t886[1]), t888[1])];
        let mut t890: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t889[2]) as u64) << 32 | vgetq_lane_u32::<0>(t889[0]) as u64) as usize as *const u64).read_unaligned(); t890[0] = vsetq_lane_u32::<0>(v as u32, t890[0]); t890[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t890[2]); }
        { let v = (((vgetq_lane_u32::<1>(t889[2]) as u64) << 32 | vgetq_lane_u32::<1>(t889[0]) as u64) as usize as *const u64).read_unaligned(); t890[0] = vsetq_lane_u32::<1>(v as u32, t890[0]); t890[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t890[2]); }
        { let v = (((vgetq_lane_u32::<2>(t889[2]) as u64) << 32 | vgetq_lane_u32::<2>(t889[0]) as u64) as usize as *const u64).read_unaligned(); t890[0] = vsetq_lane_u32::<2>(v as u32, t890[0]); t890[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t890[2]); }
        { let v = (((vgetq_lane_u32::<3>(t889[2]) as u64) << 32 | vgetq_lane_u32::<3>(t889[0]) as u64) as usize as *const u64).read_unaligned(); t890[0] = vsetq_lane_u32::<3>(v as u32, t890[0]); t890[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t890[2]); }
        { let v = (((vgetq_lane_u32::<0>(t889[3]) as u64) << 32 | vgetq_lane_u32::<0>(t889[1]) as u64) as usize as *const u64).read_unaligned(); t890[1] = vsetq_lane_u32::<0>(v as u32, t890[1]); t890[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t890[3]); }
        { let v = (((vgetq_lane_u32::<1>(t889[3]) as u64) << 32 | vgetq_lane_u32::<1>(t889[1]) as u64) as usize as *const u64).read_unaligned(); t890[1] = vsetq_lane_u32::<1>(v as u32, t890[1]); t890[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t890[3]); }
        { let v = (((vgetq_lane_u32::<2>(t889[3]) as u64) << 32 | vgetq_lane_u32::<2>(t889[1]) as u64) as usize as *const u64).read_unaligned(); t890[1] = vsetq_lane_u32::<2>(v as u32, t890[1]); t890[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t890[3]); }
        { let v = (((vgetq_lane_u32::<3>(t889[3]) as u64) << 32 | vgetq_lane_u32::<3>(t889[1]) as u64) as usize as *const u64).read_unaligned(); t890[1] = vsetq_lane_u32::<3>(v as u32, t890[1]); t890[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t890[3]); }
        let t891: [uint32x4_t; 2] = [vsubq_u32(t878[0], x4[0]), vsubq_u32(t878[1], x4[1])];
        let t892: [uint32x4_t; 2] = [t891[0], t891[1]];
        let t893: [uint32x4_t; 2] = [vandq_u32(t892[0], vdupq_n_u32(63)), vandq_u32(t892[1], vdupq_n_u32(63))];
        let t894: [uint32x4_t; 4] = [vshlq_u32(t890[0], vreinterpretq_s32_u32(t893[0])), vshlq_u32(t890[1], vreinterpretq_s32_u32(t893[1])), vorrq_u32(vshlq_u32(t890[2], vreinterpretq_s32_u32(t893[0])), vshlq_u32(t890[0], vsubq_s32(vreinterpretq_s32_u32(t893[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t890[3], vreinterpretq_s32_u32(t893[1])), vshlq_u32(t890[1], vsubq_s32(vreinterpretq_s32_u32(t893[1]), vdupq_n_s32(32))))];
        let t895: [uint32x4_t; 4] = [vorrq_u32(x5[0], t894[0]), vorrq_u32(x5[1], t894[1]), vorrq_u32(x5[2], t894[2]), vorrq_u32(x5[3], t894[3])];
        t881 = [vbslq_u32(t882[0], t895[0], t881[0]), vbslq_u32(t882[1], t895[1], t881[1]), vbslq_u32(t882[0], t895[2], t881[2]), vbslq_u32(t882[1], t895[3], t881[3])];
    }
    {
        t881 = [vbslq_u32(t883[0], x5[0], t881[0]), vbslq_u32(t883[1], x5[1], t881[1]), vbslq_u32(t883[0], x5[2], t881[2]), vbslq_u32(t883[1], x5[3], t881[3])];
    }
    res = t881;
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
        let t896: [uint32x4_t; 4];
        if (x1s) == 8u32 && ((x1f) as u64) + 7 * ((x1s) as u64) <= u32::MAX as u64 {
            let t897 = (arg0 as *const u8).add(((x1f) as usize) * 1);
            let t8980 = vld2q_u32((t897 as *const u32).add(0));
            let t8981 = vld2q_u32((t897 as *const u32).add(8));
            let t898: [uint32x4_t; 4] = [t8980.0, t8981.0, t8980.1, t8981.1];
            t896 = t898;
        } else {
            let t899: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1s)), vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1s))];
            let mut t900: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t899[0]) as usize * 1) as *const u64).read_unaligned(); t900[0] = vsetq_lane_u32::<0>(v as u32, t900[0]); t900[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t900[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t899[0]) as usize * 1) as *const u64).read_unaligned(); t900[0] = vsetq_lane_u32::<1>(v as u32, t900[0]); t900[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t900[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t899[0]) as usize * 1) as *const u64).read_unaligned(); t900[0] = vsetq_lane_u32::<2>(v as u32, t900[0]); t900[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t900[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t899[0]) as usize * 1) as *const u64).read_unaligned(); t900[0] = vsetq_lane_u32::<3>(v as u32, t900[0]); t900[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t900[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t899[1]) as usize * 1) as *const u64).read_unaligned(); t900[1] = vsetq_lane_u32::<0>(v as u32, t900[1]); t900[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t900[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t899[1]) as usize * 1) as *const u64).read_unaligned(); t900[1] = vsetq_lane_u32::<1>(v as u32, t900[1]); t900[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t900[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t899[1]) as usize * 1) as *const u64).read_unaligned(); t900[1] = vsetq_lane_u32::<2>(v as u32, t900[1]); t900[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t900[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t899[1]) as usize * 1) as *const u64).read_unaligned(); t900[1] = vsetq_lane_u32::<3>(v as u32, t900[1]); t900[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t900[3]); }
            t896 = t900;
        }
        res = t896;
        return res;
    }
    if ((x2) == (4u32)) {
        let t901: [uint32x4_t; 2];
        if (x1s) == 4u32 && ((x1f) as u64) + 7 * ((x1s) as u64) <= u32::MAX as u64 {
            let t902 = (arg0 as *const u8).add(((x1f) as usize) * 1);
            let t903: [uint32x4_t; 2] = core::array::from_fn(|r| vld1q_u32((t902 as *const u32).add(4 * r)));
            t901 = t903;
        } else {
            let t904: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1s)), vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1s))];
            let mut t905: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            t905[0] = vsetq_lane_u32::<0>((((arg0 as *const u8).add(vgetq_lane_u32::<0>(t904[0]) as usize * 1) as *const u32).read_unaligned() as u32), t905[0]);
            t905[0] = vsetq_lane_u32::<1>((((arg0 as *const u8).add(vgetq_lane_u32::<1>(t904[0]) as usize * 1) as *const u32).read_unaligned() as u32), t905[0]);
            t905[0] = vsetq_lane_u32::<2>((((arg0 as *const u8).add(vgetq_lane_u32::<2>(t904[0]) as usize * 1) as *const u32).read_unaligned() as u32), t905[0]);
            t905[0] = vsetq_lane_u32::<3>((((arg0 as *const u8).add(vgetq_lane_u32::<3>(t904[0]) as usize * 1) as *const u32).read_unaligned() as u32), t905[0]);
            t905[1] = vsetq_lane_u32::<0>((((arg0 as *const u8).add(vgetq_lane_u32::<0>(t904[1]) as usize * 1) as *const u32).read_unaligned() as u32), t905[1]);
            t905[1] = vsetq_lane_u32::<1>((((arg0 as *const u8).add(vgetq_lane_u32::<1>(t904[1]) as usize * 1) as *const u32).read_unaligned() as u32), t905[1]);
            t905[1] = vsetq_lane_u32::<2>((((arg0 as *const u8).add(vgetq_lane_u32::<2>(t904[1]) as usize * 1) as *const u32).read_unaligned() as u32), t905[1]);
            t905[1] = vsetq_lane_u32::<3>((((arg0 as *const u8).add(vgetq_lane_u32::<3>(t904[1]) as usize * 1) as *const u32).read_unaligned() as u32), t905[1]);
            t901 = t905;
        }
        let t906: [uint32x4_t; 4] = [t901[0], t901[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t906;
        return res;
    }
    if ((x2) == (2u32)) {
        let t907: [uint32x4_t; 2];
        if (x1s) == 2u32 && ((x1f) as u64) + 7 * ((x1s) as u64) <= u32::MAX as u64 {
            let t908 = (arg0 as *const u8).add(((x1f) as usize) * 1);
            let t909: [uint32x4_t; 2] = core::array::from_fn(|r| vmovl_u16(vld1_u16((t908 as *const u16).add(4 * r))));
            t907 = t909;
        } else {
            let t910: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1s)), vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1s))];
            let mut t911: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            t911[0] = vsetq_lane_u32::<0>((((arg0 as *const u8).add(vgetq_lane_u32::<0>(t910[0]) as usize * 1) as *const u16).read_unaligned() as u32), t911[0]);
            t911[0] = vsetq_lane_u32::<1>((((arg0 as *const u8).add(vgetq_lane_u32::<1>(t910[0]) as usize * 1) as *const u16).read_unaligned() as u32), t911[0]);
            t911[0] = vsetq_lane_u32::<2>((((arg0 as *const u8).add(vgetq_lane_u32::<2>(t910[0]) as usize * 1) as *const u16).read_unaligned() as u32), t911[0]);
            t911[0] = vsetq_lane_u32::<3>((((arg0 as *const u8).add(vgetq_lane_u32::<3>(t910[0]) as usize * 1) as *const u16).read_unaligned() as u32), t911[0]);
            t911[1] = vsetq_lane_u32::<0>((((arg0 as *const u8).add(vgetq_lane_u32::<0>(t910[1]) as usize * 1) as *const u16).read_unaligned() as u32), t911[1]);
            t911[1] = vsetq_lane_u32::<1>((((arg0 as *const u8).add(vgetq_lane_u32::<1>(t910[1]) as usize * 1) as *const u16).read_unaligned() as u32), t911[1]);
            t911[1] = vsetq_lane_u32::<2>((((arg0 as *const u8).add(vgetq_lane_u32::<2>(t910[1]) as usize * 1) as *const u16).read_unaligned() as u32), t911[1]);
            t911[1] = vsetq_lane_u32::<3>((((arg0 as *const u8).add(vgetq_lane_u32::<3>(t910[1]) as usize * 1) as *const u16).read_unaligned() as u32), t911[1]);
            t907 = t911;
        }
        let t912: [uint32x4_t; 4] = [t907[0], t907[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t912;
        return res;
    }
    if ((x2) == (1u32)) {
        let t913: [uint32x4_t; 2];
        if (x1s) == 1u32 && ((x1f) as u64) + 7 * ((x1s) as u64) <= u32::MAX as u64 {
            let t914 = (arg0 as *const u8).add(((x1f) as usize) * 1);
            let t916 = vmovl_u8(vld1_u8(t914.add(0)));
            let t915: [uint32x4_t; 2] = [vmovl_u16(vget_low_u16(t916)), vmovl_u16(vget_high_u16(t916))];
            t913 = t915;
        } else {
            let t917: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1s)), vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1s))];
            let mut t918: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            t918[0] = vsetq_lane_u32::<0>((((arg0 as *const u8).add(vgetq_lane_u32::<0>(t917[0]) as usize * 1) as *const u8).read_unaligned() as u32), t918[0]);
            t918[0] = vsetq_lane_u32::<1>((((arg0 as *const u8).add(vgetq_lane_u32::<1>(t917[0]) as usize * 1) as *const u8).read_unaligned() as u32), t918[0]);
            t918[0] = vsetq_lane_u32::<2>((((arg0 as *const u8).add(vgetq_lane_u32::<2>(t917[0]) as usize * 1) as *const u8).read_unaligned() as u32), t918[0]);
            t918[0] = vsetq_lane_u32::<3>((((arg0 as *const u8).add(vgetq_lane_u32::<3>(t917[0]) as usize * 1) as *const u8).read_unaligned() as u32), t918[0]);
            t918[1] = vsetq_lane_u32::<0>((((arg0 as *const u8).add(vgetq_lane_u32::<0>(t917[1]) as usize * 1) as *const u8).read_unaligned() as u32), t918[1]);
            t918[1] = vsetq_lane_u32::<1>((((arg0 as *const u8).add(vgetq_lane_u32::<1>(t917[1]) as usize * 1) as *const u8).read_unaligned() as u32), t918[1]);
            t918[1] = vsetq_lane_u32::<2>((((arg0 as *const u8).add(vgetq_lane_u32::<2>(t917[1]) as usize * 1) as *const u8).read_unaligned() as u32), t918[1]);
            t918[1] = vsetq_lane_u32::<3>((((arg0 as *const u8).add(vgetq_lane_u32::<3>(t917[1]) as usize * 1) as *const u8).read_unaligned() as u32), t918[1]);
            t913 = t918;
        }
        let t919: [uint32x4_t; 4] = [t913[0], t913[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t919;
        return res;
    }
    let t920: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1s)), vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1s))];
    let t921 = [vdupq_n_u32(!(7u32)); 2];
    let t922: [uint32x4_t; 2] = [vandq_u32(t920[0], t921[0]), vandq_u32(t920[1], t921[1])];
    let t923: [uint32x4_t; 2] = [t922[0], t922[1]];
    let t924: u64 = ((arg0) as usize as u64);
    let t925 = [vdupq_n_u32((t924) as u32); 2];
    let t926 = [vdupq_n_u32(((t924) >> 32) as u32); 2];
    let t927: [uint32x4_t; 2] = [vaddq_u32(t925[0], t923[0]), vaddq_u32(t925[1], t923[1])];
    let t928: [uint32x4_t; 2] = [vcltq_u32(t927[0], t925[0]), vcltq_u32(t927[1], t925[1])];
    let t929: [uint32x4_t; 4] = [t927[0], t927[1], vsubq_u32(vaddq_u32(t926[0], vdupq_n_u32(0)), t928[0]), vsubq_u32(vaddq_u32(t926[1], vdupq_n_u32(0)), t928[1])];
    let t930: [uint32x4_t; 4] = [t929[0], t929[1], t929[2], t929[3]];
    x3 = t930;
    let t931: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1s)), vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1s))];
    let t932 = [vdupq_n_u32(7u32); 2];
    let t933: [uint32x4_t; 2] = [vandq_u32(t931[0], t932[0]), vandq_u32(t931[1], t932[1])];
    let t934 = [vdupq_n_u32(8u32); 2];
    let t935: [uint32x4_t; 2] = [vmulq_u32(t933[0], t934[0]), vmulq_u32(t933[1], t934[1])];
    x4 = t935;
    let mut t936: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x3[2]) as u64) << 32 | vgetq_lane_u32::<0>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t936[0] = vsetq_lane_u32::<0>(v as u32, t936[0]); t936[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t936[2]); }
    { let v = (((vgetq_lane_u32::<1>(x3[2]) as u64) << 32 | vgetq_lane_u32::<1>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t936[0] = vsetq_lane_u32::<1>(v as u32, t936[0]); t936[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t936[2]); }
    { let v = (((vgetq_lane_u32::<2>(x3[2]) as u64) << 32 | vgetq_lane_u32::<2>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t936[0] = vsetq_lane_u32::<2>(v as u32, t936[0]); t936[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t936[2]); }
    { let v = (((vgetq_lane_u32::<3>(x3[2]) as u64) << 32 | vgetq_lane_u32::<3>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t936[0] = vsetq_lane_u32::<3>(v as u32, t936[0]); t936[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t936[2]); }
    { let v = (((vgetq_lane_u32::<0>(x3[3]) as u64) << 32 | vgetq_lane_u32::<0>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t936[1] = vsetq_lane_u32::<0>(v as u32, t936[1]); t936[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t936[3]); }
    { let v = (((vgetq_lane_u32::<1>(x3[3]) as u64) << 32 | vgetq_lane_u32::<1>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t936[1] = vsetq_lane_u32::<1>(v as u32, t936[1]); t936[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t936[3]); }
    { let v = (((vgetq_lane_u32::<2>(x3[3]) as u64) << 32 | vgetq_lane_u32::<2>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t936[1] = vsetq_lane_u32::<2>(v as u32, t936[1]); t936[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t936[3]); }
    { let v = (((vgetq_lane_u32::<3>(x3[3]) as u64) << 32 | vgetq_lane_u32::<3>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t936[1] = vsetq_lane_u32::<3>(v as u32, t936[1]); t936[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t936[3]); }
    let t937: [uint32x4_t; 2] = [x4[0], x4[1]];
    let t938: [uint32x4_t; 2] = [vandq_u32(t937[0], vdupq_n_u32(63)), vandq_u32(t937[1], vdupq_n_u32(63))];
    let t939: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t936[0], vnegq_s32(vreinterpretq_s32_u32(t938[0]))), vshlq_u32(t936[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t938[0])))), vorrq_u32(vshlq_u32(t936[1], vnegq_s32(vreinterpretq_s32_u32(t938[1]))), vshlq_u32(t936[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t938[1])))), vshlq_u32(t936[2], vnegq_s32(vreinterpretq_s32_u32(t938[0]))), vshlq_u32(t936[3], vnegq_s32(vreinterpretq_s32_u32(t938[1])))];
    x5 = t939;
    let t940 = [vdupq_n_u32(0u32); 2];
    let t941: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t940[0])), vmvnq_u32(vceqq_u32(x4[1], t940[1]))];
    let t942 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
    let t943: [uint32x4_t; 2] = [vaddq_u32(x4[0], t942[0]), vaddq_u32(x4[1], t942[1])];
    let t944 = [vdupq_n_u32(64u32); 2];
    let t945: [uint32x4_t; 2] = [vcgtq_u32(t943[0], t944[0]), vcgtq_u32(t943[1], t944[1])];
    let t946: [uint32x4_t; 2] = [vandq_u32(t941[0], t945[0]), vandq_u32(t941[1], t945[1])];
    let mut t947: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t948: [uint32x4_t; 2] = [vmvnq_u32(t946[0]), vmvnq_u32(t946[1])];
    if any(&t946) {
        let t949: u64 = (1u64).wrapping_mul(8u64);
        let t950 = [vdupq_n_u32((t949) as u32); 2];
        let t951 = [vdupq_n_u32(((t949) >> 32) as u32); 2];
        let t952: [uint32x4_t; 2] = [vaddq_u32(x3[0], t950[0]), vaddq_u32(x3[1], t950[1])];
        let t953: [uint32x4_t; 2] = [vcltq_u32(t952[0], x3[0]), vcltq_u32(t952[1], x3[1])];
        let t954: [uint32x4_t; 4] = [t952[0], t952[1], vsubq_u32(vaddq_u32(x3[2], t951[0]), t953[0]), vsubq_u32(vaddq_u32(x3[3], t951[1]), t953[1])];
        let mut t955: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t954[2]) as u64) << 32 | vgetq_lane_u32::<0>(t954[0]) as u64) as usize as *const u64).read_unaligned(); t955[0] = vsetq_lane_u32::<0>(v as u32, t955[0]); t955[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t955[2]); }
        { let v = (((vgetq_lane_u32::<1>(t954[2]) as u64) << 32 | vgetq_lane_u32::<1>(t954[0]) as u64) as usize as *const u64).read_unaligned(); t955[0] = vsetq_lane_u32::<1>(v as u32, t955[0]); t955[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t955[2]); }
        { let v = (((vgetq_lane_u32::<2>(t954[2]) as u64) << 32 | vgetq_lane_u32::<2>(t954[0]) as u64) as usize as *const u64).read_unaligned(); t955[0] = vsetq_lane_u32::<2>(v as u32, t955[0]); t955[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t955[2]); }
        { let v = (((vgetq_lane_u32::<3>(t954[2]) as u64) << 32 | vgetq_lane_u32::<3>(t954[0]) as u64) as usize as *const u64).read_unaligned(); t955[0] = vsetq_lane_u32::<3>(v as u32, t955[0]); t955[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t955[2]); }
        { let v = (((vgetq_lane_u32::<0>(t954[3]) as u64) << 32 | vgetq_lane_u32::<0>(t954[1]) as u64) as usize as *const u64).read_unaligned(); t955[1] = vsetq_lane_u32::<0>(v as u32, t955[1]); t955[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t955[3]); }
        { let v = (((vgetq_lane_u32::<1>(t954[3]) as u64) << 32 | vgetq_lane_u32::<1>(t954[1]) as u64) as usize as *const u64).read_unaligned(); t955[1] = vsetq_lane_u32::<1>(v as u32, t955[1]); t955[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t955[3]); }
        { let v = (((vgetq_lane_u32::<2>(t954[3]) as u64) << 32 | vgetq_lane_u32::<2>(t954[1]) as u64) as usize as *const u64).read_unaligned(); t955[1] = vsetq_lane_u32::<2>(v as u32, t955[1]); t955[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t955[3]); }
        { let v = (((vgetq_lane_u32::<3>(t954[3]) as u64) << 32 | vgetq_lane_u32::<3>(t954[1]) as u64) as usize as *const u64).read_unaligned(); t955[1] = vsetq_lane_u32::<3>(v as u32, t955[1]); t955[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t955[3]); }
        let t956: [uint32x4_t; 2] = [vsubq_u32(t944[0], x4[0]), vsubq_u32(t944[1], x4[1])];
        let t957: [uint32x4_t; 2] = [t956[0], t956[1]];
        let t958: [uint32x4_t; 2] = [vandq_u32(t957[0], vdupq_n_u32(63)), vandq_u32(t957[1], vdupq_n_u32(63))];
        let t959: [uint32x4_t; 4] = [vshlq_u32(t955[0], vreinterpretq_s32_u32(t958[0])), vshlq_u32(t955[1], vreinterpretq_s32_u32(t958[1])), vorrq_u32(vshlq_u32(t955[2], vreinterpretq_s32_u32(t958[0])), vshlq_u32(t955[0], vsubq_s32(vreinterpretq_s32_u32(t958[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t955[3], vreinterpretq_s32_u32(t958[1])), vshlq_u32(t955[1], vsubq_s32(vreinterpretq_s32_u32(t958[1]), vdupq_n_s32(32))))];
        let t960: [uint32x4_t; 4] = [vorrq_u32(x5[0], t959[0]), vorrq_u32(x5[1], t959[1]), vorrq_u32(x5[2], t959[2]), vorrq_u32(x5[3], t959[3])];
        t947 = [vbslq_u32(t946[0], t960[0], t947[0]), vbslq_u32(t946[1], t960[1], t947[1]), vbslq_u32(t946[0], t960[2], t947[2]), vbslq_u32(t946[1], t960[3], t947[3])];
    }
    {
        t947 = [vbslq_u32(t948[0], x5[0], t947[0]), vbslq_u32(t948[1], x5[1], t947[1]), vbslq_u32(t948[0], x5[2], t947[2]), vbslq_u32(t948[1], x5[3], t947[3])];
    }
    res = t947;
    return res;
}

#[inline(always)]
unsafe fn b6_i_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t961: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x0), vld1q_u32(LANE[1].as_ptr()))];
    let t962: [uint32x4_t; 2] = [vshlq_u32(t961[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(t961[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
    let t963 = [vdupq_n_u32(255u32); 2];
    let t964: [uint32x4_t; 2] = [vandq_u32(t961[0], t963[0]), vandq_u32(t961[1], t963[1])];
    let t965: [uint32x4_t; 2] = [vmulq_u32(t962[0], vdupq_n_u32(1000)), vmulq_u32(t962[1], vdupq_n_u32(1000))];
    let t966: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(t962[0]), vget_low_u32(vdupq_n_u32(1000)))), vreinterpretq_u32_u64(vmull_high_u32(t962[0], vdupq_n_u32(1000)))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(t962[1]), vget_low_u32(vdupq_n_u32(1000)))), vreinterpretq_u32_u64(vmull_high_u32(t962[1], vdupq_n_u32(1000))))];
    let t967: [uint32x4_t; 2] = [vaddq_u32(t965[0], t964[0]), vaddq_u32(t965[1], t964[1])];
    let t968: [uint32x4_t; 2] = [vcltq_u32(t967[0], t965[0]), vcltq_u32(t967[1], t965[1])];
    let t969: [uint32x4_t; 4] = [t967[0], t967[1], vsubq_u32(vaddq_u32(t966[0], vdupq_n_u32(0)), t968[0]), vsubq_u32(vaddq_u32(t966[1], vdupq_n_u32(0)), t968[1])];
    res = t969;
    return res;
}

#[inline(always)]
unsafe fn b6_i_f(l: &Launch, lid0: u32, x0: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t970: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x0), vld1q_u32(LANE[1].as_ptr()))];
    let t971: [uint32x4_t; 2] = [vshlq_u32(t970[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(t970[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
    let t972 = [vdupq_n_u32(255u32); 2];
    let t973: [uint32x4_t; 2] = [vandq_u32(t970[0], t972[0]), vandq_u32(t970[1], t972[1])];
    let t974: [uint32x4_t; 2] = [vmulq_u32(t971[0], vdupq_n_u32(1000)), vmulq_u32(t971[1], vdupq_n_u32(1000))];
    let t975: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(t971[0]), vget_low_u32(vdupq_n_u32(1000)))), vreinterpretq_u32_u64(vmull_high_u32(t971[0], vdupq_n_u32(1000)))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(t971[1]), vget_low_u32(vdupq_n_u32(1000)))), vreinterpretq_u32_u64(vmull_high_u32(t971[1], vdupq_n_u32(1000))))];
    let t976: [uint32x4_t; 2] = [vaddq_u32(t974[0], t973[0]), vaddq_u32(t974[1], t973[1])];
    let t977: [uint32x4_t; 2] = [vcltq_u32(t976[0], t974[0]), vcltq_u32(t976[1], t974[1])];
    let t978: [uint32x4_t; 4] = [t976[0], t976[1], vsubq_u32(vaddq_u32(t975[0], vdupq_n_u32(0)), t977[0]), vsubq_u32(vaddq_u32(t975[1], vdupq_n_u32(0)), t977[1])];
    res = t978;
    return res;
}

#[inline(always)]
unsafe fn b6_u8_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t979: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32((x0 & 255u32)), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32((x0 & 255u32)), vld1q_u32(LANE[1].as_ptr()))];
    let t980: [uint32x4_t; 2] = [t979[0], t979[1]];
    let t981: u64 = ((((((((x0 >> 8)) as u64)).wrapping_mul(1000u64)) as usize as *const u8)) as usize as u64);
    let t982 = [vdupq_n_u32((t981) as u32); 2];
    let t983 = [vdupq_n_u32(((t981) >> 32) as u32); 2];
    let t984: [uint32x4_t; 2] = [vaddq_u32(t982[0], t980[0]), vaddq_u32(t982[1], t980[1])];
    let t985: [uint32x4_t; 2] = [vcltq_u32(t984[0], t982[0]), vcltq_u32(t984[1], t982[1])];
    let t986: [uint32x4_t; 4] = [t984[0], t984[1], vsubq_u32(vaddq_u32(t983[0], vdupq_n_u32(0)), t985[0]), vsubq_u32(vaddq_u32(t983[1], vdupq_n_u32(0)), t985[1])];
    let t987: [uint32x4_t; 4] = [t986[0], t986[1], t986[2], t986[3]];
    res = t987;
    return res;
}

#[inline(always)]
unsafe fn b6_u8_f(l: &Launch, lid0: u32, x0: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t988: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32((x0 & 255u32)), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32((x0 & 255u32)), vld1q_u32(LANE[1].as_ptr()))];
    let t989: [uint32x4_t; 2] = [t988[0], t988[1]];
    let t990: u64 = ((((((((x0 >> 8)) as u64)).wrapping_mul(1000u64)) as usize as *const u8)) as usize as u64);
    let t991 = [vdupq_n_u32((t990) as u32); 2];
    let t992 = [vdupq_n_u32(((t990) >> 32) as u32); 2];
    let t993: [uint32x4_t; 2] = [vaddq_u32(t991[0], t989[0]), vaddq_u32(t991[1], t989[1])];
    let t994: [uint32x4_t; 2] = [vcltq_u32(t993[0], t991[0]), vcltq_u32(t993[1], t991[1])];
    let t995: [uint32x4_t; 4] = [t993[0], t993[1], vsubq_u32(vaddq_u32(t992[0], vdupq_n_u32(0)), t994[0]), vsubq_u32(vaddq_u32(t992[1], vdupq_n_u32(0)), t994[1])];
    let t996: [uint32x4_t; 4] = [t995[0], t995[1], t995[2], t995[3]];
    res = t996;
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
    let mut t1001 = live;
    loop {
        let t1003 = bits(&t1001);
        if t1003.count_ones() <= 2 {
            let mut t1004 = t1003;
            while t1004 != 0 {
                let j = t1004.trailing_zeros() as usize;
                t1004 &= t1004 - 1;
                let t1005 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x1);
                let mut x1_j: u32 = t1005[j];
                let t1006 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x2);
                let mut x2_j: u64 = ((t1006[8 + j] as u64) << 32 | t1006[j] as u64);
                let mut x3_j: u32 = x3;
                loop {
                    if !(((x3_j) < (((x1_j) & (15u32))))) { break; }
                    x2_j = ((x2_j).wrapping_mul(31u64)).wrapping_add(((x3_j) as u64));
                    x3_j = (x3_j).wrapping_add(2u32);
                }
                let mut t1007 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x2);
                t1007[j] = x2_j as u32;
                t1007[8 + j] = (x2_j as u64 >> 32) as u32;
                x2 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t1007);
            }
            break;
        }
        // Lanes part: each its own rounds.
        let t1008 = [vdupq_n_u32(15u32); 2];
        let t1009: [uint32x4_t; 2] = [vandq_u32(x1[0], t1008[0]), vandq_u32(x1[1], t1008[1])];
        let t1010 = [vdupq_n_u32(x3); 2];
        let t1011: [uint32x4_t; 2] = [vcltq_u32(t1010[0], t1009[0]), vcltq_u32(t1010[1], t1009[1])];
        t1001 = [vandq_u32(t1001[0], t1011[0]), vandq_u32(t1001[1], t1011[1])];
        if !(any(&t1001)) { break; }
        let t1013: u64 = 31u64;
        let t1014 = [vdupq_n_u32((t1013) as u32); 2];
        let t1015: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x2[0]), vget_low_u32(t1014[0]))), vreinterpretq_u32_u64(vmull_high_u32(x2[0], t1014[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x2[1]), vget_low_u32(t1014[1]))), vreinterpretq_u32_u64(vmull_high_u32(x2[1], t1014[1])))];
        let t1016: [uint32x4_t; 4] = [vmulq_u32(x2[0], t1014[0]), vmulq_u32(x2[1], t1014[1]), vmlaq_u32(t1015[0], t1014[0], x2[2]), vmlaq_u32(t1015[1], t1014[1], x2[3])];
        let t1017: u64 = ((x3) as u64);
        let t1018 = [vdupq_n_u32((t1017) as u32); 2];
        let t1019 = [vdupq_n_u32(((t1017) >> 32) as u32); 2];
        let t1020: [uint32x4_t; 2] = [vaddq_u32(t1016[0], t1018[0]), vaddq_u32(t1016[1], t1018[1])];
        let t1021: [uint32x4_t; 2] = [vcltq_u32(t1020[0], t1016[0]), vcltq_u32(t1020[1], t1016[1])];
        let t1022: [uint32x4_t; 4] = [t1020[0], t1020[1], vsubq_u32(vaddq_u32(t1016[2], t1019[0]), t1021[0]), vsubq_u32(vaddq_u32(t1016[3], t1019[1]), t1021[1])];
        x2 = [vbslq_u32(t1001[0], t1022[0], x2[0]), vbslq_u32(t1001[1], t1022[1], x2[1]), vbslq_u32(t1001[0], t1022[2], x2[2]), vbslq_u32(t1001[1], t1022[3], x2[3])];
        let t1023: [uint32x4_t; 2] = [vandq_u32(t1001[0], live[0]), vandq_u32(t1001[1], live[1])];
        x3 = (x3).wrapping_add(2u32);
    }
    res = x2;
    return res;
}

#[inline(always)]
unsafe fn b8_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t1024: u64;
    if ((x0) >= (64u32)) {
        t1024 = !(0u64);
    } else {
        t1024 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t1024;
    return res;
}

#[inline(always)]
unsafe fn b8_s_f(l: &Launch, lid0: u32, x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t1025: u64;
    if ((x0) >= (64u32)) {
        t1025 = !(0u64);
    } else {
        t1025 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t1025;
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
    let mut t1026: bool = !(arg5).is_null();
    if t1026 {
        let mut t1027: bool = ((l.group[0]) >= ((arg5.wrapping_add((((x6) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        if !t1027 {
            t1027 = ((l.group[1]) >= ((arg5.wrapping_add(((((x6).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        let mut t1028: bool = t1027;
        if !t1028 {
            t1028 = ((l.group[2]) >= ((arg5.wrapping_add(((((x6).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        t1026 = t1028;
    }
    if t1026 {
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
        let t1029: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
        let mut live = t1029;
        let gid0 = l.group[0] * l.block[0] + lid0;
        'chunk: {
            b0_r0r1r2r3sg_p(l, lid0, live, arg0, arg1, arg2, arg3, x4);
        }
    }
}
