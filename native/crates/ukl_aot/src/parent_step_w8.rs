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
unsafe fn b0_r0r1r2sg_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, arg2: *const u8, x24: *const u8) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x24 as *const u32).read_unaligned();
    let mut x48: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
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
        x48 = t5;
        let t6: u64 = (1u64).wrapping_shl((32u64) as u32);
        let t7 = [vdupq_n_u32((t6) as u32); 2];
        let t8 = [vdupq_n_u32(((t6) >> 32) as u32); 2];
        let t9: [uint32x4_t; 4] = [vandq_u32(x48[0], t7[0]), vandq_u32(x48[1], t7[1]), vandq_u32(x48[2], t8[0]), vandq_u32(x48[3], t8[1])];
        let t10: u64 = 0u64;
        let t11 = [vdupq_n_u32((t10) as u32); 2];
        let t12: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t9[0], t11[0]), vceqq_u32(t9[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t9[1], t11[1]), vceqq_u32(t9[3], vdupq_n_u32(0)))];
        let t13: [uint32x4_t; 2] = [vmvnq_u32(t12[0]), vmvnq_u32(t12[1])];
        let mut t14: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t15: [uint32x4_t; 2] = [vandq_u32(live[0], t13[0]), vandq_u32(live[1], t13[1])];
        let t16: [uint32x4_t; 2] = [vbicq_u32(live[0], t13[0]), vbicq_u32(live[1], t13[1])];
        {
            let t17: [uint32x4_t; 2] = [x48[0], x48[1]];
            let t18 = [vdupq_n_u32(2147483648u32); 2];
            let t19: [uint32x4_t; 2] = [vorrq_u32(t18[0], t17[0]), vorrq_u32(t18[1], t17[1])];
            t14 = [vbslq_u32(t15[0], t19[0], t14[0]), vbslq_u32(t15[1], t19[1], t14[1])];
        }
        {
            let t20: [uint32x4_t; 2] = [x48[0], x48[1]];
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
unsafe fn b0_r0r1r2sg_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, arg2: *const u8, x24: *const u8) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x24 as *const u32).read_unaligned();
    let mut x48: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    if ((gid0) as u64 + 7) < ((a0) as u64) {
        let t23 = b1_r0r1g_f(l, lid0, arg0, arg1);
        x48 = t23;
        let t24: u64 = (1u64).wrapping_shl((32u64) as u32);
        let t25 = [vdupq_n_u32((t24) as u32); 2];
        let t26 = [vdupq_n_u32(((t24) >> 32) as u32); 2];
        let t27: [uint32x4_t; 4] = [vandq_u32(x48[0], t25[0]), vandq_u32(x48[1], t25[1]), vandq_u32(x48[2], t26[0]), vandq_u32(x48[3], t26[1])];
        let t28: u64 = 0u64;
        let t29 = [vdupq_n_u32((t28) as u32); 2];
        let t30: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t27[0], t29[0]), vceqq_u32(t27[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t27[1], t29[1]), vceqq_u32(t27[3], vdupq_n_u32(0)))];
        let t31: [uint32x4_t; 2] = [vmvnq_u32(t30[0]), vmvnq_u32(t30[1])];
        let mut t32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t33: [uint32x4_t; 2] = [vmvnq_u32(t31[0]), vmvnq_u32(t31[1])];
        {
            let t34: [uint32x4_t; 2] = [x48[0], x48[1]];
            let t35 = [vdupq_n_u32(2147483648u32); 2];
            let t36: [uint32x4_t; 2] = [vorrq_u32(t35[0], t34[0]), vorrq_u32(t35[1], t34[1])];
            t32 = [vbslq_u32(t31[0], t36[0], t32[0]), vbslq_u32(t31[1], t36[1], t32[1])];
        }
        {
            let t37: [uint32x4_t; 2] = [x48[0], x48[1]];
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
            x48 = t43;
            let t44: u64 = (1u64).wrapping_shl((32u64) as u32);
            let t45 = [vdupq_n_u32((t44) as u32); 2];
            let t46 = [vdupq_n_u32(((t44) >> 32) as u32); 2];
            let t47: [uint32x4_t; 4] = [vandq_u32(x48[0], t45[0]), vandq_u32(x48[1], t45[1]), vandq_u32(x48[2], t46[0]), vandq_u32(x48[3], t46[1])];
            let t48: u64 = 0u64;
            let t49 = [vdupq_n_u32((t48) as u32); 2];
            let t50: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(t47[0], t49[0]), vceqq_u32(t47[2], vdupq_n_u32(0))), vandq_u32(vceqq_u32(t47[1], t49[1]), vceqq_u32(t47[3], vdupq_n_u32(0)))];
            let t51: [uint32x4_t; 2] = [vmvnq_u32(t50[0]), vmvnq_u32(t50[1])];
            let mut t52: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t53: [uint32x4_t; 2] = [vandq_u32(live[0], t51[0]), vandq_u32(live[1], t51[1])];
            let t54: [uint32x4_t; 2] = [vbicq_u32(live[0], t51[0]), vbicq_u32(live[1], t51[1])];
            {
                let t55: [uint32x4_t; 2] = [x48[0], x48[1]];
                let t56 = [vdupq_n_u32(2147483648u32); 2];
                let t57: [uint32x4_t; 2] = [vorrq_u32(t56[0], t55[0]), vorrq_u32(t56[1], t55[1])];
                t52 = [vbslq_u32(t53[0], t57[0], t52[0]), vbslq_u32(t53[1], t57[1], t52[1])];
            }
            {
                let t58: [uint32x4_t; 2] = [x48[0], x48[1]];
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
    let mut x24: u32 = 0;
    let mut x32: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
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
        x24 = gid0.wrapping_sub(1u32);
        let t68 = if (x24 & 255u32) <= 248u32 { b2_r0r1u8_p(l, lid0, live, arg0, arg1, x24) } else { b2_r0r1i_p(l, lid0, live, arg0, arg1, x24) };
        x32 = t68;
        let t69: u64 = 1u64;
        let t70 = [vdupq_n_u32((t69) as u32); 2];
        let t71: [uint32x4_t; 2] = [vandq_u32(x32[0], t70[0]), vandq_u32(x32[1], t70[1])];
        let t72: u64 = 0u64;
        let t73 = [vdupq_n_u32((t72) as u32); 2];
        let t74: [uint32x4_t; 2] = [vceqq_u32(t71[0], t73[0]), vceqq_u32(t71[1], t73[1])];
        let t75: [uint32x4_t; 2] = [vandq_u32(live[0], t74[0]), vandq_u32(live[1], t74[1])];
        {
            let t76: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[1].as_ptr()))];
            let t77: [uint32x4_t; 4] = [t76[0], t76[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t75[0], t77[0], res[0]), vbslq_u32(t75[1], t77[1], res[1]), vbslq_u32(t75[0], t77[2], res[2]), vbslq_u32(t75[1], t77[3], res[3])];
            live = [vbicq_u32(live[0], t75[0]), vbicq_u32(live[1], t75[1])];
        }
        if any(&live) {
            let t78: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x32[0]), vshlq_n_u32::<30>(x32[2])), vorrq_u32(vshrq_n_u32::<2>(x32[1]), vshlq_n_u32::<30>(x32[3])), vshrq_n_u32::<2>(x32[2]), vshrq_n_u32::<2>(x32[3])];
            let t79: [uint32x4_t; 2] = [t78[0], t78[1]];
            x40 = t79;
            let t80 = [vdupq_n_u32(1u32); 2];
            let t81: [uint32x4_t; 2] = [vsubq_u32(x40[0], t80[0]), vsubq_u32(x40[1], t80[1])];
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
                let t87: [uint32x4_t; 2] = [vsubq_u32(t61[0], x40[0]), vsubq_u32(t61[1], x40[1])];
                x48 = t87;
                let t88: u64 = 2u64;
                let t89 = [vdupq_n_u32((t88) as u32); 2];
                let t90: [uint32x4_t; 2] = [vandq_u32(x32[0], t89[0]), vandq_u32(x32[1], t89[1])];
                let t91: u64 = 0u64;
                let t92 = [vdupq_n_u32((t91) as u32); 2];
                let t93: [uint32x4_t; 2] = [vceqq_u32(t90[0], t92[0]), vceqq_u32(t90[1], t92[1])];
                let t94: [uint32x4_t; 2] = [vmvnq_u32(t93[0]), vmvnq_u32(t93[1])];
                let t95: [uint32x4_t; 2] = [vandq_u32(live[0], t94[0]), vandq_u32(live[1], t94[1])];
                {
                    let t96: [uint32x4_t; 4] = [x48[0], x48[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                    res = [vbslq_u32(t95[0], t96[0], res[0]), vbslq_u32(t95[1], t96[1], res[1]), vbslq_u32(t95[0], t96[2], res[2]), vbslq_u32(t95[1], t96[3], res[3])];
                    live = [vbicq_u32(live[0], t95[0]), vbicq_u32(live[1], t95[1])];
                }
                if any(&live) {
                    let t97: [uint32x4_t; 2] = [vceqq_u32(x48[0], t62[0]), vceqq_u32(x48[1], t62[1])];
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
                        let t105: [uint32x4_t; 4] = [vorrq_u32(t103[0], x48[0]), vorrq_u32(t103[1], x48[1]), t104[0], t104[1]];
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
    let mut x24: u32 = 0;
    let mut x32: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if (((gid0) as u64 + 7) <= u32::MAX as u64 && (((0u32) as u64) < ((gid0) as u64) || ((0u32) as u64) > ((gid0) as u64 + 7))) {
        x24 = gid0.wrapping_sub(1u32);
        let t106 = if (x24 & 255u32) <= 248u32 { b2_r0r1u8_f(l, lid0, arg0, arg1, x24) } else { b2_r0r1i_f(l, lid0, arg0, arg1, x24) };
        x32 = t106;
        let t107: u64 = 1u64;
        let t108 = [vdupq_n_u32((t107) as u32); 2];
        let t109: [uint32x4_t; 2] = [vandq_u32(x32[0], t108[0]), vandq_u32(x32[1], t108[1])];
        let t110: u64 = 0u64;
        let t111 = [vdupq_n_u32((t110) as u32); 2];
        let t112: [uint32x4_t; 2] = [vceqq_u32(t109[0], t111[0]), vceqq_u32(t109[1], t111[1])];
        {
            let t113: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[1].as_ptr()))];
            let t114: [uint32x4_t; 4] = [t113[0], t113[1], vdupq_n_u32(0), vdupq_n_u32(0)];
            res = [vbslq_u32(t112[0], t114[0], res[0]), vbslq_u32(t112[1], t114[1], res[1]), vbslq_u32(t112[0], t114[2], res[2]), vbslq_u32(t112[1], t114[3], res[3])];
            live = [vbicq_u32(live[0], t112[0]), vbicq_u32(live[1], t112[1])];
        }
        if any(&live) {
            let t115: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x32[0]), vshlq_n_u32::<30>(x32[2])), vorrq_u32(vshrq_n_u32::<2>(x32[1]), vshlq_n_u32::<30>(x32[3])), vshrq_n_u32::<2>(x32[2]), vshrq_n_u32::<2>(x32[3])];
            let t116: [uint32x4_t; 2] = [t115[0], t115[1]];
            x40 = t116;
            let t117 = [vdupq_n_u32(1u32); 2];
            let t118: [uint32x4_t; 2] = [vsubq_u32(x40[0], t117[0]), vsubq_u32(x40[1], t117[1])];
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
                let t125: [uint32x4_t; 2] = [vsubq_u32(t119[0], x40[0]), vsubq_u32(t119[1], x40[1])];
                x48 = t125;
                let t126: u64 = 2u64;
                let t127 = [vdupq_n_u32((t126) as u32); 2];
                let t128: [uint32x4_t; 2] = [vandq_u32(x32[0], t127[0]), vandq_u32(x32[1], t127[1])];
                let t129: u64 = 0u64;
                let t130 = [vdupq_n_u32((t129) as u32); 2];
                let t131: [uint32x4_t; 2] = [vceqq_u32(t128[0], t130[0]), vceqq_u32(t128[1], t130[1])];
                let t132: [uint32x4_t; 2] = [vmvnq_u32(t131[0]), vmvnq_u32(t131[1])];
                let t133: [uint32x4_t; 2] = [vandq_u32(live[0], t132[0]), vandq_u32(live[1], t132[1])];
                {
                    let t134: [uint32x4_t; 4] = [x48[0], x48[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                    res = [vbslq_u32(t133[0], t134[0], res[0]), vbslq_u32(t133[1], t134[1], res[1]), vbslq_u32(t133[0], t134[2], res[2]), vbslq_u32(t133[1], t134[3], res[3])];
                    live = [vbicq_u32(live[0], t133[0]), vbicq_u32(live[1], t133[1])];
                }
                if any(&live) {
                    let t135 = [vdupq_n_u32(0u32); 2];
                    let t136: [uint32x4_t; 2] = [vceqq_u32(x48[0], t135[0]), vceqq_u32(x48[1], t135[1])];
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
                        let t144: [uint32x4_t; 4] = [vorrq_u32(t142[0], x48[0]), vorrq_u32(t142[1], x48[1]), t143[0], t143[1]];
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
            x24 = gid0.wrapping_sub(1u32);
            let t155 = if (x24 & 255u32) <= 248u32 { b2_r0r1u8_p(l, lid0, live, arg0, arg1, x24) } else { b2_r0r1i_p(l, lid0, live, arg0, arg1, x24) };
            x32 = t155;
            let t156: u64 = 1u64;
            let t157 = [vdupq_n_u32((t156) as u32); 2];
            let t158: [uint32x4_t; 2] = [vandq_u32(x32[0], t157[0]), vandq_u32(x32[1], t157[1])];
            let t159: u64 = 0u64;
            let t160 = [vdupq_n_u32((t159) as u32); 2];
            let t161: [uint32x4_t; 2] = [vceqq_u32(t158[0], t160[0]), vceqq_u32(t158[1], t160[1])];
            let t162: [uint32x4_t; 2] = [vandq_u32(live[0], t161[0]), vandq_u32(live[1], t161[1])];
            {
                let t163: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[1].as_ptr()))];
                let t164: [uint32x4_t; 4] = [t163[0], t163[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                res = [vbslq_u32(t162[0], t164[0], res[0]), vbslq_u32(t162[1], t164[1], res[1]), vbslq_u32(t162[0], t164[2], res[2]), vbslq_u32(t162[1], t164[3], res[3])];
                live = [vbicq_u32(live[0], t162[0]), vbicq_u32(live[1], t162[1])];
            }
            if any(&live) {
                let t165: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x32[0]), vshlq_n_u32::<30>(x32[2])), vorrq_u32(vshrq_n_u32::<2>(x32[1]), vshlq_n_u32::<30>(x32[3])), vshrq_n_u32::<2>(x32[2]), vshrq_n_u32::<2>(x32[3])];
                let t166: [uint32x4_t; 2] = [t165[0], t165[1]];
                x40 = t166;
                let t167 = [vdupq_n_u32(1u32); 2];
                let t168: [uint32x4_t; 2] = [vsubq_u32(x40[0], t167[0]), vsubq_u32(x40[1], t167[1])];
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
                    let t174: [uint32x4_t; 2] = [vsubq_u32(t148[0], x40[0]), vsubq_u32(t148[1], x40[1])];
                    x48 = t174;
                    let t175: u64 = 2u64;
                    let t176 = [vdupq_n_u32((t175) as u32); 2];
                    let t177: [uint32x4_t; 2] = [vandq_u32(x32[0], t176[0]), vandq_u32(x32[1], t176[1])];
                    let t178: u64 = 0u64;
                    let t179 = [vdupq_n_u32((t178) as u32); 2];
                    let t180: [uint32x4_t; 2] = [vceqq_u32(t177[0], t179[0]), vceqq_u32(t177[1], t179[1])];
                    let t181: [uint32x4_t; 2] = [vmvnq_u32(t180[0]), vmvnq_u32(t180[1])];
                    let t182: [uint32x4_t; 2] = [vandq_u32(live[0], t181[0]), vandq_u32(live[1], t181[1])];
                    {
                        let t183: [uint32x4_t; 4] = [x48[0], x48[1], vdupq_n_u32(0), vdupq_n_u32(0)];
                        res = [vbslq_u32(t182[0], t183[0], res[0]), vbslq_u32(t182[1], t183[1], res[1]), vbslq_u32(t182[0], t183[2], res[2]), vbslq_u32(t182[1], t183[3], res[3])];
                        live = [vbicq_u32(live[0], t182[0]), vbicq_u32(live[1], t182[1])];
                    }
                    if any(&live) {
                        let t184: [uint32x4_t; 2] = [vceqq_u32(x48[0], t149[0]), vceqq_u32(x48[1], t149[1])];
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
                            let t192: [uint32x4_t; 4] = [vorrq_u32(t190[0], x48[0]), vorrq_u32(t190[1], x48[1]), t191[0], t191[1]];
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
unsafe fn b2_r0r1i_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t193 = b3_r0r1si_p(l, lid0, live, arg0, arg1, 0u32, x16);
    res = t193;
    return res;
}

#[inline(always)]
unsafe fn b2_r0r1i_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t194 = b3_r0r1si_f(l, lid0, arg0, arg1, 0u32, x16);
    res = t194;
    return res;
}

#[inline(always)]
unsafe fn b2_r0r1u8_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t195 = b3_r0r1su8_p(l, lid0, live, arg0, arg1, 0u32, x16);
    res = t195;
    return res;
}

#[inline(always)]
unsafe fn b2_r0r1u8_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t196 = b3_r0r1su8_f(l, lid0, arg0, arg1, 0u32, x16);
    res = t196;
    return res;
}

#[inline(always)]
unsafe fn b3_r0r1si_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: u64 = 0;
    let mut x56: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t197 = b4_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    x32 = t197;
    let t198: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[1].as_ptr()))];
    let t199: [uint32x4_t; 2] = [vshlq_u32(t198[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(t198[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t200 = b5_r0r1sl_p(l, lid0, live, arg0, arg1, (1u32).wrapping_add(x16), t199);
    x40 = t200;
    let t201 = b6_s_p(l, lid0, live, x32);
    x48 = t201;
    let t202 = [vdupq_n_u32(1023u32); 2];
    let t203: [uint32x4_t; 2] = [vandq_u32(t198[0], t202[0]), vandq_u32(t198[1], t202[1])];
    let t204 = [vdupq_n_u32(x32); 2];
    let t205: [uint32x4_t; 2] = [vmulq_u32(t203[0], t204[0]), vmulq_u32(t203[1], t204[1])];
    let t206 = b7_r0r1lls_p(l, lid0, live, arg0, arg1, x40, t205, 8u32);
    let t207: u64 = x48;
    let t208 = [vdupq_n_u32((t207) as u32); 2];
    let t209 = [vdupq_n_u32(((t207) >> 32) as u32); 2];
    let t210: [uint32x4_t; 4] = [vandq_u32(t206[0], t208[0]), vandq_u32(t206[1], t208[1]), vandq_u32(t206[2], t209[0]), vandq_u32(t206[3], t209[1])];
    x56 = t210;
    let t211: u64 = x48;
    let t212 = [vdupq_n_u32((t211) as u32); 2];
    let t213 = [vdupq_n_u32(((t211) >> 32) as u32); 2];
    let t214: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x56[0], t212[0]), vceqq_u32(x56[2], t213[0])), vandq_u32(vceqq_u32(x56[1], t212[1]), vceqq_u32(x56[3], t213[1]))];
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
                let t222 = b8_r0r1ss_f(l, lid0, arg0, arg1, (x16).wrapping_add(1u32), (x24).wrapping_add(j as u32));
                let t223: u64 = t222;
                t220[j] = t223 as u32;
                t220[8 + j] = (t223 >> 32) as u32;
            }
            t218 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t220);
        } else {
            let t224 = b8_r0r1si_p(l, lid0, t216, arg0, arg1, (x16).wrapping_add(1u32), x24);
            t218 = t224;
        }
        t215 = [vbslq_u32(t216[0], t218[0], t215[0]), vbslq_u32(t216[1], t218[1], t215[1]), vbslq_u32(t216[0], t218[2], t215[2]), vbslq_u32(t216[1], t218[3], t215[3])];
    }
    {
        t215 = [vbslq_u32(t217[0], x56[0], t215[0]), vbslq_u32(t217[1], x56[1], t215[1]), vbslq_u32(t217[0], x56[2], t215[2]), vbslq_u32(t217[1], x56[3], t215[3])];
    }
    res = t215;
    return res;
}

