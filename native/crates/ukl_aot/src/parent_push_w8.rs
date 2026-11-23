//! `parent_push` at 8 lanes (2 NEON registers), written by rerail-ukl-vec's Rust (`rs.rs`) from its vector tape: do
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
unsafe fn b0_r0r1r2sg_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, arg2: *const u8, x24: *const u8) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x24 as *const u32).read_unaligned();
    let mut x48: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut x56: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x64: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x72: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut x80: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x88: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x96: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = k;
    let t1: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
    let t2 = [vdupq_n_u32(a0); 2];
    let t3: [uint32x4_t; 2] = [vcgeq_u32(t1[0], t2[0]), vcgeq_u32(t1[1], t2[1])];
    let t4: [uint32x4_t; 2] = [vandq_u32(live[0], t3[0]), vandq_u32(live[1], t3[1])];
    if any(&t4) {
        live = [vbicq_u32(live[0], t4[0]), vbicq_u32(live[1], t4[1])];
    }
    if any(&live) {
        let t5 = b1_r0r1g_p(l, lid0, live, arg0, arg1);
        x48 = t5;
        let t6 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
        let t7: [uint64x2_t; 4] = [vandq_u64(x48[0], t6[0]), vandq_u64(x48[1], t6[1]), vandq_u64(x48[2], t6[2]), vandq_u64(x48[3], t6[3])];
        let t8 = [vdupq_n_u64(0u64); 4];
        let t9: [uint64x2_t; 4] = [vceqq_u64(t7[0], t8[0]), vceqq_u64(t7[1], t8[1]), vceqq_u64(t7[2], t8[2]), vceqq_u64(t7[3], t8[3])];
        let t10: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t9[0]), vmovn_u64(t9[1])), vcombine_u32(vmovn_u64(t9[2]), vmovn_u64(t9[3]))];
        let t11: [uint32x4_t; 2] = [vmvnq_u32(t10[0]), vmvnq_u32(t10[1])];
        let t12: [uint32x4_t; 2] = [vandq_u32(live[0], t11[0]), vandq_u32(live[1], t11[1])];
        if any(&t12) {
            live = [vbicq_u32(live[0], t12[0]), vbicq_u32(live[1], t12[1])];
        }
        if any(&live) {
            let t13: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x48[0]), vmovn_u64(x48[1])), vcombine_u32(vmovn_u64(x48[2]), vmovn_u64(x48[3]))];
            x56 = t13;
            let t14 = (arg2 as *mut u32).add((gid0) as usize);
            if all(&live) {
                vst1q_u32(t14.add(0), x56[0]);
                vst1q_u32(t14.add(4), x56[1]);
            } else {
                let mut t15w = 0u32;
                let t15 = core::hint::black_box(&raw mut t15w);
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t14.add(0), t15).write(vgetq_lane_u32::<0>(x56[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t14.add(1), t15).write(vgetq_lane_u32::<1>(x56[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t14.add(2), t15).write(vgetq_lane_u32::<2>(x56[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t14.add(3), t15).write(vgetq_lane_u32::<3>(x56[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t14.add(4), t15).write(vgetq_lane_u32::<0>(x56[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t14.add(5), t15).write(vgetq_lane_u32::<1>(x56[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t14.add(6), t15).write(vgetq_lane_u32::<2>(x56[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t14.add(7), t15).write(vgetq_lane_u32::<3>(x56[1]));
            }
            x64 = t1;
            let t16 = b2_r0r1l_p(l, lid0, live, arg0, arg1, x64);
            x72 = t16;
            let mut t17 = live;
            let mut t18 = [vdupq_n_u32(0); 2];
            loop {
                // Lanes part: each its own rounds.
                let t19 = [vdupq_n_u32(if true { u32::MAX } else { 0 }); 2];
                t17 = [vandq_u32(vbicq_u32(t17[0], t18[0]), t19[0]), vandq_u32(vbicq_u32(t17[1], t18[1]), t19[1])];
                if !(any(&t17)) { break; }
                let t21 = [vdupq_n_u64(1u64); 4];
                let t22: [uint64x2_t; 4] = [vandq_u64(x72[0], t21[0]), vandq_u64(x72[1], t21[1]), vandq_u64(x72[2], t21[2]), vandq_u64(x72[3], t21[3])];
                let t23: [uint64x2_t; 4] = [vceqq_u64(t22[0], t8[0]), vceqq_u64(t22[1], t8[1]), vceqq_u64(t22[2], t8[2]), vceqq_u64(t22[3], t8[3])];
                let t24: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t23[0]), vmovn_u64(t23[1])), vcombine_u32(vmovn_u64(t23[2]), vmovn_u64(t23[3]))];
                let t25: [uint32x4_t; 2] = [vmvnq_u32(t24[0]), vmvnq_u32(t24[1])];
                let mut t26: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                let t27: [uint32x4_t; 2] = [vandq_u32(t17[0], t25[0]), vandq_u32(t17[1], t25[1])];
                let t28: [uint32x4_t; 2] = [vbicq_u32(t17[0], t25[0]), vbicq_u32(t17[1], t25[1])];
                {
                    let t29 = [vdupq_n_u32(1u32); 2];
                    t26 = [vbslq_u32(t27[0], t29[0], t26[0]), vbslq_u32(t27[1], t29[1], t26[1])];
                }
                {
                    let t30: [uint64x2_t; 4] = [vshlq_u64(x72[0], vnegq_s64(vdupq_n_s64(((((1u64) as u32)) & 63) as i64))), vshlq_u64(x72[1], vnegq_s64(vdupq_n_s64(((((1u64) as u32)) & 63) as i64))), vshlq_u64(x72[2], vnegq_s64(vdupq_n_s64(((((1u64) as u32)) & 63) as i64))), vshlq_u64(x72[3], vnegq_s64(vdupq_n_s64(((((1u64) as u32)) & 63) as i64)))];
                    let t31: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t30[0]), vmovn_u64(t30[1])), vcombine_u32(vmovn_u64(t30[2]), vmovn_u64(t30[3]))];
                    t26 = [vbslq_u32(t28[0], t31[0], t26[0]), vbslq_u32(t28[1], t31[1], t26[1])];
                }
                x80 = t26;
                let t32 = [vdupq_n_u32(0u32); 2];
                let t33: [uint32x4_t; 2] = [vceqq_u32(x80[0], t32[0]), vceqq_u32(x80[1], t32[1])];
                let t34: [uint32x4_t; 2] = [vandq_u32(t17[0], t33[0]), vandq_u32(t17[1], t33[1])];
                {
                    let t35 = [vdupq_n_u32(1u32); 2];
                    x80 = [vbslq_u32(t34[0], t35[0], x80[0]), vbslq_u32(t34[1], t35[1], x80[1])];
                }
                let t36: [uint32x4_t; 2] = [vaddq_u32(x64[0], x80[0]), vaddq_u32(x64[1], x80[1])];
                x88 = t36;
                let t37: [uint32x4_t; 2] = [vcgeq_u32(x88[0], t2[0]), vcgeq_u32(x88[1], t2[1])];
                let t38: [uint32x4_t; 2] = [vcleq_u32(x88[0], x64[0]), vcleq_u32(x88[1], x64[1])];
                let t39: [uint32x4_t; 2] = [vorrq_u32(t37[0], t38[0]), vorrq_u32(t37[1], t38[1])];
                let t40: [uint32x4_t; 2] = [vandq_u32(t17[0], t39[0]), vandq_u32(t17[1], t39[1])];
                if any(&t40) {
                    t18 = [vorrq_u32(t18[0], t40[0]), vorrq_u32(t18[1], t40[1])];
                }
                let t41: [uint32x4_t; 2] = [vbicq_u32(vandq_u32(t17[0], live[0]), t18[0]), vbicq_u32(vandq_u32(t17[1], live[1]), t18[1])];
                if any(&t41) {
                    let t42 = [vdupq_n_u32(1u32); 2];
                    let t43: [uint32x4_t; 2] = [vceqq_u32(x80[0], t42[0]), vceqq_u32(x80[1], t42[1])];
                    let mut t44: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
                    let t45: [uint32x4_t; 2] = [vandq_u32(t41[0], t43[0]), vandq_u32(t41[1], t43[1])];
                    let t46: [uint32x4_t; 2] = [vbicq_u32(t41[0], t43[0]), vbicq_u32(t41[1], t43[1])];
                    {
                        t44 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t45[0])))), x72[0], t44[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t45[0])))), x72[1], t44[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t45[1])))), x72[2], t44[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t45[1])))), x72[3], t44[3])];
                    }
                    if any(&t46) {
                        let t47: [uint32x4_t; 2] = [vsubq_u32(x88[0], t42[0]), vsubq_u32(x88[1], t42[1])];
                        let t48 = b2_r0r1l_p(l, lid0, t46, arg0, arg1, t47);
                        t44 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t46[0])))), t48[0], t44[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t46[0])))), t48[1], t44[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t46[1])))), t48[2], t44[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t46[1])))), t48[3], t44[3])];
                    }
                    x96 = t44;
                    let t49 = [vdupq_n_u64(3u64); 4];
                    let t50: [uint64x2_t; 4] = [vandq_u64(x96[0], t49[0]), vandq_u64(x96[1], t49[1]), vandq_u64(x96[2], t49[2]), vandq_u64(x96[3], t49[3])];
                    let t51: [uint64x2_t; 4] = [vceqq_u64(t50[0], t21[0]), vceqq_u64(t50[1], t21[1]), vceqq_u64(t50[2], t21[2]), vceqq_u64(t50[3], t21[3])];
                    let t52: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t51[0]), vmovn_u64(t51[1])), vcombine_u32(vmovn_u64(t51[2]), vmovn_u64(t51[3]))];
                    let t53: [uint32x4_t; 2] = [vmvnq_u32(t52[0]), vmvnq_u32(t52[1])];
                    let t54: [uint64x2_t; 4] = [vshlq_u64(x96[0], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x96[1], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x96[2], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x96[3], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64)))];
                    let t55: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t54[0]), vmovn_u64(t54[1])), vcombine_u32(vmovn_u64(t54[2]), vmovn_u64(t54[3]))];
                    let t56: [uint32x4_t; 2] = [vsubq_u32(x88[0], x64[0]), vsubq_u32(x88[1], x64[1])];
                    let t57: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t55[0], t56[0])), vmvnq_u32(vceqq_u32(t55[1], t56[1]))];
                    let t58: [uint32x4_t; 2] = [vorrq_u32(t53[0], t57[0]), vorrq_u32(t53[1], t57[1])];
                    let t59: [uint32x4_t; 2] = [vandq_u32(t41[0], t58[0]), vandq_u32(t41[1], t58[1])];
                    if any(&t59) {
                        t18 = [vorrq_u32(t18[0], t59[0]), vorrq_u32(t18[1], t59[1])];
                    }
                    let t60: [uint32x4_t; 2] = [vbicq_u32(vandq_u32(t41[0], live[0]), t18[0]), vbicq_u32(vandq_u32(t41[1], live[1]), t18[1])];
                    if any(&t60) {
                        let mut t61w = 0u32;
                        let t61 = core::hint::black_box(&raw mut t61w);
                        core::hint::select_unpredictable(vgetq_lane_u32::<0>(t60[0]) != 0, (arg2 as *mut u32).add(vgetq_lane_u32::<0>(x88[0]) as usize), t61).write(vgetq_lane_u32::<0>(x56[0]));
                        core::hint::select_unpredictable(vgetq_lane_u32::<1>(t60[0]) != 0, (arg2 as *mut u32).add(vgetq_lane_u32::<1>(x88[0]) as usize), t61).write(vgetq_lane_u32::<1>(x56[0]));
                        core::hint::select_unpredictable(vgetq_lane_u32::<2>(t60[0]) != 0, (arg2 as *mut u32).add(vgetq_lane_u32::<2>(x88[0]) as usize), t61).write(vgetq_lane_u32::<2>(x56[0]));
                        core::hint::select_unpredictable(vgetq_lane_u32::<3>(t60[0]) != 0, (arg2 as *mut u32).add(vgetq_lane_u32::<3>(x88[0]) as usize), t61).write(vgetq_lane_u32::<3>(x56[0]));
                        core::hint::select_unpredictable(vgetq_lane_u32::<0>(t60[1]) != 0, (arg2 as *mut u32).add(vgetq_lane_u32::<0>(x88[1]) as usize), t61).write(vgetq_lane_u32::<0>(x56[1]));
                        core::hint::select_unpredictable(vgetq_lane_u32::<1>(t60[1]) != 0, (arg2 as *mut u32).add(vgetq_lane_u32::<1>(x88[1]) as usize), t61).write(vgetq_lane_u32::<1>(x56[1]));
                        core::hint::select_unpredictable(vgetq_lane_u32::<2>(t60[1]) != 0, (arg2 as *mut u32).add(vgetq_lane_u32::<2>(x88[1]) as usize), t61).write(vgetq_lane_u32::<2>(x56[1]));
                        core::hint::select_unpredictable(vgetq_lane_u32::<3>(t60[1]) != 0, (arg2 as *mut u32).add(vgetq_lane_u32::<3>(x88[1]) as usize), t61).write(vgetq_lane_u32::<3>(x56[1]));
                        x64 = x88;
                        let t62 = b2_r0r1l_p(l, lid0, t60, arg0, arg1, x64);
                        x72 = t62;
                    }
                }
                let t63: [uint32x4_t; 2] = [vbicq_u32(vandq_u32(t17[0], live[0]), t18[0]), vbicq_u32(vandq_u32(t17[1], live[1]), t18[1])];
            }
        }
    }
}

