//! `parent_back` at 8 lanes (2 NEON registers), written by rerail-ukl-vec's Rust (`rs.rs`) from its vector tape: do
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

/// `parent_back_inner`.
#[inline(always)]
unsafe fn b0_r0r1r2r3r4sgi_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, arg2: *const u8, arg3: *const u8, arg4: *const u8, x5: *const u8, x7: u32) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x5 as *const u32).read_unaligned();
    let mut x8: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x9: bool = false;
    let mut live = k;
    let t1: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x7), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x7), vld1q_u32(LANE[1].as_ptr()))];
    let t2 = [vdupq_n_u32(31u32); 2];
    let t3: [uint32x4_t; 2] = [vandq_u32(t1[0], t2[0]), vandq_u32(t1[1], t2[1])];
    x8 = t3;
    let t4 = b1_r3sso4_p(l, lid0, live, arg3, (((a0).wrapping_add(1023u32)).checked_div(1024u32).unwrap_or(0)).wrapping_add(1u32), 0u32, arg4.wrapping_add((32u64).wrapping_mul(4u64) as usize));
    x9 = t4;
    if !(x9) {
        let t5 = b2_o4s_p(l, lid0, live, arg4.wrapping_add((96u64).wrapping_mul(4u64) as usize), 1u32);
    }
    let t6: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
    let t7: [uint32x4_t; 2] = [vsubq_u32(t6[0], t1[0]), vsubq_u32(t6[1], t1[1])];
    b3_r0r1r2r3ssllg_p(l, lid0, live, arg0, arg1, arg2, arg3, a0, x9, t7, x8);
}

/// `parent_back_inner`.
#[inline(always)]
unsafe fn b0_r0r1r2r3r4sgi_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, arg2: *const u8, arg3: *const u8, arg4: *const u8, x5: *const u8, x7: u32) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x5 as *const u32).read_unaligned();
    let mut x8: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x9: bool = false;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let t8: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x7), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x7), vld1q_u32(LANE[1].as_ptr()))];
    let t9 = [vdupq_n_u32(31u32); 2];
    let t10: [uint32x4_t; 2] = [vandq_u32(t8[0], t9[0]), vandq_u32(t8[1], t9[1])];
    x8 = t10;
    let t11 = b1_r3sso4_f(l, lid0, arg3, (((a0).wrapping_add(1023u32)).checked_div(1024u32).unwrap_or(0)).wrapping_add(1u32), 0u32, arg4.wrapping_add((32u64).wrapping_mul(4u64) as usize));
    x9 = t11;
    if !(x9) {
        let t12 = b2_o4s_f(l, lid0, arg4.wrapping_add((96u64).wrapping_mul(4u64) as usize), 1u32);
    }
    let t13: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
    let t14: [uint32x4_t; 2] = [vsubq_u32(t13[0], t8[0]), vsubq_u32(t13[1], t8[1])];
    b3_r0r1r2r3ssllg_f(l, lid0, arg0, arg1, arg2, arg3, a0, x9, t14, x8);
}

/// `parent_back_inner`.
#[inline(always)]
unsafe fn b0_r0r1r2r3r4sgu5_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, arg2: *const u8, arg3: *const u8, arg4: *const u8, x5: *const u8, x7: u32) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x5 as *const u32).read_unaligned();
    let mut x8: u32 = 0;
    let mut x9: bool = false;
    let mut live = k;
    x8 = (x7 & 31u32);
    let t15 = b1_r3sso4_p(l, lid0, live, arg3, (((a0).wrapping_add(1023u32)).checked_div(1024u32).unwrap_or(0)).wrapping_add(1u32), 0u32, arg4.wrapping_add((32u64).wrapping_mul(4u64) as usize));
    x9 = t15;
    if !(x9) {
        let t16 = b2_o4s_p(l, lid0, live, arg4.wrapping_add((96u64).wrapping_mul(4u64) as usize), 1u32);
    }
    let t17: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
    let t18: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x7), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x7), vld1q_u32(LANE[1].as_ptr()))];
    let t19: [uint32x4_t; 2] = [vsubq_u32(t17[0], t18[0]), vsubq_u32(t17[1], t18[1])];
    b3_r0r1r2r3sslig_p(l, lid0, live, arg0, arg1, arg2, arg3, a0, x9, t19, x8);
}

/// `parent_back_inner`.
#[inline(always)]
unsafe fn b0_r0r1r2r3r4sgu5_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, arg2: *const u8, arg3: *const u8, arg4: *const u8, x5: *const u8, x7: u32) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x5 as *const u32).read_unaligned();
    let mut x8: u32 = 0;
    let mut x9: bool = false;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    x8 = (x7 & 31u32);
    let t20 = b1_r3sso4_f(l, lid0, arg3, (((a0).wrapping_add(1023u32)).checked_div(1024u32).unwrap_or(0)).wrapping_add(1u32), 0u32, arg4.wrapping_add((32u64).wrapping_mul(4u64) as usize));
    x9 = t20;
    if !(x9) {
        let t21 = b2_o4s_f(l, lid0, arg4.wrapping_add((96u64).wrapping_mul(4u64) as usize), 1u32);
    }
    let t22: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
    let t23: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x7), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x7), vld1q_u32(LANE[1].as_ptr()))];
    let t24: [uint32x4_t; 2] = [vsubq_u32(t22[0], t23[0]), vsubq_u32(t22[1], t23[1])];
    b3_r0r1r2r3sslig_f(l, lid0, arg0, arg1, arg2, arg3, a0, x9, t24, x8);
}

/// `ukl_sg_fill`.
#[inline(always)]
unsafe fn b1_r3sso4_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg3: *const u8, x1: u32, x2: u32, arg4_x3: *const u8) -> bool {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u32 = 0;
    let mut x5: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x6: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x7: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x8: u32 = 0;
    let mut live = k;
    let mut res: bool = false;
    x4 = (((x1).wrapping_add(256u32)).wrapping_sub(1u32)).checked_div(256u32).unwrap_or(0);
    let mut t25 = live;
    let mut t26 = [vdupq_n_u32(0); 2];
    loop {
        // Lanes part: each its own rounds.
        let t27 = [vdupq_n_u32(if true { u32::MAX } else { 0 }); 2];
        t25 = [vandq_u32(vbicq_u32(t25[0], t26[0]), t27[0]), vandq_u32(vbicq_u32(t25[1], t26[1]), t27[1])];
        if !(any(&t25)) { break; }
        let t29 = b4_o4_p(l, lid0, t25, arg4_x3.wrapping_add((0u64).wrapping_mul(4u64) as usize));
        let t30: [uint32x4_t; 2];
        if ((t29) < (x4)) {
            let t31 = b2_o4s_p(l, lid0, t25, arg4_x3.wrapping_add((0u64).wrapping_mul(4u64) as usize), 1u32);
            t30 = t31;
        } else {
            let t32 = [vdupq_n_u32(x4); 2];
            t30 = t32;
        }
        x5 = t30;
        let t33 = [vdupq_n_u32(x4); 2];
        let t34: [uint32x4_t; 2] = [vcgeq_u32(x5[0], t33[0]), vcgeq_u32(x5[1], t33[1])];
        let t35: [uint32x4_t; 2] = [vandq_u32(t25[0], t34[0]), vandq_u32(t25[1], t34[1])];
        if any(&t35) {
            t26 = [vorrq_u32(t26[0], t35[0]), vorrq_u32(t26[1], t35[1])];
        }
        let t36: [uint32x4_t; 2] = [vbicq_u32(vandq_u32(t25[0], live[0]), t26[0]), vbicq_u32(vandq_u32(t25[1], live[1]), t26[1])];
        if any(&t36) {
            let t37 = [vdupq_n_u32(256u32); 2];
            let t38: [uint32x4_t; 2] = [vmulq_u32(x5[0], t37[0]), vmulq_u32(x5[1], t37[1])];
            let t39: [uint32x4_t; 2] = [vaddq_u32(t38[0], t37[0]), vaddq_u32(t38[1], t37[1])];
            let t40 = [vdupq_n_u32(x1); 2];
            let t41: [uint32x4_t; 2] = [vcltq_u32(t39[0], t40[0]), vcltq_u32(t39[1], t40[1])];
            let mut t42: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t43: [uint32x4_t; 2] = [vandq_u32(t36[0], t41[0]), vandq_u32(t36[1], t41[1])];
            let t44: [uint32x4_t; 2] = [vbicq_u32(t36[0], t41[0]), vbicq_u32(t36[1], t41[1])];
            {
                let t45: [uint32x4_t; 2] = [vmulq_u32(x5[0], t37[0]), vmulq_u32(x5[1], t37[1])];
                let t46: [uint32x4_t; 2] = [vaddq_u32(t45[0], t37[0]), vaddq_u32(t45[1], t37[1])];
                t42 = [vbslq_u32(t43[0], t46[0], t42[0]), vbslq_u32(t43[1], t46[1], t42[1])];
            }
            {
                let t47 = [vdupq_n_u32(x1); 2];
                t42 = [vbslq_u32(t44[0], t47[0], t42[0]), vbslq_u32(t44[1], t47[1], t42[1])];
            }
            x6 = t42;
            let t48: [uint32x4_t; 2] = [vmulq_u32(x5[0], t37[0]), vmulq_u32(x5[1], t37[1])];
            x7 = t48;
            let mut t50 = t36;
            loop {
                // Lanes part: each its own rounds.
                let t52: [uint32x4_t; 2] = [vcltq_u32(x7[0], x6[0]), vcltq_u32(x7[1], x6[1])];
                t50 = [vandq_u32(t50[0], t52[0]), vandq_u32(t50[1], t52[1])];
                if !(any(&t50)) { break; }
                let t54 = [vdupq_n_u32(x2); 2];
                let mut t55w = 0u32;
                let t55 = core::hint::black_box(&raw mut t55w);
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(t50[0]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<0>(x7[0]) as usize), t55).write(vgetq_lane_u32::<0>(t54[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(t50[0]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<1>(x7[0]) as usize), t55).write(vgetq_lane_u32::<1>(t54[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(t50[0]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<2>(x7[0]) as usize), t55).write(vgetq_lane_u32::<2>(t54[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(t50[0]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<3>(x7[0]) as usize), t55).write(vgetq_lane_u32::<3>(t54[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(t50[1]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<0>(x7[1]) as usize), t55).write(vgetq_lane_u32::<0>(t54[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(t50[1]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<1>(x7[1]) as usize), t55).write(vgetq_lane_u32::<1>(t54[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(t50[1]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<2>(x7[1]) as usize), t55).write(vgetq_lane_u32::<2>(t54[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(t50[1]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<3>(x7[1]) as usize), t55).write(vgetq_lane_u32::<3>(t54[1]));
                let t56: [uint32x4_t; 2] = [vandq_u32(t50[0], live[0]), vandq_u32(t50[1], live[1])];
                let t57 = x7;
                x7 = [vsubq_u32(x7[0], t56[0]), vsubq_u32(x7[1], t56[1])];
            }
            b5__p(l, lid0, t36);
            let t58 = b2_o4s_p(l, lid0, t36, arg4_x3.wrapping_add((32u64).wrapping_mul(4u64) as usize), 1u32);
        }
        let t59: [uint32x4_t; 2] = [vbicq_u32(vandq_u32(t25[0], live[0]), t26[0]), vbicq_u32(vandq_u32(t25[1], live[1]), t26[1])];
    }
    if any(&live) {
        x8 = 0u32;
        let mut t60 = live;
        loop {
            // Lanes part: each its own rounds.
            let t62 = [vdupq_n_u32(if ((x8) < (65536u32)) { u32::MAX } else { 0 }); 2];
            t60 = [vandq_u32(t60[0], t62[0]), vandq_u32(t60[1], t62[1])];
            if !(any(&t60)) { break; }
            let t64 = b4_o4_p(l, lid0, t60, arg4_x3.wrapping_add((32u64).wrapping_mul(4u64) as usize));
            if ((t64) >= (x4)) {
                res = ((1i32) != 0);
                live = [vbicq_u32(live[0], t60[0]), vbicq_u32(live[1], t60[1])];
            }
            let t65: [uint32x4_t; 2] = [vandq_u32(t60[0], live[0]), vandq_u32(t60[1], live[1])];
            let t66 = x8;
            x8 = (x8).wrapping_add(1u32);
            t60 = [vandq_u32(t60[0], live[0]), vandq_u32(t60[1], live[1])];
        }
        if any(&live) {
            res = ((0i32) != 0);
            live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
        }
    }
    res
}

/// `ukl_sg_fill`.
#[inline(always)]
unsafe fn b1_r3sso4_f(l: &Launch, lid0: u32, arg3: *const u8, x1: u32, x2: u32, arg4_x3: *const u8) -> bool {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u32 = 0;
    let mut x5: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x6: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x7: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x8: u32 = 0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: bool = false;
    x4 = (((x1).wrapping_add(256u32)).wrapping_sub(1u32)).checked_div(256u32).unwrap_or(0);
    let mut t67 = live;
    let mut t68 = [vdupq_n_u32(0); 2];
    loop {
        // Lanes part: each its own rounds.
        let t69 = [vdupq_n_u32(if true { u32::MAX } else { 0 }); 2];
        t67 = [vandq_u32(vbicq_u32(t67[0], t68[0]), t69[0]), vandq_u32(vbicq_u32(t67[1], t68[1]), t69[1])];
        if !(any(&t67)) { break; }
        let t71 = b4_o4_p(l, lid0, t67, arg4_x3.wrapping_add((0u64).wrapping_mul(4u64) as usize));
        let t72: [uint32x4_t; 2];
        if ((t71) < (x4)) {
            let t73 = b2_o4s_p(l, lid0, t67, arg4_x3.wrapping_add((0u64).wrapping_mul(4u64) as usize), 1u32);
            t72 = t73;
        } else {
            let t74 = [vdupq_n_u32(x4); 2];
            t72 = t74;
        }
        x5 = t72;
        let t75 = [vdupq_n_u32(x4); 2];
        let t76: [uint32x4_t; 2] = [vcgeq_u32(x5[0], t75[0]), vcgeq_u32(x5[1], t75[1])];
        let t77: [uint32x4_t; 2] = [vandq_u32(t67[0], t76[0]), vandq_u32(t67[1], t76[1])];
        if any(&t77) {
            t68 = [vorrq_u32(t68[0], t77[0]), vorrq_u32(t68[1], t77[1])];
        }
        let t78: [uint32x4_t; 2] = [vbicq_u32(vandq_u32(t67[0], live[0]), t68[0]), vbicq_u32(vandq_u32(t67[1], live[1]), t68[1])];
        if any(&t78) {
            let t79 = [vdupq_n_u32(256u32); 2];
            let t80: [uint32x4_t; 2] = [vmulq_u32(x5[0], t79[0]), vmulq_u32(x5[1], t79[1])];
            let t81: [uint32x4_t; 2] = [vaddq_u32(t80[0], t79[0]), vaddq_u32(t80[1], t79[1])];
            let t82 = [vdupq_n_u32(x1); 2];
            let t83: [uint32x4_t; 2] = [vcltq_u32(t81[0], t82[0]), vcltq_u32(t81[1], t82[1])];
            let mut t84: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t85: [uint32x4_t; 2] = [vandq_u32(t78[0], t83[0]), vandq_u32(t78[1], t83[1])];
            let t86: [uint32x4_t; 2] = [vbicq_u32(t78[0], t83[0]), vbicq_u32(t78[1], t83[1])];
            {
                let t87: [uint32x4_t; 2] = [vmulq_u32(x5[0], t79[0]), vmulq_u32(x5[1], t79[1])];
                let t88: [uint32x4_t; 2] = [vaddq_u32(t87[0], t79[0]), vaddq_u32(t87[1], t79[1])];
                t84 = [vbslq_u32(t85[0], t88[0], t84[0]), vbslq_u32(t85[1], t88[1], t84[1])];
            }
            {
                let t89 = [vdupq_n_u32(x1); 2];
                t84 = [vbslq_u32(t86[0], t89[0], t84[0]), vbslq_u32(t86[1], t89[1], t84[1])];
            }
            x6 = t84;
            let t90: [uint32x4_t; 2] = [vmulq_u32(x5[0], t79[0]), vmulq_u32(x5[1], t79[1])];
            x7 = t90;
            let mut t92 = t78;
            loop {
                // Lanes part: each its own rounds.
                let t94: [uint32x4_t; 2] = [vcltq_u32(x7[0], x6[0]), vcltq_u32(x7[1], x6[1])];
                t92 = [vandq_u32(t92[0], t94[0]), vandq_u32(t92[1], t94[1])];
                if !(any(&t92)) { break; }
                let t96 = [vdupq_n_u32(x2); 2];
                let mut t97w = 0u32;
                let t97 = core::hint::black_box(&raw mut t97w);
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(t92[0]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<0>(x7[0]) as usize), t97).write(vgetq_lane_u32::<0>(t96[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(t92[0]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<1>(x7[0]) as usize), t97).write(vgetq_lane_u32::<1>(t96[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(t92[0]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<2>(x7[0]) as usize), t97).write(vgetq_lane_u32::<2>(t96[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(t92[0]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<3>(x7[0]) as usize), t97).write(vgetq_lane_u32::<3>(t96[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(t92[1]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<0>(x7[1]) as usize), t97).write(vgetq_lane_u32::<0>(t96[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(t92[1]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<1>(x7[1]) as usize), t97).write(vgetq_lane_u32::<1>(t96[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(t92[1]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<2>(x7[1]) as usize), t97).write(vgetq_lane_u32::<2>(t96[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(t92[1]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<3>(x7[1]) as usize), t97).write(vgetq_lane_u32::<3>(t96[1]));
                let t98: [uint32x4_t; 2] = [vandq_u32(t92[0], live[0]), vandq_u32(t92[1], live[1])];
                let t99 = x7;
                x7 = [vsubq_u32(x7[0], t98[0]), vsubq_u32(x7[1], t98[1])];
            }
            b5__p(l, lid0, t78);
            let t100 = b2_o4s_p(l, lid0, t78, arg4_x3.wrapping_add((32u64).wrapping_mul(4u64) as usize), 1u32);
        }
        let t101: [uint32x4_t; 2] = [vbicq_u32(vandq_u32(t67[0], live[0]), t68[0]), vbicq_u32(vandq_u32(t67[1], live[1]), t68[1])];
    }
    if any(&live) {
        x8 = 0u32;
        let mut t102 = live;
        loop {
            // Lanes part: each its own rounds.
            let t104 = [vdupq_n_u32(if ((x8) < (65536u32)) { u32::MAX } else { 0 }); 2];
            t102 = [vandq_u32(t102[0], t104[0]), vandq_u32(t102[1], t104[1])];
            if !(any(&t102)) { break; }
            let t106 = b4_o4_p(l, lid0, t102, arg4_x3.wrapping_add((32u64).wrapping_mul(4u64) as usize));
            if ((t106) >= (x4)) {
                res = ((1i32) != 0);
                live = [vbicq_u32(live[0], t102[0]), vbicq_u32(live[1], t102[1])];
            }
            let t107: [uint32x4_t; 2] = [vandq_u32(t102[0], live[0]), vandq_u32(t102[1], live[1])];
            let t108 = x8;
            x8 = (x8).wrapping_add(1u32);
            t102 = [vandq_u32(t102[0], live[0]), vandq_u32(t102[1], live[1])];
        }
        if any(&live) {
            res = ((0i32) != 0);
            live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
        }
    }
    res
}

/// `ukl_atomic_add`.
#[inline(always)]
unsafe fn b2_o4s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg4_x0: *const u8, x1: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut t109 = [0u32; 8];
    for r in 0..2 { vst1q_u32(t109.as_mut_ptr().add(4 * r), live[r]); }
    let mut t110 = [0u32; 8];
    let mut t111 = 0u32;
    for j in 0..8 { t110[j] = t111; t111 += t109[j] & 1; }
    let t112: [uint32x4_t; 2] = [vld1q_u32(t110.as_ptr().add(0)), vld1q_u32(t110.as_ptr().add(4))];
    let t113 = AtomicU32::from_ptr(arg4_x0 as *mut u32).fetch_add((x1).wrapping_mul(t111), Relaxed);
    let t114: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(t113), t112[0], vdupq_n_u32(x1)), vmlaq_u32(vdupq_n_u32(t113), t112[1], vdupq_n_u32(x1))];
    res = t114;
    return res;
}

/// `ukl_atomic_add`.
#[inline(always)]
unsafe fn b2_o4s_f(l: &Launch, lid0: u32, arg4_x0: *const u8, x1: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t115: [uint32x4_t; 2] = [vld1q_u32(LANE[0].as_ptr()), vld1q_u32(LANE[1].as_ptr())];
    let t116 = AtomicU32::from_ptr(arg4_x0 as *mut u32).fetch_add((x1).wrapping_mul(8u32), Relaxed);
    let t117: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(t116), t115[0], vdupq_n_u32(x1)), vmlaq_u32(vdupq_n_u32(t116), t115[1], vdupq_n_u32(x1))];
    res = t117;
    return res;
}

/// `parent_back_node`.
#[inline(always)]
unsafe fn b3_r0r1r2r3sslig_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, arg2: *const u8, arg3: *const u8, x4: u32, x5: bool, x6: [uint32x4_t; 2], x7: u32) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x9: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x10: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x11: u32 = 0;
    let mut x12: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x13: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x14: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    if !(x5) {
        let t118: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t119 = [vdupq_n_u32(x4); 2];
        let t120: [uint32x4_t; 2] = [vcltq_u32(t118[0], t119[0]), vcltq_u32(t118[1], t119[1])];
        let t121: [uint32x4_t; 2] = [vandq_u32(live[0], t120[0]), vandq_u32(live[1], t120[1])];
        if any(&t121) {
            let t122 = b7_r0r1g_p(l, lid0, t121, arg0, arg1);
            b6_ag4r2l_p(l, lid0, t121, arg2, t122);
        }
        return;
    }
    if any(&live) {
        let t123 = [vdupq_n_u32(4294967295u32); 2];
        x9 = t123;
        let t124: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t125 = [vdupq_n_u32(x4); 2];
        let t126: [uint32x4_t; 2] = [vcltq_u32(t124[0], t125[0]), vcltq_u32(t124[1], t125[1])];
        let t127: [uint32x4_t; 2] = [vandq_u32(live[0], t126[0]), vandq_u32(live[1], t126[1])];
        if any(&t127) {
            let t128 = b8_r0r1g_p(l, lid0, t127, arg0, arg1);
            x10 = t128;
            let t129: u64 = (1u64).wrapping_shl((32u64) as u32);
            let t130 = [vdupq_n_u32((t129) as u32); 2];
            let t131 = [vdupq_n_u32(((t129) >> 32) as u32); 2];
            let t132: [uint32x4_t; 4] = [vandq_u32(x10[0], t130[0]), vandq_u32(x10[1], t130[1]), vandq_u32(x10[2], t131[0]), vandq_u32(x10[3], t131[1])];
            let t133: u64 = 0u64;
            let t134 = [vdupq_n_u32((t133) as u32); 2];
            let t135: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t132[0], t134[0]), vceqq_u32(t132[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t132[1], t134[1]), vceqq_u32(t132[3], vdupq_n_u32(0)))];
            let t136: [uint32x4_t; 2] = [vmvnq_u32(t135[0]), vmvnq_u32(t135[1])];
            let mut t137: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t138: [uint32x4_t; 2] = [vandq_u32(t127[0], t136[0]), vandq_u32(t127[1], t136[1])];
            let t139: [uint32x4_t; 2] = [vbicq_u32(t127[0], t136[0]), vbicq_u32(t127[1], t136[1])];
            {
                let t140: [uint32x4_t; 2] = [x10[0], x10[1]];
                let t141 = [vdupq_n_u32(2147483648u32); 2];
                let t142: [uint32x4_t; 2] = [vorrq_u32(t141[0], t140[0]), vorrq_u32(t141[1], t140[1])];
                t137 = [vbslq_u32(t138[0], t142[0], t137[0]), vbslq_u32(t138[1], t142[1], t137[1])];
            }
            {
                let t143: [uint32x4_t; 2] = [x10[0], x10[1]];
                t137 = [vbslq_u32(t139[0], t143[0], t137[0]), vbslq_u32(t139[1], t143[1], t137[1])];
            }
            x9 = [vbslq_u32(t127[0], t137[0], x9[0]), vbslq_u32(t127[1], t137[1], x9[1])];
            b6_ag4r2l_p(l, lid0, t127, arg2, x9);
        }
        b9_r3gi_p(l, lid0, live, arg3, x7);
        let t144 = [vdupq_n_u32(x4); 2];
        let t145: [uint32x4_t; 2] = [vcgeq_u32(t124[0], t144[0]), vcgeq_u32(t124[1], t144[1])];
        let t146: [uint32x4_t; 2] = [vandq_u32(live[0], t145[0]), vandq_u32(live[1], t145[1])];
        {
            live = [vbicq_u32(live[0], t146[0]), vbicq_u32(live[1], t146[1])];
        }
        if any(&live) {
            x11 = 0u32;
            let mut t156 = live;
            loop {
                // Lanes part: each its own rounds.
                let t159 = [vdupq_n_u32(2147483648u32); 2];
                let t160: [uint32x4_t; 2] = [vandq_u32(x9[0], t159[0]), vandq_u32(x9[1], t159[1])];
                let t161 = [vdupq_n_u32(0u32); 2];
                let t162: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t160[0], t161[0])), vmvnq_u32(vceqq_u32(t160[1], t161[1]))];
                let t163 = [vdupq_n_u32(4294967294u32); 2];
                let t164: [uint32x4_t; 2] = [vcltq_u32(x9[0], t163[0]), vcltq_u32(x9[1], t163[1])];
                let t165: [uint32x4_t; 2] = [vandq_u32(t162[0], t164[0]), vandq_u32(t162[1], t164[1])];
                let t166: [uint32x4_t; 2] = [vandq_u32(vdupq_n_u32(if ((x11) < (7u32)) { u32::MAX } else { 0 }), t165[0]), vandq_u32(vdupq_n_u32(if ((x11) < (7u32)) { u32::MAX } else { 0 }), t165[1])];
                t156 = [vandq_u32(t156[0], t166[0]), vandq_u32(t156[1], t166[1])];
                if !(any(&t156)) { break; }
                let t168 = [vdupq_n_u32(2147483647u32); 2];
                let t169: [uint32x4_t; 2] = [vandq_u32(x9[0], t168[0]), vandq_u32(x9[1], t168[1])];
                x12 = t169;
                x13 = t163;
                let t170 = b10_r3ll_p(l, lid0, t156, arg3, x12, x6);
                let t171: [uint32x4_t; 2] = [vandq_u32(t156[0], t170[0]), vandq_u32(t156[1], t170[1])];
                if any(&t171) {
                    let t172 = b11_al4r2_p(l, lid0, t171, arg2, x12);
                    x13 = [vbslq_u32(t171[0], t172[0], x13[0]), vbslq_u32(t171[1], t172[1], x13[1])];
                }
                let t173: [uint32x4_t; 2] = [vceqq_u32(x13[0], t163[0]), vceqq_u32(x13[1], t163[1])];
                let t174: [uint32x4_t; 2] = [vandq_u32(t156[0], t173[0]), vandq_u32(t156[1], t173[1])];
                if any(&t174) {
                    let t175 = b8_r0r1l_p(l, lid0, t174, arg0, arg1, x12);
                    x14 = t175;
                    let t176: u64 = (1u64).wrapping_shl((32u64) as u32);
                    let t177 = [vdupq_n_u32((t176) as u32); 2];
                    let t178 = [vdupq_n_u32(((t176) >> 32) as u32); 2];
                    let t179: [uint32x4_t; 4] = [vandq_u32(x14[0], t177[0]), vandq_u32(x14[1], t177[1]), vandq_u32(x14[2], t178[0]), vandq_u32(x14[3], t178[1])];
                    let t180: u64 = 0u64;
                    let t181 = [vdupq_n_u32((t180) as u32); 2];
                    let t182: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t179[0], t181[0]), vceqq_u32(t179[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t179[1], t181[1]), vceqq_u32(t179[3], vdupq_n_u32(0)))];
                    let t183: [uint32x4_t; 2] = [vmvnq_u32(t182[0]), vmvnq_u32(t182[1])];
                    let mut t184: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                    let t185: [uint32x4_t; 2] = [vandq_u32(t174[0], t183[0]), vandq_u32(t174[1], t183[1])];
                    let t186: [uint32x4_t; 2] = [vbicq_u32(t174[0], t183[0]), vbicq_u32(t174[1], t183[1])];
                    {
                        let t187: [uint32x4_t; 2] = [x14[0], x14[1]];
                        let t188: [uint32x4_t; 2] = [vorrq_u32(t159[0], t187[0]), vorrq_u32(t159[1], t187[1])];
                        t184 = [vbslq_u32(t185[0], t188[0], t184[0]), vbslq_u32(t185[1], t188[1], t184[1])];
                    }
                    {
                        let t189: [uint32x4_t; 2] = [x14[0], x14[1]];
                        t184 = [vbslq_u32(t186[0], t189[0], t184[0]), vbslq_u32(t186[1], t189[1], t184[1])];
                    }
                    x13 = [vbslq_u32(t174[0], t184[0], x13[0]), vbslq_u32(t174[1], t184[1], x13[1])];
                }
                x9 = [vbslq_u32(t156[0], x13[0], x9[0]), vbslq_u32(t156[1], x13[1], x9[1])];
                let t190: [uint32x4_t; 2] = [vandq_u32(t156[0], live[0]), vandq_u32(t156[1], live[1])];
                let t191 = x11;
                x11 = (x11).wrapping_add(1u32);
            }
            let t192 = [vdupq_n_u32(2147483648u32); 2];
            let t193: [uint32x4_t; 2] = [vandq_u32(x9[0], t192[0]), vandq_u32(x9[1], t192[1])];
            let t194 = [vdupq_n_u32(0u32); 2];
            let t195: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t193[0], t194[0])), vmvnq_u32(vceqq_u32(t193[1], t194[1]))];
            let t196 = [vdupq_n_u32(4294967294u32); 2];
            let t197: [uint32x4_t; 2] = [vcltq_u32(x9[0], t196[0]), vcltq_u32(x9[1], t196[1])];
            let t198: [uint32x4_t; 2] = [vandq_u32(t195[0], t197[0]), vandq_u32(t195[1], t197[1])];
            let t199: [uint32x4_t; 2] = [vandq_u32(live[0], t198[0]), vandq_u32(live[1], t198[1])];
            if any(&t199) {
                let t200 = [vdupq_n_u32(2147483647u32); 2];
                let t201: [uint32x4_t; 2] = [vandq_u32(x9[0], t200[0]), vandq_u32(x9[1], t200[1])];
                let t202 = b7_r0r1l_p(l, lid0, t199, arg0, arg1, t201);
                x9 = [vbslq_u32(t199[0], t202[0], x9[0]), vbslq_u32(t199[1], t202[1], x9[1])];
            }
            b6_ag4r2l_p(l, lid0, live, arg2, x9);
        }
    }
}

/// `parent_back_node`.
#[inline(always)]
unsafe fn b3_r0r1r2r3sslig_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, arg2: *const u8, arg3: *const u8, x4: u32, x5: bool, x6: [uint32x4_t; 2], x7: u32) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x9: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x10: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x11: u32 = 0;
    let mut x12: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x13: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x14: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    if !(x5) {
        if (((gid0) as u64 + 7) <= u32::MAX as u64 && ((gid0) as u64) >= ((x4) as u64)) {
        } else if ((gid0) as u64 + 7) < ((x4) as u64) {
            let t203 = b7_r0r1g_f(l, lid0, arg0, arg1);
            b6_ag4r2l_f(l, lid0, arg2, t203);
        } else {
            // Lanes part: the chunk's lanes on both sides of the bound.
            let t204: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
            let t205 = [vdupq_n_u32(x4); 2];
            let t206: [uint32x4_t; 2] = [vcltq_u32(t204[0], t205[0]), vcltq_u32(t204[1], t205[1])];
            let t207: [uint32x4_t; 2] = [vandq_u32(live[0], t206[0]), vandq_u32(live[1], t206[1])];
            if any(&t207) {
                let t208 = b7_r0r1g_p(l, lid0, t207, arg0, arg1);
                b6_ag4r2l_p(l, lid0, t207, arg2, t208);
            }
        }
        return;
    }
    if any(&live) {
        let t209 = [vdupq_n_u32(4294967295u32); 2];
        x9 = t209;
        let t210: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t211 = [vdupq_n_u32(x4); 2];
        let t212: [uint32x4_t; 2] = [vcltq_u32(t210[0], t211[0]), vcltq_u32(t210[1], t211[1])];
        let t213: [uint32x4_t; 2] = [vandq_u32(live[0], t212[0]), vandq_u32(live[1], t212[1])];
        if any(&t213) {
            let t214 = b8_r0r1g_p(l, lid0, t213, arg0, arg1);
            x10 = t214;
            let t215: u64 = (1u64).wrapping_shl((32u64) as u32);
            let t216 = [vdupq_n_u32((t215) as u32); 2];
            let t217 = [vdupq_n_u32(((t215) >> 32) as u32); 2];
            let t218: [uint32x4_t; 4] = [vandq_u32(x10[0], t216[0]), vandq_u32(x10[1], t216[1]), vandq_u32(x10[2], t217[0]), vandq_u32(x10[3], t217[1])];
            let t219: u64 = 0u64;
            let t220 = [vdupq_n_u32((t219) as u32); 2];
            let t221: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t218[0], t220[0]), vceqq_u32(t218[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t218[1], t220[1]), vceqq_u32(t218[3], vdupq_n_u32(0)))];
            let t222: [uint32x4_t; 2] = [vmvnq_u32(t221[0]), vmvnq_u32(t221[1])];
            let mut t223: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t224: [uint32x4_t; 2] = [vandq_u32(t213[0], t222[0]), vandq_u32(t213[1], t222[1])];
            let t225: [uint32x4_t; 2] = [vbicq_u32(t213[0], t222[0]), vbicq_u32(t213[1], t222[1])];
            {
                let t226: [uint32x4_t; 2] = [x10[0], x10[1]];
                let t227 = [vdupq_n_u32(2147483648u32); 2];
                let t228: [uint32x4_t; 2] = [vorrq_u32(t227[0], t226[0]), vorrq_u32(t227[1], t226[1])];
                t223 = [vbslq_u32(t224[0], t228[0], t223[0]), vbslq_u32(t224[1], t228[1], t223[1])];
            }
            {
                let t229: [uint32x4_t; 2] = [x10[0], x10[1]];
                t223 = [vbslq_u32(t225[0], t229[0], t223[0]), vbslq_u32(t225[1], t229[1], t223[1])];
            }
            x9 = [vbslq_u32(t213[0], t223[0], x9[0]), vbslq_u32(t213[1], t223[1], x9[1])];
            b6_ag4r2l_p(l, lid0, t213, arg2, x9);
        }
        b9_r3gi_p(l, lid0, live, arg3, x7);
        let t230 = [vdupq_n_u32(x4); 2];
        let t231: [uint32x4_t; 2] = [vcgeq_u32(t210[0], t230[0]), vcgeq_u32(t210[1], t230[1])];
        let t232: [uint32x4_t; 2] = [vandq_u32(live[0], t231[0]), vandq_u32(live[1], t231[1])];
        {
            live = [vbicq_u32(live[0], t232[0]), vbicq_u32(live[1], t232[1])];
        }
        if any(&live) {
            x11 = 0u32;
            let mut t242 = live;
            loop {
                // Lanes part: each its own rounds.
                let t245 = [vdupq_n_u32(2147483648u32); 2];
                let t246: [uint32x4_t; 2] = [vandq_u32(x9[0], t245[0]), vandq_u32(x9[1], t245[1])];
                let t247 = [vdupq_n_u32(0u32); 2];
                let t248: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t246[0], t247[0])), vmvnq_u32(vceqq_u32(t246[1], t247[1]))];
                let t249 = [vdupq_n_u32(4294967294u32); 2];
                let t250: [uint32x4_t; 2] = [vcltq_u32(x9[0], t249[0]), vcltq_u32(x9[1], t249[1])];
                let t251: [uint32x4_t; 2] = [vandq_u32(t248[0], t250[0]), vandq_u32(t248[1], t250[1])];
                let t252: [uint32x4_t; 2] = [vandq_u32(vdupq_n_u32(if ((x11) < (7u32)) { u32::MAX } else { 0 }), t251[0]), vandq_u32(vdupq_n_u32(if ((x11) < (7u32)) { u32::MAX } else { 0 }), t251[1])];
                t242 = [vandq_u32(t242[0], t252[0]), vandq_u32(t242[1], t252[1])];
                if !(any(&t242)) { break; }
                let t254 = [vdupq_n_u32(2147483647u32); 2];
                let t255: [uint32x4_t; 2] = [vandq_u32(x9[0], t254[0]), vandq_u32(x9[1], t254[1])];
                x12 = t255;
                x13 = t249;
                let t256 = b10_r3ll_p(l, lid0, t242, arg3, x12, x6);
                let t257: [uint32x4_t; 2] = [vandq_u32(t242[0], t256[0]), vandq_u32(t242[1], t256[1])];
                if any(&t257) {
                    let t258 = b11_al4r2_p(l, lid0, t257, arg2, x12);
                    x13 = [vbslq_u32(t257[0], t258[0], x13[0]), vbslq_u32(t257[1], t258[1], x13[1])];
                }
                let t259: [uint32x4_t; 2] = [vceqq_u32(x13[0], t249[0]), vceqq_u32(x13[1], t249[1])];
                let t260: [uint32x4_t; 2] = [vandq_u32(t242[0], t259[0]), vandq_u32(t242[1], t259[1])];
                if any(&t260) {
                    let t261 = b8_r0r1l_p(l, lid0, t260, arg0, arg1, x12);
                    x14 = t261;
                    let t262: u64 = (1u64).wrapping_shl((32u64) as u32);
                    let t263 = [vdupq_n_u32((t262) as u32); 2];
                    let t264 = [vdupq_n_u32(((t262) >> 32) as u32); 2];
                    let t265: [uint32x4_t; 4] = [vandq_u32(x14[0], t263[0]), vandq_u32(x14[1], t263[1]), vandq_u32(x14[2], t264[0]), vandq_u32(x14[3], t264[1])];
                    let t266: u64 = 0u64;
                    let t267 = [vdupq_n_u32((t266) as u32); 2];
                    let t268: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t265[0], t267[0]), vceqq_u32(t265[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t265[1], t267[1]), vceqq_u32(t265[3], vdupq_n_u32(0)))];
                    let t269: [uint32x4_t; 2] = [vmvnq_u32(t268[0]), vmvnq_u32(t268[1])];
                    let mut t270: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                    let t271: [uint32x4_t; 2] = [vandq_u32(t260[0], t269[0]), vandq_u32(t260[1], t269[1])];
                    let t272: [uint32x4_t; 2] = [vbicq_u32(t260[0], t269[0]), vbicq_u32(t260[1], t269[1])];
                    {
                        let t273: [uint32x4_t; 2] = [x14[0], x14[1]];
                        let t274: [uint32x4_t; 2] = [vorrq_u32(t245[0], t273[0]), vorrq_u32(t245[1], t273[1])];
                        t270 = [vbslq_u32(t271[0], t274[0], t270[0]), vbslq_u32(t271[1], t274[1], t270[1])];
                    }
                    {
                        let t275: [uint32x4_t; 2] = [x14[0], x14[1]];
                        t270 = [vbslq_u32(t272[0], t275[0], t270[0]), vbslq_u32(t272[1], t275[1], t270[1])];
                    }
                    x13 = [vbslq_u32(t260[0], t270[0], x13[0]), vbslq_u32(t260[1], t270[1], x13[1])];
                }
                x9 = [vbslq_u32(t242[0], x13[0], x9[0]), vbslq_u32(t242[1], x13[1], x9[1])];
                let t276: [uint32x4_t; 2] = [vandq_u32(t242[0], live[0]), vandq_u32(t242[1], live[1])];
                let t277 = x11;
                x11 = (x11).wrapping_add(1u32);
            }
            let t278 = [vdupq_n_u32(2147483648u32); 2];
            let t279: [uint32x4_t; 2] = [vandq_u32(x9[0], t278[0]), vandq_u32(x9[1], t278[1])];
            let t280 = [vdupq_n_u32(0u32); 2];
            let t281: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t279[0], t280[0])), vmvnq_u32(vceqq_u32(t279[1], t280[1]))];
            let t282 = [vdupq_n_u32(4294967294u32); 2];
            let t283: [uint32x4_t; 2] = [vcltq_u32(x9[0], t282[0]), vcltq_u32(x9[1], t282[1])];
            let t284: [uint32x4_t; 2] = [vandq_u32(t281[0], t283[0]), vandq_u32(t281[1], t283[1])];
            let t285: [uint32x4_t; 2] = [vandq_u32(live[0], t284[0]), vandq_u32(live[1], t284[1])];
            if any(&t285) {
                let t286 = [vdupq_n_u32(2147483647u32); 2];
                let t287: [uint32x4_t; 2] = [vandq_u32(x9[0], t286[0]), vandq_u32(x9[1], t286[1])];
                let t288 = b7_r0r1l_p(l, lid0, t285, arg0, arg1, t287);
                x9 = [vbslq_u32(t285[0], t288[0], x9[0]), vbslq_u32(t285[1], t288[1], x9[1])];
            }
            b6_ag4r2l_p(l, lid0, live, arg2, x9);
        }
    }
}

