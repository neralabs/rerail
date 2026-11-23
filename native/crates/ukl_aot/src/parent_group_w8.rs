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
                    let t45 = bits(&t43);
                    if t45.count_ones() <= 2 {
                        let mut t46 = t45;
                        while t46 != 0 {
                            let j = t46.trailing_zeros() as usize;
                            t46 &= t46 - 1;
                            let t47 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x7);
                            let mut x7_j: u32 = t47[j];
                            let t48 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x8);
                            let mut x8_j: u32 = t48[j];
                            let mut x10_j: u32 = x10;
                            loop {
                                let mut t49: bool = ((x10_j) < (7u32));
                                if t49 {
                                    t49 = ((x8_j) != (4294967295u32));
                                }
                                let mut t50: bool = t49;
                                if t50 {
                                    t50 = ((((x8_j) & (2147483648u32))) != (0u32));
                                }
                                let mut t51: bool = t50;
                                if t51 {
                                    t51 = ((((x8_j) & (2147483647u32))) >= (x7_j));
                                }
                                if !(t51) { break; }
                                x8_j = (arg6.wrapping_add(((((((x8_j) & (2147483647u32))).wrapping_sub(x7_j)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned();
                                let t52 = x10_j;
                                x10_j = (x10_j).wrapping_add(1u32);
                            }
                            let mut t53 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x8);
                            t53[j] = x8_j as u32;
                            x8 = core::mem::transmute::<[u32; 8], [uint32x4_t; 2]>(t53);
                        }
                        break;
                    }
                    // Lanes part: each its own rounds.
                    let t55 = [vdupq_n_u32(4294967295u32); 2];
                    let t56: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x8[0], t55[0])), vmvnq_u32(vceqq_u32(x8[1], t55[1]))];
                    let t57: [uint32x4_t; 2] = [vandq_u32(vdupq_n_u32(if ((x10) < (7u32)) { u32::MAX } else { 0 }), t56[0]), vandq_u32(vdupq_n_u32(if ((x10) < (7u32)) { u32::MAX } else { 0 }), t56[1])];
                    let t58 = [vdupq_n_u32(2147483648u32); 2];
                    let t59: [uint32x4_t; 2] = [vandq_u32(x8[0], t58[0]), vandq_u32(x8[1], t58[1])];
                    let t60 = [vdupq_n_u32(0u32); 2];
                    let t61: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t59[0], t60[0])), vmvnq_u32(vceqq_u32(t59[1], t60[1]))];
                    let t62: [uint32x4_t; 2] = [vandq_u32(t57[0], t61[0]), vandq_u32(t57[1], t61[1])];
                    let t63 = [vdupq_n_u32(2147483647u32); 2];
                    let t64: [uint32x4_t; 2] = [vandq_u32(x8[0], t63[0]), vandq_u32(x8[1], t63[1])];
                    let t65: [uint32x4_t; 2] = [vcgeq_u32(t64[0], x7[0]), vcgeq_u32(t64[1], x7[1])];
                    let t66: [uint32x4_t; 2] = [vandq_u32(t62[0], t65[0]), vandq_u32(t62[1], t65[1])];
                    t43 = [vandq_u32(t43[0], t66[0]), vandq_u32(t43[1], t66[1])];
                    if !(any(&t43)) { break; }
                    let t68: [uint32x4_t; 2] = [vandq_u32(x8[0], t63[0]), vandq_u32(x8[1], t63[1])];
                    let t69: [uint32x4_t; 2] = [vsubq_u32(t68[0], x7[0]), vsubq_u32(t68[1], x7[1])];
                    let t70 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(t43[0], t69[0]), vandq_u32(t43[1], t69[1]))));
                    let t71: [uint32x4_t; 2] = [vbslq_u32(t43[0], t69[0], t70), vbslq_u32(t43[1], t69[1], t70)];
                    let t72 = arg6 as *const u32;
                    let mut t73: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                    t73[0] = vld1q_lane_u32::<0>(t72.add(vgetq_lane_u32::<0>(t71[0]) as usize), t73[0]);
                    t73[0] = vld1q_lane_u32::<1>(t72.add(vgetq_lane_u32::<1>(t71[0]) as usize), t73[0]);
                    t73[0] = vld1q_lane_u32::<2>(t72.add(vgetq_lane_u32::<2>(t71[0]) as usize), t73[0]);
                    t73[0] = vld1q_lane_u32::<3>(t72.add(vgetq_lane_u32::<3>(t71[0]) as usize), t73[0]);
                    t73[1] = vld1q_lane_u32::<0>(t72.add(vgetq_lane_u32::<0>(t71[1]) as usize), t73[1]);
                    t73[1] = vld1q_lane_u32::<1>(t72.add(vgetq_lane_u32::<1>(t71[1]) as usize), t73[1]);
                    t73[1] = vld1q_lane_u32::<2>(t72.add(vgetq_lane_u32::<2>(t71[1]) as usize), t73[1]);
                    t73[1] = vld1q_lane_u32::<3>(t72.add(vgetq_lane_u32::<3>(t71[1]) as usize), t73[1]);
                    x8 = [vbslq_u32(t43[0], t73[0], x8[0]), vbslq_u32(t43[1], t73[1], x8[1])];
                    let t74: [uint32x4_t; 2] = [vandq_u32(t43[0], live[0]), vandq_u32(t43[1], live[1])];
                    let t75 = x10;
                    x10 = (x10).wrapping_add(1u32);
                }
                let t76 = [vdupq_n_u32(4294967295u32); 2];
                let t77: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x8[0], t76[0])), vmvnq_u32(vceqq_u32(x8[1], t76[1]))];
                let t78 = [vdupq_n_u32(2147483648u32); 2];
                let t79: [uint32x4_t; 2] = [vandq_u32(x8[0], t78[0]), vandq_u32(x8[1], t78[1])];
                let t80 = [vdupq_n_u32(0u32); 2];
                let t81: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t79[0], t80[0])), vmvnq_u32(vceqq_u32(t79[1], t80[1]))];
                let t82: [uint32x4_t; 2] = [vandq_u32(t77[0], t81[0]), vandq_u32(t77[1], t81[1])];
                let t83: [uint32x4_t; 2] = [vandq_u32(live[0], t82[0]), vandq_u32(live[1], t82[1])];
                if any(&t83) {
                    let t84 = [vdupq_n_u32(2147483647u32); 2];
                    let t85: [uint32x4_t; 2] = [vandq_u32(x8[0], t84[0]), vandq_u32(x8[1], t84[1])];
                    let t86 = b3_r0r1l_p(l, lid0, t83, arg0, arg1, t85);
                    x8 = [vbslq_u32(t83[0], t86[0], x8[0]), vbslq_u32(t83[1], t86[1], x8[1])];
                }
                let t87 = (arg2 as *mut u32).add((gid0) as usize);
                if all(&live) {
                    vst1q_u32(t87.add(0), x8[0]);
                    vst1q_u32(t87.add(4), x8[1]);
                } else {
                    let mut t88w = 0u32;
                    let t88 = core::hint::black_box(&raw mut t88w);
                    core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t87.add(0), t88).write(vgetq_lane_u32::<0>(x8[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t87.add(1), t88).write(vgetq_lane_u32::<1>(x8[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t87.add(2), t88).write(vgetq_lane_u32::<2>(x8[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t87.add(3), t88).write(vgetq_lane_u32::<3>(x8[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t87.add(4), t88).write(vgetq_lane_u32::<0>(x8[1]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t87.add(5), t88).write(vgetq_lane_u32::<1>(x8[1]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t87.add(6), t88).write(vgetq_lane_u32::<2>(x8[1]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t87.add(7), t88).write(vgetq_lane_u32::<3>(x8[1]));
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
            let t89: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
            let t90: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x6), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x6), vld1q_u32(LANE[1].as_ptr()))];
            let t91: [uint32x4_t; 2] = [vsubq_u32(t89[0], t90[0]), vsubq_u32(t89[1], t90[1])];
            x7 = t91;
            let t92 = [vdupq_n_u32(4294967295u32); 2];
            x8 = t92;
            if (((gid0) as u64 + 7) <= u32::MAX as u64 && ((gid0) as u64) >= ((a0) as u64)) {
            } else if ((gid0) as u64 + 7) < ((a0) as u64) {
                let t93 = b1_r0r1g_f(l, lid0, arg0, arg1);
                x9 = t93;
                let t94: u64 = (1u64).wrapping_shl((32u64) as u32);
                let t95 = [vdupq_n_u32((t94) as u32); 2];
                let t96 = [vdupq_n_u32(((t94) >> 32) as u32); 2];
                let t97: [uint32x4_t; 4] = [vandq_u32(x9[0], t95[0]), vandq_u32(x9[1], t95[1]), vandq_u32(x9[2], t96[0]), vandq_u32(x9[3], t96[1])];
                let t98: u64 = 0u64;
                let t99 = [vdupq_n_u32((t98) as u32); 2];
                let t100: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t97[0], t99[0]), vceqq_u32(t97[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t97[1], t99[1]), vceqq_u32(t97[3], vdupq_n_u32(0)))];
                let t101: [uint32x4_t; 2] = [vmvnq_u32(t100[0]), vmvnq_u32(t100[1])];
                let mut t102: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                let t103: [uint32x4_t; 2] = [vmvnq_u32(t101[0]), vmvnq_u32(t101[1])];
                {
                    let t104: [uint32x4_t; 2] = [x9[0], x9[1]];
                    let t105 = [vdupq_n_u32(2147483648u32); 2];
                    let t106: [uint32x4_t; 2] = [vorrq_u32(t105[0], t104[0]), vorrq_u32(t105[1], t104[1])];
                    t102 = [vbslq_u32(t101[0], t106[0], t102[0]), vbslq_u32(t101[1], t106[1], t102[1])];
                }
                {
                    let t107: [uint32x4_t; 2] = [x9[0], x9[1]];
                    t102 = [vbslq_u32(t103[0], t107[0], t102[0]), vbslq_u32(t103[1], t107[1], t102[1])];
                }
                x8 = t102;
            } else {
                // Lanes part: the chunk's lanes on both sides of the bound.
                let t108 = [vdupq_n_u32(a0); 2];
                let t109: [uint32x4_t; 2] = [vcltq_u32(t89[0], t108[0]), vcltq_u32(t89[1], t108[1])];
                let t110: [uint32x4_t; 2] = [vandq_u32(live[0], t109[0]), vandq_u32(live[1], t109[1])];
                if any(&t110) {
                    let t111 = b1_r0r1g_p(l, lid0, t110, arg0, arg1);
                    x9 = t111;
                    let t112: u64 = (1u64).wrapping_shl((32u64) as u32);
                    let t113 = [vdupq_n_u32((t112) as u32); 2];
                    let t114 = [vdupq_n_u32(((t112) >> 32) as u32); 2];
                    let t115: [uint32x4_t; 4] = [vandq_u32(x9[0], t113[0]), vandq_u32(x9[1], t113[1]), vandq_u32(x9[2], t114[0]), vandq_u32(x9[3], t114[1])];
                    let t116: u64 = 0u64;
                    let t117 = [vdupq_n_u32((t116) as u32); 2];
                    let t118: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t115[0], t117[0]), vceqq_u32(t115[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t115[1], t117[1]), vceqq_u32(t115[3], vdupq_n_u32(0)))];
                    let t119: [uint32x4_t; 2] = [vmvnq_u32(t118[0]), vmvnq_u32(t118[1])];
                    let mut t120: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                    let t121: [uint32x4_t; 2] = [vandq_u32(t110[0], t119[0]), vandq_u32(t110[1], t119[1])];
                    let t122: [uint32x4_t; 2] = [vbicq_u32(t110[0], t119[0]), vbicq_u32(t110[1], t119[1])];
                    {
                        let t123: [uint32x4_t; 2] = [x9[0], x9[1]];
                        let t124 = [vdupq_n_u32(2147483648u32); 2];
                        let t125: [uint32x4_t; 2] = [vorrq_u32(t124[0], t123[0]), vorrq_u32(t124[1], t123[1])];
                        t120 = [vbslq_u32(t121[0], t125[0], t120[0]), vbslq_u32(t121[1], t125[1], t120[1])];
                    }
                    {
                        let t126: [uint32x4_t; 2] = [x9[0], x9[1]];
                        t120 = [vbslq_u32(t122[0], t126[0], t120[0]), vbslq_u32(t122[1], t126[1], t120[1])];
                    }
                    x8 = [vbslq_u32(t110[0], t120[0], x8[0]), vbslq_u32(t110[1], t120[1], x8[1])];
                }
            }
            let t127 = (arg6 as *mut u32).add((x6) as usize);
            vst1q_u32(t127.add(0), x8[0]);
            vst1q_u32(t127.add(4), x8[1]);
        }
    }
    if phase == 1 {
        'phase: {
            let t128: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
            let t129 = [vdupq_n_u32(a0); 2];
            let t130: [uint32x4_t; 2] = [vcgeq_u32(t128[0], t129[0]), vcgeq_u32(t128[1], t129[1])];
            let t131: [uint32x4_t; 2] = [vandq_u32(live[0], t130[0]), vandq_u32(live[1], t130[1])];
            {
                live = [vbicq_u32(live[0], t131[0]), vbicq_u32(live[1], t131[1])];
            }
            if any(&live) {
                x10 = 0u32;
                let mut t145 = live;
                loop {
                    let t147 = bits(&t145);
                    if t147.count_ones() <= 2 {
                        let mut t148 = t147;
                        while t148 != 0 {
                            let j = t148.trailing_zeros() as usize;
                            t148 &= t148 - 1;
                            let t149 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x7);
                            let mut x7_j: u32 = t149[j];
                            let t150 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x8);
                            let mut x8_j: u32 = t150[j];
                            let mut x10_j: u32 = x10;
                            loop {
                                let mut t151: bool = ((x10_j) < (7u32));
                                if t151 {
                                    t151 = ((x8_j) != (4294967295u32));
                                }
                                let mut t152: bool = t151;
                                if t152 {
                                    t152 = ((((x8_j) & (2147483648u32))) != (0u32));
                                }
                                let mut t153: bool = t152;
                                if t153 {
                                    t153 = ((((x8_j) & (2147483647u32))) >= (x7_j));
                                }
                                if !(t153) { break; }
                                x8_j = (arg6.wrapping_add(((((((x8_j) & (2147483647u32))).wrapping_sub(x7_j)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned();
                                let t154 = x10_j;
                                x10_j = (x10_j).wrapping_add(1u32);
                            }
                            let mut t155 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x8);
                            t155[j] = x8_j as u32;
                            x8 = core::mem::transmute::<[u32; 8], [uint32x4_t; 2]>(t155);
                        }
                        break;
                    }
                    // Lanes part: each its own rounds.
                    let t157 = [vdupq_n_u32(4294967295u32); 2];
                    let t158: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x8[0], t157[0])), vmvnq_u32(vceqq_u32(x8[1], t157[1]))];
                    let t159: [uint32x4_t; 2] = [vandq_u32(vdupq_n_u32(if ((x10) < (7u32)) { u32::MAX } else { 0 }), t158[0]), vandq_u32(vdupq_n_u32(if ((x10) < (7u32)) { u32::MAX } else { 0 }), t158[1])];
                    let t160 = [vdupq_n_u32(2147483648u32); 2];
                    let t161: [uint32x4_t; 2] = [vandq_u32(x8[0], t160[0]), vandq_u32(x8[1], t160[1])];
                    let t162 = [vdupq_n_u32(0u32); 2];
                    let t163: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t161[0], t162[0])), vmvnq_u32(vceqq_u32(t161[1], t162[1]))];
                    let t164: [uint32x4_t; 2] = [vandq_u32(t159[0], t163[0]), vandq_u32(t159[1], t163[1])];
                    let t165 = [vdupq_n_u32(2147483647u32); 2];
                    let t166: [uint32x4_t; 2] = [vandq_u32(x8[0], t165[0]), vandq_u32(x8[1], t165[1])];
                    let t167: [uint32x4_t; 2] = [vcgeq_u32(t166[0], x7[0]), vcgeq_u32(t166[1], x7[1])];
                    let t168: [uint32x4_t; 2] = [vandq_u32(t164[0], t167[0]), vandq_u32(t164[1], t167[1])];
                    t145 = [vandq_u32(t145[0], t168[0]), vandq_u32(t145[1], t168[1])];
                    if !(any(&t145)) { break; }
                    let t170: [uint32x4_t; 2] = [vandq_u32(x8[0], t165[0]), vandq_u32(x8[1], t165[1])];
                    let t171: [uint32x4_t; 2] = [vsubq_u32(t170[0], x7[0]), vsubq_u32(t170[1], x7[1])];
                    let t172 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(t145[0], t171[0]), vandq_u32(t145[1], t171[1]))));
                    let t173: [uint32x4_t; 2] = [vbslq_u32(t145[0], t171[0], t172), vbslq_u32(t145[1], t171[1], t172)];
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
                    x8 = [vbslq_u32(t145[0], t175[0], x8[0]), vbslq_u32(t145[1], t175[1], x8[1])];
                    let t176: [uint32x4_t; 2] = [vandq_u32(t145[0], live[0]), vandq_u32(t145[1], live[1])];
                    let t177 = x10;
                    x10 = (x10).wrapping_add(1u32);
                }
                let t178 = [vdupq_n_u32(4294967295u32); 2];
                let t179: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x8[0], t178[0])), vmvnq_u32(vceqq_u32(x8[1], t178[1]))];
                let t180 = [vdupq_n_u32(2147483648u32); 2];
                let t181: [uint32x4_t; 2] = [vandq_u32(x8[0], t180[0]), vandq_u32(x8[1], t180[1])];
                let t182 = [vdupq_n_u32(0u32); 2];
                let t183: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t181[0], t182[0])), vmvnq_u32(vceqq_u32(t181[1], t182[1]))];
                let t184: [uint32x4_t; 2] = [vandq_u32(t179[0], t183[0]), vandq_u32(t179[1], t183[1])];
                let t185: [uint32x4_t; 2] = [vandq_u32(live[0], t184[0]), vandq_u32(live[1], t184[1])];
                if any(&t185) {
                    let t186 = [vdupq_n_u32(2147483647u32); 2];
                    let t187: [uint32x4_t; 2] = [vandq_u32(x8[0], t186[0]), vandq_u32(x8[1], t186[1])];
                    let t188 = b3_r0r1l_p(l, lid0, t185, arg0, arg1, t187);
                    x8 = [vbslq_u32(t185[0], t188[0], x8[0]), vbslq_u32(t185[1], t188[1], x8[1])];
                }
                let t189 = (arg2 as *mut u32).add((gid0) as usize);
                if all(&live) {
                    vst1q_u32(t189.add(0), x8[0]);
                    vst1q_u32(t189.add(4), x8[1]);
                } else {
                    let mut t190w = 0u32;
                    let t190 = core::hint::black_box(&raw mut t190w);
                    core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t189.add(0), t190).write(vgetq_lane_u32::<0>(x8[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t189.add(1), t190).write(vgetq_lane_u32::<1>(x8[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t189.add(2), t190).write(vgetq_lane_u32::<2>(x8[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t189.add(3), t190).write(vgetq_lane_u32::<3>(x8[0]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t189.add(4), t190).write(vgetq_lane_u32::<0>(x8[1]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t189.add(5), t190).write(vgetq_lane_u32::<1>(x8[1]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t189.add(6), t190).write(vgetq_lane_u32::<2>(x8[1]));
                    core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t189.add(7), t190).write(vgetq_lane_u32::<3>(x8[1]));
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
    let t191: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
    let t192 = [vdupq_n_u32(0u32); 2];
    let t193: [uint32x4_t; 2] = [vceqq_u32(t191[0], t192[0]), vceqq_u32(t191[1], t192[1])];
    let t194: [uint32x4_t; 2] = [vandq_u32(live[0], t193[0]), vandq_u32(live[1], t193[1])];
    {
        let t195: u64 = 4294967295u64;
        let t196 = [vdupq_n_u32((t195) as u32); 2];
        let t197: [uint32x4_t; 4] = [t196[0], t196[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = [vbslq_u32(t194[0], t197[0], res[0]), vbslq_u32(t194[1], t197[1], res[1]), vbslq_u32(t194[0], t197[2], res[2]), vbslq_u32(t194[1], t197[3], res[3])];
        live = [vbicq_u32(live[0], t194[0]), vbicq_u32(live[1], t194[1])];
    }
    if any(&live) {
        x3 = gid0.wrapping_sub(1u32);
        let t198 = if (x3 & 255u32) <= 248u32 { b4_r0r1u8_p(l, lid0, live, arg0, arg1, x3) } else { b4_r0r1i_p(l, lid0, live, arg0, arg1, x3) };
        x4 = t198;
        let t199: u64 = 1u64;
        let t200 = [vdupq_n_u32((t199) as u32); 2];
        let t201: [uint32x4_t; 2] = [vandq_u32(x4[0], t200[0]), vandq_u32(x4[1], t200[1])];
        let t202: u64 = 0u64;
        let t203 = [vdupq_n_u32((t202) as u32); 2];
        let t204: [uint32x4_t; 2] = [vceqq_u32(t201[0], t203[0]), vceqq_u32(t201[1], t203[1])];
        let t205: [uint32x4_t; 2] = [vandq_u32(live[0], t204[0]), vandq_u32(live[1], t204[1])];
        {
            let t206: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[1].as_ptr()))];
            let t207: [uint32x4_t; 4] = [t206[0], t206[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t205[0], t207[0], res[0]), vbslq_u32(t205[1], t207[1], res[1]), vbslq_u32(t205[0], t207[2], res[2]), vbslq_u32(t205[1], t207[3], res[3])];
            live = [vbicq_u32(live[0], t205[0]), vbicq_u32(live[1], t205[1])];
        }
        if any(&live) {
            let t208: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x4[0]), vshlq_n_u32::<30>(x4[2])), vorrq_u32(vshrq_n_u32::<2>(x4[1]), vshlq_n_u32::<30>(x4[3])), vshrq_n_u32::<2>(x4[2]), vshrq_n_u32::<2>(x4[3])];
            let t209: [uint32x4_t; 2] = [t208[0], t208[1]];
            x5 = t209;
            let t210 = [vdupq_n_u32(1u32); 2];
            let t211: [uint32x4_t; 2] = [vsubq_u32(x5[0], t210[0]), vsubq_u32(x5[1], t210[1])];
            let t212: [uint32x4_t; 2] = [vcgeq_u32(t211[0], t191[0]), vcgeq_u32(t211[1], t191[1])];
            let t213: [uint32x4_t; 2] = [vandq_u32(live[0], t212[0]), vandq_u32(live[1], t212[1])];
            {
                let t214: u64 = 4294967295u64;
                let t215 = [vdupq_n_u32((t214) as u32); 2];
                let t216: [uint32x4_t; 4] = [t215[0], t215[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t213[0], t216[0], res[0]), vbslq_u32(t213[1], t216[1], res[1]), vbslq_u32(t213[0], t216[2], res[2]), vbslq_u32(t213[1], t216[3], res[3])];
                live = [vbicq_u32(live[0], t213[0]), vbicq_u32(live[1], t213[1])];
            }
            if any(&live) {
                let t217: [uint32x4_t; 2] = [vsubq_u32(t191[0], x5[0]), vsubq_u32(t191[1], x5[1])];
                x6 = t217;
                let t218: u64 = 2u64;
                let t219 = [vdupq_n_u32((t218) as u32); 2];
                let t220: [uint32x4_t; 2] = [vandq_u32(x4[0], t219[0]), vandq_u32(x4[1], t219[1])];
                let t221: u64 = 0u64;
                let t222 = [vdupq_n_u32((t221) as u32); 2];
                let t223: [uint32x4_t; 2] = [vceqq_u32(t220[0], t222[0]), vceqq_u32(t220[1], t222[1])];
                let t224: [uint32x4_t; 2] = [vmvnq_u32(t223[0]), vmvnq_u32(t223[1])];
                let t225: [uint32x4_t; 2] = [vandq_u32(live[0], t224[0]), vandq_u32(live[1], t224[1])];
                {
                    let t226: [uint32x4_t; 4] = [x6[0], x6[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                    res = [vbslq_u32(t225[0], t226[0], res[0]), vbslq_u32(t225[1], t226[1], res[1]), vbslq_u32(t225[0], t226[2], res[2]), vbslq_u32(t225[1], t226[3], res[3])];
                    live = [vbicq_u32(live[0], t225[0]), vbicq_u32(live[1], t225[1])];
                }
                if any(&live) {
                    let t227: [uint32x4_t; 2] = [vceqq_u32(x6[0], t192[0]), vceqq_u32(x6[1], t192[1])];
                    let t228: [uint32x4_t; 2] = [vandq_u32(live[0], t227[0]), vandq_u32(live[1], t227[1])];
                    {
                        let t229: u64 = 4294967295u64;
                        let t230 = [vdupq_n_u32((t229) as u32); 2];
                        let t231: [uint32x4_t; 4] = [t230[0], t230[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                        res = [vbslq_u32(t228[0], t231[0], res[0]), vbslq_u32(t228[1], t231[1], res[1]), vbslq_u32(t228[0], t231[2], res[2]), vbslq_u32(t228[1], t231[3], res[3])];
                        live = [vbicq_u32(live[0], t228[0]), vbicq_u32(live[1], t228[1])];
                    }
                    if any(&live) {
                        let t232: u64 = (1u64).wrapping_shl((32u64) as u32);
                        let t233 = [vdupq_n_u32((t232) as u32); 2];
                        let t234 = [vdupq_n_u32(((t232) >> 32) as u32); 2];
                        let t235: [uint32x4_t; 4] = [vorrq_u32(t233[0], x6[0]), vorrq_u32(t233[1], x6[1]), t234[0], t234[1]];
                        res = [vbslq_u32(live[0], t235[0], res[0]), vbslq_u32(live[1], t235[1], res[1]), vbslq_u32(live[0], t235[2], res[2]), vbslq_u32(live[1], t235[3], res[3])];
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
        let t236 = if (x3 & 255u32) <= 248u32 { b4_r0r1u8_f(l, lid0, arg0, arg1, x3) } else { b4_r0r1i_f(l, lid0, arg0, arg1, x3) };
        x4 = t236;
        let t237: u64 = 1u64;
        let t238 = [vdupq_n_u32((t237) as u32); 2];
        let t239: [uint32x4_t; 2] = [vandq_u32(x4[0], t238[0]), vandq_u32(x4[1], t238[1])];
        let t240: u64 = 0u64;
        let t241 = [vdupq_n_u32((t240) as u32); 2];
        let t242: [uint32x4_t; 2] = [vceqq_u32(t239[0], t241[0]), vceqq_u32(t239[1], t241[1])];
        {
            let t243: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[1].as_ptr()))];
            let t244: [uint32x4_t; 4] = [t243[0], t243[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t242[0], t244[0], res[0]), vbslq_u32(t242[1], t244[1], res[1]), vbslq_u32(t242[0], t244[2], res[2]), vbslq_u32(t242[1], t244[3], res[3])];
            live = [vbicq_u32(live[0], t242[0]), vbicq_u32(live[1], t242[1])];
        }
        if any(&live) {
            let t245: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x4[0]), vshlq_n_u32::<30>(x4[2])), vorrq_u32(vshrq_n_u32::<2>(x4[1]), vshlq_n_u32::<30>(x4[3])), vshrq_n_u32::<2>(x4[2]), vshrq_n_u32::<2>(x4[3])];
            let t246: [uint32x4_t; 2] = [t245[0], t245[1]];
            x5 = t246;
            let t247 = [vdupq_n_u32(1u32); 2];
            let t248: [uint32x4_t; 2] = [vsubq_u32(x5[0], t247[0]), vsubq_u32(x5[1], t247[1])];
            let t249: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
            let t250: [uint32x4_t; 2] = [vcgeq_u32(t248[0], t249[0]), vcgeq_u32(t248[1], t249[1])];
            let t251: [uint32x4_t; 2] = [vandq_u32(live[0], t250[0]), vandq_u32(live[1], t250[1])];
            {
                let t252: u64 = 4294967295u64;
                let t253 = [vdupq_n_u32((t252) as u32); 2];
                let t254: [uint32x4_t; 4] = [t253[0], t253[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t251[0], t254[0], res[0]), vbslq_u32(t251[1], t254[1], res[1]), vbslq_u32(t251[0], t254[2], res[2]), vbslq_u32(t251[1], t254[3], res[3])];
                live = [vbicq_u32(live[0], t251[0]), vbicq_u32(live[1], t251[1])];
            }
            if any(&live) {
                let t255: [uint32x4_t; 2] = [vsubq_u32(t249[0], x5[0]), vsubq_u32(t249[1], x5[1])];
                x6 = t255;
                let t256: u64 = 2u64;
                let t257 = [vdupq_n_u32((t256) as u32); 2];
                let t258: [uint32x4_t; 2] = [vandq_u32(x4[0], t257[0]), vandq_u32(x4[1], t257[1])];
                let t259: u64 = 0u64;
                let t260 = [vdupq_n_u32((t259) as u32); 2];
                let t261: [uint32x4_t; 2] = [vceqq_u32(t258[0], t260[0]), vceqq_u32(t258[1], t260[1])];
                let t262: [uint32x4_t; 2] = [vmvnq_u32(t261[0]), vmvnq_u32(t261[1])];
                let t263: [uint32x4_t; 2] = [vandq_u32(live[0], t262[0]), vandq_u32(live[1], t262[1])];
                {
                    let t264: [uint32x4_t; 4] = [x6[0], x6[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                    res = [vbslq_u32(t263[0], t264[0], res[0]), vbslq_u32(t263[1], t264[1], res[1]), vbslq_u32(t263[0], t264[2], res[2]), vbslq_u32(t263[1], t264[3], res[3])];
                    live = [vbicq_u32(live[0], t263[0]), vbicq_u32(live[1], t263[1])];
                }
                if any(&live) {
                    let t265 = [vdupq_n_u32(0u32); 2];
                    let t266: [uint32x4_t; 2] = [vceqq_u32(x6[0], t265[0]), vceqq_u32(x6[1], t265[1])];
                    let t267: [uint32x4_t; 2] = [vandq_u32(live[0], t266[0]), vandq_u32(live[1], t266[1])];
                    {
                        let t268: u64 = 4294967295u64;
                        let t269 = [vdupq_n_u32((t268) as u32); 2];
                        let t270: [uint32x4_t; 4] = [t269[0], t269[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                        res = [vbslq_u32(t267[0], t270[0], res[0]), vbslq_u32(t267[1], t270[1], res[1]), vbslq_u32(t267[0], t270[2], res[2]), vbslq_u32(t267[1], t270[3], res[3])];
                        live = [vbicq_u32(live[0], t267[0]), vbicq_u32(live[1], t267[1])];
                    }
                    if any(&live) {
                        let t271: u64 = (1u64).wrapping_shl((32u64) as u32);
                        let t272 = [vdupq_n_u32((t271) as u32); 2];
                        let t273 = [vdupq_n_u32(((t271) >> 32) as u32); 2];
                        let t274: [uint32x4_t; 4] = [vorrq_u32(t272[0], x6[0]), vorrq_u32(t272[1], x6[1]), t273[0], t273[1]];
                        res = [vbslq_u32(live[0], t274[0], res[0]), vbslq_u32(live[1], t274[1], res[1]), vbslq_u32(live[0], t274[2], res[2]), vbslq_u32(live[1], t274[3], res[3])];
                        live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
                    }
                }
            }
        }
    } else if false {
        let t275: u64 = 4294967295u64;
        let t276 = [vdupq_n_u32((t275) as u32); 2];
        let t277: [uint32x4_t; 4] = [t276[0], t276[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t277;
        return res;
    } else {
        // Lanes part: the chunk's lanes on both sides of the bound.
        let t278: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t279 = [vdupq_n_u32(0u32); 2];
        let t280: [uint32x4_t; 2] = [vceqq_u32(t278[0], t279[0]), vceqq_u32(t278[1], t279[1])];
        let t281: [uint32x4_t; 2] = [vandq_u32(live[0], t280[0]), vandq_u32(live[1], t280[1])];
        {
            let t282: u64 = 4294967295u64;
            let t283 = [vdupq_n_u32((t282) as u32); 2];
            let t284: [uint32x4_t; 4] = [t283[0], t283[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t281[0], t284[0], res[0]), vbslq_u32(t281[1], t284[1], res[1]), vbslq_u32(t281[0], t284[2], res[2]), vbslq_u32(t281[1], t284[3], res[3])];
            live = [vbicq_u32(live[0], t281[0]), vbicq_u32(live[1], t281[1])];
        }
        if any(&live) {
            x3 = gid0.wrapping_sub(1u32);
            let t285 = if (x3 & 255u32) <= 248u32 { b4_r0r1u8_p(l, lid0, live, arg0, arg1, x3) } else { b4_r0r1i_p(l, lid0, live, arg0, arg1, x3) };
            x4 = t285;
            let t286: u64 = 1u64;
            let t287 = [vdupq_n_u32((t286) as u32); 2];
            let t288: [uint32x4_t; 2] = [vandq_u32(x4[0], t287[0]), vandq_u32(x4[1], t287[1])];
            let t289: u64 = 0u64;
            let t290 = [vdupq_n_u32((t289) as u32); 2];
            let t291: [uint32x4_t; 2] = [vceqq_u32(t288[0], t290[0]), vceqq_u32(t288[1], t290[1])];
            let t292: [uint32x4_t; 2] = [vandq_u32(live[0], t291[0]), vandq_u32(live[1], t291[1])];
            {
                let t293: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[1].as_ptr()))];
                let t294: [uint32x4_t; 4] = [t293[0], t293[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t292[0], t294[0], res[0]), vbslq_u32(t292[1], t294[1], res[1]), vbslq_u32(t292[0], t294[2], res[2]), vbslq_u32(t292[1], t294[3], res[3])];
                live = [vbicq_u32(live[0], t292[0]), vbicq_u32(live[1], t292[1])];
            }
            if any(&live) {
                let t295: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x4[0]), vshlq_n_u32::<30>(x4[2])), vorrq_u32(vshrq_n_u32::<2>(x4[1]), vshlq_n_u32::<30>(x4[3])), vshrq_n_u32::<2>(x4[2]), vshrq_n_u32::<2>(x4[3])];
                let t296: [uint32x4_t; 2] = [t295[0], t295[1]];
                x5 = t296;
                let t297 = [vdupq_n_u32(1u32); 2];
                let t298: [uint32x4_t; 2] = [vsubq_u32(x5[0], t297[0]), vsubq_u32(x5[1], t297[1])];
                let t299: [uint32x4_t; 2] = [vcgeq_u32(t298[0], t278[0]), vcgeq_u32(t298[1], t278[1])];
                let t300: [uint32x4_t; 2] = [vandq_u32(live[0], t299[0]), vandq_u32(live[1], t299[1])];
                {
                    let t301: u64 = 4294967295u64;
                    let t302 = [vdupq_n_u32((t301) as u32); 2];
                    let t303: [uint32x4_t; 4] = [t302[0], t302[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                    res = [vbslq_u32(t300[0], t303[0], res[0]), vbslq_u32(t300[1], t303[1], res[1]), vbslq_u32(t300[0], t303[2], res[2]), vbslq_u32(t300[1], t303[3], res[3])];
                    live = [vbicq_u32(live[0], t300[0]), vbicq_u32(live[1], t300[1])];
                }
                if any(&live) {
                    let t304: [uint32x4_t; 2] = [vsubq_u32(t278[0], x5[0]), vsubq_u32(t278[1], x5[1])];
                    x6 = t304;
                    let t305: u64 = 2u64;
                    let t306 = [vdupq_n_u32((t305) as u32); 2];
                    let t307: [uint32x4_t; 2] = [vandq_u32(x4[0], t306[0]), vandq_u32(x4[1], t306[1])];
                    let t308: u64 = 0u64;
                    let t309 = [vdupq_n_u32((t308) as u32); 2];
                    let t310: [uint32x4_t; 2] = [vceqq_u32(t307[0], t309[0]), vceqq_u32(t307[1], t309[1])];
                    let t311: [uint32x4_t; 2] = [vmvnq_u32(t310[0]), vmvnq_u32(t310[1])];
                    let t312: [uint32x4_t; 2] = [vandq_u32(live[0], t311[0]), vandq_u32(live[1], t311[1])];
                    {
                        let t313: [uint32x4_t; 4] = [x6[0], x6[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                        res = [vbslq_u32(t312[0], t313[0], res[0]), vbslq_u32(t312[1], t313[1], res[1]), vbslq_u32(t312[0], t313[2], res[2]), vbslq_u32(t312[1], t313[3], res[3])];
                        live = [vbicq_u32(live[0], t312[0]), vbicq_u32(live[1], t312[1])];
                    }
                    if any(&live) {
                        let t314: [uint32x4_t; 2] = [vceqq_u32(x6[0], t279[0]), vceqq_u32(x6[1], t279[1])];
                        let t315: [uint32x4_t; 2] = [vandq_u32(live[0], t314[0]), vandq_u32(live[1], t314[1])];
                        {
                            let t316: u64 = 4294967295u64;
                            let t317 = [vdupq_n_u32((t316) as u32); 2];
                            let t318: [uint32x4_t; 4] = [t317[0], t317[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                            res = [vbslq_u32(t315[0], t318[0], res[0]), vbslq_u32(t315[1], t318[1], res[1]), vbslq_u32(t315[0], t318[2], res[2]), vbslq_u32(t315[1], t318[3], res[3])];
                            live = [vbicq_u32(live[0], t315[0]), vbicq_u32(live[1], t315[1])];
                        }
                        if any(&live) {
                            let t319: u64 = (1u64).wrapping_shl((32u64) as u32);
                            let t320 = [vdupq_n_u32((t319) as u32); 2];
                            let t321 = [vdupq_n_u32(((t319) >> 32) as u32); 2];
                            let t322: [uint32x4_t; 4] = [vorrq_u32(t320[0], x6[0]), vorrq_u32(t320[1], x6[1]), t321[0], t321[1]];
                            res = [vbslq_u32(live[0], t322[0], res[0]), vbslq_u32(live[1], t322[1], res[1]), vbslq_u32(live[0], t322[2], res[2]), vbslq_u32(live[1], t322[3], res[3])];
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
    let t323 = [vdupq_n_u32(0u32); 2];
    let t324: [uint32x4_t; 2] = [vceqq_u32(x2[0], t323[0]), vceqq_u32(x2[1], t323[1])];
    let t325: [uint32x4_t; 2] = [vandq_u32(live[0], t324[0]), vandq_u32(live[1], t324[1])];
    {
        let t326: u64 = 4294967295u64;
        let t327 = [vdupq_n_u32((t326) as u32); 2];
        let t328: [uint32x4_t; 4] = [t327[0], t327[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = [vbslq_u32(t325[0], t328[0], res[0]), vbslq_u32(t325[1], t328[1], res[1]), vbslq_u32(t325[0], t328[2], res[2]), vbslq_u32(t325[1], t328[3], res[3])];
        live = [vbicq_u32(live[0], t325[0]), vbicq_u32(live[1], t325[1])];
    }
    if any(&live) {
        let t329 = [vdupq_n_u32(1u32); 2];
        let t330: [uint32x4_t; 2] = [vsubq_u32(x2[0], t329[0]), vsubq_u32(x2[1], t329[1])];
        x3 = t330;
        let t331 = b4_r0r1l_p(l, lid0, live, arg0, arg1, x3);
        x4 = t331;
        let t332: u64 = 1u64;
        let t333 = [vdupq_n_u32((t332) as u32); 2];
        let t334: [uint32x4_t; 2] = [vandq_u32(x4[0], t333[0]), vandq_u32(x4[1], t333[1])];
        let t335: u64 = 0u64;
        let t336 = [vdupq_n_u32((t335) as u32); 2];
        let t337: [uint32x4_t; 2] = [vceqq_u32(t334[0], t336[0]), vceqq_u32(t334[1], t336[1])];
        let t338: [uint32x4_t; 2] = [vandq_u32(live[0], t337[0]), vandq_u32(live[1], t337[1])];
        {
            let t339: [uint32x4_t; 4] = [x3[0], x3[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t338[0], t339[0], res[0]), vbslq_u32(t338[1], t339[1], res[1]), vbslq_u32(t338[0], t339[2], res[2]), vbslq_u32(t338[1], t339[3], res[3])];
            live = [vbicq_u32(live[0], t338[0]), vbicq_u32(live[1], t338[1])];
        }
        if any(&live) {
            let t340: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x4[0]), vshlq_n_u32::<30>(x4[2])), vorrq_u32(vshrq_n_u32::<2>(x4[1]), vshlq_n_u32::<30>(x4[3])), vshrq_n_u32::<2>(x4[2]), vshrq_n_u32::<2>(x4[3])];
            let t341: [uint32x4_t; 2] = [t340[0], t340[1]];
            x5 = t341;
            let t342: [uint32x4_t; 2] = [vsubq_u32(x5[0], t329[0]), vsubq_u32(x5[1], t329[1])];
            let t343: [uint32x4_t; 2] = [vcgeq_u32(t342[0], x2[0]), vcgeq_u32(t342[1], x2[1])];
            let t344: [uint32x4_t; 2] = [vandq_u32(live[0], t343[0]), vandq_u32(live[1], t343[1])];
            {
                let t345: u64 = 4294967295u64;
                let t346 = [vdupq_n_u32((t345) as u32); 2];
                let t347: [uint32x4_t; 4] = [t346[0], t346[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t344[0], t347[0], res[0]), vbslq_u32(t344[1], t347[1], res[1]), vbslq_u32(t344[0], t347[2], res[2]), vbslq_u32(t344[1], t347[3], res[3])];
                live = [vbicq_u32(live[0], t344[0]), vbicq_u32(live[1], t344[1])];
            }
            if any(&live) {
                let t348: [uint32x4_t; 2] = [vsubq_u32(x2[0], x5[0]), vsubq_u32(x2[1], x5[1])];
                x6 = t348;
                let t349: u64 = 2u64;
                let t350 = [vdupq_n_u32((t349) as u32); 2];
                let t351: [uint32x4_t; 2] = [vandq_u32(x4[0], t350[0]), vandq_u32(x4[1], t350[1])];
                let t352: u64 = 0u64;
                let t353 = [vdupq_n_u32((t352) as u32); 2];
                let t354: [uint32x4_t; 2] = [vceqq_u32(t351[0], t353[0]), vceqq_u32(t351[1], t353[1])];
                let t355: [uint32x4_t; 2] = [vmvnq_u32(t354[0]), vmvnq_u32(t354[1])];
                let t356: [uint32x4_t; 2] = [vandq_u32(live[0], t355[0]), vandq_u32(live[1], t355[1])];
                {
                    let t357: [uint32x4_t; 4] = [x6[0], x6[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                    res = [vbslq_u32(t356[0], t357[0], res[0]), vbslq_u32(t356[1], t357[1], res[1]), vbslq_u32(t356[0], t357[2], res[2]), vbslq_u32(t356[1], t357[3], res[3])];
                    live = [vbicq_u32(live[0], t356[0]), vbicq_u32(live[1], t356[1])];
                }
                if any(&live) {
                    let t358: [uint32x4_t; 2] = [vceqq_u32(x6[0], t323[0]), vceqq_u32(x6[1], t323[1])];
                    let t359: [uint32x4_t; 2] = [vandq_u32(live[0], t358[0]), vandq_u32(live[1], t358[1])];
                    {
                        let t360: u64 = 4294967295u64;
                        let t361 = [vdupq_n_u32((t360) as u32); 2];
                        let t362: [uint32x4_t; 4] = [t361[0], t361[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                        res = [vbslq_u32(t359[0], t362[0], res[0]), vbslq_u32(t359[1], t362[1], res[1]), vbslq_u32(t359[0], t362[2], res[2]), vbslq_u32(t359[1], t362[3], res[3])];
                        live = [vbicq_u32(live[0], t359[0]), vbicq_u32(live[1], t359[1])];
                    }
                    if any(&live) {
                        let t363: u64 = (1u64).wrapping_shl((32u64) as u32);
                        let t364 = [vdupq_n_u32((t363) as u32); 2];
                        let t365 = [vdupq_n_u32(((t363) >> 32) as u32); 2];
                        let t366: [uint32x4_t; 4] = [vorrq_u32(t364[0], x6[0]), vorrq_u32(t364[1], x6[1]), t365[0], t365[1]];
                        res = [vbslq_u32(live[0], t366[0], res[0]), vbslq_u32(live[1], t366[1], res[1]), vbslq_u32(live[0], t366[2], res[2]), vbslq_u32(live[1], t366[3], res[3])];
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
    let t367 = b1_r0r1l_p(l, lid0, live, arg0, arg1, x2);
    x3 = t367;
    let mut t376 = live;
    loop {
        // Lanes part: each its own rounds.
        let t378: u64 = (1u64).wrapping_shl((32u64) as u32);
        let t379 = [vdupq_n_u32((t378) as u32); 2];
        let t380 = [vdupq_n_u32(((t378) >> 32) as u32); 2];
        let t381: [uint32x4_t; 4] = [vandq_u32(x3[0], t379[0]), vandq_u32(x3[1], t379[1]), vandq_u32(x3[2], t380[0]), vandq_u32(x3[3], t380[1])];
        let t382: u64 = 0u64;
        let t383 = [vdupq_n_u32((t382) as u32); 2];
        let t384: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t381[0], t383[0]), vceqq_u32(t381[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t381[1], t383[1]), vceqq_u32(t381[3], vdupq_n_u32(0)))];
        let t385: [uint32x4_t; 2] = [vmvnq_u32(t384[0]), vmvnq_u32(t384[1])];
        t376 = [vandq_u32(t376[0], t385[0]), vandq_u32(t376[1], t385[1])];
        if !(any(&t376)) { break; }
        let t387: [uint32x4_t; 2] = [x3[0], x3[1]];
        let t388 = b1_r0r1l_p(l, lid0, t376, arg0, arg1, t387);
        x3 = [vbslq_u32(t376[0], t388[0], x3[0]), vbslq_u32(t376[1], t388[1], x3[1]), vbslq_u32(t376[0], t388[2], x3[2]), vbslq_u32(t376[1], t388[3], x3[3])];
    }
    let t389: [uint32x4_t; 2] = [x3[0], x3[1]];
    res = t389;
    return res;
}

#[inline(always)]
unsafe fn b4_r0r1i_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t390 = b5_r0r1si_p(l, lid0, live, arg0, arg1, 0u32, x2);
    res = t390;
    return res;
}

#[inline(always)]
unsafe fn b4_r0r1i_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t391 = b5_r0r1si_f(l, lid0, arg0, arg1, 0u32, x2);
    res = t391;
    return res;
}

#[inline(always)]
unsafe fn b4_r0r1u8_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t392 = b5_r0r1su8_p(l, lid0, live, arg0, arg1, 0u32, x2);
    res = t392;
    return res;
}

#[inline(always)]
unsafe fn b4_r0r1u8_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t393 = b5_r0r1su8_f(l, lid0, arg0, arg1, 0u32, x2);
    res = t393;
    return res;
}

#[inline(always)]
unsafe fn b4_r0r1l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t394 = b5_r0r1sl_p(l, lid0, live, arg0, arg1, 0u32, x2);
    res = t394;
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
    let t395 = b6_r0r1s_p(l, lid0, live, arg0, arg1, x2);
    x4 = t395;
    let t396: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[1].as_ptr()))];
    let t397: [uint32x4_t; 2] = [vshlq_u32(t396[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(t396[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t398 = b7_r0r1sl_p(l, lid0, live, arg0, arg1, (1u32).wrapping_add(x2), t397);
    x5 = t398;
    let t399 = b8_s_p(l, lid0, live, x4);
    x6 = t399;
    let t400 = [vdupq_n_u32(1023u32); 2];
    let t401: [uint32x4_t; 2] = [vandq_u32(t396[0], t400[0]), vandq_u32(t396[1], t400[1])];
    let t402 = [vdupq_n_u32(x4); 2];
    let t403: [uint32x4_t; 2] = [vmulq_u32(t401[0], t402[0]), vmulq_u32(t401[1], t402[1])];
    let t404 = b9_r0r1lls_p(l, lid0, live, arg0, arg1, x5, t403, 8u32);
    let t405: u64 = x6;
    let t406 = [vdupq_n_u32((t405) as u32); 2];
    let t407 = [vdupq_n_u32(((t405) >> 32) as u32); 2];
    let t408: [uint32x4_t; 4] = [vandq_u32(t404[0], t406[0]), vandq_u32(t404[1], t406[1]), vandq_u32(t404[2], t407[0]), vandq_u32(t404[3], t407[1])];
    x7 = t408;
    let t409: u64 = x6;
    let t410 = [vdupq_n_u32((t409) as u32); 2];
    let t411 = [vdupq_n_u32(((t409) >> 32) as u32); 2];
    let t412: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x7[0], t410[0]), vceqq_u32(x7[2], t411[0])), vandq_u32(vceqq_u32(x7[1], t410[1]), vceqq_u32(x7[3], t411[1]))];
    let mut t413: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t414: [uint32x4_t; 2] = [vandq_u32(live[0], t412[0]), vandq_u32(live[1], t412[1])];
    let t415: [uint32x4_t; 2] = [vbicq_u32(live[0], t412[0]), vbicq_u32(live[1], t412[1])];
    if any(&t414) {
        let t416: [uint32x4_t; 4];
        let t417 = bits(&t414);
        if t417.count_ones() <= 2 {
            let mut t418 = [0u32; 16];
            let mut t419 = t417;
            while t419 != 0 {
                let j = t419.trailing_zeros() as usize;
                t419 &= t419 - 1;
                let t420 = b10_r0r1ss_f(l, lid0, arg0, arg1, (x2).wrapping_add(1u32), (x3).wrapping_add(j as u32));
                let t421: u64 = t420;
                t418[j] = t421 as u32;
                t418[8 + j] = (t421 >> 32) as u32;
            }
            t416 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t418);
        } else {
            let t422 = b10_r0r1si_p(l, lid0, t414, arg0, arg1, (x2).wrapping_add(1u32), x3);
            t416 = t422;
        }
        t413 = [vbslq_u32(t414[0], t416[0], t413[0]), vbslq_u32(t414[1], t416[1], t413[1]), vbslq_u32(t414[0], t416[2], t413[2]), vbslq_u32(t414[1], t416[3], t413[3])];
    }
    {
        t413 = [vbslq_u32(t415[0], x7[0], t413[0]), vbslq_u32(t415[1], x7[1], t413[1]), vbslq_u32(t415[0], x7[2], t413[2]), vbslq_u32(t415[1], x7[3], t413[3])];
    }
    res = t413;
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
    let t423 = b6_r0r1s_f(l, lid0, arg0, arg1, x2);
    x4 = t423;
    let t424: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[1].as_ptr()))];
    let t425: [uint32x4_t; 2] = [vshlq_u32(t424[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(t424[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t426 = b7_r0r1sl_f(l, lid0, arg0, arg1, (1u32).wrapping_add(x2), t425);
    x5 = t426;
    let t427 = b8_s_f(l, lid0, x4);
    x6 = t427;
    let t428 = [vdupq_n_u32(1023u32); 2];
    let t429: [uint32x4_t; 2] = [vandq_u32(t424[0], t428[0]), vandq_u32(t424[1], t428[1])];
    let t430 = [vdupq_n_u32(x4); 2];
    let t431: [uint32x4_t; 2] = [vmulq_u32(t429[0], t430[0]), vmulq_u32(t429[1], t430[1])];
    let t432 = b9_r0r1lls_f(l, lid0, arg0, arg1, x5, t431, 8u32);
    let t433: u64 = x6;
    let t434 = [vdupq_n_u32((t433) as u32); 2];
    let t435 = [vdupq_n_u32(((t433) >> 32) as u32); 2];
    let t436: [uint32x4_t; 4] = [vandq_u32(t432[0], t434[0]), vandq_u32(t432[1], t434[1]), vandq_u32(t432[2], t435[0]), vandq_u32(t432[3], t435[1])];
    x7 = t436;
    let t437: u64 = x6;
    let t438 = [vdupq_n_u32((t437) as u32); 2];
    let t439 = [vdupq_n_u32(((t437) >> 32) as u32); 2];
    let t440: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x7[0], t438[0]), vceqq_u32(x7[2], t439[0])), vandq_u32(vceqq_u32(x7[1], t438[1]), vceqq_u32(x7[3], t439[1]))];
    let mut t441: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t442: [uint32x4_t; 2] = [vmvnq_u32(t440[0]), vmvnq_u32(t440[1])];
    if any(&t440) {
        let t443: [uint32x4_t; 4];
        let t444 = bits(&t440);
        if t444.count_ones() <= 2 {
            let mut t445 = [0u32; 16];
            let mut t446 = t444;
            while t446 != 0 {
                let j = t446.trailing_zeros() as usize;
                t446 &= t446 - 1;
                let t447 = b10_r0r1ss_f(l, lid0, arg0, arg1, (x2).wrapping_add(1u32), (x3).wrapping_add(j as u32));
                let t448: u64 = t447;
                t445[j] = t448 as u32;
                t445[8 + j] = (t448 >> 32) as u32;
            }
            t443 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t445);
        } else {
            let t449 = b10_r0r1si_p(l, lid0, t440, arg0, arg1, (x2).wrapping_add(1u32), x3);
            t443 = t449;
        }
        t441 = [vbslq_u32(t440[0], t443[0], t441[0]), vbslq_u32(t440[1], t443[1], t441[1]), vbslq_u32(t440[0], t443[2], t441[2]), vbslq_u32(t440[1], t443[3], t441[3])];
    }
    {
        t441 = [vbslq_u32(t442[0], x7[0], t441[0]), vbslq_u32(t442[1], x7[1], t441[1]), vbslq_u32(t442[0], x7[2], t441[2]), vbslq_u32(t442[1], x7[3], t441[3])];
    }
    res = t441;
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
    let t450 = b6_r0r1s_p(l, lid0, live, arg0, arg1, x2);
    x4 = t450;
    let t451 = b7_r0r1ss_p(l, lid0, live, arg0, arg1, (1u32).wrapping_add(x2), (x3 >> 10));
    x5 = t451;
    let t452 = b8_s_p(l, lid0, live, x4);
    x6 = t452;
    let t453 = b9_r0r1sns_p(l, lid0, live, arg0, arg1, x5, (x3 & 1023u32).wrapping_mul(x4), x4, 8u32);
    let t454: u64 = x6;
    let t455 = [vdupq_n_u32((t454) as u32); 2];
    let t456 = [vdupq_n_u32(((t454) >> 32) as u32); 2];
    let t457: [uint32x4_t; 4] = [vandq_u32(t453[0], t455[0]), vandq_u32(t453[1], t455[1]), vandq_u32(t453[2], t456[0]), vandq_u32(t453[3], t456[1])];
    x7 = t457;
    let t458: u64 = x6;
    let t459 = [vdupq_n_u32((t458) as u32); 2];
    let t460 = [vdupq_n_u32(((t458) >> 32) as u32); 2];
    let t461: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x7[0], t459[0]), vceqq_u32(x7[2], t460[0])), vandq_u32(vceqq_u32(x7[1], t459[1]), vceqq_u32(x7[3], t460[1]))];
    let mut t462: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t463: [uint32x4_t; 2] = [vandq_u32(live[0], t461[0]), vandq_u32(live[1], t461[1])];
    let t464: [uint32x4_t; 2] = [vbicq_u32(live[0], t461[0]), vbicq_u32(live[1], t461[1])];
    if any(&t463) {
        let t465: [uint32x4_t; 4];
        let t466 = bits(&t463);
        if t466.count_ones() <= 2 {
            let mut t467 = [0u32; 16];
            let mut t468 = t466;
            while t468 != 0 {
                let j = t468.trailing_zeros() as usize;
                t468 &= t468 - 1;
                let t469 = b10_r0r1ss_f(l, lid0, arg0, arg1, (x2).wrapping_add(1u32), (x3).wrapping_add(j as u32));
                let t470: u64 = t469;
                t467[j] = t470 as u32;
                t467[8 + j] = (t470 >> 32) as u32;
            }
            t465 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t467);
        } else {
            let t471 = b10_r0r1su8_p(l, lid0, t463, arg0, arg1, (x2).wrapping_add(1u32), x3);
            t465 = t471;
        }
        t462 = [vbslq_u32(t463[0], t465[0], t462[0]), vbslq_u32(t463[1], t465[1], t462[1]), vbslq_u32(t463[0], t465[2], t462[2]), vbslq_u32(t463[1], t465[3], t462[3])];
    }
    {
        t462 = [vbslq_u32(t464[0], x7[0], t462[0]), vbslq_u32(t464[1], x7[1], t462[1]), vbslq_u32(t464[0], x7[2], t462[2]), vbslq_u32(t464[1], x7[3], t462[3])];
    }
    res = t462;
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
    let t472 = b6_r0r1s_f(l, lid0, arg0, arg1, x2);
    x4 = t472;
    let t473 = b7_r0r1ss_f(l, lid0, arg0, arg1, (1u32).wrapping_add(x2), (x3 >> 10));
    x5 = t473;
    let t474 = b8_s_f(l, lid0, x4);
    x6 = t474;
    let t475 = b9_r0r1sns_f(l, lid0, arg0, arg1, x5, (x3 & 1023u32).wrapping_mul(x4), x4, 8u32);
    let t476: u64 = x6;
    let t477 = [vdupq_n_u32((t476) as u32); 2];
    let t478 = [vdupq_n_u32(((t476) >> 32) as u32); 2];
    let t479: [uint32x4_t; 4] = [vandq_u32(t475[0], t477[0]), vandq_u32(t475[1], t477[1]), vandq_u32(t475[2], t478[0]), vandq_u32(t475[3], t478[1])];
    x7 = t479;
    let t480: u64 = x6;
    let t481 = [vdupq_n_u32((t480) as u32); 2];
    let t482 = [vdupq_n_u32(((t480) >> 32) as u32); 2];
    let t483: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x7[0], t481[0]), vceqq_u32(x7[2], t482[0])), vandq_u32(vceqq_u32(x7[1], t481[1]), vceqq_u32(x7[3], t482[1]))];
    let mut t484: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t485: [uint32x4_t; 2] = [vmvnq_u32(t483[0]), vmvnq_u32(t483[1])];
    if any(&t483) {
        let t486: [uint32x4_t; 4];
        let t487 = bits(&t483);
        if t487.count_ones() <= 2 {
            let mut t488 = [0u32; 16];
            let mut t489 = t487;
            while t489 != 0 {
                let j = t489.trailing_zeros() as usize;
                t489 &= t489 - 1;
                let t490 = b10_r0r1ss_f(l, lid0, arg0, arg1, (x2).wrapping_add(1u32), (x3).wrapping_add(j as u32));
                let t491: u64 = t490;
                t488[j] = t491 as u32;
                t488[8 + j] = (t491 >> 32) as u32;
            }
            t486 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t488);
        } else {
            let t492 = b10_r0r1su8_p(l, lid0, t483, arg0, arg1, (x2).wrapping_add(1u32), x3);
            t486 = t492;
        }
        t484 = [vbslq_u32(t483[0], t486[0], t484[0]), vbslq_u32(t483[1], t486[1], t484[1]), vbslq_u32(t483[0], t486[2], t484[2]), vbslq_u32(t483[1], t486[3], t484[3])];
    }
    {
        t484 = [vbslq_u32(t485[0], x7[0], t484[0]), vbslq_u32(t485[1], x7[1], t484[1]), vbslq_u32(t485[0], x7[2], t484[2]), vbslq_u32(t485[1], x7[3], t484[3])];
    }
    res = t484;
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
    let t493 = b6_r0r1s_p(l, lid0, live, arg0, arg1, x2);
    x4 = t493;
    let t494: [uint32x4_t; 2] = [vshlq_u32(x3[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x3[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t495 = b7_r0r1sl_p(l, lid0, live, arg0, arg1, (1u32).wrapping_add(x2), t494);
    x5 = t495;
    let t496 = b8_s_p(l, lid0, live, x4);
    x6 = t496;
    let t497 = [vdupq_n_u32(1023u32); 2];
    let t498: [uint32x4_t; 2] = [vandq_u32(x3[0], t497[0]), vandq_u32(x3[1], t497[1])];
    let t499 = [vdupq_n_u32(x4); 2];
    let t500: [uint32x4_t; 2] = [vmulq_u32(t498[0], t499[0]), vmulq_u32(t498[1], t499[1])];
    let t501 = b9_r0r1lls_p(l, lid0, live, arg0, arg1, x5, t500, 8u32);
    let t502: u64 = x6;
    let t503 = [vdupq_n_u32((t502) as u32); 2];
    let t504 = [vdupq_n_u32(((t502) >> 32) as u32); 2];
    let t505: [uint32x4_t; 4] = [vandq_u32(t501[0], t503[0]), vandq_u32(t501[1], t503[1]), vandq_u32(t501[2], t504[0]), vandq_u32(t501[3], t504[1])];
    x7 = t505;
    let t506: u64 = x6;
    let t507 = [vdupq_n_u32((t506) as u32); 2];
    let t508 = [vdupq_n_u32(((t506) >> 32) as u32); 2];
    let t509: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x7[0], t507[0]), vceqq_u32(x7[2], t508[0])), vandq_u32(vceqq_u32(x7[1], t507[1]), vceqq_u32(x7[3], t508[1]))];
    let mut t510: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t511: [uint32x4_t; 2] = [vandq_u32(live[0], t509[0]), vandq_u32(live[1], t509[1])];
    let t512: [uint32x4_t; 2] = [vbicq_u32(live[0], t509[0]), vbicq_u32(live[1], t509[1])];
    if any(&t511) {
        let t513: [uint32x4_t; 4];
        let t514 = bits(&t511);
        if t514.count_ones() <= 2 {
            let t515 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x3);
            let mut t516 = [0u32; 16];
            let mut t517 = t514;
            while t517 != 0 {
                let j = t517.trailing_zeros() as usize;
                t517 &= t517 - 1;
                let t518 = b10_r0r1ss_f(l, lid0, arg0, arg1, (x2).wrapping_add(1u32), t515[j]);
                let t519: u64 = t518;
                t516[j] = t519 as u32;
                t516[8 + j] = (t519 >> 32) as u32;
            }
            t513 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t516);
        } else {
            let t520 = b10_r0r1sl_p(l, lid0, t511, arg0, arg1, (x2).wrapping_add(1u32), x3);
            t513 = t520;
        }
        t510 = [vbslq_u32(t511[0], t513[0], t510[0]), vbslq_u32(t511[1], t513[1], t510[1]), vbslq_u32(t511[0], t513[2], t510[2]), vbslq_u32(t511[1], t513[3], t510[3])];
    }
    {
        t510 = [vbslq_u32(t512[0], x7[0], t510[0]), vbslq_u32(t512[1], x7[1], t510[1]), vbslq_u32(t512[0], x7[2], t510[2]), vbslq_u32(t512[1], x7[3], t510[3])];
    }
    res = t510;
    return res;
}

#[inline(always)]
unsafe fn b6_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t521 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    res = (((t521).wrapping_shr(((4u32).wrapping_mul(x2)) as u32)) & (15u32));
    return res;
}

#[inline(always)]
unsafe fn b6_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t522 = b11_r0r1s_f(l, lid0, arg0, arg1, 16u32);
    res = (((t522).wrapping_shr(((4u32).wrapping_mul(x2)) as u32)) & (15u32));
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u32 = 0;
    let mut x5: u32 = 0;
    let mut live = k;
    let mut res: u32 = 0;
    let t523 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 40u32);
    let t524 = b12_r0r1s_p(l, lid0, live, arg0, arg1, 0u32);
    x4 = ((x3).wrapping_mul(t523)).wrapping_add(t524);
    if ((x2) == (0u32)) {
        res = x4;
        return res;
    }
    let t525 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    let t526 = b13_ss_p(l, lid0, live, (t525).wrapping_sub((x3).wrapping_shl((10u32) as u32)), 1024u32);
    x5 = t526;
    let t527 = b12_r0r1s_p(l, lid0, live, arg0, arg1, x2);
    res = (x4).wrapping_add((x5).wrapping_mul(t527));
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1ss_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32, x3: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u32 = 0;
    let mut x5: u32 = 0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t528 = b11_r0r1s_f(l, lid0, arg0, arg1, 40u32);
    let t529 = b12_r0r1s_f(l, lid0, arg0, arg1, 0u32);
    x4 = ((x3).wrapping_mul(t528)).wrapping_add(t529);
    if ((x2) == (0u32)) {
        res = x4;
        return res;
    }
    let t530 = b11_r0r1s_f(l, lid0, arg0, arg1, 4u32);
    let t531 = b13_ss_f(l, lid0, (t530).wrapping_sub((x3).wrapping_shl((10u32) as u32)), 1024u32);
    x5 = t531;
    let t532 = b12_r0r1s_f(l, lid0, arg0, arg1, x2);
    res = (x4).wrapping_add((x5).wrapping_mul(t532));
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x5: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t533 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 40u32);
    let t534 = [vdupq_n_u32(t533); 2];
    let t535: [uint32x4_t; 2] = [vmulq_u32(x3[0], t534[0]), vmulq_u32(x3[1], t534[1])];
    let t536 = b12_r0r1s_p(l, lid0, live, arg0, arg1, 0u32);
    let t537 = [vdupq_n_u32(t536); 2];
    let t538: [uint32x4_t; 2] = [vaddq_u32(t535[0], t537[0]), vaddq_u32(t535[1], t537[1])];
    x4 = t538;
    if ((x2) == (0u32)) {
        res = x4;
        return res;
    }
    let t539 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    let t540: [uint32x4_t; 2] = [vshlq_u32(x3[0], vdupq_n_s32(((10u32) & 31) as i32)), vshlq_u32(x3[1], vdupq_n_s32(((10u32) & 31) as i32))];
    let t541 = [vdupq_n_u32(t539); 2];
    let t542: [uint32x4_t; 2] = [vsubq_u32(t541[0], t540[0]), vsubq_u32(t541[1], t540[1])];
    let t543 = b13_ls_p(l, lid0, live, t542, 1024u32);
    x5 = t543;
    let t544 = b12_r0r1s_p(l, lid0, live, arg0, arg1, x2);
    let t545 = [vdupq_n_u32(t544); 2];
    let t546: [uint32x4_t; 2] = [vmulq_u32(x5[0], t545[0]), vmulq_u32(x5[1], t545[1])];
    let t547: [uint32x4_t; 2] = [vaddq_u32(x4[0], t546[0]), vaddq_u32(x4[1], t546[1])];
    res = t547;
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1sl_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32, x3: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x5: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t548 = b11_r0r1s_f(l, lid0, arg0, arg1, 40u32);
    let t549 = [vdupq_n_u32(t548); 2];
    let t550: [uint32x4_t; 2] = [vmulq_u32(x3[0], t549[0]), vmulq_u32(x3[1], t549[1])];
    let t551 = b12_r0r1s_f(l, lid0, arg0, arg1, 0u32);
    let t552 = [vdupq_n_u32(t551); 2];
    let t553: [uint32x4_t; 2] = [vaddq_u32(t550[0], t552[0]), vaddq_u32(t550[1], t552[1])];
    x4 = t553;
    if ((x2) == (0u32)) {
        res = x4;
        return res;
    }
    let t554 = b11_r0r1s_f(l, lid0, arg0, arg1, 4u32);
    let t555: [uint32x4_t; 2] = [vshlq_u32(x3[0], vdupq_n_s32(((10u32) & 31) as i32)), vshlq_u32(x3[1], vdupq_n_s32(((10u32) & 31) as i32))];
    let t556 = [vdupq_n_u32(t554); 2];
    let t557: [uint32x4_t; 2] = [vsubq_u32(t556[0], t555[0]), vsubq_u32(t556[1], t555[1])];
    let t558 = b13_ls_f(l, lid0, t557, 1024u32);
    x5 = t558;
    let t559 = b12_r0r1s_f(l, lid0, arg0, arg1, x2);
    let t560 = [vdupq_n_u32(t559); 2];
    let t561: [uint32x4_t; 2] = [vmulq_u32(x5[0], t560[0]), vmulq_u32(x5[1], t560[1])];
    let t562: [uint32x4_t; 2] = [vaddq_u32(x4[0], t561[0]), vaddq_u32(x4[1], t561[1])];
    res = t562;
    return res;
}

#[inline(always)]
unsafe fn b8_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t563: u64;
    if ((x0) >= (8u32)) {
        t563 = !(0u64);
    } else {
        t563 = ((1u64).wrapping_shl(((((x0).wrapping_mul(8u32)) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t563;
    return res;
}

#[inline(always)]
unsafe fn b8_s_f(l: &Launch, lid0: u32, x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t564: u64;
    if ((x0) >= (8u32)) {
        t564 = !(0u64);
    } else {
        t564 = ((1u64).wrapping_shl(((((x0).wrapping_mul(8u32)) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t564;
    return res;
}

#[inline(always)]
unsafe fn b9_r0r1sns_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3f: u32, x3s: u32, x4: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t565 = b14_r0sns_p(l, lid0, live, arg0, x2, x3f, x3s, x4);
    res = t565;
    return res;
}

#[inline(always)]
unsafe fn b9_r0r1sns_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32, x3f: u32, x3s: u32, x4: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t566 = b14_r0sns_f(l, lid0, arg0, x2, x3f, x3s, x4);
    res = t566;
    return res;
}

#[inline(always)]
unsafe fn b9_r0r1lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: [uint32x4_t; 2], x3: [uint32x4_t; 2], x4: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t567 = b14_r0lls_p(l, lid0, live, arg0, x2, x3, x4);
    res = t567;
    return res;
}

#[inline(always)]
unsafe fn b9_r0r1lls_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: [uint32x4_t; 2], x3: [uint32x4_t; 2], x4: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t568 = b14_r0lls_f(l, lid0, arg0, x2, x3, x4);
    res = t568;
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
    let t569 = b11_r0r1s_f(l, lid0, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x2)));
    x4 = t569;
    let t570 = b11_r0r1s_f(l, lid0, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x2)));
    x5 = t570;
    x6 = (x3).wrapping_shr((10u32) as u32);
    let t571 = b15_r0r1sss_f(l, lid0, arg0, arg1, x5, (x6).wrapping_mul(4u32), 4u32);
    x7 = ((t571) as u32);
    let t572 = b15_r0r1sss_f(l, lid0, arg0, arg1, x5, ((x6).wrapping_mul(4u32)).wrapping_add(4u32), 4u32);
    x8 = ((t572) as u32);
    let t573 = b11_r0r1s_f(l, lid0, arg0, arg1, 4u32);
    x9 = ((t573).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
    let t574 = b16_ss_f(l, lid0, 4u32, (x9).wrapping_sub((x6).wrapping_mul(4u32)));
    x10 = t574;
    x11 = (((x3).wrapping_shr((8u32) as u32)) & (3u32));
    let t575: u32;
    if ((x11) == (0u32)) {
        t575 = 0u32;
    } else {
        let t576 = b17_ss_f(l, lid0, (x7).wrapping_add((2u32).wrapping_mul((x11).wrapping_sub(1u32))), x8);
        let t577 = b15_r0r1sss_f(l, lid0, arg0, arg1, x4, t576, 2u32);
        t575 = ((t577) as u32);
    }
    x12 = t575;
    let t578 = b17_ss_f(l, lid0, (x7).wrapping_add((2u32).wrapping_mul(x11)), x8);
    let t579 = b15_r0r1sss_f(l, lid0, arg0, arg1, x4, t578, 2u32);
    x13 = ((t579) as u32);
    x14 = (x7).wrapping_add((2u32).wrapping_mul(x10));
    x15 = ((x3) & (255u32));
    x16 = 9259542123273814144u64;
    x17 = (((x15) as u64)).wrapping_mul(72340172838076673u64);
    x18 = x12;
    x19 = x12;
    loop {
        // Rounds of one condition: every lane goes on.
        if !(((x19) < (x13))) { break; }
        let t580 = b17_ss_f(l, lid0, (x14).wrapping_add(x19), x8);
        let t581 = b15_r0r1sss_f(l, lid0, arg0, arg1, x4, t580, 8u32);
        x20 = t581;
        x21 = (((x20) | (x16))).wrapping_sub(((x17) & (!(x16))));
        x22 = ((x16) & (((((!(x20)) & (x17))) | (((!(((x20) ^ (x17)))) & (!(x21)))))));
        x23 = (x13).wrapping_sub(x19);
        let t582: u64;
        if ((x23) >= (8u32)) {
            t582 = x22;
        } else {
            t582 = ((x22) & (((1u64).wrapping_shl(((((8u32).wrapping_mul(x23)) as u64)) as u32)).wrapping_sub(1u64)));
        }
        let t583 = b18_s_f(l, lid0, t582);
        x18 = (x18).wrapping_add(t583);
        x19 = (x19).wrapping_add(8u32);
    }
    x12 = x18;
    let t584 = b11_r0r1s_f(l, lid0, arg0, arg1, 16u32);
    x24 = (((t584).wrapping_shr((((((4u32).wrapping_mul(x2)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32));
    let t585 = b17_ss_f(l, lid0, (x7).wrapping_add((2u32).wrapping_mul((x10).wrapping_sub(1u32))), x8);
    let t586 = b15_r0r1sss_f(l, lid0, arg0, arg1, x4, t585, 2u32);
    x25 = ((t586) as u32);
    let t587 = b17_ss_f(l, lid0, ((x14).wrapping_add(x25)).wrapping_add((x12).wrapping_mul(x24)), x8);
    x26 = t587;
    let t588 = b15_r0r1sss_f(l, lid0, arg0, arg1, x4, x26, 8u32);
    let t589 = b8_s_f(l, lid0, x24);
    res = ((t588) & (t589));
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
    let t590 = b11_r0r1s_p(l, lid0, live, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x2)));
    x4 = t590;
    let t591 = b11_r0r1s_p(l, lid0, live, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x2)));
    x5 = t591;
    let t592: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[1].as_ptr()))];
    let t593: [uint32x4_t; 2] = [vshlq_u32(t592[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(t592[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    x6 = t593;
    let t594 = [vdupq_n_u32(4u32); 2];
    let t595: [uint32x4_t; 2] = [vmulq_u32(x6[0], t594[0]), vmulq_u32(x6[1], t594[1])];
    let t596 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x5, t595, 4u32);
    let t597: [uint32x4_t; 2] = [t596[0], t596[1]];
    x7 = t597;
    let t598: [uint32x4_t; 2] = [vmulq_u32(x6[0], t594[0]), vmulq_u32(x6[1], t594[1])];
    let t599: [uint32x4_t; 2] = [vaddq_u32(t598[0], t594[0]), vaddq_u32(t598[1], t594[1])];
    let t600 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x5, t599, 4u32);
    let t601: [uint32x4_t; 2] = [t600[0], t600[1]];
    x8 = t601;
    let t602 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    x9 = ((t602).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
    let t603: [uint32x4_t; 2] = [vmulq_u32(x6[0], t594[0]), vmulq_u32(x6[1], t594[1])];
    let t604 = [vdupq_n_u32(x9); 2];
    let t605: [uint32x4_t; 2] = [vsubq_u32(t604[0], t603[0]), vsubq_u32(t604[1], t603[1])];
    let t606 = b16_sl_p(l, lid0, live, 4u32, t605);
    x10 = t606;
    let t607: [uint32x4_t; 2] = [vshlq_u32(t592[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(t592[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
    let t608 = [vdupq_n_u32(3u32); 2];
    let t609: [uint32x4_t; 2] = [vandq_u32(t607[0], t608[0]), vandq_u32(t607[1], t608[1])];
    x11 = t609;
    let t610 = [vdupq_n_u32(0u32); 2];
    let t611: [uint32x4_t; 2] = [vceqq_u32(x11[0], t610[0]), vceqq_u32(x11[1], t610[1])];
    let mut t612: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t613: [uint32x4_t; 2] = [vandq_u32(live[0], t611[0]), vandq_u32(live[1], t611[1])];
    let t614: [uint32x4_t; 2] = [vbicq_u32(live[0], t611[0]), vbicq_u32(live[1], t611[1])];
    {
        t612 = [vbslq_u32(t613[0], t610[0], t612[0]), vbslq_u32(t613[1], t610[1], t612[1])];
    }
    if any(&t614) {
        let t615 = [vdupq_n_u32(1u32); 2];
        let t616: [uint32x4_t; 2] = [vsubq_u32(x11[0], t615[0]), vsubq_u32(x11[1], t615[1])];
        let t617 = [vdupq_n_u32(2u32); 2];
        let t618: [uint32x4_t; 2] = [vmulq_u32(t617[0], t616[0]), vmulq_u32(t617[1], t616[1])];
        let t619: [uint32x4_t; 2] = [vaddq_u32(x7[0], t618[0]), vaddq_u32(x7[1], t618[1])];
        let t620 = b17_ll_p(l, lid0, t614, t619, x8);
        let t621 = b15_r0r1sls_p(l, lid0, t614, arg0, arg1, x4, t620, 2u32);
        let t622: [uint32x4_t; 2] = [t621[0], t621[1]];
        t612 = [vbslq_u32(t614[0], t622[0], t612[0]), vbslq_u32(t614[1], t622[1], t612[1])];
    }
    x12 = t612;
    let t623 = [vdupq_n_u32(2u32); 2];
    let t624: [uint32x4_t; 2] = [vmulq_u32(t623[0], x11[0]), vmulq_u32(t623[1], x11[1])];
    let t625: [uint32x4_t; 2] = [vaddq_u32(x7[0], t624[0]), vaddq_u32(x7[1], t624[1])];
    let t626 = b17_ll_p(l, lid0, live, t625, x8);
    let t627 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x4, t626, 2u32);
    let t628: [uint32x4_t; 2] = [t627[0], t627[1]];
    x13 = t628;
    let t629: [uint32x4_t; 2] = [vmulq_u32(t623[0], x10[0]), vmulq_u32(t623[1], x10[1])];
    let t630: [uint32x4_t; 2] = [vaddq_u32(x7[0], t629[0]), vaddq_u32(x7[1], t629[1])];
    x14 = t630;
    let t631 = [vdupq_n_u32(255u32); 2];
    let t632: [uint32x4_t; 2] = [vandq_u32(t592[0], t631[0]), vandq_u32(t592[1], t631[1])];
    x15 = t632;
    x16 = 9259542123273814144u64;
    let t633: u64 = 72340172838076673u64;
    let t634 = [vdupq_n_u32((t633) as u32); 2];
    let t635 = [vdupq_n_u32(((t633) >> 32) as u32); 2];
    let t636: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x15[0]), vget_low_u32(t634[0]))), vreinterpretq_u32_u64(vmull_high_u32(x15[0], t634[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x15[1]), vget_low_u32(t634[1]))), vreinterpretq_u32_u64(vmull_high_u32(x15[1], t634[1])))];
    let t637: [uint32x4_t; 4] = [vmulq_u32(x15[0], t634[0]), vmulq_u32(x15[1], t634[1]), vmlaq_u32(t636[0], x15[0], t635[0]), vmlaq_u32(t636[1], x15[1], t635[1])];
    x17 = t637;
    x18 = x12;
    x19 = x12;
    let mut t639 = live;
    loop {
        // Lanes part: each its own rounds.
        let t641: [uint32x4_t; 2] = [vcltq_u32(x19[0], x13[0]), vcltq_u32(x19[1], x13[1])];
        t639 = [vandq_u32(t639[0], t641[0]), vandq_u32(t639[1], t641[1])];
        if !(any(&t639)) { break; }
        let t643: [uint32x4_t; 2] = [vaddq_u32(x14[0], x19[0]), vaddq_u32(x14[1], x19[1])];
        let t644 = b17_ll_p(l, lid0, t639, t643, x8);
        let t645 = b15_r0r1sls_p(l, lid0, t639, arg0, arg1, x4, t644, 8u32);
        x20 = t645;
        let t646: u64 = x16;
        let t647 = [vdupq_n_u32((t646) as u32); 2];
        let t648 = [vdupq_n_u32(((t646) >> 32) as u32); 2];
        let t649: [uint32x4_t; 4] = [vorrq_u32(x20[0], t647[0]), vorrq_u32(x20[1], t647[1]), vorrq_u32(x20[2], t648[0]), vorrq_u32(x20[3], t648[1])];
        let t650: u64 = !(x16);
        let t651 = [vdupq_n_u32((t650) as u32); 2];
        let t652 = [vdupq_n_u32(((t650) >> 32) as u32); 2];
        let t653: [uint32x4_t; 4] = [vandq_u32(x17[0], t651[0]), vandq_u32(x17[1], t651[1]), vandq_u32(x17[2], t652[0]), vandq_u32(x17[3], t652[1])];
        let t654: [uint32x4_t; 2] = [vcltq_u32(t649[0], t653[0]), vcltq_u32(t649[1], t653[1])];
        let t655: [uint32x4_t; 4] = [vsubq_u32(t649[0], t653[0]), vsubq_u32(t649[1], t653[1]), vaddq_u32(vsubq_u32(t649[2], t653[2]), t654[0]), vaddq_u32(vsubq_u32(t649[3], t653[3]), t654[1])];
        x21 = t655;
        let t656: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x20[r]));
        let t657: [uint32x4_t; 4] = [vandq_u32(t656[0], x17[0]), vandq_u32(t656[1], x17[1]), vandq_u32(t656[2], x17[2]), vandq_u32(t656[3], x17[3])];
        let t658: [uint32x4_t; 4] = [veorq_u32(x20[0], x17[0]), veorq_u32(x20[1], x17[1]), veorq_u32(x20[2], x17[2]), veorq_u32(x20[3], x17[3])];
        let t659: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(t658[r]));
        let t660: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x21[r]));
        let t661: [uint32x4_t; 4] = [vandq_u32(t659[0], t660[0]), vandq_u32(t659[1], t660[1]), vandq_u32(t659[2], t660[2]), vandq_u32(t659[3], t660[3])];
        let t662: [uint32x4_t; 4] = [vorrq_u32(t657[0], t661[0]), vorrq_u32(t657[1], t661[1]), vorrq_u32(t657[2], t661[2]), vorrq_u32(t657[3], t661[3])];
        let t663: u64 = x16;
        let t664 = [vdupq_n_u32((t663) as u32); 2];
        let t665 = [vdupq_n_u32(((t663) >> 32) as u32); 2];
        let t666: [uint32x4_t; 4] = [vandq_u32(t664[0], t662[0]), vandq_u32(t664[1], t662[1]), vandq_u32(t665[0], t662[2]), vandq_u32(t665[1], t662[3])];
        x22 = t666;
        let t667: [uint32x4_t; 2] = [vsubq_u32(x13[0], x19[0]), vsubq_u32(x13[1], x19[1])];
        x23 = t667;
        let t668 = [vdupq_n_u32(8u32); 2];
        let t669: [uint32x4_t; 2] = [vcgeq_u32(x23[0], t668[0]), vcgeq_u32(x23[1], t668[1])];
        let mut t670: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t671: [uint32x4_t; 2] = [vandq_u32(t639[0], t669[0]), vandq_u32(t639[1], t669[1])];
        let t672: [uint32x4_t; 2] = [vbicq_u32(t639[0], t669[0]), vbicq_u32(t639[1], t669[1])];
        {
            t670 = [vbslq_u32(t671[0], x22[0], t670[0]), vbslq_u32(t671[1], x22[1], t670[1]), vbslq_u32(t671[0], x22[2], t670[2]), vbslq_u32(t671[1], x22[3], t670[3])];
        }
        {
            let t673: [uint32x4_t; 2] = [vmulq_u32(t668[0], x23[0]), vmulq_u32(t668[1], x23[1])];
            let t674: u64 = 1u64;
            let t675 = [vdupq_n_u32((t674) as u32); 2];
            let t676: [uint32x4_t; 2] = [t673[0], t673[1]];
            let t677: [uint32x4_t; 2] = [vandq_u32(t676[0], vdupq_n_u32(63)), vandq_u32(t676[1], vdupq_n_u32(63))];
            let t678: [uint32x4_t; 4] = [vshlq_u32(t675[0], vreinterpretq_s32_u32(t677[0])), vshlq_u32(t675[1], vreinterpretq_s32_u32(t677[1])), vshlq_u32(t675[0], vsubq_s32(vreinterpretq_s32_u32(t677[0]), vdupq_n_s32(32))), vshlq_u32(t675[1], vsubq_s32(vreinterpretq_s32_u32(t677[1]), vdupq_n_s32(32)))];
            let t679: u64 = 1u64;
            let t680 = [vdupq_n_u32((t679) as u32); 2];
            let t681: [uint32x4_t; 2] = [vcltq_u32(t678[0], t680[0]), vcltq_u32(t678[1], t680[1])];
            let t682: [uint32x4_t; 4] = [vsubq_u32(t678[0], t680[0]), vsubq_u32(t678[1], t680[1]), vaddq_u32(vsubq_u32(t678[2], vdupq_n_u32(0)), t681[0]), vaddq_u32(vsubq_u32(t678[3], vdupq_n_u32(0)), t681[1])];
            let t683: [uint32x4_t; 4] = [vandq_u32(x22[0], t682[0]), vandq_u32(x22[1], t682[1]), vandq_u32(x22[2], t682[2]), vandq_u32(x22[3], t682[3])];
            t670 = [vbslq_u32(t672[0], t683[0], t670[0]), vbslq_u32(t672[1], t683[1], t670[1]), vbslq_u32(t672[0], t683[2], t670[2]), vbslq_u32(t672[1], t683[3], t670[3])];
        }
        let t684 = b18_l_p(l, lid0, t639, t670);
        let t685: [uint32x4_t; 2] = [vaddq_u32(x18[0], t684[0]), vaddq_u32(x18[1], t684[1])];
        x18 = [vbslq_u32(t639[0], t685[0], x18[0]), vbslq_u32(t639[1], t685[1], x18[1])];
        let t686: [uint32x4_t; 2] = [vandq_u32(t639[0], live[0]), vandq_u32(t639[1], live[1])];
        let t687: [uint32x4_t; 2] = [vaddq_u32(x19[0], t668[0]), vaddq_u32(x19[1], t668[1])];
        x19 = t687;
    }
    x12 = x18;
    let t688 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    x24 = (((t688).wrapping_shr((((((4u32).wrapping_mul(x2)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32));
    let t689 = [vdupq_n_u32(1u32); 2];
    let t690: [uint32x4_t; 2] = [vsubq_u32(x10[0], t689[0]), vsubq_u32(x10[1], t689[1])];
    let t691: [uint32x4_t; 2] = [vmulq_u32(t623[0], t690[0]), vmulq_u32(t623[1], t690[1])];
    let t692: [uint32x4_t; 2] = [vaddq_u32(x7[0], t691[0]), vaddq_u32(x7[1], t691[1])];
    let t693 = b17_ll_p(l, lid0, live, t692, x8);
    let t694 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x4, t693, 2u32);
    let t695: [uint32x4_t; 2] = [t694[0], t694[1]];
    x25 = t695;
    let t696: [uint32x4_t; 2] = [vaddq_u32(x14[0], x25[0]), vaddq_u32(x14[1], x25[1])];
    let t697 = [vdupq_n_u32(x24); 2];
    let t698: [uint32x4_t; 2] = [vmulq_u32(x12[0], t697[0]), vmulq_u32(x12[1], t697[1])];
    let t699: [uint32x4_t; 2] = [vaddq_u32(t696[0], t698[0]), vaddq_u32(t696[1], t698[1])];
    let t700 = b17_ll_p(l, lid0, live, t699, x8);
    x26 = t700;
    let t701 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x4, x26, 8u32);
    let t702 = b8_s_p(l, lid0, live, x24);
    let t703: u64 = t702;
    let t704 = [vdupq_n_u32((t703) as u32); 2];
    let t705 = [vdupq_n_u32(((t703) >> 32) as u32); 2];
    let t706: [uint32x4_t; 4] = [vandq_u32(t701[0], t704[0]), vandq_u32(t701[1], t704[1]), vandq_u32(t701[2], t705[0]), vandq_u32(t701[3], t705[1])];
    res = t706;
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
    let t707 = b11_r0r1s_p(l, lid0, live, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x2)));
    x4 = t707;
    let t708 = b11_r0r1s_p(l, lid0, live, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x2)));
    x5 = t708;
    x6 = (x3 >> 10);
    let t709 = b15_r0r1sss_p(l, lid0, live, arg0, arg1, x5, (x6).wrapping_mul(4u32), 4u32);
    x7 = ((t709) as u32);
    let t710 = b15_r0r1sss_p(l, lid0, live, arg0, arg1, x5, ((x6).wrapping_mul(4u32)).wrapping_add(4u32), 4u32);
    x8 = ((t710) as u32);
    let t711 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    x9 = ((t711).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
    let t712 = b16_ss_p(l, lid0, live, 4u32, (x9).wrapping_sub((x6).wrapping_mul(4u32)));
    x10 = t712;
    x11 = (((x3 >> 8)) & (3u32));
    let t713: u32;
    if ((x11) == (0u32)) {
        t713 = 0u32;
    } else {
        let t714 = b17_ss_p(l, lid0, live, (x7).wrapping_add((2u32).wrapping_mul((x11).wrapping_sub(1u32))), x8);
        let t715 = b15_r0r1sss_p(l, lid0, live, arg0, arg1, x4, t714, 2u32);
        t713 = ((t715) as u32);
    }
    let t716 = [vdupq_n_u32(t713); 2];
    x12 = t716;
    let t717 = b17_ss_p(l, lid0, live, (x7).wrapping_add((2u32).wrapping_mul(x11)), x8);
    let t718 = b15_r0r1sss_p(l, lid0, live, arg0, arg1, x4, t717, 2u32);
    x13 = ((t718) as u32);
    x14 = (x7).wrapping_add((2u32).wrapping_mul(x10));
    x15 = (x3 & 255u32);
    x16 = 9259542123273814144u64;
    let t719: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x15), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x15), vld1q_u32(LANE[1].as_ptr()))];
    let t720: u64 = 72340172838076673u64;
    let t721 = [vdupq_n_u32((t720) as u32); 2];
    let t722 = [vdupq_n_u32(((t720) >> 32) as u32); 2];
    let t723: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(t719[0]), vget_low_u32(t721[0]))), vreinterpretq_u32_u64(vmull_high_u32(t719[0], t721[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(t719[1]), vget_low_u32(t721[1]))), vreinterpretq_u32_u64(vmull_high_u32(t719[1], t721[1])))];
    let t724: [uint32x4_t; 4] = [vmulq_u32(t719[0], t721[0]), vmulq_u32(t719[1], t721[1]), vmlaq_u32(t723[0], t719[0], t722[0]), vmlaq_u32(t723[1], t719[1], t722[1])];
    x17 = t724;
    x18 = x12;
    x19 = x12;
    let mut t727 = live;
    loop {
        // Lanes part: each its own rounds.
        let t729 = [vdupq_n_u32(x13); 2];
        let t730: [uint32x4_t; 2] = [vcltq_u32(x19[0], t729[0]), vcltq_u32(x19[1], t729[1])];
        t727 = [vandq_u32(t727[0], t730[0]), vandq_u32(t727[1], t730[1])];
        if !(any(&t727)) { break; }
        let t732 = [vdupq_n_u32(x14); 2];
        let t733: [uint32x4_t; 2] = [vaddq_u32(t732[0], x19[0]), vaddq_u32(t732[1], x19[1])];
        let t734 = b17_ls_p(l, lid0, t727, t733, x8);
        let t735 = b15_r0r1sls_p(l, lid0, t727, arg0, arg1, x4, t734, 8u32);
        x20 = t735;
        let t736: u64 = x16;
        let t737 = [vdupq_n_u32((t736) as u32); 2];
        let t738 = [vdupq_n_u32(((t736) >> 32) as u32); 2];
        let t739: [uint32x4_t; 4] = [vorrq_u32(x20[0], t737[0]), vorrq_u32(x20[1], t737[1]), vorrq_u32(x20[2], t738[0]), vorrq_u32(x20[3], t738[1])];
        let t740: u64 = !(x16);
        let t741 = [vdupq_n_u32((t740) as u32); 2];
        let t742 = [vdupq_n_u32(((t740) >> 32) as u32); 2];
        let t743: [uint32x4_t; 4] = [vandq_u32(x17[0], t741[0]), vandq_u32(x17[1], t741[1]), vandq_u32(x17[2], t742[0]), vandq_u32(x17[3], t742[1])];
        let t744: [uint32x4_t; 2] = [vcltq_u32(t739[0], t743[0]), vcltq_u32(t739[1], t743[1])];
        let t745: [uint32x4_t; 4] = [vsubq_u32(t739[0], t743[0]), vsubq_u32(t739[1], t743[1]), vaddq_u32(vsubq_u32(t739[2], t743[2]), t744[0]), vaddq_u32(vsubq_u32(t739[3], t743[3]), t744[1])];
        x21 = t745;
        let t746: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x20[r]));
        let t747: [uint32x4_t; 4] = [vandq_u32(t746[0], x17[0]), vandq_u32(t746[1], x17[1]), vandq_u32(t746[2], x17[2]), vandq_u32(t746[3], x17[3])];
        let t748: [uint32x4_t; 4] = [veorq_u32(x20[0], x17[0]), veorq_u32(x20[1], x17[1]), veorq_u32(x20[2], x17[2]), veorq_u32(x20[3], x17[3])];
        let t749: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(t748[r]));
        let t750: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x21[r]));
        let t751: [uint32x4_t; 4] = [vandq_u32(t749[0], t750[0]), vandq_u32(t749[1], t750[1]), vandq_u32(t749[2], t750[2]), vandq_u32(t749[3], t750[3])];
        let t752: [uint32x4_t; 4] = [vorrq_u32(t747[0], t751[0]), vorrq_u32(t747[1], t751[1]), vorrq_u32(t747[2], t751[2]), vorrq_u32(t747[3], t751[3])];
        let t753: u64 = x16;
        let t754 = [vdupq_n_u32((t753) as u32); 2];
        let t755 = [vdupq_n_u32(((t753) >> 32) as u32); 2];
        let t756: [uint32x4_t; 4] = [vandq_u32(t754[0], t752[0]), vandq_u32(t754[1], t752[1]), vandq_u32(t755[0], t752[2]), vandq_u32(t755[1], t752[3])];
        x22 = t756;
        let t757 = [vdupq_n_u32(x13); 2];
        let t758: [uint32x4_t; 2] = [vsubq_u32(t757[0], x19[0]), vsubq_u32(t757[1], x19[1])];
        x23 = t758;
        let t759 = [vdupq_n_u32(8u32); 2];
        let t760: [uint32x4_t; 2] = [vcgeq_u32(x23[0], t759[0]), vcgeq_u32(x23[1], t759[1])];
        let mut t761: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t762: [uint32x4_t; 2] = [vandq_u32(t727[0], t760[0]), vandq_u32(t727[1], t760[1])];
        let t763: [uint32x4_t; 2] = [vbicq_u32(t727[0], t760[0]), vbicq_u32(t727[1], t760[1])];
        {
            t761 = [vbslq_u32(t762[0], x22[0], t761[0]), vbslq_u32(t762[1], x22[1], t761[1]), vbslq_u32(t762[0], x22[2], t761[2]), vbslq_u32(t762[1], x22[3], t761[3])];
        }
        {
            let t764: [uint32x4_t; 2] = [vmulq_u32(t759[0], x23[0]), vmulq_u32(t759[1], x23[1])];
            let t765: u64 = 1u64;
            let t766 = [vdupq_n_u32((t765) as u32); 2];
            let t767: [uint32x4_t; 2] = [t764[0], t764[1]];
            let t768: [uint32x4_t; 2] = [vandq_u32(t767[0], vdupq_n_u32(63)), vandq_u32(t767[1], vdupq_n_u32(63))];
            let t769: [uint32x4_t; 4] = [vshlq_u32(t766[0], vreinterpretq_s32_u32(t768[0])), vshlq_u32(t766[1], vreinterpretq_s32_u32(t768[1])), vshlq_u32(t766[0], vsubq_s32(vreinterpretq_s32_u32(t768[0]), vdupq_n_s32(32))), vshlq_u32(t766[1], vsubq_s32(vreinterpretq_s32_u32(t768[1]), vdupq_n_s32(32)))];
            let t770: u64 = 1u64;
            let t771 = [vdupq_n_u32((t770) as u32); 2];
            let t772: [uint32x4_t; 2] = [vcltq_u32(t769[0], t771[0]), vcltq_u32(t769[1], t771[1])];
            let t773: [uint32x4_t; 4] = [vsubq_u32(t769[0], t771[0]), vsubq_u32(t769[1], t771[1]), vaddq_u32(vsubq_u32(t769[2], vdupq_n_u32(0)), t772[0]), vaddq_u32(vsubq_u32(t769[3], vdupq_n_u32(0)), t772[1])];
            let t774: [uint32x4_t; 4] = [vandq_u32(x22[0], t773[0]), vandq_u32(x22[1], t773[1]), vandq_u32(x22[2], t773[2]), vandq_u32(x22[3], t773[3])];
            t761 = [vbslq_u32(t763[0], t774[0], t761[0]), vbslq_u32(t763[1], t774[1], t761[1]), vbslq_u32(t763[0], t774[2], t761[2]), vbslq_u32(t763[1], t774[3], t761[3])];
        }
        let t775 = b18_l_p(l, lid0, t727, t761);
        let t776: [uint32x4_t; 2] = [vaddq_u32(x18[0], t775[0]), vaddq_u32(x18[1], t775[1])];
        x18 = [vbslq_u32(t727[0], t776[0], x18[0]), vbslq_u32(t727[1], t776[1], x18[1])];
        let t777: [uint32x4_t; 2] = [vandq_u32(t727[0], live[0]), vandq_u32(t727[1], live[1])];
        let t778: [uint32x4_t; 2] = [vaddq_u32(x19[0], t759[0]), vaddq_u32(x19[1], t759[1])];
        x19 = t778;
    }
    x12 = x18;
    let t779 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    x24 = (((t779).wrapping_shr((((((4u32).wrapping_mul(x2)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32));
    let t780 = b17_ss_p(l, lid0, live, (x7).wrapping_add((2u32).wrapping_mul((x10).wrapping_sub(1u32))), x8);
    let t781 = b15_r0r1sss_p(l, lid0, live, arg0, arg1, x4, t780, 2u32);
    x25 = ((t781) as u32);
    let t782 = [vdupq_n_u32(x24); 2];
    let t783: [uint32x4_t; 2] = [vmulq_u32(x12[0], t782[0]), vmulq_u32(x12[1], t782[1])];
    let t784 = [vdupq_n_u32((x14).wrapping_add(x25)); 2];
    let t785: [uint32x4_t; 2] = [vaddq_u32(t784[0], t783[0]), vaddq_u32(t784[1], t783[1])];
    let t786 = b17_ls_p(l, lid0, live, t785, x8);
    x26 = t786;
    let t787 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x4, x26, 8u32);
    let t788 = b8_s_p(l, lid0, live, x24);
    let t789: u64 = t788;
    let t790 = [vdupq_n_u32((t789) as u32); 2];
    let t791 = [vdupq_n_u32(((t789) >> 32) as u32); 2];
    let t792: [uint32x4_t; 4] = [vandq_u32(t787[0], t790[0]), vandq_u32(t787[1], t790[1]), vandq_u32(t787[2], t791[0]), vandq_u32(t787[3], t791[1])];
    res = t792;
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
    let t793 = b11_r0r1s_p(l, lid0, live, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x2)));
    x4 = t793;
    let t794 = b11_r0r1s_p(l, lid0, live, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x2)));
    x5 = t794;
    let t795: [uint32x4_t; 2] = [vshlq_u32(x3[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x3[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    x6 = t795;
    let t796 = [vdupq_n_u32(4u32); 2];
    let t797: [uint32x4_t; 2] = [vmulq_u32(x6[0], t796[0]), vmulq_u32(x6[1], t796[1])];
    let t798 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x5, t797, 4u32);
    let t799: [uint32x4_t; 2] = [t798[0], t798[1]];
    x7 = t799;
    let t800: [uint32x4_t; 2] = [vmulq_u32(x6[0], t796[0]), vmulq_u32(x6[1], t796[1])];
    let t801: [uint32x4_t; 2] = [vaddq_u32(t800[0], t796[0]), vaddq_u32(t800[1], t796[1])];
    let t802 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x5, t801, 4u32);
    let t803: [uint32x4_t; 2] = [t802[0], t802[1]];
    x8 = t803;
    let t804 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    x9 = ((t804).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
    let t805: [uint32x4_t; 2] = [vmulq_u32(x6[0], t796[0]), vmulq_u32(x6[1], t796[1])];
    let t806 = [vdupq_n_u32(x9); 2];
    let t807: [uint32x4_t; 2] = [vsubq_u32(t806[0], t805[0]), vsubq_u32(t806[1], t805[1])];
    let t808 = b16_sl_p(l, lid0, live, 4u32, t807);
    x10 = t808;
    let t809: [uint32x4_t; 2] = [vshlq_u32(x3[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(x3[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
    let t810 = [vdupq_n_u32(3u32); 2];
    let t811: [uint32x4_t; 2] = [vandq_u32(t809[0], t810[0]), vandq_u32(t809[1], t810[1])];
    x11 = t811;
    let t812 = [vdupq_n_u32(0u32); 2];
    let t813: [uint32x4_t; 2] = [vceqq_u32(x11[0], t812[0]), vceqq_u32(x11[1], t812[1])];
    let mut t814: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t815: [uint32x4_t; 2] = [vandq_u32(live[0], t813[0]), vandq_u32(live[1], t813[1])];
    let t816: [uint32x4_t; 2] = [vbicq_u32(live[0], t813[0]), vbicq_u32(live[1], t813[1])];
    {
        t814 = [vbslq_u32(t815[0], t812[0], t814[0]), vbslq_u32(t815[1], t812[1], t814[1])];
    }
    if any(&t816) {
        let t817 = [vdupq_n_u32(1u32); 2];
        let t818: [uint32x4_t; 2] = [vsubq_u32(x11[0], t817[0]), vsubq_u32(x11[1], t817[1])];
        let t819 = [vdupq_n_u32(2u32); 2];
        let t820: [uint32x4_t; 2] = [vmulq_u32(t819[0], t818[0]), vmulq_u32(t819[1], t818[1])];
        let t821: [uint32x4_t; 2] = [vaddq_u32(x7[0], t820[0]), vaddq_u32(x7[1], t820[1])];
        let t822 = b17_ll_p(l, lid0, t816, t821, x8);
        let t823 = b15_r0r1sls_p(l, lid0, t816, arg0, arg1, x4, t822, 2u32);
        let t824: [uint32x4_t; 2] = [t823[0], t823[1]];
        t814 = [vbslq_u32(t816[0], t824[0], t814[0]), vbslq_u32(t816[1], t824[1], t814[1])];
    }
    x12 = t814;
    let t825 = [vdupq_n_u32(2u32); 2];
    let t826: [uint32x4_t; 2] = [vmulq_u32(t825[0], x11[0]), vmulq_u32(t825[1], x11[1])];
    let t827: [uint32x4_t; 2] = [vaddq_u32(x7[0], t826[0]), vaddq_u32(x7[1], t826[1])];
    let t828 = b17_ll_p(l, lid0, live, t827, x8);
    let t829 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x4, t828, 2u32);
    let t830: [uint32x4_t; 2] = [t829[0], t829[1]];
    x13 = t830;
    let t831: [uint32x4_t; 2] = [vmulq_u32(t825[0], x10[0]), vmulq_u32(t825[1], x10[1])];
    let t832: [uint32x4_t; 2] = [vaddq_u32(x7[0], t831[0]), vaddq_u32(x7[1], t831[1])];
    x14 = t832;
    let t833 = [vdupq_n_u32(255u32); 2];
    let t834: [uint32x4_t; 2] = [vandq_u32(x3[0], t833[0]), vandq_u32(x3[1], t833[1])];
    x15 = t834;
    x16 = 9259542123273814144u64;
    let t835: u64 = 72340172838076673u64;
    let t836 = [vdupq_n_u32((t835) as u32); 2];
    let t837 = [vdupq_n_u32(((t835) >> 32) as u32); 2];
    let t838: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x15[0]), vget_low_u32(t836[0]))), vreinterpretq_u32_u64(vmull_high_u32(x15[0], t836[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x15[1]), vget_low_u32(t836[1]))), vreinterpretq_u32_u64(vmull_high_u32(x15[1], t836[1])))];
    let t839: [uint32x4_t; 4] = [vmulq_u32(x15[0], t836[0]), vmulq_u32(x15[1], t836[1]), vmlaq_u32(t838[0], x15[0], t837[0]), vmlaq_u32(t838[1], x15[1], t837[1])];
    x17 = t839;
    x18 = x12;
    x19 = x12;
    let mut t841 = live;
    loop {
        // Lanes part: each its own rounds.
        let t843: [uint32x4_t; 2] = [vcltq_u32(x19[0], x13[0]), vcltq_u32(x19[1], x13[1])];
        t841 = [vandq_u32(t841[0], t843[0]), vandq_u32(t841[1], t843[1])];
        if !(any(&t841)) { break; }
        let t845: [uint32x4_t; 2] = [vaddq_u32(x14[0], x19[0]), vaddq_u32(x14[1], x19[1])];
        let t846 = b17_ll_p(l, lid0, t841, t845, x8);
        let t847 = b15_r0r1sls_p(l, lid0, t841, arg0, arg1, x4, t846, 8u32);
        x20 = t847;
        let t848: u64 = x16;
        let t849 = [vdupq_n_u32((t848) as u32); 2];
        let t850 = [vdupq_n_u32(((t848) >> 32) as u32); 2];
        let t851: [uint32x4_t; 4] = [vorrq_u32(x20[0], t849[0]), vorrq_u32(x20[1], t849[1]), vorrq_u32(x20[2], t850[0]), vorrq_u32(x20[3], t850[1])];
        let t852: u64 = !(x16);
        let t853 = [vdupq_n_u32((t852) as u32); 2];
        let t854 = [vdupq_n_u32(((t852) >> 32) as u32); 2];
        let t855: [uint32x4_t; 4] = [vandq_u32(x17[0], t853[0]), vandq_u32(x17[1], t853[1]), vandq_u32(x17[2], t854[0]), vandq_u32(x17[3], t854[1])];
        let t856: [uint32x4_t; 2] = [vcltq_u32(t851[0], t855[0]), vcltq_u32(t851[1], t855[1])];
        let t857: [uint32x4_t; 4] = [vsubq_u32(t851[0], t855[0]), vsubq_u32(t851[1], t855[1]), vaddq_u32(vsubq_u32(t851[2], t855[2]), t856[0]), vaddq_u32(vsubq_u32(t851[3], t855[3]), t856[1])];
        x21 = t857;
        let t858: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x20[r]));
        let t859: [uint32x4_t; 4] = [vandq_u32(t858[0], x17[0]), vandq_u32(t858[1], x17[1]), vandq_u32(t858[2], x17[2]), vandq_u32(t858[3], x17[3])];
        let t860: [uint32x4_t; 4] = [veorq_u32(x20[0], x17[0]), veorq_u32(x20[1], x17[1]), veorq_u32(x20[2], x17[2]), veorq_u32(x20[3], x17[3])];
        let t861: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(t860[r]));
        let t862: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x21[r]));
        let t863: [uint32x4_t; 4] = [vandq_u32(t861[0], t862[0]), vandq_u32(t861[1], t862[1]), vandq_u32(t861[2], t862[2]), vandq_u32(t861[3], t862[3])];
        let t864: [uint32x4_t; 4] = [vorrq_u32(t859[0], t863[0]), vorrq_u32(t859[1], t863[1]), vorrq_u32(t859[2], t863[2]), vorrq_u32(t859[3], t863[3])];
        let t865: u64 = x16;
        let t866 = [vdupq_n_u32((t865) as u32); 2];
        let t867 = [vdupq_n_u32(((t865) >> 32) as u32); 2];
        let t868: [uint32x4_t; 4] = [vandq_u32(t866[0], t864[0]), vandq_u32(t866[1], t864[1]), vandq_u32(t867[0], t864[2]), vandq_u32(t867[1], t864[3])];
        x22 = t868;
        let t869: [uint32x4_t; 2] = [vsubq_u32(x13[0], x19[0]), vsubq_u32(x13[1], x19[1])];
        x23 = t869;
        let t870 = [vdupq_n_u32(8u32); 2];
        let t871: [uint32x4_t; 2] = [vcgeq_u32(x23[0], t870[0]), vcgeq_u32(x23[1], t870[1])];
        let mut t872: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t873: [uint32x4_t; 2] = [vandq_u32(t841[0], t871[0]), vandq_u32(t841[1], t871[1])];
        let t874: [uint32x4_t; 2] = [vbicq_u32(t841[0], t871[0]), vbicq_u32(t841[1], t871[1])];
        {
            t872 = [vbslq_u32(t873[0], x22[0], t872[0]), vbslq_u32(t873[1], x22[1], t872[1]), vbslq_u32(t873[0], x22[2], t872[2]), vbslq_u32(t873[1], x22[3], t872[3])];
        }
        {
            let t875: [uint32x4_t; 2] = [vmulq_u32(t870[0], x23[0]), vmulq_u32(t870[1], x23[1])];
            let t876: u64 = 1u64;
            let t877 = [vdupq_n_u32((t876) as u32); 2];
            let t878: [uint32x4_t; 2] = [t875[0], t875[1]];
            let t879: [uint32x4_t; 2] = [vandq_u32(t878[0], vdupq_n_u32(63)), vandq_u32(t878[1], vdupq_n_u32(63))];
            let t880: [uint32x4_t; 4] = [vshlq_u32(t877[0], vreinterpretq_s32_u32(t879[0])), vshlq_u32(t877[1], vreinterpretq_s32_u32(t879[1])), vshlq_u32(t877[0], vsubq_s32(vreinterpretq_s32_u32(t879[0]), vdupq_n_s32(32))), vshlq_u32(t877[1], vsubq_s32(vreinterpretq_s32_u32(t879[1]), vdupq_n_s32(32)))];
            let t881: u64 = 1u64;
            let t882 = [vdupq_n_u32((t881) as u32); 2];
            let t883: [uint32x4_t; 2] = [vcltq_u32(t880[0], t882[0]), vcltq_u32(t880[1], t882[1])];
            let t884: [uint32x4_t; 4] = [vsubq_u32(t880[0], t882[0]), vsubq_u32(t880[1], t882[1]), vaddq_u32(vsubq_u32(t880[2], vdupq_n_u32(0)), t883[0]), vaddq_u32(vsubq_u32(t880[3], vdupq_n_u32(0)), t883[1])];
            let t885: [uint32x4_t; 4] = [vandq_u32(x22[0], t884[0]), vandq_u32(x22[1], t884[1]), vandq_u32(x22[2], t884[2]), vandq_u32(x22[3], t884[3])];
            t872 = [vbslq_u32(t874[0], t885[0], t872[0]), vbslq_u32(t874[1], t885[1], t872[1]), vbslq_u32(t874[0], t885[2], t872[2]), vbslq_u32(t874[1], t885[3], t872[3])];
        }
        let t886 = b18_l_p(l, lid0, t841, t872);
        let t887: [uint32x4_t; 2] = [vaddq_u32(x18[0], t886[0]), vaddq_u32(x18[1], t886[1])];
        x18 = [vbslq_u32(t841[0], t887[0], x18[0]), vbslq_u32(t841[1], t887[1], x18[1])];
        let t888: [uint32x4_t; 2] = [vandq_u32(t841[0], live[0]), vandq_u32(t841[1], live[1])];
        let t889: [uint32x4_t; 2] = [vaddq_u32(x19[0], t870[0]), vaddq_u32(x19[1], t870[1])];
        x19 = t889;
    }
    x12 = x18;
    let t890 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    x24 = (((t890).wrapping_shr((((((4u32).wrapping_mul(x2)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32));
    let t891 = [vdupq_n_u32(1u32); 2];
    let t892: [uint32x4_t; 2] = [vsubq_u32(x10[0], t891[0]), vsubq_u32(x10[1], t891[1])];
    let t893: [uint32x4_t; 2] = [vmulq_u32(t825[0], t892[0]), vmulq_u32(t825[1], t892[1])];
    let t894: [uint32x4_t; 2] = [vaddq_u32(x7[0], t893[0]), vaddq_u32(x7[1], t893[1])];
    let t895 = b17_ll_p(l, lid0, live, t894, x8);
    let t896 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x4, t895, 2u32);
    let t897: [uint32x4_t; 2] = [t896[0], t896[1]];
    x25 = t897;
    let t898: [uint32x4_t; 2] = [vaddq_u32(x14[0], x25[0]), vaddq_u32(x14[1], x25[1])];
    let t899 = [vdupq_n_u32(x24); 2];
    let t900: [uint32x4_t; 2] = [vmulq_u32(x12[0], t899[0]), vmulq_u32(x12[1], t899[1])];
    let t901: [uint32x4_t; 2] = [vaddq_u32(t898[0], t900[0]), vaddq_u32(t898[1], t900[1])];
    let t902 = b17_ll_p(l, lid0, live, t901, x8);
    x26 = t902;
    let t903 = b15_r0r1sls_p(l, lid0, live, arg0, arg1, x4, x26, 8u32);
    let t904 = b8_s_p(l, lid0, live, x24);
    let t905: u64 = t904;
    let t906 = [vdupq_n_u32((t905) as u32); 2];
    let t907 = [vdupq_n_u32(((t905) >> 32) as u32); 2];
    let t908: [uint32x4_t; 4] = [vandq_u32(t903[0], t906[0]), vandq_u32(t903[1], t906[1]), vandq_u32(t903[2], t907[0]), vandq_u32(t903[3], t907[1])];
    res = t908;
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
    let t909 = b11_r0r1s_p(l, lid0, live, arg0, arg1, (20u32).wrapping_add((4u32).wrapping_mul(x2)));
    res = t909;
    return res;
}

#[inline(always)]
unsafe fn b12_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t910 = b11_r0r1s_f(l, lid0, arg0, arg1, (20u32).wrapping_add((4u32).wrapping_mul(x2)));
    res = t910;
    return res;
}

#[inline(always)]
unsafe fn b13_ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x1: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t911: u32;
    if ((x0) < (x1)) {
        t911 = x0;
    } else {
        t911 = x1;
    }
    res = t911;
    return res;
}

#[inline(always)]
unsafe fn b13_ss_f(l: &Launch, lid0: u32, x0: u32, x1: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t912: u32;
    if ((x0) < (x1)) {
        t912 = x0;
    } else {
        t912 = x1;
    }
    res = t912;
    return res;
}

#[inline(always)]
unsafe fn b13_ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x1: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t913 = [vdupq_n_u32(x1); 2];
    let t914: [uint32x4_t; 2] = [vcltq_u32(x0[0], t913[0]), vcltq_u32(x0[1], t913[1])];
    let mut t915: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t916: [uint32x4_t; 2] = [vandq_u32(live[0], t914[0]), vandq_u32(live[1], t914[1])];
    let t917: [uint32x4_t; 2] = [vbicq_u32(live[0], t914[0]), vbicq_u32(live[1], t914[1])];
    {
        t915 = [vbslq_u32(t916[0], x0[0], t915[0]), vbslq_u32(t916[1], x0[1], t915[1])];
    }
    {
        let t918 = [vdupq_n_u32(x1); 2];
        t915 = [vbslq_u32(t917[0], t918[0], t915[0]), vbslq_u32(t917[1], t918[1], t915[1])];
    }
    res = t915;
    return res;
}