#[inline(always)]
unsafe fn b0_r0r1r2sg_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, arg2: *const u8, x24: *const u8) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x24 as *const u32).read_unaligned();
    let mut x48: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut x56: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x64: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x72: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut x80: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x88: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x96: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    if ((gid0) as u64 + 7) < ((a0) as u64) {
        let t64 = b1_r0r1g_f(l, lid0, arg0, arg1);
        x48 = t64;
        let t65 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
        let t66: [uint64x2_t; 4] = [vandq_u64(x48[0], t65[0]), vandq_u64(x48[1], t65[1]), vandq_u64(x48[2], t65[2]), vandq_u64(x48[3], t65[3])];
        let t67 = [vdupq_n_u64(0u64); 4];
        let t68: [uint64x2_t; 4] = [vceqq_u64(t66[0], t67[0]), vceqq_u64(t66[1], t67[1]), vceqq_u64(t66[2], t67[2]), vceqq_u64(t66[3], t67[3])];
        let t69: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t68[0]), vmovn_u64(t68[1])), vcombine_u32(vmovn_u64(t68[2]), vmovn_u64(t68[3]))];
        let t70: [uint32x4_t; 2] = [vmvnq_u32(t69[0]), vmvnq_u32(t69[1])];
        if any(&t70) {
            live = [vbicq_u32(live[0], t70[0]), vbicq_u32(live[1], t70[1])];
        }
        if any(&live) {
            let t71: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x48[0]), vmovn_u64(x48[1])), vcombine_u32(vmovn_u64(x48[2]), vmovn_u64(x48[3]))];
            x56 = t71;
            let t72 = (arg2 as *mut u32).add((gid0) as usize);
            if all(&live) {
                vst1q_u32(t72.add(0), x56[0]);
                vst1q_u32(t72.add(4), x56[1]);
            } else {
                let mut t73w = 0u32;
                let t73 = core::hint::black_box(&raw mut t73w);
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t72.add(0), t73).write(vgetq_lane_u32::<0>(x56[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t72.add(1), t73).write(vgetq_lane_u32::<1>(x56[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t72.add(2), t73).write(vgetq_lane_u32::<2>(x56[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t72.add(3), t73).write(vgetq_lane_u32::<3>(x56[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t72.add(4), t73).write(vgetq_lane_u32::<0>(x56[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t72.add(5), t73).write(vgetq_lane_u32::<1>(x56[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t72.add(6), t73).write(vgetq_lane_u32::<2>(x56[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t72.add(7), t73).write(vgetq_lane_u32::<3>(x56[1]));
            }
            let t74: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
            x64 = t74;
            let t75 = b2_r0r1l_p(l, lid0, live, arg0, arg1, x64);
            x72 = t75;
            let mut t76 = live;
            let mut t77 = [vdupq_n_u32(0); 2];
            loop {
                // Lanes part: each its own rounds.
                let t78 = [vdupq_n_u32(if true { u32::MAX } else { 0 }); 2];
                t76 = [vandq_u32(vbicq_u32(t76[0], t77[0]), t78[0]), vandq_u32(vbicq_u32(t76[1], t77[1]), t78[1])];
                if !(any(&t76)) { break; }
                let t80 = [vdupq_n_u64(1u64); 4];
                let t81: [uint64x2_t; 4] = [vandq_u64(x72[0], t80[0]), vandq_u64(x72[1], t80[1]), vandq_u64(x72[2], t80[2]), vandq_u64(x72[3], t80[3])];
                let t82: [uint64x2_t; 4] = [vceqq_u64(t81[0], t67[0]), vceqq_u64(t81[1], t67[1]), vceqq_u64(t81[2], t67[2]), vceqq_u64(t81[3], t67[3])];
                let t83: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t82[0]), vmovn_u64(t82[1])), vcombine_u32(vmovn_u64(t82[2]), vmovn_u64(t82[3]))];
                let t84: [uint32x4_t; 2] = [vmvnq_u32(t83[0]), vmvnq_u32(t83[1])];
                let mut t85: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                let t86: [uint32x4_t; 2] = [vandq_u32(t76[0], t84[0]), vandq_u32(t76[1], t84[1])];
                let t87: [uint32x4_t; 2] = [vbicq_u32(t76[0], t84[0]), vbicq_u32(t76[1], t84[1])];
                {
                    let t88 = [vdupq_n_u32(1u32); 2];
                    t85 = [vbslq_u32(t86[0], t88[0], t85[0]), vbslq_u32(t86[1], t88[1], t85[1])];
                }
                {
                    let t89: [uint64x2_t; 4] = [vshlq_u64(x72[0], vnegq_s64(vdupq_n_s64(((((1u64) as u32)) & 63) as i64))), vshlq_u64(x72[1], vnegq_s64(vdupq_n_s64(((((1u64) as u32)) & 63) as i64))), vshlq_u64(x72[2], vnegq_s64(vdupq_n_s64(((((1u64) as u32)) & 63) as i64))), vshlq_u64(x72[3], vnegq_s64(vdupq_n_s64(((((1u64) as u32)) & 63) as i64)))];
                    let t90: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t89[0]), vmovn_u64(t89[1])), vcombine_u32(vmovn_u64(t89[2]), vmovn_u64(t89[3]))];
                    t85 = [vbslq_u32(t87[0], t90[0], t85[0]), vbslq_u32(t87[1], t90[1], t85[1])];
                }
                x80 = t85;
                let t91 = [vdupq_n_u32(0u32); 2];
                let t92: [uint32x4_t; 2] = [vceqq_u32(x80[0], t91[0]), vceqq_u32(x80[1], t91[1])];
                let t93: [uint32x4_t; 2] = [vandq_u32(t76[0], t92[0]), vandq_u32(t76[1], t92[1])];
                {
                    let t94 = [vdupq_n_u32(1u32); 2];
                    x80 = [vbslq_u32(t93[0], t94[0], x80[0]), vbslq_u32(t93[1], t94[1], x80[1])];
                }
                let t95: [uint32x4_t; 2] = [vaddq_u32(x64[0], x80[0]), vaddq_u32(x64[1], x80[1])];
                x88 = t95;
                let t96 = [vdupq_n_u32(a0); 2];
                let t97: [uint32x4_t; 2] = [vcgeq_u32(x88[0], t96[0]), vcgeq_u32(x88[1], t96[1])];
                let t98: [uint32x4_t; 2] = [vcleq_u32(x88[0], x64[0]), vcleq_u32(x88[1], x64[1])];
                let t99: [uint32x4_t; 2] = [vorrq_u32(t97[0], t98[0]), vorrq_u32(t97[1], t98[1])];
                let t100: [uint32x4_t; 2] = [vandq_u32(t76[0], t99[0]), vandq_u32(t76[1], t99[1])];
                if any(&t100) {
                    t77 = [vorrq_u32(t77[0], t100[0]), vorrq_u32(t77[1], t100[1])];
                }
                let t101: [uint32x4_t; 2] = [vbicq_u32(vandq_u32(t76[0], live[0]), t77[0]), vbicq_u32(vandq_u32(t76[1], live[1]), t77[1])];
                if any(&t101) {
                    let t102 = [vdupq_n_u32(1u32); 2];
                    let t103: [uint32x4_t; 2] = [vceqq_u32(x80[0], t102[0]), vceqq_u32(x80[1], t102[1])];
                    let mut t104: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
                    let t105: [uint32x4_t; 2] = [vandq_u32(t101[0], t103[0]), vandq_u32(t101[1], t103[1])];
                    let t106: [uint32x4_t; 2] = [vbicq_u32(t101[0], t103[0]), vbicq_u32(t101[1], t103[1])];
                    {
                        t104 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t105[0])))), x72[0], t104[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t105[0])))), x72[1], t104[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t105[1])))), x72[2], t104[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t105[1])))), x72[3], t104[3])];
                    }
                    if any(&t106) {
                        let t107: [uint32x4_t; 2] = [vsubq_u32(x88[0], t102[0]), vsubq_u32(x88[1], t102[1])];
                        let t108 = b2_r0r1l_p(l, lid0, t106, arg0, arg1, t107);
                        t104 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t106[0])))), t108[0], t104[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t106[0])))), t108[1], t104[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t106[1])))), t108[2], t104[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t106[1])))), t108[3], t104[3])];
                    }
                    x96 = t104;
                    let t109 = [vdupq_n_u64(3u64); 4];
                    let t110: [uint64x2_t; 4] = [vandq_u64(x96[0], t109[0]), vandq_u64(x96[1], t109[1]), vandq_u64(x96[2], t109[2]), vandq_u64(x96[3], t109[3])];
                    let t111: [uint64x2_t; 4] = [vceqq_u64(t110[0], t80[0]), vceqq_u64(t110[1], t80[1]), vceqq_u64(t110[2], t80[2]), vceqq_u64(t110[3], t80[3])];
                    let t112: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t111[0]), vmovn_u64(t111[1])), vcombine_u32(vmovn_u64(t111[2]), vmovn_u64(t111[3]))];
                    let t113: [uint32x4_t; 2] = [vmvnq_u32(t112[0]), vmvnq_u32(t112[1])];
                    let t114: [uint64x2_t; 4] = [vshlq_u64(x96[0], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x96[1], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x96[2], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x96[3], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64)))];
                    let t115: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t114[0]), vmovn_u64(t114[1])), vcombine_u32(vmovn_u64(t114[2]), vmovn_u64(t114[3]))];
                    let t116: [uint32x4_t; 2] = [vsubq_u32(x88[0], x64[0]), vsubq_u32(x88[1], x64[1])];
                    let t117: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t115[0], t116[0])), vmvnq_u32(vceqq_u32(t115[1], t116[1]))];
                    let t118: [uint32x4_t; 2] = [vorrq_u32(t113[0], t117[0]), vorrq_u32(t113[1], t117[1])];
                    let t119: [uint32x4_t; 2] = [vandq_u32(t101[0], t118[0]), vandq_u32(t101[1], t118[1])];
                    if any(&t119) {
                        t77 = [vorrq_u32(t77[0], t119[0]), vorrq_u32(t77[1], t119[1])];
                    }
                    let t120: [uint32x4_t; 2] = [vbicq_u32(vandq_u32(t101[0], live[0]), t77[0]), vbicq_u32(vandq_u32(t101[1], live[1]), t77[1])];
                    if any(&t120) {
                        let mut t121w = 0u32;
                        let t121 = core::hint::black_box(&raw mut t121w);
                        core::hint::select_unpredictable(vgetq_lane_u32::<0>(t120[0]) != 0, (arg2 as *mut u32).add(vgetq_lane_u32::<0>(x88[0]) as usize), t121).write(vgetq_lane_u32::<0>(x56[0]));
                        core::hint::select_unpredictable(vgetq_lane_u32::<1>(t120[0]) != 0, (arg2 as *mut u32).add(vgetq_lane_u32::<1>(x88[0]) as usize), t121).write(vgetq_lane_u32::<1>(x56[0]));
                        core::hint::select_unpredictable(vgetq_lane_u32::<2>(t120[0]) != 0, (arg2 as *mut u32).add(vgetq_lane_u32::<2>(x88[0]) as usize), t121).write(vgetq_lane_u32::<2>(x56[0]));
                        core::hint::select_unpredictable(vgetq_lane_u32::<3>(t120[0]) != 0, (arg2 as *mut u32).add(vgetq_lane_u32::<3>(x88[0]) as usize), t121).write(vgetq_lane_u32::<3>(x56[0]));
                        core::hint::select_unpredictable(vgetq_lane_u32::<0>(t120[1]) != 0, (arg2 as *mut u32).add(vgetq_lane_u32::<0>(x88[1]) as usize), t121).write(vgetq_lane_u32::<0>(x56[1]));
                        core::hint::select_unpredictable(vgetq_lane_u32::<1>(t120[1]) != 0, (arg2 as *mut u32).add(vgetq_lane_u32::<1>(x88[1]) as usize), t121).write(vgetq_lane_u32::<1>(x56[1]));
                        core::hint::select_unpredictable(vgetq_lane_u32::<2>(t120[1]) != 0, (arg2 as *mut u32).add(vgetq_lane_u32::<2>(x88[1]) as usize), t121).write(vgetq_lane_u32::<2>(x56[1]));
                        core::hint::select_unpredictable(vgetq_lane_u32::<3>(t120[1]) != 0, (arg2 as *mut u32).add(vgetq_lane_u32::<3>(x88[1]) as usize), t121).write(vgetq_lane_u32::<3>(x56[1]));
                        x64 = x88;
                        let t122 = b2_r0r1l_p(l, lid0, t120, arg0, arg1, x64);
                        x72 = t122;
                    }
                }
                let t123: [uint32x4_t; 2] = [vbicq_u32(vandq_u32(t76[0], live[0]), t77[0]), vbicq_u32(vandq_u32(t76[1], live[1]), t77[1])];
            }
        }
    } else if (((gid0) as u64 + 7) <= u32::MAX as u64 && ((gid0) as u64) >= ((a0) as u64)) {
        return;
    } else {
        // Lanes part: the chunk's lanes on both sides of the bound.
        let t124: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t125 = [vdupq_n_u32(a0); 2];
        let t126: [uint32x4_t; 2] = [vcgeq_u32(t124[0], t125[0]), vcgeq_u32(t124[1], t125[1])];
        let t127: [uint32x4_t; 2] = [vandq_u32(live[0], t126[0]), vandq_u32(live[1], t126[1])];
        if any(&t127) {
            live = [vbicq_u32(live[0], t127[0]), vbicq_u32(live[1], t127[1])];
        }
        if any(&live) {
            let t128 = b1_r0r1g_p(l, lid0, live, arg0, arg1);
            x48 = t128;
            let t129 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
            let t130: [uint64x2_t; 4] = [vandq_u64(x48[0], t129[0]), vandq_u64(x48[1], t129[1]), vandq_u64(x48[2], t129[2]), vandq_u64(x48[3], t129[3])];
            let t131 = [vdupq_n_u64(0u64); 4];
            let t132: [uint64x2_t; 4] = [vceqq_u64(t130[0], t131[0]), vceqq_u64(t130[1], t131[1]), vceqq_u64(t130[2], t131[2]), vceqq_u64(t130[3], t131[3])];
            let t133: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t132[0]), vmovn_u64(t132[1])), vcombine_u32(vmovn_u64(t132[2]), vmovn_u64(t132[3]))];
            let t134: [uint32x4_t; 2] = [vmvnq_u32(t133[0]), vmvnq_u32(t133[1])];
            let t135: [uint32x4_t; 2] = [vandq_u32(live[0], t134[0]), vandq_u32(live[1], t134[1])];
            if any(&t135) {
                live = [vbicq_u32(live[0], t135[0]), vbicq_u32(live[1], t135[1])];
            }
            if any(&live) {
                let t136: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x48[0]), vmovn_u64(x48[1])), vcombine_u32(vmovn_u64(x48[2]), vmovn_u64(x48[3]))];
                x56 = t136;
                let t137 = (arg2 as *mut u32).add((gid0) as usize);
                if all(&live) {
                    vst1q_u32(t137.add(0), x56[0]);
                    vst1q_u32(t137.add(4), x56[1]);
                } else {
                    let mut t138w = 0u32;
                    let t138 = core::hint::black_box(&raw mut t138w);
                    core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t137.add(0), t138).write(vgetq_lane_u32::<0>(x56[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t137.add(1), t138).write(vgetq_lane_u32::<1>(x56[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t137.add(2), t138).write(vgetq_lane_u32::<2>(x56[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t137.add(3), t138).write(vgetq_lane_u32::<3>(x56[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t137.add(4), t138).write(vgetq_lane_u32::<0>(x56[1]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t137.add(5), t138).write(vgetq_lane_u32::<1>(x56[1]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t137.add(6), t138).write(vgetq_lane_u32::<2>(x56[1]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t137.add(7), t138).write(vgetq_lane_u32::<3>(x56[1]));
                }
                x64 = t124;
                let t139 = b2_r0r1l_p(l, lid0, live, arg0, arg1, x64);
                x72 = t139;
                let mut t140 = live;
                let mut t141 = [vdupq_n_u32(0); 2];
                loop {
                    // Lanes part: each its own rounds.
                    let t142 = [vdupq_n_u32(if true { u32::MAX } else { 0 }); 2];
                    t140 = [vandq_u32(vbicq_u32(t140[0], t141[0]), t142[0]), vandq_u32(vbicq_u32(t140[1], t141[1]), t142[1])];
                    if !(any(&t140)) { break; }
                    let t144 = [vdupq_n_u64(1u64); 4];
                    let t145: [uint64x2_t; 4] = [vandq_u64(x72[0], t144[0]), vandq_u64(x72[1], t144[1]), vandq_u64(x72[2], t144[2]), vandq_u64(x72[3], t144[3])];
                    let t146: [uint64x2_t; 4] = [vceqq_u64(t145[0], t131[0]), vceqq_u64(t145[1], t131[1]), vceqq_u64(t145[2], t131[2]), vceqq_u64(t145[3], t131[3])];
                    let t147: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t146[0]), vmovn_u64(t146[1])), vcombine_u32(vmovn_u64(t146[2]), vmovn_u64(t146[3]))];
                    let t148: [uint32x4_t; 2] = [vmvnq_u32(t147[0]), vmvnq_u32(t147[1])];
                    let mut t149: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                    let t150: [uint32x4_t; 2] = [vandq_u32(t140[0], t148[0]), vandq_u32(t140[1], t148[1])];
                    let t151: [uint32x4_t; 2] = [vbicq_u32(t140[0], t148[0]), vbicq_u32(t140[1], t148[1])];
                    {
                        let t152 = [vdupq_n_u32(1u32); 2];
                        t149 = [vbslq_u32(t150[0], t152[0], t149[0]), vbslq_u32(t150[1], t152[1], t149[1])];
                    }
                    {
                        let t153: [uint64x2_t; 4] = [vshlq_u64(x72[0], vnegq_s64(vdupq_n_s64(((((1u64) as u32)) & 63) as i64))), vshlq_u64(x72[1], vnegq_s64(vdupq_n_s64(((((1u64) as u32)) & 63) as i64))), vshlq_u64(x72[2], vnegq_s64(vdupq_n_s64(((((1u64) as u32)) & 63) as i64))), vshlq_u64(x72[3], vnegq_s64(vdupq_n_s64(((((1u64) as u32)) & 63) as i64)))];
                        let t154: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t153[0]), vmovn_u64(t153[1])), vcombine_u32(vmovn_u64(t153[2]), vmovn_u64(t153[3]))];
                        t149 = [vbslq_u32(t151[0], t154[0], t149[0]), vbslq_u32(t151[1], t154[1], t149[1])];
                    }
                    x80 = t149;
                    let t155 = [vdupq_n_u32(0u32); 2];
                    let t156: [uint32x4_t; 2] = [vceqq_u32(x80[0], t155[0]), vceqq_u32(x80[1], t155[1])];
                    let t157: [uint32x4_t; 2] = [vandq_u32(t140[0], t156[0]), vandq_u32(t140[1], t156[1])];
                    {
                        let t158 = [vdupq_n_u32(1u32); 2];
                        x80 = [vbslq_u32(t157[0], t158[0], x80[0]), vbslq_u32(t157[1], t158[1], x80[1])];
                    }
                    let t159: [uint32x4_t; 2] = [vaddq_u32(x64[0], x80[0]), vaddq_u32(x64[1], x80[1])];
                    x88 = t159;
                    let t160: [uint32x4_t; 2] = [vcgeq_u32(x88[0], t125[0]), vcgeq_u32(x88[1], t125[1])];
                    let t161: [uint32x4_t; 2] = [vcleq_u32(x88[0], x64[0]), vcleq_u32(x88[1], x64[1])];
                    let t162: [uint32x4_t; 2] = [vorrq_u32(t160[0], t161[0]), vorrq_u32(t160[1], t161[1])];
                    let t163: [uint32x4_t; 2] = [vandq_u32(t140[0], t162[0]), vandq_u32(t140[1], t162[1])];
                    if any(&t163) {
                        t141 = [vorrq_u32(t141[0], t163[0]), vorrq_u32(t141[1], t163[1])];
                    }
                    let t164: [uint32x4_t; 2] = [vbicq_u32(vandq_u32(t140[0], live[0]), t141[0]), vbicq_u32(vandq_u32(t140[1], live[1]), t141[1])];
                    if any(&t164) {
                        let t165 = [vdupq_n_u32(1u32); 2];
                        let t166: [uint32x4_t; 2] = [vceqq_u32(x80[0], t165[0]), vceqq_u32(x80[1], t165[1])];
                        let mut t167: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
                        let t168: [uint32x4_t; 2] = [vandq_u32(t164[0], t166[0]), vandq_u32(t164[1], t166[1])];
                        let t169: [uint32x4_t; 2] = [vbicq_u32(t164[0], t166[0]), vbicq_u32(t164[1], t166[1])];
                        {
                            t167 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t168[0])))), x72[0], t167[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t168[0])))), x72[1], t167[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t168[1])))), x72[2], t167[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t168[1])))), x72[3], t167[3])];
                        }
                        if any(&t169) {
                            let t170: [uint32x4_t; 2] = [vsubq_u32(x88[0], t165[0]), vsubq_u32(x88[1], t165[1])];
                            let t171 = b2_r0r1l_p(l, lid0, t169, arg0, arg1, t170);
                            t167 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t169[0])))), t171[0], t167[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t169[0])))), t171[1], t167[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t169[1])))), t171[2], t167[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t169[1])))), t171[3], t167[3])];
                        }
                        x96 = t167;
                        let t172 = [vdupq_n_u64(3u64); 4];
                        let t173: [uint64x2_t; 4] = [vandq_u64(x96[0], t172[0]), vandq_u64(x96[1], t172[1]), vandq_u64(x96[2], t172[2]), vandq_u64(x96[3], t172[3])];
                        let t174: [uint64x2_t; 4] = [vceqq_u64(t173[0], t144[0]), vceqq_u64(t173[1], t144[1]), vceqq_u64(t173[2], t144[2]), vceqq_u64(t173[3], t144[3])];
                        let t175: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t174[0]), vmovn_u64(t174[1])), vcombine_u32(vmovn_u64(t174[2]), vmovn_u64(t174[3]))];
                        let t176: [uint32x4_t; 2] = [vmvnq_u32(t175[0]), vmvnq_u32(t175[1])];
                        let t177: [uint64x2_t; 4] = [vshlq_u64(x96[0], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x96[1], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x96[2], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x96[3], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64)))];
                        let t178: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t177[0]), vmovn_u64(t177[1])), vcombine_u32(vmovn_u64(t177[2]), vmovn_u64(t177[3]))];
                        let t179: [uint32x4_t; 2] = [vsubq_u32(x88[0], x64[0]), vsubq_u32(x88[1], x64[1])];
                        let t180: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t178[0], t179[0])), vmvnq_u32(vceqq_u32(t178[1], t179[1]))];
                        let t181: [uint32x4_t; 2] = [vorrq_u32(t176[0], t180[0]), vorrq_u32(t176[1], t180[1])];
                        let t182: [uint32x4_t; 2] = [vandq_u32(t164[0], t181[0]), vandq_u32(t164[1], t181[1])];
                        if any(&t182) {
                            t141 = [vorrq_u32(t141[0], t182[0]), vorrq_u32(t141[1], t182[1])];
                        }
                        let t183: [uint32x4_t; 2] = [vbicq_u32(vandq_u32(t164[0], live[0]), t141[0]), vbicq_u32(vandq_u32(t164[1], live[1]), t141[1])];
                        if any(&t183) {
                            let mut t184w = 0u32;
                            let t184 = core::hint::black_box(&raw mut t184w);
                            core::hint::select_unpredictable(vgetq_lane_u32::<0>(t183[0]) != 0, (arg2 as *mut u32).add(vgetq_lane_u32::<0>(x88[0]) as usize), t184).write(vgetq_lane_u32::<0>(x56[0]));
                            core::hint::select_unpredictable(vgetq_lane_u32::<1>(t183[0]) != 0, (arg2 as *mut u32).add(vgetq_lane_u32::<1>(x88[0]) as usize), t184).write(vgetq_lane_u32::<1>(x56[0]));
                            core::hint::select_unpredictable(vgetq_lane_u32::<2>(t183[0]) != 0, (arg2 as *mut u32).add(vgetq_lane_u32::<2>(x88[0]) as usize), t184).write(vgetq_lane_u32::<2>(x56[0]));
                            core::hint::select_unpredictable(vgetq_lane_u32::<3>(t183[0]) != 0, (arg2 as *mut u32).add(vgetq_lane_u32::<3>(x88[0]) as usize), t184).write(vgetq_lane_u32::<3>(x56[0]));
                            core::hint::select_unpredictable(vgetq_lane_u32::<0>(t183[1]) != 0, (arg2 as *mut u32).add(vgetq_lane_u32::<0>(x88[1]) as usize), t184).write(vgetq_lane_u32::<0>(x56[1]));
                            core::hint::select_unpredictable(vgetq_lane_u32::<1>(t183[1]) != 0, (arg2 as *mut u32).add(vgetq_lane_u32::<1>(x88[1]) as usize), t184).write(vgetq_lane_u32::<1>(x56[1]));
                            core::hint::select_unpredictable(vgetq_lane_u32::<2>(t183[1]) != 0, (arg2 as *mut u32).add(vgetq_lane_u32::<2>(x88[1]) as usize), t184).write(vgetq_lane_u32::<2>(x56[1]));
                            core::hint::select_unpredictable(vgetq_lane_u32::<3>(t183[1]) != 0, (arg2 as *mut u32).add(vgetq_lane_u32::<3>(x88[1]) as usize), t184).write(vgetq_lane_u32::<3>(x56[1]));
                            x64 = x88;
                            let t185 = b2_r0r1l_p(l, lid0, t183, arg0, arg1, x64);
                            x72 = t185;
                        }
                    }
                    let t186: [uint32x4_t; 2] = [vbicq_u32(vandq_u32(t140[0], live[0]), t141[0]), vbicq_u32(vandq_u32(t140[1], live[1]), t141[1])];
                }
            }
        }
    }
}

