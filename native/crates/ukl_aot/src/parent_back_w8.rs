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
unsafe fn b0_r0r1r2r3r4sgi_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, arg2: *const u8, arg3: *const u8, arg4: *const u8, x40: *const u8, x64: u32) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x40 as *const u32).read_unaligned();
    let mut x72: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x80: bool = false;
    let mut live = k;
    let t1: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x64), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x64), vld1q_u32(LANE[1].as_ptr()))];
    let t2 = [vdupq_n_u32(31u32); 2];
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
    let t9 = [vdupq_n_u32(31u32); 2];
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
unsafe fn b0_r0r1r2r3r4sgu5_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, arg2: *const u8, arg3: *const u8, arg4: *const u8, x40: *const u8, x64: u32) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x40 as *const u32).read_unaligned();
    let mut x72: u32 = 0;
    let mut x80: bool = false;
    let mut live = k;
    x72 = (x64 & 31u32);
    let t15 = b1_r3sso4_p(l, lid0, live, arg3, (((a0).wrapping_add(1023u32)).checked_div(1024u32).unwrap_or(0)).wrapping_add(1u32), 0u32, arg4.wrapping_add((32u64).wrapping_mul(4u64) as usize));
    x80 = t15;
    if !(x80) {
        let t16 = b2_o4s_p(l, lid0, live, arg4.wrapping_add((96u64).wrapping_mul(4u64) as usize), 1u32);
    }
    let t17: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
    let t18: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x64), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x64), vld1q_u32(LANE[1].as_ptr()))];
    let t19: [uint32x4_t; 2] = [vsubq_u32(t17[0], t18[0]), vsubq_u32(t17[1], t18[1])];
    b3_r0r1r2r3sslig_p(l, lid0, live, arg0, arg1, arg2, arg3, a0, x80, t19, x72);
}

#[inline(always)]
unsafe fn b0_r0r1r2r3r4sgu5_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, arg2: *const u8, arg3: *const u8, arg4: *const u8, x40: *const u8, x64: u32) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x40 as *const u32).read_unaligned();
    let mut x72: u32 = 0;
    let mut x80: bool = false;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    x72 = (x64 & 31u32);
    let t20 = b1_r3sso4_f(l, lid0, arg3, (((a0).wrapping_add(1023u32)).checked_div(1024u32).unwrap_or(0)).wrapping_add(1u32), 0u32, arg4.wrapping_add((32u64).wrapping_mul(4u64) as usize));
    x80 = t20;
    if !(x80) {
        let t21 = b2_o4s_f(l, lid0, arg4.wrapping_add((96u64).wrapping_mul(4u64) as usize), 1u32);
    }
    let t22: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
    let t23: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x64), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x64), vld1q_u32(LANE[1].as_ptr()))];
    let t24: [uint32x4_t; 2] = [vsubq_u32(t22[0], t23[0]), vsubq_u32(t22[1], t23[1])];
    b3_r0r1r2r3sslig_f(l, lid0, arg0, arg1, arg2, arg3, a0, x80, t24, x72);
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
    let mut t25 = live;
    let mut t26 = [vdupq_n_u32(0); 2];
    loop {
        // Lanes part: each its own rounds.
        let t27 = [vdupq_n_u32(if true { u32::MAX } else { 0 }); 2];
        t25 = [vandq_u32(vbicq_u32(t25[0], t26[0]), t27[0]), vandq_u32(vbicq_u32(t25[1], t26[1]), t27[1])];
        if !(any(&t25)) { break; }
        let t29 = b4_o4_p(l, lid0, t25, arg4_x24.wrapping_add((0u64).wrapping_mul(4u64) as usize));
        let t30: [uint32x4_t; 2];
        if ((t29) < (x32)) {
            let t31 = b2_o4s_p(l, lid0, t25, arg4_x24.wrapping_add((0u64).wrapping_mul(4u64) as usize), 1u32);
            t30 = t31;
        } else {
            let t32 = [vdupq_n_u32(x32); 2];
            t30 = t32;
        }
        x40 = [vbslq_u32(t25[0], t30[0], x40[0]), vbslq_u32(t25[1], t30[1], x40[1])];
        let t33 = [vdupq_n_u32(x32); 2];
        let t34: [uint32x4_t; 2] = [vcgeq_u32(x40[0], t33[0]), vcgeq_u32(x40[1], t33[1])];
        let t35: [uint32x4_t; 2] = [vandq_u32(t25[0], t34[0]), vandq_u32(t25[1], t34[1])];
        if any(&t35) {
            t26 = [vorrq_u32(t26[0], t35[0]), vorrq_u32(t26[1], t35[1])];
        }
        let t36: [uint32x4_t; 2] = [vbicq_u32(vandq_u32(t25[0], live[0]), t26[0]), vbicq_u32(vandq_u32(t25[1], live[1]), t26[1])];
        if any(&t36) {
            let t37 = [vdupq_n_u32(256u32); 2];
            let t38: [uint32x4_t; 2] = [vmulq_u32(x40[0], t37[0]), vmulq_u32(x40[1], t37[1])];
            let t39: [uint32x4_t; 2] = [vaddq_u32(t38[0], t37[0]), vaddq_u32(t38[1], t37[1])];
            let t40 = [vdupq_n_u32(x8); 2];
            let t41: [uint32x4_t; 2] = [vcltq_u32(t39[0], t40[0]), vcltq_u32(t39[1], t40[1])];
            let mut t42: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t43: [uint32x4_t; 2] = [vandq_u32(t36[0], t41[0]), vandq_u32(t36[1], t41[1])];
            let t44: [uint32x4_t; 2] = [vbicq_u32(t36[0], t41[0]), vbicq_u32(t36[1], t41[1])];
            {
                let t45: [uint32x4_t; 2] = [vmulq_u32(x40[0], t37[0]), vmulq_u32(x40[1], t37[1])];
                let t46: [uint32x4_t; 2] = [vaddq_u32(t45[0], t37[0]), vaddq_u32(t45[1], t37[1])];
                t42 = [vbslq_u32(t43[0], t46[0], t42[0]), vbslq_u32(t43[1], t46[1], t42[1])];
            }
            {
                let t47 = [vdupq_n_u32(x8); 2];
                t42 = [vbslq_u32(t44[0], t47[0], t42[0]), vbslq_u32(t44[1], t47[1], t42[1])];
            }
            x48 = t42;
            let t48: [uint32x4_t; 2] = [vmulq_u32(x40[0], t37[0]), vmulq_u32(x40[1], t37[1])];
            x56 = t48;
            let mut t50 = t36;
            loop {
                // Lanes part: each its own rounds.
                let t52: [uint32x4_t; 2] = [vcltq_u32(x56[0], x48[0]), vcltq_u32(x56[1], x48[1])];
                t50 = [vandq_u32(t50[0], t52[0]), vandq_u32(t50[1], t52[1])];
                if !(any(&t50)) { break; }
                let t54 = [vdupq_n_u32(x16); 2];
                let mut t55w = 0u32;
                let t55 = core::hint::black_box(&raw mut t55w);
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(t50[0]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<0>(x56[0]) as usize), t55).write(vgetq_lane_u32::<0>(t54[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(t50[0]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<1>(x56[0]) as usize), t55).write(vgetq_lane_u32::<1>(t54[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(t50[0]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<2>(x56[0]) as usize), t55).write(vgetq_lane_u32::<2>(t54[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(t50[0]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<3>(x56[0]) as usize), t55).write(vgetq_lane_u32::<3>(t54[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(t50[1]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<0>(x56[1]) as usize), t55).write(vgetq_lane_u32::<0>(t54[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(t50[1]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<1>(x56[1]) as usize), t55).write(vgetq_lane_u32::<1>(t54[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(t50[1]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<2>(x56[1]) as usize), t55).write(vgetq_lane_u32::<2>(t54[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(t50[1]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<3>(x56[1]) as usize), t55).write(vgetq_lane_u32::<3>(t54[1]));
                let t56: [uint32x4_t; 2] = [vandq_u32(t50[0], live[0]), vandq_u32(t50[1], live[1])];
                let t57 = x56;
                x56 = [vsubq_u32(x56[0], t56[0]), vsubq_u32(x56[1], t56[1])];
            }
            b5__p(l, lid0, t36);
            let t58 = b2_o4s_p(l, lid0, t36, arg4_x24.wrapping_add((32u64).wrapping_mul(4u64) as usize), 1u32);
        }
        let t59: [uint32x4_t; 2] = [vbicq_u32(vandq_u32(t25[0], live[0]), t26[0]), vbicq_u32(vandq_u32(t25[1], live[1]), t26[1])];
    }
    if any(&live) {
        let t60 = [vdupq_n_u32(0u32); 2];
        x40 = [vbslq_u32(live[0], t60[0], x40[0]), vbslq_u32(live[1], t60[1], x40[1])];
        let mut t61 = live;
        loop {
            // Lanes part: each its own rounds.
            let t63 = [vdupq_n_u32(65536u32); 2];
            let t64: [uint32x4_t; 2] = [vcltq_u32(x40[0], t63[0]), vcltq_u32(x40[1], t63[1])];
            t61 = [vandq_u32(t61[0], t64[0]), vandq_u32(t61[1], t64[1])];
            if !(any(&t61)) { break; }
            let t66 = b4_o4_p(l, lid0, t61, arg4_x24.wrapping_add((32u64).wrapping_mul(4u64) as usize));
            if ((t66) >= (x32)) {
                res = ((1i32) != 0);
                live = [vbicq_u32(live[0], t61[0]), vbicq_u32(live[1], t61[1])];
            }
            let t67: [uint32x4_t; 2] = [vandq_u32(t61[0], live[0]), vandq_u32(t61[1], live[1])];
            let t68 = [vdupq_n_u32(1u32); 2];
            let t69: [uint32x4_t; 2] = [vaddq_u32(x40[0], t68[0]), vaddq_u32(x40[1], t68[1])];
            let t70 = x40;
            x40 = t69;
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
unsafe fn b1_r3sso4_f(l: &Launch, lid0: u32, arg3: *const u8, x8: u32, x16: u32, arg4_x24: *const u8) -> bool {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x56: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: bool = false;
    x32 = (((x8).wrapping_add(256u32)).wrapping_sub(1u32)).checked_div(256u32).unwrap_or(0);
    let mut t71 = live;
    let mut t72 = [vdupq_n_u32(0); 2];
    loop {
        // Lanes part: each its own rounds.
        let t73 = [vdupq_n_u32(if true { u32::MAX } else { 0 }); 2];
        t71 = [vandq_u32(vbicq_u32(t71[0], t72[0]), t73[0]), vandq_u32(vbicq_u32(t71[1], t72[1]), t73[1])];
        if !(any(&t71)) { break; }
        let t75 = b4_o4_p(l, lid0, t71, arg4_x24.wrapping_add((0u64).wrapping_mul(4u64) as usize));
        let t76: [uint32x4_t; 2];
        if ((t75) < (x32)) {
            let t77 = b2_o4s_p(l, lid0, t71, arg4_x24.wrapping_add((0u64).wrapping_mul(4u64) as usize), 1u32);
            t76 = t77;
        } else {
            let t78 = [vdupq_n_u32(x32); 2];
            t76 = t78;
        }
        x40 = [vbslq_u32(t71[0], t76[0], x40[0]), vbslq_u32(t71[1], t76[1], x40[1])];
        let t79 = [vdupq_n_u32(x32); 2];
        let t80: [uint32x4_t; 2] = [vcgeq_u32(x40[0], t79[0]), vcgeq_u32(x40[1], t79[1])];
        let t81: [uint32x4_t; 2] = [vandq_u32(t71[0], t80[0]), vandq_u32(t71[1], t80[1])];
        if any(&t81) {
            t72 = [vorrq_u32(t72[0], t81[0]), vorrq_u32(t72[1], t81[1])];
        }
        let t82: [uint32x4_t; 2] = [vbicq_u32(vandq_u32(t71[0], live[0]), t72[0]), vbicq_u32(vandq_u32(t71[1], live[1]), t72[1])];
        if any(&t82) {
            let t83 = [vdupq_n_u32(256u32); 2];
            let t84: [uint32x4_t; 2] = [vmulq_u32(x40[0], t83[0]), vmulq_u32(x40[1], t83[1])];
            let t85: [uint32x4_t; 2] = [vaddq_u32(t84[0], t83[0]), vaddq_u32(t84[1], t83[1])];
            let t86 = [vdupq_n_u32(x8); 2];
            let t87: [uint32x4_t; 2] = [vcltq_u32(t85[0], t86[0]), vcltq_u32(t85[1], t86[1])];
            let mut t88: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t89: [uint32x4_t; 2] = [vandq_u32(t82[0], t87[0]), vandq_u32(t82[1], t87[1])];
            let t90: [uint32x4_t; 2] = [vbicq_u32(t82[0], t87[0]), vbicq_u32(t82[1], t87[1])];
            {
                let t91: [uint32x4_t; 2] = [vmulq_u32(x40[0], t83[0]), vmulq_u32(x40[1], t83[1])];
                let t92: [uint32x4_t; 2] = [vaddq_u32(t91[0], t83[0]), vaddq_u32(t91[1], t83[1])];
                t88 = [vbslq_u32(t89[0], t92[0], t88[0]), vbslq_u32(t89[1], t92[1], t88[1])];
            }
            {
                let t93 = [vdupq_n_u32(x8); 2];
                t88 = [vbslq_u32(t90[0], t93[0], t88[0]), vbslq_u32(t90[1], t93[1], t88[1])];
            }
            x48 = t88;
            let t94: [uint32x4_t; 2] = [vmulq_u32(x40[0], t83[0]), vmulq_u32(x40[1], t83[1])];
            x56 = t94;
            let mut t96 = t82;
            loop {
                // Lanes part: each its own rounds.
                let t98: [uint32x4_t; 2] = [vcltq_u32(x56[0], x48[0]), vcltq_u32(x56[1], x48[1])];
                t96 = [vandq_u32(t96[0], t98[0]), vandq_u32(t96[1], t98[1])];
                if !(any(&t96)) { break; }
                let t100 = [vdupq_n_u32(x16); 2];
                let mut t101w = 0u32;
                let t101 = core::hint::black_box(&raw mut t101w);
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(t96[0]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<0>(x56[0]) as usize), t101).write(vgetq_lane_u32::<0>(t100[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(t96[0]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<1>(x56[0]) as usize), t101).write(vgetq_lane_u32::<1>(t100[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(t96[0]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<2>(x56[0]) as usize), t101).write(vgetq_lane_u32::<2>(t100[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(t96[0]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<3>(x56[0]) as usize), t101).write(vgetq_lane_u32::<3>(t100[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(t96[1]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<0>(x56[1]) as usize), t101).write(vgetq_lane_u32::<0>(t100[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(t96[1]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<1>(x56[1]) as usize), t101).write(vgetq_lane_u32::<1>(t100[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(t96[1]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<2>(x56[1]) as usize), t101).write(vgetq_lane_u32::<2>(t100[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(t96[1]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<3>(x56[1]) as usize), t101).write(vgetq_lane_u32::<3>(t100[1]));
                let t102: [uint32x4_t; 2] = [vandq_u32(t96[0], live[0]), vandq_u32(t96[1], live[1])];
                let t103 = x56;
                x56 = [vsubq_u32(x56[0], t102[0]), vsubq_u32(x56[1], t102[1])];
            }
            b5__p(l, lid0, t82);
            let t104 = b2_o4s_p(l, lid0, t82, arg4_x24.wrapping_add((32u64).wrapping_mul(4u64) as usize), 1u32);
        }
        let t105: [uint32x4_t; 2] = [vbicq_u32(vandq_u32(t71[0], live[0]), t72[0]), vbicq_u32(vandq_u32(t71[1], live[1]), t72[1])];
    }
    if any(&live) {
        let t106 = [vdupq_n_u32(0u32); 2];
        x40 = [vbslq_u32(live[0], t106[0], x40[0]), vbslq_u32(live[1], t106[1], x40[1])];
        let mut t107 = live;
        loop {
            // Lanes part: each its own rounds.
            let t109 = [vdupq_n_u32(65536u32); 2];
            let t110: [uint32x4_t; 2] = [vcltq_u32(x40[0], t109[0]), vcltq_u32(x40[1], t109[1])];
            t107 = [vandq_u32(t107[0], t110[0]), vandq_u32(t107[1], t110[1])];
            if !(any(&t107)) { break; }
            let t112 = b4_o4_p(l, lid0, t107, arg4_x24.wrapping_add((32u64).wrapping_mul(4u64) as usize));
            if ((t112) >= (x32)) {
                res = ((1i32) != 0);
                live = [vbicq_u32(live[0], t107[0]), vbicq_u32(live[1], t107[1])];
            }
            let t113: [uint32x4_t; 2] = [vandq_u32(t107[0], live[0]), vandq_u32(t107[1], live[1])];
            let t114 = [vdupq_n_u32(1u32); 2];
            let t115: [uint32x4_t; 2] = [vaddq_u32(x40[0], t114[0]), vaddq_u32(x40[1], t114[1])];
            let t116 = x40;
            x40 = t115;
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
unsafe fn b2_o4s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg4_x0: *const u8, x8: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut t117 = [0u32; 8];
    for r in 0..2 { vst1q_u32(t117.as_mut_ptr().add(4 * r), live[r]); }
    let mut t118 = [0u32; 8];
    let mut t119 = 0u32;
    for j in 0..8 { t118[j] = t119; t119 += t117[j] & 1; }
    let t120: [uint32x4_t; 2] = [vld1q_u32(t118.as_ptr().add(0)), vld1q_u32(t118.as_ptr().add(4))];
    let t121 = AtomicU32::from_ptr(arg4_x0 as *mut u32).fetch_add((x8).wrapping_mul(t119), Relaxed);
    let t122: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(t121), t120[0], vdupq_n_u32(x8)), vmlaq_u32(vdupq_n_u32(t121), t120[1], vdupq_n_u32(x8))];
    res = t122;
    return res;
}

#[inline(always)]
unsafe fn b2_o4s_f(l: &Launch, lid0: u32, arg4_x0: *const u8, x8: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t123: [uint32x4_t; 2] = [vld1q_u32(LANE[0].as_ptr()), vld1q_u32(LANE[1].as_ptr())];
    let t124 = AtomicU32::from_ptr(arg4_x0 as *mut u32).fetch_add((x8).wrapping_mul(8u32), Relaxed);
    let t125: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(t124), t123[0], vdupq_n_u32(x8)), vmlaq_u32(vdupq_n_u32(t124), t123[1], vdupq_n_u32(x8))];
    res = t125;
    return res;
}

#[inline(always)]
unsafe fn b3_r0r1r2r3sslig_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, arg2: *const u8, arg3: *const u8, x32: u32, x40: bool, x48: [uint32x4_t; 2], x56: u32) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x72: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x80: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x88: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x96: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x104: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    if !(x40) {
        let t126: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t127 = [vdupq_n_u32(x32); 2];
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
        x72 = t131;
        let t132: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t133 = [vdupq_n_u32(x32); 2];
        let t134: [uint32x4_t; 2] = [vcltq_u32(t132[0], t133[0]), vcltq_u32(t132[1], t133[1])];
        let t135: [uint32x4_t; 2] = [vandq_u32(live[0], t134[0]), vandq_u32(live[1], t134[1])];
        if any(&t135) {
            let t136 = b8_r0r1g_p(l, lid0, t135, arg0, arg1);
            x80 = [vbslq_u32(t135[0], t136[0], x80[0]), vbslq_u32(t135[1], t136[1], x80[1]), vbslq_u32(t135[0], t136[2], x80[2]), vbslq_u32(t135[1], t136[3], x80[3])];
            let t137: u64 = (1u64).wrapping_shl((32u64) as u32);
            let t138 = [vdupq_n_u32((t137) as u32); 2];
            let t139 = [vdupq_n_u32(((t137) >> 32) as u32); 2];
            let t140: [uint32x4_t; 4] = [vandq_u32(x80[0], t138[0]), vandq_u32(x80[1], t138[1]), vandq_u32(x80[2], t139[0]), vandq_u32(x80[3], t139[1])];
            let t141: u64 = 0u64;
            let t142 = [vdupq_n_u32((t141) as u32); 2];
            let t143: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t140[0], t142[0]), vceqq_u32(t140[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t140[1], t142[1]), vceqq_u32(t140[3], vdupq_n_u32(0)))];
            let t144: [uint32x4_t; 2] = [vmvnq_u32(t143[0]), vmvnq_u32(t143[1])];
            let mut t145: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t146: [uint32x4_t; 2] = [vandq_u32(t135[0], t144[0]), vandq_u32(t135[1], t144[1])];
            let t147: [uint32x4_t; 2] = [vbicq_u32(t135[0], t144[0]), vbicq_u32(t135[1], t144[1])];
            {
                let t148: [uint32x4_t; 2] = [x80[0], x80[1]];
                let t149 = [vdupq_n_u32(2147483648u32); 2];
                let t150: [uint32x4_t; 2] = [vorrq_u32(t149[0], t148[0]), vorrq_u32(t149[1], t148[1])];
                t145 = [vbslq_u32(t146[0], t150[0], t145[0]), vbslq_u32(t146[1], t150[1], t145[1])];
            }
            {
                let t151: [uint32x4_t; 2] = [x80[0], x80[1]];
                t145 = [vbslq_u32(t147[0], t151[0], t145[0]), vbslq_u32(t147[1], t151[1], t145[1])];
            }
            x72 = [vbslq_u32(t135[0], t145[0], x72[0]), vbslq_u32(t135[1], t145[1], x72[1])];
            b6_ag4r2l_p(l, lid0, t135, arg2, x72);
        }
        b9_r3gi_p(l, lid0, live, arg3, x56);
        let t152 = [vdupq_n_u32(x32); 2];
        let t153: [uint32x4_t; 2] = [vcgeq_u32(t132[0], t152[0]), vcgeq_u32(t132[1], t152[1])];
        let t154: [uint32x4_t; 2] = [vandq_u32(live[0], t153[0]), vandq_u32(live[1], t153[1])];
        {
            live = [vbicq_u32(live[0], t154[0]), vbicq_u32(live[1], t154[1])];
        }
        if any(&live) {
            let t155: u64 = ((0u32) as u64);
            let t156 = [vdupq_n_u32((t155) as u32); 2];
            let t157: [uint32x4_t; 4] = [t156[0], t156[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            x80 = t157;
            let mut t169 = live;
            loop {
                // Lanes part: each its own rounds.
                let t171: [uint32x4_t; 2] = [x80[0], x80[1]];
                let t172 = [vdupq_n_u32(7u32); 2];
                let t173: [uint32x4_t; 2] = [vcltq_u32(t171[0], t172[0]), vcltq_u32(t171[1], t172[1])];
                let t174 = [vdupq_n_u32(2147483648u32); 2];
                let t175: [uint32x4_t; 2] = [vandq_u32(x72[0], t174[0]), vandq_u32(x72[1], t174[1])];
                let t176 = [vdupq_n_u32(0u32); 2];
                let t177: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t175[0], t176[0])), vmvnq_u32(vceqq_u32(t175[1], t176[1]))];
                let t178 = [vdupq_n_u32(4294967294u32); 2];
                let t179: [uint32x4_t; 2] = [vcltq_u32(x72[0], t178[0]), vcltq_u32(x72[1], t178[1])];
                let t180: [uint32x4_t; 2] = [vandq_u32(t177[0], t179[0]), vandq_u32(t177[1], t179[1])];
                let t181: [uint32x4_t; 2] = [vandq_u32(t173[0], t180[0]), vandq_u32(t173[1], t180[1])];
                t169 = [vandq_u32(t169[0], t181[0]), vandq_u32(t169[1], t181[1])];
                if !(any(&t169)) { break; }
                let t183 = [vdupq_n_u32(2147483647u32); 2];
                let t184: [uint32x4_t; 2] = [vandq_u32(x72[0], t183[0]), vandq_u32(x72[1], t183[1])];
                x88 = t184;
                x96 = t178;
                let t185 = b10_r3ll_p(l, lid0, t169, arg3, x88, x48);
                let t186: [uint32x4_t; 2] = [vandq_u32(t169[0], t185[0]), vandq_u32(t169[1], t185[1])];
                if any(&t186) {
                    let t187 = b11_al4r2_p(l, lid0, t186, arg2, x88);
                    x96 = [vbslq_u32(t186[0], t187[0], x96[0]), vbslq_u32(t186[1], t187[1], x96[1])];
                }
                let t188: [uint32x4_t; 2] = [vceqq_u32(x96[0], t178[0]), vceqq_u32(x96[1], t178[1])];
                let t189: [uint32x4_t; 2] = [vandq_u32(t169[0], t188[0]), vandq_u32(t169[1], t188[1])];
                if any(&t189) {
                    let t190 = b8_r0r1l_p(l, lid0, t189, arg0, arg1, x88);
                    x104 = t190;
                    let t191: u64 = (1u64).wrapping_shl((32u64) as u32);
                    let t192 = [vdupq_n_u32((t191) as u32); 2];
                    let t193 = [vdupq_n_u32(((t191) >> 32) as u32); 2];
                    let t194: [uint32x4_t; 4] = [vandq_u32(x104[0], t192[0]), vandq_u32(x104[1], t192[1]), vandq_u32(x104[2], t193[0]), vandq_u32(x104[3], t193[1])];
                    let t195: u64 = 0u64;
                    let t196 = [vdupq_n_u32((t195) as u32); 2];
                    let t197: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t194[0], t196[0]), vceqq_u32(t194[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t194[1], t196[1]), vceqq_u32(t194[3], vdupq_n_u32(0)))];
                    let t198: [uint32x4_t; 2] = [vmvnq_u32(t197[0]), vmvnq_u32(t197[1])];
                    let mut t199: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                    let t200: [uint32x4_t; 2] = [vandq_u32(t189[0], t198[0]), vandq_u32(t189[1], t198[1])];
                    let t201: [uint32x4_t; 2] = [vbicq_u32(t189[0], t198[0]), vbicq_u32(t189[1], t198[1])];
                    {
                        let t202: [uint32x4_t; 2] = [x104[0], x104[1]];
                        let t203: [uint32x4_t; 2] = [vorrq_u32(t174[0], t202[0]), vorrq_u32(t174[1], t202[1])];
                        t199 = [vbslq_u32(t200[0], t203[0], t199[0]), vbslq_u32(t200[1], t203[1], t199[1])];
                    }
                    {
                        let t204: [uint32x4_t; 2] = [x104[0], x104[1]];
                        t199 = [vbslq_u32(t201[0], t204[0], t199[0]), vbslq_u32(t201[1], t204[1], t199[1])];
                    }
                    x96 = [vbslq_u32(t189[0], t199[0], x96[0]), vbslq_u32(t189[1], t199[1], x96[1])];
                }
                x72 = [vbslq_u32(t169[0], x96[0], x72[0]), vbslq_u32(t169[1], x96[1], x72[1])];
                let t205: [uint32x4_t; 2] = [vandq_u32(t169[0], live[0]), vandq_u32(t169[1], live[1])];
                let t206: [uint32x4_t; 2] = [x80[0], x80[1]];
                let t207 = [vdupq_n_u32(1u32); 2];
                let t208: [uint32x4_t; 2] = [vaddq_u32(t206[0], t207[0]), vaddq_u32(t206[1], t207[1])];
                let t209 = x80;
                let t210: [uint32x4_t; 4] = [t208[0], t208[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                x80 = t210;
            }
            let t211 = [vdupq_n_u32(2147483648u32); 2];
            let t212: [uint32x4_t; 2] = [vandq_u32(x72[0], t211[0]), vandq_u32(x72[1], t211[1])];
            let t213 = [vdupq_n_u32(0u32); 2];
            let t214: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t212[0], t213[0])), vmvnq_u32(vceqq_u32(t212[1], t213[1]))];
            let t215 = [vdupq_n_u32(4294967294u32); 2];
            let t216: [uint32x4_t; 2] = [vcltq_u32(x72[0], t215[0]), vcltq_u32(x72[1], t215[1])];
            let t217: [uint32x4_t; 2] = [vandq_u32(t214[0], t216[0]), vandq_u32(t214[1], t216[1])];
            let t218: [uint32x4_t; 2] = [vandq_u32(live[0], t217[0]), vandq_u32(live[1], t217[1])];
            if any(&t218) {
                let t219 = [vdupq_n_u32(2147483647u32); 2];
                let t220: [uint32x4_t; 2] = [vandq_u32(x72[0], t219[0]), vandq_u32(x72[1], t219[1])];
                let t221 = b7_r0r1l_p(l, lid0, t218, arg0, arg1, t220);
                x72 = [vbslq_u32(t218[0], t221[0], x72[0]), vbslq_u32(t218[1], t221[1], x72[1])];
            }
            b6_ag4r2l_p(l, lid0, live, arg2, x72);
        }
    }
}

#[inline(always)]
unsafe fn b3_r0r1r2r3sslig_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, arg2: *const u8, arg3: *const u8, x32: u32, x40: bool, x48: [uint32x4_t; 2], x56: u32) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x72: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x80: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x88: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x96: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x104: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    if !(x40) {
        if (((gid0) as u64 + 7) <= u32::MAX as u64 && ((gid0) as u64) >= ((x32) as u64)) {
        } else if ((gid0) as u64 + 7) < ((x32) as u64) {
            let t222 = b7_r0r1g_f(l, lid0, arg0, arg1);
            b6_ag4r2l_f(l, lid0, arg2, t222);
        } else {
            // Lanes part: the chunk's lanes on both sides of the bound.
            let t223: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
            let t224 = [vdupq_n_u32(x32); 2];
            let t225: [uint32x4_t; 2] = [vcltq_u32(t223[0], t224[0]), vcltq_u32(t223[1], t224[1])];
            let t226: [uint32x4_t; 2] = [vandq_u32(live[0], t225[0]), vandq_u32(live[1], t225[1])];
            if any(&t226) {
                let t227 = b7_r0r1g_p(l, lid0, t226, arg0, arg1);
                b6_ag4r2l_p(l, lid0, t226, arg2, t227);
            }
        }
        return;
    }
    if any(&live) {
        let t228 = [vdupq_n_u32(4294967295u32); 2];
        x72 = t228;
        let t229: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t230 = [vdupq_n_u32(x32); 2];
        let t231: [uint32x4_t; 2] = [vcltq_u32(t229[0], t230[0]), vcltq_u32(t229[1], t230[1])];
        let t232: [uint32x4_t; 2] = [vandq_u32(live[0], t231[0]), vandq_u32(live[1], t231[1])];
        if any(&t232) {
            let t233 = b8_r0r1g_p(l, lid0, t232, arg0, arg1);
            x80 = [vbslq_u32(t232[0], t233[0], x80[0]), vbslq_u32(t232[1], t233[1], x80[1]), vbslq_u32(t232[0], t233[2], x80[2]), vbslq_u32(t232[1], t233[3], x80[3])];
            let t234: u64 = (1u64).wrapping_shl((32u64) as u32);
            let t235 = [vdupq_n_u32((t234) as u32); 2];
            let t236 = [vdupq_n_u32(((t234) >> 32) as u32); 2];
            let t237: [uint32x4_t; 4] = [vandq_u32(x80[0], t235[0]), vandq_u32(x80[1], t235[1]), vandq_u32(x80[2], t236[0]), vandq_u32(x80[3], t236[1])];
            let t238: u64 = 0u64;
            let t239 = [vdupq_n_u32((t238) as u32); 2];
            let t240: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t237[0], t239[0]), vceqq_u32(t237[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t237[1], t239[1]), vceqq_u32(t237[3], vdupq_n_u32(0)))];
            let t241: [uint32x4_t; 2] = [vmvnq_u32(t240[0]), vmvnq_u32(t240[1])];
            let mut t242: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t243: [uint32x4_t; 2] = [vandq_u32(t232[0], t241[0]), vandq_u32(t232[1], t241[1])];
            let t244: [uint32x4_t; 2] = [vbicq_u32(t232[0], t241[0]), vbicq_u32(t232[1], t241[1])];
            {
                let t245: [uint32x4_t; 2] = [x80[0], x80[1]];
                let t246 = [vdupq_n_u32(2147483648u32); 2];
                let t247: [uint32x4_t; 2] = [vorrq_u32(t246[0], t245[0]), vorrq_u32(t246[1], t245[1])];
                t242 = [vbslq_u32(t243[0], t247[0], t242[0]), vbslq_u32(t243[1], t247[1], t242[1])];
            }
            {
                let t248: [uint32x4_t; 2] = [x80[0], x80[1]];
                t242 = [vbslq_u32(t244[0], t248[0], t242[0]), vbslq_u32(t244[1], t248[1], t242[1])];
            }
            x72 = [vbslq_u32(t232[0], t242[0], x72[0]), vbslq_u32(t232[1], t242[1], x72[1])];
            b6_ag4r2l_p(l, lid0, t232, arg2, x72);
        }
        b9_r3gi_p(l, lid0, live, arg3, x56);
        let t249 = [vdupq_n_u32(x32); 2];
        let t250: [uint32x4_t; 2] = [vcgeq_u32(t229[0], t249[0]), vcgeq_u32(t229[1], t249[1])];
        let t251: [uint32x4_t; 2] = [vandq_u32(live[0], t250[0]), vandq_u32(live[1], t250[1])];
        {
            live = [vbicq_u32(live[0], t251[0]), vbicq_u32(live[1], t251[1])];
        }
        if any(&live) {
            let t252: u64 = ((0u32) as u64);
            let t253 = [vdupq_n_u32((t252) as u32); 2];
            let t254: [uint32x4_t; 4] = [t253[0], t253[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            x80 = t254;
            let mut t266 = live;
            loop {
                // Lanes part: each its own rounds.
                let t268: [uint32x4_t; 2] = [x80[0], x80[1]];
                let t269 = [vdupq_n_u32(7u32); 2];
                let t270: [uint32x4_t; 2] = [vcltq_u32(t268[0], t269[0]), vcltq_u32(t268[1], t269[1])];
                let t271 = [vdupq_n_u32(2147483648u32); 2];
                let t272: [uint32x4_t; 2] = [vandq_u32(x72[0], t271[0]), vandq_u32(x72[1], t271[1])];
                let t273 = [vdupq_n_u32(0u32); 2];
                let t274: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t272[0], t273[0])), vmvnq_u32(vceqq_u32(t272[1], t273[1]))];
                let t275 = [vdupq_n_u32(4294967294u32); 2];
                let t276: [uint32x4_t; 2] = [vcltq_u32(x72[0], t275[0]), vcltq_u32(x72[1], t275[1])];
                let t277: [uint32x4_t; 2] = [vandq_u32(t274[0], t276[0]), vandq_u32(t274[1], t276[1])];
                let t278: [uint32x4_t; 2] = [vandq_u32(t270[0], t277[0]), vandq_u32(t270[1], t277[1])];
                t266 = [vandq_u32(t266[0], t278[0]), vandq_u32(t266[1], t278[1])];
                if !(any(&t266)) { break; }
                let t280 = [vdupq_n_u32(2147483647u32); 2];
                let t281: [uint32x4_t; 2] = [vandq_u32(x72[0], t280[0]), vandq_u32(x72[1], t280[1])];
                x88 = t281;
                x96 = t275;
                let t282 = b10_r3ll_p(l, lid0, t266, arg3, x88, x48);
                let t283: [uint32x4_t; 2] = [vandq_u32(t266[0], t282[0]), vandq_u32(t266[1], t282[1])];
                if any(&t283) {
                    let t284 = b11_al4r2_p(l, lid0, t283, arg2, x88);
                    x96 = [vbslq_u32(t283[0], t284[0], x96[0]), vbslq_u32(t283[1], t284[1], x96[1])];
                }
                let t285: [uint32x4_t; 2] = [vceqq_u32(x96[0], t275[0]), vceqq_u32(x96[1], t275[1])];
                let t286: [uint32x4_t; 2] = [vandq_u32(t266[0], t285[0]), vandq_u32(t266[1], t285[1])];
                if any(&t286) {
                    let t287 = b8_r0r1l_p(l, lid0, t286, arg0, arg1, x88);
                    x104 = t287;
                    let t288: u64 = (1u64).wrapping_shl((32u64) as u32);
                    let t289 = [vdupq_n_u32((t288) as u32); 2];
                    let t290 = [vdupq_n_u32(((t288) >> 32) as u32); 2];
                    let t291: [uint32x4_t; 4] = [vandq_u32(x104[0], t289[0]), vandq_u32(x104[1], t289[1]), vandq_u32(x104[2], t290[0]), vandq_u32(x104[3], t290[1])];
                    let t292: u64 = 0u64;
                    let t293 = [vdupq_n_u32((t292) as u32); 2];
                    let t294: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t291[0], t293[0]), vceqq_u32(t291[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t291[1], t293[1]), vceqq_u32(t291[3], vdupq_n_u32(0)))];
                    let t295: [uint32x4_t; 2] = [vmvnq_u32(t294[0]), vmvnq_u32(t294[1])];
                    let mut t296: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                    let t297: [uint32x4_t; 2] = [vandq_u32(t286[0], t295[0]), vandq_u32(t286[1], t295[1])];
                    let t298: [uint32x4_t; 2] = [vbicq_u32(t286[0], t295[0]), vbicq_u32(t286[1], t295[1])];
                    {
                        let t299: [uint32x4_t; 2] = [x104[0], x104[1]];
                        let t300: [uint32x4_t; 2] = [vorrq_u32(t271[0], t299[0]), vorrq_u32(t271[1], t299[1])];
                        t296 = [vbslq_u32(t297[0], t300[0], t296[0]), vbslq_u32(t297[1], t300[1], t296[1])];
                    }
                    {
                        let t301: [uint32x4_t; 2] = [x104[0], x104[1]];
                        t296 = [vbslq_u32(t298[0], t301[0], t296[0]), vbslq_u32(t298[1], t301[1], t296[1])];
                    }
                    x96 = [vbslq_u32(t286[0], t296[0], x96[0]), vbslq_u32(t286[1], t296[1], x96[1])];
                }
                x72 = [vbslq_u32(t266[0], x96[0], x72[0]), vbslq_u32(t266[1], x96[1], x72[1])];
                let t302: [uint32x4_t; 2] = [vandq_u32(t266[0], live[0]), vandq_u32(t266[1], live[1])];
                let t303: [uint32x4_t; 2] = [x80[0], x80[1]];
                let t304 = [vdupq_n_u32(1u32); 2];
                let t305: [uint32x4_t; 2] = [vaddq_u32(t303[0], t304[0]), vaddq_u32(t303[1], t304[1])];
                let t306 = x80;
                let t307: [uint32x4_t; 4] = [t305[0], t305[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                x80 = t307;
            }
            let t308 = [vdupq_n_u32(2147483648u32); 2];
            let t309: [uint32x4_t; 2] = [vandq_u32(x72[0], t308[0]), vandq_u32(x72[1], t308[1])];
            let t310 = [vdupq_n_u32(0u32); 2];
            let t311: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t309[0], t310[0])), vmvnq_u32(vceqq_u32(t309[1], t310[1]))];
            let t312 = [vdupq_n_u32(4294967294u32); 2];
            let t313: [uint32x4_t; 2] = [vcltq_u32(x72[0], t312[0]), vcltq_u32(x72[1], t312[1])];
            let t314: [uint32x4_t; 2] = [vandq_u32(t311[0], t313[0]), vandq_u32(t311[1], t313[1])];
            let t315: [uint32x4_t; 2] = [vandq_u32(live[0], t314[0]), vandq_u32(live[1], t314[1])];
            if any(&t315) {
                let t316 = [vdupq_n_u32(2147483647u32); 2];
                let t317: [uint32x4_t; 2] = [vandq_u32(x72[0], t316[0]), vandq_u32(x72[1], t316[1])];
                let t318 = b7_r0r1l_p(l, lid0, t315, arg0, arg1, t317);
                x72 = [vbslq_u32(t315[0], t318[0], x72[0]), vbslq_u32(t315[1], t318[1], x72[1])];
            }
            b6_ag4r2l_p(l, lid0, live, arg2, x72);
        }
    }
}

#[inline(always)]
unsafe fn b3_r0r1r2r3ssllg_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, arg2: *const u8, arg3: *const u8, x32: u32, x40: bool, x48: [uint32x4_t; 2], x56: [uint32x4_t; 2]) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x72: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x80: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x88: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x96: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x104: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    if !(x40) {
        let t319: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t320 = [vdupq_n_u32(x32); 2];
        let t321: [uint32x4_t; 2] = [vcltq_u32(t319[0], t320[0]), vcltq_u32(t319[1], t320[1])];
        let t322: [uint32x4_t; 2] = [vandq_u32(live[0], t321[0]), vandq_u32(live[1], t321[1])];
        if any(&t322) {
            let t323 = b7_r0r1g_p(l, lid0, t322, arg0, arg1);
            b6_ag4r2l_p(l, lid0, t322, arg2, t323);
        }
        return;
    }
    if any(&live) {
        let t324 = [vdupq_n_u32(4294967295u32); 2];
        x72 = t324;
        let t325: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t326 = [vdupq_n_u32(x32); 2];
        let t327: [uint32x4_t; 2] = [vcltq_u32(t325[0], t326[0]), vcltq_u32(t325[1], t326[1])];
        let t328: [uint32x4_t; 2] = [vandq_u32(live[0], t327[0]), vandq_u32(live[1], t327[1])];
        if any(&t328) {
            let t329 = b8_r0r1g_p(l, lid0, t328, arg0, arg1);
            x80 = [vbslq_u32(t328[0], t329[0], x80[0]), vbslq_u32(t328[1], t329[1], x80[1]), vbslq_u32(t328[0], t329[2], x80[2]), vbslq_u32(t328[1], t329[3], x80[3])];
            let t330: u64 = (1u64).wrapping_shl((32u64) as u32);
            let t331 = [vdupq_n_u32((t330) as u32); 2];
            let t332 = [vdupq_n_u32(((t330) >> 32) as u32); 2];
            let t333: [uint32x4_t; 4] = [vandq_u32(x80[0], t331[0]), vandq_u32(x80[1], t331[1]), vandq_u32(x80[2], t332[0]), vandq_u32(x80[3], t332[1])];
            let t334: u64 = 0u64;
            let t335 = [vdupq_n_u32((t334) as u32); 2];
            let t336: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t333[0], t335[0]), vceqq_u32(t333[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t333[1], t335[1]), vceqq_u32(t333[3], vdupq_n_u32(0)))];
            let t337: [uint32x4_t; 2] = [vmvnq_u32(t336[0]), vmvnq_u32(t336[1])];
            let mut t338: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t339: [uint32x4_t; 2] = [vandq_u32(t328[0], t337[0]), vandq_u32(t328[1], t337[1])];
            let t340: [uint32x4_t; 2] = [vbicq_u32(t328[0], t337[0]), vbicq_u32(t328[1], t337[1])];
            {
                let t341: [uint32x4_t; 2] = [x80[0], x80[1]];
                let t342 = [vdupq_n_u32(2147483648u32); 2];
                let t343: [uint32x4_t; 2] = [vorrq_u32(t342[0], t341[0]), vorrq_u32(t342[1], t341[1])];
                t338 = [vbslq_u32(t339[0], t343[0], t338[0]), vbslq_u32(t339[1], t343[1], t338[1])];
            }
            {
                let t344: [uint32x4_t; 2] = [x80[0], x80[1]];
                t338 = [vbslq_u32(t340[0], t344[0], t338[0]), vbslq_u32(t340[1], t344[1], t338[1])];
            }
            x72 = [vbslq_u32(t328[0], t338[0], x72[0]), vbslq_u32(t328[1], t338[1], x72[1])];
            b6_ag4r2l_p(l, lid0, t328, arg2, x72);
        }
        b9_r3gl_p(l, lid0, live, arg3, x56);
        let t345 = [vdupq_n_u32(x32); 2];
        let t346: [uint32x4_t; 2] = [vcgeq_u32(t325[0], t345[0]), vcgeq_u32(t325[1], t345[1])];
        let t347: [uint32x4_t; 2] = [vandq_u32(live[0], t346[0]), vandq_u32(live[1], t346[1])];
        {
            live = [vbicq_u32(live[0], t347[0]), vbicq_u32(live[1], t347[1])];
        }
        if any(&live) {
            let t348: u64 = ((0u32) as u64);
            let t349 = [vdupq_n_u32((t348) as u32); 2];
            let t350: [uint32x4_t; 4] = [t349[0], t349[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            x80 = t350;
            let mut t362 = live;
            loop {
                // Lanes part: each its own rounds.
                let t364: [uint32x4_t; 2] = [x80[0], x80[1]];
                let t365 = [vdupq_n_u32(7u32); 2];
                let t366: [uint32x4_t; 2] = [vcltq_u32(t364[0], t365[0]), vcltq_u32(t364[1], t365[1])];
                let t367 = [vdupq_n_u32(2147483648u32); 2];
                let t368: [uint32x4_t; 2] = [vandq_u32(x72[0], t367[0]), vandq_u32(x72[1], t367[1])];
                let t369 = [vdupq_n_u32(0u32); 2];
                let t370: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t368[0], t369[0])), vmvnq_u32(vceqq_u32(t368[1], t369[1]))];
                let t371 = [vdupq_n_u32(4294967294u32); 2];
                let t372: [uint32x4_t; 2] = [vcltq_u32(x72[0], t371[0]), vcltq_u32(x72[1], t371[1])];
                let t373: [uint32x4_t; 2] = [vandq_u32(t370[0], t372[0]), vandq_u32(t370[1], t372[1])];
                let t374: [uint32x4_t; 2] = [vandq_u32(t366[0], t373[0]), vandq_u32(t366[1], t373[1])];
                t362 = [vandq_u32(t362[0], t374[0]), vandq_u32(t362[1], t374[1])];
                if !(any(&t362)) { break; }
                let t376 = [vdupq_n_u32(2147483647u32); 2];
                let t377: [uint32x4_t; 2] = [vandq_u32(x72[0], t376[0]), vandq_u32(x72[1], t376[1])];
                x88 = t377;
                x96 = t371;
                let t378 = b10_r3ll_p(l, lid0, t362, arg3, x88, x48);
                let t379: [uint32x4_t; 2] = [vandq_u32(t362[0], t378[0]), vandq_u32(t362[1], t378[1])];
                if any(&t379) {
                    let t380 = b11_al4r2_p(l, lid0, t379, arg2, x88);
                    x96 = [vbslq_u32(t379[0], t380[0], x96[0]), vbslq_u32(t379[1], t380[1], x96[1])];
                }
                let t381: [uint32x4_t; 2] = [vceqq_u32(x96[0], t371[0]), vceqq_u32(x96[1], t371[1])];
                let t382: [uint32x4_t; 2] = [vandq_u32(t362[0], t381[0]), vandq_u32(t362[1], t381[1])];
                if any(&t382) {
                    let t383 = b8_r0r1l_p(l, lid0, t382, arg0, arg1, x88);
                    x104 = t383;
                    let t384: u64 = (1u64).wrapping_shl((32u64) as u32);
                    let t385 = [vdupq_n_u32((t384) as u32); 2];
                    let t386 = [vdupq_n_u32(((t384) >> 32) as u32); 2];
                    let t387: [uint32x4_t; 4] = [vandq_u32(x104[0], t385[0]), vandq_u32(x104[1], t385[1]), vandq_u32(x104[2], t386[0]), vandq_u32(x104[3], t386[1])];
                    let t388: u64 = 0u64;
                    let t389 = [vdupq_n_u32((t388) as u32); 2];
                    let t390: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t387[0], t389[0]), vceqq_u32(t387[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t387[1], t389[1]), vceqq_u32(t387[3], vdupq_n_u32(0)))];
                    let t391: [uint32x4_t; 2] = [vmvnq_u32(t390[0]), vmvnq_u32(t390[1])];
                    let mut t392: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                    let t393: [uint32x4_t; 2] = [vandq_u32(t382[0], t391[0]), vandq_u32(t382[1], t391[1])];
                    let t394: [uint32x4_t; 2] = [vbicq_u32(t382[0], t391[0]), vbicq_u32(t382[1], t391[1])];
                    {
                        let t395: [uint32x4_t; 2] = [x104[0], x104[1]];
                        let t396: [uint32x4_t; 2] = [vorrq_u32(t367[0], t395[0]), vorrq_u32(t367[1], t395[1])];
                        t392 = [vbslq_u32(t393[0], t396[0], t392[0]), vbslq_u32(t393[1], t396[1], t392[1])];
                    }
                    {
                        let t397: [uint32x4_t; 2] = [x104[0], x104[1]];
                        t392 = [vbslq_u32(t394[0], t397[0], t392[0]), vbslq_u32(t394[1], t397[1], t392[1])];
                    }
                    x96 = [vbslq_u32(t382[0], t392[0], x96[0]), vbslq_u32(t382[1], t392[1], x96[1])];
                }
                x72 = [vbslq_u32(t362[0], x96[0], x72[0]), vbslq_u32(t362[1], x96[1], x72[1])];
                let t398: [uint32x4_t; 2] = [vandq_u32(t362[0], live[0]), vandq_u32(t362[1], live[1])];
                let t399: [uint32x4_t; 2] = [x80[0], x80[1]];
                let t400 = [vdupq_n_u32(1u32); 2];
                let t401: [uint32x4_t; 2] = [vaddq_u32(t399[0], t400[0]), vaddq_u32(t399[1], t400[1])];
                let t402 = x80;
                let t403: [uint32x4_t; 4] = [t401[0], t401[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                x80 = t403;
            }
            let t404 = [vdupq_n_u32(2147483648u32); 2];
            let t405: [uint32x4_t; 2] = [vandq_u32(x72[0], t404[0]), vandq_u32(x72[1], t404[1])];
            let t406 = [vdupq_n_u32(0u32); 2];
            let t407: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t405[0], t406[0])), vmvnq_u32(vceqq_u32(t405[1], t406[1]))];
            let t408 = [vdupq_n_u32(4294967294u32); 2];
            let t409: [uint32x4_t; 2] = [vcltq_u32(x72[0], t408[0]), vcltq_u32(x72[1], t408[1])];
            let t410: [uint32x4_t; 2] = [vandq_u32(t407[0], t409[0]), vandq_u32(t407[1], t409[1])];
            let t411: [uint32x4_t; 2] = [vandq_u32(live[0], t410[0]), vandq_u32(live[1], t410[1])];
            if any(&t411) {
                let t412 = [vdupq_n_u32(2147483647u32); 2];
                let t413: [uint32x4_t; 2] = [vandq_u32(x72[0], t412[0]), vandq_u32(x72[1], t412[1])];
                let t414 = b7_r0r1l_p(l, lid0, t411, arg0, arg1, t413);
                x72 = [vbslq_u32(t411[0], t414[0], x72[0]), vbslq_u32(t411[1], t414[1], x72[1])];
            }
            b6_ag4r2l_p(l, lid0, live, arg2, x72);
        }
    }
}

#[inline(always)]
unsafe fn b3_r0r1r2r3ssllg_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, arg2: *const u8, arg3: *const u8, x32: u32, x40: bool, x48: [uint32x4_t; 2], x56: [uint32x4_t; 2]) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x72: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x80: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x88: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x96: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x104: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    if !(x40) {
        if (((gid0) as u64 + 7) <= u32::MAX as u64 && ((gid0) as u64) >= ((x32) as u64)) {
        } else if ((gid0) as u64 + 7) < ((x32) as u64) {
            let t415 = b7_r0r1g_f(l, lid0, arg0, arg1);
            b6_ag4r2l_f(l, lid0, arg2, t415);
        } else {
            // Lanes part: the chunk's lanes on both sides of the bound.
            let t416: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
            let t417 = [vdupq_n_u32(x32); 2];
            let t418: [uint32x4_t; 2] = [vcltq_u32(t416[0], t417[0]), vcltq_u32(t416[1], t417[1])];
            let t419: [uint32x4_t; 2] = [vandq_u32(live[0], t418[0]), vandq_u32(live[1], t418[1])];
            if any(&t419) {
                let t420 = b7_r0r1g_p(l, lid0, t419, arg0, arg1);
                b6_ag4r2l_p(l, lid0, t419, arg2, t420);
            }
        }
        return;
    }
    if any(&live) {
        let t421 = [vdupq_n_u32(4294967295u32); 2];
        x72 = t421;
        let t422: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t423 = [vdupq_n_u32(x32); 2];
        let t424: [uint32x4_t; 2] = [vcltq_u32(t422[0], t423[0]), vcltq_u32(t422[1], t423[1])];
        let t425: [uint32x4_t; 2] = [vandq_u32(live[0], t424[0]), vandq_u32(live[1], t424[1])];
        if any(&t425) {
            let t426 = b8_r0r1g_p(l, lid0, t425, arg0, arg1);
            x80 = [vbslq_u32(t425[0], t426[0], x80[0]), vbslq_u32(t425[1], t426[1], x80[1]), vbslq_u32(t425[0], t426[2], x80[2]), vbslq_u32(t425[1], t426[3], x80[3])];
            let t427: u64 = (1u64).wrapping_shl((32u64) as u32);
            let t428 = [vdupq_n_u32((t427) as u32); 2];
            let t429 = [vdupq_n_u32(((t427) >> 32) as u32); 2];
            let t430: [uint32x4_t; 4] = [vandq_u32(x80[0], t428[0]), vandq_u32(x80[1], t428[1]), vandq_u32(x80[2], t429[0]), vandq_u32(x80[3], t429[1])];
            let t431: u64 = 0u64;
            let t432 = [vdupq_n_u32((t431) as u32); 2];
            let t433: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t430[0], t432[0]), vceqq_u32(t430[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t430[1], t432[1]), vceqq_u32(t430[3], vdupq_n_u32(0)))];
            let t434: [uint32x4_t; 2] = [vmvnq_u32(t433[0]), vmvnq_u32(t433[1])];
            let mut t435: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t436: [uint32x4_t; 2] = [vandq_u32(t425[0], t434[0]), vandq_u32(t425[1], t434[1])];
            let t437: [uint32x4_t; 2] = [vbicq_u32(t425[0], t434[0]), vbicq_u32(t425[1], t434[1])];
            {
                let t438: [uint32x4_t; 2] = [x80[0], x80[1]];
                let t439 = [vdupq_n_u32(2147483648u32); 2];
                let t440: [uint32x4_t; 2] = [vorrq_u32(t439[0], t438[0]), vorrq_u32(t439[1], t438[1])];
                t435 = [vbslq_u32(t436[0], t440[0], t435[0]), vbslq_u32(t436[1], t440[1], t435[1])];
            }
            {
                let t441: [uint32x4_t; 2] = [x80[0], x80[1]];
                t435 = [vbslq_u32(t437[0], t441[0], t435[0]), vbslq_u32(t437[1], t441[1], t435[1])];
            }
            x72 = [vbslq_u32(t425[0], t435[0], x72[0]), vbslq_u32(t425[1], t435[1], x72[1])];
            b6_ag4r2l_p(l, lid0, t425, arg2, x72);
        }
        b9_r3gl_p(l, lid0, live, arg3, x56);
        let t442 = [vdupq_n_u32(x32); 2];
        let t443: [uint32x4_t; 2] = [vcgeq_u32(t422[0], t442[0]), vcgeq_u32(t422[1], t442[1])];
        let t444: [uint32x4_t; 2] = [vandq_u32(live[0], t443[0]), vandq_u32(live[1], t443[1])];
        {
            live = [vbicq_u32(live[0], t444[0]), vbicq_u32(live[1], t444[1])];
        }
        if any(&live) {
            let t445: u64 = ((0u32) as u64);
            let t446 = [vdupq_n_u32((t445) as u32); 2];
            let t447: [uint32x4_t; 4] = [t446[0], t446[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            x80 = t447;
            let mut t459 = live;
            loop {
                // Lanes part: each its own rounds.
                let t461: [uint32x4_t; 2] = [x80[0], x80[1]];
                let t462 = [vdupq_n_u32(7u32); 2];
                let t463: [uint32x4_t; 2] = [vcltq_u32(t461[0], t462[0]), vcltq_u32(t461[1], t462[1])];
                let t464 = [vdupq_n_u32(2147483648u32); 2];
                let t465: [uint32x4_t; 2] = [vandq_u32(x72[0], t464[0]), vandq_u32(x72[1], t464[1])];
                let t466 = [vdupq_n_u32(0u32); 2];
                let t467: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t465[0], t466[0])), vmvnq_u32(vceqq_u32(t465[1], t466[1]))];
                let t468 = [vdupq_n_u32(4294967294u32); 2];
                let t469: [uint32x4_t; 2] = [vcltq_u32(x72[0], t468[0]), vcltq_u32(x72[1], t468[1])];
                let t470: [uint32x4_t; 2] = [vandq_u32(t467[0], t469[0]), vandq_u32(t467[1], t469[1])];
                let t471: [uint32x4_t; 2] = [vandq_u32(t463[0], t470[0]), vandq_u32(t463[1], t470[1])];
                t459 = [vandq_u32(t459[0], t471[0]), vandq_u32(t459[1], t471[1])];
                if !(any(&t459)) { break; }
                let t473 = [vdupq_n_u32(2147483647u32); 2];
                let t474: [uint32x4_t; 2] = [vandq_u32(x72[0], t473[0]), vandq_u32(x72[1], t473[1])];
                x88 = t474;
                x96 = t468;
                let t475 = b10_r3ll_p(l, lid0, t459, arg3, x88, x48);
                let t476: [uint32x4_t; 2] = [vandq_u32(t459[0], t475[0]), vandq_u32(t459[1], t475[1])];
                if any(&t476) {
                    let t477 = b11_al4r2_p(l, lid0, t476, arg2, x88);
                    x96 = [vbslq_u32(t476[0], t477[0], x96[0]), vbslq_u32(t476[1], t477[1], x96[1])];
                }
                let t478: [uint32x4_t; 2] = [vceqq_u32(x96[0], t468[0]), vceqq_u32(x96[1], t468[1])];
                let t479: [uint32x4_t; 2] = [vandq_u32(t459[0], t478[0]), vandq_u32(t459[1], t478[1])];
                if any(&t479) {
                    let t480 = b8_r0r1l_p(l, lid0, t479, arg0, arg1, x88);
                    x104 = t480;
                    let t481: u64 = (1u64).wrapping_shl((32u64) as u32);
                    let t482 = [vdupq_n_u32((t481) as u32); 2];
                    let t483 = [vdupq_n_u32(((t481) >> 32) as u32); 2];
                    let t484: [uint32x4_t; 4] = [vandq_u32(x104[0], t482[0]), vandq_u32(x104[1], t482[1]), vandq_u32(x104[2], t483[0]), vandq_u32(x104[3], t483[1])];
                    let t485: u64 = 0u64;
                    let t486 = [vdupq_n_u32((t485) as u32); 2];
                    let t487: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t484[0], t486[0]), vceqq_u32(t484[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t484[1], t486[1]), vceqq_u32(t484[3], vdupq_n_u32(0)))];
                    let t488: [uint32x4_t; 2] = [vmvnq_u32(t487[0]), vmvnq_u32(t487[1])];
                    let mut t489: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                    let t490: [uint32x4_t; 2] = [vandq_u32(t479[0], t488[0]), vandq_u32(t479[1], t488[1])];
                    let t491: [uint32x4_t; 2] = [vbicq_u32(t479[0], t488[0]), vbicq_u32(t479[1], t488[1])];
                    {
                        let t492: [uint32x4_t; 2] = [x104[0], x104[1]];
                        let t493: [uint32x4_t; 2] = [vorrq_u32(t464[0], t492[0]), vorrq_u32(t464[1], t492[1])];
                        t489 = [vbslq_u32(t490[0], t493[0], t489[0]), vbslq_u32(t490[1], t493[1], t489[1])];
                    }
                    {
                        let t494: [uint32x4_t; 2] = [x104[0], x104[1]];
                        t489 = [vbslq_u32(t491[0], t494[0], t489[0]), vbslq_u32(t491[1], t494[1], t489[1])];
                    }
                    x96 = [vbslq_u32(t479[0], t489[0], x96[0]), vbslq_u32(t479[1], t489[1], x96[1])];
                }
                x72 = [vbslq_u32(t459[0], x96[0], x72[0]), vbslq_u32(t459[1], x96[1], x72[1])];
                let t495: [uint32x4_t; 2] = [vandq_u32(t459[0], live[0]), vandq_u32(t459[1], live[1])];
                let t496: [uint32x4_t; 2] = [x80[0], x80[1]];
                let t497 = [vdupq_n_u32(1u32); 2];
                let t498: [uint32x4_t; 2] = [vaddq_u32(t496[0], t497[0]), vaddq_u32(t496[1], t497[1])];
                let t499 = x80;
                let t500: [uint32x4_t; 4] = [t498[0], t498[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                x80 = t500;
            }
            let t501 = [vdupq_n_u32(2147483648u32); 2];
            let t502: [uint32x4_t; 2] = [vandq_u32(x72[0], t501[0]), vandq_u32(x72[1], t501[1])];
            let t503 = [vdupq_n_u32(0u32); 2];
            let t504: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t502[0], t503[0])), vmvnq_u32(vceqq_u32(t502[1], t503[1]))];
            let t505 = [vdupq_n_u32(4294967294u32); 2];
            let t506: [uint32x4_t; 2] = [vcltq_u32(x72[0], t505[0]), vcltq_u32(x72[1], t505[1])];
            let t507: [uint32x4_t; 2] = [vandq_u32(t504[0], t506[0]), vandq_u32(t504[1], t506[1])];
            let t508: [uint32x4_t; 2] = [vandq_u32(live[0], t507[0]), vandq_u32(live[1], t507[1])];
            if any(&t508) {
                let t509 = [vdupq_n_u32(2147483647u32); 2];
                let t510: [uint32x4_t; 2] = [vandq_u32(x72[0], t509[0]), vandq_u32(x72[1], t509[1])];
                let t511 = b7_r0r1l_p(l, lid0, t508, arg0, arg1, t510);
                x72 = [vbslq_u32(t508[0], t511[0], x72[0]), vbslq_u32(t508[1], t511[1], x72[1])];
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
    let t512 = (arg2 as *mut u32).add((gid0) as usize);
    if all(&live) {
        vst1q_u32(t512.add(0), x8[0]);
        vst1q_u32(t512.add(4), x8[1]);
    } else {
        let mut t513w = 0u32;
        let t513 = core::hint::black_box(&raw mut t513w);
        core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t512.add(0), t513).write(vgetq_lane_u32::<0>(x8[0]));
        core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t512.add(1), t513).write(vgetq_lane_u32::<1>(x8[0]));
        core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t512.add(2), t513).write(vgetq_lane_u32::<2>(x8[0]));
        core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t512.add(3), t513).write(vgetq_lane_u32::<3>(x8[0]));
        core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t512.add(4), t513).write(vgetq_lane_u32::<0>(x8[1]));
        core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t512.add(5), t513).write(vgetq_lane_u32::<1>(x8[1]));
        core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t512.add(6), t513).write(vgetq_lane_u32::<2>(x8[1]));
        core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t512.add(7), t513).write(vgetq_lane_u32::<3>(x8[1]));
    }
}

