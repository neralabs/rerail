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
unsafe fn b0_r0r1r2r3sg_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, arg2: *const u8, arg3: *const u8, x32: *const u8) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x32 as *const u32).read_unaligned();
    let mut x56: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
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
        x56 = t5;
        let t6: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x56[0]), vmovn_u64(x56[1])), vcombine_u32(vmovn_u64(x56[2]), vmovn_u64(x56[3]))];
        let t7 = (arg2 as *mut u32).add((gid0) as usize);
        if all(&live) {
            vst1q_u32(t7.add(0), t6[0]);
            vst1q_u32(t7.add(4), t6[1]);
        } else {
            let mut t8w = 0u32;
            let t8 = core::hint::black_box(&raw mut t8w);
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t7.add(0), t8).write(vgetq_lane_u32::<0>(t6[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t7.add(1), t8).write(vgetq_lane_u32::<1>(t6[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t7.add(2), t8).write(vgetq_lane_u32::<2>(t6[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t7.add(3), t8).write(vgetq_lane_u32::<3>(t6[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t7.add(4), t8).write(vgetq_lane_u32::<0>(t6[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t7.add(5), t8).write(vgetq_lane_u32::<1>(t6[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t7.add(6), t8).write(vgetq_lane_u32::<2>(t6[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t7.add(7), t8).write(vgetq_lane_u32::<3>(t6[1]));
        }
        let t9 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
        let t10: [uint64x2_t; 4] = [vandq_u64(x56[0], t9[0]), vandq_u64(x56[1], t9[1]), vandq_u64(x56[2], t9[2]), vandq_u64(x56[3], t9[3])];
        let t11 = [vdupq_n_u64(0u64); 4];
        let t12: [uint64x2_t; 4] = [vceqq_u64(t10[0], t11[0]), vceqq_u64(t10[1], t11[1]), vceqq_u64(t10[2], t11[2]), vceqq_u64(t10[3], t11[3])];
        let t13: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t12[0]), vmovn_u64(t12[1])), vcombine_u32(vmovn_u64(t12[2]), vmovn_u64(t12[3]))];
        let t14: [uint32x4_t; 2] = [vmvnq_u32(t13[0]), vmvnq_u32(t13[1])];
        let mut t15: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t16: [uint32x4_t; 2] = [vandq_u32(live[0], t14[0]), vandq_u32(live[1], t14[1])];
        let t17: [uint32x4_t; 2] = [vbicq_u32(live[0], t14[0]), vbicq_u32(live[1], t14[1])];
        {
            let t18: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x56[0]), vmovn_u64(x56[1])), vcombine_u32(vmovn_u64(x56[2]), vmovn_u64(x56[3]))];
            t15 = [vbslq_u32(t16[0], t18[0], t15[0]), vbslq_u32(t16[1], t18[1], t15[1])];
        }
        {
            let t19 = [vdupq_n_u32(4294967295u32); 2];
            t15 = [vbslq_u32(t17[0], t19[0], t15[0]), vbslq_u32(t17[1], t19[1], t15[1])];
        }
        let t20 = (arg3 as *mut u32).add((gid0) as usize);
        if all(&live) {
            vst1q_u32(t20.add(0), t15[0]);
            vst1q_u32(t20.add(4), t15[1]);
        } else {
            let mut t21w = 0u32;
            let t21 = core::hint::black_box(&raw mut t21w);
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t20.add(0), t21).write(vgetq_lane_u32::<0>(t15[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t20.add(1), t21).write(vgetq_lane_u32::<1>(t15[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t20.add(2), t21).write(vgetq_lane_u32::<2>(t15[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t20.add(3), t21).write(vgetq_lane_u32::<3>(t15[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t20.add(4), t21).write(vgetq_lane_u32::<0>(t15[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t20.add(5), t21).write(vgetq_lane_u32::<1>(t15[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t20.add(6), t21).write(vgetq_lane_u32::<2>(t15[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t20.add(7), t21).write(vgetq_lane_u32::<3>(t15[1]));
        }
    }
}

#[inline(always)]
unsafe fn b0_r0r1r2r3sg_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, arg2: *const u8, arg3: *const u8, x32: *const u8) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x32 as *const u32).read_unaligned();
    let mut x56: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    if ((gid0) as u64 + 7) < ((a0) as u64) {
        let t22 = b1_r0r1g_f(l, lid0, arg0, arg1);
        x56 = t22;
        let t23: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x56[0]), vmovn_u64(x56[1])), vcombine_u32(vmovn_u64(x56[2]), vmovn_u64(x56[3]))];
        let t24 = (arg2 as *mut u32).add((gid0) as usize);
        vst1q_u32(t24.add(0), t23[0]);
        vst1q_u32(t24.add(4), t23[1]);
        let t25 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
        let t26: [uint64x2_t; 4] = [vandq_u64(x56[0], t25[0]), vandq_u64(x56[1], t25[1]), vandq_u64(x56[2], t25[2]), vandq_u64(x56[3], t25[3])];
        let t27 = [vdupq_n_u64(0u64); 4];
        let t28: [uint64x2_t; 4] = [vceqq_u64(t26[0], t27[0]), vceqq_u64(t26[1], t27[1]), vceqq_u64(t26[2], t27[2]), vceqq_u64(t26[3], t27[3])];
        let t29: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t28[0]), vmovn_u64(t28[1])), vcombine_u32(vmovn_u64(t28[2]), vmovn_u64(t28[3]))];
        let t30: [uint32x4_t; 2] = [vmvnq_u32(t29[0]), vmvnq_u32(t29[1])];
        let mut t31: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t32: [uint32x4_t; 2] = [vmvnq_u32(t30[0]), vmvnq_u32(t30[1])];
        {
            let t33: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x56[0]), vmovn_u64(x56[1])), vcombine_u32(vmovn_u64(x56[2]), vmovn_u64(x56[3]))];
            t31 = [vbslq_u32(t30[0], t33[0], t31[0]), vbslq_u32(t30[1], t33[1], t31[1])];
        }
        {
            let t34 = [vdupq_n_u32(4294967295u32); 2];
            t31 = [vbslq_u32(t32[0], t34[0], t31[0]), vbslq_u32(t32[1], t34[1], t31[1])];
        }
        let t35 = (arg3 as *mut u32).add((gid0) as usize);
        vst1q_u32(t35.add(0), t31[0]);
        vst1q_u32(t35.add(4), t31[1]);
    } else if (((gid0) as u64 + 7) <= u32::MAX as u64 && ((gid0) as u64) >= ((a0) as u64)) {
        return;
    } else {
        // Lanes part: the chunk's lanes on both sides of the bound.
        let t36: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t37 = [vdupq_n_u32(a0); 2];
        let t38: [uint32x4_t; 2] = [vcgeq_u32(t36[0], t37[0]), vcgeq_u32(t36[1], t37[1])];
        let t39: [uint32x4_t; 2] = [vandq_u32(live[0], t38[0]), vandq_u32(live[1], t38[1])];
        if any(&t39) {
            live = [vbicq_u32(live[0], t39[0]), vbicq_u32(live[1], t39[1])];
        }
        if any(&live) {
            let t40 = b1_r0r1g_p(l, lid0, live, arg0, arg1);
            x56 = t40;
            let t41: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x56[0]), vmovn_u64(x56[1])), vcombine_u32(vmovn_u64(x56[2]), vmovn_u64(x56[3]))];
            let t42 = (arg2 as *mut u32).add((gid0) as usize);
            if all(&live) {
                vst1q_u32(t42.add(0), t41[0]);
                vst1q_u32(t42.add(4), t41[1]);
            } else {
                let mut t43w = 0u32;
                let t43 = core::hint::black_box(&raw mut t43w);
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t42.add(0), t43).write(vgetq_lane_u32::<0>(t41[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t42.add(1), t43).write(vgetq_lane_u32::<1>(t41[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t42.add(2), t43).write(vgetq_lane_u32::<2>(t41[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t42.add(3), t43).write(vgetq_lane_u32::<3>(t41[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t42.add(4), t43).write(vgetq_lane_u32::<0>(t41[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t42.add(5), t43).write(vgetq_lane_u32::<1>(t41[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t42.add(6), t43).write(vgetq_lane_u32::<2>(t41[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t42.add(7), t43).write(vgetq_lane_u32::<3>(t41[1]));
            }
            let t44 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
            let t45: [uint64x2_t; 4] = [vandq_u64(x56[0], t44[0]), vandq_u64(x56[1], t44[1]), vandq_u64(x56[2], t44[2]), vandq_u64(x56[3], t44[3])];
            let t46 = [vdupq_n_u64(0u64); 4];
            let t47: [uint64x2_t; 4] = [vceqq_u64(t45[0], t46[0]), vceqq_u64(t45[1], t46[1]), vceqq_u64(t45[2], t46[2]), vceqq_u64(t45[3], t46[3])];
            let t48: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t47[0]), vmovn_u64(t47[1])), vcombine_u32(vmovn_u64(t47[2]), vmovn_u64(t47[3]))];
            let t49: [uint32x4_t; 2] = [vmvnq_u32(t48[0]), vmvnq_u32(t48[1])];
            let mut t50: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t51: [uint32x4_t; 2] = [vandq_u32(live[0], t49[0]), vandq_u32(live[1], t49[1])];
            let t52: [uint32x4_t; 2] = [vbicq_u32(live[0], t49[0]), vbicq_u32(live[1], t49[1])];
            {
                let t53: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x56[0]), vmovn_u64(x56[1])), vcombine_u32(vmovn_u64(x56[2]), vmovn_u64(x56[3]))];
                t50 = [vbslq_u32(t51[0], t53[0], t50[0]), vbslq_u32(t51[1], t53[1], t50[1])];
            }
            {
                let t54 = [vdupq_n_u32(4294967295u32); 2];
                t50 = [vbslq_u32(t52[0], t54[0], t50[0]), vbslq_u32(t52[1], t54[1], t50[1])];
            }
            let t55 = (arg3 as *mut u32).add((gid0) as usize);
            if all(&live) {
                vst1q_u32(t55.add(0), t50[0]);
                vst1q_u32(t55.add(4), t50[1]);
            } else {
                let mut t56w = 0u32;
                let t56 = core::hint::black_box(&raw mut t56w);
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t55.add(0), t56).write(vgetq_lane_u32::<0>(t50[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t55.add(1), t56).write(vgetq_lane_u32::<1>(t50[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t55.add(2), t56).write(vgetq_lane_u32::<2>(t50[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t55.add(3), t56).write(vgetq_lane_u32::<3>(t50[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t55.add(4), t56).write(vgetq_lane_u32::<0>(t50[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t55.add(5), t56).write(vgetq_lane_u32::<1>(t50[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t55.add(6), t56).write(vgetq_lane_u32::<2>(t50[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t55.add(7), t56).write(vgetq_lane_u32::<3>(t50[1]));
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
    let t57: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
    let t58 = [vdupq_n_u32(0u32); 2];
    let t59: [uint32x4_t; 2] = [vceqq_u32(t57[0], t58[0]), vceqq_u32(t57[1], t58[1])];
    let t60: [uint32x4_t; 2] = [vandq_u32(live[0], t59[0]), vandq_u32(live[1], t59[1])];
    if any(&t60) {
        let t61 = [vdupq_n_u64(4294967295u64); 4];
        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t60[0])))), t61[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t60[0])))), t61[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t60[1])))), t61[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t60[1])))), t61[3], res[3])];
        live = [vbicq_u32(live[0], t60[0]), vbicq_u32(live[1], t60[1])];
    }
    if any(&live) {
        let t62 = [vdupq_n_u32(1u32); 2];
        let t63: [uint32x4_t; 2] = [vsubq_u32(t57[0], t62[0]), vsubq_u32(t57[1], t62[1])];
        x24 = [vbslq_u32(live[0], t63[0], x24[0]), vbslq_u32(live[1], t63[1], x24[1])];
        let t64 = b2_r0r1l_p(l, lid0, live, arg0, arg1, x24);
        x32 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t64[0], x32[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t64[1], x32[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t64[2], x32[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t64[3], x32[3])];
        let t65 = [vdupq_n_u64(1u64); 4];
        let t66: [uint64x2_t; 4] = [vandq_u64(x32[0], t65[0]), vandq_u64(x32[1], t65[1]), vandq_u64(x32[2], t65[2]), vandq_u64(x32[3], t65[3])];
        let t67 = [vdupq_n_u64(0u64); 4];
        let t68: [uint64x2_t; 4] = [vceqq_u64(t66[0], t67[0]), vceqq_u64(t66[1], t67[1]), vceqq_u64(t66[2], t67[2]), vceqq_u64(t66[3], t67[3])];
        let t69: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t68[0]), vmovn_u64(t68[1])), vcombine_u32(vmovn_u64(t68[2]), vmovn_u64(t68[3]))];
        let t70: [uint32x4_t; 2] = [vandq_u32(live[0], t69[0]), vandq_u32(live[1], t69[1])];
        if any(&t70) {
            let t71: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x24[0])), vmovl_u32(vget_high_u32(x24[0])), vmovl_u32(vget_low_u32(x24[1])), vmovl_u32(vget_high_u32(x24[1]))];
            res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t70[0])))), t71[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t70[0])))), t71[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t70[1])))), t71[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t70[1])))), t71[3], res[3])];
            live = [vbicq_u32(live[0], t70[0]), vbicq_u32(live[1], t70[1])];
        }
        if any(&live) {
            let t72: [uint64x2_t; 4] = [vshlq_u64(x32[0], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[1], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[2], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[3], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64)))];
            let t73: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t72[0]), vmovn_u64(t72[1])), vcombine_u32(vmovn_u64(t72[2]), vmovn_u64(t72[3]))];
            x40 = [vbslq_u32(live[0], t73[0], x40[0]), vbslq_u32(live[1], t73[1], x40[1])];
            let t74: [uint32x4_t; 2] = [vsubq_u32(x40[0], t62[0]), vsubq_u32(x40[1], t62[1])];
            let t75: [uint32x4_t; 2] = [vcgeq_u32(t74[0], t57[0]), vcgeq_u32(t74[1], t57[1])];
            let t76: [uint32x4_t; 2] = [vandq_u32(live[0], t75[0]), vandq_u32(live[1], t75[1])];
            if any(&t76) {
                let t77 = [vdupq_n_u64(4294967295u64); 4];
                res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t76[0])))), t77[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t76[0])))), t77[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t76[1])))), t77[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t76[1])))), t77[3], res[3])];
                live = [vbicq_u32(live[0], t76[0]), vbicq_u32(live[1], t76[1])];
            }
            if any(&live) {
                let t78: [uint32x4_t; 2] = [vsubq_u32(t57[0], x40[0]), vsubq_u32(t57[1], x40[1])];
                x48 = [vbslq_u32(live[0], t78[0], x48[0]), vbslq_u32(live[1], t78[1], x48[1])];
                let t79 = [vdupq_n_u64(2u64); 4];
                let t80: [uint64x2_t; 4] = [vandq_u64(x32[0], t79[0]), vandq_u64(x32[1], t79[1]), vandq_u64(x32[2], t79[2]), vandq_u64(x32[3], t79[3])];
                let t81: [uint64x2_t; 4] = [vceqq_u64(t80[0], t67[0]), vceqq_u64(t80[1], t67[1]), vceqq_u64(t80[2], t67[2]), vceqq_u64(t80[3], t67[3])];
                let t82: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t81[0]), vmovn_u64(t81[1])), vcombine_u32(vmovn_u64(t81[2]), vmovn_u64(t81[3]))];
                let t83: [uint32x4_t; 2] = [vmvnq_u32(t82[0]), vmvnq_u32(t82[1])];
                let t84: [uint32x4_t; 2] = [vandq_u32(live[0], t83[0]), vandq_u32(live[1], t83[1])];
                if any(&t84) {
                    let t85: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                    res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t84[0])))), t85[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t84[0])))), t85[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t84[1])))), t85[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t84[1])))), t85[3], res[3])];
                    live = [vbicq_u32(live[0], t84[0]), vbicq_u32(live[1], t84[1])];
                }
                if any(&live) {
                    let t86: [uint32x4_t; 2] = [vceqq_u32(x48[0], t58[0]), vceqq_u32(x48[1], t58[1])];
                    let t87: [uint32x4_t; 2] = [vandq_u32(live[0], t86[0]), vandq_u32(live[1], t86[1])];
                    if any(&t87) {
                        let t88 = [vdupq_n_u64(4294967295u64); 4];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t87[0])))), t88[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t87[0])))), t88[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t87[1])))), t88[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t87[1])))), t88[3], res[3])];
                        live = [vbicq_u32(live[0], t87[0]), vbicq_u32(live[1], t87[1])];
                    }
                    if any(&live) {
                        let t89 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
                        let t90: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                        let t91: [uint64x2_t; 4] = [vorrq_u64(t89[0], t90[0]), vorrq_u64(t89[1], t90[1]), vorrq_u64(t89[2], t90[2]), vorrq_u64(t89[3], t90[3])];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t91[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t91[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t91[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t91[3], res[3])];
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
        let t92: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t93 = [vdupq_n_u32(1u32); 2];
        let t94: [uint32x4_t; 2] = [vsubq_u32(t92[0], t93[0]), vsubq_u32(t92[1], t93[1])];
        x24 = t94;
        let t95 = b2_r0r1l_f(l, lid0, arg0, arg1, x24);
        x32 = t95;
        let t96 = [vdupq_n_u64(1u64); 4];
        let t97: [uint64x2_t; 4] = [vandq_u64(x32[0], t96[0]), vandq_u64(x32[1], t96[1]), vandq_u64(x32[2], t96[2]), vandq_u64(x32[3], t96[3])];
        let t98 = [vdupq_n_u64(0u64); 4];
        let t99: [uint64x2_t; 4] = [vceqq_u64(t97[0], t98[0]), vceqq_u64(t97[1], t98[1]), vceqq_u64(t97[2], t98[2]), vceqq_u64(t97[3], t98[3])];
        let t100: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t99[0]), vmovn_u64(t99[1])), vcombine_u32(vmovn_u64(t99[2]), vmovn_u64(t99[3]))];
        if any(&t100) {
            let t101: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x24[0])), vmovl_u32(vget_high_u32(x24[0])), vmovl_u32(vget_low_u32(x24[1])), vmovl_u32(vget_high_u32(x24[1]))];
            res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t100[0])))), t101[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t100[0])))), t101[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t100[1])))), t101[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t100[1])))), t101[3], res[3])];
            live = [vbicq_u32(live[0], t100[0]), vbicq_u32(live[1], t100[1])];
        }
        if any(&live) {
            let t102: [uint64x2_t; 4] = [vshlq_u64(x32[0], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[1], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[2], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[3], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64)))];
            let t103: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t102[0]), vmovn_u64(t102[1])), vcombine_u32(vmovn_u64(t102[2]), vmovn_u64(t102[3]))];
            x40 = [vbslq_u32(live[0], t103[0], x40[0]), vbslq_u32(live[1], t103[1], x40[1])];
            let t104: [uint32x4_t; 2] = [vsubq_u32(x40[0], t93[0]), vsubq_u32(x40[1], t93[1])];
            let t105: [uint32x4_t; 2] = [vcgeq_u32(t104[0], t92[0]), vcgeq_u32(t104[1], t92[1])];
            let t106: [uint32x4_t; 2] = [vandq_u32(live[0], t105[0]), vandq_u32(live[1], t105[1])];
            if any(&t106) {
                let t107 = [vdupq_n_u64(4294967295u64); 4];
                res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t106[0])))), t107[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t106[0])))), t107[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t106[1])))), t107[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t106[1])))), t107[3], res[3])];
                live = [vbicq_u32(live[0], t106[0]), vbicq_u32(live[1], t106[1])];
            }
            if any(&live) {
                let t108: [uint32x4_t; 2] = [vsubq_u32(t92[0], x40[0]), vsubq_u32(t92[1], x40[1])];
                x48 = [vbslq_u32(live[0], t108[0], x48[0]), vbslq_u32(live[1], t108[1], x48[1])];
                let t109 = [vdupq_n_u64(2u64); 4];
                let t110: [uint64x2_t; 4] = [vandq_u64(x32[0], t109[0]), vandq_u64(x32[1], t109[1]), vandq_u64(x32[2], t109[2]), vandq_u64(x32[3], t109[3])];
                let t111: [uint64x2_t; 4] = [vceqq_u64(t110[0], t98[0]), vceqq_u64(t110[1], t98[1]), vceqq_u64(t110[2], t98[2]), vceqq_u64(t110[3], t98[3])];
                let t112: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t111[0]), vmovn_u64(t111[1])), vcombine_u32(vmovn_u64(t111[2]), vmovn_u64(t111[3]))];
                let t113: [uint32x4_t; 2] = [vmvnq_u32(t112[0]), vmvnq_u32(t112[1])];
                let t114: [uint32x4_t; 2] = [vandq_u32(live[0], t113[0]), vandq_u32(live[1], t113[1])];
                if any(&t114) {
                    let t115: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                    res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t114[0])))), t115[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t114[0])))), t115[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t114[1])))), t115[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t114[1])))), t115[3], res[3])];
                    live = [vbicq_u32(live[0], t114[0]), vbicq_u32(live[1], t114[1])];
                }
                if any(&live) {
                    let t116 = [vdupq_n_u32(0u32); 2];
                    let t117: [uint32x4_t; 2] = [vceqq_u32(x48[0], t116[0]), vceqq_u32(x48[1], t116[1])];
                    let t118: [uint32x4_t; 2] = [vandq_u32(live[0], t117[0]), vandq_u32(live[1], t117[1])];
                    if any(&t118) {
                        let t119 = [vdupq_n_u64(4294967295u64); 4];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t118[0])))), t119[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t118[0])))), t119[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t118[1])))), t119[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t118[1])))), t119[3], res[3])];
                        live = [vbicq_u32(live[0], t118[0]), vbicq_u32(live[1], t118[1])];
                    }
                    if any(&live) {
                        let t120 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
                        let t121: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                        let t122: [uint64x2_t; 4] = [vorrq_u64(t120[0], t121[0]), vorrq_u64(t120[1], t121[1]), vorrq_u64(t120[2], t121[2]), vorrq_u64(t120[3], t121[3])];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t122[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t122[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t122[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t122[3], res[3])];
                        live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
                    }
                }
            }
        }
    } else if false {
        let t123 = [vdupq_n_u64(4294967295u64); 4];
        res = t123;
        return res;
    } else {
        // Lanes part: the chunk's lanes on both sides of the bound.
        let t124: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t125 = [vdupq_n_u32(0u32); 2];
        let t126: [uint32x4_t; 2] = [vceqq_u32(t124[0], t125[0]), vceqq_u32(t124[1], t125[1])];
        let t127: [uint32x4_t; 2] = [vandq_u32(live[0], t126[0]), vandq_u32(live[1], t126[1])];
        if any(&t127) {
            let t128 = [vdupq_n_u64(4294967295u64); 4];
            res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t127[0])))), t128[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t127[0])))), t128[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t127[1])))), t128[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t127[1])))), t128[3], res[3])];
            live = [vbicq_u32(live[0], t127[0]), vbicq_u32(live[1], t127[1])];
        }
        if any(&live) {
            let t129 = [vdupq_n_u32(1u32); 2];
            let t130: [uint32x4_t; 2] = [vsubq_u32(t124[0], t129[0]), vsubq_u32(t124[1], t129[1])];
            x24 = [vbslq_u32(live[0], t130[0], x24[0]), vbslq_u32(live[1], t130[1], x24[1])];
            let t131 = b2_r0r1l_p(l, lid0, live, arg0, arg1, x24);
            x32 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t131[0], x32[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t131[1], x32[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t131[2], x32[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t131[3], x32[3])];
            let t132 = [vdupq_n_u64(1u64); 4];
            let t133: [uint64x2_t; 4] = [vandq_u64(x32[0], t132[0]), vandq_u64(x32[1], t132[1]), vandq_u64(x32[2], t132[2]), vandq_u64(x32[3], t132[3])];
            let t134 = [vdupq_n_u64(0u64); 4];
            let t135: [uint64x2_t; 4] = [vceqq_u64(t133[0], t134[0]), vceqq_u64(t133[1], t134[1]), vceqq_u64(t133[2], t134[2]), vceqq_u64(t133[3], t134[3])];
            let t136: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t135[0]), vmovn_u64(t135[1])), vcombine_u32(vmovn_u64(t135[2]), vmovn_u64(t135[3]))];
            let t137: [uint32x4_t; 2] = [vandq_u32(live[0], t136[0]), vandq_u32(live[1], t136[1])];
            if any(&t137) {
                let t138: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x24[0])), vmovl_u32(vget_high_u32(x24[0])), vmovl_u32(vget_low_u32(x24[1])), vmovl_u32(vget_high_u32(x24[1]))];
                res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t137[0])))), t138[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t137[0])))), t138[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t137[1])))), t138[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t137[1])))), t138[3], res[3])];
                live = [vbicq_u32(live[0], t137[0]), vbicq_u32(live[1], t137[1])];
            }
            if any(&live) {
                let t139: [uint64x2_t; 4] = [vshlq_u64(x32[0], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[1], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[2], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[3], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64)))];
                let t140: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t139[0]), vmovn_u64(t139[1])), vcombine_u32(vmovn_u64(t139[2]), vmovn_u64(t139[3]))];
                x40 = [vbslq_u32(live[0], t140[0], x40[0]), vbslq_u32(live[1], t140[1], x40[1])];
                let t141: [uint32x4_t; 2] = [vsubq_u32(x40[0], t129[0]), vsubq_u32(x40[1], t129[1])];
                let t142: [uint32x4_t; 2] = [vcgeq_u32(t141[0], t124[0]), vcgeq_u32(t141[1], t124[1])];
                let t143: [uint32x4_t; 2] = [vandq_u32(live[0], t142[0]), vandq_u32(live[1], t142[1])];
                if any(&t143) {
                    let t144 = [vdupq_n_u64(4294967295u64); 4];
                    res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t143[0])))), t144[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t143[0])))), t144[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t143[1])))), t144[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t143[1])))), t144[3], res[3])];
                    live = [vbicq_u32(live[0], t143[0]), vbicq_u32(live[1], t143[1])];
                }
                if any(&live) {
                    let t145: [uint32x4_t; 2] = [vsubq_u32(t124[0], x40[0]), vsubq_u32(t124[1], x40[1])];
                    x48 = [vbslq_u32(live[0], t145[0], x48[0]), vbslq_u32(live[1], t145[1], x48[1])];
                    let t146 = [vdupq_n_u64(2u64); 4];
                    let t147: [uint64x2_t; 4] = [vandq_u64(x32[0], t146[0]), vandq_u64(x32[1], t146[1]), vandq_u64(x32[2], t146[2]), vandq_u64(x32[3], t146[3])];
                    let t148: [uint64x2_t; 4] = [vceqq_u64(t147[0], t134[0]), vceqq_u64(t147[1], t134[1]), vceqq_u64(t147[2], t134[2]), vceqq_u64(t147[3], t134[3])];
                    let t149: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t148[0]), vmovn_u64(t148[1])), vcombine_u32(vmovn_u64(t148[2]), vmovn_u64(t148[3]))];
                    let t150: [uint32x4_t; 2] = [vmvnq_u32(t149[0]), vmvnq_u32(t149[1])];
                    let t151: [uint32x4_t; 2] = [vandq_u32(live[0], t150[0]), vandq_u32(live[1], t150[1])];
                    if any(&t151) {
                        let t152: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t151[0])))), t152[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t151[0])))), t152[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t151[1])))), t152[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t151[1])))), t152[3], res[3])];
                        live = [vbicq_u32(live[0], t151[0]), vbicq_u32(live[1], t151[1])];
                    }
                    if any(&live) {
                        let t153: [uint32x4_t; 2] = [vceqq_u32(x48[0], t125[0]), vceqq_u32(x48[1], t125[1])];
                        let t154: [uint32x4_t; 2] = [vandq_u32(live[0], t153[0]), vandq_u32(live[1], t153[1])];
                        if any(&t154) {
                            let t155 = [vdupq_n_u64(4294967295u64); 4];
                            res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t154[0])))), t155[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t154[0])))), t155[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t154[1])))), t155[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t154[1])))), t155[3], res[3])];
                            live = [vbicq_u32(live[0], t154[0]), vbicq_u32(live[1], t154[1])];
                        }
                        if any(&live) {
                            let t156 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
                            let t157: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                            let t158: [uint64x2_t; 4] = [vorrq_u64(t156[0], t157[0]), vorrq_u64(t156[1], t157[1]), vorrq_u64(t156[2], t157[2]), vorrq_u64(t156[3], t157[3])];
                            res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t158[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t158[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t158[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t158[3], res[3])];
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
    let t159 = b3_r0r1sl_p(l, lid0, live, arg0, arg1, 0u32, x16);
    res = t159;
    return res;
}

#[inline(always)]
unsafe fn b2_r0r1l_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t160 = b3_r0r1sl_f(l, lid0, arg0, arg1, 0u32, x16);
    res = t160;
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
    let t161 = b4_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    x32 = t161;
    let t162: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t163 = b5_r0r1sl_p(l, lid0, live, arg0, arg1, (1u32).wrapping_add(x16), t162);
    x40 = t163;
    let t164 = b6_s_p(l, lid0, live, x32);
    x48 = t164;
    let t165 = [vdupq_n_u32((1024u32).wrapping_sub(1u32)); 2];
    let t166: [uint32x4_t; 2] = [vandq_u32(x24[0], t165[0]), vandq_u32(x24[1], t165[1])];
    let t167 = [vdupq_n_u32(x32); 2];
    let t168: [uint32x4_t; 2] = [vmulq_u32(t166[0], t167[0]), vmulq_u32(t166[1], t167[1])];
    let t169 = b7_r0r1lls_p(l, lid0, live, arg0, arg1, x40, t168, 8u32);
    let t170 = [vdupq_n_u64(x48); 4];
    let t171: [uint64x2_t; 4] = [vandq_u64(t169[0], t170[0]), vandq_u64(t169[1], t170[1]), vandq_u64(t169[2], t170[2]), vandq_u64(t169[3], t170[3])];
    x56 = t171;
    let t172 = [vdupq_n_u64(x48); 4];
    let t173: [uint64x2_t; 4] = [vceqq_u64(x56[0], t172[0]), vceqq_u64(x56[1], t172[1]), vceqq_u64(x56[2], t172[2]), vceqq_u64(x56[3], t172[3])];
    let t174: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t173[0]), vmovn_u64(t173[1])), vcombine_u32(vmovn_u64(t173[2]), vmovn_u64(t173[3]))];
    let mut t175: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t176: [uint32x4_t; 2] = [vandq_u32(live[0], t174[0]), vandq_u32(live[1], t174[1])];
    let t177: [uint32x4_t; 2] = [vbicq_u32(live[0], t174[0]), vbicq_u32(live[1], t174[1])];
    if any(&t176) {
        let t178 = b8_r0r1sl_p(l, lid0, t176, arg0, arg1, (x16).wrapping_add(1u32), x24);
        t175 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t176[0])))), t178[0], t175[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t176[0])))), t178[1], t175[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t176[1])))), t178[2], t175[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t176[1])))), t178[3], t175[3])];
    }
    {
        t175 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t177[0])))), x56[0], t175[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t177[0])))), x56[1], t175[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t177[1])))), x56[2], t175[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t177[1])))), x56[3], t175[3])];
    }
    res = t175;
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
    let t179 = b4_r0r1s_f(l, lid0, arg0, arg1, x16);
    x32 = t179;
    let t180: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t181 = b5_r0r1sl_f(l, lid0, arg0, arg1, (1u32).wrapping_add(x16), t180);
    x40 = t181;
    let t182 = b6_s_f(l, lid0, x32);
    x48 = t182;
    let t183 = [vdupq_n_u32((1024u32).wrapping_sub(1u32)); 2];
    let t184: [uint32x4_t; 2] = [vandq_u32(x24[0], t183[0]), vandq_u32(x24[1], t183[1])];
    let t185 = [vdupq_n_u32(x32); 2];
    let t186: [uint32x4_t; 2] = [vmulq_u32(t184[0], t185[0]), vmulq_u32(t184[1], t185[1])];
    let t187 = b7_r0r1lls_f(l, lid0, arg0, arg1, x40, t186, 8u32);
    let t188 = [vdupq_n_u64(x48); 4];
    let t189: [uint64x2_t; 4] = [vandq_u64(t187[0], t188[0]), vandq_u64(t187[1], t188[1]), vandq_u64(t187[2], t188[2]), vandq_u64(t187[3], t188[3])];
    x56 = t189;
    let t190 = [vdupq_n_u64(x48); 4];
    let t191: [uint64x2_t; 4] = [vceqq_u64(x56[0], t190[0]), vceqq_u64(x56[1], t190[1]), vceqq_u64(x56[2], t190[2]), vceqq_u64(x56[3], t190[3])];
    let t192: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t191[0]), vmovn_u64(t191[1])), vcombine_u32(vmovn_u64(t191[2]), vmovn_u64(t191[3]))];
    let mut t193: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t194: [uint32x4_t; 2] = [vmvnq_u32(t192[0]), vmvnq_u32(t192[1])];
    if any(&t192) {
        let t195 = b8_r0r1sl_p(l, lid0, t192, arg0, arg1, (x16).wrapping_add(1u32), x24);
        t193 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t192[0])))), t195[0], t193[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t192[0])))), t195[1], t193[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t192[1])))), t195[2], t193[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t192[1])))), t195[3], t193[3])];
    }
    {
        t193 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t194[0])))), x56[0], t193[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t194[0])))), x56[1], t193[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t194[1])))), x56[2], t193[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t194[1])))), x56[3], t193[3])];
    }
    res = t193;
    return res;
}

