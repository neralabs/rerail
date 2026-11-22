//! `parent_link` at 8 lanes (2 NEON registers), written by rerail-ukl-vec's Rust (`rs.rs`) from its vector tape: do
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
    let mut x56: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x64: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let t1: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
    let t2 = [vdupq_n_u32(a0); 2];
    let t3: [uint32x4_t; 2] = [vcgeq_u32(t1[0], t2[0]), vcgeq_u32(t1[1], t2[1])];
    let t4: [uint32x4_t; 2] = [vandq_u32(live[0], t3[0]), vandq_u32(live[1], t3[1])];
    if any(&t4) {
        live = [vbicq_u32(live[0], t4[0]), vbicq_u32(live[1], t4[1])];
    }
    if any(&live) {
        let mut t5: [uint32x4_t; 2];
        if all(&live) {
            let t6 = (arg2 as *const u32).add((gid0) as usize);
            t5 = [vld1q_u32(t6.add(0)), vld1q_u32(t6.add(4))];
        } else {
            let t7 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t1[0]), vandq_u32(live[1], t1[1]))));
            let t8: [uint32x4_t; 2] = [vbslq_u32(live[0], t1[0], t7), vbslq_u32(live[1], t1[1], t7)];
            let t9 = arg2 as *const u32;
            let mut t10: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            t10[0] = vld1q_lane_u32::<0>(t9.add(vgetq_lane_u32::<0>(t8[0]) as usize), t10[0]);
            t10[0] = vld1q_lane_u32::<1>(t9.add(vgetq_lane_u32::<1>(t8[0]) as usize), t10[0]);
            t10[0] = vld1q_lane_u32::<2>(t9.add(vgetq_lane_u32::<2>(t8[0]) as usize), t10[0]);
            t10[0] = vld1q_lane_u32::<3>(t9.add(vgetq_lane_u32::<3>(t8[0]) as usize), t10[0]);
            t10[1] = vld1q_lane_u32::<0>(t9.add(vgetq_lane_u32::<0>(t8[1]) as usize), t10[1]);
            t10[1] = vld1q_lane_u32::<1>(t9.add(vgetq_lane_u32::<1>(t8[1]) as usize), t10[1]);
            t10[1] = vld1q_lane_u32::<2>(t9.add(vgetq_lane_u32::<2>(t8[1]) as usize), t10[1]);
            t10[1] = vld1q_lane_u32::<3>(t9.add(vgetq_lane_u32::<3>(t8[1]) as usize), t10[1]);
            t5 = t10;
        }
        x56 = t5;
        let t11 = [vdupq_n_u32(0u32); 2];
        x64 = t11;
        let mut t12 = live;
        loop {
            // Lanes part: each its own rounds.
            let t14 = [vdupq_n_u32(7u32); 2];
            let t15: [uint32x4_t; 2] = [vcltq_u32(x64[0], t14[0]), vcltq_u32(x64[1], t14[1])];
            let t16 = [vdupq_n_u32(4294967295u32); 2];
            let t17: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x56[0], t16[0])), vmvnq_u32(vceqq_u32(x56[1], t16[1]))];
            let t18: [uint32x4_t; 2] = [vandq_u32(t15[0], t17[0]), vandq_u32(t15[1], t17[1])];
            let t19 = [vdupq_n_u32(2147483648u32); 2];
            let t20: [uint32x4_t; 2] = [vandq_u32(x56[0], t19[0]), vandq_u32(x56[1], t19[1])];
            let t21: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t20[0], t11[0])), vmvnq_u32(vceqq_u32(t20[1], t11[1]))];
            let t22: [uint32x4_t; 2] = [vandq_u32(t18[0], t21[0]), vandq_u32(t18[1], t21[1])];
            t12 = [vandq_u32(t12[0], t22[0]), vandq_u32(t12[1], t22[1])];
            if !(any(&t12)) { break; }
            let t24 = [vdupq_n_u32((2147483648u32).wrapping_sub(1u32)); 2];
            let t25: [uint32x4_t; 2] = [vandq_u32(x56[0], t24[0]), vandq_u32(x56[1], t24[1])];
            let t26 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(t12[0], t25[0]), vandq_u32(t12[1], t25[1]))));
            let t27: [uint32x4_t; 2] = [vbslq_u32(t12[0], t25[0], t26), vbslq_u32(t12[1], t25[1], t26)];
            let t28 = arg2 as *const u32;
            let mut t29: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            t29[0] = vld1q_lane_u32::<0>(t28.add(vgetq_lane_u32::<0>(t27[0]) as usize), t29[0]);
            t29[0] = vld1q_lane_u32::<1>(t28.add(vgetq_lane_u32::<1>(t27[0]) as usize), t29[0]);
            t29[0] = vld1q_lane_u32::<2>(t28.add(vgetq_lane_u32::<2>(t27[0]) as usize), t29[0]);
            t29[0] = vld1q_lane_u32::<3>(t28.add(vgetq_lane_u32::<3>(t27[0]) as usize), t29[0]);
            t29[1] = vld1q_lane_u32::<0>(t28.add(vgetq_lane_u32::<0>(t27[1]) as usize), t29[1]);
            t29[1] = vld1q_lane_u32::<1>(t28.add(vgetq_lane_u32::<1>(t27[1]) as usize), t29[1]);
            t29[1] = vld1q_lane_u32::<2>(t28.add(vgetq_lane_u32::<2>(t27[1]) as usize), t29[1]);
            t29[1] = vld1q_lane_u32::<3>(t28.add(vgetq_lane_u32::<3>(t27[1]) as usize), t29[1]);
            x56 = [vbslq_u32(t12[0], t29[0], x56[0]), vbslq_u32(t12[1], t29[1], x56[1])];
            let t30: [uint32x4_t; 2] = [vandq_u32(t12[0], live[0]), vandq_u32(t12[1], live[1])];
            let t31 = [vdupq_n_u32(1u32); 2];
            let t32: [uint32x4_t; 2] = [vaddq_u32(x64[0], t31[0]), vaddq_u32(x64[1], t31[1])];
            let t33 = x64;
            x64 = t32;
        }
        let t34 = [vdupq_n_u32(4294967295u32); 2];
        let t35: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x56[0], t34[0])), vmvnq_u32(vceqq_u32(x56[1], t34[1]))];
        let t36 = [vdupq_n_u32(2147483648u32); 2];
        let t37: [uint32x4_t; 2] = [vandq_u32(x56[0], t36[0]), vandq_u32(x56[1], t36[1])];
        let t38: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t37[0], t11[0])), vmvnq_u32(vceqq_u32(t37[1], t11[1]))];
        let t39: [uint32x4_t; 2] = [vandq_u32(t35[0], t38[0]), vandq_u32(t35[1], t38[1])];
        let t40: [uint32x4_t; 2] = [vandq_u32(live[0], t39[0]), vandq_u32(live[1], t39[1])];
        if any(&t40) {
            let t41 = b1_r0r1g_p(l, lid0, t40, arg0, arg1);
            x56 = [vbslq_u32(t40[0], t41[0], x56[0]), vbslq_u32(t40[1], t41[1], x56[1])];
        }
        let t42 = (arg3 as *mut u32).add((gid0) as usize);
        if all(&live) {
            vst1q_u32(t42.add(0), x56[0]);
            vst1q_u32(t42.add(4), x56[1]);
        } else {
            let mut t43w = 0u32;
            let t43 = core::hint::black_box(&raw mut t43w);
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t42.add(0), t43).write(vgetq_lane_u32::<0>(x56[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t42.add(1), t43).write(vgetq_lane_u32::<1>(x56[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t42.add(2), t43).write(vgetq_lane_u32::<2>(x56[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t42.add(3), t43).write(vgetq_lane_u32::<3>(x56[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t42.add(4), t43).write(vgetq_lane_u32::<0>(x56[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t42.add(5), t43).write(vgetq_lane_u32::<1>(x56[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t42.add(6), t43).write(vgetq_lane_u32::<2>(x56[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t42.add(7), t43).write(vgetq_lane_u32::<3>(x56[1]));
        }
    }
}

#[inline(always)]
unsafe fn b0_r0r1r2r3sg_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, arg2: *const u8, arg3: *const u8, x32: *const u8) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x32 as *const u32).read_unaligned();
    let mut x56: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x64: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    if ((gid0) as u64 + 7) < ((a0) as u64) {
        let t44 = (arg2 as *const u32).add((gid0) as usize);
        let t45: [uint32x4_t; 2] = [vld1q_u32(t44.add(0)), vld1q_u32(t44.add(4))];
        x56 = t45;
        let t46 = [vdupq_n_u32(0u32); 2];
        x64 = t46;
        let mut t56 = live;
        loop {
            // Lanes part: each its own rounds.
            let t58 = [vdupq_n_u32(7u32); 2];
            let t59: [uint32x4_t; 2] = [vcltq_u32(x64[0], t58[0]), vcltq_u32(x64[1], t58[1])];
            let t60 = [vdupq_n_u32(4294967295u32); 2];
            let t61: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x56[0], t60[0])), vmvnq_u32(vceqq_u32(x56[1], t60[1]))];
            let t62: [uint32x4_t; 2] = [vandq_u32(t59[0], t61[0]), vandq_u32(t59[1], t61[1])];
            let t63 = [vdupq_n_u32(2147483648u32); 2];
            let t64: [uint32x4_t; 2] = [vandq_u32(x56[0], t63[0]), vandq_u32(x56[1], t63[1])];
            let t65: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t64[0], t46[0])), vmvnq_u32(vceqq_u32(t64[1], t46[1]))];
            let t66: [uint32x4_t; 2] = [vandq_u32(t62[0], t65[0]), vandq_u32(t62[1], t65[1])];
            t56 = [vandq_u32(t56[0], t66[0]), vandq_u32(t56[1], t66[1])];
            if !(any(&t56)) { break; }
            let t68 = [vdupq_n_u32((2147483648u32).wrapping_sub(1u32)); 2];
            let t69: [uint32x4_t; 2] = [vandq_u32(x56[0], t68[0]), vandq_u32(x56[1], t68[1])];
            let t70: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
            let t71: [uint32x4_t; 2] = [vbslq_u32(t56[0], t69[0], t70[0]), vbslq_u32(t56[1], t69[1], t70[1])];
            let t72 = arg2 as *const u32;
            let mut t73: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            t73[0] = vld1q_lane_u32::<0>(t72.add(vgetq_lane_u32::<0>(t71[0]) as usize), t73[0]);
            t73[0] = vld1q_lane_u32::<1>(t72.add(vgetq_lane_u32::<1>(t71[0]) as usize), t73[0]);
            t73[0] = vld1q_lane_u32::<2>(t72.add(vgetq_lane_u32::<2>(t71[0]) as usize), t73[0]);
            t73[0] = vld1q_lane_u32::<3>(t72.add(vgetq_lane_u32::<3>(t71[0]) as usize), t73[0]);
            t73[1] = vld1q_lane_u32::<0>(t72.add(vgetq_lane_u32::<0>(t71[1]) as usize), t73[1]);
            t73[1] = vld1q_lane_u32::<1>(t72.add(vgetq_lane_u32::<1>(t71[1]) as usize), t73[1]);
            t73[1] = vld1q_lane_u32::<2>(t72.add(vgetq_lane_u32::<2>(t71[1]) as usize), t73[1]);
            t73[1] = vld1q_lane_u32::<3>(t72.add(vgetq_lane_u32::<3>(t71[1]) as usize), t73[1]);
            x56 = [vbslq_u32(t56[0], t73[0], x56[0]), vbslq_u32(t56[1], t73[1], x56[1])];
            let t74: [uint32x4_t; 2] = [vandq_u32(t56[0], live[0]), vandq_u32(t56[1], live[1])];
            let t75 = [vdupq_n_u32(1u32); 2];
            let t76: [uint32x4_t; 2] = [vaddq_u32(x64[0], t75[0]), vaddq_u32(x64[1], t75[1])];
            let t77 = x64;
            x64 = t76;
        }
        let t78 = [vdupq_n_u32(4294967295u32); 2];
        let t79: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x56[0], t78[0])), vmvnq_u32(vceqq_u32(x56[1], t78[1]))];
        let t80 = [vdupq_n_u32(2147483648u32); 2];
        let t81: [uint32x4_t; 2] = [vandq_u32(x56[0], t80[0]), vandq_u32(x56[1], t80[1])];
        let t82: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t81[0], t46[0])), vmvnq_u32(vceqq_u32(t81[1], t46[1]))];
        let t83: [uint32x4_t; 2] = [vandq_u32(t79[0], t82[0]), vandq_u32(t79[1], t82[1])];
        if any(&t83) {
            let t84 = b1_r0r1g_p(l, lid0, t83, arg0, arg1);
            x56 = [vbslq_u32(t83[0], t84[0], x56[0]), vbslq_u32(t83[1], t84[1], x56[1])];
        }
        let t85 = (arg3 as *mut u32).add((gid0) as usize);
        vst1q_u32(t85.add(0), x56[0]);
        vst1q_u32(t85.add(4), x56[1]);
    } else if (((gid0) as u64 + 7) <= u32::MAX as u64 && ((gid0) as u64) >= ((a0) as u64)) {
        return;
    } else {
        // Lanes part: the chunk's lanes on both sides of the bound.
        let t86: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t87 = [vdupq_n_u32(a0); 2];
        let t88: [uint32x4_t; 2] = [vcgeq_u32(t86[0], t87[0]), vcgeq_u32(t86[1], t87[1])];
        let t89: [uint32x4_t; 2] = [vandq_u32(live[0], t88[0]), vandq_u32(live[1], t88[1])];
        if any(&t89) {
            live = [vbicq_u32(live[0], t89[0]), vbicq_u32(live[1], t89[1])];
        }
        if any(&live) {
            let mut t90: [uint32x4_t; 2];
            if all(&live) {
                let t91 = (arg2 as *const u32).add((gid0) as usize);
                t90 = [vld1q_u32(t91.add(0)), vld1q_u32(t91.add(4))];
            } else {
                let t92 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t86[0]), vandq_u32(live[1], t86[1]))));
                let t93: [uint32x4_t; 2] = [vbslq_u32(live[0], t86[0], t92), vbslq_u32(live[1], t86[1], t92)];
                let t94 = arg2 as *const u32;
                let mut t95: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                t95[0] = vld1q_lane_u32::<0>(t94.add(vgetq_lane_u32::<0>(t93[0]) as usize), t95[0]);
                t95[0] = vld1q_lane_u32::<1>(t94.add(vgetq_lane_u32::<1>(t93[0]) as usize), t95[0]);
                t95[0] = vld1q_lane_u32::<2>(t94.add(vgetq_lane_u32::<2>(t93[0]) as usize), t95[0]);
                t95[0] = vld1q_lane_u32::<3>(t94.add(vgetq_lane_u32::<3>(t93[0]) as usize), t95[0]);
                t95[1] = vld1q_lane_u32::<0>(t94.add(vgetq_lane_u32::<0>(t93[1]) as usize), t95[1]);
                t95[1] = vld1q_lane_u32::<1>(t94.add(vgetq_lane_u32::<1>(t93[1]) as usize), t95[1]);
                t95[1] = vld1q_lane_u32::<2>(t94.add(vgetq_lane_u32::<2>(t93[1]) as usize), t95[1]);
                t95[1] = vld1q_lane_u32::<3>(t94.add(vgetq_lane_u32::<3>(t93[1]) as usize), t95[1]);
                t90 = t95;
            }
            x56 = t90;
            let t96 = [vdupq_n_u32(0u32); 2];
            x64 = t96;
            let mut t97 = live;
            loop {
                // Lanes part: each its own rounds.
                let t99 = [vdupq_n_u32(7u32); 2];
                let t100: [uint32x4_t; 2] = [vcltq_u32(x64[0], t99[0]), vcltq_u32(x64[1], t99[1])];
                let t101 = [vdupq_n_u32(4294967295u32); 2];
                let t102: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x56[0], t101[0])), vmvnq_u32(vceqq_u32(x56[1], t101[1]))];
                let t103: [uint32x4_t; 2] = [vandq_u32(t100[0], t102[0]), vandq_u32(t100[1], t102[1])];
                let t104 = [vdupq_n_u32(2147483648u32); 2];
                let t105: [uint32x4_t; 2] = [vandq_u32(x56[0], t104[0]), vandq_u32(x56[1], t104[1])];
                let t106: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t105[0], t96[0])), vmvnq_u32(vceqq_u32(t105[1], t96[1]))];
                let t107: [uint32x4_t; 2] = [vandq_u32(t103[0], t106[0]), vandq_u32(t103[1], t106[1])];
                t97 = [vandq_u32(t97[0], t107[0]), vandq_u32(t97[1], t107[1])];
                if !(any(&t97)) { break; }
                let t109 = [vdupq_n_u32((2147483648u32).wrapping_sub(1u32)); 2];
                let t110: [uint32x4_t; 2] = [vandq_u32(x56[0], t109[0]), vandq_u32(x56[1], t109[1])];
                let t111 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(t97[0], t110[0]), vandq_u32(t97[1], t110[1]))));
                let t112: [uint32x4_t; 2] = [vbslq_u32(t97[0], t110[0], t111), vbslq_u32(t97[1], t110[1], t111)];
                let t113 = arg2 as *const u32;
                let mut t114: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                t114[0] = vld1q_lane_u32::<0>(t113.add(vgetq_lane_u32::<0>(t112[0]) as usize), t114[0]);
                t114[0] = vld1q_lane_u32::<1>(t113.add(vgetq_lane_u32::<1>(t112[0]) as usize), t114[0]);
                t114[0] = vld1q_lane_u32::<2>(t113.add(vgetq_lane_u32::<2>(t112[0]) as usize), t114[0]);
                t114[0] = vld1q_lane_u32::<3>(t113.add(vgetq_lane_u32::<3>(t112[0]) as usize), t114[0]);
                t114[1] = vld1q_lane_u32::<0>(t113.add(vgetq_lane_u32::<0>(t112[1]) as usize), t114[1]);
                t114[1] = vld1q_lane_u32::<1>(t113.add(vgetq_lane_u32::<1>(t112[1]) as usize), t114[1]);
                t114[1] = vld1q_lane_u32::<2>(t113.add(vgetq_lane_u32::<2>(t112[1]) as usize), t114[1]);
                t114[1] = vld1q_lane_u32::<3>(t113.add(vgetq_lane_u32::<3>(t112[1]) as usize), t114[1]);
                x56 = [vbslq_u32(t97[0], t114[0], x56[0]), vbslq_u32(t97[1], t114[1], x56[1])];
                let t115: [uint32x4_t; 2] = [vandq_u32(t97[0], live[0]), vandq_u32(t97[1], live[1])];
                let t116 = [vdupq_n_u32(1u32); 2];
                let t117: [uint32x4_t; 2] = [vaddq_u32(x64[0], t116[0]), vaddq_u32(x64[1], t116[1])];
                let t118 = x64;
                x64 = t117;
            }
            let t119 = [vdupq_n_u32(4294967295u32); 2];
            let t120: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x56[0], t119[0])), vmvnq_u32(vceqq_u32(x56[1], t119[1]))];
            let t121 = [vdupq_n_u32(2147483648u32); 2];
            let t122: [uint32x4_t; 2] = [vandq_u32(x56[0], t121[0]), vandq_u32(x56[1], t121[1])];
            let t123: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t122[0], t96[0])), vmvnq_u32(vceqq_u32(t122[1], t96[1]))];
            let t124: [uint32x4_t; 2] = [vandq_u32(t120[0], t123[0]), vandq_u32(t120[1], t123[1])];
            let t125: [uint32x4_t; 2] = [vandq_u32(live[0], t124[0]), vandq_u32(live[1], t124[1])];
            if any(&t125) {
                let t126 = b1_r0r1g_p(l, lid0, t125, arg0, arg1);
                x56 = [vbslq_u32(t125[0], t126[0], x56[0]), vbslq_u32(t125[1], t126[1], x56[1])];
            }
            let t127 = (arg3 as *mut u32).add((gid0) as usize);
            if all(&live) {
                vst1q_u32(t127.add(0), x56[0]);
                vst1q_u32(t127.add(4), x56[1]);
            } else {
                let mut t128w = 0u32;
                let t128 = core::hint::black_box(&raw mut t128w);
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t127.add(0), t128).write(vgetq_lane_u32::<0>(x56[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t127.add(1), t128).write(vgetq_lane_u32::<1>(x56[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t127.add(2), t128).write(vgetq_lane_u32::<2>(x56[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t127.add(3), t128).write(vgetq_lane_u32::<3>(x56[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t127.add(4), t128).write(vgetq_lane_u32::<0>(x56[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t127.add(5), t128).write(vgetq_lane_u32::<1>(x56[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t127.add(6), t128).write(vgetq_lane_u32::<2>(x56[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t127.add(7), t128).write(vgetq_lane_u32::<3>(x56[1]));
            }
        }
    }
}

#[inline(always)]
unsafe fn b1_r0r1g_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t129 = b2_r0r1g_p(l, lid0, live, arg0, arg1);
    x24 = t129;
    let mut t130 = live;
    loop {
        // Lanes part: each its own rounds.
        let t132 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
        let t133: [uint64x2_t; 4] = [vandq_u64(x24[0], t132[0]), vandq_u64(x24[1], t132[1]), vandq_u64(x24[2], t132[2]), vandq_u64(x24[3], t132[3])];
        let t134 = [vdupq_n_u64(0u64); 4];
        let t135: [uint64x2_t; 4] = [vceqq_u64(t133[0], t134[0]), vceqq_u64(t133[1], t134[1]), vceqq_u64(t133[2], t134[2]), vceqq_u64(t133[3], t134[3])];
        let t136: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t135[0]), vmovn_u64(t135[1])), vcombine_u32(vmovn_u64(t135[2]), vmovn_u64(t135[3]))];
        let t137: [uint32x4_t; 2] = [vmvnq_u32(t136[0]), vmvnq_u32(t136[1])];
        t130 = [vandq_u32(t130[0], t137[0]), vandq_u32(t130[1], t137[1])];
        if !(any(&t130)) { break; }
        let t139: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x24[0]), vmovn_u64(x24[1])), vcombine_u32(vmovn_u64(x24[2]), vmovn_u64(x24[3]))];
        let t140 = b2_r0r1l_p(l, lid0, t130, arg0, arg1, t139);
        x24 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t130[0])))), t140[0], x24[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t130[0])))), t140[1], x24[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t130[1])))), t140[2], x24[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t130[1])))), t140[3], x24[3])];
    }
    let t141: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x24[0]), vmovn_u64(x24[1])), vcombine_u32(vmovn_u64(x24[2]), vmovn_u64(x24[3]))];
    res = t141;
    return res;
}

