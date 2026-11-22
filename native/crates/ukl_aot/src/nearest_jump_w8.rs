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

#[inline(always)]
unsafe fn b0_r0r1sg_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: *const u8) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x16.wrapping_add(4u64 as usize) as *const u32).read_unaligned();
    let a1: u32 = (x16 as *const u32).read_unaligned();
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
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
        let t5 = b1_ag4r0_p(l, lid0, live, arg0);
        x40 = t5;
        let t6 = [vdupq_n_u32(a1); 2];
        let mut t7x = [0u32; 8];
        let mut t7y = [0u32; 8];
        for r in 0..2 { vst1q_u32(t7x.as_mut_ptr().add(4 * r), t1[r]); vst1q_u32(t7y.as_mut_ptr().add(4 * r), t6[r]); }
        for j in 0..8 { t7x[j] = (t7x[j] as u32).checked_div(t7y[j] as u32).unwrap_or(0) as u32; }
        let t8: [uint32x4_t; 2] = [vld1q_u32(t7x.as_ptr().add(0)), vld1q_u32(t7x.as_ptr().add(4))];
        let t9: [uint32x4_t; 2] = [vmulq_u32(t8[0], t6[0]), vmulq_u32(t8[1], t6[1])];
        x48 = t9;
        let t10 = [vdupq_n_u32(0u32); 2];
        x56 = t10;
        let mut t11 = live;
        loop {
            // Lanes part: each its own rounds.
            let t13 = [vdupq_n_u32(2147483648u32); 2];
            let t14: [uint32x4_t; 2] = [vandq_u32(x40[0], t13[0]), vandq_u32(x40[1], t13[1])];
            let t15: [uint32x4_t; 2] = [vceqq_u32(t14[0], t10[0]), vceqq_u32(t14[1], t10[1])];
            let t16: [uint32x4_t; 2] = [vcltq_u32(x56[0], t6[0]), vcltq_u32(x56[1], t6[1])];
            let t17: [uint32x4_t; 2] = [vandq_u32(t15[0], t16[0]), vandq_u32(t15[1], t16[1])];
            t11 = [vandq_u32(t11[0], t17[0]), vandq_u32(t11[1], t17[1])];
            if !(any(&t11)) { break; }
            let t19: [uint32x4_t; 2] = [vaddq_u32(x48[0], x40[0]), vaddq_u32(x48[1], x40[1])];
            let t20 = b1_al4r0_p(l, lid0, t11, arg0, t19);
            x64 = t20;
            x40 = x64;
            let t21 = x56;
            x56 = [vsubq_u32(x56[0], t11[0]), vsubq_u32(x56[1], t11[1])];
            b2_ag4r0l_p(l, lid0, t11, arg0, x40);
        }
        let t22 = b3_o1l_p(l, lid0, live, arg1.wrapping_add((0u64).wrapping_mul(4u64) as usize), x56);
    }
}

