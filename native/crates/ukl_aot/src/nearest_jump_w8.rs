//! `nearest_jump` at 8 lanes (2 NEON registers), written by rerail-ukl-vec's Rust (`rs.rs`) from its vector tape: do
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

/// `nearest_jump_inner`.
#[inline(always)]
unsafe fn b0_r0r1sg_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: *const u8) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x2.wrapping_add(4u64 as usize) as *const u32).read_unaligned();
    let a1: u32 = (x2 as *const u32).read_unaligned();
    let mut x4: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x5: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x6: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x7: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let t1: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
    let t2 = [vdupq_n_u32(a0); 2];
    let t3: [uint32x4_t; 2] = [vcgeq_u32(t1[0], t2[0]), vcgeq_u32(t1[1], t2[1])];
    let t4: [uint32x4_t; 2] = [vandq_u32(live[0], t3[0]), vandq_u32(live[1], t3[1])];
    {
        live = [vbicq_u32(live[0], t4[0]), vbicq_u32(live[1], t4[1])];
    }
    if any(&live) {
        let t5 = b1_ag4r0_p(l, lid0, live, arg0);
        x4 = t5;
        let t6 = [vdupq_n_u32(a1); 2];
        let mut t7x = [0u32; 8];
        let mut t7y = [0u32; 8];
        for r in 0..2 { vst1q_u32(t7x.as_mut_ptr().add(4 * r), t1[r]); vst1q_u32(t7y.as_mut_ptr().add(4 * r), t6[r]); }
        for j in 0..8 { t7x[j] = (t7x[j] as u32).checked_div(t7y[j] as u32).unwrap_or(0) as u32; }
        let t8: [uint32x4_t; 2] = [vld1q_u32(t7x.as_ptr().add(0)), vld1q_u32(t7x.as_ptr().add(4))];
        let t9: [uint32x4_t; 2] = [vmulq_u32(t8[0], t6[0]), vmulq_u32(t8[1], t6[1])];
        x5 = t9;
        let t10 = [vdupq_n_u32(0u32); 2];
        x6 = t10;
        let mut t16 = live;
        loop {
            // Lanes part: each its own rounds.
            let t18 = [vdupq_n_u32(2147483648u32); 2];
            let t19: [uint32x4_t; 2] = [vandq_u32(x4[0], t18[0]), vandq_u32(x4[1], t18[1])];
            let t20: [uint32x4_t; 2] = [vceqq_u32(t19[0], t10[0]), vceqq_u32(t19[1], t10[1])];
            let t21: [uint32x4_t; 2] = [vcltq_u32(x6[0], t6[0]), vcltq_u32(x6[1], t6[1])];
            let t22: [uint32x4_t; 2] = [vandq_u32(t20[0], t21[0]), vandq_u32(t20[1], t21[1])];
            t16 = [vandq_u32(t16[0], t22[0]), vandq_u32(t16[1], t22[1])];
            if !(any(&t16)) { break; }
            let t24: [uint32x4_t; 2] = [vaddq_u32(x5[0], x4[0]), vaddq_u32(x5[1], x4[1])];
            let t25 = b1_al4r0_p(l, lid0, t16, arg0, t24);
            x7 = t25;
            x4 = x7;
            let t26 = x6;
            x6 = [vsubq_u32(x6[0], t16[0]), vsubq_u32(x6[1], t16[1])];
            b2_ag4r0l_p(l, lid0, t16, arg0, x4);
        }
        let t27 = b3_o1l_p(l, lid0, live, arg1.wrapping_add((0u64).wrapping_mul(4u64) as usize), x6);
    }
}