#[inline(always)]
unsafe fn b2_r0r1g_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x32: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t142: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
    let t143 = [vdupq_n_u32(0u32); 2];
    let t144: [uint32x4_t; 2] = [vceqq_u32(t142[0], t143[0]), vceqq_u32(t142[1], t143[1])];
    let t145: [uint32x4_t; 2] = [vandq_u32(live[0], t144[0]), vandq_u32(live[1], t144[1])];
    if any(&t145) {
        let t146 = [vdupq_n_u64(4294967295u64); 4];
        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t145[0])))), t146[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t145[0])))), t146[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t145[1])))), t146[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t145[1])))), t146[3], res[3])];
        live = [vbicq_u32(live[0], t145[0]), vbicq_u32(live[1], t145[1])];
    }
    if any(&live) {
        let t147 = [vdupq_n_u32(1u32); 2];
        let t148: [uint32x4_t; 2] = [vsubq_u32(t142[0], t147[0]), vsubq_u32(t142[1], t147[1])];
        x24 = [vbslq_u32(live[0], t148[0], x24[0]), vbslq_u32(live[1], t148[1], x24[1])];
        let t149 = b3_r0r1l_p(l, lid0, live, arg0, arg1, x24);
        x32 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t149[0], x32[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t149[1], x32[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t149[2], x32[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t149[3], x32[3])];
        let t150 = [vdupq_n_u64(1u64); 4];
        let t151: [uint64x2_t; 4] = [vandq_u64(x32[0], t150[0]), vandq_u64(x32[1], t150[1]), vandq_u64(x32[2], t150[2]), vandq_u64(x32[3], t150[3])];
        let t152 = [vdupq_n_u64(0u64); 4];
        let t153: [uint64x2_t; 4] = [vceqq_u64(t151[0], t152[0]), vceqq_u64(t151[1], t152[1]), vceqq_u64(t151[2], t152[2]), vceqq_u64(t151[3], t152[3])];
        let t154: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t153[0]), vmovn_u64(t153[1])), vcombine_u32(vmovn_u64(t153[2]), vmovn_u64(t153[3]))];
        let t155: [uint32x4_t; 2] = [vandq_u32(live[0], t154[0]), vandq_u32(live[1], t154[1])];
        if any(&t155) {
            let t156: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x24[0])), vmovl_u32(vget_high_u32(x24[0])), vmovl_u32(vget_low_u32(x24[1])), vmovl_u32(vget_high_u32(x24[1]))];
            res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t155[0])))), t156[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t155[0])))), t156[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t155[1])))), t156[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t155[1])))), t156[3], res[3])];
            live = [vbicq_u32(live[0], t155[0]), vbicq_u32(live[1], t155[1])];
        }
        if any(&live) {
            let t157: [uint64x2_t; 4] = [vshlq_u64(x32[0], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[1], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[2], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[3], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64)))];
            let t158: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t157[0]), vmovn_u64(t157[1])), vcombine_u32(vmovn_u64(t157[2]), vmovn_u64(t157[3]))];
            x40 = [vbslq_u32(live[0], t158[0], x40[0]), vbslq_u32(live[1], t158[1], x40[1])];
            let t159: [uint32x4_t; 2] = [vsubq_u32(x40[0], t147[0]), vsubq_u32(x40[1], t147[1])];
            let t160: [uint32x4_t; 2] = [vcgeq_u32(t159[0], t142[0]), vcgeq_u32(t159[1], t142[1])];
            let t161: [uint32x4_t; 2] = [vandq_u32(live[0], t160[0]), vandq_u32(live[1], t160[1])];
            if any(&t161) {
                let t162 = [vdupq_n_u64(4294967295u64); 4];
                res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t161[0])))), t162[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t161[0])))), t162[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t161[1])))), t162[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t161[1])))), t162[3], res[3])];
                live = [vbicq_u32(live[0], t161[0]), vbicq_u32(live[1], t161[1])];
            }
            if any(&live) {
                let t163: [uint32x4_t; 2] = [vsubq_u32(t142[0], x40[0]), vsubq_u32(t142[1], x40[1])];
                x48 = [vbslq_u32(live[0], t163[0], x48[0]), vbslq_u32(live[1], t163[1], x48[1])];
                let t164 = [vdupq_n_u64(2u64); 4];
                let t165: [uint64x2_t; 4] = [vandq_u64(x32[0], t164[0]), vandq_u64(x32[1], t164[1]), vandq_u64(x32[2], t164[2]), vandq_u64(x32[3], t164[3])];
                let t166: [uint64x2_t; 4] = [vceqq_u64(t165[0], t152[0]), vceqq_u64(t165[1], t152[1]), vceqq_u64(t165[2], t152[2]), vceqq_u64(t165[3], t152[3])];
                let t167: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t166[0]), vmovn_u64(t166[1])), vcombine_u32(vmovn_u64(t166[2]), vmovn_u64(t166[3]))];
                let t168: [uint32x4_t; 2] = [vmvnq_u32(t167[0]), vmvnq_u32(t167[1])];
                let t169: [uint32x4_t; 2] = [vandq_u32(live[0], t168[0]), vandq_u32(live[1], t168[1])];
                if any(&t169) {
                    let t170: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                    res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t169[0])))), t170[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t169[0])))), t170[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t169[1])))), t170[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t169[1])))), t170[3], res[3])];
                    live = [vbicq_u32(live[0], t169[0]), vbicq_u32(live[1], t169[1])];
                }
                if any(&live) {
                    let t171: [uint32x4_t; 2] = [vceqq_u32(x48[0], t143[0]), vceqq_u32(x48[1], t143[1])];
                    let t172: [uint32x4_t; 2] = [vandq_u32(live[0], t171[0]), vandq_u32(live[1], t171[1])];
                    if any(&t172) {
                        let t173 = [vdupq_n_u64(4294967295u64); 4];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t172[0])))), t173[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t172[0])))), t173[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t172[1])))), t173[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t172[1])))), t173[3], res[3])];
                        live = [vbicq_u32(live[0], t172[0]), vbicq_u32(live[1], t172[1])];
                    }
                    if any(&live) {
                        let t174 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
                        let t175: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                        let t176: [uint64x2_t; 4] = [vorrq_u64(t174[0], t175[0]), vorrq_u64(t174[1], t175[1]), vorrq_u64(t174[2], t175[2]), vorrq_u64(t174[3], t175[3])];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t176[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t176[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t176[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t176[3], res[3])];
                        live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
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
    let mut x24: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x32: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t177 = [vdupq_n_u32(0u32); 2];
    let t178: [uint32x4_t; 2] = [vceqq_u32(x16[0], t177[0]), vceqq_u32(x16[1], t177[1])];
    let t179: [uint32x4_t; 2] = [vandq_u32(live[0], t178[0]), vandq_u32(live[1], t178[1])];
    if any(&t179) {
        let t180 = [vdupq_n_u64(4294967295u64); 4];
        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t179[0])))), t180[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t179[0])))), t180[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t179[1])))), t180[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t179[1])))), t180[3], res[3])];
        live = [vbicq_u32(live[0], t179[0]), vbicq_u32(live[1], t179[1])];
    }
    if any(&live) {
        let t181 = [vdupq_n_u32(1u32); 2];
        let t182: [uint32x4_t; 2] = [vsubq_u32(x16[0], t181[0]), vsubq_u32(x16[1], t181[1])];
        x24 = [vbslq_u32(live[0], t182[0], x24[0]), vbslq_u32(live[1], t182[1], x24[1])];
        let t183 = b3_r0r1l_p(l, lid0, live, arg0, arg1, x24);
        x32 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t183[0], x32[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t183[1], x32[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t183[2], x32[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t183[3], x32[3])];
        let t184 = [vdupq_n_u64(1u64); 4];
        let t185: [uint64x2_t; 4] = [vandq_u64(x32[0], t184[0]), vandq_u64(x32[1], t184[1]), vandq_u64(x32[2], t184[2]), vandq_u64(x32[3], t184[3])];
        let t186 = [vdupq_n_u64(0u64); 4];
        let t187: [uint64x2_t; 4] = [vceqq_u64(t185[0], t186[0]), vceqq_u64(t185[1], t186[1]), vceqq_u64(t185[2], t186[2]), vceqq_u64(t185[3], t186[3])];
        let t188: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t187[0]), vmovn_u64(t187[1])), vcombine_u32(vmovn_u64(t187[2]), vmovn_u64(t187[3]))];
        let t189: [uint32x4_t; 2] = [vandq_u32(live[0], t188[0]), vandq_u32(live[1], t188[1])];
        if any(&t189) {
            let t190: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x24[0])), vmovl_u32(vget_high_u32(x24[0])), vmovl_u32(vget_low_u32(x24[1])), vmovl_u32(vget_high_u32(x24[1]))];
            res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t189[0])))), t190[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t189[0])))), t190[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t189[1])))), t190[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t189[1])))), t190[3], res[3])];
            live = [vbicq_u32(live[0], t189[0]), vbicq_u32(live[1], t189[1])];
        }
        if any(&live) {
            let t191: [uint64x2_t; 4] = [vshlq_u64(x32[0], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[1], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[2], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[3], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64)))];
            let t192: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t191[0]), vmovn_u64(t191[1])), vcombine_u32(vmovn_u64(t191[2]), vmovn_u64(t191[3]))];
            x40 = [vbslq_u32(live[0], t192[0], x40[0]), vbslq_u32(live[1], t192[1], x40[1])];
            let t193: [uint32x4_t; 2] = [vsubq_u32(x40[0], t181[0]), vsubq_u32(x40[1], t181[1])];
            let t194: [uint32x4_t; 2] = [vcgeq_u32(t193[0], x16[0]), vcgeq_u32(t193[1], x16[1])];
            let t195: [uint32x4_t; 2] = [vandq_u32(live[0], t194[0]), vandq_u32(live[1], t194[1])];
            if any(&t195) {
                let t196 = [vdupq_n_u64(4294967295u64); 4];
                res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t195[0])))), t196[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t195[0])))), t196[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t195[1])))), t196[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t195[1])))), t196[3], res[3])];
                live = [vbicq_u32(live[0], t195[0]), vbicq_u32(live[1], t195[1])];
            }
            if any(&live) {
                let t197: [uint32x4_t; 2] = [vsubq_u32(x16[0], x40[0]), vsubq_u32(x16[1], x40[1])];
                x48 = [vbslq_u32(live[0], t197[0], x48[0]), vbslq_u32(live[1], t197[1], x48[1])];
                let t198 = [vdupq_n_u64(2u64); 4];
                let t199: [uint64x2_t; 4] = [vandq_u64(x32[0], t198[0]), vandq_u64(x32[1], t198[1]), vandq_u64(x32[2], t198[2]), vandq_u64(x32[3], t198[3])];
                let t200: [uint64x2_t; 4] = [vceqq_u64(t199[0], t186[0]), vceqq_u64(t199[1], t186[1]), vceqq_u64(t199[2], t186[2]), vceqq_u64(t199[3], t186[3])];
                let t201: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t200[0]), vmovn_u64(t200[1])), vcombine_u32(vmovn_u64(t200[2]), vmovn_u64(t200[3]))];
                let t202: [uint32x4_t; 2] = [vmvnq_u32(t201[0]), vmvnq_u32(t201[1])];
                let t203: [uint32x4_t; 2] = [vandq_u32(live[0], t202[0]), vandq_u32(live[1], t202[1])];
                if any(&t203) {
                    let t204: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                    res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t203[0])))), t204[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t203[0])))), t204[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t203[1])))), t204[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t203[1])))), t204[3], res[3])];
                    live = [vbicq_u32(live[0], t203[0]), vbicq_u32(live[1], t203[1])];
                }
                if any(&live) {
                    let t205: [uint32x4_t; 2] = [vceqq_u32(x48[0], t177[0]), vceqq_u32(x48[1], t177[1])];
                    let t206: [uint32x4_t; 2] = [vandq_u32(live[0], t205[0]), vandq_u32(live[1], t205[1])];
                    if any(&t206) {
                        let t207 = [vdupq_n_u64(4294967295u64); 4];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t206[0])))), t207[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t206[0])))), t207[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t206[1])))), t207[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t206[1])))), t207[3], res[3])];
                        live = [vbicq_u32(live[0], t206[0]), vbicq_u32(live[1], t206[1])];
                    }
                    if any(&live) {
                        let t208 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
                        let t209: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                        let t210: [uint64x2_t; 4] = [vorrq_u64(t208[0], t209[0]), vorrq_u64(t208[1], t209[1]), vorrq_u64(t208[2], t209[2]), vorrq_u64(t208[3], t209[3])];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t210[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t210[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t210[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t210[3], res[3])];
                        live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
                    }
                }
            }
        }
    }
    res
}

