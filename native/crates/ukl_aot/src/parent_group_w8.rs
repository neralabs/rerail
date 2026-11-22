//! `parent_group` at 8 lanes (2 NEON registers), written by rerail-ukl-vec's Rust (`rs.rs`) from its vector tape: do
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
unsafe fn b0_r0r1r2sr6gi_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, arg2: *const u8, x24: *const u8, arg6: *const u8, x56: u32, phase: u32, st: *mut u8) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x24 as *const u32).read_unaligned();
    let mut x64: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x72: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x80: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = k;
    if phase > 0 {
        live = (st as *const [uint32x4_t; 2]).read_unaligned();
        x64 = (st.add(64) as *const [uint32x4_t; 2]).read_unaligned();
        x72 = (st.add(128) as *const [uint32x4_t; 2]).read_unaligned();
        x80 = (st.add(192) as *const [uint64x2_t; 4]).read_unaligned();
        if !any(&live) { return; }
    }
    if phase == 0 {
        'phase: {
            let t1: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
            let t2: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x56), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x56), vld1q_u32(LANE[1].as_ptr()))];
            let t3: [uint32x4_t; 2] = [vsubq_u32(t1[0], t2[0]), vsubq_u32(t1[1], t2[1])];
            x64 = t3;
            let t4 = [vdupq_n_u32(4294967295u32); 2];
            x72 = t4;
            let t5 = [vdupq_n_u32(a0); 2];
            let t6: [uint32x4_t; 2] = [vcltq_u32(t1[0], t5[0]), vcltq_u32(t1[1], t5[1])];
            let t7: [uint32x4_t; 2] = [vandq_u32(live[0], t6[0]), vandq_u32(live[1], t6[1])];
            if any(&t7) {
                let t8 = b1_r0r1g_p(l, lid0, t7, arg0, arg1);
                x80 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t7[0])))), t8[0], x80[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t7[0])))), t8[1], x80[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t7[1])))), t8[2], x80[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t7[1])))), t8[3], x80[3])];
                let t9 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
                let t10: [uint64x2_t; 4] = [vandq_u64(x80[0], t9[0]), vandq_u64(x80[1], t9[1]), vandq_u64(x80[2], t9[2]), vandq_u64(x80[3], t9[3])];
                let t11 = [vdupq_n_u64(0u64); 4];
                let t12: [uint64x2_t; 4] = [vceqq_u64(t10[0], t11[0]), vceqq_u64(t10[1], t11[1]), vceqq_u64(t10[2], t11[2]), vceqq_u64(t10[3], t11[3])];
                let t13: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t12[0]), vmovn_u64(t12[1])), vcombine_u32(vmovn_u64(t12[2]), vmovn_u64(t12[3]))];
                let t14: [uint32x4_t; 2] = [vmvnq_u32(t13[0]), vmvnq_u32(t13[1])];
                let mut t15: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                let t16: [uint32x4_t; 2] = [vandq_u32(t7[0], t14[0]), vandq_u32(t7[1], t14[1])];
                let t17: [uint32x4_t; 2] = [vbicq_u32(t7[0], t14[0]), vbicq_u32(t7[1], t14[1])];
                {
                    let t18: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x80[0]), vmovn_u64(x80[1])), vcombine_u32(vmovn_u64(x80[2]), vmovn_u64(x80[3]))];
                    let t19 = [vdupq_n_u32(2147483648u32); 2];
                    let t20: [uint32x4_t; 2] = [vorrq_u32(t19[0], t18[0]), vorrq_u32(t19[1], t18[1])];
                    t15 = [vbslq_u32(t16[0], t20[0], t15[0]), vbslq_u32(t16[1], t20[1], t15[1])];
                }
                {
                    let t21: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x80[0]), vmovn_u64(x80[1])), vcombine_u32(vmovn_u64(x80[2]), vmovn_u64(x80[3]))];
                    t15 = [vbslq_u32(t17[0], t21[0], t15[0]), vbslq_u32(t17[1], t21[1], t15[1])];
                }
                x72 = [vbslq_u32(t7[0], t15[0], x72[0]), vbslq_u32(t7[1], t15[1], x72[1])];
            }
            let t22 = (arg6 as *mut u32).add((x56) as usize);
            if all(&live) {
                vst1q_u32(t22.add(0), x72[0]);
                vst1q_u32(t22.add(4), x72[1]);
            } else {
                let mut t23w = 0u32;
                let t23 = core::hint::black_box(&raw mut t23w);
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t22.add(0), t23).write(vgetq_lane_u32::<0>(x72[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t22.add(1), t23).write(vgetq_lane_u32::<1>(x72[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t22.add(2), t23).write(vgetq_lane_u32::<2>(x72[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t22.add(3), t23).write(vgetq_lane_u32::<3>(x72[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t22.add(4), t23).write(vgetq_lane_u32::<0>(x72[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t22.add(5), t23).write(vgetq_lane_u32::<1>(x72[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t22.add(6), t23).write(vgetq_lane_u32::<2>(x72[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t22.add(7), t23).write(vgetq_lane_u32::<3>(x72[1]));
            }
        }
    }
    if phase == 1 {
        'phase: {
            let t24: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
            let t25 = [vdupq_n_u32(a0); 2];
            let t26: [uint32x4_t; 2] = [vcgeq_u32(t24[0], t25[0]), vcgeq_u32(t24[1], t25[1])];
            let t27: [uint32x4_t; 2] = [vandq_u32(live[0], t26[0]), vandq_u32(live[1], t26[1])];
            if any(&t27) {
                live = [vbicq_u32(live[0], t27[0]), vbicq_u32(live[1], t27[1])];
            }
            if any(&live) {
                let t28 = [vdupq_n_u64(((0u32) as u64)); 4];
                x80 = t28;
                let mut t29 = live;
                loop {
                    // Lanes part: each its own rounds.
                    let t31: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x80[0]), vmovn_u64(x80[1])), vcombine_u32(vmovn_u64(x80[2]), vmovn_u64(x80[3]))];
                    let t32 = [vdupq_n_u32(7u32); 2];
                    let t33: [uint32x4_t; 2] = [vcltq_u32(t31[0], t32[0]), vcltq_u32(t31[1], t32[1])];
                    let t34 = [vdupq_n_u32(4294967295u32); 2];
                    let t35: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x72[0], t34[0])), vmvnq_u32(vceqq_u32(x72[1], t34[1]))];
                    let t36: [uint32x4_t; 2] = [vandq_u32(t33[0], t35[0]), vandq_u32(t33[1], t35[1])];
                    let t37 = [vdupq_n_u32(2147483648u32); 2];
                    let t38: [uint32x4_t; 2] = [vandq_u32(x72[0], t37[0]), vandq_u32(x72[1], t37[1])];
                    let t39 = [vdupq_n_u32(0u32); 2];
                    let t40: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t38[0], t39[0])), vmvnq_u32(vceqq_u32(t38[1], t39[1]))];
                    let t41: [uint32x4_t; 2] = [vandq_u32(t36[0], t40[0]), vandq_u32(t36[1], t40[1])];
                    let t42 = [vdupq_n_u32((2147483648u32).wrapping_sub(1u32)); 2];
                    let t43: [uint32x4_t; 2] = [vandq_u32(x72[0], t42[0]), vandq_u32(x72[1], t42[1])];
                    let t44: [uint32x4_t; 2] = [vcgeq_u32(t43[0], x64[0]), vcgeq_u32(t43[1], x64[1])];
                    let t45: [uint32x4_t; 2] = [vandq_u32(t41[0], t44[0]), vandq_u32(t41[1], t44[1])];
                    t29 = [vandq_u32(t29[0], t45[0]), vandq_u32(t29[1], t45[1])];
                    if !(any(&t29)) { break; }
                    let t47 = [vdupq_n_u32((2147483648u32).wrapping_sub(1u32)); 2];
                    let t48: [uint32x4_t; 2] = [vandq_u32(x72[0], t47[0]), vandq_u32(x72[1], t47[1])];
                    let t49: [uint32x4_t; 2] = [vsubq_u32(t48[0], x64[0]), vsubq_u32(t48[1], x64[1])];
                    let t50 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(t29[0], t49[0]), vandq_u32(t29[1], t49[1]))));
                    let t51: [uint32x4_t; 2] = [vbslq_u32(t29[0], t49[0], t50), vbslq_u32(t29[1], t49[1], t50)];
                    let t52 = arg6 as *const u32;
                    let mut t53: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                    t53[0] = vld1q_lane_u32::<0>(t52.add(vgetq_lane_u32::<0>(t51[0]) as usize), t53[0]);
                    t53[0] = vld1q_lane_u32::<1>(t52.add(vgetq_lane_u32::<1>(t51[0]) as usize), t53[0]);
                    t53[0] = vld1q_lane_u32::<2>(t52.add(vgetq_lane_u32::<2>(t51[0]) as usize), t53[0]);
                    t53[0] = vld1q_lane_u32::<3>(t52.add(vgetq_lane_u32::<3>(t51[0]) as usize), t53[0]);
                    t53[1] = vld1q_lane_u32::<0>(t52.add(vgetq_lane_u32::<0>(t51[1]) as usize), t53[1]);
                    t53[1] = vld1q_lane_u32::<1>(t52.add(vgetq_lane_u32::<1>(t51[1]) as usize), t53[1]);
                    t53[1] = vld1q_lane_u32::<2>(t52.add(vgetq_lane_u32::<2>(t51[1]) as usize), t53[1]);
                    t53[1] = vld1q_lane_u32::<3>(t52.add(vgetq_lane_u32::<3>(t51[1]) as usize), t53[1]);
                    x72 = [vbslq_u32(t29[0], t53[0], x72[0]), vbslq_u32(t29[1], t53[1], x72[1])];
                    let t54: [uint32x4_t; 2] = [vandq_u32(t29[0], live[0]), vandq_u32(t29[1], live[1])];
                    let t55: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x80[0]), vmovn_u64(x80[1])), vcombine_u32(vmovn_u64(x80[2]), vmovn_u64(x80[3]))];
                    let t56 = [vdupq_n_u32(1u32); 2];
                    let t57: [uint32x4_t; 2] = [vaddq_u32(t55[0], t56[0]), vaddq_u32(t55[1], t56[1])];
                    let t58 = x80;
                    let t59: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t57[0])), vmovl_u32(vget_high_u32(t57[0])), vmovl_u32(vget_low_u32(t57[1])), vmovl_u32(vget_high_u32(t57[1]))];
                    x80 = t59;
                }
                let t60 = [vdupq_n_u32(4294967295u32); 2];
                let t61: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x72[0], t60[0])), vmvnq_u32(vceqq_u32(x72[1], t60[1]))];
                let t62 = [vdupq_n_u32(2147483648u32); 2];
                let t63: [uint32x4_t; 2] = [vandq_u32(x72[0], t62[0]), vandq_u32(x72[1], t62[1])];
                let t64 = [vdupq_n_u32(0u32); 2];
                let t65: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t63[0], t64[0])), vmvnq_u32(vceqq_u32(t63[1], t64[1]))];
                let t66: [uint32x4_t; 2] = [vandq_u32(t61[0], t65[0]), vandq_u32(t61[1], t65[1])];
                let t67: [uint32x4_t; 2] = [vandq_u32(live[0], t66[0]), vandq_u32(live[1], t66[1])];
                if any(&t67) {
                    let t68 = [vdupq_n_u32((2147483648u32).wrapping_sub(1u32)); 2];
                    let t69: [uint32x4_t; 2] = [vandq_u32(x72[0], t68[0]), vandq_u32(x72[1], t68[1])];
                    let t70 = b3_r0r1l_p(l, lid0, t67, arg0, arg1, t69);
                    x72 = [vbslq_u32(t67[0], t70[0], x72[0]), vbslq_u32(t67[1], t70[1], x72[1])];
                }
                let t71 = (arg2 as *mut u32).add((gid0) as usize);
                if all(&live) {
                    vst1q_u32(t71.add(0), x72[0]);
                    vst1q_u32(t71.add(4), x72[1]);
                } else {
                    let mut t72w = 0u32;
                    let t72 = core::hint::black_box(&raw mut t72w);
                    core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t71.add(0), t72).write(vgetq_lane_u32::<0>(x72[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t71.add(1), t72).write(vgetq_lane_u32::<1>(x72[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t71.add(2), t72).write(vgetq_lane_u32::<2>(x72[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t71.add(3), t72).write(vgetq_lane_u32::<3>(x72[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t71.add(4), t72).write(vgetq_lane_u32::<0>(x72[1]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t71.add(5), t72).write(vgetq_lane_u32::<1>(x72[1]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t71.add(6), t72).write(vgetq_lane_u32::<2>(x72[1]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t71.add(7), t72).write(vgetq_lane_u32::<3>(x72[1]));
                }
            }
        }
    }
    if phase + 1 < 2 {
        (st as *mut [uint32x4_t; 2]).write_unaligned(live);
        (st.add(64) as *mut [uint32x4_t; 2]).write_unaligned(x64);
        (st.add(128) as *mut [uint32x4_t; 2]).write_unaligned(x72);
        (st.add(192) as *mut [uint64x2_t; 4]).write_unaligned(x80);
    }
}

#[inline(always)]
unsafe fn b0_r0r1r2sr6gi_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, arg2: *const u8, x24: *const u8, arg6: *const u8, x56: u32, phase: u32, st: *mut u8) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x24 as *const u32).read_unaligned();
    let mut x64: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x72: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x80: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    if phase > 0 {
        live = (st as *const [uint32x4_t; 2]).read_unaligned();
        x64 = (st.add(64) as *const [uint32x4_t; 2]).read_unaligned();
        x72 = (st.add(128) as *const [uint32x4_t; 2]).read_unaligned();
        x80 = (st.add(192) as *const [uint64x2_t; 4]).read_unaligned();
        if !any(&live) { return; }
    }
    if phase == 0 {
        'phase: {
            let t73: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
            let t74: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x56), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x56), vld1q_u32(LANE[1].as_ptr()))];
            let t75: [uint32x4_t; 2] = [vsubq_u32(t73[0], t74[0]), vsubq_u32(t73[1], t74[1])];
            x64 = t75;
            let t76 = [vdupq_n_u32(4294967295u32); 2];
            x72 = t76;
            if (((gid0) as u64 + 7) <= u32::MAX as u64 && ((gid0) as u64) >= ((a0) as u64)) {
            } else if ((gid0) as u64 + 7) < ((a0) as u64) {
                let t77 = b1_r0r1g_f(l, lid0, arg0, arg1);
                x80 = t77;
                let t78 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
                let t79: [uint64x2_t; 4] = [vandq_u64(x80[0], t78[0]), vandq_u64(x80[1], t78[1]), vandq_u64(x80[2], t78[2]), vandq_u64(x80[3], t78[3])];
                let t80 = [vdupq_n_u64(0u64); 4];
                let t81: [uint64x2_t; 4] = [vceqq_u64(t79[0], t80[0]), vceqq_u64(t79[1], t80[1]), vceqq_u64(t79[2], t80[2]), vceqq_u64(t79[3], t80[3])];
                let t82: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t81[0]), vmovn_u64(t81[1])), vcombine_u32(vmovn_u64(t81[2]), vmovn_u64(t81[3]))];
                let t83: [uint32x4_t; 2] = [vmvnq_u32(t82[0]), vmvnq_u32(t82[1])];
                let mut t84: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                let t85: [uint32x4_t; 2] = [vmvnq_u32(t83[0]), vmvnq_u32(t83[1])];
                {
                    let t86: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x80[0]), vmovn_u64(x80[1])), vcombine_u32(vmovn_u64(x80[2]), vmovn_u64(x80[3]))];
                    let t87 = [vdupq_n_u32(2147483648u32); 2];
                    let t88: [uint32x4_t; 2] = [vorrq_u32(t87[0], t86[0]), vorrq_u32(t87[1], t86[1])];
                    t84 = [vbslq_u32(t83[0], t88[0], t84[0]), vbslq_u32(t83[1], t88[1], t84[1])];
                }
                {
                    let t89: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x80[0]), vmovn_u64(x80[1])), vcombine_u32(vmovn_u64(x80[2]), vmovn_u64(x80[3]))];
                    t84 = [vbslq_u32(t85[0], t89[0], t84[0]), vbslq_u32(t85[1], t89[1], t84[1])];
                }
                x72 = t84;
            } else {
                // Lanes part: the chunk's lanes on both sides of the bound.
                let t90 = [vdupq_n_u32(a0); 2];
                let t91: [uint32x4_t; 2] = [vcltq_u32(t73[0], t90[0]), vcltq_u32(t73[1], t90[1])];
                let t92: [uint32x4_t; 2] = [vandq_u32(live[0], t91[0]), vandq_u32(live[1], t91[1])];
                if any(&t92) {
                    let t93 = b1_r0r1g_p(l, lid0, t92, arg0, arg1);
                    x80 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t92[0])))), t93[0], x80[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t92[0])))), t93[1], x80[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t92[1])))), t93[2], x80[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t92[1])))), t93[3], x80[3])];
                    let t94 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
                    let t95: [uint64x2_t; 4] = [vandq_u64(x80[0], t94[0]), vandq_u64(x80[1], t94[1]), vandq_u64(x80[2], t94[2]), vandq_u64(x80[3], t94[3])];
                    let t96 = [vdupq_n_u64(0u64); 4];
                    let t97: [uint64x2_t; 4] = [vceqq_u64(t95[0], t96[0]), vceqq_u64(t95[1], t96[1]), vceqq_u64(t95[2], t96[2]), vceqq_u64(t95[3], t96[3])];
                    let t98: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t97[0]), vmovn_u64(t97[1])), vcombine_u32(vmovn_u64(t97[2]), vmovn_u64(t97[3]))];
                    let t99: [uint32x4_t; 2] = [vmvnq_u32(t98[0]), vmvnq_u32(t98[1])];
                    let mut t100: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                    let t101: [uint32x4_t; 2] = [vandq_u32(t92[0], t99[0]), vandq_u32(t92[1], t99[1])];
                    let t102: [uint32x4_t; 2] = [vbicq_u32(t92[0], t99[0]), vbicq_u32(t92[1], t99[1])];
                    {
                        let t103: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x80[0]), vmovn_u64(x80[1])), vcombine_u32(vmovn_u64(x80[2]), vmovn_u64(x80[3]))];
                        let t104 = [vdupq_n_u32(2147483648u32); 2];
                        let t105: [uint32x4_t; 2] = [vorrq_u32(t104[0], t103[0]), vorrq_u32(t104[1], t103[1])];
                        t100 = [vbslq_u32(t101[0], t105[0], t100[0]), vbslq_u32(t101[1], t105[1], t100[1])];
                    }
                    {
                        let t106: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x80[0]), vmovn_u64(x80[1])), vcombine_u32(vmovn_u64(x80[2]), vmovn_u64(x80[3]))];
                        t100 = [vbslq_u32(t102[0], t106[0], t100[0]), vbslq_u32(t102[1], t106[1], t100[1])];
                    }
                    x72 = [vbslq_u32(t92[0], t100[0], x72[0]), vbslq_u32(t92[1], t100[1], x72[1])];
                }
            }
            let t107 = (arg6 as *mut u32).add((x56) as usize);
            vst1q_u32(t107.add(0), x72[0]);
            vst1q_u32(t107.add(4), x72[1]);
        }
    }
    if phase == 1 {
        'phase: {
            let t108: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
            let t109 = [vdupq_n_u32(a0); 2];
            let t110: [uint32x4_t; 2] = [vcgeq_u32(t108[0], t109[0]), vcgeq_u32(t108[1], t109[1])];
            let t111: [uint32x4_t; 2] = [vandq_u32(live[0], t110[0]), vandq_u32(live[1], t110[1])];
            if any(&t111) {
                live = [vbicq_u32(live[0], t111[0]), vbicq_u32(live[1], t111[1])];
            }
            if any(&live) {
                let t112 = [vdupq_n_u64(((0u32) as u64)); 4];
                x80 = t112;
                let mut t113 = live;
                loop {
                    // Lanes part: each its own rounds.
                    let t115: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x80[0]), vmovn_u64(x80[1])), vcombine_u32(vmovn_u64(x80[2]), vmovn_u64(x80[3]))];
                    let t116 = [vdupq_n_u32(7u32); 2];
                    let t117: [uint32x4_t; 2] = [vcltq_u32(t115[0], t116[0]), vcltq_u32(t115[1], t116[1])];
                    let t118 = [vdupq_n_u32(4294967295u32); 2];
                    let t119: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x72[0], t118[0])), vmvnq_u32(vceqq_u32(x72[1], t118[1]))];
                    let t120: [uint32x4_t; 2] = [vandq_u32(t117[0], t119[0]), vandq_u32(t117[1], t119[1])];
                    let t121 = [vdupq_n_u32(2147483648u32); 2];
                    let t122: [uint32x4_t; 2] = [vandq_u32(x72[0], t121[0]), vandq_u32(x72[1], t121[1])];
                    let t123 = [vdupq_n_u32(0u32); 2];
                    let t124: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t122[0], t123[0])), vmvnq_u32(vceqq_u32(t122[1], t123[1]))];
                    let t125: [uint32x4_t; 2] = [vandq_u32(t120[0], t124[0]), vandq_u32(t120[1], t124[1])];
                    let t126 = [vdupq_n_u32((2147483648u32).wrapping_sub(1u32)); 2];
                    let t127: [uint32x4_t; 2] = [vandq_u32(x72[0], t126[0]), vandq_u32(x72[1], t126[1])];
                    let t128: [uint32x4_t; 2] = [vcgeq_u32(t127[0], x64[0]), vcgeq_u32(t127[1], x64[1])];
                    let t129: [uint32x4_t; 2] = [vandq_u32(t125[0], t128[0]), vandq_u32(t125[1], t128[1])];
                    t113 = [vandq_u32(t113[0], t129[0]), vandq_u32(t113[1], t129[1])];
                    if !(any(&t113)) { break; }
                    let t131 = [vdupq_n_u32((2147483648u32).wrapping_sub(1u32)); 2];
                    let t132: [uint32x4_t; 2] = [vandq_u32(x72[0], t131[0]), vandq_u32(x72[1], t131[1])];
                    let t133: [uint32x4_t; 2] = [vsubq_u32(t132[0], x64[0]), vsubq_u32(t132[1], x64[1])];
                    let t134 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(t113[0], t133[0]), vandq_u32(t113[1], t133[1]))));
                    let t135: [uint32x4_t; 2] = [vbslq_u32(t113[0], t133[0], t134), vbslq_u32(t113[1], t133[1], t134)];
                    let t136 = arg6 as *const u32;
                    let mut t137: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                    t137[0] = vld1q_lane_u32::<0>(t136.add(vgetq_lane_u32::<0>(t135[0]) as usize), t137[0]);
                    t137[0] = vld1q_lane_u32::<1>(t136.add(vgetq_lane_u32::<1>(t135[0]) as usize), t137[0]);
                    t137[0] = vld1q_lane_u32::<2>(t136.add(vgetq_lane_u32::<2>(t135[0]) as usize), t137[0]);
                    t137[0] = vld1q_lane_u32::<3>(t136.add(vgetq_lane_u32::<3>(t135[0]) as usize), t137[0]);
                    t137[1] = vld1q_lane_u32::<0>(t136.add(vgetq_lane_u32::<0>(t135[1]) as usize), t137[1]);
                    t137[1] = vld1q_lane_u32::<1>(t136.add(vgetq_lane_u32::<1>(t135[1]) as usize), t137[1]);
                    t137[1] = vld1q_lane_u32::<2>(t136.add(vgetq_lane_u32::<2>(t135[1]) as usize), t137[1]);
                    t137[1] = vld1q_lane_u32::<3>(t136.add(vgetq_lane_u32::<3>(t135[1]) as usize), t137[1]);
                    x72 = [vbslq_u32(t113[0], t137[0], x72[0]), vbslq_u32(t113[1], t137[1], x72[1])];
                    let t138: [uint32x4_t; 2] = [vandq_u32(t113[0], live[0]), vandq_u32(t113[1], live[1])];
                    let t139: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x80[0]), vmovn_u64(x80[1])), vcombine_u32(vmovn_u64(x80[2]), vmovn_u64(x80[3]))];
                    let t140 = [vdupq_n_u32(1u32); 2];
                    let t141: [uint32x4_t; 2] = [vaddq_u32(t139[0], t140[0]), vaddq_u32(t139[1], t140[1])];
                    let t142 = x80;
                    let t143: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t141[0])), vmovl_u32(vget_high_u32(t141[0])), vmovl_u32(vget_low_u32(t141[1])), vmovl_u32(vget_high_u32(t141[1]))];
                    x80 = t143;
                }
                let t144 = [vdupq_n_u32(4294967295u32); 2];
                let t145: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x72[0], t144[0])), vmvnq_u32(vceqq_u32(x72[1], t144[1]))];
                let t146 = [vdupq_n_u32(2147483648u32); 2];
                let t147: [uint32x4_t; 2] = [vandq_u32(x72[0], t146[0]), vandq_u32(x72[1], t146[1])];
                let t148 = [vdupq_n_u32(0u32); 2];
                let t149: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t147[0], t148[0])), vmvnq_u32(vceqq_u32(t147[1], t148[1]))];
                let t150: [uint32x4_t; 2] = [vandq_u32(t145[0], t149[0]), vandq_u32(t145[1], t149[1])];
                let t151: [uint32x4_t; 2] = [vandq_u32(live[0], t150[0]), vandq_u32(live[1], t150[1])];
                if any(&t151) {
                    let t152 = [vdupq_n_u32((2147483648u32).wrapping_sub(1u32)); 2];
                    let t153: [uint32x4_t; 2] = [vandq_u32(x72[0], t152[0]), vandq_u32(x72[1], t152[1])];
                    let t154 = b3_r0r1l_p(l, lid0, t151, arg0, arg1, t153);
                    x72 = [vbslq_u32(t151[0], t154[0], x72[0]), vbslq_u32(t151[1], t154[1], x72[1])];
                }
                let t155 = (arg2 as *mut u32).add((gid0) as usize);
                if all(&live) {
                    vst1q_u32(t155.add(0), x72[0]);
                    vst1q_u32(t155.add(4), x72[1]);
                } else {
                    let mut t156w = 0u32;
                    let t156 = core::hint::black_box(&raw mut t156w);
                    core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t155.add(0), t156).write(vgetq_lane_u32::<0>(x72[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t155.add(1), t156).write(vgetq_lane_u32::<1>(x72[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t155.add(2), t156).write(vgetq_lane_u32::<2>(x72[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t155.add(3), t156).write(vgetq_lane_u32::<3>(x72[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t155.add(4), t156).write(vgetq_lane_u32::<0>(x72[1]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t155.add(5), t156).write(vgetq_lane_u32::<1>(x72[1]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t155.add(6), t156).write(vgetq_lane_u32::<2>(x72[1]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t155.add(7), t156).write(vgetq_lane_u32::<3>(x72[1]));
                }
            }
        }
    }
    if phase + 1 < 2 {
        (st as *mut [uint32x4_t; 2]).write_unaligned(live);
        (st.add(64) as *mut [uint32x4_t; 2]).write_unaligned(x64);
        (st.add(128) as *mut [uint32x4_t; 2]).write_unaligned(x72);
        (st.add(192) as *mut [uint64x2_t; 4]).write_unaligned(x80);
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
    let t157: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
    let t158 = [vdupq_n_u32(0u32); 2];
    let t159: [uint32x4_t; 2] = [vceqq_u32(t157[0], t158[0]), vceqq_u32(t157[1], t158[1])];
    let t160: [uint32x4_t; 2] = [vandq_u32(live[0], t159[0]), vandq_u32(live[1], t159[1])];
    if any(&t160) {
        let t161 = [vdupq_n_u64(4294967295u64); 4];
        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t160[0])))), t161[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t160[0])))), t161[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t160[1])))), t161[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t160[1])))), t161[3], res[3])];
        live = [vbicq_u32(live[0], t160[0]), vbicq_u32(live[1], t160[1])];
    }
    if any(&live) {
        let t162 = [vdupq_n_u32(1u32); 2];
        let t163: [uint32x4_t; 2] = [vsubq_u32(t157[0], t162[0]), vsubq_u32(t157[1], t162[1])];
        x24 = [vbslq_u32(live[0], t163[0], x24[0]), vbslq_u32(live[1], t163[1], x24[1])];
        let t164 = b4_r0r1l_p(l, lid0, live, arg0, arg1, x24);
        x32 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t164[0], x32[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t164[1], x32[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t164[2], x32[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t164[3], x32[3])];
        let t165 = [vdupq_n_u64(1u64); 4];
        let t166: [uint64x2_t; 4] = [vandq_u64(x32[0], t165[0]), vandq_u64(x32[1], t165[1]), vandq_u64(x32[2], t165[2]), vandq_u64(x32[3], t165[3])];
        let t167 = [vdupq_n_u64(0u64); 4];
        let t168: [uint64x2_t; 4] = [vceqq_u64(t166[0], t167[0]), vceqq_u64(t166[1], t167[1]), vceqq_u64(t166[2], t167[2]), vceqq_u64(t166[3], t167[3])];
        let t169: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t168[0]), vmovn_u64(t168[1])), vcombine_u32(vmovn_u64(t168[2]), vmovn_u64(t168[3]))];
        let t170: [uint32x4_t; 2] = [vandq_u32(live[0], t169[0]), vandq_u32(live[1], t169[1])];
        if any(&t170) {
            let t171: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x24[0])), vmovl_u32(vget_high_u32(x24[0])), vmovl_u32(vget_low_u32(x24[1])), vmovl_u32(vget_high_u32(x24[1]))];
            res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t170[0])))), t171[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t170[0])))), t171[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t170[1])))), t171[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t170[1])))), t171[3], res[3])];
            live = [vbicq_u32(live[0], t170[0]), vbicq_u32(live[1], t170[1])];
        }
        if any(&live) {
            let t172: [uint64x2_t; 4] = [vshlq_u64(x32[0], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[1], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[2], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[3], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64)))];
            let t173: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t172[0]), vmovn_u64(t172[1])), vcombine_u32(vmovn_u64(t172[2]), vmovn_u64(t172[3]))];
            x40 = [vbslq_u32(live[0], t173[0], x40[0]), vbslq_u32(live[1], t173[1], x40[1])];
            let t174: [uint32x4_t; 2] = [vsubq_u32(x40[0], t162[0]), vsubq_u32(x40[1], t162[1])];
            let t175: [uint32x4_t; 2] = [vcgeq_u32(t174[0], t157[0]), vcgeq_u32(t174[1], t157[1])];
            let t176: [uint32x4_t; 2] = [vandq_u32(live[0], t175[0]), vandq_u32(live[1], t175[1])];
            if any(&t176) {
                let t177 = [vdupq_n_u64(4294967295u64); 4];
                res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t176[0])))), t177[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t176[0])))), t177[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t176[1])))), t177[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t176[1])))), t177[3], res[3])];
                live = [vbicq_u32(live[0], t176[0]), vbicq_u32(live[1], t176[1])];
            }
            if any(&live) {
                let t178: [uint32x4_t; 2] = [vsubq_u32(t157[0], x40[0]), vsubq_u32(t157[1], x40[1])];
                x48 = [vbslq_u32(live[0], t178[0], x48[0]), vbslq_u32(live[1], t178[1], x48[1])];
                let t179 = [vdupq_n_u64(2u64); 4];
                let t180: [uint64x2_t; 4] = [vandq_u64(x32[0], t179[0]), vandq_u64(x32[1], t179[1]), vandq_u64(x32[2], t179[2]), vandq_u64(x32[3], t179[3])];
                let t181: [uint64x2_t; 4] = [vceqq_u64(t180[0], t167[0]), vceqq_u64(t180[1], t167[1]), vceqq_u64(t180[2], t167[2]), vceqq_u64(t180[3], t167[3])];
                let t182: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t181[0]), vmovn_u64(t181[1])), vcombine_u32(vmovn_u64(t181[2]), vmovn_u64(t181[3]))];
                let t183: [uint32x4_t; 2] = [vmvnq_u32(t182[0]), vmvnq_u32(t182[1])];
                let t184: [uint32x4_t; 2] = [vandq_u32(live[0], t183[0]), vandq_u32(live[1], t183[1])];
                if any(&t184) {
                    let t185: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                    res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t184[0])))), t185[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t184[0])))), t185[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t184[1])))), t185[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t184[1])))), t185[3], res[3])];
                    live = [vbicq_u32(live[0], t184[0]), vbicq_u32(live[1], t184[1])];
                }
                if any(&live) {
                    let t186: [uint32x4_t; 2] = [vceqq_u32(x48[0], t158[0]), vceqq_u32(x48[1], t158[1])];
                    let t187: [uint32x4_t; 2] = [vandq_u32(live[0], t186[0]), vandq_u32(live[1], t186[1])];
                    if any(&t187) {
                        let t188 = [vdupq_n_u64(4294967295u64); 4];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t187[0])))), t188[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t187[0])))), t188[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t187[1])))), t188[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t187[1])))), t188[3], res[3])];
                        live = [vbicq_u32(live[0], t187[0]), vbicq_u32(live[1], t187[1])];
                    }
                    if any(&live) {
                        let t189 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
                        let t190: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                        let t191: [uint64x2_t; 4] = [vorrq_u64(t189[0], t190[0]), vorrq_u64(t189[1], t190[1]), vorrq_u64(t189[2], t190[2]), vorrq_u64(t189[3], t190[3])];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t191[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t191[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t191[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t191[3], res[3])];
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
        let t192: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t193 = [vdupq_n_u32(1u32); 2];
        let t194: [uint32x4_t; 2] = [vsubq_u32(t192[0], t193[0]), vsubq_u32(t192[1], t193[1])];
        x24 = t194;
        let t195 = b4_r0r1l_f(l, lid0, arg0, arg1, x24);
        x32 = t195;
        let t196 = [vdupq_n_u64(1u64); 4];
        let t197: [uint64x2_t; 4] = [vandq_u64(x32[0], t196[0]), vandq_u64(x32[1], t196[1]), vandq_u64(x32[2], t196[2]), vandq_u64(x32[3], t196[3])];
        let t198 = [vdupq_n_u64(0u64); 4];
        let t199: [uint64x2_t; 4] = [vceqq_u64(t197[0], t198[0]), vceqq_u64(t197[1], t198[1]), vceqq_u64(t197[2], t198[2]), vceqq_u64(t197[3], t198[3])];
        let t200: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t199[0]), vmovn_u64(t199[1])), vcombine_u32(vmovn_u64(t199[2]), vmovn_u64(t199[3]))];
        if any(&t200) {
            let t201: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x24[0])), vmovl_u32(vget_high_u32(x24[0])), vmovl_u32(vget_low_u32(x24[1])), vmovl_u32(vget_high_u32(x24[1]))];
            res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t200[0])))), t201[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t200[0])))), t201[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t200[1])))), t201[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t200[1])))), t201[3], res[3])];
            live = [vbicq_u32(live[0], t200[0]), vbicq_u32(live[1], t200[1])];
        }
        if any(&live) {
            let t202: [uint64x2_t; 4] = [vshlq_u64(x32[0], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[1], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[2], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[3], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64)))];
            let t203: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t202[0]), vmovn_u64(t202[1])), vcombine_u32(vmovn_u64(t202[2]), vmovn_u64(t202[3]))];
            x40 = [vbslq_u32(live[0], t203[0], x40[0]), vbslq_u32(live[1], t203[1], x40[1])];
            let t204: [uint32x4_t; 2] = [vsubq_u32(x40[0], t193[0]), vsubq_u32(x40[1], t193[1])];
            let t205: [uint32x4_t; 2] = [vcgeq_u32(t204[0], t192[0]), vcgeq_u32(t204[1], t192[1])];
            let t206: [uint32x4_t; 2] = [vandq_u32(live[0], t205[0]), vandq_u32(live[1], t205[1])];
            if any(&t206) {
                let t207 = [vdupq_n_u64(4294967295u64); 4];
                res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t206[0])))), t207[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t206[0])))), t207[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t206[1])))), t207[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t206[1])))), t207[3], res[3])];
                live = [vbicq_u32(live[0], t206[0]), vbicq_u32(live[1], t206[1])];
            }
            if any(&live) {
                let t208: [uint32x4_t; 2] = [vsubq_u32(t192[0], x40[0]), vsubq_u32(t192[1], x40[1])];
                x48 = [vbslq_u32(live[0], t208[0], x48[0]), vbslq_u32(live[1], t208[1], x48[1])];
                let t209 = [vdupq_n_u64(2u64); 4];
                let t210: [uint64x2_t; 4] = [vandq_u64(x32[0], t209[0]), vandq_u64(x32[1], t209[1]), vandq_u64(x32[2], t209[2]), vandq_u64(x32[3], t209[3])];
                let t211: [uint64x2_t; 4] = [vceqq_u64(t210[0], t198[0]), vceqq_u64(t210[1], t198[1]), vceqq_u64(t210[2], t198[2]), vceqq_u64(t210[3], t198[3])];
                let t212: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t211[0]), vmovn_u64(t211[1])), vcombine_u32(vmovn_u64(t211[2]), vmovn_u64(t211[3]))];
                let t213: [uint32x4_t; 2] = [vmvnq_u32(t212[0]), vmvnq_u32(t212[1])];
                let t214: [uint32x4_t; 2] = [vandq_u32(live[0], t213[0]), vandq_u32(live[1], t213[1])];
                if any(&t214) {
                    let t215: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                    res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t214[0])))), t215[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t214[0])))), t215[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t214[1])))), t215[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t214[1])))), t215[3], res[3])];
                    live = [vbicq_u32(live[0], t214[0]), vbicq_u32(live[1], t214[1])];
                }
                if any(&live) {
                    let t216 = [vdupq_n_u32(0u32); 2];
                    let t217: [uint32x4_t; 2] = [vceqq_u32(x48[0], t216[0]), vceqq_u32(x48[1], t216[1])];
                    let t218: [uint32x4_t; 2] = [vandq_u32(live[0], t217[0]), vandq_u32(live[1], t217[1])];
                    if any(&t218) {
                        let t219 = [vdupq_n_u64(4294967295u64); 4];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t218[0])))), t219[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t218[0])))), t219[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t218[1])))), t219[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t218[1])))), t219[3], res[3])];
                        live = [vbicq_u32(live[0], t218[0]), vbicq_u32(live[1], t218[1])];
                    }
                    if any(&live) {
                        let t220 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
                        let t221: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                        let t222: [uint64x2_t; 4] = [vorrq_u64(t220[0], t221[0]), vorrq_u64(t220[1], t221[1]), vorrq_u64(t220[2], t221[2]), vorrq_u64(t220[3], t221[3])];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t222[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t222[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t222[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t222[3], res[3])];
                        live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
                    }
                }
            }
        }
    } else if false {
        let t223 = [vdupq_n_u64(4294967295u64); 4];
        res = t223;
        return res;
    } else {
        // Lanes part: the chunk's lanes on both sides of the bound.
        let t224: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t225 = [vdupq_n_u32(0u32); 2];
        let t226: [uint32x4_t; 2] = [vceqq_u32(t224[0], t225[0]), vceqq_u32(t224[1], t225[1])];
        let t227: [uint32x4_t; 2] = [vandq_u32(live[0], t226[0]), vandq_u32(live[1], t226[1])];
        if any(&t227) {
            let t228 = [vdupq_n_u64(4294967295u64); 4];
            res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t227[0])))), t228[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t227[0])))), t228[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t227[1])))), t228[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t227[1])))), t228[3], res[3])];
            live = [vbicq_u32(live[0], t227[0]), vbicq_u32(live[1], t227[1])];
        }
        if any(&live) {
            let t229 = [vdupq_n_u32(1u32); 2];
            let t230: [uint32x4_t; 2] = [vsubq_u32(t224[0], t229[0]), vsubq_u32(t224[1], t229[1])];
            x24 = [vbslq_u32(live[0], t230[0], x24[0]), vbslq_u32(live[1], t230[1], x24[1])];
            let t231 = b4_r0r1l_p(l, lid0, live, arg0, arg1, x24);
            x32 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t231[0], x32[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t231[1], x32[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t231[2], x32[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t231[3], x32[3])];
            let t232 = [vdupq_n_u64(1u64); 4];
            let t233: [uint64x2_t; 4] = [vandq_u64(x32[0], t232[0]), vandq_u64(x32[1], t232[1]), vandq_u64(x32[2], t232[2]), vandq_u64(x32[3], t232[3])];
            let t234 = [vdupq_n_u64(0u64); 4];
            let t235: [uint64x2_t; 4] = [vceqq_u64(t233[0], t234[0]), vceqq_u64(t233[1], t234[1]), vceqq_u64(t233[2], t234[2]), vceqq_u64(t233[3], t234[3])];
            let t236: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t235[0]), vmovn_u64(t235[1])), vcombine_u32(vmovn_u64(t235[2]), vmovn_u64(t235[3]))];
            let t237: [uint32x4_t; 2] = [vandq_u32(live[0], t236[0]), vandq_u32(live[1], t236[1])];
            if any(&t237) {
                let t238: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x24[0])), vmovl_u32(vget_high_u32(x24[0])), vmovl_u32(vget_low_u32(x24[1])), vmovl_u32(vget_high_u32(x24[1]))];
                res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t237[0])))), t238[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t237[0])))), t238[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t237[1])))), t238[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t237[1])))), t238[3], res[3])];
                live = [vbicq_u32(live[0], t237[0]), vbicq_u32(live[1], t237[1])];
            }
            if any(&live) {
                let t239: [uint64x2_t; 4] = [vshlq_u64(x32[0], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[1], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[2], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[3], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64)))];
                let t240: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t239[0]), vmovn_u64(t239[1])), vcombine_u32(vmovn_u64(t239[2]), vmovn_u64(t239[3]))];
                x40 = [vbslq_u32(live[0], t240[0], x40[0]), vbslq_u32(live[1], t240[1], x40[1])];
                let t241: [uint32x4_t; 2] = [vsubq_u32(x40[0], t229[0]), vsubq_u32(x40[1], t229[1])];
                let t242: [uint32x4_t; 2] = [vcgeq_u32(t241[0], t224[0]), vcgeq_u32(t241[1], t224[1])];
                let t243: [uint32x4_t; 2] = [vandq_u32(live[0], t242[0]), vandq_u32(live[1], t242[1])];
                if any(&t243) {
                    let t244 = [vdupq_n_u64(4294967295u64); 4];
                    res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t243[0])))), t244[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t243[0])))), t244[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t243[1])))), t244[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t243[1])))), t244[3], res[3])];
                    live = [vbicq_u32(live[0], t243[0]), vbicq_u32(live[1], t243[1])];
                }
                if any(&live) {
                    let t245: [uint32x4_t; 2] = [vsubq_u32(t224[0], x40[0]), vsubq_u32(t224[1], x40[1])];
                    x48 = [vbslq_u32(live[0], t245[0], x48[0]), vbslq_u32(live[1], t245[1], x48[1])];
                    let t246 = [vdupq_n_u64(2u64); 4];
                    let t247: [uint64x2_t; 4] = [vandq_u64(x32[0], t246[0]), vandq_u64(x32[1], t246[1]), vandq_u64(x32[2], t246[2]), vandq_u64(x32[3], t246[3])];
                    let t248: [uint64x2_t; 4] = [vceqq_u64(t247[0], t234[0]), vceqq_u64(t247[1], t234[1]), vceqq_u64(t247[2], t234[2]), vceqq_u64(t247[3], t234[3])];
                    let t249: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t248[0]), vmovn_u64(t248[1])), vcombine_u32(vmovn_u64(t248[2]), vmovn_u64(t248[3]))];
                    let t250: [uint32x4_t; 2] = [vmvnq_u32(t249[0]), vmvnq_u32(t249[1])];
                    let t251: [uint32x4_t; 2] = [vandq_u32(live[0], t250[0]), vandq_u32(live[1], t250[1])];
                    if any(&t251) {
                        let t252: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t251[0])))), t252[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t251[0])))), t252[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t251[1])))), t252[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t251[1])))), t252[3], res[3])];
                        live = [vbicq_u32(live[0], t251[0]), vbicq_u32(live[1], t251[1])];
                    }
                    if any(&live) {
                        let t253: [uint32x4_t; 2] = [vceqq_u32(x48[0], t225[0]), vceqq_u32(x48[1], t225[1])];
                        let t254: [uint32x4_t; 2] = [vandq_u32(live[0], t253[0]), vandq_u32(live[1], t253[1])];
                        if any(&t254) {
                            let t255 = [vdupq_n_u64(4294967295u64); 4];
                            res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t254[0])))), t255[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t254[0])))), t255[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t254[1])))), t255[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t254[1])))), t255[3], res[3])];
                            live = [vbicq_u32(live[0], t254[0]), vbicq_u32(live[1], t254[1])];
                        }
                        if any(&live) {
                            let t256 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
                            let t257: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                            let t258: [uint64x2_t; 4] = [vorrq_u64(t256[0], t257[0]), vorrq_u64(t256[1], t257[1]), vorrq_u64(t256[2], t257[2]), vorrq_u64(t256[3], t257[3])];
                            res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t258[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t258[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t258[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t258[3], res[3])];
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
unsafe fn b1_r0r1l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x32: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t259 = [vdupq_n_u32(0u32); 2];
    let t260: [uint32x4_t; 2] = [vceqq_u32(x16[0], t259[0]), vceqq_u32(x16[1], t259[1])];
    let t261: [uint32x4_t; 2] = [vandq_u32(live[0], t260[0]), vandq_u32(live[1], t260[1])];
    if any(&t261) {
        let t262 = [vdupq_n_u64(4294967295u64); 4];
        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t261[0])))), t262[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t261[0])))), t262[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t261[1])))), t262[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t261[1])))), t262[3], res[3])];
        live = [vbicq_u32(live[0], t261[0]), vbicq_u32(live[1], t261[1])];
    }
    if any(&live) {
        let t263 = [vdupq_n_u32(1u32); 2];
        let t264: [uint32x4_t; 2] = [vsubq_u32(x16[0], t263[0]), vsubq_u32(x16[1], t263[1])];
        x24 = [vbslq_u32(live[0], t264[0], x24[0]), vbslq_u32(live[1], t264[1], x24[1])];
        let t265 = b4_r0r1l_p(l, lid0, live, arg0, arg1, x24);
        x32 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t265[0], x32[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t265[1], x32[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t265[2], x32[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t265[3], x32[3])];
        let t266 = [vdupq_n_u64(1u64); 4];
        let t267: [uint64x2_t; 4] = [vandq_u64(x32[0], t266[0]), vandq_u64(x32[1], t266[1]), vandq_u64(x32[2], t266[2]), vandq_u64(x32[3], t266[3])];
        let t268 = [vdupq_n_u64(0u64); 4];
        let t269: [uint64x2_t; 4] = [vceqq_u64(t267[0], t268[0]), vceqq_u64(t267[1], t268[1]), vceqq_u64(t267[2], t268[2]), vceqq_u64(t267[3], t268[3])];
        let t270: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t269[0]), vmovn_u64(t269[1])), vcombine_u32(vmovn_u64(t269[2]), vmovn_u64(t269[3]))];
        let t271: [uint32x4_t; 2] = [vandq_u32(live[0], t270[0]), vandq_u32(live[1], t270[1])];
        if any(&t271) {
            let t272: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x24[0])), vmovl_u32(vget_high_u32(x24[0])), vmovl_u32(vget_low_u32(x24[1])), vmovl_u32(vget_high_u32(x24[1]))];
            res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t271[0])))), t272[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t271[0])))), t272[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t271[1])))), t272[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t271[1])))), t272[3], res[3])];
            live = [vbicq_u32(live[0], t271[0]), vbicq_u32(live[1], t271[1])];
        }
        if any(&live) {
            let t273: [uint64x2_t; 4] = [vshlq_u64(x32[0], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[1], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[2], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[3], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64)))];
            let t274: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t273[0]), vmovn_u64(t273[1])), vcombine_u32(vmovn_u64(t273[2]), vmovn_u64(t273[3]))];
            x40 = [vbslq_u32(live[0], t274[0], x40[0]), vbslq_u32(live[1], t274[1], x40[1])];
            let t275: [uint32x4_t; 2] = [vsubq_u32(x40[0], t263[0]), vsubq_u32(x40[1], t263[1])];
            let t276: [uint32x4_t; 2] = [vcgeq_u32(t275[0], x16[0]), vcgeq_u32(t275[1], x16[1])];
            let t277: [uint32x4_t; 2] = [vandq_u32(live[0], t276[0]), vandq_u32(live[1], t276[1])];
            if any(&t277) {
                let t278 = [vdupq_n_u64(4294967295u64); 4];
                res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t277[0])))), t278[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t277[0])))), t278[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t277[1])))), t278[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t277[1])))), t278[3], res[3])];
                live = [vbicq_u32(live[0], t277[0]), vbicq_u32(live[1], t277[1])];
            }
            if any(&live) {
                let t279: [uint32x4_t; 2] = [vsubq_u32(x16[0], x40[0]), vsubq_u32(x16[1], x40[1])];
                x48 = [vbslq_u32(live[0], t279[0], x48[0]), vbslq_u32(live[1], t279[1], x48[1])];
                let t280 = [vdupq_n_u64(2u64); 4];
                let t281: [uint64x2_t; 4] = [vandq_u64(x32[0], t280[0]), vandq_u64(x32[1], t280[1]), vandq_u64(x32[2], t280[2]), vandq_u64(x32[3], t280[3])];
                let t282: [uint64x2_t; 4] = [vceqq_u64(t281[0], t268[0]), vceqq_u64(t281[1], t268[1]), vceqq_u64(t281[2], t268[2]), vceqq_u64(t281[3], t268[3])];
                let t283: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t282[0]), vmovn_u64(t282[1])), vcombine_u32(vmovn_u64(t282[2]), vmovn_u64(t282[3]))];
                let t284: [uint32x4_t; 2] = [vmvnq_u32(t283[0]), vmvnq_u32(t283[1])];
                let t285: [uint32x4_t; 2] = [vandq_u32(live[0], t284[0]), vandq_u32(live[1], t284[1])];
                if any(&t285) {
                    let t286: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                    res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t285[0])))), t286[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t285[0])))), t286[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t285[1])))), t286[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t285[1])))), t286[3], res[3])];
                    live = [vbicq_u32(live[0], t285[0]), vbicq_u32(live[1], t285[1])];
                }
                if any(&live) {
                    let t287: [uint32x4_t; 2] = [vceqq_u32(x48[0], t259[0]), vceqq_u32(x48[1], t259[1])];
                    let t288: [uint32x4_t; 2] = [vandq_u32(live[0], t287[0]), vandq_u32(live[1], t287[1])];
                    if any(&t288) {
                        let t289 = [vdupq_n_u64(4294967295u64); 4];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t288[0])))), t289[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t288[0])))), t289[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t288[1])))), t289[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t288[1])))), t289[3], res[3])];
                        live = [vbicq_u32(live[0], t288[0]), vbicq_u32(live[1], t288[1])];
                    }
                    if any(&live) {
                        let t290 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
                        let t291: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                        let t292: [uint64x2_t; 4] = [vorrq_u64(t290[0], t291[0]), vorrq_u64(t290[1], t291[1]), vorrq_u64(t290[2], t291[2]), vorrq_u64(t290[3], t291[3])];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t292[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t292[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t292[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t292[3], res[3])];
                        live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
                    }
                }
            }
        }
    }
    res
}

