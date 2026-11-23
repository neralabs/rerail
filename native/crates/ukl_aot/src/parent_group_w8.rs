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
                let mut t48 = live;
                loop {
                    // Lanes part: each its own rounds.
                    let t50: [uint32x4_t; 2] = [x80[0], x80[1]];
                    let t51 = [vdupq_n_u32(7u32); 2];
                    let t52: [uint32x4_t; 2] = [vcltq_u32(t50[0], t51[0]), vcltq_u32(t50[1], t51[1])];
                    let t53 = [vdupq_n_u32(4294967295u32); 2];
                    let t54: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x72[0], t53[0])), vmvnq_u32(vceqq_u32(x72[1], t53[1]))];
                    let t55: [uint32x4_t; 2] = [vandq_u32(t52[0], t54[0]), vandq_u32(t52[1], t54[1])];
                    let t56 = [vdupq_n_u32(2147483648u32); 2];
                    let t57: [uint32x4_t; 2] = [vandq_u32(x72[0], t56[0]), vandq_u32(x72[1], t56[1])];
                    let t58 = [vdupq_n_u32(0u32); 2];
                    let t59: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t57[0], t58[0])), vmvnq_u32(vceqq_u32(t57[1], t58[1]))];
                    let t60: [uint32x4_t; 2] = [vandq_u32(t55[0], t59[0]), vandq_u32(t55[1], t59[1])];
                    let t61 = [vdupq_n_u32(2147483647u32); 2];
                    let t62: [uint32x4_t; 2] = [vandq_u32(x72[0], t61[0]), vandq_u32(x72[1], t61[1])];
                    let t63: [uint32x4_t; 2] = [vcgeq_u32(t62[0], x64[0]), vcgeq_u32(t62[1], x64[1])];
                    let t64: [uint32x4_t; 2] = [vandq_u32(t60[0], t63[0]), vandq_u32(t60[1], t63[1])];
                    t48 = [vandq_u32(t48[0], t64[0]), vandq_u32(t48[1], t64[1])];
                    if !(any(&t48)) { break; }
                    let t66: [uint32x4_t; 2] = [vandq_u32(x72[0], t61[0]), vandq_u32(x72[1], t61[1])];
                    let t67: [uint32x4_t; 2] = [vsubq_u32(t66[0], x64[0]), vsubq_u32(t66[1], x64[1])];
                    let t68 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(t48[0], t67[0]), vandq_u32(t48[1], t67[1]))));
                    let t69: [uint32x4_t; 2] = [vbslq_u32(t48[0], t67[0], t68), vbslq_u32(t48[1], t67[1], t68)];
                    let t70 = arg6 as *const u32;
                    let mut t71: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                    t71[0] = vld1q_lane_u32::<0>(t70.add(vgetq_lane_u32::<0>(t69[0]) as usize), t71[0]);
                    t71[0] = vld1q_lane_u32::<1>(t70.add(vgetq_lane_u32::<1>(t69[0]) as usize), t71[0]);
                    t71[0] = vld1q_lane_u32::<2>(t70.add(vgetq_lane_u32::<2>(t69[0]) as usize), t71[0]);
                    t71[0] = vld1q_lane_u32::<3>(t70.add(vgetq_lane_u32::<3>(t69[0]) as usize), t71[0]);
                    t71[1] = vld1q_lane_u32::<0>(t70.add(vgetq_lane_u32::<0>(t69[1]) as usize), t71[1]);
                    t71[1] = vld1q_lane_u32::<1>(t70.add(vgetq_lane_u32::<1>(t69[1]) as usize), t71[1]);
                    t71[1] = vld1q_lane_u32::<2>(t70.add(vgetq_lane_u32::<2>(t69[1]) as usize), t71[1]);
                    t71[1] = vld1q_lane_u32::<3>(t70.add(vgetq_lane_u32::<3>(t69[1]) as usize), t71[1]);
                    x72 = [vbslq_u32(t48[0], t71[0], x72[0]), vbslq_u32(t48[1], t71[1], x72[1])];
                    let t72: [uint32x4_t; 2] = [vandq_u32(t48[0], live[0]), vandq_u32(t48[1], live[1])];
                    let t73: [uint32x4_t; 2] = [x80[0], x80[1]];
                    let t74 = [vdupq_n_u32(1u32); 2];
                    let t75: [uint32x4_t; 2] = [vaddq_u32(t73[0], t74[0]), vaddq_u32(t73[1], t74[1])];
                    let t76 = x80;
                    let t77: [uint32x4_t; 4] = [t75[0], t75[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                    x80 = t77;
                }
                let t78 = [vdupq_n_u32(4294967295u32); 2];
                let t79: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x72[0], t78[0])), vmvnq_u32(vceqq_u32(x72[1], t78[1]))];
                let t80 = [vdupq_n_u32(2147483648u32); 2];
                let t81: [uint32x4_t; 2] = [vandq_u32(x72[0], t80[0]), vandq_u32(x72[1], t80[1])];
                let t82 = [vdupq_n_u32(0u32); 2];
                let t83: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t81[0], t82[0])), vmvnq_u32(vceqq_u32(t81[1], t82[1]))];
                let t84: [uint32x4_t; 2] = [vandq_u32(t79[0], t83[0]), vandq_u32(t79[1], t83[1])];
                let t85: [uint32x4_t; 2] = [vandq_u32(live[0], t84[0]), vandq_u32(live[1], t84[1])];
                if any(&t85) {
                    let t86 = [vdupq_n_u32(2147483647u32); 2];
                    let t87: [uint32x4_t; 2] = [vandq_u32(x72[0], t86[0]), vandq_u32(x72[1], t86[1])];
                    let t88 = b3_r0r1l_p(l, lid0, t85, arg0, arg1, t87);
                    x72 = [vbslq_u32(t85[0], t88[0], x72[0]), vbslq_u32(t85[1], t88[1], x72[1])];
                }
                let t89 = (arg2 as *mut u32).add((gid0) as usize);
                if all(&live) {
                    vst1q_u32(t89.add(0), x72[0]);
                    vst1q_u32(t89.add(4), x72[1]);
                } else {
                    let mut t90w = 0u32;
                    let t90 = core::hint::black_box(&raw mut t90w);
                    core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t89.add(0), t90).write(vgetq_lane_u32::<0>(x72[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t89.add(1), t90).write(vgetq_lane_u32::<1>(x72[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t89.add(2), t90).write(vgetq_lane_u32::<2>(x72[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t89.add(3), t90).write(vgetq_lane_u32::<3>(x72[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t89.add(4), t90).write(vgetq_lane_u32::<0>(x72[1]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t89.add(5), t90).write(vgetq_lane_u32::<1>(x72[1]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t89.add(6), t90).write(vgetq_lane_u32::<2>(x72[1]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t89.add(7), t90).write(vgetq_lane_u32::<3>(x72[1]));
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
            let t91: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
            let t92: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x56), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x56), vld1q_u32(LANE[1].as_ptr()))];
            let t93: [uint32x4_t; 2] = [vsubq_u32(t91[0], t92[0]), vsubq_u32(t91[1], t92[1])];
            x64 = t93;
            let t94 = [vdupq_n_u32(4294967295u32); 2];
            x72 = t94;
            if (((gid0) as u64 + 7) <= u32::MAX as u64 && ((gid0) as u64) >= ((a0) as u64)) {
            } else if ((gid0) as u64 + 7) < ((a0) as u64) {
                let t95 = b1_r0r1g_f(l, lid0, arg0, arg1);
                x80 = t95;
                let t96: u64 = (1u64).wrapping_shl((32u64) as u32);
                let t97 = [vdupq_n_u32((t96) as u32); 2];
                let t98 = [vdupq_n_u32(((t96) >> 32) as u32); 2];
                let t99: [uint32x4_t; 4] = [vandq_u32(x80[0], t97[0]), vandq_u32(x80[1], t97[1]), vandq_u32(x80[2], t98[0]), vandq_u32(x80[3], t98[1])];
                let t100: u64 = 0u64;
                let t101 = [vdupq_n_u32((t100) as u32); 2];
                let t102: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t99[0], t101[0]), vceqq_u32(t99[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t99[1], t101[1]), vceqq_u32(t99[3], vdupq_n_u32(0)))];
                let t103: [uint32x4_t; 2] = [vmvnq_u32(t102[0]), vmvnq_u32(t102[1])];
                let mut t104: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                let t105: [uint32x4_t; 2] = [vmvnq_u32(t103[0]), vmvnq_u32(t103[1])];
                {
                    let t106: [uint32x4_t; 2] = [x80[0], x80[1]];
                    let t107 = [vdupq_n_u32(2147483648u32); 2];
                    let t108: [uint32x4_t; 2] = [vorrq_u32(t107[0], t106[0]), vorrq_u32(t107[1], t106[1])];
                    t104 = [vbslq_u32(t103[0], t108[0], t104[0]), vbslq_u32(t103[1], t108[1], t104[1])];
                }
                {
                    let t109: [uint32x4_t; 2] = [x80[0], x80[1]];
                    t104 = [vbslq_u32(t105[0], t109[0], t104[0]), vbslq_u32(t105[1], t109[1], t104[1])];
                }
                x72 = t104;
            } else {
                // Lanes part: the chunk's lanes on both sides of the bound.
                let t110 = [vdupq_n_u32(a0); 2];
                let t111: [uint32x4_t; 2] = [vcltq_u32(t91[0], t110[0]), vcltq_u32(t91[1], t110[1])];
                let t112: [uint32x4_t; 2] = [vandq_u32(live[0], t111[0]), vandq_u32(live[1], t111[1])];
                if any(&t112) {
                    let t113 = b1_r0r1g_p(l, lid0, t112, arg0, arg1);
                    x80 = [vbslq_u32(t112[0], t113[0], x80[0]), vbslq_u32(t112[1], t113[1], x80[1]), vbslq_u32(t112[0], t113[2], x80[2]), vbslq_u32(t112[1], t113[3], x80[3])];
                    let t114: u64 = (1u64).wrapping_shl((32u64) as u32);
                    let t115 = [vdupq_n_u32((t114) as u32); 2];
                    let t116 = [vdupq_n_u32(((t114) >> 32) as u32); 2];
                    let t117: [uint32x4_t; 4] = [vandq_u32(x80[0], t115[0]), vandq_u32(x80[1], t115[1]), vandq_u32(x80[2], t116[0]), vandq_u32(x80[3], t116[1])];
                    let t118: u64 = 0u64;
                    let t119 = [vdupq_n_u32((t118) as u32); 2];
                    let t120: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t117[0], t119[0]), vceqq_u32(t117[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t117[1], t119[1]), vceqq_u32(t117[3], vdupq_n_u32(0)))];
                    let t121: [uint32x4_t; 2] = [vmvnq_u32(t120[0]), vmvnq_u32(t120[1])];
                    let mut t122: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                    let t123: [uint32x4_t; 2] = [vandq_u32(t112[0], t121[0]), vandq_u32(t112[1], t121[1])];
                    let t124: [uint32x4_t; 2] = [vbicq_u32(t112[0], t121[0]), vbicq_u32(t112[1], t121[1])];
                    {
                        let t125: [uint32x4_t; 2] = [x80[0], x80[1]];
                        let t126 = [vdupq_n_u32(2147483648u32); 2];
                        let t127: [uint32x4_t; 2] = [vorrq_u32(t126[0], t125[0]), vorrq_u32(t126[1], t125[1])];
                        t122 = [vbslq_u32(t123[0], t127[0], t122[0]), vbslq_u32(t123[1], t127[1], t122[1])];
                    }
                    {
                        let t128: [uint32x4_t; 2] = [x80[0], x80[1]];
                        t122 = [vbslq_u32(t124[0], t128[0], t122[0]), vbslq_u32(t124[1], t128[1], t122[1])];
                    }
                    x72 = [vbslq_u32(t112[0], t122[0], x72[0]), vbslq_u32(t112[1], t122[1], x72[1])];
                }
            }
            let t129 = (arg6 as *mut u32).add((x56) as usize);
            vst1q_u32(t129.add(0), x72[0]);
            vst1q_u32(t129.add(4), x72[1]);
        }
    }
    if phase == 1 {
        'phase: {
            let t130: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
            let t131 = [vdupq_n_u32(a0); 2];
            let t132: [uint32x4_t; 2] = [vcgeq_u32(t130[0], t131[0]), vcgeq_u32(t130[1], t131[1])];
            let t133: [uint32x4_t; 2] = [vandq_u32(live[0], t132[0]), vandq_u32(live[1], t132[1])];
            {
                live = [vbicq_u32(live[0], t133[0]), vbicq_u32(live[1], t133[1])];
            }
            if any(&live) {
                let t134: u64 = ((0u32) as u64);
                let t135 = [vdupq_n_u32((t134) as u32); 2];
                let t136: [uint32x4_t; 4] = [t135[0], t135[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                x80 = t136;
                let mut t152 = live;
                loop {
                    // Lanes part: each its own rounds.
                    let t154: [uint32x4_t; 2] = [x80[0], x80[1]];
                    let t155 = [vdupq_n_u32(7u32); 2];
                    let t156: [uint32x4_t; 2] = [vcltq_u32(t154[0], t155[0]), vcltq_u32(t154[1], t155[1])];
                    let t157 = [vdupq_n_u32(4294967295u32); 2];
                    let t158: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x72[0], t157[0])), vmvnq_u32(vceqq_u32(x72[1], t157[1]))];
                    let t159: [uint32x4_t; 2] = [vandq_u32(t156[0], t158[0]), vandq_u32(t156[1], t158[1])];
                    let t160 = [vdupq_n_u32(2147483648u32); 2];
                    let t161: [uint32x4_t; 2] = [vandq_u32(x72[0], t160[0]), vandq_u32(x72[1], t160[1])];
                    let t162 = [vdupq_n_u32(0u32); 2];
                    let t163: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t161[0], t162[0])), vmvnq_u32(vceqq_u32(t161[1], t162[1]))];
                    let t164: [uint32x4_t; 2] = [vandq_u32(t159[0], t163[0]), vandq_u32(t159[1], t163[1])];
                    let t165 = [vdupq_n_u32(2147483647u32); 2];
                    let t166: [uint32x4_t; 2] = [vandq_u32(x72[0], t165[0]), vandq_u32(x72[1], t165[1])];
                    let t167: [uint32x4_t; 2] = [vcgeq_u32(t166[0], x64[0]), vcgeq_u32(t166[1], x64[1])];
                    let t168: [uint32x4_t; 2] = [vandq_u32(t164[0], t167[0]), vandq_u32(t164[1], t167[1])];
                    t152 = [vandq_u32(t152[0], t168[0]), vandq_u32(t152[1], t168[1])];
                    if !(any(&t152)) { break; }
                    let t170: [uint32x4_t; 2] = [vandq_u32(x72[0], t165[0]), vandq_u32(x72[1], t165[1])];
                    let t171: [uint32x4_t; 2] = [vsubq_u32(t170[0], x64[0]), vsubq_u32(t170[1], x64[1])];
                    let t172 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(t152[0], t171[0]), vandq_u32(t152[1], t171[1]))));
                    let t173: [uint32x4_t; 2] = [vbslq_u32(t152[0], t171[0], t172), vbslq_u32(t152[1], t171[1], t172)];
                    let t174 = arg6 as *const u32;
                    let mut t175: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                    t175[0] = vld1q_lane_u32::<0>(t174.add(vgetq_lane_u32::<0>(t173[0]) as usize), t175[0]);
                    t175[0] = vld1q_lane_u32::<1>(t174.add(vgetq_lane_u32::<1>(t173[0]) as usize), t175[0]);
                    t175[0] = vld1q_lane_u32::<2>(t174.add(vgetq_lane_u32::<2>(t173[0]) as usize), t175[0]);
                    t175[0] = vld1q_lane_u32::<3>(t174.add(vgetq_lane_u32::<3>(t173[0]) as usize), t175[0]);
                    t175[1] = vld1q_lane_u32::<0>(t174.add(vgetq_lane_u32::<0>(t173[1]) as usize), t175[1]);
                    t175[1] = vld1q_lane_u32::<1>(t174.add(vgetq_lane_u32::<1>(t173[1]) as usize), t175[1]);
                    t175[1] = vld1q_lane_u32::<2>(t174.add(vgetq_lane_u32::<2>(t173[1]) as usize), t175[1]);
                    t175[1] = vld1q_lane_u32::<3>(t174.add(vgetq_lane_u32::<3>(t173[1]) as usize), t175[1]);
                    x72 = [vbslq_u32(t152[0], t175[0], x72[0]), vbslq_u32(t152[1], t175[1], x72[1])];
                    let t176: [uint32x4_t; 2] = [vandq_u32(t152[0], live[0]), vandq_u32(t152[1], live[1])];
                    let t177: [uint32x4_t; 2] = [x80[0], x80[1]];
                    let t178 = [vdupq_n_u32(1u32); 2];
                    let t179: [uint32x4_t; 2] = [vaddq_u32(t177[0], t178[0]), vaddq_u32(t177[1], t178[1])];
                    let t180 = x80;
                    let t181: [uint32x4_t; 4] = [t179[0], t179[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                    x80 = t181;
                }
                let t182 = [vdupq_n_u32(4294967295u32); 2];
                let t183: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x72[0], t182[0])), vmvnq_u32(vceqq_u32(x72[1], t182[1]))];
                let t184 = [vdupq_n_u32(2147483648u32); 2];
                let t185: [uint32x4_t; 2] = [vandq_u32(x72[0], t184[0]), vandq_u32(x72[1], t184[1])];
                let t186 = [vdupq_n_u32(0u32); 2];
                let t187: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t185[0], t186[0])), vmvnq_u32(vceqq_u32(t185[1], t186[1]))];
                let t188: [uint32x4_t; 2] = [vandq_u32(t183[0], t187[0]), vandq_u32(t183[1], t187[1])];
                let t189: [uint32x4_t; 2] = [vandq_u32(live[0], t188[0]), vandq_u32(live[1], t188[1])];
                if any(&t189) {
                    let t190 = [vdupq_n_u32(2147483647u32); 2];
                    let t191: [uint32x4_t; 2] = [vandq_u32(x72[0], t190[0]), vandq_u32(x72[1], t190[1])];
                    let t192 = b3_r0r1l_p(l, lid0, t189, arg0, arg1, t191);
                    x72 = [vbslq_u32(t189[0], t192[0], x72[0]), vbslq_u32(t189[1], t192[1], x72[1])];
                }
                let t193 = (arg2 as *mut u32).add((gid0) as usize);
                if all(&live) {
                    vst1q_u32(t193.add(0), x72[0]);
                    vst1q_u32(t193.add(4), x72[1]);
                } else {
                    let mut t194w = 0u32;
                    let t194 = core::hint::black_box(&raw mut t194w);
                    core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t193.add(0), t194).write(vgetq_lane_u32::<0>(x72[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t193.add(1), t194).write(vgetq_lane_u32::<1>(x72[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t193.add(2), t194).write(vgetq_lane_u32::<2>(x72[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t193.add(3), t194).write(vgetq_lane_u32::<3>(x72[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t193.add(4), t194).write(vgetq_lane_u32::<0>(x72[1]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t193.add(5), t194).write(vgetq_lane_u32::<1>(x72[1]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t193.add(6), t194).write(vgetq_lane_u32::<2>(x72[1]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t193.add(7), t194).write(vgetq_lane_u32::<3>(x72[1]));
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
    let t195: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
    let t196 = [vdupq_n_u32(0u32); 2];
    let t197: [uint32x4_t; 2] = [vceqq_u32(t195[0], t196[0]), vceqq_u32(t195[1], t196[1])];
    let t198: [uint32x4_t; 2] = [vandq_u32(live[0], t197[0]), vandq_u32(live[1], t197[1])];
    {
        let t199: u64 = 4294967295u64;
        let t200 = [vdupq_n_u32((t199) as u32); 2];
        let t201: [uint32x4_t; 4] = [t200[0], t200[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = [vbslq_u32(t198[0], t201[0], res[0]), vbslq_u32(t198[1], t201[1], res[1]), vbslq_u32(t198[0], t201[2], res[2]), vbslq_u32(t198[1], t201[3], res[3])];
        live = [vbicq_u32(live[0], t198[0]), vbicq_u32(live[1], t198[1])];
    }
    if any(&live) {
        x24 = gid0.wrapping_sub(1u32);
        let t202 = if (x24 & 255u32) <= 248u32 { b4_r0r1u8_p(l, lid0, live, arg0, arg1, x24) } else { b4_r0r1i_p(l, lid0, live, arg0, arg1, x24) };
        x32 = t202;
        let t203: u64 = 1u64;
        let t204 = [vdupq_n_u32((t203) as u32); 2];
        let t205: [uint32x4_t; 2] = [vandq_u32(x32[0], t204[0]), vandq_u32(x32[1], t204[1])];
        let t206: u64 = 0u64;
        let t207 = [vdupq_n_u32((t206) as u32); 2];
        let t208: [uint32x4_t; 2] = [vceqq_u32(t205[0], t207[0]), vceqq_u32(t205[1], t207[1])];
        let t209: [uint32x4_t; 2] = [vandq_u32(live[0], t208[0]), vandq_u32(live[1], t208[1])];
        {
            let t210: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[1].as_ptr()))];
            let t211: [uint32x4_t; 4] = [t210[0], t210[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t209[0], t211[0], res[0]), vbslq_u32(t209[1], t211[1], res[1]), vbslq_u32(t209[0], t211[2], res[2]), vbslq_u32(t209[1], t211[3], res[3])];
            live = [vbicq_u32(live[0], t209[0]), vbicq_u32(live[1], t209[1])];
        }
        if any(&live) {
            let t212: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x32[0]), vshlq_n_u32::<30>(x32[2])), vorrq_u32(vshrq_n_u32::<2>(x32[1]), vshlq_n_u32::<30>(x32[3])), vshrq_n_u32::<2>(x32[2]), vshrq_n_u32::<2>(x32[3])];
            let t213: [uint32x4_t; 2] = [t212[0], t212[1]];
            x40 = t213;
            let t214 = [vdupq_n_u32(1u32); 2];
            let t215: [uint32x4_t; 2] = [vsubq_u32(x40[0], t214[0]), vsubq_u32(x40[1], t214[1])];
            let t216: [uint32x4_t; 2] = [vcgeq_u32(t215[0], t195[0]), vcgeq_u32(t215[1], t195[1])];
            let t217: [uint32x4_t; 2] = [vandq_u32(live[0], t216[0]), vandq_u32(live[1], t216[1])];
            {
                let t218: u64 = 4294967295u64;
                let t219 = [vdupq_n_u32((t218) as u32); 2];
                let t220: [uint32x4_t; 4] = [t219[0], t219[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t217[0], t220[0], res[0]), vbslq_u32(t217[1], t220[1], res[1]), vbslq_u32(t217[0], t220[2], res[2]), vbslq_u32(t217[1], t220[3], res[3])];
                live = [vbicq_u32(live[0], t217[0]), vbicq_u32(live[1], t217[1])];
            }
            if any(&live) {
                let t221: [uint32x4_t; 2] = [vsubq_u32(t195[0], x40[0]), vsubq_u32(t195[1], x40[1])];
                x48 = t221;
                let t222: u64 = 2u64;
                let t223 = [vdupq_n_u32((t222) as u32); 2];
                let t224: [uint32x4_t; 2] = [vandq_u32(x32[0], t223[0]), vandq_u32(x32[1], t223[1])];
                let t225: u64 = 0u64;
                let t226 = [vdupq_n_u32((t225) as u32); 2];
                let t227: [uint32x4_t; 2] = [vceqq_u32(t224[0], t226[0]), vceqq_u32(t224[1], t226[1])];
                let t228: [uint32x4_t; 2] = [vmvnq_u32(t227[0]), vmvnq_u32(t227[1])];
                let t229: [uint32x4_t; 2] = [vandq_u32(live[0], t228[0]), vandq_u32(live[1], t228[1])];
                {
                    let t230: [uint32x4_t; 4] = [x48[0], x48[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                    res = [vbslq_u32(t229[0], t230[0], res[0]), vbslq_u32(t229[1], t230[1], res[1]), vbslq_u32(t229[0], t230[2], res[2]), vbslq_u32(t229[1], t230[3], res[3])];
                    live = [vbicq_u32(live[0], t229[0]), vbicq_u32(live[1], t229[1])];
                }
                if any(&live) {
                    let t231: [uint32x4_t; 2] = [vceqq_u32(x48[0], t196[0]), vceqq_u32(x48[1], t196[1])];
                    let t232: [uint32x4_t; 2] = [vandq_u32(live[0], t231[0]), vandq_u32(live[1], t231[1])];
                    {
                        let t233: u64 = 4294967295u64;
                        let t234 = [vdupq_n_u32((t233) as u32); 2];
                        let t235: [uint32x4_t; 4] = [t234[0], t234[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                        res = [vbslq_u32(t232[0], t235[0], res[0]), vbslq_u32(t232[1], t235[1], res[1]), vbslq_u32(t232[0], t235[2], res[2]), vbslq_u32(t232[1], t235[3], res[3])];
                        live = [vbicq_u32(live[0], t232[0]), vbicq_u32(live[1], t232[1])];
                    }
                    if any(&live) {
                        let t236: u64 = (1u64).wrapping_shl((32u64) as u32);
                        let t237 = [vdupq_n_u32((t236) as u32); 2];
                        let t238 = [vdupq_n_u32(((t236) >> 32) as u32); 2];
                        let t239: [uint32x4_t; 4] = [vorrq_u32(t237[0], x48[0]), vorrq_u32(t237[1], x48[1]), t238[0], t238[1]];
                        res = [vbslq_u32(live[0], t239[0], res[0]), vbslq_u32(live[1], t239[1], res[1]), vbslq_u32(live[0], t239[2], res[2]), vbslq_u32(live[1], t239[3], res[3])];
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
        let t240 = if (x24 & 255u32) <= 248u32 { b4_r0r1u8_f(l, lid0, arg0, arg1, x24) } else { b4_r0r1i_f(l, lid0, arg0, arg1, x24) };
        x32 = t240;
        let t241: u64 = 1u64;
        let t242 = [vdupq_n_u32((t241) as u32); 2];
        let t243: [uint32x4_t; 2] = [vandq_u32(x32[0], t242[0]), vandq_u32(x32[1], t242[1])];
        let t244: u64 = 0u64;
        let t245 = [vdupq_n_u32((t244) as u32); 2];
        let t246: [uint32x4_t; 2] = [vceqq_u32(t243[0], t245[0]), vceqq_u32(t243[1], t245[1])];
        {
            let t247: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[1].as_ptr()))];
            let t248: [uint32x4_t; 4] = [t247[0], t247[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t246[0], t248[0], res[0]), vbslq_u32(t246[1], t248[1], res[1]), vbslq_u32(t246[0], t248[2], res[2]), vbslq_u32(t246[1], t248[3], res[3])];
            live = [vbicq_u32(live[0], t246[0]), vbicq_u32(live[1], t246[1])];
        }
        if any(&live) {
            let t249: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x32[0]), vshlq_n_u32::<30>(x32[2])), vorrq_u32(vshrq_n_u32::<2>(x32[1]), vshlq_n_u32::<30>(x32[3])), vshrq_n_u32::<2>(x32[2]), vshrq_n_u32::<2>(x32[3])];
            let t250: [uint32x4_t; 2] = [t249[0], t249[1]];
            x40 = t250;
            let t251 = [vdupq_n_u32(1u32); 2];
            let t252: [uint32x4_t; 2] = [vsubq_u32(x40[0], t251[0]), vsubq_u32(x40[1], t251[1])];
            let t253: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
            let t254: [uint32x4_t; 2] = [vcgeq_u32(t252[0], t253[0]), vcgeq_u32(t252[1], t253[1])];
            let t255: [uint32x4_t; 2] = [vandq_u32(live[0], t254[0]), vandq_u32(live[1], t254[1])];
            {
                let t256: u64 = 4294967295u64;
                let t257 = [vdupq_n_u32((t256) as u32); 2];
                let t258: [uint32x4_t; 4] = [t257[0], t257[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t255[0], t258[0], res[0]), vbslq_u32(t255[1], t258[1], res[1]), vbslq_u32(t255[0], t258[2], res[2]), vbslq_u32(t255[1], t258[3], res[3])];
                live = [vbicq_u32(live[0], t255[0]), vbicq_u32(live[1], t255[1])];
            }
            if any(&live) {
                let t259: [uint32x4_t; 2] = [vsubq_u32(t253[0], x40[0]), vsubq_u32(t253[1], x40[1])];
                x48 = t259;
                let t260: u64 = 2u64;
                let t261 = [vdupq_n_u32((t260) as u32); 2];
                let t262: [uint32x4_t; 2] = [vandq_u32(x32[0], t261[0]), vandq_u32(x32[1], t261[1])];
                let t263: u64 = 0u64;
                let t264 = [vdupq_n_u32((t263) as u32); 2];
                let t265: [uint32x4_t; 2] = [vceqq_u32(t262[0], t264[0]), vceqq_u32(t262[1], t264[1])];
                let t266: [uint32x4_t; 2] = [vmvnq_u32(t265[0]), vmvnq_u32(t265[1])];
                let t267: [uint32x4_t; 2] = [vandq_u32(live[0], t266[0]), vandq_u32(live[1], t266[1])];
                {
                    let t268: [uint32x4_t; 4] = [x48[0], x48[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                    res = [vbslq_u32(t267[0], t268[0], res[0]), vbslq_u32(t267[1], t268[1], res[1]), vbslq_u32(t267[0], t268[2], res[2]), vbslq_u32(t267[1], t268[3], res[3])];
                    live = [vbicq_u32(live[0], t267[0]), vbicq_u32(live[1], t267[1])];
                }
                if any(&live) {
                    let t269 = [vdupq_n_u32(0u32); 2];
                    let t270: [uint32x4_t; 2] = [vceqq_u32(x48[0], t269[0]), vceqq_u32(x48[1], t269[1])];
                    let t271: [uint32x4_t; 2] = [vandq_u32(live[0], t270[0]), vandq_u32(live[1], t270[1])];
                    {
                        let t272: u64 = 4294967295u64;
                        let t273 = [vdupq_n_u32((t272) as u32); 2];
                        let t274: [uint32x4_t; 4] = [t273[0], t273[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                        res = [vbslq_u32(t271[0], t274[0], res[0]), vbslq_u32(t271[1], t274[1], res[1]), vbslq_u32(t271[0], t274[2], res[2]), vbslq_u32(t271[1], t274[3], res[3])];
                        live = [vbicq_u32(live[0], t271[0]), vbicq_u32(live[1], t271[1])];
                    }
                    if any(&live) {
                        let t275: u64 = (1u64).wrapping_shl((32u64) as u32);
                        let t276 = [vdupq_n_u32((t275) as u32); 2];
                        let t277 = [vdupq_n_u32(((t275) >> 32) as u32); 2];
                        let t278: [uint32x4_t; 4] = [vorrq_u32(t276[0], x48[0]), vorrq_u32(t276[1], x48[1]), t277[0], t277[1]];
                        res = [vbslq_u32(live[0], t278[0], res[0]), vbslq_u32(live[1], t278[1], res[1]), vbslq_u32(live[0], t278[2], res[2]), vbslq_u32(live[1], t278[3], res[3])];
                        live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
                    }
                }
            }
        }
    } else if false {
        let t279: u64 = 4294967295u64;
        let t280 = [vdupq_n_u32((t279) as u32); 2];
        let t281: [uint32x4_t; 4] = [t280[0], t280[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t281;
        return res;
    } else {
        // Lanes part: the chunk's lanes on both sides of the bound.
        let t282: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t283 = [vdupq_n_u32(0u32); 2];
        let t284: [uint32x4_t; 2] = [vceqq_u32(t282[0], t283[0]), vceqq_u32(t282[1], t283[1])];
        let t285: [uint32x4_t; 2] = [vandq_u32(live[0], t284[0]), vandq_u32(live[1], t284[1])];
        {
            let t286: u64 = 4294967295u64;
            let t287 = [vdupq_n_u32((t286) as u32); 2];
            let t288: [uint32x4_t; 4] = [t287[0], t287[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t285[0], t288[0], res[0]), vbslq_u32(t285[1], t288[1], res[1]), vbslq_u32(t285[0], t288[2], res[2]), vbslq_u32(t285[1], t288[3], res[3])];
            live = [vbicq_u32(live[0], t285[0]), vbicq_u32(live[1], t285[1])];
        }
        if any(&live) {
            x24 = gid0.wrapping_sub(1u32);
            let t289 = if (x24 & 255u32) <= 248u32 { b4_r0r1u8_p(l, lid0, live, arg0, arg1, x24) } else { b4_r0r1i_p(l, lid0, live, arg0, arg1, x24) };
            x32 = t289;
            let t290: u64 = 1u64;
            let t291 = [vdupq_n_u32((t290) as u32); 2];
            let t292: [uint32x4_t; 2] = [vandq_u32(x32[0], t291[0]), vandq_u32(x32[1], t291[1])];
            let t293: u64 = 0u64;
            let t294 = [vdupq_n_u32((t293) as u32); 2];
            let t295: [uint32x4_t; 2] = [vceqq_u32(t292[0], t294[0]), vceqq_u32(t292[1], t294[1])];
            let t296: [uint32x4_t; 2] = [vandq_u32(live[0], t295[0]), vandq_u32(live[1], t295[1])];
            {
                let t297: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[1].as_ptr()))];
                let t298: [uint32x4_t; 4] = [t297[0], t297[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t296[0], t298[0], res[0]), vbslq_u32(t296[1], t298[1], res[1]), vbslq_u32(t296[0], t298[2], res[2]), vbslq_u32(t296[1], t298[3], res[3])];
                live = [vbicq_u32(live[0], t296[0]), vbicq_u32(live[1], t296[1])];
            }
            if any(&live) {
                let t299: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x32[0]), vshlq_n_u32::<30>(x32[2])), vorrq_u32(vshrq_n_u32::<2>(x32[1]), vshlq_n_u32::<30>(x32[3])), vshrq_n_u32::<2>(x32[2]), vshrq_n_u32::<2>(x32[3])];
                let t300: [uint32x4_t; 2] = [t299[0], t299[1]];
                x40 = t300;
                let t301 = [vdupq_n_u32(1u32); 2];
                let t302: [uint32x4_t; 2] = [vsubq_u32(x40[0], t301[0]), vsubq_u32(x40[1], t301[1])];
                let t303: [uint32x4_t; 2] = [vcgeq_u32(t302[0], t282[0]), vcgeq_u32(t302[1], t282[1])];
                let t304: [uint32x4_t; 2] = [vandq_u32(live[0], t303[0]), vandq_u32(live[1], t303[1])];
                {
                    let t305: u64 = 4294967295u64;
                    let t306 = [vdupq_n_u32((t305) as u32); 2];
                    let t307: [uint32x4_t; 4] = [t306[0], t306[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                    res = [vbslq_u32(t304[0], t307[0], res[0]), vbslq_u32(t304[1], t307[1], res[1]), vbslq_u32(t304[0], t307[2], res[2]), vbslq_u32(t304[1], t307[3], res[3])];
                    live = [vbicq_u32(live[0], t304[0]), vbicq_u32(live[1], t304[1])];
                }
                if any(&live) {
                    let t308: [uint32x4_t; 2] = [vsubq_u32(t282[0], x40[0]), vsubq_u32(t282[1], x40[1])];
                    x48 = t308;
                    let t309: u64 = 2u64;
                    let t310 = [vdupq_n_u32((t309) as u32); 2];
                    let t311: [uint32x4_t; 2] = [vandq_u32(x32[0], t310[0]), vandq_u32(x32[1], t310[1])];
                    let t312: u64 = 0u64;
                    let t313 = [vdupq_n_u32((t312) as u32); 2];
                    let t314: [uint32x4_t; 2] = [vceqq_u32(t311[0], t313[0]), vceqq_u32(t311[1], t313[1])];
                    let t315: [uint32x4_t; 2] = [vmvnq_u32(t314[0]), vmvnq_u32(t314[1])];
                    let t316: [uint32x4_t; 2] = [vandq_u32(live[0], t315[0]), vandq_u32(live[1], t315[1])];
                    {
                        let t317: [uint32x4_t; 4] = [x48[0], x48[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                        res = [vbslq_u32(t316[0], t317[0], res[0]), vbslq_u32(t316[1], t317[1], res[1]), vbslq_u32(t316[0], t317[2], res[2]), vbslq_u32(t316[1], t317[3], res[3])];
                        live = [vbicq_u32(live[0], t316[0]), vbicq_u32(live[1], t316[1])];
                    }
                    if any(&live) {
                        let t318: [uint32x4_t; 2] = [vceqq_u32(x48[0], t283[0]), vceqq_u32(x48[1], t283[1])];
                        let t319: [uint32x4_t; 2] = [vandq_u32(live[0], t318[0]), vandq_u32(live[1], t318[1])];
                        {
                            let t320: u64 = 4294967295u64;
                            let t321 = [vdupq_n_u32((t320) as u32); 2];
                            let t322: [uint32x4_t; 4] = [t321[0], t321[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                            res = [vbslq_u32(t319[0], t322[0], res[0]), vbslq_u32(t319[1], t322[1], res[1]), vbslq_u32(t319[0], t322[2], res[2]), vbslq_u32(t319[1], t322[3], res[3])];
                            live = [vbicq_u32(live[0], t319[0]), vbicq_u32(live[1], t319[1])];
                        }
                        if any(&live) {
                            let t323: u64 = (1u64).wrapping_shl((32u64) as u32);
                            let t324 = [vdupq_n_u32((t323) as u32); 2];
                            let t325 = [vdupq_n_u32(((t323) >> 32) as u32); 2];
                            let t326: [uint32x4_t; 4] = [vorrq_u32(t324[0], x48[0]), vorrq_u32(t324[1], x48[1]), t325[0], t325[1]];
                            res = [vbslq_u32(live[0], t326[0], res[0]), vbslq_u32(live[1], t326[1], res[1]), vbslq_u32(live[0], t326[2], res[2]), vbslq_u32(live[1], t326[3], res[3])];
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
    let t327 = [vdupq_n_u32(0u32); 2];
    let t328: [uint32x4_t; 2] = [vceqq_u32(x16[0], t327[0]), vceqq_u32(x16[1], t327[1])];
    let t329: [uint32x4_t; 2] = [vandq_u32(live[0], t328[0]), vandq_u32(live[1], t328[1])];
    {
        let t330: u64 = 4294967295u64;
        let t331 = [vdupq_n_u32((t330) as u32); 2];
        let t332: [uint32x4_t; 4] = [t331[0], t331[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = [vbslq_u32(t329[0], t332[0], res[0]), vbslq_u32(t329[1], t332[1], res[1]), vbslq_u32(t329[0], t332[2], res[2]), vbslq_u32(t329[1], t332[3], res[3])];
        live = [vbicq_u32(live[0], t329[0]), vbicq_u32(live[1], t329[1])];
    }
    if any(&live) {
        let t333 = [vdupq_n_u32(1u32); 2];
        let t334: [uint32x4_t; 2] = [vsubq_u32(x16[0], t333[0]), vsubq_u32(x16[1], t333[1])];
        x24 = t334;
        let t335 = b4_r0r1l_p(l, lid0, live, arg0, arg1, x24);
        x32 = t335;
        let t336: u64 = 1u64;
        let t337 = [vdupq_n_u32((t336) as u32); 2];
        let t338: [uint32x4_t; 2] = [vandq_u32(x32[0], t337[0]), vandq_u32(x32[1], t337[1])];
        let t339: u64 = 0u64;
        let t340 = [vdupq_n_u32((t339) as u32); 2];
        let t341: [uint32x4_t; 2] = [vceqq_u32(t338[0], t340[0]), vceqq_u32(t338[1], t340[1])];
        let t342: [uint32x4_t; 2] = [vandq_u32(live[0], t341[0]), vandq_u32(live[1], t341[1])];
        {
            let t343: [uint32x4_t; 4] = [x24[0], x24[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t342[0], t343[0], res[0]), vbslq_u32(t342[1], t343[1], res[1]), vbslq_u32(t342[0], t343[2], res[2]), vbslq_u32(t342[1], t343[3], res[3])];
            live = [vbicq_u32(live[0], t342[0]), vbicq_u32(live[1], t342[1])];
        }
        if any(&live) {
            let t344: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x32[0]), vshlq_n_u32::<30>(x32[2])), vorrq_u32(vshrq_n_u32::<2>(x32[1]), vshlq_n_u32::<30>(x32[3])), vshrq_n_u32::<2>(x32[2]), vshrq_n_u32::<2>(x32[3])];
            let t345: [uint32x4_t; 2] = [t344[0], t344[1]];
            x40 = t345;
            let t346: [uint32x4_t; 2] = [vsubq_u32(x40[0], t333[0]), vsubq_u32(x40[1], t333[1])];
            let t347: [uint32x4_t; 2] = [vcgeq_u32(t346[0], x16[0]), vcgeq_u32(t346[1], x16[1])];
            let t348: [uint32x4_t; 2] = [vandq_u32(live[0], t347[0]), vandq_u32(live[1], t347[1])];
            {
                let t349: u64 = 4294967295u64;
                let t350 = [vdupq_n_u32((t349) as u32); 2];
                let t351: [uint32x4_t; 4] = [t350[0], t350[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t348[0], t351[0], res[0]), vbslq_u32(t348[1], t351[1], res[1]), vbslq_u32(t348[0], t351[2], res[2]), vbslq_u32(t348[1], t351[3], res[3])];
                live = [vbicq_u32(live[0], t348[0]), vbicq_u32(live[1], t348[1])];
            }
            if any(&live) {
                let t352: [uint32x4_t; 2] = [vsubq_u32(x16[0], x40[0]), vsubq_u32(x16[1], x40[1])];
                x48 = t352;
                let t353: u64 = 2u64;
                let t354 = [vdupq_n_u32((t353) as u32); 2];
                let t355: [uint32x4_t; 2] = [vandq_u32(x32[0], t354[0]), vandq_u32(x32[1], t354[1])];
                let t356: u64 = 0u64;
                let t357 = [vdupq_n_u32((t356) as u32); 2];
                let t358: [uint32x4_t; 2] = [vceqq_u32(t355[0], t357[0]), vceqq_u32(t355[1], t357[1])];
                let t359: [uint32x4_t; 2] = [vmvnq_u32(t358[0]), vmvnq_u32(t358[1])];
                let t360: [uint32x4_t; 2] = [vandq_u32(live[0], t359[0]), vandq_u32(live[1], t359[1])];
                {
                    let t361: [uint32x4_t; 4] = [x48[0], x48[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                    res = [vbslq_u32(t360[0], t361[0], res[0]), vbslq_u32(t360[1], t361[1], res[1]), vbslq_u32(t360[0], t361[2], res[2]), vbslq_u32(t360[1], t361[3], res[3])];
                    live = [vbicq_u32(live[0], t360[0]), vbicq_u32(live[1], t360[1])];
                }
                if any(&live) {
                    let t362: [uint32x4_t; 2] = [vceqq_u32(x48[0], t327[0]), vceqq_u32(x48[1], t327[1])];
                    let t363: [uint32x4_t; 2] = [vandq_u32(live[0], t362[0]), vandq_u32(live[1], t362[1])];
                    {
                        let t364: u64 = 4294967295u64;
                        let t365 = [vdupq_n_u32((t364) as u32); 2];
                        let t366: [uint32x4_t; 4] = [t365[0], t365[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                        res = [vbslq_u32(t363[0], t366[0], res[0]), vbslq_u32(t363[1], t366[1], res[1]), vbslq_u32(t363[0], t366[2], res[2]), vbslq_u32(t363[1], t366[3], res[3])];
                        live = [vbicq_u32(live[0], t363[0]), vbicq_u32(live[1], t363[1])];
                    }
                    if any(&live) {
                        let t367: u64 = (1u64).wrapping_shl((32u64) as u32);
                        let t368 = [vdupq_n_u32((t367) as u32); 2];
                        let t369 = [vdupq_n_u32(((t367) >> 32) as u32); 2];
                        let t370: [uint32x4_t; 4] = [vorrq_u32(t368[0], x48[0]), vorrq_u32(t368[1], x48[1]), t369[0], t369[1]];
                        res = [vbslq_u32(live[0], t370[0], res[0]), vbslq_u32(live[1], t370[1], res[1]), vbslq_u32(live[0], t370[2], res[2]), vbslq_u32(live[1], t370[3], res[3])];
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
    let t371 = b1_r0r1l_p(l, lid0, live, arg0, arg1, x16);
    x24 = t371;
    let mut t380 = live;
    loop {
        // Lanes part: each its own rounds.
        let t382: u64 = (1u64).wrapping_shl((32u64) as u32);
        let t383 = [vdupq_n_u32((t382) as u32); 2];
        let t384 = [vdupq_n_u32(((t382) >> 32) as u32); 2];
        let t385: [uint32x4_t; 4] = [vandq_u32(x24[0], t383[0]), vandq_u32(x24[1], t383[1]), vandq_u32(x24[2], t384[0]), vandq_u32(x24[3], t384[1])];
        let t386: u64 = 0u64;
        let t387 = [vdupq_n_u32((t386) as u32); 2];
        let t388: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t385[0], t387[0]), vceqq_u32(t385[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t385[1], t387[1]), vceqq_u32(t385[3], vdupq_n_u32(0)))];
        let t389: [uint32x4_t; 2] = [vmvnq_u32(t388[0]), vmvnq_u32(t388[1])];
        t380 = [vandq_u32(t380[0], t389[0]), vandq_u32(t380[1], t389[1])];
        if !(any(&t380)) { break; }
        let t391: [uint32x4_t; 2] = [x24[0], x24[1]];
        let t392 = b1_r0r1l_p(l, lid0, t380, arg0, arg1, t391);
        x24 = [vbslq_u32(t380[0], t392[0], x24[0]), vbslq_u32(t380[1], t392[1], x24[1]), vbslq_u32(t380[0], t392[2], x24[2]), vbslq_u32(t380[1], t392[3], x24[3])];
    }
    let t393: [uint32x4_t; 2] = [x24[0], x24[1]];
    res = t393;
    return res;
}

#[inline(always)]
unsafe fn b4_r0r1i_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t394 = b5_r0r1si_p(l, lid0, live, arg0, arg1, 0u32, x16);
    res = t394;
    return res;
}

#[inline(always)]
unsafe fn b4_r0r1i_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t395 = b5_r0r1si_f(l, lid0, arg0, arg1, 0u32, x16);
    res = t395;
    return res;
}

#[inline(always)]
unsafe fn b4_r0r1u8_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t396 = b5_r0r1su8_p(l, lid0, live, arg0, arg1, 0u32, x16);
    res = t396;
    return res;
}

#[inline(always)]
unsafe fn b4_r0r1u8_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t397 = b5_r0r1su8_f(l, lid0, arg0, arg1, 0u32, x16);
    res = t397;
    return res;
}

#[inline(always)]
unsafe fn b4_r0r1l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t398 = b5_r0r1sl_p(l, lid0, live, arg0, arg1, 0u32, x16);
    res = t398;
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
    let t399 = b6_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    x32 = t399;
    let t400: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[1].as_ptr()))];
    let t401: [uint32x4_t; 2] = [vshlq_u32(t400[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(t400[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t402 = b7_r0r1sl_p(l, lid0, live, arg0, arg1, (1u32).wrapping_add(x16), t401);
    x40 = t402;
    let t403 = b8_s_p(l, lid0, live, x32);
    x48 = t403;
    let t404 = [vdupq_n_u32(1023u32); 2];
    let t405: [uint32x4_t; 2] = [vandq_u32(t400[0], t404[0]), vandq_u32(t400[1], t404[1])];
    let t406 = [vdupq_n_u32(x32); 2];
    let t407: [uint32x4_t; 2] = [vmulq_u32(t405[0], t406[0]), vmulq_u32(t405[1], t406[1])];
    let t408 = b9_r0r1lls_p(l, lid0, live, arg0, arg1, x40, t407, 8u32);
    let t409: u64 = x48;
    let t410 = [vdupq_n_u32((t409) as u32); 2];
    let t411 = [vdupq_n_u32(((t409) >> 32) as u32); 2];
    let t412: [uint32x4_t; 4] = [vandq_u32(t408[0], t410[0]), vandq_u32(t408[1], t410[1]), vandq_u32(t408[2], t411[0]), vandq_u32(t408[3], t411[1])];
    x56 = t412;
    let t413: u64 = x48;
    let t414 = [vdupq_n_u32((t413) as u32); 2];
    let t415 = [vdupq_n_u32(((t413) >> 32) as u32); 2];
    let t416: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x56[0], t414[0]), vceqq_u32(x56[2], t415[0])), vandq_u32(vceqq_u32(x56[1], t414[1]), vceqq_u32(x56[3], t415[1]))];
    let mut t417: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t418: [uint32x4_t; 2] = [vandq_u32(live[0], t416[0]), vandq_u32(live[1], t416[1])];
    let t419: [uint32x4_t; 2] = [vbicq_u32(live[0], t416[0]), vbicq_u32(live[1], t416[1])];
    if any(&t418) {
        let t420: [uint32x4_t; 4];
        let t421 = bits(&t418);
        if t421.count_ones() <= 2 {
            let mut t422 = [0u32; 16];
            let mut t423 = t421;
            while t423 != 0 {
                let j = t423.trailing_zeros() as usize;
                t423 &= t423 - 1;
                let t424 = b10_r0r1ss_f(l, lid0, arg0, arg1, (x16).wrapping_add(1u32), (x24).wrapping_add(j as u32));
                let t425: u64 = t424;
                t422[j] = t425 as u32;
                t422[8 + j] = (t425 >> 32) as u32;
            }
            t420 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t422);
        } else {
            let t426 = b10_r0r1si_p(l, lid0, t418, arg0, arg1, (x16).wrapping_add(1u32), x24);
            t420 = t426;
        }
        t417 = [vbslq_u32(t418[0], t420[0], t417[0]), vbslq_u32(t418[1], t420[1], t417[1]), vbslq_u32(t418[0], t420[2], t417[2]), vbslq_u32(t418[1], t420[3], t417[3])];
    }
    {
        t417 = [vbslq_u32(t419[0], x56[0], t417[0]), vbslq_u32(t419[1], x56[1], t417[1]), vbslq_u32(t419[0], x56[2], t417[2]), vbslq_u32(t419[1], x56[3], t417[3])];
    }
    res = t417;
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
    let t427 = b6_r0r1s_f(l, lid0, arg0, arg1, x16);
    x32 = t427;
    let t428: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[1].as_ptr()))];
    let t429: [uint32x4_t; 2] = [vshlq_u32(t428[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(t428[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t430 = b7_r0r1sl_f(l, lid0, arg0, arg1, (1u32).wrapping_add(x16), t429);
    x40 = t430;
    let t431 = b8_s_f(l, lid0, x32);
    x48 = t431;
    let t432 = [vdupq_n_u32(1023u32); 2];
    let t433: [uint32x4_t; 2] = [vandq_u32(t428[0], t432[0]), vandq_u32(t428[1], t432[1])];
    let t434 = [vdupq_n_u32(x32); 2];
    let t435: [uint32x4_t; 2] = [vmulq_u32(t433[0], t434[0]), vmulq_u32(t433[1], t434[1])];
    let t436 = b9_r0r1lls_f(l, lid0, arg0, arg1, x40, t435, 8u32);
    let t437: u64 = x48;
    let t438 = [vdupq_n_u32((t437) as u32); 2];
    let t439 = [vdupq_n_u32(((t437) >> 32) as u32); 2];
    let t440: [uint32x4_t; 4] = [vandq_u32(t436[0], t438[0]), vandq_u32(t436[1], t438[1]), vandq_u32(t436[2], t439[0]), vandq_u32(t436[3], t439[1])];
    x56 = t440;
    let t441: u64 = x48;
    let t442 = [vdupq_n_u32((t441) as u32); 2];
    let t443 = [vdupq_n_u32(((t441) >> 32) as u32); 2];
    let t444: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x56[0], t442[0]), vceqq_u32(x56[2], t443[0])), vandq_u32(vceqq_u32(x56[1], t442[1]), vceqq_u32(x56[3], t443[1]))];
    let mut t445: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t446: [uint32x4_t; 2] = [vmvnq_u32(t444[0]), vmvnq_u32(t444[1])];
    if any(&t444) {
        let t447: [uint32x4_t; 4];
        let t448 = bits(&t444);
        if t448.count_ones() <= 2 {
            let mut t449 = [0u32; 16];
            let mut t450 = t448;
            while t450 != 0 {
                let j = t450.trailing_zeros() as usize;
                t450 &= t450 - 1;
                let t451 = b10_r0r1ss_f(l, lid0, arg0, arg1, (x16).wrapping_add(1u32), (x24).wrapping_add(j as u32));
                let t452: u64 = t451;
                t449[j] = t452 as u32;
                t449[8 + j] = (t452 >> 32) as u32;
            }
            t447 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t449);
        } else {
            let t453 = b10_r0r1si_p(l, lid0, t444, arg0, arg1, (x16).wrapping_add(1u32), x24);
            t447 = t453;
        }
        t445 = [vbslq_u32(t444[0], t447[0], t445[0]), vbslq_u32(t444[1], t447[1], t445[1]), vbslq_u32(t444[0], t447[2], t445[2]), vbslq_u32(t444[1], t447[3], t445[3])];
    }
    {
        t445 = [vbslq_u32(t446[0], x56[0], t445[0]), vbslq_u32(t446[1], x56[1], t445[1]), vbslq_u32(t446[0], x56[2], t445[2]), vbslq_u32(t446[1], x56[3], t445[3])];
    }
    res = t445;
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
    let t454 = b6_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    x32 = t454;
    let t455 = b7_r0r1ss_p(l, lid0, live, arg0, arg1, (1u32).wrapping_add(x16), (x24 >> 10));
    x40 = t455;
    let t456 = b8_s_p(l, lid0, live, x32);
    x48 = t456;
    let t457 = b9_r0r1sns_p(l, lid0, live, arg0, arg1, x40, (x24 & 1023u32).wrapping_mul(x32), x32, 8u32);
    let t458: u64 = x48;
    let t459 = [vdupq_n_u32((t458) as u32); 2];
    let t460 = [vdupq_n_u32(((t458) >> 32) as u32); 2];
    let t461: [uint32x4_t; 4] = [vandq_u32(t457[0], t459[0]), vandq_u32(t457[1], t459[1]), vandq_u32(t457[2], t460[0]), vandq_u32(t457[3], t460[1])];
    x56 = t461;
    let t462: u64 = x48;
    let t463 = [vdupq_n_u32((t462) as u32); 2];
    let t464 = [vdupq_n_u32(((t462) >> 32) as u32); 2];
    let t465: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x56[0], t463[0]), vceqq_u32(x56[2], t464[0])), vandq_u32(vceqq_u32(x56[1], t463[1]), vceqq_u32(x56[3], t464[1]))];
    let mut t466: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t467: [uint32x4_t; 2] = [vandq_u32(live[0], t465[0]), vandq_u32(live[1], t465[1])];
    let t468: [uint32x4_t; 2] = [vbicq_u32(live[0], t465[0]), vbicq_u32(live[1], t465[1])];
    if any(&t467) {
        let t469: [uint32x4_t; 4];
        let t470 = bits(&t467);
        if t470.count_ones() <= 2 {
            let mut t471 = [0u32; 16];
            let mut t472 = t470;
            while t472 != 0 {
                let j = t472.trailing_zeros() as usize;
                t472 &= t472 - 1;
                let t473 = b10_r0r1ss_f(l, lid0, arg0, arg1, (x16).wrapping_add(1u32), (x24).wrapping_add(j as u32));
                let t474: u64 = t473;
                t471[j] = t474 as u32;
                t471[8 + j] = (t474 >> 32) as u32;
            }
            t469 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t471);
        } else {
            let t475 = b10_r0r1su8_p(l, lid0, t467, arg0, arg1, (x16).wrapping_add(1u32), x24);
            t469 = t475;
        }
        t466 = [vbslq_u32(t467[0], t469[0], t466[0]), vbslq_u32(t467[1], t469[1], t466[1]), vbslq_u32(t467[0], t469[2], t466[2]), vbslq_u32(t467[1], t469[3], t466[3])];
    }
    {
        t466 = [vbslq_u32(t468[0], x56[0], t466[0]), vbslq_u32(t468[1], x56[1], t466[1]), vbslq_u32(t468[0], x56[2], t466[2]), vbslq_u32(t468[1], x56[3], t466[3])];
    }
    res = t466;
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
    let t476 = b6_r0r1s_f(l, lid0, arg0, arg1, x16);
    x32 = t476;
    let t477 = b7_r0r1ss_f(l, lid0, arg0, arg1, (1u32).wrapping_add(x16), (x24 >> 10));
    x40 = t477;
    let t478 = b8_s_f(l, lid0, x32);
    x48 = t478;
    let t479 = b9_r0r1sns_f(l, lid0, arg0, arg1, x40, (x24 & 1023u32).wrapping_mul(x32), x32, 8u32);
    let t480: u64 = x48;
    let t481 = [vdupq_n_u32((t480) as u32); 2];
    let t482 = [vdupq_n_u32(((t480) >> 32) as u32); 2];
    let t483: [uint32x4_t; 4] = [vandq_u32(t479[0], t481[0]), vandq_u32(t479[1], t481[1]), vandq_u32(t479[2], t482[0]), vandq_u32(t479[3], t482[1])];
    x56 = t483;
    let t484: u64 = x48;
    let t485 = [vdupq_n_u32((t484) as u32); 2];
    let t486 = [vdupq_n_u32(((t484) >> 32) as u32); 2];
    let t487: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x56[0], t485[0]), vceqq_u32(x56[2], t486[0])), vandq_u32(vceqq_u32(x56[1], t485[1]), vceqq_u32(x56[3], t486[1]))];
    let mut t488: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t489: [uint32x4_t; 2] = [vmvnq_u32(t487[0]), vmvnq_u32(t487[1])];
    if any(&t487) {
        let t490: [uint32x4_t; 4];
        let t491 = bits(&t487);
        if t491.count_ones() <= 2 {
            let mut t492 = [0u32; 16];
            let mut t493 = t491;
            while t493 != 0 {
                let j = t493.trailing_zeros() as usize;
                t493 &= t493 - 1;
                let t494 = b10_r0r1ss_f(l, lid0, arg0, arg1, (x16).wrapping_add(1u32), (x24).wrapping_add(j as u32));
                let t495: u64 = t494;
                t492[j] = t495 as u32;
                t492[8 + j] = (t495 >> 32) as u32;
            }
            t490 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t492);
        } else {
            let t496 = b10_r0r1su8_p(l, lid0, t487, arg0, arg1, (x16).wrapping_add(1u32), x24);
            t490 = t496;
        }
        t488 = [vbslq_u32(t487[0], t490[0], t488[0]), vbslq_u32(t487[1], t490[1], t488[1]), vbslq_u32(t487[0], t490[2], t488[2]), vbslq_u32(t487[1], t490[3], t488[3])];
    }
    {
        t488 = [vbslq_u32(t489[0], x56[0], t488[0]), vbslq_u32(t489[1], x56[1], t488[1]), vbslq_u32(t489[0], x56[2], t488[2]), vbslq_u32(t489[1], x56[3], t488[3])];
    }
    res = t488;
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
    let t497 = b6_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    x32 = t497;
    let t498: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t499 = b7_r0r1sl_p(l, lid0, live, arg0, arg1, (1u32).wrapping_add(x16), t498);
    x40 = t499;
    let t500 = b8_s_p(l, lid0, live, x32);
    x48 = t500;
    let t501 = [vdupq_n_u32(1023u32); 2];
    let t502: [uint32x4_t; 2] = [vandq_u32(x24[0], t501[0]), vandq_u32(x24[1], t501[1])];
    let t503 = [vdupq_n_u32(x32); 2];
    let t504: [uint32x4_t; 2] = [vmulq_u32(t502[0], t503[0]), vmulq_u32(t502[1], t503[1])];
    let t505 = b9_r0r1lls_p(l, lid0, live, arg0, arg1, x40, t504, 8u32);
    let t506: u64 = x48;
    let t507 = [vdupq_n_u32((t506) as u32); 2];
    let t508 = [vdupq_n_u32(((t506) >> 32) as u32); 2];
    let t509: [uint32x4_t; 4] = [vandq_u32(t505[0], t507[0]), vandq_u32(t505[1], t507[1]), vandq_u32(t505[2], t508[0]), vandq_u32(t505[3], t508[1])];
    x56 = t509;
    let t510: u64 = x48;
    let t511 = [vdupq_n_u32((t510) as u32); 2];
    let t512 = [vdupq_n_u32(((t510) >> 32) as u32); 2];
    let t513: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x56[0], t511[0]), vceqq_u32(x56[2], t512[0])), vandq_u32(vceqq_u32(x56[1], t511[1]), vceqq_u32(x56[3], t512[1]))];
    let mut t514: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t515: [uint32x4_t; 2] = [vandq_u32(live[0], t513[0]), vandq_u32(live[1], t513[1])];
    let t516: [uint32x4_t; 2] = [vbicq_u32(live[0], t513[0]), vbicq_u32(live[1], t513[1])];
    if any(&t515) {
        let t517: [uint32x4_t; 4];
        let t518 = bits(&t515);
        if t518.count_ones() <= 2 {
            let t519 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x24);
            let mut t520 = [0u32; 16];
            let mut t521 = t518;
            while t521 != 0 {
                let j = t521.trailing_zeros() as usize;
                t521 &= t521 - 1;
                let t522 = b10_r0r1ss_f(l, lid0, arg0, arg1, (x16).wrapping_add(1u32), t519[j]);
                let t523: u64 = t522;
                t520[j] = t523 as u32;
                t520[8 + j] = (t523 >> 32) as u32;
            }
            t517 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t520);
        } else {
            let t524 = b10_r0r1sl_p(l, lid0, t515, arg0, arg1, (x16).wrapping_add(1u32), x24);
            t517 = t524;
        }
        t514 = [vbslq_u32(t515[0], t517[0], t514[0]), vbslq_u32(t515[1], t517[1], t514[1]), vbslq_u32(t515[0], t517[2], t514[2]), vbslq_u32(t515[1], t517[3], t514[3])];
    }
    {
        t514 = [vbslq_u32(t516[0], x56[0], t514[0]), vbslq_u32(t516[1], x56[1], t514[1]), vbslq_u32(t516[0], x56[2], t514[2]), vbslq_u32(t516[1], x56[3], t514[3])];
    }
    res = t514;
    return res;
}

#[inline(always)]
unsafe fn b6_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t525 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    res = (((t525).wrapping_shr(((4u32).wrapping_mul(x16)) as u32)) & (15u32));
    return res;
}

#[inline(always)]
unsafe fn b6_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t526 = b11_r0r1s_f(l, lid0, arg0, arg1, 16u32);
    res = (((t526).wrapping_shr(((4u32).wrapping_mul(x16)) as u32)) & (15u32));
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: u32 = 0;
    let mut live = k;
    let mut res: u32 = 0;
    let t527 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 40u32);
    let t528 = b12_r0r1s_p(l, lid0, live, arg0, arg1, 0u32);
    x32 = ((x24).wrapping_mul(t527)).wrapping_add(t528);
    if ((x16) == (0u32)) {
        res = x32;
        return res;
    }
    let t529 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    let t530 = b13_ss_p(l, lid0, live, (t529).wrapping_sub((x24).wrapping_shl((10u32) as u32)), 1024u32);
    x40 = t530;
    let t531 = b12_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    res = (x32).wrapping_add((x40).wrapping_mul(t531));
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1ss_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32, x24: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: u32 = 0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t532 = b11_r0r1s_f(l, lid0, arg0, arg1, 40u32);
    let t533 = b12_r0r1s_f(l, lid0, arg0, arg1, 0u32);
    x32 = ((x24).wrapping_mul(t532)).wrapping_add(t533);
    if ((x16) == (0u32)) {
        res = x32;
        return res;
    }
    let t534 = b11_r0r1s_f(l, lid0, arg0, arg1, 4u32);
    let t535 = b13_ss_f(l, lid0, (t534).wrapping_sub((x24).wrapping_shl((10u32) as u32)), 1024u32);
    x40 = t535;
    let t536 = b12_r0r1s_f(l, lid0, arg0, arg1, x16);
    res = (x32).wrapping_add((x40).wrapping_mul(t536));
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t537 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 40u32);
    let t538 = [vdupq_n_u32(t537); 2];
    let t539: [uint32x4_t; 2] = [vmulq_u32(x24[0], t538[0]), vmulq_u32(x24[1], t538[1])];
    let t540 = b12_r0r1s_p(l, lid0, live, arg0, arg1, 0u32);
    let t541 = [vdupq_n_u32(t540); 2];
    let t542: [uint32x4_t; 2] = [vaddq_u32(t539[0], t541[0]), vaddq_u32(t539[1], t541[1])];
    x32 = t542;
    if ((x16) == (0u32)) {
        res = x32;
        return res;
    }
    let t543 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    let t544: [uint32x4_t; 2] = [vshlq_u32(x24[0], vdupq_n_s32(((10u32) & 31) as i32)), vshlq_u32(x24[1], vdupq_n_s32(((10u32) & 31) as i32))];
    let t545 = [vdupq_n_u32(t543); 2];
    let t546: [uint32x4_t; 2] = [vsubq_u32(t545[0], t544[0]), vsubq_u32(t545[1], t544[1])];
    let t547 = b13_ls_p(l, lid0, live, t546, 1024u32);
    x40 = t547;
    let t548 = b12_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    let t549 = [vdupq_n_u32(t548); 2];
    let t550: [uint32x4_t; 2] = [vmulq_u32(x40[0], t549[0]), vmulq_u32(x40[1], t549[1])];
    let t551: [uint32x4_t; 2] = [vaddq_u32(x32[0], t550[0]), vaddq_u32(x32[1], t550[1])];
    res = t551;
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1sl_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t552 = b11_r0r1s_f(l, lid0, arg0, arg1, 40u32);
    let t553 = [vdupq_n_u32(t552); 2];
    let t554: [uint32x4_t; 2] = [vmulq_u32(x24[0], t553[0]), vmulq_u32(x24[1], t553[1])];
    let t555 = b12_r0r1s_f(l, lid0, arg0, arg1, 0u32);
    let t556 = [vdupq_n_u32(t555); 2];
    let t557: [uint32x4_t; 2] = [vaddq_u32(t554[0], t556[0]), vaddq_u32(t554[1], t556[1])];
    x32 = t557;
    if ((x16) == (0u32)) {
        res = x32;
        return res;
    }
    let t558 = b11_r0r1s_f(l, lid0, arg0, arg1, 4u32);
    let t559: [uint32x4_t; 2] = [vshlq_u32(x24[0], vdupq_n_s32(((10u32) & 31) as i32)), vshlq_u32(x24[1], vdupq_n_s32(((10u32) & 31) as i32))];
    let t560 = [vdupq_n_u32(t558); 2];
    let t561: [uint32x4_t; 2] = [vsubq_u32(t560[0], t559[0]), vsubq_u32(t560[1], t559[1])];
    let t562 = b13_ls_f(l, lid0, t561, 1024u32);
    x40 = t562;
    let t563 = b12_r0r1s_f(l, lid0, arg0, arg1, x16);
    let t564 = [vdupq_n_u32(t563); 2];
    let t565: [uint32x4_t; 2] = [vmulq_u32(x40[0], t564[0]), vmulq_u32(x40[1], t564[1])];
    let t566: [uint32x4_t; 2] = [vaddq_u32(x32[0], t565[0]), vaddq_u32(x32[1], t565[1])];
    res = t566;
    return res;
}

