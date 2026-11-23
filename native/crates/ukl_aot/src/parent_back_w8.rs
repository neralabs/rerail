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
            let mut t49 = t36;
            loop {
                // Lanes part: each its own rounds.
                let t51: [uint32x4_t; 2] = [vcltq_u32(x56[0], x48[0]), vcltq_u32(x56[1], x48[1])];
                t49 = [vandq_u32(t49[0], t51[0]), vandq_u32(t49[1], t51[1])];
                if !(any(&t49)) { break; }
                let t53 = [vdupq_n_u32(x16); 2];
                let mut t54w = 0u32;
                let t54 = core::hint::black_box(&raw mut t54w);
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(t49[0]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<0>(x56[0]) as usize), t54).write(vgetq_lane_u32::<0>(t53[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(t49[0]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<1>(x56[0]) as usize), t54).write(vgetq_lane_u32::<1>(t53[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(t49[0]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<2>(x56[0]) as usize), t54).write(vgetq_lane_u32::<2>(t53[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(t49[0]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<3>(x56[0]) as usize), t54).write(vgetq_lane_u32::<3>(t53[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(t49[1]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<0>(x56[1]) as usize), t54).write(vgetq_lane_u32::<0>(t53[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(t49[1]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<1>(x56[1]) as usize), t54).write(vgetq_lane_u32::<1>(t53[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(t49[1]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<2>(x56[1]) as usize), t54).write(vgetq_lane_u32::<2>(t53[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(t49[1]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<3>(x56[1]) as usize), t54).write(vgetq_lane_u32::<3>(t53[1]));
                let t55: [uint32x4_t; 2] = [vandq_u32(t49[0], live[0]), vandq_u32(t49[1], live[1])];
                let t56 = x56;
                x56 = [vsubq_u32(x56[0], t55[0]), vsubq_u32(x56[1], t55[1])];
            }
            b5__p(l, lid0, t36);
            let t57 = b2_o4s_p(l, lid0, t36, arg4_x24.wrapping_add((32u64).wrapping_mul(4u64) as usize), 1u32);
        }
        let t58: [uint32x4_t; 2] = [vbicq_u32(vandq_u32(t25[0], live[0]), t26[0]), vbicq_u32(vandq_u32(t25[1], live[1]), t26[1])];
    }
    if any(&live) {
        let t59 = [vdupq_n_u32(0u32); 2];
        x40 = [vbslq_u32(live[0], t59[0], x40[0]), vbslq_u32(live[1], t59[1], x40[1])];
        let mut t60 = live;
        loop {
            // Lanes part: each its own rounds.
            let t62 = [vdupq_n_u32(65536u32); 2];
            let t63: [uint32x4_t; 2] = [vcltq_u32(x40[0], t62[0]), vcltq_u32(x40[1], t62[1])];
            t60 = [vandq_u32(t60[0], t63[0]), vandq_u32(t60[1], t63[1])];
            if !(any(&t60)) { break; }
            let t65 = b4_o4_p(l, lid0, t60, arg4_x24.wrapping_add((32u64).wrapping_mul(4u64) as usize));
            if ((t65) >= (x32)) {
                res = ((1i32) != 0);
                live = [vbicq_u32(live[0], t60[0]), vbicq_u32(live[1], t60[1])];
            }
            let t66: [uint32x4_t; 2] = [vandq_u32(t60[0], live[0]), vandq_u32(t60[1], live[1])];
            let t67 = [vdupq_n_u32(1u32); 2];
            let t68: [uint32x4_t; 2] = [vaddq_u32(x40[0], t67[0]), vaddq_u32(x40[1], t67[1])];
            let t69 = x40;
            x40 = t68;
            t60 = [vandq_u32(t60[0], live[0]), vandq_u32(t60[1], live[1])];
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
    let mut t70 = live;
    let mut t71 = [vdupq_n_u32(0); 2];
    loop {
        // Lanes part: each its own rounds.
        let t72 = [vdupq_n_u32(if true { u32::MAX } else { 0 }); 2];
        t70 = [vandq_u32(vbicq_u32(t70[0], t71[0]), t72[0]), vandq_u32(vbicq_u32(t70[1], t71[1]), t72[1])];
        if !(any(&t70)) { break; }
        let t74 = b4_o4_p(l, lid0, t70, arg4_x24.wrapping_add((0u64).wrapping_mul(4u64) as usize));
        let t75: [uint32x4_t; 2];
        if ((t74) < (x32)) {
            let t76 = b2_o4s_p(l, lid0, t70, arg4_x24.wrapping_add((0u64).wrapping_mul(4u64) as usize), 1u32);
            t75 = t76;
        } else {
            let t77 = [vdupq_n_u32(x32); 2];
            t75 = t77;
        }
        x40 = [vbslq_u32(t70[0], t75[0], x40[0]), vbslq_u32(t70[1], t75[1], x40[1])];
        let t78 = [vdupq_n_u32(x32); 2];
        let t79: [uint32x4_t; 2] = [vcgeq_u32(x40[0], t78[0]), vcgeq_u32(x40[1], t78[1])];
        let t80: [uint32x4_t; 2] = [vandq_u32(t70[0], t79[0]), vandq_u32(t70[1], t79[1])];
        if any(&t80) {
            t71 = [vorrq_u32(t71[0], t80[0]), vorrq_u32(t71[1], t80[1])];
        }
        let t81: [uint32x4_t; 2] = [vbicq_u32(vandq_u32(t70[0], live[0]), t71[0]), vbicq_u32(vandq_u32(t70[1], live[1]), t71[1])];
        if any(&t81) {
            let t82 = [vdupq_n_u32(256u32); 2];
            let t83: [uint32x4_t; 2] = [vmulq_u32(x40[0], t82[0]), vmulq_u32(x40[1], t82[1])];
            let t84: [uint32x4_t; 2] = [vaddq_u32(t83[0], t82[0]), vaddq_u32(t83[1], t82[1])];
            let t85 = [vdupq_n_u32(x8); 2];
            let t86: [uint32x4_t; 2] = [vcltq_u32(t84[0], t85[0]), vcltq_u32(t84[1], t85[1])];
            let mut t87: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t88: [uint32x4_t; 2] = [vandq_u32(t81[0], t86[0]), vandq_u32(t81[1], t86[1])];
            let t89: [uint32x4_t; 2] = [vbicq_u32(t81[0], t86[0]), vbicq_u32(t81[1], t86[1])];
            {
                let t90: [uint32x4_t; 2] = [vmulq_u32(x40[0], t82[0]), vmulq_u32(x40[1], t82[1])];
                let t91: [uint32x4_t; 2] = [vaddq_u32(t90[0], t82[0]), vaddq_u32(t90[1], t82[1])];
                t87 = [vbslq_u32(t88[0], t91[0], t87[0]), vbslq_u32(t88[1], t91[1], t87[1])];
            }
            {
                let t92 = [vdupq_n_u32(x8); 2];
                t87 = [vbslq_u32(t89[0], t92[0], t87[0]), vbslq_u32(t89[1], t92[1], t87[1])];
            }
            x48 = t87;
            let t93: [uint32x4_t; 2] = [vmulq_u32(x40[0], t82[0]), vmulq_u32(x40[1], t82[1])];
            x56 = t93;
            let mut t94 = t81;
            loop {
                // Lanes part: each its own rounds.
                let t96: [uint32x4_t; 2] = [vcltq_u32(x56[0], x48[0]), vcltq_u32(x56[1], x48[1])];
                t94 = [vandq_u32(t94[0], t96[0]), vandq_u32(t94[1], t96[1])];
                if !(any(&t94)) { break; }
                let t98 = [vdupq_n_u32(x16); 2];
                let mut t99w = 0u32;
                let t99 = core::hint::black_box(&raw mut t99w);
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(t94[0]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<0>(x56[0]) as usize), t99).write(vgetq_lane_u32::<0>(t98[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(t94[0]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<1>(x56[0]) as usize), t99).write(vgetq_lane_u32::<1>(t98[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(t94[0]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<2>(x56[0]) as usize), t99).write(vgetq_lane_u32::<2>(t98[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(t94[0]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<3>(x56[0]) as usize), t99).write(vgetq_lane_u32::<3>(t98[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(t94[1]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<0>(x56[1]) as usize), t99).write(vgetq_lane_u32::<0>(t98[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(t94[1]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<1>(x56[1]) as usize), t99).write(vgetq_lane_u32::<1>(t98[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(t94[1]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<2>(x56[1]) as usize), t99).write(vgetq_lane_u32::<2>(t98[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(t94[1]) != 0, (arg3 as *mut u32).add(vgetq_lane_u32::<3>(x56[1]) as usize), t99).write(vgetq_lane_u32::<3>(t98[1]));
                let t100: [uint32x4_t; 2] = [vandq_u32(t94[0], live[0]), vandq_u32(t94[1], live[1])];
                let t101 = x56;
                x56 = [vsubq_u32(x56[0], t100[0]), vsubq_u32(x56[1], t100[1])];
            }
            b5__p(l, lid0, t81);
            let t102 = b2_o4s_p(l, lid0, t81, arg4_x24.wrapping_add((32u64).wrapping_mul(4u64) as usize), 1u32);
        }
        let t103: [uint32x4_t; 2] = [vbicq_u32(vandq_u32(t70[0], live[0]), t71[0]), vbicq_u32(vandq_u32(t70[1], live[1]), t71[1])];
    }
    if any(&live) {
        let t104 = [vdupq_n_u32(0u32); 2];
        x40 = [vbslq_u32(live[0], t104[0], x40[0]), vbslq_u32(live[1], t104[1], x40[1])];
        let mut t105 = live;
        loop {
            // Lanes part: each its own rounds.
            let t107 = [vdupq_n_u32(65536u32); 2];
            let t108: [uint32x4_t; 2] = [vcltq_u32(x40[0], t107[0]), vcltq_u32(x40[1], t107[1])];
            t105 = [vandq_u32(t105[0], t108[0]), vandq_u32(t105[1], t108[1])];
            if !(any(&t105)) { break; }
            let t110 = b4_o4_p(l, lid0, t105, arg4_x24.wrapping_add((32u64).wrapping_mul(4u64) as usize));
            if ((t110) >= (x32)) {
                res = ((1i32) != 0);
                live = [vbicq_u32(live[0], t105[0]), vbicq_u32(live[1], t105[1])];
            }
            let t111: [uint32x4_t; 2] = [vandq_u32(t105[0], live[0]), vandq_u32(t105[1], live[1])];
            let t112 = [vdupq_n_u32(1u32); 2];
            let t113: [uint32x4_t; 2] = [vaddq_u32(x40[0], t112[0]), vaddq_u32(x40[1], t112[1])];
            let t114 = x40;
            x40 = t113;
            t105 = [vandq_u32(t105[0], live[0]), vandq_u32(t105[1], live[1])];
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
    let mut t115 = [0u32; 8];
    for r in 0..2 { vst1q_u32(t115.as_mut_ptr().add(4 * r), live[r]); }
    let mut t116 = [0u32; 8];
    let mut t117 = 0u32;
    for j in 0..8 { t116[j] = t117; t117 += t115[j] & 1; }
    let t118: [uint32x4_t; 2] = [vld1q_u32(t116.as_ptr().add(0)), vld1q_u32(t116.as_ptr().add(4))];
    let t119 = AtomicU32::from_ptr(arg4_x0 as *mut u32).fetch_add((x8).wrapping_mul(t117), Relaxed);
    let t120: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(t119), t118[0], vdupq_n_u32(x8)), vmlaq_u32(vdupq_n_u32(t119), t118[1], vdupq_n_u32(x8))];
    res = t120;
    return res;
}

#[inline(always)]
unsafe fn b2_o4s_f(l: &Launch, lid0: u32, arg4_x0: *const u8, x8: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t121: [uint32x4_t; 2] = [vld1q_u32(LANE[0].as_ptr()), vld1q_u32(LANE[1].as_ptr())];
    let t122 = AtomicU32::from_ptr(arg4_x0 as *mut u32).fetch_add((x8).wrapping_mul(8u32), Relaxed);
    let t123: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(t122), t121[0], vdupq_n_u32(x8)), vmlaq_u32(vdupq_n_u32(t122), t121[1], vdupq_n_u32(x8))];
    res = t123;
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
        let t124: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t125 = [vdupq_n_u32(x32); 2];
        let t126: [uint32x4_t; 2] = [vcltq_u32(t124[0], t125[0]), vcltq_u32(t124[1], t125[1])];
        let t127: [uint32x4_t; 2] = [vandq_u32(live[0], t126[0]), vandq_u32(live[1], t126[1])];
        if any(&t127) {
            let t128 = b7_r0r1g_p(l, lid0, t127, arg0, arg1);
            b6_ag4r2l_p(l, lid0, t127, arg2, t128);
        }
        return;
    }
    if any(&live) {
        let t129 = [vdupq_n_u32(4294967295u32); 2];
        x72 = t129;
        let t130: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t131 = [vdupq_n_u32(x32); 2];
        let t132: [uint32x4_t; 2] = [vcltq_u32(t130[0], t131[0]), vcltq_u32(t130[1], t131[1])];
        let t133: [uint32x4_t; 2] = [vandq_u32(live[0], t132[0]), vandq_u32(live[1], t132[1])];
        if any(&t133) {
            let t134 = b8_r0r1g_p(l, lid0, t133, arg0, arg1);
            x80 = [vbslq_u32(t133[0], t134[0], x80[0]), vbslq_u32(t133[1], t134[1], x80[1]), vbslq_u32(t133[0], t134[2], x80[2]), vbslq_u32(t133[1], t134[3], x80[3])];
            let t135: u64 = (1u64).wrapping_shl((32u64) as u32);
            let t136 = [vdupq_n_u32((t135) as u32); 2];
            let t137 = [vdupq_n_u32(((t135) >> 32) as u32); 2];
            let t138: [uint32x4_t; 4] = [vandq_u32(x80[0], t136[0]), vandq_u32(x80[1], t136[1]), vandq_u32(x80[2], t137[0]), vandq_u32(x80[3], t137[1])];
            let t139: u64 = 0u64;
            let t140 = [vdupq_n_u32((t139) as u32); 2];
            let t141: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t138[0], t140[0]), vceqq_u32(t138[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t138[1], t140[1]), vceqq_u32(t138[3], vdupq_n_u32(0)))];
            let t142: [uint32x4_t; 2] = [vmvnq_u32(t141[0]), vmvnq_u32(t141[1])];
            let mut t143: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t144: [uint32x4_t; 2] = [vandq_u32(t133[0], t142[0]), vandq_u32(t133[1], t142[1])];
            let t145: [uint32x4_t; 2] = [vbicq_u32(t133[0], t142[0]), vbicq_u32(t133[1], t142[1])];
            {
                let t146: [uint32x4_t; 2] = [x80[0], x80[1]];
                let t147 = [vdupq_n_u32(2147483648u32); 2];
                let t148: [uint32x4_t; 2] = [vorrq_u32(t147[0], t146[0]), vorrq_u32(t147[1], t146[1])];
                t143 = [vbslq_u32(t144[0], t148[0], t143[0]), vbslq_u32(t144[1], t148[1], t143[1])];
            }
            {
                let t149: [uint32x4_t; 2] = [x80[0], x80[1]];
                t143 = [vbslq_u32(t145[0], t149[0], t143[0]), vbslq_u32(t145[1], t149[1], t143[1])];
            }
            x72 = [vbslq_u32(t133[0], t143[0], x72[0]), vbslq_u32(t133[1], t143[1], x72[1])];
            b6_ag4r2l_p(l, lid0, t133, arg2, x72);
        }
        b9_r3gi_p(l, lid0, live, arg3, x56);
        let t150 = [vdupq_n_u32(x32); 2];
        let t151: [uint32x4_t; 2] = [vcgeq_u32(t130[0], t150[0]), vcgeq_u32(t130[1], t150[1])];
        let t152: [uint32x4_t; 2] = [vandq_u32(live[0], t151[0]), vandq_u32(live[1], t151[1])];
        {
            live = [vbicq_u32(live[0], t152[0]), vbicq_u32(live[1], t152[1])];
        }
        if any(&live) {
            let t153: u64 = ((0u32) as u64);
            let t154 = [vdupq_n_u32((t153) as u32); 2];
            let t155: [uint32x4_t; 4] = [t154[0], t154[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            x80 = t155;
            let mut t156 = live;
            loop {
                // Lanes part: each its own rounds.
                let t158: [uint32x4_t; 2] = [x80[0], x80[1]];
                let t159 = [vdupq_n_u32(7u32); 2];
                let t160: [uint32x4_t; 2] = [vcltq_u32(t158[0], t159[0]), vcltq_u32(t158[1], t159[1])];
                let t161 = [vdupq_n_u32(2147483648u32); 2];
                let t162: [uint32x4_t; 2] = [vandq_u32(x72[0], t161[0]), vandq_u32(x72[1], t161[1])];
                let t163 = [vdupq_n_u32(0u32); 2];
                let t164: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t162[0], t163[0])), vmvnq_u32(vceqq_u32(t162[1], t163[1]))];
                let t165 = [vdupq_n_u32(4294967294u32); 2];
                let t166: [uint32x4_t; 2] = [vcltq_u32(x72[0], t165[0]), vcltq_u32(x72[1], t165[1])];
                let t167: [uint32x4_t; 2] = [vandq_u32(t164[0], t166[0]), vandq_u32(t164[1], t166[1])];
                let t168: [uint32x4_t; 2] = [vandq_u32(t160[0], t167[0]), vandq_u32(t160[1], t167[1])];
                t156 = [vandq_u32(t156[0], t168[0]), vandq_u32(t156[1], t168[1])];
                if !(any(&t156)) { break; }
                let t170 = [vdupq_n_u32(2147483647u32); 2];
                let t171: [uint32x4_t; 2] = [vandq_u32(x72[0], t170[0]), vandq_u32(x72[1], t170[1])];
                x88 = t171;
                x96 = t165;
                let t172 = b10_r3ll_p(l, lid0, t156, arg3, x88, x48);
                let t173: [uint32x4_t; 2] = [vandq_u32(t156[0], t172[0]), vandq_u32(t156[1], t172[1])];
                if any(&t173) {
                    let t174 = b11_al4r2_p(l, lid0, t173, arg2, x88);
                    x96 = [vbslq_u32(t173[0], t174[0], x96[0]), vbslq_u32(t173[1], t174[1], x96[1])];
                }
                let t175: [uint32x4_t; 2] = [vceqq_u32(x96[0], t165[0]), vceqq_u32(x96[1], t165[1])];
                let t176: [uint32x4_t; 2] = [vandq_u32(t156[0], t175[0]), vandq_u32(t156[1], t175[1])];
                if any(&t176) {
                    let t177 = b8_r0r1l_p(l, lid0, t176, arg0, arg1, x88);
                    x104 = t177;
                    let t178: u64 = (1u64).wrapping_shl((32u64) as u32);
                    let t179 = [vdupq_n_u32((t178) as u32); 2];
                    let t180 = [vdupq_n_u32(((t178) >> 32) as u32); 2];
                    let t181: [uint32x4_t; 4] = [vandq_u32(x104[0], t179[0]), vandq_u32(x104[1], t179[1]), vandq_u32(x104[2], t180[0]), vandq_u32(x104[3], t180[1])];
                    let t182: u64 = 0u64;
                    let t183 = [vdupq_n_u32((t182) as u32); 2];
                    let t184: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t181[0], t183[0]), vceqq_u32(t181[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t181[1], t183[1]), vceqq_u32(t181[3], vdupq_n_u32(0)))];
                    let t185: [uint32x4_t; 2] = [vmvnq_u32(t184[0]), vmvnq_u32(t184[1])];
                    let mut t186: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                    let t187: [uint32x4_t; 2] = [vandq_u32(t176[0], t185[0]), vandq_u32(t176[1], t185[1])];
                    let t188: [uint32x4_t; 2] = [vbicq_u32(t176[0], t185[0]), vbicq_u32(t176[1], t185[1])];
                    {
                        let t189: [uint32x4_t; 2] = [x104[0], x104[1]];
                        let t190: [uint32x4_t; 2] = [vorrq_u32(t161[0], t189[0]), vorrq_u32(t161[1], t189[1])];
                        t186 = [vbslq_u32(t187[0], t190[0], t186[0]), vbslq_u32(t187[1], t190[1], t186[1])];
                    }
                    {
                        let t191: [uint32x4_t; 2] = [x104[0], x104[1]];
                        t186 = [vbslq_u32(t188[0], t191[0], t186[0]), vbslq_u32(t188[1], t191[1], t186[1])];
                    }
                    x96 = [vbslq_u32(t176[0], t186[0], x96[0]), vbslq_u32(t176[1], t186[1], x96[1])];
                }
                x72 = [vbslq_u32(t156[0], x96[0], x72[0]), vbslq_u32(t156[1], x96[1], x72[1])];
                let t192: [uint32x4_t; 2] = [vandq_u32(t156[0], live[0]), vandq_u32(t156[1], live[1])];
                let t193: [uint32x4_t; 2] = [x80[0], x80[1]];
                let t194 = [vdupq_n_u32(1u32); 2];
                let t195: [uint32x4_t; 2] = [vaddq_u32(t193[0], t194[0]), vaddq_u32(t193[1], t194[1])];
                let t196 = x80;
                let t197: [uint32x4_t; 4] = [t195[0], t195[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                x80 = t197;
            }
            let t198 = [vdupq_n_u32(2147483648u32); 2];
            let t199: [uint32x4_t; 2] = [vandq_u32(x72[0], t198[0]), vandq_u32(x72[1], t198[1])];
            let t200 = [vdupq_n_u32(0u32); 2];
            let t201: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t199[0], t200[0])), vmvnq_u32(vceqq_u32(t199[1], t200[1]))];
            let t202 = [vdupq_n_u32(4294967294u32); 2];
            let t203: [uint32x4_t; 2] = [vcltq_u32(x72[0], t202[0]), vcltq_u32(x72[1], t202[1])];
            let t204: [uint32x4_t; 2] = [vandq_u32(t201[0], t203[0]), vandq_u32(t201[1], t203[1])];
            let t205: [uint32x4_t; 2] = [vandq_u32(live[0], t204[0]), vandq_u32(live[1], t204[1])];
            if any(&t205) {
                let t206 = [vdupq_n_u32(2147483647u32); 2];
                let t207: [uint32x4_t; 2] = [vandq_u32(x72[0], t206[0]), vandq_u32(x72[1], t206[1])];
                let t208 = b7_r0r1l_p(l, lid0, t205, arg0, arg1, t207);
                x72 = [vbslq_u32(t205[0], t208[0], x72[0]), vbslq_u32(t205[1], t208[1], x72[1])];
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
            let t209 = b7_r0r1g_f(l, lid0, arg0, arg1);
            b6_ag4r2l_f(l, lid0, arg2, t209);
        } else {
            // Lanes part: the chunk's lanes on both sides of the bound.
            let t210: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
            let t211 = [vdupq_n_u32(x32); 2];
            let t212: [uint32x4_t; 2] = [vcltq_u32(t210[0], t211[0]), vcltq_u32(t210[1], t211[1])];
            let t213: [uint32x4_t; 2] = [vandq_u32(live[0], t212[0]), vandq_u32(live[1], t212[1])];
            if any(&t213) {
                let t214 = b7_r0r1g_p(l, lid0, t213, arg0, arg1);
                b6_ag4r2l_p(l, lid0, t213, arg2, t214);
            }
        }
        return;
    }
    if any(&live) {
        let t215 = [vdupq_n_u32(4294967295u32); 2];
        x72 = t215;
        let t216: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t217 = [vdupq_n_u32(x32); 2];
        let t218: [uint32x4_t; 2] = [vcltq_u32(t216[0], t217[0]), vcltq_u32(t216[1], t217[1])];
        let t219: [uint32x4_t; 2] = [vandq_u32(live[0], t218[0]), vandq_u32(live[1], t218[1])];
        if any(&t219) {
            let t220 = b8_r0r1g_p(l, lid0, t219, arg0, arg1);
            x80 = [vbslq_u32(t219[0], t220[0], x80[0]), vbslq_u32(t219[1], t220[1], x80[1]), vbslq_u32(t219[0], t220[2], x80[2]), vbslq_u32(t219[1], t220[3], x80[3])];
            let t221: u64 = (1u64).wrapping_shl((32u64) as u32);
            let t222 = [vdupq_n_u32((t221) as u32); 2];
            let t223 = [vdupq_n_u32(((t221) >> 32) as u32); 2];
            let t224: [uint32x4_t; 4] = [vandq_u32(x80[0], t222[0]), vandq_u32(x80[1], t222[1]), vandq_u32(x80[2], t223[0]), vandq_u32(x80[3], t223[1])];
            let t225: u64 = 0u64;
            let t226 = [vdupq_n_u32((t225) as u32); 2];
            let t227: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t224[0], t226[0]), vceqq_u32(t224[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t224[1], t226[1]), vceqq_u32(t224[3], vdupq_n_u32(0)))];
            let t228: [uint32x4_t; 2] = [vmvnq_u32(t227[0]), vmvnq_u32(t227[1])];
            let mut t229: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t230: [uint32x4_t; 2] = [vandq_u32(t219[0], t228[0]), vandq_u32(t219[1], t228[1])];
            let t231: [uint32x4_t; 2] = [vbicq_u32(t219[0], t228[0]), vbicq_u32(t219[1], t228[1])];
            {
                let t232: [uint32x4_t; 2] = [x80[0], x80[1]];
                let t233 = [vdupq_n_u32(2147483648u32); 2];
                let t234: [uint32x4_t; 2] = [vorrq_u32(t233[0], t232[0]), vorrq_u32(t233[1], t232[1])];
                t229 = [vbslq_u32(t230[0], t234[0], t229[0]), vbslq_u32(t230[1], t234[1], t229[1])];
            }
            {
                let t235: [uint32x4_t; 2] = [x80[0], x80[1]];
                t229 = [vbslq_u32(t231[0], t235[0], t229[0]), vbslq_u32(t231[1], t235[1], t229[1])];
            }
            x72 = [vbslq_u32(t219[0], t229[0], x72[0]), vbslq_u32(t219[1], t229[1], x72[1])];
            b6_ag4r2l_p(l, lid0, t219, arg2, x72);
        }
        b9_r3gi_p(l, lid0, live, arg3, x56);
        let t236 = [vdupq_n_u32(x32); 2];
        let t237: [uint32x4_t; 2] = [vcgeq_u32(t216[0], t236[0]), vcgeq_u32(t216[1], t236[1])];
        let t238: [uint32x4_t; 2] = [vandq_u32(live[0], t237[0]), vandq_u32(live[1], t237[1])];
        {
            live = [vbicq_u32(live[0], t238[0]), vbicq_u32(live[1], t238[1])];
        }
        if any(&live) {
            let t239: u64 = ((0u32) as u64);
            let t240 = [vdupq_n_u32((t239) as u32); 2];
            let t241: [uint32x4_t; 4] = [t240[0], t240[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            x80 = t241;
            let mut t242 = live;
            loop {
                // Lanes part: each its own rounds.
                let t244: [uint32x4_t; 2] = [x80[0], x80[1]];
                let t245 = [vdupq_n_u32(7u32); 2];
                let t246: [uint32x4_t; 2] = [vcltq_u32(t244[0], t245[0]), vcltq_u32(t244[1], t245[1])];
                let t247 = [vdupq_n_u32(2147483648u32); 2];
                let t248: [uint32x4_t; 2] = [vandq_u32(x72[0], t247[0]), vandq_u32(x72[1], t247[1])];
                let t249 = [vdupq_n_u32(0u32); 2];
                let t250: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t248[0], t249[0])), vmvnq_u32(vceqq_u32(t248[1], t249[1]))];
                let t251 = [vdupq_n_u32(4294967294u32); 2];
                let t252: [uint32x4_t; 2] = [vcltq_u32(x72[0], t251[0]), vcltq_u32(x72[1], t251[1])];
                let t253: [uint32x4_t; 2] = [vandq_u32(t250[0], t252[0]), vandq_u32(t250[1], t252[1])];
                let t254: [uint32x4_t; 2] = [vandq_u32(t246[0], t253[0]), vandq_u32(t246[1], t253[1])];
                t242 = [vandq_u32(t242[0], t254[0]), vandq_u32(t242[1], t254[1])];
                if !(any(&t242)) { break; }
                let t256 = [vdupq_n_u32(2147483647u32); 2];
                let t257: [uint32x4_t; 2] = [vandq_u32(x72[0], t256[0]), vandq_u32(x72[1], t256[1])];
                x88 = t257;
                x96 = t251;
                let t258 = b10_r3ll_p(l, lid0, t242, arg3, x88, x48);
                let t259: [uint32x4_t; 2] = [vandq_u32(t242[0], t258[0]), vandq_u32(t242[1], t258[1])];
                if any(&t259) {
                    let t260 = b11_al4r2_p(l, lid0, t259, arg2, x88);
                    x96 = [vbslq_u32(t259[0], t260[0], x96[0]), vbslq_u32(t259[1], t260[1], x96[1])];
                }
                let t261: [uint32x4_t; 2] = [vceqq_u32(x96[0], t251[0]), vceqq_u32(x96[1], t251[1])];
                let t262: [uint32x4_t; 2] = [vandq_u32(t242[0], t261[0]), vandq_u32(t242[1], t261[1])];
                if any(&t262) {
                    let t263 = b8_r0r1l_p(l, lid0, t262, arg0, arg1, x88);
                    x104 = t263;
                    let t264: u64 = (1u64).wrapping_shl((32u64) as u32);
                    let t265 = [vdupq_n_u32((t264) as u32); 2];
                    let t266 = [vdupq_n_u32(((t264) >> 32) as u32); 2];
                    let t267: [uint32x4_t; 4] = [vandq_u32(x104[0], t265[0]), vandq_u32(x104[1], t265[1]), vandq_u32(x104[2], t266[0]), vandq_u32(x104[3], t266[1])];
                    let t268: u64 = 0u64;
                    let t269 = [vdupq_n_u32((t268) as u32); 2];
                    let t270: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t267[0], t269[0]), vceqq_u32(t267[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t267[1], t269[1]), vceqq_u32(t267[3], vdupq_n_u32(0)))];
                    let t271: [uint32x4_t; 2] = [vmvnq_u32(t270[0]), vmvnq_u32(t270[1])];
                    let mut t272: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                    let t273: [uint32x4_t; 2] = [vandq_u32(t262[0], t271[0]), vandq_u32(t262[1], t271[1])];
                    let t274: [uint32x4_t; 2] = [vbicq_u32(t262[0], t271[0]), vbicq_u32(t262[1], t271[1])];
                    {
                        let t275: [uint32x4_t; 2] = [x104[0], x104[1]];
                        let t276: [uint32x4_t; 2] = [vorrq_u32(t247[0], t275[0]), vorrq_u32(t247[1], t275[1])];
                        t272 = [vbslq_u32(t273[0], t276[0], t272[0]), vbslq_u32(t273[1], t276[1], t272[1])];
                    }
                    {
                        let t277: [uint32x4_t; 2] = [x104[0], x104[1]];
                        t272 = [vbslq_u32(t274[0], t277[0], t272[0]), vbslq_u32(t274[1], t277[1], t272[1])];
                    }
                    x96 = [vbslq_u32(t262[0], t272[0], x96[0]), vbslq_u32(t262[1], t272[1], x96[1])];
                }
                x72 = [vbslq_u32(t242[0], x96[0], x72[0]), vbslq_u32(t242[1], x96[1], x72[1])];
                let t278: [uint32x4_t; 2] = [vandq_u32(t242[0], live[0]), vandq_u32(t242[1], live[1])];
                let t279: [uint32x4_t; 2] = [x80[0], x80[1]];
                let t280 = [vdupq_n_u32(1u32); 2];
                let t281: [uint32x4_t; 2] = [vaddq_u32(t279[0], t280[0]), vaddq_u32(t279[1], t280[1])];
                let t282 = x80;
                let t283: [uint32x4_t; 4] = [t281[0], t281[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                x80 = t283;
            }
            let t284 = [vdupq_n_u32(2147483648u32); 2];
            let t285: [uint32x4_t; 2] = [vandq_u32(x72[0], t284[0]), vandq_u32(x72[1], t284[1])];
            let t286 = [vdupq_n_u32(0u32); 2];
            let t287: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t285[0], t286[0])), vmvnq_u32(vceqq_u32(t285[1], t286[1]))];
            let t288 = [vdupq_n_u32(4294967294u32); 2];
            let t289: [uint32x4_t; 2] = [vcltq_u32(x72[0], t288[0]), vcltq_u32(x72[1], t288[1])];
            let t290: [uint32x4_t; 2] = [vandq_u32(t287[0], t289[0]), vandq_u32(t287[1], t289[1])];
            let t291: [uint32x4_t; 2] = [vandq_u32(live[0], t290[0]), vandq_u32(live[1], t290[1])];
            if any(&t291) {
                let t292 = [vdupq_n_u32(2147483647u32); 2];
                let t293: [uint32x4_t; 2] = [vandq_u32(x72[0], t292[0]), vandq_u32(x72[1], t292[1])];
                let t294 = b7_r0r1l_p(l, lid0, t291, arg0, arg1, t293);
                x72 = [vbslq_u32(t291[0], t294[0], x72[0]), vbslq_u32(t291[1], t294[1], x72[1])];
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
        let t295: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t296 = [vdupq_n_u32(x32); 2];
        let t297: [uint32x4_t; 2] = [vcltq_u32(t295[0], t296[0]), vcltq_u32(t295[1], t296[1])];
        let t298: [uint32x4_t; 2] = [vandq_u32(live[0], t297[0]), vandq_u32(live[1], t297[1])];
        if any(&t298) {
            let t299 = b7_r0r1g_p(l, lid0, t298, arg0, arg1);
            b6_ag4r2l_p(l, lid0, t298, arg2, t299);
        }
        return;
    }
    if any(&live) {
        let t300 = [vdupq_n_u32(4294967295u32); 2];
        x72 = t300;
        let t301: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t302 = [vdupq_n_u32(x32); 2];
        let t303: [uint32x4_t; 2] = [vcltq_u32(t301[0], t302[0]), vcltq_u32(t301[1], t302[1])];
        let t304: [uint32x4_t; 2] = [vandq_u32(live[0], t303[0]), vandq_u32(live[1], t303[1])];
        if any(&t304) {
            let t305 = b8_r0r1g_p(l, lid0, t304, arg0, arg1);
            x80 = [vbslq_u32(t304[0], t305[0], x80[0]), vbslq_u32(t304[1], t305[1], x80[1]), vbslq_u32(t304[0], t305[2], x80[2]), vbslq_u32(t304[1], t305[3], x80[3])];
            let t306: u64 = (1u64).wrapping_shl((32u64) as u32);
            let t307 = [vdupq_n_u32((t306) as u32); 2];
            let t308 = [vdupq_n_u32(((t306) >> 32) as u32); 2];
            let t309: [uint32x4_t; 4] = [vandq_u32(x80[0], t307[0]), vandq_u32(x80[1], t307[1]), vandq_u32(x80[2], t308[0]), vandq_u32(x80[3], t308[1])];
            let t310: u64 = 0u64;
            let t311 = [vdupq_n_u32((t310) as u32); 2];
            let t312: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t309[0], t311[0]), vceqq_u32(t309[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t309[1], t311[1]), vceqq_u32(t309[3], vdupq_n_u32(0)))];
            let t313: [uint32x4_t; 2] = [vmvnq_u32(t312[0]), vmvnq_u32(t312[1])];
            let mut t314: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t315: [uint32x4_t; 2] = [vandq_u32(t304[0], t313[0]), vandq_u32(t304[1], t313[1])];
            let t316: [uint32x4_t; 2] = [vbicq_u32(t304[0], t313[0]), vbicq_u32(t304[1], t313[1])];
            {
                let t317: [uint32x4_t; 2] = [x80[0], x80[1]];
                let t318 = [vdupq_n_u32(2147483648u32); 2];
                let t319: [uint32x4_t; 2] = [vorrq_u32(t318[0], t317[0]), vorrq_u32(t318[1], t317[1])];
                t314 = [vbslq_u32(t315[0], t319[0], t314[0]), vbslq_u32(t315[1], t319[1], t314[1])];
            }
            {
                let t320: [uint32x4_t; 2] = [x80[0], x80[1]];
                t314 = [vbslq_u32(t316[0], t320[0], t314[0]), vbslq_u32(t316[1], t320[1], t314[1])];
            }
            x72 = [vbslq_u32(t304[0], t314[0], x72[0]), vbslq_u32(t304[1], t314[1], x72[1])];
            b6_ag4r2l_p(l, lid0, t304, arg2, x72);
        }
        b9_r3gl_p(l, lid0, live, arg3, x56);
        let t321 = [vdupq_n_u32(x32); 2];
        let t322: [uint32x4_t; 2] = [vcgeq_u32(t301[0], t321[0]), vcgeq_u32(t301[1], t321[1])];
        let t323: [uint32x4_t; 2] = [vandq_u32(live[0], t322[0]), vandq_u32(live[1], t322[1])];
        {
            live = [vbicq_u32(live[0], t323[0]), vbicq_u32(live[1], t323[1])];
        }
        if any(&live) {
            let t324: u64 = ((0u32) as u64);
            let t325 = [vdupq_n_u32((t324) as u32); 2];
            let t326: [uint32x4_t; 4] = [t325[0], t325[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            x80 = t326;
            let mut t327 = live;
            loop {
                // Lanes part: each its own rounds.
                let t329: [uint32x4_t; 2] = [x80[0], x80[1]];
                let t330 = [vdupq_n_u32(7u32); 2];
                let t331: [uint32x4_t; 2] = [vcltq_u32(t329[0], t330[0]), vcltq_u32(t329[1], t330[1])];
                let t332 = [vdupq_n_u32(2147483648u32); 2];
                let t333: [uint32x4_t; 2] = [vandq_u32(x72[0], t332[0]), vandq_u32(x72[1], t332[1])];
                let t334 = [vdupq_n_u32(0u32); 2];
                let t335: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t333[0], t334[0])), vmvnq_u32(vceqq_u32(t333[1], t334[1]))];
                let t336 = [vdupq_n_u32(4294967294u32); 2];
                let t337: [uint32x4_t; 2] = [vcltq_u32(x72[0], t336[0]), vcltq_u32(x72[1], t336[1])];
                let t338: [uint32x4_t; 2] = [vandq_u32(t335[0], t337[0]), vandq_u32(t335[1], t337[1])];
                let t339: [uint32x4_t; 2] = [vandq_u32(t331[0], t338[0]), vandq_u32(t331[1], t338[1])];
                t327 = [vandq_u32(t327[0], t339[0]), vandq_u32(t327[1], t339[1])];
                if !(any(&t327)) { break; }
                let t341 = [vdupq_n_u32(2147483647u32); 2];
                let t342: [uint32x4_t; 2] = [vandq_u32(x72[0], t341[0]), vandq_u32(x72[1], t341[1])];
                x88 = t342;
                x96 = t336;
                let t343 = b10_r3ll_p(l, lid0, t327, arg3, x88, x48);
                let t344: [uint32x4_t; 2] = [vandq_u32(t327[0], t343[0]), vandq_u32(t327[1], t343[1])];
                if any(&t344) {
                    let t345 = b11_al4r2_p(l, lid0, t344, arg2, x88);
                    x96 = [vbslq_u32(t344[0], t345[0], x96[0]), vbslq_u32(t344[1], t345[1], x96[1])];
                }
                let t346: [uint32x4_t; 2] = [vceqq_u32(x96[0], t336[0]), vceqq_u32(x96[1], t336[1])];
                let t347: [uint32x4_t; 2] = [vandq_u32(t327[0], t346[0]), vandq_u32(t327[1], t346[1])];
                if any(&t347) {
                    let t348 = b8_r0r1l_p(l, lid0, t347, arg0, arg1, x88);
                    x104 = t348;
                    let t349: u64 = (1u64).wrapping_shl((32u64) as u32);
                    let t350 = [vdupq_n_u32((t349) as u32); 2];
                    let t351 = [vdupq_n_u32(((t349) >> 32) as u32); 2];
                    let t352: [uint32x4_t; 4] = [vandq_u32(x104[0], t350[0]), vandq_u32(x104[1], t350[1]), vandq_u32(x104[2], t351[0]), vandq_u32(x104[3], t351[1])];
                    let t353: u64 = 0u64;
                    let t354 = [vdupq_n_u32((t353) as u32); 2];
                    let t355: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t352[0], t354[0]), vceqq_u32(t352[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t352[1], t354[1]), vceqq_u32(t352[3], vdupq_n_u32(0)))];
                    let t356: [uint32x4_t; 2] = [vmvnq_u32(t355[0]), vmvnq_u32(t355[1])];
                    let mut t357: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                    let t358: [uint32x4_t; 2] = [vandq_u32(t347[0], t356[0]), vandq_u32(t347[1], t356[1])];
                    let t359: [uint32x4_t; 2] = [vbicq_u32(t347[0], t356[0]), vbicq_u32(t347[1], t356[1])];
                    {
                        let t360: [uint32x4_t; 2] = [x104[0], x104[1]];
                        let t361: [uint32x4_t; 2] = [vorrq_u32(t332[0], t360[0]), vorrq_u32(t332[1], t360[1])];
                        t357 = [vbslq_u32(t358[0], t361[0], t357[0]), vbslq_u32(t358[1], t361[1], t357[1])];
                    }
                    {
                        let t362: [uint32x4_t; 2] = [x104[0], x104[1]];
                        t357 = [vbslq_u32(t359[0], t362[0], t357[0]), vbslq_u32(t359[1], t362[1], t357[1])];
                    }
                    x96 = [vbslq_u32(t347[0], t357[0], x96[0]), vbslq_u32(t347[1], t357[1], x96[1])];
                }
                x72 = [vbslq_u32(t327[0], x96[0], x72[0]), vbslq_u32(t327[1], x96[1], x72[1])];
                let t363: [uint32x4_t; 2] = [vandq_u32(t327[0], live[0]), vandq_u32(t327[1], live[1])];
                let t364: [uint32x4_t; 2] = [x80[0], x80[1]];
                let t365 = [vdupq_n_u32(1u32); 2];
                let t366: [uint32x4_t; 2] = [vaddq_u32(t364[0], t365[0]), vaddq_u32(t364[1], t365[1])];
                let t367 = x80;
                let t368: [uint32x4_t; 4] = [t366[0], t366[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                x80 = t368;
            }
            let t369 = [vdupq_n_u32(2147483648u32); 2];
            let t370: [uint32x4_t; 2] = [vandq_u32(x72[0], t369[0]), vandq_u32(x72[1], t369[1])];
            let t371 = [vdupq_n_u32(0u32); 2];
            let t372: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t370[0], t371[0])), vmvnq_u32(vceqq_u32(t370[1], t371[1]))];
            let t373 = [vdupq_n_u32(4294967294u32); 2];
            let t374: [uint32x4_t; 2] = [vcltq_u32(x72[0], t373[0]), vcltq_u32(x72[1], t373[1])];
            let t375: [uint32x4_t; 2] = [vandq_u32(t372[0], t374[0]), vandq_u32(t372[1], t374[1])];
            let t376: [uint32x4_t; 2] = [vandq_u32(live[0], t375[0]), vandq_u32(live[1], t375[1])];
            if any(&t376) {
                let t377 = [vdupq_n_u32(2147483647u32); 2];
                let t378: [uint32x4_t; 2] = [vandq_u32(x72[0], t377[0]), vandq_u32(x72[1], t377[1])];
                let t379 = b7_r0r1l_p(l, lid0, t376, arg0, arg1, t378);
                x72 = [vbslq_u32(t376[0], t379[0], x72[0]), vbslq_u32(t376[1], t379[1], x72[1])];
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
            let t380 = b7_r0r1g_f(l, lid0, arg0, arg1);
            b6_ag4r2l_f(l, lid0, arg2, t380);
        } else {
            // Lanes part: the chunk's lanes on both sides of the bound.
            let t381: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
            let t382 = [vdupq_n_u32(x32); 2];
            let t383: [uint32x4_t; 2] = [vcltq_u32(t381[0], t382[0]), vcltq_u32(t381[1], t382[1])];
            let t384: [uint32x4_t; 2] = [vandq_u32(live[0], t383[0]), vandq_u32(live[1], t383[1])];
            if any(&t384) {
                let t385 = b7_r0r1g_p(l, lid0, t384, arg0, arg1);
                b6_ag4r2l_p(l, lid0, t384, arg2, t385);
            }
        }
        return;
    }
    if any(&live) {
        let t386 = [vdupq_n_u32(4294967295u32); 2];
        x72 = t386;
        let t387: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t388 = [vdupq_n_u32(x32); 2];
        let t389: [uint32x4_t; 2] = [vcltq_u32(t387[0], t388[0]), vcltq_u32(t387[1], t388[1])];
        let t390: [uint32x4_t; 2] = [vandq_u32(live[0], t389[0]), vandq_u32(live[1], t389[1])];
        if any(&t390) {
            let t391 = b8_r0r1g_p(l, lid0, t390, arg0, arg1);
            x80 = [vbslq_u32(t390[0], t391[0], x80[0]), vbslq_u32(t390[1], t391[1], x80[1]), vbslq_u32(t390[0], t391[2], x80[2]), vbslq_u32(t390[1], t391[3], x80[3])];
            let t392: u64 = (1u64).wrapping_shl((32u64) as u32);
            let t393 = [vdupq_n_u32((t392) as u32); 2];
            let t394 = [vdupq_n_u32(((t392) >> 32) as u32); 2];
            let t395: [uint32x4_t; 4] = [vandq_u32(x80[0], t393[0]), vandq_u32(x80[1], t393[1]), vandq_u32(x80[2], t394[0]), vandq_u32(x80[3], t394[1])];
            let t396: u64 = 0u64;
            let t397 = [vdupq_n_u32((t396) as u32); 2];
            let t398: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t395[0], t397[0]), vceqq_u32(t395[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t395[1], t397[1]), vceqq_u32(t395[3], vdupq_n_u32(0)))];
            let t399: [uint32x4_t; 2] = [vmvnq_u32(t398[0]), vmvnq_u32(t398[1])];
            let mut t400: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t401: [uint32x4_t; 2] = [vandq_u32(t390[0], t399[0]), vandq_u32(t390[1], t399[1])];
            let t402: [uint32x4_t; 2] = [vbicq_u32(t390[0], t399[0]), vbicq_u32(t390[1], t399[1])];
            {
                let t403: [uint32x4_t; 2] = [x80[0], x80[1]];
                let t404 = [vdupq_n_u32(2147483648u32); 2];
                let t405: [uint32x4_t; 2] = [vorrq_u32(t404[0], t403[0]), vorrq_u32(t404[1], t403[1])];
                t400 = [vbslq_u32(t401[0], t405[0], t400[0]), vbslq_u32(t401[1], t405[1], t400[1])];
            }
            {
                let t406: [uint32x4_t; 2] = [x80[0], x80[1]];
                t400 = [vbslq_u32(t402[0], t406[0], t400[0]), vbslq_u32(t402[1], t406[1], t400[1])];
            }
            x72 = [vbslq_u32(t390[0], t400[0], x72[0]), vbslq_u32(t390[1], t400[1], x72[1])];
            b6_ag4r2l_p(l, lid0, t390, arg2, x72);
        }
        b9_r3gl_p(l, lid0, live, arg3, x56);
        let t407 = [vdupq_n_u32(x32); 2];
        let t408: [uint32x4_t; 2] = [vcgeq_u32(t387[0], t407[0]), vcgeq_u32(t387[1], t407[1])];
        let t409: [uint32x4_t; 2] = [vandq_u32(live[0], t408[0]), vandq_u32(live[1], t408[1])];
        {
            live = [vbicq_u32(live[0], t409[0]), vbicq_u32(live[1], t409[1])];
        }
        if any(&live) {
            let t410: u64 = ((0u32) as u64);
            let t411 = [vdupq_n_u32((t410) as u32); 2];
            let t412: [uint32x4_t; 4] = [t411[0], t411[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            x80 = t412;
            let mut t413 = live;
            loop {
                // Lanes part: each its own rounds.
                let t415: [uint32x4_t; 2] = [x80[0], x80[1]];
                let t416 = [vdupq_n_u32(7u32); 2];
                let t417: [uint32x4_t; 2] = [vcltq_u32(t415[0], t416[0]), vcltq_u32(t415[1], t416[1])];
                let t418 = [vdupq_n_u32(2147483648u32); 2];
                let t419: [uint32x4_t; 2] = [vandq_u32(x72[0], t418[0]), vandq_u32(x72[1], t418[1])];
                let t420 = [vdupq_n_u32(0u32); 2];
                let t421: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t419[0], t420[0])), vmvnq_u32(vceqq_u32(t419[1], t420[1]))];
                let t422 = [vdupq_n_u32(4294967294u32); 2];
                let t423: [uint32x4_t; 2] = [vcltq_u32(x72[0], t422[0]), vcltq_u32(x72[1], t422[1])];
                let t424: [uint32x4_t; 2] = [vandq_u32(t421[0], t423[0]), vandq_u32(t421[1], t423[1])];
                let t425: [uint32x4_t; 2] = [vandq_u32(t417[0], t424[0]), vandq_u32(t417[1], t424[1])];
                t413 = [vandq_u32(t413[0], t425[0]), vandq_u32(t413[1], t425[1])];
                if !(any(&t413)) { break; }
                let t427 = [vdupq_n_u32(2147483647u32); 2];
                let t428: [uint32x4_t; 2] = [vandq_u32(x72[0], t427[0]), vandq_u32(x72[1], t427[1])];
                x88 = t428;
                x96 = t422;
                let t429 = b10_r3ll_p(l, lid0, t413, arg3, x88, x48);
                let t430: [uint32x4_t; 2] = [vandq_u32(t413[0], t429[0]), vandq_u32(t413[1], t429[1])];
                if any(&t430) {
                    let t431 = b11_al4r2_p(l, lid0, t430, arg2, x88);
                    x96 = [vbslq_u32(t430[0], t431[0], x96[0]), vbslq_u32(t430[1], t431[1], x96[1])];
                }
                let t432: [uint32x4_t; 2] = [vceqq_u32(x96[0], t422[0]), vceqq_u32(x96[1], t422[1])];
                let t433: [uint32x4_t; 2] = [vandq_u32(t413[0], t432[0]), vandq_u32(t413[1], t432[1])];
                if any(&t433) {
                    let t434 = b8_r0r1l_p(l, lid0, t433, arg0, arg1, x88);
                    x104 = t434;
                    let t435: u64 = (1u64).wrapping_shl((32u64) as u32);
                    let t436 = [vdupq_n_u32((t435) as u32); 2];
                    let t437 = [vdupq_n_u32(((t435) >> 32) as u32); 2];
                    let t438: [uint32x4_t; 4] = [vandq_u32(x104[0], t436[0]), vandq_u32(x104[1], t436[1]), vandq_u32(x104[2], t437[0]), vandq_u32(x104[3], t437[1])];
                    let t439: u64 = 0u64;
                    let t440 = [vdupq_n_u32((t439) as u32); 2];
                    let t441: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t438[0], t440[0]), vceqq_u32(t438[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t438[1], t440[1]), vceqq_u32(t438[3], vdupq_n_u32(0)))];
                    let t442: [uint32x4_t; 2] = [vmvnq_u32(t441[0]), vmvnq_u32(t441[1])];
                    let mut t443: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                    let t444: [uint32x4_t; 2] = [vandq_u32(t433[0], t442[0]), vandq_u32(t433[1], t442[1])];
                    let t445: [uint32x4_t; 2] = [vbicq_u32(t433[0], t442[0]), vbicq_u32(t433[1], t442[1])];
                    {
                        let t446: [uint32x4_t; 2] = [x104[0], x104[1]];
                        let t447: [uint32x4_t; 2] = [vorrq_u32(t418[0], t446[0]), vorrq_u32(t418[1], t446[1])];
                        t443 = [vbslq_u32(t444[0], t447[0], t443[0]), vbslq_u32(t444[1], t447[1], t443[1])];
                    }
                    {
                        let t448: [uint32x4_t; 2] = [x104[0], x104[1]];
                        t443 = [vbslq_u32(t445[0], t448[0], t443[0]), vbslq_u32(t445[1], t448[1], t443[1])];
                    }
                    x96 = [vbslq_u32(t433[0], t443[0], x96[0]), vbslq_u32(t433[1], t443[1], x96[1])];
                }
                x72 = [vbslq_u32(t413[0], x96[0], x72[0]), vbslq_u32(t413[1], x96[1], x72[1])];
                let t449: [uint32x4_t; 2] = [vandq_u32(t413[0], live[0]), vandq_u32(t413[1], live[1])];
                let t450: [uint32x4_t; 2] = [x80[0], x80[1]];
                let t451 = [vdupq_n_u32(1u32); 2];
                let t452: [uint32x4_t; 2] = [vaddq_u32(t450[0], t451[0]), vaddq_u32(t450[1], t451[1])];
                let t453 = x80;
                let t454: [uint32x4_t; 4] = [t452[0], t452[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                x80 = t454;
            }
            let t455 = [vdupq_n_u32(2147483648u32); 2];
            let t456: [uint32x4_t; 2] = [vandq_u32(x72[0], t455[0]), vandq_u32(x72[1], t455[1])];
            let t457 = [vdupq_n_u32(0u32); 2];
            let t458: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t456[0], t457[0])), vmvnq_u32(vceqq_u32(t456[1], t457[1]))];
            let t459 = [vdupq_n_u32(4294967294u32); 2];
            let t460: [uint32x4_t; 2] = [vcltq_u32(x72[0], t459[0]), vcltq_u32(x72[1], t459[1])];
            let t461: [uint32x4_t; 2] = [vandq_u32(t458[0], t460[0]), vandq_u32(t458[1], t460[1])];
            let t462: [uint32x4_t; 2] = [vandq_u32(live[0], t461[0]), vandq_u32(live[1], t461[1])];
            if any(&t462) {
                let t463 = [vdupq_n_u32(2147483647u32); 2];
                let t464: [uint32x4_t; 2] = [vandq_u32(x72[0], t463[0]), vandq_u32(x72[1], t463[1])];
                let t465 = b7_r0r1l_p(l, lid0, t462, arg0, arg1, t464);
                x72 = [vbslq_u32(t462[0], t465[0], x72[0]), vbslq_u32(t462[1], t465[1], x72[1])];
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
    let t466 = (arg2 as *mut u32).add((gid0) as usize);
    if all(&live) {
        vst1q_u32(t466.add(0), x8[0]);
        vst1q_u32(t466.add(4), x8[1]);
    } else {
        let mut t467w = 0u32;
        let t467 = core::hint::black_box(&raw mut t467w);
        core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t466.add(0), t467).write(vgetq_lane_u32::<0>(x8[0]));
        core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t466.add(1), t467).write(vgetq_lane_u32::<1>(x8[0]));
        core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t466.add(2), t467).write(vgetq_lane_u32::<2>(x8[0]));
        core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t466.add(3), t467).write(vgetq_lane_u32::<3>(x8[0]));
        core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t466.add(4), t467).write(vgetq_lane_u32::<0>(x8[1]));
        core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t466.add(5), t467).write(vgetq_lane_u32::<1>(x8[1]));
        core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t466.add(6), t467).write(vgetq_lane_u32::<2>(x8[1]));
        core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t466.add(7), t467).write(vgetq_lane_u32::<3>(x8[1]));
    }
}