#[inline(always)]
unsafe fn b4_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t196 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    res = (((t196).wrapping_shr(((4u32).wrapping_mul(x16)) as u32)) & (15u32));
    return res;
}

#[inline(always)]
unsafe fn b4_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t197 = b9_r0r1s_f(l, lid0, arg0, arg1, 16u32);
    res = (((t197).wrapping_shr(((4u32).wrapping_mul(x16)) as u32)) & (15u32));
    return res;
}

#[inline(always)]
unsafe fn b5_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t198 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 40u32);
    let t199 = [vdupq_n_u32(t198); 2];
    let t200: [uint32x4_t; 2] = [vmulq_u32(x24[0], t199[0]), vmulq_u32(x24[1], t199[1])];
    let t201 = b10_r0r1s_p(l, lid0, live, arg0, arg1, 0u32);
    let t202 = [vdupq_n_u32(t201); 2];
    let t203: [uint32x4_t; 2] = [vaddq_u32(t200[0], t202[0]), vaddq_u32(t200[1], t202[1])];
    x32 = t203;
    if ((x16) == (0u32)) {
        res = x32;
        return res;
    }
    let t204 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    let t205: [uint32x4_t; 2] = [vshlq_u32(x24[0], vdupq_n_s32(((10u32) & 31) as i32)), vshlq_u32(x24[1], vdupq_n_s32(((10u32) & 31) as i32))];
    let t206 = [vdupq_n_u32(t204); 2];
    let t207: [uint32x4_t; 2] = [vsubq_u32(t206[0], t205[0]), vsubq_u32(t206[1], t205[1])];
    let t208 = b11_ls_p(l, lid0, live, t207, 1024u32);
    x40 = t208;
    let t209 = b10_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    let t210 = [vdupq_n_u32(t209); 2];
    let t211: [uint32x4_t; 2] = [vmulq_u32(x40[0], t210[0]), vmulq_u32(x40[1], t210[1])];
    let t212: [uint32x4_t; 2] = [vaddq_u32(x32[0], t211[0]), vaddq_u32(x32[1], t211[1])];
    res = t212;
    return res;
}

