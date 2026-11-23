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
unsafe fn b0_r0r1r2sr6gi_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, arg2: *const u8, x3: *const u8, arg6: *const u8, x6: u32, phase: u32, st: *mut u8) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x3 as *const u32).read_unaligned();
    let mut x7: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x8: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x9: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x10: u32 = 0;
    let mut live = k;
    if phase > 0 {
        live = (st as *const [uint32x4_t; 2]).read_unaligned();
        x7 = (st.add(64) as *const [uint32x4_t; 2]).read_unaligned();
        x8 = (st.add(128) as *const [uint32x4_t; 2]).read_unaligned();
        x9 = (st.add(192) as *const [uint32x4_t; 4]).read_unaligned();
        x10 = (st.add(256) as *const u32).read_unaligned();
        if !any(&live) { return; }
    }
    if phase == 0 {
        'phase: {
            let t1: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
            let t2: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x6), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x6), vld1q_u32(LANE[1].as_ptr()))];
            let t3: [uint32x4_t; 2] = [vsubq_u32(t1[0], t2[0]), vsubq_u32(t1[1], t2[1])];
            x7 = t3;
            let t4 = [vdupq_n_u32(4294967295u32); 2];
            x8 = t4;
            let t5 = [vdupq_n_u32(a0); 2];
            let t6: [uint32x4_t; 2] = [vcltq_u32(t1[0], t5[0]), vcltq_u32(t1[1], t5[1])];
            let t7: [uint32x4_t; 2] = [vandq_u32(live[0], t6[0]), vandq_u32(live[1], t6[1])];
            if any(&t7) {
                let t8 = b1_r0r1g_p(l, lid0, t7, arg0, arg1);
                x9 = t8;
                let t9: u64 = (1u64).wrapping_shl((32u64) as u32);
                let t10 = [vdupq_n_u32((t9) as u32); 2];
                let t11 = [vdupq_n_u32(((t9) >> 32) as u32); 2];
                let t12: [uint32x4_t; 4] = [vandq_u32(x9[0], t10[0]), vandq_u32(x9[1], t10[1]), vandq_u32(x9[2], t11[0]), vandq_u32(x9[3], t11[1])];
                let t13: u64 = 0u64;
                let t14 = [vdupq_n_u32((t13) as u32); 2];
                let t15: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t12[0], t14[0]), vceqq_u32(t12[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t12[1], t14[1]), vceqq_u32(t12[3], vdupq_n_u32(0)))];
                let t16: [uint32x4_t; 2] = [vmvnq_u32(t15[0]), vmvnq_u32(t15[1])];
                let mut t17: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                let t18: [uint32x4_t; 2] = [vandq_u32(t7[0], t16[0]), vandq_u32(t7[1], t16[1])];
                let t19: [uint32x4_t; 2] = [vbicq_u32(t7[0], t16[0]), vbicq_u32(t7[1], t16[1])];
                {
                    let t20: [uint32x4_t; 2] = [x9[0], x9[1]];
                    let t21 = [vdupq_n_u32(2147483648u32); 2];
                    let t22: [uint32x4_t; 2] = [vorrq_u32(t21[0], t20[0]), vorrq_u32(t21[1], t20[1])];
                    t17 = [vbslq_u32(t18[0], t22[0], t17[0]), vbslq_u32(t18[1], t22[1], t17[1])];
                }
                {
                    let t23: [uint32x4_t; 2] = [x9[0], x9[1]];
                    t17 = [vbslq_u32(t19[0], t23[0], t17[0]), vbslq_u32(t19[1], t23[1], t17[1])];
                }
                x8 = [vbslq_u32(t7[0], t17[0], x8[0]), vbslq_u32(t7[1], t17[1], x8[1])];
            }
            let t24 = (arg6 as *mut u32).add((x6) as usize);
            if all(&live) {
                vst1q_u32(t24.add(0), x8[0]);
                vst1q_u32(t24.add(4), x8[1]);
            } else {
                let mut t25w = 0u32;
                let t25 = core::hint::black_box(&raw mut t25w);
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t24.add(0), t25).write(vgetq_lane_u32::<0>(x8[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t24.add(1), t25).write(vgetq_lane_u32::<1>(x8[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t24.add(2), t25).write(vgetq_lane_u32::<2>(x8[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t24.add(3), t25).write(vgetq_lane_u32::<3>(x8[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t24.add(4), t25).write(vgetq_lane_u32::<0>(x8[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t24.add(5), t25).write(vgetq_lane_u32::<1>(x8[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t24.add(6), t25).write(vgetq_lane_u32::<2>(x8[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t24.add(7), t25).write(vgetq_lane_u32::<3>(x8[1]));
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
                x10 = 0u32;
                let mut t43 = live;
                loop {
                    // Lanes part: each its own rounds.
                    let t46 = [vdupq_n_u32(4294967295u32); 2];
                    let t47: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x8[0], t46[0])), vmvnq_u32(vceqq_u32(x8[1], t46[1]))];
                    let t48: [uint32x4_t; 2] = [vandq_u32(vdupq_n_u32(if ((x10) < (7u32)) { u32::MAX } else { 0 }), t47[0]), vandq_u32(vdupq_n_u32(if ((x10) < (7u32)) { u32::MAX } else { 0 }), t47[1])];
                    let t49 = [vdupq_n_u32(2147483648u32); 2];
                    let t50: [uint32x4_t; 2] = [vandq_u32(x8[0], t49[0]), vandq_u32(x8[1], t49[1])];
                    let t51 = [vdupq_n_u32(0u32); 2];
                    let t52: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t50[0], t51[0])), vmvnq_u32(vceqq_u32(t50[1], t51[1]))];
                    let t53: [uint32x4_t; 2] = [vandq_u32(t48[0], t52[0]), vandq_u32(t48[1], t52[1])];
                    let t54 = [vdupq_n_u32(2147483647u32); 2];
                    let t55: [uint32x4_t; 2] = [vandq_u32(x8[0], t54[0]), vandq_u32(x8[1], t54[1])];
                    let t56: [uint32x4_t; 2] = [vcgeq_u32(t55[0], x7[0]), vcgeq_u32(t55[1], x7[1])];
                    let t57: [uint32x4_t; 2] = [vandq_u32(t53[0], t56[0]), vandq_u32(t53[1], t56[1])];
                    t43 = [vandq_u32(t43[0], t57[0]), vandq_u32(t43[1], t57[1])];
                    if !(any(&t43)) { break; }
                    let t59: [uint32x4_t; 2] = [vandq_u32(x8[0], t54[0]), vandq_u32(x8[1], t54[1])];
                    let t60: [uint32x4_t; 2] = [vsubq_u32(t59[0], x7[0]), vsubq_u32(t59[1], x7[1])];
                    let t61 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(t43[0], t60[0]), vandq_u32(t43[1], t60[1]))));
                    let t62: [uint32x4_t; 2] = [vbslq_u32(t43[0], t60[0], t61), vbslq_u32(t43[1], t60[1], t61)];
                    let t63 = arg6 as *const u32;
                    let mut t64: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                    t64[0] = vld1q_lane_u32::<0>(t63.add(vgetq_lane_u32::<0>(t62[0]) as usize), t64[0]);
                    t64[0] = vld1q_lane_u32::<1>(t63.add(vgetq_lane_u32::<1>(t62[0]) as usize), t64[0]);
                    t64[0] = vld1q_lane_u32::<2>(t63.add(vgetq_lane_u32::<2>(t62[0]) as usize), t64[0]);
                    t64[0] = vld1q_lane_u32::<3>(t63.add(vgetq_lane_u32::<3>(t62[0]) as usize), t64[0]);
                    t64[1] = vld1q_lane_u32::<0>(t63.add(vgetq_lane_u32::<0>(t62[1]) as usize), t64[1]);
                    t64[1] = vld1q_lane_u32::<1>(t63.add(vgetq_lane_u32::<1>(t62[1]) as usize), t64[1]);
                    t64[1] = vld1q_lane_u32::<2>(t63.add(vgetq_lane_u32::<2>(t62[1]) as usize), t64[1]);
                    t64[1] = vld1q_lane_u32::<3>(t63.add(vgetq_lane_u32::<3>(t62[1]) as usize), t64[1]);
                    x8 = [vbslq_u32(t43[0], t64[0], x8[0]), vbslq_u32(t43[1], t64[1], x8[1])];
                    let t65: [uint32x4_t; 2] = [vandq_u32(t43[0], live[0]), vandq_u32(t43[1], live[1])];
                    let t66 = x10;
                    x10 = (x10).wrapping_add(1u32);
                }
                let t67 = [vdupq_n_u32(4294967295u32); 2];
                let t68: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x8[0], t67[0])), vmvnq_u32(vceqq_u32(x8[1], t67[1]))];
                let t69 = [vdupq_n_u32(2147483648u32); 2];
                let t70: [uint32x4_t; 2] = [vandq_u32(x8[0], t69[0]), vandq_u32(x8[1], t69[1])];
                let t71 = [vdupq_n_u32(0u32); 2];
                let t72: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t70[0], t71[0])), vmvnq_u32(vceqq_u32(t70[1], t71[1]))];
                let t73: [uint32x4_t; 2] = [vandq_u32(t68[0], t72[0]), vandq_u32(t68[1], t72[1])];
                let t74: [uint32x4_t; 2] = [vandq_u32(live[0], t73[0]), vandq_u32(live[1], t73[1])];
                if any(&t74) {
                    let t75 = [vdupq_n_u32(2147483647u32); 2];
                    let t76: [uint32x4_t; 2] = [vandq_u32(x8[0], t75[0]), vandq_u32(x8[1], t75[1])];
                    let t77 = b3_r0r1l_p(l, lid0, t74, arg0, arg1, t76);
                    x8 = [vbslq_u32(t74[0], t77[0], x8[0]), vbslq_u32(t74[1], t77[1], x8[1])];
                }
                let t78 = (arg2 as *mut u32).add((gid0) as usize);
                if all(&live) {
                    vst1q_u32(t78.add(0), x8[0]);
                    vst1q_u32(t78.add(4), x8[1]);
                } else {
                    let mut t79w = 0u32;
                    let t79 = core::hint::black_box(&raw mut t79w);
                    core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t78.add(0), t79).write(vgetq_lane_u32::<0>(x8[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t78.add(1), t79).write(vgetq_lane_u32::<1>(x8[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t78.add(2), t79).write(vgetq_lane_u32::<2>(x8[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t78.add(3), t79).write(vgetq_lane_u32::<3>(x8[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t78.add(4), t79).write(vgetq_lane_u32::<0>(x8[1]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t78.add(5), t79).write(vgetq_lane_u32::<1>(x8[1]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t78.add(6), t79).write(vgetq_lane_u32::<2>(x8[1]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t78.add(7), t79).write(vgetq_lane_u32::<3>(x8[1]));
                }
            }
        }
    }
    if phase + 1 < 2 {
        (st as *mut [uint32x4_t; 2]).write_unaligned(live);
        (st.add(64) as *mut [uint32x4_t; 2]).write_unaligned(x7);
        (st.add(128) as *mut [uint32x4_t; 2]).write_unaligned(x8);
        (st.add(192) as *mut [uint32x4_t; 4]).write_unaligned(x9);
        (st.add(256) as *mut u32).write_unaligned(x10);
    }
}

#[inline(always)]
unsafe fn b0_r0r1r2sr6gi_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, arg2: *const u8, x3: *const u8, arg6: *const u8, x6: u32, phase: u32, st: *mut u8) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x3 as *const u32).read_unaligned();
    let mut x7: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x8: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x9: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x10: u32 = 0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    if phase > 0 {
        live = (st as *const [uint32x4_t; 2]).read_unaligned();
        x7 = (st.add(64) as *const [uint32x4_t; 2]).read_unaligned();
        x8 = (st.add(128) as *const [uint32x4_t; 2]).read_unaligned();
        x9 = (st.add(192) as *const [uint32x4_t; 4]).read_unaligned();
        x10 = (st.add(256) as *const u32).read_unaligned();
        if !any(&live) { return; }
    }
    if phase == 0 {
        'phase: {
            let t80: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
            let t81: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x6), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x6), vld1q_u32(LANE[1].as_ptr()))];
            let t82: [uint32x4_t; 2] = [vsubq_u32(t80[0], t81[0]), vsubq_u32(t80[1], t81[1])];
            x7 = t82;
            let t83 = [vdupq_n_u32(4294967295u32); 2];
            x8 = t83;
            if (((gid0) as u64 + 7) <= u32::MAX as u64 && ((gid0) as u64) >= ((a0) as u64)) {
            } else if ((gid0) as u64 + 7) < ((a0) as u64) {
                let t84 = b1_r0r1g_f(l, lid0, arg0, arg1);
                x9 = t84;
                let t85: u64 = (1u64).wrapping_shl((32u64) as u32);
                let t86 = [vdupq_n_u32((t85) as u32); 2];
                let t87 = [vdupq_n_u32(((t85) >> 32) as u32); 2];
                let t88: [uint32x4_t; 4] = [vandq_u32(x9[0], t86[0]), vandq_u32(x9[1], t86[1]), vandq_u32(x9[2], t87[0]), vandq_u32(x9[3], t87[1])];
                let t89: u64 = 0u64;
                let t90 = [vdupq_n_u32((t89) as u32); 2];
                let t91: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t88[0], t90[0]), vceqq_u32(t88[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t88[1], t90[1]), vceqq_u32(t88[3], vdupq_n_u32(0)))];
                let t92: [uint32x4_t; 2] = [vmvnq_u32(t91[0]), vmvnq_u32(t91[1])];
                let mut t93: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                let t94: [uint32x4_t; 2] = [vmvnq_u32(t92[0]), vmvnq_u32(t92[1])];
                {
                    let t95: [uint32x4_t; 2] = [x9[0], x9[1]];
                    let t96 = [vdupq_n_u32(2147483648u32); 2];
                    let t97: [uint32x4_t; 2] = [vorrq_u32(t96[0], t95[0]), vorrq_u32(t96[1], t95[1])];
                    t93 = [vbslq_u32(t92[0], t97[0], t93[0]), vbslq_u32(t92[1], t97[1], t93[1])];
                }
                {
                    let t98: [uint32x4_t; 2] = [x9[0], x9[1]];
                    t93 = [vbslq_u32(t94[0], t98[0], t93[0]), vbslq_u32(t94[1], t98[1], t93[1])];
                }
                x8 = t93;
            } else {
                // Lanes part: the chunk's lanes on both sides of the bound.
                let t99 = [vdupq_n_u32(a0); 2];
                let t100: [uint32x4_t; 2] = [vcltq_u32(t80[0], t99[0]), vcltq_u32(t80[1], t99[1])];
                let t101: [uint32x4_t; 2] = [vandq_u32(live[0], t100[0]), vandq_u32(live[1], t100[1])];
                if any(&t101) {
                    let t102 = b1_r0r1g_p(l, lid0, t101, arg0, arg1);
                    x9 = t102;
                    let t103: u64 = (1u64).wrapping_shl((32u64) as u32);
                    let t104 = [vdupq_n_u32((t103) as u32); 2];
                    let t105 = [vdupq_n_u32(((t103) >> 32) as u32); 2];
                    let t106: [uint32x4_t; 4] = [vandq_u32(x9[0], t104[0]), vandq_u32(x9[1], t104[1]), vandq_u32(x9[2], t105[0]), vandq_u32(x9[3], t105[1])];
                    let t107: u64 = 0u64;
                    let t108 = [vdupq_n_u32((t107) as u32); 2];
                    let t109: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t106[0], t108[0]), vceqq_u32(t106[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t106[1], t108[1]), vceqq_u32(t106[3], vdupq_n_u32(0)))];
                    let t110: [uint32x4_t; 2] = [vmvnq_u32(t109[0]), vmvnq_u32(t109[1])];
                    let mut t111: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                    let t112: [uint32x4_t; 2] = [vandq_u32(t101[0], t110[0]), vandq_u32(t101[1], t110[1])];
                    let t113: [uint32x4_t; 2] = [vbicq_u32(t101[0], t110[0]), vbicq_u32(t101[1], t110[1])];
                    {
                        let t114: [uint32x4_t; 2] = [x9[0], x9[1]];
                        let t115 = [vdupq_n_u32(2147483648u32); 2];
                        let t116: [uint32x4_t; 2] = [vorrq_u32(t115[0], t114[0]), vorrq_u32(t115[1], t114[1])];
                        t111 = [vbslq_u32(t112[0], t116[0], t111[0]), vbslq_u32(t112[1], t116[1], t111[1])];
                    }
                    {
                        let t117: [uint32x4_t; 2] = [x9[0], x9[1]];
                        t111 = [vbslq_u32(t113[0], t117[0], t111[0]), vbslq_u32(t113[1], t117[1], t111[1])];
                    }
                    x8 = [vbslq_u32(t101[0], t111[0], x8[0]), vbslq_u32(t101[1], t111[1], x8[1])];
                }
            }
            let t118 = (arg6 as *mut u32).add((x6) as usize);
            vst1q_u32(t118.add(0), x8[0]);
            vst1q_u32(t118.add(4), x8[1]);
        }
    }
    if phase == 1 {
        'phase: {
            let t119: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
            let t120 = [vdupq_n_u32(a0); 2];
            let t121: [uint32x4_t; 2] = [vcgeq_u32(t119[0], t120[0]), vcgeq_u32(t119[1], t120[1])];
            let t122: [uint32x4_t; 2] = [vandq_u32(live[0], t121[0]), vandq_u32(live[1], t121[1])];
            {
                live = [vbicq_u32(live[0], t122[0]), vbicq_u32(live[1], t122[1])];
            }
            if any(&live) {
                x10 = 0u32;
                let mut t136 = live;
                loop {
                    // Lanes part: each its own rounds.
                    let t139 = [vdupq_n_u32(4294967295u32); 2];
                    let t140: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x8[0], t139[0])), vmvnq_u32(vceqq_u32(x8[1], t139[1]))];
                    let t141: [uint32x4_t; 2] = [vandq_u32(vdupq_n_u32(if ((x10) < (7u32)) { u32::MAX } else { 0 }), t140[0]), vandq_u32(vdupq_n_u32(if ((x10) < (7u32)) { u32::MAX } else { 0 }), t140[1])];
                    let t142 = [vdupq_n_u32(2147483648u32); 2];
                    let t143: [uint32x4_t; 2] = [vandq_u32(x8[0], t142[0]), vandq_u32(x8[1], t142[1])];
                    let t144 = [vdupq_n_u32(0u32); 2];
                    let t145: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t143[0], t144[0])), vmvnq_u32(vceqq_u32(t143[1], t144[1]))];
                    let t146: [uint32x4_t; 2] = [vandq_u32(t141[0], t145[0]), vandq_u32(t141[1], t145[1])];
                    let t147 = [vdupq_n_u32(2147483647u32); 2];
                    let t148: [uint32x4_t; 2] = [vandq_u32(x8[0], t147[0]), vandq_u32(x8[1], t147[1])];
                    let t149: [uint32x4_t; 2] = [vcgeq_u32(t148[0], x7[0]), vcgeq_u32(t148[1], x7[1])];
                    let t150: [uint32x4_t; 2] = [vandq_u32(t146[0], t149[0]), vandq_u32(t146[1], t149[1])];
                    t136 = [vandq_u32(t136[0], t150[0]), vandq_u32(t136[1], t150[1])];
                    if !(any(&t136)) { break; }
                    let t152: [uint32x4_t; 2] = [vandq_u32(x8[0], t147[0]), vandq_u32(x8[1], t147[1])];
                    let t153: [uint32x4_t; 2] = [vsubq_u32(t152[0], x7[0]), vsubq_u32(t152[1], x7[1])];
                    let t154 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(t136[0], t153[0]), vandq_u32(t136[1], t153[1]))));
                    let t155: [uint32x4_t; 2] = [vbslq_u32(t136[0], t153[0], t154), vbslq_u32(t136[1], t153[1], t154)];
                    let t156 = arg6 as *const u32;
                    let mut t157: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                    t157[0] = vld1q_lane_u32::<0>(t156.add(vgetq_lane_u32::<0>(t155[0]) as usize), t157[0]);
                    t157[0] = vld1q_lane_u32::<1>(t156.add(vgetq_lane_u32::<1>(t155[0]) as usize), t157[0]);
                    t157[0] = vld1q_lane_u32::<2>(t156.add(vgetq_lane_u32::<2>(t155[0]) as usize), t157[0]);
                    t157[0] = vld1q_lane_u32::<3>(t156.add(vgetq_lane_u32::<3>(t155[0]) as usize), t157[0]);
                    t157[1] = vld1q_lane_u32::<0>(t156.add(vgetq_lane_u32::<0>(t155[1]) as usize), t157[1]);
                    t157[1] = vld1q_lane_u32::<1>(t156.add(vgetq_lane_u32::<1>(t155[1]) as usize), t157[1]);
                    t157[1] = vld1q_lane_u32::<2>(t156.add(vgetq_lane_u32::<2>(t155[1]) as usize), t157[1]);
                    t157[1] = vld1q_lane_u32::<3>(t156.add(vgetq_lane_u32::<3>(t155[1]) as usize), t157[1]);
                    x8 = [vbslq_u32(t136[0], t157[0], x8[0]), vbslq_u32(t136[1], t157[1], x8[1])];
                    let t158: [uint32x4_t; 2] = [vandq_u32(t136[0], live[0]), vandq_u32(t136[1], live[1])];
                    let t159 = x10;
                    x10 = (x10).wrapping_add(1u32);
                }
                let t160 = [vdupq_n_u32(4294967295u32); 2];
                let t161: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x8[0], t160[0])), vmvnq_u32(vceqq_u32(x8[1], t160[1]))];
                let t162 = [vdupq_n_u32(2147483648u32); 2];
                let t163: [uint32x4_t; 2] = [vandq_u32(x8[0], t162[0]), vandq_u32(x8[1], t162[1])];
                let t164 = [vdupq_n_u32(0u32); 2];
                let t165: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t163[0], t164[0])), vmvnq_u32(vceqq_u32(t163[1], t164[1]))];
                let t166: [uint32x4_t; 2] = [vandq_u32(t161[0], t165[0]), vandq_u32(t161[1], t165[1])];
                let t167: [uint32x4_t; 2] = [vandq_u32(live[0], t166[0]), vandq_u32(live[1], t166[1])];
                if any(&t167) {
                    let t168 = [vdupq_n_u32(2147483647u32); 2];
                    let t169: [uint32x4_t; 2] = [vandq_u32(x8[0], t168[0]), vandq_u32(x8[1], t168[1])];
                    let t170 = b3_r0r1l_p(l, lid0, t167, arg0, arg1, t169);
                    x8 = [vbslq_u32(t167[0], t170[0], x8[0]), vbslq_u32(t167[1], t170[1], x8[1])];
                }
                let t171 = (arg2 as *mut u32).add((gid0) as usize);
                if all(&live) {
                    vst1q_u32(t171.add(0), x8[0]);
                    vst1q_u32(t171.add(4), x8[1]);
                } else {
                    let mut t172w = 0u32;
                    let t172 = core::hint::black_box(&raw mut t172w);
                    core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t171.add(0), t172).write(vgetq_lane_u32::<0>(x8[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t171.add(1), t172).write(vgetq_lane_u32::<1>(x8[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t171.add(2), t172).write(vgetq_lane_u32::<2>(x8[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t171.add(3), t172).write(vgetq_lane_u32::<3>(x8[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t171.add(4), t172).write(vgetq_lane_u32::<0>(x8[1]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t171.add(5), t172).write(vgetq_lane_u32::<1>(x8[1]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t171.add(6), t172).write(vgetq_lane_u32::<2>(x8[1]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t171.add(7), t172).write(vgetq_lane_u32::<3>(x8[1]));
                }
            }
        }
    }
    if phase + 1 < 2 {
        (st as *mut [uint32x4_t; 2]).write_unaligned(live);
        (st.add(64) as *mut [uint32x4_t; 2]).write_unaligned(x7);
        (st.add(128) as *mut [uint32x4_t; 2]).write_unaligned(x8);
        (st.add(192) as *mut [uint32x4_t; 4]).write_unaligned(x9);
        (st.add(256) as *mut u32).write_unaligned(x10);
    }
}

#[inline(always)]
unsafe fn b1_r0r1g_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x3: u32 = 0;
    let mut x4: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x5: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x6: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t173: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
    let t174 = [vdupq_n_u32(0u32); 2];
    let t175: [uint32x4_t; 2] = [vceqq_u32(t173[0], t174[0]), vceqq_u32(t173[1], t174[1])];
    let t176: [uint32x4_t; 2] = [vandq_u32(live[0], t175[0]), vandq_u32(live[1], t175[1])];
    {
        let t177: u64 = 4294967295u64;
        let t178 = [vdupq_n_u32((t177) as u32); 2];
        let t179: [uint32x4_t; 4] = [t178[0], t178[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = [vbslq_u32(t176[0], t179[0], res[0]), vbslq_u32(t176[1], t179[1], res[1]), vbslq_u32(t176[0], t179[2], res[2]), vbslq_u32(t176[1], t179[3], res[3])];
        live = [vbicq_u32(live[0], t176[0]), vbicq_u32(live[1], t176[1])];
    }
    if any(&live) {
        x3 = gid0.wrapping_sub(1u32);
        let t180 = if (x3 & 255u32) <= 248u32 { b4_r0r1u8_p(l, lid0, live, arg0, arg1, x3) } else { b4_r0r1i_p(l, lid0, live, arg0, arg1, x3) };
        x4 = t180;
        let t181: u64 = 1u64;
        let t182 = [vdupq_n_u32((t181) as u32); 2];
        let t183: [uint32x4_t; 2] = [vandq_u32(x4[0], t182[0]), vandq_u32(x4[1], t182[1])];
        let t184: u64 = 0u64;
        let t185 = [vdupq_n_u32((t184) as u32); 2];
        let t186: [uint32x4_t; 2] = [vceqq_u32(t183[0], t185[0]), vceqq_u32(t183[1], t185[1])];
        let t187: [uint32x4_t; 2] = [vandq_u32(live[0], t186[0]), vandq_u32(live[1], t186[1])];
        {
            let t188: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[1].as_ptr()))];
            let t189: [uint32x4_t; 4] = [t188[0], t188[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t187[0], t189[0], res[0]), vbslq_u32(t187[1], t189[1], res[1]), vbslq_u32(t187[0], t189[2], res[2]), vbslq_u32(t187[1], t189[3], res[3])];
            live = [vbicq_u32(live[0], t187[0]), vbicq_u32(live[1], t187[1])];
        }
        if any(&live) {
            let t190: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x4[0]), vshlq_n_u32::<30>(x4[2])), vorrq_u32(vshrq_n_u32::<2>(x4[1]), vshlq_n_u32::<30>(x4[3])), vshrq_n_u32::<2>(x4[2]), vshrq_n_u32::<2>(x4[3])];
            let t191: [uint32x4_t; 2] = [t190[0], t190[1]];
            x5 = t191;
            let t192 = [vdupq_n_u32(1u32); 2];
            let t193: [uint32x4_t; 2] = [vsubq_u32(x5[0], t192[0]), vsubq_u32(x5[1], t192[1])];
            let t194: [uint32x4_t; 2] = [vcgeq_u32(t193[0], t173[0]), vcgeq_u32(t193[1], t173[1])];
            let t195: [uint32x4_t; 2] = [vandq_u32(live[0], t194[0]), vandq_u32(live[1], t194[1])];
            {
                let t196: u64 = 4294967295u64;
                let t197 = [vdupq_n_u32((t196) as u32); 2];
                let t198: [uint32x4_t; 4] = [t197[0], t197[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t195[0], t198[0], res[0]), vbslq_u32(t195[1], t198[1], res[1]), vbslq_u32(t195[0], t198[2], res[2]), vbslq_u32(t195[1], t198[3], res[3])];
                live = [vbicq_u32(live[0], t195[0]), vbicq_u32(live[1], t195[1])];
            }
            if any(&live) {
                let t199: [uint32x4_t; 2] = [vsubq_u32(t173[0], x5[0]), vsubq_u32(t173[1], x5[1])];
                x6 = t199;
                let t200: u64 = 2u64;
                let t201 = [vdupq_n_u32((t200) as u32); 2];
                let t202: [uint32x4_t; 2] = [vandq_u32(x4[0], t201[0]), vandq_u32(x4[1], t201[1])];
                let t203: u64 = 0u64;
                let t204 = [vdupq_n_u32((t203) as u32); 2];
                let t205: [uint32x4_t; 2] = [vceqq_u32(t202[0], t204[0]), vceqq_u32(t202[1], t204[1])];
                let t206: [uint32x4_t; 2] = [vmvnq_u32(t205[0]), vmvnq_u32(t205[1])];
                let t207: [uint32x4_t; 2] = [vandq_u32(live[0], t206[0]), vandq_u32(live[1], t206[1])];
                {
                    let t208: [uint32x4_t; 4] = [x6[0], x6[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                    res = [vbslq_u32(t207[0], t208[0], res[0]), vbslq_u32(t207[1], t208[1], res[1]), vbslq_u32(t207[0], t208[2], res[2]), vbslq_u32(t207[1], t208[3], res[3])];
                    live = [vbicq_u32(live[0], t207[0]), vbicq_u32(live[1], t207[1])];
                }
                if any(&live) {
                    let t209: [uint32x4_t; 2] = [vceqq_u32(x6[0], t174[0]), vceqq_u32(x6[1], t174[1])];
                    let t210: [uint32x4_t; 2] = [vandq_u32(live[0], t209[0]), vandq_u32(live[1], t209[1])];
                    {
                        let t211: u64 = 4294967295u64;
                        let t212 = [vdupq_n_u32((t211) as u32); 2];
                        let t213: [uint32x4_t; 4] = [t212[0], t212[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                        res = [vbslq_u32(t210[0], t213[0], res[0]), vbslq_u32(t210[1], t213[1], res[1]), vbslq_u32(t210[0], t213[2], res[2]), vbslq_u32(t210[1], t213[3], res[3])];
                        live = [vbicq_u32(live[0], t210[0]), vbicq_u32(live[1], t210[1])];
                    }
                    if any(&live) {
                        let t214: u64 = (1u64).wrapping_shl((32u64) as u32);
                        let t215 = [vdupq_n_u32((t214) as u32); 2];
                        let t216 = [vdupq_n_u32(((t214) >> 32) as u32); 2];
                        let t217: [uint32x4_t; 4] = [vorrq_u32(t215[0], x6[0]), vorrq_u32(t215[1], x6[1]), t216[0], t216[1]];
                        res = [vbslq_u32(live[0], t217[0], res[0]), vbslq_u32(live[1], t217[1], res[1]), vbslq_u32(live[0], t217[2], res[2]), vbslq_u32(live[1], t217[3], res[3])];
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
    let mut x3: u32 = 0;
    let mut x4: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x5: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x6: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if (((gid0) as u64 + 7) <= u32::MAX as u64 && (((0u32) as u64) < ((gid0) as u64) || ((0u32) as u64) > ((gid0) as u64 + 7))) {
        x3 = gid0.wrapping_sub(1u32);
        let t218 = if (x3 & 255u32) <= 248u32 { b4_r0r1u8_f(l, lid0, arg0, arg1, x3) } else { b4_r0r1i_f(l, lid0, arg0, arg1, x3) };
        x4 = t218;
        let t219: u64 = 1u64;
        let t220 = [vdupq_n_u32((t219) as u32); 2];
        let t221: [uint32x4_t; 2] = [vandq_u32(x4[0], t220[0]), vandq_u32(x4[1], t220[1])];
        let t222: u64 = 0u64;
        let t223 = [vdupq_n_u32((t222) as u32); 2];
        let t224: [uint32x4_t; 2] = [vceqq_u32(t221[0], t223[0]), vceqq_u32(t221[1], t223[1])];
        {
            let t225: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[1].as_ptr()))];
            let t226: [uint32x4_t; 4] = [t225[0], t225[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t224[0], t226[0], res[0]), vbslq_u32(t224[1], t226[1], res[1]), vbslq_u32(t224[0], t226[2], res[2]), vbslq_u32(t224[1], t226[3], res[3])];
            live = [vbicq_u32(live[0], t224[0]), vbicq_u32(live[1], t224[1])];
        }
        if any(&live) {
            let t227: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x4[0]), vshlq_n_u32::<30>(x4[2])), vorrq_u32(vshrq_n_u32::<2>(x4[1]), vshlq_n_u32::<30>(x4[3])), vshrq_n_u32::<2>(x4[2]), vshrq_n_u32::<2>(x4[3])];
            let t228: [uint32x4_t; 2] = [t227[0], t227[1]];
            x5 = t228;
            let t229 = [vdupq_n_u32(1u32); 2];
            let t230: [uint32x4_t; 2] = [vsubq_u32(x5[0], t229[0]), vsubq_u32(x5[1], t229[1])];
            let t231: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
            let t232: [uint32x4_t; 2] = [vcgeq_u32(t230[0], t231[0]), vcgeq_u32(t230[1], t231[1])];
            let t233: [uint32x4_t; 2] = [vandq_u32(live[0], t232[0]), vandq_u32(live[1], t232[1])];
            {
                let t234: u64 = 4294967295u64;
                let t235 = [vdupq_n_u32((t234) as u32); 2];
                let t236: [uint32x4_t; 4] = [t235[0], t235[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t233[0], t236[0], res[0]), vbslq_u32(t233[1], t236[1], res[1]), vbslq_u32(t233[0], t236[2], res[2]), vbslq_u32(t233[1], t236[3], res[3])];
                live = [vbicq_u32(live[0], t233[0]), vbicq_u32(live[1], t233[1])];
            }
            if any(&live) {
                let t237: [uint32x4_t; 2] = [vsubq_u32(t231[0], x5[0]), vsubq_u32(t231[1], x5[1])];
                x6 = t237;
                let t238: u64 = 2u64;
                let t239 = [vdupq_n_u32((t238) as u32); 2];
                let t240: [uint32x4_t; 2] = [vandq_u32(x4[0], t239[0]), vandq_u32(x4[1], t239[1])];
                let t241: u64 = 0u64;
                let t242 = [vdupq_n_u32((t241) as u32); 2];
                let t243: [uint32x4_t; 2] = [vceqq_u32(t240[0], t242[0]), vceqq_u32(t240[1], t242[1])];
                let t244: [uint32x4_t; 2] = [vmvnq_u32(t243[0]), vmvnq_u32(t243[1])];
                let t245: [uint32x4_t; 2] = [vandq_u32(live[0], t244[0]), vandq_u32(live[1], t244[1])];
                {
                    let t246: [uint32x4_t; 4] = [x6[0], x6[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                    res = [vbslq_u32(t245[0], t246[0], res[0]), vbslq_u32(t245[1], t246[1], res[1]), vbslq_u32(t245[0], t246[2], res[2]), vbslq_u32(t245[1], t246[3], res[3])];
                    live = [vbicq_u32(live[0], t245[0]), vbicq_u32(live[1], t245[1])];
                }
                if any(&live) {
                    let t247 = [vdupq_n_u32(0u32); 2];
                    let t248: [uint32x4_t; 2] = [vceqq_u32(x6[0], t247[0]), vceqq_u32(x6[1], t247[1])];
                    let t249: [uint32x4_t; 2] = [vandq_u32(live[0], t248[0]), vandq_u32(live[1], t248[1])];
                    {
                        let t250: u64 = 4294967295u64;
                        let t251 = [vdupq_n_u32((t250) as u32); 2];
                        let t252: [uint32x4_t; 4] = [t251[0], t251[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                        res = [vbslq_u32(t249[0], t252[0], res[0]), vbslq_u32(t249[1], t252[1], res[1]), vbslq_u32(t249[0], t252[2], res[2]), vbslq_u32(t249[1], t252[3], res[3])];
                        live = [vbicq_u32(live[0], t249[0]), vbicq_u32(live[1], t249[1])];
                    }
                    if any(&live) {
                        let t253: u64 = (1u64).wrapping_shl((32u64) as u32);
                        let t254 = [vdupq_n_u32((t253) as u32); 2];
                        let t255 = [vdupq_n_u32(((t253) >> 32) as u32); 2];
                        let t256: [uint32x4_t; 4] = [vorrq_u32(t254[0], x6[0]), vorrq_u32(t254[1], x6[1]), t255[0], t255[1]];
                        res = [vbslq_u32(live[0], t256[0], res[0]), vbslq_u32(live[1], t256[1], res[1]), vbslq_u32(live[0], t256[2], res[2]), vbslq_u32(live[1], t256[3], res[3])];
                        live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
                    }
                }
            }
        }
    } else if false {
        let t257: u64 = 4294967295u64;
        let t258 = [vdupq_n_u32((t257) as u32); 2];
        let t259: [uint32x4_t; 4] = [t258[0], t258[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t259;
        return res;
    } else {
        // Lanes part: the chunk's lanes on both sides of the bound.
        let t260: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t261 = [vdupq_n_u32(0u32); 2];
        let t262: [uint32x4_t; 2] = [vceqq_u32(t260[0], t261[0]), vceqq_u32(t260[1], t261[1])];
        let t263: [uint32x4_t; 2] = [vandq_u32(live[0], t262[0]), vandq_u32(live[1], t262[1])];
        {
            let t264: u64 = 4294967295u64;
            let t265 = [vdupq_n_u32((t264) as u32); 2];
            let t266: [uint32x4_t; 4] = [t265[0], t265[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t263[0], t266[0], res[0]), vbslq_u32(t263[1], t266[1], res[1]), vbslq_u32(t263[0], t266[2], res[2]), vbslq_u32(t263[1], t266[3], res[3])];
            live = [vbicq_u32(live[0], t263[0]), vbicq_u32(live[1], t263[1])];
        }
        if any(&live) {
            x3 = gid0.wrapping_sub(1u32);
            let t267 = if (x3 & 255u32) <= 248u32 { b4_r0r1u8_p(l, lid0, live, arg0, arg1, x3) } else { b4_r0r1i_p(l, lid0, live, arg0, arg1, x3) };
            x4 = t267;
            let t268: u64 = 1u64;
            let t269 = [vdupq_n_u32((t268) as u32); 2];
            let t270: [uint32x4_t; 2] = [vandq_u32(x4[0], t269[0]), vandq_u32(x4[1], t269[1])];
            let t271: u64 = 0u64;
            let t272 = [vdupq_n_u32((t271) as u32); 2];
            let t273: [uint32x4_t; 2] = [vceqq_u32(t270[0], t272[0]), vceqq_u32(t270[1], t272[1])];
            let t274: [uint32x4_t; 2] = [vandq_u32(live[0], t273[0]), vandq_u32(live[1], t273[1])];
            {
                let t275: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[1].as_ptr()))];
                let t276: [uint32x4_t; 4] = [t275[0], t275[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t274[0], t276[0], res[0]), vbslq_u32(t274[1], t276[1], res[1]), vbslq_u32(t274[0], t276[2], res[2]), vbslq_u32(t274[1], t276[3], res[3])];
                live = [vbicq_u32(live[0], t274[0]), vbicq_u32(live[1], t274[1])];
            }
            if any(&live) {
                let t277: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x4[0]), vshlq_n_u32::<30>(x4[2])), vorrq_u32(vshrq_n_u32::<2>(x4[1]), vshlq_n_u32::<30>(x4[3])), vshrq_n_u32::<2>(x4[2]), vshrq_n_u32::<2>(x4[3])];
                let t278: [uint32x4_t; 2] = [t277[0], t277[1]];
                x5 = t278;
                let t279 = [vdupq_n_u32(1u32); 2];
                let t280: [uint32x4_t; 2] = [vsubq_u32(x5[0], t279[0]), vsubq_u32(x5[1], t279[1])];
                let t281: [uint32x4_t; 2] = [vcgeq_u32(t280[0], t260[0]), vcgeq_u32(t280[1], t260[1])];
                let t282: [uint32x4_t; 2] = [vandq_u32(live[0], t281[0]), vandq_u32(live[1], t281[1])];
                {
                    let t283: u64 = 4294967295u64;
                    let t284 = [vdupq_n_u32((t283) as u32); 2];
                    let t285: [uint32x4_t; 4] = [t284[0], t284[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                    res = [vbslq_u32(t282[0], t285[0], res[0]), vbslq_u32(t282[1], t285[1], res[1]), vbslq_u32(t282[0], t285[2], res[2]), vbslq_u32(t282[1], t285[3], res[3])];
                    live = [vbicq_u32(live[0], t282[0]), vbicq_u32(live[1], t282[1])];
                }
                if any(&live) {
                    let t286: [uint32x4_t; 2] = [vsubq_u32(t260[0], x5[0]), vsubq_u32(t260[1], x5[1])];
                    x6 = t286;
                    let t287: u64 = 2u64;
                    let t288 = [vdupq_n_u32((t287) as u32); 2];
                    let t289: [uint32x4_t; 2] = [vandq_u32(x4[0], t288[0]), vandq_u32(x4[1], t288[1])];
                    let t290: u64 = 0u64;
                    let t291 = [vdupq_n_u32((t290) as u32); 2];
                    let t292: [uint32x4_t; 2] = [vceqq_u32(t289[0], t291[0]), vceqq_u32(t289[1], t291[1])];
                    let t293: [uint32x4_t; 2] = [vmvnq_u32(t292[0]), vmvnq_u32(t292[1])];
                    let t294: [uint32x4_t; 2] = [vandq_u32(live[0], t293[0]), vandq_u32(live[1], t293[1])];
                    {
                        let t295: [uint32x4_t; 4] = [x6[0], x6[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                        res = [vbslq_u32(t294[0], t295[0], res[0]), vbslq_u32(t294[1], t295[1], res[1]), vbslq_u32(t294[0], t295[2], res[2]), vbslq_u32(t294[1], t295[3], res[3])];
                        live = [vbicq_u32(live[0], t294[0]), vbicq_u32(live[1], t294[1])];
                    }
                    if any(&live) {
                        let t296: [uint32x4_t; 2] = [vceqq_u32(x6[0], t261[0]), vceqq_u32(x6[1], t261[1])];
                        let t297: [uint32x4_t; 2] = [vandq_u32(live[0], t296[0]), vandq_u32(live[1], t296[1])];
                        {
                            let t298: u64 = 4294967295u64;
                            let t299 = [vdupq_n_u32((t298) as u32); 2];
                            let t300: [uint32x4_t; 4] = [t299[0], t299[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                            res = [vbslq_u32(t297[0], t300[0], res[0]), vbslq_u32(t297[1], t300[1], res[1]), vbslq_u32(t297[0], t300[2], res[2]), vbslq_u32(t297[1], t300[3], res[3])];
                            live = [vbicq_u32(live[0], t297[0]), vbicq_u32(live[1], t297[1])];
                        }
                        if any(&live) {
                            let t301: u64 = (1u64).wrapping_shl((32u64) as u32);
                            let t302 = [vdupq_n_u32((t301) as u32); 2];
                            let t303 = [vdupq_n_u32(((t301) >> 32) as u32); 2];
                            let t304: [uint32x4_t; 4] = [vorrq_u32(t302[0], x6[0]), vorrq_u32(t302[1], x6[1]), t303[0], t303[1]];
                            res = [vbslq_u32(live[0], t304[0], res[0]), vbslq_u32(live[1], t304[1], res[1]), vbslq_u32(live[0], t304[2], res[2]), vbslq_u32(live[1], t304[3], res[3])];
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
unsafe fn b1_r0r1l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x3: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x4: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x5: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x6: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t305 = [vdupq_n_u32(0u32); 2];
    let t306: [uint32x4_t; 2] = [vceqq_u32(x2[0], t305[0]), vceqq_u32(x2[1], t305[1])];
    let t307: [uint32x4_t; 2] = [vandq_u32(live[0], t306[0]), vandq_u32(live[1], t306[1])];
    {
        let t308: u64 = 4294967295u64;
        let t309 = [vdupq_n_u32((t308) as u32); 2];
        let t310: [uint32x4_t; 4] = [t309[0], t309[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = [vbslq_u32(t307[0], t310[0], res[0]), vbslq_u32(t307[1], t310[1], res[1]), vbslq_u32(t307[0], t310[2], res[2]), vbslq_u32(t307[1], t310[3], res[3])];
        live = [vbicq_u32(live[0], t307[0]), vbicq_u32(live[1], t307[1])];
    }
    if any(&live) {
        let t311 = [vdupq_n_u32(1u32); 2];
        let t312: [uint32x4_t; 2] = [vsubq_u32(x2[0], t311[0]), vsubq_u32(x2[1], t311[1])];
        x3 = t312;
        let t313 = b4_r0r1l_p(l, lid0, live, arg0, arg1, x3);
        x4 = t313;
        let t314: u64 = 1u64;
        let t315 = [vdupq_n_u32((t314) as u32); 2];
        let t316: [uint32x4_t; 2] = [vandq_u32(x4[0], t315[0]), vandq_u32(x4[1], t315[1])];
        let t317: u64 = 0u64;
        let t318 = [vdupq_n_u32((t317) as u32); 2];
        let t319: [uint32x4_t; 2] = [vceqq_u32(t316[0], t318[0]), vceqq_u32(t316[1], t318[1])];
        let t320: [uint32x4_t; 2] = [vandq_u32(live[0], t319[0]), vandq_u32(live[1], t319[1])];
        {
            let t321: [uint32x4_t; 4] = [x3[0], x3[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t320[0], t321[0], res[0]), vbslq_u32(t320[1], t321[1], res[1]), vbslq_u32(t320[0], t321[2], res[2]), vbslq_u32(t320[1], t321[3], res[3])];
            live = [vbicq_u32(live[0], t320[0]), vbicq_u32(live[1], t320[1])];
        }
        if any(&live) {
            let t322: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x4[0]), vshlq_n_u32::<30>(x4[2])), vorrq_u32(vshrq_n_u32::<2>(x4[1]), vshlq_n_u32::<30>(x4[3])), vshrq_n_u32::<2>(x4[2]), vshrq_n_u32::<2>(x4[3])];
            let t323: [uint32x4_t; 2] = [t322[0], t322[1]];
            x5 = t323;
            let t324: [uint32x4_t; 2] = [vsubq_u32(x5[0], t311[0]), vsubq_u32(x5[1], t311[1])];
            let t325: [uint32x4_t; 2] = [vcgeq_u32(t324[0], x2[0]), vcgeq_u32(t324[1], x2[1])];
            let t326: [uint32x4_t; 2] = [vandq_u32(live[0], t325[0]), vandq_u32(live[1], t325[1])];
            {
                let t327: u64 = 4294967295u64;
                let t328 = [vdupq_n_u32((t327) as u32); 2];
                let t329: [uint32x4_t; 4] = [t328[0], t328[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t326[0], t329[0], res[0]), vbslq_u32(t326[1], t329[1], res[1]), vbslq_u32(t326[0], t329[2], res[2]), vbslq_u32(t326[1], t329[3], res[3])];
                live = [vbicq_u32(live[0], t326[0]), vbicq_u32(live[1], t326[1])];
            }
            if any(&live) {
                let t330: [uint32x4_t; 2] = [vsubq_u32(x2[0], x5[0]), vsubq_u32(x2[1], x5[1])];
                x6 = t330;
                let t331: u64 = 2u64;
                let t332 = [vdupq_n_u32((t331) as u32); 2];
                let t333: [uint32x4_t; 2] = [vandq_u32(x4[0], t332[0]), vandq_u32(x4[1], t332[1])];
                let t334: u64 = 0u64;
                let t335 = [vdupq_n_u32((t334) as u32); 2];
                let t336: [uint32x4_t; 2] = [vceqq_u32(t333[0], t335[0]), vceqq_u32(t333[1], t335[1])];
                let t337: [uint32x4_t; 2] = [vmvnq_u32(t336[0]), vmvnq_u32(t336[1])];
                let t338: [uint32x4_t; 2] = [vandq_u32(live[0], t337[0]), vandq_u32(live[1], t337[1])];
                {
                    let t339: [uint32x4_t; 4] = [x6[0], x6[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                    res = [vbslq_u32(t338[0], t339[0], res[0]), vbslq_u32(t338[1], t339[1], res[1]), vbslq_u32(t338[0], t339[2], res[2]), vbslq_u32(t338[1], t339[3], res[3])];
                    live = [vbicq_u32(live[0], t338[0]), vbicq_u32(live[1], t338[1])];
                }
                if any(&live) {
                    let t340: [uint32x4_t; 2] = [vceqq_u32(x6[0], t305[0]), vceqq_u32(x6[1], t305[1])];
                    let t341: [uint32x4_t; 2] = [vandq_u32(live[0], t340[0]), vandq_u32(live[1], t340[1])];
                    {
                        let t342: u64 = 4294967295u64;
                        let t343 = [vdupq_n_u32((t342) as u32); 2];
                        let t344: [uint32x4_t; 4] = [t343[0], t343[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                        res = [vbslq_u32(t341[0], t344[0], res[0]), vbslq_u32(t341[1], t344[1], res[1]), vbslq_u32(t341[0], t344[2], res[2]), vbslq_u32(t341[1], t344[3], res[3])];
                        live = [vbicq_u32(live[0], t341[0]), vbicq_u32(live[1], t341[1])];
                    }
                    if any(&live) {
                        let t345: u64 = (1u64).wrapping_shl((32u64) as u32);
                        let t346 = [vdupq_n_u32((t345) as u32); 2];
                        let t347 = [vdupq_n_u32(((t345) >> 32) as u32); 2];
                        let t348: [uint32x4_t; 4] = [vorrq_u32(t346[0], x6[0]), vorrq_u32(t346[1], x6[1]), t347[0], t347[1]];
                        res = [vbslq_u32(live[0], t348[0], res[0]), vbslq_u32(live[1], t348[1], res[1]), vbslq_u32(live[0], t348[2], res[2]), vbslq_u32(live[1], t348[3], res[3])];
                        live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
                    }
                }
            }
        }
    }
    res
}

#[inline(always)]
unsafe fn b3_r0r1l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x3: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t349 = b1_r0r1l_p(l, lid0, live, arg0, arg1, x2);
    x3 = t349;
    let mut t358 = live;
    loop {
        // Lanes part: each its own rounds.
        let t360: u64 = (1u64).wrapping_shl((32u64) as u32);
        let t361 = [vdupq_n_u32((t360) as u32); 2];
        let t362 = [vdupq_n_u32(((t360) >> 32) as u32); 2];
        let t363: [uint32x4_t; 4] = [vandq_u32(x3[0], t361[0]), vandq_u32(x3[1], t361[1]), vandq_u32(x3[2], t362[0]), vandq_u32(x3[3], t362[1])];
        let t364: u64 = 0u64;
        let t365 = [vdupq_n_u32((t364) as u32); 2];
        let t366: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t363[0], t365[0]), vceqq_u32(t363[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t363[1], t365[1]), vceqq_u32(t363[3], vdupq_n_u32(0)))];
        let t367: [uint32x4_t; 2] = [vmvnq_u32(t366[0]), vmvnq_u32(t366[1])];
        t358 = [vandq_u32(t358[0], t367[0]), vandq_u32(t358[1], t367[1])];
        if !(any(&t358)) { break; }
        let t369: [uint32x4_t; 2] = [x3[0], x3[1]];
        let t370 = b1_r0r1l_p(l, lid0, t358, arg0, arg1, t369);
        x3 = [vbslq_u32(t358[0], t370[0], x3[0]), vbslq_u32(t358[1], t370[1], x3[1]), vbslq_u32(t358[0], t370[2], x3[2]), vbslq_u32(t358[1], t370[3], x3[3])];
    }
    let t371: [uint32x4_t; 2] = [x3[0], x3[1]];
    res = t371;
    return res;
}

#[inline(always)]
unsafe fn b4_r0r1i_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t372 = b5_r0r1si_p(l, lid0, live, arg0, arg1, 0u32, x2);
    res = t372;
    return res;
}

#[inline(always)]
unsafe fn b4_r0r1i_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t373 = b5_r0r1si_f(l, lid0, arg0, arg1, 0u32, x2);
    res = t373;
    return res;
}

#[inline(always)]
unsafe fn b4_r0r1u8_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t374 = b5_r0r1su8_p(l, lid0, live, arg0, arg1, 0u32, x2);
    res = t374;
    return res;
}

#[inline(always)]
unsafe fn b4_r0r1u8_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t375 = b5_r0r1su8_f(l, lid0, arg0, arg1, 0u32, x2);
    res = t375;
    return res;
}

#[inline(always)]
unsafe fn b4_r0r1l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t376 = b5_r0r1sl_p(l, lid0, live, arg0, arg1, 0u32, x2);
    res = t376;
    return res;
}

#[inline(always)]
unsafe fn b5_r0r1si_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u32 = 0;
    let mut x5: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x6: u64 = 0;
    let mut x7: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t377 = b6_r0r1s_p(l, lid0, live, arg0, arg1, x2);
    x4 = t377;
    let t378: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[1].as_ptr()))];
    let t379: [uint32x4_t; 2] = [vshlq_u32(t378[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(t378[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t380 = b7_r0r1sl_p(l, lid0, live, arg0, arg1, (1u32).wrapping_add(x2), t379);
    x5 = t380;
    let t381 = b8_s_p(l, lid0, live, x4);
    x6 = t381;
    let t382 = [vdupq_n_u32(1023u32); 2];
    let t383: [uint32x4_t; 2] = [vandq_u32(t378[0], t382[0]), vandq_u32(t378[1], t382[1])];
    let t384 = [vdupq_n_u32(x4); 2];
    let t385: [uint32x4_t; 2] = [vmulq_u32(t383[0], t384[0]), vmulq_u32(t383[1], t384[1])];
    let t386 = b9_r0r1lls_p(l, lid0, live, arg0, arg1, x5, t385, 8u32);
    let t387: u64 = x6;
    let t388 = [vdupq_n_u32((t387) as u32); 2];
    let t389 = [vdupq_n_u32(((t387) >> 32) as u32); 2];
    let t390: [uint32x4_t; 4] = [vandq_u32(t386[0], t388[0]), vandq_u32(t386[1], t388[1]), vandq_u32(t386[2], t389[0]), vandq_u32(t386[3], t389[1])];
    x7 = t390;
    let t391: u64 = x6;
    let t392 = [vdupq_n_u32((t391) as u32); 2];
    let t393 = [vdupq_n_u32(((t391) >> 32) as u32); 2];
    let t394: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x7[0], t392[0]), vceqq_u32(x7[2], t393[0])), vandq_u32(vceqq_u32(x7[1], t392[1]), vceqq_u32(x7[3], t393[1]))];
    let mut t395: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t396: [uint32x4_t; 2] = [vandq_u32(live[0], t394[0]), vandq_u32(live[1], t394[1])];
    let t397: [uint32x4_t; 2] = [vbicq_u32(live[0], t394[0]), vbicq_u32(live[1], t394[1])];
    if any(&t396) {
        let t398: [uint32x4_t; 4];
        let t399 = bits(&t396);
        if t399.count_ones() <= 2 {
            let mut t400 = [0u32; 16];
            let mut t401 = t399;
            while t401 != 0 {
                let j = t401.trailing_zeros() as usize;
                t401 &= t401 - 1;
                let t402 = b10_r0r1ss_f(l, lid0, arg0, arg1, (x2).wrapping_add(1u32), (x3).wrapping_add(j as u32));
                let t403: u64 = t402;
                t400[j] = t403 as u32;
                t400[8 + j] = (t403 >> 32) as u32;
            }
            t398 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t400);
        } else {
            let t404 = b10_r0r1si_p(l, lid0, t396, arg0, arg1, (x2).wrapping_add(1u32), x3);
            t398 = t404;
        }
        t395 = [vbslq_u32(t396[0], t398[0], t395[0]), vbslq_u32(t396[1], t398[1], t395[1]), vbslq_u32(t396[0], t398[2], t395[2]), vbslq_u32(t396[1], t398[3], t395[3])];
    }
    {
        t395 = [vbslq_u32(t397[0], x7[0], t395[0]), vbslq_u32(t397[1], x7[1], t395[1]), vbslq_u32(t397[0], x7[2], t395[2]), vbslq_u32(t397[1], x7[3], t395[3])];
    }
    res = t395;
    return res;
}

#[inline(always)]
unsafe fn b5_r0r1si_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32, x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u32 = 0;
    let mut x5: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x6: u64 = 0;
    let mut x7: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t405 = b6_r0r1s_f(l, lid0, arg0, arg1, x2);
    x4 = t405;
    let t406: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[1].as_ptr()))];
    let t407: [uint32x4_t; 2] = [vshlq_u32(t406[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(t406[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t408 = b7_r0r1sl_f(l, lid0, arg0, arg1, (1u32).wrapping_add(x2), t407);
    x5 = t408;
    let t409 = b8_s_f(l, lid0, x4);
    x6 = t409;
    let t410 = [vdupq_n_u32(1023u32); 2];
    let t411: [uint32x4_t; 2] = [vandq_u32(t406[0], t410[0]), vandq_u32(t406[1], t410[1])];
    let t412 = [vdupq_n_u32(x4); 2];
    let t413: [uint32x4_t; 2] = [vmulq_u32(t411[0], t412[0]), vmulq_u32(t411[1], t412[1])];
    let t414 = b9_r0r1lls_f(l, lid0, arg0, arg1, x5, t413, 8u32);
    let t415: u64 = x6;
    let t416 = [vdupq_n_u32((t415) as u32); 2];
    let t417 = [vdupq_n_u32(((t415) >> 32) as u32); 2];
    let t418: [uint32x4_t; 4] = [vandq_u32(t414[0], t416[0]), vandq_u32(t414[1], t416[1]), vandq_u32(t414[2], t417[0]), vandq_u32(t414[3], t417[1])];
    x7 = t418;
    let t419: u64 = x6;
    let t420 = [vdupq_n_u32((t419) as u32); 2];
    let t421 = [vdupq_n_u32(((t419) >> 32) as u32); 2];
    let t422: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x7[0], t420[0]), vceqq_u32(x7[2], t421[0])), vandq_u32(vceqq_u32(x7[1], t420[1]), vceqq_u32(x7[3], t421[1]))];
    let mut t423: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t424: [uint32x4_t; 2] = [vmvnq_u32(t422[0]), vmvnq_u32(t422[1])];
    if any(&t422) {
        let t425: [uint32x4_t; 4];
        let t426 = bits(&t422);
        if t426.count_ones() <= 2 {
            let mut t427 = [0u32; 16];
            let mut t428 = t426;
            while t428 != 0 {
                let j = t428.trailing_zeros() as usize;
                t428 &= t428 - 1;
                let t429 = b10_r0r1ss_f(l, lid0, arg0, arg1, (x2).wrapping_add(1u32), (x3).wrapping_add(j as u32));
                let t430: u64 = t429;
                t427[j] = t430 as u32;
                t427[8 + j] = (t430 >> 32) as u32;
            }
            t425 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t427);
        } else {
            let t431 = b10_r0r1si_p(l, lid0, t422, arg0, arg1, (x2).wrapping_add(1u32), x3);
            t425 = t431;
        }
        t423 = [vbslq_u32(t422[0], t425[0], t423[0]), vbslq_u32(t422[1], t425[1], t423[1]), vbslq_u32(t422[0], t425[2], t423[2]), vbslq_u32(t422[1], t425[3], t423[3])];
    }
    {
        t423 = [vbslq_u32(t424[0], x7[0], t423[0]), vbslq_u32(t424[1], x7[1], t423[1]), vbslq_u32(t424[0], x7[2], t423[2]), vbslq_u32(t424[1], x7[3], t423[3])];
    }
    res = t423;
    return res;
}

#[inline(always)]
unsafe fn b5_r0r1su8_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u32 = 0;
    let mut x5: u32 = 0;
    let mut x6: u64 = 0;
    let mut x7: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t432 = b6_r0r1s_p(l, lid0, live, arg0, arg1, x2);
    x4 = t432;
    let t433 = b7_r0r1ss_p(l, lid0, live, arg0, arg1, (1u32).wrapping_add(x2), (x3 >> 10));
    x5 = t433;
    let t434 = b8_s_p(l, lid0, live, x4);
    x6 = t434;
    let t435 = b9_r0r1sns_p(l, lid0, live, arg0, arg1, x5, (x3 & 1023u32).wrapping_mul(x4), x4, 8u32);
    let t436: u64 = x6;
    let t437 = [vdupq_n_u32((t436) as u32); 2];
    let t438 = [vdupq_n_u32(((t436) >> 32) as u32); 2];
    let t439: [uint32x4_t; 4] = [vandq_u32(t435[0], t437[0]), vandq_u32(t435[1], t437[1]), vandq_u32(t435[2], t438[0]), vandq_u32(t435[3], t438[1])];
    x7 = t439;
    let t440: u64 = x6;
    let t441 = [vdupq_n_u32((t440) as u32); 2];
    let t442 = [vdupq_n_u32(((t440) >> 32) as u32); 2];
    let t443: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x7[0], t441[0]), vceqq_u32(x7[2], t442[0])), vandq_u32(vceqq_u32(x7[1], t441[1]), vceqq_u32(x7[3], t442[1]))];
    let mut t444: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t445: [uint32x4_t; 2] = [vandq_u32(live[0], t443[0]), vandq_u32(live[1], t443[1])];
    let t446: [uint32x4_t; 2] = [vbicq_u32(live[0], t443[0]), vbicq_u32(live[1], t443[1])];
    if any(&t445) {
        let t447: [uint32x4_t; 4];
        let t448 = bits(&t445);
        if t448.count_ones() <= 2 {
            let mut t449 = [0u32; 16];
            let mut t450 = t448;
            while t450 != 0 {
                let j = t450.trailing_zeros() as usize;
                t450 &= t450 - 1;
                let t451 = b10_r0r1ss_f(l, lid0, arg0, arg1, (x2).wrapping_add(1u32), (x3).wrapping_add(j as u32));
                let t452: u64 = t451;
                t449[j] = t452 as u32;
                t449[8 + j] = (t452 >> 32) as u32;
            }
            t447 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t449);
        } else {
            let t453 = b10_r0r1su8_p(l, lid0, t445, arg0, arg1, (x2).wrapping_add(1u32), x3);
            t447 = t453;
        }
        t444 = [vbslq_u32(t445[0], t447[0], t444[0]), vbslq_u32(t445[1], t447[1], t444[1]), vbslq_u32(t445[0], t447[2], t444[2]), vbslq_u32(t445[1], t447[3], t444[3])];
    }
    {
        t444 = [vbslq_u32(t446[0], x7[0], t444[0]), vbslq_u32(t446[1], x7[1], t444[1]), vbslq_u32(t446[0], x7[2], t444[2]), vbslq_u32(t446[1], x7[3], t444[3])];
    }
    res = t444;
    return res;
}