#[inline(always)]
unsafe fn b3_r0r1l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t211 = b4_r0r1sl_p(l, lid0, live, arg0, arg1, 0u32, x16);
    res = t211;
    return res;
}

#[inline(always)]
unsafe fn b4_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: u64 = 0;
    let mut x56: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t212 = b5_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    x32 = t212;
    let t213: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t214 = b6_r0r1sl_p(l, lid0, live, arg0, arg1, (1u32).wrapping_add(x16), t213);
    x40 = t214;
    let t215 = b7_s_p(l, lid0, live, x32);
    x48 = t215;
    let t216 = [vdupq_n_u32((1024u32).wrapping_sub(1u32)); 2];
    let t217: [uint32x4_t; 2] = [vandq_u32(x24[0], t216[0]), vandq_u32(x24[1], t216[1])];
    let t218 = [vdupq_n_u32(x32); 2];
    let t219: [uint32x4_t; 2] = [vmulq_u32(t217[0], t218[0]), vmulq_u32(t217[1], t218[1])];
    let t220 = b8_r0r1lls_p(l, lid0, live, arg0, arg1, x40, t219, 8u32);
    let t221 = [vdupq_n_u64(x48); 4];
    let t222: [uint64x2_t; 4] = [vandq_u64(t220[0], t221[0]), vandq_u64(t220[1], t221[1]), vandq_u64(t220[2], t221[2]), vandq_u64(t220[3], t221[3])];
    x56 = t222;
    let t223 = [vdupq_n_u64(x48); 4];
    let t224: [uint64x2_t; 4] = [vceqq_u64(x56[0], t223[0]), vceqq_u64(x56[1], t223[1]), vceqq_u64(x56[2], t223[2]), vceqq_u64(x56[3], t223[3])];
    let t225: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t224[0]), vmovn_u64(t224[1])), vcombine_u32(vmovn_u64(t224[2]), vmovn_u64(t224[3]))];
    let mut t226: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t227: [uint32x4_t; 2] = [vandq_u32(live[0], t225[0]), vandq_u32(live[1], t225[1])];
    let t228: [uint32x4_t; 2] = [vbicq_u32(live[0], t225[0]), vbicq_u32(live[1], t225[1])];
    if any(&t227) {
        let t229 = b9_r0r1sl_p(l, lid0, t227, arg0, arg1, (x16).wrapping_add(1u32), x24);
        t226 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t227[0])))), t229[0], t226[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t227[0])))), t229[1], t226[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t227[1])))), t229[2], t226[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t227[1])))), t229[3], t226[3])];
    }
    {
        t226 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t228[0])))), x56[0], t226[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t228[0])))), x56[1], t226[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t228[1])))), x56[2], t226[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t228[1])))), x56[3], t226[3])];
    }
    res = t226;
    return res;
}