#[inline(always)]
unsafe fn b3_r0r1si_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32, x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: u64 = 0;
    let mut x56: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t225 = b4_r0r1s_f(l, lid0, arg0, arg1, x16);
    x32 = t225;
    let t226: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[1].as_ptr()))];
    let t227: [uint32x4_t; 2] = [vshlq_u32(t226[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(t226[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t228 = b5_r0r1sl_f(l, lid0, arg0, arg1, (1u32).wrapping_add(x16), t227);
    x40 = t228;
    let t229 = b6_s_f(l, lid0, x32);
    x48 = t229;
    let t230 = [vdupq_n_u32(1023u32); 2];
    let t231: [uint32x4_t; 2] = [vandq_u32(t226[0], t230[0]), vandq_u32(t226[1], t230[1])];
    let t232 = [vdupq_n_u32(x32); 2];
    let t233: [uint32x4_t; 2] = [vmulq_u32(t231[0], t232[0]), vmulq_u32(t231[1], t232[1])];
    let t234 = b7_r0r1lls_f(l, lid0, arg0, arg1, x40, t233, 8u32);
    let t235: u64 = x48;
    let t236 = [vdupq_n_u32((t235) as u32); 2];
    let t237 = [vdupq_n_u32(((t235) >> 32) as u32); 2];
    let t238: [uint32x4_t; 4] = [vandq_u32(t234[0], t236[0]), vandq_u32(t234[1], t236[1]), vandq_u32(t234[2], t237[0]), vandq_u32(t234[3], t237[1])];
    x56 = t238;
    let t239: u64 = x48;
    let t240 = [vdupq_n_u32((t239) as u32); 2];
    let t241 = [vdupq_n_u32(((t239) >> 32) as u32); 2];
    let t242: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x56[0], t240[0]), vceqq_u32(x56[2], t241[0])), vandq_u32(vceqq_u32(x56[1], t240[1]), vceqq_u32(x56[3], t241[1]))];
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
                let t249 = b8_r0r1ss_f(l, lid0, arg0, arg1, (x16).wrapping_add(1u32), (x24).wrapping_add(j as u32));
                let t250: u64 = t249;
                t247[j] = t250 as u32;
                t247[8 + j] = (t250 >> 32) as u32;
            }
            t245 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t247);
        } else {
            let t251 = b8_r0r1si_p(l, lid0, t242, arg0, arg1, (x16).wrapping_add(1u32), x24);
            t245 = t251;
        }
        t243 = [vbslq_u32(t242[0], t245[0], t243[0]), vbslq_u32(t242[1], t245[1], t243[1]), vbslq_u32(t242[0], t245[2], t243[2]), vbslq_u32(t242[1], t245[3], t243[3])];
    }
    {
        t243 = [vbslq_u32(t244[0], x56[0], t243[0]), vbslq_u32(t244[1], x56[1], t243[1]), vbslq_u32(t244[0], x56[2], t243[2]), vbslq_u32(t244[1], x56[3], t243[3])];
    }
    res = t243;
    return res;
}

#[inline(always)]
unsafe fn b3_r0r1su8_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: u32 = 0;
    let mut x48: u64 = 0;
    let mut x56: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t252 = b4_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    x32 = t252;
    let t253 = b5_r0r1ss_p(l, lid0, live, arg0, arg1, (1u32).wrapping_add(x16), (x24 >> 10));
    x40 = t253;
    let t254 = b6_s_p(l, lid0, live, x32);
    x48 = t254;
    let t255 = b7_r0r1sns_p(l, lid0, live, arg0, arg1, x40, (x24 & 1023u32).wrapping_mul(x32), x32, 8u32);
    let t256: u64 = x48;
    let t257 = [vdupq_n_u32((t256) as u32); 2];
    let t258 = [vdupq_n_u32(((t256) >> 32) as u32); 2];
    let t259: [uint32x4_t; 4] = [vandq_u32(t255[0], t257[0]), vandq_u32(t255[1], t257[1]), vandq_u32(t255[2], t258[0]), vandq_u32(t255[3], t258[1])];
    x56 = t259;
    let t260: u64 = x48;
    let t261 = [vdupq_n_u32((t260) as u32); 2];
    let t262 = [vdupq_n_u32(((t260) >> 32) as u32); 2];
    let t263: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x56[0], t261[0]), vceqq_u32(x56[2], t262[0])), vandq_u32(vceqq_u32(x56[1], t261[1]), vceqq_u32(x56[3], t262[1]))];
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
                let t271 = b8_r0r1ss_f(l, lid0, arg0, arg1, (x16).wrapping_add(1u32), (x24).wrapping_add(j as u32));
                let t272: u64 = t271;
                t269[j] = t272 as u32;
                t269[8 + j] = (t272 >> 32) as u32;
            }
            t267 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t269);
        } else {
            let t273 = b8_r0r1su8_p(l, lid0, t265, arg0, arg1, (x16).wrapping_add(1u32), x24);
            t267 = t273;
        }
        t264 = [vbslq_u32(t265[0], t267[0], t264[0]), vbslq_u32(t265[1], t267[1], t264[1]), vbslq_u32(t265[0], t267[2], t264[2]), vbslq_u32(t265[1], t267[3], t264[3])];
    }
    {
        t264 = [vbslq_u32(t266[0], x56[0], t264[0]), vbslq_u32(t266[1], x56[1], t264[1]), vbslq_u32(t266[0], x56[2], t264[2]), vbslq_u32(t266[1], x56[3], t264[3])];
    }
    res = t264;
    return res;
}

#[inline(always)]
unsafe fn b3_r0r1su8_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32, x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: u32 = 0;
    let mut x48: u64 = 0;
    let mut x56: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t274 = b4_r0r1s_f(l, lid0, arg0, arg1, x16);
    x32 = t274;
    let t275 = b5_r0r1ss_f(l, lid0, arg0, arg1, (1u32).wrapping_add(x16), (x24 >> 10));
    x40 = t275;
    let t276 = b6_s_f(l, lid0, x32);
    x48 = t276;
    let t277 = b7_r0r1sns_f(l, lid0, arg0, arg1, x40, (x24 & 1023u32).wrapping_mul(x32), x32, 8u32);
    let t278: u64 = x48;
    let t279 = [vdupq_n_u32((t278) as u32); 2];
    let t280 = [vdupq_n_u32(((t278) >> 32) as u32); 2];
    let t281: [uint32x4_t; 4] = [vandq_u32(t277[0], t279[0]), vandq_u32(t277[1], t279[1]), vandq_u32(t277[2], t280[0]), vandq_u32(t277[3], t280[1])];
    x56 = t281;
    let t282: u64 = x48;
    let t283 = [vdupq_n_u32((t282) as u32); 2];
    let t284 = [vdupq_n_u32(((t282) >> 32) as u32); 2];
    let t285: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x56[0], t283[0]), vceqq_u32(x56[2], t284[0])), vandq_u32(vceqq_u32(x56[1], t283[1]), vceqq_u32(x56[3], t284[1]))];
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
                let t292 = b8_r0r1ss_f(l, lid0, arg0, arg1, (x16).wrapping_add(1u32), (x24).wrapping_add(j as u32));
                let t293: u64 = t292;
                t290[j] = t293 as u32;
                t290[8 + j] = (t293 >> 32) as u32;
            }
            t288 = core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(t290);
        } else {
            let t294 = b8_r0r1su8_p(l, lid0, t285, arg0, arg1, (x16).wrapping_add(1u32), x24);
            t288 = t294;
        }
        t286 = [vbslq_u32(t285[0], t288[0], t286[0]), vbslq_u32(t285[1], t288[1], t286[1]), vbslq_u32(t285[0], t288[2], t286[2]), vbslq_u32(t285[1], t288[3], t286[3])];
    }
    {
        t286 = [vbslq_u32(t287[0], x56[0], t286[0]), vbslq_u32(t287[1], x56[1], t286[1]), vbslq_u32(t287[0], x56[2], t286[2]), vbslq_u32(t287[1], x56[3], t286[3])];
    }
    res = t286;
    return res;
}

#[inline(always)]
unsafe fn b4_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t295 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    res = (((t295).wrapping_shr(((4u32).wrapping_mul(x16)) as u32)) & (15u32));
    return res;
}

#[inline(always)]
unsafe fn b4_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t296 = b9_r0r1s_f(l, lid0, arg0, arg1, 16u32);
    res = (((t296).wrapping_shr(((4u32).wrapping_mul(x16)) as u32)) & (15u32));
    return res;
}

#[inline(always)]
unsafe fn b5_r0r1ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: u32 = 0;
    let mut live = k;
    let mut res: u32 = 0;
    let t297 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 40u32);
    let t298 = b10_r0r1s_p(l, lid0, live, arg0, arg1, 0u32);
    x32 = ((x24).wrapping_mul(t297)).wrapping_add(t298);
    if ((x16) == (0u32)) {
        res = x32;
        return res;
    }
    let t299 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    let t300 = b11_ss_p(l, lid0, live, (t299).wrapping_sub((x24).wrapping_shl((10u32) as u32)), 1024u32);
    x40 = t300;
    let t301 = b10_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    res = (x32).wrapping_add((x40).wrapping_mul(t301));
    return res;
}

#[inline(always)]
unsafe fn b5_r0r1ss_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32, x24: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: u32 = 0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t302 = b9_r0r1s_f(l, lid0, arg0, arg1, 40u32);
    let t303 = b10_r0r1s_f(l, lid0, arg0, arg1, 0u32);
    x32 = ((x24).wrapping_mul(t302)).wrapping_add(t303);
    if ((x16) == (0u32)) {
        res = x32;
        return res;
    }
    let t304 = b9_r0r1s_f(l, lid0, arg0, arg1, 4u32);
    let t305 = b11_ss_f(l, lid0, (t304).wrapping_sub((x24).wrapping_shl((10u32) as u32)), 1024u32);
    x40 = t305;
    let t306 = b10_r0r1s_f(l, lid0, arg0, arg1, x16);
    res = (x32).wrapping_add((x40).wrapping_mul(t306));
    return res;
}

#[inline(always)]
unsafe fn b5_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t307 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 40u32);
    let t308 = [vdupq_n_u32(t307); 2];
    let t309: [uint32x4_t; 2] = [vmulq_u32(x24[0], t308[0]), vmulq_u32(x24[1], t308[1])];
    let t310 = b10_r0r1s_p(l, lid0, live, arg0, arg1, 0u32);
    let t311 = [vdupq_n_u32(t310); 2];
    let t312: [uint32x4_t; 2] = [vaddq_u32(t309[0], t311[0]), vaddq_u32(t309[1], t311[1])];
    x32 = t312;
    if ((x16) == (0u32)) {
        res = x32;
        return res;
    }
    let t313 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    let t314: [uint32x4_t; 2] = [vshlq_u32(x24[0], vdupq_n_s32(((10u32) & 31) as i32)), vshlq_u32(x24[1], vdupq_n_s32(((10u32) & 31) as i32))];
    let t315 = [vdupq_n_u32(t313); 2];
    let t316: [uint32x4_t; 2] = [vsubq_u32(t315[0], t314[0]), vsubq_u32(t315[1], t314[1])];
    let t317 = b11_ls_p(l, lid0, live, t316, 1024u32);
    x40 = t317;
    let t318 = b10_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    let t319 = [vdupq_n_u32(t318); 2];
    let t320: [uint32x4_t; 2] = [vmulq_u32(x40[0], t319[0]), vmulq_u32(x40[1], t319[1])];
    let t321: [uint32x4_t; 2] = [vaddq_u32(x32[0], t320[0]), vaddq_u32(x32[1], t320[1])];
    res = t321;
    return res;
}

#[inline(always)]
unsafe fn b5_r0r1sl_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t322 = b9_r0r1s_f(l, lid0, arg0, arg1, 40u32);
    let t323 = [vdupq_n_u32(t322); 2];
    let t324: [uint32x4_t; 2] = [vmulq_u32(x24[0], t323[0]), vmulq_u32(x24[1], t323[1])];
    let t325 = b10_r0r1s_f(l, lid0, arg0, arg1, 0u32);
    let t326 = [vdupq_n_u32(t325); 2];
    let t327: [uint32x4_t; 2] = [vaddq_u32(t324[0], t326[0]), vaddq_u32(t324[1], t326[1])];
    x32 = t327;
    if ((x16) == (0u32)) {
        res = x32;
        return res;
    }
    let t328 = b9_r0r1s_f(l, lid0, arg0, arg1, 4u32);
    let t329: [uint32x4_t; 2] = [vshlq_u32(x24[0], vdupq_n_s32(((10u32) & 31) as i32)), vshlq_u32(x24[1], vdupq_n_s32(((10u32) & 31) as i32))];
    let t330 = [vdupq_n_u32(t328); 2];
    let t331: [uint32x4_t; 2] = [vsubq_u32(t330[0], t329[0]), vsubq_u32(t330[1], t329[1])];
    let t332 = b11_ls_f(l, lid0, t331, 1024u32);
    x40 = t332;
    let t333 = b10_r0r1s_f(l, lid0, arg0, arg1, x16);
    let t334 = [vdupq_n_u32(t333); 2];
    let t335: [uint32x4_t; 2] = [vmulq_u32(x40[0], t334[0]), vmulq_u32(x40[1], t334[1])];
    let t336: [uint32x4_t; 2] = [vaddq_u32(x32[0], t335[0]), vaddq_u32(x32[1], t335[1])];
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
unsafe fn b6_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t339 = [vdupq_n_u32(8u32); 2];
    let t340: [uint32x4_t; 2] = [vcgeq_u32(x0[0], t339[0]), vcgeq_u32(x0[1], t339[1])];
    let mut t341: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t342: [uint32x4_t; 2] = [vandq_u32(live[0], t340[0]), vandq_u32(live[1], t340[1])];
    let t343: [uint32x4_t; 2] = [vbicq_u32(live[0], t340[0]), vbicq_u32(live[1], t340[1])];
    {
        let t344: u64 = !(0u64);
        let t345 = [vdupq_n_u32((t344) as u32); 2];
        let t346 = [vdupq_n_u32(((t344) >> 32) as u32); 2];
        let t347: [uint32x4_t; 4] = [t345[0], t345[1], t346[0], t346[1]];
        t341 = [vbslq_u32(t342[0], t347[0], t341[0]), vbslq_u32(t342[1], t347[1], t341[1]), vbslq_u32(t342[0], t347[2], t341[2]), vbslq_u32(t342[1], t347[3], t341[3])];
    }
    {
        let t348: [uint32x4_t; 2] = [vmulq_u32(x0[0], t339[0]), vmulq_u32(x0[1], t339[1])];
        let t349: u64 = 1u64;
        let t350 = [vdupq_n_u32((t349) as u32); 2];
        let t351: [uint32x4_t; 2] = [t348[0], t348[1]];
        let t352: [uint32x4_t; 2] = [vandq_u32(t351[0], vdupq_n_u32(63)), vandq_u32(t351[1], vdupq_n_u32(63))];
        let t353: [uint32x4_t; 4] = [vshlq_u32(t350[0], vreinterpretq_s32_u32(t352[0])), vshlq_u32(t350[1], vreinterpretq_s32_u32(t352[1])), vshlq_u32(t350[0], vsubq_s32(vreinterpretq_s32_u32(t352[0]), vdupq_n_s32(32))), vshlq_u32(t350[1], vsubq_s32(vreinterpretq_s32_u32(t352[1]), vdupq_n_s32(32)))];
        let t354: u64 = 1u64;
        let t355 = [vdupq_n_u32((t354) as u32); 2];
        let t356: [uint32x4_t; 2] = [vcltq_u32(t353[0], t355[0]), vcltq_u32(t353[1], t355[1])];
        let t357: [uint32x4_t; 4] = [vsubq_u32(t353[0], t355[0]), vsubq_u32(t353[1], t355[1]), vaddq_u32(vsubq_u32(t353[2], vdupq_n_u32(0)), t356[0]), vaddq_u32(vsubq_u32(t353[3], vdupq_n_u32(0)), t356[1])];
        t341 = [vbslq_u32(t343[0], t357[0], t341[0]), vbslq_u32(t343[1], t357[1], t341[1]), vbslq_u32(t343[0], t357[2], t341[2]), vbslq_u32(t343[1], t357[3], t341[3])];
    }
    res = t341;
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1sns_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24f: u32, x24s: u32, x32: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t358 = b12_r0sns_p(l, lid0, live, arg0, x16, x24f, x24s, x32);
    res = t358;
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1sns_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32, x24f: u32, x24s: u32, x32: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t359 = b12_r0sns_f(l, lid0, arg0, x16, x24f, x24s, x32);
    res = t359;
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2], x24: [uint32x4_t; 2], x32: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t360 = b12_r0lls_p(l, lid0, live, arg0, x16, x24, x32);
    res = t360;
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1lls_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2], x24: [uint32x4_t; 2], x32: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t361 = b12_r0lls_f(l, lid0, arg0, x16, x24, x32);
    res = t361;
    return res;
}

