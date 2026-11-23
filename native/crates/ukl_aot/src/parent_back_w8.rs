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

#[inline(always)]
unsafe fn b0_r0r1r2r3r4sgi_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, arg2: *const u8, arg3: *const u8, arg4: *const u8, x40: *const u8, x64: u32) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x40 as *const u32).read_unaligned();
    let mut x72: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x80: bool = false;
    let mut live = k;
    let t1: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x64), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x64), vld1q_u32(LANE[1].as_ptr()))];
    let t2 = [vdupq_n_u32((32u32).wrapping_sub(1u32)); 2];
    let t3: [uint32x4_t; 2] = [vandq_u32(t1[0], t2[0]), vandq_u32(t1[1], t2[1])];
    x72 = t3;
    let t4 = b1_r3sso4_p(l, lid0, live, arg3, (((a0).wrapping_add(1023u32)).checked_div(1024u32).unwrap_or(0)).wrapping_add(1u32), 0u32, arg4.wrapping_add((32u64).wrapping_mul(4u64) as usize));
    x80 = t4;
    if !(x80) {
        let t5 = b2_o4s_p(l, lid0, live, arg4.wrapping_add((96u64).wrapping_mul(4u64) as usize), 1u32);
    }
    let t6: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
    let t7: [uint32x4_t; 2] = [vsubq_u32(t6[0], t1[0]), vsubq_u32(t6[1], t1[1])];
    b3_r0r1r2r3ssllg_p(l, lid0, live, arg0, arg1, arg2, arg3, a0, x80, t7, x72);
}

#[inline(always)]
unsafe fn b0_r0r1r2r3r4sgi_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, arg2: *const u8, arg3: *const u8, arg4: *const u8, x40: *const u8, x64: u32) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x40 as *const u32).read_unaligned();
    let mut x72: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x80: bool = false;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let t8: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x64), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x64), vld1q_u32(LANE[1].as_ptr()))];
    let t9 = [vdupq_n_u32((32u32).wrapping_sub(1u32)); 2];
    let t10: [uint32x4_t; 2] = [vandq_u32(t8[0], t9[0]), vandq_u32(t8[1], t9[1])];
    x72 = t10;
    let t11 = b1_r3sso4_f(l, lid0, arg3, (((a0).wrapping_add(1023u32)).checked_div(1024u32).unwrap_or(0)).wrapping_add(1u32), 0u32, arg4.wrapping_add((32u64).wrapping_mul(4u64) as usize));
    x80 = t11;
    if !(x80) {
        let t12 = b2_o4s_f(l, lid0, arg4.wrapping_add((96u64).wrapping_mul(4u64) as usize), 1u32);
    }
    let t13: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
    let t14: [uint32x4_t; 2] = [vsubq_u32(t13[0], t8[0]), vsubq_u32(t13[1], t8[1])];
    b3_r0r1r2r3ssllg_f(l, lid0, arg0, arg1, arg2, arg3, a0, x80, t14, x72);
}