#[inline(always)]
unsafe fn b5_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t230 = b10_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    res = (((t230).wrapping_shr(((4u32).wrapping_mul(x16)) as u32)) & (15u32));
    return res;
}

#[inline(always)]
unsafe fn b6_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t231 = b10_r0r1s_p(l, lid0, live, arg0, arg1, 40u32);
    let t232 = [vdupq_n_u32(t231); 2];
    let t233: [uint32x4_t; 2] = [vmulq_u32(x24[0], t232[0]), vmulq_u32(x24[1], t232[1])];
    let t234 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 0u32);
    let t235 = [vdupq_n_u32(t234); 2];
    let t236: [uint32x4_t; 2] = [vaddq_u32(t233[0], t235[0]), vaddq_u32(t233[1], t235[1])];
    x32 = t236;
    if ((x16) == (0u32)) {
        res = x32;
        return res;
    }
    let t237 = b10_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    let t238: [uint32x4_t; 2] = [vshlq_u32(x24[0], vdupq_n_s32(((10u32) & 31) as i32)), vshlq_u32(x24[1], vdupq_n_s32(((10u32) & 31) as i32))];
    let t239 = [vdupq_n_u32(t237); 2];
    let t240: [uint32x4_t; 2] = [vsubq_u32(t239[0], t238[0]), vsubq_u32(t239[1], t238[1])];
    let t241 = b12_ls_p(l, lid0, live, t240, 1024u32);
    x40 = t241;
    let t242 = b11_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    let t243 = [vdupq_n_u32(t242); 2];
    let t244: [uint32x4_t; 2] = [vmulq_u32(x40[0], t243[0]), vmulq_u32(x40[1], t243[1])];
    let t245: [uint32x4_t; 2] = [vaddq_u32(x32[0], t244[0]), vaddq_u32(x32[1], t244[1])];
    res = t245;
    return res;
}