#[inline(always)]
unsafe fn b8_r0r1ss_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32, x24: u32) -> u64 {
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
    let t362 = b9_r0r1s_f(l, lid0, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x16)));
    x32 = t362;
    let t363 = b9_r0r1s_f(l, lid0, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x16)));
    x40 = t363;
    x48 = (x24).wrapping_shr((10u32) as u32);
    let t364 = b13_r0r1sss_f(l, lid0, arg0, arg1, x40, (x48).wrapping_mul(4u32), 4u32);
    x56 = ((t364) as u32);
    let t365 = b13_r0r1sss_f(l, lid0, arg0, arg1, x40, ((x48).wrapping_mul(4u32)).wrapping_add(4u32), 4u32);
    x64 = ((t365) as u32);
    let t366 = b9_r0r1s_f(l, lid0, arg0, arg1, 4u32);
    x72 = ((t366).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
    let t367 = b14_ss_f(l, lid0, 4u32, (x72).wrapping_sub((x48).wrapping_mul(4u32)));
    x80 = t367;
    x88 = (((x24).wrapping_shr((8u32) as u32)) & (3u32));
    let t368: u32;
    if ((x88) == (0u32)) {
        t368 = 0u32;
    } else {
        let t369 = b15_ss_f(l, lid0, (x56).wrapping_add((2u32).wrapping_mul((x88).wrapping_sub(1u32))), x64);
        let t370 = b13_r0r1sss_f(l, lid0, arg0, arg1, x32, t369, 2u32);
        t368 = ((t370) as u32);
    }
    x96 = t368;
    let t371 = b15_ss_f(l, lid0, (x56).wrapping_add((2u32).wrapping_mul(x88)), x64);
    let t372 = b13_r0r1sss_f(l, lid0, arg0, arg1, x32, t371, 2u32);
    x104 = ((t372) as u32);
    x112 = (x56).wrapping_add((2u32).wrapping_mul(x80));
    x120 = ((x24) & (255u32));
    x128 = 9259542123273814144u64;
    x136 = (((x120) as u64)).wrapping_mul(72340172838076673u64);
    x144 = x96;
    x152 = x96;
    loop {
        // Rounds of one condition: every lane goes on.
        if !(((x152) < (x104))) { break; }
        let t373 = b15_ss_f(l, lid0, (x112).wrapping_add(x152), x64);
        let t374 = b13_r0r1sss_f(l, lid0, arg0, arg1, x32, t373, 8u32);
        x160 = t374;
        x168 = (((x160) | (x128))).wrapping_sub(((x136) & (!(x128))));
        x176 = ((x128) & (((((!(x160)) & (x136))) | (((!(((x160) ^ (x136)))) & (!(x168)))))));
        x184 = (x104).wrapping_sub(x152);
        let t375: u64;
        if ((x184) >= (8u32)) {
            t375 = x176;
        } else {
            t375 = ((x176) & (((1u64).wrapping_shl(((((8u32).wrapping_mul(x184)) as u64)) as u32)).wrapping_sub(1u64)));
        }
        let t376 = b16_s_f(l, lid0, t375);
        x144 = (x144).wrapping_add(t376);
        x152 = (x152).wrapping_add(8u32);
    }
    x96 = x144;
    let t377 = b9_r0r1s_f(l, lid0, arg0, arg1, 16u32);
    x152 = (((t377).wrapping_shr((((((4u32).wrapping_mul(x16)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32));
    let t378 = b15_ss_f(l, lid0, (x56).wrapping_add((2u32).wrapping_mul((x80).wrapping_sub(1u32))), x64);
    let t379 = b13_r0r1sss_f(l, lid0, arg0, arg1, x32, t378, 2u32);
    x160 = ((((t379) as u32)) as u64);
    let t380 = b15_ss_f(l, lid0, ((x112).wrapping_add(((x160) as u32))).wrapping_add((x96).wrapping_mul(x152)), x64);
    x168 = ((t380) as u64);
    let t381 = b13_r0r1sss_f(l, lid0, arg0, arg1, x32, ((x168) as u32), 8u32);
    let t382 = b6_s_f(l, lid0, x152);
    res = ((t381) & (t382));
    return res;
}

#[inline(always)]
unsafe fn b8_r0r1si_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: u32) -> [uint32x4_t; 4] {
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
    let t383 = b9_r0r1s_p(l, lid0, live, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x16)));
    x32 = t383;
    let t384 = b9_r0r1s_p(l, lid0, live, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x16)));
    x40 = t384;
    let t385: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[1].as_ptr()))];
    let t386: [uint32x4_t; 2] = [vshlq_u32(t385[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(t385[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    x48 = t386;
    let t387 = [vdupq_n_u32(4u32); 2];
    let t388: [uint32x4_t; 2] = [vmulq_u32(x48[0], t387[0]), vmulq_u32(x48[1], t387[1])];
    let t389 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x40, t388, 4u32);
    let t390: [uint32x4_t; 2] = [t389[0], t389[1]];
    x56 = t390;
    let t391: [uint32x4_t; 2] = [vmulq_u32(x48[0], t387[0]), vmulq_u32(x48[1], t387[1])];
    let t392: [uint32x4_t; 2] = [vaddq_u32(t391[0], t387[0]), vaddq_u32(t391[1], t387[1])];
    let t393 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x40, t392, 4u32);
    let t394: [uint32x4_t; 2] = [t393[0], t393[1]];
    x64 = t394;
    let t395 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    x72 = ((t395).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
    let t396: [uint32x4_t; 2] = [vmulq_u32(x48[0], t387[0]), vmulq_u32(x48[1], t387[1])];
    let t397 = [vdupq_n_u32(x72); 2];
    let t398: [uint32x4_t; 2] = [vsubq_u32(t397[0], t396[0]), vsubq_u32(t397[1], t396[1])];
    let t399 = b14_sl_p(l, lid0, live, 4u32, t398);
    x80 = t399;
    let t400: [uint32x4_t; 2] = [vshlq_u32(t385[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(t385[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
    let t401 = [vdupq_n_u32(3u32); 2];
    let t402: [uint32x4_t; 2] = [vandq_u32(t400[0], t401[0]), vandq_u32(t400[1], t401[1])];
    x88 = t402;
    let t403 = [vdupq_n_u32(0u32); 2];
    let t404: [uint32x4_t; 2] = [vceqq_u32(x88[0], t403[0]), vceqq_u32(x88[1], t403[1])];
    let mut t405: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t406: [uint32x4_t; 2] = [vandq_u32(live[0], t404[0]), vandq_u32(live[1], t404[1])];
    let t407: [uint32x4_t; 2] = [vbicq_u32(live[0], t404[0]), vbicq_u32(live[1], t404[1])];
    {
        t405 = [vbslq_u32(t406[0], t403[0], t405[0]), vbslq_u32(t406[1], t403[1], t405[1])];
    }
    if any(&t407) {
        let t408 = [vdupq_n_u32(1u32); 2];
        let t409: [uint32x4_t; 2] = [vsubq_u32(x88[0], t408[0]), vsubq_u32(x88[1], t408[1])];
        let t410 = [vdupq_n_u32(2u32); 2];
        let t411: [uint32x4_t; 2] = [vmulq_u32(t410[0], t409[0]), vmulq_u32(t410[1], t409[1])];
        let t412: [uint32x4_t; 2] = [vaddq_u32(x56[0], t411[0]), vaddq_u32(x56[1], t411[1])];
        let t413 = b15_ll_p(l, lid0, t407, t412, x64);
        let t414 = b13_r0r1sls_p(l, lid0, t407, arg0, arg1, x32, t413, 2u32);
        let t415: [uint32x4_t; 2] = [t414[0], t414[1]];
        t405 = [vbslq_u32(t407[0], t415[0], t405[0]), vbslq_u32(t407[1], t415[1], t405[1])];
    }
    x96 = t405;
    let t416 = [vdupq_n_u32(2u32); 2];
    let t417: [uint32x4_t; 2] = [vmulq_u32(t416[0], x88[0]), vmulq_u32(t416[1], x88[1])];
    let t418: [uint32x4_t; 2] = [vaddq_u32(x56[0], t417[0]), vaddq_u32(x56[1], t417[1])];
    let t419 = b15_ll_p(l, lid0, live, t418, x64);
    let t420 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t419, 2u32);
    let t421: [uint32x4_t; 2] = [t420[0], t420[1]];
    x104 = t421;
    let t422: [uint32x4_t; 2] = [vmulq_u32(t416[0], x80[0]), vmulq_u32(t416[1], x80[1])];
    let t423: [uint32x4_t; 2] = [vaddq_u32(x56[0], t422[0]), vaddq_u32(x56[1], t422[1])];
    x112 = t423;
    let t424 = [vdupq_n_u32(255u32); 2];
    let t425: [uint32x4_t; 2] = [vandq_u32(t385[0], t424[0]), vandq_u32(t385[1], t424[1])];
    x120 = t425;
    x128 = 9259542123273814144u64;
    let t426: u64 = 72340172838076673u64;
    let t427 = [vdupq_n_u32((t426) as u32); 2];
    let t428 = [vdupq_n_u32(((t426) >> 32) as u32); 2];
    let t429: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x120[0]), vget_low_u32(t427[0]))), vreinterpretq_u32_u64(vmull_high_u32(x120[0], t427[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x120[1]), vget_low_u32(t427[1]))), vreinterpretq_u32_u64(vmull_high_u32(x120[1], t427[1])))];
    let t430: [uint32x4_t; 4] = [vmulq_u32(x120[0], t427[0]), vmulq_u32(x120[1], t427[1]), vmlaq_u32(t429[0], x120[0], t428[0]), vmlaq_u32(t429[1], x120[1], t428[1])];
    x136 = t430;
    x144 = x96;
    x152 = x96;
    let mut t432 = live;
    loop {
        // Lanes part: each its own rounds.
        let t434: [uint32x4_t; 2] = [vcltq_u32(x152[0], x104[0]), vcltq_u32(x152[1], x104[1])];
        t432 = [vandq_u32(t432[0], t434[0]), vandq_u32(t432[1], t434[1])];
        if !(any(&t432)) { break; }
        let t436: [uint32x4_t; 2] = [vaddq_u32(x112[0], x152[0]), vaddq_u32(x112[1], x152[1])];
        let t437 = b15_ll_p(l, lid0, t432, t436, x64);
        let t438 = b13_r0r1sls_p(l, lid0, t432, arg0, arg1, x32, t437, 8u32);
        x160 = [vbslq_u32(t432[0], t438[0], x160[0]), vbslq_u32(t432[1], t438[1], x160[1]), vbslq_u32(t432[0], t438[2], x160[2]), vbslq_u32(t432[1], t438[3], x160[3])];
        let t439: u64 = x128;
        let t440 = [vdupq_n_u32((t439) as u32); 2];
        let t441 = [vdupq_n_u32(((t439) >> 32) as u32); 2];
        let t442: [uint32x4_t; 4] = [vorrq_u32(x160[0], t440[0]), vorrq_u32(x160[1], t440[1]), vorrq_u32(x160[2], t441[0]), vorrq_u32(x160[3], t441[1])];
        let t443: u64 = !(x128);
        let t444 = [vdupq_n_u32((t443) as u32); 2];
        let t445 = [vdupq_n_u32(((t443) >> 32) as u32); 2];
        let t446: [uint32x4_t; 4] = [vandq_u32(x136[0], t444[0]), vandq_u32(x136[1], t444[1]), vandq_u32(x136[2], t445[0]), vandq_u32(x136[3], t445[1])];
        let t447: [uint32x4_t; 2] = [vcltq_u32(t442[0], t446[0]), vcltq_u32(t442[1], t446[1])];
        let t448: [uint32x4_t; 4] = [vsubq_u32(t442[0], t446[0]), vsubq_u32(t442[1], t446[1]), vaddq_u32(vsubq_u32(t442[2], t446[2]), t447[0]), vaddq_u32(vsubq_u32(t442[3], t446[3]), t447[1])];
        x168 = [vbslq_u32(t432[0], t448[0], x168[0]), vbslq_u32(t432[1], t448[1], x168[1]), vbslq_u32(t432[0], t448[2], x168[2]), vbslq_u32(t432[1], t448[3], x168[3])];
        let t449: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x160[r]));
        let t450: [uint32x4_t; 4] = [vandq_u32(t449[0], x136[0]), vandq_u32(t449[1], x136[1]), vandq_u32(t449[2], x136[2]), vandq_u32(t449[3], x136[3])];
        let t451: [uint32x4_t; 4] = [veorq_u32(x160[0], x136[0]), veorq_u32(x160[1], x136[1]), veorq_u32(x160[2], x136[2]), veorq_u32(x160[3], x136[3])];
        let t452: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(t451[r]));
        let t453: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x168[r]));
        let t454: [uint32x4_t; 4] = [vandq_u32(t452[0], t453[0]), vandq_u32(t452[1], t453[1]), vandq_u32(t452[2], t453[2]), vandq_u32(t452[3], t453[3])];
        let t455: [uint32x4_t; 4] = [vorrq_u32(t450[0], t454[0]), vorrq_u32(t450[1], t454[1]), vorrq_u32(t450[2], t454[2]), vorrq_u32(t450[3], t454[3])];
        let t456: u64 = x128;
        let t457 = [vdupq_n_u32((t456) as u32); 2];
        let t458 = [vdupq_n_u32(((t456) >> 32) as u32); 2];
        let t459: [uint32x4_t; 4] = [vandq_u32(t457[0], t455[0]), vandq_u32(t457[1], t455[1]), vandq_u32(t458[0], t455[2]), vandq_u32(t458[1], t455[3])];
        x176 = t459;
        let t460: [uint32x4_t; 2] = [vsubq_u32(x104[0], x152[0]), vsubq_u32(x104[1], x152[1])];
        x184 = t460;
        let t461 = [vdupq_n_u32(8u32); 2];
        let t462: [uint32x4_t; 2] = [vcgeq_u32(x184[0], t461[0]), vcgeq_u32(x184[1], t461[1])];
        let mut t463: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t464: [uint32x4_t; 2] = [vandq_u32(t432[0], t462[0]), vandq_u32(t432[1], t462[1])];
        let t465: [uint32x4_t; 2] = [vbicq_u32(t432[0], t462[0]), vbicq_u32(t432[1], t462[1])];
        {
            t463 = [vbslq_u32(t464[0], x176[0], t463[0]), vbslq_u32(t464[1], x176[1], t463[1]), vbslq_u32(t464[0], x176[2], t463[2]), vbslq_u32(t464[1], x176[3], t463[3])];
        }
        {
            let t466: [uint32x4_t; 2] = [vmulq_u32(t461[0], x184[0]), vmulq_u32(t461[1], x184[1])];
            let t467: u64 = 1u64;
            let t468 = [vdupq_n_u32((t467) as u32); 2];
            let t469: [uint32x4_t; 2] = [t466[0], t466[1]];
            let t470: [uint32x4_t; 2] = [vandq_u32(t469[0], vdupq_n_u32(63)), vandq_u32(t469[1], vdupq_n_u32(63))];
            let t471: [uint32x4_t; 4] = [vshlq_u32(t468[0], vreinterpretq_s32_u32(t470[0])), vshlq_u32(t468[1], vreinterpretq_s32_u32(t470[1])), vshlq_u32(t468[0], vsubq_s32(vreinterpretq_s32_u32(t470[0]), vdupq_n_s32(32))), vshlq_u32(t468[1], vsubq_s32(vreinterpretq_s32_u32(t470[1]), vdupq_n_s32(32)))];
            let t472: u64 = 1u64;
            let t473 = [vdupq_n_u32((t472) as u32); 2];
            let t474: [uint32x4_t; 2] = [vcltq_u32(t471[0], t473[0]), vcltq_u32(t471[1], t473[1])];
            let t475: [uint32x4_t; 4] = [vsubq_u32(t471[0], t473[0]), vsubq_u32(t471[1], t473[1]), vaddq_u32(vsubq_u32(t471[2], vdupq_n_u32(0)), t474[0]), vaddq_u32(vsubq_u32(t471[3], vdupq_n_u32(0)), t474[1])];
            let t476: [uint32x4_t; 4] = [vandq_u32(x176[0], t475[0]), vandq_u32(x176[1], t475[1]), vandq_u32(x176[2], t475[2]), vandq_u32(x176[3], t475[3])];
            t463 = [vbslq_u32(t465[0], t476[0], t463[0]), vbslq_u32(t465[1], t476[1], t463[1]), vbslq_u32(t465[0], t476[2], t463[2]), vbslq_u32(t465[1], t476[3], t463[3])];
        }
        let t477 = b16_l_p(l, lid0, t432, t463);
        let t478: [uint32x4_t; 2] = [vaddq_u32(x144[0], t477[0]), vaddq_u32(x144[1], t477[1])];
        x144 = [vbslq_u32(t432[0], t478[0], x144[0]), vbslq_u32(t432[1], t478[1], x144[1])];
        let t479: [uint32x4_t; 2] = [vandq_u32(t432[0], live[0]), vandq_u32(t432[1], live[1])];
        x152 = [vaddq_u32(x152[0], vandq_u32(t479[0], vdupq_n_u32(8))), vaddq_u32(x152[1], vandq_u32(t479[1], vdupq_n_u32(8)))];
    }
    x96 = x144;
    let t480 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    let t481 = [vdupq_n_u32((((t480).wrapping_shr((((((4u32).wrapping_mul(x16)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32))); 2];
    x152 = t481;
    let t482 = [vdupq_n_u32(1u32); 2];
    let t483: [uint32x4_t; 2] = [vsubq_u32(x80[0], t482[0]), vsubq_u32(x80[1], t482[1])];
    let t484: [uint32x4_t; 2] = [vmulq_u32(t416[0], t483[0]), vmulq_u32(t416[1], t483[1])];
    let t485: [uint32x4_t; 2] = [vaddq_u32(x56[0], t484[0]), vaddq_u32(x56[1], t484[1])];
    let t486 = b15_ll_p(l, lid0, live, t485, x64);
    let t487 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t486, 2u32);
    let t488: [uint32x4_t; 2] = [t487[0], t487[1]];
    let t489: [uint32x4_t; 4] = [t488[0], t488[1], vdupq_n_u32(0), vdupq_n_u32(0)];
    x160 = t489;
    let t490: [uint32x4_t; 2] = [x160[0], x160[1]];
    let t491: [uint32x4_t; 2] = [vaddq_u32(x112[0], t490[0]), vaddq_u32(x112[1], t490[1])];
    let t492: [uint32x4_t; 2] = [vmulq_u32(x96[0], x152[0]), vmulq_u32(x96[1], x152[1])];
    let t493: [uint32x4_t; 2] = [vaddq_u32(t491[0], t492[0]), vaddq_u32(t491[1], t492[1])];
    let t494 = b15_ll_p(l, lid0, live, t493, x64);
    let t495: [uint32x4_t; 4] = [t494[0], t494[1], vdupq_n_u32(0), vdupq_n_u32(0)];
    x168 = t495;
    let t496: [uint32x4_t; 2] = [x168[0], x168[1]];
    let t497 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t496, 8u32);
    let t498 = b6_l_p(l, lid0, live, x152);
    let t499: [uint32x4_t; 4] = [vandq_u32(t497[0], t498[0]), vandq_u32(t497[1], t498[1]), vandq_u32(t497[2], t498[2]), vandq_u32(t497[3], t498[3])];
    res = t499;
    return res;
}

#[inline(always)]
unsafe fn b8_r0r1su8_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: u32) -> [uint32x4_t; 4] {
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
    let t500 = b9_r0r1s_p(l, lid0, live, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x16)));
    x32 = t500;
    let t501 = b9_r0r1s_p(l, lid0, live, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x16)));
    x40 = t501;
    x48 = (x24 >> 10);
    let t502 = b13_r0r1sss_p(l, lid0, live, arg0, arg1, x40, (x48).wrapping_mul(4u32), 4u32);
    x56 = ((t502) as u32);
    let t503 = b13_r0r1sss_p(l, lid0, live, arg0, arg1, x40, ((x48).wrapping_mul(4u32)).wrapping_add(4u32), 4u32);
    x64 = ((t503) as u32);
    let t504 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    x72 = ((t504).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
    let t505 = b14_ss_p(l, lid0, live, 4u32, (x72).wrapping_sub((x48).wrapping_mul(4u32)));
    x80 = t505;
    x88 = (((x24 >> 8)) & (3u32));
    let t506: u32;
    if ((x88) == (0u32)) {
        t506 = 0u32;
    } else {
        let t507 = b15_ss_p(l, lid0, live, (x56).wrapping_add((2u32).wrapping_mul((x88).wrapping_sub(1u32))), x64);
        let t508 = b13_r0r1sss_p(l, lid0, live, arg0, arg1, x32, t507, 2u32);
        t506 = ((t508) as u32);
    }
    let t509 = [vdupq_n_u32(t506); 2];
    x96 = t509;
    let t510 = b15_ss_p(l, lid0, live, (x56).wrapping_add((2u32).wrapping_mul(x88)), x64);
    let t511 = b13_r0r1sss_p(l, lid0, live, arg0, arg1, x32, t510, 2u32);
    x104 = ((t511) as u32);
    x112 = (x56).wrapping_add((2u32).wrapping_mul(x80));
    x120 = (x24 & 255u32);
    x128 = 9259542123273814144u64;
    let t512: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x120), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x120), vld1q_u32(LANE[1].as_ptr()))];
    let t513: u64 = 72340172838076673u64;
    let t514 = [vdupq_n_u32((t513) as u32); 2];
    let t515 = [vdupq_n_u32(((t513) >> 32) as u32); 2];
    let t516: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(t512[0]), vget_low_u32(t514[0]))), vreinterpretq_u32_u64(vmull_high_u32(t512[0], t514[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(t512[1]), vget_low_u32(t514[1]))), vreinterpretq_u32_u64(vmull_high_u32(t512[1], t514[1])))];
    let t517: [uint32x4_t; 4] = [vmulq_u32(t512[0], t514[0]), vmulq_u32(t512[1], t514[1]), vmlaq_u32(t516[0], t512[0], t515[0]), vmlaq_u32(t516[1], t512[1], t515[1])];
    x136 = t517;
    x144 = x96;
    x152 = x96;
    let mut t520 = live;
    loop {
        // Lanes part: each its own rounds.
        let t522 = [vdupq_n_u32(x104); 2];
        let t523: [uint32x4_t; 2] = [vcltq_u32(x152[0], t522[0]), vcltq_u32(x152[1], t522[1])];
        t520 = [vandq_u32(t520[0], t523[0]), vandq_u32(t520[1], t523[1])];
        if !(any(&t520)) { break; }
        let t525 = [vdupq_n_u32(x112); 2];
        let t526: [uint32x4_t; 2] = [vaddq_u32(t525[0], x152[0]), vaddq_u32(t525[1], x152[1])];
        let t527 = b15_ls_p(l, lid0, t520, t526, x64);
        let t528 = b13_r0r1sls_p(l, lid0, t520, arg0, arg1, x32, t527, 8u32);
        x160 = [vbslq_u32(t520[0], t528[0], x160[0]), vbslq_u32(t520[1], t528[1], x160[1]), vbslq_u32(t520[0], t528[2], x160[2]), vbslq_u32(t520[1], t528[3], x160[3])];
        let t529: u64 = x128;
        let t530 = [vdupq_n_u32((t529) as u32); 2];
        let t531 = [vdupq_n_u32(((t529) >> 32) as u32); 2];
        let t532: [uint32x4_t; 4] = [vorrq_u32(x160[0], t530[0]), vorrq_u32(x160[1], t530[1]), vorrq_u32(x160[2], t531[0]), vorrq_u32(x160[3], t531[1])];
        let t533: u64 = !(x128);
        let t534 = [vdupq_n_u32((t533) as u32); 2];
        let t535 = [vdupq_n_u32(((t533) >> 32) as u32); 2];
        let t536: [uint32x4_t; 4] = [vandq_u32(x136[0], t534[0]), vandq_u32(x136[1], t534[1]), vandq_u32(x136[2], t535[0]), vandq_u32(x136[3], t535[1])];
        let t537: [uint32x4_t; 2] = [vcltq_u32(t532[0], t536[0]), vcltq_u32(t532[1], t536[1])];
        let t538: [uint32x4_t; 4] = [vsubq_u32(t532[0], t536[0]), vsubq_u32(t532[1], t536[1]), vaddq_u32(vsubq_u32(t532[2], t536[2]), t537[0]), vaddq_u32(vsubq_u32(t532[3], t536[3]), t537[1])];
        x168 = [vbslq_u32(t520[0], t538[0], x168[0]), vbslq_u32(t520[1], t538[1], x168[1]), vbslq_u32(t520[0], t538[2], x168[2]), vbslq_u32(t520[1], t538[3], x168[3])];
        let t539: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x160[r]));
        let t540: [uint32x4_t; 4] = [vandq_u32(t539[0], x136[0]), vandq_u32(t539[1], x136[1]), vandq_u32(t539[2], x136[2]), vandq_u32(t539[3], x136[3])];
        let t541: [uint32x4_t; 4] = [veorq_u32(x160[0], x136[0]), veorq_u32(x160[1], x136[1]), veorq_u32(x160[2], x136[2]), veorq_u32(x160[3], x136[3])];
        let t542: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(t541[r]));
        let t543: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x168[r]));
        let t544: [uint32x4_t; 4] = [vandq_u32(t542[0], t543[0]), vandq_u32(t542[1], t543[1]), vandq_u32(t542[2], t543[2]), vandq_u32(t542[3], t543[3])];
        let t545: [uint32x4_t; 4] = [vorrq_u32(t540[0], t544[0]), vorrq_u32(t540[1], t544[1]), vorrq_u32(t540[2], t544[2]), vorrq_u32(t540[3], t544[3])];
        let t546: u64 = x128;
        let t547 = [vdupq_n_u32((t546) as u32); 2];
        let t548 = [vdupq_n_u32(((t546) >> 32) as u32); 2];
        let t549: [uint32x4_t; 4] = [vandq_u32(t547[0], t545[0]), vandq_u32(t547[1], t545[1]), vandq_u32(t548[0], t545[2]), vandq_u32(t548[1], t545[3])];
        x176 = t549;
        let t550 = [vdupq_n_u32(x104); 2];
        let t551: [uint32x4_t; 2] = [vsubq_u32(t550[0], x152[0]), vsubq_u32(t550[1], x152[1])];
        x184 = t551;
        let t552 = [vdupq_n_u32(8u32); 2];
        let t553: [uint32x4_t; 2] = [vcgeq_u32(x184[0], t552[0]), vcgeq_u32(x184[1], t552[1])];
        let mut t554: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t555: [uint32x4_t; 2] = [vandq_u32(t520[0], t553[0]), vandq_u32(t520[1], t553[1])];
        let t556: [uint32x4_t; 2] = [vbicq_u32(t520[0], t553[0]), vbicq_u32(t520[1], t553[1])];
        {
            t554 = [vbslq_u32(t555[0], x176[0], t554[0]), vbslq_u32(t555[1], x176[1], t554[1]), vbslq_u32(t555[0], x176[2], t554[2]), vbslq_u32(t555[1], x176[3], t554[3])];
        }
        {
            let t557: [uint32x4_t; 2] = [vmulq_u32(t552[0], x184[0]), vmulq_u32(t552[1], x184[1])];
            let t558: u64 = 1u64;
            let t559 = [vdupq_n_u32((t558) as u32); 2];
            let t560: [uint32x4_t; 2] = [t557[0], t557[1]];
            let t561: [uint32x4_t; 2] = [vandq_u32(t560[0], vdupq_n_u32(63)), vandq_u32(t560[1], vdupq_n_u32(63))];
            let t562: [uint32x4_t; 4] = [vshlq_u32(t559[0], vreinterpretq_s32_u32(t561[0])), vshlq_u32(t559[1], vreinterpretq_s32_u32(t561[1])), vshlq_u32(t559[0], vsubq_s32(vreinterpretq_s32_u32(t561[0]), vdupq_n_s32(32))), vshlq_u32(t559[1], vsubq_s32(vreinterpretq_s32_u32(t561[1]), vdupq_n_s32(32)))];
            let t563: u64 = 1u64;
            let t564 = [vdupq_n_u32((t563) as u32); 2];
            let t565: [uint32x4_t; 2] = [vcltq_u32(t562[0], t564[0]), vcltq_u32(t562[1], t564[1])];
            let t566: [uint32x4_t; 4] = [vsubq_u32(t562[0], t564[0]), vsubq_u32(t562[1], t564[1]), vaddq_u32(vsubq_u32(t562[2], vdupq_n_u32(0)), t565[0]), vaddq_u32(vsubq_u32(t562[3], vdupq_n_u32(0)), t565[1])];
            let t567: [uint32x4_t; 4] = [vandq_u32(x176[0], t566[0]), vandq_u32(x176[1], t566[1]), vandq_u32(x176[2], t566[2]), vandq_u32(x176[3], t566[3])];
            t554 = [vbslq_u32(t556[0], t567[0], t554[0]), vbslq_u32(t556[1], t567[1], t554[1]), vbslq_u32(t556[0], t567[2], t554[2]), vbslq_u32(t556[1], t567[3], t554[3])];
        }
        let t568 = b16_l_p(l, lid0, t520, t554);
        let t569: [uint32x4_t; 2] = [vaddq_u32(x144[0], t568[0]), vaddq_u32(x144[1], t568[1])];
        x144 = [vbslq_u32(t520[0], t569[0], x144[0]), vbslq_u32(t520[1], t569[1], x144[1])];
        let t570: [uint32x4_t; 2] = [vandq_u32(t520[0], live[0]), vandq_u32(t520[1], live[1])];
        x152 = [vaddq_u32(x152[0], vandq_u32(t570[0], vdupq_n_u32(8))), vaddq_u32(x152[1], vandq_u32(t570[1], vdupq_n_u32(8)))];
    }
    x96 = x144;
    let t571 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    let t572 = [vdupq_n_u32((((t571).wrapping_shr((((((4u32).wrapping_mul(x16)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32))); 2];
    x152 = t572;
    let t573 = b15_ss_p(l, lid0, live, (x56).wrapping_add((2u32).wrapping_mul((x80).wrapping_sub(1u32))), x64);
    let t574 = b13_r0r1sss_p(l, lid0, live, arg0, arg1, x32, t573, 2u32);
    let t575: u64 = ((((t574) as u32)) as u64);
    let t576 = [vdupq_n_u32((t575) as u32); 2];
    let t577 = [vdupq_n_u32(((t575) >> 32) as u32); 2];
    let t578: [uint32x4_t; 4] = [t576[0], t576[1], t577[0], t577[1]];
    x160 = t578;
    let t579 = [vdupq_n_u32(x112); 2];
    let t580: [uint32x4_t; 2] = [x160[0], x160[1]];
    let t581: [uint32x4_t; 2] = [vaddq_u32(t579[0], t580[0]), vaddq_u32(t579[1], t580[1])];
    let t582: [uint32x4_t; 2] = [vmulq_u32(x96[0], x152[0]), vmulq_u32(x96[1], x152[1])];
    let t583: [uint32x4_t; 2] = [vaddq_u32(t581[0], t582[0]), vaddq_u32(t581[1], t582[1])];
    let t584 = b15_ls_p(l, lid0, live, t583, x64);
    let t585: [uint32x4_t; 4] = [t584[0], t584[1], vdupq_n_u32(0), vdupq_n_u32(0)];
    x168 = t585;
    let t586: [uint32x4_t; 2] = [x168[0], x168[1]];
    let t587 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t586, 8u32);
    let t588 = b6_l_p(l, lid0, live, x152);
    let t589: [uint32x4_t; 4] = [vandq_u32(t587[0], t588[0]), vandq_u32(t587[1], t588[1]), vandq_u32(t587[2], t588[2]), vandq_u32(t587[3], t588[3])];
    res = t589;
    return res;
}

#[inline(always)]
unsafe fn b9_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    res = (arg1.wrapping_add(((((x16).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned();
    return res;
}

#[inline(always)]
unsafe fn b9_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    res = (arg1.wrapping_add(((((x16).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned();
    return res;
}

#[inline(always)]
unsafe fn b10_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t590 = b9_r0r1s_p(l, lid0, live, arg0, arg1, (20u32).wrapping_add((4u32).wrapping_mul(x16)));
    res = t590;
    return res;
}

#[inline(always)]
unsafe fn b10_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t591 = b9_r0r1s_f(l, lid0, arg0, arg1, (20u32).wrapping_add((4u32).wrapping_mul(x16)));
    res = t591;
    return res;
}

#[inline(always)]
unsafe fn b11_ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x8: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t592: u32;
    if ((x0) < (x8)) {
        t592 = x0;
    } else {
        t592 = x8;
    }
    res = t592;
    return res;
}

#[inline(always)]
unsafe fn b11_ss_f(l: &Launch, lid0: u32, x0: u32, x8: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t593: u32;
    if ((x0) < (x8)) {
        t593 = x0;
    } else {
        t593 = x8;
    }
    res = t593;
    return res;
}

#[inline(always)]
unsafe fn b11_ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x8: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t594 = [vdupq_n_u32(x8); 2];
    let t595: [uint32x4_t; 2] = [vcltq_u32(x0[0], t594[0]), vcltq_u32(x0[1], t594[1])];
    let mut t596: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t597: [uint32x4_t; 2] = [vandq_u32(live[0], t595[0]), vandq_u32(live[1], t595[1])];
    let t598: [uint32x4_t; 2] = [vbicq_u32(live[0], t595[0]), vbicq_u32(live[1], t595[1])];
    {
        t596 = [vbslq_u32(t597[0], x0[0], t596[0]), vbslq_u32(t597[1], x0[1], t596[1])];
    }
    {
        let t599 = [vdupq_n_u32(x8); 2];
        t596 = [vbslq_u32(t598[0], t599[0], t596[0]), vbslq_u32(t598[1], t599[1], t596[1])];
    }
    res = t596;
    return res;
}

#[inline(always)]
unsafe fn b11_ls_f(l: &Launch, lid0: u32, x0: [uint32x4_t; 2], x8: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t600 = [vdupq_n_u32(x8); 2];
    let t601: [uint32x4_t; 2] = [vcltq_u32(x0[0], t600[0]), vcltq_u32(x0[1], t600[1])];
    let mut t602: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t603: [uint32x4_t; 2] = [vmvnq_u32(t601[0]), vmvnq_u32(t601[1])];
    {
        t602 = [vbslq_u32(t601[0], x0[0], t602[0]), vbslq_u32(t601[1], x0[1], t602[1])];
    }
    {
        let t604 = [vdupq_n_u32(x8); 2];
        t602 = [vbslq_u32(t603[0], t604[0], t602[0]), vbslq_u32(t603[1], t604[1], t602[1])];
    }
    res = t602;
    return res;
}

#[inline(always)]
unsafe fn b12_r0sns_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: u32, x16f: u32, x16s: u32, x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t605 = b17_r0an1s_p(l, lid0, live, arg0, ((((x8) as u64)) as usize as *const u8), x16f, x16s, x24);
    x32 = t605;
    let t606: [uint32x4_t; 4];
    if ((x24) >= (8u32)) {
        t606 = x32;
    } else {
        let t607 = b18_s_p(l, lid0, live, (8u32).wrapping_mul(x24));
        let t608: u64 = t607;
        let t609 = [vdupq_n_u32((t608) as u32); 2];
        let t610 = [vdupq_n_u32(((t608) >> 32) as u32); 2];
        let t611: [uint32x4_t; 4] = [vandq_u32(x32[0], t609[0]), vandq_u32(x32[1], t609[1]), vandq_u32(x32[2], t610[0]), vandq_u32(x32[3], t610[1])];
        t606 = t611;
    }
    res = t606;
    return res;
}

#[inline(always)]
unsafe fn b12_r0sns_f(l: &Launch, lid0: u32, arg0: *const u8, x8: u32, x16f: u32, x16s: u32, x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t612 = b17_r0an1s_f(l, lid0, arg0, ((((x8) as u64)) as usize as *const u8), x16f, x16s, x24);
    x32 = t612;
    let t613: [uint32x4_t; 4];
    if ((x24) >= (8u32)) {
        t613 = x32;
    } else {
        let t614 = b18_s_f(l, lid0, (8u32).wrapping_mul(x24));
        let t615: u64 = t614;
        let t616 = [vdupq_n_u32((t615) as u32); 2];
        let t617 = [vdupq_n_u32(((t615) >> 32) as u32); 2];
        let t618: [uint32x4_t; 4] = [vandq_u32(x32[0], t616[0]), vandq_u32(x32[1], t616[1]), vandq_u32(x32[2], t617[0]), vandq_u32(x32[3], t617[1])];
        t613 = t618;
    }
    res = t613;
    return res;
}

#[inline(always)]
unsafe fn b12_r0lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: [uint32x4_t; 2], x16: [uint32x4_t; 2], x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t619: [uint32x4_t; 2] = [vaddq_u32(x8[0], x16[0]), vaddq_u32(x8[1], x16[1])];
    let t620: [uint32x4_t; 2] = [vcltq_u32(t619[0], x8[0]), vcltq_u32(t619[1], x8[1])];
    let t621: [uint32x4_t; 4] = [t619[0], t619[1], vshrq_n_u32::<31>(t620[0]), vshrq_n_u32::<31>(t620[1])];
    let t622 = b17_r0ls_p(l, lid0, live, arg0, t621, x24);
    x32 = t622;
    let t623: [uint32x4_t; 4];
    if ((x24) >= (8u32)) {
        t623 = x32;
    } else {
        let t624 = b18_s_p(l, lid0, live, (8u32).wrapping_mul(x24));
        let t625: u64 = t624;
        let t626 = [vdupq_n_u32((t625) as u32); 2];
        let t627 = [vdupq_n_u32(((t625) >> 32) as u32); 2];
        let t628: [uint32x4_t; 4] = [vandq_u32(x32[0], t626[0]), vandq_u32(x32[1], t626[1]), vandq_u32(x32[2], t627[0]), vandq_u32(x32[3], t627[1])];
        t623 = t628;
    }
    res = t623;
    return res;
}

#[inline(always)]
unsafe fn b12_r0lls_f(l: &Launch, lid0: u32, arg0: *const u8, x8: [uint32x4_t; 2], x16: [uint32x4_t; 2], x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t629: [uint32x4_t; 2] = [vaddq_u32(x8[0], x16[0]), vaddq_u32(x8[1], x16[1])];
    let t630: [uint32x4_t; 2] = [vcltq_u32(t629[0], x8[0]), vcltq_u32(t629[1], x8[1])];
    let t631: [uint32x4_t; 4] = [t629[0], t629[1], vshrq_n_u32::<31>(t630[0]), vshrq_n_u32::<31>(t630[1])];
    let t632 = b17_r0ls_f(l, lid0, arg0, t631, x24);
    x32 = t632;
    let t633: [uint32x4_t; 4];
    if ((x24) >= (8u32)) {
        t633 = x32;
    } else {
        let t634 = b18_s_f(l, lid0, (8u32).wrapping_mul(x24));
        let t635: u64 = t634;
        let t636 = [vdupq_n_u32((t635) as u32); 2];
        let t637 = [vdupq_n_u32(((t635) >> 32) as u32); 2];
        let t638: [uint32x4_t; 4] = [vandq_u32(x32[0], t636[0]), vandq_u32(x32[1], t636[1]), vandq_u32(x32[2], t637[0]), vandq_u32(x32[3], t637[1])];
        t633 = t638;
    }
    res = t633;
    return res;
}

#[inline(always)]
unsafe fn b13_r0r1sss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: u32, x32: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t639 = b19_r0sss_p(l, lid0, live, arg0, x16, x24, x32);
    res = t639;
    return res;
}

#[inline(always)]
unsafe fn b13_r0r1sss_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32, x24: u32, x32: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t640 = b19_r0sss_f(l, lid0, arg0, x16, x24, x32);
    res = t640;
    return res;
}

#[inline(always)]
unsafe fn b13_r0r1sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2], x32: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t641 = b19_r0sls_p(l, lid0, live, arg0, x16, x24, x32);
    res = t641;
    return res;
}

#[inline(always)]
unsafe fn b14_ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x8: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t642: u32;
    if ((x0) < (x8)) {
        t642 = x0;
    } else {
        t642 = x8;
    }
    res = t642;
    return res;
}