#[inline(always)]
unsafe fn b1_r0r1g_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x32: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t187: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
    let t188 = [vdupq_n_u32(0u32); 2];
    let t189: [uint32x4_t; 2] = [vceqq_u32(t187[0], t188[0]), vceqq_u32(t187[1], t188[1])];
    let t190: [uint32x4_t; 2] = [vandq_u32(live[0], t189[0]), vandq_u32(live[1], t189[1])];
    if any(&t190) {
        let t191 = [vdupq_n_u64(4294967295u64); 4];
        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t190[0])))), t191[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t190[0])))), t191[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t190[1])))), t191[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t190[1])))), t191[3], res[3])];
        live = [vbicq_u32(live[0], t190[0]), vbicq_u32(live[1], t190[1])];
    }
    if any(&live) {
        let t192 = [vdupq_n_u32(1u32); 2];
        let t193: [uint32x4_t; 2] = [vsubq_u32(t187[0], t192[0]), vsubq_u32(t187[1], t192[1])];
        x24 = [vbslq_u32(live[0], t193[0], x24[0]), vbslq_u32(live[1], t193[1], x24[1])];
        let t194 = b2_r0r1l_p(l, lid0, live, arg0, arg1, x24);
        x32 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t194[0], x32[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t194[1], x32[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t194[2], x32[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t194[3], x32[3])];
        let t195 = [vdupq_n_u64(1u64); 4];
        let t196: [uint64x2_t; 4] = [vandq_u64(x32[0], t195[0]), vandq_u64(x32[1], t195[1]), vandq_u64(x32[2], t195[2]), vandq_u64(x32[3], t195[3])];
        let t197 = [vdupq_n_u64(0u64); 4];
        let t198: [uint64x2_t; 4] = [vceqq_u64(t196[0], t197[0]), vceqq_u64(t196[1], t197[1]), vceqq_u64(t196[2], t197[2]), vceqq_u64(t196[3], t197[3])];
        let t199: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t198[0]), vmovn_u64(t198[1])), vcombine_u32(vmovn_u64(t198[2]), vmovn_u64(t198[3]))];
        let t200: [uint32x4_t; 2] = [vandq_u32(live[0], t199[0]), vandq_u32(live[1], t199[1])];
        if any(&t200) {
            let t201: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x24[0])), vmovl_u32(vget_high_u32(x24[0])), vmovl_u32(vget_low_u32(x24[1])), vmovl_u32(vget_high_u32(x24[1]))];
            res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t200[0])))), t201[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t200[0])))), t201[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t200[1])))), t201[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t200[1])))), t201[3], res[3])];
            live = [vbicq_u32(live[0], t200[0]), vbicq_u32(live[1], t200[1])];
        }
        if any(&live) {
            let t202: [uint64x2_t; 4] = [vshlq_u64(x32[0], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[1], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[2], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[3], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64)))];
            let t203: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t202[0]), vmovn_u64(t202[1])), vcombine_u32(vmovn_u64(t202[2]), vmovn_u64(t202[3]))];
            x40 = [vbslq_u32(live[0], t203[0], x40[0]), vbslq_u32(live[1], t203[1], x40[1])];
            let t204: [uint32x4_t; 2] = [vsubq_u32(x40[0], t192[0]), vsubq_u32(x40[1], t192[1])];
            let t205: [uint32x4_t; 2] = [vcgeq_u32(t204[0], t187[0]), vcgeq_u32(t204[1], t187[1])];
            let t206: [uint32x4_t; 2] = [vandq_u32(live[0], t205[0]), vandq_u32(live[1], t205[1])];
            if any(&t206) {
                let t207 = [vdupq_n_u64(4294967295u64); 4];
                res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t206[0])))), t207[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t206[0])))), t207[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t206[1])))), t207[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t206[1])))), t207[3], res[3])];
                live = [vbicq_u32(live[0], t206[0]), vbicq_u32(live[1], t206[1])];
            }
            if any(&live) {
                let t208: [uint32x4_t; 2] = [vsubq_u32(t187[0], x40[0]), vsubq_u32(t187[1], x40[1])];
                x48 = [vbslq_u32(live[0], t208[0], x48[0]), vbslq_u32(live[1], t208[1], x48[1])];
                let t209 = [vdupq_n_u64(2u64); 4];
                let t210: [uint64x2_t; 4] = [vandq_u64(x32[0], t209[0]), vandq_u64(x32[1], t209[1]), vandq_u64(x32[2], t209[2]), vandq_u64(x32[3], t209[3])];
                let t211: [uint64x2_t; 4] = [vceqq_u64(t210[0], t197[0]), vceqq_u64(t210[1], t197[1]), vceqq_u64(t210[2], t197[2]), vceqq_u64(t210[3], t197[3])];
                let t212: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t211[0]), vmovn_u64(t211[1])), vcombine_u32(vmovn_u64(t211[2]), vmovn_u64(t211[3]))];
                let t213: [uint32x4_t; 2] = [vmvnq_u32(t212[0]), vmvnq_u32(t212[1])];
                let t214: [uint32x4_t; 2] = [vandq_u32(live[0], t213[0]), vandq_u32(live[1], t213[1])];
                if any(&t214) {
                    let t215: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                    res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t214[0])))), t215[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t214[0])))), t215[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t214[1])))), t215[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t214[1])))), t215[3], res[3])];
                    live = [vbicq_u32(live[0], t214[0]), vbicq_u32(live[1], t214[1])];
                }
                if any(&live) {
                    let t216: [uint32x4_t; 2] = [vceqq_u32(x48[0], t188[0]), vceqq_u32(x48[1], t188[1])];
                    let t217: [uint32x4_t; 2] = [vandq_u32(live[0], t216[0]), vandq_u32(live[1], t216[1])];
                    if any(&t217) {
                        let t218 = [vdupq_n_u64(4294967295u64); 4];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t217[0])))), t218[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t217[0])))), t218[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t217[1])))), t218[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t217[1])))), t218[3], res[3])];
                        live = [vbicq_u32(live[0], t217[0]), vbicq_u32(live[1], t217[1])];
                    }
                    if any(&live) {
                        let t219 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
                        let t220: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                        let t221: [uint64x2_t; 4] = [vorrq_u64(t219[0], t220[0]), vorrq_u64(t219[1], t220[1]), vorrq_u64(t219[2], t220[2]), vorrq_u64(t219[3], t220[3])];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t221[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t221[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t221[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t221[3], res[3])];
                        live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
                    }
                }
            }
        }
    }
    res
}