#[inline(always)]
unsafe fn b1_r3sso4_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg3: *const u8, x8: u32, x16: u32, arg4_x24: *const u8) -> bool {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x56: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: bool = false;
    x32 = (((x8).wrapping_add(256u32)).wrapping_sub(1u32)).checked_div(256u32).unwrap_or(0);
    let mut t15 = live;
    let mut t16 = [vdupq_n_u32(0); 2];
    loop {
        // Lanes part: each its own rounds.
        let t17 = [vdupq_n_u32(if true { u32::MAX } else { 0 }); 2];
        t15 = [vandq_u32(vbicq_u32(t15[0], t16[0]), t17[0]), vandq_u32(vbicq_u32(t15[1], t16[1]), t17[1])];
        if !(any(&t15)) { break; }
        let t19 = b4_o4_p(l, lid0, t15, arg4_x24.wrapping_add((0u64).wrapping_mul(4u64) as usize));
        let t20: [uint32x4_t; 2];
        if ((t19) < (x32)) {
            let t21 = b2_o4s_p(l, lid0, t15, arg4_x24.wrapping_add((0u64).wrapping_mul(4u64) as usize), 1u32);
            t20 = t21;
        } else {
            let t22 = [vdupq_n_u32(x32); 2];
            t20 = t22;
        }
        x40 = [vbslq_u32(t15[0], t20[0], x40[0]), vbslq_u32(t15[1], t20[1], x40[1])];
        let t23 = [vdupq_n_u32(x32); 2];
        let t24: [uint32x4_t; 2] = [vcgeq_u32(x40[0], t23[0]), vcgeq_u32(x40[1], t23[1])];
        let t25: [uint32x4_t; 2] = [vandq_u32(t15[0], t24[0]), vandq_u32(t15[1], t24[1])];
        if any(&t25) {
            t16 = [vorrq_u32(t16[0], t25[0]), vorrq_u32(t16[1], t25[1])];
        }
        let t26: [uint32x4_t; 2] = [vbicq_u32(vandq_u32(t15[0], live[0]), t16[0]), vbicq_u32(vandq_u32(t15[1], live[1]), t16[1])];
        if any(&t26) {
            let t27 = [vdupq_n_u32(256u32); 2];
            let t28: [uint32x4_t; 2] = [vmulq_u32(x40[0], t27[0]), vmulq_u32(x40[1], t27[1])];
            let t29: [uint32x4_t; 2] = [vaddq_u32(t28[0], t27[0]), vaddq_u32(t28[1], t27[1])];
            let t30 = [vdupq_n_u32(x8); 2];
            let t31: [uint32x4_t; 2] = [vcltq_u32(t29[0], t30[0]), vcltq_u32(t29[1], t30[1])];
            let mut t32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t33: [uint32x4_t; 2] = [vandq_u32(t26[0], t31[0]), vandq_u32(t26[1], t31[1])];
            let t34: [uint32x4_t; 2] = [vbicq_u32(t26[0], t31[0]), vbicq_u32(t26[1], t31[1])];
            {
                let t35: [uint32x4_t; 2] = [vmulq_u32(x40[0], t27[0]), vmulq_u32(x40[1], t27[1])];
                let t36: [uint32x4_t; 2] = [vaddq_u32(t35[0], t27[0]), vaddq_u32(t35[1], t27[1])];
                t32 = [vbslq_u32(t33[0], t36[0], t32[0]), vbslq_u32(t33[1], t36[1], t32[1])];
            }
            {
                let t37 = [vdupq_n_u32(x8); 2];
                t32 = [vbslq_u32(t34[0], t37[0], t32[0]), vbslq_u32(t34[1], t37[1], t32[1])];
            }
            x48 = t32;
            let t38: [uint32x4_t; 2] = [vmulq_u32(x40[0], t27[0]), vmulq_u32(x40[1], t27[1])];
            x56 = t38;
            let mut t39 = t26;
            loop {
                // Lanes part: each its own rounds.
                let t41: [uint32x4_t; 2] = [vcltq_u32(x56[0], x48[0]), vcltq_u32(x56[1], x48[1])];
                t39 = [vandq_u32(t39[0], t41[0]), vandq_u32(t39[1], t41[1])];
                if !(any(&t39)) { break; }
                let t43 = [vdupq_n_u32(x16); 2];
                let mut t44w = 0u32;
                let t44 = core::hint::black_box(&raw mut t44w);
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(t39[0]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<0>(x56[0]) as usize), t44).write(vgetq_lane_u32::<0>(t43[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(t39[0]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<1>(x56[0]) as usize), t44).write(vgetq_lane_u32::<1>(t43[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(t39[0]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<2>(x56[0]) as usize), t44).write(vgetq_lane_u32::<2>(t43[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(t39[0]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<3>(x56[0]) as usize), t44).write(vgetq_lane_u32::<3>(t43[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(t39[1]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<0>(x56[1]) as usize), t44).write(vgetq_lane_u32::<0>(t43[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(t39[1]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<1>(x56[1]) as usize), t44).write(vgetq_lane_u32::<1>(t43[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(t39[1]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<2>(x56[1]) as usize), t44).write(vgetq_lane_u32::<2>(t43[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(t39[1]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<3>(x56[1]) as usize), t44).write(vgetq_lane_u32::<3>(t43[1]));
                let t45: [uint32x4_t; 2] = [vandq_u32(t39[0], live[0]), vandq_u32(t39[1], live[1])];
                let t46 = x56;
                x56 = [vsubq_u32(x56[0], t45[0]), vsubq_u32(x56[1], t45[1])];
            }
            b5__p(l, lid0, t26);
            let t47 = b2_o4s_p(l, lid0, t26, arg4_x24.wrapping_add((32u64).wrapping_mul(4u64) as usize), 1u32);
        }
        let t48: [uint32x4_t; 2] = [vbicq_u32(vandq_u32(t15[0], live[0]), t16[0]), vbicq_u32(vandq_u32(t15[1], live[1]), t16[1])];
    }
    if any(&live) {
        let t49 = [vdupq_n_u32(0u32); 2];
        x40 = [vbslq_u32(live[0], t49[0], x40[0]), vbslq_u32(live[1], t49[1], x40[1])];
        let mut t50 = live;
        loop {
            // Lanes part: each its own rounds.
            let t52 = [vdupq_n_u32(65536u32); 2];
            let t53: [uint32x4_t; 2] = [vcltq_u32(x40[0], t52[0]), vcltq_u32(x40[1], t52[1])];
            t50 = [vandq_u32(t50[0], t53[0]), vandq_u32(t50[1], t53[1])];
            if !(any(&t50)) { break; }
            let t55 = b4_o4_p(l, lid0, t50, arg4_x24.wrapping_add((32u64).wrapping_mul(4u64) as usize));
            if ((t55) >= (x32)) {
                res = ((1i32) != 0);
                live = [vbicq_u32(live[0], t50[0]), vbicq_u32(live[1], t50[1])];
            }
            let t56: [uint32x4_t; 2] = [vandq_u32(t50[0], live[0]), vandq_u32(t50[1], live[1])];
            let t57 = [vdupq_n_u32(1u32); 2];
            let t58: [uint32x4_t; 2] = [vaddq_u32(x40[0], t57[0]), vaddq_u32(x40[1], t57[1])];
            let t59 = x40;
            x40 = t58;
            t50 = [vandq_u32(t50[0], live[0]), vandq_u32(t50[1], live[1])];
        }
        if any(&live) {
            res = ((0i32) != 0);
            live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
        }
    }
    res
}

#[inline(always)]
unsafe fn b1_r3sso4_f(l: &Launch, lid0: u32, arg3: *const u8, x8: u32, x16: u32, arg4_x24: *const u8) -> bool {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x56: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: bool = false;
    x32 = (((x8).wrapping_add(256u32)).wrapping_sub(1u32)).checked_div(256u32).unwrap_or(0);
    let mut t60 = live;
    let mut t61 = [vdupq_n_u32(0); 2];
    loop {
        // Lanes part: each its own rounds.
        let t62 = [vdupq_n_u32(if true { u32::MAX } else { 0 }); 2];
        t60 = [vandq_u32(vbicq_u32(t60[0], t61[0]), t62[0]), vandq_u32(vbicq_u32(t60[1], t61[1]), t62[1])];
        if !(any(&t60)) { break; }
        let t64 = b4_o4_p(l, lid0, t60, arg4_x24.wrapping_add((0u64).wrapping_mul(4u64) as usize));
        let t65: [uint32x4_t; 2];
        if ((t64) < (x32)) {
            let t66 = b2_o4s_p(l, lid0, t60, arg4_x24.wrapping_add((0u64).wrapping_mul(4u64) as usize), 1u32);
            t65 = t66;
        } else {
            let t67 = [vdupq_n_u32(x32); 2];
            t65 = t67;
        }
        x40 = [vbslq_u32(t60[0], t65[0], x40[0]), vbslq_u32(t60[1], t65[1], x40[1])];
        let t68 = [vdupq_n_u32(x32); 2];
        let t69: [uint32x4_t; 2] = [vcgeq_u32(x40[0], t68[0]), vcgeq_u32(x40[1], t68[1])];
        let t70: [uint32x4_t; 2] = [vandq_u32(t60[0], t69[0]), vandq_u32(t60[1], t69[1])];
        if any(&t70) {
            t61 = [vorrq_u32(t61[0], t70[0]), vorrq_u32(t61[1], t70[1])];
        }
        let t71: [uint32x4_t; 2] = [vbicq_u32(vandq_u32(t60[0], live[0]), t61[0]), vbicq_u32(vandq_u32(t60[1], live[1]), t61[1])];
        if any(&t71) {
            let t72 = [vdupq_n_u32(256u32); 2];
            let t73: [uint32x4_t; 2] = [vmulq_u32(x40[0], t72[0]), vmulq_u32(x40[1], t72[1])];
            let t74: [uint32x4_t; 2] = [vaddq_u32(t73[0], t72[0]), vaddq_u32(t73[1], t72[1])];
            let t75 = [vdupq_n_u32(x8); 2];
            let t76: [uint32x4_t; 2] = [vcltq_u32(t74[0], t75[0]), vcltq_u32(t74[1], t75[1])];
            let mut t77: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t78: [uint32x4_t; 2] = [vandq_u32(t71[0], t76[0]), vandq_u32(t71[1], t76[1])];
            let t79: [uint32x4_t; 2] = [vbicq_u32(t71[0], t76[0]), vbicq_u32(t71[1], t76[1])];
            {
                let t80: [uint32x4_t; 2] = [vmulq_u32(x40[0], t72[0]), vmulq_u32(x40[1], t72[1])];
                let t81: [uint32x4_t; 2] = [vaddq_u32(t80[0], t72[0]), vaddq_u32(t80[1], t72[1])];
                t77 = [vbslq_u32(t78[0], t81[0], t77[0]), vbslq_u32(t78[1], t81[1], t77[1])];
            }
            {
                let t82 = [vdupq_n_u32(x8); 2];
                t77 = [vbslq_u32(t79[0], t82[0], t77[0]), vbslq_u32(t79[1], t82[1], t77[1])];
            }
            x48 = t77;
            let t83: [uint32x4_t; 2] = [vmulq_u32(x40[0], t72[0]), vmulq_u32(x40[1], t72[1])];
            x56 = t83;
            let mut t84 = t71;
            loop {
                // Lanes part: each its own rounds.
                let t86: [uint32x4_t; 2] = [vcltq_u32(x56[0], x48[0]), vcltq_u32(x56[1], x48[1])];
                t84 = [vandq_u32(t84[0], t86[0]), vandq_u32(t84[1], t86[1])];
                if !(any(&t84)) { break; }
                let t88 = [vdupq_n_u32(x16); 2];
                let mut t89w = 0u32;
                let t89 = core::hint::black_box(&raw mut t89w);
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(t84[0]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<0>(x56[0]) as usize), t89).write(vgetq_lane_u32::<0>(t88[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(t84[0]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<1>(x56[0]) as usize), t89).write(vgetq_lane_u32::<1>(t88[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(t84[0]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<2>(x56[0]) as usize), t89).write(vgetq_lane_u32::<2>(t88[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(t84[0]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<3>(x56[0]) as usize), t89).write(vgetq_lane_u32::<3>(t88[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(t84[1]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<0>(x56[1]) as usize), t89).write(vgetq_lane_u32::<0>(t88[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(t84[1]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<1>(x56[1]) as usize), t89).write(vgetq_lane_u32::<1>(t88[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(t84[1]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<2>(x56[1]) as usize), t89).write(vgetq_lane_u32::<2>(t88[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(t84[1]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<3>(x56[1]) as usize), t89).write(vgetq_lane_u32::<3>(t88[1]));
                let t90: [uint32x4_t; 2] = [vandq_u32(t84[0], live[0]), vandq_u32(t84[1], live[1])];
                let t91 = x56;
                x56 = [vsubq_u32(x56[0], t90[0]), vsubq_u32(x56[1], t90[1])];
            }
            b5__p(l, lid0, t71);
            let t92 = b2_o4s_p(l, lid0, t71, arg4_x24.wrapping_add((32u64).wrapping_mul(4u64) as usize), 1u32);
        }
        let t93: [uint32x4_t; 2] = [vbicq_u32(vandq_u32(t60[0], live[0]), t61[0]), vbicq_u32(vandq_u32(t60[1], live[1]), t61[1])];
    }
    if any(&live) {
        let t94 = [vdupq_n_u32(0u32); 2];
        x40 = [vbslq_u32(live[0], t94[0], x40[0]), vbslq_u32(live[1], t94[1], x40[1])];
        let mut t95 = live;
        loop {
            // Lanes part: each its own rounds.
            let t97 = [vdupq_n_u32(65536u32); 2];
            let t98: [uint32x4_t; 2] = [vcltq_u32(x40[0], t97[0]), vcltq_u32(x40[1], t97[1])];
            t95 = [vandq_u32(t95[0], t98[0]), vandq_u32(t95[1], t98[1])];
            if !(any(&t95)) { break; }
            let t100 = b4_o4_p(l, lid0, t95, arg4_x24.wrapping_add((32u64).wrapping_mul(4u64) as usize));
            if ((t100) >= (x32)) {
                res = ((1i32) != 0);
                live = [vbicq_u32(live[0], t95[0]), vbicq_u32(live[1], t95[1])];
            }
            let t101: [uint32x4_t; 2] = [vandq_u32(t95[0], live[0]), vandq_u32(t95[1], live[1])];
            let t102 = [vdupq_n_u32(1u32); 2];
            let t103: [uint32x4_t; 2] = [vaddq_u32(x40[0], t102[0]), vaddq_u32(x40[1], t102[1])];
            let t104 = x40;
            x40 = t103;
            t95 = [vandq_u32(t95[0], live[0]), vandq_u32(t95[1], live[1])];
        }
        if any(&live) {
            res = ((0i32) != 0);
            live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
        }
    }
    res
}

#[inline(always)]
unsafe fn b2_o4s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg4_x0: *const u8, x8: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut t105 = [0u32; 8];
    for r in 0..2 { vst1q_u32(t105.as_mut_ptr().add(4 * r), live[r]); }
    let mut t106 = [0u32; 8];
    let mut t107 = 0u32;
    for j in 0..8 { t106[j] = t107; t107 += t105[j] & 1; }
    let t108: [uint32x4_t; 2] = [vld1q_u32(t106.as_ptr().add(0)), vld1q_u32(t106.as_ptr().add(4))];
    let t109 = AtomicU32::from_ptr(arg4_x0 as *mut u32).fetch_add((x8).wrapping_mul(t107), Relaxed);
    let t110: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(t109), t108[0], vdupq_n_u32(x8)), vmlaq_u32(vdupq_n_u32(t109), t108[1], vdupq_n_u32(x8))];
    res = t110;
    return res;
}

#[inline(always)]
unsafe fn b2_o4s_f(l: &Launch, lid0: u32, arg4_x0: *const u8, x8: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t111: [uint32x4_t; 2] = [vld1q_u32(LANE[0].as_ptr()), vld1q_u32(LANE[1].as_ptr())];
    let t112 = AtomicU32::from_ptr(arg4_x0 as *mut u32).fetch_add((x8).wrapping_mul(8u32), Relaxed);
    let t113: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(t112), t111[0], vdupq_n_u32(x8)), vmlaq_u32(vdupq_n_u32(t112), t111[1], vdupq_n_u32(x8))];
    res = t113;
    return res;
}

#[inline(always)]
unsafe fn b3_r0r1r2r3ssllg_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, arg2: *const u8, arg3: *const u8, x32: u32, x40: bool, x48: [uint32x4_t; 2], x56: [uint32x4_t; 2]) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x72: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x80: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut x88: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x96: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x104: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = k;
    if !(x40) {
        let t114: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t115 = [vdupq_n_u32(x32); 2];
        let t116: [uint32x4_t; 2] = [vcltq_u32(t114[0], t115[0]), vcltq_u32(t114[1], t115[1])];
        let t117: [uint32x4_t; 2] = [vandq_u32(live[0], t116[0]), vandq_u32(live[1], t116[1])];
        if any(&t117) {
            let t118 = b7_r0r1g_p(l, lid0, t117, arg0, arg1);
            b6_ag4r2l_p(l, lid0, t117, arg2, t118);
        }
        return;
    }
    if any(&live) {
        let t119 = [vdupq_n_u32(4294967295u32); 2];
        x72 = t119;
        let t120: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t121 = [vdupq_n_u32(x32); 2];
        let t122: [uint32x4_t; 2] = [vcltq_u32(t120[0], t121[0]), vcltq_u32(t120[1], t121[1])];
        let t123: [uint32x4_t; 2] = [vandq_u32(live[0], t122[0]), vandq_u32(live[1], t122[1])];
        if any(&t123) {
            let t124 = b8_r0r1g_p(l, lid0, t123, arg0, arg1);
            x80 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t123[0])))), t124[0], x80[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t123[0])))), t124[1], x80[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t123[1])))), t124[2], x80[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t123[1])))), t124[3], x80[3])];
            let t125 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
            let t126: [uint64x2_t; 4] = [vandq_u64(x80[0], t125[0]), vandq_u64(x80[1], t125[1]), vandq_u64(x80[2], t125[2]), vandq_u64(x80[3], t125[3])];
            let t127 = [vdupq_n_u64(0u64); 4];
            let t128: [uint64x2_t; 4] = [vceqq_u64(t126[0], t127[0]), vceqq_u64(t126[1], t127[1]), vceqq_u64(t126[2], t127[2]), vceqq_u64(t126[3], t127[3])];
            let t129: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t128[0]), vmovn_u64(t128[1])), vcombine_u32(vmovn_u64(t128[2]), vmovn_u64(t128[3]))];
            let t130: [uint32x4_t; 2] = [vmvnq_u32(t129[0]), vmvnq_u32(t129[1])];
            let mut t131: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t132: [uint32x4_t; 2] = [vandq_u32(t123[0], t130[0]), vandq_u32(t123[1], t130[1])];
            let t133: [uint32x4_t; 2] = [vbicq_u32(t123[0], t130[0]), vbicq_u32(t123[1], t130[1])];
            {
                let t134: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x80[0]), vmovn_u64(x80[1])), vcombine_u32(vmovn_u64(x80[2]), vmovn_u64(x80[3]))];
                let t135 = [vdupq_n_u32(2147483648u32); 2];
                let t136: [uint32x4_t; 2] = [vorrq_u32(t135[0], t134[0]), vorrq_u32(t135[1], t134[1])];
                t131 = [vbslq_u32(t132[0], t136[0], t131[0]), vbslq_u32(t132[1], t136[1], t131[1])];
            }
            {
                let t137: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x80[0]), vmovn_u64(x80[1])), vcombine_u32(vmovn_u64(x80[2]), vmovn_u64(x80[3]))];
                t131 = [vbslq_u32(t133[0], t137[0], t131[0]), vbslq_u32(t133[1], t137[1], t131[1])];
            }
            x72 = [vbslq_u32(t123[0], t131[0], x72[0]), vbslq_u32(t123[1], t131[1], x72[1])];
            b6_ag4r2l_p(l, lid0, t123, arg2, x72);
        }
        b9_r3gl_p(l, lid0, live, arg3, x56);
        let t138 = [vdupq_n_u32(x32); 2];
        let t139: [uint32x4_t; 2] = [vcgeq_u32(t120[0], t138[0]), vcgeq_u32(t120[1], t138[1])];
        let t140: [uint32x4_t; 2] = [vandq_u32(live[0], t139[0]), vandq_u32(live[1], t139[1])];
        if any(&t140) {
            live = [vbicq_u32(live[0], t140[0]), vbicq_u32(live[1], t140[1])];
        }
        if any(&live) {
            let t141 = [vdupq_n_u64(((0u32) as u64)); 4];
            x80 = t141;
            let mut t142 = live;
            loop {
                // Lanes part: each its own rounds.
                let t144: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x80[0]), vmovn_u64(x80[1])), vcombine_u32(vmovn_u64(x80[2]), vmovn_u64(x80[3]))];
                let t145 = [vdupq_n_u32(7u32); 2];
                let t146: [uint32x4_t; 2] = [vcltq_u32(t144[0], t145[0]), vcltq_u32(t144[1], t145[1])];
                let t147 = [vdupq_n_u32(2147483648u32); 2];
                let t148: [uint32x4_t; 2] = [vandq_u32(x72[0], t147[0]), vandq_u32(x72[1], t147[1])];
                let t149 = [vdupq_n_u32(0u32); 2];
                let t150: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t148[0], t149[0])), vmvnq_u32(vceqq_u32(t148[1], t149[1]))];
                let t151 = [vdupq_n_u32(4294967294u32); 2];
                let t152: [uint32x4_t; 2] = [vcltq_u32(x72[0], t151[0]), vcltq_u32(x72[1], t151[1])];
                let t153: [uint32x4_t; 2] = [vandq_u32(t150[0], t152[0]), vandq_u32(t150[1], t152[1])];
                let t154: [uint32x4_t; 2] = [vandq_u32(t146[0], t153[0]), vandq_u32(t146[1], t153[1])];
                t142 = [vandq_u32(t142[0], t154[0]), vandq_u32(t142[1], t154[1])];
                if !(any(&t142)) { break; }
                let t156 = [vdupq_n_u32((2147483648u32).wrapping_sub(1u32)); 2];
                let t157: [uint32x4_t; 2] = [vandq_u32(x72[0], t156[0]), vandq_u32(x72[1], t156[1])];
                x88 = t157;
                x96 = t151;
                let t158 = b10_r3ll_p(l, lid0, t142, arg3, x88, x48);
                let t159: [uint32x4_t; 2] = [vandq_u32(t142[0], t158[0]), vandq_u32(t142[1], t158[1])];
                if any(&t159) {
                    let t160 = b11_al4r2_p(l, lid0, t159, arg2, x88);
                    x96 = [vbslq_u32(t159[0], t160[0], x96[0]), vbslq_u32(t159[1], t160[1], x96[1])];
                }
                let t161: [uint32x4_t; 2] = [vceqq_u32(x96[0], t151[0]), vceqq_u32(x96[1], t151[1])];
                let t162: [uint32x4_t; 2] = [vandq_u32(t142[0], t161[0]), vandq_u32(t142[1], t161[1])];
                if any(&t162) {
                    let t163 = b8_r0r1l_p(l, lid0, t162, arg0, arg1, x88);
                    x104 = t163;
                    let t164 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
                    let t165: [uint64x2_t; 4] = [vandq_u64(x104[0], t164[0]), vandq_u64(x104[1], t164[1]), vandq_u64(x104[2], t164[2]), vandq_u64(x104[3], t164[3])];
                    let t166 = [vdupq_n_u64(0u64); 4];
                    let t167: [uint64x2_t; 4] = [vceqq_u64(t165[0], t166[0]), vceqq_u64(t165[1], t166[1]), vceqq_u64(t165[2], t166[2]), vceqq_u64(t165[3], t166[3])];
                    let t168: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t167[0]), vmovn_u64(t167[1])), vcombine_u32(vmovn_u64(t167[2]), vmovn_u64(t167[3]))];
                    let t169: [uint32x4_t; 2] = [vmvnq_u32(t168[0]), vmvnq_u32(t168[1])];
                    let mut t170: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                    let t171: [uint32x4_t; 2] = [vandq_u32(t162[0], t169[0]), vandq_u32(t162[1], t169[1])];
                    let t172: [uint32x4_t; 2] = [vbicq_u32(t162[0], t169[0]), vbicq_u32(t162[1], t169[1])];
                    {
                        let t173: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x104[0]), vmovn_u64(x104[1])), vcombine_u32(vmovn_u64(x104[2]), vmovn_u64(x104[3]))];
                        let t174: [uint32x4_t; 2] = [vorrq_u32(t147[0], t173[0]), vorrq_u32(t147[1], t173[1])];
                        t170 = [vbslq_u32(t171[0], t174[0], t170[0]), vbslq_u32(t171[1], t174[1], t170[1])];
                    }
                    {
                        let t175: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x104[0]), vmovn_u64(x104[1])), vcombine_u32(vmovn_u64(x104[2]), vmovn_u64(x104[3]))];
                        t170 = [vbslq_u32(t172[0], t175[0], t170[0]), vbslq_u32(t172[1], t175[1], t170[1])];
                    }
                    x96 = [vbslq_u32(t162[0], t170[0], x96[0]), vbslq_u32(t162[1], t170[1], x96[1])];
                }
                x72 = [vbslq_u32(t142[0], x96[0], x72[0]), vbslq_u32(t142[1], x96[1], x72[1])];
                let t176: [uint32x4_t; 2] = [vandq_u32(t142[0], live[0]), vandq_u32(t142[1], live[1])];
                let t177: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x80[0]), vmovn_u64(x80[1])), vcombine_u32(vmovn_u64(x80[2]), vmovn_u64(x80[3]))];
                let t178 = [vdupq_n_u32(1u32); 2];
                let t179: [uint32x4_t; 2] = [vaddq_u32(t177[0], t178[0]), vaddq_u32(t177[1], t178[1])];
                let t180 = x80;
                let t181: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t179[0])), vmovl_u32(vget_high_u32(t179[0])), vmovl_u32(vget_low_u32(t179[1])), vmovl_u32(vget_high_u32(t179[1]))];
                x80 = t181;
            }
            let t182 = [vdupq_n_u32(2147483648u32); 2];
            let t183: [uint32x4_t; 2] = [vandq_u32(x72[0], t182[0]), vandq_u32(x72[1], t182[1])];
            let t184 = [vdupq_n_u32(0u32); 2];
            let t185: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t183[0], t184[0])), vmvnq_u32(vceqq_u32(t183[1], t184[1]))];
            let t186 = [vdupq_n_u32(4294967294u32); 2];
            let t187: [uint32x4_t; 2] = [vcltq_u32(x72[0], t186[0]), vcltq_u32(x72[1], t186[1])];
            let t188: [uint32x4_t; 2] = [vandq_u32(t185[0], t187[0]), vandq_u32(t185[1], t187[1])];
            let t189: [uint32x4_t; 2] = [vandq_u32(live[0], t188[0]), vandq_u32(live[1], t188[1])];
            if any(&t189) {
                let t190 = [vdupq_n_u32((2147483648u32).wrapping_sub(1u32)); 2];
                let t191: [uint32x4_t; 2] = [vandq_u32(x72[0], t190[0]), vandq_u32(x72[1], t190[1])];
                let t192 = b7_r0r1l_p(l, lid0, t189, arg0, arg1, t191);
                x72 = [vbslq_u32(t189[0], t192[0], x72[0]), vbslq_u32(t189[1], t192[1], x72[1])];
            }
            b6_ag4r2l_p(l, lid0, live, arg2, x72);
        }
    }
}