#[inline(always)]
unsafe fn b5_r0r1sl_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t213 = b9_r0r1s_f(l, lid0, arg0, arg1, 40u32);
    let t214 = [vdupq_n_u32(t213); 2];
    let t215: [uint32x4_t; 2] = [vmulq_u32(x24[0], t214[0]), vmulq_u32(x24[1], t214[1])];
    let t216 = b10_r0r1s_f(l, lid0, arg0, arg1, 0u32);
    let t217 = [vdupq_n_u32(t216); 2];
    let t218: [uint32x4_t; 2] = [vaddq_u32(t215[0], t217[0]), vaddq_u32(t215[1], t217[1])];
    x32 = t218;
    if ((x16) == (0u32)) {
        res = x32;
        return res;
    }
    let t219 = b9_r0r1s_f(l, lid0, arg0, arg1, 4u32);
    let t220: [uint32x4_t; 2] = [vshlq_u32(x24[0], vdupq_n_s32(((10u32) & 31) as i32)), vshlq_u32(x24[1], vdupq_n_s32(((10u32) & 31) as i32))];
    let t221 = [vdupq_n_u32(t219); 2];
    let t222: [uint32x4_t; 2] = [vsubq_u32(t221[0], t220[0]), vsubq_u32(t221[1], t220[1])];
    let t223 = b11_ls_f(l, lid0, t222, 1024u32);
    x40 = t223;
    let t224 = b10_r0r1s_f(l, lid0, arg0, arg1, x16);
    let t225 = [vdupq_n_u32(t224); 2];
    let t226: [uint32x4_t; 2] = [vmulq_u32(x40[0], t225[0]), vmulq_u32(x40[1], t225[1])];
    let t227: [uint32x4_t; 2] = [vaddq_u32(x32[0], t226[0]), vaddq_u32(x32[1], t226[1])];
    res = t227;
    return res;
}