/// `parent_back_node`.
#[inline(always)]
unsafe fn b3_r0r1r2r3ssllg_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, arg2: *const u8, arg3: *const u8, x4: u32, x5: bool, x6: [uint32x4_t; 2], x7: [uint32x4_t; 2]) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x9: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x10: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x11: u32 = 0;
    let mut x12: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x13: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x14: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    if !(x5) {
        let t289: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t290 = [vdupq_n_u32(x4); 2];
        let t291: [uint32x4_t; 2] = [vcltq_u32(t289[0], t290[0]), vcltq_u32(t289[1], t290[1])];
        let t292: [uint32x4_t; 2] = [vandq_u32(live[0], t291[0]), vandq_u32(live[1], t291[1])];
        if any(&t292) {
            let t293 = b7_r0r1g_p(l, lid0, t292, arg0, arg1);
            b6_ag4r2l_p(l, lid0, t292, arg2, t293);
        }
        return;
    }
    if any(&live) {
        let t294 = [vdupq_n_u32(4294967295u32); 2];
        x9 = t294;
        let t295: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t296 = [vdupq_n_u32(x4); 2];
        let t297: [uint32x4_t; 2] = [vcltq_u32(t295[0], t296[0]), vcltq_u32(t295[1], t296[1])];
        let t298: [uint32x4_t; 2] = [vandq_u32(live[0], t297[0]), vandq_u32(live[1], t297[1])];
        if any(&t298) {
            let t299 = b8_r0r1g_p(l, lid0, t298, arg0, arg1);
            x10 = t299;
            let t300: u64 = (1u64).wrapping_shl((32u64) as u32);
            let t301 = [vdupq_n_u32((t300) as u32); 2];
            let t302 = [vdupq_n_u32(((t300) >> 32) as u32); 2];
            let t303: [uint32x4_t; 4] = [vandq_u32(x10[0], t301[0]), vandq_u32(x10[1], t301[1]), vandq_u32(x10[2], t302[0]), vandq_u32(x10[3], t302[1])];
            let t304: u64 = 0u64;
            let t305 = [vdupq_n_u32((t304) as u32); 2];
            let t306: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t303[0], t305[0]), vceqq_u32(t303[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t303[1], t305[1]), vceqq_u32(t303[3], vdupq_n_u32(0)))];
            let t307: [uint32x4_t; 2] = [vmvnq_u32(t306[0]), vmvnq_u32(t306[1])];
            let mut t308: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t309: [uint32x4_t; 2] = [vandq_u32(t298[0], t307[0]), vandq_u32(t298[1], t307[1])];
            let t310: [uint32x4_t; 2] = [vbicq_u32(t298[0], t307[0]), vbicq_u32(t298[1], t307[1])];
            {
                let t311: [uint32x4_t; 2] = [x10[0], x10[1]];
                let t312 = [vdupq_n_u32(2147483648u32); 2];
                let t313: [uint32x4_t; 2] = [vorrq_u32(t312[0], t311[0]), vorrq_u32(t312[1], t311[1])];
                t308 = [vbslq_u32(t309[0], t313[0], t308[0]), vbslq_u32(t309[1], t313[1], t308[1])];
            }
            {
                let t314: [uint32x4_t; 2] = [x10[0], x10[1]];
                t308 = [vbslq_u32(t310[0], t314[0], t308[0]), vbslq_u32(t310[1], t314[1], t308[1])];
            }
            x9 = [vbslq_u32(t298[0], t308[0], x9[0]), vbslq_u32(t298[1], t308[1], x9[1])];
            b6_ag4r2l_p(l, lid0, t298, arg2, x9);
        }
        b9_r3gl_p(l, lid0, live, arg3, x7);
        let t315 = [vdupq_n_u32(x4); 2];
        let t316: [uint32x4_t; 2] = [vcgeq_u32(t295[0], t315[0]), vcgeq_u32(t295[1], t315[1])];
        let t317: [uint32x4_t; 2] = [vandq_u32(live[0], t316[0]), vandq_u32(live[1], t316[1])];
        {
            live = [vbicq_u32(live[0], t317[0]), vbicq_u32(live[1], t317[1])];
        }
        if any(&live) {
            x11 = 0u32;
            let mut t327 = live;
            loop {
                // Lanes part: each its own rounds.
                let t330 = [vdupq_n_u32(2147483648u32); 2];
                let t331: [uint32x4_t; 2] = [vandq_u32(x9[0], t330[0]), vandq_u32(x9[1], t330[1])];
                let t332 = [vdupq_n_u32(0u32); 2];
                let t333: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t331[0], t332[0])), vmvnq_u32(vceqq_u32(t331[1], t332[1]))];
                let t334 = [vdupq_n_u32(4294967294u32); 2];
                let t335: [uint32x4_t; 2] = [vcltq_u32(x9[0], t334[0]), vcltq_u32(x9[1], t334[1])];
                let t336: [uint32x4_t; 2] = [vandq_u32(t333[0], t335[0]), vandq_u32(t333[1], t335[1])];
                let t337: [uint32x4_t; 2] = [vandq_u32(vdupq_n_u32(if ((x11) < (7u32)) { u32::MAX } else { 0 }), t336[0]), vandq_u32(vdupq_n_u32(if ((x11) < (7u32)) { u32::MAX } else { 0 }), t336[1])];
                t327 = [vandq_u32(t327[0], t337[0]), vandq_u32(t327[1], t337[1])];
                if !(any(&t327)) { break; }
                let t339 = [vdupq_n_u32(2147483647u32); 2];
                let t340: [uint32x4_t; 2] = [vandq_u32(x9[0], t339[0]), vandq_u32(x9[1], t339[1])];
                x12 = t340;
                x13 = t334;
                let t341 = b10_r3ll_p(l, lid0, t327, arg3, x12, x6);
                let t342: [uint32x4_t; 2] = [vandq_u32(t327[0], t341[0]), vandq_u32(t327[1], t341[1])];
                if any(&t342) {
                    let t343 = b11_al4r2_p(l, lid0, t342, arg2, x12);
                    x13 = [vbslq_u32(t342[0], t343[0], x13[0]), vbslq_u32(t342[1], t343[1], x13[1])];
                }
                let t344: [uint32x4_t; 2] = [vceqq_u32(x13[0], t334[0]), vceqq_u32(x13[1], t334[1])];
                let t345: [uint32x4_t; 2] = [vandq_u32(t327[0], t344[0]), vandq_u32(t327[1], t344[1])];
                if any(&t345) {
                    let t346 = b8_r0r1l_p(l, lid0, t345, arg0, arg1, x12);
                    x14 = t346;
                    let t347: u64 = (1u64).wrapping_shl((32u64) as u32);
                    let t348 = [vdupq_n_u32((t347) as u32); 2];
                    let t349 = [vdupq_n_u32(((t347) >> 32) as u32); 2];
                    let t350: [uint32x4_t; 4] = [vandq_u32(x14[0], t348[0]), vandq_u32(x14[1], t348[1]), vandq_u32(x14[2], t349[0]), vandq_u32(x14[3], t349[1])];
                    let t351: u64 = 0u64;
                    let t352 = [vdupq_n_u32((t351) as u32); 2];
                    let t353: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t350[0], t352[0]), vceqq_u32(t350[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t350[1], t352[1]), vceqq_u32(t350[3], vdupq_n_u32(0)))];
                    let t354: [uint32x4_t; 2] = [vmvnq_u32(t353[0]), vmvnq_u32(t353[1])];
                    let mut t355: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                    let t356: [uint32x4_t; 2] = [vandq_u32(t345[0], t354[0]), vandq_u32(t345[1], t354[1])];
                    let t357: [uint32x4_t; 2] = [vbicq_u32(t345[0], t354[0]), vbicq_u32(t345[1], t354[1])];
                    {
                        let t358: [uint32x4_t; 2] = [x14[0], x14[1]];
                        let t359: [uint32x4_t; 2] = [vorrq_u32(t330[0], t358[0]), vorrq_u32(t330[1], t358[1])];
                        t355 = [vbslq_u32(t356[0], t359[0], t355[0]), vbslq_u32(t356[1], t359[1], t355[1])];
                    }
                    {
                        let t360: [uint32x4_t; 2] = [x14[0], x14[1]];
                        t355 = [vbslq_u32(t357[0], t360[0], t355[0]), vbslq_u32(t357[1], t360[1], t355[1])];
                    }
                    x13 = [vbslq_u32(t345[0], t355[0], x13[0]), vbslq_u32(t345[1], t355[1], x13[1])];
                }
                x9 = [vbslq_u32(t327[0], x13[0], x9[0]), vbslq_u32(t327[1], x13[1], x9[1])];
                let t361: [uint32x4_t; 2] = [vandq_u32(t327[0], live[0]), vandq_u32(t327[1], live[1])];
                let t362 = x11;
                x11 = (x11).wrapping_add(1u32);
            }
            let t363 = [vdupq_n_u32(2147483648u32); 2];
            let t364: [uint32x4_t; 2] = [vandq_u32(x9[0], t363[0]), vandq_u32(x9[1], t363[1])];
            let t365 = [vdupq_n_u32(0u32); 2];
            let t366: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t364[0], t365[0])), vmvnq_u32(vceqq_u32(t364[1], t365[1]))];
            let t367 = [vdupq_n_u32(4294967294u32); 2];
            let t368: [uint32x4_t; 2] = [vcltq_u32(x9[0], t367[0]), vcltq_u32(x9[1], t367[1])];
            let t369: [uint32x4_t; 2] = [vandq_u32(t366[0], t368[0]), vandq_u32(t366[1], t368[1])];
            let t370: [uint32x4_t; 2] = [vandq_u32(live[0], t369[0]), vandq_u32(live[1], t369[1])];
            if any(&t370) {
                let t371 = [vdupq_n_u32(2147483647u32); 2];
                let t372: [uint32x4_t; 2] = [vandq_u32(x9[0], t371[0]), vandq_u32(x9[1], t371[1])];
                let t373 = b7_r0r1l_p(l, lid0, t370, arg0, arg1, t372);
                x9 = [vbslq_u32(t370[0], t373[0], x9[0]), vbslq_u32(t370[1], t373[1], x9[1])];
            }
            b6_ag4r2l_p(l, lid0, live, arg2, x9);
        }
    }
}

/// `parent_back_node`.
#[inline(always)]
unsafe fn b3_r0r1r2r3ssllg_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, arg2: *const u8, arg3: *const u8, x4: u32, x5: bool, x6: [uint32x4_t; 2], x7: [uint32x4_t; 2]) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x9: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x10: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x11: u32 = 0;
    let mut x12: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x13: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x14: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    if !(x5) {
        if (((gid0) as u64 + 7) <= u32::MAX as u64 && ((gid0) as u64) >= ((x4) as u64)) {
        } else if ((gid0) as u64 + 7) < ((x4) as u64) {
            let t374 = b7_r0r1g_f(l, lid0, arg0, arg1);
            b6_ag4r2l_f(l, lid0, arg2, t374);
        } else {
            // Lanes part: the chunk's lanes on both sides of the bound.
            let t375: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
            let t376 = [vdupq_n_u32(x4); 2];
            let t377: [uint32x4_t; 2] = [vcltq_u32(t375[0], t376[0]), vcltq_u32(t375[1], t376[1])];
            let t378: [uint32x4_t; 2] = [vandq_u32(live[0], t377[0]), vandq_u32(live[1], t377[1])];
            if any(&t378) {
                let t379 = b7_r0r1g_p(l, lid0, t378, arg0, arg1);
                b6_ag4r2l_p(l, lid0, t378, arg2, t379);
            }
        }
        return;
    }
    if any(&live) {
        let t380 = [vdupq_n_u32(4294967295u32); 2];
        x9 = t380;
        let t381: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t382 = [vdupq_n_u32(x4); 2];
        let t383: [uint32x4_t; 2] = [vcltq_u32(t381[0], t382[0]), vcltq_u32(t381[1], t382[1])];
        let t384: [uint32x4_t; 2] = [vandq_u32(live[0], t383[0]), vandq_u32(live[1], t383[1])];
        if any(&t384) {
            let t385 = b8_r0r1g_p(l, lid0, t384, arg0, arg1);
            x10 = t385;
            let t386: u64 = (1u64).wrapping_shl((32u64) as u32);
            let t387 = [vdupq_n_u32((t386) as u32); 2];
            let t388 = [vdupq_n_u32(((t386) >> 32) as u32); 2];
            let t389: [uint32x4_t; 4] = [vandq_u32(x10[0], t387[0]), vandq_u32(x10[1], t387[1]), vandq_u32(x10[2], t388[0]), vandq_u32(x10[3], t388[1])];
            let t390: u64 = 0u64;
            let t391 = [vdupq_n_u32((t390) as u32); 2];
            let t392: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t389[0], t391[0]), vceqq_u32(t389[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t389[1], t391[1]), vceqq_u32(t389[3], vdupq_n_u32(0)))];
            let t393: [uint32x4_t; 2] = [vmvnq_u32(t392[0]), vmvnq_u32(t392[1])];
            let mut t394: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t395: [uint32x4_t; 2] = [vandq_u32(t384[0], t393[0]), vandq_u32(t384[1], t393[1])];
            let t396: [uint32x4_t; 2] = [vbicq_u32(t384[0], t393[0]), vbicq_u32(t384[1], t393[1])];
            {
                let t397: [uint32x4_t; 2] = [x10[0], x10[1]];
                let t398 = [vdupq_n_u32(2147483648u32); 2];
                let t399: [uint32x4_t; 2] = [vorrq_u32(t398[0], t397[0]), vorrq_u32(t398[1], t397[1])];
                t394 = [vbslq_u32(t395[0], t399[0], t394[0]), vbslq_u32(t395[1], t399[1], t394[1])];
            }
            {
                let t400: [uint32x4_t; 2] = [x10[0], x10[1]];
                t394 = [vbslq_u32(t396[0], t400[0], t394[0]), vbslq_u32(t396[1], t400[1], t394[1])];
            }
            x9 = [vbslq_u32(t384[0], t394[0], x9[0]), vbslq_u32(t384[1], t394[1], x9[1])];
            b6_ag4r2l_p(l, lid0, t384, arg2, x9);
        }
        b9_r3gl_p(l, lid0, live, arg3, x7);
        let t401 = [vdupq_n_u32(x4); 2];
        let t402: [uint32x4_t; 2] = [vcgeq_u32(t381[0], t401[0]), vcgeq_u32(t381[1], t401[1])];
        let t403: [uint32x4_t; 2] = [vandq_u32(live[0], t402[0]), vandq_u32(live[1], t402[1])];
        {
            live = [vbicq_u32(live[0], t403[0]), vbicq_u32(live[1], t403[1])];
        }
        if any(&live) {
            x11 = 0u32;
            let mut t413 = live;
            loop {
                // Lanes part: each its own rounds.
                let t416 = [vdupq_n_u32(2147483648u32); 2];
                let t417: [uint32x4_t; 2] = [vandq_u32(x9[0], t416[0]), vandq_u32(x9[1], t416[1])];
                let t418 = [vdupq_n_u32(0u32); 2];
                let t419: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t417[0], t418[0])), vmvnq_u32(vceqq_u32(t417[1], t418[1]))];
                let t420 = [vdupq_n_u32(4294967294u32); 2];
                let t421: [uint32x4_t; 2] = [vcltq_u32(x9[0], t420[0]), vcltq_u32(x9[1], t420[1])];
                let t422: [uint32x4_t; 2] = [vandq_u32(t419[0], t421[0]), vandq_u32(t419[1], t421[1])];
                let t423: [uint32x4_t; 2] = [vandq_u32(vdupq_n_u32(if ((x11) < (7u32)) { u32::MAX } else { 0 }), t422[0]), vandq_u32(vdupq_n_u32(if ((x11) < (7u32)) { u32::MAX } else { 0 }), t422[1])];
                t413 = [vandq_u32(t413[0], t423[0]), vandq_u32(t413[1], t423[1])];
                if !(any(&t413)) { break; }
                let t425 = [vdupq_n_u32(2147483647u32); 2];
                let t426: [uint32x4_t; 2] = [vandq_u32(x9[0], t425[0]), vandq_u32(x9[1], t425[1])];
                x12 = t426;
                x13 = t420;
                let t427 = b10_r3ll_p(l, lid0, t413, arg3, x12, x6);
                let t428: [uint32x4_t; 2] = [vandq_u32(t413[0], t427[0]), vandq_u32(t413[1], t427[1])];
                if any(&t428) {
                    let t429 = b11_al4r2_p(l, lid0, t428, arg2, x12);
                    x13 = [vbslq_u32(t428[0], t429[0], x13[0]), vbslq_u32(t428[1], t429[1], x13[1])];
                }
                let t430: [uint32x4_t; 2] = [vceqq_u32(x13[0], t420[0]), vceqq_u32(x13[1], t420[1])];
                let t431: [uint32x4_t; 2] = [vandq_u32(t413[0], t430[0]), vandq_u32(t413[1], t430[1])];
                if any(&t431) {
                    let t432 = b8_r0r1l_p(l, lid0, t431, arg0, arg1, x12);
                    x14 = t432;
                    let t433: u64 = (1u64).wrapping_shl((32u64) as u32);
                    let t434 = [vdupq_n_u32((t433) as u32); 2];
                    let t435 = [vdupq_n_u32(((t433) >> 32) as u32); 2];
                    let t436: [uint32x4_t; 4] = [vandq_u32(x14[0], t434[0]), vandq_u32(x14[1], t434[1]), vandq_u32(x14[2], t435[0]), vandq_u32(x14[3], t435[1])];
                    let t437: u64 = 0u64;
                    let t438 = [vdupq_n_u32((t437) as u32); 2];
                    let t439: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t436[0], t438[0]), vceqq_u32(t436[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t436[1], t438[1]), vceqq_u32(t436[3], vdupq_n_u32(0)))];
                    let t440: [uint32x4_t; 2] = [vmvnq_u32(t439[0]), vmvnq_u32(t439[1])];
                    let mut t441: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                    let t442: [uint32x4_t; 2] = [vandq_u32(t431[0], t440[0]), vandq_u32(t431[1], t440[1])];
                    let t443: [uint32x4_t; 2] = [vbicq_u32(t431[0], t440[0]), vbicq_u32(t431[1], t440[1])];
                    {
                        let t444: [uint32x4_t; 2] = [x14[0], x14[1]];
                        let t445: [uint32x4_t; 2] = [vorrq_u32(t416[0], t444[0]), vorrq_u32(t416[1], t444[1])];
                        t441 = [vbslq_u32(t442[0], t445[0], t441[0]), vbslq_u32(t442[1], t445[1], t441[1])];
                    }
                    {
                        let t446: [uint32x4_t; 2] = [x14[0], x14[1]];
                        t441 = [vbslq_u32(t443[0], t446[0], t441[0]), vbslq_u32(t443[1], t446[1], t441[1])];
                    }
                    x13 = [vbslq_u32(t431[0], t441[0], x13[0]), vbslq_u32(t431[1], t441[1], x13[1])];
                }
                x9 = [vbslq_u32(t413[0], x13[0], x9[0]), vbslq_u32(t413[1], x13[1], x9[1])];
                let t447: [uint32x4_t; 2] = [vandq_u32(t413[0], live[0]), vandq_u32(t413[1], live[1])];
                let t448 = x11;
                x11 = (x11).wrapping_add(1u32);
            }
            let t449 = [vdupq_n_u32(2147483648u32); 2];
            let t450: [uint32x4_t; 2] = [vandq_u32(x9[0], t449[0]), vandq_u32(x9[1], t449[1])];
            let t451 = [vdupq_n_u32(0u32); 2];
            let t452: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t450[0], t451[0])), vmvnq_u32(vceqq_u32(t450[1], t451[1]))];
            let t453 = [vdupq_n_u32(4294967294u32); 2];
            let t454: [uint32x4_t; 2] = [vcltq_u32(x9[0], t453[0]), vcltq_u32(x9[1], t453[1])];
            let t455: [uint32x4_t; 2] = [vandq_u32(t452[0], t454[0]), vandq_u32(t452[1], t454[1])];
            let t456: [uint32x4_t; 2] = [vandq_u32(live[0], t455[0]), vandq_u32(live[1], t455[1])];
            if any(&t456) {
                let t457 = [vdupq_n_u32(2147483647u32); 2];
                let t458: [uint32x4_t; 2] = [vandq_u32(x9[0], t457[0]), vandq_u32(x9[1], t457[1])];
                let t459 = b7_r0r1l_p(l, lid0, t456, arg0, arg1, t458);
                x9 = [vbslq_u32(t456[0], t459[0], x9[0]), vbslq_u32(t456[1], t459[1], x9[1])];
            }
            b6_ag4r2l_p(l, lid0, live, arg2, x9);
        }
    }
}

/// `ukl_atomic_load`.
#[inline(always)]
unsafe fn b4_o4_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg4_x0: *const u8) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    res = (arg4_x0 as *const u32).read_unaligned();
    return res;
}

/// `ukl_fence_device`.
#[inline(always)]
unsafe fn b5__p(l: &Launch, lid0: u32, k: [uint32x4_t; 2]) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
}

/// `ukl_atomic_store`.
#[inline(always)]
unsafe fn b6_ag4r2l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg2: *const u8, x1: [uint32x4_t; 2]) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let t460 = (arg2 as *mut u32).add((gid0) as usize);
    if all(&live) {
        vst1q_u32(t460.add(0), x1[0]);
        vst1q_u32(t460.add(4), x1[1]);
    } else {
        let mut t461w = 0u32;
        let t461 = core::hint::black_box(&raw mut t461w);
        core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t460.add(0), t461).write(vgetq_lane_u32::<0>(x1[0]));
        core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t460.add(1), t461).write(vgetq_lane_u32::<1>(x1[0]));
        core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t460.add(2), t461).write(vgetq_lane_u32::<2>(x1[0]));
        core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t460.add(3), t461).write(vgetq_lane_u32::<3>(x1[0]));
        core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t460.add(4), t461).write(vgetq_lane_u32::<0>(x1[1]));
        core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t460.add(5), t461).write(vgetq_lane_u32::<1>(x1[1]));
        core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t460.add(6), t461).write(vgetq_lane_u32::<2>(x1[1]));
        core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t460.add(7), t461).write(vgetq_lane_u32::<3>(x1[1]));
    }
}

/// `ukl_atomic_store`.
#[inline(always)]
unsafe fn b6_ag4r2l_f(l: &Launch, lid0: u32, arg2: *const u8, x1: [uint32x4_t; 2]) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let t462 = (arg2 as *mut u32).add((gid0) as usize);
    vst1q_u32(t462.add(0), x1[0]);
    vst1q_u32(t462.add(4), x1[1]);
}

/// `clif_f16_parent`.
#[inline(always)]
unsafe fn b7_r0r1g_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x3: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t463 = b8_r0r1g_p(l, lid0, live, arg0, arg1);
    x3 = t463;
    let mut t472 = live;
    loop {
        // Lanes part: each its own rounds.
        let t474: u64 = (1u64).wrapping_shl((32u64) as u32);
        let t475 = [vdupq_n_u32((t474) as u32); 2];
        let t476 = [vdupq_n_u32(((t474) >> 32) as u32); 2];
        let t477: [uint32x4_t; 4] = [vandq_u32(x3[0], t475[0]), vandq_u32(x3[1], t475[1]), vandq_u32(x3[2], t476[0]), vandq_u32(x3[3], t476[1])];
        let t478: u64 = 0u64;
        let t479 = [vdupq_n_u32((t478) as u32); 2];
        let t480: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t477[0], t479[0]), vceqq_u32(t477[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t477[1], t479[1]), vceqq_u32(t477[3], vdupq_n_u32(0)))];
        let t481: [uint32x4_t; 2] = [vmvnq_u32(t480[0]), vmvnq_u32(t480[1])];
        t472 = [vandq_u32(t472[0], t481[0]), vandq_u32(t472[1], t481[1])];
        if !(any(&t472)) { break; }
        let t483: [uint32x4_t; 2] = [x3[0], x3[1]];
        let t484 = b8_r0r1l_p(l, lid0, t472, arg0, arg1, t483);
        x3 = [vbslq_u32(t472[0], t484[0], x3[0]), vbslq_u32(t472[1], t484[1], x3[1]), vbslq_u32(t472[0], t484[2], x3[2]), vbslq_u32(t472[1], t484[3], x3[3])];
    }
    let t485: [uint32x4_t; 2] = [x3[0], x3[1]];
    res = t485;
    return res;
}

/// `clif_f16_parent`.
#[inline(always)]
unsafe fn b7_r0r1g_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x3: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t486 = b8_r0r1g_f(l, lid0, arg0, arg1);
    x3 = t486;
    let mut t503 = live;
    loop {
        // Lanes part: each its own rounds.
        let t505: u64 = (1u64).wrapping_shl((32u64) as u32);
        let t506 = [vdupq_n_u32((t505) as u32); 2];
        let t507 = [vdupq_n_u32(((t505) >> 32) as u32); 2];
        let t508: [uint32x4_t; 4] = [vandq_u32(x3[0], t506[0]), vandq_u32(x3[1], t506[1]), vandq_u32(x3[2], t507[0]), vandq_u32(x3[3], t507[1])];
        let t509: u64 = 0u64;
        let t510 = [vdupq_n_u32((t509) as u32); 2];
        let t511: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t508[0], t510[0]), vceqq_u32(t508[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t508[1], t510[1]), vceqq_u32(t508[3], vdupq_n_u32(0)))];
        let t512: [uint32x4_t; 2] = [vmvnq_u32(t511[0]), vmvnq_u32(t511[1])];
        t503 = [vandq_u32(t503[0], t512[0]), vandq_u32(t503[1], t512[1])];
        if !(any(&t503)) { break; }
        let t514: [uint32x4_t; 2] = [x3[0], x3[1]];
        let t515 = b8_r0r1l_p(l, lid0, t503, arg0, arg1, t514);
        x3 = [vbslq_u32(t503[0], t515[0], x3[0]), vbslq_u32(t503[1], t515[1], x3[1]), vbslq_u32(t503[0], t515[2], x3[2]), vbslq_u32(t503[1], t515[3], x3[3])];
    }
    let t516: [uint32x4_t; 2] = [x3[0], x3[1]];
    res = t516;
    return res;
}

/// `clif_f16_parent`.
#[inline(always)]
unsafe fn b7_r0r1l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x3: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t517 = b8_r0r1l_p(l, lid0, live, arg0, arg1, x2);
    x3 = t517;
    let mut t526 = live;
    loop {
        // Lanes part: each its own rounds.
        let t528: u64 = (1u64).wrapping_shl((32u64) as u32);
        let t529 = [vdupq_n_u32((t528) as u32); 2];
        let t530 = [vdupq_n_u32(((t528) >> 32) as u32); 2];
        let t531: [uint32x4_t; 4] = [vandq_u32(x3[0], t529[0]), vandq_u32(x3[1], t529[1]), vandq_u32(x3[2], t530[0]), vandq_u32(x3[3], t530[1])];
        let t532: u64 = 0u64;
        let t533 = [vdupq_n_u32((t532) as u32); 2];
        let t534: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t531[0], t533[0]), vceqq_u32(t531[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t531[1], t533[1]), vceqq_u32(t531[3], vdupq_n_u32(0)))];
        let t535: [uint32x4_t; 2] = [vmvnq_u32(t534[0]), vmvnq_u32(t534[1])];
        t526 = [vandq_u32(t526[0], t535[0]), vandq_u32(t526[1], t535[1])];
        if !(any(&t526)) { break; }
        let t537: [uint32x4_t; 2] = [x3[0], x3[1]];
        let t538 = b8_r0r1l_p(l, lid0, t526, arg0, arg1, t537);
        x3 = [vbslq_u32(t526[0], t538[0], x3[0]), vbslq_u32(t526[1], t538[1], x3[1]), vbslq_u32(t526[0], t538[2], x3[2]), vbslq_u32(t526[1], t538[3], x3[3])];
    }
    let t539: [uint32x4_t; 2] = [x3[0], x3[1]];
    res = t539;
    return res;
}

