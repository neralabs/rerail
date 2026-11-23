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

#[inline(always)]
unsafe fn b0_r0r1r2sg_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, arg2: *const u8, x24: *const u8) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x24 as *const u32).read_unaligned();
    let mut x48: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
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
        let mut t12: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t13: [uint32x4_t; 2] = [vandq_u32(live[0], t11[0]), vandq_u32(live[1], t11[1])];
        let t14: [uint32x4_t; 2] = [vbicq_u32(live[0], t11[0]), vbicq_u32(live[1], t11[1])];
        {
            let t15: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x48[0]), vmovn_u64(x48[1])), vcombine_u32(vmovn_u64(x48[2]), vmovn_u64(x48[3]))];
            let t16 = [vdupq_n_u32(2147483648u32); 2];
            let t17: [uint32x4_t; 2] = [vorrq_u32(t16[0], t15[0]), vorrq_u32(t16[1], t15[1])];
            t12 = [vbslq_u32(t13[0], t17[0], t12[0]), vbslq_u32(t13[1], t17[1], t12[1])];
        }
        {
            let t18: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x48[0]), vmovn_u64(x48[1])), vcombine_u32(vmovn_u64(x48[2]), vmovn_u64(x48[3]))];
            t12 = [vbslq_u32(t14[0], t18[0], t12[0]), vbslq_u32(t14[1], t18[1], t12[1])];
        }
        let t19 = (arg2 as *mut u32).add((gid0) as usize);
        if all(&live) {
            vst1q_u32(t19.add(0), t12[0]);
            vst1q_u32(t19.add(4), t12[1]);
        } else {
            let mut t20w = 0u32;
            let t20 = core::hint::black_box(&raw mut t20w);
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t19.add(0), t20).write(vgetq_lane_u32::<0>(t12[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t19.add(1), t20).write(vgetq_lane_u32::<1>(t12[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t19.add(2), t20).write(vgetq_lane_u32::<2>(t12[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t19.add(3), t20).write(vgetq_lane_u32::<3>(t12[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t19.add(4), t20).write(vgetq_lane_u32::<0>(t12[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t19.add(5), t20).write(vgetq_lane_u32::<1>(t12[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t19.add(6), t20).write(vgetq_lane_u32::<2>(t12[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t19.add(7), t20).write(vgetq_lane_u32::<3>(t12[1]));
        }
    }
}

#[inline(always)]
unsafe fn b0_r0r1r2sg_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, arg2: *const u8, x24: *const u8) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x24 as *const u32).read_unaligned();
    let mut x48: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    if ((gid0) as u64 + 7) < ((a0) as u64) {
        let t21 = b1_r0r1g_f(l, lid0, arg0, arg1);
        x48 = t21;
        let t22 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
        let t23: [uint64x2_t; 4] = [vandq_u64(x48[0], t22[0]), vandq_u64(x48[1], t22[1]), vandq_u64(x48[2], t22[2]), vandq_u64(x48[3], t22[3])];
        let t24 = [vdupq_n_u64(0u64); 4];
        let t25: [uint64x2_t; 4] = [vceqq_u64(t23[0], t24[0]), vceqq_u64(t23[1], t24[1]), vceqq_u64(t23[2], t24[2]), vceqq_u64(t23[3], t24[3])];
        let t26: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t25[0]), vmovn_u64(t25[1])), vcombine_u32(vmovn_u64(t25[2]), vmovn_u64(t25[3]))];
        let t27: [uint32x4_t; 2] = [vmvnq_u32(t26[0]), vmvnq_u32(t26[1])];
        let mut t28: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t29: [uint32x4_t; 2] = [vmvnq_u32(t27[0]), vmvnq_u32(t27[1])];
        {
            let t30: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x48[0]), vmovn_u64(x48[1])), vcombine_u32(vmovn_u64(x48[2]), vmovn_u64(x48[3]))];
            let t31 = [vdupq_n_u32(2147483648u32); 2];
            let t32: [uint32x4_t; 2] = [vorrq_u32(t31[0], t30[0]), vorrq_u32(t31[1], t30[1])];
            t28 = [vbslq_u32(t27[0], t32[0], t28[0]), vbslq_u32(t27[1], t32[1], t28[1])];
        }
        {
            let t33: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x48[0]), vmovn_u64(x48[1])), vcombine_u32(vmovn_u64(x48[2]), vmovn_u64(x48[3]))];
            t28 = [vbslq_u32(t29[0], t33[0], t28[0]), vbslq_u32(t29[1], t33[1], t28[1])];
        }
        let t34 = (arg2 as *mut u32).add((gid0) as usize);
        vst1q_u32(t34.add(0), t28[0]);
        vst1q_u32(t34.add(4), t28[1]);
    } else if (((gid0) as u64 + 7) <= u32::MAX as u64 && ((gid0) as u64) >= ((a0) as u64)) {
        return;
    } else {
        // Lanes part: the chunk's lanes on both sides of the bound.
        let t35: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t36 = [vdupq_n_u32(a0); 2];
        let t37: [uint32x4_t; 2] = [vcgeq_u32(t35[0], t36[0]), vcgeq_u32(t35[1], t36[1])];
        let t38: [uint32x4_t; 2] = [vandq_u32(live[0], t37[0]), vandq_u32(live[1], t37[1])];
        if any(&t38) {
            live = [vbicq_u32(live[0], t38[0]), vbicq_u32(live[1], t38[1])];
        }
        if any(&live) {
            let t39 = b1_r0r1g_p(l, lid0, live, arg0, arg1);
            x48 = t39;
            let t40 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
            let t41: [uint64x2_t; 4] = [vandq_u64(x48[0], t40[0]), vandq_u64(x48[1], t40[1]), vandq_u64(x48[2], t40[2]), vandq_u64(x48[3], t40[3])];
            let t42 = [vdupq_n_u64(0u64); 4];
            let t43: [uint64x2_t; 4] = [vceqq_u64(t41[0], t42[0]), vceqq_u64(t41[1], t42[1]), vceqq_u64(t41[2], t42[2]), vceqq_u64(t41[3], t42[3])];
            let t44: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t43[0]), vmovn_u64(t43[1])), vcombine_u32(vmovn_u64(t43[2]), vmovn_u64(t43[3]))];
            let t45: [uint32x4_t; 2] = [vmvnq_u32(t44[0]), vmvnq_u32(t44[1])];
            let mut t46: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t47: [uint32x4_t; 2] = [vandq_u32(live[0], t45[0]), vandq_u32(live[1], t45[1])];
            let t48: [uint32x4_t; 2] = [vbicq_u32(live[0], t45[0]), vbicq_u32(live[1], t45[1])];
            {
                let t49: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x48[0]), vmovn_u64(x48[1])), vcombine_u32(vmovn_u64(x48[2]), vmovn_u64(x48[3]))];
                let t50 = [vdupq_n_u32(2147483648u32); 2];
                let t51: [uint32x4_t; 2] = [vorrq_u32(t50[0], t49[0]), vorrq_u32(t50[1], t49[1])];
                t46 = [vbslq_u32(t47[0], t51[0], t46[0]), vbslq_u32(t47[1], t51[1], t46[1])];
            }
            {
                let t52: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x48[0]), vmovn_u64(x48[1])), vcombine_u32(vmovn_u64(x48[2]), vmovn_u64(x48[3]))];
                t46 = [vbslq_u32(t48[0], t52[0], t46[0]), vbslq_u32(t48[1], t52[1], t46[1])];
            }
            let t53 = (arg2 as *mut u32).add((gid0) as usize);
            if all(&live) {
                vst1q_u32(t53.add(0), t46[0]);
                vst1q_u32(t53.add(4), t46[1]);
            } else {
                let mut t54w = 0u32;
                let t54 = core::hint::black_box(&raw mut t54w);
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t53.add(0), t54).write(vgetq_lane_u32::<0>(t46[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t53.add(1), t54).write(vgetq_lane_u32::<1>(t46[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t53.add(2), t54).write(vgetq_lane_u32::<2>(t46[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t53.add(3), t54).write(vgetq_lane_u32::<3>(t46[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t53.add(4), t54).write(vgetq_lane_u32::<0>(t46[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t53.add(5), t54).write(vgetq_lane_u32::<1>(t46[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t53.add(6), t54).write(vgetq_lane_u32::<2>(t46[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t53.add(7), t54).write(vgetq_lane_u32::<3>(t46[1]));
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
    let t55: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
    let t56 = [vdupq_n_u32(0u32); 2];
    let t57: [uint32x4_t; 2] = [vceqq_u32(t55[0], t56[0]), vceqq_u32(t55[1], t56[1])];
    let t58: [uint32x4_t; 2] = [vandq_u32(live[0], t57[0]), vandq_u32(live[1], t57[1])];
    if any(&t58) {
        let t59 = [vdupq_n_u64(4294967295u64); 4];
        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t58[0])))), t59[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t58[0])))), t59[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t58[1])))), t59[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t58[1])))), t59[3], res[3])];
        live = [vbicq_u32(live[0], t58[0]), vbicq_u32(live[1], t58[1])];
    }
    if any(&live) {
        let t60 = [vdupq_n_u32(1u32); 2];
        let t61: [uint32x4_t; 2] = [vsubq_u32(t55[0], t60[0]), vsubq_u32(t55[1], t60[1])];
        x24 = [vbslq_u32(live[0], t61[0], x24[0]), vbslq_u32(live[1], t61[1], x24[1])];
        let t62 = b2_r0r1l_p(l, lid0, live, arg0, arg1, x24);
        x32 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t62[0], x32[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t62[1], x32[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t62[2], x32[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t62[3], x32[3])];
        let t63 = [vdupq_n_u64(1u64); 4];
        let t64: [uint64x2_t; 4] = [vandq_u64(x32[0], t63[0]), vandq_u64(x32[1], t63[1]), vandq_u64(x32[2], t63[2]), vandq_u64(x32[3], t63[3])];
        let t65 = [vdupq_n_u64(0u64); 4];
        let t66: [uint64x2_t; 4] = [vceqq_u64(t64[0], t65[0]), vceqq_u64(t64[1], t65[1]), vceqq_u64(t64[2], t65[2]), vceqq_u64(t64[3], t65[3])];
        let t67: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t66[0]), vmovn_u64(t66[1])), vcombine_u32(vmovn_u64(t66[2]), vmovn_u64(t66[3]))];
        let t68: [uint32x4_t; 2] = [vandq_u32(live[0], t67[0]), vandq_u32(live[1], t67[1])];
        if any(&t68) {
            let t69: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x24[0])), vmovl_u32(vget_high_u32(x24[0])), vmovl_u32(vget_low_u32(x24[1])), vmovl_u32(vget_high_u32(x24[1]))];
            res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t68[0])))), t69[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t68[0])))), t69[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t68[1])))), t69[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t68[1])))), t69[3], res[3])];
            live = [vbicq_u32(live[0], t68[0]), vbicq_u32(live[1], t68[1])];
        }
        if any(&live) {
            let t70: [uint64x2_t; 4] = [vshlq_u64(x32[0], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[1], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[2], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[3], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64)))];
            let t71: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t70[0]), vmovn_u64(t70[1])), vcombine_u32(vmovn_u64(t70[2]), vmovn_u64(t70[3]))];
            x40 = [vbslq_u32(live[0], t71[0], x40[0]), vbslq_u32(live[1], t71[1], x40[1])];
            let t72: [uint32x4_t; 2] = [vsubq_u32(x40[0], t60[0]), vsubq_u32(x40[1], t60[1])];
            let t73: [uint32x4_t; 2] = [vcgeq_u32(t72[0], t55[0]), vcgeq_u32(t72[1], t55[1])];
            let t74: [uint32x4_t; 2] = [vandq_u32(live[0], t73[0]), vandq_u32(live[1], t73[1])];
            if any(&t74) {
                let t75 = [vdupq_n_u64(4294967295u64); 4];
                res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t74[0])))), t75[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t74[0])))), t75[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t74[1])))), t75[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t74[1])))), t75[3], res[3])];
                live = [vbicq_u32(live[0], t74[0]), vbicq_u32(live[1], t74[1])];
            }
            if any(&live) {
                let t76: [uint32x4_t; 2] = [vsubq_u32(t55[0], x40[0]), vsubq_u32(t55[1], x40[1])];
                x48 = [vbslq_u32(live[0], t76[0], x48[0]), vbslq_u32(live[1], t76[1], x48[1])];
                let t77 = [vdupq_n_u64(2u64); 4];
                let t78: [uint64x2_t; 4] = [vandq_u64(x32[0], t77[0]), vandq_u64(x32[1], t77[1]), vandq_u64(x32[2], t77[2]), vandq_u64(x32[3], t77[3])];
                let t79: [uint64x2_t; 4] = [vceqq_u64(t78[0], t65[0]), vceqq_u64(t78[1], t65[1]), vceqq_u64(t78[2], t65[2]), vceqq_u64(t78[3], t65[3])];
                let t80: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t79[0]), vmovn_u64(t79[1])), vcombine_u32(vmovn_u64(t79[2]), vmovn_u64(t79[3]))];
                let t81: [uint32x4_t; 2] = [vmvnq_u32(t80[0]), vmvnq_u32(t80[1])];
                let t82: [uint32x4_t; 2] = [vandq_u32(live[0], t81[0]), vandq_u32(live[1], t81[1])];
                if any(&t82) {
                    let t83: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                    res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t82[0])))), t83[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t82[0])))), t83[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t82[1])))), t83[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t82[1])))), t83[3], res[3])];
                    live = [vbicq_u32(live[0], t82[0]), vbicq_u32(live[1], t82[1])];
                }
                if any(&live) {
                    let t84: [uint32x4_t; 2] = [vceqq_u32(x48[0], t56[0]), vceqq_u32(x48[1], t56[1])];
                    let t85: [uint32x4_t; 2] = [vandq_u32(live[0], t84[0]), vandq_u32(live[1], t84[1])];
                    if any(&t85) {
                        let t86 = [vdupq_n_u64(4294967295u64); 4];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t85[0])))), t86[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t85[0])))), t86[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t85[1])))), t86[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t85[1])))), t86[3], res[3])];
                        live = [vbicq_u32(live[0], t85[0]), vbicq_u32(live[1], t85[1])];
                    }
                    if any(&live) {
                        let t87 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
                        let t88: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                        let t89: [uint64x2_t; 4] = [vorrq_u64(t87[0], t88[0]), vorrq_u64(t87[1], t88[1]), vorrq_u64(t87[2], t88[2]), vorrq_u64(t87[3], t88[3])];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t89[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t89[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t89[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t89[3], res[3])];
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
        let t90: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t91 = [vdupq_n_u32(1u32); 2];
        let t92: [uint32x4_t; 2] = [vsubq_u32(t90[0], t91[0]), vsubq_u32(t90[1], t91[1])];
        x24 = t92;
        let t93 = b2_r0r1l_f(l, lid0, arg0, arg1, x24);
        x32 = t93;
        let t94 = [vdupq_n_u64(1u64); 4];
        let t95: [uint64x2_t; 4] = [vandq_u64(x32[0], t94[0]), vandq_u64(x32[1], t94[1]), vandq_u64(x32[2], t94[2]), vandq_u64(x32[3], t94[3])];
        let t96 = [vdupq_n_u64(0u64); 4];
        let t97: [uint64x2_t; 4] = [vceqq_u64(t95[0], t96[0]), vceqq_u64(t95[1], t96[1]), vceqq_u64(t95[2], t96[2]), vceqq_u64(t95[3], t96[3])];
        let t98: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t97[0]), vmovn_u64(t97[1])), vcombine_u32(vmovn_u64(t97[2]), vmovn_u64(t97[3]))];
        if any(&t98) {
            let t99: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x24[0])), vmovl_u32(vget_high_u32(x24[0])), vmovl_u32(vget_low_u32(x24[1])), vmovl_u32(vget_high_u32(x24[1]))];
            res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t98[0])))), t99[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t98[0])))), t99[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t98[1])))), t99[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t98[1])))), t99[3], res[3])];
            live = [vbicq_u32(live[0], t98[0]), vbicq_u32(live[1], t98[1])];
        }
        if any(&live) {
            let t100: [uint64x2_t; 4] = [vshlq_u64(x32[0], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[1], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[2], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[3], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64)))];
            let t101: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t100[0]), vmovn_u64(t100[1])), vcombine_u32(vmovn_u64(t100[2]), vmovn_u64(t100[3]))];
            x40 = [vbslq_u32(live[0], t101[0], x40[0]), vbslq_u32(live[1], t101[1], x40[1])];
            let t102: [uint32x4_t; 2] = [vsubq_u32(x40[0], t91[0]), vsubq_u32(x40[1], t91[1])];
            let t103: [uint32x4_t; 2] = [vcgeq_u32(t102[0], t90[0]), vcgeq_u32(t102[1], t90[1])];
            let t104: [uint32x4_t; 2] = [vandq_u32(live[0], t103[0]), vandq_u32(live[1], t103[1])];
            if any(&t104) {
                let t105 = [vdupq_n_u64(4294967295u64); 4];
                res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t104[0])))), t105[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t104[0])))), t105[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t104[1])))), t105[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t104[1])))), t105[3], res[3])];
                live = [vbicq_u32(live[0], t104[0]), vbicq_u32(live[1], t104[1])];
            }
            if any(&live) {
                let t106: [uint32x4_t; 2] = [vsubq_u32(t90[0], x40[0]), vsubq_u32(t90[1], x40[1])];
                x48 = [vbslq_u32(live[0], t106[0], x48[0]), vbslq_u32(live[1], t106[1], x48[1])];
                let t107 = [vdupq_n_u64(2u64); 4];
                let t108: [uint64x2_t; 4] = [vandq_u64(x32[0], t107[0]), vandq_u64(x32[1], t107[1]), vandq_u64(x32[2], t107[2]), vandq_u64(x32[3], t107[3])];
                let t109: [uint64x2_t; 4] = [vceqq_u64(t108[0], t96[0]), vceqq_u64(t108[1], t96[1]), vceqq_u64(t108[2], t96[2]), vceqq_u64(t108[3], t96[3])];
                let t110: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t109[0]), vmovn_u64(t109[1])), vcombine_u32(vmovn_u64(t109[2]), vmovn_u64(t109[3]))];
                let t111: [uint32x4_t; 2] = [vmvnq_u32(t110[0]), vmvnq_u32(t110[1])];
                let t112: [uint32x4_t; 2] = [vandq_u32(live[0], t111[0]), vandq_u32(live[1], t111[1])];
                if any(&t112) {
                    let t113: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                    res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t112[0])))), t113[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t112[0])))), t113[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t112[1])))), t113[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t112[1])))), t113[3], res[3])];
                    live = [vbicq_u32(live[0], t112[0]), vbicq_u32(live[1], t112[1])];
                }
                if any(&live) {
                    let t114 = [vdupq_n_u32(0u32); 2];
                    let t115: [uint32x4_t; 2] = [vceqq_u32(x48[0], t114[0]), vceqq_u32(x48[1], t114[1])];
                    let t116: [uint32x4_t; 2] = [vandq_u32(live[0], t115[0]), vandq_u32(live[1], t115[1])];
                    if any(&t116) {
                        let t117 = [vdupq_n_u64(4294967295u64); 4];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t116[0])))), t117[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t116[0])))), t117[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t116[1])))), t117[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t116[1])))), t117[3], res[3])];
                        live = [vbicq_u32(live[0], t116[0]), vbicq_u32(live[1], t116[1])];
                    }
                    if any(&live) {
                        let t118 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
                        let t119: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                        let t120: [uint64x2_t; 4] = [vorrq_u64(t118[0], t119[0]), vorrq_u64(t118[1], t119[1]), vorrq_u64(t118[2], t119[2]), vorrq_u64(t118[3], t119[3])];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t120[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t120[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t120[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t120[3], res[3])];
                        live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
                    }
                }
            }
        }
    } else if false {
        let t121 = [vdupq_n_u64(4294967295u64); 4];
        res = t121;
        return res;
    } else {
        // Lanes part: the chunk's lanes on both sides of the bound.
        let t122: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t123 = [vdupq_n_u32(0u32); 2];
        let t124: [uint32x4_t; 2] = [vceqq_u32(t122[0], t123[0]), vceqq_u32(t122[1], t123[1])];
        let t125: [uint32x4_t; 2] = [vandq_u32(live[0], t124[0]), vandq_u32(live[1], t124[1])];
        if any(&t125) {
            let t126 = [vdupq_n_u64(4294967295u64); 4];
            res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t125[0])))), t126[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t125[0])))), t126[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t125[1])))), t126[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t125[1])))), t126[3], res[3])];
            live = [vbicq_u32(live[0], t125[0]), vbicq_u32(live[1], t125[1])];
        }
        if any(&live) {
            let t127 = [vdupq_n_u32(1u32); 2];
            let t128: [uint32x4_t; 2] = [vsubq_u32(t122[0], t127[0]), vsubq_u32(t122[1], t127[1])];
            x24 = [vbslq_u32(live[0], t128[0], x24[0]), vbslq_u32(live[1], t128[1], x24[1])];
            let t129 = b2_r0r1l_p(l, lid0, live, arg0, arg1, x24);
            x32 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t129[0], x32[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t129[1], x32[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t129[2], x32[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t129[3], x32[3])];
            let t130 = [vdupq_n_u64(1u64); 4];
            let t131: [uint64x2_t; 4] = [vandq_u64(x32[0], t130[0]), vandq_u64(x32[1], t130[1]), vandq_u64(x32[2], t130[2]), vandq_u64(x32[3], t130[3])];
            let t132 = [vdupq_n_u64(0u64); 4];
            let t133: [uint64x2_t; 4] = [vceqq_u64(t131[0], t132[0]), vceqq_u64(t131[1], t132[1]), vceqq_u64(t131[2], t132[2]), vceqq_u64(t131[3], t132[3])];
            let t134: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t133[0]), vmovn_u64(t133[1])), vcombine_u32(vmovn_u64(t133[2]), vmovn_u64(t133[3]))];
            let t135: [uint32x4_t; 2] = [vandq_u32(live[0], t134[0]), vandq_u32(live[1], t134[1])];
            if any(&t135) {
                let t136: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x24[0])), vmovl_u32(vget_high_u32(x24[0])), vmovl_u32(vget_low_u32(x24[1])), vmovl_u32(vget_high_u32(x24[1]))];
                res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t135[0])))), t136[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t135[0])))), t136[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t135[1])))), t136[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t135[1])))), t136[3], res[3])];
                live = [vbicq_u32(live[0], t135[0]), vbicq_u32(live[1], t135[1])];
            }
            if any(&live) {
                let t137: [uint64x2_t; 4] = [vshlq_u64(x32[0], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[1], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[2], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[3], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64)))];
                let t138: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t137[0]), vmovn_u64(t137[1])), vcombine_u32(vmovn_u64(t137[2]), vmovn_u64(t137[3]))];
                x40 = [vbslq_u32(live[0], t138[0], x40[0]), vbslq_u32(live[1], t138[1], x40[1])];
                let t139: [uint32x4_t; 2] = [vsubq_u32(x40[0], t127[0]), vsubq_u32(x40[1], t127[1])];
                let t140: [uint32x4_t; 2] = [vcgeq_u32(t139[0], t122[0]), vcgeq_u32(t139[1], t122[1])];
                let t141: [uint32x4_t; 2] = [vandq_u32(live[0], t140[0]), vandq_u32(live[1], t140[1])];
                if any(&t141) {
                    let t142 = [vdupq_n_u64(4294967295u64); 4];
                    res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t141[0])))), t142[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t141[0])))), t142[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t141[1])))), t142[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t141[1])))), t142[3], res[3])];
                    live = [vbicq_u32(live[0], t141[0]), vbicq_u32(live[1], t141[1])];
                }
                if any(&live) {
                    let t143: [uint32x4_t; 2] = [vsubq_u32(t122[0], x40[0]), vsubq_u32(t122[1], x40[1])];
                    x48 = [vbslq_u32(live[0], t143[0], x48[0]), vbslq_u32(live[1], t143[1], x48[1])];
                    let t144 = [vdupq_n_u64(2u64); 4];
                    let t145: [uint64x2_t; 4] = [vandq_u64(x32[0], t144[0]), vandq_u64(x32[1], t144[1]), vandq_u64(x32[2], t144[2]), vandq_u64(x32[3], t144[3])];
                    let t146: [uint64x2_t; 4] = [vceqq_u64(t145[0], t132[0]), vceqq_u64(t145[1], t132[1]), vceqq_u64(t145[2], t132[2]), vceqq_u64(t145[3], t132[3])];
                    let t147: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t146[0]), vmovn_u64(t146[1])), vcombine_u32(vmovn_u64(t146[2]), vmovn_u64(t146[3]))];
                    let t148: [uint32x4_t; 2] = [vmvnq_u32(t147[0]), vmvnq_u32(t147[1])];
                    let t149: [uint32x4_t; 2] = [vandq_u32(live[0], t148[0]), vandq_u32(live[1], t148[1])];
                    if any(&t149) {
                        let t150: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t149[0])))), t150[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t149[0])))), t150[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t149[1])))), t150[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t149[1])))), t150[3], res[3])];
                        live = [vbicq_u32(live[0], t149[0]), vbicq_u32(live[1], t149[1])];
                    }
                    if any(&live) {
                        let t151: [uint32x4_t; 2] = [vceqq_u32(x48[0], t123[0]), vceqq_u32(x48[1], t123[1])];
                        let t152: [uint32x4_t; 2] = [vandq_u32(live[0], t151[0]), vandq_u32(live[1], t151[1])];
                        if any(&t152) {
                            let t153 = [vdupq_n_u64(4294967295u64); 4];
                            res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t152[0])))), t153[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t152[0])))), t153[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t152[1])))), t153[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t152[1])))), t153[3], res[3])];
                            live = [vbicq_u32(live[0], t152[0]), vbicq_u32(live[1], t152[1])];
                        }
                        if any(&live) {
                            let t154 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
                            let t155: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                            let t156: [uint64x2_t; 4] = [vorrq_u64(t154[0], t155[0]), vorrq_u64(t154[1], t155[1]), vorrq_u64(t154[2], t155[2]), vorrq_u64(t154[3], t155[3])];
                            res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t156[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t156[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t156[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t156[3], res[3])];
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
    let t157 = b3_r0r1sl_p(l, lid0, live, arg0, arg1, 0u32, x16);
    res = t157;
    return res;
}

#[inline(always)]
unsafe fn b2_r0r1l_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t158 = b3_r0r1sl_f(l, lid0, arg0, arg1, 0u32, x16);
    res = t158;
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
    let t159 = b4_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    x32 = t159;
    let t160: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t161 = b5_r0r1sl_p(l, lid0, live, arg0, arg1, (1u32).wrapping_add(x16), t160);
    x40 = t161;
    let t162 = b6_s_p(l, lid0, live, x32);
    x48 = t162;
    let t163 = [vdupq_n_u32((1024u32).wrapping_sub(1u32)); 2];
    let t164: [uint32x4_t; 2] = [vandq_u32(x24[0], t163[0]), vandq_u32(x24[1], t163[1])];
    let t165 = [vdupq_n_u32(x32); 2];
    let t166: [uint32x4_t; 2] = [vmulq_u32(t164[0], t165[0]), vmulq_u32(t164[1], t165[1])];
    let t167 = b7_r0r1lls_p(l, lid0, live, arg0, arg1, x40, t166, 8u32);
    let t168 = [vdupq_n_u64(x48); 4];
    let t169: [uint64x2_t; 4] = [vandq_u64(t167[0], t168[0]), vandq_u64(t167[1], t168[1]), vandq_u64(t167[2], t168[2]), vandq_u64(t167[3], t168[3])];
    x56 = t169;
    let t170 = [vdupq_n_u64(x48); 4];
    let t171: [uint64x2_t; 4] = [vceqq_u64(x56[0], t170[0]), vceqq_u64(x56[1], t170[1]), vceqq_u64(x56[2], t170[2]), vceqq_u64(x56[3], t170[3])];
    let t172: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t171[0]), vmovn_u64(t171[1])), vcombine_u32(vmovn_u64(t171[2]), vmovn_u64(t171[3]))];
    let mut t173: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t174: [uint32x4_t; 2] = [vandq_u32(live[0], t172[0]), vandq_u32(live[1], t172[1])];
    let t175: [uint32x4_t; 2] = [vbicq_u32(live[0], t172[0]), vbicq_u32(live[1], t172[1])];
    if any(&t174) {
        let t176 = b8_r0r1sl_p(l, lid0, t174, arg0, arg1, (x16).wrapping_add(1u32), x24);
        t173 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t174[0])))), t176[0], t173[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t174[0])))), t176[1], t173[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t174[1])))), t176[2], t173[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t174[1])))), t176[3], t173[3])];
    }
    {
        t173 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t175[0])))), x56[0], t173[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t175[0])))), x56[1], t173[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t175[1])))), x56[2], t173[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t175[1])))), x56[3], t173[3])];
    }
    res = t173;
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
    let t177 = b4_r0r1s_f(l, lid0, arg0, arg1, x16);
    x32 = t177;
    let t178: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t179 = b5_r0r1sl_f(l, lid0, arg0, arg1, (1u32).wrapping_add(x16), t178);
    x40 = t179;
    let t180 = b6_s_f(l, lid0, x32);
    x48 = t180;
    let t181 = [vdupq_n_u32((1024u32).wrapping_sub(1u32)); 2];
    let t182: [uint32x4_t; 2] = [vandq_u32(x24[0], t181[0]), vandq_u32(x24[1], t181[1])];
    let t183 = [vdupq_n_u32(x32); 2];
    let t184: [uint32x4_t; 2] = [vmulq_u32(t182[0], t183[0]), vmulq_u32(t182[1], t183[1])];
    let t185 = b7_r0r1lls_f(l, lid0, arg0, arg1, x40, t184, 8u32);
    let t186 = [vdupq_n_u64(x48); 4];
    let t187: [uint64x2_t; 4] = [vandq_u64(t185[0], t186[0]), vandq_u64(t185[1], t186[1]), vandq_u64(t185[2], t186[2]), vandq_u64(t185[3], t186[3])];
    x56 = t187;
    let t188 = [vdupq_n_u64(x48); 4];
    let t189: [uint64x2_t; 4] = [vceqq_u64(x56[0], t188[0]), vceqq_u64(x56[1], t188[1]), vceqq_u64(x56[2], t188[2]), vceqq_u64(x56[3], t188[3])];
    let t190: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t189[0]), vmovn_u64(t189[1])), vcombine_u32(vmovn_u64(t189[2]), vmovn_u64(t189[3]))];
    let mut t191: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t192: [uint32x4_t; 2] = [vmvnq_u32(t190[0]), vmvnq_u32(t190[1])];
    if any(&t190) {
        let t193 = b8_r0r1sl_p(l, lid0, t190, arg0, arg1, (x16).wrapping_add(1u32), x24);
        t191 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t190[0])))), t193[0], t191[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t190[0])))), t193[1], t191[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t190[1])))), t193[2], t191[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t190[1])))), t193[3], t191[3])];
    }
    {
        t191 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t192[0])))), x56[0], t191[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t192[0])))), x56[1], t191[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t192[1])))), x56[2], t191[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t192[1])))), x56[3], t191[3])];
    }
    res = t191;
    return res;
}

