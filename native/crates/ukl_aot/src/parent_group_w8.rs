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
    let mut x80: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    if phase > 0 {
        live = (st as *const [uint32x4_t; 2]).read_unaligned();
        x64 = (st.add(64) as *const [uint32x4_t; 2]).read_unaligned();
        x72 = (st.add(128) as *const [uint32x4_t; 2]).read_unaligned();
        x80 = (st.add(192) as *const [uint32x4_t; 4]).read_unaligned();
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
                x80 = [vbslq_u32(t7[0], t8[0], x80[0]), vbslq_u32(t7[1], t8[1], x80[1]), vbslq_u32(t7[0], t8[2], x80[2]), vbslq_u32(t7[1], t8[3], x80[3])];
                let t9: u64 = (1u64).wrapping_shl((32u64) as u32);
                let t10 = [vdupq_n_u32((t9) as u32); 2];
                let t11 = [vdupq_n_u32(((t9) >> 32) as u32); 2];
                let t12: [uint32x4_t; 4] = [vandq_u32(x80[0], t10[0]), vandq_u32(x80[1], t10[1]), vandq_u32(x80[2], t11[0]), vandq_u32(x80[3], t11[1])];
                let t13: u64 = 0u64;
                let t14 = [vdupq_n_u32((t13) as u32); 2];
                let t15: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t12[0], t14[0]), vceqq_u32(t12[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t12[1], t14[1]), vceqq_u32(t12[3], vdupq_n_u32(0)))];
                let t16: [uint32x4_t; 2] = [vmvnq_u32(t15[0]), vmvnq_u32(t15[1])];
                let mut t17: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                let t18: [uint32x4_t; 2] = [vandq_u32(t7[0], t16[0]), vandq_u32(t7[1], t16[1])];
                let t19: [uint32x4_t; 2] = [vbicq_u32(t7[0], t16[0]), vbicq_u32(t7[1], t16[1])];
                {
                    let t20: [uint32x4_t; 2] = [x80[0], x80[1]];
                    let t21 = [vdupq_n_u32(2147483648u32); 2];
                    let t22: [uint32x4_t; 2] = [vorrq_u32(t21[0], t20[0]), vorrq_u32(t21[1], t20[1])];
                    t17 = [vbslq_u32(t18[0], t22[0], t17[0]), vbslq_u32(t18[1], t22[1], t17[1])];
                }
                {
                    let t23: [uint32x4_t; 2] = [x80[0], x80[1]];
                    t17 = [vbslq_u32(t19[0], t23[0], t17[0]), vbslq_u32(t19[1], t23[1], t17[1])];
                }
                x72 = [vbslq_u32(t7[0], t17[0], x72[0]), vbslq_u32(t7[1], t17[1], x72[1])];
            }
            let t24 = (arg6 as *mut u32).add((x56) as usize);
            if all(&live) {
                vst1q_u32(t24.add(0), x72[0]);
                vst1q_u32(t24.add(4), x72[1]);
            } else {
                let mut t25w = 0u32;
                let t25 = core::hint::black_box(&raw mut t25w);
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t24.add(0), t25).write(vgetq_lane_u32::<0>(x72[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t24.add(1), t25).write(vgetq_lane_u32::<1>(x72[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t24.add(2), t25).write(vgetq_lane_u32::<2>(x72[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t24.add(3), t25).write(vgetq_lane_u32::<3>(x72[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t24.add(4), t25).write(vgetq_lane_u32::<0>(x72[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t24.add(5), t25).write(vgetq_lane_u32::<1>(x72[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t24.add(6), t25).write(vgetq_lane_u32::<2>(x72[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t24.add(7), t25).write(vgetq_lane_u32::<3>(x72[1]));
            }
        }
    }
    if phase == 1 {
        'phase: {
            let t26: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
            let t27 = [vdupq_n_u32(a0); 2];
            let t28: [uint32x4_t; 2] = [vcgeq_u32(t26[0], t27[0]), vcgeq_u32(t26[1], t27[1])];
            let t29: [uint32x4_t; 2] = [vandq_u32(live[0], t28[0]), vandq_u32(live[1], t28[1])];
            {
                live = [vbicq_u32(live[0], t29[0]), vbicq_u32(live[1], t29[1])];
            }
            if any(&live) {
                let t30: u64 = ((0u32) as u64);
                let t31 = [vdupq_n_u32((t30) as u32); 2];
                let t32: [uint32x4_t; 4] = [t31[0], t31[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                x80 = t32;
                let mut t33 = live;
                loop {
                    // Lanes part: each its own rounds.
                    let t35: [uint32x4_t; 2] = [x80[0], x80[1]];
                    let t36 = [vdupq_n_u32(7u32); 2];
                    let t37: [uint32x4_t; 2] = [vcltq_u32(t35[0], t36[0]), vcltq_u32(t35[1], t36[1])];
                    let t38 = [vdupq_n_u32(4294967295u32); 2];
                    let t39: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x72[0], t38[0])), vmvnq_u32(vceqq_u32(x72[1], t38[1]))];
                    let t40: [uint32x4_t; 2] = [vandq_u32(t37[0], t39[0]), vandq_u32(t37[1], t39[1])];
                    let t41 = [vdupq_n_u32(2147483648u32); 2];
                    let t42: [uint32x4_t; 2] = [vandq_u32(x72[0], t41[0]), vandq_u32(x72[1], t41[1])];
                    let t43 = [vdupq_n_u32(0u32); 2];
                    let t44: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t42[0], t43[0])), vmvnq_u32(vceqq_u32(t42[1], t43[1]))];
                    let t45: [uint32x4_t; 2] = [vandq_u32(t40[0], t44[0]), vandq_u32(t40[1], t44[1])];
                    let t46 = [vdupq_n_u32(2147483647u32); 2];
                    let t47: [uint32x4_t; 2] = [vandq_u32(x72[0], t46[0]), vandq_u32(x72[1], t46[1])];
                    let t48: [uint32x4_t; 2] = [vcgeq_u32(t47[0], x64[0]), vcgeq_u32(t47[1], x64[1])];
                    let t49: [uint32x4_t; 2] = [vandq_u32(t45[0], t48[0]), vandq_u32(t45[1], t48[1])];
                    t33 = [vandq_u32(t33[0], t49[0]), vandq_u32(t33[1], t49[1])];
                    if !(any(&t33)) { break; }
                    let t51: [uint32x4_t; 2] = [vandq_u32(x72[0], t46[0]), vandq_u32(x72[1], t46[1])];
                    let t52: [uint32x4_t; 2] = [vsubq_u32(t51[0], x64[0]), vsubq_u32(t51[1], x64[1])];
                    let t53 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(t33[0], t52[0]), vandq_u32(t33[1], t52[1]))));
                    let t54: [uint32x4_t; 2] = [vbslq_u32(t33[0], t52[0], t53), vbslq_u32(t33[1], t52[1], t53)];
                    let t55 = arg6 as *const u32;
                    let mut t56: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                    t56[0] = vld1q_lane_u32::<0>(t55.add(vgetq_lane_u32::<0>(t54[0]) as usize), t56[0]);
                    t56[0] = vld1q_lane_u32::<1>(t55.add(vgetq_lane_u32::<1>(t54[0]) as usize), t56[0]);
                    t56[0] = vld1q_lane_u32::<2>(t55.add(vgetq_lane_u32::<2>(t54[0]) as usize), t56[0]);
                    t56[0] = vld1q_lane_u32::<3>(t55.add(vgetq_lane_u32::<3>(t54[0]) as usize), t56[0]);
                    t56[1] = vld1q_lane_u32::<0>(t55.add(vgetq_lane_u32::<0>(t54[1]) as usize), t56[1]);
                    t56[1] = vld1q_lane_u32::<1>(t55.add(vgetq_lane_u32::<1>(t54[1]) as usize), t56[1]);
                    t56[1] = vld1q_lane_u32::<2>(t55.add(vgetq_lane_u32::<2>(t54[1]) as usize), t56[1]);
                    t56[1] = vld1q_lane_u32::<3>(t55.add(vgetq_lane_u32::<3>(t54[1]) as usize), t56[1]);
                    x72 = [vbslq_u32(t33[0], t56[0], x72[0]), vbslq_u32(t33[1], t56[1], x72[1])];
                    let t57: [uint32x4_t; 2] = [vandq_u32(t33[0], live[0]), vandq_u32(t33[1], live[1])];
                    let t58: [uint32x4_t; 2] = [x80[0], x80[1]];
                    let t59 = [vdupq_n_u32(1u32); 2];
                    let t60: [uint32x4_t; 2] = [vaddq_u32(t58[0], t59[0]), vaddq_u32(t58[1], t59[1])];
                    let t61 = x80;
                    let t62: [uint32x4_t; 4] = [t60[0], t60[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                    x80 = t62;
                }
                let t63 = [vdupq_n_u32(4294967295u32); 2];
                let t64: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x72[0], t63[0])), vmvnq_u32(vceqq_u32(x72[1], t63[1]))];
                let t65 = [vdupq_n_u32(2147483648u32); 2];
                let t66: [uint32x4_t; 2] = [vandq_u32(x72[0], t65[0]), vandq_u32(x72[1], t65[1])];
                let t67 = [vdupq_n_u32(0u32); 2];
                let t68: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t66[0], t67[0])), vmvnq_u32(vceqq_u32(t66[1], t67[1]))];
                let t69: [uint32x4_t; 2] = [vandq_u32(t64[0], t68[0]), vandq_u32(t64[1], t68[1])];
                let t70: [uint32x4_t; 2] = [vandq_u32(live[0], t69[0]), vandq_u32(live[1], t69[1])];
                if any(&t70) {
                    let t71 = [vdupq_n_u32(2147483647u32); 2];
                    let t72: [uint32x4_t; 2] = [vandq_u32(x72[0], t71[0]), vandq_u32(x72[1], t71[1])];
                    let t73 = b3_r0r1l_p(l, lid0, t70, arg0, arg1, t72);
                    x72 = [vbslq_u32(t70[0], t73[0], x72[0]), vbslq_u32(t70[1], t73[1], x72[1])];
                }
                let t74 = (arg2 as *mut u32).add((gid0) as usize);
                if all(&live) {
                    vst1q_u32(t74.add(0), x72[0]);
                    vst1q_u32(t74.add(4), x72[1]);
                } else {
                    let mut t75w = 0u32;
                    let t75 = core::hint::black_box(&raw mut t75w);
                    core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t74.add(0), t75).write(vgetq_lane_u32::<0>(x72[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t74.add(1), t75).write(vgetq_lane_u32::<1>(x72[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t74.add(2), t75).write(vgetq_lane_u32::<2>(x72[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t74.add(3), t75).write(vgetq_lane_u32::<3>(x72[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t74.add(4), t75).write(vgetq_lane_u32::<0>(x72[1]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t74.add(5), t75).write(vgetq_lane_u32::<1>(x72[1]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t74.add(6), t75).write(vgetq_lane_u32::<2>(x72[1]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t74.add(7), t75).write(vgetq_lane_u32::<3>(x72[1]));
                }
            }
        }
    }
    if phase + 1 < 2 {
        (st as *mut [uint32x4_t; 2]).write_unaligned(live);
        (st.add(64) as *mut [uint32x4_t; 2]).write_unaligned(x64);
        (st.add(128) as *mut [uint32x4_t; 2]).write_unaligned(x72);
        (st.add(192) as *mut [uint32x4_t; 4]).write_unaligned(x80);
    }
}

#[inline(always)]
unsafe fn b0_r0r1r2sr6gi_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, arg2: *const u8, x24: *const u8, arg6: *const u8, x56: u32, phase: u32, st: *mut u8) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x24 as *const u32).read_unaligned();
    let mut x64: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x72: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x80: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    if phase > 0 {
        live = (st as *const [uint32x4_t; 2]).read_unaligned();
        x64 = (st.add(64) as *const [uint32x4_t; 2]).read_unaligned();
        x72 = (st.add(128) as *const [uint32x4_t; 2]).read_unaligned();
        x80 = (st.add(192) as *const [uint32x4_t; 4]).read_unaligned();
        if !any(&live) { return; }
    }
    if phase == 0 {
        'phase: {
            let t76: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
            let t77: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x56), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x56), vld1q_u32(LANE[1].as_ptr()))];
            let t78: [uint32x4_t; 2] = [vsubq_u32(t76[0], t77[0]), vsubq_u32(t76[1], t77[1])];
            x64 = t78;
            let t79 = [vdupq_n_u32(4294967295u32); 2];
            x72 = t79;
            if (((gid0) as u64 + 7) <= u32::MAX as u64 && ((gid0) as u64) >= ((a0) as u64)) {
            } else if ((gid0) as u64 + 7) < ((a0) as u64) {
                let t80 = b1_r0r1g_f(l, lid0, arg0, arg1);
                x80 = t80;
                let t81: u64 = (1u64).wrapping_shl((32u64) as u32);
                let t82 = [vdupq_n_u32((t81) as u32); 2];
                let t83 = [vdupq_n_u32(((t81) >> 32) as u32); 2];
                let t84: [uint32x4_t; 4] = [vandq_u32(x80[0], t82[0]), vandq_u32(x80[1], t82[1]), vandq_u32(x80[2], t83[0]), vandq_u32(x80[3], t83[1])];
                let t85: u64 = 0u64;
                let t86 = [vdupq_n_u32((t85) as u32); 2];
                let t87: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t84[0], t86[0]), vceqq_u32(t84[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t84[1], t86[1]), vceqq_u32(t84[3], vdupq_n_u32(0)))];
                let t88: [uint32x4_t; 2] = [vmvnq_u32(t87[0]), vmvnq_u32(t87[1])];
                let mut t89: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                let t90: [uint32x4_t; 2] = [vmvnq_u32(t88[0]), vmvnq_u32(t88[1])];
                {
                    let t91: [uint32x4_t; 2] = [x80[0], x80[1]];
                    let t92 = [vdupq_n_u32(2147483648u32); 2];
                    let t93: [uint32x4_t; 2] = [vorrq_u32(t92[0], t91[0]), vorrq_u32(t92[1], t91[1])];
                    t89 = [vbslq_u32(t88[0], t93[0], t89[0]), vbslq_u32(t88[1], t93[1], t89[1])];
                }
                {
                    let t94: [uint32x4_t; 2] = [x80[0], x80[1]];
                    t89 = [vbslq_u32(t90[0], t94[0], t89[0]), vbslq_u32(t90[1], t94[1], t89[1])];
                }
                x72 = t89;
            } else {
                // Lanes part: the chunk's lanes on both sides of the bound.
                let t95 = [vdupq_n_u32(a0); 2];
                let t96: [uint32x4_t; 2] = [vcltq_u32(t76[0], t95[0]), vcltq_u32(t76[1], t95[1])];
                let t97: [uint32x4_t; 2] = [vandq_u32(live[0], t96[0]), vandq_u32(live[1], t96[1])];
                if any(&t97) {
                    let t98 = b1_r0r1g_p(l, lid0, t97, arg0, arg1);
                    x80 = [vbslq_u32(t97[0], t98[0], x80[0]), vbslq_u32(t97[1], t98[1], x80[1]), vbslq_u32(t97[0], t98[2], x80[2]), vbslq_u32(t97[1], t98[3], x80[3])];
                    let t99: u64 = (1u64).wrapping_shl((32u64) as u32);
                    let t100 = [vdupq_n_u32((t99) as u32); 2];
                    let t101 = [vdupq_n_u32(((t99) >> 32) as u32); 2];
                    let t102: [uint32x4_t; 4] = [vandq_u32(x80[0], t100[0]), vandq_u32(x80[1], t100[1]), vandq_u32(x80[2], t101[0]), vandq_u32(x80[3], t101[1])];
                    let t103: u64 = 0u64;
                    let t104 = [vdupq_n_u32((t103) as u32); 2];
                    let t105: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t102[0], t104[0]), vceqq_u32(t102[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t102[1], t104[1]), vceqq_u32(t102[3], vdupq_n_u32(0)))];
                    let t106: [uint32x4_t; 2] = [vmvnq_u32(t105[0]), vmvnq_u32(t105[1])];
                    let mut t107: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                    let t108: [uint32x4_t; 2] = [vandq_u32(t97[0], t106[0]), vandq_u32(t97[1], t106[1])];
                    let t109: [uint32x4_t; 2] = [vbicq_u32(t97[0], t106[0]), vbicq_u32(t97[1], t106[1])];
                    {
                        let t110: [uint32x4_t; 2] = [x80[0], x80[1]];
                        let t111 = [vdupq_n_u32(2147483648u32); 2];
                        let t112: [uint32x4_t; 2] = [vorrq_u32(t111[0], t110[0]), vorrq_u32(t111[1], t110[1])];
                        t107 = [vbslq_u32(t108[0], t112[0], t107[0]), vbslq_u32(t108[1], t112[1], t107[1])];
                    }
                    {
                        let t113: [uint32x4_t; 2] = [x80[0], x80[1]];
                        t107 = [vbslq_u32(t109[0], t113[0], t107[0]), vbslq_u32(t109[1], t113[1], t107[1])];
                    }
                    x72 = [vbslq_u32(t97[0], t107[0], x72[0]), vbslq_u32(t97[1], t107[1], x72[1])];
                }
            }
            let t114 = (arg6 as *mut u32).add((x56) as usize);
            vst1q_u32(t114.add(0), x72[0]);
            vst1q_u32(t114.add(4), x72[1]);
        }
    }
    if phase == 1 {
        'phase: {
            let t115: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
            let t116 = [vdupq_n_u32(a0); 2];
            let t117: [uint32x4_t; 2] = [vcgeq_u32(t115[0], t116[0]), vcgeq_u32(t115[1], t116[1])];
            let t118: [uint32x4_t; 2] = [vandq_u32(live[0], t117[0]), vandq_u32(live[1], t117[1])];
            {
                live = [vbicq_u32(live[0], t118[0]), vbicq_u32(live[1], t118[1])];
            }
            if any(&live) {
                let t119: u64 = ((0u32) as u64);
                let t120 = [vdupq_n_u32((t119) as u32); 2];
                let t121: [uint32x4_t; 4] = [t120[0], t120[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                x80 = t121;
                let mut t122 = live;
                loop {
                    // Lanes part: each its own rounds.
                    let t124: [uint32x4_t; 2] = [x80[0], x80[1]];
                    let t125 = [vdupq_n_u32(7u32); 2];
                    let t126: [uint32x4_t; 2] = [vcltq_u32(t124[0], t125[0]), vcltq_u32(t124[1], t125[1])];
                    let t127 = [vdupq_n_u32(4294967295u32); 2];
                    let t128: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x72[0], t127[0])), vmvnq_u32(vceqq_u32(x72[1], t127[1]))];
                    let t129: [uint32x4_t; 2] = [vandq_u32(t126[0], t128[0]), vandq_u32(t126[1], t128[1])];
                    let t130 = [vdupq_n_u32(2147483648u32); 2];
                    let t131: [uint32x4_t; 2] = [vandq_u32(x72[0], t130[0]), vandq_u32(x72[1], t130[1])];
                    let t132 = [vdupq_n_u32(0u32); 2];
                    let t133: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t131[0], t132[0])), vmvnq_u32(vceqq_u32(t131[1], t132[1]))];
                    let t134: [uint32x4_t; 2] = [vandq_u32(t129[0], t133[0]), vandq_u32(t129[1], t133[1])];
                    let t135 = [vdupq_n_u32(2147483647u32); 2];
                    let t136: [uint32x4_t; 2] = [vandq_u32(x72[0], t135[0]), vandq_u32(x72[1], t135[1])];
                    let t137: [uint32x4_t; 2] = [vcgeq_u32(t136[0], x64[0]), vcgeq_u32(t136[1], x64[1])];
                    let t138: [uint32x4_t; 2] = [vandq_u32(t134[0], t137[0]), vandq_u32(t134[1], t137[1])];
                    t122 = [vandq_u32(t122[0], t138[0]), vandq_u32(t122[1], t138[1])];
                    if !(any(&t122)) { break; }
                    let t140: [uint32x4_t; 2] = [vandq_u32(x72[0], t135[0]), vandq_u32(x72[1], t135[1])];
                    let t141: [uint32x4_t; 2] = [vsubq_u32(t140[0], x64[0]), vsubq_u32(t140[1], x64[1])];
                    let t142 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(t122[0], t141[0]), vandq_u32(t122[1], t141[1]))));
                    let t143: [uint32x4_t; 2] = [vbslq_u32(t122[0], t141[0], t142), vbslq_u32(t122[1], t141[1], t142)];
                    let t144 = arg6 as *const u32;
                    let mut t145: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                    t145[0] = vld1q_lane_u32::<0>(t144.add(vgetq_lane_u32::<0>(t143[0]) as usize), t145[0]);
                    t145[0] = vld1q_lane_u32::<1>(t144.add(vgetq_lane_u32::<1>(t143[0]) as usize), t145[0]);
                    t145[0] = vld1q_lane_u32::<2>(t144.add(vgetq_lane_u32::<2>(t143[0]) as usize), t145[0]);
                    t145[0] = vld1q_lane_u32::<3>(t144.add(vgetq_lane_u32::<3>(t143[0]) as usize), t145[0]);
                    t145[1] = vld1q_lane_u32::<0>(t144.add(vgetq_lane_u32::<0>(t143[1]) as usize), t145[1]);
                    t145[1] = vld1q_lane_u32::<1>(t144.add(vgetq_lane_u32::<1>(t143[1]) as usize), t145[1]);
                    t145[1] = vld1q_lane_u32::<2>(t144.add(vgetq_lane_u32::<2>(t143[1]) as usize), t145[1]);
                    t145[1] = vld1q_lane_u32::<3>(t144.add(vgetq_lane_u32::<3>(t143[1]) as usize), t145[1]);
                    x72 = [vbslq_u32(t122[0], t145[0], x72[0]), vbslq_u32(t122[1], t145[1], x72[1])];
                    let t146: [uint32x4_t; 2] = [vandq_u32(t122[0], live[0]), vandq_u32(t122[1], live[1])];
                    let t147: [uint32x4_t; 2] = [x80[0], x80[1]];
                    let t148 = [vdupq_n_u32(1u32); 2];
                    let t149: [uint32x4_t; 2] = [vaddq_u32(t147[0], t148[0]), vaddq_u32(t147[1], t148[1])];
                    let t150 = x80;
                    let t151: [uint32x4_t; 4] = [t149[0], t149[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                    x80 = t151;
                }
                let t152 = [vdupq_n_u32(4294967295u32); 2];
                let t153: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x72[0], t152[0])), vmvnq_u32(vceqq_u32(x72[1], t152[1]))];
                let t154 = [vdupq_n_u32(2147483648u32); 2];
                let t155: [uint32x4_t; 2] = [vandq_u32(x72[0], t154[0]), vandq_u32(x72[1], t154[1])];
                let t156 = [vdupq_n_u32(0u32); 2];
                let t157: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t155[0], t156[0])), vmvnq_u32(vceqq_u32(t155[1], t156[1]))];
                let t158: [uint32x4_t; 2] = [vandq_u32(t153[0], t157[0]), vandq_u32(t153[1], t157[1])];
                let t159: [uint32x4_t; 2] = [vandq_u32(live[0], t158[0]), vandq_u32(live[1], t158[1])];
                if any(&t159) {
                    let t160 = [vdupq_n_u32(2147483647u32); 2];
                    let t161: [uint32x4_t; 2] = [vandq_u32(x72[0], t160[0]), vandq_u32(x72[1], t160[1])];
                    let t162 = b3_r0r1l_p(l, lid0, t159, arg0, arg1, t161);
                    x72 = [vbslq_u32(t159[0], t162[0], x72[0]), vbslq_u32(t159[1], t162[1], x72[1])];
                }
                let t163 = (arg2 as *mut u32).add((gid0) as usize);
                if all(&live) {
                    vst1q_u32(t163.add(0), x72[0]);
                    vst1q_u32(t163.add(4), x72[1]);
                } else {
                    let mut t164w = 0u32;
                    let t164 = core::hint::black_box(&raw mut t164w);
                    core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t163.add(0), t164).write(vgetq_lane_u32::<0>(x72[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t163.add(1), t164).write(vgetq_lane_u32::<1>(x72[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t163.add(2), t164).write(vgetq_lane_u32::<2>(x72[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t163.add(3), t164).write(vgetq_lane_u32::<3>(x72[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t163.add(4), t164).write(vgetq_lane_u32::<0>(x72[1]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t163.add(5), t164).write(vgetq_lane_u32::<1>(x72[1]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t163.add(6), t164).write(vgetq_lane_u32::<2>(x72[1]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t163.add(7), t164).write(vgetq_lane_u32::<3>(x72[1]));
                }
            }
        }
    }
    if phase + 1 < 2 {
        (st as *mut [uint32x4_t; 2]).write_unaligned(live);
        (st.add(64) as *mut [uint32x4_t; 2]).write_unaligned(x64);
        (st.add(128) as *mut [uint32x4_t; 2]).write_unaligned(x72);
        (st.add(192) as *mut [uint32x4_t; 4]).write_unaligned(x80);
    }
}

#[inline(always)]
unsafe fn b1_r0r1g_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: u32 = 0;
    let mut x32: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t165: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
    let t166 = [vdupq_n_u32(0u32); 2];
    let t167: [uint32x4_t; 2] = [vceqq_u32(t165[0], t166[0]), vceqq_u32(t165[1], t166[1])];
    let t168: [uint32x4_t; 2] = [vandq_u32(live[0], t167[0]), vandq_u32(live[1], t167[1])];
    {
        let t169: u64 = 4294967295u64;
        let t170 = [vdupq_n_u32((t169) as u32); 2];
        let t171: [uint32x4_t; 4] = [t170[0], t170[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = [vbslq_u32(t168[0], t171[0], res[0]), vbslq_u32(t168[1], t171[1], res[1]), vbslq_u32(t168[0], t171[2], res[2]), vbslq_u32(t168[1], t171[3], res[3])];
        live = [vbicq_u32(live[0], t168[0]), vbicq_u32(live[1], t168[1])];
    }
    if any(&live) {
        x24 = gid0.wrapping_sub(1u32);
        let t172 = if (x24 & 255u32) <= 248u32 { b4_r0r1u8_p(l, lid0, live, arg0, arg1, x24) } else { b4_r0r1i_p(l, lid0, live, arg0, arg1, x24) };
        x32 = t172;
        let t173: u64 = 1u64;
        let t174 = [vdupq_n_u32((t173) as u32); 2];
        let t175: [uint32x4_t; 2] = [vandq_u32(x32[0], t174[0]), vandq_u32(x32[1], t174[1])];
        let t176: u64 = 0u64;
        let t177 = [vdupq_n_u32((t176) as u32); 2];
        let t178: [uint32x4_t; 2] = [vceqq_u32(t175[0], t177[0]), vceqq_u32(t175[1], t177[1])];
        let t179: [uint32x4_t; 2] = [vandq_u32(live[0], t178[0]), vandq_u32(live[1], t178[1])];
        {
            let t180: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[1].as_ptr()))];
            let t181: [uint32x4_t; 4] = [t180[0], t180[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t179[0], t181[0], res[0]), vbslq_u32(t179[1], t181[1], res[1]), vbslq_u32(t179[0], t181[2], res[2]), vbslq_u32(t179[1], t181[3], res[3])];
            live = [vbicq_u32(live[0], t179[0]), vbicq_u32(live[1], t179[1])];
        }
        if any(&live) {
            let t182: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x32[0]), vshlq_n_u32::<30>(x32[2])), vorrq_u32(vshrq_n_u32::<2>(x32[1]), vshlq_n_u32::<30>(x32[3])), vshrq_n_u32::<2>(x32[2]), vshrq_n_u32::<2>(x32[3])];
            let t183: [uint32x4_t; 2] = [t182[0], t182[1]];
            x40 = t183;
            let t184 = [vdupq_n_u32(1u32); 2];
            let t185: [uint32x4_t; 2] = [vsubq_u32(x40[0], t184[0]), vsubq_u32(x40[1], t184[1])];
            let t186: [uint32x4_t; 2] = [vcgeq_u32(t185[0], t165[0]), vcgeq_u32(t185[1], t165[1])];
            let t187: [uint32x4_t; 2] = [vandq_u32(live[0], t186[0]), vandq_u32(live[1], t186[1])];
            {
                let t188: u64 = 4294967295u64;
                let t189 = [vdupq_n_u32((t188) as u32); 2];
                let t190: [uint32x4_t; 4] = [t189[0], t189[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t187[0], t190[0], res[0]), vbslq_u32(t187[1], t190[1], res[1]), vbslq_u32(t187[0], t190[2], res[2]), vbslq_u32(t187[1], t190[3], res[3])];
                live = [vbicq_u32(live[0], t187[0]), vbicq_u32(live[1], t187[1])];
            }
            if any(&live) {
                let t191: [uint32x4_t; 2] = [vsubq_u32(t165[0], x40[0]), vsubq_u32(t165[1], x40[1])];
                x48 = t191;
                let t192: u64 = 2u64;
                let t193 = [vdupq_n_u32((t192) as u32); 2];
                let t194: [uint32x4_t; 2] = [vandq_u32(x32[0], t193[0]), vandq_u32(x32[1], t193[1])];
                let t195: u64 = 0u64;
                let t196 = [vdupq_n_u32((t195) as u32); 2];
                let t197: [uint32x4_t; 2] = [vceqq_u32(t194[0], t196[0]), vceqq_u32(t194[1], t196[1])];
                let t198: [uint32x4_t; 2] = [vmvnq_u32(t197[0]), vmvnq_u32(t197[1])];
                let t199: [uint32x4_t; 2] = [vandq_u32(live[0], t198[0]), vandq_u32(live[1], t198[1])];
                {
                    let t200: [uint32x4_t; 4] = [x48[0], x48[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                    res = [vbslq_u32(t199[0], t200[0], res[0]), vbslq_u32(t199[1], t200[1], res[1]), vbslq_u32(t199[0], t200[2], res[2]), vbslq_u32(t199[1], t200[3], res[3])];
                    live = [vbicq_u32(live[0], t199[0]), vbicq_u32(live[1], t199[1])];
                }
                if any(&live) {
                    let t201: [uint32x4_t; 2] = [vceqq_u32(x48[0], t166[0]), vceqq_u32(x48[1], t166[1])];
                    let t202: [uint32x4_t; 2] = [vandq_u32(live[0], t201[0]), vandq_u32(live[1], t201[1])];
                    {
                        let t203: u64 = 4294967295u64;
                        let t204 = [vdupq_n_u32((t203) as u32); 2];
                        let t205: [uint32x4_t; 4] = [t204[0], t204[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                        res = [vbslq_u32(t202[0], t205[0], res[0]), vbslq_u32(t202[1], t205[1], res[1]), vbslq_u32(t202[0], t205[2], res[2]), vbslq_u32(t202[1], t205[3], res[3])];
                        live = [vbicq_u32(live[0], t202[0]), vbicq_u32(live[1], t202[1])];
                    }
                    if any(&live) {
                        let t206: u64 = (1u64).wrapping_shl((32u64) as u32);
                        let t207 = [vdupq_n_u32((t206) as u32); 2];
                        let t208 = [vdupq_n_u32(((t206) >> 32) as u32); 2];
                        let t209: [uint32x4_t; 4] = [vorrq_u32(t207[0], x48[0]), vorrq_u32(t207[1], x48[1]), t208[0], t208[1]];
                        res = [vbslq_u32(live[0], t209[0], res[0]), vbslq_u32(live[1], t209[1], res[1]), vbslq_u32(live[0], t209[2], res[2]), vbslq_u32(live[1], t209[3], res[3])];
                        live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
                    }
                }
            }
        }
    }
    res
}

#[inline(always)]
unsafe fn b1_r0r1g_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: u32 = 0;
    let mut x32: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if (((gid0) as u64 + 7) <= u32::MAX as u64 && (((0u32) as u64) < ((gid0) as u64) || ((0u32) as u64) > ((gid0) as u64 + 7))) {
        x24 = gid0.wrapping_sub(1u32);
        let t210 = if (x24 & 255u32) <= 248u32 { b4_r0r1u8_f(l, lid0, arg0, arg1, x24) } else { b4_r0r1i_f(l, lid0, arg0, arg1, x24) };
        x32 = t210;
        let t211: u64 = 1u64;
        let t212 = [vdupq_n_u32((t211) as u32); 2];
        let t213: [uint32x4_t; 2] = [vandq_u32(x32[0], t212[0]), vandq_u32(x32[1], t212[1])];
        let t214: u64 = 0u64;
        let t215 = [vdupq_n_u32((t214) as u32); 2];
        let t216: [uint32x4_t; 2] = [vceqq_u32(t213[0], t215[0]), vceqq_u32(t213[1], t215[1])];
        {
            let t217: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[1].as_ptr()))];
            let t218: [uint32x4_t; 4] = [t217[0], t217[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t216[0], t218[0], res[0]), vbslq_u32(t216[1], t218[1], res[1]), vbslq_u32(t216[0], t218[2], res[2]), vbslq_u32(t216[1], t218[3], res[3])];
            live = [vbicq_u32(live[0], t216[0]), vbicq_u32(live[1], t216[1])];
        }
        if any(&live) {
            let t219: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x32[0]), vshlq_n_u32::<30>(x32[2])), vorrq_u32(vshrq_n_u32::<2>(x32[1]), vshlq_n_u32::<30>(x32[3])), vshrq_n_u32::<2>(x32[2]), vshrq_n_u32::<2>(x32[3])];
            let t220: [uint32x4_t; 2] = [t219[0], t219[1]];
            x40 = t220;
            let t221 = [vdupq_n_u32(1u32); 2];
            let t222: [uint32x4_t; 2] = [vsubq_u32(x40[0], t221[0]), vsubq_u32(x40[1], t221[1])];
            let t223: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
            let t224: [uint32x4_t; 2] = [vcgeq_u32(t222[0], t223[0]), vcgeq_u32(t222[1], t223[1])];
            let t225: [uint32x4_t; 2] = [vandq_u32(live[0], t224[0]), vandq_u32(live[1], t224[1])];
            {
                let t226: u64 = 4294967295u64;
                let t227 = [vdupq_n_u32((t226) as u32); 2];
                let t228: [uint32x4_t; 4] = [t227[0], t227[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t225[0], t228[0], res[0]), vbslq_u32(t225[1], t228[1], res[1]), vbslq_u32(t225[0], t228[2], res[2]), vbslq_u32(t225[1], t228[3], res[3])];
                live = [vbicq_u32(live[0], t225[0]), vbicq_u32(live[1], t225[1])];
            }
            if any(&live) {
                let t229: [uint32x4_t; 2] = [vsubq_u32(t223[0], x40[0]), vsubq_u32(t223[1], x40[1])];
                x48 = t229;
                let t230: u64 = 2u64;
                let t231 = [vdupq_n_u32((t230) as u32); 2];
                let t232: [uint32x4_t; 2] = [vandq_u32(x32[0], t231[0]), vandq_u32(x32[1], t231[1])];
                let t233: u64 = 0u64;
                let t234 = [vdupq_n_u32((t233) as u32); 2];
                let t235: [uint32x4_t; 2] = [vceqq_u32(t232[0], t234[0]), vceqq_u32(t232[1], t234[1])];
                let t236: [uint32x4_t; 2] = [vmvnq_u32(t235[0]), vmvnq_u32(t235[1])];
                let t237: [uint32x4_t; 2] = [vandq_u32(live[0], t236[0]), vandq_u32(live[1], t236[1])];
                {
                    let t238: [uint32x4_t; 4] = [x48[0], x48[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                    res = [vbslq_u32(t237[0], t238[0], res[0]), vbslq_u32(t237[1], t238[1], res[1]), vbslq_u32(t237[0], t238[2], res[2]), vbslq_u32(t237[1], t238[3], res[3])];
                    live = [vbicq_u32(live[0], t237[0]), vbicq_u32(live[1], t237[1])];
                }
                if any(&live) {
                    let t239 = [vdupq_n_u32(0u32); 2];
                    let t240: [uint32x4_t; 2] = [vceqq_u32(x48[0], t239[0]), vceqq_u32(x48[1], t239[1])];
                    let t241: [uint32x4_t; 2] = [vandq_u32(live[0], t240[0]), vandq_u32(live[1], t240[1])];
                    {
                        let t242: u64 = 4294967295u64;
                        let t243 = [vdupq_n_u32((t242) as u32); 2];
                        let t244: [uint32x4_t; 4] = [t243[0], t243[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                        res = [vbslq_u32(t241[0], t244[0], res[0]), vbslq_u32(t241[1], t244[1], res[1]), vbslq_u32(t241[0], t244[2], res[2]), vbslq_u32(t241[1], t244[3], res[3])];
                        live = [vbicq_u32(live[0], t241[0]), vbicq_u32(live[1], t241[1])];
                    }
                    if any(&live) {
                        let t245: u64 = (1u64).wrapping_shl((32u64) as u32);
                        let t246 = [vdupq_n_u32((t245) as u32); 2];
                        let t247 = [vdupq_n_u32(((t245) >> 32) as u32); 2];
                        let t248: [uint32x4_t; 4] = [vorrq_u32(t246[0], x48[0]), vorrq_u32(t246[1], x48[1]), t247[0], t247[1]];
                        res = [vbslq_u32(live[0], t248[0], res[0]), vbslq_u32(live[1], t248[1], res[1]), vbslq_u32(live[0], t248[2], res[2]), vbslq_u32(live[1], t248[3], res[3])];
                        live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
                    }
                }
            }
        }
    } else if false {
        let t249: u64 = 4294967295u64;
        let t250 = [vdupq_n_u32((t249) as u32); 2];
        let t251: [uint32x4_t; 4] = [t250[0], t250[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t251;
        return res;
    } else {
        // Lanes part: the chunk's lanes on both sides of the bound.
        let t252: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t253 = [vdupq_n_u32(0u32); 2];
        let t254: [uint32x4_t; 2] = [vceqq_u32(t252[0], t253[0]), vceqq_u32(t252[1], t253[1])];
        let t255: [uint32x4_t; 2] = [vandq_u32(live[0], t254[0]), vandq_u32(live[1], t254[1])];
        {
            let t256: u64 = 4294967295u64;
            let t257 = [vdupq_n_u32((t256) as u32); 2];
            let t258: [uint32x4_t; 4] = [t257[0], t257[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t255[0], t258[0], res[0]), vbslq_u32(t255[1], t258[1], res[1]), vbslq_u32(t255[0], t258[2], res[2]), vbslq_u32(t255[1], t258[3], res[3])];
            live = [vbicq_u32(live[0], t255[0]), vbicq_u32(live[1], t255[1])];
        }
        if any(&live) {
            x24 = gid0.wrapping_sub(1u32);
            let t259 = if (x24 & 255u32) <= 248u32 { b4_r0r1u8_p(l, lid0, live, arg0, arg1, x24) } else { b4_r0r1i_p(l, lid0, live, arg0, arg1, x24) };
            x32 = t259;
            let t260: u64 = 1u64;
            let t261 = [vdupq_n_u32((t260) as u32); 2];
            let t262: [uint32x4_t; 2] = [vandq_u32(x32[0], t261[0]), vandq_u32(x32[1], t261[1])];
            let t263: u64 = 0u64;
            let t264 = [vdupq_n_u32((t263) as u32); 2];
            let t265: [uint32x4_t; 2] = [vceqq_u32(t262[0], t264[0]), vceqq_u32(t262[1], t264[1])];
            let t266: [uint32x4_t; 2] = [vandq_u32(live[0], t265[0]), vandq_u32(live[1], t265[1])];
            {
                let t267: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[1].as_ptr()))];
                let t268: [uint32x4_t; 4] = [t267[0], t267[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t266[0], t268[0], res[0]), vbslq_u32(t266[1], t268[1], res[1]), vbslq_u32(t266[0], t268[2], res[2]), vbslq_u32(t266[1], t268[3], res[3])];
                live = [vbicq_u32(live[0], t266[0]), vbicq_u32(live[1], t266[1])];
            }
            if any(&live) {
                let t269: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x32[0]), vshlq_n_u32::<30>(x32[2])), vorrq_u32(vshrq_n_u32::<2>(x32[1]), vshlq_n_u32::<30>(x32[3])), vshrq_n_u32::<2>(x32[2]), vshrq_n_u32::<2>(x32[3])];
                let t270: [uint32x4_t; 2] = [t269[0], t269[1]];
                x40 = t270;
                let t271 = [vdupq_n_u32(1u32); 2];
                let t272: [uint32x4_t; 2] = [vsubq_u32(x40[0], t271[0]), vsubq_u32(x40[1], t271[1])];
                let t273: [uint32x4_t; 2] = [vcgeq_u32(t272[0], t252[0]), vcgeq_u32(t272[1], t252[1])];
                let t274: [uint32x4_t; 2] = [vandq_u32(live[0], t273[0]), vandq_u32(live[1], t273[1])];
                {
                    let t275: u64 = 4294967295u64;
                    let t276 = [vdupq_n_u32((t275) as u32); 2];
                    let t277: [uint32x4_t; 4] = [t276[0], t276[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                    res = [vbslq_u32(t274[0], t277[0], res[0]), vbslq_u32(t274[1], t277[1], res[1]), vbslq_u32(t274[0], t277[2], res[2]), vbslq_u32(t274[1], t277[3], res[3])];
                    live = [vbicq_u32(live[0], t274[0]), vbicq_u32(live[1], t274[1])];
                }
                if any(&live) {
                    let t278: [uint32x4_t; 2] = [vsubq_u32(t252[0], x40[0]), vsubq_u32(t252[1], x40[1])];
                    x48 = t278;
                    let t279: u64 = 2u64;
                    let t280 = [vdupq_n_u32((t279) as u32); 2];
                    let t281: [uint32x4_t; 2] = [vandq_u32(x32[0], t280[0]), vandq_u32(x32[1], t280[1])];
                    let t282: u64 = 0u64;
                    let t283 = [vdupq_n_u32((t282) as u32); 2];
                    let t284: [uint32x4_t; 2] = [vceqq_u32(t281[0], t283[0]), vceqq_u32(t281[1], t283[1])];
                    let t285: [uint32x4_t; 2] = [vmvnq_u32(t284[0]), vmvnq_u32(t284[1])];
                    let t286: [uint32x4_t; 2] = [vandq_u32(live[0], t285[0]), vandq_u32(live[1], t285[1])];
                    {
                        let t287: [uint32x4_t; 4] = [x48[0], x48[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                        res = [vbslq_u32(t286[0], t287[0], res[0]), vbslq_u32(t286[1], t287[1], res[1]), vbslq_u32(t286[0], t287[2], res[2]), vbslq_u32(t286[1], t287[3], res[3])];
                        live = [vbicq_u32(live[0], t286[0]), vbicq_u32(live[1], t286[1])];
                    }
                    if any(&live) {
                        let t288: [uint32x4_t; 2] = [vceqq_u32(x48[0], t253[0]), vceqq_u32(x48[1], t253[1])];
                        let t289: [uint32x4_t; 2] = [vandq_u32(live[0], t288[0]), vandq_u32(live[1], t288[1])];
                        {
                            let t290: u64 = 4294967295u64;
                            let t291 = [vdupq_n_u32((t290) as u32); 2];
                            let t292: [uint32x4_t; 4] = [t291[0], t291[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                            res = [vbslq_u32(t289[0], t292[0], res[0]), vbslq_u32(t289[1], t292[1], res[1]), vbslq_u32(t289[0], t292[2], res[2]), vbslq_u32(t289[1], t292[3], res[3])];
                            live = [vbicq_u32(live[0], t289[0]), vbicq_u32(live[1], t289[1])];
                        }
                        if any(&live) {
                            let t293: u64 = (1u64).wrapping_shl((32u64) as u32);
                            let t294 = [vdupq_n_u32((t293) as u32); 2];
                            let t295 = [vdupq_n_u32(((t293) >> 32) as u32); 2];
                            let t296: [uint32x4_t; 4] = [vorrq_u32(t294[0], x48[0]), vorrq_u32(t294[1], x48[1]), t295[0], t295[1]];
                            res = [vbslq_u32(live[0], t296[0], res[0]), vbslq_u32(live[1], t296[1], res[1]), vbslq_u32(live[0], t296[2], res[2]), vbslq_u32(live[1], t296[3], res[3])];
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
unsafe fn b1_r0r1l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x32: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t297 = [vdupq_n_u32(0u32); 2];
    let t298: [uint32x4_t; 2] = [vceqq_u32(x16[0], t297[0]), vceqq_u32(x16[1], t297[1])];
    let t299: [uint32x4_t; 2] = [vandq_u32(live[0], t298[0]), vandq_u32(live[1], t298[1])];
    {
        let t300: u64 = 4294967295u64;
        let t301 = [vdupq_n_u32((t300) as u32); 2];
        let t302: [uint32x4_t; 4] = [t301[0], t301[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = [vbslq_u32(t299[0], t302[0], res[0]), vbslq_u32(t299[1], t302[1], res[1]), vbslq_u32(t299[0], t302[2], res[2]), vbslq_u32(t299[1], t302[3], res[3])];
        live = [vbicq_u32(live[0], t299[0]), vbicq_u32(live[1], t299[1])];
    }
    if any(&live) {
        let t303 = [vdupq_n_u32(1u32); 2];
        let t304: [uint32x4_t; 2] = [vsubq_u32(x16[0], t303[0]), vsubq_u32(x16[1], t303[1])];
        x24 = t304;
        let t305 = b4_r0r1l_p(l, lid0, live, arg0, arg1, x24);
        x32 = t305;
        let t306: u64 = 1u64;
        let t307 = [vdupq_n_u32((t306) as u32); 2];
        let t308: [uint32x4_t; 2] = [vandq_u32(x32[0], t307[0]), vandq_u32(x32[1], t307[1])];
        let t309: u64 = 0u64;
        let t310 = [vdupq_n_u32((t309) as u32); 2];
        let t311: [uint32x4_t; 2] = [vceqq_u32(t308[0], t310[0]), vceqq_u32(t308[1], t310[1])];
        let t312: [uint32x4_t; 2] = [vandq_u32(live[0], t311[0]), vandq_u32(live[1], t311[1])];
        {
            let t313: [uint32x4_t; 4] = [x24[0], x24[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t312[0], t313[0], res[0]), vbslq_u32(t312[1], t313[1], res[1]), vbslq_u32(t312[0], t313[2], res[2]), vbslq_u32(t312[1], t313[3], res[3])];
            live = [vbicq_u32(live[0], t312[0]), vbicq_u32(live[1], t312[1])];
        }
        if any(&live) {
            let t314: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x32[0]), vshlq_n_u32::<30>(x32[2])), vorrq_u32(vshrq_n_u32::<2>(x32[1]), vshlq_n_u32::<30>(x32[3])), vshrq_n_u32::<2>(x32[2]), vshrq_n_u32::<2>(x32[3])];
            let t315: [uint32x4_t; 2] = [t314[0], t314[1]];
            x40 = t315;
            let t316: [uint32x4_t; 2] = [vsubq_u32(x40[0], t303[0]), vsubq_u32(x40[1], t303[1])];
            let t317: [uint32x4_t; 2] = [vcgeq_u32(t316[0], x16[0]), vcgeq_u32(t316[1], x16[1])];
            let t318: [uint32x4_t; 2] = [vandq_u32(live[0], t317[0]), vandq_u32(live[1], t317[1])];
            {
                let t319: u64 = 4294967295u64;
                let t320 = [vdupq_n_u32((t319) as u32); 2];
                let t321: [uint32x4_t; 4] = [t320[0], t320[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t318[0], t321[0], res[0]), vbslq_u32(t318[1], t321[1], res[1]), vbslq_u32(t318[0], t321[2], res[2]), vbslq_u32(t318[1], t321[3], res[3])];
                live = [vbicq_u32(live[0], t318[0]), vbicq_u32(live[1], t318[1])];
            }
            if any(&live) {
                let t322: [uint32x4_t; 2] = [vsubq_u32(x16[0], x40[0]), vsubq_u32(x16[1], x40[1])];
                x48 = t322;
                let t323: u64 = 2u64;
                let t324 = [vdupq_n_u32((t323) as u32); 2];
                let t325: [uint32x4_t; 2] = [vandq_u32(x32[0], t324[0]), vandq_u32(x32[1], t324[1])];
                let t326: u64 = 0u64;
                let t327 = [vdupq_n_u32((t326) as u32); 2];
                let t328: [uint32x4_t; 2] = [vceqq_u32(t325[0], t327[0]), vceqq_u32(t325[1], t327[1])];
                let t329: [uint32x4_t; 2] = [vmvnq_u32(t328[0]), vmvnq_u32(t328[1])];
                let t330: [uint32x4_t; 2] = [vandq_u32(live[0], t329[0]), vandq_u32(live[1], t329[1])];
                {
                    let t331: [uint32x4_t; 4] = [x48[0], x48[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                    res = [vbslq_u32(t330[0], t331[0], res[0]), vbslq_u32(t330[1], t331[1], res[1]), vbslq_u32(t330[0], t331[2], res[2]), vbslq_u32(t330[1], t331[3], res[3])];
                    live = [vbicq_u32(live[0], t330[0]), vbicq_u32(live[1], t330[1])];
                }
                if any(&live) {
                    let t332: [uint32x4_t; 2] = [vceqq_u32(x48[0], t297[0]), vceqq_u32(x48[1], t297[1])];
                    let t333: [uint32x4_t; 2] = [vandq_u32(live[0], t332[0]), vandq_u32(live[1], t332[1])];
                    {
                        let t334: u64 = 4294967295u64;
                        let t335 = [vdupq_n_u32((t334) as u32); 2];
                        let t336: [uint32x4_t; 4] = [t335[0], t335[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                        res = [vbslq_u32(t333[0], t336[0], res[0]), vbslq_u32(t333[1], t336[1], res[1]), vbslq_u32(t333[0], t336[2], res[2]), vbslq_u32(t333[1], t336[3], res[3])];
                        live = [vbicq_u32(live[0], t333[0]), vbicq_u32(live[1], t333[1])];
                    }
                    if any(&live) {
                        let t337: u64 = (1u64).wrapping_shl((32u64) as u32);
                        let t338 = [vdupq_n_u32((t337) as u32); 2];
                        let t339 = [vdupq_n_u32(((t337) >> 32) as u32); 2];
                        let t340: [uint32x4_t; 4] = [vorrq_u32(t338[0], x48[0]), vorrq_u32(t338[1], x48[1]), t339[0], t339[1]];
                        res = [vbslq_u32(live[0], t340[0], res[0]), vbslq_u32(live[1], t340[1], res[1]), vbslq_u32(live[0], t340[2], res[2]), vbslq_u32(live[1], t340[3], res[3])];
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
    let mut x24: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t341 = b1_r0r1l_p(l, lid0, live, arg0, arg1, x16);
    x24 = t341;
    let mut t342 = live;
    loop {
        // Lanes part: each its own rounds.
        let t344: u64 = (1u64).wrapping_shl((32u64) as u32);
        let t345 = [vdupq_n_u32((t344) as u32); 2];
        let t346 = [vdupq_n_u32(((t344) >> 32) as u32); 2];
        let t347: [uint32x4_t; 4] = [vandq_u32(x24[0], t345[0]), vandq_u32(x24[1], t345[1]), vandq_u32(x24[2], t346[0]), vandq_u32(x24[3], t346[1])];
        let t348: u64 = 0u64;
        let t349 = [vdupq_n_u32((t348) as u32); 2];
        let t350: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t347[0], t349[0]), vceqq_u32(t347[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t347[1], t349[1]), vceqq_u32(t347[3], vdupq_n_u32(0)))];
        let t351: [uint32x4_t; 2] = [vmvnq_u32(t350[0]), vmvnq_u32(t350[1])];
        t342 = [vandq_u32(t342[0], t351[0]), vandq_u32(t342[1], t351[1])];
        if !(any(&t342)) { break; }
        let t353: [uint32x4_t; 2] = [x24[0], x24[1]];
        let t354 = b1_r0r1l_p(l, lid0, t342, arg0, arg1, t353);
        x24 = [vbslq_u32(t342[0], t354[0], x24[0]), vbslq_u32(t342[1], t354[1], x24[1]), vbslq_u32(t342[0], t354[2], x24[2]), vbslq_u32(t342[1], t354[3], x24[3])];
    }
    let t355: [uint32x4_t; 2] = [x24[0], x24[1]];
    res = t355;
    return res;
}

#[inline(always)]
unsafe fn b4_r0r1i_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t356 = b5_r0r1si_p(l, lid0, live, arg0, arg1, 0u32, x16);
    res = t356;
    return res;
}

#[inline(always)]
unsafe fn b4_r0r1i_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t357 = b5_r0r1si_f(l, lid0, arg0, arg1, 0u32, x16);
    res = t357;
    return res;
}

#[inline(always)]
unsafe fn b4_r0r1u8_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t358 = b5_r0r1su8_p(l, lid0, live, arg0, arg1, 0u32, x16);
    res = t358;
    return res;
}

#[inline(always)]
unsafe fn b4_r0r1u8_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t359 = b5_r0r1su8_f(l, lid0, arg0, arg1, 0u32, x16);
    res = t359;
    return res;
}

#[inline(always)]
unsafe fn b4_r0r1l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t360 = b5_r0r1sl_p(l, lid0, live, arg0, arg1, 0u32, x16);
    res = t360;
    return res;
}

#[inline(always)]
unsafe fn b5_r0r1si_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: u64 = 0;
    let mut x56: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t361 = b6_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    x32 = t361;
    let t362: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[1].as_ptr()))];
    let t363: [uint32x4_t; 2] = [vshlq_u32(t362[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(t362[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t364 = b7_r0r1sl_p(l, lid0, live, arg0, arg1, (1u32).wrapping_add(x16), t363);
    x40 = t364;
    let t365 = b8_s_p(l, lid0, live, x32);
    x48 = t365;
    let t366 = [vdupq_n_u32(1023u32); 2];
    let t367: [uint32x4_t; 2] = [vandq_u32(t362[0], t366[0]), vandq_u32(t362[1], t366[1])];
    let t368 = [vdupq_n_u32(x32); 2];
    let t369: [uint32x4_t; 2] = [vmulq_u32(t367[0], t368[0]), vmulq_u32(t367[1], t368[1])];
    let t370 = b9_r0r1lls_p(l, lid0, live, arg0, arg1, x40, t369, 8u32);
    let t371: u64 = x48;
    let t372 = [vdupq_n_u32((t371) as u32); 2];
    let t373 = [vdupq_n_u32(((t371) >> 32) as u32); 2];
    let t374: [uint32x4_t; 4] = [vandq_u32(t370[0], t372[0]), vandq_u32(t370[1], t372[1]), vandq_u32(t370[2], t373[0]), vandq_u32(t370[3], t373[1])];
    x56 = t374;
    let t375: u64 = x48;
    let t376 = [vdupq_n_u32((t375) as u32); 2];
    let t377 = [vdupq_n_u32(((t375) >> 32) as u32); 2];
    let t378: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x56[0], t376[0]), vceqq_u32(x56[2], t377[0])), vandq_u32(vceqq_u32(x56[1], t376[1]), vceqq_u32(x56[3], t377[1]))];
    let mut t379: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t380: [uint32x4_t; 2] = [vandq_u32(live[0], t378[0]), vandq_u32(live[1], t378[1])];
    let t381: [uint32x4_t; 2] = [vbicq_u32(live[0], t378[0]), vbicq_u32(live[1], t378[1])];
    if any(&t380) {
        let t382 = b10_r0r1si_p(l, lid0, t380, arg0, arg1, (x16).wrapping_add(1u32), x24);
        t379 = [vbslq_u32(t380[0], t382[0], t379[0]), vbslq_u32(t380[1], t382[1], t379[1]), vbslq_u32(t380[0], t382[2], t379[2]), vbslq_u32(t380[1], t382[3], t379[3])];
    }
    {
        t379 = [vbslq_u32(t381[0], x56[0], t379[0]), vbslq_u32(t381[1], x56[1], t379[1]), vbslq_u32(t381[0], x56[2], t379[2]), vbslq_u32(t381[1], x56[3], t379[3])];
    }
    res = t379;
    return res;
}

#[inline(always)]
unsafe fn b5_r0r1si_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32, x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: u64 = 0;
    let mut x56: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t383 = b6_r0r1s_f(l, lid0, arg0, arg1, x16);
    x32 = t383;
    let t384: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[1].as_ptr()))];
    let t385: [uint32x4_t; 2] = [vshlq_u32(t384[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(t384[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t386 = b7_r0r1sl_f(l, lid0, arg0, arg1, (1u32).wrapping_add(x16), t385);
    x40 = t386;
    let t387 = b8_s_f(l, lid0, x32);
    x48 = t387;
    let t388 = [vdupq_n_u32(1023u32); 2];
    let t389: [uint32x4_t; 2] = [vandq_u32(t384[0], t388[0]), vandq_u32(t384[1], t388[1])];
    let t390 = [vdupq_n_u32(x32); 2];
    let t391: [uint32x4_t; 2] = [vmulq_u32(t389[0], t390[0]), vmulq_u32(t389[1], t390[1])];
    let t392 = b9_r0r1lls_f(l, lid0, arg0, arg1, x40, t391, 8u32);
    let t393: u64 = x48;
    let t394 = [vdupq_n_u32((t393) as u32); 2];
    let t395 = [vdupq_n_u32(((t393) >> 32) as u32); 2];
    let t396: [uint32x4_t; 4] = [vandq_u32(t392[0], t394[0]), vandq_u32(t392[1], t394[1]), vandq_u32(t392[2], t395[0]), vandq_u32(t392[3], t395[1])];
    x56 = t396;
    let t397: u64 = x48;
    let t398 = [vdupq_n_u32((t397) as u32); 2];
    let t399 = [vdupq_n_u32(((t397) >> 32) as u32); 2];
    let t400: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x56[0], t398[0]), vceqq_u32(x56[2], t399[0])), vandq_u32(vceqq_u32(x56[1], t398[1]), vceqq_u32(x56[3], t399[1]))];
    let mut t401: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t402: [uint32x4_t; 2] = [vmvnq_u32(t400[0]), vmvnq_u32(t400[1])];
    if any(&t400) {
        let t403 = b10_r0r1si_p(l, lid0, t400, arg0, arg1, (x16).wrapping_add(1u32), x24);
        t401 = [vbslq_u32(t400[0], t403[0], t401[0]), vbslq_u32(t400[1], t403[1], t401[1]), vbslq_u32(t400[0], t403[2], t401[2]), vbslq_u32(t400[1], t403[3], t401[3])];
    }
    {
        t401 = [vbslq_u32(t402[0], x56[0], t401[0]), vbslq_u32(t402[1], x56[1], t401[1]), vbslq_u32(t402[0], x56[2], t401[2]), vbslq_u32(t402[1], x56[3], t401[3])];
    }
    res = t401;
    return res;
}

#[inline(always)]
unsafe fn b5_r0r1su8_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: u32 = 0;
    let mut x48: u64 = 0;
    let mut x56: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t404 = b6_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    x32 = t404;
    let t405 = b7_r0r1ss_p(l, lid0, live, arg0, arg1, (1u32).wrapping_add(x16), (x24 >> 10));
    x40 = t405;
    let t406 = b8_s_p(l, lid0, live, x32);
    x48 = t406;
    let t407 = b9_r0r1sns_p(l, lid0, live, arg0, arg1, x40, (x24 & 1023u32).wrapping_mul(x32), x32, 8u32);
    let t408: u64 = x48;
    let t409 = [vdupq_n_u32((t408) as u32); 2];
    let t410 = [vdupq_n_u32(((t408) >> 32) as u32); 2];
    let t411: [uint32x4_t; 4] = [vandq_u32(t407[0], t409[0]), vandq_u32(t407[1], t409[1]), vandq_u32(t407[2], t410[0]), vandq_u32(t407[3], t410[1])];
    x56 = t411;
    let t412: u64 = x48;
    let t413 = [vdupq_n_u32((t412) as u32); 2];
    let t414 = [vdupq_n_u32(((t412) >> 32) as u32); 2];
    let t415: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x56[0], t413[0]), vceqq_u32(x56[2], t414[0])), vandq_u32(vceqq_u32(x56[1], t413[1]), vceqq_u32(x56[3], t414[1]))];
    let mut t416: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t417: [uint32x4_t; 2] = [vandq_u32(live[0], t415[0]), vandq_u32(live[1], t415[1])];
    let t418: [uint32x4_t; 2] = [vbicq_u32(live[0], t415[0]), vbicq_u32(live[1], t415[1])];
    if any(&t417) {
        let t419 = b10_r0r1su8_p(l, lid0, t417, arg0, arg1, (x16).wrapping_add(1u32), x24);
        t416 = [vbslq_u32(t417[0], t419[0], t416[0]), vbslq_u32(t417[1], t419[1], t416[1]), vbslq_u32(t417[0], t419[2], t416[2]), vbslq_u32(t417[1], t419[3], t416[3])];
    }
    {
        t416 = [vbslq_u32(t418[0], x56[0], t416[0]), vbslq_u32(t418[1], x56[1], t416[1]), vbslq_u32(t418[0], x56[2], t416[2]), vbslq_u32(t418[1], x56[3], t416[3])];
    }
    res = t416;
    return res;
}

#[inline(always)]
unsafe fn b5_r0r1su8_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32, x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: u32 = 0;
    let mut x48: u64 = 0;
    let mut x56: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t420 = b6_r0r1s_f(l, lid0, arg0, arg1, x16);
    x32 = t420;
    let t421 = b7_r0r1ss_f(l, lid0, arg0, arg1, (1u32).wrapping_add(x16), (x24 >> 10));
    x40 = t421;
    let t422 = b8_s_f(l, lid0, x32);
    x48 = t422;
    let t423 = b9_r0r1sns_f(l, lid0, arg0, arg1, x40, (x24 & 1023u32).wrapping_mul(x32), x32, 8u32);
    let t424: u64 = x48;
    let t425 = [vdupq_n_u32((t424) as u32); 2];
    let t426 = [vdupq_n_u32(((t424) >> 32) as u32); 2];
    let t427: [uint32x4_t; 4] = [vandq_u32(t423[0], t425[0]), vandq_u32(t423[1], t425[1]), vandq_u32(t423[2], t426[0]), vandq_u32(t423[3], t426[1])];
    x56 = t427;
    let t428: u64 = x48;
    let t429 = [vdupq_n_u32((t428) as u32); 2];
    let t430 = [vdupq_n_u32(((t428) >> 32) as u32); 2];
    let t431: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x56[0], t429[0]), vceqq_u32(x56[2], t430[0])), vandq_u32(vceqq_u32(x56[1], t429[1]), vceqq_u32(x56[3], t430[1]))];
    let mut t432: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t433: [uint32x4_t; 2] = [vmvnq_u32(t431[0]), vmvnq_u32(t431[1])];
    if any(&t431) {
        let t434 = b10_r0r1su8_p(l, lid0, t431, arg0, arg1, (x16).wrapping_add(1u32), x24);
        t432 = [vbslq_u32(t431[0], t434[0], t432[0]), vbslq_u32(t431[1], t434[1], t432[1]), vbslq_u32(t431[0], t434[2], t432[2]), vbslq_u32(t431[1], t434[3], t432[3])];
    }
    {
        t432 = [vbslq_u32(t433[0], x56[0], t432[0]), vbslq_u32(t433[1], x56[1], t432[1]), vbslq_u32(t433[0], x56[2], t432[2]), vbslq_u32(t433[1], x56[3], t432[3])];
    }
    res = t432;
    return res;
}