#[inline(always)]
unsafe fn b8_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t567: u64;
    if ((x0) >= (8u32)) {
        t567 = !(0u64);
    } else {
        t567 = ((1u64).wrapping_shl(((((x0).wrapping_mul(8u32)) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t567;
    return res;
}

#[inline(always)]
unsafe fn b8_s_f(l: &Launch, lid0: u32, x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t568: u64;
    if ((x0) >= (8u32)) {
        t568 = !(0u64);
    } else {
        t568 = ((1u64).wrapping_shl(((((x0).wrapping_mul(8u32)) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t568;
    return res;
}

#[inline(always)]
unsafe fn b8_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t569 = [vdupq_n_u32(8u32); 2];
    let t570: [uint32x4_t; 2] = [vcgeq_u32(x0[0], t569[0]), vcgeq_u32(x0[1], t569[1])];
    let mut t571: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t572: [uint32x4_t; 2] = [vandq_u32(live[0], t570[0]), vandq_u32(live[1], t570[1])];
    let t573: [uint32x4_t; 2] = [vbicq_u32(live[0], t570[0]), vbicq_u32(live[1], t570[1])];
    {
        let t574: u64 = !(0u64);
        let t575 = [vdupq_n_u32((t574) as u32); 2];
        let t576 = [vdupq_n_u32(((t574) >> 32) as u32); 2];
        let t577: [uint32x4_t; 4] = [t575[0], t575[1], t576[0], t576[1]];
        t571 = [vbslq_u32(t572[0], t577[0], t571[0]), vbslq_u32(t572[1], t577[1], t571[1]), vbslq_u32(t572[0], t577[2], t571[2]), vbslq_u32(t572[1], t577[3], t571[3])];
    }
    {
        let t578: [uint32x4_t; 2] = [vmulq_u32(x0[0], t569[0]), vmulq_u32(x0[1], t569[1])];
        let t579: u64 = 1u64;
        let t580 = [vdupq_n_u32((t579) as u32); 2];
        let t581: [uint32x4_t; 2] = [t578[0], t578[1]];
        let t582: [uint32x4_t; 2] = [vandq_u32(t581[0], vdupq_n_u32(63)), vandq_u32(t581[1], vdupq_n_u32(63))];
        let t583: [uint32x4_t; 4] = [vshlq_u32(t580[0], vreinterpretq_s32_u32(t582[0])), vshlq_u32(t580[1], vreinterpretq_s32_u32(t582[1])), vshlq_u32(t580[0], vsubq_s32(vreinterpretq_s32_u32(t582[0]), vdupq_n_s32(32))), vshlq_u32(t580[1], vsubq_s32(vreinterpretq_s32_u32(t582[1]), vdupq_n_s32(32)))];
        let t584: u64 = 1u64;
        let t585 = [vdupq_n_u32((t584) as u32); 2];
        let t586: [uint32x4_t; 2] = [vcltq_u32(t583[0], t585[0]), vcltq_u32(t583[1], t585[1])];
        let t587: [uint32x4_t; 4] = [vsubq_u32(t583[0], t585[0]), vsubq_u32(t583[1], t585[1]), vaddq_u32(vsubq_u32(t583[2], vdupq_n_u32(0)), t586[0]), vaddq_u32(vsubq_u32(t583[3], vdupq_n_u32(0)), t586[1])];
        t571 = [vbslq_u32(t573[0], t587[0], t571[0]), vbslq_u32(t573[1], t587[1], t571[1]), vbslq_u32(t573[0], t587[2], t571[2]), vbslq_u32(t573[1], t587[3], t571[3])];
    }
    res = t571;
    return res;
}

#[inline(always)]
unsafe fn b9_r0r1sns_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24f: u32, x24s: u32, x32: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t588 = b14_r0sns_p(l, lid0, live, arg0, x16, x24f, x24s, x32);
    res = t588;
    return res;
}

#[inline(always)]
unsafe fn b9_r0r1sns_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32, x24f: u32, x24s: u32, x32: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t589 = b14_r0sns_f(l, lid0, arg0, x16, x24f, x24s, x32);
    res = t589;
    return res;
}

#[inline(always)]
unsafe fn b9_r0r1lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2], x24: [uint32x4_t; 2], x32: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t590 = b14_r0lls_p(l, lid0, live, arg0, x16, x24, x32);
    res = t590;
    return res;
}

#[inline(always)]
unsafe fn b9_r0r1lls_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2], x24: [uint32x4_t; 2], x32: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t591 = b14_r0lls_f(l, lid0, arg0, x16, x24, x32);
    res = t591;
    return res;
}

#[inline(always)]
unsafe fn b10_r0r1ss_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32, x24: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: u32 = 0;
    let mut x48: u32 = 0;
    let mut x56: u32 = 0;
    let mut x64: u32 = 0;
    let mut x72: u32 = 0;
    let mut x80: u32 = 0;
    let mut x88: u32 = 0;
    let mut x96: u32 = 0;
    let mut x104: u32 = 0;
    let mut x112: u32 = 0;
    let mut x120: u32 = 0;
    let mut x128: u64 = 0;
    let mut x136: u64 = 0;
    let mut x144: u32 = 0;
    let mut x152: u32 = 0;
    let mut x160: u64 = 0;
    let mut x168: u64 = 0;
    let mut x176: u64 = 0;
    let mut x184: u32 = 0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t592 = b11_r0r1s_f(l, lid0, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x16)));
    x32 = t592;
    let t593 = b11_r0r1s_f(l, lid0, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x16)));
    x40 = t593;
    x48 = (x24).wrapping_shr((10u32) as u32);
    let t594 = b15_r0r1sss_f(l, lid0, arg0, arg1, x40, (x48).wrapping_mul(4u32), 4u32);
    x56 = ((t594) as u32);
    let t595 = b15_r0r1sss_f(l, lid0, arg0, arg1, x40, ((x48).wrapping_mul(4u32)).wrapping_add(4u32), 4u32);
    x64 = ((t595) as u32);
    let t596 = b11_r0r1s_f(l, lid0, arg0, arg1, 4u32);
    x72 = ((t596).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
    let t597 = b16_ss_f(l, lid0, 4u32, (x72).wrapping_sub((x48).wrapping_mul(4u32)));
    x80 = t597;
    x88 = (((x24).wrapping_shr((8u32) as u32)) & (3u32));
    let t598: u32;
    if ((x88) == (0u32)) {
        t598 = 0u32;
    } else {
        let t599 = b17_ss_f(l, lid0, (x56).wrapping_add((2u32).wrapping_mul((x88).wrapping_sub(1u32))), x64);
        let t600 = b15_r0r1sss_f(l, lid0, arg0, arg1, x32, t599, 2u32);
        t598 = ((t600) as u32);
    }
    x96 = t598;
    let t601 = b17_ss_f(l, lid0, (x56).wrapping_add((2u32).wrapping_mul(x88)), x64);
    let t602 = b15_r0r1sss_f(l, lid0, arg0, arg1, x32, t601, 2u32);
    x104 = ((t602) as u32);
    x112 = (x56).wrapping_add((2u32).wrapping_mul(x80));
    x120 = ((x24) & (255u32));
    x128 = 9259542123273814144u64;
    x136 = (((x120) as u64)).wrapping_mul(72340172838076673u64);
    x144 = x96;
    x152 = x96;
    loop {
        // Rounds of one condition: every lane goes on.
        if !(((x152) < (x104))) { break; }
        let t603 = b17_ss_f(l, lid0, (x112).wrapping_add(x152), x64);
        let t604 = b15_r0r1sss_f(l, lid0, arg0, arg1, x32, t603, 8u32);
        x160 = t604;
        x168 = (((x160) | (x128))).wrapping_sub(((x136) & (!(x128))));
        x176 = ((x128) & (((((!(x160)) & (x136))) | (((!(((x160) ^ (x136)))) & (!(x168)))))));
        x184 = (x104).wrapping_sub(x152);
        let t605: u64;
        if ((x184) >= (8u32)) {
            t605 = x176;
        } else {
            t605 = ((x176) & (((1u64).wrapping_shl(((((8u32).wrapping_mul(x184)) as u64)) as u32)).wrapping_sub(1u64)));
        }
        let t606 = b18_s_f(l, lid0, t605);
        x144 = (x144).wrapping_add(t606);
        x152 = (x152).wrapping_add(8u32);
    }
    x96 = x144;
    let t607 = b11_r0r1s_f(l, lid0, arg0, arg1, 16u32);
    x152 = (((t607).wrapping_shr((((((4u32).wrapping_mul(x16)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32));
    let t608 = b17_ss_f(l, lid0, (x56).wrapping_add((2u32).wrapping_mul((x80).wrapping_sub(1u32))), x64);
    let t609 = b15_r0r1sss_f(l, lid0, arg0, arg1, x32, t608, 2u32);
    x160 = ((((t609) as u32)) as u64);
    let t610 = b17_ss_f(l, lid0, ((x112).wrapping_add(((x160) as u32))).wrapping_add((x96).wrapping_mul(x152)), x64);
    x168 = ((t610) as u64);
    let t611 = b15_r0r1sss_f(l, lid0, arg0, arg1, x32, ((x168) as u32), 8u32);
    let t612 = b8_s_f(l, lid0, x152);
    res = ((t611) & (t612));
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
    let t613 = b11_r0r1s_p(l, lid0, live, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x16)));
    x32 = t613;
    let t614 = b11_r0r1s_p(l, lid0, live, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x16)));
    x40 = t614;
    let t615: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[1].as_ptr()))];
    let t616: [uint32x4_t; 2] = [vshlq_u32(t615[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(t615[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    x48 = t616;
    let t617 = [vdupq_n_u32(4u32); 2];
    let t618: [uint32x4_t; 2] = [vmulq_u32(x48[0], t617[0]), vmulq_u32(x48[1], t617[1])];
    let t619 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x40, t618, 4u32);
    let t620: [uint32x4_t; 2] = [t619[0], t619[1]];
    x56 = t620;
    let t621: [uint32x4_t; 2] = [vmulq_u32(x48[0], t617[0]), vmulq_u32(x48[1], t617[1])];
    let t622: [uint32x4_t; 2] = [vaddq_u32(t621[0], t617[0]), vaddq_u32(t621[1], t617[1])];
    let t623 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x40, t622, 4u32);
    let t624: [uint32x4_t; 2] = [t623[0], t623[1]];
    x64 = t624;
    let t625 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    x72 = ((t625).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
    let t626: [uint32x4_t; 2] = [vmulq_u32(x48[0], t617[0]), vmulq_u32(x48[1], t617[1])];
    let t627 = [vdupq_n_u32(x72); 2];
    let t628: [uint32x4_t; 2] = [vsubq_u32(t627[0], t626[0]), vsubq_u32(t627[1], t626[1])];
    let t629 = b16_sl_p(l, lid0, live, 4u32, t628);
    x80 = t629;
    let t630: [uint32x4_t; 2] = [vshlq_u32(t615[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(t615[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
    let t631 = [vdupq_n_u32(3u32); 2];
    let t632: [uint32x4_t; 2] = [vandq_u32(t630[0], t631[0]), vandq_u32(t630[1], t631[1])];
    x88 = t632;
    let t633 = [vdupq_n_u32(0u32); 2];
    let t634: [uint32x4_t; 2] = [vceqq_u32(x88[0], t633[0]), vceqq_u32(x88[1], t633[1])];
    let mut t635: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t636: [uint32x4_t; 2] = [vandq_u32(live[0], t634[0]), vandq_u32(live[1], t634[1])];
    let t637: [uint32x4_t; 2] = [vbicq_u32(live[0], t634[0]), vbicq_u32(live[1], t634[1])];
    {
        t635 = [vbslq_u32(t636[0], t633[0], t635[0]), vbslq_u32(t636[1], t633[1], t635[1])];
    }
    if any(&t637) {
        let t638 = [vdupq_n_u32(1u32); 2];
        let t639: [uint32x4_t; 2] = [vsubq_u32(x88[0], t638[0]), vsubq_u32(x88[1], t638[1])];
        let t640 = [vdupq_n_u32(2u32); 2];
        let t641: [uint32x4_t; 2] = [vmulq_u32(t640[0], t639[0]), vmulq_u32(t640[1], t639[1])];
        let t642: [uint32x4_t; 2] = [vaddq_u32(x56[0], t641[0]), vaddq_u32(x56[1], t641[1])];
        let t643 = b17_ll_p(l, lid0, t637, t642, x64);
        let t644 = b15_r0r1sls_p(l, lid0, t637, arg0, arg1, x32, t643, 2u32);
        let t645: [uint32x4_t; 2] = [t644[0], t644[1]];
        t635 = [vbslq_u32(t637[0], t645[0], t635[0]), vbslq_u32(t637[1], t645[1], t635[1])];
    }
    x96 = t635;
    let t646 = [vdupq_n_u32(2u32); 2];
    let t647: [uint32x4_t; 2] = [vmulq_u32(t646[0], x88[0]), vmulq_u32(t646[1], x88[1])];
    let t648: [uint32x4_t; 2] = [vaddq_u32(x56[0], t647[0]), vaddq_u32(x56[1], t647[1])];
    let t649 = b17_ll_p(l, lid0, live, t648, x64);
    let t650 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t649, 2u32);
    let t651: [uint32x4_t; 2] = [t650[0], t650[1]];
    x104 = t651;
    let t652: [uint32x4_t; 2] = [vmulq_u32(t646[0], x80[0]), vmulq_u32(t646[1], x80[1])];
    let t653: [uint32x4_t; 2] = [vaddq_u32(x56[0], t652[0]), vaddq_u32(x56[1], t652[1])];
    x112 = t653;
    let t654 = [vdupq_n_u32(255u32); 2];
    let t655: [uint32x4_t; 2] = [vandq_u32(t615[0], t654[0]), vandq_u32(t615[1], t654[1])];
    x120 = t655;
    x128 = 9259542123273814144u64;
    let t656: u64 = 72340172838076673u64;
    let t657 = [vdupq_n_u32((t656) as u32); 2];
    let t658 = [vdupq_n_u32(((t656) >> 32) as u32); 2];
    let t659: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x120[0]), vget_low_u32(t657[0]))), vreinterpretq_u32_u64(vmull_high_u32(x120[0], t657[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x120[1]), vget_low_u32(t657[1]))), vreinterpretq_u32_u64(vmull_high_u32(x120[1], t657[1])))];
    let t660: [uint32x4_t; 4] = [vmulq_u32(x120[0], t657[0]), vmulq_u32(x120[1], t657[1]), vmlaq_u32(t659[0], x120[0], t658[0]), vmlaq_u32(t659[1], x120[1], t658[1])];
    x136 = t660;
    x144 = x96;
    x152 = x96;
    let mut t662 = live;
    loop {
        // Lanes part: each its own rounds.
        let t664: [uint32x4_t; 2] = [vcltq_u32(x152[0], x104[0]), vcltq_u32(x152[1], x104[1])];
        t662 = [vandq_u32(t662[0], t664[0]), vandq_u32(t662[1], t664[1])];
        if !(any(&t662)) { break; }
        let t666: [uint32x4_t; 2] = [vaddq_u32(x112[0], x152[0]), vaddq_u32(x112[1], x152[1])];
        let t667 = b17_ll_p(l, lid0, t662, t666, x64);
        let t668 = b15_r0r1sls_p(l, lid0, t662, arg0, arg1, x32, t667, 8u32);
        x160 = [vbslq_u32(t662[0], t668[0], x160[0]), vbslq_u32(t662[1], t668[1], x160[1]), vbslq_u32(t662[0], t668[2], x160[2]), vbslq_u32(t662[1], t668[3], x160[3])];
        let t669: u64 = x128;
        let t670 = [vdupq_n_u32((t669) as u32); 2];
        let t671 = [vdupq_n_u32(((t669) >> 32) as u32); 2];
        let t672: [uint32x4_t; 4] = [vorrq_u32(x160[0], t670[0]), vorrq_u32(x160[1], t670[1]), vorrq_u32(x160[2], t671[0]), vorrq_u32(x160[3], t671[1])];
        let t673: u64 = !(x128);
        let t674 = [vdupq_n_u32((t673) as u32); 2];
        let t675 = [vdupq_n_u32(((t673) >> 32) as u32); 2];
        let t676: [uint32x4_t; 4] = [vandq_u32(x136[0], t674[0]), vandq_u32(x136[1], t674[1]), vandq_u32(x136[2], t675[0]), vandq_u32(x136[3], t675[1])];
        let t677: [uint32x4_t; 2] = [vcltq_u32(t672[0], t676[0]), vcltq_u32(t672[1], t676[1])];
        let t678: [uint32x4_t; 4] = [vsubq_u32(t672[0], t676[0]), vsubq_u32(t672[1], t676[1]), vaddq_u32(vsubq_u32(t672[2], t676[2]), t677[0]), vaddq_u32(vsubq_u32(t672[3], t676[3]), t677[1])];
        x168 = [vbslq_u32(t662[0], t678[0], x168[0]), vbslq_u32(t662[1], t678[1], x168[1]), vbslq_u32(t662[0], t678[2], x168[2]), vbslq_u32(t662[1], t678[3], x168[3])];
        let t679: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x160[r]));
        let t680: [uint32x4_t; 4] = [vandq_u32(t679[0], x136[0]), vandq_u32(t679[1], x136[1]), vandq_u32(t679[2], x136[2]), vandq_u32(t679[3], x136[3])];
        let t681: [uint32x4_t; 4] = [veorq_u32(x160[0], x136[0]), veorq_u32(x160[1], x136[1]), veorq_u32(x160[2], x136[2]), veorq_u32(x160[3], x136[3])];
        let t682: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(t681[r]));
        let t683: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x168[r]));
        let t684: [uint32x4_t; 4] = [vandq_u32(t682[0], t683[0]), vandq_u32(t682[1], t683[1]), vandq_u32(t682[2], t683[2]), vandq_u32(t682[3], t683[3])];
        let t685: [uint32x4_t; 4] = [vorrq_u32(t680[0], t684[0]), vorrq_u32(t680[1], t684[1]), vorrq_u32(t680[2], t684[2]), vorrq_u32(t680[3], t684[3])];
        let t686: u64 = x128;
        let t687 = [vdupq_n_u32((t686) as u32); 2];
        let t688 = [vdupq_n_u32(((t686) >> 32) as u32); 2];
        let t689: [uint32x4_t; 4] = [vandq_u32(t687[0], t685[0]), vandq_u32(t687[1], t685[1]), vandq_u32(t688[0], t685[2]), vandq_u32(t688[1], t685[3])];
        x176 = t689;
        let t690: [uint32x4_t; 2] = [vsubq_u32(x104[0], x152[0]), vsubq_u32(x104[1], x152[1])];
        x184 = t690;
        let t691 = [vdupq_n_u32(8u32); 2];
        let t692: [uint32x4_t; 2] = [vcgeq_u32(x184[0], t691[0]), vcgeq_u32(x184[1], t691[1])];
        let mut t693: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t694: [uint32x4_t; 2] = [vandq_u32(t662[0], t692[0]), vandq_u32(t662[1], t692[1])];
        let t695: [uint32x4_t; 2] = [vbicq_u32(t662[0], t692[0]), vbicq_u32(t662[1], t692[1])];
        {
            t693 = [vbslq_u32(t694[0], x176[0], t693[0]), vbslq_u32(t694[1], x176[1], t693[1]), vbslq_u32(t694[0], x176[2], t693[2]), vbslq_u32(t694[1], x176[3], t693[3])];
        }
        {
            let t696: [uint32x4_t; 2] = [vmulq_u32(t691[0], x184[0]), vmulq_u32(t691[1], x184[1])];
            let t697: u64 = 1u64;
            let t698 = [vdupq_n_u32((t697) as u32); 2];
            let t699: [uint32x4_t; 2] = [t696[0], t696[1]];
            let t700: [uint32x4_t; 2] = [vandq_u32(t699[0], vdupq_n_u32(63)), vandq_u32(t699[1], vdupq_n_u32(63))];
            let t701: [uint32x4_t; 4] = [vshlq_u32(t698[0], vreinterpretq_s32_u32(t700[0])), vshlq_u32(t698[1], vreinterpretq_s32_u32(t700[1])), vshlq_u32(t698[0], vsubq_s32(vreinterpretq_s32_u32(t700[0]), vdupq_n_s32(32))), vshlq_u32(t698[1], vsubq_s32(vreinterpretq_s32_u32(t700[1]), vdupq_n_s32(32)))];
            let t702: u64 = 1u64;
            let t703 = [vdupq_n_u32((t702) as u32); 2];
            let t704: [uint32x4_t; 2] = [vcltq_u32(t701[0], t703[0]), vcltq_u32(t701[1], t703[1])];
            let t705: [uint32x4_t; 4] = [vsubq_u32(t701[0], t703[0]), vsubq_u32(t701[1], t703[1]), vaddq_u32(vsubq_u32(t701[2], vdupq_n_u32(0)), t704[0]), vaddq_u32(vsubq_u32(t701[3], vdupq_n_u32(0)), t704[1])];
            let t706: [uint32x4_t; 4] = [vandq_u32(x176[0], t705[0]), vandq_u32(x176[1], t705[1]), vandq_u32(x176[2], t705[2]), vandq_u32(x176[3], t705[3])];
            t693 = [vbslq_u32(t695[0], t706[0], t693[0]), vbslq_u32(t695[1], t706[1], t693[1]), vbslq_u32(t695[0], t706[2], t693[2]), vbslq_u32(t695[1], t706[3], t693[3])];
        }
        let t707 = b18_l_p(l, lid0, t662, t693);
        let t708: [uint32x4_t; 2] = [vaddq_u32(x144[0], t707[0]), vaddq_u32(x144[1], t707[1])];
        x144 = [vbslq_u32(t662[0], t708[0], x144[0]), vbslq_u32(t662[1], t708[1], x144[1])];
        let t709: [uint32x4_t; 2] = [vandq_u32(t662[0], live[0]), vandq_u32(t662[1], live[1])];
        x152 = [vaddq_u32(x152[0], vandq_u32(t709[0], vdupq_n_u32(8))), vaddq_u32(x152[1], vandq_u32(t709[1], vdupq_n_u32(8)))];
    }
    x96 = x144;
    let t710 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    let t711 = [vdupq_n_u32((((t710).wrapping_shr((((((4u32).wrapping_mul(x16)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32))); 2];
    x152 = t711;
    let t712 = [vdupq_n_u32(1u32); 2];
    let t713: [uint32x4_t; 2] = [vsubq_u32(x80[0], t712[0]), vsubq_u32(x80[1], t712[1])];
    let t714: [uint32x4_t; 2] = [vmulq_u32(t646[0], t713[0]), vmulq_u32(t646[1], t713[1])];
    let t715: [uint32x4_t; 2] = [vaddq_u32(x56[0], t714[0]), vaddq_u32(x56[1], t714[1])];
    let t716 = b17_ll_p(l, lid0, live, t715, x64);
    let t717 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t716, 2u32);
    let t718: [uint32x4_t; 2] = [t717[0], t717[1]];
    let t719: [uint32x4_t; 4] = [t718[0], t718[1], vdupq_n_u32(0), vdupq_n_u32(0)];
    x160 = t719;
    let t720: [uint32x4_t; 2] = [x160[0], x160[1]];
    let t721: [uint32x4_t; 2] = [vaddq_u32(x112[0], t720[0]), vaddq_u32(x112[1], t720[1])];
    let t722: [uint32x4_t; 2] = [vmulq_u32(x96[0], x152[0]), vmulq_u32(x96[1], x152[1])];
    let t723: [uint32x4_t; 2] = [vaddq_u32(t721[0], t722[0]), vaddq_u32(t721[1], t722[1])];
    let t724 = b17_ll_p(l, lid0, live, t723, x64);
    let t725: [uint32x4_t; 4] = [t724[0], t724[1], vdupq_n_u32(0), vdupq_n_u32(0)];
    x168 = t725;
    let t726: [uint32x4_t; 2] = [x168[0], x168[1]];
    let t727 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t726, 8u32);
    let t728 = b8_l_p(l, lid0, live, x152);
    let t729: [uint32x4_t; 4] = [vandq_u32(t727[0], t728[0]), vandq_u32(t727[1], t728[1]), vandq_u32(t727[2], t728[2]), vandq_u32(t727[3], t728[3])];
    res = t729;
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
    let t730 = b11_r0r1s_p(l, lid0, live, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x16)));
    x32 = t730;
    let t731 = b11_r0r1s_p(l, lid0, live, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x16)));
    x40 = t731;
    x48 = (x24 >> 10);
    let t732 = b15_r0r1sss_p(l, lid0, live, arg0, arg1, x40, (x48).wrapping_mul(4u32), 4u32);
    x56 = ((t732) as u32);
    let t733 = b15_r0r1sss_p(l, lid0, live, arg0, arg1, x40, ((x48).wrapping_mul(4u32)).wrapping_add(4u32), 4u32);
    x64 = ((t733) as u32);
    let t734 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    x72 = ((t734).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
    let t735 = b16_ss_p(l, lid0, live, 4u32, (x72).wrapping_sub((x48).wrapping_mul(4u32)));
    x80 = t735;
    x88 = (((x24 >> 8)) & (3u32));
    let t736: u32;
    if ((x88) == (0u32)) {
        t736 = 0u32;
    } else {
        let t737 = b17_ss_p(l, lid0, live, (x56).wrapping_add((2u32).wrapping_mul((x88).wrapping_sub(1u32))), x64);
        let t738 = b15_r0r1sss_p(l, lid0, live, arg0, arg1, x32, t737, 2u32);
        t736 = ((t738) as u32);
    }
    let t739 = [vdupq_n_u32(t736); 2];
    x96 = t739;
    let t740 = b17_ss_p(l, lid0, live, (x56).wrapping_add((2u32).wrapping_mul(x88)), x64);
    let t741 = b15_r0r1sss_p(l, lid0, live, arg0, arg1, x32, t740, 2u32);
    x104 = ((t741) as u32);
    x112 = (x56).wrapping_add((2u32).wrapping_mul(x80));
    x120 = (x24 & 255u32);
    x128 = 9259542123273814144u64;
    let t742: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x120), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x120), vld1q_u32(LANE[1].as_ptr()))];
    let t743: u64 = 72340172838076673u64;
    let t744 = [vdupq_n_u32((t743) as u32); 2];
    let t745 = [vdupq_n_u32(((t743) >> 32) as u32); 2];
    let t746: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(t742[0]), vget_low_u32(t744[0]))), vreinterpretq_u32_u64(vmull_high_u32(t742[0], t744[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(t742[1]), vget_low_u32(t744[1]))), vreinterpretq_u32_u64(vmull_high_u32(t742[1], t744[1])))];
    let t747: [uint32x4_t; 4] = [vmulq_u32(t742[0], t744[0]), vmulq_u32(t742[1], t744[1]), vmlaq_u32(t746[0], t742[0], t745[0]), vmlaq_u32(t746[1], t742[1], t745[1])];
    x136 = t747;
    x144 = x96;
    x152 = x96;
    let mut t750 = live;
    loop {
        // Lanes part: each its own rounds.
        let t752 = [vdupq_n_u32(x104); 2];
        let t753: [uint32x4_t; 2] = [vcltq_u32(x152[0], t752[0]), vcltq_u32(x152[1], t752[1])];
        t750 = [vandq_u32(t750[0], t753[0]), vandq_u32(t750[1], t753[1])];
        if !(any(&t750)) { break; }
        let t755 = [vdupq_n_u32(x112); 2];
        let t756: [uint32x4_t; 2] = [vaddq_u32(t755[0], x152[0]), vaddq_u32(t755[1], x152[1])];
        let t757 = b17_ls_p(l, lid0, t750, t756, x64);
        let t758 = b15_r0r1sls_p(l, lid0, t750, arg0, arg1, x32, t757, 8u32);
        x160 = [vbslq_u32(t750[0], t758[0], x160[0]), vbslq_u32(t750[1], t758[1], x160[1]), vbslq_u32(t750[0], t758[2], x160[2]), vbslq_u32(t750[1], t758[3], x160[3])];
        let t759: u64 = x128;
        let t760 = [vdupq_n_u32((t759) as u32); 2];
        let t761 = [vdupq_n_u32(((t759) >> 32) as u32); 2];
        let t762: [uint32x4_t; 4] = [vorrq_u32(x160[0], t760[0]), vorrq_u32(x160[1], t760[1]), vorrq_u32(x160[2], t761[0]), vorrq_u32(x160[3], t761[1])];
        let t763: u64 = !(x128);
        let t764 = [vdupq_n_u32((t763) as u32); 2];
        let t765 = [vdupq_n_u32(((t763) >> 32) as u32); 2];
        let t766: [uint32x4_t; 4] = [vandq_u32(x136[0], t764[0]), vandq_u32(x136[1], t764[1]), vandq_u32(x136[2], t765[0]), vandq_u32(x136[3], t765[1])];
        let t767: [uint32x4_t; 2] = [vcltq_u32(t762[0], t766[0]), vcltq_u32(t762[1], t766[1])];
        let t768: [uint32x4_t; 4] = [vsubq_u32(t762[0], t766[0]), vsubq_u32(t762[1], t766[1]), vaddq_u32(vsubq_u32(t762[2], t766[2]), t767[0]), vaddq_u32(vsubq_u32(t762[3], t766[3]), t767[1])];
        x168 = [vbslq_u32(t750[0], t768[0], x168[0]), vbslq_u32(t750[1], t768[1], x168[1]), vbslq_u32(t750[0], t768[2], x168[2]), vbslq_u32(t750[1], t768[3], x168[3])];
        let t769: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x160[r]));
        let t770: [uint32x4_t; 4] = [vandq_u32(t769[0], x136[0]), vandq_u32(t769[1], x136[1]), vandq_u32(t769[2], x136[2]), vandq_u32(t769[3], x136[3])];
        let t771: [uint32x4_t; 4] = [veorq_u32(x160[0], x136[0]), veorq_u32(x160[1], x136[1]), veorq_u32(x160[2], x136[2]), veorq_u32(x160[3], x136[3])];
        let t772: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(t771[r]));
        let t773: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x168[r]));
        let t774: [uint32x4_t; 4] = [vandq_u32(t772[0], t773[0]), vandq_u32(t772[1], t773[1]), vandq_u32(t772[2], t773[2]), vandq_u32(t772[3], t773[3])];
        let t775: [uint32x4_t; 4] = [vorrq_u32(t770[0], t774[0]), vorrq_u32(t770[1], t774[1]), vorrq_u32(t770[2], t774[2]), vorrq_u32(t770[3], t774[3])];
        let t776: u64 = x128;
        let t777 = [vdupq_n_u32((t776) as u32); 2];
        let t778 = [vdupq_n_u32(((t776) >> 32) as u32); 2];
        let t779: [uint32x4_t; 4] = [vandq_u32(t777[0], t775[0]), vandq_u32(t777[1], t775[1]), vandq_u32(t778[0], t775[2]), vandq_u32(t778[1], t775[3])];
        x176 = t779;
        let t780 = [vdupq_n_u32(x104); 2];
        let t781: [uint32x4_t; 2] = [vsubq_u32(t780[0], x152[0]), vsubq_u32(t780[1], x152[1])];
        x184 = t781;
        let t782 = [vdupq_n_u32(8u32); 2];
        let t783: [uint32x4_t; 2] = [vcgeq_u32(x184[0], t782[0]), vcgeq_u32(x184[1], t782[1])];
        let mut t784: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t785: [uint32x4_t; 2] = [vandq_u32(t750[0], t783[0]), vandq_u32(t750[1], t783[1])];
        let t786: [uint32x4_t; 2] = [vbicq_u32(t750[0], t783[0]), vbicq_u32(t750[1], t783[1])];
        {
            t784 = [vbslq_u32(t785[0], x176[0], t784[0]), vbslq_u32(t785[1], x176[1], t784[1]), vbslq_u32(t785[0], x176[2], t784[2]), vbslq_u32(t785[1], x176[3], t784[3])];
        }
        {
            let t787: [uint32x4_t; 2] = [vmulq_u32(t782[0], x184[0]), vmulq_u32(t782[1], x184[1])];
            let t788: u64 = 1u64;
            let t789 = [vdupq_n_u32((t788) as u32); 2];
            let t790: [uint32x4_t; 2] = [t787[0], t787[1]];
            let t791: [uint32x4_t; 2] = [vandq_u32(t790[0], vdupq_n_u32(63)), vandq_u32(t790[1], vdupq_n_u32(63))];
            let t792: [uint32x4_t; 4] = [vshlq_u32(t789[0], vreinterpretq_s32_u32(t791[0])), vshlq_u32(t789[1], vreinterpretq_s32_u32(t791[1])), vshlq_u32(t789[0], vsubq_s32(vreinterpretq_s32_u32(t791[0]), vdupq_n_s32(32))), vshlq_u32(t789[1], vsubq_s32(vreinterpretq_s32_u32(t791[1]), vdupq_n_s32(32)))];
            let t793: u64 = 1u64;
            let t794 = [vdupq_n_u32((t793) as u32); 2];
            let t795: [uint32x4_t; 2] = [vcltq_u32(t792[0], t794[0]), vcltq_u32(t792[1], t794[1])];
            let t796: [uint32x4_t; 4] = [vsubq_u32(t792[0], t794[0]), vsubq_u32(t792[1], t794[1]), vaddq_u32(vsubq_u32(t792[2], vdupq_n_u32(0)), t795[0]), vaddq_u32(vsubq_u32(t792[3], vdupq_n_u32(0)), t795[1])];
            let t797: [uint32x4_t; 4] = [vandq_u32(x176[0], t796[0]), vandq_u32(x176[1], t796[1]), vandq_u32(x176[2], t796[2]), vandq_u32(x176[3], t796[3])];
            t784 = [vbslq_u32(t786[0], t797[0], t784[0]), vbslq_u32(t786[1], t797[1], t784[1]), vbslq_u32(t786[0], t797[2], t784[2]), vbslq_u32(t786[1], t797[3], t784[3])];
        }
        let t798 = b18_l_p(l, lid0, t750, t784);
        let t799: [uint32x4_t; 2] = [vaddq_u32(x144[0], t798[0]), vaddq_u32(x144[1], t798[1])];
        x144 = [vbslq_u32(t750[0], t799[0], x144[0]), vbslq_u32(t750[1], t799[1], x144[1])];
        let t800: [uint32x4_t; 2] = [vandq_u32(t750[0], live[0]), vandq_u32(t750[1], live[1])];
        x152 = [vaddq_u32(x152[0], vandq_u32(t800[0], vdupq_n_u32(8))), vaddq_u32(x152[1], vandq_u32(t800[1], vdupq_n_u32(8)))];
    }
    x96 = x144;
    let t801 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    let t802 = [vdupq_n_u32((((t801).wrapping_shr((((((4u32).wrapping_mul(x16)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32))); 2];
    x152 = t802;
    let t803 = b17_ss_p(l, lid0, live, (x56).wrapping_add((2u32).wrapping_mul((x80).wrapping_sub(1u32))), x64);
    let t804 = b15_r0r1sss_p(l, lid0, live, arg0, arg1, x32, t803, 2u32);
    let t805: u64 = ((((t804) as u32)) as u64);
    let t806 = [vdupq_n_u32((t805) as u32); 2];
    let t807 = [vdupq_n_u32(((t805) >> 32) as u32); 2];
    let t808: [uint32x4_t; 4] = [t806[0], t806[1], t807[0], t807[1]];
    x160 = t808;
    let t809 = [vdupq_n_u32(x112); 2];
    let t810: [uint32x4_t; 2] = [x160[0], x160[1]];
    let t811: [uint32x4_t; 2] = [vaddq_u32(t809[0], t810[0]), vaddq_u32(t809[1], t810[1])];
    let t812: [uint32x4_t; 2] = [vmulq_u32(x96[0], x152[0]), vmulq_u32(x96[1], x152[1])];
    let t813: [uint32x4_t; 2] = [vaddq_u32(t811[0], t812[0]), vaddq_u32(t811[1], t812[1])];
    let t814 = b17_ls_p(l, lid0, live, t813, x64);
    let t815: [uint32x4_t; 4] = [t814[0], t814[1], vdupq_n_u32(0), vdupq_n_u32(0)];
    x168 = t815;
    let t816: [uint32x4_t; 2] = [x168[0], x168[1]];
    let t817 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t816, 8u32);
    let t818 = b8_l_p(l, lid0, live, x152);
    let t819: [uint32x4_t; 4] = [vandq_u32(t817[0], t818[0]), vandq_u32(t817[1], t818[1]), vandq_u32(t817[2], t818[2]), vandq_u32(t817[3], t818[3])];
    res = t819;
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
    let t820 = b11_r0r1s_p(l, lid0, live, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x16)));
    x32 = t820;
    let t821 = b11_r0r1s_p(l, lid0, live, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x16)));
    x40 = t821;
    let t822: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    x48 = t822;
    let t823 = [vdupq_n_u32(4u32); 2];
    let t824: [uint32x4_t; 2] = [vmulq_u32(x48[0], t823[0]), vmulq_u32(x48[1], t823[1])];
    let t825 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x40, t824, 4u32);
    let t826: [uint32x4_t; 2] = [t825[0], t825[1]];
    x56 = t826;
    let t827: [uint32x4_t; 2] = [vmulq_u32(x48[0], t823[0]), vmulq_u32(x48[1], t823[1])];
    let t828: [uint32x4_t; 2] = [vaddq_u32(t827[0], t823[0]), vaddq_u32(t827[1], t823[1])];
    let t829 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x40, t828, 4u32);
    let t830: [uint32x4_t; 2] = [t829[0], t829[1]];
    x64 = t830;
    let t831 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    x72 = ((t831).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
    let t832: [uint32x4_t; 2] = [vmulq_u32(x48[0], t823[0]), vmulq_u32(x48[1], t823[1])];
    let t833 = [vdupq_n_u32(x72); 2];
    let t834: [uint32x4_t; 2] = [vsubq_u32(t833[0], t832[0]), vsubq_u32(t833[1], t832[1])];
    let t835 = b16_sl_p(l, lid0, live, 4u32, t834);
    x80 = t835;
    let t836: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
    let t837 = [vdupq_n_u32(3u32); 2];
    let t838: [uint32x4_t; 2] = [vandq_u32(t836[0], t837[0]), vandq_u32(t836[1], t837[1])];
    x88 = t838;
    let t839 = [vdupq_n_u32(0u32); 2];
    let t840: [uint32x4_t; 2] = [vceqq_u32(x88[0], t839[0]), vceqq_u32(x88[1], t839[1])];
    let mut t841: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t842: [uint32x4_t; 2] = [vandq_u32(live[0], t840[0]), vandq_u32(live[1], t840[1])];
    let t843: [uint32x4_t; 2] = [vbicq_u32(live[0], t840[0]), vbicq_u32(live[1], t840[1])];
    {
        t841 = [vbslq_u32(t842[0], t839[0], t841[0]), vbslq_u32(t842[1], t839[1], t841[1])];
    }
    if any(&t843) {
        let t844 = [vdupq_n_u32(1u32); 2];
        let t845: [uint32x4_t; 2] = [vsubq_u32(x88[0], t844[0]), vsubq_u32(x88[1], t844[1])];
        let t846 = [vdupq_n_u32(2u32); 2];
        let t847: [uint32x4_t; 2] = [vmulq_u32(t846[0], t845[0]), vmulq_u32(t846[1], t845[1])];
        let t848: [uint32x4_t; 2] = [vaddq_u32(x56[0], t847[0]), vaddq_u32(x56[1], t847[1])];
        let t849 = b17_ll_p(l, lid0, t843, t848, x64);
        let t850 = b15_r0r1sls_p(l, lid0, t843, arg0, arg1, x32, t849, 2u32);
        let t851: [uint32x4_t; 2] = [t850[0], t850[1]];
        t841 = [vbslq_u32(t843[0], t851[0], t841[0]), vbslq_u32(t843[1], t851[1], t841[1])];
    }
    x96 = t841;
    let t852 = [vdupq_n_u32(2u32); 2];
    let t853: [uint32x4_t; 2] = [vmulq_u32(t852[0], x88[0]), vmulq_u32(t852[1], x88[1])];
    let t854: [uint32x4_t; 2] = [vaddq_u32(x56[0], t853[0]), vaddq_u32(x56[1], t853[1])];
    let t855 = b17_ll_p(l, lid0, live, t854, x64);
    let t856 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t855, 2u32);
    let t857: [uint32x4_t; 2] = [t856[0], t856[1]];
    x104 = t857;
    let t858: [uint32x4_t; 2] = [vmulq_u32(t852[0], x80[0]), vmulq_u32(t852[1], x80[1])];
    let t859: [uint32x4_t; 2] = [vaddq_u32(x56[0], t858[0]), vaddq_u32(x56[1], t858[1])];
    x112 = t859;
    let t860 = [vdupq_n_u32(255u32); 2];
    let t861: [uint32x4_t; 2] = [vandq_u32(x24[0], t860[0]), vandq_u32(x24[1], t860[1])];
    x120 = t861;
    x128 = 9259542123273814144u64;
    let t862: u64 = 72340172838076673u64;
    let t863 = [vdupq_n_u32((t862) as u32); 2];
    let t864 = [vdupq_n_u32(((t862) >> 32) as u32); 2];
    let t865: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x120[0]), vget_low_u32(t863[0]))), vreinterpretq_u32_u64(vmull_high_u32(x120[0], t863[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x120[1]), vget_low_u32(t863[1]))), vreinterpretq_u32_u64(vmull_high_u32(x120[1], t863[1])))];
    let t866: [uint32x4_t; 4] = [vmulq_u32(x120[0], t863[0]), vmulq_u32(x120[1], t863[1]), vmlaq_u32(t865[0], x120[0], t864[0]), vmlaq_u32(t865[1], x120[1], t864[1])];
    x136 = t866;
    x144 = x96;
    x152 = x96;
    let mut t868 = live;
    loop {
        // Lanes part: each its own rounds.
        let t870: [uint32x4_t; 2] = [vcltq_u32(x152[0], x104[0]), vcltq_u32(x152[1], x104[1])];
        t868 = [vandq_u32(t868[0], t870[0]), vandq_u32(t868[1], t870[1])];
        if !(any(&t868)) { break; }
        let t872: [uint32x4_t; 2] = [vaddq_u32(x112[0], x152[0]), vaddq_u32(x112[1], x152[1])];
        let t873 = b17_ll_p(l, lid0, t868, t872, x64);
        let t874 = b15_r0r1sls_p(l, lid0, t868, arg0, arg1, x32, t873, 8u32);
        x160 = [vbslq_u32(t868[0], t874[0], x160[0]), vbslq_u32(t868[1], t874[1], x160[1]), vbslq_u32(t868[0], t874[2], x160[2]), vbslq_u32(t868[1], t874[3], x160[3])];
        let t875: u64 = x128;
        let t876 = [vdupq_n_u32((t875) as u32); 2];
        let t877 = [vdupq_n_u32(((t875) >> 32) as u32); 2];
        let t878: [uint32x4_t; 4] = [vorrq_u32(x160[0], t876[0]), vorrq_u32(x160[1], t876[1]), vorrq_u32(x160[2], t877[0]), vorrq_u32(x160[3], t877[1])];
        let t879: u64 = !(x128);
        let t880 = [vdupq_n_u32((t879) as u32); 2];
        let t881 = [vdupq_n_u32(((t879) >> 32) as u32); 2];
        let t882: [uint32x4_t; 4] = [vandq_u32(x136[0], t880[0]), vandq_u32(x136[1], t880[1]), vandq_u32(x136[2], t881[0]), vandq_u32(x136[3], t881[1])];
        let t883: [uint32x4_t; 2] = [vcltq_u32(t878[0], t882[0]), vcltq_u32(t878[1], t882[1])];
        let t884: [uint32x4_t; 4] = [vsubq_u32(t878[0], t882[0]), vsubq_u32(t878[1], t882[1]), vaddq_u32(vsubq_u32(t878[2], t882[2]), t883[0]), vaddq_u32(vsubq_u32(t878[3], t882[3]), t883[1])];
        x168 = [vbslq_u32(t868[0], t884[0], x168[0]), vbslq_u32(t868[1], t884[1], x168[1]), vbslq_u32(t868[0], t884[2], x168[2]), vbslq_u32(t868[1], t884[3], x168[3])];
        let t885: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x160[r]));
        let t886: [uint32x4_t; 4] = [vandq_u32(t885[0], x136[0]), vandq_u32(t885[1], x136[1]), vandq_u32(t885[2], x136[2]), vandq_u32(t885[3], x136[3])];
        let t887: [uint32x4_t; 4] = [veorq_u32(x160[0], x136[0]), veorq_u32(x160[1], x136[1]), veorq_u32(x160[2], x136[2]), veorq_u32(x160[3], x136[3])];
        let t888: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(t887[r]));
        let t889: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x168[r]));
        let t890: [uint32x4_t; 4] = [vandq_u32(t888[0], t889[0]), vandq_u32(t888[1], t889[1]), vandq_u32(t888[2], t889[2]), vandq_u32(t888[3], t889[3])];
        let t891: [uint32x4_t; 4] = [vorrq_u32(t886[0], t890[0]), vorrq_u32(t886[1], t890[1]), vorrq_u32(t886[2], t890[2]), vorrq_u32(t886[3], t890[3])];
        let t892: u64 = x128;
        let t893 = [vdupq_n_u32((t892) as u32); 2];
        let t894 = [vdupq_n_u32(((t892) >> 32) as u32); 2];
        let t895: [uint32x4_t; 4] = [vandq_u32(t893[0], t891[0]), vandq_u32(t893[1], t891[1]), vandq_u32(t894[0], t891[2]), vandq_u32(t894[1], t891[3])];
        x176 = t895;
        let t896: [uint32x4_t; 2] = [vsubq_u32(x104[0], x152[0]), vsubq_u32(x104[1], x152[1])];
        x184 = t896;
        let t897 = [vdupq_n_u32(8u32); 2];
        let t898: [uint32x4_t; 2] = [vcgeq_u32(x184[0], t897[0]), vcgeq_u32(x184[1], t897[1])];
        let mut t899: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t900: [uint32x4_t; 2] = [vandq_u32(t868[0], t898[0]), vandq_u32(t868[1], t898[1])];
        let t901: [uint32x4_t; 2] = [vbicq_u32(t868[0], t898[0]), vbicq_u32(t868[1], t898[1])];
        {
            t899 = [vbslq_u32(t900[0], x176[0], t899[0]), vbslq_u32(t900[1], x176[1], t899[1]), vbslq_u32(t900[0], x176[2], t899[2]), vbslq_u32(t900[1], x176[3], t899[3])];
        }
        {
            let t902: [uint32x4_t; 2] = [vmulq_u32(t897[0], x184[0]), vmulq_u32(t897[1], x184[1])];
            let t903: u64 = 1u64;
            let t904 = [vdupq_n_u32((t903) as u32); 2];
            let t905: [uint32x4_t; 2] = [t902[0], t902[1]];
            let t906: [uint32x4_t; 2] = [vandq_u32(t905[0], vdupq_n_u32(63)), vandq_u32(t905[1], vdupq_n_u32(63))];
            let t907: [uint32x4_t; 4] = [vshlq_u32(t904[0], vreinterpretq_s32_u32(t906[0])), vshlq_u32(t904[1], vreinterpretq_s32_u32(t906[1])), vshlq_u32(t904[0], vsubq_s32(vreinterpretq_s32_u32(t906[0]), vdupq_n_s32(32))), vshlq_u32(t904[1], vsubq_s32(vreinterpretq_s32_u32(t906[1]), vdupq_n_s32(32)))];
            let t908: u64 = 1u64;
            let t909 = [vdupq_n_u32((t908) as u32); 2];
            let t910: [uint32x4_t; 2] = [vcltq_u32(t907[0], t909[0]), vcltq_u32(t907[1], t909[1])];
            let t911: [uint32x4_t; 4] = [vsubq_u32(t907[0], t909[0]), vsubq_u32(t907[1], t909[1]), vaddq_u32(vsubq_u32(t907[2], vdupq_n_u32(0)), t910[0]), vaddq_u32(vsubq_u32(t907[3], vdupq_n_u32(0)), t910[1])];
            let t912: [uint32x4_t; 4] = [vandq_u32(x176[0], t911[0]), vandq_u32(x176[1], t911[1]), vandq_u32(x176[2], t911[2]), vandq_u32(x176[3], t911[3])];
            t899 = [vbslq_u32(t901[0], t912[0], t899[0]), vbslq_u32(t901[1], t912[1], t899[1]), vbslq_u32(t901[0], t912[2], t899[2]), vbslq_u32(t901[1], t912[3], t899[3])];
        }
        let t913 = b18_l_p(l, lid0, t868, t899);
        let t914: [uint32x4_t; 2] = [vaddq_u32(x144[0], t913[0]), vaddq_u32(x144[1], t913[1])];
        x144 = [vbslq_u32(t868[0], t914[0], x144[0]), vbslq_u32(t868[1], t914[1], x144[1])];
        let t915: [uint32x4_t; 2] = [vandq_u32(t868[0], live[0]), vandq_u32(t868[1], live[1])];
        x152 = [vaddq_u32(x152[0], vandq_u32(t915[0], vdupq_n_u32(8))), vaddq_u32(x152[1], vandq_u32(t915[1], vdupq_n_u32(8)))];
    }
    x96 = x144;
    let t916 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    let t917 = [vdupq_n_u32((((t916).wrapping_shr((((((4u32).wrapping_mul(x16)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32))); 2];
    x152 = t917;
    let t918 = [vdupq_n_u32(1u32); 2];
    let t919: [uint32x4_t; 2] = [vsubq_u32(x80[0], t918[0]), vsubq_u32(x80[1], t918[1])];
    let t920: [uint32x4_t; 2] = [vmulq_u32(t852[0], t919[0]), vmulq_u32(t852[1], t919[1])];
    let t921: [uint32x4_t; 2] = [vaddq_u32(x56[0], t920[0]), vaddq_u32(x56[1], t920[1])];
    let t922 = b17_ll_p(l, lid0, live, t921, x64);
    let t923 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t922, 2u32);
    let t924: [uint32x4_t; 2] = [t923[0], t923[1]];
    let t925: [uint32x4_t; 4] = [t924[0], t924[1], vdupq_n_u32(0), vdupq_n_u32(0)];
    x160 = t925;
    let t926: [uint32x4_t; 2] = [x160[0], x160[1]];
    let t927: [uint32x4_t; 2] = [vaddq_u32(x112[0], t926[0]), vaddq_u32(x112[1], t926[1])];
    let t928: [uint32x4_t; 2] = [vmulq_u32(x96[0], x152[0]), vmulq_u32(x96[1], x152[1])];
    let t929: [uint32x4_t; 2] = [vaddq_u32(t927[0], t928[0]), vaddq_u32(t927[1], t928[1])];
    let t930 = b17_ll_p(l, lid0, live, t929, x64);
    let t931: [uint32x4_t; 4] = [t930[0], t930[1], vdupq_n_u32(0), vdupq_n_u32(0)];
    x168 = t931;
    let t932: [uint32x4_t; 2] = [x168[0], x168[1]];
    let t933 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t932, 8u32);
    let t934 = b8_l_p(l, lid0, live, x152);
    let t935: [uint32x4_t; 4] = [vandq_u32(t933[0], t934[0]), vandq_u32(t933[1], t934[1]), vandq_u32(t933[2], t934[2]), vandq_u32(t933[3], t934[3])];
    res = t935;
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
    let t936 = b11_r0r1s_p(l, lid0, live, arg0, arg1, (20u32).wrapping_add((4u32).wrapping_mul(x16)));
    res = t936;
    return res;
}

#[inline(always)]
unsafe fn b12_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t937 = b11_r0r1s_f(l, lid0, arg0, arg1, (20u32).wrapping_add((4u32).wrapping_mul(x16)));
    res = t937;
    return res;
}

#[inline(always)]
unsafe fn b13_ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x8: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t938: u32;
    if ((x0) < (x8)) {
        t938 = x0;
    } else {
        t938 = x8;
    }
    res = t938;
    return res;
}