#[inline(always)]
unsafe fn b14_ss_f(l: &Launch, lid0: u32, x0: u32, x8: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t643: u32;
    if ((x0) < (x8)) {
        t643 = x0;
    } else {
        t643 = x8;
    }
    res = t643;
    return res;
}

#[inline(always)]
unsafe fn b14_sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x8: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t644 = [vdupq_n_u32(x0); 2];
    let t645: [uint32x4_t; 2] = [vcltq_u32(t644[0], x8[0]), vcltq_u32(t644[1], x8[1])];
    let mut t646: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t647: [uint32x4_t; 2] = [vandq_u32(live[0], t645[0]), vandq_u32(live[1], t645[1])];
    let t648: [uint32x4_t; 2] = [vbicq_u32(live[0], t645[0]), vbicq_u32(live[1], t645[1])];
    {
        let t649 = [vdupq_n_u32(x0); 2];
        t646 = [vbslq_u32(t647[0], t649[0], t646[0]), vbslq_u32(t647[1], t649[1], t646[1])];
    }
    {
        t646 = [vbslq_u32(t648[0], x8[0], t646[0]), vbslq_u32(t648[1], x8[1], t646[1])];
    }
    res = t646;
    return res;
}

#[inline(always)]
unsafe fn b15_ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x8: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t650: u32;
    if ((x0) < (x8)) {
        t650 = x0;
    } else {
        t650 = x8;
    }
    res = t650;
    return res;
}