#[inline(always)]
unsafe fn b3_r0r1r2r3ssllg_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, arg2: *const u8, arg3: *const u8, x32: u32, x40: bool, x48: [uint32x4_t; 2], x56: [uint32x4_t; 2]) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x72: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x80: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut x88: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x96: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x104: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    if !(x40) {
        if (((gid0) as u64 + 7) <= u32::MAX as u64 && ((gid0) as u64) >= ((x32) as u64)) {
        } else if ((gid0) as u64 + 7) < ((x32) as u64) {
            let t193 = b7_r0r1g_f(l, lid0, arg0, arg1);
            b6_ag4r2l_f(l, lid0, arg2, t193);
        } else {
            // Lanes part: the chunk's lanes on both sides of the bound.
            let t194: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
            let t195 = [vdupq_n_u32(x32); 2];
            let t196: [uint32x4_t; 2] = [vcltq_u32(t194[0], t195[0]), vcltq_u32(t194[1], t195[1])];
            let t197: [uint32x4_t; 2] = [vandq_u32(live[0], t196[0]), vandq_u32(live[1], t196[1])];
            if any(&t197) {
                let t198 = b7_r0r1g_p(l, lid0, t197, arg0, arg1);
                b6_ag4r2l_p(l, lid0, t197, arg2, t198);
            }
        }
        return;
    }
    if any(&live) {
        let t199 = [vdupq_n_u32(4294967295u32); 2];
        x72 = t199;
        let t200: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t201 = [vdupq_n_u32(x32); 2];
        let t202: [uint32x4_t; 2] = [vcltq_u32(t200[0], t201[0]), vcltq_u32(t200[1], t201[1])];
        let t203: [uint32x4_t; 2] = [vandq_u32(live[0], t202[0]), vandq_u32(live[1], t202[1])];
        if any(&t203) {
            let t204 = b8_r0r1g_p(l, lid0, t203, arg0, arg1);
            x80 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t203[0])))), t204[0], x80[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t203[0])))), t204[1], x80[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t203[1])))), t204[2], x80[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t203[1])))), t204[3], x80[3])];
            let t205 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
            let t206: [uint64x2_t; 4] = [vandq_u64(x80[0], t205[0]), vandq_u64(x80[1], t205[1]), vandq_u64(x80[2], t205[2]), vandq_u64(x80[3], t205[3])];
            let t207 = [vdupq_n_u64(0u64); 4];
            let t208: [uint64x2_t; 4] = [vceqq_u64(t206[0], t207[0]), vceqq_u64(t206[1], t207[1]), vceqq_u64(t206[2], t207[2]), vceqq_u64(t206[3], t207[3])];
            let t209: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t208[0]), vmovn_u64(t208[1])), vcombine_u32(vmovn_u64(t208[2]), vmovn_u64(t208[3]))];
            let t210: [uint32x4_t; 2] = [vmvnq_u32(t209[0]), vmvnq_u32(t209[1])];
            let mut t211: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t212: [uint32x4_t; 2] = [vandq_u32(t203[0], t210[0]), vandq_u32(t203[1], t210[1])];
            let t213: [uint32x4_t; 2] = [vbicq_u32(t203[0], t210[0]), vbicq_u32(t203[1], t210[1])];
            {
                let t214: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x80[0]), vmovn_u64(x80[1])), vcombine_u32(vmovn_u64(x80[2]), vmovn_u64(x80[3]))];
                let t215 = [vdupq_n_u32(2147483648u32); 2];
                let t216: [uint32x4_t; 2] = [vorrq_u32(t215[0], t214[0]), vorrq_u32(t215[1], t214[1])];
                t211 = [vbslq_u32(t212[0], t216[0], t211[0]), vbslq_u32(t212[1], t216[1], t211[1])];
            }
            {
                let t217: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x80[0]), vmovn_u64(x80[1])), vcombine_u32(vmovn_u64(x80[2]), vmovn_u64(x80[3]))];
                t211 = [vbslq_u32(t213[0], t217[0], t211[0]), vbslq_u32(t213[1], t217[1], t211[1])];
            }
            x72 = [vbslq_u32(t203[0], t211[0], x72[0]), vbslq_u32(t203[1], t211[1], x72[1])];
            b6_ag4r2l_p(l, lid0, t203, arg2, x72);
        }
        b9_r3gl_p(l, lid0, live, arg3, x56);
        let t218 = [vdupq_n_u32(x32); 2];
        let t219: [uint32x4_t; 2] = [vcgeq_u32(t200[0], t218[0]), vcgeq_u32(t200[1], t218[1])];
        let t220: [uint32x4_t; 2] = [vandq_u32(live[0], t219[0]), vandq_u32(live[1], t219[1])];
        if any(&t220) {
            live = [vbicq_u32(live[0], t220[0]), vbicq_u32(live[1], t220[1])];
        }
        if any(&live) {
            let t221 = [vdupq_n_u64(((0u32) as u64)); 4];
            x80 = t221;
            let mut t222 = live;
            loop {
                // Lanes part: each its own rounds.
                let t224: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x80[0]), vmovn_u64(x80[1])), vcombine_u32(vmovn_u64(x80[2]), vmovn_u64(x80[3]))];
                let t225 = [vdupq_n_u32(7u32); 2];
                let t226: [uint32x4_t; 2] = [vcltq_u32(t224[0], t225[0]), vcltq_u32(t224[1], t225[1])];
                let t227 = [vdupq_n_u32(2147483648u32); 2];
                let t228: [uint32x4_t; 2] = [vandq_u32(x72[0], t227[0]), vandq_u32(x72[1], t227[1])];
                let t229 = [vdupq_n_u32(0u32); 2];
                let t230: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t228[0], t229[0])), vmvnq_u32(vceqq_u32(t228[1], t229[1]))];
                let t231 = [vdupq_n_u32(4294967294u32); 2];
                let t232: [uint32x4_t; 2] = [vcltq_u32(x72[0], t231[0]), vcltq_u32(x72[1], t231[1])];
                let t233: [uint32x4_t; 2] = [vandq_u32(t230[0], t232[0]), vandq_u32(t230[1], t232[1])];
                let t234: [uint32x4_t; 2] = [vandq_u32(t226[0], t233[0]), vandq_u32(t226[1], t233[1])];
                t222 = [vandq_u32(t222[0], t234[0]), vandq_u32(t222[1], t234[1])];
                if !(any(&t222)) { break; }
                let t236 = [vdupq_n_u32((2147483648u32).wrapping_sub(1u32)); 2];
                let t237: [uint32x4_t; 2] = [vandq_u32(x72[0], t236[0]), vandq_u32(x72[1], t236[1])];
                x88 = t237;
                x96 = t231;
                let t238 = b10_r3ll_p(l, lid0, t222, arg3, x88, x48);
                let t239: [uint32x4_t; 2] = [vandq_u32(t222[0], t238[0]), vandq_u32(t222[1], t238[1])];
                if any(&t239) {
                    let t240 = b11_al4r2_p(l, lid0, t239, arg2, x88);
                    x96 = [vbslq_u32(t239[0], t240[0], x96[0]), vbslq_u32(t239[1], t240[1], x96[1])];
                }
                let t241: [uint32x4_t; 2] = [vceqq_u32(x96[0], t231[0]), vceqq_u32(x96[1], t231[1])];
                let t242: [uint32x4_t; 2] = [vandq_u32(t222[0], t241[0]), vandq_u32(t222[1], t241[1])];
                if any(&t242) {
                    let t243 = b8_r0r1l_p(l, lid0, t242, arg0, arg1, x88);
                    x104 = t243;
                    let t244 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
                    let t245: [uint64x2_t; 4] = [vandq_u64(x104[0], t244[0]), vandq_u64(x104[1], t244[1]), vandq_u64(x104[2], t244[2]), vandq_u64(x104[3], t244[3])];
                    let t246 = [vdupq_n_u64(0u64); 4];
                    let t247: [uint64x2_t; 4] = [vceqq_u64(t245[0], t246[0]), vceqq_u64(t245[1], t246[1]), vceqq_u64(t245[2], t246[2]), vceqq_u64(t245[3], t246[3])];
                    let t248: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t247[0]), vmovn_u64(t247[1])), vcombine_u32(vmovn_u64(t247[2]), vmovn_u64(t247[3]))];
                    let t249: [uint32x4_t; 2] = [vmvnq_u32(t248[0]), vmvnq_u32(t248[1])];
                    let mut t250: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                    let t251: [uint32x4_t; 2] = [vandq_u32(t242[0], t249[0]), vandq_u32(t242[1], t249[1])];
                    let t252: [uint32x4_t; 2] = [vbicq_u32(t242[0], t249[0]), vbicq_u32(t242[1], t249[1])];
                    {
                        let t253: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x104[0]), vmovn_u64(x104[1])), vcombine_u32(vmovn_u64(x104[2]), vmovn_u64(x104[3]))];
                        let t254: [uint32x4_t; 2] = [vorrq_u32(t227[0], t253[0]), vorrq_u32(t227[1], t253[1])];
                        t250 = [vbslq_u32(t251[0], t254[0], t250[0]), vbslq_u32(t251[1], t254[1], t250[1])];
                    }
                    {
                        let t255: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x104[0]), vmovn_u64(x104[1])), vcombine_u32(vmovn_u64(x104[2]), vmovn_u64(x104[3]))];
                        t250 = [vbslq_u32(t252[0], t255[0], t250[0]), vbslq_u32(t252[1], t255[1], t250[1])];
                    }
                    x96 = [vbslq_u32(t242[0], t250[0], x96[0]), vbslq_u32(t242[1], t250[1], x96[1])];
                }
                x72 = [vbslq_u32(t222[0], x96[0], x72[0]), vbslq_u32(t222[1], x96[1], x72[1])];
                let t256: [uint32x4_t; 2] = [vandq_u32(t222[0], live[0]), vandq_u32(t222[1], live[1])];
                let t257: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x80[0]), vmovn_u64(x80[1])), vcombine_u32(vmovn_u64(x80[2]), vmovn_u64(x80[3]))];
                let t258 = [vdupq_n_u32(1u32); 2];
                let t259: [uint32x4_t; 2] = [vaddq_u32(t257[0], t258[0]), vaddq_u32(t257[1], t258[1])];
                let t260 = x80;
                let t261: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t259[0])), vmovl_u32(vget_high_u32(t259[0])), vmovl_u32(vget_low_u32(t259[1])), vmovl_u32(vget_high_u32(t259[1]))];
                x80 = t261;
            }
            let t262 = [vdupq_n_u32(2147483648u32); 2];
            let t263: [uint32x4_t; 2] = [vandq_u32(x72[0], t262[0]), vandq_u32(x72[1], t262[1])];
            let t264 = [vdupq_n_u32(0u32); 2];
            let t265: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t263[0], t264[0])), vmvnq_u32(vceqq_u32(t263[1], t264[1]))];
            let t266 = [vdupq_n_u32(4294967294u32); 2];
            let t267: [uint32x4_t; 2] = [vcltq_u32(x72[0], t266[0]), vcltq_u32(x72[1], t266[1])];
            let t268: [uint32x4_t; 2] = [vandq_u32(t265[0], t267[0]), vandq_u32(t265[1], t267[1])];
            let t269: [uint32x4_t; 2] = [vandq_u32(live[0], t268[0]), vandq_u32(live[1], t268[1])];
            if any(&t269) {
                let t270 = [vdupq_n_u32((2147483648u32).wrapping_sub(1u32)); 2];
                let t271: [uint32x4_t; 2] = [vandq_u32(x72[0], t270[0]), vandq_u32(x72[1], t270[1])];
                let t272 = b7_r0r1l_p(l, lid0, t269, arg0, arg1, t271);
                x72 = [vbslq_u32(t269[0], t272[0], x72[0]), vbslq_u32(t269[1], t272[1], x72[1])];
            }
            b6_ag4r2l_p(l, lid0, live, arg2, x72);
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
unsafe fn b6_ag4r2l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg2: *const u8, x8: [uint32x4_t; 2]) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let t273 = (arg2 as *mut u32).add((gid0) as usize);
    if all(&live) {
        vst1q_u32(t273.add(0), x8[0]);
        vst1q_u32(t273.add(4), x8[1]);
    } else {
        let mut t274w = 0u32;
        let t274 = core::hint::black_box(&raw mut t274w);
        core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t273.add(0), t274).write(vgetq_lane_u32::<0>(x8[0]));
        core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t273.add(1), t274).write(vgetq_lane_u32::<1>(x8[0]));
        core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t273.add(2), t274).write(vgetq_lane_u32::<2>(x8[0]));
        core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t273.add(3), t274).write(vgetq_lane_u32::<3>(x8[0]));
        core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t273.add(4), t274).write(vgetq_lane_u32::<0>(x8[1]));
        core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t273.add(5), t274).write(vgetq_lane_u32::<1>(x8[1]));
        core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t273.add(6), t274).write(vgetq_lane_u32::<2>(x8[1]));
        core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t273.add(7), t274).write(vgetq_lane_u32::<3>(x8[1]));
    }
}

#[inline(always)]
unsafe fn b6_ag4r2l_f(l: &Launch, lid0: u32, arg2: *const u8, x8: [uint32x4_t; 2]) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let t275 = (arg2 as *mut u32).add((gid0) as usize);
    vst1q_u32(t275.add(0), x8[0]);
    vst1q_u32(t275.add(4), x8[1]);
}

#[inline(always)]
unsafe fn b7_r0r1g_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t276 = b8_r0r1g_p(l, lid0, live, arg0, arg1);
    x24 = t276;
    let mut t277 = live;
    loop {
        // Lanes part: each its own rounds.
        let t279 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
        let t280: [uint64x2_t; 4] = [vandq_u64(x24[0], t279[0]), vandq_u64(x24[1], t279[1]), vandq_u64(x24[2], t279[2]), vandq_u64(x24[3], t279[3])];
        let t281 = [vdupq_n_u64(0u64); 4];
        let t282: [uint64x2_t; 4] = [vceqq_u64(t280[0], t281[0]), vceqq_u64(t280[1], t281[1]), vceqq_u64(t280[2], t281[2]), vceqq_u64(t280[3], t281[3])];
        let t283: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t282[0]), vmovn_u64(t282[1])), vcombine_u32(vmovn_u64(t282[2]), vmovn_u64(t282[3]))];
        let t284: [uint32x4_t; 2] = [vmvnq_u32(t283[0]), vmvnq_u32(t283[1])];
        t277 = [vandq_u32(t277[0], t284[0]), vandq_u32(t277[1], t284[1])];
        if !(any(&t277)) { break; }
        let t286: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x24[0]), vmovn_u64(x24[1])), vcombine_u32(vmovn_u64(x24[2]), vmovn_u64(x24[3]))];
        let t287 = b8_r0r1l_p(l, lid0, t277, arg0, arg1, t286);
        x24 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t277[0])))), t287[0], x24[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t277[0])))), t287[1], x24[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t277[1])))), t287[2], x24[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t277[1])))), t287[3], x24[3])];
    }
    let t288: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x24[0]), vmovn_u64(x24[1])), vcombine_u32(vmovn_u64(x24[2]), vmovn_u64(x24[3]))];
    res = t288;
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1g_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t289 = b8_r0r1g_f(l, lid0, arg0, arg1);
    x24 = t289;
    let mut t296 = live;
    loop {
        // Lanes part: each its own rounds.
        let t298 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
        let t299: [uint64x2_t; 4] = [vandq_u64(x24[0], t298[0]), vandq_u64(x24[1], t298[1]), vandq_u64(x24[2], t298[2]), vandq_u64(x24[3], t298[3])];
        let t300 = [vdupq_n_u64(0u64); 4];
        let t301: [uint64x2_t; 4] = [vceqq_u64(t299[0], t300[0]), vceqq_u64(t299[1], t300[1]), vceqq_u64(t299[2], t300[2]), vceqq_u64(t299[3], t300[3])];
        let t302: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t301[0]), vmovn_u64(t301[1])), vcombine_u32(vmovn_u64(t301[2]), vmovn_u64(t301[3]))];
        let t303: [uint32x4_t; 2] = [vmvnq_u32(t302[0]), vmvnq_u32(t302[1])];
        t296 = [vandq_u32(t296[0], t303[0]), vandq_u32(t296[1], t303[1])];
        if !(any(&t296)) { break; }
        let t305: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x24[0]), vmovn_u64(x24[1])), vcombine_u32(vmovn_u64(x24[2]), vmovn_u64(x24[3]))];
        let t306 = b8_r0r1l_p(l, lid0, t296, arg0, arg1, t305);
        x24 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t296[0])))), t306[0], x24[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t296[0])))), t306[1], x24[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t296[1])))), t306[2], x24[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t296[1])))), t306[3], x24[3])];
    }
    let t307: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x24[0]), vmovn_u64(x24[1])), vcombine_u32(vmovn_u64(x24[2]), vmovn_u64(x24[3]))];
    res = t307;
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t308 = b8_r0r1l_p(l, lid0, live, arg0, arg1, x16);
    x24 = t308;
    let mut t309 = live;
    loop {
        // Lanes part: each its own rounds.
        let t311 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
        let t312: [uint64x2_t; 4] = [vandq_u64(x24[0], t311[0]), vandq_u64(x24[1], t311[1]), vandq_u64(x24[2], t311[2]), vandq_u64(x24[3], t311[3])];
        let t313 = [vdupq_n_u64(0u64); 4];
        let t314: [uint64x2_t; 4] = [vceqq_u64(t312[0], t313[0]), vceqq_u64(t312[1], t313[1]), vceqq_u64(t312[2], t313[2]), vceqq_u64(t312[3], t313[3])];
        let t315: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t314[0]), vmovn_u64(t314[1])), vcombine_u32(vmovn_u64(t314[2]), vmovn_u64(t314[3]))];
        let t316: [uint32x4_t; 2] = [vmvnq_u32(t315[0]), vmvnq_u32(t315[1])];
        t309 = [vandq_u32(t309[0], t316[0]), vandq_u32(t309[1], t316[1])];
        if !(any(&t309)) { break; }
        let t318: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x24[0]), vmovn_u64(x24[1])), vcombine_u32(vmovn_u64(x24[2]), vmovn_u64(x24[3]))];
        let t319 = b8_r0r1l_p(l, lid0, t309, arg0, arg1, t318);
        x24 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t309[0])))), t319[0], x24[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t309[0])))), t319[1], x24[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t309[1])))), t319[2], x24[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t309[1])))), t319[3], x24[3])];
    }
    let t320: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x24[0]), vmovn_u64(x24[1])), vcombine_u32(vmovn_u64(x24[2]), vmovn_u64(x24[3]))];
    res = t320;
    return res;
}