#[inline(always)]
unsafe fn b3_r0r1l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t293 = b1_r0r1l_p(l, lid0, live, arg0, arg1, x16);
    x24 = t293;
    let mut t294 = live;
    loop {
        // Lanes part: each its own rounds.
        let t296 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
        let t297: [uint64x2_t; 4] = [vandq_u64(x24[0], t296[0]), vandq_u64(x24[1], t296[1]), vandq_u64(x24[2], t296[2]), vandq_u64(x24[3], t296[3])];
        let t298 = [vdupq_n_u64(0u64); 4];
        let t299: [uint64x2_t; 4] = [vceqq_u64(t297[0], t298[0]), vceqq_u64(t297[1], t298[1]), vceqq_u64(t297[2], t298[2]), vceqq_u64(t297[3], t298[3])];
        let t300: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t299[0]), vmovn_u64(t299[1])), vcombine_u32(vmovn_u64(t299[2]), vmovn_u64(t299[3]))];
        let t301: [uint32x4_t; 2] = [vmvnq_u32(t300[0]), vmvnq_u32(t300[1])];
        t294 = [vandq_u32(t294[0], t301[0]), vandq_u32(t294[1], t301[1])];
        if !(any(&t294)) { break; }
        let t303: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x24[0]), vmovn_u64(x24[1])), vcombine_u32(vmovn_u64(x24[2]), vmovn_u64(x24[3]))];
        let t304 = b1_r0r1l_p(l, lid0, t294, arg0, arg1, t303);
        x24 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t294[0])))), t304[0], x24[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t294[0])))), t304[1], x24[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t294[1])))), t304[2], x24[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t294[1])))), t304[3], x24[3])];
    }
    let t305: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x24[0]), vmovn_u64(x24[1])), vcombine_u32(vmovn_u64(x24[2]), vmovn_u64(x24[3]))];
    res = t305;
    return res;
}