#[inline(always)]
unsafe fn b6_ag4r2l_f(l: &Launch, lid0: u32, arg2: *const u8, x8: [uint32x4_t; 2]) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let t514 = (arg2 as *mut u32).add((gid0) as usize);
    vst1q_u32(t514.add(0), x8[0]);
    vst1q_u32(t514.add(4), x8[1]);
}

#[inline(always)]
unsafe fn b7_r0r1g_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t515 = b8_r0r1g_p(l, lid0, live, arg0, arg1);
    x24 = t515;
    let mut t524 = live;
    loop {
        // Lanes part: each its own rounds.
        let t526: u64 = (1u64).wrapping_shl((32u64) as u32);
        let t527 = [vdupq_n_u32((t526) as u32); 2];
        let t528 = [vdupq_n_u32(((t526) >> 32) as u32); 2];
        let t529: [uint32x4_t; 4] = [vandq_u32(x24[0], t527[0]), vandq_u32(x24[1], t527[1]), vandq_u32(x24[2], t528[0]), vandq_u32(x24[3], t528[1])];
        let t530: u64 = 0u64;
        let t531 = [vdupq_n_u32((t530) as u32); 2];
        let t532: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t529[0], t531[0]), vceqq_u32(t529[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t529[1], t531[1]), vceqq_u32(t529[3], vdupq_n_u32(0)))];
        let t533: [uint32x4_t; 2] = [vmvnq_u32(t532[0]), vmvnq_u32(t532[1])];
        t524 = [vandq_u32(t524[0], t533[0]), vandq_u32(t524[1], t533[1])];
        if !(any(&t524)) { break; }
        let t535: [uint32x4_t; 2] = [x24[0], x24[1]];
        let t536 = b8_r0r1l_p(l, lid0, t524, arg0, arg1, t535);
        x24 = [vbslq_u32(t524[0], t536[0], x24[0]), vbslq_u32(t524[1], t536[1], x24[1]), vbslq_u32(t524[0], t536[2], x24[2]), vbslq_u32(t524[1], t536[3], x24[3])];
    }
    let t537: [uint32x4_t; 2] = [x24[0], x24[1]];
    res = t537;
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1g_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t538 = b8_r0r1g_f(l, lid0, arg0, arg1);
    x24 = t538;
    let mut t555 = live;
    loop {
        // Lanes part: each its own rounds.
        let t557: u64 = (1u64).wrapping_shl((32u64) as u32);
        let t558 = [vdupq_n_u32((t557) as u32); 2];
        let t559 = [vdupq_n_u32(((t557) >> 32) as u32); 2];
        let t560: [uint32x4_t; 4] = [vandq_u32(x24[0], t558[0]), vandq_u32(x24[1], t558[1]), vandq_u32(x24[2], t559[0]), vandq_u32(x24[3], t559[1])];
        let t561: u64 = 0u64;
        let t562 = [vdupq_n_u32((t561) as u32); 2];
        let t563: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t560[0], t562[0]), vceqq_u32(t560[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t560[1], t562[1]), vceqq_u32(t560[3], vdupq_n_u32(0)))];
        let t564: [uint32x4_t; 2] = [vmvnq_u32(t563[0]), vmvnq_u32(t563[1])];
        t555 = [vandq_u32(t555[0], t564[0]), vandq_u32(t555[1], t564[1])];
        if !(any(&t555)) { break; }
        let t566: [uint32x4_t; 2] = [x24[0], x24[1]];
        let t567 = b8_r0r1l_p(l, lid0, t555, arg0, arg1, t566);
        x24 = [vbslq_u32(t555[0], t567[0], x24[0]), vbslq_u32(t555[1], t567[1], x24[1]), vbslq_u32(t555[0], t567[2], x24[2]), vbslq_u32(t555[1], t567[3], x24[3])];
    }
    let t568: [uint32x4_t; 2] = [x24[0], x24[1]];
    res = t568;
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t569 = b8_r0r1l_p(l, lid0, live, arg0, arg1, x16);
    x24 = t569;
    let mut t578 = live;
    loop {
        // Lanes part: each its own rounds.
        let t580: u64 = (1u64).wrapping_shl((32u64) as u32);
        let t581 = [vdupq_n_u32((t580) as u32); 2];
        let t582 = [vdupq_n_u32(((t580) >> 32) as u32); 2];
        let t583: [uint32x4_t; 4] = [vandq_u32(x24[0], t581[0]), vandq_u32(x24[1], t581[1]), vandq_u32(x24[2], t582[0]), vandq_u32(x24[3], t582[1])];
        let t584: u64 = 0u64;
        let t585 = [vdupq_n_u32((t584) as u32); 2];
        let t586: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t583[0], t585[0]), vceqq_u32(t583[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t583[1], t585[1]), vceqq_u32(t583[3], vdupq_n_u32(0)))];
        let t587: [uint32x4_t; 2] = [vmvnq_u32(t586[0]), vmvnq_u32(t586[1])];
        t578 = [vandq_u32(t578[0], t587[0]), vandq_u32(t578[1], t587[1])];
        if !(any(&t578)) { break; }
        let t589: [uint32x4_t; 2] = [x24[0], x24[1]];
        let t590 = b8_r0r1l_p(l, lid0, t578, arg0, arg1, t589);
        x24 = [vbslq_u32(t578[0], t590[0], x24[0]), vbslq_u32(t578[1], t590[1], x24[1]), vbslq_u32(t578[0], t590[2], x24[2]), vbslq_u32(t578[1], t590[3], x24[3])];
    }
    let t591: [uint32x4_t; 2] = [x24[0], x24[1]];
    res = t591;
    return res;
}