#[inline(always)]
unsafe fn b4_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t194 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    res = (((t194).wrapping_shr(((4u32).wrapping_mul(x16)) as u32)) & (15u32));
    return res;
}

#[inline(always)]
unsafe fn b4_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t195 = b9_r0r1s_f(l, lid0, arg0, arg1, 16u32);
    res = (((t195).wrapping_shr(((4u32).wrapping_mul(x16)) as u32)) & (15u32));
    return res;
}

#[inline(always)]
unsafe fn b5_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t196 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 40u32);
    let t197 = [vdupq_n_u32(t196); 2];
    let t198: [uint32x4_t; 2] = [vmulq_u32(x24[0], t197[0]), vmulq_u32(x24[1], t197[1])];
    let t199 = b10_r0r1s_p(l, lid0, live, arg0, arg1, 0u32);
    let t200 = [vdupq_n_u32(t199); 2];
    let t201: [uint32x4_t; 2] = [vaddq_u32(t198[0], t200[0]), vaddq_u32(t198[1], t200[1])];
    x32 = t201;
    if ((x16) == (0u32)) {
        res = x32;
        return res;
    }
    let t202 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    let t203: [uint32x4_t; 2] = [vshlq_u32(x24[0], vdupq_n_s32(((10u32) & 31) as i32)), vshlq_u32(x24[1], vdupq_n_s32(((10u32) & 31) as i32))];
    let t204 = [vdupq_n_u32(t202); 2];
    let t205: [uint32x4_t; 2] = [vsubq_u32(t204[0], t203[0]), vsubq_u32(t204[1], t203[1])];
    let t206 = b11_ls_p(l, lid0, live, t205, 1024u32);
    x40 = t206;
    let t207 = b10_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    let t208 = [vdupq_n_u32(t207); 2];
    let t209: [uint32x4_t; 2] = [vmulq_u32(x40[0], t208[0]), vmulq_u32(x40[1], t208[1])];
    let t210: [uint32x4_t; 2] = [vaddq_u32(x32[0], t209[0]), vaddq_u32(x32[1], t209[1])];
    res = t210;
    return res;
}

