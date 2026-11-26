//! `parent_step` at 8 lanes (2 NEON registers), written by rerail-ukl-vec's Rust (`rs.rs`) from its vector tape: do
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
unsafe fn b0_r0r1r2sg_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, arg2: *const u8, x3: *const u8) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x3 as *const u32).read_unaligned();
    let mut x5: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let t1: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
    let t2 = [vdupq_n_u32(a0); 2];
    let t3: [uint32x4_t; 2] = [vcgeq_u32(t1[0], t2[0]), vcgeq_u32(t1[1], t2[1])];
    let t4: [uint32x4_t; 2] = [vandq_u32(live[0], t3[0]), vandq_u32(live[1], t3[1])];
    {
        live = [vbicq_u32(live[0], t4[0]), vbicq_u32(live[1], t4[1])];
    }
    if any(&live) {
        let t5 = b1_r0r1g_p(l, lid0, live, arg0, arg1);
        x5 = t5;
        let t6: u64 = (1u64).wrapping_shl((32u64) as u32);
        let t7 = [vdupq_n_u32((t6) as u32); 2];
        let t8 = [vdupq_n_u32(((t6) >> 32) as u32); 2];
        let t9: [uint32x4_t; 4] = [vandq_u32(x5[0], t7[0]), vandq_u32(x5[1], t7[1]), vandq_u32(x5[2], t8[0]), vandq_u32(x5[3], t8[1])];
        let t10: u64 = 0u64;
        let t11 = [vdupq_n_u32((t10) as u32); 2];
        let t12: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t9[0], t11[0]), vceqq_u32(t9[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t9[1], t11[1]), vceqq_u32(t9[3], vdupq_n_u32(0)))];
        let t13: [uint32x4_t; 2] = [vmvnq_u32(t12[0]), vmvnq_u32(t12[1])];
        let mut t14: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t15: [uint32x4_t; 2] = [vandq_u32(live[0], t13[0]), vandq_u32(live[1], t13[1])];
        let t16: [uint32x4_t; 2] = [vbicq_u32(live[0], t13[0]), vbicq_u32(live[1], t13[1])];
        {
            let t17: [uint32x4_t; 2] = [x5[0], x5[1]];
            let t18 = [vdupq_n_u32(2147483648u32); 2];
            let t19: [uint32x4_t; 2] = [vorrq_u32(t18[0], t17[0]), vorrq_u32(t18[1], t17[1])];
            t14 = [vbslq_u32(t15[0], t19[0], t14[0]), vbslq_u32(t15[1], t19[1], t14[1])];
        }
        {
            let t20: [uint32x4_t; 2] = [x5[0], x5[1]];
            t14 = [vbslq_u32(t16[0], t20[0], t14[0]), vbslq_u32(t16[1], t20[1], t14[1])];
        }
        let t21 = (arg2 as *mut u32).add((gid0) as usize);
        if all(&live) {
            vst1q_u32(t21.add(0), t14[0]);
            vst1q_u32(t21.add(4), t14[1]);
        } else {
            let mut t22w = 0u32;
            let t22 = core::hint::black_box(&raw mut t22w);
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t21.add(0), t22).write(vgetq_lane_u32::<0>(t14[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t21.add(1), t22).write(vgetq_lane_u32::<1>(t14[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t21.add(2), t22).write(vgetq_lane_u32::<2>(t14[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t21.add(3), t22).write(vgetq_lane_u32::<3>(t14[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t21.add(4), t22).write(vgetq_lane_u32::<0>(t14[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t21.add(5), t22).write(vgetq_lane_u32::<1>(t14[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t21.add(6), t22).write(vgetq_lane_u32::<2>(t14[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t21.add(7), t22).write(vgetq_lane_u32::<3>(t14[1]));
        }
    }
}

#[inline(always)]
unsafe fn b0_r0r1r2sg_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, arg2: *const u8, x3: *const u8) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x3 as *const u32).read_unaligned();
    let mut x5: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    if ((gid0) as u64 + 7) < ((a0) as u64) {
        let t23 = b1_r0r1g_f(l, lid0, arg0, arg1);
        x5 = t23;
        let t24: u64 = (1u64).wrapping_shl((32u64) as u32);
        let t25 = [vdupq_n_u32((t24) as u32); 2];
        let t26 = [vdupq_n_u32(((t24) >> 32) as u32); 2];
        let t27: [uint32x4_t; 4] = [vandq_u32(x5[0], t25[0]), vandq_u32(x5[1], t25[1]), vandq_u32(x5[2], t26[0]), vandq_u32(x5[3], t26[1])];
        let t28: u64 = 0u64;
        let t29 = [vdupq_n_u32((t28) as u32); 2];
        let t30: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t27[0], t29[0]), vceqq_u32(t27[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t27[1], t29[1]), vceqq_u32(t27[3], vdupq_n_u32(0)))];
        let t31: [uint32x4_t; 2] = [vmvnq_u32(t30[0]), vmvnq_u32(t30[1])];
        let mut t32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t33: [uint32x4_t; 2] = [vmvnq_u32(t31[0]), vmvnq_u32(t31[1])];
        {
            let t34: [uint32x4_t; 2] = [x5[0], x5[1]];
            let t35 = [vdupq_n_u32(2147483648u32); 2];
            let t36: [uint32x4_t; 2] = [vorrq_u32(t35[0], t34[0]), vorrq_u32(t35[1], t34[1])];
            t32 = [vbslq_u32(t31[0], t36[0], t32[0]), vbslq_u32(t31[1], t36[1], t32[1])];
        }
        {
            let t37: [uint32x4_t; 2] = [x5[0], x5[1]];
            t32 = [vbslq_u32(t33[0], t37[0], t32[0]), vbslq_u32(t33[1], t37[1], t32[1])];
        }
        let t38 = (arg2 as *mut u32).add((gid0) as usize);
        vst1q_u32(t38.add(0), t32[0]);
        vst1q_u32(t38.add(4), t32[1]);
    } else if (((gid0) as u64 + 7) <= u32::MAX as u64 && ((gid0) as u64) >= ((a0) as u64)) {
        return;
    } else {
        // Lanes part: the chunk's lanes on both sides of the bound.
        let t39: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t40 = [vdupq_n_u32(a0); 2];
        let t41: [uint32x4_t; 2] = [vcgeq_u32(t39[0], t40[0]), vcgeq_u32(t39[1], t40[1])];
        let t42: [uint32x4_t; 2] = [vandq_u32(live[0], t41[0]), vandq_u32(live[1], t41[1])];
        {
            live = [vbicq_u32(live[0], t42[0]), vbicq_u32(live[1], t42[1])];
        }
        if any(&live) {
            let t43 = b1_r0r1g_p(l, lid0, live, arg0, arg1);
            x5 = t43;
            let t44: u64 = (1u64).wrapping_shl((32u64) as u32);
            let t45 = [vdupq_n_u32((t44) as u32); 2];
            let t46 = [vdupq_n_u32(((t44) >> 32) as u32); 2];
            let t47: [uint32x4_t; 4] = [vandq_u32(x5[0], t45[0]), vandq_u32(x5[1], t45[1]), vandq_u32(x5[2], t46[0]), vandq_u32(x5[3], t46[1])];
            let t48: u64 = 0u64;
            let t49 = [vdupq_n_u32((t48) as u32); 2];
            let t50: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t47[0], t49[0]), vceqq_u32(t47[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t47[1], t49[1]), vceqq_u32(t47[3], vdupq_n_u32(0)))];
            let t51: [uint32x4_t; 2] = [vmvnq_u32(t50[0]), vmvnq_u32(t50[1])];
            let mut t52: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t53: [uint32x4_t; 2] = [vandq_u32(live[0], t51[0]), vandq_u32(live[1], t51[1])];
            let t54: [uint32x4_t; 2] = [vbicq_u32(live[0], t51[0]), vbicq_u32(live[1], t51[1])];
            {
                let t55: [uint32x4_t; 2] = [x5[0], x5[1]];
                let t56 = [vdupq_n_u32(2147483648u32); 2];
                let t57: [uint32x4_t; 2] = [vorrq_u32(t56[0], t55[0]), vorrq_u32(t56[1], t55[1])];
                t52 = [vbslq_u32(t53[0], t57[0], t52[0]), vbslq_u32(t53[1], t57[1], t52[1])];
            }
            {
                let t58: [uint32x4_t; 2] = [x5[0], x5[1]];
                t52 = [vbslq_u32(t54[0], t58[0], t52[0]), vbslq_u32(t54[1], t58[1], t52[1])];
            }
            let t59 = (arg2 as *mut u32).add((gid0) as usize);
            if all(&live) {
                vst1q_u32(t59.add(0), t52[0]);
                vst1q_u32(t59.add(4), t52[1]);
            } else {
                let mut t60w = 0u32;
                let t60 = core::hint::black_box(&raw mut t60w);
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t59.add(0), t60).write(vgetq_lane_u32::<0>(t52[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t59.add(1), t60).write(vgetq_lane_u32::<1>(t52[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t59.add(2), t60).write(vgetq_lane_u32::<2>(t52[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t59.add(3), t60).write(vgetq_lane_u32::<3>(t52[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t59.add(4), t60).write(vgetq_lane_u32::<0>(t52[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t59.add(5), t60).write(vgetq_lane_u32::<1>(t52[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t59.add(6), t60).write(vgetq_lane_u32::<2>(t52[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t59.add(7), t60).write(vgetq_lane_u32::<3>(t52[1]));
            }
        }
    }
}

#[inline(always)]
unsafe fn b1_r0r1g_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x3: u32 = 0;
    let mut x4: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x5: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x6: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t61: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
    let t62 = [vdupq_n_u32(0u32); 2];
    let t63: [uint32x4_t; 2] = [vceqq_u32(t61[0], t62[0]), vceqq_u32(t61[1], t62[1])];
    let t64: [uint32x4_t; 2] = [vandq_u32(live[0], t63[0]), vandq_u32(live[1], t63[1])];
    {
        let t65: u64 = 4294967295u64;
        let t66 = [vdupq_n_u32((t65) as u32); 2];
        let t67: [uint32x4_t; 4] = [t66[0], t66[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = [vbslq_u32(t64[0], t67[0], res[0]), vbslq_u32(t64[1], t67[1], res[1]), vbslq_u32(t64[0], t67[2], res[2]), vbslq_u32(t64[1], t67[3], res[3])];
        live = [vbicq_u32(live[0], t64[0]), vbicq_u32(live[1], t64[1])];
    }
    if any(&live) {
        x3 = gid0.wrapping_sub(1u32);
        let t68 = if (x3 & 255u32) <= 248u32 { b2_r0r1u8_p(l, lid0, live, arg0, arg1, x3) } else { b2_r0r1i_p(l, lid0, live, arg0, arg1, x3) };
        x4 = t68;
        let t69: u64 = 1u64;
        let t70 = [vdupq_n_u32((t69) as u32); 2];
        let t71: [uint32x4_t; 2] = [vandq_u32(x4[0], t70[0]), vandq_u32(x4[1], t70[1])];
        let t72: u64 = 0u64;
        let t73 = [vdupq_n_u32((t72) as u32); 2];
        let t74: [uint32x4_t; 2] = [vceqq_u32(t71[0], t73[0]), vceqq_u32(t71[1], t73[1])];
        let t75: [uint32x4_t; 2] = [vandq_u32(live[0], t74[0]), vandq_u32(live[1], t74[1])];
        {
            let t76: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[1].as_ptr()))];
            let t77: [uint32x4_t; 4] = [t76[0], t76[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t75[0], t77[0], res[0]), vbslq_u32(t75[1], t77[1], res[1]), vbslq_u32(t75[0], t77[2], res[2]), vbslq_u32(t75[1], t77[3], res[3])];
            live = [vbicq_u32(live[0], t75[0]), vbicq_u32(live[1], t75[1])];
        }
        if any(&live) {
            let t78: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x4[0]), vshlq_n_u32::<30>(x4[2])), vorrq_u32(vshrq_n_u32::<2>(x4[1]), vshlq_n_u32::<30>(x4[3])), vshrq_n_u32::<2>(x4[2]), vshrq_n_u32::<2>(x4[3])];
            let t79: [uint32x4_t; 2] = [t78[0], t78[1]];
            x5 = t79;
            let t80 = [vdupq_n_u32(1u32); 2];
            let t81: [uint32x4_t; 2] = [vsubq_u32(x5[0], t80[0]), vsubq_u32(x5[1], t80[1])];
            let t82: [uint32x4_t; 2] = [vcgeq_u32(t81[0], t61[0]), vcgeq_u32(t81[1], t61[1])];
            let t83: [uint32x4_t; 2] = [vandq_u32(live[0], t82[0]), vandq_u32(live[1], t82[1])];
            {
                let t84: u64 = 4294967295u64;
                let t85 = [vdupq_n_u32((t84) as u32); 2];
                let t86: [uint32x4_t; 4] = [t85[0], t85[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t83[0], t86[0], res[0]), vbslq_u32(t83[1], t86[1], res[1]), vbslq_u32(t83[0], t86[2], res[2]), vbslq_u32(t83[1], t86[3], res[3])];
                live = [vbicq_u32(live[0], t83[0]), vbicq_u32(live[1], t83[1])];
            }
            if any(&live) {
                let t87: [uint32x4_t; 2] = [vsubq_u32(t61[0], x5[0]), vsubq_u32(t61[1], x5[1])];
                x6 = t87;
                let t88: u64 = 2u64;
                let t89 = [vdupq_n_u32((t88) as u32); 2];
                let t90: [uint32x4_t; 2] = [vandq_u32(x4[0], t89[0]), vandq_u32(x4[1], t89[1])];
                let t91: u64 = 0u64;
                let t92 = [vdupq_n_u32((t91) as u32); 2];
                let t93: [uint32x4_t; 2] = [vceqq_u32(t90[0], t92[0]), vceqq_u32(t90[1], t92[1])];
                let t94: [uint32x4_t; 2] = [vmvnq_u32(t93[0]), vmvnq_u32(t93[1])];
                let t95: [uint32x4_t; 2] = [vandq_u32(live[0], t94[0]), vandq_u32(live[1], t94[1])];
                {
                    let t96: [uint32x4_t; 4] = [x6[0], x6[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                    res = [vbslq_u32(t95[0], t96[0], res[0]), vbslq_u32(t95[1], t96[1], res[1]), vbslq_u32(t95[0], t96[2], res[2]), vbslq_u32(t95[1], t96[3], res[3])];
                    live = [vbicq_u32(live[0], t95[0]), vbicq_u32(live[1], t95[1])];
                }
                if any(&live) {
                    let t97: [uint32x4_t; 2] = [vceqq_u32(x6[0], t62[0]), vceqq_u32(x6[1], t62[1])];
                    let t98: [uint32x4_t; 2] = [vandq_u32(live[0], t97[0]), vandq_u32(live[1], t97[1])];
                    {
                        let t99: u64 = 4294967295u64;
                        let t100 = [vdupq_n_u32((t99) as u32); 2];
                        let t101: [uint32x4_t; 4] = [t100[0], t100[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                        res = [vbslq_u32(t98[0], t101[0], res[0]), vbslq_u32(t98[1], t101[1], res[1]), vbslq_u32(t98[0], t101[2], res[2]), vbslq_u32(t98[1], t101[3], res[3])];
                        live = [vbicq_u32(live[0], t98[0]), vbicq_u32(live[1], t98[1])];
                    }
                    if any(&live) {
                        let t102: u64 = (1u64).wrapping_shl((32u64) as u32);
                        let t103 = [vdupq_n_u32((t102) as u32); 2];
                        let t104 = [vdupq_n_u32(((t102) >> 32) as u32); 2];
                        let t105: [uint32x4_t; 4] = [vorrq_u32(t103[0], x6[0]), vorrq_u32(t103[1], x6[1]), t104[0], t104[1]];
                        res = [vbslq_u32(live[0], t105[0], res[0]), vbslq_u32(live[1], t105[1], res[1]), vbslq_u32(live[0], t105[2], res[2]), vbslq_u32(live[1], t105[3], res[3])];
                        live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
                    }
                }
            }
        }
    }
    res
}

#[inline(always)]
unsafe fn b1_r0r1g_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x3: u32 = 0;
    let mut x4: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x5: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x6: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if (((gid0) as u64 + 7) <= u32::MAX as u64 && (((0u32) as u64) < ((gid0) as u64) || ((0u32) as u64) > ((gid0) as u64 + 7))) {
        x3 = gid0.wrapping_sub(1u32);
        let t106 = if (x3 & 255u32) <= 248u32 { b2_r0r1u8_f(l, lid0, arg0, arg1, x3) } else { b2_r0r1i_f(l, lid0, arg0, arg1, x3) };
        x4 = t106;
        let t107: u64 = 1u64;
        let t108 = [vdupq_n_u32((t107) as u32); 2];
        let t109: [uint32x4_t; 2] = [vandq_u32(x4[0], t108[0]), vandq_u32(x4[1], t108[1])];
        let t110: u64 = 0u64;
        let t111 = [vdupq_n_u32((t110) as u32); 2];
        let t112: [uint32x4_t; 2] = [vceqq_u32(t109[0], t111[0]), vceqq_u32(t109[1], t111[1])];
        {
            let t113: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[1].as_ptr()))];
            let t114: [uint32x4_t; 4] = [t113[0], t113[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t112[0], t114[0], res[0]), vbslq_u32(t112[1], t114[1], res[1]), vbslq_u32(t112[0], t114[2], res[2]), vbslq_u32(t112[1], t114[3], res[3])];
            live = [vbicq_u32(live[0], t112[0]), vbicq_u32(live[1], t112[1])];
        }
        if any(&live) {
            let t115: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x4[0]), vshlq_n_u32::<30>(x4[2])), vorrq_u32(vshrq_n_u32::<2>(x4[1]), vshlq_n_u32::<30>(x4[3])), vshrq_n_u32::<2>(x4[2]), vshrq_n_u32::<2>(x4[3])];
            let t116: [uint32x4_t; 2] = [t115[0], t115[1]];
            x5 = t116;
            let t117 = [vdupq_n_u32(1u32); 2];
            let t118: [uint32x4_t; 2] = [vsubq_u32(x5[0], t117[0]), vsubq_u32(x5[1], t117[1])];
            let t119: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
            let t120: [uint32x4_t; 2] = [vcgeq_u32(t118[0], t119[0]), vcgeq_u32(t118[1], t119[1])];
            let t121: [uint32x4_t; 2] = [vandq_u32(live[0], t120[0]), vandq_u32(live[1], t120[1])];
            {
                let t122: u64 = 4294967295u64;
                let t123 = [vdupq_n_u32((t122) as u32); 2];
                let t124: [uint32x4_t; 4] = [t123[0], t123[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t121[0], t124[0], res[0]), vbslq_u32(t121[1], t124[1], res[1]), vbslq_u32(t121[0], t124[2], res[2]), vbslq_u32(t121[1], t124[3], res[3])];
                live = [vbicq_u32(live[0], t121[0]), vbicq_u32(live[1], t121[1])];
            }
            if any(&live) {
                let t125: [uint32x4_t; 2] = [vsubq_u32(t119[0], x5[0]), vsubq_u32(t119[1], x5[1])];
                x6 = t125;
                let t126: u64 = 2u64;
                let t127 = [vdupq_n_u32((t126) as u32); 2];
                let t128: [uint32x4_t; 2] = [vandq_u32(x4[0], t127[0]), vandq_u32(x4[1], t127[1])];
                let t129: u64 = 0u64;
                let t130 = [vdupq_n_u32((t129) as u32); 2];
                let t131: [uint32x4_t; 2] = [vceqq_u32(t128[0], t130[0]), vceqq_u32(t128[1], t130[1])];
                let t132: [uint32x4_t; 2] = [vmvnq_u32(t131[0]), vmvnq_u32(t131[1])];
                let t133: [uint32x4_t; 2] = [vandq_u32(live[0], t132[0]), vandq_u32(live[1], t132[1])];
                {
                    let t134: [uint32x4_t; 4] = [x6[0], x6[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                    res = [vbslq_u32(t133[0], t134[0], res[0]), vbslq_u32(t133[1], t134[1], res[1]), vbslq_u32(t133[0], t134[2], res[2]), vbslq_u32(t133[1], t134[3], res[3])];
                    live = [vbicq_u32(live[0], t133[0]), vbicq_u32(live[1], t133[1])];
                }
                if any(&live) {
                    let t135 = [vdupq_n_u32(0u32); 2];
                    let t136: [uint32x4_t; 2] = [vceqq_u32(x6[0], t135[0]), vceqq_u32(x6[1], t135[1])];
                    let t137: [uint32x4_t; 2] = [vandq_u32(live[0], t136[0]), vandq_u32(live[1], t136[1])];
                    {
                        let t138: u64 = 4294967295u64;
                        let t139 = [vdupq_n_u32((t138) as u32); 2];
                        let t140: [uint32x4_t; 4] = [t139[0], t139[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                        res = [vbslq_u32(t137[0], t140[0], res[0]), vbslq_u32(t137[1], t140[1], res[1]), vbslq_u32(t137[0], t140[2], res[2]), vbslq_u32(t137[1], t140[3], res[3])];
                        live = [vbicq_u32(live[0], t137[0]), vbicq_u32(live[1], t137[1])];
                    }
                    if any(&live) {
                        let t141: u64 = (1u64).wrapping_shl((32u64) as u32);
                        let t142 = [vdupq_n_u32((t141) as u32); 2];
                        let t143 = [vdupq_n_u32(((t141) >> 32) as u32); 2];
                        let t144: [uint32x4_t; 4] = [vorrq_u32(t142[0], x6[0]), vorrq_u32(t142[1], x6[1]), t143[0], t143[1]];
                        res = [vbslq_u32(live[0], t144[0], res[0]), vbslq_u32(live[1], t144[1], res[1]), vbslq_u32(live[0], t144[2], res[2]), vbslq_u32(live[1], t144[3], res[3])];
                        live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
                    }
                }
            }
        }
    } else if false {
        let t145: u64 = 4294967295u64;
        let t146 = [vdupq_n_u32((t145) as u32); 2];
        let t147: [uint32x4_t; 4] = [t146[0], t146[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t147;
        return res;
    } else {
        // Lanes part: the chunk's lanes on both sides of the bound.
        let t148: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t149 = [vdupq_n_u32(0u32); 2];
        let t150: [uint32x4_t; 2] = [vceqq_u32(t148[0], t149[0]), vceqq_u32(t148[1], t149[1])];
        let t151: [uint32x4_t; 2] = [vandq_u32(live[0], t150[0]), vandq_u32(live[1], t150[1])];
        {
            let t152: u64 = 4294967295u64;
            let t153 = [vdupq_n_u32((t152) as u32); 2];
            let t154: [uint32x4_t; 4] = [t153[0], t153[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t151[0], t154[0], res[0]), vbslq_u32(t151[1], t154[1], res[1]), vbslq_u32(t151[0], t154[2], res[2]), vbslq_u32(t151[1], t154[3], res[3])];
            live = [vbicq_u32(live[0], t151[0]), vbicq_u32(live[1], t151[1])];
        }
        if any(&live) {
            x3 = gid0.wrapping_sub(1u32);
            let t155 = if (x3 & 255u32) <= 248u32 { b2_r0r1u8_p(l, lid0, live, arg0, arg1, x3) } else { b2_r0r1i_p(l, lid0, live, arg0, arg1, x3) };
            x4 = t155;
            let t156: u64 = 1u64;
            let t157 = [vdupq_n_u32((t156) as u32); 2];
            let t158: [uint32x4_t; 2] = [vandq_u32(x4[0], t157[0]), vandq_u32(x4[1], t157[1])];
            let t159: u64 = 0u64;
            let t160 = [vdupq_n_u32((t159) as u32); 2];
            let t161: [uint32x4_t; 2] = [vceqq_u32(t158[0], t160[0]), vceqq_u32(t158[1], t160[1])];
            let t162: [uint32x4_t; 2] = [vandq_u32(live[0], t161[0]), vandq_u32(live[1], t161[1])];
            {
                let t163: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[1].as_ptr()))];
                let t164: [uint32x4_t; 4] = [t163[0], t163[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t162[0], t164[0], res[0]), vbslq_u32(t162[1], t164[1], res[1]), vbslq_u32(t162[0], t164[2], res[2]), vbslq_u32(t162[1], t164[3], res[3])];
                live = [vbicq_u32(live[0], t162[0]), vbicq_u32(live[1], t162[1])];
            }
            if any(&live) {
                let t165: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x4[0]), vshlq_n_u32::<30>(x4[2])), vorrq_u32(vshrq_n_u32::<2>(x4[1]), vshlq_n_u32::<30>(x4[3])), vshrq_n_u32::<2>(x4[2]), vshrq_n_u32::<2>(x4[3])];
                let t166: [uint32x4_t; 2] = [t165[0], t165[1]];
                x5 = t166;
                let t167 = [vdupq_n_u32(1u32); 2];
                let t168: [uint32x4_t; 2] = [vsubq_u32(x5[0], t167[0]), vsubq_u32(x5[1], t167[1])];
                let t169: [uint32x4_t; 2] = [vcgeq_u32(t168[0], t148[0]), vcgeq_u32(t168[1], t148[1])];
                let t170: [uint32x4_t; 2] = [vandq_u32(live[0], t169[0]), vandq_u32(live[1], t169[1])];
                {
                    let t171: u64 = 4294967295u64;
                    let t172 = [vdupq_n_u32((t171) as u32); 2];
                    let t173: [uint32x4_t; 4] = [t172[0], t172[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                    res = [vbslq_u32(t170[0], t173[0], res[0]), vbslq_u32(t170[1], t173[1], res[1]), vbslq_u32(t170[0], t173[2], res[2]), vbslq_u32(t170[1], t173[3], res[3])];
                    live = [vbicq_u32(live[0], t170[0]), vbicq_u32(live[1], t170[1])];
                }
                if any(&live) {
                    let t174: [uint32x4_t; 2] = [vsubq_u32(t148[0], x5[0]), vsubq_u32(t148[1], x5[1])];
                    x6 = t174;
                    let t175: u64 = 2u64;
                    let t176 = [vdupq_n_u32((t175) as u32); 2];
                    let t177: [uint32x4_t; 2] = [vandq_u32(x4[0], t176[0]), vandq_u32(x4[1], t176[1])];
                    let t178: u64 = 0u64;
                    let t179 = [vdupq_n_u32((t178) as u32); 2];
                    let t180: [uint32x4_t; 2] = [vceqq_u32(t177[0], t179[0]), vceqq_u32(t177[1], t179[1])];
                    let t181: [uint32x4_t; 2] = [vmvnq_u32(t180[0]), vmvnq_u32(t180[1])];
                    let t182: [uint32x4_t; 2] = [vandq_u32(live[0], t181[0]), vandq_u32(live[1], t181[1])];
                    {
                        let t183: [uint32x4_t; 4] = [x6[0], x6[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                        res = [vbslq_u32(t182[0], t183[0], res[0]), vbslq_u32(t182[1], t183[1], res[1]), vbslq_u32(t182[0], t183[2], res[2]), vbslq_u32(t182[1], t183[3], res[3])];
                        live = [vbicq_u32(live[0], t182[0]), vbicq_u32(live[1], t182[1])];
                    }
                    if any(&live) {
                        let t184: [uint32x4_t; 2] = [vceqq_u32(x6[0], t149[0]), vceqq_u32(x6[1], t149[1])];
                        let t185: [uint32x4_t; 2] = [vandq_u32(live[0], t184[0]), vandq_u32(live[1], t184[1])];
                        {
                            let t186: u64 = 4294967295u64;
                            let t187 = [vdupq_n_u32((t186) as u32); 2];
                            let t188: [uint32x4_t; 4] = [t187[0], t187[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                            res = [vbslq_u32(t185[0], t188[0], res[0]), vbslq_u32(t185[1], t188[1], res[1]), vbslq_u32(t185[0], t188[2], res[2]), vbslq_u32(t185[1], t188[3], res[3])];
                            live = [vbicq_u32(live[0], t185[0]), vbicq_u32(live[1], t185[1])];
                        }
                        if any(&live) {
                            let t189: u64 = (1u64).wrapping_shl((32u64) as u32);
                            let t190 = [vdupq_n_u32((t189) as u32); 2];
                            let t191 = [vdupq_n_u32(((t189) >> 32) as u32); 2];
                            let t192: [uint32x4_t; 4] = [vorrq_u32(t190[0], x6[0]), vorrq_u32(t190[1], x6[1]), t191[0], t191[1]];
                            res = [vbslq_u32(live[0], t192[0], res[0]), vbslq_u32(live[1], t192[1], res[1]), vbslq_u32(live[0], t192[2], res[2]), vbslq_u32(live[1], t192[3], res[3])];
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
unsafe fn b2_r0r1i_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t193 = b3_r0r1si_p(l, lid0, live, arg0, arg1, 0u32, x2);
    res = t193;
    return res;
}

#[inline(always)]
unsafe fn b2_r0r1i_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t194 = b3_r0r1si_f(l, lid0, arg0, arg1, 0u32, x2);
    res = t194;
    return res;
}

#[inline(always)]
unsafe fn b2_r0r1u8_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t195 = b3_r0r1su8_p(l, lid0, live, arg0, arg1, 0u32, x2);
    res = t195;
    return res;
}

#[inline(always)]
unsafe fn b2_r0r1u8_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t196 = b3_r0r1su8_f(l, lid0, arg0, arg1, 0u32, x2);
    res = t196;
    return res;
}

#[inline(always)]
unsafe fn b3_r0r1si_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u32 = 0;
    let mut x5: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x6: u64 = 0;
    let mut x7: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t197 = b4_r0r1s_p(l, lid0, live, arg0, arg1, x2);
    x4 = t197;
    let t198: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[1].as_ptr()))];
    let t199: [uint32x4_t; 2] = [vshlq_u32(t198[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(t198[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t200 = b5_r0r1sl_p(l, lid0, live, arg0, arg1, (1u32).wrapping_add(x2), t199);
    x5 = t200;
    let t201 = b6_s_p(l, lid0, live, x4);
    x6 = t201;
    let t202 = [vdupq_n_u32(1023u32); 2];
    let t203: [uint32x4_t; 2] = [vandq_u32(t198[0], t202[0]), vandq_u32(t198[1], t202[1])];
    let t204 = [vdupq_n_u32(x4); 2];
    let t205: [uint32x4_t; 2] = [vmulq_u32(t203[0], t204[0]), vmulq_u32(t203[1], t204[1])];
    let t206 = b7_r0r1lls_p(l, lid0, live, arg0, arg1, x5, t205, 8u32);
    let t207: u64 = x6;
    let t208 = [vdupq_n_u32((t207) as u32); 2];
    let t209 = [vdupq_n_u32(((t207) >> 32) as u32); 2];
    let t210: [uint32x4_t; 4] = [vandq_u32(t206[0], t208[0]), vandq_u32(t206[1], t208[1]), vandq_u32(t206[2], t209[0]), vandq_u32(t206[3], t209[1])];
    x7 = t210;
    let t211: u64 = x6;
    let t212 = [vdupq_n_u32((t211) as u32); 2];
    let t213 = [vdupq_n_u32(((t211) >> 32) as u32); 2];
    let t214: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x7[0], t212[0]), vceqq_u32(x7[2], t213[0])), vandq_u32(vceqq_u32(x7[1], t212[1]), vceqq_u32(x7[3], t213[1]))];
    let mut t215: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t216: [uint32x4_t; 2] = [vandq_u32(live[0], t214[0]), vandq_u32(live[1], t214[1])];
    let t217: [uint32x4_t; 2] = [vbicq_u32(live[0], t214[0]), vbicq_u32(live[1], t214[1])];
    if any(&t216) {
        let t218: [uint32x4_t; 4];
        let t219 = bits(&t216);
        if t219.count_ones() <= 2 {
            let mut t220 = [0u32; 16];
            let mut t221 = t219;
            while t221 != 0 {
                let j = t221.trailing_zeros() as usize;
                t221 &= t221 - 1;
                let t222 = b8_r0r1ss_f(l, lid0, arg0, arg1, (x2).wrapping_add(1u32), (x3).wrapping_add(j as u32));
                let t223: u64 = t222;
                t220[j] = t223 as u32;
                t220[8 + j] = (t223 >> 32) as u32;
            }
            t218 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t220);
        } else {
            let t224 = b8_r0r1si_p(l, lid0, t216, arg0, arg1, (x2).wrapping_add(1u32), x3);
            t218 = t224;
        }
        t215 = [vbslq_u32(t216[0], t218[0], t215[0]), vbslq_u32(t216[1], t218[1], t215[1]), vbslq_u32(t216[0], t218[2], t215[2]), vbslq_u32(t216[1], t218[3], t215[3])];
    }
    {
        t215 = [vbslq_u32(t217[0], x7[0], t215[0]), vbslq_u32(t217[1], x7[1], t215[1]), vbslq_u32(t217[0], x7[2], t215[2]), vbslq_u32(t217[1], x7[3], t215[3])];
    }
    res = t215;
    return res;
}

#[inline(always)]
unsafe fn b3_r0r1si_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32, x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u32 = 0;
    let mut x5: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x6: u64 = 0;
    let mut x7: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t225 = b4_r0r1s_f(l, lid0, arg0, arg1, x2);
    x4 = t225;
    let t226: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[1].as_ptr()))];
    let t227: [uint32x4_t; 2] = [vshlq_u32(t226[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(t226[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t228 = b5_r0r1sl_f(l, lid0, arg0, arg1, (1u32).wrapping_add(x2), t227);
    x5 = t228;
    let t229 = b6_s_f(l, lid0, x4);
    x6 = t229;
    let t230 = [vdupq_n_u32(1023u32); 2];
    let t231: [uint32x4_t; 2] = [vandq_u32(t226[0], t230[0]), vandq_u32(t226[1], t230[1])];
    let t232 = [vdupq_n_u32(x4); 2];
    let t233: [uint32x4_t; 2] = [vmulq_u32(t231[0], t232[0]), vmulq_u32(t231[1], t232[1])];
    let t234 = b7_r0r1lls_f(l, lid0, arg0, arg1, x5, t233, 8u32);
    let t235: u64 = x6;
    let t236 = [vdupq_n_u32((t235) as u32); 2];
    let t237 = [vdupq_n_u32(((t235) >> 32) as u32); 2];
    let t238: [uint32x4_t; 4] = [vandq_u32(t234[0], t236[0]), vandq_u32(t234[1], t236[1]), vandq_u32(t234[2], t237[0]), vandq_u32(t234[3], t237[1])];
    x7 = t238;
    let t239: u64 = x6;
    let t240 = [vdupq_n_u32((t239) as u32); 2];
    let t241 = [vdupq_n_u32(((t239) >> 32) as u32); 2];
    let t242: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x7[0], t240[0]), vceqq_u32(x7[2], t241[0])), vandq_u32(vceqq_u32(x7[1], t240[1]), vceqq_u32(x7[3], t241[1]))];
    let mut t243: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t244: [uint32x4_t; 2] = [vmvnq_u32(t242[0]), vmvnq_u32(t242[1])];
    if any(&t242) {
        let t245: [uint32x4_t; 4];
        let t246 = bits(&t242);
        if t246.count_ones() <= 2 {
            let mut t247 = [0u32; 16];
            let mut t248 = t246;
            while t248 != 0 {
                let j = t248.trailing_zeros() as usize;
                t248 &= t248 - 1;
                let t249 = b8_r0r1ss_f(l, lid0, arg0, arg1, (x2).wrapping_add(1u32), (x3).wrapping_add(j as u32));
                let t250: u64 = t249;
                t247[j] = t250 as u32;
                t247[8 + j] = (t250 >> 32) as u32;
            }
            t245 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t247);
        } else {
            let t251 = b8_r0r1si_p(l, lid0, t242, arg0, arg1, (x2).wrapping_add(1u32), x3);
            t245 = t251;
        }
        t243 = [vbslq_u32(t242[0], t245[0], t243[0]), vbslq_u32(t242[1], t245[1], t243[1]), vbslq_u32(t242[0], t245[2], t243[2]), vbslq_u32(t242[1], t245[3], t243[3])];
    }
    {
        t243 = [vbslq_u32(t244[0], x7[0], t243[0]), vbslq_u32(t244[1], x7[1], t243[1]), vbslq_u32(t244[0], x7[2], t243[2]), vbslq_u32(t244[1], x7[3], t243[3])];
    }
    res = t243;
    return res;
}

#[inline(always)]
unsafe fn b3_r0r1su8_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u32 = 0;
    let mut x5: u32 = 0;
    let mut x6: u64 = 0;
    let mut x7: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t252 = b4_r0r1s_p(l, lid0, live, arg0, arg1, x2);
    x4 = t252;
    let t253 = b5_r0r1ss_p(l, lid0, live, arg0, arg1, (1u32).wrapping_add(x2), (x3 >> 10));
    x5 = t253;
    let t254 = b6_s_p(l, lid0, live, x4);
    x6 = t254;
    let t255 = b7_r0r1sns_p(l, lid0, live, arg0, arg1, x5, (x3 & 1023u32).wrapping_mul(x4), x4, 8u32);
    let t256: u64 = x6;
    let t257 = [vdupq_n_u32((t256) as u32); 2];
    let t258 = [vdupq_n_u32(((t256) >> 32) as u32); 2];
    let t259: [uint32x4_t; 4] = [vandq_u32(t255[0], t257[0]), vandq_u32(t255[1], t257[1]), vandq_u32(t255[2], t258[0]), vandq_u32(t255[3], t258[1])];
    x7 = t259;
    let t260: u64 = x6;
    let t261 = [vdupq_n_u32((t260) as u32); 2];
    let t262 = [vdupq_n_u32(((t260) >> 32) as u32); 2];
    let t263: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x7[0], t261[0]), vceqq_u32(x7[2], t262[0])), vandq_u32(vceqq_u32(x7[1], t261[1]), vceqq_u32(x7[3], t262[1]))];
    let mut t264: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t265: [uint32x4_t; 2] = [vandq_u32(live[0], t263[0]), vandq_u32(live[1], t263[1])];
    let t266: [uint32x4_t; 2] = [vbicq_u32(live[0], t263[0]), vbicq_u32(live[1], t263[1])];
    if any(&t265) {
        let t267: [uint32x4_t; 4];
        let t268 = bits(&t265);
        if t268.count_ones() <= 2 {
            let mut t269 = [0u32; 16];
            let mut t270 = t268;
            while t270 != 0 {
                let j = t270.trailing_zeros() as usize;
                t270 &= t270 - 1;
                let t271 = b8_r0r1ss_f(l, lid0, arg0, arg1, (x2).wrapping_add(1u32), (x3).wrapping_add(j as u32));
                let t272: u64 = t271;
                t269[j] = t272 as u32;
                t269[8 + j] = (t272 >> 32) as u32;
            }
            t267 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t269);
        } else {
            let t273 = b8_r0r1su8_p(l, lid0, t265, arg0, arg1, (x2).wrapping_add(1u32), x3);
            t267 = t273;
        }
        t264 = [vbslq_u32(t265[0], t267[0], t264[0]), vbslq_u32(t265[1], t267[1], t264[1]), vbslq_u32(t265[0], t267[2], t264[2]), vbslq_u32(t265[1], t267[3], t264[3])];
    }
    {
        t264 = [vbslq_u32(t266[0], x7[0], t264[0]), vbslq_u32(t266[1], x7[1], t264[1]), vbslq_u32(t266[0], x7[2], t264[2]), vbslq_u32(t266[1], x7[3], t264[3])];
    }
    res = t264;
    return res;
}

#[inline(always)]
unsafe fn b3_r0r1su8_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32, x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u32 = 0;
    let mut x5: u32 = 0;
    let mut x6: u64 = 0;
    let mut x7: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t274 = b4_r0r1s_f(l, lid0, arg0, arg1, x2);
    x4 = t274;
    let t275 = b5_r0r1ss_f(l, lid0, arg0, arg1, (1u32).wrapping_add(x2), (x3 >> 10));
    x5 = t275;
    let t276 = b6_s_f(l, lid0, x4);
    x6 = t276;
    let t277 = b7_r0r1sns_f(l, lid0, arg0, arg1, x5, (x3 & 1023u32).wrapping_mul(x4), x4, 8u32);
    let t278: u64 = x6;
    let t279 = [vdupq_n_u32((t278) as u32); 2];
    let t280 = [vdupq_n_u32(((t278) >> 32) as u32); 2];
    let t281: [uint32x4_t; 4] = [vandq_u32(t277[0], t279[0]), vandq_u32(t277[1], t279[1]), vandq_u32(t277[2], t280[0]), vandq_u32(t277[3], t280[1])];
    x7 = t281;
    let t282: u64 = x6;
    let t283 = [vdupq_n_u32((t282) as u32); 2];
    let t284 = [vdupq_n_u32(((t282) >> 32) as u32); 2];
    let t285: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x7[0], t283[0]), vceqq_u32(x7[2], t284[0])), vandq_u32(vceqq_u32(x7[1], t283[1]), vceqq_u32(x7[3], t284[1]))];
    let mut t286: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t287: [uint32x4_t; 2] = [vmvnq_u32(t285[0]), vmvnq_u32(t285[1])];
    if any(&t285) {
        let t288: [uint32x4_t; 4];
        let t289 = bits(&t285);
        if t289.count_ones() <= 2 {
            let mut t290 = [0u32; 16];
            let mut t291 = t289;
            while t291 != 0 {
                let j = t291.trailing_zeros() as usize;
                t291 &= t291 - 1;
                let t292 = b8_r0r1ss_f(l, lid0, arg0, arg1, (x2).wrapping_add(1u32), (x3).wrapping_add(j as u32));
                let t293: u64 = t292;
                t290[j] = t293 as u32;
                t290[8 + j] = (t293 >> 32) as u32;
            }
            t288 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t290);
        } else {
            let t294 = b8_r0r1su8_p(l, lid0, t285, arg0, arg1, (x2).wrapping_add(1u32), x3);
            t288 = t294;
        }
        t286 = [vbslq_u32(t285[0], t288[0], t286[0]), vbslq_u32(t285[1], t288[1], t286[1]), vbslq_u32(t285[0], t288[2], t286[2]), vbslq_u32(t285[1], t288[3], t286[3])];
    }
    {
        t286 = [vbslq_u32(t287[0], x7[0], t286[0]), vbslq_u32(t287[1], x7[1], t286[1]), vbslq_u32(t287[0], x7[2], t286[2]), vbslq_u32(t287[1], x7[3], t286[3])];
    }
    res = t286;
    return res;
}

#[inline(always)]
unsafe fn b4_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t295 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    res = (((t295).wrapping_shr(((4u32).wrapping_mul(x2)) as u32)) & (15u32));
    return res;
}

#[inline(always)]
unsafe fn b4_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t296 = b9_r0r1s_f(l, lid0, arg0, arg1, 16u32);
    res = (((t296).wrapping_shr(((4u32).wrapping_mul(x2)) as u32)) & (15u32));
    return res;
}