/// `clif_f16_parent_step`.
#[inline(always)]
unsafe fn b8_r0r1g_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x3: u32 = 0;
    let mut x4: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x5: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x6: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t540: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
    let t541 = [vdupq_n_u32(0u32); 2];
    let t542: [uint32x4_t; 2] = [vceqq_u32(t540[0], t541[0]), vceqq_u32(t540[1], t541[1])];
    let t543: [uint32x4_t; 2] = [vandq_u32(live[0], t542[0]), vandq_u32(live[1], t542[1])];
    {
        let t544: u64 = 4294967295u64;
        let t545 = [vdupq_n_u32((t544) as u32); 2];
        let t546: [uint32x4_t; 4] = [t545[0], t545[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = [vbslq_u32(t543[0], t546[0], res[0]), vbslq_u32(t543[1], t546[1], res[1]), vbslq_u32(t543[0], t546[2], res[2]), vbslq_u32(t543[1], t546[3], res[3])];
        live = [vbicq_u32(live[0], t543[0]), vbicq_u32(live[1], t543[1])];
    }
    if any(&live) {
        x3 = gid0.wrapping_sub(1u32);
        let t547 = if (x3 & 255u32) <= 248u32 { b12_r0r1u8_p(l, lid0, live, arg0, arg1, x3) } else { b12_r0r1i_p(l, lid0, live, arg0, arg1, x3) };
        x4 = t547;
        let t548: u64 = 1u64;
        let t549 = [vdupq_n_u32((t548) as u32); 2];
        let t550: [uint32x4_t; 2] = [vandq_u32(x4[0], t549[0]), vandq_u32(x4[1], t549[1])];
        let t551: u64 = 0u64;
        let t552 = [vdupq_n_u32((t551) as u32); 2];
        let t553: [uint32x4_t; 2] = [vceqq_u32(t550[0], t552[0]), vceqq_u32(t550[1], t552[1])];
        let t554: [uint32x4_t; 2] = [vandq_u32(live[0], t553[0]), vandq_u32(live[1], t553[1])];
        {
            let t555: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[1].as_ptr()))];
            let t556: [uint32x4_t; 4] = [t555[0], t555[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t554[0], t556[0], res[0]), vbslq_u32(t554[1], t556[1], res[1]), vbslq_u32(t554[0], t556[2], res[2]), vbslq_u32(t554[1], t556[3], res[3])];
            live = [vbicq_u32(live[0], t554[0]), vbicq_u32(live[1], t554[1])];
        }
        if any(&live) {
            let t557: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x4[0]), vshlq_n_u32::<30>(x4[2])), vorrq_u32(vshrq_n_u32::<2>(x4[1]), vshlq_n_u32::<30>(x4[3])), vshrq_n_u32::<2>(x4[2]), vshrq_n_u32::<2>(x4[3])];
            let t558: [uint32x4_t; 2] = [t557[0], t557[1]];
            x5 = t558;
            let t559 = [vdupq_n_u32(1u32); 2];
            let t560: [uint32x4_t; 2] = [vsubq_u32(x5[0], t559[0]), vsubq_u32(x5[1], t559[1])];
            let t561: [uint32x4_t; 2] = [vcgeq_u32(t560[0], t540[0]), vcgeq_u32(t560[1], t540[1])];
            let t562: [uint32x4_t; 2] = [vandq_u32(live[0], t561[0]), vandq_u32(live[1], t561[1])];
            {
                let t563: u64 = 4294967295u64;
                let t564 = [vdupq_n_u32((t563) as u32); 2];
                let t565: [uint32x4_t; 4] = [t564[0], t564[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t562[0], t565[0], res[0]), vbslq_u32(t562[1], t565[1], res[1]), vbslq_u32(t562[0], t565[2], res[2]), vbslq_u32(t562[1], t565[3], res[3])];
                live = [vbicq_u32(live[0], t562[0]), vbicq_u32(live[1], t562[1])];
            }
            if any(&live) {
                let t566: [uint32x4_t; 2] = [vsubq_u32(t540[0], x5[0]), vsubq_u32(t540[1], x5[1])];
                x6 = t566;
                let t567: u64 = 2u64;
                let t568 = [vdupq_n_u32((t567) as u32); 2];
                let t569: [uint32x4_t; 2] = [vandq_u32(x4[0], t568[0]), vandq_u32(x4[1], t568[1])];
                let t570: u64 = 0u64;
                let t571 = [vdupq_n_u32((t570) as u32); 2];
                let t572: [uint32x4_t; 2] = [vceqq_u32(t569[0], t571[0]), vceqq_u32(t569[1], t571[1])];
                let t573: [uint32x4_t; 2] = [vmvnq_u32(t572[0]), vmvnq_u32(t572[1])];
                let t574: [uint32x4_t; 2] = [vandq_u32(live[0], t573[0]), vandq_u32(live[1], t573[1])];
                {
                    let t575: [uint32x4_t; 4] = [x6[0], x6[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                    res = [vbslq_u32(t574[0], t575[0], res[0]), vbslq_u32(t574[1], t575[1], res[1]), vbslq_u32(t574[0], t575[2], res[2]), vbslq_u32(t574[1], t575[3], res[3])];
                    live = [vbicq_u32(live[0], t574[0]), vbicq_u32(live[1], t574[1])];
                }
                if any(&live) {
                    let t576: [uint32x4_t; 2] = [vceqq_u32(x6[0], t541[0]), vceqq_u32(x6[1], t541[1])];
                    let t577: [uint32x4_t; 2] = [vandq_u32(live[0], t576[0]), vandq_u32(live[1], t576[1])];
                    {
                        let t578: u64 = 4294967295u64;
                        let t579 = [vdupq_n_u32((t578) as u32); 2];
                        let t580: [uint32x4_t; 4] = [t579[0], t579[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                        res = [vbslq_u32(t577[0], t580[0], res[0]), vbslq_u32(t577[1], t580[1], res[1]), vbslq_u32(t577[0], t580[2], res[2]), vbslq_u32(t577[1], t580[3], res[3])];
                        live = [vbicq_u32(live[0], t577[0]), vbicq_u32(live[1], t577[1])];
                    }
                    if any(&live) {
                        let t581: u64 = (1u64).wrapping_shl((32u64) as u32);
                        let t582 = [vdupq_n_u32((t581) as u32); 2];
                        let t583 = [vdupq_n_u32(((t581) >> 32) as u32); 2];
                        let t584: [uint32x4_t; 4] = [vorrq_u32(t582[0], x6[0]), vorrq_u32(t582[1], x6[1]), t583[0], t583[1]];
                        res = [vbslq_u32(live[0], t584[0], res[0]), vbslq_u32(live[1], t584[1], res[1]), vbslq_u32(live[0], t584[2], res[2]), vbslq_u32(live[1], t584[3], res[3])];
                        live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
                    }
                }
            }
        }
    }
    res
}

/// `clif_f16_parent_step`.
#[inline(always)]
unsafe fn b8_r0r1g_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x3: u32 = 0;
    let mut x4: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x5: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x6: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if (((gid0) as u64 + 7) <= u32::MAX as u64 && (((0u32) as u64) < ((gid0) as u64) || ((0u32) as u64) > ((gid0) as u64 + 7))) {
        x3 = gid0.wrapping_sub(1u32);
        let t585 = if (x3 & 255u32) <= 248u32 { b12_r0r1u8_f(l, lid0, arg0, arg1, x3) } else { b12_r0r1i_f(l, lid0, arg0, arg1, x3) };
        x4 = t585;
        let t586: u64 = 1u64;
        let t587 = [vdupq_n_u32((t586) as u32); 2];
        let t588: [uint32x4_t; 2] = [vandq_u32(x4[0], t587[0]), vandq_u32(x4[1], t587[1])];
        let t589: u64 = 0u64;
        let t590 = [vdupq_n_u32((t589) as u32); 2];
        let t591: [uint32x4_t; 2] = [vceqq_u32(t588[0], t590[0]), vceqq_u32(t588[1], t590[1])];
        {
            let t592: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[1].as_ptr()))];
            let t593: [uint32x4_t; 4] = [t592[0], t592[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t591[0], t593[0], res[0]), vbslq_u32(t591[1], t593[1], res[1]), vbslq_u32(t591[0], t593[2], res[2]), vbslq_u32(t591[1], t593[3], res[3])];
            live = [vbicq_u32(live[0], t591[0]), vbicq_u32(live[1], t591[1])];
        }
        if any(&live) {
            let t594: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x4[0]), vshlq_n_u32::<30>(x4[2])), vorrq_u32(vshrq_n_u32::<2>(x4[1]), vshlq_n_u32::<30>(x4[3])), vshrq_n_u32::<2>(x4[2]), vshrq_n_u32::<2>(x4[3])];
            let t595: [uint32x4_t; 2] = [t594[0], t594[1]];
            x5 = t595;
            let t596 = [vdupq_n_u32(1u32); 2];
            let t597: [uint32x4_t; 2] = [vsubq_u32(x5[0], t596[0]), vsubq_u32(x5[1], t596[1])];
            let t598: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
            let t599: [uint32x4_t; 2] = [vcgeq_u32(t597[0], t598[0]), vcgeq_u32(t597[1], t598[1])];
            let t600: [uint32x4_t; 2] = [vandq_u32(live[0], t599[0]), vandq_u32(live[1], t599[1])];
            {
                let t601: u64 = 4294967295u64;
                let t602 = [vdupq_n_u32((t601) as u32); 2];
                let t603: [uint32x4_t; 4] = [t602[0], t602[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t600[0], t603[0], res[0]), vbslq_u32(t600[1], t603[1], res[1]), vbslq_u32(t600[0], t603[2], res[2]), vbslq_u32(t600[1], t603[3], res[3])];
                live = [vbicq_u32(live[0], t600[0]), vbicq_u32(live[1], t600[1])];
            }
            if any(&live) {
                let t604: [uint32x4_t; 2] = [vsubq_u32(t598[0], x5[0]), vsubq_u32(t598[1], x5[1])];
                x6 = t604;
                let t605: u64 = 2u64;
                let t606 = [vdupq_n_u32((t605) as u32); 2];
                let t607: [uint32x4_t; 2] = [vandq_u32(x4[0], t606[0]), vandq_u32(x4[1], t606[1])];
                let t608: u64 = 0u64;
                let t609 = [vdupq_n_u32((t608) as u32); 2];
                let t610: [uint32x4_t; 2] = [vceqq_u32(t607[0], t609[0]), vceqq_u32(t607[1], t609[1])];
                let t611: [uint32x4_t; 2] = [vmvnq_u32(t610[0]), vmvnq_u32(t610[1])];
                let t612: [uint32x4_t; 2] = [vandq_u32(live[0], t611[0]), vandq_u32(live[1], t611[1])];
                {
                    let t613: [uint32x4_t; 4] = [x6[0], x6[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                    res = [vbslq_u32(t612[0], t613[0], res[0]), vbslq_u32(t612[1], t613[1], res[1]), vbslq_u32(t612[0], t613[2], res[2]), vbslq_u32(t612[1], t613[3], res[3])];
                    live = [vbicq_u32(live[0], t612[0]), vbicq_u32(live[1], t612[1])];
                }
                if any(&live) {
                    let t614 = [vdupq_n_u32(0u32); 2];
                    let t615: [uint32x4_t; 2] = [vceqq_u32(x6[0], t614[0]), vceqq_u32(x6[1], t614[1])];
                    let t616: [uint32x4_t; 2] = [vandq_u32(live[0], t615[0]), vandq_u32(live[1], t615[1])];
                    {
                        let t617: u64 = 4294967295u64;
                        let t618 = [vdupq_n_u32((t617) as u32); 2];
                        let t619: [uint32x4_t; 4] = [t618[0], t618[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                        res = [vbslq_u32(t616[0], t619[0], res[0]), vbslq_u32(t616[1], t619[1], res[1]), vbslq_u32(t616[0], t619[2], res[2]), vbslq_u32(t616[1], t619[3], res[3])];
                        live = [vbicq_u32(live[0], t616[0]), vbicq_u32(live[1], t616[1])];
                    }
                    if any(&live) {
                        let t620: u64 = (1u64).wrapping_shl((32u64) as u32);
                        let t621 = [vdupq_n_u32((t620) as u32); 2];
                        let t622 = [vdupq_n_u32(((t620) >> 32) as u32); 2];
                        let t623: [uint32x4_t; 4] = [vorrq_u32(t621[0], x6[0]), vorrq_u32(t621[1], x6[1]), t622[0], t622[1]];
                        res = [vbslq_u32(live[0], t623[0], res[0]), vbslq_u32(live[1], t623[1], res[1]), vbslq_u32(live[0], t623[2], res[2]), vbslq_u32(live[1], t623[3], res[3])];
                        live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
                    }
                }
            }
        }
    } else if false {
        let t624: u64 = 4294967295u64;
        let t625 = [vdupq_n_u32((t624) as u32); 2];
        let t626: [uint32x4_t; 4] = [t625[0], t625[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t626;
        return res;
    } else {
        // Lanes part: the chunk's lanes on both sides of the bound.
        let t627: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t628 = [vdupq_n_u32(0u32); 2];
        let t629: [uint32x4_t; 2] = [vceqq_u32(t627[0], t628[0]), vceqq_u32(t627[1], t628[1])];
        let t630: [uint32x4_t; 2] = [vandq_u32(live[0], t629[0]), vandq_u32(live[1], t629[1])];
        {
            let t631: u64 = 4294967295u64;
            let t632 = [vdupq_n_u32((t631) as u32); 2];
            let t633: [uint32x4_t; 4] = [t632[0], t632[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t630[0], t633[0], res[0]), vbslq_u32(t630[1], t633[1], res[1]), vbslq_u32(t630[0], t633[2], res[2]), vbslq_u32(t630[1], t633[3], res[3])];
            live = [vbicq_u32(live[0], t630[0]), vbicq_u32(live[1], t630[1])];
        }
        if any(&live) {
            x3 = gid0.wrapping_sub(1u32);
            let t634 = if (x3 & 255u32) <= 248u32 { b12_r0r1u8_p(l, lid0, live, arg0, arg1, x3) } else { b12_r0r1i_p(l, lid0, live, arg0, arg1, x3) };
            x4 = t634;
            let t635: u64 = 1u64;
            let t636 = [vdupq_n_u32((t635) as u32); 2];
            let t637: [uint32x4_t; 2] = [vandq_u32(x4[0], t636[0]), vandq_u32(x4[1], t636[1])];
            let t638: u64 = 0u64;
            let t639 = [vdupq_n_u32((t638) as u32); 2];
            let t640: [uint32x4_t; 2] = [vceqq_u32(t637[0], t639[0]), vceqq_u32(t637[1], t639[1])];
            let t641: [uint32x4_t; 2] = [vandq_u32(live[0], t640[0]), vandq_u32(live[1], t640[1])];
            {
                let t642: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[1].as_ptr()))];
                let t643: [uint32x4_t; 4] = [t642[0], t642[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t641[0], t643[0], res[0]), vbslq_u32(t641[1], t643[1], res[1]), vbslq_u32(t641[0], t643[2], res[2]), vbslq_u32(t641[1], t643[3], res[3])];
                live = [vbicq_u32(live[0], t641[0]), vbicq_u32(live[1], t641[1])];
            }
            if any(&live) {
                let t644: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x4[0]), vshlq_n_u32::<30>(x4[2])), vorrq_u32(vshrq_n_u32::<2>(x4[1]), vshlq_n_u32::<30>(x4[3])), vshrq_n_u32::<2>(x4[2]), vshrq_n_u32::<2>(x4[3])];
                let t645: [uint32x4_t; 2] = [t644[0], t644[1]];
                x5 = t645;
                let t646 = [vdupq_n_u32(1u32); 2];
                let t647: [uint32x4_t; 2] = [vsubq_u32(x5[0], t646[0]), vsubq_u32(x5[1], t646[1])];
                let t648: [uint32x4_t; 2] = [vcgeq_u32(t647[0], t627[0]), vcgeq_u32(t647[1], t627[1])];
                let t649: [uint32x4_t; 2] = [vandq_u32(live[0], t648[0]), vandq_u32(live[1], t648[1])];
                {
                    let t650: u64 = 4294967295u64;
                    let t651 = [vdupq_n_u32((t650) as u32); 2];
                    let t652: [uint32x4_t; 4] = [t651[0], t651[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                    res = [vbslq_u32(t649[0], t652[0], res[0]), vbslq_u32(t649[1], t652[1], res[1]), vbslq_u32(t649[0], t652[2], res[2]), vbslq_u32(t649[1], t652[3], res[3])];
                    live = [vbicq_u32(live[0], t649[0]), vbicq_u32(live[1], t649[1])];
                }
                if any(&live) {
                    let t653: [uint32x4_t; 2] = [vsubq_u32(t627[0], x5[0]), vsubq_u32(t627[1], x5[1])];
                    x6 = t653;
                    let t654: u64 = 2u64;
                    let t655 = [vdupq_n_u32((t654) as u32); 2];
                    let t656: [uint32x4_t; 2] = [vandq_u32(x4[0], t655[0]), vandq_u32(x4[1], t655[1])];
                    let t657: u64 = 0u64;
                    let t658 = [vdupq_n_u32((t657) as u32); 2];
                    let t659: [uint32x4_t; 2] = [vceqq_u32(t656[0], t658[0]), vceqq_u32(t656[1], t658[1])];
                    let t660: [uint32x4_t; 2] = [vmvnq_u32(t659[0]), vmvnq_u32(t659[1])];
                    let t661: [uint32x4_t; 2] = [vandq_u32(live[0], t660[0]), vandq_u32(live[1], t660[1])];
                    {
                        let t662: [uint32x4_t; 4] = [x6[0], x6[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                        res = [vbslq_u32(t661[0], t662[0], res[0]), vbslq_u32(t661[1], t662[1], res[1]), vbslq_u32(t661[0], t662[2], res[2]), vbslq_u32(t661[1], t662[3], res[3])];
                        live = [vbicq_u32(live[0], t661[0]), vbicq_u32(live[1], t661[1])];
                    }
                    if any(&live) {
                        let t663: [uint32x4_t; 2] = [vceqq_u32(x6[0], t628[0]), vceqq_u32(x6[1], t628[1])];
                        let t664: [uint32x4_t; 2] = [vandq_u32(live[0], t663[0]), vandq_u32(live[1], t663[1])];
                        {
                            let t665: u64 = 4294967295u64;
                            let t666 = [vdupq_n_u32((t665) as u32); 2];
                            let t667: [uint32x4_t; 4] = [t666[0], t666[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                            res = [vbslq_u32(t664[0], t667[0], res[0]), vbslq_u32(t664[1], t667[1], res[1]), vbslq_u32(t664[0], t667[2], res[2]), vbslq_u32(t664[1], t667[3], res[3])];
                            live = [vbicq_u32(live[0], t664[0]), vbicq_u32(live[1], t664[1])];
                        }
                        if any(&live) {
                            let t668: u64 = (1u64).wrapping_shl((32u64) as u32);
                            let t669 = [vdupq_n_u32((t668) as u32); 2];
                            let t670 = [vdupq_n_u32(((t668) >> 32) as u32); 2];
                            let t671: [uint32x4_t; 4] = [vorrq_u32(t669[0], x6[0]), vorrq_u32(t669[1], x6[1]), t670[0], t670[1]];
                            res = [vbslq_u32(live[0], t671[0], res[0]), vbslq_u32(live[1], t671[1], res[1]), vbslq_u32(live[0], t671[2], res[2]), vbslq_u32(live[1], t671[3], res[3])];
                            live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
                        }
                    }
                }
            }
        }
    }
    res
}

/// `clif_f16_parent_step`.
#[inline(always)]
unsafe fn b8_r0r1l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x3: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x4: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x5: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x6: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t672 = [vdupq_n_u32(0u32); 2];
    let t673: [uint32x4_t; 2] = [vceqq_u32(x2[0], t672[0]), vceqq_u32(x2[1], t672[1])];
    let t674: [uint32x4_t; 2] = [vandq_u32(live[0], t673[0]), vandq_u32(live[1], t673[1])];
    {
        let t675: u64 = 4294967295u64;
        let t676 = [vdupq_n_u32((t675) as u32); 2];
        let t677: [uint32x4_t; 4] = [t676[0], t676[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = [vbslq_u32(t674[0], t677[0], res[0]), vbslq_u32(t674[1], t677[1], res[1]), vbslq_u32(t674[0], t677[2], res[2]), vbslq_u32(t674[1], t677[3], res[3])];
        live = [vbicq_u32(live[0], t674[0]), vbicq_u32(live[1], t674[1])];
    }
    if any(&live) {
        let t678 = [vdupq_n_u32(1u32); 2];
        let t679: [uint32x4_t; 2] = [vsubq_u32(x2[0], t678[0]), vsubq_u32(x2[1], t678[1])];
        x3 = t679;
        let t680 = b12_r0r1l_p(l, lid0, live, arg0, arg1, x3);
        x4 = t680;
        let t681: u64 = 1u64;
        let t682 = [vdupq_n_u32((t681) as u32); 2];
        let t683: [uint32x4_t; 2] = [vandq_u32(x4[0], t682[0]), vandq_u32(x4[1], t682[1])];
        let t684: u64 = 0u64;
        let t685 = [vdupq_n_u32((t684) as u32); 2];
        let t686: [uint32x4_t; 2] = [vceqq_u32(t683[0], t685[0]), vceqq_u32(t683[1], t685[1])];
        let t687: [uint32x4_t; 2] = [vandq_u32(live[0], t686[0]), vandq_u32(live[1], t686[1])];
        {
            let t688: [uint32x4_t; 4] = [x3[0], x3[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t687[0], t688[0], res[0]), vbslq_u32(t687[1], t688[1], res[1]), vbslq_u32(t687[0], t688[2], res[2]), vbslq_u32(t687[1], t688[3], res[3])];
            live = [vbicq_u32(live[0], t687[0]), vbicq_u32(live[1], t687[1])];
        }
        if any(&live) {
            let t689: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x4[0]), vshlq_n_u32::<30>(x4[2])), vorrq_u32(vshrq_n_u32::<2>(x4[1]), vshlq_n_u32::<30>(x4[3])), vshrq_n_u32::<2>(x4[2]), vshrq_n_u32::<2>(x4[3])];
            let t690: [uint32x4_t; 2] = [t689[0], t689[1]];
            x5 = t690;
            let t691: [uint32x4_t; 2] = [vsubq_u32(x5[0], t678[0]), vsubq_u32(x5[1], t678[1])];
            let t692: [uint32x4_t; 2] = [vcgeq_u32(t691[0], x2[0]), vcgeq_u32(t691[1], x2[1])];
            let t693: [uint32x4_t; 2] = [vandq_u32(live[0], t692[0]), vandq_u32(live[1], t692[1])];
            {
                let t694: u64 = 4294967295u64;
                let t695 = [vdupq_n_u32((t694) as u32); 2];
                let t696: [uint32x4_t; 4] = [t695[0], t695[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t693[0], t696[0], res[0]), vbslq_u32(t693[1], t696[1], res[1]), vbslq_u32(t693[0], t696[2], res[2]), vbslq_u32(t693[1], t696[3], res[3])];
                live = [vbicq_u32(live[0], t693[0]), vbicq_u32(live[1], t693[1])];
            }
            if any(&live) {
                let t697: [uint32x4_t; 2] = [vsubq_u32(x2[0], x5[0]), vsubq_u32(x2[1], x5[1])];
                x6 = t697;
                let t698: u64 = 2u64;
                let t699 = [vdupq_n_u32((t698) as u32); 2];
                let t700: [uint32x4_t; 2] = [vandq_u32(x4[0], t699[0]), vandq_u32(x4[1], t699[1])];
                let t701: u64 = 0u64;
                let t702 = [vdupq_n_u32((t701) as u32); 2];
                let t703: [uint32x4_t; 2] = [vceqq_u32(t700[0], t702[0]), vceqq_u32(t700[1], t702[1])];
                let t704: [uint32x4_t; 2] = [vmvnq_u32(t703[0]), vmvnq_u32(t703[1])];
                let t705: [uint32x4_t; 2] = [vandq_u32(live[0], t704[0]), vandq_u32(live[1], t704[1])];
                {
                    let t706: [uint32x4_t; 4] = [x6[0], x6[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                    res = [vbslq_u32(t705[0], t706[0], res[0]), vbslq_u32(t705[1], t706[1], res[1]), vbslq_u32(t705[0], t706[2], res[2]), vbslq_u32(t705[1], t706[3], res[3])];
                    live = [vbicq_u32(live[0], t705[0]), vbicq_u32(live[1], t705[1])];
                }
                if any(&live) {
                    let t707: [uint32x4_t; 2] = [vceqq_u32(x6[0], t672[0]), vceqq_u32(x6[1], t672[1])];
                    let t708: [uint32x4_t; 2] = [vandq_u32(live[0], t707[0]), vandq_u32(live[1], t707[1])];
                    {
                        let t709: u64 = 4294967295u64;
                        let t710 = [vdupq_n_u32((t709) as u32); 2];
                        let t711: [uint32x4_t; 4] = [t710[0], t710[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                        res = [vbslq_u32(t708[0], t711[0], res[0]), vbslq_u32(t708[1], t711[1], res[1]), vbslq_u32(t708[0], t711[2], res[2]), vbslq_u32(t708[1], t711[3], res[3])];
                        live = [vbicq_u32(live[0], t708[0]), vbicq_u32(live[1], t708[1])];
                    }
                    if any(&live) {
                        let t712: u64 = (1u64).wrapping_shl((32u64) as u32);
                        let t713 = [vdupq_n_u32((t712) as u32); 2];
                        let t714 = [vdupq_n_u32(((t712) >> 32) as u32); 2];
                        let t715: [uint32x4_t; 4] = [vorrq_u32(t713[0], x6[0]), vorrq_u32(t713[1], x6[1]), t714[0], t714[1]];
                        res = [vbslq_u32(live[0], t715[0], res[0]), vbslq_u32(live[1], t715[1], res[1]), vbslq_u32(live[0], t715[2], res[2]), vbslq_u32(live[1], t715[3], res[3])];
                        live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
                    }
                }
            }
        }
    }
    res
}

/// `ukl_sg_publish`.
#[inline(always)]
unsafe fn b9_r3gi_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg3: *const u8, x2: u32) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let t716: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x2), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x2), vld1q_u32(LANE[1].as_ptr()))];
    let t717 = [vdupq_n_u32(31u32); 2];
    let t718: [uint32x4_t; 2] = [vceqq_u32(t716[0], t717[0]), vceqq_u32(t716[1], t717[1])];
    let t719: [uint32x4_t; 2] = [vandq_u32(live[0], t718[0]), vandq_u32(live[1], t718[1])];
    {
        let t720: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t721: [uint32x4_t; 2] = [vshlq_u32(t720[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(t720[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
        let t722: [uint32x4_t; 2] = [vshlq_u32(t720[0], vnegq_s32(vdupq_n_s32(((5u32) & 31) as i32))), vshlq_u32(t720[1], vnegq_s32(vdupq_n_s32(((5u32) & 31) as i32)))];
        let t723: [uint32x4_t; 2] = [vandq_u32(t722[0], t717[0]), vandq_u32(t722[1], t717[1])];
        let t724 = [vdupq_n_u32(1u32); 2];
        let t725: [uint32x4_t; 2] = [vshlq_u32(t724[0], vreinterpretq_s32_u32(vandq_u32(t723[0], vdupq_n_u32(31)))), vshlq_u32(t724[1], vreinterpretq_s32_u32(vandq_u32(t723[1], vdupq_n_u32(31))))];
        let t726 = b13_al4r3l_p(l, lid0, t719, arg3, t721, t725);
    }
}

/// `ukl_sg_publish`.
#[inline(always)]
unsafe fn b9_r3gl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg3: *const u8, x2: [uint32x4_t; 2]) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let t727 = [vdupq_n_u32(31u32); 2];
    let t728: [uint32x4_t; 2] = [vceqq_u32(x2[0], t727[0]), vceqq_u32(x2[1], t727[1])];
    let t729: [uint32x4_t; 2] = [vandq_u32(live[0], t728[0]), vandq_u32(live[1], t728[1])];
    {
        let t730: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t731: [uint32x4_t; 2] = [vshlq_u32(t730[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(t730[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
        let t732: [uint32x4_t; 2] = [vshlq_u32(t730[0], vnegq_s32(vdupq_n_s32(((5u32) & 31) as i32))), vshlq_u32(t730[1], vnegq_s32(vdupq_n_s32(((5u32) & 31) as i32)))];
        let t733: [uint32x4_t; 2] = [vandq_u32(t732[0], t727[0]), vandq_u32(t732[1], t727[1])];
        let t734 = [vdupq_n_u32(1u32); 2];
        let t735: [uint32x4_t; 2] = [vshlq_u32(t734[0], vreinterpretq_s32_u32(vandq_u32(t733[0], vdupq_n_u32(31)))), vshlq_u32(t734[1], vreinterpretq_s32_u32(vandq_u32(t733[1], vdupq_n_u32(31))))];
        let t736 = b13_al4r3l_p(l, lid0, t729, arg3, t731, t735);
    }
}

/// `ukl_sg_published`.
#[inline(always)]
unsafe fn b10_r3ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg3: *const u8, x1: [uint32x4_t; 2], x2: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x3: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t737: [uint32x4_t; 2] = [vshlq_u32(x1[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x1[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t738 = b11_al4r3_p(l, lid0, live, arg3, t737);
    x3 = t738;
    let t739: [uint32x4_t; 2] = [vcgeq_u32(x1[0], x2[0]), vcgeq_u32(x1[1], x2[1])];
    let t740: [uint32x4_t; 2] = [vshlq_u32(x1[0], vnegq_s32(vdupq_n_s32(((5u32) & 31) as i32))), vshlq_u32(x1[1], vnegq_s32(vdupq_n_s32(((5u32) & 31) as i32)))];
    let t741 = [vdupq_n_u32(31u32); 2];
    let t742: [uint32x4_t; 2] = [vandq_u32(t740[0], t741[0]), vandq_u32(t740[1], t741[1])];
    let t743: [uint32x4_t; 2] = [vshlq_u32(x3[0], vnegq_s32(vreinterpretq_s32_u32(vandq_u32(t742[0], vdupq_n_u32(31))))), vshlq_u32(x3[1], vnegq_s32(vreinterpretq_s32_u32(vandq_u32(t742[1], vdupq_n_u32(31)))))];
    let t744 = [vdupq_n_u32(1u32); 2];
    let t745: [uint32x4_t; 2] = [vandq_u32(t743[0], t744[0]), vandq_u32(t743[1], t744[1])];
    let t746 = [vdupq_n_u32(0u32); 2];
    let t747: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t745[0], t746[0])), vmvnq_u32(vceqq_u32(t745[1], t746[1]))];
    let t748: [uint32x4_t; 2] = [vorrq_u32(t739[0], t747[0]), vorrq_u32(t739[1], t747[1])];
    res = t748;
    return res;
}

/// `ukl_atomic_load`.
#[inline(always)]
unsafe fn b11_al4r2_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg2: *const u8, x0i: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t749 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], x0i[0]), vandq_u32(live[1], x0i[1]))));
    let t750: [uint32x4_t; 2] = [vbslq_u32(live[0], x0i[0], t749), vbslq_u32(live[1], x0i[1], t749)];
    let t751 = arg2 as *const u32;
    let mut t752: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    t752[0] = vld1q_lane_u32::<0>(t751.add(vgetq_lane_u32::<0>(t750[0]) as usize), t752[0]);
    t752[0] = vld1q_lane_u32::<1>(t751.add(vgetq_lane_u32::<1>(t750[0]) as usize), t752[0]);
    t752[0] = vld1q_lane_u32::<2>(t751.add(vgetq_lane_u32::<2>(t750[0]) as usize), t752[0]);
    t752[0] = vld1q_lane_u32::<3>(t751.add(vgetq_lane_u32::<3>(t750[0]) as usize), t752[0]);
    t752[1] = vld1q_lane_u32::<0>(t751.add(vgetq_lane_u32::<0>(t750[1]) as usize), t752[1]);
    t752[1] = vld1q_lane_u32::<1>(t751.add(vgetq_lane_u32::<1>(t750[1]) as usize), t752[1]);
    t752[1] = vld1q_lane_u32::<2>(t751.add(vgetq_lane_u32::<2>(t750[1]) as usize), t752[1]);
    t752[1] = vld1q_lane_u32::<3>(t751.add(vgetq_lane_u32::<3>(t750[1]) as usize), t752[1]);
    res = t752;
    return res;
}

/// `ukl_atomic_load`.
#[inline(always)]
unsafe fn b11_al4r3_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg3: *const u8, x0i: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t753 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], x0i[0]), vandq_u32(live[1], x0i[1]))));
    let t754: [uint32x4_t; 2] = [vbslq_u32(live[0], x0i[0], t753), vbslq_u32(live[1], x0i[1], t753)];
    let t755 = arg3 as *const u32;
    let mut t756: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    t756[0] = vld1q_lane_u32::<0>(t755.add(vgetq_lane_u32::<0>(t754[0]) as usize), t756[0]);
    t756[0] = vld1q_lane_u32::<1>(t755.add(vgetq_lane_u32::<1>(t754[0]) as usize), t756[0]);
    t756[0] = vld1q_lane_u32::<2>(t755.add(vgetq_lane_u32::<2>(t754[0]) as usize), t756[0]);
    t756[0] = vld1q_lane_u32::<3>(t755.add(vgetq_lane_u32::<3>(t754[0]) as usize), t756[0]);
    t756[1] = vld1q_lane_u32::<0>(t755.add(vgetq_lane_u32::<0>(t754[1]) as usize), t756[1]);
    t756[1] = vld1q_lane_u32::<1>(t755.add(vgetq_lane_u32::<1>(t754[1]) as usize), t756[1]);
    t756[1] = vld1q_lane_u32::<2>(t755.add(vgetq_lane_u32::<2>(t754[1]) as usize), t756[1]);
    t756[1] = vld1q_lane_u32::<3>(t755.add(vgetq_lane_u32::<3>(t754[1]) as usize), t756[1]);
    res = t756;
    return res;
}

/// `clif_f16_slot`.
#[inline(always)]
unsafe fn b12_r0r1i_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t757 = b14_r0r1si_p(l, lid0, live, arg0, arg1, 0u32, x2);
    res = t757;
    return res;
}

/// `clif_f16_slot`.
#[inline(always)]
unsafe fn b12_r0r1i_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t758 = b14_r0r1si_f(l, lid0, arg0, arg1, 0u32, x2);
    res = t758;
    return res;
}

/// `clif_f16_slot`.
#[inline(always)]
unsafe fn b12_r0r1u8_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t759 = b14_r0r1su8_p(l, lid0, live, arg0, arg1, 0u32, x2);
    res = t759;
    return res;
}

/// `clif_f16_slot`.
#[inline(always)]
unsafe fn b12_r0r1u8_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t760 = b14_r0r1su8_f(l, lid0, arg0, arg1, 0u32, x2);
    res = t760;
    return res;
}

/// `clif_f16_slot`.
#[inline(always)]
unsafe fn b12_r0r1l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t761 = b14_r0r1sl_p(l, lid0, live, arg0, arg1, 0u32, x2);
    res = t761;
    return res;
}

/// `ukl_atomic_or`.
#[inline(always)]
unsafe fn b13_al4r3l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg3: *const u8, x0i: [uint32x4_t; 2], x1: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let (mut t762, mut t763, mut t764, mut t765) = ([0u32; 8], [0u32; 8], [0u32; 8], [0u32; 8]);
    for r in 0..2 { vst1q_u32(t762.as_mut_ptr().add(4 * r), live[r]); vst1q_u32(t763.as_mut_ptr().add(4 * r), x0i[r]); vst1q_u32(t764.as_mut_ptr().add(4 * r), x1[r]); }
    for j in 0..8 { if t762[j] != 0 { t765[j] = AtomicU32::from_ptr((arg3 as *mut u8).add(t763[j] as usize * 4) as *mut u32).fetch_or(t764[j], Relaxed); } }
    let t766: [uint32x4_t; 2] = [vld1q_u32(t765.as_ptr().add(0)), vld1q_u32(t765.as_ptr().add(4))];
    res = t766;
    return res;
}

/// `clif_f16_get`.
#[inline(always)]
unsafe fn b14_r0r1si_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u32 = 0;
    let mut x5: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x6: u64 = 0;
    let mut x7: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t767 = b15_r0r1s_p(l, lid0, live, arg0, arg1, x2);
    x4 = t767;
    let t768: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[1].as_ptr()))];
    let t769: [uint32x4_t; 2] = [vshlq_u32(t768[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(t768[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t770 = b16_r0r1sl_p(l, lid0, live, arg0, arg1, (1u32).wrapping_add(x2), t769);
    x5 = t770;
    let t771 = b17_s_p(l, lid0, live, x4);
    x6 = t771;
    let t772 = [vdupq_n_u32(1023u32); 2];
    let t773: [uint32x4_t; 2] = [vandq_u32(t768[0], t772[0]), vandq_u32(t768[1], t772[1])];
    let t774 = [vdupq_n_u32(x4); 2];
    let t775: [uint32x4_t; 2] = [vmulq_u32(t773[0], t774[0]), vmulq_u32(t773[1], t774[1])];
    let t776 = b18_r0r1lls_p(l, lid0, live, arg0, arg1, x5, t775, 8u32);
    let t777: u64 = x6;
    let t778 = [vdupq_n_u32((t777) as u32); 2];
    let t779 = [vdupq_n_u32(((t777) >> 32) as u32); 2];
    let t780: [uint32x4_t; 4] = [vandq_u32(t776[0], t778[0]), vandq_u32(t776[1], t778[1]), vandq_u32(t776[2], t779[0]), vandq_u32(t776[3], t779[1])];
    x7 = t780;
    let t781: u64 = x6;
    let t782 = [vdupq_n_u32((t781) as u32); 2];
    let t783 = [vdupq_n_u32(((t781) >> 32) as u32); 2];
    let t784: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x7[0], t782[0]), vceqq_u32(x7[2], t783[0])), vandq_u32(vceqq_u32(x7[1], t782[1]), vceqq_u32(x7[3], t783[1]))];
    let mut t785: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t786: [uint32x4_t; 2] = [vandq_u32(live[0], t784[0]), vandq_u32(live[1], t784[1])];
    let t787: [uint32x4_t; 2] = [vbicq_u32(live[0], t784[0]), vbicq_u32(live[1], t784[1])];
    if any(&t786) {
        let t788: [uint32x4_t; 4];
        let t789 = bits(&t786);
        if t789.count_ones() <= 2 {
            let mut t790 = [0u32; 16];
            let mut t791 = t789;
            while t791 != 0 {
                let j = t791.trailing_zeros() as usize;
                t791 &= t791 - 1;
                let t792 = b19_r0r1ss_f(l, lid0, arg0, arg1, (x2).wrapping_add(1u32), (x3).wrapping_add(j as u32));
                let t793: u64 = t792;
                t790[j] = t793 as u32;
                t790[8 + j] = (t793 >> 32) as u32;
            }
            t788 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t790);
        } else {
            let t794 = b19_r0r1si_p(l, lid0, t786, arg0, arg1, (x2).wrapping_add(1u32), x3);
            t788 = t794;
        }
        t785 = [vbslq_u32(t786[0], t788[0], t785[0]), vbslq_u32(t786[1], t788[1], t785[1]), vbslq_u32(t786[0], t788[2], t785[2]), vbslq_u32(t786[1], t788[3], t785[3])];
    }
    {
        t785 = [vbslq_u32(t787[0], x7[0], t785[0]), vbslq_u32(t787[1], x7[1], t785[1]), vbslq_u32(t787[0], x7[2], t785[2]), vbslq_u32(t787[1], x7[3], t785[3])];
    }
    res = t785;
    return res;
}

/// `clif_f16_get`.
#[inline(always)]
unsafe fn b14_r0r1si_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32, x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u32 = 0;
    let mut x5: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x6: u64 = 0;
    let mut x7: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t795 = b15_r0r1s_f(l, lid0, arg0, arg1, x2);
    x4 = t795;
    let t796: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[1].as_ptr()))];
    let t797: [uint32x4_t; 2] = [vshlq_u32(t796[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(t796[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t798 = b16_r0r1sl_f(l, lid0, arg0, arg1, (1u32).wrapping_add(x2), t797);
    x5 = t798;
    let t799 = b17_s_f(l, lid0, x4);
    x6 = t799;
    let t800 = [vdupq_n_u32(1023u32); 2];
    let t801: [uint32x4_t; 2] = [vandq_u32(t796[0], t800[0]), vandq_u32(t796[1], t800[1])];
    let t802 = [vdupq_n_u32(x4); 2];
    let t803: [uint32x4_t; 2] = [vmulq_u32(t801[0], t802[0]), vmulq_u32(t801[1], t802[1])];
    let t804 = b18_r0r1lls_f(l, lid0, arg0, arg1, x5, t803, 8u32);
    let t805: u64 = x6;
    let t806 = [vdupq_n_u32((t805) as u32); 2];
    let t807 = [vdupq_n_u32(((t805) >> 32) as u32); 2];
    let t808: [uint32x4_t; 4] = [vandq_u32(t804[0], t806[0]), vandq_u32(t804[1], t806[1]), vandq_u32(t804[2], t807[0]), vandq_u32(t804[3], t807[1])];
    x7 = t808;
    let t809: u64 = x6;
    let t810 = [vdupq_n_u32((t809) as u32); 2];
    let t811 = [vdupq_n_u32(((t809) >> 32) as u32); 2];
    let t812: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x7[0], t810[0]), vceqq_u32(x7[2], t811[0])), vandq_u32(vceqq_u32(x7[1], t810[1]), vceqq_u32(x7[3], t811[1]))];
    let mut t813: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t814: [uint32x4_t; 2] = [vmvnq_u32(t812[0]), vmvnq_u32(t812[1])];
    if any(&t812) {
        let t815: [uint32x4_t; 4];
        let t816 = bits(&t812);
        if t816.count_ones() <= 2 {
            let mut t817 = [0u32; 16];
            let mut t818 = t816;
            while t818 != 0 {
                let j = t818.trailing_zeros() as usize;
                t818 &= t818 - 1;
                let t819 = b19_r0r1ss_f(l, lid0, arg0, arg1, (x2).wrapping_add(1u32), (x3).wrapping_add(j as u32));
                let t820: u64 = t819;
                t817[j] = t820 as u32;
                t817[8 + j] = (t820 >> 32) as u32;
            }
            t815 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t817);
        } else {
            let t821 = b19_r0r1si_p(l, lid0, t812, arg0, arg1, (x2).wrapping_add(1u32), x3);
            t815 = t821;
        }
        t813 = [vbslq_u32(t812[0], t815[0], t813[0]), vbslq_u32(t812[1], t815[1], t813[1]), vbslq_u32(t812[0], t815[2], t813[2]), vbslq_u32(t812[1], t815[3], t813[3])];
    }
    {
        t813 = [vbslq_u32(t814[0], x7[0], t813[0]), vbslq_u32(t814[1], x7[1], t813[1]), vbslq_u32(t814[0], x7[2], t813[2]), vbslq_u32(t814[1], x7[3], t813[3])];
    }
    res = t813;
    return res;
}

/// `clif_f16_get`.
#[inline(always)]
unsafe fn b14_r0r1su8_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u32 = 0;
    let mut x5: u32 = 0;
    let mut x6: u64 = 0;
    let mut x7: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t822 = b15_r0r1s_p(l, lid0, live, arg0, arg1, x2);
    x4 = t822;
    let t823 = b16_r0r1ss_p(l, lid0, live, arg0, arg1, (1u32).wrapping_add(x2), (x3 >> 10));
    x5 = t823;
    let t824 = b17_s_p(l, lid0, live, x4);
    x6 = t824;
    let t825 = b18_r0r1sns_p(l, lid0, live, arg0, arg1, x5, (x3 & 1023u32).wrapping_mul(x4), x4, 8u32);
    let t826: u64 = x6;
    let t827 = [vdupq_n_u32((t826) as u32); 2];
    let t828 = [vdupq_n_u32(((t826) >> 32) as u32); 2];
    let t829: [uint32x4_t; 4] = [vandq_u32(t825[0], t827[0]), vandq_u32(t825[1], t827[1]), vandq_u32(t825[2], t828[0]), vandq_u32(t825[3], t828[1])];
    x7 = t829;
    let t830: u64 = x6;
    let t831 = [vdupq_n_u32((t830) as u32); 2];
    let t832 = [vdupq_n_u32(((t830) >> 32) as u32); 2];
    let t833: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x7[0], t831[0]), vceqq_u32(x7[2], t832[0])), vandq_u32(vceqq_u32(x7[1], t831[1]), vceqq_u32(x7[3], t832[1]))];
    let mut t834: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t835: [uint32x4_t; 2] = [vandq_u32(live[0], t833[0]), vandq_u32(live[1], t833[1])];
    let t836: [uint32x4_t; 2] = [vbicq_u32(live[0], t833[0]), vbicq_u32(live[1], t833[1])];
    if any(&t835) {
        let t837: [uint32x4_t; 4];
        let t838 = bits(&t835);
        if t838.count_ones() <= 2 {
            let mut t839 = [0u32; 16];
            let mut t840 = t838;
            while t840 != 0 {
                let j = t840.trailing_zeros() as usize;
                t840 &= t840 - 1;
                let t841 = b19_r0r1ss_f(l, lid0, arg0, arg1, (x2).wrapping_add(1u32), (x3).wrapping_add(j as u32));
                let t842: u64 = t841;
                t839[j] = t842 as u32;
                t839[8 + j] = (t842 >> 32) as u32;
            }
            t837 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t839);
        } else {
            let t843 = b19_r0r1su8_p(l, lid0, t835, arg0, arg1, (x2).wrapping_add(1u32), x3);
            t837 = t843;
        }
        t834 = [vbslq_u32(t835[0], t837[0], t834[0]), vbslq_u32(t835[1], t837[1], t834[1]), vbslq_u32(t835[0], t837[2], t834[2]), vbslq_u32(t835[1], t837[3], t834[3])];
    }
    {
        t834 = [vbslq_u32(t836[0], x7[0], t834[0]), vbslq_u32(t836[1], x7[1], t834[1]), vbslq_u32(t836[0], x7[2], t834[2]), vbslq_u32(t836[1], x7[3], t834[3])];
    }
    res = t834;
    return res;
}

/// `clif_f16_get`.
#[inline(always)]
unsafe fn b14_r0r1su8_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32, x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u32 = 0;
    let mut x5: u32 = 0;
    let mut x6: u64 = 0;
    let mut x7: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t844 = b15_r0r1s_f(l, lid0, arg0, arg1, x2);
    x4 = t844;
    let t845 = b16_r0r1ss_f(l, lid0, arg0, arg1, (1u32).wrapping_add(x2), (x3 >> 10));
    x5 = t845;
    let t846 = b17_s_f(l, lid0, x4);
    x6 = t846;
    let t847 = b18_r0r1sns_f(l, lid0, arg0, arg1, x5, (x3 & 1023u32).wrapping_mul(x4), x4, 8u32);
    let t848: u64 = x6;
    let t849 = [vdupq_n_u32((t848) as u32); 2];
    let t850 = [vdupq_n_u32(((t848) >> 32) as u32); 2];
    let t851: [uint32x4_t; 4] = [vandq_u32(t847[0], t849[0]), vandq_u32(t847[1], t849[1]), vandq_u32(t847[2], t850[0]), vandq_u32(t847[3], t850[1])];
    x7 = t851;
    let t852: u64 = x6;
    let t853 = [vdupq_n_u32((t852) as u32); 2];
    let t854 = [vdupq_n_u32(((t852) >> 32) as u32); 2];
    let t855: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x7[0], t853[0]), vceqq_u32(x7[2], t854[0])), vandq_u32(vceqq_u32(x7[1], t853[1]), vceqq_u32(x7[3], t854[1]))];
    let mut t856: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t857: [uint32x4_t; 2] = [vmvnq_u32(t855[0]), vmvnq_u32(t855[1])];
    if any(&t855) {
        let t858: [uint32x4_t; 4];
        let t859 = bits(&t855);
        if t859.count_ones() <= 2 {
            let mut t860 = [0u32; 16];
            let mut t861 = t859;
            while t861 != 0 {
                let j = t861.trailing_zeros() as usize;
                t861 &= t861 - 1;
                let t862 = b19_r0r1ss_f(l, lid0, arg0, arg1, (x2).wrapping_add(1u32), (x3).wrapping_add(j as u32));
                let t863: u64 = t862;
                t860[j] = t863 as u32;
                t860[8 + j] = (t863 >> 32) as u32;
            }
            t858 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t860);
        } else {
            let t864 = b19_r0r1su8_p(l, lid0, t855, arg0, arg1, (x2).wrapping_add(1u32), x3);
            t858 = t864;
        }
        t856 = [vbslq_u32(t855[0], t858[0], t856[0]), vbslq_u32(t855[1], t858[1], t856[1]), vbslq_u32(t855[0], t858[2], t856[2]), vbslq_u32(t855[1], t858[3], t856[3])];
    }
    {
        t856 = [vbslq_u32(t857[0], x7[0], t856[0]), vbslq_u32(t857[1], x7[1], t856[1]), vbslq_u32(t857[0], x7[2], t856[2]), vbslq_u32(t857[1], x7[3], t856[3])];
    }
    res = t856;
    return res;
}