#[inline(always)]
unsafe fn b6_ag4r2l_f(l: &Launch, lid0: u32, arg2: *const u8, x8: [uint32x4_t; 2]) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let t468 = (arg2 as *mut u32).add((gid0) as usize);
    vst1q_u32(t468.add(0), x8[0]);
    vst1q_u32(t468.add(4), x8[1]);
}

#[inline(always)]
unsafe fn b7_r0r1g_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t469 = b8_r0r1g_p(l, lid0, live, arg0, arg1);
    x24 = t469;
    let mut t470 = live;
    loop {
        // Lanes part: each its own rounds.
        let t472: u64 = (1u64).wrapping_shl((32u64) as u32);
        let t473 = [vdupq_n_u32((t472) as u32); 2];
        let t474 = [vdupq_n_u32(((t472) >> 32) as u32); 2];
        let t475: [uint32x4_t; 4] = [vandq_u32(x24[0], t473[0]), vandq_u32(x24[1], t473[1]), vandq_u32(x24[2], t474[0]), vandq_u32(x24[3], t474[1])];
        let t476: u64 = 0u64;
        let t477 = [vdupq_n_u32((t476) as u32); 2];
        let t478: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t475[0], t477[0]), vceqq_u32(t475[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t475[1], t477[1]), vceqq_u32(t475[3], vdupq_n_u32(0)))];
        let t479: [uint32x4_t; 2] = [vmvnq_u32(t478[0]), vmvnq_u32(t478[1])];
        t470 = [vandq_u32(t470[0], t479[0]), vandq_u32(t470[1], t479[1])];
        if !(any(&t470)) { break; }
        let t481: [uint32x4_t; 2] = [x24[0], x24[1]];
        let t482 = b8_r0r1l_p(l, lid0, t470, arg0, arg1, t481);
        x24 = [vbslq_u32(t470[0], t482[0], x24[0]), vbslq_u32(t470[1], t482[1], x24[1]), vbslq_u32(t470[0], t482[2], x24[2]), vbslq_u32(t470[1], t482[3], x24[3])];
    }
    let t483: [uint32x4_t; 2] = [x24[0], x24[1]];
    res = t483;
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1g_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t484 = b8_r0r1g_f(l, lid0, arg0, arg1);
    x24 = t484;
    let mut t493 = live;
    loop {
        // Lanes part: each its own rounds.
        let t495: u64 = (1u64).wrapping_shl((32u64) as u32);
        let t496 = [vdupq_n_u32((t495) as u32); 2];
        let t497 = [vdupq_n_u32(((t495) >> 32) as u32); 2];
        let t498: [uint32x4_t; 4] = [vandq_u32(x24[0], t496[0]), vandq_u32(x24[1], t496[1]), vandq_u32(x24[2], t497[0]), vandq_u32(x24[3], t497[1])];
        let t499: u64 = 0u64;
        let t500 = [vdupq_n_u32((t499) as u32); 2];
        let t501: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t498[0], t500[0]), vceqq_u32(t498[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t498[1], t500[1]), vceqq_u32(t498[3], vdupq_n_u32(0)))];
        let t502: [uint32x4_t; 2] = [vmvnq_u32(t501[0]), vmvnq_u32(t501[1])];
        t493 = [vandq_u32(t493[0], t502[0]), vandq_u32(t493[1], t502[1])];
        if !(any(&t493)) { break; }
        let t504: [uint32x4_t; 2] = [x24[0], x24[1]];
        let t505 = b8_r0r1l_p(l, lid0, t493, arg0, arg1, t504);
        x24 = [vbslq_u32(t493[0], t505[0], x24[0]), vbslq_u32(t493[1], t505[1], x24[1]), vbslq_u32(t493[0], t505[2], x24[2]), vbslq_u32(t493[1], t505[3], x24[3])];
    }
    let t506: [uint32x4_t; 2] = [x24[0], x24[1]];
    res = t506;
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t507 = b8_r0r1l_p(l, lid0, live, arg0, arg1, x16);
    x24 = t507;
    let mut t508 = live;
    loop {
        // Lanes part: each its own rounds.
        let t510: u64 = (1u64).wrapping_shl((32u64) as u32);
        let t511 = [vdupq_n_u32((t510) as u32); 2];
        let t512 = [vdupq_n_u32(((t510) >> 32) as u32); 2];
        let t513: [uint32x4_t; 4] = [vandq_u32(x24[0], t511[0]), vandq_u32(x24[1], t511[1]), vandq_u32(x24[2], t512[0]), vandq_u32(x24[3], t512[1])];
        let t514: u64 = 0u64;
        let t515 = [vdupq_n_u32((t514) as u32); 2];
        let t516: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t513[0], t515[0]), vceqq_u32(t513[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t513[1], t515[1]), vceqq_u32(t513[3], vdupq_n_u32(0)))];
        let t517: [uint32x4_t; 2] = [vmvnq_u32(t516[0]), vmvnq_u32(t516[1])];
        t508 = [vandq_u32(t508[0], t517[0]), vandq_u32(t508[1], t517[1])];
        if !(any(&t508)) { break; }
        let t519: [uint32x4_t; 2] = [x24[0], x24[1]];
        let t520 = b8_r0r1l_p(l, lid0, t508, arg0, arg1, t519);
        x24 = [vbslq_u32(t508[0], t520[0], x24[0]), vbslq_u32(t508[1], t520[1], x24[1]), vbslq_u32(t508[0], t520[2], x24[2]), vbslq_u32(t508[1], t520[3], x24[3])];
    }
    let t521: [uint32x4_t; 2] = [x24[0], x24[1]];
    res = t521;
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
    let t522: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
    let t523 = [vdupq_n_u32(0u32); 2];
    let t524: [uint32x4_t; 2] = [vceqq_u32(t522[0], t523[0]), vceqq_u32(t522[1], t523[1])];
    let t525: [uint32x4_t; 2] = [vandq_u32(live[0], t524[0]), vandq_u32(live[1], t524[1])];
    {
        let t526: u64 = 4294967295u64;
        let t527 = [vdupq_n_u32((t526) as u32); 2];
        let t528: [uint32x4_t; 4] = [t527[0], t527[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = [vbslq_u32(t525[0], t528[0], res[0]), vbslq_u32(t525[1], t528[1], res[1]), vbslq_u32(t525[0], t528[2], res[2]), vbslq_u32(t525[1], t528[3], res[3])];
        live = [vbicq_u32(live[0], t525[0]), vbicq_u32(live[1], t525[1])];
    }
    if any(&live) {
        x24 = gid0.wrapping_sub(1u32);
        let t529 = if (x24 & 255u32) <= 248u32 { b12_r0r1u8_p(l, lid0, live, arg0, arg1, x24) } else { b12_r0r1i_p(l, lid0, live, arg0, arg1, x24) };
        x32 = t529;
        let t530: u64 = 1u64;
        let t531 = [vdupq_n_u32((t530) as u32); 2];
        let t532: [uint32x4_t; 2] = [vandq_u32(x32[0], t531[0]), vandq_u32(x32[1], t531[1])];
        let t533: u64 = 0u64;
        let t534 = [vdupq_n_u32((t533) as u32); 2];
        let t535: [uint32x4_t; 2] = [vceqq_u32(t532[0], t534[0]), vceqq_u32(t532[1], t534[1])];
        let t536: [uint32x4_t; 2] = [vandq_u32(live[0], t535[0]), vandq_u32(live[1], t535[1])];
        {
            let t537: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[1].as_ptr()))];
            let t538: [uint32x4_t; 4] = [t537[0], t537[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t536[0], t538[0], res[0]), vbslq_u32(t536[1], t538[1], res[1]), vbslq_u32(t536[0], t538[2], res[2]), vbslq_u32(t536[1], t538[3], res[3])];
            live = [vbicq_u32(live[0], t536[0]), vbicq_u32(live[1], t536[1])];
        }
        if any(&live) {
            let t539: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x32[0]), vshlq_n_u32::<30>(x32[2])), vorrq_u32(vshrq_n_u32::<2>(x32[1]), vshlq_n_u32::<30>(x32[3])), vshrq_n_u32::<2>(x32[2]), vshrq_n_u32::<2>(x32[3])];
            let t540: [uint32x4_t; 2] = [t539[0], t539[1]];
            x40 = t540;
            let t541 = [vdupq_n_u32(1u32); 2];
            let t542: [uint32x4_t; 2] = [vsubq_u32(x40[0], t541[0]), vsubq_u32(x40[1], t541[1])];
            let t543: [uint32x4_t; 2] = [vcgeq_u32(t542[0], t522[0]), vcgeq_u32(t542[1], t522[1])];
            let t544: [uint32x4_t; 2] = [vandq_u32(live[0], t543[0]), vandq_u32(live[1], t543[1])];
            {
                let t545: u64 = 4294967295u64;
                let t546 = [vdupq_n_u32((t545) as u32); 2];
                let t547: [uint32x4_t; 4] = [t546[0], t546[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t544[0], t547[0], res[0]), vbslq_u32(t544[1], t547[1], res[1]), vbslq_u32(t544[0], t547[2], res[2]), vbslq_u32(t544[1], t547[3], res[3])];
                live = [vbicq_u32(live[0], t544[0]), vbicq_u32(live[1], t544[1])];
            }
            if any(&live) {
                let t548: [uint32x4_t; 2] = [vsubq_u32(t522[0], x40[0]), vsubq_u32(t522[1], x40[1])];
                x48 = t548;
                let t549: u64 = 2u64;
                let t550 = [vdupq_n_u32((t549) as u32); 2];
                let t551: [uint32x4_t; 2] = [vandq_u32(x32[0], t550[0]), vandq_u32(x32[1], t550[1])];
                let t552: u64 = 0u64;
                let t553 = [vdupq_n_u32((t552) as u32); 2];
                let t554: [uint32x4_t; 2] = [vceqq_u32(t551[0], t553[0]), vceqq_u32(t551[1], t553[1])];
                let t555: [uint32x4_t; 2] = [vmvnq_u32(t554[0]), vmvnq_u32(t554[1])];
                let t556: [uint32x4_t; 2] = [vandq_u32(live[0], t555[0]), vandq_u32(live[1], t555[1])];
                {
                    let t557: [uint32x4_t; 4] = [x48[0], x48[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                    res = [vbslq_u32(t556[0], t557[0], res[0]), vbslq_u32(t556[1], t557[1], res[1]), vbslq_u32(t556[0], t557[2], res[2]), vbslq_u32(t556[1], t557[3], res[3])];
                    live = [vbicq_u32(live[0], t556[0]), vbicq_u32(live[1], t556[1])];
                }
                if any(&live) {
                    let t558: [uint32x4_t; 2] = [vceqq_u32(x48[0], t523[0]), vceqq_u32(x48[1], t523[1])];
                    let t559: [uint32x4_t; 2] = [vandq_u32(live[0], t558[0]), vandq_u32(live[1], t558[1])];
                    {
                        let t560: u64 = 4294967295u64;
                        let t561 = [vdupq_n_u32((t560) as u32); 2];
                        let t562: [uint32x4_t; 4] = [t561[0], t561[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                        res = [vbslq_u32(t559[0], t562[0], res[0]), vbslq_u32(t559[1], t562[1], res[1]), vbslq_u32(t559[0], t562[2], res[2]), vbslq_u32(t559[1], t562[3], res[3])];
                        live = [vbicq_u32(live[0], t559[0]), vbicq_u32(live[1], t559[1])];
                    }
                    if any(&live) {
                        let t563: u64 = (1u64).wrapping_shl((32u64) as u32);
                        let t564 = [vdupq_n_u32((t563) as u32); 2];
                        let t565 = [vdupq_n_u32(((t563) >> 32) as u32); 2];
                        let t566: [uint32x4_t; 4] = [vorrq_u32(t564[0], x48[0]), vorrq_u32(t564[1], x48[1]), t565[0], t565[1]];
                        res = [vbslq_u32(live[0], t566[0], res[0]), vbslq_u32(live[1], t566[1], res[1]), vbslq_u32(live[0], t566[2], res[2]), vbslq_u32(live[1], t566[3], res[3])];
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
        let t567 = if (x24 & 255u32) <= 248u32 { b12_r0r1u8_f(l, lid0, arg0, arg1, x24) } else { b12_r0r1i_f(l, lid0, arg0, arg1, x24) };
        x32 = t567;
        let t568: u64 = 1u64;
        let t569 = [vdupq_n_u32((t568) as u32); 2];
        let t570: [uint32x4_t; 2] = [vandq_u32(x32[0], t569[0]), vandq_u32(x32[1], t569[1])];
        let t571: u64 = 0u64;
        let t572 = [vdupq_n_u32((t571) as u32); 2];
        let t573: [uint32x4_t; 2] = [vceqq_u32(t570[0], t572[0]), vceqq_u32(t570[1], t572[1])];
        {
            let t574: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[1].as_ptr()))];
            let t575: [uint32x4_t; 4] = [t574[0], t574[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t573[0], t575[0], res[0]), vbslq_u32(t573[1], t575[1], res[1]), vbslq_u32(t573[0], t575[2], res[2]), vbslq_u32(t573[1], t575[3], res[3])];
            live = [vbicq_u32(live[0], t573[0]), vbicq_u32(live[1], t573[1])];
        }
        if any(&live) {
            let t576: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x32[0]), vshlq_n_u32::<30>(x32[2])), vorrq_u32(vshrq_n_u32::<2>(x32[1]), vshlq_n_u32::<30>(x32[3])), vshrq_n_u32::<2>(x32[2]), vshrq_n_u32::<2>(x32[3])];
            let t577: [uint32x4_t; 2] = [t576[0], t576[1]];
            x40 = t577;
            let t578 = [vdupq_n_u32(1u32); 2];
            let t579: [uint32x4_t; 2] = [vsubq_u32(x40[0], t578[0]), vsubq_u32(x40[1], t578[1])];
            let t580: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
            let t581: [uint32x4_t; 2] = [vcgeq_u32(t579[0], t580[0]), vcgeq_u32(t579[1], t580[1])];
            let t582: [uint32x4_t; 2] = [vandq_u32(live[0], t581[0]), vandq_u32(live[1], t581[1])];
            {
                let t583: u64 = 4294967295u64;
                let t584 = [vdupq_n_u32((t583) as u32); 2];
                let t585: [uint32x4_t; 4] = [t584[0], t584[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t582[0], t585[0], res[0]), vbslq_u32(t582[1], t585[1], res[1]), vbslq_u32(t582[0], t585[2], res[2]), vbslq_u32(t582[1], t585[3], res[3])];
                live = [vbicq_u32(live[0], t582[0]), vbicq_u32(live[1], t582[1])];
            }
            if any(&live) {
                let t586: [uint32x4_t; 2] = [vsubq_u32(t580[0], x40[0]), vsubq_u32(t580[1], x40[1])];
                x48 = t586;
                let t587: u64 = 2u64;
                let t588 = [vdupq_n_u32((t587) as u32); 2];
                let t589: [uint32x4_t; 2] = [vandq_u32(x32[0], t588[0]), vandq_u32(x32[1], t588[1])];
                let t590: u64 = 0u64;
                let t591 = [vdupq_n_u32((t590) as u32); 2];
                let t592: [uint32x4_t; 2] = [vceqq_u32(t589[0], t591[0]), vceqq_u32(t589[1], t591[1])];
                let t593: [uint32x4_t; 2] = [vmvnq_u32(t592[0]), vmvnq_u32(t592[1])];
                let t594: [uint32x4_t; 2] = [vandq_u32(live[0], t593[0]), vandq_u32(live[1], t593[1])];
                {
                    let t595: [uint32x4_t; 4] = [x48[0], x48[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                    res = [vbslq_u32(t594[0], t595[0], res[0]), vbslq_u32(t594[1], t595[1], res[1]), vbslq_u32(t594[0], t595[2], res[2]), vbslq_u32(t594[1], t595[3], res[3])];
                    live = [vbicq_u32(live[0], t594[0]), vbicq_u32(live[1], t594[1])];
                }
                if any(&live) {
                    let t596 = [vdupq_n_u32(0u32); 2];
                    let t597: [uint32x4_t; 2] = [vceqq_u32(x48[0], t596[0]), vceqq_u32(x48[1], t596[1])];
                    let t598: [uint32x4_t; 2] = [vandq_u32(live[0], t597[0]), vandq_u32(live[1], t597[1])];
                    {
                        let t599: u64 = 4294967295u64;
                        let t600 = [vdupq_n_u32((t599) as u32); 2];
                        let t601: [uint32x4_t; 4] = [t600[0], t600[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                        res = [vbslq_u32(t598[0], t601[0], res[0]), vbslq_u32(t598[1], t601[1], res[1]), vbslq_u32(t598[0], t601[2], res[2]), vbslq_u32(t598[1], t601[3], res[3])];
                        live = [vbicq_u32(live[0], t598[0]), vbicq_u32(live[1], t598[1])];
                    }
                    if any(&live) {
                        let t602: u64 = (1u64).wrapping_shl((32u64) as u32);
                        let t603 = [vdupq_n_u32((t602) as u32); 2];
                        let t604 = [vdupq_n_u32(((t602) >> 32) as u32); 2];
                        let t605: [uint32x4_t; 4] = [vorrq_u32(t603[0], x48[0]), vorrq_u32(t603[1], x48[1]), t604[0], t604[1]];
                        res = [vbslq_u32(live[0], t605[0], res[0]), vbslq_u32(live[1], t605[1], res[1]), vbslq_u32(live[0], t605[2], res[2]), vbslq_u32(live[1], t605[3], res[3])];
                        live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
                    }
                }
            }
        }
    } else if false {
        let t606: u64 = 4294967295u64;
        let t607 = [vdupq_n_u32((t606) as u32); 2];
        let t608: [uint32x4_t; 4] = [t607[0], t607[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t608;
        return res;
    } else {
        // Lanes part: the chunk's lanes on both sides of the bound.
        let t609: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t610 = [vdupq_n_u32(0u32); 2];
        let t611: [uint32x4_t; 2] = [vceqq_u32(t609[0], t610[0]), vceqq_u32(t609[1], t610[1])];
        let t612: [uint32x4_t; 2] = [vandq_u32(live[0], t611[0]), vandq_u32(live[1], t611[1])];
        {
            let t613: u64 = 4294967295u64;
            let t614 = [vdupq_n_u32((t613) as u32); 2];
            let t615: [uint32x4_t; 4] = [t614[0], t614[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t612[0], t615[0], res[0]), vbslq_u32(t612[1], t615[1], res[1]), vbslq_u32(t612[0], t615[2], res[2]), vbslq_u32(t612[1], t615[3], res[3])];
            live = [vbicq_u32(live[0], t612[0]), vbicq_u32(live[1], t612[1])];
        }
        if any(&live) {
            x24 = gid0.wrapping_sub(1u32);
            let t616 = if (x24 & 255u32) <= 248u32 { b12_r0r1u8_p(l, lid0, live, arg0, arg1, x24) } else { b12_r0r1i_p(l, lid0, live, arg0, arg1, x24) };
            x32 = t616;
            let t617: u64 = 1u64;
            let t618 = [vdupq_n_u32((t617) as u32); 2];
            let t619: [uint32x4_t; 2] = [vandq_u32(x32[0], t618[0]), vandq_u32(x32[1], t618[1])];
            let t620: u64 = 0u64;
            let t621 = [vdupq_n_u32((t620) as u32); 2];
            let t622: [uint32x4_t; 2] = [vceqq_u32(t619[0], t621[0]), vceqq_u32(t619[1], t621[1])];
            let t623: [uint32x4_t; 2] = [vandq_u32(live[0], t622[0]), vandq_u32(live[1], t622[1])];
            {
                let t624: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[1].as_ptr()))];
                let t625: [uint32x4_t; 4] = [t624[0], t624[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t623[0], t625[0], res[0]), vbslq_u32(t623[1], t625[1], res[1]), vbslq_u32(t623[0], t625[2], res[2]), vbslq_u32(t623[1], t625[3], res[3])];
                live = [vbicq_u32(live[0], t623[0]), vbicq_u32(live[1], t623[1])];
            }
            if any(&live) {
                let t626: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x32[0]), vshlq_n_u32::<30>(x32[2])), vorrq_u32(vshrq_n_u32::<2>(x32[1]), vshlq_n_u32::<30>(x32[3])), vshrq_n_u32::<2>(x32[2]), vshrq_n_u32::<2>(x32[3])];
                let t627: [uint32x4_t; 2] = [t626[0], t626[1]];
                x40 = t627;
                let t628 = [vdupq_n_u32(1u32); 2];
                let t629: [uint32x4_t; 2] = [vsubq_u32(x40[0], t628[0]), vsubq_u32(x40[1], t628[1])];
                let t630: [uint32x4_t; 2] = [vcgeq_u32(t629[0], t609[0]), vcgeq_u32(t629[1], t609[1])];
                let t631: [uint32x4_t; 2] = [vandq_u32(live[0], t630[0]), vandq_u32(live[1], t630[1])];
                {
                    let t632: u64 = 4294967295u64;
                    let t633 = [vdupq_n_u32((t632) as u32); 2];
                    let t634: [uint32x4_t; 4] = [t633[0], t633[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                    res = [vbslq_u32(t631[0], t634[0], res[0]), vbslq_u32(t631[1], t634[1], res[1]), vbslq_u32(t631[0], t634[2], res[2]), vbslq_u32(t631[1], t634[3], res[3])];
                    live = [vbicq_u32(live[0], t631[0]), vbicq_u32(live[1], t631[1])];
                }
                if any(&live) {
                    let t635: [uint32x4_t; 2] = [vsubq_u32(t609[0], x40[0]), vsubq_u32(t609[1], x40[1])];
                    x48 = t635;
                    let t636: u64 = 2u64;
                    let t637 = [vdupq_n_u32((t636) as u32); 2];
                    let t638: [uint32x4_t; 2] = [vandq_u32(x32[0], t637[0]), vandq_u32(x32[1], t637[1])];
                    let t639: u64 = 0u64;
                    let t640 = [vdupq_n_u32((t639) as u32); 2];
                    let t641: [uint32x4_t; 2] = [vceqq_u32(t638[0], t640[0]), vceqq_u32(t638[1], t640[1])];
                    let t642: [uint32x4_t; 2] = [vmvnq_u32(t641[0]), vmvnq_u32(t641[1])];
                    let t643: [uint32x4_t; 2] = [vandq_u32(live[0], t642[0]), vandq_u32(live[1], t642[1])];
                    {
                        let t644: [uint32x4_t; 4] = [x48[0], x48[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                        res = [vbslq_u32(t643[0], t644[0], res[0]), vbslq_u32(t643[1], t644[1], res[1]), vbslq_u32(t643[0], t644[2], res[2]), vbslq_u32(t643[1], t644[3], res[3])];
                        live = [vbicq_u32(live[0], t643[0]), vbicq_u32(live[1], t643[1])];
                    }
                    if any(&live) {
                        let t645: [uint32x4_t; 2] = [vceqq_u32(x48[0], t610[0]), vceqq_u32(x48[1], t610[1])];
                        let t646: [uint32x4_t; 2] = [vandq_u32(live[0], t645[0]), vandq_u32(live[1], t645[1])];
                        {
                            let t647: u64 = 4294967295u64;
                            let t648 = [vdupq_n_u32((t647) as u32); 2];
                            let t649: [uint32x4_t; 4] = [t648[0], t648[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                            res = [vbslq_u32(t646[0], t649[0], res[0]), vbslq_u32(t646[1], t649[1], res[1]), vbslq_u32(t646[0], t649[2], res[2]), vbslq_u32(t646[1], t649[3], res[3])];
                            live = [vbicq_u32(live[0], t646[0]), vbicq_u32(live[1], t646[1])];
                        }
                        if any(&live) {
                            let t650: u64 = (1u64).wrapping_shl((32u64) as u32);
                            let t651 = [vdupq_n_u32((t650) as u32); 2];
                            let t652 = [vdupq_n_u32(((t650) >> 32) as u32); 2];
                            let t653: [uint32x4_t; 4] = [vorrq_u32(t651[0], x48[0]), vorrq_u32(t651[1], x48[1]), t652[0], t652[1]];
                            res = [vbslq_u32(live[0], t653[0], res[0]), vbslq_u32(live[1], t653[1], res[1]), vbslq_u32(live[0], t653[2], res[2]), vbslq_u32(live[1], t653[3], res[3])];
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
    let t654 = [vdupq_n_u32(0u32); 2];
    let t655: [uint32x4_t; 2] = [vceqq_u32(x16[0], t654[0]), vceqq_u32(x16[1], t654[1])];
    let t656: [uint32x4_t; 2] = [vandq_u32(live[0], t655[0]), vandq_u32(live[1], t655[1])];
    {
        let t657: u64 = 4294967295u64;
        let t658 = [vdupq_n_u32((t657) as u32); 2];
        let t659: [uint32x4_t; 4] = [t658[0], t658[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = [vbslq_u32(t656[0], t659[0], res[0]), vbslq_u32(t656[1], t659[1], res[1]), vbslq_u32(t656[0], t659[2], res[2]), vbslq_u32(t656[1], t659[3], res[3])];
        live = [vbicq_u32(live[0], t656[0]), vbicq_u32(live[1], t656[1])];
    }
    if any(&live) {
        let t660 = [vdupq_n_u32(1u32); 2];
        let t661: [uint32x4_t; 2] = [vsubq_u32(x16[0], t660[0]), vsubq_u32(x16[1], t660[1])];
        x24 = t661;
        let t662 = b12_r0r1l_p(l, lid0, live, arg0, arg1, x24);
        x32 = t662;
        let t663: u64 = 1u64;
        let t664 = [vdupq_n_u32((t663) as u32); 2];
        let t665: [uint32x4_t; 2] = [vandq_u32(x32[0], t664[0]), vandq_u32(x32[1], t664[1])];
        let t666: u64 = 0u64;
        let t667 = [vdupq_n_u32((t666) as u32); 2];
        let t668: [uint32x4_t; 2] = [vceqq_u32(t665[0], t667[0]), vceqq_u32(t665[1], t667[1])];
        let t669: [uint32x4_t; 2] = [vandq_u32(live[0], t668[0]), vandq_u32(live[1], t668[1])];
        {
            let t670: [uint32x4_t; 4] = [x24[0], x24[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t669[0], t670[0], res[0]), vbslq_u32(t669[1], t670[1], res[1]), vbslq_u32(t669[0], t670[2], res[2]), vbslq_u32(t669[1], t670[3], res[3])];
            live = [vbicq_u32(live[0], t669[0]), vbicq_u32(live[1], t669[1])];
        }
        if any(&live) {
            let t671: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x32[0]), vshlq_n_u32::<30>(x32[2])), vorrq_u32(vshrq_n_u32::<2>(x32[1]), vshlq_n_u32::<30>(x32[3])), vshrq_n_u32::<2>(x32[2]), vshrq_n_u32::<2>(x32[3])];
            let t672: [uint32x4_t; 2] = [t671[0], t671[1]];
            x40 = t672;
            let t673: [uint32x4_t; 2] = [vsubq_u32(x40[0], t660[0]), vsubq_u32(x40[1], t660[1])];
            let t674: [uint32x4_t; 2] = [vcgeq_u32(t673[0], x16[0]), vcgeq_u32(t673[1], x16[1])];
            let t675: [uint32x4_t; 2] = [vandq_u32(live[0], t674[0]), vandq_u32(live[1], t674[1])];
            {
                let t676: u64 = 4294967295u64;
                let t677 = [vdupq_n_u32((t676) as u32); 2];
                let t678: [uint32x4_t; 4] = [t677[0], t677[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t675[0], t678[0], res[0]), vbslq_u32(t675[1], t678[1], res[1]), vbslq_u32(t675[0], t678[2], res[2]), vbslq_u32(t675[1], t678[3], res[3])];
                live = [vbicq_u32(live[0], t675[0]), vbicq_u32(live[1], t675[1])];
            }
            if any(&live) {
                let t679: [uint32x4_t; 2] = [vsubq_u32(x16[0], x40[0]), vsubq_u32(x16[1], x40[1])];
                x48 = t679;
                let t680: u64 = 2u64;
                let t681 = [vdupq_n_u32((t680) as u32); 2];
                let t682: [uint32x4_t; 2] = [vandq_u32(x32[0], t681[0]), vandq_u32(x32[1], t681[1])];
                let t683: u64 = 0u64;
                let t684 = [vdupq_n_u32((t683) as u32); 2];
                let t685: [uint32x4_t; 2] = [vceqq_u32(t682[0], t684[0]), vceqq_u32(t682[1], t684[1])];
                let t686: [uint32x4_t; 2] = [vmvnq_u32(t685[0]), vmvnq_u32(t685[1])];
                let t687: [uint32x4_t; 2] = [vandq_u32(live[0], t686[0]), vandq_u32(live[1], t686[1])];
                {
                    let t688: [uint32x4_t; 4] = [x48[0], x48[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                    res = [vbslq_u32(t687[0], t688[0], res[0]), vbslq_u32(t687[1], t688[1], res[1]), vbslq_u32(t687[0], t688[2], res[2]), vbslq_u32(t687[1], t688[3], res[3])];
                    live = [vbicq_u32(live[0], t687[0]), vbicq_u32(live[1], t687[1])];
                }
                if any(&live) {
                    let t689: [uint32x4_t; 2] = [vceqq_u32(x48[0], t654[0]), vceqq_u32(x48[1], t654[1])];
                    let t690: [uint32x4_t; 2] = [vandq_u32(live[0], t689[0]), vandq_u32(live[1], t689[1])];
                    {
                        let t691: u64 = 4294967295u64;
                        let t692 = [vdupq_n_u32((t691) as u32); 2];
                        let t693: [uint32x4_t; 4] = [t692[0], t692[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                        res = [vbslq_u32(t690[0], t693[0], res[0]), vbslq_u32(t690[1], t693[1], res[1]), vbslq_u32(t690[0], t693[2], res[2]), vbslq_u32(t690[1], t693[3], res[3])];
                        live = [vbicq_u32(live[0], t690[0]), vbicq_u32(live[1], t690[1])];
                    }
                    if any(&live) {
                        let t694: u64 = (1u64).wrapping_shl((32u64) as u32);
                        let t695 = [vdupq_n_u32((t694) as u32); 2];
                        let t696 = [vdupq_n_u32(((t694) >> 32) as u32); 2];
                        let t697: [uint32x4_t; 4] = [vorrq_u32(t695[0], x48[0]), vorrq_u32(t695[1], x48[1]), t696[0], t696[1]];
                        res = [vbslq_u32(live[0], t697[0], res[0]), vbslq_u32(live[1], t697[1], res[1]), vbslq_u32(live[0], t697[2], res[2]), vbslq_u32(live[1], t697[3], res[3])];
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
    let t698: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x16), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x16), vld1q_u32(LANE[1].as_ptr()))];
    let t699 = [vdupq_n_u32(31u32); 2];
    let t700: [uint32x4_t; 2] = [vceqq_u32(t698[0], t699[0]), vceqq_u32(t698[1], t699[1])];
    let t701: [uint32x4_t; 2] = [vandq_u32(live[0], t700[0]), vandq_u32(live[1], t700[1])];
    {
        let t702: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t703: [uint32x4_t; 2] = [vshlq_u32(t702[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(t702[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
        let t704: [uint32x4_t; 2] = [vshlq_u32(t702[0], vnegq_s32(vdupq_n_s32(((5u32) & 31) as i32))), vshlq_u32(t702[1], vnegq_s32(vdupq_n_s32(((5u32) & 31) as i32)))];
        let t705: [uint32x4_t; 2] = [vandq_u32(t704[0], t699[0]), vandq_u32(t704[1], t699[1])];
        let t706 = [vdupq_n_u32(1u32); 2];
        let t707: [uint32x4_t; 2] = [vshlq_u32(t706[0], vreinterpretq_s32_u32(vandq_u32(t705[0], vdupq_n_u32(31)))), vshlq_u32(t706[1], vreinterpretq_s32_u32(vandq_u32(t705[1], vdupq_n_u32(31))))];
        let t708 = b13_al4r3l_p(l, lid0, t701, arg3, t703, t707);
    }
}

#[inline(always)]
unsafe fn b9_r3gl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg3: *const u8, x16: [uint32x4_t; 2]) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let t709 = [vdupq_n_u32(31u32); 2];
    let t710: [uint32x4_t; 2] = [vceqq_u32(x16[0], t709[0]), vceqq_u32(x16[1], t709[1])];
    let t711: [uint32x4_t; 2] = [vandq_u32(live[0], t710[0]), vandq_u32(live[1], t710[1])];
    {
        let t712: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t713: [uint32x4_t; 2] = [vshlq_u32(t712[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(t712[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
        let t714: [uint32x4_t; 2] = [vshlq_u32(t712[0], vnegq_s32(vdupq_n_s32(((5u32) & 31) as i32))), vshlq_u32(t712[1], vnegq_s32(vdupq_n_s32(((5u32) & 31) as i32)))];
        let t715: [uint32x4_t; 2] = [vandq_u32(t714[0], t709[0]), vandq_u32(t714[1], t709[1])];
        let t716 = [vdupq_n_u32(1u32); 2];
        let t717: [uint32x4_t; 2] = [vshlq_u32(t716[0], vreinterpretq_s32_u32(vandq_u32(t715[0], vdupq_n_u32(31)))), vshlq_u32(t716[1], vreinterpretq_s32_u32(vandq_u32(t715[1], vdupq_n_u32(31))))];
        let t718 = b13_al4r3l_p(l, lid0, t711, arg3, t713, t717);
    }
}

#[inline(always)]
unsafe fn b10_r3ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg3: *const u8, x8: [uint32x4_t; 2], x16: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t719: [uint32x4_t; 2] = [vshlq_u32(x8[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x8[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t720 = b11_al4r3_p(l, lid0, live, arg3, t719);
    x24 = t720;
    let t721: [uint32x4_t; 2] = [vcgeq_u32(x8[0], x16[0]), vcgeq_u32(x8[1], x16[1])];
    let t722: [uint32x4_t; 2] = [vshlq_u32(x8[0], vnegq_s32(vdupq_n_s32(((5u32) & 31) as i32))), vshlq_u32(x8[1], vnegq_s32(vdupq_n_s32(((5u32) & 31) as i32)))];
    let t723 = [vdupq_n_u32(31u32); 2];
    let t724: [uint32x4_t; 2] = [vandq_u32(t722[0], t723[0]), vandq_u32(t722[1], t723[1])];
    let t725: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vreinterpretq_s32_u32(vandq_u32(t724[0], vdupq_n_u32(31))))), vshlq_u32(x24[1], vnegq_s32(vreinterpretq_s32_u32(vandq_u32(t724[1], vdupq_n_u32(31)))))];
    let t726 = [vdupq_n_u32(1u32); 2];
    let t727: [uint32x4_t; 2] = [vandq_u32(t725[0], t726[0]), vandq_u32(t725[1], t726[1])];
    let t728 = [vdupq_n_u32(0u32); 2];
    let t729: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t727[0], t728[0])), vmvnq_u32(vceqq_u32(t727[1], t728[1]))];
    let t730: [uint32x4_t; 2] = [vorrq_u32(t721[0], t729[0]), vorrq_u32(t721[1], t729[1])];
    res = t730;
    return res;
}

#[inline(always)]
unsafe fn b11_al4r2_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg2: *const u8, x0i: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t731 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], x0i[0]), vandq_u32(live[1], x0i[1]))));
    let t732: [uint32x4_t; 2] = [vbslq_u32(live[0], x0i[0], t731), vbslq_u32(live[1], x0i[1], t731)];
    let t733 = arg2 as *const u32;
    let mut t734: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    t734[0] = vld1q_lane_u32::<0>(t733.add(vgetq_lane_u32::<0>(t732[0]) as usize), t734[0]);
    t734[0] = vld1q_lane_u32::<1>(t733.add(vgetq_lane_u32::<1>(t732[0]) as usize), t734[0]);
    t734[0] = vld1q_lane_u32::<2>(t733.add(vgetq_lane_u32::<2>(t732[0]) as usize), t734[0]);
    t734[0] = vld1q_lane_u32::<3>(t733.add(vgetq_lane_u32::<3>(t732[0]) as usize), t734[0]);
    t734[1] = vld1q_lane_u32::<0>(t733.add(vgetq_lane_u32::<0>(t732[1]) as usize), t734[1]);
    t734[1] = vld1q_lane_u32::<1>(t733.add(vgetq_lane_u32::<1>(t732[1]) as usize), t734[1]);
    t734[1] = vld1q_lane_u32::<2>(t733.add(vgetq_lane_u32::<2>(t732[1]) as usize), t734[1]);
    t734[1] = vld1q_lane_u32::<3>(t733.add(vgetq_lane_u32::<3>(t732[1]) as usize), t734[1]);
    res = t734;
    return res;
}

#[inline(always)]
unsafe fn b11_al4r3_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg3: *const u8, x0i: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t735 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], x0i[0]), vandq_u32(live[1], x0i[1]))));
    let t736: [uint32x4_t; 2] = [vbslq_u32(live[0], x0i[0], t735), vbslq_u32(live[1], x0i[1], t735)];
    let t737 = arg3 as *const u32;
    let mut t738: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    t738[0] = vld1q_lane_u32::<0>(t737.add(vgetq_lane_u32::<0>(t736[0]) as usize), t738[0]);
    t738[0] = vld1q_lane_u32::<1>(t737.add(vgetq_lane_u32::<1>(t736[0]) as usize), t738[0]);
    t738[0] = vld1q_lane_u32::<2>(t737.add(vgetq_lane_u32::<2>(t736[0]) as usize), t738[0]);
    t738[0] = vld1q_lane_u32::<3>(t737.add(vgetq_lane_u32::<3>(t736[0]) as usize), t738[0]);
    t738[1] = vld1q_lane_u32::<0>(t737.add(vgetq_lane_u32::<0>(t736[1]) as usize), t738[1]);
    t738[1] = vld1q_lane_u32::<1>(t737.add(vgetq_lane_u32::<1>(t736[1]) as usize), t738[1]);
    t738[1] = vld1q_lane_u32::<2>(t737.add(vgetq_lane_u32::<2>(t736[1]) as usize), t738[1]);
    t738[1] = vld1q_lane_u32::<3>(t737.add(vgetq_lane_u32::<3>(t736[1]) as usize), t738[1]);
    res = t738;
    return res;
}

#[inline(always)]
unsafe fn b12_r0r1i_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t739 = b14_r0r1si_p(l, lid0, live, arg0, arg1, 0u32, x16);
    res = t739;
    return res;
}

#[inline(always)]
unsafe fn b12_r0r1i_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t740 = b14_r0r1si_f(l, lid0, arg0, arg1, 0u32, x16);
    res = t740;
    return res;
}

#[inline(always)]
unsafe fn b12_r0r1u8_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t741 = b14_r0r1su8_p(l, lid0, live, arg0, arg1, 0u32, x16);
    res = t741;
    return res;
}

#[inline(always)]
unsafe fn b12_r0r1u8_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t742 = b14_r0r1su8_f(l, lid0, arg0, arg1, 0u32, x16);
    res = t742;
    return res;
}

#[inline(always)]
unsafe fn b12_r0r1l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t743 = b14_r0r1sl_p(l, lid0, live, arg0, arg1, 0u32, x16);
    res = t743;
    return res;
}

#[inline(always)]
unsafe fn b13_al4r3l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg3: *const u8, x0i: [uint32x4_t; 2], x8: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let (mut t744, mut t745, mut t746, mut t747) = ([0u32; 8], [0u32; 8], [0u32; 8], [0u32; 8]);
    for r in 0..2 { vst1q_u32(t744.as_mut_ptr().add(4 * r), live[r]); vst1q_u32(t745.as_mut_ptr().add(4 * r), x0i[r]); vst1q_u32(t746.as_mut_ptr().add(4 * r), x8[r]); }
    for j in 0..8 { if t744[j] != 0 { t747[j] = AtomicU32::from_ptr((arg3 as *mut u8).add(t745[j] as usize * 4) as *mut u32).fetch_or(t746[j], Relaxed); } }
    let t748: [uint32x4_t; 2] = [vld1q_u32(t747.as_ptr().add(0)), vld1q_u32(t747.as_ptr().add(4))];
    res = t748;
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
    let t749 = b15_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    x32 = t749;
    let t750: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[1].as_ptr()))];
    let t751: [uint32x4_t; 2] = [vshlq_u32(t750[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(t750[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t752 = b16_r0r1sl_p(l, lid0, live, arg0, arg1, (1u32).wrapping_add(x16), t751);
    x40 = t752;
    let t753 = b17_s_p(l, lid0, live, x32);
    x48 = t753;
    let t754 = [vdupq_n_u32(1023u32); 2];
    let t755: [uint32x4_t; 2] = [vandq_u32(t750[0], t754[0]), vandq_u32(t750[1], t754[1])];
    let t756 = [vdupq_n_u32(x32); 2];
    let t757: [uint32x4_t; 2] = [vmulq_u32(t755[0], t756[0]), vmulq_u32(t755[1], t756[1])];
    let t758 = b18_r0r1lls_p(l, lid0, live, arg0, arg1, x40, t757, 8u32);
    let t759: u64 = x48;
    let t760 = [vdupq_n_u32((t759) as u32); 2];
    let t761 = [vdupq_n_u32(((t759) >> 32) as u32); 2];
    let t762: [uint32x4_t; 4] = [vandq_u32(t758[0], t760[0]), vandq_u32(t758[1], t760[1]), vandq_u32(t758[2], t761[0]), vandq_u32(t758[3], t761[1])];
    x56 = t762;
    let t763: u64 = x48;
    let t764 = [vdupq_n_u32((t763) as u32); 2];
    let t765 = [vdupq_n_u32(((t763) >> 32) as u32); 2];
    let t766: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x56[0], t764[0]), vceqq_u32(x56[2], t765[0])), vandq_u32(vceqq_u32(x56[1], t764[1]), vceqq_u32(x56[3], t765[1]))];
    let mut t767: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t768: [uint32x4_t; 2] = [vandq_u32(live[0], t766[0]), vandq_u32(live[1], t766[1])];
    let t769: [uint32x4_t; 2] = [vbicq_u32(live[0], t766[0]), vbicq_u32(live[1], t766[1])];
    if any(&t768) {
        let t770 = b19_r0r1si_p(l, lid0, t768, arg0, arg1, (x16).wrapping_add(1u32), x24);
        t767 = [vbslq_u32(t768[0], t770[0], t767[0]), vbslq_u32(t768[1], t770[1], t767[1]), vbslq_u32(t768[0], t770[2], t767[2]), vbslq_u32(t768[1], t770[3], t767[3])];
    }
    {
        t767 = [vbslq_u32(t769[0], x56[0], t767[0]), vbslq_u32(t769[1], x56[1], t767[1]), vbslq_u32(t769[0], x56[2], t767[2]), vbslq_u32(t769[1], x56[3], t767[3])];
    }
    res = t767;
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
    let t771 = b15_r0r1s_f(l, lid0, arg0, arg1, x16);
    x32 = t771;
    let t772: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[1].as_ptr()))];
    let t773: [uint32x4_t; 2] = [vshlq_u32(t772[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(t772[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t774 = b16_r0r1sl_f(l, lid0, arg0, arg1, (1u32).wrapping_add(x16), t773);
    x40 = t774;
    let t775 = b17_s_f(l, lid0, x32);
    x48 = t775;
    let t776 = [vdupq_n_u32(1023u32); 2];
    let t777: [uint32x4_t; 2] = [vandq_u32(t772[0], t776[0]), vandq_u32(t772[1], t776[1])];
    let t778 = [vdupq_n_u32(x32); 2];
    let t779: [uint32x4_t; 2] = [vmulq_u32(t777[0], t778[0]), vmulq_u32(t777[1], t778[1])];
    let t780 = b18_r0r1lls_f(l, lid0, arg0, arg1, x40, t779, 8u32);
    let t781: u64 = x48;
    let t782 = [vdupq_n_u32((t781) as u32); 2];
    let t783 = [vdupq_n_u32(((t781) >> 32) as u32); 2];
    let t784: [uint32x4_t; 4] = [vandq_u32(t780[0], t782[0]), vandq_u32(t780[1], t782[1]), vandq_u32(t780[2], t783[0]), vandq_u32(t780[3], t783[1])];
    x56 = t784;
    let t785: u64 = x48;
    let t786 = [vdupq_n_u32((t785) as u32); 2];
    let t787 = [vdupq_n_u32(((t785) >> 32) as u32); 2];
    let t788: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x56[0], t786[0]), vceqq_u32(x56[2], t787[0])), vandq_u32(vceqq_u32(x56[1], t786[1]), vceqq_u32(x56[3], t787[1]))];
    let mut t789: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t790: [uint32x4_t; 2] = [vmvnq_u32(t788[0]), vmvnq_u32(t788[1])];
    if any(&t788) {
        let t791 = b19_r0r1si_p(l, lid0, t788, arg0, arg1, (x16).wrapping_add(1u32), x24);
        t789 = [vbslq_u32(t788[0], t791[0], t789[0]), vbslq_u32(t788[1], t791[1], t789[1]), vbslq_u32(t788[0], t791[2], t789[2]), vbslq_u32(t788[1], t791[3], t789[3])];
    }
    {
        t789 = [vbslq_u32(t790[0], x56[0], t789[0]), vbslq_u32(t790[1], x56[1], t789[1]), vbslq_u32(t790[0], x56[2], t789[2]), vbslq_u32(t790[1], x56[3], t789[3])];
    }
    res = t789;
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
    let t792 = b15_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    x32 = t792;
    let t793 = b16_r0r1ss_p(l, lid0, live, arg0, arg1, (1u32).wrapping_add(x16), (x24 >> 10));
    x40 = t793;
    let t794 = b17_s_p(l, lid0, live, x32);
    x48 = t794;
    let t795 = b18_r0r1sns_p(l, lid0, live, arg0, arg1, x40, (x24 & 1023u32).wrapping_mul(x32), x32, 8u32);
    let t796: u64 = x48;
    let t797 = [vdupq_n_u32((t796) as u32); 2];
    let t798 = [vdupq_n_u32(((t796) >> 32) as u32); 2];
    let t799: [uint32x4_t; 4] = [vandq_u32(t795[0], t797[0]), vandq_u32(t795[1], t797[1]), vandq_u32(t795[2], t798[0]), vandq_u32(t795[3], t798[1])];
    x56 = t799;
    let t800: u64 = x48;
    let t801 = [vdupq_n_u32((t800) as u32); 2];
    let t802 = [vdupq_n_u32(((t800) >> 32) as u32); 2];
    let t803: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x56[0], t801[0]), vceqq_u32(x56[2], t802[0])), vandq_u32(vceqq_u32(x56[1], t801[1]), vceqq_u32(x56[3], t802[1]))];
    let mut t804: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t805: [uint32x4_t; 2] = [vandq_u32(live[0], t803[0]), vandq_u32(live[1], t803[1])];
    let t806: [uint32x4_t; 2] = [vbicq_u32(live[0], t803[0]), vbicq_u32(live[1], t803[1])];
    if any(&t805) {
        let t807 = b19_r0r1su8_p(l, lid0, t805, arg0, arg1, (x16).wrapping_add(1u32), x24);
        t804 = [vbslq_u32(t805[0], t807[0], t804[0]), vbslq_u32(t805[1], t807[1], t804[1]), vbslq_u32(t805[0], t807[2], t804[2]), vbslq_u32(t805[1], t807[3], t804[3])];
    }
    {
        t804 = [vbslq_u32(t806[0], x56[0], t804[0]), vbslq_u32(t806[1], x56[1], t804[1]), vbslq_u32(t806[0], x56[2], t804[2]), vbslq_u32(t806[1], x56[3], t804[3])];
    }
    res = t804;
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
    let t808 = b15_r0r1s_f(l, lid0, arg0, arg1, x16);
    x32 = t808;
    let t809 = b16_r0r1ss_f(l, lid0, arg0, arg1, (1u32).wrapping_add(x16), (x24 >> 10));
    x40 = t809;
    let t810 = b17_s_f(l, lid0, x32);
    x48 = t810;
    let t811 = b18_r0r1sns_f(l, lid0, arg0, arg1, x40, (x24 & 1023u32).wrapping_mul(x32), x32, 8u32);
    let t812: u64 = x48;
    let t813 = [vdupq_n_u32((t812) as u32); 2];
    let t814 = [vdupq_n_u32(((t812) >> 32) as u32); 2];
    let t815: [uint32x4_t; 4] = [vandq_u32(t811[0], t813[0]), vandq_u32(t811[1], t813[1]), vandq_u32(t811[2], t814[0]), vandq_u32(t811[3], t814[1])];
    x56 = t815;
    let t816: u64 = x48;
    let t817 = [vdupq_n_u32((t816) as u32); 2];
    let t818 = [vdupq_n_u32(((t816) >> 32) as u32); 2];
    let t819: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x56[0], t817[0]), vceqq_u32(x56[2], t818[0])), vandq_u32(vceqq_u32(x56[1], t817[1]), vceqq_u32(x56[3], t818[1]))];
    let mut t820: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t821: [uint32x4_t; 2] = [vmvnq_u32(t819[0]), vmvnq_u32(t819[1])];
    if any(&t819) {
        let t822 = b19_r0r1su8_p(l, lid0, t819, arg0, arg1, (x16).wrapping_add(1u32), x24);
        t820 = [vbslq_u32(t819[0], t822[0], t820[0]), vbslq_u32(t819[1], t822[1], t820[1]), vbslq_u32(t819[0], t822[2], t820[2]), vbslq_u32(t819[1], t822[3], t820[3])];
    }
    {
        t820 = [vbslq_u32(t821[0], x56[0], t820[0]), vbslq_u32(t821[1], x56[1], t820[1]), vbslq_u32(t821[0], x56[2], t820[2]), vbslq_u32(t821[1], x56[3], t820[3])];
    }
    res = t820;
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
    let t823 = b15_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    x32 = t823;
    let t824: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t825 = b16_r0r1sl_p(l, lid0, live, arg0, arg1, (1u32).wrapping_add(x16), t824);
    x40 = t825;
    let t826 = b17_s_p(l, lid0, live, x32);
    x48 = t826;
    let t827 = [vdupq_n_u32(1023u32); 2];
    let t828: [uint32x4_t; 2] = [vandq_u32(x24[0], t827[0]), vandq_u32(x24[1], t827[1])];
    let t829 = [vdupq_n_u32(x32); 2];
    let t830: [uint32x4_t; 2] = [vmulq_u32(t828[0], t829[0]), vmulq_u32(t828[1], t829[1])];
    let t831 = b18_r0r1lls_p(l, lid0, live, arg0, arg1, x40, t830, 8u32);
    let t832: u64 = x48;
    let t833 = [vdupq_n_u32((t832) as u32); 2];
    let t834 = [vdupq_n_u32(((t832) >> 32) as u32); 2];
    let t835: [uint32x4_t; 4] = [vandq_u32(t831[0], t833[0]), vandq_u32(t831[1], t833[1]), vandq_u32(t831[2], t834[0]), vandq_u32(t831[3], t834[1])];
    x56 = t835;
    let t836: u64 = x48;
    let t837 = [vdupq_n_u32((t836) as u32); 2];
    let t838 = [vdupq_n_u32(((t836) >> 32) as u32); 2];
    let t839: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x56[0], t837[0]), vceqq_u32(x56[2], t838[0])), vandq_u32(vceqq_u32(x56[1], t837[1]), vceqq_u32(x56[3], t838[1]))];
    let mut t840: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t841: [uint32x4_t; 2] = [vandq_u32(live[0], t839[0]), vandq_u32(live[1], t839[1])];
    let t842: [uint32x4_t; 2] = [vbicq_u32(live[0], t839[0]), vbicq_u32(live[1], t839[1])];
    if any(&t841) {
        let t843 = b19_r0r1sl_p(l, lid0, t841, arg0, arg1, (x16).wrapping_add(1u32), x24);
        t840 = [vbslq_u32(t841[0], t843[0], t840[0]), vbslq_u32(t841[1], t843[1], t840[1]), vbslq_u32(t841[0], t843[2], t840[2]), vbslq_u32(t841[1], t843[3], t840[3])];
    }
    {
        t840 = [vbslq_u32(t842[0], x56[0], t840[0]), vbslq_u32(t842[1], x56[1], t840[1]), vbslq_u32(t842[0], x56[2], t840[2]), vbslq_u32(t842[1], x56[3], t840[3])];
    }
    res = t840;
    return res;
}

