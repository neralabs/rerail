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

#[inline(always)]
unsafe fn b1_r3sso4_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg3: *const u8, x1: u32, x2: u32, arg4_x3: *const u8) -> bool {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u32 = 0;
    let mut x5: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x6: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x7: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x8: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
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
        let t60 = [vdupq_n_u32(0u32); 2];
        x8 = [vbslq_u32(live[0], t60[0], x8[0]), vbslq_u32(live[1], t60[1], x8[1])];
        let mut t61 = live;
        loop {
            // Lanes part: each its own rounds.
            let t63 = [vdupq_n_u32(65536u32); 2];
            let t64: [uint32x4_t; 2] = [vcltq_u32(x8[0], t63[0]), vcltq_u32(x8[1], t63[1])];
            t61 = [vandq_u32(t61[0], t64[0]), vandq_u32(t61[1], t64[1])];
            if !(any(&t61)) { break; }
            let t66 = b4_o4_p(l, lid0, t61, arg4_x3.wrapping_add((32u64).wrapping_mul(4u64) as usize));
            if ((t66) >= (x4)) {
                res = ((1i32) != 0);
                live = [vbicq_u32(live[0], t61[0]), vbicq_u32(live[1], t61[1])];
            }
            let t67: [uint32x4_t; 2] = [vandq_u32(t61[0], live[0]), vandq_u32(t61[1], live[1])];
            let t68 = [vdupq_n_u32(1u32); 2];
            let t69: [uint32x4_t; 2] = [vaddq_u32(x8[0], t68[0]), vaddq_u32(x8[1], t68[1])];
            let t70 = x8;
            x8 = t69;
            t61 = [vandq_u32(t61[0], live[0]), vandq_u32(t61[1], live[1])];
        }
        if any(&live) {
            res = ((0i32) != 0);
            live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
        }
    }
    res
}

#[inline(always)]
unsafe fn b1_r3sso4_f(l: &Launch, lid0: u32, arg3: *const u8, x1: u32, x2: u32, arg4_x3: *const u8) -> bool {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u32 = 0;
    let mut x5: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x6: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x7: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x8: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: bool = false;
    x4 = (((x1).wrapping_add(256u32)).wrapping_sub(1u32)).checked_div(256u32).unwrap_or(0);
    let mut t71 = live;
    let mut t72 = [vdupq_n_u32(0); 2];
    loop {
        // Lanes part: each its own rounds.
        let t73 = [vdupq_n_u32(if true { u32::MAX } else { 0 }); 2];
        t71 = [vandq_u32(vbicq_u32(t71[0], t72[0]), t73[0]), vandq_u32(vbicq_u32(t71[1], t72[1]), t73[1])];
        if !(any(&t71)) { break; }
        let t75 = b4_o4_p(l, lid0, t71, arg4_x3.wrapping_add((0u64).wrapping_mul(4u64) as usize));
        let t76: [uint32x4_t; 2];
        if ((t75) < (x4)) {
            let t77 = b2_o4s_p(l, lid0, t71, arg4_x3.wrapping_add((0u64).wrapping_mul(4u64) as usize), 1u32);
            t76 = t77;
        } else {
            let t78 = [vdupq_n_u32(x4); 2];
            t76 = t78;
        }
        x5 = t76;
        let t79 = [vdupq_n_u32(x4); 2];
        let t80: [uint32x4_t; 2] = [vcgeq_u32(x5[0], t79[0]), vcgeq_u32(x5[1], t79[1])];
        let t81: [uint32x4_t; 2] = [vandq_u32(t71[0], t80[0]), vandq_u32(t71[1], t80[1])];
        if any(&t81) {
            t72 = [vorrq_u32(t72[0], t81[0]), vorrq_u32(t72[1], t81[1])];
        }
        let t82: [uint32x4_t; 2] = [vbicq_u32(vandq_u32(t71[0], live[0]), t72[0]), vbicq_u32(vandq_u32(t71[1], live[1]), t72[1])];
        if any(&t82) {
            let t83 = [vdupq_n_u32(256u32); 2];
            let t84: [uint32x4_t; 2] = [vmulq_u32(x5[0], t83[0]), vmulq_u32(x5[1], t83[1])];
            let t85: [uint32x4_t; 2] = [vaddq_u32(t84[0], t83[0]), vaddq_u32(t84[1], t83[1])];
            let t86 = [vdupq_n_u32(x1); 2];
            let t87: [uint32x4_t; 2] = [vcltq_u32(t85[0], t86[0]), vcltq_u32(t85[1], t86[1])];
            let mut t88: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t89: [uint32x4_t; 2] = [vandq_u32(t82[0], t87[0]), vandq_u32(t82[1], t87[1])];
            let t90: [uint32x4_t; 2] = [vbicq_u32(t82[0], t87[0]), vbicq_u32(t82[1], t87[1])];
            {
                let t91: [uint32x4_t; 2] = [vmulq_u32(x5[0], t83[0]), vmulq_u32(x5[1], t83[1])];
                let t92: [uint32x4_t; 2] = [vaddq_u32(t91[0], t83[0]), vaddq_u32(t91[1], t83[1])];
                t88 = [vbslq_u32(t89[0], t92[0], t88[0]), vbslq_u32(t89[1], t92[1], t88[1])];
            }
            {
                let t93 = [vdupq_n_u32(x1); 2];
                t88 = [vbslq_u32(t90[0], t93[0], t88[0]), vbslq_u32(t90[1], t93[1], t88[1])];
            }
            x6 = t88;
            let t94: [uint32x4_t; 2] = [vmulq_u32(x5[0], t83[0]), vmulq_u32(x5[1], t83[1])];
            x7 = t94;
            let mut t96 = t82;
            loop {
                // Lanes part: each its own rounds.
                let t98: [uint32x4_t; 2] = [vcltq_u32(x7[0], x6[0]), vcltq_u32(x7[1], x6[1])];
                t96 = [vandq_u32(t96[0], t98[0]), vandq_u32(t96[1], t98[1])];
                if !(any(&t96)) { break; }
                let t100 = [vdupq_n_u32(x2); 2];
                let mut t101w = 0u32;
                let t101 = core::hint::black_box(&raw mut t101w);
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(t96[0]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<0>(x7[0]) as usize), t101).write(vgetq_lane_u32::<0>(t100[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(t96[0]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<1>(x7[0]) as usize), t101).write(vgetq_lane_u32::<1>(t100[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(t96[0]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<2>(x7[0]) as usize), t101).write(vgetq_lane_u32::<2>(t100[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(t96[0]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<3>(x7[0]) as usize), t101).write(vgetq_lane_u32::<3>(t100[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(t96[1]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<0>(x7[1]) as usize), t101).write(vgetq_lane_u32::<0>(t100[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(t96[1]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<1>(x7[1]) as usize), t101).write(vgetq_lane_u32::<1>(t100[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(t96[1]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<2>(x7[1]) as usize), t101).write(vgetq_lane_u32::<2>(t100[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(t96[1]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<3>(x7[1]) as usize), t101).write(vgetq_lane_u32::<3>(t100[1]));
                let t102: [uint32x4_t; 2] = [vandq_u32(t96[0], live[0]), vandq_u32(t96[1], live[1])];
                let t103 = x7;
                x7 = [vsubq_u32(x7[0], t102[0]), vsubq_u32(x7[1], t102[1])];
            }
            b5__p(l, lid0, t82);
            let t104 = b2_o4s_p(l, lid0, t82, arg4_x3.wrapping_add((32u64).wrapping_mul(4u64) as usize), 1u32);
        }
        let t105: [uint32x4_t; 2] = [vbicq_u32(vandq_u32(t71[0], live[0]), t72[0]), vbicq_u32(vandq_u32(t71[1], live[1]), t72[1])];
    }
    if any(&live) {
        let t106 = [vdupq_n_u32(0u32); 2];
        x8 = [vbslq_u32(live[0], t106[0], x8[0]), vbslq_u32(live[1], t106[1], x8[1])];
        let mut t107 = live;
        loop {
            // Lanes part: each its own rounds.
            let t109 = [vdupq_n_u32(65536u32); 2];
            let t110: [uint32x4_t; 2] = [vcltq_u32(x8[0], t109[0]), vcltq_u32(x8[1], t109[1])];
            t107 = [vandq_u32(t107[0], t110[0]), vandq_u32(t107[1], t110[1])];
            if !(any(&t107)) { break; }
            let t112 = b4_o4_p(l, lid0, t107, arg4_x3.wrapping_add((32u64).wrapping_mul(4u64) as usize));
            if ((t112) >= (x4)) {
                res = ((1i32) != 0);
                live = [vbicq_u32(live[0], t107[0]), vbicq_u32(live[1], t107[1])];
            }
            let t113: [uint32x4_t; 2] = [vandq_u32(t107[0], live[0]), vandq_u32(t107[1], live[1])];
            let t114 = [vdupq_n_u32(1u32); 2];
            let t115: [uint32x4_t; 2] = [vaddq_u32(x8[0], t114[0]), vaddq_u32(x8[1], t114[1])];
            let t116 = x8;
            x8 = t115;
            t107 = [vandq_u32(t107[0], live[0]), vandq_u32(t107[1], live[1])];
        }
        if any(&live) {
            res = ((0i32) != 0);
            live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
        }
    }
    res
}

#[inline(always)]
unsafe fn b2_o4s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg4_x0: *const u8, x1: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut t117 = [0u32; 8];
    for r in 0..2 { vst1q_u32(t117.as_mut_ptr().add(4 * r), live[r]); }
    let mut t118 = [0u32; 8];
    let mut t119 = 0u32;
    for j in 0..8 { t118[j] = t119; t119 += t117[j] & 1; }
    let t120: [uint32x4_t; 2] = [vld1q_u32(t118.as_ptr().add(0)), vld1q_u32(t118.as_ptr().add(4))];
    let t121 = AtomicU32::from_ptr(arg4_x0 as *mut u32).fetch_add((x1).wrapping_mul(t119), Relaxed);
    let t122: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(t121), t120[0], vdupq_n_u32(x1)), vmlaq_u32(vdupq_n_u32(t121), t120[1], vdupq_n_u32(x1))];
    res = t122;
    return res;
}

#[inline(always)]
unsafe fn b2_o4s_f(l: &Launch, lid0: u32, arg4_x0: *const u8, x1: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t123: [uint32x4_t; 2] = [vld1q_u32(LANE[0].as_ptr()), vld1q_u32(LANE[1].as_ptr())];
    let t124 = AtomicU32::from_ptr(arg4_x0 as *mut u32).fetch_add((x1).wrapping_mul(8u32), Relaxed);
    let t125: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(t124), t123[0], vdupq_n_u32(x1)), vmlaq_u32(vdupq_n_u32(t124), t123[1], vdupq_n_u32(x1))];
    res = t125;
    return res;
}

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
        let t126: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t127 = [vdupq_n_u32(x4); 2];
        let t128: [uint32x4_t; 2] = [vcltq_u32(t126[0], t127[0]), vcltq_u32(t126[1], t127[1])];
        let t129: [uint32x4_t; 2] = [vandq_u32(live[0], t128[0]), vandq_u32(live[1], t128[1])];
        if any(&t129) {
            let t130 = b7_r0r1g_p(l, lid0, t129, arg0, arg1);
            b6_ag4r2l_p(l, lid0, t129, arg2, t130);
        }
        return;
    }
    if any(&live) {
        let t131 = [vdupq_n_u32(4294967295u32); 2];
        x9 = t131;
        let t132: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t133 = [vdupq_n_u32(x4); 2];
        let t134: [uint32x4_t; 2] = [vcltq_u32(t132[0], t133[0]), vcltq_u32(t132[1], t133[1])];
        let t135: [uint32x4_t; 2] = [vandq_u32(live[0], t134[0]), vandq_u32(live[1], t134[1])];
        if any(&t135) {
            let t136 = b8_r0r1g_p(l, lid0, t135, arg0, arg1);
            x10 = t136;
            let t137: u64 = (1u64).wrapping_shl((32u64) as u32);
            let t138 = [vdupq_n_u32((t137) as u32); 2];
            let t139 = [vdupq_n_u32(((t137) >> 32) as u32); 2];
            let t140: [uint32x4_t; 4] = [vandq_u32(x10[0], t138[0]), vandq_u32(x10[1], t138[1]), vandq_u32(x10[2], t139[0]), vandq_u32(x10[3], t139[1])];
            let t141: u64 = 0u64;
            let t142 = [vdupq_n_u32((t141) as u32); 2];
            let t143: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t140[0], t142[0]), vceqq_u32(t140[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t140[1], t142[1]), vceqq_u32(t140[3], vdupq_n_u32(0)))];
            let t144: [uint32x4_t; 2] = [vmvnq_u32(t143[0]), vmvnq_u32(t143[1])];
            let mut t145: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t146: [uint32x4_t; 2] = [vandq_u32(t135[0], t144[0]), vandq_u32(t135[1], t144[1])];
            let t147: [uint32x4_t; 2] = [vbicq_u32(t135[0], t144[0]), vbicq_u32(t135[1], t144[1])];
            {
                let t148: [uint32x4_t; 2] = [x10[0], x10[1]];
                let t149 = [vdupq_n_u32(2147483648u32); 2];
                let t150: [uint32x4_t; 2] = [vorrq_u32(t149[0], t148[0]), vorrq_u32(t149[1], t148[1])];
                t145 = [vbslq_u32(t146[0], t150[0], t145[0]), vbslq_u32(t146[1], t150[1], t145[1])];
            }
            {
                let t151: [uint32x4_t; 2] = [x10[0], x10[1]];
                t145 = [vbslq_u32(t147[0], t151[0], t145[0]), vbslq_u32(t147[1], t151[1], t145[1])];
            }
            x9 = [vbslq_u32(t135[0], t145[0], x9[0]), vbslq_u32(t135[1], t145[1], x9[1])];
            b6_ag4r2l_p(l, lid0, t135, arg2, x9);
        }
        b9_r3gi_p(l, lid0, live, arg3, x7);
        let t152 = [vdupq_n_u32(x4); 2];
        let t153: [uint32x4_t; 2] = [vcgeq_u32(t132[0], t152[0]), vcgeq_u32(t132[1], t152[1])];
        let t154: [uint32x4_t; 2] = [vandq_u32(live[0], t153[0]), vandq_u32(live[1], t153[1])];
        {
            live = [vbicq_u32(live[0], t154[0]), vbicq_u32(live[1], t154[1])];
        }
        if any(&live) {
            x11 = 0u32;
            let mut t164 = live;
            loop {
                // Lanes part: each its own rounds.
                let t167 = [vdupq_n_u32(2147483648u32); 2];
                let t168: [uint32x4_t; 2] = [vandq_u32(x9[0], t167[0]), vandq_u32(x9[1], t167[1])];
                let t169 = [vdupq_n_u32(0u32); 2];
                let t170: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t168[0], t169[0])), vmvnq_u32(vceqq_u32(t168[1], t169[1]))];
                let t171 = [vdupq_n_u32(4294967294u32); 2];
                let t172: [uint32x4_t; 2] = [vcltq_u32(x9[0], t171[0]), vcltq_u32(x9[1], t171[1])];
                let t173: [uint32x4_t; 2] = [vandq_u32(t170[0], t172[0]), vandq_u32(t170[1], t172[1])];
                let t174: [uint32x4_t; 2] = [vandq_u32(vdupq_n_u32(if ((x11) < (7u32)) { u32::MAX } else { 0 }), t173[0]), vandq_u32(vdupq_n_u32(if ((x11) < (7u32)) { u32::MAX } else { 0 }), t173[1])];
                t164 = [vandq_u32(t164[0], t174[0]), vandq_u32(t164[1], t174[1])];
                if !(any(&t164)) { break; }
                let t176 = [vdupq_n_u32(2147483647u32); 2];
                let t177: [uint32x4_t; 2] = [vandq_u32(x9[0], t176[0]), vandq_u32(x9[1], t176[1])];
                x12 = t177;
                x13 = t171;
                let t178 = b10_r3ll_p(l, lid0, t164, arg3, x12, x6);
                let t179: [uint32x4_t; 2] = [vandq_u32(t164[0], t178[0]), vandq_u32(t164[1], t178[1])];
                if any(&t179) {
                    let t180 = b11_al4r2_p(l, lid0, t179, arg2, x12);
                    x13 = [vbslq_u32(t179[0], t180[0], x13[0]), vbslq_u32(t179[1], t180[1], x13[1])];
                }
                let t181: [uint32x4_t; 2] = [vceqq_u32(x13[0], t171[0]), vceqq_u32(x13[1], t171[1])];
                let t182: [uint32x4_t; 2] = [vandq_u32(t164[0], t181[0]), vandq_u32(t164[1], t181[1])];
                if any(&t182) {
                    let t183 = b8_r0r1l_p(l, lid0, t182, arg0, arg1, x12);
                    x14 = t183;
                    let t184: u64 = (1u64).wrapping_shl((32u64) as u32);
                    let t185 = [vdupq_n_u32((t184) as u32); 2];
                    let t186 = [vdupq_n_u32(((t184) >> 32) as u32); 2];
                    let t187: [uint32x4_t; 4] = [vandq_u32(x14[0], t185[0]), vandq_u32(x14[1], t185[1]), vandq_u32(x14[2], t186[0]), vandq_u32(x14[3], t186[1])];
                    let t188: u64 = 0u64;
                    let t189 = [vdupq_n_u32((t188) as u32); 2];
                    let t190: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t187[0], t189[0]), vceqq_u32(t187[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t187[1], t189[1]), vceqq_u32(t187[3], vdupq_n_u32(0)))];
                    let t191: [uint32x4_t; 2] = [vmvnq_u32(t190[0]), vmvnq_u32(t190[1])];
                    let mut t192: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                    let t193: [uint32x4_t; 2] = [vandq_u32(t182[0], t191[0]), vandq_u32(t182[1], t191[1])];
                    let t194: [uint32x4_t; 2] = [vbicq_u32(t182[0], t191[0]), vbicq_u32(t182[1], t191[1])];
                    {
                        let t195: [uint32x4_t; 2] = [x14[0], x14[1]];
                        let t196: [uint32x4_t; 2] = [vorrq_u32(t167[0], t195[0]), vorrq_u32(t167[1], t195[1])];
                        t192 = [vbslq_u32(t193[0], t196[0], t192[0]), vbslq_u32(t193[1], t196[1], t192[1])];
                    }
                    {
                        let t197: [uint32x4_t; 2] = [x14[0], x14[1]];
                        t192 = [vbslq_u32(t194[0], t197[0], t192[0]), vbslq_u32(t194[1], t197[1], t192[1])];
                    }
                    x13 = [vbslq_u32(t182[0], t192[0], x13[0]), vbslq_u32(t182[1], t192[1], x13[1])];
                }
                x9 = [vbslq_u32(t164[0], x13[0], x9[0]), vbslq_u32(t164[1], x13[1], x9[1])];
                let t198: [uint32x4_t; 2] = [vandq_u32(t164[0], live[0]), vandq_u32(t164[1], live[1])];
                let t199 = x11;
                x11 = (x11).wrapping_add(1u32);
            }
            let t200 = [vdupq_n_u32(2147483648u32); 2];
            let t201: [uint32x4_t; 2] = [vandq_u32(x9[0], t200[0]), vandq_u32(x9[1], t200[1])];
            let t202 = [vdupq_n_u32(0u32); 2];
            let t203: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t201[0], t202[0])), vmvnq_u32(vceqq_u32(t201[1], t202[1]))];
            let t204 = [vdupq_n_u32(4294967294u32); 2];
            let t205: [uint32x4_t; 2] = [vcltq_u32(x9[0], t204[0]), vcltq_u32(x9[1], t204[1])];
            let t206: [uint32x4_t; 2] = [vandq_u32(t203[0], t205[0]), vandq_u32(t203[1], t205[1])];
            let t207: [uint32x4_t; 2] = [vandq_u32(live[0], t206[0]), vandq_u32(live[1], t206[1])];
            if any(&t207) {
                let t208 = [vdupq_n_u32(2147483647u32); 2];
                let t209: [uint32x4_t; 2] = [vandq_u32(x9[0], t208[0]), vandq_u32(x9[1], t208[1])];
                let t210 = b7_r0r1l_p(l, lid0, t207, arg0, arg1, t209);
                x9 = [vbslq_u32(t207[0], t210[0], x9[0]), vbslq_u32(t207[1], t210[1], x9[1])];
            }
            b6_ag4r2l_p(l, lid0, live, arg2, x9);
        }
    }
}

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
            let t211 = b7_r0r1g_f(l, lid0, arg0, arg1);
            b6_ag4r2l_f(l, lid0, arg2, t211);
        } else {
            // Lanes part: the chunk's lanes on both sides of the bound.
            let t212: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
            let t213 = [vdupq_n_u32(x4); 2];
            let t214: [uint32x4_t; 2] = [vcltq_u32(t212[0], t213[0]), vcltq_u32(t212[1], t213[1])];
            let t215: [uint32x4_t; 2] = [vandq_u32(live[0], t214[0]), vandq_u32(live[1], t214[1])];
            if any(&t215) {
                let t216 = b7_r0r1g_p(l, lid0, t215, arg0, arg1);
                b6_ag4r2l_p(l, lid0, t215, arg2, t216);
            }
        }
        return;
    }
    if any(&live) {
        let t217 = [vdupq_n_u32(4294967295u32); 2];
        x9 = t217;
        let t218: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t219 = [vdupq_n_u32(x4); 2];
        let t220: [uint32x4_t; 2] = [vcltq_u32(t218[0], t219[0]), vcltq_u32(t218[1], t219[1])];
        let t221: [uint32x4_t; 2] = [vandq_u32(live[0], t220[0]), vandq_u32(live[1], t220[1])];
        if any(&t221) {
            let t222 = b8_r0r1g_p(l, lid0, t221, arg0, arg1);
            x10 = t222;
            let t223: u64 = (1u64).wrapping_shl((32u64) as u32);
            let t224 = [vdupq_n_u32((t223) as u32); 2];
            let t225 = [vdupq_n_u32(((t223) >> 32) as u32); 2];
            let t226: [uint32x4_t; 4] = [vandq_u32(x10[0], t224[0]), vandq_u32(x10[1], t224[1]), vandq_u32(x10[2], t225[0]), vandq_u32(x10[3], t225[1])];
            let t227: u64 = 0u64;
            let t228 = [vdupq_n_u32((t227) as u32); 2];
            let t229: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t226[0], t228[0]), vceqq_u32(t226[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t226[1], t228[1]), vceqq_u32(t226[3], vdupq_n_u32(0)))];
            let t230: [uint32x4_t; 2] = [vmvnq_u32(t229[0]), vmvnq_u32(t229[1])];
            let mut t231: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t232: [uint32x4_t; 2] = [vandq_u32(t221[0], t230[0]), vandq_u32(t221[1], t230[1])];
            let t233: [uint32x4_t; 2] = [vbicq_u32(t221[0], t230[0]), vbicq_u32(t221[1], t230[1])];
            {
                let t234: [uint32x4_t; 2] = [x10[0], x10[1]];
                let t235 = [vdupq_n_u32(2147483648u32); 2];
                let t236: [uint32x4_t; 2] = [vorrq_u32(t235[0], t234[0]), vorrq_u32(t235[1], t234[1])];
                t231 = [vbslq_u32(t232[0], t236[0], t231[0]), vbslq_u32(t232[1], t236[1], t231[1])];
            }
            {
                let t237: [uint32x4_t; 2] = [x10[0], x10[1]];
                t231 = [vbslq_u32(t233[0], t237[0], t231[0]), vbslq_u32(t233[1], t237[1], t231[1])];
            }
            x9 = [vbslq_u32(t221[0], t231[0], x9[0]), vbslq_u32(t221[1], t231[1], x9[1])];
            b6_ag4r2l_p(l, lid0, t221, arg2, x9);
        }
        b9_r3gi_p(l, lid0, live, arg3, x7);
        let t238 = [vdupq_n_u32(x4); 2];
        let t239: [uint32x4_t; 2] = [vcgeq_u32(t218[0], t238[0]), vcgeq_u32(t218[1], t238[1])];
        let t240: [uint32x4_t; 2] = [vandq_u32(live[0], t239[0]), vandq_u32(live[1], t239[1])];
        {
            live = [vbicq_u32(live[0], t240[0]), vbicq_u32(live[1], t240[1])];
        }
        if any(&live) {
            x11 = 0u32;
            let mut t250 = live;
            loop {
                // Lanes part: each its own rounds.
                let t253 = [vdupq_n_u32(2147483648u32); 2];
                let t254: [uint32x4_t; 2] = [vandq_u32(x9[0], t253[0]), vandq_u32(x9[1], t253[1])];
                let t255 = [vdupq_n_u32(0u32); 2];
                let t256: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t254[0], t255[0])), vmvnq_u32(vceqq_u32(t254[1], t255[1]))];
                let t257 = [vdupq_n_u32(4294967294u32); 2];
                let t258: [uint32x4_t; 2] = [vcltq_u32(x9[0], t257[0]), vcltq_u32(x9[1], t257[1])];
                let t259: [uint32x4_t; 2] = [vandq_u32(t256[0], t258[0]), vandq_u32(t256[1], t258[1])];
                let t260: [uint32x4_t; 2] = [vandq_u32(vdupq_n_u32(if ((x11) < (7u32)) { u32::MAX } else { 0 }), t259[0]), vandq_u32(vdupq_n_u32(if ((x11) < (7u32)) { u32::MAX } else { 0 }), t259[1])];
                t250 = [vandq_u32(t250[0], t260[0]), vandq_u32(t250[1], t260[1])];
                if !(any(&t250)) { break; }
                let t262 = [vdupq_n_u32(2147483647u32); 2];
                let t263: [uint32x4_t; 2] = [vandq_u32(x9[0], t262[0]), vandq_u32(x9[1], t262[1])];
                x12 = t263;
                x13 = t257;
                let t264 = b10_r3ll_p(l, lid0, t250, arg3, x12, x6);
                let t265: [uint32x4_t; 2] = [vandq_u32(t250[0], t264[0]), vandq_u32(t250[1], t264[1])];
                if any(&t265) {
                    let t266 = b11_al4r2_p(l, lid0, t265, arg2, x12);
                    x13 = [vbslq_u32(t265[0], t266[0], x13[0]), vbslq_u32(t265[1], t266[1], x13[1])];
                }
                let t267: [uint32x4_t; 2] = [vceqq_u32(x13[0], t257[0]), vceqq_u32(x13[1], t257[1])];
                let t268: [uint32x4_t; 2] = [vandq_u32(t250[0], t267[0]), vandq_u32(t250[1], t267[1])];
                if any(&t268) {
                    let t269 = b8_r0r1l_p(l, lid0, t268, arg0, arg1, x12);
                    x14 = t269;
                    let t270: u64 = (1u64).wrapping_shl((32u64) as u32);
                    let t271 = [vdupq_n_u32((t270) as u32); 2];
                    let t272 = [vdupq_n_u32(((t270) >> 32) as u32); 2];
                    let t273: [uint32x4_t; 4] = [vandq_u32(x14[0], t271[0]), vandq_u32(x14[1], t271[1]), vandq_u32(x14[2], t272[0]), vandq_u32(x14[3], t272[1])];
                    let t274: u64 = 0u64;
                    let t275 = [vdupq_n_u32((t274) as u32); 2];
                    let t276: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t273[0], t275[0]), vceqq_u32(t273[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t273[1], t275[1]), vceqq_u32(t273[3], vdupq_n_u32(0)))];
                    let t277: [uint32x4_t; 2] = [vmvnq_u32(t276[0]), vmvnq_u32(t276[1])];
                    let mut t278: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                    let t279: [uint32x4_t; 2] = [vandq_u32(t268[0], t277[0]), vandq_u32(t268[1], t277[1])];
                    let t280: [uint32x4_t; 2] = [vbicq_u32(t268[0], t277[0]), vbicq_u32(t268[1], t277[1])];
                    {
                        let t281: [uint32x4_t; 2] = [x14[0], x14[1]];
                        let t282: [uint32x4_t; 2] = [vorrq_u32(t253[0], t281[0]), vorrq_u32(t253[1], t281[1])];
                        t278 = [vbslq_u32(t279[0], t282[0], t278[0]), vbslq_u32(t279[1], t282[1], t278[1])];
                    }
                    {
                        let t283: [uint32x4_t; 2] = [x14[0], x14[1]];
                        t278 = [vbslq_u32(t280[0], t283[0], t278[0]), vbslq_u32(t280[1], t283[1], t278[1])];
                    }
                    x13 = [vbslq_u32(t268[0], t278[0], x13[0]), vbslq_u32(t268[1], t278[1], x13[1])];
                }
                x9 = [vbslq_u32(t250[0], x13[0], x9[0]), vbslq_u32(t250[1], x13[1], x9[1])];
                let t284: [uint32x4_t; 2] = [vandq_u32(t250[0], live[0]), vandq_u32(t250[1], live[1])];
                let t285 = x11;
                x11 = (x11).wrapping_add(1u32);
            }
            let t286 = [vdupq_n_u32(2147483648u32); 2];
            let t287: [uint32x4_t; 2] = [vandq_u32(x9[0], t286[0]), vandq_u32(x9[1], t286[1])];
            let t288 = [vdupq_n_u32(0u32); 2];
            let t289: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t287[0], t288[0])), vmvnq_u32(vceqq_u32(t287[1], t288[1]))];
            let t290 = [vdupq_n_u32(4294967294u32); 2];
            let t291: [uint32x4_t; 2] = [vcltq_u32(x9[0], t290[0]), vcltq_u32(x9[1], t290[1])];
            let t292: [uint32x4_t; 2] = [vandq_u32(t289[0], t291[0]), vandq_u32(t289[1], t291[1])];
            let t293: [uint32x4_t; 2] = [vandq_u32(live[0], t292[0]), vandq_u32(live[1], t292[1])];
            if any(&t293) {
                let t294 = [vdupq_n_u32(2147483647u32); 2];
                let t295: [uint32x4_t; 2] = [vandq_u32(x9[0], t294[0]), vandq_u32(x9[1], t294[1])];
                let t296 = b7_r0r1l_p(l, lid0, t293, arg0, arg1, t295);
                x9 = [vbslq_u32(t293[0], t296[0], x9[0]), vbslq_u32(t293[1], t296[1], x9[1])];
            }
            b6_ag4r2l_p(l, lid0, live, arg2, x9);
        }
    }
}

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
        let t297: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t298 = [vdupq_n_u32(x4); 2];
        let t299: [uint32x4_t; 2] = [vcltq_u32(t297[0], t298[0]), vcltq_u32(t297[1], t298[1])];
        let t300: [uint32x4_t; 2] = [vandq_u32(live[0], t299[0]), vandq_u32(live[1], t299[1])];
        if any(&t300) {
            let t301 = b7_r0r1g_p(l, lid0, t300, arg0, arg1);
            b6_ag4r2l_p(l, lid0, t300, arg2, t301);
        }
        return;
    }
    if any(&live) {
        let t302 = [vdupq_n_u32(4294967295u32); 2];
        x9 = t302;
        let t303: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t304 = [vdupq_n_u32(x4); 2];
        let t305: [uint32x4_t; 2] = [vcltq_u32(t303[0], t304[0]), vcltq_u32(t303[1], t304[1])];
        let t306: [uint32x4_t; 2] = [vandq_u32(live[0], t305[0]), vandq_u32(live[1], t305[1])];
        if any(&t306) {
            let t307 = b8_r0r1g_p(l, lid0, t306, arg0, arg1);
            x10 = t307;
            let t308: u64 = (1u64).wrapping_shl((32u64) as u32);
            let t309 = [vdupq_n_u32((t308) as u32); 2];
            let t310 = [vdupq_n_u32(((t308) >> 32) as u32); 2];
            let t311: [uint32x4_t; 4] = [vandq_u32(x10[0], t309[0]), vandq_u32(x10[1], t309[1]), vandq_u32(x10[2], t310[0]), vandq_u32(x10[3], t310[1])];
            let t312: u64 = 0u64;
            let t313 = [vdupq_n_u32((t312) as u32); 2];
            let t314: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t311[0], t313[0]), vceqq_u32(t311[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t311[1], t313[1]), vceqq_u32(t311[3], vdupq_n_u32(0)))];
            let t315: [uint32x4_t; 2] = [vmvnq_u32(t314[0]), vmvnq_u32(t314[1])];
            let mut t316: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t317: [uint32x4_t; 2] = [vandq_u32(t306[0], t315[0]), vandq_u32(t306[1], t315[1])];
            let t318: [uint32x4_t; 2] = [vbicq_u32(t306[0], t315[0]), vbicq_u32(t306[1], t315[1])];
            {
                let t319: [uint32x4_t; 2] = [x10[0], x10[1]];
                let t320 = [vdupq_n_u32(2147483648u32); 2];
                let t321: [uint32x4_t; 2] = [vorrq_u32(t320[0], t319[0]), vorrq_u32(t320[1], t319[1])];
                t316 = [vbslq_u32(t317[0], t321[0], t316[0]), vbslq_u32(t317[1], t321[1], t316[1])];
            }
            {
                let t322: [uint32x4_t; 2] = [x10[0], x10[1]];
                t316 = [vbslq_u32(t318[0], t322[0], t316[0]), vbslq_u32(t318[1], t322[1], t316[1])];
            }
            x9 = [vbslq_u32(t306[0], t316[0], x9[0]), vbslq_u32(t306[1], t316[1], x9[1])];
            b6_ag4r2l_p(l, lid0, t306, arg2, x9);
        }
        b9_r3gl_p(l, lid0, live, arg3, x7);
        let t323 = [vdupq_n_u32(x4); 2];
        let t324: [uint32x4_t; 2] = [vcgeq_u32(t303[0], t323[0]), vcgeq_u32(t303[1], t323[1])];
        let t325: [uint32x4_t; 2] = [vandq_u32(live[0], t324[0]), vandq_u32(live[1], t324[1])];
        {
            live = [vbicq_u32(live[0], t325[0]), vbicq_u32(live[1], t325[1])];
        }
        if any(&live) {
            x11 = 0u32;
            let mut t335 = live;
            loop {
                // Lanes part: each its own rounds.
                let t338 = [vdupq_n_u32(2147483648u32); 2];
                let t339: [uint32x4_t; 2] = [vandq_u32(x9[0], t338[0]), vandq_u32(x9[1], t338[1])];
                let t340 = [vdupq_n_u32(0u32); 2];
                let t341: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t339[0], t340[0])), vmvnq_u32(vceqq_u32(t339[1], t340[1]))];
                let t342 = [vdupq_n_u32(4294967294u32); 2];
                let t343: [uint32x4_t; 2] = [vcltq_u32(x9[0], t342[0]), vcltq_u32(x9[1], t342[1])];
                let t344: [uint32x4_t; 2] = [vandq_u32(t341[0], t343[0]), vandq_u32(t341[1], t343[1])];
                let t345: [uint32x4_t; 2] = [vandq_u32(vdupq_n_u32(if ((x11) < (7u32)) { u32::MAX } else { 0 }), t344[0]), vandq_u32(vdupq_n_u32(if ((x11) < (7u32)) { u32::MAX } else { 0 }), t344[1])];
                t335 = [vandq_u32(t335[0], t345[0]), vandq_u32(t335[1], t345[1])];
                if !(any(&t335)) { break; }
                let t347 = [vdupq_n_u32(2147483647u32); 2];
                let t348: [uint32x4_t; 2] = [vandq_u32(x9[0], t347[0]), vandq_u32(x9[1], t347[1])];
                x12 = t348;
                x13 = t342;
                let t349 = b10_r3ll_p(l, lid0, t335, arg3, x12, x6);
                let t350: [uint32x4_t; 2] = [vandq_u32(t335[0], t349[0]), vandq_u32(t335[1], t349[1])];
                if any(&t350) {
                    let t351 = b11_al4r2_p(l, lid0, t350, arg2, x12);
                    x13 = [vbslq_u32(t350[0], t351[0], x13[0]), vbslq_u32(t350[1], t351[1], x13[1])];
                }
                let t352: [uint32x4_t; 2] = [vceqq_u32(x13[0], t342[0]), vceqq_u32(x13[1], t342[1])];
                let t353: [uint32x4_t; 2] = [vandq_u32(t335[0], t352[0]), vandq_u32(t335[1], t352[1])];
                if any(&t353) {
                    let t354 = b8_r0r1l_p(l, lid0, t353, arg0, arg1, x12);
                    x14 = t354;
                    let t355: u64 = (1u64).wrapping_shl((32u64) as u32);
                    let t356 = [vdupq_n_u32((t355) as u32); 2];
                    let t357 = [vdupq_n_u32(((t355) >> 32) as u32); 2];
                    let t358: [uint32x4_t; 4] = [vandq_u32(x14[0], t356[0]), vandq_u32(x14[1], t356[1]), vandq_u32(x14[2], t357[0]), vandq_u32(x14[3], t357[1])];
                    let t359: u64 = 0u64;
                    let t360 = [vdupq_n_u32((t359) as u32); 2];
                    let t361: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t358[0], t360[0]), vceqq_u32(t358[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t358[1], t360[1]), vceqq_u32(t358[3], vdupq_n_u32(0)))];
                    let t362: [uint32x4_t; 2] = [vmvnq_u32(t361[0]), vmvnq_u32(t361[1])];
                    let mut t363: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                    let t364: [uint32x4_t; 2] = [vandq_u32(t353[0], t362[0]), vandq_u32(t353[1], t362[1])];
                    let t365: [uint32x4_t; 2] = [vbicq_u32(t353[0], t362[0]), vbicq_u32(t353[1], t362[1])];
                    {
                        let t366: [uint32x4_t; 2] = [x14[0], x14[1]];
                        let t367: [uint32x4_t; 2] = [vorrq_u32(t338[0], t366[0]), vorrq_u32(t338[1], t366[1])];
                        t363 = [vbslq_u32(t364[0], t367[0], t363[0]), vbslq_u32(t364[1], t367[1], t363[1])];
                    }
                    {
                        let t368: [uint32x4_t; 2] = [x14[0], x14[1]];
                        t363 = [vbslq_u32(t365[0], t368[0], t363[0]), vbslq_u32(t365[1], t368[1], t363[1])];
                    }
                    x13 = [vbslq_u32(t353[0], t363[0], x13[0]), vbslq_u32(t353[1], t363[1], x13[1])];
                }
                x9 = [vbslq_u32(t335[0], x13[0], x9[0]), vbslq_u32(t335[1], x13[1], x9[1])];
                let t369: [uint32x4_t; 2] = [vandq_u32(t335[0], live[0]), vandq_u32(t335[1], live[1])];
                let t370 = x11;
                x11 = (x11).wrapping_add(1u32);
            }
            let t371 = [vdupq_n_u32(2147483648u32); 2];
            let t372: [uint32x4_t; 2] = [vandq_u32(x9[0], t371[0]), vandq_u32(x9[1], t371[1])];
            let t373 = [vdupq_n_u32(0u32); 2];
            let t374: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t372[0], t373[0])), vmvnq_u32(vceqq_u32(t372[1], t373[1]))];
            let t375 = [vdupq_n_u32(4294967294u32); 2];
            let t376: [uint32x4_t; 2] = [vcltq_u32(x9[0], t375[0]), vcltq_u32(x9[1], t375[1])];
            let t377: [uint32x4_t; 2] = [vandq_u32(t374[0], t376[0]), vandq_u32(t374[1], t376[1])];
            let t378: [uint32x4_t; 2] = [vandq_u32(live[0], t377[0]), vandq_u32(live[1], t377[1])];
            if any(&t378) {
                let t379 = [vdupq_n_u32(2147483647u32); 2];
                let t380: [uint32x4_t; 2] = [vandq_u32(x9[0], t379[0]), vandq_u32(x9[1], t379[1])];
                let t381 = b7_r0r1l_p(l, lid0, t378, arg0, arg1, t380);
                x9 = [vbslq_u32(t378[0], t381[0], x9[0]), vbslq_u32(t378[1], t381[1], x9[1])];
            }
            b6_ag4r2l_p(l, lid0, live, arg2, x9);
        }
    }
}

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
            let t382 = b7_r0r1g_f(l, lid0, arg0, arg1);
            b6_ag4r2l_f(l, lid0, arg2, t382);
        } else {
            // Lanes part: the chunk's lanes on both sides of the bound.
            let t383: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
            let t384 = [vdupq_n_u32(x4); 2];
            let t385: [uint32x4_t; 2] = [vcltq_u32(t383[0], t384[0]), vcltq_u32(t383[1], t384[1])];
            let t386: [uint32x4_t; 2] = [vandq_u32(live[0], t385[0]), vandq_u32(live[1], t385[1])];
            if any(&t386) {
                let t387 = b7_r0r1g_p(l, lid0, t386, arg0, arg1);
                b6_ag4r2l_p(l, lid0, t386, arg2, t387);
            }
        }
        return;
    }
    if any(&live) {
        let t388 = [vdupq_n_u32(4294967295u32); 2];
        x9 = t388;
        let t389: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t390 = [vdupq_n_u32(x4); 2];
        let t391: [uint32x4_t; 2] = [vcltq_u32(t389[0], t390[0]), vcltq_u32(t389[1], t390[1])];
        let t392: [uint32x4_t; 2] = [vandq_u32(live[0], t391[0]), vandq_u32(live[1], t391[1])];
        if any(&t392) {
            let t393 = b8_r0r1g_p(l, lid0, t392, arg0, arg1);
            x10 = t393;
            let t394: u64 = (1u64).wrapping_shl((32u64) as u32);
            let t395 = [vdupq_n_u32((t394) as u32); 2];
            let t396 = [vdupq_n_u32(((t394) >> 32) as u32); 2];
            let t397: [uint32x4_t; 4] = [vandq_u32(x10[0], t395[0]), vandq_u32(x10[1], t395[1]), vandq_u32(x10[2], t396[0]), vandq_u32(x10[3], t396[1])];
            let t398: u64 = 0u64;
            let t399 = [vdupq_n_u32((t398) as u32); 2];
            let t400: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t397[0], t399[0]), vceqq_u32(t397[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t397[1], t399[1]), vceqq_u32(t397[3], vdupq_n_u32(0)))];
            let t401: [uint32x4_t; 2] = [vmvnq_u32(t400[0]), vmvnq_u32(t400[1])];
            let mut t402: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t403: [uint32x4_t; 2] = [vandq_u32(t392[0], t401[0]), vandq_u32(t392[1], t401[1])];
            let t404: [uint32x4_t; 2] = [vbicq_u32(t392[0], t401[0]), vbicq_u32(t392[1], t401[1])];
            {
                let t405: [uint32x4_t; 2] = [x10[0], x10[1]];
                let t406 = [vdupq_n_u32(2147483648u32); 2];
                let t407: [uint32x4_t; 2] = [vorrq_u32(t406[0], t405[0]), vorrq_u32(t406[1], t405[1])];
                t402 = [vbslq_u32(t403[0], t407[0], t402[0]), vbslq_u32(t403[1], t407[1], t402[1])];
            }
            {
                let t408: [uint32x4_t; 2] = [x10[0], x10[1]];
                t402 = [vbslq_u32(t404[0], t408[0], t402[0]), vbslq_u32(t404[1], t408[1], t402[1])];
            }
            x9 = [vbslq_u32(t392[0], t402[0], x9[0]), vbslq_u32(t392[1], t402[1], x9[1])];
            b6_ag4r2l_p(l, lid0, t392, arg2, x9);
        }
        b9_r3gl_p(l, lid0, live, arg3, x7);
        let t409 = [vdupq_n_u32(x4); 2];
        let t410: [uint32x4_t; 2] = [vcgeq_u32(t389[0], t409[0]), vcgeq_u32(t389[1], t409[1])];
        let t411: [uint32x4_t; 2] = [vandq_u32(live[0], t410[0]), vandq_u32(live[1], t410[1])];
        {
            live = [vbicq_u32(live[0], t411[0]), vbicq_u32(live[1], t411[1])];
        }
        if any(&live) {
            x11 = 0u32;
            let mut t421 = live;
            loop {
                // Lanes part: each its own rounds.
                let t424 = [vdupq_n_u32(2147483648u32); 2];
                let t425: [uint32x4_t; 2] = [vandq_u32(x9[0], t424[0]), vandq_u32(x9[1], t424[1])];
                let t426 = [vdupq_n_u32(0u32); 2];
                let t427: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t425[0], t426[0])), vmvnq_u32(vceqq_u32(t425[1], t426[1]))];
                let t428 = [vdupq_n_u32(4294967294u32); 2];
                let t429: [uint32x4_t; 2] = [vcltq_u32(x9[0], t428[0]), vcltq_u32(x9[1], t428[1])];
                let t430: [uint32x4_t; 2] = [vandq_u32(t427[0], t429[0]), vandq_u32(t427[1], t429[1])];
                let t431: [uint32x4_t; 2] = [vandq_u32(vdupq_n_u32(if ((x11) < (7u32)) { u32::MAX } else { 0 }), t430[0]), vandq_u32(vdupq_n_u32(if ((x11) < (7u32)) { u32::MAX } else { 0 }), t430[1])];
                t421 = [vandq_u32(t421[0], t431[0]), vandq_u32(t421[1], t431[1])];
                if !(any(&t421)) { break; }
                let t433 = [vdupq_n_u32(2147483647u32); 2];
                let t434: [uint32x4_t; 2] = [vandq_u32(x9[0], t433[0]), vandq_u32(x9[1], t433[1])];
                x12 = t434;
                x13 = t428;
                let t435 = b10_r3ll_p(l, lid0, t421, arg3, x12, x6);
                let t436: [uint32x4_t; 2] = [vandq_u32(t421[0], t435[0]), vandq_u32(t421[1], t435[1])];
                if any(&t436) {
                    let t437 = b11_al4r2_p(l, lid0, t436, arg2, x12);
                    x13 = [vbslq_u32(t436[0], t437[0], x13[0]), vbslq_u32(t436[1], t437[1], x13[1])];
                }
                let t438: [uint32x4_t; 2] = [vceqq_u32(x13[0], t428[0]), vceqq_u32(x13[1], t428[1])];
                let t439: [uint32x4_t; 2] = [vandq_u32(t421[0], t438[0]), vandq_u32(t421[1], t438[1])];
                if any(&t439) {
                    let t440 = b8_r0r1l_p(l, lid0, t439, arg0, arg1, x12);
                    x14 = t440;
                    let t441: u64 = (1u64).wrapping_shl((32u64) as u32);
                    let t442 = [vdupq_n_u32((t441) as u32); 2];
                    let t443 = [vdupq_n_u32(((t441) >> 32) as u32); 2];
                    let t444: [uint32x4_t; 4] = [vandq_u32(x14[0], t442[0]), vandq_u32(x14[1], t442[1]), vandq_u32(x14[2], t443[0]), vandq_u32(x14[3], t443[1])];
                    let t445: u64 = 0u64;
                    let t446 = [vdupq_n_u32((t445) as u32); 2];
                    let t447: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t444[0], t446[0]), vceqq_u32(t444[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t444[1], t446[1]), vceqq_u32(t444[3], vdupq_n_u32(0)))];
                    let t448: [uint32x4_t; 2] = [vmvnq_u32(t447[0]), vmvnq_u32(t447[1])];
                    let mut t449: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                    let t450: [uint32x4_t; 2] = [vandq_u32(t439[0], t448[0]), vandq_u32(t439[1], t448[1])];
                    let t451: [uint32x4_t; 2] = [vbicq_u32(t439[0], t448[0]), vbicq_u32(t439[1], t448[1])];
                    {
                        let t452: [uint32x4_t; 2] = [x14[0], x14[1]];
                        let t453: [uint32x4_t; 2] = [vorrq_u32(t424[0], t452[0]), vorrq_u32(t424[1], t452[1])];
                        t449 = [vbslq_u32(t450[0], t453[0], t449[0]), vbslq_u32(t450[1], t453[1], t449[1])];
                    }
                    {
                        let t454: [uint32x4_t; 2] = [x14[0], x14[1]];
                        t449 = [vbslq_u32(t451[0], t454[0], t449[0]), vbslq_u32(t451[1], t454[1], t449[1])];
                    }
                    x13 = [vbslq_u32(t439[0], t449[0], x13[0]), vbslq_u32(t439[1], t449[1], x13[1])];
                }
                x9 = [vbslq_u32(t421[0], x13[0], x9[0]), vbslq_u32(t421[1], x13[1], x9[1])];
                let t455: [uint32x4_t; 2] = [vandq_u32(t421[0], live[0]), vandq_u32(t421[1], live[1])];
                let t456 = x11;
                x11 = (x11).wrapping_add(1u32);
            }
            let t457 = [vdupq_n_u32(2147483648u32); 2];
            let t458: [uint32x4_t; 2] = [vandq_u32(x9[0], t457[0]), vandq_u32(x9[1], t457[1])];
            let t459 = [vdupq_n_u32(0u32); 2];
            let t460: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t458[0], t459[0])), vmvnq_u32(vceqq_u32(t458[1], t459[1]))];
            let t461 = [vdupq_n_u32(4294967294u32); 2];
            let t462: [uint32x4_t; 2] = [vcltq_u32(x9[0], t461[0]), vcltq_u32(x9[1], t461[1])];
            let t463: [uint32x4_t; 2] = [vandq_u32(t460[0], t462[0]), vandq_u32(t460[1], t462[1])];
            let t464: [uint32x4_t; 2] = [vandq_u32(live[0], t463[0]), vandq_u32(live[1], t463[1])];
            if any(&t464) {
                let t465 = [vdupq_n_u32(2147483647u32); 2];
                let t466: [uint32x4_t; 2] = [vandq_u32(x9[0], t465[0]), vandq_u32(x9[1], t465[1])];
                let t467 = b7_r0r1l_p(l, lid0, t464, arg0, arg1, t466);
                x9 = [vbslq_u32(t464[0], t467[0], x9[0]), vbslq_u32(t464[1], t467[1], x9[1])];
            }
            b6_ag4r2l_p(l, lid0, live, arg2, x9);
        }
    }
}