#[inline(always)]
unsafe fn b8_r0r1g_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x32: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t321: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
    let t322 = [vdupq_n_u32(0u32); 2];
    let t323: [uint32x4_t; 2] = [vceqq_u32(t321[0], t322[0]), vceqq_u32(t321[1], t322[1])];
    let t324: [uint32x4_t; 2] = [vandq_u32(live[0], t323[0]), vandq_u32(live[1], t323[1])];
    if any(&t324) {
        let t325 = [vdupq_n_u64(4294967295u64); 4];
        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t324[0])))), t325[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t324[0])))), t325[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t324[1])))), t325[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t324[1])))), t325[3], res[3])];
        live = [vbicq_u32(live[0], t324[0]), vbicq_u32(live[1], t324[1])];
    }
    if any(&live) {
        let t326 = [vdupq_n_u32(1u32); 2];
        let t327: [uint32x4_t; 2] = [vsubq_u32(t321[0], t326[0]), vsubq_u32(t321[1], t326[1])];
        x24 = [vbslq_u32(live[0], t327[0], x24[0]), vbslq_u32(live[1], t327[1], x24[1])];
        let t328 = b12_r0r1l_p(l, lid0, live, arg0, arg1, x24);
        x32 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t328[0], x32[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t328[1], x32[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t328[2], x32[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t328[3], x32[3])];
        let t329 = [vdupq_n_u64(1u64); 4];
        let t330: [uint64x2_t; 4] = [vandq_u64(x32[0], t329[0]), vandq_u64(x32[1], t329[1]), vandq_u64(x32[2], t329[2]), vandq_u64(x32[3], t329[3])];
        let t331 = [vdupq_n_u64(0u64); 4];
        let t332: [uint64x2_t; 4] = [vceqq_u64(t330[0], t331[0]), vceqq_u64(t330[1], t331[1]), vceqq_u64(t330[2], t331[2]), vceqq_u64(t330[3], t331[3])];
        let t333: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t332[0]), vmovn_u64(t332[1])), vcombine_u32(vmovn_u64(t332[2]), vmovn_u64(t332[3]))];
        let t334: [uint32x4_t; 2] = [vandq_u32(live[0], t333[0]), vandq_u32(live[1], t333[1])];
        if any(&t334) {
            let t335: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x24[0])), vmovl_u32(vget_high_u32(x24[0])), vmovl_u32(vget_low_u32(x24[1])), vmovl_u32(vget_high_u32(x24[1]))];
            res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t334[0])))), t335[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t334[0])))), t335[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t334[1])))), t335[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t334[1])))), t335[3], res[3])];
            live = [vbicq_u32(live[0], t334[0]), vbicq_u32(live[1], t334[1])];
        }
        if any(&live) {
            let t336: [uint64x2_t; 4] = [vshlq_u64(x32[0], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[1], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[2], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[3], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64)))];
            let t337: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t336[0]), vmovn_u64(t336[1])), vcombine_u32(vmovn_u64(t336[2]), vmovn_u64(t336[3]))];
            x40 = [vbslq_u32(live[0], t337[0], x40[0]), vbslq_u32(live[1], t337[1], x40[1])];
            let t338: [uint32x4_t; 2] = [vsubq_u32(x40[0], t326[0]), vsubq_u32(x40[1], t326[1])];
            let t339: [uint32x4_t; 2] = [vcgeq_u32(t338[0], t321[0]), vcgeq_u32(t338[1], t321[1])];
            let t340: [uint32x4_t; 2] = [vandq_u32(live[0], t339[0]), vandq_u32(live[1], t339[1])];
            if any(&t340) {
                let t341 = [vdupq_n_u64(4294967295u64); 4];
                res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t340[0])))), t341[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t340[0])))), t341[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t340[1])))), t341[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t340[1])))), t341[3], res[3])];
                live = [vbicq_u32(live[0], t340[0]), vbicq_u32(live[1], t340[1])];
            }
            if any(&live) {
                let t342: [uint32x4_t; 2] = [vsubq_u32(t321[0], x40[0]), vsubq_u32(t321[1], x40[1])];
                x48 = [vbslq_u32(live[0], t342[0], x48[0]), vbslq_u32(live[1], t342[1], x48[1])];
                let t343 = [vdupq_n_u64(2u64); 4];
                let t344: [uint64x2_t; 4] = [vandq_u64(x32[0], t343[0]), vandq_u64(x32[1], t343[1]), vandq_u64(x32[2], t343[2]), vandq_u64(x32[3], t343[3])];
                let t345: [uint64x2_t; 4] = [vceqq_u64(t344[0], t331[0]), vceqq_u64(t344[1], t331[1]), vceqq_u64(t344[2], t331[2]), vceqq_u64(t344[3], t331[3])];
                let t346: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t345[0]), vmovn_u64(t345[1])), vcombine_u32(vmovn_u64(t345[2]), vmovn_u64(t345[3]))];
                let t347: [uint32x4_t; 2] = [vmvnq_u32(t346[0]), vmvnq_u32(t346[1])];
                let t348: [uint32x4_t; 2] = [vandq_u32(live[0], t347[0]), vandq_u32(live[1], t347[1])];
                if any(&t348) {
                    let t349: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                    res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t348[0])))), t349[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t348[0])))), t349[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t348[1])))), t349[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t348[1])))), t349[3], res[3])];
                    live = [vbicq_u32(live[0], t348[0]), vbicq_u32(live[1], t348[1])];
                }
                if any(&live) {
                    let t350: [uint32x4_t; 2] = [vceqq_u32(x48[0], t322[0]), vceqq_u32(x48[1], t322[1])];
                    let t351: [uint32x4_t; 2] = [vandq_u32(live[0], t350[0]), vandq_u32(live[1], t350[1])];
                    if any(&t351) {
                        let t352 = [vdupq_n_u64(4294967295u64); 4];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t351[0])))), t352[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t351[0])))), t352[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t351[1])))), t352[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t351[1])))), t352[3], res[3])];
                        live = [vbicq_u32(live[0], t351[0]), vbicq_u32(live[1], t351[1])];
                    }
                    if any(&live) {
                        let t353 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
                        let t354: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                        let t355: [uint64x2_t; 4] = [vorrq_u64(t353[0], t354[0]), vorrq_u64(t353[1], t354[1]), vorrq_u64(t353[2], t354[2]), vorrq_u64(t353[3], t354[3])];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t355[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t355[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t355[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t355[3], res[3])];
                        live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
                    }
                }
            }
        }
    }
    res
}

#[inline(always)]
unsafe fn b8_r0r1g_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x32: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    if (((gid0) as u64 + 7) <= u32::MAX as u64 && (((0u32) as u64) < ((gid0) as u64) || ((0u32) as u64) > ((gid0) as u64 + 7))) {
        let t356: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t357 = [vdupq_n_u32(1u32); 2];
        let t358: [uint32x4_t; 2] = [vsubq_u32(t356[0], t357[0]), vsubq_u32(t356[1], t357[1])];
        x24 = t358;
        let t359 = b12_r0r1l_f(l, lid0, arg0, arg1, x24);
        x32 = t359;
        let t360 = [vdupq_n_u64(1u64); 4];
        let t361: [uint64x2_t; 4] = [vandq_u64(x32[0], t360[0]), vandq_u64(x32[1], t360[1]), vandq_u64(x32[2], t360[2]), vandq_u64(x32[3], t360[3])];
        let t362 = [vdupq_n_u64(0u64); 4];
        let t363: [uint64x2_t; 4] = [vceqq_u64(t361[0], t362[0]), vceqq_u64(t361[1], t362[1]), vceqq_u64(t361[2], t362[2]), vceqq_u64(t361[3], t362[3])];
        let t364: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t363[0]), vmovn_u64(t363[1])), vcombine_u32(vmovn_u64(t363[2]), vmovn_u64(t363[3]))];
        if any(&t364) {
            let t365: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x24[0])), vmovl_u32(vget_high_u32(x24[0])), vmovl_u32(vget_low_u32(x24[1])), vmovl_u32(vget_high_u32(x24[1]))];
            res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t364[0])))), t365[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t364[0])))), t365[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t364[1])))), t365[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t364[1])))), t365[3], res[3])];
            live = [vbicq_u32(live[0], t364[0]), vbicq_u32(live[1], t364[1])];
        }
        if any(&live) {
            let t366: [uint64x2_t; 4] = [vshlq_u64(x32[0], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[1], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[2], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[3], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64)))];
            let t367: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t366[0]), vmovn_u64(t366[1])), vcombine_u32(vmovn_u64(t366[2]), vmovn_u64(t366[3]))];
            x40 = [vbslq_u32(live[0], t367[0], x40[0]), vbslq_u32(live[1], t367[1], x40[1])];
            let t368: [uint32x4_t; 2] = [vsubq_u32(x40[0], t357[0]), vsubq_u32(x40[1], t357[1])];
            let t369: [uint32x4_t; 2] = [vcgeq_u32(t368[0], t356[0]), vcgeq_u32(t368[1], t356[1])];
            let t370: [uint32x4_t; 2] = [vandq_u32(live[0], t369[0]), vandq_u32(live[1], t369[1])];
            if any(&t370) {
                let t371 = [vdupq_n_u64(4294967295u64); 4];
                res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t370[0])))), t371[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t370[0])))), t371[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t370[1])))), t371[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t370[1])))), t371[3], res[3])];
                live = [vbicq_u32(live[0], t370[0]), vbicq_u32(live[1], t370[1])];
            }
            if any(&live) {
                let t372: [uint32x4_t; 2] = [vsubq_u32(t356[0], x40[0]), vsubq_u32(t356[1], x40[1])];
                x48 = [vbslq_u32(live[0], t372[0], x48[0]), vbslq_u32(live[1], t372[1], x48[1])];
                let t373 = [vdupq_n_u64(2u64); 4];
                let t374: [uint64x2_t; 4] = [vandq_u64(x32[0], t373[0]), vandq_u64(x32[1], t373[1]), vandq_u64(x32[2], t373[2]), vandq_u64(x32[3], t373[3])];
                let t375: [uint64x2_t; 4] = [vceqq_u64(t374[0], t362[0]), vceqq_u64(t374[1], t362[1]), vceqq_u64(t374[2], t362[2]), vceqq_u64(t374[3], t362[3])];
                let t376: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t375[0]), vmovn_u64(t375[1])), vcombine_u32(vmovn_u64(t375[2]), vmovn_u64(t375[3]))];
                let t377: [uint32x4_t; 2] = [vmvnq_u32(t376[0]), vmvnq_u32(t376[1])];
                let t378: [uint32x4_t; 2] = [vandq_u32(live[0], t377[0]), vandq_u32(live[1], t377[1])];
                if any(&t378) {
                    let t379: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                    res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t378[0])))), t379[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t378[0])))), t379[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t378[1])))), t379[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t378[1])))), t379[3], res[3])];
                    live = [vbicq_u32(live[0], t378[0]), vbicq_u32(live[1], t378[1])];
                }
                if any(&live) {
                    let t380 = [vdupq_n_u32(0u32); 2];
                    let t381: [uint32x4_t; 2] = [vceqq_u32(x48[0], t380[0]), vceqq_u32(x48[1], t380[1])];
                    let t382: [uint32x4_t; 2] = [vandq_u32(live[0], t381[0]), vandq_u32(live[1], t381[1])];
                    if any(&t382) {
                        let t383 = [vdupq_n_u64(4294967295u64); 4];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t382[0])))), t383[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t382[0])))), t383[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t382[1])))), t383[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t382[1])))), t383[3], res[3])];
                        live = [vbicq_u32(live[0], t382[0]), vbicq_u32(live[1], t382[1])];
                    }
                    if any(&live) {
                        let t384 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
                        let t385: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                        let t386: [uint64x2_t; 4] = [vorrq_u64(t384[0], t385[0]), vorrq_u64(t384[1], t385[1]), vorrq_u64(t384[2], t385[2]), vorrq_u64(t384[3], t385[3])];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t386[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t386[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t386[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t386[3], res[3])];
                        live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
                    }
                }
            }
        }
    } else if false {
        let t387 = [vdupq_n_u64(4294967295u64); 4];
        res = t387;
        return res;
    } else {
        // Lanes part: the chunk's lanes on both sides of the bound.
        let t388: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t389 = [vdupq_n_u32(0u32); 2];
        let t390: [uint32x4_t; 2] = [vceqq_u32(t388[0], t389[0]), vceqq_u32(t388[1], t389[1])];
        let t391: [uint32x4_t; 2] = [vandq_u32(live[0], t390[0]), vandq_u32(live[1], t390[1])];
        if any(&t391) {
            let t392 = [vdupq_n_u64(4294967295u64); 4];
            res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t391[0])))), t392[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t391[0])))), t392[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t391[1])))), t392[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t391[1])))), t392[3], res[3])];
            live = [vbicq_u32(live[0], t391[0]), vbicq_u32(live[1], t391[1])];
        }
        if any(&live) {
            let t393 = [vdupq_n_u32(1u32); 2];
            let t394: [uint32x4_t; 2] = [vsubq_u32(t388[0], t393[0]), vsubq_u32(t388[1], t393[1])];
            x24 = [vbslq_u32(live[0], t394[0], x24[0]), vbslq_u32(live[1], t394[1], x24[1])];
            let t395 = b12_r0r1l_p(l, lid0, live, arg0, arg1, x24);
            x32 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t395[0], x32[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t395[1], x32[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t395[2], x32[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t395[3], x32[3])];
            let t396 = [vdupq_n_u64(1u64); 4];
            let t397: [uint64x2_t; 4] = [vandq_u64(x32[0], t396[0]), vandq_u64(x32[1], t396[1]), vandq_u64(x32[2], t396[2]), vandq_u64(x32[3], t396[3])];
            let t398 = [vdupq_n_u64(0u64); 4];
            let t399: [uint64x2_t; 4] = [vceqq_u64(t397[0], t398[0]), vceqq_u64(t397[1], t398[1]), vceqq_u64(t397[2], t398[2]), vceqq_u64(t397[3], t398[3])];
            let t400: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t399[0]), vmovn_u64(t399[1])), vcombine_u32(vmovn_u64(t399[2]), vmovn_u64(t399[3]))];
            let t401: [uint32x4_t; 2] = [vandq_u32(live[0], t400[0]), vandq_u32(live[1], t400[1])];
            if any(&t401) {
                let t402: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x24[0])), vmovl_u32(vget_high_u32(x24[0])), vmovl_u32(vget_low_u32(x24[1])), vmovl_u32(vget_high_u32(x24[1]))];
                res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t401[0])))), t402[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t401[0])))), t402[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t401[1])))), t402[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t401[1])))), t402[3], res[3])];
                live = [vbicq_u32(live[0], t401[0]), vbicq_u32(live[1], t401[1])];
            }
            if any(&live) {
                let t403: [uint64x2_t; 4] = [vshlq_u64(x32[0], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[1], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[2], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[3], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64)))];
                let t404: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t403[0]), vmovn_u64(t403[1])), vcombine_u32(vmovn_u64(t403[2]), vmovn_u64(t403[3]))];
                x40 = [vbslq_u32(live[0], t404[0], x40[0]), vbslq_u32(live[1], t404[1], x40[1])];
                let t405: [uint32x4_t; 2] = [vsubq_u32(x40[0], t393[0]), vsubq_u32(x40[1], t393[1])];
                let t406: [uint32x4_t; 2] = [vcgeq_u32(t405[0], t388[0]), vcgeq_u32(t405[1], t388[1])];
                let t407: [uint32x4_t; 2] = [vandq_u32(live[0], t406[0]), vandq_u32(live[1], t406[1])];
                if any(&t407) {
                    let t408 = [vdupq_n_u64(4294967295u64); 4];
                    res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t407[0])))), t408[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t407[0])))), t408[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t407[1])))), t408[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t407[1])))), t408[3], res[3])];
                    live = [vbicq_u32(live[0], t407[0]), vbicq_u32(live[1], t407[1])];
                }
                if any(&live) {
                    let t409: [uint32x4_t; 2] = [vsubq_u32(t388[0], x40[0]), vsubq_u32(t388[1], x40[1])];
                    x48 = [vbslq_u32(live[0], t409[0], x48[0]), vbslq_u32(live[1], t409[1], x48[1])];
                    let t410 = [vdupq_n_u64(2u64); 4];
                    let t411: [uint64x2_t; 4] = [vandq_u64(x32[0], t410[0]), vandq_u64(x32[1], t410[1]), vandq_u64(x32[2], t410[2]), vandq_u64(x32[3], t410[3])];
                    let t412: [uint64x2_t; 4] = [vceqq_u64(t411[0], t398[0]), vceqq_u64(t411[1], t398[1]), vceqq_u64(t411[2], t398[2]), vceqq_u64(t411[3], t398[3])];
                    let t413: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t412[0]), vmovn_u64(t412[1])), vcombine_u32(vmovn_u64(t412[2]), vmovn_u64(t412[3]))];
                    let t414: [uint32x4_t; 2] = [vmvnq_u32(t413[0]), vmvnq_u32(t413[1])];
                    let t415: [uint32x4_t; 2] = [vandq_u32(live[0], t414[0]), vandq_u32(live[1], t414[1])];
                    if any(&t415) {
                        let t416: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t415[0])))), t416[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t415[0])))), t416[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t415[1])))), t416[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t415[1])))), t416[3], res[3])];
                        live = [vbicq_u32(live[0], t415[0]), vbicq_u32(live[1], t415[1])];
                    }
                    if any(&live) {
                        let t417: [uint32x4_t; 2] = [vceqq_u32(x48[0], t389[0]), vceqq_u32(x48[1], t389[1])];
                        let t418: [uint32x4_t; 2] = [vandq_u32(live[0], t417[0]), vandq_u32(live[1], t417[1])];
                        if any(&t418) {
                            let t419 = [vdupq_n_u64(4294967295u64); 4];
                            res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t418[0])))), t419[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t418[0])))), t419[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t418[1])))), t419[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t418[1])))), t419[3], res[3])];
                            live = [vbicq_u32(live[0], t418[0]), vbicq_u32(live[1], t418[1])];
                        }
                        if any(&live) {
                            let t420 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
                            let t421: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                            let t422: [uint64x2_t; 4] = [vorrq_u64(t420[0], t421[0]), vorrq_u64(t420[1], t421[1]), vorrq_u64(t420[2], t421[2]), vorrq_u64(t420[3], t421[3])];
                            res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t422[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t422[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t422[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t422[3], res[3])];
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
unsafe fn b8_r0r1l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x32: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t423 = [vdupq_n_u32(0u32); 2];
    let t424: [uint32x4_t; 2] = [vceqq_u32(x16[0], t423[0]), vceqq_u32(x16[1], t423[1])];
    let t425: [uint32x4_t; 2] = [vandq_u32(live[0], t424[0]), vandq_u32(live[1], t424[1])];
    if any(&t425) {
        let t426 = [vdupq_n_u64(4294967295u64); 4];
        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t425[0])))), t426[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t425[0])))), t426[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t425[1])))), t426[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t425[1])))), t426[3], res[3])];
        live = [vbicq_u32(live[0], t425[0]), vbicq_u32(live[1], t425[1])];
    }
    if any(&live) {
        let t427 = [vdupq_n_u32(1u32); 2];
        let t428: [uint32x4_t; 2] = [vsubq_u32(x16[0], t427[0]), vsubq_u32(x16[1], t427[1])];
        x24 = [vbslq_u32(live[0], t428[0], x24[0]), vbslq_u32(live[1], t428[1], x24[1])];
        let t429 = b12_r0r1l_p(l, lid0, live, arg0, arg1, x24);
        x32 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t429[0], x32[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t429[1], x32[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t429[2], x32[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t429[3], x32[3])];
        let t430 = [vdupq_n_u64(1u64); 4];
        let t431: [uint64x2_t; 4] = [vandq_u64(x32[0], t430[0]), vandq_u64(x32[1], t430[1]), vandq_u64(x32[2], t430[2]), vandq_u64(x32[3], t430[3])];
        let t432 = [vdupq_n_u64(0u64); 4];
        let t433: [uint64x2_t; 4] = [vceqq_u64(t431[0], t432[0]), vceqq_u64(t431[1], t432[1]), vceqq_u64(t431[2], t432[2]), vceqq_u64(t431[3], t432[3])];
        let t434: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t433[0]), vmovn_u64(t433[1])), vcombine_u32(vmovn_u64(t433[2]), vmovn_u64(t433[3]))];
        let t435: [uint32x4_t; 2] = [vandq_u32(live[0], t434[0]), vandq_u32(live[1], t434[1])];
        if any(&t435) {
            let t436: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x24[0])), vmovl_u32(vget_high_u32(x24[0])), vmovl_u32(vget_low_u32(x24[1])), vmovl_u32(vget_high_u32(x24[1]))];
            res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t435[0])))), t436[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t435[0])))), t436[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t435[1])))), t436[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t435[1])))), t436[3], res[3])];
            live = [vbicq_u32(live[0], t435[0]), vbicq_u32(live[1], t435[1])];
        }
        if any(&live) {
            let t437: [uint64x2_t; 4] = [vshlq_u64(x32[0], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[1], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[2], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[3], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64)))];
            let t438: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t437[0]), vmovn_u64(t437[1])), vcombine_u32(vmovn_u64(t437[2]), vmovn_u64(t437[3]))];
            x40 = [vbslq_u32(live[0], t438[0], x40[0]), vbslq_u32(live[1], t438[1], x40[1])];
            let t439: [uint32x4_t; 2] = [vsubq_u32(x40[0], t427[0]), vsubq_u32(x40[1], t427[1])];
            let t440: [uint32x4_t; 2] = [vcgeq_u32(t439[0], x16[0]), vcgeq_u32(t439[1], x16[1])];
            let t441: [uint32x4_t; 2] = [vandq_u32(live[0], t440[0]), vandq_u32(live[1], t440[1])];
            if any(&t441) {
                let t442 = [vdupq_n_u64(4294967295u64); 4];
                res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t441[0])))), t442[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t441[0])))), t442[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t441[1])))), t442[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t441[1])))), t442[3], res[3])];
                live = [vbicq_u32(live[0], t441[0]), vbicq_u32(live[1], t441[1])];
            }
            if any(&live) {
                let t443: [uint32x4_t; 2] = [vsubq_u32(x16[0], x40[0]), vsubq_u32(x16[1], x40[1])];
                x48 = [vbslq_u32(live[0], t443[0], x48[0]), vbslq_u32(live[1], t443[1], x48[1])];
                let t444 = [vdupq_n_u64(2u64); 4];
                let t445: [uint64x2_t; 4] = [vandq_u64(x32[0], t444[0]), vandq_u64(x32[1], t444[1]), vandq_u64(x32[2], t444[2]), vandq_u64(x32[3], t444[3])];
                let t446: [uint64x2_t; 4] = [vceqq_u64(t445[0], t432[0]), vceqq_u64(t445[1], t432[1]), vceqq_u64(t445[2], t432[2]), vceqq_u64(t445[3], t432[3])];
                let t447: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t446[0]), vmovn_u64(t446[1])), vcombine_u32(vmovn_u64(t446[2]), vmovn_u64(t446[3]))];
                let t448: [uint32x4_t; 2] = [vmvnq_u32(t447[0]), vmvnq_u32(t447[1])];
                let t449: [uint32x4_t; 2] = [vandq_u32(live[0], t448[0]), vandq_u32(live[1], t448[1])];
                if any(&t449) {
                    let t450: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                    res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t449[0])))), t450[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t449[0])))), t450[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t449[1])))), t450[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t449[1])))), t450[3], res[3])];
                    live = [vbicq_u32(live[0], t449[0]), vbicq_u32(live[1], t449[1])];
                }
                if any(&live) {
                    let t451: [uint32x4_t; 2] = [vceqq_u32(x48[0], t423[0]), vceqq_u32(x48[1], t423[1])];
                    let t452: [uint32x4_t; 2] = [vandq_u32(live[0], t451[0]), vandq_u32(live[1], t451[1])];
                    if any(&t452) {
                        let t453 = [vdupq_n_u64(4294967295u64); 4];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t452[0])))), t453[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t452[0])))), t453[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t452[1])))), t453[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t452[1])))), t453[3], res[3])];
                        live = [vbicq_u32(live[0], t452[0]), vbicq_u32(live[1], t452[1])];
                    }
                    if any(&live) {
                        let t454 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
                        let t455: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                        let t456: [uint64x2_t; 4] = [vorrq_u64(t454[0], t455[0]), vorrq_u64(t454[1], t455[1]), vorrq_u64(t454[2], t455[2]), vorrq_u64(t454[3], t455[3])];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t456[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t456[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t456[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t456[3], res[3])];
                        live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
                    }
                }
            }
        }
    }
    res
}