/// `nearest_jump_inner`.
#[inline(always)]
unsafe fn b0_r0r1sg_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: *const u8) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x2.wrapping_add(4u64 as usize) as *const u32).read_unaligned();
    let a1: u32 = (x2 as *const u32).read_unaligned();
    let mut x4: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x5: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x6: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x7: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    if ((gid0) as u64 + 7) < ((a0) as u64) {
        let t28 = b1_ag4r0_f(l, lid0, arg0);
        x4 = t28;
        let t29: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t30 = [vdupq_n_u32(a1); 2];
        let mut t31x = [0u32; 8];
        let mut t31y = [0u32; 8];
        for r in 0..2 { vst1q_u32(t31x.as_mut_ptr().add(4 * r), t29[r]); vst1q_u32(t31y.as_mut_ptr().add(4 * r), t30[r]); }
        for j in 0..8 { t31x[j] = (t31x[j] as u32).checked_div(t31y[j] as u32).unwrap_or(0) as u32; }
        let t32: [uint32x4_t; 2] = [vld1q_u32(t31x.as_ptr().add(0)), vld1q_u32(t31x.as_ptr().add(4))];
        let t33: [uint32x4_t; 2] = [vmulq_u32(t32[0], t30[0]), vmulq_u32(t32[1], t30[1])];
        x5 = t33;
        let t34 = [vdupq_n_u32(0u32); 2];
        x6 = t34;
        let mut t45 = live;
        loop {
            // Lanes part: each its own rounds.
            let t47 = [vdupq_n_u32(2147483648u32); 2];
            let t48: [uint32x4_t; 2] = [vandq_u32(x4[0], t47[0]), vandq_u32(x4[1], t47[1])];
            let t49: [uint32x4_t; 2] = [vceqq_u32(t48[0], t34[0]), vceqq_u32(t48[1], t34[1])];
            let t50: [uint32x4_t; 2] = [vcltq_u32(x6[0], t30[0]), vcltq_u32(x6[1], t30[1])];
            let t51: [uint32x4_t; 2] = [vandq_u32(t49[0], t50[0]), vandq_u32(t49[1], t50[1])];
            t45 = [vandq_u32(t45[0], t51[0]), vandq_u32(t45[1], t51[1])];
            if !(any(&t45)) { break; }
            let t53: [uint32x4_t; 2] = [vaddq_u32(x5[0], x4[0]), vaddq_u32(x5[1], x4[1])];
            let t54 = b1_al4r0_p_v0(l, lid0, t45, arg0, t53);
            x7 = t54;
            x4 = x7;
            let t55 = x6;
            x6 = [vsubq_u32(x6[0], t45[0]), vsubq_u32(x6[1], t45[1])];
            b2_ag4r0l_p_v0(l, lid0, t45, arg0, x4);
        }
        let t56 = b3_o1l_f(l, lid0, arg1.wrapping_add((0u64).wrapping_mul(4u64) as usize), x6);
    } else if (((gid0) as u64 + 7) <= u32::MAX as u64 && ((gid0) as u64) >= ((a0) as u64)) {
        return;
    } else {
        // Lanes part: the chunk's lanes on both sides of the bound.
        let t57: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t58 = [vdupq_n_u32(a0); 2];
        let t59: [uint32x4_t; 2] = [vcgeq_u32(t57[0], t58[0]), vcgeq_u32(t57[1], t58[1])];
        let t60: [uint32x4_t; 2] = [vandq_u32(live[0], t59[0]), vandq_u32(live[1], t59[1])];
        {
            live = [vbicq_u32(live[0], t60[0]), vbicq_u32(live[1], t60[1])];
        }
        if any(&live) {
            let t61 = b1_ag4r0_p(l, lid0, live, arg0);
            x4 = t61;
            let t62 = [vdupq_n_u32(a1); 2];
            let mut t63x = [0u32; 8];
            let mut t63y = [0u32; 8];
            for r in 0..2 { vst1q_u32(t63x.as_mut_ptr().add(4 * r), t57[r]); vst1q_u32(t63y.as_mut_ptr().add(4 * r), t62[r]); }
            for j in 0..8 { t63x[j] = (t63x[j] as u32).checked_div(t63y[j] as u32).unwrap_or(0) as u32; }
            let t64: [uint32x4_t; 2] = [vld1q_u32(t63x.as_ptr().add(0)), vld1q_u32(t63x.as_ptr().add(4))];
            let t65: [uint32x4_t; 2] = [vmulq_u32(t64[0], t62[0]), vmulq_u32(t64[1], t62[1])];
            x5 = t65;
            let t66 = [vdupq_n_u32(0u32); 2];
            x6 = t66;
            let mut t72 = live;
            loop {
                // Lanes part: each its own rounds.
                let t74 = [vdupq_n_u32(2147483648u32); 2];
                let t75: [uint32x4_t; 2] = [vandq_u32(x4[0], t74[0]), vandq_u32(x4[1], t74[1])];
                let t76: [uint32x4_t; 2] = [vceqq_u32(t75[0], t66[0]), vceqq_u32(t75[1], t66[1])];
                let t77: [uint32x4_t; 2] = [vcltq_u32(x6[0], t62[0]), vcltq_u32(x6[1], t62[1])];
                let t78: [uint32x4_t; 2] = [vandq_u32(t76[0], t77[0]), vandq_u32(t76[1], t77[1])];
                t72 = [vandq_u32(t72[0], t78[0]), vandq_u32(t72[1], t78[1])];
                if !(any(&t72)) { break; }
                let t80: [uint32x4_t; 2] = [vaddq_u32(x5[0], x4[0]), vaddq_u32(x5[1], x4[1])];
                let t81 = b1_al4r0_p(l, lid0, t72, arg0, t80);
                x7 = t81;
                x4 = x7;
                let t82 = x6;
                x6 = [vsubq_u32(x6[0], t72[0]), vsubq_u32(x6[1], t72[1])];
                b2_ag4r0l_p(l, lid0, t72, arg0, x4);
            }
            let t83 = b3_o1l_p(l, lid0, live, arg1.wrapping_add((0u64).wrapping_mul(4u64) as usize), x6);
        }
    }
}