#[inline(always)]
unsafe fn b4_o4_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg4_x0: *const u8) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    res = (arg4_x0 as *const u32).read_unaligned();
    return res;
}

#[inline(always)]
unsafe fn b5__p(l: &Launch, lid0: u32, k: [uint32x4_t; 2]) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
}

#[inline(always)]
unsafe fn b6_ag4r2l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg2: *const u8, x1: [uint32x4_t; 2]) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let t468 = (arg2 as *mut u32).add((gid0) as usize);
    if all(&live) {
        vst1q_u32(t468.add(0), x1[0]);
        vst1q_u32(t468.add(4), x1[1]);
    } else {
        let mut t469w = 0u32;
        let t469 = core::hint::black_box(&raw mut t469w);
        core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t468.add(0), t469).write(vgetq_lane_u32::<0>(x1[0]));
        core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t468.add(1), t469).write(vgetq_lane_u32::<1>(x1[0]));
        core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t468.add(2), t469).write(vgetq_lane_u32::<2>(x1[0]));
        core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t468.add(3), t469).write(vgetq_lane_u32::<3>(x1[0]));
        core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t468.add(4), t469).write(vgetq_lane_u32::<0>(x1[1]));
        core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t468.add(5), t469).write(vgetq_lane_u32::<1>(x1[1]));
        core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t468.add(6), t469).write(vgetq_lane_u32::<2>(x1[1]));
        core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t468.add(7), t469).write(vgetq_lane_u32::<3>(x1[1]));
    }
}

#[inline(always)]
unsafe fn b6_ag4r2l_f(l: &Launch, lid0: u32, arg2: *const u8, x1: [uint32x4_t; 2]) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let t470 = (arg2 as *mut u32).add((gid0) as usize);
    vst1q_u32(t470.add(0), x1[0]);
    vst1q_u32(t470.add(4), x1[1]);
}

#[inline(always)]
unsafe fn b7_r0r1g_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x3: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t471 = b8_r0r1g_p(l, lid0, live, arg0, arg1);
    x3 = t471;
    let mut t480 = live;
    loop {
        // Lanes part: each its own rounds.
        let t482: u64 = (1u64).wrapping_shl((32u64) as u32);
        let t483 = [vdupq_n_u32((t482) as u32); 2];
        let t484 = [vdupq_n_u32(((t482) >> 32) as u32); 2];
        let t485: [uint32x4_t; 4] = [vandq_u32(x3[0], t483[0]), vandq_u32(x3[1], t483[1]), vandq_u32(x3[2], t484[0]), vandq_u32(x3[3], t484[1])];
        let t486: u64 = 0u64;
        let t487 = [vdupq_n_u32((t486) as u32); 2];
        let t488: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t485[0], t487[0]), vceqq_u32(t485[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t485[1], t487[1]), vceqq_u32(t485[3], vdupq_n_u32(0)))];
        let t489: [uint32x4_t; 2] = [vmvnq_u32(t488[0]), vmvnq_u32(t488[1])];
        t480 = [vandq_u32(t480[0], t489[0]), vandq_u32(t480[1], t489[1])];
        if !(any(&t480)) { break; }
        let t491: [uint32x4_t; 2] = [x3[0], x3[1]];
        let t492 = b8_r0r1l_p(l, lid0, t480, arg0, arg1, t491);
        x3 = [vbslq_u32(t480[0], t492[0], x3[0]), vbslq_u32(t480[1], t492[1], x3[1]), vbslq_u32(t480[0], t492[2], x3[2]), vbslq_u32(t480[1], t492[3], x3[3])];
    }
    let t493: [uint32x4_t; 2] = [x3[0], x3[1]];
    res = t493;
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1g_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x3: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t494 = b8_r0r1g_f(l, lid0, arg0, arg1);
    x3 = t494;
    let mut t511 = live;
    loop {
        // Lanes part: each its own rounds.
        let t513: u64 = (1u64).wrapping_shl((32u64) as u32);
        let t514 = [vdupq_n_u32((t513) as u32); 2];
        let t515 = [vdupq_n_u32(((t513) >> 32) as u32); 2];
        let t516: [uint32x4_t; 4] = [vandq_u32(x3[0], t514[0]), vandq_u32(x3[1], t514[1]), vandq_u32(x3[2], t515[0]), vandq_u32(x3[3], t515[1])];
        let t517: u64 = 0u64;
        let t518 = [vdupq_n_u32((t517) as u32); 2];
        let t519: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t516[0], t518[0]), vceqq_u32(t516[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t516[1], t518[1]), vceqq_u32(t516[3], vdupq_n_u32(0)))];
        let t520: [uint32x4_t; 2] = [vmvnq_u32(t519[0]), vmvnq_u32(t519[1])];
        t511 = [vandq_u32(t511[0], t520[0]), vandq_u32(t511[1], t520[1])];
        if !(any(&t511)) { break; }
        let t522: [uint32x4_t; 2] = [x3[0], x3[1]];
        let t523 = b8_r0r1l_p(l, lid0, t511, arg0, arg1, t522);
        x3 = [vbslq_u32(t511[0], t523[0], x3[0]), vbslq_u32(t511[1], t523[1], x3[1]), vbslq_u32(t511[0], t523[2], x3[2]), vbslq_u32(t511[1], t523[3], x3[3])];
    }
    let t524: [uint32x4_t; 2] = [x3[0], x3[1]];
    res = t524;
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x3: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t525 = b8_r0r1l_p(l, lid0, live, arg0, arg1, x2);
    x3 = t525;
    let mut t534 = live;
    loop {
        // Lanes part: each its own rounds.
        let t536: u64 = (1u64).wrapping_shl((32u64) as u32);
        let t537 = [vdupq_n_u32((t536) as u32); 2];
        let t538 = [vdupq_n_u32(((t536) >> 32) as u32); 2];
        let t539: [uint32x4_t; 4] = [vandq_u32(x3[0], t537[0]), vandq_u32(x3[1], t537[1]), vandq_u32(x3[2], t538[0]), vandq_u32(x3[3], t538[1])];
        let t540: u64 = 0u64;
        let t541 = [vdupq_n_u32((t540) as u32); 2];
        let t542: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t539[0], t541[0]), vceqq_u32(t539[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t539[1], t541[1]), vceqq_u32(t539[3], vdupq_n_u32(0)))];
        let t543: [uint32x4_t; 2] = [vmvnq_u32(t542[0]), vmvnq_u32(t542[1])];
        t534 = [vandq_u32(t534[0], t543[0]), vandq_u32(t534[1], t543[1])];
        if !(any(&t534)) { break; }
        let t545: [uint32x4_t; 2] = [x3[0], x3[1]];
        let t546 = b8_r0r1l_p(l, lid0, t534, arg0, arg1, t545);
        x3 = [vbslq_u32(t534[0], t546[0], x3[0]), vbslq_u32(t534[1], t546[1], x3[1]), vbslq_u32(t534[0], t546[2], x3[2]), vbslq_u32(t534[1], t546[3], x3[3])];
    }
    let t547: [uint32x4_t; 2] = [x3[0], x3[1]];
    res = t547;
    return res;
}