#[inline(always)]
unsafe fn b13_ls_f(l: &Launch, lid0: u32, x0: [uint32x4_t; 2], x1: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t919 = [vdupq_n_u32(x1); 2];
    let t920: [uint32x4_t; 2] = [vcltq_u32(x0[0], t919[0]), vcltq_u32(x0[1], t919[1])];
    let mut t921: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t922: [uint32x4_t; 2] = [vmvnq_u32(t920[0]), vmvnq_u32(t920[1])];
    {
        t921 = [vbslq_u32(t920[0], x0[0], t921[0]), vbslq_u32(t920[1], x0[1], t921[1])];
    }
    {
        let t923 = [vdupq_n_u32(x1); 2];
        t921 = [vbslq_u32(t922[0], t923[0], t921[0]), vbslq_u32(t922[1], t923[1], t921[1])];
    }
    res = t921;
    return res;
}

#[inline(always)]
unsafe fn b14_r0sns_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1: u32, x2f: u32, x2s: u32, x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t924 = b19_r0an1s_p(l, lid0, live, arg0, ((((x1) as u64)) as usize as *const u8), x2f, x2s, x3);
    x4 = t924;
    let t925: [uint32x4_t; 4];
    if ((x3) >= (8u32)) {
        t925 = x4;
    } else {
        let t926 = b20_s_p(l, lid0, live, (8u32).wrapping_mul(x3));
        let t927: u64 = t926;
        let t928 = [vdupq_n_u32((t927) as u32); 2];
        let t929 = [vdupq_n_u32(((t927) >> 32) as u32); 2];
        let t930: [uint32x4_t; 4] = [vandq_u32(x4[0], t928[0]), vandq_u32(x4[1], t928[1]), vandq_u32(x4[2], t929[0]), vandq_u32(x4[3], t929[1])];
        t925 = t930;
    }
    res = t925;
    return res;
}