#[inline(always)]
unsafe fn b1_r0r1g_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x32: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    if (((gid0) as u64 + 7) <= u32::MAX as u64 && (((0u32) as u64) < ((gid0) as u64) || ((0u32) as u64) > ((gid0) as u64 + 7))) {
        let t222: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t223 = [vdupq_n_u32(1u32); 2];
        let t224: [uint32x4_t; 2] = [vsubq_u32(t222[0], t223[0]), vsubq_u32(t222[1], t223[1])];
        x24 = t224;
        let t225 = b2_r0r1l_f(l, lid0, arg0, arg1, x24);
        x32 = t225;
        let t226 = [vdupq_n_u64(1u64); 4];
        let t227: [uint64x2_t; 4] = [vandq_u64(x32[0], t226[0]), vandq_u64(x32[1], t226[1]), vandq_u64(x32[2], t226[2]), vandq_u64(x32[3], t226[3])];
        let t228 = [vdupq_n_u64(0u64); 4];
        let t229: [uint64x2_t; 4] = [vceqq_u64(t227[0], t228[0]), vceqq_u64(t227[1], t228[1]), vceqq_u64(t227[2], t228[2]), vceqq_u64(t227[3], t228[3])];
        let t230: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t229[0]), vmovn_u64(t229[1])), vcombine_u32(vmovn_u64(t229[2]), vmovn_u64(t229[3]))];
        if any(&t230) {
            let t231: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x24[0])), vmovl_u32(vget_high_u32(x24[0])), vmovl_u32(vget_low_u32(x24[1])), vmovl_u32(vget_high_u32(x24[1]))];
            res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t230[0])))), t231[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t230[0])))), t231[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t230[1])))), t231[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t230[1])))), t231[3], res[3])];
            live = [vbicq_u32(live[0], t230[0]), vbicq_u32(live[1], t230[1])];
        }
        if any(&live) {
            let t232: [uint64x2_t; 4] = [vshlq_u64(x32[0], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[1], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[2], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[3], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64)))];
            let t233: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t232[0]), vmovn_u64(t232[1])), vcombine_u32(vmovn_u64(t232[2]), vmovn_u64(t232[3]))];
            x40 = [vbslq_u32(live[0], t233[0], x40[0]), vbslq_u32(live[1], t233[1], x40[1])];
            let t234: [uint32x4_t; 2] = [vsubq_u32(x40[0], t223[0]), vsubq_u32(x40[1], t223[1])];
            let t235: [uint32x4_t; 2] = [vcgeq_u32(t234[0], t222[0]), vcgeq_u32(t234[1], t222[1])];
            let t236: [uint32x4_t; 2] = [vandq_u32(live[0], t235[0]), vandq_u32(live[1], t235[1])];
            if any(&t236) {
                let t237 = [vdupq_n_u64(4294967295u64); 4];
                res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t236[0])))), t237[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t236[0])))), t237[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t236[1])))), t237[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t236[1])))), t237[3], res[3])];
                live = [vbicq_u32(live[0], t236[0]), vbicq_u32(live[1], t236[1])];
            }
            if any(&live) {
                let t238: [uint32x4_t; 2] = [vsubq_u32(t222[0], x40[0]), vsubq_u32(t222[1], x40[1])];
                x48 = [vbslq_u32(live[0], t238[0], x48[0]), vbslq_u32(live[1], t238[1], x48[1])];
                let t239 = [vdupq_n_u64(2u64); 4];
                let t240: [uint64x2_t; 4] = [vandq_u64(x32[0], t239[0]), vandq_u64(x32[1], t239[1]), vandq_u64(x32[2], t239[2]), vandq_u64(x32[3], t239[3])];
                let t241: [uint64x2_t; 4] = [vceqq_u64(t240[0], t228[0]), vceqq_u64(t240[1], t228[1]), vceqq_u64(t240[2], t228[2]), vceqq_u64(t240[3], t228[3])];
                let t242: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t241[0]), vmovn_u64(t241[1])), vcombine_u32(vmovn_u64(t241[2]), vmovn_u64(t241[3]))];
                let t243: [uint32x4_t; 2] = [vmvnq_u32(t242[0]), vmvnq_u32(t242[1])];
                let t244: [uint32x4_t; 2] = [vandq_u32(live[0], t243[0]), vandq_u32(live[1], t243[1])];
                if any(&t244) {
                    let t245: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                    res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t244[0])))), t245[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t244[0])))), t245[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t244[1])))), t245[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t244[1])))), t245[3], res[3])];
                    live = [vbicq_u32(live[0], t244[0]), vbicq_u32(live[1], t244[1])];
                }
                if any(&live) {
                    let t246 = [vdupq_n_u32(0u32); 2];
                    let t247: [uint32x4_t; 2] = [vceqq_u32(x48[0], t246[0]), vceqq_u32(x48[1], t246[1])];
                    let t248: [uint32x4_t; 2] = [vandq_u32(live[0], t247[0]), vandq_u32(live[1], t247[1])];
                    if any(&t248) {
                        let t249 = [vdupq_n_u64(4294967295u64); 4];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t248[0])))), t249[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t248[0])))), t249[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t248[1])))), t249[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t248[1])))), t249[3], res[3])];
                        live = [vbicq_u32(live[0], t248[0]), vbicq_u32(live[1], t248[1])];
                    }
                    if any(&live) {
                        let t250 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
                        let t251: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                        let t252: [uint64x2_t; 4] = [vorrq_u64(t250[0], t251[0]), vorrq_u64(t250[1], t251[1]), vorrq_u64(t250[2], t251[2]), vorrq_u64(t250[3], t251[3])];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t252[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t252[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t252[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t252[3], res[3])];
                        live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
                    }
                }
            }
        }
    } else if false {
        let t253 = [vdupq_n_u64(4294967295u64); 4];
        res = t253;
        return res;
    } else {
        // Lanes part: the chunk's lanes on both sides of the bound.
        let t254: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t255 = [vdupq_n_u32(0u32); 2];
        let t256: [uint32x4_t; 2] = [vceqq_u32(t254[0], t255[0]), vceqq_u32(t254[1], t255[1])];
        let t257: [uint32x4_t; 2] = [vandq_u32(live[0], t256[0]), vandq_u32(live[1], t256[1])];
        if any(&t257) {
            let t258 = [vdupq_n_u64(4294967295u64); 4];
            res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t257[0])))), t258[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t257[0])))), t258[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t257[1])))), t258[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t257[1])))), t258[3], res[3])];
            live = [vbicq_u32(live[0], t257[0]), vbicq_u32(live[1], t257[1])];
        }
        if any(&live) {
            let t259 = [vdupq_n_u32(1u32); 2];
            let t260: [uint32x4_t; 2] = [vsubq_u32(t254[0], t259[0]), vsubq_u32(t254[1], t259[1])];
            x24 = [vbslq_u32(live[0], t260[0], x24[0]), vbslq_u32(live[1], t260[1], x24[1])];
            let t261 = b2_r0r1l_p(l, lid0, live, arg0, arg1, x24);
            x32 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t261[0], x32[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t261[1], x32[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t261[2], x32[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t261[3], x32[3])];
            let t262 = [vdupq_n_u64(1u64); 4];
            let t263: [uint64x2_t; 4] = [vandq_u64(x32[0], t262[0]), vandq_u64(x32[1], t262[1]), vandq_u64(x32[2], t262[2]), vandq_u64(x32[3], t262[3])];
            let t264 = [vdupq_n_u64(0u64); 4];
            let t265: [uint64x2_t; 4] = [vceqq_u64(t263[0], t264[0]), vceqq_u64(t263[1], t264[1]), vceqq_u64(t263[2], t264[2]), vceqq_u64(t263[3], t264[3])];
            let t266: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t265[0]), vmovn_u64(t265[1])), vcombine_u32(vmovn_u64(t265[2]), vmovn_u64(t265[3]))];
            let t267: [uint32x4_t; 2] = [vandq_u32(live[0], t266[0]), vandq_u32(live[1], t266[1])];
            if any(&t267) {
                let t268: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x24[0])), vmovl_u32(vget_high_u32(x24[0])), vmovl_u32(vget_low_u32(x24[1])), vmovl_u32(vget_high_u32(x24[1]))];
                res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t267[0])))), t268[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t267[0])))), t268[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t267[1])))), t268[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t267[1])))), t268[3], res[3])];
                live = [vbicq_u32(live[0], t267[0]), vbicq_u32(live[1], t267[1])];
            }
            if any(&live) {
                let t269: [uint64x2_t; 4] = [vshlq_u64(x32[0], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[1], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[2], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[3], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64)))];
                let t270: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t269[0]), vmovn_u64(t269[1])), vcombine_u32(vmovn_u64(t269[2]), vmovn_u64(t269[3]))];
                x40 = [vbslq_u32(live[0], t270[0], x40[0]), vbslq_u32(live[1], t270[1], x40[1])];
                let t271: [uint32x4_t; 2] = [vsubq_u32(x40[0], t259[0]), vsubq_u32(x40[1], t259[1])];
                let t272: [uint32x4_t; 2] = [vcgeq_u32(t271[0], t254[0]), vcgeq_u32(t271[1], t254[1])];
                let t273: [uint32x4_t; 2] = [vandq_u32(live[0], t272[0]), vandq_u32(live[1], t272[1])];
                if any(&t273) {
                    let t274 = [vdupq_n_u64(4294967295u64); 4];
                    res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t273[0])))), t274[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t273[0])))), t274[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t273[1])))), t274[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t273[1])))), t274[3], res[3])];
                    live = [vbicq_u32(live[0], t273[0]), vbicq_u32(live[1], t273[1])];
                }
                if any(&live) {
                    let t275: [uint32x4_t; 2] = [vsubq_u32(t254[0], x40[0]), vsubq_u32(t254[1], x40[1])];
                    x48 = [vbslq_u32(live[0], t275[0], x48[0]), vbslq_u32(live[1], t275[1], x48[1])];
                    let t276 = [vdupq_n_u64(2u64); 4];
                    let t277: [uint64x2_t; 4] = [vandq_u64(x32[0], t276[0]), vandq_u64(x32[1], t276[1]), vandq_u64(x32[2], t276[2]), vandq_u64(x32[3], t276[3])];
                    let t278: [uint64x2_t; 4] = [vceqq_u64(t277[0], t264[0]), vceqq_u64(t277[1], t264[1]), vceqq_u64(t277[2], t264[2]), vceqq_u64(t277[3], t264[3])];
                    let t279: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t278[0]), vmovn_u64(t278[1])), vcombine_u32(vmovn_u64(t278[2]), vmovn_u64(t278[3]))];
                    let t280: [uint32x4_t; 2] = [vmvnq_u32(t279[0]), vmvnq_u32(t279[1])];
                    let t281: [uint32x4_t; 2] = [vandq_u32(live[0], t280[0]), vandq_u32(live[1], t280[1])];
                    if any(&t281) {
                        let t282: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t281[0])))), t282[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t281[0])))), t282[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t281[1])))), t282[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t281[1])))), t282[3], res[3])];
                        live = [vbicq_u32(live[0], t281[0]), vbicq_u32(live[1], t281[1])];
                    }
                    if any(&live) {
                        let t283: [uint32x4_t; 2] = [vceqq_u32(x48[0], t255[0]), vceqq_u32(x48[1], t255[1])];
                        let t284: [uint32x4_t; 2] = [vandq_u32(live[0], t283[0]), vandq_u32(live[1], t283[1])];
                        if any(&t284) {
                            let t285 = [vdupq_n_u64(4294967295u64); 4];
                            res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t284[0])))), t285[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t284[0])))), t285[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t284[1])))), t285[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t284[1])))), t285[3], res[3])];
                            live = [vbicq_u32(live[0], t284[0]), vbicq_u32(live[1], t284[1])];
                        }
                        if any(&live) {
                            let t286 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
                            let t287: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                            let t288: [uint64x2_t; 4] = [vorrq_u64(t286[0], t287[0]), vorrq_u64(t286[1], t287[1]), vorrq_u64(t286[2], t287[2]), vorrq_u64(t286[3], t287[3])];
                            res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t288[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t288[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t288[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t288[3], res[3])];
                            live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
                        }
                    }
                }
            }
        }
    }
    res
}