#[inline(always)]
unsafe fn b8_r0r1g_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x3: u32 = 0;
    let mut x4: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x5: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x6: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t548: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
    let t549 = [vdupq_n_u32(0u32); 2];
    let t550: [uint32x4_t; 2] = [vceqq_u32(t548[0], t549[0]), vceqq_u32(t548[1], t549[1])];
    let t551: [uint32x4_t; 2] = [vandq_u32(live[0], t550[0]), vandq_u32(live[1], t550[1])];
    {
        let t552: u64 = 4294967295u64;
        let t553 = [vdupq_n_u32((t552) as u32); 2];
        let t554: [uint32x4_t; 4] = [t553[0], t553[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = [vbslq_u32(t551[0], t554[0], res[0]), vbslq_u32(t551[1], t554[1], res[1]), vbslq_u32(t551[0], t554[2], res[2]), vbslq_u32(t551[1], t554[3], res[3])];
        live = [vbicq_u32(live[0], t551[0]), vbicq_u32(live[1], t551[1])];
    }
    if any(&live) {
        x3 = gid0.wrapping_sub(1u32);
        let t555 = if (x3 & 255u32) <= 248u32 { b12_r0r1u8_p(l, lid0, live, arg0, arg1, x3) } else { b12_r0r1i_p(l, lid0, live, arg0, arg1, x3) };
        x4 = t555;
        let t556: u64 = 1u64;
        let t557 = [vdupq_n_u32((t556) as u32); 2];
        let t558: [uint32x4_t; 2] = [vandq_u32(x4[0], t557[0]), vandq_u32(x4[1], t557[1])];
        let t559: u64 = 0u64;
        let t560 = [vdupq_n_u32((t559) as u32); 2];
        let t561: [uint32x4_t; 2] = [vceqq_u32(t558[0], t560[0]), vceqq_u32(t558[1], t560[1])];
        let t562: [uint32x4_t; 2] = [vandq_u32(live[0], t561[0]), vandq_u32(live[1], t561[1])];
        {
            let t563: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[1].as_ptr()))];
            let t564: [uint32x4_t; 4] = [t563[0], t563[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t562[0], t564[0], res[0]), vbslq_u32(t562[1], t564[1], res[1]), vbslq_u32(t562[0], t564[2], res[2]), vbslq_u32(t562[1], t564[3], res[3])];
            live = [vbicq_u32(live[0], t562[0]), vbicq_u32(live[1], t562[1])];
        }
        if any(&live) {
            let t565: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x4[0]), vshlq_n_u32::<30>(x4[2])), vorrq_u32(vshrq_n_u32::<2>(x4[1]), vshlq_n_u32::<30>(x4[3])), vshrq_n_u32::<2>(x4[2]), vshrq_n_u32::<2>(x4[3])];
            let t566: [uint32x4_t; 2] = [t565[0], t565[1]];
            x5 = t566;
            let t567 = [vdupq_n_u32(1u32); 2];
            let t568: [uint32x4_t; 2] = [vsubq_u32(x5[0], t567[0]), vsubq_u32(x5[1], t567[1])];
            let t569: [uint32x4_t; 2] = [vcgeq_u32(t568[0], t548[0]), vcgeq_u32(t568[1], t548[1])];
            let t570: [uint32x4_t; 2] = [vandq_u32(live[0], t569[0]), vandq_u32(live[1], t569[1])];
            {
                let t571: u64 = 4294967295u64;
                let t572 = [vdupq_n_u32((t571) as u32); 2];
                let t573: [uint32x4_t; 4] = [t572[0], t572[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t570[0], t573[0], res[0]), vbslq_u32(t570[1], t573[1], res[1]), vbslq_u32(t570[0], t573[2], res[2]), vbslq_u32(t570[1], t573[3], res[3])];
                live = [vbicq_u32(live[0], t570[0]), vbicq_u32(live[1], t570[1])];
            }
            if any(&live) {
                let t574: [uint32x4_t; 2] = [vsubq_u32(t548[0], x5[0]), vsubq_u32(t548[1], x5[1])];
                x6 = t574;
                let t575: u64 = 2u64;
                let t576 = [vdupq_n_u32((t575) as u32); 2];
                let t577: [uint32x4_t; 2] = [vandq_u32(x4[0], t576[0]), vandq_u32(x4[1], t576[1])];
                let t578: u64 = 0u64;
                let t579 = [vdupq_n_u32((t578) as u32); 2];
                let t580: [uint32x4_t; 2] = [vceqq_u32(t577[0], t579[0]), vceqq_u32(t577[1], t579[1])];
                let t581: [uint32x4_t; 2] = [vmvnq_u32(t580[0]), vmvnq_u32(t580[1])];
                let t582: [uint32x4_t; 2] = [vandq_u32(live[0], t581[0]), vandq_u32(live[1], t581[1])];
                {
                    let t583: [uint32x4_t; 4] = [x6[0], x6[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                    res = [vbslq_u32(t582[0], t583[0], res[0]), vbslq_u32(t582[1], t583[1], res[1]), vbslq_u32(t582[0], t583[2], res[2]), vbslq_u32(t582[1], t583[3], res[3])];
                    live = [vbicq_u32(live[0], t582[0]), vbicq_u32(live[1], t582[1])];
                }
                if any(&live) {
                    let t584: [uint32x4_t; 2] = [vceqq_u32(x6[0], t549[0]), vceqq_u32(x6[1], t549[1])];
                    let t585: [uint32x4_t; 2] = [vandq_u32(live[0], t584[0]), vandq_u32(live[1], t584[1])];
                    {
                        let t586: u64 = 4294967295u64;
                        let t587 = [vdupq_n_u32((t586) as u32); 2];
                        let t588: [uint32x4_t; 4] = [t587[0], t587[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                        res = [vbslq_u32(t585[0], t588[0], res[0]), vbslq_u32(t585[1], t588[1], res[1]), vbslq_u32(t585[0], t588[2], res[2]), vbslq_u32(t585[1], t588[3], res[3])];
                        live = [vbicq_u32(live[0], t585[0]), vbicq_u32(live[1], t585[1])];
                    }
                    if any(&live) {
                        let t589: u64 = (1u64).wrapping_shl((32u64) as u32);
                        let t590 = [vdupq_n_u32((t589) as u32); 2];
                        let t591 = [vdupq_n_u32(((t589) >> 32) as u32); 2];
                        let t592: [uint32x4_t; 4] = [vorrq_u32(t590[0], x6[0]), vorrq_u32(t590[1], x6[1]), t591[0], t591[1]];
                        res = [vbslq_u32(live[0], t592[0], res[0]), vbslq_u32(live[1], t592[1], res[1]), vbslq_u32(live[0], t592[2], res[2]), vbslq_u32(live[1], t592[3], res[3])];
                        live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
                    }
                }
            }
        }
    }
    res
}

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
        let t593 = if (x3 & 255u32) <= 248u32 { b12_r0r1u8_f(l, lid0, arg0, arg1, x3) } else { b12_r0r1i_f(l, lid0, arg0, arg1, x3) };
        x4 = t593;
        let t594: u64 = 1u64;
        let t595 = [vdupq_n_u32((t594) as u32); 2];
        let t596: [uint32x4_t; 2] = [vandq_u32(x4[0], t595[0]), vandq_u32(x4[1], t595[1])];
        let t597: u64 = 0u64;
        let t598 = [vdupq_n_u32((t597) as u32); 2];
        let t599: [uint32x4_t; 2] = [vceqq_u32(t596[0], t598[0]), vceqq_u32(t596[1], t598[1])];
        {
            let t600: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[1].as_ptr()))];
            let t601: [uint32x4_t; 4] = [t600[0], t600[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t599[0], t601[0], res[0]), vbslq_u32(t599[1], t601[1], res[1]), vbslq_u32(t599[0], t601[2], res[2]), vbslq_u32(t599[1], t601[3], res[3])];
            live = [vbicq_u32(live[0], t599[0]), vbicq_u32(live[1], t599[1])];
        }
        if any(&live) {
            let t602: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x4[0]), vshlq_n_u32::<30>(x4[2])), vorrq_u32(vshrq_n_u32::<2>(x4[1]), vshlq_n_u32::<30>(x4[3])), vshrq_n_u32::<2>(x4[2]), vshrq_n_u32::<2>(x4[3])];
            let t603: [uint32x4_t; 2] = [t602[0], t602[1]];
            x5 = t603;
            let t604 = [vdupq_n_u32(1u32); 2];
            let t605: [uint32x4_t; 2] = [vsubq_u32(x5[0], t604[0]), vsubq_u32(x5[1], t604[1])];
            let t606: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
            let t607: [uint32x4_t; 2] = [vcgeq_u32(t605[0], t606[0]), vcgeq_u32(t605[1], t606[1])];
            let t608: [uint32x4_t; 2] = [vandq_u32(live[0], t607[0]), vandq_u32(live[1], t607[1])];
            {
                let t609: u64 = 4294967295u64;
                let t610 = [vdupq_n_u32((t609) as u32); 2];
                let t611: [uint32x4_t; 4] = [t610[0], t610[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t608[0], t611[0], res[0]), vbslq_u32(t608[1], t611[1], res[1]), vbslq_u32(t608[0], t611[2], res[2]), vbslq_u32(t608[1], t611[3], res[3])];
                live = [vbicq_u32(live[0], t608[0]), vbicq_u32(live[1], t608[1])];
            }
            if any(&live) {
                let t612: [uint32x4_t; 2] = [vsubq_u32(t606[0], x5[0]), vsubq_u32(t606[1], x5[1])];
                x6 = t612;
                let t613: u64 = 2u64;
                let t614 = [vdupq_n_u32((t613) as u32); 2];
                let t615: [uint32x4_t; 2] = [vandq_u32(x4[0], t614[0]), vandq_u32(x4[1], t614[1])];
                let t616: u64 = 0u64;
                let t617 = [vdupq_n_u32((t616) as u32); 2];
                let t618: [uint32x4_t; 2] = [vceqq_u32(t615[0], t617[0]), vceqq_u32(t615[1], t617[1])];
                let t619: [uint32x4_t; 2] = [vmvnq_u32(t618[0]), vmvnq_u32(t618[1])];
                let t620: [uint32x4_t; 2] = [vandq_u32(live[0], t619[0]), vandq_u32(live[1], t619[1])];
                {
                    let t621: [uint32x4_t; 4] = [x6[0], x6[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                    res = [vbslq_u32(t620[0], t621[0], res[0]), vbslq_u32(t620[1], t621[1], res[1]), vbslq_u32(t620[0], t621[2], res[2]), vbslq_u32(t620[1], t621[3], res[3])];
                    live = [vbicq_u32(live[0], t620[0]), vbicq_u32(live[1], t620[1])];
                }
                if any(&live) {
                    let t622 = [vdupq_n_u32(0u32); 2];
                    let t623: [uint32x4_t; 2] = [vceqq_u32(x6[0], t622[0]), vceqq_u32(x6[1], t622[1])];
                    let t624: [uint32x4_t; 2] = [vandq_u32(live[0], t623[0]), vandq_u32(live[1], t623[1])];
                    {
                        let t625: u64 = 4294967295u64;
                        let t626 = [vdupq_n_u32((t625) as u32); 2];
                        let t627: [uint32x4_t; 4] = [t626[0], t626[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                        res = [vbslq_u32(t624[0], t627[0], res[0]), vbslq_u32(t624[1], t627[1], res[1]), vbslq_u32(t624[0], t627[2], res[2]), vbslq_u32(t624[1], t627[3], res[3])];
                        live = [vbicq_u32(live[0], t624[0]), vbicq_u32(live[1], t624[1])];
                    }
                    if any(&live) {
                        let t628: u64 = (1u64).wrapping_shl((32u64) as u32);
                        let t629 = [vdupq_n_u32((t628) as u32); 2];
                        let t630 = [vdupq_n_u32(((t628) >> 32) as u32); 2];
                        let t631: [uint32x4_t; 4] = [vorrq_u32(t629[0], x6[0]), vorrq_u32(t629[1], x6[1]), t630[0], t630[1]];
                        res = [vbslq_u32(live[0], t631[0], res[0]), vbslq_u32(live[1], t631[1], res[1]), vbslq_u32(live[0], t631[2], res[2]), vbslq_u32(live[1], t631[3], res[3])];
                        live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
                    }
                }
            }
        }
    } else if false {
        let t632: u64 = 4294967295u64;
        let t633 = [vdupq_n_u32((t632) as u32); 2];
        let t634: [uint32x4_t; 4] = [t633[0], t633[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t634;
        return res;
    } else {
        // Lanes part: the chunk's lanes on both sides of the bound.
        let t635: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t636 = [vdupq_n_u32(0u32); 2];
        let t637: [uint32x4_t; 2] = [vceqq_u32(t635[0], t636[0]), vceqq_u32(t635[1], t636[1])];
        let t638: [uint32x4_t; 2] = [vandq_u32(live[0], t637[0]), vandq_u32(live[1], t637[1])];
        {
            let t639: u64 = 4294967295u64;
            let t640 = [vdupq_n_u32((t639) as u32); 2];
            let t641: [uint32x4_t; 4] = [t640[0], t640[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t638[0], t641[0], res[0]), vbslq_u32(t638[1], t641[1], res[1]), vbslq_u32(t638[0], t641[2], res[2]), vbslq_u32(t638[1], t641[3], res[3])];
            live = [vbicq_u32(live[0], t638[0]), vbicq_u32(live[1], t638[1])];
        }
        if any(&live) {
            x3 = gid0.wrapping_sub(1u32);
            let t642 = if (x3 & 255u32) <= 248u32 { b12_r0r1u8_p(l, lid0, live, arg0, arg1, x3) } else { b12_r0r1i_p(l, lid0, live, arg0, arg1, x3) };
            x4 = t642;
            let t643: u64 = 1u64;
            let t644 = [vdupq_n_u32((t643) as u32); 2];
            let t645: [uint32x4_t; 2] = [vandq_u32(x4[0], t644[0]), vandq_u32(x4[1], t644[1])];
            let t646: u64 = 0u64;
            let t647 = [vdupq_n_u32((t646) as u32); 2];
            let t648: [uint32x4_t; 2] = [vceqq_u32(t645[0], t647[0]), vceqq_u32(t645[1], t647[1])];
            let t649: [uint32x4_t; 2] = [vandq_u32(live[0], t648[0]), vandq_u32(live[1], t648[1])];
            {
                let t650: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[1].as_ptr()))];
                let t651: [uint32x4_t; 4] = [t650[0], t650[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t649[0], t651[0], res[0]), vbslq_u32(t649[1], t651[1], res[1]), vbslq_u32(t649[0], t651[2], res[2]), vbslq_u32(t649[1], t651[3], res[3])];
                live = [vbicq_u32(live[0], t649[0]), vbicq_u32(live[1], t649[1])];
            }
            if any(&live) {
                let t652: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x4[0]), vshlq_n_u32::<30>(x4[2])), vorrq_u32(vshrq_n_u32::<2>(x4[1]), vshlq_n_u32::<30>(x4[3])), vshrq_n_u32::<2>(x4[2]), vshrq_n_u32::<2>(x4[3])];
                let t653: [uint32x4_t; 2] = [t652[0], t652[1]];
                x5 = t653;
                let t654 = [vdupq_n_u32(1u32); 2];
                let t655: [uint32x4_t; 2] = [vsubq_u32(x5[0], t654[0]), vsubq_u32(x5[1], t654[1])];
                let t656: [uint32x4_t; 2] = [vcgeq_u32(t655[0], t635[0]), vcgeq_u32(t655[1], t635[1])];
                let t657: [uint32x4_t; 2] = [vandq_u32(live[0], t656[0]), vandq_u32(live[1], t656[1])];
                {
                    let t658: u64 = 4294967295u64;
                    let t659 = [vdupq_n_u32((t658) as u32); 2];
                    let t660: [uint32x4_t; 4] = [t659[0], t659[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                    res = [vbslq_u32(t657[0], t660[0], res[0]), vbslq_u32(t657[1], t660[1], res[1]), vbslq_u32(t657[0], t660[2], res[2]), vbslq_u32(t657[1], t660[3], res[3])];
                    live = [vbicq_u32(live[0], t657[0]), vbicq_u32(live[1], t657[1])];
                }
                if any(&live) {
                    let t661: [uint32x4_t; 2] = [vsubq_u32(t635[0], x5[0]), vsubq_u32(t635[1], x5[1])];
                    x6 = t661;
                    let t662: u64 = 2u64;
                    let t663 = [vdupq_n_u32((t662) as u32); 2];
                    let t664: [uint32x4_t; 2] = [vandq_u32(x4[0], t663[0]), vandq_u32(x4[1], t663[1])];
                    let t665: u64 = 0u64;
                    let t666 = [vdupq_n_u32((t665) as u32); 2];
                    let t667: [uint32x4_t; 2] = [vceqq_u32(t664[0], t666[0]), vceqq_u32(t664[1], t666[1])];
                    let t668: [uint32x4_t; 2] = [vmvnq_u32(t667[0]), vmvnq_u32(t667[1])];
                    let t669: [uint32x4_t; 2] = [vandq_u32(live[0], t668[0]), vandq_u32(live[1], t668[1])];
                    {
                        let t670: [uint32x4_t; 4] = [x6[0], x6[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                        res = [vbslq_u32(t669[0], t670[0], res[0]), vbslq_u32(t669[1], t670[1], res[1]), vbslq_u32(t669[0], t670[2], res[2]), vbslq_u32(t669[1], t670[3], res[3])];
                        live = [vbicq_u32(live[0], t669[0]), vbicq_u32(live[1], t669[1])];
                    }
                    if any(&live) {
                        let t671: [uint32x4_t; 2] = [vceqq_u32(x6[0], t636[0]), vceqq_u32(x6[1], t636[1])];
                        let t672: [uint32x4_t; 2] = [vandq_u32(live[0], t671[0]), vandq_u32(live[1], t671[1])];
                        {
                            let t673: u64 = 4294967295u64;
                            let t674 = [vdupq_n_u32((t673) as u32); 2];
                            let t675: [uint32x4_t; 4] = [t674[0], t674[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                            res = [vbslq_u32(t672[0], t675[0], res[0]), vbslq_u32(t672[1], t675[1], res[1]), vbslq_u32(t672[0], t675[2], res[2]), vbslq_u32(t672[1], t675[3], res[3])];
                            live = [vbicq_u32(live[0], t672[0]), vbicq_u32(live[1], t672[1])];
                        }
                        if any(&live) {
                            let t676: u64 = (1u64).wrapping_shl((32u64) as u32);
                            let t677 = [vdupq_n_u32((t676) as u32); 2];
                            let t678 = [vdupq_n_u32(((t676) >> 32) as u32); 2];
                            let t679: [uint32x4_t; 4] = [vorrq_u32(t677[0], x6[0]), vorrq_u32(t677[1], x6[1]), t678[0], t678[1]];
                            res = [vbslq_u32(live[0], t679[0], res[0]), vbslq_u32(live[1], t679[1], res[1]), vbslq_u32(live[0], t679[2], res[2]), vbslq_u32(live[1], t679[3], res[3])];
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
unsafe fn b8_r0r1l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x3: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x4: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x5: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x6: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t680 = [vdupq_n_u32(0u32); 2];
    let t681: [uint32x4_t; 2] = [vceqq_u32(x2[0], t680[0]), vceqq_u32(x2[1], t680[1])];
    let t682: [uint32x4_t; 2] = [vandq_u32(live[0], t681[0]), vandq_u32(live[1], t681[1])];
    {
        let t683: u64 = 4294967295u64;
        let t684 = [vdupq_n_u32((t683) as u32); 2];
        let t685: [uint32x4_t; 4] = [t684[0], t684[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = [vbslq_u32(t682[0], t685[0], res[0]), vbslq_u32(t682[1], t685[1], res[1]), vbslq_u32(t682[0], t685[2], res[2]), vbslq_u32(t682[1], t685[3], res[3])];
        live = [vbicq_u32(live[0], t682[0]), vbicq_u32(live[1], t682[1])];
    }
    if any(&live) {
        let t686 = [vdupq_n_u32(1u32); 2];
        let t687: [uint32x4_t; 2] = [vsubq_u32(x2[0], t686[0]), vsubq_u32(x2[1], t686[1])];
        x3 = t687;
        let t688 = b12_r0r1l_p(l, lid0, live, arg0, arg1, x3);
        x4 = t688;
        let t689: u64 = 1u64;
        let t690 = [vdupq_n_u32((t689) as u32); 2];
        let t691: [uint32x4_t; 2] = [vandq_u32(x4[0], t690[0]), vandq_u32(x4[1], t690[1])];
        let t692: u64 = 0u64;
        let t693 = [vdupq_n_u32((t692) as u32); 2];
        let t694: [uint32x4_t; 2] = [vceqq_u32(t691[0], t693[0]), vceqq_u32(t691[1], t693[1])];
        let t695: [uint32x4_t; 2] = [vandq_u32(live[0], t694[0]), vandq_u32(live[1], t694[1])];
        {
            let t696: [uint32x4_t; 4] = [x3[0], x3[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t695[0], t696[0], res[0]), vbslq_u32(t695[1], t696[1], res[1]), vbslq_u32(t695[0], t696[2], res[2]), vbslq_u32(t695[1], t696[3], res[3])];
            live = [vbicq_u32(live[0], t695[0]), vbicq_u32(live[1], t695[1])];
        }
        if any(&live) {
            let t697: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x4[0]), vshlq_n_u32::<30>(x4[2])), vorrq_u32(vshrq_n_u32::<2>(x4[1]), vshlq_n_u32::<30>(x4[3])), vshrq_n_u32::<2>(x4[2]), vshrq_n_u32::<2>(x4[3])];
            let t698: [uint32x4_t; 2] = [t697[0], t697[1]];
            x5 = t698;
            let t699: [uint32x4_t; 2] = [vsubq_u32(x5[0], t686[0]), vsubq_u32(x5[1], t686[1])];
            let t700: [uint32x4_t; 2] = [vcgeq_u32(t699[0], x2[0]), vcgeq_u32(t699[1], x2[1])];
            let t701: [uint32x4_t; 2] = [vandq_u32(live[0], t700[0]), vandq_u32(live[1], t700[1])];
            {
                let t702: u64 = 4294967295u64;
                let t703 = [vdupq_n_u32((t702) as u32); 2];
                let t704: [uint32x4_t; 4] = [t703[0], t703[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t701[0], t704[0], res[0]), vbslq_u32(t701[1], t704[1], res[1]), vbslq_u32(t701[0], t704[2], res[2]), vbslq_u32(t701[1], t704[3], res[3])];
                live = [vbicq_u32(live[0], t701[0]), vbicq_u32(live[1], t701[1])];
            }
            if any(&live) {
                let t705: [uint32x4_t; 2] = [vsubq_u32(x2[0], x5[0]), vsubq_u32(x2[1], x5[1])];
                x6 = t705;
                let t706: u64 = 2u64;
                let t707 = [vdupq_n_u32((t706) as u32); 2];
                let t708: [uint32x4_t; 2] = [vandq_u32(x4[0], t707[0]), vandq_u32(x4[1], t707[1])];
                let t709: u64 = 0u64;
                let t710 = [vdupq_n_u32((t709) as u32); 2];
                let t711: [uint32x4_t; 2] = [vceqq_u32(t708[0], t710[0]), vceqq_u32(t708[1], t710[1])];
                let t712: [uint32x4_t; 2] = [vmvnq_u32(t711[0]), vmvnq_u32(t711[1])];
                let t713: [uint32x4_t; 2] = [vandq_u32(live[0], t712[0]), vandq_u32(live[1], t712[1])];
                {
                    let t714: [uint32x4_t; 4] = [x6[0], x6[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                    res = [vbslq_u32(t713[0], t714[0], res[0]), vbslq_u32(t713[1], t714[1], res[1]), vbslq_u32(t713[0], t714[2], res[2]), vbslq_u32(t713[1], t714[3], res[3])];
                    live = [vbicq_u32(live[0], t713[0]), vbicq_u32(live[1], t713[1])];
                }
                if any(&live) {
                    let t715: [uint32x4_t; 2] = [vceqq_u32(x6[0], t680[0]), vceqq_u32(x6[1], t680[1])];
                    let t716: [uint32x4_t; 2] = [vandq_u32(live[0], t715[0]), vandq_u32(live[1], t715[1])];
                    {
                        let t717: u64 = 4294967295u64;
                        let t718 = [vdupq_n_u32((t717) as u32); 2];
                        let t719: [uint32x4_t; 4] = [t718[0], t718[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                        res = [vbslq_u32(t716[0], t719[0], res[0]), vbslq_u32(t716[1], t719[1], res[1]), vbslq_u32(t716[0], t719[2], res[2]), vbslq_u32(t716[1], t719[3], res[3])];
                        live = [vbicq_u32(live[0], t716[0]), vbicq_u32(live[1], t716[1])];
                    }
                    if any(&live) {
                        let t720: u64 = (1u64).wrapping_shl((32u64) as u32);
                        let t721 = [vdupq_n_u32((t720) as u32); 2];
                        let t722 = [vdupq_n_u32(((t720) >> 32) as u32); 2];
                        let t723: [uint32x4_t; 4] = [vorrq_u32(t721[0], x6[0]), vorrq_u32(t721[1], x6[1]), t722[0], t722[1]];
                        res = [vbslq_u32(live[0], t723[0], res[0]), vbslq_u32(live[1], t723[1], res[1]), vbslq_u32(live[0], t723[2], res[2]), vbslq_u32(live[1], t723[3], res[3])];
                        live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
                    }
                }
            }
        }
    }
    res
}

#[inline(always)]
unsafe fn b9_r3gi_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg3: *const u8, x2: u32) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let t724: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x2), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x2), vld1q_u32(LANE[1].as_ptr()))];
    let t725 = [vdupq_n_u32(31u32); 2];
    let t726: [uint32x4_t; 2] = [vceqq_u32(t724[0], t725[0]), vceqq_u32(t724[1], t725[1])];
    let t727: [uint32x4_t; 2] = [vandq_u32(live[0], t726[0]), vandq_u32(live[1], t726[1])];
    {
        let t728: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t729: [uint32x4_t; 2] = [vshlq_u32(t728[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(t728[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
        let t730: [uint32x4_t; 2] = [vshlq_u32(t728[0], vnegq_s32(vdupq_n_s32(((5u32) & 31) as i32))), vshlq_u32(t728[1], vnegq_s32(vdupq_n_s32(((5u32) & 31) as i32)))];
        let t731: [uint32x4_t; 2] = [vandq_u32(t730[0], t725[0]), vandq_u32(t730[1], t725[1])];
        let t732 = [vdupq_n_u32(1u32); 2];
        let t733: [uint32x4_t; 2] = [vshlq_u32(t732[0], vreinterpretq_s32_u32(vandq_u32(t731[0], vdupq_n_u32(31)))), vshlq_u32(t732[1], vreinterpretq_s32_u32(vandq_u32(t731[1], vdupq_n_u32(31))))];
        let t734 = b13_al4r3l_p(l, lid0, t727, arg3, t729, t733);
    }
}

#[inline(always)]
unsafe fn b9_r3gl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg3: *const u8, x2: [uint32x4_t; 2]) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let t735 = [vdupq_n_u32(31u32); 2];
    let t736: [uint32x4_t; 2] = [vceqq_u32(x2[0], t735[0]), vceqq_u32(x2[1], t735[1])];
    let t737: [uint32x4_t; 2] = [vandq_u32(live[0], t736[0]), vandq_u32(live[1], t736[1])];
    {
        let t738: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t739: [uint32x4_t; 2] = [vshlq_u32(t738[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(t738[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
        let t740: [uint32x4_t; 2] = [vshlq_u32(t738[0], vnegq_s32(vdupq_n_s32(((5u32) & 31) as i32))), vshlq_u32(t738[1], vnegq_s32(vdupq_n_s32(((5u32) & 31) as i32)))];
        let t741: [uint32x4_t; 2] = [vandq_u32(t740[0], t735[0]), vandq_u32(t740[1], t735[1])];
        let t742 = [vdupq_n_u32(1u32); 2];
        let t743: [uint32x4_t; 2] = [vshlq_u32(t742[0], vreinterpretq_s32_u32(vandq_u32(t741[0], vdupq_n_u32(31)))), vshlq_u32(t742[1], vreinterpretq_s32_u32(vandq_u32(t741[1], vdupq_n_u32(31))))];
        let t744 = b13_al4r3l_p(l, lid0, t737, arg3, t739, t743);
    }
}

#[inline(always)]
unsafe fn b10_r3ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg3: *const u8, x1: [uint32x4_t; 2], x2: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x3: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t745: [uint32x4_t; 2] = [vshlq_u32(x1[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x1[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t746 = b11_al4r3_p(l, lid0, live, arg3, t745);
    x3 = t746;
    let t747: [uint32x4_t; 2] = [vcgeq_u32(x1[0], x2[0]), vcgeq_u32(x1[1], x2[1])];
    let t748: [uint32x4_t; 2] = [vshlq_u32(x1[0], vnegq_s32(vdupq_n_s32(((5u32) & 31) as i32))), vshlq_u32(x1[1], vnegq_s32(vdupq_n_s32(((5u32) & 31) as i32)))];
    let t749 = [vdupq_n_u32(31u32); 2];
    let t750: [uint32x4_t; 2] = [vandq_u32(t748[0], t749[0]), vandq_u32(t748[1], t749[1])];
    let t751: [uint32x4_t; 2] = [vshlq_u32(x3[0], vnegq_s32(vreinterpretq_s32_u32(vandq_u32(t750[0], vdupq_n_u32(31))))), vshlq_u32(x3[1], vnegq_s32(vreinterpretq_s32_u32(vandq_u32(t750[1], vdupq_n_u32(31)))))];
    let t752 = [vdupq_n_u32(1u32); 2];
    let t753: [uint32x4_t; 2] = [vandq_u32(t751[0], t752[0]), vandq_u32(t751[1], t752[1])];
    let t754 = [vdupq_n_u32(0u32); 2];
    let t755: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t753[0], t754[0])), vmvnq_u32(vceqq_u32(t753[1], t754[1]))];
    let t756: [uint32x4_t; 2] = [vorrq_u32(t747[0], t755[0]), vorrq_u32(t747[1], t755[1])];
    res = t756;
    return res;
}

#[inline(always)]
unsafe fn b11_al4r2_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg2: *const u8, x0i: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t757 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], x0i[0]), vandq_u32(live[1], x0i[1]))));
    let t758: [uint32x4_t; 2] = [vbslq_u32(live[0], x0i[0], t757), vbslq_u32(live[1], x0i[1], t757)];
    let t759 = arg2 as *const u32;
    let mut t760: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    t760[0] = vld1q_lane_u32::<0>(t759.add(vgetq_lane_u32::<0>(t758[0]) as usize), t760[0]);
    t760[0] = vld1q_lane_u32::<1>(t759.add(vgetq_lane_u32::<1>(t758[0]) as usize), t760[0]);
    t760[0] = vld1q_lane_u32::<2>(t759.add(vgetq_lane_u32::<2>(t758[0]) as usize), t760[0]);
    t760[0] = vld1q_lane_u32::<3>(t759.add(vgetq_lane_u32::<3>(t758[0]) as usize), t760[0]);
    t760[1] = vld1q_lane_u32::<0>(t759.add(vgetq_lane_u32::<0>(t758[1]) as usize), t760[1]);
    t760[1] = vld1q_lane_u32::<1>(t759.add(vgetq_lane_u32::<1>(t758[1]) as usize), t760[1]);
    t760[1] = vld1q_lane_u32::<2>(t759.add(vgetq_lane_u32::<2>(t758[1]) as usize), t760[1]);
    t760[1] = vld1q_lane_u32::<3>(t759.add(vgetq_lane_u32::<3>(t758[1]) as usize), t760[1]);
    res = t760;
    return res;
}

#[inline(always)]
unsafe fn b11_al4r3_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg3: *const u8, x0i: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t761 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], x0i[0]), vandq_u32(live[1], x0i[1]))));
    let t762: [uint32x4_t; 2] = [vbslq_u32(live[0], x0i[0], t761), vbslq_u32(live[1], x0i[1], t761)];
    let t763 = arg3 as *const u32;
    let mut t764: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    t764[0] = vld1q_lane_u32::<0>(t763.add(vgetq_lane_u32::<0>(t762[0]) as usize), t764[0]);
    t764[0] = vld1q_lane_u32::<1>(t763.add(vgetq_lane_u32::<1>(t762[0]) as usize), t764[0]);
    t764[0] = vld1q_lane_u32::<2>(t763.add(vgetq_lane_u32::<2>(t762[0]) as usize), t764[0]);
    t764[0] = vld1q_lane_u32::<3>(t763.add(vgetq_lane_u32::<3>(t762[0]) as usize), t764[0]);
    t764[1] = vld1q_lane_u32::<0>(t763.add(vgetq_lane_u32::<0>(t762[1]) as usize), t764[1]);
    t764[1] = vld1q_lane_u32::<1>(t763.add(vgetq_lane_u32::<1>(t762[1]) as usize), t764[1]);
    t764[1] = vld1q_lane_u32::<2>(t763.add(vgetq_lane_u32::<2>(t762[1]) as usize), t764[1]);
    t764[1] = vld1q_lane_u32::<3>(t763.add(vgetq_lane_u32::<3>(t762[1]) as usize), t764[1]);
    res = t764;
    return res;
}

#[inline(always)]
unsafe fn b12_r0r1i_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t765 = b14_r0r1si_p(l, lid0, live, arg0, arg1, 0u32, x2);
    res = t765;
    return res;
}

#[inline(always)]
unsafe fn b12_r0r1i_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t766 = b14_r0r1si_f(l, lid0, arg0, arg1, 0u32, x2);
    res = t766;
    return res;
}

#[inline(always)]
unsafe fn b12_r0r1u8_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t767 = b14_r0r1su8_p(l, lid0, live, arg0, arg1, 0u32, x2);
    res = t767;
    return res;
}

#[inline(always)]
unsafe fn b12_r0r1u8_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t768 = b14_r0r1su8_f(l, lid0, arg0, arg1, 0u32, x2);
    res = t768;
    return res;
}

#[inline(always)]
unsafe fn b12_r0r1l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t769 = b14_r0r1sl_p(l, lid0, live, arg0, arg1, 0u32, x2);
    res = t769;
    return res;
}

#[inline(always)]
unsafe fn b13_al4r3l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg3: *const u8, x0i: [uint32x4_t; 2], x1: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let (mut t770, mut t771, mut t772, mut t773) = ([0u32; 8], [0u32; 8], [0u32; 8], [0u32; 8]);
    for r in 0..2 { vst1q_u32(t770.as_mut_ptr().add(4 * r), live[r]); vst1q_u32(t771.as_mut_ptr().add(4 * r), x0i[r]); vst1q_u32(t772.as_mut_ptr().add(4 * r), x1[r]); }
    for j in 0..8 { if t770[j] != 0 { t773[j] = AtomicU32::from_ptr((arg3 as *mut u8).add(t771[j] as usize * 4) as *mut u32).fetch_or(t772[j], Relaxed); } }
    let t774: [uint32x4_t; 2] = [vld1q_u32(t773.as_ptr().add(0)), vld1q_u32(t773.as_ptr().add(4))];
    res = t774;
    return res;
}

#[inline(always)]
unsafe fn b14_r0r1si_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u32 = 0;
    let mut x5: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x6: u64 = 0;
    let mut x7: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t775 = b15_r0r1s_p(l, lid0, live, arg0, arg1, x2);
    x4 = t775;
    let t776: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[1].as_ptr()))];
    let t777: [uint32x4_t; 2] = [vshlq_u32(t776[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(t776[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t778 = b16_r0r1sl_p(l, lid0, live, arg0, arg1, (1u32).wrapping_add(x2), t777);
    x5 = t778;
    let t779 = b17_s_p(l, lid0, live, x4);
    x6 = t779;
    let t780 = [vdupq_n_u32(1023u32); 2];
    let t781: [uint32x4_t; 2] = [vandq_u32(t776[0], t780[0]), vandq_u32(t776[1], t780[1])];
    let t782 = [vdupq_n_u32(x4); 2];
    let t783: [uint32x4_t; 2] = [vmulq_u32(t781[0], t782[0]), vmulq_u32(t781[1], t782[1])];
    let t784 = b18_r0r1lls_p(l, lid0, live, arg0, arg1, x5, t783, 8u32);
    let t785: u64 = x6;
    let t786 = [vdupq_n_u32((t785) as u32); 2];
    let t787 = [vdupq_n_u32(((t785) >> 32) as u32); 2];
    let t788: [uint32x4_t; 4] = [vandq_u32(t784[0], t786[0]), vandq_u32(t784[1], t786[1]), vandq_u32(t784[2], t787[0]), vandq_u32(t784[3], t787[1])];
    x7 = t788;
    let t789: u64 = x6;
    let t790 = [vdupq_n_u32((t789) as u32); 2];
    let t791 = [vdupq_n_u32(((t789) >> 32) as u32); 2];
    let t792: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x7[0], t790[0]), vceqq_u32(x7[2], t791[0])), vandq_u32(vceqq_u32(x7[1], t790[1]), vceqq_u32(x7[3], t791[1]))];
    let mut t793: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t794: [uint32x4_t; 2] = [vandq_u32(live[0], t792[0]), vandq_u32(live[1], t792[1])];
    let t795: [uint32x4_t; 2] = [vbicq_u32(live[0], t792[0]), vbicq_u32(live[1], t792[1])];
    if any(&t794) {
        let t796: [uint32x4_t; 4];
        let t797 = bits(&t794);
        if t797.count_ones() <= 2 {
            let mut t798 = [0u32; 16];
            let mut t799 = t797;
            while t799 != 0 {
                let j = t799.trailing_zeros() as usize;
                t799 &= t799 - 1;
                let t800 = b19_r0r1ss_f(l, lid0, arg0, arg1, (x2).wrapping_add(1u32), (x3).wrapping_add(j as u32));
                let t801: u64 = t800;
                t798[j] = t801 as u32;
                t798[8 + j] = (t801 >> 32) as u32;
            }
            t796 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t798);
        } else {
            let t802 = b19_r0r1si_p(l, lid0, t794, arg0, arg1, (x2).wrapping_add(1u32), x3);
            t796 = t802;
        }
        t793 = [vbslq_u32(t794[0], t796[0], t793[0]), vbslq_u32(t794[1], t796[1], t793[1]), vbslq_u32(t794[0], t796[2], t793[2]), vbslq_u32(t794[1], t796[3], t793[3])];
    }
    {
        t793 = [vbslq_u32(t795[0], x7[0], t793[0]), vbslq_u32(t795[1], x7[1], t793[1]), vbslq_u32(t795[0], x7[2], t793[2]), vbslq_u32(t795[1], x7[3], t793[3])];
    }
    res = t793;
    return res;
}

#[inline(always)]
unsafe fn b14_r0r1si_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32, x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u32 = 0;
    let mut x5: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x6: u64 = 0;
    let mut x7: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t803 = b15_r0r1s_f(l, lid0, arg0, arg1, x2);
    x4 = t803;
    let t804: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[1].as_ptr()))];
    let t805: [uint32x4_t; 2] = [vshlq_u32(t804[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(t804[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t806 = b16_r0r1sl_f(l, lid0, arg0, arg1, (1u32).wrapping_add(x2), t805);
    x5 = t806;
    let t807 = b17_s_f(l, lid0, x4);
    x6 = t807;
    let t808 = [vdupq_n_u32(1023u32); 2];
    let t809: [uint32x4_t; 2] = [vandq_u32(t804[0], t808[0]), vandq_u32(t804[1], t808[1])];
    let t810 = [vdupq_n_u32(x4); 2];
    let t811: [uint32x4_t; 2] = [vmulq_u32(t809[0], t810[0]), vmulq_u32(t809[1], t810[1])];
    let t812 = b18_r0r1lls_f(l, lid0, arg0, arg1, x5, t811, 8u32);
    let t813: u64 = x6;
    let t814 = [vdupq_n_u32((t813) as u32); 2];
    let t815 = [vdupq_n_u32(((t813) >> 32) as u32); 2];
    let t816: [uint32x4_t; 4] = [vandq_u32(t812[0], t814[0]), vandq_u32(t812[1], t814[1]), vandq_u32(t812[2], t815[0]), vandq_u32(t812[3], t815[1])];
    x7 = t816;
    let t817: u64 = x6;
    let t818 = [vdupq_n_u32((t817) as u32); 2];
    let t819 = [vdupq_n_u32(((t817) >> 32) as u32); 2];
    let t820: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x7[0], t818[0]), vceqq_u32(x7[2], t819[0])), vandq_u32(vceqq_u32(x7[1], t818[1]), vceqq_u32(x7[3], t819[1]))];
    let mut t821: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t822: [uint32x4_t; 2] = [vmvnq_u32(t820[0]), vmvnq_u32(t820[1])];
    if any(&t820) {
        let t823: [uint32x4_t; 4];
        let t824 = bits(&t820);
        if t824.count_ones() <= 2 {
            let mut t825 = [0u32; 16];
            let mut t826 = t824;
            while t826 != 0 {
                let j = t826.trailing_zeros() as usize;
                t826 &= t826 - 1;
                let t827 = b19_r0r1ss_f(l, lid0, arg0, arg1, (x2).wrapping_add(1u32), (x3).wrapping_add(j as u32));
                let t828: u64 = t827;
                t825[j] = t828 as u32;
                t825[8 + j] = (t828 >> 32) as u32;
            }
            t823 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t825);
        } else {
            let t829 = b19_r0r1si_p(l, lid0, t820, arg0, arg1, (x2).wrapping_add(1u32), x3);
            t823 = t829;
        }
        t821 = [vbslq_u32(t820[0], t823[0], t821[0]), vbslq_u32(t820[1], t823[1], t821[1]), vbslq_u32(t820[0], t823[2], t821[2]), vbslq_u32(t820[1], t823[3], t821[3])];
    }
    {
        t821 = [vbslq_u32(t822[0], x7[0], t821[0]), vbslq_u32(t822[1], x7[1], t821[1]), vbslq_u32(t822[0], x7[2], t821[2]), vbslq_u32(t822[1], x7[3], t821[3])];
    }
    res = t821;
    return res;
}

#[inline(always)]
unsafe fn b14_r0r1su8_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u32 = 0;
    let mut x5: u32 = 0;
    let mut x6: u64 = 0;
    let mut x7: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t830 = b15_r0r1s_p(l, lid0, live, arg0, arg1, x2);
    x4 = t830;
    let t831 = b16_r0r1ss_p(l, lid0, live, arg0, arg1, (1u32).wrapping_add(x2), (x3 >> 10));
    x5 = t831;
    let t832 = b17_s_p(l, lid0, live, x4);
    x6 = t832;
    let t833 = b18_r0r1sns_p(l, lid0, live, arg0, arg1, x5, (x3 & 1023u32).wrapping_mul(x4), x4, 8u32);
    let t834: u64 = x6;
    let t835 = [vdupq_n_u32((t834) as u32); 2];
    let t836 = [vdupq_n_u32(((t834) >> 32) as u32); 2];
    let t837: [uint32x4_t; 4] = [vandq_u32(t833[0], t835[0]), vandq_u32(t833[1], t835[1]), vandq_u32(t833[2], t836[0]), vandq_u32(t833[3], t836[1])];
    x7 = t837;
    let t838: u64 = x6;
    let t839 = [vdupq_n_u32((t838) as u32); 2];
    let t840 = [vdupq_n_u32(((t838) >> 32) as u32); 2];
    let t841: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x7[0], t839[0]), vceqq_u32(x7[2], t840[0])), vandq_u32(vceqq_u32(x7[1], t839[1]), vceqq_u32(x7[3], t840[1]))];
    let mut t842: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t843: [uint32x4_t; 2] = [vandq_u32(live[0], t841[0]), vandq_u32(live[1], t841[1])];
    let t844: [uint32x4_t; 2] = [vbicq_u32(live[0], t841[0]), vbicq_u32(live[1], t841[1])];
    if any(&t843) {
        let t845: [uint32x4_t; 4];
        let t846 = bits(&t843);
        if t846.count_ones() <= 2 {
            let mut t847 = [0u32; 16];
            let mut t848 = t846;
            while t848 != 0 {
                let j = t848.trailing_zeros() as usize;
                t848 &= t848 - 1;
                let t849 = b19_r0r1ss_f(l, lid0, arg0, arg1, (x2).wrapping_add(1u32), (x3).wrapping_add(j as u32));
                let t850: u64 = t849;
                t847[j] = t850 as u32;
                t847[8 + j] = (t850 >> 32) as u32;
            }
            t845 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t847);
        } else {
            let t851 = b19_r0r1su8_p(l, lid0, t843, arg0, arg1, (x2).wrapping_add(1u32), x3);
            t845 = t851;
        }
        t842 = [vbslq_u32(t843[0], t845[0], t842[0]), vbslq_u32(t843[1], t845[1], t842[1]), vbslq_u32(t843[0], t845[2], t842[2]), vbslq_u32(t843[1], t845[3], t842[3])];
    }
    {
        t842 = [vbslq_u32(t844[0], x7[0], t842[0]), vbslq_u32(t844[1], x7[1], t842[1]), vbslq_u32(t844[0], x7[2], t842[2]), vbslq_u32(t844[1], x7[3], t842[3])];
    }
    res = t842;
    return res;
}