#[inline(always)]
unsafe fn b5_r0r1su8_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32, x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u32 = 0;
    let mut x5: u32 = 0;
    let mut x6: u64 = 0;
    let mut x7: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t454 = b6_r0r1s_f(l, lid0, arg0, arg1, x2);
    x4 = t454;
    let t455 = b7_r0r1ss_f(l, lid0, arg0, arg1, (1u32).wrapping_add(x2), (x3 >> 10));
    x5 = t455;
    let t456 = b8_s_f(l, lid0, x4);
    x6 = t456;
    let t457 = b9_r0r1sns_f(l, lid0, arg0, arg1, x5, (x3 & 1023u32).wrapping_mul(x4), x4, 8u32);
    let t458: u64 = x6;
    let t459 = [vdupq_n_u32((t458) as u32); 2];
    let t460 = [vdupq_n_u32(((t458) >> 32) as u32); 2];
    let t461: [uint32x4_t; 4] = [vandq_u32(t457[0], t459[0]), vandq_u32(t457[1], t459[1]), vandq_u32(t457[2], t460[0]), vandq_u32(t457[3], t460[1])];
    x7 = t461;
    let t462: u64 = x6;
    let t463 = [vdupq_n_u32((t462) as u32); 2];
    let t464 = [vdupq_n_u32(((t462) >> 32) as u32); 2];
    let t465: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x7[0], t463[0]), vceqq_u32(x7[2], t464[0])), vandq_u32(vceqq_u32(x7[1], t463[1]), vceqq_u32(x7[3], t464[1]))];
    let mut t466: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t467: [uint32x4_t; 2] = [vmvnq_u32(t465[0]), vmvnq_u32(t465[1])];
    if any(&t465) {
        let t468: [uint32x4_t; 4];
        let t469 = bits(&t465);
        if t469.count_ones() <= 2 {
            let mut t470 = [0u32; 16];
            let mut t471 = t469;
            while t471 != 0 {
                let j = t471.trailing_zeros() as usize;
                t471 &= t471 - 1;
                let t472 = b10_r0r1ss_f(l, lid0, arg0, arg1, (x2).wrapping_add(1u32), (x3).wrapping_add(j as u32));
                let t473: u64 = t472;
                t470[j] = t473 as u32;
                t470[8 + j] = (t473 >> 32) as u32;
            }
            t468 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t470);
        } else {
            let t474 = b10_r0r1su8_p(l, lid0, t465, arg0, arg1, (x2).wrapping_add(1u32), x3);
            t468 = t474;
        }
        t466 = [vbslq_u32(t465[0], t468[0], t466[0]), vbslq_u32(t465[1], t468[1], t466[1]), vbslq_u32(t465[0], t468[2], t466[2]), vbslq_u32(t465[1], t468[3], t466[3])];
    }
    {
        t466 = [vbslq_u32(t467[0], x7[0], t466[0]), vbslq_u32(t467[1], x7[1], t466[1]), vbslq_u32(t467[0], x7[2], t466[2]), vbslq_u32(t467[1], x7[3], t466[3])];
    }
    res = t466;
    return res;
}