#[inline(always)]
unsafe fn b7_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t246: u64;
    if ((x0) >= (8u32)) {
        t246 = !(0u64);
    } else {
        t246 = ((1u64).wrapping_shl(((((x0).wrapping_mul(8u32)) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t246;
    return res;
}

#[inline(always)]
unsafe fn b7_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t247 = [vdupq_n_u32(8u32); 2];
    let t248: [uint32x4_t; 2] = [vcgeq_u32(x0[0], t247[0]), vcgeq_u32(x0[1], t247[1])];
    let mut t249: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t250: [uint32x4_t; 2] = [vandq_u32(live[0], t248[0]), vandq_u32(live[1], t248[1])];
    let t251: [uint32x4_t; 2] = [vbicq_u32(live[0], t248[0]), vbicq_u32(live[1], t248[1])];
    {
        let t252 = [vdupq_n_u64(!(0u64)); 4];
        t249 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t250[0])))), t252[0], t249[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t250[0])))), t252[1], t249[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t250[1])))), t252[2], t249[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t250[1])))), t252[3], t249[3])];
    }
    {
        let t253: [uint32x4_t; 2] = [vmulq_u32(x0[0], t247[0]), vmulq_u32(x0[1], t247[1])];
        let t254 = [vdupq_n_u64(1u64); 4];
        let t255: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t253[0])), vmovl_u32(vget_high_u32(t253[0])), vmovl_u32(vget_low_u32(t253[1])), vmovl_u32(vget_high_u32(t253[1]))];
        let t256: [uint64x2_t; 4] = [vshlq_u64(t254[0], vreinterpretq_s64_u64(vandq_u64(t255[0], vdupq_n_u64(63)))), vshlq_u64(t254[1], vreinterpretq_s64_u64(vandq_u64(t255[1], vdupq_n_u64(63)))), vshlq_u64(t254[2], vreinterpretq_s64_u64(vandq_u64(t255[2], vdupq_n_u64(63)))), vshlq_u64(t254[3], vreinterpretq_s64_u64(vandq_u64(t255[3], vdupq_n_u64(63))))];
        let t257: [uint64x2_t; 4] = [vsubq_u64(t256[0], t254[0]), vsubq_u64(t256[1], t254[1]), vsubq_u64(t256[2], t254[2]), vsubq_u64(t256[3], t254[3])];
        t249 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t251[0])))), t257[0], t249[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t251[0])))), t257[1], t249[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t251[1])))), t257[2], t249[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t251[1])))), t257[3], t249[3])];
    }
    res = t249;
    return res;
}

#[inline(always)]
unsafe fn b8_r0r1lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2], x24: [uint32x4_t; 2], x32: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t258 = b13_r0lls_p(l, lid0, live, arg0, x16, x24, x32);
    res = t258;
    return res;
}