#[inline(always)]
unsafe fn b14_r0r1su8_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32, x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u32 = 0;
    let mut x5: u32 = 0;
    let mut x6: u64 = 0;
    let mut x7: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t852 = b15_r0r1s_f(l, lid0, arg0, arg1, x2);
    x4 = t852;
    let t853 = b16_r0r1ss_f(l, lid0, arg0, arg1, (1u32).wrapping_add(x2), (x3 >> 10));
    x5 = t853;
    let t854 = b17_s_f(l, lid0, x4);
    x6 = t854;
    let t855 = b18_r0r1sns_f(l, lid0, arg0, arg1, x5, (x3 & 1023u32).wrapping_mul(x4), x4, 8u32);
    let t856: u64 = x6;
    let t857 = [vdupq_n_u32((t856) as u32); 2];
    let t858 = [vdupq_n_u32(((t856) >> 32) as u32); 2];
    let t859: [uint32x4_t; 4] = [vandq_u32(t855[0], t857[0]), vandq_u32(t855[1], t857[1]), vandq_u32(t855[2], t858[0]), vandq_u32(t855[3], t858[1])];
    x7 = t859;
    let t860: u64 = x6;
    let t861 = [vdupq_n_u32((t860) as u32); 2];
    let t862 = [vdupq_n_u32(((t860) >> 32) as u32); 2];
    let t863: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x7[0], t861[0]), vceqq_u32(x7[2], t862[0])), vandq_u32(vceqq_u32(x7[1], t861[1]), vceqq_u32(x7[3], t862[1]))];
    let mut t864: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t865: [uint32x4_t; 2] = [vmvnq_u32(t863[0]), vmvnq_u32(t863[1])];
    if any(&t863) {
        let t866: [uint32x4_t; 4];
        let t867 = bits(&t863);
        if t867.count_ones() <= 2 {
            let mut t868 = [0u32; 16];
            let mut t869 = t867;
            while t869 != 0 {
                let j = t869.trailing_zeros() as usize;
                t869 &= t869 - 1;
                let t870 = b19_r0r1ss_f(l, lid0, arg0, arg1, (x2).wrapping_add(1u32), (x3).wrapping_add(j as u32));
                let t871: u64 = t870;
                t868[j] = t871 as u32;
                t868[8 + j] = (t871 >> 32) as u32;
            }
            t866 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t868);
        } else {
            let t872 = b19_r0r1su8_p(l, lid0, t863, arg0, arg1, (x2).wrapping_add(1u32), x3);
            t866 = t872;
        }
        t864 = [vbslq_u32(t863[0], t866[0], t864[0]), vbslq_u32(t863[1], t866[1], t864[1]), vbslq_u32(t863[0], t866[2], t864[2]), vbslq_u32(t863[1], t866[3], t864[3])];
    }
    {
        t864 = [vbslq_u32(t865[0], x7[0], t864[0]), vbslq_u32(t865[1], x7[1], t864[1]), vbslq_u32(t865[0], x7[2], t864[2]), vbslq_u32(t865[1], x7[3], t864[3])];
    }
    res = t864;
    return res;
}

#[inline(always)]
unsafe fn b14_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u32 = 0;
    let mut x5: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x6: u64 = 0;
    let mut x7: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t873 = b15_r0r1s_p(l, lid0, live, arg0, arg1, x2);
    x4 = t873;
    let t874: [uint32x4_t; 2] = [vshlq_u32(x3[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x3[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t875 = b16_r0r1sl_p(l, lid0, live, arg0, arg1, (1u32).wrapping_add(x2), t874);
    x5 = t875;
    let t876 = b17_s_p(l, lid0, live, x4);
    x6 = t876;
    let t877 = [vdupq_n_u32(1023u32); 2];
    let t878: [uint32x4_t; 2] = [vandq_u32(x3[0], t877[0]), vandq_u32(x3[1], t877[1])];
    let t879 = [vdupq_n_u32(x4); 2];
    let t880: [uint32x4_t; 2] = [vmulq_u32(t878[0], t879[0]), vmulq_u32(t878[1], t879[1])];
    let t881 = b18_r0r1lls_p(l, lid0, live, arg0, arg1, x5, t880, 8u32);
    let t882: u64 = x6;
    let t883 = [vdupq_n_u32((t882) as u32); 2];
    let t884 = [vdupq_n_u32(((t882) >> 32) as u32); 2];
    let t885: [uint32x4_t; 4] = [vandq_u32(t881[0], t883[0]), vandq_u32(t881[1], t883[1]), vandq_u32(t881[2], t884[0]), vandq_u32(t881[3], t884[1])];
    x7 = t885;
    let t886: u64 = x6;
    let t887 = [vdupq_n_u32((t886) as u32); 2];
    let t888 = [vdupq_n_u32(((t886) >> 32) as u32); 2];
    let t889: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x7[0], t887[0]), vceqq_u32(x7[2], t888[0])), vandq_u32(vceqq_u32(x7[1], t887[1]), vceqq_u32(x7[3], t888[1]))];
    let mut t890: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t891: [uint32x4_t; 2] = [vandq_u32(live[0], t889[0]), vandq_u32(live[1], t889[1])];
    let t892: [uint32x4_t; 2] = [vbicq_u32(live[0], t889[0]), vbicq_u32(live[1], t889[1])];
    if any(&t891) {
        let t893: [uint32x4_t; 4];
        let t894 = bits(&t891);
        if t894.count_ones() <= 2 {
            let t895 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x3);
            let mut t896 = [0u32; 16];
            let mut t897 = t894;
            while t897 != 0 {
                let j = t897.trailing_zeros() as usize;
                t897 &= t897 - 1;
                let t898 = b19_r0r1ss_f(l, lid0, arg0, arg1, (x2).wrapping_add(1u32), t895[j]);
                let t899: u64 = t898;
                t896[j] = t899 as u32;
                t896[8 + j] = (t899 >> 32) as u32;
            }
            t893 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t896);
        } else {
            let t900 = b19_r0r1sl_p(l, lid0, t891, arg0, arg1, (x2).wrapping_add(1u32), x3);
            t893 = t900;
        }
        t890 = [vbslq_u32(t891[0], t893[0], t890[0]), vbslq_u32(t891[1], t893[1], t890[1]), vbslq_u32(t891[0], t893[2], t890[2]), vbslq_u32(t891[1], t893[3], t890[3])];
    }
    {
        t890 = [vbslq_u32(t892[0], x7[0], t890[0]), vbslq_u32(t892[1], x7[1], t890[1]), vbslq_u32(t892[0], x7[2], t890[2]), vbslq_u32(t892[1], x7[3], t890[3])];
    }
    res = t890;
    return res;
}

#[inline(always)]
unsafe fn b15_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t901 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    res = (((t901).wrapping_shr(((4u32).wrapping_mul(x2)) as u32)) & (15u32));
    return res;
}

#[inline(always)]
unsafe fn b15_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t902 = b20_r0r1s_f(l, lid0, arg0, arg1, 16u32);
    res = (((t902).wrapping_shr(((4u32).wrapping_mul(x2)) as u32)) & (15u32));
    return res;
}

#[inline(always)]
unsafe fn b16_r0r1ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u32 = 0;
    let mut x5: u32 = 0;
    let mut live = k;
    let mut res: u32 = 0;
    let t903 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 40u32);
    let t904 = b21_r0r1s_p(l, lid0, live, arg0, arg1, 0u32);
    x4 = ((x3).wrapping_mul(t903)).wrapping_add(t904);
    if ((x2) == (0u32)) {
        res = x4;
        return res;
    }
    let t905 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    let t906 = b22_ss_p(l, lid0, live, (t905).wrapping_sub((x3).wrapping_shl((10u32) as u32)), 1024u32);
    x5 = t906;
    let t907 = b21_r0r1s_p(l, lid0, live, arg0, arg1, x2);
    res = (x4).wrapping_add((x5).wrapping_mul(t907));
    return res;
}

#[inline(always)]
unsafe fn b16_r0r1ss_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32, x3: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u32 = 0;
    let mut x5: u32 = 0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t908 = b20_r0r1s_f(l, lid0, arg0, arg1, 40u32);
    let t909 = b21_r0r1s_f(l, lid0, arg0, arg1, 0u32);
    x4 = ((x3).wrapping_mul(t908)).wrapping_add(t909);
    if ((x2) == (0u32)) {
        res = x4;
        return res;
    }
    let t910 = b20_r0r1s_f(l, lid0, arg0, arg1, 4u32);
    let t911 = b22_ss_f(l, lid0, (t910).wrapping_sub((x3).wrapping_shl((10u32) as u32)), 1024u32);
    x5 = t911;
    let t912 = b21_r0r1s_f(l, lid0, arg0, arg1, x2);
    res = (x4).wrapping_add((x5).wrapping_mul(t912));
    return res;
}

#[inline(always)]
unsafe fn b16_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x5: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t913 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 40u32);
    let t914 = [vdupq_n_u32(t913); 2];
    let t915: [uint32x4_t; 2] = [vmulq_u32(x3[0], t914[0]), vmulq_u32(x3[1], t914[1])];
    let t916 = b21_r0r1s_p(l, lid0, live, arg0, arg1, 0u32);
    let t917 = [vdupq_n_u32(t916); 2];
    let t918: [uint32x4_t; 2] = [vaddq_u32(t915[0], t917[0]), vaddq_u32(t915[1], t917[1])];
    x4 = t918;
    if ((x2) == (0u32)) {
        res = x4;
        return res;
    }
    let t919 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    let t920: [uint32x4_t; 2] = [vshlq_u32(x3[0], vdupq_n_s32(((10u32) & 31) as i32)), vshlq_u32(x3[1], vdupq_n_s32(((10u32) & 31) as i32))];
    let t921 = [vdupq_n_u32(t919); 2];
    let t922: [uint32x4_t; 2] = [vsubq_u32(t921[0], t920[0]), vsubq_u32(t921[1], t920[1])];
    let t923 = b22_ls_p(l, lid0, live, t922, 1024u32);
    x5 = t923;
    let t924 = b21_r0r1s_p(l, lid0, live, arg0, arg1, x2);
    let t925 = [vdupq_n_u32(t924); 2];
    let t926: [uint32x4_t; 2] = [vmulq_u32(x5[0], t925[0]), vmulq_u32(x5[1], t925[1])];
    let t927: [uint32x4_t; 2] = [vaddq_u32(x4[0], t926[0]), vaddq_u32(x4[1], t926[1])];
    res = t927;
    return res;
}

#[inline(always)]
unsafe fn b16_r0r1sl_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32, x3: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x5: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t928 = b20_r0r1s_f(l, lid0, arg0, arg1, 40u32);
    let t929 = [vdupq_n_u32(t928); 2];
    let t930: [uint32x4_t; 2] = [vmulq_u32(x3[0], t929[0]), vmulq_u32(x3[1], t929[1])];
    let t931 = b21_r0r1s_f(l, lid0, arg0, arg1, 0u32);
    let t932 = [vdupq_n_u32(t931); 2];
    let t933: [uint32x4_t; 2] = [vaddq_u32(t930[0], t932[0]), vaddq_u32(t930[1], t932[1])];
    x4 = t933;
    if ((x2) == (0u32)) {
        res = x4;
        return res;
    }
    let t934 = b20_r0r1s_f(l, lid0, arg0, arg1, 4u32);
    let t935: [uint32x4_t; 2] = [vshlq_u32(x3[0], vdupq_n_s32(((10u32) & 31) as i32)), vshlq_u32(x3[1], vdupq_n_s32(((10u32) & 31) as i32))];
    let t936 = [vdupq_n_u32(t934); 2];
    let t937: [uint32x4_t; 2] = [vsubq_u32(t936[0], t935[0]), vsubq_u32(t936[1], t935[1])];
    let t938 = b22_ls_f(l, lid0, t937, 1024u32);
    x5 = t938;
    let t939 = b21_r0r1s_f(l, lid0, arg0, arg1, x2);
    let t940 = [vdupq_n_u32(t939); 2];
    let t941: [uint32x4_t; 2] = [vmulq_u32(x5[0], t940[0]), vmulq_u32(x5[1], t940[1])];
    let t942: [uint32x4_t; 2] = [vaddq_u32(x4[0], t941[0]), vaddq_u32(x4[1], t941[1])];
    res = t942;
    return res;
}