#[inline(always)]
unsafe fn b5_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u32 = 0;
    let mut x5: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x6: u64 = 0;
    let mut x7: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t475 = b6_r0r1s_p(l, lid0, live, arg0, arg1, x2);
    x4 = t475;
    let t476: [uint32x4_t; 2] = [vshlq_u32(x3[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x3[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t477 = b7_r0r1sl_p(l, lid0, live, arg0, arg1, (1u32).wrapping_add(x2), t476);
    x5 = t477;
    let t478 = b8_s_p(l, lid0, live, x4);
    x6 = t478;
    let t479 = [vdupq_n_u32(1023u32); 2];
    let t480: [uint32x4_t; 2] = [vandq_u32(x3[0], t479[0]), vandq_u32(x3[1], t479[1])];
    let t481 = [vdupq_n_u32(x4); 2];
    let t482: [uint32x4_t; 2] = [vmulq_u32(t480[0], t481[0]), vmulq_u32(t480[1], t481[1])];
    let t483 = b9_r0r1lls_p(l, lid0, live, arg0, arg1, x5, t482, 8u32);
    let t484: u64 = x6;
    let t485 = [vdupq_n_u32((t484) as u32); 2];
    let t486 = [vdupq_n_u32(((t484) >> 32) as u32); 2];
    let t487: [uint32x4_t; 4] = [vandq_u32(t483[0], t485[0]), vandq_u32(t483[1], t485[1]), vandq_u32(t483[2], t486[0]), vandq_u32(t483[3], t486[1])];
    x7 = t487;
    let t488: u64 = x6;
    let t489 = [vdupq_n_u32((t488) as u32); 2];
    let t490 = [vdupq_n_u32(((t488) >> 32) as u32); 2];
    let t491: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x7[0], t489[0]), vceqq_u32(x7[2], t490[0])), vandq_u32(vceqq_u32(x7[1], t489[1]), vceqq_u32(x7[3], t490[1]))];
    let mut t492: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t493: [uint32x4_t; 2] = [vandq_u32(live[0], t491[0]), vandq_u32(live[1], t491[1])];
    let t494: [uint32x4_t; 2] = [vbicq_u32(live[0], t491[0]), vbicq_u32(live[1], t491[1])];
    if any(&t493) {
        let t495: [uint32x4_t; 4];
        let t496 = bits(&t493);
        if t496.count_ones() <= 2 {
            let t497 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x3);
            let mut t498 = [0u32; 16];
            let mut t499 = t496;
            while t499 != 0 {
                let j = t499.trailing_zeros() as usize;
                t499 &= t499 - 1;
                let t500 = b10_r0r1ss_f(l, lid0, arg0, arg1, (x2).wrapping_add(1u32), t497[j]);
                let t501: u64 = t500;
                t498[j] = t501 as u32;
                t498[8 + j] = (t501 >> 32) as u32;
            }
            t495 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t498);
        } else {
            let t502 = b10_r0r1sl_p(l, lid0, t493, arg0, arg1, (x2).wrapping_add(1u32), x3);
            t495 = t502;
        }
        t492 = [vbslq_u32(t493[0], t495[0], t492[0]), vbslq_u32(t493[1], t495[1], t492[1]), vbslq_u32(t493[0], t495[2], t492[2]), vbslq_u32(t493[1], t495[3], t492[3])];
    }
    {
        t492 = [vbslq_u32(t494[0], x7[0], t492[0]), vbslq_u32(t494[1], x7[1], t492[1]), vbslq_u32(t494[0], x7[2], t492[2]), vbslq_u32(t494[1], x7[3], t492[3])];
    }
    res = t492;
    return res;
}

#[inline(always)]
unsafe fn b6_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t503 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    res = (((t503).wrapping_shr(((4u32).wrapping_mul(x2)) as u32)) & (15u32));
    return res;
}

#[inline(always)]
unsafe fn b6_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t504 = b11_r0r1s_f(l, lid0, arg0, arg1, 16u32);
    res = (((t504).wrapping_shr(((4u32).wrapping_mul(x2)) as u32)) & (15u32));
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u32 = 0;
    let mut x5: u32 = 0;
    let mut live = k;
    let mut res: u32 = 0;
    let t505 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 40u32);
    let t506 = b12_r0r1s_p(l, lid0, live, arg0, arg1, 0u32);
    x4 = ((x3).wrapping_mul(t505)).wrapping_add(t506);
    if ((x2) == (0u32)) {
        res = x4;
        return res;
    }
    let t507 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    let t508 = b13_ss_p(l, lid0, live, (t507).wrapping_sub((x3).wrapping_shl((10u32) as u32)), 1024u32);
    x5 = t508;
    let t509 = b12_r0r1s_p(l, lid0, live, arg0, arg1, x2);
    res = (x4).wrapping_add((x5).wrapping_mul(t509));
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1ss_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32, x3: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u32 = 0;
    let mut x5: u32 = 0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t510 = b11_r0r1s_f(l, lid0, arg0, arg1, 40u32);
    let t511 = b12_r0r1s_f(l, lid0, arg0, arg1, 0u32);
    x4 = ((x3).wrapping_mul(t510)).wrapping_add(t511);
    if ((x2) == (0u32)) {
        res = x4;
        return res;
    }
    let t512 = b11_r0r1s_f(l, lid0, arg0, arg1, 4u32);
    let t513 = b13_ss_f(l, lid0, (t512).wrapping_sub((x3).wrapping_shl((10u32) as u32)), 1024u32);
    x5 = t513;
    let t514 = b12_r0r1s_f(l, lid0, arg0, arg1, x2);
    res = (x4).wrapping_add((x5).wrapping_mul(t514));
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x5: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t515 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 40u32);
    let t516 = [vdupq_n_u32(t515); 2];
    let t517: [uint32x4_t; 2] = [vmulq_u32(x3[0], t516[0]), vmulq_u32(x3[1], t516[1])];
    let t518 = b12_r0r1s_p(l, lid0, live, arg0, arg1, 0u32);
    let t519 = [vdupq_n_u32(t518); 2];
    let t520: [uint32x4_t; 2] = [vaddq_u32(t517[0], t519[0]), vaddq_u32(t517[1], t519[1])];
    x4 = t520;
    if ((x2) == (0u32)) {
        res = x4;
        return res;
    }
    let t521 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    let t522: [uint32x4_t; 2] = [vshlq_u32(x3[0], vdupq_n_s32(((10u32) & 31) as i32)), vshlq_u32(x3[1], vdupq_n_s32(((10u32) & 31) as i32))];
    let t523 = [vdupq_n_u32(t521); 2];
    let t524: [uint32x4_t; 2] = [vsubq_u32(t523[0], t522[0]), vsubq_u32(t523[1], t522[1])];
    let t525 = b13_ls_p(l, lid0, live, t524, 1024u32);
    x5 = t525;
    let t526 = b12_r0r1s_p(l, lid0, live, arg0, arg1, x2);
    let t527 = [vdupq_n_u32(t526); 2];
    let t528: [uint32x4_t; 2] = [vmulq_u32(x5[0], t527[0]), vmulq_u32(x5[1], t527[1])];
    let t529: [uint32x4_t; 2] = [vaddq_u32(x4[0], t528[0]), vaddq_u32(x4[1], t528[1])];
    res = t529;
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1sl_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32, x3: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x5: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t530 = b11_r0r1s_f(l, lid0, arg0, arg1, 40u32);
    let t531 = [vdupq_n_u32(t530); 2];
    let t532: [uint32x4_t; 2] = [vmulq_u32(x3[0], t531[0]), vmulq_u32(x3[1], t531[1])];
    let t533 = b12_r0r1s_f(l, lid0, arg0, arg1, 0u32);
    let t534 = [vdupq_n_u32(t533); 2];
    let t535: [uint32x4_t; 2] = [vaddq_u32(t532[0], t534[0]), vaddq_u32(t532[1], t534[1])];
    x4 = t535;
    if ((x2) == (0u32)) {
        res = x4;
        return res;
    }
    let t536 = b11_r0r1s_f(l, lid0, arg0, arg1, 4u32);
    let t537: [uint32x4_t; 2] = [vshlq_u32(x3[0], vdupq_n_s32(((10u32) & 31) as i32)), vshlq_u32(x3[1], vdupq_n_s32(((10u32) & 31) as i32))];
    let t538 = [vdupq_n_u32(t536); 2];
    let t539: [uint32x4_t; 2] = [vsubq_u32(t538[0], t537[0]), vsubq_u32(t538[1], t537[1])];
    let t540 = b13_ls_f(l, lid0, t539, 1024u32);
    x5 = t540;
    let t541 = b12_r0r1s_f(l, lid0, arg0, arg1, x2);
    let t542 = [vdupq_n_u32(t541); 2];
    let t543: [uint32x4_t; 2] = [vmulq_u32(x5[0], t542[0]), vmulq_u32(x5[1], t542[1])];
    let t544: [uint32x4_t; 2] = [vaddq_u32(x4[0], t543[0]), vaddq_u32(x4[1], t543[1])];
    res = t544;
    return res;
}