#[inline(always)]
unsafe fn b6_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t228: u64;
    if ((x0) >= (8u32)) {
        t228 = !(0u64);
    } else {
        t228 = ((1u64).wrapping_shl(((((x0).wrapping_mul(8u32)) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t228;
    return res;
}

#[inline(always)]
unsafe fn b6_s_f(l: &Launch, lid0: u32, x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t229: u64;
    if ((x0) >= (8u32)) {
        t229 = !(0u64);
    } else {
        t229 = ((1u64).wrapping_shl(((((x0).wrapping_mul(8u32)) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t229;
    return res;
}

#[inline(always)]
unsafe fn b6_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t230 = [vdupq_n_u32(8u32); 2];
    let t231: [uint32x4_t; 2] = [vcgeq_u32(x0[0], t230[0]), vcgeq_u32(x0[1], t230[1])];
    let mut t232: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t233: [uint32x4_t; 2] = [vandq_u32(live[0], t231[0]), vandq_u32(live[1], t231[1])];
    let t234: [uint32x4_t; 2] = [vbicq_u32(live[0], t231[0]), vbicq_u32(live[1], t231[1])];
    {
        let t235 = [vdupq_n_u64(!(0u64)); 4];
        t232 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t233[0])))), t235[0], t232[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t233[0])))), t235[1], t232[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t233[1])))), t235[2], t232[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t233[1])))), t235[3], t232[3])];
    }
    {
        let t236: [uint32x4_t; 2] = [vmulq_u32(x0[0], t230[0]), vmulq_u32(x0[1], t230[1])];
        let t237 = [vdupq_n_u64(1u64); 4];
        let t238: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t236[0])), vmovl_u32(vget_high_u32(t236[0])), vmovl_u32(vget_low_u32(t236[1])), vmovl_u32(vget_high_u32(t236[1]))];
        let t239: [uint64x2_t; 4] = [vshlq_u64(t237[0], vreinterpretq_s64_u64(vandq_u64(t238[0], vdupq_n_u64(63)))), vshlq_u64(t237[1], vreinterpretq_s64_u64(vandq_u64(t238[1], vdupq_n_u64(63)))), vshlq_u64(t237[2], vreinterpretq_s64_u64(vandq_u64(t238[2], vdupq_n_u64(63)))), vshlq_u64(t237[3], vreinterpretq_s64_u64(vandq_u64(t238[3], vdupq_n_u64(63))))];
        let t240: [uint64x2_t; 4] = [vsubq_u64(t239[0], t237[0]), vsubq_u64(t239[1], t237[1]), vsubq_u64(t239[2], t237[2]), vsubq_u64(t239[3], t237[3])];
        t232 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t234[0])))), t240[0], t232[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t234[0])))), t240[1], t232[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t234[1])))), t240[2], t232[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t234[1])))), t240[3], t232[3])];
    }
    res = t232;
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2], x24: [uint32x4_t; 2], x32: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t241 = b12_r0lls_p(l, lid0, live, arg0, x16, x24, x32);
    res = t241;
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1lls_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2], x24: [uint32x4_t; 2], x32: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t242 = b12_r0lls_f(l, lid0, arg0, x16, x24, x32);
    res = t242;
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
    let t243 = b9_r0r1s_p(l, lid0, live, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x16)));
    x32 = t243;
    let t244 = b9_r0r1s_p(l, lid0, live, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x16)));
    x40 = t244;
    let t245: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    x48 = t245;
    let t246 = [vdupq_n_u32(4u32); 2];
    let t247: [uint32x4_t; 2] = [vmulq_u32(x48[0], t246[0]), vmulq_u32(x48[1], t246[1])];
    let t248 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x40, t247, 4u32);
    let t249: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t248[0]), vmovn_u64(t248[1])), vcombine_u32(vmovn_u64(t248[2]), vmovn_u64(t248[3]))];
    x56 = t249;
    let t250: [uint32x4_t; 2] = [vmulq_u32(x48[0], t246[0]), vmulq_u32(x48[1], t246[1])];
    let t251: [uint32x4_t; 2] = [vaddq_u32(t250[0], t246[0]), vaddq_u32(t250[1], t246[1])];
    let t252 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x40, t251, 4u32);
    let t253: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t252[0]), vmovn_u64(t252[1])), vcombine_u32(vmovn_u64(t252[2]), vmovn_u64(t252[3]))];
    x64 = t253;
    let t254 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    x72 = ((t254).wrapping_add((256u32).wrapping_sub(1u32))).wrapping_shr((8u32) as u32);
    let t255: [uint32x4_t; 2] = [vmulq_u32(x48[0], t246[0]), vmulq_u32(x48[1], t246[1])];
    let t256 = [vdupq_n_u32(x72); 2];
    let t257: [uint32x4_t; 2] = [vsubq_u32(t256[0], t255[0]), vsubq_u32(t256[1], t255[1])];
    let t258 = b14_sl_p(l, lid0, live, 4u32, t257);
    x80 = t258;
    let t259: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
    let t260 = [vdupq_n_u32((4u32).wrapping_sub(1u32)); 2];
    let t261: [uint32x4_t; 2] = [vandq_u32(t259[0], t260[0]), vandq_u32(t259[1], t260[1])];
    x88 = t261;
    let t262 = [vdupq_n_u32(0u32); 2];
    let t263: [uint32x4_t; 2] = [vceqq_u32(x88[0], t262[0]), vceqq_u32(x88[1], t262[1])];
    let mut t264: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t265: [uint32x4_t; 2] = [vandq_u32(live[0], t263[0]), vandq_u32(live[1], t263[1])];
    let t266: [uint32x4_t; 2] = [vbicq_u32(live[0], t263[0]), vbicq_u32(live[1], t263[1])];
    {
        t264 = [vbslq_u32(t265[0], t262[0], t264[0]), vbslq_u32(t265[1], t262[1], t264[1])];
    }
    if any(&t266) {
        let t267 = [vdupq_n_u32(1u32); 2];
        let t268: [uint32x4_t; 2] = [vsubq_u32(x88[0], t267[0]), vsubq_u32(x88[1], t267[1])];
        let t269 = [vdupq_n_u32(2u32); 2];
        let t270: [uint32x4_t; 2] = [vmulq_u32(t269[0], t268[0]), vmulq_u32(t269[1], t268[1])];
        let t271: [uint32x4_t; 2] = [vaddq_u32(x56[0], t270[0]), vaddq_u32(x56[1], t270[1])];
        let t272 = b15_ll_p(l, lid0, t266, t271, x64);
        let t273 = b13_r0r1sls_p(l, lid0, t266, arg0, arg1, x32, t272, 2u32);
        let t274: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t273[0]), vmovn_u64(t273[1])), vcombine_u32(vmovn_u64(t273[2]), vmovn_u64(t273[3]))];
        t264 = [vbslq_u32(t266[0], t274[0], t264[0]), vbslq_u32(t266[1], t274[1], t264[1])];
    }
    x96 = t264;
    let t275 = [vdupq_n_u32(2u32); 2];
    let t276: [uint32x4_t; 2] = [vmulq_u32(t275[0], x88[0]), vmulq_u32(t275[1], x88[1])];
    let t277: [uint32x4_t; 2] = [vaddq_u32(x56[0], t276[0]), vaddq_u32(x56[1], t276[1])];
    let t278 = b15_ll_p(l, lid0, live, t277, x64);
    let t279 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t278, 2u32);
    let t280: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t279[0]), vmovn_u64(t279[1])), vcombine_u32(vmovn_u64(t279[2]), vmovn_u64(t279[3]))];
    x104 = t280;
    let t281: [uint32x4_t; 2] = [vmulq_u32(t275[0], x80[0]), vmulq_u32(t275[1], x80[1])];
    let t282: [uint32x4_t; 2] = [vaddq_u32(x56[0], t281[0]), vaddq_u32(x56[1], t281[1])];
    x112 = t282;
    let t283 = [vdupq_n_u32((256u32).wrapping_sub(1u32)); 2];
    let t284: [uint32x4_t; 2] = [vandq_u32(x24[0], t283[0]), vandq_u32(x24[1], t283[1])];
    x120 = t284;
    x128 = 9259542123273814144u64;
    let t285: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x120[0])), vmovl_u32(vget_high_u32(x120[0])), vmovl_u32(vget_low_u32(x120[1])), vmovl_u32(vget_high_u32(x120[1]))];
    let t286 = [vdupq_n_u64(72340172838076673u64); 4];
    let mut t287: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    t287[0] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t285[0]) as u64).wrapping_mul(vgetq_lane_u64::<0>(t286[0]) as u64) as u64, t287[0]);
    t287[0] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t285[0]) as u64).wrapping_mul(vgetq_lane_u64::<1>(t286[0]) as u64) as u64, t287[0]);
    t287[1] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t285[1]) as u64).wrapping_mul(vgetq_lane_u64::<0>(t286[1]) as u64) as u64, t287[1]);
    t287[1] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t285[1]) as u64).wrapping_mul(vgetq_lane_u64::<1>(t286[1]) as u64) as u64, t287[1]);
    t287[2] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t285[2]) as u64).wrapping_mul(vgetq_lane_u64::<0>(t286[2]) as u64) as u64, t287[2]);
    t287[2] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t285[2]) as u64).wrapping_mul(vgetq_lane_u64::<1>(t286[2]) as u64) as u64, t287[2]);
    t287[3] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t285[3]) as u64).wrapping_mul(vgetq_lane_u64::<0>(t286[3]) as u64) as u64, t287[3]);
    t287[3] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t285[3]) as u64).wrapping_mul(vgetq_lane_u64::<1>(t286[3]) as u64) as u64, t287[3]);
    x136 = t287;
    x144 = x96;
    x152 = x96;
    let mut t288 = live;
    loop {
        // Lanes part: each its own rounds.
        let t290: [uint32x4_t; 2] = [vcltq_u32(x152[0], x104[0]), vcltq_u32(x152[1], x104[1])];
        t288 = [vandq_u32(t288[0], t290[0]), vandq_u32(t288[1], t290[1])];
        if !(any(&t288)) { break; }
        let t292: [uint32x4_t; 2] = [vaddq_u32(x112[0], x152[0]), vaddq_u32(x112[1], x152[1])];
        let t293 = b15_ll_p(l, lid0, t288, t292, x64);
        let t294 = b13_r0r1sls_p(l, lid0, t288, arg0, arg1, x32, t293, 8u32);
        x160 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t288[0])))), t294[0], x160[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t288[0])))), t294[1], x160[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t288[1])))), t294[2], x160[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t288[1])))), t294[3], x160[3])];
        let t295 = [vdupq_n_u64(x128); 4];
        let t296: [uint64x2_t; 4] = [vorrq_u64(x160[0], t295[0]), vorrq_u64(x160[1], t295[1]), vorrq_u64(x160[2], t295[2]), vorrq_u64(x160[3], t295[3])];
        let t297 = [vdupq_n_u64(!(x128)); 4];
        let t298: [uint64x2_t; 4] = [vandq_u64(x136[0], t297[0]), vandq_u64(x136[1], t297[1]), vandq_u64(x136[2], t297[2]), vandq_u64(x136[3], t297[3])];
        let t299: [uint64x2_t; 4] = [vsubq_u64(t296[0], t298[0]), vsubq_u64(t296[1], t298[1]), vsubq_u64(t296[2], t298[2]), vsubq_u64(t296[3], t298[3])];
        x168 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t288[0])))), t299[0], x168[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t288[0])))), t299[1], x168[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t288[1])))), t299[2], x168[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t288[1])))), t299[3], x168[3])];
        let t300: [uint64x2_t; 4] = [vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x160[0]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x160[1]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x160[2]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x160[3])))];
        let t301: [uint64x2_t; 4] = [vandq_u64(t300[0], x136[0]), vandq_u64(t300[1], x136[1]), vandq_u64(t300[2], x136[2]), vandq_u64(t300[3], x136[3])];
        let t302: [uint64x2_t; 4] = [veorq_u64(x160[0], x136[0]), veorq_u64(x160[1], x136[1]), veorq_u64(x160[2], x136[2]), veorq_u64(x160[3], x136[3])];
        let t303: [uint64x2_t; 4] = [vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(t302[0]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(t302[1]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(t302[2]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(t302[3])))];
        let t304: [uint64x2_t; 4] = [vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x168[0]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x168[1]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x168[2]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x168[3])))];
        let t305: [uint64x2_t; 4] = [vandq_u64(t303[0], t304[0]), vandq_u64(t303[1], t304[1]), vandq_u64(t303[2], t304[2]), vandq_u64(t303[3], t304[3])];
        let t306: [uint64x2_t; 4] = [vorrq_u64(t301[0], t305[0]), vorrq_u64(t301[1], t305[1]), vorrq_u64(t301[2], t305[2]), vorrq_u64(t301[3], t305[3])];
        let t307 = [vdupq_n_u64(x128); 4];
        let t308: [uint64x2_t; 4] = [vandq_u64(t307[0], t306[0]), vandq_u64(t307[1], t306[1]), vandq_u64(t307[2], t306[2]), vandq_u64(t307[3], t306[3])];
        x176 = t308;
        let t309: [uint32x4_t; 2] = [vsubq_u32(x104[0], x152[0]), vsubq_u32(x104[1], x152[1])];
        x184 = t309;
        let t310 = [vdupq_n_u32(8u32); 2];
        let t311: [uint32x4_t; 2] = [vcgeq_u32(x184[0], t310[0]), vcgeq_u32(x184[1], t310[1])];
        let mut t312: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
        let t313: [uint32x4_t; 2] = [vandq_u32(t288[0], t311[0]), vandq_u32(t288[1], t311[1])];
        let t314: [uint32x4_t; 2] = [vbicq_u32(t288[0], t311[0]), vbicq_u32(t288[1], t311[1])];
        {
            t312 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t313[0])))), x176[0], t312[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t313[0])))), x176[1], t312[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t313[1])))), x176[2], t312[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t313[1])))), x176[3], t312[3])];
        }
        {
            let t315: [uint32x4_t; 2] = [vmulq_u32(t310[0], x184[0]), vmulq_u32(t310[1], x184[1])];
            let t316 = [vdupq_n_u64(1u64); 4];
            let t317: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t315[0])), vmovl_u32(vget_high_u32(t315[0])), vmovl_u32(vget_low_u32(t315[1])), vmovl_u32(vget_high_u32(t315[1]))];
            let t318: [uint64x2_t; 4] = [vshlq_u64(t316[0], vreinterpretq_s64_u64(vandq_u64(t317[0], vdupq_n_u64(63)))), vshlq_u64(t316[1], vreinterpretq_s64_u64(vandq_u64(t317[1], vdupq_n_u64(63)))), vshlq_u64(t316[2], vreinterpretq_s64_u64(vandq_u64(t317[2], vdupq_n_u64(63)))), vshlq_u64(t316[3], vreinterpretq_s64_u64(vandq_u64(t317[3], vdupq_n_u64(63))))];
            let t319: [uint64x2_t; 4] = [vsubq_u64(t318[0], t316[0]), vsubq_u64(t318[1], t316[1]), vsubq_u64(t318[2], t316[2]), vsubq_u64(t318[3], t316[3])];
            let t320: [uint64x2_t; 4] = [vandq_u64(x176[0], t319[0]), vandq_u64(x176[1], t319[1]), vandq_u64(x176[2], t319[2]), vandq_u64(x176[3], t319[3])];
            t312 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t314[0])))), t320[0], t312[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t314[0])))), t320[1], t312[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t314[1])))), t320[2], t312[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t314[1])))), t320[3], t312[3])];
        }
        let t321 = b16_l_p(l, lid0, t288, t312);
        let t322: [uint32x4_t; 2] = [vaddq_u32(x144[0], t321[0]), vaddq_u32(x144[1], t321[1])];
        x144 = [vbslq_u32(t288[0], t322[0], x144[0]), vbslq_u32(t288[1], t322[1], x144[1])];
        let t323: [uint32x4_t; 2] = [vandq_u32(t288[0], live[0]), vandq_u32(t288[1], live[1])];
        x152 = [vaddq_u32(x152[0], vandq_u32(t323[0], vdupq_n_u32(8))), vaddq_u32(x152[1], vandq_u32(t323[1], vdupq_n_u32(8)))];
    }
    x96 = x144;
    let t324 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    let t325 = [vdupq_n_u32((((t324).wrapping_shr((((((4u32).wrapping_mul(x16)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32))); 2];
    x152 = t325;
    let t326 = [vdupq_n_u32(1u32); 2];
    let t327: [uint32x4_t; 2] = [vsubq_u32(x80[0], t326[0]), vsubq_u32(x80[1], t326[1])];
    let t328: [uint32x4_t; 2] = [vmulq_u32(t275[0], t327[0]), vmulq_u32(t275[1], t327[1])];
    let t329: [uint32x4_t; 2] = [vaddq_u32(x56[0], t328[0]), vaddq_u32(x56[1], t328[1])];
    let t330 = b15_ll_p(l, lid0, live, t329, x64);
    let t331 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t330, 2u32);
    let t332: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t331[0]), vmovn_u64(t331[1])), vcombine_u32(vmovn_u64(t331[2]), vmovn_u64(t331[3]))];
    let t333: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t332[0])), vmovl_u32(vget_high_u32(t332[0])), vmovl_u32(vget_low_u32(t332[1])), vmovl_u32(vget_high_u32(t332[1]))];
    x160 = t333;
    let t334: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x160[0]), vmovn_u64(x160[1])), vcombine_u32(vmovn_u64(x160[2]), vmovn_u64(x160[3]))];
    let t335: [uint32x4_t; 2] = [vaddq_u32(x112[0], t334[0]), vaddq_u32(x112[1], t334[1])];
    let t336: [uint32x4_t; 2] = [vmulq_u32(x96[0], x152[0]), vmulq_u32(x96[1], x152[1])];
    let t337: [uint32x4_t; 2] = [vaddq_u32(t335[0], t336[0]), vaddq_u32(t335[1], t336[1])];
    let t338 = b15_ll_p(l, lid0, live, t337, x64);
    let t339: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t338[0])), vmovl_u32(vget_high_u32(t338[0])), vmovl_u32(vget_low_u32(t338[1])), vmovl_u32(vget_high_u32(t338[1]))];
    x168 = t339;
    let t340: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x168[0]), vmovn_u64(x168[1])), vcombine_u32(vmovn_u64(x168[2]), vmovn_u64(x168[3]))];
    let t341 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t340, 8u32);
    let t342 = b6_l_p(l, lid0, live, x152);
    let t343: [uint64x2_t; 4] = [vandq_u64(t341[0], t342[0]), vandq_u64(t341[1], t342[1]), vandq_u64(t341[2], t342[2]), vandq_u64(t341[3], t342[3])];
    res = t343;
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
    let t344 = b9_r0r1s_p(l, lid0, live, arg0, arg1, (20u32).wrapping_add((4u32).wrapping_mul(x16)));
    res = t344;
    return res;
}