#[inline(always)]
unsafe fn b15_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t844 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    res = (((t844).wrapping_shr(((4u32).wrapping_mul(x16)) as u32)) & (15u32));
    return res;
}

#[inline(always)]
unsafe fn b15_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t845 = b20_r0r1s_f(l, lid0, arg0, arg1, 16u32);
    res = (((t845).wrapping_shr(((4u32).wrapping_mul(x16)) as u32)) & (15u32));
    return res;
}

#[inline(always)]
unsafe fn b16_r0r1ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: u32 = 0;
    let mut live = k;
    let mut res: u32 = 0;
    let t846 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 40u32);
    let t847 = b21_r0r1s_p(l, lid0, live, arg0, arg1, 0u32);
    x32 = ((x24).wrapping_mul(t846)).wrapping_add(t847);
    if ((x16) == (0u32)) {
        res = x32;
        return res;
    }
    let t848 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    let t849 = b22_ss_p(l, lid0, live, (t848).wrapping_sub((x24).wrapping_shl((10u32) as u32)), 1024u32);
    x40 = t849;
    let t850 = b21_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    res = (x32).wrapping_add((x40).wrapping_mul(t850));
    return res;
}

#[inline(always)]
unsafe fn b16_r0r1ss_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32, x24: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: u32 = 0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t851 = b20_r0r1s_f(l, lid0, arg0, arg1, 40u32);
    let t852 = b21_r0r1s_f(l, lid0, arg0, arg1, 0u32);
    x32 = ((x24).wrapping_mul(t851)).wrapping_add(t852);
    if ((x16) == (0u32)) {
        res = x32;
        return res;
    }
    let t853 = b20_r0r1s_f(l, lid0, arg0, arg1, 4u32);
    let t854 = b22_ss_f(l, lid0, (t853).wrapping_sub((x24).wrapping_shl((10u32) as u32)), 1024u32);
    x40 = t854;
    let t855 = b21_r0r1s_f(l, lid0, arg0, arg1, x16);
    res = (x32).wrapping_add((x40).wrapping_mul(t855));
    return res;
}