#[inline(always)]
unsafe fn b13_ss_f(l: &Launch, lid0: u32, x0: u32, x8: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t939: u32;
    if ((x0) < (x8)) {
        t939 = x0;
    } else {
        t939 = x8;
    }
    res = t939;
    return res;
}

#[inline(always)]
unsafe fn b13_ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x8: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t940 = [vdupq_n_u32(x8); 2];
    let t941: [uint32x4_t; 2] = [vcltq_u32(x0[0], t940[0]), vcltq_u32(x0[1], t940[1])];
    let mut t942: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t943: [uint32x4_t; 2] = [vandq_u32(live[0], t941[0]), vandq_u32(live[1], t941[1])];
    let t944: [uint32x4_t; 2] = [vbicq_u32(live[0], t941[0]), vbicq_u32(live[1], t941[1])];
    {
        t942 = [vbslq_u32(t943[0], x0[0], t942[0]), vbslq_u32(t943[1], x0[1], t942[1])];
    }
    {
        let t945 = [vdupq_n_u32(x8); 2];
        t942 = [vbslq_u32(t944[0], t945[0], t942[0]), vbslq_u32(t944[1], t945[1], t942[1])];
    }
    res = t942;
    return res;
}

#[inline(always)]
unsafe fn b13_ls_f(l: &Launch, lid0: u32, x0: [uint32x4_t; 2], x8: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t946 = [vdupq_n_u32(x8); 2];
    let t947: [uint32x4_t; 2] = [vcltq_u32(x0[0], t946[0]), vcltq_u32(x0[1], t946[1])];
    let mut t948: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t949: [uint32x4_t; 2] = [vmvnq_u32(t947[0]), vmvnq_u32(t947[1])];
    {
        t948 = [vbslq_u32(t947[0], x0[0], t948[0]), vbslq_u32(t947[1], x0[1], t948[1])];
    }
    {
        let t950 = [vdupq_n_u32(x8); 2];
        t948 = [vbslq_u32(t949[0], t950[0], t948[0]), vbslq_u32(t949[1], t950[1], t948[1])];
    }
    res = t948;
    return res;
}