/// `ukl_atomic_load`.
#[inline(always)]
unsafe fn b1_al4r0_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x0i: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t84 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], x0i[0]), vandq_u32(live[1], x0i[1]))));
    let t85: [uint32x4_t; 2] = [vbslq_u32(live[0], x0i[0], t84), vbslq_u32(live[1], x0i[1], t84)];
    let t86 = arg0 as *const u32;
    let mut t87: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    t87[0] = vld1q_lane_u32::<0>(t86.add(vgetq_lane_u32::<0>(t85[0]) as usize), t87[0]);
    t87[0] = vld1q_lane_u32::<1>(t86.add(vgetq_lane_u32::<1>(t85[0]) as usize), t87[0]);
    t87[0] = vld1q_lane_u32::<2>(t86.add(vgetq_lane_u32::<2>(t85[0]) as usize), t87[0]);
    t87[0] = vld1q_lane_u32::<3>(t86.add(vgetq_lane_u32::<3>(t85[0]) as usize), t87[0]);
    t87[1] = vld1q_lane_u32::<0>(t86.add(vgetq_lane_u32::<0>(t85[1]) as usize), t87[1]);
    t87[1] = vld1q_lane_u32::<1>(t86.add(vgetq_lane_u32::<1>(t85[1]) as usize), t87[1]);
    t87[1] = vld1q_lane_u32::<2>(t86.add(vgetq_lane_u32::<2>(t85[1]) as usize), t87[1]);
    t87[1] = vld1q_lane_u32::<3>(t86.add(vgetq_lane_u32::<3>(t85[1]) as usize), t87[1]);
    res = t87;
    return res;
}