#[inline(always)]
unsafe fn b8_r0r1g_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: u32 = 0;
    let mut x32: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t592: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
    let t593 = [vdupq_n_u32(0u32); 2];
    let t594: [uint32x4_t; 2] = [vceqq_u32(t592[0], t593[0]), vceqq_u32(t592[1], t593[1])];
    let t595: [uint32x4_t; 2] = [vandq_u32(live[0], t594[0]), vandq_u32(live[1], t594[1])];
    {
        let t596: u64 = 4294967295u64;
        let t597 = [vdupq_n_u32((t596) as u32); 2];
        let t598: [uint32x4_t; 4] = [t597[0], t597[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = [vbslq_u32(t595[0], t598[0], res[0]), vbslq_u32(t595[1], t598[1], res[1]), vbslq_u32(t595[0], t598[2], res[2]), vbslq_u32(t595[1], t598[3], res[3])];
        live = [vbicq_u32(live[0], t595[0]), vbicq_u32(live[1], t595[1])];
    }
    if any(&live) {
        x24 = gid0.wrapping_sub(1u32);
        let t599 = if (x24 & 255u32) <= 248u32 { b12_r0r1u8_p(l, lid0, live, arg0, arg1, x24) } else { b12_r0r1i_p(l, lid0, live, arg0, arg1, x24) };
        x32 = t599;
        let t600: u64 = 1u64;
        let t601 = [vdupq_n_u32((t600) as u32); 2];
        let t602: [uint32x4_t; 2] = [vandq_u32(x32[0], t601[0]), vandq_u32(x32[1], t601[1])];
        let t603: u64 = 0u64;
        let t604 = [vdupq_n_u32((t603) as u32); 2];
        let t605: [uint32x4_t; 2] = [vceqq_u32(t602[0], t604[0]), vceqq_u32(t602[1], t604[1])];
        let t606: [uint32x4_t; 2] = [vandq_u32(live[0], t605[0]), vandq_u32(live[1], t605[1])];
        {
            let t607: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[1].as_ptr()))];
            let t608: [uint32x4_t; 4] = [t607[0], t607[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t606[0], t608[0], res[0]), vbslq_u32(t606[1], t608[1], res[1]), vbslq_u32(t606[0], t608[2], res[2]), vbslq_u32(t606[1], t608[3], res[3])];
            live = [vbicq_u32(live[0], t606[0]), vbicq_u32(live[1], t606[1])];
        }
        if any(&live) {
            let t609: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x32[0]), vshlq_n_u32::<30>(x32[2])), vorrq_u32(vshrq_n_u32::<2>(x32[1]), vshlq_n_u32::<30>(x32[3])), vshrq_n_u32::<2>(x32[2]), vshrq_n_u32::<2>(x32[3])];
            let t610: [uint32x4_t; 2] = [t609[0], t609[1]];
            x40 = t610;
            let t611 = [vdupq_n_u32(1u32); 2];
            let t612: [uint32x4_t; 2] = [vsubq_u32(x40[0], t611[0]), vsubq_u32(x40[1], t611[1])];
            let t613: [uint32x4_t; 2] = [vcgeq_u32(t612[0], t592[0]), vcgeq_u32(t612[1], t592[1])];
            let t614: [uint32x4_t; 2] = [vandq_u32(live[0], t613[0]), vandq_u32(live[1], t613[1])];
            {
                let t615: u64 = 4294967295u64;
                let t616 = [vdupq_n_u32((t615) as u32); 2];
                let t617: [uint32x4_t; 4] = [t616[0], t616[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t614[0], t617[0], res[0]), vbslq_u32(t614[1], t617[1], res[1]), vbslq_u32(t614[0], t617[2], res[2]), vbslq_u32(t614[1], t617[3], res[3])];
                live = [vbicq_u32(live[0], t614[0]), vbicq_u32(live[1], t614[1])];
            }
            if any(&live) {
                let t618: [uint32x4_t; 2] = [vsubq_u32(t592[0], x40[0]), vsubq_u32(t592[1], x40[1])];
                x48 = t618;
                let t619: u64 = 2u64;
                let t620 = [vdupq_n_u32((t619) as u32); 2];
                let t621: [uint32x4_t; 2] = [vandq_u32(x32[0], t620[0]), vandq_u32(x32[1], t620[1])];
                let t622: u64 = 0u64;
                let t623 = [vdupq_n_u32((t622) as u32); 2];
                let t624: [uint32x4_t; 2] = [vceqq_u32(t621[0], t623[0]), vceqq_u32(t621[1], t623[1])];
                let t625: [uint32x4_t; 2] = [vmvnq_u32(t624[0]), vmvnq_u32(t624[1])];
                let t626: [uint32x4_t; 2] = [vandq_u32(live[0], t625[0]), vandq_u32(live[1], t625[1])];
                {
                    let t627: [uint32x4_t; 4] = [x48[0], x48[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                    res = [vbslq_u32(t626[0], t627[0], res[0]), vbslq_u32(t626[1], t627[1], res[1]), vbslq_u32(t626[0], t627[2], res[2]), vbslq_u32(t626[1], t627[3], res[3])];
                    live = [vbicq_u32(live[0], t626[0]), vbicq_u32(live[1], t626[1])];
                }
                if any(&live) {
                    let t628: [uint32x4_t; 2] = [vceqq_u32(x48[0], t593[0]), vceqq_u32(x48[1], t593[1])];
                    let t629: [uint32x4_t; 2] = [vandq_u32(live[0], t628[0]), vandq_u32(live[1], t628[1])];
                    {
                        let t630: u64 = 4294967295u64;
                        let t631 = [vdupq_n_u32((t630) as u32); 2];
                        let t632: [uint32x4_t; 4] = [t631[0], t631[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                        res = [vbslq_u32(t629[0], t632[0], res[0]), vbslq_u32(t629[1], t632[1], res[1]), vbslq_u32(t629[0], t632[2], res[2]), vbslq_u32(t629[1], t632[3], res[3])];
                        live = [vbicq_u32(live[0], t629[0]), vbicq_u32(live[1], t629[1])];
                    }
                    if any(&live) {
                        let t633: u64 = (1u64).wrapping_shl((32u64) as u32);
                        let t634 = [vdupq_n_u32((t633) as u32); 2];
                        let t635 = [vdupq_n_u32(((t633) >> 32) as u32); 2];
                        let t636: [uint32x4_t; 4] = [vorrq_u32(t634[0], x48[0]), vorrq_u32(t634[1], x48[1]), t635[0], t635[1]];
                        res = [vbslq_u32(live[0], t636[0], res[0]), vbslq_u32(live[1], t636[1], res[1]), vbslq_u32(live[0], t636[2], res[2]), vbslq_u32(live[1], t636[3], res[3])];
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
    let mut x24: u32 = 0;
    let mut x32: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if (((gid0) as u64 + 7) <= u32::MAX as u64 && (((0u32) as u64) < ((gid0) as u64) || ((0u32) as u64) > ((gid0) as u64 + 7))) {
        x24 = gid0.wrapping_sub(1u32);
        let t637 = if (x24 & 255u32) <= 248u32 { b12_r0r1u8_f(l, lid0, arg0, arg1, x24) } else { b12_r0r1i_f(l, lid0, arg0, arg1, x24) };
        x32 = t637;
        let t638: u64 = 1u64;
        let t639 = [vdupq_n_u32((t638) as u32); 2];
        let t640: [uint32x4_t; 2] = [vandq_u32(x32[0], t639[0]), vandq_u32(x32[1], t639[1])];
        let t641: u64 = 0u64;
        let t642 = [vdupq_n_u32((t641) as u32); 2];
        let t643: [uint32x4_t; 2] = [vceqq_u32(t640[0], t642[0]), vceqq_u32(t640[1], t642[1])];
        {
            let t644: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[1].as_ptr()))];
            let t645: [uint32x4_t; 4] = [t644[0], t644[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t643[0], t645[0], res[0]), vbslq_u32(t643[1], t645[1], res[1]), vbslq_u32(t643[0], t645[2], res[2]), vbslq_u32(t643[1], t645[3], res[3])];
            live = [vbicq_u32(live[0], t643[0]), vbicq_u32(live[1], t643[1])];
        }
        if any(&live) {
            let t646: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x32[0]), vshlq_n_u32::<30>(x32[2])), vorrq_u32(vshrq_n_u32::<2>(x32[1]), vshlq_n_u32::<30>(x32[3])), vshrq_n_u32::<2>(x32[2]), vshrq_n_u32::<2>(x32[3])];
            let t647: [uint32x4_t; 2] = [t646[0], t646[1]];
            x40 = t647;
            let t648 = [vdupq_n_u32(1u32); 2];
            let t649: [uint32x4_t; 2] = [vsubq_u32(x40[0], t648[0]), vsubq_u32(x40[1], t648[1])];
            let t650: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
            let t651: [uint32x4_t; 2] = [vcgeq_u32(t649[0], t650[0]), vcgeq_u32(t649[1], t650[1])];
            let t652: [uint32x4_t; 2] = [vandq_u32(live[0], t651[0]), vandq_u32(live[1], t651[1])];
            {
                let t653: u64 = 4294967295u64;
                let t654 = [vdupq_n_u32((t653) as u32); 2];
                let t655: [uint32x4_t; 4] = [t654[0], t654[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t652[0], t655[0], res[0]), vbslq_u32(t652[1], t655[1], res[1]), vbslq_u32(t652[0], t655[2], res[2]), vbslq_u32(t652[1], t655[3], res[3])];
                live = [vbicq_u32(live[0], t652[0]), vbicq_u32(live[1], t652[1])];
            }
            if any(&live) {
                let t656: [uint32x4_t; 2] = [vsubq_u32(t650[0], x40[0]), vsubq_u32(t650[1], x40[1])];
                x48 = t656;
                let t657: u64 = 2u64;
                let t658 = [vdupq_n_u32((t657) as u32); 2];
                let t659: [uint32x4_t; 2] = [vandq_u32(x32[0], t658[0]), vandq_u32(x32[1], t658[1])];
                let t660: u64 = 0u64;
                let t661 = [vdupq_n_u32((t660) as u32); 2];
                let t662: [uint32x4_t; 2] = [vceqq_u32(t659[0], t661[0]), vceqq_u32(t659[1], t661[1])];
                let t663: [uint32x4_t; 2] = [vmvnq_u32(t662[0]), vmvnq_u32(t662[1])];
                let t664: [uint32x4_t; 2] = [vandq_u32(live[0], t663[0]), vandq_u32(live[1], t663[1])];
                {
                    let t665: [uint32x4_t; 4] = [x48[0], x48[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                    res = [vbslq_u32(t664[0], t665[0], res[0]), vbslq_u32(t664[1], t665[1], res[1]), vbslq_u32(t664[0], t665[2], res[2]), vbslq_u32(t664[1], t665[3], res[3])];
                    live = [vbicq_u32(live[0], t664[0]), vbicq_u32(live[1], t664[1])];
                }
                if any(&live) {
                    let t666 = [vdupq_n_u32(0u32); 2];
                    let t667: [uint32x4_t; 2] = [vceqq_u32(x48[0], t666[0]), vceqq_u32(x48[1], t666[1])];
                    let t668: [uint32x4_t; 2] = [vandq_u32(live[0], t667[0]), vandq_u32(live[1], t667[1])];
                    {
                        let t669: u64 = 4294967295u64;
                        let t670 = [vdupq_n_u32((t669) as u32); 2];
                        let t671: [uint32x4_t; 4] = [t670[0], t670[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                        res = [vbslq_u32(t668[0], t671[0], res[0]), vbslq_u32(t668[1], t671[1], res[1]), vbslq_u32(t668[0], t671[2], res[2]), vbslq_u32(t668[1], t671[3], res[3])];
                        live = [vbicq_u32(live[0], t668[0]), vbicq_u32(live[1], t668[1])];
                    }
                    if any(&live) {
                        let t672: u64 = (1u64).wrapping_shl((32u64) as u32);
                        let t673 = [vdupq_n_u32((t672) as u32); 2];
                        let t674 = [vdupq_n_u32(((t672) >> 32) as u32); 2];
                        let t675: [uint32x4_t; 4] = [vorrq_u32(t673[0], x48[0]), vorrq_u32(t673[1], x48[1]), t674[0], t674[1]];
                        res = [vbslq_u32(live[0], t675[0], res[0]), vbslq_u32(live[1], t675[1], res[1]), vbslq_u32(live[0], t675[2], res[2]), vbslq_u32(live[1], t675[3], res[3])];
                        live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
                    }
                }
            }
        }
    } else if false {
        let t676: u64 = 4294967295u64;
        let t677 = [vdupq_n_u32((t676) as u32); 2];
        let t678: [uint32x4_t; 4] = [t677[0], t677[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t678;
        return res;
    } else {
        // Lanes part: the chunk's lanes on both sides of the bound.
        let t679: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t680 = [vdupq_n_u32(0u32); 2];
        let t681: [uint32x4_t; 2] = [vceqq_u32(t679[0], t680[0]), vceqq_u32(t679[1], t680[1])];
        let t682: [uint32x4_t; 2] = [vandq_u32(live[0], t681[0]), vandq_u32(live[1], t681[1])];
        {
            let t683: u64 = 4294967295u64;
            let t684 = [vdupq_n_u32((t683) as u32); 2];
            let t685: [uint32x4_t; 4] = [t684[0], t684[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t682[0], t685[0], res[0]), vbslq_u32(t682[1], t685[1], res[1]), vbslq_u32(t682[0], t685[2], res[2]), vbslq_u32(t682[1], t685[3], res[3])];
            live = [vbicq_u32(live[0], t682[0]), vbicq_u32(live[1], t682[1])];
        }
        if any(&live) {
            x24 = gid0.wrapping_sub(1u32);
            let t686 = if (x24 & 255u32) <= 248u32 { b12_r0r1u8_p(l, lid0, live, arg0, arg1, x24) } else { b12_r0r1i_p(l, lid0, live, arg0, arg1, x24) };
            x32 = t686;
            let t687: u64 = 1u64;
            let t688 = [vdupq_n_u32((t687) as u32); 2];
            let t689: [uint32x4_t; 2] = [vandq_u32(x32[0], t688[0]), vandq_u32(x32[1], t688[1])];
            let t690: u64 = 0u64;
            let t691 = [vdupq_n_u32((t690) as u32); 2];
            let t692: [uint32x4_t; 2] = [vceqq_u32(t689[0], t691[0]), vceqq_u32(t689[1], t691[1])];
            let t693: [uint32x4_t; 2] = [vandq_u32(live[0], t692[0]), vandq_u32(live[1], t692[1])];
            {
                let t694: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[1].as_ptr()))];
                let t695: [uint32x4_t; 4] = [t694[0], t694[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t693[0], t695[0], res[0]), vbslq_u32(t693[1], t695[1], res[1]), vbslq_u32(t693[0], t695[2], res[2]), vbslq_u32(t693[1], t695[3], res[3])];
                live = [vbicq_u32(live[0], t693[0]), vbicq_u32(live[1], t693[1])];
            }
            if any(&live) {
                let t696: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x32[0]), vshlq_n_u32::<30>(x32[2])), vorrq_u32(vshrq_n_u32::<2>(x32[1]), vshlq_n_u32::<30>(x32[3])), vshrq_n_u32::<2>(x32[2]), vshrq_n_u32::<2>(x32[3])];
                let t697: [uint32x4_t; 2] = [t696[0], t696[1]];
                x40 = t697;
                let t698 = [vdupq_n_u32(1u32); 2];
                let t699: [uint32x4_t; 2] = [vsubq_u32(x40[0], t698[0]), vsubq_u32(x40[1], t698[1])];
                let t700: [uint32x4_t; 2] = [vcgeq_u32(t699[0], t679[0]), vcgeq_u32(t699[1], t679[1])];
                let t701: [uint32x4_t; 2] = [vandq_u32(live[0], t700[0]), vandq_u32(live[1], t700[1])];
                {
                    let t702: u64 = 4294967295u64;
                    let t703 = [vdupq_n_u32((t702) as u32); 2];
                    let t704: [uint32x4_t; 4] = [t703[0], t703[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                    res = [vbslq_u32(t701[0], t704[0], res[0]), vbslq_u32(t701[1], t704[1], res[1]), vbslq_u32(t701[0], t704[2], res[2]), vbslq_u32(t701[1], t704[3], res[3])];
                    live = [vbicq_u32(live[0], t701[0]), vbicq_u32(live[1], t701[1])];
                }
                if any(&live) {
                    let t705: [uint32x4_t; 2] = [vsubq_u32(t679[0], x40[0]), vsubq_u32(t679[1], x40[1])];
                    x48 = t705;
                    let t706: u64 = 2u64;
                    let t707 = [vdupq_n_u32((t706) as u32); 2];
                    let t708: [uint32x4_t; 2] = [vandq_u32(x32[0], t707[0]), vandq_u32(x32[1], t707[1])];
                    let t709: u64 = 0u64;
                    let t710 = [vdupq_n_u32((t709) as u32); 2];
                    let t711: [uint32x4_t; 2] = [vceqq_u32(t708[0], t710[0]), vceqq_u32(t708[1], t710[1])];
                    let t712: [uint32x4_t; 2] = [vmvnq_u32(t711[0]), vmvnq_u32(t711[1])];
                    let t713: [uint32x4_t; 2] = [vandq_u32(live[0], t712[0]), vandq_u32(live[1], t712[1])];
                    {
                        let t714: [uint32x4_t; 4] = [x48[0], x48[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                        res = [vbslq_u32(t713[0], t714[0], res[0]), vbslq_u32(t713[1], t714[1], res[1]), vbslq_u32(t713[0], t714[2], res[2]), vbslq_u32(t713[1], t714[3], res[3])];
                        live = [vbicq_u32(live[0], t713[0]), vbicq_u32(live[1], t713[1])];
                    }
                    if any(&live) {
                        let t715: [uint32x4_t; 2] = [vceqq_u32(x48[0], t680[0]), vceqq_u32(x48[1], t680[1])];
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
                            let t723: [uint32x4_t; 4] = [vorrq_u32(t721[0], x48[0]), vorrq_u32(t721[1], x48[1]), t722[0], t722[1]];
                            res = [vbslq_u32(live[0], t723[0], res[0]), vbslq_u32(live[1], t723[1], res[1]), vbslq_u32(live[0], t723[2], res[2]), vbslq_u32(live[1], t723[3], res[3])];
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
unsafe fn b8_r0r1l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x32: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t724 = [vdupq_n_u32(0u32); 2];
    let t725: [uint32x4_t; 2] = [vceqq_u32(x16[0], t724[0]), vceqq_u32(x16[1], t724[1])];
    let t726: [uint32x4_t; 2] = [vandq_u32(live[0], t725[0]), vandq_u32(live[1], t725[1])];
    {
        let t727: u64 = 4294967295u64;
        let t728 = [vdupq_n_u32((t727) as u32); 2];
        let t729: [uint32x4_t; 4] = [t728[0], t728[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = [vbslq_u32(t726[0], t729[0], res[0]), vbslq_u32(t726[1], t729[1], res[1]), vbslq_u32(t726[0], t729[2], res[2]), vbslq_u32(t726[1], t729[3], res[3])];
        live = [vbicq_u32(live[0], t726[0]), vbicq_u32(live[1], t726[1])];
    }
    if any(&live) {
        let t730 = [vdupq_n_u32(1u32); 2];
        let t731: [uint32x4_t; 2] = [vsubq_u32(x16[0], t730[0]), vsubq_u32(x16[1], t730[1])];
        x24 = t731;
        let t732 = b12_r0r1l_p(l, lid0, live, arg0, arg1, x24);
        x32 = t732;
        let t733: u64 = 1u64;
        let t734 = [vdupq_n_u32((t733) as u32); 2];
        let t735: [uint32x4_t; 2] = [vandq_u32(x32[0], t734[0]), vandq_u32(x32[1], t734[1])];
        let t736: u64 = 0u64;
        let t737 = [vdupq_n_u32((t736) as u32); 2];
        let t738: [uint32x4_t; 2] = [vceqq_u32(t735[0], t737[0]), vceqq_u32(t735[1], t737[1])];
        let t739: [uint32x4_t; 2] = [vandq_u32(live[0], t738[0]), vandq_u32(live[1], t738[1])];
        {
            let t740: [uint32x4_t; 4] = [x24[0], x24[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t739[0], t740[0], res[0]), vbslq_u32(t739[1], t740[1], res[1]), vbslq_u32(t739[0], t740[2], res[2]), vbslq_u32(t739[1], t740[3], res[3])];
            live = [vbicq_u32(live[0], t739[0]), vbicq_u32(live[1], t739[1])];
        }
        if any(&live) {
            let t741: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x32[0]), vshlq_n_u32::<30>(x32[2])), vorrq_u32(vshrq_n_u32::<2>(x32[1]), vshlq_n_u32::<30>(x32[3])), vshrq_n_u32::<2>(x32[2]), vshrq_n_u32::<2>(x32[3])];
            let t742: [uint32x4_t; 2] = [t741[0], t741[1]];
            x40 = t742;
            let t743: [uint32x4_t; 2] = [vsubq_u32(x40[0], t730[0]), vsubq_u32(x40[1], t730[1])];
            let t744: [uint32x4_t; 2] = [vcgeq_u32(t743[0], x16[0]), vcgeq_u32(t743[1], x16[1])];
            let t745: [uint32x4_t; 2] = [vandq_u32(live[0], t744[0]), vandq_u32(live[1], t744[1])];
            {
                let t746: u64 = 4294967295u64;
                let t747 = [vdupq_n_u32((t746) as u32); 2];
                let t748: [uint32x4_t; 4] = [t747[0], t747[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t745[0], t748[0], res[0]), vbslq_u32(t745[1], t748[1], res[1]), vbslq_u32(t745[0], t748[2], res[2]), vbslq_u32(t745[1], t748[3], res[3])];
                live = [vbicq_u32(live[0], t745[0]), vbicq_u32(live[1], t745[1])];
            }
            if any(&live) {
                let t749: [uint32x4_t; 2] = [vsubq_u32(x16[0], x40[0]), vsubq_u32(x16[1], x40[1])];
                x48 = t749;
                let t750: u64 = 2u64;
                let t751 = [vdupq_n_u32((t750) as u32); 2];
                let t752: [uint32x4_t; 2] = [vandq_u32(x32[0], t751[0]), vandq_u32(x32[1], t751[1])];
                let t753: u64 = 0u64;
                let t754 = [vdupq_n_u32((t753) as u32); 2];
                let t755: [uint32x4_t; 2] = [vceqq_u32(t752[0], t754[0]), vceqq_u32(t752[1], t754[1])];
                let t756: [uint32x4_t; 2] = [vmvnq_u32(t755[0]), vmvnq_u32(t755[1])];
                let t757: [uint32x4_t; 2] = [vandq_u32(live[0], t756[0]), vandq_u32(live[1], t756[1])];
                {
                    let t758: [uint32x4_t; 4] = [x48[0], x48[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                    res = [vbslq_u32(t757[0], t758[0], res[0]), vbslq_u32(t757[1], t758[1], res[1]), vbslq_u32(t757[0], t758[2], res[2]), vbslq_u32(t757[1], t758[3], res[3])];
                    live = [vbicq_u32(live[0], t757[0]), vbicq_u32(live[1], t757[1])];
                }
                if any(&live) {
                    let t759: [uint32x4_t; 2] = [vceqq_u32(x48[0], t724[0]), vceqq_u32(x48[1], t724[1])];
                    let t760: [uint32x4_t; 2] = [vandq_u32(live[0], t759[0]), vandq_u32(live[1], t759[1])];
                    {
                        let t761: u64 = 4294967295u64;
                        let t762 = [vdupq_n_u32((t761) as u32); 2];
                        let t763: [uint32x4_t; 4] = [t762[0], t762[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                        res = [vbslq_u32(t760[0], t763[0], res[0]), vbslq_u32(t760[1], t763[1], res[1]), vbslq_u32(t760[0], t763[2], res[2]), vbslq_u32(t760[1], t763[3], res[3])];
                        live = [vbicq_u32(live[0], t760[0]), vbicq_u32(live[1], t760[1])];
                    }
                    if any(&live) {
                        let t764: u64 = (1u64).wrapping_shl((32u64) as u32);
                        let t765 = [vdupq_n_u32((t764) as u32); 2];
                        let t766 = [vdupq_n_u32(((t764) >> 32) as u32); 2];
                        let t767: [uint32x4_t; 4] = [vorrq_u32(t765[0], x48[0]), vorrq_u32(t765[1], x48[1]), t766[0], t766[1]];
                        res = [vbslq_u32(live[0], t767[0], res[0]), vbslq_u32(live[1], t767[1], res[1]), vbslq_u32(live[0], t767[2], res[2]), vbslq_u32(live[1], t767[3], res[3])];
                        live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
                    }
                }
            }
        }
    }
    res
}

#[inline(always)]
unsafe fn b9_r3gi_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg3: *const u8, x16: u32) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let t768: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x16), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x16), vld1q_u32(LANE[1].as_ptr()))];
    let t769 = [vdupq_n_u32(31u32); 2];
    let t770: [uint32x4_t; 2] = [vceqq_u32(t768[0], t769[0]), vceqq_u32(t768[1], t769[1])];
    let t771: [uint32x4_t; 2] = [vandq_u32(live[0], t770[0]), vandq_u32(live[1], t770[1])];
    {
        let t772: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t773: [uint32x4_t; 2] = [vshlq_u32(t772[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(t772[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
        let t774: [uint32x4_t; 2] = [vshlq_u32(t772[0], vnegq_s32(vdupq_n_s32(((5u32) & 31) as i32))), vshlq_u32(t772[1], vnegq_s32(vdupq_n_s32(((5u32) & 31) as i32)))];
        let t775: [uint32x4_t; 2] = [vandq_u32(t774[0], t769[0]), vandq_u32(t774[1], t769[1])];
        let t776 = [vdupq_n_u32(1u32); 2];
        let t777: [uint32x4_t; 2] = [vshlq_u32(t776[0], vreinterpretq_s32_u32(vandq_u32(t775[0], vdupq_n_u32(31)))), vshlq_u32(t776[1], vreinterpretq_s32_u32(vandq_u32(t775[1], vdupq_n_u32(31))))];
        let t778 = b13_al4r3l_p(l, lid0, t771, arg3, t773, t777);
    }
}

#[inline(always)]
unsafe fn b9_r3gl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg3: *const u8, x16: [uint32x4_t; 2]) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let t779 = [vdupq_n_u32(31u32); 2];
    let t780: [uint32x4_t; 2] = [vceqq_u32(x16[0], t779[0]), vceqq_u32(x16[1], t779[1])];
    let t781: [uint32x4_t; 2] = [vandq_u32(live[0], t780[0]), vandq_u32(live[1], t780[1])];
    {
        let t782: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t783: [uint32x4_t; 2] = [vshlq_u32(t782[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(t782[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
        let t784: [uint32x4_t; 2] = [vshlq_u32(t782[0], vnegq_s32(vdupq_n_s32(((5u32) & 31) as i32))), vshlq_u32(t782[1], vnegq_s32(vdupq_n_s32(((5u32) & 31) as i32)))];
        let t785: [uint32x4_t; 2] = [vandq_u32(t784[0], t779[0]), vandq_u32(t784[1], t779[1])];
        let t786 = [vdupq_n_u32(1u32); 2];
        let t787: [uint32x4_t; 2] = [vshlq_u32(t786[0], vreinterpretq_s32_u32(vandq_u32(t785[0], vdupq_n_u32(31)))), vshlq_u32(t786[1], vreinterpretq_s32_u32(vandq_u32(t785[1], vdupq_n_u32(31))))];
        let t788 = b13_al4r3l_p(l, lid0, t781, arg3, t783, t787);
    }
}

#[inline(always)]
unsafe fn b10_r3ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg3: *const u8, x8: [uint32x4_t; 2], x16: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t789: [uint32x4_t; 2] = [vshlq_u32(x8[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x8[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t790 = b11_al4r3_p(l, lid0, live, arg3, t789);
    x24 = t790;
    let t791: [uint32x4_t; 2] = [vcgeq_u32(x8[0], x16[0]), vcgeq_u32(x8[1], x16[1])];
    let t792: [uint32x4_t; 2] = [vshlq_u32(x8[0], vnegq_s32(vdupq_n_s32(((5u32) & 31) as i32))), vshlq_u32(x8[1], vnegq_s32(vdupq_n_s32(((5u32) & 31) as i32)))];
    let t793 = [vdupq_n_u32(31u32); 2];
    let t794: [uint32x4_t; 2] = [vandq_u32(t792[0], t793[0]), vandq_u32(t792[1], t793[1])];
    let t795: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vreinterpretq_s32_u32(vandq_u32(t794[0], vdupq_n_u32(31))))), vshlq_u32(x24[1], vnegq_s32(vreinterpretq_s32_u32(vandq_u32(t794[1], vdupq_n_u32(31)))))];
    let t796 = [vdupq_n_u32(1u32); 2];
    let t797: [uint32x4_t; 2] = [vandq_u32(t795[0], t796[0]), vandq_u32(t795[1], t796[1])];
    let t798 = [vdupq_n_u32(0u32); 2];
    let t799: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t797[0], t798[0])), vmvnq_u32(vceqq_u32(t797[1], t798[1]))];
    let t800: [uint32x4_t; 2] = [vorrq_u32(t791[0], t799[0]), vorrq_u32(t791[1], t799[1])];
    res = t800;
    return res;
}

#[inline(always)]
unsafe fn b11_al4r2_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg2: *const u8, x0i: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t801 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], x0i[0]), vandq_u32(live[1], x0i[1]))));
    let t802: [uint32x4_t; 2] = [vbslq_u32(live[0], x0i[0], t801), vbslq_u32(live[1], x0i[1], t801)];
    let t803 = arg2 as *const u32;
    let mut t804: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    t804[0] = vld1q_lane_u32::<0>(t803.add(vgetq_lane_u32::<0>(t802[0]) as usize), t804[0]);
    t804[0] = vld1q_lane_u32::<1>(t803.add(vgetq_lane_u32::<1>(t802[0]) as usize), t804[0]);
    t804[0] = vld1q_lane_u32::<2>(t803.add(vgetq_lane_u32::<2>(t802[0]) as usize), t804[0]);
    t804[0] = vld1q_lane_u32::<3>(t803.add(vgetq_lane_u32::<3>(t802[0]) as usize), t804[0]);
    t804[1] = vld1q_lane_u32::<0>(t803.add(vgetq_lane_u32::<0>(t802[1]) as usize), t804[1]);
    t804[1] = vld1q_lane_u32::<1>(t803.add(vgetq_lane_u32::<1>(t802[1]) as usize), t804[1]);
    t804[1] = vld1q_lane_u32::<2>(t803.add(vgetq_lane_u32::<2>(t802[1]) as usize), t804[1]);
    t804[1] = vld1q_lane_u32::<3>(t803.add(vgetq_lane_u32::<3>(t802[1]) as usize), t804[1]);
    res = t804;
    return res;
}

#[inline(always)]
unsafe fn b11_al4r3_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg3: *const u8, x0i: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t805 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], x0i[0]), vandq_u32(live[1], x0i[1]))));
    let t806: [uint32x4_t; 2] = [vbslq_u32(live[0], x0i[0], t805), vbslq_u32(live[1], x0i[1], t805)];
    let t807 = arg3 as *const u32;
    let mut t808: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    t808[0] = vld1q_lane_u32::<0>(t807.add(vgetq_lane_u32::<0>(t806[0]) as usize), t808[0]);
    t808[0] = vld1q_lane_u32::<1>(t807.add(vgetq_lane_u32::<1>(t806[0]) as usize), t808[0]);
    t808[0] = vld1q_lane_u32::<2>(t807.add(vgetq_lane_u32::<2>(t806[0]) as usize), t808[0]);
    t808[0] = vld1q_lane_u32::<3>(t807.add(vgetq_lane_u32::<3>(t806[0]) as usize), t808[0]);
    t808[1] = vld1q_lane_u32::<0>(t807.add(vgetq_lane_u32::<0>(t806[1]) as usize), t808[1]);
    t808[1] = vld1q_lane_u32::<1>(t807.add(vgetq_lane_u32::<1>(t806[1]) as usize), t808[1]);
    t808[1] = vld1q_lane_u32::<2>(t807.add(vgetq_lane_u32::<2>(t806[1]) as usize), t808[1]);
    t808[1] = vld1q_lane_u32::<3>(t807.add(vgetq_lane_u32::<3>(t806[1]) as usize), t808[1]);
    res = t808;
    return res;
}