#[inline(always)]
unsafe fn b9_r3gl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg3: *const u8, x16: [uint32x4_t; 2]) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let t457 = [vdupq_n_u32((32u32).wrapping_sub(1u32)); 2];
    let t458: [uint32x4_t; 2] = [vceqq_u32(x16[0], t457[0]), vceqq_u32(x16[1], t457[1])];
    let t459: [uint32x4_t; 2] = [vandq_u32(live[0], t458[0]), vandq_u32(live[1], t458[1])];
    {
        let t460: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t461: [uint32x4_t; 2] = [vshlq_u32(t460[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(t460[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
        let t462: [uint32x4_t; 2] = [vshlq_u32(t460[0], vnegq_s32(vdupq_n_s32(((5u32) & 31) as i32))), vshlq_u32(t460[1], vnegq_s32(vdupq_n_s32(((5u32) & 31) as i32)))];
        let t463 = [vdupq_n_u32(31u32); 2];
        let t464: [uint32x4_t; 2] = [vandq_u32(t462[0], t463[0]), vandq_u32(t462[1], t463[1])];
        let t465 = [vdupq_n_u32(1u32); 2];
        let t466: [uint32x4_t; 2] = [vshlq_u32(t465[0], vreinterpretq_s32_u32(vandq_u32(t464[0], vdupq_n_u32(31)))), vshlq_u32(t465[1], vreinterpretq_s32_u32(vandq_u32(t464[1], vdupq_n_u32(31))))];
        let t467 = b13_al4r3l_p(l, lid0, t459, arg3, t461, t466);
    }
}

#[inline(always)]
unsafe fn b10_r3ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg3: *const u8, x8: [uint32x4_t; 2], x16: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t468: [uint32x4_t; 2] = [vshlq_u32(x8[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x8[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t469 = b11_al4r3_p(l, lid0, live, arg3, t468);
    x24 = t469;
    let t470: [uint32x4_t; 2] = [vcgeq_u32(x8[0], x16[0]), vcgeq_u32(x8[1], x16[1])];
    let t471: [uint32x4_t; 2] = [vshlq_u32(x8[0], vnegq_s32(vdupq_n_s32(((5u32) & 31) as i32))), vshlq_u32(x8[1], vnegq_s32(vdupq_n_s32(((5u32) & 31) as i32)))];
    let t472 = [vdupq_n_u32(31u32); 2];
    let t473: [uint32x4_t; 2] = [vandq_u32(t471[0], t472[0]), vandq_u32(t471[1], t472[1])];
    let t474: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vreinterpretq_s32_u32(vandq_u32(t473[0], vdupq_n_u32(31))))), vshlq_u32(x24[1], vnegq_s32(vreinterpretq_s32_u32(vandq_u32(t473[1], vdupq_n_u32(31)))))];
    let t475 = [vdupq_n_u32(1u32); 2];
    let t476: [uint32x4_t; 2] = [vandq_u32(t474[0], t475[0]), vandq_u32(t474[1], t475[1])];
    let t477 = [vdupq_n_u32(0u32); 2];
    let t478: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t476[0], t477[0])), vmvnq_u32(vceqq_u32(t476[1], t477[1]))];
    let t479: [uint32x4_t; 2] = [vorrq_u32(t470[0], t478[0]), vorrq_u32(t470[1], t478[1])];
    res = t479;
    return res;
}

#[inline(always)]
unsafe fn b11_al4r2_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg2: *const u8, x0i: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t480 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], x0i[0]), vandq_u32(live[1], x0i[1]))));
    let t481: [uint32x4_t; 2] = [vbslq_u32(live[0], x0i[0], t480), vbslq_u32(live[1], x0i[1], t480)];
    let t482 = arg2 as *const u32;
    let mut t483: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    t483[0] = vld1q_lane_u32::<0>(t482.add(vgetq_lane_u32::<0>(t481[0]) as usize), t483[0]);
    t483[0] = vld1q_lane_u32::<1>(t482.add(vgetq_lane_u32::<1>(t481[0]) as usize), t483[0]);
    t483[0] = vld1q_lane_u32::<2>(t482.add(vgetq_lane_u32::<2>(t481[0]) as usize), t483[0]);
    t483[0] = vld1q_lane_u32::<3>(t482.add(vgetq_lane_u32::<3>(t481[0]) as usize), t483[0]);
    t483[1] = vld1q_lane_u32::<0>(t482.add(vgetq_lane_u32::<0>(t481[1]) as usize), t483[1]);
    t483[1] = vld1q_lane_u32::<1>(t482.add(vgetq_lane_u32::<1>(t481[1]) as usize), t483[1]);
    t483[1] = vld1q_lane_u32::<2>(t482.add(vgetq_lane_u32::<2>(t481[1]) as usize), t483[1]);
    t483[1] = vld1q_lane_u32::<3>(t482.add(vgetq_lane_u32::<3>(t481[1]) as usize), t483[1]);
    res = t483;
    return res;
}

#[inline(always)]
unsafe fn b11_al4r3_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg3: *const u8, x0i: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t484 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], x0i[0]), vandq_u32(live[1], x0i[1]))));
    let t485: [uint32x4_t; 2] = [vbslq_u32(live[0], x0i[0], t484), vbslq_u32(live[1], x0i[1], t484)];
    let t486 = arg3 as *const u32;
    let mut t487: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    t487[0] = vld1q_lane_u32::<0>(t486.add(vgetq_lane_u32::<0>(t485[0]) as usize), t487[0]);
    t487[0] = vld1q_lane_u32::<1>(t486.add(vgetq_lane_u32::<1>(t485[0]) as usize), t487[0]);
    t487[0] = vld1q_lane_u32::<2>(t486.add(vgetq_lane_u32::<2>(t485[0]) as usize), t487[0]);
    t487[0] = vld1q_lane_u32::<3>(t486.add(vgetq_lane_u32::<3>(t485[0]) as usize), t487[0]);
    t487[1] = vld1q_lane_u32::<0>(t486.add(vgetq_lane_u32::<0>(t485[1]) as usize), t487[1]);
    t487[1] = vld1q_lane_u32::<1>(t486.add(vgetq_lane_u32::<1>(t485[1]) as usize), t487[1]);
    t487[1] = vld1q_lane_u32::<2>(t486.add(vgetq_lane_u32::<2>(t485[1]) as usize), t487[1]);
    t487[1] = vld1q_lane_u32::<3>(t486.add(vgetq_lane_u32::<3>(t485[1]) as usize), t487[1]);
    res = t487;
    return res;
}

#[inline(always)]
unsafe fn b12_r0r1l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t488 = b14_r0r1sl_p(l, lid0, live, arg0, arg1, 0u32, x16);
    res = t488;
    return res;
}

#[inline(always)]
unsafe fn b12_r0r1l_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t489 = b14_r0r1sl_f(l, lid0, arg0, arg1, 0u32, x16);
    res = t489;
    return res;
}

#[inline(always)]
unsafe fn b13_al4r3l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg3: *const u8, x0i: [uint32x4_t; 2], x8: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let (mut t490, mut t491, mut t492, mut t493) = ([0u32; 8], [0u32; 8], [0u32; 8], [0u32; 8]);
    for r in 0..2 { vst1q_u32(t490.as_mut_ptr().add(4 * r), live[r]); vst1q_u32(t491.as_mut_ptr().add(4 * r), x0i[r]); vst1q_u32(t492.as_mut_ptr().add(4 * r), x8[r]); }
    for j in 0..8 { if t490[j] != 0 { t493[j] = AtomicU32::from_ptr((arg3 as *mut u8).add(t491[j] as usize * 4) as *mut u32).fetch_or(t492[j], Relaxed); } }
    let t494: [uint32x4_t; 2] = [vld1q_u32(t493.as_ptr().add(0)), vld1q_u32(t493.as_ptr().add(4))];
    res = t494;
    return res;
}

#[inline(always)]
unsafe fn b14_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: u64 = 0;
    let mut x56: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t495 = b15_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    x32 = t495;
    let t496: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t497 = b16_r0r1sl_p(l, lid0, live, arg0, arg1, (1u32).wrapping_add(x16), t496);
    x40 = t497;
    let t498 = b17_s_p(l, lid0, live, x32);
    x48 = t498;
    let t499 = [vdupq_n_u32((1024u32).wrapping_sub(1u32)); 2];
    let t500: [uint32x4_t; 2] = [vandq_u32(x24[0], t499[0]), vandq_u32(x24[1], t499[1])];
    let t501 = [vdupq_n_u32(x32); 2];
    let t502: [uint32x4_t; 2] = [vmulq_u32(t500[0], t501[0]), vmulq_u32(t500[1], t501[1])];
    let t503 = b18_r0r1lls_p(l, lid0, live, arg0, arg1, x40, t502, 8u32);
    let t504 = [vdupq_n_u64(x48); 4];
    let t505: [uint64x2_t; 4] = [vandq_u64(t503[0], t504[0]), vandq_u64(t503[1], t504[1]), vandq_u64(t503[2], t504[2]), vandq_u64(t503[3], t504[3])];
    x56 = t505;
    let t506 = [vdupq_n_u64(x48); 4];
    let t507: [uint64x2_t; 4] = [vceqq_u64(x56[0], t506[0]), vceqq_u64(x56[1], t506[1]), vceqq_u64(x56[2], t506[2]), vceqq_u64(x56[3], t506[3])];
    let t508: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t507[0]), vmovn_u64(t507[1])), vcombine_u32(vmovn_u64(t507[2]), vmovn_u64(t507[3]))];
    let mut t509: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t510: [uint32x4_t; 2] = [vandq_u32(live[0], t508[0]), vandq_u32(live[1], t508[1])];
    let t511: [uint32x4_t; 2] = [vbicq_u32(live[0], t508[0]), vbicq_u32(live[1], t508[1])];
    if any(&t510) {
        let t512 = b19_r0r1sl_p(l, lid0, t510, arg0, arg1, (x16).wrapping_add(1u32), x24);
        t509 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t510[0])))), t512[0], t509[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t510[0])))), t512[1], t509[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t510[1])))), t512[2], t509[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t510[1])))), t512[3], t509[3])];
    }
    {
        t509 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t511[0])))), x56[0], t509[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t511[0])))), x56[1], t509[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t511[1])))), x56[2], t509[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t511[1])))), x56[3], t509[3])];
    }
    res = t509;
    return res;
}

