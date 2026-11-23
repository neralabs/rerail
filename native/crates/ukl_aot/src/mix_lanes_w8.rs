//! `mix_lanes` at 8 lanes (2 NEON registers), written by rerail-ukl-vec's Rust (`rs.rs`) from its vector tape: do
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

/// `mix_lanes_inner`.
#[inline(always)]
unsafe fn b0_r0r1sg_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: *const u8) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x2 as *const u32).read_unaligned();
    let a1: u32 = (x2.wrapping_add(4u64 as usize) as *const u32).read_unaligned();
    let mut x4: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x5: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x6: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x7: u32 = 0;
    let mut x8: u32 = 0;
    let mut live = k;
    let t1: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
    let t2 = [vdupq_n_u32(a0); 2];
    let t3: [uint32x4_t; 2] = [vcgeq_u32(t1[0], t2[0]), vcgeq_u32(t1[1], t2[1])];
    let t4: [uint32x4_t; 2] = [vandq_u32(live[0], t3[0]), vandq_u32(live[1], t3[1])];
    {
        live = [vbicq_u32(live[0], t4[0]), vbicq_u32(live[1], t4[1])];
    }
    if any(&live) {
        let mut t5: [uint32x4_t; 2];
        if all(&live) {
            let t6 = (arg0 as *const u32).add((gid0) as usize);
            t5 = [vld1q_u32(t6.add(0)), vld1q_u32(t6.add(4))];
        } else {
            let t7 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t1[0]), vandq_u32(live[1], t1[1]))));
            let t8: [uint32x4_t; 2] = [vbslq_u32(live[0], t1[0], t7), vbslq_u32(live[1], t1[1], t7)];
            let t9 = arg0 as *const u32;
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
        x4 = t5;
        let t11 = [vdupq_n_u32(1u32); 2];
        let t12: [uint32x4_t; 2] = [vandq_u32(x4[0], t11[0]), vandq_u32(x4[1], t11[1])];
        let t13: [uint32x4_t; 2] = [vtstq_u32(t12[0], t12[0]), vtstq_u32(t12[1], t12[1])];
        let t14: [uint32x4_t; 2] = [vandq_u32(live[0], t13[0]), vandq_u32(live[1], t13[1])];
        {
            let t15 = [vdupq_n_u32(3u32); 2];
            let t16: [uint32x4_t; 2] = [vmulq_u32(x4[0], t15[0]), vmulq_u32(x4[1], t15[1])];
            let t17: [uint32x4_t; 2] = [vaddq_u32(t16[0], t11[0]), vaddq_u32(t16[1], t11[1])];
            x5 = [vbslq_u32(t14[0], t17[0], x5[0]), vbslq_u32(t14[1], t17[1], x5[1])];
        }
        let t18: [uint32x4_t; 2] = [vbicq_u32(live[0], t13[0]), vbicq_u32(live[1], t13[1])];
        {
            let t19: [uint32x4_t; 2] = [vshlq_u32(x4[0], vnegq_s32(vdupq_n_s32(((1u32) & 31) as i32))), vshlq_u32(x4[1], vnegq_s32(vdupq_n_s32(((1u32) & 31) as i32)))];
            x5 = [vbslq_u32(t18[0], t19[0], x5[0]), vbslq_u32(t18[1], t19[1], x5[1])];
        }
        let t20 = [vdupq_n_u32(4026531840u32); 2];
        let t21: [uint32x4_t; 2] = [vcltq_u32(x4[0], t20[0]), vcltq_u32(x4[1], t20[1])];
        let t22: [uint32x4_t; 2] = [vandq_u32(live[0], t21[0]), vandq_u32(live[1], t21[1])];
        {
            let t23: [uint32x4_t; 2] = [vaddq_u32(x5[0], x4[0]), vaddq_u32(x5[1], x4[1])];
            x5 = [vbslq_u32(t22[0], t23[0], x5[0]), vbslq_u32(t22[1], t23[1], x5[1])];
        }
        let t24 = [vdupq_n_u32(0u32); 2];
        let t25: [uint32x4_t; 2] = [vceqq_u32(x4[0], t24[0]), vceqq_u32(x4[1], t24[1])];
        let t26: [uint32x4_t; 2] = [vandq_u32(live[0], t25[0]), vandq_u32(live[1], t25[1])];
        {
            let t27 = [vdupq_n_u32(57005u32); 2];
            x5 = [vbslq_u32(t26[0], t27[0], x5[0]), vbslq_u32(t26[1], t27[1], x5[1])];
        }
        let t28: [uint32x4_t; 2] = [vbicq_u32(live[0], t25[0]), vbicq_u32(live[1], t25[1])];
        {
            let t29 = [vdupq_n_u32(7u32); 2];
            let t30: [uint32x4_t; 2] = [veorq_u32(x5[0], t29[0]), veorq_u32(x5[1], t29[1])];
            x5 = [vbslq_u32(t28[0], t30[0], x5[0]), vbslq_u32(t28[1], t30[1], x5[1])];
        }
        x6 = t24;
        x7 = 0u32;
        x8 = 0u32;
        let mut t35 = live;
        loop {
            let t37 = bits(&t35);
            if t37.count_ones() <= 2 {
                let mut t38 = t37;
                while t38 != 0 {
                    let j = t38.trailing_zeros() as usize;
                    t38 &= t38 - 1;
                    let t39 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x4);
                    let mut x4_j: u32 = t39[j];
                    let t40 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x5);
                    let mut x5_j: u32 = t40[j];
                    let t41 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x6);
                    let mut x6_j: u32 = t41[j];
                    let mut x8_j: u32 = x8;
                    loop {
                        if !(((x8_j) < (((x4_j) & (15u32))))) { break; }
                        x6_j = (x6_j).wrapping_add((x8_j).wrapping_mul(x5_j));
                        let t42 = x8_j;
                        x8_j = (x8_j).wrapping_add(1u32);
                    }
                    let mut t43 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x6);
                    t43[j] = x6_j as u32;
                    x6 = core::mem::transmute::<[u32; 8], [uint32x4_t; 2]>(t43);
                }
                break;
            }
            // Lanes part: each its own rounds.
            let t44 = [vdupq_n_u32(15u32); 2];
            let t45: [uint32x4_t; 2] = [vandq_u32(x4[0], t44[0]), vandq_u32(x4[1], t44[1])];
            let t46 = [vdupq_n_u32(x8); 2];
            let t47: [uint32x4_t; 2] = [vcltq_u32(t46[0], t45[0]), vcltq_u32(t46[1], t45[1])];
            t35 = [vandq_u32(t35[0], t47[0]), vandq_u32(t35[1], t47[1])];
            if !(any(&t35)) { break; }
            let t49 = [vdupq_n_u32(x8); 2];
            let t50: [uint32x4_t; 2] = [vmulq_u32(t49[0], x5[0]), vmulq_u32(t49[1], x5[1])];
            let t51: [uint32x4_t; 2] = [vaddq_u32(x6[0], t50[0]), vaddq_u32(x6[1], t50[1])];
            x6 = [vbslq_u32(t35[0], t51[0], x6[0]), vbslq_u32(t35[1], t51[1], x6[1])];
            let t52: [uint32x4_t; 2] = [vandq_u32(t35[0], live[0]), vandq_u32(t35[1], live[1])];
            let t53 = x8;
            x8 = (x8).wrapping_add(1u32);
        }
        loop {
            // Rounds of one condition: every lane goes on.
            if !(((x7) < (a1))) { break; }
            let t54 = [vdupq_n_u32(31u32); 2];
            let t55: [uint32x4_t; 2] = [vmulq_u32(x6[0], t54[0]), vmulq_u32(x6[1], t54[1])];
            let t56 = [vdupq_n_u32(x7); 2];
            let t57: [uint32x4_t; 2] = [vaddq_u32(t55[0], t56[0]), vaddq_u32(t55[1], t56[1])];
            x6 = t57;
            let t58 = x7;
            x7 = (x7).wrapping_add(1u32);
        }
        let t59: [uint32x4_t; 2] = [vaddq_u32(x5[0], x6[0]), vaddq_u32(x5[1], x6[1])];
        let t60 = (arg1 as *mut u32).add((gid0) as usize);
        if all(&live) {
            vst1q_u32(t60.add(0), t59[0]);
            vst1q_u32(t60.add(4), t59[1]);
        } else {
            let mut t61w = 0u32;
            let t61 = core::hint::black_box(&raw mut t61w);
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t60.add(0), t61).write(vgetq_lane_u32::<0>(t59[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t60.add(1), t61).write(vgetq_lane_u32::<1>(t59[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t60.add(2), t61).write(vgetq_lane_u32::<2>(t59[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t60.add(3), t61).write(vgetq_lane_u32::<3>(t59[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t60.add(4), t61).write(vgetq_lane_u32::<0>(t59[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t60.add(5), t61).write(vgetq_lane_u32::<1>(t59[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t60.add(6), t61).write(vgetq_lane_u32::<2>(t59[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t60.add(7), t61).write(vgetq_lane_u32::<3>(t59[1]));
        }
    }
}

/// `mix_lanes_inner`.
#[inline(always)]
unsafe fn b0_r0r1sg_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: *const u8) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x2 as *const u32).read_unaligned();
    let a1: u32 = (x2.wrapping_add(4u64 as usize) as *const u32).read_unaligned();
    let mut x4: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x5: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x6: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x7: u32 = 0;
    let mut x8: u32 = 0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    if ((gid0) as u64 + 7) < ((a0) as u64) {
        let t62 = (arg0 as *const u32).add((gid0) as usize);
        let t63: [uint32x4_t; 2] = [vld1q_u32(t62.add(0)), vld1q_u32(t62.add(4))];
        x4 = t63;
        let t64 = [vdupq_n_u32(1u32); 2];
        let t65: [uint32x4_t; 2] = [vandq_u32(x4[0], t64[0]), vandq_u32(x4[1], t64[1])];
        let t66: [uint32x4_t; 2] = [vtstq_u32(t65[0], t65[0]), vtstq_u32(t65[1], t65[1])];
        {
            let t67 = [vdupq_n_u32(3u32); 2];
            let t68: [uint32x4_t; 2] = [vmulq_u32(x4[0], t67[0]), vmulq_u32(x4[1], t67[1])];
            let t69: [uint32x4_t; 2] = [vaddq_u32(t68[0], t64[0]), vaddq_u32(t68[1], t64[1])];
            x5 = [vbslq_u32(t66[0], t69[0], x5[0]), vbslq_u32(t66[1], t69[1], x5[1])];
        }
        let t70: [uint32x4_t; 2] = [vmvnq_u32(t66[0]), vmvnq_u32(t66[1])];
        {
            let t71: [uint32x4_t; 2] = [vshlq_u32(x4[0], vnegq_s32(vdupq_n_s32(((1u32) & 31) as i32))), vshlq_u32(x4[1], vnegq_s32(vdupq_n_s32(((1u32) & 31) as i32)))];
            x5 = [vbslq_u32(t70[0], t71[0], x5[0]), vbslq_u32(t70[1], t71[1], x5[1])];
        }
        let t72 = [vdupq_n_u32(4026531840u32); 2];
        let t73: [uint32x4_t; 2] = [vcltq_u32(x4[0], t72[0]), vcltq_u32(x4[1], t72[1])];
        if all(&t73) {
            let t74: [uint32x4_t; 2] = [vaddq_u32(x5[0], x4[0]), vaddq_u32(x5[1], x4[1])];
            x5 = t74;
        } else {
            // Lanes part: the chunk's lanes go both ways.
            let t75: [uint32x4_t; 2] = [vandq_u32(live[0], t73[0]), vandq_u32(live[1], t73[1])];
            {
                let t76: [uint32x4_t; 2] = [vaddq_u32(x5[0], x4[0]), vaddq_u32(x5[1], x4[1])];
                x5 = [vbslq_u32(t75[0], t76[0], x5[0]), vbslq_u32(t75[1], t76[1], x5[1])];
            }
        }
        let t77 = [vdupq_n_u32(0u32); 2];
        let t78: [uint32x4_t; 2] = [vceqq_u32(x4[0], t77[0]), vceqq_u32(x4[1], t77[1])];
        if !any(&t78) {
            let t79 = [vdupq_n_u32(7u32); 2];
            let t80: [uint32x4_t; 2] = [veorq_u32(x5[0], t79[0]), veorq_u32(x5[1], t79[1])];
            x5 = t80;
        } else {
            // Lanes part: the chunk's lanes go both ways.
            let t81: [uint32x4_t; 2] = [vandq_u32(live[0], t78[0]), vandq_u32(live[1], t78[1])];
            {
                let t82 = [vdupq_n_u32(57005u32); 2];
                x5 = [vbslq_u32(t81[0], t82[0], x5[0]), vbslq_u32(t81[1], t82[1], x5[1])];
            }
            let t83: [uint32x4_t; 2] = [vbicq_u32(live[0], t78[0]), vbicq_u32(live[1], t78[1])];
            {
                let t84 = [vdupq_n_u32(7u32); 2];
                let t85: [uint32x4_t; 2] = [veorq_u32(x5[0], t84[0]), veorq_u32(x5[1], t84[1])];
                x5 = [vbslq_u32(t83[0], t85[0], x5[0]), vbslq_u32(t83[1], t85[1], x5[1])];
            }
        }
        x6 = t77;
        x7 = 0u32;
        x8 = 0u32;
        loop {
            // Full rounds: every lane goes on.
            let t90 = [vdupq_n_u32(15u32); 2];
            let t91: [uint32x4_t; 2] = [vandq_u32(x4[0], t90[0]), vandq_u32(x4[1], t90[1])];
            let t92 = [vdupq_n_u32(x8); 2];
            let t93: [uint32x4_t; 2] = [vcltq_u32(t92[0], t91[0]), vcltq_u32(t92[1], t91[1])];
            if !all(&t93) { break; }
            let t94 = [vdupq_n_u32(x8); 2];
            let t95: [uint32x4_t; 2] = [vmulq_u32(t94[0], x5[0]), vmulq_u32(t94[1], x5[1])];
            let t96: [uint32x4_t; 2] = [vaddq_u32(x6[0], t95[0]), vaddq_u32(x6[1], t95[1])];
            x6 = t96;
            let t97 = x8;
            x8 = (x8).wrapping_add(1u32);
        }
        let mut t98 = live;
        loop {
            let t100 = bits(&t98);
            if t100.count_ones() <= 2 {
                let mut t101 = t100;
                while t101 != 0 {
                    let j = t101.trailing_zeros() as usize;
                    t101 &= t101 - 1;
                    let t102 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x4);
                    let mut x4_j: u32 = t102[j];
                    let t103 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x5);
                    let mut x5_j: u32 = t103[j];
                    let t104 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x6);
                    let mut x6_j: u32 = t104[j];
                    let mut x8_j: u32 = x8;
                    loop {
                        if !(((x8_j) < (((x4_j) & (15u32))))) { break; }
                        x6_j = (x6_j).wrapping_add((x8_j).wrapping_mul(x5_j));
                        let t105 = x8_j;
                        x8_j = (x8_j).wrapping_add(1u32);
                    }
                    let mut t106 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x6);
                    t106[j] = x6_j as u32;
                    x6 = core::mem::transmute::<[u32; 8], [uint32x4_t; 2]>(t106);
                }
                break;
            }
            // Lanes part: each its own rounds.
            let t107 = [vdupq_n_u32(15u32); 2];
            let t108: [uint32x4_t; 2] = [vandq_u32(x4[0], t107[0]), vandq_u32(x4[1], t107[1])];
            let t109 = [vdupq_n_u32(x8); 2];
            let t110: [uint32x4_t; 2] = [vcltq_u32(t109[0], t108[0]), vcltq_u32(t109[1], t108[1])];
            t98 = [vandq_u32(t98[0], t110[0]), vandq_u32(t98[1], t110[1])];
            if !(any(&t98)) { break; }
            let t112 = [vdupq_n_u32(x8); 2];
            let t113: [uint32x4_t; 2] = [vmulq_u32(t112[0], x5[0]), vmulq_u32(t112[1], x5[1])];
            let t114: [uint32x4_t; 2] = [vaddq_u32(x6[0], t113[0]), vaddq_u32(x6[1], t113[1])];
            x6 = [vbslq_u32(t98[0], t114[0], x6[0]), vbslq_u32(t98[1], t114[1], x6[1])];
            let t115: [uint32x4_t; 2] = [vandq_u32(t98[0], live[0]), vandq_u32(t98[1], live[1])];
            let t116 = x8;
            x8 = (x8).wrapping_add(1u32);
        }
        loop {
            // Rounds of one condition: every lane goes on.
            if !(((x7) < (a1))) { break; }
            let t117 = [vdupq_n_u32(31u32); 2];
            let t118: [uint32x4_t; 2] = [vmulq_u32(x6[0], t117[0]), vmulq_u32(x6[1], t117[1])];
            let t119 = [vdupq_n_u32(x7); 2];
            let t120: [uint32x4_t; 2] = [vaddq_u32(t118[0], t119[0]), vaddq_u32(t118[1], t119[1])];
            x6 = t120;
            let t121 = x7;
            x7 = (x7).wrapping_add(1u32);
        }
        let t122: [uint32x4_t; 2] = [vaddq_u32(x5[0], x6[0]), vaddq_u32(x5[1], x6[1])];
        let t123 = (arg1 as *mut u32).add((gid0) as usize);
        vst1q_u32(t123.add(0), t122[0]);
        vst1q_u32(t123.add(4), t122[1]);
    } else if (((gid0) as u64 + 7) <= u32::MAX as u64 && ((gid0) as u64) >= ((a0) as u64)) {
        return;
    } else {
        // Lanes part: the chunk's lanes on both sides of the bound.
        let t124: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t125 = [vdupq_n_u32(a0); 2];
        let t126: [uint32x4_t; 2] = [vcgeq_u32(t124[0], t125[0]), vcgeq_u32(t124[1], t125[1])];
        let t127: [uint32x4_t; 2] = [vandq_u32(live[0], t126[0]), vandq_u32(live[1], t126[1])];
        {
            live = [vbicq_u32(live[0], t127[0]), vbicq_u32(live[1], t127[1])];
        }
        if any(&live) {
            let mut t128: [uint32x4_t; 2];
            if all(&live) {
                let t129 = (arg0 as *const u32).add((gid0) as usize);
                t128 = [vld1q_u32(t129.add(0)), vld1q_u32(t129.add(4))];
            } else {
                let t130 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t124[0]), vandq_u32(live[1], t124[1]))));
                let t131: [uint32x4_t; 2] = [vbslq_u32(live[0], t124[0], t130), vbslq_u32(live[1], t124[1], t130)];
                let t132 = arg0 as *const u32;
                let mut t133: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                t133[0] = vld1q_lane_u32::<0>(t132.add(vgetq_lane_u32::<0>(t131[0]) as usize), t133[0]);
                t133[0] = vld1q_lane_u32::<1>(t132.add(vgetq_lane_u32::<1>(t131[0]) as usize), t133[0]);
                t133[0] = vld1q_lane_u32::<2>(t132.add(vgetq_lane_u32::<2>(t131[0]) as usize), t133[0]);
                t133[0] = vld1q_lane_u32::<3>(t132.add(vgetq_lane_u32::<3>(t131[0]) as usize), t133[0]);
                t133[1] = vld1q_lane_u32::<0>(t132.add(vgetq_lane_u32::<0>(t131[1]) as usize), t133[1]);
                t133[1] = vld1q_lane_u32::<1>(t132.add(vgetq_lane_u32::<1>(t131[1]) as usize), t133[1]);
                t133[1] = vld1q_lane_u32::<2>(t132.add(vgetq_lane_u32::<2>(t131[1]) as usize), t133[1]);
                t133[1] = vld1q_lane_u32::<3>(t132.add(vgetq_lane_u32::<3>(t131[1]) as usize), t133[1]);
                t128 = t133;
            }
            x4 = t128;
            let t134 = [vdupq_n_u32(1u32); 2];
            let t135: [uint32x4_t; 2] = [vandq_u32(x4[0], t134[0]), vandq_u32(x4[1], t134[1])];
            let t136: [uint32x4_t; 2] = [vtstq_u32(t135[0], t135[0]), vtstq_u32(t135[1], t135[1])];
            let t137: [uint32x4_t; 2] = [vandq_u32(live[0], t136[0]), vandq_u32(live[1], t136[1])];
            {
                let t138 = [vdupq_n_u32(3u32); 2];
                let t139: [uint32x4_t; 2] = [vmulq_u32(x4[0], t138[0]), vmulq_u32(x4[1], t138[1])];
                let t140: [uint32x4_t; 2] = [vaddq_u32(t139[0], t134[0]), vaddq_u32(t139[1], t134[1])];
                x5 = [vbslq_u32(t137[0], t140[0], x5[0]), vbslq_u32(t137[1], t140[1], x5[1])];
            }
            let t141: [uint32x4_t; 2] = [vbicq_u32(live[0], t136[0]), vbicq_u32(live[1], t136[1])];
            {
                let t142: [uint32x4_t; 2] = [vshlq_u32(x4[0], vnegq_s32(vdupq_n_s32(((1u32) & 31) as i32))), vshlq_u32(x4[1], vnegq_s32(vdupq_n_s32(((1u32) & 31) as i32)))];
                x5 = [vbslq_u32(t141[0], t142[0], x5[0]), vbslq_u32(t141[1], t142[1], x5[1])];
            }
            let t143 = [vdupq_n_u32(4026531840u32); 2];
            let t144: [uint32x4_t; 2] = [vcltq_u32(x4[0], t143[0]), vcltq_u32(x4[1], t143[1])];
            let t145: [uint32x4_t; 2] = [vandq_u32(live[0], t144[0]), vandq_u32(live[1], t144[1])];
            {
                let t146: [uint32x4_t; 2] = [vaddq_u32(x5[0], x4[0]), vaddq_u32(x5[1], x4[1])];
                x5 = [vbslq_u32(t145[0], t146[0], x5[0]), vbslq_u32(t145[1], t146[1], x5[1])];
            }
            let t147 = [vdupq_n_u32(0u32); 2];
            let t148: [uint32x4_t; 2] = [vceqq_u32(x4[0], t147[0]), vceqq_u32(x4[1], t147[1])];
            let t149: [uint32x4_t; 2] = [vandq_u32(live[0], t148[0]), vandq_u32(live[1], t148[1])];
            {
                let t150 = [vdupq_n_u32(57005u32); 2];
                x5 = [vbslq_u32(t149[0], t150[0], x5[0]), vbslq_u32(t149[1], t150[1], x5[1])];
            }
            let t151: [uint32x4_t; 2] = [vbicq_u32(live[0], t148[0]), vbicq_u32(live[1], t148[1])];
            {
                let t152 = [vdupq_n_u32(7u32); 2];
                let t153: [uint32x4_t; 2] = [veorq_u32(x5[0], t152[0]), veorq_u32(x5[1], t152[1])];
                x5 = [vbslq_u32(t151[0], t153[0], x5[0]), vbslq_u32(t151[1], t153[1], x5[1])];
            }
            x6 = t147;
            x7 = 0u32;
            x8 = 0u32;
            let mut t158 = live;
            loop {
                let t160 = bits(&t158);
                if t160.count_ones() <= 2 {
                    let mut t161 = t160;
                    while t161 != 0 {
                        let j = t161.trailing_zeros() as usize;
                        t161 &= t161 - 1;
                        let t162 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x4);
                        let mut x4_j: u32 = t162[j];
                        let t163 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x5);
                        let mut x5_j: u32 = t163[j];
                        let t164 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x6);
                        let mut x6_j: u32 = t164[j];
                        let mut x8_j: u32 = x8;
                        loop {
                            if !(((x8_j) < (((x4_j) & (15u32))))) { break; }
                            x6_j = (x6_j).wrapping_add((x8_j).wrapping_mul(x5_j));
                            let t165 = x8_j;
                            x8_j = (x8_j).wrapping_add(1u32);
                        }
                        let mut t166 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x6);
                        t166[j] = x6_j as u32;
                        x6 = core::mem::transmute::<[u32; 8], [uint32x4_t; 2]>(t166);
                    }
                    break;
                }
                // Lanes part: each its own rounds.
                let t167 = [vdupq_n_u32(15u32); 2];
                let t168: [uint32x4_t; 2] = [vandq_u32(x4[0], t167[0]), vandq_u32(x4[1], t167[1])];
                let t169 = [vdupq_n_u32(x8); 2];
                let t170: [uint32x4_t; 2] = [vcltq_u32(t169[0], t168[0]), vcltq_u32(t169[1], t168[1])];
                t158 = [vandq_u32(t158[0], t170[0]), vandq_u32(t158[1], t170[1])];
                if !(any(&t158)) { break; }
                let t172 = [vdupq_n_u32(x8); 2];
                let t173: [uint32x4_t; 2] = [vmulq_u32(t172[0], x5[0]), vmulq_u32(t172[1], x5[1])];
                let t174: [uint32x4_t; 2] = [vaddq_u32(x6[0], t173[0]), vaddq_u32(x6[1], t173[1])];
                x6 = [vbslq_u32(t158[0], t174[0], x6[0]), vbslq_u32(t158[1], t174[1], x6[1])];
                let t175: [uint32x4_t; 2] = [vandq_u32(t158[0], live[0]), vandq_u32(t158[1], live[1])];
                let t176 = x8;
                x8 = (x8).wrapping_add(1u32);
            }
            loop {
                // Rounds of one condition: every lane goes on.
                if !(((x7) < (a1))) { break; }
                let t177 = [vdupq_n_u32(31u32); 2];
                let t178: [uint32x4_t; 2] = [vmulq_u32(x6[0], t177[0]), vmulq_u32(x6[1], t177[1])];
                let t179 = [vdupq_n_u32(x7); 2];
                let t180: [uint32x4_t; 2] = [vaddq_u32(t178[0], t179[0]), vaddq_u32(t178[1], t179[1])];
                x6 = t180;
                let t181 = x7;
                x7 = (x7).wrapping_add(1u32);
            }
            let t182: [uint32x4_t; 2] = [vaddq_u32(x5[0], x6[0]), vaddq_u32(x5[1], x6[1])];
            let t183 = (arg1 as *mut u32).add((gid0) as usize);
            if all(&live) {
                vst1q_u32(t183.add(0), t182[0]);
                vst1q_u32(t183.add(4), t182[1]);
            } else {
                let mut t184w = 0u32;
                let t184 = core::hint::black_box(&raw mut t184w);
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t183.add(0), t184).write(vgetq_lane_u32::<0>(t182[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t183.add(1), t184).write(vgetq_lane_u32::<1>(t182[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t183.add(2), t184).write(vgetq_lane_u32::<2>(t182[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t183.add(3), t184).write(vgetq_lane_u32::<3>(t182[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t183.add(4), t184).write(vgetq_lane_u32::<0>(t182[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t183.add(5), t184).write(vgetq_lane_u32::<1>(t182[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t183.add(6), t184).write(vgetq_lane_u32::<2>(t182[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t183.add(7), t184).write(vgetq_lane_u32::<3>(t182[1]));
            }
        }
    }
}

/// The group `l.group`'s threads, 8 at a time: `argv[j]` the address of argument `j`'s bytes (a buffer's address,
/// a value's bytes).
///
/// # Safety
///
/// `argv` holds the kernel's arguments, its buffers as large as it reads and writes; `l` is a 1-D group's launch.
pub unsafe fn mix_lanes_w8(argv: *const *const u8, l: &Launch) {
    let n = l.block[0] * l.block[1] * l.block[2];
    let arg0: *const u8 = (*argv.add(0) as *const *const u8).read_unaligned();
    let arg1: *const u8 = (*argv.add(1) as *const *const u8).read_unaligned();
    let x2: *const u8 = *argv.add(2);
    let arg3: *const u8 = (*argv.add(3) as *const *const u8).read_unaligned();
    let x4: u32 = (*argv.add(4) as *const u32).read_unaligned();
    let mut t185: bool = !(arg3).is_null();
    if t185 {
        let mut t186: bool = ((l.group[0]) >= ((arg3.wrapping_add((((x4) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        if !t186 {
            t186 = ((l.group[1]) >= ((arg3.wrapping_add(((((x4).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        let mut t187: bool = t186;
        if !t187 {
            t187 = ((l.group[2]) >= ((arg3.wrapping_add(((((x4).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        t185 = t187;
    }
    if t185 {
        return;
    }
    let mut lid0 = 0u32;
    while lid0 + 8 <= n {
        let mut live = [vdupq_n_u32(u32::MAX); 2];
        let gid0 = l.group[0] * l.block[0] + lid0;
        'chunk: {
            b0_r0r1sg_f(l, lid0, arg0, arg1, x2);
        }
        lid0 += 8;
    }
    if lid0 < n {
        // Lanes part: the group's last threads, those under `n`.
        let t188: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
        let mut live = t188;
        let gid0 = l.group[0] * l.block[0] + lid0;
        'chunk: {
            b0_r0r1sg_p(l, lid0, live, arg0, arg1, x2);
        }
    }
}