#[inline(always)]
unsafe fn b5_r0r1ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u32 = 0;
    let mut x5: u32 = 0;
    let mut live = k;
    let mut res: u32 = 0;
    let t297 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 40u32);
    let t298 = b10_r0r1s_p(l, lid0, live, arg0, arg1, 0u32);
    x4 = ((x3).wrapping_mul(t297)).wrapping_add(t298);
    if ((x2) == (0u32)) {
        res = x4;
        return res;
    }
    let t299 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    let t300 = b11_ss_p(l, lid0, live, (t299).wrapping_sub((x3).wrapping_shl((10u32) as u32)), 1024u32);
    x5 = t300;
    let t301 = b10_r0r1s_p(l, lid0, live, arg0, arg1, x2);
    res = (x4).wrapping_add((x5).wrapping_mul(t301));
    return res;
}

#[inline(always)]
unsafe fn b5_r0r1ss_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32, x3: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u32 = 0;
    let mut x5: u32 = 0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t302 = b9_r0r1s_f(l, lid0, arg0, arg1, 40u32);
    let t303 = b10_r0r1s_f(l, lid0, arg0, arg1, 0u32);
    x4 = ((x3).wrapping_mul(t302)).wrapping_add(t303);
    if ((x2) == (0u32)) {
        res = x4;
        return res;
    }
    let t304 = b9_r0r1s_f(l, lid0, arg0, arg1, 4u32);
    let t305 = b11_ss_f(l, lid0, (t304).wrapping_sub((x3).wrapping_shl((10u32) as u32)), 1024u32);
    x5 = t305;
    let t306 = b10_r0r1s_f(l, lid0, arg0, arg1, x2);
    res = (x4).wrapping_add((x5).wrapping_mul(t306));
    return res;
}

#[inline(always)]
unsafe fn b5_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x5: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t307 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 40u32);
    let t308 = [vdupq_n_u32(t307); 2];
    let t309: [uint32x4_t; 2] = [vmulq_u32(x3[0], t308[0]), vmulq_u32(x3[1], t308[1])];
    let t310 = b10_r0r1s_p(l, lid0, live, arg0, arg1, 0u32);
    let t311 = [vdupq_n_u32(t310); 2];
    let t312: [uint32x4_t; 2] = [vaddq_u32(t309[0], t311[0]), vaddq_u32(t309[1], t311[1])];
    x4 = t312;
    if ((x2) == (0u32)) {
        res = x4;
        return res;
    }
    let t313 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    let t314: [uint32x4_t; 2] = [vshlq_u32(x3[0], vdupq_n_s32(((10u32) & 31) as i32)), vshlq_u32(x3[1], vdupq_n_s32(((10u32) & 31) as i32))];
    let t315 = [vdupq_n_u32(t313); 2];
    let t316: [uint32x4_t; 2] = [vsubq_u32(t315[0], t314[0]), vsubq_u32(t315[1], t314[1])];
    let t317 = b11_ls_p(l, lid0, live, t316, 1024u32);
    x5 = t317;
    let t318 = b10_r0r1s_p(l, lid0, live, arg0, arg1, x2);
    let t319 = [vdupq_n_u32(t318); 2];
    let t320: [uint32x4_t; 2] = [vmulq_u32(x5[0], t319[0]), vmulq_u32(x5[1], t319[1])];
    let t321: [uint32x4_t; 2] = [vaddq_u32(x4[0], t320[0]), vaddq_u32(x4[1], t320[1])];
    res = t321;
    return res;
}

#[inline(always)]
unsafe fn b5_r0r1sl_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32, x3: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x5: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t322 = b9_r0r1s_f(l, lid0, arg0, arg1, 40u32);
    let t323 = [vdupq_n_u32(t322); 2];
    let t324: [uint32x4_t; 2] = [vmulq_u32(x3[0], t323[0]), vmulq_u32(x3[1], t323[1])];
    let t325 = b10_r0r1s_f(l, lid0, arg0, arg1, 0u32);
    let t326 = [vdupq_n_u32(t325); 2];
    let t327: [uint32x4_t; 2] = [vaddq_u32(t324[0], t326[0]), vaddq_u32(t324[1], t326[1])];
    x4 = t327;
    if ((x2) == (0u32)) {
        res = x4;
        return res;
    }
    let t328 = b9_r0r1s_f(l, lid0, arg0, arg1, 4u32);
    let t329: [uint32x4_t; 2] = [vshlq_u32(x3[0], vdupq_n_s32(((10u32) & 31) as i32)), vshlq_u32(x3[1], vdupq_n_s32(((10u32) & 31) as i32))];
    let t330 = [vdupq_n_u32(t328); 2];
    let t331: [uint32x4_t; 2] = [vsubq_u32(t330[0], t329[0]), vsubq_u32(t330[1], t329[1])];
    let t332 = b11_ls_f(l, lid0, t331, 1024u32);
    x5 = t332;
    let t333 = b10_r0r1s_f(l, lid0, arg0, arg1, x2);
    let t334 = [vdupq_n_u32(t333); 2];
    let t335: [uint32x4_t; 2] = [vmulq_u32(x5[0], t334[0]), vmulq_u32(x5[1], t334[1])];
    let t336: [uint32x4_t; 2] = [vaddq_u32(x4[0], t335[0]), vaddq_u32(x4[1], t335[1])];
    res = t336;
    return res;
}