#[inline(always)]
unsafe fn b9_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
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
    let t259 = b10_r0r1s_p(l, lid0, live, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x16)));
    x32 = t259;
    let t260 = b10_r0r1s_p(l, lid0, live, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x16)));
    x40 = t260;
    let t261: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    x48 = t261;
    let t262 = [vdupq_n_u32(4u32); 2];
    let t263: [uint32x4_t; 2] = [vmulq_u32(x48[0], t262[0]), vmulq_u32(x48[1], t262[1])];
    let t264 = b14_r0r1sls_p(l, lid0, live, arg0, arg1, x40, t263, 4u32);
    let t265: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t264[0]), vmovn_u64(t264[1])), vcombine_u32(vmovn_u64(t264[2]), vmovn_u64(t264[3]))];
    x56 = t265;
    let t266: [uint32x4_t; 2] = [vmulq_u32(x48[0], t262[0]), vmulq_u32(x48[1], t262[1])];
    let t267: [uint32x4_t; 2] = [vaddq_u32(t266[0], t262[0]), vaddq_u32(t266[1], t262[1])];
    let t268 = b14_r0r1sls_p(l, lid0, live, arg0, arg1, x40, t267, 4u32);
    let t269: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t268[0]), vmovn_u64(t268[1])), vcombine_u32(vmovn_u64(t268[2]), vmovn_u64(t268[3]))];
    x64 = t269;
    let t270 = b10_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    x72 = ((t270).wrapping_add((256u32).wrapping_sub(1u32))).wrapping_shr((8u32) as u32);
    let t271: [uint32x4_t; 2] = [vmulq_u32(x48[0], t262[0]), vmulq_u32(x48[1], t262[1])];
    let t272 = [vdupq_n_u32(x72); 2];
    let t273: [uint32x4_t; 2] = [vsubq_u32(t272[0], t271[0]), vsubq_u32(t272[1], t271[1])];
    let t274 = b15_sl_p(l, lid0, live, 4u32, t273);
    x80 = t274;
    let t275: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
    let t276 = [vdupq_n_u32((4u32).wrapping_sub(1u32)); 2];
    let t277: [uint32x4_t; 2] = [vandq_u32(t275[0], t276[0]), vandq_u32(t275[1], t276[1])];
    x88 = t277;
    let t278 = [vdupq_n_u32(0u32); 2];
    let t279: [uint32x4_t; 2] = [vceqq_u32(x88[0], t278[0]), vceqq_u32(x88[1], t278[1])];
    let mut t280: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t281: [uint32x4_t; 2] = [vandq_u32(live[0], t279[0]), vandq_u32(live[1], t279[1])];
    let t282: [uint32x4_t; 2] = [vbicq_u32(live[0], t279[0]), vbicq_u32(live[1], t279[1])];
    {
        t280 = [vbslq_u32(t281[0], t278[0], t280[0]), vbslq_u32(t281[1], t278[1], t280[1])];
    }
    if any(&t282) {
        let t283 = [vdupq_n_u32(1u32); 2];
        let t284: [uint32x4_t; 2] = [vsubq_u32(x88[0], t283[0]), vsubq_u32(x88[1], t283[1])];
        let t285 = [vdupq_n_u32(2u32); 2];
        let t286: [uint32x4_t; 2] = [vmulq_u32(t285[0], t284[0]), vmulq_u32(t285[1], t284[1])];
        let t287: [uint32x4_t; 2] = [vaddq_u32(x56[0], t286[0]), vaddq_u32(x56[1], t286[1])];
        let t288 = b16_ll_p(l, lid0, t282, t287, x64);
        let t289 = b14_r0r1sls_p(l, lid0, t282, arg0, arg1, x32, t288, 2u32);
        let t290: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t289[0]), vmovn_u64(t289[1])), vcombine_u32(vmovn_u64(t289[2]), vmovn_u64(t289[3]))];
        t280 = [vbslq_u32(t282[0], t290[0], t280[0]), vbslq_u32(t282[1], t290[1], t280[1])];
    }
    x96 = t280;
    let t291 = [vdupq_n_u32(2u32); 2];
    let t292: [uint32x4_t; 2] = [vmulq_u32(t291[0], x88[0]), vmulq_u32(t291[1], x88[1])];
    let t293: [uint32x4_t; 2] = [vaddq_u32(x56[0], t292[0]), vaddq_u32(x56[1], t292[1])];
    let t294 = b16_ll_p(l, lid0, live, t293, x64);
    let t295 = b14_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t294, 2u32);
    let t296: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t295[0]), vmovn_u64(t295[1])), vcombine_u32(vmovn_u64(t295[2]), vmovn_u64(t295[3]))];
    x104 = t296;
    let t297: [uint32x4_t; 2] = [vmulq_u32(t291[0], x80[0]), vmulq_u32(t291[1], x80[1])];
    let t298: [uint32x4_t; 2] = [vaddq_u32(x56[0], t297[0]), vaddq_u32(x56[1], t297[1])];
    x112 = t298;
    let t299 = [vdupq_n_u32((256u32).wrapping_sub(1u32)); 2];
    let t300: [uint32x4_t; 2] = [vandq_u32(x24[0], t299[0]), vandq_u32(x24[1], t299[1])];
    x120 = t300;
    x128 = 9259542123273814144u64;
    let t301: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x120[0])), vmovl_u32(vget_high_u32(x120[0])), vmovl_u32(vget_low_u32(x120[1])), vmovl_u32(vget_high_u32(x120[1]))];
    let t302 = [vdupq_n_u64(72340172838076673u64); 4];
    let mut t303: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    t303[0] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t301[0]) as u64).wrapping_mul(vgetq_lane_u64::<0>(t302[0]) as u64) as u64, t303[0]);
    t303[0] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t301[0]) as u64).wrapping_mul(vgetq_lane_u64::<1>(t302[0]) as u64) as u64, t303[0]);
    t303[1] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t301[1]) as u64).wrapping_mul(vgetq_lane_u64::<0>(t302[1]) as u64) as u64, t303[1]);
    t303[1] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t301[1]) as u64).wrapping_mul(vgetq_lane_u64::<1>(t302[1]) as u64) as u64, t303[1]);
    t303[2] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t301[2]) as u64).wrapping_mul(vgetq_lane_u64::<0>(t302[2]) as u64) as u64, t303[2]);
    t303[2] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t301[2]) as u64).wrapping_mul(vgetq_lane_u64::<1>(t302[2]) as u64) as u64, t303[2]);
    t303[3] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t301[3]) as u64).wrapping_mul(vgetq_lane_u64::<0>(t302[3]) as u64) as u64, t303[3]);
    t303[3] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t301[3]) as u64).wrapping_mul(vgetq_lane_u64::<1>(t302[3]) as u64) as u64, t303[3]);
    x136 = t303;
    x144 = x96;
    x152 = x96;
    let mut t304 = live;
    loop {
        // Lanes part: each its own rounds.
        let t306: [uint32x4_t; 2] = [vcltq_u32(x152[0], x104[0]), vcltq_u32(x152[1], x104[1])];
        t304 = [vandq_u32(t304[0], t306[0]), vandq_u32(t304[1], t306[1])];
        if !(any(&t304)) { break; }
        let t308: [uint32x4_t; 2] = [vaddq_u32(x112[0], x152[0]), vaddq_u32(x112[1], x152[1])];
        let t309 = b16_ll_p(l, lid0, t304, t308, x64);
        let t310 = b14_r0r1sls_p(l, lid0, t304, arg0, arg1, x32, t309, 8u32);
        x160 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t304[0])))), t310[0], x160[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t304[0])))), t310[1], x160[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t304[1])))), t310[2], x160[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t304[1])))), t310[3], x160[3])];
        let t311 = [vdupq_n_u64(x128); 4];
        let t312: [uint64x2_t; 4] = [vorrq_u64(x160[0], t311[0]), vorrq_u64(x160[1], t311[1]), vorrq_u64(x160[2], t311[2]), vorrq_u64(x160[3], t311[3])];
        let t313 = [vdupq_n_u64(!(x128)); 4];
        let t314: [uint64x2_t; 4] = [vandq_u64(x136[0], t313[0]), vandq_u64(x136[1], t313[1]), vandq_u64(x136[2], t313[2]), vandq_u64(x136[3], t313[3])];
        let t315: [uint64x2_t; 4] = [vsubq_u64(t312[0], t314[0]), vsubq_u64(t312[1], t314[1]), vsubq_u64(t312[2], t314[2]), vsubq_u64(t312[3], t314[3])];
        x168 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t304[0])))), t315[0], x168[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t304[0])))), t315[1], x168[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t304[1])))), t315[2], x168[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t304[1])))), t315[3], x168[3])];
        let t316: [uint64x2_t; 4] = [vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x160[0]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x160[1]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x160[2]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x160[3])))];
        let t317: [uint64x2_t; 4] = [vandq_u64(t316[0], x136[0]), vandq_u64(t316[1], x136[1]), vandq_u64(t316[2], x136[2]), vandq_u64(t316[3], x136[3])];
        let t318: [uint64x2_t; 4] = [veorq_u64(x160[0], x136[0]), veorq_u64(x160[1], x136[1]), veorq_u64(x160[2], x136[2]), veorq_u64(x160[3], x136[3])];
        let t319: [uint64x2_t; 4] = [vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(t318[0]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(t318[1]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(t318[2]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(t318[3])))];
        let t320: [uint64x2_t; 4] = [vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x168[0]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x168[1]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x168[2]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x168[3])))];
        let t321: [uint64x2_t; 4] = [vandq_u64(t319[0], t320[0]), vandq_u64(t319[1], t320[1]), vandq_u64(t319[2], t320[2]), vandq_u64(t319[3], t320[3])];
        let t322: [uint64x2_t; 4] = [vorrq_u64(t317[0], t321[0]), vorrq_u64(t317[1], t321[1]), vorrq_u64(t317[2], t321[2]), vorrq_u64(t317[3], t321[3])];
        let t323 = [vdupq_n_u64(x128); 4];
        let t324: [uint64x2_t; 4] = [vandq_u64(t323[0], t322[0]), vandq_u64(t323[1], t322[1]), vandq_u64(t323[2], t322[2]), vandq_u64(t323[3], t322[3])];
        x176 = t324;
        let t325: [uint32x4_t; 2] = [vsubq_u32(x104[0], x152[0]), vsubq_u32(x104[1], x152[1])];
        x184 = t325;
        let t326 = [vdupq_n_u32(8u32); 2];
        let t327: [uint32x4_t; 2] = [vcgeq_u32(x184[0], t326[0]), vcgeq_u32(x184[1], t326[1])];
        let mut t328: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
        let t329: [uint32x4_t; 2] = [vandq_u32(t304[0], t327[0]), vandq_u32(t304[1], t327[1])];
        let t330: [uint32x4_t; 2] = [vbicq_u32(t304[0], t327[0]), vbicq_u32(t304[1], t327[1])];
        {
            t328 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t329[0])))), x176[0], t328[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t329[0])))), x176[1], t328[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t329[1])))), x176[2], t328[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t329[1])))), x176[3], t328[3])];
        }
        {
            let t331: [uint32x4_t; 2] = [vmulq_u32(t326[0], x184[0]), vmulq_u32(t326[1], x184[1])];
            let t332 = [vdupq_n_u64(1u64); 4];
            let t333: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t331[0])), vmovl_u32(vget_high_u32(t331[0])), vmovl_u32(vget_low_u32(t331[1])), vmovl_u32(vget_high_u32(t331[1]))];
            let t334: [uint64x2_t; 4] = [vshlq_u64(t332[0], vreinterpretq_s64_u64(vandq_u64(t333[0], vdupq_n_u64(63)))), vshlq_u64(t332[1], vreinterpretq_s64_u64(vandq_u64(t333[1], vdupq_n_u64(63)))), vshlq_u64(t332[2], vreinterpretq_s64_u64(vandq_u64(t333[2], vdupq_n_u64(63)))), vshlq_u64(t332[3], vreinterpretq_s64_u64(vandq_u64(t333[3], vdupq_n_u64(63))))];
            let t335: [uint64x2_t; 4] = [vsubq_u64(t334[0], t332[0]), vsubq_u64(t334[1], t332[1]), vsubq_u64(t334[2], t332[2]), vsubq_u64(t334[3], t332[3])];
            let t336: [uint64x2_t; 4] = [vandq_u64(x176[0], t335[0]), vandq_u64(x176[1], t335[1]), vandq_u64(x176[2], t335[2]), vandq_u64(x176[3], t335[3])];
            t328 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t330[0])))), t336[0], t328[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t330[0])))), t336[1], t328[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t330[1])))), t336[2], t328[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t330[1])))), t336[3], t328[3])];
        }
        let t337 = b17_l_p(l, lid0, t304, t328);
        let t338: [uint32x4_t; 2] = [vaddq_u32(x144[0], t337[0]), vaddq_u32(x144[1], t337[1])];
        x144 = [vbslq_u32(t304[0], t338[0], x144[0]), vbslq_u32(t304[1], t338[1], x144[1])];
        let t339: [uint32x4_t; 2] = [vandq_u32(t304[0], live[0]), vandq_u32(t304[1], live[1])];
        x152 = [vaddq_u32(x152[0], vandq_u32(t339[0], vdupq_n_u32(8))), vaddq_u32(x152[1], vandq_u32(t339[1], vdupq_n_u32(8)))];
    }
    x96 = x144;
    let t340 = b10_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    let t341 = [vdupq_n_u32((((t340).wrapping_shr((((((4u32).wrapping_mul(x16)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32))); 2];
    x152 = t341;
    let t342 = [vdupq_n_u32(1u32); 2];
    let t343: [uint32x4_t; 2] = [vsubq_u32(x80[0], t342[0]), vsubq_u32(x80[1], t342[1])];
    let t344: [uint32x4_t; 2] = [vmulq_u32(t291[0], t343[0]), vmulq_u32(t291[1], t343[1])];
    let t345: [uint32x4_t; 2] = [vaddq_u32(x56[0], t344[0]), vaddq_u32(x56[1], t344[1])];
    let t346 = b16_ll_p(l, lid0, live, t345, x64);
    let t347 = b14_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t346, 2u32);
    let t348: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t347[0]), vmovn_u64(t347[1])), vcombine_u32(vmovn_u64(t347[2]), vmovn_u64(t347[3]))];
    let t349: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t348[0])), vmovl_u32(vget_high_u32(t348[0])), vmovl_u32(vget_low_u32(t348[1])), vmovl_u32(vget_high_u32(t348[1]))];
    x160 = t349;
    let t350: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x160[0]), vmovn_u64(x160[1])), vcombine_u32(vmovn_u64(x160[2]), vmovn_u64(x160[3]))];
    let t351: [uint32x4_t; 2] = [vaddq_u32(x112[0], t350[0]), vaddq_u32(x112[1], t350[1])];
    let t352: [uint32x4_t; 2] = [vmulq_u32(x96[0], x152[0]), vmulq_u32(x96[1], x152[1])];
    let t353: [uint32x4_t; 2] = [vaddq_u32(t351[0], t352[0]), vaddq_u32(t351[1], t352[1])];
    let t354 = b16_ll_p(l, lid0, live, t353, x64);
    let t355: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t354[0])), vmovl_u32(vget_high_u32(t354[0])), vmovl_u32(vget_low_u32(t354[1])), vmovl_u32(vget_high_u32(t354[1]))];
    x168 = t355;
    let t356: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x168[0]), vmovn_u64(x168[1])), vcombine_u32(vmovn_u64(x168[2]), vmovn_u64(x168[3]))];
    let t357 = b14_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t356, 8u32);
    let t358 = b7_l_p(l, lid0, live, x152);
    let t359: [uint64x2_t; 4] = [vandq_u64(t357[0], t358[0]), vandq_u64(t357[1], t358[1]), vandq_u64(t357[2], t358[2]), vandq_u64(t357[3], t358[3])];
    res = t359;
    return res;
}