#[inline(always)]
unsafe fn b2_r0r1l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t289 = b3_r0r1sl_p(l, lid0, live, arg0, arg1, 0u32, x16);
    res = t289;
    return res;
}

#[inline(always)]
unsafe fn b2_r0r1l_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t290 = b3_r0r1sl_f(l, lid0, arg0, arg1, 0u32, x16);
    res = t290;
    return res;
}

#[inline(always)]
unsafe fn b3_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: u64 = 0;
    let mut x56: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t291 = b4_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    x32 = t291;
    let t292: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t293 = b5_r0r1sl_p(l, lid0, live, arg0, arg1, (1u32).wrapping_add(x16), t292);
    x40 = t293;
    let t294 = b6_s_p(l, lid0, live, x32);
    x48 = t294;
    let t295 = [vdupq_n_u32((1024u32).wrapping_sub(1u32)); 2];
    let t296: [uint32x4_t; 2] = [vandq_u32(x24[0], t295[0]), vandq_u32(x24[1], t295[1])];
    let t297 = [vdupq_n_u32(x32); 2];
    let t298: [uint32x4_t; 2] = [vmulq_u32(t296[0], t297[0]), vmulq_u32(t296[1], t297[1])];
    let t299 = b7_r0r1lls_p(l, lid0, live, arg0, arg1, x40, t298, 8u32);
    let t300 = [vdupq_n_u64(x48); 4];
    let t301: [uint64x2_t; 4] = [vandq_u64(t299[0], t300[0]), vandq_u64(t299[1], t300[1]), vandq_u64(t299[2], t300[2]), vandq_u64(t299[3], t300[3])];
    x56 = t301;
    let t302 = [vdupq_n_u64(x48); 4];
    let t303: [uint64x2_t; 4] = [vceqq_u64(x56[0], t302[0]), vceqq_u64(x56[1], t302[1]), vceqq_u64(x56[2], t302[2]), vceqq_u64(x56[3], t302[3])];
    let t304: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t303[0]), vmovn_u64(t303[1])), vcombine_u32(vmovn_u64(t303[2]), vmovn_u64(t303[3]))];
    let mut t305: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t306: [uint32x4_t; 2] = [vandq_u32(live[0], t304[0]), vandq_u32(live[1], t304[1])];
    let t307: [uint32x4_t; 2] = [vbicq_u32(live[0], t304[0]), vbicq_u32(live[1], t304[1])];
    if any(&t306) {
        let t308 = b8_r0r1sl_p(l, lid0, t306, arg0, arg1, (x16).wrapping_add(1u32), x24);
        t305 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t306[0])))), t308[0], t305[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t306[0])))), t308[1], t305[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t306[1])))), t308[2], t305[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t306[1])))), t308[3], t305[3])];
    }
    {
        t305 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t307[0])))), x56[0], t305[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t307[0])))), x56[1], t305[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t307[1])))), x56[2], t305[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t307[1])))), x56[3], t305[3])];
    }
    res = t305;
    return res;
}

#[inline(always)]
unsafe fn b3_r0r1sl_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: u64 = 0;
    let mut x56: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t309 = b4_r0r1s_f(l, lid0, arg0, arg1, x16);
    x32 = t309;
    let t310: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t311 = b5_r0r1sl_f(l, lid0, arg0, arg1, (1u32).wrapping_add(x16), t310);
    x40 = t311;
    let t312 = b6_s_f(l, lid0, x32);
    x48 = t312;
    let t313 = [vdupq_n_u32((1024u32).wrapping_sub(1u32)); 2];
    let t314: [uint32x4_t; 2] = [vandq_u32(x24[0], t313[0]), vandq_u32(x24[1], t313[1])];
    let t315 = [vdupq_n_u32(x32); 2];
    let t316: [uint32x4_t; 2] = [vmulq_u32(t314[0], t315[0]), vmulq_u32(t314[1], t315[1])];
    let t317 = b7_r0r1lls_f(l, lid0, arg0, arg1, x40, t316, 8u32);
    let t318 = [vdupq_n_u64(x48); 4];
    let t319: [uint64x2_t; 4] = [vandq_u64(t317[0], t318[0]), vandq_u64(t317[1], t318[1]), vandq_u64(t317[2], t318[2]), vandq_u64(t317[3], t318[3])];
    x56 = t319;
    let t320 = [vdupq_n_u64(x48); 4];
    let t321: [uint64x2_t; 4] = [vceqq_u64(x56[0], t320[0]), vceqq_u64(x56[1], t320[1]), vceqq_u64(x56[2], t320[2]), vceqq_u64(x56[3], t320[3])];
    let t322: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t321[0]), vmovn_u64(t321[1])), vcombine_u32(vmovn_u64(t321[2]), vmovn_u64(t321[3]))];
    let mut t323: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t324: [uint32x4_t; 2] = [vmvnq_u32(t322[0]), vmvnq_u32(t322[1])];
    if any(&t322) {
        let t325 = b8_r0r1sl_p(l, lid0, t322, arg0, arg1, (x16).wrapping_add(1u32), x24);
        t323 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t322[0])))), t325[0], t323[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t322[0])))), t325[1], t323[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t322[1])))), t325[2], t323[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t322[1])))), t325[3], t323[3])];
    }
    {
        t323 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t324[0])))), x56[0], t323[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t324[0])))), x56[1], t323[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t324[1])))), x56[2], t323[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t324[1])))), x56[3], t323[3])];
    }
    res = t323;
    return res;
}

#[inline(always)]
unsafe fn b4_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t326 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    res = (((t326).wrapping_shr(((4u32).wrapping_mul(x16)) as u32)) & (15u32));
    return res;
}

#[inline(always)]
unsafe fn b4_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t327 = b9_r0r1s_f(l, lid0, arg0, arg1, 16u32);
    res = (((t327).wrapping_shr(((4u32).wrapping_mul(x16)) as u32)) & (15u32));
    return res;
}

#[inline(always)]
unsafe fn b5_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t328 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 40u32);
    let t329 = [vdupq_n_u32(t328); 2];
    let t330: [uint32x4_t; 2] = [vmulq_u32(x24[0], t329[0]), vmulq_u32(x24[1], t329[1])];
    let t331 = b10_r0r1s_p(l, lid0, live, arg0, arg1, 0u32);
    let t332 = [vdupq_n_u32(t331); 2];
    let t333: [uint32x4_t; 2] = [vaddq_u32(t330[0], t332[0]), vaddq_u32(t330[1], t332[1])];
    x32 = t333;
    if ((x16) == (0u32)) {
        res = x32;
        return res;
    }
    let t334 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    let t335: [uint32x4_t; 2] = [vshlq_u32(x24[0], vdupq_n_s32(((10u32) & 31) as i32)), vshlq_u32(x24[1], vdupq_n_s32(((10u32) & 31) as i32))];
    let t336 = [vdupq_n_u32(t334); 2];
    let t337: [uint32x4_t; 2] = [vsubq_u32(t336[0], t335[0]), vsubq_u32(t336[1], t335[1])];
    let t338 = b11_ls_p(l, lid0, live, t337, 1024u32);
    x40 = t338;
    let t339 = b10_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    let t340 = [vdupq_n_u32(t339); 2];
    let t341: [uint32x4_t; 2] = [vmulq_u32(x40[0], t340[0]), vmulq_u32(x40[1], t340[1])];
    let t342: [uint32x4_t; 2] = [vaddq_u32(x32[0], t341[0]), vaddq_u32(x32[1], t341[1])];
    res = t342;
    return res;
}

#[inline(always)]
unsafe fn b5_r0r1sl_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t343 = b9_r0r1s_f(l, lid0, arg0, arg1, 40u32);
    let t344 = [vdupq_n_u32(t343); 2];
    let t345: [uint32x4_t; 2] = [vmulq_u32(x24[0], t344[0]), vmulq_u32(x24[1], t344[1])];
    let t346 = b10_r0r1s_f(l, lid0, arg0, arg1, 0u32);
    let t347 = [vdupq_n_u32(t346); 2];
    let t348: [uint32x4_t; 2] = [vaddq_u32(t345[0], t347[0]), vaddq_u32(t345[1], t347[1])];
    x32 = t348;
    if ((x16) == (0u32)) {
        res = x32;
        return res;
    }
    let t349 = b9_r0r1s_f(l, lid0, arg0, arg1, 4u32);
    let t350: [uint32x4_t; 2] = [vshlq_u32(x24[0], vdupq_n_s32(((10u32) & 31) as i32)), vshlq_u32(x24[1], vdupq_n_s32(((10u32) & 31) as i32))];
    let t351 = [vdupq_n_u32(t349); 2];
    let t352: [uint32x4_t; 2] = [vsubq_u32(t351[0], t350[0]), vsubq_u32(t351[1], t350[1])];
    let t353 = b11_ls_f(l, lid0, t352, 1024u32);
    x40 = t353;
    let t354 = b10_r0r1s_f(l, lid0, arg0, arg1, x16);
    let t355 = [vdupq_n_u32(t354); 2];
    let t356: [uint32x4_t; 2] = [vmulq_u32(x40[0], t355[0]), vmulq_u32(x40[1], t355[1])];
    let t357: [uint32x4_t; 2] = [vaddq_u32(x32[0], t356[0]), vaddq_u32(x32[1], t356[1])];
    res = t357;
    return res;
}

#[inline(always)]
unsafe fn b6_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t358: u64;
    if ((x0) >= (8u32)) {
        t358 = !(0u64);
    } else {
        t358 = ((1u64).wrapping_shl(((((x0).wrapping_mul(8u32)) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t358;
    return res;
}

#[inline(always)]
unsafe fn b6_s_f(l: &Launch, lid0: u32, x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t359: u64;
    if ((x0) >= (8u32)) {
        t359 = !(0u64);
    } else {
        t359 = ((1u64).wrapping_shl(((((x0).wrapping_mul(8u32)) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t359;
    return res;
}

#[inline(always)]
unsafe fn b6_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t360 = [vdupq_n_u32(8u32); 2];
    let t361: [uint32x4_t; 2] = [vcgeq_u32(x0[0], t360[0]), vcgeq_u32(x0[1], t360[1])];
    let mut t362: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t363: [uint32x4_t; 2] = [vandq_u32(live[0], t361[0]), vandq_u32(live[1], t361[1])];
    let t364: [uint32x4_t; 2] = [vbicq_u32(live[0], t361[0]), vbicq_u32(live[1], t361[1])];
    {
        let t365 = [vdupq_n_u64(!(0u64)); 4];
        t362 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t363[0])))), t365[0], t362[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t363[0])))), t365[1], t362[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t363[1])))), t365[2], t362[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t363[1])))), t365[3], t362[3])];
    }
    {
        let t366: [uint32x4_t; 2] = [vmulq_u32(x0[0], t360[0]), vmulq_u32(x0[1], t360[1])];
        let t367 = [vdupq_n_u64(1u64); 4];
        let t368: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t366[0])), vmovl_u32(vget_high_u32(t366[0])), vmovl_u32(vget_low_u32(t366[1])), vmovl_u32(vget_high_u32(t366[1]))];
        let t369: [uint64x2_t; 4] = [vshlq_u64(t367[0], vreinterpretq_s64_u64(vandq_u64(t368[0], vdupq_n_u64(63)))), vshlq_u64(t367[1], vreinterpretq_s64_u64(vandq_u64(t368[1], vdupq_n_u64(63)))), vshlq_u64(t367[2], vreinterpretq_s64_u64(vandq_u64(t368[2], vdupq_n_u64(63)))), vshlq_u64(t367[3], vreinterpretq_s64_u64(vandq_u64(t368[3], vdupq_n_u64(63))))];
        let t370: [uint64x2_t; 4] = [vsubq_u64(t369[0], t367[0]), vsubq_u64(t369[1], t367[1]), vsubq_u64(t369[2], t367[2]), vsubq_u64(t369[3], t367[3])];
        t362 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t364[0])))), t370[0], t362[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t364[0])))), t370[1], t362[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t364[1])))), t370[2], t362[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t364[1])))), t370[3], t362[3])];
    }
    res = t362;
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2], x24: [uint32x4_t; 2], x32: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t371 = b12_r0lls_p(l, lid0, live, arg0, x16, x24, x32);
    res = t371;
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1lls_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2], x24: [uint32x4_t; 2], x32: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t372 = b12_r0lls_f(l, lid0, arg0, x16, x24, x32);
    res = t372;
    return res;
}