#[inline(always)]
unsafe fn b10_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t345 = b9_r0r1s_f(l, lid0, arg0, arg1, (20u32).wrapping_add((4u32).wrapping_mul(x16)));
    res = t345;
    return res;
}

#[inline(always)]
unsafe fn b11_ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x8: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t346 = [vdupq_n_u32(x8); 2];
    let t347: [uint32x4_t; 2] = [vcltq_u32(x0[0], t346[0]), vcltq_u32(x0[1], t346[1])];
    let mut t348: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t349: [uint32x4_t; 2] = [vandq_u32(live[0], t347[0]), vandq_u32(live[1], t347[1])];
    let t350: [uint32x4_t; 2] = [vbicq_u32(live[0], t347[0]), vbicq_u32(live[1], t347[1])];
    {
        t348 = [vbslq_u32(t349[0], x0[0], t348[0]), vbslq_u32(t349[1], x0[1], t348[1])];
    }
    {
        let t351 = [vdupq_n_u32(x8); 2];
        t348 = [vbslq_u32(t350[0], t351[0], t348[0]), vbslq_u32(t350[1], t351[1], t348[1])];
    }
    res = t348;
    return res;
}

#[inline(always)]
unsafe fn b11_ls_f(l: &Launch, lid0: u32, x0: [uint32x4_t; 2], x8: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t352 = [vdupq_n_u32(x8); 2];
    let t353: [uint32x4_t; 2] = [vcltq_u32(x0[0], t352[0]), vcltq_u32(x0[1], t352[1])];
    let mut t354: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t355: [uint32x4_t; 2] = [vmvnq_u32(t353[0]), vmvnq_u32(t353[1])];
    {
        t354 = [vbslq_u32(t353[0], x0[0], t354[0]), vbslq_u32(t353[1], x0[1], t354[1])];
    }
    {
        let t356 = [vdupq_n_u32(x8); 2];
        t354 = [vbslq_u32(t355[0], t356[0], t354[0]), vbslq_u32(t355[1], t356[1], t354[1])];
    }
    res = t354;
    return res;
}