#[inline(always)]
unsafe fn b5_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: u64 = 0;
    let mut x56: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t435 = b6_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    x32 = t435;
    let t436: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t437 = b7_r0r1sl_p(l, lid0, live, arg0, arg1, (1u32).wrapping_add(x16), t436);
    x40 = t437;
    let t438 = b8_s_p(l, lid0, live, x32);
    x48 = t438;
    let t439 = [vdupq_n_u32(1023u32); 2];
    let t440: [uint32x4_t; 2] = [vandq_u32(x24[0], t439[0]), vandq_u32(x24[1], t439[1])];
    let t441 = [vdupq_n_u32(x32); 2];
    let t442: [uint32x4_t; 2] = [vmulq_u32(t440[0], t441[0]), vmulq_u32(t440[1], t441[1])];
    let t443 = b9_r0r1lls_p(l, lid0, live, arg0, arg1, x40, t442, 8u32);
    let t444: u64 = x48;
    let t445 = [vdupq_n_u32((t444) as u32); 2];
    let t446 = [vdupq_n_u32(((t444) >> 32) as u32); 2];
    let t447: [uint32x4_t; 4] = [vandq_u32(t443[0], t445[0]), vandq_u32(t443[1], t445[1]), vandq_u32(t443[2], t446[0]), vandq_u32(t443[3], t446[1])];
    x56 = t447;
    let t448: u64 = x48;
    let t449 = [vdupq_n_u32((t448) as u32); 2];
    let t450 = [vdupq_n_u32(((t448) >> 32) as u32); 2];
    let t451: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x56[0], t449[0]), vceqq_u32(x56[2], t450[0])), vandq_u32(vceqq_u32(x56[1], t449[1]), vceqq_u32(x56[3], t450[1]))];
    let mut t452: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t453: [uint32x4_t; 2] = [vandq_u32(live[0], t451[0]), vandq_u32(live[1], t451[1])];
    let t454: [uint32x4_t; 2] = [vbicq_u32(live[0], t451[0]), vbicq_u32(live[1], t451[1])];
    if any(&t453) {
        let t455 = b10_r0r1sl_p(l, lid0, t453, arg0, arg1, (x16).wrapping_add(1u32), x24);
        t452 = [vbslq_u32(t453[0], t455[0], t452[0]), vbslq_u32(t453[1], t455[1], t452[1]), vbslq_u32(t453[0], t455[2], t452[2]), vbslq_u32(t453[1], t455[3], t452[3])];
    }
    {
        t452 = [vbslq_u32(t454[0], x56[0], t452[0]), vbslq_u32(t454[1], x56[1], t452[1]), vbslq_u32(t454[0], x56[2], t452[2]), vbslq_u32(t454[1], x56[3], t452[3])];
    }
    res = t452;
    return res;
}