#[inline(always)]
unsafe fn b14_r0sns_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: u32, x16f: u32, x16s: u32, x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t951 = b19_r0an1s_p(l, lid0, live, arg0, ((((x8) as u64)) as usize as *const u8), x16f, x16s, x24);
    x32 = t951;
    let t952: [uint32x4_t; 4];
    if ((x24) >= (8u32)) {
        t952 = x32;
    } else {
        let t953 = b20_s_p(l, lid0, live, (8u32).wrapping_mul(x24));
        let t954: u64 = t953;
        let t955 = [vdupq_n_u32((t954) as u32); 2];
        let t956 = [vdupq_n_u32(((t954) >> 32) as u32); 2];
        let t957: [uint32x4_t; 4] = [vandq_u32(x32[0], t955[0]), vandq_u32(x32[1], t955[1]), vandq_u32(x32[2], t956[0]), vandq_u32(x32[3], t956[1])];
        t952 = t957;
    }
    res = t952;
    return res;
}

#[inline(always)]
unsafe fn b14_r0sns_f(l: &Launch, lid0: u32, arg0: *const u8, x8: u32, x16f: u32, x16s: u32, x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t958 = b19_r0an1s_f(l, lid0, arg0, ((((x8) as u64)) as usize as *const u8), x16f, x16s, x24);
    x32 = t958;
    let t959: [uint32x4_t; 4];
    if ((x24) >= (8u32)) {
        t959 = x32;
    } else {
        let t960 = b20_s_f(l, lid0, (8u32).wrapping_mul(x24));
        let t961: u64 = t960;
        let t962 = [vdupq_n_u32((t961) as u32); 2];
        let t963 = [vdupq_n_u32(((t961) >> 32) as u32); 2];
        let t964: [uint32x4_t; 4] = [vandq_u32(x32[0], t962[0]), vandq_u32(x32[1], t962[1]), vandq_u32(x32[2], t963[0]), vandq_u32(x32[3], t963[1])];
        t959 = t964;
    }
    res = t959;
    return res;
}