#[inline(always)]
unsafe fn b15_ss_f(l: &Launch, lid0: u32, x0: u32, x8: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t651: u32;
    if ((x0) < (x8)) {
        t651 = x0;
    } else {
        t651 = x8;
    }
    res = t651;
    return res;
}

#[inline(always)]
unsafe fn b15_ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x8: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t652 = [vdupq_n_u32(x8); 2];
    let t653: [uint32x4_t; 2] = [vcltq_u32(x0[0], t652[0]), vcltq_u32(x0[1], t652[1])];
    let mut t654: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t655: [uint32x4_t; 2] = [vandq_u32(live[0], t653[0]), vandq_u32(live[1], t653[1])];
    let t656: [uint32x4_t; 2] = [vbicq_u32(live[0], t653[0]), vbicq_u32(live[1], t653[1])];
    {
        t654 = [vbslq_u32(t655[0], x0[0], t654[0]), vbslq_u32(t655[1], x0[1], t654[1])];
    }
    {
        let t657 = [vdupq_n_u32(x8); 2];
        t654 = [vbslq_u32(t656[0], t657[0], t654[0]), vbslq_u32(t656[1], t657[1], t654[1])];
    }
    res = t654;
    return res;
}

#[inline(always)]
unsafe fn b15_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x8: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t658: [uint32x4_t; 2] = [vcltq_u32(x0[0], x8[0]), vcltq_u32(x0[1], x8[1])];
    let mut t659: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t660: [uint32x4_t; 2] = [vandq_u32(live[0], t658[0]), vandq_u32(live[1], t658[1])];
    let t661: [uint32x4_t; 2] = [vbicq_u32(live[0], t658[0]), vbicq_u32(live[1], t658[1])];
    {
        t659 = [vbslq_u32(t660[0], x0[0], t659[0]), vbslq_u32(t660[1], x0[1], t659[1])];
    }
    {
        t659 = [vbslq_u32(t661[0], x8[0], t659[0]), vbslq_u32(t661[1], x8[1], t659[1])];
    }
    res = t659;
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
    let t662: [uint32x4_t; 2] = [vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[0])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[2]))))), vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[1])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[3])))))];
    res = t662;
    return res;
}

#[inline(always)]
unsafe fn b17_r0ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: u64, x16: u32) -> u64 {
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
    let mut t663: bool = ((x32) != (0u32));
    if t663 {
        t663 = (((x32).wrapping_add((x16).wrapping_mul(8u32))) > (64u32));
    }
    let t664: u64;
    if t663 {
        t664 = ((x40) | (((x24.wrapping_add((1u64).wrapping_mul(8u64) as usize) as *const u64).read_unaligned()).wrapping_shl(((((64u32).wrapping_sub(x32)) as u64)) as u32)));
    } else {
        t664 = x40;
    }
    res = t664;
    return res;
}

#[inline(always)]
unsafe fn b17_r0ss_f(l: &Launch, lid0: u32, arg0: *const u8, x8: u64, x16: u32) -> u64 {
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
    let mut t665: bool = ((x32) != (0u32));
    if t665 {
        t665 = (((x32).wrapping_add((x16).wrapping_mul(8u32))) > (64u32));
    }
    let t666: u64;
    if t665 {
        t666 = ((x40) | (((x24.wrapping_add((1u64).wrapping_mul(8u64) as usize) as *const u64).read_unaligned()).wrapping_shl(((((64u32).wrapping_sub(x32)) as u64)) as u32)));
    } else {
        t666 = x40;
    }
    res = t666;
    return res;
}