#[inline(always)]
unsafe fn b6_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t456 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    res = (((t456).wrapping_shr(((4u32).wrapping_mul(x16)) as u32)) & (15u32));
    return res;
}

#[inline(always)]
unsafe fn b6_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t457 = b11_r0r1s_f(l, lid0, arg0, arg1, 16u32);
    res = (((t457).wrapping_shr(((4u32).wrapping_mul(x16)) as u32)) & (15u32));
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: u32 = 0;
    let mut live = k;
    let mut res: u32 = 0;
    let t458 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 40u32);
    let t459 = b12_r0r1s_p(l, lid0, live, arg0, arg1, 0u32);
    x32 = ((x24).wrapping_mul(t458)).wrapping_add(t459);
    if ((x16) == (0u32)) {
        res = x32;
        return res;
    }
    let t460 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    let t461 = b13_ss_p(l, lid0, live, (t460).wrapping_sub((x24).wrapping_shl((10u32) as u32)), 1024u32);
    x40 = t461;
    let t462 = b12_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    res = (x32).wrapping_add((x40).wrapping_mul(t462));
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1ss_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32, x24: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: u32 = 0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t463 = b11_r0r1s_f(l, lid0, arg0, arg1, 40u32);
    let t464 = b12_r0r1s_f(l, lid0, arg0, arg1, 0u32);
    x32 = ((x24).wrapping_mul(t463)).wrapping_add(t464);
    if ((x16) == (0u32)) {
        res = x32;
        return res;
    }
    let t465 = b11_r0r1s_f(l, lid0, arg0, arg1, 4u32);
    let t466 = b13_ss_f(l, lid0, (t465).wrapping_sub((x24).wrapping_shl((10u32) as u32)), 1024u32);
    x40 = t466;
    let t467 = b12_r0r1s_f(l, lid0, arg0, arg1, x16);
    res = (x32).wrapping_add((x40).wrapping_mul(t467));
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t468 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 40u32);
    let t469 = [vdupq_n_u32(t468); 2];
    let t470: [uint32x4_t; 2] = [vmulq_u32(x24[0], t469[0]), vmulq_u32(x24[1], t469[1])];
    let t471 = b12_r0r1s_p(l, lid0, live, arg0, arg1, 0u32);
    let t472 = [vdupq_n_u32(t471); 2];
    let t473: [uint32x4_t; 2] = [vaddq_u32(t470[0], t472[0]), vaddq_u32(t470[1], t472[1])];
    x32 = t473;
    if ((x16) == (0u32)) {
        res = x32;
        return res;
    }
    let t474 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    let t475: [uint32x4_t; 2] = [vshlq_u32(x24[0], vdupq_n_s32(((10u32) & 31) as i32)), vshlq_u32(x24[1], vdupq_n_s32(((10u32) & 31) as i32))];
    let t476 = [vdupq_n_u32(t474); 2];
    let t477: [uint32x4_t; 2] = [vsubq_u32(t476[0], t475[0]), vsubq_u32(t476[1], t475[1])];
    let t478 = b13_ls_p(l, lid0, live, t477, 1024u32);
    x40 = t478;
    let t479 = b12_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    let t480 = [vdupq_n_u32(t479); 2];
    let t481: [uint32x4_t; 2] = [vmulq_u32(x40[0], t480[0]), vmulq_u32(x40[1], t480[1])];
    let t482: [uint32x4_t; 2] = [vaddq_u32(x32[0], t481[0]), vaddq_u32(x32[1], t481[1])];
    res = t482;
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1sl_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t483 = b11_r0r1s_f(l, lid0, arg0, arg1, 40u32);
    let t484 = [vdupq_n_u32(t483); 2];
    let t485: [uint32x4_t; 2] = [vmulq_u32(x24[0], t484[0]), vmulq_u32(x24[1], t484[1])];
    let t486 = b12_r0r1s_f(l, lid0, arg0, arg1, 0u32);
    let t487 = [vdupq_n_u32(t486); 2];
    let t488: [uint32x4_t; 2] = [vaddq_u32(t485[0], t487[0]), vaddq_u32(t485[1], t487[1])];
    x32 = t488;
    if ((x16) == (0u32)) {
        res = x32;
        return res;
    }
    let t489 = b11_r0r1s_f(l, lid0, arg0, arg1, 4u32);
    let t490: [uint32x4_t; 2] = [vshlq_u32(x24[0], vdupq_n_s32(((10u32) & 31) as i32)), vshlq_u32(x24[1], vdupq_n_s32(((10u32) & 31) as i32))];
    let t491 = [vdupq_n_u32(t489); 2];
    let t492: [uint32x4_t; 2] = [vsubq_u32(t491[0], t490[0]), vsubq_u32(t491[1], t490[1])];
    let t493 = b13_ls_f(l, lid0, t492, 1024u32);
    x40 = t493;
    let t494 = b12_r0r1s_f(l, lid0, arg0, arg1, x16);
    let t495 = [vdupq_n_u32(t494); 2];
    let t496: [uint32x4_t; 2] = [vmulq_u32(x40[0], t495[0]), vmulq_u32(x40[1], t495[1])];
    let t497: [uint32x4_t; 2] = [vaddq_u32(x32[0], t496[0]), vaddq_u32(x32[1], t496[1])];
    res = t497;
    return res;
}