/// `ukl_atomic_load`.
#[inline(always)]
unsafe fn b1_al4r0_p_v0(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x0i: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t88: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
    let t89: [uint32x4_t; 2] = [vbslq_u32(live[0], x0i[0], t88[0]), vbslq_u32(live[1], x0i[1], t88[1])];
    let t90 = arg0 as *const u32;
    let mut t91: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    t91[0] = vld1q_lane_u32::<0>(t90.add(vgetq_lane_u32::<0>(t89[0]) as usize), t91[0]);
    t91[0] = vld1q_lane_u32::<1>(t90.add(vgetq_lane_u32::<1>(t89[0]) as usize), t91[0]);
    t91[0] = vld1q_lane_u32::<2>(t90.add(vgetq_lane_u32::<2>(t89[0]) as usize), t91[0]);
    t91[0] = vld1q_lane_u32::<3>(t90.add(vgetq_lane_u32::<3>(t89[0]) as usize), t91[0]);
    t91[1] = vld1q_lane_u32::<0>(t90.add(vgetq_lane_u32::<0>(t89[1]) as usize), t91[1]);
    t91[1] = vld1q_lane_u32::<1>(t90.add(vgetq_lane_u32::<1>(t89[1]) as usize), t91[1]);
    t91[1] = vld1q_lane_u32::<2>(t90.add(vgetq_lane_u32::<2>(t89[1]) as usize), t91[1]);
    t91[1] = vld1q_lane_u32::<3>(t90.add(vgetq_lane_u32::<3>(t89[1]) as usize), t91[1]);
    res = t91;
    return res;
}

/// `ukl_atomic_load`.
#[inline(always)]
unsafe fn b1_ag4r0_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut t92: [uint32x4_t; 2];
    if all(&live) {
        let t93 = (arg0 as *const u32).add((gid0) as usize);
        t92 = [vld1q_u32(t93.add(0)), vld1q_u32(t93.add(4))];
    } else {
        let t94: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t95 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t94[0]), vandq_u32(live[1], t94[1]))));
        let t96: [uint32x4_t; 2] = [vbslq_u32(live[0], t94[0], t95), vbslq_u32(live[1], t94[1], t95)];
        let t97 = arg0 as *const u32;
        let mut t98: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t98[0] = vld1q_lane_u32::<0>(t97.add(vgetq_lane_u32::<0>(t96[0]) as usize), t98[0]);
        t98[0] = vld1q_lane_u32::<1>(t97.add(vgetq_lane_u32::<1>(t96[0]) as usize), t98[0]);
        t98[0] = vld1q_lane_u32::<2>(t97.add(vgetq_lane_u32::<2>(t96[0]) as usize), t98[0]);
        t98[0] = vld1q_lane_u32::<3>(t97.add(vgetq_lane_u32::<3>(t96[0]) as usize), t98[0]);
        t98[1] = vld1q_lane_u32::<0>(t97.add(vgetq_lane_u32::<0>(t96[1]) as usize), t98[1]);
        t98[1] = vld1q_lane_u32::<1>(t97.add(vgetq_lane_u32::<1>(t96[1]) as usize), t98[1]);
        t98[1] = vld1q_lane_u32::<2>(t97.add(vgetq_lane_u32::<2>(t96[1]) as usize), t98[1]);
        t98[1] = vld1q_lane_u32::<3>(t97.add(vgetq_lane_u32::<3>(t96[1]) as usize), t98[1]);
        t92 = t98;
    }
    res = t92;
    return res;
}

/// `ukl_atomic_load`.
#[inline(always)]
unsafe fn b1_ag4r0_f(l: &Launch, lid0: u32, arg0: *const u8) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t99 = (arg0 as *const u32).add((gid0) as usize);
    let t100: [uint32x4_t; 2] = [vld1q_u32(t99.add(0)), vld1q_u32(t99.add(4))];
    res = t100;
    return res;
}