#[inline(always)]
unsafe fn b12_r0r1i_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t809 = b14_r0r1si_p(l, lid0, live, arg0, arg1, 0u32, x16);
    res = t809;
    return res;
}

#[inline(always)]
unsafe fn b12_r0r1i_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t810 = b14_r0r1si_f(l, lid0, arg0, arg1, 0u32, x16);
    res = t810;
    return res;
}

#[inline(always)]
unsafe fn b12_r0r1u8_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t811 = b14_r0r1su8_p(l, lid0, live, arg0, arg1, 0u32, x16);
    res = t811;
    return res;
}

#[inline(always)]
unsafe fn b12_r0r1u8_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t812 = b14_r0r1su8_f(l, lid0, arg0, arg1, 0u32, x16);
    res = t812;
    return res;
}

#[inline(always)]
unsafe fn b12_r0r1l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t813 = b14_r0r1sl_p(l, lid0, live, arg0, arg1, 0u32, x16);
    res = t813;
    return res;
}

#[inline(always)]
unsafe fn b13_al4r3l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg3: *const u8, x0i: [uint32x4_t; 2], x8: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let (mut t814, mut t815, mut t816, mut t817) = ([0u32; 8], [0u32; 8], [0u32; 8], [0u32; 8]);
    for r in 0..2 { vst1q_u32(t814.as_mut_ptr().add(4 * r), live[r]); vst1q_u32(t815.as_mut_ptr().add(4 * r), x0i[r]); vst1q_u32(t816.as_mut_ptr().add(4 * r), x8[r]); }
    for j in 0..8 { if t814[j] != 0 { t817[j] = AtomicU32::from_ptr((arg3 as *mut u8).add(t815[j] as usize * 4) as *mut u32).fetch_or(t816[j], Relaxed); } }
    let t818: [uint32x4_t; 2] = [vld1q_u32(t817.as_ptr().add(0)), vld1q_u32(t817.as_ptr().add(4))];
    res = t818;
    return res;
}

#[inline(always)]
unsafe fn b14_r0r1si_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: u64 = 0;
    let mut x56: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t819 = b15_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    x32 = t819;
    let t820: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[1].as_ptr()))];
    let t821: [uint32x4_t; 2] = [vshlq_u32(t820[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(t820[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t822 = b16_r0r1sl_p(l, lid0, live, arg0, arg1, (1u32).wrapping_add(x16), t821);
    x40 = t822;
    let t823 = b17_s_p(l, lid0, live, x32);
    x48 = t823;
    let t824 = [vdupq_n_u32(1023u32); 2];
    let t825: [uint32x4_t; 2] = [vandq_u32(t820[0], t824[0]), vandq_u32(t820[1], t824[1])];
    let t826 = [vdupq_n_u32(x32); 2];
    let t827: [uint32x4_t; 2] = [vmulq_u32(t825[0], t826[0]), vmulq_u32(t825[1], t826[1])];
    let t828 = b18_r0r1lls_p(l, lid0, live, arg0, arg1, x40, t827, 8u32);
    let t829: u64 = x48;
    let t830 = [vdupq_n_u32((t829) as u32); 2];
    let t831 = [vdupq_n_u32(((t829) >> 32) as u32); 2];
    let t832: [uint32x4_t; 4] = [vandq_u32(t828[0], t830[0]), vandq_u32(t828[1], t830[1]), vandq_u32(t828[2], t831[0]), vandq_u32(t828[3], t831[1])];
    x56 = t832;
    let t833: u64 = x48;
    let t834 = [vdupq_n_u32((t833) as u32); 2];
    let t835 = [vdupq_n_u32(((t833) >> 32) as u32); 2];
    let t836: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x56[0], t834[0]), vceqq_u32(x56[2], t835[0])), vandq_u32(vceqq_u32(x56[1], t834[1]), vceqq_u32(x56[3], t835[1]))];
    let mut t837: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t838: [uint32x4_t; 2] = [vandq_u32(live[0], t836[0]), vandq_u32(live[1], t836[1])];
    let t839: [uint32x4_t; 2] = [vbicq_u32(live[0], t836[0]), vbicq_u32(live[1], t836[1])];
    if any(&t838) {
        let t840: [uint32x4_t; 4];
        let t841 = bits(&t838);
        if t841.count_ones() <= 2 {
            let mut t842 = [0u32; 16];
            let mut t843 = t841;
            while t843 != 0 {
                let j = t843.trailing_zeros() as usize;
                t843 &= t843 - 1;
                let t844 = b19_r0r1ss_f(l, lid0, arg0, arg1, (x16).wrapping_add(1u32), (x24).wrapping_add(j as u32));
                let t845: u64 = t844;
                t842[j] = t845 as u32;
                t842[8 + j] = (t845 >> 32) as u32;
            }
            t840 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t842);
        } else {
            let t846 = b19_r0r1si_p(l, lid0, t838, arg0, arg1, (x16).wrapping_add(1u32), x24);
            t840 = t846;
        }
        t837 = [vbslq_u32(t838[0], t840[0], t837[0]), vbslq_u32(t838[1], t840[1], t837[1]), vbslq_u32(t838[0], t840[2], t837[2]), vbslq_u32(t838[1], t840[3], t837[3])];
    }
    {
        t837 = [vbslq_u32(t839[0], x56[0], t837[0]), vbslq_u32(t839[1], x56[1], t837[1]), vbslq_u32(t839[0], x56[2], t837[2]), vbslq_u32(t839[1], x56[3], t837[3])];
    }
    res = t837;
    return res;
}

#[inline(always)]
unsafe fn b14_r0r1si_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32, x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: u64 = 0;
    let mut x56: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t847 = b15_r0r1s_f(l, lid0, arg0, arg1, x16);
    x32 = t847;
    let t848: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[1].as_ptr()))];
    let t849: [uint32x4_t; 2] = [vshlq_u32(t848[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(t848[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t850 = b16_r0r1sl_f(l, lid0, arg0, arg1, (1u32).wrapping_add(x16), t849);
    x40 = t850;
    let t851 = b17_s_f(l, lid0, x32);
    x48 = t851;
    let t852 = [vdupq_n_u32(1023u32); 2];
    let t853: [uint32x4_t; 2] = [vandq_u32(t848[0], t852[0]), vandq_u32(t848[1], t852[1])];
    let t854 = [vdupq_n_u32(x32); 2];
    let t855: [uint32x4_t; 2] = [vmulq_u32(t853[0], t854[0]), vmulq_u32(t853[1], t854[1])];
    let t856 = b18_r0r1lls_f(l, lid0, arg0, arg1, x40, t855, 8u32);
    let t857: u64 = x48;
    let t858 = [vdupq_n_u32((t857) as u32); 2];
    let t859 = [vdupq_n_u32(((t857) >> 32) as u32); 2];
    let t860: [uint32x4_t; 4] = [vandq_u32(t856[0], t858[0]), vandq_u32(t856[1], t858[1]), vandq_u32(t856[2], t859[0]), vandq_u32(t856[3], t859[1])];
    x56 = t860;
    let t861: u64 = x48;
    let t862 = [vdupq_n_u32((t861) as u32); 2];
    let t863 = [vdupq_n_u32(((t861) >> 32) as u32); 2];
    let t864: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x56[0], t862[0]), vceqq_u32(x56[2], t863[0])), vandq_u32(vceqq_u32(x56[1], t862[1]), vceqq_u32(x56[3], t863[1]))];
    let mut t865: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t866: [uint32x4_t; 2] = [vmvnq_u32(t864[0]), vmvnq_u32(t864[1])];
    if any(&t864) {
        let t867: [uint32x4_t; 4];
        let t868 = bits(&t864);
        if t868.count_ones() <= 2 {
            let mut t869 = [0u32; 16];
            let mut t870 = t868;
            while t870 != 0 {
                let j = t870.trailing_zeros() as usize;
                t870 &= t870 - 1;
                let t871 = b19_r0r1ss_f(l, lid0, arg0, arg1, (x16).wrapping_add(1u32), (x24).wrapping_add(j as u32));
                let t872: u64 = t871;
                t869[j] = t872 as u32;
                t869[8 + j] = (t872 >> 32) as u32;
            }
            t867 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t869);
        } else {
            let t873 = b19_r0r1si_p(l, lid0, t864, arg0, arg1, (x16).wrapping_add(1u32), x24);
            t867 = t873;
        }
        t865 = [vbslq_u32(t864[0], t867[0], t865[0]), vbslq_u32(t864[1], t867[1], t865[1]), vbslq_u32(t864[0], t867[2], t865[2]), vbslq_u32(t864[1], t867[3], t865[3])];
    }
    {
        t865 = [vbslq_u32(t866[0], x56[0], t865[0]), vbslq_u32(t866[1], x56[1], t865[1]), vbslq_u32(t866[0], x56[2], t865[2]), vbslq_u32(t866[1], x56[3], t865[3])];
    }
    res = t865;
    return res;
}

#[inline(always)]
unsafe fn b14_r0r1su8_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: u32 = 0;
    let mut x48: u64 = 0;
    let mut x56: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t874 = b15_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    x32 = t874;
    let t875 = b16_r0r1ss_p(l, lid0, live, arg0, arg1, (1u32).wrapping_add(x16), (x24 >> 10));
    x40 = t875;
    let t876 = b17_s_p(l, lid0, live, x32);
    x48 = t876;
    let t877 = b18_r0r1sns_p(l, lid0, live, arg0, arg1, x40, (x24 & 1023u32).wrapping_mul(x32), x32, 8u32);
    let t878: u64 = x48;
    let t879 = [vdupq_n_u32((t878) as u32); 2];
    let t880 = [vdupq_n_u32(((t878) >> 32) as u32); 2];
    let t881: [uint32x4_t; 4] = [vandq_u32(t877[0], t879[0]), vandq_u32(t877[1], t879[1]), vandq_u32(t877[2], t880[0]), vandq_u32(t877[3], t880[1])];
    x56 = t881;
    let t882: u64 = x48;
    let t883 = [vdupq_n_u32((t882) as u32); 2];
    let t884 = [vdupq_n_u32(((t882) >> 32) as u32); 2];
    let t885: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x56[0], t883[0]), vceqq_u32(x56[2], t884[0])), vandq_u32(vceqq_u32(x56[1], t883[1]), vceqq_u32(x56[3], t884[1]))];
    let mut t886: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t887: [uint32x4_t; 2] = [vandq_u32(live[0], t885[0]), vandq_u32(live[1], t885[1])];
    let t888: [uint32x4_t; 2] = [vbicq_u32(live[0], t885[0]), vbicq_u32(live[1], t885[1])];
    if any(&t887) {
        let t889: [uint32x4_t; 4];
        let t890 = bits(&t887);
        if t890.count_ones() <= 2 {
            let mut t891 = [0u32; 16];
            let mut t892 = t890;
            while t892 != 0 {
                let j = t892.trailing_zeros() as usize;
                t892 &= t892 - 1;
                let t893 = b19_r0r1ss_f(l, lid0, arg0, arg1, (x16).wrapping_add(1u32), (x24).wrapping_add(j as u32));
                let t894: u64 = t893;
                t891[j] = t894 as u32;
                t891[8 + j] = (t894 >> 32) as u32;
            }
            t889 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t891);
        } else {
            let t895 = b19_r0r1su8_p(l, lid0, t887, arg0, arg1, (x16).wrapping_add(1u32), x24);
            t889 = t895;
        }
        t886 = [vbslq_u32(t887[0], t889[0], t886[0]), vbslq_u32(t887[1], t889[1], t886[1]), vbslq_u32(t887[0], t889[2], t886[2]), vbslq_u32(t887[1], t889[3], t886[3])];
    }
    {
        t886 = [vbslq_u32(t888[0], x56[0], t886[0]), vbslq_u32(t888[1], x56[1], t886[1]), vbslq_u32(t888[0], x56[2], t886[2]), vbslq_u32(t888[1], x56[3], t886[3])];
    }
    res = t886;
    return res;
}

#[inline(always)]
unsafe fn b14_r0r1su8_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32, x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: u32 = 0;
    let mut x48: u64 = 0;
    let mut x56: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t896 = b15_r0r1s_f(l, lid0, arg0, arg1, x16);
    x32 = t896;
    let t897 = b16_r0r1ss_f(l, lid0, arg0, arg1, (1u32).wrapping_add(x16), (x24 >> 10));
    x40 = t897;
    let t898 = b17_s_f(l, lid0, x32);
    x48 = t898;
    let t899 = b18_r0r1sns_f(l, lid0, arg0, arg1, x40, (x24 & 1023u32).wrapping_mul(x32), x32, 8u32);
    let t900: u64 = x48;
    let t901 = [vdupq_n_u32((t900) as u32); 2];
    let t902 = [vdupq_n_u32(((t900) >> 32) as u32); 2];
    let t903: [uint32x4_t; 4] = [vandq_u32(t899[0], t901[0]), vandq_u32(t899[1], t901[1]), vandq_u32(t899[2], t902[0]), vandq_u32(t899[3], t902[1])];
    x56 = t903;
    let t904: u64 = x48;
    let t905 = [vdupq_n_u32((t904) as u32); 2];
    let t906 = [vdupq_n_u32(((t904) >> 32) as u32); 2];
    let t907: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x56[0], t905[0]), vceqq_u32(x56[2], t906[0])), vandq_u32(vceqq_u32(x56[1], t905[1]), vceqq_u32(x56[3], t906[1]))];
    let mut t908: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t909: [uint32x4_t; 2] = [vmvnq_u32(t907[0]), vmvnq_u32(t907[1])];
    if any(&t907) {
        let t910: [uint32x4_t; 4];
        let t911 = bits(&t907);
        if t911.count_ones() <= 2 {
            let mut t912 = [0u32; 16];
            let mut t913 = t911;
            while t913 != 0 {
                let j = t913.trailing_zeros() as usize;
                t913 &= t913 - 1;
                let t914 = b19_r0r1ss_f(l, lid0, arg0, arg1, (x16).wrapping_add(1u32), (x24).wrapping_add(j as u32));
                let t915: u64 = t914;
                t912[j] = t915 as u32;
                t912[8 + j] = (t915 >> 32) as u32;
            }
            t910 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t912);
        } else {
            let t916 = b19_r0r1su8_p(l, lid0, t907, arg0, arg1, (x16).wrapping_add(1u32), x24);
            t910 = t916;
        }
        t908 = [vbslq_u32(t907[0], t910[0], t908[0]), vbslq_u32(t907[1], t910[1], t908[1]), vbslq_u32(t907[0], t910[2], t908[2]), vbslq_u32(t907[1], t910[3], t908[3])];
    }
    {
        t908 = [vbslq_u32(t909[0], x56[0], t908[0]), vbslq_u32(t909[1], x56[1], t908[1]), vbslq_u32(t909[0], x56[2], t908[2]), vbslq_u32(t909[1], x56[3], t908[3])];
    }
    res = t908;
    return res;
}

#[inline(always)]
unsafe fn b14_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: u64 = 0;
    let mut x56: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t917 = b15_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    x32 = t917;
    let t918: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t919 = b16_r0r1sl_p(l, lid0, live, arg0, arg1, (1u32).wrapping_add(x16), t918);
    x40 = t919;
    let t920 = b17_s_p(l, lid0, live, x32);
    x48 = t920;
    let t921 = [vdupq_n_u32(1023u32); 2];
    let t922: [uint32x4_t; 2] = [vandq_u32(x24[0], t921[0]), vandq_u32(x24[1], t921[1])];
    let t923 = [vdupq_n_u32(x32); 2];
    let t924: [uint32x4_t; 2] = [vmulq_u32(t922[0], t923[0]), vmulq_u32(t922[1], t923[1])];
    let t925 = b18_r0r1lls_p(l, lid0, live, arg0, arg1, x40, t924, 8u32);
    let t926: u64 = x48;
    let t927 = [vdupq_n_u32((t926) as u32); 2];
    let t928 = [vdupq_n_u32(((t926) >> 32) as u32); 2];
    let t929: [uint32x4_t; 4] = [vandq_u32(t925[0], t927[0]), vandq_u32(t925[1], t927[1]), vandq_u32(t925[2], t928[0]), vandq_u32(t925[3], t928[1])];
    x56 = t929;
    let t930: u64 = x48;
    let t931 = [vdupq_n_u32((t930) as u32); 2];
    let t932 = [vdupq_n_u32(((t930) >> 32) as u32); 2];
    let t933: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x56[0], t931[0]), vceqq_u32(x56[2], t932[0])), vandq_u32(vceqq_u32(x56[1], t931[1]), vceqq_u32(x56[3], t932[1]))];
    let mut t934: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t935: [uint32x4_t; 2] = [vandq_u32(live[0], t933[0]), vandq_u32(live[1], t933[1])];
    let t936: [uint32x4_t; 2] = [vbicq_u32(live[0], t933[0]), vbicq_u32(live[1], t933[1])];
    if any(&t935) {
        let t937: [uint32x4_t; 4];
        let t938 = bits(&t935);
        if t938.count_ones() <= 2 {
            let t939 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x24);
            let mut t940 = [0u32; 16];
            let mut t941 = t938;
            while t941 != 0 {
                let j = t941.trailing_zeros() as usize;
                t941 &= t941 - 1;
                let t942 = b19_r0r1ss_f(l, lid0, arg0, arg1, (x16).wrapping_add(1u32), t939[j]);
                let t943: u64 = t942;
                t940[j] = t943 as u32;
                t940[8 + j] = (t943 >> 32) as u32;
            }
            t937 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t940);
        } else {
            let t944 = b19_r0r1sl_p(l, lid0, t935, arg0, arg1, (x16).wrapping_add(1u32), x24);
            t937 = t944;
        }
        t934 = [vbslq_u32(t935[0], t937[0], t934[0]), vbslq_u32(t935[1], t937[1], t934[1]), vbslq_u32(t935[0], t937[2], t934[2]), vbslq_u32(t935[1], t937[3], t934[3])];
    }
    {
        t934 = [vbslq_u32(t936[0], x56[0], t934[0]), vbslq_u32(t936[1], x56[1], t934[1]), vbslq_u32(t936[0], x56[2], t934[2]), vbslq_u32(t936[1], x56[3], t934[3])];
    }
    res = t934;
    return res;
}

#[inline(always)]
unsafe fn b15_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t945 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    res = (((t945).wrapping_shr(((4u32).wrapping_mul(x16)) as u32)) & (15u32));
    return res;
}

#[inline(always)]
unsafe fn b15_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t946 = b20_r0r1s_f(l, lid0, arg0, arg1, 16u32);
    res = (((t946).wrapping_shr(((4u32).wrapping_mul(x16)) as u32)) & (15u32));
    return res;
}

#[inline(always)]
unsafe fn b16_r0r1ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: u32 = 0;
    let mut live = k;
    let mut res: u32 = 0;
    let t947 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 40u32);
    let t948 = b21_r0r1s_p(l, lid0, live, arg0, arg1, 0u32);
    x32 = ((x24).wrapping_mul(t947)).wrapping_add(t948);
    if ((x16) == (0u32)) {
        res = x32;
        return res;
    }
    let t949 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    let t950 = b22_ss_p(l, lid0, live, (t949).wrapping_sub((x24).wrapping_shl((10u32) as u32)), 1024u32);
    x40 = t950;
    let t951 = b21_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    res = (x32).wrapping_add((x40).wrapping_mul(t951));
    return res;
}

#[inline(always)]
unsafe fn b16_r0r1ss_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32, x24: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: u32 = 0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t952 = b20_r0r1s_f(l, lid0, arg0, arg1, 40u32);
    let t953 = b21_r0r1s_f(l, lid0, arg0, arg1, 0u32);
    x32 = ((x24).wrapping_mul(t952)).wrapping_add(t953);
    if ((x16) == (0u32)) {
        res = x32;
        return res;
    }
    let t954 = b20_r0r1s_f(l, lid0, arg0, arg1, 4u32);
    let t955 = b22_ss_f(l, lid0, (t954).wrapping_sub((x24).wrapping_shl((10u32) as u32)), 1024u32);
    x40 = t955;
    let t956 = b21_r0r1s_f(l, lid0, arg0, arg1, x16);
    res = (x32).wrapping_add((x40).wrapping_mul(t956));
    return res;
}

#[inline(always)]
unsafe fn b16_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t957 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 40u32);
    let t958 = [vdupq_n_u32(t957); 2];
    let t959: [uint32x4_t; 2] = [vmulq_u32(x24[0], t958[0]), vmulq_u32(x24[1], t958[1])];
    let t960 = b21_r0r1s_p(l, lid0, live, arg0, arg1, 0u32);
    let t961 = [vdupq_n_u32(t960); 2];
    let t962: [uint32x4_t; 2] = [vaddq_u32(t959[0], t961[0]), vaddq_u32(t959[1], t961[1])];
    x32 = t962;
    if ((x16) == (0u32)) {
        res = x32;
        return res;
    }
    let t963 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    let t964: [uint32x4_t; 2] = [vshlq_u32(x24[0], vdupq_n_s32(((10u32) & 31) as i32)), vshlq_u32(x24[1], vdupq_n_s32(((10u32) & 31) as i32))];
    let t965 = [vdupq_n_u32(t963); 2];
    let t966: [uint32x4_t; 2] = [vsubq_u32(t965[0], t964[0]), vsubq_u32(t965[1], t964[1])];
    let t967 = b22_ls_p(l, lid0, live, t966, 1024u32);
    x40 = t967;
    let t968 = b21_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    let t969 = [vdupq_n_u32(t968); 2];
    let t970: [uint32x4_t; 2] = [vmulq_u32(x40[0], t969[0]), vmulq_u32(x40[1], t969[1])];
    let t971: [uint32x4_t; 2] = [vaddq_u32(x32[0], t970[0]), vaddq_u32(x32[1], t970[1])];
    res = t971;
    return res;
}

#[inline(always)]
unsafe fn b16_r0r1sl_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t972 = b20_r0r1s_f(l, lid0, arg0, arg1, 40u32);
    let t973 = [vdupq_n_u32(t972); 2];
    let t974: [uint32x4_t; 2] = [vmulq_u32(x24[0], t973[0]), vmulq_u32(x24[1], t973[1])];
    let t975 = b21_r0r1s_f(l, lid0, arg0, arg1, 0u32);
    let t976 = [vdupq_n_u32(t975); 2];
    let t977: [uint32x4_t; 2] = [vaddq_u32(t974[0], t976[0]), vaddq_u32(t974[1], t976[1])];
    x32 = t977;
    if ((x16) == (0u32)) {
        res = x32;
        return res;
    }
    let t978 = b20_r0r1s_f(l, lid0, arg0, arg1, 4u32);
    let t979: [uint32x4_t; 2] = [vshlq_u32(x24[0], vdupq_n_s32(((10u32) & 31) as i32)), vshlq_u32(x24[1], vdupq_n_s32(((10u32) & 31) as i32))];
    let t980 = [vdupq_n_u32(t978); 2];
    let t981: [uint32x4_t; 2] = [vsubq_u32(t980[0], t979[0]), vsubq_u32(t980[1], t979[1])];
    let t982 = b22_ls_f(l, lid0, t981, 1024u32);
    x40 = t982;
    let t983 = b21_r0r1s_f(l, lid0, arg0, arg1, x16);
    let t984 = [vdupq_n_u32(t983); 2];
    let t985: [uint32x4_t; 2] = [vmulq_u32(x40[0], t984[0]), vmulq_u32(x40[1], t984[1])];
    let t986: [uint32x4_t; 2] = [vaddq_u32(x32[0], t985[0]), vaddq_u32(x32[1], t985[1])];
    res = t986;
    return res;
}