#[inline(always)]
unsafe fn b14_r0lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: [uint32x4_t; 2], x16: [uint32x4_t; 2], x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t965: [uint32x4_t; 2] = [vaddq_u32(x8[0], x16[0]), vaddq_u32(x8[1], x16[1])];
    let t966: [uint32x4_t; 2] = [vcltq_u32(t965[0], x8[0]), vcltq_u32(t965[1], x8[1])];
    let t967: [uint32x4_t; 4] = [t965[0], t965[1], vshrq_n_u32::<31>(t966[0]), vshrq_n_u32::<31>(t966[1])];
    let t968 = b19_r0ls_p(l, lid0, live, arg0, t967, x24);
    x32 = t968;
    let t969: [uint32x4_t; 4];
    if ((x24) >= (8u32)) {
        t969 = x32;
    } else {
        let t970 = b20_s_p(l, lid0, live, (8u32).wrapping_mul(x24));
        let t971: u64 = t970;
        let t972 = [vdupq_n_u32((t971) as u32); 2];
        let t973 = [vdupq_n_u32(((t971) >> 32) as u32); 2];
        let t974: [uint32x4_t; 4] = [vandq_u32(x32[0], t972[0]), vandq_u32(x32[1], t972[1]), vandq_u32(x32[2], t973[0]), vandq_u32(x32[3], t973[1])];
        t969 = t974;
    }
    res = t969;
    return res;
}