#[inline(always)]
unsafe fn b4_r0r1l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t306 = b5_r0r1sl_p(l, lid0, live, arg0, arg1, 0u32, x16);
    res = t306;
    return res;
}

#[inline(always)]
unsafe fn b4_r0r1l_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t307 = b5_r0r1sl_f(l, lid0, arg0, arg1, 0u32, x16);
    res = t307;
    return res;
}

#[inline(always)]
unsafe fn b5_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: u64 = 0;
    let mut x56: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t308 = b6_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    x32 = t308;
    let t309: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t310 = b7_r0r1sl_p(l, lid0, live, arg0, arg1, (1u32).wrapping_add(x16), t309);
    x40 = t310;
    let t311 = b8_s_p(l, lid0, live, x32);
    x48 = t311;
    let t312 = [vdupq_n_u32((1024u32).wrapping_sub(1u32)); 2];
    let t313: [uint32x4_t; 2] = [vandq_u32(x24[0], t312[0]), vandq_u32(x24[1], t312[1])];
    let t314 = [vdupq_n_u32(x32); 2];
    let t315: [uint32x4_t; 2] = [vmulq_u32(t313[0], t314[0]), vmulq_u32(t313[1], t314[1])];
    let t316 = b9_r0r1lls_p(l, lid0, live, arg0, arg1, x40, t315, 8u32);
    let t317 = [vdupq_n_u64(x48); 4];
    let t318: [uint64x2_t; 4] = [vandq_u64(t316[0], t317[0]), vandq_u64(t316[1], t317[1]), vandq_u64(t316[2], t317[2]), vandq_u64(t316[3], t317[3])];
    x56 = t318;
    let t319 = [vdupq_n_u64(x48); 4];
    let t320: [uint64x2_t; 4] = [vceqq_u64(x56[0], t319[0]), vceqq_u64(x56[1], t319[1]), vceqq_u64(x56[2], t319[2]), vceqq_u64(x56[3], t319[3])];
    let t321: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t320[0]), vmovn_u64(t320[1])), vcombine_u32(vmovn_u64(t320[2]), vmovn_u64(t320[3]))];
    let mut t322: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t323: [uint32x4_t; 2] = [vandq_u32(live[0], t321[0]), vandq_u32(live[1], t321[1])];
    let t324: [uint32x4_t; 2] = [vbicq_u32(live[0], t321[0]), vbicq_u32(live[1], t321[1])];
    if any(&t323) {
        let t325 = b10_r0r1sl_p(l, lid0, t323, arg0, arg1, (x16).wrapping_add(1u32), x24);
        t322 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t323[0])))), t325[0], t322[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t323[0])))), t325[1], t322[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t323[1])))), t325[2], t322[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t323[1])))), t325[3], t322[3])];
    }
    {
        t322 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t324[0])))), x56[0], t322[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t324[0])))), x56[1], t322[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t324[1])))), x56[2], t322[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t324[1])))), x56[3], t322[3])];
    }
    res = t322;
    return res;
}

