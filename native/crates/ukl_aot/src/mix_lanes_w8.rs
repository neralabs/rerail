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

#[inline(always)]
unsafe fn b0_r0r1sg_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: *const u8) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x16 as *const u32).read_unaligned();
    let a1: u32 = (x16.wrapping_add(4u64 as usize) as *const u32).read_unaligned();
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x56: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x64: u32 = 0;
    let mut x72: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
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
        x72 = t24;
        let mut t31 = live;
        loop {
            // Lanes part: each its own rounds.
            let t33 = [vdupq_n_u32(15u32); 2];
            let t34: [uint32x4_t; 2] = [vandq_u32(x40[0], t33[0]), vandq_u32(x40[1], t33[1])];
            let t35: [uint32x4_t; 2] = [vcltq_u32(x72[0], t34[0]), vcltq_u32(x72[1], t34[1])];
            t31 = [vandq_u32(t31[0], t35[0]), vandq_u32(t31[1], t35[1])];
            if !(any(&t31)) { break; }
            let t37: [uint32x4_t; 2] = [vmulq_u32(x72[0], x48[0]), vmulq_u32(x72[1], x48[1])];
            let t38: [uint32x4_t; 2] = [vaddq_u32(x56[0], t37[0]), vaddq_u32(x56[1], t37[1])];
            x56 = [vbslq_u32(t31[0], t38[0], x56[0]), vbslq_u32(t31[1], t38[1], x56[1])];
            let t39: [uint32x4_t; 2] = [vandq_u32(t31[0], live[0]), vandq_u32(t31[1], live[1])];
            let t40: [uint32x4_t; 2] = [vaddq_u32(x72[0], t11[0]), vaddq_u32(x72[1], t11[1])];
            let t41 = x72;
            x72 = t40;
        }
        loop {
            // Rounds of one condition: every lane goes on.
            if !(((x64) < (a1))) { break; }
            let t42 = [vdupq_n_u32(31u32); 2];
            let t43: [uint32x4_t; 2] = [vmulq_u32(x56[0], t42[0]), vmulq_u32(x56[1], t42[1])];
            let t44 = [vdupq_n_u32(x64); 2];
            let t45: [uint32x4_t; 2] = [vaddq_u32(t43[0], t44[0]), vaddq_u32(t43[1], t44[1])];
            x56 = t45;
            let t46 = x64;
            x64 = (x64).wrapping_add(1u32);
        }
        let t47: [uint32x4_t; 2] = [vaddq_u32(x48[0], x56[0]), vaddq_u32(x48[1], x56[1])];
        let t48 = (arg1 as *mut u32).add((gid0) as usize);
        if all(&live) {
            vst1q_u32(t48.add(0), t47[0]);
            vst1q_u32(t48.add(4), t47[1]);
        } else {
            let mut t49w = 0u32;
            let t49 = core::hint::black_box(&raw mut t49w);
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t48.add(0), t49).write(vgetq_lane_u32::<0>(t47[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t48.add(1), t49).write(vgetq_lane_u32::<1>(t47[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t48.add(2), t49).write(vgetq_lane_u32::<2>(t47[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t48.add(3), t49).write(vgetq_lane_u32::<3>(t47[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t48.add(4), t49).write(vgetq_lane_u32::<0>(t47[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t48.add(5), t49).write(vgetq_lane_u32::<1>(t47[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t48.add(6), t49).write(vgetq_lane_u32::<2>(t47[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t48.add(7), t49).write(vgetq_lane_u32::<3>(t47[1]));
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
    let mut x72: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    if ((gid0) as u64 + 7) < ((a0) as u64) {
        let t50 = (arg0 as *const u32).add((gid0) as usize);
        let t51: [uint32x4_t; 2] = [vld1q_u32(t50.add(0)), vld1q_u32(t50.add(4))];
        x40 = t51;
        let t52 = [vdupq_n_u32(1u32); 2];
        let t53: [uint32x4_t; 2] = [vandq_u32(x40[0], t52[0]), vandq_u32(x40[1], t52[1])];
        let t54: [uint32x4_t; 2] = [vtstq_u32(t53[0], t53[0]), vtstq_u32(t53[1], t53[1])];
        {
            let t55 = [vdupq_n_u32(3u32); 2];
            let t56: [uint32x4_t; 2] = [vmulq_u32(x40[0], t55[0]), vmulq_u32(x40[1], t55[1])];
            let t57: [uint32x4_t; 2] = [vaddq_u32(t56[0], t52[0]), vaddq_u32(t56[1], t52[1])];
            x48 = [vbslq_u32(t54[0], t57[0], x48[0]), vbslq_u32(t54[1], t57[1], x48[1])];
        }
        let t58: [uint32x4_t; 2] = [vmvnq_u32(t54[0]), vmvnq_u32(t54[1])];
        {
            let t59: [uint32x4_t; 2] = [vshlq_u32(x40[0], vnegq_s32(vdupq_n_s32(((1u32) & 31) as i32))), vshlq_u32(x40[1], vnegq_s32(vdupq_n_s32(((1u32) & 31) as i32)))];
            x48 = [vbslq_u32(t58[0], t59[0], x48[0]), vbslq_u32(t58[1], t59[1], x48[1])];
        }
        let t60 = [vdupq_n_u32(4026531840u32); 2];
        let t61: [uint32x4_t; 2] = [vcltq_u32(x40[0], t60[0]), vcltq_u32(x40[1], t60[1])];
        if all(&t61) {
            let t62: [uint32x4_t; 2] = [vaddq_u32(x48[0], x40[0]), vaddq_u32(x48[1], x40[1])];
            x48 = t62;
        } else {
            // Lanes part: the chunk's lanes go both ways.
            let t63: [uint32x4_t; 2] = [vandq_u32(live[0], t61[0]), vandq_u32(live[1], t61[1])];
            {
                let t64: [uint32x4_t; 2] = [vaddq_u32(x48[0], x40[0]), vaddq_u32(x48[1], x40[1])];
                x48 = [vbslq_u32(t63[0], t64[0], x48[0]), vbslq_u32(t63[1], t64[1], x48[1])];
            }
        }
        let t65 = [vdupq_n_u32(0u32); 2];
        let t66: [uint32x4_t; 2] = [vceqq_u32(x40[0], t65[0]), vceqq_u32(x40[1], t65[1])];
        if !any(&t66) {
            let t67 = [vdupq_n_u32(7u32); 2];
            let t68: [uint32x4_t; 2] = [veorq_u32(x48[0], t67[0]), veorq_u32(x48[1], t67[1])];
            x48 = t68;
        } else {
            // Lanes part: the chunk's lanes go both ways.
            let t69: [uint32x4_t; 2] = [vandq_u32(live[0], t66[0]), vandq_u32(live[1], t66[1])];
            {
                let t70 = [vdupq_n_u32(57005u32); 2];
                x48 = [vbslq_u32(t69[0], t70[0], x48[0]), vbslq_u32(t69[1], t70[1], x48[1])];
            }
            let t71: [uint32x4_t; 2] = [vbicq_u32(live[0], t66[0]), vbicq_u32(live[1], t66[1])];
            {
                let t72 = [vdupq_n_u32(7u32); 2];
                let t73: [uint32x4_t; 2] = [veorq_u32(x48[0], t72[0]), veorq_u32(x48[1], t72[1])];
                x48 = [vbslq_u32(t71[0], t73[0], x48[0]), vbslq_u32(t71[1], t73[1], x48[1])];
            }
        }
        x56 = t65;
        x64 = 0u32;
        x72 = t65;
        loop {
            // Full rounds: every lane goes on.
            let t74 = [vdupq_n_u32(15u32); 2];
            let t75: [uint32x4_t; 2] = [vandq_u32(x40[0], t74[0]), vandq_u32(x40[1], t74[1])];
            let t76: [uint32x4_t; 2] = [vcltq_u32(x72[0], t75[0]), vcltq_u32(x72[1], t75[1])];
            if !all(&t76) { break; }
            let t77: [uint32x4_t; 2] = [vmulq_u32(x72[0], x48[0]), vmulq_u32(x72[1], x48[1])];
            let t78: [uint32x4_t; 2] = [vaddq_u32(x56[0], t77[0]), vaddq_u32(x56[1], t77[1])];
            x56 = t78;
            let t79: [uint32x4_t; 2] = [vaddq_u32(x72[0], t52[0]), vaddq_u32(x72[1], t52[1])];
            let t80 = x72;
            x72 = t79;
        }
        let mut t81 = live;
        loop {
            // Lanes part: each its own rounds.
            let t83 = [vdupq_n_u32(15u32); 2];
            let t84: [uint32x4_t; 2] = [vandq_u32(x40[0], t83[0]), vandq_u32(x40[1], t83[1])];
            let t85: [uint32x4_t; 2] = [vcltq_u32(x72[0], t84[0]), vcltq_u32(x72[1], t84[1])];
            t81 = [vandq_u32(t81[0], t85[0]), vandq_u32(t81[1], t85[1])];
            if !(any(&t81)) { break; }
            let t87: [uint32x4_t; 2] = [vmulq_u32(x72[0], x48[0]), vmulq_u32(x72[1], x48[1])];
            let t88: [uint32x4_t; 2] = [vaddq_u32(x56[0], t87[0]), vaddq_u32(x56[1], t87[1])];
            x56 = [vbslq_u32(t81[0], t88[0], x56[0]), vbslq_u32(t81[1], t88[1], x56[1])];
            let t89: [uint32x4_t; 2] = [vandq_u32(t81[0], live[0]), vandq_u32(t81[1], live[1])];
            let t90: [uint32x4_t; 2] = [vaddq_u32(x72[0], t52[0]), vaddq_u32(x72[1], t52[1])];
            let t91 = x72;
            x72 = t90;
        }
        loop {
            // Rounds of one condition: every lane goes on.
            if !(((x64) < (a1))) { break; }
            let t92 = [vdupq_n_u32(31u32); 2];
            let t93: [uint32x4_t; 2] = [vmulq_u32(x56[0], t92[0]), vmulq_u32(x56[1], t92[1])];
            let t94 = [vdupq_n_u32(x64); 2];
            let t95: [uint32x4_t; 2] = [vaddq_u32(t93[0], t94[0]), vaddq_u32(t93[1], t94[1])];
            x56 = t95;
            let t96 = x64;
            x64 = (x64).wrapping_add(1u32);
        }
        let t97: [uint32x4_t; 2] = [vaddq_u32(x48[0], x56[0]), vaddq_u32(x48[1], x56[1])];
        let t98 = (arg1 as *mut u32).add((gid0) as usize);
        vst1q_u32(t98.add(0), t97[0]);
        vst1q_u32(t98.add(4), t97[1]);
    } else if (((gid0) as u64 + 7) <= u32::MAX as u64 && ((gid0) as u64) >= ((a0) as u64)) {
        return;
    } else {
        // Lanes part: the chunk's lanes on both sides of the bound.
        let t99: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t100 = [vdupq_n_u32(a0); 2];
        let t101: [uint32x4_t; 2] = [vcgeq_u32(t99[0], t100[0]), vcgeq_u32(t99[1], t100[1])];
        let t102: [uint32x4_t; 2] = [vandq_u32(live[0], t101[0]), vandq_u32(live[1], t101[1])];
        if any(&t102) {
            live = [vbicq_u32(live[0], t102[0]), vbicq_u32(live[1], t102[1])];
        }
        if any(&live) {
            let mut t103: [uint32x4_t; 2];
            if all(&live) {
                let t104 = (arg0 as *const u32).add((gid0) as usize);
                t103 = [vld1q_u32(t104.add(0)), vld1q_u32(t104.add(4))];
            } else {
                let t105 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t99[0]), vandq_u32(live[1], t99[1]))));
                let t106: [uint32x4_t; 2] = [vbslq_u32(live[0], t99[0], t105), vbslq_u32(live[1], t99[1], t105)];
                let t107 = arg0 as *const u32;
                let mut t108: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                t108[0] = vld1q_lane_u32::<0>(t107.add(vgetq_lane_u32::<0>(t106[0]) as usize), t108[0]);
                t108[0] = vld1q_lane_u32::<1>(t107.add(vgetq_lane_u32::<1>(t106[0]) as usize), t108[0]);
                t108[0] = vld1q_lane_u32::<2>(t107.add(vgetq_lane_u32::<2>(t106[0]) as usize), t108[0]);
                t108[0] = vld1q_lane_u32::<3>(t107.add(vgetq_lane_u32::<3>(t106[0]) as usize), t108[0]);
                t108[1] = vld1q_lane_u32::<0>(t107.add(vgetq_lane_u32::<0>(t106[1]) as usize), t108[1]);
                t108[1] = vld1q_lane_u32::<1>(t107.add(vgetq_lane_u32::<1>(t106[1]) as usize), t108[1]);
                t108[1] = vld1q_lane_u32::<2>(t107.add(vgetq_lane_u32::<2>(t106[1]) as usize), t108[1]);
                t108[1] = vld1q_lane_u32::<3>(t107.add(vgetq_lane_u32::<3>(t106[1]) as usize), t108[1]);
                t103 = t108;
            }
            x40 = t103;
            let t109 = [vdupq_n_u32(1u32); 2];
            let t110: [uint32x4_t; 2] = [vandq_u32(x40[0], t109[0]), vandq_u32(x40[1], t109[1])];
            let t111: [uint32x4_t; 2] = [vtstq_u32(t110[0], t110[0]), vtstq_u32(t110[1], t110[1])];
            let t112: [uint32x4_t; 2] = [vandq_u32(live[0], t111[0]), vandq_u32(live[1], t111[1])];
            {
                let t113 = [vdupq_n_u32(3u32); 2];
                let t114: [uint32x4_t; 2] = [vmulq_u32(x40[0], t113[0]), vmulq_u32(x40[1], t113[1])];
                let t115: [uint32x4_t; 2] = [vaddq_u32(t114[0], t109[0]), vaddq_u32(t114[1], t109[1])];
                x48 = [vbslq_u32(t112[0], t115[0], x48[0]), vbslq_u32(t112[1], t115[1], x48[1])];
            }
            let t116: [uint32x4_t; 2] = [vbicq_u32(live[0], t111[0]), vbicq_u32(live[1], t111[1])];
            {
                let t117: [uint32x4_t; 2] = [vshlq_u32(x40[0], vnegq_s32(vdupq_n_s32(((1u32) & 31) as i32))), vshlq_u32(x40[1], vnegq_s32(vdupq_n_s32(((1u32) & 31) as i32)))];
                x48 = [vbslq_u32(t116[0], t117[0], x48[0]), vbslq_u32(t116[1], t117[1], x48[1])];
            }
            let t118 = [vdupq_n_u32(4026531840u32); 2];
            let t119: [uint32x4_t; 2] = [vcltq_u32(x40[0], t118[0]), vcltq_u32(x40[1], t118[1])];
            let t120: [uint32x4_t; 2] = [vandq_u32(live[0], t119[0]), vandq_u32(live[1], t119[1])];
            {
                let t121: [uint32x4_t; 2] = [vaddq_u32(x48[0], x40[0]), vaddq_u32(x48[1], x40[1])];
                x48 = [vbslq_u32(t120[0], t121[0], x48[0]), vbslq_u32(t120[1], t121[1], x48[1])];
            }
            let t122 = [vdupq_n_u32(0u32); 2];
            let t123: [uint32x4_t; 2] = [vceqq_u32(x40[0], t122[0]), vceqq_u32(x40[1], t122[1])];
            let t124: [uint32x4_t; 2] = [vandq_u32(live[0], t123[0]), vandq_u32(live[1], t123[1])];
            {
                let t125 = [vdupq_n_u32(57005u32); 2];
                x48 = [vbslq_u32(t124[0], t125[0], x48[0]), vbslq_u32(t124[1], t125[1], x48[1])];
            }
            let t126: [uint32x4_t; 2] = [vbicq_u32(live[0], t123[0]), vbicq_u32(live[1], t123[1])];
            {
                let t127 = [vdupq_n_u32(7u32); 2];
                let t128: [uint32x4_t; 2] = [veorq_u32(x48[0], t127[0]), veorq_u32(x48[1], t127[1])];
                x48 = [vbslq_u32(t126[0], t128[0], x48[0]), vbslq_u32(t126[1], t128[1], x48[1])];
            }
            x56 = t122;
            x64 = 0u32;
            x72 = t122;
            let mut t129 = live;
            loop {
                // Lanes part: each its own rounds.
                let t131 = [vdupq_n_u32(15u32); 2];
                let t132: [uint32x4_t; 2] = [vandq_u32(x40[0], t131[0]), vandq_u32(x40[1], t131[1])];
                let t133: [uint32x4_t; 2] = [vcltq_u32(x72[0], t132[0]), vcltq_u32(x72[1], t132[1])];
                t129 = [vandq_u32(t129[0], t133[0]), vandq_u32(t129[1], t133[1])];
                if !(any(&t129)) { break; }
                let t135: [uint32x4_t; 2] = [vmulq_u32(x72[0], x48[0]), vmulq_u32(x72[1], x48[1])];
                let t136: [uint32x4_t; 2] = [vaddq_u32(x56[0], t135[0]), vaddq_u32(x56[1], t135[1])];
                x56 = [vbslq_u32(t129[0], t136[0], x56[0]), vbslq_u32(t129[1], t136[1], x56[1])];
                let t137: [uint32x4_t; 2] = [vandq_u32(t129[0], live[0]), vandq_u32(t129[1], live[1])];
                let t138: [uint32x4_t; 2] = [vaddq_u32(x72[0], t109[0]), vaddq_u32(x72[1], t109[1])];
                let t139 = x72;
                x72 = t138;
            }
            loop {
                // Rounds of one condition: every lane goes on.
                if !(((x64) < (a1))) { break; }
                let t140 = [vdupq_n_u32(31u32); 2];
                let t141: [uint32x4_t; 2] = [vmulq_u32(x56[0], t140[0]), vmulq_u32(x56[1], t140[1])];
                let t142 = [vdupq_n_u32(x64); 2];
                let t143: [uint32x4_t; 2] = [vaddq_u32(t141[0], t142[0]), vaddq_u32(t141[1], t142[1])];
                x56 = t143;
                let t144 = x64;
                x64 = (x64).wrapping_add(1u32);
            }
            let t145: [uint32x4_t; 2] = [vaddq_u32(x48[0], x56[0]), vaddq_u32(x48[1], x56[1])];
            let t146 = (arg1 as *mut u32).add((gid0) as usize);
            if all(&live) {
                vst1q_u32(t146.add(0), t145[0]);
                vst1q_u32(t146.add(4), t145[1]);
            } else {
                let mut t147w = 0u32;
                let t147 = core::hint::black_box(&raw mut t147w);
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t146.add(0), t147).write(vgetq_lane_u32::<0>(t145[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t146.add(1), t147).write(vgetq_lane_u32::<1>(t145[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t146.add(2), t147).write(vgetq_lane_u32::<2>(t145[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t146.add(3), t147).write(vgetq_lane_u32::<3>(t145[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t146.add(4), t147).write(vgetq_lane_u32::<0>(t145[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t146.add(5), t147).write(vgetq_lane_u32::<1>(t145[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t146.add(6), t147).write(vgetq_lane_u32::<2>(t145[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t146.add(7), t147).write(vgetq_lane_u32::<3>(t145[1]));
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
    let mut t148: bool = !(arg3).is_null();
    if t148 {
        let mut t149: bool = ((l.group[0]) >= ((arg3.wrapping_add((((x40) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        if !t149 {
            t149 = ((l.group[1]) >= ((arg3.wrapping_add(((((x40).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        let mut t150: bool = t149;
        if !t150 {
            t150 = ((l.group[2]) >= ((arg3.wrapping_add(((((x40).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        t148 = t150;
    }
    if t148 {
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
        let t151: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
        let mut live = t151;
        let gid0 = l.group[0] * l.block[0] + lid0;
        'chunk: {
            b0_r0r1sg_p(l, lid0, live, arg0, arg1, x16);
        }
    }
}