#[inline(always)]
unsafe fn b8_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t545: u64;
    if ((x0) >= (8u32)) {
        t545 = !(0u64);
    } else {
        t545 = ((1u64).wrapping_shl(((((x0).wrapping_mul(8u32)) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t545;
    return res;
}

#[inline(always)]
unsafe fn b8_s_f(l: &Launch, lid0: u32, x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t546: u64;
    if ((x0) >= (8u32)) {
        t546 = !(0u64);
    } else {
        t546 = ((1u64).wrapping_shl(((((x0).wrapping_mul(8u32)) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t546;
    return res;
}

#[inline(always)]
unsafe fn b9_r0r1sns_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3f: u32, x3s: u32, x4: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t547 = b14_r0sns_p(l, lid0, live, arg0, x2, x3f, x3s, x4);
    res = t547;
    return res;
}

#[inline(always)]
unsafe fn b9_r0r1sns_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32, x3f: u32, x3s: u32, x4: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t548 = b14_r0sns_f(l, lid0, arg0, x2, x3f, x3s, x4);
    res = t548;
    return res;
}

#[inline(always)]
unsafe fn b9_r0r1lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: [uint32x4_t; 2], x3: [uint32x4_t; 2], x4: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t549 = b14_r0lls_p(l, lid0, live, arg0, x2, x3, x4);
    res = t549;
    return res;
}

#[inline(always)]
unsafe fn b9_r0r1lls_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: [uint32x4_t; 2], x3: [uint32x4_t; 2], x4: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t550 = b14_r0lls_f(l, lid0, arg0, x2, x3, x4);
    res = t550;
    return res;
}

#[inline(always)]
unsafe fn b10_r0r1ss_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32, x3: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u32 = 0;
    let mut x5: u32 = 0;
    let mut x6: u32 = 0;
    let mut x7: u32 = 0;
    let mut x8: u32 = 0;
    let mut x9: u32 = 0;
    let mut x10: u32 = 0;
    let mut x11: u32 = 0;
    let mut x12: u32 = 0;
    let mut x13: u32 = 0;
    let mut x14: u32 = 0;
    let mut x15: u32 = 0;
    let mut x16: u64 = 0;
    let mut x17: u64 = 0;
    let mut x18: u32 = 0;
    let mut x19: u32 = 0;
    let mut x20: u64 = 0;
    let mut x21: u64 = 0;
    let mut x22: u64 = 0;
    let mut x23: u32 = 0;
    let mut x24: u32 = 0;
    let mut x25: u32 = 0;
    let mut x26: u32 = 0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t551 = b11_r0r1s_f(l, lid0, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x2)));
    x4 = t551;
    let t552 = b11_r0r1s_f(l, lid0, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x2)));
    x5 = t552;
    x6 = (x3).wrapping_shr((10u32) as u32);
    let t553 = b15_r0r1sss_f(l, lid0, arg0, arg1, x5, (x6).wrapping_mul(4u32), 4u32);
    x7 = ((t553) as u32);
    let t554 = b15_r0r1sss_f(l, lid0, arg0, arg1, x5, ((x6).wrapping_mul(4u32)).wrapping_add(4u32), 4u32);
    x8 = ((t554) as u32);
    let t555 = b11_r0r1s_f(l, lid0, arg0, arg1, 4u32);
    x9 = ((t555).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
    let t556 = b16_ss_f(l, lid0, 4u32, (x9).wrapping_sub((x6).wrapping_mul(4u32)));
    x10 = t556;
    x11 = (((x3).wrapping_shr((8u32) as u32)) & (3u32));
    let t557: u32;
    if ((x11) == (0u32)) {
        t557 = 0u32;
    } else {
        let t558 = b17_ss_f(l, lid0, (x7).wrapping_add((2u32).wrapping_mul((x11).wrapping_sub(1u32))), x8);
        let t559 = b15_r0r1sss_f(l, lid0, arg0, arg1, x4, t558, 2u32);
        t557 = ((t559) as u32);
    }
    x12 = t557;
    let t560 = b17_ss_f(l, lid0, (x7).wrapping_add((2u32).wrapping_mul(x11)), x8);
    let t561 = b15_r0r1sss_f(l, lid0, arg0, arg1, x4, t560, 2u32);
    x13 = ((t561) as u32);
    x14 = (x7).wrapping_add((2u32).wrapping_mul(x10));
    x15 = ((x3) & (255u32));
    x16 = 9259542123273814144u64;
    x17 = (((x15) as u64)).wrapping_mul(72340172838076673u64);
    x18 = x12;
    x19 = x12;
    loop {
        // Rounds of one condition: every lane goes on.
        if !(((x19) < (x13))) { break; }
        let t562 = b17_ss_f(l, lid0, (x14).wrapping_add(x19), x8);
        let t563 = b15_r0r1sss_f(l, lid0, arg0, arg1, x4, t562, 8u32);
        x20 = t563;
        x21 = (((x20) | (x16))).wrapping_sub(((x17) & (!(x16))));
        x22 = ((x16) & (((((!(x20)) & (x17))) | (((!(((x20) ^ (x17)))) & (!(x21)))))));
        x23 = (x13).wrapping_sub(x19);
        let t564: u64;
        if ((x23) >= (8u32)) {
            t564 = x22;
        } else {
            t564 = ((x22) & (((1u64).wrapping_shl(((((8u32).wrapping_mul(x23)) as u64)) as u32)).wrapping_sub(1u64)));
        }
        let t565 = b18_s_f(l, lid0, t564);
        x18 = (x18).wrapping_add(t565);
        x19 = (x19).wrapping_add(8u32);
    }
    x12 = x18;
    let t566 = b11_r0r1s_f(l, lid0, arg0, arg1, 16u32);
    x24 = (((t566).wrapping_shr((((((4u32).wrapping_mul(x2)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32));
    let t567 = b17_ss_f(l, lid0, (x7).wrapping_add((2u32).wrapping_mul((x10).wrapping_sub(1u32))), x8);
    let t568 = b15_r0r1sss_f(l, lid0, arg0, arg1, x4, t567, 2u32);
    x25 = ((t568) as u32);
    let t569 = b17_ss_f(l, lid0, ((x14).wrapping_add(x25)).wrapping_add((x12).wrapping_mul(x24)), x8);
    x26 = t569;
    let t570 = b15_r0r1sss_f(l, lid0, arg0, arg1, x4, x26, 8u32);
    let t571 = b8_s_f(l, lid0, x24);
    res = ((t570) & (t571));
    return res;
}

#[inline(always)]
unsafe fn b10_r0r1si_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u32 = 0;
    let mut x5: u32 = 0;
    let mut x6: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x7: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x8: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x9: u32 = 0;
    let mut x10: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x11: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x12: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x13: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x14: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x15: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x16: u64 = 0;
    let mut x17: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x18: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x19: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x20: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x21: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x22: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x23: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x24: u32 = 0;
    let mut x25: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x26: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t572 = b11_r0r1s_p(l, lid0, live, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x2)));
    x4 = t572;
    let t573 = b11_r0r1s_p(l, lid0, live, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x2)));
    x5 = t573;
    let t574: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[1].as_ptr()))];
    let t575: [uint32x4_t; 2] = [vshlq_u32(t574[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(t574[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    x6 = t575;
    let t576 = [vdupq_n_u32(4u32); 2];
    let t577: [uint32x4_t; 2] = [vmulq_u32(x6[0], t576[0]), vmulq_u32(x6[1], t576[1])];
    let t578 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x5, t577, 4u32);
    let t579: [uint32x4_t; 2] = [t578[0], t578[1]];
    x7 = t579;
    let t580: [uint32x4_t; 2] = [vmulq_u32(x6[0], t576[0]), vmulq_u32(x6[1], t576[1])];
    let t581: [uint32x4_t; 2] = [vaddq_u32(t580[0], t576[0]), vaddq_u32(t580[1], t576[1])];
    let t582 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x5, t581, 4u32);
    let t583: [uint32x4_t; 2] = [t582[0], t582[1]];
    x8 = t583;
    let t584 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    x9 = ((t584).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
    let t585: [uint32x4_t; 2] = [vmulq_u32(x6[0], t576[0]), vmulq_u32(x6[1], t576[1])];
    let t586 = [vdupq_n_u32(x9); 2];
    let t587: [uint32x4_t; 2] = [vsubq_u32(t586[0], t585[0]), vsubq_u32(t586[1], t585[1])];
    let t588 = b16_sl_p(l, lid0, live, 4u32, t587);
    x10 = t588;
    let t589: [uint32x4_t; 2] = [vshlq_u32(t574[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(t574[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
    let t590 = [vdupq_n_u32(3u32); 2];
    let t591: [uint32x4_t; 2] = [vandq_u32(t589[0], t590[0]), vandq_u32(t589[1], t590[1])];
    x11 = t591;
    let t592 = [vdupq_n_u32(0u32); 2];
    let t593: [uint32x4_t; 2] = [vceqq_u32(x11[0], t592[0]), vceqq_u32(x11[1], t592[1])];
    let mut t594: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t595: [uint32x4_t; 2] = [vandq_u32(live[0], t593[0]), vandq_u32(live[1], t593[1])];
    let t596: [uint32x4_t; 2] = [vbicq_u32(live[0], t593[0]), vbicq_u32(live[1], t593[1])];
    {
        t594 = [vbslq_u32(t595[0], t592[0], t594[0]), vbslq_u32(t595[1], t592[1], t594[1])];
    }
    if any(&t596) {
        let t597 = [vdupq_n_u32(1u32); 2];
        let t598: [uint32x4_t; 2] = [vsubq_u32(x11[0], t597[0]), vsubq_u32(x11[1], t597[1])];
        let t599 = [vdupq_n_u32(2u32); 2];
        let t600: [uint32x4_t; 2] = [vmulq_u32(t599[0], t598[0]), vmulq_u32(t599[1], t598[1])];
        let t601: [uint32x4_t; 2] = [vaddq_u32(x7[0], t600[0]), vaddq_u32(x7[1], t600[1])];
        let t602 = b17_ll_p(l, lid0, t596, t601, x8);
        let t603 = b15_r0r1sls_p(l, lid0, t596, arg0, arg1, x4, t602, 2u32);
        let t604: [uint32x4_t; 2] = [t603[0], t603[1]];
        t594 = [vbslq_u32(t596[0], t604[0], t594[0]), vbslq_u32(t596[1], t604[1], t594[1])];
    }
    x12 = t594;
    let t605 = [vdupq_n_u32(2u32); 2];
    let t606: [uint32x4_t; 2] = [vmulq_u32(t605[0], x11[0]), vmulq_u32(t605[1], x11[1])];
    let t607: [uint32x4_t; 2] = [vaddq_u32(x7[0], t606[0]), vaddq_u32(x7[1], t606[1])];
    let t608 = b17_ll_p(l, lid0, live, t607, x8);
    let t609 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x4, t608, 2u32);
    let t610: [uint32x4_t; 2] = [t609[0], t609[1]];
    x13 = t610;
    let t611: [uint32x4_t; 2] = [vmulq_u32(t605[0], x10[0]), vmulq_u32(t605[1], x10[1])];
    let t612: [uint32x4_t; 2] = [vaddq_u32(x7[0], t611[0]), vaddq_u32(x7[1], t611[1])];
    x14 = t612;
    let t613 = [vdupq_n_u32(255u32); 2];
    let t614: [uint32x4_t; 2] = [vandq_u32(t574[0], t613[0]), vandq_u32(t574[1], t613[1])];
    x15 = t614;
    x16 = 9259542123273814144u64;
    let t615: u64 = 72340172838076673u64;
    let t616 = [vdupq_n_u32((t615) as u32); 2];
    let t617 = [vdupq_n_u32(((t615) >> 32) as u32); 2];
    let t618: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x15[0]), vget_low_u32(t616[0]))), vreinterpretq_u32_u64(vmull_high_u32(x15[0], t616[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x15[1]), vget_low_u32(t616[1]))), vreinterpretq_u32_u64(vmull_high_u32(x15[1], t616[1])))];
    let t619: [uint32x4_t; 4] = [vmulq_u32(x15[0], t616[0]), vmulq_u32(x15[1], t616[1]), vmlaq_u32(t618[0], x15[0], t617[0]), vmlaq_u32(t618[1], x15[1], t617[1])];
    x17 = t619;
    x18 = x12;
    x19 = x12;
    let mut t621 = live;
    loop {
        // Lanes part: each its own rounds.
        let t623: [uint32x4_t; 2] = [vcltq_u32(x19[0], x13[0]), vcltq_u32(x19[1], x13[1])];
        t621 = [vandq_u32(t621[0], t623[0]), vandq_u32(t621[1], t623[1])];
        if !(any(&t621)) { break; }
        let t625: [uint32x4_t; 2] = [vaddq_u32(x14[0], x19[0]), vaddq_u32(x14[1], x19[1])];
        let t626 = b17_ll_p(l, lid0, t621, t625, x8);
        let t627 = b15_r0r1sls_p(l, lid0, t621, arg0, arg1, x4, t626, 8u32);
        x20 = t627;
        let t628: u64 = x16;
        let t629 = [vdupq_n_u32((t628) as u32); 2];
        let t630 = [vdupq_n_u32(((t628) >> 32) as u32); 2];
        let t631: [uint32x4_t; 4] = [vorrq_u32(x20[0], t629[0]), vorrq_u32(x20[1], t629[1]), vorrq_u32(x20[2], t630[0]), vorrq_u32(x20[3], t630[1])];
        let t632: u64 = !(x16);
        let t633 = [vdupq_n_u32((t632) as u32); 2];
        let t634 = [vdupq_n_u32(((t632) >> 32) as u32); 2];
        let t635: [uint32x4_t; 4] = [vandq_u32(x17[0], t633[0]), vandq_u32(x17[1], t633[1]), vandq_u32(x17[2], t634[0]), vandq_u32(x17[3], t634[1])];
        let t636: [uint32x4_t; 2] = [vcltq_u32(t631[0], t635[0]), vcltq_u32(t631[1], t635[1])];
        let t637: [uint32x4_t; 4] = [vsubq_u32(t631[0], t635[0]), vsubq_u32(t631[1], t635[1]), vaddq_u32(vsubq_u32(t631[2], t635[2]), t636[0]), vaddq_u32(vsubq_u32(t631[3], t635[3]), t636[1])];
        x21 = t637;
        let t638: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x20[r]));
        let t639: [uint32x4_t; 4] = [vandq_u32(t638[0], x17[0]), vandq_u32(t638[1], x17[1]), vandq_u32(t638[2], x17[2]), vandq_u32(t638[3], x17[3])];
        let t640: [uint32x4_t; 4] = [veorq_u32(x20[0], x17[0]), veorq_u32(x20[1], x17[1]), veorq_u32(x20[2], x17[2]), veorq_u32(x20[3], x17[3])];
        let t641: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(t640[r]));
        let t642: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x21[r]));
        let t643: [uint32x4_t; 4] = [vandq_u32(t641[0], t642[0]), vandq_u32(t641[1], t642[1]), vandq_u32(t641[2], t642[2]), vandq_u32(t641[3], t642[3])];
        let t644: [uint32x4_t; 4] = [vorrq_u32(t639[0], t643[0]), vorrq_u32(t639[1], t643[1]), vorrq_u32(t639[2], t643[2]), vorrq_u32(t639[3], t643[3])];
        let t645: u64 = x16;
        let t646 = [vdupq_n_u32((t645) as u32); 2];
        let t647 = [vdupq_n_u32(((t645) >> 32) as u32); 2];
        let t648: [uint32x4_t; 4] = [vandq_u32(t646[0], t644[0]), vandq_u32(t646[1], t644[1]), vandq_u32(t647[0], t644[2]), vandq_u32(t647[1], t644[3])];
        x22 = t648;
        let t649: [uint32x4_t; 2] = [vsubq_u32(x13[0], x19[0]), vsubq_u32(x13[1], x19[1])];
        x23 = t649;
        let t650 = [vdupq_n_u32(8u32); 2];
        let t651: [uint32x4_t; 2] = [vcgeq_u32(x23[0], t650[0]), vcgeq_u32(x23[1], t650[1])];
        let mut t652: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t653: [uint32x4_t; 2] = [vandq_u32(t621[0], t651[0]), vandq_u32(t621[1], t651[1])];
        let t654: [uint32x4_t; 2] = [vbicq_u32(t621[0], t651[0]), vbicq_u32(t621[1], t651[1])];
        {
            t652 = [vbslq_u32(t653[0], x22[0], t652[0]), vbslq_u32(t653[1], x22[1], t652[1]), vbslq_u32(t653[0], x22[2], t652[2]), vbslq_u32(t653[1], x22[3], t652[3])];
        }
        {
            let t655: [uint32x4_t; 2] = [vmulq_u32(t650[0], x23[0]), vmulq_u32(t650[1], x23[1])];
            let t656: u64 = 1u64;
            let t657 = [vdupq_n_u32((t656) as u32); 2];
            let t658: [uint32x4_t; 2] = [t655[0], t655[1]];
            let t659: [uint32x4_t; 2] = [vandq_u32(t658[0], vdupq_n_u32(63)), vandq_u32(t658[1], vdupq_n_u32(63))];
            let t660: [uint32x4_t; 4] = [vshlq_u32(t657[0], vreinterpretq_s32_u32(t659[0])), vshlq_u32(t657[1], vreinterpretq_s32_u32(t659[1])), vshlq_u32(t657[0], vsubq_s32(vreinterpretq_s32_u32(t659[0]), vdupq_n_s32(32))), vshlq_u32(t657[1], vsubq_s32(vreinterpretq_s32_u32(t659[1]), vdupq_n_s32(32)))];
            let t661: u64 = 1u64;
            let t662 = [vdupq_n_u32((t661) as u32); 2];
            let t663: [uint32x4_t; 2] = [vcltq_u32(t660[0], t662[0]), vcltq_u32(t660[1], t662[1])];
            let t664: [uint32x4_t; 4] = [vsubq_u32(t660[0], t662[0]), vsubq_u32(t660[1], t662[1]), vaddq_u32(vsubq_u32(t660[2], vdupq_n_u32(0)), t663[0]), vaddq_u32(vsubq_u32(t660[3], vdupq_n_u32(0)), t663[1])];
            let t665: [uint32x4_t; 4] = [vandq_u32(x22[0], t664[0]), vandq_u32(x22[1], t664[1]), vandq_u32(x22[2], t664[2]), vandq_u32(x22[3], t664[3])];
            t652 = [vbslq_u32(t654[0], t665[0], t652[0]), vbslq_u32(t654[1], t665[1], t652[1]), vbslq_u32(t654[0], t665[2], t652[2]), vbslq_u32(t654[1], t665[3], t652[3])];
        }
        let t666 = b18_l_p(l, lid0, t621, t652);
        let t667: [uint32x4_t; 2] = [vaddq_u32(x18[0], t666[0]), vaddq_u32(x18[1], t666[1])];
        x18 = [vbslq_u32(t621[0], t667[0], x18[0]), vbslq_u32(t621[1], t667[1], x18[1])];
        let t668: [uint32x4_t; 2] = [vandq_u32(t621[0], live[0]), vandq_u32(t621[1], live[1])];
        let t669: [uint32x4_t; 2] = [vaddq_u32(x19[0], t650[0]), vaddq_u32(x19[1], t650[1])];
        x19 = t669;
    }
    x12 = x18;
    let t670 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    x24 = (((t670).wrapping_shr((((((4u32).wrapping_mul(x2)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32));
    let t671 = [vdupq_n_u32(1u32); 2];
    let t672: [uint32x4_t; 2] = [vsubq_u32(x10[0], t671[0]), vsubq_u32(x10[1], t671[1])];
    let t673: [uint32x4_t; 2] = [vmulq_u32(t605[0], t672[0]), vmulq_u32(t605[1], t672[1])];
    let t674: [uint32x4_t; 2] = [vaddq_u32(x7[0], t673[0]), vaddq_u32(x7[1], t673[1])];
    let t675 = b17_ll_p(l, lid0, live, t674, x8);
    let t676 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x4, t675, 2u32);
    let t677: [uint32x4_t; 2] = [t676[0], t676[1]];
    x25 = t677;
    let t678: [uint32x4_t; 2] = [vaddq_u32(x14[0], x25[0]), vaddq_u32(x14[1], x25[1])];
    let t679 = [vdupq_n_u32(x24); 2];
    let t680: [uint32x4_t; 2] = [vmulq_u32(x12[0], t679[0]), vmulq_u32(x12[1], t679[1])];
    let t681: [uint32x4_t; 2] = [vaddq_u32(t678[0], t680[0]), vaddq_u32(t678[1], t680[1])];
    let t682 = b17_ll_p(l, lid0, live, t681, x8);
    x26 = t682;
    let t683 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x4, x26, 8u32);
    let t684 = b8_s_p(l, lid0, live, x24);
    let t685: u64 = t684;
    let t686 = [vdupq_n_u32((t685) as u32); 2];
    let t687 = [vdupq_n_u32(((t685) >> 32) as u32); 2];
    let t688: [uint32x4_t; 4] = [vandq_u32(t683[0], t686[0]), vandq_u32(t683[1], t686[1]), vandq_u32(t683[2], t687[0]), vandq_u32(t683[3], t687[1])];
    res = t688;
    return res;
}

#[inline(always)]
unsafe fn b10_r0r1su8_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u32 = 0;
    let mut x5: u32 = 0;
    let mut x6: u32 = 0;
    let mut x7: u32 = 0;
    let mut x8: u32 = 0;
    let mut x9: u32 = 0;
    let mut x10: u32 = 0;
    let mut x11: u32 = 0;
    let mut x12: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x13: u32 = 0;
    let mut x14: u32 = 0;
    let mut x15: u32 = 0;
    let mut x16: u64 = 0;
    let mut x17: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x18: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x19: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x20: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x21: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x22: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x23: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x24: u32 = 0;
    let mut x25: u32 = 0;
    let mut x26: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t689 = b11_r0r1s_p(l, lid0, live, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x2)));
    x4 = t689;
    let t690 = b11_r0r1s_p(l, lid0, live, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x2)));
    x5 = t690;
    x6 = (x3 >> 10);
    let t691 = b15_r0r1sss_p(l, lid0, live, arg0, arg1, x5, (x6).wrapping_mul(4u32), 4u32);
    x7 = ((t691) as u32);
    let t692 = b15_r0r1sss_p(l, lid0, live, arg0, arg1, x5, ((x6).wrapping_mul(4u32)).wrapping_add(4u32), 4u32);
    x8 = ((t692) as u32);
    let t693 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    x9 = ((t693).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
    let t694 = b16_ss_p(l, lid0, live, 4u32, (x9).wrapping_sub((x6).wrapping_mul(4u32)));
    x10 = t694;
    x11 = (((x3 >> 8)) & (3u32));
    let t695: u32;
    if ((x11) == (0u32)) {
        t695 = 0u32;
    } else {
        let t696 = b17_ss_p(l, lid0, live, (x7).wrapping_add((2u32).wrapping_mul((x11).wrapping_sub(1u32))), x8);
        let t697 = b15_r0r1sss_p(l, lid0, live, arg0, arg1, x4, t696, 2u32);
        t695 = ((t697) as u32);
    }
    let t698 = [vdupq_n_u32(t695); 2];
    x12 = t698;
    let t699 = b17_ss_p(l, lid0, live, (x7).wrapping_add((2u32).wrapping_mul(x11)), x8);
    let t700 = b15_r0r1sss_p(l, lid0, live, arg0, arg1, x4, t699, 2u32);
    x13 = ((t700) as u32);
    x14 = (x7).wrapping_add((2u32).wrapping_mul(x10));
    x15 = (x3 & 255u32);
    x16 = 9259542123273814144u64;
    let t701: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x15), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x15), vld1q_u32(LANE[1].as_ptr()))];
    let t702: u64 = 72340172838076673u64;
    let t703 = [vdupq_n_u32((t702) as u32); 2];
    let t704 = [vdupq_n_u32(((t702) >> 32) as u32); 2];
    let t705: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(t701[0]), vget_low_u32(t703[0]))), vreinterpretq_u32_u64(vmull_high_u32(t701[0], t703[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(t701[1]), vget_low_u32(t703[1]))), vreinterpretq_u32_u64(vmull_high_u32(t701[1], t703[1])))];
    let t706: [uint32x4_t; 4] = [vmulq_u32(t701[0], t703[0]), vmulq_u32(t701[1], t703[1]), vmlaq_u32(t705[0], t701[0], t704[0]), vmlaq_u32(t705[1], t701[1], t704[1])];
    x17 = t706;
    x18 = x12;
    x19 = x12;
    let mut t709 = live;
    loop {
        // Lanes part: each its own rounds.
        let t711 = [vdupq_n_u32(x13); 2];
        let t712: [uint32x4_t; 2] = [vcltq_u32(x19[0], t711[0]), vcltq_u32(x19[1], t711[1])];
        t709 = [vandq_u32(t709[0], t712[0]), vandq_u32(t709[1], t712[1])];
        if !(any(&t709)) { break; }
        let t714 = [vdupq_n_u32(x14); 2];
        let t715: [uint32x4_t; 2] = [vaddq_u32(t714[0], x19[0]), vaddq_u32(t714[1], x19[1])];
        let t716 = b17_ls_p(l, lid0, t709, t715, x8);
        let t717 = b15_r0r1sls_p(l, lid0, t709, arg0, arg1, x4, t716, 8u32);
        x20 = t717;
        let t718: u64 = x16;
        let t719 = [vdupq_n_u32((t718) as u32); 2];
        let t720 = [vdupq_n_u32(((t718) >> 32) as u32); 2];
        let t721: [uint32x4_t; 4] = [vorrq_u32(x20[0], t719[0]), vorrq_u32(x20[1], t719[1]), vorrq_u32(x20[2], t720[0]), vorrq_u32(x20[3], t720[1])];
        let t722: u64 = !(x16);
        let t723 = [vdupq_n_u32((t722) as u32); 2];
        let t724 = [vdupq_n_u32(((t722) >> 32) as u32); 2];
        let t725: [uint32x4_t; 4] = [vandq_u32(x17[0], t723[0]), vandq_u32(x17[1], t723[1]), vandq_u32(x17[2], t724[0]), vandq_u32(x17[3], t724[1])];
        let t726: [uint32x4_t; 2] = [vcltq_u32(t721[0], t725[0]), vcltq_u32(t721[1], t725[1])];
        let t727: [uint32x4_t; 4] = [vsubq_u32(t721[0], t725[0]), vsubq_u32(t721[1], t725[1]), vaddq_u32(vsubq_u32(t721[2], t725[2]), t726[0]), vaddq_u32(vsubq_u32(t721[3], t725[3]), t726[1])];
        x21 = t727;
        let t728: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x20[r]));
        let t729: [uint32x4_t; 4] = [vandq_u32(t728[0], x17[0]), vandq_u32(t728[1], x17[1]), vandq_u32(t728[2], x17[2]), vandq_u32(t728[3], x17[3])];
        let t730: [uint32x4_t; 4] = [veorq_u32(x20[0], x17[0]), veorq_u32(x20[1], x17[1]), veorq_u32(x20[2], x17[2]), veorq_u32(x20[3], x17[3])];
        let t731: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(t730[r]));
        let t732: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x21[r]));
        let t733: [uint32x4_t; 4] = [vandq_u32(t731[0], t732[0]), vandq_u32(t731[1], t732[1]), vandq_u32(t731[2], t732[2]), vandq_u32(t731[3], t732[3])];
        let t734: [uint32x4_t; 4] = [vorrq_u32(t729[0], t733[0]), vorrq_u32(t729[1], t733[1]), vorrq_u32(t729[2], t733[2]), vorrq_u32(t729[3], t733[3])];
        let t735: u64 = x16;
        let t736 = [vdupq_n_u32((t735) as u32); 2];
        let t737 = [vdupq_n_u32(((t735) >> 32) as u32); 2];
        let t738: [uint32x4_t; 4] = [vandq_u32(t736[0], t734[0]), vandq_u32(t736[1], t734[1]), vandq_u32(t737[0], t734[2]), vandq_u32(t737[1], t734[3])];
        x22 = t738;
        let t739 = [vdupq_n_u32(x13); 2];
        let t740: [uint32x4_t; 2] = [vsubq_u32(t739[0], x19[0]), vsubq_u32(t739[1], x19[1])];
        x23 = t740;
        let t741 = [vdupq_n_u32(8u32); 2];
        let t742: [uint32x4_t; 2] = [vcgeq_u32(x23[0], t741[0]), vcgeq_u32(x23[1], t741[1])];
        let mut t743: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t744: [uint32x4_t; 2] = [vandq_u32(t709[0], t742[0]), vandq_u32(t709[1], t742[1])];
        let t745: [uint32x4_t; 2] = [vbicq_u32(t709[0], t742[0]), vbicq_u32(t709[1], t742[1])];
        {
            t743 = [vbslq_u32(t744[0], x22[0], t743[0]), vbslq_u32(t744[1], x22[1], t743[1]), vbslq_u32(t744[0], x22[2], t743[2]), vbslq_u32(t744[1], x22[3], t743[3])];
        }
        {
            let t746: [uint32x4_t; 2] = [vmulq_u32(t741[0], x23[0]), vmulq_u32(t741[1], x23[1])];
            let t747: u64 = 1u64;
            let t748 = [vdupq_n_u32((t747) as u32); 2];
            let t749: [uint32x4_t; 2] = [t746[0], t746[1]];
            let t750: [uint32x4_t; 2] = [vandq_u32(t749[0], vdupq_n_u32(63)), vandq_u32(t749[1], vdupq_n_u32(63))];
            let t751: [uint32x4_t; 4] = [vshlq_u32(t748[0], vreinterpretq_s32_u32(t750[0])), vshlq_u32(t748[1], vreinterpretq_s32_u32(t750[1])), vshlq_u32(t748[0], vsubq_s32(vreinterpretq_s32_u32(t750[0]), vdupq_n_s32(32))), vshlq_u32(t748[1], vsubq_s32(vreinterpretq_s32_u32(t750[1]), vdupq_n_s32(32)))];
            let t752: u64 = 1u64;
            let t753 = [vdupq_n_u32((t752) as u32); 2];
            let t754: [uint32x4_t; 2] = [vcltq_u32(t751[0], t753[0]), vcltq_u32(t751[1], t753[1])];
            let t755: [uint32x4_t; 4] = [vsubq_u32(t751[0], t753[0]), vsubq_u32(t751[1], t753[1]), vaddq_u32(vsubq_u32(t751[2], vdupq_n_u32(0)), t754[0]), vaddq_u32(vsubq_u32(t751[3], vdupq_n_u32(0)), t754[1])];
            let t756: [uint32x4_t; 4] = [vandq_u32(x22[0], t755[0]), vandq_u32(x22[1], t755[1]), vandq_u32(x22[2], t755[2]), vandq_u32(x22[3], t755[3])];
            t743 = [vbslq_u32(t745[0], t756[0], t743[0]), vbslq_u32(t745[1], t756[1], t743[1]), vbslq_u32(t745[0], t756[2], t743[2]), vbslq_u32(t745[1], t756[3], t743[3])];
        }
        let t757 = b18_l_p(l, lid0, t709, t743);
        let t758: [uint32x4_t; 2] = [vaddq_u32(x18[0], t757[0]), vaddq_u32(x18[1], t757[1])];
        x18 = [vbslq_u32(t709[0], t758[0], x18[0]), vbslq_u32(t709[1], t758[1], x18[1])];
        let t759: [uint32x4_t; 2] = [vandq_u32(t709[0], live[0]), vandq_u32(t709[1], live[1])];
        let t760: [uint32x4_t; 2] = [vaddq_u32(x19[0], t741[0]), vaddq_u32(x19[1], t741[1])];
        x19 = t760;
    }
    x12 = x18;
    let t761 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    x24 = (((t761).wrapping_shr((((((4u32).wrapping_mul(x2)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32));
    let t762 = b17_ss_p(l, lid0, live, (x7).wrapping_add((2u32).wrapping_mul((x10).wrapping_sub(1u32))), x8);
    let t763 = b15_r0r1sss_p(l, lid0, live, arg0, arg1, x4, t762, 2u32);
    x25 = ((t763) as u32);
    let t764 = [vdupq_n_u32(x24); 2];
    let t765: [uint32x4_t; 2] = [vmulq_u32(x12[0], t764[0]), vmulq_u32(x12[1], t764[1])];
    let t766 = [vdupq_n_u32((x14).wrapping_add(x25)); 2];
    let t767: [uint32x4_t; 2] = [vaddq_u32(t766[0], t765[0]), vaddq_u32(t766[1], t765[1])];
    let t768 = b17_ls_p(l, lid0, live, t767, x8);
    x26 = t768;
    let t769 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x4, x26, 8u32);
    let t770 = b8_s_p(l, lid0, live, x24);
    let t771: u64 = t770;
    let t772 = [vdupq_n_u32((t771) as u32); 2];
    let t773 = [vdupq_n_u32(((t771) >> 32) as u32); 2];
    let t774: [uint32x4_t; 4] = [vandq_u32(t769[0], t772[0]), vandq_u32(t769[1], t772[1]), vandq_u32(t769[2], t773[0]), vandq_u32(t769[3], t773[1])];
    res = t774;
    return res;
}

#[inline(always)]
unsafe fn b10_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u32 = 0;
    let mut x5: u32 = 0;
    let mut x6: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x7: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x8: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x9: u32 = 0;
    let mut x10: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x11: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x12: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x13: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x14: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x15: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x16: u64 = 0;
    let mut x17: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x18: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x19: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x20: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x21: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x22: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x23: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x24: u32 = 0;
    let mut x25: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x26: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t775 = b11_r0r1s_p(l, lid0, live, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x2)));
    x4 = t775;
    let t776 = b11_r0r1s_p(l, lid0, live, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x2)));
    x5 = t776;
    let t777: [uint32x4_t; 2] = [vshlq_u32(x3[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x3[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    x6 = t777;
    let t778 = [vdupq_n_u32(4u32); 2];
    let t779: [uint32x4_t; 2] = [vmulq_u32(x6[0], t778[0]), vmulq_u32(x6[1], t778[1])];
    let t780 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x5, t779, 4u32);
    let t781: [uint32x4_t; 2] = [t780[0], t780[1]];
    x7 = t781;
    let t782: [uint32x4_t; 2] = [vmulq_u32(x6[0], t778[0]), vmulq_u32(x6[1], t778[1])];
    let t783: [uint32x4_t; 2] = [vaddq_u32(t782[0], t778[0]), vaddq_u32(t782[1], t778[1])];
    let t784 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x5, t783, 4u32);
    let t785: [uint32x4_t; 2] = [t784[0], t784[1]];
    x8 = t785;
    let t786 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    x9 = ((t786).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
    let t787: [uint32x4_t; 2] = [vmulq_u32(x6[0], t778[0]), vmulq_u32(x6[1], t778[1])];
    let t788 = [vdupq_n_u32(x9); 2];
    let t789: [uint32x4_t; 2] = [vsubq_u32(t788[0], t787[0]), vsubq_u32(t788[1], t787[1])];
    let t790 = b16_sl_p(l, lid0, live, 4u32, t789);
    x10 = t790;
    let t791: [uint32x4_t; 2] = [vshlq_u32(x3[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(x3[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
    let t792 = [vdupq_n_u32(3u32); 2];
    let t793: [uint32x4_t; 2] = [vandq_u32(t791[0], t792[0]), vandq_u32(t791[1], t792[1])];
    x11 = t793;
    let t794 = [vdupq_n_u32(0u32); 2];
    let t795: [uint32x4_t; 2] = [vceqq_u32(x11[0], t794[0]), vceqq_u32(x11[1], t794[1])];
    let mut t796: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t797: [uint32x4_t; 2] = [vandq_u32(live[0], t795[0]), vandq_u32(live[1], t795[1])];
    let t798: [uint32x4_t; 2] = [vbicq_u32(live[0], t795[0]), vbicq_u32(live[1], t795[1])];
    {
        t796 = [vbslq_u32(t797[0], t794[0], t796[0]), vbslq_u32(t797[1], t794[1], t796[1])];
    }
    if any(&t798) {
        let t799 = [vdupq_n_u32(1u32); 2];
        let t800: [uint32x4_t; 2] = [vsubq_u32(x11[0], t799[0]), vsubq_u32(x11[1], t799[1])];
        let t801 = [vdupq_n_u32(2u32); 2];
        let t802: [uint32x4_t; 2] = [vmulq_u32(t801[0], t800[0]), vmulq_u32(t801[1], t800[1])];
        let t803: [uint32x4_t; 2] = [vaddq_u32(x7[0], t802[0]), vaddq_u32(x7[1], t802[1])];
        let t804 = b17_ll_p(l, lid0, t798, t803, x8);
        let t805 = b15_r0r1sls_p(l, lid0, t798, arg0, arg1, x4, t804, 2u32);
        let t806: [uint32x4_t; 2] = [t805[0], t805[1]];
        t796 = [vbslq_u32(t798[0], t806[0], t796[0]), vbslq_u32(t798[1], t806[1], t796[1])];
    }
    x12 = t796;
    let t807 = [vdupq_n_u32(2u32); 2];
    let t808: [uint32x4_t; 2] = [vmulq_u32(t807[0], x11[0]), vmulq_u32(t807[1], x11[1])];
    let t809: [uint32x4_t; 2] = [vaddq_u32(x7[0], t808[0]), vaddq_u32(x7[1], t808[1])];
    let t810 = b17_ll_p(l, lid0, live, t809, x8);
    let t811 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x4, t810, 2u32);
    let t812: [uint32x4_t; 2] = [t811[0], t811[1]];
    x13 = t812;
    let t813: [uint32x4_t; 2] = [vmulq_u32(t807[0], x10[0]), vmulq_u32(t807[1], x10[1])];
    let t814: [uint32x4_t; 2] = [vaddq_u32(x7[0], t813[0]), vaddq_u32(x7[1], t813[1])];
    x14 = t814;
    let t815 = [vdupq_n_u32(255u32); 2];
    let t816: [uint32x4_t; 2] = [vandq_u32(x3[0], t815[0]), vandq_u32(x3[1], t815[1])];
    x15 = t816;
    x16 = 9259542123273814144u64;
    let t817: u64 = 72340172838076673u64;
    let t818 = [vdupq_n_u32((t817) as u32); 2];
    let t819 = [vdupq_n_u32(((t817) >> 32) as u32); 2];
    let t820: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x15[0]), vget_low_u32(t818[0]))), vreinterpretq_u32_u64(vmull_high_u32(x15[0], t818[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x15[1]), vget_low_u32(t818[1]))), vreinterpretq_u32_u64(vmull_high_u32(x15[1], t818[1])))];
    let t821: [uint32x4_t; 4] = [vmulq_u32(x15[0], t818[0]), vmulq_u32(x15[1], t818[1]), vmlaq_u32(t820[0], x15[0], t819[0]), vmlaq_u32(t820[1], x15[1], t819[1])];
    x17 = t821;
    x18 = x12;
    x19 = x12;
    let mut t823 = live;
    loop {
        // Lanes part: each its own rounds.
        let t825: [uint32x4_t; 2] = [vcltq_u32(x19[0], x13[0]), vcltq_u32(x19[1], x13[1])];
        t823 = [vandq_u32(t823[0], t825[0]), vandq_u32(t823[1], t825[1])];
        if !(any(&t823)) { break; }
        let t827: [uint32x4_t; 2] = [vaddq_u32(x14[0], x19[0]), vaddq_u32(x14[1], x19[1])];
        let t828 = b17_ll_p(l, lid0, t823, t827, x8);
        let t829 = b15_r0r1sls_p(l, lid0, t823, arg0, arg1, x4, t828, 8u32);
        x20 = t829;
        let t830: u64 = x16;
        let t831 = [vdupq_n_u32((t830) as u32); 2];
        let t832 = [vdupq_n_u32(((t830) >> 32) as u32); 2];
        let t833: [uint32x4_t; 4] = [vorrq_u32(x20[0], t831[0]), vorrq_u32(x20[1], t831[1]), vorrq_u32(x20[2], t832[0]), vorrq_u32(x20[3], t832[1])];
        let t834: u64 = !(x16);
        let t835 = [vdupq_n_u32((t834) as u32); 2];
        let t836 = [vdupq_n_u32(((t834) >> 32) as u32); 2];
        let t837: [uint32x4_t; 4] = [vandq_u32(x17[0], t835[0]), vandq_u32(x17[1], t835[1]), vandq_u32(x17[2], t836[0]), vandq_u32(x17[3], t836[1])];
        let t838: [uint32x4_t; 2] = [vcltq_u32(t833[0], t837[0]), vcltq_u32(t833[1], t837[1])];
        let t839: [uint32x4_t; 4] = [vsubq_u32(t833[0], t837[0]), vsubq_u32(t833[1], t837[1]), vaddq_u32(vsubq_u32(t833[2], t837[2]), t838[0]), vaddq_u32(vsubq_u32(t833[3], t837[3]), t838[1])];
        x21 = t839;
        let t840: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x20[r]));
        let t841: [uint32x4_t; 4] = [vandq_u32(t840[0], x17[0]), vandq_u32(t840[1], x17[1]), vandq_u32(t840[2], x17[2]), vandq_u32(t840[3], x17[3])];
        let t842: [uint32x4_t; 4] = [veorq_u32(x20[0], x17[0]), veorq_u32(x20[1], x17[1]), veorq_u32(x20[2], x17[2]), veorq_u32(x20[3], x17[3])];
        let t843: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(t842[r]));
        let t844: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x21[r]));
        let t845: [uint32x4_t; 4] = [vandq_u32(t843[0], t844[0]), vandq_u32(t843[1], t844[1]), vandq_u32(t843[2], t844[2]), vandq_u32(t843[3], t844[3])];
        let t846: [uint32x4_t; 4] = [vorrq_u32(t841[0], t845[0]), vorrq_u32(t841[1], t845[1]), vorrq_u32(t841[2], t845[2]), vorrq_u32(t841[3], t845[3])];
        let t847: u64 = x16;
        let t848 = [vdupq_n_u32((t847) as u32); 2];
        let t849 = [vdupq_n_u32(((t847) >> 32) as u32); 2];
        let t850: [uint32x4_t; 4] = [vandq_u32(t848[0], t846[0]), vandq_u32(t848[1], t846[1]), vandq_u32(t849[0], t846[2]), vandq_u32(t849[1], t846[3])];
        x22 = t850;
        let t851: [uint32x4_t; 2] = [vsubq_u32(x13[0], x19[0]), vsubq_u32(x13[1], x19[1])];
        x23 = t851;
        let t852 = [vdupq_n_u32(8u32); 2];
        let t853: [uint32x4_t; 2] = [vcgeq_u32(x23[0], t852[0]), vcgeq_u32(x23[1], t852[1])];
        let mut t854: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t855: [uint32x4_t; 2] = [vandq_u32(t823[0], t853[0]), vandq_u32(t823[1], t853[1])];
        let t856: [uint32x4_t; 2] = [vbicq_u32(t823[0], t853[0]), vbicq_u32(t823[1], t853[1])];
        {
            t854 = [vbslq_u32(t855[0], x22[0], t854[0]), vbslq_u32(t855[1], x22[1], t854[1]), vbslq_u32(t855[0], x22[2], t854[2]), vbslq_u32(t855[1], x22[3], t854[3])];
        }
        {
            let t857: [uint32x4_t; 2] = [vmulq_u32(t852[0], x23[0]), vmulq_u32(t852[1], x23[1])];
            let t858: u64 = 1u64;
            let t859 = [vdupq_n_u32((t858) as u32); 2];
            let t860: [uint32x4_t; 2] = [t857[0], t857[1]];
            let t861: [uint32x4_t; 2] = [vandq_u32(t860[0], vdupq_n_u32(63)), vandq_u32(t860[1], vdupq_n_u32(63))];
            let t862: [uint32x4_t; 4] = [vshlq_u32(t859[0], vreinterpretq_s32_u32(t861[0])), vshlq_u32(t859[1], vreinterpretq_s32_u32(t861[1])), vshlq_u32(t859[0], vsubq_s32(vreinterpretq_s32_u32(t861[0]), vdupq_n_s32(32))), vshlq_u32(t859[1], vsubq_s32(vreinterpretq_s32_u32(t861[1]), vdupq_n_s32(32)))];
            let t863: u64 = 1u64;
            let t864 = [vdupq_n_u32((t863) as u32); 2];
            let t865: [uint32x4_t; 2] = [vcltq_u32(t862[0], t864[0]), vcltq_u32(t862[1], t864[1])];
            let t866: [uint32x4_t; 4] = [vsubq_u32(t862[0], t864[0]), vsubq_u32(t862[1], t864[1]), vaddq_u32(vsubq_u32(t862[2], vdupq_n_u32(0)), t865[0]), vaddq_u32(vsubq_u32(t862[3], vdupq_n_u32(0)), t865[1])];
            let t867: [uint32x4_t; 4] = [vandq_u32(x22[0], t866[0]), vandq_u32(x22[1], t866[1]), vandq_u32(x22[2], t866[2]), vandq_u32(x22[3], t866[3])];
            t854 = [vbslq_u32(t856[0], t867[0], t854[0]), vbslq_u32(t856[1], t867[1], t854[1]), vbslq_u32(t856[0], t867[2], t854[2]), vbslq_u32(t856[1], t867[3], t854[3])];
        }
        let t868 = b18_l_p(l, lid0, t823, t854);
        let t869: [uint32x4_t; 2] = [vaddq_u32(x18[0], t868[0]), vaddq_u32(x18[1], t868[1])];
        x18 = [vbslq_u32(t823[0], t869[0], x18[0]), vbslq_u32(t823[1], t869[1], x18[1])];
        let t870: [uint32x4_t; 2] = [vandq_u32(t823[0], live[0]), vandq_u32(t823[1], live[1])];
        let t871: [uint32x4_t; 2] = [vaddq_u32(x19[0], t852[0]), vaddq_u32(x19[1], t852[1])];
        x19 = t871;
    }
    x12 = x18;
    let t872 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    x24 = (((t872).wrapping_shr((((((4u32).wrapping_mul(x2)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32));
    let t873 = [vdupq_n_u32(1u32); 2];
    let t874: [uint32x4_t; 2] = [vsubq_u32(x10[0], t873[0]), vsubq_u32(x10[1], t873[1])];
    let t875: [uint32x4_t; 2] = [vmulq_u32(t807[0], t874[0]), vmulq_u32(t807[1], t874[1])];
    let t876: [uint32x4_t; 2] = [vaddq_u32(x7[0], t875[0]), vaddq_u32(x7[1], t875[1])];
    let t877 = b17_ll_p(l, lid0, live, t876, x8);
    let t878 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x4, t877, 2u32);
    let t879: [uint32x4_t; 2] = [t878[0], t878[1]];
    x25 = t879;
    let t880: [uint32x4_t; 2] = [vaddq_u32(x14[0], x25[0]), vaddq_u32(x14[1], x25[1])];
    let t881 = [vdupq_n_u32(x24); 2];
    let t882: [uint32x4_t; 2] = [vmulq_u32(x12[0], t881[0]), vmulq_u32(x12[1], t881[1])];
    let t883: [uint32x4_t; 2] = [vaddq_u32(t880[0], t882[0]), vaddq_u32(t880[1], t882[1])];
    let t884 = b17_ll_p(l, lid0, live, t883, x8);
    x26 = t884;
    let t885 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x4, x26, 8u32);
    let t886 = b8_s_p(l, lid0, live, x24);
    let t887: u64 = t886;
    let t888 = [vdupq_n_u32((t887) as u32); 2];
    let t889 = [vdupq_n_u32(((t887) >> 32) as u32); 2];
    let t890: [uint32x4_t; 4] = [vandq_u32(t885[0], t888[0]), vandq_u32(t885[1], t888[1]), vandq_u32(t885[2], t889[0]), vandq_u32(t885[3], t889[1])];
    res = t890;
    return res;
}

#[inline(always)]
unsafe fn b11_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    res = (arg1.wrapping_add(((((x2).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned();
    return res;
}

#[inline(always)]
unsafe fn b11_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    res = (arg1.wrapping_add(((((x2).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned();
    return res;
}

#[inline(always)]
unsafe fn b12_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t891 = b11_r0r1s_p(l, lid0, live, arg0, arg1, (20u32).wrapping_add((4u32).wrapping_mul(x2)));
    res = t891;
    return res;
}

#[inline(always)]
unsafe fn b12_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t892 = b11_r0r1s_f(l, lid0, arg0, arg1, (20u32).wrapping_add((4u32).wrapping_mul(x2)));
    res = t892;
    return res;
}

#[inline(always)]
unsafe fn b13_ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x1: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t893: u32;
    if ((x0) < (x1)) {
        t893 = x0;
    } else {
        t893 = x1;
    }
    res = t893;
    return res;
}

#[inline(always)]
unsafe fn b13_ss_f(l: &Launch, lid0: u32, x0: u32, x1: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t894: u32;
    if ((x0) < (x1)) {
        t894 = x0;
    } else {
        t894 = x1;
    }
    res = t894;
    return res;
}

#[inline(always)]
unsafe fn b13_ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x1: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t895 = [vdupq_n_u32(x1); 2];
    let t896: [uint32x4_t; 2] = [vcltq_u32(x0[0], t895[0]), vcltq_u32(x0[1], t895[1])];
    let mut t897: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t898: [uint32x4_t; 2] = [vandq_u32(live[0], t896[0]), vandq_u32(live[1], t896[1])];
    let t899: [uint32x4_t; 2] = [vbicq_u32(live[0], t896[0]), vbicq_u32(live[1], t896[1])];
    {
        t897 = [vbslq_u32(t898[0], x0[0], t897[0]), vbslq_u32(t898[1], x0[1], t897[1])];
    }
    {
        let t900 = [vdupq_n_u32(x1); 2];
        t897 = [vbslq_u32(t899[0], t900[0], t897[0]), vbslq_u32(t899[1], t900[1], t897[1])];
    }
    res = t897;
    return res;
}

#[inline(always)]
unsafe fn b13_ls_f(l: &Launch, lid0: u32, x0: [uint32x4_t; 2], x1: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t901 = [vdupq_n_u32(x1); 2];
    let t902: [uint32x4_t; 2] = [vcltq_u32(x0[0], t901[0]), vcltq_u32(x0[1], t901[1])];
    let mut t903: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t904: [uint32x4_t; 2] = [vmvnq_u32(t902[0]), vmvnq_u32(t902[1])];
    {
        t903 = [vbslq_u32(t902[0], x0[0], t903[0]), vbslq_u32(t902[1], x0[1], t903[1])];
    }
    {
        let t905 = [vdupq_n_u32(x1); 2];
        t903 = [vbslq_u32(t904[0], t905[0], t903[0]), vbslq_u32(t904[1], t905[1], t903[1])];
    }
    res = t903;
    return res;
}

#[inline(always)]
unsafe fn b14_r0sns_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1: u32, x2f: u32, x2s: u32, x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t906 = b19_r0an1s_p(l, lid0, live, arg0, ((((x1) as u64)) as usize as *const u8), x2f, x2s, x3);
    x4 = t906;
    let t907: [uint32x4_t; 4];
    if ((x3) >= (8u32)) {
        t907 = x4;
    } else {
        let t908 = b20_s_p(l, lid0, live, (8u32).wrapping_mul(x3));
        let t909: u64 = t908;
        let t910 = [vdupq_n_u32((t909) as u32); 2];
        let t911 = [vdupq_n_u32(((t909) >> 32) as u32); 2];
        let t912: [uint32x4_t; 4] = [vandq_u32(x4[0], t910[0]), vandq_u32(x4[1], t910[1]), vandq_u32(x4[2], t911[0]), vandq_u32(x4[3], t911[1])];
        t907 = t912;
    }
    res = t907;
    return res;
}

#[inline(always)]
unsafe fn b14_r0sns_f(l: &Launch, lid0: u32, arg0: *const u8, x1: u32, x2f: u32, x2s: u32, x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t913 = b19_r0an1s_f(l, lid0, arg0, ((((x1) as u64)) as usize as *const u8), x2f, x2s, x3);
    x4 = t913;
    let t914: [uint32x4_t; 4];
    if ((x3) >= (8u32)) {
        t914 = x4;
    } else {
        let t915 = b20_s_f(l, lid0, (8u32).wrapping_mul(x3));
        let t916: u64 = t915;
        let t917 = [vdupq_n_u32((t916) as u32); 2];
        let t918 = [vdupq_n_u32(((t916) >> 32) as u32); 2];
        let t919: [uint32x4_t; 4] = [vandq_u32(x4[0], t917[0]), vandq_u32(x4[1], t917[1]), vandq_u32(x4[2], t918[0]), vandq_u32(x4[3], t918[1])];
        t914 = t919;
    }
    res = t914;
    return res;
}

#[inline(always)]
unsafe fn b14_r0lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1: [uint32x4_t; 2], x2: [uint32x4_t; 2], x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t920: [uint32x4_t; 2] = [vaddq_u32(x1[0], x2[0]), vaddq_u32(x1[1], x2[1])];
    let t921: [uint32x4_t; 2] = [vcltq_u32(t920[0], x1[0]), vcltq_u32(t920[1], x1[1])];
    let t922: [uint32x4_t; 4] = [t920[0], t920[1], vshrq_n_u32::<31>(t921[0]), vshrq_n_u32::<31>(t921[1])];
    let t923 = b19_r0ls_p(l, lid0, live, arg0, t922, x3);
    x4 = t923;
    let t924: [uint32x4_t; 4];
    if ((x3) >= (8u32)) {
        t924 = x4;
    } else {
        let t925 = b20_s_p(l, lid0, live, (8u32).wrapping_mul(x3));
        let t926: u64 = t925;
        let t927 = [vdupq_n_u32((t926) as u32); 2];
        let t928 = [vdupq_n_u32(((t926) >> 32) as u32); 2];
        let t929: [uint32x4_t; 4] = [vandq_u32(x4[0], t927[0]), vandq_u32(x4[1], t927[1]), vandq_u32(x4[2], t928[0]), vandq_u32(x4[3], t928[1])];
        t924 = t929;
    }
    res = t924;
    return res;
}

#[inline(always)]
unsafe fn b14_r0lls_f(l: &Launch, lid0: u32, arg0: *const u8, x1: [uint32x4_t; 2], x2: [uint32x4_t; 2], x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t930: [uint32x4_t; 2] = [vaddq_u32(x1[0], x2[0]), vaddq_u32(x1[1], x2[1])];
    let t931: [uint32x4_t; 2] = [vcltq_u32(t930[0], x1[0]), vcltq_u32(t930[1], x1[1])];
    let t932: [uint32x4_t; 4] = [t930[0], t930[1], vshrq_n_u32::<31>(t931[0]), vshrq_n_u32::<31>(t931[1])];
    let t933 = b19_r0ls_f(l, lid0, arg0, t932, x3);
    x4 = t933;
    let t934: [uint32x4_t; 4];
    if ((x3) >= (8u32)) {
        t934 = x4;
    } else {
        let t935 = b20_s_f(l, lid0, (8u32).wrapping_mul(x3));
        let t936: u64 = t935;
        let t937 = [vdupq_n_u32((t936) as u32); 2];
        let t938 = [vdupq_n_u32(((t936) >> 32) as u32); 2];
        let t939: [uint32x4_t; 4] = [vandq_u32(x4[0], t937[0]), vandq_u32(x4[1], t937[1]), vandq_u32(x4[2], t938[0]), vandq_u32(x4[3], t938[1])];
        t934 = t939;
    }
    res = t934;
    return res;
}

#[inline(always)]
unsafe fn b15_r0r1sss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: u32, x4: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t940 = b21_r0sss_p(l, lid0, live, arg0, x2, x3, x4);
    res = t940;
    return res;
}

#[inline(always)]
unsafe fn b15_r0r1sss_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32, x3: u32, x4: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t941 = b21_r0sss_f(l, lid0, arg0, x2, x3, x4);
    res = t941;
    return res;
}

#[inline(always)]
unsafe fn b15_r0r1sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: [uint32x4_t; 2], x4: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t942 = b21_r0sls_p(l, lid0, live, arg0, x2, x3, x4);
    res = t942;
    return res;
}

#[inline(always)]
unsafe fn b16_ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x1: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t943: u32;
    if ((x0) < (x1)) {
        t943 = x0;
    } else {
        t943 = x1;
    }
    res = t943;
    return res;
}