#[inline(always)]
unsafe fn b8_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t498: u64;
    if ((x0) >= (8u32)) {
        t498 = !(0u64);
    } else {
        t498 = ((1u64).wrapping_shl(((((x0).wrapping_mul(8u32)) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t498;
    return res;
}

#[inline(always)]
unsafe fn b8_s_f(l: &Launch, lid0: u32, x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t499: u64;
    if ((x0) >= (8u32)) {
        t499 = !(0u64);
    } else {
        t499 = ((1u64).wrapping_shl(((((x0).wrapping_mul(8u32)) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t499;
    return res;
}

#[inline(always)]
unsafe fn b8_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t500 = [vdupq_n_u32(8u32); 2];
    let t501: [uint32x4_t; 2] = [vcgeq_u32(x0[0], t500[0]), vcgeq_u32(x0[1], t500[1])];
    let mut t502: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t503: [uint32x4_t; 2] = [vandq_u32(live[0], t501[0]), vandq_u32(live[1], t501[1])];
    let t504: [uint32x4_t; 2] = [vbicq_u32(live[0], t501[0]), vbicq_u32(live[1], t501[1])];
    {
        let t505: u64 = !(0u64);
        let t506 = [vdupq_n_u32((t505) as u32); 2];
        let t507 = [vdupq_n_u32(((t505) >> 32) as u32); 2];
        let t508: [uint32x4_t; 4] = [t506[0], t506[1], t507[0], t507[1]];
        t502 = [vbslq_u32(t503[0], t508[0], t502[0]), vbslq_u32(t503[1], t508[1], t502[1]), vbslq_u32(t503[0], t508[2], t502[2]), vbslq_u32(t503[1], t508[3], t502[3])];
    }
    {
        let t509: [uint32x4_t; 2] = [vmulq_u32(x0[0], t500[0]), vmulq_u32(x0[1], t500[1])];
        let t510: u64 = 1u64;
        let t511 = [vdupq_n_u32((t510) as u32); 2];
        let t512: [uint32x4_t; 2] = [t509[0], t509[1]];
        let t513: [uint32x4_t; 2] = [vandq_u32(t512[0], vdupq_n_u32(63)), vandq_u32(t512[1], vdupq_n_u32(63))];
        let t514: [uint32x4_t; 4] = [vshlq_u32(t511[0], vreinterpretq_s32_u32(t513[0])), vshlq_u32(t511[1], vreinterpretq_s32_u32(t513[1])), vshlq_u32(t511[0], vsubq_s32(vreinterpretq_s32_u32(t513[0]), vdupq_n_s32(32))), vshlq_u32(t511[1], vsubq_s32(vreinterpretq_s32_u32(t513[1]), vdupq_n_s32(32)))];
        let t515: u64 = 1u64;
        let t516 = [vdupq_n_u32((t515) as u32); 2];
        let t517: [uint32x4_t; 2] = [vcltq_u32(t514[0], t516[0]), vcltq_u32(t514[1], t516[1])];
        let t518: [uint32x4_t; 4] = [vsubq_u32(t514[0], t516[0]), vsubq_u32(t514[1], t516[1]), vaddq_u32(vsubq_u32(t514[2], vdupq_n_u32(0)), t517[0]), vaddq_u32(vsubq_u32(t514[3], vdupq_n_u32(0)), t517[1])];
        t502 = [vbslq_u32(t504[0], t518[0], t502[0]), vbslq_u32(t504[1], t518[1], t502[1]), vbslq_u32(t504[0], t518[2], t502[2]), vbslq_u32(t504[1], t518[3], t502[3])];
    }
    res = t502;
    return res;
}

#[inline(always)]
unsafe fn b9_r0r1sns_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24f: u32, x24s: u32, x32: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t519 = b14_r0sns_p(l, lid0, live, arg0, x16, x24f, x24s, x32);
    res = t519;
    return res;
}

#[inline(always)]
unsafe fn b9_r0r1sns_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32, x24f: u32, x24s: u32, x32: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t520 = b14_r0sns_f(l, lid0, arg0, x16, x24f, x24s, x32);
    res = t520;
    return res;
}

#[inline(always)]
unsafe fn b9_r0r1lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2], x24: [uint32x4_t; 2], x32: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t521 = b14_r0lls_p(l, lid0, live, arg0, x16, x24, x32);
    res = t521;
    return res;
}

#[inline(always)]
unsafe fn b9_r0r1lls_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2], x24: [uint32x4_t; 2], x32: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t522 = b14_r0lls_f(l, lid0, arg0, x16, x24, x32);
    res = t522;
    return res;
}

#[inline(always)]
unsafe fn b10_r0r1si_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: u32) -> [uint32x4_t; 4] {
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
    let mut x136: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x144: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x152: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x160: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x168: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x176: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x184: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t523 = b11_r0r1s_p(l, lid0, live, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x16)));
    x32 = t523;
    let t524 = b11_r0r1s_p(l, lid0, live, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x16)));
    x40 = t524;
    let t525: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[1].as_ptr()))];
    let t526: [uint32x4_t; 2] = [vshlq_u32(t525[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(t525[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    x48 = t526;
    let t527 = [vdupq_n_u32(4u32); 2];
    let t528: [uint32x4_t; 2] = [vmulq_u32(x48[0], t527[0]), vmulq_u32(x48[1], t527[1])];
    let t529 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x40, t528, 4u32);
    let t530: [uint32x4_t; 2] = [t529[0], t529[1]];
    x56 = t530;
    let t531: [uint32x4_t; 2] = [vmulq_u32(x48[0], t527[0]), vmulq_u32(x48[1], t527[1])];
    let t532: [uint32x4_t; 2] = [vaddq_u32(t531[0], t527[0]), vaddq_u32(t531[1], t527[1])];
    let t533 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x40, t532, 4u32);
    let t534: [uint32x4_t; 2] = [t533[0], t533[1]];
    x64 = t534;
    let t535 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    x72 = ((t535).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
    let t536: [uint32x4_t; 2] = [vmulq_u32(x48[0], t527[0]), vmulq_u32(x48[1], t527[1])];
    let t537 = [vdupq_n_u32(x72); 2];
    let t538: [uint32x4_t; 2] = [vsubq_u32(t537[0], t536[0]), vsubq_u32(t537[1], t536[1])];
    let t539 = b16_sl_p(l, lid0, live, 4u32, t538);
    x80 = t539;
    let t540: [uint32x4_t; 2] = [vshlq_u32(t525[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(t525[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
    let t541 = [vdupq_n_u32(3u32); 2];
    let t542: [uint32x4_t; 2] = [vandq_u32(t540[0], t541[0]), vandq_u32(t540[1], t541[1])];
    x88 = t542;
    let t543 = [vdupq_n_u32(0u32); 2];
    let t544: [uint32x4_t; 2] = [vceqq_u32(x88[0], t543[0]), vceqq_u32(x88[1], t543[1])];
    let mut t545: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t546: [uint32x4_t; 2] = [vandq_u32(live[0], t544[0]), vandq_u32(live[1], t544[1])];
    let t547: [uint32x4_t; 2] = [vbicq_u32(live[0], t544[0]), vbicq_u32(live[1], t544[1])];
    {
        t545 = [vbslq_u32(t546[0], t543[0], t545[0]), vbslq_u32(t546[1], t543[1], t545[1])];
    }
    if any(&t547) {
        let t548 = [vdupq_n_u32(1u32); 2];
        let t549: [uint32x4_t; 2] = [vsubq_u32(x88[0], t548[0]), vsubq_u32(x88[1], t548[1])];
        let t550 = [vdupq_n_u32(2u32); 2];
        let t551: [uint32x4_t; 2] = [vmulq_u32(t550[0], t549[0]), vmulq_u32(t550[1], t549[1])];
        let t552: [uint32x4_t; 2] = [vaddq_u32(x56[0], t551[0]), vaddq_u32(x56[1], t551[1])];
        let t553 = b17_ll_p(l, lid0, t547, t552, x64);
        let t554 = b15_r0r1sls_p(l, lid0, t547, arg0, arg1, x32, t553, 2u32);
        let t555: [uint32x4_t; 2] = [t554[0], t554[1]];
        t545 = [vbslq_u32(t547[0], t555[0], t545[0]), vbslq_u32(t547[1], t555[1], t545[1])];
    }
    x96 = t545;
    let t556 = [vdupq_n_u32(2u32); 2];
    let t557: [uint32x4_t; 2] = [vmulq_u32(t556[0], x88[0]), vmulq_u32(t556[1], x88[1])];
    let t558: [uint32x4_t; 2] = [vaddq_u32(x56[0], t557[0]), vaddq_u32(x56[1], t557[1])];
    let t559 = b17_ll_p(l, lid0, live, t558, x64);
    let t560 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t559, 2u32);
    let t561: [uint32x4_t; 2] = [t560[0], t560[1]];
    x104 = t561;
    let t562: [uint32x4_t; 2] = [vmulq_u32(t556[0], x80[0]), vmulq_u32(t556[1], x80[1])];
    let t563: [uint32x4_t; 2] = [vaddq_u32(x56[0], t562[0]), vaddq_u32(x56[1], t562[1])];
    x112 = t563;
    let t564 = [vdupq_n_u32(255u32); 2];
    let t565: [uint32x4_t; 2] = [vandq_u32(t525[0], t564[0]), vandq_u32(t525[1], t564[1])];
    x120 = t565;
    x128 = 9259542123273814144u64;
    let t566: u64 = 72340172838076673u64;
    let t567 = [vdupq_n_u32((t566) as u32); 2];
    let t568 = [vdupq_n_u32(((t566) >> 32) as u32); 2];
    let t569: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x120[0]), vget_low_u32(t567[0]))), vreinterpretq_u32_u64(vmull_high_u32(x120[0], t567[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x120[1]), vget_low_u32(t567[1]))), vreinterpretq_u32_u64(vmull_high_u32(x120[1], t567[1])))];
    let t570: [uint32x4_t; 4] = [vmulq_u32(x120[0], t567[0]), vmulq_u32(x120[1], t567[1]), vmlaq_u32(t569[0], x120[0], t568[0]), vmlaq_u32(t569[1], x120[1], t568[1])];
    x136 = t570;
    x144 = x96;
    x152 = x96;
    let mut t571 = live;
    loop {
        // Lanes part: each its own rounds.
        let t573: [uint32x4_t; 2] = [vcltq_u32(x152[0], x104[0]), vcltq_u32(x152[1], x104[1])];
        t571 = [vandq_u32(t571[0], t573[0]), vandq_u32(t571[1], t573[1])];
        if !(any(&t571)) { break; }
        let t575: [uint32x4_t; 2] = [vaddq_u32(x112[0], x152[0]), vaddq_u32(x112[1], x152[1])];
        let t576 = b17_ll_p(l, lid0, t571, t575, x64);
        let t577 = b15_r0r1sls_p(l, lid0, t571, arg0, arg1, x32, t576, 8u32);
        x160 = [vbslq_u32(t571[0], t577[0], x160[0]), vbslq_u32(t571[1], t577[1], x160[1]), vbslq_u32(t571[0], t577[2], x160[2]), vbslq_u32(t571[1], t577[3], x160[3])];
        let t578: u64 = x128;
        let t579 = [vdupq_n_u32((t578) as u32); 2];
        let t580 = [vdupq_n_u32(((t578) >> 32) as u32); 2];
        let t581: [uint32x4_t; 4] = [vorrq_u32(x160[0], t579[0]), vorrq_u32(x160[1], t579[1]), vorrq_u32(x160[2], t580[0]), vorrq_u32(x160[3], t580[1])];
        let t582: u64 = !(x128);
        let t583 = [vdupq_n_u32((t582) as u32); 2];
        let t584 = [vdupq_n_u32(((t582) >> 32) as u32); 2];
        let t585: [uint32x4_t; 4] = [vandq_u32(x136[0], t583[0]), vandq_u32(x136[1], t583[1]), vandq_u32(x136[2], t584[0]), vandq_u32(x136[3], t584[1])];
        let t586: [uint32x4_t; 2] = [vcltq_u32(t581[0], t585[0]), vcltq_u32(t581[1], t585[1])];
        let t587: [uint32x4_t; 4] = [vsubq_u32(t581[0], t585[0]), vsubq_u32(t581[1], t585[1]), vaddq_u32(vsubq_u32(t581[2], t585[2]), t586[0]), vaddq_u32(vsubq_u32(t581[3], t585[3]), t586[1])];
        x168 = [vbslq_u32(t571[0], t587[0], x168[0]), vbslq_u32(t571[1], t587[1], x168[1]), vbslq_u32(t571[0], t587[2], x168[2]), vbslq_u32(t571[1], t587[3], x168[3])];
        let t588: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x160[r]));
        let t589: [uint32x4_t; 4] = [vandq_u32(t588[0], x136[0]), vandq_u32(t588[1], x136[1]), vandq_u32(t588[2], x136[2]), vandq_u32(t588[3], x136[3])];
        let t590: [uint32x4_t; 4] = [veorq_u32(x160[0], x136[0]), veorq_u32(x160[1], x136[1]), veorq_u32(x160[2], x136[2]), veorq_u32(x160[3], x136[3])];
        let t591: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(t590[r]));
        let t592: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x168[r]));
        let t593: [uint32x4_t; 4] = [vandq_u32(t591[0], t592[0]), vandq_u32(t591[1], t592[1]), vandq_u32(t591[2], t592[2]), vandq_u32(t591[3], t592[3])];
        let t594: [uint32x4_t; 4] = [vorrq_u32(t589[0], t593[0]), vorrq_u32(t589[1], t593[1]), vorrq_u32(t589[2], t593[2]), vorrq_u32(t589[3], t593[3])];
        let t595: u64 = x128;
        let t596 = [vdupq_n_u32((t595) as u32); 2];
        let t597 = [vdupq_n_u32(((t595) >> 32) as u32); 2];
        let t598: [uint32x4_t; 4] = [vandq_u32(t596[0], t594[0]), vandq_u32(t596[1], t594[1]), vandq_u32(t597[0], t594[2]), vandq_u32(t597[1], t594[3])];
        x176 = t598;
        let t599: [uint32x4_t; 2] = [vsubq_u32(x104[0], x152[0]), vsubq_u32(x104[1], x152[1])];
        x184 = t599;
        let t600 = [vdupq_n_u32(8u32); 2];
        let t601: [uint32x4_t; 2] = [vcgeq_u32(x184[0], t600[0]), vcgeq_u32(x184[1], t600[1])];
        let mut t602: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t603: [uint32x4_t; 2] = [vandq_u32(t571[0], t601[0]), vandq_u32(t571[1], t601[1])];
        let t604: [uint32x4_t; 2] = [vbicq_u32(t571[0], t601[0]), vbicq_u32(t571[1], t601[1])];
        {
            t602 = [vbslq_u32(t603[0], x176[0], t602[0]), vbslq_u32(t603[1], x176[1], t602[1]), vbslq_u32(t603[0], x176[2], t602[2]), vbslq_u32(t603[1], x176[3], t602[3])];
        }
        {
            let t605: [uint32x4_t; 2] = [vmulq_u32(t600[0], x184[0]), vmulq_u32(t600[1], x184[1])];
            let t606: u64 = 1u64;
            let t607 = [vdupq_n_u32((t606) as u32); 2];
            let t608: [uint32x4_t; 2] = [t605[0], t605[1]];
            let t609: [uint32x4_t; 2] = [vandq_u32(t608[0], vdupq_n_u32(63)), vandq_u32(t608[1], vdupq_n_u32(63))];
            let t610: [uint32x4_t; 4] = [vshlq_u32(t607[0], vreinterpretq_s32_u32(t609[0])), vshlq_u32(t607[1], vreinterpretq_s32_u32(t609[1])), vshlq_u32(t607[0], vsubq_s32(vreinterpretq_s32_u32(t609[0]), vdupq_n_s32(32))), vshlq_u32(t607[1], vsubq_s32(vreinterpretq_s32_u32(t609[1]), vdupq_n_s32(32)))];
            let t611: u64 = 1u64;
            let t612 = [vdupq_n_u32((t611) as u32); 2];
            let t613: [uint32x4_t; 2] = [vcltq_u32(t610[0], t612[0]), vcltq_u32(t610[1], t612[1])];
            let t614: [uint32x4_t; 4] = [vsubq_u32(t610[0], t612[0]), vsubq_u32(t610[1], t612[1]), vaddq_u32(vsubq_u32(t610[2], vdupq_n_u32(0)), t613[0]), vaddq_u32(vsubq_u32(t610[3], vdupq_n_u32(0)), t613[1])];
            let t615: [uint32x4_t; 4] = [vandq_u32(x176[0], t614[0]), vandq_u32(x176[1], t614[1]), vandq_u32(x176[2], t614[2]), vandq_u32(x176[3], t614[3])];
            t602 = [vbslq_u32(t604[0], t615[0], t602[0]), vbslq_u32(t604[1], t615[1], t602[1]), vbslq_u32(t604[0], t615[2], t602[2]), vbslq_u32(t604[1], t615[3], t602[3])];
        }
        let t616 = b18_l_p(l, lid0, t571, t602);
        let t617: [uint32x4_t; 2] = [vaddq_u32(x144[0], t616[0]), vaddq_u32(x144[1], t616[1])];
        x144 = [vbslq_u32(t571[0], t617[0], x144[0]), vbslq_u32(t571[1], t617[1], x144[1])];
        let t618: [uint32x4_t; 2] = [vandq_u32(t571[0], live[0]), vandq_u32(t571[1], live[1])];
        x152 = [vaddq_u32(x152[0], vandq_u32(t618[0], vdupq_n_u32(8))), vaddq_u32(x152[1], vandq_u32(t618[1], vdupq_n_u32(8)))];
    }
    x96 = x144;
    let t619 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    let t620 = [vdupq_n_u32((((t619).wrapping_shr((((((4u32).wrapping_mul(x16)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32))); 2];
    x152 = t620;
    let t621 = [vdupq_n_u32(1u32); 2];
    let t622: [uint32x4_t; 2] = [vsubq_u32(x80[0], t621[0]), vsubq_u32(x80[1], t621[1])];
    let t623: [uint32x4_t; 2] = [vmulq_u32(t556[0], t622[0]), vmulq_u32(t556[1], t622[1])];
    let t624: [uint32x4_t; 2] = [vaddq_u32(x56[0], t623[0]), vaddq_u32(x56[1], t623[1])];
    let t625 = b17_ll_p(l, lid0, live, t624, x64);
    let t626 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t625, 2u32);
    let t627: [uint32x4_t; 2] = [t626[0], t626[1]];
    let t628: [uint32x4_t; 4] = [t627[0], t627[1], vdupq_n_u32(0), vdupq_n_u32(0)];
    x160 = t628;
    let t629: [uint32x4_t; 2] = [x160[0], x160[1]];
    let t630: [uint32x4_t; 2] = [vaddq_u32(x112[0], t629[0]), vaddq_u32(x112[1], t629[1])];
    let t631: [uint32x4_t; 2] = [vmulq_u32(x96[0], x152[0]), vmulq_u32(x96[1], x152[1])];
    let t632: [uint32x4_t; 2] = [vaddq_u32(t630[0], t631[0]), vaddq_u32(t630[1], t631[1])];
    let t633 = b17_ll_p(l, lid0, live, t632, x64);
    let t634: [uint32x4_t; 4] = [t633[0], t633[1], vdupq_n_u32(0), vdupq_n_u32(0)];
    x168 = t634;
    let t635: [uint32x4_t; 2] = [x168[0], x168[1]];
    let t636 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t635, 8u32);
    let t637 = b8_l_p(l, lid0, live, x152);
    let t638: [uint32x4_t; 4] = [vandq_u32(t636[0], t637[0]), vandq_u32(t636[1], t637[1]), vandq_u32(t636[2], t637[2]), vandq_u32(t636[3], t637[3])];
    res = t638;
    return res;
}

#[inline(always)]
unsafe fn b10_r0r1su8_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: u32 = 0;
    let mut x48: u32 = 0;
    let mut x56: u32 = 0;
    let mut x64: u32 = 0;
    let mut x72: u32 = 0;
    let mut x80: u32 = 0;
    let mut x88: u32 = 0;
    let mut x96: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x104: u32 = 0;
    let mut x112: u32 = 0;
    let mut x120: u32 = 0;
    let mut x128: u64 = 0;
    let mut x136: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x144: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x152: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x160: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x168: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x176: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x184: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t639 = b11_r0r1s_p(l, lid0, live, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x16)));
    x32 = t639;
    let t640 = b11_r0r1s_p(l, lid0, live, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x16)));
    x40 = t640;
    x48 = (x24 >> 10);
    let t641 = b15_r0r1sss_p(l, lid0, live, arg0, arg1, x40, (x48).wrapping_mul(4u32), 4u32);
    x56 = ((t641) as u32);
    let t642 = b15_r0r1sss_p(l, lid0, live, arg0, arg1, x40, ((x48).wrapping_mul(4u32)).wrapping_add(4u32), 4u32);
    x64 = ((t642) as u32);
    let t643 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    x72 = ((t643).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
    let t644 = b16_ss_p(l, lid0, live, 4u32, (x72).wrapping_sub((x48).wrapping_mul(4u32)));
    x80 = t644;
    x88 = (((x24 >> 8)) & (3u32));
    let t645: u32;
    if ((x88) == (0u32)) {
        t645 = 0u32;
    } else {
        let t646 = b17_ss_p(l, lid0, live, (x56).wrapping_add((2u32).wrapping_mul((x88).wrapping_sub(1u32))), x64);
        let t647 = b15_r0r1sss_p(l, lid0, live, arg0, arg1, x32, t646, 2u32);
        t645 = ((t647) as u32);
    }
    let t648 = [vdupq_n_u32(t645); 2];
    x96 = t648;
    let t649 = b17_ss_p(l, lid0, live, (x56).wrapping_add((2u32).wrapping_mul(x88)), x64);
    let t650 = b15_r0r1sss_p(l, lid0, live, arg0, arg1, x32, t649, 2u32);
    x104 = ((t650) as u32);
    x112 = (x56).wrapping_add((2u32).wrapping_mul(x80));
    x120 = (x24 & 255u32);
    x128 = 9259542123273814144u64;
    let t651: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x120), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x120), vld1q_u32(LANE[1].as_ptr()))];
    let t652: u64 = 72340172838076673u64;
    let t653 = [vdupq_n_u32((t652) as u32); 2];
    let t654 = [vdupq_n_u32(((t652) >> 32) as u32); 2];
    let t655: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(t651[0]), vget_low_u32(t653[0]))), vreinterpretq_u32_u64(vmull_high_u32(t651[0], t653[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(t651[1]), vget_low_u32(t653[1]))), vreinterpretq_u32_u64(vmull_high_u32(t651[1], t653[1])))];
    let t656: [uint32x4_t; 4] = [vmulq_u32(t651[0], t653[0]), vmulq_u32(t651[1], t653[1]), vmlaq_u32(t655[0], t651[0], t654[0]), vmlaq_u32(t655[1], t651[1], t654[1])];
    x136 = t656;
    x144 = x96;
    x152 = x96;
    let mut t657 = live;
    loop {
        // Lanes part: each its own rounds.
        let t659 = [vdupq_n_u32(x104); 2];
        let t660: [uint32x4_t; 2] = [vcltq_u32(x152[0], t659[0]), vcltq_u32(x152[1], t659[1])];
        t657 = [vandq_u32(t657[0], t660[0]), vandq_u32(t657[1], t660[1])];
        if !(any(&t657)) { break; }
        let t662 = [vdupq_n_u32(x112); 2];
        let t663: [uint32x4_t; 2] = [vaddq_u32(t662[0], x152[0]), vaddq_u32(t662[1], x152[1])];
        let t664 = b17_ls_p(l, lid0, t657, t663, x64);
        let t665 = b15_r0r1sls_p(l, lid0, t657, arg0, arg1, x32, t664, 8u32);
        x160 = [vbslq_u32(t657[0], t665[0], x160[0]), vbslq_u32(t657[1], t665[1], x160[1]), vbslq_u32(t657[0], t665[2], x160[2]), vbslq_u32(t657[1], t665[3], x160[3])];
        let t666: u64 = x128;
        let t667 = [vdupq_n_u32((t666) as u32); 2];
        let t668 = [vdupq_n_u32(((t666) >> 32) as u32); 2];
        let t669: [uint32x4_t; 4] = [vorrq_u32(x160[0], t667[0]), vorrq_u32(x160[1], t667[1]), vorrq_u32(x160[2], t668[0]), vorrq_u32(x160[3], t668[1])];
        let t670: u64 = !(x128);
        let t671 = [vdupq_n_u32((t670) as u32); 2];
        let t672 = [vdupq_n_u32(((t670) >> 32) as u32); 2];
        let t673: [uint32x4_t; 4] = [vandq_u32(x136[0], t671[0]), vandq_u32(x136[1], t671[1]), vandq_u32(x136[2], t672[0]), vandq_u32(x136[3], t672[1])];
        let t674: [uint32x4_t; 2] = [vcltq_u32(t669[0], t673[0]), vcltq_u32(t669[1], t673[1])];
        let t675: [uint32x4_t; 4] = [vsubq_u32(t669[0], t673[0]), vsubq_u32(t669[1], t673[1]), vaddq_u32(vsubq_u32(t669[2], t673[2]), t674[0]), vaddq_u32(vsubq_u32(t669[3], t673[3]), t674[1])];
        x168 = [vbslq_u32(t657[0], t675[0], x168[0]), vbslq_u32(t657[1], t675[1], x168[1]), vbslq_u32(t657[0], t675[2], x168[2]), vbslq_u32(t657[1], t675[3], x168[3])];
        let t676: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x160[r]));
        let t677: [uint32x4_t; 4] = [vandq_u32(t676[0], x136[0]), vandq_u32(t676[1], x136[1]), vandq_u32(t676[2], x136[2]), vandq_u32(t676[3], x136[3])];
        let t678: [uint32x4_t; 4] = [veorq_u32(x160[0], x136[0]), veorq_u32(x160[1], x136[1]), veorq_u32(x160[2], x136[2]), veorq_u32(x160[3], x136[3])];
        let t679: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(t678[r]));
        let t680: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x168[r]));
        let t681: [uint32x4_t; 4] = [vandq_u32(t679[0], t680[0]), vandq_u32(t679[1], t680[1]), vandq_u32(t679[2], t680[2]), vandq_u32(t679[3], t680[3])];
        let t682: [uint32x4_t; 4] = [vorrq_u32(t677[0], t681[0]), vorrq_u32(t677[1], t681[1]), vorrq_u32(t677[2], t681[2]), vorrq_u32(t677[3], t681[3])];
        let t683: u64 = x128;
        let t684 = [vdupq_n_u32((t683) as u32); 2];
        let t685 = [vdupq_n_u32(((t683) >> 32) as u32); 2];
        let t686: [uint32x4_t; 4] = [vandq_u32(t684[0], t682[0]), vandq_u32(t684[1], t682[1]), vandq_u32(t685[0], t682[2]), vandq_u32(t685[1], t682[3])];
        x176 = t686;
        let t687 = [vdupq_n_u32(x104); 2];
        let t688: [uint32x4_t; 2] = [vsubq_u32(t687[0], x152[0]), vsubq_u32(t687[1], x152[1])];
        x184 = t688;
        let t689 = [vdupq_n_u32(8u32); 2];
        let t690: [uint32x4_t; 2] = [vcgeq_u32(x184[0], t689[0]), vcgeq_u32(x184[1], t689[1])];
        let mut t691: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t692: [uint32x4_t; 2] = [vandq_u32(t657[0], t690[0]), vandq_u32(t657[1], t690[1])];
        let t693: [uint32x4_t; 2] = [vbicq_u32(t657[0], t690[0]), vbicq_u32(t657[1], t690[1])];
        {
            t691 = [vbslq_u32(t692[0], x176[0], t691[0]), vbslq_u32(t692[1], x176[1], t691[1]), vbslq_u32(t692[0], x176[2], t691[2]), vbslq_u32(t692[1], x176[3], t691[3])];
        }
        {
            let t694: [uint32x4_t; 2] = [vmulq_u32(t689[0], x184[0]), vmulq_u32(t689[1], x184[1])];
            let t695: u64 = 1u64;
            let t696 = [vdupq_n_u32((t695) as u32); 2];
            let t697: [uint32x4_t; 2] = [t694[0], t694[1]];
            let t698: [uint32x4_t; 2] = [vandq_u32(t697[0], vdupq_n_u32(63)), vandq_u32(t697[1], vdupq_n_u32(63))];
            let t699: [uint32x4_t; 4] = [vshlq_u32(t696[0], vreinterpretq_s32_u32(t698[0])), vshlq_u32(t696[1], vreinterpretq_s32_u32(t698[1])), vshlq_u32(t696[0], vsubq_s32(vreinterpretq_s32_u32(t698[0]), vdupq_n_s32(32))), vshlq_u32(t696[1], vsubq_s32(vreinterpretq_s32_u32(t698[1]), vdupq_n_s32(32)))];
            let t700: u64 = 1u64;
            let t701 = [vdupq_n_u32((t700) as u32); 2];
            let t702: [uint32x4_t; 2] = [vcltq_u32(t699[0], t701[0]), vcltq_u32(t699[1], t701[1])];
            let t703: [uint32x4_t; 4] = [vsubq_u32(t699[0], t701[0]), vsubq_u32(t699[1], t701[1]), vaddq_u32(vsubq_u32(t699[2], vdupq_n_u32(0)), t702[0]), vaddq_u32(vsubq_u32(t699[3], vdupq_n_u32(0)), t702[1])];
            let t704: [uint32x4_t; 4] = [vandq_u32(x176[0], t703[0]), vandq_u32(x176[1], t703[1]), vandq_u32(x176[2], t703[2]), vandq_u32(x176[3], t703[3])];
            t691 = [vbslq_u32(t693[0], t704[0], t691[0]), vbslq_u32(t693[1], t704[1], t691[1]), vbslq_u32(t693[0], t704[2], t691[2]), vbslq_u32(t693[1], t704[3], t691[3])];
        }
        let t705 = b18_l_p(l, lid0, t657, t691);
        let t706: [uint32x4_t; 2] = [vaddq_u32(x144[0], t705[0]), vaddq_u32(x144[1], t705[1])];
        x144 = [vbslq_u32(t657[0], t706[0], x144[0]), vbslq_u32(t657[1], t706[1], x144[1])];
        let t707: [uint32x4_t; 2] = [vandq_u32(t657[0], live[0]), vandq_u32(t657[1], live[1])];
        x152 = [vaddq_u32(x152[0], vandq_u32(t707[0], vdupq_n_u32(8))), vaddq_u32(x152[1], vandq_u32(t707[1], vdupq_n_u32(8)))];
    }
    x96 = x144;
    let t708 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    let t709 = [vdupq_n_u32((((t708).wrapping_shr((((((4u32).wrapping_mul(x16)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32))); 2];
    x152 = t709;
    let t710 = b17_ss_p(l, lid0, live, (x56).wrapping_add((2u32).wrapping_mul((x80).wrapping_sub(1u32))), x64);
    let t711 = b15_r0r1sss_p(l, lid0, live, arg0, arg1, x32, t710, 2u32);
    let t712: u64 = ((((t711) as u32)) as u64);
    let t713 = [vdupq_n_u32((t712) as u32); 2];
    let t714 = [vdupq_n_u32(((t712) >> 32) as u32); 2];
    let t715: [uint32x4_t; 4] = [t713[0], t713[1], t714[0], t714[1]];
    x160 = t715;
    let t716 = [vdupq_n_u32(x112); 2];
    let t717: [uint32x4_t; 2] = [x160[0], x160[1]];
    let t718: [uint32x4_t; 2] = [vaddq_u32(t716[0], t717[0]), vaddq_u32(t716[1], t717[1])];
    let t719: [uint32x4_t; 2] = [vmulq_u32(x96[0], x152[0]), vmulq_u32(x96[1], x152[1])];
    let t720: [uint32x4_t; 2] = [vaddq_u32(t718[0], t719[0]), vaddq_u32(t718[1], t719[1])];
    let t721 = b17_ls_p(l, lid0, live, t720, x64);
    let t722: [uint32x4_t; 4] = [t721[0], t721[1], vdupq_n_u32(0), vdupq_n_u32(0)];
    x168 = t722;
    let t723: [uint32x4_t; 2] = [x168[0], x168[1]];
    let t724 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t723, 8u32);
    let t725 = b8_l_p(l, lid0, live, x152);
    let t726: [uint32x4_t; 4] = [vandq_u32(t724[0], t725[0]), vandq_u32(t724[1], t725[1]), vandq_u32(t724[2], t725[2]), vandq_u32(t724[3], t725[3])];
    res = t726;
    return res;
}

#[inline(always)]
unsafe fn b10_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
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
    let mut x136: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x144: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x152: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x160: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x168: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x176: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x184: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t727 = b11_r0r1s_p(l, lid0, live, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x16)));
    x32 = t727;
    let t728 = b11_r0r1s_p(l, lid0, live, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x16)));
    x40 = t728;
    let t729: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    x48 = t729;
    let t730 = [vdupq_n_u32(4u32); 2];
    let t731: [uint32x4_t; 2] = [vmulq_u32(x48[0], t730[0]), vmulq_u32(x48[1], t730[1])];
    let t732 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x40, t731, 4u32);
    let t733: [uint32x4_t; 2] = [t732[0], t732[1]];
    x56 = t733;
    let t734: [uint32x4_t; 2] = [vmulq_u32(x48[0], t730[0]), vmulq_u32(x48[1], t730[1])];
    let t735: [uint32x4_t; 2] = [vaddq_u32(t734[0], t730[0]), vaddq_u32(t734[1], t730[1])];
    let t736 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x40, t735, 4u32);
    let t737: [uint32x4_t; 2] = [t736[0], t736[1]];
    x64 = t737;
    let t738 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    x72 = ((t738).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
    let t739: [uint32x4_t; 2] = [vmulq_u32(x48[0], t730[0]), vmulq_u32(x48[1], t730[1])];
    let t740 = [vdupq_n_u32(x72); 2];
    let t741: [uint32x4_t; 2] = [vsubq_u32(t740[0], t739[0]), vsubq_u32(t740[1], t739[1])];
    let t742 = b16_sl_p(l, lid0, live, 4u32, t741);
    x80 = t742;
    let t743: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
    let t744 = [vdupq_n_u32(3u32); 2];
    let t745: [uint32x4_t; 2] = [vandq_u32(t743[0], t744[0]), vandq_u32(t743[1], t744[1])];
    x88 = t745;
    let t746 = [vdupq_n_u32(0u32); 2];
    let t747: [uint32x4_t; 2] = [vceqq_u32(x88[0], t746[0]), vceqq_u32(x88[1], t746[1])];
    let mut t748: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t749: [uint32x4_t; 2] = [vandq_u32(live[0], t747[0]), vandq_u32(live[1], t747[1])];
    let t750: [uint32x4_t; 2] = [vbicq_u32(live[0], t747[0]), vbicq_u32(live[1], t747[1])];
    {
        t748 = [vbslq_u32(t749[0], t746[0], t748[0]), vbslq_u32(t749[1], t746[1], t748[1])];
    }
    if any(&t750) {
        let t751 = [vdupq_n_u32(1u32); 2];
        let t752: [uint32x4_t; 2] = [vsubq_u32(x88[0], t751[0]), vsubq_u32(x88[1], t751[1])];
        let t753 = [vdupq_n_u32(2u32); 2];
        let t754: [uint32x4_t; 2] = [vmulq_u32(t753[0], t752[0]), vmulq_u32(t753[1], t752[1])];
        let t755: [uint32x4_t; 2] = [vaddq_u32(x56[0], t754[0]), vaddq_u32(x56[1], t754[1])];
        let t756 = b17_ll_p(l, lid0, t750, t755, x64);
        let t757 = b15_r0r1sls_p(l, lid0, t750, arg0, arg1, x32, t756, 2u32);
        let t758: [uint32x4_t; 2] = [t757[0], t757[1]];
        t748 = [vbslq_u32(t750[0], t758[0], t748[0]), vbslq_u32(t750[1], t758[1], t748[1])];
    }
    x96 = t748;
    let t759 = [vdupq_n_u32(2u32); 2];
    let t760: [uint32x4_t; 2] = [vmulq_u32(t759[0], x88[0]), vmulq_u32(t759[1], x88[1])];
    let t761: [uint32x4_t; 2] = [vaddq_u32(x56[0], t760[0]), vaddq_u32(x56[1], t760[1])];
    let t762 = b17_ll_p(l, lid0, live, t761, x64);
    let t763 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t762, 2u32);
    let t764: [uint32x4_t; 2] = [t763[0], t763[1]];
    x104 = t764;
    let t765: [uint32x4_t; 2] = [vmulq_u32(t759[0], x80[0]), vmulq_u32(t759[1], x80[1])];
    let t766: [uint32x4_t; 2] = [vaddq_u32(x56[0], t765[0]), vaddq_u32(x56[1], t765[1])];
    x112 = t766;
    let t767 = [vdupq_n_u32(255u32); 2];
    let t768: [uint32x4_t; 2] = [vandq_u32(x24[0], t767[0]), vandq_u32(x24[1], t767[1])];
    x120 = t768;
    x128 = 9259542123273814144u64;
    let t769: u64 = 72340172838076673u64;
    let t770 = [vdupq_n_u32((t769) as u32); 2];
    let t771 = [vdupq_n_u32(((t769) >> 32) as u32); 2];
    let t772: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x120[0]), vget_low_u32(t770[0]))), vreinterpretq_u32_u64(vmull_high_u32(x120[0], t770[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x120[1]), vget_low_u32(t770[1]))), vreinterpretq_u32_u64(vmull_high_u32(x120[1], t770[1])))];
    let t773: [uint32x4_t; 4] = [vmulq_u32(x120[0], t770[0]), vmulq_u32(x120[1], t770[1]), vmlaq_u32(t772[0], x120[0], t771[0]), vmlaq_u32(t772[1], x120[1], t771[1])];
    x136 = t773;
    x144 = x96;
    x152 = x96;
    let mut t774 = live;
    loop {
        // Lanes part: each its own rounds.
        let t776: [uint32x4_t; 2] = [vcltq_u32(x152[0], x104[0]), vcltq_u32(x152[1], x104[1])];
        t774 = [vandq_u32(t774[0], t776[0]), vandq_u32(t774[1], t776[1])];
        if !(any(&t774)) { break; }
        let t778: [uint32x4_t; 2] = [vaddq_u32(x112[0], x152[0]), vaddq_u32(x112[1], x152[1])];
        let t779 = b17_ll_p(l, lid0, t774, t778, x64);
        let t780 = b15_r0r1sls_p(l, lid0, t774, arg0, arg1, x32, t779, 8u32);
        x160 = [vbslq_u32(t774[0], t780[0], x160[0]), vbslq_u32(t774[1], t780[1], x160[1]), vbslq_u32(t774[0], t780[2], x160[2]), vbslq_u32(t774[1], t780[3], x160[3])];
        let t781: u64 = x128;
        let t782 = [vdupq_n_u32((t781) as u32); 2];
        let t783 = [vdupq_n_u32(((t781) >> 32) as u32); 2];
        let t784: [uint32x4_t; 4] = [vorrq_u32(x160[0], t782[0]), vorrq_u32(x160[1], t782[1]), vorrq_u32(x160[2], t783[0]), vorrq_u32(x160[3], t783[1])];
        let t785: u64 = !(x128);
        let t786 = [vdupq_n_u32((t785) as u32); 2];
        let t787 = [vdupq_n_u32(((t785) >> 32) as u32); 2];
        let t788: [uint32x4_t; 4] = [vandq_u32(x136[0], t786[0]), vandq_u32(x136[1], t786[1]), vandq_u32(x136[2], t787[0]), vandq_u32(x136[3], t787[1])];
        let t789: [uint32x4_t; 2] = [vcltq_u32(t784[0], t788[0]), vcltq_u32(t784[1], t788[1])];
        let t790: [uint32x4_t; 4] = [vsubq_u32(t784[0], t788[0]), vsubq_u32(t784[1], t788[1]), vaddq_u32(vsubq_u32(t784[2], t788[2]), t789[0]), vaddq_u32(vsubq_u32(t784[3], t788[3]), t789[1])];
        x168 = [vbslq_u32(t774[0], t790[0], x168[0]), vbslq_u32(t774[1], t790[1], x168[1]), vbslq_u32(t774[0], t790[2], x168[2]), vbslq_u32(t774[1], t790[3], x168[3])];
        let t791: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x160[r]));
        let t792: [uint32x4_t; 4] = [vandq_u32(t791[0], x136[0]), vandq_u32(t791[1], x136[1]), vandq_u32(t791[2], x136[2]), vandq_u32(t791[3], x136[3])];
        let t793: [uint32x4_t; 4] = [veorq_u32(x160[0], x136[0]), veorq_u32(x160[1], x136[1]), veorq_u32(x160[2], x136[2]), veorq_u32(x160[3], x136[3])];
        let t794: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(t793[r]));
        let t795: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x168[r]));
        let t796: [uint32x4_t; 4] = [vandq_u32(t794[0], t795[0]), vandq_u32(t794[1], t795[1]), vandq_u32(t794[2], t795[2]), vandq_u32(t794[3], t795[3])];
        let t797: [uint32x4_t; 4] = [vorrq_u32(t792[0], t796[0]), vorrq_u32(t792[1], t796[1]), vorrq_u32(t792[2], t796[2]), vorrq_u32(t792[3], t796[3])];
        let t798: u64 = x128;
        let t799 = [vdupq_n_u32((t798) as u32); 2];
        let t800 = [vdupq_n_u32(((t798) >> 32) as u32); 2];
        let t801: [uint32x4_t; 4] = [vandq_u32(t799[0], t797[0]), vandq_u32(t799[1], t797[1]), vandq_u32(t800[0], t797[2]), vandq_u32(t800[1], t797[3])];
        x176 = t801;
        let t802: [uint32x4_t; 2] = [vsubq_u32(x104[0], x152[0]), vsubq_u32(x104[1], x152[1])];
        x184 = t802;
        let t803 = [vdupq_n_u32(8u32); 2];
        let t804: [uint32x4_t; 2] = [vcgeq_u32(x184[0], t803[0]), vcgeq_u32(x184[1], t803[1])];
        let mut t805: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t806: [uint32x4_t; 2] = [vandq_u32(t774[0], t804[0]), vandq_u32(t774[1], t804[1])];
        let t807: [uint32x4_t; 2] = [vbicq_u32(t774[0], t804[0]), vbicq_u32(t774[1], t804[1])];
        {
            t805 = [vbslq_u32(t806[0], x176[0], t805[0]), vbslq_u32(t806[1], x176[1], t805[1]), vbslq_u32(t806[0], x176[2], t805[2]), vbslq_u32(t806[1], x176[3], t805[3])];
        }
        {
            let t808: [uint32x4_t; 2] = [vmulq_u32(t803[0], x184[0]), vmulq_u32(t803[1], x184[1])];
            let t809: u64 = 1u64;
            let t810 = [vdupq_n_u32((t809) as u32); 2];
            let t811: [uint32x4_t; 2] = [t808[0], t808[1]];
            let t812: [uint32x4_t; 2] = [vandq_u32(t811[0], vdupq_n_u32(63)), vandq_u32(t811[1], vdupq_n_u32(63))];
            let t813: [uint32x4_t; 4] = [vshlq_u32(t810[0], vreinterpretq_s32_u32(t812[0])), vshlq_u32(t810[1], vreinterpretq_s32_u32(t812[1])), vshlq_u32(t810[0], vsubq_s32(vreinterpretq_s32_u32(t812[0]), vdupq_n_s32(32))), vshlq_u32(t810[1], vsubq_s32(vreinterpretq_s32_u32(t812[1]), vdupq_n_s32(32)))];
            let t814: u64 = 1u64;
            let t815 = [vdupq_n_u32((t814) as u32); 2];
            let t816: [uint32x4_t; 2] = [vcltq_u32(t813[0], t815[0]), vcltq_u32(t813[1], t815[1])];
            let t817: [uint32x4_t; 4] = [vsubq_u32(t813[0], t815[0]), vsubq_u32(t813[1], t815[1]), vaddq_u32(vsubq_u32(t813[2], vdupq_n_u32(0)), t816[0]), vaddq_u32(vsubq_u32(t813[3], vdupq_n_u32(0)), t816[1])];
            let t818: [uint32x4_t; 4] = [vandq_u32(x176[0], t817[0]), vandq_u32(x176[1], t817[1]), vandq_u32(x176[2], t817[2]), vandq_u32(x176[3], t817[3])];
            t805 = [vbslq_u32(t807[0], t818[0], t805[0]), vbslq_u32(t807[1], t818[1], t805[1]), vbslq_u32(t807[0], t818[2], t805[2]), vbslq_u32(t807[1], t818[3], t805[3])];
        }
        let t819 = b18_l_p(l, lid0, t774, t805);
        let t820: [uint32x4_t; 2] = [vaddq_u32(x144[0], t819[0]), vaddq_u32(x144[1], t819[1])];
        x144 = [vbslq_u32(t774[0], t820[0], x144[0]), vbslq_u32(t774[1], t820[1], x144[1])];
        let t821: [uint32x4_t; 2] = [vandq_u32(t774[0], live[0]), vandq_u32(t774[1], live[1])];
        x152 = [vaddq_u32(x152[0], vandq_u32(t821[0], vdupq_n_u32(8))), vaddq_u32(x152[1], vandq_u32(t821[1], vdupq_n_u32(8)))];
    }
    x96 = x144;
    let t822 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    let t823 = [vdupq_n_u32((((t822).wrapping_shr((((((4u32).wrapping_mul(x16)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32))); 2];
    x152 = t823;
    let t824 = [vdupq_n_u32(1u32); 2];
    let t825: [uint32x4_t; 2] = [vsubq_u32(x80[0], t824[0]), vsubq_u32(x80[1], t824[1])];
    let t826: [uint32x4_t; 2] = [vmulq_u32(t759[0], t825[0]), vmulq_u32(t759[1], t825[1])];
    let t827: [uint32x4_t; 2] = [vaddq_u32(x56[0], t826[0]), vaddq_u32(x56[1], t826[1])];
    let t828 = b17_ll_p(l, lid0, live, t827, x64);
    let t829 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t828, 2u32);
    let t830: [uint32x4_t; 2] = [t829[0], t829[1]];
    let t831: [uint32x4_t; 4] = [t830[0], t830[1], vdupq_n_u32(0), vdupq_n_u32(0)];
    x160 = t831;
    let t832: [uint32x4_t; 2] = [x160[0], x160[1]];
    let t833: [uint32x4_t; 2] = [vaddq_u32(x112[0], t832[0]), vaddq_u32(x112[1], t832[1])];
    let t834: [uint32x4_t; 2] = [vmulq_u32(x96[0], x152[0]), vmulq_u32(x96[1], x152[1])];
    let t835: [uint32x4_t; 2] = [vaddq_u32(t833[0], t834[0]), vaddq_u32(t833[1], t834[1])];
    let t836 = b17_ll_p(l, lid0, live, t835, x64);
    let t837: [uint32x4_t; 4] = [t836[0], t836[1], vdupq_n_u32(0), vdupq_n_u32(0)];
    x168 = t837;
    let t838: [uint32x4_t; 2] = [x168[0], x168[1]];
    let t839 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t838, 8u32);
    let t840 = b8_l_p(l, lid0, live, x152);
    let t841: [uint32x4_t; 4] = [vandq_u32(t839[0], t840[0]), vandq_u32(t839[1], t840[1]), vandq_u32(t839[2], t840[2]), vandq_u32(t839[3], t840[3])];
    res = t841;
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
    let t842 = b11_r0r1s_p(l, lid0, live, arg0, arg1, (20u32).wrapping_add((4u32).wrapping_mul(x16)));
    res = t842;
    return res;
}

#[inline(always)]
unsafe fn b12_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t843 = b11_r0r1s_f(l, lid0, arg0, arg1, (20u32).wrapping_add((4u32).wrapping_mul(x16)));
    res = t843;
    return res;
}

#[inline(always)]
unsafe fn b13_ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x8: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t844: u32;
    if ((x0) < (x8)) {
        t844 = x0;
    } else {
        t844 = x8;
    }
    res = t844;
    return res;
}