#[inline(always)]
unsafe fn b17_r0ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: [uint32x4_t; 4], x16: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if ((x16) == (8u32)) {
        let t667: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
        let t668: [uint32x4_t; 4] = [vandq_u32(x8[0], t667[0]), vandq_u32(x8[1], t667[1]), vandq_u32(x8[2], t667[2]), vandq_u32(x8[3], t667[3])];
        let t669: u64 = ((arg0) as usize as u64);
        let t670 = [vdupq_n_u32((t669) as u32); 2];
        let t671 = [vdupq_n_u32(((t669) >> 32) as u32); 2];
        let t672: [uint32x4_t; 2] = [vaddq_u32(t670[0], t668[0]), vaddq_u32(t670[1], t668[1])];
        let t673: [uint32x4_t; 2] = [vcltq_u32(t672[0], t670[0]), vcltq_u32(t672[1], t670[1])];
        let t674: [uint32x4_t; 4] = [t672[0], t672[1], vsubq_u32(vaddq_u32(t671[0], t668[2]), t673[0]), vsubq_u32(vaddq_u32(t671[1], t668[3]), t673[1])];
        let mut t675: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t674[2]) as u64) << 32 | vgetq_lane_u32::<0>(t674[0]) as u64) as usize as *const u64).read_unaligned(); t675[0] = vsetq_lane_u32::<0>(v as u32, t675[0]); t675[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t675[2]); }
        { let v = (((vgetq_lane_u32::<1>(t674[2]) as u64) << 32 | vgetq_lane_u32::<1>(t674[0]) as u64) as usize as *const u64).read_unaligned(); t675[0] = vsetq_lane_u32::<1>(v as u32, t675[0]); t675[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t675[2]); }
        { let v = (((vgetq_lane_u32::<2>(t674[2]) as u64) << 32 | vgetq_lane_u32::<2>(t674[0]) as u64) as usize as *const u64).read_unaligned(); t675[0] = vsetq_lane_u32::<2>(v as u32, t675[0]); t675[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t675[2]); }
        { let v = (((vgetq_lane_u32::<3>(t674[2]) as u64) << 32 | vgetq_lane_u32::<3>(t674[0]) as u64) as usize as *const u64).read_unaligned(); t675[0] = vsetq_lane_u32::<3>(v as u32, t675[0]); t675[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t675[2]); }
        { let v = (((vgetq_lane_u32::<0>(t674[3]) as u64) << 32 | vgetq_lane_u32::<0>(t674[1]) as u64) as usize as *const u64).read_unaligned(); t675[1] = vsetq_lane_u32::<0>(v as u32, t675[1]); t675[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t675[3]); }
        { let v = (((vgetq_lane_u32::<1>(t674[3]) as u64) << 32 | vgetq_lane_u32::<1>(t674[1]) as u64) as usize as *const u64).read_unaligned(); t675[1] = vsetq_lane_u32::<1>(v as u32, t675[1]); t675[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t675[3]); }
        { let v = (((vgetq_lane_u32::<2>(t674[3]) as u64) << 32 | vgetq_lane_u32::<2>(t674[1]) as u64) as usize as *const u64).read_unaligned(); t675[1] = vsetq_lane_u32::<2>(v as u32, t675[1]); t675[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t675[3]); }
        { let v = (((vgetq_lane_u32::<3>(t674[3]) as u64) << 32 | vgetq_lane_u32::<3>(t674[1]) as u64) as usize as *const u64).read_unaligned(); t675[1] = vsetq_lane_u32::<3>(v as u32, t675[1]); t675[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t675[3]); }
        res = t675;
        return res;
    }
    let t676: u64 = !(7u64);
    let t677 = [vdupq_n_u32((t676) as u32); 2];
    let t678 = [vdupq_n_u32(((t676) >> 32) as u32); 2];
    let t679: [uint32x4_t; 4] = [vandq_u32(x8[0], t677[0]), vandq_u32(x8[1], t677[1]), vandq_u32(x8[2], t678[0]), vandq_u32(x8[3], t678[1])];
    let t680: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
    let t681: [uint32x4_t; 4] = [vandq_u32(t679[0], t680[0]), vandq_u32(t679[1], t680[1]), vandq_u32(t679[2], t680[2]), vandq_u32(t679[3], t680[3])];
    let t682: u64 = ((arg0) as usize as u64);
    let t683 = [vdupq_n_u32((t682) as u32); 2];
    let t684 = [vdupq_n_u32(((t682) >> 32) as u32); 2];
    let t685: [uint32x4_t; 2] = [vaddq_u32(t683[0], t681[0]), vaddq_u32(t683[1], t681[1])];
    let t686: [uint32x4_t; 2] = [vcltq_u32(t685[0], t683[0]), vcltq_u32(t685[1], t683[1])];
    let t687: [uint32x4_t; 4] = [t685[0], t685[1], vsubq_u32(vaddq_u32(t684[0], t681[2]), t686[0]), vsubq_u32(vaddq_u32(t684[1], t681[3]), t686[1])];
    x24 = t687;
    let t688: u64 = 7u64;
    let t689 = [vdupq_n_u32((t688) as u32); 2];
    let t690: [uint32x4_t; 2] = [vandq_u32(x8[0], t689[0]), vandq_u32(x8[1], t689[1])];
    let t691 = [vdupq_n_u32(8u32); 2];
    let t692: [uint32x4_t; 2] = [vmulq_u32(t690[0], t691[0]), vmulq_u32(t690[1], t691[1])];
    x32 = t692;
    let mut t693: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x24[2]) as u64) << 32 | vgetq_lane_u32::<0>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t693[0] = vsetq_lane_u32::<0>(v as u32, t693[0]); t693[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t693[2]); }
    { let v = (((vgetq_lane_u32::<1>(x24[2]) as u64) << 32 | vgetq_lane_u32::<1>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t693[0] = vsetq_lane_u32::<1>(v as u32, t693[0]); t693[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t693[2]); }
    { let v = (((vgetq_lane_u32::<2>(x24[2]) as u64) << 32 | vgetq_lane_u32::<2>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t693[0] = vsetq_lane_u32::<2>(v as u32, t693[0]); t693[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t693[2]); }
    { let v = (((vgetq_lane_u32::<3>(x24[2]) as u64) << 32 | vgetq_lane_u32::<3>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t693[0] = vsetq_lane_u32::<3>(v as u32, t693[0]); t693[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t693[2]); }
    { let v = (((vgetq_lane_u32::<0>(x24[3]) as u64) << 32 | vgetq_lane_u32::<0>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t693[1] = vsetq_lane_u32::<0>(v as u32, t693[1]); t693[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t693[3]); }
    { let v = (((vgetq_lane_u32::<1>(x24[3]) as u64) << 32 | vgetq_lane_u32::<1>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t693[1] = vsetq_lane_u32::<1>(v as u32, t693[1]); t693[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t693[3]); }
    { let v = (((vgetq_lane_u32::<2>(x24[3]) as u64) << 32 | vgetq_lane_u32::<2>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t693[1] = vsetq_lane_u32::<2>(v as u32, t693[1]); t693[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t693[3]); }
    { let v = (((vgetq_lane_u32::<3>(x24[3]) as u64) << 32 | vgetq_lane_u32::<3>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t693[1] = vsetq_lane_u32::<3>(v as u32, t693[1]); t693[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t693[3]); }
    let t694: [uint32x4_t; 2] = [x32[0], x32[1]];
    let t695: [uint32x4_t; 2] = [vandq_u32(t694[0], vdupq_n_u32(63)), vandq_u32(t694[1], vdupq_n_u32(63))];
    let t696: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t693[0], vnegq_s32(vreinterpretq_s32_u32(t695[0]))), vshlq_u32(t693[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t695[0])))), vorrq_u32(vshlq_u32(t693[1], vnegq_s32(vreinterpretq_s32_u32(t695[1]))), vshlq_u32(t693[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t695[1])))), vshlq_u32(t693[2], vnegq_s32(vreinterpretq_s32_u32(t695[0]))), vshlq_u32(t693[3], vnegq_s32(vreinterpretq_s32_u32(t695[1])))];
    x40 = t696;
    let t697 = [vdupq_n_u32(0u32); 2];
    let t698: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t697[0])), vmvnq_u32(vceqq_u32(x32[1], t697[1]))];
    let t699 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t700: [uint32x4_t; 2] = [vaddq_u32(x32[0], t699[0]), vaddq_u32(x32[1], t699[1])];
    let t701 = [vdupq_n_u32(64u32); 2];
    let t702: [uint32x4_t; 2] = [vcgtq_u32(t700[0], t701[0]), vcgtq_u32(t700[1], t701[1])];
    let t703: [uint32x4_t; 2] = [vandq_u32(t698[0], t702[0]), vandq_u32(t698[1], t702[1])];
    let mut t704: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t705: [uint32x4_t; 2] = [vandq_u32(live[0], t703[0]), vandq_u32(live[1], t703[1])];
    let t706: [uint32x4_t; 2] = [vbicq_u32(live[0], t703[0]), vbicq_u32(live[1], t703[1])];
    if any(&t705) {
        let t707: u64 = (1u64).wrapping_mul(8u64);
        let t708 = [vdupq_n_u32((t707) as u32); 2];
        let t709 = [vdupq_n_u32(((t707) >> 32) as u32); 2];
        let t710: [uint32x4_t; 2] = [vaddq_u32(x24[0], t708[0]), vaddq_u32(x24[1], t708[1])];
        let t711: [uint32x4_t; 2] = [vcltq_u32(t710[0], x24[0]), vcltq_u32(t710[1], x24[1])];
        let t712: [uint32x4_t; 4] = [t710[0], t710[1], vsubq_u32(vaddq_u32(x24[2], t709[0]), t711[0]), vsubq_u32(vaddq_u32(x24[3], t709[1]), t711[1])];
        let mut t713: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t712[2]) as u64) << 32 | vgetq_lane_u32::<0>(t712[0]) as u64) as usize as *const u64).read_unaligned(); t713[0] = vsetq_lane_u32::<0>(v as u32, t713[0]); t713[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t713[2]); }
        { let v = (((vgetq_lane_u32::<1>(t712[2]) as u64) << 32 | vgetq_lane_u32::<1>(t712[0]) as u64) as usize as *const u64).read_unaligned(); t713[0] = vsetq_lane_u32::<1>(v as u32, t713[0]); t713[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t713[2]); }
        { let v = (((vgetq_lane_u32::<2>(t712[2]) as u64) << 32 | vgetq_lane_u32::<2>(t712[0]) as u64) as usize as *const u64).read_unaligned(); t713[0] = vsetq_lane_u32::<2>(v as u32, t713[0]); t713[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t713[2]); }
        { let v = (((vgetq_lane_u32::<3>(t712[2]) as u64) << 32 | vgetq_lane_u32::<3>(t712[0]) as u64) as usize as *const u64).read_unaligned(); t713[0] = vsetq_lane_u32::<3>(v as u32, t713[0]); t713[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t713[2]); }
        { let v = (((vgetq_lane_u32::<0>(t712[3]) as u64) << 32 | vgetq_lane_u32::<0>(t712[1]) as u64) as usize as *const u64).read_unaligned(); t713[1] = vsetq_lane_u32::<0>(v as u32, t713[1]); t713[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t713[3]); }
        { let v = (((vgetq_lane_u32::<1>(t712[3]) as u64) << 32 | vgetq_lane_u32::<1>(t712[1]) as u64) as usize as *const u64).read_unaligned(); t713[1] = vsetq_lane_u32::<1>(v as u32, t713[1]); t713[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t713[3]); }
        { let v = (((vgetq_lane_u32::<2>(t712[3]) as u64) << 32 | vgetq_lane_u32::<2>(t712[1]) as u64) as usize as *const u64).read_unaligned(); t713[1] = vsetq_lane_u32::<2>(v as u32, t713[1]); t713[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t713[3]); }
        { let v = (((vgetq_lane_u32::<3>(t712[3]) as u64) << 32 | vgetq_lane_u32::<3>(t712[1]) as u64) as usize as *const u64).read_unaligned(); t713[1] = vsetq_lane_u32::<3>(v as u32, t713[1]); t713[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t713[3]); }
        let t714: [uint32x4_t; 2] = [vsubq_u32(t701[0], x32[0]), vsubq_u32(t701[1], x32[1])];
        let t715: [uint32x4_t; 2] = [t714[0], t714[1]];
        let t716: [uint32x4_t; 2] = [vandq_u32(t715[0], vdupq_n_u32(63)), vandq_u32(t715[1], vdupq_n_u32(63))];
        let t717: [uint32x4_t; 4] = [vshlq_u32(t713[0], vreinterpretq_s32_u32(t716[0])), vshlq_u32(t713[1], vreinterpretq_s32_u32(t716[1])), vorrq_u32(vshlq_u32(t713[2], vreinterpretq_s32_u32(t716[0])), vshlq_u32(t713[0], vsubq_s32(vreinterpretq_s32_u32(t716[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t713[3], vreinterpretq_s32_u32(t716[1])), vshlq_u32(t713[1], vsubq_s32(vreinterpretq_s32_u32(t716[1]), vdupq_n_s32(32))))];
        let t718: [uint32x4_t; 4] = [vorrq_u32(x40[0], t717[0]), vorrq_u32(x40[1], t717[1]), vorrq_u32(x40[2], t717[2]), vorrq_u32(x40[3], t717[3])];
        t704 = [vbslq_u32(t705[0], t718[0], t704[0]), vbslq_u32(t705[1], t718[1], t704[1]), vbslq_u32(t705[0], t718[2], t704[2]), vbslq_u32(t705[1], t718[3], t704[3])];
    }
    {
        t704 = [vbslq_u32(t706[0], x40[0], t704[0]), vbslq_u32(t706[1], x40[1], t704[1]), vbslq_u32(t706[0], x40[2], t704[2]), vbslq_u32(t706[1], x40[3], t704[3])];
    }
    res = t704;
    return res;
}

#[inline(always)]
unsafe fn b17_r0ls_f(l: &Launch, lid0: u32, arg0: *const u8, x8: [uint32x4_t; 4], x16: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if ((x16) == (8u32)) {
        let t719: u64 = ((arg0) as usize as u64);
        let t720 = [vdupq_n_u32((t719) as u32); 2];
        let t721 = [vdupq_n_u32(((t719) >> 32) as u32); 2];
        let t722: [uint32x4_t; 2] = [vaddq_u32(t720[0], x8[0]), vaddq_u32(t720[1], x8[1])];
        let t723: [uint32x4_t; 2] = [vcltq_u32(t722[0], t720[0]), vcltq_u32(t722[1], t720[1])];
        let t724: [uint32x4_t; 4] = [t722[0], t722[1], vsubq_u32(vaddq_u32(t721[0], x8[2]), t723[0]), vsubq_u32(vaddq_u32(t721[1], x8[3]), t723[1])];
        let mut t725: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t724[2]) as u64) << 32 | vgetq_lane_u32::<0>(t724[0]) as u64) as usize as *const u64).read_unaligned(); t725[0] = vsetq_lane_u32::<0>(v as u32, t725[0]); t725[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t725[2]); }
        { let v = (((vgetq_lane_u32::<1>(t724[2]) as u64) << 32 | vgetq_lane_u32::<1>(t724[0]) as u64) as usize as *const u64).read_unaligned(); t725[0] = vsetq_lane_u32::<1>(v as u32, t725[0]); t725[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t725[2]); }
        { let v = (((vgetq_lane_u32::<2>(t724[2]) as u64) << 32 | vgetq_lane_u32::<2>(t724[0]) as u64) as usize as *const u64).read_unaligned(); t725[0] = vsetq_lane_u32::<2>(v as u32, t725[0]); t725[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t725[2]); }
        { let v = (((vgetq_lane_u32::<3>(t724[2]) as u64) << 32 | vgetq_lane_u32::<3>(t724[0]) as u64) as usize as *const u64).read_unaligned(); t725[0] = vsetq_lane_u32::<3>(v as u32, t725[0]); t725[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t725[2]); }
        { let v = (((vgetq_lane_u32::<0>(t724[3]) as u64) << 32 | vgetq_lane_u32::<0>(t724[1]) as u64) as usize as *const u64).read_unaligned(); t725[1] = vsetq_lane_u32::<0>(v as u32, t725[1]); t725[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t725[3]); }
        { let v = (((vgetq_lane_u32::<1>(t724[3]) as u64) << 32 | vgetq_lane_u32::<1>(t724[1]) as u64) as usize as *const u64).read_unaligned(); t725[1] = vsetq_lane_u32::<1>(v as u32, t725[1]); t725[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t725[3]); }
        { let v = (((vgetq_lane_u32::<2>(t724[3]) as u64) << 32 | vgetq_lane_u32::<2>(t724[1]) as u64) as usize as *const u64).read_unaligned(); t725[1] = vsetq_lane_u32::<2>(v as u32, t725[1]); t725[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t725[3]); }
        { let v = (((vgetq_lane_u32::<3>(t724[3]) as u64) << 32 | vgetq_lane_u32::<3>(t724[1]) as u64) as usize as *const u64).read_unaligned(); t725[1] = vsetq_lane_u32::<3>(v as u32, t725[1]); t725[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t725[3]); }
        res = t725;
        return res;
    }
    let t726: u64 = !(7u64);
    let t727 = [vdupq_n_u32((t726) as u32); 2];
    let t728 = [vdupq_n_u32(((t726) >> 32) as u32); 2];
    let t729: [uint32x4_t; 4] = [vandq_u32(x8[0], t727[0]), vandq_u32(x8[1], t727[1]), vandq_u32(x8[2], t728[0]), vandq_u32(x8[3], t728[1])];
    let t730: u64 = ((arg0) as usize as u64);
    let t731 = [vdupq_n_u32((t730) as u32); 2];
    let t732 = [vdupq_n_u32(((t730) >> 32) as u32); 2];
    let t733: [uint32x4_t; 2] = [vaddq_u32(t731[0], t729[0]), vaddq_u32(t731[1], t729[1])];
    let t734: [uint32x4_t; 2] = [vcltq_u32(t733[0], t731[0]), vcltq_u32(t733[1], t731[1])];
    let t735: [uint32x4_t; 4] = [t733[0], t733[1], vsubq_u32(vaddq_u32(t732[0], t729[2]), t734[0]), vsubq_u32(vaddq_u32(t732[1], t729[3]), t734[1])];
    x24 = t735;
    let t736: u64 = 7u64;
    let t737 = [vdupq_n_u32((t736) as u32); 2];
    let t738: [uint32x4_t; 2] = [vandq_u32(x8[0], t737[0]), vandq_u32(x8[1], t737[1])];
    let t739 = [vdupq_n_u32(8u32); 2];
    let t740: [uint32x4_t; 2] = [vmulq_u32(t738[0], t739[0]), vmulq_u32(t738[1], t739[1])];
    x32 = t740;
    let mut t741: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x24[2]) as u64) << 32 | vgetq_lane_u32::<0>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t741[0] = vsetq_lane_u32::<0>(v as u32, t741[0]); t741[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t741[2]); }
    { let v = (((vgetq_lane_u32::<1>(x24[2]) as u64) << 32 | vgetq_lane_u32::<1>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t741[0] = vsetq_lane_u32::<1>(v as u32, t741[0]); t741[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t741[2]); }
    { let v = (((vgetq_lane_u32::<2>(x24[2]) as u64) << 32 | vgetq_lane_u32::<2>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t741[0] = vsetq_lane_u32::<2>(v as u32, t741[0]); t741[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t741[2]); }
    { let v = (((vgetq_lane_u32::<3>(x24[2]) as u64) << 32 | vgetq_lane_u32::<3>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t741[0] = vsetq_lane_u32::<3>(v as u32, t741[0]); t741[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t741[2]); }
    { let v = (((vgetq_lane_u32::<0>(x24[3]) as u64) << 32 | vgetq_lane_u32::<0>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t741[1] = vsetq_lane_u32::<0>(v as u32, t741[1]); t741[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t741[3]); }
    { let v = (((vgetq_lane_u32::<1>(x24[3]) as u64) << 32 | vgetq_lane_u32::<1>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t741[1] = vsetq_lane_u32::<1>(v as u32, t741[1]); t741[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t741[3]); }
    { let v = (((vgetq_lane_u32::<2>(x24[3]) as u64) << 32 | vgetq_lane_u32::<2>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t741[1] = vsetq_lane_u32::<2>(v as u32, t741[1]); t741[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t741[3]); }
    { let v = (((vgetq_lane_u32::<3>(x24[3]) as u64) << 32 | vgetq_lane_u32::<3>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t741[1] = vsetq_lane_u32::<3>(v as u32, t741[1]); t741[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t741[3]); }
    let t742: [uint32x4_t; 2] = [x32[0], x32[1]];
    let t743: [uint32x4_t; 2] = [vandq_u32(t742[0], vdupq_n_u32(63)), vandq_u32(t742[1], vdupq_n_u32(63))];
    let t744: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t741[0], vnegq_s32(vreinterpretq_s32_u32(t743[0]))), vshlq_u32(t741[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t743[0])))), vorrq_u32(vshlq_u32(t741[1], vnegq_s32(vreinterpretq_s32_u32(t743[1]))), vshlq_u32(t741[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t743[1])))), vshlq_u32(t741[2], vnegq_s32(vreinterpretq_s32_u32(t743[0]))), vshlq_u32(t741[3], vnegq_s32(vreinterpretq_s32_u32(t743[1])))];
    x40 = t744;
    let t745 = [vdupq_n_u32(0u32); 2];
    let t746: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t745[0])), vmvnq_u32(vceqq_u32(x32[1], t745[1]))];
    let t747 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t748: [uint32x4_t; 2] = [vaddq_u32(x32[0], t747[0]), vaddq_u32(x32[1], t747[1])];
    let t749 = [vdupq_n_u32(64u32); 2];
    let t750: [uint32x4_t; 2] = [vcgtq_u32(t748[0], t749[0]), vcgtq_u32(t748[1], t749[1])];
    let t751: [uint32x4_t; 2] = [vandq_u32(t746[0], t750[0]), vandq_u32(t746[1], t750[1])];
    let mut t752: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t753: [uint32x4_t; 2] = [vmvnq_u32(t751[0]), vmvnq_u32(t751[1])];
    if any(&t751) {
        let t754: u64 = (1u64).wrapping_mul(8u64);
        let t755 = [vdupq_n_u32((t754) as u32); 2];
        let t756 = [vdupq_n_u32(((t754) >> 32) as u32); 2];
        let t757: [uint32x4_t; 2] = [vaddq_u32(x24[0], t755[0]), vaddq_u32(x24[1], t755[1])];
        let t758: [uint32x4_t; 2] = [vcltq_u32(t757[0], x24[0]), vcltq_u32(t757[1], x24[1])];
        let t759: [uint32x4_t; 4] = [t757[0], t757[1], vsubq_u32(vaddq_u32(x24[2], t756[0]), t758[0]), vsubq_u32(vaddq_u32(x24[3], t756[1]), t758[1])];
        let mut t760: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t759[2]) as u64) << 32 | vgetq_lane_u32::<0>(t759[0]) as u64) as usize as *const u64).read_unaligned(); t760[0] = vsetq_lane_u32::<0>(v as u32, t760[0]); t760[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t760[2]); }
        { let v = (((vgetq_lane_u32::<1>(t759[2]) as u64) << 32 | vgetq_lane_u32::<1>(t759[0]) as u64) as usize as *const u64).read_unaligned(); t760[0] = vsetq_lane_u32::<1>(v as u32, t760[0]); t760[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t760[2]); }
        { let v = (((vgetq_lane_u32::<2>(t759[2]) as u64) << 32 | vgetq_lane_u32::<2>(t759[0]) as u64) as usize as *const u64).read_unaligned(); t760[0] = vsetq_lane_u32::<2>(v as u32, t760[0]); t760[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t760[2]); }
        { let v = (((vgetq_lane_u32::<3>(t759[2]) as u64) << 32 | vgetq_lane_u32::<3>(t759[0]) as u64) as usize as *const u64).read_unaligned(); t760[0] = vsetq_lane_u32::<3>(v as u32, t760[0]); t760[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t760[2]); }
        { let v = (((vgetq_lane_u32::<0>(t759[3]) as u64) << 32 | vgetq_lane_u32::<0>(t759[1]) as u64) as usize as *const u64).read_unaligned(); t760[1] = vsetq_lane_u32::<0>(v as u32, t760[1]); t760[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t760[3]); }
        { let v = (((vgetq_lane_u32::<1>(t759[3]) as u64) << 32 | vgetq_lane_u32::<1>(t759[1]) as u64) as usize as *const u64).read_unaligned(); t760[1] = vsetq_lane_u32::<1>(v as u32, t760[1]); t760[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t760[3]); }
        { let v = (((vgetq_lane_u32::<2>(t759[3]) as u64) << 32 | vgetq_lane_u32::<2>(t759[1]) as u64) as usize as *const u64).read_unaligned(); t760[1] = vsetq_lane_u32::<2>(v as u32, t760[1]); t760[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t760[3]); }
        { let v = (((vgetq_lane_u32::<3>(t759[3]) as u64) << 32 | vgetq_lane_u32::<3>(t759[1]) as u64) as usize as *const u64).read_unaligned(); t760[1] = vsetq_lane_u32::<3>(v as u32, t760[1]); t760[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t760[3]); }
        let t761: [uint32x4_t; 2] = [vsubq_u32(t749[0], x32[0]), vsubq_u32(t749[1], x32[1])];
        let t762: [uint32x4_t; 2] = [t761[0], t761[1]];
        let t763: [uint32x4_t; 2] = [vandq_u32(t762[0], vdupq_n_u32(63)), vandq_u32(t762[1], vdupq_n_u32(63))];
        let t764: [uint32x4_t; 4] = [vshlq_u32(t760[0], vreinterpretq_s32_u32(t763[0])), vshlq_u32(t760[1], vreinterpretq_s32_u32(t763[1])), vorrq_u32(vshlq_u32(t760[2], vreinterpretq_s32_u32(t763[0])), vshlq_u32(t760[0], vsubq_s32(vreinterpretq_s32_u32(t763[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t760[3], vreinterpretq_s32_u32(t763[1])), vshlq_u32(t760[1], vsubq_s32(vreinterpretq_s32_u32(t763[1]), vdupq_n_s32(32))))];
        let t765: [uint32x4_t; 4] = [vorrq_u32(x40[0], t764[0]), vorrq_u32(x40[1], t764[1]), vorrq_u32(x40[2], t764[2]), vorrq_u32(x40[3], t764[3])];
        t752 = [vbslq_u32(t751[0], t765[0], t752[0]), vbslq_u32(t751[1], t765[1], t752[1]), vbslq_u32(t751[0], t765[2], t752[2]), vbslq_u32(t751[1], t765[3], t752[3])];
    }
    {
        t752 = [vbslq_u32(t753[0], x40[0], t752[0]), vbslq_u32(t753[1], x40[1], t752[1]), vbslq_u32(t753[0], x40[2], t752[2]), vbslq_u32(t753[1], x40[3], t752[3])];
    }
    res = t752;
    return res;
}

#[inline(always)]
unsafe fn b17_r0al1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8b: *const u8, x8i: [uint32x4_t; 2], x16: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if ((x16) == (8u32)) {
        let t766 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], x8i[0]), vandq_u32(live[1], x8i[1]))));
        let t767: [uint32x4_t; 2] = [vbslq_u32(live[0], x8i[0], t766), vbslq_u32(live[1], x8i[1], t766)];
        let mut t768: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t767[0]) as usize * 1) as *const u64).read_unaligned(); t768[0] = vsetq_lane_u32::<0>(v as u32, t768[0]); t768[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t768[2]); }
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t767[0]) as usize * 1) as *const u64).read_unaligned(); t768[0] = vsetq_lane_u32::<1>(v as u32, t768[0]); t768[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t768[2]); }
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t767[0]) as usize * 1) as *const u64).read_unaligned(); t768[0] = vsetq_lane_u32::<2>(v as u32, t768[0]); t768[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t768[2]); }
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t767[0]) as usize * 1) as *const u64).read_unaligned(); t768[0] = vsetq_lane_u32::<3>(v as u32, t768[0]); t768[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t768[2]); }
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t767[1]) as usize * 1) as *const u64).read_unaligned(); t768[1] = vsetq_lane_u32::<0>(v as u32, t768[1]); t768[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t768[3]); }
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t767[1]) as usize * 1) as *const u64).read_unaligned(); t768[1] = vsetq_lane_u32::<1>(v as u32, t768[1]); t768[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t768[3]); }
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t767[1]) as usize * 1) as *const u64).read_unaligned(); t768[1] = vsetq_lane_u32::<2>(v as u32, t768[1]); t768[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t768[3]); }
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t767[1]) as usize * 1) as *const u64).read_unaligned(); t768[1] = vsetq_lane_u32::<3>(v as u32, t768[1]); t768[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t768[3]); }
        res = t768;
        return res;
    }
    let t769: [uint32x4_t; 2] = [x8i[0], x8i[1]];
    let t770: u64 = ((x8b) as usize as u64);
    let t771 = [vdupq_n_u32((t770) as u32); 2];
    let t772 = [vdupq_n_u32(((t770) >> 32) as u32); 2];
    let t773: [uint32x4_t; 2] = [vaddq_u32(t771[0], t769[0]), vaddq_u32(t771[1], t769[1])];
    let t774: [uint32x4_t; 2] = [vcltq_u32(t773[0], t771[0]), vcltq_u32(t773[1], t771[1])];
    let t775: [uint32x4_t; 4] = [t773[0], t773[1], vsubq_u32(vaddq_u32(t772[0], vdupq_n_u32(0)), t774[0]), vsubq_u32(vaddq_u32(t772[1], vdupq_n_u32(0)), t774[1])];
    let t776: u64 = !(7u64);
    let t777 = [vdupq_n_u32((t776) as u32); 2];
    let t778 = [vdupq_n_u32(((t776) >> 32) as u32); 2];
    let t779: [uint32x4_t; 4] = [vandq_u32(t775[0], t777[0]), vandq_u32(t775[1], t777[1]), vandq_u32(t775[2], t778[0]), vandq_u32(t775[3], t778[1])];
    let t780: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
    let t781: [uint32x4_t; 4] = [vandq_u32(t779[0], t780[0]), vandq_u32(t779[1], t780[1]), vandq_u32(t779[2], t780[2]), vandq_u32(t779[3], t780[3])];
    let t782: u64 = ((arg0) as usize as u64);
    let t783 = [vdupq_n_u32((t782) as u32); 2];
    let t784 = [vdupq_n_u32(((t782) >> 32) as u32); 2];
    let t785: [uint32x4_t; 2] = [vaddq_u32(t783[0], t781[0]), vaddq_u32(t783[1], t781[1])];
    let t786: [uint32x4_t; 2] = [vcltq_u32(t785[0], t783[0]), vcltq_u32(t785[1], t783[1])];
    let t787: [uint32x4_t; 4] = [t785[0], t785[1], vsubq_u32(vaddq_u32(t784[0], t781[2]), t786[0]), vsubq_u32(vaddq_u32(t784[1], t781[3]), t786[1])];
    x24 = t787;
    let t788: [uint32x4_t; 2] = [x8i[0], x8i[1]];
    let t789: u64 = ((x8b) as usize as u64);
    let t790 = [vdupq_n_u32((t789) as u32); 2];
    let t791 = [vdupq_n_u32(((t789) >> 32) as u32); 2];
    let t792: [uint32x4_t; 2] = [vaddq_u32(t790[0], t788[0]), vaddq_u32(t790[1], t788[1])];
    let t793: [uint32x4_t; 2] = [vcltq_u32(t792[0], t790[0]), vcltq_u32(t792[1], t790[1])];
    let t794: [uint32x4_t; 4] = [t792[0], t792[1], vsubq_u32(vaddq_u32(t791[0], vdupq_n_u32(0)), t793[0]), vsubq_u32(vaddq_u32(t791[1], vdupq_n_u32(0)), t793[1])];
    let t795: u64 = 7u64;
    let t796 = [vdupq_n_u32((t795) as u32); 2];
    let t797: [uint32x4_t; 2] = [vandq_u32(t794[0], t796[0]), vandq_u32(t794[1], t796[1])];
    let t798 = [vdupq_n_u32(8u32); 2];
    let t799: [uint32x4_t; 2] = [vmulq_u32(t797[0], t798[0]), vmulq_u32(t797[1], t798[1])];
    x32 = t799;
    let mut t800: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x24[2]) as u64) << 32 | vgetq_lane_u32::<0>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t800[0] = vsetq_lane_u32::<0>(v as u32, t800[0]); t800[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t800[2]); }
    { let v = (((vgetq_lane_u32::<1>(x24[2]) as u64) << 32 | vgetq_lane_u32::<1>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t800[0] = vsetq_lane_u32::<1>(v as u32, t800[0]); t800[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t800[2]); }
    { let v = (((vgetq_lane_u32::<2>(x24[2]) as u64) << 32 | vgetq_lane_u32::<2>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t800[0] = vsetq_lane_u32::<2>(v as u32, t800[0]); t800[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t800[2]); }
    { let v = (((vgetq_lane_u32::<3>(x24[2]) as u64) << 32 | vgetq_lane_u32::<3>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t800[0] = vsetq_lane_u32::<3>(v as u32, t800[0]); t800[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t800[2]); }
    { let v = (((vgetq_lane_u32::<0>(x24[3]) as u64) << 32 | vgetq_lane_u32::<0>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t800[1] = vsetq_lane_u32::<0>(v as u32, t800[1]); t800[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t800[3]); }
    { let v = (((vgetq_lane_u32::<1>(x24[3]) as u64) << 32 | vgetq_lane_u32::<1>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t800[1] = vsetq_lane_u32::<1>(v as u32, t800[1]); t800[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t800[3]); }
    { let v = (((vgetq_lane_u32::<2>(x24[3]) as u64) << 32 | vgetq_lane_u32::<2>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t800[1] = vsetq_lane_u32::<2>(v as u32, t800[1]); t800[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t800[3]); }
    { let v = (((vgetq_lane_u32::<3>(x24[3]) as u64) << 32 | vgetq_lane_u32::<3>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t800[1] = vsetq_lane_u32::<3>(v as u32, t800[1]); t800[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t800[3]); }
    let t801: [uint32x4_t; 2] = [x32[0], x32[1]];
    let t802: [uint32x4_t; 2] = [vandq_u32(t801[0], vdupq_n_u32(63)), vandq_u32(t801[1], vdupq_n_u32(63))];
    let t803: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t800[0], vnegq_s32(vreinterpretq_s32_u32(t802[0]))), vshlq_u32(t800[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t802[0])))), vorrq_u32(vshlq_u32(t800[1], vnegq_s32(vreinterpretq_s32_u32(t802[1]))), vshlq_u32(t800[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t802[1])))), vshlq_u32(t800[2], vnegq_s32(vreinterpretq_s32_u32(t802[0]))), vshlq_u32(t800[3], vnegq_s32(vreinterpretq_s32_u32(t802[1])))];
    x40 = t803;
    let t804 = [vdupq_n_u32(0u32); 2];
    let t805: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t804[0])), vmvnq_u32(vceqq_u32(x32[1], t804[1]))];
    let t806 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t807: [uint32x4_t; 2] = [vaddq_u32(x32[0], t806[0]), vaddq_u32(x32[1], t806[1])];
    let t808 = [vdupq_n_u32(64u32); 2];
    let t809: [uint32x4_t; 2] = [vcgtq_u32(t807[0], t808[0]), vcgtq_u32(t807[1], t808[1])];
    let t810: [uint32x4_t; 2] = [vandq_u32(t805[0], t809[0]), vandq_u32(t805[1], t809[1])];
    let mut t811: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t812: [uint32x4_t; 2] = [vandq_u32(live[0], t810[0]), vandq_u32(live[1], t810[1])];
    let t813: [uint32x4_t; 2] = [vbicq_u32(live[0], t810[0]), vbicq_u32(live[1], t810[1])];
    if any(&t812) {
        let t814: u64 = (1u64).wrapping_mul(8u64);
        let t815 = [vdupq_n_u32((t814) as u32); 2];
        let t816 = [vdupq_n_u32(((t814) >> 32) as u32); 2];
        let t817: [uint32x4_t; 2] = [vaddq_u32(x24[0], t815[0]), vaddq_u32(x24[1], t815[1])];
        let t818: [uint32x4_t; 2] = [vcltq_u32(t817[0], x24[0]), vcltq_u32(t817[1], x24[1])];
        let t819: [uint32x4_t; 4] = [t817[0], t817[1], vsubq_u32(vaddq_u32(x24[2], t816[0]), t818[0]), vsubq_u32(vaddq_u32(x24[3], t816[1]), t818[1])];
        let mut t820: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t819[2]) as u64) << 32 | vgetq_lane_u32::<0>(t819[0]) as u64) as usize as *const u64).read_unaligned(); t820[0] = vsetq_lane_u32::<0>(v as u32, t820[0]); t820[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t820[2]); }
        { let v = (((vgetq_lane_u32::<1>(t819[2]) as u64) << 32 | vgetq_lane_u32::<1>(t819[0]) as u64) as usize as *const u64).read_unaligned(); t820[0] = vsetq_lane_u32::<1>(v as u32, t820[0]); t820[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t820[2]); }
        { let v = (((vgetq_lane_u32::<2>(t819[2]) as u64) << 32 | vgetq_lane_u32::<2>(t819[0]) as u64) as usize as *const u64).read_unaligned(); t820[0] = vsetq_lane_u32::<2>(v as u32, t820[0]); t820[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t820[2]); }
        { let v = (((vgetq_lane_u32::<3>(t819[2]) as u64) << 32 | vgetq_lane_u32::<3>(t819[0]) as u64) as usize as *const u64).read_unaligned(); t820[0] = vsetq_lane_u32::<3>(v as u32, t820[0]); t820[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t820[2]); }
        { let v = (((vgetq_lane_u32::<0>(t819[3]) as u64) << 32 | vgetq_lane_u32::<0>(t819[1]) as u64) as usize as *const u64).read_unaligned(); t820[1] = vsetq_lane_u32::<0>(v as u32, t820[1]); t820[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t820[3]); }
        { let v = (((vgetq_lane_u32::<1>(t819[3]) as u64) << 32 | vgetq_lane_u32::<1>(t819[1]) as u64) as usize as *const u64).read_unaligned(); t820[1] = vsetq_lane_u32::<1>(v as u32, t820[1]); t820[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t820[3]); }
        { let v = (((vgetq_lane_u32::<2>(t819[3]) as u64) << 32 | vgetq_lane_u32::<2>(t819[1]) as u64) as usize as *const u64).read_unaligned(); t820[1] = vsetq_lane_u32::<2>(v as u32, t820[1]); t820[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t820[3]); }
        { let v = (((vgetq_lane_u32::<3>(t819[3]) as u64) << 32 | vgetq_lane_u32::<3>(t819[1]) as u64) as usize as *const u64).read_unaligned(); t820[1] = vsetq_lane_u32::<3>(v as u32, t820[1]); t820[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t820[3]); }
        let t821: [uint32x4_t; 2] = [vsubq_u32(t808[0], x32[0]), vsubq_u32(t808[1], x32[1])];
        let t822: [uint32x4_t; 2] = [t821[0], t821[1]];
        let t823: [uint32x4_t; 2] = [vandq_u32(t822[0], vdupq_n_u32(63)), vandq_u32(t822[1], vdupq_n_u32(63))];
        let t824: [uint32x4_t; 4] = [vshlq_u32(t820[0], vreinterpretq_s32_u32(t823[0])), vshlq_u32(t820[1], vreinterpretq_s32_u32(t823[1])), vorrq_u32(vshlq_u32(t820[2], vreinterpretq_s32_u32(t823[0])), vshlq_u32(t820[0], vsubq_s32(vreinterpretq_s32_u32(t823[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t820[3], vreinterpretq_s32_u32(t823[1])), vshlq_u32(t820[1], vsubq_s32(vreinterpretq_s32_u32(t823[1]), vdupq_n_s32(32))))];
        let t825: [uint32x4_t; 4] = [vorrq_u32(x40[0], t824[0]), vorrq_u32(x40[1], t824[1]), vorrq_u32(x40[2], t824[2]), vorrq_u32(x40[3], t824[3])];
        t811 = [vbslq_u32(t812[0], t825[0], t811[0]), vbslq_u32(t812[1], t825[1], t811[1]), vbslq_u32(t812[0], t825[2], t811[2]), vbslq_u32(t812[1], t825[3], t811[3])];
    }
    {
        t811 = [vbslq_u32(t813[0], x40[0], t811[0]), vbslq_u32(t813[1], x40[1], t811[1]), vbslq_u32(t813[0], x40[2], t811[2]), vbslq_u32(t813[1], x40[3], t811[3])];
    }
    res = t811;
    return res;
}

#[inline(always)]
unsafe fn b17_r0an1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8b: *const u8, x8if: u32, x8is: u32, x16: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if ((x16) == (8u32)) {
        let t826: [uint32x4_t; 4];
        if (x8is) == 8u32 && ((x8if) as u64) + 7 * ((x8is) as u64) <= u32::MAX as u64 && all(&live) {
            let t827 = ((arg0).wrapping_add((x8b) as usize) as *const u8).add(((x8if) as usize) * 1);
            let t8280 = vld2q_u32((t827 as *const u32).add(0));
            let t8281 = vld2q_u32((t827 as *const u32).add(8));
            let t828: [uint32x4_t; 4] = [t8280.0, t8281.0, t8280.1, t8281.1];
            t826 = t828;
        } else {
            let t829: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8is)), vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8is))];
            let t830 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t829[0]), vandq_u32(live[1], t829[1]))));
            let t831: [uint32x4_t; 2] = [vbslq_u32(live[0], t829[0], t830), vbslq_u32(live[1], t829[1], t830)];
            let mut t832: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t831[0]) as usize * 1) as *const u64).read_unaligned(); t832[0] = vsetq_lane_u32::<0>(v as u32, t832[0]); t832[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t832[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t831[0]) as usize * 1) as *const u64).read_unaligned(); t832[0] = vsetq_lane_u32::<1>(v as u32, t832[0]); t832[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t832[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t831[0]) as usize * 1) as *const u64).read_unaligned(); t832[0] = vsetq_lane_u32::<2>(v as u32, t832[0]); t832[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t832[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t831[0]) as usize * 1) as *const u64).read_unaligned(); t832[0] = vsetq_lane_u32::<3>(v as u32, t832[0]); t832[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t832[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t831[1]) as usize * 1) as *const u64).read_unaligned(); t832[1] = vsetq_lane_u32::<0>(v as u32, t832[1]); t832[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t832[3]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t831[1]) as usize * 1) as *const u64).read_unaligned(); t832[1] = vsetq_lane_u32::<1>(v as u32, t832[1]); t832[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t832[3]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t831[1]) as usize * 1) as *const u64).read_unaligned(); t832[1] = vsetq_lane_u32::<2>(v as u32, t832[1]); t832[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t832[3]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t831[1]) as usize * 1) as *const u64).read_unaligned(); t832[1] = vsetq_lane_u32::<3>(v as u32, t832[1]); t832[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t832[3]); }
            t826 = t832;
        }
        res = t826;
        return res;
    }
    let t833: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8is)), vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8is))];
    let t834: [uint32x4_t; 2] = [t833[0], t833[1]];
    let t835: u64 = ((x8b) as usize as u64);
    let t836 = [vdupq_n_u32((t835) as u32); 2];
    let t837 = [vdupq_n_u32(((t835) >> 32) as u32); 2];
    let t838: [uint32x4_t; 2] = [vaddq_u32(t836[0], t834[0]), vaddq_u32(t836[1], t834[1])];
    let t839: [uint32x4_t; 2] = [vcltq_u32(t838[0], t836[0]), vcltq_u32(t838[1], t836[1])];
    let t840: [uint32x4_t; 4] = [t838[0], t838[1], vsubq_u32(vaddq_u32(t837[0], vdupq_n_u32(0)), t839[0]), vsubq_u32(vaddq_u32(t837[1], vdupq_n_u32(0)), t839[1])];
    let t841: u64 = !(7u64);
    let t842 = [vdupq_n_u32((t841) as u32); 2];
    let t843 = [vdupq_n_u32(((t841) >> 32) as u32); 2];
    let t844: [uint32x4_t; 4] = [vandq_u32(t840[0], t842[0]), vandq_u32(t840[1], t842[1]), vandq_u32(t840[2], t843[0]), vandq_u32(t840[3], t843[1])];
    let t845: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
    let t846: [uint32x4_t; 4] = [vandq_u32(t844[0], t845[0]), vandq_u32(t844[1], t845[1]), vandq_u32(t844[2], t845[2]), vandq_u32(t844[3], t845[3])];
    let t847: u64 = ((arg0) as usize as u64);
    let t848 = [vdupq_n_u32((t847) as u32); 2];
    let t849 = [vdupq_n_u32(((t847) >> 32) as u32); 2];
    let t850: [uint32x4_t; 2] = [vaddq_u32(t848[0], t846[0]), vaddq_u32(t848[1], t846[1])];
    let t851: [uint32x4_t; 2] = [vcltq_u32(t850[0], t848[0]), vcltq_u32(t850[1], t848[1])];
    let t852: [uint32x4_t; 4] = [t850[0], t850[1], vsubq_u32(vaddq_u32(t849[0], t846[2]), t851[0]), vsubq_u32(vaddq_u32(t849[1], t846[3]), t851[1])];
    x24 = t852;
    let t853: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8is)), vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8is))];
    let t854: [uint32x4_t; 2] = [t853[0], t853[1]];
    let t855: u64 = ((x8b) as usize as u64);
    let t856 = [vdupq_n_u32((t855) as u32); 2];
    let t857 = [vdupq_n_u32(((t855) >> 32) as u32); 2];
    let t858: [uint32x4_t; 2] = [vaddq_u32(t856[0], t854[0]), vaddq_u32(t856[1], t854[1])];
    let t859: [uint32x4_t; 2] = [vcltq_u32(t858[0], t856[0]), vcltq_u32(t858[1], t856[1])];
    let t860: [uint32x4_t; 4] = [t858[0], t858[1], vsubq_u32(vaddq_u32(t857[0], vdupq_n_u32(0)), t859[0]), vsubq_u32(vaddq_u32(t857[1], vdupq_n_u32(0)), t859[1])];
    let t861: u64 = 7u64;
    let t862 = [vdupq_n_u32((t861) as u32); 2];
    let t863: [uint32x4_t; 2] = [vandq_u32(t860[0], t862[0]), vandq_u32(t860[1], t862[1])];
    let t864 = [vdupq_n_u32(8u32); 2];
    let t865: [uint32x4_t; 2] = [vmulq_u32(t863[0], t864[0]), vmulq_u32(t863[1], t864[1])];
    x32 = t865;
    let mut t866: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x24[2]) as u64) << 32 | vgetq_lane_u32::<0>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t866[0] = vsetq_lane_u32::<0>(v as u32, t866[0]); t866[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t866[2]); }
    { let v = (((vgetq_lane_u32::<1>(x24[2]) as u64) << 32 | vgetq_lane_u32::<1>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t866[0] = vsetq_lane_u32::<1>(v as u32, t866[0]); t866[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t866[2]); }
    { let v = (((vgetq_lane_u32::<2>(x24[2]) as u64) << 32 | vgetq_lane_u32::<2>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t866[0] = vsetq_lane_u32::<2>(v as u32, t866[0]); t866[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t866[2]); }
    { let v = (((vgetq_lane_u32::<3>(x24[2]) as u64) << 32 | vgetq_lane_u32::<3>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t866[0] = vsetq_lane_u32::<3>(v as u32, t866[0]); t866[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t866[2]); }
    { let v = (((vgetq_lane_u32::<0>(x24[3]) as u64) << 32 | vgetq_lane_u32::<0>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t866[1] = vsetq_lane_u32::<0>(v as u32, t866[1]); t866[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t866[3]); }
    { let v = (((vgetq_lane_u32::<1>(x24[3]) as u64) << 32 | vgetq_lane_u32::<1>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t866[1] = vsetq_lane_u32::<1>(v as u32, t866[1]); t866[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t866[3]); }
    { let v = (((vgetq_lane_u32::<2>(x24[3]) as u64) << 32 | vgetq_lane_u32::<2>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t866[1] = vsetq_lane_u32::<2>(v as u32, t866[1]); t866[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t866[3]); }
    { let v = (((vgetq_lane_u32::<3>(x24[3]) as u64) << 32 | vgetq_lane_u32::<3>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t866[1] = vsetq_lane_u32::<3>(v as u32, t866[1]); t866[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t866[3]); }
    let t867: [uint32x4_t; 2] = [x32[0], x32[1]];
    let t868: [uint32x4_t; 2] = [vandq_u32(t867[0], vdupq_n_u32(63)), vandq_u32(t867[1], vdupq_n_u32(63))];
    let t869: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t866[0], vnegq_s32(vreinterpretq_s32_u32(t868[0]))), vshlq_u32(t866[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t868[0])))), vorrq_u32(vshlq_u32(t866[1], vnegq_s32(vreinterpretq_s32_u32(t868[1]))), vshlq_u32(t866[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t868[1])))), vshlq_u32(t866[2], vnegq_s32(vreinterpretq_s32_u32(t868[0]))), vshlq_u32(t866[3], vnegq_s32(vreinterpretq_s32_u32(t868[1])))];
    x40 = t869;
    let t870 = [vdupq_n_u32(0u32); 2];
    let t871: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t870[0])), vmvnq_u32(vceqq_u32(x32[1], t870[1]))];
    let t872 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t873: [uint32x4_t; 2] = [vaddq_u32(x32[0], t872[0]), vaddq_u32(x32[1], t872[1])];
    let t874 = [vdupq_n_u32(64u32); 2];
    let t875: [uint32x4_t; 2] = [vcgtq_u32(t873[0], t874[0]), vcgtq_u32(t873[1], t874[1])];
    let t876: [uint32x4_t; 2] = [vandq_u32(t871[0], t875[0]), vandq_u32(t871[1], t875[1])];
    let mut t877: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t878: [uint32x4_t; 2] = [vandq_u32(live[0], t876[0]), vandq_u32(live[1], t876[1])];
    let t879: [uint32x4_t; 2] = [vbicq_u32(live[0], t876[0]), vbicq_u32(live[1], t876[1])];
    if any(&t878) {
        let t880: u64 = (1u64).wrapping_mul(8u64);
        let t881 = [vdupq_n_u32((t880) as u32); 2];
        let t882 = [vdupq_n_u32(((t880) >> 32) as u32); 2];
        let t883: [uint32x4_t; 2] = [vaddq_u32(x24[0], t881[0]), vaddq_u32(x24[1], t881[1])];
        let t884: [uint32x4_t; 2] = [vcltq_u32(t883[0], x24[0]), vcltq_u32(t883[1], x24[1])];
        let t885: [uint32x4_t; 4] = [t883[0], t883[1], vsubq_u32(vaddq_u32(x24[2], t882[0]), t884[0]), vsubq_u32(vaddq_u32(x24[3], t882[1]), t884[1])];
        let mut t886: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t885[2]) as u64) << 32 | vgetq_lane_u32::<0>(t885[0]) as u64) as usize as *const u64).read_unaligned(); t886[0] = vsetq_lane_u32::<0>(v as u32, t886[0]); t886[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t886[2]); }
        { let v = (((vgetq_lane_u32::<1>(t885[2]) as u64) << 32 | vgetq_lane_u32::<1>(t885[0]) as u64) as usize as *const u64).read_unaligned(); t886[0] = vsetq_lane_u32::<1>(v as u32, t886[0]); t886[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t886[2]); }
        { let v = (((vgetq_lane_u32::<2>(t885[2]) as u64) << 32 | vgetq_lane_u32::<2>(t885[0]) as u64) as usize as *const u64).read_unaligned(); t886[0] = vsetq_lane_u32::<2>(v as u32, t886[0]); t886[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t886[2]); }
        { let v = (((vgetq_lane_u32::<3>(t885[2]) as u64) << 32 | vgetq_lane_u32::<3>(t885[0]) as u64) as usize as *const u64).read_unaligned(); t886[0] = vsetq_lane_u32::<3>(v as u32, t886[0]); t886[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t886[2]); }
        { let v = (((vgetq_lane_u32::<0>(t885[3]) as u64) << 32 | vgetq_lane_u32::<0>(t885[1]) as u64) as usize as *const u64).read_unaligned(); t886[1] = vsetq_lane_u32::<0>(v as u32, t886[1]); t886[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t886[3]); }
        { let v = (((vgetq_lane_u32::<1>(t885[3]) as u64) << 32 | vgetq_lane_u32::<1>(t885[1]) as u64) as usize as *const u64).read_unaligned(); t886[1] = vsetq_lane_u32::<1>(v as u32, t886[1]); t886[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t886[3]); }
        { let v = (((vgetq_lane_u32::<2>(t885[3]) as u64) << 32 | vgetq_lane_u32::<2>(t885[1]) as u64) as usize as *const u64).read_unaligned(); t886[1] = vsetq_lane_u32::<2>(v as u32, t886[1]); t886[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t886[3]); }
        { let v = (((vgetq_lane_u32::<3>(t885[3]) as u64) << 32 | vgetq_lane_u32::<3>(t885[1]) as u64) as usize as *const u64).read_unaligned(); t886[1] = vsetq_lane_u32::<3>(v as u32, t886[1]); t886[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t886[3]); }
        let t887: [uint32x4_t; 2] = [vsubq_u32(t874[0], x32[0]), vsubq_u32(t874[1], x32[1])];
        let t888: [uint32x4_t; 2] = [t887[0], t887[1]];
        let t889: [uint32x4_t; 2] = [vandq_u32(t888[0], vdupq_n_u32(63)), vandq_u32(t888[1], vdupq_n_u32(63))];
        let t890: [uint32x4_t; 4] = [vshlq_u32(t886[0], vreinterpretq_s32_u32(t889[0])), vshlq_u32(t886[1], vreinterpretq_s32_u32(t889[1])), vorrq_u32(vshlq_u32(t886[2], vreinterpretq_s32_u32(t889[0])), vshlq_u32(t886[0], vsubq_s32(vreinterpretq_s32_u32(t889[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t886[3], vreinterpretq_s32_u32(t889[1])), vshlq_u32(t886[1], vsubq_s32(vreinterpretq_s32_u32(t889[1]), vdupq_n_s32(32))))];
        let t891: [uint32x4_t; 4] = [vorrq_u32(x40[0], t890[0]), vorrq_u32(x40[1], t890[1]), vorrq_u32(x40[2], t890[2]), vorrq_u32(x40[3], t890[3])];
        t877 = [vbslq_u32(t878[0], t891[0], t877[0]), vbslq_u32(t878[1], t891[1], t877[1]), vbslq_u32(t878[0], t891[2], t877[2]), vbslq_u32(t878[1], t891[3], t877[3])];
    }
    {
        t877 = [vbslq_u32(t879[0], x40[0], t877[0]), vbslq_u32(t879[1], x40[1], t877[1]), vbslq_u32(t879[0], x40[2], t877[2]), vbslq_u32(t879[1], x40[3], t877[3])];
    }
    res = t877;
    return res;
}

#[inline(always)]
unsafe fn b17_r0an1s_f(l: &Launch, lid0: u32, arg0: *const u8, x8b: *const u8, x8if: u32, x8is: u32, x16: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    if ((x16) == (8u32)) {
        let t892: [uint32x4_t; 4];
        if (x8is) == 8u32 && ((x8if) as u64) + 7 * ((x8is) as u64) <= u32::MAX as u64 {
            let t893 = ((arg0).wrapping_add((x8b) as usize) as *const u8).add(((x8if) as usize) * 1);
            let t8940 = vld2q_u32((t893 as *const u32).add(0));
            let t8941 = vld2q_u32((t893 as *const u32).add(8));
            let t894: [uint32x4_t; 4] = [t8940.0, t8941.0, t8940.1, t8941.1];
            t892 = t894;
        } else {
            let t895: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8is)), vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8is))];
            let mut t896: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t895[0]) as usize * 1) as *const u64).read_unaligned(); t896[0] = vsetq_lane_u32::<0>(v as u32, t896[0]); t896[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t896[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t895[0]) as usize * 1) as *const u64).read_unaligned(); t896[0] = vsetq_lane_u32::<1>(v as u32, t896[0]); t896[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t896[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t895[0]) as usize * 1) as *const u64).read_unaligned(); t896[0] = vsetq_lane_u32::<2>(v as u32, t896[0]); t896[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t896[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t895[0]) as usize * 1) as *const u64).read_unaligned(); t896[0] = vsetq_lane_u32::<3>(v as u32, t896[0]); t896[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t896[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t895[1]) as usize * 1) as *const u64).read_unaligned(); t896[1] = vsetq_lane_u32::<0>(v as u32, t896[1]); t896[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t896[3]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t895[1]) as usize * 1) as *const u64).read_unaligned(); t896[1] = vsetq_lane_u32::<1>(v as u32, t896[1]); t896[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t896[3]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t895[1]) as usize * 1) as *const u64).read_unaligned(); t896[1] = vsetq_lane_u32::<2>(v as u32, t896[1]); t896[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t896[3]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t895[1]) as usize * 1) as *const u64).read_unaligned(); t896[1] = vsetq_lane_u32::<3>(v as u32, t896[1]); t896[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t896[3]); }
            t892 = t896;
        }
        res = t892;
        return res;
    }
    let t897: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8is)), vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8is))];
    let t898: [uint32x4_t; 2] = [t897[0], t897[1]];
    let t899: u64 = ((x8b) as usize as u64);
    let t900 = [vdupq_n_u32((t899) as u32); 2];
    let t901 = [vdupq_n_u32(((t899) >> 32) as u32); 2];
    let t902: [uint32x4_t; 2] = [vaddq_u32(t900[0], t898[0]), vaddq_u32(t900[1], t898[1])];
    let t903: [uint32x4_t; 2] = [vcltq_u32(t902[0], t900[0]), vcltq_u32(t902[1], t900[1])];
    let t904: [uint32x4_t; 4] = [t902[0], t902[1], vsubq_u32(vaddq_u32(t901[0], vdupq_n_u32(0)), t903[0]), vsubq_u32(vaddq_u32(t901[1], vdupq_n_u32(0)), t903[1])];
    let t905: u64 = !(7u64);
    let t906 = [vdupq_n_u32((t905) as u32); 2];
    let t907 = [vdupq_n_u32(((t905) >> 32) as u32); 2];
    let t908: [uint32x4_t; 4] = [vandq_u32(t904[0], t906[0]), vandq_u32(t904[1], t906[1]), vandq_u32(t904[2], t907[0]), vandq_u32(t904[3], t907[1])];
    let t909: u64 = ((arg0) as usize as u64);
    let t910 = [vdupq_n_u32((t909) as u32); 2];
    let t911 = [vdupq_n_u32(((t909) >> 32) as u32); 2];
    let t912: [uint32x4_t; 2] = [vaddq_u32(t910[0], t908[0]), vaddq_u32(t910[1], t908[1])];
    let t913: [uint32x4_t; 2] = [vcltq_u32(t912[0], t910[0]), vcltq_u32(t912[1], t910[1])];
    let t914: [uint32x4_t; 4] = [t912[0], t912[1], vsubq_u32(vaddq_u32(t911[0], t908[2]), t913[0]), vsubq_u32(vaddq_u32(t911[1], t908[3]), t913[1])];
    x24 = t914;
    let t915: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8is)), vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8is))];
    let t916: [uint32x4_t; 2] = [t915[0], t915[1]];
    let t917: u64 = ((x8b) as usize as u64);
    let t918 = [vdupq_n_u32((t917) as u32); 2];
    let t919 = [vdupq_n_u32(((t917) >> 32) as u32); 2];
    let t920: [uint32x4_t; 2] = [vaddq_u32(t918[0], t916[0]), vaddq_u32(t918[1], t916[1])];
    let t921: [uint32x4_t; 2] = [vcltq_u32(t920[0], t918[0]), vcltq_u32(t920[1], t918[1])];
    let t922: [uint32x4_t; 4] = [t920[0], t920[1], vsubq_u32(vaddq_u32(t919[0], vdupq_n_u32(0)), t921[0]), vsubq_u32(vaddq_u32(t919[1], vdupq_n_u32(0)), t921[1])];
    let t923: u64 = 7u64;
    let t924 = [vdupq_n_u32((t923) as u32); 2];
    let t925: [uint32x4_t; 2] = [vandq_u32(t922[0], t924[0]), vandq_u32(t922[1], t924[1])];
    let t926 = [vdupq_n_u32(8u32); 2];
    let t927: [uint32x4_t; 2] = [vmulq_u32(t925[0], t926[0]), vmulq_u32(t925[1], t926[1])];
    x32 = t927;
    let mut t928: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x24[2]) as u64) << 32 | vgetq_lane_u32::<0>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t928[0] = vsetq_lane_u32::<0>(v as u32, t928[0]); t928[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t928[2]); }
    { let v = (((vgetq_lane_u32::<1>(x24[2]) as u64) << 32 | vgetq_lane_u32::<1>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t928[0] = vsetq_lane_u32::<1>(v as u32, t928[0]); t928[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t928[2]); }
    { let v = (((vgetq_lane_u32::<2>(x24[2]) as u64) << 32 | vgetq_lane_u32::<2>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t928[0] = vsetq_lane_u32::<2>(v as u32, t928[0]); t928[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t928[2]); }
    { let v = (((vgetq_lane_u32::<3>(x24[2]) as u64) << 32 | vgetq_lane_u32::<3>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t928[0] = vsetq_lane_u32::<3>(v as u32, t928[0]); t928[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t928[2]); }
    { let v = (((vgetq_lane_u32::<0>(x24[3]) as u64) << 32 | vgetq_lane_u32::<0>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t928[1] = vsetq_lane_u32::<0>(v as u32, t928[1]); t928[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t928[3]); }
    { let v = (((vgetq_lane_u32::<1>(x24[3]) as u64) << 32 | vgetq_lane_u32::<1>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t928[1] = vsetq_lane_u32::<1>(v as u32, t928[1]); t928[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t928[3]); }
    { let v = (((vgetq_lane_u32::<2>(x24[3]) as u64) << 32 | vgetq_lane_u32::<2>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t928[1] = vsetq_lane_u32::<2>(v as u32, t928[1]); t928[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t928[3]); }
    { let v = (((vgetq_lane_u32::<3>(x24[3]) as u64) << 32 | vgetq_lane_u32::<3>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t928[1] = vsetq_lane_u32::<3>(v as u32, t928[1]); t928[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t928[3]); }
    let t929: [uint32x4_t; 2] = [x32[0], x32[1]];
    let t930: [uint32x4_t; 2] = [vandq_u32(t929[0], vdupq_n_u32(63)), vandq_u32(t929[1], vdupq_n_u32(63))];
    let t931: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t928[0], vnegq_s32(vreinterpretq_s32_u32(t930[0]))), vshlq_u32(t928[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t930[0])))), vorrq_u32(vshlq_u32(t928[1], vnegq_s32(vreinterpretq_s32_u32(t930[1]))), vshlq_u32(t928[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t930[1])))), vshlq_u32(t928[2], vnegq_s32(vreinterpretq_s32_u32(t930[0]))), vshlq_u32(t928[3], vnegq_s32(vreinterpretq_s32_u32(t930[1])))];
    x40 = t931;
    let t932 = [vdupq_n_u32(0u32); 2];
    let t933: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t932[0])), vmvnq_u32(vceqq_u32(x32[1], t932[1]))];
    let t934 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t935: [uint32x4_t; 2] = [vaddq_u32(x32[0], t934[0]), vaddq_u32(x32[1], t934[1])];
    let t936 = [vdupq_n_u32(64u32); 2];
    let t937: [uint32x4_t; 2] = [vcgtq_u32(t935[0], t936[0]), vcgtq_u32(t935[1], t936[1])];
    let t938: [uint32x4_t; 2] = [vandq_u32(t933[0], t937[0]), vandq_u32(t933[1], t937[1])];
    let mut t939: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t940: [uint32x4_t; 2] = [vmvnq_u32(t938[0]), vmvnq_u32(t938[1])];
    if any(&t938) {
        let t941: u64 = (1u64).wrapping_mul(8u64);
        let t942 = [vdupq_n_u32((t941) as u32); 2];
        let t943 = [vdupq_n_u32(((t941) >> 32) as u32); 2];
        let t944: [uint32x4_t; 2] = [vaddq_u32(x24[0], t942[0]), vaddq_u32(x24[1], t942[1])];
        let t945: [uint32x4_t; 2] = [vcltq_u32(t944[0], x24[0]), vcltq_u32(t944[1], x24[1])];
        let t946: [uint32x4_t; 4] = [t944[0], t944[1], vsubq_u32(vaddq_u32(x24[2], t943[0]), t945[0]), vsubq_u32(vaddq_u32(x24[3], t943[1]), t945[1])];
        let mut t947: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t946[2]) as u64) << 32 | vgetq_lane_u32::<0>(t946[0]) as u64) as usize as *const u64).read_unaligned(); t947[0] = vsetq_lane_u32::<0>(v as u32, t947[0]); t947[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t947[2]); }
        { let v = (((vgetq_lane_u32::<1>(t946[2]) as u64) << 32 | vgetq_lane_u32::<1>(t946[0]) as u64) as usize as *const u64).read_unaligned(); t947[0] = vsetq_lane_u32::<1>(v as u32, t947[0]); t947[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t947[2]); }
        { let v = (((vgetq_lane_u32::<2>(t946[2]) as u64) << 32 | vgetq_lane_u32::<2>(t946[0]) as u64) as usize as *const u64).read_unaligned(); t947[0] = vsetq_lane_u32::<2>(v as u32, t947[0]); t947[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t947[2]); }
        { let v = (((vgetq_lane_u32::<3>(t946[2]) as u64) << 32 | vgetq_lane_u32::<3>(t946[0]) as u64) as usize as *const u64).read_unaligned(); t947[0] = vsetq_lane_u32::<3>(v as u32, t947[0]); t947[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t947[2]); }
        { let v = (((vgetq_lane_u32::<0>(t946[3]) as u64) << 32 | vgetq_lane_u32::<0>(t946[1]) as u64) as usize as *const u64).read_unaligned(); t947[1] = vsetq_lane_u32::<0>(v as u32, t947[1]); t947[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t947[3]); }
        { let v = (((vgetq_lane_u32::<1>(t946[3]) as u64) << 32 | vgetq_lane_u32::<1>(t946[1]) as u64) as usize as *const u64).read_unaligned(); t947[1] = vsetq_lane_u32::<1>(v as u32, t947[1]); t947[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t947[3]); }
        { let v = (((vgetq_lane_u32::<2>(t946[3]) as u64) << 32 | vgetq_lane_u32::<2>(t946[1]) as u64) as usize as *const u64).read_unaligned(); t947[1] = vsetq_lane_u32::<2>(v as u32, t947[1]); t947[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t947[3]); }
        { let v = (((vgetq_lane_u32::<3>(t946[3]) as u64) << 32 | vgetq_lane_u32::<3>(t946[1]) as u64) as usize as *const u64).read_unaligned(); t947[1] = vsetq_lane_u32::<3>(v as u32, t947[1]); t947[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t947[3]); }
        let t948: [uint32x4_t; 2] = [vsubq_u32(t936[0], x32[0]), vsubq_u32(t936[1], x32[1])];
        let t949: [uint32x4_t; 2] = [t948[0], t948[1]];
        let t950: [uint32x4_t; 2] = [vandq_u32(t949[0], vdupq_n_u32(63)), vandq_u32(t949[1], vdupq_n_u32(63))];
        let t951: [uint32x4_t; 4] = [vshlq_u32(t947[0], vreinterpretq_s32_u32(t950[0])), vshlq_u32(t947[1], vreinterpretq_s32_u32(t950[1])), vorrq_u32(vshlq_u32(t947[2], vreinterpretq_s32_u32(t950[0])), vshlq_u32(t947[0], vsubq_s32(vreinterpretq_s32_u32(t950[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t947[3], vreinterpretq_s32_u32(t950[1])), vshlq_u32(t947[1], vsubq_s32(vreinterpretq_s32_u32(t950[1]), vdupq_n_s32(32))))];
        let t952: [uint32x4_t; 4] = [vorrq_u32(x40[0], t951[0]), vorrq_u32(x40[1], t951[1]), vorrq_u32(x40[2], t951[2]), vorrq_u32(x40[3], t951[3])];
        t939 = [vbslq_u32(t938[0], t952[0], t939[0]), vbslq_u32(t938[1], t952[1], t939[1]), vbslq_u32(t938[0], t952[2], t939[2]), vbslq_u32(t938[1], t952[3], t939[3])];
    }
    {
        t939 = [vbslq_u32(t940[0], x40[0], t939[0]), vbslq_u32(t940[1], x40[1], t939[1]), vbslq_u32(t940[0], x40[2], t939[2]), vbslq_u32(t940[1], x40[3], t939[3])];
    }
    res = t939;
    return res;
}

#[inline(always)]
unsafe fn b18_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t953: u64;
    if ((x0) >= (64u32)) {
        t953 = !(0u64);
    } else {
        t953 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t953;
    return res;
}

#[inline(always)]
unsafe fn b18_s_f(l: &Launch, lid0: u32, x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t954: u64;
    if ((x0) >= (64u32)) {
        t954 = !(0u64);
    } else {
        t954 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t954;
    return res;
}

#[inline(always)]
unsafe fn b19_r0sss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: u32, x16: u32, x24: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u64 = 0;
    let mut live = k;
    let mut res: u64 = 0;
    let t955 = b17_r0ss_p(l, lid0, live, arg0, (((x8) as u64)).wrapping_add(((x16) as u64)), x24);
    x32 = t955;
    let t956: u64;
    if ((x24) >= (8u32)) {
        t956 = x32;
    } else {
        let t957 = b18_s_p(l, lid0, live, (8u32).wrapping_mul(x24));
        t956 = ((x32) & (t957));
    }
    res = t956;
    return res;
}

#[inline(always)]
unsafe fn b19_r0sss_f(l: &Launch, lid0: u32, arg0: *const u8, x8: u32, x16: u32, x24: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u64 = 0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t958 = b17_r0ss_f(l, lid0, arg0, (((x8) as u64)).wrapping_add(((x16) as u64)), x24);
    x32 = t958;
    let t959: u64;
    if ((x24) >= (8u32)) {
        t959 = x32;
    } else {
        let t960 = b18_s_f(l, lid0, (8u32).wrapping_mul(x24));
        t959 = ((x32) & (t960));
    }
    res = t959;
    return res;
}

#[inline(always)]
unsafe fn b19_r0sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: u32, x16: [uint32x4_t; 2], x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t961 = b17_r0al1s_p(l, lid0, live, arg0, ((((x8) as u64)) as usize as *const u8), x16, x24);
    x32 = t961;
    let t962: [uint32x4_t; 4];
    if ((x24) >= (8u32)) {
        t962 = x32;
    } else {
        let t963 = b18_s_p(l, lid0, live, (8u32).wrapping_mul(x24));
        let t964: u64 = t963;
        let t965 = [vdupq_n_u32((t964) as u32); 2];
        let t966 = [vdupq_n_u32(((t964) >> 32) as u32); 2];
        let t967: [uint32x4_t; 4] = [vandq_u32(x32[0], t965[0]), vandq_u32(x32[1], t965[1]), vandq_u32(x32[2], t966[0]), vandq_u32(x32[3], t966[1])];
        t962 = t967;
    }
    res = t962;
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
    let x24: *const u8 = *argv.add(3);
    let arg4: *const u8 = (*argv.add(4) as *const *const u8).read_unaligned();
    let x48: u32 = (*argv.add(5) as *const u32).read_unaligned();
    let mut t968: bool = !(arg4).is_null();
    if t968 {
        let mut t969: bool = ((l.group[0]) >= ((arg4.wrapping_add((((x48) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        if !t969 {
            t969 = ((l.group[1]) >= ((arg4.wrapping_add(((((x48).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        let mut t970: bool = t969;
        if !t970 {
            t970 = ((l.group[2]) >= ((arg4.wrapping_add(((((x48).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        t968 = t970;
    }
    if t968 {
        return;
    }
    let mut lid0 = 0u32;
    while lid0 + 8 <= n {
        let mut live = [vdupq_n_u32(u32::MAX); 2];
        let gid0 = l.group[0] * l.block[0] + lid0;
        'chunk: {
            b0_r0r1r2sg_f(l, lid0, arg0, arg1, arg2, x24);
        }
        lid0 += 8;
    }
    if lid0 < n {
        // Lanes part: the group's last threads, those under `n`.
        let t971: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
        let mut live = t971;
        let gid0 = l.group[0] * l.block[0] + lid0;
        'chunk: {
            b0_r0r1r2sg_p(l, lid0, live, arg0, arg1, arg2, x24);
        }
    }
}