#[inline(always)]
unsafe fn b16_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t856 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 40u32);
    let t857 = [vdupq_n_u32(t856); 2];
    let t858: [uint32x4_t; 2] = [vmulq_u32(x24[0], t857[0]), vmulq_u32(x24[1], t857[1])];
    let t859 = b21_r0r1s_p(l, lid0, live, arg0, arg1, 0u32);
    let t860 = [vdupq_n_u32(t859); 2];
    let t861: [uint32x4_t; 2] = [vaddq_u32(t858[0], t860[0]), vaddq_u32(t858[1], t860[1])];
    x32 = t861;
    if ((x16) == (0u32)) {
        res = x32;
        return res;
    }
    let t862 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    let t863: [uint32x4_t; 2] = [vshlq_u32(x24[0], vdupq_n_s32(((10u32) & 31) as i32)), vshlq_u32(x24[1], vdupq_n_s32(((10u32) & 31) as i32))];
    let t864 = [vdupq_n_u32(t862); 2];
    let t865: [uint32x4_t; 2] = [vsubq_u32(t864[0], t863[0]), vsubq_u32(t864[1], t863[1])];
    let t866 = b22_ls_p(l, lid0, live, t865, 1024u32);
    x40 = t866;
    let t867 = b21_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    let t868 = [vdupq_n_u32(t867); 2];
    let t869: [uint32x4_t; 2] = [vmulq_u32(x40[0], t868[0]), vmulq_u32(x40[1], t868[1])];
    let t870: [uint32x4_t; 2] = [vaddq_u32(x32[0], t869[0]), vaddq_u32(x32[1], t869[1])];
    res = t870;
    return res;
}

#[inline(always)]
unsafe fn b16_r0r1sl_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t871 = b20_r0r1s_f(l, lid0, arg0, arg1, 40u32);
    let t872 = [vdupq_n_u32(t871); 2];
    let t873: [uint32x4_t; 2] = [vmulq_u32(x24[0], t872[0]), vmulq_u32(x24[1], t872[1])];
    let t874 = b21_r0r1s_f(l, lid0, arg0, arg1, 0u32);
    let t875 = [vdupq_n_u32(t874); 2];
    let t876: [uint32x4_t; 2] = [vaddq_u32(t873[0], t875[0]), vaddq_u32(t873[1], t875[1])];
    x32 = t876;
    if ((x16) == (0u32)) {
        res = x32;
        return res;
    }
    let t877 = b20_r0r1s_f(l, lid0, arg0, arg1, 4u32);
    let t878: [uint32x4_t; 2] = [vshlq_u32(x24[0], vdupq_n_s32(((10u32) & 31) as i32)), vshlq_u32(x24[1], vdupq_n_s32(((10u32) & 31) as i32))];
    let t879 = [vdupq_n_u32(t877); 2];
    let t880: [uint32x4_t; 2] = [vsubq_u32(t879[0], t878[0]), vsubq_u32(t879[1], t878[1])];
    let t881 = b22_ls_f(l, lid0, t880, 1024u32);
    x40 = t881;
    let t882 = b21_r0r1s_f(l, lid0, arg0, arg1, x16);
    let t883 = [vdupq_n_u32(t882); 2];
    let t884: [uint32x4_t; 2] = [vmulq_u32(x40[0], t883[0]), vmulq_u32(x40[1], t883[1])];
    let t885: [uint32x4_t; 2] = [vaddq_u32(x32[0], t884[0]), vaddq_u32(x32[1], t884[1])];
    res = t885;
    return res;
}