#[inline(always)]
unsafe fn b13_ss_f(l: &Launch, lid0: u32, x0: u32, x8: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t845: u32;
    if ((x0) < (x8)) {
        t845 = x0;
    } else {
        t845 = x8;
    }
    res = t845;
    return res;
}

#[inline(always)]
unsafe fn b13_ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x8: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t846 = [vdupq_n_u32(x8); 2];
    let t847: [uint32x4_t; 2] = [vcltq_u32(x0[0], t846[0]), vcltq_u32(x0[1], t846[1])];
    let mut t848: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t849: [uint32x4_t; 2] = [vandq_u32(live[0], t847[0]), vandq_u32(live[1], t847[1])];
    let t850: [uint32x4_t; 2] = [vbicq_u32(live[0], t847[0]), vbicq_u32(live[1], t847[1])];
    {
        t848 = [vbslq_u32(t849[0], x0[0], t848[0]), vbslq_u32(t849[1], x0[1], t848[1])];
    }
    {
        let t851 = [vdupq_n_u32(x8); 2];
        t848 = [vbslq_u32(t850[0], t851[0], t848[0]), vbslq_u32(t850[1], t851[1], t848[1])];
    }
    res = t848;
    return res;
}

#[inline(always)]
unsafe fn b13_ls_f(l: &Launch, lid0: u32, x0: [uint32x4_t; 2], x8: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t852 = [vdupq_n_u32(x8); 2];
    let t853: [uint32x4_t; 2] = [vcltq_u32(x0[0], t852[0]), vcltq_u32(x0[1], t852[1])];
    let mut t854: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t855: [uint32x4_t; 2] = [vmvnq_u32(t853[0]), vmvnq_u32(t853[1])];
    {
        t854 = [vbslq_u32(t853[0], x0[0], t854[0]), vbslq_u32(t853[1], x0[1], t854[1])];
    }
    {
        let t856 = [vdupq_n_u32(x8); 2];
        t854 = [vbslq_u32(t855[0], t856[0], t854[0]), vbslq_u32(t855[1], t856[1], t854[1])];
    }
    res = t854;
    return res;
}

#[inline(always)]
unsafe fn b14_r0sns_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: u32, x16f: u32, x16s: u32, x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t857 = b19_r0an1s_p(l, lid0, live, arg0, ((((x8) as u64)) as usize as *const u8), x16f, x16s, x24);
    x32 = t857;
    let t858: [uint32x4_t; 4];
    if ((x24) >= (8u32)) {
        t858 = x32;
    } else {
        let t859 = b20_s_p(l, lid0, live, (8u32).wrapping_mul(x24));
        let t860: u64 = t859;
        let t861 = [vdupq_n_u32((t860) as u32); 2];
        let t862 = [vdupq_n_u32(((t860) >> 32) as u32); 2];
        let t863: [uint32x4_t; 4] = [vandq_u32(x32[0], t861[0]), vandq_u32(x32[1], t861[1]), vandq_u32(x32[2], t862[0]), vandq_u32(x32[3], t862[1])];
        t858 = t863;
    }
    res = t858;
    return res;
}

#[inline(always)]
unsafe fn b14_r0sns_f(l: &Launch, lid0: u32, arg0: *const u8, x8: u32, x16f: u32, x16s: u32, x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t864 = b19_r0an1s_f(l, lid0, arg0, ((((x8) as u64)) as usize as *const u8), x16f, x16s, x24);
    x32 = t864;
    let t865: [uint32x4_t; 4];
    if ((x24) >= (8u32)) {
        t865 = x32;
    } else {
        let t866 = b20_s_f(l, lid0, (8u32).wrapping_mul(x24));
        let t867: u64 = t866;
        let t868 = [vdupq_n_u32((t867) as u32); 2];
        let t869 = [vdupq_n_u32(((t867) >> 32) as u32); 2];
        let t870: [uint32x4_t; 4] = [vandq_u32(x32[0], t868[0]), vandq_u32(x32[1], t868[1]), vandq_u32(x32[2], t869[0]), vandq_u32(x32[3], t869[1])];
        t865 = t870;
    }
    res = t865;
    return res;
}

#[inline(always)]
unsafe fn b14_r0lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: [uint32x4_t; 2], x16: [uint32x4_t; 2], x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t871: [uint32x4_t; 2] = [vaddq_u32(x8[0], x16[0]), vaddq_u32(x8[1], x16[1])];
    let t872: [uint32x4_t; 2] = [vcltq_u32(t871[0], x8[0]), vcltq_u32(t871[1], x8[1])];
    let t873: [uint32x4_t; 4] = [t871[0], t871[1], vshrq_n_u32::<31>(t872[0]), vshrq_n_u32::<31>(t872[1])];
    let t874 = b19_r0ls_p(l, lid0, live, arg0, t873, x24);
    x32 = t874;
    let t875: [uint32x4_t; 4];
    if ((x24) >= (8u32)) {
        t875 = x32;
    } else {
        let t876 = b20_s_p(l, lid0, live, (8u32).wrapping_mul(x24));
        let t877: u64 = t876;
        let t878 = [vdupq_n_u32((t877) as u32); 2];
        let t879 = [vdupq_n_u32(((t877) >> 32) as u32); 2];
        let t880: [uint32x4_t; 4] = [vandq_u32(x32[0], t878[0]), vandq_u32(x32[1], t878[1]), vandq_u32(x32[2], t879[0]), vandq_u32(x32[3], t879[1])];
        t875 = t880;
    }
    res = t875;
    return res;
}

#[inline(always)]
unsafe fn b14_r0lls_f(l: &Launch, lid0: u32, arg0: *const u8, x8: [uint32x4_t; 2], x16: [uint32x4_t; 2], x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t881: [uint32x4_t; 2] = [vaddq_u32(x8[0], x16[0]), vaddq_u32(x8[1], x16[1])];
    let t882: [uint32x4_t; 2] = [vcltq_u32(t881[0], x8[0]), vcltq_u32(t881[1], x8[1])];
    let t883: [uint32x4_t; 4] = [t881[0], t881[1], vshrq_n_u32::<31>(t882[0]), vshrq_n_u32::<31>(t882[1])];
    let t884 = b19_r0ls_f(l, lid0, arg0, t883, x24);
    x32 = t884;
    let t885: [uint32x4_t; 4];
    if ((x24) >= (8u32)) {
        t885 = x32;
    } else {
        let t886 = b20_s_f(l, lid0, (8u32).wrapping_mul(x24));
        let t887: u64 = t886;
        let t888 = [vdupq_n_u32((t887) as u32); 2];
        let t889 = [vdupq_n_u32(((t887) >> 32) as u32); 2];
        let t890: [uint32x4_t; 4] = [vandq_u32(x32[0], t888[0]), vandq_u32(x32[1], t888[1]), vandq_u32(x32[2], t889[0]), vandq_u32(x32[3], t889[1])];
        t885 = t890;
    }
    res = t885;
    return res;
}