/// `clif_f16_get`.
#[inline(always)]
unsafe fn b14_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u32 = 0;
    let mut x5: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x6: u64 = 0;
    let mut x7: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t865 = b15_r0r1s_p(l, lid0, live, arg0, arg1, x2);
    x4 = t865;
    let t866: [uint32x4_t; 2] = [vshlq_u32(x3[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x3[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t867 = b16_r0r1sl_p(l, lid0, live, arg0, arg1, (1u32).wrapping_add(x2), t866);
    x5 = t867;
    let t868 = b17_s_p(l, lid0, live, x4);
    x6 = t868;
    let t869 = [vdupq_n_u32(1023u32); 2];
    let t870: [uint32x4_t; 2] = [vandq_u32(x3[0], t869[0]), vandq_u32(x3[1], t869[1])];
    let t871 = [vdupq_n_u32(x4); 2];
    let t872: [uint32x4_t; 2] = [vmulq_u32(t870[0], t871[0]), vmulq_u32(t870[1], t871[1])];
    let t873 = b18_r0r1lls_p(l, lid0, live, arg0, arg1, x5, t872, 8u32);
    let t874: u64 = x6;
    let t875 = [vdupq_n_u32((t874) as u32); 2];
    let t876 = [vdupq_n_u32(((t874) >> 32) as u32); 2];
    let t877: [uint32x4_t; 4] = [vandq_u32(t873[0], t875[0]), vandq_u32(t873[1], t875[1]), vandq_u32(t873[2], t876[0]), vandq_u32(t873[3], t876[1])];
    x7 = t877;
    let t878: u64 = x6;
    let t879 = [vdupq_n_u32((t878) as u32); 2];
    let t880 = [vdupq_n_u32(((t878) >> 32) as u32); 2];
    let t881: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x7[0], t879[0]), vceqq_u32(x7[2], t880[0])), vandq_u32(vceqq_u32(x7[1], t879[1]), vceqq_u32(x7[3], t880[1]))];
    let mut t882: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t883: [uint32x4_t; 2] = [vandq_u32(live[0], t881[0]), vandq_u32(live[1], t881[1])];
    let t884: [uint32x4_t; 2] = [vbicq_u32(live[0], t881[0]), vbicq_u32(live[1], t881[1])];
    if any(&t883) {
        let t885: [uint32x4_t; 4];
        let t886 = bits(&t883);
        if t886.count_ones() <= 2 {
            let t887 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x3);
            let mut t888 = [0u32; 16];
            let mut t889 = t886;
            while t889 != 0 {
                let j = t889.trailing_zeros() as usize;
                t889 &= t889 - 1;
                let t890 = b19_r0r1ss_f(l, lid0, arg0, arg1, (x2).wrapping_add(1u32), t887[j]);
                let t891: u64 = t890;
                t888[j] = t891 as u32;
                t888[8 + j] = (t891 >> 32) as u32;
            }
            t885 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t888);
        } else {
            let t892 = b19_r0r1sl_p(l, lid0, t883, arg0, arg1, (x2).wrapping_add(1u32), x3);
            t885 = t892;
        }
        t882 = [vbslq_u32(t883[0], t885[0], t882[0]), vbslq_u32(t883[1], t885[1], t882[1]), vbslq_u32(t883[0], t885[2], t882[2]), vbslq_u32(t883[1], t885[3], t882[3])];
    }
    {
        t882 = [vbslq_u32(t884[0], x7[0], t882[0]), vbslq_u32(t884[1], x7[1], t882[1]), vbslq_u32(t884[0], x7[2], t882[2]), vbslq_u32(t884[1], x7[3], t882[3])];
    }
    res = t882;
    return res;
}

/// `clif_f16_width`.
#[inline(always)]
unsafe fn b15_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t893 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    res = (((t893).wrapping_shr(((4u32).wrapping_mul(x2)) as u32)) & (15u32));
    return res;
}

/// `clif_f16_width`.
#[inline(always)]
unsafe fn b15_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t894 = b20_r0r1s_f(l, lid0, arg0, arg1, 16u32);
    res = (((t894).wrapping_shr(((4u32).wrapping_mul(x2)) as u32)) & (15u32));
    return res;
}

/// `clif_f16_column`.
#[inline(always)]
unsafe fn b16_r0r1ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u32 = 0;
    let mut x5: u32 = 0;
    let mut live = k;
    let mut res: u32 = 0;
    let t895 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 40u32);
    let t896 = b21_r0r1s_p(l, lid0, live, arg0, arg1, 0u32);
    x4 = ((x3).wrapping_mul(t895)).wrapping_add(t896);
    if ((x2) == (0u32)) {
        res = x4;
        return res;
    }
    let t897 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    let t898 = b22_ss_p(l, lid0, live, (t897).wrapping_sub((x3).wrapping_shl((10u32) as u32)), 1024u32);
    x5 = t898;
    let t899 = b21_r0r1s_p(l, lid0, live, arg0, arg1, x2);
    res = (x4).wrapping_add((x5).wrapping_mul(t899));
    return res;
}

/// `clif_f16_column`.
#[inline(always)]
unsafe fn b16_r0r1ss_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32, x3: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u32 = 0;
    let mut x5: u32 = 0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t900 = b20_r0r1s_f(l, lid0, arg0, arg1, 40u32);
    let t901 = b21_r0r1s_f(l, lid0, arg0, arg1, 0u32);
    x4 = ((x3).wrapping_mul(t900)).wrapping_add(t901);
    if ((x2) == (0u32)) {
        res = x4;
        return res;
    }
    let t902 = b20_r0r1s_f(l, lid0, arg0, arg1, 4u32);
    let t903 = b22_ss_f(l, lid0, (t902).wrapping_sub((x3).wrapping_shl((10u32) as u32)), 1024u32);
    x5 = t903;
    let t904 = b21_r0r1s_f(l, lid0, arg0, arg1, x2);
    res = (x4).wrapping_add((x5).wrapping_mul(t904));
    return res;
}

/// `clif_f16_column`.
#[inline(always)]
unsafe fn b16_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x5: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t905 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 40u32);
    let t906 = [vdupq_n_u32(t905); 2];
    let t907: [uint32x4_t; 2] = [vmulq_u32(x3[0], t906[0]), vmulq_u32(x3[1], t906[1])];
    let t908 = b21_r0r1s_p(l, lid0, live, arg0, arg1, 0u32);
    let t909 = [vdupq_n_u32(t908); 2];
    let t910: [uint32x4_t; 2] = [vaddq_u32(t907[0], t909[0]), vaddq_u32(t907[1], t909[1])];
    x4 = t910;
    if ((x2) == (0u32)) {
        res = x4;
        return res;
    }
    let t911 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    let t912: [uint32x4_t; 2] = [vshlq_u32(x3[0], vdupq_n_s32(((10u32) & 31) as i32)), vshlq_u32(x3[1], vdupq_n_s32(((10u32) & 31) as i32))];
    let t913 = [vdupq_n_u32(t911); 2];
    let t914: [uint32x4_t; 2] = [vsubq_u32(t913[0], t912[0]), vsubq_u32(t913[1], t912[1])];
    let t915 = b22_ls_p(l, lid0, live, t914, 1024u32);
    x5 = t915;
    let t916 = b21_r0r1s_p(l, lid0, live, arg0, arg1, x2);
    let t917 = [vdupq_n_u32(t916); 2];
    let t918: [uint32x4_t; 2] = [vmulq_u32(x5[0], t917[0]), vmulq_u32(x5[1], t917[1])];
    let t919: [uint32x4_t; 2] = [vaddq_u32(x4[0], t918[0]), vaddq_u32(x4[1], t918[1])];
    res = t919;
    return res;
}

/// `clif_f16_column`.
#[inline(always)]
unsafe fn b16_r0r1sl_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32, x3: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x5: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t920 = b20_r0r1s_f(l, lid0, arg0, arg1, 40u32);
    let t921 = [vdupq_n_u32(t920); 2];
    let t922: [uint32x4_t; 2] = [vmulq_u32(x3[0], t921[0]), vmulq_u32(x3[1], t921[1])];
    let t923 = b21_r0r1s_f(l, lid0, arg0, arg1, 0u32);
    let t924 = [vdupq_n_u32(t923); 2];
    let t925: [uint32x4_t; 2] = [vaddq_u32(t922[0], t924[0]), vaddq_u32(t922[1], t924[1])];
    x4 = t925;
    if ((x2) == (0u32)) {
        res = x4;
        return res;
    }
    let t926 = b20_r0r1s_f(l, lid0, arg0, arg1, 4u32);
    let t927: [uint32x4_t; 2] = [vshlq_u32(x3[0], vdupq_n_s32(((10u32) & 31) as i32)), vshlq_u32(x3[1], vdupq_n_s32(((10u32) & 31) as i32))];
    let t928 = [vdupq_n_u32(t926); 2];
    let t929: [uint32x4_t; 2] = [vsubq_u32(t928[0], t927[0]), vsubq_u32(t928[1], t927[1])];
    let t930 = b22_ls_f(l, lid0, t929, 1024u32);
    x5 = t930;
    let t931 = b21_r0r1s_f(l, lid0, arg0, arg1, x2);
    let t932 = [vdupq_n_u32(t931); 2];
    let t933: [uint32x4_t; 2] = [vmulq_u32(x5[0], t932[0]), vmulq_u32(x5[1], t932[1])];
    let t934: [uint32x4_t; 2] = [vaddq_u32(x4[0], t933[0]), vaddq_u32(x4[1], t933[1])];
    res = t934;
    return res;
}

/// `clif_t_mask`.
#[inline(always)]
unsafe fn b17_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t935: u64;
    if ((x0) >= (8u32)) {
        t935 = !(0u64);
    } else {
        t935 = ((1u64).wrapping_shl(((((x0).wrapping_mul(8u32)) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t935;
    return res;
}

/// `clif_t_mask`.
#[inline(always)]
unsafe fn b17_s_f(l: &Launch, lid0: u32, x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t936: u64;
    if ((x0) >= (8u32)) {
        t936 = !(0u64);
    } else {
        t936 = ((1u64).wrapping_shl(((((x0).wrapping_mul(8u32)) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t936;
    return res;
}

/// `clif_f16_ld`.
#[inline(always)]
unsafe fn b18_r0r1sns_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3f: u32, x3s: u32, x4: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t937 = b23_r0sns_p(l, lid0, live, arg0, x2, x3f, x3s, x4);
    res = t937;
    return res;
}

/// `clif_f16_ld`.
#[inline(always)]
unsafe fn b18_r0r1sns_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32, x3f: u32, x3s: u32, x4: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t938 = b23_r0sns_f(l, lid0, arg0, x2, x3f, x3s, x4);
    res = t938;
    return res;
}

/// `clif_f16_ld`.
#[inline(always)]
unsafe fn b18_r0r1lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: [uint32x4_t; 2], x3: [uint32x4_t; 2], x4: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t939 = b23_r0lls_p(l, lid0, live, arg0, x2, x3, x4);
    res = t939;
    return res;
}

/// `clif_f16_ld`.
#[inline(always)]
unsafe fn b18_r0r1lls_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: [uint32x4_t; 2], x3: [uint32x4_t; 2], x4: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t940 = b23_r0lls_f(l, lid0, arg0, x2, x3, x4);
    res = t940;
    return res;
}

/// `clif_f16_side`.
#[inline(always)]
unsafe fn b19_r0r1ss_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32, x3: u32) -> u64 {
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
    let t941 = b20_r0r1s_f(l, lid0, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x2)));
    x4 = t941;
    let t942 = b20_r0r1s_f(l, lid0, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x2)));
    x5 = t942;
    x6 = (x3).wrapping_shr((10u32) as u32);
    let t943 = b24_r0r1sss_f(l, lid0, arg0, arg1, x5, (x6).wrapping_mul(4u32), 4u32);
    x7 = ((t943) as u32);
    let t944 = b24_r0r1sss_f(l, lid0, arg0, arg1, x5, ((x6).wrapping_mul(4u32)).wrapping_add(4u32), 4u32);
    x8 = ((t944) as u32);
    let t945 = b20_r0r1s_f(l, lid0, arg0, arg1, 4u32);
    x9 = ((t945).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
    let t946 = b25_ss_f(l, lid0, 4u32, (x9).wrapping_sub((x6).wrapping_mul(4u32)));
    x10 = t946;
    x11 = (((x3).wrapping_shr((8u32) as u32)) & (3u32));
    let t947: u32;
    if ((x11) == (0u32)) {
        t947 = 0u32;
    } else {
        let t948 = b26_ss_f(l, lid0, (x7).wrapping_add((2u32).wrapping_mul((x11).wrapping_sub(1u32))), x8);
        let t949 = b24_r0r1sss_f(l, lid0, arg0, arg1, x4, t948, 2u32);
        t947 = ((t949) as u32);
    }
    x12 = t947;
    let t950 = b26_ss_f(l, lid0, (x7).wrapping_add((2u32).wrapping_mul(x11)), x8);
    let t951 = b24_r0r1sss_f(l, lid0, arg0, arg1, x4, t950, 2u32);
    x13 = ((t951) as u32);
    x14 = (x7).wrapping_add((2u32).wrapping_mul(x10));
    x15 = ((x3) & (255u32));
    x16 = 9259542123273814144u64;
    x17 = (((x15) as u64)).wrapping_mul(72340172838076673u64);
    x18 = x12;
    x19 = x12;
    loop {
        // Rounds of one condition: every lane goes on.
        if !(((x19) < (x13))) { break; }
        let t952 = b26_ss_f(l, lid0, (x14).wrapping_add(x19), x8);
        let t953 = b24_r0r1sss_f(l, lid0, arg0, arg1, x4, t952, 8u32);
        x20 = t953;
        x21 = (((x20) | (x16))).wrapping_sub(((x17) & (!(x16))));
        x22 = ((x16) & (((((!(x20)) & (x17))) | (((!(((x20) ^ (x17)))) & (!(x21)))))));
        x23 = (x13).wrapping_sub(x19);
        let t954: u64;
        if ((x23) >= (8u32)) {
            t954 = x22;
        } else {
            t954 = ((x22) & (((1u64).wrapping_shl(((((8u32).wrapping_mul(x23)) as u64)) as u32)).wrapping_sub(1u64)));
        }
        let t955 = b27_s_f(l, lid0, t954);
        x18 = (x18).wrapping_add(t955);
        x19 = (x19).wrapping_add(8u32);
    }
    x12 = x18;
    let t956 = b20_r0r1s_f(l, lid0, arg0, arg1, 16u32);
    x24 = (((t956).wrapping_shr((((((4u32).wrapping_mul(x2)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32));
    let t957 = b26_ss_f(l, lid0, (x7).wrapping_add((2u32).wrapping_mul((x10).wrapping_sub(1u32))), x8);
    let t958 = b24_r0r1sss_f(l, lid0, arg0, arg1, x4, t957, 2u32);
    x25 = ((t958) as u32);
    let t959 = b26_ss_f(l, lid0, ((x14).wrapping_add(x25)).wrapping_add((x12).wrapping_mul(x24)), x8);
    x26 = t959;
    let t960 = b24_r0r1sss_f(l, lid0, arg0, arg1, x4, x26, 8u32);
    let t961 = b17_s_f(l, lid0, x24);
    res = ((t960) & (t961));
    return res;
}

/// `clif_f16_side`.
#[inline(always)]
unsafe fn b19_r0r1si_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: u32) -> [uint32x4_t; 4] {
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
    let t962 = b20_r0r1s_p(l, lid0, live, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x2)));
    x4 = t962;
    let t963 = b20_r0r1s_p(l, lid0, live, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x2)));
    x5 = t963;
    let t964: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[1].as_ptr()))];
    let t965: [uint32x4_t; 2] = [vshlq_u32(t964[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(t964[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    x6 = t965;
    let t966 = [vdupq_n_u32(4u32); 2];
    let t967: [uint32x4_t; 2] = [vmulq_u32(x6[0], t966[0]), vmulq_u32(x6[1], t966[1])];
    let t968 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x5, t967, 4u32);
    let t969: [uint32x4_t; 2] = [t968[0], t968[1]];
    x7 = t969;
    let t970: [uint32x4_t; 2] = [vmulq_u32(x6[0], t966[0]), vmulq_u32(x6[1], t966[1])];
    let t971: [uint32x4_t; 2] = [vaddq_u32(t970[0], t966[0]), vaddq_u32(t970[1], t966[1])];
    let t972 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x5, t971, 4u32);
    let t973: [uint32x4_t; 2] = [t972[0], t972[1]];
    x8 = t973;
    let t974 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    x9 = ((t974).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
    let t975: [uint32x4_t; 2] = [vmulq_u32(x6[0], t966[0]), vmulq_u32(x6[1], t966[1])];
    let t976 = [vdupq_n_u32(x9); 2];
    let t977: [uint32x4_t; 2] = [vsubq_u32(t976[0], t975[0]), vsubq_u32(t976[1], t975[1])];
    let t978 = b25_sl_p(l, lid0, live, 4u32, t977);
    x10 = t978;
    let t979: [uint32x4_t; 2] = [vshlq_u32(t964[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(t964[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
    let t980 = [vdupq_n_u32(3u32); 2];
    let t981: [uint32x4_t; 2] = [vandq_u32(t979[0], t980[0]), vandq_u32(t979[1], t980[1])];
    x11 = t981;
    let t982 = [vdupq_n_u32(0u32); 2];
    let t983: [uint32x4_t; 2] = [vceqq_u32(x11[0], t982[0]), vceqq_u32(x11[1], t982[1])];
    let mut t984: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t985: [uint32x4_t; 2] = [vandq_u32(live[0], t983[0]), vandq_u32(live[1], t983[1])];
    let t986: [uint32x4_t; 2] = [vbicq_u32(live[0], t983[0]), vbicq_u32(live[1], t983[1])];
    {
        t984 = [vbslq_u32(t985[0], t982[0], t984[0]), vbslq_u32(t985[1], t982[1], t984[1])];
    }
    if any(&t986) {
        let t987 = [vdupq_n_u32(1u32); 2];
        let t988: [uint32x4_t; 2] = [vsubq_u32(x11[0], t987[0]), vsubq_u32(x11[1], t987[1])];
        let t989 = [vdupq_n_u32(2u32); 2];
        let t990: [uint32x4_t; 2] = [vmulq_u32(t989[0], t988[0]), vmulq_u32(t989[1], t988[1])];
        let t991: [uint32x4_t; 2] = [vaddq_u32(x7[0], t990[0]), vaddq_u32(x7[1], t990[1])];
        let t992 = b26_ll_p(l, lid0, t986, t991, x8);
        let t993 = b24_r0r1sls_p(l, lid0, t986, arg0, arg1, x4, t992, 2u32);
        let t994: [uint32x4_t; 2] = [t993[0], t993[1]];
        t984 = [vbslq_u32(t986[0], t994[0], t984[0]), vbslq_u32(t986[1], t994[1], t984[1])];
    }
    x12 = t984;
    let t995 = [vdupq_n_u32(2u32); 2];
    let t996: [uint32x4_t; 2] = [vmulq_u32(t995[0], x11[0]), vmulq_u32(t995[1], x11[1])];
    let t997: [uint32x4_t; 2] = [vaddq_u32(x7[0], t996[0]), vaddq_u32(x7[1], t996[1])];
    let t998 = b26_ll_p(l, lid0, live, t997, x8);
    let t999 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x4, t998, 2u32);
    let t1000: [uint32x4_t; 2] = [t999[0], t999[1]];
    x13 = t1000;
    let t1001: [uint32x4_t; 2] = [vmulq_u32(t995[0], x10[0]), vmulq_u32(t995[1], x10[1])];
    let t1002: [uint32x4_t; 2] = [vaddq_u32(x7[0], t1001[0]), vaddq_u32(x7[1], t1001[1])];
    x14 = t1002;
    let t1003 = [vdupq_n_u32(255u32); 2];
    let t1004: [uint32x4_t; 2] = [vandq_u32(t964[0], t1003[0]), vandq_u32(t964[1], t1003[1])];
    x15 = t1004;
    x16 = 9259542123273814144u64;
    let t1005: u64 = 72340172838076673u64;
    let t1006 = [vdupq_n_u32((t1005) as u32); 2];
    let t1007 = [vdupq_n_u32(((t1005) >> 32) as u32); 2];
    let t1008: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x15[0]), vget_low_u32(t1006[0]))), vreinterpretq_u32_u64(vmull_high_u32(x15[0], t1006[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x15[1]), vget_low_u32(t1006[1]))), vreinterpretq_u32_u64(vmull_high_u32(x15[1], t1006[1])))];
    let t1009: [uint32x4_t; 4] = [vmulq_u32(x15[0], t1006[0]), vmulq_u32(x15[1], t1006[1]), vmlaq_u32(t1008[0], x15[0], t1007[0]), vmlaq_u32(t1008[1], x15[1], t1007[1])];
    x17 = t1009;
    x18 = x12;
    x19 = x12;
    let mut t1011 = live;
    loop {
        // Lanes part: each its own rounds.
        let t1013: [uint32x4_t; 2] = [vcltq_u32(x19[0], x13[0]), vcltq_u32(x19[1], x13[1])];
        t1011 = [vandq_u32(t1011[0], t1013[0]), vandq_u32(t1011[1], t1013[1])];
        if !(any(&t1011)) { break; }
        let t1015: [uint32x4_t; 2] = [vaddq_u32(x14[0], x19[0]), vaddq_u32(x14[1], x19[1])];
        let t1016 = b26_ll_p(l, lid0, t1011, t1015, x8);
        let t1017 = b24_r0r1sls_p(l, lid0, t1011, arg0, arg1, x4, t1016, 8u32);
        x20 = t1017;
        let t1018: u64 = x16;
        let t1019 = [vdupq_n_u32((t1018) as u32); 2];
        let t1020 = [vdupq_n_u32(((t1018) >> 32) as u32); 2];
        let t1021: [uint32x4_t; 4] = [vorrq_u32(x20[0], t1019[0]), vorrq_u32(x20[1], t1019[1]), vorrq_u32(x20[2], t1020[0]), vorrq_u32(x20[3], t1020[1])];
        let t1022: u64 = !(x16);
        let t1023 = [vdupq_n_u32((t1022) as u32); 2];
        let t1024 = [vdupq_n_u32(((t1022) >> 32) as u32); 2];
        let t1025: [uint32x4_t; 4] = [vandq_u32(x17[0], t1023[0]), vandq_u32(x17[1], t1023[1]), vandq_u32(x17[2], t1024[0]), vandq_u32(x17[3], t1024[1])];
        let t1026: [uint32x4_t; 2] = [vcltq_u32(t1021[0], t1025[0]), vcltq_u32(t1021[1], t1025[1])];
        let t1027: [uint32x4_t; 4] = [vsubq_u32(t1021[0], t1025[0]), vsubq_u32(t1021[1], t1025[1]), vaddq_u32(vsubq_u32(t1021[2], t1025[2]), t1026[0]), vaddq_u32(vsubq_u32(t1021[3], t1025[3]), t1026[1])];
        x21 = t1027;
        let t1028: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x20[r]));
        let t1029: [uint32x4_t; 4] = [vandq_u32(t1028[0], x17[0]), vandq_u32(t1028[1], x17[1]), vandq_u32(t1028[2], x17[2]), vandq_u32(t1028[3], x17[3])];
        let t1030: [uint32x4_t; 4] = [veorq_u32(x20[0], x17[0]), veorq_u32(x20[1], x17[1]), veorq_u32(x20[2], x17[2]), veorq_u32(x20[3], x17[3])];
        let t1031: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(t1030[r]));
        let t1032: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x21[r]));
        let t1033: [uint32x4_t; 4] = [vandq_u32(t1031[0], t1032[0]), vandq_u32(t1031[1], t1032[1]), vandq_u32(t1031[2], t1032[2]), vandq_u32(t1031[3], t1032[3])];
        let t1034: [uint32x4_t; 4] = [vorrq_u32(t1029[0], t1033[0]), vorrq_u32(t1029[1], t1033[1]), vorrq_u32(t1029[2], t1033[2]), vorrq_u32(t1029[3], t1033[3])];
        let t1035: u64 = x16;
        let t1036 = [vdupq_n_u32((t1035) as u32); 2];
        let t1037 = [vdupq_n_u32(((t1035) >> 32) as u32); 2];
        let t1038: [uint32x4_t; 4] = [vandq_u32(t1036[0], t1034[0]), vandq_u32(t1036[1], t1034[1]), vandq_u32(t1037[0], t1034[2]), vandq_u32(t1037[1], t1034[3])];
        x22 = t1038;
        let t1039: [uint32x4_t; 2] = [vsubq_u32(x13[0], x19[0]), vsubq_u32(x13[1], x19[1])];
        x23 = t1039;
        let t1040 = [vdupq_n_u32(8u32); 2];
        let t1041: [uint32x4_t; 2] = [vcgeq_u32(x23[0], t1040[0]), vcgeq_u32(x23[1], t1040[1])];
        let mut t1042: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t1043: [uint32x4_t; 2] = [vandq_u32(t1011[0], t1041[0]), vandq_u32(t1011[1], t1041[1])];
        let t1044: [uint32x4_t; 2] = [vbicq_u32(t1011[0], t1041[0]), vbicq_u32(t1011[1], t1041[1])];
        {
            t1042 = [vbslq_u32(t1043[0], x22[0], t1042[0]), vbslq_u32(t1043[1], x22[1], t1042[1]), vbslq_u32(t1043[0], x22[2], t1042[2]), vbslq_u32(t1043[1], x22[3], t1042[3])];
        }
        {
            let t1045: [uint32x4_t; 2] = [vmulq_u32(t1040[0], x23[0]), vmulq_u32(t1040[1], x23[1])];
            let t1046: u64 = 1u64;
            let t1047 = [vdupq_n_u32((t1046) as u32); 2];
            let t1048: [uint32x4_t; 2] = [t1045[0], t1045[1]];
            let t1049: [uint32x4_t; 2] = [vandq_u32(t1048[0], vdupq_n_u32(63)), vandq_u32(t1048[1], vdupq_n_u32(63))];
            let t1050: [uint32x4_t; 4] = [vshlq_u32(t1047[0], vreinterpretq_s32_u32(t1049[0])), vshlq_u32(t1047[1], vreinterpretq_s32_u32(t1049[1])), vshlq_u32(t1047[0], vsubq_s32(vreinterpretq_s32_u32(t1049[0]), vdupq_n_s32(32))), vshlq_u32(t1047[1], vsubq_s32(vreinterpretq_s32_u32(t1049[1]), vdupq_n_s32(32)))];
            let t1051: u64 = 1u64;
            let t1052 = [vdupq_n_u32((t1051) as u32); 2];
            let t1053: [uint32x4_t; 2] = [vcltq_u32(t1050[0], t1052[0]), vcltq_u32(t1050[1], t1052[1])];
            let t1054: [uint32x4_t; 4] = [vsubq_u32(t1050[0], t1052[0]), vsubq_u32(t1050[1], t1052[1]), vaddq_u32(vsubq_u32(t1050[2], vdupq_n_u32(0)), t1053[0]), vaddq_u32(vsubq_u32(t1050[3], vdupq_n_u32(0)), t1053[1])];
            let t1055: [uint32x4_t; 4] = [vandq_u32(x22[0], t1054[0]), vandq_u32(x22[1], t1054[1]), vandq_u32(x22[2], t1054[2]), vandq_u32(x22[3], t1054[3])];
            t1042 = [vbslq_u32(t1044[0], t1055[0], t1042[0]), vbslq_u32(t1044[1], t1055[1], t1042[1]), vbslq_u32(t1044[0], t1055[2], t1042[2]), vbslq_u32(t1044[1], t1055[3], t1042[3])];
        }
        let t1056 = b27_l_p(l, lid0, t1011, t1042);
        let t1057: [uint32x4_t; 2] = [vaddq_u32(x18[0], t1056[0]), vaddq_u32(x18[1], t1056[1])];
        x18 = [vbslq_u32(t1011[0], t1057[0], x18[0]), vbslq_u32(t1011[1], t1057[1], x18[1])];
        let t1058: [uint32x4_t; 2] = [vandq_u32(t1011[0], live[0]), vandq_u32(t1011[1], live[1])];
        let t1059: [uint32x4_t; 2] = [vaddq_u32(x19[0], t1040[0]), vaddq_u32(x19[1], t1040[1])];
        x19 = t1059;
    }
    x12 = x18;
    let t1060 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    x24 = (((t1060).wrapping_shr((((((4u32).wrapping_mul(x2)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32));
    let t1061 = [vdupq_n_u32(1u32); 2];
    let t1062: [uint32x4_t; 2] = [vsubq_u32(x10[0], t1061[0]), vsubq_u32(x10[1], t1061[1])];
    let t1063: [uint32x4_t; 2] = [vmulq_u32(t995[0], t1062[0]), vmulq_u32(t995[1], t1062[1])];
    let t1064: [uint32x4_t; 2] = [vaddq_u32(x7[0], t1063[0]), vaddq_u32(x7[1], t1063[1])];
    let t1065 = b26_ll_p(l, lid0, live, t1064, x8);
    let t1066 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x4, t1065, 2u32);
    let t1067: [uint32x4_t; 2] = [t1066[0], t1066[1]];
    x25 = t1067;
    let t1068: [uint32x4_t; 2] = [vaddq_u32(x14[0], x25[0]), vaddq_u32(x14[1], x25[1])];
    let t1069 = [vdupq_n_u32(x24); 2];
    let t1070: [uint32x4_t; 2] = [vmulq_u32(x12[0], t1069[0]), vmulq_u32(x12[1], t1069[1])];
    let t1071: [uint32x4_t; 2] = [vaddq_u32(t1068[0], t1070[0]), vaddq_u32(t1068[1], t1070[1])];
    let t1072 = b26_ll_p(l, lid0, live, t1071, x8);
    x26 = t1072;
    let t1073 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x4, x26, 8u32);
    let t1074 = b17_s_p(l, lid0, live, x24);
    let t1075: u64 = t1074;
    let t1076 = [vdupq_n_u32((t1075) as u32); 2];
    let t1077 = [vdupq_n_u32(((t1075) >> 32) as u32); 2];
    let t1078: [uint32x4_t; 4] = [vandq_u32(t1073[0], t1076[0]), vandq_u32(t1073[1], t1076[1]), vandq_u32(t1073[2], t1077[0]), vandq_u32(t1073[3], t1077[1])];
    res = t1078;
    return res;
}

/// `clif_f16_side`.
#[inline(always)]
unsafe fn b19_r0r1su8_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: u32) -> [uint32x4_t; 4] {
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
    let t1079 = b20_r0r1s_p(l, lid0, live, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x2)));
    x4 = t1079;
    let t1080 = b20_r0r1s_p(l, lid0, live, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x2)));
    x5 = t1080;
    x6 = (x3 >> 10);
    let t1081 = b24_r0r1sss_p(l, lid0, live, arg0, arg1, x5, (x6).wrapping_mul(4u32), 4u32);
    x7 = ((t1081) as u32);
    let t1082 = b24_r0r1sss_p(l, lid0, live, arg0, arg1, x5, ((x6).wrapping_mul(4u32)).wrapping_add(4u32), 4u32);
    x8 = ((t1082) as u32);
    let t1083 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    x9 = ((t1083).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
    let t1084 = b25_ss_p(l, lid0, live, 4u32, (x9).wrapping_sub((x6).wrapping_mul(4u32)));
    x10 = t1084;
    x11 = (((x3 >> 8)) & (3u32));
    let t1085: u32;
    if ((x11) == (0u32)) {
        t1085 = 0u32;
    } else {
        let t1086 = b26_ss_p(l, lid0, live, (x7).wrapping_add((2u32).wrapping_mul((x11).wrapping_sub(1u32))), x8);
        let t1087 = b24_r0r1sss_p(l, lid0, live, arg0, arg1, x4, t1086, 2u32);
        t1085 = ((t1087) as u32);
    }
    let t1088 = [vdupq_n_u32(t1085); 2];
    x12 = t1088;
    let t1089 = b26_ss_p(l, lid0, live, (x7).wrapping_add((2u32).wrapping_mul(x11)), x8);
    let t1090 = b24_r0r1sss_p(l, lid0, live, arg0, arg1, x4, t1089, 2u32);
    x13 = ((t1090) as u32);
    x14 = (x7).wrapping_add((2u32).wrapping_mul(x10));
    x15 = (x3 & 255u32);
    x16 = 9259542123273814144u64;
    let t1091: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x15), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x15), vld1q_u32(LANE[1].as_ptr()))];
    let t1092: u64 = 72340172838076673u64;
    let t1093 = [vdupq_n_u32((t1092) as u32); 2];
    let t1094 = [vdupq_n_u32(((t1092) >> 32) as u32); 2];
    let t1095: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(t1091[0]), vget_low_u32(t1093[0]))), vreinterpretq_u32_u64(vmull_high_u32(t1091[0], t1093[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(t1091[1]), vget_low_u32(t1093[1]))), vreinterpretq_u32_u64(vmull_high_u32(t1091[1], t1093[1])))];
    let t1096: [uint32x4_t; 4] = [vmulq_u32(t1091[0], t1093[0]), vmulq_u32(t1091[1], t1093[1]), vmlaq_u32(t1095[0], t1091[0], t1094[0]), vmlaq_u32(t1095[1], t1091[1], t1094[1])];
    x17 = t1096;
    x18 = x12;
    x19 = x12;
    let mut t1099 = live;
    loop {
        // Lanes part: each its own rounds.
        let t1101 = [vdupq_n_u32(x13); 2];
        let t1102: [uint32x4_t; 2] = [vcltq_u32(x19[0], t1101[0]), vcltq_u32(x19[1], t1101[1])];
        t1099 = [vandq_u32(t1099[0], t1102[0]), vandq_u32(t1099[1], t1102[1])];
        if !(any(&t1099)) { break; }
        let t1104 = [vdupq_n_u32(x14); 2];
        let t1105: [uint32x4_t; 2] = [vaddq_u32(t1104[0], x19[0]), vaddq_u32(t1104[1], x19[1])];
        let t1106 = b26_ls_p(l, lid0, t1099, t1105, x8);
        let t1107 = b24_r0r1sls_p(l, lid0, t1099, arg0, arg1, x4, t1106, 8u32);
        x20 = t1107;
        let t1108: u64 = x16;
        let t1109 = [vdupq_n_u32((t1108) as u32); 2];
        let t1110 = [vdupq_n_u32(((t1108) >> 32) as u32); 2];
        let t1111: [uint32x4_t; 4] = [vorrq_u32(x20[0], t1109[0]), vorrq_u32(x20[1], t1109[1]), vorrq_u32(x20[2], t1110[0]), vorrq_u32(x20[3], t1110[1])];
        let t1112: u64 = !(x16);
        let t1113 = [vdupq_n_u32((t1112) as u32); 2];
        let t1114 = [vdupq_n_u32(((t1112) >> 32) as u32); 2];
        let t1115: [uint32x4_t; 4] = [vandq_u32(x17[0], t1113[0]), vandq_u32(x17[1], t1113[1]), vandq_u32(x17[2], t1114[0]), vandq_u32(x17[3], t1114[1])];
        let t1116: [uint32x4_t; 2] = [vcltq_u32(t1111[0], t1115[0]), vcltq_u32(t1111[1], t1115[1])];
        let t1117: [uint32x4_t; 4] = [vsubq_u32(t1111[0], t1115[0]), vsubq_u32(t1111[1], t1115[1]), vaddq_u32(vsubq_u32(t1111[2], t1115[2]), t1116[0]), vaddq_u32(vsubq_u32(t1111[3], t1115[3]), t1116[1])];
        x21 = t1117;
        let t1118: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x20[r]));
        let t1119: [uint32x4_t; 4] = [vandq_u32(t1118[0], x17[0]), vandq_u32(t1118[1], x17[1]), vandq_u32(t1118[2], x17[2]), vandq_u32(t1118[3], x17[3])];
        let t1120: [uint32x4_t; 4] = [veorq_u32(x20[0], x17[0]), veorq_u32(x20[1], x17[1]), veorq_u32(x20[2], x17[2]), veorq_u32(x20[3], x17[3])];
        let t1121: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(t1120[r]));
        let t1122: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x21[r]));
        let t1123: [uint32x4_t; 4] = [vandq_u32(t1121[0], t1122[0]), vandq_u32(t1121[1], t1122[1]), vandq_u32(t1121[2], t1122[2]), vandq_u32(t1121[3], t1122[3])];
        let t1124: [uint32x4_t; 4] = [vorrq_u32(t1119[0], t1123[0]), vorrq_u32(t1119[1], t1123[1]), vorrq_u32(t1119[2], t1123[2]), vorrq_u32(t1119[3], t1123[3])];
        let t1125: u64 = x16;
        let t1126 = [vdupq_n_u32((t1125) as u32); 2];
        let t1127 = [vdupq_n_u32(((t1125) >> 32) as u32); 2];
        let t1128: [uint32x4_t; 4] = [vandq_u32(t1126[0], t1124[0]), vandq_u32(t1126[1], t1124[1]), vandq_u32(t1127[0], t1124[2]), vandq_u32(t1127[1], t1124[3])];
        x22 = t1128;
        let t1129 = [vdupq_n_u32(x13); 2];
        let t1130: [uint32x4_t; 2] = [vsubq_u32(t1129[0], x19[0]), vsubq_u32(t1129[1], x19[1])];
        x23 = t1130;
        let t1131 = [vdupq_n_u32(8u32); 2];
        let t1132: [uint32x4_t; 2] = [vcgeq_u32(x23[0], t1131[0]), vcgeq_u32(x23[1], t1131[1])];
        let mut t1133: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t1134: [uint32x4_t; 2] = [vandq_u32(t1099[0], t1132[0]), vandq_u32(t1099[1], t1132[1])];
        let t1135: [uint32x4_t; 2] = [vbicq_u32(t1099[0], t1132[0]), vbicq_u32(t1099[1], t1132[1])];
        {
            t1133 = [vbslq_u32(t1134[0], x22[0], t1133[0]), vbslq_u32(t1134[1], x22[1], t1133[1]), vbslq_u32(t1134[0], x22[2], t1133[2]), vbslq_u32(t1134[1], x22[3], t1133[3])];
        }
        {
            let t1136: [uint32x4_t; 2] = [vmulq_u32(t1131[0], x23[0]), vmulq_u32(t1131[1], x23[1])];
            let t1137: u64 = 1u64;
            let t1138 = [vdupq_n_u32((t1137) as u32); 2];
            let t1139: [uint32x4_t; 2] = [t1136[0], t1136[1]];
            let t1140: [uint32x4_t; 2] = [vandq_u32(t1139[0], vdupq_n_u32(63)), vandq_u32(t1139[1], vdupq_n_u32(63))];
            let t1141: [uint32x4_t; 4] = [vshlq_u32(t1138[0], vreinterpretq_s32_u32(t1140[0])), vshlq_u32(t1138[1], vreinterpretq_s32_u32(t1140[1])), vshlq_u32(t1138[0], vsubq_s32(vreinterpretq_s32_u32(t1140[0]), vdupq_n_s32(32))), vshlq_u32(t1138[1], vsubq_s32(vreinterpretq_s32_u32(t1140[1]), vdupq_n_s32(32)))];
            let t1142: u64 = 1u64;
            let t1143 = [vdupq_n_u32((t1142) as u32); 2];
            let t1144: [uint32x4_t; 2] = [vcltq_u32(t1141[0], t1143[0]), vcltq_u32(t1141[1], t1143[1])];
            let t1145: [uint32x4_t; 4] = [vsubq_u32(t1141[0], t1143[0]), vsubq_u32(t1141[1], t1143[1]), vaddq_u32(vsubq_u32(t1141[2], vdupq_n_u32(0)), t1144[0]), vaddq_u32(vsubq_u32(t1141[3], vdupq_n_u32(0)), t1144[1])];
            let t1146: [uint32x4_t; 4] = [vandq_u32(x22[0], t1145[0]), vandq_u32(x22[1], t1145[1]), vandq_u32(x22[2], t1145[2]), vandq_u32(x22[3], t1145[3])];
            t1133 = [vbslq_u32(t1135[0], t1146[0], t1133[0]), vbslq_u32(t1135[1], t1146[1], t1133[1]), vbslq_u32(t1135[0], t1146[2], t1133[2]), vbslq_u32(t1135[1], t1146[3], t1133[3])];
        }
        let t1147 = b27_l_p(l, lid0, t1099, t1133);
        let t1148: [uint32x4_t; 2] = [vaddq_u32(x18[0], t1147[0]), vaddq_u32(x18[1], t1147[1])];
        x18 = [vbslq_u32(t1099[0], t1148[0], x18[0]), vbslq_u32(t1099[1], t1148[1], x18[1])];
        let t1149: [uint32x4_t; 2] = [vandq_u32(t1099[0], live[0]), vandq_u32(t1099[1], live[1])];
        let t1150: [uint32x4_t; 2] = [vaddq_u32(x19[0], t1131[0]), vaddq_u32(x19[1], t1131[1])];
        x19 = t1150;
    }
    x12 = x18;
    let t1151 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    x24 = (((t1151).wrapping_shr((((((4u32).wrapping_mul(x2)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32));
    let t1152 = b26_ss_p(l, lid0, live, (x7).wrapping_add((2u32).wrapping_mul((x10).wrapping_sub(1u32))), x8);
    let t1153 = b24_r0r1sss_p(l, lid0, live, arg0, arg1, x4, t1152, 2u32);
    x25 = ((t1153) as u32);
    let t1154 = [vdupq_n_u32(x24); 2];
    let t1155: [uint32x4_t; 2] = [vmulq_u32(x12[0], t1154[0]), vmulq_u32(x12[1], t1154[1])];
    let t1156 = [vdupq_n_u32((x14).wrapping_add(x25)); 2];
    let t1157: [uint32x4_t; 2] = [vaddq_u32(t1156[0], t1155[0]), vaddq_u32(t1156[1], t1155[1])];
    let t1158 = b26_ls_p(l, lid0, live, t1157, x8);
    x26 = t1158;
    let t1159 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x4, x26, 8u32);
    let t1160 = b17_s_p(l, lid0, live, x24);
    let t1161: u64 = t1160;
    let t1162 = [vdupq_n_u32((t1161) as u32); 2];
    let t1163 = [vdupq_n_u32(((t1161) >> 32) as u32); 2];
    let t1164: [uint32x4_t; 4] = [vandq_u32(t1159[0], t1162[0]), vandq_u32(t1159[1], t1162[1]), vandq_u32(t1159[2], t1163[0]), vandq_u32(t1159[3], t1163[1])];
    res = t1164;
    return res;
}

/// `clif_f16_side`.
#[inline(always)]
unsafe fn b19_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
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
    let t1165 = b20_r0r1s_p(l, lid0, live, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x2)));
    x4 = t1165;
    let t1166 = b20_r0r1s_p(l, lid0, live, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x2)));
    x5 = t1166;
    let t1167: [uint32x4_t; 2] = [vshlq_u32(x3[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x3[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    x6 = t1167;
    let t1168 = [vdupq_n_u32(4u32); 2];
    let t1169: [uint32x4_t; 2] = [vmulq_u32(x6[0], t1168[0]), vmulq_u32(x6[1], t1168[1])];
    let t1170 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x5, t1169, 4u32);
    let t1171: [uint32x4_t; 2] = [t1170[0], t1170[1]];
    x7 = t1171;
    let t1172: [uint32x4_t; 2] = [vmulq_u32(x6[0], t1168[0]), vmulq_u32(x6[1], t1168[1])];
    let t1173: [uint32x4_t; 2] = [vaddq_u32(t1172[0], t1168[0]), vaddq_u32(t1172[1], t1168[1])];
    let t1174 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x5, t1173, 4u32);
    let t1175: [uint32x4_t; 2] = [t1174[0], t1174[1]];
    x8 = t1175;
    let t1176 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    x9 = ((t1176).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
    let t1177: [uint32x4_t; 2] = [vmulq_u32(x6[0], t1168[0]), vmulq_u32(x6[1], t1168[1])];
    let t1178 = [vdupq_n_u32(x9); 2];
    let t1179: [uint32x4_t; 2] = [vsubq_u32(t1178[0], t1177[0]), vsubq_u32(t1178[1], t1177[1])];
    let t1180 = b25_sl_p(l, lid0, live, 4u32, t1179);
    x10 = t1180;
    let t1181: [uint32x4_t; 2] = [vshlq_u32(x3[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(x3[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
    let t1182 = [vdupq_n_u32(3u32); 2];
    let t1183: [uint32x4_t; 2] = [vandq_u32(t1181[0], t1182[0]), vandq_u32(t1181[1], t1182[1])];
    x11 = t1183;
    let t1184 = [vdupq_n_u32(0u32); 2];
    let t1185: [uint32x4_t; 2] = [vceqq_u32(x11[0], t1184[0]), vceqq_u32(x11[1], t1184[1])];
    let mut t1186: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1187: [uint32x4_t; 2] = [vandq_u32(live[0], t1185[0]), vandq_u32(live[1], t1185[1])];
    let t1188: [uint32x4_t; 2] = [vbicq_u32(live[0], t1185[0]), vbicq_u32(live[1], t1185[1])];
    {
        t1186 = [vbslq_u32(t1187[0], t1184[0], t1186[0]), vbslq_u32(t1187[1], t1184[1], t1186[1])];
    }
    if any(&t1188) {
        let t1189 = [vdupq_n_u32(1u32); 2];
        let t1190: [uint32x4_t; 2] = [vsubq_u32(x11[0], t1189[0]), vsubq_u32(x11[1], t1189[1])];
        let t1191 = [vdupq_n_u32(2u32); 2];
        let t1192: [uint32x4_t; 2] = [vmulq_u32(t1191[0], t1190[0]), vmulq_u32(t1191[1], t1190[1])];
        let t1193: [uint32x4_t; 2] = [vaddq_u32(x7[0], t1192[0]), vaddq_u32(x7[1], t1192[1])];
        let t1194 = b26_ll_p(l, lid0, t1188, t1193, x8);
        let t1195 = b24_r0r1sls_p(l, lid0, t1188, arg0, arg1, x4, t1194, 2u32);
        let t1196: [uint32x4_t; 2] = [t1195[0], t1195[1]];
        t1186 = [vbslq_u32(t1188[0], t1196[0], t1186[0]), vbslq_u32(t1188[1], t1196[1], t1186[1])];
    }
    x12 = t1186;
    let t1197 = [vdupq_n_u32(2u32); 2];
    let t1198: [uint32x4_t; 2] = [vmulq_u32(t1197[0], x11[0]), vmulq_u32(t1197[1], x11[1])];
    let t1199: [uint32x4_t; 2] = [vaddq_u32(x7[0], t1198[0]), vaddq_u32(x7[1], t1198[1])];
    let t1200 = b26_ll_p(l, lid0, live, t1199, x8);
    let t1201 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x4, t1200, 2u32);
    let t1202: [uint32x4_t; 2] = [t1201[0], t1201[1]];
    x13 = t1202;
    let t1203: [uint32x4_t; 2] = [vmulq_u32(t1197[0], x10[0]), vmulq_u32(t1197[1], x10[1])];
    let t1204: [uint32x4_t; 2] = [vaddq_u32(x7[0], t1203[0]), vaddq_u32(x7[1], t1203[1])];
    x14 = t1204;
    let t1205 = [vdupq_n_u32(255u32); 2];
    let t1206: [uint32x4_t; 2] = [vandq_u32(x3[0], t1205[0]), vandq_u32(x3[1], t1205[1])];
    x15 = t1206;
    x16 = 9259542123273814144u64;
    let t1207: u64 = 72340172838076673u64;
    let t1208 = [vdupq_n_u32((t1207) as u32); 2];
    let t1209 = [vdupq_n_u32(((t1207) >> 32) as u32); 2];
    let t1210: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x15[0]), vget_low_u32(t1208[0]))), vreinterpretq_u32_u64(vmull_high_u32(x15[0], t1208[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x15[1]), vget_low_u32(t1208[1]))), vreinterpretq_u32_u64(vmull_high_u32(x15[1], t1208[1])))];
    let t1211: [uint32x4_t; 4] = [vmulq_u32(x15[0], t1208[0]), vmulq_u32(x15[1], t1208[1]), vmlaq_u32(t1210[0], x15[0], t1209[0]), vmlaq_u32(t1210[1], x15[1], t1209[1])];
    x17 = t1211;
    x18 = x12;
    x19 = x12;
    let mut t1213 = live;
    loop {
        // Lanes part: each its own rounds.
        let t1215: [uint32x4_t; 2] = [vcltq_u32(x19[0], x13[0]), vcltq_u32(x19[1], x13[1])];
        t1213 = [vandq_u32(t1213[0], t1215[0]), vandq_u32(t1213[1], t1215[1])];
        if !(any(&t1213)) { break; }
        let t1217: [uint32x4_t; 2] = [vaddq_u32(x14[0], x19[0]), vaddq_u32(x14[1], x19[1])];
        let t1218 = b26_ll_p(l, lid0, t1213, t1217, x8);
        let t1219 = b24_r0r1sls_p(l, lid0, t1213, arg0, arg1, x4, t1218, 8u32);
        x20 = t1219;
        let t1220: u64 = x16;
        let t1221 = [vdupq_n_u32((t1220) as u32); 2];
        let t1222 = [vdupq_n_u32(((t1220) >> 32) as u32); 2];
        let t1223: [uint32x4_t; 4] = [vorrq_u32(x20[0], t1221[0]), vorrq_u32(x20[1], t1221[1]), vorrq_u32(x20[2], t1222[0]), vorrq_u32(x20[3], t1222[1])];
        let t1224: u64 = !(x16);
        let t1225 = [vdupq_n_u32((t1224) as u32); 2];
        let t1226 = [vdupq_n_u32(((t1224) >> 32) as u32); 2];
        let t1227: [uint32x4_t; 4] = [vandq_u32(x17[0], t1225[0]), vandq_u32(x17[1], t1225[1]), vandq_u32(x17[2], t1226[0]), vandq_u32(x17[3], t1226[1])];
        let t1228: [uint32x4_t; 2] = [vcltq_u32(t1223[0], t1227[0]), vcltq_u32(t1223[1], t1227[1])];
        let t1229: [uint32x4_t; 4] = [vsubq_u32(t1223[0], t1227[0]), vsubq_u32(t1223[1], t1227[1]), vaddq_u32(vsubq_u32(t1223[2], t1227[2]), t1228[0]), vaddq_u32(vsubq_u32(t1223[3], t1227[3]), t1228[1])];
        x21 = t1229;
        let t1230: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x20[r]));
        let t1231: [uint32x4_t; 4] = [vandq_u32(t1230[0], x17[0]), vandq_u32(t1230[1], x17[1]), vandq_u32(t1230[2], x17[2]), vandq_u32(t1230[3], x17[3])];
        let t1232: [uint32x4_t; 4] = [veorq_u32(x20[0], x17[0]), veorq_u32(x20[1], x17[1]), veorq_u32(x20[2], x17[2]), veorq_u32(x20[3], x17[3])];
        let t1233: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(t1232[r]));
        let t1234: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x21[r]));
        let t1235: [uint32x4_t; 4] = [vandq_u32(t1233[0], t1234[0]), vandq_u32(t1233[1], t1234[1]), vandq_u32(t1233[2], t1234[2]), vandq_u32(t1233[3], t1234[3])];
        let t1236: [uint32x4_t; 4] = [vorrq_u32(t1231[0], t1235[0]), vorrq_u32(t1231[1], t1235[1]), vorrq_u32(t1231[2], t1235[2]), vorrq_u32(t1231[3], t1235[3])];
        let t1237: u64 = x16;
        let t1238 = [vdupq_n_u32((t1237) as u32); 2];
        let t1239 = [vdupq_n_u32(((t1237) >> 32) as u32); 2];
        let t1240: [uint32x4_t; 4] = [vandq_u32(t1238[0], t1236[0]), vandq_u32(t1238[1], t1236[1]), vandq_u32(t1239[0], t1236[2]), vandq_u32(t1239[1], t1236[3])];
        x22 = t1240;
        let t1241: [uint32x4_t; 2] = [vsubq_u32(x13[0], x19[0]), vsubq_u32(x13[1], x19[1])];
        x23 = t1241;
        let t1242 = [vdupq_n_u32(8u32); 2];
        let t1243: [uint32x4_t; 2] = [vcgeq_u32(x23[0], t1242[0]), vcgeq_u32(x23[1], t1242[1])];
        let mut t1244: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t1245: [uint32x4_t; 2] = [vandq_u32(t1213[0], t1243[0]), vandq_u32(t1213[1], t1243[1])];
        let t1246: [uint32x4_t; 2] = [vbicq_u32(t1213[0], t1243[0]), vbicq_u32(t1213[1], t1243[1])];
        {
            t1244 = [vbslq_u32(t1245[0], x22[0], t1244[0]), vbslq_u32(t1245[1], x22[1], t1244[1]), vbslq_u32(t1245[0], x22[2], t1244[2]), vbslq_u32(t1245[1], x22[3], t1244[3])];
        }
        {
            let t1247: [uint32x4_t; 2] = [vmulq_u32(t1242[0], x23[0]), vmulq_u32(t1242[1], x23[1])];
            let t1248: u64 = 1u64;
            let t1249 = [vdupq_n_u32((t1248) as u32); 2];
            let t1250: [uint32x4_t; 2] = [t1247[0], t1247[1]];
            let t1251: [uint32x4_t; 2] = [vandq_u32(t1250[0], vdupq_n_u32(63)), vandq_u32(t1250[1], vdupq_n_u32(63))];
            let t1252: [uint32x4_t; 4] = [vshlq_u32(t1249[0], vreinterpretq_s32_u32(t1251[0])), vshlq_u32(t1249[1], vreinterpretq_s32_u32(t1251[1])), vshlq_u32(t1249[0], vsubq_s32(vreinterpretq_s32_u32(t1251[0]), vdupq_n_s32(32))), vshlq_u32(t1249[1], vsubq_s32(vreinterpretq_s32_u32(t1251[1]), vdupq_n_s32(32)))];
            let t1253: u64 = 1u64;
            let t1254 = [vdupq_n_u32((t1253) as u32); 2];
            let t1255: [uint32x4_t; 2] = [vcltq_u32(t1252[0], t1254[0]), vcltq_u32(t1252[1], t1254[1])];
            let t1256: [uint32x4_t; 4] = [vsubq_u32(t1252[0], t1254[0]), vsubq_u32(t1252[1], t1254[1]), vaddq_u32(vsubq_u32(t1252[2], vdupq_n_u32(0)), t1255[0]), vaddq_u32(vsubq_u32(t1252[3], vdupq_n_u32(0)), t1255[1])];
            let t1257: [uint32x4_t; 4] = [vandq_u32(x22[0], t1256[0]), vandq_u32(x22[1], t1256[1]), vandq_u32(x22[2], t1256[2]), vandq_u32(x22[3], t1256[3])];
            t1244 = [vbslq_u32(t1246[0], t1257[0], t1244[0]), vbslq_u32(t1246[1], t1257[1], t1244[1]), vbslq_u32(t1246[0], t1257[2], t1244[2]), vbslq_u32(t1246[1], t1257[3], t1244[3])];
        }
        let t1258 = b27_l_p(l, lid0, t1213, t1244);
        let t1259: [uint32x4_t; 2] = [vaddq_u32(x18[0], t1258[0]), vaddq_u32(x18[1], t1258[1])];
        x18 = [vbslq_u32(t1213[0], t1259[0], x18[0]), vbslq_u32(t1213[1], t1259[1], x18[1])];
        let t1260: [uint32x4_t; 2] = [vandq_u32(t1213[0], live[0]), vandq_u32(t1213[1], live[1])];
        let t1261: [uint32x4_t; 2] = [vaddq_u32(x19[0], t1242[0]), vaddq_u32(x19[1], t1242[1])];
        x19 = t1261;
    }
    x12 = x18;
    let t1262 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    x24 = (((t1262).wrapping_shr((((((4u32).wrapping_mul(x2)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32));
    let t1263 = [vdupq_n_u32(1u32); 2];
    let t1264: [uint32x4_t; 2] = [vsubq_u32(x10[0], t1263[0]), vsubq_u32(x10[1], t1263[1])];
    let t1265: [uint32x4_t; 2] = [vmulq_u32(t1197[0], t1264[0]), vmulq_u32(t1197[1], t1264[1])];
    let t1266: [uint32x4_t; 2] = [vaddq_u32(x7[0], t1265[0]), vaddq_u32(x7[1], t1265[1])];
    let t1267 = b26_ll_p(l, lid0, live, t1266, x8);
    let t1268 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x4, t1267, 2u32);
    let t1269: [uint32x4_t; 2] = [t1268[0], t1268[1]];
    x25 = t1269;
    let t1270: [uint32x4_t; 2] = [vaddq_u32(x14[0], x25[0]), vaddq_u32(x14[1], x25[1])];
    let t1271 = [vdupq_n_u32(x24); 2];
    let t1272: [uint32x4_t; 2] = [vmulq_u32(x12[0], t1271[0]), vmulq_u32(x12[1], t1271[1])];
    let t1273: [uint32x4_t; 2] = [vaddq_u32(t1270[0], t1272[0]), vaddq_u32(t1270[1], t1272[1])];
    let t1274 = b26_ll_p(l, lid0, live, t1273, x8);
    x26 = t1274;
    let t1275 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x4, x26, 8u32);
    let t1276 = b17_s_p(l, lid0, live, x24);
    let t1277: u64 = t1276;
    let t1278 = [vdupq_n_u32((t1277) as u32); 2];
    let t1279 = [vdupq_n_u32(((t1277) >> 32) as u32); 2];
    let t1280: [uint32x4_t; 4] = [vandq_u32(t1275[0], t1278[0]), vandq_u32(t1275[1], t1278[1]), vandq_u32(t1275[2], t1279[0]), vandq_u32(t1275[3], t1279[1])];
    res = t1280;
    return res;
}

/// `clif_f16_view`.
#[inline(always)]
unsafe fn b20_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    res = (arg1.wrapping_add(((((x2).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned();
    return res;
}

/// `clif_f16_view`.
#[inline(always)]
unsafe fn b20_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    res = (arg1.wrapping_add(((((x2).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned();
    return res;
}

/// `clif_f16_columns`.
#[inline(always)]
unsafe fn b21_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t1281 = b20_r0r1s_p(l, lid0, live, arg0, arg1, (20u32).wrapping_add((4u32).wrapping_mul(x2)));
    res = t1281;
    return res;
}

/// `clif_f16_columns`.
#[inline(always)]
unsafe fn b21_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t1282 = b20_r0r1s_f(l, lid0, arg0, arg1, (20u32).wrapping_add((4u32).wrapping_mul(x2)));
    res = t1282;
    return res;
}

/// `ukl_min`.
#[inline(always)]
unsafe fn b22_ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x1: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t1283: u32;
    if ((x0) < (x1)) {
        t1283 = x0;
    } else {
        t1283 = x1;
    }
    res = t1283;
    return res;
}

/// `ukl_min`.
#[inline(always)]
unsafe fn b22_ss_f(l: &Launch, lid0: u32, x0: u32, x1: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t1284: u32;
    if ((x0) < (x1)) {
        t1284 = x0;
    } else {
        t1284 = x1;
    }
    res = t1284;
    return res;
}

/// `ukl_min`.
#[inline(always)]
unsafe fn b22_ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x1: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1285 = [vdupq_n_u32(x1); 2];
    let t1286: [uint32x4_t; 2] = [vcltq_u32(x0[0], t1285[0]), vcltq_u32(x0[1], t1285[1])];
    let mut t1287: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1288: [uint32x4_t; 2] = [vandq_u32(live[0], t1286[0]), vandq_u32(live[1], t1286[1])];
    let t1289: [uint32x4_t; 2] = [vbicq_u32(live[0], t1286[0]), vbicq_u32(live[1], t1286[1])];
    {
        t1287 = [vbslq_u32(t1288[0], x0[0], t1287[0]), vbslq_u32(t1288[1], x0[1], t1287[1])];
    }
    {
        let t1290 = [vdupq_n_u32(x1); 2];
        t1287 = [vbslq_u32(t1289[0], t1290[0], t1287[0]), vbslq_u32(t1289[1], t1290[1], t1287[1])];
    }
    res = t1287;
    return res;
}

/// `ukl_min`.
#[inline(always)]
unsafe fn b22_ls_f(l: &Launch, lid0: u32, x0: [uint32x4_t; 2], x1: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1291 = [vdupq_n_u32(x1); 2];
    let t1292: [uint32x4_t; 2] = [vcltq_u32(x0[0], t1291[0]), vcltq_u32(x0[1], t1291[1])];
    let mut t1293: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1294: [uint32x4_t; 2] = [vmvnq_u32(t1292[0]), vmvnq_u32(t1292[1])];
    {
        t1293 = [vbslq_u32(t1292[0], x0[0], t1293[0]), vbslq_u32(t1292[1], x0[1], t1293[1])];
    }
    {
        let t1295 = [vdupq_n_u32(x1); 2];
        t1293 = [vbslq_u32(t1294[0], t1295[0], t1293[0]), vbslq_u32(t1294[1], t1295[1], t1293[1])];
    }
    res = t1293;
    return res;
}

/// `clif_t_ld`.
#[inline(always)]
unsafe fn b23_r0sns_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1: u32, x2f: u32, x2s: u32, x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1296 = b28_r0an1s_p(l, lid0, live, arg0, ((((x1) as u64)) as usize as *const u8), x2f, x2s, x3);
    x4 = t1296;
    let t1297: [uint32x4_t; 4];
    if ((x3) >= (8u32)) {
        t1297 = x4;
    } else {
        let t1298 = b29_s_p(l, lid0, live, (8u32).wrapping_mul(x3));
        let t1299: u64 = t1298;
        let t1300 = [vdupq_n_u32((t1299) as u32); 2];
        let t1301 = [vdupq_n_u32(((t1299) >> 32) as u32); 2];
        let t1302: [uint32x4_t; 4] = [vandq_u32(x4[0], t1300[0]), vandq_u32(x4[1], t1300[1]), vandq_u32(x4[2], t1301[0]), vandq_u32(x4[3], t1301[1])];
        t1297 = t1302;
    }
    res = t1297;
    return res;
}

/// `clif_t_ld`.
#[inline(always)]
unsafe fn b23_r0sns_f(l: &Launch, lid0: u32, arg0: *const u8, x1: u32, x2f: u32, x2s: u32, x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1303 = b28_r0an1s_f(l, lid0, arg0, ((((x1) as u64)) as usize as *const u8), x2f, x2s, x3);
    x4 = t1303;
    let t1304: [uint32x4_t; 4];
    if ((x3) >= (8u32)) {
        t1304 = x4;
    } else {
        let t1305 = b29_s_f(l, lid0, (8u32).wrapping_mul(x3));
        let t1306: u64 = t1305;
        let t1307 = [vdupq_n_u32((t1306) as u32); 2];
        let t1308 = [vdupq_n_u32(((t1306) >> 32) as u32); 2];
        let t1309: [uint32x4_t; 4] = [vandq_u32(x4[0], t1307[0]), vandq_u32(x4[1], t1307[1]), vandq_u32(x4[2], t1308[0]), vandq_u32(x4[3], t1308[1])];
        t1304 = t1309;
    }
    res = t1304;
    return res;
}

/// `clif_t_ld`.
#[inline(always)]
unsafe fn b23_r0lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1: [uint32x4_t; 2], x2: [uint32x4_t; 2], x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1310: [uint32x4_t; 2] = [vaddq_u32(x1[0], x2[0]), vaddq_u32(x1[1], x2[1])];
    let t1311: [uint32x4_t; 2] = [vcltq_u32(t1310[0], x1[0]), vcltq_u32(t1310[1], x1[1])];
    let t1312: [uint32x4_t; 4] = [t1310[0], t1310[1], vshrq_n_u32::<31>(t1311[0]), vshrq_n_u32::<31>(t1311[1])];
    let t1313 = b28_r0ls_p(l, lid0, live, arg0, t1312, x3);
    x4 = t1313;
    let t1314: [uint32x4_t; 4];
    if ((x3) >= (8u32)) {
        t1314 = x4;
    } else {
        let t1315 = b29_s_p(l, lid0, live, (8u32).wrapping_mul(x3));
        let t1316: u64 = t1315;
        let t1317 = [vdupq_n_u32((t1316) as u32); 2];
        let t1318 = [vdupq_n_u32(((t1316) >> 32) as u32); 2];
        let t1319: [uint32x4_t; 4] = [vandq_u32(x4[0], t1317[0]), vandq_u32(x4[1], t1317[1]), vandq_u32(x4[2], t1318[0]), vandq_u32(x4[3], t1318[1])];
        t1314 = t1319;
    }
    res = t1314;
    return res;
}

/// `clif_t_ld`.
#[inline(always)]
unsafe fn b23_r0lls_f(l: &Launch, lid0: u32, arg0: *const u8, x1: [uint32x4_t; 2], x2: [uint32x4_t; 2], x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1320: [uint32x4_t; 2] = [vaddq_u32(x1[0], x2[0]), vaddq_u32(x1[1], x2[1])];
    let t1321: [uint32x4_t; 2] = [vcltq_u32(t1320[0], x1[0]), vcltq_u32(t1320[1], x1[1])];
    let t1322: [uint32x4_t; 4] = [t1320[0], t1320[1], vshrq_n_u32::<31>(t1321[0]), vshrq_n_u32::<31>(t1321[1])];
    let t1323 = b28_r0ls_f(l, lid0, arg0, t1322, x3);
    x4 = t1323;
    let t1324: [uint32x4_t; 4];
    if ((x3) >= (8u32)) {
        t1324 = x4;
    } else {
        let t1325 = b29_s_f(l, lid0, (8u32).wrapping_mul(x3));
        let t1326: u64 = t1325;
        let t1327 = [vdupq_n_u32((t1326) as u32); 2];
        let t1328 = [vdupq_n_u32(((t1326) >> 32) as u32); 2];
        let t1329: [uint32x4_t; 4] = [vandq_u32(x4[0], t1327[0]), vandq_u32(x4[1], t1327[1]), vandq_u32(x4[2], t1328[0]), vandq_u32(x4[3], t1328[1])];
        t1324 = t1329;
    }
    res = t1324;
    return res;
}

/// `clif_f16_ld`.
#[inline(always)]
unsafe fn b24_r0r1sss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: u32, x4: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t1330 = b30_r0sss_p(l, lid0, live, arg0, x2, x3, x4);
    res = t1330;
    return res;
}

/// `clif_f16_ld`.
#[inline(always)]
unsafe fn b24_r0r1sss_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32, x3: u32, x4: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t1331 = b30_r0sss_f(l, lid0, arg0, x2, x3, x4);
    res = t1331;
    return res;
}

/// `clif_f16_ld`.
#[inline(always)]
unsafe fn b24_r0r1sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: [uint32x4_t; 2], x4: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1332 = b30_r0sls_p(l, lid0, live, arg0, x2, x3, x4);
    res = t1332;
    return res;
}

/// `ukl_min`.
#[inline(always)]
unsafe fn b25_ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x1: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t1333: u32;
    if ((x0) < (x1)) {
        t1333 = x0;
    } else {
        t1333 = x1;
    }
    res = t1333;
    return res;
}