#[inline(always)]
unsafe fn b8_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: u32 = 0;
    let mut x48: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x56: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x64: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x72: u32 = 0;
    let mut x80: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x88: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x96: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x104: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x112: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x120: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x128: u64 = 0;
    let mut x136: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut x144: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x152: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x160: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut x168: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut x176: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut x184: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t373 = b9_r0r1s_p(l, lid0, live, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x16)));
    x32 = t373;
    let t374 = b9_r0r1s_p(l, lid0, live, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x16)));
    x40 = t374;
    let t375: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    x48 = t375;
    let t376 = [vdupq_n_u32(4u32); 2];
    let t377: [uint32x4_t; 2] = [vmulq_u32(x48[0], t376[0]), vmulq_u32(x48[1], t376[1])];
    let t378 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x40, t377, 4u32);
    let t379: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t378[0]), vmovn_u64(t378[1])), vcombine_u32(vmovn_u64(t378[2]), vmovn_u64(t378[3]))];
    x56 = t379;
    let t380: [uint32x4_t; 2] = [vmulq_u32(x48[0], t376[0]), vmulq_u32(x48[1], t376[1])];
    let t381: [uint32x4_t; 2] = [vaddq_u32(t380[0], t376[0]), vaddq_u32(t380[1], t376[1])];
    let t382 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x40, t381, 4u32);
    let t383: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t382[0]), vmovn_u64(t382[1])), vcombine_u32(vmovn_u64(t382[2]), vmovn_u64(t382[3]))];
    x64 = t383;
    let t384 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    x72 = ((t384).wrapping_add((256u32).wrapping_sub(1u32))).wrapping_shr((8u32) as u32);
    let t385: [uint32x4_t; 2] = [vmulq_u32(x48[0], t376[0]), vmulq_u32(x48[1], t376[1])];
    let t386 = [vdupq_n_u32(x72); 2];
    let t387: [uint32x4_t; 2] = [vsubq_u32(t386[0], t385[0]), vsubq_u32(t386[1], t385[1])];
    let t388 = b14_sl_p(l, lid0, live, 4u32, t387);
    x80 = t388;
    let t389: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
    let t390 = [vdupq_n_u32((4u32).wrapping_sub(1u32)); 2];
    let t391: [uint32x4_t; 2] = [vandq_u32(t389[0], t390[0]), vandq_u32(t389[1], t390[1])];
    x88 = t391;
    let t392 = [vdupq_n_u32(0u32); 2];
    let t393: [uint32x4_t; 2] = [vceqq_u32(x88[0], t392[0]), vceqq_u32(x88[1], t392[1])];
    let mut t394: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t395: [uint32x4_t; 2] = [vandq_u32(live[0], t393[0]), vandq_u32(live[1], t393[1])];
    let t396: [uint32x4_t; 2] = [vbicq_u32(live[0], t393[0]), vbicq_u32(live[1], t393[1])];
    {
        t394 = [vbslq_u32(t395[0], t392[0], t394[0]), vbslq_u32(t395[1], t392[1], t394[1])];
    }
    if any(&t396) {
        let t397 = [vdupq_n_u32(1u32); 2];
        let t398: [uint32x4_t; 2] = [vsubq_u32(x88[0], t397[0]), vsubq_u32(x88[1], t397[1])];
        let t399 = [vdupq_n_u32(2u32); 2];
        let t400: [uint32x4_t; 2] = [vmulq_u32(t399[0], t398[0]), vmulq_u32(t399[1], t398[1])];
        let t401: [uint32x4_t; 2] = [vaddq_u32(x56[0], t400[0]), vaddq_u32(x56[1], t400[1])];
        let t402 = b15_ll_p(l, lid0, t396, t401, x64);
        let t403 = b13_r0r1sls_p(l, lid0, t396, arg0, arg1, x32, t402, 2u32);
        let t404: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t403[0]), vmovn_u64(t403[1])), vcombine_u32(vmovn_u64(t403[2]), vmovn_u64(t403[3]))];
        t394 = [vbslq_u32(t396[0], t404[0], t394[0]), vbslq_u32(t396[1], t404[1], t394[1])];
    }
    x96 = t394;
    let t405 = [vdupq_n_u32(2u32); 2];
    let t406: [uint32x4_t; 2] = [vmulq_u32(t405[0], x88[0]), vmulq_u32(t405[1], x88[1])];
    let t407: [uint32x4_t; 2] = [vaddq_u32(x56[0], t406[0]), vaddq_u32(x56[1], t406[1])];
    let t408 = b15_ll_p(l, lid0, live, t407, x64);
    let t409 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t408, 2u32);
    let t410: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t409[0]), vmovn_u64(t409[1])), vcombine_u32(vmovn_u64(t409[2]), vmovn_u64(t409[3]))];
    x104 = t410;
    let t411: [uint32x4_t; 2] = [vmulq_u32(t405[0], x80[0]), vmulq_u32(t405[1], x80[1])];
    let t412: [uint32x4_t; 2] = [vaddq_u32(x56[0], t411[0]), vaddq_u32(x56[1], t411[1])];
    x112 = t412;
    let t413 = [vdupq_n_u32((256u32).wrapping_sub(1u32)); 2];
    let t414: [uint32x4_t; 2] = [vandq_u32(x24[0], t413[0]), vandq_u32(x24[1], t413[1])];
    x120 = t414;
    x128 = 9259542123273814144u64;
    let t415: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x120[0])), vmovl_u32(vget_high_u32(x120[0])), vmovl_u32(vget_low_u32(x120[1])), vmovl_u32(vget_high_u32(x120[1]))];
    let t416 = [vdupq_n_u64(72340172838076673u64); 4];
    let mut t417: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    t417[0] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t415[0]) as u64).wrapping_mul(vgetq_lane_u64::<0>(t416[0]) as u64) as u64, t417[0]);
    t417[0] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t415[0]) as u64).wrapping_mul(vgetq_lane_u64::<1>(t416[0]) as u64) as u64, t417[0]);
    t417[1] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t415[1]) as u64).wrapping_mul(vgetq_lane_u64::<0>(t416[1]) as u64) as u64, t417[1]);
    t417[1] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t415[1]) as u64).wrapping_mul(vgetq_lane_u64::<1>(t416[1]) as u64) as u64, t417[1]);
    t417[2] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t415[2]) as u64).wrapping_mul(vgetq_lane_u64::<0>(t416[2]) as u64) as u64, t417[2]);
    t417[2] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t415[2]) as u64).wrapping_mul(vgetq_lane_u64::<1>(t416[2]) as u64) as u64, t417[2]);
    t417[3] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t415[3]) as u64).wrapping_mul(vgetq_lane_u64::<0>(t416[3]) as u64) as u64, t417[3]);
    t417[3] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t415[3]) as u64).wrapping_mul(vgetq_lane_u64::<1>(t416[3]) as u64) as u64, t417[3]);
    x136 = t417;
    x144 = x96;
    x152 = x96;
    let mut t418 = live;
    loop {
        // Lanes part: each its own rounds.
        let t420: [uint32x4_t; 2] = [vcltq_u32(x152[0], x104[0]), vcltq_u32(x152[1], x104[1])];
        t418 = [vandq_u32(t418[0], t420[0]), vandq_u32(t418[1], t420[1])];
        if !(any(&t418)) { break; }
        let t422: [uint32x4_t; 2] = [vaddq_u32(x112[0], x152[0]), vaddq_u32(x112[1], x152[1])];
        let t423 = b15_ll_p(l, lid0, t418, t422, x64);
        let t424 = b13_r0r1sls_p(l, lid0, t418, arg0, arg1, x32, t423, 8u32);
        x160 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t418[0])))), t424[0], x160[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t418[0])))), t424[1], x160[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t418[1])))), t424[2], x160[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t418[1])))), t424[3], x160[3])];
        let t425 = [vdupq_n_u64(x128); 4];
        let t426: [uint64x2_t; 4] = [vorrq_u64(x160[0], t425[0]), vorrq_u64(x160[1], t425[1]), vorrq_u64(x160[2], t425[2]), vorrq_u64(x160[3], t425[3])];
        let t427 = [vdupq_n_u64(!(x128)); 4];
        let t428: [uint64x2_t; 4] = [vandq_u64(x136[0], t427[0]), vandq_u64(x136[1], t427[1]), vandq_u64(x136[2], t427[2]), vandq_u64(x136[3], t427[3])];
        let t429: [uint64x2_t; 4] = [vsubq_u64(t426[0], t428[0]), vsubq_u64(t426[1], t428[1]), vsubq_u64(t426[2], t428[2]), vsubq_u64(t426[3], t428[3])];
        x168 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t418[0])))), t429[0], x168[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t418[0])))), t429[1], x168[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t418[1])))), t429[2], x168[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t418[1])))), t429[3], x168[3])];
        let t430: [uint64x2_t; 4] = [vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x160[0]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x160[1]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x160[2]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x160[3])))];
        let t431: [uint64x2_t; 4] = [vandq_u64(t430[0], x136[0]), vandq_u64(t430[1], x136[1]), vandq_u64(t430[2], x136[2]), vandq_u64(t430[3], x136[3])];
        let t432: [uint64x2_t; 4] = [veorq_u64(x160[0], x136[0]), veorq_u64(x160[1], x136[1]), veorq_u64(x160[2], x136[2]), veorq_u64(x160[3], x136[3])];
        let t433: [uint64x2_t; 4] = [vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(t432[0]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(t432[1]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(t432[2]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(t432[3])))];
        let t434: [uint64x2_t; 4] = [vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x168[0]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x168[1]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x168[2]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x168[3])))];
        let t435: [uint64x2_t; 4] = [vandq_u64(t433[0], t434[0]), vandq_u64(t433[1], t434[1]), vandq_u64(t433[2], t434[2]), vandq_u64(t433[3], t434[3])];
        let t436: [uint64x2_t; 4] = [vorrq_u64(t431[0], t435[0]), vorrq_u64(t431[1], t435[1]), vorrq_u64(t431[2], t435[2]), vorrq_u64(t431[3], t435[3])];
        let t437 = [vdupq_n_u64(x128); 4];
        let t438: [uint64x2_t; 4] = [vandq_u64(t437[0], t436[0]), vandq_u64(t437[1], t436[1]), vandq_u64(t437[2], t436[2]), vandq_u64(t437[3], t436[3])];
        x176 = t438;
        let t439: [uint32x4_t; 2] = [vsubq_u32(x104[0], x152[0]), vsubq_u32(x104[1], x152[1])];
        x184 = t439;
        let t440 = [vdupq_n_u32(8u32); 2];
        let t441: [uint32x4_t; 2] = [vcgeq_u32(x184[0], t440[0]), vcgeq_u32(x184[1], t440[1])];
        let mut t442: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
        let t443: [uint32x4_t; 2] = [vandq_u32(t418[0], t441[0]), vandq_u32(t418[1], t441[1])];
        let t444: [uint32x4_t; 2] = [vbicq_u32(t418[0], t441[0]), vbicq_u32(t418[1], t441[1])];
        {
            t442 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t443[0])))), x176[0], t442[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t443[0])))), x176[1], t442[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t443[1])))), x176[2], t442[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t443[1])))), x176[3], t442[3])];
        }
        {
            let t445: [uint32x4_t; 2] = [vmulq_u32(t440[0], x184[0]), vmulq_u32(t440[1], x184[1])];
            let t446 = [vdupq_n_u64(1u64); 4];
            let t447: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t445[0])), vmovl_u32(vget_high_u32(t445[0])), vmovl_u32(vget_low_u32(t445[1])), vmovl_u32(vget_high_u32(t445[1]))];
            let t448: [uint64x2_t; 4] = [vshlq_u64(t446[0], vreinterpretq_s64_u64(vandq_u64(t447[0], vdupq_n_u64(63)))), vshlq_u64(t446[1], vreinterpretq_s64_u64(vandq_u64(t447[1], vdupq_n_u64(63)))), vshlq_u64(t446[2], vreinterpretq_s64_u64(vandq_u64(t447[2], vdupq_n_u64(63)))), vshlq_u64(t446[3], vreinterpretq_s64_u64(vandq_u64(t447[3], vdupq_n_u64(63))))];
            let t449: [uint64x2_t; 4] = [vsubq_u64(t448[0], t446[0]), vsubq_u64(t448[1], t446[1]), vsubq_u64(t448[2], t446[2]), vsubq_u64(t448[3], t446[3])];
            let t450: [uint64x2_t; 4] = [vandq_u64(x176[0], t449[0]), vandq_u64(x176[1], t449[1]), vandq_u64(x176[2], t449[2]), vandq_u64(x176[3], t449[3])];
            t442 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t444[0])))), t450[0], t442[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t444[0])))), t450[1], t442[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t444[1])))), t450[2], t442[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t444[1])))), t450[3], t442[3])];
        }
        let t451 = b16_l_p(l, lid0, t418, t442);
        let t452: [uint32x4_t; 2] = [vaddq_u32(x144[0], t451[0]), vaddq_u32(x144[1], t451[1])];
        x144 = [vbslq_u32(t418[0], t452[0], x144[0]), vbslq_u32(t418[1], t452[1], x144[1])];
        let t453: [uint32x4_t; 2] = [vandq_u32(t418[0], live[0]), vandq_u32(t418[1], live[1])];
        x152 = [vaddq_u32(x152[0], vandq_u32(t453[0], vdupq_n_u32(8))), vaddq_u32(x152[1], vandq_u32(t453[1], vdupq_n_u32(8)))];
    }
    x96 = x144;
    let t454 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    let t455 = [vdupq_n_u32((((t454).wrapping_shr((((((4u32).wrapping_mul(x16)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32))); 2];
    x152 = t455;
    let t456 = [vdupq_n_u32(1u32); 2];
    let t457: [uint32x4_t; 2] = [vsubq_u32(x80[0], t456[0]), vsubq_u32(x80[1], t456[1])];
    let t458: [uint32x4_t; 2] = [vmulq_u32(t405[0], t457[0]), vmulq_u32(t405[1], t457[1])];
    let t459: [uint32x4_t; 2] = [vaddq_u32(x56[0], t458[0]), vaddq_u32(x56[1], t458[1])];
    let t460 = b15_ll_p(l, lid0, live, t459, x64);
    let t461 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t460, 2u32);
    let t462: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t461[0]), vmovn_u64(t461[1])), vcombine_u32(vmovn_u64(t461[2]), vmovn_u64(t461[3]))];
    let t463: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t462[0])), vmovl_u32(vget_high_u32(t462[0])), vmovl_u32(vget_low_u32(t462[1])), vmovl_u32(vget_high_u32(t462[1]))];
    x160 = t463;
    let t464: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x160[0]), vmovn_u64(x160[1])), vcombine_u32(vmovn_u64(x160[2]), vmovn_u64(x160[3]))];
    let t465: [uint32x4_t; 2] = [vaddq_u32(x112[0], t464[0]), vaddq_u32(x112[1], t464[1])];
    let t466: [uint32x4_t; 2] = [vmulq_u32(x96[0], x152[0]), vmulq_u32(x96[1], x152[1])];
    let t467: [uint32x4_t; 2] = [vaddq_u32(t465[0], t466[0]), vaddq_u32(t465[1], t466[1])];
    let t468 = b15_ll_p(l, lid0, live, t467, x64);
    let t469: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t468[0])), vmovl_u32(vget_high_u32(t468[0])), vmovl_u32(vget_low_u32(t468[1])), vmovl_u32(vget_high_u32(t468[1]))];
    x168 = t469;
    let t470: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x168[0]), vmovn_u64(x168[1])), vcombine_u32(vmovn_u64(x168[2]), vmovn_u64(x168[3]))];
    let t471 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t470, 8u32);
    let t472 = b6_l_p(l, lid0, live, x152);
    let t473: [uint64x2_t; 4] = [vandq_u64(t471[0], t472[0]), vandq_u64(t471[1], t472[1]), vandq_u64(t471[2], t472[2]), vandq_u64(t471[3], t472[3])];
    res = t473;
    return res;
}