#[inline(always)]
unsafe fn b15_r0r1sss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: u32, x32: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t891 = b21_r0sss_p(l, lid0, live, arg0, x16, x24, x32);
    res = t891;
    return res;
}

#[inline(always)]
unsafe fn b15_r0r1sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2], x32: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t892 = b21_r0sls_p(l, lid0, live, arg0, x16, x24, x32);
    res = t892;
    return res;
}

#[inline(always)]
unsafe fn b16_ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x8: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t893: u32;
    if ((x0) < (x8)) {
        t893 = x0;
    } else {
        t893 = x8;
    }
    res = t893;
    return res;
}

#[inline(always)]
unsafe fn b16_sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x8: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t894 = [vdupq_n_u32(x0); 2];
    let t895: [uint32x4_t; 2] = [vcltq_u32(t894[0], x8[0]), vcltq_u32(t894[1], x8[1])];
    let mut t896: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t897: [uint32x4_t; 2] = [vandq_u32(live[0], t895[0]), vandq_u32(live[1], t895[1])];
    let t898: [uint32x4_t; 2] = [vbicq_u32(live[0], t895[0]), vbicq_u32(live[1], t895[1])];
    {
        let t899 = [vdupq_n_u32(x0); 2];
        t896 = [vbslq_u32(t897[0], t899[0], t896[0]), vbslq_u32(t897[1], t899[1], t896[1])];
    }
    {
        t896 = [vbslq_u32(t898[0], x8[0], t896[0]), vbslq_u32(t898[1], x8[1], t896[1])];
    }
    res = t896;
    return res;
}

#[inline(always)]
unsafe fn b17_ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x8: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t900: u32;
    if ((x0) < (x8)) {
        t900 = x0;
    } else {
        t900 = x8;
    }
    res = t900;
    return res;
}

#[inline(always)]
unsafe fn b17_ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x8: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t901 = [vdupq_n_u32(x8); 2];
    let t902: [uint32x4_t; 2] = [vcltq_u32(x0[0], t901[0]), vcltq_u32(x0[1], t901[1])];
    let mut t903: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t904: [uint32x4_t; 2] = [vandq_u32(live[0], t902[0]), vandq_u32(live[1], t902[1])];
    let t905: [uint32x4_t; 2] = [vbicq_u32(live[0], t902[0]), vbicq_u32(live[1], t902[1])];
    {
        t903 = [vbslq_u32(t904[0], x0[0], t903[0]), vbslq_u32(t904[1], x0[1], t903[1])];
    }
    {
        let t906 = [vdupq_n_u32(x8); 2];
        t903 = [vbslq_u32(t905[0], t906[0], t903[0]), vbslq_u32(t905[1], t906[1], t903[1])];
    }
    res = t903;
    return res;
}

#[inline(always)]
unsafe fn b17_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x8: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t907: [uint32x4_t; 2] = [vcltq_u32(x0[0], x8[0]), vcltq_u32(x0[1], x8[1])];
    let mut t908: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t909: [uint32x4_t; 2] = [vandq_u32(live[0], t907[0]), vandq_u32(live[1], t907[1])];
    let t910: [uint32x4_t; 2] = [vbicq_u32(live[0], t907[0]), vbicq_u32(live[1], t907[1])];
    {
        t908 = [vbslq_u32(t909[0], x0[0], t908[0]), vbslq_u32(t909[1], x0[1], t908[1])];
    }
    {
        t908 = [vbslq_u32(t910[0], x8[0], t908[0]), vbslq_u32(t910[1], x8[1], t908[1])];
    }
    res = t908;
    return res;
}

#[inline(always)]
unsafe fn b18_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t911: [uint32x4_t; 2] = [vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[0])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[2]))))), vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[1])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[3])))))];
    res = t911;
    return res;
}

#[inline(always)]
unsafe fn b19_r0ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: u64, x16: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: *const u8 = core::ptr::null();
    let mut x32: u32 = 0;
    let mut x40: u64 = 0;
    let mut live = k;
    let mut res: u64 = 0;
    if ((x16) == (8u32)) {
        res = (arg0.wrapping_add(x8 as usize) as *const u64).read_unaligned();
        return res;
    }
    x24 = arg0.wrapping_add(((x8) & (!(7u64))) as usize);
    x32 = (((((x8) & (7u64))) as u32)).wrapping_mul(8u32);
    x40 = ((x24 as *const u64).read_unaligned()).wrapping_shr((((x32) as u64)) as u32);
    let mut t912: bool = ((x32) != (0u32));
    if t912 {
        t912 = (((x32).wrapping_add((x16).wrapping_mul(8u32))) > (64u32));
    }
    let t913: u64;
    if t912 {
        t913 = ((x40) | (((x24.wrapping_add((1u64).wrapping_mul(8u64) as usize) as *const u64).read_unaligned()).wrapping_shl(((((64u32).wrapping_sub(x32)) as u64)) as u32)));
    } else {
        t913 = x40;
    }
    res = t913;
    return res;
}