#[inline(always)]
unsafe fn b17_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t886: u64;
    if ((x0) >= (8u32)) {
        t886 = !(0u64);
    } else {
        t886 = ((1u64).wrapping_shl(((((x0).wrapping_mul(8u32)) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t886;
    return res;
}

#[inline(always)]
unsafe fn b17_s_f(l: &Launch, lid0: u32, x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t887: u64;
    if ((x0) >= (8u32)) {
        t887 = !(0u64);
    } else {
        t887 = ((1u64).wrapping_shl(((((x0).wrapping_mul(8u32)) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t887;
    return res;
}

#[inline(always)]
unsafe fn b17_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t888 = [vdupq_n_u32(8u32); 2];
    let t889: [uint32x4_t; 2] = [vcgeq_u32(x0[0], t888[0]), vcgeq_u32(x0[1], t888[1])];
    let mut t890: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t891: [uint32x4_t; 2] = [vandq_u32(live[0], t889[0]), vandq_u32(live[1], t889[1])];
    let t892: [uint32x4_t; 2] = [vbicq_u32(live[0], t889[0]), vbicq_u32(live[1], t889[1])];
    {
        let t893: u64 = !(0u64);
        let t894 = [vdupq_n_u32((t893) as u32); 2];
        let t895 = [vdupq_n_u32(((t893) >> 32) as u32); 2];
        let t896: [uint32x4_t; 4] = [t894[0], t894[1], t895[0], t895[1]];
        t890 = [vbslq_u32(t891[0], t896[0], t890[0]), vbslq_u32(t891[1], t896[1], t890[1]), vbslq_u32(t891[0], t896[2], t890[2]), vbslq_u32(t891[1], t896[3], t890[3])];
    }
    {
        let t897: [uint32x4_t; 2] = [vmulq_u32(x0[0], t888[0]), vmulq_u32(x0[1], t888[1])];
        let t898: u64 = 1u64;
        let t899 = [vdupq_n_u32((t898) as u32); 2];
        let t900: [uint32x4_t; 2] = [t897[0], t897[1]];
        let t901: [uint32x4_t; 2] = [vandq_u32(t900[0], vdupq_n_u32(63)), vandq_u32(t900[1], vdupq_n_u32(63))];
        let t902: [uint32x4_t; 4] = [vshlq_u32(t899[0], vreinterpretq_s32_u32(t901[0])), vshlq_u32(t899[1], vreinterpretq_s32_u32(t901[1])), vshlq_u32(t899[0], vsubq_s32(vreinterpretq_s32_u32(t901[0]), vdupq_n_s32(32))), vshlq_u32(t899[1], vsubq_s32(vreinterpretq_s32_u32(t901[1]), vdupq_n_s32(32)))];
        let t903: u64 = 1u64;
        let t904 = [vdupq_n_u32((t903) as u32); 2];
        let t905: [uint32x4_t; 2] = [vcltq_u32(t902[0], t904[0]), vcltq_u32(t902[1], t904[1])];
        let t906: [uint32x4_t; 4] = [vsubq_u32(t902[0], t904[0]), vsubq_u32(t902[1], t904[1]), vaddq_u32(vsubq_u32(t902[2], vdupq_n_u32(0)), t905[0]), vaddq_u32(vsubq_u32(t902[3], vdupq_n_u32(0)), t905[1])];
        t890 = [vbslq_u32(t892[0], t906[0], t890[0]), vbslq_u32(t892[1], t906[1], t890[1]), vbslq_u32(t892[0], t906[2], t890[2]), vbslq_u32(t892[1], t906[3], t890[3])];
    }
    res = t890;
    return res;
}

#[inline(always)]
unsafe fn b18_r0r1sns_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24f: u32, x24s: u32, x32: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t907 = b23_r0sns_p(l, lid0, live, arg0, x16, x24f, x24s, x32);
    res = t907;
    return res;
}

#[inline(always)]
unsafe fn b18_r0r1sns_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32, x24f: u32, x24s: u32, x32: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t908 = b23_r0sns_f(l, lid0, arg0, x16, x24f, x24s, x32);
    res = t908;
    return res;
}

#[inline(always)]
unsafe fn b18_r0r1lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2], x24: [uint32x4_t; 2], x32: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t909 = b23_r0lls_p(l, lid0, live, arg0, x16, x24, x32);
    res = t909;
    return res;
}

#[inline(always)]
unsafe fn b18_r0r1lls_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2], x24: [uint32x4_t; 2], x32: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t910 = b23_r0lls_f(l, lid0, arg0, x16, x24, x32);
    res = t910;
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
    let t911 = b20_r0r1s_p(l, lid0, live, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x16)));
    x32 = t911;
    let t912 = b20_r0r1s_p(l, lid0, live, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x16)));
    x40 = t912;
    let t913: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[1].as_ptr()))];
    let t914: [uint32x4_t; 2] = [vshlq_u32(t913[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(t913[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    x48 = t914;
    let t915 = [vdupq_n_u32(4u32); 2];
    let t916: [uint32x4_t; 2] = [vmulq_u32(x48[0], t915[0]), vmulq_u32(x48[1], t915[1])];
    let t917 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x40, t916, 4u32);
    let t918: [uint32x4_t; 2] = [t917[0], t917[1]];
    x56 = t918;
    let t919: [uint32x4_t; 2] = [vmulq_u32(x48[0], t915[0]), vmulq_u32(x48[1], t915[1])];
    let t920: [uint32x4_t; 2] = [vaddq_u32(t919[0], t915[0]), vaddq_u32(t919[1], t915[1])];
    let t921 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x40, t920, 4u32);
    let t922: [uint32x4_t; 2] = [t921[0], t921[1]];
    x64 = t922;
    let t923 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    x72 = ((t923).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
    let t924: [uint32x4_t; 2] = [vmulq_u32(x48[0], t915[0]), vmulq_u32(x48[1], t915[1])];
    let t925 = [vdupq_n_u32(x72); 2];
    let t926: [uint32x4_t; 2] = [vsubq_u32(t925[0], t924[0]), vsubq_u32(t925[1], t924[1])];
    let t927 = b25_sl_p(l, lid0, live, 4u32, t926);
    x80 = t927;
    let t928: [uint32x4_t; 2] = [vshlq_u32(t913[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(t913[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
    let t929 = [vdupq_n_u32(3u32); 2];
    let t930: [uint32x4_t; 2] = [vandq_u32(t928[0], t929[0]), vandq_u32(t928[1], t929[1])];
    x88 = t930;
    let t931 = [vdupq_n_u32(0u32); 2];
    let t932: [uint32x4_t; 2] = [vceqq_u32(x88[0], t931[0]), vceqq_u32(x88[1], t931[1])];
    let mut t933: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t934: [uint32x4_t; 2] = [vandq_u32(live[0], t932[0]), vandq_u32(live[1], t932[1])];
    let t935: [uint32x4_t; 2] = [vbicq_u32(live[0], t932[0]), vbicq_u32(live[1], t932[1])];
    {
        t933 = [vbslq_u32(t934[0], t931[0], t933[0]), vbslq_u32(t934[1], t931[1], t933[1])];
    }
    if any(&t935) {
        let t936 = [vdupq_n_u32(1u32); 2];
        let t937: [uint32x4_t; 2] = [vsubq_u32(x88[0], t936[0]), vsubq_u32(x88[1], t936[1])];
        let t938 = [vdupq_n_u32(2u32); 2];
        let t939: [uint32x4_t; 2] = [vmulq_u32(t938[0], t937[0]), vmulq_u32(t938[1], t937[1])];
        let t940: [uint32x4_t; 2] = [vaddq_u32(x56[0], t939[0]), vaddq_u32(x56[1], t939[1])];
        let t941 = b26_ll_p(l, lid0, t935, t940, x64);
        let t942 = b24_r0r1sls_p(l, lid0, t935, arg0, arg1, x32, t941, 2u32);
        let t943: [uint32x4_t; 2] = [t942[0], t942[1]];
        t933 = [vbslq_u32(t935[0], t943[0], t933[0]), vbslq_u32(t935[1], t943[1], t933[1])];
    }
    x96 = t933;
    let t944 = [vdupq_n_u32(2u32); 2];
    let t945: [uint32x4_t; 2] = [vmulq_u32(t944[0], x88[0]), vmulq_u32(t944[1], x88[1])];
    let t946: [uint32x4_t; 2] = [vaddq_u32(x56[0], t945[0]), vaddq_u32(x56[1], t945[1])];
    let t947 = b26_ll_p(l, lid0, live, t946, x64);
    let t948 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t947, 2u32);
    let t949: [uint32x4_t; 2] = [t948[0], t948[1]];
    x104 = t949;
    let t950: [uint32x4_t; 2] = [vmulq_u32(t944[0], x80[0]), vmulq_u32(t944[1], x80[1])];
    let t951: [uint32x4_t; 2] = [vaddq_u32(x56[0], t950[0]), vaddq_u32(x56[1], t950[1])];
    x112 = t951;
    let t952 = [vdupq_n_u32(255u32); 2];
    let t953: [uint32x4_t; 2] = [vandq_u32(t913[0], t952[0]), vandq_u32(t913[1], t952[1])];
    x120 = t953;
    x128 = 9259542123273814144u64;
    let t954: u64 = 72340172838076673u64;
    let t955 = [vdupq_n_u32((t954) as u32); 2];
    let t956 = [vdupq_n_u32(((t954) >> 32) as u32); 2];
    let t957: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x120[0]), vget_low_u32(t955[0]))), vreinterpretq_u32_u64(vmull_high_u32(x120[0], t955[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x120[1]), vget_low_u32(t955[1]))), vreinterpretq_u32_u64(vmull_high_u32(x120[1], t955[1])))];
    let t958: [uint32x4_t; 4] = [vmulq_u32(x120[0], t955[0]), vmulq_u32(x120[1], t955[1]), vmlaq_u32(t957[0], x120[0], t956[0]), vmlaq_u32(t957[1], x120[1], t956[1])];
    x136 = t958;
    x144 = x96;
    x152 = x96;
    let mut t959 = live;
    loop {
        // Lanes part: each its own rounds.
        let t961: [uint32x4_t; 2] = [vcltq_u32(x152[0], x104[0]), vcltq_u32(x152[1], x104[1])];
        t959 = [vandq_u32(t959[0], t961[0]), vandq_u32(t959[1], t961[1])];
        if !(any(&t959)) { break; }
        let t963: [uint32x4_t; 2] = [vaddq_u32(x112[0], x152[0]), vaddq_u32(x112[1], x152[1])];
        let t964 = b26_ll_p(l, lid0, t959, t963, x64);
        let t965 = b24_r0r1sls_p(l, lid0, t959, arg0, arg1, x32, t964, 8u32);
        x160 = [vbslq_u32(t959[0], t965[0], x160[0]), vbslq_u32(t959[1], t965[1], x160[1]), vbslq_u32(t959[0], t965[2], x160[2]), vbslq_u32(t959[1], t965[3], x160[3])];
        let t966: u64 = x128;
        let t967 = [vdupq_n_u32((t966) as u32); 2];
        let t968 = [vdupq_n_u32(((t966) >> 32) as u32); 2];
        let t969: [uint32x4_t; 4] = [vorrq_u32(x160[0], t967[0]), vorrq_u32(x160[1], t967[1]), vorrq_u32(x160[2], t968[0]), vorrq_u32(x160[3], t968[1])];
        let t970: u64 = !(x128);
        let t971 = [vdupq_n_u32((t970) as u32); 2];
        let t972 = [vdupq_n_u32(((t970) >> 32) as u32); 2];
        let t973: [uint32x4_t; 4] = [vandq_u32(x136[0], t971[0]), vandq_u32(x136[1], t971[1]), vandq_u32(x136[2], t972[0]), vandq_u32(x136[3], t972[1])];
        let t974: [uint32x4_t; 2] = [vcltq_u32(t969[0], t973[0]), vcltq_u32(t969[1], t973[1])];
        let t975: [uint32x4_t; 4] = [vsubq_u32(t969[0], t973[0]), vsubq_u32(t969[1], t973[1]), vaddq_u32(vsubq_u32(t969[2], t973[2]), t974[0]), vaddq_u32(vsubq_u32(t969[3], t973[3]), t974[1])];
        x168 = [vbslq_u32(t959[0], t975[0], x168[0]), vbslq_u32(t959[1], t975[1], x168[1]), vbslq_u32(t959[0], t975[2], x168[2]), vbslq_u32(t959[1], t975[3], x168[3])];
        let t976: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x160[r]));
        let t977: [uint32x4_t; 4] = [vandq_u32(t976[0], x136[0]), vandq_u32(t976[1], x136[1]), vandq_u32(t976[2], x136[2]), vandq_u32(t976[3], x136[3])];
        let t978: [uint32x4_t; 4] = [veorq_u32(x160[0], x136[0]), veorq_u32(x160[1], x136[1]), veorq_u32(x160[2], x136[2]), veorq_u32(x160[3], x136[3])];
        let t979: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(t978[r]));
        let t980: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x168[r]));
        let t981: [uint32x4_t; 4] = [vandq_u32(t979[0], t980[0]), vandq_u32(t979[1], t980[1]), vandq_u32(t979[2], t980[2]), vandq_u32(t979[3], t980[3])];
        let t982: [uint32x4_t; 4] = [vorrq_u32(t977[0], t981[0]), vorrq_u32(t977[1], t981[1]), vorrq_u32(t977[2], t981[2]), vorrq_u32(t977[3], t981[3])];
        let t983: u64 = x128;
        let t984 = [vdupq_n_u32((t983) as u32); 2];
        let t985 = [vdupq_n_u32(((t983) >> 32) as u32); 2];
        let t986: [uint32x4_t; 4] = [vandq_u32(t984[0], t982[0]), vandq_u32(t984[1], t982[1]), vandq_u32(t985[0], t982[2]), vandq_u32(t985[1], t982[3])];
        x176 = t986;
        let t987: [uint32x4_t; 2] = [vsubq_u32(x104[0], x152[0]), vsubq_u32(x104[1], x152[1])];
        x184 = t987;
        let t988 = [vdupq_n_u32(8u32); 2];
        let t989: [uint32x4_t; 2] = [vcgeq_u32(x184[0], t988[0]), vcgeq_u32(x184[1], t988[1])];
        let mut t990: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t991: [uint32x4_t; 2] = [vandq_u32(t959[0], t989[0]), vandq_u32(t959[1], t989[1])];
        let t992: [uint32x4_t; 2] = [vbicq_u32(t959[0], t989[0]), vbicq_u32(t959[1], t989[1])];
        {
            t990 = [vbslq_u32(t991[0], x176[0], t990[0]), vbslq_u32(t991[1], x176[1], t990[1]), vbslq_u32(t991[0], x176[2], t990[2]), vbslq_u32(t991[1], x176[3], t990[3])];
        }
        {
            let t993: [uint32x4_t; 2] = [vmulq_u32(t988[0], x184[0]), vmulq_u32(t988[1], x184[1])];
            let t994: u64 = 1u64;
            let t995 = [vdupq_n_u32((t994) as u32); 2];
            let t996: [uint32x4_t; 2] = [t993[0], t993[1]];
            let t997: [uint32x4_t; 2] = [vandq_u32(t996[0], vdupq_n_u32(63)), vandq_u32(t996[1], vdupq_n_u32(63))];
            let t998: [uint32x4_t; 4] = [vshlq_u32(t995[0], vreinterpretq_s32_u32(t997[0])), vshlq_u32(t995[1], vreinterpretq_s32_u32(t997[1])), vshlq_u32(t995[0], vsubq_s32(vreinterpretq_s32_u32(t997[0]), vdupq_n_s32(32))), vshlq_u32(t995[1], vsubq_s32(vreinterpretq_s32_u32(t997[1]), vdupq_n_s32(32)))];
            let t999: u64 = 1u64;
            let t1000 = [vdupq_n_u32((t999) as u32); 2];
            let t1001: [uint32x4_t; 2] = [vcltq_u32(t998[0], t1000[0]), vcltq_u32(t998[1], t1000[1])];
            let t1002: [uint32x4_t; 4] = [vsubq_u32(t998[0], t1000[0]), vsubq_u32(t998[1], t1000[1]), vaddq_u32(vsubq_u32(t998[2], vdupq_n_u32(0)), t1001[0]), vaddq_u32(vsubq_u32(t998[3], vdupq_n_u32(0)), t1001[1])];
            let t1003: [uint32x4_t; 4] = [vandq_u32(x176[0], t1002[0]), vandq_u32(x176[1], t1002[1]), vandq_u32(x176[2], t1002[2]), vandq_u32(x176[3], t1002[3])];
            t990 = [vbslq_u32(t992[0], t1003[0], t990[0]), vbslq_u32(t992[1], t1003[1], t990[1]), vbslq_u32(t992[0], t1003[2], t990[2]), vbslq_u32(t992[1], t1003[3], t990[3])];
        }
        let t1004 = b27_l_p(l, lid0, t959, t990);
        let t1005: [uint32x4_t; 2] = [vaddq_u32(x144[0], t1004[0]), vaddq_u32(x144[1], t1004[1])];
        x144 = [vbslq_u32(t959[0], t1005[0], x144[0]), vbslq_u32(t959[1], t1005[1], x144[1])];
        let t1006: [uint32x4_t; 2] = [vandq_u32(t959[0], live[0]), vandq_u32(t959[1], live[1])];
        x152 = [vaddq_u32(x152[0], vandq_u32(t1006[0], vdupq_n_u32(8))), vaddq_u32(x152[1], vandq_u32(t1006[1], vdupq_n_u32(8)))];
    }
    x96 = x144;
    let t1007 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    let t1008 = [vdupq_n_u32((((t1007).wrapping_shr((((((4u32).wrapping_mul(x16)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32))); 2];
    x152 = t1008;
    let t1009 = [vdupq_n_u32(1u32); 2];
    let t1010: [uint32x4_t; 2] = [vsubq_u32(x80[0], t1009[0]), vsubq_u32(x80[1], t1009[1])];
    let t1011: [uint32x4_t; 2] = [vmulq_u32(t944[0], t1010[0]), vmulq_u32(t944[1], t1010[1])];
    let t1012: [uint32x4_t; 2] = [vaddq_u32(x56[0], t1011[0]), vaddq_u32(x56[1], t1011[1])];
    let t1013 = b26_ll_p(l, lid0, live, t1012, x64);
    let t1014 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t1013, 2u32);
    let t1015: [uint32x4_t; 2] = [t1014[0], t1014[1]];
    let t1016: [uint32x4_t; 4] = [t1015[0], t1015[1], vdupq_n_u32(0), vdupq_n_u32(0)];
    x160 = t1016;
    let t1017: [uint32x4_t; 2] = [x160[0], x160[1]];
    let t1018: [uint32x4_t; 2] = [vaddq_u32(x112[0], t1017[0]), vaddq_u32(x112[1], t1017[1])];
    let t1019: [uint32x4_t; 2] = [vmulq_u32(x96[0], x152[0]), vmulq_u32(x96[1], x152[1])];
    let t1020: [uint32x4_t; 2] = [vaddq_u32(t1018[0], t1019[0]), vaddq_u32(t1018[1], t1019[1])];
    let t1021 = b26_ll_p(l, lid0, live, t1020, x64);
    let t1022: [uint32x4_t; 4] = [t1021[0], t1021[1], vdupq_n_u32(0), vdupq_n_u32(0)];
    x168 = t1022;
    let t1023: [uint32x4_t; 2] = [x168[0], x168[1]];
    let t1024 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t1023, 8u32);
    let t1025 = b17_l_p(l, lid0, live, x152);
    let t1026: [uint32x4_t; 4] = [vandq_u32(t1024[0], t1025[0]), vandq_u32(t1024[1], t1025[1]), vandq_u32(t1024[2], t1025[2]), vandq_u32(t1024[3], t1025[3])];
    res = t1026;
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
    let t1027 = b20_r0r1s_p(l, lid0, live, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x16)));
    x32 = t1027;
    let t1028 = b20_r0r1s_p(l, lid0, live, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x16)));
    x40 = t1028;
    x48 = (x24 >> 10);
    let t1029 = b24_r0r1sss_p(l, lid0, live, arg0, arg1, x40, (x48).wrapping_mul(4u32), 4u32);
    x56 = ((t1029) as u32);
    let t1030 = b24_r0r1sss_p(l, lid0, live, arg0, arg1, x40, ((x48).wrapping_mul(4u32)).wrapping_add(4u32), 4u32);
    x64 = ((t1030) as u32);
    let t1031 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    x72 = ((t1031).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
    let t1032 = b25_ss_p(l, lid0, live, 4u32, (x72).wrapping_sub((x48).wrapping_mul(4u32)));
    x80 = t1032;
    x88 = (((x24 >> 8)) & (3u32));
    let t1033: u32;
    if ((x88) == (0u32)) {
        t1033 = 0u32;
    } else {
        let t1034 = b26_ss_p(l, lid0, live, (x56).wrapping_add((2u32).wrapping_mul((x88).wrapping_sub(1u32))), x64);
        let t1035 = b24_r0r1sss_p(l, lid0, live, arg0, arg1, x32, t1034, 2u32);
        t1033 = ((t1035) as u32);
    }
    let t1036 = [vdupq_n_u32(t1033); 2];
    x96 = t1036;
    let t1037 = b26_ss_p(l, lid0, live, (x56).wrapping_add((2u32).wrapping_mul(x88)), x64);
    let t1038 = b24_r0r1sss_p(l, lid0, live, arg0, arg1, x32, t1037, 2u32);
    x104 = ((t1038) as u32);
    x112 = (x56).wrapping_add((2u32).wrapping_mul(x80));
    x120 = (x24 & 255u32);
    x128 = 9259542123273814144u64;
    let t1039: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x120), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x120), vld1q_u32(LANE[1].as_ptr()))];
    let t1040: u64 = 72340172838076673u64;
    let t1041 = [vdupq_n_u32((t1040) as u32); 2];
    let t1042 = [vdupq_n_u32(((t1040) >> 32) as u32); 2];
    let t1043: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(t1039[0]), vget_low_u32(t1041[0]))), vreinterpretq_u32_u64(vmull_high_u32(t1039[0], t1041[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(t1039[1]), vget_low_u32(t1041[1]))), vreinterpretq_u32_u64(vmull_high_u32(t1039[1], t1041[1])))];
    let t1044: [uint32x4_t; 4] = [vmulq_u32(t1039[0], t1041[0]), vmulq_u32(t1039[1], t1041[1]), vmlaq_u32(t1043[0], t1039[0], t1042[0]), vmlaq_u32(t1043[1], t1039[1], t1042[1])];
    x136 = t1044;
    x144 = x96;
    x152 = x96;
    let mut t1045 = live;
    loop {
        // Lanes part: each its own rounds.
        let t1047 = [vdupq_n_u32(x104); 2];
        let t1048: [uint32x4_t; 2] = [vcltq_u32(x152[0], t1047[0]), vcltq_u32(x152[1], t1047[1])];
        t1045 = [vandq_u32(t1045[0], t1048[0]), vandq_u32(t1045[1], t1048[1])];
        if !(any(&t1045)) { break; }
        let t1050 = [vdupq_n_u32(x112); 2];
        let t1051: [uint32x4_t; 2] = [vaddq_u32(t1050[0], x152[0]), vaddq_u32(t1050[1], x152[1])];
        let t1052 = b26_ls_p(l, lid0, t1045, t1051, x64);
        let t1053 = b24_r0r1sls_p(l, lid0, t1045, arg0, arg1, x32, t1052, 8u32);
        x160 = [vbslq_u32(t1045[0], t1053[0], x160[0]), vbslq_u32(t1045[1], t1053[1], x160[1]), vbslq_u32(t1045[0], t1053[2], x160[2]), vbslq_u32(t1045[1], t1053[3], x160[3])];
        let t1054: u64 = x128;
        let t1055 = [vdupq_n_u32((t1054) as u32); 2];
        let t1056 = [vdupq_n_u32(((t1054) >> 32) as u32); 2];
        let t1057: [uint32x4_t; 4] = [vorrq_u32(x160[0], t1055[0]), vorrq_u32(x160[1], t1055[1]), vorrq_u32(x160[2], t1056[0]), vorrq_u32(x160[3], t1056[1])];
        let t1058: u64 = !(x128);
        let t1059 = [vdupq_n_u32((t1058) as u32); 2];
        let t1060 = [vdupq_n_u32(((t1058) >> 32) as u32); 2];
        let t1061: [uint32x4_t; 4] = [vandq_u32(x136[0], t1059[0]), vandq_u32(x136[1], t1059[1]), vandq_u32(x136[2], t1060[0]), vandq_u32(x136[3], t1060[1])];
        let t1062: [uint32x4_t; 2] = [vcltq_u32(t1057[0], t1061[0]), vcltq_u32(t1057[1], t1061[1])];
        let t1063: [uint32x4_t; 4] = [vsubq_u32(t1057[0], t1061[0]), vsubq_u32(t1057[1], t1061[1]), vaddq_u32(vsubq_u32(t1057[2], t1061[2]), t1062[0]), vaddq_u32(vsubq_u32(t1057[3], t1061[3]), t1062[1])];
        x168 = [vbslq_u32(t1045[0], t1063[0], x168[0]), vbslq_u32(t1045[1], t1063[1], x168[1]), vbslq_u32(t1045[0], t1063[2], x168[2]), vbslq_u32(t1045[1], t1063[3], x168[3])];
        let t1064: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x160[r]));
        let t1065: [uint32x4_t; 4] = [vandq_u32(t1064[0], x136[0]), vandq_u32(t1064[1], x136[1]), vandq_u32(t1064[2], x136[2]), vandq_u32(t1064[3], x136[3])];
        let t1066: [uint32x4_t; 4] = [veorq_u32(x160[0], x136[0]), veorq_u32(x160[1], x136[1]), veorq_u32(x160[2], x136[2]), veorq_u32(x160[3], x136[3])];
        let t1067: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(t1066[r]));
        let t1068: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x168[r]));
        let t1069: [uint32x4_t; 4] = [vandq_u32(t1067[0], t1068[0]), vandq_u32(t1067[1], t1068[1]), vandq_u32(t1067[2], t1068[2]), vandq_u32(t1067[3], t1068[3])];
        let t1070: [uint32x4_t; 4] = [vorrq_u32(t1065[0], t1069[0]), vorrq_u32(t1065[1], t1069[1]), vorrq_u32(t1065[2], t1069[2]), vorrq_u32(t1065[3], t1069[3])];
        let t1071: u64 = x128;
        let t1072 = [vdupq_n_u32((t1071) as u32); 2];
        let t1073 = [vdupq_n_u32(((t1071) >> 32) as u32); 2];
        let t1074: [uint32x4_t; 4] = [vandq_u32(t1072[0], t1070[0]), vandq_u32(t1072[1], t1070[1]), vandq_u32(t1073[0], t1070[2]), vandq_u32(t1073[1], t1070[3])];
        x176 = t1074;
        let t1075 = [vdupq_n_u32(x104); 2];
        let t1076: [uint32x4_t; 2] = [vsubq_u32(t1075[0], x152[0]), vsubq_u32(t1075[1], x152[1])];
        x184 = t1076;
        let t1077 = [vdupq_n_u32(8u32); 2];
        let t1078: [uint32x4_t; 2] = [vcgeq_u32(x184[0], t1077[0]), vcgeq_u32(x184[1], t1077[1])];
        let mut t1079: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t1080: [uint32x4_t; 2] = [vandq_u32(t1045[0], t1078[0]), vandq_u32(t1045[1], t1078[1])];
        let t1081: [uint32x4_t; 2] = [vbicq_u32(t1045[0], t1078[0]), vbicq_u32(t1045[1], t1078[1])];
        {
            t1079 = [vbslq_u32(t1080[0], x176[0], t1079[0]), vbslq_u32(t1080[1], x176[1], t1079[1]), vbslq_u32(t1080[0], x176[2], t1079[2]), vbslq_u32(t1080[1], x176[3], t1079[3])];
        }
        {
            let t1082: [uint32x4_t; 2] = [vmulq_u32(t1077[0], x184[0]), vmulq_u32(t1077[1], x184[1])];
            let t1083: u64 = 1u64;
            let t1084 = [vdupq_n_u32((t1083) as u32); 2];
            let t1085: [uint32x4_t; 2] = [t1082[0], t1082[1]];
            let t1086: [uint32x4_t; 2] = [vandq_u32(t1085[0], vdupq_n_u32(63)), vandq_u32(t1085[1], vdupq_n_u32(63))];
            let t1087: [uint32x4_t; 4] = [vshlq_u32(t1084[0], vreinterpretq_s32_u32(t1086[0])), vshlq_u32(t1084[1], vreinterpretq_s32_u32(t1086[1])), vshlq_u32(t1084[0], vsubq_s32(vreinterpretq_s32_u32(t1086[0]), vdupq_n_s32(32))), vshlq_u32(t1084[1], vsubq_s32(vreinterpretq_s32_u32(t1086[1]), vdupq_n_s32(32)))];
            let t1088: u64 = 1u64;
            let t1089 = [vdupq_n_u32((t1088) as u32); 2];
            let t1090: [uint32x4_t; 2] = [vcltq_u32(t1087[0], t1089[0]), vcltq_u32(t1087[1], t1089[1])];
            let t1091: [uint32x4_t; 4] = [vsubq_u32(t1087[0], t1089[0]), vsubq_u32(t1087[1], t1089[1]), vaddq_u32(vsubq_u32(t1087[2], vdupq_n_u32(0)), t1090[0]), vaddq_u32(vsubq_u32(t1087[3], vdupq_n_u32(0)), t1090[1])];
            let t1092: [uint32x4_t; 4] = [vandq_u32(x176[0], t1091[0]), vandq_u32(x176[1], t1091[1]), vandq_u32(x176[2], t1091[2]), vandq_u32(x176[3], t1091[3])];
            t1079 = [vbslq_u32(t1081[0], t1092[0], t1079[0]), vbslq_u32(t1081[1], t1092[1], t1079[1]), vbslq_u32(t1081[0], t1092[2], t1079[2]), vbslq_u32(t1081[1], t1092[3], t1079[3])];
        }
        let t1093 = b27_l_p(l, lid0, t1045, t1079);
        let t1094: [uint32x4_t; 2] = [vaddq_u32(x144[0], t1093[0]), vaddq_u32(x144[1], t1093[1])];
        x144 = [vbslq_u32(t1045[0], t1094[0], x144[0]), vbslq_u32(t1045[1], t1094[1], x144[1])];
        let t1095: [uint32x4_t; 2] = [vandq_u32(t1045[0], live[0]), vandq_u32(t1045[1], live[1])];
        x152 = [vaddq_u32(x152[0], vandq_u32(t1095[0], vdupq_n_u32(8))), vaddq_u32(x152[1], vandq_u32(t1095[1], vdupq_n_u32(8)))];
    }
    x96 = x144;
    let t1096 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    let t1097 = [vdupq_n_u32((((t1096).wrapping_shr((((((4u32).wrapping_mul(x16)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32))); 2];
    x152 = t1097;
    let t1098 = b26_ss_p(l, lid0, live, (x56).wrapping_add((2u32).wrapping_mul((x80).wrapping_sub(1u32))), x64);
    let t1099 = b24_r0r1sss_p(l, lid0, live, arg0, arg1, x32, t1098, 2u32);
    let t1100: u64 = ((((t1099) as u32)) as u64);
    let t1101 = [vdupq_n_u32((t1100) as u32); 2];
    let t1102 = [vdupq_n_u32(((t1100) >> 32) as u32); 2];
    let t1103: [uint32x4_t; 4] = [t1101[0], t1101[1], t1102[0], t1102[1]];
    x160 = t1103;
    let t1104 = [vdupq_n_u32(x112); 2];
    let t1105: [uint32x4_t; 2] = [x160[0], x160[1]];
    let t1106: [uint32x4_t; 2] = [vaddq_u32(t1104[0], t1105[0]), vaddq_u32(t1104[1], t1105[1])];
    let t1107: [uint32x4_t; 2] = [vmulq_u32(x96[0], x152[0]), vmulq_u32(x96[1], x152[1])];
    let t1108: [uint32x4_t; 2] = [vaddq_u32(t1106[0], t1107[0]), vaddq_u32(t1106[1], t1107[1])];
    let t1109 = b26_ls_p(l, lid0, live, t1108, x64);
    let t1110: [uint32x4_t; 4] = [t1109[0], t1109[1], vdupq_n_u32(0), vdupq_n_u32(0)];
    x168 = t1110;
    let t1111: [uint32x4_t; 2] = [x168[0], x168[1]];
    let t1112 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t1111, 8u32);
    let t1113 = b17_l_p(l, lid0, live, x152);
    let t1114: [uint32x4_t; 4] = [vandq_u32(t1112[0], t1113[0]), vandq_u32(t1112[1], t1113[1]), vandq_u32(t1112[2], t1113[2]), vandq_u32(t1112[3], t1113[3])];
    res = t1114;
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
    let t1115 = b20_r0r1s_p(l, lid0, live, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x16)));
    x32 = t1115;
    let t1116 = b20_r0r1s_p(l, lid0, live, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x16)));
    x40 = t1116;
    let t1117: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    x48 = t1117;
    let t1118 = [vdupq_n_u32(4u32); 2];
    let t1119: [uint32x4_t; 2] = [vmulq_u32(x48[0], t1118[0]), vmulq_u32(x48[1], t1118[1])];
    let t1120 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x40, t1119, 4u32);
    let t1121: [uint32x4_t; 2] = [t1120[0], t1120[1]];
    x56 = t1121;
    let t1122: [uint32x4_t; 2] = [vmulq_u32(x48[0], t1118[0]), vmulq_u32(x48[1], t1118[1])];
    let t1123: [uint32x4_t; 2] = [vaddq_u32(t1122[0], t1118[0]), vaddq_u32(t1122[1], t1118[1])];
    let t1124 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x40, t1123, 4u32);
    let t1125: [uint32x4_t; 2] = [t1124[0], t1124[1]];
    x64 = t1125;
    let t1126 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    x72 = ((t1126).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
    let t1127: [uint32x4_t; 2] = [vmulq_u32(x48[0], t1118[0]), vmulq_u32(x48[1], t1118[1])];
    let t1128 = [vdupq_n_u32(x72); 2];
    let t1129: [uint32x4_t; 2] = [vsubq_u32(t1128[0], t1127[0]), vsubq_u32(t1128[1], t1127[1])];
    let t1130 = b25_sl_p(l, lid0, live, 4u32, t1129);
    x80 = t1130;
    let t1131: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
    let t1132 = [vdupq_n_u32(3u32); 2];
    let t1133: [uint32x4_t; 2] = [vandq_u32(t1131[0], t1132[0]), vandq_u32(t1131[1], t1132[1])];
    x88 = t1133;
    let t1134 = [vdupq_n_u32(0u32); 2];
    let t1135: [uint32x4_t; 2] = [vceqq_u32(x88[0], t1134[0]), vceqq_u32(x88[1], t1134[1])];
    let mut t1136: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1137: [uint32x4_t; 2] = [vandq_u32(live[0], t1135[0]), vandq_u32(live[1], t1135[1])];
    let t1138: [uint32x4_t; 2] = [vbicq_u32(live[0], t1135[0]), vbicq_u32(live[1], t1135[1])];
    {
        t1136 = [vbslq_u32(t1137[0], t1134[0], t1136[0]), vbslq_u32(t1137[1], t1134[1], t1136[1])];
    }
    if any(&t1138) {
        let t1139 = [vdupq_n_u32(1u32); 2];
        let t1140: [uint32x4_t; 2] = [vsubq_u32(x88[0], t1139[0]), vsubq_u32(x88[1], t1139[1])];
        let t1141 = [vdupq_n_u32(2u32); 2];
        let t1142: [uint32x4_t; 2] = [vmulq_u32(t1141[0], t1140[0]), vmulq_u32(t1141[1], t1140[1])];
        let t1143: [uint32x4_t; 2] = [vaddq_u32(x56[0], t1142[0]), vaddq_u32(x56[1], t1142[1])];
        let t1144 = b26_ll_p(l, lid0, t1138, t1143, x64);
        let t1145 = b24_r0r1sls_p(l, lid0, t1138, arg0, arg1, x32, t1144, 2u32);
        let t1146: [uint32x4_t; 2] = [t1145[0], t1145[1]];
        t1136 = [vbslq_u32(t1138[0], t1146[0], t1136[0]), vbslq_u32(t1138[1], t1146[1], t1136[1])];
    }
    x96 = t1136;
    let t1147 = [vdupq_n_u32(2u32); 2];
    let t1148: [uint32x4_t; 2] = [vmulq_u32(t1147[0], x88[0]), vmulq_u32(t1147[1], x88[1])];
    let t1149: [uint32x4_t; 2] = [vaddq_u32(x56[0], t1148[0]), vaddq_u32(x56[1], t1148[1])];
    let t1150 = b26_ll_p(l, lid0, live, t1149, x64);
    let t1151 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t1150, 2u32);
    let t1152: [uint32x4_t; 2] = [t1151[0], t1151[1]];
    x104 = t1152;
    let t1153: [uint32x4_t; 2] = [vmulq_u32(t1147[0], x80[0]), vmulq_u32(t1147[1], x80[1])];
    let t1154: [uint32x4_t; 2] = [vaddq_u32(x56[0], t1153[0]), vaddq_u32(x56[1], t1153[1])];
    x112 = t1154;
    let t1155 = [vdupq_n_u32(255u32); 2];
    let t1156: [uint32x4_t; 2] = [vandq_u32(x24[0], t1155[0]), vandq_u32(x24[1], t1155[1])];
    x120 = t1156;
    x128 = 9259542123273814144u64;
    let t1157: u64 = 72340172838076673u64;
    let t1158 = [vdupq_n_u32((t1157) as u32); 2];
    let t1159 = [vdupq_n_u32(((t1157) >> 32) as u32); 2];
    let t1160: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x120[0]), vget_low_u32(t1158[0]))), vreinterpretq_u32_u64(vmull_high_u32(x120[0], t1158[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x120[1]), vget_low_u32(t1158[1]))), vreinterpretq_u32_u64(vmull_high_u32(x120[1], t1158[1])))];
    let t1161: [uint32x4_t; 4] = [vmulq_u32(x120[0], t1158[0]), vmulq_u32(x120[1], t1158[1]), vmlaq_u32(t1160[0], x120[0], t1159[0]), vmlaq_u32(t1160[1], x120[1], t1159[1])];
    x136 = t1161;
    x144 = x96;
    x152 = x96;
    let mut t1162 = live;
    loop {
        // Lanes part: each its own rounds.
        let t1164: [uint32x4_t; 2] = [vcltq_u32(x152[0], x104[0]), vcltq_u32(x152[1], x104[1])];
        t1162 = [vandq_u32(t1162[0], t1164[0]), vandq_u32(t1162[1], t1164[1])];
        if !(any(&t1162)) { break; }
        let t1166: [uint32x4_t; 2] = [vaddq_u32(x112[0], x152[0]), vaddq_u32(x112[1], x152[1])];
        let t1167 = b26_ll_p(l, lid0, t1162, t1166, x64);
        let t1168 = b24_r0r1sls_p(l, lid0, t1162, arg0, arg1, x32, t1167, 8u32);
        x160 = [vbslq_u32(t1162[0], t1168[0], x160[0]), vbslq_u32(t1162[1], t1168[1], x160[1]), vbslq_u32(t1162[0], t1168[2], x160[2]), vbslq_u32(t1162[1], t1168[3], x160[3])];
        let t1169: u64 = x128;
        let t1170 = [vdupq_n_u32((t1169) as u32); 2];
        let t1171 = [vdupq_n_u32(((t1169) >> 32) as u32); 2];
        let t1172: [uint32x4_t; 4] = [vorrq_u32(x160[0], t1170[0]), vorrq_u32(x160[1], t1170[1]), vorrq_u32(x160[2], t1171[0]), vorrq_u32(x160[3], t1171[1])];
        let t1173: u64 = !(x128);
        let t1174 = [vdupq_n_u32((t1173) as u32); 2];
        let t1175 = [vdupq_n_u32(((t1173) >> 32) as u32); 2];
        let t1176: [uint32x4_t; 4] = [vandq_u32(x136[0], t1174[0]), vandq_u32(x136[1], t1174[1]), vandq_u32(x136[2], t1175[0]), vandq_u32(x136[3], t1175[1])];
        let t1177: [uint32x4_t; 2] = [vcltq_u32(t1172[0], t1176[0]), vcltq_u32(t1172[1], t1176[1])];
        let t1178: [uint32x4_t; 4] = [vsubq_u32(t1172[0], t1176[0]), vsubq_u32(t1172[1], t1176[1]), vaddq_u32(vsubq_u32(t1172[2], t1176[2]), t1177[0]), vaddq_u32(vsubq_u32(t1172[3], t1176[3]), t1177[1])];
        x168 = [vbslq_u32(t1162[0], t1178[0], x168[0]), vbslq_u32(t1162[1], t1178[1], x168[1]), vbslq_u32(t1162[0], t1178[2], x168[2]), vbslq_u32(t1162[1], t1178[3], x168[3])];
        let t1179: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x160[r]));
        let t1180: [uint32x4_t; 4] = [vandq_u32(t1179[0], x136[0]), vandq_u32(t1179[1], x136[1]), vandq_u32(t1179[2], x136[2]), vandq_u32(t1179[3], x136[3])];
        let t1181: [uint32x4_t; 4] = [veorq_u32(x160[0], x136[0]), veorq_u32(x160[1], x136[1]), veorq_u32(x160[2], x136[2]), veorq_u32(x160[3], x136[3])];
        let t1182: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(t1181[r]));
        let t1183: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x168[r]));
        let t1184: [uint32x4_t; 4] = [vandq_u32(t1182[0], t1183[0]), vandq_u32(t1182[1], t1183[1]), vandq_u32(t1182[2], t1183[2]), vandq_u32(t1182[3], t1183[3])];
        let t1185: [uint32x4_t; 4] = [vorrq_u32(t1180[0], t1184[0]), vorrq_u32(t1180[1], t1184[1]), vorrq_u32(t1180[2], t1184[2]), vorrq_u32(t1180[3], t1184[3])];
        let t1186: u64 = x128;
        let t1187 = [vdupq_n_u32((t1186) as u32); 2];
        let t1188 = [vdupq_n_u32(((t1186) >> 32) as u32); 2];
        let t1189: [uint32x4_t; 4] = [vandq_u32(t1187[0], t1185[0]), vandq_u32(t1187[1], t1185[1]), vandq_u32(t1188[0], t1185[2]), vandq_u32(t1188[1], t1185[3])];
        x176 = t1189;
        let t1190: [uint32x4_t; 2] = [vsubq_u32(x104[0], x152[0]), vsubq_u32(x104[1], x152[1])];
        x184 = t1190;
        let t1191 = [vdupq_n_u32(8u32); 2];
        let t1192: [uint32x4_t; 2] = [vcgeq_u32(x184[0], t1191[0]), vcgeq_u32(x184[1], t1191[1])];
        let mut t1193: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t1194: [uint32x4_t; 2] = [vandq_u32(t1162[0], t1192[0]), vandq_u32(t1162[1], t1192[1])];
        let t1195: [uint32x4_t; 2] = [vbicq_u32(t1162[0], t1192[0]), vbicq_u32(t1162[1], t1192[1])];
        {
            t1193 = [vbslq_u32(t1194[0], x176[0], t1193[0]), vbslq_u32(t1194[1], x176[1], t1193[1]), vbslq_u32(t1194[0], x176[2], t1193[2]), vbslq_u32(t1194[1], x176[3], t1193[3])];
        }
        {
            let t1196: [uint32x4_t; 2] = [vmulq_u32(t1191[0], x184[0]), vmulq_u32(t1191[1], x184[1])];
            let t1197: u64 = 1u64;
            let t1198 = [vdupq_n_u32((t1197) as u32); 2];
            let t1199: [uint32x4_t; 2] = [t1196[0], t1196[1]];
            let t1200: [uint32x4_t; 2] = [vandq_u32(t1199[0], vdupq_n_u32(63)), vandq_u32(t1199[1], vdupq_n_u32(63))];
            let t1201: [uint32x4_t; 4] = [vshlq_u32(t1198[0], vreinterpretq_s32_u32(t1200[0])), vshlq_u32(t1198[1], vreinterpretq_s32_u32(t1200[1])), vshlq_u32(t1198[0], vsubq_s32(vreinterpretq_s32_u32(t1200[0]), vdupq_n_s32(32))), vshlq_u32(t1198[1], vsubq_s32(vreinterpretq_s32_u32(t1200[1]), vdupq_n_s32(32)))];
            let t1202: u64 = 1u64;
            let t1203 = [vdupq_n_u32((t1202) as u32); 2];
            let t1204: [uint32x4_t; 2] = [vcltq_u32(t1201[0], t1203[0]), vcltq_u32(t1201[1], t1203[1])];
            let t1205: [uint32x4_t; 4] = [vsubq_u32(t1201[0], t1203[0]), vsubq_u32(t1201[1], t1203[1]), vaddq_u32(vsubq_u32(t1201[2], vdupq_n_u32(0)), t1204[0]), vaddq_u32(vsubq_u32(t1201[3], vdupq_n_u32(0)), t1204[1])];
            let t1206: [uint32x4_t; 4] = [vandq_u32(x176[0], t1205[0]), vandq_u32(x176[1], t1205[1]), vandq_u32(x176[2], t1205[2]), vandq_u32(x176[3], t1205[3])];
            t1193 = [vbslq_u32(t1195[0], t1206[0], t1193[0]), vbslq_u32(t1195[1], t1206[1], t1193[1]), vbslq_u32(t1195[0], t1206[2], t1193[2]), vbslq_u32(t1195[1], t1206[3], t1193[3])];
        }
        let t1207 = b27_l_p(l, lid0, t1162, t1193);
        let t1208: [uint32x4_t; 2] = [vaddq_u32(x144[0], t1207[0]), vaddq_u32(x144[1], t1207[1])];
        x144 = [vbslq_u32(t1162[0], t1208[0], x144[0]), vbslq_u32(t1162[1], t1208[1], x144[1])];
        let t1209: [uint32x4_t; 2] = [vandq_u32(t1162[0], live[0]), vandq_u32(t1162[1], live[1])];
        x152 = [vaddq_u32(x152[0], vandq_u32(t1209[0], vdupq_n_u32(8))), vaddq_u32(x152[1], vandq_u32(t1209[1], vdupq_n_u32(8)))];
    }
    x96 = x144;
    let t1210 = b20_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    let t1211 = [vdupq_n_u32((((t1210).wrapping_shr((((((4u32).wrapping_mul(x16)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32))); 2];
    x152 = t1211;
    let t1212 = [vdupq_n_u32(1u32); 2];
    let t1213: [uint32x4_t; 2] = [vsubq_u32(x80[0], t1212[0]), vsubq_u32(x80[1], t1212[1])];
    let t1214: [uint32x4_t; 2] = [vmulq_u32(t1147[0], t1213[0]), vmulq_u32(t1147[1], t1213[1])];
    let t1215: [uint32x4_t; 2] = [vaddq_u32(x56[0], t1214[0]), vaddq_u32(x56[1], t1214[1])];
    let t1216 = b26_ll_p(l, lid0, live, t1215, x64);
    let t1217 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t1216, 2u32);
    let t1218: [uint32x4_t; 2] = [t1217[0], t1217[1]];
    let t1219: [uint32x4_t; 4] = [t1218[0], t1218[1], vdupq_n_u32(0), vdupq_n_u32(0)];
    x160 = t1219;
    let t1220: [uint32x4_t; 2] = [x160[0], x160[1]];
    let t1221: [uint32x4_t; 2] = [vaddq_u32(x112[0], t1220[0]), vaddq_u32(x112[1], t1220[1])];
    let t1222: [uint32x4_t; 2] = [vmulq_u32(x96[0], x152[0]), vmulq_u32(x96[1], x152[1])];
    let t1223: [uint32x4_t; 2] = [vaddq_u32(t1221[0], t1222[0]), vaddq_u32(t1221[1], t1222[1])];
    let t1224 = b26_ll_p(l, lid0, live, t1223, x64);
    let t1225: [uint32x4_t; 4] = [t1224[0], t1224[1], vdupq_n_u32(0), vdupq_n_u32(0)];
    x168 = t1225;
    let t1226: [uint32x4_t; 2] = [x168[0], x168[1]];
    let t1227 = b24_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t1226, 8u32);
    let t1228 = b17_l_p(l, lid0, live, x152);
    let t1229: [uint32x4_t; 4] = [vandq_u32(t1227[0], t1228[0]), vandq_u32(t1227[1], t1228[1]), vandq_u32(t1227[2], t1228[2]), vandq_u32(t1227[3], t1228[3])];
    res = t1229;
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
    let t1230 = b20_r0r1s_p(l, lid0, live, arg0, arg1, (20u32).wrapping_add((4u32).wrapping_mul(x16)));
    res = t1230;
    return res;
}

#[inline(always)]
unsafe fn b21_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t1231 = b20_r0r1s_f(l, lid0, arg0, arg1, (20u32).wrapping_add((4u32).wrapping_mul(x16)));
    res = t1231;
    return res;
}

#[inline(always)]
unsafe fn b22_ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x8: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t1232: u32;
    if ((x0) < (x8)) {
        t1232 = x0;
    } else {
        t1232 = x8;
    }
    res = t1232;
    return res;
}