#[inline(always)]
unsafe fn b16_ss_f(l: &Launch, lid0: u32, x0: u32, x1: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t944: u32;
    if ((x0) < (x1)) {
        t944 = x0;
    } else {
        t944 = x1;
    }
    res = t944;
    return res;
}

#[inline(always)]
unsafe fn b16_sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x1: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t945 = [vdupq_n_u32(x0); 2];
    let t946: [uint32x4_t; 2] = [vcltq_u32(t945[0], x1[0]), vcltq_u32(t945[1], x1[1])];
    let mut t947: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t948: [uint32x4_t; 2] = [vandq_u32(live[0], t946[0]), vandq_u32(live[1], t946[1])];
    let t949: [uint32x4_t; 2] = [vbicq_u32(live[0], t946[0]), vbicq_u32(live[1], t946[1])];
    {
        let t950 = [vdupq_n_u32(x0); 2];
        t947 = [vbslq_u32(t948[0], t950[0], t947[0]), vbslq_u32(t948[1], t950[1], t947[1])];
    }
    {
        t947 = [vbslq_u32(t949[0], x1[0], t947[0]), vbslq_u32(t949[1], x1[1], t947[1])];
    }
    res = t947;
    return res;
}

#[inline(always)]
unsafe fn b17_ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x1: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t951: u32;
    if ((x0) < (x1)) {
        t951 = x0;
    } else {
        t951 = x1;
    }
    res = t951;
    return res;
}