#[inline(always)]
unsafe fn b17_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t987: u64;
    if ((x0) >= (8u32)) {
        t987 = !(0u64);
    } else {
        t987 = ((1u64).wrapping_shl(((((x0).wrapping_mul(8u32)) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t987;
    return res;
}

#[inline(always)]
unsafe fn b17_s_f(l: &Launch, lid0: u32, x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t988: u64;
    if ((x0) >= (8u32)) {
        t988 = !(0u64);
    } else {
        t988 = ((1u64).wrapping_shl(((((x0).wrapping_mul(8u32)) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t988;
    return res;
}

#[inline(always)]
unsafe fn b17_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t989 = [vdupq_n_u32(8u32); 2];
    let t990: [uint32x4_t; 2] = [vcgeq_u32(x0[0], t989[0]), vcgeq_u32(x0[1], t989[1])];
    let mut t991: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t992: [uint32x4_t; 2] = [vandq_u32(live[0], t990[0]), vandq_u32(live[1], t990[1])];
    let t993: [uint32x4_t; 2] = [vbicq_u32(live[0], t990[0]), vbicq_u32(live[1], t990[1])];
    {
        let t994: u64 = !(0u64);
        let t995 = [vdupq_n_u32((t994) as u32); 2];
        let t996 = [vdupq_n_u32(((t994) >> 32) as u32); 2];
        let t997: [uint32x4_t; 4] = [t995[0], t995[1], t996[0], t996[1]];
        t991 = [vbslq_u32(t992[0], t997[0], t991[0]), vbslq_u32(t992[1], t997[1], t991[1]), vbslq_u32(t992[0], t997[2], t991[2]), vbslq_u32(t992[1], t997[3], t991[3])];
    }
    {
        let t998: [uint32x4_t; 2] = [vmulq_u32(x0[0], t989[0]), vmulq_u32(x0[1], t989[1])];
        let t999: u64 = 1u64;
        let t1000 = [vdupq_n_u32((t999) as u32); 2];
        let t1001: [uint32x4_t; 2] = [t998[0], t998[1]];
        let t1002: [uint32x4_t; 2] = [vandq_u32(t1001[0], vdupq_n_u32(63)), vandq_u32(t1001[1], vdupq_n_u32(63))];
        let t1003: [uint32x4_t; 4] = [vshlq_u32(t1000[0], vreinterpretq_s32_u32(t1002[0])), vshlq_u32(t1000[1], vreinterpretq_s32_u32(t1002[1])), vshlq_u32(t1000[0], vsubq_s32(vreinterpretq_s32_u32(t1002[0]), vdupq_n_s32(32))), vshlq_u32(t1000[1], vsubq_s32(vreinterpretq_s32_u32(t1002[1]), vdupq_n_s32(32)))];
        let t1004: u64 = 1u64;
        let t1005 = [vdupq_n_u32((t1004) as u32); 2];
        let t1006: [uint32x4_t; 2] = [vcltq_u32(t1003[0], t1005[0]), vcltq_u32(t1003[1], t1005[1])];
        let t1007: [uint32x4_t; 4] = [vsubq_u32(t1003[0], t1005[0]), vsubq_u32(t1003[1], t1005[1]), vaddq_u32(vsubq_u32(t1003[2], vdupq_n_u32(0)), t1006[0]), vaddq_u32(vsubq_u32(t1003[3], vdupq_n_u32(0)), t1006[1])];
        t991 = [vbslq_u32(t993[0], t1007[0], t991[0]), vbslq_u32(t993[1], t1007[1], t991[1]), vbslq_u32(t993[0], t1007[2], t991[2]), vbslq_u32(t993[1], t1007[3], t991[3])];
    }
    res = t991;
    return res;
}

#[inline(always)]
unsafe fn b18_r0r1sns_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24f: u32, x24s: u32, x32: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1008 = b23_r0sns_p(l, lid0, live, arg0, x16, x24f, x24s, x32);
    res = t1008;
    return res;
}

#[inline(always)]
unsafe fn b18_r0r1sns_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32, x24f: u32, x24s: u32, x32: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1009 = b23_r0sns_f(l, lid0, arg0, x16, x24f, x24s, x32);
    res = t1009;
    return res;
}

#[inline(always)]
unsafe fn b18_r0r1lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2], x24: [uint32x4_t; 2], x32: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1010 = b23_r0lls_p(l, lid0, live, arg0, x16, x24, x32);
    res = t1010;
    return res;
}

#[inline(always)]
unsafe fn b18_r0r1lls_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2], x24: [uint32x4_t; 2], x32: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1011 = b23_r0lls_f(l, lid0, arg0, x16, x24, x32);
    res = t1011;
    return res;
}

#[inline(always)]
unsafe fn b19_r0r1ss_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32, x24: u32) -> u64 {
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
    let t1012 = b20_r0r1s_f(l, lid0, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x16)));
    x32 = t1012;
    let t1013 = b20_r0r1s_f(l, lid0, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x16)));
    x40 = t1013;
    x48 = (x24).wrapping_shr((10u32) as u32);
    let t1014 = b24_r0r1sss_f(l, lid0, arg0, arg1, x40, (x48).wrapping_mul(4u32), 4u32);
    x56 = ((t1014) as u32);
    let t1015 = b24_r0r1sss_f(l, lid0, arg0, arg1, x40, ((x48).wrapping_mul(4u32)).wrapping_add(4u32), 4u32);
    x64 = ((t1015) as u32);
    let t1016 = b20_r0r1s_f(l, lid0, arg0, arg1, 4u32);
    x72 = ((t1016).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
    let t1017 = b25_ss_f(l, lid0, 4u32, (x72).wrapping_sub((x48).wrapping_mul(4u32)));
    x80 = t1017;
    x88 = (((x24).wrapping_shr((8u32) as u32)) & (3u32));
    let t1018: u32;
    if ((x88) == (0u32)) {
        t1018 = 0u32;
    } else {
        let t1019 = b26_ss_f(l, lid0, (x56).wrapping_add((2u32).wrapping_mul((x88).wrapping_sub(1u32))), x64);
        let t1020 = b24_r0r1sss_f(l, lid0, arg0, arg1, x32, t1019, 2u32);
        t1018 = ((t1020) as u32);
    }
    x96 = t1018;
    let t1021 = b26_ss_f(l, lid0, (x56).wrapping_add((2u32).wrapping_mul(x88)), x64);
    let t1022 = b24_r0r1sss_f(l, lid0, arg0, arg1, x32, t1021, 2u32);
    x104 = ((t1022) as u32);
    x112 = (x56).wrapping_add((2u32).wrapping_mul(x80));
    x120 = ((x24) & (255u32));
    x128 = 9259542123273814144u64;
    x136 = (((x120) as u64)).wrapping_mul(72340172838076673u64);
    x144 = x96;
    x152 = x96;
    loop {
        // Rounds of one condition: every lane goes on.
        if !(((x152) < (x104))) { break; }
        let t1023 = b26_ss_f(l, lid0, (x112).wrapping_add(x152), x64);
        let t1024 = b24_r0r1sss_f(l, lid0, arg0, arg1, x32, t1023, 8u32);
        x160 = t1024;
        x168 = (((x160) | (x128))).wrapping_sub(((x136) & (!(x128))));
        x176 = ((x128) & (((((!(x160)) & (x136))) | (((!(((x160) ^ (x136)))) & (!(x168)))))));
        x184 = (x104).wrapping_sub(x152);
        let t1025: u64;
        if ((x184) >= (8u32)) {
            t1025 = x176;
        } else {
            t1025 = ((x176) & (((1u64).wrapping_shl(((((8u32).wrapping_mul(x184)) as u64)) as u32)).wrapping_sub(1u64)));
        }
        let t1026 = b27_s_f(l, lid0, t1025);
        x144 = (x144).wrapping_add(t1026);
        x152 = (x152).wrapping_add(8u32);
    }
    x96 = x144;
    let t1027 = b20_r0r1s_f(l, lid0, arg0, arg1, 16u32);
    x152 = (((t1027).wrapping_shr((((((4u32).wrapping_mul(x16)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32));
    let t1028 = b26_ss_f(l, lid0, (x56).wrapping_add((2u32).wrapping_mul((x80).wrapping_sub(1u32))), x64);
    let t1029 = b24_r0r1sss_f(l, lid0, arg0, arg1, x32, t1028, 2u32);
    x160 = ((((t1029) as u32)) as u64);
    let t1030 = b26_ss_f(l, lid0, ((x112).wrapping_add(((x160) as u32))).wrapping_add((x96).wrapping_mul(x152)), x64);
    x168 = ((t1030) as u64);
    let t1031 = b24_r0r1sss_f(l, lid0, arg0, arg1, x32, ((x168) as u32), 8u32);
    let t1032 = b17_s_f(l, lid0, x152);
    res = ((t1031) & (t1032));
    return res;
}

#[inline(always)]
unsafe fn b19_r0r1si_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: u32) -> [uint32x4_t; 4] {
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
    let t1033 = b20_r0r1s_p(l, lid0, live, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x16)));
    x32 = t1033;
    let t1034 = b20_r0r1s_p(l, lid0, live, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x16)));
    x40 = t1034;
    let t1035: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[1].as_ptr()))];
    let t1036: [uint32x4_t; 2] = [vshlq_u32(t1035[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(t1035[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    x48 = t1036;
    let t1037 = [vdupq_n_u32(4u32); 2];
    let t1038: [uint32x4_t; 2] = [vmulq_u32(x48[0], t1037[0]), vmulq_u32(x48[1], t1037[1])];
    let t1039 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x40, t1038, 4u32);
    let t1040: [uint32x4_t; 2] = [t1039[0], t1039[1]];
    x56 = t1040;
    let t1041: [uint32x4_t; 2] = [vmulq_u32(x48[0], t1037[0]), vmulq_u32(x48[1], t1037[1])];
    let t1042: [uint32x4_t; 2] = [vaddq_u32(t1041[0], t1037[0]), vaddq_u32(t1041[1], t1037[1])];
    let t1043 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x40, t1042, 4u32);
    let t1044: [uint32x4_t; 2] = [t1043[0], t1043[1]];
    x64 = t1044;
    let t1045 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    x72 = ((t1045).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
    let t1046: [uint32x4_t; 2] = [vmulq_u32(x48[0], t1037[0]), vmulq_u32(x48[1], t1037[1])];
    let t1047 = [vdupq_n_u32(x72); 2];
    let t1048: [uint32x4_t; 2] = [vsubq_u32(t1047[0], t1046[0]), vsubq_u32(t1047[1], t1046[1])];
    let t1049 = b25_sl_p(l, lid0, live, 4u32, t1048);
    x80 = t1049;
    let t1050: [uint32x4_t; 2] = [vshlq_u32(t1035[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(t1035[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
    let t1051 = [vdupq_n_u32(3u32); 2];
    let t1052: [uint32x4_t; 2] = [vandq_u32(t1050[0], t1051[0]), vandq_u32(t1050[1], t1051[1])];
    x88 = t1052;
    let t1053 = [vdupq_n_u32(0u32); 2];
    let t1054: [uint32x4_t; 2] = [vceqq_u32(x88[0], t1053[0]), vceqq_u32(x88[1], t1053[1])];
    let mut t1055: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1056: [uint32x4_t; 2] = [vandq_u32(live[0], t1054[0]), vandq_u32(live[1], t1054[1])];
    let t1057: [uint32x4_t; 2] = [vbicq_u32(live[0], t1054[0]), vbicq_u32(live[1], t1054[1])];
    {
        t1055 = [vbslq_u32(t1056[0], t1053[0], t1055[0]), vbslq_u32(t1056[1], t1053[1], t1055[1])];
    }
    if any(&t1057) {
        let t1058 = [vdupq_n_u32(1u32); 2];
        let t1059: [uint32x4_t; 2] = [vsubq_u32(x88[0], t1058[0]), vsubq_u32(x88[1], t1058[1])];
        let t1060 = [vdupq_n_u32(2u32); 2];
        let t1061: [uint32x4_t; 2] = [vmulq_u32(t1060[0], t1059[0]), vmulq_u32(t1060[1], t1059[1])];
        let t1062: [uint32x4_t; 2] = [vaddq_u32(x56[0], t1061[0]), vaddq_u32(x56[1], t1061[1])];
        let t1063 = b26_ll_p(l, lid0, t1057, t1062, x64);
        let t1064 = b24_r0r1sls_p(l, lid0, t1057, arg0, arg1, x32, t1063, 2u32);
        let t1065: [uint32x4_t; 2] = [t1064[0], t1064[1]];
        t1055 = [vbslq_u32(t1057[0], t1065[0], t1055[0]), vbslq_u32(t1057[1], t1065[1], t1055[1])];
    }
    x96 = t1055;
    let t1066 = [vdupq_n_u32(2u32); 2];
    let t1067: [uint32x4_t; 2] = [vmulq_u32(t1066[0], x88[0]), vmulq_u32(t1066[1], x88[1])];
    let t1068: [uint32x4_t; 2] = [vaddq_u32(x56[0], t1067[0]), vaddq_u32(x56[1], t1067[1])];
    let t1069 = b26_ll_p(l, lid0, live, t1068, x64);
    let t1070 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t1069, 2u32);
    let t1071: [uint32x4_t; 2] = [t1070[0], t1070[1]];
    x104 = t1071;
    let t1072: [uint32x4_t; 2] = [vmulq_u32(t1066[0], x80[0]), vmulq_u32(t1066[1], x80[1])];
    let t1073: [uint32x4_t; 2] = [vaddq_u32(x56[0], t1072[0]), vaddq_u32(x56[1], t1072[1])];
    x112 = t1073;
    let t1074 = [vdupq_n_u32(255u32); 2];
    let t1075: [uint32x4_t; 2] = [vandq_u32(t1035[0], t1074[0]), vandq_u32(t1035[1], t1074[1])];
    x120 = t1075;
    x128 = 9259542123273814144u64;
    let t1076: u64 = 72340172838076673u64;
    let t1077 = [vdupq_n_u32((t1076) as u32); 2];
    let t1078 = [vdupq_n_u32(((t1076) >> 32) as u32); 2];
    let t1079: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x120[0]), vget_low_u32(t1077[0]))), vreinterpretq_u32_u64(vmull_high_u32(x120[0], t1077[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x120[1]), vget_low_u32(t1077[1]))), vreinterpretq_u32_u64(vmull_high_u32(x120[1], t1077[1])))];
    let t1080: [uint32x4_t; 4] = [vmulq_u32(x120[0], t1077[0]), vmulq_u32(x120[1], t1077[1]), vmlaq_u32(t1079[0], x120[0], t1078[0]), vmlaq_u32(t1079[1], x120[1], t1078[1])];
    x136 = t1080;
    x144 = x96;
    x152 = x96;
    let mut t1082 = live;
    loop {
        // Lanes part: each its own rounds.
        let t1084: [uint32x4_t; 2] = [vcltq_u32(x152[0], x104[0]), vcltq_u32(x152[1], x104[1])];
        t1082 = [vandq_u32(t1082[0], t1084[0]), vandq_u32(t1082[1], t1084[1])];
        if !(any(&t1082)) { break; }
        let t1086: [uint32x4_t; 2] = [vaddq_u32(x112[0], x152[0]), vaddq_u32(x112[1], x152[1])];
        let t1087 = b26_ll_p(l, lid0, t1082, t1086, x64);
        let t1088 = b24_r0r1sls_p(l, lid0, t1082, arg0, arg1, x32, t1087, 8u32);
        x160 = [vbslq_u32(t1082[0], t1088[0], x160[0]), vbslq_u32(t1082[1], t1088[1], x160[1]), vbslq_u32(t1082[0], t1088[2], x160[2]), vbslq_u32(t1082[1], t1088[3], x160[3])];
        let t1089: u64 = x128;
        let t1090 = [vdupq_n_u32((t1089) as u32); 2];
        let t1091 = [vdupq_n_u32(((t1089) >> 32) as u32); 2];
        let t1092: [uint32x4_t; 4] = [vorrq_u32(x160[0], t1090[0]), vorrq_u32(x160[1], t1090[1]), vorrq_u32(x160[2], t1091[0]), vorrq_u32(x160[3], t1091[1])];
        let t1093: u64 = !(x128);
        let t1094 = [vdupq_n_u32((t1093) as u32); 2];
        let t1095 = [vdupq_n_u32(((t1093) >> 32) as u32); 2];
        let t1096: [uint32x4_t; 4] = [vandq_u32(x136[0], t1094[0]), vandq_u32(x136[1], t1094[1]), vandq_u32(x136[2], t1095[0]), vandq_u32(x136[3], t1095[1])];
        let t1097: [uint32x4_t; 2] = [vcltq_u32(t1092[0], t1096[0]), vcltq_u32(t1092[1], t1096[1])];
        let t1098: [uint32x4_t; 4] = [vsubq_u32(t1092[0], t1096[0]), vsubq_u32(t1092[1], t1096[1]), vaddq_u32(vsubq_u32(t1092[2], t1096[2]), t1097[0]), vaddq_u32(vsubq_u32(t1092[3], t1096[3]), t1097[1])];
        x168 = [vbslq_u32(t1082[0], t1098[0], x168[0]), vbslq_u32(t1082[1], t1098[1], x168[1]), vbslq_u32(t1082[0], t1098[2], x168[2]), vbslq_u32(t1082[1], t1098[3], x168[3])];
        let t1099: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x160[r]));
        let t1100: [uint32x4_t; 4] = [vandq_u32(t1099[0], x136[0]), vandq_u32(t1099[1], x136[1]), vandq_u32(t1099[2], x136[2]), vandq_u32(t1099[3], x136[3])];
        let t1101: [uint32x4_t; 4] = [veorq_u32(x160[0], x136[0]), veorq_u32(x160[1], x136[1]), veorq_u32(x160[2], x136[2]), veorq_u32(x160[3], x136[3])];
        let t1102: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(t1101[r]));
        let t1103: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x168[r]));
        let t1104: [uint32x4_t; 4] = [vandq_u32(t1102[0], t1103[0]), vandq_u32(t1102[1], t1103[1]), vandq_u32(t1102[2], t1103[2]), vandq_u32(t1102[3], t1103[3])];
        let t1105: [uint32x4_t; 4] = [vorrq_u32(t1100[0], t1104[0]), vorrq_u32(t1100[1], t1104[1]), vorrq_u32(t1100[2], t1104[2]), vorrq_u32(t1100[3], t1104[3])];
        let t1106: u64 = x128;
        let t1107 = [vdupq_n_u32((t1106) as u32); 2];
        let t1108 = [vdupq_n_u32(((t1106) >> 32) as u32); 2];
        let t1109: [uint32x4_t; 4] = [vandq_u32(t1107[0], t1105[0]), vandq_u32(t1107[1], t1105[1]), vandq_u32(t1108[0], t1105[2]), vandq_u32(t1108[1], t1105[3])];
        x176 = t1109;
        let t1110: [uint32x4_t; 2] = [vsubq_u32(x104[0], x152[0]), vsubq_u32(x104[1], x152[1])];
        x184 = t1110;
        let t1111 = [vdupq_n_u32(8u32); 2];
        let t1112: [uint32x4_t; 2] = [vcgeq_u32(x184[0], t1111[0]), vcgeq_u32(x184[1], t1111[1])];
        let mut t1113: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t1114: [uint32x4_t; 2] = [vandq_u32(t1082[0], t1112[0]), vandq_u32(t1082[1], t1112[1])];
        let t1115: [uint32x4_t; 2] = [vbicq_u32(t1082[0], t1112[0]), vbicq_u32(t1082[1], t1112[1])];
        {
            t1113 = [vbslq_u32(t1114[0], x176[0], t1113[0]), vbslq_u32(t1114[1], x176[1], t1113[1]), vbslq_u32(t1114[0], x176[2], t1113[2]), vbslq_u32(t1114[1], x176[3], t1113[3])];
        }
        {
            let t1116: [uint32x4_t; 2] = [vmulq_u32(t1111[0], x184[0]), vmulq_u32(t1111[1], x184[1])];
            let t1117: u64 = 1u64;
            let t1118 = [vdupq_n_u32((t1117) as u32); 2];
            let t1119: [uint32x4_t; 2] = [t1116[0], t1116[1]];
            let t1120: [uint32x4_t; 2] = [vandq_u32(t1119[0], vdupq_n_u32(63)), vandq_u32(t1119[1], vdupq_n_u32(63))];
            let t1121: [uint32x4_t; 4] = [vshlq_u32(t1118[0], vreinterpretq_s32_u32(t1120[0])), vshlq_u32(t1118[1], vreinterpretq_s32_u32(t1120[1])), vshlq_u32(t1118[0], vsubq_s32(vreinterpretq_s32_u32(t1120[0]), vdupq_n_s32(32))), vshlq_u32(t1118[1], vsubq_s32(vreinterpretq_s32_u32(t1120[1]), vdupq_n_s32(32)))];
            let t1122: u64 = 1u64;
            let t1123 = [vdupq_n_u32((t1122) as u32); 2];
            let t1124: [uint32x4_t; 2] = [vcltq_u32(t1121[0], t1123[0]), vcltq_u32(t1121[1], t1123[1])];
            let t1125: [uint32x4_t; 4] = [vsubq_u32(t1121[0], t1123[0]), vsubq_u32(t1121[1], t1123[1]), vaddq_u32(vsubq_u32(t1121[2], vdupq_n_u32(0)), t1124[0]), vaddq_u32(vsubq_u32(t1121[3], vdupq_n_u32(0)), t1124[1])];
            let t1126: [uint32x4_t; 4] = [vandq_u32(x176[0], t1125[0]), vandq_u32(x176[1], t1125[1]), vandq_u32(x176[2], t1125[2]), vandq_u32(x176[3], t1125[3])];
            t1113 = [vbslq_u32(t1115[0], t1126[0], t1113[0]), vbslq_u32(t1115[1], t1126[1], t1113[1]), vbslq_u32(t1115[0], t1126[2], t1113[2]), vbslq_u32(t1115[1], t1126[3], t1113[3])];
        }
        let t1127 = b27_l_p(l, lid0, t1082, t1113);
        let t1128: [uint32x4_t; 2] = [vaddq_u32(x144[0], t1127[0]), vaddq_u32(x144[1], t1127[1])];
        x144 = [vbslq_u32(t1082[0], t1128[0], x144[0]), vbslq_u32(t1082[1], t1128[1], x144[1])];
        let t1129: [uint32x4_t; 2] = [vandq_u32(t1082[0], live[0]), vandq_u32(t1082[1], live[1])];
        x152 = [vaddq_u32(x152[0], vandq_u32(t1129[0], vdupq_n_u32(8))), vaddq_u32(x152[1], vandq_u32(t1129[1], vdupq_n_u32(8)))];
    }
    x96 = x144;
    let t1130 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    let t1131 = [vdupq_n_u32((((t1130).wrapping_shr((((((4u32).wrapping_mul(x16)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32))); 2];
    x152 = t1131;
    let t1132 = [vdupq_n_u32(1u32); 2];
    let t1133: [uint32x4_t; 2] = [vsubq_u32(x80[0], t1132[0]), vsubq_u32(x80[1], t1132[1])];
    let t1134: [uint32x4_t; 2] = [vmulq_u32(t1066[0], t1133[0]), vmulq_u32(t1066[1], t1133[1])];
    let t1135: [uint32x4_t; 2] = [vaddq_u32(x56[0], t1134[0]), vaddq_u32(x56[1], t1134[1])];
    let t1136 = b26_ll_p(l, lid0, live, t1135, x64);
    let t1137 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t1136, 2u32);
    let t1138: [uint32x4_t; 2] = [t1137[0], t1137[1]];
    let t1139: [uint32x4_t; 4] = [t1138[0], t1138[1], vdupq_n_u32(0), vdupq_n_u32(0)];
    x160 = t1139;
    let t1140: [uint32x4_t; 2] = [x160[0], x160[1]];
    let t1141: [uint32x4_t; 2] = [vaddq_u32(x112[0], t1140[0]), vaddq_u32(x112[1], t1140[1])];
    let t1142: [uint32x4_t; 2] = [vmulq_u32(x96[0], x152[0]), vmulq_u32(x96[1], x152[1])];
    let t1143: [uint32x4_t; 2] = [vaddq_u32(t1141[0], t1142[0]), vaddq_u32(t1141[1], t1142[1])];
    let t1144 = b26_ll_p(l, lid0, live, t1143, x64);
    let t1145: [uint32x4_t; 4] = [t1144[0], t1144[1], vdupq_n_u32(0), vdupq_n_u32(0)];
    x168 = t1145;
    let t1146: [uint32x4_t; 2] = [x168[0], x168[1]];
    let t1147 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t1146, 8u32);
    let t1148 = b17_l_p(l, lid0, live, x152);
    let t1149: [uint32x4_t; 4] = [vandq_u32(t1147[0], t1148[0]), vandq_u32(t1147[1], t1148[1]), vandq_u32(t1147[2], t1148[2]), vandq_u32(t1147[3], t1148[3])];
    res = t1149;
    return res;
}

#[inline(always)]
unsafe fn b19_r0r1su8_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: u32) -> [uint32x4_t; 4] {
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
    let t1150 = b20_r0r1s_p(l, lid0, live, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x16)));
    x32 = t1150;
    let t1151 = b20_r0r1s_p(l, lid0, live, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x16)));
    x40 = t1151;
    x48 = (x24 >> 10);
    let t1152 = b24_r0r1sss_p(l, lid0, live, arg0, arg1, x40, (x48).wrapping_mul(4u32), 4u32);
    x56 = ((t1152) as u32);
    let t1153 = b24_r0r1sss_p(l, lid0, live, arg0, arg1, x40, ((x48).wrapping_mul(4u32)).wrapping_add(4u32), 4u32);
    x64 = ((t1153) as u32);
    let t1154 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    x72 = ((t1154).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
    let t1155 = b25_ss_p(l, lid0, live, 4u32, (x72).wrapping_sub((x48).wrapping_mul(4u32)));
    x80 = t1155;
    x88 = (((x24 >> 8)) & (3u32));
    let t1156: u32;
    if ((x88) == (0u32)) {
        t1156 = 0u32;
    } else {
        let t1157 = b26_ss_p(l, lid0, live, (x56).wrapping_add((2u32).wrapping_mul((x88).wrapping_sub(1u32))), x64);
        let t1158 = b24_r0r1sss_p(l, lid0, live, arg0, arg1, x32, t1157, 2u32);
        t1156 = ((t1158) as u32);
    }
    let t1159 = [vdupq_n_u32(t1156); 2];
    x96 = t1159;
    let t1160 = b26_ss_p(l, lid0, live, (x56).wrapping_add((2u32).wrapping_mul(x88)), x64);
    let t1161 = b24_r0r1sss_p(l, lid0, live, arg0, arg1, x32, t1160, 2u32);
    x104 = ((t1161) as u32);
    x112 = (x56).wrapping_add((2u32).wrapping_mul(x80));
    x120 = (x24 & 255u32);
    x128 = 9259542123273814144u64;
    let t1162: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x120), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x120), vld1q_u32(LANE[1].as_ptr()))];
    let t1163: u64 = 72340172838076673u64;
    let t1164 = [vdupq_n_u32((t1163) as u32); 2];
    let t1165 = [vdupq_n_u32(((t1163) >> 32) as u32); 2];
    let t1166: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(t1162[0]), vget_low_u32(t1164[0]))), vreinterpretq_u32_u64(vmull_high_u32(t1162[0], t1164[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(t1162[1]), vget_low_u32(t1164[1]))), vreinterpretq_u32_u64(vmull_high_u32(t1162[1], t1164[1])))];
    let t1167: [uint32x4_t; 4] = [vmulq_u32(t1162[0], t1164[0]), vmulq_u32(t1162[1], t1164[1]), vmlaq_u32(t1166[0], t1162[0], t1165[0]), vmlaq_u32(t1166[1], t1162[1], t1165[1])];
    x136 = t1167;
    x144 = x96;
    x152 = x96;
    let mut t1170 = live;
    loop {
        // Lanes part: each its own rounds.
        let t1172 = [vdupq_n_u32(x104); 2];
        let t1173: [uint32x4_t; 2] = [vcltq_u32(x152[0], t1172[0]), vcltq_u32(x152[1], t1172[1])];
        t1170 = [vandq_u32(t1170[0], t1173[0]), vandq_u32(t1170[1], t1173[1])];
        if !(any(&t1170)) { break; }
        let t1175 = [vdupq_n_u32(x112); 2];
        let t1176: [uint32x4_t; 2] = [vaddq_u32(t1175[0], x152[0]), vaddq_u32(t1175[1], x152[1])];
        let t1177 = b26_ls_p(l, lid0, t1170, t1176, x64);
        let t1178 = b24_r0r1sls_p(l, lid0, t1170, arg0, arg1, x32, t1177, 8u32);
        x160 = [vbslq_u32(t1170[0], t1178[0], x160[0]), vbslq_u32(t1170[1], t1178[1], x160[1]), vbslq_u32(t1170[0], t1178[2], x160[2]), vbslq_u32(t1170[1], t1178[3], x160[3])];
        let t1179: u64 = x128;
        let t1180 = [vdupq_n_u32((t1179) as u32); 2];
        let t1181 = [vdupq_n_u32(((t1179) >> 32) as u32); 2];
        let t1182: [uint32x4_t; 4] = [vorrq_u32(x160[0], t1180[0]), vorrq_u32(x160[1], t1180[1]), vorrq_u32(x160[2], t1181[0]), vorrq_u32(x160[3], t1181[1])];
        let t1183: u64 = !(x128);
        let t1184 = [vdupq_n_u32((t1183) as u32); 2];
        let t1185 = [vdupq_n_u32(((t1183) >> 32) as u32); 2];
        let t1186: [uint32x4_t; 4] = [vandq_u32(x136[0], t1184[0]), vandq_u32(x136[1], t1184[1]), vandq_u32(x136[2], t1185[0]), vandq_u32(x136[3], t1185[1])];
        let t1187: [uint32x4_t; 2] = [vcltq_u32(t1182[0], t1186[0]), vcltq_u32(t1182[1], t1186[1])];
        let t1188: [uint32x4_t; 4] = [vsubq_u32(t1182[0], t1186[0]), vsubq_u32(t1182[1], t1186[1]), vaddq_u32(vsubq_u32(t1182[2], t1186[2]), t1187[0]), vaddq_u32(vsubq_u32(t1182[3], t1186[3]), t1187[1])];
        x168 = [vbslq_u32(t1170[0], t1188[0], x168[0]), vbslq_u32(t1170[1], t1188[1], x168[1]), vbslq_u32(t1170[0], t1188[2], x168[2]), vbslq_u32(t1170[1], t1188[3], x168[3])];
        let t1189: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x160[r]));
        let t1190: [uint32x4_t; 4] = [vandq_u32(t1189[0], x136[0]), vandq_u32(t1189[1], x136[1]), vandq_u32(t1189[2], x136[2]), vandq_u32(t1189[3], x136[3])];
        let t1191: [uint32x4_t; 4] = [veorq_u32(x160[0], x136[0]), veorq_u32(x160[1], x136[1]), veorq_u32(x160[2], x136[2]), veorq_u32(x160[3], x136[3])];
        let t1192: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(t1191[r]));
        let t1193: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x168[r]));
        let t1194: [uint32x4_t; 4] = [vandq_u32(t1192[0], t1193[0]), vandq_u32(t1192[1], t1193[1]), vandq_u32(t1192[2], t1193[2]), vandq_u32(t1192[3], t1193[3])];
        let t1195: [uint32x4_t; 4] = [vorrq_u32(t1190[0], t1194[0]), vorrq_u32(t1190[1], t1194[1]), vorrq_u32(t1190[2], t1194[2]), vorrq_u32(t1190[3], t1194[3])];
        let t1196: u64 = x128;
        let t1197 = [vdupq_n_u32((t1196) as u32); 2];
        let t1198 = [vdupq_n_u32(((t1196) >> 32) as u32); 2];
        let t1199: [uint32x4_t; 4] = [vandq_u32(t1197[0], t1195[0]), vandq_u32(t1197[1], t1195[1]), vandq_u32(t1198[0], t1195[2]), vandq_u32(t1198[1], t1195[3])];
        x176 = t1199;
        let t1200 = [vdupq_n_u32(x104); 2];
        let t1201: [uint32x4_t; 2] = [vsubq_u32(t1200[0], x152[0]), vsubq_u32(t1200[1], x152[1])];
        x184 = t1201;
        let t1202 = [vdupq_n_u32(8u32); 2];
        let t1203: [uint32x4_t; 2] = [vcgeq_u32(x184[0], t1202[0]), vcgeq_u32(x184[1], t1202[1])];
        let mut t1204: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t1205: [uint32x4_t; 2] = [vandq_u32(t1170[0], t1203[0]), vandq_u32(t1170[1], t1203[1])];
        let t1206: [uint32x4_t; 2] = [vbicq_u32(t1170[0], t1203[0]), vbicq_u32(t1170[1], t1203[1])];
        {
            t1204 = [vbslq_u32(t1205[0], x176[0], t1204[0]), vbslq_u32(t1205[1], x176[1], t1204[1]), vbslq_u32(t1205[0], x176[2], t1204[2]), vbslq_u32(t1205[1], x176[3], t1204[3])];
        }
        {
            let t1207: [uint32x4_t; 2] = [vmulq_u32(t1202[0], x184[0]), vmulq_u32(t1202[1], x184[1])];
            let t1208: u64 = 1u64;
            let t1209 = [vdupq_n_u32((t1208) as u32); 2];
            let t1210: [uint32x4_t; 2] = [t1207[0], t1207[1]];
            let t1211: [uint32x4_t; 2] = [vandq_u32(t1210[0], vdupq_n_u32(63)), vandq_u32(t1210[1], vdupq_n_u32(63))];
            let t1212: [uint32x4_t; 4] = [vshlq_u32(t1209[0], vreinterpretq_s32_u32(t1211[0])), vshlq_u32(t1209[1], vreinterpretq_s32_u32(t1211[1])), vshlq_u32(t1209[0], vsubq_s32(vreinterpretq_s32_u32(t1211[0]), vdupq_n_s32(32))), vshlq_u32(t1209[1], vsubq_s32(vreinterpretq_s32_u32(t1211[1]), vdupq_n_s32(32)))];
            let t1213: u64 = 1u64;
            let t1214 = [vdupq_n_u32((t1213) as u32); 2];
            let t1215: [uint32x4_t; 2] = [vcltq_u32(t1212[0], t1214[0]), vcltq_u32(t1212[1], t1214[1])];
            let t1216: [uint32x4_t; 4] = [vsubq_u32(t1212[0], t1214[0]), vsubq_u32(t1212[1], t1214[1]), vaddq_u32(vsubq_u32(t1212[2], vdupq_n_u32(0)), t1215[0]), vaddq_u32(vsubq_u32(t1212[3], vdupq_n_u32(0)), t1215[1])];
            let t1217: [uint32x4_t; 4] = [vandq_u32(x176[0], t1216[0]), vandq_u32(x176[1], t1216[1]), vandq_u32(x176[2], t1216[2]), vandq_u32(x176[3], t1216[3])];
            t1204 = [vbslq_u32(t1206[0], t1217[0], t1204[0]), vbslq_u32(t1206[1], t1217[1], t1204[1]), vbslq_u32(t1206[0], t1217[2], t1204[2]), vbslq_u32(t1206[1], t1217[3], t1204[3])];
        }
        let t1218 = b27_l_p(l, lid0, t1170, t1204);
        let t1219: [uint32x4_t; 2] = [vaddq_u32(x144[0], t1218[0]), vaddq_u32(x144[1], t1218[1])];
        x144 = [vbslq_u32(t1170[0], t1219[0], x144[0]), vbslq_u32(t1170[1], t1219[1], x144[1])];
        let t1220: [uint32x4_t; 2] = [vandq_u32(t1170[0], live[0]), vandq_u32(t1170[1], live[1])];
        x152 = [vaddq_u32(x152[0], vandq_u32(t1220[0], vdupq_n_u32(8))), vaddq_u32(x152[1], vandq_u32(t1220[1], vdupq_n_u32(8)))];
    }
    x96 = x144;
    let t1221 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    let t1222 = [vdupq_n_u32((((t1221).wrapping_shr((((((4u32).wrapping_mul(x16)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32))); 2];
    x152 = t1222;
    let t1223 = b26_ss_p(l, lid0, live, (x56).wrapping_add((2u32).wrapping_mul((x80).wrapping_sub(1u32))), x64);
    let t1224 = b24_r0r1sss_p(l, lid0, live, arg0, arg1, x32, t1223, 2u32);
    let t1225: u64 = ((((t1224) as u32)) as u64);
    let t1226 = [vdupq_n_u32((t1225) as u32); 2];
    let t1227 = [vdupq_n_u32(((t1225) >> 32) as u32); 2];
    let t1228: [uint32x4_t; 4] = [t1226[0], t1226[1], t1227[0], t1227[1]];
    x160 = t1228;
    let t1229 = [vdupq_n_u32(x112); 2];
    let t1230: [uint32x4_t; 2] = [x160[0], x160[1]];
    let t1231: [uint32x4_t; 2] = [vaddq_u32(t1229[0], t1230[0]), vaddq_u32(t1229[1], t1230[1])];
    let t1232: [uint32x4_t; 2] = [vmulq_u32(x96[0], x152[0]), vmulq_u32(x96[1], x152[1])];
    let t1233: [uint32x4_t; 2] = [vaddq_u32(t1231[0], t1232[0]), vaddq_u32(t1231[1], t1232[1])];
    let t1234 = b26_ls_p(l, lid0, live, t1233, x64);
    let t1235: [uint32x4_t; 4] = [t1234[0], t1234[1], vdupq_n_u32(0), vdupq_n_u32(0)];
    x168 = t1235;
    let t1236: [uint32x4_t; 2] = [x168[0], x168[1]];
    let t1237 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t1236, 8u32);
    let t1238 = b17_l_p(l, lid0, live, x152);
    let t1239: [uint32x4_t; 4] = [vandq_u32(t1237[0], t1238[0]), vandq_u32(t1237[1], t1238[1]), vandq_u32(t1237[2], t1238[2]), vandq_u32(t1237[3], t1238[3])];
    res = t1239;
    return res;
}

#[inline(always)]
unsafe fn b19_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
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
    let t1240 = b20_r0r1s_p(l, lid0, live, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x16)));
    x32 = t1240;
    let t1241 = b20_r0r1s_p(l, lid0, live, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x16)));
    x40 = t1241;
    let t1242: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    x48 = t1242;
    let t1243 = [vdupq_n_u32(4u32); 2];
    let t1244: [uint32x4_t; 2] = [vmulq_u32(x48[0], t1243[0]), vmulq_u32(x48[1], t1243[1])];
    let t1245 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x40, t1244, 4u32);
    let t1246: [uint32x4_t; 2] = [t1245[0], t1245[1]];
    x56 = t1246;
    let t1247: [uint32x4_t; 2] = [vmulq_u32(x48[0], t1243[0]), vmulq_u32(x48[1], t1243[1])];
    let t1248: [uint32x4_t; 2] = [vaddq_u32(t1247[0], t1243[0]), vaddq_u32(t1247[1], t1243[1])];
    let t1249 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x40, t1248, 4u32);
    let t1250: [uint32x4_t; 2] = [t1249[0], t1249[1]];
    x64 = t1250;
    let t1251 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    x72 = ((t1251).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
    let t1252: [uint32x4_t; 2] = [vmulq_u32(x48[0], t1243[0]), vmulq_u32(x48[1], t1243[1])];
    let t1253 = [vdupq_n_u32(x72); 2];
    let t1254: [uint32x4_t; 2] = [vsubq_u32(t1253[0], t1252[0]), vsubq_u32(t1253[1], t1252[1])];
    let t1255 = b25_sl_p(l, lid0, live, 4u32, t1254);
    x80 = t1255;
    let t1256: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
    let t1257 = [vdupq_n_u32(3u32); 2];
    let t1258: [uint32x4_t; 2] = [vandq_u32(t1256[0], t1257[0]), vandq_u32(t1256[1], t1257[1])];
    x88 = t1258;
    let t1259 = [vdupq_n_u32(0u32); 2];
    let t1260: [uint32x4_t; 2] = [vceqq_u32(x88[0], t1259[0]), vceqq_u32(x88[1], t1259[1])];
    let mut t1261: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1262: [uint32x4_t; 2] = [vandq_u32(live[0], t1260[0]), vandq_u32(live[1], t1260[1])];
    let t1263: [uint32x4_t; 2] = [vbicq_u32(live[0], t1260[0]), vbicq_u32(live[1], t1260[1])];
    {
        t1261 = [vbslq_u32(t1262[0], t1259[0], t1261[0]), vbslq_u32(t1262[1], t1259[1], t1261[1])];
    }
    if any(&t1263) {
        let t1264 = [vdupq_n_u32(1u32); 2];
        let t1265: [uint32x4_t; 2] = [vsubq_u32(x88[0], t1264[0]), vsubq_u32(x88[1], t1264[1])];
        let t1266 = [vdupq_n_u32(2u32); 2];
        let t1267: [uint32x4_t; 2] = [vmulq_u32(t1266[0], t1265[0]), vmulq_u32(t1266[1], t1265[1])];
        let t1268: [uint32x4_t; 2] = [vaddq_u32(x56[0], t1267[0]), vaddq_u32(x56[1], t1267[1])];
        let t1269 = b26_ll_p(l, lid0, t1263, t1268, x64);
        let t1270 = b24_r0r1sls_p(l, lid0, t1263, arg0, arg1, x32, t1269, 2u32);
        let t1271: [uint32x4_t; 2] = [t1270[0], t1270[1]];
        t1261 = [vbslq_u32(t1263[0], t1271[0], t1261[0]), vbslq_u32(t1263[1], t1271[1], t1261[1])];
    }
    x96 = t1261;
    let t1272 = [vdupq_n_u32(2u32); 2];
    let t1273: [uint32x4_t; 2] = [vmulq_u32(t1272[0], x88[0]), vmulq_u32(t1272[1], x88[1])];
    let t1274: [uint32x4_t; 2] = [vaddq_u32(x56[0], t1273[0]), vaddq_u32(x56[1], t1273[1])];
    let t1275 = b26_ll_p(l, lid0, live, t1274, x64);
    let t1276 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t1275, 2u32);
    let t1277: [uint32x4_t; 2] = [t1276[0], t1276[1]];
    x104 = t1277;
    let t1278: [uint32x4_t; 2] = [vmulq_u32(t1272[0], x80[0]), vmulq_u32(t1272[1], x80[1])];
    let t1279: [uint32x4_t; 2] = [vaddq_u32(x56[0], t1278[0]), vaddq_u32(x56[1], t1278[1])];
    x112 = t1279;
    let t1280 = [vdupq_n_u32(255u32); 2];
    let t1281: [uint32x4_t; 2] = [vandq_u32(x24[0], t1280[0]), vandq_u32(x24[1], t1280[1])];
    x120 = t1281;
    x128 = 9259542123273814144u64;
    let t1282: u64 = 72340172838076673u64;
    let t1283 = [vdupq_n_u32((t1282) as u32); 2];
    let t1284 = [vdupq_n_u32(((t1282) >> 32) as u32); 2];
    let t1285: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x120[0]), vget_low_u32(t1283[0]))), vreinterpretq_u32_u64(vmull_high_u32(x120[0], t1283[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x120[1]), vget_low_u32(t1283[1]))), vreinterpretq_u32_u64(vmull_high_u32(x120[1], t1283[1])))];
    let t1286: [uint32x4_t; 4] = [vmulq_u32(x120[0], t1283[0]), vmulq_u32(x120[1], t1283[1]), vmlaq_u32(t1285[0], x120[0], t1284[0]), vmlaq_u32(t1285[1], x120[1], t1284[1])];
    x136 = t1286;
    x144 = x96;
    x152 = x96;
    let mut t1288 = live;
    loop {
        // Lanes part: each its own rounds.
        let t1290: [uint32x4_t; 2] = [vcltq_u32(x152[0], x104[0]), vcltq_u32(x152[1], x104[1])];
        t1288 = [vandq_u32(t1288[0], t1290[0]), vandq_u32(t1288[1], t1290[1])];
        if !(any(&t1288)) { break; }
        let t1292: [uint32x4_t; 2] = [vaddq_u32(x112[0], x152[0]), vaddq_u32(x112[1], x152[1])];
        let t1293 = b26_ll_p(l, lid0, t1288, t1292, x64);
        let t1294 = b24_r0r1sls_p(l, lid0, t1288, arg0, arg1, x32, t1293, 8u32);
        x160 = [vbslq_u32(t1288[0], t1294[0], x160[0]), vbslq_u32(t1288[1], t1294[1], x160[1]), vbslq_u32(t1288[0], t1294[2], x160[2]), vbslq_u32(t1288[1], t1294[3], x160[3])];
        let t1295: u64 = x128;
        let t1296 = [vdupq_n_u32((t1295) as u32); 2];
        let t1297 = [vdupq_n_u32(((t1295) >> 32) as u32); 2];
        let t1298: [uint32x4_t; 4] = [vorrq_u32(x160[0], t1296[0]), vorrq_u32(x160[1], t1296[1]), vorrq_u32(x160[2], t1297[0]), vorrq_u32(x160[3], t1297[1])];
        let t1299: u64 = !(x128);
        let t1300 = [vdupq_n_u32((t1299) as u32); 2];
        let t1301 = [vdupq_n_u32(((t1299) >> 32) as u32); 2];
        let t1302: [uint32x4_t; 4] = [vandq_u32(x136[0], t1300[0]), vandq_u32(x136[1], t1300[1]), vandq_u32(x136[2], t1301[0]), vandq_u32(x136[3], t1301[1])];
        let t1303: [uint32x4_t; 2] = [vcltq_u32(t1298[0], t1302[0]), vcltq_u32(t1298[1], t1302[1])];
        let t1304: [uint32x4_t; 4] = [vsubq_u32(t1298[0], t1302[0]), vsubq_u32(t1298[1], t1302[1]), vaddq_u32(vsubq_u32(t1298[2], t1302[2]), t1303[0]), vaddq_u32(vsubq_u32(t1298[3], t1302[3]), t1303[1])];
        x168 = [vbslq_u32(t1288[0], t1304[0], x168[0]), vbslq_u32(t1288[1], t1304[1], x168[1]), vbslq_u32(t1288[0], t1304[2], x168[2]), vbslq_u32(t1288[1], t1304[3], x168[3])];
        let t1305: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x160[r]));
        let t1306: [uint32x4_t; 4] = [vandq_u32(t1305[0], x136[0]), vandq_u32(t1305[1], x136[1]), vandq_u32(t1305[2], x136[2]), vandq_u32(t1305[3], x136[3])];
        let t1307: [uint32x4_t; 4] = [veorq_u32(x160[0], x136[0]), veorq_u32(x160[1], x136[1]), veorq_u32(x160[2], x136[2]), veorq_u32(x160[3], x136[3])];
        let t1308: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(t1307[r]));
        let t1309: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x168[r]));
        let t1310: [uint32x4_t; 4] = [vandq_u32(t1308[0], t1309[0]), vandq_u32(t1308[1], t1309[1]), vandq_u32(t1308[2], t1309[2]), vandq_u32(t1308[3], t1309[3])];
        let t1311: [uint32x4_t; 4] = [vorrq_u32(t1306[0], t1310[0]), vorrq_u32(t1306[1], t1310[1]), vorrq_u32(t1306[2], t1310[2]), vorrq_u32(t1306[3], t1310[3])];
        let t1312: u64 = x128;
        let t1313 = [vdupq_n_u32((t1312) as u32); 2];
        let t1314 = [vdupq_n_u32(((t1312) >> 32) as u32); 2];
        let t1315: [uint32x4_t; 4] = [vandq_u32(t1313[0], t1311[0]), vandq_u32(t1313[1], t1311[1]), vandq_u32(t1314[0], t1311[2]), vandq_u32(t1314[1], t1311[3])];
        x176 = t1315;
        let t1316: [uint32x4_t; 2] = [vsubq_u32(x104[0], x152[0]), vsubq_u32(x104[1], x152[1])];
        x184 = t1316;
        let t1317 = [vdupq_n_u32(8u32); 2];
        let t1318: [uint32x4_t; 2] = [vcgeq_u32(x184[0], t1317[0]), vcgeq_u32(x184[1], t1317[1])];
        let mut t1319: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t1320: [uint32x4_t; 2] = [vandq_u32(t1288[0], t1318[0]), vandq_u32(t1288[1], t1318[1])];
        let t1321: [uint32x4_t; 2] = [vbicq_u32(t1288[0], t1318[0]), vbicq_u32(t1288[1], t1318[1])];
        {
            t1319 = [vbslq_u32(t1320[0], x176[0], t1319[0]), vbslq_u32(t1320[1], x176[1], t1319[1]), vbslq_u32(t1320[0], x176[2], t1319[2]), vbslq_u32(t1320[1], x176[3], t1319[3])];
        }
        {
            let t1322: [uint32x4_t; 2] = [vmulq_u32(t1317[0], x184[0]), vmulq_u32(t1317[1], x184[1])];
            let t1323: u64 = 1u64;
            let t1324 = [vdupq_n_u32((t1323) as u32); 2];
            let t1325: [uint32x4_t; 2] = [t1322[0], t1322[1]];
            let t1326: [uint32x4_t; 2] = [vandq_u32(t1325[0], vdupq_n_u32(63)), vandq_u32(t1325[1], vdupq_n_u32(63))];
            let t1327: [uint32x4_t; 4] = [vshlq_u32(t1324[0], vreinterpretq_s32_u32(t1326[0])), vshlq_u32(t1324[1], vreinterpretq_s32_u32(t1326[1])), vshlq_u32(t1324[0], vsubq_s32(vreinterpretq_s32_u32(t1326[0]), vdupq_n_s32(32))), vshlq_u32(t1324[1], vsubq_s32(vreinterpretq_s32_u32(t1326[1]), vdupq_n_s32(32)))];
            let t1328: u64 = 1u64;
            let t1329 = [vdupq_n_u32((t1328) as u32); 2];
            let t1330: [uint32x4_t; 2] = [vcltq_u32(t1327[0], t1329[0]), vcltq_u32(t1327[1], t1329[1])];
            let t1331: [uint32x4_t; 4] = [vsubq_u32(t1327[0], t1329[0]), vsubq_u32(t1327[1], t1329[1]), vaddq_u32(vsubq_u32(t1327[2], vdupq_n_u32(0)), t1330[0]), vaddq_u32(vsubq_u32(t1327[3], vdupq_n_u32(0)), t1330[1])];
            let t1332: [uint32x4_t; 4] = [vandq_u32(x176[0], t1331[0]), vandq_u32(x176[1], t1331[1]), vandq_u32(x176[2], t1331[2]), vandq_u32(x176[3], t1331[3])];
            t1319 = [vbslq_u32(t1321[0], t1332[0], t1319[0]), vbslq_u32(t1321[1], t1332[1], t1319[1]), vbslq_u32(t1321[0], t1332[2], t1319[2]), vbslq_u32(t1321[1], t1332[3], t1319[3])];
        }
        let t1333 = b27_l_p(l, lid0, t1288, t1319);
        let t1334: [uint32x4_t; 2] = [vaddq_u32(x144[0], t1333[0]), vaddq_u32(x144[1], t1333[1])];
        x144 = [vbslq_u32(t1288[0], t1334[0], x144[0]), vbslq_u32(t1288[1], t1334[1], x144[1])];
        let t1335: [uint32x4_t; 2] = [vandq_u32(t1288[0], live[0]), vandq_u32(t1288[1], live[1])];
        x152 = [vaddq_u32(x152[0], vandq_u32(t1335[0], vdupq_n_u32(8))), vaddq_u32(x152[1], vandq_u32(t1335[1], vdupq_n_u32(8)))];
    }
    x96 = x144;
    let t1336 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    let t1337 = [vdupq_n_u32((((t1336).wrapping_shr((((((4u32).wrapping_mul(x16)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32))); 2];
    x152 = t1337;
    let t1338 = [vdupq_n_u32(1u32); 2];
    let t1339: [uint32x4_t; 2] = [vsubq_u32(x80[0], t1338[0]), vsubq_u32(x80[1], t1338[1])];
    let t1340: [uint32x4_t; 2] = [vmulq_u32(t1272[0], t1339[0]), vmulq_u32(t1272[1], t1339[1])];
    let t1341: [uint32x4_t; 2] = [vaddq_u32(x56[0], t1340[0]), vaddq_u32(x56[1], t1340[1])];
    let t1342 = b26_ll_p(l, lid0, live, t1341, x64);
    let t1343 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t1342, 2u32);
    let t1344: [uint32x4_t; 2] = [t1343[0], t1343[1]];
    let t1345: [uint32x4_t; 4] = [t1344[0], t1344[1], vdupq_n_u32(0), vdupq_n_u32(0)];
    x160 = t1345;
    let t1346: [uint32x4_t; 2] = [x160[0], x160[1]];
    let t1347: [uint32x4_t; 2] = [vaddq_u32(x112[0], t1346[0]), vaddq_u32(x112[1], t1346[1])];
    let t1348: [uint32x4_t; 2] = [vmulq_u32(x96[0], x152[0]), vmulq_u32(x96[1], x152[1])];
    let t1349: [uint32x4_t; 2] = [vaddq_u32(t1347[0], t1348[0]), vaddq_u32(t1347[1], t1348[1])];
    let t1350 = b26_ll_p(l, lid0, live, t1349, x64);
    let t1351: [uint32x4_t; 4] = [t1350[0], t1350[1], vdupq_n_u32(0), vdupq_n_u32(0)];
    x168 = t1351;
    let t1352: [uint32x4_t; 2] = [x168[0], x168[1]];
    let t1353 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t1352, 8u32);
    let t1354 = b17_l_p(l, lid0, live, x152);
    let t1355: [uint32x4_t; 4] = [vandq_u32(t1353[0], t1354[0]), vandq_u32(t1353[1], t1354[1]), vandq_u32(t1353[2], t1354[2]), vandq_u32(t1353[3], t1354[3])];
    res = t1355;
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
    let t1356 = b20_r0r1s_p(l, lid0, live, arg0, arg1, (20u32).wrapping_add((4u32).wrapping_mul(x16)));
    res = t1356;
    return res;
}

#[inline(always)]
unsafe fn b21_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t1357 = b20_r0r1s_f(l, lid0, arg0, arg1, (20u32).wrapping_add((4u32).wrapping_mul(x16)));
    res = t1357;
    return res;
}

#[inline(always)]
unsafe fn b22_ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x8: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t1358: u32;
    if ((x0) < (x8)) {
        t1358 = x0;
    } else {
        t1358 = x8;
    }
    res = t1358;
    return res;
}