#[inline(always)]
unsafe fn b5_r0r1sl_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: u64 = 0;
    let mut x56: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t326 = b6_r0r1s_f(l, lid0, arg0, arg1, x16);
    x32 = t326;
    let t327: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t328 = b7_r0r1sl_f(l, lid0, arg0, arg1, (1u32).wrapping_add(x16), t327);
    x40 = t328;
    let t329 = b8_s_f(l, lid0, x32);
    x48 = t329;
    let t330 = [vdupq_n_u32((1024u32).wrapping_sub(1u32)); 2];
    let t331: [uint32x4_t; 2] = [vandq_u32(x24[0], t330[0]), vandq_u32(x24[1], t330[1])];
    let t332 = [vdupq_n_u32(x32); 2];
    let t333: [uint32x4_t; 2] = [vmulq_u32(t331[0], t332[0]), vmulq_u32(t331[1], t332[1])];
    let t334 = b9_r0r1lls_f(l, lid0, arg0, arg1, x40, t333, 8u32);
    let t335 = [vdupq_n_u64(x48); 4];
    let t336: [uint64x2_t; 4] = [vandq_u64(t334[0], t335[0]), vandq_u64(t334[1], t335[1]), vandq_u64(t334[2], t335[2]), vandq_u64(t334[3], t335[3])];
    x56 = t336;
    let t337 = [vdupq_n_u64(x48); 4];
    let t338: [uint64x2_t; 4] = [vceqq_u64(x56[0], t337[0]), vceqq_u64(x56[1], t337[1]), vceqq_u64(x56[2], t337[2]), vceqq_u64(x56[3], t337[3])];
    let t339: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t338[0]), vmovn_u64(t338[1])), vcombine_u32(vmovn_u64(t338[2]), vmovn_u64(t338[3]))];
    let mut t340: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t341: [uint32x4_t; 2] = [vmvnq_u32(t339[0]), vmvnq_u32(t339[1])];
    if any(&t339) {
        let t342 = b10_r0r1sl_p(l, lid0, t339, arg0, arg1, (x16).wrapping_add(1u32), x24);
        t340 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t339[0])))), t342[0], t340[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t339[0])))), t342[1], t340[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t339[1])))), t342[2], t340[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t339[1])))), t342[3], t340[3])];
    }
    {
        t340 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t341[0])))), x56[0], t340[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t341[0])))), x56[1], t340[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t341[1])))), x56[2], t340[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t341[1])))), x56[3], t340[3])];
    }
    res = t340;
    return res;
}