#[inline(always)]
unsafe fn b5_r0r1sl_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t211 = b9_r0r1s_f(l, lid0, arg0, arg1, 40u32);
    let t212 = [vdupq_n_u32(t211); 2];
    let t213: [uint32x4_t; 2] = [vmulq_u32(x24[0], t212[0]), vmulq_u32(x24[1], t212[1])];
    let t214 = b10_r0r1s_f(l, lid0, arg0, arg1, 0u32);
    let t215 = [vdupq_n_u32(t214); 2];
    let t216: [uint32x4_t; 2] = [vaddq_u32(t213[0], t215[0]), vaddq_u32(t213[1], t215[1])];
    x32 = t216;
    if ((x16) == (0u32)) {
        res = x32;
        return res;
    }
    let t217 = b9_r0r1s_f(l, lid0, arg0, arg1, 4u32);
    let t218: [uint32x4_t; 2] = [vshlq_u32(x24[0], vdupq_n_s32(((10u32) & 31) as i32)), vshlq_u32(x24[1], vdupq_n_s32(((10u32) & 31) as i32))];
    let t219 = [vdupq_n_u32(t217); 2];
    let t220: [uint32x4_t; 2] = [vsubq_u32(t219[0], t218[0]), vsubq_u32(t219[1], t218[1])];
    let t221 = b11_ls_f(l, lid0, t220, 1024u32);
    x40 = t221;
    let t222 = b10_r0r1s_f(l, lid0, arg0, arg1, x16);
    let t223 = [vdupq_n_u32(t222); 2];
    let t224: [uint32x4_t; 2] = [vmulq_u32(x40[0], t223[0]), vmulq_u32(x40[1], t223[1])];
    let t225: [uint32x4_t; 2] = [vaddq_u32(x32[0], t224[0]), vaddq_u32(x32[1], t224[1])];
    res = t225;
    return res;
}