/// `ukl_min`.
#[inline(always)]
unsafe fn b25_ss_f(l: &Launch, lid0: u32, x0: u32, x1: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t1334: u32;
    if ((x0) < (x1)) {
        t1334 = x0;
    } else {
        t1334 = x1;
    }
    res = t1334;
    return res;
}

/// `ukl_min`.
#[inline(always)]
unsafe fn b25_sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x1: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1335 = [vdupq_n_u32(x0); 2];
    let t1336: [uint32x4_t; 2] = [vcltq_u32(t1335[0], x1[0]), vcltq_u32(t1335[1], x1[1])];
    let mut t1337: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1338: [uint32x4_t; 2] = [vandq_u32(live[0], t1336[0]), vandq_u32(live[1], t1336[1])];
    let t1339: [uint32x4_t; 2] = [vbicq_u32(live[0], t1336[0]), vbicq_u32(live[1], t1336[1])];
    {
        let t1340 = [vdupq_n_u32(x0); 2];
        t1337 = [vbslq_u32(t1338[0], t1340[0], t1337[0]), vbslq_u32(t1338[1], t1340[1], t1337[1])];
    }
    {
        t1337 = [vbslq_u32(t1339[0], x1[0], t1337[0]), vbslq_u32(t1339[1], x1[1], t1337[1])];
    }
    res = t1337;
    return res;
}

/// `ukl_min`.
#[inline(always)]
unsafe fn b26_ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x1: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t1341: u32;
    if ((x0) < (x1)) {
        t1341 = x0;
    } else {
        t1341 = x1;
    }
    res = t1341;
    return res;
}

/// `ukl_min`.
#[inline(always)]
unsafe fn b26_ss_f(l: &Launch, lid0: u32, x0: u32, x1: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t1342: u32;
    if ((x0) < (x1)) {
        t1342 = x0;
    } else {
        t1342 = x1;
    }
    res = t1342;
    return res;
}

/// `ukl_min`.
#[inline(always)]
unsafe fn b26_ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x1: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1343 = [vdupq_n_u32(x1); 2];
    let t1344: [uint32x4_t; 2] = [vcltq_u32(x0[0], t1343[0]), vcltq_u32(x0[1], t1343[1])];
    let mut t1345: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1346: [uint32x4_t; 2] = [vandq_u32(live[0], t1344[0]), vandq_u32(live[1], t1344[1])];
    let t1347: [uint32x4_t; 2] = [vbicq_u32(live[0], t1344[0]), vbicq_u32(live[1], t1344[1])];
    {
        t1345 = [vbslq_u32(t1346[0], x0[0], t1345[0]), vbslq_u32(t1346[1], x0[1], t1345[1])];
    }
    {
        let t1348 = [vdupq_n_u32(x1); 2];
        t1345 = [vbslq_u32(t1347[0], t1348[0], t1345[0]), vbslq_u32(t1347[1], t1348[1], t1345[1])];
    }
    res = t1345;
    return res;
}

/// `ukl_min`.
#[inline(always)]
unsafe fn b26_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x1: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1349: [uint32x4_t; 2] = [vcltq_u32(x0[0], x1[0]), vcltq_u32(x0[1], x1[1])];
    let mut t1350: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1351: [uint32x4_t; 2] = [vandq_u32(live[0], t1349[0]), vandq_u32(live[1], t1349[1])];
    let t1352: [uint32x4_t; 2] = [vbicq_u32(live[0], t1349[0]), vbicq_u32(live[1], t1349[1])];
    {
        t1350 = [vbslq_u32(t1351[0], x0[0], t1350[0]), vbslq_u32(t1351[1], x0[1], t1350[1])];
    }
    {
        t1350 = [vbslq_u32(t1352[0], x1[0], t1350[0]), vbslq_u32(t1352[1], x1[1], t1350[1])];
    }
    res = t1350;
    return res;
}

/// `ukl_popcount64`.
#[inline(always)]
unsafe fn b27_s_f(l: &Launch, lid0: u32, x0: u64) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    res = (((((x0) as u64).count_ones() as u64)) as u32);
    return res;
}

/// `ukl_popcount64`.
#[inline(always)]
unsafe fn b27_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1353: [uint32x4_t; 2] = [vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[0])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[2]))))), vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[1])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[3])))))];
    res = t1353;
    return res;
}

/// `ukl_ld_window`.
#[inline(always)]
unsafe fn b28_r0ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1: u64, x2: u32) -> u64 {
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
    x3 = arg0.wrapping_add(((x1) & (!(7u64))) as usize);
    x4 = (((((x1) & (7u64))) as u32)).wrapping_mul(8u32);
    x5 = ((x3 as *const u64).read_unaligned()).wrapping_shr((((x4) as u64)) as u32);
    let mut t1354: bool = ((x4) != (0u32));
    if t1354 {
        t1354 = (((x4).wrapping_add((x2).wrapping_mul(8u32))) > (64u32));
    }
    let t1355: u64;
    if t1354 {
        t1355 = ((x5) | (((x3.wrapping_add((1u64).wrapping_mul(8u64) as usize) as *const u64).read_unaligned()).wrapping_shl(((((64u32).wrapping_sub(x4)) as u64)) as u32)));
    } else {
        t1355 = x5;
    }
    res = t1355;
    return res;
}

/// `ukl_ld_window`.
#[inline(always)]
unsafe fn b28_r0ss_f(l: &Launch, lid0: u32, arg0: *const u8, x1: u64, x2: u32) -> u64 {
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
    x3 = arg0.wrapping_add(((x1) & (!(7u64))) as usize);
    x4 = (((((x1) & (7u64))) as u32)).wrapping_mul(8u32);
    x5 = ((x3 as *const u64).read_unaligned()).wrapping_shr((((x4) as u64)) as u32);
    let mut t1356: bool = ((x4) != (0u32));
    if t1356 {
        t1356 = (((x4).wrapping_add((x2).wrapping_mul(8u32))) > (64u32));
    }
    let t1357: u64;
    if t1356 {
        t1357 = ((x5) | (((x3.wrapping_add((1u64).wrapping_mul(8u64) as usize) as *const u64).read_unaligned()).wrapping_shl(((((64u32).wrapping_sub(x4)) as u64)) as u32)));
    } else {
        t1357 = x5;
    }
    res = t1357;
    return res;
}