#[inline(always)]
unsafe fn b9_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    res = (arg1.wrapping_add(((((x16).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned();
    return res;
}

#[inline(always)]
unsafe fn b9_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    res = (arg1.wrapping_add(((((x16).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned();
    return res;
}

#[inline(always)]
unsafe fn b10_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t474 = b9_r0r1s_p(l, lid0, live, arg0, arg1, (20u32).wrapping_add((4u32).wrapping_mul(x16)));
    res = t474;
    return res;
}

#[inline(always)]
unsafe fn b10_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t475 = b9_r0r1s_f(l, lid0, arg0, arg1, (20u32).wrapping_add((4u32).wrapping_mul(x16)));
    res = t475;
    return res;
}

#[inline(always)]
unsafe fn b11_ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x8: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t476 = [vdupq_n_u32(x8); 2];
    let t477: [uint32x4_t; 2] = [vcltq_u32(x0[0], t476[0]), vcltq_u32(x0[1], t476[1])];
    let mut t478: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t479: [uint32x4_t; 2] = [vandq_u32(live[0], t477[0]), vandq_u32(live[1], t477[1])];
    let t480: [uint32x4_t; 2] = [vbicq_u32(live[0], t477[0]), vbicq_u32(live[1], t477[1])];
    {
        t478 = [vbslq_u32(t479[0], x0[0], t478[0]), vbslq_u32(t479[1], x0[1], t478[1])];
    }
    {
        let t481 = [vdupq_n_u32(x8); 2];
        t478 = [vbslq_u32(t480[0], t481[0], t478[0]), vbslq_u32(t480[1], t481[1], t478[1])];
    }
    res = t478;
    return res;
}

#[inline(always)]
unsafe fn b11_ls_f(l: &Launch, lid0: u32, x0: [uint32x4_t; 2], x8: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t482 = [vdupq_n_u32(x8); 2];
    let t483: [uint32x4_t; 2] = [vcltq_u32(x0[0], t482[0]), vcltq_u32(x0[1], t482[1])];
    let mut t484: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t485: [uint32x4_t; 2] = [vmvnq_u32(t483[0]), vmvnq_u32(t483[1])];
    {
        t484 = [vbslq_u32(t483[0], x0[0], t484[0]), vbslq_u32(t483[1], x0[1], t484[1])];
    }
    {
        let t486 = [vdupq_n_u32(x8); 2];
        t484 = [vbslq_u32(t485[0], t486[0], t484[0]), vbslq_u32(t485[1], t486[1], t484[1])];
    }
    res = t484;
    return res;
}

#[inline(always)]
unsafe fn b12_r0lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: [uint32x4_t; 2], x16: [uint32x4_t; 2], x24: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t487: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x8[0])), vmovl_u32(vget_high_u32(x8[0])), vmovl_u32(vget_low_u32(x8[1])), vmovl_u32(vget_high_u32(x8[1]))];
    let t488: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x16[0])), vmovl_u32(vget_high_u32(x16[0])), vmovl_u32(vget_low_u32(x16[1])), vmovl_u32(vget_high_u32(x16[1]))];
    let t489: [uint64x2_t; 4] = [vaddq_u64(t487[0], t488[0]), vaddq_u64(t487[1], t488[1]), vaddq_u64(t487[2], t488[2]), vaddq_u64(t487[3], t488[3])];
    let t490 = b17_r0ls_p(l, lid0, live, arg0, t489, x24);
    x32 = t490;
    let t491: [uint64x2_t; 4];
    if ((x24) >= (8u32)) {
        t491 = x32;
    } else {
        let t492 = b18_s_p(l, lid0, live, (8u32).wrapping_mul(x24));
        let t493 = [vdupq_n_u64(t492); 4];
        let t494: [uint64x2_t; 4] = [vandq_u64(x32[0], t493[0]), vandq_u64(x32[1], t493[1]), vandq_u64(x32[2], t493[2]), vandq_u64(x32[3], t493[3])];
        t491 = t494;
    }
    res = t491;
    return res;
}

#[inline(always)]
unsafe fn b12_r0lls_f(l: &Launch, lid0: u32, arg0: *const u8, x8: [uint32x4_t; 2], x16: [uint32x4_t; 2], x24: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t495: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x8[0])), vmovl_u32(vget_high_u32(x8[0])), vmovl_u32(vget_low_u32(x8[1])), vmovl_u32(vget_high_u32(x8[1]))];
    let t496: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x16[0])), vmovl_u32(vget_high_u32(x16[0])), vmovl_u32(vget_low_u32(x16[1])), vmovl_u32(vget_high_u32(x16[1]))];
    let t497: [uint64x2_t; 4] = [vaddq_u64(t495[0], t496[0]), vaddq_u64(t495[1], t496[1]), vaddq_u64(t495[2], t496[2]), vaddq_u64(t495[3], t496[3])];
    let t498 = b17_r0ls_f(l, lid0, arg0, t497, x24);
    x32 = t498;
    let t499: [uint64x2_t; 4];
    if ((x24) >= (8u32)) {
        t499 = x32;
    } else {
        let t500 = b18_s_f(l, lid0, (8u32).wrapping_mul(x24));
        let t501 = [vdupq_n_u64(t500); 4];
        let t502: [uint64x2_t; 4] = [vandq_u64(x32[0], t501[0]), vandq_u64(x32[1], t501[1]), vandq_u64(x32[2], t501[2]), vandq_u64(x32[3], t501[3])];
        t499 = t502;
    }
    res = t499;
    return res;
}

#[inline(always)]
unsafe fn b13_r0r1sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2], x32: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t503 = b19_r0sls_p(l, lid0, live, arg0, x16, x24, x32);
    res = t503;
    return res;
}

#[inline(always)]
unsafe fn b14_sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x8: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t504 = [vdupq_n_u32(x0); 2];
    let t505: [uint32x4_t; 2] = [vcltq_u32(t504[0], x8[0]), vcltq_u32(t504[1], x8[1])];
    let mut t506: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t507: [uint32x4_t; 2] = [vandq_u32(live[0], t505[0]), vandq_u32(live[1], t505[1])];
    let t508: [uint32x4_t; 2] = [vbicq_u32(live[0], t505[0]), vbicq_u32(live[1], t505[1])];
    {
        let t509 = [vdupq_n_u32(x0); 2];
        t506 = [vbslq_u32(t507[0], t509[0], t506[0]), vbslq_u32(t507[1], t509[1], t506[1])];
    }
    {
        t506 = [vbslq_u32(t508[0], x8[0], t506[0]), vbslq_u32(t508[1], x8[1], t506[1])];
    }
    res = t506;
    return res;
}

#[inline(always)]
unsafe fn b15_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x8: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t510: [uint32x4_t; 2] = [vcltq_u32(x0[0], x8[0]), vcltq_u32(x0[1], x8[1])];
    let mut t511: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t512: [uint32x4_t; 2] = [vandq_u32(live[0], t510[0]), vandq_u32(live[1], t510[1])];
    let t513: [uint32x4_t; 2] = [vbicq_u32(live[0], t510[0]), vbicq_u32(live[1], t510[1])];
    {
        t511 = [vbslq_u32(t512[0], x0[0], t511[0]), vbslq_u32(t512[1], x0[1], t511[1])];
    }
    {
        t511 = [vbslq_u32(t513[0], x8[0], t511[0]), vbslq_u32(t513[1], x8[1], t511[1])];
    }
    res = t511;
    return res;
}

#[inline(always)]
unsafe fn b16_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint64x2_t; 4]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t514: [uint64x2_t; 4] = [vpaddlq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u64(x0[0]))))), vpaddlq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u64(x0[1]))))), vpaddlq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u64(x0[2]))))), vpaddlq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u64(x0[3])))))];
    let t515: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t514[0]), vmovn_u64(t514[1])), vcombine_u32(vmovn_u64(t514[2]), vmovn_u64(t514[3]))];
    res = t515;
    return res;
}