#[inline(always)]
unsafe fn b14_r0sns_f(l: &Launch, lid0: u32, arg0: *const u8, x1: u32, x2f: u32, x2s: u32, x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t931 = b19_r0an1s_f(l, lid0, arg0, ((((x1) as u64)) as usize as *const u8), x2f, x2s, x3);
    x4 = t931;
    let t932: [uint32x4_t; 4];
    if ((x3) >= (8u32)) {
        t932 = x4;
    } else {
        let t933 = b20_s_f(l, lid0, (8u32).wrapping_mul(x3));
        let t934: u64 = t933;
        let t935 = [vdupq_n_u32((t934) as u32); 2];
        let t936 = [vdupq_n_u32(((t934) >> 32) as u32); 2];
        let t937: [uint32x4_t; 4] = [vandq_u32(x4[0], t935[0]), vandq_u32(x4[1], t935[1]), vandq_u32(x4[2], t936[0]), vandq_u32(x4[3], t936[1])];
        t932 = t937;
    }
    res = t932;
    return res;
}

#[inline(always)]
unsafe fn b14_r0lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1: [uint32x4_t; 2], x2: [uint32x4_t; 2], x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t938: [uint32x4_t; 2] = [vaddq_u32(x1[0], x2[0]), vaddq_u32(x1[1], x2[1])];
    let t939: [uint32x4_t; 2] = [vcltq_u32(t938[0], x1[0]), vcltq_u32(t938[1], x1[1])];
    let t940: [uint32x4_t; 4] = [t938[0], t938[1], vshrq_n_u32::<31>(t939[0]), vshrq_n_u32::<31>(t939[1])];
    let t941 = b19_r0ls_p(l, lid0, live, arg0, t940, x3);
    x4 = t941;
    let t942: [uint32x4_t; 4];
    if ((x3) >= (8u32)) {
        t942 = x4;
    } else {
        let t943 = b20_s_p(l, lid0, live, (8u32).wrapping_mul(x3));
        let t944: u64 = t943;
        let t945 = [vdupq_n_u32((t944) as u32); 2];
        let t946 = [vdupq_n_u32(((t944) >> 32) as u32); 2];
        let t947: [uint32x4_t; 4] = [vandq_u32(x4[0], t945[0]), vandq_u32(x4[1], t945[1]), vandq_u32(x4[2], t946[0]), vandq_u32(x4[3], t946[1])];
        t942 = t947;
    }
    res = t942;
    return res;
}