#[inline(always)]
unsafe fn b6_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t343 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    res = (((t343).wrapping_shr(((4u32).wrapping_mul(x16)) as u32)) & (15u32));
    return res;
}

#[inline(always)]
unsafe fn b6_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t344 = b11_r0r1s_f(l, lid0, arg0, arg1, 16u32);
    res = (((t344).wrapping_shr(((4u32).wrapping_mul(x16)) as u32)) & (15u32));
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t345 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 40u32);
    let t346 = [vdupq_n_u32(t345); 2];
    let t347: [uint32x4_t; 2] = [vmulq_u32(x24[0], t346[0]), vmulq_u32(x24[1], t346[1])];
    let t348 = b12_r0r1s_p(l, lid0, live, arg0, arg1, 0u32);
    let t349 = [vdupq_n_u32(t348); 2];
    let t350: [uint32x4_t; 2] = [vaddq_u32(t347[0], t349[0]), vaddq_u32(t347[1], t349[1])];
    x32 = t350;
    if ((x16) == (0u32)) {
        res = x32;
        return res;
    }
    let t351 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    let t352: [uint32x4_t; 2] = [vshlq_u32(x24[0], vdupq_n_s32(((10u32) & 31) as i32)), vshlq_u32(x24[1], vdupq_n_s32(((10u32) & 31) as i32))];
    let t353 = [vdupq_n_u32(t351); 2];
    let t354: [uint32x4_t; 2] = [vsubq_u32(t353[0], t352[0]), vsubq_u32(t353[1], t352[1])];
    let t355 = b13_ls_p(l, lid0, live, t354, 1024u32);
    x40 = t355;
    let t356 = b12_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    let t357 = [vdupq_n_u32(t356); 2];
    let t358: [uint32x4_t; 2] = [vmulq_u32(x40[0], t357[0]), vmulq_u32(x40[1], t357[1])];
    let t359: [uint32x4_t; 2] = [vaddq_u32(x32[0], t358[0]), vaddq_u32(x32[1], t358[1])];
    res = t359;
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1sl_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t360 = b11_r0r1s_f(l, lid0, arg0, arg1, 40u32);
    let t361 = [vdupq_n_u32(t360); 2];
    let t362: [uint32x4_t; 2] = [vmulq_u32(x24[0], t361[0]), vmulq_u32(x24[1], t361[1])];
    let t363 = b12_r0r1s_f(l, lid0, arg0, arg1, 0u32);
    let t364 = [vdupq_n_u32(t363); 2];
    let t365: [uint32x4_t; 2] = [vaddq_u32(t362[0], t364[0]), vaddq_u32(t362[1], t364[1])];
    x32 = t365;
    if ((x16) == (0u32)) {
        res = x32;
        return res;
    }
    let t366 = b11_r0r1s_f(l, lid0, arg0, arg1, 4u32);
    let t367: [uint32x4_t; 2] = [vshlq_u32(x24[0], vdupq_n_s32(((10u32) & 31) as i32)), vshlq_u32(x24[1], vdupq_n_s32(((10u32) & 31) as i32))];
    let t368 = [vdupq_n_u32(t366); 2];
    let t369: [uint32x4_t; 2] = [vsubq_u32(t368[0], t367[0]), vsubq_u32(t368[1], t367[1])];
    let t370 = b13_ls_f(l, lid0, t369, 1024u32);
    x40 = t370;
    let t371 = b12_r0r1s_f(l, lid0, arg0, arg1, x16);
    let t372 = [vdupq_n_u32(t371); 2];
    let t373: [uint32x4_t; 2] = [vmulq_u32(x40[0], t372[0]), vmulq_u32(x40[1], t372[1])];
    let t374: [uint32x4_t; 2] = [vaddq_u32(x32[0], t373[0]), vaddq_u32(x32[1], t373[1])];
    res = t374;
    return res;
}

#[inline(always)]
unsafe fn b8_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t375: u64;
    if ((x0) >= (8u32)) {
        t375 = !(0u64);
    } else {
        t375 = ((1u64).wrapping_shl(((((x0).wrapping_mul(8u32)) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t375;
    return res;
}

#[inline(always)]
unsafe fn b8_s_f(l: &Launch, lid0: u32, x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t376: u64;
    if ((x0) >= (8u32)) {
        t376 = !(0u64);
    } else {
        t376 = ((1u64).wrapping_shl(((((x0).wrapping_mul(8u32)) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t376;
    return res;
}

#[inline(always)]
unsafe fn b8_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t377 = [vdupq_n_u32(8u32); 2];
    let t378: [uint32x4_t; 2] = [vcgeq_u32(x0[0], t377[0]), vcgeq_u32(x0[1], t377[1])];
    let mut t379: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t380: [uint32x4_t; 2] = [vandq_u32(live[0], t378[0]), vandq_u32(live[1], t378[1])];
    let t381: [uint32x4_t; 2] = [vbicq_u32(live[0], t378[0]), vbicq_u32(live[1], t378[1])];
    {
        let t382 = [vdupq_n_u64(!(0u64)); 4];
        t379 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t380[0])))), t382[0], t379[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t380[0])))), t382[1], t379[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t380[1])))), t382[2], t379[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t380[1])))), t382[3], t379[3])];
    }
    {
        let t383: [uint32x4_t; 2] = [vmulq_u32(x0[0], t377[0]), vmulq_u32(x0[1], t377[1])];
        let t384 = [vdupq_n_u64(1u64); 4];
        let t385: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t383[0])), vmovl_u32(vget_high_u32(t383[0])), vmovl_u32(vget_low_u32(t383[1])), vmovl_u32(vget_high_u32(t383[1]))];
        let t386: [uint64x2_t; 4] = [vshlq_u64(t384[0], vreinterpretq_s64_u64(vandq_u64(t385[0], vdupq_n_u64(63)))), vshlq_u64(t384[1], vreinterpretq_s64_u64(vandq_u64(t385[1], vdupq_n_u64(63)))), vshlq_u64(t384[2], vreinterpretq_s64_u64(vandq_u64(t385[2], vdupq_n_u64(63)))), vshlq_u64(t384[3], vreinterpretq_s64_u64(vandq_u64(t385[3], vdupq_n_u64(63))))];
        let t387: [uint64x2_t; 4] = [vsubq_u64(t386[0], t384[0]), vsubq_u64(t386[1], t384[1]), vsubq_u64(t386[2], t384[2]), vsubq_u64(t386[3], t384[3])];
        t379 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t381[0])))), t387[0], t379[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t381[0])))), t387[1], t379[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t381[1])))), t387[2], t379[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t381[1])))), t387[3], t379[3])];
    }
    res = t379;
    return res;
}

#[inline(always)]
unsafe fn b9_r0r1lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2], x24: [uint32x4_t; 2], x32: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t388 = b14_r0lls_p(l, lid0, live, arg0, x16, x24, x32);
    res = t388;
    return res;
}

#[inline(always)]
unsafe fn b9_r0r1lls_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2], x24: [uint32x4_t; 2], x32: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t389 = b14_r0lls_f(l, lid0, arg0, x16, x24, x32);
    res = t389;
    return res;
}