#[inline(always)]
unsafe fn b10_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    res = (arg1.wrapping_add(((((x16).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned();
    return res;
}

#[inline(always)]
unsafe fn b11_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t360 = b10_r0r1s_p(l, lid0, live, arg0, arg1, (20u32).wrapping_add((4u32).wrapping_mul(x16)));
    res = t360;
    return res;
}

#[inline(always)]
unsafe fn b12_ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x8: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t361 = [vdupq_n_u32(x8); 2];
    let t362: [uint32x4_t; 2] = [vcltq_u32(x0[0], t361[0]), vcltq_u32(x0[1], t361[1])];
    let mut t363: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t364: [uint32x4_t; 2] = [vandq_u32(live[0], t362[0]), vandq_u32(live[1], t362[1])];
    let t365: [uint32x4_t; 2] = [vbicq_u32(live[0], t362[0]), vbicq_u32(live[1], t362[1])];
    {
        t363 = [vbslq_u32(t364[0], x0[0], t363[0]), vbslq_u32(t364[1], x0[1], t363[1])];
    }
    {
        let t366 = [vdupq_n_u32(x8); 2];
        t363 = [vbslq_u32(t365[0], t366[0], t363[0]), vbslq_u32(t365[1], t366[1], t363[1])];
    }
    res = t363;
    return res;
}

#[inline(always)]
unsafe fn b13_r0lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: [uint32x4_t; 2], x16: [uint32x4_t; 2], x24: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t367: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x8[0])), vmovl_u32(vget_high_u32(x8[0])), vmovl_u32(vget_low_u32(x8[1])), vmovl_u32(vget_high_u32(x8[1]))];
    let t368: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x16[0])), vmovl_u32(vget_high_u32(x16[0])), vmovl_u32(vget_low_u32(x16[1])), vmovl_u32(vget_high_u32(x16[1]))];
    let t369: [uint64x2_t; 4] = [vaddq_u64(t367[0], t368[0]), vaddq_u64(t367[1], t368[1]), vaddq_u64(t367[2], t368[2]), vaddq_u64(t367[3], t368[3])];
    let t370 = b18_r0ls_p(l, lid0, live, arg0, t369, x24);
    x32 = t370;
    let t371: [uint64x2_t; 4];
    if ((x24) >= (8u32)) {
        t371 = x32;
    } else {
        let t372 = b19_s_p(l, lid0, live, (8u32).wrapping_mul(x24));
        let t373 = [vdupq_n_u64(t372); 4];
        let t374: [uint64x2_t; 4] = [vandq_u64(x32[0], t373[0]), vandq_u64(x32[1], t373[1]), vandq_u64(x32[2], t373[2]), vandq_u64(x32[3], t373[3])];
        t371 = t374;
    }
    res = t371;
    return res;
}

#[inline(always)]
unsafe fn b14_r0r1sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2], x32: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t375 = b20_r0sls_p(l, lid0, live, arg0, x16, x24, x32);
    res = t375;
    return res;
}

#[inline(always)]
unsafe fn b15_sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x8: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t376 = [vdupq_n_u32(x0); 2];
    let t377: [uint32x4_t; 2] = [vcltq_u32(t376[0], x8[0]), vcltq_u32(t376[1], x8[1])];
    let mut t378: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t379: [uint32x4_t; 2] = [vandq_u32(live[0], t377[0]), vandq_u32(live[1], t377[1])];
    let t380: [uint32x4_t; 2] = [vbicq_u32(live[0], t377[0]), vbicq_u32(live[1], t377[1])];
    {
        let t381 = [vdupq_n_u32(x0); 2];
        t378 = [vbslq_u32(t379[0], t381[0], t378[0]), vbslq_u32(t379[1], t381[1], t378[1])];
    }
    {
        t378 = [vbslq_u32(t380[0], x8[0], t378[0]), vbslq_u32(t380[1], x8[1], t378[1])];
    }
    res = t378;
    return res;
}

#[inline(always)]
unsafe fn b16_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x8: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t382: [uint32x4_t; 2] = [vcltq_u32(x0[0], x8[0]), vcltq_u32(x0[1], x8[1])];
    let mut t383: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t384: [uint32x4_t; 2] = [vandq_u32(live[0], t382[0]), vandq_u32(live[1], t382[1])];
    let t385: [uint32x4_t; 2] = [vbicq_u32(live[0], t382[0]), vbicq_u32(live[1], t382[1])];
    {
        t383 = [vbslq_u32(t384[0], x0[0], t383[0]), vbslq_u32(t384[1], x0[1], t383[1])];
    }
    {
        t383 = [vbslq_u32(t385[0], x8[0], t383[0]), vbslq_u32(t385[1], x8[1], t383[1])];
    }
    res = t383;
    return res;
}

#[inline(always)]
unsafe fn b17_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint64x2_t; 4]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t386: [uint64x2_t; 4] = [vpaddlq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u64(x0[0]))))), vpaddlq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u64(x0[1]))))), vpaddlq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u64(x0[2]))))), vpaddlq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u64(x0[3])))))];
    let t387: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t386[0]), vmovn_u64(t386[1])), vcombine_u32(vmovn_u64(t386[2]), vmovn_u64(t386[3]))];
    res = t387;
    return res;
}