#[inline(always)]
unsafe fn b22_ss_f(l: &Launch, lid0: u32, x0: u32, x8: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t1233: u32;
    if ((x0) < (x8)) {
        t1233 = x0;
    } else {
        t1233 = x8;
    }
    res = t1233;
    return res;
}

#[inline(always)]
unsafe fn b22_ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x8: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1234 = [vdupq_n_u32(x8); 2];
    let t1235: [uint32x4_t; 2] = [vcltq_u32(x0[0], t1234[0]), vcltq_u32(x0[1], t1234[1])];
    let mut t1236: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1237: [uint32x4_t; 2] = [vandq_u32(live[0], t1235[0]), vandq_u32(live[1], t1235[1])];
    let t1238: [uint32x4_t; 2] = [vbicq_u32(live[0], t1235[0]), vbicq_u32(live[1], t1235[1])];
    {
        t1236 = [vbslq_u32(t1237[0], x0[0], t1236[0]), vbslq_u32(t1237[1], x0[1], t1236[1])];
    }
    {
        let t1239 = [vdupq_n_u32(x8); 2];
        t1236 = [vbslq_u32(t1238[0], t1239[0], t1236[0]), vbslq_u32(t1238[1], t1239[1], t1236[1])];
    }
    res = t1236;
    return res;
}

#[inline(always)]
unsafe fn b22_ls_f(l: &Launch, lid0: u32, x0: [uint32x4_t; 2], x8: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1240 = [vdupq_n_u32(x8); 2];
    let t1241: [uint32x4_t; 2] = [vcltq_u32(x0[0], t1240[0]), vcltq_u32(x0[1], t1240[1])];
    let mut t1242: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1243: [uint32x4_t; 2] = [vmvnq_u32(t1241[0]), vmvnq_u32(t1241[1])];
    {
        t1242 = [vbslq_u32(t1241[0], x0[0], t1242[0]), vbslq_u32(t1241[1], x0[1], t1242[1])];
    }
    {
        let t1244 = [vdupq_n_u32(x8); 2];
        t1242 = [vbslq_u32(t1243[0], t1244[0], t1242[0]), vbslq_u32(t1243[1], t1244[1], t1242[1])];
    }
    res = t1242;
    return res;
}

#[inline(always)]
unsafe fn b23_r0sns_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: u32, x16f: u32, x16s: u32, x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1245 = b28_r0an1s_p(l, lid0, live, arg0, ((((x8) as u64)) as usize as *const u8), x16f, x16s, x24);
    x32 = t1245;
    let t1246: [uint32x4_t; 4];
    if ((x24) >= (8u32)) {
        t1246 = x32;
    } else {
        let t1247 = b29_s_p(l, lid0, live, (8u32).wrapping_mul(x24));
        let t1248: u64 = t1247;
        let t1249 = [vdupq_n_u32((t1248) as u32); 2];
        let t1250 = [vdupq_n_u32(((t1248) >> 32) as u32); 2];
        let t1251: [uint32x4_t; 4] = [vandq_u32(x32[0], t1249[0]), vandq_u32(x32[1], t1249[1]), vandq_u32(x32[2], t1250[0]), vandq_u32(x32[3], t1250[1])];
        t1246 = t1251;
    }
    res = t1246;
    return res;
}

#[inline(always)]
unsafe fn b23_r0sns_f(l: &Launch, lid0: u32, arg0: *const u8, x8: u32, x16f: u32, x16s: u32, x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1252 = b28_r0an1s_f(l, lid0, arg0, ((((x8) as u64)) as usize as *const u8), x16f, x16s, x24);
    x32 = t1252;
    let t1253: [uint32x4_t; 4];
    if ((x24) >= (8u32)) {
        t1253 = x32;
    } else {
        let t1254 = b29_s_f(l, lid0, (8u32).wrapping_mul(x24));
        let t1255: u64 = t1254;
        let t1256 = [vdupq_n_u32((t1255) as u32); 2];
        let t1257 = [vdupq_n_u32(((t1255) >> 32) as u32); 2];
        let t1258: [uint32x4_t; 4] = [vandq_u32(x32[0], t1256[0]), vandq_u32(x32[1], t1256[1]), vandq_u32(x32[2], t1257[0]), vandq_u32(x32[3], t1257[1])];
        t1253 = t1258;
    }
    res = t1253;
    return res;
}

#[inline(always)]
unsafe fn b23_r0lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: [uint32x4_t; 2], x16: [uint32x4_t; 2], x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1259: [uint32x4_t; 2] = [vaddq_u32(x8[0], x16[0]), vaddq_u32(x8[1], x16[1])];
    let t1260: [uint32x4_t; 2] = [vcltq_u32(t1259[0], x8[0]), vcltq_u32(t1259[1], x8[1])];
    let t1261: [uint32x4_t; 4] = [t1259[0], t1259[1], vshrq_n_u32::<31>(t1260[0]), vshrq_n_u32::<31>(t1260[1])];
    let t1262 = b28_r0ls_p(l, lid0, live, arg0, t1261, x24);
    x32 = t1262;
    let t1263: [uint32x4_t; 4];
    if ((x24) >= (8u32)) {
        t1263 = x32;
    } else {
        let t1264 = b29_s_p(l, lid0, live, (8u32).wrapping_mul(x24));
        let t1265: u64 = t1264;
        let t1266 = [vdupq_n_u32((t1265) as u32); 2];
        let t1267 = [vdupq_n_u32(((t1265) >> 32) as u32); 2];
        let t1268: [uint32x4_t; 4] = [vandq_u32(x32[0], t1266[0]), vandq_u32(x32[1], t1266[1]), vandq_u32(x32[2], t1267[0]), vandq_u32(x32[3], t1267[1])];
        t1263 = t1268;
    }
    res = t1263;
    return res;
}

#[inline(always)]
unsafe fn b23_r0lls_f(l: &Launch, lid0: u32, arg0: *const u8, x8: [uint32x4_t; 2], x16: [uint32x4_t; 2], x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1269: [uint32x4_t; 2] = [vaddq_u32(x8[0], x16[0]), vaddq_u32(x8[1], x16[1])];
    let t1270: [uint32x4_t; 2] = [vcltq_u32(t1269[0], x8[0]), vcltq_u32(t1269[1], x8[1])];
    let t1271: [uint32x4_t; 4] = [t1269[0], t1269[1], vshrq_n_u32::<31>(t1270[0]), vshrq_n_u32::<31>(t1270[1])];
    let t1272 = b28_r0ls_f(l, lid0, arg0, t1271, x24);
    x32 = t1272;
    let t1273: [uint32x4_t; 4];
    if ((x24) >= (8u32)) {
        t1273 = x32;
    } else {
        let t1274 = b29_s_f(l, lid0, (8u32).wrapping_mul(x24));
        let t1275: u64 = t1274;
        let t1276 = [vdupq_n_u32((t1275) as u32); 2];
        let t1277 = [vdupq_n_u32(((t1275) >> 32) as u32); 2];
        let t1278: [uint32x4_t; 4] = [vandq_u32(x32[0], t1276[0]), vandq_u32(x32[1], t1276[1]), vandq_u32(x32[2], t1277[0]), vandq_u32(x32[3], t1277[1])];
        t1273 = t1278;
    }
    res = t1273;
    return res;
}

#[inline(always)]
unsafe fn b24_r0r1sss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: u32, x32: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t1279 = b30_r0sss_p(l, lid0, live, arg0, x16, x24, x32);
    res = t1279;
    return res;
}

#[inline(always)]
unsafe fn b24_r0r1sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2], x32: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1280 = b30_r0sls_p(l, lid0, live, arg0, x16, x24, x32);
    res = t1280;
    return res;
}

#[inline(always)]
unsafe fn b25_ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x8: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t1281: u32;
    if ((x0) < (x8)) {
        t1281 = x0;
    } else {
        t1281 = x8;
    }
    res = t1281;
    return res;
}

#[inline(always)]
unsafe fn b25_sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x8: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1282 = [vdupq_n_u32(x0); 2];
    let t1283: [uint32x4_t; 2] = [vcltq_u32(t1282[0], x8[0]), vcltq_u32(t1282[1], x8[1])];
    let mut t1284: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1285: [uint32x4_t; 2] = [vandq_u32(live[0], t1283[0]), vandq_u32(live[1], t1283[1])];
    let t1286: [uint32x4_t; 2] = [vbicq_u32(live[0], t1283[0]), vbicq_u32(live[1], t1283[1])];
    {
        let t1287 = [vdupq_n_u32(x0); 2];
        t1284 = [vbslq_u32(t1285[0], t1287[0], t1284[0]), vbslq_u32(t1285[1], t1287[1], t1284[1])];
    }
    {
        t1284 = [vbslq_u32(t1286[0], x8[0], t1284[0]), vbslq_u32(t1286[1], x8[1], t1284[1])];
    }
    res = t1284;
    return res;
}

#[inline(always)]
unsafe fn b26_ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x8: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t1288: u32;
    if ((x0) < (x8)) {
        t1288 = x0;
    } else {
        t1288 = x8;
    }
    res = t1288;
    return res;
}

#[inline(always)]
unsafe fn b26_ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x8: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1289 = [vdupq_n_u32(x8); 2];
    let t1290: [uint32x4_t; 2] = [vcltq_u32(x0[0], t1289[0]), vcltq_u32(x0[1], t1289[1])];
    let mut t1291: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1292: [uint32x4_t; 2] = [vandq_u32(live[0], t1290[0]), vandq_u32(live[1], t1290[1])];
    let t1293: [uint32x4_t; 2] = [vbicq_u32(live[0], t1290[0]), vbicq_u32(live[1], t1290[1])];
    {
        t1291 = [vbslq_u32(t1292[0], x0[0], t1291[0]), vbslq_u32(t1292[1], x0[1], t1291[1])];
    }
    {
        let t1294 = [vdupq_n_u32(x8); 2];
        t1291 = [vbslq_u32(t1293[0], t1294[0], t1291[0]), vbslq_u32(t1293[1], t1294[1], t1291[1])];
    }
    res = t1291;
    return res;
}

#[inline(always)]
unsafe fn b26_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x8: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1295: [uint32x4_t; 2] = [vcltq_u32(x0[0], x8[0]), vcltq_u32(x0[1], x8[1])];
    let mut t1296: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1297: [uint32x4_t; 2] = [vandq_u32(live[0], t1295[0]), vandq_u32(live[1], t1295[1])];
    let t1298: [uint32x4_t; 2] = [vbicq_u32(live[0], t1295[0]), vbicq_u32(live[1], t1295[1])];
    {
        t1296 = [vbslq_u32(t1297[0], x0[0], t1296[0]), vbslq_u32(t1297[1], x0[1], t1296[1])];
    }
    {
        t1296 = [vbslq_u32(t1298[0], x8[0], t1296[0]), vbslq_u32(t1298[1], x8[1], t1296[1])];
    }
    res = t1296;
    return res;
}

