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

#[inline(always)]
unsafe fn b0_r0r1sg_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: *const u8) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x16 as *const u32).read_unaligned();
    let a1: u32 = (x16.wrapping_add(4u64 as usize) as *const u32).read_unaligned();
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x56: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x64: u32 = 0;
    let mut x72: u32 = 0;
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
        x40 = t5;
        let t11 = [vdupq_n_u32(1u32); 2];
        let t12: [uint32x4_t; 2] = [vandq_u32(x40[0], t11[0]), vandq_u32(x40[1], t11[1])];
        let t13: [uint32x4_t; 2] = [vtstq_u32(t12[0], t12[0]), vtstq_u32(t12[1], t12[1])];
        let t14: [uint32x4_t; 2] = [vandq_u32(live[0], t13[0]), vandq_u32(live[1], t13[1])];
        {
            let t15 = [vdupq_n_u32(3u32); 2];
            let t16: [uint32x4_t; 2] = [vmulq_u32(x40[0], t15[0]), vmulq_u32(x40[1], t15[1])];
            let t17: [uint32x4_t; 2] = [vaddq_u32(t16[0], t11[0]), vaddq_u32(t16[1], t11[1])];
            x48 = [vbslq_u32(t14[0], t17[0], x48[0]), vbslq_u32(t14[1], t17[1], x48[1])];
        }
        let t18: [uint32x4_t; 2] = [vbicq_u32(live[0], t13[0]), vbicq_u32(live[1], t13[1])];
        {
            let t19: [uint32x4_t; 2] = [vshlq_u32(x40[0], vnegq_s32(vdupq_n_s32(((1u32) & 31) as i32))), vshlq_u32(x40[1], vnegq_s32(vdupq_n_s32(((1u32) & 31) as i32)))];
            x48 = [vbslq_u32(t18[0], t19[0], x48[0]), vbslq_u32(t18[1], t19[1], x48[1])];
        }
        let t20 = [vdupq_n_u32(4026531840u32); 2];
        let t21: [uint32x4_t; 2] = [vcltq_u32(x40[0], t20[0]), vcltq_u32(x40[1], t20[1])];
        let t22: [uint32x4_t; 2] = [vandq_u32(live[0], t21[0]), vandq_u32(live[1], t21[1])];
        {
            let t23: [uint32x4_t; 2] = [vaddq_u32(x48[0], x40[0]), vaddq_u32(x48[1], x40[1])];
            x48 = [vbslq_u32(t22[0], t23[0], x48[0]), vbslq_u32(t22[1], t23[1], x48[1])];
        }
        let t24 = [vdupq_n_u32(0u32); 2];
        let t25: [uint32x4_t; 2] = [vceqq_u32(x40[0], t24[0]), vceqq_u32(x40[1], t24[1])];
        let t26: [uint32x4_t; 2] = [vandq_u32(live[0], t25[0]), vandq_u32(live[1], t25[1])];
        {
            let t27 = [vdupq_n_u32(57005u32); 2];
            x48 = [vbslq_u32(t26[0], t27[0], x48[0]), vbslq_u32(t26[1], t27[1], x48[1])];
        }
        let t28: [uint32x4_t; 2] = [vbicq_u32(live[0], t25[0]), vbicq_u32(live[1], t25[1])];
        {
            let t29 = [vdupq_n_u32(7u32); 2];
            let t30: [uint32x4_t; 2] = [veorq_u32(x48[0], t29[0]), veorq_u32(x48[1], t29[1])];
            x48 = [vbslq_u32(t28[0], t30[0], x48[0]), vbslq_u32(t28[1], t30[1], x48[1])];
        }
        x56 = t24;
        x64 = 0u32;
        x72 = 0u32;
        let mut t35 = live;
        loop {
            // Lanes part: each its own rounds.
            let t37 = [vdupq_n_u32(15u32); 2];
            let t38: [uint32x4_t; 2] = [vandq_u32(x40[0], t37[0]), vandq_u32(x40[1], t37[1])];
            let t39 = [vdupq_n_u32(x72); 2];
            let t40: [uint32x4_t; 2] = [vcltq_u32(t39[0], t38[0]), vcltq_u32(t39[1], t38[1])];
            t35 = [vandq_u32(t35[0], t40[0]), vandq_u32(t35[1], t40[1])];
            if !(any(&t35)) { break; }
            let t42 = [vdupq_n_u32(x72); 2];
            let t43: [uint32x4_t; 2] = [vmulq_u32(t42[0], x48[0]), vmulq_u32(t42[1], x48[1])];
            let t44: [uint32x4_t; 2] = [vaddq_u32(x56[0], t43[0]), vaddq_u32(x56[1], t43[1])];
            x56 = [vbslq_u32(t35[0], t44[0], x56[0]), vbslq_u32(t35[1], t44[1], x56[1])];
            let t45: [uint32x4_t; 2] = [vandq_u32(t35[0], live[0]), vandq_u32(t35[1], live[1])];
            let t46 = x72;
            x72 = (x72).wrapping_add(1u32);
        }
        loop {
            // Rounds of one condition: every lane goes on.
            if !(((x64) < (a1))) { break; }
            let t47 = [vdupq_n_u32(31u32); 2];
            let t48: [uint32x4_t; 2] = [vmulq_u32(x56[0], t47[0]), vmulq_u32(x56[1], t47[1])];
            let t49 = [vdupq_n_u32(x64); 2];
            let t50: [uint32x4_t; 2] = [vaddq_u32(t48[0], t49[0]), vaddq_u32(t48[1], t49[1])];
            x56 = t50;
            let t51 = x64;
            x64 = (x64).wrapping_add(1u32);
        }
        let t52: [uint32x4_t; 2] = [vaddq_u32(x48[0], x56[0]), vaddq_u32(x48[1], x56[1])];
        let t53 = (arg1 as *mut u32).add((gid0) as usize);
        if all(&live) {
            vst1q_u32(t53.add(0), t52[0]);
            vst1q_u32(t53.add(4), t52[1]);
        } else {
            let mut t54w = 0u32;
            let t54 = core::hint::black_box(&raw mut t54w);
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t53.add(0), t54).write(vgetq_lane_u32::<0>(t52[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t53.add(1), t54).write(vgetq_lane_u32::<1>(t52[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t53.add(2), t54).write(vgetq_lane_u32::<2>(t52[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t53.add(3), t54).write(vgetq_lane_u32::<3>(t52[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t53.add(4), t54).write(vgetq_lane_u32::<0>(t52[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t53.add(5), t54).write(vgetq_lane_u32::<1>(t52[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t53.add(6), t54).write(vgetq_lane_u32::<2>(t52[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t53.add(7), t54).write(vgetq_lane_u32::<3>(t52[1]));
        }
    }
}

#[inline(always)]
unsafe fn b0_r0r1sg_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: *const u8) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x16 as *const u32).read_unaligned();
    let a1: u32 = (x16.wrapping_add(4u64 as usize) as *const u32).read_unaligned();
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x56: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x64: u32 = 0;
    let mut x72: u32 = 0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    if ((gid0) as u64 + 7) < ((a0) as u64) {
        let t55 = (arg0 as *const u32).add((gid0) as usize);
        let t56: [uint32x4_t; 2] = [vld1q_u32(t55.add(0)), vld1q_u32(t55.add(4))];
        x40 = t56;
        let t57 = [vdupq_n_u32(1u32); 2];
        let t58: [uint32x4_t; 2] = [vandq_u32(x40[0], t57[0]), vandq_u32(x40[1], t57[1])];
        let t59: [uint32x4_t; 2] = [vtstq_u32(t58[0], t58[0]), vtstq_u32(t58[1], t58[1])];
        {
            let t60 = [vdupq_n_u32(3u32); 2];
            let t61: [uint32x4_t; 2] = [vmulq_u32(x40[0], t60[0]), vmulq_u32(x40[1], t60[1])];
            let t62: [uint32x4_t; 2] = [vaddq_u32(t61[0], t57[0]), vaddq_u32(t61[1], t57[1])];
            x48 = [vbslq_u32(t59[0], t62[0], x48[0]), vbslq_u32(t59[1], t62[1], x48[1])];
        }
        let t63: [uint32x4_t; 2] = [vmvnq_u32(t59[0]), vmvnq_u32(t59[1])];
        {
            let t64: [uint32x4_t; 2] = [vshlq_u32(x40[0], vnegq_s32(vdupq_n_s32(((1u32) & 31) as i32))), vshlq_u32(x40[1], vnegq_s32(vdupq_n_s32(((1u32) & 31) as i32)))];
            x48 = [vbslq_u32(t63[0], t64[0], x48[0]), vbslq_u32(t63[1], t64[1], x48[1])];
        }
        let t65 = [vdupq_n_u32(4026531840u32); 2];
        let t66: [uint32x4_t; 2] = [vcltq_u32(x40[0], t65[0]), vcltq_u32(x40[1], t65[1])];
        if all(&t66) {
            let t67: [uint32x4_t; 2] = [vaddq_u32(x48[0], x40[0]), vaddq_u32(x48[1], x40[1])];
            x48 = t67;
        } else {
            // Lanes part: the chunk's lanes go both ways.
            let t68: [uint32x4_t; 2] = [vandq_u32(live[0], t66[0]), vandq_u32(live[1], t66[1])];
            {
                let t69: [uint32x4_t; 2] = [vaddq_u32(x48[0], x40[0]), vaddq_u32(x48[1], x40[1])];
                x48 = [vbslq_u32(t68[0], t69[0], x48[0]), vbslq_u32(t68[1], t69[1], x48[1])];
            }
        }
        let t70 = [vdupq_n_u32(0u32); 2];
        let t71: [uint32x4_t; 2] = [vceqq_u32(x40[0], t70[0]), vceqq_u32(x40[1], t70[1])];
        if !any(&t71) {
            let t72 = [vdupq_n_u32(7u32); 2];
            let t73: [uint32x4_t; 2] = [veorq_u32(x48[0], t72[0]), veorq_u32(x48[1], t72[1])];
            x48 = t73;
        } else {
            // Lanes part: the chunk's lanes go both ways.
            let t74: [uint32x4_t; 2] = [vandq_u32(live[0], t71[0]), vandq_u32(live[1], t71[1])];
            {
                let t75 = [vdupq_n_u32(57005u32); 2];
                x48 = [vbslq_u32(t74[0], t75[0], x48[0]), vbslq_u32(t74[1], t75[1], x48[1])];
            }
            let t76: [uint32x4_t; 2] = [vbicq_u32(live[0], t71[0]), vbicq_u32(live[1], t71[1])];
            {
                let t77 = [vdupq_n_u32(7u32); 2];
                let t78: [uint32x4_t; 2] = [veorq_u32(x48[0], t77[0]), veorq_u32(x48[1], t77[1])];
                x48 = [vbslq_u32(t76[0], t78[0], x48[0]), vbslq_u32(t76[1], t78[1], x48[1])];
            }
        }
        x56 = t70;
        x64 = 0u32;
        x72 = 0u32;
        loop {
            // Full rounds: every lane goes on.
            let t83 = [vdupq_n_u32(15u32); 2];
            let t84: [uint32x4_t; 2] = [vandq_u32(x40[0], t83[0]), vandq_u32(x40[1], t83[1])];
            let t85 = [vdupq_n_u32(x72); 2];
            let t86: [uint32x4_t; 2] = [vcltq_u32(t85[0], t84[0]), vcltq_u32(t85[1], t84[1])];
            if !all(&t86) { break; }
            let t87 = [vdupq_n_u32(x72); 2];
            let t88: [uint32x4_t; 2] = [vmulq_u32(t87[0], x48[0]), vmulq_u32(t87[1], x48[1])];
            let t89: [uint32x4_t; 2] = [vaddq_u32(x56[0], t88[0]), vaddq_u32(x56[1], t88[1])];
            x56 = t89;
            let t90 = x72;
            x72 = (x72).wrapping_add(1u32);
        }
        let mut t91 = live;
        loop {
            // Lanes part: each its own rounds.
            let t93 = [vdupq_n_u32(15u32); 2];
            let t94: [uint32x4_t; 2] = [vandq_u32(x40[0], t93[0]), vandq_u32(x40[1], t93[1])];
            let t95 = [vdupq_n_u32(x72); 2];
            let t96: [uint32x4_t; 2] = [vcltq_u32(t95[0], t94[0]), vcltq_u32(t95[1], t94[1])];
            t91 = [vandq_u32(t91[0], t96[0]), vandq_u32(t91[1], t96[1])];
            if !(any(&t91)) { break; }
            let t98 = [vdupq_n_u32(x72); 2];
            let t99: [uint32x4_t; 2] = [vmulq_u32(t98[0], x48[0]), vmulq_u32(t98[1], x48[1])];
            let t100: [uint32x4_t; 2] = [vaddq_u32(x56[0], t99[0]), vaddq_u32(x56[1], t99[1])];
            x56 = [vbslq_u32(t91[0], t100[0], x56[0]), vbslq_u32(t91[1], t100[1], x56[1])];
            let t101: [uint32x4_t; 2] = [vandq_u32(t91[0], live[0]), vandq_u32(t91[1], live[1])];
            let t102 = x72;
            x72 = (x72).wrapping_add(1u32);
        }
        loop {
            // Rounds of one condition: every lane goes on.
            if !(((x64) < (a1))) { break; }
            let t103 = [vdupq_n_u32(31u32); 2];
            let t104: [uint32x4_t; 2] = [vmulq_u32(x56[0], t103[0]), vmulq_u32(x56[1], t103[1])];
            let t105 = [vdupq_n_u32(x64); 2];
            let t106: [uint32x4_t; 2] = [vaddq_u32(t104[0], t105[0]), vaddq_u32(t104[1], t105[1])];
            x56 = t106;
            let t107 = x64;
            x64 = (x64).wrapping_add(1u32);
        }
        let t108: [uint32x4_t; 2] = [vaddq_u32(x48[0], x56[0]), vaddq_u32(x48[1], x56[1])];
        let t109 = (arg1 as *mut u32).add((gid0) as usize);
        vst1q_u32(t109.add(0), t108[0]);
        vst1q_u32(t109.add(4), t108[1]);
    } else if (((gid0) as u64 + 7) <= u32::MAX as u64 && ((gid0) as u64) >= ((a0) as u64)) {
        return;
    } else {
        // Lanes part: the chunk's lanes on both sides of the bound.
        let t110: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t111 = [vdupq_n_u32(a0); 2];
        let t112: [uint32x4_t; 2] = [vcgeq_u32(t110[0], t111[0]), vcgeq_u32(t110[1], t111[1])];
        let t113: [uint32x4_t; 2] = [vandq_u32(live[0], t112[0]), vandq_u32(live[1], t112[1])];
        {
            live = [vbicq_u32(live[0], t113[0]), vbicq_u32(live[1], t113[1])];
        }
        if any(&live) {
            let mut t114: [uint32x4_t; 2];
            if all(&live) {
                let t115 = (arg0 as *const u32).add((gid0) as usize);
                t114 = [vld1q_u32(t115.add(0)), vld1q_u32(t115.add(4))];
            } else {
                let t116 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t110[0]), vandq_u32(live[1], t110[1]))));
                let t117: [uint32x4_t; 2] = [vbslq_u32(live[0], t110[0], t116), vbslq_u32(live[1], t110[1], t116)];
                let t118 = arg0 as *const u32;
                let mut t119: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                t119[0] = vld1q_lane_u32::<0>(t118.add(vgetq_lane_u32::<0>(t117[0]) as usize), t119[0]);
                t119[0] = vld1q_lane_u32::<1>(t118.add(vgetq_lane_u32::<1>(t117[0]) as usize), t119[0]);
                t119[0] = vld1q_lane_u32::<2>(t118.add(vgetq_lane_u32::<2>(t117[0]) as usize), t119[0]);
                t119[0] = vld1q_lane_u32::<3>(t118.add(vgetq_lane_u32::<3>(t117[0]) as usize), t119[0]);
                t119[1] = vld1q_lane_u32::<0>(t118.add(vgetq_lane_u32::<0>(t117[1]) as usize), t119[1]);
                t119[1] = vld1q_lane_u32::<1>(t118.add(vgetq_lane_u32::<1>(t117[1]) as usize), t119[1]);
                t119[1] = vld1q_lane_u32::<2>(t118.add(vgetq_lane_u32::<2>(t117[1]) as usize), t119[1]);
                t119[1] = vld1q_lane_u32::<3>(t118.add(vgetq_lane_u32::<3>(t117[1]) as usize), t119[1]);
                t114 = t119;
            }
            x40 = t114;
            let t120 = [vdupq_n_u32(1u32); 2];
            let t121: [uint32x4_t; 2] = [vandq_u32(x40[0], t120[0]), vandq_u32(x40[1], t120[1])];
            let t122: [uint32x4_t; 2] = [vtstq_u32(t121[0], t121[0]), vtstq_u32(t121[1], t121[1])];
            let t123: [uint32x4_t; 2] = [vandq_u32(live[0], t122[0]), vandq_u32(live[1], t122[1])];
            {
                let t124 = [vdupq_n_u32(3u32); 2];
                let t125: [uint32x4_t; 2] = [vmulq_u32(x40[0], t124[0]), vmulq_u32(x40[1], t124[1])];
                let t126: [uint32x4_t; 2] = [vaddq_u32(t125[0], t120[0]), vaddq_u32(t125[1], t120[1])];
                x48 = [vbslq_u32(t123[0], t126[0], x48[0]), vbslq_u32(t123[1], t126[1], x48[1])];
            }
            let t127: [uint32x4_t; 2] = [vbicq_u32(live[0], t122[0]), vbicq_u32(live[1], t122[1])];
            {
                let t128: [uint32x4_t; 2] = [vshlq_u32(x40[0], vnegq_s32(vdupq_n_s32(((1u32) & 31) as i32))), vshlq_u32(x40[1], vnegq_s32(vdupq_n_s32(((1u32) & 31) as i32)))];
                x48 = [vbslq_u32(t127[0], t128[0], x48[0]), vbslq_u32(t127[1], t128[1], x48[1])];
            }
            let t129 = [vdupq_n_u32(4026531840u32); 2];
            let t130: [uint32x4_t; 2] = [vcltq_u32(x40[0], t129[0]), vcltq_u32(x40[1], t129[1])];
            let t131: [uint32x4_t; 2] = [vandq_u32(live[0], t130[0]), vandq_u32(live[1], t130[1])];
            {
                let t132: [uint32x4_t; 2] = [vaddq_u32(x48[0], x40[0]), vaddq_u32(x48[1], x40[1])];
                x48 = [vbslq_u32(t131[0], t132[0], x48[0]), vbslq_u32(t131[1], t132[1], x48[1])];
            }
            let t133 = [vdupq_n_u32(0u32); 2];
            let t134: [uint32x4_t; 2] = [vceqq_u32(x40[0], t133[0]), vceqq_u32(x40[1], t133[1])];
            let t135: [uint32x4_t; 2] = [vandq_u32(live[0], t134[0]), vandq_u32(live[1], t134[1])];
            {
                let t136 = [vdupq_n_u32(57005u32); 2];
                x48 = [vbslq_u32(t135[0], t136[0], x48[0]), vbslq_u32(t135[1], t136[1], x48[1])];
            }
            let t137: [uint32x4_t; 2] = [vbicq_u32(live[0], t134[0]), vbicq_u32(live[1], t134[1])];
            {
                let t138 = [vdupq_n_u32(7u32); 2];
                let t139: [uint32x4_t; 2] = [veorq_u32(x48[0], t138[0]), veorq_u32(x48[1], t138[1])];
                x48 = [vbslq_u32(t137[0], t139[0], x48[0]), vbslq_u32(t137[1], t139[1], x48[1])];
            }
            x56 = t133;
            x64 = 0u32;
            x72 = 0u32;
            let mut t144 = live;
            loop {
                // Lanes part: each its own rounds.
                let t146 = [vdupq_n_u32(15u32); 2];
                let t147: [uint32x4_t; 2] = [vandq_u32(x40[0], t146[0]), vandq_u32(x40[1], t146[1])];
                let t148 = [vdupq_n_u32(x72); 2];
                let t149: [uint32x4_t; 2] = [vcltq_u32(t148[0], t147[0]), vcltq_u32(t148[1], t147[1])];
                t144 = [vandq_u32(t144[0], t149[0]), vandq_u32(t144[1], t149[1])];
                if !(any(&t144)) { break; }
                let t151 = [vdupq_n_u32(x72); 2];
                let t152: [uint32x4_t; 2] = [vmulq_u32(t151[0], x48[0]), vmulq_u32(t151[1], x48[1])];
                let t153: [uint32x4_t; 2] = [vaddq_u32(x56[0], t152[0]), vaddq_u32(x56[1], t152[1])];
                x56 = [vbslq_u32(t144[0], t153[0], x56[0]), vbslq_u32(t144[1], t153[1], x56[1])];
                let t154: [uint32x4_t; 2] = [vandq_u32(t144[0], live[0]), vandq_u32(t144[1], live[1])];
                let t155 = x72;
                x72 = (x72).wrapping_add(1u32);
            }
            loop {
                // Rounds of one condition: every lane goes on.
                if !(((x64) < (a1))) { break; }
                let t156 = [vdupq_n_u32(31u32); 2];
                let t157: [uint32x4_t; 2] = [vmulq_u32(x56[0], t156[0]), vmulq_u32(x56[1], t156[1])];
                let t158 = [vdupq_n_u32(x64); 2];
                let t159: [uint32x4_t; 2] = [vaddq_u32(t157[0], t158[0]), vaddq_u32(t157[1], t158[1])];
                x56 = t159;
                let t160 = x64;
                x64 = (x64).wrapping_add(1u32);
            }
            let t161: [uint32x4_t; 2] = [vaddq_u32(x48[0], x56[0]), vaddq_u32(x48[1], x56[1])];
            let t162 = (arg1 as *mut u32).add((gid0) as usize);
            if all(&live) {
                vst1q_u32(t162.add(0), t161[0]);
                vst1q_u32(t162.add(4), t161[1]);
            } else {
                let mut t163w = 0u32;
                let t163 = core::hint::black_box(&raw mut t163w);
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t162.add(0), t163).write(vgetq_lane_u32::<0>(t161[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t162.add(1), t163).write(vgetq_lane_u32::<1>(t161[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t162.add(2), t163).write(vgetq_lane_u32::<2>(t161[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t162.add(3), t163).write(vgetq_lane_u32::<3>(t161[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t162.add(4), t163).write(vgetq_lane_u32::<0>(t161[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t162.add(5), t163).write(vgetq_lane_u32::<1>(t161[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t162.add(6), t163).write(vgetq_lane_u32::<2>(t161[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t162.add(7), t163).write(vgetq_lane_u32::<3>(t161[1]));
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
    let x16: *const u8 = *argv.add(2);
    let arg3: *const u8 = (*argv.add(3) as *const *const u8).read_unaligned();
    let x40: u32 = (*argv.add(4) as *const u32).read_unaligned();
    let mut t164: bool = !(arg3).is_null();
    if t164 {
        let mut t165: bool = ((l.group[0]) >= ((arg3.wrapping_add((((x40) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        if !t165 {
            t165 = ((l.group[1]) >= ((arg3.wrapping_add(((((x40).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        let mut t166: bool = t165;
        if !t166 {
            t166 = ((l.group[2]) >= ((arg3.wrapping_add(((((x40).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        t164 = t166;
    }
    if t164 {
        return;
    }
    let mut lid0 = 0u32;
    while lid0 + 8 <= n {
        let mut live = [vdupq_n_u32(u32::MAX); 2];
        let gid0 = l.group[0] * l.block[0] + lid0;
        'chunk: {
            b0_r0r1sg_f(l, lid0, arg0, arg1, x16);
        }
        lid0 += 8;
    }
    if lid0 < n {
        // Lanes part: the group's last threads, those under `n`.
        let t167: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
        let mut live = t167;
        let gid0 = l.group[0] * l.block[0] + lid0;
        'chunk: {
            b0_r0r1sg_p(l, lid0, live, arg0, arg1, x16);
        }
    }
}