#[inline(always)]
unsafe fn b6_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t226: u64;
    if ((x0) >= (8u32)) {
        t226 = !(0u64);
    } else {
        t226 = ((1u64).wrapping_shl(((((x0).wrapping_mul(8u32)) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t226;
    return res;
}

#[inline(always)]
unsafe fn b6_s_f(l: &Launch, lid0: u32, x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t227: u64;
    if ((x0) >= (8u32)) {
        t227 = !(0u64);
    } else {
        t227 = ((1u64).wrapping_shl(((((x0).wrapping_mul(8u32)) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t227;
    return res;
}

#[inline(always)]
unsafe fn b6_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t228 = [vdupq_n_u32(8u32); 2];
    let t229: [uint32x4_t; 2] = [vcgeq_u32(x0[0], t228[0]), vcgeq_u32(x0[1], t228[1])];
    let mut t230: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t231: [uint32x4_t; 2] = [vandq_u32(live[0], t229[0]), vandq_u32(live[1], t229[1])];
    let t232: [uint32x4_t; 2] = [vbicq_u32(live[0], t229[0]), vbicq_u32(live[1], t229[1])];
    {
        let t233 = [vdupq_n_u64(!(0u64)); 4];
        t230 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t231[0])))), t233[0], t230[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t231[0])))), t233[1], t230[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t231[1])))), t233[2], t230[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t231[1])))), t233[3], t230[3])];
    }
    {
        let t234: [uint32x4_t; 2] = [vmulq_u32(x0[0], t228[0]), vmulq_u32(x0[1], t228[1])];
        let t235 = [vdupq_n_u64(1u64); 4];
        let t236: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t234[0])), vmovl_u32(vget_high_u32(t234[0])), vmovl_u32(vget_low_u32(t234[1])), vmovl_u32(vget_high_u32(t234[1]))];
        let t237: [uint64x2_t; 4] = [vshlq_u64(t235[0], vreinterpretq_s64_u64(vandq_u64(t236[0], vdupq_n_u64(63)))), vshlq_u64(t235[1], vreinterpretq_s64_u64(vandq_u64(t236[1], vdupq_n_u64(63)))), vshlq_u64(t235[2], vreinterpretq_s64_u64(vandq_u64(t236[2], vdupq_n_u64(63)))), vshlq_u64(t235[3], vreinterpretq_s64_u64(vandq_u64(t236[3], vdupq_n_u64(63))))];
        let t238: [uint64x2_t; 4] = [vsubq_u64(t237[0], t235[0]), vsubq_u64(t237[1], t235[1]), vsubq_u64(t237[2], t235[2]), vsubq_u64(t237[3], t235[3])];
        t230 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t232[0])))), t238[0], t230[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t232[0])))), t238[1], t230[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t232[1])))), t238[2], t230[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t232[1])))), t238[3], t230[3])];
    }
    res = t230;
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2], x24: [uint32x4_t; 2], x32: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t239 = b12_r0lls_p(l, lid0, live, arg0, x16, x24, x32);
    res = t239;
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1lls_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2], x24: [uint32x4_t; 2], x32: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t240 = b12_r0lls_f(l, lid0, arg0, x16, x24, x32);
    res = t240;
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
    let t241 = b9_r0r1s_p(l, lid0, live, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x16)));
    x32 = t241;
    let t242 = b9_r0r1s_p(l, lid0, live, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x16)));
    x40 = t242;
    let t243: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    x48 = t243;
    let t244 = [vdupq_n_u32(4u32); 2];
    let t245: [uint32x4_t; 2] = [vmulq_u32(x48[0], t244[0]), vmulq_u32(x48[1], t244[1])];
    let t246 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x40, t245, 4u32);
    let t247: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t246[0]), vmovn_u64(t246[1])), vcombine_u32(vmovn_u64(t246[2]), vmovn_u64(t246[3]))];
    x56 = t247;
    let t248: [uint32x4_t; 2] = [vmulq_u32(x48[0], t244[0]), vmulq_u32(x48[1], t244[1])];
    let t249: [uint32x4_t; 2] = [vaddq_u32(t248[0], t244[0]), vaddq_u32(t248[1], t244[1])];
    let t250 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x40, t249, 4u32);
    let t251: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t250[0]), vmovn_u64(t250[1])), vcombine_u32(vmovn_u64(t250[2]), vmovn_u64(t250[3]))];
    x64 = t251;
    let t252 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    x72 = ((t252).wrapping_add((256u32).wrapping_sub(1u32))).wrapping_shr((8u32) as u32);
    let t253: [uint32x4_t; 2] = [vmulq_u32(x48[0], t244[0]), vmulq_u32(x48[1], t244[1])];
    let t254 = [vdupq_n_u32(x72); 2];
    let t255: [uint32x4_t; 2] = [vsubq_u32(t254[0], t253[0]), vsubq_u32(t254[1], t253[1])];
    let t256 = b14_sl_p(l, lid0, live, 4u32, t255);
    x80 = t256;
    let t257: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
    let t258 = [vdupq_n_u32((4u32).wrapping_sub(1u32)); 2];
    let t259: [uint32x4_t; 2] = [vandq_u32(t257[0], t258[0]), vandq_u32(t257[1], t258[1])];
    x88 = t259;
    let t260 = [vdupq_n_u32(0u32); 2];
    let t261: [uint32x4_t; 2] = [vceqq_u32(x88[0], t260[0]), vceqq_u32(x88[1], t260[1])];
    let mut t262: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t263: [uint32x4_t; 2] = [vandq_u32(live[0], t261[0]), vandq_u32(live[1], t261[1])];
    let t264: [uint32x4_t; 2] = [vbicq_u32(live[0], t261[0]), vbicq_u32(live[1], t261[1])];
    {
        t262 = [vbslq_u32(t263[0], t260[0], t262[0]), vbslq_u32(t263[1], t260[1], t262[1])];
    }
    if any(&t264) {
        let t265 = [vdupq_n_u32(1u32); 2];
        let t266: [uint32x4_t; 2] = [vsubq_u32(x88[0], t265[0]), vsubq_u32(x88[1], t265[1])];
        let t267 = [vdupq_n_u32(2u32); 2];
        let t268: [uint32x4_t; 2] = [vmulq_u32(t267[0], t266[0]), vmulq_u32(t267[1], t266[1])];
        let t269: [uint32x4_t; 2] = [vaddq_u32(x56[0], t268[0]), vaddq_u32(x56[1], t268[1])];
        let t270 = b15_ll_p(l, lid0, t264, t269, x64);
        let t271 = b13_r0r1sls_p(l, lid0, t264, arg0, arg1, x32, t270, 2u32);
        let t272: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t271[0]), vmovn_u64(t271[1])), vcombine_u32(vmovn_u64(t271[2]), vmovn_u64(t271[3]))];
        t262 = [vbslq_u32(t264[0], t272[0], t262[0]), vbslq_u32(t264[1], t272[1], t262[1])];
    }
    x96 = t262;
    let t273 = [vdupq_n_u32(2u32); 2];
    let t274: [uint32x4_t; 2] = [vmulq_u32(t273[0], x88[0]), vmulq_u32(t273[1], x88[1])];
    let t275: [uint32x4_t; 2] = [vaddq_u32(x56[0], t274[0]), vaddq_u32(x56[1], t274[1])];
    let t276 = b15_ll_p(l, lid0, live, t275, x64);
    let t277 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t276, 2u32);
    let t278: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t277[0]), vmovn_u64(t277[1])), vcombine_u32(vmovn_u64(t277[2]), vmovn_u64(t277[3]))];
    x104 = t278;
    let t279: [uint32x4_t; 2] = [vmulq_u32(t273[0], x80[0]), vmulq_u32(t273[1], x80[1])];
    let t280: [uint32x4_t; 2] = [vaddq_u32(x56[0], t279[0]), vaddq_u32(x56[1], t279[1])];
    x112 = t280;
    let t281 = [vdupq_n_u32((256u32).wrapping_sub(1u32)); 2];
    let t282: [uint32x4_t; 2] = [vandq_u32(x24[0], t281[0]), vandq_u32(x24[1], t281[1])];
    x120 = t282;
    x128 = 9259542123273814144u64;
    let t283: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x120[0])), vmovl_u32(vget_high_u32(x120[0])), vmovl_u32(vget_low_u32(x120[1])), vmovl_u32(vget_high_u32(x120[1]))];
    let t284 = [vdupq_n_u64(72340172838076673u64); 4];
    let mut t285: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    t285[0] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t283[0]) as u64).wrapping_mul(vgetq_lane_u64::<0>(t284[0]) as u64) as u64, t285[0]);
    t285[0] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t283[0]) as u64).wrapping_mul(vgetq_lane_u64::<1>(t284[0]) as u64) as u64, t285[0]);
    t285[1] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t283[1]) as u64).wrapping_mul(vgetq_lane_u64::<0>(t284[1]) as u64) as u64, t285[1]);
    t285[1] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t283[1]) as u64).wrapping_mul(vgetq_lane_u64::<1>(t284[1]) as u64) as u64, t285[1]);
    t285[2] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t283[2]) as u64).wrapping_mul(vgetq_lane_u64::<0>(t284[2]) as u64) as u64, t285[2]);
    t285[2] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t283[2]) as u64).wrapping_mul(vgetq_lane_u64::<1>(t284[2]) as u64) as u64, t285[2]);
    t285[3] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t283[3]) as u64).wrapping_mul(vgetq_lane_u64::<0>(t284[3]) as u64) as u64, t285[3]);
    t285[3] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t283[3]) as u64).wrapping_mul(vgetq_lane_u64::<1>(t284[3]) as u64) as u64, t285[3]);
    x136 = t285;
    x144 = x96;
    x152 = x96;
    let mut t286 = live;
    loop {
        // Lanes part: each its own rounds.
        let t288: [uint32x4_t; 2] = [vcltq_u32(x152[0], x104[0]), vcltq_u32(x152[1], x104[1])];
        t286 = [vandq_u32(t286[0], t288[0]), vandq_u32(t286[1], t288[1])];
        if !(any(&t286)) { break; }
        let t290: [uint32x4_t; 2] = [vaddq_u32(x112[0], x152[0]), vaddq_u32(x112[1], x152[1])];
        let t291 = b15_ll_p(l, lid0, t286, t290, x64);
        let t292 = b13_r0r1sls_p(l, lid0, t286, arg0, arg1, x32, t291, 8u32);
        x160 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t286[0])))), t292[0], x160[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t286[0])))), t292[1], x160[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t286[1])))), t292[2], x160[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t286[1])))), t292[3], x160[3])];
        let t293 = [vdupq_n_u64(x128); 4];
        let t294: [uint64x2_t; 4] = [vorrq_u64(x160[0], t293[0]), vorrq_u64(x160[1], t293[1]), vorrq_u64(x160[2], t293[2]), vorrq_u64(x160[3], t293[3])];
        let t295 = [vdupq_n_u64(!(x128)); 4];
        let t296: [uint64x2_t; 4] = [vandq_u64(x136[0], t295[0]), vandq_u64(x136[1], t295[1]), vandq_u64(x136[2], t295[2]), vandq_u64(x136[3], t295[3])];
        let t297: [uint64x2_t; 4] = [vsubq_u64(t294[0], t296[0]), vsubq_u64(t294[1], t296[1]), vsubq_u64(t294[2], t296[2]), vsubq_u64(t294[3], t296[3])];
        x168 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t286[0])))), t297[0], x168[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t286[0])))), t297[1], x168[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t286[1])))), t297[2], x168[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t286[1])))), t297[3], x168[3])];
        let t298: [uint64x2_t; 4] = [vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x160[0]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x160[1]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x160[2]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x160[3])))];
        let t299: [uint64x2_t; 4] = [vandq_u64(t298[0], x136[0]), vandq_u64(t298[1], x136[1]), vandq_u64(t298[2], x136[2]), vandq_u64(t298[3], x136[3])];
        let t300: [uint64x2_t; 4] = [veorq_u64(x160[0], x136[0]), veorq_u64(x160[1], x136[1]), veorq_u64(x160[2], x136[2]), veorq_u64(x160[3], x136[3])];
        let t301: [uint64x2_t; 4] = [vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(t300[0]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(t300[1]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(t300[2]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(t300[3])))];
        let t302: [uint64x2_t; 4] = [vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x168[0]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x168[1]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x168[2]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x168[3])))];
        let t303: [uint64x2_t; 4] = [vandq_u64(t301[0], t302[0]), vandq_u64(t301[1], t302[1]), vandq_u64(t301[2], t302[2]), vandq_u64(t301[3], t302[3])];
        let t304: [uint64x2_t; 4] = [vorrq_u64(t299[0], t303[0]), vorrq_u64(t299[1], t303[1]), vorrq_u64(t299[2], t303[2]), vorrq_u64(t299[3], t303[3])];
        let t305 = [vdupq_n_u64(x128); 4];
        let t306: [uint64x2_t; 4] = [vandq_u64(t305[0], t304[0]), vandq_u64(t305[1], t304[1]), vandq_u64(t305[2], t304[2]), vandq_u64(t305[3], t304[3])];
        x176 = t306;
        let t307: [uint32x4_t; 2] = [vsubq_u32(x104[0], x152[0]), vsubq_u32(x104[1], x152[1])];
        x184 = t307;
        let t308 = [vdupq_n_u32(8u32); 2];
        let t309: [uint32x4_t; 2] = [vcgeq_u32(x184[0], t308[0]), vcgeq_u32(x184[1], t308[1])];
        let mut t310: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
        let t311: [uint32x4_t; 2] = [vandq_u32(t286[0], t309[0]), vandq_u32(t286[1], t309[1])];
        let t312: [uint32x4_t; 2] = [vbicq_u32(t286[0], t309[0]), vbicq_u32(t286[1], t309[1])];
        {
            t310 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t311[0])))), x176[0], t310[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t311[0])))), x176[1], t310[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t311[1])))), x176[2], t310[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t311[1])))), x176[3], t310[3])];
        }
        {
            let t313: [uint32x4_t; 2] = [vmulq_u32(t308[0], x184[0]), vmulq_u32(t308[1], x184[1])];
            let t314 = [vdupq_n_u64(1u64); 4];
            let t315: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t313[0])), vmovl_u32(vget_high_u32(t313[0])), vmovl_u32(vget_low_u32(t313[1])), vmovl_u32(vget_high_u32(t313[1]))];
            let t316: [uint64x2_t; 4] = [vshlq_u64(t314[0], vreinterpretq_s64_u64(vandq_u64(t315[0], vdupq_n_u64(63)))), vshlq_u64(t314[1], vreinterpretq_s64_u64(vandq_u64(t315[1], vdupq_n_u64(63)))), vshlq_u64(t314[2], vreinterpretq_s64_u64(vandq_u64(t315[2], vdupq_n_u64(63)))), vshlq_u64(t314[3], vreinterpretq_s64_u64(vandq_u64(t315[3], vdupq_n_u64(63))))];
            let t317: [uint64x2_t; 4] = [vsubq_u64(t316[0], t314[0]), vsubq_u64(t316[1], t314[1]), vsubq_u64(t316[2], t314[2]), vsubq_u64(t316[3], t314[3])];
            let t318: [uint64x2_t; 4] = [vandq_u64(x176[0], t317[0]), vandq_u64(x176[1], t317[1]), vandq_u64(x176[2], t317[2]), vandq_u64(x176[3], t317[3])];
            t310 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t312[0])))), t318[0], t310[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t312[0])))), t318[1], t310[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t312[1])))), t318[2], t310[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t312[1])))), t318[3], t310[3])];
        }
        let t319 = b16_l_p(l, lid0, t286, t310);
        let t320: [uint32x4_t; 2] = [vaddq_u32(x144[0], t319[0]), vaddq_u32(x144[1], t319[1])];
        x144 = [vbslq_u32(t286[0], t320[0], x144[0]), vbslq_u32(t286[1], t320[1], x144[1])];
        let t321: [uint32x4_t; 2] = [vandq_u32(t286[0], live[0]), vandq_u32(t286[1], live[1])];
        x152 = [vaddq_u32(x152[0], vandq_u32(t321[0], vdupq_n_u32(8))), vaddq_u32(x152[1], vandq_u32(t321[1], vdupq_n_u32(8)))];
    }
    x96 = x144;
    let t322 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    let t323 = [vdupq_n_u32((((t322).wrapping_shr((((((4u32).wrapping_mul(x16)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32))); 2];
    x152 = t323;
    let t324 = [vdupq_n_u32(1u32); 2];
    let t325: [uint32x4_t; 2] = [vsubq_u32(x80[0], t324[0]), vsubq_u32(x80[1], t324[1])];
    let t326: [uint32x4_t; 2] = [vmulq_u32(t273[0], t325[0]), vmulq_u32(t273[1], t325[1])];
    let t327: [uint32x4_t; 2] = [vaddq_u32(x56[0], t326[0]), vaddq_u32(x56[1], t326[1])];
    let t328 = b15_ll_p(l, lid0, live, t327, x64);
    let t329 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t328, 2u32);
    let t330: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t329[0]), vmovn_u64(t329[1])), vcombine_u32(vmovn_u64(t329[2]), vmovn_u64(t329[3]))];
    let t331: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t330[0])), vmovl_u32(vget_high_u32(t330[0])), vmovl_u32(vget_low_u32(t330[1])), vmovl_u32(vget_high_u32(t330[1]))];
    x160 = t331;
    let t332: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x160[0]), vmovn_u64(x160[1])), vcombine_u32(vmovn_u64(x160[2]), vmovn_u64(x160[3]))];
    let t333: [uint32x4_t; 2] = [vaddq_u32(x112[0], t332[0]), vaddq_u32(x112[1], t332[1])];
    let t334: [uint32x4_t; 2] = [vmulq_u32(x96[0], x152[0]), vmulq_u32(x96[1], x152[1])];
    let t335: [uint32x4_t; 2] = [vaddq_u32(t333[0], t334[0]), vaddq_u32(t333[1], t334[1])];
    let t336 = b15_ll_p(l, lid0, live, t335, x64);
    let t337: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t336[0])), vmovl_u32(vget_high_u32(t336[0])), vmovl_u32(vget_low_u32(t336[1])), vmovl_u32(vget_high_u32(t336[1]))];
    x168 = t337;
    let t338: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x168[0]), vmovn_u64(x168[1])), vcombine_u32(vmovn_u64(x168[2]), vmovn_u64(x168[3]))];
    let t339 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t338, 8u32);
    let t340 = b6_l_p(l, lid0, live, x152);
    let t341: [uint64x2_t; 4] = [vandq_u64(t339[0], t340[0]), vandq_u64(t339[1], t340[1]), vandq_u64(t339[2], t340[2]), vandq_u64(t339[3], t340[3])];
    res = t341;
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
    let t342 = b9_r0r1s_p(l, lid0, live, arg0, arg1, (20u32).wrapping_add((4u32).wrapping_mul(x16)));
    res = t342;
    return res;
}