#[inline(always)]
unsafe fn b12_r0lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: [uint32x4_t; 2], x16: [uint32x4_t; 2], x24: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t357: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x8[0])), vmovl_u32(vget_high_u32(x8[0])), vmovl_u32(vget_low_u32(x8[1])), vmovl_u32(vget_high_u32(x8[1]))];
    let t358: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x16[0])), vmovl_u32(vget_high_u32(x16[0])), vmovl_u32(vget_low_u32(x16[1])), vmovl_u32(vget_high_u32(x16[1]))];
    let t359: [uint64x2_t; 4] = [vaddq_u64(t357[0], t358[0]), vaddq_u64(t357[1], t358[1]), vaddq_u64(t357[2], t358[2]), vaddq_u64(t357[3], t358[3])];
    let t360 = b17_r0ls_p(l, lid0, live, arg0, t359, x24);
    x32 = t360;
    let t361: [uint64x2_t; 4];
    if ((x24) >= (8u32)) {
        t361 = x32;
    } else {
        let t362 = b18_s_p(l, lid0, live, (8u32).wrapping_mul(x24));
        let t363 = [vdupq_n_u64(t362); 4];
        let t364: [uint64x2_t; 4] = [vandq_u64(x32[0], t363[0]), vandq_u64(x32[1], t363[1]), vandq_u64(x32[2], t363[2]), vandq_u64(x32[3], t363[3])];
        t361 = t364;
    }
    res = t361;
    return res;
}