#[inline(always)]
unsafe fn b10_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
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
    let t390 = b11_r0r1s_p(l, lid0, live, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x16)));
    x32 = t390;
    let t391 = b11_r0r1s_p(l, lid0, live, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x16)));
    x40 = t391;
    let t392: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    x48 = t392;
    let t393 = [vdupq_n_u32(4u32); 2];
    let t394: [uint32x4_t; 2] = [vmulq_u32(x48[0], t393[0]), vmulq_u32(x48[1], t393[1])];
    let t395 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x40, t394, 4u32);
    let t396: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t395[0]), vmovn_u64(t395[1])), vcombine_u32(vmovn_u64(t395[2]), vmovn_u64(t395[3]))];
    x56 = t396;
    let t397: [uint32x4_t; 2] = [vmulq_u32(x48[0], t393[0]), vmulq_u32(x48[1], t393[1])];
    let t398: [uint32x4_t; 2] = [vaddq_u32(t397[0], t393[0]), vaddq_u32(t397[1], t393[1])];
    let t399 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x40, t398, 4u32);
    let t400: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t399[0]), vmovn_u64(t399[1])), vcombine_u32(vmovn_u64(t399[2]), vmovn_u64(t399[3]))];
    x64 = t400;
    let t401 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    x72 = ((t401).wrapping_add((256u32).wrapping_sub(1u32))).wrapping_shr((8u32) as u32);
    let t402: [uint32x4_t; 2] = [vmulq_u32(x48[0], t393[0]), vmulq_u32(x48[1], t393[1])];
    let t403 = [vdupq_n_u32(x72); 2];
    let t404: [uint32x4_t; 2] = [vsubq_u32(t403[0], t402[0]), vsubq_u32(t403[1], t402[1])];
    let t405 = b16_sl_p(l, lid0, live, 4u32, t404);
    x80 = t405;
    let t406: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
    let t407 = [vdupq_n_u32((4u32).wrapping_sub(1u32)); 2];
    let t408: [uint32x4_t; 2] = [vandq_u32(t406[0], t407[0]), vandq_u32(t406[1], t407[1])];
    x88 = t408;
    let t409 = [vdupq_n_u32(0u32); 2];
    let t410: [uint32x4_t; 2] = [vceqq_u32(x88[0], t409[0]), vceqq_u32(x88[1], t409[1])];
    let mut t411: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t412: [uint32x4_t; 2] = [vandq_u32(live[0], t410[0]), vandq_u32(live[1], t410[1])];
    let t413: [uint32x4_t; 2] = [vbicq_u32(live[0], t410[0]), vbicq_u32(live[1], t410[1])];
    {
        t411 = [vbslq_u32(t412[0], t409[0], t411[0]), vbslq_u32(t412[1], t409[1], t411[1])];
    }
    if any(&t413) {
        let t414 = [vdupq_n_u32(1u32); 2];
        let t415: [uint32x4_t; 2] = [vsubq_u32(x88[0], t414[0]), vsubq_u32(x88[1], t414[1])];
        let t416 = [vdupq_n_u32(2u32); 2];
        let t417: [uint32x4_t; 2] = [vmulq_u32(t416[0], t415[0]), vmulq_u32(t416[1], t415[1])];
        let t418: [uint32x4_t; 2] = [vaddq_u32(x56[0], t417[0]), vaddq_u32(x56[1], t417[1])];
        let t419 = b17_ll_p(l, lid0, t413, t418, x64);
        let t420 = b15_r0r1sls_p(l, lid0, t413, arg0, arg1, x32, t419, 2u32);
        let t421: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t420[0]), vmovn_u64(t420[1])), vcombine_u32(vmovn_u64(t420[2]), vmovn_u64(t420[3]))];
        t411 = [vbslq_u32(t413[0], t421[0], t411[0]), vbslq_u32(t413[1], t421[1], t411[1])];
    }
    x96 = t411;
    let t422 = [vdupq_n_u32(2u32); 2];
    let t423: [uint32x4_t; 2] = [vmulq_u32(t422[0], x88[0]), vmulq_u32(t422[1], x88[1])];
    let t424: [uint32x4_t; 2] = [vaddq_u32(x56[0], t423[0]), vaddq_u32(x56[1], t423[1])];
    let t425 = b17_ll_p(l, lid0, live, t424, x64);
    let t426 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t425, 2u32);
    let t427: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t426[0]), vmovn_u64(t426[1])), vcombine_u32(vmovn_u64(t426[2]), vmovn_u64(t426[3]))];
    x104 = t427;
    let t428: [uint32x4_t; 2] = [vmulq_u32(t422[0], x80[0]), vmulq_u32(t422[1], x80[1])];
    let t429: [uint32x4_t; 2] = [vaddq_u32(x56[0], t428[0]), vaddq_u32(x56[1], t428[1])];
    x112 = t429;
    let t430 = [vdupq_n_u32((256u32).wrapping_sub(1u32)); 2];
    let t431: [uint32x4_t; 2] = [vandq_u32(x24[0], t430[0]), vandq_u32(x24[1], t430[1])];
    x120 = t431;
    x128 = 9259542123273814144u64;
    let t432: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x120[0])), vmovl_u32(vget_high_u32(x120[0])), vmovl_u32(vget_low_u32(x120[1])), vmovl_u32(vget_high_u32(x120[1]))];
    let t433 = [vdupq_n_u64(72340172838076673u64); 4];
    let mut t434: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    t434[0] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t432[0]) as u64).wrapping_mul(vgetq_lane_u64::<0>(t433[0]) as u64) as u64, t434[0]);
    t434[0] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t432[0]) as u64).wrapping_mul(vgetq_lane_u64::<1>(t433[0]) as u64) as u64, t434[0]);
    t434[1] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t432[1]) as u64).wrapping_mul(vgetq_lane_u64::<0>(t433[1]) as u64) as u64, t434[1]);
    t434[1] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t432[1]) as u64).wrapping_mul(vgetq_lane_u64::<1>(t433[1]) as u64) as u64, t434[1]);
    t434[2] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t432[2]) as u64).wrapping_mul(vgetq_lane_u64::<0>(t433[2]) as u64) as u64, t434[2]);
    t434[2] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t432[2]) as u64).wrapping_mul(vgetq_lane_u64::<1>(t433[2]) as u64) as u64, t434[2]);
    t434[3] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t432[3]) as u64).wrapping_mul(vgetq_lane_u64::<0>(t433[3]) as u64) as u64, t434[3]);
    t434[3] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t432[3]) as u64).wrapping_mul(vgetq_lane_u64::<1>(t433[3]) as u64) as u64, t434[3]);
    x136 = t434;
    x144 = x96;
    x152 = x96;
    let mut t435 = live;
    loop {
        // Lanes part: each its own rounds.
        let t437: [uint32x4_t; 2] = [vcltq_u32(x152[0], x104[0]), vcltq_u32(x152[1], x104[1])];
        t435 = [vandq_u32(t435[0], t437[0]), vandq_u32(t435[1], t437[1])];
        if !(any(&t435)) { break; }
        let t439: [uint32x4_t; 2] = [vaddq_u32(x112[0], x152[0]), vaddq_u32(x112[1], x152[1])];
        let t440 = b17_ll_p(l, lid0, t435, t439, x64);
        let t441 = b15_r0r1sls_p(l, lid0, t435, arg0, arg1, x32, t440, 8u32);
        x160 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t435[0])))), t441[0], x160[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t435[0])))), t441[1], x160[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t435[1])))), t441[2], x160[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t435[1])))), t441[3], x160[3])];
        let t442 = [vdupq_n_u64(x128); 4];
        let t443: [uint64x2_t; 4] = [vorrq_u64(x160[0], t442[0]), vorrq_u64(x160[1], t442[1]), vorrq_u64(x160[2], t442[2]), vorrq_u64(x160[3], t442[3])];
        let t444 = [vdupq_n_u64(!(x128)); 4];
        let t445: [uint64x2_t; 4] = [vandq_u64(x136[0], t444[0]), vandq_u64(x136[1], t444[1]), vandq_u64(x136[2], t444[2]), vandq_u64(x136[3], t444[3])];
        let t446: [uint64x2_t; 4] = [vsubq_u64(t443[0], t445[0]), vsubq_u64(t443[1], t445[1]), vsubq_u64(t443[2], t445[2]), vsubq_u64(t443[3], t445[3])];
        x168 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t435[0])))), t446[0], x168[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t435[0])))), t446[1], x168[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t435[1])))), t446[2], x168[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t435[1])))), t446[3], x168[3])];
        let t447: [uint64x2_t; 4] = [vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x160[0]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x160[1]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x160[2]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x160[3])))];
        let t448: [uint64x2_t; 4] = [vandq_u64(t447[0], x136[0]), vandq_u64(t447[1], x136[1]), vandq_u64(t447[2], x136[2]), vandq_u64(t447[3], x136[3])];
        let t449: [uint64x2_t; 4] = [veorq_u64(x160[0], x136[0]), veorq_u64(x160[1], x136[1]), veorq_u64(x160[2], x136[2]), veorq_u64(x160[3], x136[3])];
        let t450: [uint64x2_t; 4] = [vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(t449[0]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(t449[1]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(t449[2]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(t449[3])))];
        let t451: [uint64x2_t; 4] = [vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x168[0]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x168[1]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x168[2]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x168[3])))];
        let t452: [uint64x2_t; 4] = [vandq_u64(t450[0], t451[0]), vandq_u64(t450[1], t451[1]), vandq_u64(t450[2], t451[2]), vandq_u64(t450[3], t451[3])];
        let t453: [uint64x2_t; 4] = [vorrq_u64(t448[0], t452[0]), vorrq_u64(t448[1], t452[1]), vorrq_u64(t448[2], t452[2]), vorrq_u64(t448[3], t452[3])];
        let t454 = [vdupq_n_u64(x128); 4];
        let t455: [uint64x2_t; 4] = [vandq_u64(t454[0], t453[0]), vandq_u64(t454[1], t453[1]), vandq_u64(t454[2], t453[2]), vandq_u64(t454[3], t453[3])];
        x176 = t455;
        let t456: [uint32x4_t; 2] = [vsubq_u32(x104[0], x152[0]), vsubq_u32(x104[1], x152[1])];
        x184 = t456;
        let t457 = [vdupq_n_u32(8u32); 2];
        let t458: [uint32x4_t; 2] = [vcgeq_u32(x184[0], t457[0]), vcgeq_u32(x184[1], t457[1])];
        let mut t459: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
        let t460: [uint32x4_t; 2] = [vandq_u32(t435[0], t458[0]), vandq_u32(t435[1], t458[1])];
        let t461: [uint32x4_t; 2] = [vbicq_u32(t435[0], t458[0]), vbicq_u32(t435[1], t458[1])];
        {
            t459 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t460[0])))), x176[0], t459[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t460[0])))), x176[1], t459[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t460[1])))), x176[2], t459[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t460[1])))), x176[3], t459[3])];
        }
        {
            let t462: [uint32x4_t; 2] = [vmulq_u32(t457[0], x184[0]), vmulq_u32(t457[1], x184[1])];
            let t463 = [vdupq_n_u64(1u64); 4];
            let t464: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t462[0])), vmovl_u32(vget_high_u32(t462[0])), vmovl_u32(vget_low_u32(t462[1])), vmovl_u32(vget_high_u32(t462[1]))];
            let t465: [uint64x2_t; 4] = [vshlq_u64(t463[0], vreinterpretq_s64_u64(vandq_u64(t464[0], vdupq_n_u64(63)))), vshlq_u64(t463[1], vreinterpretq_s64_u64(vandq_u64(t464[1], vdupq_n_u64(63)))), vshlq_u64(t463[2], vreinterpretq_s64_u64(vandq_u64(t464[2], vdupq_n_u64(63)))), vshlq_u64(t463[3], vreinterpretq_s64_u64(vandq_u64(t464[3], vdupq_n_u64(63))))];
            let t466: [uint64x2_t; 4] = [vsubq_u64(t465[0], t463[0]), vsubq_u64(t465[1], t463[1]), vsubq_u64(t465[2], t463[2]), vsubq_u64(t465[3], t463[3])];
            let t467: [uint64x2_t; 4] = [vandq_u64(x176[0], t466[0]), vandq_u64(x176[1], t466[1]), vandq_u64(x176[2], t466[2]), vandq_u64(x176[3], t466[3])];
            t459 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t461[0])))), t467[0], t459[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t461[0])))), t467[1], t459[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t461[1])))), t467[2], t459[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t461[1])))), t467[3], t459[3])];
        }
        let t468 = b18_l_p(l, lid0, t435, t459);
        let t469: [uint32x4_t; 2] = [vaddq_u32(x144[0], t468[0]), vaddq_u32(x144[1], t468[1])];
        x144 = [vbslq_u32(t435[0], t469[0], x144[0]), vbslq_u32(t435[1], t469[1], x144[1])];
        let t470: [uint32x4_t; 2] = [vandq_u32(t435[0], live[0]), vandq_u32(t435[1], live[1])];
        x152 = [vaddq_u32(x152[0], vandq_u32(t470[0], vdupq_n_u32(8))), vaddq_u32(x152[1], vandq_u32(t470[1], vdupq_n_u32(8)))];
    }
    x96 = x144;
    let t471 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    let t472 = [vdupq_n_u32((((t471).wrapping_shr((((((4u32).wrapping_mul(x16)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32))); 2];
    x152 = t472;
    let t473 = [vdupq_n_u32(1u32); 2];
    let t474: [uint32x4_t; 2] = [vsubq_u32(x80[0], t473[0]), vsubq_u32(x80[1], t473[1])];
    let t475: [uint32x4_t; 2] = [vmulq_u32(t422[0], t474[0]), vmulq_u32(t422[1], t474[1])];
    let t476: [uint32x4_t; 2] = [vaddq_u32(x56[0], t475[0]), vaddq_u32(x56[1], t475[1])];
    let t477 = b17_ll_p(l, lid0, live, t476, x64);
    let t478 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t477, 2u32);
    let t479: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t478[0]), vmovn_u64(t478[1])), vcombine_u32(vmovn_u64(t478[2]), vmovn_u64(t478[3]))];
    let t480: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t479[0])), vmovl_u32(vget_high_u32(t479[0])), vmovl_u32(vget_low_u32(t479[1])), vmovl_u32(vget_high_u32(t479[1]))];
    x160 = t480;
    let t481: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x160[0]), vmovn_u64(x160[1])), vcombine_u32(vmovn_u64(x160[2]), vmovn_u64(x160[3]))];
    let t482: [uint32x4_t; 2] = [vaddq_u32(x112[0], t481[0]), vaddq_u32(x112[1], t481[1])];
    let t483: [uint32x4_t; 2] = [vmulq_u32(x96[0], x152[0]), vmulq_u32(x96[1], x152[1])];
    let t484: [uint32x4_t; 2] = [vaddq_u32(t482[0], t483[0]), vaddq_u32(t482[1], t483[1])];
    let t485 = b17_ll_p(l, lid0, live, t484, x64);
    let t486: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t485[0])), vmovl_u32(vget_high_u32(t485[0])), vmovl_u32(vget_low_u32(t485[1])), vmovl_u32(vget_high_u32(t485[1]))];
    x168 = t486;
    let t487: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x168[0]), vmovn_u64(x168[1])), vcombine_u32(vmovn_u64(x168[2]), vmovn_u64(x168[3]))];
    let t488 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t487, 8u32);
    let t489 = b8_l_p(l, lid0, live, x152);
    let t490: [uint64x2_t; 4] = [vandq_u64(t488[0], t489[0]), vandq_u64(t488[1], t489[1]), vandq_u64(t488[2], t489[2]), vandq_u64(t488[3], t489[3])];
    res = t490;
    return res;
}