#[inline(always)]
unsafe fn b0_r0r1sg_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: *const u8) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x16.wrapping_add(4u64 as usize) as *const u32).read_unaligned();
    let a1: u32 = (x16 as *const u32).read_unaligned();
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x56: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x64: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    if ((gid0) as u64 + 7) < ((a0) as u64) {
        let t23 = b1_ag4r0_f(l, lid0, arg0);
        x40 = t23;
        let t24: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t25 = [vdupq_n_u32(a1); 2];
        let mut t26x = [0u32; 8];
        let mut t26y = [0u32; 8];
        for r in 0..2 { vst1q_u32(t26x.as_mut_ptr().add(4 * r), t24[r]); vst1q_u32(t26y.as_mut_ptr().add(4 * r), t25[r]); }
        for j in 0..8 { t26x[j] = (t26x[j] as u32).checked_div(t26y[j] as u32).unwrap_or(0) as u32; }
        let t27: [uint32x4_t; 2] = [vld1q_u32(t26x.as_ptr().add(0)), vld1q_u32(t26x.as_ptr().add(4))];
        let t28: [uint32x4_t; 2] = [vmulq_u32(t27[0], t25[0]), vmulq_u32(t27[1], t25[1])];
        x48 = t28;
        let t29 = [vdupq_n_u32(0u32); 2];
        x56 = t29;
        let mut t35 = live;
        loop {
            // Lanes part: each its own rounds.
            let t37 = [vdupq_n_u32(2147483648u32); 2];
            let t38: [uint32x4_t; 2] = [vandq_u32(x40[0], t37[0]), vandq_u32(x40[1], t37[1])];
            let t39: [uint32x4_t; 2] = [vceqq_u32(t38[0], t29[0]), vceqq_u32(t38[1], t29[1])];
            let t40: [uint32x4_t; 2] = [vcltq_u32(x56[0], t25[0]), vcltq_u32(x56[1], t25[1])];
            let t41: [uint32x4_t; 2] = [vandq_u32(t39[0], t40[0]), vandq_u32(t39[1], t40[1])];
            t35 = [vandq_u32(t35[0], t41[0]), vandq_u32(t35[1], t41[1])];
            if !(any(&t35)) { break; }
            let t43: [uint32x4_t; 2] = [vaddq_u32(x48[0], x40[0]), vaddq_u32(x48[1], x40[1])];
            let t44 = b1_al4r0_p_v0(l, lid0, t35, arg0, t43);
            x64 = t44;
            x40 = x64;
            let t45 = x56;
            x56 = [vsubq_u32(x56[0], t35[0]), vsubq_u32(x56[1], t35[1])];
            b2_ag4r0l_p_v0(l, lid0, t35, arg0, x40);
        }
        let t46 = b3_o1l_f(l, lid0, arg1.wrapping_add((0u64).wrapping_mul(4u64) as usize), x56);
    } else if (((gid0) as u64 + 7) <= u32::MAX as u64 && ((gid0) as u64) >= ((a0) as u64)) {
        return;
    } else {
        // Lanes part: the chunk's lanes on both sides of the bound.
        let t47: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t48 = [vdupq_n_u32(a0); 2];
        let t49: [uint32x4_t; 2] = [vcgeq_u32(t47[0], t48[0]), vcgeq_u32(t47[1], t48[1])];
        let t50: [uint32x4_t; 2] = [vandq_u32(live[0], t49[0]), vandq_u32(live[1], t49[1])];
        if any(&t50) {
            live = [vbicq_u32(live[0], t50[0]), vbicq_u32(live[1], t50[1])];
        }
        if any(&live) {
            let t51 = b1_ag4r0_p(l, lid0, live, arg0);
            x40 = t51;
            let t52 = [vdupq_n_u32(a1); 2];
            let mut t53x = [0u32; 8];
            let mut t53y = [0u32; 8];
            for r in 0..2 { vst1q_u32(t53x.as_mut_ptr().add(4 * r), t47[r]); vst1q_u32(t53y.as_mut_ptr().add(4 * r), t52[r]); }
            for j in 0..8 { t53x[j] = (t53x[j] as u32).checked_div(t53y[j] as u32).unwrap_or(0) as u32; }
            let t54: [uint32x4_t; 2] = [vld1q_u32(t53x.as_ptr().add(0)), vld1q_u32(t53x.as_ptr().add(4))];
            let t55: [uint32x4_t; 2] = [vmulq_u32(t54[0], t52[0]), vmulq_u32(t54[1], t52[1])];
            x48 = t55;
            let t56 = [vdupq_n_u32(0u32); 2];
            x56 = t56;
            let mut t57 = live;
            loop {
                // Lanes part: each its own rounds.
                let t59 = [vdupq_n_u32(2147483648u32); 2];
                let t60: [uint32x4_t; 2] = [vandq_u32(x40[0], t59[0]), vandq_u32(x40[1], t59[1])];
                let t61: [uint32x4_t; 2] = [vceqq_u32(t60[0], t56[0]), vceqq_u32(t60[1], t56[1])];
                let t62: [uint32x4_t; 2] = [vcltq_u32(x56[0], t52[0]), vcltq_u32(x56[1], t52[1])];
                let t63: [uint32x4_t; 2] = [vandq_u32(t61[0], t62[0]), vandq_u32(t61[1], t62[1])];
                t57 = [vandq_u32(t57[0], t63[0]), vandq_u32(t57[1], t63[1])];
                if !(any(&t57)) { break; }
                let t65: [uint32x4_t; 2] = [vaddq_u32(x48[0], x40[0]), vaddq_u32(x48[1], x40[1])];
                let t66 = b1_al4r0_p(l, lid0, t57, arg0, t65);
                x64 = t66;
                x40 = x64;
                let t67 = x56;
                x56 = [vsubq_u32(x56[0], t57[0]), vsubq_u32(x56[1], t57[1])];
                b2_ag4r0l_p(l, lid0, t57, arg0, x40);
            }
            let t68 = b3_o1l_p(l, lid0, live, arg1.wrapping_add((0u64).wrapping_mul(4u64) as usize), x56);
        }
    }
}