#[inline(always)]
unsafe fn b17_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t943: u64;
    if ((x0) >= (8u32)) {
        t943 = !(0u64);
    } else {
        t943 = ((1u64).wrapping_shl(((((x0).wrapping_mul(8u32)) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t943;
    return res;
}

#[inline(always)]
unsafe fn b17_s_f(l: &Launch, lid0: u32, x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t944: u64;
    if ((x0) >= (8u32)) {
        t944 = !(0u64);
    } else {
        t944 = ((1u64).wrapping_shl(((((x0).wrapping_mul(8u32)) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t944;
    return res;
}

#[inline(always)]
unsafe fn b18_r0r1sns_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3f: u32, x3s: u32, x4: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t945 = b23_r0sns_p(l, lid0, live, arg0, x2, x3f, x3s, x4);
    res = t945;
    return res;
}

#[inline(always)]
unsafe fn b18_r0r1sns_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32, x3f: u32, x3s: u32, x4: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t946 = b23_r0sns_f(l, lid0, arg0, x2, x3f, x3s, x4);
    res = t946;
    return res;
}

#[inline(always)]
unsafe fn b18_r0r1lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: [uint32x4_t; 2], x3: [uint32x4_t; 2], x4: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t947 = b23_r0lls_p(l, lid0, live, arg0, x2, x3, x4);
    res = t947;
    return res;
}

#[inline(always)]
unsafe fn b18_r0r1lls_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: [uint32x4_t; 2], x3: [uint32x4_t; 2], x4: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t948 = b23_r0lls_f(l, lid0, arg0, x2, x3, x4);
    res = t948;
    return res;
}

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
    let t949 = b20_r0r1s_f(l, lid0, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x2)));
    x4 = t949;
    let t950 = b20_r0r1s_f(l, lid0, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x2)));
    x5 = t950;
    x6 = (x3).wrapping_shr((10u32) as u32);
    let t951 = b24_r0r1sss_f(l, lid0, arg0, arg1, x5, (x6).wrapping_mul(4u32), 4u32);
    x7 = ((t951) as u32);
    let t952 = b24_r0r1sss_f(l, lid0, arg0, arg1, x5, ((x6).wrapping_mul(4u32)).wrapping_add(4u32), 4u32);
    x8 = ((t952) as u32);
    let t953 = b20_r0r1s_f(l, lid0, arg0, arg1, 4u32);
    x9 = ((t953).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
    let t954 = b25_ss_f(l, lid0, 4u32, (x9).wrapping_sub((x6).wrapping_mul(4u32)));
    x10 = t954;
    x11 = (((x3).wrapping_shr((8u32) as u32)) & (3u32));
    let t955: u32;
    if ((x11) == (0u32)) {
        t955 = 0u32;
    } else {
        let t956 = b26_ss_f(l, lid0, (x7).wrapping_add((2u32).wrapping_mul((x11).wrapping_sub(1u32))), x8);
        let t957 = b24_r0r1sss_f(l, lid0, arg0, arg1, x4, t956, 2u32);
        t955 = ((t957) as u32);
    }
    x12 = t955;
    let t958 = b26_ss_f(l, lid0, (x7).wrapping_add((2u32).wrapping_mul(x11)), x8);
    let t959 = b24_r0r1sss_f(l, lid0, arg0, arg1, x4, t958, 2u32);
    x13 = ((t959) as u32);
    x14 = (x7).wrapping_add((2u32).wrapping_mul(x10));
    x15 = ((x3) & (255u32));
    x16 = 9259542123273814144u64;
    x17 = (((x15) as u64)).wrapping_mul(72340172838076673u64);
    x18 = x12;
    x19 = x12;
    loop {
        // Rounds of one condition: every lane goes on.
        if !(((x19) < (x13))) { break; }
        let t960 = b26_ss_f(l, lid0, (x14).wrapping_add(x19), x8);
        let t961 = b24_r0r1sss_f(l, lid0, arg0, arg1, x4, t960, 8u32);
        x20 = t961;
        x21 = (((x20) | (x16))).wrapping_sub(((x17) & (!(x16))));
        x22 = ((x16) & (((((!(x20)) & (x17))) | (((!(((x20) ^ (x17)))) & (!(x21)))))));
        x23 = (x13).wrapping_sub(x19);
        let t962: u64;
        if ((x23) >= (8u32)) {
            t962 = x22;
        } else {
            t962 = ((x22) & (((1u64).wrapping_shl(((((8u32).wrapping_mul(x23)) as u64)) as u32)).wrapping_sub(1u64)));
        }
        let t963 = b27_s_f(l, lid0, t962);
        x18 = (x18).wrapping_add(t963);
        x19 = (x19).wrapping_add(8u32);
    }
    x12 = x18;
    let t964 = b20_r0r1s_f(l, lid0, arg0, arg1, 16u32);
    x24 = (((t964).wrapping_shr((((((4u32).wrapping_mul(x2)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32));
    let t965 = b26_ss_f(l, lid0, (x7).wrapping_add((2u32).wrapping_mul((x10).wrapping_sub(1u32))), x8);
    let t966 = b24_r0r1sss_f(l, lid0, arg0, arg1, x4, t965, 2u32);
    x25 = ((t966) as u32);
    let t967 = b26_ss_f(l, lid0, ((x14).wrapping_add(x25)).wrapping_add((x12).wrapping_mul(x24)), x8);
    x26 = t967;
    let t968 = b24_r0r1sss_f(l, lid0, arg0, arg1, x4, x26, 8u32);
    let t969 = b17_s_f(l, lid0, x24);
    res = ((t968) & (t969));
    return res;
}

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
    let t970 = b20_r0r1s_p(l, lid0, live, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x2)));
    x4 = t970;
    let t971 = b20_r0r1s_p(l, lid0, live, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x2)));
    x5 = t971;
    let t972: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[1].as_ptr()))];
    let t973: [uint32x4_t; 2] = [vshlq_u32(t972[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(t972[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    x6 = t973;
    let t974 = [vdupq_n_u32(4u32); 2];
    let t975: [uint32x4_t; 2] = [vmulq_u32(x6[0], t974[0]), vmulq_u32(x6[1], t974[1])];
    let t976 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x5, t975, 4u32);
    let t977: [uint32x4_t; 2] = [t976[0], t976[1]];
    x7 = t977;
    let t978: [uint32x4_t; 2] = [vmulq_u32(x6[0], t974[0]), vmulq_u32(x6[1], t974[1])];
    let t979: [uint32x4_t; 2] = [vaddq_u32(t978[0], t974[0]), vaddq_u32(t978[1], t974[1])];
    let t980 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x5, t979, 4u32);
    let t981: [uint32x4_t; 2] = [t980[0], t980[1]];
    x8 = t981;
    let t982 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    x9 = ((t982).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
    let t983: [uint32x4_t; 2] = [vmulq_u32(x6[0], t974[0]), vmulq_u32(x6[1], t974[1])];
    let t984 = [vdupq_n_u32(x9); 2];
    let t985: [uint32x4_t; 2] = [vsubq_u32(t984[0], t983[0]), vsubq_u32(t984[1], t983[1])];
    let t986 = b25_sl_p(l, lid0, live, 4u32, t985);
    x10 = t986;
    let t987: [uint32x4_t; 2] = [vshlq_u32(t972[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(t972[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
    let t988 = [vdupq_n_u32(3u32); 2];
    let t989: [uint32x4_t; 2] = [vandq_u32(t987[0], t988[0]), vandq_u32(t987[1], t988[1])];
    x11 = t989;
    let t990 = [vdupq_n_u32(0u32); 2];
    let t991: [uint32x4_t; 2] = [vceqq_u32(x11[0], t990[0]), vceqq_u32(x11[1], t990[1])];
    let mut t992: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t993: [uint32x4_t; 2] = [vandq_u32(live[0], t991[0]), vandq_u32(live[1], t991[1])];
    let t994: [uint32x4_t; 2] = [vbicq_u32(live[0], t991[0]), vbicq_u32(live[1], t991[1])];
    {
        t992 = [vbslq_u32(t993[0], t990[0], t992[0]), vbslq_u32(t993[1], t990[1], t992[1])];
    }
    if any(&t994) {
        let t995 = [vdupq_n_u32(1u32); 2];
        let t996: [uint32x4_t; 2] = [vsubq_u32(x11[0], t995[0]), vsubq_u32(x11[1], t995[1])];
        let t997 = [vdupq_n_u32(2u32); 2];
        let t998: [uint32x4_t; 2] = [vmulq_u32(t997[0], t996[0]), vmulq_u32(t997[1], t996[1])];
        let t999: [uint32x4_t; 2] = [vaddq_u32(x7[0], t998[0]), vaddq_u32(x7[1], t998[1])];
        let t1000 = b26_ll_p(l, lid0, t994, t999, x8);
        let t1001 = b24_r0r1sls_p(l, lid0, t994, arg0, arg1, x4, t1000, 2u32);
        let t1002: [uint32x4_t; 2] = [t1001[0], t1001[1]];
        t992 = [vbslq_u32(t994[0], t1002[0], t992[0]), vbslq_u32(t994[1], t1002[1], t992[1])];
    }
    x12 = t992;
    let t1003 = [vdupq_n_u32(2u32); 2];
    let t1004: [uint32x4_t; 2] = [vmulq_u32(t1003[0], x11[0]), vmulq_u32(t1003[1], x11[1])];
    let t1005: [uint32x4_t; 2] = [vaddq_u32(x7[0], t1004[0]), vaddq_u32(x7[1], t1004[1])];
    let t1006 = b26_ll_p(l, lid0, live, t1005, x8);
    let t1007 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x4, t1006, 2u32);
    let t1008: [uint32x4_t; 2] = [t1007[0], t1007[1]];
    x13 = t1008;
    let t1009: [uint32x4_t; 2] = [vmulq_u32(t1003[0], x10[0]), vmulq_u32(t1003[1], x10[1])];
    let t1010: [uint32x4_t; 2] = [vaddq_u32(x7[0], t1009[0]), vaddq_u32(x7[1], t1009[1])];
    x14 = t1010;
    let t1011 = [vdupq_n_u32(255u32); 2];
    let t1012: [uint32x4_t; 2] = [vandq_u32(t972[0], t1011[0]), vandq_u32(t972[1], t1011[1])];
    x15 = t1012;
    x16 = 9259542123273814144u64;
    let t1013: u64 = 72340172838076673u64;
    let t1014 = [vdupq_n_u32((t1013) as u32); 2];
    let t1015 = [vdupq_n_u32(((t1013) >> 32) as u32); 2];
    let t1016: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x15[0]), vget_low_u32(t1014[0]))), vreinterpretq_u32_u64(vmull_high_u32(x15[0], t1014[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x15[1]), vget_low_u32(t1014[1]))), vreinterpretq_u32_u64(vmull_high_u32(x15[1], t1014[1])))];
    let t1017: [uint32x4_t; 4] = [vmulq_u32(x15[0], t1014[0]), vmulq_u32(x15[1], t1014[1]), vmlaq_u32(t1016[0], x15[0], t1015[0]), vmlaq_u32(t1016[1], x15[1], t1015[1])];
    x17 = t1017;
    x18 = x12;
    x19 = x12;
    let mut t1019 = live;
    loop {
        // Lanes part: each its own rounds.
        let t1021: [uint32x4_t; 2] = [vcltq_u32(x19[0], x13[0]), vcltq_u32(x19[1], x13[1])];
        t1019 = [vandq_u32(t1019[0], t1021[0]), vandq_u32(t1019[1], t1021[1])];
        if !(any(&t1019)) { break; }
        let t1023: [uint32x4_t; 2] = [vaddq_u32(x14[0], x19[0]), vaddq_u32(x14[1], x19[1])];
        let t1024 = b26_ll_p(l, lid0, t1019, t1023, x8);
        let t1025 = b24_r0r1sls_p(l, lid0, t1019, arg0, arg1, x4, t1024, 8u32);
        x20 = t1025;
        let t1026: u64 = x16;
        let t1027 = [vdupq_n_u32((t1026) as u32); 2];
        let t1028 = [vdupq_n_u32(((t1026) >> 32) as u32); 2];
        let t1029: [uint32x4_t; 4] = [vorrq_u32(x20[0], t1027[0]), vorrq_u32(x20[1], t1027[1]), vorrq_u32(x20[2], t1028[0]), vorrq_u32(x20[3], t1028[1])];
        let t1030: u64 = !(x16);
        let t1031 = [vdupq_n_u32((t1030) as u32); 2];
        let t1032 = [vdupq_n_u32(((t1030) >> 32) as u32); 2];
        let t1033: [uint32x4_t; 4] = [vandq_u32(x17[0], t1031[0]), vandq_u32(x17[1], t1031[1]), vandq_u32(x17[2], t1032[0]), vandq_u32(x17[3], t1032[1])];
        let t1034: [uint32x4_t; 2] = [vcltq_u32(t1029[0], t1033[0]), vcltq_u32(t1029[1], t1033[1])];
        let t1035: [uint32x4_t; 4] = [vsubq_u32(t1029[0], t1033[0]), vsubq_u32(t1029[1], t1033[1]), vaddq_u32(vsubq_u32(t1029[2], t1033[2]), t1034[0]), vaddq_u32(vsubq_u32(t1029[3], t1033[3]), t1034[1])];
        x21 = t1035;
        let t1036: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x20[r]));
        let t1037: [uint32x4_t; 4] = [vandq_u32(t1036[0], x17[0]), vandq_u32(t1036[1], x17[1]), vandq_u32(t1036[2], x17[2]), vandq_u32(t1036[3], x17[3])];
        let t1038: [uint32x4_t; 4] = [veorq_u32(x20[0], x17[0]), veorq_u32(x20[1], x17[1]), veorq_u32(x20[2], x17[2]), veorq_u32(x20[3], x17[3])];
        let t1039: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(t1038[r]));
        let t1040: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x21[r]));
        let t1041: [uint32x4_t; 4] = [vandq_u32(t1039[0], t1040[0]), vandq_u32(t1039[1], t1040[1]), vandq_u32(t1039[2], t1040[2]), vandq_u32(t1039[3], t1040[3])];
        let t1042: [uint32x4_t; 4] = [vorrq_u32(t1037[0], t1041[0]), vorrq_u32(t1037[1], t1041[1]), vorrq_u32(t1037[2], t1041[2]), vorrq_u32(t1037[3], t1041[3])];
        let t1043: u64 = x16;
        let t1044 = [vdupq_n_u32((t1043) as u32); 2];
        let t1045 = [vdupq_n_u32(((t1043) >> 32) as u32); 2];
        let t1046: [uint32x4_t; 4] = [vandq_u32(t1044[0], t1042[0]), vandq_u32(t1044[1], t1042[1]), vandq_u32(t1045[0], t1042[2]), vandq_u32(t1045[1], t1042[3])];
        x22 = t1046;
        let t1047: [uint32x4_t; 2] = [vsubq_u32(x13[0], x19[0]), vsubq_u32(x13[1], x19[1])];
        x23 = t1047;
        let t1048 = [vdupq_n_u32(8u32); 2];
        let t1049: [uint32x4_t; 2] = [vcgeq_u32(x23[0], t1048[0]), vcgeq_u32(x23[1], t1048[1])];
        let mut t1050: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t1051: [uint32x4_t; 2] = [vandq_u32(t1019[0], t1049[0]), vandq_u32(t1019[1], t1049[1])];
        let t1052: [uint32x4_t; 2] = [vbicq_u32(t1019[0], t1049[0]), vbicq_u32(t1019[1], t1049[1])];
        {
            t1050 = [vbslq_u32(t1051[0], x22[0], t1050[0]), vbslq_u32(t1051[1], x22[1], t1050[1]), vbslq_u32(t1051[0], x22[2], t1050[2]), vbslq_u32(t1051[1], x22[3], t1050[3])];
        }
        {
            let t1053: [uint32x4_t; 2] = [vmulq_u32(t1048[0], x23[0]), vmulq_u32(t1048[1], x23[1])];
            let t1054: u64 = 1u64;
            let t1055 = [vdupq_n_u32((t1054) as u32); 2];
            let t1056: [uint32x4_t; 2] = [t1053[0], t1053[1]];
            let t1057: [uint32x4_t; 2] = [vandq_u32(t1056[0], vdupq_n_u32(63)), vandq_u32(t1056[1], vdupq_n_u32(63))];
            let t1058: [uint32x4_t; 4] = [vshlq_u32(t1055[0], vreinterpretq_s32_u32(t1057[0])), vshlq_u32(t1055[1], vreinterpretq_s32_u32(t1057[1])), vshlq_u32(t1055[0], vsubq_s32(vreinterpretq_s32_u32(t1057[0]), vdupq_n_s32(32))), vshlq_u32(t1055[1], vsubq_s32(vreinterpretq_s32_u32(t1057[1]), vdupq_n_s32(32)))];
            let t1059: u64 = 1u64;
            let t1060 = [vdupq_n_u32((t1059) as u32); 2];
            let t1061: [uint32x4_t; 2] = [vcltq_u32(t1058[0], t1060[0]), vcltq_u32(t1058[1], t1060[1])];
            let t1062: [uint32x4_t; 4] = [vsubq_u32(t1058[0], t1060[0]), vsubq_u32(t1058[1], t1060[1]), vaddq_u32(vsubq_u32(t1058[2], vdupq_n_u32(0)), t1061[0]), vaddq_u32(vsubq_u32(t1058[3], vdupq_n_u32(0)), t1061[1])];
            let t1063: [uint32x4_t; 4] = [vandq_u32(x22[0], t1062[0]), vandq_u32(x22[1], t1062[1]), vandq_u32(x22[2], t1062[2]), vandq_u32(x22[3], t1062[3])];
            t1050 = [vbslq_u32(t1052[0], t1063[0], t1050[0]), vbslq_u32(t1052[1], t1063[1], t1050[1]), vbslq_u32(t1052[0], t1063[2], t1050[2]), vbslq_u32(t1052[1], t1063[3], t1050[3])];
        }
        let t1064 = b27_l_p(l, lid0, t1019, t1050);
        let t1065: [uint32x4_t; 2] = [vaddq_u32(x18[0], t1064[0]), vaddq_u32(x18[1], t1064[1])];
        x18 = [vbslq_u32(t1019[0], t1065[0], x18[0]), vbslq_u32(t1019[1], t1065[1], x18[1])];
        let t1066: [uint32x4_t; 2] = [vandq_u32(t1019[0], live[0]), vandq_u32(t1019[1], live[1])];
        let t1067: [uint32x4_t; 2] = [vaddq_u32(x19[0], t1048[0]), vaddq_u32(x19[1], t1048[1])];
        x19 = t1067;
    }
    x12 = x18;
    let t1068 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    x24 = (((t1068).wrapping_shr((((((4u32).wrapping_mul(x2)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32));
    let t1069 = [vdupq_n_u32(1u32); 2];
    let t1070: [uint32x4_t; 2] = [vsubq_u32(x10[0], t1069[0]), vsubq_u32(x10[1], t1069[1])];
    let t1071: [uint32x4_t; 2] = [vmulq_u32(t1003[0], t1070[0]), vmulq_u32(t1003[1], t1070[1])];
    let t1072: [uint32x4_t; 2] = [vaddq_u32(x7[0], t1071[0]), vaddq_u32(x7[1], t1071[1])];
    let t1073 = b26_ll_p(l, lid0, live, t1072, x8);
    let t1074 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x4, t1073, 2u32);
    let t1075: [uint32x4_t; 2] = [t1074[0], t1074[1]];
    x25 = t1075;
    let t1076: [uint32x4_t; 2] = [vaddq_u32(x14[0], x25[0]), vaddq_u32(x14[1], x25[1])];
    let t1077 = [vdupq_n_u32(x24); 2];
    let t1078: [uint32x4_t; 2] = [vmulq_u32(x12[0], t1077[0]), vmulq_u32(x12[1], t1077[1])];
    let t1079: [uint32x4_t; 2] = [vaddq_u32(t1076[0], t1078[0]), vaddq_u32(t1076[1], t1078[1])];
    let t1080 = b26_ll_p(l, lid0, live, t1079, x8);
    x26 = t1080;
    let t1081 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x4, x26, 8u32);
    let t1082 = b17_s_p(l, lid0, live, x24);
    let t1083: u64 = t1082;
    let t1084 = [vdupq_n_u32((t1083) as u32); 2];
    let t1085 = [vdupq_n_u32(((t1083) >> 32) as u32); 2];
    let t1086: [uint32x4_t; 4] = [vandq_u32(t1081[0], t1084[0]), vandq_u32(t1081[1], t1084[1]), vandq_u32(t1081[2], t1085[0]), vandq_u32(t1081[3], t1085[1])];
    res = t1086;
    return res;
}

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
    let t1087 = b20_r0r1s_p(l, lid0, live, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x2)));
    x4 = t1087;
    let t1088 = b20_r0r1s_p(l, lid0, live, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x2)));
    x5 = t1088;
    x6 = (x3 >> 10);
    let t1089 = b24_r0r1sss_p(l, lid0, live, arg0, arg1, x5, (x6).wrapping_mul(4u32), 4u32);
    x7 = ((t1089) as u32);
    let t1090 = b24_r0r1sss_p(l, lid0, live, arg0, arg1, x5, ((x6).wrapping_mul(4u32)).wrapping_add(4u32), 4u32);
    x8 = ((t1090) as u32);
    let t1091 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    x9 = ((t1091).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
    let t1092 = b25_ss_p(l, lid0, live, 4u32, (x9).wrapping_sub((x6).wrapping_mul(4u32)));
    x10 = t1092;
    x11 = (((x3 >> 8)) & (3u32));
    let t1093: u32;
    if ((x11) == (0u32)) {
        t1093 = 0u32;
    } else {
        let t1094 = b26_ss_p(l, lid0, live, (x7).wrapping_add((2u32).wrapping_mul((x11).wrapping_sub(1u32))), x8);
        let t1095 = b24_r0r1sss_p(l, lid0, live, arg0, arg1, x4, t1094, 2u32);
        t1093 = ((t1095) as u32);
    }
    let t1096 = [vdupq_n_u32(t1093); 2];
    x12 = t1096;
    let t1097 = b26_ss_p(l, lid0, live, (x7).wrapping_add((2u32).wrapping_mul(x11)), x8);
    let t1098 = b24_r0r1sss_p(l, lid0, live, arg0, arg1, x4, t1097, 2u32);
    x13 = ((t1098) as u32);
    x14 = (x7).wrapping_add((2u32).wrapping_mul(x10));
    x15 = (x3 & 255u32);
    x16 = 9259542123273814144u64;
    let t1099: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x15), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x15), vld1q_u32(LANE[1].as_ptr()))];
    let t1100: u64 = 72340172838076673u64;
    let t1101 = [vdupq_n_u32((t1100) as u32); 2];
    let t1102 = [vdupq_n_u32(((t1100) >> 32) as u32); 2];
    let t1103: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(t1099[0]), vget_low_u32(t1101[0]))), vreinterpretq_u32_u64(vmull_high_u32(t1099[0], t1101[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(t1099[1]), vget_low_u32(t1101[1]))), vreinterpretq_u32_u64(vmull_high_u32(t1099[1], t1101[1])))];
    let t1104: [uint32x4_t; 4] = [vmulq_u32(t1099[0], t1101[0]), vmulq_u32(t1099[1], t1101[1]), vmlaq_u32(t1103[0], t1099[0], t1102[0]), vmlaq_u32(t1103[1], t1099[1], t1102[1])];
    x17 = t1104;
    x18 = x12;
    x19 = x12;
    let mut t1107 = live;
    loop {
        // Lanes part: each its own rounds.
        let t1109 = [vdupq_n_u32(x13); 2];
        let t1110: [uint32x4_t; 2] = [vcltq_u32(x19[0], t1109[0]), vcltq_u32(x19[1], t1109[1])];
        t1107 = [vandq_u32(t1107[0], t1110[0]), vandq_u32(t1107[1], t1110[1])];
        if !(any(&t1107)) { break; }
        let t1112 = [vdupq_n_u32(x14); 2];
        let t1113: [uint32x4_t; 2] = [vaddq_u32(t1112[0], x19[0]), vaddq_u32(t1112[1], x19[1])];
        let t1114 = b26_ls_p(l, lid0, t1107, t1113, x8);
        let t1115 = b24_r0r1sls_p(l, lid0, t1107, arg0, arg1, x4, t1114, 8u32);
        x20 = t1115;
        let t1116: u64 = x16;
        let t1117 = [vdupq_n_u32((t1116) as u32); 2];
        let t1118 = [vdupq_n_u32(((t1116) >> 32) as u32); 2];
        let t1119: [uint32x4_t; 4] = [vorrq_u32(x20[0], t1117[0]), vorrq_u32(x20[1], t1117[1]), vorrq_u32(x20[2], t1118[0]), vorrq_u32(x20[3], t1118[1])];
        let t1120: u64 = !(x16);
        let t1121 = [vdupq_n_u32((t1120) as u32); 2];
        let t1122 = [vdupq_n_u32(((t1120) >> 32) as u32); 2];
        let t1123: [uint32x4_t; 4] = [vandq_u32(x17[0], t1121[0]), vandq_u32(x17[1], t1121[1]), vandq_u32(x17[2], t1122[0]), vandq_u32(x17[3], t1122[1])];
        let t1124: [uint32x4_t; 2] = [vcltq_u32(t1119[0], t1123[0]), vcltq_u32(t1119[1], t1123[1])];
        let t1125: [uint32x4_t; 4] = [vsubq_u32(t1119[0], t1123[0]), vsubq_u32(t1119[1], t1123[1]), vaddq_u32(vsubq_u32(t1119[2], t1123[2]), t1124[0]), vaddq_u32(vsubq_u32(t1119[3], t1123[3]), t1124[1])];
        x21 = t1125;
        let t1126: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x20[r]));
        let t1127: [uint32x4_t; 4] = [vandq_u32(t1126[0], x17[0]), vandq_u32(t1126[1], x17[1]), vandq_u32(t1126[2], x17[2]), vandq_u32(t1126[3], x17[3])];
        let t1128: [uint32x4_t; 4] = [veorq_u32(x20[0], x17[0]), veorq_u32(x20[1], x17[1]), veorq_u32(x20[2], x17[2]), veorq_u32(x20[3], x17[3])];
        let t1129: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(t1128[r]));
        let t1130: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x21[r]));
        let t1131: [uint32x4_t; 4] = [vandq_u32(t1129[0], t1130[0]), vandq_u32(t1129[1], t1130[1]), vandq_u32(t1129[2], t1130[2]), vandq_u32(t1129[3], t1130[3])];
        let t1132: [uint32x4_t; 4] = [vorrq_u32(t1127[0], t1131[0]), vorrq_u32(t1127[1], t1131[1]), vorrq_u32(t1127[2], t1131[2]), vorrq_u32(t1127[3], t1131[3])];
        let t1133: u64 = x16;
        let t1134 = [vdupq_n_u32((t1133) as u32); 2];
        let t1135 = [vdupq_n_u32(((t1133) >> 32) as u32); 2];
        let t1136: [uint32x4_t; 4] = [vandq_u32(t1134[0], t1132[0]), vandq_u32(t1134[1], t1132[1]), vandq_u32(t1135[0], t1132[2]), vandq_u32(t1135[1], t1132[3])];
        x22 = t1136;
        let t1137 = [vdupq_n_u32(x13); 2];
        let t1138: [uint32x4_t; 2] = [vsubq_u32(t1137[0], x19[0]), vsubq_u32(t1137[1], x19[1])];
        x23 = t1138;
        let t1139 = [vdupq_n_u32(8u32); 2];
        let t1140: [uint32x4_t; 2] = [vcgeq_u32(x23[0], t1139[0]), vcgeq_u32(x23[1], t1139[1])];
        let mut t1141: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t1142: [uint32x4_t; 2] = [vandq_u32(t1107[0], t1140[0]), vandq_u32(t1107[1], t1140[1])];
        let t1143: [uint32x4_t; 2] = [vbicq_u32(t1107[0], t1140[0]), vbicq_u32(t1107[1], t1140[1])];
        {
            t1141 = [vbslq_u32(t1142[0], x22[0], t1141[0]), vbslq_u32(t1142[1], x22[1], t1141[1]), vbslq_u32(t1142[0], x22[2], t1141[2]), vbslq_u32(t1142[1], x22[3], t1141[3])];
        }
        {
            let t1144: [uint32x4_t; 2] = [vmulq_u32(t1139[0], x23[0]), vmulq_u32(t1139[1], x23[1])];
            let t1145: u64 = 1u64;
            let t1146 = [vdupq_n_u32((t1145) as u32); 2];
            let t1147: [uint32x4_t; 2] = [t1144[0], t1144[1]];
            let t1148: [uint32x4_t; 2] = [vandq_u32(t1147[0], vdupq_n_u32(63)), vandq_u32(t1147[1], vdupq_n_u32(63))];
            let t1149: [uint32x4_t; 4] = [vshlq_u32(t1146[0], vreinterpretq_s32_u32(t1148[0])), vshlq_u32(t1146[1], vreinterpretq_s32_u32(t1148[1])), vshlq_u32(t1146[0], vsubq_s32(vreinterpretq_s32_u32(t1148[0]), vdupq_n_s32(32))), vshlq_u32(t1146[1], vsubq_s32(vreinterpretq_s32_u32(t1148[1]), vdupq_n_s32(32)))];
            let t1150: u64 = 1u64;
            let t1151 = [vdupq_n_u32((t1150) as u32); 2];
            let t1152: [uint32x4_t; 2] = [vcltq_u32(t1149[0], t1151[0]), vcltq_u32(t1149[1], t1151[1])];
            let t1153: [uint32x4_t; 4] = [vsubq_u32(t1149[0], t1151[0]), vsubq_u32(t1149[1], t1151[1]), vaddq_u32(vsubq_u32(t1149[2], vdupq_n_u32(0)), t1152[0]), vaddq_u32(vsubq_u32(t1149[3], vdupq_n_u32(0)), t1152[1])];
            let t1154: [uint32x4_t; 4] = [vandq_u32(x22[0], t1153[0]), vandq_u32(x22[1], t1153[1]), vandq_u32(x22[2], t1153[2]), vandq_u32(x22[3], t1153[3])];
            t1141 = [vbslq_u32(t1143[0], t1154[0], t1141[0]), vbslq_u32(t1143[1], t1154[1], t1141[1]), vbslq_u32(t1143[0], t1154[2], t1141[2]), vbslq_u32(t1143[1], t1154[3], t1141[3])];
        }
        let t1155 = b27_l_p(l, lid0, t1107, t1141);
        let t1156: [uint32x4_t; 2] = [vaddq_u32(x18[0], t1155[0]), vaddq_u32(x18[1], t1155[1])];
        x18 = [vbslq_u32(t1107[0], t1156[0], x18[0]), vbslq_u32(t1107[1], t1156[1], x18[1])];
        let t1157: [uint32x4_t; 2] = [vandq_u32(t1107[0], live[0]), vandq_u32(t1107[1], live[1])];
        let t1158: [uint32x4_t; 2] = [vaddq_u32(x19[0], t1139[0]), vaddq_u32(x19[1], t1139[1])];
        x19 = t1158;
    }
    x12 = x18;
    let t1159 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    x24 = (((t1159).wrapping_shr((((((4u32).wrapping_mul(x2)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32));
    let t1160 = b26_ss_p(l, lid0, live, (x7).wrapping_add((2u32).wrapping_mul((x10).wrapping_sub(1u32))), x8);
    let t1161 = b24_r0r1sss_p(l, lid0, live, arg0, arg1, x4, t1160, 2u32);
    x25 = ((t1161) as u32);
    let t1162 = [vdupq_n_u32(x24); 2];
    let t1163: [uint32x4_t; 2] = [vmulq_u32(x12[0], t1162[0]), vmulq_u32(x12[1], t1162[1])];
    let t1164 = [vdupq_n_u32((x14).wrapping_add(x25)); 2];
    let t1165: [uint32x4_t; 2] = [vaddq_u32(t1164[0], t1163[0]), vaddq_u32(t1164[1], t1163[1])];
    let t1166 = b26_ls_p(l, lid0, live, t1165, x8);
    x26 = t1166;
    let t1167 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x4, x26, 8u32);
    let t1168 = b17_s_p(l, lid0, live, x24);
    let t1169: u64 = t1168;
    let t1170 = [vdupq_n_u32((t1169) as u32); 2];
    let t1171 = [vdupq_n_u32(((t1169) >> 32) as u32); 2];
    let t1172: [uint32x4_t; 4] = [vandq_u32(t1167[0], t1170[0]), vandq_u32(t1167[1], t1170[1]), vandq_u32(t1167[2], t1171[0]), vandq_u32(t1167[3], t1171[1])];
    res = t1172;
    return res;
}

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
    let t1173 = b20_r0r1s_p(l, lid0, live, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x2)));
    x4 = t1173;
    let t1174 = b20_r0r1s_p(l, lid0, live, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x2)));
    x5 = t1174;
    let t1175: [uint32x4_t; 2] = [vshlq_u32(x3[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x3[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    x6 = t1175;
    let t1176 = [vdupq_n_u32(4u32); 2];
    let t1177: [uint32x4_t; 2] = [vmulq_u32(x6[0], t1176[0]), vmulq_u32(x6[1], t1176[1])];
    let t1178 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x5, t1177, 4u32);
    let t1179: [uint32x4_t; 2] = [t1178[0], t1178[1]];
    x7 = t1179;
    let t1180: [uint32x4_t; 2] = [vmulq_u32(x6[0], t1176[0]), vmulq_u32(x6[1], t1176[1])];
    let t1181: [uint32x4_t; 2] = [vaddq_u32(t1180[0], t1176[0]), vaddq_u32(t1180[1], t1176[1])];
    let t1182 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x5, t1181, 4u32);
    let t1183: [uint32x4_t; 2] = [t1182[0], t1182[1]];
    x8 = t1183;
    let t1184 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    x9 = ((t1184).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
    let t1185: [uint32x4_t; 2] = [vmulq_u32(x6[0], t1176[0]), vmulq_u32(x6[1], t1176[1])];
    let t1186 = [vdupq_n_u32(x9); 2];
    let t1187: [uint32x4_t; 2] = [vsubq_u32(t1186[0], t1185[0]), vsubq_u32(t1186[1], t1185[1])];
    let t1188 = b25_sl_p(l, lid0, live, 4u32, t1187);
    x10 = t1188;
    let t1189: [uint32x4_t; 2] = [vshlq_u32(x3[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(x3[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
    let t1190 = [vdupq_n_u32(3u32); 2];
    let t1191: [uint32x4_t; 2] = [vandq_u32(t1189[0], t1190[0]), vandq_u32(t1189[1], t1190[1])];
    x11 = t1191;
    let t1192 = [vdupq_n_u32(0u32); 2];
    let t1193: [uint32x4_t; 2] = [vceqq_u32(x11[0], t1192[0]), vceqq_u32(x11[1], t1192[1])];
    let mut t1194: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1195: [uint32x4_t; 2] = [vandq_u32(live[0], t1193[0]), vandq_u32(live[1], t1193[1])];
    let t1196: [uint32x4_t; 2] = [vbicq_u32(live[0], t1193[0]), vbicq_u32(live[1], t1193[1])];
    {
        t1194 = [vbslq_u32(t1195[0], t1192[0], t1194[0]), vbslq_u32(t1195[1], t1192[1], t1194[1])];
    }
    if any(&t1196) {
        let t1197 = [vdupq_n_u32(1u32); 2];
        let t1198: [uint32x4_t; 2] = [vsubq_u32(x11[0], t1197[0]), vsubq_u32(x11[1], t1197[1])];
        let t1199 = [vdupq_n_u32(2u32); 2];
        let t1200: [uint32x4_t; 2] = [vmulq_u32(t1199[0], t1198[0]), vmulq_u32(t1199[1], t1198[1])];
        let t1201: [uint32x4_t; 2] = [vaddq_u32(x7[0], t1200[0]), vaddq_u32(x7[1], t1200[1])];
        let t1202 = b26_ll_p(l, lid0, t1196, t1201, x8);
        let t1203 = b24_r0r1sls_p(l, lid0, t1196, arg0, arg1, x4, t1202, 2u32);
        let t1204: [uint32x4_t; 2] = [t1203[0], t1203[1]];
        t1194 = [vbslq_u32(t1196[0], t1204[0], t1194[0]), vbslq_u32(t1196[1], t1204[1], t1194[1])];
    }
    x12 = t1194;
    let t1205 = [vdupq_n_u32(2u32); 2];
    let t1206: [uint32x4_t; 2] = [vmulq_u32(t1205[0], x11[0]), vmulq_u32(t1205[1], x11[1])];
    let t1207: [uint32x4_t; 2] = [vaddq_u32(x7[0], t1206[0]), vaddq_u32(x7[1], t1206[1])];
    let t1208 = b26_ll_p(l, lid0, live, t1207, x8);
    let t1209 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x4, t1208, 2u32);
    let t1210: [uint32x4_t; 2] = [t1209[0], t1209[1]];
    x13 = t1210;
    let t1211: [uint32x4_t; 2] = [vmulq_u32(t1205[0], x10[0]), vmulq_u32(t1205[1], x10[1])];
    let t1212: [uint32x4_t; 2] = [vaddq_u32(x7[0], t1211[0]), vaddq_u32(x7[1], t1211[1])];
    x14 = t1212;
    let t1213 = [vdupq_n_u32(255u32); 2];
    let t1214: [uint32x4_t; 2] = [vandq_u32(x3[0], t1213[0]), vandq_u32(x3[1], t1213[1])];
    x15 = t1214;
    x16 = 9259542123273814144u64;
    let t1215: u64 = 72340172838076673u64;
    let t1216 = [vdupq_n_u32((t1215) as u32); 2];
    let t1217 = [vdupq_n_u32(((t1215) >> 32) as u32); 2];
    let t1218: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x15[0]), vget_low_u32(t1216[0]))), vreinterpretq_u32_u64(vmull_high_u32(x15[0], t1216[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x15[1]), vget_low_u32(t1216[1]))), vreinterpretq_u32_u64(vmull_high_u32(x15[1], t1216[1])))];
    let t1219: [uint32x4_t; 4] = [vmulq_u32(x15[0], t1216[0]), vmulq_u32(x15[1], t1216[1]), vmlaq_u32(t1218[0], x15[0], t1217[0]), vmlaq_u32(t1218[1], x15[1], t1217[1])];
    x17 = t1219;
    x18 = x12;
    x19 = x12;
    let mut t1221 = live;
    loop {
        // Lanes part: each its own rounds.
        let t1223: [uint32x4_t; 2] = [vcltq_u32(x19[0], x13[0]), vcltq_u32(x19[1], x13[1])];
        t1221 = [vandq_u32(t1221[0], t1223[0]), vandq_u32(t1221[1], t1223[1])];
        if !(any(&t1221)) { break; }
        let t1225: [uint32x4_t; 2] = [vaddq_u32(x14[0], x19[0]), vaddq_u32(x14[1], x19[1])];
        let t1226 = b26_ll_p(l, lid0, t1221, t1225, x8);
        let t1227 = b24_r0r1sls_p(l, lid0, t1221, arg0, arg1, x4, t1226, 8u32);
        x20 = t1227;
        let t1228: u64 = x16;
        let t1229 = [vdupq_n_u32((t1228) as u32); 2];
        let t1230 = [vdupq_n_u32(((t1228) >> 32) as u32); 2];
        let t1231: [uint32x4_t; 4] = [vorrq_u32(x20[0], t1229[0]), vorrq_u32(x20[1], t1229[1]), vorrq_u32(x20[2], t1230[0]), vorrq_u32(x20[3], t1230[1])];
        let t1232: u64 = !(x16);
        let t1233 = [vdupq_n_u32((t1232) as u32); 2];
        let t1234 = [vdupq_n_u32(((t1232) >> 32) as u32); 2];
        let t1235: [uint32x4_t; 4] = [vandq_u32(x17[0], t1233[0]), vandq_u32(x17[1], t1233[1]), vandq_u32(x17[2], t1234[0]), vandq_u32(x17[3], t1234[1])];
        let t1236: [uint32x4_t; 2] = [vcltq_u32(t1231[0], t1235[0]), vcltq_u32(t1231[1], t1235[1])];
        let t1237: [uint32x4_t; 4] = [vsubq_u32(t1231[0], t1235[0]), vsubq_u32(t1231[1], t1235[1]), vaddq_u32(vsubq_u32(t1231[2], t1235[2]), t1236[0]), vaddq_u32(vsubq_u32(t1231[3], t1235[3]), t1236[1])];
        x21 = t1237;
        let t1238: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x20[r]));
        let t1239: [uint32x4_t; 4] = [vandq_u32(t1238[0], x17[0]), vandq_u32(t1238[1], x17[1]), vandq_u32(t1238[2], x17[2]), vandq_u32(t1238[3], x17[3])];
        let t1240: [uint32x4_t; 4] = [veorq_u32(x20[0], x17[0]), veorq_u32(x20[1], x17[1]), veorq_u32(x20[2], x17[2]), veorq_u32(x20[3], x17[3])];
        let t1241: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(t1240[r]));
        let t1242: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x21[r]));
        let t1243: [uint32x4_t; 4] = [vandq_u32(t1241[0], t1242[0]), vandq_u32(t1241[1], t1242[1]), vandq_u32(t1241[2], t1242[2]), vandq_u32(t1241[3], t1242[3])];
        let t1244: [uint32x4_t; 4] = [vorrq_u32(t1239[0], t1243[0]), vorrq_u32(t1239[1], t1243[1]), vorrq_u32(t1239[2], t1243[2]), vorrq_u32(t1239[3], t1243[3])];
        let t1245: u64 = x16;
        let t1246 = [vdupq_n_u32((t1245) as u32); 2];
        let t1247 = [vdupq_n_u32(((t1245) >> 32) as u32); 2];
        let t1248: [uint32x4_t; 4] = [vandq_u32(t1246[0], t1244[0]), vandq_u32(t1246[1], t1244[1]), vandq_u32(t1247[0], t1244[2]), vandq_u32(t1247[1], t1244[3])];
        x22 = t1248;
        let t1249: [uint32x4_t; 2] = [vsubq_u32(x13[0], x19[0]), vsubq_u32(x13[1], x19[1])];
        x23 = t1249;
        let t1250 = [vdupq_n_u32(8u32); 2];
        let t1251: [uint32x4_t; 2] = [vcgeq_u32(x23[0], t1250[0]), vcgeq_u32(x23[1], t1250[1])];
        let mut t1252: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t1253: [uint32x4_t; 2] = [vandq_u32(t1221[0], t1251[0]), vandq_u32(t1221[1], t1251[1])];
        let t1254: [uint32x4_t; 2] = [vbicq_u32(t1221[0], t1251[0]), vbicq_u32(t1221[1], t1251[1])];
        {
            t1252 = [vbslq_u32(t1253[0], x22[0], t1252[0]), vbslq_u32(t1253[1], x22[1], t1252[1]), vbslq_u32(t1253[0], x22[2], t1252[2]), vbslq_u32(t1253[1], x22[3], t1252[3])];
        }
        {
            let t1255: [uint32x4_t; 2] = [vmulq_u32(t1250[0], x23[0]), vmulq_u32(t1250[1], x23[1])];
            let t1256: u64 = 1u64;
            let t1257 = [vdupq_n_u32((t1256) as u32); 2];
            let t1258: [uint32x4_t; 2] = [t1255[0], t1255[1]];
            let t1259: [uint32x4_t; 2] = [vandq_u32(t1258[0], vdupq_n_u32(63)), vandq_u32(t1258[1], vdupq_n_u32(63))];
            let t1260: [uint32x4_t; 4] = [vshlq_u32(t1257[0], vreinterpretq_s32_u32(t1259[0])), vshlq_u32(t1257[1], vreinterpretq_s32_u32(t1259[1])), vshlq_u32(t1257[0], vsubq_s32(vreinterpretq_s32_u32(t1259[0]), vdupq_n_s32(32))), vshlq_u32(t1257[1], vsubq_s32(vreinterpretq_s32_u32(t1259[1]), vdupq_n_s32(32)))];
            let t1261: u64 = 1u64;
            let t1262 = [vdupq_n_u32((t1261) as u32); 2];
            let t1263: [uint32x4_t; 2] = [vcltq_u32(t1260[0], t1262[0]), vcltq_u32(t1260[1], t1262[1])];
            let t1264: [uint32x4_t; 4] = [vsubq_u32(t1260[0], t1262[0]), vsubq_u32(t1260[1], t1262[1]), vaddq_u32(vsubq_u32(t1260[2], vdupq_n_u32(0)), t1263[0]), vaddq_u32(vsubq_u32(t1260[3], vdupq_n_u32(0)), t1263[1])];
            let t1265: [uint32x4_t; 4] = [vandq_u32(x22[0], t1264[0]), vandq_u32(x22[1], t1264[1]), vandq_u32(x22[2], t1264[2]), vandq_u32(x22[3], t1264[3])];
            t1252 = [vbslq_u32(t1254[0], t1265[0], t1252[0]), vbslq_u32(t1254[1], t1265[1], t1252[1]), vbslq_u32(t1254[0], t1265[2], t1252[2]), vbslq_u32(t1254[1], t1265[3], t1252[3])];
        }
        let t1266 = b27_l_p(l, lid0, t1221, t1252);
        let t1267: [uint32x4_t; 2] = [vaddq_u32(x18[0], t1266[0]), vaddq_u32(x18[1], t1266[1])];
        x18 = [vbslq_u32(t1221[0], t1267[0], x18[0]), vbslq_u32(t1221[1], t1267[1], x18[1])];
        let t1268: [uint32x4_t; 2] = [vandq_u32(t1221[0], live[0]), vandq_u32(t1221[1], live[1])];
        let t1269: [uint32x4_t; 2] = [vaddq_u32(x19[0], t1250[0]), vaddq_u32(x19[1], t1250[1])];
        x19 = t1269;
    }
    x12 = x18;
    let t1270 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    x24 = (((t1270).wrapping_shr((((((4u32).wrapping_mul(x2)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32));
    let t1271 = [vdupq_n_u32(1u32); 2];
    let t1272: [uint32x4_t; 2] = [vsubq_u32(x10[0], t1271[0]), vsubq_u32(x10[1], t1271[1])];
    let t1273: [uint32x4_t; 2] = [vmulq_u32(t1205[0], t1272[0]), vmulq_u32(t1205[1], t1272[1])];
    let t1274: [uint32x4_t; 2] = [vaddq_u32(x7[0], t1273[0]), vaddq_u32(x7[1], t1273[1])];
    let t1275 = b26_ll_p(l, lid0, live, t1274, x8);
    let t1276 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x4, t1275, 2u32);
    let t1277: [uint32x4_t; 2] = [t1276[0], t1276[1]];
    x25 = t1277;
    let t1278: [uint32x4_t; 2] = [vaddq_u32(x14[0], x25[0]), vaddq_u32(x14[1], x25[1])];
    let t1279 = [vdupq_n_u32(x24); 2];
    let t1280: [uint32x4_t; 2] = [vmulq_u32(x12[0], t1279[0]), vmulq_u32(x12[1], t1279[1])];
    let t1281: [uint32x4_t; 2] = [vaddq_u32(t1278[0], t1280[0]), vaddq_u32(t1278[1], t1280[1])];
    let t1282 = b26_ll_p(l, lid0, live, t1281, x8);
    x26 = t1282;
    let t1283 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x4, x26, 8u32);
    let t1284 = b17_s_p(l, lid0, live, x24);
    let t1285: u64 = t1284;
    let t1286 = [vdupq_n_u32((t1285) as u32); 2];
    let t1287 = [vdupq_n_u32(((t1285) >> 32) as u32); 2];
    let t1288: [uint32x4_t; 4] = [vandq_u32(t1283[0], t1286[0]), vandq_u32(t1283[1], t1286[1]), vandq_u32(t1283[2], t1287[0]), vandq_u32(t1283[3], t1287[1])];
    res = t1288;
    return res;
}

#[inline(always)]
unsafe fn b20_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    res = (arg1.wrapping_add(((((x2).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned();
    return res;
}

#[inline(always)]
unsafe fn b20_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    res = (arg1.wrapping_add(((((x2).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned();
    return res;
}

#[inline(always)]
unsafe fn b21_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t1289 = b20_r0r1s_p(l, lid0, live, arg0, arg1, (20u32).wrapping_add((4u32).wrapping_mul(x2)));
    res = t1289;
    return res;
}

#[inline(always)]
unsafe fn b21_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t1290 = b20_r0r1s_f(l, lid0, arg0, arg1, (20u32).wrapping_add((4u32).wrapping_mul(x2)));
    res = t1290;
    return res;
}

#[inline(always)]
unsafe fn b22_ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x1: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t1291: u32;
    if ((x0) < (x1)) {
        t1291 = x0;
    } else {
        t1291 = x1;
    }
    res = t1291;
    return res;
}

#[inline(always)]
unsafe fn b22_ss_f(l: &Launch, lid0: u32, x0: u32, x1: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t1292: u32;
    if ((x0) < (x1)) {
        t1292 = x0;
    } else {
        t1292 = x1;
    }
    res = t1292;
    return res;
}

#[inline(always)]
unsafe fn b22_ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x1: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1293 = [vdupq_n_u32(x1); 2];
    let t1294: [uint32x4_t; 2] = [vcltq_u32(x0[0], t1293[0]), vcltq_u32(x0[1], t1293[1])];
    let mut t1295: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1296: [uint32x4_t; 2] = [vandq_u32(live[0], t1294[0]), vandq_u32(live[1], t1294[1])];
    let t1297: [uint32x4_t; 2] = [vbicq_u32(live[0], t1294[0]), vbicq_u32(live[1], t1294[1])];
    {
        t1295 = [vbslq_u32(t1296[0], x0[0], t1295[0]), vbslq_u32(t1296[1], x0[1], t1295[1])];
    }
    {
        let t1298 = [vdupq_n_u32(x1); 2];
        t1295 = [vbslq_u32(t1297[0], t1298[0], t1295[0]), vbslq_u32(t1297[1], t1298[1], t1295[1])];
    }
    res = t1295;
    return res;
}

#[inline(always)]
unsafe fn b22_ls_f(l: &Launch, lid0: u32, x0: [uint32x4_t; 2], x1: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1299 = [vdupq_n_u32(x1); 2];
    let t1300: [uint32x4_t; 2] = [vcltq_u32(x0[0], t1299[0]), vcltq_u32(x0[1], t1299[1])];
    let mut t1301: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1302: [uint32x4_t; 2] = [vmvnq_u32(t1300[0]), vmvnq_u32(t1300[1])];
    {
        t1301 = [vbslq_u32(t1300[0], x0[0], t1301[0]), vbslq_u32(t1300[1], x0[1], t1301[1])];
    }
    {
        let t1303 = [vdupq_n_u32(x1); 2];
        t1301 = [vbslq_u32(t1302[0], t1303[0], t1301[0]), vbslq_u32(t1302[1], t1303[1], t1301[1])];
    }
    res = t1301;
    return res;
}

#[inline(always)]
unsafe fn b23_r0sns_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1: u32, x2f: u32, x2s: u32, x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1304 = b28_r0an1s_p(l, lid0, live, arg0, ((((x1) as u64)) as usize as *const u8), x2f, x2s, x3);
    x4 = t1304;
    let t1305: [uint32x4_t; 4];
    if ((x3) >= (8u32)) {
        t1305 = x4;
    } else {
        let t1306 = b29_s_p(l, lid0, live, (8u32).wrapping_mul(x3));
        let t1307: u64 = t1306;
        let t1308 = [vdupq_n_u32((t1307) as u32); 2];
        let t1309 = [vdupq_n_u32(((t1307) >> 32) as u32); 2];
        let t1310: [uint32x4_t; 4] = [vandq_u32(x4[0], t1308[0]), vandq_u32(x4[1], t1308[1]), vandq_u32(x4[2], t1309[0]), vandq_u32(x4[3], t1309[1])];
        t1305 = t1310;
    }
    res = t1305;
    return res;
}

#[inline(always)]
unsafe fn b23_r0sns_f(l: &Launch, lid0: u32, arg0: *const u8, x1: u32, x2f: u32, x2s: u32, x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1311 = b28_r0an1s_f(l, lid0, arg0, ((((x1) as u64)) as usize as *const u8), x2f, x2s, x3);
    x4 = t1311;
    let t1312: [uint32x4_t; 4];
    if ((x3) >= (8u32)) {
        t1312 = x4;
    } else {
        let t1313 = b29_s_f(l, lid0, (8u32).wrapping_mul(x3));
        let t1314: u64 = t1313;
        let t1315 = [vdupq_n_u32((t1314) as u32); 2];
        let t1316 = [vdupq_n_u32(((t1314) >> 32) as u32); 2];
        let t1317: [uint32x4_t; 4] = [vandq_u32(x4[0], t1315[0]), vandq_u32(x4[1], t1315[1]), vandq_u32(x4[2], t1316[0]), vandq_u32(x4[3], t1316[1])];
        t1312 = t1317;
    }
    res = t1312;
    return res;
}

#[inline(always)]
unsafe fn b23_r0lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1: [uint32x4_t; 2], x2: [uint32x4_t; 2], x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1318: [uint32x4_t; 2] = [vaddq_u32(x1[0], x2[0]), vaddq_u32(x1[1], x2[1])];
    let t1319: [uint32x4_t; 2] = [vcltq_u32(t1318[0], x1[0]), vcltq_u32(t1318[1], x1[1])];
    let t1320: [uint32x4_t; 4] = [t1318[0], t1318[1], vshrq_n_u32::<31>(t1319[0]), vshrq_n_u32::<31>(t1319[1])];
    let t1321 = b28_r0ls_p(l, lid0, live, arg0, t1320, x3);
    x4 = t1321;
    let t1322: [uint32x4_t; 4];
    if ((x3) >= (8u32)) {
        t1322 = x4;
    } else {
        let t1323 = b29_s_p(l, lid0, live, (8u32).wrapping_mul(x3));
        let t1324: u64 = t1323;
        let t1325 = [vdupq_n_u32((t1324) as u32); 2];
        let t1326 = [vdupq_n_u32(((t1324) >> 32) as u32); 2];
        let t1327: [uint32x4_t; 4] = [vandq_u32(x4[0], t1325[0]), vandq_u32(x4[1], t1325[1]), vandq_u32(x4[2], t1326[0]), vandq_u32(x4[3], t1326[1])];
        t1322 = t1327;
    }
    res = t1322;
    return res;
}

#[inline(always)]
unsafe fn b23_r0lls_f(l: &Launch, lid0: u32, arg0: *const u8, x1: [uint32x4_t; 2], x2: [uint32x4_t; 2], x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1328: [uint32x4_t; 2] = [vaddq_u32(x1[0], x2[0]), vaddq_u32(x1[1], x2[1])];
    let t1329: [uint32x4_t; 2] = [vcltq_u32(t1328[0], x1[0]), vcltq_u32(t1328[1], x1[1])];
    let t1330: [uint32x4_t; 4] = [t1328[0], t1328[1], vshrq_n_u32::<31>(t1329[0]), vshrq_n_u32::<31>(t1329[1])];
    let t1331 = b28_r0ls_f(l, lid0, arg0, t1330, x3);
    x4 = t1331;
    let t1332: [uint32x4_t; 4];
    if ((x3) >= (8u32)) {
        t1332 = x4;
    } else {
        let t1333 = b29_s_f(l, lid0, (8u32).wrapping_mul(x3));
        let t1334: u64 = t1333;
        let t1335 = [vdupq_n_u32((t1334) as u32); 2];
        let t1336 = [vdupq_n_u32(((t1334) >> 32) as u32); 2];
        let t1337: [uint32x4_t; 4] = [vandq_u32(x4[0], t1335[0]), vandq_u32(x4[1], t1335[1]), vandq_u32(x4[2], t1336[0]), vandq_u32(x4[3], t1336[1])];
        t1332 = t1337;
    }
    res = t1332;
    return res;
}

#[inline(always)]
unsafe fn b24_r0r1sss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: u32, x4: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t1338 = b30_r0sss_p(l, lid0, live, arg0, x2, x3, x4);
    res = t1338;
    return res;
}

#[inline(always)]
unsafe fn b24_r0r1sss_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32, x3: u32, x4: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t1339 = b30_r0sss_f(l, lid0, arg0, x2, x3, x4);
    res = t1339;
    return res;
}

#[inline(always)]
unsafe fn b24_r0r1sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: [uint32x4_t; 2], x4: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1340 = b30_r0sls_p(l, lid0, live, arg0, x2, x3, x4);
    res = t1340;
    return res;
}

#[inline(always)]
unsafe fn b25_ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x1: u32) -> u32 {
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

#[inline(always)]
unsafe fn b25_ss_f(l: &Launch, lid0: u32, x0: u32, x1: u32) -> u32 {
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

#[inline(always)]
unsafe fn b25_sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x1: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1343 = [vdupq_n_u32(x0); 2];
    let t1344: [uint32x4_t; 2] = [vcltq_u32(t1343[0], x1[0]), vcltq_u32(t1343[1], x1[1])];
    let mut t1345: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1346: [uint32x4_t; 2] = [vandq_u32(live[0], t1344[0]), vandq_u32(live[1], t1344[1])];
    let t1347: [uint32x4_t; 2] = [vbicq_u32(live[0], t1344[0]), vbicq_u32(live[1], t1344[1])];
    {
        let t1348 = [vdupq_n_u32(x0); 2];
        t1345 = [vbslq_u32(t1346[0], t1348[0], t1345[0]), vbslq_u32(t1346[1], t1348[1], t1345[1])];
    }
    {
        t1345 = [vbslq_u32(t1347[0], x1[0], t1345[0]), vbslq_u32(t1347[1], x1[1], t1345[1])];
    }
    res = t1345;
    return res;
}

#[inline(always)]
unsafe fn b26_ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x1: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t1349: u32;
    if ((x0) < (x1)) {
        t1349 = x0;
    } else {
        t1349 = x1;
    }
    res = t1349;
    return res;
}