#[inline(always)]
unsafe fn b19_r0ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: [uint32x4_t; 4], x16: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if ((x16) == (8u32)) {
        let t914: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
        let t915: [uint32x4_t; 4] = [vandq_u32(x8[0], t914[0]), vandq_u32(x8[1], t914[1]), vandq_u32(x8[2], t914[2]), vandq_u32(x8[3], t914[3])];
        let t916: u64 = ((arg0) as usize as u64);
        let t917 = [vdupq_n_u32((t916) as u32); 2];
        let t918 = [vdupq_n_u32(((t916) >> 32) as u32); 2];
        let t919: [uint32x4_t; 2] = [vaddq_u32(t917[0], t915[0]), vaddq_u32(t917[1], t915[1])];
        let t920: [uint32x4_t; 2] = [vcltq_u32(t919[0], t917[0]), vcltq_u32(t919[1], t917[1])];
        let t921: [uint32x4_t; 4] = [t919[0], t919[1], vsubq_u32(vaddq_u32(t918[0], t915[2]), t920[0]), vsubq_u32(vaddq_u32(t918[1], t915[3]), t920[1])];
        let mut t922: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t921[2]) as u64) << 32 | vgetq_lane_u32::<0>(t921[0]) as u64) as usize as *const u64).read_unaligned(); t922[0] = vsetq_lane_u32::<0>(v as u32, t922[0]); t922[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t922[2]); }
        { let v = (((vgetq_lane_u32::<1>(t921[2]) as u64) << 32 | vgetq_lane_u32::<1>(t921[0]) as u64) as usize as *const u64).read_unaligned(); t922[0] = vsetq_lane_u32::<1>(v as u32, t922[0]); t922[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t922[2]); }
        { let v = (((vgetq_lane_u32::<2>(t921[2]) as u64) << 32 | vgetq_lane_u32::<2>(t921[0]) as u64) as usize as *const u64).read_unaligned(); t922[0] = vsetq_lane_u32::<2>(v as u32, t922[0]); t922[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t922[2]); }
        { let v = (((vgetq_lane_u32::<3>(t921[2]) as u64) << 32 | vgetq_lane_u32::<3>(t921[0]) as u64) as usize as *const u64).read_unaligned(); t922[0] = vsetq_lane_u32::<3>(v as u32, t922[0]); t922[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t922[2]); }
        { let v = (((vgetq_lane_u32::<0>(t921[3]) as u64) << 32 | vgetq_lane_u32::<0>(t921[1]) as u64) as usize as *const u64).read_unaligned(); t922[1] = vsetq_lane_u32::<0>(v as u32, t922[1]); t922[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t922[3]); }
        { let v = (((vgetq_lane_u32::<1>(t921[3]) as u64) << 32 | vgetq_lane_u32::<1>(t921[1]) as u64) as usize as *const u64).read_unaligned(); t922[1] = vsetq_lane_u32::<1>(v as u32, t922[1]); t922[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t922[3]); }
        { let v = (((vgetq_lane_u32::<2>(t921[3]) as u64) << 32 | vgetq_lane_u32::<2>(t921[1]) as u64) as usize as *const u64).read_unaligned(); t922[1] = vsetq_lane_u32::<2>(v as u32, t922[1]); t922[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t922[3]); }
        { let v = (((vgetq_lane_u32::<3>(t921[3]) as u64) << 32 | vgetq_lane_u32::<3>(t921[1]) as u64) as usize as *const u64).read_unaligned(); t922[1] = vsetq_lane_u32::<3>(v as u32, t922[1]); t922[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t922[3]); }
        res = t922;
        return res;
    }
    let t923: u64 = !(7u64);
    let t924 = [vdupq_n_u32((t923) as u32); 2];
    let t925 = [vdupq_n_u32(((t923) >> 32) as u32); 2];
    let t926: [uint32x4_t; 4] = [vandq_u32(x8[0], t924[0]), vandq_u32(x8[1], t924[1]), vandq_u32(x8[2], t925[0]), vandq_u32(x8[3], t925[1])];
    let t927: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
    let t928: [uint32x4_t; 4] = [vandq_u32(t926[0], t927[0]), vandq_u32(t926[1], t927[1]), vandq_u32(t926[2], t927[2]), vandq_u32(t926[3], t927[3])];
    let t929: u64 = ((arg0) as usize as u64);
    let t930 = [vdupq_n_u32((t929) as u32); 2];
    let t931 = [vdupq_n_u32(((t929) >> 32) as u32); 2];
    let t932: [uint32x4_t; 2] = [vaddq_u32(t930[0], t928[0]), vaddq_u32(t930[1], t928[1])];
    let t933: [uint32x4_t; 2] = [vcltq_u32(t932[0], t930[0]), vcltq_u32(t932[1], t930[1])];
    let t934: [uint32x4_t; 4] = [t932[0], t932[1], vsubq_u32(vaddq_u32(t931[0], t928[2]), t933[0]), vsubq_u32(vaddq_u32(t931[1], t928[3]), t933[1])];
    x24 = t934;
    let t935: u64 = 7u64;
    let t936 = [vdupq_n_u32((t935) as u32); 2];
    let t937: [uint32x4_t; 2] = [vandq_u32(x8[0], t936[0]), vandq_u32(x8[1], t936[1])];
    let t938 = [vdupq_n_u32(8u32); 2];
    let t939: [uint32x4_t; 2] = [vmulq_u32(t937[0], t938[0]), vmulq_u32(t937[1], t938[1])];
    x32 = t939;
    let mut t940: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x24[2]) as u64) << 32 | vgetq_lane_u32::<0>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t940[0] = vsetq_lane_u32::<0>(v as u32, t940[0]); t940[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t940[2]); }
    { let v = (((vgetq_lane_u32::<1>(x24[2]) as u64) << 32 | vgetq_lane_u32::<1>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t940[0] = vsetq_lane_u32::<1>(v as u32, t940[0]); t940[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t940[2]); }
    { let v = (((vgetq_lane_u32::<2>(x24[2]) as u64) << 32 | vgetq_lane_u32::<2>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t940[0] = vsetq_lane_u32::<2>(v as u32, t940[0]); t940[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t940[2]); }
    { let v = (((vgetq_lane_u32::<3>(x24[2]) as u64) << 32 | vgetq_lane_u32::<3>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t940[0] = vsetq_lane_u32::<3>(v as u32, t940[0]); t940[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t940[2]); }
    { let v = (((vgetq_lane_u32::<0>(x24[3]) as u64) << 32 | vgetq_lane_u32::<0>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t940[1] = vsetq_lane_u32::<0>(v as u32, t940[1]); t940[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t940[3]); }
    { let v = (((vgetq_lane_u32::<1>(x24[3]) as u64) << 32 | vgetq_lane_u32::<1>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t940[1] = vsetq_lane_u32::<1>(v as u32, t940[1]); t940[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t940[3]); }
    { let v = (((vgetq_lane_u32::<2>(x24[3]) as u64) << 32 | vgetq_lane_u32::<2>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t940[1] = vsetq_lane_u32::<2>(v as u32, t940[1]); t940[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t940[3]); }
    { let v = (((vgetq_lane_u32::<3>(x24[3]) as u64) << 32 | vgetq_lane_u32::<3>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t940[1] = vsetq_lane_u32::<3>(v as u32, t940[1]); t940[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t940[3]); }
    let t941: [uint32x4_t; 2] = [x32[0], x32[1]];
    let t942: [uint32x4_t; 2] = [vandq_u32(t941[0], vdupq_n_u32(63)), vandq_u32(t941[1], vdupq_n_u32(63))];
    let t943: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t940[0], vnegq_s32(vreinterpretq_s32_u32(t942[0]))), vshlq_u32(t940[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t942[0])))), vorrq_u32(vshlq_u32(t940[1], vnegq_s32(vreinterpretq_s32_u32(t942[1]))), vshlq_u32(t940[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t942[1])))), vshlq_u32(t940[2], vnegq_s32(vreinterpretq_s32_u32(t942[0]))), vshlq_u32(t940[3], vnegq_s32(vreinterpretq_s32_u32(t942[1])))];
    x40 = t943;
    let t944 = [vdupq_n_u32(0u32); 2];
    let t945: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t944[0])), vmvnq_u32(vceqq_u32(x32[1], t944[1]))];
    let t946 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t947: [uint32x4_t; 2] = [vaddq_u32(x32[0], t946[0]), vaddq_u32(x32[1], t946[1])];
    let t948 = [vdupq_n_u32(64u32); 2];
    let t949: [uint32x4_t; 2] = [vcgtq_u32(t947[0], t948[0]), vcgtq_u32(t947[1], t948[1])];
    let t950: [uint32x4_t; 2] = [vandq_u32(t945[0], t949[0]), vandq_u32(t945[1], t949[1])];
    let mut t951: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t952: [uint32x4_t; 2] = [vandq_u32(live[0], t950[0]), vandq_u32(live[1], t950[1])];
    let t953: [uint32x4_t; 2] = [vbicq_u32(live[0], t950[0]), vbicq_u32(live[1], t950[1])];
    if any(&t952) {
        let t954: u64 = (1u64).wrapping_mul(8u64);
        let t955 = [vdupq_n_u32((t954) as u32); 2];
        let t956 = [vdupq_n_u32(((t954) >> 32) as u32); 2];
        let t957: [uint32x4_t; 2] = [vaddq_u32(x24[0], t955[0]), vaddq_u32(x24[1], t955[1])];
        let t958: [uint32x4_t; 2] = [vcltq_u32(t957[0], x24[0]), vcltq_u32(t957[1], x24[1])];
        let t959: [uint32x4_t; 4] = [t957[0], t957[1], vsubq_u32(vaddq_u32(x24[2], t956[0]), t958[0]), vsubq_u32(vaddq_u32(x24[3], t956[1]), t958[1])];
        let mut t960: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t959[2]) as u64) << 32 | vgetq_lane_u32::<0>(t959[0]) as u64) as usize as *const u64).read_unaligned(); t960[0] = vsetq_lane_u32::<0>(v as u32, t960[0]); t960[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t960[2]); }
        { let v = (((vgetq_lane_u32::<1>(t959[2]) as u64) << 32 | vgetq_lane_u32::<1>(t959[0]) as u64) as usize as *const u64).read_unaligned(); t960[0] = vsetq_lane_u32::<1>(v as u32, t960[0]); t960[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t960[2]); }
        { let v = (((vgetq_lane_u32::<2>(t959[2]) as u64) << 32 | vgetq_lane_u32::<2>(t959[0]) as u64) as usize as *const u64).read_unaligned(); t960[0] = vsetq_lane_u32::<2>(v as u32, t960[0]); t960[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t960[2]); }
        { let v = (((vgetq_lane_u32::<3>(t959[2]) as u64) << 32 | vgetq_lane_u32::<3>(t959[0]) as u64) as usize as *const u64).read_unaligned(); t960[0] = vsetq_lane_u32::<3>(v as u32, t960[0]); t960[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t960[2]); }
        { let v = (((vgetq_lane_u32::<0>(t959[3]) as u64) << 32 | vgetq_lane_u32::<0>(t959[1]) as u64) as usize as *const u64).read_unaligned(); t960[1] = vsetq_lane_u32::<0>(v as u32, t960[1]); t960[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t960[3]); }
        { let v = (((vgetq_lane_u32::<1>(t959[3]) as u64) << 32 | vgetq_lane_u32::<1>(t959[1]) as u64) as usize as *const u64).read_unaligned(); t960[1] = vsetq_lane_u32::<1>(v as u32, t960[1]); t960[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t960[3]); }
        { let v = (((vgetq_lane_u32::<2>(t959[3]) as u64) << 32 | vgetq_lane_u32::<2>(t959[1]) as u64) as usize as *const u64).read_unaligned(); t960[1] = vsetq_lane_u32::<2>(v as u32, t960[1]); t960[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t960[3]); }
        { let v = (((vgetq_lane_u32::<3>(t959[3]) as u64) << 32 | vgetq_lane_u32::<3>(t959[1]) as u64) as usize as *const u64).read_unaligned(); t960[1] = vsetq_lane_u32::<3>(v as u32, t960[1]); t960[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t960[3]); }
        let t961: [uint32x4_t; 2] = [vsubq_u32(t948[0], x32[0]), vsubq_u32(t948[1], x32[1])];
        let t962: [uint32x4_t; 2] = [t961[0], t961[1]];
        let t963: [uint32x4_t; 2] = [vandq_u32(t962[0], vdupq_n_u32(63)), vandq_u32(t962[1], vdupq_n_u32(63))];
        let t964: [uint32x4_t; 4] = [vshlq_u32(t960[0], vreinterpretq_s32_u32(t963[0])), vshlq_u32(t960[1], vreinterpretq_s32_u32(t963[1])), vorrq_u32(vshlq_u32(t960[2], vreinterpretq_s32_u32(t963[0])), vshlq_u32(t960[0], vsubq_s32(vreinterpretq_s32_u32(t963[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t960[3], vreinterpretq_s32_u32(t963[1])), vshlq_u32(t960[1], vsubq_s32(vreinterpretq_s32_u32(t963[1]), vdupq_n_s32(32))))];
        let t965: [uint32x4_t; 4] = [vorrq_u32(x40[0], t964[0]), vorrq_u32(x40[1], t964[1]), vorrq_u32(x40[2], t964[2]), vorrq_u32(x40[3], t964[3])];
        t951 = [vbslq_u32(t952[0], t965[0], t951[0]), vbslq_u32(t952[1], t965[1], t951[1]), vbslq_u32(t952[0], t965[2], t951[2]), vbslq_u32(t952[1], t965[3], t951[3])];
    }
    {
        t951 = [vbslq_u32(t953[0], x40[0], t951[0]), vbslq_u32(t953[1], x40[1], t951[1]), vbslq_u32(t953[0], x40[2], t951[2]), vbslq_u32(t953[1], x40[3], t951[3])];
    }
    res = t951;
    return res;
}

#[inline(always)]
unsafe fn b19_r0ls_f(l: &Launch, lid0: u32, arg0: *const u8, x8: [uint32x4_t; 4], x16: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if ((x16) == (8u32)) {
        let t966: u64 = ((arg0) as usize as u64);
        let t967 = [vdupq_n_u32((t966) as u32); 2];
        let t968 = [vdupq_n_u32(((t966) >> 32) as u32); 2];
        let t969: [uint32x4_t; 2] = [vaddq_u32(t967[0], x8[0]), vaddq_u32(t967[1], x8[1])];
        let t970: [uint32x4_t; 2] = [vcltq_u32(t969[0], t967[0]), vcltq_u32(t969[1], t967[1])];
        let t971: [uint32x4_t; 4] = [t969[0], t969[1], vsubq_u32(vaddq_u32(t968[0], x8[2]), t970[0]), vsubq_u32(vaddq_u32(t968[1], x8[3]), t970[1])];
        let mut t972: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t971[2]) as u64) << 32 | vgetq_lane_u32::<0>(t971[0]) as u64) as usize as *const u64).read_unaligned(); t972[0] = vsetq_lane_u32::<0>(v as u32, t972[0]); t972[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t972[2]); }
        { let v = (((vgetq_lane_u32::<1>(t971[2]) as u64) << 32 | vgetq_lane_u32::<1>(t971[0]) as u64) as usize as *const u64).read_unaligned(); t972[0] = vsetq_lane_u32::<1>(v as u32, t972[0]); t972[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t972[2]); }
        { let v = (((vgetq_lane_u32::<2>(t971[2]) as u64) << 32 | vgetq_lane_u32::<2>(t971[0]) as u64) as usize as *const u64).read_unaligned(); t972[0] = vsetq_lane_u32::<2>(v as u32, t972[0]); t972[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t972[2]); }
        { let v = (((vgetq_lane_u32::<3>(t971[2]) as u64) << 32 | vgetq_lane_u32::<3>(t971[0]) as u64) as usize as *const u64).read_unaligned(); t972[0] = vsetq_lane_u32::<3>(v as u32, t972[0]); t972[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t972[2]); }
        { let v = (((vgetq_lane_u32::<0>(t971[3]) as u64) << 32 | vgetq_lane_u32::<0>(t971[1]) as u64) as usize as *const u64).read_unaligned(); t972[1] = vsetq_lane_u32::<0>(v as u32, t972[1]); t972[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t972[3]); }
        { let v = (((vgetq_lane_u32::<1>(t971[3]) as u64) << 32 | vgetq_lane_u32::<1>(t971[1]) as u64) as usize as *const u64).read_unaligned(); t972[1] = vsetq_lane_u32::<1>(v as u32, t972[1]); t972[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t972[3]); }
        { let v = (((vgetq_lane_u32::<2>(t971[3]) as u64) << 32 | vgetq_lane_u32::<2>(t971[1]) as u64) as usize as *const u64).read_unaligned(); t972[1] = vsetq_lane_u32::<2>(v as u32, t972[1]); t972[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t972[3]); }
        { let v = (((vgetq_lane_u32::<3>(t971[3]) as u64) << 32 | vgetq_lane_u32::<3>(t971[1]) as u64) as usize as *const u64).read_unaligned(); t972[1] = vsetq_lane_u32::<3>(v as u32, t972[1]); t972[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t972[3]); }
        res = t972;
        return res;
    }
    let t973: u64 = !(7u64);
    let t974 = [vdupq_n_u32((t973) as u32); 2];
    let t975 = [vdupq_n_u32(((t973) >> 32) as u32); 2];
    let t976: [uint32x4_t; 4] = [vandq_u32(x8[0], t974[0]), vandq_u32(x8[1], t974[1]), vandq_u32(x8[2], t975[0]), vandq_u32(x8[3], t975[1])];
    let t977: u64 = ((arg0) as usize as u64);
    let t978 = [vdupq_n_u32((t977) as u32); 2];
    let t979 = [vdupq_n_u32(((t977) >> 32) as u32); 2];
    let t980: [uint32x4_t; 2] = [vaddq_u32(t978[0], t976[0]), vaddq_u32(t978[1], t976[1])];
    let t981: [uint32x4_t; 2] = [vcltq_u32(t980[0], t978[0]), vcltq_u32(t980[1], t978[1])];
    let t982: [uint32x4_t; 4] = [t980[0], t980[1], vsubq_u32(vaddq_u32(t979[0], t976[2]), t981[0]), vsubq_u32(vaddq_u32(t979[1], t976[3]), t981[1])];
    x24 = t982;
    let t983: u64 = 7u64;
    let t984 = [vdupq_n_u32((t983) as u32); 2];
    let t985: [uint32x4_t; 2] = [vandq_u32(x8[0], t984[0]), vandq_u32(x8[1], t984[1])];
    let t986 = [vdupq_n_u32(8u32); 2];
    let t987: [uint32x4_t; 2] = [vmulq_u32(t985[0], t986[0]), vmulq_u32(t985[1], t986[1])];
    x32 = t987;
    let mut t988: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x24[2]) as u64) << 32 | vgetq_lane_u32::<0>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t988[0] = vsetq_lane_u32::<0>(v as u32, t988[0]); t988[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t988[2]); }
    { let v = (((vgetq_lane_u32::<1>(x24[2]) as u64) << 32 | vgetq_lane_u32::<1>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t988[0] = vsetq_lane_u32::<1>(v as u32, t988[0]); t988[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t988[2]); }
    { let v = (((vgetq_lane_u32::<2>(x24[2]) as u64) << 32 | vgetq_lane_u32::<2>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t988[0] = vsetq_lane_u32::<2>(v as u32, t988[0]); t988[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t988[2]); }
    { let v = (((vgetq_lane_u32::<3>(x24[2]) as u64) << 32 | vgetq_lane_u32::<3>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t988[0] = vsetq_lane_u32::<3>(v as u32, t988[0]); t988[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t988[2]); }
    { let v = (((vgetq_lane_u32::<0>(x24[3]) as u64) << 32 | vgetq_lane_u32::<0>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t988[1] = vsetq_lane_u32::<0>(v as u32, t988[1]); t988[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t988[3]); }
    { let v = (((vgetq_lane_u32::<1>(x24[3]) as u64) << 32 | vgetq_lane_u32::<1>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t988[1] = vsetq_lane_u32::<1>(v as u32, t988[1]); t988[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t988[3]); }
    { let v = (((vgetq_lane_u32::<2>(x24[3]) as u64) << 32 | vgetq_lane_u32::<2>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t988[1] = vsetq_lane_u32::<2>(v as u32, t988[1]); t988[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t988[3]); }
    { let v = (((vgetq_lane_u32::<3>(x24[3]) as u64) << 32 | vgetq_lane_u32::<3>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t988[1] = vsetq_lane_u32::<3>(v as u32, t988[1]); t988[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t988[3]); }
    let t989: [uint32x4_t; 2] = [x32[0], x32[1]];
    let t990: [uint32x4_t; 2] = [vandq_u32(t989[0], vdupq_n_u32(63)), vandq_u32(t989[1], vdupq_n_u32(63))];
    let t991: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t988[0], vnegq_s32(vreinterpretq_s32_u32(t990[0]))), vshlq_u32(t988[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t990[0])))), vorrq_u32(vshlq_u32(t988[1], vnegq_s32(vreinterpretq_s32_u32(t990[1]))), vshlq_u32(t988[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t990[1])))), vshlq_u32(t988[2], vnegq_s32(vreinterpretq_s32_u32(t990[0]))), vshlq_u32(t988[3], vnegq_s32(vreinterpretq_s32_u32(t990[1])))];
    x40 = t991;
    let t992 = [vdupq_n_u32(0u32); 2];
    let t993: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t992[0])), vmvnq_u32(vceqq_u32(x32[1], t992[1]))];
    let t994 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t995: [uint32x4_t; 2] = [vaddq_u32(x32[0], t994[0]), vaddq_u32(x32[1], t994[1])];
    let t996 = [vdupq_n_u32(64u32); 2];
    let t997: [uint32x4_t; 2] = [vcgtq_u32(t995[0], t996[0]), vcgtq_u32(t995[1], t996[1])];
    let t998: [uint32x4_t; 2] = [vandq_u32(t993[0], t997[0]), vandq_u32(t993[1], t997[1])];
    let mut t999: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1000: [uint32x4_t; 2] = [vmvnq_u32(t998[0]), vmvnq_u32(t998[1])];
    if any(&t998) {
        let t1001: u64 = (1u64).wrapping_mul(8u64);
        let t1002 = [vdupq_n_u32((t1001) as u32); 2];
        let t1003 = [vdupq_n_u32(((t1001) >> 32) as u32); 2];
        let t1004: [uint32x4_t; 2] = [vaddq_u32(x24[0], t1002[0]), vaddq_u32(x24[1], t1002[1])];
        let t1005: [uint32x4_t; 2] = [vcltq_u32(t1004[0], x24[0]), vcltq_u32(t1004[1], x24[1])];
        let t1006: [uint32x4_t; 4] = [t1004[0], t1004[1], vsubq_u32(vaddq_u32(x24[2], t1003[0]), t1005[0]), vsubq_u32(vaddq_u32(x24[3], t1003[1]), t1005[1])];
        let mut t1007: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1006[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1006[0]) as u64) as usize as *const u64).read_unaligned(); t1007[0] = vsetq_lane_u32::<0>(v as u32, t1007[0]); t1007[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1007[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1006[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1006[0]) as u64) as usize as *const u64).read_unaligned(); t1007[0] = vsetq_lane_u32::<1>(v as u32, t1007[0]); t1007[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1007[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1006[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1006[0]) as u64) as usize as *const u64).read_unaligned(); t1007[0] = vsetq_lane_u32::<2>(v as u32, t1007[0]); t1007[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1007[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1006[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1006[0]) as u64) as usize as *const u64).read_unaligned(); t1007[0] = vsetq_lane_u32::<3>(v as u32, t1007[0]); t1007[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1007[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1006[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1006[1]) as u64) as usize as *const u64).read_unaligned(); t1007[1] = vsetq_lane_u32::<0>(v as u32, t1007[1]); t1007[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1007[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1006[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1006[1]) as u64) as usize as *const u64).read_unaligned(); t1007[1] = vsetq_lane_u32::<1>(v as u32, t1007[1]); t1007[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1007[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1006[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1006[1]) as u64) as usize as *const u64).read_unaligned(); t1007[1] = vsetq_lane_u32::<2>(v as u32, t1007[1]); t1007[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1007[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1006[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1006[1]) as u64) as usize as *const u64).read_unaligned(); t1007[1] = vsetq_lane_u32::<3>(v as u32, t1007[1]); t1007[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1007[3]); }
        let t1008: [uint32x4_t; 2] = [vsubq_u32(t996[0], x32[0]), vsubq_u32(t996[1], x32[1])];
        let t1009: [uint32x4_t; 2] = [t1008[0], t1008[1]];
        let t1010: [uint32x4_t; 2] = [vandq_u32(t1009[0], vdupq_n_u32(63)), vandq_u32(t1009[1], vdupq_n_u32(63))];
        let t1011: [uint32x4_t; 4] = [vshlq_u32(t1007[0], vreinterpretq_s32_u32(t1010[0])), vshlq_u32(t1007[1], vreinterpretq_s32_u32(t1010[1])), vorrq_u32(vshlq_u32(t1007[2], vreinterpretq_s32_u32(t1010[0])), vshlq_u32(t1007[0], vsubq_s32(vreinterpretq_s32_u32(t1010[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1007[3], vreinterpretq_s32_u32(t1010[1])), vshlq_u32(t1007[1], vsubq_s32(vreinterpretq_s32_u32(t1010[1]), vdupq_n_s32(32))))];
        let t1012: [uint32x4_t; 4] = [vorrq_u32(x40[0], t1011[0]), vorrq_u32(x40[1], t1011[1]), vorrq_u32(x40[2], t1011[2]), vorrq_u32(x40[3], t1011[3])];
        t999 = [vbslq_u32(t998[0], t1012[0], t999[0]), vbslq_u32(t998[1], t1012[1], t999[1]), vbslq_u32(t998[0], t1012[2], t999[2]), vbslq_u32(t998[1], t1012[3], t999[3])];
    }
    {
        t999 = [vbslq_u32(t1000[0], x40[0], t999[0]), vbslq_u32(t1000[1], x40[1], t999[1]), vbslq_u32(t1000[0], x40[2], t999[2]), vbslq_u32(t1000[1], x40[3], t999[3])];
    }
    res = t999;
    return res;
}

#[inline(always)]
unsafe fn b19_r0al1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8b: *const u8, x8i: [uint32x4_t; 2], x16: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if ((x16) == (8u32)) {
        let t1013 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], x8i[0]), vandq_u32(live[1], x8i[1]))));
        let t1014: [uint32x4_t; 2] = [vbslq_u32(live[0], x8i[0], t1013), vbslq_u32(live[1], x8i[1], t1013)];
        let mut t1015: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1014[0]) as usize * 1) as *const u64).read_unaligned(); t1015[0] = vsetq_lane_u32::<0>(v as u32, t1015[0]); t1015[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1015[2]); }
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1014[0]) as usize * 1) as *const u64).read_unaligned(); t1015[0] = vsetq_lane_u32::<1>(v as u32, t1015[0]); t1015[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1015[2]); }
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1014[0]) as usize * 1) as *const u64).read_unaligned(); t1015[0] = vsetq_lane_u32::<2>(v as u32, t1015[0]); t1015[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1015[2]); }
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1014[0]) as usize * 1) as *const u64).read_unaligned(); t1015[0] = vsetq_lane_u32::<3>(v as u32, t1015[0]); t1015[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1015[2]); }
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1014[1]) as usize * 1) as *const u64).read_unaligned(); t1015[1] = vsetq_lane_u32::<0>(v as u32, t1015[1]); t1015[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1015[3]); }
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1014[1]) as usize * 1) as *const u64).read_unaligned(); t1015[1] = vsetq_lane_u32::<1>(v as u32, t1015[1]); t1015[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1015[3]); }
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1014[1]) as usize * 1) as *const u64).read_unaligned(); t1015[1] = vsetq_lane_u32::<2>(v as u32, t1015[1]); t1015[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1015[3]); }
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1014[1]) as usize * 1) as *const u64).read_unaligned(); t1015[1] = vsetq_lane_u32::<3>(v as u32, t1015[1]); t1015[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1015[3]); }
        res = t1015;
        return res;
    }
    let t1016: [uint32x4_t; 2] = [x8i[0], x8i[1]];
    let t1017: u64 = ((x8b) as usize as u64);
    let t1018 = [vdupq_n_u32((t1017) as u32); 2];
    let t1019 = [vdupq_n_u32(((t1017) >> 32) as u32); 2];
    let t1020: [uint32x4_t; 2] = [vaddq_u32(t1018[0], t1016[0]), vaddq_u32(t1018[1], t1016[1])];
    let t1021: [uint32x4_t; 2] = [vcltq_u32(t1020[0], t1018[0]), vcltq_u32(t1020[1], t1018[1])];
    let t1022: [uint32x4_t; 4] = [t1020[0], t1020[1], vsubq_u32(vaddq_u32(t1019[0], vdupq_n_u32(0)), t1021[0]), vsubq_u32(vaddq_u32(t1019[1], vdupq_n_u32(0)), t1021[1])];
    let t1023: u64 = !(7u64);
    let t1024 = [vdupq_n_u32((t1023) as u32); 2];
    let t1025 = [vdupq_n_u32(((t1023) >> 32) as u32); 2];
    let t1026: [uint32x4_t; 4] = [vandq_u32(t1022[0], t1024[0]), vandq_u32(t1022[1], t1024[1]), vandq_u32(t1022[2], t1025[0]), vandq_u32(t1022[3], t1025[1])];
    let t1027: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
    let t1028: [uint32x4_t; 4] = [vandq_u32(t1026[0], t1027[0]), vandq_u32(t1026[1], t1027[1]), vandq_u32(t1026[2], t1027[2]), vandq_u32(t1026[3], t1027[3])];
    let t1029: u64 = ((arg0) as usize as u64);
    let t1030 = [vdupq_n_u32((t1029) as u32); 2];
    let t1031 = [vdupq_n_u32(((t1029) >> 32) as u32); 2];
    let t1032: [uint32x4_t; 2] = [vaddq_u32(t1030[0], t1028[0]), vaddq_u32(t1030[1], t1028[1])];
    let t1033: [uint32x4_t; 2] = [vcltq_u32(t1032[0], t1030[0]), vcltq_u32(t1032[1], t1030[1])];
    let t1034: [uint32x4_t; 4] = [t1032[0], t1032[1], vsubq_u32(vaddq_u32(t1031[0], t1028[2]), t1033[0]), vsubq_u32(vaddq_u32(t1031[1], t1028[3]), t1033[1])];
    x24 = t1034;
    let t1035: [uint32x4_t; 2] = [x8i[0], x8i[1]];
    let t1036: u64 = ((x8b) as usize as u64);
    let t1037 = [vdupq_n_u32((t1036) as u32); 2];
    let t1038 = [vdupq_n_u32(((t1036) >> 32) as u32); 2];
    let t1039: [uint32x4_t; 2] = [vaddq_u32(t1037[0], t1035[0]), vaddq_u32(t1037[1], t1035[1])];
    let t1040: [uint32x4_t; 2] = [vcltq_u32(t1039[0], t1037[0]), vcltq_u32(t1039[1], t1037[1])];
    let t1041: [uint32x4_t; 4] = [t1039[0], t1039[1], vsubq_u32(vaddq_u32(t1038[0], vdupq_n_u32(0)), t1040[0]), vsubq_u32(vaddq_u32(t1038[1], vdupq_n_u32(0)), t1040[1])];
    let t1042: u64 = 7u64;
    let t1043 = [vdupq_n_u32((t1042) as u32); 2];
    let t1044: [uint32x4_t; 2] = [vandq_u32(t1041[0], t1043[0]), vandq_u32(t1041[1], t1043[1])];
    let t1045 = [vdupq_n_u32(8u32); 2];
    let t1046: [uint32x4_t; 2] = [vmulq_u32(t1044[0], t1045[0]), vmulq_u32(t1044[1], t1045[1])];
    x32 = t1046;
    let mut t1047: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x24[2]) as u64) << 32 | vgetq_lane_u32::<0>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1047[0] = vsetq_lane_u32::<0>(v as u32, t1047[0]); t1047[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1047[2]); }
    { let v = (((vgetq_lane_u32::<1>(x24[2]) as u64) << 32 | vgetq_lane_u32::<1>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1047[0] = vsetq_lane_u32::<1>(v as u32, t1047[0]); t1047[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1047[2]); }
    { let v = (((vgetq_lane_u32::<2>(x24[2]) as u64) << 32 | vgetq_lane_u32::<2>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1047[0] = vsetq_lane_u32::<2>(v as u32, t1047[0]); t1047[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1047[2]); }
    { let v = (((vgetq_lane_u32::<3>(x24[2]) as u64) << 32 | vgetq_lane_u32::<3>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1047[0] = vsetq_lane_u32::<3>(v as u32, t1047[0]); t1047[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1047[2]); }
    { let v = (((vgetq_lane_u32::<0>(x24[3]) as u64) << 32 | vgetq_lane_u32::<0>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1047[1] = vsetq_lane_u32::<0>(v as u32, t1047[1]); t1047[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1047[3]); }
    { let v = (((vgetq_lane_u32::<1>(x24[3]) as u64) << 32 | vgetq_lane_u32::<1>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1047[1] = vsetq_lane_u32::<1>(v as u32, t1047[1]); t1047[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1047[3]); }
    { let v = (((vgetq_lane_u32::<2>(x24[3]) as u64) << 32 | vgetq_lane_u32::<2>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1047[1] = vsetq_lane_u32::<2>(v as u32, t1047[1]); t1047[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1047[3]); }
    { let v = (((vgetq_lane_u32::<3>(x24[3]) as u64) << 32 | vgetq_lane_u32::<3>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1047[1] = vsetq_lane_u32::<3>(v as u32, t1047[1]); t1047[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1047[3]); }
    let t1048: [uint32x4_t; 2] = [x32[0], x32[1]];
    let t1049: [uint32x4_t; 2] = [vandq_u32(t1048[0], vdupq_n_u32(63)), vandq_u32(t1048[1], vdupq_n_u32(63))];
    let t1050: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t1047[0], vnegq_s32(vreinterpretq_s32_u32(t1049[0]))), vshlq_u32(t1047[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1049[0])))), vorrq_u32(vshlq_u32(t1047[1], vnegq_s32(vreinterpretq_s32_u32(t1049[1]))), vshlq_u32(t1047[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1049[1])))), vshlq_u32(t1047[2], vnegq_s32(vreinterpretq_s32_u32(t1049[0]))), vshlq_u32(t1047[3], vnegq_s32(vreinterpretq_s32_u32(t1049[1])))];
    x40 = t1050;
    let t1051 = [vdupq_n_u32(0u32); 2];
    let t1052: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t1051[0])), vmvnq_u32(vceqq_u32(x32[1], t1051[1]))];
    let t1053 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t1054: [uint32x4_t; 2] = [vaddq_u32(x32[0], t1053[0]), vaddq_u32(x32[1], t1053[1])];
    let t1055 = [vdupq_n_u32(64u32); 2];
    let t1056: [uint32x4_t; 2] = [vcgtq_u32(t1054[0], t1055[0]), vcgtq_u32(t1054[1], t1055[1])];
    let t1057: [uint32x4_t; 2] = [vandq_u32(t1052[0], t1056[0]), vandq_u32(t1052[1], t1056[1])];
    let mut t1058: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1059: [uint32x4_t; 2] = [vandq_u32(live[0], t1057[0]), vandq_u32(live[1], t1057[1])];
    let t1060: [uint32x4_t; 2] = [vbicq_u32(live[0], t1057[0]), vbicq_u32(live[1], t1057[1])];
    if any(&t1059) {
        let t1061: u64 = (1u64).wrapping_mul(8u64);
        let t1062 = [vdupq_n_u32((t1061) as u32); 2];
        let t1063 = [vdupq_n_u32(((t1061) >> 32) as u32); 2];
        let t1064: [uint32x4_t; 2] = [vaddq_u32(x24[0], t1062[0]), vaddq_u32(x24[1], t1062[1])];
        let t1065: [uint32x4_t; 2] = [vcltq_u32(t1064[0], x24[0]), vcltq_u32(t1064[1], x24[1])];
        let t1066: [uint32x4_t; 4] = [t1064[0], t1064[1], vsubq_u32(vaddq_u32(x24[2], t1063[0]), t1065[0]), vsubq_u32(vaddq_u32(x24[3], t1063[1]), t1065[1])];
        let mut t1067: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1066[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1066[0]) as u64) as usize as *const u64).read_unaligned(); t1067[0] = vsetq_lane_u32::<0>(v as u32, t1067[0]); t1067[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1067[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1066[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1066[0]) as u64) as usize as *const u64).read_unaligned(); t1067[0] = vsetq_lane_u32::<1>(v as u32, t1067[0]); t1067[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1067[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1066[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1066[0]) as u64) as usize as *const u64).read_unaligned(); t1067[0] = vsetq_lane_u32::<2>(v as u32, t1067[0]); t1067[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1067[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1066[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1066[0]) as u64) as usize as *const u64).read_unaligned(); t1067[0] = vsetq_lane_u32::<3>(v as u32, t1067[0]); t1067[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1067[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1066[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1066[1]) as u64) as usize as *const u64).read_unaligned(); t1067[1] = vsetq_lane_u32::<0>(v as u32, t1067[1]); t1067[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1067[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1066[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1066[1]) as u64) as usize as *const u64).read_unaligned(); t1067[1] = vsetq_lane_u32::<1>(v as u32, t1067[1]); t1067[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1067[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1066[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1066[1]) as u64) as usize as *const u64).read_unaligned(); t1067[1] = vsetq_lane_u32::<2>(v as u32, t1067[1]); t1067[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1067[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1066[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1066[1]) as u64) as usize as *const u64).read_unaligned(); t1067[1] = vsetq_lane_u32::<3>(v as u32, t1067[1]); t1067[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1067[3]); }
        let t1068: [uint32x4_t; 2] = [vsubq_u32(t1055[0], x32[0]), vsubq_u32(t1055[1], x32[1])];
        let t1069: [uint32x4_t; 2] = [t1068[0], t1068[1]];
        let t1070: [uint32x4_t; 2] = [vandq_u32(t1069[0], vdupq_n_u32(63)), vandq_u32(t1069[1], vdupq_n_u32(63))];
        let t1071: [uint32x4_t; 4] = [vshlq_u32(t1067[0], vreinterpretq_s32_u32(t1070[0])), vshlq_u32(t1067[1], vreinterpretq_s32_u32(t1070[1])), vorrq_u32(vshlq_u32(t1067[2], vreinterpretq_s32_u32(t1070[0])), vshlq_u32(t1067[0], vsubq_s32(vreinterpretq_s32_u32(t1070[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1067[3], vreinterpretq_s32_u32(t1070[1])), vshlq_u32(t1067[1], vsubq_s32(vreinterpretq_s32_u32(t1070[1]), vdupq_n_s32(32))))];
        let t1072: [uint32x4_t; 4] = [vorrq_u32(x40[0], t1071[0]), vorrq_u32(x40[1], t1071[1]), vorrq_u32(x40[2], t1071[2]), vorrq_u32(x40[3], t1071[3])];
        t1058 = [vbslq_u32(t1059[0], t1072[0], t1058[0]), vbslq_u32(t1059[1], t1072[1], t1058[1]), vbslq_u32(t1059[0], t1072[2], t1058[2]), vbslq_u32(t1059[1], t1072[3], t1058[3])];
    }
    {
        t1058 = [vbslq_u32(t1060[0], x40[0], t1058[0]), vbslq_u32(t1060[1], x40[1], t1058[1]), vbslq_u32(t1060[0], x40[2], t1058[2]), vbslq_u32(t1060[1], x40[3], t1058[3])];
    }
    res = t1058;
    return res;
}

#[inline(always)]
unsafe fn b19_r0an1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8b: *const u8, x8if: u32, x8is: u32, x16: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if ((x16) == (8u32)) {
        let t1073: [uint32x4_t; 4];
        if (x8is) == 8u32 && ((x8if) as u64) + 7 * ((x8is) as u64) <= u32::MAX as u64 && all(&live) {
            let t1074 = ((arg0).wrapping_add((x8b) as usize) as *const u8).add(((x8if) as usize) * 1);
            let t10750 = vld2q_u32((t1074 as *const u32).add(0));
            let t10751 = vld2q_u32((t1074 as *const u32).add(8));
            let t1075: [uint32x4_t; 4] = [t10750.0, t10751.0, t10750.1, t10751.1];
            t1073 = t1075;
        } else {
            let t1076: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8is)), vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8is))];
            let t1077 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t1076[0]), vandq_u32(live[1], t1076[1]))));
            let t1078: [uint32x4_t; 2] = [vbslq_u32(live[0], t1076[0], t1077), vbslq_u32(live[1], t1076[1], t1077)];
            let mut t1079: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1078[0]) as usize * 1) as *const u64).read_unaligned(); t1079[0] = vsetq_lane_u32::<0>(v as u32, t1079[0]); t1079[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1079[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1078[0]) as usize * 1) as *const u64).read_unaligned(); t1079[0] = vsetq_lane_u32::<1>(v as u32, t1079[0]); t1079[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1079[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1078[0]) as usize * 1) as *const u64).read_unaligned(); t1079[0] = vsetq_lane_u32::<2>(v as u32, t1079[0]); t1079[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1079[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1078[0]) as usize * 1) as *const u64).read_unaligned(); t1079[0] = vsetq_lane_u32::<3>(v as u32, t1079[0]); t1079[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1079[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1078[1]) as usize * 1) as *const u64).read_unaligned(); t1079[1] = vsetq_lane_u32::<0>(v as u32, t1079[1]); t1079[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1079[3]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1078[1]) as usize * 1) as *const u64).read_unaligned(); t1079[1] = vsetq_lane_u32::<1>(v as u32, t1079[1]); t1079[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1079[3]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1078[1]) as usize * 1) as *const u64).read_unaligned(); t1079[1] = vsetq_lane_u32::<2>(v as u32, t1079[1]); t1079[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1079[3]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1078[1]) as usize * 1) as *const u64).read_unaligned(); t1079[1] = vsetq_lane_u32::<3>(v as u32, t1079[1]); t1079[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1079[3]); }
            t1073 = t1079;
        }
        res = t1073;
        return res;
    }
    let t1080: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8is)), vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8is))];
    let t1081: [uint32x4_t; 2] = [t1080[0], t1080[1]];
    let t1082: u64 = ((x8b) as usize as u64);
    let t1083 = [vdupq_n_u32((t1082) as u32); 2];
    let t1084 = [vdupq_n_u32(((t1082) >> 32) as u32); 2];
    let t1085: [uint32x4_t; 2] = [vaddq_u32(t1083[0], t1081[0]), vaddq_u32(t1083[1], t1081[1])];
    let t1086: [uint32x4_t; 2] = [vcltq_u32(t1085[0], t1083[0]), vcltq_u32(t1085[1], t1083[1])];
    let t1087: [uint32x4_t; 4] = [t1085[0], t1085[1], vsubq_u32(vaddq_u32(t1084[0], vdupq_n_u32(0)), t1086[0]), vsubq_u32(vaddq_u32(t1084[1], vdupq_n_u32(0)), t1086[1])];
    let t1088: u64 = !(7u64);
    let t1089 = [vdupq_n_u32((t1088) as u32); 2];
    let t1090 = [vdupq_n_u32(((t1088) >> 32) as u32); 2];
    let t1091: [uint32x4_t; 4] = [vandq_u32(t1087[0], t1089[0]), vandq_u32(t1087[1], t1089[1]), vandq_u32(t1087[2], t1090[0]), vandq_u32(t1087[3], t1090[1])];
    let t1092: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
    let t1093: [uint32x4_t; 4] = [vandq_u32(t1091[0], t1092[0]), vandq_u32(t1091[1], t1092[1]), vandq_u32(t1091[2], t1092[2]), vandq_u32(t1091[3], t1092[3])];
    let t1094: u64 = ((arg0) as usize as u64);
    let t1095 = [vdupq_n_u32((t1094) as u32); 2];
    let t1096 = [vdupq_n_u32(((t1094) >> 32) as u32); 2];
    let t1097: [uint32x4_t; 2] = [vaddq_u32(t1095[0], t1093[0]), vaddq_u32(t1095[1], t1093[1])];
    let t1098: [uint32x4_t; 2] = [vcltq_u32(t1097[0], t1095[0]), vcltq_u32(t1097[1], t1095[1])];
    let t1099: [uint32x4_t; 4] = [t1097[0], t1097[1], vsubq_u32(vaddq_u32(t1096[0], t1093[2]), t1098[0]), vsubq_u32(vaddq_u32(t1096[1], t1093[3]), t1098[1])];
    x24 = t1099;
    let t1100: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8is)), vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8is))];
    let t1101: [uint32x4_t; 2] = [t1100[0], t1100[1]];
    let t1102: u64 = ((x8b) as usize as u64);
    let t1103 = [vdupq_n_u32((t1102) as u32); 2];
    let t1104 = [vdupq_n_u32(((t1102) >> 32) as u32); 2];
    let t1105: [uint32x4_t; 2] = [vaddq_u32(t1103[0], t1101[0]), vaddq_u32(t1103[1], t1101[1])];
    let t1106: [uint32x4_t; 2] = [vcltq_u32(t1105[0], t1103[0]), vcltq_u32(t1105[1], t1103[1])];
    let t1107: [uint32x4_t; 4] = [t1105[0], t1105[1], vsubq_u32(vaddq_u32(t1104[0], vdupq_n_u32(0)), t1106[0]), vsubq_u32(vaddq_u32(t1104[1], vdupq_n_u32(0)), t1106[1])];
    let t1108: u64 = 7u64;
    let t1109 = [vdupq_n_u32((t1108) as u32); 2];
    let t1110: [uint32x4_t; 2] = [vandq_u32(t1107[0], t1109[0]), vandq_u32(t1107[1], t1109[1])];
    let t1111 = [vdupq_n_u32(8u32); 2];
    let t1112: [uint32x4_t; 2] = [vmulq_u32(t1110[0], t1111[0]), vmulq_u32(t1110[1], t1111[1])];
    x32 = t1112;
    let mut t1113: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x24[2]) as u64) << 32 | vgetq_lane_u32::<0>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1113[0] = vsetq_lane_u32::<0>(v as u32, t1113[0]); t1113[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1113[2]); }
    { let v = (((vgetq_lane_u32::<1>(x24[2]) as u64) << 32 | vgetq_lane_u32::<1>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1113[0] = vsetq_lane_u32::<1>(v as u32, t1113[0]); t1113[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1113[2]); }
    { let v = (((vgetq_lane_u32::<2>(x24[2]) as u64) << 32 | vgetq_lane_u32::<2>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1113[0] = vsetq_lane_u32::<2>(v as u32, t1113[0]); t1113[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1113[2]); }
    { let v = (((vgetq_lane_u32::<3>(x24[2]) as u64) << 32 | vgetq_lane_u32::<3>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1113[0] = vsetq_lane_u32::<3>(v as u32, t1113[0]); t1113[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1113[2]); }
    { let v = (((vgetq_lane_u32::<0>(x24[3]) as u64) << 32 | vgetq_lane_u32::<0>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1113[1] = vsetq_lane_u32::<0>(v as u32, t1113[1]); t1113[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1113[3]); }
    { let v = (((vgetq_lane_u32::<1>(x24[3]) as u64) << 32 | vgetq_lane_u32::<1>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1113[1] = vsetq_lane_u32::<1>(v as u32, t1113[1]); t1113[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1113[3]); }
    { let v = (((vgetq_lane_u32::<2>(x24[3]) as u64) << 32 | vgetq_lane_u32::<2>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1113[1] = vsetq_lane_u32::<2>(v as u32, t1113[1]); t1113[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1113[3]); }
    { let v = (((vgetq_lane_u32::<3>(x24[3]) as u64) << 32 | vgetq_lane_u32::<3>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1113[1] = vsetq_lane_u32::<3>(v as u32, t1113[1]); t1113[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1113[3]); }
    let t1114: [uint32x4_t; 2] = [x32[0], x32[1]];
    let t1115: [uint32x4_t; 2] = [vandq_u32(t1114[0], vdupq_n_u32(63)), vandq_u32(t1114[1], vdupq_n_u32(63))];
    let t1116: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t1113[0], vnegq_s32(vreinterpretq_s32_u32(t1115[0]))), vshlq_u32(t1113[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1115[0])))), vorrq_u32(vshlq_u32(t1113[1], vnegq_s32(vreinterpretq_s32_u32(t1115[1]))), vshlq_u32(t1113[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1115[1])))), vshlq_u32(t1113[2], vnegq_s32(vreinterpretq_s32_u32(t1115[0]))), vshlq_u32(t1113[3], vnegq_s32(vreinterpretq_s32_u32(t1115[1])))];
    x40 = t1116;
    let t1117 = [vdupq_n_u32(0u32); 2];
    let t1118: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t1117[0])), vmvnq_u32(vceqq_u32(x32[1], t1117[1]))];
    let t1119 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t1120: [uint32x4_t; 2] = [vaddq_u32(x32[0], t1119[0]), vaddq_u32(x32[1], t1119[1])];
    let t1121 = [vdupq_n_u32(64u32); 2];
    let t1122: [uint32x4_t; 2] = [vcgtq_u32(t1120[0], t1121[0]), vcgtq_u32(t1120[1], t1121[1])];
    let t1123: [uint32x4_t; 2] = [vandq_u32(t1118[0], t1122[0]), vandq_u32(t1118[1], t1122[1])];
    let mut t1124: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1125: [uint32x4_t; 2] = [vandq_u32(live[0], t1123[0]), vandq_u32(live[1], t1123[1])];
    let t1126: [uint32x4_t; 2] = [vbicq_u32(live[0], t1123[0]), vbicq_u32(live[1], t1123[1])];
    if any(&t1125) {
        let t1127: u64 = (1u64).wrapping_mul(8u64);
        let t1128 = [vdupq_n_u32((t1127) as u32); 2];
        let t1129 = [vdupq_n_u32(((t1127) >> 32) as u32); 2];
        let t1130: [uint32x4_t; 2] = [vaddq_u32(x24[0], t1128[0]), vaddq_u32(x24[1], t1128[1])];
        let t1131: [uint32x4_t; 2] = [vcltq_u32(t1130[0], x24[0]), vcltq_u32(t1130[1], x24[1])];
        let t1132: [uint32x4_t; 4] = [t1130[0], t1130[1], vsubq_u32(vaddq_u32(x24[2], t1129[0]), t1131[0]), vsubq_u32(vaddq_u32(x24[3], t1129[1]), t1131[1])];
        let mut t1133: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1132[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1132[0]) as u64) as usize as *const u64).read_unaligned(); t1133[0] = vsetq_lane_u32::<0>(v as u32, t1133[0]); t1133[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1133[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1132[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1132[0]) as u64) as usize as *const u64).read_unaligned(); t1133[0] = vsetq_lane_u32::<1>(v as u32, t1133[0]); t1133[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1133[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1132[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1132[0]) as u64) as usize as *const u64).read_unaligned(); t1133[0] = vsetq_lane_u32::<2>(v as u32, t1133[0]); t1133[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1133[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1132[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1132[0]) as u64) as usize as *const u64).read_unaligned(); t1133[0] = vsetq_lane_u32::<3>(v as u32, t1133[0]); t1133[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1133[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1132[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1132[1]) as u64) as usize as *const u64).read_unaligned(); t1133[1] = vsetq_lane_u32::<0>(v as u32, t1133[1]); t1133[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1133[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1132[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1132[1]) as u64) as usize as *const u64).read_unaligned(); t1133[1] = vsetq_lane_u32::<1>(v as u32, t1133[1]); t1133[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1133[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1132[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1132[1]) as u64) as usize as *const u64).read_unaligned(); t1133[1] = vsetq_lane_u32::<2>(v as u32, t1133[1]); t1133[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1133[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1132[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1132[1]) as u64) as usize as *const u64).read_unaligned(); t1133[1] = vsetq_lane_u32::<3>(v as u32, t1133[1]); t1133[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1133[3]); }
        let t1134: [uint32x4_t; 2] = [vsubq_u32(t1121[0], x32[0]), vsubq_u32(t1121[1], x32[1])];
        let t1135: [uint32x4_t; 2] = [t1134[0], t1134[1]];
        let t1136: [uint32x4_t; 2] = [vandq_u32(t1135[0], vdupq_n_u32(63)), vandq_u32(t1135[1], vdupq_n_u32(63))];
        let t1137: [uint32x4_t; 4] = [vshlq_u32(t1133[0], vreinterpretq_s32_u32(t1136[0])), vshlq_u32(t1133[1], vreinterpretq_s32_u32(t1136[1])), vorrq_u32(vshlq_u32(t1133[2], vreinterpretq_s32_u32(t1136[0])), vshlq_u32(t1133[0], vsubq_s32(vreinterpretq_s32_u32(t1136[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1133[3], vreinterpretq_s32_u32(t1136[1])), vshlq_u32(t1133[1], vsubq_s32(vreinterpretq_s32_u32(t1136[1]), vdupq_n_s32(32))))];
        let t1138: [uint32x4_t; 4] = [vorrq_u32(x40[0], t1137[0]), vorrq_u32(x40[1], t1137[1]), vorrq_u32(x40[2], t1137[2]), vorrq_u32(x40[3], t1137[3])];
        t1124 = [vbslq_u32(t1125[0], t1138[0], t1124[0]), vbslq_u32(t1125[1], t1138[1], t1124[1]), vbslq_u32(t1125[0], t1138[2], t1124[2]), vbslq_u32(t1125[1], t1138[3], t1124[3])];
    }
    {
        t1124 = [vbslq_u32(t1126[0], x40[0], t1124[0]), vbslq_u32(t1126[1], x40[1], t1124[1]), vbslq_u32(t1126[0], x40[2], t1124[2]), vbslq_u32(t1126[1], x40[3], t1124[3])];
    }
    res = t1124;
    return res;
}

#[inline(always)]
unsafe fn b19_r0an1s_f(l: &Launch, lid0: u32, arg0: *const u8, x8b: *const u8, x8if: u32, x8is: u32, x16: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if ((x16) == (8u32)) {
        let t1139: [uint32x4_t; 4];
        if (x8is) == 8u32 && ((x8if) as u64) + 7 * ((x8is) as u64) <= u32::MAX as u64 {
            let t1140 = ((arg0).wrapping_add((x8b) as usize) as *const u8).add(((x8if) as usize) * 1);
            let t11410 = vld2q_u32((t1140 as *const u32).add(0));
            let t11411 = vld2q_u32((t1140 as *const u32).add(8));
            let t1141: [uint32x4_t; 4] = [t11410.0, t11411.0, t11410.1, t11411.1];
            t1139 = t1141;
        } else {
            let t1142: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8is)), vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8is))];
            let mut t1143: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1142[0]) as usize * 1) as *const u64).read_unaligned(); t1143[0] = vsetq_lane_u32::<0>(v as u32, t1143[0]); t1143[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1143[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1142[0]) as usize * 1) as *const u64).read_unaligned(); t1143[0] = vsetq_lane_u32::<1>(v as u32, t1143[0]); t1143[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1143[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1142[0]) as usize * 1) as *const u64).read_unaligned(); t1143[0] = vsetq_lane_u32::<2>(v as u32, t1143[0]); t1143[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1143[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1142[0]) as usize * 1) as *const u64).read_unaligned(); t1143[0] = vsetq_lane_u32::<3>(v as u32, t1143[0]); t1143[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1143[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1142[1]) as usize * 1) as *const u64).read_unaligned(); t1143[1] = vsetq_lane_u32::<0>(v as u32, t1143[1]); t1143[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1143[3]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1142[1]) as usize * 1) as *const u64).read_unaligned(); t1143[1] = vsetq_lane_u32::<1>(v as u32, t1143[1]); t1143[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1143[3]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1142[1]) as usize * 1) as *const u64).read_unaligned(); t1143[1] = vsetq_lane_u32::<2>(v as u32, t1143[1]); t1143[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1143[3]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1142[1]) as usize * 1) as *const u64).read_unaligned(); t1143[1] = vsetq_lane_u32::<3>(v as u32, t1143[1]); t1143[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1143[3]); }
            t1139 = t1143;
        }
        res = t1139;
        return res;
    }
    let t1144: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8is)), vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8is))];
    let t1145: [uint32x4_t; 2] = [t1144[0], t1144[1]];
    let t1146: u64 = ((x8b) as usize as u64);
    let t1147 = [vdupq_n_u32((t1146) as u32); 2];
    let t1148 = [vdupq_n_u32(((t1146) >> 32) as u32); 2];
    let t1149: [uint32x4_t; 2] = [vaddq_u32(t1147[0], t1145[0]), vaddq_u32(t1147[1], t1145[1])];
    let t1150: [uint32x4_t; 2] = [vcltq_u32(t1149[0], t1147[0]), vcltq_u32(t1149[1], t1147[1])];
    let t1151: [uint32x4_t; 4] = [t1149[0], t1149[1], vsubq_u32(vaddq_u32(t1148[0], vdupq_n_u32(0)), t1150[0]), vsubq_u32(vaddq_u32(t1148[1], vdupq_n_u32(0)), t1150[1])];
    let t1152: u64 = !(7u64);
    let t1153 = [vdupq_n_u32((t1152) as u32); 2];
    let t1154 = [vdupq_n_u32(((t1152) >> 32) as u32); 2];
    let t1155: [uint32x4_t; 4] = [vandq_u32(t1151[0], t1153[0]), vandq_u32(t1151[1], t1153[1]), vandq_u32(t1151[2], t1154[0]), vandq_u32(t1151[3], t1154[1])];
    let t1156: u64 = ((arg0) as usize as u64);
    let t1157 = [vdupq_n_u32((t1156) as u32); 2];
    let t1158 = [vdupq_n_u32(((t1156) >> 32) as u32); 2];
    let t1159: [uint32x4_t; 2] = [vaddq_u32(t1157[0], t1155[0]), vaddq_u32(t1157[1], t1155[1])];
    let t1160: [uint32x4_t; 2] = [vcltq_u32(t1159[0], t1157[0]), vcltq_u32(t1159[1], t1157[1])];
    let t1161: [uint32x4_t; 4] = [t1159[0], t1159[1], vsubq_u32(vaddq_u32(t1158[0], t1155[2]), t1160[0]), vsubq_u32(vaddq_u32(t1158[1], t1155[3]), t1160[1])];
    x24 = t1161;
    let t1162: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8is)), vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8is))];
    let t1163: [uint32x4_t; 2] = [t1162[0], t1162[1]];
    let t1164: u64 = ((x8b) as usize as u64);
    let t1165 = [vdupq_n_u32((t1164) as u32); 2];
    let t1166 = [vdupq_n_u32(((t1164) >> 32) as u32); 2];
    let t1167: [uint32x4_t; 2] = [vaddq_u32(t1165[0], t1163[0]), vaddq_u32(t1165[1], t1163[1])];
    let t1168: [uint32x4_t; 2] = [vcltq_u32(t1167[0], t1165[0]), vcltq_u32(t1167[1], t1165[1])];
    let t1169: [uint32x4_t; 4] = [t1167[0], t1167[1], vsubq_u32(vaddq_u32(t1166[0], vdupq_n_u32(0)), t1168[0]), vsubq_u32(vaddq_u32(t1166[1], vdupq_n_u32(0)), t1168[1])];
    let t1170: u64 = 7u64;
    let t1171 = [vdupq_n_u32((t1170) as u32); 2];
    let t1172: [uint32x4_t; 2] = [vandq_u32(t1169[0], t1171[0]), vandq_u32(t1169[1], t1171[1])];
    let t1173 = [vdupq_n_u32(8u32); 2];
    let t1174: [uint32x4_t; 2] = [vmulq_u32(t1172[0], t1173[0]), vmulq_u32(t1172[1], t1173[1])];
    x32 = t1174;
    let mut t1175: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x24[2]) as u64) << 32 | vgetq_lane_u32::<0>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1175[0] = vsetq_lane_u32::<0>(v as u32, t1175[0]); t1175[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1175[2]); }
    { let v = (((vgetq_lane_u32::<1>(x24[2]) as u64) << 32 | vgetq_lane_u32::<1>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1175[0] = vsetq_lane_u32::<1>(v as u32, t1175[0]); t1175[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1175[2]); }
    { let v = (((vgetq_lane_u32::<2>(x24[2]) as u64) << 32 | vgetq_lane_u32::<2>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1175[0] = vsetq_lane_u32::<2>(v as u32, t1175[0]); t1175[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1175[2]); }
    { let v = (((vgetq_lane_u32::<3>(x24[2]) as u64) << 32 | vgetq_lane_u32::<3>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1175[0] = vsetq_lane_u32::<3>(v as u32, t1175[0]); t1175[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1175[2]); }
    { let v = (((vgetq_lane_u32::<0>(x24[3]) as u64) << 32 | vgetq_lane_u32::<0>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1175[1] = vsetq_lane_u32::<0>(v as u32, t1175[1]); t1175[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1175[3]); }
    { let v = (((vgetq_lane_u32::<1>(x24[3]) as u64) << 32 | vgetq_lane_u32::<1>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1175[1] = vsetq_lane_u32::<1>(v as u32, t1175[1]); t1175[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1175[3]); }
    { let v = (((vgetq_lane_u32::<2>(x24[3]) as u64) << 32 | vgetq_lane_u32::<2>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1175[1] = vsetq_lane_u32::<2>(v as u32, t1175[1]); t1175[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1175[3]); }
    { let v = (((vgetq_lane_u32::<3>(x24[3]) as u64) << 32 | vgetq_lane_u32::<3>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1175[1] = vsetq_lane_u32::<3>(v as u32, t1175[1]); t1175[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1175[3]); }
    let t1176: [uint32x4_t; 2] = [x32[0], x32[1]];
    let t1177: [uint32x4_t; 2] = [vandq_u32(t1176[0], vdupq_n_u32(63)), vandq_u32(t1176[1], vdupq_n_u32(63))];
    let t1178: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t1175[0], vnegq_s32(vreinterpretq_s32_u32(t1177[0]))), vshlq_u32(t1175[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1177[0])))), vorrq_u32(vshlq_u32(t1175[1], vnegq_s32(vreinterpretq_s32_u32(t1177[1]))), vshlq_u32(t1175[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1177[1])))), vshlq_u32(t1175[2], vnegq_s32(vreinterpretq_s32_u32(t1177[0]))), vshlq_u32(t1175[3], vnegq_s32(vreinterpretq_s32_u32(t1177[1])))];
    x40 = t1178;
    let t1179 = [vdupq_n_u32(0u32); 2];
    let t1180: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t1179[0])), vmvnq_u32(vceqq_u32(x32[1], t1179[1]))];
    let t1181 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t1182: [uint32x4_t; 2] = [vaddq_u32(x32[0], t1181[0]), vaddq_u32(x32[1], t1181[1])];
    let t1183 = [vdupq_n_u32(64u32); 2];
    let t1184: [uint32x4_t; 2] = [vcgtq_u32(t1182[0], t1183[0]), vcgtq_u32(t1182[1], t1183[1])];
    let t1185: [uint32x4_t; 2] = [vandq_u32(t1180[0], t1184[0]), vandq_u32(t1180[1], t1184[1])];
    let mut t1186: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1187: [uint32x4_t; 2] = [vmvnq_u32(t1185[0]), vmvnq_u32(t1185[1])];
    if any(&t1185) {
        let t1188: u64 = (1u64).wrapping_mul(8u64);
        let t1189 = [vdupq_n_u32((t1188) as u32); 2];
        let t1190 = [vdupq_n_u32(((t1188) >> 32) as u32); 2];
        let t1191: [uint32x4_t; 2] = [vaddq_u32(x24[0], t1189[0]), vaddq_u32(x24[1], t1189[1])];
        let t1192: [uint32x4_t; 2] = [vcltq_u32(t1191[0], x24[0]), vcltq_u32(t1191[1], x24[1])];
        let t1193: [uint32x4_t; 4] = [t1191[0], t1191[1], vsubq_u32(vaddq_u32(x24[2], t1190[0]), t1192[0]), vsubq_u32(vaddq_u32(x24[3], t1190[1]), t1192[1])];
        let mut t1194: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1193[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1193[0]) as u64) as usize as *const u64).read_unaligned(); t1194[0] = vsetq_lane_u32::<0>(v as u32, t1194[0]); t1194[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1194[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1193[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1193[0]) as u64) as usize as *const u64).read_unaligned(); t1194[0] = vsetq_lane_u32::<1>(v as u32, t1194[0]); t1194[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1194[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1193[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1193[0]) as u64) as usize as *const u64).read_unaligned(); t1194[0] = vsetq_lane_u32::<2>(v as u32, t1194[0]); t1194[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1194[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1193[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1193[0]) as u64) as usize as *const u64).read_unaligned(); t1194[0] = vsetq_lane_u32::<3>(v as u32, t1194[0]); t1194[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1194[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1193[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1193[1]) as u64) as usize as *const u64).read_unaligned(); t1194[1] = vsetq_lane_u32::<0>(v as u32, t1194[1]); t1194[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1194[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1193[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1193[1]) as u64) as usize as *const u64).read_unaligned(); t1194[1] = vsetq_lane_u32::<1>(v as u32, t1194[1]); t1194[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1194[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1193[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1193[1]) as u64) as usize as *const u64).read_unaligned(); t1194[1] = vsetq_lane_u32::<2>(v as u32, t1194[1]); t1194[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1194[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1193[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1193[1]) as u64) as usize as *const u64).read_unaligned(); t1194[1] = vsetq_lane_u32::<3>(v as u32, t1194[1]); t1194[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1194[3]); }
        let t1195: [uint32x4_t; 2] = [vsubq_u32(t1183[0], x32[0]), vsubq_u32(t1183[1], x32[1])];
        let t1196: [uint32x4_t; 2] = [t1195[0], t1195[1]];
        let t1197: [uint32x4_t; 2] = [vandq_u32(t1196[0], vdupq_n_u32(63)), vandq_u32(t1196[1], vdupq_n_u32(63))];
        let t1198: [uint32x4_t; 4] = [vshlq_u32(t1194[0], vreinterpretq_s32_u32(t1197[0])), vshlq_u32(t1194[1], vreinterpretq_s32_u32(t1197[1])), vorrq_u32(vshlq_u32(t1194[2], vreinterpretq_s32_u32(t1197[0])), vshlq_u32(t1194[0], vsubq_s32(vreinterpretq_s32_u32(t1197[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1194[3], vreinterpretq_s32_u32(t1197[1])), vshlq_u32(t1194[1], vsubq_s32(vreinterpretq_s32_u32(t1197[1]), vdupq_n_s32(32))))];
        let t1199: [uint32x4_t; 4] = [vorrq_u32(x40[0], t1198[0]), vorrq_u32(x40[1], t1198[1]), vorrq_u32(x40[2], t1198[2]), vorrq_u32(x40[3], t1198[3])];
        t1186 = [vbslq_u32(t1185[0], t1199[0], t1186[0]), vbslq_u32(t1185[1], t1199[1], t1186[1]), vbslq_u32(t1185[0], t1199[2], t1186[2]), vbslq_u32(t1185[1], t1199[3], t1186[3])];
    }
    {
        t1186 = [vbslq_u32(t1187[0], x40[0], t1186[0]), vbslq_u32(t1187[1], x40[1], t1186[1]), vbslq_u32(t1187[0], x40[2], t1186[2]), vbslq_u32(t1187[1], x40[3], t1186[3])];
    }
    res = t1186;
    return res;
}

#[inline(always)]
unsafe fn b20_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t1200: u64;
    if ((x0) >= (64u32)) {
        t1200 = !(0u64);
    } else {
        t1200 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t1200;
    return res;
}

#[inline(always)]
unsafe fn b20_s_f(l: &Launch, lid0: u32, x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t1201: u64;
    if ((x0) >= (64u32)) {
        t1201 = !(0u64);
    } else {
        t1201 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t1201;
    return res;
}

#[inline(always)]
unsafe fn b21_r0sss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: u32, x16: u32, x24: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u64 = 0;
    let mut live = k;
    let mut res: u64 = 0;
    let t1202 = b19_r0ss_p(l, lid0, live, arg0, (((x8) as u64)).wrapping_add(((x16) as u64)), x24);
    x32 = t1202;
    let t1203: u64;
    if ((x24) >= (8u32)) {
        t1203 = x32;
    } else {
        let t1204 = b20_s_p(l, lid0, live, (8u32).wrapping_mul(x24));
        t1203 = ((x32) & (t1204));
    }
    res = t1203;
    return res;
}

#[inline(always)]
unsafe fn b21_r0sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: u32, x16: [uint32x4_t; 2], x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1205 = b19_r0al1s_p(l, lid0, live, arg0, ((((x8) as u64)) as usize as *const u8), x16, x24);
    x32 = t1205;
    let t1206: [uint32x4_t; 4];
    if ((x24) >= (8u32)) {
        t1206 = x32;
    } else {
        let t1207 = b20_s_p(l, lid0, live, (8u32).wrapping_mul(x24));
        let t1208: u64 = t1207;
        let t1209 = [vdupq_n_u32((t1208) as u32); 2];
        let t1210 = [vdupq_n_u32(((t1208) >> 32) as u32); 2];
        let t1211: [uint32x4_t; 4] = [vandq_u32(x32[0], t1209[0]), vandq_u32(x32[1], t1209[1]), vandq_u32(x32[2], t1210[0]), vandq_u32(x32[3], t1210[1])];
        t1206 = t1211;
    }
    res = t1206;
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
    let mut t1212: bool = !(arg4).is_null();
    if t1212 {
        let mut t1213: bool = ((l.group[0]) >= ((arg4.wrapping_add((((x48) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        if !t1213 {
            t1213 = ((l.group[1]) >= ((arg4.wrapping_add(((((x48).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        let mut t1214: bool = t1213;
        if !t1214 {
            t1214 = ((l.group[2]) >= ((arg4.wrapping_add(((((x48).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        t1212 = t1214;
    }
    if t1212 {
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
            let t1215: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
            let mut live = t1215;
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
            let t1216: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
            let mut live = t1216;
            let gid0 = l.group[0] * l.block[0] + lid0;
            'chunk: {
                b0_r0r1r2sr6gi_p(l, lid0, live, arg0, arg1, arg2, x24, arg6, lid0, phase, st.add((lid0 / 8) as usize * 256));
            }
        }
    }
}