#[inline(always)]
unsafe fn b11_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    res = (arg1.wrapping_add(((((x16).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned();
    return res;
}

#[inline(always)]
unsafe fn b11_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    res = (arg1.wrapping_add(((((x16).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned();
    return res;
}

#[inline(always)]
unsafe fn b12_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t491 = b11_r0r1s_p(l, lid0, live, arg0, arg1, (20u32).wrapping_add((4u32).wrapping_mul(x16)));
    res = t491;
    return res;
}

#[inline(always)]
unsafe fn b12_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t492 = b11_r0r1s_f(l, lid0, arg0, arg1, (20u32).wrapping_add((4u32).wrapping_mul(x16)));
    res = t492;
    return res;
}

#[inline(always)]
unsafe fn b13_ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x8: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t493 = [vdupq_n_u32(x8); 2];
    let t494: [uint32x4_t; 2] = [vcltq_u32(x0[0], t493[0]), vcltq_u32(x0[1], t493[1])];
    let mut t495: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t496: [uint32x4_t; 2] = [vandq_u32(live[0], t494[0]), vandq_u32(live[1], t494[1])];
    let t497: [uint32x4_t; 2] = [vbicq_u32(live[0], t494[0]), vbicq_u32(live[1], t494[1])];
    {
        t495 = [vbslq_u32(t496[0], x0[0], t495[0]), vbslq_u32(t496[1], x0[1], t495[1])];
    }
    {
        let t498 = [vdupq_n_u32(x8); 2];
        t495 = [vbslq_u32(t497[0], t498[0], t495[0]), vbslq_u32(t497[1], t498[1], t495[1])];
    }
    res = t495;
    return res;
}

#[inline(always)]
unsafe fn b13_ls_f(l: &Launch, lid0: u32, x0: [uint32x4_t; 2], x8: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t499 = [vdupq_n_u32(x8); 2];
    let t500: [uint32x4_t; 2] = [vcltq_u32(x0[0], t499[0]), vcltq_u32(x0[1], t499[1])];
    let mut t501: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t502: [uint32x4_t; 2] = [vmvnq_u32(t500[0]), vmvnq_u32(t500[1])];
    {
        t501 = [vbslq_u32(t500[0], x0[0], t501[0]), vbslq_u32(t500[1], x0[1], t501[1])];
    }
    {
        let t503 = [vdupq_n_u32(x8); 2];
        t501 = [vbslq_u32(t502[0], t503[0], t501[0]), vbslq_u32(t502[1], t503[1], t501[1])];
    }
    res = t501;
    return res;
}

#[inline(always)]
unsafe fn b14_r0lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: [uint32x4_t; 2], x16: [uint32x4_t; 2], x24: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t504: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x8[0])), vmovl_u32(vget_high_u32(x8[0])), vmovl_u32(vget_low_u32(x8[1])), vmovl_u32(vget_high_u32(x8[1]))];
    let t505: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x16[0])), vmovl_u32(vget_high_u32(x16[0])), vmovl_u32(vget_low_u32(x16[1])), vmovl_u32(vget_high_u32(x16[1]))];
    let t506: [uint64x2_t; 4] = [vaddq_u64(t504[0], t505[0]), vaddq_u64(t504[1], t505[1]), vaddq_u64(t504[2], t505[2]), vaddq_u64(t504[3], t505[3])];
    let t507 = b19_r0ls_p(l, lid0, live, arg0, t506, x24);
    x32 = t507;
    let t508: [uint64x2_t; 4];
    if ((x24) >= (8u32)) {
        t508 = x32;
    } else {
        let t509 = b20_s_p(l, lid0, live, (8u32).wrapping_mul(x24));
        let t510 = [vdupq_n_u64(t509); 4];
        let t511: [uint64x2_t; 4] = [vandq_u64(x32[0], t510[0]), vandq_u64(x32[1], t510[1]), vandq_u64(x32[2], t510[2]), vandq_u64(x32[3], t510[3])];
        t508 = t511;
    }
    res = t508;
    return res;
}

#[inline(always)]
unsafe fn b14_r0lls_f(l: &Launch, lid0: u32, arg0: *const u8, x8: [uint32x4_t; 2], x16: [uint32x4_t; 2], x24: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t512: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x8[0])), vmovl_u32(vget_high_u32(x8[0])), vmovl_u32(vget_low_u32(x8[1])), vmovl_u32(vget_high_u32(x8[1]))];
    let t513: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x16[0])), vmovl_u32(vget_high_u32(x16[0])), vmovl_u32(vget_low_u32(x16[1])), vmovl_u32(vget_high_u32(x16[1]))];
    let t514: [uint64x2_t; 4] = [vaddq_u64(t512[0], t513[0]), vaddq_u64(t512[1], t513[1]), vaddq_u64(t512[2], t513[2]), vaddq_u64(t512[3], t513[3])];
    let t515 = b19_r0ls_f(l, lid0, arg0, t514, x24);
    x32 = t515;
    let t516: [uint64x2_t; 4];
    if ((x24) >= (8u32)) {
        t516 = x32;
    } else {
        let t517 = b20_s_f(l, lid0, (8u32).wrapping_mul(x24));
        let t518 = [vdupq_n_u64(t517); 4];
        let t519: [uint64x2_t; 4] = [vandq_u64(x32[0], t518[0]), vandq_u64(x32[1], t518[1]), vandq_u64(x32[2], t518[2]), vandq_u64(x32[3], t518[3])];
        t516 = t519;
    }
    res = t516;
    return res;
}

#[inline(always)]
unsafe fn b15_r0r1sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2], x32: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t520 = b21_r0sls_p(l, lid0, live, arg0, x16, x24, x32);
    res = t520;
    return res;
}

#[inline(always)]
unsafe fn b16_sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x8: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t521 = [vdupq_n_u32(x0); 2];
    let t522: [uint32x4_t; 2] = [vcltq_u32(t521[0], x8[0]), vcltq_u32(t521[1], x8[1])];
    let mut t523: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t524: [uint32x4_t; 2] = [vandq_u32(live[0], t522[0]), vandq_u32(live[1], t522[1])];
    let t525: [uint32x4_t; 2] = [vbicq_u32(live[0], t522[0]), vbicq_u32(live[1], t522[1])];
    {
        let t526 = [vdupq_n_u32(x0); 2];
        t523 = [vbslq_u32(t524[0], t526[0], t523[0]), vbslq_u32(t524[1], t526[1], t523[1])];
    }
    {
        t523 = [vbslq_u32(t525[0], x8[0], t523[0]), vbslq_u32(t525[1], x8[1], t523[1])];
    }
    res = t523;
    return res;
}

#[inline(always)]
unsafe fn b17_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x8: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t527: [uint32x4_t; 2] = [vcltq_u32(x0[0], x8[0]), vcltq_u32(x0[1], x8[1])];
    let mut t528: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t529: [uint32x4_t; 2] = [vandq_u32(live[0], t527[0]), vandq_u32(live[1], t527[1])];
    let t530: [uint32x4_t; 2] = [vbicq_u32(live[0], t527[0]), vbicq_u32(live[1], t527[1])];
    {
        t528 = [vbslq_u32(t529[0], x0[0], t528[0]), vbslq_u32(t529[1], x0[1], t528[1])];
    }
    {
        t528 = [vbslq_u32(t530[0], x8[0], t528[0]), vbslq_u32(t530[1], x8[1], t528[1])];
    }
    res = t528;
    return res;
}

#[inline(always)]
unsafe fn b18_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint64x2_t; 4]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t531: [uint64x2_t; 4] = [vpaddlq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u64(x0[0]))))), vpaddlq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u64(x0[1]))))), vpaddlq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u64(x0[2]))))), vpaddlq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u64(x0[3])))))];
    let t532: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t531[0]), vmovn_u64(t531[1])), vcombine_u32(vmovn_u64(t531[2]), vmovn_u64(t531[3]))];
    res = t532;
    return res;
}

#[inline(always)]
unsafe fn b19_r0ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: [uint64x2_t; 4], x16: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t533 = [vdupq_n_u64(!(7u64)); 4];
    let t534: [uint64x2_t; 4] = [vandq_u64(x8[0], t533[0]), vandq_u64(x8[1], t533[1]), vandq_u64(x8[2], t533[2]), vandq_u64(x8[3], t533[3])];
    let t535: [uint64x2_t; 4] = [vandq_u64(t534[0], vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0]))))), vandq_u64(t534[1], vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0]))))), vandq_u64(t534[2], vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1]))))), vandq_u64(t534[3], vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))))];
    let t536: [uint64x2_t; 4] = [vaddq_u64(vdupq_n_u64((arg0) as u64), t535[0]), vaddq_u64(vdupq_n_u64((arg0) as u64), t535[1]), vaddq_u64(vdupq_n_u64((arg0) as u64), t535[2]), vaddq_u64(vdupq_n_u64((arg0) as u64), t535[3])];
    x24 = t536;
    let t537 = [vdupq_n_u64(7u64); 4];
    let t538: [uint64x2_t; 4] = [vandq_u64(x8[0], t537[0]), vandq_u64(x8[1], t537[1]), vandq_u64(x8[2], t537[2]), vandq_u64(x8[3], t537[3])];
    let t539: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t538[0]), vmovn_u64(t538[1])), vcombine_u32(vmovn_u64(t538[2]), vmovn_u64(t538[3]))];
    let t540 = [vdupq_n_u32(8u32); 2];
    let t541: [uint32x4_t; 2] = [vmulq_u32(t539[0], t540[0]), vmulq_u32(t539[1], t540[1])];
    x32 = t541;
    let mut t542: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    t542[0] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[0]) as usize as *const u64).read_unaligned(), t542[0]);
    t542[0] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[0]) as usize as *const u64).read_unaligned(), t542[0]);
    t542[1] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[1]) as usize as *const u64).read_unaligned(), t542[1]);
    t542[1] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[1]) as usize as *const u64).read_unaligned(), t542[1]);
    t542[2] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[2]) as usize as *const u64).read_unaligned(), t542[2]);
    t542[2] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[2]) as usize as *const u64).read_unaligned(), t542[2]);
    t542[3] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[3]) as usize as *const u64).read_unaligned(), t542[3]);
    t542[3] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[3]) as usize as *const u64).read_unaligned(), t542[3]);
    let t543: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x32[0])), vmovl_u32(vget_high_u32(x32[0])), vmovl_u32(vget_low_u32(x32[1])), vmovl_u32(vget_high_u32(x32[1]))];
    let t544: [uint64x2_t; 4] = [vshlq_u64(t542[0], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t543[0], vdupq_n_u64(63))))), vshlq_u64(t542[1], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t543[1], vdupq_n_u64(63))))), vshlq_u64(t542[2], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t543[2], vdupq_n_u64(63))))), vshlq_u64(t542[3], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t543[3], vdupq_n_u64(63)))))];
    x40 = t544;
    let t545 = [vdupq_n_u32(0u32); 2];
    let t546: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t545[0])), vmvnq_u32(vceqq_u32(x32[1], t545[1]))];
    let t547 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t548: [uint32x4_t; 2] = [vaddq_u32(x32[0], t547[0]), vaddq_u32(x32[1], t547[1])];
    let t549 = [vdupq_n_u32(64u32); 2];
    let t550: [uint32x4_t; 2] = [vcgtq_u32(t548[0], t549[0]), vcgtq_u32(t548[1], t549[1])];
    let t551: [uint32x4_t; 2] = [vandq_u32(t546[0], t550[0]), vandq_u32(t546[1], t550[1])];
    let mut t552: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t553: [uint32x4_t; 2] = [vandq_u32(live[0], t551[0]), vandq_u32(live[1], t551[1])];
    let t554: [uint32x4_t; 2] = [vbicq_u32(live[0], t551[0]), vbicq_u32(live[1], t551[1])];
    if any(&t553) {
        let t555 = [vdupq_n_u64((1u64).wrapping_mul(8u64)); 4];
        let t556: [uint64x2_t; 4] = [vaddq_u64(x24[0], t555[0]), vaddq_u64(x24[1], t555[1]), vaddq_u64(x24[2], t555[2]), vaddq_u64(x24[3], t555[3])];
        let mut t557: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
        t557[0] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t556[0]) as usize as *const u64).read_unaligned(), t557[0]);
        t557[0] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t556[0]) as usize as *const u64).read_unaligned(), t557[0]);
        t557[1] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t556[1]) as usize as *const u64).read_unaligned(), t557[1]);
        t557[1] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t556[1]) as usize as *const u64).read_unaligned(), t557[1]);
        t557[2] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t556[2]) as usize as *const u64).read_unaligned(), t557[2]);
        t557[2] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t556[2]) as usize as *const u64).read_unaligned(), t557[2]);
        t557[3] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t556[3]) as usize as *const u64).read_unaligned(), t557[3]);
        t557[3] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t556[3]) as usize as *const u64).read_unaligned(), t557[3]);
        let t558: [uint32x4_t; 2] = [vsubq_u32(t549[0], x32[0]), vsubq_u32(t549[1], x32[1])];
        let t559: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t558[0])), vmovl_u32(vget_high_u32(t558[0])), vmovl_u32(vget_low_u32(t558[1])), vmovl_u32(vget_high_u32(t558[1]))];
        let t560: [uint64x2_t; 4] = [vshlq_u64(t557[0], vreinterpretq_s64_u64(vandq_u64(t559[0], vdupq_n_u64(63)))), vshlq_u64(t557[1], vreinterpretq_s64_u64(vandq_u64(t559[1], vdupq_n_u64(63)))), vshlq_u64(t557[2], vreinterpretq_s64_u64(vandq_u64(t559[2], vdupq_n_u64(63)))), vshlq_u64(t557[3], vreinterpretq_s64_u64(vandq_u64(t559[3], vdupq_n_u64(63))))];
        let t561: [uint64x2_t; 4] = [vorrq_u64(x40[0], t560[0]), vorrq_u64(x40[1], t560[1]), vorrq_u64(x40[2], t560[2]), vorrq_u64(x40[3], t560[3])];
        t552 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t553[0])))), t561[0], t552[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t553[0])))), t561[1], t552[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t553[1])))), t561[2], t552[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t553[1])))), t561[3], t552[3])];
    }
    {
        t552 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t554[0])))), x40[0], t552[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t554[0])))), x40[1], t552[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t554[1])))), x40[2], t552[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t554[1])))), x40[3], t552[3])];
    }
    res = t552;
    return res;
}