#[inline(always)]
unsafe fn b10_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t343 = b9_r0r1s_f(l, lid0, arg0, arg1, (20u32).wrapping_add((4u32).wrapping_mul(x16)));
    res = t343;
    return res;
}

#[inline(always)]
unsafe fn b11_ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x8: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t344 = [vdupq_n_u32(x8); 2];
    let t345: [uint32x4_t; 2] = [vcltq_u32(x0[0], t344[0]), vcltq_u32(x0[1], t344[1])];
    let mut t346: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t347: [uint32x4_t; 2] = [vandq_u32(live[0], t345[0]), vandq_u32(live[1], t345[1])];
    let t348: [uint32x4_t; 2] = [vbicq_u32(live[0], t345[0]), vbicq_u32(live[1], t345[1])];
    {
        t346 = [vbslq_u32(t347[0], x0[0], t346[0]), vbslq_u32(t347[1], x0[1], t346[1])];
    }
    {
        let t349 = [vdupq_n_u32(x8); 2];
        t346 = [vbslq_u32(t348[0], t349[0], t346[0]), vbslq_u32(t348[1], t349[1], t346[1])];
    }
    res = t346;
    return res;
}

#[inline(always)]
unsafe fn b11_ls_f(l: &Launch, lid0: u32, x0: [uint32x4_t; 2], x8: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t350 = [vdupq_n_u32(x8); 2];
    let t351: [uint32x4_t; 2] = [vcltq_u32(x0[0], t350[0]), vcltq_u32(x0[1], t350[1])];
    let mut t352: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t353: [uint32x4_t; 2] = [vmvnq_u32(t351[0]), vmvnq_u32(t351[1])];
    {
        t352 = [vbslq_u32(t351[0], x0[0], t352[0]), vbslq_u32(t351[1], x0[1], t352[1])];
    }
    {
        let t354 = [vdupq_n_u32(x8); 2];
        t352 = [vbslq_u32(t353[0], t354[0], t352[0]), vbslq_u32(t353[1], t354[1], t352[1])];
    }
    res = t352;
    return res;
}