#[inline(always)]
unsafe fn b6_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t337: u64;
    if ((x0) >= (8u32)) {
        t337 = !(0u64);
    } else {
        t337 = ((1u64).wrapping_shl(((((x0).wrapping_mul(8u32)) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t337;
    return res;
}

#[inline(always)]
unsafe fn b6_s_f(l: &Launch, lid0: u32, x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t338: u64;
    if ((x0) >= (8u32)) {
        t338 = !(0u64);
    } else {
        t338 = ((1u64).wrapping_shl(((((x0).wrapping_mul(8u32)) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t338;
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1sns_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3f: u32, x3s: u32, x4: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t339 = b12_r0sns_p(l, lid0, live, arg0, x2, x3f, x3s, x4);
    res = t339;
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1sns_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32, x3f: u32, x3s: u32, x4: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t340 = b12_r0sns_f(l, lid0, arg0, x2, x3f, x3s, x4);
    res = t340;
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: [uint32x4_t; 2], x3: [uint32x4_t; 2], x4: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t341 = b12_r0lls_p(l, lid0, live, arg0, x2, x3, x4);
    res = t341;
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1lls_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: [uint32x4_t; 2], x3: [uint32x4_t; 2], x4: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t342 = b12_r0lls_f(l, lid0, arg0, x2, x3, x4);
    res = t342;
    return res;
}

#[inline(always)]
unsafe fn b8_r0r1ss_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32, x3: u32) -> u64 {
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
    let t343 = b9_r0r1s_f(l, lid0, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x2)));
    x4 = t343;
    let t344 = b9_r0r1s_f(l, lid0, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x2)));
    x5 = t344;
    x6 = (x3).wrapping_shr((10u32) as u32);
    let t345 = b13_r0r1sss_f(l, lid0, arg0, arg1, x5, (x6).wrapping_mul(4u32), 4u32);
    x7 = ((t345) as u32);
    let t346 = b13_r0r1sss_f(l, lid0, arg0, arg1, x5, ((x6).wrapping_mul(4u32)).wrapping_add(4u32), 4u32);
    x8 = ((t346) as u32);
    let t347 = b9_r0r1s_f(l, lid0, arg0, arg1, 4u32);
    x9 = ((t347).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
    let t348 = b14_ss_f(l, lid0, 4u32, (x9).wrapping_sub((x6).wrapping_mul(4u32)));
    x10 = t348;
    x11 = (((x3).wrapping_shr((8u32) as u32)) & (3u32));
    let t349: u32;
    if ((x11) == (0u32)) {
        t349 = 0u32;
    } else {
        let t350 = b15_ss_f(l, lid0, (x7).wrapping_add((2u32).wrapping_mul((x11).wrapping_sub(1u32))), x8);
        let t351 = b13_r0r1sss_f(l, lid0, arg0, arg1, x4, t350, 2u32);
        t349 = ((t351) as u32);
    }
    x12 = t349;
    let t352 = b15_ss_f(l, lid0, (x7).wrapping_add((2u32).wrapping_mul(x11)), x8);
    let t353 = b13_r0r1sss_f(l, lid0, arg0, arg1, x4, t352, 2u32);
    x13 = ((t353) as u32);
    x14 = (x7).wrapping_add((2u32).wrapping_mul(x10));
    x15 = ((x3) & (255u32));
    x16 = 9259542123273814144u64;
    x17 = (((x15) as u64)).wrapping_mul(72340172838076673u64);
    x18 = x12;
    x19 = x12;
    loop {
        // Rounds of one condition: every lane goes on.
        if !(((x19) < (x13))) { break; }
        let t354 = b15_ss_f(l, lid0, (x14).wrapping_add(x19), x8);
        let t355 = b13_r0r1sss_f(l, lid0, arg0, arg1, x4, t354, 8u32);
        x20 = t355;
        x21 = (((x20) | (x16))).wrapping_sub(((x17) & (!(x16))));
        x22 = ((x16) & (((((!(x20)) & (x17))) | (((!(((x20) ^ (x17)))) & (!(x21)))))));
        x23 = (x13).wrapping_sub(x19);
        let t356: u64;
        if ((x23) >= (8u32)) {
            t356 = x22;
        } else {
            t356 = ((x22) & (((1u64).wrapping_shl(((((8u32).wrapping_mul(x23)) as u64)) as u32)).wrapping_sub(1u64)));
        }
        let t357 = b16_s_f(l, lid0, t356);
        x18 = (x18).wrapping_add(t357);
        x19 = (x19).wrapping_add(8u32);
    }
    x12 = x18;
    let t358 = b9_r0r1s_f(l, lid0, arg0, arg1, 16u32);
    x24 = (((t358).wrapping_shr((((((4u32).wrapping_mul(x2)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32));
    let t359 = b15_ss_f(l, lid0, (x7).wrapping_add((2u32).wrapping_mul((x10).wrapping_sub(1u32))), x8);
    let t360 = b13_r0r1sss_f(l, lid0, arg0, arg1, x4, t359, 2u32);
    x25 = ((t360) as u32);
    let t361 = b15_ss_f(l, lid0, ((x14).wrapping_add(x25)).wrapping_add((x12).wrapping_mul(x24)), x8);
    x26 = t361;
    let t362 = b13_r0r1sss_f(l, lid0, arg0, arg1, x4, x26, 8u32);
    let t363 = b6_s_f(l, lid0, x24);
    res = ((t362) & (t363));
    return res;
}

#[inline(always)]
unsafe fn b8_r0r1si_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: u32) -> [uint32x4_t; 4] {
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
    let t364 = b9_r0r1s_p(l, lid0, live, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x2)));
    x4 = t364;
    let t365 = b9_r0r1s_p(l, lid0, live, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x2)));
    x5 = t365;
    let t366: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x3), vld1q_u32(LANE[1].as_ptr()))];
    let t367: [uint32x4_t; 2] = [vshlq_u32(t366[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(t366[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    x6 = t367;
    let t368 = [vdupq_n_u32(4u32); 2];
    let t369: [uint32x4_t; 2] = [vmulq_u32(x6[0], t368[0]), vmulq_u32(x6[1], t368[1])];
    let t370 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x5, t369, 4u32);
    let t371: [uint32x4_t; 2] = [t370[0], t370[1]];
    x7 = t371;
    let t372: [uint32x4_t; 2] = [vmulq_u32(x6[0], t368[0]), vmulq_u32(x6[1], t368[1])];
    let t373: [uint32x4_t; 2] = [vaddq_u32(t372[0], t368[0]), vaddq_u32(t372[1], t368[1])];
    let t374 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x5, t373, 4u32);
    let t375: [uint32x4_t; 2] = [t374[0], t374[1]];
    x8 = t375;
    let t376 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    x9 = ((t376).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
    let t377: [uint32x4_t; 2] = [vmulq_u32(x6[0], t368[0]), vmulq_u32(x6[1], t368[1])];
    let t378 = [vdupq_n_u32(x9); 2];
    let t379: [uint32x4_t; 2] = [vsubq_u32(t378[0], t377[0]), vsubq_u32(t378[1], t377[1])];
    let t380 = b14_sl_p(l, lid0, live, 4u32, t379);
    x10 = t380;
    let t381: [uint32x4_t; 2] = [vshlq_u32(t366[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(t366[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
    let t382 = [vdupq_n_u32(3u32); 2];
    let t383: [uint32x4_t; 2] = [vandq_u32(t381[0], t382[0]), vandq_u32(t381[1], t382[1])];
    x11 = t383;
    let t384 = [vdupq_n_u32(0u32); 2];
    let t385: [uint32x4_t; 2] = [vceqq_u32(x11[0], t384[0]), vceqq_u32(x11[1], t384[1])];
    let mut t386: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t387: [uint32x4_t; 2] = [vandq_u32(live[0], t385[0]), vandq_u32(live[1], t385[1])];
    let t388: [uint32x4_t; 2] = [vbicq_u32(live[0], t385[0]), vbicq_u32(live[1], t385[1])];
    {
        t386 = [vbslq_u32(t387[0], t384[0], t386[0]), vbslq_u32(t387[1], t384[1], t386[1])];
    }
    if any(&t388) {
        let t389 = [vdupq_n_u32(1u32); 2];
        let t390: [uint32x4_t; 2] = [vsubq_u32(x11[0], t389[0]), vsubq_u32(x11[1], t389[1])];
        let t391 = [vdupq_n_u32(2u32); 2];
        let t392: [uint32x4_t; 2] = [vmulq_u32(t391[0], t390[0]), vmulq_u32(t391[1], t390[1])];
        let t393: [uint32x4_t; 2] = [vaddq_u32(x7[0], t392[0]), vaddq_u32(x7[1], t392[1])];
        let t394 = b15_ll_p(l, lid0, t388, t393, x8);
        let t395 = b13_r0r1sls_p(l, lid0, t388, arg0, arg1, x4, t394, 2u32);
        let t396: [uint32x4_t; 2] = [t395[0], t395[1]];
        t386 = [vbslq_u32(t388[0], t396[0], t386[0]), vbslq_u32(t388[1], t396[1], t386[1])];
    }
    x12 = t386;
    let t397 = [vdupq_n_u32(2u32); 2];
    let t398: [uint32x4_t; 2] = [vmulq_u32(t397[0], x11[0]), vmulq_u32(t397[1], x11[1])];
    let t399: [uint32x4_t; 2] = [vaddq_u32(x7[0], t398[0]), vaddq_u32(x7[1], t398[1])];
    let t400 = b15_ll_p(l, lid0, live, t399, x8);
    let t401 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x4, t400, 2u32);
    let t402: [uint32x4_t; 2] = [t401[0], t401[1]];
    x13 = t402;
    let t403: [uint32x4_t; 2] = [vmulq_u32(t397[0], x10[0]), vmulq_u32(t397[1], x10[1])];
    let t404: [uint32x4_t; 2] = [vaddq_u32(x7[0], t403[0]), vaddq_u32(x7[1], t403[1])];
    x14 = t404;
    let t405 = [vdupq_n_u32(255u32); 2];
    let t406: [uint32x4_t; 2] = [vandq_u32(t366[0], t405[0]), vandq_u32(t366[1], t405[1])];
    x15 = t406;
    x16 = 9259542123273814144u64;
    let t407: u64 = 72340172838076673u64;
    let t408 = [vdupq_n_u32((t407) as u32); 2];
    let t409 = [vdupq_n_u32(((t407) >> 32) as u32); 2];
    let t410: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x15[0]), vget_low_u32(t408[0]))), vreinterpretq_u32_u64(vmull_high_u32(x15[0], t408[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x15[1]), vget_low_u32(t408[1]))), vreinterpretq_u32_u64(vmull_high_u32(x15[1], t408[1])))];
    let t411: [uint32x4_t; 4] = [vmulq_u32(x15[0], t408[0]), vmulq_u32(x15[1], t408[1]), vmlaq_u32(t410[0], x15[0], t409[0]), vmlaq_u32(t410[1], x15[1], t409[1])];
    x17 = t411;
    x18 = x12;
    x19 = x12;
    let mut t413 = live;
    loop {
        // Lanes part: each its own rounds.
        let t415: [uint32x4_t; 2] = [vcltq_u32(x19[0], x13[0]), vcltq_u32(x19[1], x13[1])];
        t413 = [vandq_u32(t413[0], t415[0]), vandq_u32(t413[1], t415[1])];
        if !(any(&t413)) { break; }
        let t417: [uint32x4_t; 2] = [vaddq_u32(x14[0], x19[0]), vaddq_u32(x14[1], x19[1])];
        let t418 = b15_ll_p(l, lid0, t413, t417, x8);
        let t419 = b13_r0r1sls_p(l, lid0, t413, arg0, arg1, x4, t418, 8u32);
        x20 = t419;
        let t420: u64 = x16;
        let t421 = [vdupq_n_u32((t420) as u32); 2];
        let t422 = [vdupq_n_u32(((t420) >> 32) as u32); 2];
        let t423: [uint32x4_t; 4] = [vorrq_u32(x20[0], t421[0]), vorrq_u32(x20[1], t421[1]), vorrq_u32(x20[2], t422[0]), vorrq_u32(x20[3], t422[1])];
        let t424: u64 = !(x16);
        let t425 = [vdupq_n_u32((t424) as u32); 2];
        let t426 = [vdupq_n_u32(((t424) >> 32) as u32); 2];
        let t427: [uint32x4_t; 4] = [vandq_u32(x17[0], t425[0]), vandq_u32(x17[1], t425[1]), vandq_u32(x17[2], t426[0]), vandq_u32(x17[3], t426[1])];
        let t428: [uint32x4_t; 2] = [vcltq_u32(t423[0], t427[0]), vcltq_u32(t423[1], t427[1])];
        let t429: [uint32x4_t; 4] = [vsubq_u32(t423[0], t427[0]), vsubq_u32(t423[1], t427[1]), vaddq_u32(vsubq_u32(t423[2], t427[2]), t428[0]), vaddq_u32(vsubq_u32(t423[3], t427[3]), t428[1])];
        x21 = t429;
        let t430: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x20[r]));
        let t431: [uint32x4_t; 4] = [vandq_u32(t430[0], x17[0]), vandq_u32(t430[1], x17[1]), vandq_u32(t430[2], x17[2]), vandq_u32(t430[3], x17[3])];
        let t432: [uint32x4_t; 4] = [veorq_u32(x20[0], x17[0]), veorq_u32(x20[1], x17[1]), veorq_u32(x20[2], x17[2]), veorq_u32(x20[3], x17[3])];
        let t433: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(t432[r]));
        let t434: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x21[r]));
        let t435: [uint32x4_t; 4] = [vandq_u32(t433[0], t434[0]), vandq_u32(t433[1], t434[1]), vandq_u32(t433[2], t434[2]), vandq_u32(t433[3], t434[3])];
        let t436: [uint32x4_t; 4] = [vorrq_u32(t431[0], t435[0]), vorrq_u32(t431[1], t435[1]), vorrq_u32(t431[2], t435[2]), vorrq_u32(t431[3], t435[3])];
        let t437: u64 = x16;
        let t438 = [vdupq_n_u32((t437) as u32); 2];
        let t439 = [vdupq_n_u32(((t437) >> 32) as u32); 2];
        let t440: [uint32x4_t; 4] = [vandq_u32(t438[0], t436[0]), vandq_u32(t438[1], t436[1]), vandq_u32(t439[0], t436[2]), vandq_u32(t439[1], t436[3])];
        x22 = t440;
        let t441: [uint32x4_t; 2] = [vsubq_u32(x13[0], x19[0]), vsubq_u32(x13[1], x19[1])];
        x23 = t441;
        let t442 = [vdupq_n_u32(8u32); 2];
        let t443: [uint32x4_t; 2] = [vcgeq_u32(x23[0], t442[0]), vcgeq_u32(x23[1], t442[1])];
        let mut t444: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t445: [uint32x4_t; 2] = [vandq_u32(t413[0], t443[0]), vandq_u32(t413[1], t443[1])];
        let t446: [uint32x4_t; 2] = [vbicq_u32(t413[0], t443[0]), vbicq_u32(t413[1], t443[1])];
        {
            t444 = [vbslq_u32(t445[0], x22[0], t444[0]), vbslq_u32(t445[1], x22[1], t444[1]), vbslq_u32(t445[0], x22[2], t444[2]), vbslq_u32(t445[1], x22[3], t444[3])];
        }
        {
            let t447: [uint32x4_t; 2] = [vmulq_u32(t442[0], x23[0]), vmulq_u32(t442[1], x23[1])];
            let t448: u64 = 1u64;
            let t449 = [vdupq_n_u32((t448) as u32); 2];
            let t450: [uint32x4_t; 2] = [t447[0], t447[1]];
            let t451: [uint32x4_t; 2] = [vandq_u32(t450[0], vdupq_n_u32(63)), vandq_u32(t450[1], vdupq_n_u32(63))];
            let t452: [uint32x4_t; 4] = [vshlq_u32(t449[0], vreinterpretq_s32_u32(t451[0])), vshlq_u32(t449[1], vreinterpretq_s32_u32(t451[1])), vshlq_u32(t449[0], vsubq_s32(vreinterpretq_s32_u32(t451[0]), vdupq_n_s32(32))), vshlq_u32(t449[1], vsubq_s32(vreinterpretq_s32_u32(t451[1]), vdupq_n_s32(32)))];
            let t453: u64 = 1u64;
            let t454 = [vdupq_n_u32((t453) as u32); 2];
            let t455: [uint32x4_t; 2] = [vcltq_u32(t452[0], t454[0]), vcltq_u32(t452[1], t454[1])];
            let t456: [uint32x4_t; 4] = [vsubq_u32(t452[0], t454[0]), vsubq_u32(t452[1], t454[1]), vaddq_u32(vsubq_u32(t452[2], vdupq_n_u32(0)), t455[0]), vaddq_u32(vsubq_u32(t452[3], vdupq_n_u32(0)), t455[1])];
            let t457: [uint32x4_t; 4] = [vandq_u32(x22[0], t456[0]), vandq_u32(x22[1], t456[1]), vandq_u32(x22[2], t456[2]), vandq_u32(x22[3], t456[3])];
            t444 = [vbslq_u32(t446[0], t457[0], t444[0]), vbslq_u32(t446[1], t457[1], t444[1]), vbslq_u32(t446[0], t457[2], t444[2]), vbslq_u32(t446[1], t457[3], t444[3])];
        }
        let t458 = b16_l_p(l, lid0, t413, t444);
        let t459: [uint32x4_t; 2] = [vaddq_u32(x18[0], t458[0]), vaddq_u32(x18[1], t458[1])];
        x18 = [vbslq_u32(t413[0], t459[0], x18[0]), vbslq_u32(t413[1], t459[1], x18[1])];
        let t460: [uint32x4_t; 2] = [vandq_u32(t413[0], live[0]), vandq_u32(t413[1], live[1])];
        let t461: [uint32x4_t; 2] = [vaddq_u32(x19[0], t442[0]), vaddq_u32(x19[1], t442[1])];
        x19 = t461;
    }
    x12 = x18;
    let t462 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    x24 = (((t462).wrapping_shr((((((4u32).wrapping_mul(x2)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32));
    let t463 = [vdupq_n_u32(1u32); 2];
    let t464: [uint32x4_t; 2] = [vsubq_u32(x10[0], t463[0]), vsubq_u32(x10[1], t463[1])];
    let t465: [uint32x4_t; 2] = [vmulq_u32(t397[0], t464[0]), vmulq_u32(t397[1], t464[1])];
    let t466: [uint32x4_t; 2] = [vaddq_u32(x7[0], t465[0]), vaddq_u32(x7[1], t465[1])];
    let t467 = b15_ll_p(l, lid0, live, t466, x8);
    let t468 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x4, t467, 2u32);
    let t469: [uint32x4_t; 2] = [t468[0], t468[1]];
    x25 = t469;
    let t470: [uint32x4_t; 2] = [vaddq_u32(x14[0], x25[0]), vaddq_u32(x14[1], x25[1])];
    let t471 = [vdupq_n_u32(x24); 2];
    let t472: [uint32x4_t; 2] = [vmulq_u32(x12[0], t471[0]), vmulq_u32(x12[1], t471[1])];
    let t473: [uint32x4_t; 2] = [vaddq_u32(t470[0], t472[0]), vaddq_u32(t470[1], t472[1])];
    let t474 = b15_ll_p(l, lid0, live, t473, x8);
    x26 = t474;
    let t475 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x4, x26, 8u32);
    let t476 = b6_s_p(l, lid0, live, x24);
    let t477: u64 = t476;
    let t478 = [vdupq_n_u32((t477) as u32); 2];
    let t479 = [vdupq_n_u32(((t477) >> 32) as u32); 2];
    let t480: [uint32x4_t; 4] = [vandq_u32(t475[0], t478[0]), vandq_u32(t475[1], t478[1]), vandq_u32(t475[2], t479[0]), vandq_u32(t475[3], t479[1])];
    res = t480;
    return res;
}

#[inline(always)]
unsafe fn b8_r0r1su8_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: u32) -> [uint32x4_t; 4] {
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
    let t481 = b9_r0r1s_p(l, lid0, live, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x2)));
    x4 = t481;
    let t482 = b9_r0r1s_p(l, lid0, live, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x2)));
    x5 = t482;
    x6 = (x3 >> 10);
    let t483 = b13_r0r1sss_p(l, lid0, live, arg0, arg1, x5, (x6).wrapping_mul(4u32), 4u32);
    x7 = ((t483) as u32);
    let t484 = b13_r0r1sss_p(l, lid0, live, arg0, arg1, x5, ((x6).wrapping_mul(4u32)).wrapping_add(4u32), 4u32);
    x8 = ((t484) as u32);
    let t485 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    x9 = ((t485).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
    let t486 = b14_ss_p(l, lid0, live, 4u32, (x9).wrapping_sub((x6).wrapping_mul(4u32)));
    x10 = t486;
    x11 = (((x3 >> 8)) & (3u32));
    let t487: u32;
    if ((x11) == (0u32)) {
        t487 = 0u32;
    } else {
        let t488 = b15_ss_p(l, lid0, live, (x7).wrapping_add((2u32).wrapping_mul((x11).wrapping_sub(1u32))), x8);
        let t489 = b13_r0r1sss_p(l, lid0, live, arg0, arg1, x4, t488, 2u32);
        t487 = ((t489) as u32);
    }
    let t490 = [vdupq_n_u32(t487); 2];
    x12 = t490;
    let t491 = b15_ss_p(l, lid0, live, (x7).wrapping_add((2u32).wrapping_mul(x11)), x8);
    let t492 = b13_r0r1sss_p(l, lid0, live, arg0, arg1, x4, t491, 2u32);
    x13 = ((t492) as u32);
    x14 = (x7).wrapping_add((2u32).wrapping_mul(x10));
    x15 = (x3 & 255u32);
    x16 = 9259542123273814144u64;
    let t493: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x15), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x15), vld1q_u32(LANE[1].as_ptr()))];
    let t494: u64 = 72340172838076673u64;
    let t495 = [vdupq_n_u32((t494) as u32); 2];
    let t496 = [vdupq_n_u32(((t494) >> 32) as u32); 2];
    let t497: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(t493[0]), vget_low_u32(t495[0]))), vreinterpretq_u32_u64(vmull_high_u32(t493[0], t495[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(t493[1]), vget_low_u32(t495[1]))), vreinterpretq_u32_u64(vmull_high_u32(t493[1], t495[1])))];
    let t498: [uint32x4_t; 4] = [vmulq_u32(t493[0], t495[0]), vmulq_u32(t493[1], t495[1]), vmlaq_u32(t497[0], t493[0], t496[0]), vmlaq_u32(t497[1], t493[1], t496[1])];
    x17 = t498;
    x18 = x12;
    x19 = x12;
    let mut t501 = live;
    loop {
        // Lanes part: each its own rounds.
        let t503 = [vdupq_n_u32(x13); 2];
        let t504: [uint32x4_t; 2] = [vcltq_u32(x19[0], t503[0]), vcltq_u32(x19[1], t503[1])];
        t501 = [vandq_u32(t501[0], t504[0]), vandq_u32(t501[1], t504[1])];
        if !(any(&t501)) { break; }
        let t506 = [vdupq_n_u32(x14); 2];
        let t507: [uint32x4_t; 2] = [vaddq_u32(t506[0], x19[0]), vaddq_u32(t506[1], x19[1])];
        let t508 = b15_ls_p(l, lid0, t501, t507, x8);
        let t509 = b13_r0r1sls_p(l, lid0, t501, arg0, arg1, x4, t508, 8u32);
        x20 = t509;
        let t510: u64 = x16;
        let t511 = [vdupq_n_u32((t510) as u32); 2];
        let t512 = [vdupq_n_u32(((t510) >> 32) as u32); 2];
        let t513: [uint32x4_t; 4] = [vorrq_u32(x20[0], t511[0]), vorrq_u32(x20[1], t511[1]), vorrq_u32(x20[2], t512[0]), vorrq_u32(x20[3], t512[1])];
        let t514: u64 = !(x16);
        let t515 = [vdupq_n_u32((t514) as u32); 2];
        let t516 = [vdupq_n_u32(((t514) >> 32) as u32); 2];
        let t517: [uint32x4_t; 4] = [vandq_u32(x17[0], t515[0]), vandq_u32(x17[1], t515[1]), vandq_u32(x17[2], t516[0]), vandq_u32(x17[3], t516[1])];
        let t518: [uint32x4_t; 2] = [vcltq_u32(t513[0], t517[0]), vcltq_u32(t513[1], t517[1])];
        let t519: [uint32x4_t; 4] = [vsubq_u32(t513[0], t517[0]), vsubq_u32(t513[1], t517[1]), vaddq_u32(vsubq_u32(t513[2], t517[2]), t518[0]), vaddq_u32(vsubq_u32(t513[3], t517[3]), t518[1])];
        x21 = t519;
        let t520: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x20[r]));
        let t521: [uint32x4_t; 4] = [vandq_u32(t520[0], x17[0]), vandq_u32(t520[1], x17[1]), vandq_u32(t520[2], x17[2]), vandq_u32(t520[3], x17[3])];
        let t522: [uint32x4_t; 4] = [veorq_u32(x20[0], x17[0]), veorq_u32(x20[1], x17[1]), veorq_u32(x20[2], x17[2]), veorq_u32(x20[3], x17[3])];
        let t523: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(t522[r]));
        let t524: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x21[r]));
        let t525: [uint32x4_t; 4] = [vandq_u32(t523[0], t524[0]), vandq_u32(t523[1], t524[1]), vandq_u32(t523[2], t524[2]), vandq_u32(t523[3], t524[3])];
        let t526: [uint32x4_t; 4] = [vorrq_u32(t521[0], t525[0]), vorrq_u32(t521[1], t525[1]), vorrq_u32(t521[2], t525[2]), vorrq_u32(t521[3], t525[3])];
        let t527: u64 = x16;
        let t528 = [vdupq_n_u32((t527) as u32); 2];
        let t529 = [vdupq_n_u32(((t527) >> 32) as u32); 2];
        let t530: [uint32x4_t; 4] = [vandq_u32(t528[0], t526[0]), vandq_u32(t528[1], t526[1]), vandq_u32(t529[0], t526[2]), vandq_u32(t529[1], t526[3])];
        x22 = t530;
        let t531 = [vdupq_n_u32(x13); 2];
        let t532: [uint32x4_t; 2] = [vsubq_u32(t531[0], x19[0]), vsubq_u32(t531[1], x19[1])];
        x23 = t532;
        let t533 = [vdupq_n_u32(8u32); 2];
        let t534: [uint32x4_t; 2] = [vcgeq_u32(x23[0], t533[0]), vcgeq_u32(x23[1], t533[1])];
        let mut t535: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t536: [uint32x4_t; 2] = [vandq_u32(t501[0], t534[0]), vandq_u32(t501[1], t534[1])];
        let t537: [uint32x4_t; 2] = [vbicq_u32(t501[0], t534[0]), vbicq_u32(t501[1], t534[1])];
        {
            t535 = [vbslq_u32(t536[0], x22[0], t535[0]), vbslq_u32(t536[1], x22[1], t535[1]), vbslq_u32(t536[0], x22[2], t535[2]), vbslq_u32(t536[1], x22[3], t535[3])];
        }
        {
            let t538: [uint32x4_t; 2] = [vmulq_u32(t533[0], x23[0]), vmulq_u32(t533[1], x23[1])];
            let t539: u64 = 1u64;
            let t540 = [vdupq_n_u32((t539) as u32); 2];
            let t541: [uint32x4_t; 2] = [t538[0], t538[1]];
            let t542: [uint32x4_t; 2] = [vandq_u32(t541[0], vdupq_n_u32(63)), vandq_u32(t541[1], vdupq_n_u32(63))];
            let t543: [uint32x4_t; 4] = [vshlq_u32(t540[0], vreinterpretq_s32_u32(t542[0])), vshlq_u32(t540[1], vreinterpretq_s32_u32(t542[1])), vshlq_u32(t540[0], vsubq_s32(vreinterpretq_s32_u32(t542[0]), vdupq_n_s32(32))), vshlq_u32(t540[1], vsubq_s32(vreinterpretq_s32_u32(t542[1]), vdupq_n_s32(32)))];
            let t544: u64 = 1u64;
            let t545 = [vdupq_n_u32((t544) as u32); 2];
            let t546: [uint32x4_t; 2] = [vcltq_u32(t543[0], t545[0]), vcltq_u32(t543[1], t545[1])];
            let t547: [uint32x4_t; 4] = [vsubq_u32(t543[0], t545[0]), vsubq_u32(t543[1], t545[1]), vaddq_u32(vsubq_u32(t543[2], vdupq_n_u32(0)), t546[0]), vaddq_u32(vsubq_u32(t543[3], vdupq_n_u32(0)), t546[1])];
            let t548: [uint32x4_t; 4] = [vandq_u32(x22[0], t547[0]), vandq_u32(x22[1], t547[1]), vandq_u32(x22[2], t547[2]), vandq_u32(x22[3], t547[3])];
            t535 = [vbslq_u32(t537[0], t548[0], t535[0]), vbslq_u32(t537[1], t548[1], t535[1]), vbslq_u32(t537[0], t548[2], t535[2]), vbslq_u32(t537[1], t548[3], t535[3])];
        }
        let t549 = b16_l_p(l, lid0, t501, t535);
        let t550: [uint32x4_t; 2] = [vaddq_u32(x18[0], t549[0]), vaddq_u32(x18[1], t549[1])];
        x18 = [vbslq_u32(t501[0], t550[0], x18[0]), vbslq_u32(t501[1], t550[1], x18[1])];
        let t551: [uint32x4_t; 2] = [vandq_u32(t501[0], live[0]), vandq_u32(t501[1], live[1])];
        let t552: [uint32x4_t; 2] = [vaddq_u32(x19[0], t533[0]), vaddq_u32(x19[1], t533[1])];
        x19 = t552;
    }
    x12 = x18;
    let t553 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    x24 = (((t553).wrapping_shr((((((4u32).wrapping_mul(x2)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32));
    let t554 = b15_ss_p(l, lid0, live, (x7).wrapping_add((2u32).wrapping_mul((x10).wrapping_sub(1u32))), x8);
    let t555 = b13_r0r1sss_p(l, lid0, live, arg0, arg1, x4, t554, 2u32);
    x25 = ((t555) as u32);
    let t556 = [vdupq_n_u32(x24); 2];
    let t557: [uint32x4_t; 2] = [vmulq_u32(x12[0], t556[0]), vmulq_u32(x12[1], t556[1])];
    let t558 = [vdupq_n_u32((x14).wrapping_add(x25)); 2];
    let t559: [uint32x4_t; 2] = [vaddq_u32(t558[0], t557[0]), vaddq_u32(t558[1], t557[1])];
    let t560 = b15_ls_p(l, lid0, live, t559, x8);
    x26 = t560;
    let t561 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x4, x26, 8u32);
    let t562 = b6_s_p(l, lid0, live, x24);
    let t563: u64 = t562;
    let t564 = [vdupq_n_u32((t563) as u32); 2];
    let t565 = [vdupq_n_u32(((t563) >> 32) as u32); 2];
    let t566: [uint32x4_t; 4] = [vandq_u32(t561[0], t564[0]), vandq_u32(t561[1], t564[1]), vandq_u32(t561[2], t565[0]), vandq_u32(t561[3], t565[1])];
    res = t566;
    return res;
}

#[inline(always)]
unsafe fn b9_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    res = (arg1.wrapping_add(((((x2).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned();
    return res;
}

#[inline(always)]
unsafe fn b9_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    res = (arg1.wrapping_add(((((x2).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned();
    return res;
}

#[inline(always)]
unsafe fn b10_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t567 = b9_r0r1s_p(l, lid0, live, arg0, arg1, (20u32).wrapping_add((4u32).wrapping_mul(x2)));
    res = t567;
    return res;
}

#[inline(always)]
unsafe fn b10_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t568 = b9_r0r1s_f(l, lid0, arg0, arg1, (20u32).wrapping_add((4u32).wrapping_mul(x2)));
    res = t568;
    return res;
}

#[inline(always)]
unsafe fn b11_ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x1: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t569: u32;
    if ((x0) < (x1)) {
        t569 = x0;
    } else {
        t569 = x1;
    }
    res = t569;
    return res;
}

#[inline(always)]
unsafe fn b11_ss_f(l: &Launch, lid0: u32, x0: u32, x1: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t570: u32;
    if ((x0) < (x1)) {
        t570 = x0;
    } else {
        t570 = x1;
    }
    res = t570;
    return res;
}

#[inline(always)]
unsafe fn b11_ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x1: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t571 = [vdupq_n_u32(x1); 2];
    let t572: [uint32x4_t; 2] = [vcltq_u32(x0[0], t571[0]), vcltq_u32(x0[1], t571[1])];
    let mut t573: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t574: [uint32x4_t; 2] = [vandq_u32(live[0], t572[0]), vandq_u32(live[1], t572[1])];
    let t575: [uint32x4_t; 2] = [vbicq_u32(live[0], t572[0]), vbicq_u32(live[1], t572[1])];
    {
        t573 = [vbslq_u32(t574[0], x0[0], t573[0]), vbslq_u32(t574[1], x0[1], t573[1])];
    }
    {
        let t576 = [vdupq_n_u32(x1); 2];
        t573 = [vbslq_u32(t575[0], t576[0], t573[0]), vbslq_u32(t575[1], t576[1], t573[1])];
    }
    res = t573;
    return res;
}

#[inline(always)]
unsafe fn b11_ls_f(l: &Launch, lid0: u32, x0: [uint32x4_t; 2], x1: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t577 = [vdupq_n_u32(x1); 2];
    let t578: [uint32x4_t; 2] = [vcltq_u32(x0[0], t577[0]), vcltq_u32(x0[1], t577[1])];
    let mut t579: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t580: [uint32x4_t; 2] = [vmvnq_u32(t578[0]), vmvnq_u32(t578[1])];
    {
        t579 = [vbslq_u32(t578[0], x0[0], t579[0]), vbslq_u32(t578[1], x0[1], t579[1])];
    }
    {
        let t581 = [vdupq_n_u32(x1); 2];
        t579 = [vbslq_u32(t580[0], t581[0], t579[0]), vbslq_u32(t580[1], t581[1], t579[1])];
    }
    res = t579;
    return res;
}

#[inline(always)]
unsafe fn b12_r0sns_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1: u32, x2f: u32, x2s: u32, x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t582 = b17_r0an1s_p(l, lid0, live, arg0, ((((x1) as u64)) as usize as *const u8), x2f, x2s, x3);
    x4 = t582;
    let t583: [uint32x4_t; 4];
    if ((x3) >= (8u32)) {
        t583 = x4;
    } else {
        let t584 = b18_s_p(l, lid0, live, (8u32).wrapping_mul(x3));
        let t585: u64 = t584;
        let t586 = [vdupq_n_u32((t585) as u32); 2];
        let t587 = [vdupq_n_u32(((t585) >> 32) as u32); 2];
        let t588: [uint32x4_t; 4] = [vandq_u32(x4[0], t586[0]), vandq_u32(x4[1], t586[1]), vandq_u32(x4[2], t587[0]), vandq_u32(x4[3], t587[1])];
        t583 = t588;
    }
    res = t583;
    return res;
}

#[inline(always)]
unsafe fn b12_r0sns_f(l: &Launch, lid0: u32, arg0: *const u8, x1: u32, x2f: u32, x2s: u32, x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t589 = b17_r0an1s_f(l, lid0, arg0, ((((x1) as u64)) as usize as *const u8), x2f, x2s, x3);
    x4 = t589;
    let t590: [uint32x4_t; 4];
    if ((x3) >= (8u32)) {
        t590 = x4;
    } else {
        let t591 = b18_s_f(l, lid0, (8u32).wrapping_mul(x3));
        let t592: u64 = t591;
        let t593 = [vdupq_n_u32((t592) as u32); 2];
        let t594 = [vdupq_n_u32(((t592) >> 32) as u32); 2];
        let t595: [uint32x4_t; 4] = [vandq_u32(x4[0], t593[0]), vandq_u32(x4[1], t593[1]), vandq_u32(x4[2], t594[0]), vandq_u32(x4[3], t594[1])];
        t590 = t595;
    }
    res = t590;
    return res;
}

#[inline(always)]
unsafe fn b12_r0lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1: [uint32x4_t; 2], x2: [uint32x4_t; 2], x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t596: [uint32x4_t; 2] = [vaddq_u32(x1[0], x2[0]), vaddq_u32(x1[1], x2[1])];
    let t597: [uint32x4_t; 2] = [vcltq_u32(t596[0], x1[0]), vcltq_u32(t596[1], x1[1])];
    let t598: [uint32x4_t; 4] = [t596[0], t596[1], vshrq_n_u32::<31>(t597[0]), vshrq_n_u32::<31>(t597[1])];
    let t599 = b17_r0ls_p(l, lid0, live, arg0, t598, x3);
    x4 = t599;
    let t600: [uint32x4_t; 4];
    if ((x3) >= (8u32)) {
        t600 = x4;
    } else {
        let t601 = b18_s_p(l, lid0, live, (8u32).wrapping_mul(x3));
        let t602: u64 = t601;
        let t603 = [vdupq_n_u32((t602) as u32); 2];
        let t604 = [vdupq_n_u32(((t602) >> 32) as u32); 2];
        let t605: [uint32x4_t; 4] = [vandq_u32(x4[0], t603[0]), vandq_u32(x4[1], t603[1]), vandq_u32(x4[2], t604[0]), vandq_u32(x4[3], t604[1])];
        t600 = t605;
    }
    res = t600;
    return res;
}

#[inline(always)]
unsafe fn b12_r0lls_f(l: &Launch, lid0: u32, arg0: *const u8, x1: [uint32x4_t; 2], x2: [uint32x4_t; 2], x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t606: [uint32x4_t; 2] = [vaddq_u32(x1[0], x2[0]), vaddq_u32(x1[1], x2[1])];
    let t607: [uint32x4_t; 2] = [vcltq_u32(t606[0], x1[0]), vcltq_u32(t606[1], x1[1])];
    let t608: [uint32x4_t; 4] = [t606[0], t606[1], vshrq_n_u32::<31>(t607[0]), vshrq_n_u32::<31>(t607[1])];
    let t609 = b17_r0ls_f(l, lid0, arg0, t608, x3);
    x4 = t609;
    let t610: [uint32x4_t; 4];
    if ((x3) >= (8u32)) {
        t610 = x4;
    } else {
        let t611 = b18_s_f(l, lid0, (8u32).wrapping_mul(x3));
        let t612: u64 = t611;
        let t613 = [vdupq_n_u32((t612) as u32); 2];
        let t614 = [vdupq_n_u32(((t612) >> 32) as u32); 2];
        let t615: [uint32x4_t; 4] = [vandq_u32(x4[0], t613[0]), vandq_u32(x4[1], t613[1]), vandq_u32(x4[2], t614[0]), vandq_u32(x4[3], t614[1])];
        t610 = t615;
    }
    res = t610;
    return res;
}

#[inline(always)]
unsafe fn b13_r0r1sss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: u32, x4: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t616 = b19_r0sss_p(l, lid0, live, arg0, x2, x3, x4);
    res = t616;
    return res;
}

#[inline(always)]
unsafe fn b13_r0r1sss_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: u32, x3: u32, x4: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t617 = b19_r0sss_f(l, lid0, arg0, x2, x3, x4);
    res = t617;
    return res;
}

#[inline(always)]
unsafe fn b13_r0r1sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: u32, x3: [uint32x4_t; 2], x4: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t618 = b19_r0sls_p(l, lid0, live, arg0, x2, x3, x4);
    res = t618;
    return res;
}

#[inline(always)]
unsafe fn b14_ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x1: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t619: u32;
    if ((x0) < (x1)) {
        t619 = x0;
    } else {
        t619 = x1;
    }
    res = t619;
    return res;
}

#[inline(always)]
unsafe fn b14_ss_f(l: &Launch, lid0: u32, x0: u32, x1: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t620: u32;
    if ((x0) < (x1)) {
        t620 = x0;
    } else {
        t620 = x1;
    }
    res = t620;
    return res;
}

#[inline(always)]
unsafe fn b14_sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x1: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t621 = [vdupq_n_u32(x0); 2];
    let t622: [uint32x4_t; 2] = [vcltq_u32(t621[0], x1[0]), vcltq_u32(t621[1], x1[1])];
    let mut t623: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t624: [uint32x4_t; 2] = [vandq_u32(live[0], t622[0]), vandq_u32(live[1], t622[1])];
    let t625: [uint32x4_t; 2] = [vbicq_u32(live[0], t622[0]), vbicq_u32(live[1], t622[1])];
    {
        let t626 = [vdupq_n_u32(x0); 2];
        t623 = [vbslq_u32(t624[0], t626[0], t623[0]), vbslq_u32(t624[1], t626[1], t623[1])];
    }
    {
        t623 = [vbslq_u32(t625[0], x1[0], t623[0]), vbslq_u32(t625[1], x1[1], t623[1])];
    }
    res = t623;
    return res;
}

