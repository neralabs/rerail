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
    x3 = arg0.wrapping_add(((x1) & (!(7u64))) as usize);
    x4 = (((((x1) & (7u64))) as u32)).wrapping_mul(8u32);
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
    x3 = arg0.wrapping_add(((x1) & (!(7u64))) as usize);
    x4 = (((((x1) & (7u64))) as u32)).wrapping_mul(8u32);
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
        let t652 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([t651[0], t651[1]]);
        let t653 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([t651[2], t651[3]]);
        let t655 = bits(&live);
        let t656 = t655.trailing_zeros() as usize % 8;
        let t654: [usize; 8] = [{ let j = if t655 >> 0 & 1 != 0 { 0 } else { t656 }; ((t653[j] as u64) << 32 | t652[j] as u64) as usize }, { let j = if t655 >> 1 & 1 != 0 { 1 } else { t656 }; ((t653[j] as u64) << 32 | t652[j] as u64) as usize }, { let j = if t655 >> 2 & 1 != 0 { 2 } else { t656 }; ((t653[j] as u64) << 32 | t652[j] as u64) as usize }, { let j = if t655 >> 3 & 1 != 0 { 3 } else { t656 }; ((t653[j] as u64) << 32 | t652[j] as u64) as usize }, { let j = if t655 >> 4 & 1 != 0 { 4 } else { t656 }; ((t653[j] as u64) << 32 | t652[j] as u64) as usize }, { let j = if t655 >> 5 & 1 != 0 { 5 } else { t656 }; ((t653[j] as u64) << 32 | t652[j] as u64) as usize }, { let j = if t655 >> 6 & 1 != 0 { 6 } else { t656 }; ((t653[j] as u64) << 32 | t652[j] as u64) as usize }, { let j = if t655 >> 7 & 1 != 0 { 7 } else { t656 }; ((t653[j] as u64) << 32 | t652[j] as u64) as usize }];
        let mut t657: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (t654[0] as *const u64).read_unaligned(); t657[0] = vsetq_lane_u32::<0>(v as u32, t657[0]); t657[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t657[2]); }
        { let v = (t654[1] as *const u64).read_unaligned(); t657[0] = vsetq_lane_u32::<1>(v as u32, t657[0]); t657[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t657[2]); }
        { let v = (t654[2] as *const u64).read_unaligned(); t657[0] = vsetq_lane_u32::<2>(v as u32, t657[0]); t657[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t657[2]); }
        { let v = (t654[3] as *const u64).read_unaligned(); t657[0] = vsetq_lane_u32::<3>(v as u32, t657[0]); t657[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t657[2]); }
        { let v = (t654[4] as *const u64).read_unaligned(); t657[1] = vsetq_lane_u32::<0>(v as u32, t657[1]); t657[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t657[3]); }
        { let v = (t654[5] as *const u64).read_unaligned(); t657[1] = vsetq_lane_u32::<1>(v as u32, t657[1]); t657[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t657[3]); }
        { let v = (t654[6] as *const u64).read_unaligned(); t657[1] = vsetq_lane_u32::<2>(v as u32, t657[1]); t657[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t657[3]); }
        { let v = (t654[7] as *const u64).read_unaligned(); t657[1] = vsetq_lane_u32::<3>(v as u32, t657[1]); t657[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t657[3]); }
        res = t657;
        return res;
    }
    if ((x2) == (4u32)) {
        let t658: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
        let t659: [uint32x4_t; 4] = [vandq_u32(x1[0], t658[0]), vandq_u32(x1[1], t658[1]), vandq_u32(x1[2], t658[2]), vandq_u32(x1[3], t658[3])];
        let t660: u64 = ((arg0) as usize as u64);
        let t661 = [vdupq_n_u32((t660) as u32); 2];
        let t662 = [vdupq_n_u32(((t660) >> 32) as u32); 2];
        let t663: [uint32x4_t; 2] = [vaddq_u32(t661[0], t659[0]), vaddq_u32(t661[1], t659[1])];
        let t664: [uint32x4_t; 2] = [vcltq_u32(t663[0], t661[0]), vcltq_u32(t663[1], t661[1])];
        let t665: [uint32x4_t; 4] = [t663[0], t663[1], vsubq_u32(vaddq_u32(t662[0], t659[2]), t664[0]), vsubq_u32(vaddq_u32(t662[1], t659[3]), t664[1])];
        let t666 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([t665[0], t665[1]]);
        let t667 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([t665[2], t665[3]]);
        let t669 = bits(&live);
        let t670 = t669.trailing_zeros() as usize % 8;
        let t668: [usize; 8] = [{ let j = if t669 >> 0 & 1 != 0 { 0 } else { t670 }; ((t667[j] as u64) << 32 | t666[j] as u64) as usize }, { let j = if t669 >> 1 & 1 != 0 { 1 } else { t670 }; ((t667[j] as u64) << 32 | t666[j] as u64) as usize }, { let j = if t669 >> 2 & 1 != 0 { 2 } else { t670 }; ((t667[j] as u64) << 32 | t666[j] as u64) as usize }, { let j = if t669 >> 3 & 1 != 0 { 3 } else { t670 }; ((t667[j] as u64) << 32 | t666[j] as u64) as usize }, { let j = if t669 >> 4 & 1 != 0 { 4 } else { t670 }; ((t667[j] as u64) << 32 | t666[j] as u64) as usize }, { let j = if t669 >> 5 & 1 != 0 { 5 } else { t670 }; ((t667[j] as u64) << 32 | t666[j] as u64) as usize }, { let j = if t669 >> 6 & 1 != 0 { 6 } else { t670 }; ((t667[j] as u64) << 32 | t666[j] as u64) as usize }, { let j = if t669 >> 7 & 1 != 0 { 7 } else { t670 }; ((t667[j] as u64) << 32 | t666[j] as u64) as usize }];
        let mut t671: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t671[0] = vsetq_lane_u32::<0>((t668[0] as *const u32).read_unaligned() as u32, t671[0]);
        t671[0] = vsetq_lane_u32::<1>((t668[1] as *const u32).read_unaligned() as u32, t671[0]);
        t671[0] = vsetq_lane_u32::<2>((t668[2] as *const u32).read_unaligned() as u32, t671[0]);
        t671[0] = vsetq_lane_u32::<3>((t668[3] as *const u32).read_unaligned() as u32, t671[0]);
        t671[1] = vsetq_lane_u32::<0>((t668[4] as *const u32).read_unaligned() as u32, t671[1]);
        t671[1] = vsetq_lane_u32::<1>((t668[5] as *const u32).read_unaligned() as u32, t671[1]);
        t671[1] = vsetq_lane_u32::<2>((t668[6] as *const u32).read_unaligned() as u32, t671[1]);
        t671[1] = vsetq_lane_u32::<3>((t668[7] as *const u32).read_unaligned() as u32, t671[1]);
        let t672: [uint32x4_t; 4] = [t671[0], t671[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t672;
        return res;
    }
    if ((x2) == (2u32)) {
        let t673: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
        let t674: [uint32x4_t; 4] = [vandq_u32(x1[0], t673[0]), vandq_u32(x1[1], t673[1]), vandq_u32(x1[2], t673[2]), vandq_u32(x1[3], t673[3])];
        let t675: u64 = ((arg0) as usize as u64);
        let t676 = [vdupq_n_u32((t675) as u32); 2];
        let t677 = [vdupq_n_u32(((t675) >> 32) as u32); 2];
        let t678: [uint32x4_t; 2] = [vaddq_u32(t676[0], t674[0]), vaddq_u32(t676[1], t674[1])];
        let t679: [uint32x4_t; 2] = [vcltq_u32(t678[0], t676[0]), vcltq_u32(t678[1], t676[1])];
        let t680: [uint32x4_t; 4] = [t678[0], t678[1], vsubq_u32(vaddq_u32(t677[0], t674[2]), t679[0]), vsubq_u32(vaddq_u32(t677[1], t674[3]), t679[1])];
        let t681 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([t680[0], t680[1]]);
        let t682 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([t680[2], t680[3]]);
        let t684 = bits(&live);
        let t685 = t684.trailing_zeros() as usize % 8;
        let t683: [usize; 8] = [{ let j = if t684 >> 0 & 1 != 0 { 0 } else { t685 }; ((t682[j] as u64) << 32 | t681[j] as u64) as usize }, { let j = if t684 >> 1 & 1 != 0 { 1 } else { t685 }; ((t682[j] as u64) << 32 | t681[j] as u64) as usize }, { let j = if t684 >> 2 & 1 != 0 { 2 } else { t685 }; ((t682[j] as u64) << 32 | t681[j] as u64) as usize }, { let j = if t684 >> 3 & 1 != 0 { 3 } else { t685 }; ((t682[j] as u64) << 32 | t681[j] as u64) as usize }, { let j = if t684 >> 4 & 1 != 0 { 4 } else { t685 }; ((t682[j] as u64) << 32 | t681[j] as u64) as usize }, { let j = if t684 >> 5 & 1 != 0 { 5 } else { t685 }; ((t682[j] as u64) << 32 | t681[j] as u64) as usize }, { let j = if t684 >> 6 & 1 != 0 { 6 } else { t685 }; ((t682[j] as u64) << 32 | t681[j] as u64) as usize }, { let j = if t684 >> 7 & 1 != 0 { 7 } else { t685 }; ((t682[j] as u64) << 32 | t681[j] as u64) as usize }];
        let mut t686: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t686[0] = vsetq_lane_u32::<0>((t683[0] as *const u16).read_unaligned() as u32, t686[0]);
        t686[0] = vsetq_lane_u32::<1>((t683[1] as *const u16).read_unaligned() as u32, t686[0]);
        t686[0] = vsetq_lane_u32::<2>((t683[2] as *const u16).read_unaligned() as u32, t686[0]);
        t686[0] = vsetq_lane_u32::<3>((t683[3] as *const u16).read_unaligned() as u32, t686[0]);
        t686[1] = vsetq_lane_u32::<0>((t683[4] as *const u16).read_unaligned() as u32, t686[1]);
        t686[1] = vsetq_lane_u32::<1>((t683[5] as *const u16).read_unaligned() as u32, t686[1]);
        t686[1] = vsetq_lane_u32::<2>((t683[6] as *const u16).read_unaligned() as u32, t686[1]);
        t686[1] = vsetq_lane_u32::<3>((t683[7] as *const u16).read_unaligned() as u32, t686[1]);
        let t687: [uint32x4_t; 4] = [t686[0], t686[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t687;
        return res;
    }
    if ((x2) == (1u32)) {
        let t688: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
        let t689: [uint32x4_t; 4] = [vandq_u32(x1[0], t688[0]), vandq_u32(x1[1], t688[1]), vandq_u32(x1[2], t688[2]), vandq_u32(x1[3], t688[3])];
        let t690: u64 = ((arg0) as usize as u64);
        let t691 = [vdupq_n_u32((t690) as u32); 2];
        let t692 = [vdupq_n_u32(((t690) >> 32) as u32); 2];
        let t693: [uint32x4_t; 2] = [vaddq_u32(t691[0], t689[0]), vaddq_u32(t691[1], t689[1])];
        let t694: [uint32x4_t; 2] = [vcltq_u32(t693[0], t691[0]), vcltq_u32(t693[1], t691[1])];
        let t695: [uint32x4_t; 4] = [t693[0], t693[1], vsubq_u32(vaddq_u32(t692[0], t689[2]), t694[0]), vsubq_u32(vaddq_u32(t692[1], t689[3]), t694[1])];
        let t696 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([t695[0], t695[1]]);
        let t697 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([t695[2], t695[3]]);
        let t699 = bits(&live);
        let t700 = t699.trailing_zeros() as usize % 8;
        let t698: [usize; 8] = [{ let j = if t699 >> 0 & 1 != 0 { 0 } else { t700 }; ((t697[j] as u64) << 32 | t696[j] as u64) as usize }, { let j = if t699 >> 1 & 1 != 0 { 1 } else { t700 }; ((t697[j] as u64) << 32 | t696[j] as u64) as usize }, { let j = if t699 >> 2 & 1 != 0 { 2 } else { t700 }; ((t697[j] as u64) << 32 | t696[j] as u64) as usize }, { let j = if t699 >> 3 & 1 != 0 { 3 } else { t700 }; ((t697[j] as u64) << 32 | t696[j] as u64) as usize }, { let j = if t699 >> 4 & 1 != 0 { 4 } else { t700 }; ((t697[j] as u64) << 32 | t696[j] as u64) as usize }, { let j = if t699 >> 5 & 1 != 0 { 5 } else { t700 }; ((t697[j] as u64) << 32 | t696[j] as u64) as usize }, { let j = if t699 >> 6 & 1 != 0 { 6 } else { t700 }; ((t697[j] as u64) << 32 | t696[j] as u64) as usize }, { let j = if t699 >> 7 & 1 != 0 { 7 } else { t700 }; ((t697[j] as u64) << 32 | t696[j] as u64) as usize }];
        let mut t701: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t701[0] = vsetq_lane_u32::<0>((t698[0] as *const u8).read_unaligned() as u32, t701[0]);
        t701[0] = vsetq_lane_u32::<1>((t698[1] as *const u8).read_unaligned() as u32, t701[0]);
        t701[0] = vsetq_lane_u32::<2>((t698[2] as *const u8).read_unaligned() as u32, t701[0]);
        t701[0] = vsetq_lane_u32::<3>((t698[3] as *const u8).read_unaligned() as u32, t701[0]);
        t701[1] = vsetq_lane_u32::<0>((t698[4] as *const u8).read_unaligned() as u32, t701[1]);
        t701[1] = vsetq_lane_u32::<1>((t698[5] as *const u8).read_unaligned() as u32, t701[1]);
        t701[1] = vsetq_lane_u32::<2>((t698[6] as *const u8).read_unaligned() as u32, t701[1]);
        t701[1] = vsetq_lane_u32::<3>((t698[7] as *const u8).read_unaligned() as u32, t701[1]);
        let t702: [uint32x4_t; 4] = [t701[0], t701[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t702;
        return res;
    }
    let t703: u64 = !(7u64);
    let t704 = [vdupq_n_u32((t703) as u32); 2];
    let t705 = [vdupq_n_u32(((t703) >> 32) as u32); 2];
    let t706: [uint32x4_t; 4] = [vandq_u32(x1[0], t704[0]), vandq_u32(x1[1], t704[1]), vandq_u32(x1[2], t705[0]), vandq_u32(x1[3], t705[1])];
    let t707: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
    let t708: [uint32x4_t; 4] = [vandq_u32(t706[0], t707[0]), vandq_u32(t706[1], t707[1]), vandq_u32(t706[2], t707[2]), vandq_u32(t706[3], t707[3])];
    let t709: u64 = ((arg0) as usize as u64);
    let t710 = [vdupq_n_u32((t709) as u32); 2];
    let t711 = [vdupq_n_u32(((t709) >> 32) as u32); 2];
    let t712: [uint32x4_t; 2] = [vaddq_u32(t710[0], t708[0]), vaddq_u32(t710[1], t708[1])];
    let t713: [uint32x4_t; 2] = [vcltq_u32(t712[0], t710[0]), vcltq_u32(t712[1], t710[1])];
    let t714: [uint32x4_t; 4] = [t712[0], t712[1], vsubq_u32(vaddq_u32(t711[0], t708[2]), t713[0]), vsubq_u32(vaddq_u32(t711[1], t708[3]), t713[1])];
    x3 = t714;
    let t715: u64 = 7u64;
    let t716 = [vdupq_n_u32((t715) as u32); 2];
    let t717: [uint32x4_t; 2] = [vandq_u32(x1[0], t716[0]), vandq_u32(x1[1], t716[1])];
    let t718 = [vdupq_n_u32(8u32); 2];
    let t719: [uint32x4_t; 2] = [vmulq_u32(t717[0], t718[0]), vmulq_u32(t717[1], t718[1])];
    x4 = t719;
    let t720 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([x3[0], x3[1]]);
    let t721 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([x3[2], x3[3]]);
    let t723 = bits(&live);
    let t724 = t723.trailing_zeros() as usize % 8;
    let t722: [usize; 8] = [{ let j = if t723 >> 0 & 1 != 0 { 0 } else { t724 }; ((t721[j] as u64) << 32 | t720[j] as u64) as usize }, { let j = if t723 >> 1 & 1 != 0 { 1 } else { t724 }; ((t721[j] as u64) << 32 | t720[j] as u64) as usize }, { let j = if t723 >> 2 & 1 != 0 { 2 } else { t724 }; ((t721[j] as u64) << 32 | t720[j] as u64) as usize }, { let j = if t723 >> 3 & 1 != 0 { 3 } else { t724 }; ((t721[j] as u64) << 32 | t720[j] as u64) as usize }, { let j = if t723 >> 4 & 1 != 0 { 4 } else { t724 }; ((t721[j] as u64) << 32 | t720[j] as u64) as usize }, { let j = if t723 >> 5 & 1 != 0 { 5 } else { t724 }; ((t721[j] as u64) << 32 | t720[j] as u64) as usize }, { let j = if t723 >> 6 & 1 != 0 { 6 } else { t724 }; ((t721[j] as u64) << 32 | t720[j] as u64) as usize }, { let j = if t723 >> 7 & 1 != 0 { 7 } else { t724 }; ((t721[j] as u64) << 32 | t720[j] as u64) as usize }];
    let mut t725: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (t722[0] as *const u64).read_unaligned(); t725[0] = vsetq_lane_u32::<0>(v as u32, t725[0]); t725[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t725[2]); }
    { let v = (t722[1] as *const u64).read_unaligned(); t725[0] = vsetq_lane_u32::<1>(v as u32, t725[0]); t725[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t725[2]); }
    { let v = (t722[2] as *const u64).read_unaligned(); t725[0] = vsetq_lane_u32::<2>(v as u32, t725[0]); t725[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t725[2]); }
    { let v = (t722[3] as *const u64).read_unaligned(); t725[0] = vsetq_lane_u32::<3>(v as u32, t725[0]); t725[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t725[2]); }
    { let v = (t722[4] as *const u64).read_unaligned(); t725[1] = vsetq_lane_u32::<0>(v as u32, t725[1]); t725[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t725[3]); }
    { let v = (t722[5] as *const u64).read_unaligned(); t725[1] = vsetq_lane_u32::<1>(v as u32, t725[1]); t725[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t725[3]); }
    { let v = (t722[6] as *const u64).read_unaligned(); t725[1] = vsetq_lane_u32::<2>(v as u32, t725[1]); t725[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t725[3]); }
    { let v = (t722[7] as *const u64).read_unaligned(); t725[1] = vsetq_lane_u32::<3>(v as u32, t725[1]); t725[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t725[3]); }
    let t726: [uint32x4_t; 2] = [x4[0], x4[1]];
    let t727: [uint32x4_t; 2] = [vandq_u32(t726[0], vdupq_n_u32(63)), vandq_u32(t726[1], vdupq_n_u32(63))];
    let t728: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t725[0], vnegq_s32(vreinterpretq_s32_u32(t727[0]))), vshlq_u32(t725[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t727[0])))), vorrq_u32(vshlq_u32(t725[1], vnegq_s32(vreinterpretq_s32_u32(t727[1]))), vshlq_u32(t725[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t727[1])))), vshlq_u32(t725[2], vnegq_s32(vreinterpretq_s32_u32(t727[0]))), vshlq_u32(t725[3], vnegq_s32(vreinterpretq_s32_u32(t727[1])))];
    x5 = t728;
    let t729 = [vdupq_n_u32(0u32); 2];
    let t730: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t729[0])), vmvnq_u32(vceqq_u32(x4[1], t729[1]))];
    let t731 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
    let t732: [uint32x4_t; 2] = [vaddq_u32(x4[0], t731[0]), vaddq_u32(x4[1], t731[1])];
    let t733 = [vdupq_n_u32(64u32); 2];
    let t734: [uint32x4_t; 2] = [vcgtq_u32(t732[0], t733[0]), vcgtq_u32(t732[1], t733[1])];
    let t735: [uint32x4_t; 2] = [vandq_u32(t730[0], t734[0]), vandq_u32(t730[1], t734[1])];
    let mut t736: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t737: [uint32x4_t; 2] = [vandq_u32(live[0], t735[0]), vandq_u32(live[1], t735[1])];
    let t738: [uint32x4_t; 2] = [vbicq_u32(live[0], t735[0]), vbicq_u32(live[1], t735[1])];
    {
        let t739: u64 = (1u64).wrapping_mul(8u64);
        let t740 = [vdupq_n_u32((t739) as u32); 2];
        let t741 = [vdupq_n_u32(((t739) >> 32) as u32); 2];
        let t742: [uint32x4_t; 2] = [vaddq_u32(x3[0], t740[0]), vaddq_u32(x3[1], t740[1])];
        let t743: [uint32x4_t; 2] = [vcltq_u32(t742[0], x3[0]), vcltq_u32(t742[1], x3[1])];
        let t744: [uint32x4_t; 4] = [t742[0], t742[1], vsubq_u32(vaddq_u32(x3[2], t741[0]), t743[0]), vsubq_u32(vaddq_u32(x3[3], t741[1]), t743[1])];
        let t745 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([t744[0], t744[1]]);
        let t746 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([t744[2], t744[3]]);
        let t748 = bits(&t737);
        let t749 = t748.trailing_zeros() as usize % 8;
        let t747: [usize; 8] = [{ let j = if t748 >> 0 & 1 != 0 { 0 } else { t749 }; ((t746[j] as u64) << 32 | t745[j] as u64) as usize }, { let j = if t748 >> 1 & 1 != 0 { 1 } else { t749 }; ((t746[j] as u64) << 32 | t745[j] as u64) as usize }, { let j = if t748 >> 2 & 1 != 0 { 2 } else { t749 }; ((t746[j] as u64) << 32 | t745[j] as u64) as usize }, { let j = if t748 >> 3 & 1 != 0 { 3 } else { t749 }; ((t746[j] as u64) << 32 | t745[j] as u64) as usize }, { let j = if t748 >> 4 & 1 != 0 { 4 } else { t749 }; ((t746[j] as u64) << 32 | t745[j] as u64) as usize }, { let j = if t748 >> 5 & 1 != 0 { 5 } else { t749 }; ((t746[j] as u64) << 32 | t745[j] as u64) as usize }, { let j = if t748 >> 6 & 1 != 0 { 6 } else { t749 }; ((t746[j] as u64) << 32 | t745[j] as u64) as usize }, { let j = if t748 >> 7 & 1 != 0 { 7 } else { t749 }; ((t746[j] as u64) << 32 | t745[j] as u64) as usize }];
        let mut t750: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (t747[0] as *const u64).read_unaligned(); t750[0] = vsetq_lane_u32::<0>(v as u32, t750[0]); t750[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t750[2]); }
        { let v = (t747[1] as *const u64).read_unaligned(); t750[0] = vsetq_lane_u32::<1>(v as u32, t750[0]); t750[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t750[2]); }
        { let v = (t747[2] as *const u64).read_unaligned(); t750[0] = vsetq_lane_u32::<2>(v as u32, t750[0]); t750[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t750[2]); }
        { let v = (t747[3] as *const u64).read_unaligned(); t750[0] = vsetq_lane_u32::<3>(v as u32, t750[0]); t750[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t750[2]); }
        { let v = (t747[4] as *const u64).read_unaligned(); t750[1] = vsetq_lane_u32::<0>(v as u32, t750[1]); t750[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t750[3]); }
        { let v = (t747[5] as *const u64).read_unaligned(); t750[1] = vsetq_lane_u32::<1>(v as u32, t750[1]); t750[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t750[3]); }
        { let v = (t747[6] as *const u64).read_unaligned(); t750[1] = vsetq_lane_u32::<2>(v as u32, t750[1]); t750[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t750[3]); }
        { let v = (t747[7] as *const u64).read_unaligned(); t750[1] = vsetq_lane_u32::<3>(v as u32, t750[1]); t750[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t750[3]); }
        let t751: [uint32x4_t; 2] = [vsubq_u32(t733[0], x4[0]), vsubq_u32(t733[1], x4[1])];
        let t752: [uint32x4_t; 2] = [t751[0], t751[1]];
        let t753: [uint32x4_t; 2] = [vandq_u32(t752[0], vdupq_n_u32(63)), vandq_u32(t752[1], vdupq_n_u32(63))];
        let t754: [uint32x4_t; 4] = [vshlq_u32(t750[0], vreinterpretq_s32_u32(t753[0])), vshlq_u32(t750[1], vreinterpretq_s32_u32(t753[1])), vorrq_u32(vshlq_u32(t750[2], vreinterpretq_s32_u32(t753[0])), vshlq_u32(t750[0], vsubq_s32(vreinterpretq_s32_u32(t753[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t750[3], vreinterpretq_s32_u32(t753[1])), vshlq_u32(t750[1], vsubq_s32(vreinterpretq_s32_u32(t753[1]), vdupq_n_s32(32))))];
        let t755: [uint32x4_t; 4] = [vorrq_u32(x5[0], t754[0]), vorrq_u32(x5[1], t754[1]), vorrq_u32(x5[2], t754[2]), vorrq_u32(x5[3], t754[3])];
        t736 = [vbslq_u32(t737[0], t755[0], t736[0]), vbslq_u32(t737[1], t755[1], t736[1]), vbslq_u32(t737[0], t755[2], t736[2]), vbslq_u32(t737[1], t755[3], t736[3])];
    }
    {
        t736 = [vbslq_u32(t738[0], x5[0], t736[0]), vbslq_u32(t738[1], x5[1], t736[1]), vbslq_u32(t738[0], x5[2], t736[2]), vbslq_u32(t738[1], x5[3], t736[3])];
    }
    res = t736;
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
        let t756: u64 = ((arg0) as usize as u64);
        let t757 = [vdupq_n_u32((t756) as u32); 2];
        let t758 = [vdupq_n_u32(((t756) >> 32) as u32); 2];
        let t759: [uint32x4_t; 2] = [vaddq_u32(t757[0], x1[0]), vaddq_u32(t757[1], x1[1])];
        let t760: [uint32x4_t; 2] = [vcltq_u32(t759[0], t757[0]), vcltq_u32(t759[1], t757[1])];
        let t761: [uint32x4_t; 4] = [t759[0], t759[1], vsubq_u32(vaddq_u32(t758[0], x1[2]), t760[0]), vsubq_u32(vaddq_u32(t758[1], x1[3]), t760[1])];
        let t762 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([t761[0], t761[1]]);
        let t763 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([t761[2], t761[3]]);
        let t764: [usize; 8] = [((t763[0] as u64) << 32 | t762[0] as u64) as usize, ((t763[1] as u64) << 32 | t762[1] as u64) as usize, ((t763[2] as u64) << 32 | t762[2] as u64) as usize, ((t763[3] as u64) << 32 | t762[3] as u64) as usize, ((t763[4] as u64) << 32 | t762[4] as u64) as usize, ((t763[5] as u64) << 32 | t762[5] as u64) as usize, ((t763[6] as u64) << 32 | t762[6] as u64) as usize, ((t763[7] as u64) << 32 | t762[7] as u64) as usize];
        let mut t765: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (t764[0] as *const u64).read_unaligned(); t765[0] = vsetq_lane_u32::<0>(v as u32, t765[0]); t765[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t765[2]); }
        { let v = (t764[1] as *const u64).read_unaligned(); t765[0] = vsetq_lane_u32::<1>(v as u32, t765[0]); t765[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t765[2]); }
        { let v = (t764[2] as *const u64).read_unaligned(); t765[0] = vsetq_lane_u32::<2>(v as u32, t765[0]); t765[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t765[2]); }
        { let v = (t764[3] as *const u64).read_unaligned(); t765[0] = vsetq_lane_u32::<3>(v as u32, t765[0]); t765[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t765[2]); }
        { let v = (t764[4] as *const u64).read_unaligned(); t765[1] = vsetq_lane_u32::<0>(v as u32, t765[1]); t765[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t765[3]); }
        { let v = (t764[5] as *const u64).read_unaligned(); t765[1] = vsetq_lane_u32::<1>(v as u32, t765[1]); t765[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t765[3]); }
        { let v = (t764[6] as *const u64).read_unaligned(); t765[1] = vsetq_lane_u32::<2>(v as u32, t765[1]); t765[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t765[3]); }
        { let v = (t764[7] as *const u64).read_unaligned(); t765[1] = vsetq_lane_u32::<3>(v as u32, t765[1]); t765[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t765[3]); }
        res = t765;
        return res;
    }
    if ((x2) == (4u32)) {
        let t766: u64 = ((arg0) as usize as u64);
        let t767 = [vdupq_n_u32((t766) as u32); 2];
        let t768 = [vdupq_n_u32(((t766) >> 32) as u32); 2];
        let t769: [uint32x4_t; 2] = [vaddq_u32(t767[0], x1[0]), vaddq_u32(t767[1], x1[1])];
        let t770: [uint32x4_t; 2] = [vcltq_u32(t769[0], t767[0]), vcltq_u32(t769[1], t767[1])];
        let t771: [uint32x4_t; 4] = [t769[0], t769[1], vsubq_u32(vaddq_u32(t768[0], x1[2]), t770[0]), vsubq_u32(vaddq_u32(t768[1], x1[3]), t770[1])];
        let t772 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([t771[0], t771[1]]);
        let t773 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([t771[2], t771[3]]);
        let t774: [usize; 8] = [((t773[0] as u64) << 32 | t772[0] as u64) as usize, ((t773[1] as u64) << 32 | t772[1] as u64) as usize, ((t773[2] as u64) << 32 | t772[2] as u64) as usize, ((t773[3] as u64) << 32 | t772[3] as u64) as usize, ((t773[4] as u64) << 32 | t772[4] as u64) as usize, ((t773[5] as u64) << 32 | t772[5] as u64) as usize, ((t773[6] as u64) << 32 | t772[6] as u64) as usize, ((t773[7] as u64) << 32 | t772[7] as u64) as usize];
        let mut t775: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t775[0] = vsetq_lane_u32::<0>((t774[0] as *const u32).read_unaligned() as u32, t775[0]);
        t775[0] = vsetq_lane_u32::<1>((t774[1] as *const u32).read_unaligned() as u32, t775[0]);
        t775[0] = vsetq_lane_u32::<2>((t774[2] as *const u32).read_unaligned() as u32, t775[0]);
        t775[0] = vsetq_lane_u32::<3>((t774[3] as *const u32).read_unaligned() as u32, t775[0]);
        t775[1] = vsetq_lane_u32::<0>((t774[4] as *const u32).read_unaligned() as u32, t775[1]);
        t775[1] = vsetq_lane_u32::<1>((t774[5] as *const u32).read_unaligned() as u32, t775[1]);
        t775[1] = vsetq_lane_u32::<2>((t774[6] as *const u32).read_unaligned() as u32, t775[1]);
        t775[1] = vsetq_lane_u32::<3>((t774[7] as *const u32).read_unaligned() as u32, t775[1]);
        let t776: [uint32x4_t; 4] = [t775[0], t775[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t776;
        return res;
    }
    if ((x2) == (2u32)) {
        let t777: u64 = ((arg0) as usize as u64);
        let t778 = [vdupq_n_u32((t777) as u32); 2];
        let t779 = [vdupq_n_u32(((t777) >> 32) as u32); 2];
        let t780: [uint32x4_t; 2] = [vaddq_u32(t778[0], x1[0]), vaddq_u32(t778[1], x1[1])];
        let t781: [uint32x4_t; 2] = [vcltq_u32(t780[0], t778[0]), vcltq_u32(t780[1], t778[1])];
        let t782: [uint32x4_t; 4] = [t780[0], t780[1], vsubq_u32(vaddq_u32(t779[0], x1[2]), t781[0]), vsubq_u32(vaddq_u32(t779[1], x1[3]), t781[1])];
        let t783 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([t782[0], t782[1]]);
        let t784 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([t782[2], t782[3]]);
        let t785: [usize; 8] = [((t784[0] as u64) << 32 | t783[0] as u64) as usize, ((t784[1] as u64) << 32 | t783[1] as u64) as usize, ((t784[2] as u64) << 32 | t783[2] as u64) as usize, ((t784[3] as u64) << 32 | t783[3] as u64) as usize, ((t784[4] as u64) << 32 | t783[4] as u64) as usize, ((t784[5] as u64) << 32 | t783[5] as u64) as usize, ((t784[6] as u64) << 32 | t783[6] as u64) as usize, ((t784[7] as u64) << 32 | t783[7] as u64) as usize];
        let mut t786: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t786[0] = vsetq_lane_u32::<0>((t785[0] as *const u16).read_unaligned() as u32, t786[0]);
        t786[0] = vsetq_lane_u32::<1>((t785[1] as *const u16).read_unaligned() as u32, t786[0]);
        t786[0] = vsetq_lane_u32::<2>((t785[2] as *const u16).read_unaligned() as u32, t786[0]);
        t786[0] = vsetq_lane_u32::<3>((t785[3] as *const u16).read_unaligned() as u32, t786[0]);
        t786[1] = vsetq_lane_u32::<0>((t785[4] as *const u16).read_unaligned() as u32, t786[1]);
        t786[1] = vsetq_lane_u32::<1>((t785[5] as *const u16).read_unaligned() as u32, t786[1]);
        t786[1] = vsetq_lane_u32::<2>((t785[6] as *const u16).read_unaligned() as u32, t786[1]);
        t786[1] = vsetq_lane_u32::<3>((t785[7] as *const u16).read_unaligned() as u32, t786[1]);
        let t787: [uint32x4_t; 4] = [t786[0], t786[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t787;
        return res;
    }
    if ((x2) == (1u32)) {
        let t788: u64 = ((arg0) as usize as u64);
        let t789 = [vdupq_n_u32((t788) as u32); 2];
        let t790 = [vdupq_n_u32(((t788) >> 32) as u32); 2];
        let t791: [uint32x4_t; 2] = [vaddq_u32(t789[0], x1[0]), vaddq_u32(t789[1], x1[1])];
        let t792: [uint32x4_t; 2] = [vcltq_u32(t791[0], t789[0]), vcltq_u32(t791[1], t789[1])];
        let t793: [uint32x4_t; 4] = [t791[0], t791[1], vsubq_u32(vaddq_u32(t790[0], x1[2]), t792[0]), vsubq_u32(vaddq_u32(t790[1], x1[3]), t792[1])];
        let t794 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([t793[0], t793[1]]);
        let t795 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([t793[2], t793[3]]);
        let t796: [usize; 8] = [((t795[0] as u64) << 32 | t794[0] as u64) as usize, ((t795[1] as u64) << 32 | t794[1] as u64) as usize, ((t795[2] as u64) << 32 | t794[2] as u64) as usize, ((t795[3] as u64) << 32 | t794[3] as u64) as usize, ((t795[4] as u64) << 32 | t794[4] as u64) as usize, ((t795[5] as u64) << 32 | t794[5] as u64) as usize, ((t795[6] as u64) << 32 | t794[6] as u64) as usize, ((t795[7] as u64) << 32 | t794[7] as u64) as usize];
        let mut t797: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t797[0] = vsetq_lane_u32::<0>((t796[0] as *const u8).read_unaligned() as u32, t797[0]);
        t797[0] = vsetq_lane_u32::<1>((t796[1] as *const u8).read_unaligned() as u32, t797[0]);
        t797[0] = vsetq_lane_u32::<2>((t796[2] as *const u8).read_unaligned() as u32, t797[0]);
        t797[0] = vsetq_lane_u32::<3>((t796[3] as *const u8).read_unaligned() as u32, t797[0]);
        t797[1] = vsetq_lane_u32::<0>((t796[4] as *const u8).read_unaligned() as u32, t797[1]);
        t797[1] = vsetq_lane_u32::<1>((t796[5] as *const u8).read_unaligned() as u32, t797[1]);
        t797[1] = vsetq_lane_u32::<2>((t796[6] as *const u8).read_unaligned() as u32, t797[1]);
        t797[1] = vsetq_lane_u32::<3>((t796[7] as *const u8).read_unaligned() as u32, t797[1]);
        let t798: [uint32x4_t; 4] = [t797[0], t797[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t798;
        return res;
    }
    let t799: u64 = !(7u64);
    let t800 = [vdupq_n_u32((t799) as u32); 2];
    let t801 = [vdupq_n_u32(((t799) >> 32) as u32); 2];
    let t802: [uint32x4_t; 4] = [vandq_u32(x1[0], t800[0]), vandq_u32(x1[1], t800[1]), vandq_u32(x1[2], t801[0]), vandq_u32(x1[3], t801[1])];
    let t803: u64 = ((arg0) as usize as u64);
    let t804 = [vdupq_n_u32((t803) as u32); 2];
    let t805 = [vdupq_n_u32(((t803) >> 32) as u32); 2];
    let t806: [uint32x4_t; 2] = [vaddq_u32(t804[0], t802[0]), vaddq_u32(t804[1], t802[1])];
    let t807: [uint32x4_t; 2] = [vcltq_u32(t806[0], t804[0]), vcltq_u32(t806[1], t804[1])];
    let t808: [uint32x4_t; 4] = [t806[0], t806[1], vsubq_u32(vaddq_u32(t805[0], t802[2]), t807[0]), vsubq_u32(vaddq_u32(t805[1], t802[3]), t807[1])];
    x3 = t808;
    let t809: u64 = 7u64;
    let t810 = [vdupq_n_u32((t809) as u32); 2];
    let t811: [uint32x4_t; 2] = [vandq_u32(x1[0], t810[0]), vandq_u32(x1[1], t810[1])];
    let t812 = [vdupq_n_u32(8u32); 2];
    let t813: [uint32x4_t; 2] = [vmulq_u32(t811[0], t812[0]), vmulq_u32(t811[1], t812[1])];
    x4 = t813;
    let t814 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([x3[0], x3[1]]);
    let t815 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([x3[2], x3[3]]);
    let t816: [usize; 8] = [((t815[0] as u64) << 32 | t814[0] as u64) as usize, ((t815[1] as u64) << 32 | t814[1] as u64) as usize, ((t815[2] as u64) << 32 | t814[2] as u64) as usize, ((t815[3] as u64) << 32 | t814[3] as u64) as usize, ((t815[4] as u64) << 32 | t814[4] as u64) as usize, ((t815[5] as u64) << 32 | t814[5] as u64) as usize, ((t815[6] as u64) << 32 | t814[6] as u64) as usize, ((t815[7] as u64) << 32 | t814[7] as u64) as usize];
    let mut t817: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (t816[0] as *const u64).read_unaligned(); t817[0] = vsetq_lane_u32::<0>(v as u32, t817[0]); t817[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t817[2]); }
    { let v = (t816[1] as *const u64).read_unaligned(); t817[0] = vsetq_lane_u32::<1>(v as u32, t817[0]); t817[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t817[2]); }
    { let v = (t816[2] as *const u64).read_unaligned(); t817[0] = vsetq_lane_u32::<2>(v as u32, t817[0]); t817[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t817[2]); }
    { let v = (t816[3] as *const u64).read_unaligned(); t817[0] = vsetq_lane_u32::<3>(v as u32, t817[0]); t817[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t817[2]); }
    { let v = (t816[4] as *const u64).read_unaligned(); t817[1] = vsetq_lane_u32::<0>(v as u32, t817[1]); t817[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t817[3]); }
    { let v = (t816[5] as *const u64).read_unaligned(); t817[1] = vsetq_lane_u32::<1>(v as u32, t817[1]); t817[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t817[3]); }
    { let v = (t816[6] as *const u64).read_unaligned(); t817[1] = vsetq_lane_u32::<2>(v as u32, t817[1]); t817[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t817[3]); }
    { let v = (t816[7] as *const u64).read_unaligned(); t817[1] = vsetq_lane_u32::<3>(v as u32, t817[1]); t817[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t817[3]); }
    let t818: [uint32x4_t; 2] = [x4[0], x4[1]];
    let t819: [uint32x4_t; 2] = [vandq_u32(t818[0], vdupq_n_u32(63)), vandq_u32(t818[1], vdupq_n_u32(63))];
    let t820: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t817[0], vnegq_s32(vreinterpretq_s32_u32(t819[0]))), vshlq_u32(t817[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t819[0])))), vorrq_u32(vshlq_u32(t817[1], vnegq_s32(vreinterpretq_s32_u32(t819[1]))), vshlq_u32(t817[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t819[1])))), vshlq_u32(t817[2], vnegq_s32(vreinterpretq_s32_u32(t819[0]))), vshlq_u32(t817[3], vnegq_s32(vreinterpretq_s32_u32(t819[1])))];
    x5 = t820;
    let t821 = [vdupq_n_u32(0u32); 2];
    let t822: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t821[0])), vmvnq_u32(vceqq_u32(x4[1], t821[1]))];
    let t823 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
    let t824: [uint32x4_t; 2] = [vaddq_u32(x4[0], t823[0]), vaddq_u32(x4[1], t823[1])];
    let t825 = [vdupq_n_u32(64u32); 2];
    let t826: [uint32x4_t; 2] = [vcgtq_u32(t824[0], t825[0]), vcgtq_u32(t824[1], t825[1])];
    let t827: [uint32x4_t; 2] = [vandq_u32(t822[0], t826[0]), vandq_u32(t822[1], t826[1])];
    let mut t828: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t829: [uint32x4_t; 2] = [vmvnq_u32(t827[0]), vmvnq_u32(t827[1])];
    {
        let t830: u64 = (1u64).wrapping_mul(8u64);
        let t831 = [vdupq_n_u32((t830) as u32); 2];
        let t832 = [vdupq_n_u32(((t830) >> 32) as u32); 2];
        let t833: [uint32x4_t; 2] = [vaddq_u32(x3[0], t831[0]), vaddq_u32(x3[1], t831[1])];
        let t834: [uint32x4_t; 2] = [vcltq_u32(t833[0], x3[0]), vcltq_u32(t833[1], x3[1])];
        let t835: [uint32x4_t; 4] = [t833[0], t833[1], vsubq_u32(vaddq_u32(x3[2], t832[0]), t834[0]), vsubq_u32(vaddq_u32(x3[3], t832[1]), t834[1])];
        let t836 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([t835[0], t835[1]]);
        let t837 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([t835[2], t835[3]]);
        let t839 = bits(&t827);
        let t840 = t839.trailing_zeros() as usize % 8;
        let t838: [usize; 8] = [{ let j = if t839 >> 0 & 1 != 0 { 0 } else { t840 }; ((t837[j] as u64) << 32 | t836[j] as u64) as usize }, { let j = if t839 >> 1 & 1 != 0 { 1 } else { t840 }; ((t837[j] as u64) << 32 | t836[j] as u64) as usize }, { let j = if t839 >> 2 & 1 != 0 { 2 } else { t840 }; ((t837[j] as u64) << 32 | t836[j] as u64) as usize }, { let j = if t839 >> 3 & 1 != 0 { 3 } else { t840 }; ((t837[j] as u64) << 32 | t836[j] as u64) as usize }, { let j = if t839 >> 4 & 1 != 0 { 4 } else { t840 }; ((t837[j] as u64) << 32 | t836[j] as u64) as usize }, { let j = if t839 >> 5 & 1 != 0 { 5 } else { t840 }; ((t837[j] as u64) << 32 | t836[j] as u64) as usize }, { let j = if t839 >> 6 & 1 != 0 { 6 } else { t840 }; ((t837[j] as u64) << 32 | t836[j] as u64) as usize }, { let j = if t839 >> 7 & 1 != 0 { 7 } else { t840 }; ((t837[j] as u64) << 32 | t836[j] as u64) as usize }];
        let mut t841: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (t838[0] as *const u64).read_unaligned(); t841[0] = vsetq_lane_u32::<0>(v as u32, t841[0]); t841[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t841[2]); }
        { let v = (t838[1] as *const u64).read_unaligned(); t841[0] = vsetq_lane_u32::<1>(v as u32, t841[0]); t841[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t841[2]); }
        { let v = (t838[2] as *const u64).read_unaligned(); t841[0] = vsetq_lane_u32::<2>(v as u32, t841[0]); t841[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t841[2]); }
        { let v = (t838[3] as *const u64).read_unaligned(); t841[0] = vsetq_lane_u32::<3>(v as u32, t841[0]); t841[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t841[2]); }
        { let v = (t838[4] as *const u64).read_unaligned(); t841[1] = vsetq_lane_u32::<0>(v as u32, t841[1]); t841[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t841[3]); }
        { let v = (t838[5] as *const u64).read_unaligned(); t841[1] = vsetq_lane_u32::<1>(v as u32, t841[1]); t841[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t841[3]); }
        { let v = (t838[6] as *const u64).read_unaligned(); t841[1] = vsetq_lane_u32::<2>(v as u32, t841[1]); t841[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t841[3]); }
        { let v = (t838[7] as *const u64).read_unaligned(); t841[1] = vsetq_lane_u32::<3>(v as u32, t841[1]); t841[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t841[3]); }
        let t842: [uint32x4_t; 2] = [vsubq_u32(t825[0], x4[0]), vsubq_u32(t825[1], x4[1])];
        let t843: [uint32x4_t; 2] = [t842[0], t842[1]];
        let t844: [uint32x4_t; 2] = [vandq_u32(t843[0], vdupq_n_u32(63)), vandq_u32(t843[1], vdupq_n_u32(63))];
        let t845: [uint32x4_t; 4] = [vshlq_u32(t841[0], vreinterpretq_s32_u32(t844[0])), vshlq_u32(t841[1], vreinterpretq_s32_u32(t844[1])), vorrq_u32(vshlq_u32(t841[2], vreinterpretq_s32_u32(t844[0])), vshlq_u32(t841[0], vsubq_s32(vreinterpretq_s32_u32(t844[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t841[3], vreinterpretq_s32_u32(t844[1])), vshlq_u32(t841[1], vsubq_s32(vreinterpretq_s32_u32(t844[1]), vdupq_n_s32(32))))];
        let t846: [uint32x4_t; 4] = [vorrq_u32(x5[0], t845[0]), vorrq_u32(x5[1], t845[1]), vorrq_u32(x5[2], t845[2]), vorrq_u32(x5[3], t845[3])];
        t828 = [vbslq_u32(t827[0], t846[0], t828[0]), vbslq_u32(t827[1], t846[1], t828[1]), vbslq_u32(t827[0], t846[2], t828[2]), vbslq_u32(t827[1], t846[3], t828[3])];
    }
    {
        t828 = [vbslq_u32(t829[0], x5[0], t828[0]), vbslq_u32(t829[1], x5[1], t828[1]), vbslq_u32(t829[0], x5[2], t828[2]), vbslq_u32(t829[1], x5[3], t828[3])];
    }
    res = t828;
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
        let t847 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], x1i[0]), vandq_u32(live[1], x1i[1]))));
        let t848: [uint32x4_t; 2] = [vbslq_u32(live[0], x1i[0], t847), vbslq_u32(live[1], x1i[1], t847)];
        let mut t849: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t850 = vminvq_u32(vminq_u32(t848[0], t848[1]));
        if t850 == vmaxvq_u32(vmaxq_u32(t848[0], t848[1])) {
            let t851 = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(t850 as usize * 1) as *const u64).read_unaligned();
            t849 = [vdupq_n_u32(t851 as u32), vdupq_n_u32(t851 as u32), vdupq_n_u32((t851 >> 32) as u32), vdupq_n_u32((t851 >> 32) as u32)];
        } else {
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t848[0]) as usize * 1) as *const u64).read_unaligned(); t849[0] = vsetq_lane_u32::<0>(v as u32, t849[0]); t849[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t849[2]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t848[0]) as usize * 1) as *const u64).read_unaligned(); t849[0] = vsetq_lane_u32::<1>(v as u32, t849[0]); t849[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t849[2]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t848[0]) as usize * 1) as *const u64).read_unaligned(); t849[0] = vsetq_lane_u32::<2>(v as u32, t849[0]); t849[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t849[2]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t848[0]) as usize * 1) as *const u64).read_unaligned(); t849[0] = vsetq_lane_u32::<3>(v as u32, t849[0]); t849[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t849[2]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t848[1]) as usize * 1) as *const u64).read_unaligned(); t849[1] = vsetq_lane_u32::<0>(v as u32, t849[1]); t849[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t849[3]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t848[1]) as usize * 1) as *const u64).read_unaligned(); t849[1] = vsetq_lane_u32::<1>(v as u32, t849[1]); t849[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t849[3]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t848[1]) as usize * 1) as *const u64).read_unaligned(); t849[1] = vsetq_lane_u32::<2>(v as u32, t849[1]); t849[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t849[3]); }
            { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t848[1]) as usize * 1) as *const u64).read_unaligned(); t849[1] = vsetq_lane_u32::<3>(v as u32, t849[1]); t849[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t849[3]); }
        }
        res = t849;
        return res;
    }
    if ((x2) == (4u32)) {
        let t852 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], x1i[0]), vandq_u32(live[1], x1i[1]))));
        let t853: [uint32x4_t; 2] = [vbslq_u32(live[0], x1i[0], t852), vbslq_u32(live[1], x1i[1], t852)];
        let mut t854: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t855 = vminvq_u32(vminq_u32(t853[0], t853[1]));
        if t855 == vmaxvq_u32(vmaxq_u32(t853[0], t853[1])) {
            t854 = [vdupq_n_u32(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(t855 as usize * 1) as *const u32).read_unaligned() as u32)); 2];
        } else {
            t854[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t853[0]) as usize * 1) as *const u32).read_unaligned() as u32), t854[0]);
            t854[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t853[0]) as usize * 1) as *const u32).read_unaligned() as u32), t854[0]);
            t854[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t853[0]) as usize * 1) as *const u32).read_unaligned() as u32), t854[0]);
            t854[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t853[0]) as usize * 1) as *const u32).read_unaligned() as u32), t854[0]);
            t854[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t853[1]) as usize * 1) as *const u32).read_unaligned() as u32), t854[1]);
            t854[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t853[1]) as usize * 1) as *const u32).read_unaligned() as u32), t854[1]);
            t854[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t853[1]) as usize * 1) as *const u32).read_unaligned() as u32), t854[1]);
            t854[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t853[1]) as usize * 1) as *const u32).read_unaligned() as u32), t854[1]);
        }
        let t856: [uint32x4_t; 4] = [t854[0], t854[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t856;
        return res;
    }
    if ((x2) == (2u32)) {
        let t857 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], x1i[0]), vandq_u32(live[1], x1i[1]))));
        let t858: [uint32x4_t; 2] = [vbslq_u32(live[0], x1i[0], t857), vbslq_u32(live[1], x1i[1], t857)];
        let mut t859: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t860 = vminvq_u32(vminq_u32(t858[0], t858[1]));
        if t860 == vmaxvq_u32(vmaxq_u32(t858[0], t858[1])) {
            t859 = [vdupq_n_u32(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(t860 as usize * 1) as *const u16).read_unaligned() as u32)); 2];
        } else {
            t859[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t858[0]) as usize * 1) as *const u16).read_unaligned() as u32), t859[0]);
            t859[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t858[0]) as usize * 1) as *const u16).read_unaligned() as u32), t859[0]);
            t859[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t858[0]) as usize * 1) as *const u16).read_unaligned() as u32), t859[0]);
            t859[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t858[0]) as usize * 1) as *const u16).read_unaligned() as u32), t859[0]);
            t859[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t858[1]) as usize * 1) as *const u16).read_unaligned() as u32), t859[1]);
            t859[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t858[1]) as usize * 1) as *const u16).read_unaligned() as u32), t859[1]);
            t859[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t858[1]) as usize * 1) as *const u16).read_unaligned() as u32), t859[1]);
            t859[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t858[1]) as usize * 1) as *const u16).read_unaligned() as u32), t859[1]);
        }
        let t861: [uint32x4_t; 4] = [t859[0], t859[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t861;
        return res;
    }
    if ((x2) == (1u32)) {
        let t862 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], x1i[0]), vandq_u32(live[1], x1i[1]))));
        let t863: [uint32x4_t; 2] = [vbslq_u32(live[0], x1i[0], t862), vbslq_u32(live[1], x1i[1], t862)];
        let mut t864: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t865 = vminvq_u32(vminq_u32(t863[0], t863[1]));
        if t865 == vmaxvq_u32(vmaxq_u32(t863[0], t863[1])) {
            t864 = [vdupq_n_u32(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(t865 as usize * 1) as *const u8).read_unaligned() as u32)); 2];
        } else {
            t864[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t863[0]) as usize * 1) as *const u8).read_unaligned() as u32), t864[0]);
            t864[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t863[0]) as usize * 1) as *const u8).read_unaligned() as u32), t864[0]);
            t864[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t863[0]) as usize * 1) as *const u8).read_unaligned() as u32), t864[0]);
            t864[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t863[0]) as usize * 1) as *const u8).read_unaligned() as u32), t864[0]);
            t864[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t863[1]) as usize * 1) as *const u8).read_unaligned() as u32), t864[1]);
            t864[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t863[1]) as usize * 1) as *const u8).read_unaligned() as u32), t864[1]);
            t864[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t863[1]) as usize * 1) as *const u8).read_unaligned() as u32), t864[1]);
            t864[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t863[1]) as usize * 1) as *const u8).read_unaligned() as u32), t864[1]);
        }
        let t866: [uint32x4_t; 4] = [t864[0], t864[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t866;
        return res;
    }
    let t867: [uint32x4_t; 2] = [x1i[0], x1i[1]];
    let t868: u64 = ((x1b) as usize as u64);
    let t869 = [vdupq_n_u32((t868) as u32); 2];
    let t870 = [vdupq_n_u32(((t868) >> 32) as u32); 2];
    let t871: [uint32x4_t; 2] = [vaddq_u32(t869[0], t867[0]), vaddq_u32(t869[1], t867[1])];
    let t872: [uint32x4_t; 2] = [vcltq_u32(t871[0], t869[0]), vcltq_u32(t871[1], t869[1])];
    let t873: [uint32x4_t; 4] = [t871[0], t871[1], vsubq_u32(vaddq_u32(t870[0], vdupq_n_u32(0)), t872[0]), vsubq_u32(vaddq_u32(t870[1], vdupq_n_u32(0)), t872[1])];
    let t874: u64 = !(7u64);
    let t875 = [vdupq_n_u32((t874) as u32); 2];
    let t876 = [vdupq_n_u32(((t874) >> 32) as u32); 2];
    let t877: [uint32x4_t; 4] = [vandq_u32(t873[0], t875[0]), vandq_u32(t873[1], t875[1]), vandq_u32(t873[2], t876[0]), vandq_u32(t873[3], t876[1])];
    let t878: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
    let t879: [uint32x4_t; 4] = [vandq_u32(t877[0], t878[0]), vandq_u32(t877[1], t878[1]), vandq_u32(t877[2], t878[2]), vandq_u32(t877[3], t878[3])];
    let t880: u64 = ((arg0) as usize as u64);
    let t881 = [vdupq_n_u32((t880) as u32); 2];
    let t882 = [vdupq_n_u32(((t880) >> 32) as u32); 2];
    let t883: [uint32x4_t; 2] = [vaddq_u32(t881[0], t879[0]), vaddq_u32(t881[1], t879[1])];
    let t884: [uint32x4_t; 2] = [vcltq_u32(t883[0], t881[0]), vcltq_u32(t883[1], t881[1])];
    let t885: [uint32x4_t; 4] = [t883[0], t883[1], vsubq_u32(vaddq_u32(t882[0], t879[2]), t884[0]), vsubq_u32(vaddq_u32(t882[1], t879[3]), t884[1])];
    x3 = t885;
    let t886: [uint32x4_t; 2] = [x1i[0], x1i[1]];
    let t887: u64 = ((x1b) as usize as u64);
    let t888 = [vdupq_n_u32((t887) as u32); 2];
    let t889 = [vdupq_n_u32(((t887) >> 32) as u32); 2];
    let t890: [uint32x4_t; 2] = [vaddq_u32(t888[0], t886[0]), vaddq_u32(t888[1], t886[1])];
    let t891: [uint32x4_t; 2] = [vcltq_u32(t890[0], t888[0]), vcltq_u32(t890[1], t888[1])];
    let t892: [uint32x4_t; 4] = [t890[0], t890[1], vsubq_u32(vaddq_u32(t889[0], vdupq_n_u32(0)), t891[0]), vsubq_u32(vaddq_u32(t889[1], vdupq_n_u32(0)), t891[1])];
    let t893: u64 = 7u64;
    let t894 = [vdupq_n_u32((t893) as u32); 2];
    let t895: [uint32x4_t; 2] = [vandq_u32(t892[0], t894[0]), vandq_u32(t892[1], t894[1])];
    let t896 = [vdupq_n_u32(8u32); 2];
    let t897: [uint32x4_t; 2] = [vmulq_u32(t895[0], t896[0]), vmulq_u32(t895[1], t896[1])];
    x4 = t897;
    let t898 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([x3[0], x3[1]]);
    let t899 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([x3[2], x3[3]]);
    let t901 = bits(&live);
    let t902 = t901.trailing_zeros() as usize % 8;
    let t900: [usize; 8] = [{ let j = if t901 >> 0 & 1 != 0 { 0 } else { t902 }; ((t899[j] as u64) << 32 | t898[j] as u64) as usize }, { let j = if t901 >> 1 & 1 != 0 { 1 } else { t902 }; ((t899[j] as u64) << 32 | t898[j] as u64) as usize }, { let j = if t901 >> 2 & 1 != 0 { 2 } else { t902 }; ((t899[j] as u64) << 32 | t898[j] as u64) as usize }, { let j = if t901 >> 3 & 1 != 0 { 3 } else { t902 }; ((t899[j] as u64) << 32 | t898[j] as u64) as usize }, { let j = if t901 >> 4 & 1 != 0 { 4 } else { t902 }; ((t899[j] as u64) << 32 | t898[j] as u64) as usize }, { let j = if t901 >> 5 & 1 != 0 { 5 } else { t902 }; ((t899[j] as u64) << 32 | t898[j] as u64) as usize }, { let j = if t901 >> 6 & 1 != 0 { 6 } else { t902 }; ((t899[j] as u64) << 32 | t898[j] as u64) as usize }, { let j = if t901 >> 7 & 1 != 0 { 7 } else { t902 }; ((t899[j] as u64) << 32 | t898[j] as u64) as usize }];
    let mut t903: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (t900[0] as *const u64).read_unaligned(); t903[0] = vsetq_lane_u32::<0>(v as u32, t903[0]); t903[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t903[2]); }
    { let v = (t900[1] as *const u64).read_unaligned(); t903[0] = vsetq_lane_u32::<1>(v as u32, t903[0]); t903[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t903[2]); }
    { let v = (t900[2] as *const u64).read_unaligned(); t903[0] = vsetq_lane_u32::<2>(v as u32, t903[0]); t903[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t903[2]); }
    { let v = (t900[3] as *const u64).read_unaligned(); t903[0] = vsetq_lane_u32::<3>(v as u32, t903[0]); t903[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t903[2]); }
    { let v = (t900[4] as *const u64).read_unaligned(); t903[1] = vsetq_lane_u32::<0>(v as u32, t903[1]); t903[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t903[3]); }
    { let v = (t900[5] as *const u64).read_unaligned(); t903[1] = vsetq_lane_u32::<1>(v as u32, t903[1]); t903[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t903[3]); }
    { let v = (t900[6] as *const u64).read_unaligned(); t903[1] = vsetq_lane_u32::<2>(v as u32, t903[1]); t903[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t903[3]); }
    { let v = (t900[7] as *const u64).read_unaligned(); t903[1] = vsetq_lane_u32::<3>(v as u32, t903[1]); t903[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t903[3]); }
    let t904: [uint32x4_t; 2] = [x4[0], x4[1]];
    let t905: [uint32x4_t; 2] = [vandq_u32(t904[0], vdupq_n_u32(63)), vandq_u32(t904[1], vdupq_n_u32(63))];
    let t906: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t903[0], vnegq_s32(vreinterpretq_s32_u32(t905[0]))), vshlq_u32(t903[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t905[0])))), vorrq_u32(vshlq_u32(t903[1], vnegq_s32(vreinterpretq_s32_u32(t905[1]))), vshlq_u32(t903[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t905[1])))), vshlq_u32(t903[2], vnegq_s32(vreinterpretq_s32_u32(t905[0]))), vshlq_u32(t903[3], vnegq_s32(vreinterpretq_s32_u32(t905[1])))];
    x5 = t906;
    let t907 = [vdupq_n_u32(0u32); 2];
    let t908: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t907[0])), vmvnq_u32(vceqq_u32(x4[1], t907[1]))];
    let t909 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
    let t910: [uint32x4_t; 2] = [vaddq_u32(x4[0], t909[0]), vaddq_u32(x4[1], t909[1])];
    let t911 = [vdupq_n_u32(64u32); 2];
    let t912: [uint32x4_t; 2] = [vcgtq_u32(t910[0], t911[0]), vcgtq_u32(t910[1], t911[1])];
    let t913: [uint32x4_t; 2] = [vandq_u32(t908[0], t912[0]), vandq_u32(t908[1], t912[1])];
    let mut t914: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t915: [uint32x4_t; 2] = [vandq_u32(live[0], t913[0]), vandq_u32(live[1], t913[1])];
    let t916: [uint32x4_t; 2] = [vbicq_u32(live[0], t913[0]), vbicq_u32(live[1], t913[1])];
    {
        let t917: u64 = (1u64).wrapping_mul(8u64);
        let t918 = [vdupq_n_u32((t917) as u32); 2];
        let t919 = [vdupq_n_u32(((t917) >> 32) as u32); 2];
        let t920: [uint32x4_t; 2] = [vaddq_u32(x3[0], t918[0]), vaddq_u32(x3[1], t918[1])];
        let t921: [uint32x4_t; 2] = [vcltq_u32(t920[0], x3[0]), vcltq_u32(t920[1], x3[1])];
        let t922: [uint32x4_t; 4] = [t920[0], t920[1], vsubq_u32(vaddq_u32(x3[2], t919[0]), t921[0]), vsubq_u32(vaddq_u32(x3[3], t919[1]), t921[1])];
        let t923 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([t922[0], t922[1]]);
        let t924 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([t922[2], t922[3]]);
        let t926 = bits(&t915);
        let t927 = t926.trailing_zeros() as usize % 8;
        let t925: [usize; 8] = [{ let j = if t926 >> 0 & 1 != 0 { 0 } else { t927 }; ((t924[j] as u64) << 32 | t923[j] as u64) as usize }, { let j = if t926 >> 1 & 1 != 0 { 1 } else { t927 }; ((t924[j] as u64) << 32 | t923[j] as u64) as usize }, { let j = if t926 >> 2 & 1 != 0 { 2 } else { t927 }; ((t924[j] as u64) << 32 | t923[j] as u64) as usize }, { let j = if t926 >> 3 & 1 != 0 { 3 } else { t927 }; ((t924[j] as u64) << 32 | t923[j] as u64) as usize }, { let j = if t926 >> 4 & 1 != 0 { 4 } else { t927 }; ((t924[j] as u64) << 32 | t923[j] as u64) as usize }, { let j = if t926 >> 5 & 1 != 0 { 5 } else { t927 }; ((t924[j] as u64) << 32 | t923[j] as u64) as usize }, { let j = if t926 >> 6 & 1 != 0 { 6 } else { t927 }; ((t924[j] as u64) << 32 | t923[j] as u64) as usize }, { let j = if t926 >> 7 & 1 != 0 { 7 } else { t927 }; ((t924[j] as u64) << 32 | t923[j] as u64) as usize }];
        let mut t928: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (t925[0] as *const u64).read_unaligned(); t928[0] = vsetq_lane_u32::<0>(v as u32, t928[0]); t928[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t928[2]); }
        { let v = (t925[1] as *const u64).read_unaligned(); t928[0] = vsetq_lane_u32::<1>(v as u32, t928[0]); t928[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t928[2]); }
        { let v = (t925[2] as *const u64).read_unaligned(); t928[0] = vsetq_lane_u32::<2>(v as u32, t928[0]); t928[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t928[2]); }
        { let v = (t925[3] as *const u64).read_unaligned(); t928[0] = vsetq_lane_u32::<3>(v as u32, t928[0]); t928[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t928[2]); }
        { let v = (t925[4] as *const u64).read_unaligned(); t928[1] = vsetq_lane_u32::<0>(v as u32, t928[1]); t928[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t928[3]); }
        { let v = (t925[5] as *const u64).read_unaligned(); t928[1] = vsetq_lane_u32::<1>(v as u32, t928[1]); t928[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t928[3]); }
        { let v = (t925[6] as *const u64).read_unaligned(); t928[1] = vsetq_lane_u32::<2>(v as u32, t928[1]); t928[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t928[3]); }
        { let v = (t925[7] as *const u64).read_unaligned(); t928[1] = vsetq_lane_u32::<3>(v as u32, t928[1]); t928[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t928[3]); }
        let t929: [uint32x4_t; 2] = [vsubq_u32(t911[0], x4[0]), vsubq_u32(t911[1], x4[1])];
        let t930: [uint32x4_t; 2] = [t929[0], t929[1]];
        let t931: [uint32x4_t; 2] = [vandq_u32(t930[0], vdupq_n_u32(63)), vandq_u32(t930[1], vdupq_n_u32(63))];
        let t932: [uint32x4_t; 4] = [vshlq_u32(t928[0], vreinterpretq_s32_u32(t931[0])), vshlq_u32(t928[1], vreinterpretq_s32_u32(t931[1])), vorrq_u32(vshlq_u32(t928[2], vreinterpretq_s32_u32(t931[0])), vshlq_u32(t928[0], vsubq_s32(vreinterpretq_s32_u32(t931[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t928[3], vreinterpretq_s32_u32(t931[1])), vshlq_u32(t928[1], vsubq_s32(vreinterpretq_s32_u32(t931[1]), vdupq_n_s32(32))))];
        let t933: [uint32x4_t; 4] = [vorrq_u32(x5[0], t932[0]), vorrq_u32(x5[1], t932[1]), vorrq_u32(x5[2], t932[2]), vorrq_u32(x5[3], t932[3])];
        t914 = [vbslq_u32(t915[0], t933[0], t914[0]), vbslq_u32(t915[1], t933[1], t914[1]), vbslq_u32(t915[0], t933[2], t914[2]), vbslq_u32(t915[1], t933[3], t914[3])];
    }
    {
        t914 = [vbslq_u32(t916[0], x5[0], t914[0]), vbslq_u32(t916[1], x5[1], t914[1]), vbslq_u32(t916[0], x5[2], t914[2]), vbslq_u32(t916[1], x5[3], t914[3])];
    }
    res = t914;
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
        let t934: [uint32x4_t; 4];
        if (x1is) == 8u32 && ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 && all(&live) {
            let t935 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
            let t9360 = vld2q_u32((t935 as *const u32).add(0));
            let t9361 = vld2q_u32((t935 as *const u32).add(8));
            let t936: [uint32x4_t; 4] = [t9360.0, t9361.0, t9360.1, t9361.1];
            t934 = t936;
        } else {
            let mut t937: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            if ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 && all(&live) {
                let t938 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
                let t939 = ((x1is) as usize) * 1;
                { let v = (t938.add(0 * t939) as *const u64).read_unaligned(); t937[0] = vsetq_lane_u32::<0>(v as u32, t937[0]); t937[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t937[2]); }
                { let v = (t938.add(1 * t939) as *const u64).read_unaligned(); t937[0] = vsetq_lane_u32::<1>(v as u32, t937[0]); t937[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t937[2]); }
                { let v = (t938.add(2 * t939) as *const u64).read_unaligned(); t937[0] = vsetq_lane_u32::<2>(v as u32, t937[0]); t937[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t937[2]); }
                { let v = (t938.add(3 * t939) as *const u64).read_unaligned(); t937[0] = vsetq_lane_u32::<3>(v as u32, t937[0]); t937[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t937[2]); }
                { let v = (t938.add(4 * t939) as *const u64).read_unaligned(); t937[1] = vsetq_lane_u32::<0>(v as u32, t937[1]); t937[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t937[3]); }
                { let v = (t938.add(5 * t939) as *const u64).read_unaligned(); t937[1] = vsetq_lane_u32::<1>(v as u32, t937[1]); t937[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t937[3]); }
                { let v = (t938.add(6 * t939) as *const u64).read_unaligned(); t937[1] = vsetq_lane_u32::<2>(v as u32, t937[1]); t937[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t937[3]); }
                { let v = (t938.add(7 * t939) as *const u64).read_unaligned(); t937[1] = vsetq_lane_u32::<3>(v as u32, t937[1]); t937[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t937[3]); }
            } else {
                let t940: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
                let t941 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t940[0]), vandq_u32(live[1], t940[1]))));
                let t942: [uint32x4_t; 2] = [vbslq_u32(live[0], t940[0], t941), vbslq_u32(live[1], t940[1], t941)];
                let mut t943: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
                let t944 = vminvq_u32(vminq_u32(t942[0], t942[1]));
                if t944 == vmaxvq_u32(vmaxq_u32(t942[0], t942[1])) {
                    let t945 = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(t944 as usize * 1) as *const u64).read_unaligned();
                    t943 = [vdupq_n_u32(t945 as u32), vdupq_n_u32(t945 as u32), vdupq_n_u32((t945 >> 32) as u32), vdupq_n_u32((t945 >> 32) as u32)];
                } else {
                    { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t942[0]) as usize * 1) as *const u64).read_unaligned(); t943[0] = vsetq_lane_u32::<0>(v as u32, t943[0]); t943[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t943[2]); }
                    { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t942[0]) as usize * 1) as *const u64).read_unaligned(); t943[0] = vsetq_lane_u32::<1>(v as u32, t943[0]); t943[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t943[2]); }
                    { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t942[0]) as usize * 1) as *const u64).read_unaligned(); t943[0] = vsetq_lane_u32::<2>(v as u32, t943[0]); t943[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t943[2]); }
                    { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t942[0]) as usize * 1) as *const u64).read_unaligned(); t943[0] = vsetq_lane_u32::<3>(v as u32, t943[0]); t943[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t943[2]); }
                    { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t942[1]) as usize * 1) as *const u64).read_unaligned(); t943[1] = vsetq_lane_u32::<0>(v as u32, t943[1]); t943[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t943[3]); }
                    { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t942[1]) as usize * 1) as *const u64).read_unaligned(); t943[1] = vsetq_lane_u32::<1>(v as u32, t943[1]); t943[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t943[3]); }
                    { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t942[1]) as usize * 1) as *const u64).read_unaligned(); t943[1] = vsetq_lane_u32::<2>(v as u32, t943[1]); t943[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t943[3]); }
                    { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t942[1]) as usize * 1) as *const u64).read_unaligned(); t943[1] = vsetq_lane_u32::<3>(v as u32, t943[1]); t943[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t943[3]); }
                }
                t937 = t943;
            }
            t934 = t937;
        }
        res = t934;
        return res;
    }
    if ((x2) == (4u32)) {
        let t946: [uint32x4_t; 2];
        if (x1is) == 4u32 && ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 && all(&live) {
            let t947 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
            let t948: [uint32x4_t; 2] = core::array::from_fn(|r| vld1q_u32((t947 as *const u32).add(4 * r)));
            t946 = t948;
        } else {
            let mut t949: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            if ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 && all(&live) {
                let t950 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
                let t951 = ((x1is) as usize) * 1;
                t949[0] = vld1q_lane_u32::<0>(t950.add(0 * t951) as *const u32, t949[0]);
                t949[0] = vld1q_lane_u32::<1>(t950.add(1 * t951) as *const u32, t949[0]);
                t949[0] = vld1q_lane_u32::<2>(t950.add(2 * t951) as *const u32, t949[0]);
                t949[0] = vld1q_lane_u32::<3>(t950.add(3 * t951) as *const u32, t949[0]);
                t949[1] = vld1q_lane_u32::<0>(t950.add(4 * t951) as *const u32, t949[1]);
                t949[1] = vld1q_lane_u32::<1>(t950.add(5 * t951) as *const u32, t949[1]);
                t949[1] = vld1q_lane_u32::<2>(t950.add(6 * t951) as *const u32, t949[1]);
                t949[1] = vld1q_lane_u32::<3>(t950.add(7 * t951) as *const u32, t949[1]);
            } else {
                let t952: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
                let t953 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t952[0]), vandq_u32(live[1], t952[1]))));
                let t954: [uint32x4_t; 2] = [vbslq_u32(live[0], t952[0], t953), vbslq_u32(live[1], t952[1], t953)];
                let mut t955: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                let t956 = vminvq_u32(vminq_u32(t954[0], t954[1]));
                if t956 == vmaxvq_u32(vmaxq_u32(t954[0], t954[1])) {
                    t955 = [vdupq_n_u32(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(t956 as usize * 1) as *const u32).read_unaligned() as u32)); 2];
                } else {
                    t955[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t954[0]) as usize * 1) as *const u32).read_unaligned() as u32), t955[0]);
                    t955[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t954[0]) as usize * 1) as *const u32).read_unaligned() as u32), t955[0]);
                    t955[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t954[0]) as usize * 1) as *const u32).read_unaligned() as u32), t955[0]);
                    t955[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t954[0]) as usize * 1) as *const u32).read_unaligned() as u32), t955[0]);
                    t955[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t954[1]) as usize * 1) as *const u32).read_unaligned() as u32), t955[1]);
                    t955[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t954[1]) as usize * 1) as *const u32).read_unaligned() as u32), t955[1]);
                    t955[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t954[1]) as usize * 1) as *const u32).read_unaligned() as u32), t955[1]);
                    t955[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t954[1]) as usize * 1) as *const u32).read_unaligned() as u32), t955[1]);
                }
                t949 = t955;
            }
            t946 = t949;
        }
        let t957: [uint32x4_t; 4] = [t946[0], t946[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t957;
        return res;
    }
    if ((x2) == (2u32)) {
        let t958: [uint32x4_t; 2];
        if (x1is) == 2u32 && ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 && all(&live) {
            let t959 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
            let t960: [uint32x4_t; 2] = core::array::from_fn(|r| vmovl_u16(vld1_u16((t959 as *const u16).add(4 * r))));
            t958 = t960;
        } else {
            let mut t961: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            if ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 && all(&live) {
                let t962 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
                let t963 = ((x1is) as usize) * 1;
                t961[0] = vsetq_lane_u32::<0>((t962.add(0 * t963) as *const u16).read_unaligned() as u32, t961[0]);
                t961[0] = vsetq_lane_u32::<1>((t962.add(1 * t963) as *const u16).read_unaligned() as u32, t961[0]);
                t961[0] = vsetq_lane_u32::<2>((t962.add(2 * t963) as *const u16).read_unaligned() as u32, t961[0]);
                t961[0] = vsetq_lane_u32::<3>((t962.add(3 * t963) as *const u16).read_unaligned() as u32, t961[0]);
                t961[1] = vsetq_lane_u32::<0>((t962.add(4 * t963) as *const u16).read_unaligned() as u32, t961[1]);
                t961[1] = vsetq_lane_u32::<1>((t962.add(5 * t963) as *const u16).read_unaligned() as u32, t961[1]);
                t961[1] = vsetq_lane_u32::<2>((t962.add(6 * t963) as *const u16).read_unaligned() as u32, t961[1]);
                t961[1] = vsetq_lane_u32::<3>((t962.add(7 * t963) as *const u16).read_unaligned() as u32, t961[1]);
            } else {
                let t964: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
                let t965 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t964[0]), vandq_u32(live[1], t964[1]))));
                let t966: [uint32x4_t; 2] = [vbslq_u32(live[0], t964[0], t965), vbslq_u32(live[1], t964[1], t965)];
                let mut t967: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                let t968 = vminvq_u32(vminq_u32(t966[0], t966[1]));
                if t968 == vmaxvq_u32(vmaxq_u32(t966[0], t966[1])) {
                    t967 = [vdupq_n_u32(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(t968 as usize * 1) as *const u16).read_unaligned() as u32)); 2];
                } else {
                    t967[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t966[0]) as usize * 1) as *const u16).read_unaligned() as u32), t967[0]);
                    t967[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t966[0]) as usize * 1) as *const u16).read_unaligned() as u32), t967[0]);
                    t967[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t966[0]) as usize * 1) as *const u16).read_unaligned() as u32), t967[0]);
                    t967[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t966[0]) as usize * 1) as *const u16).read_unaligned() as u32), t967[0]);
                    t967[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t966[1]) as usize * 1) as *const u16).read_unaligned() as u32), t967[1]);
                    t967[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t966[1]) as usize * 1) as *const u16).read_unaligned() as u32), t967[1]);
                    t967[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t966[1]) as usize * 1) as *const u16).read_unaligned() as u32), t967[1]);
                    t967[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t966[1]) as usize * 1) as *const u16).read_unaligned() as u32), t967[1]);
                }
                t961 = t967;
            }
            t958 = t961;
        }
        let t969: [uint32x4_t; 4] = [t958[0], t958[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t969;
        return res;
    }
    if ((x2) == (1u32)) {
        let t970: [uint32x4_t; 2];
        if (x1is) == 1u32 && ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 && all(&live) {
            let t971 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
            let t973 = vmovl_u8(vld1_u8(t971.add(0)));
            let t972: [uint32x4_t; 2] = [vmovl_u16(vget_low_u16(t973)), vmovl_u16(vget_high_u16(t973))];
            t970 = t972;
        } else {
            let mut t974: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            if ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 && all(&live) {
                let t975 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
                let t976 = ((x1is) as usize) * 1;
                t974[0] = vsetq_lane_u32::<0>((t975.add(0 * t976) as *const u8).read_unaligned() as u32, t974[0]);
                t974[0] = vsetq_lane_u32::<1>((t975.add(1 * t976) as *const u8).read_unaligned() as u32, t974[0]);
                t974[0] = vsetq_lane_u32::<2>((t975.add(2 * t976) as *const u8).read_unaligned() as u32, t974[0]);
                t974[0] = vsetq_lane_u32::<3>((t975.add(3 * t976) as *const u8).read_unaligned() as u32, t974[0]);
                t974[1] = vsetq_lane_u32::<0>((t975.add(4 * t976) as *const u8).read_unaligned() as u32, t974[1]);
                t974[1] = vsetq_lane_u32::<1>((t975.add(5 * t976) as *const u8).read_unaligned() as u32, t974[1]);
                t974[1] = vsetq_lane_u32::<2>((t975.add(6 * t976) as *const u8).read_unaligned() as u32, t974[1]);
                t974[1] = vsetq_lane_u32::<3>((t975.add(7 * t976) as *const u8).read_unaligned() as u32, t974[1]);
            } else {
                let t977: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
                let t978 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t977[0]), vandq_u32(live[1], t977[1]))));
                let t979: [uint32x4_t; 2] = [vbslq_u32(live[0], t977[0], t978), vbslq_u32(live[1], t977[1], t978)];
                let mut t980: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                let t981 = vminvq_u32(vminq_u32(t979[0], t979[1]));
                if t981 == vmaxvq_u32(vmaxq_u32(t979[0], t979[1])) {
                    t980 = [vdupq_n_u32(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(t981 as usize * 1) as *const u8).read_unaligned() as u32)); 2];
                } else {
                    t980[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t979[0]) as usize * 1) as *const u8).read_unaligned() as u32), t980[0]);
                    t980[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t979[0]) as usize * 1) as *const u8).read_unaligned() as u32), t980[0]);
                    t980[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t979[0]) as usize * 1) as *const u8).read_unaligned() as u32), t980[0]);
                    t980[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t979[0]) as usize * 1) as *const u8).read_unaligned() as u32), t980[0]);
                    t980[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t979[1]) as usize * 1) as *const u8).read_unaligned() as u32), t980[1]);
                    t980[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t979[1]) as usize * 1) as *const u8).read_unaligned() as u32), t980[1]);
                    t980[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t979[1]) as usize * 1) as *const u8).read_unaligned() as u32), t980[1]);
                    t980[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t979[1]) as usize * 1) as *const u8).read_unaligned() as u32), t980[1]);
                }
                t974 = t980;
            }
            t970 = t974;
        }
        let t982: [uint32x4_t; 4] = [t970[0], t970[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t982;
        return res;
    }
    let t983: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
    let t984: [uint32x4_t; 2] = [t983[0], t983[1]];
    let t985: u64 = ((x1b) as usize as u64);
    let t986 = [vdupq_n_u32((t985) as u32); 2];
    let t987 = [vdupq_n_u32(((t985) >> 32) as u32); 2];
    let t988: [uint32x4_t; 2] = [vaddq_u32(t986[0], t984[0]), vaddq_u32(t986[1], t984[1])];
    let t989: [uint32x4_t; 2] = [vcltq_u32(t988[0], t986[0]), vcltq_u32(t988[1], t986[1])];
    let t990: [uint32x4_t; 4] = [t988[0], t988[1], vsubq_u32(vaddq_u32(t987[0], vdupq_n_u32(0)), t989[0]), vsubq_u32(vaddq_u32(t987[1], vdupq_n_u32(0)), t989[1])];
    let t991: u64 = !(7u64);
    let t992 = [vdupq_n_u32((t991) as u32); 2];
    let t993 = [vdupq_n_u32(((t991) >> 32) as u32); 2];
    let t994: [uint32x4_t; 4] = [vandq_u32(t990[0], t992[0]), vandq_u32(t990[1], t992[1]), vandq_u32(t990[2], t993[0]), vandq_u32(t990[3], t993[1])];
    let t995: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
    let t996: [uint32x4_t; 4] = [vandq_u32(t994[0], t995[0]), vandq_u32(t994[1], t995[1]), vandq_u32(t994[2], t995[2]), vandq_u32(t994[3], t995[3])];
    let t997: u64 = ((arg0) as usize as u64);
    let t998 = [vdupq_n_u32((t997) as u32); 2];
    let t999 = [vdupq_n_u32(((t997) >> 32) as u32); 2];
    let t1000: [uint32x4_t; 2] = [vaddq_u32(t998[0], t996[0]), vaddq_u32(t998[1], t996[1])];
    let t1001: [uint32x4_t; 2] = [vcltq_u32(t1000[0], t998[0]), vcltq_u32(t1000[1], t998[1])];
    let t1002: [uint32x4_t; 4] = [t1000[0], t1000[1], vsubq_u32(vaddq_u32(t999[0], t996[2]), t1001[0]), vsubq_u32(vaddq_u32(t999[1], t996[3]), t1001[1])];
    x3 = t1002;
    let t1003: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
    let t1004: [uint32x4_t; 2] = [t1003[0], t1003[1]];
    let t1005: u64 = ((x1b) as usize as u64);
    let t1006 = [vdupq_n_u32((t1005) as u32); 2];
    let t1007 = [vdupq_n_u32(((t1005) >> 32) as u32); 2];
    let t1008: [uint32x4_t; 2] = [vaddq_u32(t1006[0], t1004[0]), vaddq_u32(t1006[1], t1004[1])];
    let t1009: [uint32x4_t; 2] = [vcltq_u32(t1008[0], t1006[0]), vcltq_u32(t1008[1], t1006[1])];
    let t1010: [uint32x4_t; 4] = [t1008[0], t1008[1], vsubq_u32(vaddq_u32(t1007[0], vdupq_n_u32(0)), t1009[0]), vsubq_u32(vaddq_u32(t1007[1], vdupq_n_u32(0)), t1009[1])];
    let t1011: u64 = 7u64;
    let t1012 = [vdupq_n_u32((t1011) as u32); 2];
    let t1013: [uint32x4_t; 2] = [vandq_u32(t1010[0], t1012[0]), vandq_u32(t1010[1], t1012[1])];
    let t1014 = [vdupq_n_u32(8u32); 2];
    let t1015: [uint32x4_t; 2] = [vmulq_u32(t1013[0], t1014[0]), vmulq_u32(t1013[1], t1014[1])];
    x4 = t1015;
    let t1016 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([x3[0], x3[1]]);
    let t1017 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([x3[2], x3[3]]);
    let t1019 = bits(&live);
    let t1020 = t1019.trailing_zeros() as usize % 8;
    let t1018: [usize; 8] = [{ let j = if t1019 >> 0 & 1 != 0 { 0 } else { t1020 }; ((t1017[j] as u64) << 32 | t1016[j] as u64) as usize }, { let j = if t1019 >> 1 & 1 != 0 { 1 } else { t1020 }; ((t1017[j] as u64) << 32 | t1016[j] as u64) as usize }, { let j = if t1019 >> 2 & 1 != 0 { 2 } else { t1020 }; ((t1017[j] as u64) << 32 | t1016[j] as u64) as usize }, { let j = if t1019 >> 3 & 1 != 0 { 3 } else { t1020 }; ((t1017[j] as u64) << 32 | t1016[j] as u64) as usize }, { let j = if t1019 >> 4 & 1 != 0 { 4 } else { t1020 }; ((t1017[j] as u64) << 32 | t1016[j] as u64) as usize }, { let j = if t1019 >> 5 & 1 != 0 { 5 } else { t1020 }; ((t1017[j] as u64) << 32 | t1016[j] as u64) as usize }, { let j = if t1019 >> 6 & 1 != 0 { 6 } else { t1020 }; ((t1017[j] as u64) << 32 | t1016[j] as u64) as usize }, { let j = if t1019 >> 7 & 1 != 0 { 7 } else { t1020 }; ((t1017[j] as u64) << 32 | t1016[j] as u64) as usize }];
    let mut t1021: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (t1018[0] as *const u64).read_unaligned(); t1021[0] = vsetq_lane_u32::<0>(v as u32, t1021[0]); t1021[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1021[2]); }
    { let v = (t1018[1] as *const u64).read_unaligned(); t1021[0] = vsetq_lane_u32::<1>(v as u32, t1021[0]); t1021[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1021[2]); }
    { let v = (t1018[2] as *const u64).read_unaligned(); t1021[0] = vsetq_lane_u32::<2>(v as u32, t1021[0]); t1021[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1021[2]); }
    { let v = (t1018[3] as *const u64).read_unaligned(); t1021[0] = vsetq_lane_u32::<3>(v as u32, t1021[0]); t1021[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1021[2]); }
    { let v = (t1018[4] as *const u64).read_unaligned(); t1021[1] = vsetq_lane_u32::<0>(v as u32, t1021[1]); t1021[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1021[3]); }
    { let v = (t1018[5] as *const u64).read_unaligned(); t1021[1] = vsetq_lane_u32::<1>(v as u32, t1021[1]); t1021[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1021[3]); }
    { let v = (t1018[6] as *const u64).read_unaligned(); t1021[1] = vsetq_lane_u32::<2>(v as u32, t1021[1]); t1021[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1021[3]); }
    { let v = (t1018[7] as *const u64).read_unaligned(); t1021[1] = vsetq_lane_u32::<3>(v as u32, t1021[1]); t1021[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1021[3]); }
    let t1022: [uint32x4_t; 2] = [x4[0], x4[1]];
    let t1023: [uint32x4_t; 2] = [vandq_u32(t1022[0], vdupq_n_u32(63)), vandq_u32(t1022[1], vdupq_n_u32(63))];
    let t1024: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t1021[0], vnegq_s32(vreinterpretq_s32_u32(t1023[0]))), vshlq_u32(t1021[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1023[0])))), vorrq_u32(vshlq_u32(t1021[1], vnegq_s32(vreinterpretq_s32_u32(t1023[1]))), vshlq_u32(t1021[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1023[1])))), vshlq_u32(t1021[2], vnegq_s32(vreinterpretq_s32_u32(t1023[0]))), vshlq_u32(t1021[3], vnegq_s32(vreinterpretq_s32_u32(t1023[1])))];
    x5 = t1024;
    let t1025 = [vdupq_n_u32(0u32); 2];
    let t1026: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t1025[0])), vmvnq_u32(vceqq_u32(x4[1], t1025[1]))];
    let t1027 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
    let t1028: [uint32x4_t; 2] = [vaddq_u32(x4[0], t1027[0]), vaddq_u32(x4[1], t1027[1])];
    let t1029 = [vdupq_n_u32(64u32); 2];
    let t1030: [uint32x4_t; 2] = [vcgtq_u32(t1028[0], t1029[0]), vcgtq_u32(t1028[1], t1029[1])];
    let t1031: [uint32x4_t; 2] = [vandq_u32(t1026[0], t1030[0]), vandq_u32(t1026[1], t1030[1])];
    let mut t1032: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1033: [uint32x4_t; 2] = [vandq_u32(live[0], t1031[0]), vandq_u32(live[1], t1031[1])];
    let t1034: [uint32x4_t; 2] = [vbicq_u32(live[0], t1031[0]), vbicq_u32(live[1], t1031[1])];
    {
        let t1035: u64 = (1u64).wrapping_mul(8u64);
        let t1036 = [vdupq_n_u32((t1035) as u32); 2];
        let t1037 = [vdupq_n_u32(((t1035) >> 32) as u32); 2];
        let t1038: [uint32x4_t; 2] = [vaddq_u32(x3[0], t1036[0]), vaddq_u32(x3[1], t1036[1])];
        let t1039: [uint32x4_t; 2] = [vcltq_u32(t1038[0], x3[0]), vcltq_u32(t1038[1], x3[1])];
        let t1040: [uint32x4_t; 4] = [t1038[0], t1038[1], vsubq_u32(vaddq_u32(x3[2], t1037[0]), t1039[0]), vsubq_u32(vaddq_u32(x3[3], t1037[1]), t1039[1])];
        let t1041 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([t1040[0], t1040[1]]);
        let t1042 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([t1040[2], t1040[3]]);
        let t1044 = bits(&t1033);
        let t1045 = t1044.trailing_zeros() as usize % 8;
        let t1043: [usize; 8] = [{ let j = if t1044 >> 0 & 1 != 0 { 0 } else { t1045 }; ((t1042[j] as u64) << 32 | t1041[j] as u64) as usize }, { let j = if t1044 >> 1 & 1 != 0 { 1 } else { t1045 }; ((t1042[j] as u64) << 32 | t1041[j] as u64) as usize }, { let j = if t1044 >> 2 & 1 != 0 { 2 } else { t1045 }; ((t1042[j] as u64) << 32 | t1041[j] as u64) as usize }, { let j = if t1044 >> 3 & 1 != 0 { 3 } else { t1045 }; ((t1042[j] as u64) << 32 | t1041[j] as u64) as usize }, { let j = if t1044 >> 4 & 1 != 0 { 4 } else { t1045 }; ((t1042[j] as u64) << 32 | t1041[j] as u64) as usize }, { let j = if t1044 >> 5 & 1 != 0 { 5 } else { t1045 }; ((t1042[j] as u64) << 32 | t1041[j] as u64) as usize }, { let j = if t1044 >> 6 & 1 != 0 { 6 } else { t1045 }; ((t1042[j] as u64) << 32 | t1041[j] as u64) as usize }, { let j = if t1044 >> 7 & 1 != 0 { 7 } else { t1045 }; ((t1042[j] as u64) << 32 | t1041[j] as u64) as usize }];
        let mut t1046: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (t1043[0] as *const u64).read_unaligned(); t1046[0] = vsetq_lane_u32::<0>(v as u32, t1046[0]); t1046[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1046[2]); }
        { let v = (t1043[1] as *const u64).read_unaligned(); t1046[0] = vsetq_lane_u32::<1>(v as u32, t1046[0]); t1046[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1046[2]); }
        { let v = (t1043[2] as *const u64).read_unaligned(); t1046[0] = vsetq_lane_u32::<2>(v as u32, t1046[0]); t1046[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1046[2]); }
        { let v = (t1043[3] as *const u64).read_unaligned(); t1046[0] = vsetq_lane_u32::<3>(v as u32, t1046[0]); t1046[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1046[2]); }
        { let v = (t1043[4] as *const u64).read_unaligned(); t1046[1] = vsetq_lane_u32::<0>(v as u32, t1046[1]); t1046[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1046[3]); }
        { let v = (t1043[5] as *const u64).read_unaligned(); t1046[1] = vsetq_lane_u32::<1>(v as u32, t1046[1]); t1046[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1046[3]); }
        { let v = (t1043[6] as *const u64).read_unaligned(); t1046[1] = vsetq_lane_u32::<2>(v as u32, t1046[1]); t1046[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1046[3]); }
        { let v = (t1043[7] as *const u64).read_unaligned(); t1046[1] = vsetq_lane_u32::<3>(v as u32, t1046[1]); t1046[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1046[3]); }
        let t1047: [uint32x4_t; 2] = [vsubq_u32(t1029[0], x4[0]), vsubq_u32(t1029[1], x4[1])];
        let t1048: [uint32x4_t; 2] = [t1047[0], t1047[1]];
        let t1049: [uint32x4_t; 2] = [vandq_u32(t1048[0], vdupq_n_u32(63)), vandq_u32(t1048[1], vdupq_n_u32(63))];
        let t1050: [uint32x4_t; 4] = [vshlq_u32(t1046[0], vreinterpretq_s32_u32(t1049[0])), vshlq_u32(t1046[1], vreinterpretq_s32_u32(t1049[1])), vorrq_u32(vshlq_u32(t1046[2], vreinterpretq_s32_u32(t1049[0])), vshlq_u32(t1046[0], vsubq_s32(vreinterpretq_s32_u32(t1049[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1046[3], vreinterpretq_s32_u32(t1049[1])), vshlq_u32(t1046[1], vsubq_s32(vreinterpretq_s32_u32(t1049[1]), vdupq_n_s32(32))))];
        let t1051: [uint32x4_t; 4] = [vorrq_u32(x5[0], t1050[0]), vorrq_u32(x5[1], t1050[1]), vorrq_u32(x5[2], t1050[2]), vorrq_u32(x5[3], t1050[3])];
        t1032 = [vbslq_u32(t1033[0], t1051[0], t1032[0]), vbslq_u32(t1033[1], t1051[1], t1032[1]), vbslq_u32(t1033[0], t1051[2], t1032[2]), vbslq_u32(t1033[1], t1051[3], t1032[3])];
    }
    {
        t1032 = [vbslq_u32(t1034[0], x5[0], t1032[0]), vbslq_u32(t1034[1], x5[1], t1032[1]), vbslq_u32(t1034[0], x5[2], t1032[2]), vbslq_u32(t1034[1], x5[3], t1032[3])];
    }
    res = t1032;
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
        let t1052: [uint32x4_t; 4];
        if (x1is) == 8u32 && ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 {
            let t1053 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
            let t10540 = vld2q_u32((t1053 as *const u32).add(0));
            let t10541 = vld2q_u32((t1053 as *const u32).add(8));
            let t1054: [uint32x4_t; 4] = [t10540.0, t10541.0, t10540.1, t10541.1];
            t1052 = t1054;
        } else {
            let mut t1055: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            if ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 {
                let t1056 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
                let t1057 = ((x1is) as usize) * 1;
                { let v = (t1056.add(0 * t1057) as *const u64).read_unaligned(); t1055[0] = vsetq_lane_u32::<0>(v as u32, t1055[0]); t1055[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1055[2]); }
                { let v = (t1056.add(1 * t1057) as *const u64).read_unaligned(); t1055[0] = vsetq_lane_u32::<1>(v as u32, t1055[0]); t1055[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1055[2]); }
                { let v = (t1056.add(2 * t1057) as *const u64).read_unaligned(); t1055[0] = vsetq_lane_u32::<2>(v as u32, t1055[0]); t1055[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1055[2]); }
                { let v = (t1056.add(3 * t1057) as *const u64).read_unaligned(); t1055[0] = vsetq_lane_u32::<3>(v as u32, t1055[0]); t1055[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1055[2]); }
                { let v = (t1056.add(4 * t1057) as *const u64).read_unaligned(); t1055[1] = vsetq_lane_u32::<0>(v as u32, t1055[1]); t1055[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1055[3]); }
                { let v = (t1056.add(5 * t1057) as *const u64).read_unaligned(); t1055[1] = vsetq_lane_u32::<1>(v as u32, t1055[1]); t1055[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1055[3]); }
                { let v = (t1056.add(6 * t1057) as *const u64).read_unaligned(); t1055[1] = vsetq_lane_u32::<2>(v as u32, t1055[1]); t1055[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1055[3]); }
                { let v = (t1056.add(7 * t1057) as *const u64).read_unaligned(); t1055[1] = vsetq_lane_u32::<3>(v as u32, t1055[1]); t1055[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1055[3]); }
            } else {
                let t1058: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
                let mut t1059: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
                let t1060 = vminvq_u32(vminq_u32(t1058[0], t1058[1]));
                if t1060 == vmaxvq_u32(vmaxq_u32(t1058[0], t1058[1])) {
                    let t1061 = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(t1060 as usize * 1) as *const u64).read_unaligned();
                    t1059 = [vdupq_n_u32(t1061 as u32), vdupq_n_u32(t1061 as u32), vdupq_n_u32((t1061 >> 32) as u32), vdupq_n_u32((t1061 >> 32) as u32)];
                } else {
                    { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1058[0]) as usize * 1) as *const u64).read_unaligned(); t1059[0] = vsetq_lane_u32::<0>(v as u32, t1059[0]); t1059[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1059[2]); }
                    { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1058[0]) as usize * 1) as *const u64).read_unaligned(); t1059[0] = vsetq_lane_u32::<1>(v as u32, t1059[0]); t1059[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1059[2]); }
                    { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1058[0]) as usize * 1) as *const u64).read_unaligned(); t1059[0] = vsetq_lane_u32::<2>(v as u32, t1059[0]); t1059[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1059[2]); }
                    { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1058[0]) as usize * 1) as *const u64).read_unaligned(); t1059[0] = vsetq_lane_u32::<3>(v as u32, t1059[0]); t1059[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1059[2]); }
                    { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1058[1]) as usize * 1) as *const u64).read_unaligned(); t1059[1] = vsetq_lane_u32::<0>(v as u32, t1059[1]); t1059[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1059[3]); }
                    { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1058[1]) as usize * 1) as *const u64).read_unaligned(); t1059[1] = vsetq_lane_u32::<1>(v as u32, t1059[1]); t1059[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1059[3]); }
                    { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1058[1]) as usize * 1) as *const u64).read_unaligned(); t1059[1] = vsetq_lane_u32::<2>(v as u32, t1059[1]); t1059[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1059[3]); }
                    { let v = (((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1058[1]) as usize * 1) as *const u64).read_unaligned(); t1059[1] = vsetq_lane_u32::<3>(v as u32, t1059[1]); t1059[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1059[3]); }
                }
                t1055 = t1059;
            }
            t1052 = t1055;
        }
        res = t1052;
        return res;
    }
    if ((x2) == (4u32)) {
        let t1062: [uint32x4_t; 2];
        if (x1is) == 4u32 && ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 {
            let t1063 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
            let t1064: [uint32x4_t; 2] = core::array::from_fn(|r| vld1q_u32((t1063 as *const u32).add(4 * r)));
            t1062 = t1064;
        } else {
            let mut t1065: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            if ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 {
                let t1066 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
                let t1067 = ((x1is) as usize) * 1;
                t1065[0] = vld1q_lane_u32::<0>(t1066.add(0 * t1067) as *const u32, t1065[0]);
                t1065[0] = vld1q_lane_u32::<1>(t1066.add(1 * t1067) as *const u32, t1065[0]);
                t1065[0] = vld1q_lane_u32::<2>(t1066.add(2 * t1067) as *const u32, t1065[0]);
                t1065[0] = vld1q_lane_u32::<3>(t1066.add(3 * t1067) as *const u32, t1065[0]);
                t1065[1] = vld1q_lane_u32::<0>(t1066.add(4 * t1067) as *const u32, t1065[1]);
                t1065[1] = vld1q_lane_u32::<1>(t1066.add(5 * t1067) as *const u32, t1065[1]);
                t1065[1] = vld1q_lane_u32::<2>(t1066.add(6 * t1067) as *const u32, t1065[1]);
                t1065[1] = vld1q_lane_u32::<3>(t1066.add(7 * t1067) as *const u32, t1065[1]);
            } else {
                let t1068: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
                let mut t1069: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                let t1070 = vminvq_u32(vminq_u32(t1068[0], t1068[1]));
                if t1070 == vmaxvq_u32(vmaxq_u32(t1068[0], t1068[1])) {
                    t1069 = [vdupq_n_u32(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(t1070 as usize * 1) as *const u32).read_unaligned() as u32)); 2];
                } else {
                    t1069[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1068[0]) as usize * 1) as *const u32).read_unaligned() as u32), t1069[0]);
                    t1069[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1068[0]) as usize * 1) as *const u32).read_unaligned() as u32), t1069[0]);
                    t1069[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1068[0]) as usize * 1) as *const u32).read_unaligned() as u32), t1069[0]);
                    t1069[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1068[0]) as usize * 1) as *const u32).read_unaligned() as u32), t1069[0]);
                    t1069[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1068[1]) as usize * 1) as *const u32).read_unaligned() as u32), t1069[1]);
                    t1069[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1068[1]) as usize * 1) as *const u32).read_unaligned() as u32), t1069[1]);
                    t1069[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1068[1]) as usize * 1) as *const u32).read_unaligned() as u32), t1069[1]);
                    t1069[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1068[1]) as usize * 1) as *const u32).read_unaligned() as u32), t1069[1]);
                }
                t1065 = t1069;
            }
            t1062 = t1065;
        }
        let t1071: [uint32x4_t; 4] = [t1062[0], t1062[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1071;
        return res;
    }
    if ((x2) == (2u32)) {
        let t1072: [uint32x4_t; 2];
        if (x1is) == 2u32 && ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 {
            let t1073 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
            let t1074: [uint32x4_t; 2] = core::array::from_fn(|r| vmovl_u16(vld1_u16((t1073 as *const u16).add(4 * r))));
            t1072 = t1074;
        } else {
            let mut t1075: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            if ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 {
                let t1076 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
                let t1077 = ((x1is) as usize) * 1;
                t1075[0] = vsetq_lane_u32::<0>((t1076.add(0 * t1077) as *const u16).read_unaligned() as u32, t1075[0]);
                t1075[0] = vsetq_lane_u32::<1>((t1076.add(1 * t1077) as *const u16).read_unaligned() as u32, t1075[0]);
                t1075[0] = vsetq_lane_u32::<2>((t1076.add(2 * t1077) as *const u16).read_unaligned() as u32, t1075[0]);
                t1075[0] = vsetq_lane_u32::<3>((t1076.add(3 * t1077) as *const u16).read_unaligned() as u32, t1075[0]);
                t1075[1] = vsetq_lane_u32::<0>((t1076.add(4 * t1077) as *const u16).read_unaligned() as u32, t1075[1]);
                t1075[1] = vsetq_lane_u32::<1>((t1076.add(5 * t1077) as *const u16).read_unaligned() as u32, t1075[1]);
                t1075[1] = vsetq_lane_u32::<2>((t1076.add(6 * t1077) as *const u16).read_unaligned() as u32, t1075[1]);
                t1075[1] = vsetq_lane_u32::<3>((t1076.add(7 * t1077) as *const u16).read_unaligned() as u32, t1075[1]);
            } else {
                let t1078: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
                let mut t1079: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                let t1080 = vminvq_u32(vminq_u32(t1078[0], t1078[1]));
                if t1080 == vmaxvq_u32(vmaxq_u32(t1078[0], t1078[1])) {
                    t1079 = [vdupq_n_u32(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(t1080 as usize * 1) as *const u16).read_unaligned() as u32)); 2];
                } else {
                    t1079[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1078[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1079[0]);
                    t1079[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1078[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1079[0]);
                    t1079[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1078[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1079[0]);
                    t1079[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1078[0]) as usize * 1) as *const u16).read_unaligned() as u32), t1079[0]);
                    t1079[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1078[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1079[1]);
                    t1079[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1078[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1079[1]);
                    t1079[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1078[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1079[1]);
                    t1079[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1078[1]) as usize * 1) as *const u16).read_unaligned() as u32), t1079[1]);
                }
                t1075 = t1079;
            }
            t1072 = t1075;
        }
        let t1081: [uint32x4_t; 4] = [t1072[0], t1072[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1081;
        return res;
    }
    if ((x2) == (1u32)) {
        let t1082: [uint32x4_t; 2];
        if (x1is) == 1u32 && ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 {
            let t1083 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
            let t1085 = vmovl_u8(vld1_u8(t1083.add(0)));
            let t1084: [uint32x4_t; 2] = [vmovl_u16(vget_low_u16(t1085)), vmovl_u16(vget_high_u16(t1085))];
            t1082 = t1084;
        } else {
            let mut t1086: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            if ((x1if) as u64) + 7 * ((x1is) as u64) <= u32::MAX as u64 {
                let t1087 = ((arg0).wrapping_add((x1b) as usize) as *const u8).add(((x1if) as usize) * 1);
                let t1088 = ((x1is) as usize) * 1;
                t1086[0] = vsetq_lane_u32::<0>((t1087.add(0 * t1088) as *const u8).read_unaligned() as u32, t1086[0]);
                t1086[0] = vsetq_lane_u32::<1>((t1087.add(1 * t1088) as *const u8).read_unaligned() as u32, t1086[0]);
                t1086[0] = vsetq_lane_u32::<2>((t1087.add(2 * t1088) as *const u8).read_unaligned() as u32, t1086[0]);
                t1086[0] = vsetq_lane_u32::<3>((t1087.add(3 * t1088) as *const u8).read_unaligned() as u32, t1086[0]);
                t1086[1] = vsetq_lane_u32::<0>((t1087.add(4 * t1088) as *const u8).read_unaligned() as u32, t1086[1]);
                t1086[1] = vsetq_lane_u32::<1>((t1087.add(5 * t1088) as *const u8).read_unaligned() as u32, t1086[1]);
                t1086[1] = vsetq_lane_u32::<2>((t1087.add(6 * t1088) as *const u8).read_unaligned() as u32, t1086[1]);
                t1086[1] = vsetq_lane_u32::<3>((t1087.add(7 * t1088) as *const u8).read_unaligned() as u32, t1086[1]);
            } else {
                let t1089: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
                let mut t1090: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                let t1091 = vminvq_u32(vminq_u32(t1089[0], t1089[1]));
                if t1091 == vmaxvq_u32(vmaxq_u32(t1089[0], t1089[1])) {
                    t1090 = [vdupq_n_u32(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(t1091 as usize * 1) as *const u8).read_unaligned() as u32)); 2];
                } else {
                    t1090[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1089[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1090[0]);
                    t1090[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1089[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1090[0]);
                    t1090[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1089[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1090[0]);
                    t1090[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1089[0]) as usize * 1) as *const u8).read_unaligned() as u32), t1090[0]);
                    t1090[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1089[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1090[1]);
                    t1090[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1089[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1090[1]);
                    t1090[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1089[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1090[1]);
                    t1090[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((x1b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1089[1]) as usize * 1) as *const u8).read_unaligned() as u32), t1090[1]);
                }
                t1086 = t1090;
            }
            t1082 = t1086;
        }
        let t1092: [uint32x4_t; 4] = [t1082[0], t1082[1], vdupq_n_u32(0), vdupq_n_u32(0)];
        res = t1092;
        return res;
    }
    let t1093: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
    let t1094: [uint32x4_t; 2] = [t1093[0], t1093[1]];
    let t1095: u64 = ((x1b) as usize as u64);
    let t1096 = [vdupq_n_u32((t1095) as u32); 2];
    let t1097 = [vdupq_n_u32(((t1095) >> 32) as u32); 2];
    let t1098: [uint32x4_t; 2] = [vaddq_u32(t1096[0], t1094[0]), vaddq_u32(t1096[1], t1094[1])];
    let t1099: [uint32x4_t; 2] = [vcltq_u32(t1098[0], t1096[0]), vcltq_u32(t1098[1], t1096[1])];
    let t1100: [uint32x4_t; 4] = [t1098[0], t1098[1], vsubq_u32(vaddq_u32(t1097[0], vdupq_n_u32(0)), t1099[0]), vsubq_u32(vaddq_u32(t1097[1], vdupq_n_u32(0)), t1099[1])];
    let t1101: u64 = !(7u64);
    let t1102 = [vdupq_n_u32((t1101) as u32); 2];
    let t1103 = [vdupq_n_u32(((t1101) >> 32) as u32); 2];
    let t1104: [uint32x4_t; 4] = [vandq_u32(t1100[0], t1102[0]), vandq_u32(t1100[1], t1102[1]), vandq_u32(t1100[2], t1103[0]), vandq_u32(t1100[3], t1103[1])];
    let t1105: u64 = ((arg0) as usize as u64);
    let t1106 = [vdupq_n_u32((t1105) as u32); 2];
    let t1107 = [vdupq_n_u32(((t1105) >> 32) as u32); 2];
    let t1108: [uint32x4_t; 2] = [vaddq_u32(t1106[0], t1104[0]), vaddq_u32(t1106[1], t1104[1])];
    let t1109: [uint32x4_t; 2] = [vcltq_u32(t1108[0], t1106[0]), vcltq_u32(t1108[1], t1106[1])];
    let t1110: [uint32x4_t; 4] = [t1108[0], t1108[1], vsubq_u32(vaddq_u32(t1107[0], t1104[2]), t1109[0]), vsubq_u32(vaddq_u32(t1107[1], t1104[3]), t1109[1])];
    x3 = t1110;
    let t1111: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x1is)), vmlaq_u32(vdupq_n_u32(x1if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x1is))];
    let t1112: [uint32x4_t; 2] = [t1111[0], t1111[1]];
    let t1113: u64 = ((x1b) as usize as u64);
    let t1114 = [vdupq_n_u32((t1113) as u32); 2];
    let t1115 = [vdupq_n_u32(((t1113) >> 32) as u32); 2];
    let t1116: [uint32x4_t; 2] = [vaddq_u32(t1114[0], t1112[0]), vaddq_u32(t1114[1], t1112[1])];
    let t1117: [uint32x4_t; 2] = [vcltq_u32(t1116[0], t1114[0]), vcltq_u32(t1116[1], t1114[1])];
    let t1118: [uint32x4_t; 4] = [t1116[0], t1116[1], vsubq_u32(vaddq_u32(t1115[0], vdupq_n_u32(0)), t1117[0]), vsubq_u32(vaddq_u32(t1115[1], vdupq_n_u32(0)), t1117[1])];
    let t1119: u64 = 7u64;
    let t1120 = [vdupq_n_u32((t1119) as u32); 2];
    let t1121: [uint32x4_t; 2] = [vandq_u32(t1118[0], t1120[0]), vandq_u32(t1118[1], t1120[1])];
    let t1122 = [vdupq_n_u32(8u32); 2];
    let t1123: [uint32x4_t; 2] = [vmulq_u32(t1121[0], t1122[0]), vmulq_u32(t1121[1], t1122[1])];
    x4 = t1123;
    let t1124 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([x3[0], x3[1]]);
    let t1125 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([x3[2], x3[3]]);
    let t1126: [usize; 8] = [((t1125[0] as u64) << 32 | t1124[0] as u64) as usize, ((t1125[1] as u64) << 32 | t1124[1] as u64) as usize, ((t1125[2] as u64) << 32 | t1124[2] as u64) as usize, ((t1125[3] as u64) << 32 | t1124[3] as u64) as usize, ((t1125[4] as u64) << 32 | t1124[4] as u64) as usize, ((t1125[5] as u64) << 32 | t1124[5] as u64) as usize, ((t1125[6] as u64) << 32 | t1124[6] as u64) as usize, ((t1125[7] as u64) << 32 | t1124[7] as u64) as usize];
    let mut t1127: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (t1126[0] as *const u64).read_unaligned(); t1127[0] = vsetq_lane_u32::<0>(v as u32, t1127[0]); t1127[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1127[2]); }
    { let v = (t1126[1] as *const u64).read_unaligned(); t1127[0] = vsetq_lane_u32::<1>(v as u32, t1127[0]); t1127[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1127[2]); }
    { let v = (t1126[2] as *const u64).read_unaligned(); t1127[0] = vsetq_lane_u32::<2>(v as u32, t1127[0]); t1127[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1127[2]); }
    { let v = (t1126[3] as *const u64).read_unaligned(); t1127[0] = vsetq_lane_u32::<3>(v as u32, t1127[0]); t1127[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1127[2]); }
    { let v = (t1126[4] as *const u64).read_unaligned(); t1127[1] = vsetq_lane_u32::<0>(v as u32, t1127[1]); t1127[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1127[3]); }
    { let v = (t1126[5] as *const u64).read_unaligned(); t1127[1] = vsetq_lane_u32::<1>(v as u32, t1127[1]); t1127[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1127[3]); }
    { let v = (t1126[6] as *const u64).read_unaligned(); t1127[1] = vsetq_lane_u32::<2>(v as u32, t1127[1]); t1127[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1127[3]); }
    { let v = (t1126[7] as *const u64).read_unaligned(); t1127[1] = vsetq_lane_u32::<3>(v as u32, t1127[1]); t1127[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1127[3]); }
    let t1128: [uint32x4_t; 2] = [x4[0], x4[1]];
    let t1129: [uint32x4_t; 2] = [vandq_u32(t1128[0], vdupq_n_u32(63)), vandq_u32(t1128[1], vdupq_n_u32(63))];
    let t1130: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t1127[0], vnegq_s32(vreinterpretq_s32_u32(t1129[0]))), vshlq_u32(t1127[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1129[0])))), vorrq_u32(vshlq_u32(t1127[1], vnegq_s32(vreinterpretq_s32_u32(t1129[1]))), vshlq_u32(t1127[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t1129[1])))), vshlq_u32(t1127[2], vnegq_s32(vreinterpretq_s32_u32(t1129[0]))), vshlq_u32(t1127[3], vnegq_s32(vreinterpretq_s32_u32(t1129[1])))];
    x5 = t1130;
    let t1131 = [vdupq_n_u32(0u32); 2];
    let t1132: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t1131[0])), vmvnq_u32(vceqq_u32(x4[1], t1131[1]))];
    let t1133 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
    let t1134: [uint32x4_t; 2] = [vaddq_u32(x4[0], t1133[0]), vaddq_u32(x4[1], t1133[1])];
    let t1135 = [vdupq_n_u32(64u32); 2];
    let t1136: [uint32x4_t; 2] = [vcgtq_u32(t1134[0], t1135[0]), vcgtq_u32(t1134[1], t1135[1])];
    let t1137: [uint32x4_t; 2] = [vandq_u32(t1132[0], t1136[0]), vandq_u32(t1132[1], t1136[1])];
    let mut t1138: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1139: [uint32x4_t; 2] = [vmvnq_u32(t1137[0]), vmvnq_u32(t1137[1])];
    {
        let t1140: u64 = (1u64).wrapping_mul(8u64);
        let t1141 = [vdupq_n_u32((t1140) as u32); 2];
        let t1142 = [vdupq_n_u32(((t1140) >> 32) as u32); 2];
        let t1143: [uint32x4_t; 2] = [vaddq_u32(x3[0], t1141[0]), vaddq_u32(x3[1], t1141[1])];
        let t1144: [uint32x4_t; 2] = [vcltq_u32(t1143[0], x3[0]), vcltq_u32(t1143[1], x3[1])];
        let t1145: [uint32x4_t; 4] = [t1143[0], t1143[1], vsubq_u32(vaddq_u32(x3[2], t1142[0]), t1144[0]), vsubq_u32(vaddq_u32(x3[3], t1142[1]), t1144[1])];
        let t1146 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([t1145[0], t1145[1]]);
        let t1147 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>([t1145[2], t1145[3]]);
        let t1149 = bits(&t1137);
        let t1150 = t1149.trailing_zeros() as usize % 8;
        let t1148: [usize; 8] = [{ let j = if t1149 >> 0 & 1 != 0 { 0 } else { t1150 }; ((t1147[j] as u64) << 32 | t1146[j] as u64) as usize }, { let j = if t1149 >> 1 & 1 != 0 { 1 } else { t1150 }; ((t1147[j] as u64) << 32 | t1146[j] as u64) as usize }, { let j = if t1149 >> 2 & 1 != 0 { 2 } else { t1150 }; ((t1147[j] as u64) << 32 | t1146[j] as u64) as usize }, { let j = if t1149 >> 3 & 1 != 0 { 3 } else { t1150 }; ((t1147[j] as u64) << 32 | t1146[j] as u64) as usize }, { let j = if t1149 >> 4 & 1 != 0 { 4 } else { t1150 }; ((t1147[j] as u64) << 32 | t1146[j] as u64) as usize }, { let j = if t1149 >> 5 & 1 != 0 { 5 } else { t1150 }; ((t1147[j] as u64) << 32 | t1146[j] as u64) as usize }, { let j = if t1149 >> 6 & 1 != 0 { 6 } else { t1150 }; ((t1147[j] as u64) << 32 | t1146[j] as u64) as usize }, { let j = if t1149 >> 7 & 1 != 0 { 7 } else { t1150 }; ((t1147[j] as u64) << 32 | t1146[j] as u64) as usize }];
        let mut t1151: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (t1148[0] as *const u64).read_unaligned(); t1151[0] = vsetq_lane_u32::<0>(v as u32, t1151[0]); t1151[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t1151[2]); }
        { let v = (t1148[1] as *const u64).read_unaligned(); t1151[0] = vsetq_lane_u32::<1>(v as u32, t1151[0]); t1151[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t1151[2]); }
        { let v = (t1148[2] as *const u64).read_unaligned(); t1151[0] = vsetq_lane_u32::<2>(v as u32, t1151[0]); t1151[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t1151[2]); }
        { let v = (t1148[3] as *const u64).read_unaligned(); t1151[0] = vsetq_lane_u32::<3>(v as u32, t1151[0]); t1151[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t1151[2]); }
        { let v = (t1148[4] as *const u64).read_unaligned(); t1151[1] = vsetq_lane_u32::<0>(v as u32, t1151[1]); t1151[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t1151[3]); }
        { let v = (t1148[5] as *const u64).read_unaligned(); t1151[1] = vsetq_lane_u32::<1>(v as u32, t1151[1]); t1151[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t1151[3]); }
        { let v = (t1148[6] as *const u64).read_unaligned(); t1151[1] = vsetq_lane_u32::<2>(v as u32, t1151[1]); t1151[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t1151[3]); }
        { let v = (t1148[7] as *const u64).read_unaligned(); t1151[1] = vsetq_lane_u32::<3>(v as u32, t1151[1]); t1151[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t1151[3]); }
        let t1152: [uint32x4_t; 2] = [vsubq_u32(t1135[0], x4[0]), vsubq_u32(t1135[1], x4[1])];
        let t1153: [uint32x4_t; 2] = [t1152[0], t1152[1]];
        let t1154: [uint32x4_t; 2] = [vandq_u32(t1153[0], vdupq_n_u32(63)), vandq_u32(t1153[1], vdupq_n_u32(63))];
        let t1155: [uint32x4_t; 4] = [vshlq_u32(t1151[0], vreinterpretq_s32_u32(t1154[0])), vshlq_u32(t1151[1], vreinterpretq_s32_u32(t1154[1])), vorrq_u32(vshlq_u32(t1151[2], vreinterpretq_s32_u32(t1154[0])), vshlq_u32(t1151[0], vsubq_s32(vreinterpretq_s32_u32(t1154[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t1151[3], vreinterpretq_s32_u32(t1154[1])), vshlq_u32(t1151[1], vsubq_s32(vreinterpretq_s32_u32(t1154[1]), vdupq_n_s32(32))))];
        let t1156: [uint32x4_t; 4] = [vorrq_u32(x5[0], t1155[0]), vorrq_u32(x5[1], t1155[1]), vorrq_u32(x5[2], t1155[2]), vorrq_u32(x5[3], t1155[3])];
        t1138 = [vbslq_u32(t1137[0], t1156[0], t1138[0]), vbslq_u32(t1137[1], t1156[1], t1138[1]), vbslq_u32(t1137[0], t1156[2], t1138[2]), vbslq_u32(t1137[1], t1156[3], t1138[3])];
    }
    {
        t1138 = [vbslq_u32(t1139[0], x5[0], t1138[0]), vbslq_u32(t1139[1], x5[1], t1138[1]), vbslq_u32(t1139[0], x5[2], t1138[2]), vbslq_u32(t1139[1], x5[3], t1138[3])];
    }
    res = t1138;
    return res;
}

#[inline(always)]
unsafe fn b18_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t1157: u64;
    if ((x0) >= (64u32)) {
        t1157 = !(0u64);
    } else {
        t1157 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t1157;
    return res;
}

#[inline(always)]
unsafe fn b18_s_f(l: &Launch, lid0: u32, x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t1158: u64;
    if ((x0) >= (64u32)) {
        t1158 = !(0u64);
    } else {
        t1158 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t1158;
    return res;
}

#[inline(always)]
unsafe fn b19_r0sss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1: u32, x2: u32, x3: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u64 = 0;
    let mut live = k;
    let mut res: u64 = 0;
    let t1159 = b17_r0ss_p(l, lid0, live, arg0, (((x1) as u64)).wrapping_add(((x2) as u64)), x3);
    x4 = t1159;
    let t1160: u64;
    if ((x3) >= (8u32)) {
        t1160 = x4;
    } else {
        let t1161 = b18_s_p(l, lid0, live, (8u32).wrapping_mul(x3));
        t1160 = ((x4) & (t1161));
    }
    res = t1160;
    return res;
}

#[inline(always)]
unsafe fn b19_r0sss_f(l: &Launch, lid0: u32, arg0: *const u8, x1: u32, x2: u32, x3: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: u64 = 0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t1162 = b17_r0ss_f(l, lid0, arg0, (((x1) as u64)).wrapping_add(((x2) as u64)), x3);
    x4 = t1162;
    let t1163: u64;
    if ((x3) >= (8u32)) {
        t1163 = x4;
    } else {
        let t1164 = b18_s_f(l, lid0, (8u32).wrapping_mul(x3));
        t1163 = ((x4) & (t1164));
    }
    res = t1163;
    return res;
}

#[inline(always)]
unsafe fn b19_r0sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x1: u32, x2: [uint32x4_t; 2], x3: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t1165 = b17_r0al1s_p(l, lid0, live, arg0, ((((x1) as u64)) as usize as *const u8), x2, x3);
    x4 = t1165;
    let t1166: [uint32x4_t; 4];
    if ((x3) >= (8u32)) {
        t1166 = x4;
    } else {
        let t1167 = b18_s_p(l, lid0, live, (8u32).wrapping_mul(x3));
        let t1168: u64 = t1167;
        let t1169 = [vdupq_n_u32((t1168) as u32); 2];
        let t1170 = [vdupq_n_u32(((t1168) >> 32) as u32); 2];
        let t1171: [uint32x4_t; 4] = [vandq_u32(x4[0], t1169[0]), vandq_u32(x4[1], t1169[1]), vandq_u32(x4[2], t1170[0]), vandq_u32(x4[3], t1170[1])];
        t1166 = t1171;
    }
    res = t1166;
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
    let mut t1172: bool = !(arg4).is_null();
    if t1172 {
        let mut t1173: bool = ((l.group[0]) >= ((arg4.wrapping_add((((x5) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        if !t1173 {
            t1173 = ((l.group[1]) >= ((arg4.wrapping_add(((((x5).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        let mut t1174: bool = t1173;
        if !t1174 {
            t1174 = ((l.group[2]) >= ((arg4.wrapping_add(((((x5).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        t1172 = t1174;
    }
    if t1172 {
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
        let t1175: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
        let mut live = t1175;
        let gid0 = l.group[0] * l.block[0] + lid0;
        'chunk: {
            b0_r0r1r2sg_p(l, lid0, live, arg0, arg1, arg2, x3);
        }
    }
}