#[inline(always)]
unsafe fn b17_ss_f(l: &Launch, lid0: u32, x0: u32, x1: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t952: u32;
    if ((x0) < (x1)) {
        t952 = x0;
    } else {
        t952 = x1;
    }
    res = t952;
    return res;
}

#[inline(always)]
unsafe fn b17_ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x1: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t953 = [vdupq_n_u32(x1); 2];
    let t954: [uint32x4_t; 2] = [vcltq_u32(x0[0], t953[0]), vcltq_u32(x0[1], t953[1])];
    let mut t955: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t956: [uint32x4_t; 2] = [vandq_u32(live[0], t954[0]), vandq_u32(live[1], t954[1])];
    let t957: [uint32x4_t; 2] = [vbicq_u32(live[0], t954[0]), vbicq_u32(live[1], t954[1])];
    {
        t955 = [vbslq_u32(t956[0], x0[0], t955[0]), vbslq_u32(t956[1], x0[1], t955[1])];
    }
    {
        let t958 = [vdupq_n_u32(x1); 2];
        t955 = [vbslq_u32(t957[0], t958[0], t955[0]), vbslq_u32(t957[1], t958[1], t955[1])];
    }
    res = t955;
    return res;
}

#[inline(always)]
unsafe fn b17_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x1: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t959: [uint32x4_t; 2] = [vcltq_u32(x0[0], x1[0]), vcltq_u32(x0[1], x1[1])];
    let mut t960: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t961: [uint32x4_t; 2] = [vandq_u32(live[0], t959[0]), vandq_u32(live[1], t959[1])];
    let t962: [uint32x4_t; 2] = [vbicq_u32(live[0], t959[0]), vbicq_u32(live[1], t959[1])];
    {
        t960 = [vbslq_u32(t961[0], x0[0], t960[0]), vbslq_u32(t961[1], x0[1], t960[1])];
    }
    {
        t960 = [vbslq_u32(t962[0], x1[0], t960[0]), vbslq_u32(t962[1], x1[1], t960[1])];
    }
    res = t960;
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
    let t963: [uint32x4_t; 2] = [vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[0])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[2]))))), vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[1])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[3])))))];
    res = t963;
    return res;
}

#[inline(always)]
unsafe fn b19_r0ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1: u64, x2: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x3: *const u8 = core::ptr::null();
    let mut x4: u32 = 0;
    let mut x5: u64 = 0;
    let mut live = k;
    let mut res: u64 = 0;
    if ((x2) == (8u32)) {
        res = (arg0.wrapping_add(x1 as usize) as *const u64).read_unaligned();
        return res;
    }
    x3 = arg0.wrapping_add(((x1) & (!(7u64))) as usize);
    x4 = (((((x1) & (7u64))) as u32)).wrapping_mul(8u32);
    x5 = ((x3 as *const u64).read_unaligned()).wrapping_shr((((x4) as u64)) as u32);
    let mut t964: bool = ((x4) != (0u32));
    if t964 {
        t964 = (((x4).wrapping_add((x2).wrapping_mul(8u32))) > (64u32));
    }
    let t965: u64;
    if t964 {
        t965 = ((x5) | (((x3.wrapping_add((1u64).wrapping_mul(8u64) as usize) as *const u64).read_unaligned()).wrapping_shl(((((64u32).wrapping_sub(x4)) as u64)) as u32)));
    } else {
        t965 = x5;
    }
    res = t965;
    return res;
}

#[inline(always)]
unsafe fn b19_r0ss_f(l: &Launch, lid0: u32, arg0: *const u8, x1: u64, x2: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x3: *const u8 = core::ptr::null();
    let mut x4: u32 = 0;
    let mut x5: u64 = 0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    if ((x2) == (8u32)) {
        res = (arg0.wrapping_add(x1 as usize) as *const u64).read_unaligned();
        return res;
    }
    x3 = arg0.wrapping_add(((x1) & (!(7u64))) as usize);
    x4 = (((((x1) & (7u64))) as u32)).wrapping_mul(8u32);
    x5 = ((x3 as *const u64).read_unaligned()).wrapping_shr((((x4) as u64)) as u32);
    let mut t966: bool = ((x4) != (0u32));
    if t966 {
        t966 = (((x4).wrapping_add((x2).wrapping_mul(8u32))) > (64u32));
    }
    let t967: u64;
    if t966 {
        t967 = ((x5) | (((x3.wrapping_add((1u64).wrapping_mul(8u64) as usize) as *const u64).read_unaligned()).wrapping_shl(((((64u32).wrapping_sub(x4)) as u64)) as u32)));
    } else {
        t967 = x5;
    }
    res = t967;
    return res;
}