/// `ukl_ld_window`.
#[inline(always)]
unsafe fn b28_r0ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1: [uint32x4_t; 4], x2: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x3: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x4: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x5: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if ((x2) == (8u32)) {
        let t1358: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
        let t1359: [uint32x4_t; 4] = [vandq_u32(x1[0], t1358[0]), vandq_u32(x1[1], t1358[1]), vandq_u32(x1[2], t1358[2]), vandq_u32(x1[3], t1358[3])];
        let t1360: u64 = ((arg0) as usize as u64);
        let t1361 = [vdupq_n_u32((t1360) as u32); 2];
        let t1362 = [vdupq_n_u32(((t1360) >> 32) as u32); 2];
        let t1363: [uint32x4_t; 2] = [vaddq_u32(t1361[0], t1359[0]), vaddq_u32(t1361[1], t1359[1])];
        let t1364: [uint32x4_t; 2] = [vcltq_u32(t1363[0], t1361[0]), vcltq_u32(t1363[1], t1361[1])];
        let t1365: [uint32x4_t; 4] = [t1363[0], t1363[1], vsubq_u32(vaddq_u32(t1362[0], t1359[2]), t1364[0]), vsubq_u32(vaddq_u32(t1362[1], t1359[3]), t1364[1])];
        let t1366 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([t1365[0], t1365[1]]);
        let t1367 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([t1365[2], t1365[3]]);
        let t1369 = bits(&live);
        let t1370 = t1369.trailing_zeros() as usize % 8;
        let t1368: [usize; 8] = [{ let j = if t1369 >> 0 & 1 != 0 { 0 } else { t1370 }; ((t1367[j] as u64) << 32 | t1366[j] as u64) as usize }, { let j = if t1369 >> 1 & 1 != 0 { 1 } else { t1370 }; ((t1367[j] as u64) << 32 | t1366[j] as u64) as usize }, { let j = if t1369 >> 2 & 1 != 0 { 2 } else { t1370 }; ((t1367[j] as u64) << 32 | t1366[j] as u64) as usize }, { let j = if t1369 >> 3 & 1 != 0 { 3 } else { t1370 }; ((t1367[j] as u64) << 32 | t1366[j] as u64) as usize }, { let j = if t1369 >> 4 & 1 != 0 { 4 } else { t1370 }; ((t1367[j] as u64) << 32 | t1366[j] as u64) as usize }, { let j = if t1369 >> 5 & 1 != 0 { 5 } else { t1370 }; ((t1367[j] as u64) << 32 | t1366[j] as u64) as usize }, { let j = if t1369 >> 6 & 1 != 0 { 6 } else { t1370 }; ((t1367[j] as u64) << 32 | t1366[j] as u64) as usize }, { let j = if t1369 >> 7 & 1 != 0 { 7 } else { t1370 }; ((t1367[j] as u64) << 32 | t1366[j] as u64) as usize }];
        let mut t1371: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (t1368[0] as *const u64).read_unaligned(); t1371[0] = vsetq_lane_u32::<0>(v as u32, t1371[0]); t1371[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1371[2]); }
        { let v = (t1368[1] as *const u64).read_unaligned(); t1371[0] = vsetq_lane_u32::<1>(v as u32, t1371[0]); t1371[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1371[2]); }
        { let v = (t1368[2] as *const u64).read_unaligned(); t1371[0] = vsetq_lane_u32::<2>(v as u32, t1371[0]); t1371[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1371[2]); }
        { let v = (t1368[3] as *const u64).read_unaligned(); t1371[0] = vsetq_lane_u32::<3>(v as u32, t1371[0]); t1371[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1371[2]); }
        { let v = (t1368[4] as *const u64).read_unaligned(); t1371[1] = vsetq_lane_u32::<0>(v as u32, t1371[1]); t1371[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1371[3]); }
        { let v = (t1368[5] as *const u64).read_unaligned(); t1371[1] = vsetq_lane_u32::<1>(v as u32, t1371[1]); t1371[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1371[3]); }
        { let v = (t1368[6] as *const u64).read_unaligned(); t1371[1] = vsetq_lane_u32::<2>(v as u32, t1371[1]); t1371[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1371[3]); }
        { let v = (t1368[7] as *const u64).read_unaligned(); t1371[1] = vsetq_lane_u32::<3>(v as u32, t1371[1]); t1371[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1371[3]); }
        res = t1371;
        return res;
    }
    if ((x2) == (4u32)) {
        let t1372: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
        let t1373: [uint32x4_t; 4] = [vandq_u32(x1[0], t1372[0]), vandq_u32(x1[1], t1372[1]), vandq_u32(x1[2], t1372[2]), vandq_u32(x1[3], t1372[3])];
        let t1374: u64 = ((arg0) as usize as u64);
        let t1375 = [vdupq_n_u32((t1374) as u32); 2];
        let t1376 = [vdupq_n_u32(((t1374) >> 32) as u32); 2];
        let t1377: [uint32x4_t; 2] = [vaddq_u32(t1375[0], t1373[0]), vaddq_u32(t1375[1], t1373[1])];
        let t1378: [uint32x4_t; 2] = [vcltq_u32(t1377[0], t1375[0]), vcltq_u32(t1377[1], t1375[1])];
        let t1379: [uint32x4_t; 4] = [t1377[0], t1377[1], vsubq_u32(vaddq_u32(t1376[0], t1373[2]), t1378[0]), vsubq_u32(vaddq_u32(t1376[1], t1373[3]), t1378[1])];
        let t1380 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([t1379[0], t1379[1]]);
        let t1381 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([t1379[2], t1379[3]]);
        let t1383 = bits(&live);
        let t1384 = t1383.trailing_zeros() as usize % 8;
        let t1382: [usize; 8] = [{ let j = if t1383 >> 0 & 1 != 0 { 0 } else { t1384 }; ((t1381[j] as u64) << 32 | t1380[j] as u64) as usize }, { let j = if t1383 >> 1 & 1 != 0 { 1 } else { t1384 }; ((t1381[j] as u64) << 32 | t1380[j] as u64) as usize }, { let j = if t1383 >> 2 & 1 != 0 { 2 } else { t1384 }; ((t1381[j] as u64) << 32 | t1380[j] as u64) as usize }, { let j = if t1383 >> 3 & 1 != 0 { 3 } else { t1384 }; ((t1381[j] as u64) << 32 | t1380[j] as u64) as usize }, { let j = if t1383 >> 4 & 1 != 0 { 4 } else { t1384 }; ((t1381[j] as u64) << 32 | t1380[j] as u64) as usize }, { let j = if t1383 >> 5 & 1 != 0 { 5 } else { t1384 }; ((t1381[j] as u64) << 32 | t1380[j] as u64) as usize }, { let j = if t1383 >> 6 & 1 != 0 { 6 } else { t1384 }; ((t1381[j] as u64) << 32 | t1380[j] as u64) as usize }, { let j = if t1383 >> 7 & 1 != 0 { 7 } else { t1384 }; ((t1381[j] as u64) << 32 | t1380[j] as u64) as usize }];
        let mut t1385: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t1385[0] = vsetq_lane_u32::<0>((t1382[0] as *const u32).read_unaligned() as u32, t1385[0]);
        t1385[0] = vsetq_lane_u32::<1>((t1382[1] as *const u32).read_unaligned() as u32, t1385[0]);
        t1385[0] = vsetq_lane_u32::<2>((t1382[2] as *const u32).read_unaligned() as u32, t1385[0]);
        t1385[0] = vsetq_lane_u32::<3>((t1382[3] as *const u32).read_unaligned() as u32, t1385[0]);
        t1385[1] = vsetq_lane_u32::<0>((t1382[4] as *const u32).read_unaligned() as u32, t1385[1]);
        t1385[1] = vsetq_lane_u32::<1>((t1382[5] as *const u32).read_unaligned() as u32, t1385[1]);
        t1385[1] = vsetq_lane_u32::<2>((t1382[6] as *const u32).read_unaligned() as u32, t1385[1]);
        t1385[1] = vsetq_lane_u32::<3>((t1382[7] as *const u32).read_unaligned() as u32, t1385[1]);
        let t1386: [uint32x4_t; 4] = [t1385[0], t1385[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1386;
        return res;
    }
    if ((x2) == (2u32)) {
        let t1387: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
        let t1388: [uint32x4_t; 4] = [vandq_u32(x1[0], t1387[0]), vandq_u32(x1[1], t1387[1]), vandq_u32(x1[2], t1387[2]), vandq_u32(x1[3], t1387[3])];
        let t1389: u64 = ((arg0) as usize as u64);
        let t1390 = [vdupq_n_u32((t1389) as u32); 2];
        let t1391 = [vdupq_n_u32(((t1389) >> 32) as u32); 2];
        let t1392: [uint32x4_t; 2] = [vaddq_u32(t1390[0], t1388[0]), vaddq_u32(t1390[1], t1388[1])];
        let t1393: [uint32x4_t; 2] = [vcltq_u32(t1392[0], t1390[0]), vcltq_u32(t1392[1], t1390[1])];
        let t1394: [uint32x4_t; 4] = [t1392[0], t1392[1], vsubq_u32(vaddq_u32(t1391[0], t1388[2]), t1393[0]), vsubq_u32(vaddq_u32(t1391[1], t1388[3]), t1393[1])];
        let t1395 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([t1394[0], t1394[1]]);
        let t1396 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([t1394[2], t1394[3]]);
        let t1398 = bits(&live);
        let t1399 = t1398.trailing_zeros() as usize % 8;
        let t1397: [usize; 8] = [{ let j = if t1398 >> 0 & 1 != 0 { 0 } else { t1399 }; ((t1396[j] as u64) << 32 | t1395[j] as u64) as usize }, { let j = if t1398 >> 1 & 1 != 0 { 1 } else { t1399 }; ((t1396[j] as u64) << 32 | t1395[j] as u64) as usize }, { let j = if t1398 >> 2 & 1 != 0 { 2 } else { t1399 }; ((t1396[j] as u64) << 32 | t1395[j] as u64) as usize }, { let j = if t1398 >> 3 & 1 != 0 { 3 } else { t1399 }; ((t1396[j] as u64) << 32 | t1395[j] as u64) as usize }, { let j = if t1398 >> 4 & 1 != 0 { 4 } else { t1399 }; ((t1396[j] as u64) << 32 | t1395[j] as u64) as usize }, { let j = if t1398 >> 5 & 1 != 0 { 5 } else { t1399 }; ((t1396[j] as u64) << 32 | t1395[j] as u64) as usize }, { let j = if t1398 >> 6 & 1 != 0 { 6 } else { t1399 }; ((t1396[j] as u64) << 32 | t1395[j] as u64) as usize }, { let j = if t1398 >> 7 & 1 != 0 { 7 } else { t1399 }; ((t1396[j] as u64) << 32 | t1395[j] as u64) as usize }];
        let mut t1400: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t1400[0] = vsetq_lane_u32::<0>((t1397[0] as *const u16).read_unaligned() as u32, t1400[0]);
        t1400[0] = vsetq_lane_u32::<1>((t1397[1] as *const u16).read_unaligned() as u32, t1400[0]);
        t1400[0] = vsetq_lane_u32::<2>((t1397[2] as *const u16).read_unaligned() as u32, t1400[0]);
        t1400[0] = vsetq_lane_u32::<3>((t1397[3] as *const u16).read_unaligned() as u32, t1400[0]);
        t1400[1] = vsetq_lane_u32::<0>((t1397[4] as *const u16).read_unaligned() as u32, t1400[1]);
        t1400[1] = vsetq_lane_u32::<1>((t1397[5] as *const u16).read_unaligned() as u32, t1400[1]);
        t1400[1] = vsetq_lane_u32::<2>((t1397[6] as *const u16).read_unaligned() as u32, t1400[1]);
        t1400[1] = vsetq_lane_u32::<3>((t1397[7] as *const u16).read_unaligned() as u32, t1400[1]);
        let t1401: [uint32x4_t; 4] = [t1400[0], t1400[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1401;
        return res;
    }
    if ((x2) == (1u32)) {
        let t1402: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
        let t1403: [uint32x4_t; 4] = [vandq_u32(x1[0], t1402[0]), vandq_u32(x1[1], t1402[1]), vandq_u32(x1[2], t1402[2]), vandq_u32(x1[3], t1402[3])];
        let t1404: u64 = ((arg0) as usize as u64);
        let t1405 = [vdupq_n_u32((t1404) as u32); 2];
        let t1406 = [vdupq_n_u32(((t1404) >> 32) as u32); 2];
        let t1407: [uint32x4_t; 2] = [vaddq_u32(t1405[0], t1403[0]), vaddq_u32(t1405[1], t1403[1])];
        let t1408: [uint32x4_t; 2] = [vcltq_u32(t1407[0], t1405[0]), vcltq_u32(t1407[1], t1405[1])];
        let t1409: [uint32x4_t; 4] = [t1407[0], t1407[1], vsubq_u32(vaddq_u32(t1406[0], t1403[2]), t1408[0]), vsubq_u32(vaddq_u32(t1406[1], t1403[3]), t1408[1])];
        let t1410 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([t1409[0], t1409[1]]);
        let t1411 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([t1409[2], t1409[3]]);
        let t1413 = bits(&live);
        let t1414 = t1413.trailing_zeros() as usize % 8;
        let t1412: [usize; 8] = [{ let j = if t1413 >> 0 & 1 != 0 { 0 } else { t1414 }; ((t1411[j] as u64) << 32 | t1410[j] as u64) as usize }, { let j = if t1413 >> 1 & 1 != 0 { 1 } else { t1414 }; ((t1411[j] as u64) << 32 | t1410[j] as u64) as usize }, { let j = if t1413 >> 2 & 1 != 0 { 2 } else { t1414 }; ((t1411[j] as u64) << 32 | t1410[j] as u64) as usize }, { let j = if t1413 >> 3 & 1 != 0 { 3 } else { t1414 }; ((t1411[j] as u64) << 32 | t1410[j] as u64) as usize }, { let j = if t1413 >> 4 & 1 != 0 { 4 } else { t1414 }; ((t1411[j] as u64) << 32 | t1410[j] as u64) as usize }, { let j = if t1413 >> 5 & 1 != 0 { 5 } else { t1414 }; ((t1411[j] as u64) << 32 | t1410[j] as u64) as usize }, { let j = if t1413 >> 6 & 1 != 0 { 6 } else { t1414 }; ((t1411[j] as u64) << 32 | t1410[j] as u64) as usize }, { let j = if t1413 >> 7 & 1 != 0 { 7 } else { t1414 }; ((t1411[j] as u64) << 32 | t1410[j] as u64) as usize }];
        let mut t1415: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t1415[0] = vsetq_lane_u32::<0>((t1412[0] as *const u8).read_unaligned() as u32, t1415[0]);
        t1415[0] = vsetq_lane_u32::<1>((t1412[1] as *const u8).read_unaligned() as u32, t1415[0]);
        t1415[0] = vsetq_lane_u32::<2>((t1412[2] as *const u8).read_unaligned() as u32, t1415[0]);
        t1415[0] = vsetq_lane_u32::<3>((t1412[3] as *const u8).read_unaligned() as u32, t1415[0]);
        t1415[1] = vsetq_lane_u32::<0>((t1412[4] as *const u8).read_unaligned() as u32, t1415[1]);
        t1415[1] = vsetq_lane_u32::<1>((t1412[5] as *const u8).read_unaligned() as u32, t1415[1]);
        t1415[1] = vsetq_lane_u32::<2>((t1412[6] as *const u8).read_unaligned() as u32, t1415[1]);
        t1415[1] = vsetq_lane_u32::<3>((t1412[7] as *const u8).read_unaligned() as u32, t1415[1]);
        let t1416: [uint32x4_t; 4] = [t1415[0], t1415[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1416;
        return res;
    }
    let t1417: u64 = !(7u64);
    let t1418 = [vdupq_n_u32((t1417) as u32); 2];
    let t1419 = [vdupq_n_u32(((t1417) >> 32) as u32); 2];
    let t1420: [uint32x4_t; 4] = [vandq_u32(x1[0], t1418[0]), vandq_u32(x1[1], t1418[1]), vandq_u32(x1[2], t1419[0]), vandq_u32(x1[3], t1419[1])];
    let t1421: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
    let t1422: [uint32x4_t; 4] = [vandq_u32(t1420[0], t1421[0]), vandq_u32(t1420[1], t1421[1]), vandq_u32(t1420[2], t1421[2]), vandq_u32(t1420[3], t1421[3])];
    let t1423: u64 = ((arg0) as usize as u64);
    let t1424 = [vdupq_n_u32((t1423) as u32); 2];
    let t1425 = [vdupq_n_u32(((t1423) >> 32) as u32); 2];
    let t1426: [uint32x4_t; 2] = [vaddq_u32(t1424[0], t1422[0]), vaddq_u32(t1424[1], t1422[1])];
    let t1427: [uint32x4_t; 2] = [vcltq_u32(t1426[0], t1424[0]), vcltq_u32(t1426[1], t1424[1])];
    let t1428: [uint32x4_t; 4] = [t1426[0], t1426[1], vsubq_u32(vaddq_u32(t1425[0], t1422[2]), t1427[0]), vsubq_u32(vaddq_u32(t1425[1], t1422[3]), t1427[1])];
    x3 = t1428;
    let t1429 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([x3[0], x3[1]]);
    let t1430 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([x3[2], x3[3]]);
    let t1432 = bits(&live);
    let t1433 = t1432.trailing_zeros() as usize % 8;
    let t1431: [usize; 8] = [{ let j = if t1432 >> 0 & 1 != 0 { 0 } else { t1433 }; ((t1430[j] as u64) << 32 | t1429[j] as u64) as usize }, { let j = if t1432 >> 1 & 1 != 0 { 1 } else { t1433 }; ((t1430[j] as u64) << 32 | t1429[j] as u64) as usize }, { let j = if t1432 >> 2 & 1 != 0 { 2 } else { t1433 }; ((t1430[j] as u64) << 32 | t1429[j] as u64) as usize }, { let j = if t1432 >> 3 & 1 != 0 { 3 } else { t1433 }; ((t1430[j] as u64) << 32 | t1429[j] as u64) as usize }, { let j = if t1432 >> 4 & 1 != 0 { 4 } else { t1433 }; ((t1430[j] as u64) << 32 | t1429[j] as u64) as usize }, { let j = if t1432 >> 5 & 1 != 0 { 5 } else { t1433 }; ((t1430[j] as u64) << 32 | t1429[j] as u64) as usize }, { let j = if t1432 >> 6 & 1 != 0 { 6 } else { t1433 }; ((t1430[j] as u64) << 32 | t1429[j] as u64) as usize }, { let j = if t1432 >> 7 & 1 != 0 { 7 } else { t1433 }; ((t1430[j] as u64) << 32 | t1429[j] as u64) as usize }];
    let t1434: u64 = 7u64;
    let t1435 = [vdupq_n_u32((t1434) as u32); 2];
    let t1436: [uint32x4_t; 2] = [vandq_u32(x1[0], t1435[0]), vandq_u32(x1[1], t1435[1])];
    let t1437 = [vdupq_n_u32(8u32); 2];
    let t1438: [uint32x4_t; 2] = [vmulq_u32(t1436[0], t1437[0]), vmulq_u32(t1436[1], t1437[1])];
    x4 = t1438;
    let mut t1439: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (t1431[0] as *const u64).read_unaligned(); t1439[0] = vsetq_lane_u32::<0>(v as u32, t1439[0]); t1439[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1439[2]); }
    { let v = (t1431[1] as *const u64).read_unaligned(); t1439[0] = vsetq_lane_u32::<1>(v as u32, t1439[0]); t1439[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1439[2]); }
    { let v = (t1431[2] as *const u64).read_unaligned(); t1439[0] = vsetq_lane_u32::<2>(v as u32, t1439[0]); t1439[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1439[2]); }
    { let v = (t1431[3] as *const u64).read_unaligned(); t1439[0] = vsetq_lane_u32::<3>(v as u32, t1439[0]); t1439[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1439[2]); }
    { let v = (t1431[4] as *const u64).read_unaligned(); t1439[1] = vsetq_lane_u32::<0>(v as u32, t1439[1]); t1439[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1439[3]); }
    { let v = (t1431[5] as *const u64).read_unaligned(); t1439[1] = vsetq_lane_u32::<1>(v as u32, t1439[1]); t1439[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1439[3]); }
    { let v = (t1431[6] as *const u64).read_unaligned(); t1439[1] = vsetq_lane_u32::<2>(v as u32, t1439[1]); t1439[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1439[3]); }
    { let v = (t1431[7] as *const u64).read_unaligned(); t1439[1] = vsetq_lane_u32::<3>(v as u32, t1439[1]); t1439[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1439[3]); }
    let t1440: [uint32x4_t; 2] = [x4[0], x4[1]];
    let t1441: [uint32x4_t; 2] = [vandq_u32(t1440[0], vdupq_n_u32(63)), vandq_u32(t1440[1], vdupq_n_u32(63))];
    let t1442: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t1439[0], vnegq_s32(vreinterpretq_s32_u32(t1441[0]))), vshlq_u32(t1439[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1441[0])))), vorrq_u32(vshlq_u32(t1439[1], vnegq_s32(vreinterpretq_s32_u32(t1441[1]))), vshlq_u32(t1439[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1441[1])))), vshlq_u32(t1439[2], vnegq_s32(vreinterpretq_s32_u32(t1441[0]))), vshlq_u32(t1439[3], vnegq_s32(vreinterpretq_s32_u32(t1441[1])))];
    x5 = t1442;
    let t1443 = [vdupq_n_u32(0u32); 2];
    let t1444: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t1443[0])), vmvnq_u32(vceqq_u32(x4[1], t1443[1]))];
    let t1445 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
    let t1446: [uint32x4_t; 2] = [vaddq_u32(x4[0], t1445[0]), vaddq_u32(x4[1], t1445[1])];
    let t1447 = [vdupq_n_u32(64u32); 2];
    let t1448: [uint32x4_t; 2] = [vcgtq_u32(t1446[0], t1447[0]), vcgtq_u32(t1446[1], t1447[1])];
    let t1449: [uint32x4_t; 2] = [vandq_u32(t1444[0], t1448[0]), vandq_u32(t1444[1], t1448[1])];
    let mut t1450: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1451: [uint32x4_t; 2] = [vandq_u32(live[0], t1449[0]), vandq_u32(live[1], t1449[1])];
    let t1452: [uint32x4_t; 2] = [vbicq_u32(live[0], t1449[0]), vbicq_u32(live[1], t1449[1])];
    {
        let t1453: u64 = (1u64).wrapping_mul(8u64);
        let t1454 = [vdupq_n_u32((t1453) as u32); 2];
        let t1455 = [vdupq_n_u32(((t1453) >> 32) as u32); 2];
        let t1456: [uint32x4_t; 2] = [vaddq_u32(x3[0], t1454[0]), vaddq_u32(x3[1], t1454[1])];
        let t1457: [uint32x4_t; 2] = [vcltq_u32(t1456[0], x3[0]), vcltq_u32(t1456[1], x3[1])];
        let t1458: [uint32x4_t; 4] = [t1456[0], t1456[1], vsubq_u32(vaddq_u32(x3[2], t1455[0]), t1457[0]), vsubq_u32(vaddq_u32(x3[3], t1455[1]), t1457[1])];
        let t1459: [usize; 8] = core::array::from_fn(|j| t1431[j].wrapping_add(((1u64).wrapping_mul(8u64)) as u64 as usize));
        let mut t1460: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (t1459[0] as *const u64).read_unaligned(); t1460[0] = vsetq_lane_u32::<0>(v as u32, t1460[0]); t1460[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1460[2]); }
        { let v = (t1459[1] as *const u64).read_unaligned(); t1460[0] = vsetq_lane_u32::<1>(v as u32, t1460[0]); t1460[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1460[2]); }
        { let v = (t1459[2] as *const u64).read_unaligned(); t1460[0] = vsetq_lane_u32::<2>(v as u32, t1460[0]); t1460[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1460[2]); }
        { let v = (t1459[3] as *const u64).read_unaligned(); t1460[0] = vsetq_lane_u32::<3>(v as u32, t1460[0]); t1460[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1460[2]); }
        { let v = (t1459[4] as *const u64).read_unaligned(); t1460[1] = vsetq_lane_u32::<0>(v as u32, t1460[1]); t1460[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1460[3]); }
        { let v = (t1459[5] as *const u64).read_unaligned(); t1460[1] = vsetq_lane_u32::<1>(v as u32, t1460[1]); t1460[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1460[3]); }
        { let v = (t1459[6] as *const u64).read_unaligned(); t1460[1] = vsetq_lane_u32::<2>(v as u32, t1460[1]); t1460[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1460[3]); }
        { let v = (t1459[7] as *const u64).read_unaligned(); t1460[1] = vsetq_lane_u32::<3>(v as u32, t1460[1]); t1460[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1460[3]); }
        let t1461: [uint32x4_t; 2] = [vsubq_u32(t1447[0], x4[0]), vsubq_u32(t1447[1], x4[1])];
        let t1462: [uint32x4_t; 2] = [t1461[0], t1461[1]];
        let t1463: [uint32x4_t; 2] = [vandq_u32(t1462[0], vdupq_n_u32(63)), vandq_u32(t1462[1], vdupq_n_u32(63))];
        let t1464: [uint32x4_t; 4] = [vshlq_u32(t1460[0], vreinterpretq_s32_u32(t1463[0])), vshlq_u32(t1460[1], vreinterpretq_s32_u32(t1463[1])), vorrq_u32(vshlq_u32(t1460[2], vreinterpretq_s32_u32(t1463[0])), vshlq_u32(t1460[0], vsubq_s32(vreinterpretq_s32_u32(t1463[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1460[3], vreinterpretq_s32_u32(t1463[1])), vshlq_u32(t1460[1], vsubq_s32(vreinterpretq_s32_u32(t1463[1]), vdupq_n_s32(32))))];
        let t1465: [uint32x4_t; 4] = [vorrq_u32(x5[0], t1464[0]), vorrq_u32(x5[1], t1464[1]), vorrq_u32(x5[2], t1464[2]), vorrq_u32(x5[3], t1464[3])];
        t1450 = [vbslq_u32(t1451[0], t1465[0], t1450[0]), vbslq_u32(t1451[1], t1465[1], t1450[1]), vbslq_u32(t1451[0], t1465[2], t1450[2]), vbslq_u32(t1451[1], t1465[3], t1450[3])];
    }
    {
        t1450 = [vbslq_u32(t1452[0], x5[0], t1450[0]), vbslq_u32(t1452[1], x5[1], t1450[1]), vbslq_u32(t1452[0], x5[2], t1450[2]), vbslq_u32(t1452[1], x5[3], t1450[3])];
    }
    res = t1450;
    return res;
}

/// `ukl_ld_window`.
#[inline(always)]
unsafe fn b28_r0ls_f(l: &Launch, lid0: u32, arg0: *const u8, x1: [uint32x4_t; 4], x2: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x3: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x4: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x5: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if ((x2) == (8u32)) {
        let t1466: u64 = ((arg0) as usize as u64);
        let t1467 = [vdupq_n_u32((t1466) as u32); 2];
        let t1468 = [vdupq_n_u32(((t1466) >> 32) as u32); 2];
        let t1469: [uint32x4_t; 2] = [vaddq_u32(t1467[0], x1[0]), vaddq_u32(t1467[1], x1[1])];
        let t1470: [uint32x4_t; 2] = [vcltq_u32(t1469[0], t1467[0]), vcltq_u32(t1469[1], t1467[1])];
        let t1471: [uint32x4_t; 4] = [t1469[0], t1469[1], vsubq_u32(vaddq_u32(t1468[0], x1[2]), t1470[0]), vsubq_u32(vaddq_u32(t1468[1], x1[3]), t1470[1])];
        let t1472 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([t1471[0], t1471[1]]);
        let t1473 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([t1471[2], t1471[3]]);
        let t1474: [usize; 8] = [((t1473[0] as u64) << 32 | t1472[0] as u64) as usize, ((t1473[1] as u64) << 32 | t1472[1] as u64) as usize, ((t1473[2] as u64) << 32 | t1472[2] as u64) as usize, ((t1473[3] as u64) << 32 | t1472[3] as u64) as usize, ((t1473[4] as u64) << 32 | t1472[4] as u64) as usize, ((t1473[5] as u64) << 32 | t1472[5] as u64) as usize, ((t1473[6] as u64) << 32 | t1472[6] as u64) as usize, ((t1473[7] as u64) << 32 | t1472[7] as u64) as usize];
        let mut t1475: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (t1474[0] as *const u64).read_unaligned(); t1475[0] = vsetq_lane_u32::<0>(v as u32, t1475[0]); t1475[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1475[2]); }
        { let v = (t1474[1] as *const u64).read_unaligned(); t1475[0] = vsetq_lane_u32::<1>(v as u32, t1475[0]); t1475[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1475[2]); }
        { let v = (t1474[2] as *const u64).read_unaligned(); t1475[0] = vsetq_lane_u32::<2>(v as u32, t1475[0]); t1475[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1475[2]); }
        { let v = (t1474[3] as *const u64).read_unaligned(); t1475[0] = vsetq_lane_u32::<3>(v as u32, t1475[0]); t1475[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1475[2]); }
        { let v = (t1474[4] as *const u64).read_unaligned(); t1475[1] = vsetq_lane_u32::<0>(v as u32, t1475[1]); t1475[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1475[3]); }
        { let v = (t1474[5] as *const u64).read_unaligned(); t1475[1] = vsetq_lane_u32::<1>(v as u32, t1475[1]); t1475[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1475[3]); }
        { let v = (t1474[6] as *const u64).read_unaligned(); t1475[1] = vsetq_lane_u32::<2>(v as u32, t1475[1]); t1475[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1475[3]); }
        { let v = (t1474[7] as *const u64).read_unaligned(); t1475[1] = vsetq_lane_u32::<3>(v as u32, t1475[1]); t1475[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1475[3]); }
        res = t1475;
        return res;
    }
    if ((x2) == (4u32)) {
        let t1476: u64 = ((arg0) as usize as u64);
        let t1477 = [vdupq_n_u32((t1476) as u32); 2];
        let t1478 = [vdupq_n_u32(((t1476) >> 32) as u32); 2];
        let t1479: [uint32x4_t; 2] = [vaddq_u32(t1477[0], x1[0]), vaddq_u32(t1477[1], x1[1])];
        let t1480: [uint32x4_t; 2] = [vcltq_u32(t1479[0], t1477[0]), vcltq_u32(t1479[1], t1477[1])];
        let t1481: [uint32x4_t; 4] = [t1479[0], t1479[1], vsubq_u32(vaddq_u32(t1478[0], x1[2]), t1480[0]), vsubq_u32(vaddq_u32(t1478[1], x1[3]), t1480[1])];
        let t1482 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([t1481[0], t1481[1]]);
        let t1483 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([t1481[2], t1481[3]]);
        let t1484: [usize; 8] = [((t1483[0] as u64) << 32 | t1482[0] as u64) as usize, ((t1483[1] as u64) << 32 | t1482[1] as u64) as usize, ((t1483[2] as u64) << 32 | t1482[2] as u64) as usize, ((t1483[3] as u64) << 32 | t1482[3] as u64) as usize, ((t1483[4] as u64) << 32 | t1482[4] as u64) as usize, ((t1483[5] as u64) << 32 | t1482[5] as u64) as usize, ((t1483[6] as u64) << 32 | t1482[6] as u64) as usize, ((t1483[7] as u64) << 32 | t1482[7] as u64) as usize];
        let mut t1485: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t1485[0] = vsetq_lane_u32::<0>((t1484[0] as *const u32).read_unaligned() as u32, t1485[0]);
        t1485[0] = vsetq_lane_u32::<1>((t1484[1] as *const u32).read_unaligned() as u32, t1485[0]);
        t1485[0] = vsetq_lane_u32::<2>((t1484[2] as *const u32).read_unaligned() as u32, t1485[0]);
        t1485[0] = vsetq_lane_u32::<3>((t1484[3] as *const u32).read_unaligned() as u32, t1485[0]);
        t1485[1] = vsetq_lane_u32::<0>((t1484[4] as *const u32).read_unaligned() as u32, t1485[1]);
        t1485[1] = vsetq_lane_u32::<1>((t1484[5] as *const u32).read_unaligned() as u32, t1485[1]);
        t1485[1] = vsetq_lane_u32::<2>((t1484[6] as *const u32).read_unaligned() as u32, t1485[1]);
        t1485[1] = vsetq_lane_u32::<3>((t1484[7] as *const u32).read_unaligned() as u32, t1485[1]);
        let t1486: [uint32x4_t; 4] = [t1485[0], t1485[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1486;
        return res;
    }
    if ((x2) == (2u32)) {
        let t1487: u64 = ((arg0) as usize as u64);
        let t1488 = [vdupq_n_u32((t1487) as u32); 2];
        let t1489 = [vdupq_n_u32(((t1487) >> 32) as u32); 2];
        let t1490: [uint32x4_t; 2] = [vaddq_u32(t1488[0], x1[0]), vaddq_u32(t1488[1], x1[1])];
        let t1491: [uint32x4_t; 2] = [vcltq_u32(t1490[0], t1488[0]), vcltq_u32(t1490[1], t1488[1])];
        let t1492: [uint32x4_t; 4] = [t1490[0], t1490[1], vsubq_u32(vaddq_u32(t1489[0], x1[2]), t1491[0]), vsubq_u32(vaddq_u32(t1489[1], x1[3]), t1491[1])];
        let t1493 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([t1492[0], t1492[1]]);
        let t1494 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([t1492[2], t1492[3]]);
        let t1495: [usize; 8] = [((t1494[0] as u64) << 32 | t1493[0] as u64) as usize, ((t1494[1] as u64) << 32 | t1493[1] as u64) as usize, ((t1494[2] as u64) << 32 | t1493[2] as u64) as usize, ((t1494[3] as u64) << 32 | t1493[3] as u64) as usize, ((t1494[4] as u64) << 32 | t1493[4] as u64) as usize, ((t1494[5] as u64) << 32 | t1493[5] as u64) as usize, ((t1494[6] as u64) << 32 | t1493[6] as u64) as usize, ((t1494[7] as u64) << 32 | t1493[7] as u64) as usize];
        let mut t1496: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t1496[0] = vsetq_lane_u32::<0>((t1495[0] as *const u16).read_unaligned() as u32, t1496[0]);
        t1496[0] = vsetq_lane_u32::<1>((t1495[1] as *const u16).read_unaligned() as u32, t1496[0]);
        t1496[0] = vsetq_lane_u32::<2>((t1495[2] as *const u16).read_unaligned() as u32, t1496[0]);
        t1496[0] = vsetq_lane_u32::<3>((t1495[3] as *const u16).read_unaligned() as u32, t1496[0]);
        t1496[1] = vsetq_lane_u32::<0>((t1495[4] as *const u16).read_unaligned() as u32, t1496[1]);
        t1496[1] = vsetq_lane_u32::<1>((t1495[5] as *const u16).read_unaligned() as u32, t1496[1]);
        t1496[1] = vsetq_lane_u32::<2>((t1495[6] as *const u16).read_unaligned() as u32, t1496[1]);
        t1496[1] = vsetq_lane_u32::<3>((t1495[7] as *const u16).read_unaligned() as u32, t1496[1]);
        let t1497: [uint32x4_t; 4] = [t1496[0], t1496[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1497;
        return res;
    }
    if ((x2) == (1u32)) {
        let t1498: u64 = ((arg0) as usize as u64);
        let t1499 = [vdupq_n_u32((t1498) as u32); 2];
        let t1500 = [vdupq_n_u32(((t1498) >> 32) as u32); 2];
        let t1501: [uint32x4_t; 2] = [vaddq_u32(t1499[0], x1[0]), vaddq_u32(t1499[1], x1[1])];
        let t1502: [uint32x4_t; 2] = [vcltq_u32(t1501[0], t1499[0]), vcltq_u32(t1501[1], t1499[1])];
        let t1503: [uint32x4_t; 4] = [t1501[0], t1501[1], vsubq_u32(vaddq_u32(t1500[0], x1[2]), t1502[0]), vsubq_u32(vaddq_u32(t1500[1], x1[3]), t1502[1])];
        let t1504 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([t1503[0], t1503[1]]);
        let t1505 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([t1503[2], t1503[3]]);
        let t1506: [usize; 8] = [((t1505[0] as u64) << 32 | t1504[0] as u64) as usize, ((t1505[1] as u64) << 32 | t1504[1] as u64) as usize, ((t1505[2] as u64) << 32 | t1504[2] as u64) as usize, ((t1505[3] as u64) << 32 | t1504[3] as u64) as usize, ((t1505[4] as u64) << 32 | t1504[4] as u64) as usize, ((t1505[5] as u64) << 32 | t1504[5] as u64) as usize, ((t1505[6] as u64) << 32 | t1504[6] as u64) as usize, ((t1505[7] as u64) << 32 | t1504[7] as u64) as usize];
        let mut t1507: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t1507[0] = vsetq_lane_u32::<0>((t1506[0] as *const u8).read_unaligned() as u32, t1507[0]);
        t1507[0] = vsetq_lane_u32::<1>((t1506[1] as *const u8).read_unaligned() as u32, t1507[0]);
        t1507[0] = vsetq_lane_u32::<2>((t1506[2] as *const u8).read_unaligned() as u32, t1507[0]);
        t1507[0] = vsetq_lane_u32::<3>((t1506[3] as *const u8).read_unaligned() as u32, t1507[0]);
        t1507[1] = vsetq_lane_u32::<0>((t1506[4] as *const u8).read_unaligned() as u32, t1507[1]);
        t1507[1] = vsetq_lane_u32::<1>((t1506[5] as *const u8).read_unaligned() as u32, t1507[1]);
        t1507[1] = vsetq_lane_u32::<2>((t1506[6] as *const u8).read_unaligned() as u32, t1507[1]);
        t1507[1] = vsetq_lane_u32::<3>((t1506[7] as *const u8).read_unaligned() as u32, t1507[1]);
        let t1508: [uint32x4_t; 4] = [t1507[0], t1507[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1508;
        return res;
    }
    let t1509: u64 = !(7u64);
    let t1510 = [vdupq_n_u32((t1509) as u32); 2];
    let t1511 = [vdupq_n_u32(((t1509) >> 32) as u32); 2];
    let t1512: [uint32x4_t; 4] = [vandq_u32(x1[0], t1510[0]), vandq_u32(x1[1], t1510[1]), vandq_u32(x1[2], t1511[0]), vandq_u32(x1[3], t1511[1])];
    let t1513: u64 = ((arg0) as usize as u64);
    let t1514 = [vdupq_n_u32((t1513) as u32); 2];
    let t1515 = [vdupq_n_u32(((t1513) >> 32) as u32); 2];
    let t1516: [uint32x4_t; 2] = [vaddq_u32(t1514[0], t1512[0]), vaddq_u32(t1514[1], t1512[1])];
    let t1517: [uint32x4_t; 2] = [vcltq_u32(t1516[0], t1514[0]), vcltq_u32(t1516[1], t1514[1])];
    let t1518: [uint32x4_t; 4] = [t1516[0], t1516[1], vsubq_u32(vaddq_u32(t1515[0], t1512[2]), t1517[0]), vsubq_u32(vaddq_u32(t1515[1], t1512[3]), t1517[1])];
    x3 = t1518;
    let t1519 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([x3[0], x3[1]]);
    let t1520 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([x3[2], x3[3]]);
    let t1521: [usize; 8] = [((t1520[0] as u64) << 32 | t1519[0] as u64) as usize, ((t1520[1] as u64) << 32 | t1519[1] as u64) as usize, ((t1520[2] as u64) << 32 | t1519[2] as u64) as usize, ((t1520[3] as u64) << 32 | t1519[3] as u64) as usize, ((t1520[4] as u64) << 32 | t1519[4] as u64) as usize, ((t1520[5] as u64) << 32 | t1519[5] as u64) as usize, ((t1520[6] as u64) << 32 | t1519[6] as u64) as usize, ((t1520[7] as u64) << 32 | t1519[7] as u64) as usize];
    let t1522: u64 = 7u64;
    let t1523 = [vdupq_n_u32((t1522) as u32); 2];
    let t1524: [uint32x4_t; 2] = [vandq_u32(x1[0], t1523[0]), vandq_u32(x1[1], t1523[1])];
    let t1525 = [vdupq_n_u32(8u32); 2];
    let t1526: [uint32x4_t; 2] = [vmulq_u32(t1524[0], t1525[0]), vmulq_u32(t1524[1], t1525[1])];
    x4 = t1526;
    let mut t1527: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (t1521[0] as *const u64).read_unaligned(); t1527[0] = vsetq_lane_u32::<0>(v as u32, t1527[0]); t1527[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1527[2]); }
    { let v = (t1521[1] as *const u64).read_unaligned(); t1527[0] = vsetq_lane_u32::<1>(v as u32, t1527[0]); t1527[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1527[2]); }
    { let v = (t1521[2] as *const u64).read_unaligned(); t1527[0] = vsetq_lane_u32::<2>(v as u32, t1527[0]); t1527[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1527[2]); }
    { let v = (t1521[3] as *const u64).read_unaligned(); t1527[0] = vsetq_lane_u32::<3>(v as u32, t1527[0]); t1527[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1527[2]); }
    { let v = (t1521[4] as *const u64).read_unaligned(); t1527[1] = vsetq_lane_u32::<0>(v as u32, t1527[1]); t1527[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1527[3]); }
    { let v = (t1521[5] as *const u64).read_unaligned(); t1527[1] = vsetq_lane_u32::<1>(v as u32, t1527[1]); t1527[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1527[3]); }
    { let v = (t1521[6] as *const u64).read_unaligned(); t1527[1] = vsetq_lane_u32::<2>(v as u32, t1527[1]); t1527[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1527[3]); }
    { let v = (t1521[7] as *const u64).read_unaligned(); t1527[1] = vsetq_lane_u32::<3>(v as u32, t1527[1]); t1527[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1527[3]); }
    let t1528: [uint32x4_t; 2] = [x4[0], x4[1]];
    let t1529: [uint32x4_t; 2] = [vandq_u32(t1528[0], vdupq_n_u32(63)), vandq_u32(t1528[1], vdupq_n_u32(63))];
    let t1530: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t1527[0], vnegq_s32(vreinterpretq_s32_u32(t1529[0]))), vshlq_u32(t1527[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1529[0])))), vorrq_u32(vshlq_u32(t1527[1], vnegq_s32(vreinterpretq_s32_u32(t1529[1]))), vshlq_u32(t1527[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1529[1])))), vshlq_u32(t1527[2], vnegq_s32(vreinterpretq_s32_u32(t1529[0]))), vshlq_u32(t1527[3], vnegq_s32(vreinterpretq_s32_u32(t1529[1])))];
    x5 = t1530;
    let t1531 = [vdupq_n_u32(0u32); 2];
    let t1532: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t1531[0])), vmvnq_u32(vceqq_u32(x4[1], t1531[1]))];
    let t1533 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
    let t1534: [uint32x4_t; 2] = [vaddq_u32(x4[0], t1533[0]), vaddq_u32(x4[1], t1533[1])];
    let t1535 = [vdupq_n_u32(64u32); 2];
    let t1536: [uint32x4_t; 2] = [vcgtq_u32(t1534[0], t1535[0]), vcgtq_u32(t1534[1], t1535[1])];
    let t1537: [uint32x4_t; 2] = [vandq_u32(t1532[0], t1536[0]), vandq_u32(t1532[1], t1536[1])];
    let mut t1538: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1539: [uint32x4_t; 2] = [vmvnq_u32(t1537[0]), vmvnq_u32(t1537[1])];
    {
        let t1540: u64 = (1u64).wrapping_mul(8u64);
        let t1541 = [vdupq_n_u32((t1540) as u32); 2];
        let t1542 = [vdupq_n_u32(((t1540) >> 32) as u32); 2];
        let t1543: [uint32x4_t; 2] = [vaddq_u32(x3[0], t1541[0]), vaddq_u32(x3[1], t1541[1])];
        let t1544: [uint32x4_t; 2] = [vcltq_u32(t1543[0], x3[0]), vcltq_u32(t1543[1], x3[1])];
        let t1545: [uint32x4_t; 4] = [t1543[0], t1543[1], vsubq_u32(vaddq_u32(x3[2], t1542[0]), t1544[0]), vsubq_u32(vaddq_u32(x3[3], t1542[1]), t1544[1])];
        let t1546: [usize; 8] = core::array::from_fn(|j| t1521[j].wrapping_add(((1u64).wrapping_mul(8u64)) as u64 as usize));
        let mut t1547: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (t1546[0] as *const u64).read_unaligned(); t1547[0] = vsetq_lane_u32::<0>(v as u32, t1547[0]); t1547[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1547[2]); }
        { let v = (t1546[1] as *const u64).read_unaligned(); t1547[0] = vsetq_lane_u32::<1>(v as u32, t1547[0]); t1547[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1547[2]); }
        { let v = (t1546[2] as *const u64).read_unaligned(); t1547[0] = vsetq_lane_u32::<2>(v as u32, t1547[0]); t1547[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1547[2]); }
        { let v = (t1546[3] as *const u64).read_unaligned(); t1547[0] = vsetq_lane_u32::<3>(v as u32, t1547[0]); t1547[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1547[2]); }
        { let v = (t1546[4] as *const u64).read_unaligned(); t1547[1] = vsetq_lane_u32::<0>(v as u32, t1547[1]); t1547[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1547[3]); }
        { let v = (t1546[5] as *const u64).read_unaligned(); t1547[1] = vsetq_lane_u32::<1>(v as u32, t1547[1]); t1547[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1547[3]); }
        { let v = (t1546[6] as *const u64).read_unaligned(); t1547[1] = vsetq_lane_u32::<2>(v as u32, t1547[1]); t1547[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1547[3]); }
        { let v = (t1546[7] as *const u64).read_unaligned(); t1547[1] = vsetq_lane_u32::<3>(v as u32, t1547[1]); t1547[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1547[3]); }
        let t1548: [uint32x4_t; 2] = [vsubq_u32(t1535[0], x4[0]), vsubq_u32(t1535[1], x4[1])];
        let t1549: [uint32x4_t; 2] = [t1548[0], t1548[1]];
        let t1550: [uint32x4_t; 2] = [vandq_u32(t1549[0], vdupq_n_u32(63)), vandq_u32(t1549[1], vdupq_n_u32(63))];
        let t1551: [uint32x4_t; 4] = [vshlq_u32(t1547[0], vreinterpretq_s32_u32(t1550[0])), vshlq_u32(t1547[1], vreinterpretq_s32_u32(t1550[1])), vorrq_u32(vshlq_u32(t1547[2], vreinterpretq_s32_u32(t1550[0])), vshlq_u32(t1547[0], vsubq_s32(vreinterpretq_s32_u32(t1550[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1547[3], vreinterpretq_s32_u32(t1550[1])), vshlq_u32(t1547[1], vsubq_s32(vreinterpretq_s32_u32(t1550[1]), vdupq_n_s32(32))))];
        let t1552: [uint32x4_t; 4] = [vorrq_u32(x5[0], t1551[0]), vorrq_u32(x5[1], t1551[1]), vorrq_u32(x5[2], t1551[2]), vorrq_u32(x5[3], t1551[3])];
        t1538 = [vbslq_u32(t1537[0], t1552[0], t1538[0]), vbslq_u32(t1537[1], t1552[1], t1538[1]), vbslq_u32(t1537[0], t1552[2], t1538[2]), vbslq_u32(t1537[1], t1552[3], t1538[3])];
    }
    {
        t1538 = [vbslq_u32(t1539[0], x5[0], t1538[0]), vbslq_u32(t1539[1], x5[1], t1538[1]), vbslq_u32(t1539[0], x5[2], t1538[2]), vbslq_u32(t1539[1], x5[3], t1538[3])];
    }
    res = t1538;
    return res;
}

/// `ukl_ld_window`.
#[inline(always)]
unsafe fn b28_r0al1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1b: *const u8, x1i: [uint32x4_t; 2], x2: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x3: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x4: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x5: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if ((x2) == (8u32)) {
        let t1553 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], x1i[0]), vandq_u32(live[1], x1i[1]))));
        let t1554: [uint32x4_t; 2] = [vbslq_u32(live[0], x1i[0], t1553), vbslq_u32(live[1], x1i[1], t1553)];
        let mut t1555: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t1556 = vminvq_u32(vminq_u32(t1554[0], t1554[1]));
        if t1556 == vmaxvq_u32(vmaxq_u32(t1554[0], t1554[1])) {
            let t1557 = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(t1556 as usize * 1) as *const u64).read_unaligned();
            t1555 = [vdupq_n_u32(t1557 as u32), vdupq_n_u32(t1557 as u32), vdupq_n_u32((t1557 >> 32) as u32), vdupq_n_u32((t1557 >> 32) as u32)];
        } else {
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1554[0]) as usize * 1) as *const u64).read_unaligned(); t1555[0] = vsetq_lane_u32::<0>(v as u32, t1555[0]); t1555[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1555[2]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1554[0]) as usize * 1) as *const u64).read_unaligned(); t1555[0] = vsetq_lane_u32::<1>(v as u32, t1555[0]); t1555[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1555[2]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1554[0]) as usize * 1) as *const u64).read_unaligned(); t1555[0] = vsetq_lane_u32::<2>(v as u32, t1555[0]); t1555[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1555[2]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1554[0]) as usize * 1) as *const u64).read_unaligned(); t1555[0] = vsetq_lane_u32::<3>(v as u32, t1555[0]); t1555[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1555[2]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1554[1]) as usize * 1) as *const u64).read_unaligned(); t1555[1] = vsetq_lane_u32::<0>(v as u32, t1555[1]); t1555[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1555[3]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1554[1]) as usize * 1) as *const u64).read_unaligned(); t1555[1] = vsetq_lane_u32::<1>(v as u32, t1555[1]); t1555[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1555[3]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1554[1]) as usize * 1) as *const u64).read_unaligned(); t1555[1] = vsetq_lane_u32::<2>(v as u32, t1555[1]); t1555[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1555[3]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1554[1]) as usize * 1) as *const u64).read_unaligned(); t1555[1] = vsetq_lane_u32::<3>(v as u32, t1555[1]); t1555[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1555[3]); }
        }
        res = t1555;
        return res;
    }
    if ((x2) == (4u32)) {
        let t1558 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], x1i[0]), vandq_u32(live[1], x1i[1]))));
        let t1559: [uint32x4_t; 2] = [vbslq_u32(live[0], x1i[0], t1558), vbslq_u32(live[1], x1i[1], t1558)];
        let mut t1560: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t1561 = vminvq_u32(vminq_u32(t1559[0], t1559[1]));
        if t1561 == vmaxvq_u32(vmaxq_u32(t1559[0], t1559[1])) {
            t1560 = [vdupq_n_u32(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(t1561 as usize * 1) as *const u32).read_unaligned() as u32)); 2];
        } else {
            t1560[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1559[0]) as usize * 1) as *const u32).read_unaligned() as u32), t1560[0]);
            t1560[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1559[0]) as usize * 1) as *const u32).read_unaligned() as u32), t1560[0]);
            t1560[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1559[0]) as usize * 1) as *const u32).read_unaligned() as u32), t1560[0]);
            t1560[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1559[0]) as usize * 1) as *const u32).read_unaligned() as u32), t1560[0]);
            t1560[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1559[1]) as usize * 1) as *const u32).read_unaligned() as u32), t1560[1]);
            t1560[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1559[1]) as usize * 1) as *const u32).read_unaligned() as u32), t1560[1]);
            t1560[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1559[1]) as usize * 1) as *const u32).read_unaligned() as u32), t1560[1]);
            t1560[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1559[1]) as usize * 1) as *const u32).read_unaligned() as u32), t1560[1]);
        }
        let t1562: [uint32x4_t; 4] = [t1560[0], t1560[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1562;
        return res;
    }
    if ((x2) == (2u32)) {
        let t1563 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], x1i[0]), vandq_u32(live[1], x1i[1]))));
        let t1564: [uint32x4_t; 2] = [vbslq_u32(live[0], x1i[0], t1563), vbslq_u32(live[1], x1i[1], t1563)];
        let mut t1565: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t1566 = vminvq_u32(vminq_u32(t1564[0], t1564[1]));
        if t1566 == vmaxvq_u32(vmaxq_u32(t1564[0], t1564[1])) {
            t1565 = [vdupq_n_u32(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(t1566 as usize * 1) as *const u16).read_unaligned() as u32)); 2];
        } else {
            t1565[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1564[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1565[0]);
            t1565[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1564[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1565[0]);
            t1565[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1564[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1565[0]);
            t1565[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1564[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1565[0]);
            t1565[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1564[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1565[1]);
            t1565[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1564[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1565[1]);
            t1565[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1564[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1565[1]);
            t1565[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1564[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1565[1]);
        }
        let t1567: [uint32x4_t; 4] = [t1565[0], t1565[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1567;
        return res;
    }
    if ((x2) == (1u32)) {
        let t1568 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], x1i[0]), vandq_u32(live[1], x1i[1]))));
        let t1569: [uint32x4_t; 2] = [vbslq_u32(live[0], x1i[0], t1568), vbslq_u32(live[1], x1i[1], t1568)];
        let mut t1570: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t1571 = vminvq_u32(vminq_u32(t1569[0], t1569[1]));
        if t1571 == vmaxvq_u32(vmaxq_u32(t1569[0], t1569[1])) {
            t1570 = [vdupq_n_u32(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(t1571 as usize * 1) as *const u8).read_unaligned() as u32)); 2];
        } else {
            t1570[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1569[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1570[0]);
            t1570[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1569[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1570[0]);
            t1570[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1569[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1570[0]);
            t1570[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1569[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1570[0]);
            t1570[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1569[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1570[1]);
            t1570[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1569[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1570[1]);
            t1570[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1569[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1570[1]);
            t1570[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1569[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1570[1]);
        }
        let t1572: [uint32x4_t; 4] = [t1570[0], t1570[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1572;
        return res;
    }
    let t1573: [uint32x4_t; 2] = [x1i[0], x1i[1]];
    let t1574: u64 = ((x1b) as usize as u64);
    let t1575 = [vdupq_n_u32((t1574) as u32); 2];
    let t1576 = [vdupq_n_u32(((t1574) >> 32) as u32); 2];
    let t1577: [uint32x4_t; 2] = [vaddq_u32(t1575[0], t1573[0]), vaddq_u32(t1575[1], t1573[1])];
    let t1578: [uint32x4_t; 2] = [vcltq_u32(t1577[0], t1575[0]), vcltq_u32(t1577[1], t1575[1])];
    let t1579: [uint32x4_t; 4] = [t1577[0], t1577[1], vsubq_u32(vaddq_u32(t1576[0], vdupq_n_u32(0)), t1578[0]), vsubq_u32(vaddq_u32(t1576[1], vdupq_n_u32(0)), t1578[1])];
    let t1580: u64 = !(7u64);
    let t1581 = [vdupq_n_u32((t1580) as u32); 2];
    let t1582 = [vdupq_n_u32(((t1580) >> 32) as u32); 2];
    let t1583: [uint32x4_t; 4] = [vandq_u32(t1579[0], t1581[0]), vandq_u32(t1579[1], t1581[1]), vandq_u32(t1579[2], t1582[0]), vandq_u32(t1579[3], t1582[1])];
    let t1584: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
    let t1585: [uint32x4_t; 4] = [vandq_u32(t1583[0], t1584[0]), vandq_u32(t1583[1], t1584[1]), vandq_u32(t1583[2], t1584[2]), vandq_u32(t1583[3], t1584[3])];
    let t1586: u64 = ((arg0) as usize as u64);
    let t1587 = [vdupq_n_u32((t1586) as u32); 2];
    let t1588 = [vdupq_n_u32(((t1586) >> 32) as u32); 2];
    let t1589: [uint32x4_t; 2] = [vaddq_u32(t1587[0], t1585[0]), vaddq_u32(t1587[1], t1585[1])];
    let t1590: [uint32x4_t; 2] = [vcltq_u32(t1589[0], t1587[0]), vcltq_u32(t1589[1], t1587[1])];
    let t1591: [uint32x4_t; 4] = [t1589[0], t1589[1], vsubq_u32(vaddq_u32(t1588[0], t1585[2]), t1590[0]), vsubq_u32(vaddq_u32(t1588[1], t1585[3]), t1590[1])];
    x3 = t1591;
    let t1592 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([x3[0], x3[1]]);
    let t1593 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([x3[2], x3[3]]);
    let t1595 = bits(&live);
    let t1596 = t1595.trailing_zeros() as usize % 8;
    let t1594: [usize; 8] = [{ let j = if t1595 >> 0 & 1 != 0 { 0 } else { t1596 }; ((t1593[j] as u64) << 32 | t1592[j] as u64) as usize }, { let j = if t1595 >> 1 & 1 != 0 { 1 } else { t1596 }; ((t1593[j] as u64) << 32 | t1592[j] as u64) as usize }, { let j = if t1595 >> 2 & 1 != 0 { 2 } else { t1596 }; ((t1593[j] as u64) << 32 | t1592[j] as u64) as usize }, { let j = if t1595 >> 3 & 1 != 0 { 3 } else { t1596 }; ((t1593[j] as u64) << 32 | t1592[j] as u64) as usize }, { let j = if t1595 >> 4 & 1 != 0 { 4 } else { t1596 }; ((t1593[j] as u64) << 32 | t1592[j] as u64) as usize }, { let j = if t1595 >> 5 & 1 != 0 { 5 } else { t1596 }; ((t1593[j] as u64) << 32 | t1592[j] as u64) as usize }, { let j = if t1595 >> 6 & 1 != 0 { 6 } else { t1596 }; ((t1593[j] as u64) << 32 | t1592[j] as u64) as usize }, { let j = if t1595 >> 7 & 1 != 0 { 7 } else { t1596 }; ((t1593[j] as u64) << 32 | t1592[j] as u64) as usize }];
    let t1597: [uint32x4_t; 2] = [x1i[0], x1i[1]];
    let t1598: u64 = ((x1b) as usize as u64);
    let t1599 = [vdupq_n_u32((t1598) as u32); 2];
    let t1600 = [vdupq_n_u32(((t1598) >> 32) as u32); 2];
    let t1601: [uint32x4_t; 2] = [vaddq_u32(t1599[0], t1597[0]), vaddq_u32(t1599[1], t1597[1])];
    let t1602: [uint32x4_t; 2] = [vcltq_u32(t1601[0], t1599[0]), vcltq_u32(t1601[1], t1599[1])];
    let t1603: [uint32x4_t; 4] = [t1601[0], t1601[1], vsubq_u32(vaddq_u32(t1600[0], vdupq_n_u32(0)), t1602[0]), vsubq_u32(vaddq_u32(t1600[1], vdupq_n_u32(0)), t1602[1])];
    let t1604: u64 = 7u64;
    let t1605 = [vdupq_n_u32((t1604) as u32); 2];
    let t1606: [uint32x4_t; 2] = [vandq_u32(t1603[0], t1605[0]), vandq_u32(t1603[1], t1605[1])];
    let t1607 = [vdupq_n_u32(8u32); 2];
    let t1608: [uint32x4_t; 2] = [vmulq_u32(t1606[0], t1607[0]), vmulq_u32(t1606[1], t1607[1])];
    x4 = t1608;
    let mut t1609: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (t1594[0] as *const u64).read_unaligned(); t1609[0] = vsetq_lane_u32::<0>(v as u32, t1609[0]); t1609[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1609[2]); }
    { let v = (t1594[1] as *const u64).read_unaligned(); t1609[0] = vsetq_lane_u32::<1>(v as u32, t1609[0]); t1609[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1609[2]); }
    { let v = (t1594[2] as *const u64).read_unaligned(); t1609[0] = vsetq_lane_u32::<2>(v as u32, t1609[0]); t1609[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1609[2]); }
    { let v = (t1594[3] as *const u64).read_unaligned(); t1609[0] = vsetq_lane_u32::<3>(v as u32, t1609[0]); t1609[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1609[2]); }
    { let v = (t1594[4] as *const u64).read_unaligned(); t1609[1] = vsetq_lane_u32::<0>(v as u32, t1609[1]); t1609[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1609[3]); }
    { let v = (t1594[5] as *const u64).read_unaligned(); t1609[1] = vsetq_lane_u32::<1>(v as u32, t1609[1]); t1609[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1609[3]); }
    { let v = (t1594[6] as *const u64).read_unaligned(); t1609[1] = vsetq_lane_u32::<2>(v as u32, t1609[1]); t1609[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1609[3]); }
    { let v = (t1594[7] as *const u64).read_unaligned(); t1609[1] = vsetq_lane_u32::<3>(v as u32, t1609[1]); t1609[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1609[3]); }
    let t1610: [uint32x4_t; 2] = [x4[0], x4[1]];
    let t1611: [uint32x4_t; 2] = [vandq_u32(t1610[0], vdupq_n_u32(63)), vandq_u32(t1610[1], vdupq_n_u32(63))];
    let t1612: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t1609[0], vnegq_s32(vreinterpretq_s32_u32(t1611[0]))), vshlq_u32(t1609[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1611[0])))), vorrq_u32(vshlq_u32(t1609[1], vnegq_s32(vreinterpretq_s32_u32(t1611[1]))), vshlq_u32(t1609[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1611[1])))), vshlq_u32(t1609[2], vnegq_s32(vreinterpretq_s32_u32(t1611[0]))), vshlq_u32(t1609[3], vnegq_s32(vreinterpretq_s32_u32(t1611[1])))];
    x5 = t1612;
    let t1613 = [vdupq_n_u32(0u32); 2];
    let t1614: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t1613[0])), vmvnq_u32(vceqq_u32(x4[1], t1613[1]))];
    let t1615 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
    let t1616: [uint32x4_t; 2] = [vaddq_u32(x4[0], t1615[0]), vaddq_u32(x4[1], t1615[1])];
    let t1617 = [vdupq_n_u32(64u32); 2];
    let t1618: [uint32x4_t; 2] = [vcgtq_u32(t1616[0], t1617[0]), vcgtq_u32(t1616[1], t1617[1])];
    let t1619: [uint32x4_t; 2] = [vandq_u32(t1614[0], t1618[0]), vandq_u32(t1614[1], t1618[1])];
    let mut t1620: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1621: [uint32x4_t; 2] = [vandq_u32(live[0], t1619[0]), vandq_u32(live[1], t1619[1])];
    let t1622: [uint32x4_t; 2] = [vbicq_u32(live[0], t1619[0]), vbicq_u32(live[1], t1619[1])];
    {
        let t1623: u64 = (1u64).wrapping_mul(8u64);
        let t1624 = [vdupq_n_u32((t1623) as u32); 2];
        let t1625 = [vdupq_n_u32(((t1623) >> 32) as u32); 2];
        let t1626: [uint32x4_t; 2] = [vaddq_u32(x3[0], t1624[0]), vaddq_u32(x3[1], t1624[1])];
        let t1627: [uint32x4_t; 2] = [vcltq_u32(t1626[0], x3[0]), vcltq_u32(t1626[1], x3[1])];
        let t1628: [uint32x4_t; 4] = [t1626[0], t1626[1], vsubq_u32(vaddq_u32(x3[2], t1625[0]), t1627[0]), vsubq_u32(vaddq_u32(x3[3], t1625[1]), t1627[1])];
        let t1629: [usize; 8] = core::array::from_fn(|j| t1594[j].wrapping_add(((1u64).wrapping_mul(8u64)) as u64 as usize));
        let mut t1630: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (t1629[0] as *const u64).read_unaligned(); t1630[0] = vsetq_lane_u32::<0>(v as u32, t1630[0]); t1630[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1630[2]); }
        { let v = (t1629[1] as *const u64).read_unaligned(); t1630[0] = vsetq_lane_u32::<1>(v as u32, t1630[0]); t1630[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1630[2]); }
        { let v = (t1629[2] as *const u64).read_unaligned(); t1630[0] = vsetq_lane_u32::<2>(v as u32, t1630[0]); t1630[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1630[2]); }
        { let v = (t1629[3] as *const u64).read_unaligned(); t1630[0] = vsetq_lane_u32::<3>(v as u32, t1630[0]); t1630[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1630[2]); }
        { let v = (t1629[4] as *const u64).read_unaligned(); t1630[1] = vsetq_lane_u32::<0>(v as u32, t1630[1]); t1630[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1630[3]); }
        { let v = (t1629[5] as *const u64).read_unaligned(); t1630[1] = vsetq_lane_u32::<1>(v as u32, t1630[1]); t1630[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1630[3]); }
        { let v = (t1629[6] as *const u64).read_unaligned(); t1630[1] = vsetq_lane_u32::<2>(v as u32, t1630[1]); t1630[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1630[3]); }
        { let v = (t1629[7] as *const u64).read_unaligned(); t1630[1] = vsetq_lane_u32::<3>(v as u32, t1630[1]); t1630[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1630[3]); }
        let t1631: [uint32x4_t; 2] = [vsubq_u32(t1617[0], x4[0]), vsubq_u32(t1617[1], x4[1])];
        let t1632: [uint32x4_t; 2] = [t1631[0], t1631[1]];
        let t1633: [uint32x4_t; 2] = [vandq_u32(t1632[0], vdupq_n_u32(63)), vandq_u32(t1632[1], vdupq_n_u32(63))];
        let t1634: [uint32x4_t; 4] = [vshlq_u32(t1630[0], vreinterpretq_s32_u32(t1633[0])), vshlq_u32(t1630[1], vreinterpretq_s32_u32(t1633[1])), vorrq_u32(vshlq_u32(t1630[2], vreinterpretq_s32_u32(t1633[0])), vshlq_u32(t1630[0], vsubq_s32(vreinterpretq_s32_u32(t1633[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1630[3], vreinterpretq_s32_u32(t1633[1])), vshlq_u32(t1630[1], vsubq_s32(vreinterpretq_s32_u32(t1633[1]), vdupq_n_s32(32))))];
        let t1635: [uint32x4_t; 4] = [vorrq_u32(x5[0], t1634[0]), vorrq_u32(x5[1], t1634[1]), vorrq_u32(x5[2], t1634[2]), vorrq_u32(x5[3], t1634[3])];
        t1620 = [vbslq_u32(t1621[0], t1635[0], t1620[0]), vbslq_u32(t1621[1], t1635[1], t1620[1]), vbslq_u32(t1621[0], t1635[2], t1620[2]), vbslq_u32(t1621[1], t1635[3], t1620[3])];
    }
    {
        t1620 = [vbslq_u32(t1622[0], x5[0], t1620[0]), vbslq_u32(t1622[1], x5[1], t1620[1]), vbslq_u32(t1622[0], x5[2], t1620[2]), vbslq_u32(t1622[1], x5[3], t1620[3])];
    }
    res = t1620;
    return res;
}

/// `ukl_ld_window`.
#[inline(always)]
unsafe fn b28_r0an1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1b: *const u8, x1if: u32, x1is: u32, x2: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x3: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x4: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x5: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if ((x2) == (8u32)) {
        let t1636: [uint32x4_t; 4];
        if (x1is) == 8u32 && ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 && all(&live) {
            let t1637 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
            let t16380 = vld2q_u32((t1637 as *const u32).add(0));
            let t16381 = vld2q_u32((t1637 as *const u32).add(8));
            let t1638: [uint32x4_t; 4] = [t16380.0, t16381.0, t16380.1, t16381.1];
            t1636 = t1638;
        } else {
            let mut t1639: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            if ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 && all(&live) {
                let t1640 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
                let t1641 = ((x1is) as usize) * 1;
                { let v = (t1640.add(0 * t1641) as *const u64).read_unaligned(); t1639[0] = vsetq_lane_u32::<0>(v as u32, t1639[0]); t1639[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1639[2]); }
                { let v = (t1640.add(1 * t1641) as *const u64).read_unaligned(); t1639[0] = vsetq_lane_u32::<1>(v as u32, t1639[0]); t1639[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1639[2]); }
                { let v = (t1640.add(2 * t1641) as *const u64).read_unaligned(); t1639[0] = vsetq_lane_u32::<2>(v as u32, t1639[0]); t1639[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1639[2]); }
                { let v = (t1640.add(3 * t1641) as *const u64).read_unaligned(); t1639[0] = vsetq_lane_u32::<3>(v as u32, t1639[0]); t1639[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1639[2]); }
                { let v = (t1640.add(4 * t1641) as *const u64).read_unaligned(); t1639[1] = vsetq_lane_u32::<0>(v as u32, t1639[1]); t1639[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1639[3]); }
                { let v = (t1640.add(5 * t1641) as *const u64).read_unaligned(); t1639[1] = vsetq_lane_u32::<1>(v as u32, t1639[1]); t1639[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1639[3]); }
                { let v = (t1640.add(6 * t1641) as *const u64).read_unaligned(); t1639[1] = vsetq_lane_u32::<2>(v as u32, t1639[1]); t1639[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1639[3]); }
                { let v = (t1640.add(7 * t1641) as *const u64).read_unaligned(); t1639[1] = vsetq_lane_u32::<3>(v as u32, t1639[1]); t1639[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1639[3]); }
            } else {
                let t1642: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
                let t1643 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t1642[0]), vandq_u32(live[1], t1642[1]))));
                let t1644: [uint32x4_t; 2] = [vbslq_u32(live[0], t1642[0], t1643), vbslq_u32(live[1], t1642[1], t1643)];
                let mut t1645: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
                let t1646 = vminvq_u32(vminq_u32(t1644[0], t1644[1]));
                if t1646 == vmaxvq_u32(vmaxq_u32(t1644[0], t1644[1])) {
                    let t1647 = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(t1646 as usize * 1) as *const u64).read_unaligned();
                    t1645 = [vdupq_n_u32(t1647 as u32), vdupq_n_u32(t1647 as u32), vdupq_n_u32((t1647 >> 32) as u32), vdupq_n_u32((t1647 >> 32) as u32)];
                } else {
                    { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1644[0]) as usize * 1) as *const u64).read_unaligned(); t1645[0] = vsetq_lane_u32::<0>(v as u32, t1645[0]); t1645[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1645[2]); }
                    { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1644[0]) as usize * 1) as *const u64).read_unaligned(); t1645[0] = vsetq_lane_u32::<1>(v as u32, t1645[0]); t1645[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1645[2]); }
                    { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1644[0]) as usize * 1) as *const u64).read_unaligned(); t1645[0] = vsetq_lane_u32::<2>(v as u32, t1645[0]); t1645[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1645[2]); }
                    { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1644[0]) as usize * 1) as *const u64).read_unaligned(); t1645[0] = vsetq_lane_u32::<3>(v as u32, t1645[0]); t1645[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1645[2]); }
                    { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1644[1]) as usize * 1) as *const u64).read_unaligned(); t1645[1] = vsetq_lane_u32::<0>(v as u32, t1645[1]); t1645[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1645[3]); }
                    { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1644[1]) as usize * 1) as *const u64).read_unaligned(); t1645[1] = vsetq_lane_u32::<1>(v as u32, t1645[1]); t1645[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1645[3]); }
                    { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1644[1]) as usize * 1) as *const u64).read_unaligned(); t1645[1] = vsetq_lane_u32::<2>(v as u32, t1645[1]); t1645[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1645[3]); }
                    { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1644[1]) as usize * 1) as *const u64).read_unaligned(); t1645[1] = vsetq_lane_u32::<3>(v as u32, t1645[1]); t1645[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1645[3]); }
                }
                t1639 = t1645;
            }
            t1636 = t1639;
        }
        res = t1636;
        return res;
    }
    if ((x2) == (4u32)) {
        let t1648: [uint32x4_t; 2];
        if (x1is) == 4u32 && ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 && all(&live) {
            let t1649 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
            let t1650: [uint32x4_t; 2] = core::array::from_fn(|r| vld1q_u32((t1649 as *const u32).add(4 * r)));
            t1648 = t1650;
        } else {
            let mut t1651: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            if ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 && all(&live) {
                let t1652 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
                let t1653 = ((x1is) as usize) * 1;
                t1651[0] = vld1q_lane_u32::<0>(t1652.add(0 * t1653) as *const u32, t1651[0]);
                t1651[0] = vld1q_lane_u32::<1>(t1652.add(1 * t1653) as *const u32, t1651[0]);
                t1651[0] = vld1q_lane_u32::<2>(t1652.add(2 * t1653) as *const u32, t1651[0]);
                t1651[0] = vld1q_lane_u32::<3>(t1652.add(3 * t1653) as *const u32, t1651[0]);
                t1651[1] = vld1q_lane_u32::<0>(t1652.add(4 * t1653) as *const u32, t1651[1]);
                t1651[1] = vld1q_lane_u32::<1>(t1652.add(5 * t1653) as *const u32, t1651[1]);
                t1651[1] = vld1q_lane_u32::<2>(t1652.add(6 * t1653) as *const u32, t1651[1]);
                t1651[1] = vld1q_lane_u32::<3>(t1652.add(7 * t1653) as *const u32, t1651[1]);
            } else {
                let t1654: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
                let t1655 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t1654[0]), vandq_u32(live[1], t1654[1]))));
                let t1656: [uint32x4_t; 2] = [vbslq_u32(live[0], t1654[0], t1655), vbslq_u32(live[1], t1654[1], t1655)];
                let mut t1657: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                let t1658 = vminvq_u32(vminq_u32(t1656[0], t1656[1]));
                if t1658 == vmaxvq_u32(vmaxq_u32(t1656[0], t1656[1])) {
                    t1657 = [vdupq_n_u32(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(t1658 as usize * 1) as *const u32).read_unaligned() as u32)); 2];
                } else {
                    t1657[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1656[0]) as usize * 1) as *const u32).read_unaligned() as u32), t1657[0]);
                    t1657[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1656[0]) as usize * 1) as *const u32).read_unaligned() as u32), t1657[0]);
                    t1657[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1656[0]) as usize * 1) as *const u32).read_unaligned() as u32), t1657[0]);
                    t1657[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1656[0]) as usize * 1) as *const u32).read_unaligned() as u32), t1657[0]);
                    t1657[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1656[1]) as usize * 1) as *const u32).read_unaligned() as u32), t1657[1]);
                    t1657[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1656[1]) as usize * 1) as *const u32).read_unaligned() as u32), t1657[1]);
                    t1657[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1656[1]) as usize * 1) as *const u32).read_unaligned() as u32), t1657[1]);
                    t1657[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1656[1]) as usize * 1) as *const u32).read_unaligned() as u32), t1657[1]);
                }
                t1651 = t1657;
            }
            t1648 = t1651;
        }
        let t1659: [uint32x4_t; 4] = [t1648[0], t1648[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1659;
        return res;
    }
    if ((x2) == (2u32)) {
        let t1660: [uint32x4_t; 2];
        if (x1is) == 2u32 && ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 && all(&live) {
            let t1661 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
            let t1662: [uint32x4_t; 2] = core::array::from_fn(|r| vmovl_u16(vld1_u16((t1661 as *const u16).add(4 * r))));
            t1660 = t1662;
        } else {
            let mut t1663: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            if ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 && all(&live) {
                let t1664 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
                let t1665 = ((x1is) as usize) * 1;
                t1663[0] = vsetq_lane_u32::<0>((t1664.add(0 * t1665) as *const u16).read_unaligned() as u32, t1663[0]);
                t1663[0] = vsetq_lane_u32::<1>((t1664.add(1 * t1665) as *const u16).read_unaligned() as u32, t1663[0]);
                t1663[0] = vsetq_lane_u32::<2>((t1664.add(2 * t1665) as *const u16).read_unaligned() as u32, t1663[0]);
                t1663[0] = vsetq_lane_u32::<3>((t1664.add(3 * t1665) as *const u16).read_unaligned() as u32, t1663[0]);
                t1663[1] = vsetq_lane_u32::<0>((t1664.add(4 * t1665) as *const u16).read_unaligned() as u32, t1663[1]);
                t1663[1] = vsetq_lane_u32::<1>((t1664.add(5 * t1665) as *const u16).read_unaligned() as u32, t1663[1]);
                t1663[1] = vsetq_lane_u32::<2>((t1664.add(6 * t1665) as *const u16).read_unaligned() as u32, t1663[1]);
                t1663[1] = vsetq_lane_u32::<3>((t1664.add(7 * t1665) as *const u16).read_unaligned() as u32, t1663[1]);
            } else {
                let t1666: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
                let t1667 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t1666[0]), vandq_u32(live[1], t1666[1]))));
                let t1668: [uint32x4_t; 2] = [vbslq_u32(live[0], t1666[0], t1667), vbslq_u32(live[1], t1666[1], t1667)];
                let mut t1669: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                let t1670 = vminvq_u32(vminq_u32(t1668[0], t1668[1]));
                if t1670 == vmaxvq_u32(vmaxq_u32(t1668[0], t1668[1])) {
                    t1669 = [vdupq_n_u32(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(t1670 as usize * 1) as *const u16).read_unaligned() as u32)); 2];
                } else {
                    t1669[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1668[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1669[0]);
                    t1669[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1668[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1669[0]);
                    t1669[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1668[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1669[0]);
                    t1669[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1668[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1669[0]);
                    t1669[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1668[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1669[1]);
                    t1669[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1668[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1669[1]);
                    t1669[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1668[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1669[1]);
                    t1669[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1668[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1669[1]);
                }
                t1663 = t1669;
            }
            t1660 = t1663;
        }
        let t1671: [uint32x4_t; 4] = [t1660[0], t1660[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1671;
        return res;
    }
    if ((x2) == (1u32)) {
        let t1672: [uint32x4_t; 2];
        if (x1is) == 1u32 && ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 && all(&live) {
            let t1673 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
            let t1675 = vmovl_u8(vld1_u8(t1673.add(0)));
            let t1674: [uint32x4_t; 2] = [vmovl_u16(vget_low_u16(t1675)), vmovl_u16(vget_high_u16(t1675))];
            t1672 = t1674;
        } else {
            let mut t1676: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            if ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 && all(&live) {
                let t1677 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
                let t1678 = ((x1is) as usize) * 1;
                t1676[0] = vsetq_lane_u32::<0>((t1677.add(0 * t1678) as *const u8).read_unaligned() as u32, t1676[0]);
                t1676[0] = vsetq_lane_u32::<1>((t1677.add(1 * t1678) as *const u8).read_unaligned() as u32, t1676[0]);
                t1676[0] = vsetq_lane_u32::<2>((t1677.add(2 * t1678) as *const u8).read_unaligned() as u32, t1676[0]);
                t1676[0] = vsetq_lane_u32::<3>((t1677.add(3 * t1678) as *const u8).read_unaligned() as u32, t1676[0]);
                t1676[1] = vsetq_lane_u32::<0>((t1677.add(4 * t1678) as *const u8).read_unaligned() as u32, t1676[1]);
                t1676[1] = vsetq_lane_u32::<1>((t1677.add(5 * t1678) as *const u8).read_unaligned() as u32, t1676[1]);
                t1676[1] = vsetq_lane_u32::<2>((t1677.add(6 * t1678) as *const u8).read_unaligned() as u32, t1676[1]);
                t1676[1] = vsetq_lane_u32::<3>((t1677.add(7 * t1678) as *const u8).read_unaligned() as u32, t1676[1]);
            } else {
                let t1679: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
                let t1680 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t1679[0]), vandq_u32(live[1], t1679[1]))));
                let t1681: [uint32x4_t; 2] = [vbslq_u32(live[0], t1679[0], t1680), vbslq_u32(live[1], t1679[1], t1680)];
                let mut t1682: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                let t1683 = vminvq_u32(vminq_u32(t1681[0], t1681[1]));
                if t1683 == vmaxvq_u32(vmaxq_u32(t1681[0], t1681[1])) {
                    t1682 = [vdupq_n_u32(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(t1683 as usize * 1) as *const u8).read_unaligned() as u32)); 2];
                } else {
                    t1682[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1681[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1682[0]);
                    t1682[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1681[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1682[0]);
                    t1682[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1681[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1682[0]);
                    t1682[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1681[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1682[0]);
                    t1682[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1681[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1682[1]);
                    t1682[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1681[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1682[1]);
                    t1682[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1681[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1682[1]);
                    t1682[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1681[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1682[1]);
                }
                t1676 = t1682;
            }
            t1672 = t1676;
        }
        let t1684: [uint32x4_t; 4] = [t1672[0], t1672[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1684;
        return res;
    }
    let t1685: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
    let t1686: [uint32x4_t; 2] = [t1685[0], t1685[1]];
    let t1687: u64 = ((x1b) as usize as u64);
    let t1688 = [vdupq_n_u32((t1687) as u32); 2];
    let t1689 = [vdupq_n_u32(((t1687) >> 32) as u32); 2];
    let t1690: [uint32x4_t; 2] = [vaddq_u32(t1688[0], t1686[0]), vaddq_u32(t1688[1], t1686[1])];
    let t1691: [uint32x4_t; 2] = [vcltq_u32(t1690[0], t1688[0]), vcltq_u32(t1690[1], t1688[1])];
    let t1692: [uint32x4_t; 4] = [t1690[0], t1690[1], vsubq_u32(vaddq_u32(t1689[0], vdupq_n_u32(0)), t1691[0]), vsubq_u32(vaddq_u32(t1689[1], vdupq_n_u32(0)), t1691[1])];
    let t1693: u64 = !(7u64);
    let t1694 = [vdupq_n_u32((t1693) as u32); 2];
    let t1695 = [vdupq_n_u32(((t1693) >> 32) as u32); 2];
    let t1696: [uint32x4_t; 4] = [vandq_u32(t1692[0], t1694[0]), vandq_u32(t1692[1], t1694[1]), vandq_u32(t1692[2], t1695[0]), vandq_u32(t1692[3], t1695[1])];
    let t1697: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
    let t1698: [uint32x4_t; 4] = [vandq_u32(t1696[0], t1697[0]), vandq_u32(t1696[1], t1697[1]), vandq_u32(t1696[2], t1697[2]), vandq_u32(t1696[3], t1697[3])];
    let t1699: u64 = ((arg0) as usize as u64);
    let t1700 = [vdupq_n_u32((t1699) as u32); 2];
    let t1701 = [vdupq_n_u32(((t1699) >> 32) as u32); 2];
    let t1702: [uint32x4_t; 2] = [vaddq_u32(t1700[0], t1698[0]), vaddq_u32(t1700[1], t1698[1])];
    let t1703: [uint32x4_t; 2] = [vcltq_u32(t1702[0], t1700[0]), vcltq_u32(t1702[1], t1700[1])];
    let t1704: [uint32x4_t; 4] = [t1702[0], t1702[1], vsubq_u32(vaddq_u32(t1701[0], t1698[2]), t1703[0]), vsubq_u32(vaddq_u32(t1701[1], t1698[3]), t1703[1])];
    x3 = t1704;
    let t1705 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([x3[0], x3[1]]);
    let t1706 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([x3[2], x3[3]]);
    let t1708 = bits(&live);
    let t1709 = t1708.trailing_zeros() as usize % 8;
    let t1707: [usize; 8] = [{ let j = if t1708 >> 0 & 1 != 0 { 0 } else { t1709 }; ((t1706[j] as u64) << 32 | t1705[j] as u64) as usize }, { let j = if t1708 >> 1 & 1 != 0 { 1 } else { t1709 }; ((t1706[j] as u64) << 32 | t1705[j] as u64) as usize }, { let j = if t1708 >> 2 & 1 != 0 { 2 } else { t1709 }; ((t1706[j] as u64) << 32 | t1705[j] as u64) as usize }, { let j = if t1708 >> 3 & 1 != 0 { 3 } else { t1709 }; ((t1706[j] as u64) << 32 | t1705[j] as u64) as usize }, { let j = if t1708 >> 4 & 1 != 0 { 4 } else { t1709 }; ((t1706[j] as u64) << 32 | t1705[j] as u64) as usize }, { let j = if t1708 >> 5 & 1 != 0 { 5 } else { t1709 }; ((t1706[j] as u64) << 32 | t1705[j] as u64) as usize }, { let j = if t1708 >> 6 & 1 != 0 { 6 } else { t1709 }; ((t1706[j] as u64) << 32 | t1705[j] as u64) as usize }, { let j = if t1708 >> 7 & 1 != 0 { 7 } else { t1709 }; ((t1706[j] as u64) << 32 | t1705[j] as u64) as usize }];
    let t1710: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
    let t1711: [uint32x4_t; 2] = [t1710[0], t1710[1]];
    let t1712: u64 = ((x1b) as usize as u64);
    let t1713 = [vdupq_n_u32((t1712) as u32); 2];
    let t1714 = [vdupq_n_u32(((t1712) >> 32) as u32); 2];
    let t1715: [uint32x4_t; 2] = [vaddq_u32(t1713[0], t1711[0]), vaddq_u32(t1713[1], t1711[1])];
    let t1716: [uint32x4_t; 2] = [vcltq_u32(t1715[0], t1713[0]), vcltq_u32(t1715[1], t1713[1])];
    let t1717: [uint32x4_t; 4] = [t1715[0], t1715[1], vsubq_u32(vaddq_u32(t1714[0], vdupq_n_u32(0)), t1716[0]), vsubq_u32(vaddq_u32(t1714[1], vdupq_n_u32(0)), t1716[1])];
    let t1718: u64 = 7u64;
    let t1719 = [vdupq_n_u32((t1718) as u32); 2];
    let t1720: [uint32x4_t; 2] = [vandq_u32(t1717[0], t1719[0]), vandq_u32(t1717[1], t1719[1])];
    let t1721 = [vdupq_n_u32(8u32); 2];
    let t1722: [uint32x4_t; 2] = [vmulq_u32(t1720[0], t1721[0]), vmulq_u32(t1720[1], t1721[1])];
    x4 = t1722;
    let mut t1723: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (t1707[0] as *const u64).read_unaligned(); t1723[0] = vsetq_lane_u32::<0>(v as u32, t1723[0]); t1723[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1723[2]); }
    { let v = (t1707[1] as *const u64).read_unaligned(); t1723[0] = vsetq_lane_u32::<1>(v as u32, t1723[0]); t1723[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1723[2]); }
    { let v = (t1707[2] as *const u64).read_unaligned(); t1723[0] = vsetq_lane_u32::<2>(v as u32, t1723[0]); t1723[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1723[2]); }
    { let v = (t1707[3] as *const u64).read_unaligned(); t1723[0] = vsetq_lane_u32::<3>(v as u32, t1723[0]); t1723[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1723[2]); }
    { let v = (t1707[4] as *const u64).read_unaligned(); t1723[1] = vsetq_lane_u32::<0>(v as u32, t1723[1]); t1723[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1723[3]); }
    { let v = (t1707[5] as *const u64).read_unaligned(); t1723[1] = vsetq_lane_u32::<1>(v as u32, t1723[1]); t1723[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1723[3]); }
    { let v = (t1707[6] as *const u64).read_unaligned(); t1723[1] = vsetq_lane_u32::<2>(v as u32, t1723[1]); t1723[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1723[3]); }
    { let v = (t1707[7] as *const u64).read_unaligned(); t1723[1] = vsetq_lane_u32::<3>(v as u32, t1723[1]); t1723[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1723[3]); }
    let t1724: [uint32x4_t; 2] = [x4[0], x4[1]];
    let t1725: [uint32x4_t; 2] = [vandq_u32(t1724[0], vdupq_n_u32(63)), vandq_u32(t1724[1], vdupq_n_u32(63))];
    let t1726: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t1723[0], vnegq_s32(vreinterpretq_s32_u32(t1725[0]))), vshlq_u32(t1723[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1725[0])))), vorrq_u32(vshlq_u32(t1723[1], vnegq_s32(vreinterpretq_s32_u32(t1725[1]))), vshlq_u32(t1723[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1725[1])))), vshlq_u32(t1723[2], vnegq_s32(vreinterpretq_s32_u32(t1725[0]))), vshlq_u32(t1723[3], vnegq_s32(vreinterpretq_s32_u32(t1725[1])))];
    x5 = t1726;
    let t1727 = [vdupq_n_u32(0u32); 2];
    let t1728: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t1727[0])), vmvnq_u32(vceqq_u32(x4[1], t1727[1]))];
    let t1729 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
    let t1730: [uint32x4_t; 2] = [vaddq_u32(x4[0], t1729[0]), vaddq_u32(x4[1], t1729[1])];
    let t1731 = [vdupq_n_u32(64u32); 2];
    let t1732: [uint32x4_t; 2] = [vcgtq_u32(t1730[0], t1731[0]), vcgtq_u32(t1730[1], t1731[1])];
    let t1733: [uint32x4_t; 2] = [vandq_u32(t1728[0], t1732[0]), vandq_u32(t1728[1], t1732[1])];
    let mut t1734: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1735: [uint32x4_t; 2] = [vandq_u32(live[0], t1733[0]), vandq_u32(live[1], t1733[1])];
    let t1736: [uint32x4_t; 2] = [vbicq_u32(live[0], t1733[0]), vbicq_u32(live[1], t1733[1])];
    {
        let t1737: u64 = (1u64).wrapping_mul(8u64);
        let t1738 = [vdupq_n_u32((t1737) as u32); 2];
        let t1739 = [vdupq_n_u32(((t1737) >> 32) as u32); 2];
        let t1740: [uint32x4_t; 2] = [vaddq_u32(x3[0], t1738[0]), vaddq_u32(x3[1], t1738[1])];
        let t1741: [uint32x4_t; 2] = [vcltq_u32(t1740[0], x3[0]), vcltq_u32(t1740[1], x3[1])];
        let t1742: [uint32x4_t; 4] = [t1740[0], t1740[1], vsubq_u32(vaddq_u32(x3[2], t1739[0]), t1741[0]), vsubq_u32(vaddq_u32(x3[3], t1739[1]), t1741[1])];
        let t1743: [usize; 8] = core::array::from_fn(|j| t1707[j].wrapping_add(((1u64).wrapping_mul(8u64)) as u64 as usize));
        let mut t1744: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (t1743[0] as *const u64).read_unaligned(); t1744[0] = vsetq_lane_u32::<0>(v as u32, t1744[0]); t1744[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1744[2]); }
        { let v = (t1743[1] as *const u64).read_unaligned(); t1744[0] = vsetq_lane_u32::<1>(v as u32, t1744[0]); t1744[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1744[2]); }
        { let v = (t1743[2] as *const u64).read_unaligned(); t1744[0] = vsetq_lane_u32::<2>(v as u32, t1744[0]); t1744[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1744[2]); }
        { let v = (t1743[3] as *const u64).read_unaligned(); t1744[0] = vsetq_lane_u32::<3>(v as u32, t1744[0]); t1744[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1744[2]); }
        { let v = (t1743[4] as *const u64).read_unaligned(); t1744[1] = vsetq_lane_u32::<0>(v as u32, t1744[1]); t1744[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1744[3]); }
        { let v = (t1743[5] as *const u64).read_unaligned(); t1744[1] = vsetq_lane_u32::<1>(v as u32, t1744[1]); t1744[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1744[3]); }
        { let v = (t1743[6] as *const u64).read_unaligned(); t1744[1] = vsetq_lane_u32::<2>(v as u32, t1744[1]); t1744[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1744[3]); }
        { let v = (t1743[7] as *const u64).read_unaligned(); t1744[1] = vsetq_lane_u32::<3>(v as u32, t1744[1]); t1744[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1744[3]); }
        let t1745: [uint32x4_t; 2] = [vsubq_u32(t1731[0], x4[0]), vsubq_u32(t1731[1], x4[1])];
        let t1746: [uint32x4_t; 2] = [t1745[0], t1745[1]];
        let t1747: [uint32x4_t; 2] = [vandq_u32(t1746[0], vdupq_n_u32(63)), vandq_u32(t1746[1], vdupq_n_u32(63))];
        let t1748: [uint32x4_t; 4] = [vshlq_u32(t1744[0], vreinterpretq_s32_u32(t1747[0])), vshlq_u32(t1744[1], vreinterpretq_s32_u32(t1747[1])), vorrq_u32(vshlq_u32(t1744[2], vreinterpretq_s32_u32(t1747[0])), vshlq_u32(t1744[0], vsubq_s32(vreinterpretq_s32_u32(t1747[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1744[3], vreinterpretq_s32_u32(t1747[1])), vshlq_u32(t1744[1], vsubq_s32(vreinterpretq_s32_u32(t1747[1]), vdupq_n_s32(32))))];
        let t1749: [uint32x4_t; 4] = [vorrq_u32(x5[0], t1748[0]), vorrq_u32(x5[1], t1748[1]), vorrq_u32(x5[2], t1748[2]), vorrq_u32(x5[3], t1748[3])];
        t1734 = [vbslq_u32(t1735[0], t1749[0], t1734[0]), vbslq_u32(t1735[1], t1749[1], t1734[1]), vbslq_u32(t1735[0], t1749[2], t1734[2]), vbslq_u32(t1735[1], t1749[3], t1734[3])];
    }
    {
        t1734 = [vbslq_u32(t1736[0], x5[0], t1734[0]), vbslq_u32(t1736[1], x5[1], t1734[1]), vbslq_u32(t1736[0], x5[2], t1734[2]), vbslq_u32(t1736[1], x5[3], t1734[3])];
    }
    res = t1734;
    return res;
}

/// `ukl_ld_window`.
#[inline(always)]
unsafe fn b28_r0an1s_f(l: &Launch, lid0: u32, arg0: *const u8, x1b: *const u8, x1if: u32, x1is: u32, x2: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x3: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x4: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x5: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if ((x2) == (8u32)) {
        let t1750: [uint32x4_t; 4];
        if (x1is) == 8u32 && ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 {
            let t1751 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
            let t17520 = vld2q_u32((t1751 as *const u32).add(0));
            let t17521 = vld2q_u32((t1751 as *const u32).add(8));
            let t1752: [uint32x4_t; 4] = [t17520.0, t17521.0, t17520.1, t17521.1];
            t1750 = t1752;
        } else {
            let mut t1753: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            if ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 {
                let t1754 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
                let t1755 = ((x1is) as usize) * 1;
                { let v = (t1754.add(0 * t1755) as *const u64).read_unaligned(); t1753[0] = vsetq_lane_u32::<0>(v as u32, t1753[0]); t1753[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1753[2]); }
                { let v = (t1754.add(1 * t1755) as *const u64).read_unaligned(); t1753[0] = vsetq_lane_u32::<1>(v as u32, t1753[0]); t1753[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1753[2]); }
                { let v = (t1754.add(2 * t1755) as *const u64).read_unaligned(); t1753[0] = vsetq_lane_u32::<2>(v as u32, t1753[0]); t1753[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1753[2]); }
                { let v = (t1754.add(3 * t1755) as *const u64).read_unaligned(); t1753[0] = vsetq_lane_u32::<3>(v as u32, t1753[0]); t1753[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1753[2]); }
                { let v = (t1754.add(4 * t1755) as *const u64).read_unaligned(); t1753[1] = vsetq_lane_u32::<0>(v as u32, t1753[1]); t1753[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1753[3]); }
                { let v = (t1754.add(5 * t1755) as *const u64).read_unaligned(); t1753[1] = vsetq_lane_u32::<1>(v as u32, t1753[1]); t1753[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1753[3]); }
                { let v = (t1754.add(6 * t1755) as *const u64).read_unaligned(); t1753[1] = vsetq_lane_u32::<2>(v as u32, t1753[1]); t1753[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1753[3]); }
                { let v = (t1754.add(7 * t1755) as *const u64).read_unaligned(); t1753[1] = vsetq_lane_u32::<3>(v as u32, t1753[1]); t1753[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1753[3]); }
            } else {
                let t1756: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
                let mut t1757: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
                let t1758 = vminvq_u32(vminq_u32(t1756[0], t1756[1]));
                if t1758 == vmaxvq_u32(vmaxq_u32(t1756[0], t1756[1])) {
                    let t1759 = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(t1758 as usize * 1) as *const u64).read_unaligned();
                    t1757 = [vdupq_n_u32(t1759 as u32), vdupq_n_u32(t1759 as u32), vdupq_n_u32((t1759 >> 32) as u32), vdupq_n_u32((t1759 >> 32) as u32)];
                } else {
                    { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1756[0]) as usize * 1) as *const u64).read_unaligned(); t1757[0] = vsetq_lane_u32::<0>(v as u32, t1757[0]); t1757[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1757[2]); }
                    { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1756[0]) as usize * 1) as *const u64).read_unaligned(); t1757[0] = vsetq_lane_u32::<1>(v as u32, t1757[0]); t1757[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1757[2]); }
                    { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1756[0]) as usize * 1) as *const u64).read_unaligned(); t1757[0] = vsetq_lane_u32::<2>(v as u32, t1757[0]); t1757[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1757[2]); }
                    { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1756[0]) as usize * 1) as *const u64).read_unaligned(); t1757[0] = vsetq_lane_u32::<3>(v as u32, t1757[0]); t1757[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1757[2]); }
                    { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1756[1]) as usize * 1) as *const u64).read_unaligned(); t1757[1] = vsetq_lane_u32::<0>(v as u32, t1757[1]); t1757[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1757[3]); }
                    { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1756[1]) as usize * 1) as *const u64).read_unaligned(); t1757[1] = vsetq_lane_u32::<1>(v as u32, t1757[1]); t1757[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1757[3]); }
                    { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1756[1]) as usize * 1) as *const u64).read_unaligned(); t1757[1] = vsetq_lane_u32::<2>(v as u32, t1757[1]); t1757[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1757[3]); }
                    { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1756[1]) as usize * 1) as *const u64).read_unaligned(); t1757[1] = vsetq_lane_u32::<3>(v as u32, t1757[1]); t1757[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1757[3]); }
                }
                t1753 = t1757;
            }
            t1750 = t1753;
        }
        res = t1750;
        return res;
    }
    if ((x2) == (4u32)) {
        let t1760: [uint32x4_t; 2];
        if (x1is) == 4u32 && ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 {
            let t1761 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
            let t1762: [uint32x4_t; 2] = core::array::from_fn(|r| vld1q_u32((t1761 as *const u32).add(4 * r)));
            t1760 = t1762;
        } else {
            let mut t1763: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            if ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 {
                let t1764 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
                let t1765 = ((x1is) as usize) * 1;
                t1763[0] = vld1q_lane_u32::<0>(t1764.add(0 * t1765) as *const u32, t1763[0]);
                t1763[0] = vld1q_lane_u32::<1>(t1764.add(1 * t1765) as *const u32, t1763[0]);
                t1763[0] = vld1q_lane_u32::<2>(t1764.add(2 * t1765) as *const u32, t1763[0]);
                t1763[0] = vld1q_lane_u32::<3>(t1764.add(3 * t1765) as *const u32, t1763[0]);
                t1763[1] = vld1q_lane_u32::<0>(t1764.add(4 * t1765) as *const u32, t1763[1]);
                t1763[1] = vld1q_lane_u32::<1>(t1764.add(5 * t1765) as *const u32, t1763[1]);
                t1763[1] = vld1q_lane_u32::<2>(t1764.add(6 * t1765) as *const u32, t1763[1]);
                t1763[1] = vld1q_lane_u32::<3>(t1764.add(7 * t1765) as *const u32, t1763[1]);
            } else {
                let t1766: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
                let mut t1767: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                let t1768 = vminvq_u32(vminq_u32(t1766[0], t1766[1]));
                if t1768 == vmaxvq_u32(vmaxq_u32(t1766[0], t1766[1])) {
                    t1767 = [vdupq_n_u32(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(t1768 as usize * 1) as *const u32).read_unaligned() as u32)); 2];
                } else {
                    t1767[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1766[0]) as usize * 1) as *const u32).read_unaligned() as u32), t1767[0]);
                    t1767[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1766[0]) as usize * 1) as *const u32).read_unaligned() as u32), t1767[0]);
                    t1767[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1766[0]) as usize * 1) as *const u32).read_unaligned() as u32), t1767[0]);
                    t1767[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1766[0]) as usize * 1) as *const u32).read_unaligned() as u32), t1767[0]);
                    t1767[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1766[1]) as usize * 1) as *const u32).read_unaligned() as u32), t1767[1]);
                    t1767[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1766[1]) as usize * 1) as *const u32).read_unaligned() as u32), t1767[1]);
                    t1767[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1766[1]) as usize * 1) as *const u32).read_unaligned() as u32), t1767[1]);
                    t1767[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1766[1]) as usize * 1) as *const u32).read_unaligned() as u32), t1767[1]);
                }
                t1763 = t1767;
            }
            t1760 = t1763;
        }
        let t1769: [uint32x4_t; 4] = [t1760[0], t1760[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1769;
        return res;
    }
    if ((x2) == (2u32)) {
        let t1770: [uint32x4_t; 2];
        if (x1is) == 2u32 && ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 {
            let t1771 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
            let t1772: [uint32x4_t; 2] = core::array::from_fn(|r| vmovl_u16(vld1_u16((t1771 as *const u16).add(4 * r))));
            t1770 = t1772;
        } else {
            let mut t1773: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            if ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 {
                let t1774 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
                let t1775 = ((x1is) as usize) * 1;
                t1773[0] = vsetq_lane_u32::<0>((t1774.add(0 * t1775) as *const u16).read_unaligned() as u32, t1773[0]);
                t1773[0] = vsetq_lane_u32::<1>((t1774.add(1 * t1775) as *const u16).read_unaligned() as u32, t1773[0]);
                t1773[0] = vsetq_lane_u32::<2>((t1774.add(2 * t1775) as *const u16).read_unaligned() as u32, t1773[0]);
                t1773[0] = vsetq_lane_u32::<3>((t1774.add(3 * t1775) as *const u16).read_unaligned() as u32, t1773[0]);
                t1773[1] = vsetq_lane_u32::<0>((t1774.add(4 * t1775) as *const u16).read_unaligned() as u32, t1773[1]);
                t1773[1] = vsetq_lane_u32::<1>((t1774.add(5 * t1775) as *const u16).read_unaligned() as u32, t1773[1]);
                t1773[1] = vsetq_lane_u32::<2>((t1774.add(6 * t1775) as *const u16).read_unaligned() as u32, t1773[1]);
                t1773[1] = vsetq_lane_u32::<3>((t1774.add(7 * t1775) as *const u16).read_unaligned() as u32, t1773[1]);
            } else {
                let t1776: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
                let mut t1777: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                let t1778 = vminvq_u32(vminq_u32(t1776[0], t1776[1]));
                if t1778 == vmaxvq_u32(vmaxq_u32(t1776[0], t1776[1])) {
                    t1777 = [vdupq_n_u32(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(t1778 as usize * 1) as *const u16).read_unaligned() as u32)); 2];
                } else {
                    t1777[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1776[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1777[0]);
                    t1777[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1776[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1777[0]);
                    t1777[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1776[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1777[0]);
                    t1777[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1776[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1777[0]);
                    t1777[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1776[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1777[1]);
                    t1777[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1776[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1777[1]);
                    t1777[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1776[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1777[1]);
                    t1777[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1776[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1777[1]);
                }
                t1773 = t1777;
            }
            t1770 = t1773;
        }
        let t1779: [uint32x4_t; 4] = [t1770[0], t1770[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1779;
        return res;
    }
    if ((x2) == (1u32)) {
        let t1780: [uint32x4_t; 2];
        if (x1is) == 1u32 && ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 {
            let t1781 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
            let t1783 = vmovl_u8(vld1_u8(t1781.add(0)));
            let t1782: [uint32x4_t; 2] = [vmovl_u16(vget_low_u16(t1783)), vmovl_u16(vget_high_u16(t1783))];
            t1780 = t1782;
        } else {
            let mut t1784: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            if ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 {
                let t1785 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
                let t1786 = ((x1is) as usize) * 1;
                t1784[0] = vsetq_lane_u32::<0>((t1785.add(0 * t1786) as *const u8).read_unaligned() as u32, t1784[0]);
                t1784[0] = vsetq_lane_u32::<1>((t1785.add(1 * t1786) as *const u8).read_unaligned() as u32, t1784[0]);
                t1784[0] = vsetq_lane_u32::<2>((t1785.add(2 * t1786) as *const u8).read_unaligned() as u32, t1784[0]);
                t1784[0] = vsetq_lane_u32::<3>((t1785.add(3 * t1786) as *const u8).read_unaligned() as u32, t1784[0]);
                t1784[1] = vsetq_lane_u32::<0>((t1785.add(4 * t1786) as *const u8).read_unaligned() as u32, t1784[1]);
                t1784[1] = vsetq_lane_u32::<1>((t1785.add(5 * t1786) as *const u8).read_unaligned() as u32, t1784[1]);
                t1784[1] = vsetq_lane_u32::<2>((t1785.add(6 * t1786) as *const u8).read_unaligned() as u32, t1784[1]);
                t1784[1] = vsetq_lane_u32::<3>((t1785.add(7 * t1786) as *const u8).read_unaligned() as u32, t1784[1]);
            } else {
                let t1787: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
                let mut t1788: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                let t1789 = vminvq_u32(vminq_u32(t1787[0], t1787[1]));
                if t1789 == vmaxvq_u32(vmaxq_u32(t1787[0], t1787[1])) {
                    t1788 = [vdupq_n_u32(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(t1789 as usize * 1) as *const u8).read_unaligned() as u32)); 2];
                } else {
                    t1788[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1787[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1788[0]);
                    t1788[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1787[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1788[0]);
                    t1788[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1787[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1788[0]);
                    t1788[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1787[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1788[0]);
                    t1788[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1787[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1788[1]);
                    t1788[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1787[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1788[1]);
                    t1788[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1787[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1788[1]);
                    t1788[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1787[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1788[1]);
                }
                t1784 = t1788;
            }
            t1780 = t1784;
        }
        let t1790: [uint32x4_t; 4] = [t1780[0], t1780[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1790;
        return res;
    }
    let t1791: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
    let t1792: [uint32x4_t; 2] = [t1791[0], t1791[1]];
    let t1793: u64 = ((x1b) as usize as u64);
    let t1794 = [vdupq_n_u32((t1793) as u32); 2];
    let t1795 = [vdupq_n_u32(((t1793) >> 32) as u32); 2];
    let t1796: [uint32x4_t; 2] = [vaddq_u32(t1794[0], t1792[0]), vaddq_u32(t1794[1], t1792[1])];
    let t1797: [uint32x4_t; 2] = [vcltq_u32(t1796[0], t1794[0]), vcltq_u32(t1796[1], t1794[1])];
    let t1798: [uint32x4_t; 4] = [t1796[0], t1796[1], vsubq_u32(vaddq_u32(t1795[0], vdupq_n_u32(0)), t1797[0]), vsubq_u32(vaddq_u32(t1795[1], vdupq_n_u32(0)), t1797[1])];
    let t1799: u64 = !(7u64);
    let t1800 = [vdupq_n_u32((t1799) as u32); 2];
    let t1801 = [vdupq_n_u32(((t1799) >> 32) as u32); 2];
    let t1802: [uint32x4_t; 4] = [vandq_u32(t1798[0], t1800[0]), vandq_u32(t1798[1], t1800[1]), vandq_u32(t1798[2], t1801[0]), vandq_u32(t1798[3], t1801[1])];
    let t1803: u64 = ((arg0) as usize as u64);
    let t1804 = [vdupq_n_u32((t1803) as u32); 2];
    let t1805 = [vdupq_n_u32(((t1803) >> 32) as u32); 2];
    let t1806: [uint32x4_t; 2] = [vaddq_u32(t1804[0], t1802[0]), vaddq_u32(t1804[1], t1802[1])];
    let t1807: [uint32x4_t; 2] = [vcltq_u32(t1806[0], t1804[0]), vcltq_u32(t1806[1], t1804[1])];
    let t1808: [uint32x4_t; 4] = [t1806[0], t1806[1], vsubq_u32(vaddq_u32(t1805[0], t1802[2]), t1807[0]), vsubq_u32(vaddq_u32(t1805[1], t1802[3]), t1807[1])];
    x3 = t1808;
    let t1809 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([x3[0], x3[1]]);
    let t1810 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([x3[2], x3[3]]);
    let t1811: [usize; 8] = [((t1810[0] as u64) << 32 | t1809[0] as u64) as usize, ((t1810[1] as u64) << 32 | t1809[1] as u64) as usize, ((t1810[2] as u64) << 32 | t1809[2] as u64) as usize, ((t1810[3] as u64) << 32 | t1809[3] as u64) as usize, ((t1810[4] as u64) << 32 | t1809[4] as u64) as usize, ((t1810[5] as u64) << 32 | t1809[5] as u64) as usize, ((t1810[6] as u64) << 32 | t1809[6] as u64) as usize, ((t1810[7] as u64) << 32 | t1809[7] as u64) as usize];
    let t1812: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
    let t1813: [uint32x4_t; 2] = [t1812[0], t1812[1]];
    let t1814: u64 = ((x1b) as usize as u64);
    let t1815 = [vdupq_n_u32((t1814) as u32); 2];
    let t1816 = [vdupq_n_u32(((t1814) >> 32) as u32); 2];
    let t1817: [uint32x4_t; 2] = [vaddq_u32(t1815[0], t1813[0]), vaddq_u32(t1815[1], t1813[1])];
    let t1818: [uint32x4_t; 2] = [vcltq_u32(t1817[0], t1815[0]), vcltq_u32(t1817[1], t1815[1])];
    let t1819: [uint32x4_t; 4] = [t1817[0], t1817[1], vsubq_u32(vaddq_u32(t1816[0], vdupq_n_u32(0)), t1818[0]), vsubq_u32(vaddq_u32(t1816[1], vdupq_n_u32(0)), t1818[1])];
    let t1820: u64 = 7u64;
    let t1821 = [vdupq_n_u32((t1820) as u32); 2];
    let t1822: [uint32x4_t; 2] = [vandq_u32(t1819[0], t1821[0]), vandq_u32(t1819[1], t1821[1])];
    let t1823 = [vdupq_n_u32(8u32); 2];
    let t1824: [uint32x4_t; 2] = [vmulq_u32(t1822[0], t1823[0]), vmulq_u32(t1822[1], t1823[1])];
    x4 = t1824;
    let mut t1825: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (t1811[0] as *const u64).read_unaligned(); t1825[0] = vsetq_lane_u32::<0>(v as u32, t1825[0]); t1825[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1825[2]); }
    { let v = (t1811[1] as *const u64).read_unaligned(); t1825[0] = vsetq_lane_u32::<1>(v as u32, t1825[0]); t1825[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1825[2]); }
    { let v = (t1811[2] as *const u64).read_unaligned(); t1825[0] = vsetq_lane_u32::<2>(v as u32, t1825[0]); t1825[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1825[2]); }
    { let v = (t1811[3] as *const u64).read_unaligned(); t1825[0] = vsetq_lane_u32::<3>(v as u32, t1825[0]); t1825[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1825[2]); }
    { let v = (t1811[4] as *const u64).read_unaligned(); t1825[1] = vsetq_lane_u32::<0>(v as u32, t1825[1]); t1825[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1825[3]); }
    { let v = (t1811[5] as *const u64).read_unaligned(); t1825[1] = vsetq_lane_u32::<1>(v as u32, t1825[1]); t1825[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1825[3]); }
    { let v = (t1811[6] as *const u64).read_unaligned(); t1825[1] = vsetq_lane_u32::<2>(v as u32, t1825[1]); t1825[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1825[3]); }
    { let v = (t1811[7] as *const u64).read_unaligned(); t1825[1] = vsetq_lane_u32::<3>(v as u32, t1825[1]); t1825[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1825[3]); }
    let t1826: [uint32x4_t; 2] = [x4[0], x4[1]];
    let t1827: [uint32x4_t; 2] = [vandq_u32(t1826[0], vdupq_n_u32(63)), vandq_u32(t1826[1], vdupq_n_u32(63))];
    let t1828: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t1825[0], vnegq_s32(vreinterpretq_s32_u32(t1827[0]))), vshlq_u32(t1825[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1827[0])))), vorrq_u32(vshlq_u32(t1825[1], vnegq_s32(vreinterpretq_s32_u32(t1827[1]))), vshlq_u32(t1825[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1827[1])))), vshlq_u32(t1825[2], vnegq_s32(vreinterpretq_s32_u32(t1827[0]))), vshlq_u32(t1825[3], vnegq_s32(vreinterpretq_s32_u32(t1827[1])))];
    x5 = t1828;
    let t1829 = [vdupq_n_u32(0u32); 2];
    let t1830: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t1829[0])), vmvnq_u32(vceqq_u32(x4[1], t1829[1]))];
    let t1831 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
    let t1832: [uint32x4_t; 2] = [vaddq_u32(x4[0], t1831[0]), vaddq_u32(x4[1], t1831[1])];
    let t1833 = [vdupq_n_u32(64u32); 2];
    let t1834: [uint32x4_t; 2] = [vcgtq_u32(t1832[0], t1833[0]), vcgtq_u32(t1832[1], t1833[1])];
    let t1835: [uint32x4_t; 2] = [vandq_u32(t1830[0], t1834[0]), vandq_u32(t1830[1], t1834[1])];
    let mut t1836: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1837: [uint32x4_t; 2] = [vmvnq_u32(t1835[0]), vmvnq_u32(t1835[1])];
    {
        let t1838: u64 = (1u64).wrapping_mul(8u64);
        let t1839 = [vdupq_n_u32((t1838) as u32); 2];
        let t1840 = [vdupq_n_u32(((t1838) >> 32) as u32); 2];
        let t1841: [uint32x4_t; 2] = [vaddq_u32(x3[0], t1839[0]), vaddq_u32(x3[1], t1839[1])];
        let t1842: [uint32x4_t; 2] = [vcltq_u32(t1841[0], x3[0]), vcltq_u32(t1841[1], x3[1])];
        let t1843: [uint32x4_t; 4] = [t1841[0], t1841[1], vsubq_u32(vaddq_u32(x3[2], t1840[0]), t1842[0]), vsubq_u32(vaddq_u32(x3[3], t1840[1]), t1842[1])];
        let t1844: [usize; 8] = core::array::from_fn(|j| t1811[j].wrapping_add(((1u64).wrapping_mul(8u64)) as u64 as usize));
        let mut t1845: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (t1844[0] as *const u64).read_unaligned(); t1845[0] = vsetq_lane_u32::<0>(v as u32, t1845[0]); t1845[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1845[2]); }
        { let v = (t1844[1] as *const u64).read_unaligned(); t1845[0] = vsetq_lane_u32::<1>(v as u32, t1845[0]); t1845[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1845[2]); }
        { let v = (t1844[2] as *const u64).read_unaligned(); t1845[0] = vsetq_lane_u32::<2>(v as u32, t1845[0]); t1845[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1845[2]); }
        { let v = (t1844[3] as *const u64).read_unaligned(); t1845[0] = vsetq_lane_u32::<3>(v as u32, t1845[0]); t1845[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1845[2]); }
        { let v = (t1844[4] as *const u64).read_unaligned(); t1845[1] = vsetq_lane_u32::<0>(v as u32, t1845[1]); t1845[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1845[3]); }
        { let v = (t1844[5] as *const u64).read_unaligned(); t1845[1] = vsetq_lane_u32::<1>(v as u32, t1845[1]); t1845[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1845[3]); }
        { let v = (t1844[6] as *const u64).read_unaligned(); t1845[1] = vsetq_lane_u32::<2>(v as u32, t1845[1]); t1845[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1845[3]); }
        { let v = (t1844[7] as *const u64).read_unaligned(); t1845[1] = vsetq_lane_u32::<3>(v as u32, t1845[1]); t1845[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1845[3]); }
        let t1846: [uint32x4_t; 2] = [vsubq_u32(t1833[0], x4[0]), vsubq_u32(t1833[1], x4[1])];
        let t1847: [uint32x4_t; 2] = [t1846[0], t1846[1]];
        let t1848: [uint32x4_t; 2] = [vandq_u32(t1847[0], vdupq_n_u32(63)), vandq_u32(t1847[1], vdupq_n_u32(63))];
        let t1849: [uint32x4_t; 4] = [vshlq_u32(t1845[0], vreinterpretq_s32_u32(t1848[0])), vshlq_u32(t1845[1], vreinterpretq_s32_u32(t1848[1])), vorrq_u32(vshlq_u32(t1845[2], vreinterpretq_s32_u32(t1848[0])), vshlq_u32(t1845[0], vsubq_s32(vreinterpretq_s32_u32(t1848[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1845[3], vreinterpretq_s32_u32(t1848[1])), vshlq_u32(t1845[1], vsubq_s32(vreinterpretq_s32_u32(t1848[1]), vdupq_n_s32(32))))];
        let t1850: [uint32x4_t; 4] = [vorrq_u32(x5[0], t1849[0]), vorrq_u32(x5[1], t1849[1]), vorrq_u32(x5[2], t1849[2]), vorrq_u32(x5[3], t1849[3])];
        t1836 = [vbslq_u32(t1835[0], t1850[0], t1836[0]), vbslq_u32(t1835[1], t1850[1], t1836[1]), vbslq_u32(t1835[0], t1850[2], t1836[2]), vbslq_u32(t1835[1], t1850[3], t1836[3])];
    }
    {
        t1836 = [vbslq_u32(t1837[0], x5[0], t1836[0]), vbslq_u32(t1837[1], x5[1], t1836[1]), vbslq_u32(t1837[0], x5[2], t1836[2]), vbslq_u32(t1837[1], x5[3], t1836[3])];
    }
    res = t1836;
    return res;
}

/// `ukl_mask64`.
#[inline(always)]
unsafe fn b29_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t1851: u64;
    if ((x0) >= (64u32)) {
        t1851 = !(0u64);
    } else {
        t1851 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t1851;
    return res;
}

/// `ukl_mask64`.
#[inline(always)]
unsafe fn b29_s_f(l: &Launch, lid0: u32, x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t1852: u64;
    if ((x0) >= (64u32)) {
        t1852 = !(0u64);
    } else {
        t1852 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t1852;
    return res;
}

/// `clif_t_ld`.
#[inline(always)]
unsafe fn b30_r0sss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1: u32, x2: u32, x3: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u64 = 0;
    let mut live = k;
    let mut res: u64 = 0;
    let t1853 = b28_r0ss_p(l, lid0, live, arg0, (((x1) as u64)).wrapping_add(((x2) as u64)), x3);
    x4 = t1853;
    let t1854: u64;
    if ((x3) >= (8u32)) {
        t1854 = x4;
    } else {
        let t1855 = b29_s_p(l, lid0, live, (8u32).wrapping_mul(x3));
        t1854 = ((x4) & (t1855));
    }
    res = t1854;
    return res;
}

/// `clif_t_ld`.
#[inline(always)]
unsafe fn b30_r0sss_f(l: &Launch, lid0: u32, arg0: *const u8, x1: u32, x2: u32, x3: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u64 = 0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t1856 = b28_r0ss_f(l, lid0, arg0, (((x1) as u64)).wrapping_add(((x2) as u64)), x3);
    x4 = t1856;
    let t1857: u64;
    if ((x3) >= (8u32)) {
        t1857 = x4;
    } else {
        let t1858 = b29_s_f(l, lid0, (8u32).wrapping_mul(x3));
        t1857 = ((x4) & (t1858));
    }
    res = t1857;
    return res;
}

/// `clif_t_ld`.
#[inline(always)]
unsafe fn b30_r0sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1: u32, x2: [uint32x4_t; 2], x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1859 = b28_r0al1s_p(l, lid0, live, arg0, ((((x1) as u64)) as usize as *const u8), x2, x3);
    x4 = t1859;
    let t1860: [uint32x4_t; 4];
    if ((x3) >= (8u32)) {
        t1860 = x4;
    } else {
        let t1861 = b29_s_p(l, lid0, live, (8u32).wrapping_mul(x3));
        let t1862: u64 = t1861;
        let t1863 = [vdupq_n_u32((t1862) as u32); 2];
        let t1864 = [vdupq_n_u32(((t1862) >> 32) as u32); 2];
        let t1865: [uint32x4_t; 4] = [vandq_u32(x4[0], t1863[0]), vandq_u32(x4[1], t1863[1]), vandq_u32(x4[2], t1864[0]), vandq_u32(x4[3], t1864[1])];
        t1860 = t1865;
    }
    res = t1860;
    return res;
}

/// The group `l.group`'s threads, 8 at a time: `argv[j]` the address of argument `j`'s bytes (a buffer's address,
/// a value's bytes).
///
/// # Safety
///
/// `argv` holds the kernel's arguments, its buffers as large as it reads and writes; `l` is a 1-D group's launch.
pub unsafe fn parent_back_w8(argv: *const *const u8, l: &Launch) {
    let n = l.block[0] * l.block[1] * l.block[2];
    let arg0: *const u8 = (*argv.add(0) as *const *const u8).read_unaligned();
    let arg1: *const u8 = (*argv.add(1) as *const *const u8).read_unaligned();
    let arg2: *const u8 = (*argv.add(2) as *const *const u8).read_unaligned();
    let arg3: *const u8 = (*argv.add(3) as *const *const u8).read_unaligned();
    let arg4: *const u8 = (*argv.add(4) as *const *const u8).read_unaligned();
    let x5: *const u8 = *argv.add(5);
    let arg6: *const u8 = (*argv.add(6) as *const *const u8).read_unaligned();
    let x7: u32 = (*argv.add(7) as *const u32).read_unaligned();
    let mut t1866: bool = !(arg6).is_null();
    if t1866 {
        let mut t1867: bool = ((l.group[0]) >= ((arg6.wrapping_add((((x7) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        if !t1867 {
            t1867 = ((l.group[1]) >= ((arg6.wrapping_add(((((x7).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        let mut t1868: bool = t1867;
        if !t1868 {
            t1868 = ((l.group[2]) >= ((arg6.wrapping_add(((((x7).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        t1866 = t1868;
    }
    if t1866 {
        return;
    }
    let mut lid0 = 0u32;
    while lid0 + 8 <= n {
        let mut live = [vdupq_n_u32(u32::MAX); 2];
        let gid0 = l.group[0] * l.block[0] + lid0;
        'chunk: {
            if (lid0 & 31u32) <= 24u32 { b0_r0r1r2r3r4sgu5_f(l, lid0, arg0, arg1, arg2, arg3, arg4, x5, lid0) } else { b0_r0r1r2r3r4sgi_f(l, lid0, arg0, arg1, arg2, arg3, arg4, x5, lid0) };
        }
        lid0 += 8;
    }
    if lid0 < n {
        // Lanes part: the group's last threads, those under `n`.
        let t1869: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
        let mut live = t1869;
        let gid0 = l.group[0] * l.block[0] + lid0;
        'chunk: {
            if (lid0 & 31u32) <= 24u32 { b0_r0r1r2r3r4sgu5_p(l, lid0, live, arg0, arg1, arg2, arg3, arg4, x5, lid0) } else { b0_r0r1r2r3r4sgi_p(l, lid0, live, arg0, arg1, arg2, arg3, arg4, x5, lid0) };
        }
    }
}