#[inline(always)]
unsafe fn b14_r0lls_f(l: &Launch, lid0: u32, arg0: *const u8, x1: [uint32x4_t; 2], x2: [uint32x4_t; 2], x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t948: [uint32x4_t; 2] = [vaddq_u32(x1[0], x2[0]), vaddq_u32(x1[1], x2[1])];
    let t949: [uint32x4_t; 2] = [vcltq_u32(t948[0], x1[0]), vcltq_u32(t948[1], x1[1])];
    let t950: [uint32x4_t; 4] = [t948[0], t948[1], vshrq_n_u32::<31>(t949[0]), vshrq_n_u32::<31>(t949[1])];
    let t951 = b19_r0ls_f(l, lid0, arg0, t950, x3);
    x4 = t951;
    let t952: [uint32x4_t; 4];
    if ((x3) >= (8u32)) {
        t952 = x4;
    } else {
        let t953 = b20_s_f(l, lid0, (8u32).wrapping_mul(x3));
        let t954: u64 = t953;
        let t955 = [vdupq_n_u32((t954) as u32); 2];
        let t956 = [vdupq_n_u32(((t954) >> 32) as u32); 2];
        let t957: [uint32x4_t; 4] = [vandq_u32(x4[0], t955[0]), vandq_u32(x4[1], t955[1]), vandq_u32(x4[2], t956[0]), vandq_u32(x4[3], t956[1])];
        t952 = t957;
    }
    res = t952;
    return res;
}