#[inline(always)]
unsafe fn b12_r0lls_f(l: &Launch, lid0: u32, arg0: *const u8, x8: [uint32x4_t; 2], x16: [uint32x4_t; 2], x24: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t365: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x8[0])), vmovl_u32(vget_high_u32(x8[0])), vmovl_u32(vget_low_u32(x8[1])), vmovl_u32(vget_high_u32(x8[1]))];
    let t366: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x16[0])), vmovl_u32(vget_high_u32(x16[0])), vmovl_u32(vget_low_u32(x16[1])), vmovl_u32(vget_high_u32(x16[1]))];
    let t367: [uint64x2_t; 4] = [vaddq_u64(t365[0], t366[0]), vaddq_u64(t365[1], t366[1]), vaddq_u64(t365[2], t366[2]), vaddq_u64(t365[3], t366[3])];
    let t368 = b17_r0ls_f(l, lid0, arg0, t367, x24);
    x32 = t368;
    let t369: [uint64x2_t; 4];
    if ((x24) >= (8u32)) {
        t369 = x32;
    } else {
        let t370 = b18_s_f(l, lid0, (8u32).wrapping_mul(x24));
        let t371 = [vdupq_n_u64(t370); 4];
        let t372: [uint64x2_t; 4] = [vandq_u64(x32[0], t371[0]), vandq_u64(x32[1], t371[1]), vandq_u64(x32[2], t371[2]), vandq_u64(x32[3], t371[3])];
        t369 = t372;
    }
    res = t369;
    return res;
}

#[inline(always)]
unsafe fn b13_r0r1sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2], x32: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t373 = b19_r0sls_p(l, lid0, live, arg0, x16, x24, x32);
    res = t373;
    return res;
}

#[inline(always)]
unsafe fn b14_sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x8: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t374 = [vdupq_n_u32(x0); 2];
    let t375: [uint32x4_t; 2] = [vcltq_u32(t374[0], x8[0]), vcltq_u32(t374[1], x8[1])];
    let mut t376: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t377: [uint32x4_t; 2] = [vandq_u32(live[0], t375[0]), vandq_u32(live[1], t375[1])];
    let t378: [uint32x4_t; 2] = [vbicq_u32(live[0], t375[0]), vbicq_u32(live[1], t375[1])];
    {
        let t379 = [vdupq_n_u32(x0); 2];
        t376 = [vbslq_u32(t377[0], t379[0], t376[0]), vbslq_u32(t377[1], t379[1], t376[1])];
    }
    {
        t376 = [vbslq_u32(t378[0], x8[0], t376[0]), vbslq_u32(t378[1], x8[1], t376[1])];
    }
    res = t376;
    return res;
}

#[inline(always)]
unsafe fn b15_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x8: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t380: [uint32x4_t; 2] = [vcltq_u32(x0[0], x8[0]), vcltq_u32(x0[1], x8[1])];
    let mut t381: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t382: [uint32x4_t; 2] = [vandq_u32(live[0], t380[0]), vandq_u32(live[1], t380[1])];
    let t383: [uint32x4_t; 2] = [vbicq_u32(live[0], t380[0]), vbicq_u32(live[1], t380[1])];
    {
        t381 = [vbslq_u32(t382[0], x0[0], t381[0]), vbslq_u32(t382[1], x0[1], t381[1])];
    }
    {
        t381 = [vbslq_u32(t383[0], x8[0], t381[0]), vbslq_u32(t383[1], x8[1], t381[1])];
    }
    res = t381;
    return res;
}