#[inline(always)]
unsafe fn b27_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t1299: [uint32x4_t; 2] = [vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[0])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[2]))))), vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[1])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[3])))))];
    res = t1299;
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
    let mut t1300: bool = ((x32) != (0u32));
    if t1300 {
        t1300 = (((x32).wrapping_add((x16).wrapping_mul(8u32))) > (64u32));
    }
    let t1301: u64;
    if t1300 {
        t1301 = ((x40) | (((x24.wrapping_add((1u64).wrapping_mul(8u64) as usize) as *const u64).read_unaligned()).wrapping_shl(((((64u32).wrapping_sub(x32)) as u64)) as u32)));
    } else {
        t1301 = x40;
    }
    res = t1301;
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
        let t1302: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
        let t1303: [uint32x4_t; 4] = [vandq_u32(x8[0], t1302[0]), vandq_u32(x8[1], t1302[1]), vandq_u32(x8[2], t1302[2]), vandq_u32(x8[3], t1302[3])];
        let t1304: u64 = ((arg0) as usize as u64);
        let t1305 = [vdupq_n_u32((t1304) as u32); 2];
        let t1306 = [vdupq_n_u32(((t1304) >> 32) as u32); 2];
        let t1307: [uint32x4_t; 2] = [vaddq_u32(t1305[0], t1303[0]), vaddq_u32(t1305[1], t1303[1])];
        let t1308: [uint32x4_t; 2] = [vcltq_u32(t1307[0], t1305[0]), vcltq_u32(t1307[1], t1305[1])];
        let t1309: [uint32x4_t; 4] = [t1307[0], t1307[1], vsubq_u32(vaddq_u32(t1306[0], t1303[2]), t1308[0]), vsubq_u32(vaddq_u32(t1306[1], t1303[3]), t1308[1])];
        let mut t1310: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1309[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1309[0]) as u64) as usize as *const u64).read_unaligned(); t1310[0] = vsetq_lane_u32::<0>(v as u32, t1310[0]); t1310[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1310[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1309[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1309[0]) as u64) as usize as *const u64).read_unaligned(); t1310[0] = vsetq_lane_u32::<1>(v as u32, t1310[0]); t1310[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1310[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1309[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1309[0]) as u64) as usize as *const u64).read_unaligned(); t1310[0] = vsetq_lane_u32::<2>(v as u32, t1310[0]); t1310[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1310[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1309[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1309[0]) as u64) as usize as *const u64).read_unaligned(); t1310[0] = vsetq_lane_u32::<3>(v as u32, t1310[0]); t1310[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1310[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1309[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1309[1]) as u64) as usize as *const u64).read_unaligned(); t1310[1] = vsetq_lane_u32::<0>(v as u32, t1310[1]); t1310[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1310[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1309[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1309[1]) as u64) as usize as *const u64).read_unaligned(); t1310[1] = vsetq_lane_u32::<1>(v as u32, t1310[1]); t1310[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1310[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1309[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1309[1]) as u64) as usize as *const u64).read_unaligned(); t1310[1] = vsetq_lane_u32::<2>(v as u32, t1310[1]); t1310[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1310[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1309[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1309[1]) as u64) as usize as *const u64).read_unaligned(); t1310[1] = vsetq_lane_u32::<3>(v as u32, t1310[1]); t1310[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1310[3]); }
        res = t1310;
        return res;
    }
    let t1311: u64 = !(7u64);
    let t1312 = [vdupq_n_u32((t1311) as u32); 2];
    let t1313 = [vdupq_n_u32(((t1311) >> 32) as u32); 2];
    let t1314: [uint32x4_t; 4] = [vandq_u32(x8[0], t1312[0]), vandq_u32(x8[1], t1312[1]), vandq_u32(x8[2], t1313[0]), vandq_u32(x8[3], t1313[1])];
    let t1315: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
    let t1316: [uint32x4_t; 4] = [vandq_u32(t1314[0], t1315[0]), vandq_u32(t1314[1], t1315[1]), vandq_u32(t1314[2], t1315[2]), vandq_u32(t1314[3], t1315[3])];
    let t1317: u64 = ((arg0) as usize as u64);
    let t1318 = [vdupq_n_u32((t1317) as u32); 2];
    let t1319 = [vdupq_n_u32(((t1317) >> 32) as u32); 2];
    let t1320: [uint32x4_t; 2] = [vaddq_u32(t1318[0], t1316[0]), vaddq_u32(t1318[1], t1316[1])];
    let t1321: [uint32x4_t; 2] = [vcltq_u32(t1320[0], t1318[0]), vcltq_u32(t1320[1], t1318[1])];
    let t1322: [uint32x4_t; 4] = [t1320[0], t1320[1], vsubq_u32(vaddq_u32(t1319[0], t1316[2]), t1321[0]), vsubq_u32(vaddq_u32(t1319[1], t1316[3]), t1321[1])];
    x24 = t1322;
    let t1323: u64 = 7u64;
    let t1324 = [vdupq_n_u32((t1323) as u32); 2];
    let t1325: [uint32x4_t; 2] = [vandq_u32(x8[0], t1324[0]), vandq_u32(x8[1], t1324[1])];
    let t1326 = [vdupq_n_u32(8u32); 2];
    let t1327: [uint32x4_t; 2] = [vmulq_u32(t1325[0], t1326[0]), vmulq_u32(t1325[1], t1326[1])];
    x32 = t1327;
    let mut t1328: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x24[2]) as u64) << 32 | vgetq_lane_u32::<0>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1328[0] = vsetq_lane_u32::<0>(v as u32, t1328[0]); t1328[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1328[2]); }
    { let v = (((vgetq_lane_u32::<1>(x24[2]) as u64) << 32 | vgetq_lane_u32::<1>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1328[0] = vsetq_lane_u32::<1>(v as u32, t1328[0]); t1328[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1328[2]); }
    { let v = (((vgetq_lane_u32::<2>(x24[2]) as u64) << 32 | vgetq_lane_u32::<2>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1328[0] = vsetq_lane_u32::<2>(v as u32, t1328[0]); t1328[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1328[2]); }
    { let v = (((vgetq_lane_u32::<3>(x24[2]) as u64) << 32 | vgetq_lane_u32::<3>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1328[0] = vsetq_lane_u32::<3>(v as u32, t1328[0]); t1328[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1328[2]); }
    { let v = (((vgetq_lane_u32::<0>(x24[3]) as u64) << 32 | vgetq_lane_u32::<0>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1328[1] = vsetq_lane_u32::<0>(v as u32, t1328[1]); t1328[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1328[3]); }
    { let v = (((vgetq_lane_u32::<1>(x24[3]) as u64) << 32 | vgetq_lane_u32::<1>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1328[1] = vsetq_lane_u32::<1>(v as u32, t1328[1]); t1328[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1328[3]); }
    { let v = (((vgetq_lane_u32::<2>(x24[3]) as u64) << 32 | vgetq_lane_u32::<2>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1328[1] = vsetq_lane_u32::<2>(v as u32, t1328[1]); t1328[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1328[3]); }
    { let v = (((vgetq_lane_u32::<3>(x24[3]) as u64) << 32 | vgetq_lane_u32::<3>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1328[1] = vsetq_lane_u32::<3>(v as u32, t1328[1]); t1328[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1328[3]); }
    let t1329: [uint32x4_t; 2] = [x32[0], x32[1]];
    let t1330: [uint32x4_t; 2] = [vandq_u32(t1329[0], vdupq_n_u32(63)), vandq_u32(t1329[1], vdupq_n_u32(63))];
    let t1331: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t1328[0], vnegq_s32(vreinterpretq_s32_u32(t1330[0]))), vshlq_u32(t1328[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1330[0])))), vorrq_u32(vshlq_u32(t1328[1], vnegq_s32(vreinterpretq_s32_u32(t1330[1]))), vshlq_u32(t1328[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1330[1])))), vshlq_u32(t1328[2], vnegq_s32(vreinterpretq_s32_u32(t1330[0]))), vshlq_u32(t1328[3], vnegq_s32(vreinterpretq_s32_u32(t1330[1])))];
    x40 = t1331;
    let t1332 = [vdupq_n_u32(0u32); 2];
    let t1333: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t1332[0])), vmvnq_u32(vceqq_u32(x32[1], t1332[1]))];
    let t1334 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t1335: [uint32x4_t; 2] = [vaddq_u32(x32[0], t1334[0]), vaddq_u32(x32[1], t1334[1])];
    let t1336 = [vdupq_n_u32(64u32); 2];
    let t1337: [uint32x4_t; 2] = [vcgtq_u32(t1335[0], t1336[0]), vcgtq_u32(t1335[1], t1336[1])];
    let t1338: [uint32x4_t; 2] = [vandq_u32(t1333[0], t1337[0]), vandq_u32(t1333[1], t1337[1])];
    let mut t1339: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1340: [uint32x4_t; 2] = [vandq_u32(live[0], t1338[0]), vandq_u32(live[1], t1338[1])];
    let t1341: [uint32x4_t; 2] = [vbicq_u32(live[0], t1338[0]), vbicq_u32(live[1], t1338[1])];
    if any(&t1340) {
        let t1342: u64 = (1u64).wrapping_mul(8u64);
        let t1343 = [vdupq_n_u32((t1342) as u32); 2];
        let t1344 = [vdupq_n_u32(((t1342) >> 32) as u32); 2];
        let t1345: [uint32x4_t; 2] = [vaddq_u32(x24[0], t1343[0]), vaddq_u32(x24[1], t1343[1])];
        let t1346: [uint32x4_t; 2] = [vcltq_u32(t1345[0], x24[0]), vcltq_u32(t1345[1], x24[1])];
        let t1347: [uint32x4_t; 4] = [t1345[0], t1345[1], vsubq_u32(vaddq_u32(x24[2], t1344[0]), t1346[0]), vsubq_u32(vaddq_u32(x24[3], t1344[1]), t1346[1])];
        let mut t1348: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1347[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1347[0]) as u64) as usize as *const u64).read_unaligned(); t1348[0] = vsetq_lane_u32::<0>(v as u32, t1348[0]); t1348[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1348[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1347[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1347[0]) as u64) as usize as *const u64).read_unaligned(); t1348[0] = vsetq_lane_u32::<1>(v as u32, t1348[0]); t1348[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1348[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1347[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1347[0]) as u64) as usize as *const u64).read_unaligned(); t1348[0] = vsetq_lane_u32::<2>(v as u32, t1348[0]); t1348[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1348[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1347[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1347[0]) as u64) as usize as *const u64).read_unaligned(); t1348[0] = vsetq_lane_u32::<3>(v as u32, t1348[0]); t1348[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1348[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1347[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1347[1]) as u64) as usize as *const u64).read_unaligned(); t1348[1] = vsetq_lane_u32::<0>(v as u32, t1348[1]); t1348[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1348[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1347[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1347[1]) as u64) as usize as *const u64).read_unaligned(); t1348[1] = vsetq_lane_u32::<1>(v as u32, t1348[1]); t1348[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1348[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1347[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1347[1]) as u64) as usize as *const u64).read_unaligned(); t1348[1] = vsetq_lane_u32::<2>(v as u32, t1348[1]); t1348[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1348[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1347[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1347[1]) as u64) as usize as *const u64).read_unaligned(); t1348[1] = vsetq_lane_u32::<3>(v as u32, t1348[1]); t1348[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1348[3]); }
        let t1349: [uint32x4_t; 2] = [vsubq_u32(t1336[0], x32[0]), vsubq_u32(t1336[1], x32[1])];
        let t1350: [uint32x4_t; 2] = [t1349[0], t1349[1]];
        let t1351: [uint32x4_t; 2] = [vandq_u32(t1350[0], vdupq_n_u32(63)), vandq_u32(t1350[1], vdupq_n_u32(63))];
        let t1352: [uint32x4_t; 4] = [vshlq_u32(t1348[0], vreinterpretq_s32_u32(t1351[0])), vshlq_u32(t1348[1], vreinterpretq_s32_u32(t1351[1])), vorrq_u32(vshlq_u32(t1348[2], vreinterpretq_s32_u32(t1351[0])), vshlq_u32(t1348[0], vsubq_s32(vreinterpretq_s32_u32(t1351[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1348[3], vreinterpretq_s32_u32(t1351[1])), vshlq_u32(t1348[1], vsubq_s32(vreinterpretq_s32_u32(t1351[1]), vdupq_n_s32(32))))];
        let t1353: [uint32x4_t; 4] = [vorrq_u32(x40[0], t1352[0]), vorrq_u32(x40[1], t1352[1]), vorrq_u32(x40[2], t1352[2]), vorrq_u32(x40[3], t1352[3])];
        t1339 = [vbslq_u32(t1340[0], t1353[0], t1339[0]), vbslq_u32(t1340[1], t1353[1], t1339[1]), vbslq_u32(t1340[0], t1353[2], t1339[2]), vbslq_u32(t1340[1], t1353[3], t1339[3])];
    }
    {
        t1339 = [vbslq_u32(t1341[0], x40[0], t1339[0]), vbslq_u32(t1341[1], x40[1], t1339[1]), vbslq_u32(t1341[0], x40[2], t1339[2]), vbslq_u32(t1341[1], x40[3], t1339[3])];
    }
    res = t1339;
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
        let t1354: u64 = ((arg0) as usize as u64);
        let t1355 = [vdupq_n_u32((t1354) as u32); 2];
        let t1356 = [vdupq_n_u32(((t1354) >> 32) as u32); 2];
        let t1357: [uint32x4_t; 2] = [vaddq_u32(t1355[0], x8[0]), vaddq_u32(t1355[1], x8[1])];
        let t1358: [uint32x4_t; 2] = [vcltq_u32(t1357[0], t1355[0]), vcltq_u32(t1357[1], t1355[1])];
        let t1359: [uint32x4_t; 4] = [t1357[0], t1357[1], vsubq_u32(vaddq_u32(t1356[0], x8[2]), t1358[0]), vsubq_u32(vaddq_u32(t1356[1], x8[3]), t1358[1])];
        let mut t1360: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1359[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1359[0]) as u64) as usize as *const u64).read_unaligned(); t1360[0] = vsetq_lane_u32::<0>(v as u32, t1360[0]); t1360[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1360[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1359[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1359[0]) as u64) as usize as *const u64).read_unaligned(); t1360[0] = vsetq_lane_u32::<1>(v as u32, t1360[0]); t1360[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1360[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1359[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1359[0]) as u64) as usize as *const u64).read_unaligned(); t1360[0] = vsetq_lane_u32::<2>(v as u32, t1360[0]); t1360[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1360[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1359[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1359[0]) as u64) as usize as *const u64).read_unaligned(); t1360[0] = vsetq_lane_u32::<3>(v as u32, t1360[0]); t1360[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1360[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1359[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1359[1]) as u64) as usize as *const u64).read_unaligned(); t1360[1] = vsetq_lane_u32::<0>(v as u32, t1360[1]); t1360[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1360[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1359[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1359[1]) as u64) as usize as *const u64).read_unaligned(); t1360[1] = vsetq_lane_u32::<1>(v as u32, t1360[1]); t1360[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1360[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1359[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1359[1]) as u64) as usize as *const u64).read_unaligned(); t1360[1] = vsetq_lane_u32::<2>(v as u32, t1360[1]); t1360[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1360[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1359[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1359[1]) as u64) as usize as *const u64).read_unaligned(); t1360[1] = vsetq_lane_u32::<3>(v as u32, t1360[1]); t1360[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1360[3]); }
        res = t1360;
        return res;
    }
    let t1361: u64 = !(7u64);
    let t1362 = [vdupq_n_u32((t1361) as u32); 2];
    let t1363 = [vdupq_n_u32(((t1361) >> 32) as u32); 2];
    let t1364: [uint32x4_t; 4] = [vandq_u32(x8[0], t1362[0]), vandq_u32(x8[1], t1362[1]), vandq_u32(x8[2], t1363[0]), vandq_u32(x8[3], t1363[1])];
    let t1365: u64 = ((arg0) as usize as u64);
    let t1366 = [vdupq_n_u32((t1365) as u32); 2];
    let t1367 = [vdupq_n_u32(((t1365) >> 32) as u32); 2];
    let t1368: [uint32x4_t; 2] = [vaddq_u32(t1366[0], t1364[0]), vaddq_u32(t1366[1], t1364[1])];
    let t1369: [uint32x4_t; 2] = [vcltq_u32(t1368[0], t1366[0]), vcltq_u32(t1368[1], t1366[1])];
    let t1370: [uint32x4_t; 4] = [t1368[0], t1368[1], vsubq_u32(vaddq_u32(t1367[0], t1364[2]), t1369[0]), vsubq_u32(vaddq_u32(t1367[1], t1364[3]), t1369[1])];
    x24 = t1370;
    let t1371: u64 = 7u64;
    let t1372 = [vdupq_n_u32((t1371) as u32); 2];
    let t1373: [uint32x4_t; 2] = [vandq_u32(x8[0], t1372[0]), vandq_u32(x8[1], t1372[1])];
    let t1374 = [vdupq_n_u32(8u32); 2];
    let t1375: [uint32x4_t; 2] = [vmulq_u32(t1373[0], t1374[0]), vmulq_u32(t1373[1], t1374[1])];
    x32 = t1375;
    let mut t1376: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x24[2]) as u64) << 32 | vgetq_lane_u32::<0>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1376[0] = vsetq_lane_u32::<0>(v as u32, t1376[0]); t1376[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1376[2]); }
    { let v = (((vgetq_lane_u32::<1>(x24[2]) as u64) << 32 | vgetq_lane_u32::<1>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1376[0] = vsetq_lane_u32::<1>(v as u32, t1376[0]); t1376[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1376[2]); }
    { let v = (((vgetq_lane_u32::<2>(x24[2]) as u64) << 32 | vgetq_lane_u32::<2>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1376[0] = vsetq_lane_u32::<2>(v as u32, t1376[0]); t1376[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1376[2]); }
    { let v = (((vgetq_lane_u32::<3>(x24[2]) as u64) << 32 | vgetq_lane_u32::<3>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1376[0] = vsetq_lane_u32::<3>(v as u32, t1376[0]); t1376[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1376[2]); }
    { let v = (((vgetq_lane_u32::<0>(x24[3]) as u64) << 32 | vgetq_lane_u32::<0>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1376[1] = vsetq_lane_u32::<0>(v as u32, t1376[1]); t1376[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1376[3]); }
    { let v = (((vgetq_lane_u32::<1>(x24[3]) as u64) << 32 | vgetq_lane_u32::<1>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1376[1] = vsetq_lane_u32::<1>(v as u32, t1376[1]); t1376[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1376[3]); }
    { let v = (((vgetq_lane_u32::<2>(x24[3]) as u64) << 32 | vgetq_lane_u32::<2>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1376[1] = vsetq_lane_u32::<2>(v as u32, t1376[1]); t1376[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1376[3]); }
    { let v = (((vgetq_lane_u32::<3>(x24[3]) as u64) << 32 | vgetq_lane_u32::<3>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1376[1] = vsetq_lane_u32::<3>(v as u32, t1376[1]); t1376[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1376[3]); }
    let t1377: [uint32x4_t; 2] = [x32[0], x32[1]];
    let t1378: [uint32x4_t; 2] = [vandq_u32(t1377[0], vdupq_n_u32(63)), vandq_u32(t1377[1], vdupq_n_u32(63))];
    let t1379: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t1376[0], vnegq_s32(vreinterpretq_s32_u32(t1378[0]))), vshlq_u32(t1376[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1378[0])))), vorrq_u32(vshlq_u32(t1376[1], vnegq_s32(vreinterpretq_s32_u32(t1378[1]))), vshlq_u32(t1376[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1378[1])))), vshlq_u32(t1376[2], vnegq_s32(vreinterpretq_s32_u32(t1378[0]))), vshlq_u32(t1376[3], vnegq_s32(vreinterpretq_s32_u32(t1378[1])))];
    x40 = t1379;
    let t1380 = [vdupq_n_u32(0u32); 2];
    let t1381: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t1380[0])), vmvnq_u32(vceqq_u32(x32[1], t1380[1]))];
    let t1382 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t1383: [uint32x4_t; 2] = [vaddq_u32(x32[0], t1382[0]), vaddq_u32(x32[1], t1382[1])];
    let t1384 = [vdupq_n_u32(64u32); 2];
    let t1385: [uint32x4_t; 2] = [vcgtq_u32(t1383[0], t1384[0]), vcgtq_u32(t1383[1], t1384[1])];
    let t1386: [uint32x4_t; 2] = [vandq_u32(t1381[0], t1385[0]), vandq_u32(t1381[1], t1385[1])];
    let mut t1387: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1388: [uint32x4_t; 2] = [vmvnq_u32(t1386[0]), vmvnq_u32(t1386[1])];
    if any(&t1386) {
        let t1389: u64 = (1u64).wrapping_mul(8u64);
        let t1390 = [vdupq_n_u32((t1389) as u32); 2];
        let t1391 = [vdupq_n_u32(((t1389) >> 32) as u32); 2];
        let t1392: [uint32x4_t; 2] = [vaddq_u32(x24[0], t1390[0]), vaddq_u32(x24[1], t1390[1])];
        let t1393: [uint32x4_t; 2] = [vcltq_u32(t1392[0], x24[0]), vcltq_u32(t1392[1], x24[1])];
        let t1394: [uint32x4_t; 4] = [t1392[0], t1392[1], vsubq_u32(vaddq_u32(x24[2], t1391[0]), t1393[0]), vsubq_u32(vaddq_u32(x24[3], t1391[1]), t1393[1])];
        let mut t1395: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1394[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1394[0]) as u64) as usize as *const u64).read_unaligned(); t1395[0] = vsetq_lane_u32::<0>(v as u32, t1395[0]); t1395[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1395[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1394[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1394[0]) as u64) as usize as *const u64).read_unaligned(); t1395[0] = vsetq_lane_u32::<1>(v as u32, t1395[0]); t1395[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1395[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1394[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1394[0]) as u64) as usize as *const u64).read_unaligned(); t1395[0] = vsetq_lane_u32::<2>(v as u32, t1395[0]); t1395[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1395[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1394[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1394[0]) as u64) as usize as *const u64).read_unaligned(); t1395[0] = vsetq_lane_u32::<3>(v as u32, t1395[0]); t1395[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1395[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1394[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1394[1]) as u64) as usize as *const u64).read_unaligned(); t1395[1] = vsetq_lane_u32::<0>(v as u32, t1395[1]); t1395[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1395[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1394[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1394[1]) as u64) as usize as *const u64).read_unaligned(); t1395[1] = vsetq_lane_u32::<1>(v as u32, t1395[1]); t1395[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1395[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1394[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1394[1]) as u64) as usize as *const u64).read_unaligned(); t1395[1] = vsetq_lane_u32::<2>(v as u32, t1395[1]); t1395[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1395[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1394[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1394[1]) as u64) as usize as *const u64).read_unaligned(); t1395[1] = vsetq_lane_u32::<3>(v as u32, t1395[1]); t1395[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1395[3]); }
        let t1396: [uint32x4_t; 2] = [vsubq_u32(t1384[0], x32[0]), vsubq_u32(t1384[1], x32[1])];
        let t1397: [uint32x4_t; 2] = [t1396[0], t1396[1]];
        let t1398: [uint32x4_t; 2] = [vandq_u32(t1397[0], vdupq_n_u32(63)), vandq_u32(t1397[1], vdupq_n_u32(63))];
        let t1399: [uint32x4_t; 4] = [vshlq_u32(t1395[0], vreinterpretq_s32_u32(t1398[0])), vshlq_u32(t1395[1], vreinterpretq_s32_u32(t1398[1])), vorrq_u32(vshlq_u32(t1395[2], vreinterpretq_s32_u32(t1398[0])), vshlq_u32(t1395[0], vsubq_s32(vreinterpretq_s32_u32(t1398[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1395[3], vreinterpretq_s32_u32(t1398[1])), vshlq_u32(t1395[1], vsubq_s32(vreinterpretq_s32_u32(t1398[1]), vdupq_n_s32(32))))];
        let t1400: [uint32x4_t; 4] = [vorrq_u32(x40[0], t1399[0]), vorrq_u32(x40[1], t1399[1]), vorrq_u32(x40[2], t1399[2]), vorrq_u32(x40[3], t1399[3])];
        t1387 = [vbslq_u32(t1386[0], t1400[0], t1387[0]), vbslq_u32(t1386[1], t1400[1], t1387[1]), vbslq_u32(t1386[0], t1400[2], t1387[2]), vbslq_u32(t1386[1], t1400[3], t1387[3])];
    }
    {
        t1387 = [vbslq_u32(t1388[0], x40[0], t1387[0]), vbslq_u32(t1388[1], x40[1], t1387[1]), vbslq_u32(t1388[0], x40[2], t1387[2]), vbslq_u32(t1388[1], x40[3], t1387[3])];
    }
    res = t1387;
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
        let t1401 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], x8i[0]), vandq_u32(live[1], x8i[1]))));
        let t1402: [uint32x4_t; 2] = [vbslq_u32(live[0], x8i[0], t1401), vbslq_u32(live[1], x8i[1], t1401)];
        let mut t1403: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1402[0]) as usize * 1) as *const u64).read_unaligned(); t1403[0] = vsetq_lane_u32::<0>(v as u32, t1403[0]); t1403[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1403[2]); }
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1402[0]) as usize * 1) as *const u64).read_unaligned(); t1403[0] = vsetq_lane_u32::<1>(v as u32, t1403[0]); t1403[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1403[2]); }
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1402[0]) as usize * 1) as *const u64).read_unaligned(); t1403[0] = vsetq_lane_u32::<2>(v as u32, t1403[0]); t1403[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1403[2]); }
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1402[0]) as usize * 1) as *const u64).read_unaligned(); t1403[0] = vsetq_lane_u32::<3>(v as u32, t1403[0]); t1403[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1403[2]); }
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1402[1]) as usize * 1) as *const u64).read_unaligned(); t1403[1] = vsetq_lane_u32::<0>(v as u32, t1403[1]); t1403[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1403[3]); }
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1402[1]) as usize * 1) as *const u64).read_unaligned(); t1403[1] = vsetq_lane_u32::<1>(v as u32, t1403[1]); t1403[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1403[3]); }
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1402[1]) as usize * 1) as *const u64).read_unaligned(); t1403[1] = vsetq_lane_u32::<2>(v as u32, t1403[1]); t1403[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1403[3]); }
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1402[1]) as usize * 1) as *const u64).read_unaligned(); t1403[1] = vsetq_lane_u32::<3>(v as u32, t1403[1]); t1403[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1403[3]); }
        res = t1403;
        return res;
    }
    let t1404: [uint32x4_t; 2] = [x8i[0], x8i[1]];
    let t1405: u64 = ((x8b) as usize as u64);
    let t1406 = [vdupq_n_u32((t1405) as u32); 2];
    let t1407 = [vdupq_n_u32(((t1405) >> 32) as u32); 2];
    let t1408: [uint32x4_t; 2] = [vaddq_u32(t1406[0], t1404[0]), vaddq_u32(t1406[1], t1404[1])];
    let t1409: [uint32x4_t; 2] = [vcltq_u32(t1408[0], t1406[0]), vcltq_u32(t1408[1], t1406[1])];
    let t1410: [uint32x4_t; 4] = [t1408[0], t1408[1], vsubq_u32(vaddq_u32(t1407[0], vdupq_n_u32(0)), t1409[0]), vsubq_u32(vaddq_u32(t1407[1], vdupq_n_u32(0)), t1409[1])];
    let t1411: u64 = !(7u64);
    let t1412 = [vdupq_n_u32((t1411) as u32); 2];
    let t1413 = [vdupq_n_u32(((t1411) >> 32) as u32); 2];
    let t1414: [uint32x4_t; 4] = [vandq_u32(t1410[0], t1412[0]), vandq_u32(t1410[1], t1412[1]), vandq_u32(t1410[2], t1413[0]), vandq_u32(t1410[3], t1413[1])];
    let t1415: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
    let t1416: [uint32x4_t; 4] = [vandq_u32(t1414[0], t1415[0]), vandq_u32(t1414[1], t1415[1]), vandq_u32(t1414[2], t1415[2]), vandq_u32(t1414[3], t1415[3])];
    let t1417: u64 = ((arg0) as usize as u64);
    let t1418 = [vdupq_n_u32((t1417) as u32); 2];
    let t1419 = [vdupq_n_u32(((t1417) >> 32) as u32); 2];
    let t1420: [uint32x4_t; 2] = [vaddq_u32(t1418[0], t1416[0]), vaddq_u32(t1418[1], t1416[1])];
    let t1421: [uint32x4_t; 2] = [vcltq_u32(t1420[0], t1418[0]), vcltq_u32(t1420[1], t1418[1])];
    let t1422: [uint32x4_t; 4] = [t1420[0], t1420[1], vsubq_u32(vaddq_u32(t1419[0], t1416[2]), t1421[0]), vsubq_u32(vaddq_u32(t1419[1], t1416[3]), t1421[1])];
    x24 = t1422;
    let t1423: [uint32x4_t; 2] = [x8i[0], x8i[1]];
    let t1424: u64 = ((x8b) as usize as u64);
    let t1425 = [vdupq_n_u32((t1424) as u32); 2];
    let t1426 = [vdupq_n_u32(((t1424) >> 32) as u32); 2];
    let t1427: [uint32x4_t; 2] = [vaddq_u32(t1425[0], t1423[0]), vaddq_u32(t1425[1], t1423[1])];
    let t1428: [uint32x4_t; 2] = [vcltq_u32(t1427[0], t1425[0]), vcltq_u32(t1427[1], t1425[1])];
    let t1429: [uint32x4_t; 4] = [t1427[0], t1427[1], vsubq_u32(vaddq_u32(t1426[0], vdupq_n_u32(0)), t1428[0]), vsubq_u32(vaddq_u32(t1426[1], vdupq_n_u32(0)), t1428[1])];
    let t1430: u64 = 7u64;
    let t1431 = [vdupq_n_u32((t1430) as u32); 2];
    let t1432: [uint32x4_t; 2] = [vandq_u32(t1429[0], t1431[0]), vandq_u32(t1429[1], t1431[1])];
    let t1433 = [vdupq_n_u32(8u32); 2];
    let t1434: [uint32x4_t; 2] = [vmulq_u32(t1432[0], t1433[0]), vmulq_u32(t1432[1], t1433[1])];
    x32 = t1434;
    let mut t1435: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x24[2]) as u64) << 32 | vgetq_lane_u32::<0>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1435[0] = vsetq_lane_u32::<0>(v as u32, t1435[0]); t1435[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1435[2]); }
    { let v = (((vgetq_lane_u32::<1>(x24[2]) as u64) << 32 | vgetq_lane_u32::<1>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1435[0] = vsetq_lane_u32::<1>(v as u32, t1435[0]); t1435[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1435[2]); }
    { let v = (((vgetq_lane_u32::<2>(x24[2]) as u64) << 32 | vgetq_lane_u32::<2>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1435[0] = vsetq_lane_u32::<2>(v as u32, t1435[0]); t1435[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1435[2]); }
    { let v = (((vgetq_lane_u32::<3>(x24[2]) as u64) << 32 | vgetq_lane_u32::<3>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1435[0] = vsetq_lane_u32::<3>(v as u32, t1435[0]); t1435[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1435[2]); }
    { let v = (((vgetq_lane_u32::<0>(x24[3]) as u64) << 32 | vgetq_lane_u32::<0>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1435[1] = vsetq_lane_u32::<0>(v as u32, t1435[1]); t1435[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1435[3]); }
    { let v = (((vgetq_lane_u32::<1>(x24[3]) as u64) << 32 | vgetq_lane_u32::<1>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1435[1] = vsetq_lane_u32::<1>(v as u32, t1435[1]); t1435[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1435[3]); }
    { let v = (((vgetq_lane_u32::<2>(x24[3]) as u64) << 32 | vgetq_lane_u32::<2>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1435[1] = vsetq_lane_u32::<2>(v as u32, t1435[1]); t1435[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1435[3]); }
    { let v = (((vgetq_lane_u32::<3>(x24[3]) as u64) << 32 | vgetq_lane_u32::<3>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1435[1] = vsetq_lane_u32::<3>(v as u32, t1435[1]); t1435[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1435[3]); }
    let t1436: [uint32x4_t; 2] = [x32[0], x32[1]];
    let t1437: [uint32x4_t; 2] = [vandq_u32(t1436[0], vdupq_n_u32(63)), vandq_u32(t1436[1], vdupq_n_u32(63))];
    let t1438: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t1435[0], vnegq_s32(vreinterpretq_s32_u32(t1437[0]))), vshlq_u32(t1435[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1437[0])))), vorrq_u32(vshlq_u32(t1435[1], vnegq_s32(vreinterpretq_s32_u32(t1437[1]))), vshlq_u32(t1435[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1437[1])))), vshlq_u32(t1435[2], vnegq_s32(vreinterpretq_s32_u32(t1437[0]))), vshlq_u32(t1435[3], vnegq_s32(vreinterpretq_s32_u32(t1437[1])))];
    x40 = t1438;
    let t1439 = [vdupq_n_u32(0u32); 2];
    let t1440: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t1439[0])), vmvnq_u32(vceqq_u32(x32[1], t1439[1]))];
    let t1441 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t1442: [uint32x4_t; 2] = [vaddq_u32(x32[0], t1441[0]), vaddq_u32(x32[1], t1441[1])];
    let t1443 = [vdupq_n_u32(64u32); 2];
    let t1444: [uint32x4_t; 2] = [vcgtq_u32(t1442[0], t1443[0]), vcgtq_u32(t1442[1], t1443[1])];
    let t1445: [uint32x4_t; 2] = [vandq_u32(t1440[0], t1444[0]), vandq_u32(t1440[1], t1444[1])];
    let mut t1446: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1447: [uint32x4_t; 2] = [vandq_u32(live[0], t1445[0]), vandq_u32(live[1], t1445[1])];
    let t1448: [uint32x4_t; 2] = [vbicq_u32(live[0], t1445[0]), vbicq_u32(live[1], t1445[1])];
    if any(&t1447) {
        let t1449: u64 = (1u64).wrapping_mul(8u64);
        let t1450 = [vdupq_n_u32((t1449) as u32); 2];
        let t1451 = [vdupq_n_u32(((t1449) >> 32) as u32); 2];
        let t1452: [uint32x4_t; 2] = [vaddq_u32(x24[0], t1450[0]), vaddq_u32(x24[1], t1450[1])];
        let t1453: [uint32x4_t; 2] = [vcltq_u32(t1452[0], x24[0]), vcltq_u32(t1452[1], x24[1])];
        let t1454: [uint32x4_t; 4] = [t1452[0], t1452[1], vsubq_u32(vaddq_u32(x24[2], t1451[0]), t1453[0]), vsubq_u32(vaddq_u32(x24[3], t1451[1]), t1453[1])];
        let mut t1455: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1454[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1454[0]) as u64) as usize as *const u64).read_unaligned(); t1455[0] = vsetq_lane_u32::<0>(v as u32, t1455[0]); t1455[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1455[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1454[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1454[0]) as u64) as usize as *const u64).read_unaligned(); t1455[0] = vsetq_lane_u32::<1>(v as u32, t1455[0]); t1455[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1455[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1454[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1454[0]) as u64) as usize as *const u64).read_unaligned(); t1455[0] = vsetq_lane_u32::<2>(v as u32, t1455[0]); t1455[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1455[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1454[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1454[0]) as u64) as usize as *const u64).read_unaligned(); t1455[0] = vsetq_lane_u32::<3>(v as u32, t1455[0]); t1455[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1455[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1454[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1454[1]) as u64) as usize as *const u64).read_unaligned(); t1455[1] = vsetq_lane_u32::<0>(v as u32, t1455[1]); t1455[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1455[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1454[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1454[1]) as u64) as usize as *const u64).read_unaligned(); t1455[1] = vsetq_lane_u32::<1>(v as u32, t1455[1]); t1455[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1455[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1454[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1454[1]) as u64) as usize as *const u64).read_unaligned(); t1455[1] = vsetq_lane_u32::<2>(v as u32, t1455[1]); t1455[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1455[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1454[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1454[1]) as u64) as usize as *const u64).read_unaligned(); t1455[1] = vsetq_lane_u32::<3>(v as u32, t1455[1]); t1455[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1455[3]); }
        let t1456: [uint32x4_t; 2] = [vsubq_u32(t1443[0], x32[0]), vsubq_u32(t1443[1], x32[1])];
        let t1457: [uint32x4_t; 2] = [t1456[0], t1456[1]];
        let t1458: [uint32x4_t; 2] = [vandq_u32(t1457[0], vdupq_n_u32(63)), vandq_u32(t1457[1], vdupq_n_u32(63))];
        let t1459: [uint32x4_t; 4] = [vshlq_u32(t1455[0], vreinterpretq_s32_u32(t1458[0])), vshlq_u32(t1455[1], vreinterpretq_s32_u32(t1458[1])), vorrq_u32(vshlq_u32(t1455[2], vreinterpretq_s32_u32(t1458[0])), vshlq_u32(t1455[0], vsubq_s32(vreinterpretq_s32_u32(t1458[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1455[3], vreinterpretq_s32_u32(t1458[1])), vshlq_u32(t1455[1], vsubq_s32(vreinterpretq_s32_u32(t1458[1]), vdupq_n_s32(32))))];
        let t1460: [uint32x4_t; 4] = [vorrq_u32(x40[0], t1459[0]), vorrq_u32(x40[1], t1459[1]), vorrq_u32(x40[2], t1459[2]), vorrq_u32(x40[3], t1459[3])];
        t1446 = [vbslq_u32(t1447[0], t1460[0], t1446[0]), vbslq_u32(t1447[1], t1460[1], t1446[1]), vbslq_u32(t1447[0], t1460[2], t1446[2]), vbslq_u32(t1447[1], t1460[3], t1446[3])];
    }
    {
        t1446 = [vbslq_u32(t1448[0], x40[0], t1446[0]), vbslq_u32(t1448[1], x40[1], t1446[1]), vbslq_u32(t1448[0], x40[2], t1446[2]), vbslq_u32(t1448[1], x40[3], t1446[3])];
    }
    res = t1446;
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
        let t1461: [uint32x4_t; 4];
        if (x8is) == 8u32 && ((x8if) as u64) + 7 * ((x8is) as u64) <= u32::MAX as u64 && all(&live) {
            let t1462 = ((arg0).wrapping_add((x8b) as usize) as *const u8).add(((x8if) as usize) * 1);
            let t14630 = vld2q_u32((t1462 as *const u32).add(0));
            let t14631 = vld2q_u32((t1462 as *const u32).add(8));
            let t1463: [uint32x4_t; 4] = [t14630.0, t14631.0, t14630.1, t14631.1];
            t1461 = t1463;
        } else {
            let t1464: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8is)), vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8is))];
            let t1465 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t1464[0]), vandq_u32(live[1], t1464[1]))));
            let t1466: [uint32x4_t; 2] = [vbslq_u32(live[0], t1464[0], t1465), vbslq_u32(live[1], t1464[1], t1465)];
            let mut t1467: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1466[0]) as usize * 1) as *const u64).read_unaligned(); t1467[0] = vsetq_lane_u32::<0>(v as u32, t1467[0]); t1467[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1467[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1466[0]) as usize * 1) as *const u64).read_unaligned(); t1467[0] = vsetq_lane_u32::<1>(v as u32, t1467[0]); t1467[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1467[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1466[0]) as usize * 1) as *const u64).read_unaligned(); t1467[0] = vsetq_lane_u32::<2>(v as u32, t1467[0]); t1467[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1467[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1466[0]) as usize * 1) as *const u64).read_unaligned(); t1467[0] = vsetq_lane_u32::<3>(v as u32, t1467[0]); t1467[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1467[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1466[1]) as usize * 1) as *const u64).read_unaligned(); t1467[1] = vsetq_lane_u32::<0>(v as u32, t1467[1]); t1467[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1467[3]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1466[1]) as usize * 1) as *const u64).read_unaligned(); t1467[1] = vsetq_lane_u32::<1>(v as u32, t1467[1]); t1467[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1467[3]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1466[1]) as usize * 1) as *const u64).read_unaligned(); t1467[1] = vsetq_lane_u32::<2>(v as u32, t1467[1]); t1467[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1467[3]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1466[1]) as usize * 1) as *const u64).read_unaligned(); t1467[1] = vsetq_lane_u32::<3>(v as u32, t1467[1]); t1467[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1467[3]); }
            t1461 = t1467;
        }
        res = t1461;
        return res;
    }
    let t1468: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8is)), vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8is))];
    let t1469: [uint32x4_t; 2] = [t1468[0], t1468[1]];
    let t1470: u64 = ((x8b) as usize as u64);
    let t1471 = [vdupq_n_u32((t1470) as u32); 2];
    let t1472 = [vdupq_n_u32(((t1470) >> 32) as u32); 2];
    let t1473: [uint32x4_t; 2] = [vaddq_u32(t1471[0], t1469[0]), vaddq_u32(t1471[1], t1469[1])];
    let t1474: [uint32x4_t; 2] = [vcltq_u32(t1473[0], t1471[0]), vcltq_u32(t1473[1], t1471[1])];
    let t1475: [uint32x4_t; 4] = [t1473[0], t1473[1], vsubq_u32(vaddq_u32(t1472[0], vdupq_n_u32(0)), t1474[0]), vsubq_u32(vaddq_u32(t1472[1], vdupq_n_u32(0)), t1474[1])];
    let t1476: u64 = !(7u64);
    let t1477 = [vdupq_n_u32((t1476) as u32); 2];
    let t1478 = [vdupq_n_u32(((t1476) >> 32) as u32); 2];
    let t1479: [uint32x4_t; 4] = [vandq_u32(t1475[0], t1477[0]), vandq_u32(t1475[1], t1477[1]), vandq_u32(t1475[2], t1478[0]), vandq_u32(t1475[3], t1478[1])];
    let t1480: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
    let t1481: [uint32x4_t; 4] = [vandq_u32(t1479[0], t1480[0]), vandq_u32(t1479[1], t1480[1]), vandq_u32(t1479[2], t1480[2]), vandq_u32(t1479[3], t1480[3])];
    let t1482: u64 = ((arg0) as usize as u64);
    let t1483 = [vdupq_n_u32((t1482) as u32); 2];
    let t1484 = [vdupq_n_u32(((t1482) >> 32) as u32); 2];
    let t1485: [uint32x4_t; 2] = [vaddq_u32(t1483[0], t1481[0]), vaddq_u32(t1483[1], t1481[1])];
    let t1486: [uint32x4_t; 2] = [vcltq_u32(t1485[0], t1483[0]), vcltq_u32(t1485[1], t1483[1])];
    let t1487: [uint32x4_t; 4] = [t1485[0], t1485[1], vsubq_u32(vaddq_u32(t1484[0], t1481[2]), t1486[0]), vsubq_u32(vaddq_u32(t1484[1], t1481[3]), t1486[1])];
    x24 = t1487;
    let t1488: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8is)), vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8is))];
    let t1489: [uint32x4_t; 2] = [t1488[0], t1488[1]];
    let t1490: u64 = ((x8b) as usize as u64);
    let t1491 = [vdupq_n_u32((t1490) as u32); 2];
    let t1492 = [vdupq_n_u32(((t1490) >> 32) as u32); 2];
    let t1493: [uint32x4_t; 2] = [vaddq_u32(t1491[0], t1489[0]), vaddq_u32(t1491[1], t1489[1])];
    let t1494: [uint32x4_t; 2] = [vcltq_u32(t1493[0], t1491[0]), vcltq_u32(t1493[1], t1491[1])];
    let t1495: [uint32x4_t; 4] = [t1493[0], t1493[1], vsubq_u32(vaddq_u32(t1492[0], vdupq_n_u32(0)), t1494[0]), vsubq_u32(vaddq_u32(t1492[1], vdupq_n_u32(0)), t1494[1])];
    let t1496: u64 = 7u64;
    let t1497 = [vdupq_n_u32((t1496) as u32); 2];
    let t1498: [uint32x4_t; 2] = [vandq_u32(t1495[0], t1497[0]), vandq_u32(t1495[1], t1497[1])];
    let t1499 = [vdupq_n_u32(8u32); 2];
    let t1500: [uint32x4_t; 2] = [vmulq_u32(t1498[0], t1499[0]), vmulq_u32(t1498[1], t1499[1])];
    x32 = t1500;
    let mut t1501: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x24[2]) as u64) << 32 | vgetq_lane_u32::<0>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1501[0] = vsetq_lane_u32::<0>(v as u32, t1501[0]); t1501[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1501[2]); }
    { let v = (((vgetq_lane_u32::<1>(x24[2]) as u64) << 32 | vgetq_lane_u32::<1>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1501[0] = vsetq_lane_u32::<1>(v as u32, t1501[0]); t1501[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1501[2]); }
    { let v = (((vgetq_lane_u32::<2>(x24[2]) as u64) << 32 | vgetq_lane_u32::<2>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1501[0] = vsetq_lane_u32::<2>(v as u32, t1501[0]); t1501[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1501[2]); }
    { let v = (((vgetq_lane_u32::<3>(x24[2]) as u64) << 32 | vgetq_lane_u32::<3>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1501[0] = vsetq_lane_u32::<3>(v as u32, t1501[0]); t1501[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1501[2]); }
    { let v = (((vgetq_lane_u32::<0>(x24[3]) as u64) << 32 | vgetq_lane_u32::<0>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1501[1] = vsetq_lane_u32::<0>(v as u32, t1501[1]); t1501[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1501[3]); }
    { let v = (((vgetq_lane_u32::<1>(x24[3]) as u64) << 32 | vgetq_lane_u32::<1>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1501[1] = vsetq_lane_u32::<1>(v as u32, t1501[1]); t1501[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1501[3]); }
    { let v = (((vgetq_lane_u32::<2>(x24[3]) as u64) << 32 | vgetq_lane_u32::<2>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1501[1] = vsetq_lane_u32::<2>(v as u32, t1501[1]); t1501[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1501[3]); }
    { let v = (((vgetq_lane_u32::<3>(x24[3]) as u64) << 32 | vgetq_lane_u32::<3>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1501[1] = vsetq_lane_u32::<3>(v as u32, t1501[1]); t1501[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1501[3]); }
    let t1502: [uint32x4_t; 2] = [x32[0], x32[1]];
    let t1503: [uint32x4_t; 2] = [vandq_u32(t1502[0], vdupq_n_u32(63)), vandq_u32(t1502[1], vdupq_n_u32(63))];
    let t1504: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t1501[0], vnegq_s32(vreinterpretq_s32_u32(t1503[0]))), vshlq_u32(t1501[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1503[0])))), vorrq_u32(vshlq_u32(t1501[1], vnegq_s32(vreinterpretq_s32_u32(t1503[1]))), vshlq_u32(t1501[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1503[1])))), vshlq_u32(t1501[2], vnegq_s32(vreinterpretq_s32_u32(t1503[0]))), vshlq_u32(t1501[3], vnegq_s32(vreinterpretq_s32_u32(t1503[1])))];
    x40 = t1504;
    let t1505 = [vdupq_n_u32(0u32); 2];
    let t1506: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t1505[0])), vmvnq_u32(vceqq_u32(x32[1], t1505[1]))];
    let t1507 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t1508: [uint32x4_t; 2] = [vaddq_u32(x32[0], t1507[0]), vaddq_u32(x32[1], t1507[1])];
    let t1509 = [vdupq_n_u32(64u32); 2];
    let t1510: [uint32x4_t; 2] = [vcgtq_u32(t1508[0], t1509[0]), vcgtq_u32(t1508[1], t1509[1])];
    let t1511: [uint32x4_t; 2] = [vandq_u32(t1506[0], t1510[0]), vandq_u32(t1506[1], t1510[1])];
    let mut t1512: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1513: [uint32x4_t; 2] = [vandq_u32(live[0], t1511[0]), vandq_u32(live[1], t1511[1])];
    let t1514: [uint32x4_t; 2] = [vbicq_u32(live[0], t1511[0]), vbicq_u32(live[1], t1511[1])];
    if any(&t1513) {
        let t1515: u64 = (1u64).wrapping_mul(8u64);
        let t1516 = [vdupq_n_u32((t1515) as u32); 2];
        let t1517 = [vdupq_n_u32(((t1515) >> 32) as u32); 2];
        let t1518: [uint32x4_t; 2] = [vaddq_u32(x24[0], t1516[0]), vaddq_u32(x24[1], t1516[1])];
        let t1519: [uint32x4_t; 2] = [vcltq_u32(t1518[0], x24[0]), vcltq_u32(t1518[1], x24[1])];
        let t1520: [uint32x4_t; 4] = [t1518[0], t1518[1], vsubq_u32(vaddq_u32(x24[2], t1517[0]), t1519[0]), vsubq_u32(vaddq_u32(x24[3], t1517[1]), t1519[1])];
        let mut t1521: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1520[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1520[0]) as u64) as usize as *const u64).read_unaligned(); t1521[0] = vsetq_lane_u32::<0>(v as u32, t1521[0]); t1521[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1521[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1520[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1520[0]) as u64) as usize as *const u64).read_unaligned(); t1521[0] = vsetq_lane_u32::<1>(v as u32, t1521[0]); t1521[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1521[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1520[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1520[0]) as u64) as usize as *const u64).read_unaligned(); t1521[0] = vsetq_lane_u32::<2>(v as u32, t1521[0]); t1521[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1521[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1520[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1520[0]) as u64) as usize as *const u64).read_unaligned(); t1521[0] = vsetq_lane_u32::<3>(v as u32, t1521[0]); t1521[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1521[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1520[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1520[1]) as u64) as usize as *const u64).read_unaligned(); t1521[1] = vsetq_lane_u32::<0>(v as u32, t1521[1]); t1521[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1521[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1520[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1520[1]) as u64) as usize as *const u64).read_unaligned(); t1521[1] = vsetq_lane_u32::<1>(v as u32, t1521[1]); t1521[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1521[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1520[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1520[1]) as u64) as usize as *const u64).read_unaligned(); t1521[1] = vsetq_lane_u32::<2>(v as u32, t1521[1]); t1521[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1521[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1520[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1520[1]) as u64) as usize as *const u64).read_unaligned(); t1521[1] = vsetq_lane_u32::<3>(v as u32, t1521[1]); t1521[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1521[3]); }
        let t1522: [uint32x4_t; 2] = [vsubq_u32(t1509[0], x32[0]), vsubq_u32(t1509[1], x32[1])];
        let t1523: [uint32x4_t; 2] = [t1522[0], t1522[1]];
        let t1524: [uint32x4_t; 2] = [vandq_u32(t1523[0], vdupq_n_u32(63)), vandq_u32(t1523[1], vdupq_n_u32(63))];
        let t1525: [uint32x4_t; 4] = [vshlq_u32(t1521[0], vreinterpretq_s32_u32(t1524[0])), vshlq_u32(t1521[1], vreinterpretq_s32_u32(t1524[1])), vorrq_u32(vshlq_u32(t1521[2], vreinterpretq_s32_u32(t1524[0])), vshlq_u32(t1521[0], vsubq_s32(vreinterpretq_s32_u32(t1524[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1521[3], vreinterpretq_s32_u32(t1524[1])), vshlq_u32(t1521[1], vsubq_s32(vreinterpretq_s32_u32(t1524[1]), vdupq_n_s32(32))))];
        let t1526: [uint32x4_t; 4] = [vorrq_u32(x40[0], t1525[0]), vorrq_u32(x40[1], t1525[1]), vorrq_u32(x40[2], t1525[2]), vorrq_u32(x40[3], t1525[3])];
        t1512 = [vbslq_u32(t1513[0], t1526[0], t1512[0]), vbslq_u32(t1513[1], t1526[1], t1512[1]), vbslq_u32(t1513[0], t1526[2], t1512[2]), vbslq_u32(t1513[1], t1526[3], t1512[3])];
    }
    {
        t1512 = [vbslq_u32(t1514[0], x40[0], t1512[0]), vbslq_u32(t1514[1], x40[1], t1512[1]), vbslq_u32(t1514[0], x40[2], t1512[2]), vbslq_u32(t1514[1], x40[3], t1512[3])];
    }
    res = t1512;
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
        let t1527: [uint32x4_t; 4];
        if (x8is) == 8u32 && ((x8if) as u64) + 7 * ((x8is) as u64) <= u32::MAX as u64 {
            let t1528 = ((arg0).wrapping_add((x8b) as usize) as *const u8).add(((x8if) as usize) * 1);
            let t15290 = vld2q_u32((t1528 as *const u32).add(0));
            let t15291 = vld2q_u32((t1528 as *const u32).add(8));
            let t1529: [uint32x4_t; 4] = [t15290.0, t15291.0, t15290.1, t15291.1];
            t1527 = t1529;
        } else {
            let t1530: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8is)), vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8is))];
            let mut t1531: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1530[0]) as usize * 1) as *const u64).read_unaligned(); t1531[0] = vsetq_lane_u32::<0>(v as u32, t1531[0]); t1531[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1531[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1530[0]) as usize * 1) as *const u64).read_unaligned(); t1531[0] = vsetq_lane_u32::<1>(v as u32, t1531[0]); t1531[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1531[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1530[0]) as usize * 1) as *const u64).read_unaligned(); t1531[0] = vsetq_lane_u32::<2>(v as u32, t1531[0]); t1531[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1531[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1530[0]) as usize * 1) as *const u64).read_unaligned(); t1531[0] = vsetq_lane_u32::<3>(v as u32, t1531[0]); t1531[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1531[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1530[1]) as usize * 1) as *const u64).read_unaligned(); t1531[1] = vsetq_lane_u32::<0>(v as u32, t1531[1]); t1531[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1531[3]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1530[1]) as usize * 1) as *const u64).read_unaligned(); t1531[1] = vsetq_lane_u32::<1>(v as u32, t1531[1]); t1531[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1531[3]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1530[1]) as usize * 1) as *const u64).read_unaligned(); t1531[1] = vsetq_lane_u32::<2>(v as u32, t1531[1]); t1531[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1531[3]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1530[1]) as usize * 1) as *const u64).read_unaligned(); t1531[1] = vsetq_lane_u32::<3>(v as u32, t1531[1]); t1531[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1531[3]); }
            t1527 = t1531;
        }
        res = t1527;
        return res;
    }
    let t1532: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8is)), vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8is))];
    let t1533: [uint32x4_t; 2] = [t1532[0], t1532[1]];
    let t1534: u64 = ((x8b) as usize as u64);
    let t1535 = [vdupq_n_u32((t1534) as u32); 2];
    let t1536 = [vdupq_n_u32(((t1534) >> 32) as u32); 2];
    let t1537: [uint32x4_t; 2] = [vaddq_u32(t1535[0], t1533[0]), vaddq_u32(t1535[1], t1533[1])];
    let t1538: [uint32x4_t; 2] = [vcltq_u32(t1537[0], t1535[0]), vcltq_u32(t1537[1], t1535[1])];
    let t1539: [uint32x4_t; 4] = [t1537[0], t1537[1], vsubq_u32(vaddq_u32(t1536[0], vdupq_n_u32(0)), t1538[0]), vsubq_u32(vaddq_u32(t1536[1], vdupq_n_u32(0)), t1538[1])];
    let t1540: u64 = !(7u64);
    let t1541 = [vdupq_n_u32((t1540) as u32); 2];
    let t1542 = [vdupq_n_u32(((t1540) >> 32) as u32); 2];
    let t1543: [uint32x4_t; 4] = [vandq_u32(t1539[0], t1541[0]), vandq_u32(t1539[1], t1541[1]), vandq_u32(t1539[2], t1542[0]), vandq_u32(t1539[3], t1542[1])];
    let t1544: u64 = ((arg0) as usize as u64);
    let t1545 = [vdupq_n_u32((t1544) as u32); 2];
    let t1546 = [vdupq_n_u32(((t1544) >> 32) as u32); 2];
    let t1547: [uint32x4_t; 2] = [vaddq_u32(t1545[0], t1543[0]), vaddq_u32(t1545[1], t1543[1])];
    let t1548: [uint32x4_t; 2] = [vcltq_u32(t1547[0], t1545[0]), vcltq_u32(t1547[1], t1545[1])];
    let t1549: [uint32x4_t; 4] = [t1547[0], t1547[1], vsubq_u32(vaddq_u32(t1546[0], t1543[2]), t1548[0]), vsubq_u32(vaddq_u32(t1546[1], t1543[3]), t1548[1])];
    x24 = t1549;
    let t1550: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8is)), vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8is))];
    let t1551: [uint32x4_t; 2] = [t1550[0], t1550[1]];
    let t1552: u64 = ((x8b) as usize as u64);
    let t1553 = [vdupq_n_u32((t1552) as u32); 2];
    let t1554 = [vdupq_n_u32(((t1552) >> 32) as u32); 2];
    let t1555: [uint32x4_t; 2] = [vaddq_u32(t1553[0], t1551[0]), vaddq_u32(t1553[1], t1551[1])];
    let t1556: [uint32x4_t; 2] = [vcltq_u32(t1555[0], t1553[0]), vcltq_u32(t1555[1], t1553[1])];
    let t1557: [uint32x4_t; 4] = [t1555[0], t1555[1], vsubq_u32(vaddq_u32(t1554[0], vdupq_n_u32(0)), t1556[0]), vsubq_u32(vaddq_u32(t1554[1], vdupq_n_u32(0)), t1556[1])];
    let t1558: u64 = 7u64;
    let t1559 = [vdupq_n_u32((t1558) as u32); 2];
    let t1560: [uint32x4_t; 2] = [vandq_u32(t1557[0], t1559[0]), vandq_u32(t1557[1], t1559[1])];
    let t1561 = [vdupq_n_u32(8u32); 2];
    let t1562: [uint32x4_t; 2] = [vmulq_u32(t1560[0], t1561[0]), vmulq_u32(t1560[1], t1561[1])];
    x32 = t1562;
    let mut t1563: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x24[2]) as u64) << 32 | vgetq_lane_u32::<0>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1563[0] = vsetq_lane_u32::<0>(v as u32, t1563[0]); t1563[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1563[2]); }
    { let v = (((vgetq_lane_u32::<1>(x24[2]) as u64) << 32 | vgetq_lane_u32::<1>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1563[0] = vsetq_lane_u32::<1>(v as u32, t1563[0]); t1563[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1563[2]); }
    { let v = (((vgetq_lane_u32::<2>(x24[2]) as u64) << 32 | vgetq_lane_u32::<2>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1563[0] = vsetq_lane_u32::<2>(v as u32, t1563[0]); t1563[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1563[2]); }
    { let v = (((vgetq_lane_u32::<3>(x24[2]) as u64) << 32 | vgetq_lane_u32::<3>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t1563[0] = vsetq_lane_u32::<3>(v as u32, t1563[0]); t1563[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1563[2]); }
    { let v = (((vgetq_lane_u32::<0>(x24[3]) as u64) << 32 | vgetq_lane_u32::<0>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1563[1] = vsetq_lane_u32::<0>(v as u32, t1563[1]); t1563[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1563[3]); }
    { let v = (((vgetq_lane_u32::<1>(x24[3]) as u64) << 32 | vgetq_lane_u32::<1>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1563[1] = vsetq_lane_u32::<1>(v as u32, t1563[1]); t1563[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1563[3]); }
    { let v = (((vgetq_lane_u32::<2>(x24[3]) as u64) << 32 | vgetq_lane_u32::<2>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1563[1] = vsetq_lane_u32::<2>(v as u32, t1563[1]); t1563[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1563[3]); }
    { let v = (((vgetq_lane_u32::<3>(x24[3]) as u64) << 32 | vgetq_lane_u32::<3>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t1563[1] = vsetq_lane_u32::<3>(v as u32, t1563[1]); t1563[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1563[3]); }
    let t1564: [uint32x4_t; 2] = [x32[0], x32[1]];
    let t1565: [uint32x4_t; 2] = [vandq_u32(t1564[0], vdupq_n_u32(63)), vandq_u32(t1564[1], vdupq_n_u32(63))];
    let t1566: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t1563[0], vnegq_s32(vreinterpretq_s32_u32(t1565[0]))), vshlq_u32(t1563[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1565[0])))), vorrq_u32(vshlq_u32(t1563[1], vnegq_s32(vreinterpretq_s32_u32(t1565[1]))), vshlq_u32(t1563[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1565[1])))), vshlq_u32(t1563[2], vnegq_s32(vreinterpretq_s32_u32(t1565[0]))), vshlq_u32(t1563[3], vnegq_s32(vreinterpretq_s32_u32(t1565[1])))];
    x40 = t1566;
    let t1567 = [vdupq_n_u32(0u32); 2];
    let t1568: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t1567[0])), vmvnq_u32(vceqq_u32(x32[1], t1567[1]))];
    let t1569 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t1570: [uint32x4_t; 2] = [vaddq_u32(x32[0], t1569[0]), vaddq_u32(x32[1], t1569[1])];
    let t1571 = [vdupq_n_u32(64u32); 2];
    let t1572: [uint32x4_t; 2] = [vcgtq_u32(t1570[0], t1571[0]), vcgtq_u32(t1570[1], t1571[1])];
    let t1573: [uint32x4_t; 2] = [vandq_u32(t1568[0], t1572[0]), vandq_u32(t1568[1], t1572[1])];
    let mut t1574: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1575: [uint32x4_t; 2] = [vmvnq_u32(t1573[0]), vmvnq_u32(t1573[1])];
    if any(&t1573) {
        let t1576: u64 = (1u64).wrapping_mul(8u64);
        let t1577 = [vdupq_n_u32((t1576) as u32); 2];
        let t1578 = [vdupq_n_u32(((t1576) >> 32) as u32); 2];
        let t1579: [uint32x4_t; 2] = [vaddq_u32(x24[0], t1577[0]), vaddq_u32(x24[1], t1577[1])];
        let t1580: [uint32x4_t; 2] = [vcltq_u32(t1579[0], x24[0]), vcltq_u32(t1579[1], x24[1])];
        let t1581: [uint32x4_t; 4] = [t1579[0], t1579[1], vsubq_u32(vaddq_u32(x24[2], t1578[0]), t1580[0]), vsubq_u32(vaddq_u32(x24[3], t1578[1]), t1580[1])];
        let mut t1582: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1581[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1581[0]) as u64) as usize as *const u64).read_unaligned(); t1582[0] = vsetq_lane_u32::<0>(v as u32, t1582[0]); t1582[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1582[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1581[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1581[0]) as u64) as usize as *const u64).read_unaligned(); t1582[0] = vsetq_lane_u32::<1>(v as u32, t1582[0]); t1582[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1582[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1581[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1581[0]) as u64) as usize as *const u64).read_unaligned(); t1582[0] = vsetq_lane_u32::<2>(v as u32, t1582[0]); t1582[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1582[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1581[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1581[0]) as u64) as usize as *const u64).read_unaligned(); t1582[0] = vsetq_lane_u32::<3>(v as u32, t1582[0]); t1582[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1582[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1581[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1581[1]) as u64) as usize as *const u64).read_unaligned(); t1582[1] = vsetq_lane_u32::<0>(v as u32, t1582[1]); t1582[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1582[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1581[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1581[1]) as u64) as usize as *const u64).read_unaligned(); t1582[1] = vsetq_lane_u32::<1>(v as u32, t1582[1]); t1582[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1582[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1581[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1581[1]) as u64) as usize as *const u64).read_unaligned(); t1582[1] = vsetq_lane_u32::<2>(v as u32, t1582[1]); t1582[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1582[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1581[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1581[1]) as u64) as usize as *const u64).read_unaligned(); t1582[1] = vsetq_lane_u32::<3>(v as u32, t1582[1]); t1582[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1582[3]); }
        let t1583: [uint32x4_t; 2] = [vsubq_u32(t1571[0], x32[0]), vsubq_u32(t1571[1], x32[1])];
        let t1584: [uint32x4_t; 2] = [t1583[0], t1583[1]];
        let t1585: [uint32x4_t; 2] = [vandq_u32(t1584[0], vdupq_n_u32(63)), vandq_u32(t1584[1], vdupq_n_u32(63))];
        let t1586: [uint32x4_t; 4] = [vshlq_u32(t1582[0], vreinterpretq_s32_u32(t1585[0])), vshlq_u32(t1582[1], vreinterpretq_s32_u32(t1585[1])), vorrq_u32(vshlq_u32(t1582[2], vreinterpretq_s32_u32(t1585[0])), vshlq_u32(t1582[0], vsubq_s32(vreinterpretq_s32_u32(t1585[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1582[3], vreinterpretq_s32_u32(t1585[1])), vshlq_u32(t1582[1], vsubq_s32(vreinterpretq_s32_u32(t1585[1]), vdupq_n_s32(32))))];
        let t1587: [uint32x4_t; 4] = [vorrq_u32(x40[0], t1586[0]), vorrq_u32(x40[1], t1586[1]), vorrq_u32(x40[2], t1586[2]), vorrq_u32(x40[3], t1586[3])];
        t1574 = [vbslq_u32(t1573[0], t1587[0], t1574[0]), vbslq_u32(t1573[1], t1587[1], t1574[1]), vbslq_u32(t1573[0], t1587[2], t1574[2]), vbslq_u32(t1573[1], t1587[3], t1574[3])];
    }
    {
        t1574 = [vbslq_u32(t1575[0], x40[0], t1574[0]), vbslq_u32(t1575[1], x40[1], t1574[1]), vbslq_u32(t1575[0], x40[2], t1574[2]), vbslq_u32(t1575[1], x40[3], t1574[3])];
    }
    res = t1574;
    return res;
}

#[inline(always)]
unsafe fn b29_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t1588: u64;
    if ((x0) >= (64u32)) {
        t1588 = !(0u64);
    } else {
        t1588 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t1588;
    return res;
}

#[inline(always)]
unsafe fn b29_s_f(l: &Launch, lid0: u32, x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t1589: u64;
    if ((x0) >= (64u32)) {
        t1589 = !(0u64);
    } else {
        t1589 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t1589;
    return res;
}

#[inline(always)]
unsafe fn b30_r0sss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: u32, x16: u32, x24: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u64 = 0;
    let mut live = k;
    let mut res: u64 = 0;
    let t1590 = b28_r0ss_p(l, lid0, live, arg0, (((x8) as u64)).wrapping_add(((x16) as u64)), x24);
    x32 = t1590;
    let t1591: u64;
    if ((x24) >= (8u32)) {
        t1591 = x32;
    } else {
        let t1592 = b29_s_p(l, lid0, live, (8u32).wrapping_mul(x24));
        t1591 = ((x32) & (t1592));
    }
    res = t1591;
    return res;
}

#[inline(always)]
unsafe fn b30_r0sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: u32, x16: [uint32x4_t; 2], x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1593 = b28_r0al1s_p(l, lid0, live, arg0, ((((x8) as u64)) as usize as *const u8), x16, x24);
    x32 = t1593;
    let t1594: [uint32x4_t; 4];
    if ((x24) >= (8u32)) {
        t1594 = x32;
    } else {
        let t1595 = b29_s_p(l, lid0, live, (8u32).wrapping_mul(x24));
        let t1596: u64 = t1595;
        let t1597 = [vdupq_n_u32((t1596) as u32); 2];
        let t1598 = [vdupq_n_u32(((t1596) >> 32) as u32); 2];
        let t1599: [uint32x4_t; 4] = [vandq_u32(x32[0], t1597[0]), vandq_u32(x32[1], t1597[1]), vandq_u32(x32[2], t1598[0]), vandq_u32(x32[3], t1598[1])];
        t1594 = t1599;
    }
    res = t1594;
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
    let mut t1600: bool = !(arg6).is_null();
    if t1600 {
        let mut t1601: bool = ((l.group[0]) >= ((arg6.wrapping_add((((x64) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        if !t1601 {
            t1601 = ((l.group[1]) >= ((arg6.wrapping_add(((((x64).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        let mut t1602: bool = t1601;
        if !t1602 {
            t1602 = ((l.group[2]) >= ((arg6.wrapping_add(((((x64).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        t1600 = t1602;
    }
    if t1600 {
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
        let t1603: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
        let mut live = t1603;
        let gid0 = l.group[0] * l.block[0] + lid0;
        'chunk: {
            if (lid0 & 31u32) <= 24u32 { b0_r0r1r2r3r4sgu5_p(l, lid0, live, arg0, arg1, arg2, arg3, arg4, x40, lid0) } else { b0_r0r1r2r3r4sgi_p(l, lid0, live, arg0, arg1, arg2, arg3, arg4, x40, lid0) };
        }
    }
}