#[inline(always)]
unsafe fn b14_r0lls_f(l: &Launch, lid0: u32, arg0: *const u8, x8: [uint32x4_t; 2], x16: [uint32x4_t; 2], x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t975: [uint32x4_t; 2] = [vaddq_u32(x8[0], x16[0]), vaddq_u32(x8[1], x16[1])];
    let t976: [uint32x4_t; 2] = [vcltq_u32(t975[0], x8[0]), vcltq_u32(t975[1], x8[1])];
    let t977: [uint32x4_t; 4] = [t975[0], t975[1], vshrq_n_u32::<31>(t976[0]), vshrq_n_u32::<31>(t976[1])];
    let t978 = b19_r0ls_f(l, lid0, arg0, t977, x24);
    x32 = t978;
    let t979: [uint32x4_t; 4];
    if ((x24) >= (8u32)) {
        t979 = x32;
    } else {
        let t980 = b20_s_f(l, lid0, (8u32).wrapping_mul(x24));
        let t981: u64 = t980;
        let t982 = [vdupq_n_u32((t981) as u32); 2];
        let t983 = [vdupq_n_u32(((t981) >> 32) as u32); 2];
        let t984: [uint32x4_t; 4] = [vandq_u32(x32[0], t982[0]), vandq_u32(x32[1], t982[1]), vandq_u32(x32[2], t983[0]), vandq_u32(x32[3], t983[1])];
        t979 = t984;
    }
    res = t979;
    return res;
}

#[inline(always)]
unsafe fn b15_r0r1sss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: u32, x32: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t985 = b21_r0sss_p(l, lid0, live, arg0, x16, x24, x32);
    res = t985;
    return res;
}

#[inline(always)]
unsafe fn b15_r0r1sss_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32, x24: u32, x32: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t986 = b21_r0sss_f(l, lid0, arg0, x16, x24, x32);
    res = t986;
    return res;
}

#[inline(always)]
unsafe fn b15_r0r1sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2], x32: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t987 = b21_r0sls_p(l, lid0, live, arg0, x16, x24, x32);
    res = t987;
    return res;
}

#[inline(always)]
unsafe fn b16_ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x8: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t988: u32;
    if ((x0) < (x8)) {
        t988 = x0;
    } else {
        t988 = x8;
    }
    res = t988;
    return res;
}

#[inline(always)]
unsafe fn b16_ss_f(l: &Launch, lid0: u32, x0: u32, x8: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t989: u32;
    if ((x0) < (x8)) {
        t989 = x0;
    } else {
        t989 = x8;
    }
    res = t989;
    return res;
}

#[inline(always)]
unsafe fn b16_sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x8: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t990 = [vdupq_n_u32(x0); 2];
    let t991: [uint32x4_t; 2] = [vcltq_u32(t990[0], x8[0]), vcltq_u32(t990[1], x8[1])];
    let mut t992: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t993: [uint32x4_t; 2] = [vandq_u32(live[0], t991[0]), vandq_u32(live[1], t991[1])];
    let t994: [uint32x4_t; 2] = [vbicq_u32(live[0], t991[0]), vbicq_u32(live[1], t991[1])];
    {
        let t995 = [vdupq_n_u32(x0); 2];
        t992 = [vbslq_u32(t993[0], t995[0], t992[0]), vbslq_u32(t993[1], t995[1], t992[1])];
    }
    {
        t992 = [vbslq_u32(t994[0], x8[0], t992[0]), vbslq_u32(t994[1], x8[1], t992[1])];
    }
    res = t992;
    return res;
}

#[inline(always)]
unsafe fn b17_ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x8: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t996: u32;
    if ((x0) < (x8)) {
        t996 = x0;
    } else {
        t996 = x8;
    }
    res = t996;
    return res;
}

#[inline(always)]
unsafe fn b17_ss_f(l: &Launch, lid0: u32, x0: u32, x8: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t997: u32;
    if ((x0) < (x8)) {
        t997 = x0;
    } else {
        t997 = x8;
    }
    res = t997;
    return res;
}

#[inline(always)]
unsafe fn b17_ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x8: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t998 = [vdupq_n_u32(x8); 2];
    let t999: [uint32x4_t; 2] = [vcltq_u32(x0[0], t998[0]), vcltq_u32(x0[1], t998[1])];
    let mut t1000: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1001: [uint32x4_t; 2] = [vandq_u32(live[0], t999[0]), vandq_u32(live[1], t999[1])];
    let t1002: [uint32x4_t; 2] = [vbicq_u32(live[0], t999[0]), vbicq_u32(live[1], t999[1])];
    {
        t1000 = [vbslq_u32(t1001[0], x0[0], t1000[0]), vbslq_u32(t1001[1], x0[1], t1000[1])];
    }
    {
        let t1003 = [vdupq_n_u32(x8); 2];
        t1000 = [vbslq_u32(t1002[0], t1003[0], t1000[0]), vbslq_u32(t1002[1], t1003[1], t1000[1])];
    }
    res = t1000;
    return res;
}

#[inline(always)]
unsafe fn b17_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x8: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1004: [uint32x4_t; 2] = [vcltq_u32(x0[0], x8[0]), vcltq_u32(x0[1], x8[1])];
    let mut t1005: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1006: [uint32x4_t; 2] = [vandq_u32(live[0], t1004[0]), vandq_u32(live[1], t1004[1])];
    let t1007: [uint32x4_t; 2] = [vbicq_u32(live[0], t1004[0]), vbicq_u32(live[1], t1004[1])];
    {
        t1005 = [vbslq_u32(t1006[0], x0[0], t1005[0]), vbslq_u32(t1006[1], x0[1], t1005[1])];
    }
    {
        t1005 = [vbslq_u32(t1007[0], x8[0], t1005[0]), vbslq_u32(t1007[1], x8[1], t1005[1])];
    }
    res = t1005;
    return res;
}

#[inline(always)]
unsafe fn b18_s_f(l: &Launch, lid0: u32, x0: u64) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    res = (((((x0) as u64).count_ones() as u64)) as u32);
    return res;
}

#[inline(always)]
unsafe fn b18_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1008: [uint32x4_t; 2] = [vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[0])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[2]))))), vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[1])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[3])))))];
    res = t1008;
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
    let mut t1009: bool = ((x32) != (0u32));
    if t1009 {
        t1009 = (((x32).wrapping_add((x16).wrapping_mul(8u32))) > (64u32));
    }
    let t1010: u64;
    if t1009 {
        t1010 = ((x40) | (((x24.wrapping_add((1u64).wrapping_mul(8u64) as usize) as *const u64).read_unaligned()).wrapping_shl(((((64u32).wrapping_sub(x32)) as u64)) as u32)));
    } else {
        t1010 = x40;
    }
    res = t1010;
    return res;
}