#[inline(always)]
unsafe fn b14_r0r1sl_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: u64 = 0;
    let mut x56: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t513 = b15_r0r1s_f(l, lid0, arg0, arg1, x16);
    x32 = t513;
    let t514: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t515 = b16_r0r1sl_f(l, lid0, arg0, arg1, (1u32).wrapping_add(x16), t514);
    x40 = t515;
    let t516 = b17_s_f(l, lid0, x32);
    x48 = t516;
    let t517 = [vdupq_n_u32((1024u32).wrapping_sub(1u32)); 2];
    let t518: [uint32x4_t; 2] = [vandq_u32(x24[0], t517[0]), vandq_u32(x24[1], t517[1])];
    let t519 = [vdupq_n_u32(x32); 2];
    let t520: [uint32x4_t; 2] = [vmulq_u32(t518[0], t519[0]), vmulq_u32(t518[1], t519[1])];
    let t521 = b18_r0r1lls_f(l, lid0, arg0, arg1, x40, t520, 8u32);
    let t522 = [vdupq_n_u64(x48); 4];
    let t523: [uint64x2_t; 4] = [vandq_u64(t521[0], t522[0]), vandq_u64(t521[1], t522[1]), vandq_u64(t521[2], t522[2]), vandq_u64(t521[3], t522[3])];
    x56 = t523;
    let t524 = [vdupq_n_u64(x48); 4];
    let t525: [uint64x2_t; 4] = [vceqq_u64(x56[0], t524[0]), vceqq_u64(x56[1], t524[1]), vceqq_u64(x56[2], t524[2]), vceqq_u64(x56[3], t524[3])];
    let t526: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t525[0]), vmovn_u64(t525[1])), vcombine_u32(vmovn_u64(t525[2]), vmovn_u64(t525[3]))];
    let mut t527: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t528: [uint32x4_t; 2] = [vmvnq_u32(t526[0]), vmvnq_u32(t526[1])];
    if any(&t526) {
        let t529 = b19_r0r1sl_p(l, lid0, t526, arg0, arg1, (x16).wrapping_add(1u32), x24);
        t527 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t526[0])))), t529[0], t527[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t526[0])))), t529[1], t527[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t526[1])))), t529[2], t527[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t526[1])))), t529[3], t527[3])];
    }
    {
        t527 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t528[0])))), x56[0], t527[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t528[0])))), x56[1], t527[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t528[1])))), x56[2], t527[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t528[1])))), x56[3], t527[3])];
    }
    res = t527;
    return res;
}

#[inline(always)]
unsafe fn b15_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t530 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    res = (((t530).wrapping_shr(((4u32).wrapping_mul(x16)) as u32)) & (15u32));
    return res;
}

#[inline(always)]
unsafe fn b15_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t531 = b20_r0r1s_f(l, lid0, arg0, arg1, 16u32);
    res = (((t531).wrapping_shr(((4u32).wrapping_mul(x16)) as u32)) & (15u32));
    return res;
}

#[inline(always)]
unsafe fn b16_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t532 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 40u32);
    let t533 = [vdupq_n_u32(t532); 2];
    let t534: [uint32x4_t; 2] = [vmulq_u32(x24[0], t533[0]), vmulq_u32(x24[1], t533[1])];
    let t535 = b21_r0r1s_p(l, lid0, live, arg0, arg1, 0u32);
    let t536 = [vdupq_n_u32(t535); 2];
    let t537: [uint32x4_t; 2] = [vaddq_u32(t534[0], t536[0]), vaddq_u32(t534[1], t536[1])];
    x32 = t537;
    if ((x16) == (0u32)) {
        res = x32;
        return res;
    }
    let t538 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    let t539: [uint32x4_t; 2] = [vshlq_u32(x24[0], vdupq_n_s32(((10u32) & 31) as i32)), vshlq_u32(x24[1], vdupq_n_s32(((10u32) & 31) as i32))];
    let t540 = [vdupq_n_u32(t538); 2];
    let t541: [uint32x4_t; 2] = [vsubq_u32(t540[0], t539[0]), vsubq_u32(t540[1], t539[1])];
    let t542 = b22_ls_p(l, lid0, live, t541, 1024u32);
    x40 = t542;
    let t543 = b21_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    let t544 = [vdupq_n_u32(t543); 2];
    let t545: [uint32x4_t; 2] = [vmulq_u32(x40[0], t544[0]), vmulq_u32(x40[1], t544[1])];
    let t546: [uint32x4_t; 2] = [vaddq_u32(x32[0], t545[0]), vaddq_u32(x32[1], t545[1])];
    res = t546;
    return res;
}

#[inline(always)]
unsafe fn b16_r0r1sl_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t547 = b20_r0r1s_f(l, lid0, arg0, arg1, 40u32);
    let t548 = [vdupq_n_u32(t547); 2];
    let t549: [uint32x4_t; 2] = [vmulq_u32(x24[0], t548[0]), vmulq_u32(x24[1], t548[1])];
    let t550 = b21_r0r1s_f(l, lid0, arg0, arg1, 0u32);
    let t551 = [vdupq_n_u32(t550); 2];
    let t552: [uint32x4_t; 2] = [vaddq_u32(t549[0], t551[0]), vaddq_u32(t549[1], t551[1])];
    x32 = t552;
    if ((x16) == (0u32)) {
        res = x32;
        return res;
    }
    let t553 = b20_r0r1s_f(l, lid0, arg0, arg1, 4u32);
    let t554: [uint32x4_t; 2] = [vshlq_u32(x24[0], vdupq_n_s32(((10u32) & 31) as i32)), vshlq_u32(x24[1], vdupq_n_s32(((10u32) & 31) as i32))];
    let t555 = [vdupq_n_u32(t553); 2];
    let t556: [uint32x4_t; 2] = [vsubq_u32(t555[0], t554[0]), vsubq_u32(t555[1], t554[1])];
    let t557 = b22_ls_f(l, lid0, t556, 1024u32);
    x40 = t557;
    let t558 = b21_r0r1s_f(l, lid0, arg0, arg1, x16);
    let t559 = [vdupq_n_u32(t558); 2];
    let t560: [uint32x4_t; 2] = [vmulq_u32(x40[0], t559[0]), vmulq_u32(x40[1], t559[1])];
    let t561: [uint32x4_t; 2] = [vaddq_u32(x32[0], t560[0]), vaddq_u32(x32[1], t560[1])];
    res = t561;
    return res;
}

#[inline(always)]
unsafe fn b17_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t562: u64;
    if ((x0) >= (8u32)) {
        t562 = !(0u64);
    } else {
        t562 = ((1u64).wrapping_shl(((((x0).wrapping_mul(8u32)) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t562;
    return res;
}

#[inline(always)]
unsafe fn b17_s_f(l: &Launch, lid0: u32, x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
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
unsafe fn b17_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t564 = [vdupq_n_u32(8u32); 2];
    let t565: [uint32x4_t; 2] = [vcgeq_u32(x0[0], t564[0]), vcgeq_u32(x0[1], t564[1])];
    let mut t566: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t567: [uint32x4_t; 2] = [vandq_u32(live[0], t565[0]), vandq_u32(live[1], t565[1])];
    let t568: [uint32x4_t; 2] = [vbicq_u32(live[0], t565[0]), vbicq_u32(live[1], t565[1])];
    {
        let t569 = [vdupq_n_u64(!(0u64)); 4];
        t566 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t567[0])))), t569[0], t566[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t567[0])))), t569[1], t566[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t567[1])))), t569[2], t566[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t567[1])))), t569[3], t566[3])];
    }
    {
        let t570: [uint32x4_t; 2] = [vmulq_u32(x0[0], t564[0]), vmulq_u32(x0[1], t564[1])];
        let t571 = [vdupq_n_u64(1u64); 4];
        let t572: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t570[0])), vmovl_u32(vget_high_u32(t570[0])), vmovl_u32(vget_low_u32(t570[1])), vmovl_u32(vget_high_u32(t570[1]))];
        let t573: [uint64x2_t; 4] = [vshlq_u64(t571[0], vreinterpretq_s64_u64(vandq_u64(t572[0], vdupq_n_u64(63)))), vshlq_u64(t571[1], vreinterpretq_s64_u64(vandq_u64(t572[1], vdupq_n_u64(63)))), vshlq_u64(t571[2], vreinterpretq_s64_u64(vandq_u64(t572[2], vdupq_n_u64(63)))), vshlq_u64(t571[3], vreinterpretq_s64_u64(vandq_u64(t572[3], vdupq_n_u64(63))))];
        let t574: [uint64x2_t; 4] = [vsubq_u64(t573[0], t571[0]), vsubq_u64(t573[1], t571[1]), vsubq_u64(t573[2], t571[2]), vsubq_u64(t573[3], t571[3])];
        t566 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t568[0])))), t574[0], t566[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t568[0])))), t574[1], t566[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t568[1])))), t574[2], t566[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t568[1])))), t574[3], t566[3])];
    }
    res = t566;
    return res;
}

#[inline(always)]
unsafe fn b18_r0r1lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2], x24: [uint32x4_t; 2], x32: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t575 = b23_r0lls_p(l, lid0, live, arg0, x16, x24, x32);
    res = t575;
    return res;
}

#[inline(always)]
unsafe fn b18_r0r1lls_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2], x24: [uint32x4_t; 2], x32: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t576 = b23_r0lls_f(l, lid0, arg0, x16, x24, x32);
    res = t576;
    return res;
}