/// `ukl_atomic_store`.
#[inline(always)]
unsafe fn b2_ag4r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1: [uint32x4_t; 2]) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let t101 = (arg0 as *mut u32).add((gid0) as usize);
    if all(&live) {
        vst1q_u32(t101.add(0), x1[0]);
        vst1q_u32(t101.add(4), x1[1]);
    } else {
        let mut t102w = 0u32;
        let t102 = core::hint::black_box(&raw mut t102w);
        core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t101.add(0), t102).write(vgetq_lane_u32::<0>(x1[0]));
        core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t101.add(1), t102).write(vgetq_lane_u32::<1>(x1[0]));
        core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t101.add(2), t102).write(vgetq_lane_u32::<2>(x1[0]));
        core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t101.add(3), t102).write(vgetq_lane_u32::<3>(x1[0]));
        core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t101.add(4), t102).write(vgetq_lane_u32::<0>(x1[1]));
        core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t101.add(5), t102).write(vgetq_lane_u32::<1>(x1[1]));
        core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t101.add(6), t102).write(vgetq_lane_u32::<2>(x1[1]));
        core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t101.add(7), t102).write(vgetq_lane_u32::<3>(x1[1]));
    }
}

/// `ukl_atomic_store`.
#[inline(always)]
unsafe fn b2_ag4r0l_p_v0(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1: [uint32x4_t; 2]) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let t103 = (arg0 as *mut u32).add((gid0) as usize);
    vst1q_u32(t103.add(0), vbslq_u32(live[0], x1[0], vld1q_u32(t103.add(0))));
    vst1q_u32(t103.add(4), vbslq_u32(live[1], x1[1], vld1q_u32(t103.add(4))));
}

/// `ukl_atomic_max`.
#[inline(always)]
unsafe fn b3_o1l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg1_x0: *const u8, x1: [uint32x4_t; 2]) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t104: [uint32x4_t; 2] = [vbslq_u32(live[0], x1[0], vdupq_n_u32(0)), vbslq_u32(live[1], x1[1], vdupq_n_u32(0))];
    let t105 = vmaxvq_u32(vmaxq_u32(t104[0], t104[1]));
    if t105 != 0 { AtomicU32::from_ptr(arg1_x0 as *mut u32).fetch_max(t105, Relaxed); }
    return res;
}

/// `ukl_atomic_max`.
#[inline(always)]
unsafe fn b3_o1l_f(l: &Launch, lid0: u32, arg1_x0: *const u8, x1: [uint32x4_t; 2]) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t106 = vmaxvq_u32(vmaxq_u32(x1[0], x1[1]));
    if t106 != 0 { AtomicU32::from_ptr(arg1_x0 as *mut u32).fetch_max(t106, Relaxed); }
    return res;
}

/// The group `l.group`'s threads, 8 at a time: `argv[j]` the address of argument `j`'s bytes (a buffer's address,
/// a value's bytes).
///
/// # Safety
///
/// `argv` holds the kernel's arguments, its buffers as large as it reads and writes; `l` is a 1-D group's launch.
pub unsafe fn nearest_jump_w8(argv: *const *const u8, l: &Launch) {
    let n = l.block[0] * l.block[1] * l.block[2];
    let arg0: *const u8 = (*argv.add(0) as *const *const u8).read_unaligned();
    let arg1: *const u8 = (*argv.add(1) as *const *const u8).read_unaligned();
    let x2: *const u8 = *argv.add(2);
    let arg3: *const u8 = (*argv.add(3) as *const *const u8).read_unaligned();
    let x4: u32 = (*argv.add(4) as *const u32).read_unaligned();
    let mut t107: bool = !(arg3).is_null();
    if t107 {
        let mut t108: bool = ((l.group[0]) >= ((arg3.wrapping_add((((x4) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        if !t108 {
            t108 = ((l.group[1]) >= ((arg3.wrapping_add(((((x4).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        let mut t109: bool = t108;
        if !t109 {
            t109 = ((l.group[2]) >= ((arg3.wrapping_add(((((x4).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        t107 = t109;
    }
    if t107 {
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
        let t110: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
        let mut live = t110;
        let gid0 = l.group[0] * l.block[0] + lid0;
        'chunk: {
            b0_r0r1sg_p(l, lid0, live, arg0, arg1, x2);
        }
    }
}