#[inline(always)]
unsafe fn b19_r0ls_f(l: &Launch, lid0: u32, arg0: *const u8, x8: [uint64x2_t; 4], x16: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t562 = [vdupq_n_u64(!(7u64)); 4];
    let t563: [uint64x2_t; 4] = [vandq_u64(x8[0], t562[0]), vandq_u64(x8[1], t562[1]), vandq_u64(x8[2], t562[2]), vandq_u64(x8[3], t562[3])];
    let t564: [uint64x2_t; 4] = [vaddq_u64(vdupq_n_u64((arg0) as u64), t563[0]), vaddq_u64(vdupq_n_u64((arg0) as u64), t563[1]), vaddq_u64(vdupq_n_u64((arg0) as u64), t563[2]), vaddq_u64(vdupq_n_u64((arg0) as u64), t563[3])];
    x24 = t564;
    let t565 = [vdupq_n_u64(7u64); 4];
    let t566: [uint64x2_t; 4] = [vandq_u64(x8[0], t565[0]), vandq_u64(x8[1], t565[1]), vandq_u64(x8[2], t565[2]), vandq_u64(x8[3], t565[3])];
    let t567: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t566[0]), vmovn_u64(t566[1])), vcombine_u32(vmovn_u64(t566[2]), vmovn_u64(t566[3]))];
    let t568 = [vdupq_n_u32(8u32); 2];
    let t569: [uint32x4_t; 2] = [vmulq_u32(t567[0], t568[0]), vmulq_u32(t567[1], t568[1])];
    x32 = t569;
    let mut t570: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    t570[0] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[0]) as usize as *const u64).read_unaligned(), t570[0]);
    t570[0] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[0]) as usize as *const u64).read_unaligned(), t570[0]);
    t570[1] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[1]) as usize as *const u64).read_unaligned(), t570[1]);
    t570[1] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[1]) as usize as *const u64).read_unaligned(), t570[1]);
    t570[2] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[2]) as usize as *const u64).read_unaligned(), t570[2]);
    t570[2] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[2]) as usize as *const u64).read_unaligned(), t570[2]);
    t570[3] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[3]) as usize as *const u64).read_unaligned(), t570[3]);
    t570[3] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[3]) as usize as *const u64).read_unaligned(), t570[3]);
    let t571: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x32[0])), vmovl_u32(vget_high_u32(x32[0])), vmovl_u32(vget_low_u32(x32[1])), vmovl_u32(vget_high_u32(x32[1]))];
    let t572: [uint64x2_t; 4] = [vshlq_u64(t570[0], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t571[0], vdupq_n_u64(63))))), vshlq_u64(t570[1], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t571[1], vdupq_n_u64(63))))), vshlq_u64(t570[2], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t571[2], vdupq_n_u64(63))))), vshlq_u64(t570[3], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t571[3], vdupq_n_u64(63)))))];
    x40 = t572;
    let t573 = [vdupq_n_u32(0u32); 2];
    let t574: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t573[0])), vmvnq_u32(vceqq_u32(x32[1], t573[1]))];
    let t575 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t576: [uint32x4_t; 2] = [vaddq_u32(x32[0], t575[0]), vaddq_u32(x32[1], t575[1])];
    let t577 = [vdupq_n_u32(64u32); 2];
    let t578: [uint32x4_t; 2] = [vcgtq_u32(t576[0], t577[0]), vcgtq_u32(t576[1], t577[1])];
    let t579: [uint32x4_t; 2] = [vandq_u32(t574[0], t578[0]), vandq_u32(t574[1], t578[1])];
    let mut t580: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t581: [uint32x4_t; 2] = [vmvnq_u32(t579[0]), vmvnq_u32(t579[1])];
    if any(&t579) {
        let t582 = [vdupq_n_u64((1u64).wrapping_mul(8u64)); 4];
        let t583: [uint64x2_t; 4] = [vaddq_u64(x24[0], t582[0]), vaddq_u64(x24[1], t582[1]), vaddq_u64(x24[2], t582[2]), vaddq_u64(x24[3], t582[3])];
        let mut t584: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
        t584[0] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t583[0]) as usize as *const u64).read_unaligned(), t584[0]);
        t584[0] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t583[0]) as usize as *const u64).read_unaligned(), t584[0]);
        t584[1] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t583[1]) as usize as *const u64).read_unaligned(), t584[1]);
        t584[1] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t583[1]) as usize as *const u64).read_unaligned(), t584[1]);
        t584[2] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t583[2]) as usize as *const u64).read_unaligned(), t584[2]);
        t584[2] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t583[2]) as usize as *const u64).read_unaligned(), t584[2]);
        t584[3] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t583[3]) as usize as *const u64).read_unaligned(), t584[3]);
        t584[3] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t583[3]) as usize as *const u64).read_unaligned(), t584[3]);
        let t585: [uint32x4_t; 2] = [vsubq_u32(t577[0], x32[0]), vsubq_u32(t577[1], x32[1])];
        let t586: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t585[0])), vmovl_u32(vget_high_u32(t585[0])), vmovl_u32(vget_low_u32(t585[1])), vmovl_u32(vget_high_u32(t585[1]))];
        let t587: [uint64x2_t; 4] = [vshlq_u64(t584[0], vreinterpretq_s64_u64(vandq_u64(t586[0], vdupq_n_u64(63)))), vshlq_u64(t584[1], vreinterpretq_s64_u64(vandq_u64(t586[1], vdupq_n_u64(63)))), vshlq_u64(t584[2], vreinterpretq_s64_u64(vandq_u64(t586[2], vdupq_n_u64(63)))), vshlq_u64(t584[3], vreinterpretq_s64_u64(vandq_u64(t586[3], vdupq_n_u64(63))))];
        let t588: [uint64x2_t; 4] = [vorrq_u64(x40[0], t587[0]), vorrq_u64(x40[1], t587[1]), vorrq_u64(x40[2], t587[2]), vorrq_u64(x40[3], t587[3])];
        t580 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t579[0])))), t588[0], t580[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t579[0])))), t588[1], t580[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t579[1])))), t588[2], t580[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t579[1])))), t588[3], t580[3])];
    }
    {
        t580 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t581[0])))), x40[0], t580[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t581[0])))), x40[1], t580[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t581[1])))), x40[2], t580[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t581[1])))), x40[3], t580[3])];
    }
    res = t580;
    return res;
}

#[inline(always)]
unsafe fn b20_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t589: u64;
    if ((x0) >= (64u32)) {
        t589 = !(0u64);
    } else {
        t589 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t589;
    return res;
}

#[inline(always)]
unsafe fn b20_s_f(l: &Launch, lid0: u32, x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t590: u64;
    if ((x0) >= (64u32)) {
        t590 = !(0u64);
    } else {
        t590 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t590;
    return res;
}

#[inline(always)]
unsafe fn b21_r0sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: u32, x16: [uint32x4_t; 2], x24: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t591 = [vdupq_n_u64(((x8) as u64)); 4];
    let t592: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x16[0])), vmovl_u32(vget_high_u32(x16[0])), vmovl_u32(vget_low_u32(x16[1])), vmovl_u32(vget_high_u32(x16[1]))];
    let t593: [uint64x2_t; 4] = [vaddq_u64(t591[0], t592[0]), vaddq_u64(t591[1], t592[1]), vaddq_u64(t591[2], t592[2]), vaddq_u64(t591[3], t592[3])];
    let t594 = b19_r0ls_p(l, lid0, live, arg0, t593, x24);
    x32 = t594;
    let t595: [uint64x2_t; 4];
    if ((x24) >= (8u32)) {
        t595 = x32;
    } else {
        let t596 = b20_s_p(l, lid0, live, (8u32).wrapping_mul(x24));
        let t597 = [vdupq_n_u64(t596); 4];
        let t598: [uint64x2_t; 4] = [vandq_u64(x32[0], t597[0]), vandq_u64(x32[1], t597[1]), vandq_u64(x32[2], t597[2]), vandq_u64(x32[3], t597[3])];
        t595 = t598;
    }
    res = t595;
    return res;
}

/// The group `l.group`'s threads, 8 at a time: `argv[j]` the address of argument `j`'s bytes (a buffer's address,
/// a value's bytes).
///
/// # Safety
///
/// `argv` holds the kernel's arguments, its buffers as large as it reads and writes; `l` is a 1-D group's launch.
pub unsafe fn parent_group_w8(argv: *const *const u8, l: &Launch) {
    let n = l.block[0] * l.block[1] * l.block[2];
    let arg0: *const u8 = (*argv.add(0) as *const *const u8).read_unaligned();
    let arg1: *const u8 = (*argv.add(1) as *const *const u8).read_unaligned();
    let arg2: *const u8 = (*argv.add(2) as *const *const u8).read_unaligned();
    let x24: *const u8 = *argv.add(3);
    let arg4: *const u8 = (*argv.add(4) as *const *const u8).read_unaligned();
    let x48: u32 = (*argv.add(5) as *const u32).read_unaligned();
    let mut tg = [0u64; 128];
    let arg6: *const u8 = tg.as_mut_ptr() as *const u8;
    let mut t599: bool = !(arg4).is_null();
    if t599 {
        let mut t600: bool = ((l.group[0]) >= ((arg4.wrapping_add((((x48) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        if !t600 {
            t600 = ((l.group[1]) >= ((arg4.wrapping_add(((((x48).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        let mut t601: bool = t600;
        if !t601 {
            t601 = ((l.group[2]) >= ((arg4.wrapping_add(((((x48).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        t599 = t601;
    }
    if t599 {
        return;
    }
    // Phases: each chunk's, then the next phase's (its barriers).
    let mut state: Vec<u8> = Vec::with_capacity(n.div_ceil(8) as usize * 256);
    let st: *mut u8 = state.as_mut_ptr();
    {
        let phase = 0u32;
        let mut lid0 = 0u32;
        while lid0 + 8 <= n {
            let mut live = [vdupq_n_u32(u32::MAX); 2];
            let gid0 = l.group[0] * l.block[0] + lid0;
            'chunk: {
                b0_r0r1r2sr6gi_f(l, lid0, arg0, arg1, arg2, x24, arg6, lid0, phase, st.add((lid0 / 8) as usize * 256));
            }
            lid0 += 8;
        }
        if lid0 < n {
            // Lanes part: the group's last threads, those under `n`.
            let t602: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
            let mut live = t602;
            let gid0 = l.group[0] * l.block[0] + lid0;
            'chunk: {
                b0_r0r1r2sr6gi_p(l, lid0, live, arg0, arg1, arg2, x24, arg6, lid0, phase, st.add((lid0 / 8) as usize * 256));
            }
        }
    }
    {
        let phase = 1u32;
        let mut lid0 = 0u32;
        while lid0 + 8 <= n {
            let mut live = [vdupq_n_u32(u32::MAX); 2];
            let gid0 = l.group[0] * l.block[0] + lid0;
            'chunk: {
                b0_r0r1r2sr6gi_f(l, lid0, arg0, arg1, arg2, x24, arg6, lid0, phase, st.add((lid0 / 8) as usize * 256));
            }
            lid0 += 8;
        }
        if lid0 < n {
            // Lanes part: the group's last threads, those under `n`.
            let t603: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
            let mut live = t603;
            let gid0 = l.group[0] * l.block[0] + lid0;
            'chunk: {
                b0_r0r1r2sr6gi_p(l, lid0, live, arg0, arg1, arg2, x24, arg6, lid0, phase, st.add((lid0 / 8) as usize * 256));
            }
        }
    }
}