#[inline(always)]
unsafe fn b19_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
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
    let t577 = b20_r0r1s_p(l, lid0, live, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x16)));
    x32 = t577;
    let t578 = b20_r0r1s_p(l, lid0, live, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x16)));
    x40 = t578;
    let t579: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    x48 = t579;
    let t580 = [vdupq_n_u32(4u32); 2];
    let t581: [uint32x4_t; 2] = [vmulq_u32(x48[0], t580[0]), vmulq_u32(x48[1], t580[1])];
    let t582 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x40, t581, 4u32);
    let t583: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t582[0]), vmovn_u64(t582[1])), vcombine_u32(vmovn_u64(t582[2]), vmovn_u64(t582[3]))];
    x56 = t583;
    let t584: [uint32x4_t; 2] = [vmulq_u32(x48[0], t580[0]), vmulq_u32(x48[1], t580[1])];
    let t585: [uint32x4_t; 2] = [vaddq_u32(t584[0], t580[0]), vaddq_u32(t584[1], t580[1])];
    let t586 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x40, t585, 4u32);
    let t587: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t586[0]), vmovn_u64(t586[1])), vcombine_u32(vmovn_u64(t586[2]), vmovn_u64(t586[3]))];
    x64 = t587;
    let t588 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    x72 = ((t588).wrapping_add((256u32).wrapping_sub(1u32))).wrapping_shr((8u32) as u32);
    let t589: [uint32x4_t; 2] = [vmulq_u32(x48[0], t580[0]), vmulq_u32(x48[1], t580[1])];
    let t590 = [vdupq_n_u32(x72); 2];
    let t591: [uint32x4_t; 2] = [vsubq_u32(t590[0], t589[0]), vsubq_u32(t590[1], t589[1])];
    let t592 = b25_sl_p(l, lid0, live, 4u32, t591);
    x80 = t592;
    let t593: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
    let t594 = [vdupq_n_u32((4u32).wrapping_sub(1u32)); 2];
    let t595: [uint32x4_t; 2] = [vandq_u32(t593[0], t594[0]), vandq_u32(t593[1], t594[1])];
    x88 = t595;
    let t596 = [vdupq_n_u32(0u32); 2];
    let t597: [uint32x4_t; 2] = [vceqq_u32(x88[0], t596[0]), vceqq_u32(x88[1], t596[1])];
    let mut t598: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t599: [uint32x4_t; 2] = [vandq_u32(live[0], t597[0]), vandq_u32(live[1], t597[1])];
    let t600: [uint32x4_t; 2] = [vbicq_u32(live[0], t597[0]), vbicq_u32(live[1], t597[1])];
    {
        t598 = [vbslq_u32(t599[0], t596[0], t598[0]), vbslq_u32(t599[1], t596[1], t598[1])];
    }
    if any(&t600) {
        let t601 = [vdupq_n_u32(1u32); 2];
        let t602: [uint32x4_t; 2] = [vsubq_u32(x88[0], t601[0]), vsubq_u32(x88[1], t601[1])];
        let t603 = [vdupq_n_u32(2u32); 2];
        let t604: [uint32x4_t; 2] = [vmulq_u32(t603[0], t602[0]), vmulq_u32(t603[1], t602[1])];
        let t605: [uint32x4_t; 2] = [vaddq_u32(x56[0], t604[0]), vaddq_u32(x56[1], t604[1])];
        let t606 = b26_ll_p(l, lid0, t600, t605, x64);
        let t607 = b24_r0r1sls_p(l, lid0, t600, arg0, arg1, x32, t606, 2u32);
        let t608: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t607[0]), vmovn_u64(t607[1])), vcombine_u32(vmovn_u64(t607[2]), vmovn_u64(t607[3]))];
        t598 = [vbslq_u32(t600[0], t608[0], t598[0]), vbslq_u32(t600[1], t608[1], t598[1])];
    }
    x96 = t598;
    let t609 = [vdupq_n_u32(2u32); 2];
    let t610: [uint32x4_t; 2] = [vmulq_u32(t609[0], x88[0]), vmulq_u32(t609[1], x88[1])];
    let t611: [uint32x4_t; 2] = [vaddq_u32(x56[0], t610[0]), vaddq_u32(x56[1], t610[1])];
    let t612 = b26_ll_p(l, lid0, live, t611, x64);
    let t613 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t612, 2u32);
    let t614: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t613[0]), vmovn_u64(t613[1])), vcombine_u32(vmovn_u64(t613[2]), vmovn_u64(t613[3]))];
    x104 = t614;
    let t615: [uint32x4_t; 2] = [vmulq_u32(t609[0], x80[0]), vmulq_u32(t609[1], x80[1])];
    let t616: [uint32x4_t; 2] = [vaddq_u32(x56[0], t615[0]), vaddq_u32(x56[1], t615[1])];
    x112 = t616;
    let t617 = [vdupq_n_u32((256u32).wrapping_sub(1u32)); 2];
    let t618: [uint32x4_t; 2] = [vandq_u32(x24[0], t617[0]), vandq_u32(x24[1], t617[1])];
    x120 = t618;
    x128 = 9259542123273814144u64;
    let t619: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x120[0])), vmovl_u32(vget_high_u32(x120[0])), vmovl_u32(vget_low_u32(x120[1])), vmovl_u32(vget_high_u32(x120[1]))];
    let t620 = [vdupq_n_u64(72340172838076673u64); 4];
    let mut t621: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    t621[0] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t619[0]) as u64).wrapping_mul(vgetq_lane_u64::<0>(t620[0]) as u64) as u64, t621[0]);
    t621[0] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t619[0]) as u64).wrapping_mul(vgetq_lane_u64::<1>(t620[0]) as u64) as u64, t621[0]);
    t621[1] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t619[1]) as u64).wrapping_mul(vgetq_lane_u64::<0>(t620[1]) as u64) as u64, t621[1]);
    t621[1] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t619[1]) as u64).wrapping_mul(vgetq_lane_u64::<1>(t620[1]) as u64) as u64, t621[1]);
    t621[2] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t619[2]) as u64).wrapping_mul(vgetq_lane_u64::<0>(t620[2]) as u64) as u64, t621[2]);
    t621[2] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t619[2]) as u64).wrapping_mul(vgetq_lane_u64::<1>(t620[2]) as u64) as u64, t621[2]);
    t621[3] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t619[3]) as u64).wrapping_mul(vgetq_lane_u64::<0>(t620[3]) as u64) as u64, t621[3]);
    t621[3] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t619[3]) as u64).wrapping_mul(vgetq_lane_u64::<1>(t620[3]) as u64) as u64, t621[3]);
    x136 = t621;
    x144 = x96;
    x152 = x96;
    let mut t622 = live;
    loop {
        // Lanes part: each its own rounds.
        let t624: [uint32x4_t; 2] = [vcltq_u32(x152[0], x104[0]), vcltq_u32(x152[1], x104[1])];
        t622 = [vandq_u32(t622[0], t624[0]), vandq_u32(t622[1], t624[1])];
        if !(any(&t622)) { break; }
        let t626: [uint32x4_t; 2] = [vaddq_u32(x112[0], x152[0]), vaddq_u32(x112[1], x152[1])];
        let t627 = b26_ll_p(l, lid0, t622, t626, x64);
        let t628 = b24_r0r1sls_p(l, lid0, t622, arg0, arg1, x32, t627, 8u32);
        x160 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t622[0])))), t628[0], x160[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t622[0])))), t628[1], x160[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t622[1])))), t628[2], x160[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t622[1])))), t628[3], x160[3])];
        let t629 = [vdupq_n_u64(x128); 4];
        let t630: [uint64x2_t; 4] = [vorrq_u64(x160[0], t629[0]), vorrq_u64(x160[1], t629[1]), vorrq_u64(x160[2], t629[2]), vorrq_u64(x160[3], t629[3])];
        let t631 = [vdupq_n_u64(!(x128)); 4];
        let t632: [uint64x2_t; 4] = [vandq_u64(x136[0], t631[0]), vandq_u64(x136[1], t631[1]), vandq_u64(x136[2], t631[2]), vandq_u64(x136[3], t631[3])];
        let t633: [uint64x2_t; 4] = [vsubq_u64(t630[0], t632[0]), vsubq_u64(t630[1], t632[1]), vsubq_u64(t630[2], t632[2]), vsubq_u64(t630[3], t632[3])];
        x168 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t622[0])))), t633[0], x168[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t622[0])))), t633[1], x168[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t622[1])))), t633[2], x168[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t622[1])))), t633[3], x168[3])];
        let t634: [uint64x2_t; 4] = [vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x160[0]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x160[1]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x160[2]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x160[3])))];
        let t635: [uint64x2_t; 4] = [vandq_u64(t634[0], x136[0]), vandq_u64(t634[1], x136[1]), vandq_u64(t634[2], x136[2]), vandq_u64(t634[3], x136[3])];
        let t636: [uint64x2_t; 4] = [veorq_u64(x160[0], x136[0]), veorq_u64(x160[1], x136[1]), veorq_u64(x160[2], x136[2]), veorq_u64(x160[3], x136[3])];
        let t637: [uint64x2_t; 4] = [vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(t636[0]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(t636[1]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(t636[2]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(t636[3])))];
        let t638: [uint64x2_t; 4] = [vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x168[0]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x168[1]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x168[2]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x168[3])))];
        let t639: [uint64x2_t; 4] = [vandq_u64(t637[0], t638[0]), vandq_u64(t637[1], t638[1]), vandq_u64(t637[2], t638[2]), vandq_u64(t637[3], t638[3])];
        let t640: [uint64x2_t; 4] = [vorrq_u64(t635[0], t639[0]), vorrq_u64(t635[1], t639[1]), vorrq_u64(t635[2], t639[2]), vorrq_u64(t635[3], t639[3])];
        let t641 = [vdupq_n_u64(x128); 4];
        let t642: [uint64x2_t; 4] = [vandq_u64(t641[0], t640[0]), vandq_u64(t641[1], t640[1]), vandq_u64(t641[2], t640[2]), vandq_u64(t641[3], t640[3])];
        x176 = t642;
        let t643: [uint32x4_t; 2] = [vsubq_u32(x104[0], x152[0]), vsubq_u32(x104[1], x152[1])];
        x184 = t643;
        let t644 = [vdupq_n_u32(8u32); 2];
        let t645: [uint32x4_t; 2] = [vcgeq_u32(x184[0], t644[0]), vcgeq_u32(x184[1], t644[1])];
        let mut t646: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
        let t647: [uint32x4_t; 2] = [vandq_u32(t622[0], t645[0]), vandq_u32(t622[1], t645[1])];
        let t648: [uint32x4_t; 2] = [vbicq_u32(t622[0], t645[0]), vbicq_u32(t622[1], t645[1])];
        {
            t646 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t647[0])))), x176[0], t646[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t647[0])))), x176[1], t646[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t647[1])))), x176[2], t646[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t647[1])))), x176[3], t646[3])];
        }
        {
            let t649: [uint32x4_t; 2] = [vmulq_u32(t644[0], x184[0]), vmulq_u32(t644[1], x184[1])];
            let t650 = [vdupq_n_u64(1u64); 4];
            let t651: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t649[0])), vmovl_u32(vget_high_u32(t649[0])), vmovl_u32(vget_low_u32(t649[1])), vmovl_u32(vget_high_u32(t649[1]))];
            let t652: [uint64x2_t; 4] = [vshlq_u64(t650[0], vreinterpretq_s64_u64(vandq_u64(t651[0], vdupq_n_u64(63)))), vshlq_u64(t650[1], vreinterpretq_s64_u64(vandq_u64(t651[1], vdupq_n_u64(63)))), vshlq_u64(t650[2], vreinterpretq_s64_u64(vandq_u64(t651[2], vdupq_n_u64(63)))), vshlq_u64(t650[3], vreinterpretq_s64_u64(vandq_u64(t651[3], vdupq_n_u64(63))))];
            let t653: [uint64x2_t; 4] = [vsubq_u64(t652[0], t650[0]), vsubq_u64(t652[1], t650[1]), vsubq_u64(t652[2], t650[2]), vsubq_u64(t652[3], t650[3])];
            let t654: [uint64x2_t; 4] = [vandq_u64(x176[0], t653[0]), vandq_u64(x176[1], t653[1]), vandq_u64(x176[2], t653[2]), vandq_u64(x176[3], t653[3])];
            t646 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t648[0])))), t654[0], t646[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t648[0])))), t654[1], t646[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t648[1])))), t654[2], t646[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t648[1])))), t654[3], t646[3])];
        }
        let t655 = b27_l_p(l, lid0, t622, t646);
        let t656: [uint32x4_t; 2] = [vaddq_u32(x144[0], t655[0]), vaddq_u32(x144[1], t655[1])];
        x144 = [vbslq_u32(t622[0], t656[0], x144[0]), vbslq_u32(t622[1], t656[1], x144[1])];
        let t657: [uint32x4_t; 2] = [vandq_u32(t622[0], live[0]), vandq_u32(t622[1], live[1])];
        x152 = [vaddq_u32(x152[0], vandq_u32(t657[0], vdupq_n_u32(8))), vaddq_u32(x152[1], vandq_u32(t657[1], vdupq_n_u32(8)))];
    }
    x96 = x144;
    let t658 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    let t659 = [vdupq_n_u32((((t658).wrapping_shr((((((4u32).wrapping_mul(x16)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32))); 2];
    x152 = t659;
    let t660 = [vdupq_n_u32(1u32); 2];
    let t661: [uint32x4_t; 2] = [vsubq_u32(x80[0], t660[0]), vsubq_u32(x80[1], t660[1])];
    let t662: [uint32x4_t; 2] = [vmulq_u32(t609[0], t661[0]), vmulq_u32(t609[1], t661[1])];
    let t663: [uint32x4_t; 2] = [vaddq_u32(x56[0], t662[0]), vaddq_u32(x56[1], t662[1])];
    let t664 = b26_ll_p(l, lid0, live, t663, x64);
    let t665 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t664, 2u32);
    let t666: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t665[0]), vmovn_u64(t665[1])), vcombine_u32(vmovn_u64(t665[2]), vmovn_u64(t665[3]))];
    let t667: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t666[0])), vmovl_u32(vget_high_u32(t666[0])), vmovl_u32(vget_low_u32(t666[1])), vmovl_u32(vget_high_u32(t666[1]))];
    x160 = t667;
    let t668: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x160[0]), vmovn_u64(x160[1])), vcombine_u32(vmovn_u64(x160[2]), vmovn_u64(x160[3]))];
    let t669: [uint32x4_t; 2] = [vaddq_u32(x112[0], t668[0]), vaddq_u32(x112[1], t668[1])];
    let t670: [uint32x4_t; 2] = [vmulq_u32(x96[0], x152[0]), vmulq_u32(x96[1], x152[1])];
    let t671: [uint32x4_t; 2] = [vaddq_u32(t669[0], t670[0]), vaddq_u32(t669[1], t670[1])];
    let t672 = b26_ll_p(l, lid0, live, t671, x64);
    let t673: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t672[0])), vmovl_u32(vget_high_u32(t672[0])), vmovl_u32(vget_low_u32(t672[1])), vmovl_u32(vget_high_u32(t672[1]))];
    x168 = t673;
    let t674: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x168[0]), vmovn_u64(x168[1])), vcombine_u32(vmovn_u64(x168[2]), vmovn_u64(x168[3]))];
    let t675 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t674, 8u32);
    let t676 = b17_l_p(l, lid0, live, x152);
    let t677: [uint64x2_t; 4] = [vandq_u64(t675[0], t676[0]), vandq_u64(t675[1], t676[1]), vandq_u64(t675[2], t676[2]), vandq_u64(t675[3], t676[3])];
    res = t677;
    return res;
}

#[inline(always)]
unsafe fn b20_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    res = (arg1.wrapping_add(((((x16).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned();
    return res;
}

#[inline(always)]
unsafe fn b20_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    res = (arg1.wrapping_add(((((x16).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned();
    return res;
}

#[inline(always)]
unsafe fn b21_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t678 = b20_r0r1s_p(l, lid0, live, arg0, arg1, (20u32).wrapping_add((4u32).wrapping_mul(x16)));
    res = t678;
    return res;
}

#[inline(always)]
unsafe fn b21_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t679 = b20_r0r1s_f(l, lid0, arg0, arg1, (20u32).wrapping_add((4u32).wrapping_mul(x16)));
    res = t679;
    return res;
}

#[inline(always)]
unsafe fn b22_ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x8: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t680 = [vdupq_n_u32(x8); 2];
    let t681: [uint32x4_t; 2] = [vcltq_u32(x0[0], t680[0]), vcltq_u32(x0[1], t680[1])];
    let mut t682: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t683: [uint32x4_t; 2] = [vandq_u32(live[0], t681[0]), vandq_u32(live[1], t681[1])];
    let t684: [uint32x4_t; 2] = [vbicq_u32(live[0], t681[0]), vbicq_u32(live[1], t681[1])];
    {
        t682 = [vbslq_u32(t683[0], x0[0], t682[0]), vbslq_u32(t683[1], x0[1], t682[1])];
    }
    {
        let t685 = [vdupq_n_u32(x8); 2];
        t682 = [vbslq_u32(t684[0], t685[0], t682[0]), vbslq_u32(t684[1], t685[1], t682[1])];
    }
    res = t682;
    return res;
}

#[inline(always)]
unsafe fn b22_ls_f(l: &Launch, lid0: u32, x0: [uint32x4_t; 2], x8: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t686 = [vdupq_n_u32(x8); 2];
    let t687: [uint32x4_t; 2] = [vcltq_u32(x0[0], t686[0]), vcltq_u32(x0[1], t686[1])];
    let mut t688: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t689: [uint32x4_t; 2] = [vmvnq_u32(t687[0]), vmvnq_u32(t687[1])];
    {
        t688 = [vbslq_u32(t687[0], x0[0], t688[0]), vbslq_u32(t687[1], x0[1], t688[1])];
    }
    {
        let t690 = [vdupq_n_u32(x8); 2];
        t688 = [vbslq_u32(t689[0], t690[0], t688[0]), vbslq_u32(t689[1], t690[1], t688[1])];
    }
    res = t688;
    return res;
}

#[inline(always)]
unsafe fn b23_r0lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: [uint32x4_t; 2], x16: [uint32x4_t; 2], x24: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t691: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x8[0])), vmovl_u32(vget_high_u32(x8[0])), vmovl_u32(vget_low_u32(x8[1])), vmovl_u32(vget_high_u32(x8[1]))];
    let t692: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x16[0])), vmovl_u32(vget_high_u32(x16[0])), vmovl_u32(vget_low_u32(x16[1])), vmovl_u32(vget_high_u32(x16[1]))];
    let t693: [uint64x2_t; 4] = [vaddq_u64(t691[0], t692[0]), vaddq_u64(t691[1], t692[1]), vaddq_u64(t691[2], t692[2]), vaddq_u64(t691[3], t692[3])];
    let t694 = b28_r0ls_p(l, lid0, live, arg0, t693, x24);
    x32 = t694;
    let t695: [uint64x2_t; 4];
    if ((x24) >= (8u32)) {
        t695 = x32;
    } else {
        let t696 = b29_s_p(l, lid0, live, (8u32).wrapping_mul(x24));
        let t697 = [vdupq_n_u64(t696); 4];
        let t698: [uint64x2_t; 4] = [vandq_u64(x32[0], t697[0]), vandq_u64(x32[1], t697[1]), vandq_u64(x32[2], t697[2]), vandq_u64(x32[3], t697[3])];
        t695 = t698;
    }
    res = t695;
    return res;
}

#[inline(always)]
unsafe fn b23_r0lls_f(l: &Launch, lid0: u32, arg0: *const u8, x8: [uint32x4_t; 2], x16: [uint32x4_t; 2], x24: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t699: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x8[0])), vmovl_u32(vget_high_u32(x8[0])), vmovl_u32(vget_low_u32(x8[1])), vmovl_u32(vget_high_u32(x8[1]))];
    let t700: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x16[0])), vmovl_u32(vget_high_u32(x16[0])), vmovl_u32(vget_low_u32(x16[1])), vmovl_u32(vget_high_u32(x16[1]))];
    let t701: [uint64x2_t; 4] = [vaddq_u64(t699[0], t700[0]), vaddq_u64(t699[1], t700[1]), vaddq_u64(t699[2], t700[2]), vaddq_u64(t699[3], t700[3])];
    let t702 = b28_r0ls_f(l, lid0, arg0, t701, x24);
    x32 = t702;
    let t703: [uint64x2_t; 4];
    if ((x24) >= (8u32)) {
        t703 = x32;
    } else {
        let t704 = b29_s_f(l, lid0, (8u32).wrapping_mul(x24));
        let t705 = [vdupq_n_u64(t704); 4];
        let t706: [uint64x2_t; 4] = [vandq_u64(x32[0], t705[0]), vandq_u64(x32[1], t705[1]), vandq_u64(x32[2], t705[2]), vandq_u64(x32[3], t705[3])];
        t703 = t706;
    }
    res = t703;
    return res;
}

#[inline(always)]
unsafe fn b24_r0r1sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2], x32: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t707 = b30_r0sls_p(l, lid0, live, arg0, x16, x24, x32);
    res = t707;
    return res;
}

#[inline(always)]
unsafe fn b25_sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x8: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t708 = [vdupq_n_u32(x0); 2];
    let t709: [uint32x4_t; 2] = [vcltq_u32(t708[0], x8[0]), vcltq_u32(t708[1], x8[1])];
    let mut t710: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t711: [uint32x4_t; 2] = [vandq_u32(live[0], t709[0]), vandq_u32(live[1], t709[1])];
    let t712: [uint32x4_t; 2] = [vbicq_u32(live[0], t709[0]), vbicq_u32(live[1], t709[1])];
    {
        let t713 = [vdupq_n_u32(x0); 2];
        t710 = [vbslq_u32(t711[0], t713[0], t710[0]), vbslq_u32(t711[1], t713[1], t710[1])];
    }
    {
        t710 = [vbslq_u32(t712[0], x8[0], t710[0]), vbslq_u32(t712[1], x8[1], t710[1])];
    }
    res = t710;
    return res;
}

#[inline(always)]
unsafe fn b26_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x8: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t714: [uint32x4_t; 2] = [vcltq_u32(x0[0], x8[0]), vcltq_u32(x0[1], x8[1])];
    let mut t715: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t716: [uint32x4_t; 2] = [vandq_u32(live[0], t714[0]), vandq_u32(live[1], t714[1])];
    let t717: [uint32x4_t; 2] = [vbicq_u32(live[0], t714[0]), vbicq_u32(live[1], t714[1])];
    {
        t715 = [vbslq_u32(t716[0], x0[0], t715[0]), vbslq_u32(t716[1], x0[1], t715[1])];
    }
    {
        t715 = [vbslq_u32(t717[0], x8[0], t715[0]), vbslq_u32(t717[1], x8[1], t715[1])];
    }
    res = t715;
    return res;
}