#[inline(always)]
unsafe fn b26_ss_f(l: &Launch, lid0: u32, x0: u32, x1: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t1350: u32;
    if ((x0) < (x1)) {
        t1350 = x0;
    } else {
        t1350 = x1;
    }
    res = t1350;
    return res;
}

#[inline(always)]
unsafe fn b26_ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x1: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1351 = [vdupq_n_u32(x1); 2];
    let t1352: [uint32x4_t; 2] = [vcltq_u32(x0[0], t1351[0]), vcltq_u32(x0[1], t1351[1])];
    let mut t1353: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1354: [uint32x4_t; 2] = [vandq_u32(live[0], t1352[0]), vandq_u32(live[1], t1352[1])];
    let t1355: [uint32x4_t; 2] = [vbicq_u32(live[0], t1352[0]), vbicq_u32(live[1], t1352[1])];
    {
        t1353 = [vbslq_u32(t1354[0], x0[0], t1353[0]), vbslq_u32(t1354[1], x0[1], t1353[1])];
    }
    {
        let t1356 = [vdupq_n_u32(x1); 2];
        t1353 = [vbslq_u32(t1355[0], t1356[0], t1353[0]), vbslq_u32(t1355[1], t1356[1], t1353[1])];
    }
    res = t1353;
    return res;
}

#[inline(always)]
unsafe fn b26_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x1: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1357: [uint32x4_t; 2] = [vcltq_u32(x0[0], x1[0]), vcltq_u32(x0[1], x1[1])];
    let mut t1358: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1359: [uint32x4_t; 2] = [vandq_u32(live[0], t1357[0]), vandq_u32(live[1], t1357[1])];
    let t1360: [uint32x4_t; 2] = [vbicq_u32(live[0], t1357[0]), vbicq_u32(live[1], t1357[1])];
    {
        t1358 = [vbslq_u32(t1359[0], x0[0], t1358[0]), vbslq_u32(t1359[1], x0[1], t1358[1])];
    }
    {
        t1358 = [vbslq_u32(t1360[0], x1[0], t1358[0]), vbslq_u32(t1360[1], x1[1], t1358[1])];
    }
    res = t1358;
    return res;
}

#[inline(always)]
unsafe fn b27_s_f(l: &Launch, lid0: u32, x0: u64) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    res = (((((x0) as u64).count_ones() as u64)) as u32);
    return res;
}

#[inline(always)]
unsafe fn b27_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1361: [uint32x4_t; 2] = [vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[0])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[2]))))), vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[1])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[3])))))];
    res = t1361;
    return res;
}

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
    let mut t1362: bool = ((x4) != (0u32));
    if t1362 {
        t1362 = (((x4).wrapping_add((x2).wrapping_mul(8u32))) > (64u32));
    }
    let t1363: u64;
    if t1362 {
        t1363 = ((x5) | (((x3.wrapping_add((1u64).wrapping_mul(8u64) as usize) as *const u64).read_unaligned()).wrapping_shl(((((64u32).wrapping_sub(x4)) as u64)) as u32)));
    } else {
        t1363 = x5;
    }
    res = t1363;
    return res;
}

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
    let mut t1364: bool = ((x4) != (0u32));
    if t1364 {
        t1364 = (((x4).wrapping_add((x2).wrapping_mul(8u32))) > (64u32));
    }
    let t1365: u64;
    if t1364 {
        t1365 = ((x5) | (((x3.wrapping_add((1u64).wrapping_mul(8u64) as usize) as *const u64).read_unaligned()).wrapping_shl(((((64u32).wrapping_sub(x4)) as u64)) as u32)));
    } else {
        t1365 = x5;
    }
    res = t1365;
    return res;
}