#[inline(always)]
unsafe fn b12_r0lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: [uint32x4_t; 2], x16: [uint32x4_t; 2], x24: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t355: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x8[0])), vmovl_u32(vget_high_u32(x8[0])), vmovl_u32(vget_low_u32(x8[1])), vmovl_u32(vget_high_u32(x8[1]))];
    let t356: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x16[0])), vmovl_u32(vget_high_u32(x16[0])), vmovl_u32(vget_low_u32(x16[1])), vmovl_u32(vget_high_u32(x16[1]))];
    let t357: [uint64x2_t; 4] = [vaddq_u64(t355[0], t356[0]), vaddq_u64(t355[1], t356[1]), vaddq_u64(t355[2], t356[2]), vaddq_u64(t355[3], t356[3])];
    let t358 = b17_r0ls_p(l, lid0, live, arg0, t357, x24);
    x32 = t358;
    let t359: [uint64x2_t; 4];
    if ((x24) >= (8u32)) {
        t359 = x32;
    } else {
        let t360 = b18_s_p(l, lid0, live, (8u32).wrapping_mul(x24));
        let t361 = [vdupq_n_u64(t360); 4];
        let t362: [uint64x2_t; 4] = [vandq_u64(x32[0], t361[0]), vandq_u64(x32[1], t361[1]), vandq_u64(x32[2], t361[2]), vandq_u64(x32[3], t361[3])];
        t359 = t362;
    }
    res = t359;
    return res;
}

