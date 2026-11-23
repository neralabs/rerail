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
            let t11 = vminvq_u32(vminq_u32(t9[0], t9[1]));
            if t11 == vmaxvq_u32(vmaxq_u32(t9[0], t9[1])) {
                let t12 = ((arg0 as *const u8).add(t11 as usize * 8) as *const u64).read_unaligned();
                t10 = [vdupq_n_u32(t12 as u32), vdupq_n_u32(t12 as u32), vdupq_n_u32((t12 >> 32) as u32), vdupq_n_u32((t12 >> 32) as u32)];
            } else {
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t9[0]) as usize * 8) as *const u64).read_unaligned(); t10[0] = vsetq_lane_u32::<0>(v as u32, t10[0]); t10[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t10[2]); }
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t9[0]) as usize * 8) as *const u64).read_unaligned(); t10[0] = vsetq_lane_u32::<1>(v as u32, t10[0]); t10[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t10[2]); }
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t9[0]) as usize * 8) as *const u64).read_unaligned(); t10[0] = vsetq_lane_u32::<2>(v as u32, t10[0]); t10[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t10[2]); }
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t9[0]) as usize * 8) as *const u64).read_unaligned(); t10[0] = vsetq_lane_u32::<3>(v as u32, t10[0]); t10[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t10[2]); }
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t9[1]) as usize * 8) as *const u64).read_unaligned(); t10[1] = vsetq_lane_u32::<0>(v as u32, t10[1]); t10[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t10[3]); }
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t9[1]) as usize * 8) as *const u64).read_unaligned(); t10[1] = vsetq_lane_u32::<1>(v as u32, t10[1]); t10[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t10[3]); }
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t9[1]) as usize * 8) as *const u64).read_unaligned(); t10[1] = vsetq_lane_u32::<2>(v as u32, t10[1]); t10[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t10[3]); }
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t9[1]) as usize * 8) as *const u64).read_unaligned(); t10[1] = vsetq_lane_u32::<3>(v as u32, t10[1]); t10[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t10[3]); }
            }
            t6 = t10;
        }
        x6 = t6;
        let t13 = (arg1 as *const u8).add(((gid0) as usize) * 8);
        let mut t14: [uint32x4_t; 4];
        if all(&live) {
            let t150 = vld2q_u32((t13 as *const u32).add(0));
            let t151 = vld2q_u32((t13 as *const u32).add(8));
            let t15: [uint32x4_t; 4] = [t150.0, t151.0, t150.1, t151.1];
            t14 = t15;
        } else {
            let t16 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t1[0]), vandq_u32(live[1], t1[1]))));
            let t17: [uint32x4_t; 2] = [vbslq_u32(live[0], t1[0], t16), vbslq_u32(live[1], t1[1], t16)];
            let mut t18: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            let t19 = vminvq_u32(vminq_u32(t17[0], t17[1]));
            if t19 == vmaxvq_u32(vmaxq_u32(t17[0], t17[1])) {
                let t20 = ((arg1 as *const u8).add(t19 as usize * 8) as *const u64).read_unaligned();
                t18 = [vdupq_n_u32(t20 as u32), vdupq_n_u32(t20 as u32), vdupq_n_u32((t20 >> 32) as u32), vdupq_n_u32((t20 >> 32) as u32)];
            } else {
                { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<0>(t17[0]) as usize * 8) as *const u64).read_unaligned(); t18[0] = vsetq_lane_u32::<0>(v as u32, t18[0]); t18[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t18[2]); }
                { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<1>(t17[0]) as usize * 8) as *const u64).read_unaligned(); t18[0] = vsetq_lane_u32::<1>(v as u32, t18[0]); t18[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t18[2]); }
                { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<2>(t17[0]) as usize * 8) as *const u64).read_unaligned(); t18[0] = vsetq_lane_u32::<2>(v as u32, t18[0]); t18[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t18[2]); }
                { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<3>(t17[0]) as usize * 8) as *const u64).read_unaligned(); t18[0] = vsetq_lane_u32::<3>(v as u32, t18[0]); t18[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t18[2]); }
                { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<0>(t17[1]) as usize * 8) as *const u64).read_unaligned(); t18[1] = vsetq_lane_u32::<0>(v as u32, t18[1]); t18[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t18[3]); }
                { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<1>(t17[1]) as usize * 8) as *const u64).read_unaligned(); t18[1] = vsetq_lane_u32::<1>(v as u32, t18[1]); t18[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t18[3]); }
                { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<2>(t17[1]) as usize * 8) as *const u64).read_unaligned(); t18[1] = vsetq_lane_u32::<2>(v as u32, t18[1]); t18[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t18[3]); }
                { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<3>(t17[1]) as usize * 8) as *const u64).read_unaligned(); t18[1] = vsetq_lane_u32::<3>(v as u32, t18[1]); t18[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t18[3]); }
            }
            t14 = t18;
        }
        x7 = t14;
        let mut t21: [uint32x4_t; 2];
        if all(&live) {
            let t22 = (arg2 as *const u32).add((gid0) as usize);
            t21 = [vld1q_u32(t22.add(0)), vld1q_u32(t22.add(4))];
        } else {
            let t23 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t1[0]), vandq_u32(live[1], t1[1]))));
            let t24: [uint32x4_t; 2] = [vbslq_u32(live[0], t1[0], t23), vbslq_u32(live[1], t1[1], t23)];
            let t25 = arg2 as *const u32;
            let mut t26: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            t26[0] = vld1q_lane_u32::<0>(t25.add(vgetq_lane_u32::<0>(t24[0]) as usize), t26[0]);
            t26[0] = vld1q_lane_u32::<1>(t25.add(vgetq_lane_u32::<1>(t24[0]) as usize), t26[0]);
            t26[0] = vld1q_lane_u32::<2>(t25.add(vgetq_lane_u32::<2>(t24[0]) as usize), t26[0]);
            t26[0] = vld1q_lane_u32::<3>(t25.add(vgetq_lane_u32::<3>(t24[0]) as usize), t26[0]);
            t26[1] = vld1q_lane_u32::<0>(t25.add(vgetq_lane_u32::<0>(t24[1]) as usize), t26[1]);
            t26[1] = vld1q_lane_u32::<1>(t25.add(vgetq_lane_u32::<1>(t24[1]) as usize), t26[1]);
            t26[1] = vld1q_lane_u32::<2>(t25.add(vgetq_lane_u32::<2>(t24[1]) as usize), t26[1]);
            t26[1] = vld1q_lane_u32::<3>(t25.add(vgetq_lane_u32::<3>(t24[1]) as usize), t26[1]);
            t21 = t26;
        }
        x8 = t21;
        let t27 = [vdupq_n_u32(63u32); 2];
        let t28: [uint32x4_t; 2] = [vandq_u32(x8[0], t27[0]), vandq_u32(x8[1], t27[1])];
        x9 = t28;
        x10 = a0;
        let t29: [uint32x4_t; 2] = [vaddq_u32(x6[0], x7[0]), vaddq_u32(x6[1], x7[1])];
        let t30: [uint32x4_t; 2] = [vcltq_u32(t29[0], x6[0]), vcltq_u32(t29[1], x6[1])];
        let t31: [uint32x4_t; 4] = [t29[0], t29[1], vsubq_u32(vaddq_u32(x6[2], x7[2]), t30[0]), vsubq_u32(vaddq_u32(x6[3], x7[3]), t30[1])];
        let t32 = (arg3 as *const u8).add(((gid0.wrapping_add((0u32).wrapping_mul(x10))) as usize) * 8);
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
        let t34: [uint32x4_t; 2] = [vcltq_u32(x6[0], x7[0]), vcltq_u32(x6[1], x7[1])];
        let t35: [uint32x4_t; 4] = [vsubq_u32(x6[0], x7[0]), vsubq_u32(x6[1], x7[1]), vaddq_u32(vsubq_u32(x6[2], x7[2]), t34[0]), vaddq_u32(vsubq_u32(x6[3], x7[3]), t34[1])];
        let t36 = (arg3 as *const u8).add(((gid0.wrapping_add((1u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t36 as *mut u32).add(0), uint32x4x2_t(t35[0], t35[2]));
            vst2q_u32((t36 as *mut u32).add(8), uint32x4x2_t(t35[1], t35[3]));
        } else {
            let mut t37w = 0u64;
            let t37 = core::hint::black_box(&raw mut t37w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t36 as *mut u64).add(0), t37).write_unaligned(((vgetq_lane_u32::<0>(t35[2]) as u64) << 32 | vgetq_lane_u32::<0>(t35[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t36 as *mut u64).add(1), t37).write_unaligned(((vgetq_lane_u32::<1>(t35[2]) as u64) << 32 | vgetq_lane_u32::<1>(t35[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t36 as *mut u64).add(2), t37).write_unaligned(((vgetq_lane_u32::<2>(t35[2]) as u64) << 32 | vgetq_lane_u32::<2>(t35[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t36 as *mut u64).add(3), t37).write_unaligned(((vgetq_lane_u32::<3>(t35[2]) as u64) << 32 | vgetq_lane_u32::<3>(t35[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t36 as *mut u64).add(4), t37).write_unaligned(((vgetq_lane_u32::<0>(t35[3]) as u64) << 32 | vgetq_lane_u32::<0>(t35[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t36 as *mut u64).add(5), t37).write_unaligned(((vgetq_lane_u32::<1>(t35[3]) as u64) << 32 | vgetq_lane_u32::<1>(t35[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t36 as *mut u64).add(6), t37).write_unaligned(((vgetq_lane_u32::<2>(t35[3]) as u64) << 32 | vgetq_lane_u32::<2>(t35[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t36 as *mut u64).add(7), t37).write_unaligned(((vgetq_lane_u32::<3>(t35[3]) as u64) << 32 | vgetq_lane_u32::<3>(t35[1]) as u64));
        }
        let t38: [uint32x4_t; 2] = [x9[0], x9[1]];
        let t39: [uint32x4_t; 2] = [vandq_u32(t38[0], vdupq_n_u32(63)), vandq_u32(t38[1], vdupq_n_u32(63))];
        let t40: [uint32x4_t; 4] = [vshlq_u32(x6[0], vreinterpretq_s32_u32(t39[0])), vshlq_u32(x6[1], vreinterpretq_s32_u32(t39[1])), vorrq_u32(vshlq_u32(x6[2], vreinterpretq_s32_u32(t39[0])), vshlq_u32(x6[0], vsubq_s32(vreinterpretq_s32_u32(t39[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(x6[3], vreinterpretq_s32_u32(t39[1])), vshlq_u32(x6[1], vsubq_s32(vreinterpretq_s32_u32(t39[1]), vdupq_n_s32(32))))];
        let t41 = (arg3 as *const u8).add(((gid0.wrapping_add((2u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t41 as *mut u32).add(0), uint32x4x2_t(t40[0], t40[2]));
            vst2q_u32((t41 as *mut u32).add(8), uint32x4x2_t(t40[1], t40[3]));
        } else {
            let mut t42w = 0u64;
            let t42 = core::hint::black_box(&raw mut t42w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t41 as *mut u64).add(0), t42).write_unaligned(((vgetq_lane_u32::<0>(t40[2]) as u64) << 32 | vgetq_lane_u32::<0>(t40[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t41 as *mut u64).add(1), t42).write_unaligned(((vgetq_lane_u32::<1>(t40[2]) as u64) << 32 | vgetq_lane_u32::<1>(t40[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t41 as *mut u64).add(2), t42).write_unaligned(((vgetq_lane_u32::<2>(t40[2]) as u64) << 32 | vgetq_lane_u32::<2>(t40[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t41 as *mut u64).add(3), t42).write_unaligned(((vgetq_lane_u32::<3>(t40[2]) as u64) << 32 | vgetq_lane_u32::<3>(t40[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t41 as *mut u64).add(4), t42).write_unaligned(((vgetq_lane_u32::<0>(t40[3]) as u64) << 32 | vgetq_lane_u32::<0>(t40[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t41 as *mut u64).add(5), t42).write_unaligned(((vgetq_lane_u32::<1>(t40[3]) as u64) << 32 | vgetq_lane_u32::<1>(t40[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t41 as *mut u64).add(6), t42).write_unaligned(((vgetq_lane_u32::<2>(t40[3]) as u64) << 32 | vgetq_lane_u32::<2>(t40[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t41 as *mut u64).add(7), t42).write_unaligned(((vgetq_lane_u32::<3>(t40[3]) as u64) << 32 | vgetq_lane_u32::<3>(t40[1]) as u64));
        }
        let t43: [uint32x4_t; 2] = [x9[0], x9[1]];
        let t44: [uint32x4_t; 2] = [vandq_u32(t43[0], vdupq_n_u32(63)), vandq_u32(t43[1], vdupq_n_u32(63))];
        let t45: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(x6[0], vnegq_s32(vreinterpretq_s32_u32(t44[0]))), vshlq_u32(x6[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t44[0])))), vorrq_u32(vshlq_u32(x6[1], vnegq_s32(vreinterpretq_s32_u32(t44[1]))), vshlq_u32(x6[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t44[1])))), vshlq_u32(x6[2], vnegq_s32(vreinterpretq_s32_u32(t44[0]))), vshlq_u32(x6[3], vnegq_s32(vreinterpretq_s32_u32(t44[1])))];
        let t46 = (arg3 as *const u8).add(((gid0.wrapping_add((3u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t46 as *mut u32).add(0), uint32x4x2_t(t45[0], t45[2]));
            vst2q_u32((t46 as *mut u32).add(8), uint32x4x2_t(t45[1], t45[3]));
        } else {
            let mut t47w = 0u64;
            let t47 = core::hint::black_box(&raw mut t47w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t46 as *mut u64).add(0), t47).write_unaligned(((vgetq_lane_u32::<0>(t45[2]) as u64) << 32 | vgetq_lane_u32::<0>(t45[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t46 as *mut u64).add(1), t47).write_unaligned(((vgetq_lane_u32::<1>(t45[2]) as u64) << 32 | vgetq_lane_u32::<1>(t45[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t46 as *mut u64).add(2), t47).write_unaligned(((vgetq_lane_u32::<2>(t45[2]) as u64) << 32 | vgetq_lane_u32::<2>(t45[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t46 as *mut u64).add(3), t47).write_unaligned(((vgetq_lane_u32::<3>(t45[2]) as u64) << 32 | vgetq_lane_u32::<3>(t45[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t46 as *mut u64).add(4), t47).write_unaligned(((vgetq_lane_u32::<0>(t45[3]) as u64) << 32 | vgetq_lane_u32::<0>(t45[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t46 as *mut u64).add(5), t47).write_unaligned(((vgetq_lane_u32::<1>(t45[3]) as u64) << 32 | vgetq_lane_u32::<1>(t45[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t46 as *mut u64).add(6), t47).write_unaligned(((vgetq_lane_u32::<2>(t45[3]) as u64) << 32 | vgetq_lane_u32::<2>(t45[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t46 as *mut u64).add(7), t47).write_unaligned(((vgetq_lane_u32::<3>(t45[3]) as u64) << 32 | vgetq_lane_u32::<3>(t45[1]) as u64));
        }
        let t48: [uint32x4_t; 2] = [x9[0], x9[1]];
        let t49: [uint32x4_t; 2] = [vandq_u32(t48[0], vdupq_n_u32(63)), vandq_u32(t48[1], vdupq_n_u32(63))];
        let t50: [uint32x4_t; 2] = [vcgeq_s32(vreinterpretq_s32_u32(t49[0]), vdupq_n_s32(32)), vcgeq_s32(vreinterpretq_s32_u32(t49[1]), vdupq_n_s32(32))];
        let t51: [uint32x4_t; 4] = [vbslq_u32(t50[0], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t49[0])))), vorrq_u32(vshlq_u32(x6[0], vnegq_s32(vreinterpretq_s32_u32(t49[0]))), vshlq_u32(x6[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t49[0]))))), vbslq_u32(t50[1], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t49[1])))), vorrq_u32(vshlq_u32(x6[1], vnegq_s32(vreinterpretq_s32_u32(t49[1]))), vshlq_u32(x6[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t49[1]))))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vnegq_s32(vreinterpretq_s32_u32(t49[0])))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vnegq_s32(vreinterpretq_s32_u32(t49[1]))))];
        let t52 = (arg3 as *const u8).add(((gid0.wrapping_add((4u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t52 as *mut u32).add(0), uint32x4x2_t(t51[0], t51[2]));
            vst2q_u32((t52 as *mut u32).add(8), uint32x4x2_t(t51[1], t51[3]));
        } else {
            let mut t53w = 0u64;
            let t53 = core::hint::black_box(&raw mut t53w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t52 as *mut u64).add(0), t53).write_unaligned(((vgetq_lane_u32::<0>(t51[2]) as u64) << 32 | vgetq_lane_u32::<0>(t51[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t52 as *mut u64).add(1), t53).write_unaligned(((vgetq_lane_u32::<1>(t51[2]) as u64) << 32 | vgetq_lane_u32::<1>(t51[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t52 as *mut u64).add(2), t53).write_unaligned(((vgetq_lane_u32::<2>(t51[2]) as u64) << 32 | vgetq_lane_u32::<2>(t51[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t52 as *mut u64).add(3), t53).write_unaligned(((vgetq_lane_u32::<3>(t51[2]) as u64) << 32 | vgetq_lane_u32::<3>(t51[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t52 as *mut u64).add(4), t53).write_unaligned(((vgetq_lane_u32::<0>(t51[3]) as u64) << 32 | vgetq_lane_u32::<0>(t51[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t52 as *mut u64).add(5), t53).write_unaligned(((vgetq_lane_u32::<1>(t51[3]) as u64) << 32 | vgetq_lane_u32::<1>(t51[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t52 as *mut u64).add(6), t53).write_unaligned(((vgetq_lane_u32::<2>(t51[3]) as u64) << 32 | vgetq_lane_u32::<2>(t51[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t52 as *mut u64).add(7), t53).write_unaligned(((vgetq_lane_u32::<3>(t51[3]) as u64) << 32 | vgetq_lane_u32::<3>(t51[1]) as u64));
        }
        let t54: [uint32x4_t; 4] = [vshlq_n_u32::<13>(x6[0]), vshlq_n_u32::<13>(x6[1]), vorrq_u32(vshlq_n_u32::<13>(x6[2]), vshrq_n_u32::<19>(x6[0])), vorrq_u32(vshlq_n_u32::<13>(x6[3]), vshrq_n_u32::<19>(x6[1]))];
        let t55: [uint32x4_t; 2] = [vshrq_n_u32::<19>(x7[2]), vshrq_n_u32::<19>(x7[3])];
        let t56: [uint32x4_t; 4] = [vorrq_u32(t54[0], t55[0]), vorrq_u32(t54[1], t55[1]), t54[2], t54[3]];
        let t57 = (arg3 as *const u8).add(((gid0.wrapping_add((5u32).wrapping_mul(x10))) as usize) * 8);
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
        let t59: [uint32x4_t; 4] = [vdupq_n_u32(0), vdupq_n_u32(0), vshlq_n_u32::<8>(x6[0]), vshlq_n_u32::<8>(x6[1])];
        let t60 = (arg3 as *const u8).add(((gid0.wrapping_add((6u32).wrapping_mul(x10))) as usize) * 8);
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
        let t62: [uint32x4_t; 2] = [vshrq_n_u32::<3>(x6[2]), vshrq_n_u32::<3>(x6[3])];
        let t63: [uint32x4_t; 4] = [t62[0], t62[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        let t64 = (arg3 as *const u8).add(((gid0.wrapping_add((7u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t64 as *mut u32).add(0), uint32x4x2_t(t63[0], t63[2]));
            vst2q_u32((t64 as *mut u32).add(8), uint32x4x2_t(t63[1], t63[3]));
        } else {
            let mut t65w = 0u64;
            let t65 = core::hint::black_box(&raw mut t65w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t64 as *mut u64).add(0), t65).write_unaligned(((vgetq_lane_u32::<0>(t63[2]) as u64) << 32 | vgetq_lane_u32::<0>(t63[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t64 as *mut u64).add(1), t65).write_unaligned(((vgetq_lane_u32::<1>(t63[2]) as u64) << 32 | vgetq_lane_u32::<1>(t63[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t64 as *mut u64).add(2), t65).write_unaligned(((vgetq_lane_u32::<2>(t63[2]) as u64) << 32 | vgetq_lane_u32::<2>(t63[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t64 as *mut u64).add(3), t65).write_unaligned(((vgetq_lane_u32::<3>(t63[2]) as u64) << 32 | vgetq_lane_u32::<3>(t63[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t64 as *mut u64).add(4), t65).write_unaligned(((vgetq_lane_u32::<0>(t63[3]) as u64) << 32 | vgetq_lane_u32::<0>(t63[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t64 as *mut u64).add(5), t65).write_unaligned(((vgetq_lane_u32::<1>(t63[3]) as u64) << 32 | vgetq_lane_u32::<1>(t63[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t64 as *mut u64).add(6), t65).write_unaligned(((vgetq_lane_u32::<2>(t63[3]) as u64) << 32 | vgetq_lane_u32::<2>(t63[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t64 as *mut u64).add(7), t65).write_unaligned(((vgetq_lane_u32::<3>(t63[3]) as u64) << 32 | vgetq_lane_u32::<3>(t63[1]) as u64));
        }
        let t66 = vdupq_n_s32(((((40i64) as u32)) & 63) as i32);
        let t67: [uint32x4_t; 2] = [vcgeq_s32(t66, vdupq_n_s32(32)), vcgeq_s32(t66, vdupq_n_s32(32))];
        let t68: [uint32x4_t; 4] = [vbslq_u32(t67[0], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vsubq_s32(vdupq_n_s32(32), t66))), vorrq_u32(vshlq_u32(x6[0], vnegq_s32(t66)), vshlq_u32(x6[2], vsubq_s32(vdupq_n_s32(32), t66)))), vbslq_u32(t67[1], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vsubq_s32(vdupq_n_s32(32), t66))), vorrq_u32(vshlq_u32(x6[1], vnegq_s32(t66)), vshlq_u32(x6[3], vsubq_s32(vdupq_n_s32(32), t66)))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vnegq_s32(t66))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vnegq_s32(t66)))];
        let t69 = (arg3 as *const u8).add(((gid0.wrapping_add((8u32).wrapping_mul(x10))) as usize) * 8);
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
        let t71: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x6[0]), vget_low_u32(x7[0]))), vreinterpretq_u32_u64(vmull_high_u32(x6[0], x7[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x6[1]), vget_low_u32(x7[1]))), vreinterpretq_u32_u64(vmull_high_u32(x6[1], x7[1])))];
        let t72: [uint32x4_t; 4] = [vmulq_u32(x6[0], x7[0]), vmulq_u32(x6[1], x7[1]), vmlaq_u32(vmlaq_u32(t71[0], x6[0], x7[2]), x7[0], x6[2]), vmlaq_u32(vmlaq_u32(t71[1], x6[1], x7[3]), x7[1], x6[3])];
        let t73 = (arg3 as *const u8).add(((gid0.wrapping_add((9u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t73 as *mut u32).add(0), uint32x4x2_t(t72[0], t72[2]));
            vst2q_u32((t73 as *mut u32).add(8), uint32x4x2_t(t72[1], t72[3]));
        } else {
            let mut t74w = 0u64;
            let t74 = core::hint::black_box(&raw mut t74w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t73 as *mut u64).add(0), t74).write_unaligned(((vgetq_lane_u32::<0>(t72[2]) as u64) << 32 | vgetq_lane_u32::<0>(t72[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t73 as *mut u64).add(1), t74).write_unaligned(((vgetq_lane_u32::<1>(t72[2]) as u64) << 32 | vgetq_lane_u32::<1>(t72[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t73 as *mut u64).add(2), t74).write_unaligned(((vgetq_lane_u32::<2>(t72[2]) as u64) << 32 | vgetq_lane_u32::<2>(t72[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t73 as *mut u64).add(3), t74).write_unaligned(((vgetq_lane_u32::<3>(t72[2]) as u64) << 32 | vgetq_lane_u32::<3>(t72[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t73 as *mut u64).add(4), t74).write_unaligned(((vgetq_lane_u32::<0>(t72[3]) as u64) << 32 | vgetq_lane_u32::<0>(t72[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t73 as *mut u64).add(5), t74).write_unaligned(((vgetq_lane_u32::<1>(t72[3]) as u64) << 32 | vgetq_lane_u32::<1>(t72[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t73 as *mut u64).add(6), t74).write_unaligned(((vgetq_lane_u32::<2>(t72[3]) as u64) << 32 | vgetq_lane_u32::<2>(t72[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t73 as *mut u64).add(7), t74).write_unaligned(((vgetq_lane_u32::<3>(t72[3]) as u64) << 32 | vgetq_lane_u32::<3>(t72[1]) as u64));
        }
        let t75: u64 = 0u64;
        let t76 = [vdupq_n_u32((t75) as u32); 2];
        let t77: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x7[0], t76[0]), vceqq_u32(x7[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(x7[1], t76[1]), vceqq_u32(x7[3], vdupq_n_u32(0)))];
        let t78: [uint32x4_t; 2] = [vmvnq_u32(t77[0]), vmvnq_u32(t77[1])];
        let mut t79: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t80: [uint32x4_t; 2] = [vandq_u32(live[0], t78[0]), vandq_u32(live[1], t78[1])];
        let t81: [uint32x4_t; 2] = [vbicq_u32(live[0], t78[0]), vbicq_u32(live[1], t78[1])];
        {
            let (mut t82xl, mut t82xh, mut t82yl, mut t82yh) = ([0u32; 8], [0u32; 8], [0u32; 8], [0u32; 8]);
            vst1q_u32(t82xl.as_mut_ptr().add(0), x6[0]); vst1q_u32(t82xh.as_mut_ptr().add(0), x6[2]); vst1q_u32(t82yl.as_mut_ptr().add(0), x7[0]); vst1q_u32(t82yh.as_mut_ptr().add(0), x7[2]);
            vst1q_u32(t82xl.as_mut_ptr().add(4), x6[1]); vst1q_u32(t82xh.as_mut_ptr().add(4), x6[3]); vst1q_u32(t82yl.as_mut_ptr().add(4), x7[1]); vst1q_u32(t82yh.as_mut_ptr().add(4), x7[3]);
            for j in 0..8 { let q = (((t82xh[j] as u64) << 32 | t82xl[j] as u64) as u64).checked_div(((t82yh[j] as u64) << 32 | t82yl[j] as u64) as u64).unwrap_or(0) as u64; t82xl[j] = q as u32; t82xh[j] = (q >> 32) as u32; }
            let t83: [uint32x4_t; 4] = [vld1q_u32(t82xl.as_ptr().add(0)), vld1q_u32(t82xl.as_ptr().add(4)), vld1q_u32(t82xh.as_ptr().add(0)), vld1q_u32(t82xh.as_ptr().add(4))];
            t79 = [vbslq_u32(t80[0], t83[0], t79[0]), vbslq_u32(t80[1], t83[1], t79[1]), vbslq_u32(t80[0], t83[2], t79[2]), vbslq_u32(t80[1], t83[3], t79[3])];
        }
        {
            let t84: u64 = 7u64;
            let t85 = [vdupq_n_u32((t84) as u32); 2];
            let t86: [uint32x4_t; 4] = [t85[0], t85[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            t79 = [vbslq_u32(t81[0], t86[0], t79[0]), vbslq_u32(t81[1], t86[1], t79[1]), vbslq_u32(t81[0], t86[2], t79[2]), vbslq_u32(t81[1], t86[3], t79[3])];
        }
        let t87 = (arg3 as *const u8).add(((gid0.wrapping_add((10u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t87 as *mut u32).add(0), uint32x4x2_t(t79[0], t79[2]));
            vst2q_u32((t87 as *mut u32).add(8), uint32x4x2_t(t79[1], t79[3]));
        } else {
            let mut t88w = 0u64;
            let t88 = core::hint::black_box(&raw mut t88w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t87 as *mut u64).add(0), t88).write_unaligned(((vgetq_lane_u32::<0>(t79[2]) as u64) << 32 | vgetq_lane_u32::<0>(t79[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t87 as *mut u64).add(1), t88).write_unaligned(((vgetq_lane_u32::<1>(t79[2]) as u64) << 32 | vgetq_lane_u32::<1>(t79[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t87 as *mut u64).add(2), t88).write_unaligned(((vgetq_lane_u32::<2>(t79[2]) as u64) << 32 | vgetq_lane_u32::<2>(t79[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t87 as *mut u64).add(3), t88).write_unaligned(((vgetq_lane_u32::<3>(t79[2]) as u64) << 32 | vgetq_lane_u32::<3>(t79[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t87 as *mut u64).add(4), t88).write_unaligned(((vgetq_lane_u32::<0>(t79[3]) as u64) << 32 | vgetq_lane_u32::<0>(t79[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t87 as *mut u64).add(5), t88).write_unaligned(((vgetq_lane_u32::<1>(t79[3]) as u64) << 32 | vgetq_lane_u32::<1>(t79[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t87 as *mut u64).add(6), t88).write_unaligned(((vgetq_lane_u32::<2>(t79[3]) as u64) << 32 | vgetq_lane_u32::<2>(t79[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t87 as *mut u64).add(7), t88).write_unaligned(((vgetq_lane_u32::<3>(t79[3]) as u64) << 32 | vgetq_lane_u32::<3>(t79[1]) as u64));
        }
        let t89: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x6[2], x7[2]), vandq_u32(vceqq_u32(x6[2], x7[2]), vcltq_u32(x6[0], x7[0]))), vorrq_u32(vcltq_u32(x6[3], x7[3]), vandq_u32(vceqq_u32(x6[3], x7[3]), vcltq_u32(x6[1], x7[1])))];
        let t90: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t89[0]), vshrq_n_u32::<31>(t89[1])];
        let t91: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x7[2], x6[2]), vandq_u32(vceqq_u32(x7[2], x6[2]), vcltq_u32(x7[0], x6[0]))), vorrq_u32(vcltq_u32(x7[3], x6[3]), vandq_u32(vceqq_u32(x7[3], x6[3]), vcltq_u32(x7[1], x6[1])))];
        let t92: [uint32x4_t; 2] = [vmvnq_u32(t91[0]), vmvnq_u32(t91[1])];
        let t93: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t92[0]), vshrq_n_u32::<31>(t92[1])];
        let t94: [uint32x4_t; 4] = [vshlq_n_u32::<1>(t93[0]), vshlq_n_u32::<1>(t93[1]), vshrq_n_u32::<31>(t93[0]), vshrq_n_u32::<31>(t93[1])];
        let t95: [uint32x4_t; 4] = [vorrq_u32(t90[0], t94[0]), vorrq_u32(t90[1], t94[1]), t94[2], t94[3]];
        let t96: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x6[0], x7[0]), vceqq_u32(x6[2], x7[2])), vandq_u32(vceqq_u32(x6[1], x7[1]), vceqq_u32(x6[3], x7[3]))];
        let t97: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t96[0]), vshrq_n_u32::<31>(t96[1])];
        let t98: [uint32x4_t; 4] = [vshlq_n_u32::<2>(t97[0]), vshlq_n_u32::<2>(t97[1]), vshrq_n_u32::<30>(t97[0]), vshrq_n_u32::<30>(t97[1])];
        let t99: [uint32x4_t; 4] = [vorrq_u32(t95[0], t98[0]), vorrq_u32(t95[1], t98[1]), vorrq_u32(t95[2], t98[2]), vorrq_u32(t95[3], t98[3])];
        let t100: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x6[0], x7[0]), vceqq_u32(x6[2], x7[2])), vandq_u32(vceqq_u32(x6[1], x7[1]), vceqq_u32(x6[3], x7[3]))];
        let t101: [uint32x4_t; 2] = [vmvnq_u32(t100[0]), vmvnq_u32(t100[1])];
        let t102: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t101[0]), vshrq_n_u32::<31>(t101[1])];
        let t103: [uint32x4_t; 4] = [vshlq_n_u32::<3>(t102[0]), vshlq_n_u32::<3>(t102[1]), vshrq_n_u32::<29>(t102[0]), vshrq_n_u32::<29>(t102[1])];
        let t104: [uint32x4_t; 4] = [vorrq_u32(t99[0], t103[0]), vorrq_u32(t99[1], t103[1]), vorrq_u32(t99[2], t103[2]), vorrq_u32(t99[3], t103[3])];
        let t105: [uint32x4_t; 2] = [vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x6[2]), vreinterpretq_s32_u32(x7[2])), vandq_u32(vceqq_u32(x6[2], x7[2]), vcltq_u32(x6[0], x7[0]))), vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x6[3]), vreinterpretq_s32_u32(x7[3])), vandq_u32(vceqq_u32(x6[3], x7[3]), vcltq_u32(x6[1], x7[1])))];
        let t106: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t105[0]), vshrq_n_u32::<31>(t105[1])];
        let t107: [uint32x4_t; 4] = [vshlq_n_u32::<4>(t106[0]), vshlq_n_u32::<4>(t106[1]), vshrq_n_u32::<28>(t106[0]), vshrq_n_u32::<28>(t106[1])];
        let t108: [uint32x4_t; 4] = [vorrq_u32(t104[0], t107[0]), vorrq_u32(t104[1], t107[1]), vorrq_u32(t104[2], t107[2]), vorrq_u32(t104[3], t107[3])];
        let t109: [uint32x4_t; 2] = [vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x6[2]), vreinterpretq_s32_u32(x7[2])), vandq_u32(vceqq_u32(x6[2], x7[2]), vcltq_u32(x6[0], x7[0]))), vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x6[3]), vreinterpretq_s32_u32(x7[3])), vandq_u32(vceqq_u32(x6[3], x7[3]), vcltq_u32(x6[1], x7[1])))];
        let t110: [uint32x4_t; 2] = [vmvnq_u32(t109[0]), vmvnq_u32(t109[1])];
        let t111: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t110[0]), vshrq_n_u32::<31>(t110[1])];
        let t112: [uint32x4_t; 4] = [vshlq_n_u32::<5>(t111[0]), vshlq_n_u32::<5>(t111[1]), vshrq_n_u32::<27>(t111[0]), vshrq_n_u32::<27>(t111[1])];
        let t113: [uint32x4_t; 4] = [vorrq_u32(t108[0], t112[0]), vorrq_u32(t108[1], t112[1]), vorrq_u32(t108[2], t112[2]), vorrq_u32(t108[3], t112[3])];
        let t114: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x7[2], x6[2]), vandq_u32(vceqq_u32(x7[2], x6[2]), vcltq_u32(x7[0], x6[0]))), vorrq_u32(vcltq_u32(x7[3], x6[3]), vandq_u32(vceqq_u32(x7[3], x6[3]), vcltq_u32(x7[1], x6[1])))];
        let t115: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t114[0]), vshrq_n_u32::<31>(t114[1])];
        let t116: [uint32x4_t; 4] = [vshlq_n_u32::<6>(t115[0]), vshlq_n_u32::<6>(t115[1]), vshrq_n_u32::<26>(t115[0]), vshrq_n_u32::<26>(t115[1])];
        let t117: [uint32x4_t; 4] = [vorrq_u32(t113[0], t116[0]), vorrq_u32(t113[1], t116[1]), vorrq_u32(t113[2], t116[2]), vorrq_u32(t113[3], t116[3])];
        let t118 = (arg3 as *const u8).add(((gid0.wrapping_add((11u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t118 as *mut u32).add(0), uint32x4x2_t(t117[0], t117[2]));
            vst2q_u32((t118 as *mut u32).add(8), uint32x4x2_t(t117[1], t117[3]));
        } else {
            let mut t119w = 0u64;
            let t119 = core::hint::black_box(&raw mut t119w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t118 as *mut u64).add(0), t119).write_unaligned(((vgetq_lane_u32::<0>(t117[2]) as u64) << 32 | vgetq_lane_u32::<0>(t117[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t118 as *mut u64).add(1), t119).write_unaligned(((vgetq_lane_u32::<1>(t117[2]) as u64) << 32 | vgetq_lane_u32::<1>(t117[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t118 as *mut u64).add(2), t119).write_unaligned(((vgetq_lane_u32::<2>(t117[2]) as u64) << 32 | vgetq_lane_u32::<2>(t117[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t118 as *mut u64).add(3), t119).write_unaligned(((vgetq_lane_u32::<3>(t117[2]) as u64) << 32 | vgetq_lane_u32::<3>(t117[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t118 as *mut u64).add(4), t119).write_unaligned(((vgetq_lane_u32::<0>(t117[3]) as u64) << 32 | vgetq_lane_u32::<0>(t117[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t118 as *mut u64).add(5), t119).write_unaligned(((vgetq_lane_u32::<1>(t117[3]) as u64) << 32 | vgetq_lane_u32::<1>(t117[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t118 as *mut u64).add(6), t119).write_unaligned(((vgetq_lane_u32::<2>(t117[3]) as u64) << 32 | vgetq_lane_u32::<2>(t117[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t118 as *mut u64).add(7), t119).write_unaligned(((vgetq_lane_u32::<3>(t117[3]) as u64) << 32 | vgetq_lane_u32::<3>(t117[1]) as u64));
        }
        let t120: [uint32x4_t; 2] = [vaddq_u32(x8[0], x6[0]), vaddq_u32(x8[1], x6[1])];
        let t121: [uint32x4_t; 2] = [vcltq_u32(t120[0], x8[0]), vcltq_u32(t120[1], x8[1])];
        let t122: [uint32x4_t; 4] = [t120[0], t120[1], vsubq_u32(vaddq_u32(vdupq_n_u32(0), x6[2]), t121[0]), vsubq_u32(vaddq_u32(vdupq_n_u32(0), x6[3]), t121[1])];
        let t123 = (arg3 as *const u8).add(((gid0.wrapping_add((12u32).wrapping_mul(x10))) as usize) * 8);
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
        let t125: [uint32x4_t; 2] = [vreinterpretq_u32_s32(vshrq_n_s32::<31>(vreinterpretq_s32_u32(x8[0]))), vreinterpretq_u32_s32(vshrq_n_s32::<31>(vreinterpretq_s32_u32(x8[1])))];
        let t126: [uint32x4_t; 4] = [x8[0], x8[1], t125[0], t125[1]];
        let t127 = (arg3 as *const u8).add(((gid0.wrapping_add((13u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t127 as *mut u32).add(0), uint32x4x2_t(t126[0], t126[2]));
            vst2q_u32((t127 as *mut u32).add(8), uint32x4x2_t(t126[1], t126[3]));
        } else {
            let mut t128w = 0u64;
            let t128 = core::hint::black_box(&raw mut t128w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t127 as *mut u64).add(0), t128).write_unaligned(((vgetq_lane_u32::<0>(t126[2]) as u64) << 32 | vgetq_lane_u32::<0>(t126[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t127 as *mut u64).add(1), t128).write_unaligned(((vgetq_lane_u32::<1>(t126[2]) as u64) << 32 | vgetq_lane_u32::<1>(t126[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t127 as *mut u64).add(2), t128).write_unaligned(((vgetq_lane_u32::<2>(t126[2]) as u64) << 32 | vgetq_lane_u32::<2>(t126[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t127 as *mut u64).add(3), t128).write_unaligned(((vgetq_lane_u32::<3>(t126[2]) as u64) << 32 | vgetq_lane_u32::<3>(t126[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t127 as *mut u64).add(4), t128).write_unaligned(((vgetq_lane_u32::<0>(t126[3]) as u64) << 32 | vgetq_lane_u32::<0>(t126[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t127 as *mut u64).add(5), t128).write_unaligned(((vgetq_lane_u32::<1>(t126[3]) as u64) << 32 | vgetq_lane_u32::<1>(t126[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t127 as *mut u64).add(6), t128).write_unaligned(((vgetq_lane_u32::<2>(t126[3]) as u64) << 32 | vgetq_lane_u32::<2>(t126[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t127 as *mut u64).add(7), t128).write_unaligned(((vgetq_lane_u32::<3>(t126[3]) as u64) << 32 | vgetq_lane_u32::<3>(t126[1]) as u64));
        }
        let t129: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<3>(x6[0]), vshlq_n_u32::<29>(x6[2])), vorrq_u32(vshrq_n_u32::<3>(x6[1]), vshlq_n_u32::<29>(x6[3])), vshrq_n_u32::<3>(x6[2]), vshrq_n_u32::<3>(x6[3])];
        let t130: [uint32x4_t; 2] = [t129[0], t129[1]];
        let t131: [uint32x4_t; 4] = [t130[0], t130[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        let t132 = (arg3 as *const u8).add(((gid0.wrapping_add((14u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t132 as *mut u32).add(0), uint32x4x2_t(t131[0], t131[2]));
            vst2q_u32((t132 as *mut u32).add(8), uint32x4x2_t(t131[1], t131[3]));
        } else {
            let mut t133w = 0u64;
            let t133 = core::hint::black_box(&raw mut t133w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t132 as *mut u64).add(0), t133).write_unaligned(((vgetq_lane_u32::<0>(t131[2]) as u64) << 32 | vgetq_lane_u32::<0>(t131[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t132 as *mut u64).add(1), t133).write_unaligned(((vgetq_lane_u32::<1>(t131[2]) as u64) << 32 | vgetq_lane_u32::<1>(t131[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t132 as *mut u64).add(2), t133).write_unaligned(((vgetq_lane_u32::<2>(t131[2]) as u64) << 32 | vgetq_lane_u32::<2>(t131[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t132 as *mut u64).add(3), t133).write_unaligned(((vgetq_lane_u32::<3>(t131[2]) as u64) << 32 | vgetq_lane_u32::<3>(t131[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t132 as *mut u64).add(4), t133).write_unaligned(((vgetq_lane_u32::<0>(t131[3]) as u64) << 32 | vgetq_lane_u32::<0>(t131[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t132 as *mut u64).add(5), t133).write_unaligned(((vgetq_lane_u32::<1>(t131[3]) as u64) << 32 | vgetq_lane_u32::<1>(t131[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t132 as *mut u64).add(6), t133).write_unaligned(((vgetq_lane_u32::<2>(t131[3]) as u64) << 32 | vgetq_lane_u32::<2>(t131[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t132 as *mut u64).add(7), t133).write_unaligned(((vgetq_lane_u32::<3>(t131[3]) as u64) << 32 | vgetq_lane_u32::<3>(t131[1]) as u64));
        }
        let t134: u64 = 1u64;
        let t135 = [vdupq_n_u32((t134) as u32); 2];
        let t136: [uint32x4_t; 2] = [vandq_u32(x6[0], t135[0]), vandq_u32(x6[1], t135[1])];
        let t137: [uint32x4_t; 2] = [vtstq_u32(t136[0], t136[0]), vtstq_u32(t136[1], t136[1])];
        let t138: [uint32x4_t; 2] = [vandq_u32(live[0], t137[0]), vandq_u32(live[1], t137[1])];
        {
            let t139: [uint32x4_t; 4] = [veorq_u32(x6[0], x7[0]), veorq_u32(x6[1], x7[1]), veorq_u32(x6[2], x7[2]), veorq_u32(x6[3], x7[3])];
            x11 = [vbslq_u32(t138[0], t139[0], x11[0]), vbslq_u32(t138[1], t139[1], x11[1]), vbslq_u32(t138[0], t139[2], x11[2]), vbslq_u32(t138[1], t139[3], x11[3])];
        }
        let t140: [uint32x4_t; 2] = [vbicq_u32(live[0], t137[0]), vbicq_u32(live[1], t137[1])];
        {
            let t141: [uint32x4_t; 2] = [vaddq_u32(x6[0], x8[0]), vaddq_u32(x6[1], x8[1])];
            let t142: [uint32x4_t; 2] = [vcltq_u32(t141[0], x6[0]), vcltq_u32(t141[1], x6[1])];
            let t143: [uint32x4_t; 4] = [t141[0], t141[1], vsubq_u32(vaddq_u32(x6[2], vdupq_n_u32(0)), t142[0]), vsubq_u32(vaddq_u32(x6[3], vdupq_n_u32(0)), t142[1])];
            x11 = [vbslq_u32(t140[0], t143[0], x11[0]), vbslq_u32(t140[1], t143[1], x11[1]), vbslq_u32(t140[0], t143[2], x11[2]), vbslq_u32(t140[1], t143[3], x11[3])];
        }
        let t144 = (arg3 as *const u8).add(((gid0.wrapping_add((15u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t144 as *mut u32).add(0), uint32x4x2_t(x11[0], x11[2]));
            vst2q_u32((t144 as *mut u32).add(8), uint32x4x2_t(x11[1], x11[3]));
        } else {
            let mut t145w = 0u64;
            let t145 = core::hint::black_box(&raw mut t145w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t144 as *mut u64).add(0), t145).write_unaligned(((vgetq_lane_u32::<0>(x11[2]) as u64) << 32 | vgetq_lane_u32::<0>(x11[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t144 as *mut u64).add(1), t145).write_unaligned(((vgetq_lane_u32::<1>(x11[2]) as u64) << 32 | vgetq_lane_u32::<1>(x11[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t144 as *mut u64).add(2), t145).write_unaligned(((vgetq_lane_u32::<2>(x11[2]) as u64) << 32 | vgetq_lane_u32::<2>(x11[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t144 as *mut u64).add(3), t145).write_unaligned(((vgetq_lane_u32::<3>(x11[2]) as u64) << 32 | vgetq_lane_u32::<3>(x11[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t144 as *mut u64).add(4), t145).write_unaligned(((vgetq_lane_u32::<0>(x11[3]) as u64) << 32 | vgetq_lane_u32::<0>(x11[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t144 as *mut u64).add(5), t145).write_unaligned(((vgetq_lane_u32::<1>(x11[3]) as u64) << 32 | vgetq_lane_u32::<1>(x11[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t144 as *mut u64).add(6), t145).write_unaligned(((vgetq_lane_u32::<2>(x11[3]) as u64) << 32 | vgetq_lane_u32::<2>(x11[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t144 as *mut u64).add(7), t145).write_unaligned(((vgetq_lane_u32::<3>(x11[3]) as u64) << 32 | vgetq_lane_u32::<3>(x11[1]) as u64));
        }
        let t146 = b1_l_p(l, lid0, live, x6);
        let t147 = b2_l_p(l, lid0, live, x7);
        let t148: [uint32x4_t; 4] = [vshlq_n_u32::<8>(t147[0]), vshlq_n_u32::<8>(t147[1]), vshrq_n_u32::<24>(t147[0]), vshrq_n_u32::<24>(t147[1])];
        let t149: [uint32x4_t; 2] = [vaddq_u32(t146[0], t148[0]), vaddq_u32(t146[1], t148[1])];
        let t150: [uint32x4_t; 2] = [vcltq_u32(t149[0], t146[0]), vcltq_u32(t149[1], t146[1])];
        let t151: [uint32x4_t; 4] = [t149[0], t149[1], vsubq_u32(vaddq_u32(vdupq_n_u32(0), t148[2]), t150[0]), vsubq_u32(vaddq_u32(vdupq_n_u32(0), t148[3]), t150[1])];
        let t152 = (arg3 as *const u8).add(((gid0.wrapping_add((16u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t152 as *mut u32).add(0), uint32x4x2_t(t151[0], t151[2]));
            vst2q_u32((t152 as *mut u32).add(8), uint32x4x2_t(t151[1], t151[3]));
        } else {
            let mut t153w = 0u64;
            let t153 = core::hint::black_box(&raw mut t153w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t152 as *mut u64).add(0), t153).write_unaligned(((vgetq_lane_u32::<0>(t151[2]) as u64) << 32 | vgetq_lane_u32::<0>(t151[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t152 as *mut u64).add(1), t153).write_unaligned(((vgetq_lane_u32::<1>(t151[2]) as u64) << 32 | vgetq_lane_u32::<1>(t151[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t152 as *mut u64).add(2), t153).write_unaligned(((vgetq_lane_u32::<2>(t151[2]) as u64) << 32 | vgetq_lane_u32::<2>(t151[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t152 as *mut u64).add(3), t153).write_unaligned(((vgetq_lane_u32::<3>(t151[2]) as u64) << 32 | vgetq_lane_u32::<3>(t151[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t152 as *mut u64).add(4), t153).write_unaligned(((vgetq_lane_u32::<0>(t151[3]) as u64) << 32 | vgetq_lane_u32::<0>(t151[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t152 as *mut u64).add(5), t153).write_unaligned(((vgetq_lane_u32::<1>(t151[3]) as u64) << 32 | vgetq_lane_u32::<1>(t151[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t152 as *mut u64).add(6), t153).write_unaligned(((vgetq_lane_u32::<2>(t151[3]) as u64) << 32 | vgetq_lane_u32::<2>(t151[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t152 as *mut u64).add(7), t153).write_unaligned(((vgetq_lane_u32::<3>(t151[3]) as u64) << 32 | vgetq_lane_u32::<3>(t151[1]) as u64));
        }
        let t154 = [vdupq_n_u32(x10); 2];
        let mut t155x = [0u32; 8];
        let mut t155y = [0u32; 8];
        for r in 0..2 { vst1q_u32(t155x.as_mut_ptr().add(4 * r), x8[r]); vst1q_u32(t155y.as_mut_ptr().add(4 * r), t154[r]); }
        for j in 0..8 { t155x[j] = (t155x[j] as u32).checked_rem(t155y[j] as u32).unwrap_or(0) as u32; }
        let t156: [uint32x4_t; 2] = [vld1q_u32(t155x.as_ptr().add(0)), vld1q_u32(t155x.as_ptr().add(4))];
        let t157 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t156[0]), vandq_u32(live[1], t156[1]))));
        let t158: [uint32x4_t; 2] = [vbslq_u32(live[0], t156[0], t157), vbslq_u32(live[1], t156[1], t157)];
        let mut t159: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t160 = vminvq_u32(vminq_u32(t158[0], t158[1]));
        if t160 == vmaxvq_u32(vmaxq_u32(t158[0], t158[1])) {
            let t161 = ((arg0 as *const u8).add(t160 as usize * 8) as *const u64).read_unaligned();
            t159 = [vdupq_n_u32(t161 as u32), vdupq_n_u32(t161 as u32), vdupq_n_u32((t161 >> 32) as u32), vdupq_n_u32((t161 >> 32) as u32)];
        } else {
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t158[0]) as usize * 8) as *const u64).read_unaligned(); t159[0] = vsetq_lane_u32::<0>(v as u32, t159[0]); t159[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t159[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t158[0]) as usize * 8) as *const u64).read_unaligned(); t159[0] = vsetq_lane_u32::<1>(v as u32, t159[0]); t159[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t159[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t158[0]) as usize * 8) as *const u64).read_unaligned(); t159[0] = vsetq_lane_u32::<2>(v as u32, t159[0]); t159[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t159[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t158[0]) as usize * 8) as *const u64).read_unaligned(); t159[0] = vsetq_lane_u32::<3>(v as u32, t159[0]); t159[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t159[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t158[1]) as usize * 8) as *const u64).read_unaligned(); t159[1] = vsetq_lane_u32::<0>(v as u32, t159[1]); t159[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t159[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t158[1]) as usize * 8) as *const u64).read_unaligned(); t159[1] = vsetq_lane_u32::<1>(v as u32, t159[1]); t159[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t159[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t158[1]) as usize * 8) as *const u64).read_unaligned(); t159[1] = vsetq_lane_u32::<2>(v as u32, t159[1]); t159[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t159[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t158[1]) as usize * 8) as *const u64).read_unaligned(); t159[1] = vsetq_lane_u32::<3>(v as u32, t159[1]); t159[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t159[3]); }
        }
        let t162 = (arg3 as *const u8).add(((gid0.wrapping_add((17u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t162 as *mut u32).add(0), uint32x4x2_t(t159[0], t159[2]));
            vst2q_u32((t162 as *mut u32).add(8), uint32x4x2_t(t159[1], t159[3]));
        } else {
            let mut t163w = 0u64;
            let t163 = core::hint::black_box(&raw mut t163w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t162 as *mut u64).add(0), t163).write_unaligned(((vgetq_lane_u32::<0>(t159[2]) as u64) << 32 | vgetq_lane_u32::<0>(t159[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t162 as *mut u64).add(1), t163).write_unaligned(((vgetq_lane_u32::<1>(t159[2]) as u64) << 32 | vgetq_lane_u32::<1>(t159[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t162 as *mut u64).add(2), t163).write_unaligned(((vgetq_lane_u32::<2>(t159[2]) as u64) << 32 | vgetq_lane_u32::<2>(t159[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t162 as *mut u64).add(3), t163).write_unaligned(((vgetq_lane_u32::<3>(t159[2]) as u64) << 32 | vgetq_lane_u32::<3>(t159[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t162 as *mut u64).add(4), t163).write_unaligned(((vgetq_lane_u32::<0>(t159[3]) as u64) << 32 | vgetq_lane_u32::<0>(t159[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t162 as *mut u64).add(5), t163).write_unaligned(((vgetq_lane_u32::<1>(t159[3]) as u64) << 32 | vgetq_lane_u32::<1>(t159[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t162 as *mut u64).add(6), t163).write_unaligned(((vgetq_lane_u32::<2>(t159[3]) as u64) << 32 | vgetq_lane_u32::<2>(t159[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t162 as *mut u64).add(7), t163).write_unaligned(((vgetq_lane_u32::<3>(t159[3]) as u64) << 32 | vgetq_lane_u32::<3>(t159[1]) as u64));
        }
        let t164 = b3_ll_p(l, lid0, live, x6, x8);
        let t165 = (arg3 as *const u8).add(((gid0.wrapping_add((18u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t165 as *mut u32).add(0), uint32x4x2_t(t164[0], t164[2]));
            vst2q_u32((t165 as *mut u32).add(8), uint32x4x2_t(t164[1], t164[3]));
        } else {
            let mut t166w = 0u64;
            let t166 = core::hint::black_box(&raw mut t166w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t165 as *mut u64).add(0), t166).write_unaligned(((vgetq_lane_u32::<0>(t164[2]) as u64) << 32 | vgetq_lane_u32::<0>(t164[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t165 as *mut u64).add(1), t166).write_unaligned(((vgetq_lane_u32::<1>(t164[2]) as u64) << 32 | vgetq_lane_u32::<1>(t164[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t165 as *mut u64).add(2), t166).write_unaligned(((vgetq_lane_u32::<2>(t164[2]) as u64) << 32 | vgetq_lane_u32::<2>(t164[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t165 as *mut u64).add(3), t166).write_unaligned(((vgetq_lane_u32::<3>(t164[2]) as u64) << 32 | vgetq_lane_u32::<3>(t164[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t165 as *mut u64).add(4), t166).write_unaligned(((vgetq_lane_u32::<0>(t164[3]) as u64) << 32 | vgetq_lane_u32::<0>(t164[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t165 as *mut u64).add(5), t166).write_unaligned(((vgetq_lane_u32::<1>(t164[3]) as u64) << 32 | vgetq_lane_u32::<1>(t164[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t165 as *mut u64).add(6), t166).write_unaligned(((vgetq_lane_u32::<2>(t164[3]) as u64) << 32 | vgetq_lane_u32::<2>(t164[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t165 as *mut u64).add(7), t166).write_unaligned(((vgetq_lane_u32::<3>(t164[3]) as u64) << 32 | vgetq_lane_u32::<3>(t164[1]) as u64));
        }
        x12 = x6;
        x13 = 0u32;
        let mut t171 = live;
        loop {
            let t173 = bits(&t171);
            if t173.count_ones() <= 2 {
                let mut t174 = t173;
                while t174 != 0 {
                    let j = t174.trailing_zeros() as usize;
                    t174 &= t174 - 1;
                    let t175 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x7);
                    let mut x7_j: u64 = ((t175[8 + j] as u64) << 32 | t175[j] as u64);
                    let t176 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x8);
                    let mut x8_j: u32 = t176[j];
                    let t177 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x12);
                    let mut x12_j: u64 = ((t177[8 + j] as u64) << 32 | t177[j] as u64);
                    let mut x13_j: u32 = x13;
                    loop {
                        if !(((x13_j) < (((x8_j) & (7u32))))) { break; }
                        x12_j = ((x12_j).wrapping_mul(3u64)).wrapping_add(x7_j);
                        let t178 = x13_j;
                        x13_j = (x13_j).wrapping_add(1u32);
                    }
                    let mut t179 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x12);
                    t179[j] = x12_j as u32;
                    t179[8 + j] = (x12_j as u64 >> 32) as u32;
                    x12 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t179);
                }
                break;
            }
            // Lanes part: each its own rounds.
            let t180 = [vdupq_n_u32(7u32); 2];
            let t181: [uint32x4_t; 2] = [vandq_u32(x8[0], t180[0]), vandq_u32(x8[1], t180[1])];
            let t182 = [vdupq_n_u32(x13); 2];
            let t183: [uint32x4_t; 2] = [vcltq_u32(t182[0], t181[0]), vcltq_u32(t182[1], t181[1])];
            t171 = [vandq_u32(t171[0], t183[0]), vandq_u32(t171[1], t183[1])];
            if !(any(&t171)) { break; }
            let t185: u64 = 3u64;
            let t186 = [vdupq_n_u32((t185) as u32); 2];
            let t187: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x12[0]), vget_low_u32(t186[0]))), vreinterpretq_u32_u64(vmull_high_u32(x12[0], t186[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x12[1]), vget_low_u32(t186[1]))), vreinterpretq_u32_u64(vmull_high_u32(x12[1], t186[1])))];
            let t188: [uint32x4_t; 4] = [vmulq_u32(x12[0], t186[0]), vmulq_u32(x12[1], t186[1]), vmlaq_u32(t187[0], t186[0], x12[2]), vmlaq_u32(t187[1], t186[1], x12[3])];
            let t189: [uint32x4_t; 2] = [vaddq_u32(t188[0], x7[0]), vaddq_u32(t188[1], x7[1])];
            let t190: [uint32x4_t; 2] = [vcltq_u32(t189[0], t188[0]), vcltq_u32(t189[1], t188[1])];
            let t191: [uint32x4_t; 4] = [t189[0], t189[1], vsubq_u32(vaddq_u32(t188[2], x7[2]), t190[0]), vsubq_u32(vaddq_u32(t188[3], x7[3]), t190[1])];
            x12 = [vbslq_u32(t171[0], t191[0], x12[0]), vbslq_u32(t171[1], t191[1], x12[1]), vbslq_u32(t171[0], t191[2], x12[2]), vbslq_u32(t171[1], t191[3], x12[3])];
            let t192: [uint32x4_t; 2] = [vandq_u32(t171[0], live[0]), vandq_u32(t171[1], live[1])];
            let t193 = x13;
            x13 = (x13).wrapping_add(1u32);
        }
        let t194 = (arg3 as *const u8).add(((gid0.wrapping_add((19u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t194 as *mut u32).add(0), uint32x4x2_t(x12[0], x12[2]));
            vst2q_u32((t194 as *mut u32).add(8), uint32x4x2_t(x12[1], x12[3]));
        } else {
            let mut t195w = 0u64;
            let t195 = core::hint::black_box(&raw mut t195w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t194 as *mut u64).add(0), t195).write_unaligned(((vgetq_lane_u32::<0>(x12[2]) as u64) << 32 | vgetq_lane_u32::<0>(x12[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t194 as *mut u64).add(1), t195).write_unaligned(((vgetq_lane_u32::<1>(x12[2]) as u64) << 32 | vgetq_lane_u32::<1>(x12[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t194 as *mut u64).add(2), t195).write_unaligned(((vgetq_lane_u32::<2>(x12[2]) as u64) << 32 | vgetq_lane_u32::<2>(x12[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t194 as *mut u64).add(3), t195).write_unaligned(((vgetq_lane_u32::<3>(x12[2]) as u64) << 32 | vgetq_lane_u32::<3>(x12[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t194 as *mut u64).add(4), t195).write_unaligned(((vgetq_lane_u32::<0>(x12[3]) as u64) << 32 | vgetq_lane_u32::<0>(x12[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t194 as *mut u64).add(5), t195).write_unaligned(((vgetq_lane_u32::<1>(x12[3]) as u64) << 32 | vgetq_lane_u32::<1>(x12[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t194 as *mut u64).add(6), t195).write_unaligned(((vgetq_lane_u32::<2>(x12[3]) as u64) << 32 | vgetq_lane_u32::<2>(x12[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t194 as *mut u64).add(7), t195).write_unaligned(((vgetq_lane_u32::<3>(x12[3]) as u64) << 32 | vgetq_lane_u32::<3>(x12[1]) as u64));
        }
        let t196: u64 = a1;
        let t197 = [vdupq_n_u32((t196) as u32); 2];
        let t198 = [vdupq_n_u32(((t196) >> 32) as u32); 2];
        let t199: [uint32x4_t; 2] = [vaddq_u32(x6[0], t197[0]), vaddq_u32(x6[1], t197[1])];
        let t200: [uint32x4_t; 2] = [vcltq_u32(t199[0], x6[0]), vcltq_u32(t199[1], x6[1])];
        let t201: [uint32x4_t; 4] = [t199[0], t199[1], vsubq_u32(vaddq_u32(x6[2], t198[0]), t200[0]), vsubq_u32(vaddq_u32(x6[3], t198[1]), t200[1])];
        let t202: u64 = (a1).wrapping_shl((3u64) as u32);
        let t203 = [vdupq_n_u32((t202) as u32); 2];
        let t204 = [vdupq_n_u32(((t202) >> 32) as u32); 2];
        let t205: [uint32x4_t; 2] = [vaddq_u32(t201[0], t203[0]), vaddq_u32(t201[1], t203[1])];
        let t206: [uint32x4_t; 2] = [vcltq_u32(t205[0], t201[0]), vcltq_u32(t205[1], t201[1])];
        let t207: [uint32x4_t; 4] = [t205[0], t205[1], vsubq_u32(vaddq_u32(t201[2], t204[0]), t206[0]), vsubq_u32(vaddq_u32(t201[3], t204[1]), t206[1])];
        let t208 = (arg3 as *const u8).add(((gid0.wrapping_add((20u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t208 as *mut u32).add(0), uint32x4x2_t(t207[0], t207[2]));
            vst2q_u32((t208 as *mut u32).add(8), uint32x4x2_t(t207[1], t207[3]));
        } else {
            let mut t209w = 0u64;
            let t209 = core::hint::black_box(&raw mut t209w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t208 as *mut u64).add(0), t209).write_unaligned(((vgetq_lane_u32::<0>(t207[2]) as u64) << 32 | vgetq_lane_u32::<0>(t207[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t208 as *mut u64).add(1), t209).write_unaligned(((vgetq_lane_u32::<1>(t207[2]) as u64) << 32 | vgetq_lane_u32::<1>(t207[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t208 as *mut u64).add(2), t209).write_unaligned(((vgetq_lane_u32::<2>(t207[2]) as u64) << 32 | vgetq_lane_u32::<2>(t207[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t208 as *mut u64).add(3), t209).write_unaligned(((vgetq_lane_u32::<3>(t207[2]) as u64) << 32 | vgetq_lane_u32::<3>(t207[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t208 as *mut u64).add(4), t209).write_unaligned(((vgetq_lane_u32::<0>(t207[3]) as u64) << 32 | vgetq_lane_u32::<0>(t207[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t208 as *mut u64).add(5), t209).write_unaligned(((vgetq_lane_u32::<1>(t207[3]) as u64) << 32 | vgetq_lane_u32::<1>(t207[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t208 as *mut u64).add(6), t209).write_unaligned(((vgetq_lane_u32::<2>(t207[3]) as u64) << 32 | vgetq_lane_u32::<2>(t207[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t208 as *mut u64).add(7), t209).write_unaligned(((vgetq_lane_u32::<3>(t207[3]) as u64) << 32 | vgetq_lane_u32::<3>(t207[1]) as u64));
        }
        let t210: u64 = 4294967295u64;
        let t211 = [vdupq_n_u32((t210) as u32); 2];
        let t212: [uint32x4_t; 2] = [vandq_u32(x6[0], t211[0]), vandq_u32(x6[1], t211[1])];
        let t213: u64 = 4294967295u64;
        let t214 = [vdupq_n_u32((t213) as u32); 2];
        let t215: [uint32x4_t; 2] = [vandq_u32(x7[0], t214[0]), vandq_u32(x7[1], t214[1])];
        let t216: [uint32x4_t; 2] = [vceqq_u32(t212[0], t215[0]), vceqq_u32(t212[1], t215[1])];
        let mut t217: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t218: [uint32x4_t; 2] = [vandq_u32(live[0], t216[0]), vandq_u32(live[1], t216[1])];
        let t219: [uint32x4_t; 2] = [vbicq_u32(live[0], t216[0]), vbicq_u32(live[1], t216[1])];
        {
            let t220 = [vdupq_n_u32(1u32); 2];
            t217 = [vbslq_u32(t218[0], t220[0], t217[0]), vbslq_u32(t218[1], t220[1], t217[1])];
        }
        {
            let t221 = [vdupq_n_u32(2u32); 2];
            t217 = [vbslq_u32(t219[0], t221[0], t217[0]), vbslq_u32(t219[1], t221[1], t217[1])];
        }
        let t222: [uint32x4_t; 4] = [t217[0], t217[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        let t223 = (arg3 as *const u8).add(((gid0.wrapping_add((21u32).wrapping_mul(x10))) as usize) * 8);
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
        let t225: [uint32x4_t; 2] = [vtstq_u32(vorrq_u32(x6[0], x6[2]), vorrq_u32(x6[0], x6[2])), vtstq_u32(vorrq_u32(x6[1], x6[3]), vorrq_u32(x6[1], x6[3]))];
        let mut t226: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t227: [uint32x4_t; 2] = [vandq_u32(live[0], t225[0]), vandq_u32(live[1], t225[1])];
        let t228: [uint32x4_t; 2] = [vbicq_u32(live[0], t225[0]), vbicq_u32(live[1], t225[1])];
        {
            let t229 = [vdupq_n_u32(5u32); 2];
            t226 = [vbslq_u32(t227[0], t229[0], t226[0]), vbslq_u32(t227[1], t229[1], t226[1])];
        }
        {
            let t230 = [vdupq_n_u32(6u32); 2];
            t226 = [vbslq_u32(t228[0], t230[0], t226[0]), vbslq_u32(t228[1], t230[1], t226[1])];
        }
        let t231: [uint32x4_t; 4] = [t226[0], t226[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        let t232 = (arg3 as *const u8).add(((gid0.wrapping_add((22u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t232 as *mut u32).add(0), uint32x4x2_t(t231[0], t231[2]));
            vst2q_u32((t232 as *mut u32).add(8), uint32x4x2_t(t231[1], t231[3]));
        } else {
            let mut t233w = 0u64;
            let t233 = core::hint::black_box(&raw mut t233w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t232 as *mut u64).add(0), t233).write_unaligned(((vgetq_lane_u32::<0>(t231[2]) as u64) << 32 | vgetq_lane_u32::<0>(t231[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t232 as *mut u64).add(1), t233).write_unaligned(((vgetq_lane_u32::<1>(t231[2]) as u64) << 32 | vgetq_lane_u32::<1>(t231[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t232 as *mut u64).add(2), t233).write_unaligned(((vgetq_lane_u32::<2>(t231[2]) as u64) << 32 | vgetq_lane_u32::<2>(t231[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t232 as *mut u64).add(3), t233).write_unaligned(((vgetq_lane_u32::<3>(t231[2]) as u64) << 32 | vgetq_lane_u32::<3>(t231[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t232 as *mut u64).add(4), t233).write_unaligned(((vgetq_lane_u32::<0>(t231[3]) as u64) << 32 | vgetq_lane_u32::<0>(t231[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t232 as *mut u64).add(5), t233).write_unaligned(((vgetq_lane_u32::<1>(t231[3]) as u64) << 32 | vgetq_lane_u32::<1>(t231[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t232 as *mut u64).add(6), t233).write_unaligned(((vgetq_lane_u32::<2>(t231[3]) as u64) << 32 | vgetq_lane_u32::<2>(t231[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t232 as *mut u64).add(7), t233).write_unaligned(((vgetq_lane_u32::<3>(t231[3]) as u64) << 32 | vgetq_lane_u32::<3>(t231[1]) as u64));
        }
        let t234 = b4_ll_p(l, lid0, live, x6, x7);
        let t235 = (arg3 as *const u8).add(((gid0.wrapping_add((23u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t235 as *mut u32).add(0), uint32x4x2_t(t234[0], t234[2]));
            vst2q_u32((t235 as *mut u32).add(8), uint32x4x2_t(t234[1], t234[3]));
        } else {
            let mut t236w = 0u64;
            let t236 = core::hint::black_box(&raw mut t236w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t235 as *mut u64).add(0), t236).write_unaligned(((vgetq_lane_u32::<0>(t234[2]) as u64) << 32 | vgetq_lane_u32::<0>(t234[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t235 as *mut u64).add(1), t236).write_unaligned(((vgetq_lane_u32::<1>(t234[2]) as u64) << 32 | vgetq_lane_u32::<1>(t234[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t235 as *mut u64).add(2), t236).write_unaligned(((vgetq_lane_u32::<2>(t234[2]) as u64) << 32 | vgetq_lane_u32::<2>(t234[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t235 as *mut u64).add(3), t236).write_unaligned(((vgetq_lane_u32::<3>(t234[2]) as u64) << 32 | vgetq_lane_u32::<3>(t234[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t235 as *mut u64).add(4), t236).write_unaligned(((vgetq_lane_u32::<0>(t234[3]) as u64) << 32 | vgetq_lane_u32::<0>(t234[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t235 as *mut u64).add(5), t236).write_unaligned(((vgetq_lane_u32::<1>(t234[3]) as u64) << 32 | vgetq_lane_u32::<1>(t234[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t235 as *mut u64).add(6), t236).write_unaligned(((vgetq_lane_u32::<2>(t234[3]) as u64) << 32 | vgetq_lane_u32::<2>(t234[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t235 as *mut u64).add(7), t236).write_unaligned(((vgetq_lane_u32::<3>(t234[3]) as u64) << 32 | vgetq_lane_u32::<3>(t234[1]) as u64));
        }
        let t237: u32;
        if ((((a1) & (1u64))) != (0u64)) {
            t237 = 8u32;
        } else {
            t237 = 4u32;
        }
        x14 = t237;
        let t238 = b5_r0ns_p(l, lid0, live, arg0, gid0.wrapping_mul(x14), x14, 8u32);
        let t239 = (arg3 as *const u8).add(((gid0.wrapping_add((24u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t239 as *mut u32).add(0), uint32x4x2_t(t238[0], t238[2]));
            vst2q_u32((t239 as *mut u32).add(8), uint32x4x2_t(t238[1], t238[3]));
        } else {
            let mut t240w = 0u64;
            let t240 = core::hint::black_box(&raw mut t240w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t239 as *mut u64).add(0), t240).write_unaligned(((vgetq_lane_u32::<0>(t238[2]) as u64) << 32 | vgetq_lane_u32::<0>(t238[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t239 as *mut u64).add(1), t240).write_unaligned(((vgetq_lane_u32::<1>(t238[2]) as u64) << 32 | vgetq_lane_u32::<1>(t238[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t239 as *mut u64).add(2), t240).write_unaligned(((vgetq_lane_u32::<2>(t238[2]) as u64) << 32 | vgetq_lane_u32::<2>(t238[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t239 as *mut u64).add(3), t240).write_unaligned(((vgetq_lane_u32::<3>(t238[2]) as u64) << 32 | vgetq_lane_u32::<3>(t238[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t239 as *mut u64).add(4), t240).write_unaligned(((vgetq_lane_u32::<0>(t238[3]) as u64) << 32 | vgetq_lane_u32::<0>(t238[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t239 as *mut u64).add(5), t240).write_unaligned(((vgetq_lane_u32::<1>(t238[3]) as u64) << 32 | vgetq_lane_u32::<1>(t238[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t239 as *mut u64).add(6), t240).write_unaligned(((vgetq_lane_u32::<2>(t238[3]) as u64) << 32 | vgetq_lane_u32::<2>(t238[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t239 as *mut u64).add(7), t240).write_unaligned(((vgetq_lane_u32::<3>(t238[3]) as u64) << 32 | vgetq_lane_u32::<3>(t238[1]) as u64));
        }
        let t241 = if (gid0.wrapping_add(3u32) & 255u32) <= 248u32 { b6_u8_p(l, lid0, live, gid0.wrapping_add(3u32)) } else { b6_i_p(l, lid0, live, gid0.wrapping_add(3u32)) };
        let t242 = (arg3 as *const u8).add(((gid0.wrapping_add((25u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t242 as *mut u32).add(0), uint32x4x2_t(t241[0], t241[2]));
            vst2q_u32((t242 as *mut u32).add(8), uint32x4x2_t(t241[1], t241[3]));
        } else {
            let mut t243w = 0u64;
            let t243 = core::hint::black_box(&raw mut t243w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t242 as *mut u64).add(0), t243).write_unaligned(((vgetq_lane_u32::<0>(t241[2]) as u64) << 32 | vgetq_lane_u32::<0>(t241[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t242 as *mut u64).add(1), t243).write_unaligned(((vgetq_lane_u32::<1>(t241[2]) as u64) << 32 | vgetq_lane_u32::<1>(t241[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t242 as *mut u64).add(2), t243).write_unaligned(((vgetq_lane_u32::<2>(t241[2]) as u64) << 32 | vgetq_lane_u32::<2>(t241[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t242 as *mut u64).add(3), t243).write_unaligned(((vgetq_lane_u32::<3>(t241[2]) as u64) << 32 | vgetq_lane_u32::<3>(t241[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t242 as *mut u64).add(4), t243).write_unaligned(((vgetq_lane_u32::<0>(t241[3]) as u64) << 32 | vgetq_lane_u32::<0>(t241[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t242 as *mut u64).add(5), t243).write_unaligned(((vgetq_lane_u32::<1>(t241[3]) as u64) << 32 | vgetq_lane_u32::<1>(t241[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t242 as *mut u64).add(6), t243).write_unaligned(((vgetq_lane_u32::<2>(t241[3]) as u64) << 32 | vgetq_lane_u32::<2>(t241[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t242 as *mut u64).add(7), t243).write_unaligned(((vgetq_lane_u32::<3>(t241[3]) as u64) << 32 | vgetq_lane_u32::<3>(t241[1]) as u64));
        }
        let t244: u64 = 3u64;
        let t245 = [vdupq_n_u32((t244) as u32); 2];
        let t246: [uint32x4_t; 2] = [vandq_u32(x6[0], t245[0]), vandq_u32(x6[1], t245[1])];
        let t247: u64 = 0u64;
        let t248 = [vdupq_n_u32((t247) as u32); 2];
        let t249: [uint32x4_t; 2] = [vceqq_u32(t246[0], t248[0]), vceqq_u32(t246[1], t248[1])];
        let mut t250: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t251: [uint32x4_t; 2] = [vandq_u32(live[0], t249[0]), vandq_u32(live[1], t249[1])];
        let t252: [uint32x4_t; 2] = [vbicq_u32(live[0], t249[0]), vbicq_u32(live[1], t249[1])];
        if any(&t251) {
            let t253: [uint32x4_t; 4];
            let t254 = bits(&t251);
            if t254.count_ones() <= 2 {
                let t255 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x7);
                let t256 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x8);
                let mut t257 = [0u32; 16];
                let mut t258 = t254;
                while t258 != 0 {
                    let j = t258.trailing_zeros() as usize;
                    t258 &= t258 - 1;
                    let t259 = b7_ss_f(l, lid0, ((t255[8 + j] as u64) << 32 | t255[j] as u64), t256[j]);
                    let t260: u64 = t259;
                    t257[j] = t260 as u32;
                    t257[8 + j] = (t260 >> 32) as u32;
                }
                t253 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t257);
            } else {
                let t261 = b7_ll_p(l, lid0, t251, x7, x8);
                t253 = t261;
            }
            t250 = [vbslq_u32(t251[0], t253[0], t250[0]), vbslq_u32(t251[1], t253[1], t250[1]), vbslq_u32(t251[0], t253[2], t250[2]), vbslq_u32(t251[1], t253[3], t250[3])];
        }
        {
            t250 = [vbslq_u32(t252[0], x6[0], t250[0]), vbslq_u32(t252[1], x6[1], t250[1]), vbslq_u32(t252[0], x6[2], t250[2]), vbslq_u32(t252[1], x6[3], t250[3])];
        }
        let t262 = (arg3 as *const u8).add(((gid0.wrapping_add((26u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t262 as *mut u32).add(0), uint32x4x2_t(t250[0], t250[2]));
            vst2q_u32((t262 as *mut u32).add(8), uint32x4x2_t(t250[1], t250[3]));
        } else {
            let mut t263w = 0u64;
            let t263 = core::hint::black_box(&raw mut t263w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t262 as *mut u64).add(0), t263).write_unaligned(((vgetq_lane_u32::<0>(t250[2]) as u64) << 32 | vgetq_lane_u32::<0>(t250[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t262 as *mut u64).add(1), t263).write_unaligned(((vgetq_lane_u32::<1>(t250[2]) as u64) << 32 | vgetq_lane_u32::<1>(t250[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t262 as *mut u64).add(2), t263).write_unaligned(((vgetq_lane_u32::<2>(t250[2]) as u64) << 32 | vgetq_lane_u32::<2>(t250[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t262 as *mut u64).add(3), t263).write_unaligned(((vgetq_lane_u32::<3>(t250[2]) as u64) << 32 | vgetq_lane_u32::<3>(t250[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t262 as *mut u64).add(4), t263).write_unaligned(((vgetq_lane_u32::<0>(t250[3]) as u64) << 32 | vgetq_lane_u32::<0>(t250[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t262 as *mut u64).add(5), t263).write_unaligned(((vgetq_lane_u32::<1>(t250[3]) as u64) << 32 | vgetq_lane_u32::<1>(t250[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t262 as *mut u64).add(6), t263).write_unaligned(((vgetq_lane_u32::<2>(t250[3]) as u64) << 32 | vgetq_lane_u32::<2>(t250[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t262 as *mut u64).add(7), t263).write_unaligned(((vgetq_lane_u32::<3>(t250[3]) as u64) << 32 | vgetq_lane_u32::<3>(t250[1]) as u64));
        }
        x15 = (((((a1) & (7u64))) as u32)).wrapping_add(1u32);
        let t264 = b5_r0ns_p(l, lid0, live, arg0, gid0.wrapping_mul(x15), x15, x15);
        let t265 = b8_s_p(l, lid0, live, (8u32).wrapping_mul(x15));
        let t266: u64 = t265;
        let t267 = [vdupq_n_u32((t266) as u32); 2];
        let t268 = [vdupq_n_u32(((t266) >> 32) as u32); 2];
        let t269: [uint32x4_t; 4] = [vandq_u32(t264[0], t267[0]), vandq_u32(t264[1], t267[1]), vandq_u32(t264[2], t268[0]), vandq_u32(t264[3], t268[1])];
        let t270 = (arg3 as *const u8).add(((gid0.wrapping_add((27u32).wrapping_mul(x10))) as usize) * 8);
        if all(&live) {
            vst2q_u32((t270 as *mut u32).add(0), uint32x4x2_t(t269[0], t269[2]));
            vst2q_u32((t270 as *mut u32).add(8), uint32x4x2_t(t269[1], t269[3]));
        } else {
            let mut t271w = 0u64;
            let t271 = core::hint::black_box(&raw mut t271w) as *mut u64;
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t270 as *mut u64).add(0), t271).write_unaligned(((vgetq_lane_u32::<0>(t269[2]) as u64) << 32 | vgetq_lane_u32::<0>(t269[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t270 as *mut u64).add(1), t271).write_unaligned(((vgetq_lane_u32::<1>(t269[2]) as u64) << 32 | vgetq_lane_u32::<1>(t269[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t270 as *mut u64).add(2), t271).write_unaligned(((vgetq_lane_u32::<2>(t269[2]) as u64) << 32 | vgetq_lane_u32::<2>(t269[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t270 as *mut u64).add(3), t271).write_unaligned(((vgetq_lane_u32::<3>(t269[2]) as u64) << 32 | vgetq_lane_u32::<3>(t269[0]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t270 as *mut u64).add(4), t271).write_unaligned(((vgetq_lane_u32::<0>(t269[3]) as u64) << 32 | vgetq_lane_u32::<0>(t269[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t270 as *mut u64).add(5), t271).write_unaligned(((vgetq_lane_u32::<1>(t269[3]) as u64) << 32 | vgetq_lane_u32::<1>(t269[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t270 as *mut u64).add(6), t271).write_unaligned(((vgetq_lane_u32::<2>(t269[3]) as u64) << 32 | vgetq_lane_u32::<2>(t269[1]) as u64));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t270 as *mut u64).add(7), t271).write_unaligned(((vgetq_lane_u32::<3>(t269[3]) as u64) << 32 | vgetq_lane_u32::<3>(t269[1]) as u64));
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
        let t272 = (arg0 as *const u8).add(((gid0) as usize) * 8);
        let t2740 = vld2q_u32((t272 as *const u32).add(0));
        let t2741 = vld2q_u32((t272 as *const u32).add(8));
        let t274: [uint32x4_t; 4] = [t2740.0, t2741.0, t2740.1, t2741.1];
        x6 = t274;
        let t275 = (arg1 as *const u8).add(((gid0) as usize) * 8);
        let t2770 = vld2q_u32((t275 as *const u32).add(0));
        let t2771 = vld2q_u32((t275 as *const u32).add(8));
        let t277: [uint32x4_t; 4] = [t2770.0, t2771.0, t2770.1, t2771.1];
        x7 = t277;
        let t278 = (arg2 as *const u32).add((gid0) as usize);
        let t279: [uint32x4_t; 2] = [vld1q_u32(t278.add(0)), vld1q_u32(t278.add(4))];
        x8 = t279;
        let t280 = [vdupq_n_u32(63u32); 2];
        let t281: [uint32x4_t; 2] = [vandq_u32(x8[0], t280[0]), vandq_u32(x8[1], t280[1])];
        x9 = t281;
        x10 = a0;
        let t282: [uint32x4_t; 2] = [vaddq_u32(x6[0], x7[0]), vaddq_u32(x6[1], x7[1])];
        let t283: [uint32x4_t; 2] = [vcltq_u32(t282[0], x6[0]), vcltq_u32(t282[1], x6[1])];
        let t284: [uint32x4_t; 4] = [t282[0], t282[1], vsubq_u32(vaddq_u32(x6[2], x7[2]), t283[0]), vsubq_u32(vaddq_u32(x6[3], x7[3]), t283[1])];
        let t285 = (arg3 as *const u8).add(((gid0.wrapping_add((0u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t285 as *mut u32).add(0), uint32x4x2_t(t284[0], t284[2]));
        vst2q_u32((t285 as *mut u32).add(8), uint32x4x2_t(t284[1], t284[3]));
        let t286: [uint32x4_t; 2] = [vcltq_u32(x6[0], x7[0]), vcltq_u32(x6[1], x7[1])];
        let t287: [uint32x4_t; 4] = [vsubq_u32(x6[0], x7[0]), vsubq_u32(x6[1], x7[1]), vaddq_u32(vsubq_u32(x6[2], x7[2]), t286[0]), vaddq_u32(vsubq_u32(x6[3], x7[3]), t286[1])];
        let t288 = (arg3 as *const u8).add(((gid0.wrapping_add((1u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t288 as *mut u32).add(0), uint32x4x2_t(t287[0], t287[2]));
        vst2q_u32((t288 as *mut u32).add(8), uint32x4x2_t(t287[1], t287[3]));
        let t289: [uint32x4_t; 2] = [x9[0], x9[1]];
        let t290: [uint32x4_t; 2] = [vandq_u32(t289[0], vdupq_n_u32(63)), vandq_u32(t289[1], vdupq_n_u32(63))];
        let t291: [uint32x4_t; 4] = [vshlq_u32(x6[0], vreinterpretq_s32_u32(t290[0])), vshlq_u32(x6[1], vreinterpretq_s32_u32(t290[1])), vorrq_u32(vshlq_u32(x6[2], vreinterpretq_s32_u32(t290[0])), vshlq_u32(x6[0], vsubq_s32(vreinterpretq_s32_u32(t290[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(x6[3], vreinterpretq_s32_u32(t290[1])), vshlq_u32(x6[1], vsubq_s32(vreinterpretq_s32_u32(t290[1]), vdupq_n_s32(32))))];
        let t292 = (arg3 as *const u8).add(((gid0.wrapping_add((2u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t292 as *mut u32).add(0), uint32x4x2_t(t291[0], t291[2]));
        vst2q_u32((t292 as *mut u32).add(8), uint32x4x2_t(t291[1], t291[3]));
        let t293: [uint32x4_t; 2] = [x9[0], x9[1]];
        let t294: [uint32x4_t; 2] = [vandq_u32(t293[0], vdupq_n_u32(63)), vandq_u32(t293[1], vdupq_n_u32(63))];
        let t295: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(x6[0], vnegq_s32(vreinterpretq_s32_u32(t294[0]))), vshlq_u32(x6[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t294[0])))), vorrq_u32(vshlq_u32(x6[1], vnegq_s32(vreinterpretq_s32_u32(t294[1]))), vshlq_u32(x6[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t294[1])))), vshlq_u32(x6[2], vnegq_s32(vreinterpretq_s32_u32(t294[0]))), vshlq_u32(x6[3], vnegq_s32(vreinterpretq_s32_u32(t294[1])))];
        let t296 = (arg3 as *const u8).add(((gid0.wrapping_add((3u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t296 as *mut u32).add(0), uint32x4x2_t(t295[0], t295[2]));
        vst2q_u32((t296 as *mut u32).add(8), uint32x4x2_t(t295[1], t295[3]));
        let t297: [uint32x4_t; 2] = [x9[0], x9[1]];
        let t298: [uint32x4_t; 2] = [vandq_u32(t297[0], vdupq_n_u32(63)), vandq_u32(t297[1], vdupq_n_u32(63))];
        let t299: [uint32x4_t; 2] = [vcgeq_s32(vreinterpretq_s32_u32(t298[0]), vdupq_n_s32(32)), vcgeq_s32(vreinterpretq_s32_u32(t298[1]), vdupq_n_s32(32))];
        let t300: [uint32x4_t; 4] = [vbslq_u32(t299[0], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t298[0])))), vorrq_u32(vshlq_u32(x6[0], vnegq_s32(vreinterpretq_s32_u32(t298[0]))), vshlq_u32(x6[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t298[0]))))), vbslq_u32(t299[1], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t298[1])))), vorrq_u32(vshlq_u32(x6[1], vnegq_s32(vreinterpretq_s32_u32(t298[1]))), vshlq_u32(x6[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t298[1]))))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vnegq_s32(vreinterpretq_s32_u32(t298[0])))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vnegq_s32(vreinterpretq_s32_u32(t298[1]))))];
        let t301 = (arg3 as *const u8).add(((gid0.wrapping_add((4u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t301 as *mut u32).add(0), uint32x4x2_t(t300[0], t300[2]));
        vst2q_u32((t301 as *mut u32).add(8), uint32x4x2_t(t300[1], t300[3]));
        let t302: [uint32x4_t; 4] = [vshlq_n_u32::<13>(x6[0]), vshlq_n_u32::<13>(x6[1]), vorrq_u32(vshlq_n_u32::<13>(x6[2]), vshrq_n_u32::<19>(x6[0])), vorrq_u32(vshlq_n_u32::<13>(x6[3]), vshrq_n_u32::<19>(x6[1]))];
        let t303: [uint32x4_t; 2] = [vshrq_n_u32::<19>(x7[2]), vshrq_n_u32::<19>(x7[3])];
        let t304: [uint32x4_t; 4] = [vorrq_u32(t302[0], t303[0]), vorrq_u32(t302[1], t303[1]), t302[2], t302[3]];
        let t305 = (arg3 as *const u8).add(((gid0.wrapping_add((5u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t305 as *mut u32).add(0), uint32x4x2_t(t304[0], t304[2]));
        vst2q_u32((t305 as *mut u32).add(8), uint32x4x2_t(t304[1], t304[3]));
        let t306: [uint32x4_t; 4] = [vdupq_n_u32(0), vdupq_n_u32(0), vshlq_n_u32::<8>(x6[0]), vshlq_n_u32::<8>(x6[1])];
        let t307 = (arg3 as *const u8).add(((gid0.wrapping_add((6u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t307 as *mut u32).add(0), uint32x4x2_t(t306[0], t306[2]));
        vst2q_u32((t307 as *mut u32).add(8), uint32x4x2_t(t306[1], t306[3]));
        let t308: [uint32x4_t; 2] = [vshrq_n_u32::<3>(x6[2]), vshrq_n_u32::<3>(x6[3])];
        let t309: [uint32x4_t; 4] = [t308[0], t308[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        let t310 = (arg3 as *const u8).add(((gid0.wrapping_add((7u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t310 as *mut u32).add(0), uint32x4x2_t(t309[0], t309[2]));
        vst2q_u32((t310 as *mut u32).add(8), uint32x4x2_t(t309[1], t309[3]));
        let t311 = vdupq_n_s32(((((40i64) as u32)) & 63) as i32);
        let t312: [uint32x4_t; 2] = [vcgeq_s32(t311, vdupq_n_s32(32)), vcgeq_s32(t311, vdupq_n_s32(32))];
        let t313: [uint32x4_t; 4] = [vbslq_u32(t312[0], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vsubq_s32(vdupq_n_s32(32), t311))), vorrq_u32(vshlq_u32(x6[0], vnegq_s32(t311)), vshlq_u32(x6[2], vsubq_s32(vdupq_n_s32(32), t311)))), vbslq_u32(t312[1], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vsubq_s32(vdupq_n_s32(32), t311))), vorrq_u32(vshlq_u32(x6[1], vnegq_s32(t311)), vshlq_u32(x6[3], vsubq_s32(vdupq_n_s32(32), t311)))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vnegq_s32(t311))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vnegq_s32(t311)))];
        let t314 = (arg3 as *const u8).add(((gid0.wrapping_add((8u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t314 as *mut u32).add(0), uint32x4x2_t(t313[0], t313[2]));
        vst2q_u32((t314 as *mut u32).add(8), uint32x4x2_t(t313[1], t313[3]));
        let t315: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x6[0]), vget_low_u32(x7[0]))), vreinterpretq_u32_u64(vmull_high_u32(x6[0], x7[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x6[1]), vget_low_u32(x7[1]))), vreinterpretq_u32_u64(vmull_high_u32(x6[1], x7[1])))];
        let t316: [uint32x4_t; 4] = [vmulq_u32(x6[0], x7[0]), vmulq_u32(x6[1], x7[1]), vmlaq_u32(vmlaq_u32(t315[0], x6[0], x7[2]), x7[0], x6[2]), vmlaq_u32(vmlaq_u32(t315[1], x6[1], x7[3]), x7[1], x6[3])];
        let t317 = (arg3 as *const u8).add(((gid0.wrapping_add((9u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t317 as *mut u32).add(0), uint32x4x2_t(t316[0], t316[2]));
        vst2q_u32((t317 as *mut u32).add(8), uint32x4x2_t(t316[1], t316[3]));
        let t318: u64 = 0u64;
        let t319 = [vdupq_n_u32((t318) as u32); 2];
        let t320: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x7[0], t319[0]), vceqq_u32(x7[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(x7[1], t319[1]), vceqq_u32(x7[3], vdupq_n_u32(0)))];
        let t321: [uint32x4_t; 2] = [vmvnq_u32(t320[0]), vmvnq_u32(t320[1])];
        let mut t322: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t323: [uint32x4_t; 2] = [vmvnq_u32(t321[0]), vmvnq_u32(t321[1])];
        {
            let (mut t324xl, mut t324xh, mut t324yl, mut t324yh) = ([0u32; 8], [0u32; 8], [0u32; 8], [0u32; 8]);
            vst1q_u32(t324xl.as_mut_ptr().add(0), x6[0]); vst1q_u32(t324xh.as_mut_ptr().add(0), x6[2]); vst1q_u32(t324yl.as_mut_ptr().add(0), x7[0]); vst1q_u32(t324yh.as_mut_ptr().add(0), x7[2]);
            vst1q_u32(t324xl.as_mut_ptr().add(4), x6[1]); vst1q_u32(t324xh.as_mut_ptr().add(4), x6[3]); vst1q_u32(t324yl.as_mut_ptr().add(4), x7[1]); vst1q_u32(t324yh.as_mut_ptr().add(4), x7[3]);
            for j in 0..8 { let q = (((t324xh[j] as u64) << 32 | t324xl[j] as u64) as u64).checked_div(((t324yh[j] as u64) << 32 | t324yl[j] as u64) as u64).unwrap_or(0) as u64; t324xl[j] = q as u32; t324xh[j] = (q >> 32) as u32; }
            let t325: [uint32x4_t; 4] = [vld1q_u32(t324xl.as_ptr().add(0)), vld1q_u32(t324xl.as_ptr().add(4)), vld1q_u32(t324xh.as_ptr().add(0)), vld1q_u32(t324xh.as_ptr().add(4))];
            t322 = [vbslq_u32(t321[0], t325[0], t322[0]), vbslq_u32(t321[1], t325[1], t322[1]), vbslq_u32(t321[0], t325[2], t322[2]), vbslq_u32(t321[1], t325[3], t322[3])];
        }
        {
            let t326: u64 = 7u64;
            let t327 = [vdupq_n_u32((t326) as u32); 2];
            let t328: [uint32x4_t; 4] = [t327[0], t327[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            t322 = [vbslq_u32(t323[0], t328[0], t322[0]), vbslq_u32(t323[1], t328[1], t322[1]), vbslq_u32(t323[0], t328[2], t322[2]), vbslq_u32(t323[1], t328[3], t322[3])];
        }
        let t329 = (arg3 as *const u8).add(((gid0.wrapping_add((10u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t329 as *mut u32).add(0), uint32x4x2_t(t322[0], t322[2]));
        vst2q_u32((t329 as *mut u32).add(8), uint32x4x2_t(t322[1], t322[3]));
        let t330: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x6[2], x7[2]), vandq_u32(vceqq_u32(x6[2], x7[2]), vcltq_u32(x6[0], x7[0]))), vorrq_u32(vcltq_u32(x6[3], x7[3]), vandq_u32(vceqq_u32(x6[3], x7[3]), vcltq_u32(x6[1], x7[1])))];
        let t331: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t330[0]), vshrq_n_u32::<31>(t330[1])];
        let t332: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x7[2], x6[2]), vandq_u32(vceqq_u32(x7[2], x6[2]), vcltq_u32(x7[0], x6[0]))), vorrq_u32(vcltq_u32(x7[3], x6[3]), vandq_u32(vceqq_u32(x7[3], x6[3]), vcltq_u32(x7[1], x6[1])))];
        let t333: [uint32x4_t; 2] = [vmvnq_u32(t332[0]), vmvnq_u32(t332[1])];
        let t334: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t333[0]), vshrq_n_u32::<31>(t333[1])];
        let t335: [uint32x4_t; 4] = [vshlq_n_u32::<1>(t334[0]), vshlq_n_u32::<1>(t334[1]), vshrq_n_u32::<31>(t334[0]), vshrq_n_u32::<31>(t334[1])];
        let t336: [uint32x4_t; 4] = [vorrq_u32(t331[0], t335[0]), vorrq_u32(t331[1], t335[1]), t335[2], t335[3]];
        let t337: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x6[0], x7[0]), vceqq_u32(x6[2], x7[2])), vandq_u32(vceqq_u32(x6[1], x7[1]), vceqq_u32(x6[3], x7[3]))];
        let t338: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t337[0]), vshrq_n_u32::<31>(t337[1])];
        let t339: [uint32x4_t; 4] = [vshlq_n_u32::<2>(t338[0]), vshlq_n_u32::<2>(t338[1]), vshrq_n_u32::<30>(t338[0]), vshrq_n_u32::<30>(t338[1])];
        let t340: [uint32x4_t; 4] = [vorrq_u32(t336[0], t339[0]), vorrq_u32(t336[1], t339[1]), vorrq_u32(t336[2], t339[2]), vorrq_u32(t336[3], t339[3])];
        let t341: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x6[0], x7[0]), vceqq_u32(x6[2], x7[2])), vandq_u32(vceqq_u32(x6[1], x7[1]), vceqq_u32(x6[3], x7[3]))];
        let t342: [uint32x4_t; 2] = [vmvnq_u32(t341[0]), vmvnq_u32(t341[1])];
        let t343: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t342[0]), vshrq_n_u32::<31>(t342[1])];
        let t344: [uint32x4_t; 4] = [vshlq_n_u32::<3>(t343[0]), vshlq_n_u32::<3>(t343[1]), vshrq_n_u32::<29>(t343[0]), vshrq_n_u32::<29>(t343[1])];
        let t345: [uint32x4_t; 4] = [vorrq_u32(t340[0], t344[0]), vorrq_u32(t340[1], t344[1]), vorrq_u32(t340[2], t344[2]), vorrq_u32(t340[3], t344[3])];
        let t346: [uint32x4_t; 2] = [vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x6[2]), vreinterpretq_s32_u32(x7[2])), vandq_u32(vceqq_u32(x6[2], x7[2]), vcltq_u32(x6[0], x7[0]))), vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x6[3]), vreinterpretq_s32_u32(x7[3])), vandq_u32(vceqq_u32(x6[3], x7[3]), vcltq_u32(x6[1], x7[1])))];
        let t347: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t346[0]), vshrq_n_u32::<31>(t346[1])];
        let t348: [uint32x4_t; 4] = [vshlq_n_u32::<4>(t347[0]), vshlq_n_u32::<4>(t347[1]), vshrq_n_u32::<28>(t347[0]), vshrq_n_u32::<28>(t347[1])];
        let t349: [uint32x4_t; 4] = [vorrq_u32(t345[0], t348[0]), vorrq_u32(t345[1], t348[1]), vorrq_u32(t345[2], t348[2]), vorrq_u32(t345[3], t348[3])];
        let t350: [uint32x4_t; 2] = [vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x6[2]), vreinterpretq_s32_u32(x7[2])), vandq_u32(vceqq_u32(x6[2], x7[2]), vcltq_u32(x6[0], x7[0]))), vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x6[3]), vreinterpretq_s32_u32(x7[3])), vandq_u32(vceqq_u32(x6[3], x7[3]), vcltq_u32(x6[1], x7[1])))];
        let t351: [uint32x4_t; 2] = [vmvnq_u32(t350[0]), vmvnq_u32(t350[1])];
        let t352: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t351[0]), vshrq_n_u32::<31>(t351[1])];
        let t353: [uint32x4_t; 4] = [vshlq_n_u32::<5>(t352[0]), vshlq_n_u32::<5>(t352[1]), vshrq_n_u32::<27>(t352[0]), vshrq_n_u32::<27>(t352[1])];
        let t354: [uint32x4_t; 4] = [vorrq_u32(t349[0], t353[0]), vorrq_u32(t349[1], t353[1]), vorrq_u32(t349[2], t353[2]), vorrq_u32(t349[3], t353[3])];
        let t355: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x7[2], x6[2]), vandq_u32(vceqq_u32(x7[2], x6[2]), vcltq_u32(x7[0], x6[0]))), vorrq_u32(vcltq_u32(x7[3], x6[3]), vandq_u32(vceqq_u32(x7[3], x6[3]), vcltq_u32(x7[1], x6[1])))];
        let t356: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t355[0]), vshrq_n_u32::<31>(t355[1])];
        let t357: [uint32x4_t; 4] = [vshlq_n_u32::<6>(t356[0]), vshlq_n_u32::<6>(t356[1]), vshrq_n_u32::<26>(t356[0]), vshrq_n_u32::<26>(t356[1])];
        let t358: [uint32x4_t; 4] = [vorrq_u32(t354[0], t357[0]), vorrq_u32(t354[1], t357[1]), vorrq_u32(t354[2], t357[2]), vorrq_u32(t354[3], t357[3])];
        let t359 = (arg3 as *const u8).add(((gid0.wrapping_add((11u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t359 as *mut u32).add(0), uint32x4x2_t(t358[0], t358[2]));
        vst2q_u32((t359 as *mut u32).add(8), uint32x4x2_t(t358[1], t358[3]));
        let t360: [uint32x4_t; 2] = [vaddq_u32(x8[0], x6[0]), vaddq_u32(x8[1], x6[1])];
        let t361: [uint32x4_t; 2] = [vcltq_u32(t360[0], x8[0]), vcltq_u32(t360[1], x8[1])];
        let t362: [uint32x4_t; 4] = [t360[0], t360[1], vsubq_u32(vaddq_u32(vdupq_n_u32(0), x6[2]), t361[0]), vsubq_u32(vaddq_u32(vdupq_n_u32(0), x6[3]), t361[1])];
        let t363 = (arg3 as *const u8).add(((gid0.wrapping_add((12u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t363 as *mut u32).add(0), uint32x4x2_t(t362[0], t362[2]));
        vst2q_u32((t363 as *mut u32).add(8), uint32x4x2_t(t362[1], t362[3]));
        let t364: [uint32x4_t; 2] = [vreinterpretq_u32_s32(vshrq_n_s32::<31>(vreinterpretq_s32_u32(x8[0]))), vreinterpretq_u32_s32(vshrq_n_s32::<31>(vreinterpretq_s32_u32(x8[1])))];
        let t365: [uint32x4_t; 4] = [x8[0], x8[1], t364[0], t364[1]];
        let t366 = (arg3 as *const u8).add(((gid0.wrapping_add((13u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t366 as *mut u32).add(0), uint32x4x2_t(t365[0], t365[2]));
        vst2q_u32((t366 as *mut u32).add(8), uint32x4x2_t(t365[1], t365[3]));
        let t367: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<3>(x6[0]), vshlq_n_u32::<29>(x6[2])), vorrq_u32(vshrq_n_u32::<3>(x6[1]), vshlq_n_u32::<29>(x6[3])), vshrq_n_u32::<3>(x6[2]), vshrq_n_u32::<3>(x6[3])];
        let t368: [uint32x4_t; 2] = [t367[0], t367[1]];
        let t369: [uint32x4_t; 4] = [t368[0], t368[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        let t370 = (arg3 as *const u8).add(((gid0.wrapping_add((14u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t370 as *mut u32).add(0), uint32x4x2_t(t369[0], t369[2]));
        vst2q_u32((t370 as *mut u32).add(8), uint32x4x2_t(t369[1], t369[3]));
        let t371: u64 = 1u64;
        let t372 = [vdupq_n_u32((t371) as u32); 2];
        let t373: [uint32x4_t; 2] = [vandq_u32(x6[0], t372[0]), vandq_u32(x6[1], t372[1])];
        let t374: [uint32x4_t; 2] = [vtstq_u32(t373[0], t373[0]), vtstq_u32(t373[1], t373[1])];
        {
            let t375: [uint32x4_t; 4] = [veorq_u32(x6[0], x7[0]), veorq_u32(x6[1], x7[1]), veorq_u32(x6[2], x7[2]), veorq_u32(x6[3], x7[3])];
            x11 = [vbslq_u32(t374[0], t375[0], x11[0]), vbslq_u32(t374[1], t375[1], x11[1]), vbslq_u32(t374[0], t375[2], x11[2]), vbslq_u32(t374[1], t375[3], x11[3])];
        }
        let t376: [uint32x4_t; 2] = [vmvnq_u32(t374[0]), vmvnq_u32(t374[1])];
        {
            let t377: [uint32x4_t; 2] = [vaddq_u32(x6[0], x8[0]), vaddq_u32(x6[1], x8[1])];
            let t378: [uint32x4_t; 2] = [vcltq_u32(t377[0], x6[0]), vcltq_u32(t377[1], x6[1])];
            let t379: [uint32x4_t; 4] = [t377[0], t377[1], vsubq_u32(vaddq_u32(x6[2], vdupq_n_u32(0)), t378[0]), vsubq_u32(vaddq_u32(x6[3], vdupq_n_u32(0)), t378[1])];
            x11 = [vbslq_u32(t376[0], t379[0], x11[0]), vbslq_u32(t376[1], t379[1], x11[1]), vbslq_u32(t376[0], t379[2], x11[2]), vbslq_u32(t376[1], t379[3], x11[3])];
        }
        let t380 = (arg3 as *const u8).add(((gid0.wrapping_add((15u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t380 as *mut u32).add(0), uint32x4x2_t(x11[0], x11[2]));
        vst2q_u32((t380 as *mut u32).add(8), uint32x4x2_t(x11[1], x11[3]));
        let t381 = b1_l_f(l, lid0, x6);
        let t382 = b2_l_f(l, lid0, x7);
        let t383: [uint32x4_t; 4] = [vshlq_n_u32::<8>(t382[0]), vshlq_n_u32::<8>(t382[1]), vshrq_n_u32::<24>(t382[0]), vshrq_n_u32::<24>(t382[1])];
        let t384: [uint32x4_t; 2] = [vaddq_u32(t381[0], t383[0]), vaddq_u32(t381[1], t383[1])];
        let t385: [uint32x4_t; 2] = [vcltq_u32(t384[0], t381[0]), vcltq_u32(t384[1], t381[1])];
        let t386: [uint32x4_t; 4] = [t384[0], t384[1], vsubq_u32(vaddq_u32(vdupq_n_u32(0), t383[2]), t385[0]), vsubq_u32(vaddq_u32(vdupq_n_u32(0), t383[3]), t385[1])];
        let t387 = (arg3 as *const u8).add(((gid0.wrapping_add((16u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t387 as *mut u32).add(0), uint32x4x2_t(t386[0], t386[2]));
        vst2q_u32((t387 as *mut u32).add(8), uint32x4x2_t(t386[1], t386[3]));
        let t388 = [vdupq_n_u32(x10); 2];
        let mut t389x = [0u32; 8];
        let mut t389y = [0u32; 8];
        for r in 0..2 { vst1q_u32(t389x.as_mut_ptr().add(4 * r), x8[r]); vst1q_u32(t389y.as_mut_ptr().add(4 * r), t388[r]); }
        for j in 0..8 { t389x[j] = (t389x[j] as u32).checked_rem(t389y[j] as u32).unwrap_or(0) as u32; }
        let t390: [uint32x4_t; 2] = [vld1q_u32(t389x.as_ptr().add(0)), vld1q_u32(t389x.as_ptr().add(4))];
        let mut t391: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t392 = vminvq_u32(vminq_u32(t390[0], t390[1]));
        if t392 == vmaxvq_u32(vmaxq_u32(t390[0], t390[1])) {
            let t393 = ((arg0 as *const u8).add(t392 as usize * 8) as *const u64).read_unaligned();
            t391 = [vdupq_n_u32(t393 as u32), vdupq_n_u32(t393 as u32), vdupq_n_u32((t393 >> 32) as u32), vdupq_n_u32((t393 >> 32) as u32)];
        } else {
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t390[0]) as usize * 8) as *const u64).read_unaligned(); t391[0] = vsetq_lane_u32::<0>(v as u32, t391[0]); t391[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t391[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t390[0]) as usize * 8) as *const u64).read_unaligned(); t391[0] = vsetq_lane_u32::<1>(v as u32, t391[0]); t391[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t391[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t390[0]) as usize * 8) as *const u64).read_unaligned(); t391[0] = vsetq_lane_u32::<2>(v as u32, t391[0]); t391[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t391[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t390[0]) as usize * 8) as *const u64).read_unaligned(); t391[0] = vsetq_lane_u32::<3>(v as u32, t391[0]); t391[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t391[2]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t390[1]) as usize * 8) as *const u64).read_unaligned(); t391[1] = vsetq_lane_u32::<0>(v as u32, t391[1]); t391[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t391[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t390[1]) as usize * 8) as *const u64).read_unaligned(); t391[1] = vsetq_lane_u32::<1>(v as u32, t391[1]); t391[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t391[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t390[1]) as usize * 8) as *const u64).read_unaligned(); t391[1] = vsetq_lane_u32::<2>(v as u32, t391[1]); t391[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t391[3]); }
            { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t390[1]) as usize * 8) as *const u64).read_unaligned(); t391[1] = vsetq_lane_u32::<3>(v as u32, t391[1]); t391[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t391[3]); }
        }
        let t394 = (arg3 as *const u8).add(((gid0.wrapping_add((17u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t394 as *mut u32).add(0), uint32x4x2_t(t391[0], t391[2]));
        vst2q_u32((t394 as *mut u32).add(8), uint32x4x2_t(t391[1], t391[3]));
        let t395 = b3_ll_f(l, lid0, x6, x8);
        let t396 = (arg3 as *const u8).add(((gid0.wrapping_add((18u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t396 as *mut u32).add(0), uint32x4x2_t(t395[0], t395[2]));
        vst2q_u32((t396 as *mut u32).add(8), uint32x4x2_t(t395[1], t395[3]));
        x12 = x6;
        x13 = 0u32;
        let mut t405 = live;
        loop {
            let t407 = bits(&t405);
            if t407.count_ones() <= 2 {
                let mut t408 = t407;
                while t408 != 0 {
                    let j = t408.trailing_zeros() as usize;
                    t408 &= t408 - 1;
                    let t409 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x7);
                    let mut x7_j: u64 = ((t409[8 + j] as u64) << 32 | t409[j] as u64);
                    let t410 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x8);
                    let mut x8_j: u32 = t410[j];
                    let t411 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x12);
                    let mut x12_j: u64 = ((t411[8 + j] as u64) << 32 | t411[j] as u64);
                    let mut x13_j: u32 = x13;
                    loop {
                        if !(((x13_j) < (((x8_j) & (7u32))))) { break; }
                        x12_j = ((x12_j).wrapping_mul(3u64)).wrapping_add(x7_j);
                        let t412 = x13_j;
                        x13_j = (x13_j).wrapping_add(1u32);
                    }
                    let mut t413 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x12);
                    t413[j] = x12_j as u32;
                    t413[8 + j] = (x12_j as u64 >> 32) as u32;
                    x12 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t413);
                }
                break;
            }
            // Lanes part: each its own rounds.
            let t414 = [vdupq_n_u32(7u32); 2];
            let t415: [uint32x4_t; 2] = [vandq_u32(x8[0], t414[0]), vandq_u32(x8[1], t414[1])];
            let t416 = [vdupq_n_u32(x13); 2];
            let t417: [uint32x4_t; 2] = [vcltq_u32(t416[0], t415[0]), vcltq_u32(t416[1], t415[1])];
            t405 = [vandq_u32(t405[0], t417[0]), vandq_u32(t405[1], t417[1])];
            if !(any(&t405)) { break; }
            let t419: u64 = 3u64;
            let t420 = [vdupq_n_u32((t419) as u32); 2];
            let t421: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x12[0]), vget_low_u32(t420[0]))), vreinterpretq_u32_u64(vmull_high_u32(x12[0], t420[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x12[1]), vget_low_u32(t420[1]))), vreinterpretq_u32_u64(vmull_high_u32(x12[1], t420[1])))];
            let t422: [uint32x4_t; 4] = [vmulq_u32(x12[0], t420[0]), vmulq_u32(x12[1], t420[1]), vmlaq_u32(t421[0], t420[0], x12[2]), vmlaq_u32(t421[1], t420[1], x12[3])];
            let t423: [uint32x4_t; 2] = [vaddq_u32(t422[0], x7[0]), vaddq_u32(t422[1], x7[1])];
            let t424: [uint32x4_t; 2] = [vcltq_u32(t423[0], t422[0]), vcltq_u32(t423[1], t422[1])];
            let t425: [uint32x4_t; 4] = [t423[0], t423[1], vsubq_u32(vaddq_u32(t422[2], x7[2]), t424[0]), vsubq_u32(vaddq_u32(t422[3], x7[3]), t424[1])];
            x12 = [vbslq_u32(t405[0], t425[0], x12[0]), vbslq_u32(t405[1], t425[1], x12[1]), vbslq_u32(t405[0], t425[2], x12[2]), vbslq_u32(t405[1], t425[3], x12[3])];
            let t426: [uint32x4_t; 2] = [vandq_u32(t405[0], live[0]), vandq_u32(t405[1], live[1])];
            let t427 = x13;
            x13 = (x13).wrapping_add(1u32);
        }
        let t428 = (arg3 as *const u8).add(((gid0.wrapping_add((19u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t428 as *mut u32).add(0), uint32x4x2_t(x12[0], x12[2]));
        vst2q_u32((t428 as *mut u32).add(8), uint32x4x2_t(x12[1], x12[3]));
        let t429: u64 = a1;
        let t430 = [vdupq_n_u32((t429) as u32); 2];
        let t431 = [vdupq_n_u32(((t429) >> 32) as u32); 2];
        let t432: [uint32x4_t; 2] = [vaddq_u32(x6[0], t430[0]), vaddq_u32(x6[1], t430[1])];
        let t433: [uint32x4_t; 2] = [vcltq_u32(t432[0], x6[0]), vcltq_u32(t432[1], x6[1])];
        let t434: [uint32x4_t; 4] = [t432[0], t432[1], vsubq_u32(vaddq_u32(x6[2], t431[0]), t433[0]), vsubq_u32(vaddq_u32(x6[3], t431[1]), t433[1])];
        let t435: u64 = (a1).wrapping_shl((3u64) as u32);
        let t436 = [vdupq_n_u32((t435) as u32); 2];
        let t437 = [vdupq_n_u32(((t435) >> 32) as u32); 2];
        let t438: [uint32x4_t; 2] = [vaddq_u32(t434[0], t436[0]), vaddq_u32(t434[1], t436[1])];
        let t439: [uint32x4_t; 2] = [vcltq_u32(t438[0], t434[0]), vcltq_u32(t438[1], t434[1])];
        let t440: [uint32x4_t; 4] = [t438[0], t438[1], vsubq_u32(vaddq_u32(t434[2], t437[0]), t439[0]), vsubq_u32(vaddq_u32(t434[3], t437[1]), t439[1])];
        let t441 = (arg3 as *const u8).add(((gid0.wrapping_add((20u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t441 as *mut u32).add(0), uint32x4x2_t(t440[0], t440[2]));
        vst2q_u32((t441 as *mut u32).add(8), uint32x4x2_t(t440[1], t440[3]));
        let t442: u64 = 4294967295u64;
        let t443 = [vdupq_n_u32((t442) as u32); 2];
        let t444: [uint32x4_t; 2] = [vandq_u32(x6[0], t443[0]), vandq_u32(x6[1], t443[1])];
        let t445: u64 = 4294967295u64;
        let t446 = [vdupq_n_u32((t445) as u32); 2];
        let t447: [uint32x4_t; 2] = [vandq_u32(x7[0], t446[0]), vandq_u32(x7[1], t446[1])];
        let t448: [uint32x4_t; 2] = [vceqq_u32(t444[0], t447[0]), vceqq_u32(t444[1], t447[1])];
        let mut t449: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t450: [uint32x4_t; 2] = [vmvnq_u32(t448[0]), vmvnq_u32(t448[1])];
        {
            let t451 = [vdupq_n_u32(1u32); 2];
            t449 = [vbslq_u32(t448[0], t451[0], t449[0]), vbslq_u32(t448[1], t451[1], t449[1])];
        }
        {
            let t452 = [vdupq_n_u32(2u32); 2];
            t449 = [vbslq_u32(t450[0], t452[0], t449[0]), vbslq_u32(t450[1], t452[1], t449[1])];
        }
        let t453: [uint32x4_t; 4] = [t449[0], t449[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        let t454 = (arg3 as *const u8).add(((gid0.wrapping_add((21u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t454 as *mut u32).add(0), uint32x4x2_t(t453[0], t453[2]));
        vst2q_u32((t454 as *mut u32).add(8), uint32x4x2_t(t453[1], t453[3]));
        let t455: [uint32x4_t; 2] = [vtstq_u32(vorrq_u32(x6[0], x6[2]), vorrq_u32(x6[0], x6[2])), vtstq_u32(vorrq_u32(x6[1], x6[3]), vorrq_u32(x6[1], x6[3]))];
        let mut t456: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t457: [uint32x4_t; 2] = [vmvnq_u32(t455[0]), vmvnq_u32(t455[1])];
        {
            let t458 = [vdupq_n_u32(5u32); 2];
            t456 = [vbslq_u32(t455[0], t458[0], t456[0]), vbslq_u32(t455[1], t458[1], t456[1])];
        }
        {
            let t459 = [vdupq_n_u32(6u32); 2];
            t456 = [vbslq_u32(t457[0], t459[0], t456[0]), vbslq_u32(t457[1], t459[1], t456[1])];
        }
        let t460: [uint32x4_t; 4] = [t456[0], t456[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        let t461 = (arg3 as *const u8).add(((gid0.wrapping_add((22u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t461 as *mut u32).add(0), uint32x4x2_t(t460[0], t460[2]));
        vst2q_u32((t461 as *mut u32).add(8), uint32x4x2_t(t460[1], t460[3]));
        let t462 = b4_ll_f(l, lid0, x6, x7);
        let t463 = (arg3 as *const u8).add(((gid0.wrapping_add((23u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t463 as *mut u32).add(0), uint32x4x2_t(t462[0], t462[2]));
        vst2q_u32((t463 as *mut u32).add(8), uint32x4x2_t(t462[1], t462[3]));
        let t464: u32;
        if ((((a1) & (1u64))) != (0u64)) {
            t464 = 8u32;
        } else {
            t464 = 4u32;
        }
        x14 = t464;
        let t465 = b5_r0ns_f_v0(l, lid0, arg0, gid0.wrapping_mul(x14), x14, 8u32);
        let t466 = (arg3 as *const u8).add(((gid0.wrapping_add((24u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t466 as *mut u32).add(0), uint32x4x2_t(t465[0], t465[2]));
        vst2q_u32((t466 as *mut u32).add(8), uint32x4x2_t(t465[1], t465[3]));
        let t467 = if (gid0.wrapping_add(3u32) & 255u32) <= 248u32 { b6_u8_f(l, lid0, gid0.wrapping_add(3u32)) } else { b6_i_f(l, lid0, gid0.wrapping_add(3u32)) };
        let t468 = (arg3 as *const u8).add(((gid0.wrapping_add((25u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t468 as *mut u32).add(0), uint32x4x2_t(t467[0], t467[2]));
        vst2q_u32((t468 as *mut u32).add(8), uint32x4x2_t(t467[1], t467[3]));
        let t469: u64 = 3u64;
        let t470 = [vdupq_n_u32((t469) as u32); 2];
        let t471: [uint32x4_t; 2] = [vandq_u32(x6[0], t470[0]), vandq_u32(x6[1], t470[1])];
        let t472: u64 = 0u64;
        let t473 = [vdupq_n_u32((t472) as u32); 2];
        let t474: [uint32x4_t; 2] = [vceqq_u32(t471[0], t473[0]), vceqq_u32(t471[1], t473[1])];
        let mut t475: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t476: [uint32x4_t; 2] = [vmvnq_u32(t474[0]), vmvnq_u32(t474[1])];
        if any(&t474) {
            let t477: [uint32x4_t; 4];
            let t478 = bits(&t474);
            if t478.count_ones() <= 2 {
                let t479 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x7);
                let t480 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x8);
                let mut t481 = [0u32; 16];
                let mut t482 = t478;
                while t482 != 0 {
                    let j = t482.trailing_zeros() as usize;
                    t482 &= t482 - 1;
                    let t483 = b7_ss_f(l, lid0, ((t479[8 + j] as u64) << 32 | t479[j] as u64), t480[j]);
                    let t484: u64 = t483;
                    t481[j] = t484 as u32;
                    t481[8 + j] = (t484 >> 32) as u32;
                }
                t477 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t481);
            } else {
                let t485 = b7_ll_p(l, lid0, t474, x7, x8);
                t477 = t485;
            }
            t475 = [vbslq_u32(t474[0], t477[0], t475[0]), vbslq_u32(t474[1], t477[1], t475[1]), vbslq_u32(t474[0], t477[2], t475[2]), vbslq_u32(t474[1], t477[3], t475[3])];
        }
        {
            t475 = [vbslq_u32(t476[0], x6[0], t475[0]), vbslq_u32(t476[1], x6[1], t475[1]), vbslq_u32(t476[0], x6[2], t475[2]), vbslq_u32(t476[1], x6[3], t475[3])];
        }
        let t486 = (arg3 as *const u8).add(((gid0.wrapping_add((26u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t486 as *mut u32).add(0), uint32x4x2_t(t475[0], t475[2]));
        vst2q_u32((t486 as *mut u32).add(8), uint32x4x2_t(t475[1], t475[3]));
        x15 = (((((a1) & (7u64))) as u32)).wrapping_add(1u32);
        let t487 = b5_r0ns_f_v0(l, lid0, arg0, gid0.wrapping_mul(x15), x15, x15);
        let t488 = b8_s_f(l, lid0, (8u32).wrapping_mul(x15));
        let t489: u64 = t488;
        let t490 = [vdupq_n_u32((t489) as u32); 2];
        let t491 = [vdupq_n_u32(((t489) >> 32) as u32); 2];
        let t492: [uint32x4_t; 4] = [vandq_u32(t487[0], t490[0]), vandq_u32(t487[1], t490[1]), vandq_u32(t487[2], t491[0]), vandq_u32(t487[3], t491[1])];
        let t493 = (arg3 as *const u8).add(((gid0.wrapping_add((27u32).wrapping_mul(x10))) as usize) * 8);
        vst2q_u32((t493 as *mut u32).add(0), uint32x4x2_t(t492[0], t492[2]));
        vst2q_u32((t493 as *mut u32).add(8), uint32x4x2_t(t492[1], t492[3]));
    } else if (((gid0) as u64 + 7) <= u32::MAX as u64 && ((gid0) as u64) >= ((a0) as u64)) {
        return;
    } else {
        // Lanes part: the chunk's lanes on both sides of the bound.
        let t494: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t495 = [vdupq_n_u32(a0); 2];
        let t496: [uint32x4_t; 2] = [vcgeq_u32(t494[0], t495[0]), vcgeq_u32(t494[1], t495[1])];
        let t497: [uint32x4_t; 2] = [vandq_u32(live[0], t496[0]), vandq_u32(live[1], t496[1])];
        {
            live = [vbicq_u32(live[0], t497[0]), vbicq_u32(live[1], t497[1])];
        }
        if any(&live) {
            let t498 = (arg0 as *const u8).add(((gid0) as usize) * 8);
            let mut t499: [uint32x4_t; 4];
            if all(&live) {
                let t5000 = vld2q_u32((t498 as *const u32).add(0));
                let t5001 = vld2q_u32((t498 as *const u32).add(8));
                let t500: [uint32x4_t; 4] = [t5000.0, t5001.0, t5000.1, t5001.1];
                t499 = t500;
            } else {
                let t501 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t494[0]), vandq_u32(live[1], t494[1]))));
                let t502: [uint32x4_t; 2] = [vbslq_u32(live[0], t494[0], t501), vbslq_u32(live[1], t494[1], t501)];
                let mut t503: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
                let t504 = vminvq_u32(vminq_u32(t502[0], t502[1]));
                if t504 == vmaxvq_u32(vmaxq_u32(t502[0], t502[1])) {
                    let t505 = ((arg0 as *const u8).add(t504 as usize * 8) as *const u64).read_unaligned();
                    t503 = [vdupq_n_u32(t505 as u32), vdupq_n_u32(t505 as u32), vdupq_n_u32((t505 >> 32) as u32), vdupq_n_u32((t505 >> 32) as u32)];
                } else {
                    { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t502[0]) as usize * 8) as *const u64).read_unaligned(); t503[0] = vsetq_lane_u32::<0>(v as u32, t503[0]); t503[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t503[2]); }
                    { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t502[0]) as usize * 8) as *const u64).read_unaligned(); t503[0] = vsetq_lane_u32::<1>(v as u32, t503[0]); t503[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t503[2]); }
                    { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t502[0]) as usize * 8) as *const u64).read_unaligned(); t503[0] = vsetq_lane_u32::<2>(v as u32, t503[0]); t503[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t503[2]); }
                    { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t502[0]) as usize * 8) as *const u64).read_unaligned(); t503[0] = vsetq_lane_u32::<3>(v as u32, t503[0]); t503[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t503[2]); }
                    { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t502[1]) as usize * 8) as *const u64).read_unaligned(); t503[1] = vsetq_lane_u32::<0>(v as u32, t503[1]); t503[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t503[3]); }
                    { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t502[1]) as usize * 8) as *const u64).read_unaligned(); t503[1] = vsetq_lane_u32::<1>(v as u32, t503[1]); t503[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t503[3]); }
                    { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t502[1]) as usize * 8) as *const u64).read_unaligned(); t503[1] = vsetq_lane_u32::<2>(v as u32, t503[1]); t503[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t503[3]); }
                    { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t502[1]) as usize * 8) as *const u64).read_unaligned(); t503[1] = vsetq_lane_u32::<3>(v as u32, t503[1]); t503[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t503[3]); }
                }
                t499 = t503;
            }
            x6 = t499;
            let t506 = (arg1 as *const u8).add(((gid0) as usize) * 8);
            let mut t507: [uint32x4_t; 4];
            if all(&live) {
                let t5080 = vld2q_u32((t506 as *const u32).add(0));
                let t5081 = vld2q_u32((t506 as *const u32).add(8));
                let t508: [uint32x4_t; 4] = [t5080.0, t5081.0, t5080.1, t5081.1];
                t507 = t508;
            } else {
                let t509 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t494[0]), vandq_u32(live[1], t494[1]))));
                let t510: [uint32x4_t; 2] = [vbslq_u32(live[0], t494[0], t509), vbslq_u32(live[1], t494[1], t509)];
                let mut t511: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
                let t512 = vminvq_u32(vminq_u32(t510[0], t510[1]));
                if t512 == vmaxvq_u32(vmaxq_u32(t510[0], t510[1])) {
                    let t513 = ((arg1 as *const u8).add(t512 as usize * 8) as *const u64).read_unaligned();
                    t511 = [vdupq_n_u32(t513 as u32), vdupq_n_u32(t513 as u32), vdupq_n_u32((t513 >> 32) as u32), vdupq_n_u32((t513 >> 32) as u32)];
                } else {
                    { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<0>(t510[0]) as usize * 8) as *const u64).read_unaligned(); t511[0] = vsetq_lane_u32::<0>(v as u32, t511[0]); t511[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t511[2]); }
                    { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<1>(t510[0]) as usize * 8) as *const u64).read_unaligned(); t511[0] = vsetq_lane_u32::<1>(v as u32, t511[0]); t511[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t511[2]); }
                    { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<2>(t510[0]) as usize * 8) as *const u64).read_unaligned(); t511[0] = vsetq_lane_u32::<2>(v as u32, t511[0]); t511[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t511[2]); }
                    { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<3>(t510[0]) as usize * 8) as *const u64).read_unaligned(); t511[0] = vsetq_lane_u32::<3>(v as u32, t511[0]); t511[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t511[2]); }
                    { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<0>(t510[1]) as usize * 8) as *const u64).read_unaligned(); t511[1] = vsetq_lane_u32::<0>(v as u32, t511[1]); t511[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t511[3]); }
                    { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<1>(t510[1]) as usize * 8) as *const u64).read_unaligned(); t511[1] = vsetq_lane_u32::<1>(v as u32, t511[1]); t511[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t511[3]); }
                    { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<2>(t510[1]) as usize * 8) as *const u64).read_unaligned(); t511[1] = vsetq_lane_u32::<2>(v as u32, t511[1]); t511[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t511[3]); }
                    { let v = ((arg1 as *const u8).add(vgetq_lane_u32::<3>(t510[1]) as usize * 8) as *const u64).read_unaligned(); t511[1] = vsetq_lane_u32::<3>(v as u32, t511[1]); t511[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t511[3]); }
                }
                t507 = t511;
            }
            x7 = t507;
            let mut t514: [uint32x4_t; 2];
            if all(&live) {
                let t515 = (arg2 as *const u32).add((gid0) as usize);
                t514 = [vld1q_u32(t515.add(0)), vld1q_u32(t515.add(4))];
            } else {
                let t516 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t494[0]), vandq_u32(live[1], t494[1]))));
                let t517: [uint32x4_t; 2] = [vbslq_u32(live[0], t494[0], t516), vbslq_u32(live[1], t494[1], t516)];
                let t518 = arg2 as *const u32;
                let mut t519: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                t519[0] = vld1q_lane_u32::<0>(t518.add(vgetq_lane_u32::<0>(t517[0]) as usize), t519[0]);
                t519[0] = vld1q_lane_u32::<1>(t518.add(vgetq_lane_u32::<1>(t517[0]) as usize), t519[0]);
                t519[0] = vld1q_lane_u32::<2>(t518.add(vgetq_lane_u32::<2>(t517[0]) as usize), t519[0]);
                t519[0] = vld1q_lane_u32::<3>(t518.add(vgetq_lane_u32::<3>(t517[0]) as usize), t519[0]);
                t519[1] = vld1q_lane_u32::<0>(t518.add(vgetq_lane_u32::<0>(t517[1]) as usize), t519[1]);
                t519[1] = vld1q_lane_u32::<1>(t518.add(vgetq_lane_u32::<1>(t517[1]) as usize), t519[1]);
                t519[1] = vld1q_lane_u32::<2>(t518.add(vgetq_lane_u32::<2>(t517[1]) as usize), t519[1]);
                t519[1] = vld1q_lane_u32::<3>(t518.add(vgetq_lane_u32::<3>(t517[1]) as usize), t519[1]);
                t514 = t519;
            }
            x8 = t514;
            let t520 = [vdupq_n_u32(63u32); 2];
            let t521: [uint32x4_t; 2] = [vandq_u32(x8[0], t520[0]), vandq_u32(x8[1], t520[1])];
            x9 = t521;
            x10 = a0;
            let t522: [uint32x4_t; 2] = [vaddq_u32(x6[0], x7[0]), vaddq_u32(x6[1], x7[1])];
            let t523: [uint32x4_t; 2] = [vcltq_u32(t522[0], x6[0]), vcltq_u32(t522[1], x6[1])];
            let t524: [uint32x4_t; 4] = [t522[0], t522[1], vsubq_u32(vaddq_u32(x6[2], x7[2]), t523[0]), vsubq_u32(vaddq_u32(x6[3], x7[3]), t523[1])];
            let t525 = (arg3 as *const u8).add(((gid0.wrapping_add((0u32).wrapping_mul(x10))) as usize) * 8);
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
            let t527: [uint32x4_t; 2] = [vcltq_u32(x6[0], x7[0]), vcltq_u32(x6[1], x7[1])];
            let t528: [uint32x4_t; 4] = [vsubq_u32(x6[0], x7[0]), vsubq_u32(x6[1], x7[1]), vaddq_u32(vsubq_u32(x6[2], x7[2]), t527[0]), vaddq_u32(vsubq_u32(x6[3], x7[3]), t527[1])];
            let t529 = (arg3 as *const u8).add(((gid0.wrapping_add((1u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t529 as *mut u32).add(0), uint32x4x2_t(t528[0], t528[2]));
                vst2q_u32((t529 as *mut u32).add(8), uint32x4x2_t(t528[1], t528[3]));
            } else {
                let mut t530w = 0u64;
                let t530 = core::hint::black_box(&raw mut t530w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t529 as *mut u64).add(0), t530).write_unaligned(((vgetq_lane_u32::<0>(t528[2]) as u64) << 32 | vgetq_lane_u32::<0>(t528[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t529 as *mut u64).add(1), t530).write_unaligned(((vgetq_lane_u32::<1>(t528[2]) as u64) << 32 | vgetq_lane_u32::<1>(t528[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t529 as *mut u64).add(2), t530).write_unaligned(((vgetq_lane_u32::<2>(t528[2]) as u64) << 32 | vgetq_lane_u32::<2>(t528[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t529 as *mut u64).add(3), t530).write_unaligned(((vgetq_lane_u32::<3>(t528[2]) as u64) << 32 | vgetq_lane_u32::<3>(t528[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t529 as *mut u64).add(4), t530).write_unaligned(((vgetq_lane_u32::<0>(t528[3]) as u64) << 32 | vgetq_lane_u32::<0>(t528[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t529 as *mut u64).add(5), t530).write_unaligned(((vgetq_lane_u32::<1>(t528[3]) as u64) << 32 | vgetq_lane_u32::<1>(t528[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t529 as *mut u64).add(6), t530).write_unaligned(((vgetq_lane_u32::<2>(t528[3]) as u64) << 32 | vgetq_lane_u32::<2>(t528[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t529 as *mut u64).add(7), t530).write_unaligned(((vgetq_lane_u32::<3>(t528[3]) as u64) << 32 | vgetq_lane_u32::<3>(t528[1]) as u64));
            }
            let t531: [uint32x4_t; 2] = [x9[0], x9[1]];
            let t532: [uint32x4_t; 2] = [vandq_u32(t531[0], vdupq_n_u32(63)), vandq_u32(t531[1], vdupq_n_u32(63))];
            let t533: [uint32x4_t; 4] = [vshlq_u32(x6[0], vreinterpretq_s32_u32(t532[0])), vshlq_u32(x6[1], vreinterpretq_s32_u32(t532[1])), vorrq_u32(vshlq_u32(x6[2], vreinterpretq_s32_u32(t532[0])), vshlq_u32(x6[0], vsubq_s32(vreinterpretq_s32_u32(t532[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(x6[3], vreinterpretq_s32_u32(t532[1])), vshlq_u32(x6[1], vsubq_s32(vreinterpretq_s32_u32(t532[1]), vdupq_n_s32(32))))];
            let t534 = (arg3 as *const u8).add(((gid0.wrapping_add((2u32).wrapping_mul(x10))) as usize) * 8);
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
            let t536: [uint32x4_t; 2] = [x9[0], x9[1]];
            let t537: [uint32x4_t; 2] = [vandq_u32(t536[0], vdupq_n_u32(63)), vandq_u32(t536[1], vdupq_n_u32(63))];
            let t538: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(x6[0], vnegq_s32(vreinterpretq_s32_u32(t537[0]))), vshlq_u32(x6[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t537[0])))), vorrq_u32(vshlq_u32(x6[1], vnegq_s32(vreinterpretq_s32_u32(t537[1]))), vshlq_u32(x6[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t537[1])))), vshlq_u32(x6[2], vnegq_s32(vreinterpretq_s32_u32(t537[0]))), vshlq_u32(x6[3], vnegq_s32(vreinterpretq_s32_u32(t537[1])))];
            let t539 = (arg3 as *const u8).add(((gid0.wrapping_add((3u32).wrapping_mul(x10))) as usize) * 8);
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
            let t541: [uint32x4_t; 2] = [x9[0], x9[1]];
            let t542: [uint32x4_t; 2] = [vandq_u32(t541[0], vdupq_n_u32(63)), vandq_u32(t541[1], vdupq_n_u32(63))];
            let t543: [uint32x4_t; 2] = [vcgeq_s32(vreinterpretq_s32_u32(t542[0]), vdupq_n_s32(32)), vcgeq_s32(vreinterpretq_s32_u32(t542[1]), vdupq_n_s32(32))];
            let t544: [uint32x4_t; 4] = [vbslq_u32(t543[0], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t542[0])))), vorrq_u32(vshlq_u32(x6[0], vnegq_s32(vreinterpretq_s32_u32(t542[0]))), vshlq_u32(x6[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t542[0]))))), vbslq_u32(t543[1], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t542[1])))), vorrq_u32(vshlq_u32(x6[1], vnegq_s32(vreinterpretq_s32_u32(t542[1]))), vshlq_u32(x6[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t542[1]))))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vnegq_s32(vreinterpretq_s32_u32(t542[0])))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vnegq_s32(vreinterpretq_s32_u32(t542[1]))))];
            let t545 = (arg3 as *const u8).add(((gid0.wrapping_add((4u32).wrapping_mul(x10))) as usize) * 8);
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
            let t547: [uint32x4_t; 4] = [vshlq_n_u32::<13>(x6[0]), vshlq_n_u32::<13>(x6[1]), vorrq_u32(vshlq_n_u32::<13>(x6[2]), vshrq_n_u32::<19>(x6[0])), vorrq_u32(vshlq_n_u32::<13>(x6[3]), vshrq_n_u32::<19>(x6[1]))];
            let t548: [uint32x4_t; 2] = [vshrq_n_u32::<19>(x7[2]), vshrq_n_u32::<19>(x7[3])];
            let t549: [uint32x4_t; 4] = [vorrq_u32(t547[0], t548[0]), vorrq_u32(t547[1], t548[1]), t547[2], t547[3]];
            let t550 = (arg3 as *const u8).add(((gid0.wrapping_add((5u32).wrapping_mul(x10))) as usize) * 8);
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
            let t552: [uint32x4_t; 4] = [vdupq_n_u32(0), vdupq_n_u32(0), vshlq_n_u32::<8>(x6[0]), vshlq_n_u32::<8>(x6[1])];
            let t553 = (arg3 as *const u8).add(((gid0.wrapping_add((6u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t553 as *mut u32).add(0), uint32x4x2_t(t552[0], t552[2]));
                vst2q_u32((t553 as *mut u32).add(8), uint32x4x2_t(t552[1], t552[3]));
            } else {
                let mut t554w = 0u64;
                let t554 = core::hint::black_box(&raw mut t554w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t553 as *mut u64).add(0), t554).write_unaligned(((vgetq_lane_u32::<0>(t552[2]) as u64) << 32 | vgetq_lane_u32::<0>(t552[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t553 as *mut u64).add(1), t554).write_unaligned(((vgetq_lane_u32::<1>(t552[2]) as u64) << 32 | vgetq_lane_u32::<1>(t552[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t553 as *mut u64).add(2), t554).write_unaligned(((vgetq_lane_u32::<2>(t552[2]) as u64) << 32 | vgetq_lane_u32::<2>(t552[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t553 as *mut u64).add(3), t554).write_unaligned(((vgetq_lane_u32::<3>(t552[2]) as u64) << 32 | vgetq_lane_u32::<3>(t552[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t553 as *mut u64).add(4), t554).write_unaligned(((vgetq_lane_u32::<0>(t552[3]) as u64) << 32 | vgetq_lane_u32::<0>(t552[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t553 as *mut u64).add(5), t554).write_unaligned(((vgetq_lane_u32::<1>(t552[3]) as u64) << 32 | vgetq_lane_u32::<1>(t552[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t553 as *mut u64).add(6), t554).write_unaligned(((vgetq_lane_u32::<2>(t552[3]) as u64) << 32 | vgetq_lane_u32::<2>(t552[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t553 as *mut u64).add(7), t554).write_unaligned(((vgetq_lane_u32::<3>(t552[3]) as u64) << 32 | vgetq_lane_u32::<3>(t552[1]) as u64));
            }
            let t555: [uint32x4_t; 2] = [vshrq_n_u32::<3>(x6[2]), vshrq_n_u32::<3>(x6[3])];
            let t556: [uint32x4_t; 4] = [t555[0], t555[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            let t557 = (arg3 as *const u8).add(((gid0.wrapping_add((7u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t557 as *mut u32).add(0), uint32x4x2_t(t556[0], t556[2]));
                vst2q_u32((t557 as *mut u32).add(8), uint32x4x2_t(t556[1], t556[3]));
            } else {
                let mut t558w = 0u64;
                let t558 = core::hint::black_box(&raw mut t558w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t557 as *mut u64).add(0), t558).write_unaligned(((vgetq_lane_u32::<0>(t556[2]) as u64) << 32 | vgetq_lane_u32::<0>(t556[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t557 as *mut u64).add(1), t558).write_unaligned(((vgetq_lane_u32::<1>(t556[2]) as u64) << 32 | vgetq_lane_u32::<1>(t556[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t557 as *mut u64).add(2), t558).write_unaligned(((vgetq_lane_u32::<2>(t556[2]) as u64) << 32 | vgetq_lane_u32::<2>(t556[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t557 as *mut u64).add(3), t558).write_unaligned(((vgetq_lane_u32::<3>(t556[2]) as u64) << 32 | vgetq_lane_u32::<3>(t556[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t557 as *mut u64).add(4), t558).write_unaligned(((vgetq_lane_u32::<0>(t556[3]) as u64) << 32 | vgetq_lane_u32::<0>(t556[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t557 as *mut u64).add(5), t558).write_unaligned(((vgetq_lane_u32::<1>(t556[3]) as u64) << 32 | vgetq_lane_u32::<1>(t556[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t557 as *mut u64).add(6), t558).write_unaligned(((vgetq_lane_u32::<2>(t556[3]) as u64) << 32 | vgetq_lane_u32::<2>(t556[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t557 as *mut u64).add(7), t558).write_unaligned(((vgetq_lane_u32::<3>(t556[3]) as u64) << 32 | vgetq_lane_u32::<3>(t556[1]) as u64));
            }
            let t559 = vdupq_n_s32(((((40i64) as u32)) & 63) as i32);
            let t560: [uint32x4_t; 2] = [vcgeq_s32(t559, vdupq_n_s32(32)), vcgeq_s32(t559, vdupq_n_s32(32))];
            let t561: [uint32x4_t; 4] = [vbslq_u32(t560[0], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vsubq_s32(vdupq_n_s32(32), t559))), vorrq_u32(vshlq_u32(x6[0], vnegq_s32(t559)), vshlq_u32(x6[2], vsubq_s32(vdupq_n_s32(32), t559)))), vbslq_u32(t560[1], vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vsubq_s32(vdupq_n_s32(32), t559))), vorrq_u32(vshlq_u32(x6[1], vnegq_s32(t559)), vshlq_u32(x6[3], vsubq_s32(vdupq_n_s32(32), t559)))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[2]), vnegq_s32(t559))), vreinterpretq_u32_s32(vshlq_s32(vreinterpretq_s32_u32(x6[3]), vnegq_s32(t559)))];
            let t562 = (arg3 as *const u8).add(((gid0.wrapping_add((8u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t562 as *mut u32).add(0), uint32x4x2_t(t561[0], t561[2]));
                vst2q_u32((t562 as *mut u32).add(8), uint32x4x2_t(t561[1], t561[3]));
            } else {
                let mut t563w = 0u64;
                let t563 = core::hint::black_box(&raw mut t563w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t562 as *mut u64).add(0), t563).write_unaligned(((vgetq_lane_u32::<0>(t561[2]) as u64) << 32 | vgetq_lane_u32::<0>(t561[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t562 as *mut u64).add(1), t563).write_unaligned(((vgetq_lane_u32::<1>(t561[2]) as u64) << 32 | vgetq_lane_u32::<1>(t561[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t562 as *mut u64).add(2), t563).write_unaligned(((vgetq_lane_u32::<2>(t561[2]) as u64) << 32 | vgetq_lane_u32::<2>(t561[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t562 as *mut u64).add(3), t563).write_unaligned(((vgetq_lane_u32::<3>(t561[2]) as u64) << 32 | vgetq_lane_u32::<3>(t561[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t562 as *mut u64).add(4), t563).write_unaligned(((vgetq_lane_u32::<0>(t561[3]) as u64) << 32 | vgetq_lane_u32::<0>(t561[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t562 as *mut u64).add(5), t563).write_unaligned(((vgetq_lane_u32::<1>(t561[3]) as u64) << 32 | vgetq_lane_u32::<1>(t561[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t562 as *mut u64).add(6), t563).write_unaligned(((vgetq_lane_u32::<2>(t561[3]) as u64) << 32 | vgetq_lane_u32::<2>(t561[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t562 as *mut u64).add(7), t563).write_unaligned(((vgetq_lane_u32::<3>(t561[3]) as u64) << 32 | vgetq_lane_u32::<3>(t561[1]) as u64));
            }
            let t564: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x6[0]), vget_low_u32(x7[0]))), vreinterpretq_u32_u64(vmull_high_u32(x6[0], x7[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x6[1]), vget_low_u32(x7[1]))), vreinterpretq_u32_u64(vmull_high_u32(x6[1], x7[1])))];
            let t565: [uint32x4_t; 4] = [vmulq_u32(x6[0], x7[0]), vmulq_u32(x6[1], x7[1]), vmlaq_u32(vmlaq_u32(t564[0], x6[0], x7[2]), x7[0], x6[2]), vmlaq_u32(vmlaq_u32(t564[1], x6[1], x7[3]), x7[1], x6[3])];
            let t566 = (arg3 as *const u8).add(((gid0.wrapping_add((9u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t566 as *mut u32).add(0), uint32x4x2_t(t565[0], t565[2]));
                vst2q_u32((t566 as *mut u32).add(8), uint32x4x2_t(t565[1], t565[3]));
            } else {
                let mut t567w = 0u64;
                let t567 = core::hint::black_box(&raw mut t567w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t566 as *mut u64).add(0), t567).write_unaligned(((vgetq_lane_u32::<0>(t565[2]) as u64) << 32 | vgetq_lane_u32::<0>(t565[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t566 as *mut u64).add(1), t567).write_unaligned(((vgetq_lane_u32::<1>(t565[2]) as u64) << 32 | vgetq_lane_u32::<1>(t565[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t566 as *mut u64).add(2), t567).write_unaligned(((vgetq_lane_u32::<2>(t565[2]) as u64) << 32 | vgetq_lane_u32::<2>(t565[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t566 as *mut u64).add(3), t567).write_unaligned(((vgetq_lane_u32::<3>(t565[2]) as u64) << 32 | vgetq_lane_u32::<3>(t565[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t566 as *mut u64).add(4), t567).write_unaligned(((vgetq_lane_u32::<0>(t565[3]) as u64) << 32 | vgetq_lane_u32::<0>(t565[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t566 as *mut u64).add(5), t567).write_unaligned(((vgetq_lane_u32::<1>(t565[3]) as u64) << 32 | vgetq_lane_u32::<1>(t565[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t566 as *mut u64).add(6), t567).write_unaligned(((vgetq_lane_u32::<2>(t565[3]) as u64) << 32 | vgetq_lane_u32::<2>(t565[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t566 as *mut u64).add(7), t567).write_unaligned(((vgetq_lane_u32::<3>(t565[3]) as u64) << 32 | vgetq_lane_u32::<3>(t565[1]) as u64));
            }
            let t568: u64 = 0u64;
            let t569 = [vdupq_n_u32((t568) as u32); 2];
            let t570: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x7[0], t569[0]), vceqq_u32(x7[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(x7[1], t569[1]), vceqq_u32(x7[3], vdupq_n_u32(0)))];
            let t571: [uint32x4_t; 2] = [vmvnq_u32(t570[0]), vmvnq_u32(t570[1])];
            let mut t572: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            let t573: [uint32x4_t; 2] = [vandq_u32(live[0], t571[0]), vandq_u32(live[1], t571[1])];
            let t574: [uint32x4_t; 2] = [vbicq_u32(live[0], t571[0]), vbicq_u32(live[1], t571[1])];
            {
                let (mut t575xl, mut t575xh, mut t575yl, mut t575yh) = ([0u32; 8], [0u32; 8], [0u32; 8], [0u32; 8]);
                vst1q_u32(t575xl.as_mut_ptr().add(0), x6[0]); vst1q_u32(t575xh.as_mut_ptr().add(0), x6[2]); vst1q_u32(t575yl.as_mut_ptr().add(0), x7[0]); vst1q_u32(t575yh.as_mut_ptr().add(0), x7[2]);
                vst1q_u32(t575xl.as_mut_ptr().add(4), x6[1]); vst1q_u32(t575xh.as_mut_ptr().add(4), x6[3]); vst1q_u32(t575yl.as_mut_ptr().add(4), x7[1]); vst1q_u32(t575yh.as_mut_ptr().add(4), x7[3]);
                for j in 0..8 { let q = (((t575xh[j] as u64) << 32 | t575xl[j] as u64) as u64).checked_div(((t575yh[j] as u64) << 32 | t575yl[j] as u64) as u64).unwrap_or(0) as u64; t575xl[j] = q as u32; t575xh[j] = (q >> 32) as u32; }
                let t576: [uint32x4_t; 4] = [vld1q_u32(t575xl.as_ptr().add(0)), vld1q_u32(t575xl.as_ptr().add(4)), vld1q_u32(t575xh.as_ptr().add(0)), vld1q_u32(t575xh.as_ptr().add(4))];
                t572 = [vbslq_u32(t573[0], t576[0], t572[0]), vbslq_u32(t573[1], t576[1], t572[1]), vbslq_u32(t573[0], t576[2], t572[2]), vbslq_u32(t573[1], t576[3], t572[3])];
            }
            {
                let t577: u64 = 7u64;
                let t578 = [vdupq_n_u32((t577) as u32); 2];
                let t579: [uint32x4_t; 4] = [t578[0], t578[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                t572 = [vbslq_u32(t574[0], t579[0], t572[0]), vbslq_u32(t574[1], t579[1], t572[1]), vbslq_u32(t574[0], t579[2], t572[2]), vbslq_u32(t574[1], t579[3], t572[3])];
            }
            let t580 = (arg3 as *const u8).add(((gid0.wrapping_add((10u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t580 as *mut u32).add(0), uint32x4x2_t(t572[0], t572[2]));
                vst2q_u32((t580 as *mut u32).add(8), uint32x4x2_t(t572[1], t572[3]));
            } else {
                let mut t581w = 0u64;
                let t581 = core::hint::black_box(&raw mut t581w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t580 as *mut u64).add(0), t581).write_unaligned(((vgetq_lane_u32::<0>(t572[2]) as u64) << 32 | vgetq_lane_u32::<0>(t572[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t580 as *mut u64).add(1), t581).write_unaligned(((vgetq_lane_u32::<1>(t572[2]) as u64) << 32 | vgetq_lane_u32::<1>(t572[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t580 as *mut u64).add(2), t581).write_unaligned(((vgetq_lane_u32::<2>(t572[2]) as u64) << 32 | vgetq_lane_u32::<2>(t572[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t580 as *mut u64).add(3), t581).write_unaligned(((vgetq_lane_u32::<3>(t572[2]) as u64) << 32 | vgetq_lane_u32::<3>(t572[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t580 as *mut u64).add(4), t581).write_unaligned(((vgetq_lane_u32::<0>(t572[3]) as u64) << 32 | vgetq_lane_u32::<0>(t572[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t580 as *mut u64).add(5), t581).write_unaligned(((vgetq_lane_u32::<1>(t572[3]) as u64) << 32 | vgetq_lane_u32::<1>(t572[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t580 as *mut u64).add(6), t581).write_unaligned(((vgetq_lane_u32::<2>(t572[3]) as u64) << 32 | vgetq_lane_u32::<2>(t572[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t580 as *mut u64).add(7), t581).write_unaligned(((vgetq_lane_u32::<3>(t572[3]) as u64) << 32 | vgetq_lane_u32::<3>(t572[1]) as u64));
            }
            let t582: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x6[2], x7[2]), vandq_u32(vceqq_u32(x6[2], x7[2]), vcltq_u32(x6[0], x7[0]))), vorrq_u32(vcltq_u32(x6[3], x7[3]), vandq_u32(vceqq_u32(x6[3], x7[3]), vcltq_u32(x6[1], x7[1])))];
            let t583: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t582[0]), vshrq_n_u32::<31>(t582[1])];
            let t584: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x7[2], x6[2]), vandq_u32(vceqq_u32(x7[2], x6[2]), vcltq_u32(x7[0], x6[0]))), vorrq_u32(vcltq_u32(x7[3], x6[3]), vandq_u32(vceqq_u32(x7[3], x6[3]), vcltq_u32(x7[1], x6[1])))];
            let t585: [uint32x4_t; 2] = [vmvnq_u32(t584[0]), vmvnq_u32(t584[1])];
            let t586: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t585[0]), vshrq_n_u32::<31>(t585[1])];
            let t587: [uint32x4_t; 4] = [vshlq_n_u32::<1>(t586[0]), vshlq_n_u32::<1>(t586[1]), vshrq_n_u32::<31>(t586[0]), vshrq_n_u32::<31>(t586[1])];
            let t588: [uint32x4_t; 4] = [vorrq_u32(t583[0], t587[0]), vorrq_u32(t583[1], t587[1]), t587[2], t587[3]];
            let t589: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x6[0], x7[0]), vceqq_u32(x6[2], x7[2])), vandq_u32(vceqq_u32(x6[1], x7[1]), vceqq_u32(x6[3], x7[3]))];
            let t590: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t589[0]), vshrq_n_u32::<31>(t589[1])];
            let t591: [uint32x4_t; 4] = [vshlq_n_u32::<2>(t590[0]), vshlq_n_u32::<2>(t590[1]), vshrq_n_u32::<30>(t590[0]), vshrq_n_u32::<30>(t590[1])];
            let t592: [uint32x4_t; 4] = [vorrq_u32(t588[0], t591[0]), vorrq_u32(t588[1], t591[1]), vorrq_u32(t588[2], t591[2]), vorrq_u32(t588[3], t591[3])];
            let t593: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x6[0], x7[0]), vceqq_u32(x6[2], x7[2])), vandq_u32(vceqq_u32(x6[1], x7[1]), vceqq_u32(x6[3], x7[3]))];
            let t594: [uint32x4_t; 2] = [vmvnq_u32(t593[0]), vmvnq_u32(t593[1])];
            let t595: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t594[0]), vshrq_n_u32::<31>(t594[1])];
            let t596: [uint32x4_t; 4] = [vshlq_n_u32::<3>(t595[0]), vshlq_n_u32::<3>(t595[1]), vshrq_n_u32::<29>(t595[0]), vshrq_n_u32::<29>(t595[1])];
            let t597: [uint32x4_t; 4] = [vorrq_u32(t592[0], t596[0]), vorrq_u32(t592[1], t596[1]), vorrq_u32(t592[2], t596[2]), vorrq_u32(t592[3], t596[3])];
            let t598: [uint32x4_t; 2] = [vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x6[2]), vreinterpretq_s32_u32(x7[2])), vandq_u32(vceqq_u32(x6[2], x7[2]), vcltq_u32(x6[0], x7[0]))), vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x6[3]), vreinterpretq_s32_u32(x7[3])), vandq_u32(vceqq_u32(x6[3], x7[3]), vcltq_u32(x6[1], x7[1])))];
            let t599: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t598[0]), vshrq_n_u32::<31>(t598[1])];
            let t600: [uint32x4_t; 4] = [vshlq_n_u32::<4>(t599[0]), vshlq_n_u32::<4>(t599[1]), vshrq_n_u32::<28>(t599[0]), vshrq_n_u32::<28>(t599[1])];
            let t601: [uint32x4_t; 4] = [vorrq_u32(t597[0], t600[0]), vorrq_u32(t597[1], t600[1]), vorrq_u32(t597[2], t600[2]), vorrq_u32(t597[3], t600[3])];
            let t602: [uint32x4_t; 2] = [vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x6[2]), vreinterpretq_s32_u32(x7[2])), vandq_u32(vceqq_u32(x6[2], x7[2]), vcltq_u32(x6[0], x7[0]))), vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(x6[3]), vreinterpretq_s32_u32(x7[3])), vandq_u32(vceqq_u32(x6[3], x7[3]), vcltq_u32(x6[1], x7[1])))];
            let t603: [uint32x4_t; 2] = [vmvnq_u32(t602[0]), vmvnq_u32(t602[1])];
            let t604: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t603[0]), vshrq_n_u32::<31>(t603[1])];
            let t605: [uint32x4_t; 4] = [vshlq_n_u32::<5>(t604[0]), vshlq_n_u32::<5>(t604[1]), vshrq_n_u32::<27>(t604[0]), vshrq_n_u32::<27>(t604[1])];
            let t606: [uint32x4_t; 4] = [vorrq_u32(t601[0], t605[0]), vorrq_u32(t601[1], t605[1]), vorrq_u32(t601[2], t605[2]), vorrq_u32(t601[3], t605[3])];
            let t607: [uint32x4_t; 2] = [vorrq_u32(vcltq_u32(x7[2], x6[2]), vandq_u32(vceqq_u32(x7[2], x6[2]), vcltq_u32(x7[0], x6[0]))), vorrq_u32(vcltq_u32(x7[3], x6[3]), vandq_u32(vceqq_u32(x7[3], x6[3]), vcltq_u32(x7[1], x6[1])))];
            let t608: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t607[0]), vshrq_n_u32::<31>(t607[1])];
            let t609: [uint32x4_t; 4] = [vshlq_n_u32::<6>(t608[0]), vshlq_n_u32::<6>(t608[1]), vshrq_n_u32::<26>(t608[0]), vshrq_n_u32::<26>(t608[1])];
            let t610: [uint32x4_t; 4] = [vorrq_u32(t606[0], t609[0]), vorrq_u32(t606[1], t609[1]), vorrq_u32(t606[2], t609[2]), vorrq_u32(t606[3], t609[3])];
            let t611 = (arg3 as *const u8).add(((gid0.wrapping_add((11u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t611 as *mut u32).add(0), uint32x4x2_t(t610[0], t610[2]));
                vst2q_u32((t611 as *mut u32).add(8), uint32x4x2_t(t610[1], t610[3]));
            } else {
                let mut t612w = 0u64;
                let t612 = core::hint::black_box(&raw mut t612w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t611 as *mut u64).add(0), t612).write_unaligned(((vgetq_lane_u32::<0>(t610[2]) as u64) << 32 | vgetq_lane_u32::<0>(t610[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t611 as *mut u64).add(1), t612).write_unaligned(((vgetq_lane_u32::<1>(t610[2]) as u64) << 32 | vgetq_lane_u32::<1>(t610[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t611 as *mut u64).add(2), t612).write_unaligned(((vgetq_lane_u32::<2>(t610[2]) as u64) << 32 | vgetq_lane_u32::<2>(t610[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t611 as *mut u64).add(3), t612).write_unaligned(((vgetq_lane_u32::<3>(t610[2]) as u64) << 32 | vgetq_lane_u32::<3>(t610[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t611 as *mut u64).add(4), t612).write_unaligned(((vgetq_lane_u32::<0>(t610[3]) as u64) << 32 | vgetq_lane_u32::<0>(t610[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t611 as *mut u64).add(5), t612).write_unaligned(((vgetq_lane_u32::<1>(t610[3]) as u64) << 32 | vgetq_lane_u32::<1>(t610[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t611 as *mut u64).add(6), t612).write_unaligned(((vgetq_lane_u32::<2>(t610[3]) as u64) << 32 | vgetq_lane_u32::<2>(t610[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t611 as *mut u64).add(7), t612).write_unaligned(((vgetq_lane_u32::<3>(t610[3]) as u64) << 32 | vgetq_lane_u32::<3>(t610[1]) as u64));
            }
            let t613: [uint32x4_t; 2] = [vaddq_u32(x8[0], x6[0]), vaddq_u32(x8[1], x6[1])];
            let t614: [uint32x4_t; 2] = [vcltq_u32(t613[0], x8[0]), vcltq_u32(t613[1], x8[1])];
            let t615: [uint32x4_t; 4] = [t613[0], t613[1], vsubq_u32(vaddq_u32(vdupq_n_u32(0), x6[2]), t614[0]), vsubq_u32(vaddq_u32(vdupq_n_u32(0), x6[3]), t614[1])];
            let t616 = (arg3 as *const u8).add(((gid0.wrapping_add((12u32).wrapping_mul(x10))) as usize) * 8);
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
            let t618: [uint32x4_t; 2] = [vreinterpretq_u32_s32(vshrq_n_s32::<31>(vreinterpretq_s32_u32(x8[0]))), vreinterpretq_u32_s32(vshrq_n_s32::<31>(vreinterpretq_s32_u32(x8[1])))];
            let t619: [uint32x4_t; 4] = [x8[0], x8[1], t618[0], t618[1]];
            let t620 = (arg3 as *const u8).add(((gid0.wrapping_add((13u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t620 as *mut u32).add(0), uint32x4x2_t(t619[0], t619[2]));
                vst2q_u32((t620 as *mut u32).add(8), uint32x4x2_t(t619[1], t619[3]));
            } else {
                let mut t621w = 0u64;
                let t621 = core::hint::black_box(&raw mut t621w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t620 as *mut u64).add(0), t621).write_unaligned(((vgetq_lane_u32::<0>(t619[2]) as u64) << 32 | vgetq_lane_u32::<0>(t619[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t620 as *mut u64).add(1), t621).write_unaligned(((vgetq_lane_u32::<1>(t619[2]) as u64) << 32 | vgetq_lane_u32::<1>(t619[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t620 as *mut u64).add(2), t621).write_unaligned(((vgetq_lane_u32::<2>(t619[2]) as u64) << 32 | vgetq_lane_u32::<2>(t619[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t620 as *mut u64).add(3), t621).write_unaligned(((vgetq_lane_u32::<3>(t619[2]) as u64) << 32 | vgetq_lane_u32::<3>(t619[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t620 as *mut u64).add(4), t621).write_unaligned(((vgetq_lane_u32::<0>(t619[3]) as u64) << 32 | vgetq_lane_u32::<0>(t619[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t620 as *mut u64).add(5), t621).write_unaligned(((vgetq_lane_u32::<1>(t619[3]) as u64) << 32 | vgetq_lane_u32::<1>(t619[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t620 as *mut u64).add(6), t621).write_unaligned(((vgetq_lane_u32::<2>(t619[3]) as u64) << 32 | vgetq_lane_u32::<2>(t619[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t620 as *mut u64).add(7), t621).write_unaligned(((vgetq_lane_u32::<3>(t619[3]) as u64) << 32 | vgetq_lane_u32::<3>(t619[1]) as u64));
            }
            let t622: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<3>(x6[0]), vshlq_n_u32::<29>(x6[2])), vorrq_u32(vshrq_n_u32::<3>(x6[1]), vshlq_n_u32::<29>(x6[3])), vshrq_n_u32::<3>(x6[2]), vshrq_n_u32::<3>(x6[3])];
            let t623: [uint32x4_t; 2] = [t622[0], t622[1]];
            let t624: [uint32x4_t; 4] = [t623[0], t623[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            let t625 = (arg3 as *const u8).add(((gid0.wrapping_add((14u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t625 as *mut u32).add(0), uint32x4x2_t(t624[0], t624[2]));
                vst2q_u32((t625 as *mut u32).add(8), uint32x4x2_t(t624[1], t624[3]));
            } else {
                let mut t626w = 0u64;
                let t626 = core::hint::black_box(&raw mut t626w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t625 as *mut u64).add(0), t626).write_unaligned(((vgetq_lane_u32::<0>(t624[2]) as u64) << 32 | vgetq_lane_u32::<0>(t624[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t625 as *mut u64).add(1), t626).write_unaligned(((vgetq_lane_u32::<1>(t624[2]) as u64) << 32 | vgetq_lane_u32::<1>(t624[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t625 as *mut u64).add(2), t626).write_unaligned(((vgetq_lane_u32::<2>(t624[2]) as u64) << 32 | vgetq_lane_u32::<2>(t624[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t625 as *mut u64).add(3), t626).write_unaligned(((vgetq_lane_u32::<3>(t624[2]) as u64) << 32 | vgetq_lane_u32::<3>(t624[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t625 as *mut u64).add(4), t626).write_unaligned(((vgetq_lane_u32::<0>(t624[3]) as u64) << 32 | vgetq_lane_u32::<0>(t624[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t625 as *mut u64).add(5), t626).write_unaligned(((vgetq_lane_u32::<1>(t624[3]) as u64) << 32 | vgetq_lane_u32::<1>(t624[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t625 as *mut u64).add(6), t626).write_unaligned(((vgetq_lane_u32::<2>(t624[3]) as u64) << 32 | vgetq_lane_u32::<2>(t624[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t625 as *mut u64).add(7), t626).write_unaligned(((vgetq_lane_u32::<3>(t624[3]) as u64) << 32 | vgetq_lane_u32::<3>(t624[1]) as u64));
            }
            let t627: u64 = 1u64;
            let t628 = [vdupq_n_u32((t627) as u32); 2];
            let t629: [uint32x4_t; 2] = [vandq_u32(x6[0], t628[0]), vandq_u32(x6[1], t628[1])];
            let t630: [uint32x4_t; 2] = [vtstq_u32(t629[0], t629[0]), vtstq_u32(t629[1], t629[1])];
            let t631: [uint32x4_t; 2] = [vandq_u32(live[0], t630[0]), vandq_u32(live[1], t630[1])];
            {
                let t632: [uint32x4_t; 4] = [veorq_u32(x6[0], x7[0]), veorq_u32(x6[1], x7[1]), veorq_u32(x6[2], x7[2]), veorq_u32(x6[3], x7[3])];
                x11 = [vbslq_u32(t631[0], t632[0], x11[0]), vbslq_u32(t631[1], t632[1], x11[1]), vbslq_u32(t631[0], t632[2], x11[2]), vbslq_u32(t631[1], t632[3], x11[3])];
            }
            let t633: [uint32x4_t; 2] = [vbicq_u32(live[0], t630[0]), vbicq_u32(live[1], t630[1])];
            {
                let t634: [uint32x4_t; 2] = [vaddq_u32(x6[0], x8[0]), vaddq_u32(x6[1], x8[1])];
                let t635: [uint32x4_t; 2] = [vcltq_u32(t634[0], x6[0]), vcltq_u32(t634[1], x6[1])];
                let t636: [uint32x4_t; 4] = [t634[0], t634[1], vsubq_u32(vaddq_u32(x6[2], vdupq_n_u32(0)), t635[0]), vsubq_u32(vaddq_u32(x6[3], vdupq_n_u32(0)), t635[1])];
                x11 = [vbslq_u32(t633[0], t636[0], x11[0]), vbslq_u32(t633[1], t636[1], x11[1]), vbslq_u32(t633[0], t636[2], x11[2]), vbslq_u32(t633[1], t636[3], x11[3])];
            }
            let t637 = (arg3 as *const u8).add(((gid0.wrapping_add((15u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t637 as *mut u32).add(0), uint32x4x2_t(x11[0], x11[2]));
                vst2q_u32((t637 as *mut u32).add(8), uint32x4x2_t(x11[1], x11[3]));
            } else {
                let mut t638w = 0u64;
                let t638 = core::hint::black_box(&raw mut t638w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t637 as *mut u64).add(0), t638).write_unaligned(((vgetq_lane_u32::<0>(x11[2]) as u64) << 32 | vgetq_lane_u32::<0>(x11[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t637 as *mut u64).add(1), t638).write_unaligned(((vgetq_lane_u32::<1>(x11[2]) as u64) << 32 | vgetq_lane_u32::<1>(x11[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t637 as *mut u64).add(2), t638).write_unaligned(((vgetq_lane_u32::<2>(x11[2]) as u64) << 32 | vgetq_lane_u32::<2>(x11[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t637 as *mut u64).add(3), t638).write_unaligned(((vgetq_lane_u32::<3>(x11[2]) as u64) << 32 | vgetq_lane_u32::<3>(x11[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t637 as *mut u64).add(4), t638).write_unaligned(((vgetq_lane_u32::<0>(x11[3]) as u64) << 32 | vgetq_lane_u32::<0>(x11[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t637 as *mut u64).add(5), t638).write_unaligned(((vgetq_lane_u32::<1>(x11[3]) as u64) << 32 | vgetq_lane_u32::<1>(x11[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t637 as *mut u64).add(6), t638).write_unaligned(((vgetq_lane_u32::<2>(x11[3]) as u64) << 32 | vgetq_lane_u32::<2>(x11[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t637 as *mut u64).add(7), t638).write_unaligned(((vgetq_lane_u32::<3>(x11[3]) as u64) << 32 | vgetq_lane_u32::<3>(x11[1]) as u64));
            }
            let t639 = b1_l_p(l, lid0, live, x6);
            let t640 = b2_l_p(l, lid0, live, x7);
            let t641: [uint32x4_t; 4] = [vshlq_n_u32::<8>(t640[0]), vshlq_n_u32::<8>(t640[1]), vshrq_n_u32::<24>(t640[0]), vshrq_n_u32::<24>(t640[1])];
            let t642: [uint32x4_t; 2] = [vaddq_u32(t639[0], t641[0]), vaddq_u32(t639[1], t641[1])];
            let t643: [uint32x4_t; 2] = [vcltq_u32(t642[0], t639[0]), vcltq_u32(t642[1], t639[1])];
            let t644: [uint32x4_t; 4] = [t642[0], t642[1], vsubq_u32(vaddq_u32(vdupq_n_u32(0), t641[2]), t643[0]), vsubq_u32(vaddq_u32(vdupq_n_u32(0), t641[3]), t643[1])];
            let t645 = (arg3 as *const u8).add(((gid0.wrapping_add((16u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t645 as *mut u32).add(0), uint32x4x2_t(t644[0], t644[2]));
                vst2q_u32((t645 as *mut u32).add(8), uint32x4x2_t(t644[1], t644[3]));
            } else {
                let mut t646w = 0u64;
                let t646 = core::hint::black_box(&raw mut t646w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t645 as *mut u64).add(0), t646).write_unaligned(((vgetq_lane_u32::<0>(t644[2]) as u64) << 32 | vgetq_lane_u32::<0>(t644[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t645 as *mut u64).add(1), t646).write_unaligned(((vgetq_lane_u32::<1>(t644[2]) as u64) << 32 | vgetq_lane_u32::<1>(t644[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t645 as *mut u64).add(2), t646).write_unaligned(((vgetq_lane_u32::<2>(t644[2]) as u64) << 32 | vgetq_lane_u32::<2>(t644[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t645 as *mut u64).add(3), t646).write_unaligned(((vgetq_lane_u32::<3>(t644[2]) as u64) << 32 | vgetq_lane_u32::<3>(t644[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t645 as *mut u64).add(4), t646).write_unaligned(((vgetq_lane_u32::<0>(t644[3]) as u64) << 32 | vgetq_lane_u32::<0>(t644[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t645 as *mut u64).add(5), t646).write_unaligned(((vgetq_lane_u32::<1>(t644[3]) as u64) << 32 | vgetq_lane_u32::<1>(t644[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t645 as *mut u64).add(6), t646).write_unaligned(((vgetq_lane_u32::<2>(t644[3]) as u64) << 32 | vgetq_lane_u32::<2>(t644[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t645 as *mut u64).add(7), t646).write_unaligned(((vgetq_lane_u32::<3>(t644[3]) as u64) << 32 | vgetq_lane_u32::<3>(t644[1]) as u64));
            }
            let t647 = [vdupq_n_u32(x10); 2];
            let mut t648x = [0u32; 8];
            let mut t648y = [0u32; 8];
            for r in 0..2 { vst1q_u32(t648x.as_mut_ptr().add(4 * r), x8[r]); vst1q_u32(t648y.as_mut_ptr().add(4 * r), t647[r]); }
            for j in 0..8 { t648x[j] = (t648x[j] as u32).checked_rem(t648y[j] as u32).unwrap_or(0) as u32; }
            let t649: [uint32x4_t; 2] = [vld1q_u32(t648x.as_ptr().add(0)), vld1q_u32(t648x.as_ptr().add(4))];
            let t650 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t649[0]), vandq_u32(live[1], t649[1]))));
            let t651: [uint32x4_t; 2] = [vbslq_u32(live[0], t649[0], t650), vbslq_u32(live[1], t649[1], t650)];
            let mut t652: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            let t653 = vminvq_u32(vminq_u32(t651[0], t651[1]));
            if t653 == vmaxvq_u32(vmaxq_u32(t651[0], t651[1])) {
                let t654 = ((arg0 as *const u8).add(t653 as usize * 8) as *const u64).read_unaligned();
                t652 = [vdupq_n_u32(t654 as u32), vdupq_n_u32(t654 as u32), vdupq_n_u32((t654 >> 32) as u32), vdupq_n_u32((t654 >> 32) as u32)];
            } else {
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t651[0]) as usize * 8) as *const u64).read_unaligned(); t652[0] = vsetq_lane_u32::<0>(v as u32, t652[0]); t652[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t652[2]); }
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t651[0]) as usize * 8) as *const u64).read_unaligned(); t652[0] = vsetq_lane_u32::<1>(v as u32, t652[0]); t652[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t652[2]); }
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t651[0]) as usize * 8) as *const u64).read_unaligned(); t652[0] = vsetq_lane_u32::<2>(v as u32, t652[0]); t652[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t652[2]); }
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t651[0]) as usize * 8) as *const u64).read_unaligned(); t652[0] = vsetq_lane_u32::<3>(v as u32, t652[0]); t652[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t652[2]); }
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t651[1]) as usize * 8) as *const u64).read_unaligned(); t652[1] = vsetq_lane_u32::<0>(v as u32, t652[1]); t652[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t652[3]); }
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t651[1]) as usize * 8) as *const u64).read_unaligned(); t652[1] = vsetq_lane_u32::<1>(v as u32, t652[1]); t652[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t652[3]); }
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t651[1]) as usize * 8) as *const u64).read_unaligned(); t652[1] = vsetq_lane_u32::<2>(v as u32, t652[1]); t652[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t652[3]); }
                { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t651[1]) as usize * 8) as *const u64).read_unaligned(); t652[1] = vsetq_lane_u32::<3>(v as u32, t652[1]); t652[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t652[3]); }
            }
            let t655 = (arg3 as *const u8).add(((gid0.wrapping_add((17u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t655 as *mut u32).add(0), uint32x4x2_t(t652[0], t652[2]));
                vst2q_u32((t655 as *mut u32).add(8), uint32x4x2_t(t652[1], t652[3]));
            } else {
                let mut t656w = 0u64;
                let t656 = core::hint::black_box(&raw mut t656w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t655 as *mut u64).add(0), t656).write_unaligned(((vgetq_lane_u32::<0>(t652[2]) as u64) << 32 | vgetq_lane_u32::<0>(t652[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t655 as *mut u64).add(1), t656).write_unaligned(((vgetq_lane_u32::<1>(t652[2]) as u64) << 32 | vgetq_lane_u32::<1>(t652[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t655 as *mut u64).add(2), t656).write_unaligned(((vgetq_lane_u32::<2>(t652[2]) as u64) << 32 | vgetq_lane_u32::<2>(t652[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t655 as *mut u64).add(3), t656).write_unaligned(((vgetq_lane_u32::<3>(t652[2]) as u64) << 32 | vgetq_lane_u32::<3>(t652[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t655 as *mut u64).add(4), t656).write_unaligned(((vgetq_lane_u32::<0>(t652[3]) as u64) << 32 | vgetq_lane_u32::<0>(t652[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t655 as *mut u64).add(5), t656).write_unaligned(((vgetq_lane_u32::<1>(t652[3]) as u64) << 32 | vgetq_lane_u32::<1>(t652[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t655 as *mut u64).add(6), t656).write_unaligned(((vgetq_lane_u32::<2>(t652[3]) as u64) << 32 | vgetq_lane_u32::<2>(t652[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t655 as *mut u64).add(7), t656).write_unaligned(((vgetq_lane_u32::<3>(t652[3]) as u64) << 32 | vgetq_lane_u32::<3>(t652[1]) as u64));
            }
            let t657 = b3_ll_p(l, lid0, live, x6, x8);
            let t658 = (arg3 as *const u8).add(((gid0.wrapping_add((18u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t658 as *mut u32).add(0), uint32x4x2_t(t657[0], t657[2]));
                vst2q_u32((t658 as *mut u32).add(8), uint32x4x2_t(t657[1], t657[3]));
            } else {
                let mut t659w = 0u64;
                let t659 = core::hint::black_box(&raw mut t659w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t658 as *mut u64).add(0), t659).write_unaligned(((vgetq_lane_u32::<0>(t657[2]) as u64) << 32 | vgetq_lane_u32::<0>(t657[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t658 as *mut u64).add(1), t659).write_unaligned(((vgetq_lane_u32::<1>(t657[2]) as u64) << 32 | vgetq_lane_u32::<1>(t657[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t658 as *mut u64).add(2), t659).write_unaligned(((vgetq_lane_u32::<2>(t657[2]) as u64) << 32 | vgetq_lane_u32::<2>(t657[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t658 as *mut u64).add(3), t659).write_unaligned(((vgetq_lane_u32::<3>(t657[2]) as u64) << 32 | vgetq_lane_u32::<3>(t657[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t658 as *mut u64).add(4), t659).write_unaligned(((vgetq_lane_u32::<0>(t657[3]) as u64) << 32 | vgetq_lane_u32::<0>(t657[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t658 as *mut u64).add(5), t659).write_unaligned(((vgetq_lane_u32::<1>(t657[3]) as u64) << 32 | vgetq_lane_u32::<1>(t657[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t658 as *mut u64).add(6), t659).write_unaligned(((vgetq_lane_u32::<2>(t657[3]) as u64) << 32 | vgetq_lane_u32::<2>(t657[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t658 as *mut u64).add(7), t659).write_unaligned(((vgetq_lane_u32::<3>(t657[3]) as u64) << 32 | vgetq_lane_u32::<3>(t657[1]) as u64));
            }
            x12 = x6;
            x13 = 0u32;
            let mut t664 = live;
            loop {
                let t666 = bits(&t664);
                if t666.count_ones() <= 2 {
                    let mut t667 = t666;
                    while t667 != 0 {
                        let j = t667.trailing_zeros() as usize;
                        t667 &= t667 - 1;
                        let t668 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x7);
                        let mut x7_j: u64 = ((t668[8 + j] as u64) << 32 | t668[j] as u64);
                        let t669 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x8);
                        let mut x8_j: u32 = t669[j];
                        let t670 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x12);
                        let mut x12_j: u64 = ((t670[8 + j] as u64) << 32 | t670[j] as u64);
                        let mut x13_j: u32 = x13;
                        loop {
                            if !(((x13_j) < (((x8_j) & (7u32))))) { break; }
                            x12_j = ((x12_j).wrapping_mul(3u64)).wrapping_add(x7_j);
                            let t671 = x13_j;
                            x13_j = (x13_j).wrapping_add(1u32);
                        }
                        let mut t672 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x12);
                        t672[j] = x12_j as u32;
                        t672[8 + j] = (x12_j as u64 >> 32) as u32;
                        x12 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t672);
                    }
                    break;
                }
                // Lanes part: each its own rounds.
                let t673 = [vdupq_n_u32(7u32); 2];
                let t674: [uint32x4_t; 2] = [vandq_u32(x8[0], t673[0]), vandq_u32(x8[1], t673[1])];
                let t675 = [vdupq_n_u32(x13); 2];
                let t676: [uint32x4_t; 2] = [vcltq_u32(t675[0], t674[0]), vcltq_u32(t675[1], t674[1])];
                t664 = [vandq_u32(t664[0], t676[0]), vandq_u32(t664[1], t676[1])];
                if !(any(&t664)) { break; }
                let t678: u64 = 3u64;
                let t679 = [vdupq_n_u32((t678) as u32); 2];
                let t680: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x12[0]), vget_low_u32(t679[0]))), vreinterpretq_u32_u64(vmull_high_u32(x12[0], t679[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x12[1]), vget_low_u32(t679[1]))), vreinterpretq_u32_u64(vmull_high_u32(x12[1], t679[1])))];
                let t681: [uint32x4_t; 4] = [vmulq_u32(x12[0], t679[0]), vmulq_u32(x12[1], t679[1]), vmlaq_u32(t680[0], t679[0], x12[2]), vmlaq_u32(t680[1], t679[1], x12[3])];
                let t682: [uint32x4_t; 2] = [vaddq_u32(t681[0], x7[0]), vaddq_u32(t681[1], x7[1])];
                let t683: [uint32x4_t; 2] = [vcltq_u32(t682[0], t681[0]), vcltq_u32(t682[1], t681[1])];
                let t684: [uint32x4_t; 4] = [t682[0], t682[1], vsubq_u32(vaddq_u32(t681[2], x7[2]), t683[0]), vsubq_u32(vaddq_u32(t681[3], x7[3]), t683[1])];
                x12 = [vbslq_u32(t664[0], t684[0], x12[0]), vbslq_u32(t664[1], t684[1], x12[1]), vbslq_u32(t664[0], t684[2], x12[2]), vbslq_u32(t664[1], t684[3], x12[3])];
                let t685: [uint32x4_t; 2] = [vandq_u32(t664[0], live[0]), vandq_u32(t664[1], live[1])];
                let t686 = x13;
                x13 = (x13).wrapping_add(1u32);
            }
            let t687 = (arg3 as *const u8).add(((gid0.wrapping_add((19u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t687 as *mut u32).add(0), uint32x4x2_t(x12[0], x12[2]));
                vst2q_u32((t687 as *mut u32).add(8), uint32x4x2_t(x12[1], x12[3]));
            } else {
                let mut t688w = 0u64;
                let t688 = core::hint::black_box(&raw mut t688w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t687 as *mut u64).add(0), t688).write_unaligned(((vgetq_lane_u32::<0>(x12[2]) as u64) << 32 | vgetq_lane_u32::<0>(x12[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t687 as *mut u64).add(1), t688).write_unaligned(((vgetq_lane_u32::<1>(x12[2]) as u64) << 32 | vgetq_lane_u32::<1>(x12[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t687 as *mut u64).add(2), t688).write_unaligned(((vgetq_lane_u32::<2>(x12[2]) as u64) << 32 | vgetq_lane_u32::<2>(x12[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t687 as *mut u64).add(3), t688).write_unaligned(((vgetq_lane_u32::<3>(x12[2]) as u64) << 32 | vgetq_lane_u32::<3>(x12[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t687 as *mut u64).add(4), t688).write_unaligned(((vgetq_lane_u32::<0>(x12[3]) as u64) << 32 | vgetq_lane_u32::<0>(x12[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t687 as *mut u64).add(5), t688).write_unaligned(((vgetq_lane_u32::<1>(x12[3]) as u64) << 32 | vgetq_lane_u32::<1>(x12[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t687 as *mut u64).add(6), t688).write_unaligned(((vgetq_lane_u32::<2>(x12[3]) as u64) << 32 | vgetq_lane_u32::<2>(x12[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t687 as *mut u64).add(7), t688).write_unaligned(((vgetq_lane_u32::<3>(x12[3]) as u64) << 32 | vgetq_lane_u32::<3>(x12[1]) as u64));
            }
            let t689: u64 = a1;
            let t690 = [vdupq_n_u32((t689) as u32); 2];
            let t691 = [vdupq_n_u32(((t689) >> 32) as u32); 2];
            let t692: [uint32x4_t; 2] = [vaddq_u32(x6[0], t690[0]), vaddq_u32(x6[1], t690[1])];
            let t693: [uint32x4_t; 2] = [vcltq_u32(t692[0], x6[0]), vcltq_u32(t692[1], x6[1])];
            let t694: [uint32x4_t; 4] = [t692[0], t692[1], vsubq_u32(vaddq_u32(x6[2], t691[0]), t693[0]), vsubq_u32(vaddq_u32(x6[3], t691[1]), t693[1])];
            let t695: u64 = (a1).wrapping_shl((3u64) as u32);
            let t696 = [vdupq_n_u32((t695) as u32); 2];
            let t697 = [vdupq_n_u32(((t695) >> 32) as u32); 2];
            let t698: [uint32x4_t; 2] = [vaddq_u32(t694[0], t696[0]), vaddq_u32(t694[1], t696[1])];
            let t699: [uint32x4_t; 2] = [vcltq_u32(t698[0], t694[0]), vcltq_u32(t698[1], t694[1])];
            let t700: [uint32x4_t; 4] = [t698[0], t698[1], vsubq_u32(vaddq_u32(t694[2], t697[0]), t699[0]), vsubq_u32(vaddq_u32(t694[3], t697[1]), t699[1])];
            let t701 = (arg3 as *const u8).add(((gid0.wrapping_add((20u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t701 as *mut u32).add(0), uint32x4x2_t(t700[0], t700[2]));
                vst2q_u32((t701 as *mut u32).add(8), uint32x4x2_t(t700[1], t700[3]));
            } else {
                let mut t702w = 0u64;
                let t702 = core::hint::black_box(&raw mut t702w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t701 as *mut u64).add(0), t702).write_unaligned(((vgetq_lane_u32::<0>(t700[2]) as u64) << 32 | vgetq_lane_u32::<0>(t700[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t701 as *mut u64).add(1), t702).write_unaligned(((vgetq_lane_u32::<1>(t700[2]) as u64) << 32 | vgetq_lane_u32::<1>(t700[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t701 as *mut u64).add(2), t702).write_unaligned(((vgetq_lane_u32::<2>(t700[2]) as u64) << 32 | vgetq_lane_u32::<2>(t700[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t701 as *mut u64).add(3), t702).write_unaligned(((vgetq_lane_u32::<3>(t700[2]) as u64) << 32 | vgetq_lane_u32::<3>(t700[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t701 as *mut u64).add(4), t702).write_unaligned(((vgetq_lane_u32::<0>(t700[3]) as u64) << 32 | vgetq_lane_u32::<0>(t700[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t701 as *mut u64).add(5), t702).write_unaligned(((vgetq_lane_u32::<1>(t700[3]) as u64) << 32 | vgetq_lane_u32::<1>(t700[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t701 as *mut u64).add(6), t702).write_unaligned(((vgetq_lane_u32::<2>(t700[3]) as u64) << 32 | vgetq_lane_u32::<2>(t700[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t701 as *mut u64).add(7), t702).write_unaligned(((vgetq_lane_u32::<3>(t700[3]) as u64) << 32 | vgetq_lane_u32::<3>(t700[1]) as u64));
            }
            let t703: u64 = 4294967295u64;
            let t704 = [vdupq_n_u32((t703) as u32); 2];
            let t705: [uint32x4_t; 2] = [vandq_u32(x6[0], t704[0]), vandq_u32(x6[1], t704[1])];
            let t706: u64 = 4294967295u64;
            let t707 = [vdupq_n_u32((t706) as u32); 2];
            let t708: [uint32x4_t; 2] = [vandq_u32(x7[0], t707[0]), vandq_u32(x7[1], t707[1])];
            let t709: [uint32x4_t; 2] = [vceqq_u32(t705[0], t708[0]), vceqq_u32(t705[1], t708[1])];
            let mut t710: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t711: [uint32x4_t; 2] = [vandq_u32(live[0], t709[0]), vandq_u32(live[1], t709[1])];
            let t712: [uint32x4_t; 2] = [vbicq_u32(live[0], t709[0]), vbicq_u32(live[1], t709[1])];
            {
                let t713 = [vdupq_n_u32(1u32); 2];
                t710 = [vbslq_u32(t711[0], t713[0], t710[0]), vbslq_u32(t711[1], t713[1], t710[1])];
            }
            {
                let t714 = [vdupq_n_u32(2u32); 2];
                t710 = [vbslq_u32(t712[0], t714[0], t710[0]), vbslq_u32(t712[1], t714[1], t710[1])];
            }
            let t715: [uint32x4_t; 4] = [t710[0], t710[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            let t716 = (arg3 as *const u8).add(((gid0.wrapping_add((21u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t716 as *mut u32).add(0), uint32x4x2_t(t715[0], t715[2]));
                vst2q_u32((t716 as *mut u32).add(8), uint32x4x2_t(t715[1], t715[3]));
            } else {
                let mut t717w = 0u64;
                let t717 = core::hint::black_box(&raw mut t717w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t716 as *mut u64).add(0), t717).write_unaligned(((vgetq_lane_u32::<0>(t715[2]) as u64) << 32 | vgetq_lane_u32::<0>(t715[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t716 as *mut u64).add(1), t717).write_unaligned(((vgetq_lane_u32::<1>(t715[2]) as u64) << 32 | vgetq_lane_u32::<1>(t715[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t716 as *mut u64).add(2), t717).write_unaligned(((vgetq_lane_u32::<2>(t715[2]) as u64) << 32 | vgetq_lane_u32::<2>(t715[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t716 as *mut u64).add(3), t717).write_unaligned(((vgetq_lane_u32::<3>(t715[2]) as u64) << 32 | vgetq_lane_u32::<3>(t715[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t716 as *mut u64).add(4), t717).write_unaligned(((vgetq_lane_u32::<0>(t715[3]) as u64) << 32 | vgetq_lane_u32::<0>(t715[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t716 as *mut u64).add(5), t717).write_unaligned(((vgetq_lane_u32::<1>(t715[3]) as u64) << 32 | vgetq_lane_u32::<1>(t715[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t716 as *mut u64).add(6), t717).write_unaligned(((vgetq_lane_u32::<2>(t715[3]) as u64) << 32 | vgetq_lane_u32::<2>(t715[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t716 as *mut u64).add(7), t717).write_unaligned(((vgetq_lane_u32::<3>(t715[3]) as u64) << 32 | vgetq_lane_u32::<3>(t715[1]) as u64));
            }
            let t718: [uint32x4_t; 2] = [vtstq_u32(vorrq_u32(x6[0], x6[2]), vorrq_u32(x6[0], x6[2])), vtstq_u32(vorrq_u32(x6[1], x6[3]), vorrq_u32(x6[1], x6[3]))];
            let mut t719: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t720: [uint32x4_t; 2] = [vandq_u32(live[0], t718[0]), vandq_u32(live[1], t718[1])];
            let t721: [uint32x4_t; 2] = [vbicq_u32(live[0], t718[0]), vbicq_u32(live[1], t718[1])];
            {
                let t722 = [vdupq_n_u32(5u32); 2];
                t719 = [vbslq_u32(t720[0], t722[0], t719[0]), vbslq_u32(t720[1], t722[1], t719[1])];
            }
            {
                let t723 = [vdupq_n_u32(6u32); 2];
                t719 = [vbslq_u32(t721[0], t723[0], t719[0]), vbslq_u32(t721[1], t723[1], t719[1])];
            }
            let t724: [uint32x4_t; 4] = [t719[0], t719[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            let t725 = (arg3 as *const u8).add(((gid0.wrapping_add((22u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t725 as *mut u32).add(0), uint32x4x2_t(t724[0], t724[2]));
                vst2q_u32((t725 as *mut u32).add(8), uint32x4x2_t(t724[1], t724[3]));
            } else {
                let mut t726w = 0u64;
                let t726 = core::hint::black_box(&raw mut t726w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t725 as *mut u64).add(0), t726).write_unaligned(((vgetq_lane_u32::<0>(t724[2]) as u64) << 32 | vgetq_lane_u32::<0>(t724[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t725 as *mut u64).add(1), t726).write_unaligned(((vgetq_lane_u32::<1>(t724[2]) as u64) << 32 | vgetq_lane_u32::<1>(t724[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t725 as *mut u64).add(2), t726).write_unaligned(((vgetq_lane_u32::<2>(t724[2]) as u64) << 32 | vgetq_lane_u32::<2>(t724[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t725 as *mut u64).add(3), t726).write_unaligned(((vgetq_lane_u32::<3>(t724[2]) as u64) << 32 | vgetq_lane_u32::<3>(t724[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t725 as *mut u64).add(4), t726).write_unaligned(((vgetq_lane_u32::<0>(t724[3]) as u64) << 32 | vgetq_lane_u32::<0>(t724[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t725 as *mut u64).add(5), t726).write_unaligned(((vgetq_lane_u32::<1>(t724[3]) as u64) << 32 | vgetq_lane_u32::<1>(t724[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t725 as *mut u64).add(6), t726).write_unaligned(((vgetq_lane_u32::<2>(t724[3]) as u64) << 32 | vgetq_lane_u32::<2>(t724[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t725 as *mut u64).add(7), t726).write_unaligned(((vgetq_lane_u32::<3>(t724[3]) as u64) << 32 | vgetq_lane_u32::<3>(t724[1]) as u64));
            }
            let t727 = b4_ll_p(l, lid0, live, x6, x7);
            let t728 = (arg3 as *const u8).add(((gid0.wrapping_add((23u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t728 as *mut u32).add(0), uint32x4x2_t(t727[0], t727[2]));
                vst2q_u32((t728 as *mut u32).add(8), uint32x4x2_t(t727[1], t727[3]));
            } else {
                let mut t729w = 0u64;
                let t729 = core::hint::black_box(&raw mut t729w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t728 as *mut u64).add(0), t729).write_unaligned(((vgetq_lane_u32::<0>(t727[2]) as u64) << 32 | vgetq_lane_u32::<0>(t727[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t728 as *mut u64).add(1), t729).write_unaligned(((vgetq_lane_u32::<1>(t727[2]) as u64) << 32 | vgetq_lane_u32::<1>(t727[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t728 as *mut u64).add(2), t729).write_unaligned(((vgetq_lane_u32::<2>(t727[2]) as u64) << 32 | vgetq_lane_u32::<2>(t727[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t728 as *mut u64).add(3), t729).write_unaligned(((vgetq_lane_u32::<3>(t727[2]) as u64) << 32 | vgetq_lane_u32::<3>(t727[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t728 as *mut u64).add(4), t729).write_unaligned(((vgetq_lane_u32::<0>(t727[3]) as u64) << 32 | vgetq_lane_u32::<0>(t727[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t728 as *mut u64).add(5), t729).write_unaligned(((vgetq_lane_u32::<1>(t727[3]) as u64) << 32 | vgetq_lane_u32::<1>(t727[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t728 as *mut u64).add(6), t729).write_unaligned(((vgetq_lane_u32::<2>(t727[3]) as u64) << 32 | vgetq_lane_u32::<2>(t727[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t728 as *mut u64).add(7), t729).write_unaligned(((vgetq_lane_u32::<3>(t727[3]) as u64) << 32 | vgetq_lane_u32::<3>(t727[1]) as u64));
            }
            let t730: u32;
            if ((((a1) & (1u64))) != (0u64)) {
                t730 = 8u32;
            } else {
                t730 = 4u32;
            }
            x14 = t730;
            let t731 = b5_r0ns_p(l, lid0, live, arg0, gid0.wrapping_mul(x14), x14, 8u32);
            let t732 = (arg3 as *const u8).add(((gid0.wrapping_add((24u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t732 as *mut u32).add(0), uint32x4x2_t(t731[0], t731[2]));
                vst2q_u32((t732 as *mut u32).add(8), uint32x4x2_t(t731[1], t731[3]));
            } else {
                let mut t733w = 0u64;
                let t733 = core::hint::black_box(&raw mut t733w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t732 as *mut u64).add(0), t733).write_unaligned(((vgetq_lane_u32::<0>(t731[2]) as u64) << 32 | vgetq_lane_u32::<0>(t731[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t732 as *mut u64).add(1), t733).write_unaligned(((vgetq_lane_u32::<1>(t731[2]) as u64) << 32 | vgetq_lane_u32::<1>(t731[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t732 as *mut u64).add(2), t733).write_unaligned(((vgetq_lane_u32::<2>(t731[2]) as u64) << 32 | vgetq_lane_u32::<2>(t731[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t732 as *mut u64).add(3), t733).write_unaligned(((vgetq_lane_u32::<3>(t731[2]) as u64) << 32 | vgetq_lane_u32::<3>(t731[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t732 as *mut u64).add(4), t733).write_unaligned(((vgetq_lane_u32::<0>(t731[3]) as u64) << 32 | vgetq_lane_u32::<0>(t731[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t732 as *mut u64).add(5), t733).write_unaligned(((vgetq_lane_u32::<1>(t731[3]) as u64) << 32 | vgetq_lane_u32::<1>(t731[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t732 as *mut u64).add(6), t733).write_unaligned(((vgetq_lane_u32::<2>(t731[3]) as u64) << 32 | vgetq_lane_u32::<2>(t731[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t732 as *mut u64).add(7), t733).write_unaligned(((vgetq_lane_u32::<3>(t731[3]) as u64) << 32 | vgetq_lane_u32::<3>(t731[1]) as u64));
            }
            let t734 = if (gid0.wrapping_add(3u32) & 255u32) <= 248u32 { b6_u8_p(l, lid0, live, gid0.wrapping_add(3u32)) } else { b6_i_p(l, lid0, live, gid0.wrapping_add(3u32)) };
            let t735 = (arg3 as *const u8).add(((gid0.wrapping_add((25u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t735 as *mut u32).add(0), uint32x4x2_t(t734[0], t734[2]));
                vst2q_u32((t735 as *mut u32).add(8), uint32x4x2_t(t734[1], t734[3]));
            } else {
                let mut t736w = 0u64;
                let t736 = core::hint::black_box(&raw mut t736w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t735 as *mut u64).add(0), t736).write_unaligned(((vgetq_lane_u32::<0>(t734[2]) as u64) << 32 | vgetq_lane_u32::<0>(t734[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t735 as *mut u64).add(1), t736).write_unaligned(((vgetq_lane_u32::<1>(t734[2]) as u64) << 32 | vgetq_lane_u32::<1>(t734[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t735 as *mut u64).add(2), t736).write_unaligned(((vgetq_lane_u32::<2>(t734[2]) as u64) << 32 | vgetq_lane_u32::<2>(t734[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t735 as *mut u64).add(3), t736).write_unaligned(((vgetq_lane_u32::<3>(t734[2]) as u64) << 32 | vgetq_lane_u32::<3>(t734[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t735 as *mut u64).add(4), t736).write_unaligned(((vgetq_lane_u32::<0>(t734[3]) as u64) << 32 | vgetq_lane_u32::<0>(t734[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t735 as *mut u64).add(5), t736).write_unaligned(((vgetq_lane_u32::<1>(t734[3]) as u64) << 32 | vgetq_lane_u32::<1>(t734[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t735 as *mut u64).add(6), t736).write_unaligned(((vgetq_lane_u32::<2>(t734[3]) as u64) << 32 | vgetq_lane_u32::<2>(t734[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t735 as *mut u64).add(7), t736).write_unaligned(((vgetq_lane_u32::<3>(t734[3]) as u64) << 32 | vgetq_lane_u32::<3>(t734[1]) as u64));
            }
            let t737: u64 = 3u64;
            let t738 = [vdupq_n_u32((t737) as u32); 2];
            let t739: [uint32x4_t; 2] = [vandq_u32(x6[0], t738[0]), vandq_u32(x6[1], t738[1])];
            let t740: u64 = 0u64;
            let t741 = [vdupq_n_u32((t740) as u32); 2];
            let t742: [uint32x4_t; 2] = [vceqq_u32(t739[0], t741[0]), vceqq_u32(t739[1], t741[1])];
            let mut t743: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            let t744: [uint32x4_t; 2] = [vandq_u32(live[0], t742[0]), vandq_u32(live[1], t742[1])];
            let t745: [uint32x4_t; 2] = [vbicq_u32(live[0], t742[0]), vbicq_u32(live[1], t742[1])];
            if any(&t744) {
                let t746: [uint32x4_t; 4];
                let t747 = bits(&t744);
                if t747.count_ones() <= 2 {
                    let t748 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x7);
                    let t749 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x8);
                    let mut t750 = [0u32; 16];
                    let mut t751 = t747;
                    while t751 != 0 {
                        let j = t751.trailing_zeros() as usize;
                        t751 &= t751 - 1;
                        let t752 = b7_ss_f(l, lid0, ((t748[8 + j] as u64) << 32 | t748[j] as u64), t749[j]);
                        let t753: u64 = t752;
                        t750[j] = t753 as u32;
                        t750[8 + j] = (t753 >> 32) as u32;
                    }
                    t746 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t750);
                } else {
                    let t754 = b7_ll_p(l, lid0, t744, x7, x8);
                    t746 = t754;
                }
                t743 = [vbslq_u32(t744[0], t746[0], t743[0]), vbslq_u32(t744[1], t746[1], t743[1]), vbslq_u32(t744[0], t746[2], t743[2]), vbslq_u32(t744[1], t746[3], t743[3])];
            }
            {
                t743 = [vbslq_u32(t745[0], x6[0], t743[0]), vbslq_u32(t745[1], x6[1], t743[1]), vbslq_u32(t745[0], x6[2], t743[2]), vbslq_u32(t745[1], x6[3], t743[3])];
            }
            let t755 = (arg3 as *const u8).add(((gid0.wrapping_add((26u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t755 as *mut u32).add(0), uint32x4x2_t(t743[0], t743[2]));
                vst2q_u32((t755 as *mut u32).add(8), uint32x4x2_t(t743[1], t743[3]));
            } else {
                let mut t756w = 0u64;
                let t756 = core::hint::black_box(&raw mut t756w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t755 as *mut u64).add(0), t756).write_unaligned(((vgetq_lane_u32::<0>(t743[2]) as u64) << 32 | vgetq_lane_u32::<0>(t743[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t755 as *mut u64).add(1), t756).write_unaligned(((vgetq_lane_u32::<1>(t743[2]) as u64) << 32 | vgetq_lane_u32::<1>(t743[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t755 as *mut u64).add(2), t756).write_unaligned(((vgetq_lane_u32::<2>(t743[2]) as u64) << 32 | vgetq_lane_u32::<2>(t743[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t755 as *mut u64).add(3), t756).write_unaligned(((vgetq_lane_u32::<3>(t743[2]) as u64) << 32 | vgetq_lane_u32::<3>(t743[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t755 as *mut u64).add(4), t756).write_unaligned(((vgetq_lane_u32::<0>(t743[3]) as u64) << 32 | vgetq_lane_u32::<0>(t743[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t755 as *mut u64).add(5), t756).write_unaligned(((vgetq_lane_u32::<1>(t743[3]) as u64) << 32 | vgetq_lane_u32::<1>(t743[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t755 as *mut u64).add(6), t756).write_unaligned(((vgetq_lane_u32::<2>(t743[3]) as u64) << 32 | vgetq_lane_u32::<2>(t743[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t755 as *mut u64).add(7), t756).write_unaligned(((vgetq_lane_u32::<3>(t743[3]) as u64) << 32 | vgetq_lane_u32::<3>(t743[1]) as u64));
            }
            x15 = (((((a1) & (7u64))) as u32)).wrapping_add(1u32);
            let t757 = b5_r0ns_p(l, lid0, live, arg0, gid0.wrapping_mul(x15), x15, x15);
            let t758 = b8_s_p(l, lid0, live, (8u32).wrapping_mul(x15));
            let t759: u64 = t758;
            let t760 = [vdupq_n_u32((t759) as u32); 2];
            let t761 = [vdupq_n_u32(((t759) >> 32) as u32); 2];
            let t762: [uint32x4_t; 4] = [vandq_u32(t757[0], t760[0]), vandq_u32(t757[1], t760[1]), vandq_u32(t757[2], t761[0]), vandq_u32(t757[3], t761[1])];
            let t763 = (arg3 as *const u8).add(((gid0.wrapping_add((27u32).wrapping_mul(x10))) as usize) * 8);
            if all(&live) {
                vst2q_u32((t763 as *mut u32).add(0), uint32x4x2_t(t762[0], t762[2]));
                vst2q_u32((t763 as *mut u32).add(8), uint32x4x2_t(t762[1], t762[3]));
            } else {
                let mut t764w = 0u64;
                let t764 = core::hint::black_box(&raw mut t764w) as *mut u64;
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, (t763 as *mut u64).add(0), t764).write_unaligned(((vgetq_lane_u32::<0>(t762[2]) as u64) << 32 | vgetq_lane_u32::<0>(t762[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, (t763 as *mut u64).add(1), t764).write_unaligned(((vgetq_lane_u32::<1>(t762[2]) as u64) << 32 | vgetq_lane_u32::<1>(t762[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, (t763 as *mut u64).add(2), t764).write_unaligned(((vgetq_lane_u32::<2>(t762[2]) as u64) << 32 | vgetq_lane_u32::<2>(t762[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, (t763 as *mut u64).add(3), t764).write_unaligned(((vgetq_lane_u32::<3>(t762[2]) as u64) << 32 | vgetq_lane_u32::<3>(t762[0]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, (t763 as *mut u64).add(4), t764).write_unaligned(((vgetq_lane_u32::<0>(t762[3]) as u64) << 32 | vgetq_lane_u32::<0>(t762[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, (t763 as *mut u64).add(5), t764).write_unaligned(((vgetq_lane_u32::<1>(t762[3]) as u64) << 32 | vgetq_lane_u32::<1>(t762[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, (t763 as *mut u64).add(6), t764).write_unaligned(((vgetq_lane_u32::<2>(t762[3]) as u64) << 32 | vgetq_lane_u32::<2>(t762[1]) as u64));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, (t763 as *mut u64).add(7), t764).write_unaligned(((vgetq_lane_u32::<3>(t762[3]) as u64) << 32 | vgetq_lane_u32::<3>(t762[1]) as u64));
            }
        }
    }
}

#[inline(always)]
unsafe fn b1_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t765: [uint32x4_t; 2] = [vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[0])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[2]))))), vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[1])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[3])))))];
    res = t765;
    return res;
}

#[inline(always)]
unsafe fn b1_l_f(l: &Launch, lid0: u32, x0: [uint32x4_t; 4]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t766: [uint32x4_t; 2] = [vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[0])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[2]))))), vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[1])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[3])))))];
    res = t766;
    return res;
}

#[inline(always)]
unsafe fn b2_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t767: [uint32x4_t; 2] = [vbslq_u32(vceqzq_u32(x0[2]), vaddq_u32(vclzq_u32(x0[0]), vdupq_n_u32(32)), vclzq_u32(x0[2])), vbslq_u32(vceqzq_u32(x0[3]), vaddq_u32(vclzq_u32(x0[1]), vdupq_n_u32(32)), vclzq_u32(x0[3]))];
    res = t767;
    return res;
}

#[inline(always)]
unsafe fn b2_l_f(l: &Launch, lid0: u32, x0: [uint32x4_t; 4]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t768: [uint32x4_t; 2] = [vbslq_u32(vceqzq_u32(x0[2]), vaddq_u32(vclzq_u32(x0[0]), vdupq_n_u32(32)), vclzq_u32(x0[2])), vbslq_u32(vceqzq_u32(x0[3]), vaddq_u32(vclzq_u32(x0[1]), vdupq_n_u32(32)), vclzq_u32(x0[3]))];
    res = t768;
    return res;
}

#[inline(always)]
unsafe fn b3_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], x1: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x2: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t769: u64 = 1u64;
    let t770 = [vdupq_n_u32((t769) as u32); 2];
    let t771: [uint32x4_t; 2] = [vandq_u32(x0[0], t770[0]), vandq_u32(x0[1], t770[1])];
    let t772: u64 = 0u64;
    let t773 = [vdupq_n_u32((t772) as u32); 2];
    let t774: [uint32x4_t; 2] = [vceqq_u32(t771[0], t773[0]), vceqq_u32(t771[1], t773[1])];
    let t775: [uint32x4_t; 2] = [vandq_u32(live[0], t774[0]), vandq_u32(live[1], t774[1])];
    {
        let t776: [uint32x4_t; 4] = [x1[0], x1[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = [vbslq_u32(t775[0], t776[0], res[0]), vbslq_u32(t775[1], t776[1], res[1]), vbslq_u32(t775[0], t776[2], res[2]), vbslq_u32(t775[1], t776[3], res[3])];
        live = [vbicq_u32(live[0], t775[0]), vbicq_u32(live[1], t775[1])];
    }
    if any(&live) {
        let t777: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x0[0]), vshlq_n_u32::<30>(x0[2])), vorrq_u32(vshrq_n_u32::<2>(x0[1]), vshlq_n_u32::<30>(x0[3])), vshrq_n_u32::<2>(x0[2]), vshrq_n_u32::<2>(x0[3])];
        let t778: [uint32x4_t; 2] = [t777[0], t777[1]];
        x2 = t778;
        let t779 = [vdupq_n_u32(1u32); 2];
        let t780: [uint32x4_t; 2] = [vsubq_u32(x2[0], t779[0]), vsubq_u32(x2[1], t779[1])];
        let t781: [uint32x4_t; 2] = [vcgeq_u32(t780[0], x1[0]), vcgeq_u32(t780[1], x1[1])];
        let t782: [uint32x4_t; 2] = [vandq_u32(live[0], t781[0]), vandq_u32(live[1], t781[1])];
        {
            let t783: u64 = 4294967295u64;
            let t784 = [vdupq_n_u32((t783) as u32); 2];
            let t785: [uint32x4_t; 4] = [t784[0], t784[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t782[0], t785[0], res[0]), vbslq_u32(t782[1], t785[1], res[1]), vbslq_u32(t782[0], t785[2], res[2]), vbslq_u32(t782[1], t785[3], res[3])];
            live = [vbicq_u32(live[0], t782[0]), vbicq_u32(live[1], t782[1])];
        }
        if any(&live) {
            let t786: u64 = 2u64;
            let t787 = [vdupq_n_u32((t786) as u32); 2];
            let t788: [uint32x4_t; 2] = [vandq_u32(x0[0], t787[0]), vandq_u32(x0[1], t787[1])];
            let t789: u64 = 0u64;
            let t790 = [vdupq_n_u32((t789) as u32); 2];
            let t791: [uint32x4_t; 2] = [vceqq_u32(t788[0], t790[0]), vceqq_u32(t788[1], t790[1])];
            let t792: [uint32x4_t; 2] = [vmvnq_u32(t791[0]), vmvnq_u32(t791[1])];
            let t793: [uint32x4_t; 2] = [vandq_u32(live[0], t792[0]), vandq_u32(live[1], t792[1])];
            {
                let t794: [uint32x4_t; 2] = [vsubq_u32(x1[0], x2[0]), vsubq_u32(x1[1], x2[1])];
                let t795: [uint32x4_t; 4] = [t794[0], t794[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t793[0], t795[0], res[0]), vbslq_u32(t793[1], t795[1], res[1]), vbslq_u32(t793[0], t795[2], res[2]), vbslq_u32(t793[1], t795[3], res[3])];
                live = [vbicq_u32(live[0], t793[0]), vbicq_u32(live[1], t793[1])];
            }
            if any(&live) {
                let t796: [uint32x4_t; 2] = [vsubq_u32(x1[0], x2[0]), vsubq_u32(x1[1], x2[1])];
                let t797: u64 = (1u64).wrapping_shl((32u64) as u32);
                let t798 = [vdupq_n_u32((t797) as u32); 2];
                let t799 = [vdupq_n_u32(((t797) >> 32) as u32); 2];
                let t800: [uint32x4_t; 4] = [vorrq_u32(t798[0], t796[0]), vorrq_u32(t798[1], t796[1]), t799[0], t799[1]];
                res = [vbslq_u32(live[0], t800[0], res[0]), vbslq_u32(live[1], t800[1], res[1]), vbslq_u32(live[0], t800[2], res[2]), vbslq_u32(live[1], t800[3], res[3])];
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
    let t801: u64 = 1u64;
    let t802 = [vdupq_n_u32((t801) as u32); 2];
    let t803: [uint32x4_t; 2] = [vandq_u32(x0[0], t802[0]), vandq_u32(x0[1], t802[1])];
    let t804: u64 = 0u64;
    let t805 = [vdupq_n_u32((t804) as u32); 2];
    let t806: [uint32x4_t; 2] = [vceqq_u32(t803[0], t805[0]), vceqq_u32(t803[1], t805[1])];
    {
        let t807: [uint32x4_t; 4] = [x1[0], x1[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = [vbslq_u32(t806[0], t807[0], res[0]), vbslq_u32(t806[1], t807[1], res[1]), vbslq_u32(t806[0], t807[2], res[2]), vbslq_u32(t806[1], t807[3], res[3])];
        live = [vbicq_u32(live[0], t806[0]), vbicq_u32(live[1], t806[1])];
    }
    if any(&live) {
        let t808: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x0[0]), vshlq_n_u32::<30>(x0[2])), vorrq_u32(vshrq_n_u32::<2>(x0[1]), vshlq_n_u32::<30>(x0[3])), vshrq_n_u32::<2>(x0[2]), vshrq_n_u32::<2>(x0[3])];
        let t809: [uint32x4_t; 2] = [t808[0], t808[1]];
        x2 = t809;
        let t810 = [vdupq_n_u32(1u32); 2];
        let t811: [uint32x4_t; 2] = [vsubq_u32(x2[0], t810[0]), vsubq_u32(x2[1], t810[1])];
        let t812: [uint32x4_t; 2] = [vcgeq_u32(t811[0], x1[0]), vcgeq_u32(t811[1], x1[1])];
        let t813: [uint32x4_t; 2] = [vandq_u32(live[0], t812[0]), vandq_u32(live[1], t812[1])];
        {
            let t814: u64 = 4294967295u64;
            let t815 = [vdupq_n_u32((t814) as u32); 2];
            let t816: [uint32x4_t; 4] = [t815[0], t815[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t813[0], t816[0], res[0]), vbslq_u32(t813[1], t816[1], res[1]), vbslq_u32(t813[0], t816[2], res[2]), vbslq_u32(t813[1], t816[3], res[3])];
            live = [vbicq_u32(live[0], t813[0]), vbicq_u32(live[1], t813[1])];
        }
        if any(&live) {
            let t817: u64 = 2u64;
            let t818 = [vdupq_n_u32((t817) as u32); 2];
            let t819: [uint32x4_t; 2] = [vandq_u32(x0[0], t818[0]), vandq_u32(x0[1], t818[1])];
            let t820: u64 = 0u64;
            let t821 = [vdupq_n_u32((t820) as u32); 2];
            let t822: [uint32x4_t; 2] = [vceqq_u32(t819[0], t821[0]), vceqq_u32(t819[1], t821[1])];
            let t823: [uint32x4_t; 2] = [vmvnq_u32(t822[0]), vmvnq_u32(t822[1])];
            let t824: [uint32x4_t; 2] = [vandq_u32(live[0], t823[0]), vandq_u32(live[1], t823[1])];
            {
                let t825: [uint32x4_t; 2] = [vsubq_u32(x1[0], x2[0]), vsubq_u32(x1[1], x2[1])];
                let t826: [uint32x4_t; 4] = [t825[0], t825[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t824[0], t826[0], res[0]), vbslq_u32(t824[1], t826[1], res[1]), vbslq_u32(t824[0], t826[2], res[2]), vbslq_u32(t824[1], t826[3], res[3])];
                live = [vbicq_u32(live[0], t824[0]), vbicq_u32(live[1], t824[1])];
            }
            if any(&live) {
                let t827: [uint32x4_t; 2] = [vsubq_u32(x1[0], x2[0]), vsubq_u32(x1[1], x2[1])];
                let t828: u64 = (1u64).wrapping_shl((32u64) as u32);
                let t829 = [vdupq_n_u32((t828) as u32); 2];
                let t830 = [vdupq_n_u32(((t828) >> 32) as u32); 2];
                let t831: [uint32x4_t; 4] = [vorrq_u32(t829[0], t827[0]), vorrq_u32(t829[1], t827[1]), t830[0], t830[1]];
                res = [vbslq_u32(live[0], t831[0], res[0]), vbslq_u32(live[1], t831[1], res[1]), vbslq_u32(live[0], t831[2], res[2]), vbslq_u32(live[1], t831[3], res[3])];
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
    let (mut t832l, mut t832h) = ([0u32; 8], [0u32; 8]);
    { let v = (((((vgetq_lane_u32::<0>(x0[2]) as u64) << 32 | vgetq_lane_u32::<0>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<0>(x1[2]) as u64) << 32 | vgetq_lane_u32::<0>(x1[0]) as u64) as u64) as u128) >> 64) as u64; t832l[0] = v as u32; t832h[0] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<1>(x0[2]) as u64) << 32 | vgetq_lane_u32::<1>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<1>(x1[2]) as u64) << 32 | vgetq_lane_u32::<1>(x1[0]) as u64) as u64) as u128) >> 64) as u64; t832l[1] = v as u32; t832h[1] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<2>(x0[2]) as u64) << 32 | vgetq_lane_u32::<2>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<2>(x1[2]) as u64) << 32 | vgetq_lane_u32::<2>(x1[0]) as u64) as u64) as u128) >> 64) as u64; t832l[2] = v as u32; t832h[2] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<3>(x0[2]) as u64) << 32 | vgetq_lane_u32::<3>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<3>(x1[2]) as u64) << 32 | vgetq_lane_u32::<3>(x1[0]) as u64) as u64) as u128) >> 64) as u64; t832l[3] = v as u32; t832h[3] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<0>(x0[3]) as u64) << 32 | vgetq_lane_u32::<0>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<0>(x1[3]) as u64) << 32 | vgetq_lane_u32::<0>(x1[1]) as u64) as u64) as u128) >> 64) as u64; t832l[4] = v as u32; t832h[4] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<1>(x0[3]) as u64) << 32 | vgetq_lane_u32::<1>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<1>(x1[3]) as u64) << 32 | vgetq_lane_u32::<1>(x1[1]) as u64) as u64) as u128) >> 64) as u64; t832l[5] = v as u32; t832h[5] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<2>(x0[3]) as u64) << 32 | vgetq_lane_u32::<2>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<2>(x1[3]) as u64) << 32 | vgetq_lane_u32::<2>(x1[1]) as u64) as u64) as u128) >> 64) as u64; t832l[6] = v as u32; t832h[6] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<3>(x0[3]) as u64) << 32 | vgetq_lane_u32::<3>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<3>(x1[3]) as u64) << 32 | vgetq_lane_u32::<3>(x1[1]) as u64) as u64) as u128) >> 64) as u64; t832l[7] = v as u32; t832h[7] = (v >> 32) as u32; }
    let t833: [uint32x4_t; 4] = [vld1q_u32(t832l.as_ptr().add(0)), vld1q_u32(t832l.as_ptr().add(4)), vld1q_u32(t832h.as_ptr().add(0)), vld1q_u32(t832h.as_ptr().add(4))];
    res = t833;
    return res;
}

#[inline(always)]
unsafe fn b4_ll_f(l: &Launch, lid0: u32, x0: [uint32x4_t; 4], x1: [uint32x4_t; 4]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let (mut t834l, mut t834h) = ([0u32; 8], [0u32; 8]);
    { let v = (((((vgetq_lane_u32::<0>(x0[2]) as u64) << 32 | vgetq_lane_u32::<0>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<0>(x1[2]) as u64) << 32 | vgetq_lane_u32::<0>(x1[0]) as u64) as u64) as u128) >> 64) as u64; t834l[0] = v as u32; t834h[0] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<1>(x0[2]) as u64) << 32 | vgetq_lane_u32::<1>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<1>(x1[2]) as u64) << 32 | vgetq_lane_u32::<1>(x1[0]) as u64) as u64) as u128) >> 64) as u64; t834l[1] = v as u32; t834h[1] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<2>(x0[2]) as u64) << 32 | vgetq_lane_u32::<2>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<2>(x1[2]) as u64) << 32 | vgetq_lane_u32::<2>(x1[0]) as u64) as u64) as u128) >> 64) as u64; t834l[2] = v as u32; t834h[2] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<3>(x0[2]) as u64) << 32 | vgetq_lane_u32::<3>(x0[0]) as u64) as u64) as u128 * (((vgetq_lane_u32::<3>(x1[2]) as u64) << 32 | vgetq_lane_u32::<3>(x1[0]) as u64) as u64) as u128) >> 64) as u64; t834l[3] = v as u32; t834h[3] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<0>(x0[3]) as u64) << 32 | vgetq_lane_u32::<0>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<0>(x1[3]) as u64) << 32 | vgetq_lane_u32::<0>(x1[1]) as u64) as u64) as u128) >> 64) as u64; t834l[4] = v as u32; t834h[4] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<1>(x0[3]) as u64) << 32 | vgetq_lane_u32::<1>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<1>(x1[3]) as u64) << 32 | vgetq_lane_u32::<1>(x1[1]) as u64) as u64) as u128) >> 64) as u64; t834l[5] = v as u32; t834h[5] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<2>(x0[3]) as u64) << 32 | vgetq_lane_u32::<2>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<2>(x1[3]) as u64) << 32 | vgetq_lane_u32::<2>(x1[1]) as u64) as u64) as u128) >> 64) as u64; t834l[6] = v as u32; t834h[6] = (v >> 32) as u32; }
    { let v = (((((vgetq_lane_u32::<3>(x0[3]) as u64) << 32 | vgetq_lane_u32::<3>(x0[1]) as u64) as u64) as u128 * (((vgetq_lane_u32::<3>(x1[3]) as u64) << 32 | vgetq_lane_u32::<3>(x1[1]) as u64) as u64) as u128) >> 64) as u64; t834l[7] = v as u32; t834h[7] = (v >> 32) as u32; }
    let t835: [uint32x4_t; 4] = [vld1q_u32(t834l.as_ptr().add(0)), vld1q_u32(t834l.as_ptr().add(4)), vld1q_u32(t834h.as_ptr().add(0)), vld1q_u32(t834h.as_ptr().add(4))];
    res = t835;
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
        let t836: [uint32x4_t; 4];
        if (x1s) == 8u32 && ((x1f) as u64) + 7 * ((x1s) as u64) <= u32::MAX as u64 && all(&live) {
            let t837 = (arg0 as *const u8).add(((x1f) as usize) * 1);
            let t8380 = vld2q_u32((t837 as *const u32).add(0));
            let t8381 = vld2q_u32((t837 as *const u32).add(8));
            let t838: [uint32x4_t; 4] = [t8380.0, t8381.0, t8380.1, t8381.1];
            t836 = t838;
        } else {
            let mut t839: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            if ((x1f) as u64) + 7 * ((x1s) as u64) <= u32::MAX as u64 && all(&live) {
                let t840 = (arg0 as *const u8).add(((x1f) as usize) * 1);
                let t841 = ((x1s) as usize) * 1;
                { let v = (t840.add(0 * t841) as *const u64).read_unaligned(); t839[0] = vsetq_lane_u32::<0>(v as u32, t839[0]); t839[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t839[2]); }
                { let v = (t840.add(1 * t841) as *const u64).read_unaligned(); t839[0] = vsetq_lane_u32::<1>(v as u32, t839[0]); t839[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t839[2]); }
                { let v = (t840.add(2 * t841) as *const u64).read_unaligned(); t839[0] = vsetq_lane_u32::<2>(v as u32, t839[0]); t839[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t839[2]); }
                { let v = (t840.add(3 * t841) as *const u64).read_unaligned(); t839[0] = vsetq_lane_u32::<3>(v as u32, t839[0]); t839[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t839[2]); }
                { let v = (t840.add(4 * t841) as *const u64).read_unaligned(); t839[1] = vsetq_lane_u32::<0>(v as u32, t839[1]); t839[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t839[3]); }
                { let v = (t840.add(5 * t841) as *const u64).read_unaligned(); t839[1] = vsetq_lane_u32::<1>(v as u32, t839[1]); t839[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t839[3]); }
                { let v = (t840.add(6 * t841) as *const u64).read_unaligned(); t839[1] = vsetq_lane_u32::<2>(v as u32, t839[1]); t839[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t839[3]); }
                { let v = (t840.add(7 * t841) as *const u64).read_unaligned(); t839[1] = vsetq_lane_u32::<3>(v as u32, t839[1]); t839[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t839[3]); }
            } else {
                let t842: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1s)), vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1s))];
                let t843 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t842[0]), vandq_u32(live[1], t842[1]))));
                let t844: [uint32x4_t; 2] = [vbslq_u32(live[0], t842[0], t843), vbslq_u32(live[1], t842[1], t843)];
                let mut t845: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
                let t846 = vminvq_u32(vminq_u32(t844[0], t844[1]));
                if t846 == vmaxvq_u32(vmaxq_u32(t844[0], t844[1])) {
                    let t847 = ((arg0 as *const u8).add(t846 as usize * 1) as *const u64).read_unaligned();
                    t845 = [vdupq_n_u32(t847 as u32), vdupq_n_u32(t847 as u32), vdupq_n_u32((t847 >> 32) as u32), vdupq_n_u32((t847 >> 32) as u32)];
                } else {
                    { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t844[0]) as usize * 1) as *const u64).read_unaligned(); t845[0] = vsetq_lane_u32::<0>(v as u32, t845[0]); t845[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t845[2]); }
                    { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t844[0]) as usize * 1) as *const u64).read_unaligned(); t845[0] = vsetq_lane_u32::<1>(v as u32, t845[0]); t845[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t845[2]); }
                    { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t844[0]) as usize * 1) as *const u64).read_unaligned(); t845[0] = vsetq_lane_u32::<2>(v as u32, t845[0]); t845[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t845[2]); }
                    { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t844[0]) as usize * 1) as *const u64).read_unaligned(); t845[0] = vsetq_lane_u32::<3>(v as u32, t845[0]); t845[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t845[2]); }
                    { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t844[1]) as usize * 1) as *const u64).read_unaligned(); t845[1] = vsetq_lane_u32::<0>(v as u32, t845[1]); t845[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t845[3]); }
                    { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t844[1]) as usize * 1) as *const u64).read_unaligned(); t845[1] = vsetq_lane_u32::<1>(v as u32, t845[1]); t845[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t845[3]); }
                    { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t844[1]) as usize * 1) as *const u64).read_unaligned(); t845[1] = vsetq_lane_u32::<2>(v as u32, t845[1]); t845[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t845[3]); }
                    { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t844[1]) as usize * 1) as *const u64).read_unaligned(); t845[1] = vsetq_lane_u32::<3>(v as u32, t845[1]); t845[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t845[3]); }
                }
                t839 = t845;
            }
            t836 = t839;
        }
        res = t836;
        return res;
    }
    if ((x2) == (4u32)) {
        let t848: [uint32x4_t; 2];
        if (x1s) == 4u32 && ((x1f) as u64) + 7 * ((x1s) as u64) <= u32::MAX as u64 && all(&live) {
            let t849 = (arg0 as *const u8).add(((x1f) as usize) * 1);
            let t850: [uint32x4_t; 2] = core::array::from_fn(|r| vld1q_u32((t849 as *const u32).add(4 * r)));
            t848 = t850;
        } else {
            let mut t851: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            if ((x1f) as u64) + 7 * ((x1s) as u64) <= u32::MAX as u64 && all(&live) {
                let t852 = (arg0 as *const u8).add(((x1f) as usize) * 1);
                let t853 = ((x1s) as usize) * 1;
                t851[0] = vld1q_lane_u32::<0>(t852.add(0 * t853) as *const u32, t851[0]);
                t851[0] = vld1q_lane_u32::<1>(t852.add(1 * t853) as *const u32, t851[0]);
                t851[0] = vld1q_lane_u32::<2>(t852.add(2 * t853) as *const u32, t851[0]);
                t851[0] = vld1q_lane_u32::<3>(t852.add(3 * t853) as *const u32, t851[0]);
                t851[1] = vld1q_lane_u32::<0>(t852.add(4 * t853) as *const u32, t851[1]);
                t851[1] = vld1q_lane_u32::<1>(t852.add(5 * t853) as *const u32, t851[1]);
                t851[1] = vld1q_lane_u32::<2>(t852.add(6 * t853) as *const u32, t851[1]);
                t851[1] = vld1q_lane_u32::<3>(t852.add(7 * t853) as *const u32, t851[1]);
            } else {
                let t854: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1s)), vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1s))];
                let t855 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t854[0]), vandq_u32(live[1], t854[1]))));
                let t856: [uint32x4_t; 2] = [vbslq_u32(live[0], t854[0], t855), vbslq_u32(live[1], t854[1], t855)];
                let mut t857: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                let t858 = vminvq_u32(vminq_u32(t856[0], t856[1]));
                if t858 == vmaxvq_u32(vmaxq_u32(t856[0], t856[1])) {
                    t857 = [vdupq_n_u32((((arg0 as *const u8).add(t858 as usize * 1) as *const u32).read_unaligned() as u32)); 2];
                } else {
                    t857[0] = vsetq_lane_u32::<0>((((arg0 as *const u8).add(vgetq_lane_u32::<0>(t856[0]) as usize * 1) as *const u32).read_unaligned() as u32), t857[0]);
                    t857[0] = vsetq_lane_u32::<1>((((arg0 as *const u8).add(vgetq_lane_u32::<1>(t856[0]) as usize * 1) as *const u32).read_unaligned() as u32), t857[0]);
                    t857[0] = vsetq_lane_u32::<2>((((arg0 as *const u8).add(vgetq_lane_u32::<2>(t856[0]) as usize * 1) as *const u32).read_unaligned() as u32), t857[0]);
                    t857[0] = vsetq_lane_u32::<3>((((arg0 as *const u8).add(vgetq_lane_u32::<3>(t856[0]) as usize * 1) as *const u32).read_unaligned() as u32), t857[0]);
                    t857[1] = vsetq_lane_u32::<0>((((arg0 as *const u8).add(vgetq_lane_u32::<0>(t856[1]) as usize * 1) as *const u32).read_unaligned() as u32), t857[1]);
                    t857[1] = vsetq_lane_u32::<1>((((arg0 as *const u8).add(vgetq_lane_u32::<1>(t856[1]) as usize * 1) as *const u32).read_unaligned() as u32), t857[1]);
                    t857[1] = vsetq_lane_u32::<2>((((arg0 as *const u8).add(vgetq_lane_u32::<2>(t856[1]) as usize * 1) as *const u32).read_unaligned() as u32), t857[1]);
                    t857[1] = vsetq_lane_u32::<3>((((arg0 as *const u8).add(vgetq_lane_u32::<3>(t856[1]) as usize * 1) as *const u32).read_unaligned() as u32), t857[1]);
                }
                t851 = t857;
            }
            t848 = t851;
        }
        let t859: [uint32x4_t; 4] = [t848[0], t848[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t859;
        return res;
    }
    if ((x2) == (2u32)) {
        let t860: [uint32x4_t; 2];
        if (x1s) == 2u32 && ((x1f) as u64) + 7 * ((x1s) as u64) <= u32::MAX as u64 && all(&live) {
            let t861 = (arg0 as *const u8).add(((x1f) as usize) * 1);
            let t862: [uint32x4_t; 2] = core::array::from_fn(|r| vmovl_u16(vld1_u16((t861 as *const u16).add(4 * r))));
            t860 = t862;
        } else {
            let mut t863: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            if ((x1f) as u64) + 7 * ((x1s) as u64) <= u32::MAX as u64 && all(&live) {
                let t864 = (arg0 as *const u8).add(((x1f) as usize) * 1);
                let t865 = ((x1s) as usize) * 1;
                t863[0] = vsetq_lane_u32::<0>((t864.add(0 * t865) as *const u16).read_unaligned() as u32, t863[0]);
                t863[0] = vsetq_lane_u32::<1>((t864.add(1 * t865) as *const u16).read_unaligned() as u32, t863[0]);
                t863[0] = vsetq_lane_u32::<2>((t864.add(2 * t865) as *const u16).read_unaligned() as u32, t863[0]);
                t863[0] = vsetq_lane_u32::<3>((t864.add(3 * t865) as *const u16).read_unaligned() as u32, t863[0]);
                t863[1] = vsetq_lane_u32::<0>((t864.add(4 * t865) as *const u16).read_unaligned() as u32, t863[1]);
                t863[1] = vsetq_lane_u32::<1>((t864.add(5 * t865) as *const u16).read_unaligned() as u32, t863[1]);
                t863[1] = vsetq_lane_u32::<2>((t864.add(6 * t865) as *const u16).read_unaligned() as u32, t863[1]);
                t863[1] = vsetq_lane_u32::<3>((t864.add(7 * t865) as *const u16).read_unaligned() as u32, t863[1]);
            } else {
                let t866: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1s)), vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1s))];
                let t867 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t866[0]), vandq_u32(live[1], t866[1]))));
                let t868: [uint32x4_t; 2] = [vbslq_u32(live[0], t866[0], t867), vbslq_u32(live[1], t866[1], t867)];
                let mut t869: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                let t870 = vminvq_u32(vminq_u32(t868[0], t868[1]));
                if t870 == vmaxvq_u32(vmaxq_u32(t868[0], t868[1])) {
                    t869 = [vdupq_n_u32((((arg0 as *const u8).add(t870 as usize * 1) as *const u16).read_unaligned() as u32)); 2];
                } else {
                    t869[0] = vsetq_lane_u32::<0>((((arg0 as *const u8).add(vgetq_lane_u32::<0>(t868[0]) as usize * 1) as *const u16).read_unaligned() as u32), t869[0]);
                    t869[0] = vsetq_lane_u32::<1>((((arg0 as *const u8).add(vgetq_lane_u32::<1>(t868[0]) as usize * 1) as *const u16).read_unaligned() as u32), t869[0]);
                    t869[0] = vsetq_lane_u32::<2>((((arg0 as *const u8).add(vgetq_lane_u32::<2>(t868[0]) as usize * 1) as *const u16).read_unaligned() as u32), t869[0]);
                    t869[0] = vsetq_lane_u32::<3>((((arg0 as *const u8).add(vgetq_lane_u32::<3>(t868[0]) as usize * 1) as *const u16).read_unaligned() as u32), t869[0]);
                    t869[1] = vsetq_lane_u32::<0>((((arg0 as *const u8).add(vgetq_lane_u32::<0>(t868[1]) as usize * 1) as *const u16).read_unaligned() as u32), t869[1]);
                    t869[1] = vsetq_lane_u32::<1>((((arg0 as *const u8).add(vgetq_lane_u32::<1>(t868[1]) as usize * 1) as *const u16).read_unaligned() as u32), t869[1]);
                    t869[1] = vsetq_lane_u32::<2>((((arg0 as *const u8).add(vgetq_lane_u32::<2>(t868[1]) as usize * 1) as *const u16).read_unaligned() as u32), t869[1]);
                    t869[1] = vsetq_lane_u32::<3>((((arg0 as *const u8).add(vgetq_lane_u32::<3>(t868[1]) as usize * 1) as *const u16).read_unaligned() as u32), t869[1]);
                }
                t863 = t869;
            }
            t860 = t863;
        }
        let t871: [uint32x4_t; 4] = [t860[0], t860[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t871;
        return res;
    }
    if ((x2) == (1u32)) {
        let t872: [uint32x4_t; 2];
        if (x1s) == 1u32 && ((x1f) as u64) + 7 * ((x1s) as u64) <= u32::MAX as u64 && all(&live) {
            let t873 = (arg0 as *const u8).add(((x1f) as usize) * 1);
            let t875 = vmovl_u8(vld1_u8(t873.add(0)));
            let t874: [uint32x4_t; 2] = [vmovl_u16(vget_low_u16(t875)), vmovl_u16(vget_high_u16(t875))];
            t872 = t874;
        } else {
            let mut t876: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            if ((x1f) as u64) + 7 * ((x1s) as u64) <= u32::MAX as u64 && all(&live) {
                let t877 = (arg0 as *const u8).add(((x1f) as usize) * 1);
                let t878 = ((x1s) as usize) * 1;
                t876[0] = vsetq_lane_u32::<0>((t877.add(0 * t878) as *const u8).read_unaligned() as u32, t876[0]);
                t876[0] = vsetq_lane_u32::<1>((t877.add(1 * t878) as *const u8).read_unaligned() as u32, t876[0]);
                t876[0] = vsetq_lane_u32::<2>((t877.add(2 * t878) as *const u8).read_unaligned() as u32, t876[0]);
                t876[0] = vsetq_lane_u32::<3>((t877.add(3 * t878) as *const u8).read_unaligned() as u32, t876[0]);
                t876[1] = vsetq_lane_u32::<0>((t877.add(4 * t878) as *const u8).read_unaligned() as u32, t876[1]);
                t876[1] = vsetq_lane_u32::<1>((t877.add(5 * t878) as *const u8).read_unaligned() as u32, t876[1]);
                t876[1] = vsetq_lane_u32::<2>((t877.add(6 * t878) as *const u8).read_unaligned() as u32, t876[1]);
                t876[1] = vsetq_lane_u32::<3>((t877.add(7 * t878) as *const u8).read_unaligned() as u32, t876[1]);
            } else {
                let t879: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1s)), vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1s))];
                let t880 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t879[0]), vandq_u32(live[1], t879[1]))));
                let t881: [uint32x4_t; 2] = [vbslq_u32(live[0], t879[0], t880), vbslq_u32(live[1], t879[1], t880)];
                let mut t882: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                let t883 = vminvq_u32(vminq_u32(t881[0], t881[1]));
                if t883 == vmaxvq_u32(vmaxq_u32(t881[0], t881[1])) {
                    t882 = [vdupq_n_u32((((arg0 as *const u8).add(t883 as usize * 1) as *const u8).read_unaligned() as u32)); 2];
                } else {
                    t882[0] = vsetq_lane_u32::<0>((((arg0 as *const u8).add(vgetq_lane_u32::<0>(t881[0]) as usize * 1) as *const u8).read_unaligned() as u32), t882[0]);
                    t882[0] = vsetq_lane_u32::<1>((((arg0 as *const u8).add(vgetq_lane_u32::<1>(t881[0]) as usize * 1) as *const u8).read_unaligned() as u32), t882[0]);
                    t882[0] = vsetq_lane_u32::<2>((((arg0 as *const u8).add(vgetq_lane_u32::<2>(t881[0]) as usize * 1) as *const u8).read_unaligned() as u32), t882[0]);
                    t882[0] = vsetq_lane_u32::<3>((((arg0 as *const u8).add(vgetq_lane_u32::<3>(t881[0]) as usize * 1) as *const u8).read_unaligned() as u32), t882[0]);
                    t882[1] = vsetq_lane_u32::<0>((((arg0 as *const u8).add(vgetq_lane_u32::<0>(t881[1]) as usize * 1) as *const u8).read_unaligned() as u32), t882[1]);
                    t882[1] = vsetq_lane_u32::<1>((((arg0 as *const u8).add(vgetq_lane_u32::<1>(t881[1]) as usize * 1) as *const u8).read_unaligned() as u32), t882[1]);
                    t882[1] = vsetq_lane_u32::<2>((((arg0 as *const u8).add(vgetq_lane_u32::<2>(t881[1]) as usize * 1) as *const u8).read_unaligned() as u32), t882[1]);
                    t882[1] = vsetq_lane_u32::<3>((((arg0 as *const u8).add(vgetq_lane_u32::<3>(t881[1]) as usize * 1) as *const u8).read_unaligned() as u32), t882[1]);
                }
                t876 = t882;
            }
            t872 = t876;
        }
        let t884: [uint32x4_t; 4] = [t872[0], t872[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t884;
        return res;
    }
    let t885: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1s)), vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1s))];
    let t886 = [vdupq_n_u32(!(7u32)); 2];
    let t887: [uint32x4_t; 2] = [vandq_u32(t885[0], t886[0]), vandq_u32(t885[1], t886[1])];
    let t888: [uint32x4_t; 2] = [t887[0], t887[1]];
    let t889: u64 = ((arg0) as usize as u64);
    let t890 = [vdupq_n_u32((t889) as u32); 2];
    let t891 = [vdupq_n_u32(((t889) >> 32) as u32); 2];
    let t892: [uint32x4_t; 2] = [vaddq_u32(t890[0], t888[0]), vaddq_u32(t890[1], t888[1])];
    let t893: [uint32x4_t; 2] = [vcltq_u32(t892[0], t890[0]), vcltq_u32(t892[1], t890[1])];
    let t894: [uint32x4_t; 4] = [t892[0], t892[1], vsubq_u32(vaddq_u32(t891[0], vdupq_n_u32(0)), t893[0]), vsubq_u32(vaddq_u32(t891[1], vdupq_n_u32(0)), t893[1])];
    let t895: [uint32x4_t; 4] = [t894[0], t894[1], t894[2], t894[3]];
    x3 = t895;
    let t896 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([x3[0], x3[1]]);
    let t897 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([x3[2], x3[3]]);
    let t899 = bits(&live);
    let t900 = t899.trailing_zeros() as usize % 8;
    let t898: [usize; 8] = [{ let j = if t899 >> 0 & 1 != 0 { 0 } else { t900 }; ((t897[j] as u64) << 32 | t896[j] as u64) as usize }, { let j = if t899 >> 1 & 1 != 0 { 1 } else { t900 }; ((t897[j] as u64) << 32 | t896[j] as u64) as usize }, { let j = if t899 >> 2 & 1 != 0 { 2 } else { t900 }; ((t897[j] as u64) << 32 | t896[j] as u64) as usize }, { let j = if t899 >> 3 & 1 != 0 { 3 } else { t900 }; ((t897[j] as u64) << 32 | t896[j] as u64) as usize }, { let j = if t899 >> 4 & 1 != 0 { 4 } else { t900 }; ((t897[j] as u64) << 32 | t896[j] as u64) as usize }, { let j = if t899 >> 5 & 1 != 0 { 5 } else { t900 }; ((t897[j] as u64) << 32 | t896[j] as u64) as usize }, { let j = if t899 >> 6 & 1 != 0 { 6 } else { t900 }; ((t897[j] as u64) << 32 | t896[j] as u64) as usize }, { let j = if t899 >> 7 & 1 != 0 { 7 } else { t900 }; ((t897[j] as u64) << 32 | t896[j] as u64) as usize }];
    let t901: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1s)), vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1s))];
    let t902 = [vdupq_n_u32(7u32); 2];
    let t903: [uint32x4_t; 2] = [vandq_u32(t901[0], t902[0]), vandq_u32(t901[1], t902[1])];
    let t904 = [vdupq_n_u32(8u32); 2];
    let t905: [uint32x4_t; 2] = [vmulq_u32(t903[0], t904[0]), vmulq_u32(t903[1], t904[1])];
    x4 = t905;
    let mut t906: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (t898[0] as *const u64).read_unaligned(); t906[0] = vsetq_lane_u32::<0>(v as u32, t906[0]); t906[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t906[2]); }
    { let v = (t898[1] as *const u64).read_unaligned(); t906[0] = vsetq_lane_u32::<1>(v as u32, t906[0]); t906[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t906[2]); }
    { let v = (t898[2] as *const u64).read_unaligned(); t906[0] = vsetq_lane_u32::<2>(v as u32, t906[0]); t906[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t906[2]); }
    { let v = (t898[3] as *const u64).read_unaligned(); t906[0] = vsetq_lane_u32::<3>(v as u32, t906[0]); t906[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t906[2]); }
    { let v = (t898[4] as *const u64).read_unaligned(); t906[1] = vsetq_lane_u32::<0>(v as u32, t906[1]); t906[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t906[3]); }
    { let v = (t898[5] as *const u64).read_unaligned(); t906[1] = vsetq_lane_u32::<1>(v as u32, t906[1]); t906[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t906[3]); }
    { let v = (t898[6] as *const u64).read_unaligned(); t906[1] = vsetq_lane_u32::<2>(v as u32, t906[1]); t906[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t906[3]); }
    { let v = (t898[7] as *const u64).read_unaligned(); t906[1] = vsetq_lane_u32::<3>(v as u32, t906[1]); t906[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t906[3]); }
    let t907: [uint32x4_t; 2] = [x4[0], x4[1]];
    let t908: [uint32x4_t; 2] = [vandq_u32(t907[0], vdupq_n_u32(63)), vandq_u32(t907[1], vdupq_n_u32(63))];
    let t909: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t906[0], vnegq_s32(vreinterpretq_s32_u32(t908[0]))), vshlq_u32(t906[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t908[0])))), vorrq_u32(vshlq_u32(t906[1], vnegq_s32(vreinterpretq_s32_u32(t908[1]))), vshlq_u32(t906[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t908[1])))), vshlq_u32(t906[2], vnegq_s32(vreinterpretq_s32_u32(t908[0]))), vshlq_u32(t906[3], vnegq_s32(vreinterpretq_s32_u32(t908[1])))];
    x5 = t909;
    let t910 = [vdupq_n_u32(0u32); 2];
    let t911: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t910[0])), vmvnq_u32(vceqq_u32(x4[1], t910[1]))];
    let t912 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
    let t913: [uint32x4_t; 2] = [vaddq_u32(x4[0], t912[0]), vaddq_u32(x4[1], t912[1])];
    let t914 = [vdupq_n_u32(64u32); 2];
    let t915: [uint32x4_t; 2] = [vcgtq_u32(t913[0], t914[0]), vcgtq_u32(t913[1], t914[1])];
    let t916: [uint32x4_t; 2] = [vandq_u32(t911[0], t915[0]), vandq_u32(t911[1], t915[1])];
    let mut t917: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t918: [uint32x4_t; 2] = [vandq_u32(live[0], t916[0]), vandq_u32(live[1], t916[1])];
    let t919: [uint32x4_t; 2] = [vbicq_u32(live[0], t916[0]), vbicq_u32(live[1], t916[1])];
    {
        let t920: u64 = (1u64).wrapping_mul(8u64);
        let t921 = [vdupq_n_u32((t920) as u32); 2];
        let t922 = [vdupq_n_u32(((t920) >> 32) as u32); 2];
        let t923: [uint32x4_t; 2] = [vaddq_u32(x3[0], t921[0]), vaddq_u32(x3[1], t921[1])];
        let t924: [uint32x4_t; 2] = [vcltq_u32(t923[0], x3[0]), vcltq_u32(t923[1], x3[1])];
        let t925: [uint32x4_t; 4] = [t923[0], t923[1], vsubq_u32(vaddq_u32(x3[2], t922[0]), t924[0]), vsubq_u32(vaddq_u32(x3[3], t922[1]), t924[1])];
        let t926: [usize; 8] = core::array::from_fn(|j| t898[j].wrapping_add(((1u64).wrapping_mul(8u64)) as u64 as usize));
        let mut t927: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (t926[0] as *const u64).read_unaligned(); t927[0] = vsetq_lane_u32::<0>(v as u32, t927[0]); t927[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t927[2]); }
        { let v = (t926[1] as *const u64).read_unaligned(); t927[0] = vsetq_lane_u32::<1>(v as u32, t927[0]); t927[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t927[2]); }
        { let v = (t926[2] as *const u64).read_unaligned(); t927[0] = vsetq_lane_u32::<2>(v as u32, t927[0]); t927[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t927[2]); }
        { let v = (t926[3] as *const u64).read_unaligned(); t927[0] = vsetq_lane_u32::<3>(v as u32, t927[0]); t927[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t927[2]); }
        { let v = (t926[4] as *const u64).read_unaligned(); t927[1] = vsetq_lane_u32::<0>(v as u32, t927[1]); t927[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t927[3]); }
        { let v = (t926[5] as *const u64).read_unaligned(); t927[1] = vsetq_lane_u32::<1>(v as u32, t927[1]); t927[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t927[3]); }
        { let v = (t926[6] as *const u64).read_unaligned(); t927[1] = vsetq_lane_u32::<2>(v as u32, t927[1]); t927[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t927[3]); }
        { let v = (t926[7] as *const u64).read_unaligned(); t927[1] = vsetq_lane_u32::<3>(v as u32, t927[1]); t927[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t927[3]); }
        let t928: [uint32x4_t; 2] = [vsubq_u32(t914[0], x4[0]), vsubq_u32(t914[1], x4[1])];
        let t929: [uint32x4_t; 2] = [t928[0], t928[1]];
        let t930: [uint32x4_t; 2] = [vandq_u32(t929[0], vdupq_n_u32(63)), vandq_u32(t929[1], vdupq_n_u32(63))];
        let t931: [uint32x4_t; 4] = [vshlq_u32(t927[0], vreinterpretq_s32_u32(t930[0])), vshlq_u32(t927[1], vreinterpretq_s32_u32(t930[1])), vorrq_u32(vshlq_u32(t927[2], vreinterpretq_s32_u32(t930[0])), vshlq_u32(t927[0], vsubq_s32(vreinterpretq_s32_u32(t930[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t927[3], vreinterpretq_s32_u32(t930[1])), vshlq_u32(t927[1], vsubq_s32(vreinterpretq_s32_u32(t930[1]), vdupq_n_s32(32))))];
        let t932: [uint32x4_t; 4] = [vorrq_u32(x5[0], t931[0]), vorrq_u32(x5[1], t931[1]), vorrq_u32(x5[2], t931[2]), vorrq_u32(x5[3], t931[3])];
        t917 = [vbslq_u32(t918[0], t932[0], t917[0]), vbslq_u32(t918[1], t932[1], t917[1]), vbslq_u32(t918[0], t932[2], t917[2]), vbslq_u32(t918[1], t932[3], t917[3])];
    }
    {
        t917 = [vbslq_u32(t919[0], x5[0], t917[0]), vbslq_u32(t919[1], x5[1], t917[1]), vbslq_u32(t919[0], x5[2], t917[2]), vbslq_u32(t919[1], x5[3], t917[3])];
    }
    res = t917;
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
        let t933: [uint32x4_t; 4];
        if (x1s) == 8u32 && ((x1f) as u64) + 7 * ((x1s) as u64) <= u32::MAX as u64 {
            let t934 = (arg0 as *const u8).add(((x1f) as usize) * 1);
            let t9350 = vld2q_u32((t934 as *const u32).add(0));
            let t9351 = vld2q_u32((t934 as *const u32).add(8));
            let t935: [uint32x4_t; 4] = [t9350.0, t9351.0, t9350.1, t9351.1];
            t933 = t935;
        } else {
            let mut t936: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            if ((x1f) as u64) + 7 * ((x1s) as u64) <= u32::MAX as u64 {
                let t937 = (arg0 as *const u8).add(((x1f) as usize) * 1);
                let t938 = ((x1s) as usize) * 1;
                { let v = (t937.add(0 * t938) as *const u64).read_unaligned(); t936[0] = vsetq_lane_u32::<0>(v as u32, t936[0]); t936[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t936[2]); }
                { let v = (t937.add(1 * t938) as *const u64).read_unaligned(); t936[0] = vsetq_lane_u32::<1>(v as u32, t936[0]); t936[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t936[2]); }
                { let v = (t937.add(2 * t938) as *const u64).read_unaligned(); t936[0] = vsetq_lane_u32::<2>(v as u32, t936[0]); t936[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t936[2]); }
                { let v = (t937.add(3 * t938) as *const u64).read_unaligned(); t936[0] = vsetq_lane_u32::<3>(v as u32, t936[0]); t936[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t936[2]); }
                { let v = (t937.add(4 * t938) as *const u64).read_unaligned(); t936[1] = vsetq_lane_u32::<0>(v as u32, t936[1]); t936[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t936[3]); }
                { let v = (t937.add(5 * t938) as *const u64).read_unaligned(); t936[1] = vsetq_lane_u32::<1>(v as u32, t936[1]); t936[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t936[3]); }
                { let v = (t937.add(6 * t938) as *const u64).read_unaligned(); t936[1] = vsetq_lane_u32::<2>(v as u32, t936[1]); t936[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t936[3]); }
                { let v = (t937.add(7 * t938) as *const u64).read_unaligned(); t936[1] = vsetq_lane_u32::<3>(v as u32, t936[1]); t936[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t936[3]); }
            } else {
                let t939: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1s)), vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1s))];
                let mut t940: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
                let t941 = vminvq_u32(vminq_u32(t939[0], t939[1]));
                if t941 == vmaxvq_u32(vmaxq_u32(t939[0], t939[1])) {
                    let t942 = ((arg0 as *const u8).add(t941 as usize * 1) as *const u64).read_unaligned();
                    t940 = [vdupq_n_u32(t942 as u32), vdupq_n_u32(t942 as u32), vdupq_n_u32((t942 >> 32) as u32), vdupq_n_u32((t942 >> 32) as u32)];
                } else {
                    { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t939[0]) as usize * 1) as *const u64).read_unaligned(); t940[0] = vsetq_lane_u32::<0>(v as u32, t940[0]); t940[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t940[2]); }
                    { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t939[0]) as usize * 1) as *const u64).read_unaligned(); t940[0] = vsetq_lane_u32::<1>(v as u32, t940[0]); t940[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t940[2]); }
                    { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t939[0]) as usize * 1) as *const u64).read_unaligned(); t940[0] = vsetq_lane_u32::<2>(v as u32, t940[0]); t940[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t940[2]); }
                    { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t939[0]) as usize * 1) as *const u64).read_unaligned(); t940[0] = vsetq_lane_u32::<3>(v as u32, t940[0]); t940[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t940[2]); }
                    { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<0>(t939[1]) as usize * 1) as *const u64).read_unaligned(); t940[1] = vsetq_lane_u32::<0>(v as u32, t940[1]); t940[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t940[3]); }
                    { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<1>(t939[1]) as usize * 1) as *const u64).read_unaligned(); t940[1] = vsetq_lane_u32::<1>(v as u32, t940[1]); t940[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t940[3]); }
                    { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<2>(t939[1]) as usize * 1) as *const u64).read_unaligned(); t940[1] = vsetq_lane_u32::<2>(v as u32, t940[1]); t940[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t940[3]); }
                    { let v = ((arg0 as *const u8).add(vgetq_lane_u32::<3>(t939[1]) as usize * 1) as *const u64).read_unaligned(); t940[1] = vsetq_lane_u32::<3>(v as u32, t940[1]); t940[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t940[3]); }
                }
                t936 = t940;
            }
            t933 = t936;
        }
        res = t933;
        return res;
    }
    if ((x2) == (4u32)) {
        let t943: [uint32x4_t; 2];
        if (x1s) == 4u32 && ((x1f) as u64) + 7 * ((x1s) as u64) <= u32::MAX as u64 {
            let t944 = (arg0 as *const u8).add(((x1f) as usize) * 1);
            let t945: [uint32x4_t; 2] = core::array::from_fn(|r| vld1q_u32((t944 as *const u32).add(4 * r)));
            t943 = t945;
        } else {
            let mut t946: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            if ((x1f) as u64) + 7 * ((x1s) as u64) <= u32::MAX as u64 {
                let t947 = (arg0 as *const u8).add(((x1f) as usize) * 1);
                let t948 = ((x1s) as usize) * 1;
                t946[0] = vld1q_lane_u32::<0>(t947.add(0 * t948) as *const u32, t946[0]);
                t946[0] = vld1q_lane_u32::<1>(t947.add(1 * t948) as *const u32, t946[0]);
                t946[0] = vld1q_lane_u32::<2>(t947.add(2 * t948) as *const u32, t946[0]);
                t946[0] = vld1q_lane_u32::<3>(t947.add(3 * t948) as *const u32, t946[0]);
                t946[1] = vld1q_lane_u32::<0>(t947.add(4 * t948) as *const u32, t946[1]);
                t946[1] = vld1q_lane_u32::<1>(t947.add(5 * t948) as *const u32, t946[1]);
                t946[1] = vld1q_lane_u32::<2>(t947.add(6 * t948) as *const u32, t946[1]);
                t946[1] = vld1q_lane_u32::<3>(t947.add(7 * t948) as *const u32, t946[1]);
            } else {
                let t949: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1s)), vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1s))];
                let mut t950: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                let t951 = vminvq_u32(vminq_u32(t949[0], t949[1]));
                if t951 == vmaxvq_u32(vmaxq_u32(t949[0], t949[1])) {
                    t950 = [vdupq_n_u32((((arg0 as *const u8).add(t951 as usize * 1) as *const u32).read_unaligned() as u32)); 2];
                } else {
                    t950[0] = vsetq_lane_u32::<0>((((arg0 as *const u8).add(vgetq_lane_u32::<0>(t949[0]) as usize * 1) as *const u32).read_unaligned() as u32), t950[0]);
                    t950[0] = vsetq_lane_u32::<1>((((arg0 as *const u8).add(vgetq_lane_u32::<1>(t949[0]) as usize * 1) as *const u32).read_unaligned() as u32), t950[0]);
                    t950[0] = vsetq_lane_u32::<2>((((arg0 as *const u8).add(vgetq_lane_u32::<2>(t949[0]) as usize * 1) as *const u32).read_unaligned() as u32), t950[0]);
                    t950[0] = vsetq_lane_u32::<3>((((arg0 as *const u8).add(vgetq_lane_u32::<3>(t949[0]) as usize * 1) as *const u32).read_unaligned() as u32), t950[0]);
                    t950[1] = vsetq_lane_u32::<0>((((arg0 as *const u8).add(vgetq_lane_u32::<0>(t949[1]) as usize * 1) as *const u32).read_unaligned() as u32), t950[1]);
                    t950[1] = vsetq_lane_u32::<1>((((arg0 as *const u8).add(vgetq_lane_u32::<1>(t949[1]) as usize * 1) as *const u32).read_unaligned() as u32), t950[1]);
                    t950[1] = vsetq_lane_u32::<2>((((arg0 as *const u8).add(vgetq_lane_u32::<2>(t949[1]) as usize * 1) as *const u32).read_unaligned() as u32), t950[1]);
                    t950[1] = vsetq_lane_u32::<3>((((arg0 as *const u8).add(vgetq_lane_u32::<3>(t949[1]) as usize * 1) as *const u32).read_unaligned() as u32), t950[1]);
                }
                t946 = t950;
            }
            t943 = t946;
        }
        let t952: [uint32x4_t; 4] = [t943[0], t943[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t952;
        return res;
    }
    if ((x2) == (2u32)) {
        let t953: [uint32x4_t; 2];
        if (x1s) == 2u32 && ((x1f) as u64) + 7 * ((x1s) as u64) <= u32::MAX as u64 {
            let t954 = (arg0 as *const u8).add(((x1f) as usize) * 1);
            let t955: [uint32x4_t; 2] = core::array::from_fn(|r| vmovl_u16(vld1_u16((t954 as *const u16).add(4 * r))));
            t953 = t955;
        } else {
            let mut t956: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            if ((x1f) as u64) + 7 * ((x1s) as u64) <= u32::MAX as u64 {
                let t957 = (arg0 as *const u8).add(((x1f) as usize) * 1);
                let t958 = ((x1s) as usize) * 1;
                t956[0] = vsetq_lane_u32::<0>((t957.add(0 * t958) as *const u16).read_unaligned() as u32, t956[0]);
                t956[0] = vsetq_lane_u32::<1>((t957.add(1 * t958) as *const u16).read_unaligned() as u32, t956[0]);
                t956[0] = vsetq_lane_u32::<2>((t957.add(2 * t958) as *const u16).read_unaligned() as u32, t956[0]);
                t956[0] = vsetq_lane_u32::<3>((t957.add(3 * t958) as *const u16).read_unaligned() as u32, t956[0]);
                t956[1] = vsetq_lane_u32::<0>((t957.add(4 * t958) as *const u16).read_unaligned() as u32, t956[1]);
                t956[1] = vsetq_lane_u32::<1>((t957.add(5 * t958) as *const u16).read_unaligned() as u32, t956[1]);
                t956[1] = vsetq_lane_u32::<2>((t957.add(6 * t958) as *const u16).read_unaligned() as u32, t956[1]);
                t956[1] = vsetq_lane_u32::<3>((t957.add(7 * t958) as *const u16).read_unaligned() as u32, t956[1]);
            } else {
                let t959: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1s)), vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1s))];
                let mut t960: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                let t961 = vminvq_u32(vminq_u32(t959[0], t959[1]));
                if t961 == vmaxvq_u32(vmaxq_u32(t959[0], t959[1])) {
                    t960 = [vdupq_n_u32((((arg0 as *const u8).add(t961 as usize * 1) as *const u16).read_unaligned() as u32)); 2];
                } else {
                    t960[0] = vsetq_lane_u32::<0>((((arg0 as *const u8).add(vgetq_lane_u32::<0>(t959[0]) as usize * 1) as *const u16).read_unaligned() as u32), t960[0]);
                    t960[0] = vsetq_lane_u32::<1>((((arg0 as *const u8).add(vgetq_lane_u32::<1>(t959[0]) as usize * 1) as *const u16).read_unaligned() as u32), t960[0]);
                    t960[0] = vsetq_lane_u32::<2>((((arg0 as *const u8).add(vgetq_lane_u32::<2>(t959[0]) as usize * 1) as *const u16).read_unaligned() as u32), t960[0]);
                    t960[0] = vsetq_lane_u32::<3>((((arg0 as *const u8).add(vgetq_lane_u32::<3>(t959[0]) as usize * 1) as *const u16).read_unaligned() as u32), t960[0]);
                    t960[1] = vsetq_lane_u32::<0>((((arg0 as *const u8).add(vgetq_lane_u32::<0>(t959[1]) as usize * 1) as *const u16).read_unaligned() as u32), t960[1]);
                    t960[1] = vsetq_lane_u32::<1>((((arg0 as *const u8).add(vgetq_lane_u32::<1>(t959[1]) as usize * 1) as *const u16).read_unaligned() as u32), t960[1]);
                    t960[1] = vsetq_lane_u32::<2>((((arg0 as *const u8).add(vgetq_lane_u32::<2>(t959[1]) as usize * 1) as *const u16).read_unaligned() as u32), t960[1]);
                    t960[1] = vsetq_lane_u32::<3>((((arg0 as *const u8).add(vgetq_lane_u32::<3>(t959[1]) as usize * 1) as *const u16).read_unaligned() as u32), t960[1]);
                }
                t956 = t960;
            }
            t953 = t956;
        }
        let t962: [uint32x4_t; 4] = [t953[0], t953[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t962;
        return res;
    }
    if ((x2) == (1u32)) {
        let t963: [uint32x4_t; 2];
        if (x1s) == 1u32 && ((x1f) as u64) + 7 * ((x1s) as u64) <= u32::MAX as u64 {
            let t964 = (arg0 as *const u8).add(((x1f) as usize) * 1);
            let t966 = vmovl_u8(vld1_u8(t964.add(0)));
            let t965: [uint32x4_t; 2] = [vmovl_u16(vget_low_u16(t966)), vmovl_u16(vget_high_u16(t966))];
            t963 = t965;
        } else {
            let mut t967: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            if ((x1f) as u64) + 7 * ((x1s) as u64) <= u32::MAX as u64 {
                let t968 = (arg0 as *const u8).add(((x1f) as usize) * 1);
                let t969 = ((x1s) as usize) * 1;
                t967[0] = vsetq_lane_u32::<0>((t968.add(0 * t969) as *const u8).read_unaligned() as u32, t967[0]);
                t967[0] = vsetq_lane_u32::<1>((t968.add(1 * t969) as *const u8).read_unaligned() as u32, t967[0]);
                t967[0] = vsetq_lane_u32::<2>((t968.add(2 * t969) as *const u8).read_unaligned() as u32, t967[0]);
                t967[0] = vsetq_lane_u32::<3>((t968.add(3 * t969) as *const u8).read_unaligned() as u32, t967[0]);
                t967[1] = vsetq_lane_u32::<0>((t968.add(4 * t969) as *const u8).read_unaligned() as u32, t967[1]);
                t967[1] = vsetq_lane_u32::<1>((t968.add(5 * t969) as *const u8).read_unaligned() as u32, t967[1]);
                t967[1] = vsetq_lane_u32::<2>((t968.add(6 * t969) as *const u8).read_unaligned() as u32, t967[1]);
                t967[1] = vsetq_lane_u32::<3>((t968.add(7 * t969) as *const u8).read_unaligned() as u32, t967[1]);
            } else {
                let t970: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1s)), vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1s))];
                let mut t971: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                let t972 = vminvq_u32(vminq_u32(t970[0], t970[1]));
                if t972 == vmaxvq_u32(vmaxq_u32(t970[0], t970[1])) {
                    t971 = [vdupq_n_u32((((arg0 as *const u8).add(t972 as usize * 1) as *const u8).read_unaligned() as u32)); 2];
                } else {
                    t971[0] = vsetq_lane_u32::<0>((((arg0 as *const u8).add(vgetq_lane_u32::<0>(t970[0]) as usize * 1) as *const u8).read_unaligned() as u32), t971[0]);
                    t971[0] = vsetq_lane_u32::<1>((((arg0 as *const u8).add(vgetq_lane_u32::<1>(t970[0]) as usize * 1) as *const u8).read_unaligned() as u32), t971[0]);
                    t971[0] = vsetq_lane_u32::<2>((((arg0 as *const u8).add(vgetq_lane_u32::<2>(t970[0]) as usize * 1) as *const u8).read_unaligned() as u32), t971[0]);
                    t971[0] = vsetq_lane_u32::<3>((((arg0 as *const u8).add(vgetq_lane_u32::<3>(t970[0]) as usize * 1) as *const u8).read_unaligned() as u32), t971[0]);
                    t971[1] = vsetq_lane_u32::<0>((((arg0 as *const u8).add(vgetq_lane_u32::<0>(t970[1]) as usize * 1) as *const u8).read_unaligned() as u32), t971[1]);
                    t971[1] = vsetq_lane_u32::<1>((((arg0 as *const u8).add(vgetq_lane_u32::<1>(t970[1]) as usize * 1) as *const u8).read_unaligned() as u32), t971[1]);
                    t971[1] = vsetq_lane_u32::<2>((((arg0 as *const u8).add(vgetq_lane_u32::<2>(t970[1]) as usize * 1) as *const u8).read_unaligned() as u32), t971[1]);
                    t971[1] = vsetq_lane_u32::<3>((((arg0 as *const u8).add(vgetq_lane_u32::<3>(t970[1]) as usize * 1) as *const u8).read_unaligned() as u32), t971[1]);
                }
                t967 = t971;
            }
            t963 = t967;
        }
        let t973: [uint32x4_t; 4] = [t963[0], t963[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t973;
        return res;
    }
    let t974: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1s)), vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1s))];
    let t975 = [vdupq_n_u32(!(7u32)); 2];
    let t976: [uint32x4_t; 2] = [vandq_u32(t974[0], t975[0]), vandq_u32(t974[1], t975[1])];
    let t977: [uint32x4_t; 2] = [t976[0], t976[1]];
    let t978: u64 = ((arg0) as usize as u64);
    let t979 = [vdupq_n_u32((t978) as u32); 2];
    let t980 = [vdupq_n_u32(((t978) >> 32) as u32); 2];
    let t981: [uint32x4_t; 2] = [vaddq_u32(t979[0], t977[0]), vaddq_u32(t979[1], t977[1])];
    let t982: [uint32x4_t; 2] = [vcltq_u32(t981[0], t979[0]), vcltq_u32(t981[1], t979[1])];
    let t983: [uint32x4_t; 4] = [t981[0], t981[1], vsubq_u32(vaddq_u32(t980[0], vdupq_n_u32(0)), t982[0]), vsubq_u32(vaddq_u32(t980[1], vdupq_n_u32(0)), t982[1])];
    let t984: [uint32x4_t; 4] = [t983[0], t983[1], t983[2], t983[3]];
    x3 = t984;
    let t985 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([x3[0], x3[1]]);
    let t986 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([x3[2], x3[3]]);
    let t987: [usize; 8] = [((t986[0] as u64) << 32 | t985[0] as u64) as usize, ((t986[1] as u64) << 32 | t985[1] as u64) as usize, ((t986[2] as u64) << 32 | t985[2] as u64) as usize, ((t986[3] as u64) << 32 | t985[3] as u64) as usize, ((t986[4] as u64) << 32 | t985[4] as u64) as usize, ((t986[5] as u64) << 32 | t985[5] as u64) as usize, ((t986[6] as u64) << 32 | t985[6] as u64) as usize, ((t986[7] as u64) << 32 | t985[7] as u64) as usize];
    let t988: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1s)), vmlaq_u32(vdupq_n_u32(x1f), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1s))];
    let t989 = [vdupq_n_u32(7u32); 2];
    let t990: [uint32x4_t; 2] = [vandq_u32(t988[0], t989[0]), vandq_u32(t988[1], t989[1])];
    let t991 = [vdupq_n_u32(8u32); 2];
    let t992: [uint32x4_t; 2] = [vmulq_u32(t990[0], t991[0]), vmulq_u32(t990[1], t991[1])];
    x4 = t992;
    let mut t993: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (t987[0] as *const u64).read_unaligned(); t993[0] = vsetq_lane_u32::<0>(v as u32, t993[0]); t993[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t993[2]); }
    { let v = (t987[1] as *const u64).read_unaligned(); t993[0] = vsetq_lane_u32::<1>(v as u32, t993[0]); t993[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t993[2]); }
    { let v = (t987[2] as *const u64).read_unaligned(); t993[0] = vsetq_lane_u32::<2>(v as u32, t993[0]); t993[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t993[2]); }
    { let v = (t987[3] as *const u64).read_unaligned(); t993[0] = vsetq_lane_u32::<3>(v as u32, t993[0]); t993[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t993[2]); }
    { let v = (t987[4] as *const u64).read_unaligned(); t993[1] = vsetq_lane_u32::<0>(v as u32, t993[1]); t993[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t993[3]); }
    { let v = (t987[5] as *const u64).read_unaligned(); t993[1] = vsetq_lane_u32::<1>(v as u32, t993[1]); t993[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t993[3]); }
    { let v = (t987[6] as *const u64).read_unaligned(); t993[1] = vsetq_lane_u32::<2>(v as u32, t993[1]); t993[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t993[3]); }
    { let v = (t987[7] as *const u64).read_unaligned(); t993[1] = vsetq_lane_u32::<3>(v as u32, t993[1]); t993[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t993[3]); }
    let t994: [uint32x4_t; 2] = [x4[0], x4[1]];
    let t995: [uint32x4_t; 2] = [vandq_u32(t994[0], vdupq_n_u32(63)), vandq_u32(t994[1], vdupq_n_u32(63))];
    let t996: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t993[0], vnegq_s32(vreinterpretq_s32_u32(t995[0]))), vshlq_u32(t993[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t995[0])))), vorrq_u32(vshlq_u32(t993[1], vnegq_s32(vreinterpretq_s32_u32(t995[1]))), vshlq_u32(t993[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t995[1])))), vshlq_u32(t993[2], vnegq_s32(vreinterpretq_s32_u32(t995[0]))), vshlq_u32(t993[3], vnegq_s32(vreinterpretq_s32_u32(t995[1])))];
    x5 = t996;
    let t997 = [vdupq_n_u32(0u32); 2];
    let t998: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t997[0])), vmvnq_u32(vceqq_u32(x4[1], t997[1]))];
    let t999 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
    let t1000: [uint32x4_t; 2] = [vaddq_u32(x4[0], t999[0]), vaddq_u32(x4[1], t999[1])];
    let t1001 = [vdupq_n_u32(64u32); 2];
    let t1002: [uint32x4_t; 2] = [vcgtq_u32(t1000[0], t1001[0]), vcgtq_u32(t1000[1], t1001[1])];
    let t1003: [uint32x4_t; 2] = [vandq_u32(t998[0], t1002[0]), vandq_u32(t998[1], t1002[1])];
    let mut t1004: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1005: [uint32x4_t; 2] = [vmvnq_u32(t1003[0]), vmvnq_u32(t1003[1])];
    {
        let t1006: u64 = (1u64).wrapping_mul(8u64);
        let t1007 = [vdupq_n_u32((t1006) as u32); 2];
        let t1008 = [vdupq_n_u32(((t1006) >> 32) as u32); 2];
        let t1009: [uint32x4_t; 2] = [vaddq_u32(x3[0], t1007[0]), vaddq_u32(x3[1], t1007[1])];
        let t1010: [uint32x4_t; 2] = [vcltq_u32(t1009[0], x3[0]), vcltq_u32(t1009[1], x3[1])];
        let t1011: [uint32x4_t; 4] = [t1009[0], t1009[1], vsubq_u32(vaddq_u32(x3[2], t1008[0]), t1010[0]), vsubq_u32(vaddq_u32(x3[3], t1008[1]), t1010[1])];
        let t1012: [usize; 8] = core::array::from_fn(|j| t987[j].wrapping_add(((1u64).wrapping_mul(8u64)) as u64 as usize));
        let mut t1013: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (t1012[0] as *const u64).read_unaligned(); t1013[0] = vsetq_lane_u32::<0>(v as u32, t1013[0]); t1013[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1013[2]); }
        { let v = (t1012[1] as *const u64).read_unaligned(); t1013[0] = vsetq_lane_u32::<1>(v as u32, t1013[0]); t1013[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1013[2]); }
        { let v = (t1012[2] as *const u64).read_unaligned(); t1013[0] = vsetq_lane_u32::<2>(v as u32, t1013[0]); t1013[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1013[2]); }
        { let v = (t1012[3] as *const u64).read_unaligned(); t1013[0] = vsetq_lane_u32::<3>(v as u32, t1013[0]); t1013[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1013[2]); }
        { let v = (t1012[4] as *const u64).read_unaligned(); t1013[1] = vsetq_lane_u32::<0>(v as u32, t1013[1]); t1013[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1013[3]); }
        { let v = (t1012[5] as *const u64).read_unaligned(); t1013[1] = vsetq_lane_u32::<1>(v as u32, t1013[1]); t1013[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1013[3]); }
        { let v = (t1012[6] as *const u64).read_unaligned(); t1013[1] = vsetq_lane_u32::<2>(v as u32, t1013[1]); t1013[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1013[3]); }
        { let v = (t1012[7] as *const u64).read_unaligned(); t1013[1] = vsetq_lane_u32::<3>(v as u32, t1013[1]); t1013[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1013[3]); }
        let t1014: [uint32x4_t; 2] = [vsubq_u32(t1001[0], x4[0]), vsubq_u32(t1001[1], x4[1])];
        let t1015: [uint32x4_t; 2] = [t1014[0], t1014[1]];
        let t1016: [uint32x4_t; 2] = [vandq_u32(t1015[0], vdupq_n_u32(63)), vandq_u32(t1015[1], vdupq_n_u32(63))];
        let t1017: [uint32x4_t; 4] = [vshlq_u32(t1013[0], vreinterpretq_s32_u32(t1016[0])), vshlq_u32(t1013[1], vreinterpretq_s32_u32(t1016[1])), vorrq_u32(vshlq_u32(t1013[2], vreinterpretq_s32_u32(t1016[0])), vshlq_u32(t1013[0], vsubq_s32(vreinterpretq_s32_u32(t1016[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1013[3], vreinterpretq_s32_u32(t1016[1])), vshlq_u32(t1013[1], vsubq_s32(vreinterpretq_s32_u32(t1016[1]), vdupq_n_s32(32))))];
        let t1018: [uint32x4_t; 4] = [vorrq_u32(x5[0], t1017[0]), vorrq_u32(x5[1], t1017[1]), vorrq_u32(x5[2], t1017[2]), vorrq_u32(x5[3], t1017[3])];
        t1004 = [vbslq_u32(t1003[0], t1018[0], t1004[0]), vbslq_u32(t1003[1], t1018[1], t1004[1]), vbslq_u32(t1003[0], t1018[2], t1004[2]), vbslq_u32(t1003[1], t1018[3], t1004[3])];
    }
    {
        t1004 = [vbslq_u32(t1005[0], x5[0], t1004[0]), vbslq_u32(t1005[1], x5[1], t1004[1]), vbslq_u32(t1005[0], x5[2], t1004[2]), vbslq_u32(t1005[1], x5[3], t1004[3])];
    }
    res = t1004;
    return res;
}

#[inline(always)]
unsafe fn b6_i_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1019: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x0), vld1q_u32(LANE[1].as_ptr()))];
    let t1020: [uint32x4_t; 2] = [vshlq_u32(t1019[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(t1019[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
    let t1021 = [vdupq_n_u32(255u32); 2];
    let t1022: [uint32x4_t; 2] = [vandq_u32(t1019[0], t1021[0]), vandq_u32(t1019[1], t1021[1])];
    let t1023: [uint32x4_t; 2] = [vmulq_u32(t1020[0], vdupq_n_u32(1000)), vmulq_u32(t1020[1], vdupq_n_u32(1000))];
    let t1024: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(t1020[0]), vget_low_u32(vdupq_n_u32(1000)))), vreinterpretq_u32_u64(vmull_high_u32(t1020[0], vdupq_n_u32(1000)))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(t1020[1]), vget_low_u32(vdupq_n_u32(1000)))), vreinterpretq_u32_u64(vmull_high_u32(t1020[1], vdupq_n_u32(1000))))];
    let t1025: [uint32x4_t; 2] = [vaddq_u32(t1023[0], t1022[0]), vaddq_u32(t1023[1], t1022[1])];
    let t1026: [uint32x4_t; 2] = [vcltq_u32(t1025[0], t1023[0]), vcltq_u32(t1025[1], t1023[1])];
    let t1027: [uint32x4_t; 4] = [t1025[0], t1025[1], vsubq_u32(vaddq_u32(t1024[0], vdupq_n_u32(0)), t1026[0]), vsubq_u32(vaddq_u32(t1024[1], vdupq_n_u32(0)), t1026[1])];
    res = t1027;
    return res;
}

#[inline(always)]
unsafe fn b6_i_f(l: &Launch, lid0: u32, x0: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1028: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x0), vld1q_u32(LANE[1].as_ptr()))];
    let t1029: [uint32x4_t; 2] = [vshlq_u32(t1028[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(t1028[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
    let t1030 = [vdupq_n_u32(255u32); 2];
    let t1031: [uint32x4_t; 2] = [vandq_u32(t1028[0], t1030[0]), vandq_u32(t1028[1], t1030[1])];
    let t1032: [uint32x4_t; 2] = [vmulq_u32(t1029[0], vdupq_n_u32(1000)), vmulq_u32(t1029[1], vdupq_n_u32(1000))];
    let t1033: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(t1029[0]), vget_low_u32(vdupq_n_u32(1000)))), vreinterpretq_u32_u64(vmull_high_u32(t1029[0], vdupq_n_u32(1000)))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(t1029[1]), vget_low_u32(vdupq_n_u32(1000)))), vreinterpretq_u32_u64(vmull_high_u32(t1029[1], vdupq_n_u32(1000))))];
    let t1034: [uint32x4_t; 2] = [vaddq_u32(t1032[0], t1031[0]), vaddq_u32(t1032[1], t1031[1])];
    let t1035: [uint32x4_t; 2] = [vcltq_u32(t1034[0], t1032[0]), vcltq_u32(t1034[1], t1032[1])];
    let t1036: [uint32x4_t; 4] = [t1034[0], t1034[1], vsubq_u32(vaddq_u32(t1033[0], vdupq_n_u32(0)), t1035[0]), vsubq_u32(vaddq_u32(t1033[1], vdupq_n_u32(0)), t1035[1])];
    res = t1036;
    return res;
}

#[inline(always)]
unsafe fn b6_u8_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1037: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32((x0 & 255u32)), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32((x0 & 255u32)), vld1q_u32(LANE[1].as_ptr()))];
    let t1038: [uint32x4_t; 2] = [t1037[0], t1037[1]];
    let t1039: u64 = ((((((((x0 >> 8)) as u64)).wrapping_mul(1000u64)) as usize as *const u8)) as usize as u64);
    let t1040 = [vdupq_n_u32((t1039) as u32); 2];
    let t1041 = [vdupq_n_u32(((t1039) >> 32) as u32); 2];
    let t1042: [uint32x4_t; 2] = [vaddq_u32(t1040[0], t1038[0]), vaddq_u32(t1040[1], t1038[1])];
    let t1043: [uint32x4_t; 2] = [vcltq_u32(t1042[0], t1040[0]), vcltq_u32(t1042[1], t1040[1])];
    let t1044: [uint32x4_t; 4] = [t1042[0], t1042[1], vsubq_u32(vaddq_u32(t1041[0], vdupq_n_u32(0)), t1043[0]), vsubq_u32(vaddq_u32(t1041[1], vdupq_n_u32(0)), t1043[1])];
    let t1045: [uint32x4_t; 4] = [t1044[0], t1044[1], t1044[2], t1044[3]];
    res = t1045;
    return res;
}

#[inline(always)]
unsafe fn b6_u8_f(l: &Launch, lid0: u32, x0: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1046: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32((x0 & 255u32)), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32((x0 & 255u32)), vld1q_u32(LANE[1].as_ptr()))];
    let t1047: [uint32x4_t; 2] = [t1046[0], t1046[1]];
    let t1048: u64 = ((((((((x0 >> 8)) as u64)).wrapping_mul(1000u64)) as usize as *const u8)) as usize as u64);
    let t1049 = [vdupq_n_u32((t1048) as u32); 2];
    let t1050 = [vdupq_n_u32(((t1048) >> 32) as u32); 2];
    let t1051: [uint32x4_t; 2] = [vaddq_u32(t1049[0], t1047[0]), vaddq_u32(t1049[1], t1047[1])];
    let t1052: [uint32x4_t; 2] = [vcltq_u32(t1051[0], t1049[0]), vcltq_u32(t1051[1], t1049[1])];
    let t1053: [uint32x4_t; 4] = [t1051[0], t1051[1], vsubq_u32(vaddq_u32(t1050[0], vdupq_n_u32(0)), t1052[0]), vsubq_u32(vaddq_u32(t1050[1], vdupq_n_u32(0)), t1052[1])];
    let t1054: [uint32x4_t; 4] = [t1053[0], t1053[1], t1053[2], t1053[3]];
    res = t1054;
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
    let mut t1059 = live;
    loop {
        let t1061 = bits(&t1059);
        if t1061.count_ones() <= 2 {
            let mut t1062 = t1061;
            while t1062 != 0 {
                let j = t1062.trailing_zeros() as usize;
                t1062 &= t1062 - 1;
                let t1063 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x1);
                let mut x1_j: u32 = t1063[j];
                let t1064 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x2);
                let mut x2_j: u64 = ((t1064[8 + j] as u64) << 32 | t1064[j] as u64);
                let mut x3_j: u32 = x3;
                loop {
                    if !(((x3_j) < (((x1_j) & (15u32))))) { break; }
                    x2_j = ((x2_j).wrapping_mul(31u64)).wrapping_add(((x3_j) as u64));
                    x3_j = (x3_j).wrapping_add(2u32);
                }
                let mut t1065 = core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(x2);
                t1065[j] = x2_j as u32;
                t1065[8 + j] = (x2_j as u64 >> 32) as u32;
                x2 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t1065);
            }
            break;
        }
        // Lanes part: each its own rounds.
        let t1066 = [vdupq_n_u32(15u32); 2];
        let t1067: [uint32x4_t; 2] = [vandq_u32(x1[0], t1066[0]), vandq_u32(x1[1], t1066[1])];
        let t1068 = [vdupq_n_u32(x3); 2];
        let t1069: [uint32x4_t; 2] = [vcltq_u32(t1068[0], t1067[0]), vcltq_u32(t1068[1], t1067[1])];
        t1059 = [vandq_u32(t1059[0], t1069[0]), vandq_u32(t1059[1], t1069[1])];
        if !(any(&t1059)) { break; }
        let t1071: u64 = 31u64;
        let t1072 = [vdupq_n_u32((t1071) as u32); 2];
        let t1073: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x2[0]), vget_low_u32(t1072[0]))), vreinterpretq_u32_u64(vmull_high_u32(x2[0], t1072[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x2[1]), vget_low_u32(t1072[1]))), vreinterpretq_u32_u64(vmull_high_u32(x2[1], t1072[1])))];
        let t1074: [uint32x4_t; 4] = [vmulq_u32(x2[0], t1072[0]), vmulq_u32(x2[1], t1072[1]), vmlaq_u32(t1073[0], t1072[0], x2[2]), vmlaq_u32(t1073[1], t1072[1], x2[3])];
        let t1075: u64 = ((x3) as u64);
        let t1076 = [vdupq_n_u32((t1075) as u32); 2];
        let t1077 = [vdupq_n_u32(((t1075) >> 32) as u32); 2];
        let t1078: [uint32x4_t; 2] = [vaddq_u32(t1074[0], t1076[0]), vaddq_u32(t1074[1], t1076[1])];
        let t1079: [uint32x4_t; 2] = [vcltq_u32(t1078[0], t1074[0]), vcltq_u32(t1078[1], t1074[1])];
        let t1080: [uint32x4_t; 4] = [t1078[0], t1078[1], vsubq_u32(vaddq_u32(t1074[2], t1077[0]), t1079[0]), vsubq_u32(vaddq_u32(t1074[3], t1077[1]), t1079[1])];
        x2 = [vbslq_u32(t1059[0], t1080[0], x2[0]), vbslq_u32(t1059[1], t1080[1], x2[1]), vbslq_u32(t1059[0], t1080[2], x2[2]), vbslq_u32(t1059[1], t1080[3], x2[3])];
        let t1081: [uint32x4_t; 2] = [vandq_u32(t1059[0], live[0]), vandq_u32(t1059[1], live[1])];
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
    let t1082: u64;
    if ((x0) >= (64u32)) {
        t1082 = !(0u64);
    } else {
        t1082 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t1082;
    return res;
}

#[inline(always)]
unsafe fn b8_s_f(l: &Launch, lid0: u32, x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t1083: u64;
    if ((x0) >= (64u32)) {
        t1083 = !(0u64);
    } else {
        t1083 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t1083;
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
    let mut t1084: bool = !(arg5).is_null();
    if t1084 {
        let mut t1085: bool = ((l.group[0]) >= ((arg5.wrapping_add((((x6) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        if !t1085 {
            t1085 = ((l.group[1]) >= ((arg5.wrapping_add(((((x6).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        let mut t1086: bool = t1085;
        if !t1086 {
            t1086 = ((l.group[2]) >= ((arg5.wrapping_add(((((x6).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        t1084 = t1086;
    }
    if t1084 {
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
        let t1087: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
        let mut live = t1087;
        let gid0 = l.group[0] * l.block[0] + lid0;
        'chunk: {
            b0_r0r1r2r3sg_p(l, lid0, live, arg0, arg1, arg2, arg3, x4);
        }
    }
}