#[inline(always)]
unsafe fn b1_al4r0_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x0i: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t69 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], x0i[0]), vandq_u32(live[1], x0i[1]))));
    let t70: [uint32x4_t; 2] = [vbslq_u32(live[0], x0i[0], t69), vbslq_u32(live[1], x0i[1], t69)];
    let t71 = arg0 as *const u32;
    let mut t72: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    t72[0] = vld1q_lane_u32::<0>(t71.add(vgetq_lane_u32::<0>(t70[0]) as usize), t72[0]);
    t72[0] = vld1q_lane_u32::<1>(t71.add(vgetq_lane_u32::<1>(t70[0]) as usize), t72[0]);
    t72[0] = vld1q_lane_u32::<2>(t71.add(vgetq_lane_u32::<2>(t70[0]) as usize), t72[0]);
    t72[0] = vld1q_lane_u32::<3>(t71.add(vgetq_lane_u32::<3>(t70[0]) as usize), t72[0]);
    t72[1] = vld1q_lane_u32::<0>(t71.add(vgetq_lane_u32::<0>(t70[1]) as usize), t72[1]);
    t72[1] = vld1q_lane_u32::<1>(t71.add(vgetq_lane_u32::<1>(t70[1]) as usize), t72[1]);
    t72[1] = vld1q_lane_u32::<2>(t71.add(vgetq_lane_u32::<2>(t70[1]) as usize), t72[1]);
    t72[1] = vld1q_lane_u32::<3>(t71.add(vgetq_lane_u32::<3>(t70[1]) as usize), t72[1]);
    res = t72;
    return res;
}

#[inline(always)]
unsafe fn b1_al4r0_p_v0(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x0i: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t73: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
    let t74: [uint32x4_t; 2] = [vbslq_u32(live[0], x0i[0], t73[0]), vbslq_u32(live[1], x0i[1], t73[1])];
    let t75 = arg0 as *const u32;
    let mut t76: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    t76[0] = vld1q_lane_u32::<0>(t75.add(vgetq_lane_u32::<0>(t74[0]) as usize), t76[0]);
    t76[0] = vld1q_lane_u32::<1>(t75.add(vgetq_lane_u32::<1>(t74[0]) as usize), t76[0]);
    t76[0] = vld1q_lane_u32::<2>(t75.add(vgetq_lane_u32::<2>(t74[0]) as usize), t76[0]);
    t76[0] = vld1q_lane_u32::<3>(t75.add(vgetq_lane_u32::<3>(t74[0]) as usize), t76[0]);
    t76[1] = vld1q_lane_u32::<0>(t75.add(vgetq_lane_u32::<0>(t74[1]) as usize), t76[1]);
    t76[1] = vld1q_lane_u32::<1>(t75.add(vgetq_lane_u32::<1>(t74[1]) as usize), t76[1]);
    t76[1] = vld1q_lane_u32::<2>(t75.add(vgetq_lane_u32::<2>(t74[1]) as usize), t76[1]);
    t76[1] = vld1q_lane_u32::<3>(t75.add(vgetq_lane_u32::<3>(t74[1]) as usize), t76[1]);
    res = t76;
    return res;
}

#[inline(always)]
unsafe fn b1_ag4r0_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut t77: [uint32x4_t; 2];
    if all(&live) {
        let t78 = (arg0 as *const u32).add((gid0) as usize);
        t77 = [vld1q_u32(t78.add(0)), vld1q_u32(t78.add(4))];
    } else {
        let t79: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t80 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t79[0]), vandq_u32(live[1], t79[1]))));
        let t81: [uint32x4_t; 2] = [vbslq_u32(live[0], t79[0], t80), vbslq_u32(live[1], t79[1], t80)];
        let t82 = arg0 as *const u32;
        let mut t83: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t83[0] = vld1q_lane_u32::<0>(t82.add(vgetq_lane_u32::<0>(t81[0]) as usize), t83[0]);
        t83[0] = vld1q_lane_u32::<1>(t82.add(vgetq_lane_u32::<1>(t81[0]) as usize), t83[0]);
        t83[0] = vld1q_lane_u32::<2>(t82.add(vgetq_lane_u32::<2>(t81[0]) as usize), t83[0]);
        t83[0] = vld1q_lane_u32::<3>(t82.add(vgetq_lane_u32::<3>(t81[0]) as usize), t83[0]);
        t83[1] = vld1q_lane_u32::<0>(t82.add(vgetq_lane_u32::<0>(t81[1]) as usize), t83[1]);
        t83[1] = vld1q_lane_u32::<1>(t82.add(vgetq_lane_u32::<1>(t81[1]) as usize), t83[1]);
        t83[1] = vld1q_lane_u32::<2>(t82.add(vgetq_lane_u32::<2>(t81[1]) as usize), t83[1]);
        t83[1] = vld1q_lane_u32::<3>(t82.add(vgetq_lane_u32::<3>(t81[1]) as usize), t83[1]);
        t77 = t83;
    }
    res = t77;
    return res;
}