#[inline(always)]
unsafe fn b22_ss_f(l: &Launch, lid0: u32, x0: u32, x8: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t1359: u32;
    if ((x0) < (x8)) {
        t1359 = x0;
    } else {
        t1359 = x8;
    }
    res = t1359;
    return res;
}

#[inline(always)]
unsafe fn b22_ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x8: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1360 = [vdupq_n_u32(x8); 2];
    let t1361: [uint32x4_t; 2] = [vcltq_u32(x0[0], t1360[0]), vcltq_u32(x0[1], t1360[1])];
    let mut t1362: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1363: [uint32x4_t; 2] = [vandq_u32(live[0], t1361[0]), vandq_u32(live[1], t1361[1])];
    let t1364: [uint32x4_t; 2] = [vbicq_u32(live[0], t1361[0]), vbicq_u32(live[1], t1361[1])];
    {
        t1362 = [vbslq_u32(t1363[0], x0[0], t1362[0]), vbslq_u32(t1363[1], x0[1], t1362[1])];
    }
    {
        let t1365 = [vdupq_n_u32(x8); 2];
        t1362 = [vbslq_u32(t1364[0], t1365[0], t1362[0]), vbslq_u32(t1364[1], t1365[1], t1362[1])];
    }
    res = t1362;
    return res;
}

#[inline(always)]
unsafe fn b22_ls_f(l: &Launch, lid0: u32, x0: [uint32x4_t; 2], x8: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1366 = [vdupq_n_u32(x8); 2];
    let t1367: [uint32x4_t; 2] = [vcltq_u32(x0[0], t1366[0]), vcltq_u32(x0[1], t1366[1])];
    let mut t1368: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1369: [uint32x4_t; 2] = [vmvnq_u32(t1367[0]), vmvnq_u32(t1367[1])];
    {
        t1368 = [vbslq_u32(t1367[0], x0[0], t1368[0]), vbslq_u32(t1367[1], x0[1], t1368[1])];
    }
    {
        let t1370 = [vdupq_n_u32(x8); 2];
        t1368 = [vbslq_u32(t1369[0], t1370[0], t1368[0]), vbslq_u32(t1369[1], t1370[1], t1368[1])];
    }
    res = t1368;
    return res;
}

#[inline(always)]
unsafe fn b23_r0sns_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: u32, x16f: u32, x16s: u32, x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1371 = b28_r0an1s_p(l, lid0, live, arg0, ((((x8) as u64)) as usize as *const u8), x16f, x16s, x24);
    x32 = t1371;
    let t1372: [uint32x4_t; 4];
    if ((x24) >= (8u32)) {
        t1372 = x32;
    } else {
        let t1373 = b29_s_p(l, lid0, live, (8u32).wrapping_mul(x24));
        let t1374: u64 = t1373;
        let t1375 = [vdupq_n_u32((t1374) as u32); 2];
        let t1376 = [vdupq_n_u32(((t1374) >> 32) as u32); 2];
        let t1377: [uint32x4_t; 4] = [vandq_u32(x32[0], t1375[0]), vandq_u32(x32[1], t1375[1]), vandq_u32(x32[2], t1376[0]), vandq_u32(x32[3], t1376[1])];
        t1372 = t1377;
    }
    res = t1372;
    return res;
}

#[inline(always)]
unsafe fn b23_r0sns_f(l: &Launch, lid0: u32, arg0: *const u8, x8: u32, x16f: u32, x16s: u32, x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1378 = b28_r0an1s_f(l, lid0, arg0, ((((x8) as u64)) as usize as *const u8), x16f, x16s, x24);
    x32 = t1378;
    let t1379: [uint32x4_t; 4];
    if ((x24) >= (8u32)) {
        t1379 = x32;
    } else {
        let t1380 = b29_s_f(l, lid0, (8u32).wrapping_mul(x24));
        let t1381: u64 = t1380;
        let t1382 = [vdupq_n_u32((t1381) as u32); 2];
        let t1383 = [vdupq_n_u32(((t1381) >> 32) as u32); 2];
        let t1384: [uint32x4_t; 4] = [vandq_u32(x32[0], t1382[0]), vandq_u32(x32[1], t1382[1]), vandq_u32(x32[2], t1383[0]), vandq_u32(x32[3], t1383[1])];
        t1379 = t1384;
    }
    res = t1379;
    return res;
}

#[inline(always)]
unsafe fn b23_r0lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: [uint32x4_t; 2], x16: [uint32x4_t; 2], x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1385: [uint32x4_t; 2] = [vaddq_u32(x8[0], x16[0]), vaddq_u32(x8[1], x16[1])];
    let t1386: [uint32x4_t; 2] = [vcltq_u32(t1385[0], x8[0]), vcltq_u32(t1385[1], x8[1])];
    let t1387: [uint32x4_t; 4] = [t1385[0], t1385[1], vshrq_n_u32::<31>(t1386[0]), vshrq_n_u32::<31>(t1386[1])];
    let t1388 = b28_r0ls_p(l, lid0, live, arg0, t1387, x24);
    x32 = t1388;
    let t1389: [uint32x4_t; 4];
    if ((x24) >= (8u32)) {
        t1389 = x32;
    } else {
        let t1390 = b29_s_p(l, lid0, live, (8u32).wrapping_mul(x24));
        let t1391: u64 = t1390;
        let t1392 = [vdupq_n_u32((t1391) as u32); 2];
        let t1393 = [vdupq_n_u32(((t1391) >> 32) as u32); 2];
        let t1394: [uint32x4_t; 4] = [vandq_u32(x32[0], t1392[0]), vandq_u32(x32[1], t1392[1]), vandq_u32(x32[2], t1393[0]), vandq_u32(x32[3], t1393[1])];
        t1389 = t1394;
    }
    res = t1389;
    return res;
}

#[inline(always)]
unsafe fn b23_r0lls_f(l: &Launch, lid0: u32, arg0: *const u8, x8: [uint32x4_t; 2], x16: [uint32x4_t; 2], x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1395: [uint32x4_t; 2] = [vaddq_u32(x8[0], x16[0]), vaddq_u32(x8[1], x16[1])];
    let t1396: [uint32x4_t; 2] = [vcltq_u32(t1395[0], x8[0]), vcltq_u32(t1395[1], x8[1])];
    let t1397: [uint32x4_t; 4] = [t1395[0], t1395[1], vshrq_n_u32::<31>(t1396[0]), vshrq_n_u32::<31>(t1396[1])];
    let t1398 = b28_r0ls_f(l, lid0, arg0, t1397, x24);
    x32 = t1398;
    let t1399: [uint32x4_t; 4];
    if ((x24) >= (8u32)) {
        t1399 = x32;
    } else {
        let t1400 = b29_s_f(l, lid0, (8u32).wrapping_mul(x24));
        let t1401: u64 = t1400;
        let t1402 = [vdupq_n_u32((t1401) as u32); 2];
        let t1403 = [vdupq_n_u32(((t1401) >> 32) as u32); 2];
        let t1404: [uint32x4_t; 4] = [vandq_u32(x32[0], t1402[0]), vandq_u32(x32[1], t1402[1]), vandq_u32(x32[2], t1403[0]), vandq_u32(x32[3], t1403[1])];
        t1399 = t1404;
    }
    res = t1399;
    return res;
}

#[inline(always)]
unsafe fn b24_r0r1sss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: u32, x32: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t1405 = b30_r0sss_p(l, lid0, live, arg0, x16, x24, x32);
    res = t1405;
    return res;
}

#[inline(always)]
unsafe fn b24_r0r1sss_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32, x24: u32, x32: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t1406 = b30_r0sss_f(l, lid0, arg0, x16, x24, x32);
    res = t1406;
    return res;
}

#[inline(always)]
unsafe fn b24_r0r1sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2], x32: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1407 = b30_r0sls_p(l, lid0, live, arg0, x16, x24, x32);
    res = t1407;
    return res;
}

#[inline(always)]
unsafe fn b25_ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x8: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t1408: u32;
    if ((x0) < (x8)) {
        t1408 = x0;
    } else {
        t1408 = x8;
    }
    res = t1408;
    return res;
}

#[inline(always)]
unsafe fn b25_ss_f(l: &Launch, lid0: u32, x0: u32, x8: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t1409: u32;
    if ((x0) < (x8)) {
        t1409 = x0;
    } else {
        t1409 = x8;
    }
    res = t1409;
    return res;
}

#[inline(always)]
unsafe fn b25_sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x8: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1410 = [vdupq_n_u32(x0); 2];
    let t1411: [uint32x4_t; 2] = [vcltq_u32(t1410[0], x8[0]), vcltq_u32(t1410[1], x8[1])];
    let mut t1412: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1413: [uint32x4_t; 2] = [vandq_u32(live[0], t1411[0]), vandq_u32(live[1], t1411[1])];
    let t1414: [uint32x4_t; 2] = [vbicq_u32(live[0], t1411[0]), vbicq_u32(live[1], t1411[1])];
    {
        let t1415 = [vdupq_n_u32(x0); 2];
        t1412 = [vbslq_u32(t1413[0], t1415[0], t1412[0]), vbslq_u32(t1413[1], t1415[1], t1412[1])];
    }
    {
        t1412 = [vbslq_u32(t1414[0], x8[0], t1412[0]), vbslq_u32(t1414[1], x8[1], t1412[1])];
    }
    res = t1412;
    return res;
}

#[inline(always)]
unsafe fn b26_ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x8: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t1416: u32;
    if ((x0) < (x8)) {
        t1416 = x0;
    } else {
        t1416 = x8;
    }
    res = t1416;
    return res;
}

#[inline(always)]
unsafe fn b26_ss_f(l: &Launch, lid0: u32, x0: u32, x8: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t1417: u32;
    if ((x0) < (x8)) {
        t1417 = x0;
    } else {
        t1417 = x8;
    }
    res = t1417;
    return res;
}

#[inline(always)]
unsafe fn b26_ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x8: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1418 = [vdupq_n_u32(x8); 2];
    let t1419: [uint32x4_t; 2] = [vcltq_u32(x0[0], t1418[0]), vcltq_u32(x0[1], t1418[1])];
    let mut t1420: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1421: [uint32x4_t; 2] = [vandq_u32(live[0], t1419[0]), vandq_u32(live[1], t1419[1])];
    let t1422: [uint32x4_t; 2] = [vbicq_u32(live[0], t1419[0]), vbicq_u32(live[1], t1419[1])];
    {
        t1420 = [vbslq_u32(t1421[0], x0[0], t1420[0]), vbslq_u32(t1421[1], x0[1], t1420[1])];
    }
    {
        let t1423 = [vdupq_n_u32(x8); 2];
        t1420 = [vbslq_u32(t1422[0], t1423[0], t1420[0]), vbslq_u32(t1422[1], t1423[1], t1420[1])];
    }
    res = t1420;
    return res;
}

#[inline(always)]
unsafe fn b26_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x8: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1424: [uint32x4_t; 2] = [vcltq_u32(x0[0], x8[0]), vcltq_u32(x0[1], x8[1])];
    let mut t1425: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1426: [uint32x4_t; 2] = [vandq_u32(live[0], t1424[0]), vandq_u32(live[1], t1424[1])];
    let t1427: [uint32x4_t; 2] = [vbicq_u32(live[0], t1424[0]), vbicq_u32(live[1], t1424[1])];
    {
        t1425 = [vbslq_u32(t1426[0], x0[0], t1425[0]), vbslq_u32(t1426[1], x0[1], t1425[1])];
    }
    {
        t1425 = [vbslq_u32(t1427[0], x8[0], t1425[0]), vbslq_u32(t1427[1], x8[1], t1425[1])];
    }
    res = t1425;
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
    let t1428: [uint32x4_t; 2] = [vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[0])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[2]))))), vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[1])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[3])))))];
    res = t1428;
    return res;
}

#[inline(always)]
unsafe fn b28_r0ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: u64, x16: u32) -> u64 {
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
    let mut t1429: bool = ((x32) != (0u32));
    if t1429 {
        t1429 = (((x32).wrapping_add((x16).wrapping_mul(8u32))) > (64u32));
    }
    let t1430: u64;
    if t1429 {
        t1430 = ((x40) | (((x24.wrapping_add((1u64).wrapping_mul(8u64) as usize) as *const u64).read_unaligned()).wrapping_shl(((((64u32).wrapping_sub(x32)) as u64)) as u32)));
    } else {
        t1430 = x40;
    }
    res = t1430;
    return res;
}

#[inline(always)]
unsafe fn b28_r0ss_f(l: &Launch, lid0: u32, arg0: *const u8, x8: u64, x16: u32) -> u64 {
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
    let mut t1431: bool = ((x32) != (0u32));
    if t1431 {
        t1431 = (((x32).wrapping_add((x16).wrapping_mul(8u32))) > (64u32));
    }
    let t1432: u64;
    if t1431 {
        t1432 = ((x40) | (((x24.wrapping_add((1u64).wrapping_mul(8u64) as usize) as *const u64).read_unaligned()).wrapping_shl(((((64u32).wrapping_sub(x32)) as u64)) as u32)));
    } else {
        t1432 = x40;
    }
    res = t1432;
    return res;
}