#[inline(always)]
unsafe fn b19_r0ss_f(l: &Launch, lid0: u32, arg0: *const u8, x8: u64, x16: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: *const u8 = core::ptr::null();
    let mut x32: u32 = 0;
    let mut x40: u64 = 0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    if ((x16) == (8u32)) {
        res = (arg0.wrapping_add(x8 as usize) as *const u64).read_unaligned();
        return res;
    }
    x24 = arg0.wrapping_add(((x8) & (!(7u64))) as usize);
    x32 = (((((x8) & (7u64))) as u32)).wrapping_mul(8u32);
    x40 = ((x24 as *const u64).read_unaligned()).wrapping_shr((((x32) as u64)) as u32);
    let mut t1011: bool = ((x32) != (0u32));
    if t1011 {
        t1011 = (((x32).wrapping_add((x16).wrapping_mul(8u32))) > (64u32));
    }
    let t1012: u64;
    if t1011 {
        t1012 = ((x40) | (((x24.wrapping_add((1u64).wrapping_mul(8u64) as usize) as *const u64).read_unaligned()).wrapping_shl(((((64u32).wrapping_sub(x32)) as u64)) as u32)));
    } else {
        t1012 = x40;
    }
    res = t1012;
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
        let t1013: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
        let t1014: [uint32x4_t; 4] = [vandq_u32(x8[0], t1013[0]), vandq_u32(x8[1], t1013[1]), vandq_u32(x8[2], t1013[2]), vandq_u32(x8[3], t1013[3])];
        let t1015: u64 = ((arg0) as usize as u64);
        let t1016 = [vdupq_n_u32((t1015) as u32); 2];
        let t1017 = [vdupq_n_u32(((t1015) >> 32) as u32); 2];
        let t1018: [uint32x4_t; 2] = [vaddq_u32(t1016[0], t1014[0]), vaddq_u32(t1016[1], t1014[1])];
        let t1019: [uint32x4_t; 2] = [vcltq_u32(t1018[0], t1016[0]), vcltq_u32(t1018[1], t1016[1])];
        let t1020: [uint32x4_t; 4] = [t1018[0], t1018[1], vsubq_u32(vaddq_u32(t1017[0], t1014[2]), t1019[0]), vsubq_u32(vaddq_u32(t1017[1], t1014[3]), t1019[1])];
        let mut t1021: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1020[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1020[0]) as u64) as usize as *const u64).read_unaligned(); t1021[0] = vsetq_lane_u32::<0>(v as u32, t1021[0]); t1021[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1021[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1020[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1020[0]) as u64) as usize as *const u64).read_unaligned(); t1021[0] = vsetq_lane_u32::<1>(v as u32, t1021[0]); t1021[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1021[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1020[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1020[0]) as u64) as usize as *const u64).read_unaligned(); t1021[0] = vsetq_lane_u32::<2>(v as u32, t1021[0]); t1021[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1021[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1020[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1020[0]) as u64) as usize as *const u64).read_unaligned(); t1021[0] = vsetq_lane_u32::<3>(v as u32, t1021[0]); t1021[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1021[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1020[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1020[1]) as u64) as usize as *const u64).read_unaligned(); t1021[1] = vsetq_lane_u32::<0>(v as u32, t1021[1]); t1021[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1021[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1020[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1020[1]) as u64) as usize as *const u64).read_unaligned(); t1021[1] = vsetq_lane_u32::<1>(v as u32, t1021[1]); t1021[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1021[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1020[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1020[1]) as u64) as usize as *const u64).read_unaligned(); t1021[1] = vsetq_lane_u32::<2>(v as u32, t1021[1]); t1021[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1021[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1020[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1020[1]) as u64) as usize as *const u64).read_unaligned(); t1021[1] = vsetq_lane_u32::<3>(v as u32, t1021[1]); t1021[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1021[3]); }
        res = t1021;
        return res;
    }
    let t1022: u64 = !(7u64);
    let t1023 = [vdupq_n_u32((t1022) as u32); 2];
    let t1024 = [vdupq_n_u32(((t1022) >> 32) as u32); 2];
    let t1025: [uint32x4_t; 4] = [vandq_u32(x8[0], t1023[0]), vandq_u32(x8[1], t1023[1]), vandq_u32(x8[2], t1024[0]), vandq_u32(x8[3], t1024[1])];
    let t1026: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
    let t1027: [uint32x4_t; 4] = [vandq_u32(t1025[0], t1026[0]), vandq_u32(t1025[1], t1026[1]), vandq_u32(t1025[2], t1026[2]), vandq_u32(t1025[3], t1026[3])];
    let t1028: u64 = ((arg0) as usize as u64);
    let t1029 = [vdupq_n_u32((t1028) as u32); 2];
    let t1030 = [vdupq_n_u32(((t1028) >> 32) as u32); 2];
    let t1031: [uint32x4_t; 2] = [vaddq_u32(t1029[0], t1027[0]), vaddq_u32(t1029[1], t1027[1])];
    let t1032: [uint32x4_t; 2] = [vcltq_u32(t1031[0], t1029[0]), vcltq_u32(t1031[1], t1029[1])];
    let t1033: [uint32x4_t; 4] = [t1031[0], t1031[1], vsubq_u32(vaddq_u32(t1030[0], t1027[2]), t1032[0]), vsubq_u32(vaddq_u32(t1030[1], t1027[3]), t1032[1])];
    x24 = t1033;
    let t1034: u64 = 7u64;
    let t1035 = [vdupq_n_u32((t1034) as u32); 2];
    let t1036: [uint32x4_t; 2] = [vandq_u32(x8[0], t1035[0]), vandq_u32(x8[1], t1035[1])];
    let t1037 = [vdupq_n_u32(8u32); 2];
    let t1038: [uint32x4_t; 2] = [vmulq_u32(t1036[0], t1037[0]), vmulq_u32(t1036[1], t1037[1])];
    x32 = t1038;
    let mut t1039: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x24[2]) as u64) << 32 | vgetq_lane_u32::<0>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1039[0] = vsetq_lane_u32::<0>(v as u32, t1039[0]); t1039[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1039[2]); }
    { let v = (((vgetq_lane_u32::<1>(x24[2]) as u64) << 32 | vgetq_lane_u32::<1>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1039[0] = vsetq_lane_u32::<1>(v as u32, t1039[0]); t1039[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1039[2]); }
    { let v = (((vgetq_lane_u32::<2>(x24[2]) as u64) << 32 | vgetq_lane_u32::<2>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1039[0] = vsetq_lane_u32::<2>(v as u32, t1039[0]); t1039[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1039[2]); }
    { let v = (((vgetq_lane_u32::<3>(x24[2]) as u64) << 32 | vgetq_lane_u32::<3>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1039[0] = vsetq_lane_u32::<3>(v as u32, t1039[0]); t1039[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1039[2]); }
    { let v = (((vgetq_lane_u32::<0>(x24[3]) as u64) << 32 | vgetq_lane_u32::<0>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1039[1] = vsetq_lane_u32::<0>(v as u32, t1039[1]); t1039[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1039[3]); }
    { let v = (((vgetq_lane_u32::<1>(x24[3]) as u64) << 32 | vgetq_lane_u32::<1>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1039[1] = vsetq_lane_u32::<1>(v as u32, t1039[1]); t1039[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1039[3]); }
    { let v = (((vgetq_lane_u32::<2>(x24[3]) as u64) << 32 | vgetq_lane_u32::<2>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1039[1] = vsetq_lane_u32::<2>(v as u32, t1039[1]); t1039[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1039[3]); }
    { let v = (((vgetq_lane_u32::<3>(x24[3]) as u64) << 32 | vgetq_lane_u32::<3>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1039[1] = vsetq_lane_u32::<3>(v as u32, t1039[1]); t1039[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1039[3]); }
    let t1040: [uint32x4_t; 2] = [x32[0], x32[1]];
    let t1041: [uint32x4_t; 2] = [vandq_u32(t1040[0], vdupq_n_u32(63)), vandq_u32(t1040[1], vdupq_n_u32(63))];
    let t1042: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t1039[0], vnegq_s32(vreinterpretq_s32_u32(t1041[0]))), vshlq_u32(t1039[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1041[0])))), vorrq_u32(vshlq_u32(t1039[1], vnegq_s32(vreinterpretq_s32_u32(t1041[1]))), vshlq_u32(t1039[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1041[1])))), vshlq_u32(t1039[2], vnegq_s32(vreinterpretq_s32_u32(t1041[0]))), vshlq_u32(t1039[3], vnegq_s32(vreinterpretq_s32_u32(t1041[1])))];
    x40 = t1042;
    let t1043 = [vdupq_n_u32(0u32); 2];
    let t1044: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t1043[0])), vmvnq_u32(vceqq_u32(x32[1], t1043[1]))];
    let t1045 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t1046: [uint32x4_t; 2] = [vaddq_u32(x32[0], t1045[0]), vaddq_u32(x32[1], t1045[1])];
    let t1047 = [vdupq_n_u32(64u32); 2];
    let t1048: [uint32x4_t; 2] = [vcgtq_u32(t1046[0], t1047[0]), vcgtq_u32(t1046[1], t1047[1])];
    let t1049: [uint32x4_t; 2] = [vandq_u32(t1044[0], t1048[0]), vandq_u32(t1044[1], t1048[1])];
    let mut t1050: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1051: [uint32x4_t; 2] = [vandq_u32(live[0], t1049[0]), vandq_u32(live[1], t1049[1])];
    let t1052: [uint32x4_t; 2] = [vbicq_u32(live[0], t1049[0]), vbicq_u32(live[1], t1049[1])];
    if any(&t1051) {
        let t1053: u64 = (1u64).wrapping_mul(8u64);
        let t1054 = [vdupq_n_u32((t1053) as u32); 2];
        let t1055 = [vdupq_n_u32(((t1053) >> 32) as u32); 2];
        let t1056: [uint32x4_t; 2] = [vaddq_u32(x24[0], t1054[0]), vaddq_u32(x24[1], t1054[1])];
        let t1057: [uint32x4_t; 2] = [vcltq_u32(t1056[0], x24[0]), vcltq_u32(t1056[1], x24[1])];
        let t1058: [uint32x4_t; 4] = [t1056[0], t1056[1], vsubq_u32(vaddq_u32(x24[2], t1055[0]), t1057[0]), vsubq_u32(vaddq_u32(x24[3], t1055[1]), t1057[1])];
        let mut t1059: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1058[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1058[0]) as u64) as usize as *const u64).read_unaligned(); t1059[0] = vsetq_lane_u32::<0>(v as u32, t1059[0]); t1059[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1059[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1058[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1058[0]) as u64) as usize as *const u64).read_unaligned(); t1059[0] = vsetq_lane_u32::<1>(v as u32, t1059[0]); t1059[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1059[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1058[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1058[0]) as u64) as usize as *const u64).read_unaligned(); t1059[0] = vsetq_lane_u32::<2>(v as u32, t1059[0]); t1059[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1059[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1058[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1058[0]) as u64) as usize as *const u64).read_unaligned(); t1059[0] = vsetq_lane_u32::<3>(v as u32, t1059[0]); t1059[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1059[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1058[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1058[1]) as u64) as usize as *const u64).read_unaligned(); t1059[1] = vsetq_lane_u32::<0>(v as u32, t1059[1]); t1059[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1059[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1058[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1058[1]) as u64) as usize as *const u64).read_unaligned(); t1059[1] = vsetq_lane_u32::<1>(v as u32, t1059[1]); t1059[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1059[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1058[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1058[1]) as u64) as usize as *const u64).read_unaligned(); t1059[1] = vsetq_lane_u32::<2>(v as u32, t1059[1]); t1059[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1059[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1058[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1058[1]) as u64) as usize as *const u64).read_unaligned(); t1059[1] = vsetq_lane_u32::<3>(v as u32, t1059[1]); t1059[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1059[3]); }
        let t1060: [uint32x4_t; 2] = [vsubq_u32(t1047[0], x32[0]), vsubq_u32(t1047[1], x32[1])];
        let t1061: [uint32x4_t; 2] = [t1060[0], t1060[1]];
        let t1062: [uint32x4_t; 2] = [vandq_u32(t1061[0], vdupq_n_u32(63)), vandq_u32(t1061[1], vdupq_n_u32(63))];
        let t1063: [uint32x4_t; 4] = [vshlq_u32(t1059[0], vreinterpretq_s32_u32(t1062[0])), vshlq_u32(t1059[1], vreinterpretq_s32_u32(t1062[1])), vorrq_u32(vshlq_u32(t1059[2], vreinterpretq_s32_u32(t1062[0])), vshlq_u32(t1059[0], vsubq_s32(vreinterpretq_s32_u32(t1062[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1059[3], vreinterpretq_s32_u32(t1062[1])), vshlq_u32(t1059[1], vsubq_s32(vreinterpretq_s32_u32(t1062[1]), vdupq_n_s32(32))))];
        let t1064: [uint32x4_t; 4] = [vorrq_u32(x40[0], t1063[0]), vorrq_u32(x40[1], t1063[1]), vorrq_u32(x40[2], t1063[2]), vorrq_u32(x40[3], t1063[3])];
        t1050 = [vbslq_u32(t1051[0], t1064[0], t1050[0]), vbslq_u32(t1051[1], t1064[1], t1050[1]), vbslq_u32(t1051[0], t1064[2], t1050[2]), vbslq_u32(t1051[1], t1064[3], t1050[3])];
    }
    {
        t1050 = [vbslq_u32(t1052[0], x40[0], t1050[0]), vbslq_u32(t1052[1], x40[1], t1050[1]), vbslq_u32(t1052[0], x40[2], t1050[2]), vbslq_u32(t1052[1], x40[3], t1050[3])];
    }
    res = t1050;
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
        let t1065: u64 = ((arg0) as usize as u64);
        let t1066 = [vdupq_n_u32((t1065) as u32); 2];
        let t1067 = [vdupq_n_u32(((t1065) >> 32) as u32); 2];
        let t1068: [uint32x4_t; 2] = [vaddq_u32(t1066[0], x8[0]), vaddq_u32(t1066[1], x8[1])];
        let t1069: [uint32x4_t; 2] = [vcltq_u32(t1068[0], t1066[0]), vcltq_u32(t1068[1], t1066[1])];
        let t1070: [uint32x4_t; 4] = [t1068[0], t1068[1], vsubq_u32(vaddq_u32(t1067[0], x8[2]), t1069[0]), vsubq_u32(vaddq_u32(t1067[1], x8[3]), t1069[1])];
        let mut t1071: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1070[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1070[0]) as u64) as usize as *const u64).read_unaligned(); t1071[0] = vsetq_lane_u32::<0>(v as u32, t1071[0]); t1071[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1071[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1070[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1070[0]) as u64) as usize as *const u64).read_unaligned(); t1071[0] = vsetq_lane_u32::<1>(v as u32, t1071[0]); t1071[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1071[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1070[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1070[0]) as u64) as usize as *const u64).read_unaligned(); t1071[0] = vsetq_lane_u32::<2>(v as u32, t1071[0]); t1071[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1071[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1070[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1070[0]) as u64) as usize as *const u64).read_unaligned(); t1071[0] = vsetq_lane_u32::<3>(v as u32, t1071[0]); t1071[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1071[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1070[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1070[1]) as u64) as usize as *const u64).read_unaligned(); t1071[1] = vsetq_lane_u32::<0>(v as u32, t1071[1]); t1071[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1071[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1070[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1070[1]) as u64) as usize as *const u64).read_unaligned(); t1071[1] = vsetq_lane_u32::<1>(v as u32, t1071[1]); t1071[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1071[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1070[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1070[1]) as u64) as usize as *const u64).read_unaligned(); t1071[1] = vsetq_lane_u32::<2>(v as u32, t1071[1]); t1071[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1071[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1070[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1070[1]) as u64) as usize as *const u64).read_unaligned(); t1071[1] = vsetq_lane_u32::<3>(v as u32, t1071[1]); t1071[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1071[3]); }
        res = t1071;
        return res;
    }
    let t1072: u64 = !(7u64);
    let t1073 = [vdupq_n_u32((t1072) as u32); 2];
    let t1074 = [vdupq_n_u32(((t1072) >> 32) as u32); 2];
    let t1075: [uint32x4_t; 4] = [vandq_u32(x8[0], t1073[0]), vandq_u32(x8[1], t1073[1]), vandq_u32(x8[2], t1074[0]), vandq_u32(x8[3], t1074[1])];
    let t1076: u64 = ((arg0) as usize as u64);
    let t1077 = [vdupq_n_u32((t1076) as u32); 2];
    let t1078 = [vdupq_n_u32(((t1076) >> 32) as u32); 2];
    let t1079: [uint32x4_t; 2] = [vaddq_u32(t1077[0], t1075[0]), vaddq_u32(t1077[1], t1075[1])];
    let t1080: [uint32x4_t; 2] = [vcltq_u32(t1079[0], t1077[0]), vcltq_u32(t1079[1], t1077[1])];
    let t1081: [uint32x4_t; 4] = [t1079[0], t1079[1], vsubq_u32(vaddq_u32(t1078[0], t1075[2]), t1080[0]), vsubq_u32(vaddq_u32(t1078[1], t1075[3]), t1080[1])];
    x24 = t1081;
    let t1082: u64 = 7u64;
    let t1083 = [vdupq_n_u32((t1082) as u32); 2];
    let t1084: [uint32x4_t; 2] = [vandq_u32(x8[0], t1083[0]), vandq_u32(x8[1], t1083[1])];
    let t1085 = [vdupq_n_u32(8u32); 2];
    let t1086: [uint32x4_t; 2] = [vmulq_u32(t1084[0], t1085[0]), vmulq_u32(t1084[1], t1085[1])];
    x32 = t1086;
    let mut t1087: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x24[2]) as u64) << 32 | vgetq_lane_u32::<0>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1087[0] = vsetq_lane_u32::<0>(v as u32, t1087[0]); t1087[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1087[2]); }
    { let v = (((vgetq_lane_u32::<1>(x24[2]) as u64) << 32 | vgetq_lane_u32::<1>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1087[0] = vsetq_lane_u32::<1>(v as u32, t1087[0]); t1087[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1087[2]); }
    { let v = (((vgetq_lane_u32::<2>(x24[2]) as u64) << 32 | vgetq_lane_u32::<2>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1087[0] = vsetq_lane_u32::<2>(v as u32, t1087[0]); t1087[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1087[2]); }
    { let v = (((vgetq_lane_u32::<3>(x24[2]) as u64) << 32 | vgetq_lane_u32::<3>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1087[0] = vsetq_lane_u32::<3>(v as u32, t1087[0]); t1087[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1087[2]); }
    { let v = (((vgetq_lane_u32::<0>(x24[3]) as u64) << 32 | vgetq_lane_u32::<0>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1087[1] = vsetq_lane_u32::<0>(v as u32, t1087[1]); t1087[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1087[3]); }
    { let v = (((vgetq_lane_u32::<1>(x24[3]) as u64) << 32 | vgetq_lane_u32::<1>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1087[1] = vsetq_lane_u32::<1>(v as u32, t1087[1]); t1087[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1087[3]); }
    { let v = (((vgetq_lane_u32::<2>(x24[3]) as u64) << 32 | vgetq_lane_u32::<2>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1087[1] = vsetq_lane_u32::<2>(v as u32, t1087[1]); t1087[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1087[3]); }
    { let v = (((vgetq_lane_u32::<3>(x24[3]) as u64) << 32 | vgetq_lane_u32::<3>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1087[1] = vsetq_lane_u32::<3>(v as u32, t1087[1]); t1087[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1087[3]); }
    let t1088: [uint32x4_t; 2] = [x32[0], x32[1]];
    let t1089: [uint32x4_t; 2] = [vandq_u32(t1088[0], vdupq_n_u32(63)), vandq_u32(t1088[1], vdupq_n_u32(63))];
    let t1090: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t1087[0], vnegq_s32(vreinterpretq_s32_u32(t1089[0]))), vshlq_u32(t1087[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1089[0])))), vorrq_u32(vshlq_u32(t1087[1], vnegq_s32(vreinterpretq_s32_u32(t1089[1]))), vshlq_u32(t1087[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1089[1])))), vshlq_u32(t1087[2], vnegq_s32(vreinterpretq_s32_u32(t1089[0]))), vshlq_u32(t1087[3], vnegq_s32(vreinterpretq_s32_u32(t1089[1])))];
    x40 = t1090;
    let t1091 = [vdupq_n_u32(0u32); 2];
    let t1092: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t1091[0])), vmvnq_u32(vceqq_u32(x32[1], t1091[1]))];
    let t1093 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t1094: [uint32x4_t; 2] = [vaddq_u32(x32[0], t1093[0]), vaddq_u32(x32[1], t1093[1])];
    let t1095 = [vdupq_n_u32(64u32); 2];
    let t1096: [uint32x4_t; 2] = [vcgtq_u32(t1094[0], t1095[0]), vcgtq_u32(t1094[1], t1095[1])];
    let t1097: [uint32x4_t; 2] = [vandq_u32(t1092[0], t1096[0]), vandq_u32(t1092[1], t1096[1])];
    let mut t1098: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1099: [uint32x4_t; 2] = [vmvnq_u32(t1097[0]), vmvnq_u32(t1097[1])];
    if any(&t1097) {
        let t1100: u64 = (1u64).wrapping_mul(8u64);
        let t1101 = [vdupq_n_u32((t1100) as u32); 2];
        let t1102 = [vdupq_n_u32(((t1100) >> 32) as u32); 2];
        let t1103: [uint32x4_t; 2] = [vaddq_u32(x24[0], t1101[0]), vaddq_u32(x24[1], t1101[1])];
        let t1104: [uint32x4_t; 2] = [vcltq_u32(t1103[0], x24[0]), vcltq_u32(t1103[1], x24[1])];
        let t1105: [uint32x4_t; 4] = [t1103[0], t1103[1], vsubq_u32(vaddq_u32(x24[2], t1102[0]), t1104[0]), vsubq_u32(vaddq_u32(x24[3], t1102[1]), t1104[1])];
        let mut t1106: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1105[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1105[0]) as u64) as usize as *const u64).read_unaligned(); t1106[0] = vsetq_lane_u32::<0>(v as u32, t1106[0]); t1106[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1106[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1105[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1105[0]) as u64) as usize as *const u64).read_unaligned(); t1106[0] = vsetq_lane_u32::<1>(v as u32, t1106[0]); t1106[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1106[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1105[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1105[0]) as u64) as usize as *const u64).read_unaligned(); t1106[0] = vsetq_lane_u32::<2>(v as u32, t1106[0]); t1106[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1106[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1105[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1105[0]) as u64) as usize as *const u64).read_unaligned(); t1106[0] = vsetq_lane_u32::<3>(v as u32, t1106[0]); t1106[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1106[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1105[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1105[1]) as u64) as usize as *const u64).read_unaligned(); t1106[1] = vsetq_lane_u32::<0>(v as u32, t1106[1]); t1106[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1106[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1105[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1105[1]) as u64) as usize as *const u64).read_unaligned(); t1106[1] = vsetq_lane_u32::<1>(v as u32, t1106[1]); t1106[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1106[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1105[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1105[1]) as u64) as usize as *const u64).read_unaligned(); t1106[1] = vsetq_lane_u32::<2>(v as u32, t1106[1]); t1106[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1106[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1105[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1105[1]) as u64) as usize as *const u64).read_unaligned(); t1106[1] = vsetq_lane_u32::<3>(v as u32, t1106[1]); t1106[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1106[3]); }
        let t1107: [uint32x4_t; 2] = [vsubq_u32(t1095[0], x32[0]), vsubq_u32(t1095[1], x32[1])];
        let t1108: [uint32x4_t; 2] = [t1107[0], t1107[1]];
        let t1109: [uint32x4_t; 2] = [vandq_u32(t1108[0], vdupq_n_u32(63)), vandq_u32(t1108[1], vdupq_n_u32(63))];
        let t1110: [uint32x4_t; 4] = [vshlq_u32(t1106[0], vreinterpretq_s32_u32(t1109[0])), vshlq_u32(t1106[1], vreinterpretq_s32_u32(t1109[1])), vorrq_u32(vshlq_u32(t1106[2], vreinterpretq_s32_u32(t1109[0])), vshlq_u32(t1106[0], vsubq_s32(vreinterpretq_s32_u32(t1109[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1106[3], vreinterpretq_s32_u32(t1109[1])), vshlq_u32(t1106[1], vsubq_s32(vreinterpretq_s32_u32(t1109[1]), vdupq_n_s32(32))))];
        let t1111: [uint32x4_t; 4] = [vorrq_u32(x40[0], t1110[0]), vorrq_u32(x40[1], t1110[1]), vorrq_u32(x40[2], t1110[2]), vorrq_u32(x40[3], t1110[3])];
        t1098 = [vbslq_u32(t1097[0], t1111[0], t1098[0]), vbslq_u32(t1097[1], t1111[1], t1098[1]), vbslq_u32(t1097[0], t1111[2], t1098[2]), vbslq_u32(t1097[1], t1111[3], t1098[3])];
    }
    {
        t1098 = [vbslq_u32(t1099[0], x40[0], t1098[0]), vbslq_u32(t1099[1], x40[1], t1098[1]), vbslq_u32(t1099[0], x40[2], t1098[2]), vbslq_u32(t1099[1], x40[3], t1098[3])];
    }
    res = t1098;
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
        let t1112 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], x8i[0]), vandq_u32(live[1], x8i[1]))));
        let t1113: [uint32x4_t; 2] = [vbslq_u32(live[0], x8i[0], t1112), vbslq_u32(live[1], x8i[1], t1112)];
        let mut t1114: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1113[0]) as usize * 1) as *const u64).read_unaligned(); t1114[0] = vsetq_lane_u32::<0>(v as u32, t1114[0]); t1114[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1114[2]); }
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1113[0]) as usize * 1) as *const u64).read_unaligned(); t1114[0] = vsetq_lane_u32::<1>(v as u32, t1114[0]); t1114[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1114[2]); }
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1113[0]) as usize * 1) as *const u64).read_unaligned(); t1114[0] = vsetq_lane_u32::<2>(v as u32, t1114[0]); t1114[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1114[2]); }
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1113[0]) as usize * 1) as *const u64).read_unaligned(); t1114[0] = vsetq_lane_u32::<3>(v as u32, t1114[0]); t1114[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1114[2]); }
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1113[1]) as usize * 1) as *const u64).read_unaligned(); t1114[1] = vsetq_lane_u32::<0>(v as u32, t1114[1]); t1114[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1114[3]); }
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1113[1]) as usize * 1) as *const u64).read_unaligned(); t1114[1] = vsetq_lane_u32::<1>(v as u32, t1114[1]); t1114[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1114[3]); }
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1113[1]) as usize * 1) as *const u64).read_unaligned(); t1114[1] = vsetq_lane_u32::<2>(v as u32, t1114[1]); t1114[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1114[3]); }
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1113[1]) as usize * 1) as *const u64).read_unaligned(); t1114[1] = vsetq_lane_u32::<3>(v as u32, t1114[1]); t1114[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1114[3]); }
        res = t1114;
        return res;
    }
    let t1115: [uint32x4_t; 2] = [x8i[0], x8i[1]];
    let t1116: u64 = ((x8b) as usize as u64);
    let t1117 = [vdupq_n_u32((t1116) as u32); 2];
    let t1118 = [vdupq_n_u32(((t1116) >> 32) as u32); 2];
    let t1119: [uint32x4_t; 2] = [vaddq_u32(t1117[0], t1115[0]), vaddq_u32(t1117[1], t1115[1])];
    let t1120: [uint32x4_t; 2] = [vcltq_u32(t1119[0], t1117[0]), vcltq_u32(t1119[1], t1117[1])];
    let t1121: [uint32x4_t; 4] = [t1119[0], t1119[1], vsubq_u32(vaddq_u32(t1118[0], vdupq_n_u32(0)), t1120[0]), vsubq_u32(vaddq_u32(t1118[1], vdupq_n_u32(0)), t1120[1])];
    let t1122: u64 = !(7u64);
    let t1123 = [vdupq_n_u32((t1122) as u32); 2];
    let t1124 = [vdupq_n_u32(((t1122) >> 32) as u32); 2];
    let t1125: [uint32x4_t; 4] = [vandq_u32(t1121[0], t1123[0]), vandq_u32(t1121[1], t1123[1]), vandq_u32(t1121[2], t1124[0]), vandq_u32(t1121[3], t1124[1])];
    let t1126: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
    let t1127: [uint32x4_t; 4] = [vandq_u32(t1125[0], t1126[0]), vandq_u32(t1125[1], t1126[1]), vandq_u32(t1125[2], t1126[2]), vandq_u32(t1125[3], t1126[3])];
    let t1128: u64 = ((arg0) as usize as u64);
    let t1129 = [vdupq_n_u32((t1128) as u32); 2];
    let t1130 = [vdupq_n_u32(((t1128) >> 32) as u32); 2];
    let t1131: [uint32x4_t; 2] = [vaddq_u32(t1129[0], t1127[0]), vaddq_u32(t1129[1], t1127[1])];
    let t1132: [uint32x4_t; 2] = [vcltq_u32(t1131[0], t1129[0]), vcltq_u32(t1131[1], t1129[1])];
    let t1133: [uint32x4_t; 4] = [t1131[0], t1131[1], vsubq_u32(vaddq_u32(t1130[0], t1127[2]), t1132[0]), vsubq_u32(vaddq_u32(t1130[1], t1127[3]), t1132[1])];
    x24 = t1133;
    let t1134: [uint32x4_t; 2] = [x8i[0], x8i[1]];
    let t1135: u64 = ((x8b) as usize as u64);
    let t1136 = [vdupq_n_u32((t1135) as u32); 2];
    let t1137 = [vdupq_n_u32(((t1135) >> 32) as u32); 2];
    let t1138: [uint32x4_t; 2] = [vaddq_u32(t1136[0], t1134[0]), vaddq_u32(t1136[1], t1134[1])];
    let t1139: [uint32x4_t; 2] = [vcltq_u32(t1138[0], t1136[0]), vcltq_u32(t1138[1], t1136[1])];
    let t1140: [uint32x4_t; 4] = [t1138[0], t1138[1], vsubq_u32(vaddq_u32(t1137[0], vdupq_n_u32(0)), t1139[0]), vsubq_u32(vaddq_u32(t1137[1], vdupq_n_u32(0)), t1139[1])];
    let t1141: u64 = 7u64;
    let t1142 = [vdupq_n_u32((t1141) as u32); 2];
    let t1143: [uint32x4_t; 2] = [vandq_u32(t1140[0], t1142[0]), vandq_u32(t1140[1], t1142[1])];
    let t1144 = [vdupq_n_u32(8u32); 2];
    let t1145: [uint32x4_t; 2] = [vmulq_u32(t1143[0], t1144[0]), vmulq_u32(t1143[1], t1144[1])];
    x32 = t1145;
    let mut t1146: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x24[2]) as u64) << 32 | vgetq_lane_u32::<0>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1146[0] = vsetq_lane_u32::<0>(v as u32, t1146[0]); t1146[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1146[2]); }
    { let v = (((vgetq_lane_u32::<1>(x24[2]) as u64) << 32 | vgetq_lane_u32::<1>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1146[0] = vsetq_lane_u32::<1>(v as u32, t1146[0]); t1146[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1146[2]); }
    { let v = (((vgetq_lane_u32::<2>(x24[2]) as u64) << 32 | vgetq_lane_u32::<2>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1146[0] = vsetq_lane_u32::<2>(v as u32, t1146[0]); t1146[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1146[2]); }
    { let v = (((vgetq_lane_u32::<3>(x24[2]) as u64) << 32 | vgetq_lane_u32::<3>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1146[0] = vsetq_lane_u32::<3>(v as u32, t1146[0]); t1146[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1146[2]); }
    { let v = (((vgetq_lane_u32::<0>(x24[3]) as u64) << 32 | vgetq_lane_u32::<0>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1146[1] = vsetq_lane_u32::<0>(v as u32, t1146[1]); t1146[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1146[3]); }
    { let v = (((vgetq_lane_u32::<1>(x24[3]) as u64) << 32 | vgetq_lane_u32::<1>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1146[1] = vsetq_lane_u32::<1>(v as u32, t1146[1]); t1146[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1146[3]); }
    { let v = (((vgetq_lane_u32::<2>(x24[3]) as u64) << 32 | vgetq_lane_u32::<2>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1146[1] = vsetq_lane_u32::<2>(v as u32, t1146[1]); t1146[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1146[3]); }
    { let v = (((vgetq_lane_u32::<3>(x24[3]) as u64) << 32 | vgetq_lane_u32::<3>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1146[1] = vsetq_lane_u32::<3>(v as u32, t1146[1]); t1146[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1146[3]); }
    let t1147: [uint32x4_t; 2] = [x32[0], x32[1]];
    let t1148: [uint32x4_t; 2] = [vandq_u32(t1147[0], vdupq_n_u32(63)), vandq_u32(t1147[1], vdupq_n_u32(63))];
    let t1149: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t1146[0], vnegq_s32(vreinterpretq_s32_u32(t1148[0]))), vshlq_u32(t1146[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1148[0])))), vorrq_u32(vshlq_u32(t1146[1], vnegq_s32(vreinterpretq_s32_u32(t1148[1]))), vshlq_u32(t1146[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1148[1])))), vshlq_u32(t1146[2], vnegq_s32(vreinterpretq_s32_u32(t1148[0]))), vshlq_u32(t1146[3], vnegq_s32(vreinterpretq_s32_u32(t1148[1])))];
    x40 = t1149;
    let t1150 = [vdupq_n_u32(0u32); 2];
    let t1151: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t1150[0])), vmvnq_u32(vceqq_u32(x32[1], t1150[1]))];
    let t1152 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t1153: [uint32x4_t; 2] = [vaddq_u32(x32[0], t1152[0]), vaddq_u32(x32[1], t1152[1])];
    let t1154 = [vdupq_n_u32(64u32); 2];
    let t1155: [uint32x4_t; 2] = [vcgtq_u32(t1153[0], t1154[0]), vcgtq_u32(t1153[1], t1154[1])];
    let t1156: [uint32x4_t; 2] = [vandq_u32(t1151[0], t1155[0]), vandq_u32(t1151[1], t1155[1])];
    let mut t1157: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1158: [uint32x4_t; 2] = [vandq_u32(live[0], t1156[0]), vandq_u32(live[1], t1156[1])];
    let t1159: [uint32x4_t; 2] = [vbicq_u32(live[0], t1156[0]), vbicq_u32(live[1], t1156[1])];
    if any(&t1158) {
        let t1160: u64 = (1u64).wrapping_mul(8u64);
        let t1161 = [vdupq_n_u32((t1160) as u32); 2];
        let t1162 = [vdupq_n_u32(((t1160) >> 32) as u32); 2];
        let t1163: [uint32x4_t; 2] = [vaddq_u32(x24[0], t1161[0]), vaddq_u32(x24[1], t1161[1])];
        let t1164: [uint32x4_t; 2] = [vcltq_u32(t1163[0], x24[0]), vcltq_u32(t1163[1], x24[1])];
        let t1165: [uint32x4_t; 4] = [t1163[0], t1163[1], vsubq_u32(vaddq_u32(x24[2], t1162[0]), t1164[0]), vsubq_u32(vaddq_u32(x24[3], t1162[1]), t1164[1])];
        let mut t1166: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1165[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1165[0]) as u64) as usize as *const u64).read_unaligned(); t1166[0] = vsetq_lane_u32::<0>(v as u32, t1166[0]); t1166[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1166[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1165[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1165[0]) as u64) as usize as *const u64).read_unaligned(); t1166[0] = vsetq_lane_u32::<1>(v as u32, t1166[0]); t1166[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1166[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1165[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1165[0]) as u64) as usize as *const u64).read_unaligned(); t1166[0] = vsetq_lane_u32::<2>(v as u32, t1166[0]); t1166[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1166[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1165[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1165[0]) as u64) as usize as *const u64).read_unaligned(); t1166[0] = vsetq_lane_u32::<3>(v as u32, t1166[0]); t1166[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1166[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1165[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1165[1]) as u64) as usize as *const u64).read_unaligned(); t1166[1] = vsetq_lane_u32::<0>(v as u32, t1166[1]); t1166[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1166[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1165[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1165[1]) as u64) as usize as *const u64).read_unaligned(); t1166[1] = vsetq_lane_u32::<1>(v as u32, t1166[1]); t1166[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1166[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1165[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1165[1]) as u64) as usize as *const u64).read_unaligned(); t1166[1] = vsetq_lane_u32::<2>(v as u32, t1166[1]); t1166[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1166[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1165[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1165[1]) as u64) as usize as *const u64).read_unaligned(); t1166[1] = vsetq_lane_u32::<3>(v as u32, t1166[1]); t1166[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1166[3]); }
        let t1167: [uint32x4_t; 2] = [vsubq_u32(t1154[0], x32[0]), vsubq_u32(t1154[1], x32[1])];
        let t1168: [uint32x4_t; 2] = [t1167[0], t1167[1]];
        let t1169: [uint32x4_t; 2] = [vandq_u32(t1168[0], vdupq_n_u32(63)), vandq_u32(t1168[1], vdupq_n_u32(63))];
        let t1170: [uint32x4_t; 4] = [vshlq_u32(t1166[0], vreinterpretq_s32_u32(t1169[0])), vshlq_u32(t1166[1], vreinterpretq_s32_u32(t1169[1])), vorrq_u32(vshlq_u32(t1166[2], vreinterpretq_s32_u32(t1169[0])), vshlq_u32(t1166[0], vsubq_s32(vreinterpretq_s32_u32(t1169[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1166[3], vreinterpretq_s32_u32(t1169[1])), vshlq_u32(t1166[1], vsubq_s32(vreinterpretq_s32_u32(t1169[1]), vdupq_n_s32(32))))];
        let t1171: [uint32x4_t; 4] = [vorrq_u32(x40[0], t1170[0]), vorrq_u32(x40[1], t1170[1]), vorrq_u32(x40[2], t1170[2]), vorrq_u32(x40[3], t1170[3])];
        t1157 = [vbslq_u32(t1158[0], t1171[0], t1157[0]), vbslq_u32(t1158[1], t1171[1], t1157[1]), vbslq_u32(t1158[0], t1171[2], t1157[2]), vbslq_u32(t1158[1], t1171[3], t1157[3])];
    }
    {
        t1157 = [vbslq_u32(t1159[0], x40[0], t1157[0]), vbslq_u32(t1159[1], x40[1], t1157[1]), vbslq_u32(t1159[0], x40[2], t1157[2]), vbslq_u32(t1159[1], x40[3], t1157[3])];
    }
    res = t1157;
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
        let t1172: [uint32x4_t; 4];
        if (x8is) == 8u32 && ((x8if) as u64) + 7 * ((x8is) as u64) <= u32::MAX as u64 && all(&live) {
            let t1173 = ((arg0).wrapping_add((x8b) as usize) as *const u8).add(((x8if) as usize) * 1);
            let t11740 = vld2q_u32((t1173 as *const u32).add(0));
            let t11741 = vld2q_u32((t1173 as *const u32).add(8));
            let t1174: [uint32x4_t; 4] = [t11740.0, t11741.0, t11740.1, t11741.1];
            t1172 = t1174;
        } else {
            let t1175: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8is)), vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8is))];
            let t1176 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t1175[0]), vandq_u32(live[1], t1175[1]))));
            let t1177: [uint32x4_t; 2] = [vbslq_u32(live[0], t1175[0], t1176), vbslq_u32(live[1], t1175[1], t1176)];
            let mut t1178: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1177[0]) as usize * 1) as *const u64).read_unaligned(); t1178[0] = vsetq_lane_u32::<0>(v as u32, t1178[0]); t1178[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1178[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1177[0]) as usize * 1) as *const u64).read_unaligned(); t1178[0] = vsetq_lane_u32::<1>(v as u32, t1178[0]); t1178[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1178[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1177[0]) as usize * 1) as *const u64).read_unaligned(); t1178[0] = vsetq_lane_u32::<2>(v as u32, t1178[0]); t1178[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1178[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1177[0]) as usize * 1) as *const u64).read_unaligned(); t1178[0] = vsetq_lane_u32::<3>(v as u32, t1178[0]); t1178[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1178[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1177[1]) as usize * 1) as *const u64).read_unaligned(); t1178[1] = vsetq_lane_u32::<0>(v as u32, t1178[1]); t1178[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1178[3]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1177[1]) as usize * 1) as *const u64).read_unaligned(); t1178[1] = vsetq_lane_u32::<1>(v as u32, t1178[1]); t1178[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1178[3]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1177[1]) as usize * 1) as *const u64).read_unaligned(); t1178[1] = vsetq_lane_u32::<2>(v as u32, t1178[1]); t1178[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1178[3]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1177[1]) as usize * 1) as *const u64).read_unaligned(); t1178[1] = vsetq_lane_u32::<3>(v as u32, t1178[1]); t1178[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1178[3]); }
            t1172 = t1178;
        }
        res = t1172;
        return res;
    }
    let t1179: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8is)), vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8is))];
    let t1180: [uint32x4_t; 2] = [t1179[0], t1179[1]];
    let t1181: u64 = ((x8b) as usize as u64);
    let t1182 = [vdupq_n_u32((t1181) as u32); 2];
    let t1183 = [vdupq_n_u32(((t1181) >> 32) as u32); 2];
    let t1184: [uint32x4_t; 2] = [vaddq_u32(t1182[0], t1180[0]), vaddq_u32(t1182[1], t1180[1])];
    let t1185: [uint32x4_t; 2] = [vcltq_u32(t1184[0], t1182[0]), vcltq_u32(t1184[1], t1182[1])];
    let t1186: [uint32x4_t; 4] = [t1184[0], t1184[1], vsubq_u32(vaddq_u32(t1183[0], vdupq_n_u32(0)), t1185[0]), vsubq_u32(vaddq_u32(t1183[1], vdupq_n_u32(0)), t1185[1])];
    let t1187: u64 = !(7u64);
    let t1188 = [vdupq_n_u32((t1187) as u32); 2];
    let t1189 = [vdupq_n_u32(((t1187) >> 32) as u32); 2];
    let t1190: [uint32x4_t; 4] = [vandq_u32(t1186[0], t1188[0]), vandq_u32(t1186[1], t1188[1]), vandq_u32(t1186[2], t1189[0]), vandq_u32(t1186[3], t1189[1])];
    let t1191: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
    let t1192: [uint32x4_t; 4] = [vandq_u32(t1190[0], t1191[0]), vandq_u32(t1190[1], t1191[1]), vandq_u32(t1190[2], t1191[2]), vandq_u32(t1190[3], t1191[3])];
    let t1193: u64 = ((arg0) as usize as u64);
    let t1194 = [vdupq_n_u32((t1193) as u32); 2];
    let t1195 = [vdupq_n_u32(((t1193) >> 32) as u32); 2];
    let t1196: [uint32x4_t; 2] = [vaddq_u32(t1194[0], t1192[0]), vaddq_u32(t1194[1], t1192[1])];
    let t1197: [uint32x4_t; 2] = [vcltq_u32(t1196[0], t1194[0]), vcltq_u32(t1196[1], t1194[1])];
    let t1198: [uint32x4_t; 4] = [t1196[0], t1196[1], vsubq_u32(vaddq_u32(t1195[0], t1192[2]), t1197[0]), vsubq_u32(vaddq_u32(t1195[1], t1192[3]), t1197[1])];
    x24 = t1198;
    let t1199: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8is)), vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8is))];
    let t1200: [uint32x4_t; 2] = [t1199[0], t1199[1]];
    let t1201: u64 = ((x8b) as usize as u64);
    let t1202 = [vdupq_n_u32((t1201) as u32); 2];
    let t1203 = [vdupq_n_u32(((t1201) >> 32) as u32); 2];
    let t1204: [uint32x4_t; 2] = [vaddq_u32(t1202[0], t1200[0]), vaddq_u32(t1202[1], t1200[1])];
    let t1205: [uint32x4_t; 2] = [vcltq_u32(t1204[0], t1202[0]), vcltq_u32(t1204[1], t1202[1])];
    let t1206: [uint32x4_t; 4] = [t1204[0], t1204[1], vsubq_u32(vaddq_u32(t1203[0], vdupq_n_u32(0)), t1205[0]), vsubq_u32(vaddq_u32(t1203[1], vdupq_n_u32(0)), t1205[1])];
    let t1207: u64 = 7u64;
    let t1208 = [vdupq_n_u32((t1207) as u32); 2];
    let t1209: [uint32x4_t; 2] = [vandq_u32(t1206[0], t1208[0]), vandq_u32(t1206[1], t1208[1])];
    let t1210 = [vdupq_n_u32(8u32); 2];
    let t1211: [uint32x4_t; 2] = [vmulq_u32(t1209[0], t1210[0]), vmulq_u32(t1209[1], t1210[1])];
    x32 = t1211;
    let mut t1212: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x24[2]) as u64) << 32 | vgetq_lane_u32::<0>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1212[0] = vsetq_lane_u32::<0>(v as u32, t1212[0]); t1212[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1212[2]); }
    { let v = (((vgetq_lane_u32::<1>(x24[2]) as u64) << 32 | vgetq_lane_u32::<1>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1212[0] = vsetq_lane_u32::<1>(v as u32, t1212[0]); t1212[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1212[2]); }
    { let v = (((vgetq_lane_u32::<2>(x24[2]) as u64) << 32 | vgetq_lane_u32::<2>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1212[0] = vsetq_lane_u32::<2>(v as u32, t1212[0]); t1212[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1212[2]); }
    { let v = (((vgetq_lane_u32::<3>(x24[2]) as u64) << 32 | vgetq_lane_u32::<3>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1212[0] = vsetq_lane_u32::<3>(v as u32, t1212[0]); t1212[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1212[2]); }
    { let v = (((vgetq_lane_u32::<0>(x24[3]) as u64) << 32 | vgetq_lane_u32::<0>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1212[1] = vsetq_lane_u32::<0>(v as u32, t1212[1]); t1212[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1212[3]); }
    { let v = (((vgetq_lane_u32::<1>(x24[3]) as u64) << 32 | vgetq_lane_u32::<1>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1212[1] = vsetq_lane_u32::<1>(v as u32, t1212[1]); t1212[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1212[3]); }
    { let v = (((vgetq_lane_u32::<2>(x24[3]) as u64) << 32 | vgetq_lane_u32::<2>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1212[1] = vsetq_lane_u32::<2>(v as u32, t1212[1]); t1212[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1212[3]); }
    { let v = (((vgetq_lane_u32::<3>(x24[3]) as u64) << 32 | vgetq_lane_u32::<3>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1212[1] = vsetq_lane_u32::<3>(v as u32, t1212[1]); t1212[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1212[3]); }
    let t1213: [uint32x4_t; 2] = [x32[0], x32[1]];
    let t1214: [uint32x4_t; 2] = [vandq_u32(t1213[0], vdupq_n_u32(63)), vandq_u32(t1213[1], vdupq_n_u32(63))];
    let t1215: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t1212[0], vnegq_s32(vreinterpretq_s32_u32(t1214[0]))), vshlq_u32(t1212[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1214[0])))), vorrq_u32(vshlq_u32(t1212[1], vnegq_s32(vreinterpretq_s32_u32(t1214[1]))), vshlq_u32(t1212[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1214[1])))), vshlq_u32(t1212[2], vnegq_s32(vreinterpretq_s32_u32(t1214[0]))), vshlq_u32(t1212[3], vnegq_s32(vreinterpretq_s32_u32(t1214[1])))];
    x40 = t1215;
    let t1216 = [vdupq_n_u32(0u32); 2];
    let t1217: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t1216[0])), vmvnq_u32(vceqq_u32(x32[1], t1216[1]))];
    let t1218 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t1219: [uint32x4_t; 2] = [vaddq_u32(x32[0], t1218[0]), vaddq_u32(x32[1], t1218[1])];
    let t1220 = [vdupq_n_u32(64u32); 2];
    let t1221: [uint32x4_t; 2] = [vcgtq_u32(t1219[0], t1220[0]), vcgtq_u32(t1219[1], t1220[1])];
    let t1222: [uint32x4_t; 2] = [vandq_u32(t1217[0], t1221[0]), vandq_u32(t1217[1], t1221[1])];
    let mut t1223: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1224: [uint32x4_t; 2] = [vandq_u32(live[0], t1222[0]), vandq_u32(live[1], t1222[1])];
    let t1225: [uint32x4_t; 2] = [vbicq_u32(live[0], t1222[0]), vbicq_u32(live[1], t1222[1])];
    if any(&t1224) {
        let t1226: u64 = (1u64).wrapping_mul(8u64);
        let t1227 = [vdupq_n_u32((t1226) as u32); 2];
        let t1228 = [vdupq_n_u32(((t1226) >> 32) as u32); 2];
        let t1229: [uint32x4_t; 2] = [vaddq_u32(x24[0], t1227[0]), vaddq_u32(x24[1], t1227[1])];
        let t1230: [uint32x4_t; 2] = [vcltq_u32(t1229[0], x24[0]), vcltq_u32(t1229[1], x24[1])];
        let t1231: [uint32x4_t; 4] = [t1229[0], t1229[1], vsubq_u32(vaddq_u32(x24[2], t1228[0]), t1230[0]), vsubq_u32(vaddq_u32(x24[3], t1228[1]), t1230[1])];
        let mut t1232: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1231[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1231[0]) as u64) as usize as *const u64).read_unaligned(); t1232[0] = vsetq_lane_u32::<0>(v as u32, t1232[0]); t1232[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1232[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1231[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1231[0]) as u64) as usize as *const u64).read_unaligned(); t1232[0] = vsetq_lane_u32::<1>(v as u32, t1232[0]); t1232[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1232[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1231[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1231[0]) as u64) as usize as *const u64).read_unaligned(); t1232[0] = vsetq_lane_u32::<2>(v as u32, t1232[0]); t1232[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1232[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1231[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1231[0]) as u64) as usize as *const u64).read_unaligned(); t1232[0] = vsetq_lane_u32::<3>(v as u32, t1232[0]); t1232[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1232[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1231[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1231[1]) as u64) as usize as *const u64).read_unaligned(); t1232[1] = vsetq_lane_u32::<0>(v as u32, t1232[1]); t1232[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1232[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1231[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1231[1]) as u64) as usize as *const u64).read_unaligned(); t1232[1] = vsetq_lane_u32::<1>(v as u32, t1232[1]); t1232[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1232[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1231[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1231[1]) as u64) as usize as *const u64).read_unaligned(); t1232[1] = vsetq_lane_u32::<2>(v as u32, t1232[1]); t1232[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1232[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1231[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1231[1]) as u64) as usize as *const u64).read_unaligned(); t1232[1] = vsetq_lane_u32::<3>(v as u32, t1232[1]); t1232[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1232[3]); }
        let t1233: [uint32x4_t; 2] = [vsubq_u32(t1220[0], x32[0]), vsubq_u32(t1220[1], x32[1])];
        let t1234: [uint32x4_t; 2] = [t1233[0], t1233[1]];
        let t1235: [uint32x4_t; 2] = [vandq_u32(t1234[0], vdupq_n_u32(63)), vandq_u32(t1234[1], vdupq_n_u32(63))];
        let t1236: [uint32x4_t; 4] = [vshlq_u32(t1232[0], vreinterpretq_s32_u32(t1235[0])), vshlq_u32(t1232[1], vreinterpretq_s32_u32(t1235[1])), vorrq_u32(vshlq_u32(t1232[2], vreinterpretq_s32_u32(t1235[0])), vshlq_u32(t1232[0], vsubq_s32(vreinterpretq_s32_u32(t1235[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1232[3], vreinterpretq_s32_u32(t1235[1])), vshlq_u32(t1232[1], vsubq_s32(vreinterpretq_s32_u32(t1235[1]), vdupq_n_s32(32))))];
        let t1237: [uint32x4_t; 4] = [vorrq_u32(x40[0], t1236[0]), vorrq_u32(x40[1], t1236[1]), vorrq_u32(x40[2], t1236[2]), vorrq_u32(x40[3], t1236[3])];
        t1223 = [vbslq_u32(t1224[0], t1237[0], t1223[0]), vbslq_u32(t1224[1], t1237[1], t1223[1]), vbslq_u32(t1224[0], t1237[2], t1223[2]), vbslq_u32(t1224[1], t1237[3], t1223[3])];
    }
    {
        t1223 = [vbslq_u32(t1225[0], x40[0], t1223[0]), vbslq_u32(t1225[1], x40[1], t1223[1]), vbslq_u32(t1225[0], x40[2], t1223[2]), vbslq_u32(t1225[1], x40[3], t1223[3])];
    }
    res = t1223;
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
        let t1238: [uint32x4_t; 4];
        if (x8is) == 8u32 && ((x8if) as u64) + 7 * ((x8is) as u64) <= u32::MAX as u64 {
            let t1239 = ((arg0).wrapping_add((x8b) as usize) as *const u8).add(((x8if) as usize) * 1);
            let t12400 = vld2q_u32((t1239 as *const u32).add(0));
            let t12401 = vld2q_u32((t1239 as *const u32).add(8));
            let t1240: [uint32x4_t; 4] = [t12400.0, t12401.0, t12400.1, t12401.1];
            t1238 = t1240;
        } else {
            let t1241: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8is)), vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8is))];
            let mut t1242: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1241[0]) as usize * 1) as *const u64).read_unaligned(); t1242[0] = vsetq_lane_u32::<0>(v as u32, t1242[0]); t1242[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1242[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1241[0]) as usize * 1) as *const u64).read_unaligned(); t1242[0] = vsetq_lane_u32::<1>(v as u32, t1242[0]); t1242[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1242[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1241[0]) as usize * 1) as *const u64).read_unaligned(); t1242[0] = vsetq_lane_u32::<2>(v as u32, t1242[0]); t1242[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1242[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1241[0]) as usize * 1) as *const u64).read_unaligned(); t1242[0] = vsetq_lane_u32::<3>(v as u32, t1242[0]); t1242[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1242[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1241[1]) as usize * 1) as *const u64).read_unaligned(); t1242[1] = vsetq_lane_u32::<0>(v as u32, t1242[1]); t1242[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1242[3]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1241[1]) as usize * 1) as *const u64).read_unaligned(); t1242[1] = vsetq_lane_u32::<1>(v as u32, t1242[1]); t1242[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1242[3]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1241[1]) as usize * 1) as *const u64).read_unaligned(); t1242[1] = vsetq_lane_u32::<2>(v as u32, t1242[1]); t1242[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1242[3]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1241[1]) as usize * 1) as *const u64).read_unaligned(); t1242[1] = vsetq_lane_u32::<3>(v as u32, t1242[1]); t1242[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1242[3]); }
            t1238 = t1242;
        }
        res = t1238;
        return res;
    }
    let t1243: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8is)), vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8is))];
    let t1244: [uint32x4_t; 2] = [t1243[0], t1243[1]];
    let t1245: u64 = ((x8b) as usize as u64);
    let t1246 = [vdupq_n_u32((t1245) as u32); 2];
    let t1247 = [vdupq_n_u32(((t1245) >> 32) as u32); 2];
    let t1248: [uint32x4_t; 2] = [vaddq_u32(t1246[0], t1244[0]), vaddq_u32(t1246[1], t1244[1])];
    let t1249: [uint32x4_t; 2] = [vcltq_u32(t1248[0], t1246[0]), vcltq_u32(t1248[1], t1246[1])];
    let t1250: [uint32x4_t; 4] = [t1248[0], t1248[1], vsubq_u32(vaddq_u32(t1247[0], vdupq_n_u32(0)), t1249[0]), vsubq_u32(vaddq_u32(t1247[1], vdupq_n_u32(0)), t1249[1])];
    let t1251: u64 = !(7u64);
    let t1252 = [vdupq_n_u32((t1251) as u32); 2];
    let t1253 = [vdupq_n_u32(((t1251) >> 32) as u32); 2];
    let t1254: [uint32x4_t; 4] = [vandq_u32(t1250[0], t1252[0]), vandq_u32(t1250[1], t1252[1]), vandq_u32(t1250[2], t1253[0]), vandq_u32(t1250[3], t1253[1])];
    let t1255: u64 = ((arg0) as usize as u64);
    let t1256 = [vdupq_n_u32((t1255) as u32); 2];
    let t1257 = [vdupq_n_u32(((t1255) >> 32) as u32); 2];
    let t1258: [uint32x4_t; 2] = [vaddq_u32(t1256[0], t1254[0]), vaddq_u32(t1256[1], t1254[1])];
    let t1259: [uint32x4_t; 2] = [vcltq_u32(t1258[0], t1256[0]), vcltq_u32(t1258[1], t1256[1])];
    let t1260: [uint32x4_t; 4] = [t1258[0], t1258[1], vsubq_u32(vaddq_u32(t1257[0], t1254[2]), t1259[0]), vsubq_u32(vaddq_u32(t1257[1], t1254[3]), t1259[1])];
    x24 = t1260;
    let t1261: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8is)), vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8is))];
    let t1262: [uint32x4_t; 2] = [t1261[0], t1261[1]];
    let t1263: u64 = ((x8b) as usize as u64);
    let t1264 = [vdupq_n_u32((t1263) as u32); 2];
    let t1265 = [vdupq_n_u32(((t1263) >> 32) as u32); 2];
    let t1266: [uint32x4_t; 2] = [vaddq_u32(t1264[0], t1262[0]), vaddq_u32(t1264[1], t1262[1])];
    let t1267: [uint32x4_t; 2] = [vcltq_u32(t1266[0], t1264[0]), vcltq_u32(t1266[1], t1264[1])];
    let t1268: [uint32x4_t; 4] = [t1266[0], t1266[1], vsubq_u32(vaddq_u32(t1265[0], vdupq_n_u32(0)), t1267[0]), vsubq_u32(vaddq_u32(t1265[1], vdupq_n_u32(0)), t1267[1])];
    let t1269: u64 = 7u64;
    let t1270 = [vdupq_n_u32((t1269) as u32); 2];
    let t1271: [uint32x4_t; 2] = [vandq_u32(t1268[0], t1270[0]), vandq_u32(t1268[1], t1270[1])];
    let t1272 = [vdupq_n_u32(8u32); 2];
    let t1273: [uint32x4_t; 2] = [vmulq_u32(t1271[0], t1272[0]), vmulq_u32(t1271[1], t1272[1])];
    x32 = t1273;
    let mut t1274: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x24[2]) as u64) << 32 | vgetq_lane_u32::<0>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1274[0] = vsetq_lane_u32::<0>(v as u32, t1274[0]); t1274[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1274[2]); }
    { let v = (((vgetq_lane_u32::<1>(x24[2]) as u64) << 32 | vgetq_lane_u32::<1>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1274[0] = vsetq_lane_u32::<1>(v as u32, t1274[0]); t1274[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1274[2]); }
    { let v = (((vgetq_lane_u32::<2>(x24[2]) as u64) << 32 | vgetq_lane_u32::<2>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1274[0] = vsetq_lane_u32::<2>(v as u32, t1274[0]); t1274[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1274[2]); }
    { let v = (((vgetq_lane_u32::<3>(x24[2]) as u64) << 32 | vgetq_lane_u32::<3>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1274[0] = vsetq_lane_u32::<3>(v as u32, t1274[0]); t1274[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1274[2]); }
    { let v = (((vgetq_lane_u32::<0>(x24[3]) as u64) << 32 | vgetq_lane_u32::<0>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1274[1] = vsetq_lane_u32::<0>(v as u32, t1274[1]); t1274[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1274[3]); }
    { let v = (((vgetq_lane_u32::<1>(x24[3]) as u64) << 32 | vgetq_lane_u32::<1>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1274[1] = vsetq_lane_u32::<1>(v as u32, t1274[1]); t1274[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1274[3]); }
    { let v = (((vgetq_lane_u32::<2>(x24[3]) as u64) << 32 | vgetq_lane_u32::<2>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1274[1] = vsetq_lane_u32::<2>(v as u32, t1274[1]); t1274[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1274[3]); }
    { let v = (((vgetq_lane_u32::<3>(x24[3]) as u64) << 32 | vgetq_lane_u32::<3>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1274[1] = vsetq_lane_u32::<3>(v as u32, t1274[1]); t1274[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1274[3]); }
    let t1275: [uint32x4_t; 2] = [x32[0], x32[1]];
    let t1276: [uint32x4_t; 2] = [vandq_u32(t1275[0], vdupq_n_u32(63)), vandq_u32(t1275[1], vdupq_n_u32(63))];
    let t1277: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t1274[0], vnegq_s32(vreinterpretq_s32_u32(t1276[0]))), vshlq_u32(t1274[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1276[0])))), vorrq_u32(vshlq_u32(t1274[1], vnegq_s32(vreinterpretq_s32_u32(t1276[1]))), vshlq_u32(t1274[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1276[1])))), vshlq_u32(t1274[2], vnegq_s32(vreinterpretq_s32_u32(t1276[0]))), vshlq_u32(t1274[3], vnegq_s32(vreinterpretq_s32_u32(t1276[1])))];
    x40 = t1277;
    let t1278 = [vdupq_n_u32(0u32); 2];
    let t1279: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t1278[0])), vmvnq_u32(vceqq_u32(x32[1], t1278[1]))];
    let t1280 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t1281: [uint32x4_t; 2] = [vaddq_u32(x32[0], t1280[0]), vaddq_u32(x32[1], t1280[1])];
    let t1282 = [vdupq_n_u32(64u32); 2];
    let t1283: [uint32x4_t; 2] = [vcgtq_u32(t1281[0], t1282[0]), vcgtq_u32(t1281[1], t1282[1])];
    let t1284: [uint32x4_t; 2] = [vandq_u32(t1279[0], t1283[0]), vandq_u32(t1279[1], t1283[1])];
    let mut t1285: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1286: [uint32x4_t; 2] = [vmvnq_u32(t1284[0]), vmvnq_u32(t1284[1])];
    if any(&t1284) {
        let t1287: u64 = (1u64).wrapping_mul(8u64);
        let t1288 = [vdupq_n_u32((t1287) as u32); 2];
        let t1289 = [vdupq_n_u32(((t1287) >> 32) as u32); 2];
        let t1290: [uint32x4_t; 2] = [vaddq_u32(x24[0], t1288[0]), vaddq_u32(x24[1], t1288[1])];
        let t1291: [uint32x4_t; 2] = [vcltq_u32(t1290[0], x24[0]), vcltq_u32(t1290[1], x24[1])];
        let t1292: [uint32x4_t; 4] = [t1290[0], t1290[1], vsubq_u32(vaddq_u32(x24[2], t1289[0]), t1291[0]), vsubq_u32(vaddq_u32(x24[3], t1289[1]), t1291[1])];
        let mut t1293: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1292[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1292[0]) as u64) as usize as *const u64).read_unaligned(); t1293[0] = vsetq_lane_u32::<0>(v as u32, t1293[0]); t1293[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1293[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1292[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1292[0]) as u64) as usize as *const u64).read_unaligned(); t1293[0] = vsetq_lane_u32::<1>(v as u32, t1293[0]); t1293[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1293[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1292[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1292[0]) as u64) as usize as *const u64).read_unaligned(); t1293[0] = vsetq_lane_u32::<2>(v as u32, t1293[0]); t1293[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1293[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1292[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1292[0]) as u64) as usize as *const u64).read_unaligned(); t1293[0] = vsetq_lane_u32::<3>(v as u32, t1293[0]); t1293[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1293[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1292[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1292[1]) as u64) as usize as *const u64).read_unaligned(); t1293[1] = vsetq_lane_u32::<0>(v as u32, t1293[1]); t1293[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1293[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1292[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1292[1]) as u64) as usize as *const u64).read_unaligned(); t1293[1] = vsetq_lane_u32::<1>(v as u32, t1293[1]); t1293[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1293[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1292[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1292[1]) as u64) as usize as *const u64).read_unaligned(); t1293[1] = vsetq_lane_u32::<2>(v as u32, t1293[1]); t1293[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1293[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1292[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1292[1]) as u64) as usize as *const u64).read_unaligned(); t1293[1] = vsetq_lane_u32::<3>(v as u32, t1293[1]); t1293[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1293[3]); }
        let t1294: [uint32x4_t; 2] = [vsubq_u32(t1282[0], x32[0]), vsubq_u32(t1282[1], x32[1])];
        let t1295: [uint32x4_t; 2] = [t1294[0], t1294[1]];
        let t1296: [uint32x4_t; 2] = [vandq_u32(t1295[0], vdupq_n_u32(63)), vandq_u32(t1295[1], vdupq_n_u32(63))];
        let t1297: [uint32x4_t; 4] = [vshlq_u32(t1293[0], vreinterpretq_s32_u32(t1296[0])), vshlq_u32(t1293[1], vreinterpretq_s32_u32(t1296[1])), vorrq_u32(vshlq_u32(t1293[2], vreinterpretq_s32_u32(t1296[0])), vshlq_u32(t1293[0], vsubq_s32(vreinterpretq_s32_u32(t1296[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1293[3], vreinterpretq_s32_u32(t1296[1])), vshlq_u32(t1293[1], vsubq_s32(vreinterpretq_s32_u32(t1296[1]), vdupq_n_s32(32))))];
        let t1298: [uint32x4_t; 4] = [vorrq_u32(x40[0], t1297[0]), vorrq_u32(x40[1], t1297[1]), vorrq_u32(x40[2], t1297[2]), vorrq_u32(x40[3], t1297[3])];
        t1285 = [vbslq_u32(t1284[0], t1298[0], t1285[0]), vbslq_u32(t1284[1], t1298[1], t1285[1]), vbslq_u32(t1284[0], t1298[2], t1285[2]), vbslq_u32(t1284[1], t1298[3], t1285[3])];
    }
    {
        t1285 = [vbslq_u32(t1286[0], x40[0], t1285[0]), vbslq_u32(t1286[1], x40[1], t1285[1]), vbslq_u32(t1286[0], x40[2], t1285[2]), vbslq_u32(t1286[1], x40[3], t1285[3])];
    }
    res = t1285;
    return res;
}

#[inline(always)]
unsafe fn b20_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t1299: u64;
    if ((x0) >= (64u32)) {
        t1299 = !(0u64);
    } else {
        t1299 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t1299;
    return res;
}

#[inline(always)]
unsafe fn b20_s_f(l: &Launch, lid0: u32, x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t1300: u64;
    if ((x0) >= (64u32)) {
        t1300 = !(0u64);
    } else {
        t1300 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t1300;
    return res;
}

#[inline(always)]
unsafe fn b21_r0sss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: u32, x16: u32, x24: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u64 = 0;
    let mut live = k;
    let mut res: u64 = 0;
    let t1301 = b19_r0ss_p(l, lid0, live, arg0, (((x8) as u64)).wrapping_add(((x16) as u64)), x24);
    x32 = t1301;
    let t1302: u64;
    if ((x24) >= (8u32)) {
        t1302 = x32;
    } else {
        let t1303 = b20_s_p(l, lid0, live, (8u32).wrapping_mul(x24));
        t1302 = ((x32) & (t1303));
    }
    res = t1302;
    return res;
}

#[inline(always)]
unsafe fn b21_r0sss_f(l: &Launch, lid0: u32, arg0: *const u8, x8: u32, x16: u32, x24: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u64 = 0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t1304 = b19_r0ss_f(l, lid0, arg0, (((x8) as u64)).wrapping_add(((x16) as u64)), x24);
    x32 = t1304;
    let t1305: u64;
    if ((x24) >= (8u32)) {
        t1305 = x32;
    } else {
        let t1306 = b20_s_f(l, lid0, (8u32).wrapping_mul(x24));
        t1305 = ((x32) & (t1306));
    }
    res = t1305;
    return res;
}

#[inline(always)]
unsafe fn b21_r0sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: u32, x16: [uint32x4_t; 2], x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1307 = b19_r0al1s_p(l, lid0, live, arg0, ((((x8) as u64)) as usize as *const u8), x16, x24);
    x32 = t1307;
    let t1308: [uint32x4_t; 4];
    if ((x24) >= (8u32)) {
        t1308 = x32;
    } else {
        let t1309 = b20_s_p(l, lid0, live, (8u32).wrapping_mul(x24));
        let t1310: u64 = t1309;
        let t1311 = [vdupq_n_u32((t1310) as u32); 2];
        let t1312 = [vdupq_n_u32(((t1310) >> 32) as u32); 2];
        let t1313: [uint32x4_t; 4] = [vandq_u32(x32[0], t1311[0]), vandq_u32(x32[1], t1311[1]), vandq_u32(x32[2], t1312[0]), vandq_u32(x32[3], t1312[1])];
        t1308 = t1313;
    }
    res = t1308;
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
    let mut t1314: bool = !(arg4).is_null();
    if t1314 {
        let mut t1315: bool = ((l.group[0]) >= ((arg4.wrapping_add((((x48) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        if !t1315 {
            t1315 = ((l.group[1]) >= ((arg4.wrapping_add(((((x48).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        let mut t1316: bool = t1315;
        if !t1316 {
            t1316 = ((l.group[2]) >= ((arg4.wrapping_add(((((x48).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        t1314 = t1316;
    }
    if t1314 {
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
            let t1317: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
            let mut live = t1317;
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
            let t1318: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
            let mut live = t1318;
            let gid0 = l.group[0] * l.block[0] + lid0;
            'chunk: {
                b0_r0r1r2sr6gi_p(l, lid0, live, arg0, arg1, arg2, x24, arg6, lid0, phase, st.add((lid0 / 8) as usize * 256));
            }
        }
    }
}