#[inline(always)]
unsafe fn b16_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint64x2_t; 4]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t384: [uint64x2_t; 4] = [vpaddlq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u64(x0[0]))))), vpaddlq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u64(x0[1]))))), vpaddlq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u64(x0[2]))))), vpaddlq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u64(x0[3])))))];
    let t385: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t384[0]), vmovn_u64(t384[1])), vcombine_u32(vmovn_u64(t384[2]), vmovn_u64(t384[3]))];
    res = t385;
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
    let t386 = [vdupq_n_u64(!(7u64)); 4];
    let t387: [uint64x2_t; 4] = [vandq_u64(x8[0], t386[0]), vandq_u64(x8[1], t386[1]), vandq_u64(x8[2], t386[2]), vandq_u64(x8[3], t386[3])];
    let t388: [uint64x2_t; 4] = [vandq_u64(t387[0], vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0]))))), vandq_u64(t387[1], vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0]))))), vandq_u64(t387[2], vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1]))))), vandq_u64(t387[3], vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))))];
    let t389: [uint64x2_t; 4] = [vaddq_u64(vdupq_n_u64((arg0) as u64), t388[0]), vaddq_u64(vdupq_n_u64((arg0) as u64), t388[1]), vaddq_u64(vdupq_n_u64((arg0) as u64), t388[2]), vaddq_u64(vdupq_n_u64((arg0) as u64), t388[3])];
    x24 = t389;
    let t390 = [vdupq_n_u64(7u64); 4];
    let t391: [uint64x2_t; 4] = [vandq_u64(x8[0], t390[0]), vandq_u64(x8[1], t390[1]), vandq_u64(x8[2], t390[2]), vandq_u64(x8[3], t390[3])];
    let t392: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t391[0]), vmovn_u64(t391[1])), vcombine_u32(vmovn_u64(t391[2]), vmovn_u64(t391[3]))];
    let t393 = [vdupq_n_u32(8u32); 2];
    let t394: [uint32x4_t; 2] = [vmulq_u32(t392[0], t393[0]), vmulq_u32(t392[1], t393[1])];
    x32 = t394;
    let mut t395: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    t395[0] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[0]) as usize as *const u64).read_unaligned(), t395[0]);
    t395[0] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[0]) as usize as *const u64).read_unaligned(), t395[0]);
    t395[1] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[1]) as usize as *const u64).read_unaligned(), t395[1]);
    t395[1] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[1]) as usize as *const u64).read_unaligned(), t395[1]);
    t395[2] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[2]) as usize as *const u64).read_unaligned(), t395[2]);
    t395[2] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[2]) as usize as *const u64).read_unaligned(), t395[2]);
    t395[3] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[3]) as usize as *const u64).read_unaligned(), t395[3]);
    t395[3] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[3]) as usize as *const u64).read_unaligned(), t395[3]);
    let t396: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x32[0])), vmovl_u32(vget_high_u32(x32[0])), vmovl_u32(vget_low_u32(x32[1])), vmovl_u32(vget_high_u32(x32[1]))];
    let t397: [uint64x2_t; 4] = [vshlq_u64(t395[0], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t396[0], vdupq_n_u64(63))))), vshlq_u64(t395[1], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t396[1], vdupq_n_u64(63))))), vshlq_u64(t395[2], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t396[2], vdupq_n_u64(63))))), vshlq_u64(t395[3], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t396[3], vdupq_n_u64(63)))))];
    x40 = t397;
    let t398 = [vdupq_n_u32(0u32); 2];
    let t399: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t398[0])), vmvnq_u32(vceqq_u32(x32[1], t398[1]))];
    let t400 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t401: [uint32x4_t; 2] = [vaddq_u32(x32[0], t400[0]), vaddq_u32(x32[1], t400[1])];
    let t402 = [vdupq_n_u32(64u32); 2];
    let t403: [uint32x4_t; 2] = [vcgtq_u32(t401[0], t402[0]), vcgtq_u32(t401[1], t402[1])];
    let t404: [uint32x4_t; 2] = [vandq_u32(t399[0], t403[0]), vandq_u32(t399[1], t403[1])];
    let mut t405: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t406: [uint32x4_t; 2] = [vandq_u32(live[0], t404[0]), vandq_u32(live[1], t404[1])];
    let t407: [uint32x4_t; 2] = [vbicq_u32(live[0], t404[0]), vbicq_u32(live[1], t404[1])];
    if any(&t406) {
        let t408 = [vdupq_n_u64((1u64).wrapping_mul(8u64)); 4];
        let t409: [uint64x2_t; 4] = [vaddq_u64(x24[0], t408[0]), vaddq_u64(x24[1], t408[1]), vaddq_u64(x24[2], t408[2]), vaddq_u64(x24[3], t408[3])];
        let mut t410: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
        t410[0] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t409[0]) as usize as *const u64).read_unaligned(), t410[0]);
        t410[0] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t409[0]) as usize as *const u64).read_unaligned(), t410[0]);
        t410[1] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t409[1]) as usize as *const u64).read_unaligned(), t410[1]);
        t410[1] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t409[1]) as usize as *const u64).read_unaligned(), t410[1]);
        t410[2] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t409[2]) as usize as *const u64).read_unaligned(), t410[2]);
        t410[2] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t409[2]) as usize as *const u64).read_unaligned(), t410[2]);
        t410[3] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t409[3]) as usize as *const u64).read_unaligned(), t410[3]);
        t410[3] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t409[3]) as usize as *const u64).read_unaligned(), t410[3]);
        let t411: [uint32x4_t; 2] = [vsubq_u32(t402[0], x32[0]), vsubq_u32(t402[1], x32[1])];
        let t412: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t411[0])), vmovl_u32(vget_high_u32(t411[0])), vmovl_u32(vget_low_u32(t411[1])), vmovl_u32(vget_high_u32(t411[1]))];
        let t413: [uint64x2_t; 4] = [vshlq_u64(t410[0], vreinterpretq_s64_u64(vandq_u64(t412[0], vdupq_n_u64(63)))), vshlq_u64(t410[1], vreinterpretq_s64_u64(vandq_u64(t412[1], vdupq_n_u64(63)))), vshlq_u64(t410[2], vreinterpretq_s64_u64(vandq_u64(t412[2], vdupq_n_u64(63)))), vshlq_u64(t410[3], vreinterpretq_s64_u64(vandq_u64(t412[3], vdupq_n_u64(63))))];
        let t414: [uint64x2_t; 4] = [vorrq_u64(x40[0], t413[0]), vorrq_u64(x40[1], t413[1]), vorrq_u64(x40[2], t413[2]), vorrq_u64(x40[3], t413[3])];
        t405 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t406[0])))), t414[0], t405[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t406[0])))), t414[1], t405[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t406[1])))), t414[2], t405[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t406[1])))), t414[3], t405[3])];
    }
    {
        t405 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t407[0])))), x40[0], t405[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t407[0])))), x40[1], t405[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t407[1])))), x40[2], t405[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t407[1])))), x40[3], t405[3])];
    }
    res = t405;
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
    let t415 = [vdupq_n_u64(!(7u64)); 4];
    let t416: [uint64x2_t; 4] = [vandq_u64(x8[0], t415[0]), vandq_u64(x8[1], t415[1]), vandq_u64(x8[2], t415[2]), vandq_u64(x8[3], t415[3])];
    let t417: [uint64x2_t; 4] = [vaddq_u64(vdupq_n_u64((arg0) as u64), t416[0]), vaddq_u64(vdupq_n_u64((arg0) as u64), t416[1]), vaddq_u64(vdupq_n_u64((arg0) as u64), t416[2]), vaddq_u64(vdupq_n_u64((arg0) as u64), t416[3])];
    x24 = t417;
    let t418 = [vdupq_n_u64(7u64); 4];
    let t419: [uint64x2_t; 4] = [vandq_u64(x8[0], t418[0]), vandq_u64(x8[1], t418[1]), vandq_u64(x8[2], t418[2]), vandq_u64(x8[3], t418[3])];
    let t420: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t419[0]), vmovn_u64(t419[1])), vcombine_u32(vmovn_u64(t419[2]), vmovn_u64(t419[3]))];
    let t421 = [vdupq_n_u32(8u32); 2];
    let t422: [uint32x4_t; 2] = [vmulq_u32(t420[0], t421[0]), vmulq_u32(t420[1], t421[1])];
    x32 = t422;
    let mut t423: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    t423[0] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[0]) as usize as *const u64).read_unaligned(), t423[0]);
    t423[0] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[0]) as usize as *const u64).read_unaligned(), t423[0]);
    t423[1] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[1]) as usize as *const u64).read_unaligned(), t423[1]);
    t423[1] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[1]) as usize as *const u64).read_unaligned(), t423[1]);
    t423[2] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[2]) as usize as *const u64).read_unaligned(), t423[2]);
    t423[2] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[2]) as usize as *const u64).read_unaligned(), t423[2]);
    t423[3] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[3]) as usize as *const u64).read_unaligned(), t423[3]);
    t423[3] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[3]) as usize as *const u64).read_unaligned(), t423[3]);
    let t424: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x32[0])), vmovl_u32(vget_high_u32(x32[0])), vmovl_u32(vget_low_u32(x32[1])), vmovl_u32(vget_high_u32(x32[1]))];
    let t425: [uint64x2_t; 4] = [vshlq_u64(t423[0], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t424[0], vdupq_n_u64(63))))), vshlq_u64(t423[1], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t424[1], vdupq_n_u64(63))))), vshlq_u64(t423[2], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t424[2], vdupq_n_u64(63))))), vshlq_u64(t423[3], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t424[3], vdupq_n_u64(63)))))];
    x40 = t425;
    let t426 = [vdupq_n_u32(0u32); 2];
    let t427: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t426[0])), vmvnq_u32(vceqq_u32(x32[1], t426[1]))];
    let t428 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t429: [uint32x4_t; 2] = [vaddq_u32(x32[0], t428[0]), vaddq_u32(x32[1], t428[1])];
    let t430 = [vdupq_n_u32(64u32); 2];
    let t431: [uint32x4_t; 2] = [vcgtq_u32(t429[0], t430[0]), vcgtq_u32(t429[1], t430[1])];
    let t432: [uint32x4_t; 2] = [vandq_u32(t427[0], t431[0]), vandq_u32(t427[1], t431[1])];
    let mut t433: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t434: [uint32x4_t; 2] = [vmvnq_u32(t432[0]), vmvnq_u32(t432[1])];
    if any(&t432) {
        let t435 = [vdupq_n_u64((1u64).wrapping_mul(8u64)); 4];
        let t436: [uint64x2_t; 4] = [vaddq_u64(x24[0], t435[0]), vaddq_u64(x24[1], t435[1]), vaddq_u64(x24[2], t435[2]), vaddq_u64(x24[3], t435[3])];
        let mut t437: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
        t437[0] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t436[0]) as usize as *const u64).read_unaligned(), t437[0]);
        t437[0] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t436[0]) as usize as *const u64).read_unaligned(), t437[0]);
        t437[1] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t436[1]) as usize as *const u64).read_unaligned(), t437[1]);
        t437[1] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t436[1]) as usize as *const u64).read_unaligned(), t437[1]);
        t437[2] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t436[2]) as usize as *const u64).read_unaligned(), t437[2]);
        t437[2] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t436[2]) as usize as *const u64).read_unaligned(), t437[2]);
        t437[3] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t436[3]) as usize as *const u64).read_unaligned(), t437[3]);
        t437[3] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t436[3]) as usize as *const u64).read_unaligned(), t437[3]);
        let t438: [uint32x4_t; 2] = [vsubq_u32(t430[0], x32[0]), vsubq_u32(t430[1], x32[1])];
        let t439: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t438[0])), vmovl_u32(vget_high_u32(t438[0])), vmovl_u32(vget_low_u32(t438[1])), vmovl_u32(vget_high_u32(t438[1]))];
        let t440: [uint64x2_t; 4] = [vshlq_u64(t437[0], vreinterpretq_s64_u64(vandq_u64(t439[0], vdupq_n_u64(63)))), vshlq_u64(t437[1], vreinterpretq_s64_u64(vandq_u64(t439[1], vdupq_n_u64(63)))), vshlq_u64(t437[2], vreinterpretq_s64_u64(vandq_u64(t439[2], vdupq_n_u64(63)))), vshlq_u64(t437[3], vreinterpretq_s64_u64(vandq_u64(t439[3], vdupq_n_u64(63))))];
        let t441: [uint64x2_t; 4] = [vorrq_u64(x40[0], t440[0]), vorrq_u64(x40[1], t440[1]), vorrq_u64(x40[2], t440[2]), vorrq_u64(x40[3], t440[3])];
        t433 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t432[0])))), t441[0], t433[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t432[0])))), t441[1], t433[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t432[1])))), t441[2], t433[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t432[1])))), t441[3], t433[3])];
    }
    {
        t433 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t434[0])))), x40[0], t433[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t434[0])))), x40[1], t433[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t434[1])))), x40[2], t433[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t434[1])))), x40[3], t433[3])];
    }
    res = t433;
    return res;
}

#[inline(always)]
unsafe fn b18_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t442: u64;
    if ((x0) >= (64u32)) {
        t442 = !(0u64);
    } else {
        t442 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t442;
    return res;
}

#[inline(always)]
unsafe fn b18_s_f(l: &Launch, lid0: u32, x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t443: u64;
    if ((x0) >= (64u32)) {
        t443 = !(0u64);
    } else {
        t443 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t443;
    return res;
}

#[inline(always)]
unsafe fn b19_r0sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: u32, x16: [uint32x4_t; 2], x24: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t444 = [vdupq_n_u64(((x8) as u64)); 4];
    let t445: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x16[0])), vmovl_u32(vget_high_u32(x16[0])), vmovl_u32(vget_low_u32(x16[1])), vmovl_u32(vget_high_u32(x16[1]))];
    let t446: [uint64x2_t; 4] = [vaddq_u64(t444[0], t445[0]), vaddq_u64(t444[1], t445[1]), vaddq_u64(t444[2], t445[2]), vaddq_u64(t444[3], t445[3])];
    let t447 = b17_r0ls_p(l, lid0, live, arg0, t446, x24);
    x32 = t447;
    let t448: [uint64x2_t; 4];
    if ((x24) >= (8u32)) {
        t448 = x32;
    } else {
        let t449 = b18_s_p(l, lid0, live, (8u32).wrapping_mul(x24));
        let t450 = [vdupq_n_u64(t449); 4];
        let t451: [uint64x2_t; 4] = [vandq_u64(x32[0], t450[0]), vandq_u64(x32[1], t450[1]), vandq_u64(x32[2], t450[2]), vandq_u64(x32[3], t450[3])];
        t448 = t451;
    }
    res = t448;
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
    let arg3: *const u8 = (*argv.add(3) as *const *const u8).read_unaligned();
    let x32: *const u8 = *argv.add(4);
    let arg5: *const u8 = (*argv.add(5) as *const *const u8).read_unaligned();
    let x56: u32 = (*argv.add(6) as *const u32).read_unaligned();
    let mut t452: bool = !(arg5).is_null();
    if t452 {
        let mut t453: bool = ((l.group[0]) >= ((arg5.wrapping_add((((x56) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        if !t453 {
            t453 = ((l.group[1]) >= ((arg5.wrapping_add(((((x56).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        let mut t454: bool = t453;
        if !t454 {
            t454 = ((l.group[2]) >= ((arg5.wrapping_add(((((x56).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        t452 = t454;
    }
    if t452 {
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
        let t455: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
        let mut live = t455;
        let gid0 = l.group[0] * l.block[0] + lid0;
        'chunk: {
            b0_r0r1r2r3sg_p(l, lid0, live, arg0, arg1, arg2, arg3, x32);
        }
    }
}