#[inline(always)]
unsafe fn b18_r0ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: [uint64x2_t; 4], x16: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t388 = [vdupq_n_u64(!(7u64)); 4];
    let t389: [uint64x2_t; 4] = [vandq_u64(x8[0], t388[0]), vandq_u64(x8[1], t388[1]), vandq_u64(x8[2], t388[2]), vandq_u64(x8[3], t388[3])];
    let t390: [uint64x2_t; 4] = [vandq_u64(t389[0], vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0]))))), vandq_u64(t389[1], vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0]))))), vandq_u64(t389[2], vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1]))))), vandq_u64(t389[3], vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))))];
    let t391: [uint64x2_t; 4] = [vaddq_u64(vdupq_n_u64((arg0) as u64), t390[0]), vaddq_u64(vdupq_n_u64((arg0) as u64), t390[1]), vaddq_u64(vdupq_n_u64((arg0) as u64), t390[2]), vaddq_u64(vdupq_n_u64((arg0) as u64), t390[3])];
    x24 = t391;
    let t392 = [vdupq_n_u64(7u64); 4];
    let t393: [uint64x2_t; 4] = [vandq_u64(x8[0], t392[0]), vandq_u64(x8[1], t392[1]), vandq_u64(x8[2], t392[2]), vandq_u64(x8[3], t392[3])];
    let t394: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t393[0]), vmovn_u64(t393[1])), vcombine_u32(vmovn_u64(t393[2]), vmovn_u64(t393[3]))];
    let t395 = [vdupq_n_u32(8u32); 2];
    let t396: [uint32x4_t; 2] = [vmulq_u32(t394[0], t395[0]), vmulq_u32(t394[1], t395[1])];
    x32 = t396;
    let mut t397: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    t397[0] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[0]) as usize as *const u64).read_unaligned(), t397[0]);
    t397[0] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[0]) as usize as *const u64).read_unaligned(), t397[0]);
    t397[1] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[1]) as usize as *const u64).read_unaligned(), t397[1]);
    t397[1] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[1]) as usize as *const u64).read_unaligned(), t397[1]);
    t397[2] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[2]) as usize as *const u64).read_unaligned(), t397[2]);
    t397[2] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[2]) as usize as *const u64).read_unaligned(), t397[2]);
    t397[3] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[3]) as usize as *const u64).read_unaligned(), t397[3]);
    t397[3] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[3]) as usize as *const u64).read_unaligned(), t397[3]);
    let t398: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x32[0])), vmovl_u32(vget_high_u32(x32[0])), vmovl_u32(vget_low_u32(x32[1])), vmovl_u32(vget_high_u32(x32[1]))];
    let t399: [uint64x2_t; 4] = [vshlq_u64(t397[0], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t398[0], vdupq_n_u64(63))))), vshlq_u64(t397[1], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t398[1], vdupq_n_u64(63))))), vshlq_u64(t397[2], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t398[2], vdupq_n_u64(63))))), vshlq_u64(t397[3], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t398[3], vdupq_n_u64(63)))))];
    x40 = t399;
    let t400 = [vdupq_n_u32(0u32); 2];
    let t401: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t400[0])), vmvnq_u32(vceqq_u32(x32[1], t400[1]))];
    let t402 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t403: [uint32x4_t; 2] = [vaddq_u32(x32[0], t402[0]), vaddq_u32(x32[1], t402[1])];
    let t404 = [vdupq_n_u32(64u32); 2];
    let t405: [uint32x4_t; 2] = [vcgtq_u32(t403[0], t404[0]), vcgtq_u32(t403[1], t404[1])];
    let t406: [uint32x4_t; 2] = [vandq_u32(t401[0], t405[0]), vandq_u32(t401[1], t405[1])];
    let mut t407: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t408: [uint32x4_t; 2] = [vandq_u32(live[0], t406[0]), vandq_u32(live[1], t406[1])];
    let t409: [uint32x4_t; 2] = [vbicq_u32(live[0], t406[0]), vbicq_u32(live[1], t406[1])];
    if any(&t408) {
        let t410 = [vdupq_n_u64((1u64).wrapping_mul(8u64)); 4];
        let t411: [uint64x2_t; 4] = [vaddq_u64(x24[0], t410[0]), vaddq_u64(x24[1], t410[1]), vaddq_u64(x24[2], t410[2]), vaddq_u64(x24[3], t410[3])];
        let mut t412: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
        t412[0] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t411[0]) as usize as *const u64).read_unaligned(), t412[0]);
        t412[0] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t411[0]) as usize as *const u64).read_unaligned(), t412[0]);
        t412[1] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t411[1]) as usize as *const u64).read_unaligned(), t412[1]);
        t412[1] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t411[1]) as usize as *const u64).read_unaligned(), t412[1]);
        t412[2] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t411[2]) as usize as *const u64).read_unaligned(), t412[2]);
        t412[2] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t411[2]) as usize as *const u64).read_unaligned(), t412[2]);
        t412[3] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t411[3]) as usize as *const u64).read_unaligned(), t412[3]);
        t412[3] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t411[3]) as usize as *const u64).read_unaligned(), t412[3]);
        let t413: [uint32x4_t; 2] = [vsubq_u32(t404[0], x32[0]), vsubq_u32(t404[1], x32[1])];
        let t414: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t413[0])), vmovl_u32(vget_high_u32(t413[0])), vmovl_u32(vget_low_u32(t413[1])), vmovl_u32(vget_high_u32(t413[1]))];
        let t415: [uint64x2_t; 4] = [vshlq_u64(t412[0], vreinterpretq_s64_u64(vandq_u64(t414[0], vdupq_n_u64(63)))), vshlq_u64(t412[1], vreinterpretq_s64_u64(vandq_u64(t414[1], vdupq_n_u64(63)))), vshlq_u64(t412[2], vreinterpretq_s64_u64(vandq_u64(t414[2], vdupq_n_u64(63)))), vshlq_u64(t412[3], vreinterpretq_s64_u64(vandq_u64(t414[3], vdupq_n_u64(63))))];
        let t416: [uint64x2_t; 4] = [vorrq_u64(x40[0], t415[0]), vorrq_u64(x40[1], t415[1]), vorrq_u64(x40[2], t415[2]), vorrq_u64(x40[3], t415[3])];
        t407 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t408[0])))), t416[0], t407[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t408[0])))), t416[1], t407[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t408[1])))), t416[2], t407[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t408[1])))), t416[3], t407[3])];
    }
    {
        t407 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t409[0])))), x40[0], t407[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t409[0])))), x40[1], t407[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t409[1])))), x40[2], t407[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t409[1])))), x40[3], t407[3])];
    }
    res = t407;
    return res;
}

#[inline(always)]
unsafe fn b19_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t417: u64;
    if ((x0) >= (64u32)) {
        t417 = !(0u64);
    } else {
        t417 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t417;
    return res;
}

#[inline(always)]
unsafe fn b20_r0sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: u32, x16: [uint32x4_t; 2], x24: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t418 = [vdupq_n_u64(((x8) as u64)); 4];
    let t419: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x16[0])), vmovl_u32(vget_high_u32(x16[0])), vmovl_u32(vget_low_u32(x16[1])), vmovl_u32(vget_high_u32(x16[1]))];
    let t420: [uint64x2_t; 4] = [vaddq_u64(t418[0], t419[0]), vaddq_u64(t418[1], t419[1]), vaddq_u64(t418[2], t419[2]), vaddq_u64(t418[3], t419[3])];
    let t421 = b18_r0ls_p(l, lid0, live, arg0, t420, x24);
    x32 = t421;
    let t422: [uint64x2_t; 4];
    if ((x24) >= (8u32)) {
        t422 = x32;
    } else {
        let t423 = b19_s_p(l, lid0, live, (8u32).wrapping_mul(x24));
        let t424 = [vdupq_n_u64(t423); 4];
        let t425: [uint64x2_t; 4] = [vandq_u64(x32[0], t424[0]), vandq_u64(x32[1], t424[1]), vandq_u64(x32[2], t424[2]), vandq_u64(x32[3], t424[3])];
        t422 = t425;
    }
    res = t422;
    return res;
}

/// The group `l.group`'s threads, 8 at a time: `argv[j]` the address of argument `j`'s bytes (a buffer's address,
/// a value's bytes).
///
/// # Safety
///
/// `argv` holds the kernel's arguments, its buffers as large as it reads and writes; `l` is a 1-D group's launch.
pub unsafe fn parent_link_w8(argv: *const *const u8, l: &Launch) {
    let n = l.block[0] * l.block[1] * l.block[2];
    let arg0: *const u8 = (*argv.add(0) as *const *const u8).read_unaligned();
    let arg1: *const u8 = (*argv.add(1) as *const *const u8).read_unaligned();
    let arg2: *const u8 = (*argv.add(2) as *const *const u8).read_unaligned();
    let arg3: *const u8 = (*argv.add(3) as *const *const u8).read_unaligned();
    let x32: *const u8 = *argv.add(4);
    let arg5: *const u8 = (*argv.add(5) as *const *const u8).read_unaligned();
    let x56: u32 = (*argv.add(6) as *const u32).read_unaligned();
    let mut t426: bool = !(arg5).is_null();
    if t426 {
        let mut t427: bool = ((l.group[0]) >= ((arg5.wrapping_add((((x56) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        if !t427 {
            t427 = ((l.group[1]) >= ((arg5.wrapping_add(((((x56).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        let mut t428: bool = t427;
        if !t428 {
            t428 = ((l.group[2]) >= ((arg5.wrapping_add(((((x56).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        t426 = t428;
    }
    if t426 {
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
        let t429: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
        let mut live = t429;
        let gid0 = l.group[0] * l.block[0] + lid0;
        'chunk: {
            b0_r0r1r2r3sg_p(l, lid0, live, arg0, arg1, arg2, arg3, x32);
        }
    }
}