#[inline(always)]
unsafe fn b19_r0ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1: [uint32x4_t; 4], x2: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x3: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x4: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x5: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if ((x2) == (8u32)) {
        let t968: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
        let t969: [uint32x4_t; 4] = [vandq_u32(x1[0], t968[0]), vandq_u32(x1[1], t968[1]), vandq_u32(x1[2], t968[2]), vandq_u32(x1[3], t968[3])];
        let t970: u64 = ((arg0) as usize as u64);
        let t971 = [vdupq_n_u32((t970) as u32); 2];
        let t972 = [vdupq_n_u32(((t970) >> 32) as u32); 2];
        let t973: [uint32x4_t; 2] = [vaddq_u32(t971[0], t969[0]), vaddq_u32(t971[1], t969[1])];
        let t974: [uint32x4_t; 2] = [vcltq_u32(t973[0], t971[0]), vcltq_u32(t973[1], t971[1])];
        let t975: [uint32x4_t; 4] = [t973[0], t973[1], vsubq_u32(vaddq_u32(t972[0], t969[2]), t974[0]), vsubq_u32(vaddq_u32(t972[1], t969[3]), t974[1])];
        let mut t976: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t975[2]) as u64) << 32 | vgetq_lane_u32::<0>(t975[0]) as u64) as usize as *const u64).read_unaligned(); t976[0] = vsetq_lane_u32::<0>(v as u32, t976[0]); t976[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t976[2]); }
        { let v = (((vgetq_lane_u32::<1>(t975[2]) as u64) << 32 | vgetq_lane_u32::<1>(t975[0]) as u64) as usize as *const u64).read_unaligned(); t976[0] = vsetq_lane_u32::<1>(v as u32, t976[0]); t976[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t976[2]); }
        { let v = (((vgetq_lane_u32::<2>(t975[2]) as u64) << 32 | vgetq_lane_u32::<2>(t975[0]) as u64) as usize as *const u64).read_unaligned(); t976[0] = vsetq_lane_u32::<2>(v as u32, t976[0]); t976[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t976[2]); }
        { let v = (((vgetq_lane_u32::<3>(t975[2]) as u64) << 32 | vgetq_lane_u32::<3>(t975[0]) as u64) as usize as *const u64).read_unaligned(); t976[0] = vsetq_lane_u32::<3>(v as u32, t976[0]); t976[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t976[2]); }
        { let v = (((vgetq_lane_u32::<0>(t975[3]) as u64) << 32 | vgetq_lane_u32::<0>(t975[1]) as u64) as usize as *const u64).read_unaligned(); t976[1] = vsetq_lane_u32::<0>(v as u32, t976[1]); t976[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t976[3]); }
        { let v = (((vgetq_lane_u32::<1>(t975[3]) as u64) << 32 | vgetq_lane_u32::<1>(t975[1]) as u64) as usize as *const u64).read_unaligned(); t976[1] = vsetq_lane_u32::<1>(v as u32, t976[1]); t976[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t976[3]); }
        { let v = (((vgetq_lane_u32::<2>(t975[3]) as u64) << 32 | vgetq_lane_u32::<2>(t975[1]) as u64) as usize as *const u64).read_unaligned(); t976[1] = vsetq_lane_u32::<2>(v as u32, t976[1]); t976[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t976[3]); }
        { let v = (((vgetq_lane_u32::<3>(t975[3]) as u64) << 32 | vgetq_lane_u32::<3>(t975[1]) as u64) as usize as *const u64).read_unaligned(); t976[1] = vsetq_lane_u32::<3>(v as u32, t976[1]); t976[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t976[3]); }
        res = t976;
        return res;
    }
    let t977: u64 = !(7u64);
    let t978 = [vdupq_n_u32((t977) as u32); 2];
    let t979 = [vdupq_n_u32(((t977) >> 32) as u32); 2];
    let t980: [uint32x4_t; 4] = [vandq_u32(x1[0], t978[0]), vandq_u32(x1[1], t978[1]), vandq_u32(x1[2], t979[0]), vandq_u32(x1[3], t979[1])];
    let t981: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
    let t982: [uint32x4_t; 4] = [vandq_u32(t980[0], t981[0]), vandq_u32(t980[1], t981[1]), vandq_u32(t980[2], t981[2]), vandq_u32(t980[3], t981[3])];
    let t983: u64 = ((arg0) as usize as u64);
    let t984 = [vdupq_n_u32((t983) as u32); 2];
    let t985 = [vdupq_n_u32(((t983) >> 32) as u32); 2];
    let t986: [uint32x4_t; 2] = [vaddq_u32(t984[0], t982[0]), vaddq_u32(t984[1], t982[1])];
    let t987: [uint32x4_t; 2] = [vcltq_u32(t986[0], t984[0]), vcltq_u32(t986[1], t984[1])];
    let t988: [uint32x4_t; 4] = [t986[0], t986[1], vsubq_u32(vaddq_u32(t985[0], t982[2]), t987[0]), vsubq_u32(vaddq_u32(t985[1], t982[3]), t987[1])];
    x3 = t988;
    let t989: u64 = 7u64;
    let t990 = [vdupq_n_u32((t989) as u32); 2];
    let t991: [uint32x4_t; 2] = [vandq_u32(x1[0], t990[0]), vandq_u32(x1[1], t990[1])];
    let t992 = [vdupq_n_u32(8u32); 2];
    let t993: [uint32x4_t; 2] = [vmulq_u32(t991[0], t992[0]), vmulq_u32(t991[1], t992[1])];
    x4 = t993;
    let mut t994: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x3[2]) as u64) << 32 | vgetq_lane_u32::<0>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t994[0] = vsetq_lane_u32::<0>(v as u32, t994[0]); t994[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t994[2]); }
    { let v = (((vgetq_lane_u32::<1>(x3[2]) as u64) << 32 | vgetq_lane_u32::<1>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t994[0] = vsetq_lane_u32::<1>(v as u32, t994[0]); t994[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t994[2]); }
    { let v = (((vgetq_lane_u32::<2>(x3[2]) as u64) << 32 | vgetq_lane_u32::<2>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t994[0] = vsetq_lane_u32::<2>(v as u32, t994[0]); t994[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t994[2]); }
    { let v = (((vgetq_lane_u32::<3>(x3[2]) as u64) << 32 | vgetq_lane_u32::<3>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t994[0] = vsetq_lane_u32::<3>(v as u32, t994[0]); t994[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t994[2]); }
    { let v = (((vgetq_lane_u32::<0>(x3[3]) as u64) << 32 | vgetq_lane_u32::<0>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t994[1] = vsetq_lane_u32::<0>(v as u32, t994[1]); t994[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t994[3]); }
    { let v = (((vgetq_lane_u32::<1>(x3[3]) as u64) << 32 | vgetq_lane_u32::<1>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t994[1] = vsetq_lane_u32::<1>(v as u32, t994[1]); t994[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t994[3]); }
    { let v = (((vgetq_lane_u32::<2>(x3[3]) as u64) << 32 | vgetq_lane_u32::<2>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t994[1] = vsetq_lane_u32::<2>(v as u32, t994[1]); t994[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t994[3]); }
    { let v = (((vgetq_lane_u32::<3>(x3[3]) as u64) << 32 | vgetq_lane_u32::<3>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t994[1] = vsetq_lane_u32::<3>(v as u32, t994[1]); t994[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t994[3]); }
    let t995: [uint32x4_t; 2] = [x4[0], x4[1]];
    let t996: [uint32x4_t; 2] = [vandq_u32(t995[0], vdupq_n_u32(63)), vandq_u32(t995[1], vdupq_n_u32(63))];
    let t997: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t994[0], vnegq_s32(vreinterpretq_s32_u32(t996[0]))), vshlq_u32(t994[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t996[0])))), vorrq_u32(vshlq_u32(t994[1], vnegq_s32(vreinterpretq_s32_u32(t996[1]))), vshlq_u32(t994[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t996[1])))), vshlq_u32(t994[2], vnegq_s32(vreinterpretq_s32_u32(t996[0]))), vshlq_u32(t994[3], vnegq_s32(vreinterpretq_s32_u32(t996[1])))];
    x5 = t997;
    let t998 = [vdupq_n_u32(0u32); 2];
    let t999: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t998[0])), vmvnq_u32(vceqq_u32(x4[1], t998[1]))];
    let t1000 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
    let t1001: [uint32x4_t; 2] = [vaddq_u32(x4[0], t1000[0]), vaddq_u32(x4[1], t1000[1])];
    let t1002 = [vdupq_n_u32(64u32); 2];
    let t1003: [uint32x4_t; 2] = [vcgtq_u32(t1001[0], t1002[0]), vcgtq_u32(t1001[1], t1002[1])];
    let t1004: [uint32x4_t; 2] = [vandq_u32(t999[0], t1003[0]), vandq_u32(t999[1], t1003[1])];
    let mut t1005: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1006: [uint32x4_t; 2] = [vandq_u32(live[0], t1004[0]), vandq_u32(live[1], t1004[1])];
    let t1007: [uint32x4_t; 2] = [vbicq_u32(live[0], t1004[0]), vbicq_u32(live[1], t1004[1])];
    if any(&t1006) {
        let t1008: u64 = (1u64).wrapping_mul(8u64);
        let t1009 = [vdupq_n_u32((t1008) as u32); 2];
        let t1010 = [vdupq_n_u32(((t1008) >> 32) as u32); 2];
        let t1011: [uint32x4_t; 2] = [vaddq_u32(x3[0], t1009[0]), vaddq_u32(x3[1], t1009[1])];
        let t1012: [uint32x4_t; 2] = [vcltq_u32(t1011[0], x3[0]), vcltq_u32(t1011[1], x3[1])];
        let t1013: [uint32x4_t; 4] = [t1011[0], t1011[1], vsubq_u32(vaddq_u32(x3[2], t1010[0]), t1012[0]), vsubq_u32(vaddq_u32(x3[3], t1010[1]), t1012[1])];
        let mut t1014: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1013[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1013[0]) as u64) as usize as *const u64).read_unaligned(); t1014[0] = vsetq_lane_u32::<0>(v as u32, t1014[0]); t1014[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1014[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1013[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1013[0]) as u64) as usize as *const u64).read_unaligned(); t1014[0] = vsetq_lane_u32::<1>(v as u32, t1014[0]); t1014[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1014[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1013[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1013[0]) as u64) as usize as *const u64).read_unaligned(); t1014[0] = vsetq_lane_u32::<2>(v as u32, t1014[0]); t1014[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1014[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1013[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1013[0]) as u64) as usize as *const u64).read_unaligned(); t1014[0] = vsetq_lane_u32::<3>(v as u32, t1014[0]); t1014[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1014[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1013[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1013[1]) as u64) as usize as *const u64).read_unaligned(); t1014[1] = vsetq_lane_u32::<0>(v as u32, t1014[1]); t1014[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1014[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1013[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1013[1]) as u64) as usize as *const u64).read_unaligned(); t1014[1] = vsetq_lane_u32::<1>(v as u32, t1014[1]); t1014[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1014[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1013[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1013[1]) as u64) as usize as *const u64).read_unaligned(); t1014[1] = vsetq_lane_u32::<2>(v as u32, t1014[1]); t1014[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1014[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1013[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1013[1]) as u64) as usize as *const u64).read_unaligned(); t1014[1] = vsetq_lane_u32::<3>(v as u32, t1014[1]); t1014[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1014[3]); }
        let t1015: [uint32x4_t; 2] = [vsubq_u32(t1002[0], x4[0]), vsubq_u32(t1002[1], x4[1])];
        let t1016: [uint32x4_t; 2] = [t1015[0], t1015[1]];
        let t1017: [uint32x4_t; 2] = [vandq_u32(t1016[0], vdupq_n_u32(63)), vandq_u32(t1016[1], vdupq_n_u32(63))];
        let t1018: [uint32x4_t; 4] = [vshlq_u32(t1014[0], vreinterpretq_s32_u32(t1017[0])), vshlq_u32(t1014[1], vreinterpretq_s32_u32(t1017[1])), vorrq_u32(vshlq_u32(t1014[2], vreinterpretq_s32_u32(t1017[0])), vshlq_u32(t1014[0], vsubq_s32(vreinterpretq_s32_u32(t1017[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1014[3], vreinterpretq_s32_u32(t1017[1])), vshlq_u32(t1014[1], vsubq_s32(vreinterpretq_s32_u32(t1017[1]), vdupq_n_s32(32))))];
        let t1019: [uint32x4_t; 4] = [vorrq_u32(x5[0], t1018[0]), vorrq_u32(x5[1], t1018[1]), vorrq_u32(x5[2], t1018[2]), vorrq_u32(x5[3], t1018[3])];
        t1005 = [vbslq_u32(t1006[0], t1019[0], t1005[0]), vbslq_u32(t1006[1], t1019[1], t1005[1]), vbslq_u32(t1006[0], t1019[2], t1005[2]), vbslq_u32(t1006[1], t1019[3], t1005[3])];
    }
    {
        t1005 = [vbslq_u32(t1007[0], x5[0], t1005[0]), vbslq_u32(t1007[1], x5[1], t1005[1]), vbslq_u32(t1007[0], x5[2], t1005[2]), vbslq_u32(t1007[1], x5[3], t1005[3])];
    }
    res = t1005;
    return res;
}

#[inline(always)]
unsafe fn b19_r0ls_f(l: &Launch, lid0: u32, arg0: *const u8, x1: [uint32x4_t; 4], x2: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x3: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x4: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x5: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if ((x2) == (8u32)) {
        let t1020: u64 = ((arg0) as usize as u64);
        let t1021 = [vdupq_n_u32((t1020) as u32); 2];
        let t1022 = [vdupq_n_u32(((t1020) >> 32) as u32); 2];
        let t1023: [uint32x4_t; 2] = [vaddq_u32(t1021[0], x1[0]), vaddq_u32(t1021[1], x1[1])];
        let t1024: [uint32x4_t; 2] = [vcltq_u32(t1023[0], t1021[0]), vcltq_u32(t1023[1], t1021[1])];
        let t1025: [uint32x4_t; 4] = [t1023[0], t1023[1], vsubq_u32(vaddq_u32(t1022[0], x1[2]), t1024[0]), vsubq_u32(vaddq_u32(t1022[1], x1[3]), t1024[1])];
        let mut t1026: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1025[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1025[0]) as u64) as usize as *const u64).read_unaligned(); t1026[0] = vsetq_lane_u32::<0>(v as u32, t1026[0]); t1026[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1026[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1025[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1025[0]) as u64) as usize as *const u64).read_unaligned(); t1026[0] = vsetq_lane_u32::<1>(v as u32, t1026[0]); t1026[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1026[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1025[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1025[0]) as u64) as usize as *const u64).read_unaligned(); t1026[0] = vsetq_lane_u32::<2>(v as u32, t1026[0]); t1026[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1026[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1025[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1025[0]) as u64) as usize as *const u64).read_unaligned(); t1026[0] = vsetq_lane_u32::<3>(v as u32, t1026[0]); t1026[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1026[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1025[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1025[1]) as u64) as usize as *const u64).read_unaligned(); t1026[1] = vsetq_lane_u32::<0>(v as u32, t1026[1]); t1026[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1026[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1025[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1025[1]) as u64) as usize as *const u64).read_unaligned(); t1026[1] = vsetq_lane_u32::<1>(v as u32, t1026[1]); t1026[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1026[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1025[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1025[1]) as u64) as usize as *const u64).read_unaligned(); t1026[1] = vsetq_lane_u32::<2>(v as u32, t1026[1]); t1026[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1026[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1025[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1025[1]) as u64) as usize as *const u64).read_unaligned(); t1026[1] = vsetq_lane_u32::<3>(v as u32, t1026[1]); t1026[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1026[3]); }
        res = t1026;
        return res;
    }
    let t1027: u64 = !(7u64);
    let t1028 = [vdupq_n_u32((t1027) as u32); 2];
    let t1029 = [vdupq_n_u32(((t1027) >> 32) as u32); 2];
    let t1030: [uint32x4_t; 4] = [vandq_u32(x1[0], t1028[0]), vandq_u32(x1[1], t1028[1]), vandq_u32(x1[2], t1029[0]), vandq_u32(x1[3], t1029[1])];
    let t1031: u64 = ((arg0) as usize as u64);
    let t1032 = [vdupq_n_u32((t1031) as u32); 2];
    let t1033 = [vdupq_n_u32(((t1031) >> 32) as u32); 2];
    let t1034: [uint32x4_t; 2] = [vaddq_u32(t1032[0], t1030[0]), vaddq_u32(t1032[1], t1030[1])];
    let t1035: [uint32x4_t; 2] = [vcltq_u32(t1034[0], t1032[0]), vcltq_u32(t1034[1], t1032[1])];
    let t1036: [uint32x4_t; 4] = [t1034[0], t1034[1], vsubq_u32(vaddq_u32(t1033[0], t1030[2]), t1035[0]), vsubq_u32(vaddq_u32(t1033[1], t1030[3]), t1035[1])];
    x3 = t1036;
    let t1037: u64 = 7u64;
    let t1038 = [vdupq_n_u32((t1037) as u32); 2];
    let t1039: [uint32x4_t; 2] = [vandq_u32(x1[0], t1038[0]), vandq_u32(x1[1], t1038[1])];
    let t1040 = [vdupq_n_u32(8u32); 2];
    let t1041: [uint32x4_t; 2] = [vmulq_u32(t1039[0], t1040[0]), vmulq_u32(t1039[1], t1040[1])];
    x4 = t1041;
    let mut t1042: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x3[2]) as u64) << 32 | vgetq_lane_u32::<0>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1042[0] = vsetq_lane_u32::<0>(v as u32, t1042[0]); t1042[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1042[2]); }
    { let v = (((vgetq_lane_u32::<1>(x3[2]) as u64) << 32 | vgetq_lane_u32::<1>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1042[0] = vsetq_lane_u32::<1>(v as u32, t1042[0]); t1042[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1042[2]); }
    { let v = (((vgetq_lane_u32::<2>(x3[2]) as u64) << 32 | vgetq_lane_u32::<2>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1042[0] = vsetq_lane_u32::<2>(v as u32, t1042[0]); t1042[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1042[2]); }
    { let v = (((vgetq_lane_u32::<3>(x3[2]) as u64) << 32 | vgetq_lane_u32::<3>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1042[0] = vsetq_lane_u32::<3>(v as u32, t1042[0]); t1042[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1042[2]); }
    { let v = (((vgetq_lane_u32::<0>(x3[3]) as u64) << 32 | vgetq_lane_u32::<0>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1042[1] = vsetq_lane_u32::<0>(v as u32, t1042[1]); t1042[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1042[3]); }
    { let v = (((vgetq_lane_u32::<1>(x3[3]) as u64) << 32 | vgetq_lane_u32::<1>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1042[1] = vsetq_lane_u32::<1>(v as u32, t1042[1]); t1042[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1042[3]); }
    { let v = (((vgetq_lane_u32::<2>(x3[3]) as u64) << 32 | vgetq_lane_u32::<2>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1042[1] = vsetq_lane_u32::<2>(v as u32, t1042[1]); t1042[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1042[3]); }
    { let v = (((vgetq_lane_u32::<3>(x3[3]) as u64) << 32 | vgetq_lane_u32::<3>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1042[1] = vsetq_lane_u32::<3>(v as u32, t1042[1]); t1042[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1042[3]); }
    let t1043: [uint32x4_t; 2] = [x4[0], x4[1]];
    let t1044: [uint32x4_t; 2] = [vandq_u32(t1043[0], vdupq_n_u32(63)), vandq_u32(t1043[1], vdupq_n_u32(63))];
    let t1045: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t1042[0], vnegq_s32(vreinterpretq_s32_u32(t1044[0]))), vshlq_u32(t1042[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1044[0])))), vorrq_u32(vshlq_u32(t1042[1], vnegq_s32(vreinterpretq_s32_u32(t1044[1]))), vshlq_u32(t1042[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1044[1])))), vshlq_u32(t1042[2], vnegq_s32(vreinterpretq_s32_u32(t1044[0]))), vshlq_u32(t1042[3], vnegq_s32(vreinterpretq_s32_u32(t1044[1])))];
    x5 = t1045;
    let t1046 = [vdupq_n_u32(0u32); 2];
    let t1047: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t1046[0])), vmvnq_u32(vceqq_u32(x4[1], t1046[1]))];
    let t1048 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
    let t1049: [uint32x4_t; 2] = [vaddq_u32(x4[0], t1048[0]), vaddq_u32(x4[1], t1048[1])];
    let t1050 = [vdupq_n_u32(64u32); 2];
    let t1051: [uint32x4_t; 2] = [vcgtq_u32(t1049[0], t1050[0]), vcgtq_u32(t1049[1], t1050[1])];
    let t1052: [uint32x4_t; 2] = [vandq_u32(t1047[0], t1051[0]), vandq_u32(t1047[1], t1051[1])];
    let mut t1053: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1054: [uint32x4_t; 2] = [vmvnq_u32(t1052[0]), vmvnq_u32(t1052[1])];
    if any(&t1052) {
        let t1055: u64 = (1u64).wrapping_mul(8u64);
        let t1056 = [vdupq_n_u32((t1055) as u32); 2];
        let t1057 = [vdupq_n_u32(((t1055) >> 32) as u32); 2];
        let t1058: [uint32x4_t; 2] = [vaddq_u32(x3[0], t1056[0]), vaddq_u32(x3[1], t1056[1])];
        let t1059: [uint32x4_t; 2] = [vcltq_u32(t1058[0], x3[0]), vcltq_u32(t1058[1], x3[1])];
        let t1060: [uint32x4_t; 4] = [t1058[0], t1058[1], vsubq_u32(vaddq_u32(x3[2], t1057[0]), t1059[0]), vsubq_u32(vaddq_u32(x3[3], t1057[1]), t1059[1])];
        let mut t1061: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1060[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1060[0]) as u64) as usize as *const u64).read_unaligned(); t1061[0] = vsetq_lane_u32::<0>(v as u32, t1061[0]); t1061[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1061[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1060[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1060[0]) as u64) as usize as *const u64).read_unaligned(); t1061[0] = vsetq_lane_u32::<1>(v as u32, t1061[0]); t1061[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1061[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1060[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1060[0]) as u64) as usize as *const u64).read_unaligned(); t1061[0] = vsetq_lane_u32::<2>(v as u32, t1061[0]); t1061[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1061[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1060[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1060[0]) as u64) as usize as *const u64).read_unaligned(); t1061[0] = vsetq_lane_u32::<3>(v as u32, t1061[0]); t1061[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1061[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1060[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1060[1]) as u64) as usize as *const u64).read_unaligned(); t1061[1] = vsetq_lane_u32::<0>(v as u32, t1061[1]); t1061[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1061[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1060[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1060[1]) as u64) as usize as *const u64).read_unaligned(); t1061[1] = vsetq_lane_u32::<1>(v as u32, t1061[1]); t1061[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1061[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1060[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1060[1]) as u64) as usize as *const u64).read_unaligned(); t1061[1] = vsetq_lane_u32::<2>(v as u32, t1061[1]); t1061[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1061[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1060[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1060[1]) as u64) as usize as *const u64).read_unaligned(); t1061[1] = vsetq_lane_u32::<3>(v as u32, t1061[1]); t1061[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1061[3]); }
        let t1062: [uint32x4_t; 2] = [vsubq_u32(t1050[0], x4[0]), vsubq_u32(t1050[1], x4[1])];
        let t1063: [uint32x4_t; 2] = [t1062[0], t1062[1]];
        let t1064: [uint32x4_t; 2] = [vandq_u32(t1063[0], vdupq_n_u32(63)), vandq_u32(t1063[1], vdupq_n_u32(63))];
        let t1065: [uint32x4_t; 4] = [vshlq_u32(t1061[0], vreinterpretq_s32_u32(t1064[0])), vshlq_u32(t1061[1], vreinterpretq_s32_u32(t1064[1])), vorrq_u32(vshlq_u32(t1061[2], vreinterpretq_s32_u32(t1064[0])), vshlq_u32(t1061[0], vsubq_s32(vreinterpretq_s32_u32(t1064[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1061[3], vreinterpretq_s32_u32(t1064[1])), vshlq_u32(t1061[1], vsubq_s32(vreinterpretq_s32_u32(t1064[1]), vdupq_n_s32(32))))];
        let t1066: [uint32x4_t; 4] = [vorrq_u32(x5[0], t1065[0]), vorrq_u32(x5[1], t1065[1]), vorrq_u32(x5[2], t1065[2]), vorrq_u32(x5[3], t1065[3])];
        t1053 = [vbslq_u32(t1052[0], t1066[0], t1053[0]), vbslq_u32(t1052[1], t1066[1], t1053[1]), vbslq_u32(t1052[0], t1066[2], t1053[2]), vbslq_u32(t1052[1], t1066[3], t1053[3])];
    }
    {
        t1053 = [vbslq_u32(t1054[0], x5[0], t1053[0]), vbslq_u32(t1054[1], x5[1], t1053[1]), vbslq_u32(t1054[0], x5[2], t1053[2]), vbslq_u32(t1054[1], x5[3], t1053[3])];
    }
    res = t1053;
    return res;
}

#[inline(always)]
unsafe fn b19_r0al1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1b: *const u8, x1i: [uint32x4_t; 2], x2: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x3: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x4: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x5: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if ((x2) == (8u32)) {
        let t1067 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], x1i[0]), vandq_u32(live[1], x1i[1]))));
        let t1068: [uint32x4_t; 2] = [vbslq_u32(live[0], x1i[0], t1067), vbslq_u32(live[1], x1i[1], t1067)];
        let mut t1069: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1068[0]) as usize * 1) as *const u64).read_unaligned(); t1069[0] = vsetq_lane_u32::<0>(v as u32, t1069[0]); t1069[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1069[2]); }
        { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1068[0]) as usize * 1) as *const u64).read_unaligned(); t1069[0] = vsetq_lane_u32::<1>(v as u32, t1069[0]); t1069[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1069[2]); }
        { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1068[0]) as usize * 1) as *const u64).read_unaligned(); t1069[0] = vsetq_lane_u32::<2>(v as u32, t1069[0]); t1069[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1069[2]); }
        { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1068[0]) as usize * 1) as *const u64).read_unaligned(); t1069[0] = vsetq_lane_u32::<3>(v as u32, t1069[0]); t1069[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1069[2]); }
        { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1068[1]) as usize * 1) as *const u64).read_unaligned(); t1069[1] = vsetq_lane_u32::<0>(v as u32, t1069[1]); t1069[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1069[3]); }
        { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1068[1]) as usize * 1) as *const u64).read_unaligned(); t1069[1] = vsetq_lane_u32::<1>(v as u32, t1069[1]); t1069[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1069[3]); }
        { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1068[1]) as usize * 1) as *const u64).read_unaligned(); t1069[1] = vsetq_lane_u32::<2>(v as u32, t1069[1]); t1069[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1069[3]); }
        { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1068[1]) as usize * 1) as *const u64).read_unaligned(); t1069[1] = vsetq_lane_u32::<3>(v as u32, t1069[1]); t1069[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1069[3]); }
        res = t1069;
        return res;
    }
    let t1070: [uint32x4_t; 2] = [x1i[0], x1i[1]];
    let t1071: u64 = ((x1b) as usize as u64);
    let t1072 = [vdupq_n_u32((t1071) as u32); 2];
    let t1073 = [vdupq_n_u32(((t1071) >> 32) as u32); 2];
    let t1074: [uint32x4_t; 2] = [vaddq_u32(t1072[0], t1070[0]), vaddq_u32(t1072[1], t1070[1])];
    let t1075: [uint32x4_t; 2] = [vcltq_u32(t1074[0], t1072[0]), vcltq_u32(t1074[1], t1072[1])];
    let t1076: [uint32x4_t; 4] = [t1074[0], t1074[1], vsubq_u32(vaddq_u32(t1073[0], vdupq_n_u32(0)), t1075[0]), vsubq_u32(vaddq_u32(t1073[1], vdupq_n_u32(0)), t1075[1])];
    let t1077: u64 = !(7u64);
    let t1078 = [vdupq_n_u32((t1077) as u32); 2];
    let t1079 = [vdupq_n_u32(((t1077) >> 32) as u32); 2];
    let t1080: [uint32x4_t; 4] = [vandq_u32(t1076[0], t1078[0]), vandq_u32(t1076[1], t1078[1]), vandq_u32(t1076[2], t1079[0]), vandq_u32(t1076[3], t1079[1])];
    let t1081: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
    let t1082: [uint32x4_t; 4] = [vandq_u32(t1080[0], t1081[0]), vandq_u32(t1080[1], t1081[1]), vandq_u32(t1080[2], t1081[2]), vandq_u32(t1080[3], t1081[3])];
    let t1083: u64 = ((arg0) as usize as u64);
    let t1084 = [vdupq_n_u32((t1083) as u32); 2];
    let t1085 = [vdupq_n_u32(((t1083) >> 32) as u32); 2];
    let t1086: [uint32x4_t; 2] = [vaddq_u32(t1084[0], t1082[0]), vaddq_u32(t1084[1], t1082[1])];
    let t1087: [uint32x4_t; 2] = [vcltq_u32(t1086[0], t1084[0]), vcltq_u32(t1086[1], t1084[1])];
    let t1088: [uint32x4_t; 4] = [t1086[0], t1086[1], vsubq_u32(vaddq_u32(t1085[0], t1082[2]), t1087[0]), vsubq_u32(vaddq_u32(t1085[1], t1082[3]), t1087[1])];
    x3 = t1088;
    let t1089: [uint32x4_t; 2] = [x1i[0], x1i[1]];
    let t1090: u64 = ((x1b) as usize as u64);
    let t1091 = [vdupq_n_u32((t1090) as u32); 2];
    let t1092 = [vdupq_n_u32(((t1090) >> 32) as u32); 2];
    let t1093: [uint32x4_t; 2] = [vaddq_u32(t1091[0], t1089[0]), vaddq_u32(t1091[1], t1089[1])];
    let t1094: [uint32x4_t; 2] = [vcltq_u32(t1093[0], t1091[0]), vcltq_u32(t1093[1], t1091[1])];
    let t1095: [uint32x4_t; 4] = [t1093[0], t1093[1], vsubq_u32(vaddq_u32(t1092[0], vdupq_n_u32(0)), t1094[0]), vsubq_u32(vaddq_u32(t1092[1], vdupq_n_u32(0)), t1094[1])];
    let t1096: u64 = 7u64;
    let t1097 = [vdupq_n_u32((t1096) as u32); 2];
    let t1098: [uint32x4_t; 2] = [vandq_u32(t1095[0], t1097[0]), vandq_u32(t1095[1], t1097[1])];
    let t1099 = [vdupq_n_u32(8u32); 2];
    let t1100: [uint32x4_t; 2] = [vmulq_u32(t1098[0], t1099[0]), vmulq_u32(t1098[1], t1099[1])];
    x4 = t1100;
    let mut t1101: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x3[2]) as u64) << 32 | vgetq_lane_u32::<0>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1101[0] = vsetq_lane_u32::<0>(v as u32, t1101[0]); t1101[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1101[2]); }
    { let v = (((vgetq_lane_u32::<1>(x3[2]) as u64) << 32 | vgetq_lane_u32::<1>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1101[0] = vsetq_lane_u32::<1>(v as u32, t1101[0]); t1101[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1101[2]); }
    { let v = (((vgetq_lane_u32::<2>(x3[2]) as u64) << 32 | vgetq_lane_u32::<2>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1101[0] = vsetq_lane_u32::<2>(v as u32, t1101[0]); t1101[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1101[2]); }
    { let v = (((vgetq_lane_u32::<3>(x3[2]) as u64) << 32 | vgetq_lane_u32::<3>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1101[0] = vsetq_lane_u32::<3>(v as u32, t1101[0]); t1101[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1101[2]); }
    { let v = (((vgetq_lane_u32::<0>(x3[3]) as u64) << 32 | vgetq_lane_u32::<0>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1101[1] = vsetq_lane_u32::<0>(v as u32, t1101[1]); t1101[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1101[3]); }
    { let v = (((vgetq_lane_u32::<1>(x3[3]) as u64) << 32 | vgetq_lane_u32::<1>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1101[1] = vsetq_lane_u32::<1>(v as u32, t1101[1]); t1101[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1101[3]); }
    { let v = (((vgetq_lane_u32::<2>(x3[3]) as u64) << 32 | vgetq_lane_u32::<2>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1101[1] = vsetq_lane_u32::<2>(v as u32, t1101[1]); t1101[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1101[3]); }
    { let v = (((vgetq_lane_u32::<3>(x3[3]) as u64) << 32 | vgetq_lane_u32::<3>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1101[1] = vsetq_lane_u32::<3>(v as u32, t1101[1]); t1101[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1101[3]); }
    let t1102: [uint32x4_t; 2] = [x4[0], x4[1]];
    let t1103: [uint32x4_t; 2] = [vandq_u32(t1102[0], vdupq_n_u32(63)), vandq_u32(t1102[1], vdupq_n_u32(63))];
    let t1104: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t1101[0], vnegq_s32(vreinterpretq_s32_u32(t1103[0]))), vshlq_u32(t1101[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1103[0])))), vorrq_u32(vshlq_u32(t1101[1], vnegq_s32(vreinterpretq_s32_u32(t1103[1]))), vshlq_u32(t1101[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1103[1])))), vshlq_u32(t1101[2], vnegq_s32(vreinterpretq_s32_u32(t1103[0]))), vshlq_u32(t1101[3], vnegq_s32(vreinterpretq_s32_u32(t1103[1])))];
    x5 = t1104;
    let t1105 = [vdupq_n_u32(0u32); 2];
    let t1106: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t1105[0])), vmvnq_u32(vceqq_u32(x4[1], t1105[1]))];
    let t1107 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
    let t1108: [uint32x4_t; 2] = [vaddq_u32(x4[0], t1107[0]), vaddq_u32(x4[1], t1107[1])];
    let t1109 = [vdupq_n_u32(64u32); 2];
    let t1110: [uint32x4_t; 2] = [vcgtq_u32(t1108[0], t1109[0]), vcgtq_u32(t1108[1], t1109[1])];
    let t1111: [uint32x4_t; 2] = [vandq_u32(t1106[0], t1110[0]), vandq_u32(t1106[1], t1110[1])];
    let mut t1112: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1113: [uint32x4_t; 2] = [vandq_u32(live[0], t1111[0]), vandq_u32(live[1], t1111[1])];
    let t1114: [uint32x4_t; 2] = [vbicq_u32(live[0], t1111[0]), vbicq_u32(live[1], t1111[1])];
    if any(&t1113) {
        let t1115: u64 = (1u64).wrapping_mul(8u64);
        let t1116 = [vdupq_n_u32((t1115) as u32); 2];
        let t1117 = [vdupq_n_u32(((t1115) >> 32) as u32); 2];
        let t1118: [uint32x4_t; 2] = [vaddq_u32(x3[0], t1116[0]), vaddq_u32(x3[1], t1116[1])];
        let t1119: [uint32x4_t; 2] = [vcltq_u32(t1118[0], x3[0]), vcltq_u32(t1118[1], x3[1])];
        let t1120: [uint32x4_t; 4] = [t1118[0], t1118[1], vsubq_u32(vaddq_u32(x3[2], t1117[0]), t1119[0]), vsubq_u32(vaddq_u32(x3[3], t1117[1]), t1119[1])];
        let mut t1121: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1120[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1120[0]) as u64) as usize as *const u64).read_unaligned(); t1121[0] = vsetq_lane_u32::<0>(v as u32, t1121[0]); t1121[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1121[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1120[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1120[0]) as u64) as usize as *const u64).read_unaligned(); t1121[0] = vsetq_lane_u32::<1>(v as u32, t1121[0]); t1121[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1121[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1120[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1120[0]) as u64) as usize as *const u64).read_unaligned(); t1121[0] = vsetq_lane_u32::<2>(v as u32, t1121[0]); t1121[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1121[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1120[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1120[0]) as u64) as usize as *const u64).read_unaligned(); t1121[0] = vsetq_lane_u32::<3>(v as u32, t1121[0]); t1121[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1121[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1120[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1120[1]) as u64) as usize as *const u64).read_unaligned(); t1121[1] = vsetq_lane_u32::<0>(v as u32, t1121[1]); t1121[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1121[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1120[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1120[1]) as u64) as usize as *const u64).read_unaligned(); t1121[1] = vsetq_lane_u32::<1>(v as u32, t1121[1]); t1121[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1121[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1120[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1120[1]) as u64) as usize as *const u64).read_unaligned(); t1121[1] = vsetq_lane_u32::<2>(v as u32, t1121[1]); t1121[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1121[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1120[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1120[1]) as u64) as usize as *const u64).read_unaligned(); t1121[1] = vsetq_lane_u32::<3>(v as u32, t1121[1]); t1121[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1121[3]); }
        let t1122: [uint32x4_t; 2] = [vsubq_u32(t1109[0], x4[0]), vsubq_u32(t1109[1], x4[1])];
        let t1123: [uint32x4_t; 2] = [t1122[0], t1122[1]];
        let t1124: [uint32x4_t; 2] = [vandq_u32(t1123[0], vdupq_n_u32(63)), vandq_u32(t1123[1], vdupq_n_u32(63))];
        let t1125: [uint32x4_t; 4] = [vshlq_u32(t1121[0], vreinterpretq_s32_u32(t1124[0])), vshlq_u32(t1121[1], vreinterpretq_s32_u32(t1124[1])), vorrq_u32(vshlq_u32(t1121[2], vreinterpretq_s32_u32(t1124[0])), vshlq_u32(t1121[0], vsubq_s32(vreinterpretq_s32_u32(t1124[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1121[3], vreinterpretq_s32_u32(t1124[1])), vshlq_u32(t1121[1], vsubq_s32(vreinterpretq_s32_u32(t1124[1]), vdupq_n_s32(32))))];
        let t1126: [uint32x4_t; 4] = [vorrq_u32(x5[0], t1125[0]), vorrq_u32(x5[1], t1125[1]), vorrq_u32(x5[2], t1125[2]), vorrq_u32(x5[3], t1125[3])];
        t1112 = [vbslq_u32(t1113[0], t1126[0], t1112[0]), vbslq_u32(t1113[1], t1126[1], t1112[1]), vbslq_u32(t1113[0], t1126[2], t1112[2]), vbslq_u32(t1113[1], t1126[3], t1112[3])];
    }
    {
        t1112 = [vbslq_u32(t1114[0], x5[0], t1112[0]), vbslq_u32(t1114[1], x5[1], t1112[1]), vbslq_u32(t1114[0], x5[2], t1112[2]), vbslq_u32(t1114[1], x5[3], t1112[3])];
    }
    res = t1112;
    return res;
}

#[inline(always)]
unsafe fn b19_r0an1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1b: *const u8, x1if: u32, x1is: u32, x2: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x3: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x4: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x5: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if ((x2) == (8u32)) {
        let t1127: [uint32x4_t; 4];
        if (x1is) == 8u32 && ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 && all(&live) {
            let t1128 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
            let t11290 = vld2q_u32((t1128 as *const u32).add(0));
            let t11291 = vld2q_u32((t1128 as *const u32).add(8));
            let t1129: [uint32x4_t; 4] = [t11290.0, t11291.0, t11290.1, t11291.1];
            t1127 = t1129;
        } else {
            let t1130: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
            let t1131 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t1130[0]), vandq_u32(live[1], t1130[1]))));
            let t1132: [uint32x4_t; 2] = [vbslq_u32(live[0], t1130[0], t1131), vbslq_u32(live[1], t1130[1], t1131)];
            let mut t1133: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1132[0]) as usize * 1) as *const u64).read_unaligned(); t1133[0] = vsetq_lane_u32::<0>(v as u32, t1133[0]); t1133[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1133[2]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1132[0]) as usize * 1) as *const u64).read_unaligned(); t1133[0] = vsetq_lane_u32::<1>(v as u32, t1133[0]); t1133[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1133[2]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1132[0]) as usize * 1) as *const u64).read_unaligned(); t1133[0] = vsetq_lane_u32::<2>(v as u32, t1133[0]); t1133[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1133[2]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1132[0]) as usize * 1) as *const u64).read_unaligned(); t1133[0] = vsetq_lane_u32::<3>(v as u32, t1133[0]); t1133[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1133[2]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1132[1]) as usize * 1) as *const u64).read_unaligned(); t1133[1] = vsetq_lane_u32::<0>(v as u32, t1133[1]); t1133[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1133[3]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1132[1]) as usize * 1) as *const u64).read_unaligned(); t1133[1] = vsetq_lane_u32::<1>(v as u32, t1133[1]); t1133[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1133[3]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1132[1]) as usize * 1) as *const u64).read_unaligned(); t1133[1] = vsetq_lane_u32::<2>(v as u32, t1133[1]); t1133[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1133[3]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1132[1]) as usize * 1) as *const u64).read_unaligned(); t1133[1] = vsetq_lane_u32::<3>(v as u32, t1133[1]); t1133[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1133[3]); }
            t1127 = t1133;
        }
        res = t1127;
        return res;
    }
    let t1134: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
    let t1135: [uint32x4_t; 2] = [t1134[0], t1134[1]];
    let t1136: u64 = ((x1b) as usize as u64);
    let t1137 = [vdupq_n_u32((t1136) as u32); 2];
    let t1138 = [vdupq_n_u32(((t1136) >> 32) as u32); 2];
    let t1139: [uint32x4_t; 2] = [vaddq_u32(t1137[0], t1135[0]), vaddq_u32(t1137[1], t1135[1])];
    let t1140: [uint32x4_t; 2] = [vcltq_u32(t1139[0], t1137[0]), vcltq_u32(t1139[1], t1137[1])];
    let t1141: [uint32x4_t; 4] = [t1139[0], t1139[1], vsubq_u32(vaddq_u32(t1138[0], vdupq_n_u32(0)), t1140[0]), vsubq_u32(vaddq_u32(t1138[1], vdupq_n_u32(0)), t1140[1])];
    let t1142: u64 = !(7u64);
    let t1143 = [vdupq_n_u32((t1142) as u32); 2];
    let t1144 = [vdupq_n_u32(((t1142) >> 32) as u32); 2];
    let t1145: [uint32x4_t; 4] = [vandq_u32(t1141[0], t1143[0]), vandq_u32(t1141[1], t1143[1]), vandq_u32(t1141[2], t1144[0]), vandq_u32(t1141[3], t1144[1])];
    let t1146: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
    let t1147: [uint32x4_t; 4] = [vandq_u32(t1145[0], t1146[0]), vandq_u32(t1145[1], t1146[1]), vandq_u32(t1145[2], t1146[2]), vandq_u32(t1145[3], t1146[3])];
    let t1148: u64 = ((arg0) as usize as u64);
    let t1149 = [vdupq_n_u32((t1148) as u32); 2];
    let t1150 = [vdupq_n_u32(((t1148) >> 32) as u32); 2];
    let t1151: [uint32x4_t; 2] = [vaddq_u32(t1149[0], t1147[0]), vaddq_u32(t1149[1], t1147[1])];
    let t1152: [uint32x4_t; 2] = [vcltq_u32(t1151[0], t1149[0]), vcltq_u32(t1151[1], t1149[1])];
    let t1153: [uint32x4_t; 4] = [t1151[0], t1151[1], vsubq_u32(vaddq_u32(t1150[0], t1147[2]), t1152[0]), vsubq_u32(vaddq_u32(t1150[1], t1147[3]), t1152[1])];
    x3 = t1153;
    let t1154: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
    let t1155: [uint32x4_t; 2] = [t1154[0], t1154[1]];
    let t1156: u64 = ((x1b) as usize as u64);
    let t1157 = [vdupq_n_u32((t1156) as u32); 2];
    let t1158 = [vdupq_n_u32(((t1156) >> 32) as u32); 2];
    let t1159: [uint32x4_t; 2] = [vaddq_u32(t1157[0], t1155[0]), vaddq_u32(t1157[1], t1155[1])];
    let t1160: [uint32x4_t; 2] = [vcltq_u32(t1159[0], t1157[0]), vcltq_u32(t1159[1], t1157[1])];
    let t1161: [uint32x4_t; 4] = [t1159[0], t1159[1], vsubq_u32(vaddq_u32(t1158[0], vdupq_n_u32(0)), t1160[0]), vsubq_u32(vaddq_u32(t1158[1], vdupq_n_u32(0)), t1160[1])];
    let t1162: u64 = 7u64;
    let t1163 = [vdupq_n_u32((t1162) as u32); 2];
    let t1164: [uint32x4_t; 2] = [vandq_u32(t1161[0], t1163[0]), vandq_u32(t1161[1], t1163[1])];
    let t1165 = [vdupq_n_u32(8u32); 2];
    let t1166: [uint32x4_t; 2] = [vmulq_u32(t1164[0], t1165[0]), vmulq_u32(t1164[1], t1165[1])];
    x4 = t1166;
    let mut t1167: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x3[2]) as u64) << 32 | vgetq_lane_u32::<0>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1167[0] = vsetq_lane_u32::<0>(v as u32, t1167[0]); t1167[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1167[2]); }
    { let v = (((vgetq_lane_u32::<1>(x3[2]) as u64) << 32 | vgetq_lane_u32::<1>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1167[0] = vsetq_lane_u32::<1>(v as u32, t1167[0]); t1167[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1167[2]); }
    { let v = (((vgetq_lane_u32::<2>(x3[2]) as u64) << 32 | vgetq_lane_u32::<2>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1167[0] = vsetq_lane_u32::<2>(v as u32, t1167[0]); t1167[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1167[2]); }
    { let v = (((vgetq_lane_u32::<3>(x3[2]) as u64) << 32 | vgetq_lane_u32::<3>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1167[0] = vsetq_lane_u32::<3>(v as u32, t1167[0]); t1167[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1167[2]); }
    { let v = (((vgetq_lane_u32::<0>(x3[3]) as u64) << 32 | vgetq_lane_u32::<0>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1167[1] = vsetq_lane_u32::<0>(v as u32, t1167[1]); t1167[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1167[3]); }
    { let v = (((vgetq_lane_u32::<1>(x3[3]) as u64) << 32 | vgetq_lane_u32::<1>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1167[1] = vsetq_lane_u32::<1>(v as u32, t1167[1]); t1167[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1167[3]); }
    { let v = (((vgetq_lane_u32::<2>(x3[3]) as u64) << 32 | vgetq_lane_u32::<2>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1167[1] = vsetq_lane_u32::<2>(v as u32, t1167[1]); t1167[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1167[3]); }
    { let v = (((vgetq_lane_u32::<3>(x3[3]) as u64) << 32 | vgetq_lane_u32::<3>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1167[1] = vsetq_lane_u32::<3>(v as u32, t1167[1]); t1167[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1167[3]); }
    let t1168: [uint32x4_t; 2] = [x4[0], x4[1]];
    let t1169: [uint32x4_t; 2] = [vandq_u32(t1168[0], vdupq_n_u32(63)), vandq_u32(t1168[1], vdupq_n_u32(63))];
    let t1170: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t1167[0], vnegq_s32(vreinterpretq_s32_u32(t1169[0]))), vshlq_u32(t1167[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1169[0])))), vorrq_u32(vshlq_u32(t1167[1], vnegq_s32(vreinterpretq_s32_u32(t1169[1]))), vshlq_u32(t1167[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1169[1])))), vshlq_u32(t1167[2], vnegq_s32(vreinterpretq_s32_u32(t1169[0]))), vshlq_u32(t1167[3], vnegq_s32(vreinterpretq_s32_u32(t1169[1])))];
    x5 = t1170;
    let t1171 = [vdupq_n_u32(0u32); 2];
    let t1172: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t1171[0])), vmvnq_u32(vceqq_u32(x4[1], t1171[1]))];
    let t1173 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
    let t1174: [uint32x4_t; 2] = [vaddq_u32(x4[0], t1173[0]), vaddq_u32(x4[1], t1173[1])];
    let t1175 = [vdupq_n_u32(64u32); 2];
    let t1176: [uint32x4_t; 2] = [vcgtq_u32(t1174[0], t1175[0]), vcgtq_u32(t1174[1], t1175[1])];
    let t1177: [uint32x4_t; 2] = [vandq_u32(t1172[0], t1176[0]), vandq_u32(t1172[1], t1176[1])];
    let mut t1178: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1179: [uint32x4_t; 2] = [vandq_u32(live[0], t1177[0]), vandq_u32(live[1], t1177[1])];
    let t1180: [uint32x4_t; 2] = [vbicq_u32(live[0], t1177[0]), vbicq_u32(live[1], t1177[1])];
    if any(&t1179) {
        let t1181: u64 = (1u64).wrapping_mul(8u64);
        let t1182 = [vdupq_n_u32((t1181) as u32); 2];
        let t1183 = [vdupq_n_u32(((t1181) >> 32) as u32); 2];
        let t1184: [uint32x4_t; 2] = [vaddq_u32(x3[0], t1182[0]), vaddq_u32(x3[1], t1182[1])];
        let t1185: [uint32x4_t; 2] = [vcltq_u32(t1184[0], x3[0]), vcltq_u32(t1184[1], x3[1])];
        let t1186: [uint32x4_t; 4] = [t1184[0], t1184[1], vsubq_u32(vaddq_u32(x3[2], t1183[0]), t1185[0]), vsubq_u32(vaddq_u32(x3[3], t1183[1]), t1185[1])];
        let mut t1187: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1186[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1186[0]) as u64) as usize as *const u64).read_unaligned(); t1187[0] = vsetq_lane_u32::<0>(v as u32, t1187[0]); t1187[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1187[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1186[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1186[0]) as u64) as usize as *const u64).read_unaligned(); t1187[0] = vsetq_lane_u32::<1>(v as u32, t1187[0]); t1187[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1187[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1186[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1186[0]) as u64) as usize as *const u64).read_unaligned(); t1187[0] = vsetq_lane_u32::<2>(v as u32, t1187[0]); t1187[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1187[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1186[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1186[0]) as u64) as usize as *const u64).read_unaligned(); t1187[0] = vsetq_lane_u32::<3>(v as u32, t1187[0]); t1187[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1187[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1186[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1186[1]) as u64) as usize as *const u64).read_unaligned(); t1187[1] = vsetq_lane_u32::<0>(v as u32, t1187[1]); t1187[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1187[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1186[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1186[1]) as u64) as usize as *const u64).read_unaligned(); t1187[1] = vsetq_lane_u32::<1>(v as u32, t1187[1]); t1187[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1187[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1186[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1186[1]) as u64) as usize as *const u64).read_unaligned(); t1187[1] = vsetq_lane_u32::<2>(v as u32, t1187[1]); t1187[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1187[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1186[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1186[1]) as u64) as usize as *const u64).read_unaligned(); t1187[1] = vsetq_lane_u32::<3>(v as u32, t1187[1]); t1187[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1187[3]); }
        let t1188: [uint32x4_t; 2] = [vsubq_u32(t1175[0], x4[0]), vsubq_u32(t1175[1], x4[1])];
        let t1189: [uint32x4_t; 2] = [t1188[0], t1188[1]];
        let t1190: [uint32x4_t; 2] = [vandq_u32(t1189[0], vdupq_n_u32(63)), vandq_u32(t1189[1], vdupq_n_u32(63))];
        let t1191: [uint32x4_t; 4] = [vshlq_u32(t1187[0], vreinterpretq_s32_u32(t1190[0])), vshlq_u32(t1187[1], vreinterpretq_s32_u32(t1190[1])), vorrq_u32(vshlq_u32(t1187[2], vreinterpretq_s32_u32(t1190[0])), vshlq_u32(t1187[0], vsubq_s32(vreinterpretq_s32_u32(t1190[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1187[3], vreinterpretq_s32_u32(t1190[1])), vshlq_u32(t1187[1], vsubq_s32(vreinterpretq_s32_u32(t1190[1]), vdupq_n_s32(32))))];
        let t1192: [uint32x4_t; 4] = [vorrq_u32(x5[0], t1191[0]), vorrq_u32(x5[1], t1191[1]), vorrq_u32(x5[2], t1191[2]), vorrq_u32(x5[3], t1191[3])];
        t1178 = [vbslq_u32(t1179[0], t1192[0], t1178[0]), vbslq_u32(t1179[1], t1192[1], t1178[1]), vbslq_u32(t1179[0], t1192[2], t1178[2]), vbslq_u32(t1179[1], t1192[3], t1178[3])];
    }
    {
        t1178 = [vbslq_u32(t1180[0], x5[0], t1178[0]), vbslq_u32(t1180[1], x5[1], t1178[1]), vbslq_u32(t1180[0], x5[2], t1178[2]), vbslq_u32(t1180[1], x5[3], t1178[3])];
    }
    res = t1178;
    return res;
}

#[inline(always)]
unsafe fn b19_r0an1s_f(l: &Launch, lid0: u32, arg0: *const u8, x1b: *const u8, x1if: u32, x1is: u32, x2: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x3: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x4: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x5: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if ((x2) == (8u32)) {
        let t1193: [uint32x4_t; 4];
        if (x1is) == 8u32 && ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 {
            let t1194 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
            let t11950 = vld2q_u32((t1194 as *const u32).add(0));
            let t11951 = vld2q_u32((t1194 as *const u32).add(8));
            let t1195: [uint32x4_t; 4] = [t11950.0, t11951.0, t11950.1, t11951.1];
            t1193 = t1195;
        } else {
            let t1196: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
            let mut t1197: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1196[0]) as usize * 1) as *const u64).read_unaligned(); t1197[0] = vsetq_lane_u32::<0>(v as u32, t1197[0]); t1197[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1197[2]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1196[0]) as usize * 1) as *const u64).read_unaligned(); t1197[0] = vsetq_lane_u32::<1>(v as u32, t1197[0]); t1197[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1197[2]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1196[0]) as usize * 1) as *const u64).read_unaligned(); t1197[0] = vsetq_lane_u32::<2>(v as u32, t1197[0]); t1197[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1197[2]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1196[0]) as usize * 1) as *const u64).read_unaligned(); t1197[0] = vsetq_lane_u32::<3>(v as u32, t1197[0]); t1197[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1197[2]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1196[1]) as usize * 1) as *const u64).read_unaligned(); t1197[1] = vsetq_lane_u32::<0>(v as u32, t1197[1]); t1197[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1197[3]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1196[1]) as usize * 1) as *const u64).read_unaligned(); t1197[1] = vsetq_lane_u32::<1>(v as u32, t1197[1]); t1197[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1197[3]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1196[1]) as usize * 1) as *const u64).read_unaligned(); t1197[1] = vsetq_lane_u32::<2>(v as u32, t1197[1]); t1197[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1197[3]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1196[1]) as usize * 1) as *const u64).read_unaligned(); t1197[1] = vsetq_lane_u32::<3>(v as u32, t1197[1]); t1197[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1197[3]); }
            t1193 = t1197;
        }
        res = t1193;
        return res;
    }
    let t1198: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
    let t1199: [uint32x4_t; 2] = [t1198[0], t1198[1]];
    let t1200: u64 = ((x1b) as usize as u64);
    let t1201 = [vdupq_n_u32((t1200) as u32); 2];
    let t1202 = [vdupq_n_u32(((t1200) >> 32) as u32); 2];
    let t1203: [uint32x4_t; 2] = [vaddq_u32(t1201[0], t1199[0]), vaddq_u32(t1201[1], t1199[1])];
    let t1204: [uint32x4_t; 2] = [vcltq_u32(t1203[0], t1201[0]), vcltq_u32(t1203[1], t1201[1])];
    let t1205: [uint32x4_t; 4] = [t1203[0], t1203[1], vsubq_u32(vaddq_u32(t1202[0], vdupq_n_u32(0)), t1204[0]), vsubq_u32(vaddq_u32(t1202[1], vdupq_n_u32(0)), t1204[1])];
    let t1206: u64 = !(7u64);
    let t1207 = [vdupq_n_u32((t1206) as u32); 2];
    let t1208 = [vdupq_n_u32(((t1206) >> 32) as u32); 2];
    let t1209: [uint32x4_t; 4] = [vandq_u32(t1205[0], t1207[0]), vandq_u32(t1205[1], t1207[1]), vandq_u32(t1205[2], t1208[0]), vandq_u32(t1205[3], t1208[1])];
    let t1210: u64 = ((arg0) as usize as u64);
    let t1211 = [vdupq_n_u32((t1210) as u32); 2];
    let t1212 = [vdupq_n_u32(((t1210) >> 32) as u32); 2];
    let t1213: [uint32x4_t; 2] = [vaddq_u32(t1211[0], t1209[0]), vaddq_u32(t1211[1], t1209[1])];
    let t1214: [uint32x4_t; 2] = [vcltq_u32(t1213[0], t1211[0]), vcltq_u32(t1213[1], t1211[1])];
    let t1215: [uint32x4_t; 4] = [t1213[0], t1213[1], vsubq_u32(vaddq_u32(t1212[0], t1209[2]), t1214[0]), vsubq_u32(vaddq_u32(t1212[1], t1209[3]), t1214[1])];
    x3 = t1215;
    let t1216: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
    let t1217: [uint32x4_t; 2] = [t1216[0], t1216[1]];
    let t1218: u64 = ((x1b) as usize as u64);
    let t1219 = [vdupq_n_u32((t1218) as u32); 2];
    let t1220 = [vdupq_n_u32(((t1218) >> 32) as u32); 2];
    let t1221: [uint32x4_t; 2] = [vaddq_u32(t1219[0], t1217[0]), vaddq_u32(t1219[1], t1217[1])];
    let t1222: [uint32x4_t; 2] = [vcltq_u32(t1221[0], t1219[0]), vcltq_u32(t1221[1], t1219[1])];
    let t1223: [uint32x4_t; 4] = [t1221[0], t1221[1], vsubq_u32(vaddq_u32(t1220[0], vdupq_n_u32(0)), t1222[0]), vsubq_u32(vaddq_u32(t1220[1], vdupq_n_u32(0)), t1222[1])];
    let t1224: u64 = 7u64;
    let t1225 = [vdupq_n_u32((t1224) as u32); 2];
    let t1226: [uint32x4_t; 2] = [vandq_u32(t1223[0], t1225[0]), vandq_u32(t1223[1], t1225[1])];
    let t1227 = [vdupq_n_u32(8u32); 2];
    let t1228: [uint32x4_t; 2] = [vmulq_u32(t1226[0], t1227[0]), vmulq_u32(t1226[1], t1227[1])];
    x4 = t1228;
    let mut t1229: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x3[2]) as u64) << 32 | vgetq_lane_u32::<0>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1229[0] = vsetq_lane_u32::<0>(v as u32, t1229[0]); t1229[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1229[2]); }
    { let v = (((vgetq_lane_u32::<1>(x3[2]) as u64) << 32 | vgetq_lane_u32::<1>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1229[0] = vsetq_lane_u32::<1>(v as u32, t1229[0]); t1229[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1229[2]); }
    { let v = (((vgetq_lane_u32::<2>(x3[2]) as u64) << 32 | vgetq_lane_u32::<2>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1229[0] = vsetq_lane_u32::<2>(v as u32, t1229[0]); t1229[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1229[2]); }
    { let v = (((vgetq_lane_u32::<3>(x3[2]) as u64) << 32 | vgetq_lane_u32::<3>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1229[0] = vsetq_lane_u32::<3>(v as u32, t1229[0]); t1229[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1229[2]); }
    { let v = (((vgetq_lane_u32::<0>(x3[3]) as u64) << 32 | vgetq_lane_u32::<0>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1229[1] = vsetq_lane_u32::<0>(v as u32, t1229[1]); t1229[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1229[3]); }
    { let v = (((vgetq_lane_u32::<1>(x3[3]) as u64) << 32 | vgetq_lane_u32::<1>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1229[1] = vsetq_lane_u32::<1>(v as u32, t1229[1]); t1229[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1229[3]); }
    { let v = (((vgetq_lane_u32::<2>(x3[3]) as u64) << 32 | vgetq_lane_u32::<2>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1229[1] = vsetq_lane_u32::<2>(v as u32, t1229[1]); t1229[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1229[3]); }
    { let v = (((vgetq_lane_u32::<3>(x3[3]) as u64) << 32 | vgetq_lane_u32::<3>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1229[1] = vsetq_lane_u32::<3>(v as u32, t1229[1]); t1229[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1229[3]); }
    let t1230: [uint32x4_t; 2] = [x4[0], x4[1]];
    let t1231: [uint32x4_t; 2] = [vandq_u32(t1230[0], vdupq_n_u32(63)), vandq_u32(t1230[1], vdupq_n_u32(63))];
    let t1232: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t1229[0], vnegq_s32(vreinterpretq_s32_u32(t1231[0]))), vshlq_u32(t1229[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1231[0])))), vorrq_u32(vshlq_u32(t1229[1], vnegq_s32(vreinterpretq_s32_u32(t1231[1]))), vshlq_u32(t1229[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1231[1])))), vshlq_u32(t1229[2], vnegq_s32(vreinterpretq_s32_u32(t1231[0]))), vshlq_u32(t1229[3], vnegq_s32(vreinterpretq_s32_u32(t1231[1])))];
    x5 = t1232;
    let t1233 = [vdupq_n_u32(0u32); 2];
    let t1234: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t1233[0])), vmvnq_u32(vceqq_u32(x4[1], t1233[1]))];
    let t1235 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
    let t1236: [uint32x4_t; 2] = [vaddq_u32(x4[0], t1235[0]), vaddq_u32(x4[1], t1235[1])];
    let t1237 = [vdupq_n_u32(64u32); 2];
    let t1238: [uint32x4_t; 2] = [vcgtq_u32(t1236[0], t1237[0]), vcgtq_u32(t1236[1], t1237[1])];
    let t1239: [uint32x4_t; 2] = [vandq_u32(t1234[0], t1238[0]), vandq_u32(t1234[1], t1238[1])];
    let mut t1240: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1241: [uint32x4_t; 2] = [vmvnq_u32(t1239[0]), vmvnq_u32(t1239[1])];
    if any(&t1239) {
        let t1242: u64 = (1u64).wrapping_mul(8u64);
        let t1243 = [vdupq_n_u32((t1242) as u32); 2];
        let t1244 = [vdupq_n_u32(((t1242) >> 32) as u32); 2];
        let t1245: [uint32x4_t; 2] = [vaddq_u32(x3[0], t1243[0]), vaddq_u32(x3[1], t1243[1])];
        let t1246: [uint32x4_t; 2] = [vcltq_u32(t1245[0], x3[0]), vcltq_u32(t1245[1], x3[1])];
        let t1247: [uint32x4_t; 4] = [t1245[0], t1245[1], vsubq_u32(vaddq_u32(x3[2], t1244[0]), t1246[0]), vsubq_u32(vaddq_u32(x3[3], t1244[1]), t1246[1])];
        let mut t1248: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1247[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1247[0]) as u64) as usize as *const u64).read_unaligned(); t1248[0] = vsetq_lane_u32::<0>(v as u32, t1248[0]); t1248[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1248[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1247[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1247[0]) as u64) as usize as *const u64).read_unaligned(); t1248[0] = vsetq_lane_u32::<1>(v as u32, t1248[0]); t1248[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1248[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1247[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1247[0]) as u64) as usize as *const u64).read_unaligned(); t1248[0] = vsetq_lane_u32::<2>(v as u32, t1248[0]); t1248[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1248[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1247[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1247[0]) as u64) as usize as *const u64).read_unaligned(); t1248[0] = vsetq_lane_u32::<3>(v as u32, t1248[0]); t1248[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1248[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1247[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1247[1]) as u64) as usize as *const u64).read_unaligned(); t1248[1] = vsetq_lane_u32::<0>(v as u32, t1248[1]); t1248[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1248[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1247[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1247[1]) as u64) as usize as *const u64).read_unaligned(); t1248[1] = vsetq_lane_u32::<1>(v as u32, t1248[1]); t1248[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1248[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1247[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1247[1]) as u64) as usize as *const u64).read_unaligned(); t1248[1] = vsetq_lane_u32::<2>(v as u32, t1248[1]); t1248[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1248[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1247[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1247[1]) as u64) as usize as *const u64).read_unaligned(); t1248[1] = vsetq_lane_u32::<3>(v as u32, t1248[1]); t1248[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1248[3]); }
        let t1249: [uint32x4_t; 2] = [vsubq_u32(t1237[0], x4[0]), vsubq_u32(t1237[1], x4[1])];
        let t1250: [uint32x4_t; 2] = [t1249[0], t1249[1]];
        let t1251: [uint32x4_t; 2] = [vandq_u32(t1250[0], vdupq_n_u32(63)), vandq_u32(t1250[1], vdupq_n_u32(63))];
        let t1252: [uint32x4_t; 4] = [vshlq_u32(t1248[0], vreinterpretq_s32_u32(t1251[0])), vshlq_u32(t1248[1], vreinterpretq_s32_u32(t1251[1])), vorrq_u32(vshlq_u32(t1248[2], vreinterpretq_s32_u32(t1251[0])), vshlq_u32(t1248[0], vsubq_s32(vreinterpretq_s32_u32(t1251[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1248[3], vreinterpretq_s32_u32(t1251[1])), vshlq_u32(t1248[1], vsubq_s32(vreinterpretq_s32_u32(t1251[1]), vdupq_n_s32(32))))];
        let t1253: [uint32x4_t; 4] = [vorrq_u32(x5[0], t1252[0]), vorrq_u32(x5[1], t1252[1]), vorrq_u32(x5[2], t1252[2]), vorrq_u32(x5[3], t1252[3])];
        t1240 = [vbslq_u32(t1239[0], t1253[0], t1240[0]), vbslq_u32(t1239[1], t1253[1], t1240[1]), vbslq_u32(t1239[0], t1253[2], t1240[2]), vbslq_u32(t1239[1], t1253[3], t1240[3])];
    }
    {
        t1240 = [vbslq_u32(t1241[0], x5[0], t1240[0]), vbslq_u32(t1241[1], x5[1], t1240[1]), vbslq_u32(t1241[0], x5[2], t1240[2]), vbslq_u32(t1241[1], x5[3], t1240[3])];
    }
    res = t1240;
    return res;
}

#[inline(always)]
unsafe fn b20_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t1254: u64;
    if ((x0) >= (64u32)) {
        t1254 = !(0u64);
    } else {
        t1254 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t1254;
    return res;
}

#[inline(always)]
unsafe fn b20_s_f(l: &Launch, lid0: u32, x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t1255: u64;
    if ((x0) >= (64u32)) {
        t1255 = !(0u64);
    } else {
        t1255 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t1255;
    return res;
}

#[inline(always)]
unsafe fn b21_r0sss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1: u32, x2: u32, x3: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u64 = 0;
    let mut live = k;
    let mut res: u64 = 0;
    let t1256 = b19_r0ss_p(l, lid0, live, arg0, (((x1) as u64)).wrapping_add(((x2) as u64)), x3);
    x4 = t1256;
    let t1257: u64;
    if ((x3) >= (8u32)) {
        t1257 = x4;
    } else {
        let t1258 = b20_s_p(l, lid0, live, (8u32).wrapping_mul(x3));
        t1257 = ((x4) & (t1258));
    }
    res = t1257;
    return res;
}

#[inline(always)]
unsafe fn b21_r0sss_f(l: &Launch, lid0: u32, arg0: *const u8, x1: u32, x2: u32, x3: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u64 = 0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t1259 = b19_r0ss_f(l, lid0, arg0, (((x1) as u64)).wrapping_add(((x2) as u64)), x3);
    x4 = t1259;
    let t1260: u64;
    if ((x3) >= (8u32)) {
        t1260 = x4;
    } else {
        let t1261 = b20_s_f(l, lid0, (8u32).wrapping_mul(x3));
        t1260 = ((x4) & (t1261));
    }
    res = t1260;
    return res;
}

#[inline(always)]
unsafe fn b21_r0sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1: u32, x2: [uint32x4_t; 2], x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1262 = b19_r0al1s_p(l, lid0, live, arg0, ((((x1) as u64)) as usize as *const u8), x2, x3);
    x4 = t1262;
    let t1263: [uint32x4_t; 4];
    if ((x3) >= (8u32)) {
        t1263 = x4;
    } else {
        let t1264 = b20_s_p(l, lid0, live, (8u32).wrapping_mul(x3));
        let t1265: u64 = t1264;
        let t1266 = [vdupq_n_u32((t1265) as u32); 2];
        let t1267 = [vdupq_n_u32(((t1265) >> 32) as u32); 2];
        let t1268: [uint32x4_t; 4] = [vandq_u32(x4[0], t1266[0]), vandq_u32(x4[1], t1266[1]), vandq_u32(x4[2], t1267[0]), vandq_u32(x4[3], t1267[1])];
        t1263 = t1268;
    }
    res = t1263;
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
    let x3: *const u8 = *argv.add(3);
    let arg4: *const u8 = (*argv.add(4) as *const *const u8).read_unaligned();
    let x5: u32 = (*argv.add(5) as *const u32).read_unaligned();
    let mut tg = [0u64; 128];
    let arg6: *const u8 = tg.as_mut_ptr() as *const u8;
    let mut t1269: bool = !(arg4).is_null();
    if t1269 {
        let mut t1270: bool = ((l.group[0]) >= ((arg4.wrapping_add((((x5) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        if !t1270 {
            t1270 = ((l.group[1]) >= ((arg4.wrapping_add(((((x5).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        let mut t1271: bool = t1270;
        if !t1271 {
            t1271 = ((l.group[2]) >= ((arg4.wrapping_add(((((x5).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        t1269 = t1271;
    }
    if t1269 {
        return;
    }
    // Phases: each chunk's, then the next phase's (its barriers).
    let mut state: Vec<u8> = Vec::with_capacity(n.div_ceil(8) as usize * 320);
    let st: *mut u8 = state.as_mut_ptr();
    {
        let phase = 0u32;
        let mut lid0 = 0u32;
        while lid0 + 8 <= n {
            let mut live = [vdupq_n_u32(u32::MAX); 2];
            let gid0 = l.group[0] * l.block[0] + lid0;
            'chunk: {
                b0_r0r1r2sr6gi_f(l, lid0, arg0, arg1, arg2, x3, arg6, lid0, phase, st.add((lid0 / 8) as usize * 320));
            }
            lid0 += 8;
        }
        if lid0 < n {
            // Lanes part: the group's last threads, those under `n`.
            let t1272: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
            let mut live = t1272;
            let gid0 = l.group[0] * l.block[0] + lid0;
            'chunk: {
                b0_r0r1r2sr6gi_p(l, lid0, live, arg0, arg1, arg2, x3, arg6, lid0, phase, st.add((lid0 / 8) as usize * 320));
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
                b0_r0r1r2sr6gi_f(l, lid0, arg0, arg1, arg2, x3, arg6, lid0, phase, st.add((lid0 / 8) as usize * 320));
            }
            lid0 += 8;
        }
        if lid0 < n {
            // Lanes part: the group's last threads, those under `n`.
            let t1273: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
            let mut live = t1273;
            let gid0 = l.group[0] * l.block[0] + lid0;
            'chunk: {
                b0_r0r1r2sr6gi_p(l, lid0, live, arg0, arg1, arg2, x3, arg6, lid0, phase, st.add((lid0 / 8) as usize * 320));
            }
        }
    }
}