#[inline(always)]
unsafe fn b15_ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x1: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t627: u32;
    if ((x0) < (x1)) {
        t627 = x0;
    } else {
        t627 = x1;
    }
    res = t627;
    return res;
}

#[inline(always)]
unsafe fn b15_ss_f(l: &Launch, lid0: u32, x0: u32, x1: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t628: u32;
    if ((x0) < (x1)) {
        t628 = x0;
    } else {
        t628 = x1;
    }
    res = t628;
    return res;
}

#[inline(always)]
unsafe fn b15_ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x1: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t629 = [vdupq_n_u32(x1); 2];
    let t630: [uint32x4_t; 2] = [vcltq_u32(x0[0], t629[0]), vcltq_u32(x0[1], t629[1])];
    let mut t631: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t632: [uint32x4_t; 2] = [vandq_u32(live[0], t630[0]), vandq_u32(live[1], t630[1])];
    let t633: [uint32x4_t; 2] = [vbicq_u32(live[0], t630[0]), vbicq_u32(live[1], t630[1])];
    {
        t631 = [vbslq_u32(t632[0], x0[0], t631[0]), vbslq_u32(t632[1], x0[1], t631[1])];
    }
    {
        let t634 = [vdupq_n_u32(x1); 2];
        t631 = [vbslq_u32(t633[0], t634[0], t631[0]), vbslq_u32(t633[1], t634[1], t631[1])];
    }
    res = t631;
    return res;
}

#[inline(always)]
unsafe fn b15_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x1: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t635: [uint32x4_t; 2] = [vcltq_u32(x0[0], x1[0]), vcltq_u32(x0[1], x1[1])];
    let mut t636: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t637: [uint32x4_t; 2] = [vandq_u32(live[0], t635[0]), vandq_u32(live[1], t635[1])];
    let t638: [uint32x4_t; 2] = [vbicq_u32(live[0], t635[0]), vbicq_u32(live[1], t635[1])];
    {
        t636 = [vbslq_u32(t637[0], x0[0], t636[0]), vbslq_u32(t637[1], x0[1], t636[1])];
    }
    {
        t636 = [vbslq_u32(t638[0], x1[0], t636[0]), vbslq_u32(t638[1], x1[1], t636[1])];
    }
    res = t636;
    return res;
}

#[inline(always)]
unsafe fn b16_s_f(l: &Launch, lid0: u32, x0: u64) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    res = (((((x0) as u64).count_ones() as u64)) as u32);
    return res;
}

#[inline(always)]
unsafe fn b16_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t639: [uint32x4_t; 2] = [vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[0])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[2]))))), vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[1])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[3])))))];
    res = t639;
    return res;
}

#[inline(always)]
unsafe fn b17_r0ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1: u64, x2: u32) -> u64 {
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
    x3 = ((arg0.wrapping_add(x1 as usize)) as usize).wrapping_sub(((((((((((((arg0) as usize as u64)).wrapping_add(x1)) & (7u64))) as u32)) as u64)) as u64 as usize as *const u8)) as usize) as *const u8;
    x4 = ((((((((arg0) as usize as u64)).wrapping_add(x1)) & (7u64))) as u32)).wrapping_mul(8u32);
    x5 = ((x3 as *const u64).read_unaligned()).wrapping_shr((((x4) as u64)) as u32);
    let mut t640: bool = ((x4) != (0u32));
    if t640 {
        t640 = (((x4).wrapping_add((x2).wrapping_mul(8u32))) > (64u32));
    }
    let t641: u64;
    if t640 {
        t641 = ((x5) | (((x3.wrapping_add((1u64).wrapping_mul(8u64) as usize) as *const u64).read_unaligned()).wrapping_shl(((((64u32).wrapping_sub(x4)) as u64)) as u32)));
    } else {
        t641 = x5;
    }
    res = t641;
    return res;
}

#[inline(always)]
unsafe fn b17_r0ss_f(l: &Launch, lid0: u32, arg0: *const u8, x1: u64, x2: u32) -> u64 {
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
    x3 = ((arg0.wrapping_add(x1 as usize)) as usize).wrapping_sub(((((((((((((arg0) as usize as u64)).wrapping_add(x1)) & (7u64))) as u32)) as u64)) as u64 as usize as *const u8)) as usize) as *const u8;
    x4 = ((((((((arg0) as usize as u64)).wrapping_add(x1)) & (7u64))) as u32)).wrapping_mul(8u32);
    x5 = ((x3 as *const u64).read_unaligned()).wrapping_shr((((x4) as u64)) as u32);
    let mut t642: bool = ((x4) != (0u32));
    if t642 {
        t642 = (((x4).wrapping_add((x2).wrapping_mul(8u32))) > (64u32));
    }
    let t643: u64;
    if t642 {
        t643 = ((x5) | (((x3.wrapping_add((1u64).wrapping_mul(8u64) as usize) as *const u64).read_unaligned()).wrapping_shl(((((64u32).wrapping_sub(x4)) as u64)) as u32)));
    } else {
        t643 = x5;
    }
    res = t643;
    return res;
}