#[inline(always)]
unsafe fn b12_r0lls_f(l: &Launch, lid0: u32, arg0: *const u8, x8: [uint32x4_t; 2], x16: [uint32x4_t; 2], x24: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t363: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x8[0])), vmovl_u32(vget_high_u32(x8[0])), vmovl_u32(vget_low_u32(x8[1])), vmovl_u32(vget_high_u32(x8[1]))];
    let t364: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x16[0])), vmovl_u32(vget_high_u32(x16[0])), vmovl_u32(vget_low_u32(x16[1])), vmovl_u32(vget_high_u32(x16[1]))];
    let t365: [uint64x2_t; 4] = [vaddq_u64(t363[0], t364[0]), vaddq_u64(t363[1], t364[1]), vaddq_u64(t363[2], t364[2]), vaddq_u64(t363[3], t364[3])];
    let t366 = b17_r0ls_f(l, lid0, arg0, t365, x24);
    x32 = t366;
    let t367: [uint64x2_t; 4];
    if ((x24) >= (8u32)) {
        t367 = x32;
    } else {
        let t368 = b18_s_f(l, lid0, (8u32).wrapping_mul(x24));
        let t369 = [vdupq_n_u64(t368); 4];
        let t370: [uint64x2_t; 4] = [vandq_u64(x32[0], t369[0]), vandq_u64(x32[1], t369[1]), vandq_u64(x32[2], t369[2]), vandq_u64(x32[3], t369[3])];
        t367 = t370;
    }
    res = t367;
    return res;
}

#[inline(always)]
unsafe fn b13_r0r1sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2], x32: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t371 = b19_r0sls_p(l, lid0, live, arg0, x16, x24, x32);
    res = t371;
    return res;
}

#[inline(always)]
unsafe fn b14_sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x8: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t372 = [vdupq_n_u32(x0); 2];
    let t373: [uint32x4_t; 2] = [vcltq_u32(t372[0], x8[0]), vcltq_u32(t372[1], x8[1])];
    let mut t374: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t375: [uint32x4_t; 2] = [vandq_u32(live[0], t373[0]), vandq_u32(live[1], t373[1])];
    let t376: [uint32x4_t; 2] = [vbicq_u32(live[0], t373[0]), vbicq_u32(live[1], t373[1])];
    {
        let t377 = [vdupq_n_u32(x0); 2];
        t374 = [vbslq_u32(t375[0], t377[0], t374[0]), vbslq_u32(t375[1], t377[1], t374[1])];
    }
    {
        t374 = [vbslq_u32(t376[0], x8[0], t374[0]), vbslq_u32(t376[1], x8[1], t374[1])];
    }
    res = t374;
    return res;
}

#[inline(always)]
unsafe fn b15_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x8: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t378: [uint32x4_t; 2] = [vcltq_u32(x0[0], x8[0]), vcltq_u32(x0[1], x8[1])];
    let mut t379: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t380: [uint32x4_t; 2] = [vandq_u32(live[0], t378[0]), vandq_u32(live[1], t378[1])];
    let t381: [uint32x4_t; 2] = [vbicq_u32(live[0], t378[0]), vbicq_u32(live[1], t378[1])];
    {
        t379 = [vbslq_u32(t380[0], x0[0], t379[0]), vbslq_u32(t380[1], x0[1], t379[1])];
    }
    {
        t379 = [vbslq_u32(t381[0], x8[0], t379[0]), vbslq_u32(t381[1], x8[1], t379[1])];
    }
    res = t379;
    return res;
}