#[inline(always)]
unsafe fn b15_r0r1sss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: u32, x4: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t958 = b21_r0sss_p(l, lid0, live, arg0, x2, x3, x4);
    res = t958;
    return res;
}

#[inline(always)]
unsafe fn b15_r0r1sss_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32, x3: u32, x4: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t959 = b21_r0sss_f(l, lid0, arg0, x2, x3, x4);
    res = t959;
    return res;
}

#[inline(always)]
unsafe fn b15_r0r1sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: [uint32x4_t; 2], x4: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t960 = b21_r0sls_p(l, lid0, live, arg0, x2, x3, x4);
    res = t960;
    return res;
}

#[inline(always)]
unsafe fn b16_ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x1: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t961: u32;
    if ((x0) < (x1)) {
        t961 = x0;
    } else {
        t961 = x1;
    }
    res = t961;
    return res;
}

#[inline(always)]
unsafe fn b16_ss_f(l: &Launch, lid0: u32, x0: u32, x1: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t962: u32;
    if ((x0) < (x1)) {
        t962 = x0;
    } else {
        t962 = x1;
    }
    res = t962;
    return res;
}

#[inline(always)]
unsafe fn b16_sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x1: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t963 = [vdupq_n_u32(x0); 2];
    let t964: [uint32x4_t; 2] = [vcltq_u32(t963[0], x1[0]), vcltq_u32(t963[1], x1[1])];
    let mut t965: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t966: [uint32x4_t; 2] = [vandq_u32(live[0], t964[0]), vandq_u32(live[1], t964[1])];
    let t967: [uint32x4_t; 2] = [vbicq_u32(live[0], t964[0]), vbicq_u32(live[1], t964[1])];
    {
        let t968 = [vdupq_n_u32(x0); 2];
        t965 = [vbslq_u32(t966[0], t968[0], t965[0]), vbslq_u32(t966[1], t968[1], t965[1])];
    }
    {
        t965 = [vbslq_u32(t967[0], x1[0], t965[0]), vbslq_u32(t967[1], x1[1], t965[1])];
    }
    res = t965;
    return res;
}

#[inline(always)]
unsafe fn b17_ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x1: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t969: u32;
    if ((x0) < (x1)) {
        t969 = x0;
    } else {
        t969 = x1;
    }
    res = t969;
    return res;
}

#[inline(always)]
unsafe fn b17_ss_f(l: &Launch, lid0: u32, x0: u32, x1: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t970: u32;
    if ((x0) < (x1)) {
        t970 = x0;
    } else {
        t970 = x1;
    }
    res = t970;
    return res;
}

#[inline(always)]
unsafe fn b17_ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x1: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t971 = [vdupq_n_u32(x1); 2];
    let t972: [uint32x4_t; 2] = [vcltq_u32(x0[0], t971[0]), vcltq_u32(x0[1], t971[1])];
    let mut t973: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t974: [uint32x4_t; 2] = [vandq_u32(live[0], t972[0]), vandq_u32(live[1], t972[1])];
    let t975: [uint32x4_t; 2] = [vbicq_u32(live[0], t972[0]), vbicq_u32(live[1], t972[1])];
    {
        t973 = [vbslq_u32(t974[0], x0[0], t973[0]), vbslq_u32(t974[1], x0[1], t973[1])];
    }
    {
        let t976 = [vdupq_n_u32(x1); 2];
        t973 = [vbslq_u32(t975[0], t976[0], t973[0]), vbslq_u32(t975[1], t976[1], t973[1])];
    }
    res = t973;
    return res;
}