#[inline(always)]
unsafe fn b28_r0ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1: [uint32x4_t; 4], x2: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x3: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x4: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x5: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if ((x2) == (8u32)) {
        let t1366: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
        let t1367: [uint32x4_t; 4] = [vandq_u32(x1[0], t1366[0]), vandq_u32(x1[1], t1366[1]), vandq_u32(x1[2], t1366[2]), vandq_u32(x1[3], t1366[3])];
        let t1368: u64 = ((arg0) as usize as u64);
        let t1369 = [vdupq_n_u32((t1368) as u32); 2];
        let t1370 = [vdupq_n_u32(((t1368) >> 32) as u32); 2];
        let t1371: [uint32x4_t; 2] = [vaddq_u32(t1369[0], t1367[0]), vaddq_u32(t1369[1], t1367[1])];
        let t1372: [uint32x4_t; 2] = [vcltq_u32(t1371[0], t1369[0]), vcltq_u32(t1371[1], t1369[1])];
        let t1373: [uint32x4_t; 4] = [t1371[0], t1371[1], vsubq_u32(vaddq_u32(t1370[0], t1367[2]), t1372[0]), vsubq_u32(vaddq_u32(t1370[1], t1367[3]), t1372[1])];
        let mut t1374: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1373[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1373[0]) as u64) as usize as *const u64).read_unaligned(); t1374[0] = vsetq_lane_u32::<0>(v as u32, t1374[0]); t1374[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1374[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1373[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1373[0]) as u64) as usize as *const u64).read_unaligned(); t1374[0] = vsetq_lane_u32::<1>(v as u32, t1374[0]); t1374[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1374[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1373[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1373[0]) as u64) as usize as *const u64).read_unaligned(); t1374[0] = vsetq_lane_u32::<2>(v as u32, t1374[0]); t1374[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1374[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1373[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1373[0]) as u64) as usize as *const u64).read_unaligned(); t1374[0] = vsetq_lane_u32::<3>(v as u32, t1374[0]); t1374[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1374[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1373[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1373[1]) as u64) as usize as *const u64).read_unaligned(); t1374[1] = vsetq_lane_u32::<0>(v as u32, t1374[1]); t1374[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1374[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1373[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1373[1]) as u64) as usize as *const u64).read_unaligned(); t1374[1] = vsetq_lane_u32::<1>(v as u32, t1374[1]); t1374[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1374[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1373[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1373[1]) as u64) as usize as *const u64).read_unaligned(); t1374[1] = vsetq_lane_u32::<2>(v as u32, t1374[1]); t1374[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1374[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1373[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1373[1]) as u64) as usize as *const u64).read_unaligned(); t1374[1] = vsetq_lane_u32::<3>(v as u32, t1374[1]); t1374[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1374[3]); }
        res = t1374;
        return res;
    }
    if ((x2) == (4u32)) {
        let t1375: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
        let t1376: [uint32x4_t; 4] = [vandq_u32(x1[0], t1375[0]), vandq_u32(x1[1], t1375[1]), vandq_u32(x1[2], t1375[2]), vandq_u32(x1[3], t1375[3])];
        let t1377: u64 = ((arg0) as usize as u64);
        let t1378 = [vdupq_n_u32((t1377) as u32); 2];
        let t1379 = [vdupq_n_u32(((t1377) >> 32) as u32); 2];
        let t1380: [uint32x4_t; 2] = [vaddq_u32(t1378[0], t1376[0]), vaddq_u32(t1378[1], t1376[1])];
        let t1381: [uint32x4_t; 2] = [vcltq_u32(t1380[0], t1378[0]), vcltq_u32(t1380[1], t1378[1])];
        let t1382: [uint32x4_t; 4] = [t1380[0], t1380[1], vsubq_u32(vaddq_u32(t1379[0], t1376[2]), t1381[0]), vsubq_u32(vaddq_u32(t1379[1], t1376[3]), t1381[1])];
        let mut t1383: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t1383[0] = vsetq_lane_u32::<0>((((vgetq_lane_u32::<0>(t1382[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1382[0]) as u64) as usize as *const u32).read_unaligned() as u32, t1383[0]);
        t1383[0] = vsetq_lane_u32::<1>((((vgetq_lane_u32::<1>(t1382[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1382[0]) as u64) as usize as *const u32).read_unaligned() as u32, t1383[0]);
        t1383[0] = vsetq_lane_u32::<2>((((vgetq_lane_u32::<2>(t1382[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1382[0]) as u64) as usize as *const u32).read_unaligned() as u32, t1383[0]);
        t1383[0] = vsetq_lane_u32::<3>((((vgetq_lane_u32::<3>(t1382[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1382[0]) as u64) as usize as *const u32).read_unaligned() as u32, t1383[0]);
        t1383[1] = vsetq_lane_u32::<0>((((vgetq_lane_u32::<0>(t1382[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1382[1]) as u64) as usize as *const u32).read_unaligned() as u32, t1383[1]);
        t1383[1] = vsetq_lane_u32::<1>((((vgetq_lane_u32::<1>(t1382[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1382[1]) as u64) as usize as *const u32).read_unaligned() as u32, t1383[1]);
        t1383[1] = vsetq_lane_u32::<2>((((vgetq_lane_u32::<2>(t1382[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1382[1]) as u64) as usize as *const u32).read_unaligned() as u32, t1383[1]);
        t1383[1] = vsetq_lane_u32::<3>((((vgetq_lane_u32::<3>(t1382[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1382[1]) as u64) as usize as *const u32).read_unaligned() as u32, t1383[1]);
        let t1384: [uint32x4_t; 4] = [t1383[0], t1383[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1384;
        return res;
    }
    if ((x2) == (2u32)) {
        let t1385: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
        let t1386: [uint32x4_t; 4] = [vandq_u32(x1[0], t1385[0]), vandq_u32(x1[1], t1385[1]), vandq_u32(x1[2], t1385[2]), vandq_u32(x1[3], t1385[3])];
        let t1387: u64 = ((arg0) as usize as u64);
        let t1388 = [vdupq_n_u32((t1387) as u32); 2];
        let t1389 = [vdupq_n_u32(((t1387) >> 32) as u32); 2];
        let t1390: [uint32x4_t; 2] = [vaddq_u32(t1388[0], t1386[0]), vaddq_u32(t1388[1], t1386[1])];
        let t1391: [uint32x4_t; 2] = [vcltq_u32(t1390[0], t1388[0]), vcltq_u32(t1390[1], t1388[1])];
        let t1392: [uint32x4_t; 4] = [t1390[0], t1390[1], vsubq_u32(vaddq_u32(t1389[0], t1386[2]), t1391[0]), vsubq_u32(vaddq_u32(t1389[1], t1386[3]), t1391[1])];
        let mut t1393: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t1393[0] = vsetq_lane_u32::<0>((((vgetq_lane_u32::<0>(t1392[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1392[0]) as u64) as usize as *const u16).read_unaligned() as u32, t1393[0]);
        t1393[0] = vsetq_lane_u32::<1>((((vgetq_lane_u32::<1>(t1392[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1392[0]) as u64) as usize as *const u16).read_unaligned() as u32, t1393[0]);
        t1393[0] = vsetq_lane_u32::<2>((((vgetq_lane_u32::<2>(t1392[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1392[0]) as u64) as usize as *const u16).read_unaligned() as u32, t1393[0]);
        t1393[0] = vsetq_lane_u32::<3>((((vgetq_lane_u32::<3>(t1392[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1392[0]) as u64) as usize as *const u16).read_unaligned() as u32, t1393[0]);
        t1393[1] = vsetq_lane_u32::<0>((((vgetq_lane_u32::<0>(t1392[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1392[1]) as u64) as usize as *const u16).read_unaligned() as u32, t1393[1]);
        t1393[1] = vsetq_lane_u32::<1>((((vgetq_lane_u32::<1>(t1392[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1392[1]) as u64) as usize as *const u16).read_unaligned() as u32, t1393[1]);
        t1393[1] = vsetq_lane_u32::<2>((((vgetq_lane_u32::<2>(t1392[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1392[1]) as u64) as usize as *const u16).read_unaligned() as u32, t1393[1]);
        t1393[1] = vsetq_lane_u32::<3>((((vgetq_lane_u32::<3>(t1392[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1392[1]) as u64) as usize as *const u16).read_unaligned() as u32, t1393[1]);
        let t1394: [uint32x4_t; 4] = [t1393[0], t1393[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1394;
        return res;
    }
    if ((x2) == (1u32)) {
        let t1395: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
        let t1396: [uint32x4_t; 4] = [vandq_u32(x1[0], t1395[0]), vandq_u32(x1[1], t1395[1]), vandq_u32(x1[2], t1395[2]), vandq_u32(x1[3], t1395[3])];
        let t1397: u64 = ((arg0) as usize as u64);
        let t1398 = [vdupq_n_u32((t1397) as u32); 2];
        let t1399 = [vdupq_n_u32(((t1397) >> 32) as u32); 2];
        let t1400: [uint32x4_t; 2] = [vaddq_u32(t1398[0], t1396[0]), vaddq_u32(t1398[1], t1396[1])];
        let t1401: [uint32x4_t; 2] = [vcltq_u32(t1400[0], t1398[0]), vcltq_u32(t1400[1], t1398[1])];
        let t1402: [uint32x4_t; 4] = [t1400[0], t1400[1], vsubq_u32(vaddq_u32(t1399[0], t1396[2]), t1401[0]), vsubq_u32(vaddq_u32(t1399[1], t1396[3]), t1401[1])];
        let mut t1403: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t1403[0] = vsetq_lane_u32::<0>((((vgetq_lane_u32::<0>(t1402[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1402[0]) as u64) as usize as *const u8).read_unaligned() as u32, t1403[0]);
        t1403[0] = vsetq_lane_u32::<1>((((vgetq_lane_u32::<1>(t1402[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1402[0]) as u64) as usize as *const u8).read_unaligned() as u32, t1403[0]);
        t1403[0] = vsetq_lane_u32::<2>((((vgetq_lane_u32::<2>(t1402[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1402[0]) as u64) as usize as *const u8).read_unaligned() as u32, t1403[0]);
        t1403[0] = vsetq_lane_u32::<3>((((vgetq_lane_u32::<3>(t1402[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1402[0]) as u64) as usize as *const u8).read_unaligned() as u32, t1403[0]);
        t1403[1] = vsetq_lane_u32::<0>((((vgetq_lane_u32::<0>(t1402[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1402[1]) as u64) as usize as *const u8).read_unaligned() as u32, t1403[1]);
        t1403[1] = vsetq_lane_u32::<1>((((vgetq_lane_u32::<1>(t1402[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1402[1]) as u64) as usize as *const u8).read_unaligned() as u32, t1403[1]);
        t1403[1] = vsetq_lane_u32::<2>((((vgetq_lane_u32::<2>(t1402[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1402[1]) as u64) as usize as *const u8).read_unaligned() as u32, t1403[1]);
        t1403[1] = vsetq_lane_u32::<3>((((vgetq_lane_u32::<3>(t1402[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1402[1]) as u64) as usize as *const u8).read_unaligned() as u32, t1403[1]);
        let t1404: [uint32x4_t; 4] = [t1403[0], t1403[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1404;
        return res;
    }
    let t1405: u64 = !(7u64);
    let t1406 = [vdupq_n_u32((t1405) as u32); 2];
    let t1407 = [vdupq_n_u32(((t1405) >> 32) as u32); 2];
    let t1408: [uint32x4_t; 4] = [vandq_u32(x1[0], t1406[0]), vandq_u32(x1[1], t1406[1]), vandq_u32(x1[2], t1407[0]), vandq_u32(x1[3], t1407[1])];
    let t1409: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
    let t1410: [uint32x4_t; 4] = [vandq_u32(t1408[0], t1409[0]), vandq_u32(t1408[1], t1409[1]), vandq_u32(t1408[2], t1409[2]), vandq_u32(t1408[3], t1409[3])];
    let t1411: u64 = ((arg0) as usize as u64);
    let t1412 = [vdupq_n_u32((t1411) as u32); 2];
    let t1413 = [vdupq_n_u32(((t1411) >> 32) as u32); 2];
    let t1414: [uint32x4_t; 2] = [vaddq_u32(t1412[0], t1410[0]), vaddq_u32(t1412[1], t1410[1])];
    let t1415: [uint32x4_t; 2] = [vcltq_u32(t1414[0], t1412[0]), vcltq_u32(t1414[1], t1412[1])];
    let t1416: [uint32x4_t; 4] = [t1414[0], t1414[1], vsubq_u32(vaddq_u32(t1413[0], t1410[2]), t1415[0]), vsubq_u32(vaddq_u32(t1413[1], t1410[3]), t1415[1])];
    x3 = t1416;
    let t1417: u64 = 7u64;
    let t1418 = [vdupq_n_u32((t1417) as u32); 2];
    let t1419: [uint32x4_t; 2] = [vandq_u32(x1[0], t1418[0]), vandq_u32(x1[1], t1418[1])];
    let t1420 = [vdupq_n_u32(8u32); 2];
    let t1421: [uint32x4_t; 2] = [vmulq_u32(t1419[0], t1420[0]), vmulq_u32(t1419[1], t1420[1])];
    x4 = t1421;
    let mut t1422: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x3[2]) as u64) << 32 | vgetq_lane_u32::<0>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1422[0] = vsetq_lane_u32::<0>(v as u32, t1422[0]); t1422[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1422[2]); }
    { let v = (((vgetq_lane_u32::<1>(x3[2]) as u64) << 32 | vgetq_lane_u32::<1>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1422[0] = vsetq_lane_u32::<1>(v as u32, t1422[0]); t1422[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1422[2]); }
    { let v = (((vgetq_lane_u32::<2>(x3[2]) as u64) << 32 | vgetq_lane_u32::<2>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1422[0] = vsetq_lane_u32::<2>(v as u32, t1422[0]); t1422[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1422[2]); }
    { let v = (((vgetq_lane_u32::<3>(x3[2]) as u64) << 32 | vgetq_lane_u32::<3>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1422[0] = vsetq_lane_u32::<3>(v as u32, t1422[0]); t1422[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1422[2]); }
    { let v = (((vgetq_lane_u32::<0>(x3[3]) as u64) << 32 | vgetq_lane_u32::<0>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1422[1] = vsetq_lane_u32::<0>(v as u32, t1422[1]); t1422[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1422[3]); }
    { let v = (((vgetq_lane_u32::<1>(x3[3]) as u64) << 32 | vgetq_lane_u32::<1>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1422[1] = vsetq_lane_u32::<1>(v as u32, t1422[1]); t1422[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1422[3]); }
    { let v = (((vgetq_lane_u32::<2>(x3[3]) as u64) << 32 | vgetq_lane_u32::<2>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1422[1] = vsetq_lane_u32::<2>(v as u32, t1422[1]); t1422[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1422[3]); }
    { let v = (((vgetq_lane_u32::<3>(x3[3]) as u64) << 32 | vgetq_lane_u32::<3>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1422[1] = vsetq_lane_u32::<3>(v as u32, t1422[1]); t1422[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1422[3]); }
    let t1423: [uint32x4_t; 2] = [x4[0], x4[1]];
    let t1424: [uint32x4_t; 2] = [vandq_u32(t1423[0], vdupq_n_u32(63)), vandq_u32(t1423[1], vdupq_n_u32(63))];
    let t1425: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t1422[0], vnegq_s32(vreinterpretq_s32_u32(t1424[0]))), vshlq_u32(t1422[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1424[0])))), vorrq_u32(vshlq_u32(t1422[1], vnegq_s32(vreinterpretq_s32_u32(t1424[1]))), vshlq_u32(t1422[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1424[1])))), vshlq_u32(t1422[2], vnegq_s32(vreinterpretq_s32_u32(t1424[0]))), vshlq_u32(t1422[3], vnegq_s32(vreinterpretq_s32_u32(t1424[1])))];
    x5 = t1425;
    let t1426 = [vdupq_n_u32(0u32); 2];
    let t1427: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t1426[0])), vmvnq_u32(vceqq_u32(x4[1], t1426[1]))];
    let t1428 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
    let t1429: [uint32x4_t; 2] = [vaddq_u32(x4[0], t1428[0]), vaddq_u32(x4[1], t1428[1])];
    let t1430 = [vdupq_n_u32(64u32); 2];
    let t1431: [uint32x4_t; 2] = [vcgtq_u32(t1429[0], t1430[0]), vcgtq_u32(t1429[1], t1430[1])];
    let t1432: [uint32x4_t; 2] = [vandq_u32(t1427[0], t1431[0]), vandq_u32(t1427[1], t1431[1])];
    let mut t1433: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1434: [uint32x4_t; 2] = [vandq_u32(live[0], t1432[0]), vandq_u32(live[1], t1432[1])];
    let t1435: [uint32x4_t; 2] = [vbicq_u32(live[0], t1432[0]), vbicq_u32(live[1], t1432[1])];
    if any(&t1434) {
        let t1436: u64 = (1u64).wrapping_mul(8u64);
        let t1437 = [vdupq_n_u32((t1436) as u32); 2];
        let t1438 = [vdupq_n_u32(((t1436) >> 32) as u32); 2];
        let t1439: [uint32x4_t; 2] = [vaddq_u32(x3[0], t1437[0]), vaddq_u32(x3[1], t1437[1])];
        let t1440: [uint32x4_t; 2] = [vcltq_u32(t1439[0], x3[0]), vcltq_u32(t1439[1], x3[1])];
        let t1441: [uint32x4_t; 4] = [t1439[0], t1439[1], vsubq_u32(vaddq_u32(x3[2], t1438[0]), t1440[0]), vsubq_u32(vaddq_u32(x3[3], t1438[1]), t1440[1])];
        let mut t1442: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1441[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1441[0]) as u64) as usize as *const u64).read_unaligned(); t1442[0] = vsetq_lane_u32::<0>(v as u32, t1442[0]); t1442[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1442[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1441[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1441[0]) as u64) as usize as *const u64).read_unaligned(); t1442[0] = vsetq_lane_u32::<1>(v as u32, t1442[0]); t1442[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1442[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1441[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1441[0]) as u64) as usize as *const u64).read_unaligned(); t1442[0] = vsetq_lane_u32::<2>(v as u32, t1442[0]); t1442[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1442[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1441[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1441[0]) as u64) as usize as *const u64).read_unaligned(); t1442[0] = vsetq_lane_u32::<3>(v as u32, t1442[0]); t1442[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1442[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1441[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1441[1]) as u64) as usize as *const u64).read_unaligned(); t1442[1] = vsetq_lane_u32::<0>(v as u32, t1442[1]); t1442[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1442[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1441[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1441[1]) as u64) as usize as *const u64).read_unaligned(); t1442[1] = vsetq_lane_u32::<1>(v as u32, t1442[1]); t1442[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1442[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1441[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1441[1]) as u64) as usize as *const u64).read_unaligned(); t1442[1] = vsetq_lane_u32::<2>(v as u32, t1442[1]); t1442[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1442[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1441[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1441[1]) as u64) as usize as *const u64).read_unaligned(); t1442[1] = vsetq_lane_u32::<3>(v as u32, t1442[1]); t1442[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1442[3]); }
        let t1443: [uint32x4_t; 2] = [vsubq_u32(t1430[0], x4[0]), vsubq_u32(t1430[1], x4[1])];
        let t1444: [uint32x4_t; 2] = [t1443[0], t1443[1]];
        let t1445: [uint32x4_t; 2] = [vandq_u32(t1444[0], vdupq_n_u32(63)), vandq_u32(t1444[1], vdupq_n_u32(63))];
        let t1446: [uint32x4_t; 4] = [vshlq_u32(t1442[0], vreinterpretq_s32_u32(t1445[0])), vshlq_u32(t1442[1], vreinterpretq_s32_u32(t1445[1])), vorrq_u32(vshlq_u32(t1442[2], vreinterpretq_s32_u32(t1445[0])), vshlq_u32(t1442[0], vsubq_s32(vreinterpretq_s32_u32(t1445[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1442[3], vreinterpretq_s32_u32(t1445[1])), vshlq_u32(t1442[1], vsubq_s32(vreinterpretq_s32_u32(t1445[1]), vdupq_n_s32(32))))];
        let t1447: [uint32x4_t; 4] = [vorrq_u32(x5[0], t1446[0]), vorrq_u32(x5[1], t1446[1]), vorrq_u32(x5[2], t1446[2]), vorrq_u32(x5[3], t1446[3])];
        t1433 = [vbslq_u32(t1434[0], t1447[0], t1433[0]), vbslq_u32(t1434[1], t1447[1], t1433[1]), vbslq_u32(t1434[0], t1447[2], t1433[2]), vbslq_u32(t1434[1], t1447[3], t1433[3])];
    }
    {
        t1433 = [vbslq_u32(t1435[0], x5[0], t1433[0]), vbslq_u32(t1435[1], x5[1], t1433[1]), vbslq_u32(t1435[0], x5[2], t1433[2]), vbslq_u32(t1435[1], x5[3], t1433[3])];
    }
    res = t1433;
    return res;
}

#[inline(always)]
unsafe fn b28_r0ls_f(l: &Launch, lid0: u32, arg0: *const u8, x1: [uint32x4_t; 4], x2: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x3: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x4: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x5: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if ((x2) == (8u32)) {
        let t1448: u64 = ((arg0) as usize as u64);
        let t1449 = [vdupq_n_u32((t1448) as u32); 2];
        let t1450 = [vdupq_n_u32(((t1448) >> 32) as u32); 2];
        let t1451: [uint32x4_t; 2] = [vaddq_u32(t1449[0], x1[0]), vaddq_u32(t1449[1], x1[1])];
        let t1452: [uint32x4_t; 2] = [vcltq_u32(t1451[0], t1449[0]), vcltq_u32(t1451[1], t1449[1])];
        let t1453: [uint32x4_t; 4] = [t1451[0], t1451[1], vsubq_u32(vaddq_u32(t1450[0], x1[2]), t1452[0]), vsubq_u32(vaddq_u32(t1450[1], x1[3]), t1452[1])];
        let mut t1454: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1453[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1453[0]) as u64) as usize as *const u64).read_unaligned(); t1454[0] = vsetq_lane_u32::<0>(v as u32, t1454[0]); t1454[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1454[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1453[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1453[0]) as u64) as usize as *const u64).read_unaligned(); t1454[0] = vsetq_lane_u32::<1>(v as u32, t1454[0]); t1454[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1454[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1453[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1453[0]) as u64) as usize as *const u64).read_unaligned(); t1454[0] = vsetq_lane_u32::<2>(v as u32, t1454[0]); t1454[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1454[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1453[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1453[0]) as u64) as usize as *const u64).read_unaligned(); t1454[0] = vsetq_lane_u32::<3>(v as u32, t1454[0]); t1454[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1454[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1453[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1453[1]) as u64) as usize as *const u64).read_unaligned(); t1454[1] = vsetq_lane_u32::<0>(v as u32, t1454[1]); t1454[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1454[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1453[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1453[1]) as u64) as usize as *const u64).read_unaligned(); t1454[1] = vsetq_lane_u32::<1>(v as u32, t1454[1]); t1454[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1454[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1453[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1453[1]) as u64) as usize as *const u64).read_unaligned(); t1454[1] = vsetq_lane_u32::<2>(v as u32, t1454[1]); t1454[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1454[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1453[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1453[1]) as u64) as usize as *const u64).read_unaligned(); t1454[1] = vsetq_lane_u32::<3>(v as u32, t1454[1]); t1454[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1454[3]); }
        res = t1454;
        return res;
    }
    if ((x2) == (4u32)) {
        let t1455: u64 = ((arg0) as usize as u64);
        let t1456 = [vdupq_n_u32((t1455) as u32); 2];
        let t1457 = [vdupq_n_u32(((t1455) >> 32) as u32); 2];
        let t1458: [uint32x4_t; 2] = [vaddq_u32(t1456[0], x1[0]), vaddq_u32(t1456[1], x1[1])];
        let t1459: [uint32x4_t; 2] = [vcltq_u32(t1458[0], t1456[0]), vcltq_u32(t1458[1], t1456[1])];
        let t1460: [uint32x4_t; 4] = [t1458[0], t1458[1], vsubq_u32(vaddq_u32(t1457[0], x1[2]), t1459[0]), vsubq_u32(vaddq_u32(t1457[1], x1[3]), t1459[1])];
        let mut t1461: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t1461[0] = vsetq_lane_u32::<0>((((vgetq_lane_u32::<0>(t1460[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1460[0]) as u64) as usize as *const u32).read_unaligned() as u32, t1461[0]);
        t1461[0] = vsetq_lane_u32::<1>((((vgetq_lane_u32::<1>(t1460[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1460[0]) as u64) as usize as *const u32).read_unaligned() as u32, t1461[0]);
        t1461[0] = vsetq_lane_u32::<2>((((vgetq_lane_u32::<2>(t1460[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1460[0]) as u64) as usize as *const u32).read_unaligned() as u32, t1461[0]);
        t1461[0] = vsetq_lane_u32::<3>((((vgetq_lane_u32::<3>(t1460[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1460[0]) as u64) as usize as *const u32).read_unaligned() as u32, t1461[0]);
        t1461[1] = vsetq_lane_u32::<0>((((vgetq_lane_u32::<0>(t1460[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1460[1]) as u64) as usize as *const u32).read_unaligned() as u32, t1461[1]);
        t1461[1] = vsetq_lane_u32::<1>((((vgetq_lane_u32::<1>(t1460[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1460[1]) as u64) as usize as *const u32).read_unaligned() as u32, t1461[1]);
        t1461[1] = vsetq_lane_u32::<2>((((vgetq_lane_u32::<2>(t1460[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1460[1]) as u64) as usize as *const u32).read_unaligned() as u32, t1461[1]);
        t1461[1] = vsetq_lane_u32::<3>((((vgetq_lane_u32::<3>(t1460[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1460[1]) as u64) as usize as *const u32).read_unaligned() as u32, t1461[1]);
        let t1462: [uint32x4_t; 4] = [t1461[0], t1461[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1462;
        return res;
    }
    if ((x2) == (2u32)) {
        let t1463: u64 = ((arg0) as usize as u64);
        let t1464 = [vdupq_n_u32((t1463) as u32); 2];
        let t1465 = [vdupq_n_u32(((t1463) >> 32) as u32); 2];
        let t1466: [uint32x4_t; 2] = [vaddq_u32(t1464[0], x1[0]), vaddq_u32(t1464[1], x1[1])];
        let t1467: [uint32x4_t; 2] = [vcltq_u32(t1466[0], t1464[0]), vcltq_u32(t1466[1], t1464[1])];
        let t1468: [uint32x4_t; 4] = [t1466[0], t1466[1], vsubq_u32(vaddq_u32(t1465[0], x1[2]), t1467[0]), vsubq_u32(vaddq_u32(t1465[1], x1[3]), t1467[1])];
        let mut t1469: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t1469[0] = vsetq_lane_u32::<0>((((vgetq_lane_u32::<0>(t1468[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1468[0]) as u64) as usize as *const u16).read_unaligned() as u32, t1469[0]);
        t1469[0] = vsetq_lane_u32::<1>((((vgetq_lane_u32::<1>(t1468[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1468[0]) as u64) as usize as *const u16).read_unaligned() as u32, t1469[0]);
        t1469[0] = vsetq_lane_u32::<2>((((vgetq_lane_u32::<2>(t1468[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1468[0]) as u64) as usize as *const u16).read_unaligned() as u32, t1469[0]);
        t1469[0] = vsetq_lane_u32::<3>((((vgetq_lane_u32::<3>(t1468[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1468[0]) as u64) as usize as *const u16).read_unaligned() as u32, t1469[0]);
        t1469[1] = vsetq_lane_u32::<0>((((vgetq_lane_u32::<0>(t1468[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1468[1]) as u64) as usize as *const u16).read_unaligned() as u32, t1469[1]);
        t1469[1] = vsetq_lane_u32::<1>((((vgetq_lane_u32::<1>(t1468[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1468[1]) as u64) as usize as *const u16).read_unaligned() as u32, t1469[1]);
        t1469[1] = vsetq_lane_u32::<2>((((vgetq_lane_u32::<2>(t1468[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1468[1]) as u64) as usize as *const u16).read_unaligned() as u32, t1469[1]);
        t1469[1] = vsetq_lane_u32::<3>((((vgetq_lane_u32::<3>(t1468[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1468[1]) as u64) as usize as *const u16).read_unaligned() as u32, t1469[1]);
        let t1470: [uint32x4_t; 4] = [t1469[0], t1469[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1470;
        return res;
    }
    if ((x2) == (1u32)) {
        let t1471: u64 = ((arg0) as usize as u64);
        let t1472 = [vdupq_n_u32((t1471) as u32); 2];
        let t1473 = [vdupq_n_u32(((t1471) >> 32) as u32); 2];
        let t1474: [uint32x4_t; 2] = [vaddq_u32(t1472[0], x1[0]), vaddq_u32(t1472[1], x1[1])];
        let t1475: [uint32x4_t; 2] = [vcltq_u32(t1474[0], t1472[0]), vcltq_u32(t1474[1], t1472[1])];
        let t1476: [uint32x4_t; 4] = [t1474[0], t1474[1], vsubq_u32(vaddq_u32(t1473[0], x1[2]), t1475[0]), vsubq_u32(vaddq_u32(t1473[1], x1[3]), t1475[1])];
        let mut t1477: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t1477[0] = vsetq_lane_u32::<0>((((vgetq_lane_u32::<0>(t1476[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1476[0]) as u64) as usize as *const u8).read_unaligned() as u32, t1477[0]);
        t1477[0] = vsetq_lane_u32::<1>((((vgetq_lane_u32::<1>(t1476[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1476[0]) as u64) as usize as *const u8).read_unaligned() as u32, t1477[0]);
        t1477[0] = vsetq_lane_u32::<2>((((vgetq_lane_u32::<2>(t1476[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1476[0]) as u64) as usize as *const u8).read_unaligned() as u32, t1477[0]);
        t1477[0] = vsetq_lane_u32::<3>((((vgetq_lane_u32::<3>(t1476[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1476[0]) as u64) as usize as *const u8).read_unaligned() as u32, t1477[0]);
        t1477[1] = vsetq_lane_u32::<0>((((vgetq_lane_u32::<0>(t1476[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1476[1]) as u64) as usize as *const u8).read_unaligned() as u32, t1477[1]);
        t1477[1] = vsetq_lane_u32::<1>((((vgetq_lane_u32::<1>(t1476[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1476[1]) as u64) as usize as *const u8).read_unaligned() as u32, t1477[1]);
        t1477[1] = vsetq_lane_u32::<2>((((vgetq_lane_u32::<2>(t1476[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1476[1]) as u64) as usize as *const u8).read_unaligned() as u32, t1477[1]);
        t1477[1] = vsetq_lane_u32::<3>((((vgetq_lane_u32::<3>(t1476[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1476[1]) as u64) as usize as *const u8).read_unaligned() as u32, t1477[1]);
        let t1478: [uint32x4_t; 4] = [t1477[0], t1477[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1478;
        return res;
    }
    let t1479: u64 = !(7u64);
    let t1480 = [vdupq_n_u32((t1479) as u32); 2];
    let t1481 = [vdupq_n_u32(((t1479) >> 32) as u32); 2];
    let t1482: [uint32x4_t; 4] = [vandq_u32(x1[0], t1480[0]), vandq_u32(x1[1], t1480[1]), vandq_u32(x1[2], t1481[0]), vandq_u32(x1[3], t1481[1])];
    let t1483: u64 = ((arg0) as usize as u64);
    let t1484 = [vdupq_n_u32((t1483) as u32); 2];
    let t1485 = [vdupq_n_u32(((t1483) >> 32) as u32); 2];
    let t1486: [uint32x4_t; 2] = [vaddq_u32(t1484[0], t1482[0]), vaddq_u32(t1484[1], t1482[1])];
    let t1487: [uint32x4_t; 2] = [vcltq_u32(t1486[0], t1484[0]), vcltq_u32(t1486[1], t1484[1])];
    let t1488: [uint32x4_t; 4] = [t1486[0], t1486[1], vsubq_u32(vaddq_u32(t1485[0], t1482[2]), t1487[0]), vsubq_u32(vaddq_u32(t1485[1], t1482[3]), t1487[1])];
    x3 = t1488;
    let t1489: u64 = 7u64;
    let t1490 = [vdupq_n_u32((t1489) as u32); 2];
    let t1491: [uint32x4_t; 2] = [vandq_u32(x1[0], t1490[0]), vandq_u32(x1[1], t1490[1])];
    let t1492 = [vdupq_n_u32(8u32); 2];
    let t1493: [uint32x4_t; 2] = [vmulq_u32(t1491[0], t1492[0]), vmulq_u32(t1491[1], t1492[1])];
    x4 = t1493;
    let mut t1494: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x3[2]) as u64) << 32 | vgetq_lane_u32::<0>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1494[0] = vsetq_lane_u32::<0>(v as u32, t1494[0]); t1494[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1494[2]); }
    { let v = (((vgetq_lane_u32::<1>(x3[2]) as u64) << 32 | vgetq_lane_u32::<1>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1494[0] = vsetq_lane_u32::<1>(v as u32, t1494[0]); t1494[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1494[2]); }
    { let v = (((vgetq_lane_u32::<2>(x3[2]) as u64) << 32 | vgetq_lane_u32::<2>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1494[0] = vsetq_lane_u32::<2>(v as u32, t1494[0]); t1494[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1494[2]); }
    { let v = (((vgetq_lane_u32::<3>(x3[2]) as u64) << 32 | vgetq_lane_u32::<3>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1494[0] = vsetq_lane_u32::<3>(v as u32, t1494[0]); t1494[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1494[2]); }
    { let v = (((vgetq_lane_u32::<0>(x3[3]) as u64) << 32 | vgetq_lane_u32::<0>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1494[1] = vsetq_lane_u32::<0>(v as u32, t1494[1]); t1494[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1494[3]); }
    { let v = (((vgetq_lane_u32::<1>(x3[3]) as u64) << 32 | vgetq_lane_u32::<1>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1494[1] = vsetq_lane_u32::<1>(v as u32, t1494[1]); t1494[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1494[3]); }
    { let v = (((vgetq_lane_u32::<2>(x3[3]) as u64) << 32 | vgetq_lane_u32::<2>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1494[1] = vsetq_lane_u32::<2>(v as u32, t1494[1]); t1494[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1494[3]); }
    { let v = (((vgetq_lane_u32::<3>(x3[3]) as u64) << 32 | vgetq_lane_u32::<3>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1494[1] = vsetq_lane_u32::<3>(v as u32, t1494[1]); t1494[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1494[3]); }
    let t1495: [uint32x4_t; 2] = [x4[0], x4[1]];
    let t1496: [uint32x4_t; 2] = [vandq_u32(t1495[0], vdupq_n_u32(63)), vandq_u32(t1495[1], vdupq_n_u32(63))];
    let t1497: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t1494[0], vnegq_s32(vreinterpretq_s32_u32(t1496[0]))), vshlq_u32(t1494[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1496[0])))), vorrq_u32(vshlq_u32(t1494[1], vnegq_s32(vreinterpretq_s32_u32(t1496[1]))), vshlq_u32(t1494[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1496[1])))), vshlq_u32(t1494[2], vnegq_s32(vreinterpretq_s32_u32(t1496[0]))), vshlq_u32(t1494[3], vnegq_s32(vreinterpretq_s32_u32(t1496[1])))];
    x5 = t1497;
    let t1498 = [vdupq_n_u32(0u32); 2];
    let t1499: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t1498[0])), vmvnq_u32(vceqq_u32(x4[1], t1498[1]))];
    let t1500 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
    let t1501: [uint32x4_t; 2] = [vaddq_u32(x4[0], t1500[0]), vaddq_u32(x4[1], t1500[1])];
    let t1502 = [vdupq_n_u32(64u32); 2];
    let t1503: [uint32x4_t; 2] = [vcgtq_u32(t1501[0], t1502[0]), vcgtq_u32(t1501[1], t1502[1])];
    let t1504: [uint32x4_t; 2] = [vandq_u32(t1499[0], t1503[0]), vandq_u32(t1499[1], t1503[1])];
    let mut t1505: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1506: [uint32x4_t; 2] = [vmvnq_u32(t1504[0]), vmvnq_u32(t1504[1])];
    if any(&t1504) {
        let t1507: u64 = (1u64).wrapping_mul(8u64);
        let t1508 = [vdupq_n_u32((t1507) as u32); 2];
        let t1509 = [vdupq_n_u32(((t1507) >> 32) as u32); 2];
        let t1510: [uint32x4_t; 2] = [vaddq_u32(x3[0], t1508[0]), vaddq_u32(x3[1], t1508[1])];
        let t1511: [uint32x4_t; 2] = [vcltq_u32(t1510[0], x3[0]), vcltq_u32(t1510[1], x3[1])];
        let t1512: [uint32x4_t; 4] = [t1510[0], t1510[1], vsubq_u32(vaddq_u32(x3[2], t1509[0]), t1511[0]), vsubq_u32(vaddq_u32(x3[3], t1509[1]), t1511[1])];
        let mut t1513: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1512[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1512[0]) as u64) as usize as *const u64).read_unaligned(); t1513[0] = vsetq_lane_u32::<0>(v as u32, t1513[0]); t1513[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1513[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1512[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1512[0]) as u64) as usize as *const u64).read_unaligned(); t1513[0] = vsetq_lane_u32::<1>(v as u32, t1513[0]); t1513[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1513[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1512[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1512[0]) as u64) as usize as *const u64).read_unaligned(); t1513[0] = vsetq_lane_u32::<2>(v as u32, t1513[0]); t1513[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1513[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1512[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1512[0]) as u64) as usize as *const u64).read_unaligned(); t1513[0] = vsetq_lane_u32::<3>(v as u32, t1513[0]); t1513[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1513[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1512[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1512[1]) as u64) as usize as *const u64).read_unaligned(); t1513[1] = vsetq_lane_u32::<0>(v as u32, t1513[1]); t1513[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1513[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1512[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1512[1]) as u64) as usize as *const u64).read_unaligned(); t1513[1] = vsetq_lane_u32::<1>(v as u32, t1513[1]); t1513[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1513[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1512[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1512[1]) as u64) as usize as *const u64).read_unaligned(); t1513[1] = vsetq_lane_u32::<2>(v as u32, t1513[1]); t1513[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1513[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1512[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1512[1]) as u64) as usize as *const u64).read_unaligned(); t1513[1] = vsetq_lane_u32::<3>(v as u32, t1513[1]); t1513[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1513[3]); }
        let t1514: [uint32x4_t; 2] = [vsubq_u32(t1502[0], x4[0]), vsubq_u32(t1502[1], x4[1])];
        let t1515: [uint32x4_t; 2] = [t1514[0], t1514[1]];
        let t1516: [uint32x4_t; 2] = [vandq_u32(t1515[0], vdupq_n_u32(63)), vandq_u32(t1515[1], vdupq_n_u32(63))];
        let t1517: [uint32x4_t; 4] = [vshlq_u32(t1513[0], vreinterpretq_s32_u32(t1516[0])), vshlq_u32(t1513[1], vreinterpretq_s32_u32(t1516[1])), vorrq_u32(vshlq_u32(t1513[2], vreinterpretq_s32_u32(t1516[0])), vshlq_u32(t1513[0], vsubq_s32(vreinterpretq_s32_u32(t1516[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1513[3], vreinterpretq_s32_u32(t1516[1])), vshlq_u32(t1513[1], vsubq_s32(vreinterpretq_s32_u32(t1516[1]), vdupq_n_s32(32))))];
        let t1518: [uint32x4_t; 4] = [vorrq_u32(x5[0], t1517[0]), vorrq_u32(x5[1], t1517[1]), vorrq_u32(x5[2], t1517[2]), vorrq_u32(x5[3], t1517[3])];
        t1505 = [vbslq_u32(t1504[0], t1518[0], t1505[0]), vbslq_u32(t1504[1], t1518[1], t1505[1]), vbslq_u32(t1504[0], t1518[2], t1505[2]), vbslq_u32(t1504[1], t1518[3], t1505[3])];
    }
    {
        t1505 = [vbslq_u32(t1506[0], x5[0], t1505[0]), vbslq_u32(t1506[1], x5[1], t1505[1]), vbslq_u32(t1506[0], x5[2], t1505[2]), vbslq_u32(t1506[1], x5[3], t1505[3])];
    }
    res = t1505;
    return res;
}

#[inline(always)]
unsafe fn b28_r0al1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1b: *const u8, x1i: [uint32x4_t; 2], x2: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x3: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x4: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x5: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if ((x2) == (8u32)) {
        let t1519 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], x1i[0]), vandq_u32(live[1], x1i[1]))));
        let t1520: [uint32x4_t; 2] = [vbslq_u32(live[0], x1i[0], t1519), vbslq_u32(live[1], x1i[1], t1519)];
        let mut t1521: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1520[0]) as usize * 1) as *const u64).read_unaligned(); t1521[0] = vsetq_lane_u32::<0>(v as u32, t1521[0]); t1521[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1521[2]); }
        { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1520[0]) as usize * 1) as *const u64).read_unaligned(); t1521[0] = vsetq_lane_u32::<1>(v as u32, t1521[0]); t1521[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1521[2]); }
        { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1520[0]) as usize * 1) as *const u64).read_unaligned(); t1521[0] = vsetq_lane_u32::<2>(v as u32, t1521[0]); t1521[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1521[2]); }
        { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1520[0]) as usize * 1) as *const u64).read_unaligned(); t1521[0] = vsetq_lane_u32::<3>(v as u32, t1521[0]); t1521[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1521[2]); }
        { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1520[1]) as usize * 1) as *const u64).read_unaligned(); t1521[1] = vsetq_lane_u32::<0>(v as u32, t1521[1]); t1521[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1521[3]); }
        { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1520[1]) as usize * 1) as *const u64).read_unaligned(); t1521[1] = vsetq_lane_u32::<1>(v as u32, t1521[1]); t1521[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1521[3]); }
        { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1520[1]) as usize * 1) as *const u64).read_unaligned(); t1521[1] = vsetq_lane_u32::<2>(v as u32, t1521[1]); t1521[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1521[3]); }
        { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1520[1]) as usize * 1) as *const u64).read_unaligned(); t1521[1] = vsetq_lane_u32::<3>(v as u32, t1521[1]); t1521[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1521[3]); }
        res = t1521;
        return res;
    }
    if ((x2) == (4u32)) {
        let t1522 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], x1i[0]), vandq_u32(live[1], x1i[1]))));
        let t1523: [uint32x4_t; 2] = [vbslq_u32(live[0], x1i[0], t1522), vbslq_u32(live[1], x1i[1], t1522)];
        let mut t1524: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t1524[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1523[0]) as usize * 1) as *const u32).read_unaligned() as u32), t1524[0]);
        t1524[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1523[0]) as usize * 1) as *const u32).read_unaligned() as u32), t1524[0]);
        t1524[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1523[0]) as usize * 1) as *const u32).read_unaligned() as u32), t1524[0]);
        t1524[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1523[0]) as usize * 1) as *const u32).read_unaligned() as u32), t1524[0]);
        t1524[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1523[1]) as usize * 1) as *const u32).read_unaligned() as u32), t1524[1]);
        t1524[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1523[1]) as usize * 1) as *const u32).read_unaligned() as u32), t1524[1]);
        t1524[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1523[1]) as usize * 1) as *const u32).read_unaligned() as u32), t1524[1]);
        t1524[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1523[1]) as usize * 1) as *const u32).read_unaligned() as u32), t1524[1]);
        let t1525: [uint32x4_t; 4] = [t1524[0], t1524[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1525;
        return res;
    }
    if ((x2) == (2u32)) {
        let t1526 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], x1i[0]), vandq_u32(live[1], x1i[1]))));
        let t1527: [uint32x4_t; 2] = [vbslq_u32(live[0], x1i[0], t1526), vbslq_u32(live[1], x1i[1], t1526)];
        let mut t1528: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t1528[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1527[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1528[0]);
        t1528[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1527[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1528[0]);
        t1528[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1527[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1528[0]);
        t1528[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1527[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1528[0]);
        t1528[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1527[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1528[1]);
        t1528[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1527[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1528[1]);
        t1528[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1527[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1528[1]);
        t1528[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1527[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1528[1]);
        let t1529: [uint32x4_t; 4] = [t1528[0], t1528[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1529;
        return res;
    }
    if ((x2) == (1u32)) {
        let t1530 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], x1i[0]), vandq_u32(live[1], x1i[1]))));
        let t1531: [uint32x4_t; 2] = [vbslq_u32(live[0], x1i[0], t1530), vbslq_u32(live[1], x1i[1], t1530)];
        let mut t1532: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t1532[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1531[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1532[0]);
        t1532[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1531[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1532[0]);
        t1532[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1531[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1532[0]);
        t1532[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1531[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1532[0]);
        t1532[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1531[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1532[1]);
        t1532[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1531[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1532[1]);
        t1532[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1531[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1532[1]);
        t1532[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1531[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1532[1]);
        let t1533: [uint32x4_t; 4] = [t1532[0], t1532[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1533;
        return res;
    }
    let t1534: [uint32x4_t; 2] = [x1i[0], x1i[1]];
    let t1535: u64 = ((x1b) as usize as u64);
    let t1536 = [vdupq_n_u32((t1535) as u32); 2];
    let t1537 = [vdupq_n_u32(((t1535) >> 32) as u32); 2];
    let t1538: [uint32x4_t; 2] = [vaddq_u32(t1536[0], t1534[0]), vaddq_u32(t1536[1], t1534[1])];
    let t1539: [uint32x4_t; 2] = [vcltq_u32(t1538[0], t1536[0]), vcltq_u32(t1538[1], t1536[1])];
    let t1540: [uint32x4_t; 4] = [t1538[0], t1538[1], vsubq_u32(vaddq_u32(t1537[0], vdupq_n_u32(0)), t1539[0]), vsubq_u32(vaddq_u32(t1537[1], vdupq_n_u32(0)), t1539[1])];
    let t1541: u64 = !(7u64);
    let t1542 = [vdupq_n_u32((t1541) as u32); 2];
    let t1543 = [vdupq_n_u32(((t1541) >> 32) as u32); 2];
    let t1544: [uint32x4_t; 4] = [vandq_u32(t1540[0], t1542[0]), vandq_u32(t1540[1], t1542[1]), vandq_u32(t1540[2], t1543[0]), vandq_u32(t1540[3], t1543[1])];
    let t1545: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
    let t1546: [uint32x4_t; 4] = [vandq_u32(t1544[0], t1545[0]), vandq_u32(t1544[1], t1545[1]), vandq_u32(t1544[2], t1545[2]), vandq_u32(t1544[3], t1545[3])];
    let t1547: u64 = ((arg0) as usize as u64);
    let t1548 = [vdupq_n_u32((t1547) as u32); 2];
    let t1549 = [vdupq_n_u32(((t1547) >> 32) as u32); 2];
    let t1550: [uint32x4_t; 2] = [vaddq_u32(t1548[0], t1546[0]), vaddq_u32(t1548[1], t1546[1])];
    let t1551: [uint32x4_t; 2] = [vcltq_u32(t1550[0], t1548[0]), vcltq_u32(t1550[1], t1548[1])];
    let t1552: [uint32x4_t; 4] = [t1550[0], t1550[1], vsubq_u32(vaddq_u32(t1549[0], t1546[2]), t1551[0]), vsubq_u32(vaddq_u32(t1549[1], t1546[3]), t1551[1])];
    x3 = t1552;
    let t1553: [uint32x4_t; 2] = [x1i[0], x1i[1]];
    let t1554: u64 = ((x1b) as usize as u64);
    let t1555 = [vdupq_n_u32((t1554) as u32); 2];
    let t1556 = [vdupq_n_u32(((t1554) >> 32) as u32); 2];
    let t1557: [uint32x4_t; 2] = [vaddq_u32(t1555[0], t1553[0]), vaddq_u32(t1555[1], t1553[1])];
    let t1558: [uint32x4_t; 2] = [vcltq_u32(t1557[0], t1555[0]), vcltq_u32(t1557[1], t1555[1])];
    let t1559: [uint32x4_t; 4] = [t1557[0], t1557[1], vsubq_u32(vaddq_u32(t1556[0], vdupq_n_u32(0)), t1558[0]), vsubq_u32(vaddq_u32(t1556[1], vdupq_n_u32(0)), t1558[1])];
    let t1560: u64 = 7u64;
    let t1561 = [vdupq_n_u32((t1560) as u32); 2];
    let t1562: [uint32x4_t; 2] = [vandq_u32(t1559[0], t1561[0]), vandq_u32(t1559[1], t1561[1])];
    let t1563 = [vdupq_n_u32(8u32); 2];
    let t1564: [uint32x4_t; 2] = [vmulq_u32(t1562[0], t1563[0]), vmulq_u32(t1562[1], t1563[1])];
    x4 = t1564;
    let mut t1565: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x3[2]) as u64) << 32 | vgetq_lane_u32::<0>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1565[0] = vsetq_lane_u32::<0>(v as u32, t1565[0]); t1565[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1565[2]); }
    { let v = (((vgetq_lane_u32::<1>(x3[2]) as u64) << 32 | vgetq_lane_u32::<1>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1565[0] = vsetq_lane_u32::<1>(v as u32, t1565[0]); t1565[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1565[2]); }
    { let v = (((vgetq_lane_u32::<2>(x3[2]) as u64) << 32 | vgetq_lane_u32::<2>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1565[0] = vsetq_lane_u32::<2>(v as u32, t1565[0]); t1565[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1565[2]); }
    { let v = (((vgetq_lane_u32::<3>(x3[2]) as u64) << 32 | vgetq_lane_u32::<3>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1565[0] = vsetq_lane_u32::<3>(v as u32, t1565[0]); t1565[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1565[2]); }
    { let v = (((vgetq_lane_u32::<0>(x3[3]) as u64) << 32 | vgetq_lane_u32::<0>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1565[1] = vsetq_lane_u32::<0>(v as u32, t1565[1]); t1565[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1565[3]); }
    { let v = (((vgetq_lane_u32::<1>(x3[3]) as u64) << 32 | vgetq_lane_u32::<1>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1565[1] = vsetq_lane_u32::<1>(v as u32, t1565[1]); t1565[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1565[3]); }
    { let v = (((vgetq_lane_u32::<2>(x3[3]) as u64) << 32 | vgetq_lane_u32::<2>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1565[1] = vsetq_lane_u32::<2>(v as u32, t1565[1]); t1565[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1565[3]); }
    { let v = (((vgetq_lane_u32::<3>(x3[3]) as u64) << 32 | vgetq_lane_u32::<3>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1565[1] = vsetq_lane_u32::<3>(v as u32, t1565[1]); t1565[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1565[3]); }
    let t1566: [uint32x4_t; 2] = [x4[0], x4[1]];
    let t1567: [uint32x4_t; 2] = [vandq_u32(t1566[0], vdupq_n_u32(63)), vandq_u32(t1566[1], vdupq_n_u32(63))];
    let t1568: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t1565[0], vnegq_s32(vreinterpretq_s32_u32(t1567[0]))), vshlq_u32(t1565[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1567[0])))), vorrq_u32(vshlq_u32(t1565[1], vnegq_s32(vreinterpretq_s32_u32(t1567[1]))), vshlq_u32(t1565[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1567[1])))), vshlq_u32(t1565[2], vnegq_s32(vreinterpretq_s32_u32(t1567[0]))), vshlq_u32(t1565[3], vnegq_s32(vreinterpretq_s32_u32(t1567[1])))];
    x5 = t1568;
    let t1569 = [vdupq_n_u32(0u32); 2];
    let t1570: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t1569[0])), vmvnq_u32(vceqq_u32(x4[1], t1569[1]))];
    let t1571 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
    let t1572: [uint32x4_t; 2] = [vaddq_u32(x4[0], t1571[0]), vaddq_u32(x4[1], t1571[1])];
    let t1573 = [vdupq_n_u32(64u32); 2];
    let t1574: [uint32x4_t; 2] = [vcgtq_u32(t1572[0], t1573[0]), vcgtq_u32(t1572[1], t1573[1])];
    let t1575: [uint32x4_t; 2] = [vandq_u32(t1570[0], t1574[0]), vandq_u32(t1570[1], t1574[1])];
    let mut t1576: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1577: [uint32x4_t; 2] = [vandq_u32(live[0], t1575[0]), vandq_u32(live[1], t1575[1])];
    let t1578: [uint32x4_t; 2] = [vbicq_u32(live[0], t1575[0]), vbicq_u32(live[1], t1575[1])];
    if any(&t1577) {
        let t1579: u64 = (1u64).wrapping_mul(8u64);
        let t1580 = [vdupq_n_u32((t1579) as u32); 2];
        let t1581 = [vdupq_n_u32(((t1579) >> 32) as u32); 2];
        let t1582: [uint32x4_t; 2] = [vaddq_u32(x3[0], t1580[0]), vaddq_u32(x3[1], t1580[1])];
        let t1583: [uint32x4_t; 2] = [vcltq_u32(t1582[0], x3[0]), vcltq_u32(t1582[1], x3[1])];
        let t1584: [uint32x4_t; 4] = [t1582[0], t1582[1], vsubq_u32(vaddq_u32(x3[2], t1581[0]), t1583[0]), vsubq_u32(vaddq_u32(x3[3], t1581[1]), t1583[1])];
        let mut t1585: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1584[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1584[0]) as u64) as usize as *const u64).read_unaligned(); t1585[0] = vsetq_lane_u32::<0>(v as u32, t1585[0]); t1585[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1585[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1584[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1584[0]) as u64) as usize as *const u64).read_unaligned(); t1585[0] = vsetq_lane_u32::<1>(v as u32, t1585[0]); t1585[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1585[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1584[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1584[0]) as u64) as usize as *const u64).read_unaligned(); t1585[0] = vsetq_lane_u32::<2>(v as u32, t1585[0]); t1585[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1585[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1584[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1584[0]) as u64) as usize as *const u64).read_unaligned(); t1585[0] = vsetq_lane_u32::<3>(v as u32, t1585[0]); t1585[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1585[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1584[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1584[1]) as u64) as usize as *const u64).read_unaligned(); t1585[1] = vsetq_lane_u32::<0>(v as u32, t1585[1]); t1585[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1585[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1584[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1584[1]) as u64) as usize as *const u64).read_unaligned(); t1585[1] = vsetq_lane_u32::<1>(v as u32, t1585[1]); t1585[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1585[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1584[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1584[1]) as u64) as usize as *const u64).read_unaligned(); t1585[1] = vsetq_lane_u32::<2>(v as u32, t1585[1]); t1585[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1585[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1584[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1584[1]) as u64) as usize as *const u64).read_unaligned(); t1585[1] = vsetq_lane_u32::<3>(v as u32, t1585[1]); t1585[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1585[3]); }
        let t1586: [uint32x4_t; 2] = [vsubq_u32(t1573[0], x4[0]), vsubq_u32(t1573[1], x4[1])];
        let t1587: [uint32x4_t; 2] = [t1586[0], t1586[1]];
        let t1588: [uint32x4_t; 2] = [vandq_u32(t1587[0], vdupq_n_u32(63)), vandq_u32(t1587[1], vdupq_n_u32(63))];
        let t1589: [uint32x4_t; 4] = [vshlq_u32(t1585[0], vreinterpretq_s32_u32(t1588[0])), vshlq_u32(t1585[1], vreinterpretq_s32_u32(t1588[1])), vorrq_u32(vshlq_u32(t1585[2], vreinterpretq_s32_u32(t1588[0])), vshlq_u32(t1585[0], vsubq_s32(vreinterpretq_s32_u32(t1588[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1585[3], vreinterpretq_s32_u32(t1588[1])), vshlq_u32(t1585[1], vsubq_s32(vreinterpretq_s32_u32(t1588[1]), vdupq_n_s32(32))))];
        let t1590: [uint32x4_t; 4] = [vorrq_u32(x5[0], t1589[0]), vorrq_u32(x5[1], t1589[1]), vorrq_u32(x5[2], t1589[2]), vorrq_u32(x5[3], t1589[3])];
        t1576 = [vbslq_u32(t1577[0], t1590[0], t1576[0]), vbslq_u32(t1577[1], t1590[1], t1576[1]), vbslq_u32(t1577[0], t1590[2], t1576[2]), vbslq_u32(t1577[1], t1590[3], t1576[3])];
    }
    {
        t1576 = [vbslq_u32(t1578[0], x5[0], t1576[0]), vbslq_u32(t1578[1], x5[1], t1576[1]), vbslq_u32(t1578[0], x5[2], t1576[2]), vbslq_u32(t1578[1], x5[3], t1576[3])];
    }
    res = t1576;
    return res;
}

#[inline(always)]
unsafe fn b28_r0an1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1b: *const u8, x1if: u32, x1is: u32, x2: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x3: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x4: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x5: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if ((x2) == (8u32)) {
        let t1591: [uint32x4_t; 4];
        if (x1is) == 8u32 && ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 && all(&live) {
            let t1592 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
            let t15930 = vld2q_u32((t1592 as *const u32).add(0));
            let t15931 = vld2q_u32((t1592 as *const u32).add(8));
            let t1593: [uint32x4_t; 4] = [t15930.0, t15931.0, t15930.1, t15931.1];
            t1591 = t1593;
        } else {
            let t1594: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
            let t1595 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t1594[0]), vandq_u32(live[1], t1594[1]))));
            let t1596: [uint32x4_t; 2] = [vbslq_u32(live[0], t1594[0], t1595), vbslq_u32(live[1], t1594[1], t1595)];
            let mut t1597: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1596[0]) as usize * 1) as *const u64).read_unaligned(); t1597[0] = vsetq_lane_u32::<0>(v as u32, t1597[0]); t1597[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1597[2]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1596[0]) as usize * 1) as *const u64).read_unaligned(); t1597[0] = vsetq_lane_u32::<1>(v as u32, t1597[0]); t1597[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1597[2]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1596[0]) as usize * 1) as *const u64).read_unaligned(); t1597[0] = vsetq_lane_u32::<2>(v as u32, t1597[0]); t1597[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1597[2]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1596[0]) as usize * 1) as *const u64).read_unaligned(); t1597[0] = vsetq_lane_u32::<3>(v as u32, t1597[0]); t1597[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1597[2]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1596[1]) as usize * 1) as *const u64).read_unaligned(); t1597[1] = vsetq_lane_u32::<0>(v as u32, t1597[1]); t1597[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1597[3]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1596[1]) as usize * 1) as *const u64).read_unaligned(); t1597[1] = vsetq_lane_u32::<1>(v as u32, t1597[1]); t1597[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1597[3]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1596[1]) as usize * 1) as *const u64).read_unaligned(); t1597[1] = vsetq_lane_u32::<2>(v as u32, t1597[1]); t1597[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1597[3]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1596[1]) as usize * 1) as *const u64).read_unaligned(); t1597[1] = vsetq_lane_u32::<3>(v as u32, t1597[1]); t1597[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1597[3]); }
            t1591 = t1597;
        }
        res = t1591;
        return res;
    }
    if ((x2) == (4u32)) {
        let t1598: [uint32x4_t; 2];
        if (x1is) == 4u32 && ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 && all(&live) {
            let t1599 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
            let t1600: [uint32x4_t; 2] = core::array::from_fn(|r| vld1q_u32((t1599 as *const u32).add(4 * r)));
            t1598 = t1600;
        } else {
            let t1601: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
            let t1602 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t1601[0]), vandq_u32(live[1], t1601[1]))));
            let t1603: [uint32x4_t; 2] = [vbslq_u32(live[0], t1601[0], t1602), vbslq_u32(live[1], t1601[1], t1602)];
            let mut t1604: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            t1604[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1603[0]) as usize * 1) as *const u32).read_unaligned() as u32), t1604[0]);
            t1604[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1603[0]) as usize * 1) as *const u32).read_unaligned() as u32), t1604[0]);
            t1604[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1603[0]) as usize * 1) as *const u32).read_unaligned() as u32), t1604[0]);
            t1604[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1603[0]) as usize * 1) as *const u32).read_unaligned() as u32), t1604[0]);
            t1604[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1603[1]) as usize * 1) as *const u32).read_unaligned() as u32), t1604[1]);
            t1604[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1603[1]) as usize * 1) as *const u32).read_unaligned() as u32), t1604[1]);
            t1604[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1603[1]) as usize * 1) as *const u32).read_unaligned() as u32), t1604[1]);
            t1604[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1603[1]) as usize * 1) as *const u32).read_unaligned() as u32), t1604[1]);
            t1598 = t1604;
        }
        let t1605: [uint32x4_t; 4] = [t1598[0], t1598[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1605;
        return res;
    }
    if ((x2) == (2u32)) {
        let t1606: [uint32x4_t; 2];
        if (x1is) == 2u32 && ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 && all(&live) {
            let t1607 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
            let t1608: [uint32x4_t; 2] = core::array::from_fn(|r| vmovl_u16(vld1_u16((t1607 as *const u16).add(4 * r))));
            t1606 = t1608;
        } else {
            let t1609: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
            let t1610 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t1609[0]), vandq_u32(live[1], t1609[1]))));
            let t1611: [uint32x4_t; 2] = [vbslq_u32(live[0], t1609[0], t1610), vbslq_u32(live[1], t1609[1], t1610)];
            let mut t1612: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            t1612[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1611[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1612[0]);
            t1612[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1611[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1612[0]);
            t1612[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1611[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1612[0]);
            t1612[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1611[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1612[0]);
            t1612[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1611[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1612[1]);
            t1612[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1611[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1612[1]);
            t1612[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1611[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1612[1]);
            t1612[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1611[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1612[1]);
            t1606 = t1612;
        }
        let t1613: [uint32x4_t; 4] = [t1606[0], t1606[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1613;
        return res;
    }
    if ((x2) == (1u32)) {
        let t1614: [uint32x4_t; 2];
        if (x1is) == 1u32 && ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 && all(&live) {
            let t1615 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
            let t1617 = vmovl_u8(vld1_u8(t1615.add(0)));
            let t1616: [uint32x4_t; 2] = [vmovl_u16(vget_low_u16(t1617)), vmovl_u16(vget_high_u16(t1617))];
            t1614 = t1616;
        } else {
            let t1618: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
            let t1619 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t1618[0]), vandq_u32(live[1], t1618[1]))));
            let t1620: [uint32x4_t; 2] = [vbslq_u32(live[0], t1618[0], t1619), vbslq_u32(live[1], t1618[1], t1619)];
            let mut t1621: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            t1621[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1620[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1621[0]);
            t1621[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1620[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1621[0]);
            t1621[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1620[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1621[0]);
            t1621[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1620[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1621[0]);
            t1621[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1620[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1621[1]);
            t1621[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1620[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1621[1]);
            t1621[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1620[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1621[1]);
            t1621[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1620[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1621[1]);
            t1614 = t1621;
        }
        let t1622: [uint32x4_t; 4] = [t1614[0], t1614[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1622;
        return res;
    }
    let t1623: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
    let t1624: [uint32x4_t; 2] = [t1623[0], t1623[1]];
    let t1625: u64 = ((x1b) as usize as u64);
    let t1626 = [vdupq_n_u32((t1625) as u32); 2];
    let t1627 = [vdupq_n_u32(((t1625) >> 32) as u32); 2];
    let t1628: [uint32x4_t; 2] = [vaddq_u32(t1626[0], t1624[0]), vaddq_u32(t1626[1], t1624[1])];
    let t1629: [uint32x4_t; 2] = [vcltq_u32(t1628[0], t1626[0]), vcltq_u32(t1628[1], t1626[1])];
    let t1630: [uint32x4_t; 4] = [t1628[0], t1628[1], vsubq_u32(vaddq_u32(t1627[0], vdupq_n_u32(0)), t1629[0]), vsubq_u32(vaddq_u32(t1627[1], vdupq_n_u32(0)), t1629[1])];
    let t1631: u64 = !(7u64);
    let t1632 = [vdupq_n_u32((t1631) as u32); 2];
    let t1633 = [vdupq_n_u32(((t1631) >> 32) as u32); 2];
    let t1634: [uint32x4_t; 4] = [vandq_u32(t1630[0], t1632[0]), vandq_u32(t1630[1], t1632[1]), vandq_u32(t1630[2], t1633[0]), vandq_u32(t1630[3], t1633[1])];
    let t1635: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
    let t1636: [uint32x4_t; 4] = [vandq_u32(t1634[0], t1635[0]), vandq_u32(t1634[1], t1635[1]), vandq_u32(t1634[2], t1635[2]), vandq_u32(t1634[3], t1635[3])];
    let t1637: u64 = ((arg0) as usize as u64);
    let t1638 = [vdupq_n_u32((t1637) as u32); 2];
    let t1639 = [vdupq_n_u32(((t1637) >> 32) as u32); 2];
    let t1640: [uint32x4_t; 2] = [vaddq_u32(t1638[0], t1636[0]), vaddq_u32(t1638[1], t1636[1])];
    let t1641: [uint32x4_t; 2] = [vcltq_u32(t1640[0], t1638[0]), vcltq_u32(t1640[1], t1638[1])];
    let t1642: [uint32x4_t; 4] = [t1640[0], t1640[1], vsubq_u32(vaddq_u32(t1639[0], t1636[2]), t1641[0]), vsubq_u32(vaddq_u32(t1639[1], t1636[3]), t1641[1])];
    x3 = t1642;
    let t1643: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
    let t1644: [uint32x4_t; 2] = [t1643[0], t1643[1]];
    let t1645: u64 = ((x1b) as usize as u64);
    let t1646 = [vdupq_n_u32((t1645) as u32); 2];
    let t1647 = [vdupq_n_u32(((t1645) >> 32) as u32); 2];
    let t1648: [uint32x4_t; 2] = [vaddq_u32(t1646[0], t1644[0]), vaddq_u32(t1646[1], t1644[1])];
    let t1649: [uint32x4_t; 2] = [vcltq_u32(t1648[0], t1646[0]), vcltq_u32(t1648[1], t1646[1])];
    let t1650: [uint32x4_t; 4] = [t1648[0], t1648[1], vsubq_u32(vaddq_u32(t1647[0], vdupq_n_u32(0)), t1649[0]), vsubq_u32(vaddq_u32(t1647[1], vdupq_n_u32(0)), t1649[1])];
    let t1651: u64 = 7u64;
    let t1652 = [vdupq_n_u32((t1651) as u32); 2];
    let t1653: [uint32x4_t; 2] = [vandq_u32(t1650[0], t1652[0]), vandq_u32(t1650[1], t1652[1])];
    let t1654 = [vdupq_n_u32(8u32); 2];
    let t1655: [uint32x4_t; 2] = [vmulq_u32(t1653[0], t1654[0]), vmulq_u32(t1653[1], t1654[1])];
    x4 = t1655;
    let mut t1656: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x3[2]) as u64) << 32 | vgetq_lane_u32::<0>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1656[0] = vsetq_lane_u32::<0>(v as u32, t1656[0]); t1656[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1656[2]); }
    { let v = (((vgetq_lane_u32::<1>(x3[2]) as u64) << 32 | vgetq_lane_u32::<1>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1656[0] = vsetq_lane_u32::<1>(v as u32, t1656[0]); t1656[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1656[2]); }
    { let v = (((vgetq_lane_u32::<2>(x3[2]) as u64) << 32 | vgetq_lane_u32::<2>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1656[0] = vsetq_lane_u32::<2>(v as u32, t1656[0]); t1656[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1656[2]); }
    { let v = (((vgetq_lane_u32::<3>(x3[2]) as u64) << 32 | vgetq_lane_u32::<3>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1656[0] = vsetq_lane_u32::<3>(v as u32, t1656[0]); t1656[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1656[2]); }
    { let v = (((vgetq_lane_u32::<0>(x3[3]) as u64) << 32 | vgetq_lane_u32::<0>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1656[1] = vsetq_lane_u32::<0>(v as u32, t1656[1]); t1656[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1656[3]); }
    { let v = (((vgetq_lane_u32::<1>(x3[3]) as u64) << 32 | vgetq_lane_u32::<1>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1656[1] = vsetq_lane_u32::<1>(v as u32, t1656[1]); t1656[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1656[3]); }
    { let v = (((vgetq_lane_u32::<2>(x3[3]) as u64) << 32 | vgetq_lane_u32::<2>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1656[1] = vsetq_lane_u32::<2>(v as u32, t1656[1]); t1656[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1656[3]); }
    { let v = (((vgetq_lane_u32::<3>(x3[3]) as u64) << 32 | vgetq_lane_u32::<3>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1656[1] = vsetq_lane_u32::<3>(v as u32, t1656[1]); t1656[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1656[3]); }
    let t1657: [uint32x4_t; 2] = [x4[0], x4[1]];
    let t1658: [uint32x4_t; 2] = [vandq_u32(t1657[0], vdupq_n_u32(63)), vandq_u32(t1657[1], vdupq_n_u32(63))];
    let t1659: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t1656[0], vnegq_s32(vreinterpretq_s32_u32(t1658[0]))), vshlq_u32(t1656[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1658[0])))), vorrq_u32(vshlq_u32(t1656[1], vnegq_s32(vreinterpretq_s32_u32(t1658[1]))), vshlq_u32(t1656[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1658[1])))), vshlq_u32(t1656[2], vnegq_s32(vreinterpretq_s32_u32(t1658[0]))), vshlq_u32(t1656[3], vnegq_s32(vreinterpretq_s32_u32(t1658[1])))];
    x5 = t1659;
    let t1660 = [vdupq_n_u32(0u32); 2];
    let t1661: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t1660[0])), vmvnq_u32(vceqq_u32(x4[1], t1660[1]))];
    let t1662 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
    let t1663: [uint32x4_t; 2] = [vaddq_u32(x4[0], t1662[0]), vaddq_u32(x4[1], t1662[1])];
    let t1664 = [vdupq_n_u32(64u32); 2];
    let t1665: [uint32x4_t; 2] = [vcgtq_u32(t1663[0], t1664[0]), vcgtq_u32(t1663[1], t1664[1])];
    let t1666: [uint32x4_t; 2] = [vandq_u32(t1661[0], t1665[0]), vandq_u32(t1661[1], t1665[1])];
    let mut t1667: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1668: [uint32x4_t; 2] = [vandq_u32(live[0], t1666[0]), vandq_u32(live[1], t1666[1])];
    let t1669: [uint32x4_t; 2] = [vbicq_u32(live[0], t1666[0]), vbicq_u32(live[1], t1666[1])];
    if any(&t1668) {
        let t1670: u64 = (1u64).wrapping_mul(8u64);
        let t1671 = [vdupq_n_u32((t1670) as u32); 2];
        let t1672 = [vdupq_n_u32(((t1670) >> 32) as u32); 2];
        let t1673: [uint32x4_t; 2] = [vaddq_u32(x3[0], t1671[0]), vaddq_u32(x3[1], t1671[1])];
        let t1674: [uint32x4_t; 2] = [vcltq_u32(t1673[0], x3[0]), vcltq_u32(t1673[1], x3[1])];
        let t1675: [uint32x4_t; 4] = [t1673[0], t1673[1], vsubq_u32(vaddq_u32(x3[2], t1672[0]), t1674[0]), vsubq_u32(vaddq_u32(x3[3], t1672[1]), t1674[1])];
        let mut t1676: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1675[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1675[0]) as u64) as usize as *const u64).read_unaligned(); t1676[0] = vsetq_lane_u32::<0>(v as u32, t1676[0]); t1676[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1676[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1675[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1675[0]) as u64) as usize as *const u64).read_unaligned(); t1676[0] = vsetq_lane_u32::<1>(v as u32, t1676[0]); t1676[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1676[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1675[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1675[0]) as u64) as usize as *const u64).read_unaligned(); t1676[0] = vsetq_lane_u32::<2>(v as u32, t1676[0]); t1676[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1676[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1675[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1675[0]) as u64) as usize as *const u64).read_unaligned(); t1676[0] = vsetq_lane_u32::<3>(v as u32, t1676[0]); t1676[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1676[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1675[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1675[1]) as u64) as usize as *const u64).read_unaligned(); t1676[1] = vsetq_lane_u32::<0>(v as u32, t1676[1]); t1676[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1676[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1675[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1675[1]) as u64) as usize as *const u64).read_unaligned(); t1676[1] = vsetq_lane_u32::<1>(v as u32, t1676[1]); t1676[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1676[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1675[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1675[1]) as u64) as usize as *const u64).read_unaligned(); t1676[1] = vsetq_lane_u32::<2>(v as u32, t1676[1]); t1676[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1676[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1675[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1675[1]) as u64) as usize as *const u64).read_unaligned(); t1676[1] = vsetq_lane_u32::<3>(v as u32, t1676[1]); t1676[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1676[3]); }
        let t1677: [uint32x4_t; 2] = [vsubq_u32(t1664[0], x4[0]), vsubq_u32(t1664[1], x4[1])];
        let t1678: [uint32x4_t; 2] = [t1677[0], t1677[1]];
        let t1679: [uint32x4_t; 2] = [vandq_u32(t1678[0], vdupq_n_u32(63)), vandq_u32(t1678[1], vdupq_n_u32(63))];
        let t1680: [uint32x4_t; 4] = [vshlq_u32(t1676[0], vreinterpretq_s32_u32(t1679[0])), vshlq_u32(t1676[1], vreinterpretq_s32_u32(t1679[1])), vorrq_u32(vshlq_u32(t1676[2], vreinterpretq_s32_u32(t1679[0])), vshlq_u32(t1676[0], vsubq_s32(vreinterpretq_s32_u32(t1679[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1676[3], vreinterpretq_s32_u32(t1679[1])), vshlq_u32(t1676[1], vsubq_s32(vreinterpretq_s32_u32(t1679[1]), vdupq_n_s32(32))))];
        let t1681: [uint32x4_t; 4] = [vorrq_u32(x5[0], t1680[0]), vorrq_u32(x5[1], t1680[1]), vorrq_u32(x5[2], t1680[2]), vorrq_u32(x5[3], t1680[3])];
        t1667 = [vbslq_u32(t1668[0], t1681[0], t1667[0]), vbslq_u32(t1668[1], t1681[1], t1667[1]), vbslq_u32(t1668[0], t1681[2], t1667[2]), vbslq_u32(t1668[1], t1681[3], t1667[3])];
    }
    {
        t1667 = [vbslq_u32(t1669[0], x5[0], t1667[0]), vbslq_u32(t1669[1], x5[1], t1667[1]), vbslq_u32(t1669[0], x5[2], t1667[2]), vbslq_u32(t1669[1], x5[3], t1667[3])];
    }
    res = t1667;
    return res;
}

#[inline(always)]
unsafe fn b28_r0an1s_f(l: &Launch, lid0: u32, arg0: *const u8, x1b: *const u8, x1if: u32, x1is: u32, x2: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x3: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x4: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x5: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if ((x2) == (8u32)) {
        let t1682: [uint32x4_t; 4];
        if (x1is) == 8u32 && ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 {
            let t1683 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
            let t16840 = vld2q_u32((t1683 as *const u32).add(0));
            let t16841 = vld2q_u32((t1683 as *const u32).add(8));
            let t1684: [uint32x4_t; 4] = [t16840.0, t16841.0, t16840.1, t16841.1];
            t1682 = t1684;
        } else {
            let t1685: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
            let mut t1686: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1685[0]) as usize * 1) as *const u64).read_unaligned(); t1686[0] = vsetq_lane_u32::<0>(v as u32, t1686[0]); t1686[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1686[2]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1685[0]) as usize * 1) as *const u64).read_unaligned(); t1686[0] = vsetq_lane_u32::<1>(v as u32, t1686[0]); t1686[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1686[2]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1685[0]) as usize * 1) as *const u64).read_unaligned(); t1686[0] = vsetq_lane_u32::<2>(v as u32, t1686[0]); t1686[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1686[2]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1685[0]) as usize * 1) as *const u64).read_unaligned(); t1686[0] = vsetq_lane_u32::<3>(v as u32, t1686[0]); t1686[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1686[2]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1685[1]) as usize * 1) as *const u64).read_unaligned(); t1686[1] = vsetq_lane_u32::<0>(v as u32, t1686[1]); t1686[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1686[3]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1685[1]) as usize * 1) as *const u64).read_unaligned(); t1686[1] = vsetq_lane_u32::<1>(v as u32, t1686[1]); t1686[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1686[3]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1685[1]) as usize * 1) as *const u64).read_unaligned(); t1686[1] = vsetq_lane_u32::<2>(v as u32, t1686[1]); t1686[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1686[3]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1685[1]) as usize * 1) as *const u64).read_unaligned(); t1686[1] = vsetq_lane_u32::<3>(v as u32, t1686[1]); t1686[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1686[3]); }
            t1682 = t1686;
        }
        res = t1682;
        return res;
    }
    if ((x2) == (4u32)) {
        let t1687: [uint32x4_t; 2];
        if (x1is) == 4u32 && ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 {
            let t1688 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
            let t1689: [uint32x4_t; 2] = core::array::from_fn(|r| vld1q_u32((t1688 as *const u32).add(4 * r)));
            t1687 = t1689;
        } else {
            let t1690: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
            let mut t1691: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            t1691[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1690[0]) as usize * 1) as *const u32).read_unaligned() as u32), t1691[0]);
            t1691[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1690[0]) as usize * 1) as *const u32).read_unaligned() as u32), t1691[0]);
            t1691[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1690[0]) as usize * 1) as *const u32).read_unaligned() as u32), t1691[0]);
            t1691[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1690[0]) as usize * 1) as *const u32).read_unaligned() as u32), t1691[0]);
            t1691[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1690[1]) as usize * 1) as *const u32).read_unaligned() as u32), t1691[1]);
            t1691[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1690[1]) as usize * 1) as *const u32).read_unaligned() as u32), t1691[1]);
            t1691[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1690[1]) as usize * 1) as *const u32).read_unaligned() as u32), t1691[1]);
            t1691[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1690[1]) as usize * 1) as *const u32).read_unaligned() as u32), t1691[1]);
            t1687 = t1691;
        }
        let t1692: [uint32x4_t; 4] = [t1687[0], t1687[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1692;
        return res;
    }
    if ((x2) == (2u32)) {
        let t1693: [uint32x4_t; 2];
        if (x1is) == 2u32 && ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 {
            let t1694 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
            let t1695: [uint32x4_t; 2] = core::array::from_fn(|r| vmovl_u16(vld1_u16((t1694 as *const u16).add(4 * r))));
            t1693 = t1695;
        } else {
            let t1696: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
            let mut t1697: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            t1697[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1696[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1697[0]);
            t1697[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1696[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1697[0]);
            t1697[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1696[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1697[0]);
            t1697[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1696[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1697[0]);
            t1697[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1696[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1697[1]);
            t1697[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1696[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1697[1]);
            t1697[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1696[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1697[1]);
            t1697[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1696[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1697[1]);
            t1693 = t1697;
        }
        let t1698: [uint32x4_t; 4] = [t1693[0], t1693[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1698;
        return res;
    }
    if ((x2) == (1u32)) {
        let t1699: [uint32x4_t; 2];
        if (x1is) == 1u32 && ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 {
            let t1700 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
            let t1702 = vmovl_u8(vld1_u8(t1700.add(0)));
            let t1701: [uint32x4_t; 2] = [vmovl_u16(vget_low_u16(t1702)), vmovl_u16(vget_high_u16(t1702))];
            t1699 = t1701;
        } else {
            let t1703: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
            let mut t1704: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            t1704[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1703[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1704[0]);
            t1704[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1703[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1704[0]);
            t1704[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1703[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1704[0]);
            t1704[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1703[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1704[0]);
            t1704[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1703[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1704[1]);
            t1704[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1703[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1704[1]);
            t1704[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1703[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1704[1]);
            t1704[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1703[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1704[1]);
            t1699 = t1704;
        }
        let t1705: [uint32x4_t; 4] = [t1699[0], t1699[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1705;
        return res;
    }
    let t1706: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
    let t1707: [uint32x4_t; 2] = [t1706[0], t1706[1]];
    let t1708: u64 = ((x1b) as usize as u64);
    let t1709 = [vdupq_n_u32((t1708) as u32); 2];
    let t1710 = [vdupq_n_u32(((t1708) >> 32) as u32); 2];
    let t1711: [uint32x4_t; 2] = [vaddq_u32(t1709[0], t1707[0]), vaddq_u32(t1709[1], t1707[1])];
    let t1712: [uint32x4_t; 2] = [vcltq_u32(t1711[0], t1709[0]), vcltq_u32(t1711[1], t1709[1])];
    let t1713: [uint32x4_t; 4] = [t1711[0], t1711[1], vsubq_u32(vaddq_u32(t1710[0], vdupq_n_u32(0)), t1712[0]), vsubq_u32(vaddq_u32(t1710[1], vdupq_n_u32(0)), t1712[1])];
    let t1714: u64 = !(7u64);
    let t1715 = [vdupq_n_u32((t1714) as u32); 2];
    let t1716 = [vdupq_n_u32(((t1714) >> 32) as u32); 2];
    let t1717: [uint32x4_t; 4] = [vandq_u32(t1713[0], t1715[0]), vandq_u32(t1713[1], t1715[1]), vandq_u32(t1713[2], t1716[0]), vandq_u32(t1713[3], t1716[1])];
    let t1718: u64 = ((arg0) as usize as u64);
    let t1719 = [vdupq_n_u32((t1718) as u32); 2];
    let t1720 = [vdupq_n_u32(((t1718) >> 32) as u32); 2];
    let t1721: [uint32x4_t; 2] = [vaddq_u32(t1719[0], t1717[0]), vaddq_u32(t1719[1], t1717[1])];
    let t1722: [uint32x4_t; 2] = [vcltq_u32(t1721[0], t1719[0]), vcltq_u32(t1721[1], t1719[1])];
    let t1723: [uint32x4_t; 4] = [t1721[0], t1721[1], vsubq_u32(vaddq_u32(t1720[0], t1717[2]), t1722[0]), vsubq_u32(vaddq_u32(t1720[1], t1717[3]), t1722[1])];
    x3 = t1723;
    let t1724: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
    let t1725: [uint32x4_t; 2] = [t1724[0], t1724[1]];
    let t1726: u64 = ((x1b) as usize as u64);
    let t1727 = [vdupq_n_u32((t1726) as u32); 2];
    let t1728 = [vdupq_n_u32(((t1726) >> 32) as u32); 2];
    let t1729: [uint32x4_t; 2] = [vaddq_u32(t1727[0], t1725[0]), vaddq_u32(t1727[1], t1725[1])];
    let t1730: [uint32x4_t; 2] = [vcltq_u32(t1729[0], t1727[0]), vcltq_u32(t1729[1], t1727[1])];
    let t1731: [uint32x4_t; 4] = [t1729[0], t1729[1], vsubq_u32(vaddq_u32(t1728[0], vdupq_n_u32(0)), t1730[0]), vsubq_u32(vaddq_u32(t1728[1], vdupq_n_u32(0)), t1730[1])];
    let t1732: u64 = 7u64;
    let t1733 = [vdupq_n_u32((t1732) as u32); 2];
    let t1734: [uint32x4_t; 2] = [vandq_u32(t1731[0], t1733[0]), vandq_u32(t1731[1], t1733[1])];
    let t1735 = [vdupq_n_u32(8u32); 2];
    let t1736: [uint32x4_t; 2] = [vmulq_u32(t1734[0], t1735[0]), vmulq_u32(t1734[1], t1735[1])];
    x4 = t1736;
    let mut t1737: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x3[2]) as u64) << 32 | vgetq_lane_u32::<0>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1737[0] = vsetq_lane_u32::<0>(v as u32, t1737[0]); t1737[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1737[2]); }
    { let v = (((vgetq_lane_u32::<1>(x3[2]) as u64) << 32 | vgetq_lane_u32::<1>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1737[0] = vsetq_lane_u32::<1>(v as u32, t1737[0]); t1737[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1737[2]); }
    { let v = (((vgetq_lane_u32::<2>(x3[2]) as u64) << 32 | vgetq_lane_u32::<2>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1737[0] = vsetq_lane_u32::<2>(v as u32, t1737[0]); t1737[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1737[2]); }
    { let v = (((vgetq_lane_u32::<3>(x3[2]) as u64) << 32 | vgetq_lane_u32::<3>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1737[0] = vsetq_lane_u32::<3>(v as u32, t1737[0]); t1737[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1737[2]); }
    { let v = (((vgetq_lane_u32::<0>(x3[3]) as u64) << 32 | vgetq_lane_u32::<0>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1737[1] = vsetq_lane_u32::<0>(v as u32, t1737[1]); t1737[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1737[3]); }
    { let v = (((vgetq_lane_u32::<1>(x3[3]) as u64) << 32 | vgetq_lane_u32::<1>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1737[1] = vsetq_lane_u32::<1>(v as u32, t1737[1]); t1737[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1737[3]); }
    { let v = (((vgetq_lane_u32::<2>(x3[3]) as u64) << 32 | vgetq_lane_u32::<2>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1737[1] = vsetq_lane_u32::<2>(v as u32, t1737[1]); t1737[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1737[3]); }
    { let v = (((vgetq_lane_u32::<3>(x3[3]) as u64) << 32 | vgetq_lane_u32::<3>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1737[1] = vsetq_lane_u32::<3>(v as u32, t1737[1]); t1737[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1737[3]); }
    let t1738: [uint32x4_t; 2] = [x4[0], x4[1]];
    let t1739: [uint32x4_t; 2] = [vandq_u32(t1738[0], vdupq_n_u32(63)), vandq_u32(t1738[1], vdupq_n_u32(63))];
    let t1740: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t1737[0], vnegq_s32(vreinterpretq_s32_u32(t1739[0]))), vshlq_u32(t1737[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1739[0])))), vorrq_u32(vshlq_u32(t1737[1], vnegq_s32(vreinterpretq_s32_u32(t1739[1]))), vshlq_u32(t1737[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1739[1])))), vshlq_u32(t1737[2], vnegq_s32(vreinterpretq_s32_u32(t1739[0]))), vshlq_u32(t1737[3], vnegq_s32(vreinterpretq_s32_u32(t1739[1])))];
    x5 = t1740;
    let t1741 = [vdupq_n_u32(0u32); 2];
    let t1742: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t1741[0])), vmvnq_u32(vceqq_u32(x4[1], t1741[1]))];
    let t1743 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
    let t1744: [uint32x4_t; 2] = [vaddq_u32(x4[0], t1743[0]), vaddq_u32(x4[1], t1743[1])];
    let t1745 = [vdupq_n_u32(64u32); 2];
    let t1746: [uint32x4_t; 2] = [vcgtq_u32(t1744[0], t1745[0]), vcgtq_u32(t1744[1], t1745[1])];
    let t1747: [uint32x4_t; 2] = [vandq_u32(t1742[0], t1746[0]), vandq_u32(t1742[1], t1746[1])];
    let mut t1748: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1749: [uint32x4_t; 2] = [vmvnq_u32(t1747[0]), vmvnq_u32(t1747[1])];
    if any(&t1747) {
        let t1750: u64 = (1u64).wrapping_mul(8u64);
        let t1751 = [vdupq_n_u32((t1750) as u32); 2];
        let t1752 = [vdupq_n_u32(((t1750) >> 32) as u32); 2];
        let t1753: [uint32x4_t; 2] = [vaddq_u32(x3[0], t1751[0]), vaddq_u32(x3[1], t1751[1])];
        let t1754: [uint32x4_t; 2] = [vcltq_u32(t1753[0], x3[0]), vcltq_u32(t1753[1], x3[1])];
        let t1755: [uint32x4_t; 4] = [t1753[0], t1753[1], vsubq_u32(vaddq_u32(x3[2], t1752[0]), t1754[0]), vsubq_u32(vaddq_u32(x3[3], t1752[1]), t1754[1])];
        let mut t1756: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1755[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1755[0]) as u64) as usize as *const u64).read_unaligned(); t1756[0] = vsetq_lane_u32::<0>(v as u32, t1756[0]); t1756[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1756[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1755[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1755[0]) as u64) as usize as *const u64).read_unaligned(); t1756[0] = vsetq_lane_u32::<1>(v as u32, t1756[0]); t1756[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1756[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1755[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1755[0]) as u64) as usize as *const u64).read_unaligned(); t1756[0] = vsetq_lane_u32::<2>(v as u32, t1756[0]); t1756[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1756[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1755[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1755[0]) as u64) as usize as *const u64).read_unaligned(); t1756[0] = vsetq_lane_u32::<3>(v as u32, t1756[0]); t1756[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1756[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1755[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1755[1]) as u64) as usize as *const u64).read_unaligned(); t1756[1] = vsetq_lane_u32::<0>(v as u32, t1756[1]); t1756[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1756[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1755[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1755[1]) as u64) as usize as *const u64).read_unaligned(); t1756[1] = vsetq_lane_u32::<1>(v as u32, t1756[1]); t1756[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1756[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1755[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1755[1]) as u64) as usize as *const u64).read_unaligned(); t1756[1] = vsetq_lane_u32::<2>(v as u32, t1756[1]); t1756[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1756[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1755[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1755[1]) as u64) as usize as *const u64).read_unaligned(); t1756[1] = vsetq_lane_u32::<3>(v as u32, t1756[1]); t1756[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1756[3]); }
        let t1757: [uint32x4_t; 2] = [vsubq_u32(t1745[0], x4[0]), vsubq_u32(t1745[1], x4[1])];
        let t1758: [uint32x4_t; 2] = [t1757[0], t1757[1]];
        let t1759: [uint32x4_t; 2] = [vandq_u32(t1758[0], vdupq_n_u32(63)), vandq_u32(t1758[1], vdupq_n_u32(63))];
        let t1760: [uint32x4_t; 4] = [vshlq_u32(t1756[0], vreinterpretq_s32_u32(t1759[0])), vshlq_u32(t1756[1], vreinterpretq_s32_u32(t1759[1])), vorrq_u32(vshlq_u32(t1756[2], vreinterpretq_s32_u32(t1759[0])), vshlq_u32(t1756[0], vsubq_s32(vreinterpretq_s32_u32(t1759[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1756[3], vreinterpretq_s32_u32(t1759[1])), vshlq_u32(t1756[1], vsubq_s32(vreinterpretq_s32_u32(t1759[1]), vdupq_n_s32(32))))];
        let t1761: [uint32x4_t; 4] = [vorrq_u32(x5[0], t1760[0]), vorrq_u32(x5[1], t1760[1]), vorrq_u32(x5[2], t1760[2]), vorrq_u32(x5[3], t1760[3])];
        t1748 = [vbslq_u32(t1747[0], t1761[0], t1748[0]), vbslq_u32(t1747[1], t1761[1], t1748[1]), vbslq_u32(t1747[0], t1761[2], t1748[2]), vbslq_u32(t1747[1], t1761[3], t1748[3])];
    }
    {
        t1748 = [vbslq_u32(t1749[0], x5[0], t1748[0]), vbslq_u32(t1749[1], x5[1], t1748[1]), vbslq_u32(t1749[0], x5[2], t1748[2]), vbslq_u32(t1749[1], x5[3], t1748[3])];
    }
    res = t1748;
    return res;
}

#[inline(always)]
unsafe fn b29_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t1762: u64;
    if ((x0) >= (64u32)) {
        t1762 = !(0u64);
    } else {
        t1762 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t1762;
    return res;
}

#[inline(always)]
unsafe fn b29_s_f(l: &Launch, lid0: u32, x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t1763: u64;
    if ((x0) >= (64u32)) {
        t1763 = !(0u64);
    } else {
        t1763 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t1763;
    return res;
}

#[inline(always)]
unsafe fn b30_r0sss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1: u32, x2: u32, x3: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u64 = 0;
    let mut live = k;
    let mut res: u64 = 0;
    let t1764 = b28_r0ss_p(l, lid0, live, arg0, (((x1) as u64)).wrapping_add(((x2) as u64)), x3);
    x4 = t1764;
    let t1765: u64;
    if ((x3) >= (8u32)) {
        t1765 = x4;
    } else {
        let t1766 = b29_s_p(l, lid0, live, (8u32).wrapping_mul(x3));
        t1765 = ((x4) & (t1766));
    }
    res = t1765;
    return res;
}

#[inline(always)]
unsafe fn b30_r0sss_f(l: &Launch, lid0: u32, arg0: *const u8, x1: u32, x2: u32, x3: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u64 = 0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t1767 = b28_r0ss_f(l, lid0, arg0, (((x1) as u64)).wrapping_add(((x2) as u64)), x3);
    x4 = t1767;
    let t1768: u64;
    if ((x3) >= (8u32)) {
        t1768 = x4;
    } else {
        let t1769 = b29_s_f(l, lid0, (8u32).wrapping_mul(x3));
        t1768 = ((x4) & (t1769));
    }
    res = t1768;
    return res;
}

#[inline(always)]
unsafe fn b30_r0sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1: u32, x2: [uint32x4_t; 2], x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1770 = b28_r0al1s_p(l, lid0, live, arg0, ((((x1) as u64)) as usize as *const u8), x2, x3);
    x4 = t1770;
    let t1771: [uint32x4_t; 4];
    if ((x3) >= (8u32)) {
        t1771 = x4;
    } else {
        let t1772 = b29_s_p(l, lid0, live, (8u32).wrapping_mul(x3));
        let t1773: u64 = t1772;
        let t1774 = [vdupq_n_u32((t1773) as u32); 2];
        let t1775 = [vdupq_n_u32(((t1773) >> 32) as u32); 2];
        let t1776: [uint32x4_t; 4] = [vandq_u32(x4[0], t1774[0]), vandq_u32(x4[1], t1774[1]), vandq_u32(x4[2], t1775[0]), vandq_u32(x4[3], t1775[1])];
        t1771 = t1776;
    }
    res = t1771;
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
    let mut t1777: bool = !(arg6).is_null();
    if t1777 {
        let mut t1778: bool = ((l.group[0]) >= ((arg6.wrapping_add((((x7) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        if !t1778 {
            t1778 = ((l.group[1]) >= ((arg6.wrapping_add(((((x7).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        let mut t1779: bool = t1778;
        if !t1779 {
            t1779 = ((l.group[2]) >= ((arg6.wrapping_add(((((x7).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        t1777 = t1779;
    }
    if t1777 {
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
        let t1780: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
        let mut live = t1780;
        let gid0 = l.group[0] * l.block[0] + lid0;
        'chunk: {
            if (lid0 & 31u32) <= 24u32 { b0_r0r1r2r3r4sgu5_p(l, lid0, live, arg0, arg1, arg2, arg3, arg4, x5, lid0) } else { b0_r0r1r2r3r4sgi_p(l, lid0, live, arg0, arg1, arg2, arg3, arg4, x5, lid0) };
        }
    }
}