#[inline(always)]
unsafe fn b17_r0ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: [uint64x2_t; 4], x16: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t516 = [vdupq_n_u64(!(7u64)); 4];
    let t517: [uint64x2_t; 4] = [vandq_u64(x8[0], t516[0]), vandq_u64(x8[1], t516[1]), vandq_u64(x8[2], t516[2]), vandq_u64(x8[3], t516[3])];
    let t518: [uint64x2_t; 4] = [vandq_u64(t517[0], vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0]))))), vandq_u64(t517[1], vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0]))))), vandq_u64(t517[2], vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1]))))), vandq_u64(t517[3], vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))))];
    let t519: [uint64x2_t; 4] = [vaddq_u64(vdupq_n_u64((arg0) as u64), t518[0]), vaddq_u64(vdupq_n_u64((arg0) as u64), t518[1]), vaddq_u64(vdupq_n_u64((arg0) as u64), t518[2]), vaddq_u64(vdupq_n_u64((arg0) as u64), t518[3])];
    x24 = t519;
    let t520 = [vdupq_n_u64(7u64); 4];
    let t521: [uint64x2_t; 4] = [vandq_u64(x8[0], t520[0]), vandq_u64(x8[1], t520[1]), vandq_u64(x8[2], t520[2]), vandq_u64(x8[3], t520[3])];
    let t522: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t521[0]), vmovn_u64(t521[1])), vcombine_u32(vmovn_u64(t521[2]), vmovn_u64(t521[3]))];
    let t523 = [vdupq_n_u32(8u32); 2];
    let t524: [uint32x4_t; 2] = [vmulq_u32(t522[0], t523[0]), vmulq_u32(t522[1], t523[1])];
    x32 = t524;
    let mut t525: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    t525[0] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[0]) as usize as *const u64).read_unaligned(), t525[0]);
    t525[0] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[0]) as usize as *const u64).read_unaligned(), t525[0]);
    t525[1] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[1]) as usize as *const u64).read_unaligned(), t525[1]);
    t525[1] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[1]) as usize as *const u64).read_unaligned(), t525[1]);
    t525[2] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[2]) as usize as *const u64).read_unaligned(), t525[2]);
    t525[2] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[2]) as usize as *const u64).read_unaligned(), t525[2]);
    t525[3] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[3]) as usize as *const u64).read_unaligned(), t525[3]);
    t525[3] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[3]) as usize as *const u64).read_unaligned(), t525[3]);
    let t526: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x32[0])), vmovl_u32(vget_high_u32(x32[0])), vmovl_u32(vget_low_u32(x32[1])), vmovl_u32(vget_high_u32(x32[1]))];
    let t527: [uint64x2_t; 4] = [vshlq_u64(t525[0], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t526[0], vdupq_n_u64(63))))), vshlq_u64(t525[1], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t526[1], vdupq_n_u64(63))))), vshlq_u64(t525[2], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t526[2], vdupq_n_u64(63))))), vshlq_u64(t525[3], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t526[3], vdupq_n_u64(63)))))];
    x40 = t527;
    let t528 = [vdupq_n_u32(0u32); 2];
    let t529: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t528[0])), vmvnq_u32(vceqq_u32(x32[1], t528[1]))];
    let t530 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t531: [uint32x4_t; 2] = [vaddq_u32(x32[0], t530[0]), vaddq_u32(x32[1], t530[1])];
    let t532 = [vdupq_n_u32(64u32); 2];
    let t533: [uint32x4_t; 2] = [vcgtq_u32(t531[0], t532[0]), vcgtq_u32(t531[1], t532[1])];
    let t534: [uint32x4_t; 2] = [vandq_u32(t529[0], t533[0]), vandq_u32(t529[1], t533[1])];
    let mut t535: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t536: [uint32x4_t; 2] = [vandq_u32(live[0], t534[0]), vandq_u32(live[1], t534[1])];
    let t537: [uint32x4_t; 2] = [vbicq_u32(live[0], t534[0]), vbicq_u32(live[1], t534[1])];
    if any(&t536) {
        let t538 = [vdupq_n_u64((1u64).wrapping_mul(8u64)); 4];
        let t539: [uint64x2_t; 4] = [vaddq_u64(x24[0], t538[0]), vaddq_u64(x24[1], t538[1]), vaddq_u64(x24[2], t538[2]), vaddq_u64(x24[3], t538[3])];
        let mut t540: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
        t540[0] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t539[0]) as usize as *const u64).read_unaligned(), t540[0]);
        t540[0] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t539[0]) as usize as *const u64).read_unaligned(), t540[0]);
        t540[1] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t539[1]) as usize as *const u64).read_unaligned(), t540[1]);
        t540[1] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t539[1]) as usize as *const u64).read_unaligned(), t540[1]);
        t540[2] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t539[2]) as usize as *const u64).read_unaligned(), t540[2]);
        t540[2] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t539[2]) as usize as *const u64).read_unaligned(), t540[2]);
        t540[3] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t539[3]) as usize as *const u64).read_unaligned(), t540[3]);
        t540[3] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t539[3]) as usize as *const u64).read_unaligned(), t540[3]);
        let t541: [uint32x4_t; 2] = [vsubq_u32(t532[0], x32[0]), vsubq_u32(t532[1], x32[1])];
        let t542: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t541[0])), vmovl_u32(vget_high_u32(t541[0])), vmovl_u32(vget_low_u32(t541[1])), vmovl_u32(vget_high_u32(t541[1]))];
        let t543: [uint64x2_t; 4] = [vshlq_u64(t540[0], vreinterpretq_s64_u64(vandq_u64(t542[0], vdupq_n_u64(63)))), vshlq_u64(t540[1], vreinterpretq_s64_u64(vandq_u64(t542[1], vdupq_n_u64(63)))), vshlq_u64(t540[2], vreinterpretq_s64_u64(vandq_u64(t542[2], vdupq_n_u64(63)))), vshlq_u64(t540[3], vreinterpretq_s64_u64(vandq_u64(t542[3], vdupq_n_u64(63))))];
        let t544: [uint64x2_t; 4] = [vorrq_u64(x40[0], t543[0]), vorrq_u64(x40[1], t543[1]), vorrq_u64(x40[2], t543[2]), vorrq_u64(x40[3], t543[3])];
        t535 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t536[0])))), t544[0], t535[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t536[0])))), t544[1], t535[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t536[1])))), t544[2], t535[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t536[1])))), t544[3], t535[3])];
    }
    {
        t535 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t537[0])))), x40[0], t535[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t537[0])))), x40[1], t535[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t537[1])))), x40[2], t535[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t537[1])))), x40[3], t535[3])];
    }
    res = t535;
    return res;
}

#[inline(always)]
unsafe fn b17_r0ls_f(l: &Launch, lid0: u32, arg0: *const u8, x8: [uint64x2_t; 4], x16: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t545 = [vdupq_n_u64(!(7u64)); 4];
    let t546: [uint64x2_t; 4] = [vandq_u64(x8[0], t545[0]), vandq_u64(x8[1], t545[1]), vandq_u64(x8[2], t545[2]), vandq_u64(x8[3], t545[3])];
    let t547: [uint64x2_t; 4] = [vaddq_u64(vdupq_n_u64((arg0) as u64), t546[0]), vaddq_u64(vdupq_n_u64((arg0) as u64), t546[1]), vaddq_u64(vdupq_n_u64((arg0) as u64), t546[2]), vaddq_u64(vdupq_n_u64((arg0) as u64), t546[3])];
    x24 = t547;
    let t548 = [vdupq_n_u64(7u64); 4];
    let t549: [uint64x2_t; 4] = [vandq_u64(x8[0], t548[0]), vandq_u64(x8[1], t548[1]), vandq_u64(x8[2], t548[2]), vandq_u64(x8[3], t548[3])];
    let t550: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t549[0]), vmovn_u64(t549[1])), vcombine_u32(vmovn_u64(t549[2]), vmovn_u64(t549[3]))];
    let t551 = [vdupq_n_u32(8u32); 2];
    let t552: [uint32x4_t; 2] = [vmulq_u32(t550[0], t551[0]), vmulq_u32(t550[1], t551[1])];
    x32 = t552;
    let mut t553: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    t553[0] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[0]) as usize as *const u64).read_unaligned(), t553[0]);
    t553[0] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[0]) as usize as *const u64).read_unaligned(), t553[0]);
    t553[1] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[1]) as usize as *const u64).read_unaligned(), t553[1]);
    t553[1] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[1]) as usize as *const u64).read_unaligned(), t553[1]);
    t553[2] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[2]) as usize as *const u64).read_unaligned(), t553[2]);
    t553[2] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[2]) as usize as *const u64).read_unaligned(), t553[2]);
    t553[3] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[3]) as usize as *const u64).read_unaligned(), t553[3]);
    t553[3] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[3]) as usize as *const u64).read_unaligned(), t553[3]);
    let t554: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x32[0])), vmovl_u32(vget_high_u32(x32[0])), vmovl_u32(vget_low_u32(x32[1])), vmovl_u32(vget_high_u32(x32[1]))];
    let t555: [uint64x2_t; 4] = [vshlq_u64(t553[0], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t554[0], vdupq_n_u64(63))))), vshlq_u64(t553[1], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t554[1], vdupq_n_u64(63))))), vshlq_u64(t553[2], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t554[2], vdupq_n_u64(63))))), vshlq_u64(t553[3], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t554[3], vdupq_n_u64(63)))))];
    x40 = t555;
    let t556 = [vdupq_n_u32(0u32); 2];
    let t557: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t556[0])), vmvnq_u32(vceqq_u32(x32[1], t556[1]))];
    let t558 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t559: [uint32x4_t; 2] = [vaddq_u32(x32[0], t558[0]), vaddq_u32(x32[1], t558[1])];
    let t560 = [vdupq_n_u32(64u32); 2];
    let t561: [uint32x4_t; 2] = [vcgtq_u32(t559[0], t560[0]), vcgtq_u32(t559[1], t560[1])];
    let t562: [uint32x4_t; 2] = [vandq_u32(t557[0], t561[0]), vandq_u32(t557[1], t561[1])];
    let mut t563: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t564: [uint32x4_t; 2] = [vmvnq_u32(t562[0]), vmvnq_u32(t562[1])];
    if any(&t562) {
        let t565 = [vdupq_n_u64((1u64).wrapping_mul(8u64)); 4];
        let t566: [uint64x2_t; 4] = [vaddq_u64(x24[0], t565[0]), vaddq_u64(x24[1], t565[1]), vaddq_u64(x24[2], t565[2]), vaddq_u64(x24[3], t565[3])];
        let mut t567: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
        t567[0] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t566[0]) as usize as *const u64).read_unaligned(), t567[0]);
        t567[0] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t566[0]) as usize as *const u64).read_unaligned(), t567[0]);
        t567[1] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t566[1]) as usize as *const u64).read_unaligned(), t567[1]);
        t567[1] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t566[1]) as usize as *const u64).read_unaligned(), t567[1]);
        t567[2] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t566[2]) as usize as *const u64).read_unaligned(), t567[2]);
        t567[2] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t566[2]) as usize as *const u64).read_unaligned(), t567[2]);
        t567[3] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t566[3]) as usize as *const u64).read_unaligned(), t567[3]);
        t567[3] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t566[3]) as usize as *const u64).read_unaligned(), t567[3]);
        let t568: [uint32x4_t; 2] = [vsubq_u32(t560[0], x32[0]), vsubq_u32(t560[1], x32[1])];
        let t569: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t568[0])), vmovl_u32(vget_high_u32(t568[0])), vmovl_u32(vget_low_u32(t568[1])), vmovl_u32(vget_high_u32(t568[1]))];
        let t570: [uint64x2_t; 4] = [vshlq_u64(t567[0], vreinterpretq_s64_u64(vandq_u64(t569[0], vdupq_n_u64(63)))), vshlq_u64(t567[1], vreinterpretq_s64_u64(vandq_u64(t569[1], vdupq_n_u64(63)))), vshlq_u64(t567[2], vreinterpretq_s64_u64(vandq_u64(t569[2], vdupq_n_u64(63)))), vshlq_u64(t567[3], vreinterpretq_s64_u64(vandq_u64(t569[3], vdupq_n_u64(63))))];
        let t571: [uint64x2_t; 4] = [vorrq_u64(x40[0], t570[0]), vorrq_u64(x40[1], t570[1]), vorrq_u64(x40[2], t570[2]), vorrq_u64(x40[3], t570[3])];
        t563 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t562[0])))), t571[0], t563[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t562[0])))), t571[1], t563[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t562[1])))), t571[2], t563[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t562[1])))), t571[3], t563[3])];
    }
    {
        t563 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t564[0])))), x40[0], t563[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t564[0])))), x40[1], t563[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t564[1])))), x40[2], t563[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t564[1])))), x40[3], t563[3])];
    }
    res = t563;
    return res;
}

#[inline(always)]
unsafe fn b18_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t572: u64;
    if ((x0) >= (64u32)) {
        t572 = !(0u64);
    } else {
        t572 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t572;
    return res;
}

#[inline(always)]
unsafe fn b18_s_f(l: &Launch, lid0: u32, x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t573: u64;
    if ((x0) >= (64u32)) {
        t573 = !(0u64);
    } else {
        t573 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t573;
    return res;
}

#[inline(always)]
unsafe fn b19_r0sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: u32, x16: [uint32x4_t; 2], x24: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t574 = [vdupq_n_u64(((x8) as u64)); 4];
    let t575: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x16[0])), vmovl_u32(vget_high_u32(x16[0])), vmovl_u32(vget_low_u32(x16[1])), vmovl_u32(vget_high_u32(x16[1]))];
    let t576: [uint64x2_t; 4] = [vaddq_u64(t574[0], t575[0]), vaddq_u64(t574[1], t575[1]), vaddq_u64(t574[2], t575[2]), vaddq_u64(t574[3], t575[3])];
    let t577 = b17_r0ls_p(l, lid0, live, arg0, t576, x24);
    x32 = t577;
    let t578: [uint64x2_t; 4];
    if ((x24) >= (8u32)) {
        t578 = x32;
    } else {
        let t579 = b18_s_p(l, lid0, live, (8u32).wrapping_mul(x24));
        let t580 = [vdupq_n_u64(t579); 4];
        let t581: [uint64x2_t; 4] = [vandq_u64(x32[0], t580[0]), vandq_u64(x32[1], t580[1]), vandq_u64(x32[2], t580[2]), vandq_u64(x32[3], t580[3])];
        t578 = t581;
    }
    res = t578;
    return res;
}

/// The group `l.group`'s threads, 8 at a time: `argv[j]` the address of argument `j`'s bytes (a buffer's address,
/// a value's bytes).
///
/// # Safety
///
/// `argv` holds the kernel's arguments, its buffers as large as it reads and writes; `l` is a 1-D group's launch.
pub unsafe fn parent_push_w8(argv: *const *const u8, l: &Launch) {
    let n = l.block[0] * l.block[1] * l.block[2];
    let arg0: *const u8 = (*argv.add(0) as *const *const u8).read_unaligned();
    let arg1: *const u8 = (*argv.add(1) as *const *const u8).read_unaligned();
    let arg2: *const u8 = (*argv.add(2) as *const *const u8).read_unaligned();
    let x24: *const u8 = *argv.add(3);
    let arg4: *const u8 = (*argv.add(4) as *const *const u8).read_unaligned();
    let x48: u32 = (*argv.add(5) as *const u32).read_unaligned();
    let mut t582: bool = !(arg4).is_null();
    if t582 {
        let mut t583: bool = ((l.group[0]) >= ((arg4.wrapping_add((((x48) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        if !t583 {
            t583 = ((l.group[1]) >= ((arg4.wrapping_add(((((x48).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        let mut t584: bool = t583;
        if !t584 {
            t584 = ((l.group[2]) >= ((arg4.wrapping_add(((((x48).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        t582 = t584;
    }
    if t582 {
        return;
    }
    let mut lid0 = 0u32;
    while lid0 + 8 <= n {
        let mut live = [vdupq_n_u32(u32::MAX); 2];
        let gid0 = l.group[0] * l.block[0] + lid0;
        'chunk: {
            b0_r0r1r2sg_f(l, lid0, arg0, arg1, arg2, x24);
        }
        lid0 += 8;
    }
    if lid0 < n {
        // Lanes part: the group's last threads, those under `n`.
        let t585: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
        let mut live = t585;
        let gid0 = l.group[0] * l.block[0] + lid0;
        'chunk: {
            b0_r0r1r2sg_p(l, lid0, live, arg0, arg1, arg2, x24);
        }
    }
}