#[inline(always)]
unsafe fn b1_ag4r0_f(l: &Launch, lid0: u32, arg0: *const u8) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t84 = (arg0 as *const u32).add((gid0) as usize);
    let t85: [uint32x4_t; 2] = [vld1q_u32(t84.add(0)), vld1q_u32(t84.add(4))];
    res = t85;
    return res;
}

#[inline(always)]
unsafe fn b2_ag4r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: [uint32x4_t; 2]) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let t86 = (arg0 as *mut u32).add((gid0) as usize);
    if all(&live) {
        vst1q_u32(t86.add(0), x8[0]);
        vst1q_u32(t86.add(4), x8[1]);
    } else {
        let mut t87w = 0u32;
        let t87 = core::hint::black_box(&raw mut t87w);
        core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t86.add(0), t87).write(vgetq_lane_u32::<0>(x8[0]));
        core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t86.add(1), t87).write(vgetq_lane_u32::<1>(x8[0]));
        core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t86.add(2), t87).write(vgetq_lane_u32::<2>(x8[0]));
        core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t86.add(3), t87).write(vgetq_lane_u32::<3>(x8[0]));
        core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t86.add(4), t87).write(vgetq_lane_u32::<0>(x8[1]));
        core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t86.add(5), t87).write(vgetq_lane_u32::<1>(x8[1]));
        core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t86.add(6), t87).write(vgetq_lane_u32::<2>(x8[1]));
        core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t86.add(7), t87).write(vgetq_lane_u32::<3>(x8[1]));
    }
}

#[inline(always)]
unsafe fn b2_ag4r0l_p_v0(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: [uint32x4_t; 2]) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let t88 = (arg0 as *mut u32).add((gid0) as usize);
    vst1q_u32(t88.add(0), vbslq_u32(live[0], x8[0], vld1q_u32(t88.add(0))));
    vst1q_u32(t88.add(4), vbslq_u32(live[1], x8[1], vld1q_u32(t88.add(4))));
}

#[inline(always)]
unsafe fn b3_o1l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg1_x0: *const u8, x8: [uint32x4_t; 2]) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t89: [uint32x4_t; 2] = [vbslq_u32(live[0], x8[0], vdupq_n_u32(0)), vbslq_u32(live[1], x8[1], vdupq_n_u32(0))];
    let t90 = vmaxvq_u32(vmaxq_u32(t89[0], t89[1]));
    if t90 != 0 { AtomicU32::from_ptr(arg1_x0 as *mut u32).fetch_max(t90, Relaxed); }
    return res;
}

#[inline(always)]
unsafe fn b3_o1l_f(l: &Launch, lid0: u32, arg1_x0: *const u8, x8: [uint32x4_t; 2]) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t91 = vmaxvq_u32(vmaxq_u32(x8[0], x8[1]));
    if t91 != 0 { AtomicU32::from_ptr(arg1_x0 as *mut u32).fetch_max(t91, Relaxed); }
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
    let x16: *const u8 = *argv.add(2);
    let arg3: *const u8 = (*argv.add(3) as *const *const u8).read_unaligned();
    let x40: u32 = (*argv.add(4) as *const u32).read_unaligned();
    let mut t92: bool = !(arg3).is_null();
    if t92 {
        let mut t93: bool = ((l.group[0]) >= ((arg3.wrapping_add((((x40) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        if !t93 {
            t93 = ((l.group[1]) >= ((arg3.wrapping_add(((((x40).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        let mut t94: bool = t93;
        if !t94 {
            t94 = ((l.group[2]) >= ((arg3.wrapping_add(((((x40).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        t92 = t94;
    }
    if t92 {
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
        let t95: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
        let mut live = t95;
        let gid0 = l.group[0] * l.block[0] + lid0;
        'chunk: {
            b0_r0r1sg_p(l, lid0, live, arg0, arg1, x16);
        }
    }
}