#[inline(always)]
unsafe fn b17_r0ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1: [uint32x4_t; 4], x2: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x3: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x4: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x5: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if ((x2) == (8u32)) {
        let t644: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
        let t645: [uint32x4_t; 4] = [vandq_u32(x1[0], t644[0]), vandq_u32(x1[1], t644[1]), vandq_u32(x1[2], t644[2]), vandq_u32(x1[3], t644[3])];
        let t646: u64 = ((arg0) as usize as u64);
        let t647 = [vdupq_n_u32((t646) as u32); 2];
        let t648 = [vdupq_n_u32(((t646) >> 32) as u32); 2];
        let t649: [uint32x4_t; 2] = [vaddq_u32(t647[0], t645[0]), vaddq_u32(t647[1], t645[1])];
        let t650: [uint32x4_t; 2] = [vcltq_u32(t649[0], t647[0]), vcltq_u32(t649[1], t647[1])];
        let t651: [uint32x4_t; 4] = [t649[0], t649[1], vsubq_u32(vaddq_u32(t648[0], t645[2]), t650[0]), vsubq_u32(vaddq_u32(t648[1], t645[3]), t650[1])];
        let mut t652: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t651[2]) as u64) << 32 | vgetq_lane_u32::<0>(t651[0]) as u64) as usize as *const u64).read_unaligned(); t652[0] = vsetq_lane_u32::<0>(v as u32, t652[0]); t652[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t652[2]); }
        { let v = (((vgetq_lane_u32::<1>(t651[2]) as u64) << 32 | vgetq_lane_u32::<1>(t651[0]) as u64) as usize as *const u64).read_unaligned(); t652[0] = vsetq_lane_u32::<1>(v as u32, t652[0]); t652[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t652[2]); }
        { let v = (((vgetq_lane_u32::<2>(t651[2]) as u64) << 32 | vgetq_lane_u32::<2>(t651[0]) as u64) as usize as *const u64).read_unaligned(); t652[0] = vsetq_lane_u32::<2>(v as u32, t652[0]); t652[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t652[2]); }
        { let v = (((vgetq_lane_u32::<3>(t651[2]) as u64) << 32 | vgetq_lane_u32::<3>(t651[0]) as u64) as usize as *const u64).read_unaligned(); t652[0] = vsetq_lane_u32::<3>(v as u32, t652[0]); t652[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t652[2]); }
        { let v = (((vgetq_lane_u32::<0>(t651[3]) as u64) << 32 | vgetq_lane_u32::<0>(t651[1]) as u64) as usize as *const u64).read_unaligned(); t652[1] = vsetq_lane_u32::<0>(v as u32, t652[1]); t652[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t652[3]); }
        { let v = (((vgetq_lane_u32::<1>(t651[3]) as u64) << 32 | vgetq_lane_u32::<1>(t651[1]) as u64) as usize as *const u64).read_unaligned(); t652[1] = vsetq_lane_u32::<1>(v as u32, t652[1]); t652[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t652[3]); }
        { let v = (((vgetq_lane_u32::<2>(t651[3]) as u64) << 32 | vgetq_lane_u32::<2>(t651[1]) as u64) as usize as *const u64).read_unaligned(); t652[1] = vsetq_lane_u32::<2>(v as u32, t652[1]); t652[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t652[3]); }
        { let v = (((vgetq_lane_u32::<3>(t651[3]) as u64) << 32 | vgetq_lane_u32::<3>(t651[1]) as u64) as usize as *const u64).read_unaligned(); t652[1] = vsetq_lane_u32::<3>(v as u32, t652[1]); t652[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t652[3]); }
        res = t652;
        return res;
    }
    if ((x2) == (4u32)) {
        let t653: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
        let t654: [uint32x4_t; 4] = [vandq_u32(x1[0], t653[0]), vandq_u32(x1[1], t653[1]), vandq_u32(x1[2], t653[2]), vandq_u32(x1[3], t653[3])];
        let t655: u64 = ((arg0) as usize as u64);
        let t656 = [vdupq_n_u32((t655) as u32); 2];
        let t657 = [vdupq_n_u32(((t655) >> 32) as u32); 2];
        let t658: [uint32x4_t; 2] = [vaddq_u32(t656[0], t654[0]), vaddq_u32(t656[1], t654[1])];
        let t659: [uint32x4_t; 2] = [vcltq_u32(t658[0], t656[0]), vcltq_u32(t658[1], t656[1])];
        let t660: [uint32x4_t; 4] = [t658[0], t658[1], vsubq_u32(vaddq_u32(t657[0], t654[2]), t659[0]), vsubq_u32(vaddq_u32(t657[1], t654[3]), t659[1])];
        let mut t661: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t661[0] = vsetq_lane_u32::<0>((((vgetq_lane_u32::<0>(t660[2]) as u64) << 32 | vgetq_lane_u32::<0>(t660[0]) as u64) as usize as *const u32).read_unaligned() as u32, t661[0]);
        t661[0] = vsetq_lane_u32::<1>((((vgetq_lane_u32::<1>(t660[2]) as u64) << 32 | vgetq_lane_u32::<1>(t660[0]) as u64) as usize as *const u32).read_unaligned() as u32, t661[0]);
        t661[0] = vsetq_lane_u32::<2>((((vgetq_lane_u32::<2>(t660[2]) as u64) << 32 | vgetq_lane_u32::<2>(t660[0]) as u64) as usize as *const u32).read_unaligned() as u32, t661[0]);
        t661[0] = vsetq_lane_u32::<3>((((vgetq_lane_u32::<3>(t660[2]) as u64) << 32 | vgetq_lane_u32::<3>(t660[0]) as u64) as usize as *const u32).read_unaligned() as u32, t661[0]);
        t661[1] = vsetq_lane_u32::<0>((((vgetq_lane_u32::<0>(t660[3]) as u64) << 32 | vgetq_lane_u32::<0>(t660[1]) as u64) as usize as *const u32).read_unaligned() as u32, t661[1]);
        t661[1] = vsetq_lane_u32::<1>((((vgetq_lane_u32::<1>(t660[3]) as u64) << 32 | vgetq_lane_u32::<1>(t660[1]) as u64) as usize as *const u32).read_unaligned() as u32, t661[1]);
        t661[1] = vsetq_lane_u32::<2>((((vgetq_lane_u32::<2>(t660[3]) as u64) << 32 | vgetq_lane_u32::<2>(t660[1]) as u64) as usize as *const u32).read_unaligned() as u32, t661[1]);
        t661[1] = vsetq_lane_u32::<3>((((vgetq_lane_u32::<3>(t660[3]) as u64) << 32 | vgetq_lane_u32::<3>(t660[1]) as u64) as usize as *const u32).read_unaligned() as u32, t661[1]);
        let t662: [uint32x4_t; 4] = [t661[0], t661[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t662;
        return res;
    }
    if ((x2) == (2u32)) {
        let t663: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
        let t664: [uint32x4_t; 4] = [vandq_u32(x1[0], t663[0]), vandq_u32(x1[1], t663[1]), vandq_u32(x1[2], t663[2]), vandq_u32(x1[3], t663[3])];
        let t665: u64 = ((arg0) as usize as u64);
        let t666 = [vdupq_n_u32((t665) as u32); 2];
        let t667 = [vdupq_n_u32(((t665) >> 32) as u32); 2];
        let t668: [uint32x4_t; 2] = [vaddq_u32(t666[0], t664[0]), vaddq_u32(t666[1], t664[1])];
        let t669: [uint32x4_t; 2] = [vcltq_u32(t668[0], t666[0]), vcltq_u32(t668[1], t666[1])];
        let t670: [uint32x4_t; 4] = [t668[0], t668[1], vsubq_u32(vaddq_u32(t667[0], t664[2]), t669[0]), vsubq_u32(vaddq_u32(t667[1], t664[3]), t669[1])];
        let mut t671: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t671[0] = vsetq_lane_u32::<0>((((vgetq_lane_u32::<0>(t670[2]) as u64) << 32 | vgetq_lane_u32::<0>(t670[0]) as u64) as usize as *const u16).read_unaligned() as u32, t671[0]);
        t671[0] = vsetq_lane_u32::<1>((((vgetq_lane_u32::<1>(t670[2]) as u64) << 32 | vgetq_lane_u32::<1>(t670[0]) as u64) as usize as *const u16).read_unaligned() as u32, t671[0]);
        t671[0] = vsetq_lane_u32::<2>((((vgetq_lane_u32::<2>(t670[2]) as u64) << 32 | vgetq_lane_u32::<2>(t670[0]) as u64) as usize as *const u16).read_unaligned() as u32, t671[0]);
        t671[0] = vsetq_lane_u32::<3>((((vgetq_lane_u32::<3>(t670[2]) as u64) << 32 | vgetq_lane_u32::<3>(t670[0]) as u64) as usize as *const u16).read_unaligned() as u32, t671[0]);
        t671[1] = vsetq_lane_u32::<0>((((vgetq_lane_u32::<0>(t670[3]) as u64) << 32 | vgetq_lane_u32::<0>(t670[1]) as u64) as usize as *const u16).read_unaligned() as u32, t671[1]);
        t671[1] = vsetq_lane_u32::<1>((((vgetq_lane_u32::<1>(t670[3]) as u64) << 32 | vgetq_lane_u32::<1>(t670[1]) as u64) as usize as *const u16).read_unaligned() as u32, t671[1]);
        t671[1] = vsetq_lane_u32::<2>((((vgetq_lane_u32::<2>(t670[3]) as u64) << 32 | vgetq_lane_u32::<2>(t670[1]) as u64) as usize as *const u16).read_unaligned() as u32, t671[1]);
        t671[1] = vsetq_lane_u32::<3>((((vgetq_lane_u32::<3>(t670[3]) as u64) << 32 | vgetq_lane_u32::<3>(t670[1]) as u64) as usize as *const u16).read_unaligned() as u32, t671[1]);
        let t672: [uint32x4_t; 4] = [t671[0], t671[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t672;
        return res;
    }
    if ((x2) == (1u32)) {
        let t673: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
        let t674: [uint32x4_t; 4] = [vandq_u32(x1[0], t673[0]), vandq_u32(x1[1], t673[1]), vandq_u32(x1[2], t673[2]), vandq_u32(x1[3], t673[3])];
        let t675: u64 = ((arg0) as usize as u64);
        let t676 = [vdupq_n_u32((t675) as u32); 2];
        let t677 = [vdupq_n_u32(((t675) >> 32) as u32); 2];
        let t678: [uint32x4_t; 2] = [vaddq_u32(t676[0], t674[0]), vaddq_u32(t676[1], t674[1])];
        let t679: [uint32x4_t; 2] = [vcltq_u32(t678[0], t676[0]), vcltq_u32(t678[1], t676[1])];
        let t680: [uint32x4_t; 4] = [t678[0], t678[1], vsubq_u32(vaddq_u32(t677[0], t674[2]), t679[0]), vsubq_u32(vaddq_u32(t677[1], t674[3]), t679[1])];
        let mut t681: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t681[0] = vsetq_lane_u32::<0>((((vgetq_lane_u32::<0>(t680[2]) as u64) << 32 | vgetq_lane_u32::<0>(t680[0]) as u64) as usize as *const u8).read_unaligned() as u32, t681[0]);
        t681[0] = vsetq_lane_u32::<1>((((vgetq_lane_u32::<1>(t680[2]) as u64) << 32 | vgetq_lane_u32::<1>(t680[0]) as u64) as usize as *const u8).read_unaligned() as u32, t681[0]);
        t681[0] = vsetq_lane_u32::<2>((((vgetq_lane_u32::<2>(t680[2]) as u64) << 32 | vgetq_lane_u32::<2>(t680[0]) as u64) as usize as *const u8).read_unaligned() as u32, t681[0]);
        t681[0] = vsetq_lane_u32::<3>((((vgetq_lane_u32::<3>(t680[2]) as u64) << 32 | vgetq_lane_u32::<3>(t680[0]) as u64) as usize as *const u8).read_unaligned() as u32, t681[0]);
        t681[1] = vsetq_lane_u32::<0>((((vgetq_lane_u32::<0>(t680[3]) as u64) << 32 | vgetq_lane_u32::<0>(t680[1]) as u64) as usize as *const u8).read_unaligned() as u32, t681[1]);
        t681[1] = vsetq_lane_u32::<1>((((vgetq_lane_u32::<1>(t680[3]) as u64) << 32 | vgetq_lane_u32::<1>(t680[1]) as u64) as usize as *const u8).read_unaligned() as u32, t681[1]);
        t681[1] = vsetq_lane_u32::<2>((((vgetq_lane_u32::<2>(t680[3]) as u64) << 32 | vgetq_lane_u32::<2>(t680[1]) as u64) as usize as *const u8).read_unaligned() as u32, t681[1]);
        t681[1] = vsetq_lane_u32::<3>((((vgetq_lane_u32::<3>(t680[3]) as u64) << 32 | vgetq_lane_u32::<3>(t680[1]) as u64) as usize as *const u8).read_unaligned() as u32, t681[1]);
        let t682: [uint32x4_t; 4] = [t681[0], t681[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t682;
        return res;
    }
    let t683: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
    let t684: [uint32x4_t; 4] = [vandq_u32(x1[0], t683[0]), vandq_u32(x1[1], t683[1]), vandq_u32(x1[2], t683[2]), vandq_u32(x1[3], t683[3])];
    let t685: u64 = ((arg0) as usize as u64);
    let t686 = [vdupq_n_u32((t685) as u32); 2];
    let t687 = [vdupq_n_u32(((t685) >> 32) as u32); 2];
    let t688: [uint32x4_t; 2] = [vaddq_u32(t686[0], t684[0]), vaddq_u32(t686[1], t684[1])];
    let t689: [uint32x4_t; 2] = [vcltq_u32(t688[0], t686[0]), vcltq_u32(t688[1], t686[1])];
    let t690: [uint32x4_t; 4] = [t688[0], t688[1], vsubq_u32(vaddq_u32(t687[0], t684[2]), t689[0]), vsubq_u32(vaddq_u32(t687[1], t684[3]), t689[1])];
    let t691: u64 = ((arg0) as usize as u64);
    let t692 = [vdupq_n_u32((t691) as u32); 2];
    let t693 = [vdupq_n_u32(((t691) >> 32) as u32); 2];
    let t694: [uint32x4_t; 2] = [vaddq_u32(t692[0], x1[0]), vaddq_u32(t692[1], x1[1])];
    let t695: [uint32x4_t; 2] = [vcltq_u32(t694[0], t692[0]), vcltq_u32(t694[1], t692[1])];
    let t696: [uint32x4_t; 4] = [t694[0], t694[1], vsubq_u32(vaddq_u32(t693[0], x1[2]), t695[0]), vsubq_u32(vaddq_u32(t693[1], x1[3]), t695[1])];
    let t697: u64 = 7u64;
    let t698 = [vdupq_n_u32((t697) as u32); 2];
    let t699: [uint32x4_t; 2] = [vandq_u32(t696[0], t698[0]), vandq_u32(t696[1], t698[1])];
    let t700: [uint32x4_t; 2] = [vcltq_u32(t690[0], t699[0]), vcltq_u32(t690[1], t699[1])];
    let t701: [uint32x4_t; 4] = [vsubq_u32(t690[0], t699[0]), vsubq_u32(t690[1], t699[1]), vaddq_u32(vsubq_u32(t690[2], vdupq_n_u32(0)), t700[0]), vaddq_u32(vsubq_u32(t690[3], vdupq_n_u32(0)), t700[1])];
    x3 = t701;
    let t702: u64 = ((arg0) as usize as u64);
    let t703 = [vdupq_n_u32((t702) as u32); 2];
    let t704 = [vdupq_n_u32(((t702) >> 32) as u32); 2];
    let t705: [uint32x4_t; 2] = [vaddq_u32(t703[0], x1[0]), vaddq_u32(t703[1], x1[1])];
    let t706: [uint32x4_t; 2] = [vcltq_u32(t705[0], t703[0]), vcltq_u32(t705[1], t703[1])];
    let t707: [uint32x4_t; 4] = [t705[0], t705[1], vsubq_u32(vaddq_u32(t704[0], x1[2]), t706[0]), vsubq_u32(vaddq_u32(t704[1], x1[3]), t706[1])];
    let t708: u64 = 7u64;
    let t709 = [vdupq_n_u32((t708) as u32); 2];
    let t710: [uint32x4_t; 2] = [vandq_u32(t707[0], t709[0]), vandq_u32(t707[1], t709[1])];
    let t711 = [vdupq_n_u32(8u32); 2];
    let t712: [uint32x4_t; 2] = [vmulq_u32(t710[0], t711[0]), vmulq_u32(t710[1], t711[1])];
    x4 = t712;
    let mut t713: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x3[2]) as u64) << 32 | vgetq_lane_u32::<0>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t713[0] = vsetq_lane_u32::<0>(v as u32, t713[0]); t713[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t713[2]); }
    { let v = (((vgetq_lane_u32::<1>(x3[2]) as u64) << 32 | vgetq_lane_u32::<1>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t713[0] = vsetq_lane_u32::<1>(v as u32, t713[0]); t713[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t713[2]); }
    { let v = (((vgetq_lane_u32::<2>(x3[2]) as u64) << 32 | vgetq_lane_u32::<2>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t713[0] = vsetq_lane_u32::<2>(v as u32, t713[0]); t713[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t713[2]); }
    { let v = (((vgetq_lane_u32::<3>(x3[2]) as u64) << 32 | vgetq_lane_u32::<3>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t713[0] = vsetq_lane_u32::<3>(v as u32, t713[0]); t713[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t713[2]); }
    { let v = (((vgetq_lane_u32::<0>(x3[3]) as u64) << 32 | vgetq_lane_u32::<0>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t713[1] = vsetq_lane_u32::<0>(v as u32, t713[1]); t713[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t713[3]); }
    { let v = (((vgetq_lane_u32::<1>(x3[3]) as u64) << 32 | vgetq_lane_u32::<1>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t713[1] = vsetq_lane_u32::<1>(v as u32, t713[1]); t713[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t713[3]); }
    { let v = (((vgetq_lane_u32::<2>(x3[3]) as u64) << 32 | vgetq_lane_u32::<2>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t713[1] = vsetq_lane_u32::<2>(v as u32, t713[1]); t713[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t713[3]); }
    { let v = (((vgetq_lane_u32::<3>(x3[3]) as u64) << 32 | vgetq_lane_u32::<3>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t713[1] = vsetq_lane_u32::<3>(v as u32, t713[1]); t713[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t713[3]); }
    let t714: [uint32x4_t; 2] = [x4[0], x4[1]];
    let t715: [uint32x4_t; 2] = [vandq_u32(t714[0], vdupq_n_u32(63)), vandq_u32(t714[1], vdupq_n_u32(63))];
    let t716: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t713[0], vnegq_s32(vreinterpretq_s32_u32(t715[0]))), vshlq_u32(t713[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t715[0])))), vorrq_u32(vshlq_u32(t713[1], vnegq_s32(vreinterpretq_s32_u32(t715[1]))), vshlq_u32(t713[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t715[1])))), vshlq_u32(t713[2], vnegq_s32(vreinterpretq_s32_u32(t715[0]))), vshlq_u32(t713[3], vnegq_s32(vreinterpretq_s32_u32(t715[1])))];
    x5 = t716;
    let t717 = [vdupq_n_u32(0u32); 2];
    let t718: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t717[0])), vmvnq_u32(vceqq_u32(x4[1], t717[1]))];
    let t719 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
    let t720: [uint32x4_t; 2] = [vaddq_u32(x4[0], t719[0]), vaddq_u32(x4[1], t719[1])];
    let t721 = [vdupq_n_u32(64u32); 2];
    let t722: [uint32x4_t; 2] = [vcgtq_u32(t720[0], t721[0]), vcgtq_u32(t720[1], t721[1])];
    let t723: [uint32x4_t; 2] = [vandq_u32(t718[0], t722[0]), vandq_u32(t718[1], t722[1])];
    let mut t724: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t725: [uint32x4_t; 2] = [vandq_u32(live[0], t723[0]), vandq_u32(live[1], t723[1])];
    let t726: [uint32x4_t; 2] = [vbicq_u32(live[0], t723[0]), vbicq_u32(live[1], t723[1])];
    if any(&t725) {
        let t727: u64 = (1u64).wrapping_mul(8u64);
        let t728 = [vdupq_n_u32((t727) as u32); 2];
        let t729 = [vdupq_n_u32(((t727) >> 32) as u32); 2];
        let t730: [uint32x4_t; 2] = [vaddq_u32(x3[0], t728[0]), vaddq_u32(x3[1], t728[1])];
        let t731: [uint32x4_t; 2] = [vcltq_u32(t730[0], x3[0]), vcltq_u32(t730[1], x3[1])];
        let t732: [uint32x4_t; 4] = [t730[0], t730[1], vsubq_u32(vaddq_u32(x3[2], t729[0]), t731[0]), vsubq_u32(vaddq_u32(x3[3], t729[1]), t731[1])];
        let mut t733: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t732[2]) as u64) << 32 | vgetq_lane_u32::<0>(t732[0]) as u64) as usize as *const u64).read_unaligned(); t733[0] = vsetq_lane_u32::<0>(v as u32, t733[0]); t733[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t733[2]); }
        { let v = (((vgetq_lane_u32::<1>(t732[2]) as u64) << 32 | vgetq_lane_u32::<1>(t732[0]) as u64) as usize as *const u64).read_unaligned(); t733[0] = vsetq_lane_u32::<1>(v as u32, t733[0]); t733[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t733[2]); }
        { let v = (((vgetq_lane_u32::<2>(t732[2]) as u64) << 32 | vgetq_lane_u32::<2>(t732[0]) as u64) as usize as *const u64).read_unaligned(); t733[0] = vsetq_lane_u32::<2>(v as u32, t733[0]); t733[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t733[2]); }
        { let v = (((vgetq_lane_u32::<3>(t732[2]) as u64) << 32 | vgetq_lane_u32::<3>(t732[0]) as u64) as usize as *const u64).read_unaligned(); t733[0] = vsetq_lane_u32::<3>(v as u32, t733[0]); t733[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t733[2]); }
        { let v = (((vgetq_lane_u32::<0>(t732[3]) as u64) << 32 | vgetq_lane_u32::<0>(t732[1]) as u64) as usize as *const u64).read_unaligned(); t733[1] = vsetq_lane_u32::<0>(v as u32, t733[1]); t733[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t733[3]); }
        { let v = (((vgetq_lane_u32::<1>(t732[3]) as u64) << 32 | vgetq_lane_u32::<1>(t732[1]) as u64) as usize as *const u64).read_unaligned(); t733[1] = vsetq_lane_u32::<1>(v as u32, t733[1]); t733[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t733[3]); }
        { let v = (((vgetq_lane_u32::<2>(t732[3]) as u64) << 32 | vgetq_lane_u32::<2>(t732[1]) as u64) as usize as *const u64).read_unaligned(); t733[1] = vsetq_lane_u32::<2>(v as u32, t733[1]); t733[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t733[3]); }
        { let v = (((vgetq_lane_u32::<3>(t732[3]) as u64) << 32 | vgetq_lane_u32::<3>(t732[1]) as u64) as usize as *const u64).read_unaligned(); t733[1] = vsetq_lane_u32::<3>(v as u32, t733[1]); t733[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t733[3]); }
        let t734: [uint32x4_t; 2] = [vsubq_u32(t721[0], x4[0]), vsubq_u32(t721[1], x4[1])];
        let t735: [uint32x4_t; 2] = [t734[0], t734[1]];
        let t736: [uint32x4_t; 2] = [vandq_u32(t735[0], vdupq_n_u32(63)), vandq_u32(t735[1], vdupq_n_u32(63))];
        let t737: [uint32x4_t; 4] = [vshlq_u32(t733[0], vreinterpretq_s32_u32(t736[0])), vshlq_u32(t733[1], vreinterpretq_s32_u32(t736[1])), vorrq_u32(vshlq_u32(t733[2], vreinterpretq_s32_u32(t736[0])), vshlq_u32(t733[0], vsubq_s32(vreinterpretq_s32_u32(t736[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t733[3], vreinterpretq_s32_u32(t736[1])), vshlq_u32(t733[1], vsubq_s32(vreinterpretq_s32_u32(t736[1]), vdupq_n_s32(32))))];
        let t738: [uint32x4_t; 4] = [vorrq_u32(x5[0], t737[0]), vorrq_u32(x5[1], t737[1]), vorrq_u32(x5[2], t737[2]), vorrq_u32(x5[3], t737[3])];
        t724 = [vbslq_u32(t725[0], t738[0], t724[0]), vbslq_u32(t725[1], t738[1], t724[1]), vbslq_u32(t725[0], t738[2], t724[2]), vbslq_u32(t725[1], t738[3], t724[3])];
    }
    {
        t724 = [vbslq_u32(t726[0], x5[0], t724[0]), vbslq_u32(t726[1], x5[1], t724[1]), vbslq_u32(t726[0], x5[2], t724[2]), vbslq_u32(t726[1], x5[3], t724[3])];
    }
    res = t724;
    return res;
}

#[inline(always)]
unsafe fn b17_r0ls_f(l: &Launch, lid0: u32, arg0: *const u8, x1: [uint32x4_t; 4], x2: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x3: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x4: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x5: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if ((x2) == (8u32)) {
        let t739: u64 = ((arg0) as usize as u64);
        let t740 = [vdupq_n_u32((t739) as u32); 2];
        let t741 = [vdupq_n_u32(((t739) >> 32) as u32); 2];
        let t742: [uint32x4_t; 2] = [vaddq_u32(t740[0], x1[0]), vaddq_u32(t740[1], x1[1])];
        let t743: [uint32x4_t; 2] = [vcltq_u32(t742[0], t740[0]), vcltq_u32(t742[1], t740[1])];
        let t744: [uint32x4_t; 4] = [t742[0], t742[1], vsubq_u32(vaddq_u32(t741[0], x1[2]), t743[0]), vsubq_u32(vaddq_u32(t741[1], x1[3]), t743[1])];
        let mut t745: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t744[2]) as u64) << 32 | vgetq_lane_u32::<0>(t744[0]) as u64) as usize as *const u64).read_unaligned(); t745[0] = vsetq_lane_u32::<0>(v as u32, t745[0]); t745[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t745[2]); }
        { let v = (((vgetq_lane_u32::<1>(t744[2]) as u64) << 32 | vgetq_lane_u32::<1>(t744[0]) as u64) as usize as *const u64).read_unaligned(); t745[0] = vsetq_lane_u32::<1>(v as u32, t745[0]); t745[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t745[2]); }
        { let v = (((vgetq_lane_u32::<2>(t744[2]) as u64) << 32 | vgetq_lane_u32::<2>(t744[0]) as u64) as usize as *const u64).read_unaligned(); t745[0] = vsetq_lane_u32::<2>(v as u32, t745[0]); t745[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t745[2]); }
        { let v = (((vgetq_lane_u32::<3>(t744[2]) as u64) << 32 | vgetq_lane_u32::<3>(t744[0]) as u64) as usize as *const u64).read_unaligned(); t745[0] = vsetq_lane_u32::<3>(v as u32, t745[0]); t745[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t745[2]); }
        { let v = (((vgetq_lane_u32::<0>(t744[3]) as u64) << 32 | vgetq_lane_u32::<0>(t744[1]) as u64) as usize as *const u64).read_unaligned(); t745[1] = vsetq_lane_u32::<0>(v as u32, t745[1]); t745[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t745[3]); }
        { let v = (((vgetq_lane_u32::<1>(t744[3]) as u64) << 32 | vgetq_lane_u32::<1>(t744[1]) as u64) as usize as *const u64).read_unaligned(); t745[1] = vsetq_lane_u32::<1>(v as u32, t745[1]); t745[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t745[3]); }
        { let v = (((vgetq_lane_u32::<2>(t744[3]) as u64) << 32 | vgetq_lane_u32::<2>(t744[1]) as u64) as usize as *const u64).read_unaligned(); t745[1] = vsetq_lane_u32::<2>(v as u32, t745[1]); t745[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t745[3]); }
        { let v = (((vgetq_lane_u32::<3>(t744[3]) as u64) << 32 | vgetq_lane_u32::<3>(t744[1]) as u64) as usize as *const u64).read_unaligned(); t745[1] = vsetq_lane_u32::<3>(v as u32, t745[1]); t745[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t745[3]); }
        res = t745;
        return res;
    }
    if ((x2) == (4u32)) {
        let t746: u64 = ((arg0) as usize as u64);
        let t747 = [vdupq_n_u32((t746) as u32); 2];
        let t748 = [vdupq_n_u32(((t746) >> 32) as u32); 2];
        let t749: [uint32x4_t; 2] = [vaddq_u32(t747[0], x1[0]), vaddq_u32(t747[1], x1[1])];
        let t750: [uint32x4_t; 2] = [vcltq_u32(t749[0], t747[0]), vcltq_u32(t749[1], t747[1])];
        let t751: [uint32x4_t; 4] = [t749[0], t749[1], vsubq_u32(vaddq_u32(t748[0], x1[2]), t750[0]), vsubq_u32(vaddq_u32(t748[1], x1[3]), t750[1])];
        let mut t752: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t752[0] = vsetq_lane_u32::<0>((((vgetq_lane_u32::<0>(t751[2]) as u64) << 32 | vgetq_lane_u32::<0>(t751[0]) as u64) as usize as *const u32).read_unaligned() as u32, t752[0]);
        t752[0] = vsetq_lane_u32::<1>((((vgetq_lane_u32::<1>(t751[2]) as u64) << 32 | vgetq_lane_u32::<1>(t751[0]) as u64) as usize as *const u32).read_unaligned() as u32, t752[0]);
        t752[0] = vsetq_lane_u32::<2>((((vgetq_lane_u32::<2>(t751[2]) as u64) << 32 | vgetq_lane_u32::<2>(t751[0]) as u64) as usize as *const u32).read_unaligned() as u32, t752[0]);
        t752[0] = vsetq_lane_u32::<3>((((vgetq_lane_u32::<3>(t751[2]) as u64) << 32 | vgetq_lane_u32::<3>(t751[0]) as u64) as usize as *const u32).read_unaligned() as u32, t752[0]);
        t752[1] = vsetq_lane_u32::<0>((((vgetq_lane_u32::<0>(t751[3]) as u64) << 32 | vgetq_lane_u32::<0>(t751[1]) as u64) as usize as *const u32).read_unaligned() as u32, t752[1]);
        t752[1] = vsetq_lane_u32::<1>((((vgetq_lane_u32::<1>(t751[3]) as u64) << 32 | vgetq_lane_u32::<1>(t751[1]) as u64) as usize as *const u32).read_unaligned() as u32, t752[1]);
        t752[1] = vsetq_lane_u32::<2>((((vgetq_lane_u32::<2>(t751[3]) as u64) << 32 | vgetq_lane_u32::<2>(t751[1]) as u64) as usize as *const u32).read_unaligned() as u32, t752[1]);
        t752[1] = vsetq_lane_u32::<3>((((vgetq_lane_u32::<3>(t751[3]) as u64) << 32 | vgetq_lane_u32::<3>(t751[1]) as u64) as usize as *const u32).read_unaligned() as u32, t752[1]);
        let t753: [uint32x4_t; 4] = [t752[0], t752[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t753;
        return res;
    }
    if ((x2) == (2u32)) {
        let t754: u64 = ((arg0) as usize as u64);
        let t755 = [vdupq_n_u32((t754) as u32); 2];
        let t756 = [vdupq_n_u32(((t754) >> 32) as u32); 2];
        let t757: [uint32x4_t; 2] = [vaddq_u32(t755[0], x1[0]), vaddq_u32(t755[1], x1[1])];
        let t758: [uint32x4_t; 2] = [vcltq_u32(t757[0], t755[0]), vcltq_u32(t757[1], t755[1])];
        let t759: [uint32x4_t; 4] = [t757[0], t757[1], vsubq_u32(vaddq_u32(t756[0], x1[2]), t758[0]), vsubq_u32(vaddq_u32(t756[1], x1[3]), t758[1])];
        let mut t760: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t760[0] = vsetq_lane_u32::<0>((((vgetq_lane_u32::<0>(t759[2]) as u64) << 32 | vgetq_lane_u32::<0>(t759[0]) as u64) as usize as *const u16).read_unaligned() as u32, t760[0]);
        t760[0] = vsetq_lane_u32::<1>((((vgetq_lane_u32::<1>(t759[2]) as u64) << 32 | vgetq_lane_u32::<1>(t759[0]) as u64) as usize as *const u16).read_unaligned() as u32, t760[0]);
        t760[0] = vsetq_lane_u32::<2>((((vgetq_lane_u32::<2>(t759[2]) as u64) << 32 | vgetq_lane_u32::<2>(t759[0]) as u64) as usize as *const u16).read_unaligned() as u32, t760[0]);
        t760[0] = vsetq_lane_u32::<3>((((vgetq_lane_u32::<3>(t759[2]) as u64) << 32 | vgetq_lane_u32::<3>(t759[0]) as u64) as usize as *const u16).read_unaligned() as u32, t760[0]);
        t760[1] = vsetq_lane_u32::<0>((((vgetq_lane_u32::<0>(t759[3]) as u64) << 32 | vgetq_lane_u32::<0>(t759[1]) as u64) as usize as *const u16).read_unaligned() as u32, t760[1]);
        t760[1] = vsetq_lane_u32::<1>((((vgetq_lane_u32::<1>(t759[3]) as u64) << 32 | vgetq_lane_u32::<1>(t759[1]) as u64) as usize as *const u16).read_unaligned() as u32, t760[1]);
        t760[1] = vsetq_lane_u32::<2>((((vgetq_lane_u32::<2>(t759[3]) as u64) << 32 | vgetq_lane_u32::<2>(t759[1]) as u64) as usize as *const u16).read_unaligned() as u32, t760[1]);
        t760[1] = vsetq_lane_u32::<3>((((vgetq_lane_u32::<3>(t759[3]) as u64) << 32 | vgetq_lane_u32::<3>(t759[1]) as u64) as usize as *const u16).read_unaligned() as u32, t760[1]);
        let t761: [uint32x4_t; 4] = [t760[0], t760[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t761;
        return res;
    }
    if ((x2) == (1u32)) {
        let t762: u64 = ((arg0) as usize as u64);
        let t763 = [vdupq_n_u32((t762) as u32); 2];
        let t764 = [vdupq_n_u32(((t762) >> 32) as u32); 2];
        let t765: [uint32x4_t; 2] = [vaddq_u32(t763[0], x1[0]), vaddq_u32(t763[1], x1[1])];
        let t766: [uint32x4_t; 2] = [vcltq_u32(t765[0], t763[0]), vcltq_u32(t765[1], t763[1])];
        let t767: [uint32x4_t; 4] = [t765[0], t765[1], vsubq_u32(vaddq_u32(t764[0], x1[2]), t766[0]), vsubq_u32(vaddq_u32(t764[1], x1[3]), t766[1])];
        let mut t768: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t768[0] = vsetq_lane_u32::<0>((((vgetq_lane_u32::<0>(t767[2]) as u64) << 32 | vgetq_lane_u32::<0>(t767[0]) as u64) as usize as *const u8).read_unaligned() as u32, t768[0]);
        t768[0] = vsetq_lane_u32::<1>((((vgetq_lane_u32::<1>(t767[2]) as u64) << 32 | vgetq_lane_u32::<1>(t767[0]) as u64) as usize as *const u8).read_unaligned() as u32, t768[0]);
        t768[0] = vsetq_lane_u32::<2>((((vgetq_lane_u32::<2>(t767[2]) as u64) << 32 | vgetq_lane_u32::<2>(t767[0]) as u64) as usize as *const u8).read_unaligned() as u32, t768[0]);
        t768[0] = vsetq_lane_u32::<3>((((vgetq_lane_u32::<3>(t767[2]) as u64) << 32 | vgetq_lane_u32::<3>(t767[0]) as u64) as usize as *const u8).read_unaligned() as u32, t768[0]);
        t768[1] = vsetq_lane_u32::<0>((((vgetq_lane_u32::<0>(t767[3]) as u64) << 32 | vgetq_lane_u32::<0>(t767[1]) as u64) as usize as *const u8).read_unaligned() as u32, t768[1]);
        t768[1] = vsetq_lane_u32::<1>((((vgetq_lane_u32::<1>(t767[3]) as u64) << 32 | vgetq_lane_u32::<1>(t767[1]) as u64) as usize as *const u8).read_unaligned() as u32, t768[1]);
        t768[1] = vsetq_lane_u32::<2>((((vgetq_lane_u32::<2>(t767[3]) as u64) << 32 | vgetq_lane_u32::<2>(t767[1]) as u64) as usize as *const u8).read_unaligned() as u32, t768[1]);
        t768[1] = vsetq_lane_u32::<3>((((vgetq_lane_u32::<3>(t767[3]) as u64) << 32 | vgetq_lane_u32::<3>(t767[1]) as u64) as usize as *const u8).read_unaligned() as u32, t768[1]);
        let t769: [uint32x4_t; 4] = [t768[0], t768[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t769;
        return res;
    }
    let t770: u64 = ((arg0) as usize as u64);
    let t771 = [vdupq_n_u32((t770) as u32); 2];
    let t772 = [vdupq_n_u32(((t770) >> 32) as u32); 2];
    let t773: [uint32x4_t; 2] = [vaddq_u32(t771[0], x1[0]), vaddq_u32(t771[1], x1[1])];
    let t774: [uint32x4_t; 2] = [vcltq_u32(t773[0], t771[0]), vcltq_u32(t773[1], t771[1])];
    let t775: [uint32x4_t; 4] = [t773[0], t773[1], vsubq_u32(vaddq_u32(t772[0], x1[2]), t774[0]), vsubq_u32(vaddq_u32(t772[1], x1[3]), t774[1])];
    let t776: u64 = ((arg0) as usize as u64);
    let t777 = [vdupq_n_u32((t776) as u32); 2];
    let t778 = [vdupq_n_u32(((t776) >> 32) as u32); 2];
    let t779: [uint32x4_t; 2] = [vaddq_u32(t777[0], x1[0]), vaddq_u32(t777[1], x1[1])];
    let t780: [uint32x4_t; 2] = [vcltq_u32(t779[0], t777[0]), vcltq_u32(t779[1], t777[1])];
    let t781: [uint32x4_t; 4] = [t779[0], t779[1], vsubq_u32(vaddq_u32(t778[0], x1[2]), t780[0]), vsubq_u32(vaddq_u32(t778[1], x1[3]), t780[1])];
    let t782: u64 = 7u64;
    let t783 = [vdupq_n_u32((t782) as u32); 2];
    let t784: [uint32x4_t; 2] = [vandq_u32(t781[0], t783[0]), vandq_u32(t781[1], t783[1])];
    let t785: [uint32x4_t; 2] = [vcltq_u32(t775[0], t784[0]), vcltq_u32(t775[1], t784[1])];
    let t786: [uint32x4_t; 4] = [vsubq_u32(t775[0], t784[0]), vsubq_u32(t775[1], t784[1]), vaddq_u32(vsubq_u32(t775[2], vdupq_n_u32(0)), t785[0]), vaddq_u32(vsubq_u32(t775[3], vdupq_n_u32(0)), t785[1])];
    x3 = t786;
    let t787: u64 = ((arg0) as usize as u64);
    let t788 = [vdupq_n_u32((t787) as u32); 2];
    let t789 = [vdupq_n_u32(((t787) >> 32) as u32); 2];
    let t790: [uint32x4_t; 2] = [vaddq_u32(t788[0], x1[0]), vaddq_u32(t788[1], x1[1])];
    let t791: [uint32x4_t; 2] = [vcltq_u32(t790[0], t788[0]), vcltq_u32(t790[1], t788[1])];
    let t792: [uint32x4_t; 4] = [t790[0], t790[1], vsubq_u32(vaddq_u32(t789[0], x1[2]), t791[0]), vsubq_u32(vaddq_u32(t789[1], x1[3]), t791[1])];
    let t793: u64 = 7u64;
    let t794 = [vdupq_n_u32((t793) as u32); 2];
    let t795: [uint32x4_t; 2] = [vandq_u32(t792[0], t794[0]), vandq_u32(t792[1], t794[1])];
    let t796 = [vdupq_n_u32(8u32); 2];
    let t797: [uint32x4_t; 2] = [vmulq_u32(t795[0], t796[0]), vmulq_u32(t795[1], t796[1])];
    x4 = t797;
    let mut t798: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x3[2]) as u64) << 32 | vgetq_lane_u32::<0>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t798[0] = vsetq_lane_u32::<0>(v as u32, t798[0]); t798[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t798[2]); }
    { let v = (((vgetq_lane_u32::<1>(x3[2]) as u64) << 32 | vgetq_lane_u32::<1>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t798[0] = vsetq_lane_u32::<1>(v as u32, t798[0]); t798[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t798[2]); }
    { let v = (((vgetq_lane_u32::<2>(x3[2]) as u64) << 32 | vgetq_lane_u32::<2>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t798[0] = vsetq_lane_u32::<2>(v as u32, t798[0]); t798[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t798[2]); }
    { let v = (((vgetq_lane_u32::<3>(x3[2]) as u64) << 32 | vgetq_lane_u32::<3>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t798[0] = vsetq_lane_u32::<3>(v as u32, t798[0]); t798[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t798[2]); }
    { let v = (((vgetq_lane_u32::<0>(x3[3]) as u64) << 32 | vgetq_lane_u32::<0>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t798[1] = vsetq_lane_u32::<0>(v as u32, t798[1]); t798[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t798[3]); }
    { let v = (((vgetq_lane_u32::<1>(x3[3]) as u64) << 32 | vgetq_lane_u32::<1>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t798[1] = vsetq_lane_u32::<1>(v as u32, t798[1]); t798[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t798[3]); }
    { let v = (((vgetq_lane_u32::<2>(x3[3]) as u64) << 32 | vgetq_lane_u32::<2>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t798[1] = vsetq_lane_u32::<2>(v as u32, t798[1]); t798[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t798[3]); }
    { let v = (((vgetq_lane_u32::<3>(x3[3]) as u64) << 32 | vgetq_lane_u32::<3>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t798[1] = vsetq_lane_u32::<3>(v as u32, t798[1]); t798[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t798[3]); }
    let t799: [uint32x4_t; 2] = [x4[0], x4[1]];
    let t800: [uint32x4_t; 2] = [vandq_u32(t799[0], vdupq_n_u32(63)), vandq_u32(t799[1], vdupq_n_u32(63))];
    let t801: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t798[0], vnegq_s32(vreinterpretq_s32_u32(t800[0]))), vshlq_u32(t798[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t800[0])))), vorrq_u32(vshlq_u32(t798[1], vnegq_s32(vreinterpretq_s32_u32(t800[1]))), vshlq_u32(t798[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t800[1])))), vshlq_u32(t798[2], vnegq_s32(vreinterpretq_s32_u32(t800[0]))), vshlq_u32(t798[3], vnegq_s32(vreinterpretq_s32_u32(t800[1])))];
    x5 = t801;
    let t802 = [vdupq_n_u32(0u32); 2];
    let t803: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t802[0])), vmvnq_u32(vceqq_u32(x4[1], t802[1]))];
    let t804 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
    let t805: [uint32x4_t; 2] = [vaddq_u32(x4[0], t804[0]), vaddq_u32(x4[1], t804[1])];
    let t806 = [vdupq_n_u32(64u32); 2];
    let t807: [uint32x4_t; 2] = [vcgtq_u32(t805[0], t806[0]), vcgtq_u32(t805[1], t806[1])];
    let t808: [uint32x4_t; 2] = [vandq_u32(t803[0], t807[0]), vandq_u32(t803[1], t807[1])];
    let mut t809: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t810: [uint32x4_t; 2] = [vmvnq_u32(t808[0]), vmvnq_u32(t808[1])];
    if any(&t808) {
        let t811: u64 = (1u64).wrapping_mul(8u64);
        let t812 = [vdupq_n_u32((t811) as u32); 2];
        let t813 = [vdupq_n_u32(((t811) >> 32) as u32); 2];
        let t814: [uint32x4_t; 2] = [vaddq_u32(x3[0], t812[0]), vaddq_u32(x3[1], t812[1])];
        let t815: [uint32x4_t; 2] = [vcltq_u32(t814[0], x3[0]), vcltq_u32(t814[1], x3[1])];
        let t816: [uint32x4_t; 4] = [t814[0], t814[1], vsubq_u32(vaddq_u32(x3[2], t813[0]), t815[0]), vsubq_u32(vaddq_u32(x3[3], t813[1]), t815[1])];
        let mut t817: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t816[2]) as u64) << 32 | vgetq_lane_u32::<0>(t816[0]) as u64) as usize as *const u64).read_unaligned(); t817[0] = vsetq_lane_u32::<0>(v as u32, t817[0]); t817[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t817[2]); }
        { let v = (((vgetq_lane_u32::<1>(t816[2]) as u64) << 32 | vgetq_lane_u32::<1>(t816[0]) as u64) as usize as *const u64).read_unaligned(); t817[0] = vsetq_lane_u32::<1>(v as u32, t817[0]); t817[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t817[2]); }
        { let v = (((vgetq_lane_u32::<2>(t816[2]) as u64) << 32 | vgetq_lane_u32::<2>(t816[0]) as u64) as usize as *const u64).read_unaligned(); t817[0] = vsetq_lane_u32::<2>(v as u32, t817[0]); t817[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t817[2]); }
        { let v = (((vgetq_lane_u32::<3>(t816[2]) as u64) << 32 | vgetq_lane_u32::<3>(t816[0]) as u64) as usize as *const u64).read_unaligned(); t817[0] = vsetq_lane_u32::<3>(v as u32, t817[0]); t817[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t817[2]); }
        { let v = (((vgetq_lane_u32::<0>(t816[3]) as u64) << 32 | vgetq_lane_u32::<0>(t816[1]) as u64) as usize as *const u64).read_unaligned(); t817[1] = vsetq_lane_u32::<0>(v as u32, t817[1]); t817[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t817[3]); }
        { let v = (((vgetq_lane_u32::<1>(t816[3]) as u64) << 32 | vgetq_lane_u32::<1>(t816[1]) as u64) as usize as *const u64).read_unaligned(); t817[1] = vsetq_lane_u32::<1>(v as u32, t817[1]); t817[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t817[3]); }
        { let v = (((vgetq_lane_u32::<2>(t816[3]) as u64) << 32 | vgetq_lane_u32::<2>(t816[1]) as u64) as usize as *const u64).read_unaligned(); t817[1] = vsetq_lane_u32::<2>(v as u32, t817[1]); t817[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t817[3]); }
        { let v = (((vgetq_lane_u32::<3>(t816[3]) as u64) << 32 | vgetq_lane_u32::<3>(t816[1]) as u64) as usize as *const u64).read_unaligned(); t817[1] = vsetq_lane_u32::<3>(v as u32, t817[1]); t817[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t817[3]); }
        let t818: [uint32x4_t; 2] = [vsubq_u32(t806[0], x4[0]), vsubq_u32(t806[1], x4[1])];
        let t819: [uint32x4_t; 2] = [t818[0], t818[1]];
        let t820: [uint32x4_t; 2] = [vandq_u32(t819[0], vdupq_n_u32(63)), vandq_u32(t819[1], vdupq_n_u32(63))];
        let t821: [uint32x4_t; 4] = [vshlq_u32(t817[0], vreinterpretq_s32_u32(t820[0])), vshlq_u32(t817[1], vreinterpretq_s32_u32(t820[1])), vorrq_u32(vshlq_u32(t817[2], vreinterpretq_s32_u32(t820[0])), vshlq_u32(t817[0], vsubq_s32(vreinterpretq_s32_u32(t820[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t817[3], vreinterpretq_s32_u32(t820[1])), vshlq_u32(t817[1], vsubq_s32(vreinterpretq_s32_u32(t820[1]), vdupq_n_s32(32))))];
        let t822: [uint32x4_t; 4] = [vorrq_u32(x5[0], t821[0]), vorrq_u32(x5[1], t821[1]), vorrq_u32(x5[2], t821[2]), vorrq_u32(x5[3], t821[3])];
        t809 = [vbslq_u32(t808[0], t822[0], t809[0]), vbslq_u32(t808[1], t822[1], t809[1]), vbslq_u32(t808[0], t822[2], t809[2]), vbslq_u32(t808[1], t822[3], t809[3])];
    }
    {
        t809 = [vbslq_u32(t810[0], x5[0], t809[0]), vbslq_u32(t810[1], x5[1], t809[1]), vbslq_u32(t810[0], x5[2], t809[2]), vbslq_u32(t810[1], x5[3], t809[3])];
    }
    res = t809;
    return res;
}

#[inline(always)]
unsafe fn b17_r0al1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1b: *const u8, x1i: [uint32x4_t; 2], x2: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x3: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x4: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x5: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if ((x2) == (8u32)) {
        let t823 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], x1i[0]), vandq_u32(live[1], x1i[1]))));
        let t824: [uint32x4_t; 2] = [vbslq_u32(live[0], x1i[0], t823), vbslq_u32(live[1], x1i[1], t823)];
        let mut t825: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t824[0]) as usize * 1) as *const u64).read_unaligned(); t825[0] = vsetq_lane_u32::<0>(v as u32, t825[0]); t825[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t825[2]); }
        { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t824[0]) as usize * 1) as *const u64).read_unaligned(); t825[0] = vsetq_lane_u32::<1>(v as u32, t825[0]); t825[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t825[2]); }
        { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t824[0]) as usize * 1) as *const u64).read_unaligned(); t825[0] = vsetq_lane_u32::<2>(v as u32, t825[0]); t825[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t825[2]); }
        { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t824[0]) as usize * 1) as *const u64).read_unaligned(); t825[0] = vsetq_lane_u32::<3>(v as u32, t825[0]); t825[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t825[2]); }
        { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t824[1]) as usize * 1) as *const u64).read_unaligned(); t825[1] = vsetq_lane_u32::<0>(v as u32, t825[1]); t825[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t825[3]); }
        { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t824[1]) as usize * 1) as *const u64).read_unaligned(); t825[1] = vsetq_lane_u32::<1>(v as u32, t825[1]); t825[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t825[3]); }
        { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t824[1]) as usize * 1) as *const u64).read_unaligned(); t825[1] = vsetq_lane_u32::<2>(v as u32, t825[1]); t825[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t825[3]); }
        { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t824[1]) as usize * 1) as *const u64).read_unaligned(); t825[1] = vsetq_lane_u32::<3>(v as u32, t825[1]); t825[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t825[3]); }
        res = t825;
        return res;
    }
    if ((x2) == (4u32)) {
        let t826 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], x1i[0]), vandq_u32(live[1], x1i[1]))));
        let t827: [uint32x4_t; 2] = [vbslq_u32(live[0], x1i[0], t826), vbslq_u32(live[1], x1i[1], t826)];
        let mut t828: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t828[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t827[0]) as usize * 1) as *const u32).read_unaligned() as u32), t828[0]);
        t828[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t827[0]) as usize * 1) as *const u32).read_unaligned() as u32), t828[0]);
        t828[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t827[0]) as usize * 1) as *const u32).read_unaligned() as u32), t828[0]);
        t828[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t827[0]) as usize * 1) as *const u32).read_unaligned() as u32), t828[0]);
        t828[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t827[1]) as usize * 1) as *const u32).read_unaligned() as u32), t828[1]);
        t828[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t827[1]) as usize * 1) as *const u32).read_unaligned() as u32), t828[1]);
        t828[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t827[1]) as usize * 1) as *const u32).read_unaligned() as u32), t828[1]);
        t828[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t827[1]) as usize * 1) as *const u32).read_unaligned() as u32), t828[1]);
        let t829: [uint32x4_t; 4] = [t828[0], t828[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t829;
        return res;
    }
    if ((x2) == (2u32)) {
        let t830 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], x1i[0]), vandq_u32(live[1], x1i[1]))));
        let t831: [uint32x4_t; 2] = [vbslq_u32(live[0], x1i[0], t830), vbslq_u32(live[1], x1i[1], t830)];
        let mut t832: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t832[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t831[0]) as usize * 1) as *const u16).read_unaligned() as u32), t832[0]);
        t832[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t831[0]) as usize * 1) as *const u16).read_unaligned() as u32), t832[0]);
        t832[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t831[0]) as usize * 1) as *const u16).read_unaligned() as u32), t832[0]);
        t832[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t831[0]) as usize * 1) as *const u16).read_unaligned() as u32), t832[0]);
        t832[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t831[1]) as usize * 1) as *const u16).read_unaligned() as u32), t832[1]);
        t832[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t831[1]) as usize * 1) as *const u16).read_unaligned() as u32), t832[1]);
        t832[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t831[1]) as usize * 1) as *const u16).read_unaligned() as u32), t832[1]);
        t832[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t831[1]) as usize * 1) as *const u16).read_unaligned() as u32), t832[1]);
        let t833: [uint32x4_t; 4] = [t832[0], t832[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t833;
        return res;
    }
    if ((x2) == (1u32)) {
        let t834 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], x1i[0]), vandq_u32(live[1], x1i[1]))));
        let t835: [uint32x4_t; 2] = [vbslq_u32(live[0], x1i[0], t834), vbslq_u32(live[1], x1i[1], t834)];
        let mut t836: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t836[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t835[0]) as usize * 1) as *const u8).read_unaligned() as u32), t836[0]);
        t836[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t835[0]) as usize * 1) as *const u8).read_unaligned() as u32), t836[0]);
        t836[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t835[0]) as usize * 1) as *const u8).read_unaligned() as u32), t836[0]);
        t836[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t835[0]) as usize * 1) as *const u8).read_unaligned() as u32), t836[0]);
        t836[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t835[1]) as usize * 1) as *const u8).read_unaligned() as u32), t836[1]);
        t836[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t835[1]) as usize * 1) as *const u8).read_unaligned() as u32), t836[1]);
        t836[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t835[1]) as usize * 1) as *const u8).read_unaligned() as u32), t836[1]);
        t836[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t835[1]) as usize * 1) as *const u8).read_unaligned() as u32), t836[1]);
        let t837: [uint32x4_t; 4] = [t836[0], t836[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t837;
        return res;
    }
    let t838: [uint32x4_t; 2] = [x1i[0], x1i[1]];
    let t839: u64 = (((x1b).wrapping_add((((arg0) as usize as u64)) as usize)) as usize as u64);
    let t840 = [vdupq_n_u32((t839) as u32); 2];
    let t841 = [vdupq_n_u32(((t839) >> 32) as u32); 2];
    let t842: [uint32x4_t; 2] = [vaddq_u32(t840[0], t838[0]), vaddq_u32(t840[1], t838[1])];
    let t843: [uint32x4_t; 2] = [vcltq_u32(t842[0], t840[0]), vcltq_u32(t842[1], t840[1])];
    let t844: [uint32x4_t; 4] = [t842[0], t842[1], vsubq_u32(vaddq_u32(t841[0], vdupq_n_u32(0)), t843[0]), vsubq_u32(vaddq_u32(t841[1], vdupq_n_u32(0)), t843[1])];
    let t845: u64 = 7u64;
    let t846 = [vdupq_n_u32((t845) as u32); 2];
    let t847: [uint32x4_t; 2] = [vandq_u32(t844[0], t846[0]), vandq_u32(t844[1], t846[1])];
    let t848: [uint32x4_t; 2] = [x1i[0], x1i[1]];
    let t849: u64 = (((arg0).wrapping_add((x1b) as usize)) as usize as u64);
    let t850 = [vdupq_n_u32((t849) as u32); 2];
    let t851 = [vdupq_n_u32(((t849) >> 32) as u32); 2];
    let t852: [uint32x4_t; 2] = [vaddq_u32(t850[0], t848[0]), vaddq_u32(t850[1], t848[1])];
    let t853: [uint32x4_t; 2] = [vcltq_u32(t852[0], t850[0]), vcltq_u32(t852[1], t850[1])];
    let t854: [uint32x4_t; 4] = [t852[0], t852[1], vsubq_u32(vaddq_u32(t851[0], vdupq_n_u32(0)), t853[0]), vsubq_u32(vaddq_u32(t851[1], vdupq_n_u32(0)), t853[1])];
    let t855: [uint32x4_t; 2] = [vcltq_u32(t854[0], t847[0]), vcltq_u32(t854[1], t847[1])];
    let t856: [uint32x4_t; 4] = [vsubq_u32(t854[0], t847[0]), vsubq_u32(t854[1], t847[1]), vaddq_u32(vsubq_u32(t854[2], vdupq_n_u32(0)), t855[0]), vaddq_u32(vsubq_u32(t854[3], vdupq_n_u32(0)), t855[1])];
    x3 = t856;
    let t857: [uint32x4_t; 2] = [x1i[0], x1i[1]];
    let t858: u64 = (((x1b).wrapping_add((((arg0) as usize as u64)) as usize)) as usize as u64);
    let t859 = [vdupq_n_u32((t858) as u32); 2];
    let t860 = [vdupq_n_u32(((t858) >> 32) as u32); 2];
    let t861: [uint32x4_t; 2] = [vaddq_u32(t859[0], t857[0]), vaddq_u32(t859[1], t857[1])];
    let t862: [uint32x4_t; 2] = [vcltq_u32(t861[0], t859[0]), vcltq_u32(t861[1], t859[1])];
    let t863: [uint32x4_t; 4] = [t861[0], t861[1], vsubq_u32(vaddq_u32(t860[0], vdupq_n_u32(0)), t862[0]), vsubq_u32(vaddq_u32(t860[1], vdupq_n_u32(0)), t862[1])];
    let t864: u64 = 7u64;
    let t865 = [vdupq_n_u32((t864) as u32); 2];
    let t866: [uint32x4_t; 2] = [vandq_u32(t863[0], t865[0]), vandq_u32(t863[1], t865[1])];
    let t867 = [vdupq_n_u32(8u32); 2];
    let t868: [uint32x4_t; 2] = [vmulq_u32(t866[0], t867[0]), vmulq_u32(t866[1], t867[1])];
    x4 = t868;
    let mut t869: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x3[2]) as u64) << 32 | vgetq_lane_u32::<0>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t869[0] = vsetq_lane_u32::<0>(v as u32, t869[0]); t869[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t869[2]); }
    { let v = (((vgetq_lane_u32::<1>(x3[2]) as u64) << 32 | vgetq_lane_u32::<1>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t869[0] = vsetq_lane_u32::<1>(v as u32, t869[0]); t869[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t869[2]); }
    { let v = (((vgetq_lane_u32::<2>(x3[2]) as u64) << 32 | vgetq_lane_u32::<2>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t869[0] = vsetq_lane_u32::<2>(v as u32, t869[0]); t869[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t869[2]); }
    { let v = (((vgetq_lane_u32::<3>(x3[2]) as u64) << 32 | vgetq_lane_u32::<3>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t869[0] = vsetq_lane_u32::<3>(v as u32, t869[0]); t869[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t869[2]); }
    { let v = (((vgetq_lane_u32::<0>(x3[3]) as u64) << 32 | vgetq_lane_u32::<0>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t869[1] = vsetq_lane_u32::<0>(v as u32, t869[1]); t869[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t869[3]); }
    { let v = (((vgetq_lane_u32::<1>(x3[3]) as u64) << 32 | vgetq_lane_u32::<1>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t869[1] = vsetq_lane_u32::<1>(v as u32, t869[1]); t869[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t869[3]); }
    { let v = (((vgetq_lane_u32::<2>(x3[3]) as u64) << 32 | vgetq_lane_u32::<2>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t869[1] = vsetq_lane_u32::<2>(v as u32, t869[1]); t869[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t869[3]); }
    { let v = (((vgetq_lane_u32::<3>(x3[3]) as u64) << 32 | vgetq_lane_u32::<3>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t869[1] = vsetq_lane_u32::<3>(v as u32, t869[1]); t869[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t869[3]); }
    let t870: [uint32x4_t; 2] = [x4[0], x4[1]];
    let t871: [uint32x4_t; 2] = [vandq_u32(t870[0], vdupq_n_u32(63)), vandq_u32(t870[1], vdupq_n_u32(63))];
    let t872: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t869[0], vnegq_s32(vreinterpretq_s32_u32(t871[0]))), vshlq_u32(t869[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t871[0])))), vorrq_u32(vshlq_u32(t869[1], vnegq_s32(vreinterpretq_s32_u32(t871[1]))), vshlq_u32(t869[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t871[1])))), vshlq_u32(t869[2], vnegq_s32(vreinterpretq_s32_u32(t871[0]))), vshlq_u32(t869[3], vnegq_s32(vreinterpretq_s32_u32(t871[1])))];
    x5 = t872;
    let t873 = [vdupq_n_u32(0u32); 2];
    let t874: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t873[0])), vmvnq_u32(vceqq_u32(x4[1], t873[1]))];
    let t875 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
    let t876: [uint32x4_t; 2] = [vaddq_u32(x4[0], t875[0]), vaddq_u32(x4[1], t875[1])];
    let t877 = [vdupq_n_u32(64u32); 2];
    let t878: [uint32x4_t; 2] = [vcgtq_u32(t876[0], t877[0]), vcgtq_u32(t876[1], t877[1])];
    let t879: [uint32x4_t; 2] = [vandq_u32(t874[0], t878[0]), vandq_u32(t874[1], t878[1])];
    let mut t880: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t881: [uint32x4_t; 2] = [vandq_u32(live[0], t879[0]), vandq_u32(live[1], t879[1])];
    let t882: [uint32x4_t; 2] = [vbicq_u32(live[0], t879[0]), vbicq_u32(live[1], t879[1])];
    if any(&t881) {
        let t883: u64 = (1u64).wrapping_mul(8u64);
        let t884 = [vdupq_n_u32((t883) as u32); 2];
        let t885 = [vdupq_n_u32(((t883) >> 32) as u32); 2];
        let t886: [uint32x4_t; 2] = [vaddq_u32(x3[0], t884[0]), vaddq_u32(x3[1], t884[1])];
        let t887: [uint32x4_t; 2] = [vcltq_u32(t886[0], x3[0]), vcltq_u32(t886[1], x3[1])];
        let t888: [uint32x4_t; 4] = [t886[0], t886[1], vsubq_u32(vaddq_u32(x3[2], t885[0]), t887[0]), vsubq_u32(vaddq_u32(x3[3], t885[1]), t887[1])];
        let mut t889: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t888[2]) as u64) << 32 | vgetq_lane_u32::<0>(t888[0]) as u64) as usize as *const u64).read_unaligned(); t889[0] = vsetq_lane_u32::<0>(v as u32, t889[0]); t889[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t889[2]); }
        { let v = (((vgetq_lane_u32::<1>(t888[2]) as u64) << 32 | vgetq_lane_u32::<1>(t888[0]) as u64) as usize as *const u64).read_unaligned(); t889[0] = vsetq_lane_u32::<1>(v as u32, t889[0]); t889[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t889[2]); }
        { let v = (((vgetq_lane_u32::<2>(t888[2]) as u64) << 32 | vgetq_lane_u32::<2>(t888[0]) as u64) as usize as *const u64).read_unaligned(); t889[0] = vsetq_lane_u32::<2>(v as u32, t889[0]); t889[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t889[2]); }
        { let v = (((vgetq_lane_u32::<3>(t888[2]) as u64) << 32 | vgetq_lane_u32::<3>(t888[0]) as u64) as usize as *const u64).read_unaligned(); t889[0] = vsetq_lane_u32::<3>(v as u32, t889[0]); t889[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t889[2]); }
        { let v = (((vgetq_lane_u32::<0>(t888[3]) as u64) << 32 | vgetq_lane_u32::<0>(t888[1]) as u64) as usize as *const u64).read_unaligned(); t889[1] = vsetq_lane_u32::<0>(v as u32, t889[1]); t889[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t889[3]); }
        { let v = (((vgetq_lane_u32::<1>(t888[3]) as u64) << 32 | vgetq_lane_u32::<1>(t888[1]) as u64) as usize as *const u64).read_unaligned(); t889[1] = vsetq_lane_u32::<1>(v as u32, t889[1]); t889[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t889[3]); }
        { let v = (((vgetq_lane_u32::<2>(t888[3]) as u64) << 32 | vgetq_lane_u32::<2>(t888[1]) as u64) as usize as *const u64).read_unaligned(); t889[1] = vsetq_lane_u32::<2>(v as u32, t889[1]); t889[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t889[3]); }
        { let v = (((vgetq_lane_u32::<3>(t888[3]) as u64) << 32 | vgetq_lane_u32::<3>(t888[1]) as u64) as usize as *const u64).read_unaligned(); t889[1] = vsetq_lane_u32::<3>(v as u32, t889[1]); t889[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t889[3]); }
        let t890: [uint32x4_t; 2] = [vsubq_u32(t877[0], x4[0]), vsubq_u32(t877[1], x4[1])];
        let t891: [uint32x4_t; 2] = [t890[0], t890[1]];
        let t892: [uint32x4_t; 2] = [vandq_u32(t891[0], vdupq_n_u32(63)), vandq_u32(t891[1], vdupq_n_u32(63))];
        let t893: [uint32x4_t; 4] = [vshlq_u32(t889[0], vreinterpretq_s32_u32(t892[0])), vshlq_u32(t889[1], vreinterpretq_s32_u32(t892[1])), vorrq_u32(vshlq_u32(t889[2], vreinterpretq_s32_u32(t892[0])), vshlq_u32(t889[0], vsubq_s32(vreinterpretq_s32_u32(t892[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t889[3], vreinterpretq_s32_u32(t892[1])), vshlq_u32(t889[1], vsubq_s32(vreinterpretq_s32_u32(t892[1]), vdupq_n_s32(32))))];
        let t894: [uint32x4_t; 4] = [vorrq_u32(x5[0], t893[0]), vorrq_u32(x5[1], t893[1]), vorrq_u32(x5[2], t893[2]), vorrq_u32(x5[3], t893[3])];
        t880 = [vbslq_u32(t881[0], t894[0], t880[0]), vbslq_u32(t881[1], t894[1], t880[1]), vbslq_u32(t881[0], t894[2], t880[2]), vbslq_u32(t881[1], t894[3], t880[3])];
    }
    {
        t880 = [vbslq_u32(t882[0], x5[0], t880[0]), vbslq_u32(t882[1], x5[1], t880[1]), vbslq_u32(t882[0], x5[2], t880[2]), vbslq_u32(t882[1], x5[3], t880[3])];
    }
    res = t880;
    return res;
}

#[inline(always)]
unsafe fn b17_r0an1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1b: *const u8, x1if: u32, x1is: u32, x2: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x3: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x4: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x5: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if ((x2) == (8u32)) {
        let t895: [uint32x4_t; 4];
        if (x1is) == 8u32 && ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 && all(&live) {
            let t896 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
            let t8970 = vld2q_u32((t896 as *const u32).add(0));
            let t8971 = vld2q_u32((t896 as *const u32).add(8));
            let t897: [uint32x4_t; 4] = [t8970.0, t8971.0, t8970.1, t8971.1];
            t895 = t897;
        } else {
            let t898: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
            let t899 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t898[0]), vandq_u32(live[1], t898[1]))));
            let t900: [uint32x4_t; 2] = [vbslq_u32(live[0], t898[0], t899), vbslq_u32(live[1], t898[1], t899)];
            let mut t901: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t900[0]) as usize * 1) as *const u64).read_unaligned(); t901[0] = vsetq_lane_u32::<0>(v as u32, t901[0]); t901[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t901[2]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t900[0]) as usize * 1) as *const u64).read_unaligned(); t901[0] = vsetq_lane_u32::<1>(v as u32, t901[0]); t901[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t901[2]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t900[0]) as usize * 1) as *const u64).read_unaligned(); t901[0] = vsetq_lane_u32::<2>(v as u32, t901[0]); t901[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t901[2]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t900[0]) as usize * 1) as *const u64).read_unaligned(); t901[0] = vsetq_lane_u32::<3>(v as u32, t901[0]); t901[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t901[2]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t900[1]) as usize * 1) as *const u64).read_unaligned(); t901[1] = vsetq_lane_u32::<0>(v as u32, t901[1]); t901[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t901[3]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t900[1]) as usize * 1) as *const u64).read_unaligned(); t901[1] = vsetq_lane_u32::<1>(v as u32, t901[1]); t901[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t901[3]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t900[1]) as usize * 1) as *const u64).read_unaligned(); t901[1] = vsetq_lane_u32::<2>(v as u32, t901[1]); t901[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t901[3]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t900[1]) as usize * 1) as *const u64).read_unaligned(); t901[1] = vsetq_lane_u32::<3>(v as u32, t901[1]); t901[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t901[3]); }
            t895 = t901;
        }
        res = t895;
        return res;
    }
    if ((x2) == (4u32)) {
        let t902: [uint32x4_t; 2];
        if (x1is) == 4u32 && ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 && all(&live) {
            let t903 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
            let t904: [uint32x4_t; 2] = core::array::from_fn(|r| vld1q_u32((t903 as *const u32).add(4 * r)));
            t902 = t904;
        } else {
            let t905: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
            let t906 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t905[0]), vandq_u32(live[1], t905[1]))));
            let t907: [uint32x4_t; 2] = [vbslq_u32(live[0], t905[0], t906), vbslq_u32(live[1], t905[1], t906)];
            let mut t908: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            t908[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t907[0]) as usize * 1) as *const u32).read_unaligned() as u32), t908[0]);
            t908[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t907[0]) as usize * 1) as *const u32).read_unaligned() as u32), t908[0]);
            t908[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t907[0]) as usize * 1) as *const u32).read_unaligned() as u32), t908[0]);
            t908[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t907[0]) as usize * 1) as *const u32).read_unaligned() as u32), t908[0]);
            t908[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t907[1]) as usize * 1) as *const u32).read_unaligned() as u32), t908[1]);
            t908[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t907[1]) as usize * 1) as *const u32).read_unaligned() as u32), t908[1]);
            t908[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t907[1]) as usize * 1) as *const u32).read_unaligned() as u32), t908[1]);
            t908[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t907[1]) as usize * 1) as *const u32).read_unaligned() as u32), t908[1]);
            t902 = t908;
        }
        let t909: [uint32x4_t; 4] = [t902[0], t902[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t909;
        return res;
    }
    if ((x2) == (2u32)) {
        let t910: [uint32x4_t; 2];
        if (x1is) == 2u32 && ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 && all(&live) {
            let t911 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
            let t912: [uint32x4_t; 2] = core::array::from_fn(|r| vmovl_u16(vld1_u16((t911 as *const u16).add(4 * r))));
            t910 = t912;
        } else {
            let t913: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
            let t914 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t913[0]), vandq_u32(live[1], t913[1]))));
            let t915: [uint32x4_t; 2] = [vbslq_u32(live[0], t913[0], t914), vbslq_u32(live[1], t913[1], t914)];
            let mut t916: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            t916[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t915[0]) as usize * 1) as *const u16).read_unaligned() as u32), t916[0]);
            t916[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t915[0]) as usize * 1) as *const u16).read_unaligned() as u32), t916[0]);
            t916[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t915[0]) as usize * 1) as *const u16).read_unaligned() as u32), t916[0]);
            t916[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t915[0]) as usize * 1) as *const u16).read_unaligned() as u32), t916[0]);
            t916[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t915[1]) as usize * 1) as *const u16).read_unaligned() as u32), t916[1]);
            t916[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t915[1]) as usize * 1) as *const u16).read_unaligned() as u32), t916[1]);
            t916[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t915[1]) as usize * 1) as *const u16).read_unaligned() as u32), t916[1]);
            t916[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t915[1]) as usize * 1) as *const u16).read_unaligned() as u32), t916[1]);
            t910 = t916;
        }
        let t917: [uint32x4_t; 4] = [t910[0], t910[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t917;
        return res;
    }
    if ((x2) == (1u32)) {
        let t918: [uint32x4_t; 2];
        if (x1is) == 1u32 && ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 && all(&live) {
            let t919 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
            let t921 = vmovl_u8(vld1_u8(t919.add(0)));
            let t920: [uint32x4_t; 2] = [vmovl_u16(vget_low_u16(t921)), vmovl_u16(vget_high_u16(t921))];
            t918 = t920;
        } else {
            let t922: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
            let t923 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t922[0]), vandq_u32(live[1], t922[1]))));
            let t924: [uint32x4_t; 2] = [vbslq_u32(live[0], t922[0], t923), vbslq_u32(live[1], t922[1], t923)];
            let mut t925: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            t925[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t924[0]) as usize * 1) as *const u8).read_unaligned() as u32), t925[0]);
            t925[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t924[0]) as usize * 1) as *const u8).read_unaligned() as u32), t925[0]);
            t925[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t924[0]) as usize * 1) as *const u8).read_unaligned() as u32), t925[0]);
            t925[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t924[0]) as usize * 1) as *const u8).read_unaligned() as u32), t925[0]);
            t925[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t924[1]) as usize * 1) as *const u8).read_unaligned() as u32), t925[1]);
            t925[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t924[1]) as usize * 1) as *const u8).read_unaligned() as u32), t925[1]);
            t925[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t924[1]) as usize * 1) as *const u8).read_unaligned() as u32), t925[1]);
            t925[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t924[1]) as usize * 1) as *const u8).read_unaligned() as u32), t925[1]);
            t918 = t925;
        }
        let t926: [uint32x4_t; 4] = [t918[0], t918[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t926;
        return res;
    }
    let t927: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
    let t928: [uint32x4_t; 2] = [t927[0], t927[1]];
    let t929: u64 = (((x1b).wrapping_add((((arg0) as usize as u64)) as usize)) as usize as u64);
    let t930 = [vdupq_n_u32((t929) as u32); 2];
    let t931 = [vdupq_n_u32(((t929) >> 32) as u32); 2];
    let t932: [uint32x4_t; 2] = [vaddq_u32(t930[0], t928[0]), vaddq_u32(t930[1], t928[1])];
    let t933: [uint32x4_t; 2] = [vcltq_u32(t932[0], t930[0]), vcltq_u32(t932[1], t930[1])];
    let t934: [uint32x4_t; 4] = [t932[0], t932[1], vsubq_u32(vaddq_u32(t931[0], vdupq_n_u32(0)), t933[0]), vsubq_u32(vaddq_u32(t931[1], vdupq_n_u32(0)), t933[1])];
    let t935: u64 = 7u64;
    let t936 = [vdupq_n_u32((t935) as u32); 2];
    let t937: [uint32x4_t; 2] = [vandq_u32(t934[0], t936[0]), vandq_u32(t934[1], t936[1])];
    let t938: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
    let t939: [uint32x4_t; 2] = [t938[0], t938[1]];
    let t940: u64 = (((arg0).wrapping_add((x1b) as usize)) as usize as u64);
    let t941 = [vdupq_n_u32((t940) as u32); 2];
    let t942 = [vdupq_n_u32(((t940) >> 32) as u32); 2];
    let t943: [uint32x4_t; 2] = [vaddq_u32(t941[0], t939[0]), vaddq_u32(t941[1], t939[1])];
    let t944: [uint32x4_t; 2] = [vcltq_u32(t943[0], t941[0]), vcltq_u32(t943[1], t941[1])];
    let t945: [uint32x4_t; 4] = [t943[0], t943[1], vsubq_u32(vaddq_u32(t942[0], vdupq_n_u32(0)), t944[0]), vsubq_u32(vaddq_u32(t942[1], vdupq_n_u32(0)), t944[1])];
    let t946: [uint32x4_t; 2] = [vcltq_u32(t945[0], t937[0]), vcltq_u32(t945[1], t937[1])];
    let t947: [uint32x4_t; 4] = [vsubq_u32(t945[0], t937[0]), vsubq_u32(t945[1], t937[1]), vaddq_u32(vsubq_u32(t945[2], vdupq_n_u32(0)), t946[0]), vaddq_u32(vsubq_u32(t945[3], vdupq_n_u32(0)), t946[1])];
    x3 = t947;
    let t948: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
    let t949: [uint32x4_t; 2] = [t948[0], t948[1]];
    let t950: u64 = (((x1b).wrapping_add((((arg0) as usize as u64)) as usize)) as usize as u64);
    let t951 = [vdupq_n_u32((t950) as u32); 2];
    let t952 = [vdupq_n_u32(((t950) >> 32) as u32); 2];
    let t953: [uint32x4_t; 2] = [vaddq_u32(t951[0], t949[0]), vaddq_u32(t951[1], t949[1])];
    let t954: [uint32x4_t; 2] = [vcltq_u32(t953[0], t951[0]), vcltq_u32(t953[1], t951[1])];
    let t955: [uint32x4_t; 4] = [t953[0], t953[1], vsubq_u32(vaddq_u32(t952[0], vdupq_n_u32(0)), t954[0]), vsubq_u32(vaddq_u32(t952[1], vdupq_n_u32(0)), t954[1])];
    let t956: u64 = 7u64;
    let t957 = [vdupq_n_u32((t956) as u32); 2];
    let t958: [uint32x4_t; 2] = [vandq_u32(t955[0], t957[0]), vandq_u32(t955[1], t957[1])];
    let t959 = [vdupq_n_u32(8u32); 2];
    let t960: [uint32x4_t; 2] = [vmulq_u32(t958[0], t959[0]), vmulq_u32(t958[1], t959[1])];
    x4 = t960;
    let mut t961: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x3[2]) as u64) << 32 | vgetq_lane_u32::<0>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t961[0] = vsetq_lane_u32::<0>(v as u32, t961[0]); t961[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t961[2]); }
    { let v = (((vgetq_lane_u32::<1>(x3[2]) as u64) << 32 | vgetq_lane_u32::<1>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t961[0] = vsetq_lane_u32::<1>(v as u32, t961[0]); t961[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t961[2]); }
    { let v = (((vgetq_lane_u32::<2>(x3[2]) as u64) << 32 | vgetq_lane_u32::<2>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t961[0] = vsetq_lane_u32::<2>(v as u32, t961[0]); t961[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t961[2]); }
    { let v = (((vgetq_lane_u32::<3>(x3[2]) as u64) << 32 | vgetq_lane_u32::<3>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t961[0] = vsetq_lane_u32::<3>(v as u32, t961[0]); t961[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t961[2]); }
    { let v = (((vgetq_lane_u32::<0>(x3[3]) as u64) << 32 | vgetq_lane_u32::<0>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t961[1] = vsetq_lane_u32::<0>(v as u32, t961[1]); t961[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t961[3]); }
    { let v = (((vgetq_lane_u32::<1>(x3[3]) as u64) << 32 | vgetq_lane_u32::<1>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t961[1] = vsetq_lane_u32::<1>(v as u32, t961[1]); t961[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t961[3]); }
    { let v = (((vgetq_lane_u32::<2>(x3[3]) as u64) << 32 | vgetq_lane_u32::<2>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t961[1] = vsetq_lane_u32::<2>(v as u32, t961[1]); t961[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t961[3]); }
    { let v = (((vgetq_lane_u32::<3>(x3[3]) as u64) << 32 | vgetq_lane_u32::<3>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t961[1] = vsetq_lane_u32::<3>(v as u32, t961[1]); t961[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t961[3]); }
    let t962: [uint32x4_t; 2] = [x4[0], x4[1]];
    let t963: [uint32x4_t; 2] = [vandq_u32(t962[0], vdupq_n_u32(63)), vandq_u32(t962[1], vdupq_n_u32(63))];
    let t964: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t961[0], vnegq_s32(vreinterpretq_s32_u32(t963[0]))), vshlq_u32(t961[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t963[0])))), vorrq_u32(vshlq_u32(t961[1], vnegq_s32(vreinterpretq_s32_u32(t963[1]))), vshlq_u32(t961[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t963[1])))), vshlq_u32(t961[2], vnegq_s32(vreinterpretq_s32_u32(t963[0]))), vshlq_u32(t961[3], vnegq_s32(vreinterpretq_s32_u32(t963[1])))];
    x5 = t964;
    let t965 = [vdupq_n_u32(0u32); 2];
    let t966: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t965[0])), vmvnq_u32(vceqq_u32(x4[1], t965[1]))];
    let t967 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
    let t968: [uint32x4_t; 2] = [vaddq_u32(x4[0], t967[0]), vaddq_u32(x4[1], t967[1])];
    let t969 = [vdupq_n_u32(64u32); 2];
    let t970: [uint32x4_t; 2] = [vcgtq_u32(t968[0], t969[0]), vcgtq_u32(t968[1], t969[1])];
    let t971: [uint32x4_t; 2] = [vandq_u32(t966[0], t970[0]), vandq_u32(t966[1], t970[1])];
    let mut t972: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t973: [uint32x4_t; 2] = [vandq_u32(live[0], t971[0]), vandq_u32(live[1], t971[1])];
    let t974: [uint32x4_t; 2] = [vbicq_u32(live[0], t971[0]), vbicq_u32(live[1], t971[1])];
    if any(&t973) {
        let t975: u64 = (1u64).wrapping_mul(8u64);
        let t976 = [vdupq_n_u32((t975) as u32); 2];
        let t977 = [vdupq_n_u32(((t975) >> 32) as u32); 2];
        let t978: [uint32x4_t; 2] = [vaddq_u32(x3[0], t976[0]), vaddq_u32(x3[1], t976[1])];
        let t979: [uint32x4_t; 2] = [vcltq_u32(t978[0], x3[0]), vcltq_u32(t978[1], x3[1])];
        let t980: [uint32x4_t; 4] = [t978[0], t978[1], vsubq_u32(vaddq_u32(x3[2], t977[0]), t979[0]), vsubq_u32(vaddq_u32(x3[3], t977[1]), t979[1])];
        let mut t981: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t980[2]) as u64) << 32 | vgetq_lane_u32::<0>(t980[0]) as u64) as usize as *const u64).read_unaligned(); t981[0] = vsetq_lane_u32::<0>(v as u32, t981[0]); t981[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t981[2]); }
        { let v = (((vgetq_lane_u32::<1>(t980[2]) as u64) << 32 | vgetq_lane_u32::<1>(t980[0]) as u64) as usize as *const u64).read_unaligned(); t981[0] = vsetq_lane_u32::<1>(v as u32, t981[0]); t981[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t981[2]); }
        { let v = (((vgetq_lane_u32::<2>(t980[2]) as u64) << 32 | vgetq_lane_u32::<2>(t980[0]) as u64) as usize as *const u64).read_unaligned(); t981[0] = vsetq_lane_u32::<2>(v as u32, t981[0]); t981[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t981[2]); }
        { let v = (((vgetq_lane_u32::<3>(t980[2]) as u64) << 32 | vgetq_lane_u32::<3>(t980[0]) as u64) as usize as *const u64).read_unaligned(); t981[0] = vsetq_lane_u32::<3>(v as u32, t981[0]); t981[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t981[2]); }
        { let v = (((vgetq_lane_u32::<0>(t980[3]) as u64) << 32 | vgetq_lane_u32::<0>(t980[1]) as u64) as usize as *const u64).read_unaligned(); t981[1] = vsetq_lane_u32::<0>(v as u32, t981[1]); t981[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t981[3]); }
        { let v = (((vgetq_lane_u32::<1>(t980[3]) as u64) << 32 | vgetq_lane_u32::<1>(t980[1]) as u64) as usize as *const u64).read_unaligned(); t981[1] = vsetq_lane_u32::<1>(v as u32, t981[1]); t981[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t981[3]); }
        { let v = (((vgetq_lane_u32::<2>(t980[3]) as u64) << 32 | vgetq_lane_u32::<2>(t980[1]) as u64) as usize as *const u64).read_unaligned(); t981[1] = vsetq_lane_u32::<2>(v as u32, t981[1]); t981[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t981[3]); }
        { let v = (((vgetq_lane_u32::<3>(t980[3]) as u64) << 32 | vgetq_lane_u32::<3>(t980[1]) as u64) as usize as *const u64).read_unaligned(); t981[1] = vsetq_lane_u32::<3>(v as u32, t981[1]); t981[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t981[3]); }
        let t982: [uint32x4_t; 2] = [vsubq_u32(t969[0], x4[0]), vsubq_u32(t969[1], x4[1])];
        let t983: [uint32x4_t; 2] = [t982[0], t982[1]];
        let t984: [uint32x4_t; 2] = [vandq_u32(t983[0], vdupq_n_u32(63)), vandq_u32(t983[1], vdupq_n_u32(63))];
        let t985: [uint32x4_t; 4] = [vshlq_u32(t981[0], vreinterpretq_s32_u32(t984[0])), vshlq_u32(t981[1], vreinterpretq_s32_u32(t984[1])), vorrq_u32(vshlq_u32(t981[2], vreinterpretq_s32_u32(t984[0])), vshlq_u32(t981[0], vsubq_s32(vreinterpretq_s32_u32(t984[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t981[3], vreinterpretq_s32_u32(t984[1])), vshlq_u32(t981[1], vsubq_s32(vreinterpretq_s32_u32(t984[1]), vdupq_n_s32(32))))];
        let t986: [uint32x4_t; 4] = [vorrq_u32(x5[0], t985[0]), vorrq_u32(x5[1], t985[1]), vorrq_u32(x5[2], t985[2]), vorrq_u32(x5[3], t985[3])];
        t972 = [vbslq_u32(t973[0], t986[0], t972[0]), vbslq_u32(t973[1], t986[1], t972[1]), vbslq_u32(t973[0], t986[2], t972[2]), vbslq_u32(t973[1], t986[3], t972[3])];
    }
    {
        t972 = [vbslq_u32(t974[0], x5[0], t972[0]), vbslq_u32(t974[1], x5[1], t972[1]), vbslq_u32(t974[0], x5[2], t972[2]), vbslq_u32(t974[1], x5[3], t972[3])];
    }
    res = t972;
    return res;
}

#[inline(always)]
unsafe fn b17_r0an1s_f(l: &Launch, lid0: u32, arg0: *const u8, x1b: *const u8, x1if: u32, x1is: u32, x2: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x3: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x4: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x5: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if ((x2) == (8u32)) {
        let t987: [uint32x4_t; 4];
        if (x1is) == 8u32 && ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 {
            let t988 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
            let t9890 = vld2q_u32((t988 as *const u32).add(0));
            let t9891 = vld2q_u32((t988 as *const u32).add(8));
            let t989: [uint32x4_t; 4] = [t9890.0, t9891.0, t9890.1, t9891.1];
            t987 = t989;
        } else {
            let t990: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
            let mut t991: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t990[0]) as usize * 1) as *const u64).read_unaligned(); t991[0] = vsetq_lane_u32::<0>(v as u32, t991[0]); t991[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t991[2]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t990[0]) as usize * 1) as *const u64).read_unaligned(); t991[0] = vsetq_lane_u32::<1>(v as u32, t991[0]); t991[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t991[2]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t990[0]) as usize * 1) as *const u64).read_unaligned(); t991[0] = vsetq_lane_u32::<2>(v as u32, t991[0]); t991[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t991[2]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t990[0]) as usize * 1) as *const u64).read_unaligned(); t991[0] = vsetq_lane_u32::<3>(v as u32, t991[0]); t991[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t991[2]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t990[1]) as usize * 1) as *const u64).read_unaligned(); t991[1] = vsetq_lane_u32::<0>(v as u32, t991[1]); t991[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t991[3]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t990[1]) as usize * 1) as *const u64).read_unaligned(); t991[1] = vsetq_lane_u32::<1>(v as u32, t991[1]); t991[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t991[3]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t990[1]) as usize * 1) as *const u64).read_unaligned(); t991[1] = vsetq_lane_u32::<2>(v as u32, t991[1]); t991[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t991[3]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t990[1]) as usize * 1) as *const u64).read_unaligned(); t991[1] = vsetq_lane_u32::<3>(v as u32, t991[1]); t991[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t991[3]); }
            t987 = t991;
        }
        res = t987;
        return res;
    }
    if ((x2) == (4u32)) {
        let t992: [uint32x4_t; 2];
        if (x1is) == 4u32 && ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 {
            let t993 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
            let t994: [uint32x4_t; 2] = core::array::from_fn(|r| vld1q_u32((t993 as *const u32).add(4 * r)));
            t992 = t994;
        } else {
            let t995: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
            let mut t996: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            t996[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t995[0]) as usize * 1) as *const u32).read_unaligned() as u32), t996[0]);
            t996[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t995[0]) as usize * 1) as *const u32).read_unaligned() as u32), t996[0]);
            t996[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t995[0]) as usize * 1) as *const u32).read_unaligned() as u32), t996[0]);
            t996[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t995[0]) as usize * 1) as *const u32).read_unaligned() as u32), t996[0]);
            t996[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t995[1]) as usize * 1) as *const u32).read_unaligned() as u32), t996[1]);
            t996[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t995[1]) as usize * 1) as *const u32).read_unaligned() as u32), t996[1]);
            t996[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t995[1]) as usize * 1) as *const u32).read_unaligned() as u32), t996[1]);
            t996[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t995[1]) as usize * 1) as *const u32).read_unaligned() as u32), t996[1]);
            t992 = t996;
        }
        let t997: [uint32x4_t; 4] = [t992[0], t992[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t997;
        return res;
    }
    if ((x2) == (2u32)) {
        let t998: [uint32x4_t; 2];
        if (x1is) == 2u32 && ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 {
            let t999 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
            let t1000: [uint32x4_t; 2] = core::array::from_fn(|r| vmovl_u16(vld1_u16((t999 as *const u16).add(4 * r))));
            t998 = t1000;
        } else {
            let t1001: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
            let mut t1002: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            t1002[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1001[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1002[0]);
            t1002[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1001[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1002[0]);
            t1002[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1001[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1002[0]);
            t1002[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1001[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1002[0]);
            t1002[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1001[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1002[1]);
            t1002[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1001[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1002[1]);
            t1002[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1001[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1002[1]);
            t1002[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1001[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1002[1]);
            t998 = t1002;
        }
        let t1003: [uint32x4_t; 4] = [t998[0], t998[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1003;
        return res;
    }
    if ((x2) == (1u32)) {
        let t1004: [uint32x4_t; 2];
        if (x1is) == 1u32 && ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 {
            let t1005 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
            let t1007 = vmovl_u8(vld1_u8(t1005.add(0)));
            let t1006: [uint32x4_t; 2] = [vmovl_u16(vget_low_u16(t1007)), vmovl_u16(vget_high_u16(t1007))];
            t1004 = t1006;
        } else {
            let t1008: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
            let mut t1009: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            t1009[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1008[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1009[0]);
            t1009[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1008[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1009[0]);
            t1009[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1008[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1009[0]);
            t1009[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1008[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1009[0]);
            t1009[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1008[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1009[1]);
            t1009[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1008[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1009[1]);
            t1009[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1008[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1009[1]);
            t1009[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1008[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1009[1]);
            t1004 = t1009;
        }
        let t1010: [uint32x4_t; 4] = [t1004[0], t1004[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1010;
        return res;
    }
    let t1011: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
    let t1012: [uint32x4_t; 2] = [t1011[0], t1011[1]];
    let t1013: u64 = (((x1b).wrapping_add((((arg0) as usize as u64)) as usize)) as usize as u64);
    let t1014 = [vdupq_n_u32((t1013) as u32); 2];
    let t1015 = [vdupq_n_u32(((t1013) >> 32) as u32); 2];
    let t1016: [uint32x4_t; 2] = [vaddq_u32(t1014[0], t1012[0]), vaddq_u32(t1014[1], t1012[1])];
    let t1017: [uint32x4_t; 2] = [vcltq_u32(t1016[0], t1014[0]), vcltq_u32(t1016[1], t1014[1])];
    let t1018: [uint32x4_t; 4] = [t1016[0], t1016[1], vsubq_u32(vaddq_u32(t1015[0], vdupq_n_u32(0)), t1017[0]), vsubq_u32(vaddq_u32(t1015[1], vdupq_n_u32(0)), t1017[1])];
    let t1019: u64 = 7u64;
    let t1020 = [vdupq_n_u32((t1019) as u32); 2];
    let t1021: [uint32x4_t; 2] = [vandq_u32(t1018[0], t1020[0]), vandq_u32(t1018[1], t1020[1])];
    let t1022: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
    let t1023: [uint32x4_t; 2] = [t1022[0], t1022[1]];
    let t1024: u64 = (((arg0).wrapping_add((x1b) as usize)) as usize as u64);
    let t1025 = [vdupq_n_u32((t1024) as u32); 2];
    let t1026 = [vdupq_n_u32(((t1024) >> 32) as u32); 2];
    let t1027: [uint32x4_t; 2] = [vaddq_u32(t1025[0], t1023[0]), vaddq_u32(t1025[1], t1023[1])];
    let t1028: [uint32x4_t; 2] = [vcltq_u32(t1027[0], t1025[0]), vcltq_u32(t1027[1], t1025[1])];
    let t1029: [uint32x4_t; 4] = [t1027[0], t1027[1], vsubq_u32(vaddq_u32(t1026[0], vdupq_n_u32(0)), t1028[0]), vsubq_u32(vaddq_u32(t1026[1], vdupq_n_u32(0)), t1028[1])];
    let t1030: [uint32x4_t; 2] = [vcltq_u32(t1029[0], t1021[0]), vcltq_u32(t1029[1], t1021[1])];
    let t1031: [uint32x4_t; 4] = [vsubq_u32(t1029[0], t1021[0]), vsubq_u32(t1029[1], t1021[1]), vaddq_u32(vsubq_u32(t1029[2], vdupq_n_u32(0)), t1030[0]), vaddq_u32(vsubq_u32(t1029[3], vdupq_n_u32(0)), t1030[1])];
    x3 = t1031;
    let t1032: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
    let t1033: [uint32x4_t; 2] = [t1032[0], t1032[1]];
    let t1034: u64 = (((x1b).wrapping_add((((arg0) as usize as u64)) as usize)) as usize as u64);
    let t1035 = [vdupq_n_u32((t1034) as u32); 2];
    let t1036 = [vdupq_n_u32(((t1034) >> 32) as u32); 2];
    let t1037: [uint32x4_t; 2] = [vaddq_u32(t1035[0], t1033[0]), vaddq_u32(t1035[1], t1033[1])];
    let t1038: [uint32x4_t; 2] = [vcltq_u32(t1037[0], t1035[0]), vcltq_u32(t1037[1], t1035[1])];
    let t1039: [uint32x4_t; 4] = [t1037[0], t1037[1], vsubq_u32(vaddq_u32(t1036[0], vdupq_n_u32(0)), t1038[0]), vsubq_u32(vaddq_u32(t1036[1], vdupq_n_u32(0)), t1038[1])];
    let t1040: u64 = 7u64;
    let t1041 = [vdupq_n_u32((t1040) as u32); 2];
    let t1042: [uint32x4_t; 2] = [vandq_u32(t1039[0], t1041[0]), vandq_u32(t1039[1], t1041[1])];
    let t1043 = [vdupq_n_u32(8u32); 2];
    let t1044: [uint32x4_t; 2] = [vmulq_u32(t1042[0], t1043[0]), vmulq_u32(t1042[1], t1043[1])];
    x4 = t1044;
    let mut t1045: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x3[2]) as u64) << 32 | vgetq_lane_u32::<0>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1045[0] = vsetq_lane_u32::<0>(v as u32, t1045[0]); t1045[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1045[2]); }
    { let v = (((vgetq_lane_u32::<1>(x3[2]) as u64) << 32 | vgetq_lane_u32::<1>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1045[0] = vsetq_lane_u32::<1>(v as u32, t1045[0]); t1045[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1045[2]); }
    { let v = (((vgetq_lane_u32::<2>(x3[2]) as u64) << 32 | vgetq_lane_u32::<2>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1045[0] = vsetq_lane_u32::<2>(v as u32, t1045[0]); t1045[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1045[2]); }
    { let v = (((vgetq_lane_u32::<3>(x3[2]) as u64) << 32 | vgetq_lane_u32::<3>(x3[0]) as u64) as usize as *const u64).read_unaligned(); t1045[0] = vsetq_lane_u32::<3>(v as u32, t1045[0]); t1045[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1045[2]); }
    { let v = (((vgetq_lane_u32::<0>(x3[3]) as u64) << 32 | vgetq_lane_u32::<0>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1045[1] = vsetq_lane_u32::<0>(v as u32, t1045[1]); t1045[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1045[3]); }
    { let v = (((vgetq_lane_u32::<1>(x3[3]) as u64) << 32 | vgetq_lane_u32::<1>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1045[1] = vsetq_lane_u32::<1>(v as u32, t1045[1]); t1045[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1045[3]); }
    { let v = (((vgetq_lane_u32::<2>(x3[3]) as u64) << 32 | vgetq_lane_u32::<2>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1045[1] = vsetq_lane_u32::<2>(v as u32, t1045[1]); t1045[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1045[3]); }
    { let v = (((vgetq_lane_u32::<3>(x3[3]) as u64) << 32 | vgetq_lane_u32::<3>(x3[1]) as u64) as usize as *const u64).read_unaligned(); t1045[1] = vsetq_lane_u32::<3>(v as u32, t1045[1]); t1045[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1045[3]); }
    let t1046: [uint32x4_t; 2] = [x4[0], x4[1]];
    let t1047: [uint32x4_t; 2] = [vandq_u32(t1046[0], vdupq_n_u32(63)), vandq_u32(t1046[1], vdupq_n_u32(63))];
    let t1048: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t1045[0], vnegq_s32(vreinterpretq_s32_u32(t1047[0]))), vshlq_u32(t1045[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1047[0])))), vorrq_u32(vshlq_u32(t1045[1], vnegq_s32(vreinterpretq_s32_u32(t1047[1]))), vshlq_u32(t1045[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1047[1])))), vshlq_u32(t1045[2], vnegq_s32(vreinterpretq_s32_u32(t1047[0]))), vshlq_u32(t1045[3], vnegq_s32(vreinterpretq_s32_u32(t1047[1])))];
    x5 = t1048;
    let t1049 = [vdupq_n_u32(0u32); 2];
    let t1050: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t1049[0])), vmvnq_u32(vceqq_u32(x4[1], t1049[1]))];
    let t1051 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
    let t1052: [uint32x4_t; 2] = [vaddq_u32(x4[0], t1051[0]), vaddq_u32(x4[1], t1051[1])];
    let t1053 = [vdupq_n_u32(64u32); 2];
    let t1054: [uint32x4_t; 2] = [vcgtq_u32(t1052[0], t1053[0]), vcgtq_u32(t1052[1], t1053[1])];
    let t1055: [uint32x4_t; 2] = [vandq_u32(t1050[0], t1054[0]), vandq_u32(t1050[1], t1054[1])];
    let mut t1056: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1057: [uint32x4_t; 2] = [vmvnq_u32(t1055[0]), vmvnq_u32(t1055[1])];
    if any(&t1055) {
        let t1058: u64 = (1u64).wrapping_mul(8u64);
        let t1059 = [vdupq_n_u32((t1058) as u32); 2];
        let t1060 = [vdupq_n_u32(((t1058) >> 32) as u32); 2];
        let t1061: [uint32x4_t; 2] = [vaddq_u32(x3[0], t1059[0]), vaddq_u32(x3[1], t1059[1])];
        let t1062: [uint32x4_t; 2] = [vcltq_u32(t1061[0], x3[0]), vcltq_u32(t1061[1], x3[1])];
        let t1063: [uint32x4_t; 4] = [t1061[0], t1061[1], vsubq_u32(vaddq_u32(x3[2], t1060[0]), t1062[0]), vsubq_u32(vaddq_u32(x3[3], t1060[1]), t1062[1])];
        let mut t1064: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t1063[2]) as u64) << 32 | vgetq_lane_u32::<0>(t1063[0]) as u64) as usize as *const u64).read_unaligned(); t1064[0] = vsetq_lane_u32::<0>(v as u32, t1064[0]); t1064[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1064[2]); }
        { let v = (((vgetq_lane_u32::<1>(t1063[2]) as u64) << 32 | vgetq_lane_u32::<1>(t1063[0]) as u64) as usize as *const u64).read_unaligned(); t1064[0] = vsetq_lane_u32::<1>(v as u32, t1064[0]); t1064[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1064[2]); }
        { let v = (((vgetq_lane_u32::<2>(t1063[2]) as u64) << 32 | vgetq_lane_u32::<2>(t1063[0]) as u64) as usize as *const u64).read_unaligned(); t1064[0] = vsetq_lane_u32::<2>(v as u32, t1064[0]); t1064[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1064[2]); }
        { let v = (((vgetq_lane_u32::<3>(t1063[2]) as u64) << 32 | vgetq_lane_u32::<3>(t1063[0]) as u64) as usize as *const u64).read_unaligned(); t1064[0] = vsetq_lane_u32::<3>(v as u32, t1064[0]); t1064[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1064[2]); }
        { let v = (((vgetq_lane_u32::<0>(t1063[3]) as u64) << 32 | vgetq_lane_u32::<0>(t1063[1]) as u64) as usize as *const u64).read_unaligned(); t1064[1] = vsetq_lane_u32::<0>(v as u32, t1064[1]); t1064[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1064[3]); }
        { let v = (((vgetq_lane_u32::<1>(t1063[3]) as u64) << 32 | vgetq_lane_u32::<1>(t1063[1]) as u64) as usize as *const u64).read_unaligned(); t1064[1] = vsetq_lane_u32::<1>(v as u32, t1064[1]); t1064[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1064[3]); }
        { let v = (((vgetq_lane_u32::<2>(t1063[3]) as u64) << 32 | vgetq_lane_u32::<2>(t1063[1]) as u64) as usize as *const u64).read_unaligned(); t1064[1] = vsetq_lane_u32::<2>(v as u32, t1064[1]); t1064[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1064[3]); }
        { let v = (((vgetq_lane_u32::<3>(t1063[3]) as u64) << 32 | vgetq_lane_u32::<3>(t1063[1]) as u64) as usize as *const u64).read_unaligned(); t1064[1] = vsetq_lane_u32::<3>(v as u32, t1064[1]); t1064[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1064[3]); }
        let t1065: [uint32x4_t; 2] = [vsubq_u32(t1053[0], x4[0]), vsubq_u32(t1053[1], x4[1])];
        let t1066: [uint32x4_t; 2] = [t1065[0], t1065[1]];
        let t1067: [uint32x4_t; 2] = [vandq_u32(t1066[0], vdupq_n_u32(63)), vandq_u32(t1066[1], vdupq_n_u32(63))];
        let t1068: [uint32x4_t; 4] = [vshlq_u32(t1064[0], vreinterpretq_s32_u32(t1067[0])), vshlq_u32(t1064[1], vreinterpretq_s32_u32(t1067[1])), vorrq_u32(vshlq_u32(t1064[2], vreinterpretq_s32_u32(t1067[0])), vshlq_u32(t1064[0], vsubq_s32(vreinterpretq_s32_u32(t1067[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1064[3], vreinterpretq_s32_u32(t1067[1])), vshlq_u32(t1064[1], vsubq_s32(vreinterpretq_s32_u32(t1067[1]), vdupq_n_s32(32))))];
        let t1069: [uint32x4_t; 4] = [vorrq_u32(x5[0], t1068[0]), vorrq_u32(x5[1], t1068[1]), vorrq_u32(x5[2], t1068[2]), vorrq_u32(x5[3], t1068[3])];
        t1056 = [vbslq_u32(t1055[0], t1069[0], t1056[0]), vbslq_u32(t1055[1], t1069[1], t1056[1]), vbslq_u32(t1055[0], t1069[2], t1056[2]), vbslq_u32(t1055[1], t1069[3], t1056[3])];
    }
    {
        t1056 = [vbslq_u32(t1057[0], x5[0], t1056[0]), vbslq_u32(t1057[1], x5[1], t1056[1]), vbslq_u32(t1057[0], x5[2], t1056[2]), vbslq_u32(t1057[1], x5[3], t1056[3])];
    }
    res = t1056;
    return res;
}

#[inline(always)]
unsafe fn b18_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t1070: u64;
    if ((x0) >= (64u32)) {
        t1070 = !(0u64);
    } else {
        t1070 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t1070;
    return res;
}

#[inline(always)]
unsafe fn b18_s_f(l: &Launch, lid0: u32, x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t1071: u64;
    if ((x0) >= (64u32)) {
        t1071 = !(0u64);
    } else {
        t1071 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t1071;
    return res;
}

#[inline(always)]
unsafe fn b19_r0sss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1: u32, x2: u32, x3: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u64 = 0;
    let mut live = k;
    let mut res: u64 = 0;
    let t1072 = b17_r0ss_p(l, lid0, live, arg0, (((x1) as u64)).wrapping_add(((x2) as u64)), x3);
    x4 = t1072;
    let t1073: u64;
    if ((x3) >= (8u32)) {
        t1073 = x4;
    } else {
        let t1074 = b18_s_p(l, lid0, live, (8u32).wrapping_mul(x3));
        t1073 = ((x4) & (t1074));
    }
    res = t1073;
    return res;
}

#[inline(always)]
unsafe fn b19_r0sss_f(l: &Launch, lid0: u32, arg0: *const u8, x1: u32, x2: u32, x3: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u64 = 0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t1075 = b17_r0ss_f(l, lid0, arg0, (((x1) as u64)).wrapping_add(((x2) as u64)), x3);
    x4 = t1075;
    let t1076: u64;
    if ((x3) >= (8u32)) {
        t1076 = x4;
    } else {
        let t1077 = b18_s_f(l, lid0, (8u32).wrapping_mul(x3));
        t1076 = ((x4) & (t1077));
    }
    res = t1076;
    return res;
}

#[inline(always)]
unsafe fn b19_r0sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1: u32, x2: [uint32x4_t; 2], x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1078 = b17_r0al1s_p(l, lid0, live, arg0, ((((x1) as u64)) as usize as *const u8), x2, x3);
    x4 = t1078;
    let t1079: [uint32x4_t; 4];
    if ((x3) >= (8u32)) {
        t1079 = x4;
    } else {
        let t1080 = b18_s_p(l, lid0, live, (8u32).wrapping_mul(x3));
        let t1081: u64 = t1080;
        let t1082 = [vdupq_n_u32((t1081) as u32); 2];
        let t1083 = [vdupq_n_u32(((t1081) >> 32) as u32); 2];
        let t1084: [uint32x4_t; 4] = [vandq_u32(x4[0], t1082[0]), vandq_u32(x4[1], t1082[1]), vandq_u32(x4[2], t1083[0]), vandq_u32(x4[3], t1083[1])];
        t1079 = t1084;
    }
    res = t1079;
    return res;
}

/// The group `l.group`'s threads, 8 at a time: `argv[j]` the address of argument `j`'s bytes (a buffer's address,
/// a value's bytes).
///
/// # Safety
///
/// `argv` holds the kernel's arguments, its buffers as large as it reads and writes; `l` is a 1-D group's launch.
pub unsafe fn parent_step_w8(argv: *const *const u8, l: &Launch) {
    let n = l.block[0] * l.block[1] * l.block[2];
    let arg0: *const u8 = (*argv.add(0) as *const *const u8).read_unaligned();
    let arg1: *const u8 = (*argv.add(1) as *const *const u8).read_unaligned();
    let arg2: *const u8 = (*argv.add(2) as *const *const u8).read_unaligned();
    let x3: *const u8 = *argv.add(3);
    let arg4: *const u8 = (*argv.add(4) as *const *const u8).read_unaligned();
    let x5: u32 = (*argv.add(5) as *const u32).read_unaligned();
    let mut t1085: bool = !(arg4).is_null();
    if t1085 {
        let mut t1086: bool = ((l.group[0]) >= ((arg4.wrapping_add((((x5) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        if !t1086 {
            t1086 = ((l.group[1]) >= ((arg4.wrapping_add(((((x5).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        let mut t1087: bool = t1086;
        if !t1087 {
            t1087 = ((l.group[2]) >= ((arg4.wrapping_add(((((x5).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        t1085 = t1087;
    }
    if t1085 {
        return;
    }
    let mut lid0 = 0u32;
    while lid0 + 8 <= n {
        let mut live = [vdupq_n_u32(u32::MAX); 2];
        let gid0 = l.group[0] * l.block[0] + lid0;
        'chunk: {
            b0_r0r1r2sg_f(l, lid0, arg0, arg1, arg2, x3);
        }
        lid0 += 8;
    }
    if lid0 < n {
        // Lanes part: the group's last threads, those under `n`.
        let t1088: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
        let mut live = t1088;
        let gid0 = l.group[0] * l.block[0] + lid0;
        'chunk: {
            b0_r0r1r2sg_p(l, lid0, live, arg0, arg1, arg2, x3);
        }
    }
}