#[inline(always)]
unsafe fn b28_r0ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: [uint32x4_t; 4], x16: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if ((x16) == (8u32)) {
        let t1433: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
        let t1434: [uint32x4_t; 4] = [vandq_u32(x8[0], t1433[0]), vandq_u32(x8[1], t1433[1]), vandq_u32(x8[2], t1433[2]), vandq_u32(x8[3], t1433[3])];
        let t1435: u64 = ((arg0) as usize as u64);
        let t1436 = [vdupq_n_u32((t1435) as u32); 2];
        let t1437 = [vdupq_n_u32(((t1435) >> 32) as u32); 2];
        let t1438: [uint32x4_t; 2] = [vaddq_u32(t1436[0], t1434[0]), vaddq_u32(t1436[1], t1434[1])];
        let t1439: [uint32x4_t; 2] = [vcltq_u32(t1438[0], t1436[0]), vcltq_u32(t1438[1], t1436[1])];
        let t1440: [uint32x4_t; 4] = [t1438[0], t1438[1], vsubq_u32(vaddq_u32(t1437[0], t1434[2]), t1439[0]), vsubq_u32(vaddq_u32(t1437[1], t1434[3]), t1439[1])];
        let mut t1441: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1440[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1440[0]) as u64) as usize as *const u64).read_unaligned(); t1441[0] = vsetq_lane_u32::<0>(v as u32, t1441[0]); t1441[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1441[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1440[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1440[0]) as u64) as usize as *const u64).read_unaligned(); t1441[0] = vsetq_lane_u32::<1>(v as u32, t1441[0]); t1441[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1441[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1440[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1440[0]) as u64) as usize as *const u64).read_unaligned(); t1441[0] = vsetq_lane_u32::<2>(v as u32, t1441[0]); t1441[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1441[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1440[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1440[0]) as u64) as usize as *const u64).read_unaligned(); t1441[0] = vsetq_lane_u32::<3>(v as u32, t1441[0]); t1441[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1441[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1440[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1440[1]) as u64) as usize as *const u64).read_unaligned(); t1441[1] = vsetq_lane_u32::<0>(v as u32, t1441[1]); t1441[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1441[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1440[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1440[1]) as u64) as usize as *const u64).read_unaligned(); t1441[1] = vsetq_lane_u32::<1>(v as u32, t1441[1]); t1441[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1441[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1440[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1440[1]) as u64) as usize as *const u64).read_unaligned(); t1441[1] = vsetq_lane_u32::<2>(v as u32, t1441[1]); t1441[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1441[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1440[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1440[1]) as u64) as usize as *const u64).read_unaligned(); t1441[1] = vsetq_lane_u32::<3>(v as u32, t1441[1]); t1441[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1441[3]); }
        res = t1441;
        return res;
    }
    let t1442: u64 = !(7u64);
    let t1443 = [vdupq_n_u32((t1442) as u32); 2];
    let t1444 = [vdupq_n_u32(((t1442) >> 32) as u32); 2];
    let t1445: [uint32x4_t; 4] = [vandq_u32(x8[0], t1443[0]), vandq_u32(x8[1], t1443[1]), vandq_u32(x8[2], t1444[0]), vandq_u32(x8[3], t1444[1])];
    let t1446: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
    let t1447: [uint32x4_t; 4] = [vandq_u32(t1445[0], t1446[0]), vandq_u32(t1445[1], t1446[1]), vandq_u32(t1445[2], t1446[2]), vandq_u32(t1445[3], t1446[3])];
    let t1448: u64 = ((arg0) as usize as u64);
    let t1449 = [vdupq_n_u32((t1448) as u32); 2];
    let t1450 = [vdupq_n_u32(((t1448) >> 32) as u32); 2];
    let t1451: [uint32x4_t; 2] = [vaddq_u32(t1449[0], t1447[0]), vaddq_u32(t1449[1], t1447[1])];
    let t1452: [uint32x4_t; 2] = [vcltq_u32(t1451[0], t1449[0]), vcltq_u32(t1451[1], t1449[1])];
    let t1453: [uint32x4_t; 4] = [t1451[0], t1451[1], vsubq_u32(vaddq_u32(t1450[0], t1447[2]), t1452[0]), vsubq_u32(vaddq_u32(t1450[1], t1447[3]), t1452[1])];
    x24 = t1453;
    let t1454: u64 = 7u64;
    let t1455 = [vdupq_n_u32((t1454) as u32); 2];
    let t1456: [uint32x4_t; 2] = [vandq_u32(x8[0], t1455[0]), vandq_u32(x8[1], t1455[1])];
    let t1457 = [vdupq_n_u32(8u32); 2];
    let t1458: [uint32x4_t; 2] = [vmulq_u32(t1456[0], t1457[0]), vmulq_u32(t1456[1], t1457[1])];
    x32 = t1458;
    let mut t1459: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x24[2]) as u64) << 32 | vgetq_lane_u32::<0>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1459[0] = vsetq_lane_u32::<0>(v as u32, t1459[0]); t1459[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1459[2]); }
    { let v = (((vgetq_lane_u32::<1>(x24[2]) as u64) << 32 | vgetq_lane_u32::<1>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1459[0] = vsetq_lane_u32::<1>(v as u32, t1459[0]); t1459[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1459[2]); }
    { let v = (((vgetq_lane_u32::<2>(x24[2]) as u64) << 32 | vgetq_lane_u32::<2>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1459[0] = vsetq_lane_u32::<2>(v as u32, t1459[0]); t1459[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1459[2]); }
    { let v = (((vgetq_lane_u32::<3>(x24[2]) as u64) << 32 | vgetq_lane_u32::<3>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1459[0] = vsetq_lane_u32::<3>(v as u32, t1459[0]); t1459[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1459[2]); }
    { let v = (((vgetq_lane_u32::<0>(x24[3]) as u64) << 32 | vgetq_lane_u32::<0>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1459[1] = vsetq_lane_u32::<0>(v as u32, t1459[1]); t1459[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1459[3]); }
    { let v = (((vgetq_lane_u32::<1>(x24[3]) as u64) << 32 | vgetq_lane_u32::<1>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1459[1] = vsetq_lane_u32::<1>(v as u32, t1459[1]); t1459[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1459[3]); }
    { let v = (((vgetq_lane_u32::<2>(x24[3]) as u64) << 32 | vgetq_lane_u32::<2>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1459[1] = vsetq_lane_u32::<2>(v as u32, t1459[1]); t1459[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1459[3]); }
    { let v = (((vgetq_lane_u32::<3>(x24[3]) as u64) << 32 | vgetq_lane_u32::<3>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1459[1] = vsetq_lane_u32::<3>(v as u32, t1459[1]); t1459[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1459[3]); }
    let t1460: [uint32x4_t; 2] = [x32[0], x32[1]];
    let t1461: [uint32x4_t; 2] = [vandq_u32(t1460[0], vdupq_n_u32(63)), vandq_u32(t1460[1], vdupq_n_u32(63))];
    let t1462: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t1459[0], vnegq_s32(vreinterpretq_s32_u32(t1461[0]))), vshlq_u32(t1459[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1461[0])))), vorrq_u32(vshlq_u32(t1459[1], vnegq_s32(vreinterpretq_s32_u32(t1461[1]))), vshlq_u32(t1459[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1461[1])))), vshlq_u32(t1459[2], vnegq_s32(vreinterpretq_s32_u32(t1461[0]))), vshlq_u32(t1459[3], vnegq_s32(vreinterpretq_s32_u32(t1461[1])))];
    x40 = t1462;
    let t1463 = [vdupq_n_u32(0u32); 2];
    let t1464: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t1463[0])), vmvnq_u32(vceqq_u32(x32[1], t1463[1]))];
    let t1465 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t1466: [uint32x4_t; 2] = [vaddq_u32(x32[0], t1465[0]), vaddq_u32(x32[1], t1465[1])];
    let t1467 = [vdupq_n_u32(64u32); 2];
    let t1468: [uint32x4_t; 2] = [vcgtq_u32(t1466[0], t1467[0]), vcgtq_u32(t1466[1], t1467[1])];
    let t1469: [uint32x4_t; 2] = [vandq_u32(t1464[0], t1468[0]), vandq_u32(t1464[1], t1468[1])];
    let mut t1470: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1471: [uint32x4_t; 2] = [vandq_u32(live[0], t1469[0]), vandq_u32(live[1], t1469[1])];
    let t1472: [uint32x4_t; 2] = [vbicq_u32(live[0], t1469[0]), vbicq_u32(live[1], t1469[1])];
    if any(&t1471) {
        let t1473: u64 = (1u64).wrapping_mul(8u64);
        let t1474 = [vdupq_n_u32((t1473) as u32); 2];
        let t1475 = [vdupq_n_u32(((t1473) >> 32) as u32); 2];
        let t1476: [uint32x4_t; 2] = [vaddq_u32(x24[0], t1474[0]), vaddq_u32(x24[1], t1474[1])];
        let t1477: [uint32x4_t; 2] = [vcltq_u32(t1476[0], x24[0]), vcltq_u32(t1476[1], x24[1])];
        let t1478: [uint32x4_t; 4] = [t1476[0], t1476[1], vsubq_u32(vaddq_u32(x24[2], t1475[0]), t1477[0]), vsubq_u32(vaddq_u32(x24[3], t1475[1]), t1477[1])];
        let mut t1479: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1478[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1478[0]) as u64) as usize as *const u64).read_unaligned(); t1479[0] = vsetq_lane_u32::<0>(v as u32, t1479[0]); t1479[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1479[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1478[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1478[0]) as u64) as usize as *const u64).read_unaligned(); t1479[0] = vsetq_lane_u32::<1>(v as u32, t1479[0]); t1479[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1479[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1478[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1478[0]) as u64) as usize as *const u64).read_unaligned(); t1479[0] = vsetq_lane_u32::<2>(v as u32, t1479[0]); t1479[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1479[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1478[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1478[0]) as u64) as usize as *const u64).read_unaligned(); t1479[0] = vsetq_lane_u32::<3>(v as u32, t1479[0]); t1479[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1479[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1478[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1478[1]) as u64) as usize as *const u64).read_unaligned(); t1479[1] = vsetq_lane_u32::<0>(v as u32, t1479[1]); t1479[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1479[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1478[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1478[1]) as u64) as usize as *const u64).read_unaligned(); t1479[1] = vsetq_lane_u32::<1>(v as u32, t1479[1]); t1479[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1479[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1478[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1478[1]) as u64) as usize as *const u64).read_unaligned(); t1479[1] = vsetq_lane_u32::<2>(v as u32, t1479[1]); t1479[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1479[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1478[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1478[1]) as u64) as usize as *const u64).read_unaligned(); t1479[1] = vsetq_lane_u32::<3>(v as u32, t1479[1]); t1479[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1479[3]); }
        let t1480: [uint32x4_t; 2] = [vsubq_u32(t1467[0], x32[0]), vsubq_u32(t1467[1], x32[1])];
        let t1481: [uint32x4_t; 2] = [t1480[0], t1480[1]];
        let t1482: [uint32x4_t; 2] = [vandq_u32(t1481[0], vdupq_n_u32(63)), vandq_u32(t1481[1], vdupq_n_u32(63))];
        let t1483: [uint32x4_t; 4] = [vshlq_u32(t1479[0], vreinterpretq_s32_u32(t1482[0])), vshlq_u32(t1479[1], vreinterpretq_s32_u32(t1482[1])), vorrq_u32(vshlq_u32(t1479[2], vreinterpretq_s32_u32(t1482[0])), vshlq_u32(t1479[0], vsubq_s32(vreinterpretq_s32_u32(t1482[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1479[3], vreinterpretq_s32_u32(t1482[1])), vshlq_u32(t1479[1], vsubq_s32(vreinterpretq_s32_u32(t1482[1]), vdupq_n_s32(32))))];
        let t1484: [uint32x4_t; 4] = [vorrq_u32(x40[0], t1483[0]), vorrq_u32(x40[1], t1483[1]), vorrq_u32(x40[2], t1483[2]), vorrq_u32(x40[3], t1483[3])];
        t1470 = [vbslq_u32(t1471[0], t1484[0], t1470[0]), vbslq_u32(t1471[1], t1484[1], t1470[1]), vbslq_u32(t1471[0], t1484[2], t1470[2]), vbslq_u32(t1471[1], t1484[3], t1470[3])];
    }
    {
        t1470 = [vbslq_u32(t1472[0], x40[0], t1470[0]), vbslq_u32(t1472[1], x40[1], t1470[1]), vbslq_u32(t1472[0], x40[2], t1470[2]), vbslq_u32(t1472[1], x40[3], t1470[3])];
    }
    res = t1470;
    return res;
}

#[inline(always)]
unsafe fn b28_r0ls_f(l: &Launch, lid0: u32, arg0: *const u8, x8: [uint32x4_t; 4], x16: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if ((x16) == (8u32)) {
        let t1485: u64 = ((arg0) as usize as u64);
        let t1486 = [vdupq_n_u32((t1485) as u32); 2];
        let t1487 = [vdupq_n_u32(((t1485) >> 32) as u32); 2];
        let t1488: [uint32x4_t; 2] = [vaddq_u32(t1486[0], x8[0]), vaddq_u32(t1486[1], x8[1])];
        let t1489: [uint32x4_t; 2] = [vcltq_u32(t1488[0], t1486[0]), vcltq_u32(t1488[1], t1486[1])];
        let t1490: [uint32x4_t; 4] = [t1488[0], t1488[1], vsubq_u32(vaddq_u32(t1487[0], x8[2]), t1489[0]), vsubq_u32(vaddq_u32(t1487[1], x8[3]), t1489[1])];
        let mut t1491: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1490[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1490[0]) as u64) as usize as *const u64).read_unaligned(); t1491[0] = vsetq_lane_u32::<0>(v as u32, t1491[0]); t1491[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1491[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1490[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1490[0]) as u64) as usize as *const u64).read_unaligned(); t1491[0] = vsetq_lane_u32::<1>(v as u32, t1491[0]); t1491[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1491[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1490[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1490[0]) as u64) as usize as *const u64).read_unaligned(); t1491[0] = vsetq_lane_u32::<2>(v as u32, t1491[0]); t1491[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1491[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1490[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1490[0]) as u64) as usize as *const u64).read_unaligned(); t1491[0] = vsetq_lane_u32::<3>(v as u32, t1491[0]); t1491[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1491[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1490[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1490[1]) as u64) as usize as *const u64).read_unaligned(); t1491[1] = vsetq_lane_u32::<0>(v as u32, t1491[1]); t1491[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1491[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1490[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1490[1]) as u64) as usize as *const u64).read_unaligned(); t1491[1] = vsetq_lane_u32::<1>(v as u32, t1491[1]); t1491[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1491[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1490[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1490[1]) as u64) as usize as *const u64).read_unaligned(); t1491[1] = vsetq_lane_u32::<2>(v as u32, t1491[1]); t1491[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1491[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1490[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1490[1]) as u64) as usize as *const u64).read_unaligned(); t1491[1] = vsetq_lane_u32::<3>(v as u32, t1491[1]); t1491[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1491[3]); }
        res = t1491;
        return res;
    }
    let t1492: u64 = !(7u64);
    let t1493 = [vdupq_n_u32((t1492) as u32); 2];
    let t1494 = [vdupq_n_u32(((t1492) >> 32) as u32); 2];
    let t1495: [uint32x4_t; 4] = [vandq_u32(x8[0], t1493[0]), vandq_u32(x8[1], t1493[1]), vandq_u32(x8[2], t1494[0]), vandq_u32(x8[3], t1494[1])];
    let t1496: u64 = ((arg0) as usize as u64);
    let t1497 = [vdupq_n_u32((t1496) as u32); 2];
    let t1498 = [vdupq_n_u32(((t1496) >> 32) as u32); 2];
    let t1499: [uint32x4_t; 2] = [vaddq_u32(t1497[0], t1495[0]), vaddq_u32(t1497[1], t1495[1])];
    let t1500: [uint32x4_t; 2] = [vcltq_u32(t1499[0], t1497[0]), vcltq_u32(t1499[1], t1497[1])];
    let t1501: [uint32x4_t; 4] = [t1499[0], t1499[1], vsubq_u32(vaddq_u32(t1498[0], t1495[2]), t1500[0]), vsubq_u32(vaddq_u32(t1498[1], t1495[3]), t1500[1])];
    x24 = t1501;
    let t1502: u64 = 7u64;
    let t1503 = [vdupq_n_u32((t1502) as u32); 2];
    let t1504: [uint32x4_t; 2] = [vandq_u32(x8[0], t1503[0]), vandq_u32(x8[1], t1503[1])];
    let t1505 = [vdupq_n_u32(8u32); 2];
    let t1506: [uint32x4_t; 2] = [vmulq_u32(t1504[0], t1505[0]), vmulq_u32(t1504[1], t1505[1])];
    x32 = t1506;
    let mut t1507: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x24[2]) as u64) << 32 | vgetq_lane_u32::<0>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1507[0] = vsetq_lane_u32::<0>(v as u32, t1507[0]); t1507[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1507[2]); }
    { let v = (((vgetq_lane_u32::<1>(x24[2]) as u64) << 32 | vgetq_lane_u32::<1>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1507[0] = vsetq_lane_u32::<1>(v as u32, t1507[0]); t1507[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1507[2]); }
    { let v = (((vgetq_lane_u32::<2>(x24[2]) as u64) << 32 | vgetq_lane_u32::<2>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1507[0] = vsetq_lane_u32::<2>(v as u32, t1507[0]); t1507[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1507[2]); }
    { let v = (((vgetq_lane_u32::<3>(x24[2]) as u64) << 32 | vgetq_lane_u32::<3>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1507[0] = vsetq_lane_u32::<3>(v as u32, t1507[0]); t1507[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1507[2]); }
    { let v = (((vgetq_lane_u32::<0>(x24[3]) as u64) << 32 | vgetq_lane_u32::<0>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1507[1] = vsetq_lane_u32::<0>(v as u32, t1507[1]); t1507[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1507[3]); }
    { let v = (((vgetq_lane_u32::<1>(x24[3]) as u64) << 32 | vgetq_lane_u32::<1>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1507[1] = vsetq_lane_u32::<1>(v as u32, t1507[1]); t1507[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1507[3]); }
    { let v = (((vgetq_lane_u32::<2>(x24[3]) as u64) << 32 | vgetq_lane_u32::<2>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1507[1] = vsetq_lane_u32::<2>(v as u32, t1507[1]); t1507[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1507[3]); }
    { let v = (((vgetq_lane_u32::<3>(x24[3]) as u64) << 32 | vgetq_lane_u32::<3>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1507[1] = vsetq_lane_u32::<3>(v as u32, t1507[1]); t1507[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1507[3]); }
    let t1508: [uint32x4_t; 2] = [x32[0], x32[1]];
    let t1509: [uint32x4_t; 2] = [vandq_u32(t1508[0], vdupq_n_u32(63)), vandq_u32(t1508[1], vdupq_n_u32(63))];
    let t1510: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t1507[0], vnegq_s32(vreinterpretq_s32_u32(t1509[0]))), vshlq_u32(t1507[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1509[0])))), vorrq_u32(vshlq_u32(t1507[1], vnegq_s32(vreinterpretq_s32_u32(t1509[1]))), vshlq_u32(t1507[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1509[1])))), vshlq_u32(t1507[2], vnegq_s32(vreinterpretq_s32_u32(t1509[0]))), vshlq_u32(t1507[3], vnegq_s32(vreinterpretq_s32_u32(t1509[1])))];
    x40 = t1510;
    let t1511 = [vdupq_n_u32(0u32); 2];
    let t1512: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t1511[0])), vmvnq_u32(vceqq_u32(x32[1], t1511[1]))];
    let t1513 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t1514: [uint32x4_t; 2] = [vaddq_u32(x32[0], t1513[0]), vaddq_u32(x32[1], t1513[1])];
    let t1515 = [vdupq_n_u32(64u32); 2];
    let t1516: [uint32x4_t; 2] = [vcgtq_u32(t1514[0], t1515[0]), vcgtq_u32(t1514[1], t1515[1])];
    let t1517: [uint32x4_t; 2] = [vandq_u32(t1512[0], t1516[0]), vandq_u32(t1512[1], t1516[1])];
    let mut t1518: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1519: [uint32x4_t; 2] = [vmvnq_u32(t1517[0]), vmvnq_u32(t1517[1])];
    if any(&t1517) {
        let t1520: u64 = (1u64).wrapping_mul(8u64);
        let t1521 = [vdupq_n_u32((t1520) as u32); 2];
        let t1522 = [vdupq_n_u32(((t1520) >> 32) as u32); 2];
        let t1523: [uint32x4_t; 2] = [vaddq_u32(x24[0], t1521[0]), vaddq_u32(x24[1], t1521[1])];
        let t1524: [uint32x4_t; 2] = [vcltq_u32(t1523[0], x24[0]), vcltq_u32(t1523[1], x24[1])];
        let t1525: [uint32x4_t; 4] = [t1523[0], t1523[1], vsubq_u32(vaddq_u32(x24[2], t1522[0]), t1524[0]), vsubq_u32(vaddq_u32(x24[3], t1522[1]), t1524[1])];
        let mut t1526: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1525[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1525[0]) as u64) as usize as *const u64).read_unaligned(); t1526[0] = vsetq_lane_u32::<0>(v as u32, t1526[0]); t1526[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1526[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1525[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1525[0]) as u64) as usize as *const u64).read_unaligned(); t1526[0] = vsetq_lane_u32::<1>(v as u32, t1526[0]); t1526[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1526[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1525[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1525[0]) as u64) as usize as *const u64).read_unaligned(); t1526[0] = vsetq_lane_u32::<2>(v as u32, t1526[0]); t1526[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1526[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1525[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1525[0]) as u64) as usize as *const u64).read_unaligned(); t1526[0] = vsetq_lane_u32::<3>(v as u32, t1526[0]); t1526[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1526[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1525[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1525[1]) as u64) as usize as *const u64).read_unaligned(); t1526[1] = vsetq_lane_u32::<0>(v as u32, t1526[1]); t1526[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1526[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1525[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1525[1]) as u64) as usize as *const u64).read_unaligned(); t1526[1] = vsetq_lane_u32::<1>(v as u32, t1526[1]); t1526[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1526[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1525[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1525[1]) as u64) as usize as *const u64).read_unaligned(); t1526[1] = vsetq_lane_u32::<2>(v as u32, t1526[1]); t1526[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1526[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1525[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1525[1]) as u64) as usize as *const u64).read_unaligned(); t1526[1] = vsetq_lane_u32::<3>(v as u32, t1526[1]); t1526[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1526[3]); }
        let t1527: [uint32x4_t; 2] = [vsubq_u32(t1515[0], x32[0]), vsubq_u32(t1515[1], x32[1])];
        let t1528: [uint32x4_t; 2] = [t1527[0], t1527[1]];
        let t1529: [uint32x4_t; 2] = [vandq_u32(t1528[0], vdupq_n_u32(63)), vandq_u32(t1528[1], vdupq_n_u32(63))];
        let t1530: [uint32x4_t; 4] = [vshlq_u32(t1526[0], vreinterpretq_s32_u32(t1529[0])), vshlq_u32(t1526[1], vreinterpretq_s32_u32(t1529[1])), vorrq_u32(vshlq_u32(t1526[2], vreinterpretq_s32_u32(t1529[0])), vshlq_u32(t1526[0], vsubq_s32(vreinterpretq_s32_u32(t1529[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1526[3], vreinterpretq_s32_u32(t1529[1])), vshlq_u32(t1526[1], vsubq_s32(vreinterpretq_s32_u32(t1529[1]), vdupq_n_s32(32))))];
        let t1531: [uint32x4_t; 4] = [vorrq_u32(x40[0], t1530[0]), vorrq_u32(x40[1], t1530[1]), vorrq_u32(x40[2], t1530[2]), vorrq_u32(x40[3], t1530[3])];
        t1518 = [vbslq_u32(t1517[0], t1531[0], t1518[0]), vbslq_u32(t1517[1], t1531[1], t1518[1]), vbslq_u32(t1517[0], t1531[2], t1518[2]), vbslq_u32(t1517[1], t1531[3], t1518[3])];
    }
    {
        t1518 = [vbslq_u32(t1519[0], x40[0], t1518[0]), vbslq_u32(t1519[1], x40[1], t1518[1]), vbslq_u32(t1519[0], x40[2], t1518[2]), vbslq_u32(t1519[1], x40[3], t1518[3])];
    }
    res = t1518;
    return res;
}

#[inline(always)]
unsafe fn b28_r0al1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8b: *const u8, x8i: [uint32x4_t; 2], x16: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if ((x16) == (8u32)) {
        let t1532 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], x8i[0]), vandq_u32(live[1], x8i[1]))));
        let t1533: [uint32x4_t; 2] = [vbslq_u32(live[0], x8i[0], t1532), vbslq_u32(live[1], x8i[1], t1532)];
        let mut t1534: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1533[0]) as usize * 1) as *const u64).read_unaligned(); t1534[0] = vsetq_lane_u32::<0>(v as u32, t1534[0]); t1534[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1534[2]); }
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1533[0]) as usize * 1) as *const u64).read_unaligned(); t1534[0] = vsetq_lane_u32::<1>(v as u32, t1534[0]); t1534[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1534[2]); }
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1533[0]) as usize * 1) as *const u64).read_unaligned(); t1534[0] = vsetq_lane_u32::<2>(v as u32, t1534[0]); t1534[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1534[2]); }
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1533[0]) as usize * 1) as *const u64).read_unaligned(); t1534[0] = vsetq_lane_u32::<3>(v as u32, t1534[0]); t1534[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1534[2]); }
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1533[1]) as usize * 1) as *const u64).read_unaligned(); t1534[1] = vsetq_lane_u32::<0>(v as u32, t1534[1]); t1534[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1534[3]); }
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1533[1]) as usize * 1) as *const u64).read_unaligned(); t1534[1] = vsetq_lane_u32::<1>(v as u32, t1534[1]); t1534[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1534[3]); }
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1533[1]) as usize * 1) as *const u64).read_unaligned(); t1534[1] = vsetq_lane_u32::<2>(v as u32, t1534[1]); t1534[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1534[3]); }
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1533[1]) as usize * 1) as *const u64).read_unaligned(); t1534[1] = vsetq_lane_u32::<3>(v as u32, t1534[1]); t1534[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1534[3]); }
        res = t1534;
        return res;
    }
    let t1535: [uint32x4_t; 2] = [x8i[0], x8i[1]];
    let t1536: u64 = ((x8b) as usize as u64);
    let t1537 = [vdupq_n_u32((t1536) as u32); 2];
    let t1538 = [vdupq_n_u32(((t1536) >> 32) as u32); 2];
    let t1539: [uint32x4_t; 2] = [vaddq_u32(t1537[0], t1535[0]), vaddq_u32(t1537[1], t1535[1])];
    let t1540: [uint32x4_t; 2] = [vcltq_u32(t1539[0], t1537[0]), vcltq_u32(t1539[1], t1537[1])];
    let t1541: [uint32x4_t; 4] = [t1539[0], t1539[1], vsubq_u32(vaddq_u32(t1538[0], vdupq_n_u32(0)), t1540[0]), vsubq_u32(vaddq_u32(t1538[1], vdupq_n_u32(0)), t1540[1])];
    let t1542: u64 = !(7u64);
    let t1543 = [vdupq_n_u32((t1542) as u32); 2];
    let t1544 = [vdupq_n_u32(((t1542) >> 32) as u32); 2];
    let t1545: [uint32x4_t; 4] = [vandq_u32(t1541[0], t1543[0]), vandq_u32(t1541[1], t1543[1]), vandq_u32(t1541[2], t1544[0]), vandq_u32(t1541[3], t1544[1])];
    let t1546: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
    let t1547: [uint32x4_t; 4] = [vandq_u32(t1545[0], t1546[0]), vandq_u32(t1545[1], t1546[1]), vandq_u32(t1545[2], t1546[2]), vandq_u32(t1545[3], t1546[3])];
    let t1548: u64 = ((arg0) as usize as u64);
    let t1549 = [vdupq_n_u32((t1548) as u32); 2];
    let t1550 = [vdupq_n_u32(((t1548) >> 32) as u32); 2];
    let t1551: [uint32x4_t; 2] = [vaddq_u32(t1549[0], t1547[0]), vaddq_u32(t1549[1], t1547[1])];
    let t1552: [uint32x4_t; 2] = [vcltq_u32(t1551[0], t1549[0]), vcltq_u32(t1551[1], t1549[1])];
    let t1553: [uint32x4_t; 4] = [t1551[0], t1551[1], vsubq_u32(vaddq_u32(t1550[0], t1547[2]), t1552[0]), vsubq_u32(vaddq_u32(t1550[1], t1547[3]), t1552[1])];
    x24 = t1553;
    let t1554: [uint32x4_t; 2] = [x8i[0], x8i[1]];
    let t1555: u64 = ((x8b) as usize as u64);
    let t1556 = [vdupq_n_u32((t1555) as u32); 2];
    let t1557 = [vdupq_n_u32(((t1555) >> 32) as u32); 2];
    let t1558: [uint32x4_t; 2] = [vaddq_u32(t1556[0], t1554[0]), vaddq_u32(t1556[1], t1554[1])];
    let t1559: [uint32x4_t; 2] = [vcltq_u32(t1558[0], t1556[0]), vcltq_u32(t1558[1], t1556[1])];
    let t1560: [uint32x4_t; 4] = [t1558[0], t1558[1], vsubq_u32(vaddq_u32(t1557[0], vdupq_n_u32(0)), t1559[0]), vsubq_u32(vaddq_u32(t1557[1], vdupq_n_u32(0)), t1559[1])];
    let t1561: u64 = 7u64;
    let t1562 = [vdupq_n_u32((t1561) as u32); 2];
    let t1563: [uint32x4_t; 2] = [vandq_u32(t1560[0], t1562[0]), vandq_u32(t1560[1], t1562[1])];
    let t1564 = [vdupq_n_u32(8u32); 2];
    let t1565: [uint32x4_t; 2] = [vmulq_u32(t1563[0], t1564[0]), vmulq_u32(t1563[1], t1564[1])];
    x32 = t1565;
    let mut t1566: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x24[2]) as u64) << 32 | vgetq_lane_u32::<0>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1566[0] = vsetq_lane_u32::<0>(v as u32, t1566[0]); t1566[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1566[2]); }
    { let v = (((vgetq_lane_u32::<1>(x24[2]) as u64) << 32 | vgetq_lane_u32::<1>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1566[0] = vsetq_lane_u32::<1>(v as u32, t1566[0]); t1566[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1566[2]); }
    { let v = (((vgetq_lane_u32::<2>(x24[2]) as u64) << 32 | vgetq_lane_u32::<2>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1566[0] = vsetq_lane_u32::<2>(v as u32, t1566[0]); t1566[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1566[2]); }
    { let v = (((vgetq_lane_u32::<3>(x24[2]) as u64) << 32 | vgetq_lane_u32::<3>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1566[0] = vsetq_lane_u32::<3>(v as u32, t1566[0]); t1566[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1566[2]); }
    { let v = (((vgetq_lane_u32::<0>(x24[3]) as u64) << 32 | vgetq_lane_u32::<0>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1566[1] = vsetq_lane_u32::<0>(v as u32, t1566[1]); t1566[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1566[3]); }
    { let v = (((vgetq_lane_u32::<1>(x24[3]) as u64) << 32 | vgetq_lane_u32::<1>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1566[1] = vsetq_lane_u32::<1>(v as u32, t1566[1]); t1566[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1566[3]); }
    { let v = (((vgetq_lane_u32::<2>(x24[3]) as u64) << 32 | vgetq_lane_u32::<2>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1566[1] = vsetq_lane_u32::<2>(v as u32, t1566[1]); t1566[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1566[3]); }
    { let v = (((vgetq_lane_u32::<3>(x24[3]) as u64) << 32 | vgetq_lane_u32::<3>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1566[1] = vsetq_lane_u32::<3>(v as u32, t1566[1]); t1566[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1566[3]); }
    let t1567: [uint32x4_t; 2] = [x32[0], x32[1]];
    let t1568: [uint32x4_t; 2] = [vandq_u32(t1567[0], vdupq_n_u32(63)), vandq_u32(t1567[1], vdupq_n_u32(63))];
    let t1569: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t1566[0], vnegq_s32(vreinterpretq_s32_u32(t1568[0]))), vshlq_u32(t1566[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1568[0])))), vorrq_u32(vshlq_u32(t1566[1], vnegq_s32(vreinterpretq_s32_u32(t1568[1]))), vshlq_u32(t1566[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1568[1])))), vshlq_u32(t1566[2], vnegq_s32(vreinterpretq_s32_u32(t1568[0]))), vshlq_u32(t1566[3], vnegq_s32(vreinterpretq_s32_u32(t1568[1])))];
    x40 = t1569;
    let t1570 = [vdupq_n_u32(0u32); 2];
    let t1571: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t1570[0])), vmvnq_u32(vceqq_u32(x32[1], t1570[1]))];
    let t1572 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t1573: [uint32x4_t; 2] = [vaddq_u32(x32[0], t1572[0]), vaddq_u32(x32[1], t1572[1])];
    let t1574 = [vdupq_n_u32(64u32); 2];
    let t1575: [uint32x4_t; 2] = [vcgtq_u32(t1573[0], t1574[0]), vcgtq_u32(t1573[1], t1574[1])];
    let t1576: [uint32x4_t; 2] = [vandq_u32(t1571[0], t1575[0]), vandq_u32(t1571[1], t1575[1])];
    let mut t1577: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1578: [uint32x4_t; 2] = [vandq_u32(live[0], t1576[0]), vandq_u32(live[1], t1576[1])];
    let t1579: [uint32x4_t; 2] = [vbicq_u32(live[0], t1576[0]), vbicq_u32(live[1], t1576[1])];
    if any(&t1578) {
        let t1580: u64 = (1u64).wrapping_mul(8u64);
        let t1581 = [vdupq_n_u32((t1580) as u32); 2];
        let t1582 = [vdupq_n_u32(((t1580) >> 32) as u32); 2];
        let t1583: [uint32x4_t; 2] = [vaddq_u32(x24[0], t1581[0]), vaddq_u32(x24[1], t1581[1])];
        let t1584: [uint32x4_t; 2] = [vcltq_u32(t1583[0], x24[0]), vcltq_u32(t1583[1], x24[1])];
        let t1585: [uint32x4_t; 4] = [t1583[0], t1583[1], vsubq_u32(vaddq_u32(x24[2], t1582[0]), t1584[0]), vsubq_u32(vaddq_u32(x24[3], t1582[1]), t1584[1])];
        let mut t1586: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1585[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1585[0]) as u64) as usize as *const u64).read_unaligned(); t1586[0] = vsetq_lane_u32::<0>(v as u32, t1586[0]); t1586[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1586[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1585[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1585[0]) as u64) as usize as *const u64).read_unaligned(); t1586[0] = vsetq_lane_u32::<1>(v as u32, t1586[0]); t1586[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1586[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1585[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1585[0]) as u64) as usize as *const u64).read_unaligned(); t1586[0] = vsetq_lane_u32::<2>(v as u32, t1586[0]); t1586[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1586[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1585[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1585[0]) as u64) as usize as *const u64).read_unaligned(); t1586[0] = vsetq_lane_u32::<3>(v as u32, t1586[0]); t1586[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1586[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1585[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1585[1]) as u64) as usize as *const u64).read_unaligned(); t1586[1] = vsetq_lane_u32::<0>(v as u32, t1586[1]); t1586[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1586[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1585[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1585[1]) as u64) as usize as *const u64).read_unaligned(); t1586[1] = vsetq_lane_u32::<1>(v as u32, t1586[1]); t1586[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1586[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1585[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1585[1]) as u64) as usize as *const u64).read_unaligned(); t1586[1] = vsetq_lane_u32::<2>(v as u32, t1586[1]); t1586[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1586[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1585[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1585[1]) as u64) as usize as *const u64).read_unaligned(); t1586[1] = vsetq_lane_u32::<3>(v as u32, t1586[1]); t1586[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1586[3]); }
        let t1587: [uint32x4_t; 2] = [vsubq_u32(t1574[0], x32[0]), vsubq_u32(t1574[1], x32[1])];
        let t1588: [uint32x4_t; 2] = [t1587[0], t1587[1]];
        let t1589: [uint32x4_t; 2] = [vandq_u32(t1588[0], vdupq_n_u32(63)), vandq_u32(t1588[1], vdupq_n_u32(63))];
        let t1590: [uint32x4_t; 4] = [vshlq_u32(t1586[0], vreinterpretq_s32_u32(t1589[0])), vshlq_u32(t1586[1], vreinterpretq_s32_u32(t1589[1])), vorrq_u32(vshlq_u32(t1586[2], vreinterpretq_s32_u32(t1589[0])), vshlq_u32(t1586[0], vsubq_s32(vreinterpretq_s32_u32(t1589[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1586[3], vreinterpretq_s32_u32(t1589[1])), vshlq_u32(t1586[1], vsubq_s32(vreinterpretq_s32_u32(t1589[1]), vdupq_n_s32(32))))];
        let t1591: [uint32x4_t; 4] = [vorrq_u32(x40[0], t1590[0]), vorrq_u32(x40[1], t1590[1]), vorrq_u32(x40[2], t1590[2]), vorrq_u32(x40[3], t1590[3])];
        t1577 = [vbslq_u32(t1578[0], t1591[0], t1577[0]), vbslq_u32(t1578[1], t1591[1], t1577[1]), vbslq_u32(t1578[0], t1591[2], t1577[2]), vbslq_u32(t1578[1], t1591[3], t1577[3])];
    }
    {
        t1577 = [vbslq_u32(t1579[0], x40[0], t1577[0]), vbslq_u32(t1579[1], x40[1], t1577[1]), vbslq_u32(t1579[0], x40[2], t1577[2]), vbslq_u32(t1579[1], x40[3], t1577[3])];
    }
    res = t1577;
    return res;
}

#[inline(always)]
unsafe fn b28_r0an1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8b: *const u8, x8if: u32, x8is: u32, x16: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if ((x16) == (8u32)) {
        let t1592: [uint32x4_t; 4];
        if (x8is) == 8u32 && ((x8if) as u64) + 7 * ((x8is) as u64) <= u32::MAX as u64 && all(&live) {
            let t1593 = ((arg0).wrapping_add((x8b) as usize) as *const u8).add(((x8if) as usize) * 1);
            let t15940 = vld2q_u32((t1593 as *const u32).add(0));
            let t15941 = vld2q_u32((t1593 as *const u32).add(8));
            let t1594: [uint32x4_t; 4] = [t15940.0, t15941.0, t15940.1, t15941.1];
            t1592 = t1594;
        } else {
            let t1595: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8is)), vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8is))];
            let t1596 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t1595[0]), vandq_u32(live[1], t1595[1]))));
            let t1597: [uint32x4_t; 2] = [vbslq_u32(live[0], t1595[0], t1596), vbslq_u32(live[1], t1595[1], t1596)];
            let mut t1598: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1597[0]) as usize * 1) as *const u64).read_unaligned(); t1598[0] = vsetq_lane_u32::<0>(v as u32, t1598[0]); t1598[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1598[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1597[0]) as usize * 1) as *const u64).read_unaligned(); t1598[0] = vsetq_lane_u32::<1>(v as u32, t1598[0]); t1598[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1598[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1597[0]) as usize * 1) as *const u64).read_unaligned(); t1598[0] = vsetq_lane_u32::<2>(v as u32, t1598[0]); t1598[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1598[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1597[0]) as usize * 1) as *const u64).read_unaligned(); t1598[0] = vsetq_lane_u32::<3>(v as u32, t1598[0]); t1598[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1598[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1597[1]) as usize * 1) as *const u64).read_unaligned(); t1598[1] = vsetq_lane_u32::<0>(v as u32, t1598[1]); t1598[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1598[3]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1597[1]) as usize * 1) as *const u64).read_unaligned(); t1598[1] = vsetq_lane_u32::<1>(v as u32, t1598[1]); t1598[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1598[3]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1597[1]) as usize * 1) as *const u64).read_unaligned(); t1598[1] = vsetq_lane_u32::<2>(v as u32, t1598[1]); t1598[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1598[3]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1597[1]) as usize * 1) as *const u64).read_unaligned(); t1598[1] = vsetq_lane_u32::<3>(v as u32, t1598[1]); t1598[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1598[3]); }
            t1592 = t1598;
        }
        res = t1592;
        return res;
    }
    let t1599: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8is)), vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8is))];
    let t1600: [uint32x4_t; 2] = [t1599[0], t1599[1]];
    let t1601: u64 = ((x8b) as usize as u64);
    let t1602 = [vdupq_n_u32((t1601) as u32); 2];
    let t1603 = [vdupq_n_u32(((t1601) >> 32) as u32); 2];
    let t1604: [uint32x4_t; 2] = [vaddq_u32(t1602[0], t1600[0]), vaddq_u32(t1602[1], t1600[1])];
    let t1605: [uint32x4_t; 2] = [vcltq_u32(t1604[0], t1602[0]), vcltq_u32(t1604[1], t1602[1])];
    let t1606: [uint32x4_t; 4] = [t1604[0], t1604[1], vsubq_u32(vaddq_u32(t1603[0], vdupq_n_u32(0)), t1605[0]), vsubq_u32(vaddq_u32(t1603[1], vdupq_n_u32(0)), t1605[1])];
    let t1607: u64 = !(7u64);
    let t1608 = [vdupq_n_u32((t1607) as u32); 2];
    let t1609 = [vdupq_n_u32(((t1607) >> 32) as u32); 2];
    let t1610: [uint32x4_t; 4] = [vandq_u32(t1606[0], t1608[0]), vandq_u32(t1606[1], t1608[1]), vandq_u32(t1606[2], t1609[0]), vandq_u32(t1606[3], t1609[1])];
    let t1611: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
    let t1612: [uint32x4_t; 4] = [vandq_u32(t1610[0], t1611[0]), vandq_u32(t1610[1], t1611[1]), vandq_u32(t1610[2], t1611[2]), vandq_u32(t1610[3], t1611[3])];
    let t1613: u64 = ((arg0) as usize as u64);
    let t1614 = [vdupq_n_u32((t1613) as u32); 2];
    let t1615 = [vdupq_n_u32(((t1613) >> 32) as u32); 2];
    let t1616: [uint32x4_t; 2] = [vaddq_u32(t1614[0], t1612[0]), vaddq_u32(t1614[1], t1612[1])];
    let t1617: [uint32x4_t; 2] = [vcltq_u32(t1616[0], t1614[0]), vcltq_u32(t1616[1], t1614[1])];
    let t1618: [uint32x4_t; 4] = [t1616[0], t1616[1], vsubq_u32(vaddq_u32(t1615[0], t1612[2]), t1617[0]), vsubq_u32(vaddq_u32(t1615[1], t1612[3]), t1617[1])];
    x24 = t1618;
    let t1619: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8is)), vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8is))];
    let t1620: [uint32x4_t; 2] = [t1619[0], t1619[1]];
    let t1621: u64 = ((x8b) as usize as u64);
    let t1622 = [vdupq_n_u32((t1621) as u32); 2];
    let t1623 = [vdupq_n_u32(((t1621) >> 32) as u32); 2];
    let t1624: [uint32x4_t; 2] = [vaddq_u32(t1622[0], t1620[0]), vaddq_u32(t1622[1], t1620[1])];
    let t1625: [uint32x4_t; 2] = [vcltq_u32(t1624[0], t1622[0]), vcltq_u32(t1624[1], t1622[1])];
    let t1626: [uint32x4_t; 4] = [t1624[0], t1624[1], vsubq_u32(vaddq_u32(t1623[0], vdupq_n_u32(0)), t1625[0]), vsubq_u32(vaddq_u32(t1623[1], vdupq_n_u32(0)), t1625[1])];
    let t1627: u64 = 7u64;
    let t1628 = [vdupq_n_u32((t1627) as u32); 2];
    let t1629: [uint32x4_t; 2] = [vandq_u32(t1626[0], t1628[0]), vandq_u32(t1626[1], t1628[1])];
    let t1630 = [vdupq_n_u32(8u32); 2];
    let t1631: [uint32x4_t; 2] = [vmulq_u32(t1629[0], t1630[0]), vmulq_u32(t1629[1], t1630[1])];
    x32 = t1631;
    let mut t1632: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x24[2]) as u64) << 32 | vgetq_lane_u32::<0>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1632[0] = vsetq_lane_u32::<0>(v as u32, t1632[0]); t1632[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1632[2]); }
    { let v = (((vgetq_lane_u32::<1>(x24[2]) as u64) << 32 | vgetq_lane_u32::<1>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1632[0] = vsetq_lane_u32::<1>(v as u32, t1632[0]); t1632[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1632[2]); }
    { let v = (((vgetq_lane_u32::<2>(x24[2]) as u64) << 32 | vgetq_lane_u32::<2>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1632[0] = vsetq_lane_u32::<2>(v as u32, t1632[0]); t1632[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1632[2]); }
    { let v = (((vgetq_lane_u32::<3>(x24[2]) as u64) << 32 | vgetq_lane_u32::<3>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1632[0] = vsetq_lane_u32::<3>(v as u32, t1632[0]); t1632[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1632[2]); }
    { let v = (((vgetq_lane_u32::<0>(x24[3]) as u64) << 32 | vgetq_lane_u32::<0>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1632[1] = vsetq_lane_u32::<0>(v as u32, t1632[1]); t1632[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1632[3]); }
    { let v = (((vgetq_lane_u32::<1>(x24[3]) as u64) << 32 | vgetq_lane_u32::<1>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1632[1] = vsetq_lane_u32::<1>(v as u32, t1632[1]); t1632[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1632[3]); }
    { let v = (((vgetq_lane_u32::<2>(x24[3]) as u64) << 32 | vgetq_lane_u32::<2>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1632[1] = vsetq_lane_u32::<2>(v as u32, t1632[1]); t1632[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1632[3]); }
    { let v = (((vgetq_lane_u32::<3>(x24[3]) as u64) << 32 | vgetq_lane_u32::<3>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1632[1] = vsetq_lane_u32::<3>(v as u32, t1632[1]); t1632[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1632[3]); }
    let t1633: [uint32x4_t; 2] = [x32[0], x32[1]];
    let t1634: [uint32x4_t; 2] = [vandq_u32(t1633[0], vdupq_n_u32(63)), vandq_u32(t1633[1], vdupq_n_u32(63))];
    let t1635: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t1632[0], vnegq_s32(vreinterpretq_s32_u32(t1634[0]))), vshlq_u32(t1632[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1634[0])))), vorrq_u32(vshlq_u32(t1632[1], vnegq_s32(vreinterpretq_s32_u32(t1634[1]))), vshlq_u32(t1632[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1634[1])))), vshlq_u32(t1632[2], vnegq_s32(vreinterpretq_s32_u32(t1634[0]))), vshlq_u32(t1632[3], vnegq_s32(vreinterpretq_s32_u32(t1634[1])))];
    x40 = t1635;
    let t1636 = [vdupq_n_u32(0u32); 2];
    let t1637: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t1636[0])), vmvnq_u32(vceqq_u32(x32[1], t1636[1]))];
    let t1638 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t1639: [uint32x4_t; 2] = [vaddq_u32(x32[0], t1638[0]), vaddq_u32(x32[1], t1638[1])];
    let t1640 = [vdupq_n_u32(64u32); 2];
    let t1641: [uint32x4_t; 2] = [vcgtq_u32(t1639[0], t1640[0]), vcgtq_u32(t1639[1], t1640[1])];
    let t1642: [uint32x4_t; 2] = [vandq_u32(t1637[0], t1641[0]), vandq_u32(t1637[1], t1641[1])];
    let mut t1643: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1644: [uint32x4_t; 2] = [vandq_u32(live[0], t1642[0]), vandq_u32(live[1], t1642[1])];
    let t1645: [uint32x4_t; 2] = [vbicq_u32(live[0], t1642[0]), vbicq_u32(live[1], t1642[1])];
    if any(&t1644) {
        let t1646: u64 = (1u64).wrapping_mul(8u64);
        let t1647 = [vdupq_n_u32((t1646) as u32); 2];
        let t1648 = [vdupq_n_u32(((t1646) >> 32) as u32); 2];
        let t1649: [uint32x4_t; 2] = [vaddq_u32(x24[0], t1647[0]), vaddq_u32(x24[1], t1647[1])];
        let t1650: [uint32x4_t; 2] = [vcltq_u32(t1649[0], x24[0]), vcltq_u32(t1649[1], x24[1])];
        let t1651: [uint32x4_t; 4] = [t1649[0], t1649[1], vsubq_u32(vaddq_u32(x24[2], t1648[0]), t1650[0]), vsubq_u32(vaddq_u32(x24[3], t1648[1]), t1650[1])];
        let mut t1652: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1651[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1651[0]) as u64) as usize as *const u64).read_unaligned(); t1652[0] = vsetq_lane_u32::<0>(v as u32, t1652[0]); t1652[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1652[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1651[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1651[0]) as u64) as usize as *const u64).read_unaligned(); t1652[0] = vsetq_lane_u32::<1>(v as u32, t1652[0]); t1652[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1652[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1651[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1651[0]) as u64) as usize as *const u64).read_unaligned(); t1652[0] = vsetq_lane_u32::<2>(v as u32, t1652[0]); t1652[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1652[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1651[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1651[0]) as u64) as usize as *const u64).read_unaligned(); t1652[0] = vsetq_lane_u32::<3>(v as u32, t1652[0]); t1652[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1652[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1651[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1651[1]) as u64) as usize as *const u64).read_unaligned(); t1652[1] = vsetq_lane_u32::<0>(v as u32, t1652[1]); t1652[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1652[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1651[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1651[1]) as u64) as usize as *const u64).read_unaligned(); t1652[1] = vsetq_lane_u32::<1>(v as u32, t1652[1]); t1652[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1652[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1651[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1651[1]) as u64) as usize as *const u64).read_unaligned(); t1652[1] = vsetq_lane_u32::<2>(v as u32, t1652[1]); t1652[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1652[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1651[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1651[1]) as u64) as usize as *const u64).read_unaligned(); t1652[1] = vsetq_lane_u32::<3>(v as u32, t1652[1]); t1652[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1652[3]); }
        let t1653: [uint32x4_t; 2] = [vsubq_u32(t1640[0], x32[0]), vsubq_u32(t1640[1], x32[1])];
        let t1654: [uint32x4_t; 2] = [t1653[0], t1653[1]];
        let t1655: [uint32x4_t; 2] = [vandq_u32(t1654[0], vdupq_n_u32(63)), vandq_u32(t1654[1], vdupq_n_u32(63))];
        let t1656: [uint32x4_t; 4] = [vshlq_u32(t1652[0], vreinterpretq_s32_u32(t1655[0])), vshlq_u32(t1652[1], vreinterpretq_s32_u32(t1655[1])), vorrq_u32(vshlq_u32(t1652[2], vreinterpretq_s32_u32(t1655[0])), vshlq_u32(t1652[0], vsubq_s32(vreinterpretq_s32_u32(t1655[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1652[3], vreinterpretq_s32_u32(t1655[1])), vshlq_u32(t1652[1], vsubq_s32(vreinterpretq_s32_u32(t1655[1]), vdupq_n_s32(32))))];
        let t1657: [uint32x4_t; 4] = [vorrq_u32(x40[0], t1656[0]), vorrq_u32(x40[1], t1656[1]), vorrq_u32(x40[2], t1656[2]), vorrq_u32(x40[3], t1656[3])];
        t1643 = [vbslq_u32(t1644[0], t1657[0], t1643[0]), vbslq_u32(t1644[1], t1657[1], t1643[1]), vbslq_u32(t1644[0], t1657[2], t1643[2]), vbslq_u32(t1644[1], t1657[3], t1643[3])];
    }
    {
        t1643 = [vbslq_u32(t1645[0], x40[0], t1643[0]), vbslq_u32(t1645[1], x40[1], t1643[1]), vbslq_u32(t1645[0], x40[2], t1643[2]), vbslq_u32(t1645[1], x40[3], t1643[3])];
    }
    res = t1643;
    return res;
}

#[inline(always)]
unsafe fn b28_r0an1s_f(l: &Launch, lid0: u32, arg0: *const u8, x8b: *const u8, x8if: u32, x8is: u32, x16: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if ((x16) == (8u32)) {
        let t1658: [uint32x4_t; 4];
        if (x8is) == 8u32 && ((x8if) as u64) + 7 * ((x8is) as u64) <= u32::MAX as u64 {
            let t1659 = ((arg0).wrapping_add((x8b) as usize) as *const u8).add(((x8if) as usize) * 1);
            let t16600 = vld2q_u32((t1659 as *const u32).add(0));
            let t16601 = vld2q_u32((t1659 as *const u32).add(8));
            let t1660: [uint32x4_t; 4] = [t16600.0, t16601.0, t16600.1, t16601.1];
            t1658 = t1660;
        } else {
            let t1661: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8is)), vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8is))];
            let mut t1662: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1661[0]) as usize * 1) as *const u64).read_unaligned(); t1662[0] = vsetq_lane_u32::<0>(v as u32, t1662[0]); t1662[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1662[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1661[0]) as usize * 1) as *const u64).read_unaligned(); t1662[0] = vsetq_lane_u32::<1>(v as u32, t1662[0]); t1662[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1662[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1661[0]) as usize * 1) as *const u64).read_unaligned(); t1662[0] = vsetq_lane_u32::<2>(v as u32, t1662[0]); t1662[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1662[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1661[0]) as usize * 1) as *const u64).read_unaligned(); t1662[0] = vsetq_lane_u32::<3>(v as u32, t1662[0]); t1662[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1662[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1661[1]) as usize * 1) as *const u64).read_unaligned(); t1662[1] = vsetq_lane_u32::<0>(v as u32, t1662[1]); t1662[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1662[3]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1661[1]) as usize * 1) as *const u64).read_unaligned(); t1662[1] = vsetq_lane_u32::<1>(v as u32, t1662[1]); t1662[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1662[3]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1661[1]) as usize * 1) as *const u64).read_unaligned(); t1662[1] = vsetq_lane_u32::<2>(v as u32, t1662[1]); t1662[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1662[3]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1661[1]) as usize * 1) as *const u64).read_unaligned(); t1662[1] = vsetq_lane_u32::<3>(v as u32, t1662[1]); t1662[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1662[3]); }
            t1658 = t1662;
        }
        res = t1658;
        return res;
    }
    let t1663: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8is)), vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8is))];
    let t1664: [uint32x4_t; 2] = [t1663[0], t1663[1]];
    let t1665: u64 = ((x8b) as usize as u64);
    let t1666 = [vdupq_n_u32((t1665) as u32); 2];
    let t1667 = [vdupq_n_u32(((t1665) >> 32) as u32); 2];
    let t1668: [uint32x4_t; 2] = [vaddq_u32(t1666[0], t1664[0]), vaddq_u32(t1666[1], t1664[1])];
    let t1669: [uint32x4_t; 2] = [vcltq_u32(t1668[0], t1666[0]), vcltq_u32(t1668[1], t1666[1])];
    let t1670: [uint32x4_t; 4] = [t1668[0], t1668[1], vsubq_u32(vaddq_u32(t1667[0], vdupq_n_u32(0)), t1669[0]), vsubq_u32(vaddq_u32(t1667[1], vdupq_n_u32(0)), t1669[1])];
    let t1671: u64 = !(7u64);
    let t1672 = [vdupq_n_u32((t1671) as u32); 2];
    let t1673 = [vdupq_n_u32(((t1671) >> 32) as u32); 2];
    let t1674: [uint32x4_t; 4] = [vandq_u32(t1670[0], t1672[0]), vandq_u32(t1670[1], t1672[1]), vandq_u32(t1670[2], t1673[0]), vandq_u32(t1670[3], t1673[1])];
    let t1675: u64 = ((arg0) as usize as u64);
    let t1676 = [vdupq_n_u32((t1675) as u32); 2];
    let t1677 = [vdupq_n_u32(((t1675) >> 32) as u32); 2];
    let t1678: [uint32x4_t; 2] = [vaddq_u32(t1676[0], t1674[0]), vaddq_u32(t1676[1], t1674[1])];
    let t1679: [uint32x4_t; 2] = [vcltq_u32(t1678[0], t1676[0]), vcltq_u32(t1678[1], t1676[1])];
    let t1680: [uint32x4_t; 4] = [t1678[0], t1678[1], vsubq_u32(vaddq_u32(t1677[0], t1674[2]), t1679[0]), vsubq_u32(vaddq_u32(t1677[1], t1674[3]), t1679[1])];
    x24 = t1680;
    let t1681: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8is)), vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8is))];
    let t1682: [uint32x4_t; 2] = [t1681[0], t1681[1]];
    let t1683: u64 = ((x8b) as usize as u64);
    let t1684 = [vdupq_n_u32((t1683) as u32); 2];
    let t1685 = [vdupq_n_u32(((t1683) >> 32) as u32); 2];
    let t1686: [uint32x4_t; 2] = [vaddq_u32(t1684[0], t1682[0]), vaddq_u32(t1684[1], t1682[1])];
    let t1687: [uint32x4_t; 2] = [vcltq_u32(t1686[0], t1684[0]), vcltq_u32(t1686[1], t1684[1])];
    let t1688: [uint32x4_t; 4] = [t1686[0], t1686[1], vsubq_u32(vaddq_u32(t1685[0], vdupq_n_u32(0)), t1687[0]), vsubq_u32(vaddq_u32(t1685[1], vdupq_n_u32(0)), t1687[1])];
    let t1689: u64 = 7u64;
    let t1690 = [vdupq_n_u32((t1689) as u32); 2];
    let t1691: [uint32x4_t; 2] = [vandq_u32(t1688[0], t1690[0]), vandq_u32(t1688[1], t1690[1])];
    let t1692 = [vdupq_n_u32(8u32); 2];
    let t1693: [uint32x4_t; 2] = [vmulq_u32(t1691[0], t1692[0]), vmulq_u32(t1691[1], t1692[1])];
    x32 = t1693;
    let mut t1694: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x24[2]) as u64) << 32 | vgetq_lane_u32::<0>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1694[0] = vsetq_lane_u32::<0>(v as u32, t1694[0]); t1694[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1694[2]); }
    { let v = (((vgetq_lane_u32::<1>(x24[2]) as u64) << 32 | vgetq_lane_u32::<1>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1694[0] = vsetq_lane_u32::<1>(v as u32, t1694[0]); t1694[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1694[2]); }
    { let v = (((vgetq_lane_u32::<2>(x24[2]) as u64) << 32 | vgetq_lane_u32::<2>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1694[0] = vsetq_lane_u32::<2>(v as u32, t1694[0]); t1694[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1694[2]); }
    { let v = (((vgetq_lane_u32::<3>(x24[2]) as u64) << 32 | vgetq_lane_u32::<3>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1694[0] = vsetq_lane_u32::<3>(v as u32, t1694[0]); t1694[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1694[2]); }
    { let v = (((vgetq_lane_u32::<0>(x24[3]) as u64) << 32 | vgetq_lane_u32::<0>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1694[1] = vsetq_lane_u32::<0>(v as u32, t1694[1]); t1694[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1694[3]); }
    { let v = (((vgetq_lane_u32::<1>(x24[3]) as u64) << 32 | vgetq_lane_u32::<1>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1694[1] = vsetq_lane_u32::<1>(v as u32, t1694[1]); t1694[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1694[3]); }
    { let v = (((vgetq_lane_u32::<2>(x24[3]) as u64) << 32 | vgetq_lane_u32::<2>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1694[1] = vsetq_lane_u32::<2>(v as u32, t1694[1]); t1694[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1694[3]); }
    { let v = (((vgetq_lane_u32::<3>(x24[3]) as u64) << 32 | vgetq_lane_u32::<3>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1694[1] = vsetq_lane_u32::<3>(v as u32, t1694[1]); t1694[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1694[3]); }
    let t1695: [uint32x4_t; 2] = [x32[0], x32[1]];
    let t1696: [uint32x4_t; 2] = [vandq_u32(t1695[0], vdupq_n_u32(63)), vandq_u32(t1695[1], vdupq_n_u32(63))];
    let t1697: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t1694[0], vnegq_s32(vreinterpretq_s32_u32(t1696[0]))), vshlq_u32(t1694[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1696[0])))), vorrq_u32(vshlq_u32(t1694[1], vnegq_s32(vreinterpretq_s32_u32(t1696[1]))), vshlq_u32(t1694[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1696[1])))), vshlq_u32(t1694[2], vnegq_s32(vreinterpretq_s32_u32(t1696[0]))), vshlq_u32(t1694[3], vnegq_s32(vreinterpretq_s32_u32(t1696[1])))];
    x40 = t1697;
    let t1698 = [vdupq_n_u32(0u32); 2];
    let t1699: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t1698[0])), vmvnq_u32(vceqq_u32(x32[1], t1698[1]))];
    let t1700 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t1701: [uint32x4_t; 2] = [vaddq_u32(x32[0], t1700[0]), vaddq_u32(x32[1], t1700[1])];
    let t1702 = [vdupq_n_u32(64u32); 2];
    let t1703: [uint32x4_t; 2] = [vcgtq_u32(t1701[0], t1702[0]), vcgtq_u32(t1701[1], t1702[1])];
    let t1704: [uint32x4_t; 2] = [vandq_u32(t1699[0], t1703[0]), vandq_u32(t1699[1], t1703[1])];
    let mut t1705: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1706: [uint32x4_t; 2] = [vmvnq_u32(t1704[0]), vmvnq_u32(t1704[1])];
    if any(&t1704) {
        let t1707: u64 = (1u64).wrapping_mul(8u64);
        let t1708 = [vdupq_n_u32((t1707) as u32); 2];
        let t1709 = [vdupq_n_u32(((t1707) >> 32) as u32); 2];
        let t1710: [uint32x4_t; 2] = [vaddq_u32(x24[0], t1708[0]), vaddq_u32(x24[1], t1708[1])];
        let t1711: [uint32x4_t; 2] = [vcltq_u32(t1710[0], x24[0]), vcltq_u32(t1710[1], x24[1])];
        let t1712: [uint32x4_t; 4] = [t1710[0], t1710[1], vsubq_u32(vaddq_u32(x24[2], t1709[0]), t1711[0]), vsubq_u32(vaddq_u32(x24[3], t1709[1]), t1711[1])];
        let mut t1713: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1712[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1712[0]) as u64) as usize as *const u64).read_unaligned(); t1713[0] = vsetq_lane_u32::<0>(v as u32, t1713[0]); t1713[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1713[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1712[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1712[0]) as u64) as usize as *const u64).read_unaligned(); t1713[0] = vsetq_lane_u32::<1>(v as u32, t1713[0]); t1713[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1713[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1712[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1712[0]) as u64) as usize as *const u64).read_unaligned(); t1713[0] = vsetq_lane_u32::<2>(v as u32, t1713[0]); t1713[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1713[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1712[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1712[0]) as u64) as usize as *const u64).read_unaligned(); t1713[0] = vsetq_lane_u32::<3>(v as u32, t1713[0]); t1713[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1713[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1712[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1712[1]) as u64) as usize as *const u64).read_unaligned(); t1713[1] = vsetq_lane_u32::<0>(v as u32, t1713[1]); t1713[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1713[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1712[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1712[1]) as u64) as usize as *const u64).read_unaligned(); t1713[1] = vsetq_lane_u32::<1>(v as u32, t1713[1]); t1713[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1713[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1712[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1712[1]) as u64) as usize as *const u64).read_unaligned(); t1713[1] = vsetq_lane_u32::<2>(v as u32, t1713[1]); t1713[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1713[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1712[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1712[1]) as u64) as usize as *const u64).read_unaligned(); t1713[1] = vsetq_lane_u32::<3>(v as u32, t1713[1]); t1713[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1713[3]); }
        let t1714: [uint32x4_t; 2] = [vsubq_u32(t1702[0], x32[0]), vsubq_u32(t1702[1], x32[1])];
        let t1715: [uint32x4_t; 2] = [t1714[0], t1714[1]];
        let t1716: [uint32x4_t; 2] = [vandq_u32(t1715[0], vdupq_n_u32(63)), vandq_u32(t1715[1], vdupq_n_u32(63))];
        let t1717: [uint32x4_t; 4] = [vshlq_u32(t1713[0], vreinterpretq_s32_u32(t1716[0])), vshlq_u32(t1713[1], vreinterpretq_s32_u32(t1716[1])), vorrq_u32(vshlq_u32(t1713[2], vreinterpretq_s32_u32(t1716[0])), vshlq_u32(t1713[0], vsubq_s32(vreinterpretq_s32_u32(t1716[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1713[3], vreinterpretq_s32_u32(t1716[1])), vshlq_u32(t1713[1], vsubq_s32(vreinterpretq_s32_u32(t1716[1]), vdupq_n_s32(32))))];
        let t1718: [uint32x4_t; 4] = [vorrq_u32(x40[0], t1717[0]), vorrq_u32(x40[1], t1717[1]), vorrq_u32(x40[2], t1717[2]), vorrq_u32(x40[3], t1717[3])];
        t1705 = [vbslq_u32(t1704[0], t1718[0], t1705[0]), vbslq_u32(t1704[1], t1718[1], t1705[1]), vbslq_u32(t1704[0], t1718[2], t1705[2]), vbslq_u32(t1704[1], t1718[3], t1705[3])];
    }
    {
        t1705 = [vbslq_u32(t1706[0], x40[0], t1705[0]), vbslq_u32(t1706[1], x40[1], t1705[1]), vbslq_u32(t1706[0], x40[2], t1705[2]), vbslq_u32(t1706[1], x40[3], t1705[3])];
    }
    res = t1705;
    return res;
}

#[inline(always)]
unsafe fn b29_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t1719: u64;
    if ((x0) >= (64u32)) {
        t1719 = !(0u64);
    } else {
        t1719 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t1719;
    return res;
}

#[inline(always)]
unsafe fn b29_s_f(l: &Launch, lid0: u32, x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t1720: u64;
    if ((x0) >= (64u32)) {
        t1720 = !(0u64);
    } else {
        t1720 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t1720;
    return res;
}

#[inline(always)]
unsafe fn b30_r0sss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: u32, x16: u32, x24: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u64 = 0;
    let mut live = k;
    let mut res: u64 = 0;
    let t1721 = b28_r0ss_p(l, lid0, live, arg0, (((x8) as u64)).wrapping_add(((x16) as u64)), x24);
    x32 = t1721;
    let t1722: u64;
    if ((x24) >= (8u32)) {
        t1722 = x32;
    } else {
        let t1723 = b29_s_p(l, lid0, live, (8u32).wrapping_mul(x24));
        t1722 = ((x32) & (t1723));
    }
    res = t1722;
    return res;
}

#[inline(always)]
unsafe fn b30_r0sss_f(l: &Launch, lid0: u32, arg0: *const u8, x8: u32, x16: u32, x24: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u64 = 0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t1724 = b28_r0ss_f(l, lid0, arg0, (((x8) as u64)).wrapping_add(((x16) as u64)), x24);
    x32 = t1724;
    let t1725: u64;
    if ((x24) >= (8u32)) {
        t1725 = x32;
    } else {
        let t1726 = b29_s_f(l, lid0, (8u32).wrapping_mul(x24));
        t1725 = ((x32) & (t1726));
    }
    res = t1725;
    return res;
}

#[inline(always)]
unsafe fn b30_r0sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: u32, x16: [uint32x4_t; 2], x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1727 = b28_r0al1s_p(l, lid0, live, arg0, ((((x8) as u64)) as usize as *const u8), x16, x24);
    x32 = t1727;
    let t1728: [uint32x4_t; 4];
    if ((x24) >= (8u32)) {
        t1728 = x32;
    } else {
        let t1729 = b29_s_p(l, lid0, live, (8u32).wrapping_mul(x24));
        let t1730: u64 = t1729;
        let t1731 = [vdupq_n_u32((t1730) as u32); 2];
        let t1732 = [vdupq_n_u32(((t1730) >> 32) as u32); 2];
        let t1733: [uint32x4_t; 4] = [vandq_u32(x32[0], t1731[0]), vandq_u32(x32[1], t1731[1]), vandq_u32(x32[2], t1732[0]), vandq_u32(x32[3], t1732[1])];
        t1728 = t1733;
    }
    res = t1728;
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
    let mut t1734: bool = !(arg6).is_null();
    if t1734 {
        let mut t1735: bool = ((l.group[0]) >= ((arg6.wrapping_add((((x64) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        if !t1735 {
            t1735 = ((l.group[1]) >= ((arg6.wrapping_add(((((x64).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        let mut t1736: bool = t1735;
        if !t1736 {
            t1736 = ((l.group[2]) >= ((arg6.wrapping_add(((((x64).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        t1734 = t1736;
    }
    if t1734 {
        return;
    }
    let mut lid0 = 0u32;
    while lid0 + 8 <= n {
        let mut live = [vdupq_n_u32(u32::MAX); 2];
        let gid0 = l.group[0] * l.block[0] + lid0;
        'chunk: {
            if (lid0 & 31u32) <= 24u32 { b0_r0r1r2r3r4sgu5_f(l, lid0, arg0, arg1, arg2, arg3, arg4, x40, lid0) } else { b0_r0r1r2r3r4sgi_f(l, lid0, arg0, arg1, arg2, arg3, arg4, x40, lid0) };
        }
        lid0 += 8;
    }
    if lid0 < n {
        // Lanes part: the group's last threads, those under `n`.
        let t1737: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
        let mut live = t1737;
        let gid0 = l.group[0] * l.block[0] + lid0;
        'chunk: {
            if (lid0 & 31u32) <= 24u32 { b0_r0r1r2r3r4sgu5_p(l, lid0, live, arg0, arg1, arg2, arg3, arg4, x40, lid0) } else { b0_r0r1r2r3r4sgi_p(l, lid0, live, arg0, arg1, arg2, arg3, arg4, x40, lid0) };
        }
    }
}