#[inline(always)]
unsafe fn b16_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint64x2_t; 4]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t382: [uint64x2_t; 4] = [vpaddlq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u64(x0[0]))))), vpaddlq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u64(x0[1]))))), vpaddlq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u64(x0[2]))))), vpaddlq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u64(x0[3])))))];
    let t383: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t382[0]), vmovn_u64(t382[1])), vcombine_u32(vmovn_u64(t382[2]), vmovn_u64(t382[3]))];
    res = t383;
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
    let t384 = [vdupq_n_u64(!(7u64)); 4];
    let t385: [uint64x2_t; 4] = [vandq_u64(x8[0], t384[0]), vandq_u64(x8[1], t384[1]), vandq_u64(x8[2], t384[2]), vandq_u64(x8[3], t384[3])];
    let t386: [uint64x2_t; 4] = [vandq_u64(t385[0], vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0]))))), vandq_u64(t385[1], vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0]))))), vandq_u64(t385[2], vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1]))))), vandq_u64(t385[3], vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))))];
    let t387: [uint64x2_t; 4] = [vaddq_u64(vdupq_n_u64((arg0) as u64), t386[0]), vaddq_u64(vdupq_n_u64((arg0) as u64), t386[1]), vaddq_u64(vdupq_n_u64((arg0) as u64), t386[2]), vaddq_u64(vdupq_n_u64((arg0) as u64), t386[3])];
    x24 = t387;
    let t388 = [vdupq_n_u64(7u64); 4];
    let t389: [uint64x2_t; 4] = [vandq_u64(x8[0], t388[0]), vandq_u64(x8[1], t388[1]), vandq_u64(x8[2], t388[2]), vandq_u64(x8[3], t388[3])];
    let t390: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t389[0]), vmovn_u64(t389[1])), vcombine_u32(vmovn_u64(t389[2]), vmovn_u64(t389[3]))];
    let t391 = [vdupq_n_u32(8u32); 2];
    let t392: [uint32x4_t; 2] = [vmulq_u32(t390[0], t391[0]), vmulq_u32(t390[1], t391[1])];
    x32 = t392;
    let mut t393: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    t393[0] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[0]) as usize as *const u64).read_unaligned(), t393[0]);
    t393[0] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[0]) as usize as *const u64).read_unaligned(), t393[0]);
    t393[1] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[1]) as usize as *const u64).read_unaligned(), t393[1]);
    t393[1] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[1]) as usize as *const u64).read_unaligned(), t393[1]);
    t393[2] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[2]) as usize as *const u64).read_unaligned(), t393[2]);
    t393[2] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[2]) as usize as *const u64).read_unaligned(), t393[2]);
    t393[3] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[3]) as usize as *const u64).read_unaligned(), t393[3]);
    t393[3] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[3]) as usize as *const u64).read_unaligned(), t393[3]);
    let t394: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x32[0])), vmovl_u32(vget_high_u32(x32[0])), vmovl_u32(vget_low_u32(x32[1])), vmovl_u32(vget_high_u32(x32[1]))];
    let t395: [uint64x2_t; 4] = [vshlq_u64(t393[0], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t394[0], vdupq_n_u64(63))))), vshlq_u64(t393[1], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t394[1], vdupq_n_u64(63))))), vshlq_u64(t393[2], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t394[2], vdupq_n_u64(63))))), vshlq_u64(t393[3], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t394[3], vdupq_n_u64(63)))))];
    x40 = t395;
    let t396 = [vdupq_n_u32(0u32); 2];
    let t397: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t396[0])), vmvnq_u32(vceqq_u32(x32[1], t396[1]))];
    let t398 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t399: [uint32x4_t; 2] = [vaddq_u32(x32[0], t398[0]), vaddq_u32(x32[1], t398[1])];
    let t400 = [vdupq_n_u32(64u32); 2];
    let t401: [uint32x4_t; 2] = [vcgtq_u32(t399[0], t400[0]), vcgtq_u32(t399[1], t400[1])];
    let t402: [uint32x4_t; 2] = [vandq_u32(t397[0], t401[0]), vandq_u32(t397[1], t401[1])];
    let mut t403: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t404: [uint32x4_t; 2] = [vandq_u32(live[0], t402[0]), vandq_u32(live[1], t402[1])];
    let t405: [uint32x4_t; 2] = [vbicq_u32(live[0], t402[0]), vbicq_u32(live[1], t402[1])];
    if any(&t404) {
        let t406 = [vdupq_n_u64((1u64).wrapping_mul(8u64)); 4];
        let t407: [uint64x2_t; 4] = [vaddq_u64(x24[0], t406[0]), vaddq_u64(x24[1], t406[1]), vaddq_u64(x24[2], t406[2]), vaddq_u64(x24[3], t406[3])];
        let mut t408: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
        t408[0] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t407[0]) as usize as *const u64).read_unaligned(), t408[0]);
        t408[0] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t407[0]) as usize as *const u64).read_unaligned(), t408[0]);
        t408[1] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t407[1]) as usize as *const u64).read_unaligned(), t408[1]);
        t408[1] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t407[1]) as usize as *const u64).read_unaligned(), t408[1]);
        t408[2] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t407[2]) as usize as *const u64).read_unaligned(), t408[2]);
        t408[2] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t407[2]) as usize as *const u64).read_unaligned(), t408[2]);
        t408[3] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t407[3]) as usize as *const u64).read_unaligned(), t408[3]);
        t408[3] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t407[3]) as usize as *const u64).read_unaligned(), t408[3]);
        let t409: [uint32x4_t; 2] = [vsubq_u32(t400[0], x32[0]), vsubq_u32(t400[1], x32[1])];
        let t410: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t409[0])), vmovl_u32(vget_high_u32(t409[0])), vmovl_u32(vget_low_u32(t409[1])), vmovl_u32(vget_high_u32(t409[1]))];
        let t411: [uint64x2_t; 4] = [vshlq_u64(t408[0], vreinterpretq_s64_u64(vandq_u64(t410[0], vdupq_n_u64(63)))), vshlq_u64(t408[1], vreinterpretq_s64_u64(vandq_u64(t410[1], vdupq_n_u64(63)))), vshlq_u64(t408[2], vreinterpretq_s64_u64(vandq_u64(t410[2], vdupq_n_u64(63)))), vshlq_u64(t408[3], vreinterpretq_s64_u64(vandq_u64(t410[3], vdupq_n_u64(63))))];
        let t412: [uint64x2_t; 4] = [vorrq_u64(x40[0], t411[0]), vorrq_u64(x40[1], t411[1]), vorrq_u64(x40[2], t411[2]), vorrq_u64(x40[3], t411[3])];
        t403 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t404[0])))), t412[0], t403[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t404[0])))), t412[1], t403[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t404[1])))), t412[2], t403[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t404[1])))), t412[3], t403[3])];
    }
    {
        t403 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t405[0])))), x40[0], t403[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t405[0])))), x40[1], t403[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t405[1])))), x40[2], t403[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t405[1])))), x40[3], t403[3])];
    }
    res = t403;
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
    let t413 = [vdupq_n_u64(!(7u64)); 4];
    let t414: [uint64x2_t; 4] = [vandq_u64(x8[0], t413[0]), vandq_u64(x8[1], t413[1]), vandq_u64(x8[2], t413[2]), vandq_u64(x8[3], t413[3])];
    let t415: [uint64x2_t; 4] = [vaddq_u64(vdupq_n_u64((arg0) as u64), t414[0]), vaddq_u64(vdupq_n_u64((arg0) as u64), t414[1]), vaddq_u64(vdupq_n_u64((arg0) as u64), t414[2]), vaddq_u64(vdupq_n_u64((arg0) as u64), t414[3])];
    x24 = t415;
    let t416 = [vdupq_n_u64(7u64); 4];
    let t417: [uint64x2_t; 4] = [vandq_u64(x8[0], t416[0]), vandq_u64(x8[1], t416[1]), vandq_u64(x8[2], t416[2]), vandq_u64(x8[3], t416[3])];
    let t418: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t417[0]), vmovn_u64(t417[1])), vcombine_u32(vmovn_u64(t417[2]), vmovn_u64(t417[3]))];
    let t419 = [vdupq_n_u32(8u32); 2];
    let t420: [uint32x4_t; 2] = [vmulq_u32(t418[0], t419[0]), vmulq_u32(t418[1], t419[1])];
    x32 = t420;
    let mut t421: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    t421[0] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[0]) as usize as *const u64).read_unaligned(), t421[0]);
    t421[0] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[0]) as usize as *const u64).read_unaligned(), t421[0]);
    t421[1] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[1]) as usize as *const u64).read_unaligned(), t421[1]);
    t421[1] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[1]) as usize as *const u64).read_unaligned(), t421[1]);
    t421[2] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[2]) as usize as *const u64).read_unaligned(), t421[2]);
    t421[2] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[2]) as usize as *const u64).read_unaligned(), t421[2]);
    t421[3] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[3]) as usize as *const u64).read_unaligned(), t421[3]);
    t421[3] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[3]) as usize as *const u64).read_unaligned(), t421[3]);
    let t422: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x32[0])), vmovl_u32(vget_high_u32(x32[0])), vmovl_u32(vget_low_u32(x32[1])), vmovl_u32(vget_high_u32(x32[1]))];
    let t423: [uint64x2_t; 4] = [vshlq_u64(t421[0], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t422[0], vdupq_n_u64(63))))), vshlq_u64(t421[1], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t422[1], vdupq_n_u64(63))))), vshlq_u64(t421[2], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t422[2], vdupq_n_u64(63))))), vshlq_u64(t421[3], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t422[3], vdupq_n_u64(63)))))];
    x40 = t423;
    let t424 = [vdupq_n_u32(0u32); 2];
    let t425: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t424[0])), vmvnq_u32(vceqq_u32(x32[1], t424[1]))];
    let t426 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t427: [uint32x4_t; 2] = [vaddq_u32(x32[0], t426[0]), vaddq_u32(x32[1], t426[1])];
    let t428 = [vdupq_n_u32(64u32); 2];
    let t429: [uint32x4_t; 2] = [vcgtq_u32(t427[0], t428[0]), vcgtq_u32(t427[1], t428[1])];
    let t430: [uint32x4_t; 2] = [vandq_u32(t425[0], t429[0]), vandq_u32(t425[1], t429[1])];
    let mut t431: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t432: [uint32x4_t; 2] = [vmvnq_u32(t430[0]), vmvnq_u32(t430[1])];
    if any(&t430) {
        let t433 = [vdupq_n_u64((1u64).wrapping_mul(8u64)); 4];
        let t434: [uint64x2_t; 4] = [vaddq_u64(x24[0], t433[0]), vaddq_u64(x24[1], t433[1]), vaddq_u64(x24[2], t433[2]), vaddq_u64(x24[3], t433[3])];
        let mut t435: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
        t435[0] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t434[0]) as usize as *const u64).read_unaligned(), t435[0]);
        t435[0] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t434[0]) as usize as *const u64).read_unaligned(), t435[0]);
        t435[1] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t434[1]) as usize as *const u64).read_unaligned(), t435[1]);
        t435[1] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t434[1]) as usize as *const u64).read_unaligned(), t435[1]);
        t435[2] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t434[2]) as usize as *const u64).read_unaligned(), t435[2]);
        t435[2] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t434[2]) as usize as *const u64).read_unaligned(), t435[2]);
        t435[3] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t434[3]) as usize as *const u64).read_unaligned(), t435[3]);
        t435[3] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t434[3]) as usize as *const u64).read_unaligned(), t435[3]);
        let t436: [uint32x4_t; 2] = [vsubq_u32(t428[0], x32[0]), vsubq_u32(t428[1], x32[1])];
        let t437: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t436[0])), vmovl_u32(vget_high_u32(t436[0])), vmovl_u32(vget_low_u32(t436[1])), vmovl_u32(vget_high_u32(t436[1]))];
        let t438: [uint64x2_t; 4] = [vshlq_u64(t435[0], vreinterpretq_s64_u64(vandq_u64(t437[0], vdupq_n_u64(63)))), vshlq_u64(t435[1], vreinterpretq_s64_u64(vandq_u64(t437[1], vdupq_n_u64(63)))), vshlq_u64(t435[2], vreinterpretq_s64_u64(vandq_u64(t437[2], vdupq_n_u64(63)))), vshlq_u64(t435[3], vreinterpretq_s64_u64(vandq_u64(t437[3], vdupq_n_u64(63))))];
        let t439: [uint64x2_t; 4] = [vorrq_u64(x40[0], t438[0]), vorrq_u64(x40[1], t438[1]), vorrq_u64(x40[2], t438[2]), vorrq_u64(x40[3], t438[3])];
        t431 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t430[0])))), t439[0], t431[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t430[0])))), t439[1], t431[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t430[1])))), t439[2], t431[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t430[1])))), t439[3], t431[3])];
    }
    {
        t431 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t432[0])))), x40[0], t431[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t432[0])))), x40[1], t431[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t432[1])))), x40[2], t431[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t432[1])))), x40[3], t431[3])];
    }
    res = t431;
    return res;
}

#[inline(always)]
unsafe fn b18_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t440: u64;
    if ((x0) >= (64u32)) {
        t440 = !(0u64);
    } else {
        t440 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t440;
    return res;
}

#[inline(always)]
unsafe fn b18_s_f(l: &Launch, lid0: u32, x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t441: u64;
    if ((x0) >= (64u32)) {
        t441 = !(0u64);
    } else {
        t441 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t441;
    return res;
}

#[inline(always)]
unsafe fn b19_r0sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: u32, x16: [uint32x4_t; 2], x24: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t442 = [vdupq_n_u64(((x8) as u64)); 4];
    let t443: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x16[0])), vmovl_u32(vget_high_u32(x16[0])), vmovl_u32(vget_low_u32(x16[1])), vmovl_u32(vget_high_u32(x16[1]))];
    let t444: [uint64x2_t; 4] = [vaddq_u64(t442[0], t443[0]), vaddq_u64(t442[1], t443[1]), vaddq_u64(t442[2], t443[2]), vaddq_u64(t442[3], t443[3])];
    let t445 = b17_r0ls_p(l, lid0, live, arg0, t444, x24);
    x32 = t445;
    let t446: [uint64x2_t; 4];
    if ((x24) >= (8u32)) {
        t446 = x32;
    } else {
        let t447 = b18_s_p(l, lid0, live, (8u32).wrapping_mul(x24));
        let t448 = [vdupq_n_u64(t447); 4];
        let t449: [uint64x2_t; 4] = [vandq_u64(x32[0], t448[0]), vandq_u64(x32[1], t448[1]), vandq_u64(x32[2], t448[2]), vandq_u64(x32[3], t448[3])];
        t446 = t449;
    }
    res = t446;
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
    let x24: *const u8 = *argv.add(3);
    let arg4: *const u8 = (*argv.add(4) as *const *const u8).read_unaligned();
    let x48: u32 = (*argv.add(5) as *const u32).read_unaligned();
    let mut t450: bool = !(arg4).is_null();
    if t450 {
        let mut t451: bool = ((l.group[0]) >= ((arg4.wrapping_add((((x48) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        if !t451 {
            t451 = ((l.group[1]) >= ((arg4.wrapping_add(((((x48).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        let mut t452: bool = t451;
        if !t452 {
            t452 = ((l.group[2]) >= ((arg4.wrapping_add(((((x48).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        t450 = t452;
    }
    if t450 {
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
        let t453: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
        let mut live = t453;
        let gid0 = l.group[0] * l.block[0] + lid0;
        'chunk: {
            b0_r0r1r2sg_p(l, lid0, live, arg0, arg1, arg2, x24);
        }
    }
}