#[inline(always)]
unsafe fn b27_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint64x2_t; 4]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t718: [uint64x2_t; 4] = [vpaddlq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u64(x0[0]))))), vpaddlq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u64(x0[1]))))), vpaddlq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u64(x0[2]))))), vpaddlq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u64(x0[3])))))];
    let t719: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t718[0]), vmovn_u64(t718[1])), vcombine_u32(vmovn_u64(t718[2]), vmovn_u64(t718[3]))];
    res = t719;
    return res;
}

#[inline(always)]
unsafe fn b28_r0ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: [uint64x2_t; 4], x16: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t720 = [vdupq_n_u64(!(7u64)); 4];
    let t721: [uint64x2_t; 4] = [vandq_u64(x8[0], t720[0]), vandq_u64(x8[1], t720[1]), vandq_u64(x8[2], t720[2]), vandq_u64(x8[3], t720[3])];
    let t722: [uint64x2_t; 4] = [vandq_u64(t721[0], vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0]))))), vandq_u64(t721[1], vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0]))))), vandq_u64(t721[2], vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1]))))), vandq_u64(t721[3], vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))))];
    let t723: [uint64x2_t; 4] = [vaddq_u64(vdupq_n_u64((arg0) as u64), t722[0]), vaddq_u64(vdupq_n_u64((arg0) as u64), t722[1]), vaddq_u64(vdupq_n_u64((arg0) as u64), t722[2]), vaddq_u64(vdupq_n_u64((arg0) as u64), t722[3])];
    x24 = t723;
    let t724 = [vdupq_n_u64(7u64); 4];
    let t725: [uint64x2_t; 4] = [vandq_u64(x8[0], t724[0]), vandq_u64(x8[1], t724[1]), vandq_u64(x8[2], t724[2]), vandq_u64(x8[3], t724[3])];
    let t726: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t725[0]), vmovn_u64(t725[1])), vcombine_u32(vmovn_u64(t725[2]), vmovn_u64(t725[3]))];
    let t727 = [vdupq_n_u32(8u32); 2];
    let t728: [uint32x4_t; 2] = [vmulq_u32(t726[0], t727[0]), vmulq_u32(t726[1], t727[1])];
    x32 = t728;
    let mut t729: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    t729[0] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[0]) as usize as *const u64).read_unaligned(), t729[0]);
    t729[0] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[0]) as usize as *const u64).read_unaligned(), t729[0]);
    t729[1] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[1]) as usize as *const u64).read_unaligned(), t729[1]);
    t729[1] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[1]) as usize as *const u64).read_unaligned(), t729[1]);
    t729[2] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[2]) as usize as *const u64).read_unaligned(), t729[2]);
    t729[2] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[2]) as usize as *const u64).read_unaligned(), t729[2]);
    t729[3] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[3]) as usize as *const u64).read_unaligned(), t729[3]);
    t729[3] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[3]) as usize as *const u64).read_unaligned(), t729[3]);
    let t730: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x32[0])), vmovl_u32(vget_high_u32(x32[0])), vmovl_u32(vget_low_u32(x32[1])), vmovl_u32(vget_high_u32(x32[1]))];
    let t731: [uint64x2_t; 4] = [vshlq_u64(t729[0], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t730[0], vdupq_n_u64(63))))), vshlq_u64(t729[1], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t730[1], vdupq_n_u64(63))))), vshlq_u64(t729[2], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t730[2], vdupq_n_u64(63))))), vshlq_u64(t729[3], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t730[3], vdupq_n_u64(63)))))];
    x40 = t731;
    let t732 = [vdupq_n_u32(0u32); 2];
    let t733: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t732[0])), vmvnq_u32(vceqq_u32(x32[1], t732[1]))];
    let t734 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t735: [uint32x4_t; 2] = [vaddq_u32(x32[0], t734[0]), vaddq_u32(x32[1], t734[1])];
    let t736 = [vdupq_n_u32(64u32); 2];
    let t737: [uint32x4_t; 2] = [vcgtq_u32(t735[0], t736[0]), vcgtq_u32(t735[1], t736[1])];
    let t738: [uint32x4_t; 2] = [vandq_u32(t733[0], t737[0]), vandq_u32(t733[1], t737[1])];
    let mut t739: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t740: [uint32x4_t; 2] = [vandq_u32(live[0], t738[0]), vandq_u32(live[1], t738[1])];
    let t741: [uint32x4_t; 2] = [vbicq_u32(live[0], t738[0]), vbicq_u32(live[1], t738[1])];
    if any(&t740) {
        let t742 = [vdupq_n_u64((1u64).wrapping_mul(8u64)); 4];
        let t743: [uint64x2_t; 4] = [vaddq_u64(x24[0], t742[0]), vaddq_u64(x24[1], t742[1]), vaddq_u64(x24[2], t742[2]), vaddq_u64(x24[3], t742[3])];
        let mut t744: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
        t744[0] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t743[0]) as usize as *const u64).read_unaligned(), t744[0]);
        t744[0] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t743[0]) as usize as *const u64).read_unaligned(), t744[0]);
        t744[1] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t743[1]) as usize as *const u64).read_unaligned(), t744[1]);
        t744[1] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t743[1]) as usize as *const u64).read_unaligned(), t744[1]);
        t744[2] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t743[2]) as usize as *const u64).read_unaligned(), t744[2]);
        t744[2] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t743[2]) as usize as *const u64).read_unaligned(), t744[2]);
        t744[3] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t743[3]) as usize as *const u64).read_unaligned(), t744[3]);
        t744[3] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t743[3]) as usize as *const u64).read_unaligned(), t744[3]);
        let t745: [uint32x4_t; 2] = [vsubq_u32(t736[0], x32[0]), vsubq_u32(t736[1], x32[1])];
        let t746: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t745[0])), vmovl_u32(vget_high_u32(t745[0])), vmovl_u32(vget_low_u32(t745[1])), vmovl_u32(vget_high_u32(t745[1]))];
        let t747: [uint64x2_t; 4] = [vshlq_u64(t744[0], vreinterpretq_s64_u64(vandq_u64(t746[0], vdupq_n_u64(63)))), vshlq_u64(t744[1], vreinterpretq_s64_u64(vandq_u64(t746[1], vdupq_n_u64(63)))), vshlq_u64(t744[2], vreinterpretq_s64_u64(vandq_u64(t746[2], vdupq_n_u64(63)))), vshlq_u64(t744[3], vreinterpretq_s64_u64(vandq_u64(t746[3], vdupq_n_u64(63))))];
        let t748: [uint64x2_t; 4] = [vorrq_u64(x40[0], t747[0]), vorrq_u64(x40[1], t747[1]), vorrq_u64(x40[2], t747[2]), vorrq_u64(x40[3], t747[3])];
        t739 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t740[0])))), t748[0], t739[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t740[0])))), t748[1], t739[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t740[1])))), t748[2], t739[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t740[1])))), t748[3], t739[3])];
    }
    {
        t739 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t741[0])))), x40[0], t739[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t741[0])))), x40[1], t739[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t741[1])))), x40[2], t739[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t741[1])))), x40[3], t739[3])];
    }
    res = t739;
    return res;
}

#[inline(always)]
unsafe fn b28_r0ls_f(l: &Launch, lid0: u32, arg0: *const u8, x8: [uint64x2_t; 4], x16: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t749 = [vdupq_n_u64(!(7u64)); 4];
    let t750: [uint64x2_t; 4] = [vandq_u64(x8[0], t749[0]), vandq_u64(x8[1], t749[1]), vandq_u64(x8[2], t749[2]), vandq_u64(x8[3], t749[3])];
    let t751: [uint64x2_t; 4] = [vaddq_u64(vdupq_n_u64((arg0) as u64), t750[0]), vaddq_u64(vdupq_n_u64((arg0) as u64), t750[1]), vaddq_u64(vdupq_n_u64((arg0) as u64), t750[2]), vaddq_u64(vdupq_n_u64((arg0) as u64), t750[3])];
    x24 = t751;
    let t752 = [vdupq_n_u64(7u64); 4];
    let t753: [uint64x2_t; 4] = [vandq_u64(x8[0], t752[0]), vandq_u64(x8[1], t752[1]), vandq_u64(x8[2], t752[2]), vandq_u64(x8[3], t752[3])];
    let t754: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t753[0]), vmovn_u64(t753[1])), vcombine_u32(vmovn_u64(t753[2]), vmovn_u64(t753[3]))];
    let t755 = [vdupq_n_u32(8u32); 2];
    let t756: [uint32x4_t; 2] = [vmulq_u32(t754[0], t755[0]), vmulq_u32(t754[1], t755[1])];
    x32 = t756;
    let mut t757: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    t757[0] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[0]) as usize as *const u64).read_unaligned(), t757[0]);
    t757[0] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[0]) as usize as *const u64).read_unaligned(), t757[0]);
    t757[1] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[1]) as usize as *const u64).read_unaligned(), t757[1]);
    t757[1] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[1]) as usize as *const u64).read_unaligned(), t757[1]);
    t757[2] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[2]) as usize as *const u64).read_unaligned(), t757[2]);
    t757[2] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[2]) as usize as *const u64).read_unaligned(), t757[2]);
    t757[3] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[3]) as usize as *const u64).read_unaligned(), t757[3]);
    t757[3] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[3]) as usize as *const u64).read_unaligned(), t757[3]);
    let t758: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x32[0])), vmovl_u32(vget_high_u32(x32[0])), vmovl_u32(vget_low_u32(x32[1])), vmovl_u32(vget_high_u32(x32[1]))];
    let t759: [uint64x2_t; 4] = [vshlq_u64(t757[0], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t758[0], vdupq_n_u64(63))))), vshlq_u64(t757[1], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t758[1], vdupq_n_u64(63))))), vshlq_u64(t757[2], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t758[2], vdupq_n_u64(63))))), vshlq_u64(t757[3], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t758[3], vdupq_n_u64(63)))))];
    x40 = t759;
    let t760 = [vdupq_n_u32(0u32); 2];
    let t761: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t760[0])), vmvnq_u32(vceqq_u32(x32[1], t760[1]))];
    let t762 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t763: [uint32x4_t; 2] = [vaddq_u32(x32[0], t762[0]), vaddq_u32(x32[1], t762[1])];
    let t764 = [vdupq_n_u32(64u32); 2];
    let t765: [uint32x4_t; 2] = [vcgtq_u32(t763[0], t764[0]), vcgtq_u32(t763[1], t764[1])];
    let t766: [uint32x4_t; 2] = [vandq_u32(t761[0], t765[0]), vandq_u32(t761[1], t765[1])];
    let mut t767: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t768: [uint32x4_t; 2] = [vmvnq_u32(t766[0]), vmvnq_u32(t766[1])];
    if any(&t766) {
        let t769 = [vdupq_n_u64((1u64).wrapping_mul(8u64)); 4];
        let t770: [uint64x2_t; 4] = [vaddq_u64(x24[0], t769[0]), vaddq_u64(x24[1], t769[1]), vaddq_u64(x24[2], t769[2]), vaddq_u64(x24[3], t769[3])];
        let mut t771: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
        t771[0] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t770[0]) as usize as *const u64).read_unaligned(), t771[0]);
        t771[0] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t770[0]) as usize as *const u64).read_unaligned(), t771[0]);
        t771[1] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t770[1]) as usize as *const u64).read_unaligned(), t771[1]);
        t771[1] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t770[1]) as usize as *const u64).read_unaligned(), t771[1]);
        t771[2] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t770[2]) as usize as *const u64).read_unaligned(), t771[2]);
        t771[2] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t770[2]) as usize as *const u64).read_unaligned(), t771[2]);
        t771[3] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t770[3]) as usize as *const u64).read_unaligned(), t771[3]);
        t771[3] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t770[3]) as usize as *const u64).read_unaligned(), t771[3]);
        let t772: [uint32x4_t; 2] = [vsubq_u32(t764[0], x32[0]), vsubq_u32(t764[1], x32[1])];
        let t773: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t772[0])), vmovl_u32(vget_high_u32(t772[0])), vmovl_u32(vget_low_u32(t772[1])), vmovl_u32(vget_high_u32(t772[1]))];
        let t774: [uint64x2_t; 4] = [vshlq_u64(t771[0], vreinterpretq_s64_u64(vandq_u64(t773[0], vdupq_n_u64(63)))), vshlq_u64(t771[1], vreinterpretq_s64_u64(vandq_u64(t773[1], vdupq_n_u64(63)))), vshlq_u64(t771[2], vreinterpretq_s64_u64(vandq_u64(t773[2], vdupq_n_u64(63)))), vshlq_u64(t771[3], vreinterpretq_s64_u64(vandq_u64(t773[3], vdupq_n_u64(63))))];
        let t775: [uint64x2_t; 4] = [vorrq_u64(x40[0], t774[0]), vorrq_u64(x40[1], t774[1]), vorrq_u64(x40[2], t774[2]), vorrq_u64(x40[3], t774[3])];
        t767 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t766[0])))), t775[0], t767[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t766[0])))), t775[1], t767[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t766[1])))), t775[2], t767[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t766[1])))), t775[3], t767[3])];
    }
    {
        t767 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t768[0])))), x40[0], t767[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t768[0])))), x40[1], t767[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t768[1])))), x40[2], t767[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t768[1])))), x40[3], t767[3])];
    }
    res = t767;
    return res;
}

#[inline(always)]
unsafe fn b29_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t776: u64;
    if ((x0) >= (64u32)) {
        t776 = !(0u64);
    } else {
        t776 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t776;
    return res;
}

#[inline(always)]
unsafe fn b29_s_f(l: &Launch, lid0: u32, x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t777: u64;
    if ((x0) >= (64u32)) {
        t777 = !(0u64);
    } else {
        t777 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t777;
    return res;
}

#[inline(always)]
unsafe fn b30_r0sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: u32, x16: [uint32x4_t; 2], x24: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t778 = [vdupq_n_u64(((x8) as u64)); 4];
    let t779: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x16[0])), vmovl_u32(vget_high_u32(x16[0])), vmovl_u32(vget_low_u32(x16[1])), vmovl_u32(vget_high_u32(x16[1]))];
    let t780: [uint64x2_t; 4] = [vaddq_u64(t778[0], t779[0]), vaddq_u64(t778[1], t779[1]), vaddq_u64(t778[2], t779[2]), vaddq_u64(t778[3], t779[3])];
    let t781 = b28_r0ls_p(l, lid0, live, arg0, t780, x24);
    x32 = t781;
    let t782: [uint64x2_t; 4];
    if ((x24) >= (8u32)) {
        t782 = x32;
    } else {
        let t783 = b29_s_p(l, lid0, live, (8u32).wrapping_mul(x24));
        let t784 = [vdupq_n_u64(t783); 4];
        let t785: [uint64x2_t; 4] = [vandq_u64(x32[0], t784[0]), vandq_u64(x32[1], t784[1]), vandq_u64(x32[2], t784[2]), vandq_u64(x32[3], t784[3])];
        t782 = t785;
    }
    res = t782;
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
    let x40: *const u8 = *argv.add(5);
    let arg6: *const u8 = (*argv.add(6) as *const *const u8).read_unaligned();
    let x64: u32 = (*argv.add(7) as *const u32).read_unaligned();
    let mut t786: bool = !(arg6).is_null();
    if t786 {
        let mut t787: bool = ((l.group[0]) >= ((arg6.wrapping_add((((x64) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        if !t787 {
            t787 = ((l.group[1]) >= ((arg6.wrapping_add(((((x64).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        let mut t788: bool = t787;
        if !t788 {
            t788 = ((l.group[2]) >= ((arg6.wrapping_add(((((x64).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        t786 = t788;
    }
    if t786 {
        return;
    }
    let mut lid0 = 0u32;
    while lid0 + 8 <= n {
        let mut live = [vdupq_n_u32(u32::MAX); 2];
        let gid0 = l.group[0] * l.block[0] + lid0;
        'chunk: {
            b0_r0r1r2r3r4sgi_f(l, lid0, arg0, arg1, arg2, arg3, arg4, x40, lid0);
        }
        lid0 += 8;
    }
    if lid0 < n {
        // Lanes part: the group's last threads, those under `n`.
        let t789: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
        let mut live = t789;
        let gid0 = l.group[0] * l.block[0] + lid0;
        'chunk: {
            b0_r0r1r2r3r4sgi_p(l, lid0, live, arg0, arg1, arg2, arg3, arg4, x40, lid0);
        }
    }
}