#[inline(always)]
unsafe fn b17_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x1: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t977: [uint32x4_t; 2] = [vcltq_u32(x0[0], x1[0]), vcltq_u32(x0[1], x1[1])];
    let mut t978: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t979: [uint32x4_t; 2] = [vandq_u32(live[0], t977[0]), vandq_u32(live[1], t977[1])];
    let t980: [uint32x4_t; 2] = [vbicq_u32(live[0], t977[0]), vbicq_u32(live[1], t977[1])];
    {
        t978 = [vbslq_u32(t979[0], x0[0], t978[0]), vbslq_u32(t979[1], x0[1], t978[1])];
    }
    {
        t978 = [vbslq_u32(t980[0], x1[0], t978[0]), vbslq_u32(t980[1], x1[1], t978[1])];
    }
    res = t978;
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
    let t981: [uint32x4_t; 2] = [vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[0])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[2]))))), vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[1])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[3])))))];
    res = t981;
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
    if ((x2) == (4u32)) {
        res = (((arg0.wrapping_add(x1 as usize) as *const u32).read_unaligned()) as u64);
        return res;
    }
    if ((x2) == (2u32)) {
        res = (((arg0.wrapping_add(x1 as usize) as *const u16).read_unaligned()) as u64);
        return res;
    }
    if ((x2) == (1u32)) {
        res = (((arg0.wrapping_add(x1 as usize) as *const u8).read_unaligned()) as u64);
        return res;
    }
    x3 = ((arg0.wrapping_add(x1 as usize)) as usize).wrapping_sub(((((((((((((arg0) as usize as u64)).wrapping_add(x1)) & (7u64))) as u32)) as u64)) as u64 as usize as *const u8)) as usize) as *const u8;
    x4 = ((((((((arg0) as usize as u64)).wrapping_add(x1)) & (7u64))) as u32)).wrapping_mul(8u32);
    x5 = ((x3 as *const u64).read_unaligned()).wrapping_shr((((x4) as u64)) as u32);
    let mut t982: bool = ((x4) != (0u32));
    if t982 {
        t982 = (((x4).wrapping_add((x2).wrapping_mul(8u32))) > (64u32));
    }
    let t983: u64;
    if t982 {
        t983 = ((x5) | (((x3.wrapping_add((1u64).wrapping_mul(8u64) as usize) as *const u64).read_unaligned()).wrapping_shl(((((64u32).wrapping_sub(x4)) as u64)) as u32)));
    } else {
        t983 = x5;
    }
    res = t983;
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
    if ((x2) == (4u32)) {
        res = (((arg0.wrapping_add(x1 as usize) as *const u32).read_unaligned()) as u64);
        return res;
    }
    if ((x2) == (2u32)) {
        res = (((arg0.wrapping_add(x1 as usize) as *const u16).read_unaligned()) as u64);
        return res;
    }
    if ((x2) == (1u32)) {
        res = (((arg0.wrapping_add(x1 as usize) as *const u8).read_unaligned()) as u64);
        return res;
    }
    x3 = ((arg0.wrapping_add(x1 as usize)) as usize).wrapping_sub(((((((((((((arg0) as usize as u64)).wrapping_add(x1)) & (7u64))) as u32)) as u64)) as u64 as usize as *const u8)) as usize) as *const u8;
    x4 = ((((((((arg0) as usize as u64)).wrapping_add(x1)) & (7u64))) as u32)).wrapping_mul(8u32);
    x5 = ((x3 as *const u64).read_unaligned()).wrapping_shr((((x4) as u64)) as u32);
    let mut t984: bool = ((x4) != (0u32));
    if t984 {
        t984 = (((x4).wrapping_add((x2).wrapping_mul(8u32))) > (64u32));
    }
    let t985: u64;
    if t984 {
        t985 = ((x5) | (((x3.wrapping_add((1u64).wrapping_mul(8u64) as usize) as *const u64).read_unaligned()).wrapping_shl(((((64u32).wrapping_sub(x4)) as u64)) as u32)));
    } else {
        t985 = x5;
    }
    res = t985;
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
        let t986: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
        let t987: [uint32x4_t; 4] = [vandq_u32(x1[0], t986[0]), vandq_u32(x1[1], t986[1]), vandq_u32(x1[2], t986[2]), vandq_u32(x1[3], t986[3])];
        let t988: u64 = ((arg0) as usize as u64);
        let t989 = [vdupq_n_u32((t988) as u32); 2];
        let t990 = [vdupq_n_u32(((t988) >> 32) as u32); 2];
        let t991: [uint32x4_t; 2] = [vaddq_u32(t989[0], t987[0]), vaddq_u32(t989[1], t987[1])];
        let t992: [uint32x4_t; 2] = [vcltq_u32(t991[0], t989[0]), vcltq_u32(t991[1], t989[1])];
        let t993: [uint32x4_t; 4] = [t991[0], t991[1], vsubq_u32(vaddq_u32(t990[0], t987[2]), t992[0]), vsubq_u32(vaddq_u32(t990[1], t987[3]), t992[1])];
        let mut t994: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t993[2]) as u64) << 32 | vgetq_lane_u32::<0>(t993[0]) as u64) as usize as *const u64).read_unaligned(); t994[0] = vsetq_lane_u32::<0>(v as u32, t994[0]); t994[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t994[2]); }
        { let v = (((vgetq_lane_u32::<1>(t993[2]) as u64) << 32 | vgetq_lane_u32::<1>(t993[0]) as u64) as usize as *const u64).read_unaligned(); t994[0] = vsetq_lane_u32::<1>(v as u32, t994[0]); t994[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t994[2]); }
        { let v = (((vgetq_lane_u32::<2>(t993[2]) as u64) << 32 | vgetq_lane_u32::<2>(t993[0]) as u64) as usize as *const u64).read_unaligned(); t994[0] = vsetq_lane_u32::<2>(v as u32, t994[0]); t994[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t994[2]); }
        { let v = (((vgetq_lane_u32::<3>(t993[2]) as u64) << 32 | vgetq_lane_u32::<3>(t993[0]) as u64) as usize as *const u64).read_unaligned(); t994[0] = vsetq_lane_u32::<3>(v as u32, t994[0]); t994[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t994[2]); }
        { let v = (((vgetq_lane_u32::<0>(t993[3]) as u64) << 32 | vgetq_lane_u32::<0>(t993[1]) as u64) as usize as *const u64).read_unaligned(); t994[1] = vsetq_lane_u32::<0>(v as u32, t994[1]); t994[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t994[3]); }
        { let v = (((vgetq_lane_u32::<1>(t993[3]) as u64) << 32 | vgetq_lane_u32::<1>(t993[1]) as u64) as usize as *const u64).read_unaligned(); t994[1] = vsetq_lane_u32::<1>(v as u32, t994[1]); t994[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t994[3]); }
        { let v = (((vgetq_lane_u32::<2>(t993[3]) as u64) << 32 | vgetq_lane_u32::<2>(t993[1]) as u64) as usize as *const u64).read_unaligned(); t994[1] = vsetq_lane_u32::<2>(v as u32, t994[1]); t994[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t994[3]); }
        { let v = (((vgetq_lane_u32::<3>(t993[3]) as u64) << 32 | vgetq_lane_u32::<3>(t993[1]) as u64) as usize as *const u64).read_unaligned(); t994[1] = vsetq_lane_u32::<3>(v as u32, t994[1]); t994[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t994[3]); }
        res = t994;
        return res;
    }
    if ((x2) == (4u32)) {
        let t995: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
        let t996: [uint32x4_t; 4] = [vandq_u32(x1[0], t995[0]), vandq_u32(x1[1], t995[1]), vandq_u32(x1[2], t995[2]), vandq_u32(x1[3], t995[3])];
        let t997: u64 = ((arg0) as usize as u64);
        let t998 = [vdupq_n_u32((t997) as u32); 2];
        let t999 = [vdupq_n_u32(((t997) >> 32) as u32); 2];
        let t1000: [uint32x4_t; 2] = [vaddq_u32(t998[0], t996[0]), vaddq_u32(t998[1], t996[1])];
        let t1001: [uint32x4_t; 2] = [vcltq_u32(t1000[0], t998[0]), vcltq_u32(t1000[1], t998[1])];
        let t1002: [uint32x4_t; 4] = [t1000[0], t1000[1], vsubq_u32(vaddq_u32(t999[0], t996[2]), t1001[0]), vsubq_u32(vaddq_u32(t999[1], t996[3]), t1001[1])];
        let mut t1003: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t1003[0] = vsetq_lane_u32::<0>((((vgetq_lane_u32::<0>(t1002[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1002[0]) as u64) as usize as *const u32).read_unaligned() as u32, t1003[0]);
        t1003[0] = vsetq_lane_u32::<1>((((vgetq_lane_u32::<1>(t1002[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1002[0]) as u64) as usize as *const u32).read_unaligned() as u32, t1003[0]);
        t1003[0] = vsetq_lane_u32::<2>((((vgetq_lane_u32::<2>(t1002[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1002[0]) as u64) as usize as *const u32).read_unaligned() as u32, t1003[0]);
        t1003[0] = vsetq_lane_u32::<3>((((vgetq_lane_u32::<3>(t1002[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1002[0]) as u64) as usize as *const u32).read_unaligned() as u32, t1003[0]);
        t1003[1] = vsetq_lane_u32::<0>((((vgetq_lane_u32::<0>(t1002[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1002[1]) as u64) as usize as *const u32).read_unaligned() as u32, t1003[1]);
        t1003[1] = vsetq_lane_u32::<1>((((vgetq_lane_u32::<1>(t1002[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1002[1]) as u64) as usize as *const u32).read_unaligned() as u32, t1003[1]);
        t1003[1] = vsetq_lane_u32::<2>((((vgetq_lane_u32::<2>(t1002[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1002[1]) as u64) as usize as *const u32).read_unaligned() as u32, t1003[1]);
        t1003[1] = vsetq_lane_u32::<3>((((vgetq_lane_u32::<3>(t1002[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1002[1]) as u64) as usize as *const u32).read_unaligned() as u32, t1003[1]);
        let t1004: [uint32x4_t; 4] = [t1003[0], t1003[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1004;
        return res;
    }
    if ((x2) == (2u32)) {
        let t1005: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
        let t1006: [uint32x4_t; 4] = [vandq_u32(x1[0], t1005[0]), vandq_u32(x1[1], t1005[1]), vandq_u32(x1[2], t1005[2]), vandq_u32(x1[3], t1005[3])];
        let t1007: u64 = ((arg0) as usize as u64);
        let t1008 = [vdupq_n_u32((t1007) as u32); 2];
        let t1009 = [vdupq_n_u32(((t1007) >> 32) as u32); 2];
        let t1010: [uint32x4_t; 2] = [vaddq_u32(t1008[0], t1006[0]), vaddq_u32(t1008[1], t1006[1])];
        let t1011: [uint32x4_t; 2] = [vcltq_u32(t1010[0], t1008[0]), vcltq_u32(t1010[1], t1008[1])];
        let t1012: [uint32x4_t; 4] = [t1010[0], t1010[1], vsubq_u32(vaddq_u32(t1009[0], t1006[2]), t1011[0]), vsubq_u32(vaddq_u32(t1009[1], t1006[3]), t1011[1])];
        let mut t1013: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t1013[0] = vsetq_lane_u32::<0>((((vgetq_lane_u32::<0>(t1012[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1012[0]) as u64) as usize as *const u16).read_unaligned() as u32, t1013[0]);
        t1013[0] = vsetq_lane_u32::<1>((((vgetq_lane_u32::<1>(t1012[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1012[0]) as u64) as usize as *const u16).read_unaligned() as u32, t1013[0]);
        t1013[0] = vsetq_lane_u32::<2>((((vgetq_lane_u32::<2>(t1012[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1012[0]) as u64) as usize as *const u16).read_unaligned() as u32, t1013[0]);
        t1013[0] = vsetq_lane_u32::<3>((((vgetq_lane_u32::<3>(t1012[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1012[0]) as u64) as usize as *const u16).read_unaligned() as u32, t1013[0]);
        t1013[1] = vsetq_lane_u32::<0>((((vgetq_lane_u32::<0>(t1012[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1012[1]) as u64) as usize as *const u16).read_unaligned() as u32, t1013[1]);
        t1013[1] = vsetq_lane_u32::<1>((((vgetq_lane_u32::<1>(t1012[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1012[1]) as u64) as usize as *const u16).read_unaligned() as u32, t1013[1]);
        t1013[1] = vsetq_lane_u32::<2>((((vgetq_lane_u32::<2>(t1012[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1012[1]) as u64) as usize as *const u16).read_unaligned() as u32, t1013[1]);
        t1013[1] = vsetq_lane_u32::<3>((((vgetq_lane_u32::<3>(t1012[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1012[1]) as u64) as usize as *const u16).read_unaligned() as u32, t1013[1]);
        let t1014: [uint32x4_t; 4] = [t1013[0], t1013[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1014;
        return res;
    }
    if ((x2) == (1u32)) {
        let t1015: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
        let t1016: [uint32x4_t; 4] = [vandq_u32(x1[0], t1015[0]), vandq_u32(x1[1], t1015[1]), vandq_u32(x1[2], t1015[2]), vandq_u32(x1[3], t1015[3])];
        let t1017: u64 = ((arg0) as usize as u64);
        let t1018 = [vdupq_n_u32((t1017) as u32); 2];
        let t1019 = [vdupq_n_u32(((t1017) >> 32) as u32); 2];
        let t1020: [uint32x4_t; 2] = [vaddq_u32(t1018[0], t1016[0]), vaddq_u32(t1018[1], t1016[1])];
        let t1021: [uint32x4_t; 2] = [vcltq_u32(t1020[0], t1018[0]), vcltq_u32(t1020[1], t1018[1])];
        let t1022: [uint32x4_t; 4] = [t1020[0], t1020[1], vsubq_u32(vaddq_u32(t1019[0], t1016[2]), t1021[0]), vsubq_u32(vaddq_u32(t1019[1], t1016[3]), t1021[1])];
        let mut t1023: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t1023[0] = vsetq_lane_u32::<0>((((vgetq_lane_u32::<0>(t1022[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1022[0]) as u64) as usize as *const u8).read_unaligned() as u32, t1023[0]);
        t1023[0] = vsetq_lane_u32::<1>((((vgetq_lane_u32::<1>(t1022[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1022[0]) as u64) as usize as *const u8).read_unaligned() as u32, t1023[0]);
        t1023[0] = vsetq_lane_u32::<2>((((vgetq_lane_u32::<2>(t1022[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1022[0]) as u64) as usize as *const u8).read_unaligned() as u32, t1023[0]);
        t1023[0] = vsetq_lane_u32::<3>((((vgetq_lane_u32::<3>(t1022[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1022[0]) as u64) as usize as *const u8).read_unaligned() as u32, t1023[0]);
        t1023[1] = vsetq_lane_u32::<0>((((vgetq_lane_u32::<0>(t1022[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1022[1]) as u64) as usize as *const u8).read_unaligned() as u32, t1023[1]);
        t1023[1] = vsetq_lane_u32::<1>((((vgetq_lane_u32::<1>(t1022[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1022[1]) as u64) as usize as *const u8).read_unaligned() as u32, t1023[1]);
        t1023[1] = vsetq_lane_u32::<2>((((vgetq_lane_u32::<2>(t1022[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1022[1]) as u64) as usize as *const u8).read_unaligned() as u32, t1023[1]);
        t1023[1] = vsetq_lane_u32::<3>((((vgetq_lane_u32::<3>(t1022[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1022[1]) as u64) as usize as *const u8).read_unaligned() as u32, t1023[1]);
        let t1024: [uint32x4_t; 4] = [t1023[0], t1023[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1024;
        return res;
    }
    let t1025: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
    let t1026: [uint32x4_t; 4] = [vandq_u32(x1[0], t1025[0]), vandq_u32(x1[1], t1025[1]), vandq_u32(x1[2], t1025[2]), vandq_u32(x1[3], t1025[3])];
    let t1027: u64 = ((arg0) as usize as u64);
    let t1028 = [vdupq_n_u32((t1027) as u32); 2];
    let t1029 = [vdupq_n_u32(((t1027) >> 32) as u32); 2];
    let t1030: [uint32x4_t; 2] = [vaddq_u32(t1028[0], t1026[0]), vaddq_u32(t1028[1], t1026[1])];
    let t1031: [uint32x4_t; 2] = [vcltq_u32(t1030[0], t1028[0]), vcltq_u32(t1030[1], t1028[1])];
    let t1032: [uint32x4_t; 4] = [t1030[0], t1030[1], vsubq_u32(vaddq_u32(t1029[0], t1026[2]), t1031[0]), vsubq_u32(vaddq_u32(t1029[1], t1026[3]), t1031[1])];
    let t1033: u64 = ((arg0) as usize as u64);
    let t1034 = [vdupq_n_u32((t1033) as u32); 2];
    let t1035 = [vdupq_n_u32(((t1033) >> 32) as u32); 2];
    let t1036: [uint32x4_t; 2] = [vaddq_u32(t1034[0], x1[0]), vaddq_u32(t1034[1], x1[1])];
    let t1037: [uint32x4_t; 2] = [vcltq_u32(t1036[0], t1034[0]), vcltq_u32(t1036[1], t1034[1])];
    let t1038: [uint32x4_t; 4] = [t1036[0], t1036[1], vsubq_u32(vaddq_u32(t1035[0], x1[2]), t1037[0]), vsubq_u32(vaddq_u32(t1035[1], x1[3]), t1037[1])];
    let t1039: u64 = 7u64;
    let t1040 = [vdupq_n_u32((t1039) as u32); 2];
    let t1041: [uint32x4_t; 2] = [vandq_u32(t1038[0], t1040[0]), vandq_u32(t1038[1], t1040[1])];
    let t1042: [uint32x4_t; 2] = [vcltq_u32(t1032[0], t1041[0]), vcltq_u32(t1032[1], t1041[1])];
    let t1043: [uint32x4_t; 4] = [vsubq_u32(t1032[0], t1041[0]), vsubq_u32(t1032[1], t1041[1]), vaddq_u32(vsubq_u32(t1032[2], vdupq_n_u32(0)), t1042[0]), vaddq_u32(vsubq_u32(t1032[3], vdupq_n_u32(0)), t1042[1])];
    x3 = t1043;
    let t1044: u64 = ((arg0) as usize as u64);
    let t1045 = [vdupq_n_u32((t1044) as u32); 2];
    let t1046 = [vdupq_n_u32(((t1044) >> 32) as u32); 2];
    let t1047: [uint32x4_t; 2] = [vaddq_u32(t1045[0], x1[0]), vaddq_u32(t1045[1], x1[1])];
    let t1048: [uint32x4_t; 2] = [vcltq_u32(t1047[0], t1045[0]), vcltq_u32(t1047[1], t1045[1])];
    let t1049: [uint32x4_t; 4] = [t1047[0], t1047[1], vsubq_u32(vaddq_u32(t1046[0], x1[2]), t1048[0]), vsubq_u32(vaddq_u32(t1046[1], x1[3]), t1048[1])];
    let t1050: u64 = 7u64;
    let t1051 = [vdupq_n_u32((t1050) as u32); 2];
    let t1052: [uint32x4_t; 2] = [vandq_u32(t1049[0], t1051[0]), vandq_u32(t1049[1], t1051[1])];
    let t1053 = [vdupq_n_u32(8u32); 2];
    let t1054: [uint32x4_t; 2] = [vmulq_u32(t1052[0], t1053[0]), vmulq_u32(t1052[1], t1053[1])];
    x4 = t1054;
    let mut t1055: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x3[2]) as u64) << 32 | vgetq_lane_u32::<0>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1055[0] = vsetq_lane_u32::<0>(v as u32, t1055[0]); t1055[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1055[2]); }
    { let v = (((vgetq_lane_u32::<1>(x3[2]) as u64) << 32 | vgetq_lane_u32::<1>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1055[0] = vsetq_lane_u32::<1>(v as u32, t1055[0]); t1055[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1055[2]); }
    { let v = (((vgetq_lane_u32::<2>(x3[2]) as u64) << 32 | vgetq_lane_u32::<2>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1055[0] = vsetq_lane_u32::<2>(v as u32, t1055[0]); t1055[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1055[2]); }
    { let v = (((vgetq_lane_u32::<3>(x3[2]) as u64) << 32 | vgetq_lane_u32::<3>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1055[0] = vsetq_lane_u32::<3>(v as u32, t1055[0]); t1055[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1055[2]); }
    { let v = (((vgetq_lane_u32::<0>(x3[3]) as u64) << 32 | vgetq_lane_u32::<0>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1055[1] = vsetq_lane_u32::<0>(v as u32, t1055[1]); t1055[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1055[3]); }
    { let v = (((vgetq_lane_u32::<1>(x3[3]) as u64) << 32 | vgetq_lane_u32::<1>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1055[1] = vsetq_lane_u32::<1>(v as u32, t1055[1]); t1055[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1055[3]); }
    { let v = (((vgetq_lane_u32::<2>(x3[3]) as u64) << 32 | vgetq_lane_u32::<2>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1055[1] = vsetq_lane_u32::<2>(v as u32, t1055[1]); t1055[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1055[3]); }
    { let v = (((vgetq_lane_u32::<3>(x3[3]) as u64) << 32 | vgetq_lane_u32::<3>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1055[1] = vsetq_lane_u32::<3>(v as u32, t1055[1]); t1055[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1055[3]); }
    let t1056: [uint32x4_t; 2] = [x4[0], x4[1]];
    let t1057: [uint32x4_t; 2] = [vandq_u32(t1056[0], vdupq_n_u32(63)), vandq_u32(t1056[1], vdupq_n_u32(63))];
    let t1058: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t1055[0], vnegq_s32(vreinterpretq_s32_u32(t1057[0]))), vshlq_u32(t1055[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1057[0])))), vorrq_u32(vshlq_u32(t1055[1], vnegq_s32(vreinterpretq_s32_u32(t1057[1]))), vshlq_u32(t1055[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1057[1])))), vshlq_u32(t1055[2], vnegq_s32(vreinterpretq_s32_u32(t1057[0]))), vshlq_u32(t1055[3], vnegq_s32(vreinterpretq_s32_u32(t1057[1])))];
    x5 = t1058;
    let t1059 = [vdupq_n_u32(0u32); 2];
    let t1060: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t1059[0])), vmvnq_u32(vceqq_u32(x4[1], t1059[1]))];
    let t1061 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
    let t1062: [uint32x4_t; 2] = [vaddq_u32(x4[0], t1061[0]), vaddq_u32(x4[1], t1061[1])];
    let t1063 = [vdupq_n_u32(64u32); 2];
    let t1064: [uint32x4_t; 2] = [vcgtq_u32(t1062[0], t1063[0]), vcgtq_u32(t1062[1], t1063[1])];
    let t1065: [uint32x4_t; 2] = [vandq_u32(t1060[0], t1064[0]), vandq_u32(t1060[1], t1064[1])];
    let mut t1066: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1067: [uint32x4_t; 2] = [vandq_u32(live[0], t1065[0]), vandq_u32(live[1], t1065[1])];
    let t1068: [uint32x4_t; 2] = [vbicq_u32(live[0], t1065[0]), vbicq_u32(live[1], t1065[1])];
    if any(&t1067) {
        let t1069: u64 = (1u64).wrapping_mul(8u64);
        let t1070 = [vdupq_n_u32((t1069) as u32); 2];
        let t1071 = [vdupq_n_u32(((t1069) >> 32) as u32); 2];
        let t1072: [uint32x4_t; 2] = [vaddq_u32(x3[0], t1070[0]), vaddq_u32(x3[1], t1070[1])];
        let t1073: [uint32x4_t; 2] = [vcltq_u32(t1072[0], x3[0]), vcltq_u32(t1072[1], x3[1])];
        let t1074: [uint32x4_t; 4] = [t1072[0], t1072[1], vsubq_u32(vaddq_u32(x3[2], t1071[0]), t1073[0]), vsubq_u32(vaddq_u32(x3[3], t1071[1]), t1073[1])];
        let mut t1075: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1074[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1074[0]) as u64) as usize as *const u64).read_unaligned(); t1075[0] = vsetq_lane_u32::<0>(v as u32, t1075[0]); t1075[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1075[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1074[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1074[0]) as u64) as usize as *const u64).read_unaligned(); t1075[0] = vsetq_lane_u32::<1>(v as u32, t1075[0]); t1075[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1075[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1074[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1074[0]) as u64) as usize as *const u64).read_unaligned(); t1075[0] = vsetq_lane_u32::<2>(v as u32, t1075[0]); t1075[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1075[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1074[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1074[0]) as u64) as usize as *const u64).read_unaligned(); t1075[0] = vsetq_lane_u32::<3>(v as u32, t1075[0]); t1075[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1075[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1074[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1074[1]) as u64) as usize as *const u64).read_unaligned(); t1075[1] = vsetq_lane_u32::<0>(v as u32, t1075[1]); t1075[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1075[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1074[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1074[1]) as u64) as usize as *const u64).read_unaligned(); t1075[1] = vsetq_lane_u32::<1>(v as u32, t1075[1]); t1075[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1075[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1074[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1074[1]) as u64) as usize as *const u64).read_unaligned(); t1075[1] = vsetq_lane_u32::<2>(v as u32, t1075[1]); t1075[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1075[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1074[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1074[1]) as u64) as usize as *const u64).read_unaligned(); t1075[1] = vsetq_lane_u32::<3>(v as u32, t1075[1]); t1075[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1075[3]); }
        let t1076: [uint32x4_t; 2] = [vsubq_u32(t1063[0], x4[0]), vsubq_u32(t1063[1], x4[1])];
        let t1077: [uint32x4_t; 2] = [t1076[0], t1076[1]];
        let t1078: [uint32x4_t; 2] = [vandq_u32(t1077[0], vdupq_n_u32(63)), vandq_u32(t1077[1], vdupq_n_u32(63))];
        let t1079: [uint32x4_t; 4] = [vshlq_u32(t1075[0], vreinterpretq_s32_u32(t1078[0])), vshlq_u32(t1075[1], vreinterpretq_s32_u32(t1078[1])), vorrq_u32(vshlq_u32(t1075[2], vreinterpretq_s32_u32(t1078[0])), vshlq_u32(t1075[0], vsubq_s32(vreinterpretq_s32_u32(t1078[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1075[3], vreinterpretq_s32_u32(t1078[1])), vshlq_u32(t1075[1], vsubq_s32(vreinterpretq_s32_u32(t1078[1]), vdupq_n_s32(32))))];
        let t1080: [uint32x4_t; 4] = [vorrq_u32(x5[0], t1079[0]), vorrq_u32(x5[1], t1079[1]), vorrq_u32(x5[2], t1079[2]), vorrq_u32(x5[3], t1079[3])];
        t1066 = [vbslq_u32(t1067[0], t1080[0], t1066[0]), vbslq_u32(t1067[1], t1080[1], t1066[1]), vbslq_u32(t1067[0], t1080[2], t1066[2]), vbslq_u32(t1067[1], t1080[3], t1066[3])];
    }
    {
        t1066 = [vbslq_u32(t1068[0], x5[0], t1066[0]), vbslq_u32(t1068[1], x5[1], t1066[1]), vbslq_u32(t1068[0], x5[2], t1066[2]), vbslq_u32(t1068[1], x5[3], t1066[3])];
    }
    res = t1066;
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
        let t1081: u64 = ((arg0) as usize as u64);
        let t1082 = [vdupq_n_u32((t1081) as u32); 2];
        let t1083 = [vdupq_n_u32(((t1081) >> 32) as u32); 2];
        let t1084: [uint32x4_t; 2] = [vaddq_u32(t1082[0], x1[0]), vaddq_u32(t1082[1], x1[1])];
        let t1085: [uint32x4_t; 2] = [vcltq_u32(t1084[0], t1082[0]), vcltq_u32(t1084[1], t1082[1])];
        let t1086: [uint32x4_t; 4] = [t1084[0], t1084[1], vsubq_u32(vaddq_u32(t1083[0], x1[2]), t1085[0]), vsubq_u32(vaddq_u32(t1083[1], x1[3]), t1085[1])];
        let mut t1087: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1086[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1086[0]) as u64) as usize as *const u64).read_unaligned(); t1087[0] = vsetq_lane_u32::<0>(v as u32, t1087[0]); t1087[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1087[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1086[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1086[0]) as u64) as usize as *const u64).read_unaligned(); t1087[0] = vsetq_lane_u32::<1>(v as u32, t1087[0]); t1087[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1087[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1086[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1086[0]) as u64) as usize as *const u64).read_unaligned(); t1087[0] = vsetq_lane_u32::<2>(v as u32, t1087[0]); t1087[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1087[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1086[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1086[0]) as u64) as usize as *const u64).read_unaligned(); t1087[0] = vsetq_lane_u32::<3>(v as u32, t1087[0]); t1087[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1087[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1086[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1086[1]) as u64) as usize as *const u64).read_unaligned(); t1087[1] = vsetq_lane_u32::<0>(v as u32, t1087[1]); t1087[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1087[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1086[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1086[1]) as u64) as usize as *const u64).read_unaligned(); t1087[1] = vsetq_lane_u32::<1>(v as u32, t1087[1]); t1087[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1087[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1086[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1086[1]) as u64) as usize as *const u64).read_unaligned(); t1087[1] = vsetq_lane_u32::<2>(v as u32, t1087[1]); t1087[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1087[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1086[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1086[1]) as u64) as usize as *const u64).read_unaligned(); t1087[1] = vsetq_lane_u32::<3>(v as u32, t1087[1]); t1087[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1087[3]); }
        res = t1087;
        return res;
    }
    if ((x2) == (4u32)) {
        let t1088: u64 = ((arg0) as usize as u64);
        let t1089 = [vdupq_n_u32((t1088) as u32); 2];
        let t1090 = [vdupq_n_u32(((t1088) >> 32) as u32); 2];
        let t1091: [uint32x4_t; 2] = [vaddq_u32(t1089[0], x1[0]), vaddq_u32(t1089[1], x1[1])];
        let t1092: [uint32x4_t; 2] = [vcltq_u32(t1091[0], t1089[0]), vcltq_u32(t1091[1], t1089[1])];
        let t1093: [uint32x4_t; 4] = [t1091[0], t1091[1], vsubq_u32(vaddq_u32(t1090[0], x1[2]), t1092[0]), vsubq_u32(vaddq_u32(t1090[1], x1[3]), t1092[1])];
        let mut t1094: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t1094[0] = vsetq_lane_u32::<0>((((vgetq_lane_u32::<0>(t1093[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1093[0]) as u64) as usize as *const u32).read_unaligned() as u32, t1094[0]);
        t1094[0] = vsetq_lane_u32::<1>((((vgetq_lane_u32::<1>(t1093[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1093[0]) as u64) as usize as *const u32).read_unaligned() as u32, t1094[0]);
        t1094[0] = vsetq_lane_u32::<2>((((vgetq_lane_u32::<2>(t1093[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1093[0]) as u64) as usize as *const u32).read_unaligned() as u32, t1094[0]);
        t1094[0] = vsetq_lane_u32::<3>((((vgetq_lane_u32::<3>(t1093[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1093[0]) as u64) as usize as *const u32).read_unaligned() as u32, t1094[0]);
        t1094[1] = vsetq_lane_u32::<0>((((vgetq_lane_u32::<0>(t1093[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1093[1]) as u64) as usize as *const u32).read_unaligned() as u32, t1094[1]);
        t1094[1] = vsetq_lane_u32::<1>((((vgetq_lane_u32::<1>(t1093[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1093[1]) as u64) as usize as *const u32).read_unaligned() as u32, t1094[1]);
        t1094[1] = vsetq_lane_u32::<2>((((vgetq_lane_u32::<2>(t1093[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1093[1]) as u64) as usize as *const u32).read_unaligned() as u32, t1094[1]);
        t1094[1] = vsetq_lane_u32::<3>((((vgetq_lane_u32::<3>(t1093[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1093[1]) as u64) as usize as *const u32).read_unaligned() as u32, t1094[1]);
        let t1095: [uint32x4_t; 4] = [t1094[0], t1094[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1095;
        return res;
    }
    if ((x2) == (2u32)) {
        let t1096: u64 = ((arg0) as usize as u64);
        let t1097 = [vdupq_n_u32((t1096) as u32); 2];
        let t1098 = [vdupq_n_u32(((t1096) >> 32) as u32); 2];
        let t1099: [uint32x4_t; 2] = [vaddq_u32(t1097[0], x1[0]), vaddq_u32(t1097[1], x1[1])];
        let t1100: [uint32x4_t; 2] = [vcltq_u32(t1099[0], t1097[0]), vcltq_u32(t1099[1], t1097[1])];
        let t1101: [uint32x4_t; 4] = [t1099[0], t1099[1], vsubq_u32(vaddq_u32(t1098[0], x1[2]), t1100[0]), vsubq_u32(vaddq_u32(t1098[1], x1[3]), t1100[1])];
        let mut t1102: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t1102[0] = vsetq_lane_u32::<0>((((vgetq_lane_u32::<0>(t1101[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1101[0]) as u64) as usize as *const u16).read_unaligned() as u32, t1102[0]);
        t1102[0] = vsetq_lane_u32::<1>((((vgetq_lane_u32::<1>(t1101[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1101[0]) as u64) as usize as *const u16).read_unaligned() as u32, t1102[0]);
        t1102[0] = vsetq_lane_u32::<2>((((vgetq_lane_u32::<2>(t1101[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1101[0]) as u64) as usize as *const u16).read_unaligned() as u32, t1102[0]);
        t1102[0] = vsetq_lane_u32::<3>((((vgetq_lane_u32::<3>(t1101[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1101[0]) as u64) as usize as *const u16).read_unaligned() as u32, t1102[0]);
        t1102[1] = vsetq_lane_u32::<0>((((vgetq_lane_u32::<0>(t1101[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1101[1]) as u64) as usize as *const u16).read_unaligned() as u32, t1102[1]);
        t1102[1] = vsetq_lane_u32::<1>((((vgetq_lane_u32::<1>(t1101[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1101[1]) as u64) as usize as *const u16).read_unaligned() as u32, t1102[1]);
        t1102[1] = vsetq_lane_u32::<2>((((vgetq_lane_u32::<2>(t1101[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1101[1]) as u64) as usize as *const u16).read_unaligned() as u32, t1102[1]);
        t1102[1] = vsetq_lane_u32::<3>((((vgetq_lane_u32::<3>(t1101[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1101[1]) as u64) as usize as *const u16).read_unaligned() as u32, t1102[1]);
        let t1103: [uint32x4_t; 4] = [t1102[0], t1102[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1103;
        return res;
    }
    if ((x2) == (1u32)) {
        let t1104: u64 = ((arg0) as usize as u64);
        let t1105 = [vdupq_n_u32((t1104) as u32); 2];
        let t1106 = [vdupq_n_u32(((t1104) >> 32) as u32); 2];
        let t1107: [uint32x4_t; 2] = [vaddq_u32(t1105[0], x1[0]), vaddq_u32(t1105[1], x1[1])];
        let t1108: [uint32x4_t; 2] = [vcltq_u32(t1107[0], t1105[0]), vcltq_u32(t1107[1], t1105[1])];
        let t1109: [uint32x4_t; 4] = [t1107[0], t1107[1], vsubq_u32(vaddq_u32(t1106[0], x1[2]), t1108[0]), vsubq_u32(vaddq_u32(t1106[1], x1[3]), t1108[1])];
        let mut t1110: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t1110[0] = vsetq_lane_u32::<0>((((vgetq_lane_u32::<0>(t1109[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1109[0]) as u64) as usize as *const u8).read_unaligned() as u32, t1110[0]);
        t1110[0] = vsetq_lane_u32::<1>((((vgetq_lane_u32::<1>(t1109[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1109[0]) as u64) as usize as *const u8).read_unaligned() as u32, t1110[0]);
        t1110[0] = vsetq_lane_u32::<2>((((vgetq_lane_u32::<2>(t1109[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1109[0]) as u64) as usize as *const u8).read_unaligned() as u32, t1110[0]);
        t1110[0] = vsetq_lane_u32::<3>((((vgetq_lane_u32::<3>(t1109[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1109[0]) as u64) as usize as *const u8).read_unaligned() as u32, t1110[0]);
        t1110[1] = vsetq_lane_u32::<0>((((vgetq_lane_u32::<0>(t1109[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1109[1]) as u64) as usize as *const u8).read_unaligned() as u32, t1110[1]);
        t1110[1] = vsetq_lane_u32::<1>((((vgetq_lane_u32::<1>(t1109[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1109[1]) as u64) as usize as *const u8).read_unaligned() as u32, t1110[1]);
        t1110[1] = vsetq_lane_u32::<2>((((vgetq_lane_u32::<2>(t1109[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1109[1]) as u64) as usize as *const u8).read_unaligned() as u32, t1110[1]);
        t1110[1] = vsetq_lane_u32::<3>((((vgetq_lane_u32::<3>(t1109[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1109[1]) as u64) as usize as *const u8).read_unaligned() as u32, t1110[1]);
        let t1111: [uint32x4_t; 4] = [t1110[0], t1110[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1111;
        return res;
    }
    let t1112: u64 = ((arg0) as usize as u64);
    let t1113 = [vdupq_n_u32((t1112) as u32); 2];
    let t1114 = [vdupq_n_u32(((t1112) >> 32) as u32); 2];
    let t1115: [uint32x4_t; 2] = [vaddq_u32(t1113[0], x1[0]), vaddq_u32(t1113[1], x1[1])];
    let t1116: [uint32x4_t; 2] = [vcltq_u32(t1115[0], t1113[0]), vcltq_u32(t1115[1], t1113[1])];
    let t1117: [uint32x4_t; 4] = [t1115[0], t1115[1], vsubq_u32(vaddq_u32(t1114[0], x1[2]), t1116[0]), vsubq_u32(vaddq_u32(t1114[1], x1[3]), t1116[1])];
    let t1118: u64 = ((arg0) as usize as u64);
    let t1119 = [vdupq_n_u32((t1118) as u32); 2];
    let t1120 = [vdupq_n_u32(((t1118) >> 32) as u32); 2];
    let t1121: [uint32x4_t; 2] = [vaddq_u32(t1119[0], x1[0]), vaddq_u32(t1119[1], x1[1])];
    let t1122: [uint32x4_t; 2] = [vcltq_u32(t1121[0], t1119[0]), vcltq_u32(t1121[1], t1119[1])];
    let t1123: [uint32x4_t; 4] = [t1121[0], t1121[1], vsubq_u32(vaddq_u32(t1120[0], x1[2]), t1122[0]), vsubq_u32(vaddq_u32(t1120[1], x1[3]), t1122[1])];
    let t1124: u64 = 7u64;
    let t1125 = [vdupq_n_u32((t1124) as u32); 2];
    let t1126: [uint32x4_t; 2] = [vandq_u32(t1123[0], t1125[0]), vandq_u32(t1123[1], t1125[1])];
    let t1127: [uint32x4_t; 2] = [vcltq_u32(t1117[0], t1126[0]), vcltq_u32(t1117[1], t1126[1])];
    let t1128: [uint32x4_t; 4] = [vsubq_u32(t1117[0], t1126[0]), vsubq_u32(t1117[1], t1126[1]), vaddq_u32(vsubq_u32(t1117[2], vdupq_n_u32(0)), t1127[0]), vaddq_u32(vsubq_u32(t1117[3], vdupq_n_u32(0)), t1127[1])];
    x3 = t1128;
    let t1129: u64 = ((arg0) as usize as u64);
    let t1130 = [vdupq_n_u32((t1129) as u32); 2];
    let t1131 = [vdupq_n_u32(((t1129) >> 32) as u32); 2];
    let t1132: [uint32x4_t; 2] = [vaddq_u32(t1130[0], x1[0]), vaddq_u32(t1130[1], x1[1])];
    let t1133: [uint32x4_t; 2] = [vcltq_u32(t1132[0], t1130[0]), vcltq_u32(t1132[1], t1130[1])];
    let t1134: [uint32x4_t; 4] = [t1132[0], t1132[1], vsubq_u32(vaddq_u32(t1131[0], x1[2]), t1133[0]), vsubq_u32(vaddq_u32(t1131[1], x1[3]), t1133[1])];
    let t1135: u64 = 7u64;
    let t1136 = [vdupq_n_u32((t1135) as u32); 2];
    let t1137: [uint32x4_t; 2] = [vandq_u32(t1134[0], t1136[0]), vandq_u32(t1134[1], t1136[1])];
    let t1138 = [vdupq_n_u32(8u32); 2];
    let t1139: [uint32x4_t; 2] = [vmulq_u32(t1137[0], t1138[0]), vmulq_u32(t1137[1], t1138[1])];
    x4 = t1139;
    let mut t1140: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x3[2]) as u64) << 32 | vgetq_lane_u32::<0>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1140[0] = vsetq_lane_u32::<0>(v as u32, t1140[0]); t1140[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1140[2]); }
    { let v = (((vgetq_lane_u32::<1>(x3[2]) as u64) << 32 | vgetq_lane_u32::<1>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1140[0] = vsetq_lane_u32::<1>(v as u32, t1140[0]); t1140[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1140[2]); }
    { let v = (((vgetq_lane_u32::<2>(x3[2]) as u64) << 32 | vgetq_lane_u32::<2>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1140[0] = vsetq_lane_u32::<2>(v as u32, t1140[0]); t1140[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1140[2]); }
    { let v = (((vgetq_lane_u32::<3>(x3[2]) as u64) << 32 | vgetq_lane_u32::<3>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1140[0] = vsetq_lane_u32::<3>(v as u32, t1140[0]); t1140[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1140[2]); }
    { let v = (((vgetq_lane_u32::<0>(x3[3]) as u64) << 32 | vgetq_lane_u32::<0>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1140[1] = vsetq_lane_u32::<0>(v as u32, t1140[1]); t1140[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1140[3]); }
    { let v = (((vgetq_lane_u32::<1>(x3[3]) as u64) << 32 | vgetq_lane_u32::<1>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1140[1] = vsetq_lane_u32::<1>(v as u32, t1140[1]); t1140[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1140[3]); }
    { let v = (((vgetq_lane_u32::<2>(x3[3]) as u64) << 32 | vgetq_lane_u32::<2>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1140[1] = vsetq_lane_u32::<2>(v as u32, t1140[1]); t1140[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1140[3]); }
    { let v = (((vgetq_lane_u32::<3>(x3[3]) as u64) << 32 | vgetq_lane_u32::<3>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1140[1] = vsetq_lane_u32::<3>(v as u32, t1140[1]); t1140[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1140[3]); }
    let t1141: [uint32x4_t; 2] = [x4[0], x4[1]];
    let t1142: [uint32x4_t; 2] = [vandq_u32(t1141[0], vdupq_n_u32(63)), vandq_u32(t1141[1], vdupq_n_u32(63))];
    let t1143: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t1140[0], vnegq_s32(vreinterpretq_s32_u32(t1142[0]))), vshlq_u32(t1140[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1142[0])))), vorrq_u32(vshlq_u32(t1140[1], vnegq_s32(vreinterpretq_s32_u32(t1142[1]))), vshlq_u32(t1140[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1142[1])))), vshlq_u32(t1140[2], vnegq_s32(vreinterpretq_s32_u32(t1142[0]))), vshlq_u32(t1140[3], vnegq_s32(vreinterpretq_s32_u32(t1142[1])))];
    x5 = t1143;
    let t1144 = [vdupq_n_u32(0u32); 2];
    let t1145: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t1144[0])), vmvnq_u32(vceqq_u32(x4[1], t1144[1]))];
    let t1146 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
    let t1147: [uint32x4_t; 2] = [vaddq_u32(x4[0], t1146[0]), vaddq_u32(x4[1], t1146[1])];
    let t1148 = [vdupq_n_u32(64u32); 2];
    let t1149: [uint32x4_t; 2] = [vcgtq_u32(t1147[0], t1148[0]), vcgtq_u32(t1147[1], t1148[1])];
    let t1150: [uint32x4_t; 2] = [vandq_u32(t1145[0], t1149[0]), vandq_u32(t1145[1], t1149[1])];
    let mut t1151: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1152: [uint32x4_t; 2] = [vmvnq_u32(t1150[0]), vmvnq_u32(t1150[1])];
    if any(&t1150) {
        let t1153: u64 = (1u64).wrapping_mul(8u64);
        let t1154 = [vdupq_n_u32((t1153) as u32); 2];
        let t1155 = [vdupq_n_u32(((t1153) >> 32) as u32); 2];
        let t1156: [uint32x4_t; 2] = [vaddq_u32(x3[0], t1154[0]), vaddq_u32(x3[1], t1154[1])];
        let t1157: [uint32x4_t; 2] = [vcltq_u32(t1156[0], x3[0]), vcltq_u32(t1156[1], x3[1])];
        let t1158: [uint32x4_t; 4] = [t1156[0], t1156[1], vsubq_u32(vaddq_u32(x3[2], t1155[0]), t1157[0]), vsubq_u32(vaddq_u32(x3[3], t1155[1]), t1157[1])];
        let mut t1159: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1158[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1158[0]) as u64) as usize as *const u64).read_unaligned(); t1159[0] = vsetq_lane_u32::<0>(v as u32, t1159[0]); t1159[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1159[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1158[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1158[0]) as u64) as usize as *const u64).read_unaligned(); t1159[0] = vsetq_lane_u32::<1>(v as u32, t1159[0]); t1159[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1159[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1158[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1158[0]) as u64) as usize as *const u64).read_unaligned(); t1159[0] = vsetq_lane_u32::<2>(v as u32, t1159[0]); t1159[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1159[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1158[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1158[0]) as u64) as usize as *const u64).read_unaligned(); t1159[0] = vsetq_lane_u32::<3>(v as u32, t1159[0]); t1159[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1159[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1158[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1158[1]) as u64) as usize as *const u64).read_unaligned(); t1159[1] = vsetq_lane_u32::<0>(v as u32, t1159[1]); t1159[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1159[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1158[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1158[1]) as u64) as usize as *const u64).read_unaligned(); t1159[1] = vsetq_lane_u32::<1>(v as u32, t1159[1]); t1159[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1159[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1158[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1158[1]) as u64) as usize as *const u64).read_unaligned(); t1159[1] = vsetq_lane_u32::<2>(v as u32, t1159[1]); t1159[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1159[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1158[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1158[1]) as u64) as usize as *const u64).read_unaligned(); t1159[1] = vsetq_lane_u32::<3>(v as u32, t1159[1]); t1159[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1159[3]); }
        let t1160: [uint32x4_t; 2] = [vsubq_u32(t1148[0], x4[0]), vsubq_u32(t1148[1], x4[1])];
        let t1161: [uint32x4_t; 2] = [t1160[0], t1160[1]];
        let t1162: [uint32x4_t; 2] = [vandq_u32(t1161[0], vdupq_n_u32(63)), vandq_u32(t1161[1], vdupq_n_u32(63))];
        let t1163: [uint32x4_t; 4] = [vshlq_u32(t1159[0], vreinterpretq_s32_u32(t1162[0])), vshlq_u32(t1159[1], vreinterpretq_s32_u32(t1162[1])), vorrq_u32(vshlq_u32(t1159[2], vreinterpretq_s32_u32(t1162[0])), vshlq_u32(t1159[0], vsubq_s32(vreinterpretq_s32_u32(t1162[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1159[3], vreinterpretq_s32_u32(t1162[1])), vshlq_u32(t1159[1], vsubq_s32(vreinterpretq_s32_u32(t1162[1]), vdupq_n_s32(32))))];
        let t1164: [uint32x4_t; 4] = [vorrq_u32(x5[0], t1163[0]), vorrq_u32(x5[1], t1163[1]), vorrq_u32(x5[2], t1163[2]), vorrq_u32(x5[3], t1163[3])];
        t1151 = [vbslq_u32(t1150[0], t1164[0], t1151[0]), vbslq_u32(t1150[1], t1164[1], t1151[1]), vbslq_u32(t1150[0], t1164[2], t1151[2]), vbslq_u32(t1150[1], t1164[3], t1151[3])];
    }
    {
        t1151 = [vbslq_u32(t1152[0], x5[0], t1151[0]), vbslq_u32(t1152[1], x5[1], t1151[1]), vbslq_u32(t1152[0], x5[2], t1151[2]), vbslq_u32(t1152[1], x5[3], t1151[3])];
    }
    res = t1151;
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
        let t1165 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], x1i[0]), vandq_u32(live[1], x1i[1]))));
        let t1166: [uint32x4_t; 2] = [vbslq_u32(live[0], x1i[0], t1165), vbslq_u32(live[1], x1i[1], t1165)];
        let mut t1167: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1166[0]) as usize * 1) as *const u64).read_unaligned(); t1167[0] = vsetq_lane_u32::<0>(v as u32, t1167[0]); t1167[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1167[2]); }
        { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1166[0]) as usize * 1) as *const u64).read_unaligned(); t1167[0] = vsetq_lane_u32::<1>(v as u32, t1167[0]); t1167[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1167[2]); }
        { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1166[0]) as usize * 1) as *const u64).read_unaligned(); t1167[0] = vsetq_lane_u32::<2>(v as u32, t1167[0]); t1167[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1167[2]); }
        { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1166[0]) as usize * 1) as *const u64).read_unaligned(); t1167[0] = vsetq_lane_u32::<3>(v as u32, t1167[0]); t1167[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1167[2]); }
        { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1166[1]) as usize * 1) as *const u64).read_unaligned(); t1167[1] = vsetq_lane_u32::<0>(v as u32, t1167[1]); t1167[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1167[3]); }
        { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1166[1]) as usize * 1) as *const u64).read_unaligned(); t1167[1] = vsetq_lane_u32::<1>(v as u32, t1167[1]); t1167[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1167[3]); }
        { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1166[1]) as usize * 1) as *const u64).read_unaligned(); t1167[1] = vsetq_lane_u32::<2>(v as u32, t1167[1]); t1167[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1167[3]); }
        { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1166[1]) as usize * 1) as *const u64).read_unaligned(); t1167[1] = vsetq_lane_u32::<3>(v as u32, t1167[1]); t1167[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1167[3]); }
        res = t1167;
        return res;
    }
    if ((x2) == (4u32)) {
        let t1168 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], x1i[0]), vandq_u32(live[1], x1i[1]))));
        let t1169: [uint32x4_t; 2] = [vbslq_u32(live[0], x1i[0], t1168), vbslq_u32(live[1], x1i[1], t1168)];
        let mut t1170: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t1170[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1169[0]) as usize * 1) as *const u32).read_unaligned() as u32), t1170[0]);
        t1170[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1169[0]) as usize * 1) as *const u32).read_unaligned() as u32), t1170[0]);
        t1170[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1169[0]) as usize * 1) as *const u32).read_unaligned() as u32), t1170[0]);
        t1170[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1169[0]) as usize * 1) as *const u32).read_unaligned() as u32), t1170[0]);
        t1170[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1169[1]) as usize * 1) as *const u32).read_unaligned() as u32), t1170[1]);
        t1170[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1169[1]) as usize * 1) as *const u32).read_unaligned() as u32), t1170[1]);
        t1170[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1169[1]) as usize * 1) as *const u32).read_unaligned() as u32), t1170[1]);
        t1170[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1169[1]) as usize * 1) as *const u32).read_unaligned() as u32), t1170[1]);
        let t1171: [uint32x4_t; 4] = [t1170[0], t1170[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1171;
        return res;
    }
    if ((x2) == (2u32)) {
        let t1172 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], x1i[0]), vandq_u32(live[1], x1i[1]))));
        let t1173: [uint32x4_t; 2] = [vbslq_u32(live[0], x1i[0], t1172), vbslq_u32(live[1], x1i[1], t1172)];
        let mut t1174: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t1174[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1173[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1174[0]);
        t1174[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1173[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1174[0]);
        t1174[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1173[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1174[0]);
        t1174[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1173[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1174[0]);
        t1174[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1173[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1174[1]);
        t1174[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1173[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1174[1]);
        t1174[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1173[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1174[1]);
        t1174[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1173[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1174[1]);
        let t1175: [uint32x4_t; 4] = [t1174[0], t1174[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1175;
        return res;
    }
    if ((x2) == (1u32)) {
        let t1176 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], x1i[0]), vandq_u32(live[1], x1i[1]))));
        let t1177: [uint32x4_t; 2] = [vbslq_u32(live[0], x1i[0], t1176), vbslq_u32(live[1], x1i[1], t1176)];
        let mut t1178: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t1178[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1177[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1178[0]);
        t1178[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1177[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1178[0]);
        t1178[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1177[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1178[0]);
        t1178[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1177[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1178[0]);
        t1178[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1177[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1178[1]);
        t1178[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1177[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1178[1]);
        t1178[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1177[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1178[1]);
        t1178[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1177[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1178[1]);
        let t1179: [uint32x4_t; 4] = [t1178[0], t1178[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1179;
        return res;
    }
    let t1180: [uint32x4_t; 2] = [x1i[0], x1i[1]];
    let t1181: u64 = (((x1b).wrapping_add((((arg0) as usize as u64)) as usize)) as usize as u64);
    let t1182 = [vdupq_n_u32((t1181) as u32); 2];
    let t1183 = [vdupq_n_u32(((t1181) >> 32) as u32); 2];
    let t1184: [uint32x4_t; 2] = [vaddq_u32(t1182[0], t1180[0]), vaddq_u32(t1182[1], t1180[1])];
    let t1185: [uint32x4_t; 2] = [vcltq_u32(t1184[0], t1182[0]), vcltq_u32(t1184[1], t1182[1])];
    let t1186: [uint32x4_t; 4] = [t1184[0], t1184[1], vsubq_u32(vaddq_u32(t1183[0], vdupq_n_u32(0)), t1185[0]), vsubq_u32(vaddq_u32(t1183[1], vdupq_n_u32(0)), t1185[1])];
    let t1187: u64 = 7u64;
    let t1188 = [vdupq_n_u32((t1187) as u32); 2];
    let t1189: [uint32x4_t; 2] = [vandq_u32(t1186[0], t1188[0]), vandq_u32(t1186[1], t1188[1])];
    let t1190: [uint32x4_t; 2] = [x1i[0], x1i[1]];
    let t1191: u64 = (((arg0).wrapping_add((x1b) as usize)) as usize as u64);
    let t1192 = [vdupq_n_u32((t1191) as u32); 2];
    let t1193 = [vdupq_n_u32(((t1191) >> 32) as u32); 2];
    let t1194: [uint32x4_t; 2] = [vaddq_u32(t1192[0], t1190[0]), vaddq_u32(t1192[1], t1190[1])];
    let t1195: [uint32x4_t; 2] = [vcltq_u32(t1194[0], t1192[0]), vcltq_u32(t1194[1], t1192[1])];
    let t1196: [uint32x4_t; 4] = [t1194[0], t1194[1], vsubq_u32(vaddq_u32(t1193[0], vdupq_n_u32(0)), t1195[0]), vsubq_u32(vaddq_u32(t1193[1], vdupq_n_u32(0)), t1195[1])];
    let t1197: [uint32x4_t; 2] = [vcltq_u32(t1196[0], t1189[0]), vcltq_u32(t1196[1], t1189[1])];
    let t1198: [uint32x4_t; 4] = [vsubq_u32(t1196[0], t1189[0]), vsubq_u32(t1196[1], t1189[1]), vaddq_u32(vsubq_u32(t1196[2], vdupq_n_u32(0)), t1197[0]), vaddq_u32(vsubq_u32(t1196[3], vdupq_n_u32(0)), t1197[1])];
    x3 = t1198;
    let t1199: [uint32x4_t; 2] = [x1i[0], x1i[1]];
    let t1200: u64 = (((x1b).wrapping_add((((arg0) as usize as u64)) as usize)) as usize as u64);
    let t1201 = [vdupq_n_u32((t1200) as u32); 2];
    let t1202 = [vdupq_n_u32(((t1200) >> 32) as u32); 2];
    let t1203: [uint32x4_t; 2] = [vaddq_u32(t1201[0], t1199[0]), vaddq_u32(t1201[1], t1199[1])];
    let t1204: [uint32x4_t; 2] = [vcltq_u32(t1203[0], t1201[0]), vcltq_u32(t1203[1], t1201[1])];
    let t1205: [uint32x4_t; 4] = [t1203[0], t1203[1], vsubq_u32(vaddq_u32(t1202[0], vdupq_n_u32(0)), t1204[0]), vsubq_u32(vaddq_u32(t1202[1], vdupq_n_u32(0)), t1204[1])];
    let t1206: u64 = 7u64;
    let t1207 = [vdupq_n_u32((t1206) as u32); 2];
    let t1208: [uint32x4_t; 2] = [vandq_u32(t1205[0], t1207[0]), vandq_u32(t1205[1], t1207[1])];
    let t1209 = [vdupq_n_u32(8u32); 2];
    let t1210: [uint32x4_t; 2] = [vmulq_u32(t1208[0], t1209[0]), vmulq_u32(t1208[1], t1209[1])];
    x4 = t1210;
    let mut t1211: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x3[2]) as u64) << 32 | vgetq_lane_u32::<0>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1211[0] = vsetq_lane_u32::<0>(v as u32, t1211[0]); t1211[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1211[2]); }
    { let v = (((vgetq_lane_u32::<1>(x3[2]) as u64) << 32 | vgetq_lane_u32::<1>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1211[0] = vsetq_lane_u32::<1>(v as u32, t1211[0]); t1211[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1211[2]); }
    { let v = (((vgetq_lane_u32::<2>(x3[2]) as u64) << 32 | vgetq_lane_u32::<2>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1211[0] = vsetq_lane_u32::<2>(v as u32, t1211[0]); t1211[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1211[2]); }
    { let v = (((vgetq_lane_u32::<3>(x3[2]) as u64) << 32 | vgetq_lane_u32::<3>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1211[0] = vsetq_lane_u32::<3>(v as u32, t1211[0]); t1211[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1211[2]); }
    { let v = (((vgetq_lane_u32::<0>(x3[3]) as u64) << 32 | vgetq_lane_u32::<0>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1211[1] = vsetq_lane_u32::<0>(v as u32, t1211[1]); t1211[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1211[3]); }
    { let v = (((vgetq_lane_u32::<1>(x3[3]) as u64) << 32 | vgetq_lane_u32::<1>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1211[1] = vsetq_lane_u32::<1>(v as u32, t1211[1]); t1211[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1211[3]); }
    { let v = (((vgetq_lane_u32::<2>(x3[3]) as u64) << 32 | vgetq_lane_u32::<2>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1211[1] = vsetq_lane_u32::<2>(v as u32, t1211[1]); t1211[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1211[3]); }
    { let v = (((vgetq_lane_u32::<3>(x3[3]) as u64) << 32 | vgetq_lane_u32::<3>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1211[1] = vsetq_lane_u32::<3>(v as u32, t1211[1]); t1211[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1211[3]); }
    let t1212: [uint32x4_t; 2] = [x4[0], x4[1]];
    let t1213: [uint32x4_t; 2] = [vandq_u32(t1212[0], vdupq_n_u32(63)), vandq_u32(t1212[1], vdupq_n_u32(63))];
    let t1214: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t1211[0], vnegq_s32(vreinterpretq_s32_u32(t1213[0]))), vshlq_u32(t1211[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1213[0])))), vorrq_u32(vshlq_u32(t1211[1], vnegq_s32(vreinterpretq_s32_u32(t1213[1]))), vshlq_u32(t1211[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1213[1])))), vshlq_u32(t1211[2], vnegq_s32(vreinterpretq_s32_u32(t1213[0]))), vshlq_u32(t1211[3], vnegq_s32(vreinterpretq_s32_u32(t1213[1])))];
    x5 = t1214;
    let t1215 = [vdupq_n_u32(0u32); 2];
    let t1216: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t1215[0])), vmvnq_u32(vceqq_u32(x4[1], t1215[1]))];
    let t1217 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
    let t1218: [uint32x4_t; 2] = [vaddq_u32(x4[0], t1217[0]), vaddq_u32(x4[1], t1217[1])];
    let t1219 = [vdupq_n_u32(64u32); 2];
    let t1220: [uint32x4_t; 2] = [vcgtq_u32(t1218[0], t1219[0]), vcgtq_u32(t1218[1], t1219[1])];
    let t1221: [uint32x4_t; 2] = [vandq_u32(t1216[0], t1220[0]), vandq_u32(t1216[1], t1220[1])];
    let mut t1222: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1223: [uint32x4_t; 2] = [vandq_u32(live[0], t1221[0]), vandq_u32(live[1], t1221[1])];
    let t1224: [uint32x4_t; 2] = [vbicq_u32(live[0], t1221[0]), vbicq_u32(live[1], t1221[1])];
    if any(&t1223) {
        let t1225: u64 = (1u64).wrapping_mul(8u64);
        let t1226 = [vdupq_n_u32((t1225) as u32); 2];
        let t1227 = [vdupq_n_u32(((t1225) >> 32) as u32); 2];
        let t1228: [uint32x4_t; 2] = [vaddq_u32(x3[0], t1226[0]), vaddq_u32(x3[1], t1226[1])];
        let t1229: [uint32x4_t; 2] = [vcltq_u32(t1228[0], x3[0]), vcltq_u32(t1228[1], x3[1])];
        let t1230: [uint32x4_t; 4] = [t1228[0], t1228[1], vsubq_u32(vaddq_u32(x3[2], t1227[0]), t1229[0]), vsubq_u32(vaddq_u32(x3[3], t1227[1]), t1229[1])];
        let mut t1231: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1230[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1230[0]) as u64) as usize as *const u64).read_unaligned(); t1231[0] = vsetq_lane_u32::<0>(v as u32, t1231[0]); t1231[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1231[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1230[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1230[0]) as u64) as usize as *const u64).read_unaligned(); t1231[0] = vsetq_lane_u32::<1>(v as u32, t1231[0]); t1231[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1231[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1230[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1230[0]) as u64) as usize as *const u64).read_unaligned(); t1231[0] = vsetq_lane_u32::<2>(v as u32, t1231[0]); t1231[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1231[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1230[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1230[0]) as u64) as usize as *const u64).read_unaligned(); t1231[0] = vsetq_lane_u32::<3>(v as u32, t1231[0]); t1231[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1231[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1230[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1230[1]) as u64) as usize as *const u64).read_unaligned(); t1231[1] = vsetq_lane_u32::<0>(v as u32, t1231[1]); t1231[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1231[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1230[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1230[1]) as u64) as usize as *const u64).read_unaligned(); t1231[1] = vsetq_lane_u32::<1>(v as u32, t1231[1]); t1231[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1231[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1230[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1230[1]) as u64) as usize as *const u64).read_unaligned(); t1231[1] = vsetq_lane_u32::<2>(v as u32, t1231[1]); t1231[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1231[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1230[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1230[1]) as u64) as usize as *const u64).read_unaligned(); t1231[1] = vsetq_lane_u32::<3>(v as u32, t1231[1]); t1231[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1231[3]); }
        let t1232: [uint32x4_t; 2] = [vsubq_u32(t1219[0], x4[0]), vsubq_u32(t1219[1], x4[1])];
        let t1233: [uint32x4_t; 2] = [t1232[0], t1232[1]];
        let t1234: [uint32x4_t; 2] = [vandq_u32(t1233[0], vdupq_n_u32(63)), vandq_u32(t1233[1], vdupq_n_u32(63))];
        let t1235: [uint32x4_t; 4] = [vshlq_u32(t1231[0], vreinterpretq_s32_u32(t1234[0])), vshlq_u32(t1231[1], vreinterpretq_s32_u32(t1234[1])), vorrq_u32(vshlq_u32(t1231[2], vreinterpretq_s32_u32(t1234[0])), vshlq_u32(t1231[0], vsubq_s32(vreinterpretq_s32_u32(t1234[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1231[3], vreinterpretq_s32_u32(t1234[1])), vshlq_u32(t1231[1], vsubq_s32(vreinterpretq_s32_u32(t1234[1]), vdupq_n_s32(32))))];
        let t1236: [uint32x4_t; 4] = [vorrq_u32(x5[0], t1235[0]), vorrq_u32(x5[1], t1235[1]), vorrq_u32(x5[2], t1235[2]), vorrq_u32(x5[3], t1235[3])];
        t1222 = [vbslq_u32(t1223[0], t1236[0], t1222[0]), vbslq_u32(t1223[1], t1236[1], t1222[1]), vbslq_u32(t1223[0], t1236[2], t1222[2]), vbslq_u32(t1223[1], t1236[3], t1222[3])];
    }
    {
        t1222 = [vbslq_u32(t1224[0], x5[0], t1222[0]), vbslq_u32(t1224[1], x5[1], t1222[1]), vbslq_u32(t1224[0], x5[2], t1222[2]), vbslq_u32(t1224[1], x5[3], t1222[3])];
    }
    res = t1222;
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
        let t1237: [uint32x4_t; 4];
        if (x1is) == 8u32 && ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 && all(&live) {
            let t1238 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
            let t12390 = vld2q_u32((t1238 as *const u32).add(0));
            let t12391 = vld2q_u32((t1238 as *const u32).add(8));
            let t1239: [uint32x4_t; 4] = [t12390.0, t12391.0, t12390.1, t12391.1];
            t1237 = t1239;
        } else {
            let t1240: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
            let t1241 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t1240[0]), vandq_u32(live[1], t1240[1]))));
            let t1242: [uint32x4_t; 2] = [vbslq_u32(live[0], t1240[0], t1241), vbslq_u32(live[1], t1240[1], t1241)];
            let mut t1243: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1242[0]) as usize * 1) as *const u64).read_unaligned(); t1243[0] = vsetq_lane_u32::<0>(v as u32, t1243[0]); t1243[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1243[2]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1242[0]) as usize * 1) as *const u64).read_unaligned(); t1243[0] = vsetq_lane_u32::<1>(v as u32, t1243[0]); t1243[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1243[2]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1242[0]) as usize * 1) as *const u64).read_unaligned(); t1243[0] = vsetq_lane_u32::<2>(v as u32, t1243[0]); t1243[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1243[2]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1242[0]) as usize * 1) as *const u64).read_unaligned(); t1243[0] = vsetq_lane_u32::<3>(v as u32, t1243[0]); t1243[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1243[2]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1242[1]) as usize * 1) as *const u64).read_unaligned(); t1243[1] = vsetq_lane_u32::<0>(v as u32, t1243[1]); t1243[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1243[3]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1242[1]) as usize * 1) as *const u64).read_unaligned(); t1243[1] = vsetq_lane_u32::<1>(v as u32, t1243[1]); t1243[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1243[3]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1242[1]) as usize * 1) as *const u64).read_unaligned(); t1243[1] = vsetq_lane_u32::<2>(v as u32, t1243[1]); t1243[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1243[3]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1242[1]) as usize * 1) as *const u64).read_unaligned(); t1243[1] = vsetq_lane_u32::<3>(v as u32, t1243[1]); t1243[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1243[3]); }
            t1237 = t1243;
        }
        res = t1237;
        return res;
    }
    if ((x2) == (4u32)) {
        let t1244: [uint32x4_t; 2];
        if (x1is) == 4u32 && ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 && all(&live) {
            let t1245 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
            let t1246: [uint32x4_t; 2] = core::array::from_fn(|r| vld1q_u32((t1245 as *const u32).add(4 * r)));
            t1244 = t1246;
        } else {
            let t1247: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
            let t1248 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t1247[0]), vandq_u32(live[1], t1247[1]))));
            let t1249: [uint32x4_t; 2] = [vbslq_u32(live[0], t1247[0], t1248), vbslq_u32(live[1], t1247[1], t1248)];
            let mut t1250: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            t1250[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1249[0]) as usize * 1) as *const u32).read_unaligned() as u32), t1250[0]);
            t1250[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1249[0]) as usize * 1) as *const u32).read_unaligned() as u32), t1250[0]);
            t1250[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1249[0]) as usize * 1) as *const u32).read_unaligned() as u32), t1250[0]);
            t1250[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1249[0]) as usize * 1) as *const u32).read_unaligned() as u32), t1250[0]);
            t1250[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1249[1]) as usize * 1) as *const u32).read_unaligned() as u32), t1250[1]);
            t1250[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1249[1]) as usize * 1) as *const u32).read_unaligned() as u32), t1250[1]);
            t1250[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1249[1]) as usize * 1) as *const u32).read_unaligned() as u32), t1250[1]);
            t1250[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1249[1]) as usize * 1) as *const u32).read_unaligned() as u32), t1250[1]);
            t1244 = t1250;
        }
        let t1251: [uint32x4_t; 4] = [t1244[0], t1244[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1251;
        return res;
    }
    if ((x2) == (2u32)) {
        let t1252: [uint32x4_t; 2];
        if (x1is) == 2u32 && ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 && all(&live) {
            let t1253 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
            let t1254: [uint32x4_t; 2] = core::array::from_fn(|r| vmovl_u16(vld1_u16((t1253 as *const u16).add(4 * r))));
            t1252 = t1254;
        } else {
            let t1255: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
            let t1256 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t1255[0]), vandq_u32(live[1], t1255[1]))));
            let t1257: [uint32x4_t; 2] = [vbslq_u32(live[0], t1255[0], t1256), vbslq_u32(live[1], t1255[1], t1256)];
            let mut t1258: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            t1258[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1257[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1258[0]);
            t1258[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1257[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1258[0]);
            t1258[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1257[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1258[0]);
            t1258[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1257[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1258[0]);
            t1258[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1257[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1258[1]);
            t1258[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1257[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1258[1]);
            t1258[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1257[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1258[1]);
            t1258[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1257[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1258[1]);
            t1252 = t1258;
        }
        let t1259: [uint32x4_t; 4] = [t1252[0], t1252[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1259;
        return res;
    }
    if ((x2) == (1u32)) {
        let t1260: [uint32x4_t; 2];
        if (x1is) == 1u32 && ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 && all(&live) {
            let t1261 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
            let t1263 = vmovl_u8(vld1_u8(t1261.add(0)));
            let t1262: [uint32x4_t; 2] = [vmovl_u16(vget_low_u16(t1263)), vmovl_u16(vget_high_u16(t1263))];
            t1260 = t1262;
        } else {
            let t1264: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
            let t1265 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t1264[0]), vandq_u32(live[1], t1264[1]))));
            let t1266: [uint32x4_t; 2] = [vbslq_u32(live[0], t1264[0], t1265), vbslq_u32(live[1], t1264[1], t1265)];
            let mut t1267: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            t1267[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1266[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1267[0]);
            t1267[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1266[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1267[0]);
            t1267[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1266[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1267[0]);
            t1267[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1266[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1267[0]);
            t1267[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1266[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1267[1]);
            t1267[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1266[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1267[1]);
            t1267[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1266[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1267[1]);
            t1267[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1266[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1267[1]);
            t1260 = t1267;
        }
        let t1268: [uint32x4_t; 4] = [t1260[0], t1260[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1268;
        return res;
    }
    let t1269: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
    let t1270: [uint32x4_t; 2] = [t1269[0], t1269[1]];
    let t1271: u64 = (((x1b).wrapping_add((((arg0) as usize as u64)) as usize)) as usize as u64);
    let t1272 = [vdupq_n_u32((t1271) as u32); 2];
    let t1273 = [vdupq_n_u32(((t1271) >> 32) as u32); 2];
    let t1274: [uint32x4_t; 2] = [vaddq_u32(t1272[0], t1270[0]), vaddq_u32(t1272[1], t1270[1])];
    let t1275: [uint32x4_t; 2] = [vcltq_u32(t1274[0], t1272[0]), vcltq_u32(t1274[1], t1272[1])];
    let t1276: [uint32x4_t; 4] = [t1274[0], t1274[1], vsubq_u32(vaddq_u32(t1273[0], vdupq_n_u32(0)), t1275[0]), vsubq_u32(vaddq_u32(t1273[1], vdupq_n_u32(0)), t1275[1])];
    let t1277: u64 = 7u64;
    let t1278 = [vdupq_n_u32((t1277) as u32); 2];
    let t1279: [uint32x4_t; 2] = [vandq_u32(t1276[0], t1278[0]), vandq_u32(t1276[1], t1278[1])];
    let t1280: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
    let t1281: [uint32x4_t; 2] = [t1280[0], t1280[1]];
    let t1282: u64 = (((arg0).wrapping_add((x1b) as usize)) as usize as u64);
    let t1283 = [vdupq_n_u32((t1282) as u32); 2];
    let t1284 = [vdupq_n_u32(((t1282) >> 32) as u32); 2];
    let t1285: [uint32x4_t; 2] = [vaddq_u32(t1283[0], t1281[0]), vaddq_u32(t1283[1], t1281[1])];
    let t1286: [uint32x4_t; 2] = [vcltq_u32(t1285[0], t1283[0]), vcltq_u32(t1285[1], t1283[1])];
    let t1287: [uint32x4_t; 4] = [t1285[0], t1285[1], vsubq_u32(vaddq_u32(t1284[0], vdupq_n_u32(0)), t1286[0]), vsubq_u32(vaddq_u32(t1284[1], vdupq_n_u32(0)), t1286[1])];
    let t1288: [uint32x4_t; 2] = [vcltq_u32(t1287[0], t1279[0]), vcltq_u32(t1287[1], t1279[1])];
    let t1289: [uint32x4_t; 4] = [vsubq_u32(t1287[0], t1279[0]), vsubq_u32(t1287[1], t1279[1]), vaddq_u32(vsubq_u32(t1287[2], vdupq_n_u32(0)), t1288[0]), vaddq_u32(vsubq_u32(t1287[3], vdupq_n_u32(0)), t1288[1])];
    x3 = t1289;
    let t1290: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
    let t1291: [uint32x4_t; 2] = [t1290[0], t1290[1]];
    let t1292: u64 = (((x1b).wrapping_add((((arg0) as usize as u64)) as usize)) as usize as u64);
    let t1293 = [vdupq_n_u32((t1292) as u32); 2];
    let t1294 = [vdupq_n_u32(((t1292) >> 32) as u32); 2];
    let t1295: [uint32x4_t; 2] = [vaddq_u32(t1293[0], t1291[0]), vaddq_u32(t1293[1], t1291[1])];
    let t1296: [uint32x4_t; 2] = [vcltq_u32(t1295[0], t1293[0]), vcltq_u32(t1295[1], t1293[1])];
    let t1297: [uint32x4_t; 4] = [t1295[0], t1295[1], vsubq_u32(vaddq_u32(t1294[0], vdupq_n_u32(0)), t1296[0]), vsubq_u32(vaddq_u32(t1294[1], vdupq_n_u32(0)), t1296[1])];
    let t1298: u64 = 7u64;
    let t1299 = [vdupq_n_u32((t1298) as u32); 2];
    let t1300: [uint32x4_t; 2] = [vandq_u32(t1297[0], t1299[0]), vandq_u32(t1297[1], t1299[1])];
    let t1301 = [vdupq_n_u32(8u32); 2];
    let t1302: [uint32x4_t; 2] = [vmulq_u32(t1300[0], t1301[0]), vmulq_u32(t1300[1], t1301[1])];
    x4 = t1302;
    let mut t1303: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x3[2]) as u64) << 32 | vgetq_lane_u32::<0>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1303[0] = vsetq_lane_u32::<0>(v as u32, t1303[0]); t1303[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1303[2]); }
    { let v = (((vgetq_lane_u32::<1>(x3[2]) as u64) << 32 | vgetq_lane_u32::<1>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1303[0] = vsetq_lane_u32::<1>(v as u32, t1303[0]); t1303[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1303[2]); }
    { let v = (((vgetq_lane_u32::<2>(x3[2]) as u64) << 32 | vgetq_lane_u32::<2>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1303[0] = vsetq_lane_u32::<2>(v as u32, t1303[0]); t1303[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1303[2]); }
    { let v = (((vgetq_lane_u32::<3>(x3[2]) as u64) << 32 | vgetq_lane_u32::<3>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1303[0] = vsetq_lane_u32::<3>(v as u32, t1303[0]); t1303[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1303[2]); }
    { let v = (((vgetq_lane_u32::<0>(x3[3]) as u64) << 32 | vgetq_lane_u32::<0>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1303[1] = vsetq_lane_u32::<0>(v as u32, t1303[1]); t1303[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1303[3]); }
    { let v = (((vgetq_lane_u32::<1>(x3[3]) as u64) << 32 | vgetq_lane_u32::<1>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1303[1] = vsetq_lane_u32::<1>(v as u32, t1303[1]); t1303[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1303[3]); }
    { let v = (((vgetq_lane_u32::<2>(x3[3]) as u64) << 32 | vgetq_lane_u32::<2>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1303[1] = vsetq_lane_u32::<2>(v as u32, t1303[1]); t1303[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1303[3]); }
    { let v = (((vgetq_lane_u32::<3>(x3[3]) as u64) << 32 | vgetq_lane_u32::<3>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1303[1] = vsetq_lane_u32::<3>(v as u32, t1303[1]); t1303[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1303[3]); }
    let t1304: [uint32x4_t; 2] = [x4[0], x4[1]];
    let t1305: [uint32x4_t; 2] = [vandq_u32(t1304[0], vdupq_n_u32(63)), vandq_u32(t1304[1], vdupq_n_u32(63))];
    let t1306: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t1303[0], vnegq_s32(vreinterpretq_s32_u32(t1305[0]))), vshlq_u32(t1303[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1305[0])))), vorrq_u32(vshlq_u32(t1303[1], vnegq_s32(vreinterpretq_s32_u32(t1305[1]))), vshlq_u32(t1303[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1305[1])))), vshlq_u32(t1303[2], vnegq_s32(vreinterpretq_s32_u32(t1305[0]))), vshlq_u32(t1303[3], vnegq_s32(vreinterpretq_s32_u32(t1305[1])))];
    x5 = t1306;
    let t1307 = [vdupq_n_u32(0u32); 2];
    let t1308: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t1307[0])), vmvnq_u32(vceqq_u32(x4[1], t1307[1]))];
    let t1309 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
    let t1310: [uint32x4_t; 2] = [vaddq_u32(x4[0], t1309[0]), vaddq_u32(x4[1], t1309[1])];
    let t1311 = [vdupq_n_u32(64u32); 2];
    let t1312: [uint32x4_t; 2] = [vcgtq_u32(t1310[0], t1311[0]), vcgtq_u32(t1310[1], t1311[1])];
    let t1313: [uint32x4_t; 2] = [vandq_u32(t1308[0], t1312[0]), vandq_u32(t1308[1], t1312[1])];
    let mut t1314: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1315: [uint32x4_t; 2] = [vandq_u32(live[0], t1313[0]), vandq_u32(live[1], t1313[1])];
    let t1316: [uint32x4_t; 2] = [vbicq_u32(live[0], t1313[0]), vbicq_u32(live[1], t1313[1])];
    if any(&t1315) {
        let t1317: u64 = (1u64).wrapping_mul(8u64);
        let t1318 = [vdupq_n_u32((t1317) as u32); 2];
        let t1319 = [vdupq_n_u32(((t1317) >> 32) as u32); 2];
        let t1320: [uint32x4_t; 2] = [vaddq_u32(x3[0], t1318[0]), vaddq_u32(x3[1], t1318[1])];
        let t1321: [uint32x4_t; 2] = [vcltq_u32(t1320[0], x3[0]), vcltq_u32(t1320[1], x3[1])];
        let t1322: [uint32x4_t; 4] = [t1320[0], t1320[1], vsubq_u32(vaddq_u32(x3[2], t1319[0]), t1321[0]), vsubq_u32(vaddq_u32(x3[3], t1319[1]), t1321[1])];
        let mut t1323: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1322[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1322[0]) as u64) as usize as *const u64).read_unaligned(); t1323[0] = vsetq_lane_u32::<0>(v as u32, t1323[0]); t1323[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1323[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1322[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1322[0]) as u64) as usize as *const u64).read_unaligned(); t1323[0] = vsetq_lane_u32::<1>(v as u32, t1323[0]); t1323[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1323[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1322[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1322[0]) as u64) as usize as *const u64).read_unaligned(); t1323[0] = vsetq_lane_u32::<2>(v as u32, t1323[0]); t1323[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1323[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1322[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1322[0]) as u64) as usize as *const u64).read_unaligned(); t1323[0] = vsetq_lane_u32::<3>(v as u32, t1323[0]); t1323[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1323[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1322[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1322[1]) as u64) as usize as *const u64).read_unaligned(); t1323[1] = vsetq_lane_u32::<0>(v as u32, t1323[1]); t1323[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1323[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1322[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1322[1]) as u64) as usize as *const u64).read_unaligned(); t1323[1] = vsetq_lane_u32::<1>(v as u32, t1323[1]); t1323[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1323[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1322[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1322[1]) as u64) as usize as *const u64).read_unaligned(); t1323[1] = vsetq_lane_u32::<2>(v as u32, t1323[1]); t1323[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1323[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1322[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1322[1]) as u64) as usize as *const u64).read_unaligned(); t1323[1] = vsetq_lane_u32::<3>(v as u32, t1323[1]); t1323[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1323[3]); }
        let t1324: [uint32x4_t; 2] = [vsubq_u32(t1311[0], x4[0]), vsubq_u32(t1311[1], x4[1])];
        let t1325: [uint32x4_t; 2] = [t1324[0], t1324[1]];
        let t1326: [uint32x4_t; 2] = [vandq_u32(t1325[0], vdupq_n_u32(63)), vandq_u32(t1325[1], vdupq_n_u32(63))];
        let t1327: [uint32x4_t; 4] = [vshlq_u32(t1323[0], vreinterpretq_s32_u32(t1326[0])), vshlq_u32(t1323[1], vreinterpretq_s32_u32(t1326[1])), vorrq_u32(vshlq_u32(t1323[2], vreinterpretq_s32_u32(t1326[0])), vshlq_u32(t1323[0], vsubq_s32(vreinterpretq_s32_u32(t1326[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1323[3], vreinterpretq_s32_u32(t1326[1])), vshlq_u32(t1323[1], vsubq_s32(vreinterpretq_s32_u32(t1326[1]), vdupq_n_s32(32))))];
        let t1328: [uint32x4_t; 4] = [vorrq_u32(x5[0], t1327[0]), vorrq_u32(x5[1], t1327[1]), vorrq_u32(x5[2], t1327[2]), vorrq_u32(x5[3], t1327[3])];
        t1314 = [vbslq_u32(t1315[0], t1328[0], t1314[0]), vbslq_u32(t1315[1], t1328[1], t1314[1]), vbslq_u32(t1315[0], t1328[2], t1314[2]), vbslq_u32(t1315[1], t1328[3], t1314[3])];
    }
    {
        t1314 = [vbslq_u32(t1316[0], x5[0], t1314[0]), vbslq_u32(t1316[1], x5[1], t1314[1]), vbslq_u32(t1316[0], x5[2], t1314[2]), vbslq_u32(t1316[1], x5[3], t1314[3])];
    }
    res = t1314;
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
        let t1329: [uint32x4_t; 4];
        if (x1is) == 8u32 && ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 {
            let t1330 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
            let t13310 = vld2q_u32((t1330 as *const u32).add(0));
            let t13311 = vld2q_u32((t1330 as *const u32).add(8));
            let t1331: [uint32x4_t; 4] = [t13310.0, t13311.0, t13310.1, t13311.1];
            t1329 = t1331;
        } else {
            let t1332: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
            let mut t1333: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1332[0]) as usize * 1) as *const u64).read_unaligned(); t1333[0] = vsetq_lane_u32::<0>(v as u32, t1333[0]); t1333[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1333[2]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1332[0]) as usize * 1) as *const u64).read_unaligned(); t1333[0] = vsetq_lane_u32::<1>(v as u32, t1333[0]); t1333[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1333[2]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1332[0]) as usize * 1) as *const u64).read_unaligned(); t1333[0] = vsetq_lane_u32::<2>(v as u32, t1333[0]); t1333[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1333[2]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1332[0]) as usize * 1) as *const u64).read_unaligned(); t1333[0] = vsetq_lane_u32::<3>(v as u32, t1333[0]); t1333[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1333[2]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1332[1]) as usize * 1) as *const u64).read_unaligned(); t1333[1] = vsetq_lane_u32::<0>(v as u32, t1333[1]); t1333[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1333[3]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1332[1]) as usize * 1) as *const u64).read_unaligned(); t1333[1] = vsetq_lane_u32::<1>(v as u32, t1333[1]); t1333[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1333[3]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1332[1]) as usize * 1) as *const u64).read_unaligned(); t1333[1] = vsetq_lane_u32::<2>(v as u32, t1333[1]); t1333[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1333[3]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1332[1]) as usize * 1) as *const u64).read_unaligned(); t1333[1] = vsetq_lane_u32::<3>(v as u32, t1333[1]); t1333[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1333[3]); }
            t1329 = t1333;
        }
        res = t1329;
        return res;
    }
    if ((x2) == (4u32)) {
        let t1334: [uint32x4_t; 2];
        if (x1is) == 4u32 && ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 {
            let t1335 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
            let t1336: [uint32x4_t; 2] = core::array::from_fn(|r| vld1q_u32((t1335 as *const u32).add(4 * r)));
            t1334 = t1336;
        } else {
            let t1337: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
            let mut t1338: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            t1338[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1337[0]) as usize * 1) as *const u32).read_unaligned() as u32), t1338[0]);
            t1338[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1337[0]) as usize * 1) as *const u32).read_unaligned() as u32), t1338[0]);
            t1338[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1337[0]) as usize * 1) as *const u32).read_unaligned() as u32), t1338[0]);
            t1338[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1337[0]) as usize * 1) as *const u32).read_unaligned() as u32), t1338[0]);
            t1338[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1337[1]) as usize * 1) as *const u32).read_unaligned() as u32), t1338[1]);
            t1338[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1337[1]) as usize * 1) as *const u32).read_unaligned() as u32), t1338[1]);
            t1338[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1337[1]) as usize * 1) as *const u32).read_unaligned() as u32), t1338[1]);
            t1338[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1337[1]) as usize * 1) as *const u32).read_unaligned() as u32), t1338[1]);
            t1334 = t1338;
        }
        let t1339: [uint32x4_t; 4] = [t1334[0], t1334[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1339;
        return res;
    }
    if ((x2) == (2u32)) {
        let t1340: [uint32x4_t; 2];
        if (x1is) == 2u32 && ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 {
            let t1341 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
            let t1342: [uint32x4_t; 2] = core::array::from_fn(|r| vmovl_u16(vld1_u16((t1341 as *const u16).add(4 * r))));
            t1340 = t1342;
        } else {
            let t1343: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
            let mut t1344: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            t1344[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1343[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1344[0]);
            t1344[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1343[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1344[0]);
            t1344[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1343[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1344[0]);
            t1344[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1343[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1344[0]);
            t1344[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1343[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1344[1]);
            t1344[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1343[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1344[1]);
            t1344[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1343[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1344[1]);
            t1344[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1343[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1344[1]);
            t1340 = t1344;
        }
        let t1345: [uint32x4_t; 4] = [t1340[0], t1340[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1345;
        return res;
    }
    if ((x2) == (1u32)) {
        let t1346: [uint32x4_t; 2];
        if (x1is) == 1u32 && ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 {
            let t1347 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
            let t1349 = vmovl_u8(vld1_u8(t1347.add(0)));
            let t1348: [uint32x4_t; 2] = [vmovl_u16(vget_low_u16(t1349)), vmovl_u16(vget_high_u16(t1349))];
            t1346 = t1348;
        } else {
            let t1350: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
            let mut t1351: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            t1351[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1350[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1351[0]);
            t1351[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1350[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1351[0]);
            t1351[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1350[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1351[0]);
            t1351[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1350[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1351[0]);
            t1351[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1350[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1351[1]);
            t1351[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1350[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1351[1]);
            t1351[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1350[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1351[1]);
            t1351[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1350[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1351[1]);
            t1346 = t1351;
        }
        let t1352: [uint32x4_t; 4] = [t1346[0], t1346[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1352;
        return res;
    }
    let t1353: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
    let t1354: [uint32x4_t; 2] = [t1353[0], t1353[1]];
    let t1355: u64 = (((x1b).wrapping_add((((arg0) as usize as u64)) as usize)) as usize as u64);
    let t1356 = [vdupq_n_u32((t1355) as u32); 2];
    let t1357 = [vdupq_n_u32(((t1355) >> 32) as u32); 2];
    let t1358: [uint32x4_t; 2] = [vaddq_u32(t1356[0], t1354[0]), vaddq_u32(t1356[1], t1354[1])];
    let t1359: [uint32x4_t; 2] = [vcltq_u32(t1358[0], t1356[0]), vcltq_u32(t1358[1], t1356[1])];
    let t1360: [uint32x4_t; 4] = [t1358[0], t1358[1], vsubq_u32(vaddq_u32(t1357[0], vdupq_n_u32(0)), t1359[0]), vsubq_u32(vaddq_u32(t1357[1], vdupq_n_u32(0)), t1359[1])];
    let t1361: u64 = 7u64;
    let t1362 = [vdupq_n_u32((t1361) as u32); 2];
    let t1363: [uint32x4_t; 2] = [vandq_u32(t1360[0], t1362[0]), vandq_u32(t1360[1], t1362[1])];
    let t1364: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
    let t1365: [uint32x4_t; 2] = [t1364[0], t1364[1]];
    let t1366: u64 = (((arg0).wrapping_add((x1b) as usize)) as usize as u64);
    let t1367 = [vdupq_n_u32((t1366) as u32); 2];
    let t1368 = [vdupq_n_u32(((t1366) >> 32) as u32); 2];
    let t1369: [uint32x4_t; 2] = [vaddq_u32(t1367[0], t1365[0]), vaddq_u32(t1367[1], t1365[1])];
    let t1370: [uint32x4_t; 2] = [vcltq_u32(t1369[0], t1367[0]), vcltq_u32(t1369[1], t1367[1])];
    let t1371: [uint32x4_t; 4] = [t1369[0], t1369[1], vsubq_u32(vaddq_u32(t1368[0], vdupq_n_u32(0)), t1370[0]), vsubq_u32(vaddq_u32(t1368[1], vdupq_n_u32(0)), t1370[1])];
    let t1372: [uint32x4_t; 2] = [vcltq_u32(t1371[0], t1363[0]), vcltq_u32(t1371[1], t1363[1])];
    let t1373: [uint32x4_t; 4] = [vsubq_u32(t1371[0], t1363[0]), vsubq_u32(t1371[1], t1363[1]), vaddq_u32(vsubq_u32(t1371[2], vdupq_n_u32(0)), t1372[0]), vaddq_u32(vsubq_u32(t1371[3], vdupq_n_u32(0)), t1372[1])];
    x3 = t1373;
    let t1374: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
    let t1375: [uint32x4_t; 2] = [t1374[0], t1374[1]];
    let t1376: u64 = (((x1b).wrapping_add((((arg0) as usize as u64)) as usize)) as usize as u64);
    let t1377 = [vdupq_n_u32((t1376) as u32); 2];
    let t1378 = [vdupq_n_u32(((t1376) >> 32) as u32); 2];
    let t1379: [uint32x4_t; 2] = [vaddq_u32(t1377[0], t1375[0]), vaddq_u32(t1377[1], t1375[1])];
    let t1380: [uint32x4_t; 2] = [vcltq_u32(t1379[0], t1377[0]), vcltq_u32(t1379[1], t1377[1])];
    let t1381: [uint32x4_t; 4] = [t1379[0], t1379[1], vsubq_u32(vaddq_u32(t1378[0], vdupq_n_u32(0)), t1380[0]), vsubq_u32(vaddq_u32(t1378[1], vdupq_n_u32(0)), t1380[1])];
    let t1382: u64 = 7u64;
    let t1383 = [vdupq_n_u32((t1382) as u32); 2];
    let t1384: [uint32x4_t; 2] = [vandq_u32(t1381[0], t1383[0]), vandq_u32(t1381[1], t1383[1])];
    let t1385 = [vdupq_n_u32(8u32); 2];
    let t1386: [uint32x4_t; 2] = [vmulq_u32(t1384[0], t1385[0]), vmulq_u32(t1384[1], t1385[1])];
    x4 = t1386;
    let mut t1387: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x3[2]) as u64) << 32 | vgetq_lane_u32::<0>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1387[0] = vsetq_lane_u32::<0>(v as u32, t1387[0]); t1387[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1387[2]); }
    { let v = (((vgetq_lane_u32::<1>(x3[2]) as u64) << 32 | vgetq_lane_u32::<1>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1387[0] = vsetq_lane_u32::<1>(v as u32, t1387[0]); t1387[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1387[2]); }
    { let v = (((vgetq_lane_u32::<2>(x3[2]) as u64) << 32 | vgetq_lane_u32::<2>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1387[0] = vsetq_lane_u32::<2>(v as u32, t1387[0]); t1387[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1387[2]); }
    { let v = (((vgetq_lane_u32::<3>(x3[2]) as u64) << 32 | vgetq_lane_u32::<3>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1387[0] = vsetq_lane_u32::<3>(v as u32, t1387[0]); t1387[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1387[2]); }
    { let v = (((vgetq_lane_u32::<0>(x3[3]) as u64) << 32 | vgetq_lane_u32::<0>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1387[1] = vsetq_lane_u32::<0>(v as u32, t1387[1]); t1387[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1387[3]); }
    { let v = (((vgetq_lane_u32::<1>(x3[3]) as u64) << 32 | vgetq_lane_u32::<1>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1387[1] = vsetq_lane_u32::<1>(v as u32, t1387[1]); t1387[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1387[3]); }
    { let v = (((vgetq_lane_u32::<2>(x3[3]) as u64) << 32 | vgetq_lane_u32::<2>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1387[1] = vsetq_lane_u32::<2>(v as u32, t1387[1]); t1387[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1387[3]); }
    { let v = (((vgetq_lane_u32::<3>(x3[3]) as u64) << 32 | vgetq_lane_u32::<3>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1387[1] = vsetq_lane_u32::<3>(v as u32, t1387[1]); t1387[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1387[3]); }
    let t1388: [uint32x4_t; 2] = [x4[0], x4[1]];
    let t1389: [uint32x4_t; 2] = [vandq_u32(t1388[0], vdupq_n_u32(63)), vandq_u32(t1388[1], vdupq_n_u32(63))];
    let t1390: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t1387[0], vnegq_s32(vreinterpretq_s32_u32(t1389[0]))), vshlq_u32(t1387[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1389[0])))), vorrq_u32(vshlq_u32(t1387[1], vnegq_s32(vreinterpretq_s32_u32(t1389[1]))), vshlq_u32(t1387[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1389[1])))), vshlq_u32(t1387[2], vnegq_s32(vreinterpretq_s32_u32(t1389[0]))), vshlq_u32(t1387[3], vnegq_s32(vreinterpretq_s32_u32(t1389[1])))];
    x5 = t1390;
    let t1391 = [vdupq_n_u32(0u32); 2];
    let t1392: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t1391[0])), vmvnq_u32(vceqq_u32(x4[1], t1391[1]))];
    let t1393 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
    let t1394: [uint32x4_t; 2] = [vaddq_u32(x4[0], t1393[0]), vaddq_u32(x4[1], t1393[1])];
    let t1395 = [vdupq_n_u32(64u32); 2];
    let t1396: [uint32x4_t; 2] = [vcgtq_u32(t1394[0], t1395[0]), vcgtq_u32(t1394[1], t1395[1])];
    let t1397: [uint32x4_t; 2] = [vandq_u32(t1392[0], t1396[0]), vandq_u32(t1392[1], t1396[1])];
    let mut t1398: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1399: [uint32x4_t; 2] = [vmvnq_u32(t1397[0]), vmvnq_u32(t1397[1])];
    if any(&t1397) {
        let t1400: u64 = (1u64).wrapping_mul(8u64);
        let t1401 = [vdupq_n_u32((t1400) as u32); 2];
        let t1402 = [vdupq_n_u32(((t1400) >> 32) as u32); 2];
        let t1403: [uint32x4_t; 2] = [vaddq_u32(x3[0], t1401[0]), vaddq_u32(x3[1], t1401[1])];
        let t1404: [uint32x4_t; 2] = [vcltq_u32(t1403[0], x3[0]), vcltq_u32(t1403[1], x3[1])];
        let t1405: [uint32x4_t; 4] = [t1403[0], t1403[1], vsubq_u32(vaddq_u32(x3[2], t1402[0]), t1404[0]), vsubq_u32(vaddq_u32(x3[3], t1402[1]), t1404[1])];
        let mut t1406: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1405[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1405[0]) as u64) as usize as *const u64).read_unaligned(); t1406[0] = vsetq_lane_u32::<0>(v as u32, t1406[0]); t1406[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1406[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1405[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1405[0]) as u64) as usize as *const u64).read_unaligned(); t1406[0] = vsetq_lane_u32::<1>(v as u32, t1406[0]); t1406[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1406[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1405[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1405[0]) as u64) as usize as *const u64).read_unaligned(); t1406[0] = vsetq_lane_u32::<2>(v as u32, t1406[0]); t1406[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1406[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1405[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1405[0]) as u64) as usize as *const u64).read_unaligned(); t1406[0] = vsetq_lane_u32::<3>(v as u32, t1406[0]); t1406[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1406[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1405[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1405[1]) as u64) as usize as *const u64).read_unaligned(); t1406[1] = vsetq_lane_u32::<0>(v as u32, t1406[1]); t1406[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1406[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1405[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1405[1]) as u64) as usize as *const u64).read_unaligned(); t1406[1] = vsetq_lane_u32::<1>(v as u32, t1406[1]); t1406[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1406[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1405[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1405[1]) as u64) as usize as *const u64).read_unaligned(); t1406[1] = vsetq_lane_u32::<2>(v as u32, t1406[1]); t1406[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1406[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1405[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1405[1]) as u64) as usize as *const u64).read_unaligned(); t1406[1] = vsetq_lane_u32::<3>(v as u32, t1406[1]); t1406[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1406[3]); }
        let t1407: [uint32x4_t; 2] = [vsubq_u32(t1395[0], x4[0]), vsubq_u32(t1395[1], x4[1])];
        let t1408: [uint32x4_t; 2] = [t1407[0], t1407[1]];
        let t1409: [uint32x4_t; 2] = [vandq_u32(t1408[0], vdupq_n_u32(63)), vandq_u32(t1408[1], vdupq_n_u32(63))];
        let t1410: [uint32x4_t; 4] = [vshlq_u32(t1406[0], vreinterpretq_s32_u32(t1409[0])), vshlq_u32(t1406[1], vreinterpretq_s32_u32(t1409[1])), vorrq_u32(vshlq_u32(t1406[2], vreinterpretq_s32_u32(t1409[0])), vshlq_u32(t1406[0], vsubq_s32(vreinterpretq_s32_u32(t1409[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1406[3], vreinterpretq_s32_u32(t1409[1])), vshlq_u32(t1406[1], vsubq_s32(vreinterpretq_s32_u32(t1409[1]), vdupq_n_s32(32))))];
        let t1411: [uint32x4_t; 4] = [vorrq_u32(x5[0], t1410[0]), vorrq_u32(x5[1], t1410[1]), vorrq_u32(x5[2], t1410[2]), vorrq_u32(x5[3], t1410[3])];
        t1398 = [vbslq_u32(t1397[0], t1411[0], t1398[0]), vbslq_u32(t1397[1], t1411[1], t1398[1]), vbslq_u32(t1397[0], t1411[2], t1398[2]), vbslq_u32(t1397[1], t1411[3], t1398[3])];
    }
    {
        t1398 = [vbslq_u32(t1399[0], x5[0], t1398[0]), vbslq_u32(t1399[1], x5[1], t1398[1]), vbslq_u32(t1399[0], x5[2], t1398[2]), vbslq_u32(t1399[1], x5[3], t1398[3])];
    }
    res = t1398;
    return res;
}

#[inline(always)]
unsafe fn b20_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t1412: u64;
    if ((x0) >= (64u32)) {
        t1412 = !(0u64);
    } else {
        t1412 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t1412;
    return res;
}

#[inline(always)]
unsafe fn b20_s_f(l: &Launch, lid0: u32, x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t1413: u64;
    if ((x0) >= (64u32)) {
        t1413 = !(0u64);
    } else {
        t1413 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t1413;
    return res;
}

#[inline(always)]
unsafe fn b21_r0sss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1: u32, x2: u32, x3: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u64 = 0;
    let mut live = k;
    let mut res: u64 = 0;
    let t1414 = b19_r0ss_p(l, lid0, live, arg0, (((x1) as u64)).wrapping_add(((x2) as u64)), x3);
    x4 = t1414;
    let t1415: u64;
    if ((x3) >= (8u32)) {
        t1415 = x4;
    } else {
        let t1416 = b20_s_p(l, lid0, live, (8u32).wrapping_mul(x3));
        t1415 = ((x4) & (t1416));
    }
    res = t1415;
    return res;
}

#[inline(always)]
unsafe fn b21_r0sss_f(l: &Launch, lid0: u32, arg0: *const u8, x1: u32, x2: u32, x3: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u64 = 0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t1417 = b19_r0ss_f(l, lid0, arg0, (((x1) as u64)).wrapping_add(((x2) as u64)), x3);
    x4 = t1417;
    let t1418: u64;
    if ((x3) >= (8u32)) {
        t1418 = x4;
    } else {
        let t1419 = b20_s_f(l, lid0, (8u32).wrapping_mul(x3));
        t1418 = ((x4) & (t1419));
    }
    res = t1418;
    return res;
}

#[inline(always)]
unsafe fn b21_r0sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1: u32, x2: [uint32x4_t; 2], x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1420 = b19_r0al1s_p(l, lid0, live, arg0, ((((x1) as u64)) as usize as *const u8), x2, x3);
    x4 = t1420;
    let t1421: [uint32x4_t; 4];
    if ((x3) >= (8u32)) {
        t1421 = x4;
    } else {
        let t1422 = b20_s_p(l, lid0, live, (8u32).wrapping_mul(x3));
        let t1423: u64 = t1422;
        let t1424 = [vdupq_n_u32((t1423) as u32); 2];
        let t1425 = [vdupq_n_u32(((t1423) >> 32) as u32); 2];
        let t1426: [uint32x4_t; 4] = [vandq_u32(x4[0], t1424[0]), vandq_u32(x4[1], t1424[1]), vandq_u32(x4[2], t1425[0]), vandq_u32(x4[3], t1425[1])];
        t1421 = t1426;
    }
    res = t1421;
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
    let mut t1427: bool = !(arg4).is_null();
    if t1427 {
        let mut t1428: bool = ((l.group[0]) >= ((arg4.wrapping_add((((x5) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        if !t1428 {
            t1428 = ((l.group[1]) >= ((arg4.wrapping_add(((((x5).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        let mut t1429: bool = t1428;
        if !t1429 {
            t1429 = ((l.group[2]) >= ((arg4.wrapping_add(((((x5).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        t1427 = t1429;
    }
    if t1427 {
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
            let t1430: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
            let mut live = t1430;
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
            let t1431: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
            let mut live = t1431;
            let gid0 = l.group[0] * l.block[0] + lid0;
            'chunk: {
                b0_r0r1r2sr6gi_p(l, lid0, live, arg0, arg1, arg2, x3, arg6, lid0, phase, st.add((lid0 / 8) as usize * 320));
            }
        }
    }
}
